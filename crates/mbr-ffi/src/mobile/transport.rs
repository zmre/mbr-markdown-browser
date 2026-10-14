//! `mbr://` requests in, streamed responses out.
//!
//! The app's `URLSchemeHandler` hands each request to [`super::MbrApp::handle`],
//! which runs it through mbr's axum router in-process (`Router::oneshot`): no
//! socket, no port. The adapter supplies what a socket would have and
//! smooths over what WebKit does differently for a custom scheme:
//!
//! | Concern | What the adapter does |
//! |---------|-----------------------|
//! | `ConnectInfo<SocketAddr>` (every edit handler extracts it) | injected as a request extension: loopback `127.0.0.1:0` |
//! | `Host` | derived from the URL authority (`localhost`) when the request has none, as an HTTP client would send it |
//! | `X-MBR-Edit` + same-origin | forwarded verbatim; WebKit sends no `Origin` on a same-origin `mbr://` POST, and a foreign `Origin` is still refused |
//! | `Content-Encoding` | `Accept-Encoding` is dropped: WebKit hands a scheme handler's body to the page undecoded |
//! | Redirects | WebKit ignores a 3xx from a scheme handler, so a 3xx answering a page load becomes a document that calls `location.replace` |
//! | Large bodies | streamed: [`MbrResponse::next_chunk`] pulls the body a frame at a time, with a small bounded buffer, so a video never sits in memory whole |
//!
//! So the one contract with the Swift side is: **load pages from
//! `mbr://localhost/`**. Any other authority fails the edit endpoints' `Host`
//! check, which is right: it would not be this app's page.

use std::net::SocketAddr;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::ConnectInfo;
use axum::http::{
    self, HeaderName, HeaderValue, Method, Request, Response, StatusCode, Uri, header,
};
use futures::StreamExt;
use tower::ServiceExt;

use super::MobileError;

/// The authority the app must load pages from. See the module docs.
pub const MOBILE_HOST: &str = "localhost";

/// Frames buffered ahead of the app's reads of one body. A frame is at most
/// `ServeFile`'s or the vault streamer's chunk (64–256 KiB), so a response
/// holds well under a megabyte however large the file.
const BUFFERED_FRAMES: usize = 4;

/// One HTTP header. A list rather than a map so repeated headers survive.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(target_vendor = "apple", derive(uniffi::Record))]
pub struct MobileHeader {
    pub name: String,
    pub value: String,
}

impl MobileHeader {
    pub fn new(name: &str, value: &str) -> Self {
        Self {
            name: name.to_owned(),
            value: value.to_owned(),
        }
    }
}

/// A response whose body is read in chunks.
///
/// `status` and `headers` are final when [`super::MbrApp::handle`] returns;
/// the body follows through [`Self::next_chunk`] until it returns `None`.
/// Dropping the response early cancels the rest of the body.
#[cfg_attr(target_vendor = "apple", derive(uniffi::Object))]
pub struct MbrResponse {
    status: u16,
    headers: Vec<MobileHeader>,
    body: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Result<Bytes, String>>>,
}

#[cfg_attr(target_vendor = "apple", uniffi::export)]
impl MbrResponse {
    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn headers(&self) -> Vec<MobileHeader> {
        self.headers.clone()
    }

    /// The next piece of the body, or `None` once it is complete. An error
    /// means the body broke off (a file vanished mid-read): the app should
    /// fail the load rather than show a truncated resource.
    pub async fn next_chunk(&self) -> Result<Option<Vec<u8>>, MobileError> {
        match self.body.lock().await.recv().await {
            None => Ok(None),
            Some(Ok(bytes)) => Ok(Some(bytes.to_vec())),
            Some(Err(message)) => Err(MobileError::Internal { message }),
        }
    }

    /// The rest of the body at once — for small responses (JSON, pages)
    /// where streaming buys nothing.
    pub async fn read_all(&self) -> Result<Vec<u8>, MobileError> {
        let mut body = Vec::new();
        while let Some(chunk) = self.next_chunk().await? {
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

impl MbrResponse {
    /// Takes a router response apart and starts pumping its body on
    /// `runtime`, a few frames ahead of the reader.
    pub(crate) fn from_response(
        response: Response<Body>,
        runtime: &tokio::runtime::Handle,
    ) -> Self {
        let (parts, body) = response.into_parts();
        let headers = parts
            .headers
            .iter()
            .map(|(name, value)| MobileHeader {
                name: name.as_str().to_owned(),
                value: String::from_utf8_lossy(value.as_bytes()).into_owned(),
            })
            .collect();
        let (tx, rx) = tokio::sync::mpsc::channel(BUFFERED_FRAMES);
        runtime.spawn(async move {
            let mut stream = body.into_data_stream();
            while let Some(frame) = stream.next().await {
                let item = frame.map_err(|e| e.to_string());
                let failed = item.is_err();
                // The reader went away: stop reading (and drop the body).
                if tx.send(item).await.is_err() || failed {
                    break;
                }
            }
        });
        Self {
            status: parts.status.as_u16(),
            headers,
            body: tokio::sync::Mutex::new(rx),
        }
    }
}

/// Turns the scheme handler's view of a request into an axum request.
pub(crate) fn build_request(
    method: &str,
    url: &str,
    headers: &[MobileHeader],
    body: Vec<u8>,
) -> Result<Request<Body>, MobileError> {
    let invalid = |message: String| MobileError::InvalidRequest { message };

    let method = Method::from_bytes(method.as_bytes())
        .map_err(|e| invalid(format!("method {method:?}: {e}")))?;
    let uri: Uri = url
        .parse()
        .map_err(|e| invalid(format!("url {url:?}: {e}")))?;
    // Route on origin-form (`/path?query`), as a server would receive it.
    let path_and_query = uri
        .path_and_query()
        .map_or("/", http::uri::PathAndQuery::as_str);
    let origin_form: Uri = path_and_query
        .parse()
        .map_err(|e| invalid(format!("path {path_and_query:?}: {e}")))?;

    let mut request = Request::builder()
        .method(method)
        .uri(origin_form)
        .body(Body::from(body))
        .map_err(|e| invalid(e.to_string()))?;

    let request_headers = request.headers_mut();
    for MobileHeader { name, value } in headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| invalid(format!("header name {name:?}: {e}")))?;
        let value = HeaderValue::from_str(value)
            .map_err(|e| invalid(format!("header value for {name}: {e}")))?;
        request_headers.append(name, value);
    }
    // WebKit hands a custom scheme's response body to the page as-is; it does
    // not undo a `Content-Encoding`, so the compression layer must not apply
    // one.
    request_headers.remove(header::ACCEPT_ENCODING);
    if !request_headers.contains_key(header::HOST)
        && let Some(authority) = uri.authority()
    {
        let host = HeaderValue::from_str(authority.as_str())
            .map_err(|e| invalid(format!("host {authority}: {e}")))?;
        request_headers.insert(header::HOST, host);
    }

    // Every edit handler extracts `ConnectInfo`; without it axum answers 500.
    // The only caller is this app's web view, i.e. loopback.
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 0))));
    Ok(request)
}

/// Runs one request through the router.
///
/// `home` is where `/` goes instead of the router (a single-file vault's
/// page); a page load answered with a redirect is replayed in the page.
pub(crate) async fn dispatch(
    router: Router,
    request: Request<Body>,
    home: Option<&str>,
) -> Response<Body> {
    let navigation = is_document_navigation(&request);
    let response = match home {
        Some(home)
            if request.uri().path() == "/"
                && matches!(*request.method(), Method::GET | Method::HEAD) =>
        {
            redirect(home)
        }
        _ => match router.oneshot(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        },
    };
    if navigation {
        redirect_as_document(response)
    } else {
        response
    }
}

/// A plain `302` to `location`.
fn redirect(location: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, location)
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// A top-level page load, as opposed to a `fetch` or subresource: a `GET`
/// that accepts HTML.
pub(crate) fn is_document_navigation(request: &Request<Body>) -> bool {
    request.method() == Method::GET
        && request
            .headers()
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|accept| accept.contains("text/html"))
}

/// Turns a 3xx answer to a page load into a document that navigates itself.
///
/// WebKit does not follow redirects returned by a `URLSchemeHandler`: it shows
/// the (empty) 301 body at the *old* URL (verified on the iOS 27 simulator).
/// mbr relies on its 301 to the trailing-slash URL, since that slash is what
/// the page's own relative links resolve against, so the redirect is replayed
/// in the page instead. `location.replace` keeps the slashless URL out of the
/// back/forward history, like a real redirect.
pub(crate) fn redirect_as_document(response: Response<Body>) -> Response<Body> {
    if !response.status().is_redirection() {
        return response;
    }
    let Some(location) = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
    else {
        return response;
    };
    // JSON is a valid JS string literal; `<` is escaped so the value can never
    // close the script element.
    let target = serde_json::to_string(location)
        .unwrap_or_else(|_| "\"/\"".to_owned())
        .replace('<', "\\u003c");
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><script>location.replace({target})</script>"
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn body_text(response: Response<Body>) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn redirect_to(location: &str) -> Response<Body> {
        Response::builder()
            .status(StatusCode::MOVED_PERMANENTLY)
            .header(header::LOCATION, location)
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn a_redirect_becomes_a_self_navigating_document() {
        let response = redirect_as_document(redirect_to("/note/"));
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            body_text(response)
                .await
                .contains(r#"location.replace("/note/")"#)
        );
    }

    #[tokio::test]
    async fn the_redirect_document_cannot_close_its_script() {
        let response = redirect_as_document(redirect_to("/x</script><b>"));
        let html = body_text(response).await;
        assert_eq!(html.matches("</script>").count(), 1, "{html}");
    }

    #[tokio::test]
    async fn non_redirects_pass_through_untouched() {
        let original = Response::builder()
            .status(StatusCode::OK)
            .header(header::LOCATION, "/ignored/")
            .body(Body::from("page"))
            .unwrap();
        let response = redirect_as_document(original);
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_text(response).await, "page");
        // A 3xx without a Location has nowhere to go.
        let bare = Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            redirect_as_document(bare).status(),
            StatusCode::NOT_MODIFIED
        );
    }

    #[test]
    fn build_request_derives_host_and_injects_a_loopback_peer() {
        let request = build_request(
            "GET",
            "mbr://localhost/a%20b/?x=1",
            &[MobileHeader::new("Accept-Encoding", "gzip")],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(request.uri(), "/a%20b/?x=1");
        assert_eq!(request.headers()[header::HOST], "localhost");
        assert!(!request.headers().contains_key(header::ACCEPT_ENCODING));
        let ConnectInfo(peer) = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .unwrap();
        assert!(peer.ip().is_loopback());
        // A Host the request names itself is kept.
        let explicit = build_request(
            "GET",
            "mbr://localhost/",
            &[MobileHeader::new("Host", "elsewhere")],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(explicit.headers()[header::HOST], "elsewhere");
    }

    #[test]
    fn build_request_rejects_garbage() {
        assert!(build_request("GET", "not a url", &[], Vec::new()).is_err());
        assert!(build_request("G E T", "mbr://localhost/", &[], Vec::new()).is_err());
        assert!(
            build_request(
                "GET",
                "mbr://localhost/",
                &[MobileHeader::new("bad header", "x")],
                Vec::new()
            )
            .is_err()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn bodies_stream_in_frames_and_stop_when_dropped() {
        let frames = futures::stream::iter(
            (0..10u8).map(|i| Ok::<_, std::io::Error>(Bytes::from(vec![i; 1000]))),
        );
        let response = Response::new(Body::from_stream(frames));
        let streamed = MbrResponse::from_response(response, &tokio::runtime::Handle::current());
        assert_eq!(streamed.status(), 200);
        let first = streamed.next_chunk().await.unwrap().unwrap();
        assert_eq!(first, vec![0; 1000]);
        let rest = streamed.read_all().await.unwrap();
        assert_eq!(rest.len(), 9000);
        assert_eq!(streamed.next_chunk().await.unwrap(), None);

        let broken = futures::stream::iter(vec![
            Ok(Bytes::from_static(b"ok")),
            Err(std::io::Error::other("vanished")),
        ]);
        let response = MbrResponse::from_response(
            Response::new(Body::from_stream(broken)),
            &tokio::runtime::Handle::current(),
        );
        assert_eq!(response.next_chunk().await.unwrap().unwrap(), b"ok");
        assert!(response.next_chunk().await.is_err());
    }
}

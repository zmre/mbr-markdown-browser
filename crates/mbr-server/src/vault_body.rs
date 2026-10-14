//! Serving a file that exists only inside a [`Vault`] — no
//! [`Vault::local_path`], so `tower_http`'s `ServeFile` cannot open it.
//!
//! The desktop never comes here: its `LocalVault` always has a local path and
//! keeps going through `ServeFile`. This is the path for vaults that hold their
//! bytes somewhere else (an in-memory vault in tests; later a sync library or
//! Android's storage framework), and it answers the same way `ServeFile` does:
//!
//! - `Accept-Ranges: bytes` on every answer, and `Range` honoured with a
//!   `206 Partial Content` plus `Content-Range` — media elements seek with it;
//! - the range grammar and the `416 Range Not Satisfiable` cases are
//!   `http-range-header`'s, the crate `ServeFile` itself parses with, so a
//!   malformed, unsatisfiable or multi-part range is refused identically
//!   whichever path served the file;
//! - the body is **streamed** in [`CHUNK_BYTES`] reads through
//!   [`Vault::read_range`], each on the blocking pool, so a large video never
//!   sits in memory whole (the iOS WebContent process is killed near 2 GB).
//!   Small files are one read.

use std::ops::RangeInclusive;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Response, StatusCode, header};
use futures::StreamExt;
use mbr_core::vault::{Vault, VaultPath};

/// Bytes per streamed read.
pub(crate) const CHUNK_BYTES: usize = 256 * 1024;

/// What a request's `Range` header asks for, against a file of known size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RequestedRange {
    /// No `Range` header: the whole file, `200`.
    Whole,
    /// One satisfiable range, `206`.
    Partial(RangeInclusive<u64>),
    /// Malformed, multi-part or out of bounds: `416`.
    Unsatisfiable,
}

/// Parses `Range` as `ServeFile` does (see the module docs).
pub(crate) fn requested_range(headers: &HeaderMap, size: u64) -> RequestedRange {
    let Some(value) = headers.get(header::RANGE) else {
        return RequestedRange::Whole;
    };
    let Ok(value) = value.to_str() else {
        return RequestedRange::Unsatisfiable;
    };
    match http_range_header::parse_range_header(value).and_then(|parsed| parsed.validate(size)) {
        Ok(ranges) if ranges.len() == 1 => {
            let range = ranges.into_iter().next().unwrap_or(0..=0);
            // An empty file has no byte to satisfy any range with.
            if size == 0 || range.start() > range.end() {
                RequestedRange::Unsatisfiable
            } else {
                RequestedRange::Partial(range)
            }
        }
        _ => RequestedRange::Unsatisfiable,
    }
}

/// The response for `path` in `vault`, a file of `size` bytes.
///
/// `content_type` and `last_modified` are the caller's (it already knows the
/// file's name and stat). `HEAD` answers the headers without a body.
pub(crate) fn vault_file_response(
    vault: Arc<dyn Vault>,
    path: VaultPath,
    size: u64,
    content_type: &str,
    last_modified: Option<String>,
    headers: &HeaderMap,
    head_only: bool,
) -> Response<Body> {
    let mut builder = Response::builder()
        .header(header::ACCEPT_RANGES, "bytes")
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_str(content_type)
                .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
        );
    if let Some(last_modified) = last_modified {
        builder = builder.header(header::LAST_MODIFIED, last_modified);
    }
    let (status, range) = match requested_range(headers, size) {
        RequestedRange::Whole => (StatusCode::OK, (size > 0).then(|| 0..=size - 1)),
        RequestedRange::Partial(range) => {
            builder = builder.header(
                header::CONTENT_RANGE,
                format!("bytes {}-{}/{size}", range.start(), range.end()),
            );
            (StatusCode::PARTIAL_CONTENT, Some(range))
        }
        RequestedRange::Unsatisfiable => {
            return builder
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{size}"))
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(Body::from("Range Not Satisfiable"))
                .unwrap_or_else(|_| Response::new(Body::empty()));
        }
    };
    let length = range.as_ref().map_or(0, |r| r.end() - r.start() + 1);
    let body = match range {
        Some(range) if !head_only => streamed(vault, path, range),
        _ => Body::empty(),
    };
    builder
        .status(status)
        .header(header::CONTENT_LENGTH, length)
        .body(body)
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// The bytes of `range`, read [`CHUNK_BYTES`] at a time on the blocking pool.
/// A read that fails ends the stream with the error, which aborts the body:
/// the client sees a truncated response rather than wrong bytes.
fn streamed(vault: Arc<dyn Vault>, path: VaultPath, range: RangeInclusive<u64>) -> Body {
    let end = *range.end() + 1;
    let stream = futures::stream::unfold(*range.start(), move |offset| {
        let vault = Arc::clone(&vault);
        let path = path.clone();
        async move {
            if offset >= end {
                return None;
            }
            let want = usize::try_from(end - offset)
                .unwrap_or(usize::MAX)
                .min(CHUNK_BYTES);
            let read = tokio::task::spawn_blocking(move || vault.read_range(&path, offset, want))
                .await
                .map_err(std::io::Error::other)
                .and_then(|r| r.map_err(std::io::Error::from));
            match read {
                // A file that shrank since its stat: stop rather than spin.
                Ok(bytes) if bytes.is_empty() => Some((
                    Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof)),
                    end,
                )),
                Ok(bytes) => {
                    let next = offset + bytes.len() as u64;
                    Some((Ok(bytes), next))
                }
                Err(e) => Some((Err(e), end)),
            }
        }
    });
    Body::from_stream(stream.map(|chunk| chunk.map(axum::body::Bytes::from)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mbr_core::vault::MemVault;

    fn range_header(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::RANGE, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn range_grammar_matches_serve_file() {
        let none = HeaderMap::new();
        assert_eq!(requested_range(&none, 10), RequestedRange::Whole);
        let r = |v: &str| requested_range(&range_header(v), 1000);
        assert_eq!(r("bytes=0-99"), RequestedRange::Partial(0..=99));
        assert_eq!(r("bytes=900-"), RequestedRange::Partial(900..=999));
        assert_eq!(r("bytes=-100"), RequestedRange::Partial(900..=999));
        // An end past the file is clamped, as RFC 9110 says.
        assert_eq!(r("bytes=990-5000"), RequestedRange::Partial(990..=999));
        assert_eq!(r("bytes=1000-"), RequestedRange::Unsatisfiable);
        assert_eq!(r("bytes=-2000"), RequestedRange::Unsatisfiable);
        assert_eq!(r("bytes=50-10"), RequestedRange::Unsatisfiable);
        assert_eq!(r("items=0-1"), RequestedRange::Unsatisfiable);
        assert_eq!(r("bytes=abc"), RequestedRange::Unsatisfiable);
        assert_eq!(r("bytes=0-1,5-6"), RequestedRange::Unsatisfiable);
        assert_eq!(
            requested_range(&range_header("bytes=0-0"), 0),
            RequestedRange::Unsatisfiable
        );
    }

    async fn body_of(response: Response<Body>) -> Vec<u8> {
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    fn vault_with(bytes: &[u8]) -> (Arc<dyn Vault>, VaultPath) {
        let vault: Arc<dyn Vault> = Arc::new(MemVault::new().with_file("v.bin", bytes));
        (vault, VaultPath::new("v.bin").unwrap())
    }

    #[tokio::test]
    async fn whole_partial_and_refused_responses() {
        let data: Vec<u8> = (0..=255u8).cycle().take(CHUNK_BYTES * 2 + 17).collect();
        let size = data.len() as u64;
        let (vault, path) = vault_with(&data);
        let respond = |headers: HeaderMap| {
            vault_file_response(
                Arc::clone(&vault),
                path.clone(),
                size,
                "video/mp4",
                Some("Thu, 01 Jan 1970 00:00:00 GMT".into()),
                &headers,
                false,
            )
        };

        let whole = respond(HeaderMap::new());
        assert_eq!(whole.status(), StatusCode::OK);
        assert_eq!(whole.headers()[header::ACCEPT_RANGES], "bytes");
        assert_eq!(whole.headers()[header::CONTENT_LENGTH], size.to_string());
        assert_eq!(whole.headers()[header::CONTENT_TYPE], "video/mp4");
        assert!(whole.headers().contains_key(header::LAST_MODIFIED));
        assert_eq!(body_of(whole).await, data, "streamed across chunks");

        let first = respond(range_header("bytes=0-99"));
        assert_eq!(first.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            first.headers()[header::CONTENT_RANGE],
            format!("bytes 0-99/{size}")
        );
        assert_eq!(first.headers()[header::CONTENT_LENGTH], "100");
        assert_eq!(body_of(first).await, data[..100]);

        let open = respond(range_header(&format!("bytes={}-", CHUNK_BYTES - 3)));
        assert_eq!(open.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(body_of(open).await, data[CHUNK_BYTES - 3..]);

        let suffix = respond(range_header("bytes=-10"));
        assert_eq!(
            suffix.headers()[header::CONTENT_RANGE],
            format!("bytes {}-{}/{size}", size - 10, size - 1)
        );
        assert_eq!(body_of(suffix).await, data[data.len() - 10..]);

        for bad in [format!("bytes={size}-"), "bytes=x-y".to_string()] {
            let refused = respond(range_header(&bad));
            assert_eq!(refused.status(), StatusCode::RANGE_NOT_SATISFIABLE, "{bad}");
            assert_eq!(
                refused.headers()[header::CONTENT_RANGE],
                format!("bytes */{size}")
            );
        }
    }

    #[tokio::test]
    async fn head_and_empty_files_have_no_body() {
        let (vault, path) = vault_with(b"hello");
        let head = vault_file_response(
            Arc::clone(&vault),
            path.clone(),
            5,
            "text/plain",
            None,
            &HeaderMap::new(),
            true,
        );
        assert_eq!(head.headers()[header::CONTENT_LENGTH], "5");
        assert!(body_of(head).await.is_empty());

        let (empty, empty_path) = vault_with(b"");
        let response = vault_file_response(
            empty,
            empty_path,
            0,
            "text/plain",
            None,
            &HeaderMap::new(),
            false,
        );
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_LENGTH], "0");
        assert!(body_of(response).await.is_empty());
    }
}

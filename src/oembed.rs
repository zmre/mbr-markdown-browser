use crate::errors::OembedError;
use crate::media::MediaEmbed;
use futures::StreamExt;
use regex::Regex;
use reqwest::Client;
use scraper::{ElementRef, Html, Selector};
use std::error::Error;
use std::net::IpAddr;
use std::sync::LazyLock;
use std::time::Duration;
use url::Url;

/// Maximum bytes of a remote page body to read when extracting metadata.
/// All metadata we care about lives in `<head>`, which fits comfortably here.
const MAX_OEMBED_BODY_BYTES: usize = 512 * 1024;

/// Maximum number of HTTP redirects to follow when fetching page metadata.
const MAX_REDIRECTS: usize = 5;

/// Shared HTTP client for oembed fetches, built exactly once.
///
/// Constructing a client rebuilds the webpki root certificate store (expensive)
/// and discards the connection pool, so we share a single instance across every
/// fetch. Redirects are disabled so that every hop can be validated against
/// `is_public_ip` (SSRF protection); `fetch_page_info` follows redirects
/// manually. No timeout is baked in — it varies by mode (500ms server, disabled
/// in build), so it is applied per request via `RequestBuilder::timeout`.
static OEMBED_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    crate::http_client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("failed to build HTTP client")
});

/// Returns true if the IP address is publicly routable. Used to prevent SSRF:
/// oembed must never fetch loopback, private, or link-local addresses (e.g.
/// cloud metadata endpoints like 169.254.169.254).
fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !(v4.is_loopback()
                || v4.is_unspecified()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast())
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            // IPv4-mapped addresses (e.g. ::ffff:127.0.0.1) inherit IPv4 rules
            Some(mapped) => is_public_ip(IpAddr::V4(mapped)),
            None => {
                !(v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_unique_local()
                    || v6.is_unicast_link_local())
            }
        },
    }
}

/// Validate that a URL's target passes the given IP check before fetching.
/// IP-literal hosts are checked directly; domain hosts are resolved via DNS
/// and EVERY resolved address must pass (conservative vs. split-horizon DNS).
async fn check_url_target<F: Fn(IpAddr) -> bool>(url: &Url, check: &F) -> Result<(), OembedError> {
    fn disallowed(detail: String) -> OembedError {
        OembedError::DisallowedAddress { detail }
    }
    let host = url.host().ok_or_else(|| OembedError::NoHost {
        url: url.to_string(),
    })?;
    match host {
        url::Host::Ipv4(ip) => check(IpAddr::V4(ip))
            .then_some(())
            .ok_or_else(|| disallowed(ip.to_string())),
        url::Host::Ipv6(ip) => check(IpAddr::V6(ip))
            .then_some(())
            .ok_or_else(|| disallowed(ip.to_string())),
        url::Host::Domain(domain) => {
            let port = url
                .port_or_known_default()
                .ok_or_else(|| OembedError::NoKnownPort {
                    url: url.to_string(),
                })?;
            let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((domain, port))
                .await
                .map_err(|source| OembedError::DnsResolveFailed {
                    domain: domain.to_string(),
                    source,
                })?
                .collect();
            if addrs.is_empty() {
                return Err(OembedError::NoAddressesResolved {
                    domain: domain.to_string(),
                });
            }
            addrs
                .iter()
                .all(|addr| check(addr.ip()))
                .then_some(())
                .ok_or_else(|| disallowed(format!("resolved for host {domain}")))
        }
    }
}

/// Formats an error with its full source chain for detailed logging.
/// Walks the `source()` chain to reveal nested errors (e.g., reqwest -> hyper -> io).
fn format_error_chain(err: &dyn Error) -> String {
    let mut chain = vec![err.to_string()];
    let mut current = err.source();
    while let Some(source) = current {
        chain.push(source.to_string());
        current = source.source();
    }
    chain.join(" -> ")
}

// Precompiled selectors for efficient HTML parsing
// All metadata lives in <head>, so we scope our searches there
static HEAD_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("head").expect("Invalid head selector"));
static TITLE_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("title").expect("Invalid title selector"));

// OpenGraph meta tags (priority sources)
static OG_TITLE_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse("meta[property='og:title']").expect("Invalid og:title selector")
});
static OG_IMAGE_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse("meta[property='og:image']").expect("Invalid og:image selector")
});
static OG_DESCRIPTION_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse("meta[property='og:description']").expect("Invalid og:description selector")
});

// Fallback selectors
static META_DESCRIPTION_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse("meta[name='description']").expect("Invalid meta description selector")
});
static FAVICON_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("link[rel='icon']").expect("Invalid favicon selector"));
static ALT_FAVICON_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse("link[rel='alternate icon']").expect("Invalid alternate favicon selector")
});

/// Supported image types for favicons
const SUPPORTED_FAVICON_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/jpg",
    "image/svg+xml",
    "image/x-icon",
    "image/vnd.microsoft.icon",
];

/// Check if a favicon type is supported (or if no type is specified, assume supported)
fn is_supported_favicon_type(type_attr: Option<&str>) -> bool {
    type_attr.is_none_or(|t| SUPPORTED_FAVICON_TYPES.contains(&t))
}

/// Extract content attribute from first matching element, HTML-escaped
fn extract_content(head: &ElementRef, selector: &Selector) -> Option<String> {
    head.select(selector)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(|s| html_escape::encode_text(s).to_string())
}

/// Extract content attribute from first matching element, RAW (unescaped).
/// Used for values rendered into attribute contexts, where escaping happens
/// at render time to avoid double-escaping.
fn extract_content_raw(head: &ElementRef, selector: &Selector) -> Option<String> {
    head.select(selector)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(String::from)
}

/// Extract favicon href from first matching element with a supported image type
fn extract_favicon(head: &ElementRef, selector: &Selector) -> Option<String> {
    head.select(selector)
        .find(|el| is_supported_favicon_type(el.value().attr("type")))
        .and_then(|el| el.value().attr("href"))
        .map(|s| s.to_string())
}

// Giphy regex patterns - compiled once for efficiency
static GIPHY_MEDIA_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://(?:media\d*|i)\.giphy\.com/").unwrap());
static GIPHY_PAGE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:www\.)?giphy\.com/gifs/(?:[^/]+-)?([a-zA-Z0-9]+)(?:\?.*)?$").unwrap()
});

// GIST regex patterns
static GIST_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://gist\.github\.com/").unwrap());

#[derive(Default, Clone)]
pub struct PageInfo {
    pub url: String,
    /// Page title, stored text-escaped (safe for element text context only)
    pub title: Option<String>,
    /// Page description, stored text-escaped (safe for element text context only)
    pub description: Option<String>,
    /// Image/favicon URL, stored RAW; escaped at render time for attribute context
    pub image: Option<String>,
    pub embed_html: Option<String>, // For YouTube and other embeddable content
}

impl PageInfo {
    /// Estimates the memory size of this PageInfo in bytes.
    ///
    /// Used by OembedCache for size-based eviction. This is an approximation
    /// that accounts for the struct overhead plus the heap-allocated strings.
    pub fn estimated_size(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.url.len()
            + self.title.as_ref().map_or(0, |s| s.len())
            + self.description.as_ref().map_or(0, |s| s.len())
            + self.image.as_ref().map_or(0, |s| s.len())
            + self.embed_html.as_ref().map_or(0, |s| s.len())
    }

    /// Check if URL is a Giphy URL and create embed HTML
    /// Matches patterns like:
    /// - https://media.giphy.com/media/ID/giphy.gif
    /// - https://media1.giphy.com/media/.../giphy.gif (or .webp)
    /// - https://giphy.com/gifs/name-ID
    /// - https://i.giphy.com/ID.gif
    fn giphy_embed(url: &str) -> Option<String> {
        // Pattern for direct media URLs (media.giphy.com, media1.giphy.com, i.giphy.com)
        // These are direct image files - embed as img tags
        if GIPHY_MEDIA_RE.is_match(url) {
            return Some(format!(
                r#"<figure class="giphy-embed">
                    <img src="{}" alt="Giphy animation" loading="lazy" />
                </figure>"#,
                url
            ));
        }

        // Pattern for giphy.com/gifs/... URLs - extract the ID and convert to media URL
        // Format: https://giphy.com/gifs/description-ID or https://giphy.com/gifs/ID
        if let Some(caps) = GIPHY_PAGE_RE.captures(url)
            && let Some(id) = caps.get(1)
        {
            let gif_url = format!("https://media.giphy.com/media/{}/giphy.gif", id.as_str());
            return Some(format!(
                r#"<figure class="giphy-embed">
                    <img src="{}" alt="Giphy animation" loading="lazy" />
                </figure>"#,
                gif_url
            ));
        }

        None
    }

    /// Check if URL is a gist URL and create embed HTML
    /// Matches patterns like:
    /// - https://gist.github.com/rpinna/b97f8505940f255e8ebbd9a17c76f3ea
    /// - https://gist.github.com/rpinna/b97f8505940f255e8ebbd9a17c76f3ea#file-order-ts
    fn gist_embed(url: &str) -> Option<String> {
        if GIST_RE.is_match(url) {
            // Handle fragment - .js must come before #fragment
            let script_url = if let Some(hash_pos) = url.find('#') {
                let (path, fragment) = url.split_at(hash_pos);
                format!("{}.js{}", path, fragment)
            } else {
                format!("{}.js", url)
            };
            return Some(format!(r#"<script src="{}"></script>"#, script_url));
        }
        None
    }

    pub async fn new_from_url(url: &str, timeout_ms: u64) -> Result<Self, OembedError> {
        // If timeout is 0, oembed is disabled - return a plain link without network call or substitution
        if timeout_ms == 0 {
            // tracing::debug!("Oembed disabled, ignoring url {}", &url);
            return Ok(PageInfo {
                url: url.to_string(),
                ..Default::default()
            });
        }

        // Check for Giphy - embed directly without fetching
        if let Some(embed_html) = Self::giphy_embed(url) {
            return Ok(PageInfo {
                url: url.to_string(),
                embed_html: Some(embed_html),
                ..Default::default()
            });
        }

        // Check for GitHub gist - embed without fetching
        if let Some(embed_html) = Self::gist_embed(url) {
            return Ok(PageInfo {
                url: url.to_string(),
                embed_html: Some(embed_html),
                ..Default::default()
            });
        }

        // Check for media extension (video/audio/PDF) - embed without fetching
        // Parameters: server_mode=false (safe default for static builds),
        // transcode_enabled=true, hls_enabled=false
        if let Some(media) = MediaEmbed::from_bare_url(url) {
            return Ok(PageInfo {
                url: url.to_string(),
                embed_html: Some(media.to_html(false, true, false)),
                ..Default::default()
            });
        }

        // For other URLs, fetch and parse OpenGraph metadata. The shared client
        // carries no baked timeout; apply this request's timeout per hop.
        // (timeout_ms is guaranteed > 0 here: the 0 case returned above.)
        match Self::fetch_page_info(url, Duration::from_millis(timeout_ms)).await {
            Ok(info) => Ok(info),
            Err(e) => {
                // Any error (timeout, network, etc.) - return a plain link
                tracing::warn!(
                    "Error fetching URL ({}) for enriched display: {}",
                    &url,
                    format_error_chain(&e)
                );
                Ok(PageInfo {
                    url: url.to_string(),
                    ..Default::default()
                })
            }
        }
    }

    /// Fetch page metadata with SSRF protections: a public-IP check on every
    /// hop, manual redirect following (max [`MAX_REDIRECTS`]), a content-type
    /// check, and a body size cap of [`MAX_OEMBED_BODY_BYTES`].
    async fn fetch_page_info(url: &str, timeout: Duration) -> Result<Self, OembedError> {
        Self::fetch_page_info_inner(url, timeout, is_public_ip).await
    }

    /// Inner fetch with an injectable IP check so tests can exercise the
    /// redirect and body handling against loopback mock servers.
    ///
    /// Uses the shared [`OEMBED_CLIENT`] and applies `timeout` per request, so
    /// the timeout bounds each redirect hop individually (matching the previous
    /// per-client timeout semantics).
    async fn fetch_page_info_inner<F: Fn(IpAddr) -> bool>(
        url: &str,
        timeout: Duration,
        check: F,
    ) -> Result<Self, OembedError> {
        let mut current_url = Url::parse(url)?;
        let mut redirects = 0;
        let response = loop {
            if !matches!(current_url.scheme(), "http" | "https") {
                return Err(OembedError::NonHttpScheme {
                    url: current_url.to_string(),
                });
            }
            check_url_target(&current_url, &check).await?;

            // Security: reqwest re-resolves DNS when sending the request, so a
            // malicious DNS server could return a public address to our check
            // and a private one to reqwest (DNS-rebinding TOCTOU). We accept
            // this residual risk: mbr is a local previewer, exploiting the gap
            // requires an attacker-controlled rebinding DNS server, and closing
            // it would require pinning resolved IPs via a custom connector.
            let response = OEMBED_CLIENT
                .get(current_url.clone())
                .timeout(timeout)
                .send()
                .await?;

            let status = response.status().as_u16();
            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                if redirects >= MAX_REDIRECTS {
                    return Err(OembedError::TooManyRedirects {
                        max: MAX_REDIRECTS,
                        url: url.to_string(),
                    });
                }
                redirects += 1;
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .ok_or_else(|| OembedError::RedirectWithoutLocation {
                        url: current_url.to_string(),
                    })?
                    .to_str()
                    .map_err(|_| OembedError::InvalidLocationHeader {
                        url: current_url.to_string(),
                    })?;
                // Resolve relative redirects against the current URL
                current_url = current_url.join(location)?;
                continue;
            }
            break response;
        };

        // Check for Cloudflare challenge block
        if response
            .headers()
            .get("cf-mitigated")
            .is_some_and(|v| v == "challenge")
        {
            return Err(OembedError::CloudflareChallenge);
        }

        // Only parse HTML responses. A missing Content-Type is tolerated.
        if let Some(content_type) = response.headers().get(reqwest::header::CONTENT_TYPE) {
            let content_type = content_type.to_str().unwrap_or_default();
            if !content_type.contains("text/html")
                && !content_type.contains("application/xhtml+xml")
            {
                return Err(OembedError::NonHtmlContentType {
                    content_type: content_type.to_string(),
                    url: current_url.to_string(),
                });
            }
        }

        // Stream the body up to MAX_OEMBED_BODY_BYTES, then stop polling.
        // Truncation (not an error): all metadata lives in <head>, which fits
        // well within the cap, and Html::parse_document is lenient.
        let mut buf: Vec<u8> = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let remaining = MAX_OEMBED_BODY_BYTES - buf.len();
            if chunk.len() >= remaining {
                buf.extend_from_slice(&chunk[..remaining]);
                break;
            }
            buf.extend_from_slice(&chunk);
        }
        // Lossy UTF-8 conversion: we give up charset detection for non-UTF-8
        // pages, which may garble some metadata. That is acceptable graceful
        // degradation for a best-effort link preview.
        let body = String::from_utf8_lossy(&buf);
        let document = Html::parse_document(&body);

        // All metadata lives in <head> - extract with og:* priority, then fallbacks
        let (title, description, image) = document
            .select(&HEAD_SELECTOR)
            .next()
            .map(|head| {
                // Priority: og:* tags first
                let og_title = extract_content(&head, &OG_TITLE_SELECTOR);
                let og_desc = extract_content(&head, &OG_DESCRIPTION_SELECTOR);
                // Raw: image is only rendered in attribute context, escaped there
                let og_image = extract_content_raw(&head, &OG_IMAGE_SELECTOR);

                // Fallbacks only computed if og:* not found
                let title = og_title.or_else(|| {
                    head.select(&TITLE_SELECTOR)
                        .next()
                        .map(|el| el.text().collect::<String>())
                        .filter(|s| !s.is_empty())
                        .map(|s| html_escape::encode_text(&s).to_string())
                });

                let description =
                    og_desc.or_else(|| extract_content(&head, &META_DESCRIPTION_SELECTOR));

                let image = og_image
                    .or_else(|| extract_favicon(&head, &FAVICON_SELECTOR))
                    .or_else(|| extract_favicon(&head, &ALT_FAVICON_SELECTOR));

                (title, description, image)
            })
            .unwrap_or((None, None, None));

        Ok(PageInfo {
            url: url.to_string(),
            title,
            description,
            image,
            embed_html: None,
        })
    }

    pub fn text(&self) -> String {
        format!(
            "{}: {}",
            self.title.clone().unwrap_or("no title".to_string()),
            self.url
        )
    }

    pub fn html(&self) -> String {
        // If we have embed HTML (e.g., YouTube), use that
        if let Some(embed) = &self.embed_html {
            return embed.clone();
        }

        // Otherwise, create a link (TODO: make this a rich card with image/description)
        // Attribute values (image src, href) are escaped for double-quoted
        // attribute context here at render time; title/description are stored
        // text-escaped at extraction time (element text context).
        if let Some(title) = self.title.clone() {
            let img_tag = self
                .image
                .as_ref()
                .map(|src| {
                    format!(
                        "<img src=\"{}\"/>",
                        html_escape::encode_double_quoted_attribute(src)
                    )
                })
                .unwrap_or_default();
            format!(
                "<article class='mbr-social-link-box'>
                    {}
                    <a href=\"{}\" class='mbr-social-link'>
                        <header>{}</header>
                        <p>{}</p>
                    </a>
                </article>
                ",
                img_tag,
                html_escape::encode_double_quoted_attribute(&self.url),
                title,
                self.description.as_deref().unwrap_or(""),
            )
        } else {
            format!(
                "<a href=\"{}\">{}</a>",
                html_escape::encode_double_quoted_attribute(&self.url),
                html_escape::encode_text(&self.url),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Spawn a minimal HTTP/1.1 mock server on 127.0.0.1:0. The response is
    /// built from the bound address (so redirect targets can reference the
    /// server itself) and served identically for every connection. Returns
    /// the bound address and a counter of received requests.
    async fn spawn_mock_server(
        build_response: impl FnOnce(SocketAddr) -> String,
    ) -> (SocketAddr, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let response = build_response(addr);
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_counter = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                hits_counter.fetch_add(1, Ordering::SeqCst);
                let response = response.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    // A single read suffices for the small GET requests we send
                    let _ = stream.read(&mut buf).await;
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        (addr, hits)
    }

    fn html_response(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
    }

    #[test]
    fn test_is_public_ip() {
        for ip in [
            "127.0.0.1",
            "0.0.0.0",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "::1",
            "::",
            "fc00::1",
            "fe80::1",
            "::ffff:10.0.0.1",
        ] {
            assert!(
                !is_public_ip(ip.parse().unwrap()),
                "{ip} should be rejected"
            );
        }
        for ip in ["93.184.216.34", "172.32.0.1", "2606:2800::1"] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip} should be accepted");
        }
    }

    #[tokio::test]
    async fn test_fetch_rejects_loopback_literal() {
        let (addr, hits) =
            spawn_mock_server(|_| html_response("<html><head><title>x</title></head></html>"))
                .await;
        let url = format!("http://127.0.0.1:{}/", addr.port());
        let result = PageInfo::fetch_page_info(&url, Duration::from_secs(2)).await;
        assert!(result.is_err());
        // The check runs before connecting, so the mock never sees a request
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_fetch_rejects_private_literal_without_network() {
        let result = PageInfo::fetch_page_info("http://192.168.0.1/", Duration::from_secs(2)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_rejects_localhost_hostname() {
        // Rejected via DNS resolution: localhost resolves to loopback addresses
        let result = PageInfo::fetch_page_info("http://localhost:9/", Duration::from_secs(2)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_follows_redirect_and_checks_every_hop() {
        // Mock A redirects to a second loopback address (127.0.0.2). The
        // checker allows only mock A's address, so the second hop is rejected.
        let (addr, hits) = spawn_mock_server(|_| {
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.2:9/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_string()
        })
        .await;
        let url = format!("http://127.0.0.1:{}/", addr.port());
        let result = PageInfo::fetch_page_info_inner(&url, Duration::from_secs(2), |ip| {
            ip == IpAddr::V4(Ipv4Addr::LOCALHOST)
        })
        .await;
        assert!(result.is_err());
        // The first hop was fetched; the redirect target was rejected pre-connect
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_fetch_redirect_limit() {
        // Mock redirects to itself forever; must fail after MAX_REDIRECTS hops
        let (addr, _hits) = spawn_mock_server(|addr| {
            format!(
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{}/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                addr.port()
            )
        })
        .await;
        let url = format!("http://127.0.0.1:{}/", addr.port());
        let result = PageInfo::fetch_page_info_inner(&url, Duration::from_secs(5), |_| true).await;
        let err = result.err().expect("must fail after redirect limit");
        assert!(err.to_string().contains("redirect"));
    }

    #[tokio::test]
    async fn test_fetch_truncates_large_body() {
        // Body far exceeds MAX_OEMBED_BODY_BYTES; the <head> metadata still
        // parses because we truncate rather than error
        let body = format!(
            "<html><head><title>Big</title></head><body>{}</body></html>",
            "x".repeat(1_100_000)
        );
        let (addr, _hits) = spawn_mock_server(move |_| html_response(&body)).await;
        let url = format!("http://127.0.0.1:{}/", addr.port());
        let info = PageInfo::fetch_page_info_inner(&url, Duration::from_secs(5), |_| true)
            .await
            .unwrap();
        assert_eq!(info.title.as_deref(), Some("Big"));
    }

    #[tokio::test]
    async fn test_fetch_rejects_non_html_content_type() {
        let (addr, _hits) = spawn_mock_server(|_| {
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabcd"
                .to_string()
        })
        .await;
        let url = format!("http://127.0.0.1:{}/", addr.port());
        let result = PageInfo::fetch_page_info_inner(&url, Duration::from_secs(2), |_| true).await;
        let err = result.err().expect("must reject non-HTML content type");
        assert!(err.to_string().contains("non-HTML"));
    }

    #[tokio::test]
    async fn test_fetch_allows_missing_content_type() {
        let body = "<html><head><title>NoCT</title></head><body></body></html>";
        let (addr, _hits) = spawn_mock_server(|_| {
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
        })
        .await;
        let url = format!("http://127.0.0.1:{}/", addr.port());
        let info = PageInfo::fetch_page_info_inner(&url, Duration::from_secs(2), |_| true)
            .await
            .unwrap();
        assert_eq!(info.title.as_deref(), Some("NoCT"));
    }

    #[tokio::test]
    async fn test_shared_client_reused_across_fetches() {
        // The oembed client must be a single shared instance so the webpki root
        // store is built once and connections pool across fetches. Every deref
        // of the LazyLock yields the same instance.
        let first: *const Client = &*OEMBED_CLIENT;
        let second: *const Client = &*OEMBED_CLIENT;
        assert!(
            std::ptr::eq(first, second),
            "OEMBED_CLIENT must be a single shared instance"
        );

        // Two sequential fetches both succeed through the shared client, and the
        // per-request timeout still applies (a non-trivial timeout is passed).
        let (addr, hits) =
            spawn_mock_server(|_| html_response("<html><head><title>Shared</title></head></html>"))
                .await;
        let url = format!("http://127.0.0.1:{}/", addr.port());
        for _ in 0..2 {
            let info = PageInfo::fetch_page_info_inner(&url, Duration::from_secs(2), |_| true)
                .await
                .unwrap();
            assert_eq!(info.title.as_deref(), Some("Shared"));
        }
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_giphy_media_url() {
        let url = "https://media1.giphy.com/media/v1.Y2lkPTc5MGI3NjExaThmbzNhb3duNGM5NGVxemo4aHlnYTM1YXA4cGxmc2l2ejdjc2s4ZCZlcD12MV9pbnRlcm5hbF9naWZfYnlfaWQmY3Q9Zw/CAxbo8KC2A0y4/giphy.gif";
        let embed = PageInfo::giphy_embed(url);
        assert!(embed.is_some());
        let html = embed.unwrap();
        assert!(html.contains("giphy-embed"));
        assert!(html.contains(url));
    }

    #[test]
    fn test_giphy_media_url_without_version() {
        let url = "https://media.giphy.com/media/CAxbo8KC2A0y4/giphy.gif";
        let embed = PageInfo::giphy_embed(url);
        assert!(embed.is_some());
        let html = embed.unwrap();
        assert!(html.contains("giphy-embed"));
        assert!(html.contains(url));
    }

    #[test]
    fn test_giphy_i_url() {
        let url = "https://i.giphy.com/CAxbo8KC2A0y4.gif";
        let embed = PageInfo::giphy_embed(url);
        assert!(embed.is_some());
        let html = embed.unwrap();
        assert!(html.contains("giphy-embed"));
        assert!(html.contains(url));
    }

    #[test]
    fn test_giphy_page_url() {
        let url = "https://giphy.com/gifs/cat-funny-CAxbo8KC2A0y4";
        let embed = PageInfo::giphy_embed(url);
        assert!(embed.is_some());
        let html = embed.unwrap();
        assert!(html.contains("giphy-embed"));
        assert!(html.contains("https://media.giphy.com/media/CAxbo8KC2A0y4/giphy.gif"));
    }

    #[test]
    fn test_giphy_page_url_simple() {
        let url = "https://giphy.com/gifs/CAxbo8KC2A0y4";
        let embed = PageInfo::giphy_embed(url);
        assert!(embed.is_some());
        let html = embed.unwrap();
        assert!(html.contains("https://media.giphy.com/media/CAxbo8KC2A0y4/giphy.gif"));
    }

    #[test]
    fn test_non_giphy_url() {
        let url = "https://example.com/image.gif";
        let embed = PageInfo::giphy_embed(url);
        assert!(embed.is_none());
    }

    #[test]
    fn test_youtube_not_giphy() {
        let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
        let embed = PageInfo::giphy_embed(url);
        assert!(embed.is_none());
    }

    #[tokio::test]
    async fn test_zero_timeout_returns_plain_link() {
        // With timeout=0, should return plain link without network call
        let url = "https://example.com/some-page";
        let result = PageInfo::new_from_url(url, 0).await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.url, url);
        assert!(info.title.is_none()); // No network call, so no title fetched
        assert!(info.embed_html.is_none());
        // html() should return a plain link
        assert!(info.html().contains("<a href="));
        assert!(info.html().contains(url));
    }

    #[tokio::test]
    async fn test_zero_timeout_disables_youtube_embed() {
        // With timeout=0, ALL enrichment is disabled - even no-network embeds like YouTube
        let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
        let result = PageInfo::new_from_url(url, 0).await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert!(info.embed_html.is_none());
        // Should return a plain link instead
        assert!(info.html().contains("<a href="));
    }

    #[tokio::test]
    async fn test_zero_timeout_disables_giphy_embed() {
        // With timeout=0, ALL enrichment is disabled - even no-network embeds like Giphy
        let url = "https://media.giphy.com/media/CAxbo8KC2A0y4/giphy.gif";
        let result = PageInfo::new_from_url(url, 0).await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert!(info.embed_html.is_none());
        // Should return a plain link instead
        assert!(info.html().contains("<a href="));
    }

    #[test]
    fn test_estimated_size_empty() {
        let info = PageInfo::default();
        // Should at least include the struct size
        assert!(info.estimated_size() >= std::mem::size_of::<PageInfo>());
    }

    #[test]
    fn test_estimated_size_with_fields() {
        let info = PageInfo {
            url: "https://example.com".to_string(),
            title: Some("Test Title".to_string()),
            description: Some("A longer description text".to_string()),
            image: Some("https://example.com/image.png".to_string()),
            embed_html: None,
        };
        let size = info.estimated_size();
        // Size should include all the string lengths
        let expected_min = std::mem::size_of::<PageInfo>()
            + info.url.len()
            + info.title.as_ref().unwrap().len()
            + info.description.as_ref().unwrap().len()
            + info.image.as_ref().unwrap().len();
        assert_eq!(size, expected_min);
    }

    #[test]
    fn test_clone() {
        let info = PageInfo {
            url: "https://example.com".to_string(),
            title: Some("Test".to_string()),
            description: None,
            image: None,
            embed_html: Some("<div></div>".to_string()),
        };
        let cloned = info.clone();
        assert_eq!(cloned.url, info.url);
        assert_eq!(cloned.title, info.title);
        assert_eq!(cloned.embed_html, info.embed_html);
    }

    #[test]
    fn test_gist_embed_basic() {
        let url = "https://gist.github.com/rpinna/b97f8505940f255e8ebbd9a17c76f3ea";
        let embed = PageInfo::gist_embed(url);
        assert!(embed.is_some());
        let html = embed.unwrap();
        assert!(html.contains(
            r#"<script src="https://gist.github.com/rpinna/b97f8505940f255e8ebbd9a17c76f3ea.js"></script>"#
        ));
    }

    #[test]
    fn test_gist_embed_with_fragment() {
        let url = "https://gist.github.com/rpinna/b97f8505940f255e8ebbd9a17c76f3ea#file-order-ts";
        let embed = PageInfo::gist_embed(url);
        assert!(embed.is_some());
        let html = embed.unwrap();
        // .js should come BEFORE the fragment
        assert!(html.contains(
            r#"<script src="https://gist.github.com/rpinna/b97f8505940f255e8ebbd9a17c76f3ea.js#file-order-ts"></script>"#
        ));
    }

    #[test]
    fn test_gist_embed_http() {
        let url = "http://gist.github.com/user/abc123";
        let embed = PageInfo::gist_embed(url);
        assert!(embed.is_some());
    }

    #[test]
    fn test_gist_non_gist_url() {
        let url = "https://github.com/user/repo";
        let embed = PageInfo::gist_embed(url);
        assert!(embed.is_none());
    }

    #[test]
    fn test_gist_not_youtube() {
        let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
        let embed = PageInfo::gist_embed(url);
        assert!(embed.is_none());
    }

    // Bare media URL tests
    #[tokio::test]
    async fn test_bare_mp4_url_returns_video_embed() {
        let url = "https://example.com/videos/demo.mp4";
        let result = PageInfo::new_from_url(url, 500).await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert!(info.embed_html.is_some());
        let html = info.embed_html.unwrap();
        assert!(html.contains("<video"));
        assert!(html.contains(url));
    }

    #[tokio::test]
    async fn test_bare_mp3_url_returns_audio_embed() {
        let url = "https://example.com/audio/podcast.mp3";
        let result = PageInfo::new_from_url(url, 500).await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert!(info.embed_html.is_some());
        let html = info.embed_html.unwrap();
        assert!(html.contains("<audio"));
        assert!(html.contains(url));
    }

    #[tokio::test]
    async fn test_bare_pdf_url_returns_pdf_embed() {
        let url = "https://example.com/docs/report.pdf";
        let result = PageInfo::new_from_url(url, 500).await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert!(info.embed_html.is_some());
        let html = info.embed_html.unwrap();
        assert!(html.contains("pdf-embed"));
        assert!(html.contains(r#"type="application/pdf""#));
        assert!(html.contains(url));
    }

    #[tokio::test]
    async fn test_mp4_in_path_but_not_extension_goes_to_opengraph() {
        // URL has .mp4 in the path but not as the file extension
        // This should NOT match media detection and should proceed to OpenGraph
        let url = "https://example.com/videos/mp4-format/info";
        let result = PageInfo::new_from_url(url, 500).await;
        assert!(result.is_ok());
        let info = result.unwrap();
        // Should NOT have embed_html (or if it times out, it's a plain link)
        // The key is that it shouldn't detect as video
        if let Some(html) = &info.embed_html {
            assert!(!html.contains("<video"));
        }
    }

    #[tokio::test]
    async fn test_bare_m4v_url_returns_video_embed() {
        // .m4v is a supported video extension (unlike .mov which Vid doesn't detect)
        let url = "https://example.com/videos/clip.m4v";
        let result = PageInfo::new_from_url(url, 500).await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert!(info.embed_html.is_some());
        let html = info.embed_html.unwrap();
        assert!(html.contains("<video"));
    }

    #[tokio::test]
    async fn test_bare_wav_url_returns_audio_embed() {
        let url = "https://example.com/sounds/effect.wav";
        let result = PageInfo::new_from_url(url, 500).await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert!(info.embed_html.is_some());
        let html = info.embed_html.unwrap();
        assert!(html.contains("<audio"));
    }

    // Tests for format_error_chain
    #[test]
    fn test_format_error_chain_single() {
        let err = std::io::Error::other("simple error");
        let chain = format_error_chain(&err);
        assert_eq!(chain, "simple error");
    }

    #[test]
    fn test_format_error_chain_nested() {
        // Create a nested error chain using Box<dyn Error>
        #[derive(Debug)]
        struct OuterError {
            source: Box<dyn Error + Send + Sync>,
        }
        impl std::fmt::Display for OuterError {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "outer error")
            }
        }
        impl Error for OuterError {
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                Some(self.source.as_ref())
            }
        }

        let inner = std::io::Error::other("inner error");
        let outer = OuterError {
            source: Box::new(inner),
        };
        let chain = format_error_chain(&outer);
        assert!(chain.contains("outer error"));
        assert!(chain.contains("inner error"));
        assert!(chain.contains(" -> "));
    }

    // Tests for is_supported_favicon_type
    #[test]
    fn test_supported_favicon_type_none() {
        // No type attribute means assume supported
        assert!(is_supported_favicon_type(None));
    }

    #[test]
    fn test_supported_favicon_type_png() {
        assert!(is_supported_favicon_type(Some("image/png")));
    }

    #[test]
    fn test_supported_favicon_type_svg() {
        assert!(is_supported_favicon_type(Some("image/svg+xml")));
    }

    #[test]
    fn test_supported_favicon_type_ico() {
        assert!(is_supported_favicon_type(Some("image/x-icon")));
        assert!(is_supported_favicon_type(Some("image/vnd.microsoft.icon")));
    }

    #[test]
    fn test_unsupported_favicon_type() {
        assert!(!is_supported_favicon_type(Some("image/webp")));
        assert!(!is_supported_favicon_type(Some("text/html")));
    }

    // Tests for PageInfo::text()
    #[test]
    fn test_text_with_title() {
        let info = PageInfo {
            url: "https://example.com".to_string(),
            title: Some("My Title".to_string()),
            ..Default::default()
        };
        let text = info.text();
        assert_eq!(text, "My Title: https://example.com");
    }

    #[test]
    fn test_text_without_title() {
        let info = PageInfo {
            url: "https://example.com".to_string(),
            ..Default::default()
        };
        let text = info.text();
        assert_eq!(text, "no title: https://example.com");
    }

    // Tests for PageInfo::html()
    #[test]
    fn test_html_with_embed() {
        let info = PageInfo {
            url: "https://example.com".to_string(),
            embed_html: Some("<div>embedded</div>".to_string()),
            ..Default::default()
        };
        let html = info.html();
        assert_eq!(html, "<div>embedded</div>");
    }

    #[test]
    fn test_html_with_title_and_image() {
        let info = PageInfo {
            url: "https://example.com".to_string(),
            title: Some("My Title".to_string()),
            description: Some("My description".to_string()),
            image: Some("https://example.com/image.png".to_string()),
            embed_html: None,
        };
        let html = info.html();
        assert!(html.contains("mbr-social-link-box"));
        assert!(html.contains("My Title"));
        assert!(html.contains("My description"));
        assert!(html.contains(r#"<img src="https://example.com/image.png"/>"#));
    }

    #[test]
    fn test_html_with_title_no_image() {
        let info = PageInfo {
            url: "https://example.com".to_string(),
            title: Some("Just Title".to_string()),
            description: None,
            image: None,
            embed_html: None,
        };
        let html = info.html();
        assert!(html.contains("mbr-social-link-box"));
        assert!(html.contains("Just Title"));
        assert!(!html.contains("<img"));
    }

    #[test]
    fn test_html_no_title_plain_link() {
        let info = PageInfo {
            url: "https://example.com/page".to_string(),
            title: None,
            description: Some("ignored without title".to_string()),
            image: None,
            embed_html: None,
        };
        let html = info.html();
        // Should be a plain link
        assert!(html.contains(r#"<a href="https://example.com/page">"#));
        assert!(html.contains("https://example.com/page</a>"));
        assert!(!html.contains("mbr-social-link-box"));
    }

    #[test]
    fn test_html_image_with_quotes_escaped() {
        // Single-quote breakout attempt: harmless inside double-quoted attribute
        let info = PageInfo {
            url: "https://example.com".to_string(),
            title: Some("Title".to_string()),
            image: Some("x' onerror='alert(1)".to_string()),
            ..Default::default()
        };
        let html = info.html();
        // The whole payload stays inside the double-quoted src value
        assert!(html.contains(r#"<img src="x' onerror='alert(1)"/>"#));

        // Double-quote breakout attempt: quote must be escaped to &quot;
        let info = PageInfo {
            url: "https://example.com".to_string(),
            title: Some("Title".to_string()),
            image: Some(r#"x" onerror="alert(1)"#.to_string()),
            ..Default::default()
        };
        let html = info.html();
        assert!(html.contains("&quot;"));
        // No parseable onerror attribute: the closing quote never appears raw
        assert!(!html.contains(r#"" onerror=""#));
    }

    #[test]
    fn test_html_url_with_quote_escaped() {
        let evil_url = r#"https://example.com/"><script>alert(1)</script>"#;

        // Titled-card branch
        let info = PageInfo {
            url: evil_url.to_string(),
            title: Some("Title".to_string()),
            ..Default::default()
        };
        let html = info.html();
        assert!(!html.contains(r#""><script>"#));
        assert!(!html.contains("<script>"));

        // Plain-link branch
        let info = PageInfo {
            url: evil_url.to_string(),
            ..Default::default()
        };
        let html = info.html();
        assert!(!html.contains(r#""><script>"#));
        // Visible link text is text-escaped too
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn test_html_favicon_quote_breakout_escaped() {
        let info = PageInfo {
            url: "https://example.com".to_string(),
            title: Some("Title".to_string()),
            image: Some(r#"x.png"><script>alert(1)</script>"#.to_string()),
            ..Default::default()
        };
        let html = info.html();
        assert!(!html.contains("<script>"));
        assert!(html.contains("&quot;&gt;&lt;script&gt;"));
    }

    #[test]
    fn test_extract_og_image_is_raw() {
        // og:image containing & must be stored raw and escaped exactly once at render
        let doc = Html::parse_document(
            r#"<html><head>
                <meta property="og:title" content="T"/>
                <meta property="og:image" content="https://example.com/img?a=1&b=2"/>
            </head><body></body></html>"#,
        );
        let head = doc.select(&HEAD_SELECTOR).next().unwrap();
        let image = extract_content_raw(&head, &OG_IMAGE_SELECTOR).unwrap();
        // Stored raw: scraper decodes the attribute, no entity-encoding applied
        assert_eq!(image, "https://example.com/img?a=1&b=2");

        let info = PageInfo {
            url: "https://example.com".to_string(),
            title: Some("T".to_string()),
            image: Some(image),
            ..Default::default()
        };
        let html = info.html();
        // Escaped exactly once: &amp; not &amp;amp;
        assert!(html.contains(r#"<img src="https://example.com/img?a=1&amp;b=2"/>"#));
        assert!(!html.contains("&amp;amp;"));
    }

    // Tests for Giphy URL with query params
    #[test]
    fn test_giphy_page_url_with_query() {
        let url = "https://giphy.com/gifs/cat-funny-CAxbo8KC2A0y4?utm_source=test";
        let embed = PageInfo::giphy_embed(url);
        assert!(embed.is_some());
        let html = embed.unwrap();
        assert!(html.contains("https://media.giphy.com/media/CAxbo8KC2A0y4/giphy.gif"));
    }

    #[test]
    fn test_giphy_webp_extension() {
        let url = "https://media.giphy.com/media/CAxbo8KC2A0y4/giphy.webp";
        let embed = PageInfo::giphy_embed(url);
        assert!(embed.is_some());
        let html = embed.unwrap();
        assert!(html.contains(url));
    }
}

use crate::media::MediaEmbed;
use regex::Regex;
use reqwest::Client;
use scraper::{ElementRef, Html, Selector};
use std::error::Error;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::sync::LazyLock;
use std::time::Duration;
use url::Url;

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

/// Check if an IPv4 address is in a private/reserved range (SSRF protection)
fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_private()              // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
        || ip.is_loopback()      // 127.0.0.0/8
        || ip.is_link_local()    // 169.254.0.0/16
        || ip.is_broadcast()     // 255.255.255.255
        || ip.is_unspecified()   // 0.0.0.0
        || ip.is_documentation() // 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24
        || ip.octets()[0] == 0 // 0.0.0.0/8 (current network)
}

/// Check if an IPv6 address is in a private/reserved range (SSRF protection)
fn is_private_ipv6(ip: Ipv6Addr) -> bool {
    ip.is_loopback()       // ::1
        || ip.is_unspecified() // ::
        // Unique local addresses (fc00::/7) - check first byte
        || (ip.segments()[0] & 0xfe00) == 0xfc00
        // Link-local addresses (fe80::/10)
        || (ip.segments()[0] & 0xffc0) == 0xfe80
}

/// Check if an IP address is private/reserved (SSRF protection)
fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_ipv4(v4),
        IpAddr::V6(v6) => is_private_ipv6(v6),
    }
}

/// Check if a hostname is explicitly local (SSRF protection)
fn is_local_hostname(host: &str) -> bool {
    let host_lower = host.to_lowercase();
    host_lower == "localhost"
        || host_lower.ends_with(".localhost")
        || host_lower.ends_with(".local")
        || host_lower.ends_with(".internal")
        || host_lower.ends_with(".localdomain")
        || host_lower == "127.0.0.1"
        || host_lower == "::1"
        || host_lower == "[::1]"
        || host_lower.starts_with("192.168.")
        || host_lower.starts_with("10.")
        || host_lower.starts_with("172.16.")
        || host_lower.starts_with("172.17.")
        || host_lower.starts_with("172.18.")
        || host_lower.starts_with("172.19.")
        || host_lower.starts_with("172.20.")
        || host_lower.starts_with("172.21.")
        || host_lower.starts_with("172.22.")
        || host_lower.starts_with("172.23.")
        || host_lower.starts_with("172.24.")
        || host_lower.starts_with("172.25.")
        || host_lower.starts_with("172.26.")
        || host_lower.starts_with("172.27.")
        || host_lower.starts_with("172.28.")
        || host_lower.starts_with("172.29.")
        || host_lower.starts_with("172.30.")
        || host_lower.starts_with("172.31.")
}

/// Validate that a URL is safe to fetch (SSRF protection).
/// Returns Ok(()) if safe, Err with reason if blocked.
fn validate_url_for_ssrf(url_str: &str) -> Result<(), String> {
    let parsed = Url::parse(url_str).map_err(|e| format!("Invalid URL: {e}"))?;

    // Only allow http/https
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!("Blocked scheme: {scheme}"));
    }

    // Check hostname
    let host = parsed
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;

    // Check for obviously local hostnames
    if is_local_hostname(host) {
        return Err(format!("Blocked local hostname: {host}"));
    }

    // Try to parse as IP address directly
    if let Ok(ip) = host.parse::<IpAddr>()
        && is_private_ip(ip)
    {
        return Err(format!("Blocked private IP: {ip}"));
    }

    // For hostnames, resolve and check all IPs
    // Use port 80 for resolution (port doesn't affect DNS lookup)
    let socket_addr = format!("{host}:80");
    if let Ok(addrs) = socket_addr.to_socket_addrs() {
        for addr in addrs {
            if is_private_ip(addr.ip()) {
                return Err(format!(
                    "Hostname {host} resolves to private IP: {}",
                    addr.ip()
                ));
            }
        }
    }
    // Note: If DNS resolution fails, we allow the request to proceed.
    // The HTTP client will fail with a more descriptive error.

    Ok(())
}

#[derive(Default, Clone)]
pub struct PageInfo {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
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

    /// Extract YouTube video ID from various YouTube URL formats
    fn extract_youtube_id(url: &str) -> Option<String> {
        // Matches:
        // - https://www.youtube.com/watch?v=VIDEO_ID
        // - https://youtube.com/watch?v=VIDEO_ID
        // - https://youtu.be/VIDEO_ID
        // - https://www.youtube.com/embed/VIDEO_ID
        // - https://www.youtube.com/v/VIDEO_ID
        let patterns = [
            r"(?:youtube\.com/watch\?.*v=|youtu\.be/|youtube\.com/embed/|youtube\.com/v/)([a-zA-Z0-9_-]{11})",
        ];

        patterns.iter().find_map(|pattern| {
            Regex::new(pattern)
                .ok()
                .and_then(|re| re.captures(url))
                .and_then(|caps| caps.get(1))
                .map(|id| id.as_str().to_string())
        })
    }

    /// Check if URL is a YouTube URL and create embed HTML
    fn youtube_embed(url: &str) -> Option<String> {
        Self::extract_youtube_id(url).map(|video_id| {
            format!(
                r#"<figure class="video-embed youtube-embed">
                    <iframe
                        width="560"
                        height="315"
                        src="https://www.youtube.com/embed/{}"
                        title="YouTube video player"
                        frameborder="0"
                        allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"
                        referrerpolicy="strict-origin-when-cross-origin"
                        allowfullscreen>
                    </iframe>
                </figure>"#,
                video_id
            )
        })
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

    pub async fn new_from_url(
        url: &str,
        timeout_ms: u64,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // If timeout is 0, oembed is disabled - return a plain link without network call or substitution
        if timeout_ms == 0 {
            // tracing::debug!("Oembed disabled, ignoring url {}", &url);
            return Ok(PageInfo {
                url: url.to_string(),
                ..Default::default()
            });
        }

        // Check for YouTube first - no need to fetch the page
        if let Some(embed_html) = Self::youtube_embed(url) {
            return Ok(PageInfo {
                url: url.to_string(),
                embed_html: Some(embed_html),
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

        // SSRF protection: validate URL before making network request
        if let Err(reason) = validate_url_for_ssrf(url) {
            tracing::warn!("SSRF protection blocked URL ({}): {}", url, reason);
            return Ok(PageInfo {
                url: url.to_string(),
                ..Default::default()
            });
        }

        // Build a client with the configured timeout
        let client = Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()?;

        // For other URLs, fetch and parse OpenGraph metadata
        match Self::fetch_page_info(&client, url).await {
            Ok(info) => Ok(info),
            Err(e) => {
                // Any error (timeout, network, etc.) - return a plain link
                tracing::warn!(
                    "Error fetching URL ({}) for enriched display: {}",
                    &url,
                    format_error_chain(e.as_ref())
                );
                Ok(PageInfo {
                    url: url.to_string(),
                    ..Default::default()
                })
            }
        }
    }

    async fn fetch_page_info(
        client: &Client,
        url: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let response = client.get(url).send().await?;

        // Check for Cloudflare challenge block
        if response
            .headers()
            .get("cf-mitigated")
            .is_some_and(|v| v == "challenge")
        {
            return Err("Blocked by Cloudflare challenge".into());
        }

        let body = response.text().await?;
        let document = Html::parse_document(&body);

        // All metadata lives in <head> - extract with og:* priority, then fallbacks
        let (title, description, image) = document
            .select(&HEAD_SELECTOR)
            .next()
            .map(|head| {
                // Priority: og:* tags first
                let og_title = extract_content(&head, &OG_TITLE_SELECTOR);
                let og_desc = extract_content(&head, &OG_DESCRIPTION_SELECTOR);
                let og_image = extract_content(&head, &OG_IMAGE_SELECTOR);

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
        if let Some(title) = self.title.clone() {
            let img_tag = self
                .image
                .as_ref()
                .map(|src| format!("<img src='{}'/>", src))
                .unwrap_or_default();
            format!(
                "<article class='mbr-social-link-box'>
                    {}
                    <a href='{}' class='mbr-social-link'>
                        <header>{}</header>
                        <p>{}</p>
                    </a>    
                </article>
                ",
                img_tag,
                &self.url,
                title,
                self.description.as_deref().unwrap_or(""),
            )
        } else {
            format!("<a href='{}'>{}</a>", &self.url, &self.url,)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_youtube_embed() {
        let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
        let embed = PageInfo::youtube_embed(url);
        assert!(embed.is_some());
        let html = embed.unwrap();
        assert!(html.contains("youtube-embed"));
        assert!(html.contains("dQw4w9WgXcQ"));
    }

    #[test]
    fn test_youtube_short_url() {
        let url = "https://youtu.be/dQw4w9WgXcQ";
        let embed = PageInfo::youtube_embed(url);
        assert!(embed.is_some());
        let html = embed.unwrap();
        assert!(html.contains("dQw4w9WgXcQ"));
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

    // Tests for extract_youtube_id - additional patterns
    #[test]
    fn test_youtube_embed_url() {
        let url = "https://www.youtube.com/embed/dQw4w9WgXcQ";
        let id = PageInfo::extract_youtube_id(url);
        assert_eq!(id, Some("dQw4w9WgXcQ".to_string()));
    }

    #[test]
    fn test_youtube_v_url() {
        let url = "https://www.youtube.com/v/dQw4w9WgXcQ";
        let id = PageInfo::extract_youtube_id(url);
        assert_eq!(id, Some("dQw4w9WgXcQ".to_string()));
    }

    #[test]
    fn test_youtube_watch_with_extra_params() {
        let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s&list=PLtest";
        let id = PageInfo::extract_youtube_id(url);
        assert_eq!(id, Some("dQw4w9WgXcQ".to_string()));
    }

    #[test]
    fn test_youtube_without_www() {
        let url = "https://youtube.com/watch?v=dQw4w9WgXcQ";
        let id = PageInfo::extract_youtube_id(url);
        assert_eq!(id, Some("dQw4w9WgXcQ".to_string()));
    }

    #[test]
    fn test_youtube_invalid_id_length() {
        // YouTube IDs are exactly 11 characters
        let url = "https://www.youtube.com/watch?v=short";
        let id = PageInfo::extract_youtube_id(url);
        assert!(id.is_none());
    }

    #[test]
    fn test_youtube_not_youtube() {
        let url = "https://example.com/watch?v=dQw4w9WgXcQ";
        let id = PageInfo::extract_youtube_id(url);
        assert!(id.is_none());
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
        assert!(html.contains("<img src='https://example.com/image.png'/>"));
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
        assert!(html.contains("<a href='https://example.com/page'>"));
        assert!(html.contains("https://example.com/page</a>"));
        assert!(!html.contains("mbr-social-link-box"));
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

    // SSRF protection tests
    #[test]
    fn test_ssrf_blocks_localhost() {
        let result = validate_url_for_ssrf("http://localhost/admin");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Blocked local hostname"));
    }

    #[test]
    fn test_ssrf_blocks_localhost_variant() {
        let result = validate_url_for_ssrf("http://test.localhost/admin");
        assert!(result.is_err());
    }

    #[test]
    fn test_ssrf_blocks_127_0_0_1() {
        let result = validate_url_for_ssrf("http://127.0.0.1/secret");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Blocked"));
    }

    #[test]
    fn test_ssrf_blocks_private_10_x() {
        let result = validate_url_for_ssrf("http://10.0.0.1/internal");
        assert!(result.is_err());
    }

    #[test]
    fn test_ssrf_blocks_private_172_16_x() {
        let result = validate_url_for_ssrf("http://172.16.0.1/internal");
        assert!(result.is_err());
    }

    #[test]
    fn test_ssrf_blocks_private_192_168_x() {
        let result = validate_url_for_ssrf("http://192.168.1.1/router");
        assert!(result.is_err());
    }

    #[test]
    fn test_ssrf_blocks_ipv6_loopback() {
        let result = validate_url_for_ssrf("http://[::1]/admin");
        assert!(result.is_err());
    }

    #[test]
    fn test_ssrf_blocks_local_domain() {
        let result = validate_url_for_ssrf("http://printer.local/config");
        assert!(result.is_err());
    }

    #[test]
    fn test_ssrf_blocks_internal_domain() {
        let result = validate_url_for_ssrf("http://db.internal/");
        assert!(result.is_err());
    }

    #[test]
    fn test_ssrf_allows_public_url() {
        let result = validate_url_for_ssrf("https://example.com/page");
        assert!(result.is_ok());
    }

    #[test]
    fn test_ssrf_allows_https() {
        let result = validate_url_for_ssrf("https://github.com/user/repo");
        assert!(result.is_ok());
    }

    #[test]
    fn test_ssrf_blocks_file_scheme() {
        let result = validate_url_for_ssrf("file:///etc/passwd");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Blocked scheme"));
    }

    #[test]
    fn test_ssrf_blocks_ftp_scheme() {
        let result = validate_url_for_ssrf("ftp://ftp.example.com/file");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Blocked scheme"));
    }

    #[test]
    fn test_is_private_ipv4() {
        use std::net::Ipv4Addr;

        // Private ranges
        assert!(is_private_ipv4(Ipv4Addr::new(10, 0, 0, 1)));
        assert!(is_private_ipv4(Ipv4Addr::new(10, 255, 255, 255)));
        assert!(is_private_ipv4(Ipv4Addr::new(172, 16, 0, 1)));
        assert!(is_private_ipv4(Ipv4Addr::new(172, 31, 255, 255)));
        assert!(is_private_ipv4(Ipv4Addr::new(192, 168, 0, 1)));
        assert!(is_private_ipv4(Ipv4Addr::new(192, 168, 255, 255)));

        // Loopback
        assert!(is_private_ipv4(Ipv4Addr::new(127, 0, 0, 1)));
        assert!(is_private_ipv4(Ipv4Addr::new(127, 255, 255, 255)));

        // Link-local
        assert!(is_private_ipv4(Ipv4Addr::new(169, 254, 0, 1)));

        // Public IPs should NOT be private
        assert!(!is_private_ipv4(Ipv4Addr::new(8, 8, 8, 8)));
        assert!(!is_private_ipv4(Ipv4Addr::new(1, 1, 1, 1)));
        assert!(!is_private_ipv4(Ipv4Addr::new(93, 184, 216, 34)));
    }

    #[test]
    fn test_is_private_ipv6() {
        use std::net::Ipv6Addr;

        // Loopback
        assert!(is_private_ipv6(Ipv6Addr::LOCALHOST));

        // Unspecified
        assert!(is_private_ipv6(Ipv6Addr::UNSPECIFIED));

        // Unique local (fc00::/7)
        assert!(is_private_ipv6("fc00::1".parse().unwrap()));
        assert!(is_private_ipv6("fd00::1".parse().unwrap()));

        // Link-local (fe80::/10)
        assert!(is_private_ipv6("fe80::1".parse().unwrap()));

        // Public IPv6 should NOT be private
        assert!(!is_private_ipv6("2001:4860:4860::8888".parse().unwrap())); // Google DNS
    }

    #[tokio::test]
    async fn test_ssrf_protection_in_new_from_url() {
        // Attempting to fetch a localhost URL should return plain link, not error
        let result = PageInfo::new_from_url("http://localhost/admin", 500).await;
        assert!(result.is_ok());
        let info = result.unwrap();
        // Should return a plain link (no embed, no title fetched)
        assert!(info.embed_html.is_none());
        assert!(info.title.is_none());
        assert_eq!(info.url, "http://localhost/admin");
    }

    #[tokio::test]
    async fn test_ssrf_protection_private_ip() {
        let result = PageInfo::new_from_url("http://192.168.1.1/", 500).await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert!(info.embed_html.is_none());
        assert!(info.title.is_none());
    }
}

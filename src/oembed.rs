use regex::Regex;
use reqwest::Client;
use scraper::{Html, Selector};
use std::sync::LazyLock;
use std::time::Duration;

static META_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("meta").expect("Invalid meta selector"));

// Giphy regex patterns - compiled once for efficiency
static GIPHY_MEDIA_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://(?:media\d*|i)\.giphy\.com/").unwrap());
static GIPHY_PAGE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:www\.)?giphy\.com/gifs/(?:[^/]+-)?([a-zA-Z0-9]+)(?:\?.*)?$").unwrap()
});

#[derive(Default)]
pub struct PageInfo {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    pub embed_html: Option<String>, // For YouTube and other embeddable content
}

impl PageInfo {
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

    pub async fn new_from_url(
        url: &str,
        timeout_ms: u64,
    ) -> Result<Self, Box<dyn std::error::Error>> {
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

        // If timeout is 0, oembed is disabled - return a plain link without network call
        if timeout_ms == 0 {
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
            Err(_) => {
                // Any error (timeout, network, etc.) - return a plain link
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
        let body = response.text().await?;
        let document = Html::parse_document(&body);
        let mut title: Option<String> = None;
        let mut image: Option<String> = None;
        let mut description: Option<String> = None;
        for element in document.select(&META_SELECTOR) {
            if let Some(property) = element.value().attr("property") {
                match property {
                    "og:title" => {
                        if let Some(content) = element.value().attr("content") {
                            title = Some(html_escape::encode_text(content).to_string());
                        }
                    }
                    "og:image" => {
                        if let Some(content) = element.value().attr("content") {
                            image = Some(content.to_string());
                        }
                    }
                    "og:description" => {
                        if let Some(content) = element.value().attr("content") {
                            description = Some(content.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
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
        format!(
            "<a href='{}'>{}</a>",
            &self.url,
            self.title.clone().unwrap_or(self.url.clone())
        )
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
    async fn test_zero_timeout_still_embeds_youtube() {
        // YouTube embeds should still work with timeout=0 (no network needed)
        let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
        let result = PageInfo::new_from_url(url, 0).await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert!(info.embed_html.is_some());
        assert!(info.embed_html.unwrap().contains("youtube-embed"));
    }

    #[tokio::test]
    async fn test_zero_timeout_still_embeds_giphy() {
        // Giphy embeds should still work with timeout=0 (no network needed)
        let url = "https://media.giphy.com/media/CAxbo8KC2A0y4/giphy.gif";
        let result = PageInfo::new_from_url(url, 0).await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert!(info.embed_html.is_some());
        assert!(info.embed_html.unwrap().contains("giphy-embed"));
    }
}

use regex::Regex;
use reqwest::Client;
use scraper::{Html, Selector};
use std::sync::LazyLock;
use std::time::Duration;

static META_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("meta").expect("Invalid meta selector"));

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

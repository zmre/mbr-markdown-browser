//! Media embedding detection and HTML generation for image syntax extensions.
//!
//! This module handles the `![caption](url)` markdown syntax when the URL points to
//! media files (video, audio, PDF) or embeddable content (YouTube).

use crate::audio::Audio;
use crate::vid::Vid;
use regex::Regex;
use std::sync::LazyLock;

static EXTENSION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\.([0-9a-zA-Z]+)([?#].*)?$").expect("Invalid EXTENSION_RE regex pattern")
});

static YOUTUBE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:youtube\.com/watch\?.*v=|youtu\.be/|youtube\.com/embed/|youtube\.com/v/)([a-zA-Z0-9_-]{11})",
    )
    .expect("Invalid YOUTUBE_RE regex pattern")
});

/// Represents different types of media that can be embedded via image syntax
#[derive(Debug, PartialEq)]
pub enum MediaEmbed {
    /// Video files (mp4, webm, etc.) - uses HTML5 video with VidStack enhancement
    Video(Vid),
    /// Audio files (mp3, wav, etc.) - uses HTML5 audio
    Audio(Audio),
    /// YouTube videos - uses iframe embed
    YouTube {
        video_id: String,
        caption: Option<String>,
    },
    /// PDF documents - uses object tag with fallback link
    Pdf {
        url: String,
        caption: Option<String>,
    },
}

impl MediaEmbed {
    /// Try to detect media type from URL and create appropriate embed
    ///
    /// Priority order:
    /// 1. YouTube URLs (checked first since they might not have extensions)
    /// 2. Video files by extension
    /// 3. Audio files by extension
    /// 4. PDF files by extension
    ///
    /// Returns None if the URL doesn't match any known media type
    pub fn from_url_and_title(url: &str, title: &str) -> Option<Self> {
        // Check YouTube first (doesn't rely on extension)
        if let Some(video_id) = Self::extract_youtube_id(url) {
            return Some(MediaEmbed::YouTube {
                video_id,
                caption: if title.is_empty() {
                    None
                } else {
                    Some(title.to_string())
                },
            });
        }

        // Check by extension
        if let Some(ext) = Self::extension_from_url(url) {
            let ext_lower = ext.to_lowercase();

            // Video extensions (handled by Vid)
            if let Some(vid) = Vid::from_url_and_title(url, title) {
                return Some(MediaEmbed::Video(vid));
            }

            // Audio extensions
            if let Some(audio) = Audio::from_url_and_title(url, title) {
                return Some(MediaEmbed::Audio(audio));
            }

            // PDF
            if ext_lower == "pdf" {
                return Some(MediaEmbed::Pdf {
                    url: url.to_string(),
                    caption: if title.is_empty() {
                        None
                    } else {
                        Some(title.to_string())
                    },
                });
            }
        }

        None
    }

    /// Generate opening HTML for the media embed
    /// When open_only is true, leaves figcaption open for markdown parser to fill
    pub fn to_html(&self, open_only: bool) -> String {
        match self {
            MediaEmbed::Video(vid) => vid.to_html(open_only),
            MediaEmbed::Audio(audio) => audio.to_html(open_only),
            MediaEmbed::YouTube { video_id, caption } => {
                Self::youtube_to_html(video_id, caption.as_deref(), open_only)
            }
            MediaEmbed::Pdf { url, caption } => {
                Self::pdf_to_html(url, caption.as_deref(), open_only)
            }
        }
    }

    /// Generate closing HTML tags
    pub fn html_close(&self) -> String {
        match self {
            MediaEmbed::Video(_) => Vid::html_close(),
            MediaEmbed::Audio(_) => Audio::html_close().to_string(),
            MediaEmbed::YouTube { .. } | MediaEmbed::Pdf { .. } => {
                "</figcaption></figure>".to_string()
            }
        }
    }

    fn extract_youtube_id(url: &str) -> Option<String> {
        YOUTUBE_RE
            .captures(url)
            .and_then(|caps| caps.get(1))
            .map(|id| id.as_str().to_string())
    }

    fn extension_from_url(url: &str) -> Option<String> {
        EXTENSION_RE.captures(url).map(|cap| cap[1].to_string())
    }

    fn youtube_to_html(video_id: &str, caption: Option<&str>, open_only: bool) -> String {
        format!(
            r#"
            <figure class="video-embed youtube-embed">
                <iframe
                    width="560"
                    height="315"
                    src="https://www.youtube.com/embed/{video_id}"
                    title="YouTube video player"
                    frameborder="0"
                    allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"
                    referrerpolicy="strict-origin-when-cross-origin"
                    allowfullscreen>
                </iframe>
                <figcaption>{caption}{close}"#,
            video_id = video_id,
            caption = caption.unwrap_or(""),
            close = if open_only {
                ""
            } else {
                "</figcaption></figure>"
            }
        )
    }

    fn pdf_to_html(url: &str, caption: Option<&str>, open_only: bool) -> String {
        // Graceful degradation: object tag with fallback download link
        // The data-pdf-url attribute allows JavaScript enhancement (e.g., PDF.js)
        format!(
            r#"
            <figure class="pdf-embed" data-pdf-url="{url}">
                <object data="{url}" type="application/pdf" width="100%" height="600px">
                    <p class="pdf-fallback">
                        PDF cannot be displayed inline.
                        <a href="{url}" download data-pdf-fallback>Download PDF</a>
                    </p>
                </object>
                <figcaption>{caption}{close}"#,
            url = url,
            caption = caption.unwrap_or(""),
            close = if open_only {
                ""
            } else {
                "</figcaption></figure>"
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // YouTube detection tests
    #[test]
    fn test_youtube_watch_url() {
        let embed =
            MediaEmbed::from_url_and_title("https://www.youtube.com/watch?v=dQw4w9WgXcQ", "Title");
        assert!(matches!(
            embed,
            Some(MediaEmbed::YouTube { video_id, .. }) if video_id == "dQw4w9WgXcQ"
        ));
    }

    #[test]
    fn test_youtube_short_url() {
        let embed = MediaEmbed::from_url_and_title("https://youtu.be/dQw4w9WgXcQ", "");
        assert!(matches!(
            embed,
            Some(MediaEmbed::YouTube { video_id, caption }) if video_id == "dQw4w9WgXcQ" && caption.is_none()
        ));
    }

    #[test]
    fn test_youtube_embed_url() {
        let embed =
            MediaEmbed::from_url_and_title("https://www.youtube.com/embed/dQw4w9WgXcQ", "Caption");
        assert!(matches!(
            embed,
            Some(MediaEmbed::YouTube { video_id, caption }) if video_id == "dQw4w9WgXcQ" && caption == Some("Caption".to_string())
        ));
    }

    #[test]
    fn test_youtube_with_extra_params() {
        let embed = MediaEmbed::from_url_and_title(
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=30s",
            "",
        );
        assert!(matches!(
            embed,
            Some(MediaEmbed::YouTube { video_id, .. }) if video_id == "dQw4w9WgXcQ"
        ));
    }

    // Video detection tests
    #[test]
    fn test_video_mp4() {
        let embed = MediaEmbed::from_url_and_title("video.mp4", "My Video");
        assert!(matches!(embed, Some(MediaEmbed::Video(_))));
    }

    #[test]
    fn test_video_webm_not_detected_by_vid() {
        // webm is not in Vid's list, so it won't be detected as video
        // This is existing behavior - webm goes through as audio since Audio supports it
        let embed = MediaEmbed::from_url_and_title("video.webm", "");
        assert!(matches!(embed, Some(MediaEmbed::Audio(_))));
    }

    // Audio detection tests
    #[test]
    fn test_audio_mp3() {
        let embed = MediaEmbed::from_url_and_title("podcast.mp3", "Episode 1");
        assert!(matches!(embed, Some(MediaEmbed::Audio(_))));
    }

    #[test]
    fn test_audio_wav() {
        let embed = MediaEmbed::from_url_and_title("sound.wav", "");
        assert!(matches!(embed, Some(MediaEmbed::Audio(_))));
    }

    // PDF detection tests
    #[test]
    fn test_pdf() {
        let embed = MediaEmbed::from_url_and_title("document.pdf", "Important Doc");
        assert!(matches!(
            embed,
            Some(MediaEmbed::Pdf { url, caption }) if url == "document.pdf" && caption == Some("Important Doc".to_string())
        ));
    }

    #[test]
    fn test_pdf_with_path() {
        let embed = MediaEmbed::from_url_and_title("/docs/report.pdf", "");
        assert!(matches!(
            embed,
            Some(MediaEmbed::Pdf { url, caption }) if url == "/docs/report.pdf" && caption.is_none()
        ));
    }

    #[test]
    fn test_pdf_case_insensitive() {
        let embed = MediaEmbed::from_url_and_title("document.PDF", "");
        assert!(matches!(embed, Some(MediaEmbed::Pdf { .. })));
    }

    // Non-media files
    #[test]
    fn test_image_not_detected() {
        assert!(MediaEmbed::from_url_and_title("photo.jpg", "").is_none());
        assert!(MediaEmbed::from_url_and_title("image.png", "").is_none());
        assert!(MediaEmbed::from_url_and_title("graphic.gif", "").is_none());
    }

    #[test]
    fn test_unknown_extension() {
        assert!(MediaEmbed::from_url_and_title("file.xyz", "").is_none());
    }

    #[test]
    fn test_no_extension() {
        assert!(MediaEmbed::from_url_and_title("https://example.com/page", "").is_none());
    }

    // HTML generation tests
    #[test]
    fn test_youtube_html() {
        let embed = MediaEmbed::YouTube {
            video_id: "abc123xyz".to_string(),
            caption: Some("Test Video".to_string()),
        };
        let html = embed.to_html(false);
        assert!(html.contains("youtube-embed"));
        assert!(html.contains("https://www.youtube.com/embed/abc123xyz"));
        assert!(html.contains("<figcaption>Test Video</figcaption>"));
    }

    #[test]
    fn test_pdf_html() {
        let embed = MediaEmbed::Pdf {
            url: "/docs/test.pdf".to_string(),
            caption: Some("My PDF".to_string()),
        };
        let html = embed.to_html(false);
        assert!(html.contains("pdf-embed"));
        assert!(html.contains(r#"data="/docs/test.pdf""#));
        assert!(html.contains(r#"type="application/pdf""#));
        assert!(html.contains("data-pdf-fallback"));
        assert!(html.contains("<figcaption>My PDF</figcaption>"));
    }

    #[test]
    fn test_pdf_html_open_only() {
        let embed = MediaEmbed::Pdf {
            url: "doc.pdf".to_string(),
            caption: None,
        };
        let html = embed.to_html(true);
        assert!(html.contains("<object"));
        assert!(!html.contains("</figcaption></figure>"));
    }

    #[test]
    fn test_html_close() {
        let youtube = MediaEmbed::YouTube {
            video_id: "x".to_string(),
            caption: None,
        };
        let pdf = MediaEmbed::Pdf {
            url: "x.pdf".to_string(),
            caption: None,
        };
        assert_eq!(youtube.html_close(), "</figcaption></figure>");
        assert_eq!(pdf.html_close(), "</figcaption></figure>");
    }
}

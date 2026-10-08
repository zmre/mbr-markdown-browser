//! Media embedding detection and HTML generation for image syntax extensions.
//!
//! This module handles the `![caption](url)` markdown syntax when the URL points to
//! media files (video, audio, PDF) or embeddable content (YouTube).
//!
//! It also owns [`MediaViewerType`], the kind of `/.mbr/{videos,pdfs,audio,images}/`
//! viewer page a media file opens in, shared by the server, the static build and
//! launch-URL construction.

use crate::audio::Audio;
use crate::vid::Vid;
use regex::Regex;
use std::borrow::Cow;
use std::sync::LazyLock;

const PDF_EMBED_HEIGHT: &str = "600px";

static EXTENSION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\.([0-9a-zA-Z]+)([?#].*)?$").expect("Invalid EXTENSION_RE regex pattern")
});

static YOUTUBE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:youtube(?:-nocookie)?\.com/watch\?.*v=|youtu\.be/|youtube(?:-nocookie)?\.com/embed/|youtube(?:-nocookie)?\.com/v/)([a-zA-Z0-9_-]{11})",
    )
    .expect("Invalid YOUTUBE_RE regex pattern")
});

/// Represents different types of media that can be embedded via image syntax
#[derive(Debug, PartialEq)]
pub enum MediaEmbed {
    /// Video files (mp4, webm, etc.) - uses HTML5 video
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
    /// Create a MediaEmbed for bare URLs (no caption)
    ///
    /// This is used by the oembed system to detect media files from bare URLs
    /// in markdown text before attempting OpenGraph fetching.
    pub fn from_bare_url(url: &str) -> Option<Self> {
        Self::from_url_and_title(url, "")
    }

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
    ///
    /// - `open_only`: When true, leaves figcaption open for markdown parser to fill
    /// - `server_mode`: True in server/GUI mode, false in build/CLI mode
    /// - `transcode_enabled`: True when dynamic transcoding is enabled
    pub fn to_html(&self, open_only: bool, server_mode: bool, transcode_enabled: bool) -> String {
        match self {
            MediaEmbed::Video(vid) => vid.to_html(open_only, server_mode, transcode_enabled),
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

    /// Escape a caption for HTML element-text context (`<figcaption>`).
    ///
    /// Captions come from the markdown link title, which pulldown-cmark hands
    /// over unescaped.
    fn escaped_caption(caption: Option<&str>) -> Cow<'_, str> {
        caption.map(html_escape::encode_text).unwrap_or_default()
    }

    fn youtube_to_html(video_id: &str, caption: Option<&str>, open_only: bool) -> String {
        format!(
            r#"
            <figure class="video-embed youtube-embed">
                <iframe
                    width="{yt_width}"
                    height="{yt_height}"
                    src="https://www.youtube-nocookie.com/embed/{video_id}"
                    title="YouTube video player"
                    frameborder="0"
                    allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"
                    referrerpolicy="strict-origin-when-cross-origin"
                    allowfullscreen>
                </iframe>
                <figcaption>{caption}{close}"#,
            yt_width = crate::constants::YOUTUBE_EMBED_WIDTH,
            yt_height = crate::constants::YOUTUBE_EMBED_HEIGHT,
            video_id = html_escape::encode_double_quoted_attribute(video_id),
            caption = Self::escaped_caption(caption),
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
        //
        // The URL is the raw markdown link destination (pulldown-cmark does not
        // escape it, and neither does link_transform), so it must be escaped for
        // double-quoted attribute context or an `&` corrupts the output and a
        // `"` breaks out of the attribute entirely.
        let escaped_url = html_escape::encode_double_quoted_attribute(url);
        format!(
            r#"
            <figure class="pdf-embed" data-pdf-url="{url}">
                <object data="{url}" type="application/pdf" width="100%" height="{pdf_height}">
                    <p class="pdf-fallback">
                        PDF cannot be displayed inline.
                        <a href="{url}" download data-pdf-fallback>Download PDF</a>
                    </p>
                </object>
                <figcaption>{caption}{close}"#,
            url = escaped_url,
            pdf_height = PDF_EMBED_HEIGHT,
            caption = Self::escaped_caption(caption),
            close = if open_only {
                ""
            } else {
                "</figcaption></figure>"
            }
        )
    }
}

/// Type of media for the viewer page.
///
/// Used to route requests to the appropriate media viewer template
/// at `/.mbr/videos/`, `/.mbr/pdfs/`, `/.mbr/audio/`, or `/.mbr/images/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaViewerType {
    Video,
    Pdf,
    Audio,
    Image,
}

impl MediaViewerType {
    /// Parse from route path.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert_eq!(MediaViewerType::from_route("/.mbr/videos/"), Some(MediaViewerType::Video));
    /// assert_eq!(MediaViewerType::from_route("/.mbr/pdfs/"), Some(MediaViewerType::Pdf));
    /// assert_eq!(MediaViewerType::from_route("/.mbr/audio/"), Some(MediaViewerType::Audio));
    /// assert_eq!(MediaViewerType::from_route("/.mbr/images/"), Some(MediaViewerType::Image));
    /// assert_eq!(MediaViewerType::from_route("/some/other/path"), None);
    /// ```
    #[must_use]
    pub fn from_route(path: &str) -> Option<Self> {
        match path {
            "/.mbr/videos/" => Some(Self::Video),
            "/.mbr/pdfs/" => Some(Self::Pdf),
            "/.mbr/audio/" => Some(Self::Audio),
            "/.mbr/images/" => Some(Self::Image),
            _ => None,
        }
    }

    /// Template name for this media type.
    #[must_use]
    pub const fn template_name(&self) -> &'static str {
        "media_viewer.html"
    }

    /// Human-readable label for this media type.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Video => "Video",
            Self::Pdf => "PDF",
            Self::Audio => "Audio",
            Self::Image => "Image",
        }
    }

    /// Lowercase string representation for template context.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Pdf => "pdf",
            Self::Audio => "audio",
            Self::Image => "image",
        }
    }

    /// Determine media type from a file extension (case-insensitive).
    ///
    /// Returns `None` for unrecognized extensions.
    #[must_use]
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            // Video
            "mp4" | "m4v" | "mov" | "webm" | "flv" | "mpg" | "mpeg" | "avi" | "3gp" | "wmv"
            | "mkv" | "ts" | "mts" | "m2ts" | "vob" | "divx" | "xvid" | "asf" | "rm" | "rmvb"
            | "f4v" | "ogv" => Some(Self::Video),
            // Audio
            "mp3" | "wav" | "ogg" | "flac" | "aac" | "m4a" | "aiff" | "aif" | "oga" | "opus"
            | "wma" => Some(Self::Audio),
            // Image
            "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "svg" => {
                Some(Self::Image)
            }
            // PDF
            "pdf" => Some(Self::Pdf),
            _ => None,
        }
    }

    /// Determine media type from a file path by inspecting its extension.
    ///
    /// Returns `None` if the path has no extension or the extension is unrecognized.
    #[must_use]
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(Self::from_extension)
    }

    /// Returns the server route path for this media viewer type.
    #[must_use]
    pub const fn route_path(&self) -> &'static str {
        match self {
            Self::Video => "/.mbr/videos/",
            Self::Pdf => "/.mbr/pdfs/",
            Self::Audio => "/.mbr/audio/",
            Self::Image => "/.mbr/images/",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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
            "https://www.youtube-nocookie.com/watch?v=dQw4w9WgXcQ&t=30s",
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
        let html = embed.to_html(false, false, false);
        assert!(html.contains("youtube-embed"));
        assert!(html.contains("https://www.youtube-nocookie.com/embed/abc123xyz"));
        assert!(html.contains("<figcaption>Test Video</figcaption>"));
    }

    #[test]
    fn test_pdf_html() {
        let embed = MediaEmbed::Pdf {
            url: "/docs/test.pdf".to_string(),
            caption: Some("My PDF".to_string()),
        };
        let html = embed.to_html(false, false, false);
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
        let html = embed.to_html(true, false, false);
        assert!(html.contains("<object"));
        assert!(!html.contains("</figcaption></figure>"));
    }

    #[test]
    fn test_youtube_v_url() {
        let embed = MediaEmbed::from_url_and_title("https://www.youtube.com/v/dQw4w9WgXcQ", "");
        assert!(matches!(
            embed,
            Some(MediaEmbed::YouTube { video_id, .. }) if video_id == "dQw4w9WgXcQ"
        ));
    }

    #[test]
    fn test_youtube_without_www() {
        let embed = MediaEmbed::from_url_and_title("https://youtube.com/watch?v=dQw4w9WgXcQ", "");
        assert!(matches!(
            embed,
            Some(MediaEmbed::YouTube { video_id, .. }) if video_id == "dQw4w9WgXcQ"
        ));
    }

    #[test]
    fn test_youtube_invalid_id_length() {
        let embed = MediaEmbed::from_url_and_title("https://www.youtube.com/watch?v=short", "");
        assert!(embed.is_none());
    }

    #[test]
    fn test_youtube_not_youtube() {
        let embed = MediaEmbed::from_url_and_title("https://example.com/watch?v=dQw4w9WgXcQ", "");
        assert!(embed.is_none());
    }

    #[test]
    fn test_youtube_nocookie_embed_url() {
        let embed = MediaEmbed::from_url_and_title(
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
            "Caption",
        );
        assert!(matches!(
            embed,
            Some(MediaEmbed::YouTube { video_id, caption }) if video_id == "dQw4w9WgXcQ" && caption == Some("Caption".to_string())
        ));
    }

    #[test]
    fn test_youtube_nocookie_v_url() {
        let embed =
            MediaEmbed::from_url_and_title("https://www.youtube-nocookie.com/v/dQw4w9WgXcQ", "");
        assert!(matches!(
            embed,
            Some(MediaEmbed::YouTube { video_id, .. }) if video_id == "dQw4w9WgXcQ"
        ));
    }

    /// Regression: a PDF link destination containing a double quote must not be
    /// able to close `data-pdf-url`/`data`/`href` and inject new attributes.
    #[test]
    fn test_pdf_html_escapes_hostile_url() {
        let embed =
            MediaEmbed::from_url_and_title(r#"a"onerror="alert(1)"b.pdf"#, "").expect("pdf embed");
        let html = embed.to_html(false, false, false);
        assert!(
            html.contains("&quot;"),
            "double quotes must be escaped: {html}"
        );
        // The hostile destination must stay inside each attribute value; the
        // injected text may appear only as escaped data, never as an attribute.
        assert_eq!(
            html.matches(r#"a&quot;onerror=&quot;alert(1)&quot;b.pdf"#)
                .count(),
            3,
            "data-pdf-url, object data, and href must all be escaped: {html}"
        );
        assert!(
            !html.contains(r#""onerror=""#),
            "must not emit an injected attribute: {html}"
        );
    }

    /// Regression: a bare `&` in a filename is invalid in an attribute value
    /// and must be encoded in all three places the URL is interpolated.
    #[test]
    fn test_pdf_html_escapes_ampersand_in_url() {
        let embed = MediaEmbed::from_url_and_title("/docs/Q&A-report.pdf", "").expect("pdf embed");
        let html = embed.to_html(false, false, false);
        assert!(
            !html.contains("Q&A-report"),
            "bare ampersand emitted: {html}"
        );
        assert_eq!(
            html.matches("/docs/Q&amp;A-report.pdf").count(),
            3,
            "data-pdf-url, object data, and href must all be encoded: {html}"
        );
    }

    /// Regression: captions land in element-text context and must be escaped.
    #[test]
    fn test_pdf_html_escapes_caption() {
        let embed = MediaEmbed::from_url_and_title("doc.pdf", "<script>alert(1)</script> & more")
            .expect("pdf embed");
        let html = embed.to_html(false, false, false);
        assert!(
            html.contains("&lt;script&gt;alert(1)&lt;/script&gt; &amp; more"),
            "caption must be escaped: {html}"
        );
        assert!(
            !html.contains("<script>"),
            "must not emit a raw script tag: {html}"
        );
    }

    /// Escaping must not double-encode ordinary destinations or captions.
    #[test]
    fn test_pdf_html_ordinary_values_round_trip_unchanged() {
        let embed =
            MediaEmbed::from_url_and_title("/docs/my_report-v2.pdf", "My PDF").expect("pdf embed");
        let html = embed.to_html(false, false, false);
        assert!(html.contains(r#"data-pdf-url="/docs/my_report-v2.pdf""#));
        assert!(html.contains(r#"data="/docs/my_report-v2.pdf""#));
        assert!(html.contains(r#"href="/docs/my_report-v2.pdf""#));
        assert!(html.contains("<figcaption>My PDF</figcaption>"));
        assert!(!html.contains("&amp;"), "nothing to escape here: {html}");
    }

    /// Regression: the YouTube video id and caption are interpolated too.
    #[test]
    fn test_youtube_html_escapes_video_id_and_caption() {
        let embed = MediaEmbed::YouTube {
            video_id: r#"x"onload="alert(1)"#.to_string(),
            caption: Some("<b>hi</b> & bye".to_string()),
        };
        let html = embed.to_html(false, false, false);
        assert!(
            html.contains("&quot;"),
            "double quotes must be escaped: {html}"
        );
        assert!(
            html.contains(
                r#"src="https://www.youtube-nocookie.com/embed/x&quot;onload=&quot;alert(1)""#
            ),
            "iframe src must be fully escaped: {html}"
        );
        assert!(
            !html.contains(r#""onload=""#),
            "must not emit an injected attribute: {html}"
        );
        assert!(html.contains("&lt;b&gt;hi&lt;/b&gt; &amp; bye"));
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

    // ==================== MediaViewerType Tests ====================

    #[test]
    fn test_media_viewer_type_from_route_videos() {
        assert_eq!(
            MediaViewerType::from_route("/.mbr/videos/"),
            Some(MediaViewerType::Video)
        );
    }

    #[test]
    fn test_media_viewer_type_from_route_pdfs() {
        assert_eq!(
            MediaViewerType::from_route("/.mbr/pdfs/"),
            Some(MediaViewerType::Pdf)
        );
    }

    #[test]
    fn test_media_viewer_type_from_route_audio() {
        assert_eq!(
            MediaViewerType::from_route("/.mbr/audio/"),
            Some(MediaViewerType::Audio)
        );
    }

    #[test]
    fn test_media_viewer_type_from_route_images() {
        assert_eq!(
            MediaViewerType::from_route("/.mbr/images/"),
            Some(MediaViewerType::Image)
        );
    }

    #[test]
    fn test_media_viewer_type_from_route_invalid() {
        assert_eq!(MediaViewerType::from_route("/some/other/path"), None);
        assert_eq!(MediaViewerType::from_route("/.mbr/videos"), None); // missing trailing slash
        assert_eq!(MediaViewerType::from_route("/.mbr/unknown/"), None);
    }

    #[test]
    fn test_media_viewer_type_template_name() {
        assert_eq!(MediaViewerType::Video.template_name(), "media_viewer.html");
        assert_eq!(MediaViewerType::Pdf.template_name(), "media_viewer.html");
        assert_eq!(MediaViewerType::Audio.template_name(), "media_viewer.html");
    }

    #[test]
    fn test_media_viewer_type_label() {
        assert_eq!(MediaViewerType::Video.label(), "Video");
        assert_eq!(MediaViewerType::Pdf.label(), "PDF");
        assert_eq!(MediaViewerType::Audio.label(), "Audio");
    }

    #[test]
    fn test_media_viewer_type_as_str() {
        assert_eq!(MediaViewerType::Video.as_str(), "video");
        assert_eq!(MediaViewerType::Pdf.as_str(), "pdf");
        assert_eq!(MediaViewerType::Audio.as_str(), "audio");
    }

    #[test]
    fn test_media_viewer_type_from_extension_video() {
        for ext in &[
            "mp4", "m4v", "mov", "webm", "flv", "mpg", "mpeg", "avi", "3gp", "wmv", "mkv", "ts",
            "mts", "m2ts", "vob", "divx", "xvid", "asf", "rm", "rmvb", "f4v", "ogv",
        ] {
            assert_eq!(
                MediaViewerType::from_extension(ext),
                Some(MediaViewerType::Video),
                "Expected Video for extension '{ext}'"
            );
        }
    }

    #[test]
    fn test_media_viewer_type_from_extension_audio() {
        for ext in &[
            "mp3", "wav", "ogg", "flac", "aac", "m4a", "aiff", "aif", "oga", "opus", "wma",
        ] {
            assert_eq!(
                MediaViewerType::from_extension(ext),
                Some(MediaViewerType::Audio),
                "Expected Audio for extension '{ext}'"
            );
        }
    }

    #[test]
    fn test_media_viewer_type_from_extension_image() {
        for ext in &[
            "jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff", "svg",
        ] {
            assert_eq!(
                MediaViewerType::from_extension(ext),
                Some(MediaViewerType::Image),
                "Expected Image for extension '{ext}'"
            );
        }
    }

    #[test]
    fn test_media_viewer_type_from_extension_pdf() {
        assert_eq!(
            MediaViewerType::from_extension("pdf"),
            Some(MediaViewerType::Pdf)
        );
    }

    #[test]
    fn test_media_viewer_type_from_extension_case_insensitive() {
        assert_eq!(
            MediaViewerType::from_extension("MP4"),
            Some(MediaViewerType::Video)
        );
        assert_eq!(
            MediaViewerType::from_extension("Pdf"),
            Some(MediaViewerType::Pdf)
        );
        assert_eq!(
            MediaViewerType::from_extension("JPG"),
            Some(MediaViewerType::Image)
        );
    }

    #[test]
    fn test_media_viewer_type_from_extension_unknown() {
        assert_eq!(MediaViewerType::from_extension("md"), None);
        assert_eq!(MediaViewerType::from_extension("html"), None);
        assert_eq!(MediaViewerType::from_extension("rs"), None);
        assert_eq!(MediaViewerType::from_extension(""), None);
    }

    #[test]
    fn test_media_viewer_type_from_path() {
        assert_eq!(
            MediaViewerType::from_path(Path::new("videos/demo.mp4")),
            Some(MediaViewerType::Video)
        );
        assert_eq!(
            MediaViewerType::from_path(Path::new("music/song.mp3")),
            Some(MediaViewerType::Audio)
        );
        assert_eq!(
            MediaViewerType::from_path(Path::new("images/photo.jpg")),
            Some(MediaViewerType::Image)
        );
        assert_eq!(
            MediaViewerType::from_path(Path::new("docs/paper.pdf")),
            Some(MediaViewerType::Pdf)
        );
        assert_eq!(MediaViewerType::from_path(Path::new("readme.md")), None);
        assert_eq!(MediaViewerType::from_path(Path::new("noext")), None);
    }

    #[test]
    fn test_media_viewer_type_route_path() {
        assert_eq!(MediaViewerType::Video.route_path(), "/.mbr/videos/");
        assert_eq!(MediaViewerType::Pdf.route_path(), "/.mbr/pdfs/");
        assert_eq!(MediaViewerType::Audio.route_path(), "/.mbr/audio/");
        assert_eq!(MediaViewerType::Image.route_path(), "/.mbr/images/");
    }

    #[test]
    fn test_media_viewer_type_route_path_roundtrips_with_from_route() {
        for media_type in &[
            MediaViewerType::Video,
            MediaViewerType::Pdf,
            MediaViewerType::Audio,
            MediaViewerType::Image,
        ] {
            assert_eq!(
                MediaViewerType::from_route(media_type.route_path()),
                Some(*media_type),
                "route_path -> from_route roundtrip failed for {media_type:?}"
            );
        }
    }
}

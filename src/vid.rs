use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use regex::Regex;
use std::sync::LazyLock;

// Compile regexes once at startup
static TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?x)^\s*\{\{\s*vid\s*\((?P<params>.*?)\)\s*\}\}\s*$"#)
        .expect("Invalid TAG_RE regex pattern")
});
static KV_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Match key="value" pairs, supporting both straight quotes (") and
    // curly/smart quotes (" " U+201C/U+201D) from pulldown-cmark's smart punctuation
    Regex::new(
        r#"\b(?P<key>\w+)\s*=\s*["'""\u{201C}\u{201D}](?P<val>[^'""]*?)["'""\u{201C}\u{201D}]"#,
    )
    .expect("Invalid KV_RE regex pattern")
});
static EXTENSION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\.([0-9a-zA-Z]+)([?#].*)?$").expect("Invalid EXTENSION_RE regex pattern")
});
static TIME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"#t=([0-9]+(:[0-9]+)*)(,([0-9]+(:[0-9]+)*))?$")
        .expect("Invalid TIME_RE regex pattern")
});

#[derive(Debug, PartialEq, Default)]
pub struct Vid {
    pub url: String,
    pub ext: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub caption: Option<String>,
}

impl Vid {
    pub fn from_url_and_title(url: &str, title: &str) -> Option<Self> {
        let (start, end, url) = Self::start_stop_from_url(url);
        let ext = Self::extension_from_url(url);
        match ext.as_deref() {
            Some("mp4") | Some("mpg") | Some("avi") | Some("ogv") | Some("ogg") | Some("m4v")
            | Some("mkv") => Some(Self {
                url: url.to_string(),
                ext,
                start,
                end,
                caption: Some(title.to_string()),
            }),
            _ => None,
        }
    }

    pub fn from_vid(input: &str) -> Option<Self> {
        // 1) match the whole tag {{ vid( … ) }}
        // 2) capture everything inside the parens as "params"
        // 3) match individual key="value" pairs
        let caps = TAG_RE.captures(input)?;
        let params_str = &caps["params"];

        let mut vid: Vid = Default::default();
        let mut path: Option<String> = None;

        for kv in KV_RE.captures_iter(params_str) {
            let key = &kv["key"];
            let val = &kv["val"];
            match key {
                "path" => path = Some(val.to_string()),
                "start" => vid.start = Some(val.to_string()),
                "end" => vid.end = Some(val.to_string()),
                "caption" => vid.caption = Some(val.to_string()),
                _ => { /* ignore unknown keys */ }
            }
        }

        const CUSTOM_ENCODE_SET: &AsciiSet =
            &NON_ALPHANUMERIC.remove(b'.').remove(b'/').remove(b'?');

        match path {
            Some(p) => {
                vid.url = utf8_percent_encode(format!("/videos/{p}").as_str(), CUSTOM_ENCODE_SET)
                    .to_string();
                vid.ext = Self::extension_from_url(&vid.url);
                Some(vid)
            }
            None => None,
        }
    }

    pub fn to_mime_type(&self) -> String {
        match self.ext.as_deref() {
            Some("m4v") => "video/mpeg".to_string(),
            Some("mov") => "video/quicktime".to_string(),
            Some("avi") => "video/x-msvideo".to_string(),
            Some("ogg") | Some("ogv") => "video/ogg".to_string(),
            Some(ext) => format!("video/{ext}"),
            None => "x".to_string(),
        }
    }

    /// Generate HTML for video embed.
    ///
    /// - `open_only`: When true, leaves figcaption open for markdown parser to fill
    /// - `server_mode`: True in server/GUI mode, false in build/CLI mode
    /// - `transcode_enabled`: True when dynamic transcoding is enabled
    ///
    /// When both `server_mode` and `transcode_enabled` are true, generates multiple
    /// `<source>` tags with media queries for responsive video loading using HLS:
    /// - Original MP4 for wide screens (>= 1280px) - all browsers
    /// - HLS 720p for medium screens (>= 640px) - Safari only (native HLS)
    /// - HLS 480p as fallback for small screens - Safari only
    /// - Original MP4 as final fallback (no media query) - Chrome/Firefox/Edge on mobile
    ///
    /// Note: Time fragments (#t=start,end) only apply to MP4 sources, not HLS.
    pub fn to_html(&self, open_only: bool, server_mode: bool, transcode_enabled: bool) -> String {
        let mut time = "".to_string();
        if let Some(start) = self.start.as_ref() {
            time = format!("#t={start}");
            if let Some(end) = self.end.as_ref() {
                time += ",";
                time += end.as_str();
            }
        }

        // Build source tags based on transcode mode
        let sources = if server_mode && transcode_enabled {
            // Generate multiple sources with media queries for responsive loading
            // HLS variants for Safari, MP4 fallback for other browsers
            let base_url = &self.url;
            let mime = self.to_mime_type();

            // Strip extension for HLS variant URLs
            let url_base = match base_url.rsplit_once('.') {
                Some((base, _)) => base.to_string(),
                None => base_url.clone(),
            };

            // HLS mime type for playlists
            let hls_mime = "application/vnd.apple.mpegurl";

            format!(
                r#"<source src='{base_url}{time}' media="(min-width: 1280px)" type="{mime}">
                    <source src='{url_base}-720p.m3u8' media="(min-width: 640px)" type="{hls_mime}">
                    <source src='{url_base}-480p.m3u8' type="{hls_mime}">
                    <source src='{base_url}{time}' type="{mime}">"#,
            )
        } else {
            // Single source - original behavior
            format!(
                "<source src='{}{}' type='{}'>",
                self.url,
                time,
                self.to_mime_type()
            )
        };

        format!(
            r#"
            <figure>
                <video controls preload="none" playsinline loop="false" poster="{}.cover.png">
                    {}
                    <track kind="captions" label="English captions" src="{}.captions.en.vtt" srclang="en" language="en-US" default type="vtt" data-type="vtt" />
                    <track kind="chapters" language="en-US" label="Chapters" src="{}.chapters.en.vtt" srclang="en" default type="vtt" data-type="vtt" />
                </video>
                <figcaption>{}
                <mbr-video-extras src='{}' start='{}' end='{}'></mbr-video-extras>
                {}
            "#,
            self.url,
            sources,
            self.url,
            self.url,
            self.caption.as_deref().unwrap_or(""),
            self.url,
            self.start.as_ref().unwrap_or(&"".to_string()),
            self.end.as_ref().unwrap_or(&"".to_string()),
            {
                if open_only {
                    "".to_string()
                } else {
                    Self::html_close()
                }
            }
        )
    }

    pub fn html_close() -> String {
        "</figcaption></figure>".to_string()
    }

    fn extension_from_url(url: &str) -> Option<String> {
        EXTENSION_RE.captures(url).map(|cap| cap[1].to_string())
    }

    fn start_stop_from_url(url: &str) -> (Option<String>, Option<String>, &str) {
        match TIME_RE.captures(url) {
            Some(cap) => {
                let url = match url.rsplit_once('#') {
                    Some((base, _)) => base,
                    None => url,
                };
                (
                    cap.get(1).map(|t| t.as_str().to_string()),
                    cap.get(4).map(|t| t.as_str().to_string()),
                    url,
                )
            }
            None => (None, None, url),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_url_and_title_valid() {
        let url = "video.mp4";
        let title = "A video";
        let vid = Vid::from_url_and_title(url, title).unwrap();
        assert_eq!(vid.url, url);
        assert_eq!(vid.ext.as_deref(), Some("mp4"));
        assert_eq!(vid.caption.as_deref(), Some(title));
        assert!(vid.start.is_none());
        assert!(vid.end.is_none());
    }

    #[test]
    fn test_from_url_and_title_invalid() {
        let url = "image.png";
        let title = "Not a video";
        assert!(Vid::from_url_and_title(url, title).is_none());
    }

    #[test]
    fn test_from_url_and_title_with_time() {
        let url = "video.mp4#t=10,20";
        let title = "Timed video";
        let vid = Vid::from_url_and_title(url, title).unwrap();
        assert_eq!(vid.url, "video.mp4");
        assert_eq!(vid.ext.as_deref(), Some("mp4"));
        assert_eq!(vid.start.as_deref(), Some("10"));
        assert_eq!(vid.end.as_deref(), Some("20"));
    }

    #[test]
    fn test_from_vid_valid() {
        let input = r#"{{ vid(path="foo.mp4", start="10", end="20", caption="Test") }}"#;
        let vid = Vid::from_vid(input).unwrap();
        assert!(vid.url.contains("/videos/foo.mp4"));
        assert_eq!(vid.start.as_deref(), Some("10"));
        assert_eq!(vid.end.as_deref(), Some("20"));
        assert_eq!(vid.caption.as_deref(), Some("Test"));
    }

    #[test]
    fn test_from_vid_invalid() {
        let input = r#"{{ notvid(path="foo.mp4") }}"#;
        assert!(Vid::from_vid(input).is_none());
    }

    #[test]
    fn test_from_vid_missing_path() {
        let input = r#"{{ vid(caption="No path") }}"#;
        assert!(Vid::from_vid(input).is_none());
    }

    #[test]
    fn test_to_html() {
        let vid = Vid {
            url: "/videos/foo.mp4".to_string(),
            ext: Some("mp4".to_string()),
            start: Some("10".to_string()),
            end: Some("20".to_string()),
            caption: Some("Caption".to_string()),
        };
        let html = vid.to_html(false, false, false);
        assert!(html.contains("<video"));
        assert!(html.contains("src='/videos/foo.mp4#t=10,20'"));
        assert!(html.contains("<figcaption>Caption"));
        assert!(html.contains(
            "<mbr-video-extras src='/videos/foo.mp4' start='10' end='20'></mbr-video-extras>"
        ));
        assert!(html.contains("</figcaption></figure>"));
    }

    #[test]
    fn test_to_html_with_transcode_enabled() {
        let vid = Vid {
            url: "/videos/foo.mp4".to_string(),
            ext: Some("mp4".to_string()),
            start: Some("10".to_string()),
            end: Some("20".to_string()),
            caption: Some("Caption".to_string()),
        };
        // With server_mode=true and transcode_enabled=true, should generate HLS sources
        let html = vid.to_html(false, true, true);
        assert!(html.contains("<video"));
        // Original MP4 source with media query for wide screens
        assert!(html.contains(r#"src='/videos/foo.mp4#t=10,20' media="(min-width: 1280px)""#));
        // HLS 720p variant (no time fragment - HLS doesn't support it)
        assert!(html.contains(r#"src='/videos/foo-720p.m3u8' media="(min-width: 640px)""#));
        assert!(html.contains(r#"type="application/vnd.apple.mpegurl""#));
        // HLS 480p variant (no media query - smallest HLS)
        assert!(html.contains("src='/videos/foo-480p.m3u8'"));
        // MP4 fallback (no media query) for non-Safari browsers
        // Count the number of times the original MP4 appears (should be twice)
        assert_eq!(
            html.matches("/videos/foo.mp4#t=10,20").count(),
            2,
            "Original MP4 should appear twice: once for wide screens, once as fallback"
        );
        assert!(html.contains("<figcaption>Caption"));
    }

    #[test]
    fn test_to_html_transcode_requires_server_mode() {
        let vid = Vid {
            url: "/videos/foo.mp4".to_string(),
            ext: Some("mp4".to_string()),
            start: None,
            end: None,
            caption: None,
        };
        // transcode_enabled=true but server_mode=false should NOT generate multiple sources
        let html = vid.to_html(false, false, true);
        // Should only have single source (original behavior)
        assert!(!html.contains("-720p.m3u8"));
        assert!(!html.contains("-480p.m3u8"));
        assert!(html.contains("src='/videos/foo.mp4'"));
    }

    #[test]
    fn test_extension_from_url() {
        assert_eq!(
            Vid::extension_from_url("foo/bar/video.mp4"),
            Some("mp4".to_string())
        );
        assert_eq!(
            Vid::extension_from_url("foo/bar/video.mp4?query=1"),
            Some("mp4".to_string())
        );
        assert_eq!(Vid::extension_from_url("foo/bar/video"), None);
    }

    #[test]
    fn test_mimetype_from_url() {
        let title = "Whatever";
        let url = "x/y/video.mp4#t=10,20";
        let vid = Vid::from_url_and_title(url, title).unwrap();
        let url2 = "x/y/video.ogv#t=10,20";
        let vid2 = Vid::from_url_and_title(url2, title).unwrap();

        assert_eq!(vid.to_mime_type(), "video/mp4");
        assert_eq!(vid2.to_mime_type(), "video/ogg");
    }

    #[test]
    fn test_start_stop_from_url() {
        let (start, end, url) = Vid::start_stop_from_url("foo.mp4#t=10,20");
        assert_eq!(start, Some("10".to_string()));
        assert_eq!(end, Some("20".to_string()));
        assert_eq!(url, "foo.mp4");

        let (start, end, url) = Vid::start_stop_from_url("foo.mp4#t=10:10:10,20:20:20");
        assert_eq!(start, Some("10:10:10".to_string()));
        assert_eq!(end, Some("20:20:20".to_string()));
        assert_eq!(url, "foo.mp4");

        let (start, end, url) = Vid::start_stop_from_url("foo.mp4#t=10");
        assert_eq!(start, Some("10".to_string()));
        assert!(end.is_none());
        assert_eq!(url, "foo.mp4");

        let (start, end, url) = Vid::start_stop_from_url("foo.mp4");
        assert!(start.is_none());
        assert!(end.is_none());
        assert_eq!(url, "foo.mp4");
    }
}

#[cfg(test)]
mod markdown_integration_tests {
    use super::*;

    #[test]
    fn test_from_vid_with_spaces_in_path() {
        let input = r#"{{ vid(path="Eric Jones/Eric Jones - Metal 3.mp4")}}"#;
        let vid = Vid::from_vid(input).unwrap();
        println!("URL: {}", &vid.url);
        assert!(vid.url.contains("/videos/"));
        assert!(vid.url.contains("Eric%20Jones")); // spaces should be URL-encoded
    }
}

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
            Some("mp4") | Some("mpg") | Some("avi") | Some("ogv") | Some("ogg") | Some("m4v") => {
                Some(Self {
                    url: url.to_string(),
                    ext,
                    start,
                    end,
                    caption: Some(title.to_string()),
                })
            }
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

    pub fn to_html(&self, open_only: bool) -> String {
        let mut time = "".to_string();
        if let Some(start) = self.start.as_ref() {
            time = format!("#t={start}");
            if let Some(end) = self.end.as_ref() {
                time += ",";
                time += end.as_str();
            }
        }

        // TODO: look at path and look for expected variants like *.chapters.vtt and *.en.vtt and add tracks below if found
        // TODO: same with cover art
        // TODO: consider using my other rust project to extract captions and chapters and cover art if it doesn't exist -- as a config-gated option, but on the fly
        //      ACTUALLY, maybe automatically specify those files in the HTML, then let the server generate them on-demand if they don't exist
        //      REALLY need to be able to save these things back into the files better so I can preserve what I have now, too
        //      MAY consider dynamic transcoding down to 480p or 360 or whatever for mobile with a special filename, too (can i do that streaming for server mode?)
        //      AND if I'm doing that for videos, do I want to do something similar for images?  I could make all images clickable to zoom in and have mobile-ready versions
        //      in a produced site or on-demand

        //
        //

        format!(
            r#"
            <figure>
                <video controls preload="metadata" poster="{}.cover.png">
                    <source src='{}{}' type="{}">
                    <track kind="captions" label="English captions" src="{}.captions.en.vtt" srclang="en" language="en-US" default type="vtt" data-type="vtt" />
                    <track kind="chapters" language="en-US" label="Chapters" src="{}.chapters.en.vtt" srclang="en" default type="vtt" data-type="vtt" />
                </video>
                <figcaption>{}{}
            "#,
            self.url,
            self.url,
            time,
            self.to_mime_type(),
            self.url,
            self.url,
            self.caption.as_deref().unwrap_or(""),
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
        let html = vid.to_html(false);
        assert!(html.contains("<video"));
        assert!(html.contains("src='/videos/foo.mp4#t=10,20'"));
        assert!(html.contains("<figcaption>Caption</figcaption>"));
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

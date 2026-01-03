use regex::Regex;
use std::sync::LazyLock;

static EXTENSION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\.([0-9a-zA-Z]+)([?#].*)?$").expect("Invalid EXTENSION_RE regex pattern")
});

/// Audio file extensions we support
const AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "ogg", "flac", "aac", "m4a", "webm"];

#[derive(Debug, PartialEq, Default)]
pub struct Audio {
    pub url: String,
    pub ext: Option<String>,
    pub caption: Option<String>,
}

impl Audio {
    /// Try to create an Audio from a URL if it has an audio file extension
    pub fn from_url_and_title(url: &str, title: &str) -> Option<Self> {
        let ext = Self::extension_from_url(url)?;
        if AUDIO_EXTENSIONS.contains(&ext.to_lowercase().as_str()) {
            Some(Self {
                url: url.to_string(),
                ext: Some(ext),
                caption: if title.is_empty() {
                    None
                } else {
                    Some(title.to_string())
                },
            })
        } else {
            None
        }
    }

    pub fn to_mime_type(&self) -> String {
        match self.ext.as_deref() {
            Some("mp3") => "audio/mpeg".to_string(),
            Some("m4a") => "audio/mp4".to_string(),
            Some("ogg") => "audio/ogg".to_string(),
            Some("wav") => "audio/wav".to_string(),
            Some("flac") => "audio/flac".to_string(),
            Some("aac") => "audio/aac".to_string(),
            Some("webm") => "audio/webm".to_string(),
            Some(ext) => format!("audio/{ext}"),
            None => "audio/mpeg".to_string(),
        }
    }

    /// Generate HTML for audio embedding
    /// If open_only is true, leaves the figcaption unclosed for the markdown parser to add content
    pub fn to_html(&self, open_only: bool) -> String {
        format!(
            r#"
            <figure class="audio-embed">
                <audio controls preload="metadata">
                    <source src="{}" type="{}">
                    Your browser does not support the audio element.
                </audio>
                <figcaption>{}{}"#,
            self.url,
            self.to_mime_type(),
            self.caption.as_deref().unwrap_or(""),
            if open_only { "" } else { Self::html_close() }
        )
    }

    pub fn html_close() -> &'static str {
        "</figcaption></figure>"
    }

    fn extension_from_url(url: &str) -> Option<String> {
        EXTENSION_RE.captures(url).map(|cap| cap[1].to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_url_and_title_mp3() {
        let audio = Audio::from_url_and_title("podcast.mp3", "Episode 1").unwrap();
        assert_eq!(audio.url, "podcast.mp3");
        assert_eq!(audio.ext.as_deref(), Some("mp3"));
        assert_eq!(audio.caption.as_deref(), Some("Episode 1"));
    }

    #[test]
    fn test_from_url_and_title_various_formats() {
        for ext in &["mp3", "wav", "ogg", "flac", "aac", "m4a", "webm"] {
            let url = format!("audio.{}", ext);
            let audio = Audio::from_url_and_title(&url, "Test");
            assert!(audio.is_some(), "Should recognize .{} files", ext);
        }
    }

    #[test]
    fn test_from_url_and_title_not_audio() {
        assert!(Audio::from_url_and_title("image.png", "Not audio").is_none());
        assert!(Audio::from_url_and_title("video.mp4", "Not audio").is_none());
        assert!(Audio::from_url_and_title("document.pdf", "Not audio").is_none());
    }

    #[test]
    fn test_from_url_and_title_empty_caption() {
        let audio = Audio::from_url_and_title("track.mp3", "").unwrap();
        assert!(audio.caption.is_none());
    }

    #[test]
    fn test_to_mime_type() {
        let cases = [
            ("mp3", "audio/mpeg"),
            ("m4a", "audio/mp4"),
            ("ogg", "audio/ogg"),
            ("wav", "audio/wav"),
            ("flac", "audio/flac"),
            ("aac", "audio/aac"),
            ("webm", "audio/webm"),
        ];
        for (ext, expected_mime) in cases {
            let audio = Audio {
                url: format!("file.{}", ext),
                ext: Some(ext.to_string()),
                caption: None,
            };
            assert_eq!(audio.to_mime_type(), expected_mime);
        }
    }

    #[test]
    fn test_to_html() {
        let audio = Audio {
            url: "/audio/song.mp3".to_string(),
            ext: Some("mp3".to_string()),
            caption: Some("My Song".to_string()),
        };
        let html = audio.to_html(false);
        assert!(html.contains("<audio controls"));
        assert!(html.contains(r#"src="/audio/song.mp3""#));
        assert!(html.contains(r#"type="audio/mpeg""#));
        assert!(html.contains("<figcaption>My Song</figcaption>"));
    }

    #[test]
    fn test_to_html_open_only() {
        let audio = Audio {
            url: "song.mp3".to_string(),
            ext: Some("mp3".to_string()),
            caption: Some("Test".to_string()),
        };
        let html = audio.to_html(true);
        assert!(html.contains("<audio"));
        assert!(!html.contains("</figcaption></figure>"));
    }

    #[test]
    fn test_extension_from_url_with_query() {
        let audio = Audio::from_url_and_title("song.mp3?token=abc", "Test").unwrap();
        assert_eq!(audio.ext.as_deref(), Some("mp3"));
    }

    #[test]
    fn test_case_insensitive_extension() {
        assert!(Audio::from_url_and_title("song.MP3", "Test").is_some());
        assert!(Audio::from_url_and_title("song.Mp3", "Test").is_some());
    }
}

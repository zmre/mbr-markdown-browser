use crate::media::MediaEmbed;
use crate::oembed::PageInfo;
use crate::vid::Vid;
use pulldown_cmark::{
    Event, MetadataBlockKind, Options, Parser as MDParser, Tag, TagEnd, TextMergeStream,
};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};
use yaml_rust2::{Yaml, YamlLoader};

struct EventState {
    #[allow(dead_code)] // Reserved for future use (resolving relative paths)
    root_path: PathBuf,
    /// Track the current media embed type (if any) for proper closing tags
    current_media: Option<MediaEmbed>,
    in_metadata: bool,
    in_link: bool, // Track when inside a link (including autolinks like <http://...>)
    metadata_source: Option<MetadataBlockKind>,
    metadata_parsed: Option<Yaml>,
    oembed_timeout_ms: u64,
}

pub type SimpleMetadata = HashMap<String, String>;

pub async fn render(
    file: PathBuf,
    root_path: &Path,
    oembed_timeout_ms: u64,
) -> Result<(SimpleMetadata, String), Box<dyn std::error::Error>> {
    // Create parser with example Markdown text.
    let markdown_input = fs::read_to_string(file)?;
    let parser = MDParser::new_ext(&markdown_input, Options::all());
    let parser = TextMergeStream::new(parser);

    // Write to a new String buffer.
    let mut html_output = String::with_capacity(markdown_input.capacity() * 2);
    let mut state = EventState {
        root_path: root_path.to_path_buf(),
        current_media: None,
        in_metadata: false,
        in_link: false,
        metadata_source: None,
        metadata_parsed: None,
        oembed_timeout_ms,
    };
    let mut processed_events = Vec::new();

    for event in parser {
        let (processed, new_state) = process_event(event, state).await;
        state = new_state;
        processed_events.push(processed);
    }

    crate::html::push_html(&mut html_output, processed_events.into_iter());
    Ok((
        yaml_frontmatter_simplified(&state.metadata_parsed),
        html_output,
    ))
}

fn yaml_frontmatter_simplified(y: &Option<Yaml>) -> SimpleMetadata {
    // do i want to fail on yaml parse fail? or silently ignore?
    // for now, i'm ignoring, though I should at least print a warning
    match y.clone().unwrap_or(Yaml::Null).into_hash() {
        Some(y) => {
            let mut hm = HashMap::with_capacity(y.capacity());
            for (k, v) in y.iter() {
                match (k, v) {
                    (Yaml::String(key), Yaml::String(value)) => {
                        tracing::trace!("Frontmatter: {key} = {value}");
                        hm.insert(key.to_string(), value.to_string());
                    }
                    (Yaml::String(key), Yaml::Array(vals)) => {
                        let vals = vals
                            .iter()
                            .filter_map(|val| val.clone().into_string())
                            .fold(String::new(), |accum, i| {
                                let join = if !accum.is_empty() { ", " } else { "" };
                                accum + join + i.as_str()
                            });
                        tracing::trace!("Frontmatter: {key} = [{vals}]");
                        hm.insert(key.to_string(), vals);
                    }
                    (Yaml::String(key), Yaml::Hash(hash)) => {
                        tracing::trace!("Frontmatter: {key} = (nested hash)");
                        // TODO: recursively parse this, then modify all keys
                        // to have a leading `key.` before inserting
                        let hash = yaml_frontmatter_simplified(&Some(Yaml::Hash(hash.clone())));
                        for (k, v) in hash {
                            hm.insert(key.to_string() + "." + k.as_str(), v);
                        }
                    }
                    (Yaml::String(key), other_val) => {
                        tracing::trace!("Frontmatter: {key} = {:?}", &other_val);
                        if let Some(str_val) = other_val.clone().into_string() {
                            hm.insert(key.to_string(), str_val);
                        }
                    }
                    (k, v) => {
                        tracing::warn!("Unexpected frontmatter key-value: {:?} = {:?}", k, v);
                    }
                }
            }
            hm
        }
        None => HashMap::new(),
    }
}

/// Maximum bytes to read when extracting frontmatter metadata.
/// Frontmatter should always be at the top of the file, so 8KB is plenty.
const FRONTMATTER_MAX_BYTES: usize = 8 * 1024;

pub fn extract_metadata_from_file<P: AsRef<Path>>(
    path: P,
) -> Result<SimpleMetadata, Box<dyn std::error::Error>> {
    let path = path.as_ref();
    // Only read the first 8KB - frontmatter is always at the top
    let mut file = File::open(path)?;
    let file_len = file.metadata().map(|m| m.len() as usize).unwrap_or(0);
    let read_len = file_len.min(FRONTMATTER_MAX_BYTES);
    let mut buffer = vec![0u8; read_len];
    file.read_exact(&mut buffer)?;
    let markdown_input = String::from_utf8_lossy(&buffer);
    let parser = MDParser::new_ext(&markdown_input, Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    let parser = TextMergeStream::new(parser);
    let mut in_metadata = false;
    let hm = HashMap::new();
    for event in parser.take(4) {
        match &event {
            Event::Start(Tag::MetadataBlock(MetadataBlockKind::YamlStyle)) => {
                in_metadata = true;
            }
            Event::End(TagEnd::MetadataBlock(MetadataBlockKind::YamlStyle)) => {
                break;
            }
            Event::Text(text) => {
                if in_metadata {
                    let metadata_parsed =
                        YamlLoader::load_from_str(text).map(|ys| ys[0].clone()).ok();

                    return Ok(yaml_frontmatter_simplified(&metadata_parsed));
                }
            }
            _ => {}
        }
    }
    Ok(hm)
}

async fn process_event(
    event: pulldown_cmark::Event<'_>,
    mut state: EventState,
) -> (pulldown_cmark::Event<'_>, EventState) {
    match &event {
        Event::Start(Tag::Image {
            link_type: _,
            dest_url,
            title,
            id: _,
        }) => match MediaEmbed::from_url_and_title(dest_url, title) {
            Some(media) => {
                // the link title is actually the next Text event so need to split this to only produce the open tags
                let html = media.to_html(true);
                state.current_media = Some(media);
                (Event::Html(html.into()), state)
            }
            _ => (event.clone(), state),
        },
        Event::Start(Tag::MetadataBlock(v)) => {
            state.metadata_source = Some(*v);
            state.in_metadata = true;
            (event.clone(), state)
        }
        Event::End(TagEnd::MetadataBlock(_)) => {
            state.in_metadata = false;
            (event.clone(), state)
        }
        Event::End(TagEnd::Image) => {
            if let Some(media) = state.current_media.take() {
                (Event::Html(media.html_close().into()), state)
            } else {
                (event, state)
            }
        }
        // Track when we're inside a link (including autolinks like <http://...>)
        Event::Start(Tag::Link { .. }) => {
            state.in_link = true;
            (event.clone(), state)
        }
        Event::End(TagEnd::Link) => {
            state.in_link = false;
            (event, state)
        }
        Event::Text(text) => {
            // println!("Text: {}", &text);
            if state.in_metadata {
                state.metadata_parsed =
                    YamlLoader::load_from_str(text).map(|ys| ys[0].clone()).ok();
                (event, state)
            } else if let Some(remaining_text) = text.strip_prefix("[-] ") {
                // Canceled todo item: `- [-] canceled task` or `* [-] canceled task`
                let html = format!(
                    r#"<input disabled type="checkbox" class="canceled-checkbox"/><s>{}</s>"#,
                    html_escape::encode_text(remaining_text)
                );
                (Event::Html(html.into()), state)
            } else if !state.in_link && text.starts_with("http") && !text.contains(" ") {
                // Only process bare URLs that are NOT inside a link element.
                // URLs in <http://...> autolinks or [text](url) links are already
                // handled by markdown and shouldn't trigger oembed fetching.
                let info = PageInfo::new_from_url(text, state.oembed_timeout_ms)
                    .await
                    .unwrap_or(PageInfo {
                        url: text.clone().to_string(),
                        ..Default::default()
                    });
                (Event::Html(info.html().into()), state)
            } else if text.trim_start().starts_with("{{") {
                if let Some(vid) = Vid::from_vid(text) {
                    (Event::Html(vid.to_html(false).into()), state)
                } else {
                    (event, state)
                }
            } else {
                (event, state)
            }
        }
        // Event::Code(code) => {
        //     // TODO: detect mermaid
        //     println!("****** code: {}", &code);
        //     (event, state)
        // }
        _ => {
            // println!("Event: {:?}", &event);
            (event, state)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    async fn render_markdown(content: &str) -> String {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let (_, html) = render(path, &root, 100).await.unwrap();
        html
    }

    #[tokio::test]
    async fn test_canceled_checkbox_dash() {
        let md = "- [-] canceled task";
        let html = render_markdown(md).await;
        assert!(html.contains(r#"<input disabled type="checkbox" class="canceled-checkbox"/>"#));
        assert!(html.contains("<s>canceled task</s>"));
    }

    #[tokio::test]
    async fn test_canceled_checkbox_asterisk() {
        let md = "* [-] another canceled item";
        let html = render_markdown(md).await;
        assert!(html.contains(r#"<input disabled type="checkbox" class="canceled-checkbox"/>"#));
        assert!(html.contains("<s>another canceled item</s>"));
    }

    #[tokio::test]
    async fn test_unchecked_checkbox() {
        let md = "- [ ] unchecked item";
        let html = render_markdown(md).await;
        assert!(html.contains(r#"<input disabled="" type="checkbox"/>"#));
        assert!(!html.contains("canceled-checkbox"));
    }

    #[tokio::test]
    async fn test_checked_checkbox() {
        let md = "- [x] checked item";
        let html = render_markdown(md).await;
        assert!(html.contains(r#"<input disabled="" type="checkbox" checked=""/>"#));
        assert!(!html.contains("canceled-checkbox"));
    }

    #[tokio::test]
    async fn test_canceled_checkbox_with_special_chars() {
        // Test that special characters are preserved in canceled checkbox text
        let md = "- [-] text with special chars: & < > \"";
        let html = render_markdown(md).await;
        // The canceled checkbox renders with strikethrough
        assert!(html.contains("<s>"));
        assert!(html.contains("</s>"));
        assert!(html.contains("canceled-checkbox"));
    }

    #[tokio::test]
    async fn test_canceled_checkbox_plain_text() {
        // Verify canceled checkboxes work with plain text
        let md = "- [-] plain canceled text";
        let html = render_markdown(md).await;
        assert!(html.contains("<s>plain canceled text</s>"));
    }

    #[tokio::test]
    async fn test_yaml_frontmatter() {
        let md = "---\ntitle: Test Title\n---\n\n# Heading";
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(md.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let (metadata, _) = render(path, &root, 100).await.unwrap();
        assert_eq!(metadata.get("title"), Some(&"Test Title".to_string()));
    }

    // Media embed tests
    #[tokio::test]
    async fn test_video_embed_from_image_syntax() {
        let md = "![My Video](video.mp4)";
        let html = render_markdown(md).await;
        assert!(html.contains("<video"));
        assert!(html.contains("video.mp4"));
        assert!(html.contains("<figcaption>"));
        assert!(html.contains("My Video"));
        assert!(html.contains("</figcaption></figure>"));
    }

    #[tokio::test]
    async fn test_audio_embed_from_image_syntax() {
        let md = "![Episode 1](podcast.mp3)";
        let html = render_markdown(md).await;
        assert!(html.contains("<audio"));
        assert!(html.contains("audio-embed"));
        assert!(html.contains("podcast.mp3"));
        assert!(html.contains("<figcaption>"));
        assert!(html.contains("Episode 1"));
        assert!(html.contains("</figcaption></figure>"));
    }

    #[tokio::test]
    async fn test_youtube_embed_from_image_syntax() {
        let md = "![Watch this](https://www.youtube.com/watch?v=dQw4w9WgXcQ)";
        let html = render_markdown(md).await;
        assert!(html.contains("youtube-embed"));
        assert!(html.contains("youtube.com/embed/dQw4w9WgXcQ"));
        assert!(html.contains("<figcaption>"));
        assert!(html.contains("Watch this"));
        assert!(html.contains("</figcaption></figure>"));
    }

    #[tokio::test]
    async fn test_youtube_short_url_embed() {
        let md = "![](https://youtu.be/dQw4w9WgXcQ)";
        let html = render_markdown(md).await;
        assert!(html.contains("youtube-embed"));
        assert!(html.contains("youtube.com/embed/dQw4w9WgXcQ"));
    }

    #[tokio::test]
    async fn test_pdf_embed_from_image_syntax() {
        let md = "![Important Document](report.pdf)";
        let html = render_markdown(md).await;
        assert!(html.contains("pdf-embed"));
        assert!(html.contains(r#"data="report.pdf""#));
        assert!(html.contains(r#"type="application/pdf""#));
        assert!(html.contains("data-pdf-fallback"));
        assert!(html.contains("<figcaption>"));
        assert!(html.contains("Important Document"));
        assert!(html.contains("</figcaption></figure>"));
    }

    #[tokio::test]
    async fn test_pdf_embed_with_path() {
        let md = "![](docs/manual.pdf)";
        let html = render_markdown(md).await;
        assert!(html.contains("pdf-embed"));
        assert!(html.contains(r#"data="docs/manual.pdf""#));
    }

    #[tokio::test]
    async fn test_regular_image_not_converted() {
        let md = "![Alt text](photo.jpg)";
        let html = render_markdown(md).await;
        assert!(html.contains("<img"));
        assert!(html.contains("photo.jpg"));
        assert!(!html.contains("<video"));
        assert!(!html.contains("<audio"));
        assert!(!html.contains("pdf-embed"));
    }

    #[tokio::test]
    async fn test_multiple_media_types_in_document() {
        let md = r#"
# My Media

![Video](clip.mp4)

![Audio](song.mp3)

![PDF](doc.pdf)

![Image](photo.png)
"#;
        let html = render_markdown(md).await;
        assert!(html.contains("<video"));
        assert!(html.contains("<audio"));
        assert!(html.contains("pdf-embed"));
        assert!(html.contains("<img"));
    }
}

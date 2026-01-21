use crate::link_index::{OutboundLink, is_internal_link, split_url_anchor};
use crate::link_transform::{LinkTransformConfig, transform_link};
use crate::media::MediaEmbed;
use crate::oembed::PageInfo;
use crate::oembed_cache::OembedCache;
use crate::vid::Vid;
use pulldown_cmark::{
    CowStr, Event, HeadingLevel, MetadataBlockKind, Options, Parser as MDParser, Tag, TagEnd,
    TextMergeStream,
};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
use yaml_rust2::{Yaml, YamlLoader};

/// Represents a heading in the document for table of contents generation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HeadingInfo {
    pub level: u8,
    pub text: String,
    pub id: String,
}

struct EventState {
    #[allow(dead_code)] // Reserved for future use (resolving relative paths)
    root_path: PathBuf,
    /// Track the current media embed type (if any) for proper closing tags
    current_media: Option<MediaEmbed>,
    in_metadata: bool,
    in_link: bool, // Track when inside a link (including autolinks like <http://...>)
    metadata_source: Option<MetadataBlockKind>,
    metadata_parsed: Option<Yaml>,
    /// Configuration for transforming relative links
    link_transform_config: LinkTransformConfig,
    /// Pre-fetched oembed results for bare URLs (populated during parallel fetch phase)
    prefetched_oembed: HashMap<String, PageInfo>,
    /// True in server/GUI mode, false in build/CLI mode
    server_mode: bool,
    /// True when dynamic video transcoding is enabled
    transcode_enabled: bool,
    /// Collected outbound links from the document
    collected_links: Vec<OutboundLink>,
    /// Current link destination URL being processed (set on Start(Link))
    current_link_dest: Option<String>,
    /// Current link text being accumulated
    current_link_text: String,
}

pub type SimpleMetadata = HashMap<String, String>;

/// First pass: extract headings and generate anchor IDs
fn extract_headings(markdown_input: &str) -> (Vec<HeadingInfo>, HashMap<String, String>) {
    let parser = MDParser::new_ext(markdown_input, Options::all());
    let parser = TextMergeStream::new(parser);

    let mut headings = Vec::new();
    let mut anchor_ids: HashMap<String, usize> = HashMap::new();
    let mut heading_id_map: HashMap<String, String> = HashMap::new(); // Maps heading text to ID
    let mut in_heading: Option<(HeadingLevel, String)> = None;
    let mut heading_index = 0;

    for event in parser {
        match event {
            Event::Start(Tag::Heading {
                level,
                id: _,
                classes: _,
                attrs: _,
            }) => {
                in_heading = Some((level, String::new()));
            }
            Event::Text(ref text) => {
                if let Some((level, ref mut heading_text)) = in_heading {
                    heading_text.push_str(text);
                    in_heading = Some((level, heading_text.clone()));
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((heading_level, text)) = in_heading.take() {
                    let id = generate_anchor_id(&text, &mut anchor_ids);
                    let level_num = match heading_level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        HeadingLevel::H4 => 4,
                        HeadingLevel::H5 => 5,
                        HeadingLevel::H6 => 6,
                    };

                    headings.push(HeadingInfo {
                        level: level_num,
                        text: text.clone(),
                        id: id.clone(),
                    });

                    // Store mapping for second pass
                    heading_id_map.insert(format!("{}:{}", heading_index, text), id);
                    heading_index += 1;
                }
            }
            _ => {}
        }
    }

    (headings, heading_id_map)
}

pub async fn render(
    file: PathBuf,
    root_path: &Path,
    oembed_timeout_ms: u64,
    link_transform_config: LinkTransformConfig,
    server_mode: bool,
    transcode_enabled: bool,
) -> Result<(SimpleMetadata, Vec<HeadingInfo>, String, Vec<OutboundLink>), Box<dyn std::error::Error>>
{
    render_with_cache(
        file,
        root_path,
        oembed_timeout_ms,
        link_transform_config,
        None,
        server_mode,
        transcode_enabled,
    )
    .await
}

/// Renders markdown to HTML with optional OEmbed caching support.
///
/// When `oembed_cache` is provided, cached results are used when available and
/// new results are cached for future use. URLs are fetched in parallel for improved
/// performance when multiple bare URLs are present in the document.
///
/// Returns: (frontmatter, headings, html, outbound_links)
///
/// - `server_mode`: True in server/GUI mode, false in build/CLI mode
/// - `transcode_enabled`: True when dynamic video transcoding is enabled
pub async fn render_with_cache(
    file: PathBuf,
    root_path: &Path,
    oembed_timeout_ms: u64,
    link_transform_config: LinkTransformConfig,
    oembed_cache: Option<Arc<OembedCache>>,
    server_mode: bool,
    transcode_enabled: bool,
) -> Result<(SimpleMetadata, Vec<HeadingInfo>, String, Vec<OutboundLink>), Box<dyn std::error::Error>>
{
    // Read markdown input
    let markdown_input = fs::read_to_string(&file)?;

    // First pass: extract headings and generate IDs
    let (headings, _heading_id_map) = extract_headings(&markdown_input);

    // Second pass: collect and inject heading IDs into events
    let parser = MDParser::new_ext(&markdown_input, Options::all());
    let parser = TextMergeStream::new(parser);

    let mut events_with_ids = Vec::new();
    let mut heading_index = 0;
    let mut in_heading_text = None;

    for event in parser {
        match &event {
            Event::Start(Tag::Heading {
                level: _,
                id: _,
                classes: _,
                attrs: _,
            }) => {
                in_heading_text = Some(String::new());
                // We'll modify this event after we collect the text
                events_with_ids.push(event);
            }
            Event::Text(text) if in_heading_text.is_some() => {
                if let Some(ref mut heading_text) = in_heading_text {
                    heading_text.push_str(text);
                }
                events_with_ids.push(event);
            }
            Event::End(TagEnd::Heading(_)) => {
                // Now we have the full heading text, find the matching Start event and inject ID
                if in_heading_text.take().is_some() && heading_index < headings.len() {
                    let heading_info = &headings[heading_index];
                    // Go back and modify the Start(Heading) event
                    // Find the last Start(Heading) event
                    for i in (0..events_with_ids.len()).rev() {
                        if let Event::Start(Tag::Heading {
                            level,
                            id: _,
                            classes,
                            attrs,
                        }) = &events_with_ids[i]
                        {
                            // Replace it with one that has the ID
                            events_with_ids[i] = Event::Start(Tag::Heading {
                                level: *level,
                                id: Some(CowStr::from(heading_info.id.clone())),
                                classes: classes.clone(),
                                attrs: attrs.clone(),
                            });
                            break;
                        }
                    }
                    heading_index += 1;
                }
                events_with_ids.push(event);
            }
            _ => {
                events_with_ids.push(event);
            }
        }
    }

    // Collect bare URLs that need oembed fetching and fetch them in parallel
    let prefetched_oembed =
        prefetch_oembed_urls(&events_with_ids, oembed_timeout_ms, &oembed_cache).await;

    // Third pass: process events through our custom logic
    let mut state = EventState {
        root_path: root_path.to_path_buf(),
        current_media: None,
        in_metadata: false,
        in_link: false,
        metadata_source: None,
        metadata_parsed: None,
        link_transform_config,
        prefetched_oembed,
        server_mode,
        transcode_enabled,
        collected_links: Vec::new(),
        current_link_dest: None,
        current_link_text: String::new(),
    };
    let mut processed_events = Vec::new();

    for event in events_with_ids {
        let (processed, new_state) = process_event(event, state);
        state = new_state;
        processed_events.push(processed);
    }

    // Write to a new String buffer.
    let mut html_output = String::with_capacity(markdown_input.capacity() * 2);
    crate::html::push_html(&mut html_output, processed_events.into_iter());
    Ok((
        yaml_frontmatter_simplified(&state.metadata_parsed),
        headings,
        html_output,
        state.collected_links,
    ))
}

/// Pre-pass to collect all bare URLs that need oembed fetching.
///
/// This identifies text events that look like bare URLs (start with "http", no spaces,
/// and not inside a link element). These URLs are then fetched in parallel for better
/// performance.
fn collect_bare_urls(events: &[Event<'_>]) -> HashSet<String> {
    let mut urls = HashSet::new();
    let mut in_link = false;
    let mut in_metadata = false;

    for event in events {
        match event {
            Event::Start(Tag::Link { .. }) => in_link = true,
            Event::End(TagEnd::Link) => in_link = false,
            Event::Start(Tag::MetadataBlock(_)) => in_metadata = true,
            Event::End(TagEnd::MetadataBlock(_)) => in_metadata = false,
            Event::Text(text) => {
                if !in_link
                    && !in_metadata
                    && text.starts_with("http")
                    && !text.contains(' ')
                    && !text.trim_start().starts_with("{{")
                {
                    urls.insert(text.to_string());
                }
            }
            _ => {}
        }
    }

    urls
}

/// Fetches oembed data for a collection of URLs in parallel.
///
/// Uses the cache when available to avoid redundant network requests.
/// New results are stored in the cache for future use.
async fn prefetch_oembed_urls(
    events: &[Event<'_>],
    oembed_timeout_ms: u64,
    oembed_cache: &Option<Arc<OembedCache>>,
) -> HashMap<String, PageInfo> {
    let urls = collect_bare_urls(events);

    if urls.is_empty() {
        return HashMap::new();
    }

    tracing::debug!("oembed prefetch: found {} bare URLs to fetch", urls.len());

    // Partition URLs into cached and uncached
    let (cached, uncached): (Vec<_>, Vec<_>) = urls
        .into_iter()
        .partition(|url| oembed_cache.as_ref().and_then(|c| c.get(url)).is_some());

    let mut results = HashMap::new();

    // Add cached results
    if let Some(cache) = oembed_cache {
        for url in cached {
            if let Some(info) = cache.get(&url) {
                results.insert(url, info);
            }
        }
    }

    // Fetch uncached URLs in parallel
    if !uncached.is_empty() {
        tracing::debug!(
            "oembed prefetch: {} cached, {} to fetch",
            results.len(),
            uncached.len()
        );

        let fetch_futures: Vec<_> = uncached
            .into_iter()
            .map(|url| async move {
                tracing::debug!("oembed fetch start: {}", url);
                let result = PageInfo::new_from_url(&url, oembed_timeout_ms)
                    .await
                    .unwrap_or_else(|_| PageInfo {
                        url: url.clone(),
                        ..Default::default()
                    });
                tracing::debug!("oembed fetch complete: {}", url);
                (url, result)
            })
            .collect();

        let fetched: Vec<_> = futures::future::join_all(fetch_futures).await;

        // Store results and cache them
        for (url, info) in fetched {
            if let Some(cache) = oembed_cache {
                cache.insert(url.clone(), info.clone());
            }
            results.insert(url, info);
        }
    }

    results
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
                    (Yaml::String(key), Yaml::Integer(val)) => {
                        tracing::trace!("Frontmatter: {key} = {val}");
                        hm.insert(key.to_string(), val.to_string());
                    }
                    (Yaml::String(key), Yaml::Real(val)) => {
                        tracing::trace!("Frontmatter: {key} = {val}");
                        hm.insert(key.to_string(), val.to_string());
                    }
                    (Yaml::String(key), Yaml::Boolean(val)) => {
                        tracing::trace!("Frontmatter: {key} = {val}");
                        hm.insert(key.to_string(), val.to_string());
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

/// Generates a URL-safe anchor ID from heading text.
/// Handles duplicates by appending -2, -3, etc.
fn generate_anchor_id(text: &str, anchor_ids: &mut HashMap<String, usize>) -> String {
    // Convert to lowercase and replace spaces and special chars with dashes
    let base_id = text
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else if c.is_whitespace() {
                '-'
            } else {
                // Remove special characters
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");

    // Handle empty IDs
    let base_id = if base_id.is_empty() {
        "heading".to_string()
    } else {
        base_id
    };

    // Check for duplicates and increment counter
    let count = anchor_ids.entry(base_id.clone()).or_insert(0);
    *count += 1;

    if *count == 1 {
        base_id
    } else {
        format!("{}-{}", base_id, count)
    }
}

/// Processes a single markdown event, transforming it as needed.
///
/// This function is now synchronous because all async work (oembed fetching)
/// is done in the prefetch phase. Bare URLs are looked up in the prefetched
/// results instead of being fetched inline.
fn process_event(
    event: pulldown_cmark::Event<'_>,
    mut state: EventState,
) -> (pulldown_cmark::Event<'_>, EventState) {
    match &event {
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => {
            // Transform the URL first for trailing-slash URL convention
            // This applies to all images/media, not just regular images
            let transformed_url = transform_link(dest_url, &state.link_transform_config);

            match MediaEmbed::from_url_and_title(&transformed_url, title) {
                Some(media) => {
                    // the link title is actually the next Text event so need to split this to only produce the open tags
                    let html = media.to_html(true, state.server_mode, state.transcode_enabled);
                    state.current_media = Some(media);
                    (Event::Html(html.into()), state)
                }
                _ => {
                    let new_event = Event::Start(Tag::Image {
                        link_type: *link_type,
                        dest_url: CowStr::from(transformed_url),
                        title: title.clone(),
                        id: id.clone(),
                    });
                    (new_event, state)
                }
            }
        }
        Event::End(TagEnd::Image) => {
            if let Some(media) = state.current_media.take() {
                (Event::Html(media.html_close().into()), state)
            } else {
                (event, state)
            }
        }
        Event::Start(Tag::MetadataBlock(v)) => {
            state.metadata_source = Some(*v);
            state.in_metadata = true;
            (event.clone(), state)
        }
        Event::End(TagEnd::MetadataBlock(_)) => {
            state.in_metadata = false;
            (event.clone(), state)
        }
        // Track when we're inside a link (including autolinks like <http://...>)
        // and transform the link URL for trailing-slash URL convention
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => {
            state.in_link = true;
            // Store the original destination URL for link tracking
            state.current_link_dest = Some(dest_url.to_string());
            state.current_link_text.clear();
            let transformed_url = transform_link(dest_url, &state.link_transform_config);
            let new_event = Event::Start(Tag::Link {
                link_type: *link_type,
                dest_url: CowStr::from(transformed_url),
                title: title.clone(),
                id: id.clone(),
            });
            (new_event, state)
        }
        Event::End(TagEnd::Link) => {
            state.in_link = false;
            // Collect the outbound link
            if let Some(dest_url) = state.current_link_dest.take() {
                let (path, anchor) = split_url_anchor(&dest_url);
                let internal = is_internal_link(&dest_url);
                let link = OutboundLink {
                    to: path,
                    text: std::mem::take(&mut state.current_link_text),
                    anchor,
                    internal,
                };
                state.collected_links.push(link);
            }
            (event, state)
        }
        Event::Text(text) => {
            // Accumulate link text when inside a link
            if state.in_link {
                state.current_link_text.push_str(text);
            }
            if state.in_metadata {
                state.metadata_parsed = YamlLoader::load_from_str(text)
                    .ok()
                    .and_then(|ys| ys.into_iter().next());
                (event, state)
            } else if let Some(remaining_text) = text.strip_prefix("[-] ") {
                // Canceled todo item: `- [-] canceled task` or `* [-] canceled task`
                let html = format!(
                    r#"<input disabled type="checkbox" class="canceled-checkbox"/><s>{}</s>"#,
                    html_escape::encode_text(remaining_text)
                );
                (Event::Html(html.into()), state)
            } else if !state.in_link && text.starts_with("http") && !text.contains(' ') {
                // Only process bare URLs that are NOT inside a link element.
                // URLs in <http://...> autolinks or [text](url) links are already
                // handled by markdown and shouldn't trigger oembed fetching.
                //
                // Look up the prefetched result instead of fetching inline.
                let url_str = text.to_string();
                let info = state
                    .prefetched_oembed
                    .get(&url_str)
                    .cloned()
                    .unwrap_or_else(|| PageInfo {
                        url: url_str,
                        ..Default::default()
                    });
                (Event::Html(info.html().into()), state)
            } else if text.trim_start().starts_with("{{") {
                if let Some(vid) = Vid::from_vid(text) {
                    (
                        Event::Html(
                            vid.to_html(false, state.server_mode, state.transcode_enabled)
                                .into(),
                        ),
                        state,
                    )
                } else {
                    (event, state)
                }
            } else {
                (event, state)
            }
        }
        _ => (event, state),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    async fn render_markdown(content: &str) -> String {
        render_markdown_with_config(content, false).await
    }

    async fn render_markdown_with_config(content: &str, is_index_file: bool) -> String {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file,
        };
        // Tests run with server_mode=false, transcode_enabled=false
        let (_, _, html, _) = render(path, &root, 100, config, false, false)
            .await
            .unwrap();
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
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
        };
        let (metadata, _, _, _) = render(path, &root, 100, config, false, false)
            .await
            .unwrap();
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
        // URL is transformed for trailing-slash convention (../report.pdf for non-index files)
        assert!(
            html.contains(r#"data="../report.pdf""#),
            "PDF URL should be transformed. Got: {}",
            html
        );
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
        // URL is transformed for trailing-slash convention (../docs/manual.pdf for non-index files)
        assert!(
            html.contains(r#"data="../docs/manual.pdf""#),
            "PDF URL should be transformed. Got: {}",
            html
        );
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

    #[tokio::test]
    async fn test_vid_shortcode() {
        let md = r#"{{ vid(path="test/video.mp4") }}"#;
        let html = render_markdown(md).await;
        println!("Output HTML: {}", &html);
        assert!(html.contains("<video"), "Should contain video element");
        assert!(
            html.contains("/videos/test/video.mp4"),
            "Should contain video path"
        );
    }

    #[tokio::test]
    async fn test_vid_shortcode_with_spaces() {
        let md = r#"{{ vid(path="Eric Jones/Eric Jones - Metal 3.mp4")}}"#;
        let html = render_markdown(md).await;
        println!("Output HTML: {}", &html);
        assert!(html.contains("<video"), "Should contain video element");
        assert!(
            html.contains("/videos/Eric%20Jones"),
            "Should contain URL-encoded path"
        );
    }

    // Link transformation tests
    #[tokio::test]
    async fn test_link_transformation_regular_markdown() {
        // Regular markdown file (not index) - links get ../ prefix
        let md = "[Other Doc](other.md)";
        let html = render_markdown_with_config(md, false).await;
        assert!(
            html.contains(r#"href="../other/""#),
            "Regular markdown should transform other.md to ../other/. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_link_transformation_index_file() {
        // Index file - links don't get ../ prefix
        let md = "[Other Doc](other.md)";
        let html = render_markdown_with_config(md, true).await;
        assert!(
            html.contains(r#"href="other/""#),
            "Index file should transform other.md to other/. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_link_transformation_preserves_absolute_urls() {
        let md = "[External](https://example.com)";
        let html = render_markdown(md).await;
        assert!(
            html.contains(r#"href="https://example.com""#),
            "Absolute URLs should remain unchanged"
        );
    }

    #[tokio::test]
    async fn test_link_transformation_with_anchor() {
        let md = "[Section](other.md#section)";
        let html = render_markdown_with_config(md, false).await;
        assert!(
            html.contains(r#"href="../other/#section""#),
            "Links with anchors should transform correctly. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_image_transformation_regular_markdown() {
        // Regular images (not media embeds) should also be transformed
        let md = "![Alt](images/photo.jpg)";
        let html = render_markdown_with_config(md, false).await;
        assert!(
            html.contains(r#"src="../images/photo.jpg""#),
            "Image URLs should be transformed. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_image_transformation_index_file() {
        let md = "![Alt](images/photo.jpg)";
        let html = render_markdown_with_config(md, true).await;
        assert!(
            html.contains(r#"src="images/photo.jpg""#),
            "Index file image URLs shouldn't get ../. Got: {}",
            html
        );
    }

    // Media embed URL transformation tests
    #[tokio::test]
    async fn test_video_embed_url_transformation() {
        // Video embeds in regular markdown files should get ../ prefix
        let md = "![My Video](video.mp4)";
        let html = render_markdown_with_config(md, false).await;
        assert!(
            html.contains("../video.mp4"),
            "Video URLs should be transformed with ../. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_video_embed_url_transformation_index_file() {
        // Video embeds in index files should NOT get ../ prefix
        let md = "![My Video](video.mp4)";
        let html = render_markdown_with_config(md, true).await;
        assert!(
            !html.contains("../video.mp4"),
            "Index file video URLs shouldn't get ../. Got: {}",
            html
        );
        assert!(
            html.contains("video.mp4"),
            "Video URL should be present. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_audio_embed_url_transformation() {
        // Audio embeds in regular markdown files should get ../ prefix
        let md = "![Podcast](episode.mp3)";
        let html = render_markdown_with_config(md, false).await;
        assert!(
            html.contains("../episode.mp3"),
            "Audio URLs should be transformed with ../. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_pdf_embed_url_transformation() {
        // PDF embeds in regular markdown files should get ../ prefix
        let md = "![Document](report.pdf)";
        let html = render_markdown_with_config(md, false).await;
        assert!(
            html.contains("../report.pdf"),
            "PDF URLs should be transformed with ../. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_pdf_embed_url_transformation_index_file() {
        // PDF embeds in index files should NOT get ../ prefix
        let md = "![Document](report.pdf)";
        let html = render_markdown_with_config(md, true).await;
        assert!(
            !html.contains("../report.pdf"),
            "Index file PDF URLs shouldn't get ../. Got: {}",
            html
        );
        assert!(
            html.contains("report.pdf"),
            "PDF URL should be present. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_media_embed_peer_file_transformation() {
        // Test the specific bug case: peer file in same folder as markdown
        // Markdown: docs/guide.md references peer-video.mp4 (docs/peer-video.mp4)
        // When served at /docs/guide/, browser sees ../peer-video.mp4 → /docs/peer-video.mp4 (correct!)
        let md = "![](peer-video.mp4)";
        let html = render_markdown_with_config(md, false).await;
        assert!(
            html.contains("../peer-video.mp4"),
            "Peer file video should get ../ prefix. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_media_embed_explicit_relative_path() {
        // Test ./file.mp4 syntax also gets transformed correctly
        let md = "![](./peer-video.mp4)";
        let html = render_markdown_with_config(md, false).await;
        assert!(
            html.contains("../peer-video.mp4"),
            "./peer-video.mp4 should transform to ../peer-video.mp4. Got: {}",
            html
        );
    }
}

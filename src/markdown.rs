use crate::attrs::ParsedAttrs;
use crate::errors::MarkdownError;
use crate::link_index::{OutboundLink, is_internal_link, split_url_anchor};
use crate::link_transform::{LinkTransformConfig, transform_link};
use crate::media::MediaEmbed;
use crate::oembed::PageInfo;
use crate::oembed_cache::OembedCache;
use crate::vid::Vid;
use crate::wikilink::{parse_tag_link, transform_wikilinks};
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

/// Markdown parser options.
///
/// Uses `Options::all()` to enable all pulldown-cmark features including wikilinks.
///
/// Wikilink processing flow:
/// 1. `transform_wikilinks` runs FIRST on raw markdown, converting tag-style wikilinks
///    like `[[Tags:rust]]` to standard markdown links `[rust](/tags/rust/)`
/// 2. pulldown-cmark then parses the result, handling plain wikilinks like `[[Whatever]]`
///    natively with its ENABLE_WIKILINKS support
///
/// This hybrid approach allows us to:
/// - Support custom tag-source links (`[[Source:value]]`)
/// - Preserve standard wikilink behavior for plain `[[page]]` links
fn markdown_options() -> Options {
    Options::all()
}

/// Represents a heading in the document for table of contents generation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HeadingInfo {
    pub level: u8,
    pub text: String,
    pub id: String,
}

/// Result of rendering a markdown file to HTML.
///
/// Contains the rendered HTML along with metadata extracted during parsing.
#[derive(Debug, Clone)]
pub struct MarkdownRenderResult {
    /// Frontmatter metadata (from YAML block at top of file)
    pub frontmatter: SimpleMetadata,
    /// Table of contents (headings extracted from document)
    pub headings: Vec<HeadingInfo>,
    /// Rendered HTML content
    pub html: String,
    /// Links discovered during rendering (for backlink tracking)
    pub outbound_links: Vec<OutboundLink>,
    /// True if the document's first heading is an H1 (affects title rendering)
    pub has_h1: bool,
    /// Word count of the document (excluding code blocks and metadata)
    pub word_count: usize,
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
    /// Valid tag sources for detecting tag links (e.g., "tags", "performers")
    valid_tag_sources: HashSet<String>,
    /// Word count accumulator for text content
    word_count: usize,
    /// Track if we're inside a code block (to exclude from word count)
    in_code_block: bool,
}

pub type SimpleMetadata = HashMap<String, serde_json::Value>;

/// Extracts the first H1 heading text from markdown content.
///
/// This is used to provide a title fallback when no frontmatter title exists.
/// Only extracts the first H1 found; subsequent H1s are ignored.
pub fn extract_first_h1(markdown_input: &str) -> Option<String> {
    // Use minimal parser options: only YAML metadata (to skip frontmatter blocks)
    // ATX headings are parsed by default without any feature flags
    let parser = MDParser::new_ext(markdown_input, Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    let parser = TextMergeStream::new(parser);

    let mut in_h1 = false;
    let mut h1_text = String::new();

    for event in parser {
        match event {
            Event::Start(Tag::Heading {
                level: HeadingLevel::H1,
                ..
            }) => {
                in_h1 = true;
            }
            Event::Text(text) if in_h1 => {
                h1_text.push_str(&text);
            }
            Event::End(TagEnd::Heading(HeadingLevel::H1)) => {
                if !h1_text.is_empty() {
                    return Some(h1_text);
                }
                in_h1 = false;
            }
            _ => {}
        }
    }
    None
}

/// Em dash character (U+2014) - what `---` becomes with smart punctuation
const EM_DASH: &str = "\u{2014}";

/// Transform events: detect `--- {attrs}` pattern and convert to Rule + attrs.
///
/// When pulldown-cmark (with TextMergeStream) sees `--- {#id .class}` on a single line,
/// it produces:
/// - Start(Paragraph)
/// - Text("— {#id .class}") (em dash + space + attrs, merged into one Text)
/// - End(Paragraph)
///
/// This function detects that pattern and transforms it into a single Rule event,
/// extracting the attributes for section rendering.
///
/// Returns (transformed_events, section_attrs) where section_attrs maps section
/// index to parsed attributes.
fn transform_rule_attrs(events: Vec<Event<'_>>) -> (Vec<Event<'_>>, HashMap<usize, ParsedAttrs>) {
    let mut result = Vec::with_capacity(events.len());
    let mut section_attrs = HashMap::new();
    let mut section_index = 0;
    let mut i = 0;

    while i < events.len() {
        // Detect pattern: Start(Paragraph), Text("— {attrs}"), End(Paragraph)
        // TextMergeStream merges adjacent Text events, so we see a single Text event
        if i + 2 < events.len()
            && let (Event::Start(Tag::Paragraph), Event::Text(text), Event::End(TagEnd::Paragraph)) =
                (&events[i], &events[i + 1], &events[i + 2])
            // Check: text starts with em dash + space + "{" and ends with "}"
            && text.starts_with(EM_DASH)
            && let Some(attrs_str) = text.strip_prefix(EM_DASH)
            && attrs_str.starts_with(" {")
            && attrs_str.ends_with('}')
            && let Some(attrs) = ParsedAttrs::parse(attrs_str.trim())
        {
            // Transform: emit Rule instead of paragraph
            result.push(Event::Rule);
            section_index += 1;
            section_attrs.insert(section_index, attrs);
            i += 3; // Skip all 3 events
            continue;
        }

        // Track real Rule events for section counting
        if matches!(&events[i], Event::Rule) {
            section_index += 1;
        }

        result.push(events[i].clone());
        i += 1;
    }

    (result, section_attrs)
}

pub async fn render(
    file: PathBuf,
    root_path: &Path,
    oembed_timeout_ms: u64,
    link_transform_config: LinkTransformConfig,
    server_mode: bool,
    transcode_enabled: bool,
    valid_tag_sources: HashSet<String>,
) -> Result<MarkdownRenderResult, MarkdownError> {
    render_with_cache(
        file,
        root_path,
        oembed_timeout_ms,
        link_transform_config,
        None,
        server_mode,
        transcode_enabled,
        valid_tag_sources,
    )
    .await
}

/// Renders markdown to HTML with optional OEmbed caching support.
///
/// When `oembed_cache` is provided, cached results are used when available and
/// new results are cached for future use. URLs are fetched in parallel for improved
/// performance when multiple bare URLs are present in the document.
///
/// - `server_mode`: True in server/GUI mode, false in build/CLI mode
/// - `transcode_enabled`: True when dynamic video transcoding is enabled
/// - `valid_tag_sources`: Set of valid tag source names for wikilink transformation
#[allow(clippy::too_many_arguments)]
pub async fn render_with_cache(
    file: PathBuf,
    root_path: &Path,
    oembed_timeout_ms: u64,
    link_transform_config: LinkTransformConfig,
    oembed_cache: Option<Arc<OembedCache>>,
    server_mode: bool,
    transcode_enabled: bool,
    valid_tag_sources: HashSet<String>,
) -> Result<MarkdownRenderResult, MarkdownError> {
    // Read markdown input
    let raw_markdown_input = fs::read_to_string(&file).map_err(|e| MarkdownError::ReadFailed {
        path: file.clone(),
        source: e,
    })?;

    // Transform [[Source:value]] wikilinks to standard markdown links before parsing
    let markdown_input = if valid_tag_sources.is_empty() {
        raw_markdown_input
    } else {
        transform_wikilinks(&raw_markdown_input, &valid_tag_sources)
    };

    // Single pass: collect events, extract headings, and inject anchor IDs inline
    let parser = MDParser::new_ext(&markdown_input, markdown_options());
    let parser = TextMergeStream::new(parser);

    let mut events_with_ids = Vec::new();
    let mut headings = Vec::new();
    let mut anchor_ids: HashMap<String, usize> = HashMap::new();
    let mut in_heading_text: Option<String> = None;

    for event in parser {
        match &event {
            Event::Start(Tag::Heading { .. }) => {
                in_heading_text = Some(String::new());
                events_with_ids.push(event);
            }
            Event::Text(text) if in_heading_text.is_some() => {
                if let Some(ref mut heading_text) = in_heading_text {
                    heading_text.push_str(text);
                }
                events_with_ids.push(event);
            }
            Event::End(TagEnd::Heading(heading_level)) => {
                if let Some(text) = in_heading_text.take() {
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

                    // Walk backward to find the matching Start(Heading) and inject the ID
                    for i in (0..events_with_ids.len()).rev() {
                        if let Event::Start(Tag::Heading {
                            level,
                            id: _,
                            classes,
                            attrs,
                        }) = &events_with_ids[i]
                        {
                            events_with_ids[i] = Event::Start(Tag::Heading {
                                level: *level,
                                id: Some(CowStr::from(id)),
                                classes: classes.clone(),
                                attrs: attrs.clone(),
                            });
                            break;
                        }
                    }
                }
                events_with_ids.push(event);
            }
            _ => {
                events_with_ids.push(event);
            }
        }
    }

    // Detect if the first heading is an H1 (used for conditional title rendering in templates)
    let has_h1 = headings.first().is_some_and(|h| h.level == 1);

    // Transform rule attrs: detect `--- {attrs}` pattern and convert to Rule + attrs
    let (events_with_ids, section_attrs) = transform_rule_attrs(events_with_ids);

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
        valid_tag_sources,
        word_count: 0,
        in_code_block: false,
    };
    let mut processed_events = Vec::new();

    for event in events_with_ids {
        let (processed, new_state) = process_event(event, state);
        state = new_state;
        processed_events.push(processed);
    }

    // Write to a new String buffer with MBR extensions (sections, mermaid)
    let mut html_output = String::with_capacity(markdown_input.capacity() * 2);

    // Deduplicate outbound links by target URL - if a page links to the same
    // target multiple times, we only keep the first occurrence
    let mut seen_targets: HashSet<String> = HashSet::new();
    let deduplicated_links: Vec<OutboundLink> = state
        .collected_links
        .into_iter()
        .filter(|link| seen_targets.insert(link.to.clone()))
        .collect();

    crate::html::push_html_mbr_with_attrs(
        &mut html_output,
        processed_events.into_iter(),
        section_attrs,
    );

    // Extract frontmatter and inject H1 title if no frontmatter title exists
    let mut frontmatter = yaml_frontmatter_simplified(&state.metadata_parsed);
    if !frontmatter.contains_key("title")
        && let Some(h1_text) = headings
            .first()
            .filter(|h| h.level == 1)
            .map(|h| h.text.clone())
    {
        frontmatter.insert("title".to_string(), serde_json::Value::String(h1_text));
    }

    Ok(MarkdownRenderResult {
        frontmatter,
        headings,
        html: html_output,
        outbound_links: deduplicated_links,
        has_h1,
        word_count: state.word_count,
    })
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
    match y.as_ref().and_then(|yaml| yaml.as_hash()) {
        Some(hash) => yaml_hash_to_metadata(hash),
        None => HashMap::new(),
    }
}

/// Converts a YAML hash to simplified metadata, borrowing instead of cloning.
fn yaml_hash_to_metadata(hash: &yaml_rust2::yaml::Hash) -> SimpleMetadata {
    let mut hm = HashMap::with_capacity(hash.len());
    for (k, v) in hash.iter() {
        match (k, v) {
            (Yaml::String(key), Yaml::String(value)) => {
                tracing::trace!("Frontmatter: {key} = {value}");
                hm.insert(key.clone(), serde_json::Value::String(value.clone()));
            }
            (Yaml::String(key), Yaml::Array(vals)) => {
                // Preserve arrays as JSON arrays instead of joining them
                let arr: Vec<serde_json::Value> = vals
                    .iter()
                    .filter_map(|val| val.as_str())
                    .map(|s| serde_json::Value::String(s.to_string()))
                    .collect();
                tracing::trace!("Frontmatter: {key} = {:?}", &arr);
                hm.insert(key.clone(), serde_json::Value::Array(arr));
            }
            (Yaml::String(key), Yaml::Hash(nested_hash)) => {
                tracing::trace!("Frontmatter: {key} = (nested hash)");
                // Recursively parse nested hashes and flatten with dot notation
                let nested = yaml_hash_to_metadata(nested_hash);
                for (k, v) in nested {
                    hm.insert(key.to_string() + "." + k.as_str(), v);
                }
            }
            (Yaml::String(key), Yaml::Integer(val)) => {
                tracing::trace!("Frontmatter: {key} = {val}");
                hm.insert(key.clone(), serde_json::json!(val));
            }
            (Yaml::String(key), Yaml::Real(val)) => {
                tracing::trace!("Frontmatter: {key} = {val}");
                hm.insert(key.clone(), serde_json::Value::String(val.clone()));
            }
            (Yaml::String(key), Yaml::Boolean(val)) => {
                tracing::trace!("Frontmatter: {key} = {val}");
                hm.insert(key.clone(), serde_json::json!(val));
            }
            (Yaml::String(key), other_val) => {
                tracing::trace!("Frontmatter: {key} = {:?}", &other_val);
                if let Some(str_val) = other_val.as_str() {
                    hm.insert(key.clone(), serde_json::Value::String(str_val.to_string()));
                }
            }
            (k, v) => {
                tracing::warn!("Unexpected frontmatter key-value: {:?} = {:?}", k, v);
            }
        }
    }
    hm
}

/// Maximum bytes to read when extracting frontmatter metadata.
/// Frontmatter should always be at the top of the file, so 8KB is plenty.
const FRONTMATTER_MAX_BYTES: usize = 8 * 1024;

pub fn extract_metadata_from_file<P: AsRef<Path>>(
    path: P,
) -> Result<SimpleMetadata, MarkdownError> {
    let path = path.as_ref();
    // Only read the first 8KB - frontmatter is always at the top
    let mut file = File::open(path).map_err(|e| MarkdownError::ReadFailed {
        path: path.to_path_buf(),
        source: e,
    })?;
    let file_len = file.metadata().map(|m| m.len() as usize).unwrap_or(0);
    let read_len = file_len.min(FRONTMATTER_MAX_BYTES);
    let mut buffer = vec![0u8; read_len];
    file.read_exact(&mut buffer)
        .map_err(|e| MarkdownError::ReadFailed {
            path: path.to_path_buf(),
            source: e,
        })?;
    let markdown_input = String::from_utf8_lossy(&buffer);
    let parser = MDParser::new_ext(&markdown_input, Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    let parser = TextMergeStream::new(parser);
    let mut in_metadata = false;
    let mut hm = HashMap::new();
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

                    hm = yaml_frontmatter_simplified(&metadata_parsed);
                    break;
                }
            }
            _ => {}
        }
    }

    // If no frontmatter title, try to extract the first H1 from the content
    if !hm.contains_key("title")
        && let Some(h1_text) = extract_first_h1(&markdown_input)
    {
        hm.insert("title".to_string(), serde_json::Value::String(h1_text));
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
        // Also detect and transform tag links like [text](Tags:rust) -> [text](/tags/rust/)
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

            // First check if this is a tag link (e.g., Tags:rust, performers:Joshua Jay)
            // If so, transform to the tag URL path (/tags/rust/, /performers/joshua_jay/)
            let transformed_url =
                if let Some(wikilink) = parse_tag_link(dest_url, &state.valid_tag_sources) {
                    wikilink.url_path()
                } else {
                    // Not a tag link, use regular link transformation
                    transform_link(dest_url, &state.link_transform_config)
                };

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
        // Track code blocks to exclude from word count
        Event::Start(Tag::CodeBlock(_)) => {
            state.in_code_block = true;
            (event, state)
        }
        Event::End(TagEnd::CodeBlock) => {
            state.in_code_block = false;
            (event, state)
        }
        Event::Text(text) => {
            // Accumulate link text when inside a link
            if state.in_link {
                state.current_link_text.push_str(text);
            }
            // Count words in text content (excluding metadata and code blocks)
            if !state.in_metadata && !state.in_code_block {
                state.word_count += text.split_whitespace().count();
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
        render_markdown_with_config(content, false, HashSet::new()).await
    }

    async fn render_markdown_with_tags(content: &str, tag_sources: HashSet<String>) -> String {
        render_markdown_with_config(content, false, tag_sources).await
    }

    async fn render_markdown_with_config(
        content: &str,
        is_index_file: bool,
        tag_sources: HashSet<String>,
    ) -> String {
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
        let result = render(path, &root, 100, config, false, false, tag_sources)
            .await
            .unwrap();
        result.html
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
        let result = render(path, &root, 100, config, false, false, HashSet::new())
            .await
            .unwrap();
        assert_eq!(
            result.frontmatter.get("title"),
            Some(&serde_json::Value::String("Test Title".to_string()))
        );
    }

    // H1 extraction tests
    #[test]
    fn test_extract_first_h1_basic() {
        let md = "# Hello World\n\nSome content";
        let result = extract_first_h1(md);
        assert_eq!(result, Some("Hello World".to_string()));
    }

    #[test]
    fn test_extract_first_h1_with_inline_formatting() {
        let md = "# Hello **World**\n\nSome content";
        let result = extract_first_h1(md);
        assert_eq!(result, Some("Hello World".to_string()));
    }

    #[test]
    fn test_extract_first_h1_none_when_no_h1() {
        let md = "## This is H2\n\nSome content";
        let result = extract_first_h1(md);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_first_h1_returns_first_only() {
        let md = "# First H1\n\n# Second H1";
        let result = extract_first_h1(md);
        assert_eq!(result, Some("First H1".to_string()));
    }

    #[test]
    fn test_extract_first_h1_empty_doc() {
        let md = "";
        let result = extract_first_h1(md);
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_has_h1_true_when_first_heading_is_h1() {
        let md = "# Main Title\n\n## Subsection";
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(md.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
        };
        let result = render(path, &root, 100, config, false, false, HashSet::new())
            .await
            .unwrap();
        assert!(result.has_h1);
    }

    #[tokio::test]
    async fn test_has_h1_false_when_first_heading_is_h2() {
        let md = "## Subsection\n\n# Late H1";
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(md.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
        };
        let result = render(path, &root, 100, config, false, false, HashSet::new())
            .await
            .unwrap();
        assert!(!result.has_h1);
    }

    #[tokio::test]
    async fn test_title_fallback_from_h1() {
        // No frontmatter title, but has H1 - should extract title from H1
        let md = "# My Document Title\n\nSome content here.";
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(md.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
        };
        let result = render(path, &root, 100, config, false, false, HashSet::new())
            .await
            .unwrap();
        assert!(result.has_h1);
        assert_eq!(
            result.frontmatter.get("title"),
            Some(&serde_json::Value::String("My Document Title".to_string()))
        );
    }

    #[tokio::test]
    async fn test_frontmatter_title_takes_precedence() {
        // Frontmatter title should take precedence over H1
        let md = "---\ntitle: Frontmatter Title\n---\n\n# H1 Title";
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(md.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
        };
        let result = render(path, &root, 100, config, false, false, HashSet::new())
            .await
            .unwrap();
        assert!(result.has_h1);
        assert_eq!(
            result.frontmatter.get("title"),
            Some(&serde_json::Value::String("Frontmatter Title".to_string()))
        );
    }

    #[tokio::test]
    async fn test_no_title_when_no_frontmatter_and_no_h1() {
        // No frontmatter and no H1 - should have no title
        let md = "## Subsection\n\nSome content.";
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(md.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
        };
        let result = render(path, &root, 100, config, false, false, HashSet::new())
            .await
            .unwrap();
        assert!(!result.has_h1);
        assert_eq!(result.frontmatter.get("title"), None);
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
        let html = render_markdown_with_config(md, false, HashSet::new()).await;
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
        let html = render_markdown_with_config(md, true, HashSet::new()).await;
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
        let html = render_markdown_with_config(md, false, HashSet::new()).await;
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
        let html = render_markdown_with_config(md, false, HashSet::new()).await;
        assert!(
            html.contains(r#"src="../images/photo.jpg""#),
            "Image URLs should be transformed. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_image_transformation_index_file() {
        let md = "![Alt](images/photo.jpg)";
        let html = render_markdown_with_config(md, true, HashSet::new()).await;
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
        let html = render_markdown_with_config(md, false, HashSet::new()).await;
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
        let html = render_markdown_with_config(md, true, HashSet::new()).await;
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
        let html = render_markdown_with_config(md, false, HashSet::new()).await;
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
        let html = render_markdown_with_config(md, false, HashSet::new()).await;
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
        let html = render_markdown_with_config(md, true, HashSet::new()).await;
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
        let html = render_markdown_with_config(md, false, HashSet::new()).await;
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
        let html = render_markdown_with_config(md, false, HashSet::new()).await;
        assert!(
            html.contains("../peer-video.mp4"),
            "./peer-video.mp4 should transform to ../peer-video.mp4. Got: {}",
            html
        );
    }

    // Section attributes tests
    #[tokio::test]
    async fn test_section_attrs_with_id() {
        // Test that --- {#id} applies ID to the next section
        let md = "First section\n\n--- {#intro}\n\nSecond section";
        let html = render_markdown(md).await;
        assert!(
            html.contains(r#"<section id="intro">"#),
            "Section should have id='intro'. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_section_attrs_with_class() {
        // Test that --- {.highlight} applies class to the next section
        let md = "First section\n\n--- {.highlight}\n\nSecond section";
        let html = render_markdown(md).await;
        assert!(
            html.contains(r#"<section class="highlight">"#),
            "Section should have class='highlight'. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_section_attrs_with_multiple_classes() {
        // Test multiple classes
        let md = "First section\n\n--- {.slide .center}\n\nSecond section";
        let html = render_markdown(md).await;
        assert!(
            html.contains(r#"<section class="slide center">"#),
            "Section should have class='slide center'. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_section_attrs_with_data_attributes() {
        // Test data attributes
        let md = "First section\n\n--- {data-transition=\"slide\"}\n\nSecond section";
        let html = render_markdown(md).await;
        assert!(
            html.contains(r#"data-transition="slide""#),
            "Section should have data-transition='slide'. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_section_attrs_mixed() {
        // Test ID, class, and data attribute together
        let md = "First section\n\n--- {#main .highlight data-bg=\"blue\"}\n\nSecond section";
        let html = render_markdown(md).await;
        assert!(
            html.contains(r#"id="main""#),
            "Section should have id='main'. Got: {}",
            html
        );
        assert!(
            html.contains(r#"class="highlight""#),
            "Section should have class='highlight'. Got: {}",
            html
        );
        assert!(
            html.contains(r#"data-bg="blue""#),
            "Section should have data-bg='blue'. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_section_attrs_multiple_rules() {
        // Test multiple rules with attrs
        let md = "Section 0\n\n--- {#one}\n\nSection 1\n\n--- {#two}\n\nSection 2";
        let html = render_markdown(md).await;
        assert!(
            html.contains(r#"<section id="one">"#),
            "First rule section should have id='one'. Got: {}",
            html
        );
        assert!(
            html.contains(r#"<section id="two">"#),
            "Second rule section should have id='two'. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_plain_rule_still_works() {
        // Test that plain --- without attrs still creates a section
        let md = "First section\n\n---\n\nSecond section";
        let html = render_markdown(md).await;
        // Should have at least 2 sections (one before and one after the rule)
        let section_count = html.matches("<section>").count();
        assert!(
            section_count >= 1,
            "Plain rule should create sections. Got: {}",
            html
        );
        assert!(
            html.contains("<hr />"),
            "Should contain <hr /> divider. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_em_dash_with_non_attrs_text() {
        // Test that --- followed by text that isn't attrs is rendered normally
        // This becomes paragraph with em dash + text (not transformed to Rule)
        let md = "Some text\n\n--- not attrs\n\nMore text";
        let html = render_markdown(md).await;
        // Should NOT have a <hr /> since it's not a valid rule pattern
        // The em dash paragraph should be preserved as text
        assert!(
            html.contains("—"),
            "Em dash should be preserved. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_section_empty_attrs() {
        // Test that --- {} creates a section without any attributes
        let md = "First section\n\n--- {}\n\nSecond section";
        let html = render_markdown(md).await;
        // Should have a plain section (no id, class, or attrs)
        // The section should close and reopen with just <section>
        assert!(
            html.contains("<section>"),
            "Empty attrs should create plain section. Got: {}",
            html
        );
        assert!(
            html.contains("<hr />"),
            "Should contain <hr /> divider. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_section_attrs_with_whitespace() {
        // Test that whitespace inside braces is handled
        let md = "First section\n\n--- {  #intro  .highlight  }\n\nSecond section";
        let html = render_markdown(md).await;
        assert!(
            html.contains(r#"id="intro""#),
            "Whitespace should not affect ID parsing. Got: {}",
            html
        );
        assert!(
            html.contains(r#"class="highlight""#),
            "Whitespace should not affect class parsing. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_section_attrs_curly_quotes() {
        // Test curly quotes from smart punctuation (pulldown-cmark converts " to "")
        // Build the attrs string with explicit curly quotes
        let md = "First section\n\n--- {data-x=\u{201C}value\u{201D}}\n\nSecond section";
        let html = render_markdown(md).await;
        // The curly quotes should be normalized to straight quotes in output
        assert!(
            html.contains(r#"data-x="value""#),
            "Curly quotes should be normalized. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_section_attrs_html_escaping() {
        // Test that attribute values with HTML special chars are escaped
        // Note: Can't use <script> directly as pulldown-cmark interprets it as HTML
        // Use & and ' which need escaping but don't break markdown parsing
        let md = "First section\n\n--- {data-val=\"a & b\"}\n\nSecond section";
        let html = render_markdown(md).await;
        // The & should be escaped to &amp;
        assert!(
            html.contains("&amp;"),
            "HTML special chars should be escaped. Got: {}",
            html
        );
        assert!(
            html.contains(r#"data-val="a &amp; b""#),
            "Value should have escaped &. Got: {}",
            html
        );
    }

    // Wikilink and tag link tests

    fn make_sources(sources: &[&str]) -> HashSet<String> {
        sources.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn test_wikilink_transformation() {
        // [[Tags:rust]] should become a link to /tags/rust/
        let sources = make_sources(&["tags"]);
        let md = "Check out [[Tags:rust]] for more info.";
        let html = render_markdown_with_tags(md, sources).await;
        assert!(
            html.contains(r#"href="/tags/rust/""#),
            "Wikilink should transform to tag URL. Got: {}",
            html
        );
        assert!(
            html.contains(">rust<"),
            "Link text should be the tag value. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_wikilink_with_spaces() {
        // [[performers:Joshua Jay]] should become a link to /performers/joshua_jay/
        let sources = make_sources(&["performers"]);
        let md = "Watch [[performers:Joshua Jay]] perform!";
        let html = render_markdown_with_tags(md, sources).await;
        assert!(
            html.contains(r#"href="/performers/joshua_jay/""#),
            "Wikilink with spaces should normalize URL. Got: {}",
            html
        );
        assert!(
            html.contains(">Joshua Jay<"),
            "Link text should preserve original case. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_wikilink_unknown_source_becomes_native_wikilink() {
        // [[category:books]] - category is not a valid tag source, so transform_wikilinks
        // leaves it alone. But pulldown-cmark's native wikilink support picks it up
        // and renders it as a link to "category:books".
        let sources = make_sources(&["tags"]);
        let md = "See [[category:books]] for more.";
        let html = render_markdown_with_tags(md, sources).await;
        // With native wikilink support, this becomes a link (not literal text)
        assert!(
            html.contains("<a"),
            "Wikilink should become a link via pulldown-cmark. Got: {}",
            html
        );
        // The link destination should be the wikilink content
        assert!(
            html.contains("category:books"),
            "Link should reference the wikilink content. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_markdown_tag_link() {
        // [text](Tags:rust) should become a link to /tags/rust/
        let sources = make_sources(&["tags"]);
        let md = "[Learn Rust](Tags:rust)";
        let html = render_markdown_with_tags(md, sources).await;
        assert!(
            html.contains(r#"href="/tags/rust/""#),
            "Tag link should transform to tag URL. Got: {}",
            html
        );
        assert!(
            html.contains(">Learn Rust<"),
            "Link text should be preserved. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_markdown_tag_link_normalized() {
        // [Great performer](performers:joshua_jay) -> /performers/joshua_jay/
        // Note: Markdown link destinations can't contain unescaped spaces,
        // so tag values in [text](Source:value) format must be pre-normalized.
        // Use [[Source:value with spaces]] wikilink format for values with spaces.
        let sources = make_sources(&["performers"]);
        let md = "[Great performer](performers:joshua_jay)";
        let html = render_markdown_with_tags(md, sources).await;
        assert!(
            html.contains(r#"href="/performers/joshua_jay/""#),
            "Tag link should transform to tag URL. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_url_scheme_not_treated_as_tag() {
        // [Example](https://example.com) should remain a regular URL
        let sources = make_sources(&["tags", "https"]); // Even if https is a source
        let md = "[Example](https://example.com)";
        let html = render_markdown_with_tags(md, sources).await;
        assert!(
            html.contains(r#"href="https://example.com""#),
            "URL schemes should not be treated as tag sources. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_multiple_wikilinks() {
        // Multiple wikilinks in one document
        let sources = make_sources(&["tags"]);
        let md = "Learn [[Tags:rust]] and [[Tags:python]] today!";
        let html = render_markdown_with_tags(md, sources).await;
        assert!(
            html.contains(r#"href="/tags/rust/""#),
            "First wikilink should work. Got: {}",
            html
        );
        assert!(
            html.contains(r#"href="/tags/python/""#),
            "Second wikilink should work. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_nested_tag_source() {
        // [[taxonomy.tags:rust]] for nested frontmatter fields
        let sources = make_sources(&["taxonomy.tags"]);
        let md = "See [[taxonomy.tags:rust]] for more.";
        let html = render_markdown_with_tags(md, sources).await;
        assert!(
            html.contains(r#"href="/taxonomy.tags/rust/""#),
            "Nested tag source should work. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_no_tag_sources_uses_native_wikilinks() {
        // When no tag sources configured, transform_wikilinks is skipped entirely.
        // pulldown-cmark's native wikilink support still applies, rendering
        // [[Tags:rust]] as a link to "Tags:rust".
        let sources = HashSet::new();
        let md = "See [[Tags:rust]] for more.";
        let html = render_markdown_with_tags(md, sources).await;
        // With native wikilink support, this becomes a link (not literal text)
        assert!(
            html.contains("<a"),
            "Wikilink should become a link via pulldown-cmark. Got: {}",
            html
        );
        assert!(
            html.contains("Tags:rust"),
            "Link should reference the wikilink content. Got: {}",
            html
        );
    }

    // Regression tests for plain wikilinks (no colon/source prefix)
    // These verify that pulldown-cmark's native wikilink support works correctly

    #[tokio::test]
    async fn test_plain_wikilink_works() {
        // Plain [[MyPage]] should become a link to MyPage
        let html = render_markdown("Check out [[MyPage]] for more.").await;
        assert!(
            html.contains("<a"),
            "Plain wikilink should become a link. Got: {}",
            html
        );
        assert!(
            html.contains("MyPage"),
            "Link should reference MyPage. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_plain_wikilink_with_spaces() {
        // [[My Page]] should work with spaces
        let html = render_markdown("See [[My Page]] here.").await;
        assert!(
            html.contains("<a"),
            "Wikilink with spaces should become a link. Got: {}",
            html
        );
        assert!(
            html.contains("My Page"),
            "Link should preserve the page name. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_tag_and_plain_wikilinks_together() {
        // Both tag-style and plain wikilinks should work in the same document
        let sources = make_sources(&["tags"]);
        let md = "See [[Tags:rust]] and also [[MyPage]] for info.";
        let html = render_markdown_with_tags(md, sources).await;
        // Tag wikilink should go to /tags/rust/
        assert!(
            html.contains(r#"href="/tags/rust/""#),
            "Tag wikilink should transform to /tags/rust/. Got: {}",
            html
        );
        // Plain wikilink should become a link to MyPage
        assert!(
            html.contains("MyPage"),
            "Plain wikilink should reference MyPage. Got: {}",
            html
        );
        // Should have two links
        let link_count = html.matches("<a").count();
        assert!(
            link_count >= 2,
            "Should have at least 2 links. Got {} in: {}",
            link_count,
            html
        );
    }
}

use crate::oembed::PageInfo;
use crate::vid::Vid;
use pulldown_cmark::{
    Event, MetadataBlockKind, Options, Parser as MDParser, Tag, TagEnd, TextMergeStream,
};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
use yaml_rust2::{Yaml, YamlLoader};

struct EventState {
    root_path: PathBuf,
    in_vid: bool,
    in_metadata: bool,
    in_link: bool, // Track when inside a link (including autolinks like <http://...>)
    metadata_source: Option<MetadataBlockKind>,
    metadata_parsed: Option<Yaml>,
}

pub type SimpleMetadata = HashMap<String, String>;

pub async fn render(
    file: PathBuf,
    root_path: &Path,
) -> Result<(SimpleMetadata, String), Box<dyn std::error::Error>> {
    // Create parser with example Markdown text.
    let markdown_input = fs::read_to_string(file)?;
    let parser = MDParser::new_ext(&markdown_input, Options::all());
    let parser = TextMergeStream::new(parser);

    // Write to a new String buffer.
    let mut html_output = String::with_capacity(markdown_input.capacity() * 2);
    let mut state = EventState {
        root_path: root_path.to_path_buf(),
        in_vid: false,
        in_metadata: false,
        in_link: false,
        metadata_source: None,
        metadata_parsed: None,
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
                        println!("Got {key}, {value}");
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
                        println!("Got {key}, hash: {vals}");
                        hm.insert(key.to_string(), vals);
                    }
                    (Yaml::String(key), Yaml::Hash(hash)) => {
                        println!("Got {key}, recursive");
                        // TODO: recursively parse this, then modify all keys
                        // to have a leading `key.` before inserting
                        let hash = yaml_frontmatter_simplified(&Some(Yaml::Hash(hash.clone())));
                        for (k, v) in hash {
                            hm.insert(key.to_string() + "." + k.as_str(), v);
                        }
                    }
                    (Yaml::String(key), other_val) => {
                        println!("Got {key}, {:?}", &other_val);
                        if let Some(str_val) = other_val.clone().into_string() {
                            hm.insert(key.to_string(), str_val);
                        }
                    }
                    (k, v) => {
                        eprintln!("Got unknown {:?}, {:?}", k, v);
                        // no op -- silent ignore though I could print a warn? TODO
                    }
                }
            }
            hm
        }
        None => HashMap::new(),
    }
}

pub fn extract_metadata_from_file<P: AsRef<Path>>(
    path: P,
) -> Result<SimpleMetadata, Box<dyn std::error::Error>> {
    let path = path.as_ref();
    // TODO: in case of super long markdown files, I expect we can/should cap the string length
    // using some kind of buffer reader and max of ... 5000 bytes?  something like that is probably ample enough
    let markdown_input = fs::read_to_string(path)?;
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
                in_metadata = false;
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
) -> (pulldown_cmark::Event, EventState) {
    match &event {
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => match Vid::from_url_and_title(dest_url, title) {
            Some(vid) => {
                // the link title is actually the next Text event so need to split this to only produce the open tags
                state.in_vid = true;
                (Event::Html(vid.to_html(true).into()), state)
            }
            _ => (event.clone(), state),
        },
        Event::Start(Tag::MetadataBlock(v)) => {
            state.metadata_source = Some(v.clone());
            state.in_metadata = true;
            (event.clone(), state)
        }
        Event::End(TagEnd::MetadataBlock(v)) => {
            state.in_metadata = false;
            (event.clone(), state)
        }
        Event::End(TagEnd::Image) => {
            if state.in_vid {
                state.in_vid = false;
                (Event::Html(Vid::html_close().into()), state)
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
            } else if !state.in_link && text.starts_with("http") && !text.contains(" ") {
                // Only process bare URLs that are NOT inside a link element.
                // URLs in <http://...> autolinks or [text](url) links are already
                // handled by markdown and shouldn't trigger oembed fetching.
                let info = PageInfo::new_from_url(text).await.unwrap_or(PageInfo {
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
        //Event::Code(code) => {
        // println!("code: {}", &code);
        //(event, state)
        //}
        _ => {
            // println!("Event: {:?}", &event);
            (event, state)
        }
    }
}

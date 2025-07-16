use crate::oembed::PageInfo;
use futures::stream::{self, StreamExt};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use pulldown_cmark::{Event, Options, Parser as MDParser, TextMergeStream};
use regex::Regex;
use std::{fs, path::PathBuf};

pub async fn render(file: PathBuf) -> Result<String, Box<dyn std::error::Error>> {
    // Create parser with example Markdown text.
    let markdown_input = fs::read_to_string(file)?;
    let parser = MDParser::new_ext(&markdown_input, Options::all());
    let parser = TextMergeStream::new(parser);

    // Write to a new String buffer.
    let mut html_output = String::with_capacity(markdown_input.capacity() * 2);

    let events = stream::iter(parser).then(|event| async move { process_event(event).await });
    let processed_events: Vec<_> = events.collect().await;

    pulldown_cmark::html::push_html(&mut html_output, processed_events.into_iter());
    Ok(html_output)
}

async fn process_event(event: pulldown_cmark::Event<'_>) -> pulldown_cmark::Event<'_> {
    match event {
        Event::Text(ref text) => {
            println!("Text: {}", &text);
            if text.starts_with("http") && !text.contains(" ") {
                let info = PageInfo::new_from_url(text).await.unwrap_or(PageInfo {
                    url: text.clone().to_string(),
                    ..Default::default()
                });
                // Event::Text(info.text().into())
                Event::Html(info.html().into())
            } else if text.trim_start().starts_with("{{ vid(path=") {
                // TODO: extract path and start and end
                //
                if let Some(vid) = Vid::from(text) {
                    println!("vid: {:?}", &vid);
                    let mut time = "".to_string();
                    if let Some(start) = vid.start {
                        time = format!("#t={}", start);
                        if let Some(end) = vid.end {
                            time += ",";
                            time += end.as_str();
                        }
                    }
                    Event::Html(
                        format!(
                            "<video controls><source src='{}{}'></video>",
                            vid.path, time
                        )
                        .into(),
                    )
                } else {
                    event
                }
            } else {
                event
            }
        }
        Event::Code(ref code) => {
            println!("code: {}", &code);
            event
        }
        _ => {
            println!("Event: {:?}", &event);
            event
        }
    }
}

#[derive(Debug, PartialEq, Default)]
pub struct Vid {
    pub path: String,
    pub start: Option<String>,
    pub end: Option<String>,
    pub caption: Option<String>,
}

impl Vid {
    pub fn from(input: &str) -> Option<Self> {
        // 1) match the whole tag {{ vid( … ) }}
        // 2) capture everything inside the parens as "params"
        let tag_re = Regex::new(r#"(?x)^\s*\{\{\s*vid\s*\((?P<params>.*?)\)\s*\}\}\s*$"#).unwrap();

        // 3) match individual key="value" pairs
        let kv_re = Regex::new(r#"\b(?P<key>\w+)\s*=\s*["'“](?P<val>[^'"”]*)["'”]"#).unwrap();

        let caps = tag_re.captures(input)?;
        let params_str = &caps["params"];

        let mut vid: Vid = Default::default();
        let mut path: Option<String> = None;

        for kv in kv_re.captures_iter(params_str) {
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
                vid.path =
                    utf8_percent_encode(format!("/videos/{}", p).as_str(), CUSTOM_ENCODE_SET)
                        .to_string();
                Some(vid)
            }
            None => None,
        }
    }
}

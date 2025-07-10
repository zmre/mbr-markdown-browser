use crate::oembed::PageInfo;
use futures::stream::{self, StreamExt};
use pulldown_cmark::{Event, Options, Parser as MDParser};
use std::{fs, path::PathBuf};

pub async fn render(file: PathBuf) -> Result<String, Box<dyn std::error::Error>> {
    // Create parser with example Markdown text.
    let markdown_input = fs::read_to_string(file)?;
    let parser = MDParser::new_ext(&markdown_input, Options::all());

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

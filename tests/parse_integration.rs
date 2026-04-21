use mbr::{Event, HeadingLevel, Tag, TagEnd};
use std::io::Write;
use tempfile::NamedTempFile;

fn parse_str(content: &str) -> mbr::ParsedDocument {
    let mut f = NamedTempFile::new().unwrap();
    write!(f, "{content}").unwrap();
    mbr::markdown::parse(f.path()).unwrap()
}

#[test]
fn parse_returns_bold_events() {
    let doc = parse_str("**bold text**");
    let events: Vec<_> = doc.events().collect();

    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Start(Tag::Strong))),
        "expected Start(Strong) event in {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::End(TagEnd::Strong))),
        "expected End(Strong) event in {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Text(t) if t.as_ref() == "bold text")),
        "expected Text(\"bold text\") event in {events:?}"
    );
}

#[test]
fn parse_returns_italic_events() {
    let doc = parse_str("*italic text*");
    let events: Vec<_> = doc.events().collect();

    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Start(Tag::Emphasis))),
        "expected Start(Emphasis) event in {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::End(TagEnd::Emphasis))),
        "expected End(Emphasis) event in {events:?}"
    );
}

#[test]
fn parse_returns_heading_events() {
    let doc = parse_str("# My Heading");
    let events: Vec<_> = doc.events().collect();

    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::Start(Tag::Heading {
                level: HeadingLevel::H1,
                ..
            })
        )),
        "expected Start(Heading H1) event in {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::End(TagEnd::Heading(HeadingLevel::H1)))),
        "expected End(Heading H1) event in {events:?}"
    );
}

#[test]
fn parse_extracts_headings_metadata() {
    let doc = parse_str("# Title\n\n## Subtitle\n\nSome text");
    assert_eq!(doc.headings.len(), 2);
    assert_eq!(doc.headings[0].text, "Title");
    assert_eq!(doc.headings[0].level, 1);
    assert_eq!(doc.headings[1].text, "Subtitle");
    assert_eq!(doc.headings[1].level, 2);
    assert!(doc.has_h1);
}

#[test]
fn parse_extracts_frontmatter() {
    let doc = parse_str("---\ntitle: Hello World\ntags: rust, markdown\n---\n\nContent here");
    assert_eq!(
        doc.frontmatter.get("title").and_then(|v| v.as_str()),
        Some("Hello World")
    );
}

#[test]
fn parse_counts_words() {
    let doc = parse_str("one two three four five");
    assert_eq!(doc.word_count, 5);
}

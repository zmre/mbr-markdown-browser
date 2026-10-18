//! Editor math parity, Rust half: `tests/fixtures/editor-math-cases.json`
//! states, for each source, the math spans the renderer finds — display or
//! inline, and the TeX between the delimiters. This test holds pulldown-cmark
//! (with the renderer's options) to that table; the editor's math plugin
//! (`components/src/editor-plugins/math.test.ts`) is held to the same file, so
//! the two cannot disagree about what is math without one of them failing.
//!
//! The table is hand-written rather than generated: it is the specification,
//! and a pulldown-cmark upgrade that changes math semantics should fail here
//! first.

use std::path::Path;

use pulldown_cmark::{Event, Parser};
use serde::Deserialize;

use crate::markdown::markdown_options;

#[derive(Debug, Deserialize, PartialEq)]
struct MathSpan {
    display: bool,
    tex: String,
}

#[derive(Debug, Deserialize)]
struct Case {
    source: String,
    math: Vec<MathSpan>,
}

fn renderer_math(source: &str) -> Vec<MathSpan> {
    Parser::new_ext(source, markdown_options())
        .filter_map(|event| match event {
            Event::InlineMath(tex) => Some(MathSpan {
                display: false,
                tex: tex.into_string(),
            }),
            Event::DisplayMath(tex) => Some(MathSpan {
                display: true,
                tex: tex.into_string(),
            }),
            _ => None,
        })
        .collect()
}

#[test]
fn renderer_math_matches_the_shared_table() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/editor-math-cases.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("reading {}: {err}", path.display()));
    let cases: Vec<Case> = serde_json::from_str(&text).expect("editor-math-cases.json is valid");
    assert!(cases.len() > 20, "the math table lost its cases");
    for case in &cases {
        assert_eq!(
            renderer_math(&case.source),
            case.math,
            "math spans of {:?}",
            case.source
        );
    }
}

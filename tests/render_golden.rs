//! Golden-output corpus for the custom HTML renderer (`src/html.rs`).
//!
//! `src/html.rs` is a fork of pulldown-cmark's writer with MBR extensions
//! (section wrapping, mermaid blocks, destination filtering, image loading
//! hints). Substring assertions cannot catch structural regressions there —
//! notably mis-nesting, where `<section>`/`</section>` counts stay balanced
//! while the tags land inside a blockquote or list. Each fixture is therefore
//! compared byte-for-byte against its expected output.
//!
//! Scope: this exercises `html::push_html_mbr` over a raw pulldown-cmark event
//! stream. It deliberately does not run `markdown.rs`'s event transforms (link
//! rewriting, media embeds, oembed), so the fixtures stay stable and readable.
//!
//! ## Adding or updating a fixture
//!
//! Drop `tests/fixtures/render/<name>.md` in place and run:
//!
//! ```sh
//! MBR_UPDATE_GOLDEN=1 cargo test --test render_golden
//! ```
//!
//! Then **read the resulting `<name>.expected.html` diff before committing it**.
//! A golden file regenerated from a broken renderer enshrines the break.

use mbr::html::push_html_mbr;
use pulldown_cmark::{Options, Parser};
use std::path::{Path, PathBuf};

/// Environment variable that rewrites every `*.expected.html` from the current
/// renderer output instead of asserting against it.
const UPDATE_ENV: &str = "MBR_UPDATE_GOLDEN";

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/render")
}

/// Renders exactly the way the application does: every pulldown-cmark option on
/// (`markdown::markdown_options()` is `Options::all()`) plus the MBR defaults.
fn render(markdown: &str) -> String {
    let parser = Parser::new_ext(markdown, Options::all());
    let mut html = String::new();
    push_html_mbr(&mut html, parser);
    html
}

/// Every `*.md` fixture, sorted so failures are reported in a stable order.
fn fixtures() -> Vec<PathBuf> {
    let dir = fixture_dir();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no fixtures found in {}", dir.display());
    paths
}

fn expected_path(markdown_path: &Path) -> PathBuf {
    markdown_path.with_extension("expected.html")
}

#[test]
fn golden_render_matches_expected_html() {
    let updating = std::env::var_os(UPDATE_ENV).is_some();
    let mismatches: Vec<String> = fixtures()
        .iter()
        .filter_map(|markdown_path| {
            let markdown = std::fs::read_to_string(markdown_path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", markdown_path.display()));
            let actual = render(&markdown);
            let golden = expected_path(markdown_path);

            if updating {
                std::fs::write(&golden, &actual)
                    .unwrap_or_else(|e| panic!("cannot write {}: {e}", golden.display()));
                return None;
            }

            let expected = std::fs::read_to_string(&golden).unwrap_or_else(|e| {
                panic!(
                    "missing golden file {} ({e}); regenerate with {UPDATE_ENV}=1 cargo test \
                     --test render_golden",
                    golden.display()
                )
            });

            (actual != expected).then(|| {
                format!(
                    "--- {} ---\n=== expected ===\n{expected}\n=== actual ===\n{actual}",
                    markdown_path.display()
                )
            })
        })
        .collect();

    assert!(
        mismatches.is_empty(),
        "{} golden fixture(s) changed. Review every diff before regenerating with \
         `{UPDATE_ENV}=1 cargo test --test render_golden`.\n\n{}",
        mismatches.len(),
        mismatches.join("\n\n")
    );
}

/// Guards against an `.expected.html` outliving the `.md` it was generated from.
#[test]
fn every_golden_file_has_a_markdown_source() {
    let dir = fixture_dir();
    let orphans: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "html"))
        .filter(|path| !path.with_extension("").with_extension("md").is_file())
        .map(|path| path.display().to_string())
        .collect();

    assert!(
        orphans.is_empty(),
        "golden file(s) with no matching .md source: {orphans:?}"
    );
}

/// Structural invariant that byte-equality alone would silently re-bless: a
/// `<section>` boundary must never appear inside a block container.
///
/// `<section>`/`</section>` open/close *counts* stay balanced when the tags are
/// mis-nested, so this walks the tag stream and checks nesting instead.
#[test]
fn no_section_boundary_is_emitted_inside_a_block_container() {
    const CONTAINERS: [&str; 5] = ["blockquote", "ul", "ol", "li", "dd"];

    /// `chunk` is the text following a `<`. True when it opens tag `name`.
    fn opens(chunk: &str, name: &str) -> bool {
        chunk
            .strip_prefix(name)
            .is_some_and(|rest| rest.starts_with(['>', ' ', '\t', '\n', '\r', '/']))
    }

    /// `chunk` is the text following a `<`. True when it closes tag `name`.
    fn closes(chunk: &str, name: &str) -> bool {
        chunk
            .strip_prefix('/')
            .is_some_and(|rest| opens(rest, name))
    }

    for markdown_path in fixtures() {
        let markdown = std::fs::read_to_string(&markdown_path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", markdown_path.display()));
        let html = render(&markdown);

        for container in CONTAINERS {
            let mut depth = 0usize;
            for chunk in html.split('<').skip(1) {
                if closes(chunk, container) {
                    depth = depth.saturating_sub(1);
                } else if opens(chunk, container) {
                    depth += 1;
                } else if depth > 0 {
                    assert!(
                        !opens(chunk, "section") && !closes(chunk, "section"),
                        "{}: a section boundary was emitted inside <{container}>:\n{html}",
                        markdown_path.display()
                    );
                }
            }
            assert_eq!(
                depth,
                0,
                "{}: unbalanced <{container}> tags:\n{html}",
                markdown_path.display()
            );
        }
    }
}

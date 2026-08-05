//! Golden output and line-number invariants for task rendering.
//!
//! `tests/render_golden.rs` deliberately drives `html::push_html_mbr` over a
//! raw pulldown-cmark stream, so it cannot see anything `markdown.rs` does.
//! Task rendering lives entirely in `markdown.rs` — the source line numbers
//! come from the parser's byte offsets, and the annotation chips are assembled
//! across a window of inline events — so it needs a corpus of its own that runs
//! the real pipeline.
//!
//! ## Adding or updating a fixture
//!
//! Add the fixture's stem to [`TASK_FIXTURES`] and run:
//!
//! ```sh
//! MBR_UPDATE_GOLDEN=1 cargo test --test task_render_golden
//! ```
//!
//! Then **read the resulting `<name>.pipeline.html` diff before committing it**.

use mbr::link_transform::LinkTransformConfig;
use mbr::tasks::scan_source_tasks;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Environment variable that rewrites every `*.pipeline.html` from the current
/// renderer output instead of asserting against it. Shared with
/// `tests/render_golden.rs` so one regeneration run refreshes both corpora.
const UPDATE_ENV: &str = "MBR_UPDATE_GOLDEN";

/// Fixtures that exercise task rendering and therefore carry a pipeline golden.
///
/// Listed explicitly rather than discovered, so that adding a task to some
/// other fixture is a deliberate act with a reviewed golden behind it.
const TASK_FIXTURES: [&str; 2] = ["task-lists", "task-annotations"];

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/render")
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

/// Renders `markdown` through mbr's full event pipeline.
///
/// Uses `render_sync` (rather than a bespoke event walk) so the corpus pins the
/// output that a static build and a served page actually produce. Oembed is off
/// and there are no tag sources, keeping the fixtures free of network effects
/// and of the wikilink substitution pass.
fn render_pipeline(markdown: &str) -> String {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("fixture.md");
    std::fs::write(&path, markdown).expect("write fixture");

    mbr::markdown::render_sync(
        path,
        dir.path(),
        0, // oembed disabled
        LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
            url_depth: None,
            current_page_url: String::new(),
        },
        None,  // no oembed cache
        false, // server_mode
        false, // transcode_enabled
        std::collections::HashSet::new(),
        false, // mark_incomplete
        &[],
        None, // no wikilink index
    )
    .expect("render")
    .html
}

/// The 1-based source lines carried by the rendered checkboxes.
fn rendered_task_lines(html: &str) -> BTreeSet<u32> {
    const ATTR: &str = "data-mbr-task-line=\"";
    html.match_indices(ATTR)
        .map(|(at, _)| {
            let rest = &html[at + ATTR.len()..];
            let end = rest.find('"').expect("unterminated line attribute");
            rest[..end].parse().expect("line attribute is a number")
        })
        .collect()
}

/// The invariant the whole offset-based approach exists to provide: the line a
/// checkbox advertises is the line `tasks::scan_source_tasks` — which is what
/// feeds the task index, the browser panel and (later) the line patcher — says
/// that task is on.
///
/// Run over every fixture, not just the task ones, so that a fixture whose
/// tasks hide inside a code fence or a heading keeps both sides honest about
/// what is *not* a task.
#[test]
fn rendered_line_numbers_agree_with_scan_source_tasks() {
    for markdown_path in fixtures() {
        let markdown = std::fs::read_to_string(&markdown_path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", markdown_path.display()));

        let rendered = rendered_task_lines(&render_pipeline(&markdown));
        let scanned: BTreeSet<u32> = scan_source_tasks(&markdown)
            .into_iter()
            .map(|task| task.line)
            .collect();

        assert_eq!(
            rendered,
            scanned,
            "{}: rendered checkbox lines disagree with scan_source_tasks",
            markdown_path.display()
        );
    }
}

#[test]
fn task_pipeline_output_matches_golden() {
    let updating = std::env::var_os(UPDATE_ENV).is_some();
    let mismatches: Vec<String> = TASK_FIXTURES
        .iter()
        .filter_map(|stem| {
            let markdown_path = fixture_dir().join(format!("{stem}.md"));
            let markdown = std::fs::read_to_string(&markdown_path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", markdown_path.display()));
            let actual = render_pipeline(&markdown);
            let golden = fixture_dir().join(format!("{stem}.pipeline.html"));

            if updating {
                std::fs::write(&golden, &actual)
                    .unwrap_or_else(|e| panic!("cannot write {}: {e}", golden.display()));
                return None;
            }

            let expected = std::fs::read_to_string(&golden).unwrap_or_else(|e| {
                panic!(
                    "missing golden file {} ({e}); regenerate with {UPDATE_ENV}=1 cargo test \
                     --test task_render_golden",
                    golden.display()
                )
            });

            (actual != expected).then(|| {
                format!("--- {stem} ---\n=== expected ===\n{expected}\n=== actual ===\n{actual}")
            })
        })
        .collect();

    assert!(
        mismatches.is_empty(),
        "{} task golden fixture(s) changed. Review every diff before regenerating with \
         `{UPDATE_ENV}=1 cargo test --test task_render_golden`.\n\n{}",
        mismatches.len(),
        mismatches.join("\n\n")
    );
}

/// Guards against a `.pipeline.html` outliving the fixture it came from, and
/// against a task fixture that never got a golden.
#[test]
fn every_task_fixture_has_a_pipeline_golden() {
    for stem in TASK_FIXTURES {
        assert!(
            fixture_dir().join(format!("{stem}.md")).is_file(),
            "{stem} is listed in TASK_FIXTURES but has no .md source"
        );
    }

    let orphans: Vec<String> = std::fs::read_dir(fixture_dir())
        .expect("read fixture dir")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.ends_with(".pipeline.html"))
        })
        .filter(|path| {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let stem = name.trim_end_matches(".pipeline.html");
            !TASK_FIXTURES.contains(&stem)
        })
        .map(|path| path.display().to_string())
        .collect();

    assert!(
        orphans.is_empty(),
        "pipeline golden(s) not listed in TASK_FIXTURES: {orphans:?}"
    );
}

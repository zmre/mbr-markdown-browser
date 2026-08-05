//! Benchmarks for task parsing.
//!
//! `scan_source_tasks` runs over every markdown file in the repo when the task
//! index is built, so both the per-line primitive and the whole-document scan
//! are on a hot path. The `prose_only` case is the one that matters most: it
//! measures what the scan costs on documents that contain no tasks at all, which
//! is the overwhelming majority of a large repository.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use mbr::tasks::{TaskStatus, parse_task_line, scan_source_tasks, set_marker};

/// A task line carrying every annotation the grammar supports.
const FULLY_ANNOTATED: &str = "  - [x] write the quarterly report !!! #work #ops @due(2026-08-05 03:00 PM) @done(2026-08-04 12:11 PM)";

/// A plain task with nothing to strip — the common case in real notes.
const PLAIN: &str = "- [ ] buy milk";

/// Prose that the pre-filter should reject before the regex ever runs.
const PROSE: &str = "This is an ordinary sentence in a markdown document.";

/// A list line that survives the pre-filter but is not a task, so it pays for a
/// full regex miss.
const NEAR_MISS: &str = "- a bullet that mentions [brackets] but is not a task";

/// Builds a synthetic document of roughly `lines` lines mixing prose, headings,
/// tasks, fenced code and frontmatter, in about the proportions a real notes
/// file has.
fn synthetic_document(lines: usize) -> String {
    let mut doc = String::with_capacity(lines * 48);
    doc.push_str("---\ntitle: Synthetic Notes\ntags: [bench]\n---\n\n");

    for i in 0..lines {
        match i % 10 {
            0 => doc.push_str(&format!("## Section {i}\n")),
            1 | 2 => doc.push_str(&format!("Some prose on line {i} explaining the section.\n")),
            3 => doc.push_str(&format!("- [ ] open task {i} #project @due(2026-08-05)\n")),
            4 => doc.push_str(&format!("\t- [x] subtask {i} @done(2026-08-04 09:30)\n")),
            5 => doc.push_str(&format!("- [-] canceled task {i} !!\n")),
            6 => doc.push_str("```rust\nlet x = 1; // - [ ] not a task\n```\n"),
            7 => doc.push_str(&format!("- a plain bullet {i}\n")),
            8 => doc.push_str(&format!("> a quoted line {i}\n")),
            _ => doc.push('\n'),
        }
    }
    doc
}

/// A document of pure prose, to isolate the cost of scanning files with no tasks.
fn prose_document(lines: usize) -> String {
    (0..lines)
        .map(|i| format!("Line {i} of ordinary prose with no task markers at all.\n"))
        .collect()
}

fn bench_parse_task_line(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_task_line");

    group.bench_function("fully_annotated", |b| {
        b.iter(|| parse_task_line(black_box(FULLY_ANNOTATED), 42));
    });
    group.bench_function("plain", |b| {
        b.iter(|| parse_task_line(black_box(PLAIN), 42));
    });
    group.bench_function("prose_rejected", |b| {
        b.iter(|| parse_task_line(black_box(PROSE), 42));
    });
    group.bench_function("near_miss", |b| {
        b.iter(|| parse_task_line(black_box(NEAR_MISS), 42));
    });

    group.finish();
}

fn bench_set_marker(c: &mut Criterion) {
    c.bench_function("set_marker", |b| {
        b.iter(|| set_marker(black_box(FULLY_ANNOTATED), TaskStatus::Open));
    });
}

fn bench_scan_source_tasks(c: &mut Criterion) {
    let mut group = c.benchmark_group("scan_source_tasks");

    let mixed = synthetic_document(500);
    group.bench_function("mixed_500_lines", |b| {
        b.iter(|| scan_source_tasks(black_box(&mixed)));
    });

    let prose = prose_document(500);
    group.bench_function("prose_only_500_lines", |b| {
        b.iter(|| scan_source_tasks(black_box(&prose)));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_parse_task_line,
    bench_set_marker,
    bench_scan_source_tasks
);
criterion_main!(benches);

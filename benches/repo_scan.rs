//! Benchmarks for repository scanning (startup performance).
//!
//! Measures how fast the repo scanner finds and indexes markdown files.

mod fixtures;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use mbr::tag_index::{TagIndex, TaggedPage};
use rayon::prelude::*;
use std::time::Duration;

fn bench_scan_all(c: &mut Criterion) {
    let mut group = c.benchmark_group("repo_scan_all");
    group.sample_size(10);

    let sizes = [50, 500];

    for &size in &sizes {
        let dir = fixtures::create_benchmark_repo(size, size / 5);
        let root = dir.path().to_path_buf();

        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &root, |b, root_path| {
            b.iter(|| {
                let repo = mbr::repo::Repo::init(
                    root_path.clone(),
                    "static",
                    &["md".to_string()],
                    &[],
                    &[],
                    "index.md",
                    &[],
                    &[],
                );
                repo.scan_all().expect("scan failed");
                repo
            });
        });
    }

    group.finish();
}

fn bench_populate_basic_metadata(c: &mut Criterion) {
    let mut group = c.benchmark_group("populate_basic_metadata");
    group.sample_size(10);

    let dir = fixtures::create_benchmark_repo(100, 500);
    let root = dir.path().to_path_buf();

    let repo = mbr::repo::Repo::init(
        root.clone(),
        "static",
        &["md".to_string()],
        &[],
        &[],
        "index.md",
        &[],
        &[],
    );
    repo.scan_all().expect("scan failed");

    group.bench_function("500_files", |b| {
        b.iter(|| repo.populate_basic_metadata());
    });

    group.finish();
}

/// Number of pages in the tag-index workload.
const TAG_WORKLOAD_PAGES: usize = 10_000;
/// Number of long-tail tags the workload spreads its secondary tag over.
const TAG_WORKLOAD_LONG_TAIL: usize = 200;

/// Builds a realistic tag workload: every page carries one dominant tag (the
/// default `tags: [note]` shape of a large vault) plus one long-tail topic tag.
///
/// The dominant bucket therefore holds all `pages` entries, which is the shape
/// that made the old clone-the-whole-Vec insert path quadratic.
fn tag_workload(pages: usize, long_tail_tags: usize) -> Vec<(String, TaggedPage)> {
    (0..pages)
        .flat_map(|i| {
            let url = format!("/notes/note-{i:05}/");
            let title = format!("Note {i}");
            let topic = format!("topic-{}", i % long_tail_tags);
            [
                (
                    "note".to_string(),
                    TaggedPage::with_description(
                        url.clone(),
                        title.clone(),
                        "A note in a large vault",
                        "note",
                    ),
                ),
                (
                    topic.clone(),
                    TaggedPage::with_description(url, title, "A note in a large vault", topic),
                ),
            ]
        })
        .collect()
}

/// Fills a fresh index with the whole workload.
fn fill_index(index: &TagIndex, workload: &[(String, TaggedPage)]) {
    for (value, page) in workload {
        index.add_page("tags", value, page.clone());
    }
}

fn bench_tag_index(c: &mut Criterion) {
    let mut group = c.benchmark_group("tag_index");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(5));

    let workload = tag_workload(TAG_WORKLOAD_PAGES, TAG_WORKLOAD_LONG_TAIL);
    group.throughput(Throughput::Elements(workload.len() as u64));

    group.bench_function("add_page_10k_pages_serial", |b| {
        b.iter_batched(
            TagIndex::new,
            |index| {
                fill_index(&index, &workload);
                index
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("add_page_10k_pages_parallel", |b| {
        b.iter_batched(
            TagIndex::new,
            |index| {
                workload.par_iter().for_each(|(value, page)| {
                    index.add_page("tags", value, page.clone());
                });
                index
            },
            BatchSize::SmallInput,
        );
    });

    let populated = TagIndex::new();
    fill_index(&populated, &workload);

    group.throughput(Throughput::Elements(TAG_WORKLOAD_PAGES as u64));
    group.bench_function("get_pages_dominant_tag", |b| {
        b.iter(|| populated.get_pages("tags", "note"));
    });

    group.throughput(Throughput::Elements(TAG_WORKLOAD_LONG_TAIL as u64 + 1));
    group.bench_function("get_all_tags", |b| {
        b.iter(|| populated.get_all_tags("tags"));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_scan_all,
    bench_populate_basic_metadata,
    bench_tag_index
);
criterion_main!(benches);

//! Benchmarks for the markdown rendering pipeline.
//!
//! This is the critical path — every page load goes through these functions.

mod fixtures;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use mbr::link_transform::LinkTransformConfig;
use std::collections::HashSet;

fn bench_render(c: &mut Criterion) {
    let mut group = c.benchmark_group("markdown_render");

    let cases = [
        ("small", fixtures::small_markdown()),
        ("medium", fixtures::medium_markdown()),
        ("large", fixtures::large_markdown()),
    ];

    let rt = tokio::runtime::Runtime::new().unwrap();

    // Server and GUI mode default `mark_incomplete` on, so the marked variant is
    // the one most page loads actually take. Benching both isolates what
    // incomplete-marker scanning costs: pass 1 records a source line per text
    // run, and pass 3 runs an unanchored regex over every run in an eligible
    // block. The fixtures contain no markers, which is the case that has to stay
    // free.
    let markers = mbr::config::default_incomplete_markers();

    for (name, content) in &cases {
        let dir = fixtures::create_single_file_repo(content);
        let file = fixtures::test_md_path(&dir);
        let root = dir.path().to_path_buf();
        let config = LinkTransformConfig::default();
        let tag_sources = HashSet::new();

        group.throughput(Throughput::Bytes(content.len() as u64));

        for (label, mark_incomplete) in [("render", false), ("render_marked", true)] {
            group.bench_with_input(BenchmarkId::new(label, name), content, |b, _| {
                b.to_async(&rt).iter(|| {
                    let file = file.clone();
                    let root = root.clone();
                    let config = config.clone();
                    let tag_sources = tag_sources.clone();
                    let markers = markers.clone();
                    async move {
                        mbr::markdown::render(
                            file,
                            &root,
                            0, // disable oembed for benchmarks
                            config,
                            false, // server_mode
                            false, // transcode_enabled
                            tag_sources,
                            mark_incomplete,
                            &markers,
                            None, // no wikilink index in benchmarks
                        )
                        .await
                        .unwrap()
                    }
                });
            });
        }
    }

    group.finish();
}

fn bench_extract_first_h1(c: &mut Criterion) {
    let mut group = c.benchmark_group("extract_first_h1");

    let cases = [
        ("small", fixtures::small_markdown()),
        ("medium", fixtures::medium_markdown()),
        ("large", fixtures::large_markdown()),
    ];

    for (name, content) in &cases {
        group.throughput(Throughput::Bytes(content.len() as u64));
        group.bench_with_input(BenchmarkId::new("extract", name), content, |b, md| {
            b.iter(|| mbr::markdown::extract_first_h1(md));
        });
    }

    group.finish();
}

fn bench_extract_metadata(c: &mut Criterion) {
    let mut group = c.benchmark_group("extract_metadata");

    let cases = [
        ("small", fixtures::small_markdown()),
        ("medium", fixtures::medium_markdown()),
        ("large", fixtures::large_markdown()),
    ];

    for (name, content) in &cases {
        let dir = fixtures::create_single_file_repo(content);
        let file = fixtures::test_md_path(&dir);

        group.throughput(Throughput::Bytes(content.len() as u64));
        group.bench_with_input(BenchmarkId::new("extract", name), &file, |b, path| {
            b.iter(|| mbr::markdown::extract_metadata_from_file(path).unwrap());
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_render,
    bench_extract_first_h1,
    bench_extract_metadata
);
criterion_main!(benches);

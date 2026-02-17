//! Benchmarks for search functionality.
//!
//! Measures parse_query micro-benchmark and full search against a 500-file repo.

mod fixtures;

use criterion::{Criterion, criterion_group, criterion_main};
use mbr::search::{SearchEngine, SearchQuery, SearchScope, parse_query};
use std::sync::Arc;

fn bench_parse_query(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_query");

    group.bench_function("simple_terms", |b| {
        b.iter(|| parse_query("rust async programming"));
    });

    group.bench_function("faceted", |b| {
        b.iter(|| parse_query("category:rust tags:async guide"));
    });

    group.bench_function("url_containing", |b| {
        b.iter(|| parse_query("https://example.com/page search terms"));
    });

    group.finish();
}

fn bench_search_metadata(c: &mut Criterion) {
    let mut group = c.benchmark_group("search");
    group.sample_size(20);

    let dir = fixtures::create_benchmark_repo(500, 50);
    let root = dir.path().to_path_buf();

    let repo = mbr::repo::Repo::init(
        root.clone(),
        "static",
        &["md".to_string()],
        &[],
        &[],
        "index.md",
        &[],
    );
    repo.scan_all().expect("scan failed");

    let engine = SearchEngine::new(Arc::new(repo), root);

    let query = SearchQuery {
        q: "Document".to_string(),
        limit: 50,
        scope: SearchScope::Metadata,
        filetype: None,
        folder_scope: Default::default(),
        folder: None,
    };

    group.bench_function("metadata_500_files", |b| {
        b.iter(|| engine.search(&query).unwrap());
    });

    let faceted_query = SearchQuery {
        q: "tags:rust".to_string(),
        limit: 50,
        scope: SearchScope::Metadata,
        filetype: None,
        folder_scope: Default::default(),
        folder: None,
    };

    group.bench_function("faceted_500_files", |b| {
        b.iter(|| engine.search(&faceted_query).unwrap());
    });

    group.finish();
}

criterion_group!(benches, bench_parse_query, bench_search_metadata);
criterion_main!(benches);

//! Benchmarks for URL path resolution.
//!
//! Measures the per-request overhead of determining what resource to serve.

mod fixtures;

use criterion::{Criterion, criterion_group, criterion_main};
use mbr::path_resolver::{PathResolverConfig, resolve_request_path};

fn bench_resolve_request_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("path_resolver");

    // Create a repo with nested dirs and files
    let dir = fixtures::create_benchmark_repo(50, 20);
    let root = dir.path().canonicalize().unwrap();

    let md_extensions = vec!["md".to_string()];
    let tag_sources = vec!["tags".to_string()];

    let config = PathResolverConfig {
        base_dir: &root,
        canonical_base_dir: Some(&root),
        static_folder: "static",
        markdown_extensions: &md_extensions,
        index_file: "index.md",
        tag_sources: &tag_sources,
    };

    // Direct markdown file
    group.bench_function("markdown_file", |b| {
        b.iter(|| resolve_request_path(&config, "folder_0/doc_0.md"));
    });

    // Trailing slash (dir → look for index.md)
    group.bench_function("directory_trailing_slash", |b| {
        b.iter(|| resolve_request_path(&config, "folder_0/"));
    });

    // Static file
    group.bench_function("static_file", |b| {
        b.iter(|| resolve_request_path(&config, "static/file_0.txt"));
    });

    // Not found
    group.bench_function("not_found", |b| {
        b.iter(|| resolve_request_path(&config, "nonexistent/path/here"));
    });

    // Tag page
    group.bench_function("tag_page", |b| {
        b.iter(|| resolve_request_path(&config, "tags/rust/"));
    });

    // Tag source index
    group.bench_function("tag_source_index", |b| {
        b.iter(|| resolve_request_path(&config, "tags/"));
    });

    // Path traversal attempt (should resolve to NotFound or be blocked)
    group.bench_function("path_traversal", |b| {
        b.iter(|| resolve_request_path(&config, "../../../etc/passwd"));
    });

    group.finish();
}

criterion_group!(benches, bench_resolve_request_path);
criterion_main!(benches);

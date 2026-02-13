//! Benchmarks for repository scanning (startup performance).
//!
//! Measures how fast the repo scanner finds and indexes markdown files.

mod fixtures;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

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
    );
    repo.scan_all().expect("scan failed");

    group.bench_function("500_files", |b| {
        b.iter(|| repo.populate_basic_metadata());
    });

    group.finish();
}

criterion_group!(benches, bench_scan_all, bench_populate_basic_metadata);
criterion_main!(benches);

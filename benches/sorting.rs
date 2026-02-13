//! Benchmarks for file sorting.
//!
//! Measures single-field and multi-field sort performance at various sizes.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use mbr::SortField;
use serde_json::{Value, json};

fn generate_files(count: usize) -> Vec<Value> {
    (0..count)
        .map(|i| {
            let title = format!("Document {:05}", count - i); // reverse order for worst case
            let mut frontmatter = serde_json::Map::new();
            frontmatter.insert("title".to_string(), json!(title));
            frontmatter.insert("order".to_string(), json!(i % 100));
            frontmatter.insert("category".to_string(), json!(format!("cat_{}", i % 10)));

            json!({
                "name": format!("doc_{}.md", i),
                "title": title,
                "created": 1000 + (i as u64) * 100,
                "modified": 2000 + (i as u64) * 50,
                "frontmatter": frontmatter,
            })
        })
        .collect()
}

fn bench_sort_single_field(c: &mut Criterion) {
    let mut group = c.benchmark_group("sort_single_field");

    let sizes = [100, 500, 2000];
    let config = vec![SortField {
        field: "title".to_string(),
        order: "asc".to_string(),
        compare: "string".to_string(),
    }];

    for &size in &sizes {
        let files = generate_files(size);
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &files, |b, data| {
            b.iter(|| {
                let mut files = data.clone();
                mbr::sort_files(&mut files, &config);
                files
            });
        });
    }

    group.finish();
}

fn bench_sort_multi_field(c: &mut Criterion) {
    let mut group = c.benchmark_group("sort_multi_field");

    let sizes = [100, 500, 2000];
    let config = vec![
        SortField {
            field: "category".to_string(),
            order: "asc".to_string(),
            compare: "string".to_string(),
        },
        SortField {
            field: "order".to_string(),
            order: "asc".to_string(),
            compare: "numeric".to_string(),
        },
        SortField {
            field: "title".to_string(),
            order: "asc".to_string(),
            compare: "string".to_string(),
        },
    ];

    for &size in &sizes {
        let files = generate_files(size);
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &files, |b, data| {
            b.iter(|| {
                let mut files = data.clone();
                mbr::sort_files(&mut files, &config);
                files
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_sort_single_field, bench_sort_multi_field);
criterion_main!(benches);

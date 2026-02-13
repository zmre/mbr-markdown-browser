//! Benchmarks for link processing: wikilinks, link transforms, and outbound link resolution.

mod fixtures;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use mbr::link_index::{OutboundLink, resolve_outbound_links};
use mbr::link_transform::{LinkTransformConfig, transform_link};
use mbr::wikilink::transform_wikilinks;
use std::collections::HashSet;

fn bench_transform_wikilinks(c: &mut Criterion) {
    let mut group = c.benchmark_group("transform_wikilinks");

    let valid_sources: HashSet<String> = ["tags", "performers", "categories"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    // Input with 2 wikilinks
    let small_input = "Some text with [[Tags:rust]] and [[Tags:async]] in it.";

    // Input with 50 wikilinks
    let large_input = {
        let mut s = String::new();
        for i in 0..50 {
            s.push_str(&format!(
                "Paragraph {} with [[Tags:topic_{}]] and some surrounding text. ",
                i, i
            ));
        }
        s
    };

    group.bench_with_input(
        BenchmarkId::new("wikilinks", "2_links"),
        &small_input,
        |b, input| {
            b.iter(|| transform_wikilinks(input, &valid_sources));
        },
    );

    group.bench_with_input(
        BenchmarkId::new("wikilinks", "50_links"),
        &large_input,
        |b, input| {
            b.iter(|| transform_wikilinks(input, &valid_sources));
        },
    );

    group.finish();
}

fn bench_transform_link(c: &mut Criterion) {
    let mut group = c.benchmark_group("transform_link");

    let config = LinkTransformConfig::default();

    let urls = [
        ("relative_md", "guide.md"),
        ("relative_md_path", "../docs/guide.md"),
        ("absolute", "/docs/guide.md"),
        ("external", "https://example.com/page"),
        ("anchor", "#section"),
        ("index_collapse", "docs/index.md"),
        ("parent_traversal", "../../other/page.md"),
    ];

    for (name, url) in &urls {
        group.bench_with_input(BenchmarkId::new("link", name), url, |b, u| {
            b.iter(|| transform_link(u, &config));
        });
    }

    group.finish();
}

fn bench_resolve_outbound_links(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolve_outbound_links");

    let links: Vec<OutboundLink> = (0..50)
        .map(|i| OutboundLink {
            to: format!("../sibling/page_{}.md", i),
            text: format!("Page {}", i),
            anchor: if i % 3 == 0 {
                Some(format!("#section-{}", i))
            } else {
                None
            },
            internal: true,
        })
        .collect();

    group.bench_function("50_links", |b| {
        b.iter(|| resolve_outbound_links("/docs/current/", links.clone()));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_transform_wikilinks,
    bench_transform_link,
    bench_resolve_outbound_links
);
criterion_main!(benches);

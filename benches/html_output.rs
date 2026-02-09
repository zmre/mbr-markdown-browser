//! Benchmarks for HTML generation from pulldown-cmark events.
//!
//! Measures the HTML output stage in isolation from markdown parsing,
//! and quantifies the overhead of section wrapping.

mod fixtures;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use mbr::html::{HtmlConfig, push_html_mbr, push_html_with_config};
use pulldown_cmark::{Event, Options, Parser};

fn collect_events(markdown: &str) -> Vec<Event<'_>> {
    Parser::new_ext(markdown, Options::all()).collect()
}

fn bench_push_html_mbr(c: &mut Criterion) {
    let mut group = c.benchmark_group("push_html_mbr");

    let cases = [
        ("small", fixtures::small_markdown()),
        ("medium", fixtures::medium_markdown()),
        ("large", fixtures::large_markdown()),
    ];

    for (name, content) in &cases {
        let events: Vec<Event<'_>> = collect_events(content);
        let event_count = events.len();

        group.throughput(Throughput::Elements(event_count as u64));
        group.bench_with_input(
            BenchmarkId::new("mbr_defaults", name),
            &events,
            |b, evts| {
                b.iter(|| {
                    let mut html = String::with_capacity(evts.len() * 64);
                    push_html_mbr(&mut html, evts.iter().cloned());
                    html
                });
            },
        );
    }

    group.finish();
}

fn bench_push_html_no_sections(c: &mut Criterion) {
    let mut group = c.benchmark_group("push_html_no_sections");

    let cases = [
        ("small", fixtures::small_markdown()),
        ("medium", fixtures::medium_markdown()),
        ("large", fixtures::large_markdown()),
    ];

    for (name, content) in &cases {
        let events: Vec<Event<'_>> = collect_events(content);
        let event_count = events.len();

        let config = HtmlConfig {
            enable_sections: false,
            ..HtmlConfig::mbr_defaults()
        };

        group.throughput(Throughput::Elements(event_count as u64));
        group.bench_with_input(BenchmarkId::new("no_sections", name), &events, |b, evts| {
            let config = config.clone();
            b.iter(|| {
                let mut html = String::with_capacity(evts.len() * 64);
                push_html_with_config(&mut html, evts.iter().cloned(), config.clone());
                html
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_push_html_mbr, bench_push_html_no_sections);
criterion_main!(benches);

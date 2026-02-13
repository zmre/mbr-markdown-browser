//! Benchmarks for Tera template rendering.
//!
//! Measures the template stage that wraps rendered markdown HTML into full pages.

mod fixtures;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use mbr::templates::Templates;
use serde_json::json;
use std::collections::HashMap;

fn bench_render_markdown(c: &mut Criterion) {
    let mut group = c.benchmark_group("template_render_markdown");

    let dir = fixtures::create_benchmark_repo(1, 0);
    let templates = Templates::new(dir.path(), None).expect("failed to create templates");

    // Pre-render HTML content at different sizes
    let small_html = "<h1>Hello</h1><p>Small page content.</p>";
    let medium_html = {
        let mut h = String::with_capacity(8_000);
        h.push_str("<h1>Medium Document</h1>");
        for i in 0..20 {
            h.push_str(&format!(
                "<h2>Section {}</h2><p>Paragraph content for section {}. \
                 This contains enough text to be representative of a typical rendered page.</p>",
                i, i
            ));
        }
        h
    };
    let large_html = {
        let mut h = String::with_capacity(40_000);
        h.push_str("<h1>Large Document</h1>");
        for i in 0..100 {
            h.push_str(&format!(
                "<h2>Section {}</h2><p>Detailed content for section {}. \
                 The template engine must handle large HTML payloads efficiently.</p>\
                 <pre><code>fn example_{}() {{ }}\n</code></pre>",
                i, i, i
            ));
        }
        h
    };

    let cases = [
        ("small", small_html.to_string()),
        ("medium", medium_html),
        ("large", large_html),
    ];

    for (name, html) in &cases {
        let frontmatter: HashMap<String, serde_json::Value> = [
            ("title".to_string(), json!("Benchmark Page")),
            ("description".to_string(), json!("A page for benchmarking")),
            (
                "tags".to_string(),
                json!(["rust", "benchmark", "performance"]),
            ),
        ]
        .into_iter()
        .collect();

        let extra_context: HashMap<String, serde_json::Value> = [
            (
                "breadcrumbs".to_string(),
                json!([
                    {"name": "Home", "url": "/"},
                    {"name": "Docs", "url": "/docs/"},
                    {"name": "Page", "url": "/docs/page/"}
                ]),
            ),
            ("current_dir_name".to_string(), json!("page")),
            ("server_mode".to_string(), json!(true)),
            ("relative_base".to_string(), json!(".mbr/")),
            ("relative_root".to_string(), json!("")),
            ("has_h1".to_string(), json!(true)),
            ("word_count".to_string(), json!(500)),
            ("reading_time_minutes".to_string(), json!(3)),
            ("file_path".to_string(), json!("docs/page.md")),
            ("sidebar_style".to_string(), json!("")),
            ("sidebar_max_items".to_string(), json!(20)),
            ("tag_sources".to_string(), json!("[]")),
            (
                "headings".to_string(),
                json!([
                    {"level": 1, "text": "Introduction", "id": "introduction"},
                    {"level": 2, "text": "Getting Started", "id": "getting-started"},
                    {"level": 2, "text": "Configuration", "id": "configuration"},
                ]),
            ),
        ]
        .into_iter()
        .collect();

        group.bench_with_input(BenchmarkId::new("render", name), html, |b, html_content| {
            b.iter(|| {
                templates
                    .render_markdown(html_content, frontmatter.clone(), extra_context.clone())
                    .unwrap()
            });
        });
    }

    group.finish();
}

fn bench_render_section(c: &mut Criterion) {
    let dir = fixtures::create_benchmark_repo(1, 0);
    let templates = Templates::new(dir.path(), None).expect("failed to create templates");

    let context_data: HashMap<String, serde_json::Value> = [
        ("current_dir_name".to_string(), json!("docs")),
        ("current_path".to_string(), json!("/docs/")),
        ("server_mode".to_string(), json!(true)),
        ("relative_base".to_string(), json!(".mbr/")),
        ("relative_root".to_string(), json!("")),
        ("sidebar_style".to_string(), json!("")),
        ("sidebar_max_items".to_string(), json!(20)),
        (
            "breadcrumbs".to_string(),
            json!([{"name": "Home", "url": "/"}]),
        ),
        (
            "subdirs".to_string(),
            json!([
                {"name": "guide", "url_path": "/docs/guide/"},
                {"name": "reference", "url_path": "/docs/reference/"},
                {"name": "tutorials", "url_path": "/docs/tutorials/"},
            ]),
        ),
        (
            "files".to_string(),
            json!([
                {"name": "README.md", "title": "Getting Started", "url_path": "/docs/README/"},
                {"name": "FAQ.md", "title": "FAQ", "url_path": "/docs/FAQ/"},
            ]),
        ),
    ]
    .into_iter()
    .collect();

    c.bench_function("template_render_section", |b| {
        b.iter(|| templates.render_section(context_data.clone()).unwrap());
    });
}

criterion_group!(benches, bench_render_markdown, bench_render_section);
criterion_main!(benches);

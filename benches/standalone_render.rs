//! Benchmarks for rendering a note from an in-memory string
//! (`standalone::Renderer::render_source`), the path an embedding app's live
//! preview takes on every keystroke. The budget it serves is 50 ms per
//! keystroke end to end, so a warm render must stay far below that.
//!
//! - `warm/*`: one `Renderer` kept across renders (templates loaded once),
//!   which is how a preview is meant to use it.
//! - `cold/*`: the one-shot `render_note_source`, which loads the templates
//!   on every call, for comparison.
//!
//! The invoice case renders through a per-note page template (`template:`)
//! that includes mbr's head partial and a sibling partial, both read and
//! parsed on every render.

mod fixtures;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use mbr::Config;
use mbr::standalone::{Renderer, render_note_source};
use std::hint::black_box;
use std::path::Path;

/// An invoice with a bill table, rendered through `invoices/invoice.html`.
fn invoice_markdown() -> String {
    let mut md = String::from(
        "---\ntitle: Invoice 2026-001\ntemplate: invoice.html\nclient: {name: Acme Corp}\n\
         due: 2026-12-19\nitems:\n",
    );
    for i in 0..12 {
        md.push_str(&format!(
            "  - {{desc: Line item {i}, qty: {}, rate: {}.50}}\n",
            i + 1,
            100 + i * 7
        ));
    }
    md.push_str("---\n\n# Invoice 2026-001\n\n| Item | Qty | Rate | Amount |\n|---|--:|--:|--:|\n");
    for i in 0..12 {
        md.push_str(&format!(
            "| Line item {i} | {} | {}.50 | {}.00 |\n",
            i + 1,
            100 + i * 7,
            (i + 1) * (100 + i * 7)
        ));
    }
    md.push_str(
        "\nPayment is due within **30 days**. See [terms](../terms.md) and \
         [[Acme Corp]].\n\nThank you for your business.\n",
    );
    md
}

const INVOICE_TEMPLATE: &str = concat!(
    "<!doctype html><html><head>{% include \"/.mbr/_head.html\" %}</head><body>",
    "<h1>{{ title }} for {{ frontmatter.client.name }}</h1>",
    "{% include \"lines.html\" %}",
    "<div class=\"notes\">{{ markdown | safe }}</div></body></html>",
);

const LINES_TEMPLATE: &str = "<table>{% for i in frontmatter.items %}<tr><td>{{ i.desc }}</td>\
    <td>{{ i.qty * i.rate }}</td></tr>{% endfor %}</table>";

fn bench_render_source(c: &mut Criterion) {
    let dir = fixtures::create_single_file_repo("# placeholder\n");
    let root = dir.path().canonicalize().expect("canonical root");
    std::fs::create_dir_all(root.join("invoices")).expect("invoices folder");
    std::fs::write(root.join("invoices/invoice.html"), INVOICE_TEMPLATE).expect("template");
    std::fs::write(root.join("invoices/lines.html"), LINES_TEMPLATE).expect("sibling");
    let config = Config {
        root_dir: root,
        oembed_timeout_ms: 0,
        ..Default::default()
    };

    let cases = [
        (
            "typical_note",
            "notes/guide.md",
            fixtures::medium_markdown(),
        ),
        (
            "invoice_template",
            "invoices/2026-001.md",
            invoice_markdown(),
        ),
    ];

    let renderer = Renderer::new(&config).expect("renderer");
    let mut warm = c.benchmark_group("standalone_render_source_warm");
    for (name, relative, source) in &cases {
        // Fail loudly, not as a fast error path.
        renderer
            .render_source(Path::new(relative), source)
            .expect("render");
        warm.bench_with_input(BenchmarkId::from_parameter(name), source, |b, source| {
            b.iter(|| {
                renderer
                    .render_source(black_box(Path::new(relative)), black_box(source))
                    .expect("render")
            })
        });
    }
    warm.finish();

    let mut cold = c.benchmark_group("standalone_render_source_cold");
    for (name, relative, source) in &cases {
        cold.bench_with_input(BenchmarkId::from_parameter(name), source, |b, source| {
            b.iter(|| {
                render_note_source(&config, black_box(Path::new(relative)), black_box(source))
                    .expect("render")
            })
        });
    }
    cold.finish();
}

criterion_group!(benches, bench_render_source);
criterion_main!(benches);

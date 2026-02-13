//! Shared fixtures for criterion benchmarks.
//!
//! Provides reusable markdown content at various sizes and a helper to build
//! temporary repositories for filesystem-dependent benchmarks.

// Each benchmark binary includes this module but only uses a subset of functions.
#![allow(dead_code)]

use tempfile::TempDir;

/// Small markdown (~200 words): 1 heading, 1 link, no frontmatter.
/// Tests the overhead floor of the render pipeline.
pub fn small_markdown() -> String {
    r#"# Introduction

This is a short markdown document used for benchmarking the render pipeline.
It contains a single heading, a paragraph with an [internal link](/docs/guide/),
and basic prose to measure the minimum overhead of processing.

Markdown rendering involves several stages: parsing the raw text with pulldown-cmark,
extracting frontmatter metadata, transforming links for the target output mode,
and generating the final HTML. Even for small documents this pipeline must execute
quickly because it runs on every page load in server mode.

The goal of this benchmark is to establish a baseline for the smallest practical
document. Any optimization to the core pipeline should show improvement here first
since there is minimal content to mask constant-time overhead.

Performance matters especially in server mode where sub-second rendering is expected.
Each request triggers a full parse-and-render cycle, so even small improvements
compound across thousands of requests during a typical browsing session.

When browsing a large repository with tens of thousands of files, the initial scan
and subsequent navigation must feel instantaneous to the user.
"#
    .to_string()
}

/// Medium markdown (~2,000 words): YAML frontmatter, 8 headings, 3 code blocks,
/// 5 internal links, 2 external links, 1 wikilink, section attrs. Typical document.
pub fn medium_markdown() -> String {
    let mut md = String::with_capacity(16_000);
    md.push_str(
        r#"---
title: Getting Started with Rust
description: A comprehensive guide to setting up your Rust development environment
tags: [rust, programming, tutorial]
author: Test Author
date: 2024-01-15
---

# Getting Started with Rust

Rust is a systems programming language that runs blazingly fast, prevents segfaults,
and guarantees thread safety. In this guide we will walk through setting up a complete
development environment, understanding the core concepts, and building your first project.

## Installation

The recommended way to install Rust is through rustup, the official installer and
version management tool. Visit the [official installation page](/docs/installation/)
for detailed platform-specific instructions.

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
rustc --version
```

After installation, verify that both `rustc` and `cargo` are available on your path.
The Rust toolchain includes the compiler, the package manager, and the standard library
documentation available offline via `rustup doc`.

## Core Concepts

Rust introduces several unique concepts that distinguish it from other systems languages.
Understanding these fundamentals is essential before diving into larger projects.

### Ownership and Borrowing

The ownership system is Rust's most distinctive feature. Every value in Rust has a single
owner, and when that owner goes out of scope, the value is dropped. This eliminates the
need for a garbage collector while preventing memory leaks.

Borrowing allows you to reference data without taking ownership. References can be either
shared (`&T`) or exclusive (`&mut T`), but never both simultaneously. This rule prevents
data races at compile time without runtime overhead.

For a deeper exploration of these concepts, see the [borrowing guide](/docs/borrowing/)
and the [Rust Book](https://doc.rust-lang.org/book/).

### Pattern Matching

Pattern matching in Rust is exhaustive and expressive. The `match` keyword handles
complex branching logic while the compiler ensures every possible case is covered.

```rust
enum Command {
    Quit,
    Echo(String),
    Move { x: i32, y: i32 },
}

fn process(cmd: Command) {
    match cmd {
        Command::Quit => println!("Quitting"),
        Command::Echo(msg) => println!("{}", msg),
        Command::Move { x, y } => println!("Moving to ({}, {})", x, y),
    }
}
```

### Error Handling

Rust uses the `Result<T, E>` type for recoverable errors and `panic!` for unrecoverable
situations. The question mark operator (`?`) provides ergonomic error propagation.

For more details see the [error handling patterns](/docs/errors/) documentation and
the related [[Tags:error-handling]] topic.

## Project Structure

A typical Rust project follows a standard layout managed by Cargo:

```
my-project/
├── Cargo.toml
├── src/
│   ├── main.rs
│   └── lib.rs
├── tests/
│   └── integration.rs
└── benches/
    └── benchmark.rs
```

--- {#build-section .highlight}

## Building and Testing

Cargo provides built-in support for building, testing, and benchmarking your code.
The most common commands are straightforward and composable.

Running `cargo build` compiles your project in debug mode. For release builds with
optimizations enabled, use `cargo build --release`. The resulting binary is placed
in the `target/` directory under the appropriate profile subdirectory.

Testing is equally simple. Unit tests live alongside the code they test in `#[cfg(test)]`
modules, while integration tests go in the `tests/` directory. Run all tests with
`cargo test` or filter by name with `cargo test test_name`.

Benchmarks use criterion for statistical rigor. Each benchmark measures throughput or
latency across multiple iterations, providing confidence intervals and detecting
regressions automatically. See the [benchmarking guide](/docs/benchmarks/) for setup
instructions.

Documentation tests are extracted from doc comments and run as part of the test suite.
This ensures code examples in documentation stay correct as the codebase evolves.

## Async Programming

Asynchronous programming in Rust uses the `async`/`await` syntax with an executor like
Tokio. This model provides excellent performance for I/O-bound workloads without the
overhead of OS threads.

The [async ecosystem](https://rust-lang.github.io/async-book/) continues to mature
rapidly, with libraries for HTTP clients, database connections, and message queues
all supporting async interfaces.

## Ecosystem and Tooling

The Rust ecosystem provides excellent tooling beyond the compiler:

- **clippy**: A linter that catches common mistakes and suggests improvements
- **rustfmt**: An opinionated code formatter for consistent style
- **rust-analyzer**: A language server for IDE integration with completion and diagnostics
- **cargo-watch**: Automatic rebuilds on file changes for rapid iteration

The crates.io registry hosts over 100,000 packages covering everything from web
frameworks to embedded systems drivers. Popular crates include serde for serialization,
tokio for async runtime, and clap for argument parsing.

## Summary

Rust offers a unique combination of performance, safety, and ergonomics. Its ownership
system prevents entire classes of bugs at compile time while zero-cost abstractions
enable writing high-level code that compiles to efficient machine instructions.

The learning curve is steeper than many languages but the compiler's error messages
guide you toward correct code. With practice, the borrow checker becomes a helpful
ally rather than an obstacle, catching subtle bugs that would be difficult to find
in languages without these guarantees.

For next steps, explore the [advanced topics](/docs/advanced/) section or browse
the full [API reference](/docs/api/) documentation.
"#,
    );

    // Pad to approximately 2000 words with additional content
    for i in 0..8 {
        md.push_str(&format!(
            "\n## Extended Section {}\n\n\
            This additional section provides further detail on the topic at hand. \
            The rendering pipeline must handle documents of this size efficiently \
            because they represent the typical document in most markdown repositories. \
            Each section adds headings, paragraphs, and inline formatting that exercises \
            different parts of the parser and HTML generator.\n\n\
            Additional paragraphs ensure the word count reaches the target range. \
            The benchmark measures end-to-end throughput including frontmatter extraction, \
            wikilink transformation, link rewriting, and HTML generation. Consistent \
            measurements across runs help identify performance regressions early in the \
            development cycle.\n",
            i + 1
        ));
    }

    md
}

/// Large markdown (~10,000 words): 30+ headings, 10+ code blocks, tables, task lists,
/// 20+ links. Stress test for O(n) vs O(n²) behavior.
pub fn large_markdown() -> String {
    let mut md = String::with_capacity(80_000);
    md.push_str(
        r#"---
title: Comprehensive Rust Reference
description: An exhaustive reference covering all major Rust features and patterns
tags: [rust, reference, advanced, programming, systems]
author: Test Author
date: 2024-06-01
category: reference
series: rust-fundamentals
---

# Comprehensive Rust Reference

This document serves as a comprehensive reference for the Rust programming language,
covering everything from basic syntax to advanced type system features. It is designed
to stress-test the markdown rendering pipeline with realistic content at scale.

"#,
    );

    // Generate 30+ sections with varied content
    let topics = [
        ("Variables and Types", "type_system"),
        ("Functions and Closures", "functions"),
        ("Control Flow", "control"),
        ("Structs and Enums", "data_types"),
        ("Traits and Generics", "traits"),
        ("Lifetime Annotations", "lifetimes"),
        ("Error Handling Patterns", "errors"),
        ("Collections and Iterators", "collections"),
        ("Concurrency Primitives", "concurrency"),
        ("Async/Await Runtime", "async"),
        ("Unsafe Rust", "unsafe_rust"),
        ("Macros and Metaprogramming", "macros"),
        ("Module System", "modules"),
        ("Testing Strategies", "testing"),
        ("Build Configuration", "build"),
        ("FFI and Interop", "ffi"),
        ("Memory Layout", "memory"),
        ("Smart Pointers", "pointers"),
        ("Pin and Unpin", "pin"),
        ("Trait Objects vs Generics", "dispatch"),
        ("SIMD and Intrinsics", "simd"),
        ("Custom Allocators", "allocators"),
        ("Embedded Development", "embedded"),
        ("WebAssembly Target", "wasm"),
        ("Benchmarking Practices", "bench"),
        ("Profiling Techniques", "profiling"),
        ("Release Optimization", "optimization"),
        ("Cross Compilation", "cross"),
        ("Package Publishing", "publish"),
        ("Workspace Management", "workspace"),
    ];

    for (i, (title, id)) in topics.iter().enumerate() {
        md.push_str(&format!("## {} {{#{}}}\n\n", title, id));

        // Add a table for every 5th section
        if i % 5 == 0 {
            md.push_str(
                "| Feature | Status | Notes |\n\
                 |---------|--------|-------|\n\
                 | Basic support | Stable | Available since 1.0 |\n\
                 | Advanced usage | Stable | Requires careful design |\n\
                 | Nightly features | Unstable | Use with caution |\n\
                 | Platform support | Varies | Check target docs |\n\n",
            );
        }

        // Add a code block for every 3rd section
        if i % 3 == 0 {
            md.push_str(&format!(
                "```rust\n\
                 // Example: {}\n\
                 fn example_{}() {{\n\
                 \x20   let data = vec![1, 2, 3, 4, 5];\n\
                 \x20   let result: Vec<_> = data.iter()\n\
                 \x20       .filter(|&&x| x > 2)\n\
                 \x20       .map(|&x| x * 2)\n\
                 \x20       .collect();\n\
                 \x20   assert_eq!(result, vec![6, 8, 10]);\n\
                 }}\n\
                 ```\n\n",
                title, id
            ));
        }

        // Add task lists for every 7th section
        if i % 7 == 0 {
            md.push_str(
                "### Checklist\n\n\
                 - [x] Read the documentation\n\
                 - [x] Set up development environment\n\
                 - [ ] Complete all exercises\n\
                 - [ ] Write tests for edge cases\n\
                 - [ ] Review with team\n\n",
            );
        }

        // Add internal and external links
        if i % 2 == 0 {
            md.push_str(&format!(
                "For more details, see the [detailed {} guide](/docs/{}/). \
                 The official documentation at [doc.rust-lang.org](https://doc.rust-lang.org) \
                 provides additional context.\n\n",
                title.to_lowercase(),
                id
            ));
        } else {
            md.push_str(&format!(
                "Related topics include [previous section](/docs/{}/prev/) and \
                 [next section](/docs/{}/next/). Check the \
                 [Rust Reference](https://doc.rust-lang.org/reference/) for formal specifications.\n\n",
                id, id
            ));
        }

        // Add substantial prose for each section (targeting ~300 words per section)
        for j in 0..4 {
            md.push_str(&format!(
                "The {} aspect of {} is fundamental to writing correct and efficient Rust code. \
                 Understanding how the compiler enforces these rules helps developers write better \
                 abstractions without sacrificing performance. The type system ensures that invalid \
                 states are unrepresentable, catching bugs at compile time that would be runtime \
                 errors in other languages. This paragraph explores subsection {} of this topic \
                 with sufficient detail to reach the target word count for large document benchmarks.\n\n\
                 When working with {} in production code, consider the tradeoffs between simplicity \
                 and flexibility. Generic code can be more reusable but also more complex to reason \
                 about. Concrete implementations are often easier to understand and debug but may \
                 require duplication. The right choice depends on the specific requirements of your \
                 project and the experience level of your team. Measure both compile time and runtime \
                 performance to make informed decisions.\n\n",
                if j % 2 == 0 { "theoretical" } else { "practical" },
                title.to_lowercase(),
                j + 1,
                title.to_lowercase(),
            ));
        }
    }

    md
}

/// Creates a temporary benchmark repository with the specified number of markdown
/// and other (static) files. Each markdown file has YAML frontmatter with title
/// and tags. Files are distributed across subdirectories.
///
/// Returns a `TempDir` that is cleaned up when dropped.
pub fn create_benchmark_repo(md_count: usize, other_count: usize) -> TempDir {
    let dir = TempDir::new().expect("failed to create temp dir");
    let root = dir.path();

    // Create .mbr directory (required for template loading)
    std::fs::create_dir_all(root.join(".mbr")).expect("failed to create .mbr");

    // Create static folder
    std::fs::create_dir_all(root.join("static")).expect("failed to create static");

    // Distribute markdown files across subdirectories
    let folder_count = (md_count / 10).max(1);
    for f in 0..folder_count {
        let folder = root.join(format!("folder_{}", f));
        std::fs::create_dir_all(&folder).expect("failed to create folder");
    }

    for i in 0..md_count {
        let folder_idx = i % folder_count;
        let folder = root.join(format!("folder_{}", folder_idx));
        let tags = match i % 4 {
            0 => "[rust, programming]",
            1 => "[python, scripting]",
            2 => "[javascript, web]",
            _ => "[devops, infrastructure]",
        };
        let content = format!(
            "---\ntitle: Document {}\ntags: {}\n---\n\n# Document {}\n\n\
            This is benchmark document number {}. It contains enough text to be \
            realistic but not so much as to dominate the scan time. The frontmatter \
            includes a title and tags for metadata extraction testing.\n\n\
            Additional paragraph to provide body content for search indexing.\n",
            i, tags, i, i
        );
        std::fs::write(folder.join(format!("doc_{}.md", i)), content)
            .expect("failed to write markdown");
    }

    // Create static/other files
    for i in 0..other_count {
        let ext = match i % 4 {
            0 => "txt",
            1 => "json",
            2 => "css",
            _ => "js",
        };
        let path = root.join("static").join(format!("file_{}.{}", i, ext));
        std::fs::write(&path, format!("/* static file {} */", i))
            .expect("failed to write static file");
    }

    dir
}

/// Creates a temporary directory with a single markdown file containing the given content.
/// Useful for benchmarks that need a real filesystem path.
pub fn create_single_file_repo(content: &str) -> TempDir {
    let dir = TempDir::new().expect("failed to create temp dir");
    let root = dir.path();
    std::fs::create_dir_all(root.join(".mbr")).expect("failed to create .mbr");
    std::fs::write(root.join("test.md"), content).expect("failed to write test.md");
    dir
}

/// Returns the path to the test markdown file in a single-file repo.
pub fn test_md_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("test.md")
}

/// Creates a `LinkTransformConfig` suitable for benchmarking.
pub fn bench_link_transform_config() -> mbr::link_transform::LinkTransformConfig {
    mbr::link_transform::LinkTransformConfig::default()
}

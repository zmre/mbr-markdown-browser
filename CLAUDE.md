# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**mbr** (markdown browser) is a Rust application that serves as a markdown previewer, browser, and static site generator. It renders markdown files on-the-fly via a local web server, supports navigation between markdown files, browsing by tags/folders, and searching. The key principle is that any markdown repository can customize its UI via a `.mbr/` folder.

## READ SKILLS (MANDATORY)

This is a rust project and a serious engineering work.  ALWAYS USE the engineer subagent unless expressly told otherwise.  Always read the rust language skill.

## Code Quality (MANDATORY)

Before completing ANY Rust code changes, you MUST run these checks:

```bash
# Format all Rust code
cargo fmt

# Check for lint issues (warnings are errors)
# IMPORTANT: Use --all-targets to check test code too (matches CI)
cargo clippy --all-targets -- -D warnings

# Run tests
cargo test
```

**These are blocking requirements.** Do not consider Rust work complete until:
1. `cargo fmt` has been run (code is formatted)
2. `cargo clippy --all-targets -- -D warnings` passes with no errors
3. `cargo test` passes

CI will reject any PR that fails these checks. The pre-commit hook enforces this locally, but you should run these explicitly to catch issues early.

**Why `--all-targets`?** Without this flag, clippy skips `#[cfg(test)]` code. CI runs with `--all-targets`, so you must too to catch all lints locally.

## When to Update Documentation and Tests (MANDATORY)

When making code changes, you MUST also update:

### Documentation Updates

Update `docs/reference/cli.md` when:
- Adding or removing CLI flags/options
- Changing default values for CLI flags
- Changing behavior of existing CLI flags

Update `docs/reference/configuration.md` when:
- Adding new configuration options to `config.rs`
- Changing default values for configuration options
- Changing behavior of existing configuration options or environment variables
- Adding or changing feature-specific settings (oembed, link tracking, video metadata, transcoding, PDF cover extraction)

Update other docs in `docs/` when:
- Adding new features that users need to know about
- Changing how existing features work
- Adding new markdown extensions or shortcodes

### Test Updates

Add or update tests when:
- Adding new functions (add unit tests in the same file)
- Adding new CLI options (add integration tests)
- Fixing bugs (add regression tests to prevent recurrence)
- Changing behavior of existing functions (update existing tests)

**Test locations:**
- Unit tests: In the same `.rs` file under `#[cfg(test)] mod tests`
- Integration tests: `tests/server_integration.rs` for HTTP/server behavior
- Build tests: `tests/build_integration.rs` for static site generation
- Property tests: Use `proptest` for invariant verification

## Goals



- **Speed** - Sub-second rendering, instant navigation, fast site builds.
  - In this tool, **performance is extremely important** -- for launch of GUI and server, render of a markdown, build of a site, and for built sites, loading and rendering in a browser.  Everything should be near instantaneous and we should be constantly looking for safe ways to make things fast, but without using local cache files.  This tool may be used on repositories with tens of thousands of markdown files and as many assets (images, pdfs, etc.) as well and it MUST perform well even on big repositories. Anything slow must be async and background and out of the critical path. It should also be made as fast as possible.
- **No lock-in** - Works with any markdown repository without modifications to the markdown files
- **Customizable** - Override styles, templates, and components per-repository
- **Rich content** - Embed videos, audio, PDFs, diagrams, and more with native markdown image syntax that works magically with other media types
- **Zero run-time dependencies** - Everything is self-contained in a single binary; no calls to external tools or special dir structures

## Build Commands

When running, pick a port to use randomly between 5202 and 5999 and then specify that port so as not to clash with coincidental use of the tool elsewhere or feature dev in other worktrees. Swap the chosen port for the 5220 used in the examples below.

```bash
# Build and run (CLI mode - outputs HTML to stdout)
cargo run -- -p 5220 README.md

# Run with web server
cargo run -- -s -p 5220 README.md

# Run with GUI window (launches native browser via wry/tao)
cargo run -- -g -p 5220 README.md

# Generate static site (outputs to build/ folder)
cargo run -- -b /path/to/markdown/repo

# Generate static site to custom output directory
cargo run -- -b --output ./public /path/to/markdown/repo

# Development with auto-reload
cargo watch -q -c -x 'run --release -- -s -p 5220 README.md'
```

### Key CLI Options

| Option | Description | Default |
|--------|-------------|---------|
| `-s, --server` | Start web server | (none) |
| `-g, --gui` | Launch native GUI window | (none) |
| `-b, --build` | Generate static site | (none) |
| `--output <PATH>` | Output directory for static build | `build` |
| `--port <PORT>` | Server port | `5200` |
| `--host <HOST>` | Server IP address | `127.0.0.1` |
| `--theme <THEME>` | Pico CSS theme (amber, blue, cyan, etc.) | `default` |
| `--oembed-timeout-ms <MS>` | URL metadata fetch timeout (0 to disable) | `500` (server), `0` (build) |
| `--oembed-cache-size <BYTES>` | Max oembed cache size | `2097152` (2MB) |
| `--build-concurrency <N>` | Parallel file processing limit | auto (2x cores, max 32) |
| `--template-folder <PATH>` | Custom template folder | (uses `.mbr/`) |
| `--transcode` | Enable HLS video transcoding for 720p/480p variants (server/GUI only) | `false` |
| `--skip-link-checks` | Skip internal link validation during build | `false` |
| `--fail-on-broken-links` | Exit non-zero if the build finds broken internal links (for CI) | `false` |
| `--no-link-tracking` | Disable bidirectional link tracking (backlinks) | `false` |
| `--no-relationship-tracking` | Disable typed relationship tracking (frontmatter relationships) | `false` |
| `--no-tasks` | Disable the task browser (`POST /.mbr/tasks`); server/GUI only | `false` |
| `--mark-incomplete` | Highlight TK/TODO/FIXME/XXX anywhere in a line | server/GUI: on, build: off |
| `--no-mark-incomplete` | Disable incomplete-marker highlighting | (unset) |
| `--title-prefix <TEXT>` | Text to prepend to all page titles | `""` (empty) |
| `--title-suffix <TEXT>` | Text to append to all page titles | `""` (empty) |
| `-v, --verbose` | Increase log verbosity | warn level |

**Boolean flag naming convention:**
- `--skip-X` — Skips a build-time operation (e.g., `--skip-link-checks` skips validation during build)
- `--no-X` — Disables a runtime feature (e.g., `--no-link-tracking` disables backlink tracking)

See `docs/reference/cli.md` for CLI flag documentation and `docs/reference/configuration.md` for configuration file, environment variable, and feature-specific settings.

## Testing

The project has comprehensive test coverage with ~462 tests:

```bash
# Run all tests
cargo test

# Run specific test modules
cargo test --lib                    # Unit tests (~274 tests)
cargo test --test server_integration # Integration tests (~68 tests)

# Run with output
cargo test -- --nocapture
```

### Test Structure

| Location | Description | Count |
|----------|-------------|-------|
| `src/*.rs` (unit tests) | Unit tests for each module | ~354 |
| `src/main.rs` | URL path builder tests | 10 |
| `tests/build_integration.rs` | Build/static site tests | ~30 |
| `tests/server_integration.rs` | HTTP integration tests | ~68 |
| Doc tests | Code examples in documentation | 7 |

Property tests use `proptest` to verify invariants like:
- Path resolution determinism and safety
- Breadcrumb generation consistency
- URL path validity (no double slashes, proper prefixes/suffixes)

## Benchmarks

Criterion benchmarks measure performance of the critical rendering pipeline and supporting modules. Use `--no-default-features` to avoid requiring GUI/media-metadata system dependencies.

```bash
# Compile benchmarks (fast check, no execution)
cargo bench --no-default-features --no-run

# Run all Rust benchmarks
cargo bench --no-default-features --benches

# Run a single benchmark suite
cargo bench --no-default-features --bench markdown_render

# Save baseline for future comparison
cargo bench --no-default-features --benches -- --save-baseline v0.4.2

# Compare against a saved baseline
cargo bench --no-default-features --benches -- --baseline v0.4.2

# Run frontend benchmarks
cd components && bun run bench
```

### Benchmark Suites

| Suite | Module | What it measures |
|-------|--------|-----------------|
| `markdown_render` | `markdown.rs` | Full render pipeline, H1 extraction, metadata extraction |
| `html_output` | `html.rs` | HTML generation from events, section wrapping overhead |
| `path_resolver` | `path_resolver.rs` | Per-request URL path resolution |
| `search` | `search.rs` | Query parsing, metadata search on 500-file repo |
| `repo_scan` | `repo.rs` | Directory scanning, metadata population |
| `template_render` | `templates.rs` | Tera template rendering at 3 sizes |
| `sorting` | `sorting.rs` | Single/multi-field sorting at 100-2000 items |
| `link_processing` | `wikilink.rs`, `link_transform.rs`, `link_index.rs` | Wikilinks, link transforms, outbound resolution |
| `cache_operations` | `oembed_cache.rs` | LRU cache get/insert/eviction |

## Frontend Components

The `components/` directory contains Lit web components (TypeScript) compiled to standalone JS modules embedded into the Rust binary:

```bash
cd components
bun install        # NOT npm
bun run dev        # Development server
bun run build      # Production build (tsc + vite)
```

Built components are placed in `dist/` and compiled into the binary via `include_bytes!`.

The build produces **five bundles** — one main bundle plus four lazy chunks, each with its own vite config:

| Bundle | Vite config | Contents | Loaded |
|--------|-------------|----------|--------|
| `mbr-components.min.js` | `vite.config.ts` | Main bundle: all always-on elements + lazy-chunk trigger elements | Every page |
| `mbr-editor.min.js` | `vite.editor.config.ts` | Milkdown/Crepe markdown editor | Lazily, when editing opens |
| `mbr-graph.min.js` | `vite.graph.config.ts` | `<mbr-mini-graph>` + d3-force (~57 kB min / ~19 kB gz) | Lazily, when the info panel first opens |
| `mbr-genealogy.min.js` | `vite.genealogy.config.ts` | Genealogy charts: family-chart + timeline tree (~204 kB min / ~61 kB gz) | Lazily, near-viewport on person pages (prefetched there) |
| `mbr-tasks.min.js` | `vite.tasks.config.ts` | `<mbr-tasks-panel>`: the two-pane task browser (~56 kB min / ~16 kB gz) | Lazily, when the task browser first opens. **Excluded from static builds** — see `TASKS_CHUNK_ROUTE` |

Stateful modules (top-level fetches/caches like `shared.ts`) live only in the main bundle; chunk elements receive data and services via Lit properties.

## Architecture

### Rust Modules (src/)

| Module | Purpose |
|--------|---------|
| `main.rs` | Entry point, CLI mode selection, `build_url_path()` |
| `lib.rs` | Library crate exports for integration tests |
| `cli.rs` | Clap argument parsing (-s server, -g gui, -b build) |
| `config.rs` | Figment-based config from `.mbr/config.toml` + env vars (`MBR_*`) |
| `errors.rs` | Error types (`MbrError`, `ConfigError`, `BuildError`) |
| `server.rs` | Axum web server - routes, static file serving, markdown rendering |
| `build.rs` | Static site generator - parallel HTML generation, asset symlinking |
| `path_resolver.rs` | Pure path resolution logic (`ResolvedPath` enum) |
| `markdown.rs` | pulldown-cmark markdown parsing with YAML frontmatter extraction. Three passes: `collect_events_and_headings` (headings, `--- {attrs}`, task rewrite — the only place parser byte ranges exist), `process_all_events` (links, media, oembed), and `mark_incomplete_blocks` (TK/TODO highlighting). The third needs source lines the first one has and the second one destroys, so `TextLines` carries them across by **event index** — sound only because `process_event` is a strict 1:1 map, which is now stated as an `# Invariant` and pinned by a `debug_assert_eq!`. Recording a line per text run is enough because `SoftBreak`/`HardBreak` end a merge, so a merged `Event::Text` never spans a source line *outside code blocks* — which is also why `mark_incomplete_blocks` skips code (a `code_depth` guard; a fence inside a list item used to be wrapped) and image alt text (`html::raw_text` silently drops injected `Event::Html`). Anchors are `id="mbr-marker-{line}"`, at most one per source line |
| `templates.rs` | Tera template engine - renders markdown into HTML wrapper |
| `relationships.rs` | Typed frontmatter relationships (parse/registry/index) with inverse/symmetric derivation |
| `repo.rs` | Parallel directory scanner using papaya/rayon for site metadata |
| `browser.rs` | Native GUI window using wry/tao with devtools (requires `gui` feature) |
| `external_open.rs` | Hands off-site links to the OS default handler (requires `gui` feature). Two policies, and the gap between them is the design: `decide_without_frame_info` backs the nav handler and lets **all** http(s) proceed, because wry passes it a bare URL and calls it for iframe loads too — cancelling off-origin there would blank the YouTube embed at `media.rs:160`; `SiteOrigin::decide` is origin-aware and used only where a frame provably isn't involved (new-window handler, IPC). Cross-origin http(s) clicks arrive instead from `mbr-link-enhancement.ts` over IPC, revalidated by `parse_ipc_open_request` because page content can post to IPC. **Launching is GUI-only and fails closed**: `open_external` refuses unless `mark_gui_active()` ran, which only `launch_browser` does — a server must never be induced to start applications on its host. Uses NSWorkspace/ShellExecuteW/gio, never a subprocess |
| `external_open.rs` | GUI-only: which navigations leave the mbr window, and the OS hand-off (`NSWorkspace` / `ShellExecuteW` / gio — never a subprocess). Two policies, deliberately: `decide_without_frame_info` answers wry's navigation handler, which **is also called for `<iframe>` loads** (wry passes a bare URL and never checks `targetFrame.isMainFrame`), so it lets *all* http(s) through — cancelling cross-origin http(s) there would blank the YouTube embed at `media.rs:160`. `SiteOrigin::decide` is the full origin-aware policy and is only used where a frame cannot be involved: wry's new-window handler and `parse_ipc_open_request`. Clicked cross-origin links come from `components/src/mbr-link-enhancement.ts` over wry IPC and are re-validated here, since anything that can run script in the page can post to that channel. `javascript:`/`vbscript:`/`data:` are refused by both. URLs are never parsed and reserialized — `message://%3C…%3E` must reach the mail client byte for byte |
| `quicklook.rs` | QuickLook preview rendering via UniFFI for macOS integration. `preview_mode_for()` routes by extension: markdown extensions (config + the built-in list `MBR.app` registers for) take the markdown pipeline; everything else renders verbatim in a `<pre>`, syntax highlighted when `embedded_hljs::language_for_extension()` matches. Text reads are capped at 1 MiB and highlighting at 256 KiB, and invalid UTF-8 is lossy-decoded — the app claims `public.plain-text`, so arbitrary files land here |
| `vid.rs` | Video embed handling and shortcodes |
| `video_transcode.rs` | HLS-based video transcoding - playlist generation and segment transcoding (requires `media-metadata` feature) |
| `video_remux.rs` | On-the-fly stream-copy ("remux") fMP4 HLS variant for videos a browser refuses to play. Drops data/subtitle tracks without re-encoding; available without `--transcode`, server/GUI only (requires `media-metadata`) |
| `video_transcode_cache.rs` | LRU cache for HLS playlists and segments using papaya concurrent hashmap. Owns the single-flight state machine; `spawn_generation` detaches the work so a client disconnect can never leave a key stuck in-progress |
| `oembed.rs` | Auto-embed for bare URLs in markdown (YouTube, Giphy, OpenGraph) |
| `oembed_cache.rs` | LRU cache for oembed metadata using papaya concurrent hashmap |
| `html.rs` | Custom HTML output for pulldown-cmark with section support |
| `attrs.rs` | Reusable attribute parser for `{#id .class key=value}` syntax |
| `link_transform.rs` | Rewrites authored hrefs for the trailing-slash URL convention. `strip_markdown_extension` only detects *extension-bearing* markdown, so an extension-less target (`[x](../folder/file)` — Obsidian/zk style) is ambiguous: it may be a page or a real file named `file` (`LICENSE`, `Makefile`, `Dockerfile`). `LinkTransformConfig::markdown_page_probe` asks the repository, via `filesystem_markdown_page_probe` → the same `resolve_request_path` a live request uses, so the transform can never disagree with what the server serves. `None` (CLI/QuickLook/link-grep/unit tests) keeps the historical static-file behaviour. The `../` arithmetic — `"../".repeat(parent_count + 1)` on non-index pages — correctly compensates for the trailing slash and must not be "fixed" |
| `page_errors.rs` | Per-page problems for `GET /{page}/errors.json`. `validate_rendered_links` reads the `<a href>` values that actually went into the HTML; `validate_internal_links` (authored destinations) is kept only for tag pages, which have no rendered body. Reading the *emitted* href is the whole point — a checker that re-derives the target from markdown re-applies the same rules the transform used, so a transform defect is invisible to it by construction. Reports three defects, all as `BrokenInternalLink` (the wire format is pinned by `mbr-page-errors.ts`, and an unknown variant would be counted and rendered by nothing): missing target, a **markdown** page link with no trailing slash, and a `../` chain escaping the repo root. The trailing-slash rule is markdown-only on purpose, and the boundary is `link_transform`'s `markdown_page_probe` — the only thing that can *emit* the slash, and it answers yes for `ResolvedPath::MarkdownFile` alone. Demanding it of a directory listing, tag page or tag source index flags an href the renderer cannot spell any other way, and flags it wrongly: those URLs serve 200 in place (no `canonical_page_redirect`) and their templates emit site-absolute links in server mode and root-anchored `../`-chains in build mode, neither of which depends on the base's trailing slash. Only a markdown *body* carries links authored relative to the page's own location |
| `tasks.rs` | Pure task-line parsing: the `- [ ]`/`[x]`/`[-]`/`[>]` grammar, `@due`/`@done`/`#tag`/`!!` annotations, `scan_source_tasks` (skips code fences and frontmatter), `set_marker` for single-byte status rewrites, `set_status` (marker + `@done(...)` stamp, clock passed in), and `patch_task_line` — the whole body of `POST /.mbr/task` minus the I/O, so line addressing, the `expected` check and terminator preservation are testable without a filesystem. Knows nothing about the filesystem. Also owns **incomplete markers**: `MarkerRule` is the single definition of "this is a `TODO:`" shared with `markdown.rs`, so the panel and the highlight cannot disagree. Boundaries are per-alternative and conditional (`\b` only on the side whose own edge character is word-ish) — a marker configured as `TODO:` would otherwise demand a word character after the colon and never match. **Use `MarkerRule::cached` on any per-file or per-request path**: compiling costs ~80µs, several times a small page's entire render, and `mark_incomplete` is on by default in server/GUI mode. Keyed by the marker list, not a `OnceLock`, because one process renders with several configurations. `parse_marker_line` never runs on a line `parse_task_line` claimed — the `or_else` in `scan_source_tasks_with_markers` *is* the "checkbox wins" rule, and `filter_map` yielding at most one entry per line is what makes the `#mbr-marker-{line}` anchor unique by construction. A marker also carries `marker_start`/`marker_end`, the word's position within `text` in **UTF-16 code units** (a JS string index, not a byte offset — the panel slices with them directly). Sent rather than re-found in the browser because the grammar is markup-aware and the boundaries are per-alternative; the offset is computed against the *collapsed* text, since the raw-line offset stops indexing `text` once whitespace is collapsed |
| `task_index.rs` | `TaskIndex`: lazy, in-memory, papaya-backed map of `PathBuf -> Arc<FileTasks>`, holding only files that contain tasks **or incomplete markers**. Built on the **first** task query (never at startup, no on-disk cache) via one **sequential** read pass under `spawn_blocking` — sequential for the reason documented at `search.rs:362`/`:658`. Single-flight via `tokio::sync::OnceCell::get_or_try_init`, which leaves the cell unset on failure so a failed build is retried rather than poisoned. `invalidate_file` / `rebuild_if_built` are no-ops until the index has been built. The `MarkerRule` is compiled **once** in `with_markers`, the same way `ignore_globs` are, since the build reads every markdown file; an empty marker slice disables marker scanning, matching `incomplete_markers = []`. `count_with` filters on `TaskKind::Task`, so a note whose only entry is a `TODO:` line does not report a bogus `0/1` progress bar |
| `task_query.rs` | Pure filtering, grouping and counting for `POST /.mbr/tasks`. `run_query` takes an index snapshot plus `today: NaiveDate` (a parameter, so bucketing is testable without mocking the clock) and returns the whole response body. Each `TaskHit` carries **both** `url_path` (where a reader goes) and `path` (the repo-relative source file `POST /.mbr/task` patches) — the second cannot be derived from the first, since `docs/index.md` is served at `/docs/`, the static-folder overlay hides a directory level, and the extension is gone. `IncludeFilter` (`all`/`tasks`/`markers`) is single-select like `due`, not an array like `statuses` — three mutually exclusive options in an array admit two meaningless states. Both it and the calendar-mode exclusion of markers live in `Filters::admissible`, deliberately: `group_by_due` consults `admissible` *before* incrementing a bucket's totals, so a marker cannot leak into a calendar count. Every other filter needs no special case — a marker is `Open`/`Normal`/`NoDue`, so it falls out of `statuses`, `priorities` and `due` naturally. The **wire** default stays `All` (the panel always sends an explicit `include`); the *panel's* starting value is the separate `tasks_default_include` config option, which reuses this same enum so serde rejects a typo at startup |

### Key Pure Functions (Testable)

These functions have been extracted for testability:

**path_resolver.rs:**
- `resolve_request_path()` - Determines resource type from URL path

**server.rs:**
- `generate_breadcrumbs()` - Creates navigation breadcrumbs from path
- `get_current_dir_name()` - Extracts directory name from path
- `get_parent_path()` - Gets parent directory URL
- `markdown_file_to_json()` - Converts file metadata to JSON

**repo.rs:**
- `should_ignore()` - Checks if path should be ignored
- `build_markdown_url_path()` - Generates URL for markdown file
- `build_static_url_path()` - Generates URL for static file
- `is_markdown_extension()` - Checks file extension

**main.rs:**
- `build_url_path()` - Builds URL from filesystem path

### Request Flow

1. Server receives URL request
2. `path_resolver::resolve_request_path()` determines resource type
3. Returns `ResolvedPath::MarkdownFile`, `StaticFile`, `DirectoryListing`, or `NotFound`
4. For markdown: parses with pulldown-cmark, extracts YAML frontmatter
5. Renders through Tera templates from `.mbr/` or compiled-in defaults
6. Serves with embedded CSS/JS from `/.mbr/*` paths

### Configuration Hierarchy

Lowest precedence first (later layers win):

1. Compiled-in defaults (config.rs `Default` impl)
2. `.mbr/config.toml` in the markdown root
3. Environment variables (`MBR_*` prefix)
4. Command-line flags

Environment variables deliberately override `.mbr/config.toml`, because the
config file ships inside the markdown repository and the operator serving it may
not be its author.

The root directory is found by searching upward for common repository markers:
- **Directories** (in order): `.mbr/`, `.git/`, `.zk/`, `.obsidian/`
- **Files** (if no dirs found): `book.toml`, `mkdocs.yml`, `docusaurus.config.js`

The `static_folder` config option (default: `"static"`) creates a URL overlay - files in `static/images/` become available at `/images/`. It may also point *outside* the markdown root, up to two levels above it (`../static` for `project/content` + `project/static`; `../../static` for SvelteKit's `src/routes` + `static`). `config::resolve_static_overlay` is the single statement of that policy — the validator and the repo scanner share the one call so they cannot drift — and it refuses anything past the two-level budget, anything reached through `$HOME` or the filesystem root, and any directory that *contains* the root.

### Key Endpoints

- `/{path}/` - Markdown files rendered to HTML (trailing slash convention)
- `/{path}` - **301 to the canonical `/{path}/`** when the path names a markdown page (also `/{path}.md` and `/{dir}` for a dir with an index file). The trailing slash decides the browser's base for the page's *own* relative links, so serving in place at a slashless URL breaks every link on the page it just served — one click later. Query string preserved; fragment left to the client per RFC 9110 §10.2.2. Static files, directory listings and `/.mbr/*` are never redirected. See `path_resolver::canonical_page_redirect`
- `/.mbr/site.json` - Full site index with all files and frontmatter
- `/.mbr/*` - Static assets (theme.css, components)
- `POST /.mbr/search` - Metadata + content search
- `POST /.mbr/tasks` - Task query: filter, group, and count markdown tasks. Server/GUI only, gated on `tasks_enabled` (404 when off). The `include` field (`all` default / `tasks` / `markers`) selects checkbox tasks, `TODO:`-style incomplete markers, or both; markers are additionally gated on `mark_incomplete`, since the `#mbr-marker-{line}` anchor a hit deep-links to is emitted by the highlighting pass
- `POST /.mbr/task` - Single-line task toggle (`{path, line, expected, to}`). Gated on `edit_enabled` + `check_edit_access`, **not** on `tasks_enabled`, since in-document checkboxes exist whether or not the task browser does. Per-line optimistic concurrency: `expected` must still match the line on disk (409 otherwise), which is the line-sized analogue of `/.mbr/edit`'s `base_hash`. Maintains the `@done(...)` stamp per `tasks_stamp_done`

### Lit Web Components

Components in `components/src/`:
- `mbr-browse.ts` - Directory/file browser (`<mbr-browse>` element)
- `mbr-info.ts` - Info panel (`<mbr-info>`, Ctrl/Cmd+G): metadata, links, relationships, and the mini link graph at the top. Lazy-loads the `mbr-graph.min.js` chunk on first open and binds data/services (`.fetchLinks`, `.getMeta`, …) onto `<mbr-mini-graph>`; graph section is omitted when the page has no `links.json` (link tracking disabled).
- `graph/` - Shared pure graph code (relationship-graph building, viewport math, links.json BFS) used by both graph features, plus `mbr-mini-graph.ts` — the force-directed neighborhood graph element shipped in the `mbr-graph.min.js` chunk. Depth defaults come from the `graph_depth` config option (default 2, range 1–5, env `MBR_GRAPH_DEPTH`), exposed to the frontend as `window.__MBR_CONFIG__.graphDepth`.
- `mbr-genealogy.ts` - Person-page trigger element (`<mbr-genealogy>`, main bundle). Emitted by `templates/_display_enhancements.html` gated on `{% if type and type == "person" %}`. Builds the relationship graph from `site.json`; renders nothing when the person has no resolved relationships, otherwise lazy-loads the `mbr-genealogy.min.js` chunk when scrolled near the viewport.
- `genealogy/` - Genealogy chunk source: chart registry + selector (localStorage `mbr_genealogy_chart`), family-chart view (default), and the custom d3-free timeline-tree layout/view. The registry is the extension point for future chart types (sunburst, edge bundling, birth-place bubble map).
- `mbr-find-bar.ts` - GUI-only find-in-page bar (`<mbr-find-bar>`), emitted from `templates/_footer.html` under `{% if gui_mode %}` so it never ships to server or static pages (where the browser's own find works). Driven by the native Edit menu in `src/browser.rs`, which calls its `open`/`close`/`findNext`/`findPrevious` methods via `evaluate_script` — **those four names are referenced from Rust string literals, so renaming them cannot fail at compile time**; `mbr-find-bar.test.ts` asserts they exist.
- `mbr-link-enhancement.ts` - GUI-only link handling (`<mbr-link-enhancement>`, emitted from `templates/_footer.html` under `{% if gui_mode %}`, and additionally guarded by `isGuiMode()`): the hover tooltips that stand in for the missing URL bar, plus **the delegated click listener that hands cross-origin `http(s)` links to the OS** via `window.ipc.postMessage("mbr:open-external:" + url)`. That half lives in the page rather than in `external_open.rs` for one reason: it can tell a clicked link from an `<iframe>` load, and wry's navigation handler cannot. It sends `anchor.href` (resolved, not the raw attribute), skips modifier- and non-left-clicks, `target`/`download` links and already-cancelled events the way a browser would, and calls `preventDefault()` **only after** the message is away, so a missing `window.ipc` degrades to an in-window navigation instead of a dead link. Application schemes are left to the Rust side.
- `find-in-page.ts` - Pure matching logic behind the find bar (text indexing over `main#wrapper`, query compilation, match offsets, Range construction). Kept separate from the element so it is unit-testable under happy-dom, which has no `CSS.highlights`. Highlight styles live in `templates/theme.css`, not the element's `static styles`, because `CSS.highlights` is a document-scoped registry and the ranges are in the light DOM.
- `mbr-tasks.ts` - Task-browser trigger (`<mbr-tasks>`, main bundle): a clipboard button in the nav plus the lowercase `t` shortcut. Emitted by `_nav.html` under `{% if server_mode and tasks_enabled %}`, and renders nothing without `tasksEnabled` (the index is built from live files, so static builds have no `POST /.mbr/tasks`). Lazy-loads the `mbr-tasks.min.js` chunk on first open via the same overridable-importer seam as `mbr-info.ts` (`setTasksChunkImporter`), and injects the endpoint, `resolveUrl` and `getTasksDefaultInclude()` as properties — the chunk cannot import `shared.ts` for any of them.
- `tasks/` - Task-browser chunk: `mbr-tasks-panel.ts` (the two-pane overlay), `task-card.ts` (one card, restating theme.css's `--mbr-task-*` and `--mbr-incomplete-*` vocabulary inside the shadow root — custom properties cross the shadow boundary but the rules using them do not; a marker's card washes **only** the word at `marker_start`..`marker_end`, and degrades to plain text if that range is unusable), plus pure helpers — `types.ts` (the wire contract, derived from `src/task_query.rs`, **plus the `TaskToggler` service type the trigger injects**), `task-format.ts` (local-time date parsing, runtime overdue marking, progress math), `task-groups.ts` (display groups and the flat row list the keyboard walks; synthesizes the aggregate "Upcoming" heading the server does not send; `taskHref` is the **one** place a hit's deep link is built, picking `#mbr-marker-N` vs `#mbr-task-N` by kind — `_navigateTo` routes through it so click and `Enter` cannot disagree with the rendered `href`), `folder-tree.ts` (folder pane from the `folders` facet). Filtering, grouping and the x/y counts are the **server's** job — every filter change is a new debounced request. `Space` / `x` toggle the focused task and the card checkboxes are clickable, but only when the injected `editEnabled` is true; those keys otherwise stay with the filter field, which keeps focus throughout (they are claimed only once `_focusRow` is on a task, the same trade `Enter` makes). A `TaskHit` whose `kind` is `marker` is **read-only** in three places — `_renderTaskRow`'s `editable`, the `Space`/`x` branch (which returns *before* `preventDefault()`, so the key falls back to the filter field the way it does on a heading), and `_writeStatus` as defence in depth — and `task-card.ts` omits its checkbox **entirely** rather than disabling one, since `data-mbr-task-line`/`-status` are exactly what `task-toggle.ts` reads back and absent markup cannot be mistargeted; a `.mbr-task-check-spacer` keeps the text on the same rail. The ⚙ popover's fourth fieldset ("Show" → `include: all|tasks|markers`) is a `<select>` rendered **last**, and `_effectiveInclude` pins it to `tasks` in calendar mode — derived, not assigned, so `_setMode` needs no change and the user's category-mode choice survives the round trip. `_include` starts from the injected `defaultInclude` (`tasks_default_include`, default `tasks`), applied in `firstUpdated` because the panel is rebuilt on every open. **Two widen-when-empty fallbacks**, `_includeFallbackPending` and `_folderFallbackPending`, both armed only by the initial open and both captured-and-disarmed at the top of `_runQuery` (so a run the user supersedes hands its fallback to nobody, and the check runs *before* anything is committed — no flash of "No tasks match"). They chain: include first (widening what counts as an entry is less destructive than dropping the folder the user is standing in), re-arming the folder flag as it goes. Terminates because nothing re-arms the include flag and the folder flag is re-armed only on the include branch, which therefore runs once — at most three requests. Both **mutate the state they widen** rather than widening invisibly, so the Show select and the folder pane keep describing what is on screen.
- `mbr-task-doc.ts` - In-document task behaviour (`<mbr-task-doc>`, main bundle, emitted from `_display_enhancements.html`): one delegated `click`/`contextmenu` listener on `main#wrapper` (left click completes, right click cancels, both `editEnabled`-gated), and the fragment handler that scrolls a deep-linked task clear of the sticky header and flashes it on load and on `hashchange`. `taskAnchorFromHash` returns the **element id** rather than a line number, so one strict regex (`^#?(mbr-(?:task|marker)-(\d+))$`) covers both anchors the renderer emits — `mbr-task-N` on a checkbox, `mbr-marker-N` on an incomplete-marker highlight — and the scroll/flash generalises for free (`flashTarget` is `closest('li') ?? el`, and `theme.css` styles the bare `.mbr-task-flash`). The write half needs no marker guard: `checkboxFrom` demands an `HTMLInputElement.mbr-task-check`, which a marker span can never be. The fragment half runs in static builds too. **The click handler deliberately does not `preventDefault()`** — cancelling a checkbox's click restores its pre-click state *after* the listener returns, silently undoing the optimistic flip; `data-mbr-task-status`, not `checked`, is the state the next click reads. A successful write no longer reloads the page, so the handler finishes the render itself: the optimistic flip covers box + status + strikethrough and `syncDoneChip` draws the stamp from the response.
- `task-toggle.ts` - The one implementation of `POST /.mbr/task` (main bundle; injected into the panel as a property because it is stateful). Sources `expected` from `/.mbr/raw/<path>` and caches the file's lines for the page's lifetime — the rendered HTML cannot supply it, since annotations are stripped out of the display text. A successful write refreshes the cached line from the response and hands the same text back to the caller; a 409 drops the file. Also owns the **live-reload seam**: **every** task write registers itself, and `wasSelfWrite()` makes `<mbr-live-reload>` skip events for that file for a short window. A *window*, not a single consumable event, because one write is announced several times — the handler broadcasts before it responds, then the watcher echoes the atomic rename (twice, on macOS, ~7ms later) — and it is registered *before* the request, because the handler's broadcast can reach the page while the fetch is still pending. Suppression is not optional: the edit token lives only in memory, so a reload per checkbox would 401 the next click on a token-protected server. What the reload used to buy is replaced by `task-chips.ts` plus the optimistic flip.
- `task-chips.ts` - Redraws the `@done(...)` chip in the document from the source line the server wrote back, since nothing re-renders the page after a toggle. Mirrors `tasks.rs`'s `DATE_ANNOTATION`/`DATETIME` grammar to read the stamp and `html.rs::push_task_time` to render it — hand-formatted in English rather than `toLocaleDateString`, because the chip lands among sibling chips the *server* rendered (the opposite of `tasks/task-format.ts`'s choice, which renders the panel's own view in the reader's locale).
- `edit-token.ts` - The optional `Authorization: Bearer` token, held in **one in-memory variable for the life of the page** and never in `localStorage`/`sessionStorage`: mbr renders arbitrary markdown and markdown may contain raw HTML, so web storage would turn "may write while this page is open" into a durable, replayable credential. The editor is its own chunk and cannot import this module, so `openEditor()` takes `token` in and hands new ones back through `onToken`, with `<mbr-editor>` doing the plumbing. Also records `noteEditTokenRequired()` on a 401 so the editor opens with its otherwise-hidden token field visible, and forgets everything when `isEditEnabled()` is false. `edit-token.test.ts` guards the invariant twice: a full editor-open → token → toggle cycle with both storages instrumented, plus a source scan (the editor chunk is not importable under happy-dom) that also pins the list of modules allowed to touch `localStorage` at all.
- `shared.ts` - Shared state (site navigation data)

These are Lit-based custom elements using decorators (`@customElement`, `@state`, etc.) and compile to ES modules loaded by the HTML template.

Display-enhancement elements (dynamic loaders like `<mbr-mermaid>` and `<mbr-hljs>`, and the person-page `<mbr-genealogy>`) are included from `templates/_display_enhancements.html`. The old mermaid-based `<mbr-relationships>` element has been removed.

### Template System

The project uses Tera templates with a partial-based architecture. Templates are in `templates/`:

**Main Templates:**
- `index.html` - Markdown page template
- `section.html` - Directory listing template (subdirectories)
- `home.html` - Home/root directory listing template

**Partial Templates (underscore prefix, not exposed as URLs):**
- `_head.html` - Base head with meta tags and core CSS
- `_head_markdown.html` - Extended head for markdown pages (includes `_head.html`); prefetches the genealogy chunk on person pages
- `_nav.html` - Navigation header with breadcrumbs and menus
- `_footer.html` - Page footer with web components
- `_scripts.html` - Script includes
- `_display_enhancements.html` - Display-enhancement elements (mermaid/hljs loaders; `<mbr-task-doc>`; `<mbr-genealogy>` gated on `{% if type and type == "person" %}`)

**Tera Template Gotchas:**
- Chained `default()` filters don't work as expected for variable fallbacks. Use conditionals instead:
  ```jinja
  {# BAD - fails if current_dir_name doesn't exist #}
  {{ title | default(value=current_dir_name) | default(value="") }}

  {# GOOD - handles missing variables properly #}
  {% if title %}{{ title }}{% elif current_dir_name %}{{ current_dir_name }}{% endif %}
  ```
- Use `{% if varname %}` to check variable existence before using
- `{{ frontmatter_json | safe }}` outputs frontmatter as JSON (excludes rendered markdown for efficiency)
- `server_mode` is a boolean template variable: `true` in server/GUI mode, `false` in static builds. Use it to conditionally include live-only features (e.g., `{% if server_mode %}<mbr-live-reload>{% endif %}`)

### Customization Points

Users override defaults by creating files in their markdown repo's `.mbr/` folder:
- `.mbr/config.toml` - Configuration overrides
- `.mbr/index.html` - Main template
- `.mbr/*.html` - Any partial templates
- `.mbr/theme.css` - CSS theme
- `.mbr/user.css` - Additional user styles
- `.mbr/components/*.js` - Component overrides

### Oembed System

The oembed system auto-embeds bare URLs in markdown with rich previews:

**Supported URL types:**
- **YouTube** - Embedded player (no network call needed)
- **Giphy** - GIF embeds (no network call needed)
- **OpenGraph** - Fetches page metadata (title, description, image) for rich link previews

**Architecture:**
- `oembed.rs` - URL detection and embed generation
- `oembed_cache.rs` - LRU cache using papaya concurrent hashmap
- URLs are fetched in parallel during markdown rendering
- Cache is shared across requests (server mode) or files (build mode)

**Mode-specific defaults:**
| Mode | oembed_timeout_ms | Reason |
|------|-------------------|--------|
| Server/GUI | 500 | Good UX with reasonable timeout |
| Build | 0 (disabled) | Default off, but overhead is minimal when enabled |

### Theming

The project uses Pico CSS with color theme variants:

```bash
mbr -s --theme amber ~/notes     # Use amber color theme
mbr -s --theme fluid.blue ~/notes # Fluid typography with blue
```

**Available themes:** default, fluid, amber, blue, cyan, fuchsia, green, grey, indigo, jade, lime, orange, pink, pumpkin, purple, red, sand, slate, violet, yellow, zinc

Theme files are in `templates/pico-main/` and loaded dynamically by `embedded_pico.rs`.

### Markdown Extensions

**Section Attributes:**
When `enable_sections` is active, horizontal rules create `<section>` elements. Add attributes to the following section:
```markdown
--- {#intro .highlight data-transition="slide"}

Content in section with id="intro" class="highlight" data-transition="slide"
```

Attribute syntax follows pulldown-cmark heading attrs spec:
- `#id` - ID attribute (last one wins if multiple)
- `.class` - CSS class (multiple allowed)
- `key=value` or `key="quoted value"` - Custom attributes

**Implementation:** `attrs.rs` parses the attribute block. `markdown.rs` transforms the event stream (pulldown-cmark converts `--- {attrs}` to em-dash + text paragraph). `html.rs` applies attrs when opening sections.

**Vid Shortcode:**
Embed videos with the `{{ vid(...) }}` shortcode:
```markdown
{{ vid(path="videos/demo.mp4") }}
{{ vid(path="Eric Jones/Eric Jones - Metal 3.mp4", start="10", end="30", caption="Great performance") }}
```

The shortcode supports:
- `path` - Video path relative to `/videos/` folder (required)
- `start` / `end` - Playback timestamps (optional)
- `caption` - Figure caption (optional)

**Note:** Pulldown-cmark's smart punctuation converts `"` to curly quotes (`"` `"`), so the regex supports both straight and curly quotes.

**Page Styles and Types:**
Two frontmatter fields form the `<body>` class list: `style` (a string, a space-separated string, or an array — every entry is a class) and `type` (a single string, slugified via `markdown::slugify` then `collapse_dashes`, e.g. `Meeting Notes` → `meeting-notes`, `Book Review (2024)` → `book-review-2024`). `collapse_dashes` is applied to the class only, never inside `slugify`: `slugify` also generates heading anchor ids, whose doubled dashes (`Hello, World!` → `hello--world`) are frozen because they are live `#anchor` targets in existing repos. `Templates::render_markdown_with_tera` combines them in `body_class_list` (`src/templates.rs`) — type first, styles after, deduped first-seen — and inserts the result as the `style` context variable, so a repo's existing `.mbr/index.html` gains the feature untouched. The `type` context variable stays exactly as authored, because templates gate on its value (`{% if type == "person" %}`). `type` is aligned with the [OKF spec](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md) and is the preferred spelling for a note's kind; `style` remains for pure presentation. Shipped styles with CSS in `templates/theme.css`: `outline`, `kanban`, plus `slides` (`reveal-slides.css`, triggered client-side by the body class in `mbr-slides.ts`). User docs: `docs/markdown/styles.md`.

### Static Site Generation

The `-b/--build` flag generates a complete static site:

```bash
mbr -b /path/to/markdown/repo              # Output to ./build
mbr -b --output ./public /path/to/repo      # Custom output directory
mbr -b --build-concurrency 8 /path/to/repo  # Explicit concurrency limit
```

**Build process:**
1. Renders all markdown files to HTML **in parallel** (uses `futures::stream::buffer_unordered`)
2. Generates section pages for directories **in parallel**
3. Symlinks assets (images, PDFs, videos) - macOS/Linux only
4. Copies `.mbr/` folder with default files
5. Creates `.mbr/site.json` with full site metadata

**Performance optimizations:**
- **Parallel rendering**: Default concurrency is 2x CPU cores (max 32). Control with `--build-concurrency`.
- **Oembed disabled by default**: Build mode sets `oembed_timeout_ms=0`. Oembed fetching is parallelized and cached, so overhead is minimal when enabled.
- To enable oembed in builds: `mbr -b --oembed-timeout-ms 500 /path/to/repo`

**Output structure:**
```
build/
├── index.html              # Home page
├── README/index.html       # /README/ → README.md
├── docs/
│   ├── index.html          # Section page
│   └── guide/index.html    # docs/guide.md
├── images/ → ../images     # Symlinked assets
└── .mbr/
    ├── site.json           # Generated site metadata
    ├── theme.css           # Default or custom
    └── *.js/*.css          # Built-in assets
```

**Note:** Asset placement is platform dependent. macOS/Linux symlink assets into the
output; Windows copies them, because `CreateSymbolicLinkW` requires Developer Mode or
elevation. The decision lives in `build::AssetPlacement::for_current_platform()` and
`Builder::place_asset()` takes the strategy as a parameter so both paths are testable
on any host.

## Key Dependencies

**Rust:**
- **axum/tower** - Web server framework
- **pulldown-cmark** - Markdown parsing (with SIMD)
- **tera** - Template engine
- **figment** - Configuration management
- **wry/tao** - Native webview GUI (optional, `gui` feature)
- **muda** - Native menu bar (macOS, optional, `gui` feature)
- **uniffi** - FFI bindings generator for Swift/Kotlin
- **papaya** - Concurrent hash maps (used for oembed cache, repo metadata)
- **rayon** - Parallel iteration for repo scanning
- **futures** - Async streams with `buffer_unordered` for parallel builds
- **reqwest** - HTTP client for oembed fetching (with rustls-tls)
- **proptest** - Property-based testing (dev)
- **tempfile** - Temporary directories for tests (dev)

**Frontend:**
- **lit** - Web components framework
- **vite** - Build tool
- **d3-force** - Force simulation for the info-panel mini link graph (ISC; in the lazy `mbr-graph.min.js` chunk)
- **family-chart** - Genealogy family chart on person pages (ISC; depends on d3, ISC/BSD-3; in the lazy `mbr-genealogy.min.js` chunk)

## macOS App Bundle

The project includes a native macOS app bundle in `macos/`:
- `MBR.app/Contents/MacOS/mbr` - Binary (symlinked/copied during build)
- `MBR.app/Contents/Resources/AppIcon.icns` - Application icon
- `MBR.app/Contents/Info.plist` - App metadata

The app uses **muda** crate for native menubar with standard macOS keyboard shortcuts (Cmd+Q quit, Cmd+W close window). Platform-specific code is gated with `#[cfg(target_os = "macos")]`.

## Nix Packaging

The project uses Nix flakes for reproducible builds:

```bash
# Build the binary and macOS app bundle
nix build .#mbr

# Create release archives (tar.gz for all platforms, zip for macOS)
nix run .#release

# Check flake validity
nix flake check
```

The flake uses `rustPlatform.buildRustPackage` with a `postInstall` phase that copies the macOS app bundle and performs ad-hoc code signing. Release archives are created in `release/`.

**Note:** Code signing verification may fail in Nix sandbox environment due to metadata changes, but the app still runs correctly.

## Active Technologies
- Rust (stable, matches project) (001-pdf-cover-images)
- Filesystem (sidecar files: `.pdf.cover.jpg`) (001-pdf-cover-images)
- Rust 2024 Edition (1.85+) for backend; TypeScript 5.x (strict mode) for components + Lit 3.x (web components), Pico CSS (styling), serde (JSON serialization) (003-media-browser-component)
- N/A - client-side only, data from site.json (003-media-browser-component)
- TypeScript 5.x (strict mode) for Lit components + Lit 3.x, Browser localStorage API (004-video-progress)
- Browser localStorage (key-value, ~5MB limit per origin) (004-video-progress)

## Recent Changes
- 001-pdf-cover-images: Added Rust (stable, matches project)

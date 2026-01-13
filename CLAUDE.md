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
- Changing default values for any configuration
- Adding new configuration options to `config.rs`
- Changing behavior of existing options

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

In this tool, **performance is extremely important** -- for launch of GUI and server, render of a markdown, build of a site, and for built sites, loading and rendering in a browser.  Everything should be near instantaneous and we should be constantly looking for safe ways to make things fast, but without using local cache files.  This tool may be used on repositories with tens of thousands of markdown files and as many assets (images, pdfs, etc.) as well and it MUST perform well even on big repositories. Anything slow must be async and background and out of the critical path. It should also be made as fast as possible.

## Build Commands

```bash
# Build and run (CLI mode - outputs HTML to stdout)
cargo run -- README.md

# Run with web server
cargo run -- -s README.md

# Run with GUI window (launches native browser via wry/tao)
cargo run -- -g README.md

# Generate static site (outputs to build/ folder)
cargo run -- -b /path/to/markdown/repo

# Generate static site to custom output directory
cargo run -- -b --output ./public /path/to/markdown/repo

# Development with auto-reload
cargo watch -q -c -x 'run --release -- -s README.md'
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
| `-v, --verbose` | Increase log verbosity | warn level |

See `docs/reference/cli.md` for complete documentation.

## Testing

The project has comprehensive test coverage with ~387 tests:

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
| `src/*.rs` (unit tests) | Unit tests for each module | ~274 |
| `src/main.rs` | URL path builder tests | 10 |
| `tests/build_integration.rs` | Build/static site tests | ~30 |
| `tests/server_integration.rs` | HTTP integration tests | ~68 |
| Doc tests | Code examples in documentation | 5 |

Property tests use `proptest` to verify invariants like:
- Path resolution determinism and safety
- Breadcrumb generation consistency
- URL path validity (no double slashes, proper prefixes/suffixes)

## Frontend Components

The `components/` directory contains Lit web components (TypeScript) compiled to standalone JS modules embedded into the Rust binary:

```bash
cd components
bun install        # NOT npm
bun run dev        # Development server
bun run build      # Production build (tsc + vite)
```

Built components are placed in `dist/` and compiled into the binary via `include_bytes!`.

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
| `markdown.rs` | pulldown-cmark markdown parsing with YAML frontmatter extraction |
| `templates.rs` | Tera template engine - renders markdown into HTML wrapper |
| `repo.rs` | Parallel directory scanner using papaya/rayon for site metadata |
| `browser.rs` | Native GUI window using wry/tao with devtools (requires `gui` feature) |
| `quicklook.rs` | QuickLook preview rendering via UniFFI for macOS integration |
| `vid.rs` | Video embed handling with VidStack player and shortcodes |
| `oembed.rs` | Auto-embed for bare URLs in markdown (YouTube, Giphy, OpenGraph) |
| `oembed_cache.rs` | LRU cache for oembed metadata using papaya concurrent hashmap |
| `html.rs` | Custom HTML output for pulldown-cmark |

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

1. Compiled-in defaults (config.rs `Default` impl)
2. Environment variables (`MBR_*` prefix)
3. `.mbr/config.toml` in the markdown root

The root directory is found by searching upward for common repository markers:
- **Directories** (in order): `.mbr/`, `.git/`, `.zk/`, `.obsidian/`
- **Files** (if no dirs found): `book.toml`, `mkdocs.yml`, `docusaurus.config.js`

The `static_folder` config option (default: `"static"`) creates a URL overlay - files in `static/images/` become available at `/images/`.

### Key Endpoints

- `/{path}` - Markdown files rendered to HTML (trailing slash convention)
- `/.mbr/site.json` - Full site index with all files and frontmatter
- `/.mbr/*` - Static assets (theme.css, components, vidstack player)

### Lit Web Components

Components in `components/src/`:
- `mbr-browse.ts` - Directory/file browser (`<mbr-browse>` element)
- `shared.ts` - Shared state (site navigation data)

These are Lit-based custom elements using decorators (`@customElement`, `@state`, etc.) and compile to ES modules loaded by the HTML template.

### Template System

The project uses Tera templates with a partial-based architecture. Templates are in `templates/`:

**Main Templates:**
- `index.html` - Markdown page template
- `section.html` - Directory listing template (subdirectories)
- `home.html` - Home/root directory listing template

**Partial Templates (underscore prefix, not exposed as URLs):**
- `_head.html` - Base head with meta tags and core CSS
- `_head_markdown.html` - Extended head for markdown pages (includes `_head.html`)
- `_nav.html` - Navigation header with breadcrumbs and menus
- `_footer.html` - Page footer with web components
- `_scripts.html` - Base script includes
- `_scripts_markdown.html` - Extended scripts for markdown (hljs, mermaid, vidstack)

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

**Note:** Static site generation is not supported on Windows (symlinks require admin).

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

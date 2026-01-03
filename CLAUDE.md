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
cargo clippy -- -D warnings

# Run tests
cargo test
```

**These are blocking requirements.** Do not consider Rust work complete until:
1. `cargo fmt` has been run (code is formatted)
2. `cargo clippy -- -D warnings` passes with no errors
3. `cargo test` passes

CI will reject any PR that fails these checks. The pre-commit hook enforces this locally, but you should run these explicitly to catch issues early.

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

## Testing

The project has comprehensive test coverage with ~150 tests:

```bash
# Run all tests
cargo test

# Run specific test modules
cargo test --lib                    # Unit tests (~120 tests)
cargo test --test server_integration # Integration tests (18 tests)

# Run with output
cargo test -- --nocapture
```

### Test Structure

| Location | Description | Count |
|----------|-------------|-------|
| `src/*/tests` | Unit tests for each module | ~80 |
| `src/*/proptests` | Property-based tests (proptest) | ~25 |
| `src/main.rs` | URL path builder tests | 7 |
| `tests/server_integration.rs` | HTTP integration tests | 18 |
| Doc tests | Code examples in documentation | 3 |

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
| `browser.rs` | Native GUI window using wry/tao with devtools |
| `vid.rs` | Video embed handling with VidStack player and shortcodes |
| `oembed.rs` | Auto-embed for bare URLs in markdown |
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

The root directory is found by searching upward for a `.mbr/` folder.

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

### Customization Points

Users override defaults by creating files in their markdown repo's `.mbr/` folder:
- `.mbr/config.toml` - Configuration overrides
- `.mbr/index.html` - Main template
- `.mbr/*.html` - Any partial templates
- `.mbr/theme.css` - CSS theme
- `.mbr/user.css` - Additional user styles
- `.mbr/components/*.js` - Component overrides

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
```

**Build process:**
1. Renders all markdown files to HTML
2. Generates section pages for directories
3. Symlinks assets (images, PDFs, videos) - macOS/Linux only
4. Copies `.mbr/` folder with default files
5. Creates `.mbr/site.json` with full site metadata

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
- **wry/tao** - Native webview GUI
- **muda** - Native menu bar (macOS)
- **papaya** - Concurrent hash maps
- **rayon** - Parallel iteration for repo scanning
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

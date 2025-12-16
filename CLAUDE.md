# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**mbr** (markdown browser) is a Rust application that serves as a markdown previewer, browser, and (eventually) static site generator. It renders markdown files on-the-fly via a local web server, supports navigation between markdown files, browsing by tags/folders, and searching. The key principle is that any markdown repository can customize its UI via a `.mbr/` folder.

## Build Commands

```bash
# Build and run (CLI mode - outputs HTML to stdout)
cargo run -- README.md

# Run with web server
cargo run -- -s README.md

# Run with GUI window (launches native browser via wry/tao)
cargo run -- -g README.md

# Development with auto-reload
cargo watch -q -c -x 'run --release -- -s README.md'
```

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

### Rust Crates (src/)

| Module | Purpose |
|--------|---------|
| `main.rs` | Entry point, CLI mode selection |
| `cli.rs` | Clap argument parsing (-s server, -g gui) |
| `config.rs` | Figment-based config from `.mbr/config.toml` + env vars (`MBR_*`) |
| `server.rs` | Axum web server - routes, static file serving, markdown rendering |
| `markdown.rs` | pulldown-cmark markdown parsing with YAML frontmatter extraction |
| `templates.rs` | Tera template engine - renders markdown into HTML wrapper |
| `repo.rs` | Parallel directory scanner using papaya/rayon for site metadata |
| `browser.rs` | Native GUI window using wry/tao with devtools |
| `vid.rs` | Video embed handling with VidStack player |
| `oembed.rs` | Auto-embed for bare URLs in markdown |
| `html.rs` | Custom HTML output for pulldown-cmark |

### Request Flow

1. Server receives URL request
2. Looks for matching markdown file (e.g., `/foo/` -> `foo.md` or `foo/index.md`)
3. Parses markdown with pulldown-cmark, extracts YAML frontmatter
4. Renders through Tera templates from `.mbr/` or compiled-in defaults
5. Serves with embedded CSS/JS from `/.mbr/*` paths

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

### Customization Points

Users override defaults by creating files in their markdown repo's `.mbr/` folder:
- `.mbr/config.toml` - Configuration overrides
- `.mbr/index.html` - Main template
- `.mbr/theme.css` - CSS theme
- `.mbr/user.css` - Additional user styles
- `.mbr/components/*.js` - Component overrides

## Key Dependencies

**Rust:**
- **axum/tower** - Web server framework
- **pulldown-cmark** - Markdown parsing (with SIMD)
- **tera** - Template engine
- **figment** - Configuration management
- **wry/tao** - Native webview GUI
- **papaya** - Concurrent hash maps
- **rayon** - Parallel iteration for repo scanning

**Frontend:**
- **lit** - Web components framework
- **vite** - Build tool
- update memory based on current project state
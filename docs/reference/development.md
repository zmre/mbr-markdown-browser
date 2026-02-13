---
title: Development Guide
description: Building and contributing to mbr
---

# Development Guide

This guide covers building mbr from source, development workflows, and contributing.

## Quick Start

For rapid UI iteration without full Rust rebuilds, use the `--template-folder` flag to load templates and assets from disk instead of the compiled-in defaults.

### Terminal 1: Component Watcher

Watches TypeScript sources and rebuilds to `templates/components-js/` on change:

```bash
cd components
bun install     # First time only
bun run watch
```

### Terminal 2: Rust Server with Hot Reload

Watches Rust files and restarts the server, while ignoring template/component changes (those are handled by Terminal 1):

```bash
cargo watch -i "templates/**" -i "components/**" -i "*.md" -q -c -x 'run --release --bin mbr -- -s --template-folder ./templates ./docs'
```

This command:
- `-i "templates/**"` - Ignores template file changes (HTML, CSS, JS)
- `-i "components/**"` - Ignores TypeScript source changes
- `-i "*.md"` - Ignores the markdown files we might be using for testing
- `-q` - Quiet mode (less cargo-watch output)
- `-c` - Clears screen between runs
- `--template-folder ./templates` - Loads templates from disk instead of compiled defaults

### How It Works

With `--template-folder ./templates`:

1. **Templates** (`*.html`) are loaded from `./templates/` with fallback to compiled defaults
2. **Assets** (`*.css`, `*.js`) are served from `./templates/` with fallback to compiled defaults
3. **Components** (`/.mbr/components/*`) are mapped to `./templates/components-js/*`
4. **File watcher** monitors both the markdown directory and the template folder for hot reload

When you edit:
- **Rust files** → cargo watch rebuilds and restarts the server
- **HTML/CSS files in templates/** → Browser auto-reloads via WebSocket
- **TypeScript in components/src/** → Vite rebuilds to `templates/components-js/`, then browser auto-reloads

## Code Quality Requirements

All Rust code must pass formatting and linting checks before commit. CI enforces these as blocking checks.

### Formatting (cargo fmt)

All Rust code must be formatted with `rustfmt`:

```bash
# Check formatting (CI runs this)
cargo fmt -- --check

# Auto-format all files
cargo fmt
```

### Linting (cargo clippy)

All clippy warnings are treated as errors:

```bash
# Check for lint issues (CI runs this)
cargo clippy -- -D warnings

# See warnings without failing
cargo clippy
```

### Pre-commit Hook

The project includes a pre-commit hook that automatically:
1. Runs `cargo fmt` and re-stages formatted files
2. Runs `cargo clippy -- -D warnings` and blocks commit on failure
3. Syncs npm dependencies if `components/package.json` changed

**Setup (automatic in nix shell):**
```bash
git config core.hooksPath .githooks
```

The nix dev shell automatically configures this when you run `nix develop`.

**Manual setup:**
```bash
# If not using nix, manually configure the hooks
git config --local core.hooksPath .githooks
```

### CI Checks

GitHub Actions runs on every push to main and all PRs:
- `cargo test --all-features`
- `cargo clippy -- -D warnings`
- `cargo fmt -- --check`
- `bun run test` (components)
- `bun run build` (components)

All checks must pass before merge.

## Performance Benchmarks

See the [interactive benchmark dashboard](../benchmarks/) for performance trends across releases.

Benchmarks are automatically captured during the release process (`scripts/bump-version.sh`). To run benchmarks manually:

```bash
# Run benchmarks and save results for a version
./scripts/save-benchmarks.sh 0.5.0

# Save from existing criterion results without re-running
./scripts/save-benchmarks.sh 0.5.0 --no-run

# Import a saved baseline
./scripts/save-benchmarks.sh 0.4.2 --no-run --from-baseline v0.4.2
```

Skip benchmarks during a release with `SKIP_BENCHMARKS=1 ./scripts/bump-version.sh 0.5.0`.

## Build Commands

```bash
# Build release binary
cargo build --release

# Run tests
cargo test

# Build components only
cd components && bun run build

# Format and lint
cargo fmt && cargo clippy -- -D warnings
```

## Architecture Notes

The `--template-folder` flag serves dual purposes:

1. **Development**: Point to `./templates` for rapid UI iteration
2. **User customization**: Share a custom theme across multiple markdown repos

```bash
# Use a shared theme for any markdown repo
mbr -s --template-folder ~/my-mbr-theme /path/to/markdown/repo
```

### Fallback Chain

Asset resolution follows this priority:
1. `--template-folder` path (if specified)
2. `.mbr/` folder in the markdown repo
3. Compiled-in defaults

This means you can partially override - missing files fall back to defaults.

## QuickLook Extension

MBR includes a macOS QuickLook preview extension that renders markdown files using MBR's rendering engine. The extension is bundled with MBR.app and auto-registers when the app is run.

### Building the QuickLook Extension

The extension uses UniFFI to call Rust code from Swift. Build with:

```bash
# From nix development shell
nix develop -c bash -c './quicklook/build.sh'

# Build and install into local MBR.app
nix develop -c bash -c './quicklook/build.sh install'
```

**Requirements:**
- Nix development shell (provides xcodegen, ffmpeg, pkg-config)
- Xcode command line tools

### Extension Architecture

```
quicklook/
├── build.sh                          # Build script
├── project.yml                       # xcodegen project definition
├── Host/                             # Minimal host app (required for embedding)
│   ├── AppDelegate.swift
│   └── Info.plist
├── MBRPreview/                       # QuickLook extension target
│   ├── PreviewViewController.swift   # Main extension controller
│   ├── Info.plist                    # Supported UTIs, extension config
│   └── MBRPreview.entitlements       # Sandbox entitlements
└── Generated/                        # UniFFI-generated Swift bindings
    ├── mbr.swift
    └── mbrFFI.modulemap
```

### How It Works

1. **UniFFI Bindings**: The Rust `render_preview()` function (in `src/quicklook.rs`) is exposed to Swift via UniFFI
2. **Static Library**: Rust code is compiled as `libmbr.a` without GUI dependencies (`--no-default-features`)
3. **Swift Extension**: `PreviewViewController.swift` calls the Rust function and displays HTML in a WebView

### Feature Flags

The `gui` feature controls whether wry/tao/muda/rfd dependencies are included:

```bash
# Build with GUI (default) - for main MBR binary
cargo build --release

# Build without GUI - for QuickLook extension
cargo build --release --no-default-features
```

The QuickLook extension **must** be built without the `gui` feature because:
- QuickLook extensions run in a sandboxed environment without GUI access
- wry/tao require SDL3 which isn't available in the sandbox

### Testing the Extension

```bash
# After running build.sh install and launching MBR.app once:
qlmanage -p /path/to/file.md

# Check if extension is registered
pluginkit -m -i com.zmre.mbr.MBRPreview
```

### Troubleshooting

**Extension not appearing:**
1. Run MBR.app once to register the extension
2. Check `pluginkit -m` output for registration

**Extension crashes:**
1. Check crash logs in `~/Library/Logs/DiagnosticReports/`
2. Ensure extension was built with `--no-default-features`

**Conflicting extensions:**
```bash
# List all markdown QuickLook extensions
pluginkit -m | grep -i markdown
```

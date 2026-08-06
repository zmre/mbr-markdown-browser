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

GitHub Actions runs on every push to main and all PRs. Everything except the
Windows and component legs goes through Nix, so the same derivation is shared
between CI, Release, and your laptop:

- `nix build .#fmt` — rustfmt
- `nix build .#clippy` — clippy with `-D warnings` (Linux + macOS)
- `nix build .#tests` — the shipped feature set (`cliFeatures`), **not**
  `--all-features`: `ffi` is macOS-only by design, so `--all-features` fails to
  compile on Linux
- `nix build .#clippy-minimal` / `.#tests-minimal` — the feature set Windows
  ships (`--no-default-features --features gui`), run on Linux
- `nix build .#mbr` — full build on x86_64-linux, aarch64-linux, and macOS
- `nix build .#swiftfmt` / `.#swiftlint-check` — QuickLook extension (macOS)
- `cargo clippy` / `cargo test` / `cargo build` on Windows (no Nix there)
- `bun run test` and `bun run build` (components)

All checks must pass before merge.

### Binary Cache

CI pushes to [zmre.cachix.org](https://zmre.cachix.org) so that contributors and
end users substitute prebuilt outputs instead of compiling ffmpeg, pdfium, and
the full crate graph. Two things keep that working, and both are easy to break:

**`pushFilter` on every `cachix-action` block.** The action's post-job daemon
uploads everything the job put in the store, so the filter belongs on every
block — including jobs that build nothing new. A new job with a `cachix-action`
step and no `pushFilter` silently starts uploading source and vendor trees.

**Pins on the consumer artifacts.** Cachix's free tier garbage-collects
least-recently-used, which is exactly backwards here: the ~950 MB crane
dependency layer is touched by every CI run and stays warm, while
`packages.default` is fetched only when someone runs `nix run` and is therefore
always the coldest thing in the cache. The `build` job pins these after a green
build on `main` (see the comment on that step for the full reasoning):

| Pin name | Attr | Why |
|---|---|---|
| `mbr-<system>` | `.#default` | What `nix run` / `nix build` / `nix profile install` resolve to |
| `ffmpeg-minimal-static-<system>` | `.#ffmpegMinimalStatic` | Fixed-version, 30–60 min to rebuild |
| `x264-static-<system>` | `.#x264Static` | Same; pinned separately because Cachix does not document pins as covering the closure |

The crate dependency layer is deliberately **not** pinned — it churns on every
`Cargo.lock` bump and CI keeps it warm by itself.

Verify coverage from outside without building anything. Store paths are
content-addressed, so evaluating a revision anywhere reproduces what CI built:

```bash
REV=github:zmre/mbr-markdown-browser/<commit-sha>
for s in x86_64-linux aarch64-linux aarch64-darwin; do
  p=$(nix eval --raw "$REV#packages.$s.default.outPath")
  code=$(curl -s -o /dev/null -w '%{http_code}' \
    "https://zmre.cachix.org/$(basename "$p" | cut -d- -f1).narinfo")
  echo "$code  $s  $(basename "$p")"
done
```

`200` means a consumer gets a download; `404` means they compile. A `404` for a
path CI demonstrably pushed is *eviction*, not a broken push — check the cache's
usage page before touching the workflow.

Pushes and pins are skipped when `CACHIX_AUTH_TOKEN` is absent, so PRs from
forks are unaffected rather than failing.

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

## Release Packaging and Nix-Store Independence

```bash
# Build the distributable archives (macOS: .app + CLI tarballs)
nix build .#release

# Wrap a built app in the drag-to-/Applications disk image
scripts/make-macos-dmg.sh /path/to/MBR.app dist/mbr-macos-arm64.dmg
```

Release artifacts have to run on a Mac that has never had Nix installed, so
**no Mach-O in them may reference `/nix/store`** — not in `LC_LOAD_DYLIB`, not
in `LC_LOAD_WEAK_DYLIB`, not in `LC_RPATH`.

Most of this is structural: ffmpeg and x264 are statically linked, and pdfium is
`dlopen`'d at runtime from `Contents/Frameworks/` (app bundle) or `lib/` next to
the executable (CLI tarball), never from a compiled-in store path. The one
genuine dynamic leak is libiconv, which Nix's linker resolves to its own copy
instead of the `/usr/lib/libiconv.2.dylib` that macOS ships.

The `release` derivation in `flake.nix` therefore does two things, in this order
and **before** any `codesign` step (`install_name_tool` invalidates signatures):

1. `portablize` walks every Mach-O in the staged tree and rewrites store
   libiconv references to `/usr/lib/libiconv.2.dylib`.
2. `auditPortable` re-scans and **fails the build** if any store reference
   survives.

The audit exists because both halves of step 1 fail silently on their own:
`install_name_tool -change` is a no-op when the old path does not match, and an
earlier version applied it to a hardcoded list of two files, which would have
missed a third Mach-O such as `PlugIns/MBRPreview.appex`. A store dependency
other than libiconv is deliberately *not* auto-mapped to `/usr/lib/<name>` — a
wrong guess would trade a loud failure for a subtle one — so it fails the audit
and must be handled consciously.

To verify a downloaded artifact by hand:

```bash
# Should print nothing
otool -L MBR.app/Contents/MacOS/mbr | grep /nix/store

# Strongest check: run it and watch what dyld actually loads
DYLD_PRINT_LIBRARIES=1 MBR.app/Contents/MacOS/mbr --version 2>&1 | grep /nix/store
```

Note that `strings` on these binaries *does* show `/nix/store/eeee…eeee/...`
paths. Those are harmless: Rust's `--remap-path-prefix` scrubs the real hashes
to `e`s, and they appear only in panic-location and `tracing` metadata strings,
never in a load command.

### Code signing: never strip an ad-hoc signature

**On Apple Silicon an unsigned Mach-O is SIGKILLed by the kernel.** It is not a
Gatekeeper prompt or a warning — the process dies with exit 137 and prints
nothing. The ad-hoc signature that the linker (and `install_name_tool`) applies
automatically is what makes a binary runnable at all.

`codesign` cannot reach its daemon inside the Nix sandbox, so every signing call
in the `release` derivation is expected to fail and is written `|| true`. What it
must **never** do is fall back to `codesign --remove-signature`: that fallback
ran on every build and stripped the ad-hoc signature, so the app tarball shipped
a bundle that could not launch (`codesign --verify` reported "code object is not
signed at all"). Leaving the failed-to-re-sign ad-hoc signature in place is
strictly better. The derivation now fails the build if any shipped Mach-O ends up
unsigned.

Proper signing happens later, in `scripts/make-macos-dmg.sh`, which runs on the
CI runner outside the sandbox where `codesign` works, and gates itself with
`codesign --verify --deep --strict`. This is why the DMG was correctly signed
while the raw tarball was not.

```bash
# Both should print "Signature=adhoc" (or better) and then exit 0
codesign -dv MBR.app/Contents/MacOS/mbr
MBR.app/Contents/MacOS/mbr --version; echo "rc=$?"
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

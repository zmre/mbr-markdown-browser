# QuickLook Extension Development Guide

This document covers building, testing, debugging, and troubleshooting the MBR QuickLook extension.

## Architecture Overview

The QuickLook extension consists of:

1. **Rust library** (`src/quicklook.rs`) - Core rendering logic exposed via UniFFI
2. **Swift extension** (`MBRPreview/`) - macOS QuickLook extension that calls Rust
3. **Host app** (`Host/`) - Required container app for the extension
4. **UniFFI bindings** (`Generated/`) - Auto-generated Swift/C bindings

### Data-based, not view-based

`Info.plist` sets `QLIsDataBasedPreview` to `true` and the principal class is a
`QLPreviewProvider` subclass (`MBRPreview/PreviewProvider.swift`) implementing
`providePreview(for:)`. macOS asks for a *generation-based* preview; a view-based
reply is refused outright with

```
Error Domain=QuickLookPreviewErrors Code=1
"View based preview response received when expecting a generation based preview"
```

and QuickLook then falls back to the system plain-text previewer — which looks
exactly like the extension not being installed. (This is what issue #302 was.)

So there is no `WKWebView` in the extension and nothing for it to host: the
provider hands QuickLook one HTML document and a dictionary of attachments.

**Set `reply.attachments` before returning the reply, not inside the
`dataCreationBlock`.** `QLPreviewReply.h` says the block may update them, but on
macOS 26 attachments set there never arrive: the `cid:` URLs resolve to nothing
and every image in the preview is blank, with no error anywhere.

### Security invariant: a preview can only read files it was handed

Previewed markdown is untrusted and may contain raw HTML, so a `<script>` inside a
`.md` runs (JavaScript *is* enabled in a data-based HTML preview — that is what
makes highlight.js and mermaid work). It cannot, however, reach the filesystem:

- Local assets are `QLPreviewReply` attachments, addressed as `cid:<id>`. `cid:`
  resolves against that dictionary and nothing else, so there is no path for a
  script to name a file that is not already in it.
- The dictionary is built by `collect_preview_attachments` (`src/quicklook.rs`),
  which attaches a file only when `Path::canonicalize` puts it inside the
  previewed repository root **and** it is a regular file. `..`, extra leading
  slashes and symlinks pointing out of the repo all resolve away before the
  comparison, so none of them get in.

This is strictly stronger than the `mbrfile://` `WKURLSchemeHandler` it replaced,
which *was* reachable from page script and had to refuse bad paths one request at
a time. It also matters less that `MBRPreview/MBRPreview.entitlements` grants read
access to `/` — see the comment in that file for why it is that broad — because
nothing in the preview can ask for a path any more.

To sanity-check by hand, preview a `.md` containing:

```html
<img src="/../../etc/passwd">
<img src="/images/real-image.png">
```

Only the second must render. The first keeps its authored URL (`/../../etc/passwd`)
in the HTML, because no attachment was minted for it, and therefore loads nothing.


## Useful Tips

* Watch logs for mbr: `/usr/bin/log stream --style compact --predicate 'eventMessage CONTAINS "mbr"' --color=auto`
* xattr -cr Path/to/app
* xattr -r -d com.apple.quarantine "path/to/app"
* qlmanage -r
* qlmanage -r cache
* restart finder?
*
* binary distribution for macos on nix?
  * See https://github.com/NixOS/nixpkgs/blob/nixos-unstable/pkgs/by-name/ap/apparency/package.nix
* Should the plugin be under MyApp.app/Contents/Library/QuickLook instead?

## Building

### Prerequisites

- Xcode (for Swift compilation)
- Rust toolchain
- `xcodegen` (`brew install xcodegen`)
- `bun` (for building web components)

### Build Commands

```bash
# Build everything (Rust + Swift extension)
cd quicklook
./build.sh

# Build and install to MBR.app-template
./build.sh install
```

### Manual Build Steps

If you need more control:

```bash
# 0. Build js artifacts needed by rust build
cd components ; bun install && bun run build

# 1. Build Rust library with FFI feature (minimal features for sandbox)
cargo build --release --no-default-features --features ffi

# 2. Regenerate UniFFI bindings (REQUIRED after Rust API changes!)
cargo run --bin uniffi-bindgen --features ffi -- \
    generate --library target/release/libmbr.a \
    --language swift --out-dir quicklook/Generated

# 3. Generate Xcode project
cd quicklook
xcodegen generate

# 4. Build extension
xcodebuild -project MBRQuickLook.xcodeproj \
    -scheme MBRQuickLook \
    -configuration Release \
    -arch arm64 build
```

## Installation & Registration

### Install Location

The extension must be inside a signed app bundle. Options:

1. **~/Applications/** - User-level installation (recommended for dev)
2. **/Applications/** - System-level installation
3. **DerivedData** - Xcode automatically registers after build

### Manual Installation

```bash
# Copy the host app (contains the extension in PlugIns/)
cp -R ~/Library/Developer/Xcode/DerivedData/MBRQuickLook-*/Build/Products/Release/MBRQuickLookHost.app \
    ~/Applications/

# Re-sign (required after copying)
codesign --force --sign - \
    --entitlements quicklook/MBRPreview/MBRPreview.entitlements \
    ~/Applications/MBRQuickLookHost.app/Contents/PlugIns/MBRPreview.appex
codesign --force --sign - ~/Applications/MBRQuickLookHost.app

# Register with Launch Services
/System/Library/Frameworks/CoreServices.framework/Versions/Current/Frameworks/LaunchServices.framework/Versions/Current/Support/lsregister -f ~/Applications/MBRQuickLookHost.app

# Enable the extension
pluginkit -e use -i com.zmre.mbr.quicklook-host.MBRPreview
```

### Verify Registration

```bash
# List all QuickLook preview extensions
pluginkit -mAv -p com.apple.quicklook.preview

# Filter for MBR
pluginkit -mAv -p com.apple.quicklook.preview 2>&1 | grep -i mbr

# The output shows:
#   + = enabled
#   - = disabled
#   (no prefix) = available but not explicitly enabled/disabled
```

### Enable/Disable Extension

```bash
# Enable
pluginkit -e use -i com.zmre.mbr.quicklook-host.MBRPreview

# Disable
pluginkit -e ignore -i com.zmre.mbr.quicklook-host.MBRPreview
```

## Testing

### Quick Manual Test

```bash
# Preview a markdown file
qlmanage -p /path/to/file.md

# Preview with explicit content type
qlmanage -c net.daringfireball.markdown -p /path/to/file.md
```

### Check File UTI

The extension triggers based on UTI (Uniform Type Identifier). Check what UTI macOS assigns:

```bash
mdls -name kMDItemContentType -name kMDItemContentTypeTree /path/to/file.md
```

Expected output for markdown:
```
kMDItemContentType     = "net.daringfireball.markdown"
kMDItemContentTypeTree = (
    "public.item",
    "public.text",
    "public.data",
    "public.content",
    "net.daringfireball.markdown",
    "public.plain-text"
)
```

### Supported UTIs

The extension handles these UTIs (defined in `MBRPreview/Info.plist`):

- `net.daringfireball.markdown`
- `public.markdown`
- `dyn.ah62d4rv4ge81e5pe` (the dynamic UTI for `.rmd`)
- `public.plain-text` (covers `.txt`, `.log`, `.csv` and ~90 source extensions
  via `public.source-code`; those render through the plain-text path in
  `src/quicklook.rs`, not the markdown parser)

Entries that resolve to no declared type on the machine are skipped, but each one
costs an `Invalid content type identifier ... specified in extension` error from
QuickLook on every preview — so do not add speculative identifiers.

### Rust Unit Tests

```bash
# Run all quicklook tests (requires ffi feature)
cargo test --lib --features ffi quicklook

# Run specific test
cargo test --lib --features ffi test_render_preview_with_static_folder_image
```

## Debugging

### Is the extension even being invoked?

The extension logs to the `com.zmre.mbr.MBRPreview` subsystem at `.info`, which
is **not persisted by default** — without this, a perfectly working extension
looks silent. Stream it live while triggering a preview:

```bash
# In one shell (note: /usr/bin/log — `log` is a zsh builtin)
/usr/bin/log stream --style compact --level info \
  --predicate 'subsystem == "com.zmre.mbr.MBRPreview"'

# In another
qlmanage -p /path/to/file.md
```

A working preview logs three lines: `providePreview called for:`, `configRoot =`,
and `rendered N bytes of HTML with M attachment(s)`. No lines at all means
QuickLook never routed to the extension — see *Extension Not Being Invoked* below.

To persist the logs instead of streaming them (survives across runs, needs sudo):

```bash
sudo log config --subsystem com.zmre.mbr.MBRPreview --mode "level:debug,persist:debug"
sudo log config --subsystem com.zmre.mbr.MBRPreview --reset   # undo
```

**Test without `-c`.** `qlmanage -p -c net.daringfireball.markdown file.md` forces
the content type and takes a different routing path than Finder's spacebar. Plain
`qlmanage -p file.md` routes the way Finder does, which is what you want to test.
When QuickLook declines the extension, the `com.apple.quicklook` subsystem shows
`got displayBundleID com.apple.qldisplay.Text` (the system plain-text previewer);
when it accepts, it shows `com.apple.qldisplay.Web2`.

### Crash Logs

QuickLook extension crashes are logged to:

```
~/Library/Logs/DiagnosticReports/MBRPreview-*.ips
```

Check for recent crashes:

```bash
# Find recent crash logs
find ~/Library/Logs/DiagnosticReports -name "*MBR*" -mmin -30

# Read a crash log (JSON format)
cat ~/Library/Logs/DiagnosticReports/MBRPreview-*.ips | jq .
```

Key things to look for in crash logs:

- **Exception type**: `EXC_BREAKPOINT` often indicates Swift assertion failure
- **Stack trace**: Look for `makeRustCall`, `renderPreview`, `PreviewProvider`
- **UniFFI errors**: Crashes in `makeRustCall` often mean stale bindings

### Common Crash: Stale UniFFI Bindings

If the crash log shows:
```
"symbol":"_assertionFailure(_:_:file:line:flags:)"
"symbol":"specialized makeRustCall<A, B>(_:errorHandler:)"
"symbol":"renderPreview(filePath:configRoot:)"
```

**Solution**: Regenerate UniFFI bindings:

```bash
cargo build --release --no-default-features --features ffi
cargo run --bin uniffi-bindgen --features ffi -- \
    generate --library target/release/libmbr.a \
    --language swift --out-dir quicklook/Generated
```

Then rebuild and reinstall the extension.

### Kill QuickLook Processes

Sometimes you need to restart QuickLook services:

```bash
# Kill qlmanage
pkill -f qlmanage

# Kill QuickLook daemon (will auto-restart)
pkill -f quicklookd

# Nuclear option: restart Finder (also restarts QuickLook)
killall Finder
```

### System Logs

View QuickLook-related system logs:

```bash
# Stream logs while testing
log stream --predicate 'subsystem == "com.apple.quicklook"' --level debug

# View recent logs
log show --last 5m --predicate 'subsystem == "com.apple.quicklook"'
```

### Xcode Debugging

To debug the extension in Xcode:

1. Open `MBRQuickLook.xcodeproj`
2. Select the `MBRPreview` scheme
3. Edit scheme > Run > Info > Set "Executable" to "Ask on Launch"
4. Run, then select `qlmanage` when prompted
5. Add arguments: `-p /path/to/test/file.md`

## Troubleshooting

### Extension Not Being Invoked (Plain Text Shown)

Symptoms:
- QuickLook shows plain text instead of rendered markdown
- No debug files created in `/tmp/`
- No crash logs

Causes and solutions:

1. **Extension not registered**
   ```bash
   pluginkit -mAv -p com.apple.quicklook.preview | grep mbr
   # Should show the extension
   ```

2. **Extension disabled**
   ```bash
   pluginkit -e use -i com.zmre.mbr.quicklook-host.MBRPreview
   ```

3. **Old extension cached** - Kill QuickLook processes:
   ```bash
   pkill -f qlmanage
   pkill -f quicklookd
   ```

4. **Wrong UTI** - Check file's UTI matches supported types:
   ```bash
   mdls -name kMDItemContentType /path/to/file.md
   ```

5. **Competing extension** - Another extension might handle markdown. Note that
   `qlmanage -m plugins` **cannot see app extensions** — it lists only legacy
   `.qlgenerator` bundles and prints nothing for mbr whether or not things work.
   Use:
   ```bash
   pluginkit -mv -p com.apple.quicklook.preview
   pluginkit -e ignore -i <other.extension.id>   # to rule one out
   ```

6. **Stale Xcode build registered** - An Xcode Run registers a competing copy
   from DerivedData with a `!` (debugger) election, which macOS prioritises over
   the installed `+` one. Clear it:
   ```bash
   pluginkit -e default -i com.zmre.mbr.quicklook-host.MBRPreview
   ```

7. **App not launched from a real install location** - The extension only
   registers when the containing app is launched from a real install location.
   Running `MBR.app` out of `/nix/store` or a build output directory registers
   nothing.

### Extension Crashes on Launch

Symptoms:
- No log lines from the extension's subsystem (see above)
- Crash logs in `~/Library/Logs/DiagnosticReports/`

A crash looks exactly like "not installed": QuickLook swallows it and falls back
to the system previewer.

Common causes:

1. **Stale UniFFI bindings** - Regenerate (see above)
2. **Missing Rust library** - Rebuild with `cargo build --release --features ffi`
3. **Signing issues** - Re-sign the extension

### Images Broken in Preview

Symptoms:
- Markdown renders but images show as broken

Check:

1. **Attachments were made** - the extension's log line says how many:
   `rendered N bytes of HTML with M attachment(s)`. `M = 0` with images on the
   page means `collect_preview_attachments` resolved none of them.
2. **Config root detection** - the `configRoot =` log line; images resolve
   relative to it.
3. **Static folder** - verify images exist in the repo or its `static/` folder.
4. **Relative image URLs are not attached.** Only root-relative URLs
   (`![x](/images/y.png)`) become attachments. An image authored relative to the
   note (`![x](y.png)`) is rendered as a relative URL, which a data-based preview
   has no way to resolve, so it will not display. This has always been true of
   QuickLook previews; it is not new.
5. **Size caps** - an asset over 16 MiB, or one that would push the page's total
   past 64 MiB, is deliberately not attached (`MAX_ATTACHMENT_BYTES` /
   `MAX_TOTAL_ATTACHMENT_BYTES` in `src/quicklook.rs`).

### "Can't get generator" Error

When using `qlmanage -g` to force a generator:

```
qlmanage -g /path/to/extension.appex -c net.daringfireball.markdown -p file.md
Can't get generator at /path/to/extension.appex
```

This happens because `-g` is for old-style `.qlgenerator` bundles, not modern `.appex` extensions. Modern extensions are selected automatically based on UTI.

## UniFFI Binding Notes

### When to Regenerate

Regenerate bindings after ANY change to:

- Function signatures in `src/quicklook.rs`
- Error types (`QuickLookError`)
- Return types
- The `#[uniffi::export]` macro usage

### Binding Files

Generated files in `quicklook/Generated/`:

- `mbr.swift` - Swift bindings
- `mbrFFI.h` - C header for FFI
- `mbrFFI.modulemap` - Module map for Swift imports

### Debugging Binding Issues

If you suspect binding mismatch:

1. Check Rust function signature matches Swift usage
2. Compare error types between Rust and generated Swift
3. Look for `uniffi` version mismatch between Cargo.toml deps

## Entitlements

The extension runs in a sandboxed environment. Key entitlements in `MBRPreview.entitlements`:

- `com.apple.security.app-sandbox` - Required for extensions
- `com.apple.security.files.user-selected.read-only` - File access
- `com.apple.security.network.client` - For potential network requests
- `com.apple.security.temporary-exception.files.absolute-path.read-only` - Full filesystem read

## Version Matching

The extension version should match the host app version. Warning during build:

```
warning: The CFBundleShortVersionString of an app extension ('1.0') must match that of its containing parent app ('0.3.0').
```

Update versions in:
- `Host/Info.plist`
- `MBRPreview/Info.plist`

## Quick Reference

| Task | Command |
|------|---------|
| Build extension | `./build.sh` |
| Build + install | `./build.sh install` |
| Regenerate bindings | `cargo run --bin uniffi-bindgen --features ffi -- generate --library target/release/libmbr.a --language swift --out-dir quicklook/Generated` |
| Test preview | `qlmanage -p /path/to/file.md` |
| Check registration | `pluginkit -mAv -p com.apple.quicklook.preview \| grep mbr` |
| Enable extension | `pluginkit -e use -i com.zmre.mbr.quicklook-host.MBRPreview` |
| Check file UTI | `mdls -name kMDItemContentType /path/to/file.md` |
| View crash logs | `ls ~/Library/Logs/DiagnosticReports/*MBR*` |
| Run Rust tests | `cargo test --lib --features ffi quicklook` |
| Kill QuickLook | `pkill -f qlmanage && pkill -f quicklookd` |

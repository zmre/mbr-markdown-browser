# QuickLook Extension Development Guide

This document covers building, testing, debugging, and troubleshooting the MBR QuickLook extension.

## Architecture Overview

The QuickLook extension consists of:

1. **Rust library** (`src/quicklook.rs`) - Core rendering logic exposed via UniFFI
2. **Swift extension** (`MBRPreview/`) - macOS QuickLook extension that calls Rust
3. **Host app** (`Host/`) - Required container app for the extension
4. **UniFFI bindings** (`Generated/`) - Auto-generated Swift/C bindings

The extension uses a custom URL scheme (`mbrfile://`) to serve local assets (images, etc.) through a `WKURLSchemeHandler` in the WebView.

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
- `com.unknown.md`
- `dyn.ah62d4rv4ge81e5pe` (dynamic UTI fallback)

### Rust Unit Tests

```bash
# Run all quicklook tests (requires ffi feature)
cargo test --lib --features ffi quicklook

# Run specific test
cargo test --lib --features ffi test_render_preview_with_static_folder_image
```

## Debugging

### Debug Output Files

The Swift extension writes debug files to `/tmp/` when it runs:

- `/tmp/mbr_ql_start.txt` - Written at extension start
- `/tmp/mbr_ql_path.txt` - File path being previewed
- `/tmp/mbr_ql_root.txt` - Detected config root
- `/tmp/mbr_ql_html.txt` - Generated HTML (first 5000 chars)
- `/tmp/mbr_ql_error.txt` - Any errors

Check if the extension is being invoked:

```bash
# Clear old files
rm -f /tmp/mbr_ql_*

# Trigger QuickLook
qlmanage -p /path/to/file.md

# Check for debug files
ls -la /tmp/mbr_ql_*
```

If no files are created, the extension is not being invoked at all.

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
- **Stack trace**: Look for `makeRustCall`, `renderPreview`, `PreviewViewController`
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

5. **Competing extension** - Another extension might handle markdown:
   ```bash
   qlmanage -m plugins | grep -i markdown
   ```

### Extension Crashes on Launch

Symptoms:
- Debug files not created
- Crash logs in `~/Library/Logs/DiagnosticReports/`

Common causes:

1. **Stale UniFFI bindings** - Regenerate (see above)
2. **Missing Rust library** - Rebuild with `cargo build --release --features ffi`
3. **Signing issues** - Re-sign the extension

### Images Broken in Preview

Symptoms:
- Markdown renders but images show as broken
- `mbrfile://` URLs not resolving

Check:

1. **Config root detection** - Check `/tmp/mbr_ql_root.txt`
2. **Static folder** - Verify images exist in `static/` folder
3. **URL scheme handler** - Check for errors in Swift console

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

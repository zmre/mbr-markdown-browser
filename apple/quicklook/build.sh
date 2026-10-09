#!/usr/bin/env bash
# Build script for MBR QuickLook extension
#
# Usage:
#   ./build.sh          - Build extension only
#   ./build.sh install  - Build and install into local MBR.app

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$SCRIPT_DIR"

# Build the `mbr-ffi` crate alone: the UniFFI exports Swift calls, over the
# render core (`mbr-core`), with no server, watcher, static site generator,
# CLI, GUI or media-metadata. QuickLook extensions run sandboxed without GUI
# access or ffmpeg, and none of the rest is reachable from a preview. Produces
# target/release/libmbr_ffi.a. Same command as flake.nix's
# mbr-quicklook-staticlib.
echo "Building Rust library (mbr-ffi, for QuickLook)..."
cargo build --release -p mbr-ffi --lib --manifest-path "$PROJECT_ROOT/Cargo.toml"

# Regenerate Xcode project
echo "Generating Xcode project..."
xcodegen generate

# A fixed derived-data directory per checkout. Xcode's default
# (~/Library/Developer/Xcode/DerivedData/MBRQuickLook-<hash of the project
# path>) gets a new directory for every checkout or move of this project, and
# picking one of those by name installed whichever happened to sort first —
# often a stale build from another worktree.
DERIVED_DATA="$PROJECT_ROOT/target/quicklook-derived-data"

# Build the extension
echo "Building QuickLook extension..."
xcodebuild \
    -project MBRQuickLook.xcodeproj \
    -scheme MBRQuickLook \
    -configuration Release \
    -arch arm64 \
    -derivedDataPath "$DERIVED_DATA" \
    build

EXTENSION_PATH="$DERIVED_DATA/Build/Products/Release/MBRQuickLookHost.app/Contents/PlugIns/MBRPreview.appex"

echo ""
echo "Build complete!"
echo "Extension at: $EXTENSION_PATH"

# Install if requested
if [[ "${1:-}" == "install" ]]; then
    MBR_APP="$PROJECT_ROOT/macos/MBR.app-template"
    PLUGINS_DIR="$MBR_APP/Contents/PlugIns"
    MBR_BINARY="$MBR_APP/Contents/MacOS/mbr"

    echo ""
    echo "Installing extension into MBR.app..."

    # Create PlugIns directory if needed
    mkdir -p "$PLUGINS_DIR"

    # Remove old extension if exists
    rm -rf "$PLUGINS_DIR/MBRPreview.appex"

    # Copy new extension
    cp -R "$EXTENSION_PATH" "$PLUGINS_DIR/"

    # Replace symlink with actual binary (codesign requires regular files)
    if [[ -L "$MBR_BINARY" ]]; then
        echo "Replacing binary symlink with actual file..."
        REAL_BINARY=$(readlink -f "$MBR_BINARY")
        rm "$MBR_BINARY"
        cp "$REAL_BINARY" "$MBR_BINARY"
    fi

    # Re-sign the app bundle (preserving extension entitlements)
    echo "Re-signing MBR.app..."
    # First re-sign the extension with its entitlements
    /usr/bin/codesign --force --sign - \
        --entitlements "$SCRIPT_DIR/MBRPreview/MBRPreview.entitlements" \
        "$PLUGINS_DIR/MBRPreview.appex"
    # Then re-sign the host app
    /usr/bin/codesign --force --sign - "$MBR_APP"

    echo ""
    echo "Installation complete!"
    echo "Extension installed at: $PLUGINS_DIR/MBRPreview.appex"
    echo ""
    echo "To register the QuickLook extension, run MBR.app once:"
    echo "  open '$MBR_APP'"
    echo ""
    echo "Then test with:"
    echo "  qlmanage -p /path/to/file.md"
fi

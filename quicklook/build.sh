#!/usr/bin/env bash
# Build script for MBR QuickLook extension
#
# Usage:
#   ./build.sh          - Build extension only
#   ./build.sh install  - Build and install into local MBR.app

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$SCRIPT_DIR"

# Ensure Rust library is built WITHOUT GUI or media-metadata features
# QuickLook extensions run in a sandboxed environment without GUI access or ffmpeg
echo "Building Rust library (minimal features for QuickLook)..."
cargo build --release --no-default-features --manifest-path "$PROJECT_ROOT/Cargo.toml"

# Regenerate Xcode project
echo "Generating Xcode project..."
xcodegen generate

# Build the extension
echo "Building QuickLook extension..."
xcodebuild \
    -project MBRQuickLook.xcodeproj \
    -scheme MBRQuickLook \
    -configuration Release \
    -arch arm64 \
    build

# Get the derived data path
DERIVED_DATA="$HOME/Library/Developer/Xcode/DerivedData"
BUILD_DIR=$(find "$DERIVED_DATA" -maxdepth 1 -name "MBRQuickLook-*" -type d | head -1)
EXTENSION_PATH="$BUILD_DIR/Build/Products/Release/MBRQuickLookHost.app/Contents/PlugIns/MBRPreview.appex"

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

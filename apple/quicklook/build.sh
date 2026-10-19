#!/usr/bin/env bash
# Build script for MBR QuickLook extension
#
# Usage:
#   ./build.sh          - Build extension only
#   ./build.sh install  - Build and install both extensions into local MBR.app

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
#
# The caller's DEVELOPER_DIR (the Xcode one xcodebuild below needs) is kept
# away from cargo. Inside the nix dev shell, the cc wrapper that links build
# scripts carries its own SDK (DEVELOPER_DIR_<role>) and refuses to link when
# DEVELOPER_DIR names a different one ("Multiple conflicting values defined
# for DEVELOPER_DIR_arm64_apple_darwin"), so any build script that needs
# relinking failed the build. Outside nix, unsetting it just falls back to
# xcode-select's toolchain.
echo "Building Rust library (mbr-ffi, for QuickLook)..."
env -u DEVELOPER_DIR cargo build --release -p mbr-ffi --lib --manifest-path "$PROJECT_ROOT/Cargo.toml"

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

HOST_PLUGINS="$DERIVED_DATA/Build/Products/Release/MBRQuickLookHost.app/Contents/PlugIns"
# Both extensions, each with the entitlements it is signed with. The preview
# answers the spacebar panel; the thumbnail answers Finder's icons, gallery
# view and Get Info (one extension point per appex).
EXTENSIONS=(MBRPreview MBRThumbnail)

echo ""
echo "Build complete!"
for name in "${EXTENSIONS[@]}"; do
    echo "Extension at: $HOST_PLUGINS/$name.appex"
done

# Install if requested
if [[ "${1:-}" == "install" ]]; then
    MBR_APP="$PROJECT_ROOT/macos/MBR.app-template"
    PLUGINS_DIR="$MBR_APP/Contents/PlugIns"
    MBR_BINARY="$MBR_APP/Contents/MacOS/mbr"

    echo ""
    echo "Installing extension into MBR.app..."

    # Create PlugIns directory if needed
    mkdir -p "$PLUGINS_DIR"

    # Replace each extension with the fresh build
    for name in "${EXTENSIONS[@]}"; do
        rm -rf "$PLUGINS_DIR/$name.appex"
        cp -R "$HOST_PLUGINS/$name.appex" "$PLUGINS_DIR/"
    done

    # Replace symlink with actual binary (codesign requires regular files)
    if [[ -L "$MBR_BINARY" ]]; then
        echo "Replacing binary symlink with actual file..."
        REAL_BINARY=$(readlink -f "$MBR_BINARY")
        rm "$MBR_BINARY"
        cp "$REAL_BINARY" "$MBR_BINARY"
    fi

    # Re-sign the app bundle (preserving extension entitlements)
    echo "Re-signing MBR.app..."
    # First re-sign each extension with its own entitlements
    for name in "${EXTENSIONS[@]}"; do
        /usr/bin/codesign --force --sign - \
            --entitlements "$SCRIPT_DIR/$name/$name.entitlements" \
            "$PLUGINS_DIR/$name.appex"
    done
    # Then re-sign the host app
    /usr/bin/codesign --force --sign - "$MBR_APP"

    echo ""
    echo "Installation complete!"
    for name in "${EXTENSIONS[@]}"; do
        echo "Extension installed at: $PLUGINS_DIR/$name.appex"
    done
    echo ""
    echo "To register the QuickLook extension, run MBR.app once:"
    echo "  open '$MBR_APP'"
    echo ""
    echo "Then test the preview with:"
    echo "  qlmanage -p /path/to/file.md"
    echo ""
    echo "Thumbnails: qlmanage -t never reaches thumbnail extensions on current"
    echo "macOS. The thumbnail agent only discovers a new or updated extension"
    echo "when it starts, so log out and back in (or reboot), make sure it is"
    echo "enabled (pluginkit -e use -i <thumbnail bundle id>), clear the cache"
    echo "(qlmanage -r cache), then look at markdown files in Finder's icon or"
    echo "gallery view, or ask QLThumbnailGenerator as Finder does. See"
    echo "DEVELOP.md, 'Testing thumbnails'."
fi

#!/bin/bash
# Build script for macOS .app bundle
# Creates MBR.app with proper structure for distribution

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
APP_DIR="$PROJECT_DIR/macos/MBR.app"

echo "Building mbr release binary..."
cd "$PROJECT_DIR"
cargo build --release

echo "Updating app bundle..."
# Ensure the binary is in place (symlink or copy)
if [ -L "$APP_DIR/Contents/MacOS/mbr" ]; then
    # Symlink exists - verify it's valid
    if [ ! -e "$APP_DIR/Contents/MacOS/mbr" ]; then
        echo "Symlink is broken, recreating..."
        rm "$APP_DIR/Contents/MacOS/mbr"
        ln -s "$PROJECT_DIR/target/release/mbr" "$APP_DIR/Contents/MacOS/mbr"
    fi
else
    # Create symlink (for development) or copy (for distribution)
    if [ "$1" = "--dist" ]; then
        echo "Copying binary for distribution..."
        cp "$PROJECT_DIR/target/release/mbr" "$APP_DIR/Contents/MacOS/mbr"
    else
        echo "Creating symlink for development..."
        mkdir -p "$APP_DIR/Contents/MacOS"
        ln -sf "$PROJECT_DIR/target/release/mbr" "$APP_DIR/Contents/MacOS/mbr"
    fi
fi

# Ensure icon is up to date
if [ "$PROJECT_DIR/mbr-icon.png" -nt "$PROJECT_DIR/macos/AppIcon.icns" ]; then
    echo "Regenerating icon..."
    mkdir -p "$PROJECT_DIR/macos/AppIcon.iconset"
    sips -z 16 16     "$PROJECT_DIR/mbr-icon.png" --out "$PROJECT_DIR/macos/AppIcon.iconset/icon_16x16.png" > /dev/null
    sips -z 32 32     "$PROJECT_DIR/mbr-icon.png" --out "$PROJECT_DIR/macos/AppIcon.iconset/icon_16x16@2x.png" > /dev/null
    sips -z 32 32     "$PROJECT_DIR/mbr-icon.png" --out "$PROJECT_DIR/macos/AppIcon.iconset/icon_32x32.png" > /dev/null
    sips -z 64 64     "$PROJECT_DIR/mbr-icon.png" --out "$PROJECT_DIR/macos/AppIcon.iconset/icon_32x32@2x.png" > /dev/null
    sips -z 128 128   "$PROJECT_DIR/mbr-icon.png" --out "$PROJECT_DIR/macos/AppIcon.iconset/icon_128x128.png" > /dev/null
    sips -z 256 256   "$PROJECT_DIR/mbr-icon.png" --out "$PROJECT_DIR/macos/AppIcon.iconset/icon_128x128@2x.png" > /dev/null
    sips -z 256 256   "$PROJECT_DIR/mbr-icon.png" --out "$PROJECT_DIR/macos/AppIcon.iconset/icon_256x256.png" > /dev/null
    sips -z 512 512   "$PROJECT_DIR/mbr-icon.png" --out "$PROJECT_DIR/macos/AppIcon.iconset/icon_256x256@2x.png" > /dev/null
    sips -z 512 512   "$PROJECT_DIR/mbr-icon.png" --out "$PROJECT_DIR/macos/AppIcon.iconset/icon_512x512.png" > /dev/null
    sips -z 1024 1024 "$PROJECT_DIR/mbr-icon.png" --out "$PROJECT_DIR/macos/AppIcon.iconset/icon_512x512@2x.png" > /dev/null
    iconutil -c icns "$PROJECT_DIR/macos/AppIcon.iconset" -o "$PROJECT_DIR/macos/AppIcon.icns"
    cp "$PROJECT_DIR/macos/AppIcon.icns" "$APP_DIR/Contents/Resources/AppIcon.icns"
fi

echo ""
echo "✅ MBR.app built successfully!"
echo "   Location: $APP_DIR"
echo ""
echo "To run: open $APP_DIR"
echo "To install: cp -r $APP_DIR /Applications/"

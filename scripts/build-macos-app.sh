#!/bin/bash
# Build script for macOS .app bundle
# Creates MBR.app with proper structure for distribution

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
APP_DIR="$PROJECT_DIR/macos/MBR.app-template"

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

# The icon is checked in. scripts/gen-app-icons.sh regenerates it from the
# Icon Composer package (it needs Xcode's Icon Composer, so it is not run from
# here); warn when the checked-in .icns looks older than what it is made from.
ICNS="$PROJECT_DIR/macos/AppIcon.icns"
if [ -n "$(find "$SCRIPT_DIR/gen-app-icons.sh" "$PROJECT_DIR/apple/ios/MBRMobile/AppIcon.icon" \
        -type f -newer "$ICNS" -print -quit)" ]; then
    echo "warning: macos/AppIcon.icns is older than the icon sources; run scripts/gen-app-icons.sh"
fi
mkdir -p "$APP_DIR/Contents/Resources"
cp "$ICNS" "$APP_DIR/Contents/Resources/AppIcon.icns"

echo ""
echo "✅ MBR.app built successfully!"
echo "   Location: $APP_DIR"
echo ""
echo "To run: open $APP_DIR"
echo "To install: cp -r $APP_DIR /Applications/"

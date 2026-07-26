#!/usr/bin/env bash
# Build a universal (arm64 + x86_64) MBR.app and wrap it in a .dmg.
#
# The two architectures are built separately (Nix can only build for the host
# platform, so CI builds arm64 on an Apple Silicon runner and x86_64 on an Intel
# runner). This script fuses them with `lipo`, re-signs, and produces a DMG with
# the customary drag-to-/Applications layout.
#
# Usage:
#   scripts/make-universal-dmg.sh <arm64-app> <x86_64-app> <output.dmg>
#
#   <arm64-app>   path to MBR.app built for aarch64-darwin
#   <x86_64-app>  path to MBR.app built for x86_64-darwin
#   <output.dmg>  path to write, e.g. dist/mbr-macos-universal.dmg
#
# If only one app is available, pass it as both arguments — the result is a
# single-architecture DMG rather than a failure.

set -euo pipefail

if [ $# -ne 3 ]; then
    echo "usage: $0 <arm64-app> <x86_64-app> <output.dmg>" >&2
    exit 2
fi

ARM_APP="$1"
INTEL_APP="$2"
OUTPUT="$3"
VOLNAME="MBR"

for app in "$ARM_APP" "$INTEL_APP"; do
    if [ ! -d "$app" ]; then
        echo "error: not a directory: $app" >&2
        exit 1
    fi
done

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

STAGE="$WORK/stage"
mkdir -p "$STAGE"

# Start from the arm64 bundle; it is the reference for structure and resources.
# `ditto` rather than `cp -R`: it preserves extended attributes and resource
# forks, which matter on an already-signed bundle.
ditto "$ARM_APP" "$STAGE/MBR.app"
APP="$STAGE/MBR.app"
chmod -R u+w "$APP"

# Fuse every Mach-O binary that exists in both bundles. This covers the main
# executable, the bundled pdfium dylib, and the QuickLook .appex executable
# without hardcoding their paths, so new nested binaries are picked up too.
fused=0
skipped=0
while IFS= read -r -d '' target; do
    rel="${target#"$APP"/}"
    counterpart="$INTEL_APP/$rel"

    # Only Mach-O files can be fused; skip plists, icons, etc.
    if ! file -b "$target" | grep -q 'Mach-O'; then
        continue
    fi

    if [ ! -f "$counterpart" ]; then
        echo "warn: no x86_64 counterpart for $rel — leaving single-arch" >&2
        skipped=$((skipped + 1))
        continue
    fi

    # Already universal (e.g. a prebuilt fat dylib): nothing to do.
    if lipo -archs "$target" 2>/dev/null | grep -q 'x86_64'; then
        continue
    fi

    lipo -create "$target" "$counterpart" -output "$WORK/fused.tmp"
    # Preserve the original mode; lipo's output is 0644.
    chmod --reference="$target" "$WORK/fused.tmp" 2>/dev/null ||
        chmod "$(stat -f '%Lp' "$target")" "$WORK/fused.tmp"
    mv "$WORK/fused.tmp" "$target"
    echo "fused: $rel ($(lipo -archs "$target"))"
    fused=$((fused + 1))
done < <(find "$APP" -type f -perm -u+r -print0)

echo "Fused $fused binaries ($skipped skipped)."

if [ "$fused" -eq 0 ]; then
    echo "error: no binaries were fused — the two bundles do not match" >&2
    exit 1
fi

# Replacing the executables breaks the bundle seal (Contents/_CodeSignature
# hashes them), so everything must be re-signed innermost-out. Signing has to be
# the LAST step: an ad-hoc-signed app gets Gatekeeper's recoverable "Open Anyway"
# flow, whereas one modified after signing is reported as "damaged" with no way
# through. `--deep` is deprecated for signing on macOS 13+, so sign nested code
# explicitly; it is still fine for verification.
ENTITLEMENTS="$(cd "$(dirname "$0")/.." && pwd)/quicklook/MBRPreview/MBRPreview.entitlements"

# Strip build-machine metadata (quarantine, provenance xattrs) before signing.
xattr -cr "$APP"

if [ -f "$APP/Contents/Frameworks/libpdfium.dylib" ]; then
    codesign --force --sign - --timestamp=none "$APP/Contents/Frameworks/libpdfium.dylib"
fi

if [ -d "$APP/Contents/PlugIns/MBRPreview.appex" ]; then
    if [ -f "$ENTITLEMENTS" ]; then
        codesign --force --sign - --timestamp=none \
            --entitlements "$ENTITLEMENTS" \
            "$APP/Contents/PlugIns/MBRPreview.appex"
    else
        codesign --force --sign - --timestamp=none "$APP/Contents/PlugIns/MBRPreview.appex"
    fi
fi

codesign --force --sign - --timestamp=none "$APP"
codesign --verify --deep --strict "$APP"

# Drag-to-install target. A plain symlink is all the standard DMG layout needs;
# a background image would require a scripted Finder window, which does not work
# on headless CI runners.
ln -s /Applications "$STAGE/Applications"

mkdir -p "$(dirname "$OUTPUT")"
rm -f "$OUTPUT"

# hdiutil intermittently fails with "Resource busy" on CI when a previous
# attachment has not finished detaching, so retry a few times.
for attempt in 1 2 3; do
    # -fs HFS+ is deliberate: hdiutil now defaults to APFS, which requires
    # macOS 10.13+ to mount. UDZO mounts in-kernel and has no minimum-OS
    # constraint. Building straight from -srcfolder never attaches the volume
    # and never talks to Finder, which is what makes this work headlessly.
    if hdiutil create \
        -volname "$VOLNAME" \
        -srcfolder "$STAGE" \
        -fs HFS+ \
        -format UDZO \
        -imagekey zlib-level=9 \
        -ov \
        "$OUTPUT"; then
        break
    fi
    if [ "$attempt" -eq 3 ]; then
        echo "error: hdiutil create failed after 3 attempts" >&2
        exit 1
    fi
    echo "hdiutil create failed (attempt $attempt), retrying..." >&2
    sleep 5
done

echo ""
echo "Created $OUTPUT"
ls -lh "$OUTPUT"

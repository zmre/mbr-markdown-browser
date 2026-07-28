#!/usr/bin/env bash
# Wrap a built MBR.app in a drag-to-/Applications .dmg.
#
# macOS releases are Apple Silicon (arm64) only. Intel macOS was dropped from
# the release pipeline: no CI job ever warmed an Intel Darwin cache, so every
# release paid a from-source build of ffmpeg-static and the entire dependency
# graph on a slow Intel runner. The flake still builds on x86_64-darwin for
# anyone compiling from source — there is simply no prebuilt Intel artifact.
#
# Usage:
#   scripts/make-macos-dmg.sh <app> <output.dmg>
#
#   <app>         path to MBR.app built for aarch64-darwin
#   <output.dmg>  path to write, e.g. dist/mbr-macos-arm64.dmg

set -euo pipefail

if [ $# -ne 2 ]; then
    echo "usage: $0 <app> <output.dmg>" >&2
    exit 2
fi

APP_SRC="$1"
OUTPUT="$2"
VOLNAME="MBR"

if [ ! -d "$APP_SRC" ]; then
    echo "error: not a directory: $APP_SRC" >&2
    exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

STAGE="$WORK/stage"
mkdir -p "$STAGE"

# `ditto` rather than `cp -R`: it preserves extended attributes and resource
# forks, which matter on an already-signed bundle.
ditto "$APP_SRC" "$STAGE/MBR.app"
APP="$STAGE/MBR.app"
# Nix store copies are read-only; codesign needs to write in place.
chmod -R u+w "$APP"

# Re-sign even though the Nix build already ad-hoc signed this bundle: it has
# since been through a tar roundtrip between runners and has its xattrs stripped
# just below, either of which can perturb the bundle seal. Re-signing is cheap
# and idempotent, and it must be the LAST step before packaging — an
# ad-hoc-signed app gets Gatekeeper's recoverable "Open Anyway" flow, whereas
# one modified after signing is reported as "damaged" with no way through.
# Sign innermost-out; `--deep` is deprecated for signing on macOS 13+, so nested
# code is signed explicitly, but --deep is still fine for verification.
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

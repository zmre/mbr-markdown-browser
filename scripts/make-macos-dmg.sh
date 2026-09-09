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
#
# Signs, and notarizes, for real when IDENTITY and KEYCHAIN are both set in
# the environment (a "Developer ID Application: ..." common name and the
# keychain holding its private key — see .github/workflows/release.yml's
# "Import the Developer ID certificate" step and docs/releasing.md). Both
# empty is the default and reproduces the script's original ad-hoc-only
# behavior: no secrets, no network calls, just a Gatekeeper-blocked-but-openable
# DMG — which is what a dry run, or any local run without the signing secrets,
# exercises. Setting exactly one of the two is a misconfiguration and is
# rejected rather than silently falling back to ad-hoc.
#
# Notarization additionally needs APPLE_API_KEY_P8 / APPLE_API_KEY_ID /
# APPLE_API_ISSUER_ID, read by scripts/notarize.sh — this script does not
# inspect them itself, so a missing one surfaces there with its own message.

set -euo pipefail

if [ $# -ne 2 ]; then
    echo "usage: $0 <app> <output.dmg>" >&2
    exit 2
fi

APP_SRC="$1"
OUTPUT="$2"
VOLNAME="MBR"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ ! -d "$APP_SRC" ]; then
    echo "error: not a directory: $APP_SRC" >&2
    exit 1
fi

IDENTITY="${IDENTITY:-}"
KEYCHAIN="${KEYCHAIN:-}"
if [ -n "$IDENTITY" ] || [ -n "$KEYCHAIN" ]; then
    if [ -z "$IDENTITY" ] || [ -z "$KEYCHAIN" ]; then
        echo "error: IDENTITY and KEYCHAIN must both be set to sign for real (one is empty)" >&2
        exit 1
    fi
    REAL_SIGN=1
else
    REAL_SIGN=0
fi

WORK="$(mktemp -d)"
# Also detaches a DMG mounted by the verification step below, if the script
# fails or is interrupted after attaching it. MOUNT is set just before the
# attach and cleared right after the matching detach, so a clean run leaves
# this a no-op.
MOUNT=""
cleanup() {
    [ -n "$MOUNT" ] && hdiutil detach "$MOUNT" >/dev/null 2>&1
    rm -rf "$WORK"
}
trap cleanup EXIT

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

# Real signing needs the hardened runtime (`--options runtime`) — required for
# notarization — and a real `--timestamp` (contacts Apple's timestamp server;
# without it the signature expires with the certificate and notarization
# refuses it outright). Ad-hoc signing can do neither: identity `-` has no
# timestamp service and hardened runtime buys nothing without notarization.
sign() {
    local path="$1"
    shift
    if [ "$REAL_SIGN" -eq 1 ]; then
        codesign --force --options runtime --timestamp \
            --keychain "$KEYCHAIN" --sign "$IDENTITY" "$@" "$path"
    else
        codesign --force --sign - --timestamp=none "$@" "$path"
    fi
}

if [ -f "$APP/Contents/Frameworks/libpdfium.dylib" ]; then
    sign "$APP/Contents/Frameworks/libpdfium.dylib"
fi

if [ -d "$APP/Contents/PlugIns/MBRPreview.appex" ]; then
    if [ -f "$ENTITLEMENTS" ]; then
        sign "$APP/Contents/PlugIns/MBRPreview.appex" --entitlements "$ENTITLEMENTS"
    else
        sign "$APP/Contents/PlugIns/MBRPreview.appex"
    fi
fi

sign "$APP"
codesign --verify --deep --strict "$APP"

# Notarize the APP before building the DMG, and staple the ticket into the
# bundle. Stapling the DMG alone (the common shortcut) leaves the app itself
# ticketless, so a user who drags it to /Applications and first launches it
# offline gets a Gatekeeper failure. Two round trips is the price of the app
# working without a network.
if [ "$REAL_SIGN" -eq 1 ]; then
    # notarytool takes a zip/dmg/pkg, never a bare .app directory. `ditto -c -k
    # --keepParent` is the archiver Apple documents as preserving the
    # signature; `zip` mangles symlinks and extended attributes, and the
    # submission is rejected for it.
    ditto -c -k --keepParent "$APP" "$WORK/MBR.zip"
    "$SCRIPT_DIR/notarize.sh" "$WORK/MBR.zip"

    xcrun stapler staple "$APP"
    xcrun stapler validate "$APP"
fi

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

# The DMG is a separate artifact from the app and needs its own signature and
# its own ticket — Gatekeeper checks the disk image the user double-clicks,
# not just what is inside it. No hardened runtime here: a disk image is not
# code, so notarization does not require `--options runtime` on it.
if [ "$REAL_SIGN" -eq 1 ]; then
    codesign --force --timestamp --keychain "$KEYCHAIN" --sign "$IDENTITY" "$OUTPUT"
    "$SCRIPT_DIR/notarize.sh" "$OUTPUT"
    xcrun stapler staple "$OUTPUT"

    # The end-to-end assertion, made against the artifacts about to be
    # published rather than against any step that produced them. `source=
    # Notarized Developer ID` is the exact string that means a downloaded copy
    # opens with no right-click-Open dance; anything else (`Unnotarized
    # Developer ID`, a rejection) fails the build here instead of on a user's
    # Mac.
    echo "--- verify: app ---"
    codesign --verify --deep --strict --verbose=2 "$APP"
    spctl --assess --type exec -vvv "$APP" 2>&1 | tee "$WORK/spctl-app.txt"
    grep -q "source=Notarized Developer ID" "$WORK/spctl-app.txt"

    echo "--- verify: dmg ---"
    xcrun stapler validate "$OUTPUT"

    echo "--- verify: staple survives a mount+copy (what a user actually does) ---"
    MOUNT=$(hdiutil attach "$OUTPUT" -nobrowse -readonly | awk -F'\t' 'END { print $NF }')
    spctl --assess --type exec -vvv "$MOUNT/MBR.app" 2>&1 | tee "$WORK/spctl-mounted.txt"
    grep -q "source=Notarized Developer ID" "$WORK/spctl-mounted.txt"
    hdiutil detach "$MOUNT" >/dev/null 2>&1
    MOUNT=""
fi

echo ""
echo "Created $OUTPUT"
ls -lh "$OUTPUT"

#!/usr/bin/env bash
# Regenerate every checked-in app-icon artifact from two sources of truth:
#
#   graphic-sources/mbr-logo-simple.svg          the designer's logo export
#   apple/ios/MBRMobile/AppIcon.icon/icon.json   the Icon Composer document
#                                                (layers, glass, appearances)
#
# Writes:
#   apple/ios/MBRMobile/AppIcon.icon/Assets/lines.svg   the tree's lines, one
#                                                       1024x1024 flat layer
#   apple/ios/MBRMobile/AppIcon.icon/Assets/nodes.svg   the nodes, one
#                                                       1024x1024 glass layer
#   macos/AppIcon.iconset/*.png                         the ten legacy sizes
#   macos/AppIcon.icns                                  iconutil of the iconset
#   macos/MBR.app-template/Contents/Resources/AppIcon.icns   a copy of it
#   macos/Assets.car                                    actool's compiled icon
#
# The Mac app ships its icon twice. Assets.car is the real one: macOS 26 looks
# up the asset Info.plist's CFBundleIconName names (AppIcon) in the bundle's
# compiled asset catalog and draws the layered Liquid Glass icon from it.
# AppIcon.icns is the fallback, for macOS before 26 and anything that reads
# CFBundleIconFile. flake.nix copies both into the bundle from their paths
# under macos/, and a flake sees only git-tracked files: `git add
# macos/Assets.car` after this script writes it (Step C says when it does).
#
# The iOS app ships the .icon package itself: Xcode derives every bitmap (and
# the dark / clear / tinted variants) from it at build time, so no iOS PNG is
# generated here. The Linux icon (mbr-icon.png) is a separate artwork and is
# not touched.
#
# Needs: Xcode with Icon Composer (its `ictool` renders the package) and
# actool (compiles it), the latter through Apple's /usr/bin/xcrun — the nix
# shell's own `xcrun` is xcbuild's and finds no Xcode tool; assetutil,
# iconutil and xmllint (macOS).
#
# ictool 27.1 quirks, measured — keep icon.json in the shape it has:
#   - The key `color-space-for-untagged-svg-colors` makes ictool refuse the
#     document ("The data couldn't be read because it isn't in the correct
#     format"). Do not add it.
#   - `fill-specializations` is IGNORED when a top-level `fill` key is
#     present, and with the system's dark background the Dark rendition
#     recolours light foreground layers to the dark fill (the nodes went navy).
#     The working shape — the one Icon Composer 2 writes itself — is no
#     top-level `fill`, and `fill-specializations` with a base entry (no
#     `appearance`) plus an `"appearance": "dark"` entry, both navy. With it
#     Dark keeps the navy background and the nodes stay pale.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
XCODE_DEV="${MBR_XCODE_DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
ICTOOL="$XCODE_DEV/../Applications/Icon Composer.app/Contents/Executables/ictool"
# Apple's xcrun, by absolute path: the one on PATH in `nix develop` is
# xcbuild's and knows no Xcode tool.
XCRUN=/usr/bin/xcrun

SOURCE_SVG="$PROJECT_ROOT/graphic-sources/mbr-logo-simple.svg"
ICON_PKG="$PROJECT_ROOT/apple/ios/MBRMobile/AppIcon.icon"
ICONSET="$PROJECT_ROOT/macos/AppIcon.iconset"
ICNS="$PROJECT_ROOT/macos/AppIcon.icns"
BUNDLE_ICNS="$PROJECT_ROOT/macos/MBR.app-template/Contents/Resources/AppIcon.icns"
CAR="$PROJECT_ROOT/macos/Assets.car"
FLAKE="$PROJECT_ROOT/flake.nix"

# The icon's palette. These must match the source SVG's <style> block (the
# guard below checks); if the designer changes a colour, update it here AND
# the matching srgb components of `fill-specializations` in icon.json.
LINE_COLOR="#39d2e1"
NODE_COLOR="#c9faff"
BG_COLOR="#2c214f"
# The source draws its lines at stroke-width 9 (in its 1000-unit square);
# that thin a line vanishes below ~100px, so the icon uses 14. Icon only —
# the graphic-sources SVG is left as the designer exported it.
ICON_STROKE=14

for tool in xmllint iconutil; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "error: $tool not found on PATH" >&2
        exit 1
    }
done
[[ -x "$ICTOOL" ]] || {
    echo "error: ictool not found at $ICTOOL" >&2
    echo "       Icon Composer ships inside Xcode; set MBR_XCODE_DEVELOPER_DIR if Xcode is elsewhere" >&2
    exit 1
}
xcrun_tool() { DEVELOPER_DIR="$XCODE_DEV" "$XCRUN" "$@"; }
for tool in actool assetutil; do
    xcrun_tool --find "$tool" >/dev/null 2>&1 || {
        echo "error: $XCRUN cannot find $tool with DEVELOPER_DIR=$XCODE_DEV" >&2
        echo "       set MBR_XCODE_DEVELOPER_DIR if Xcode is elsewhere" >&2
        exit 1
    }
done
[[ -f "$SOURCE_SVG" ]] || { echo "error: $SOURCE_SVG missing" >&2; exit 1; }
[[ -f "$ICON_PKG/icon.json" ]] || { echo "error: $ICON_PKG/icon.json missing" >&2; exit 1; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/mbr-app-icons.XXXXXX")"
trap 'rm -rf "$WORK" 2>/dev/null || true' EXIT

xpath() { xmllint --xpath "$1" "$SOURCE_SVG"; }

# Negate a decimal string exactly (no float round trip); zero stays "0".
negate() {
    if awk -v v="$1" 'BEGIN { exit !(v + 0 == 0) }'; then
        echo 0
    elif [[ "$1" == -* ]]; then
        echo "${1#-}"
    else
        echo "-$1"
    fi
}

# --- Step A: the two layer SVGs ---------------------------------------------

echo "Reading $SOURCE_SVG..."
for color in "$LINE_COLOR" "$NODE_COLOR" "$BG_COLOR"; do
    grep -qiF "$color" "$SOURCE_SVG" || {
        echo "error: the source no longer uses $color; the palette changed." >&2
        echo "       Update LINE_COLOR/NODE_COLOR/BG_COLOR here and the srgb values in icon.json." >&2
        exit 1
    }
done

# The source's <rect> is the icon square; everything else lies inside it. The
# layers map that square onto a 1024x1024 canvas: scale by 1024/width, after
# moving the rect's corner to the origin.
rect_x="$(xpath "string((//*[local-name()='rect'])[1]/@x)")"
rect_y="$(xpath "string((//*[local-name()='rect'])[1]/@y)")"
rect_w="$(xpath "string((//*[local-name()='rect'])[1]/@width)")"
rect_x="${rect_x:-0}"
rect_y="${rect_y:-0}"
[[ -n "$rect_w" ]] || { echo "error: no <rect> with a width in the source" >&2; exit 1; }
scale="$(awk -v w="$rect_w" 'BEGIN { printf "%.6g", 1024 / w }')"
transform="scale($scale) translate($(negate "$rect_x") $(negate "$rect_y"))"
echo "  rect ${rect_w}x${rect_w} at ($rect_x, $rect_y) -> transform=\"$transform\""

# xmllint prints each matched element on its own line, without the document's
# xmlns (the layer <svg> declares it). Only the class attribute goes: the
# colours and stroke live on the group instead of a <style> block.
xpath "//*[local-name()='path']" | sed -E 's/ class="[^"]*"//g' >"$WORK/paths.txt"
xpath "//*[local-name()='circle']" | sed -E 's/ class="[^"]*"//g' >"$WORK/circles.txt"
n_paths="$(grep -c '<path' "$WORK/paths.txt" || true)"
n_circles="$(grep -c '<circle' "$WORK/circles.txt" || true)"
[[ "$n_paths" -gt 0 ]] || { echo "error: no <path> elements found in the source" >&2; exit 1; }
[[ "$n_circles" -gt 0 ]] || { echo "error: no <circle> elements found in the source" >&2; exit 1; }

write_layer() {
    local out="$1" group_attrs="$2" elements="$3"
    {
        echo '<?xml version="1.0" encoding="UTF-8"?>'
        echo '<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">'
        echo "  <g transform=\"$transform\" $group_attrs>"
        sed 's/^[[:space:]]*/    /' "$elements"
        echo '  </g>'
        echo '</svg>'
    } >"$WORK/layer.svg"
    xmllint --noout "$WORK/layer.svg"
    mkdir -p "$(dirname "$out")"
    cp "$WORK/layer.svg" "$out"
}

echo "Writing layer SVGs..."
write_layer "$ICON_PKG/Assets/lines.svg" \
    "fill=\"none\" stroke=\"$LINE_COLOR\" stroke-width=\"$ICON_STROKE\" stroke-linecap=\"round\" stroke-linejoin=\"round\"" \
    "$WORK/paths.txt"
write_layer "$ICON_PKG/Assets/nodes.svg" \
    "fill=\"$NODE_COLOR\"" \
    "$WORK/circles.txt"
echo "  lines.svg: $n_paths paths at stroke $ICON_STROKE; nodes.svg: $n_circles circles"

# --- Step B: the macOS .icns, the fallback icon -----------------------------
#
# Read through CFBundleIconFile by macOS before 26 and by anything that never
# looks at the asset catalog. Each of the ten iconset sizes is ictool's own
# macOS render at exactly that pixel size: full-bleed, with the rounded-square
# corners ictool draws, nothing composited onto it. It used to carry Apple's
# 824/1024 inset and a shadow (the grid actool's own fallback .icns uses), but
# macOS 26 treats any legacy .icns as legacy artwork: one that does not fill
# its canvas is set on a white system squircle — the Dock showed a navy square
# inside a white one — and even a full-bleed one gets a thin light frame.
# Full-bleed is the best a legacy .icns can do there; only the compiled icon
# (Step C), which macOS 26 prefers, has no frame. Verified with
# NSWorkspace.icon(forFile:), which draws what the Dock and Finder draw.

render_size() {
    local px="$1" out="$2"
    # ictool prints an empty JSON object on stdout for a successful export.
    # Its PNG is byte-identical across runs, so it is used as is.
    "$ICTOOL" "$ICON_PKG" --export-image --output-file "$out" \
        --platform macOS --rendition Default \
        --width "$px" --height "$px" --scale 1 >/dev/null
}

echo "Rendering the macOS iconset through ictool..."
mkdir -p "$ICONSET"
# iconset entry -> pixel size. 32, 256 and 512 each serve two entries.
ICONSET_ENTRIES=(
    "icon_16x16:16"
    "icon_16x16@2x:32"
    "icon_32x32:32"
    "icon_32x32@2x:64"
    "icon_128x128:128"
    "icon_128x128@2x:256"
    "icon_256x256:256"
    "icon_256x256@2x:512"
    "icon_512x512:512"
    "icon_512x512@2x:1024"
)
for entry in "${ICONSET_ENTRIES[@]}"; do
    name="${entry%%:*}"
    px="${entry##*:}"
    if [[ ! -f "$WORK/size-$px.png" ]]; then
        render_size "$px" "$WORK/size-$px.png"
    fi
    cp "$WORK/size-$px.png" "$ICONSET/$name.png"
done

echo "Writing $ICNS..."
iconutil -c icns "$ICONSET" -o "$ICNS"
mkdir -p "$(dirname "$BUNDLE_ICNS")"
cp "$ICNS" "$BUNDLE_ICNS"

# --- Step C: the macOS 26 icon, actool's Assets.car -------------------------
#
# macOS 26 draws the app icon from the bundle's compiled asset catalog — the
# asset Info.plist's CFBundleIconName names, AppIcon in flake.nix's plist and
# the template's — as the real layered Liquid Glass icon: no white backing, no
# frame, the dark / clear / tinted appearances included. actool compiles the
# Icon Composer package for macOS into Assets.car (and writes a 4-size
# AppIcon.icns beside it, ignored: Step B's is the fallback). The deployment
# target is flake.nix's LSMinimumSystemVersion, which actool records as the
# catalog's PlatformVersion.
#
# A rebuilt car is never byte-identical: the catalog header carries a build
# Timestamp, and the seven bitmap renditions actool rasterizes for pre-26
# readers (the "Icon Image" entries, 16..1024px) get a fresh temp-file name
# (UUID + pid) and with it a fresh SHA1Digest — measured on two runs of the
# same input. Everything else in `assetutil --info` is stable and names the
# inputs: the Xcode version (AssetStorageVersion), PlatformVersion, the layer
# SVGs' and colours' own digests, and every layer's glass properties; a
# one-colour change to a layer changes that layer's digest (measured too). So
# a no-op run keeps the old file when the old and new listings agree once the
# unstable lines are dropped, and git sees no change. To force a rebuild,
# delete macos/Assets.car first.

min_os="$(sed -nE '/LSMinimumSystemVersion/{n;s/.*<string>([0-9.]+)<\/string>.*/\1/p;q;}' "$FLAKE")"
if [[ -z "$min_os" ]]; then
    echo "  flake.nix names no LSMinimumSystemVersion; compiling for 11.0"
    min_os=11.0
fi

echo "Compiling the macOS asset catalog with actool (deployment target $min_os)..."
mkdir -p "$WORK/car"
# actool reports on stdout, warnings and errors included.
if ! xcrun_tool actool --output-format human-readable-text --notices --warnings --errors \
    --app-icon AppIcon --output-partial-info-plist "$WORK/car/partial.plist" \
    --platform macosx --target-device mac --minimum-deployment-target "$min_os" \
    --compile "$WORK/car" "$ICON_PKG" >"$WORK/car/actool.log" 2>&1; then
    cat "$WORK/car/actool.log" >&2
    echo "error: actool failed" >&2
    exit 1
fi
grep -iE 'warning|error' "$WORK/car/actool.log" >&2 || true
# actool exits 0 even when it compiled no icon of that name (measured with a
# package not called AppIcon.icon), so the result is checked, not the status:
# the partial plist must name the icon the bundles' Info.plists expect.
icon_name="$(xmllint --xpath 'string(//key[.="CFBundleIconName"]/following-sibling::string[1])' \
    "$WORK/car/partial.plist" 2>/dev/null || true)"
[[ "$icon_name" == "AppIcon" ]] || {
    cat "$WORK/car/actool.log" >&2
    echo "error: actool did not compile an app icon named AppIcon (its partial plist says '${icon_name:-nothing}')" >&2
    exit 1
}

# The stable part of `assetutil --info` (see above). assetutil prints each
# entry's keys sorted, so an entry's AssetType line comes before the lines
# dropped for it; top-level entries open at two spaces, their keys sit at four,
# and a layer nested in a group sits deeper and is never a bitmap rendition.
car_fingerprint() {
    xcrun_tool assetutil --info "$1" | awk '
        /^  \{/ { bitmap = 0 }
        /^    "AssetType" : "Icon Image"/ { bitmap = 1 }
        /"Timestamp" :/ { next }
        bitmap && /^    "(RenditionName|SHA1Digest)" :/ { next }
        { print }'
}
car_fingerprint "$WORK/car/Assets.car" >"$WORK/car/new.info"
[[ -s "$WORK/car/new.info" ]] || {
    echo "error: assetutil cannot read the Assets.car actool wrote" >&2
    exit 1
}
if [[ -f "$CAR" ]] && car_fingerprint "$CAR" 2>/dev/null | cmp -s - "$WORK/car/new.info"; then
    echo "  $CAR already holds these assets; kept (a rewrite would change only actool's timestamp and temp names)"
    # Its mtime moves all the same: build-macos-app.sh judges staleness by
    # mtime against the layer SVGs Step A just rewrote, and git keeps no mtimes.
    touch "$CAR"
else
    cp "$WORK/car/Assets.car" "$CAR"
    echo "  wrote $CAR — git add it; the flake copies only tracked files"
fi

echo ""
echo "Done. Regenerated:"
echo "  $ICON_PKG/Assets/{lines,nodes}.svg"
echo "  $ICONSET/*.png"
echo "  $ICNS"
echo "  $BUNDLE_ICNS"
echo "  $CAR"

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
#
# The iOS app ships the .icon package itself: Xcode derives every bitmap (and
# the dark / clear / tinted variants) from it at build time, so no iOS PNG is
# generated here. The Linux icon (mbr-icon.png) is a separate artwork and is
# not touched.
#
# Needs: Xcode with Icon Composer (its `ictool` renders the package),
# iconutil, xmllint, and ImageMagick's `magick` (in `nix develop`).
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

SOURCE_SVG="$PROJECT_ROOT/graphic-sources/mbr-logo-simple.svg"
ICON_PKG="$PROJECT_ROOT/apple/ios/MBRMobile/AppIcon.icon"
ICONSET="$PROJECT_ROOT/macos/AppIcon.iconset"
ICNS="$PROJECT_ROOT/macos/AppIcon.icns"
BUNDLE_ICNS="$PROJECT_ROOT/macos/MBR.app-template/Contents/Resources/AppIcon.icns"

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

for tool in xmllint iconutil magick; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "error: $tool not found on PATH (magick comes from \`nix develop\`)" >&2
        exit 1
    }
done
[[ -x "$ICTOOL" ]] || {
    echo "error: ictool not found at $ICTOOL" >&2
    echo "       Icon Composer ships inside Xcode; set MBR_XCODE_DEVELOPER_DIR if Xcode is elsewhere" >&2
    exit 1
}
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

# --- Step B: the macOS .icns -------------------------------------------------
#
# A legacy .icns has no system inset: whatever is in the file is drawn as-is,
# on macOS 26 as well as before. Apple's own Mac icons therefore sit on a
# 1024-point grid with the artwork in the central 824x824 and a soft shadow
# beneath, and that is exactly the geometry actool's fallback .icns uses when
# it compiles an Icon Composer package for macOS (measured at 256px: a
# 206x206 square at +25+25, shadow bbox 216 at +20+22). actool's own .icns
# stops at 256px, useless for Finder's 512/1024, so each of the ten iconset
# sizes is rendered through ictool at its inner size and placed on that grid.

render_size() {
    local px="$1" out="$2"
    local inner sigma off
    inner=$(( (px * 824 + 512) / 1024 ))
    sigma="$(awk -v p="$px" 'BEGIN { printf "%.2f", p * 0.010 }')"
    off="$(awk -v p="$px" 'BEGIN { printf "%d", p * 0.008 + 0.5 }')"
    # ictool prints an empty JSON object on stdout for a successful export.
    "$ICTOOL" "$ICON_PKG" --export-image --output-file "$WORK/inner-$px.png" \
        --platform macOS --rendition Default \
        --width "$inner" --height "$inner" --scale 1 >/dev/null
    magick -size "${px}x${px}" xc:none "$WORK/inner-$px.png" \
        -gravity center -composite "$WORK/base-$px.png"
    magick "$WORK/base-$px.png" \
        \( +clone -background black -shadow "50x${sigma}+0+0" \) \
        -gravity center -geometry "+0+${off}" -compose DstOver -composite \
        -background none -gravity center -extent "${px}x${px}" \
        -define png:exclude-chunks=date,time "$out"
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

echo ""
echo "Done. Regenerated:"
echo "  $ICON_PKG/Assets/{lines,nodes}.svg"
echo "  $ICONSET/*.png"
echo "  $ICNS"
echo "  $BUNDLE_ICNS"

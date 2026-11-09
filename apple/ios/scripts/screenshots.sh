#!/usr/bin/env bash
# Take the documentation's iOS screenshots from the simulator.
#
# Usage (from apple/ios, after ./build.sh):
#   scripts/screenshots.sh            # light appearance
#   scripts/screenshots.sh --dark     # dark appearance (files get a -dark suffix)
#   scripts/screenshots.sh --no-uitest  # skip the UI-test shots (menus, search, swipes)
#
# Environment:
#   MBR_SIM_DEVICE   simulator name (default "iPhone 17"); an iPad adds -ipad
#   MBR_SHOTS_OUT    output folder (default docs/images/screens/ios)
#
# The app is reinstalled first, so its sample vault (the repository's docs/,
# bundled at build time) is fresh. The `screens` and `download` self-tests and
# the UI tests pause at each state and print `SCREEN <name>` / `SHOT <name>`;
# this script saves a `simctl io screenshot` at each marker it knows.
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$here/../.." && pwd)"
device="${MBR_SIM_DEVICE:-iPhone 17}"
out="${MBR_SHOTS_OUT:-$repo/docs/images/screens/ios}"
appearance=light
uitest=1
for arg in "$@"; do
    case "$arg" in
        --dark) appearance=dark ;;
        --no-uitest) uitest=0 ;;
        *) echo "unknown argument: $arg" >&2; exit 2 ;;
    esac
done

suffix=""
[[ $appearance == dark ]] && suffix="-dark"
[[ $device == iPad* ]] && suffix="$suffix-ipad"

# Apple's simctl/xcodebuild, never nix's.
export DEVELOPER_DIR="${MBR_XCODE_DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
unset SDKROOT

app="$repo/target/ios-derived-data/Build/Products/Debug-iphonesimulator/MBR.app"
bundle=com.zmre.mbr.ios
[[ -d $app ]] || { echo "no app at $app; run ./build.sh first" >&2; exit 1; }
mkdir -p "$out"

# Which marker becomes which file. Markers not listed are ignored.
declare -A names=(
    [page]=ios-page
    [list]=ios-vault-list
    [deck]=ios-flashcards
    [editor]=ios-editor
    [tasks]=ios-tasks
    [settings]=ios-settings
    [downloading]=ios-downloading
    [downloading-offline]=ios-downloading-offline
    [title-menu]=ios-title-menu
    [more-menu]=ios-more-menu
    [native-search]=ios-native-search
    [swipe-browse]=ios-swipe-browse
    [swipe-info]=ios-swipe-info
)

shot() {
    local marker=$1 file=${names[$1]:-}
    [[ -n $file ]] || return 0
    # Let the transition settle; the app holds each state for several seconds.
    # The offline variant lasts only a moment, so it is taken without waiting.
    [[ $marker == downloading-offline ]] || sleep 1
    xcrun simctl io "$device" screenshot --type=png "$out/$file$suffix.png" >/dev/null 2>&1 &&
        echo "saved $file$suffix.png"
}

echo "== $device ($appearance) -> $out"
xcrun simctl boot "$device" 2>/dev/null || true
xcrun simctl bootstatus "$device" -b >/dev/null
xcrun simctl ui "$device" appearance "$appearance"
# A tidy status bar: 9:41, full signal and battery.
xcrun simctl status_bar "$device" override --time "9:41" --dataNetwork wifi \
    --wifiBars 3 --cellularBars 4 --batteryState charged --batteryLevel 100 || true

xcrun simctl terminate "$device" "$bundle" 2>/dev/null || true
xcrun simctl uninstall "$device" "$bundle" 2>/dev/null || true
xcrun simctl install "$device" "$app"

# Run one self-test scenario, saving a shot at each SCREEN marker, until the
# scenario reports done (or a timeout).
selftest() {
    local mode=$1 timeout=$2
    echo "-- self-test $mode"
    local log
    log="$(mktemp)"
    xcrun simctl launch --console-pty --terminate-running-process "$device" "$bundle" \
        -mbrSelfTest "$mode" -showTimingBar NO >"$log" 2>&1 &
    local pid=$! seen=0 waited=0
    while (( waited < timeout )); do
        local lines
        lines=$(grep -c . "$log" || true)
        while (( seen < lines )); do
            seen=$((seen + 1))
            local line
            line=$(sed -n "${seen}p" "$log")
            if [[ $line =~ SCREEN\ ([a-z-]+) ]]; then
                # `SCREEN downloading` is followed at once by the offline
                # variant; the online page is shot when it comes back below.
                [[ ${BASH_REMATCH[1]} == downloading ]] || shot "${BASH_REMATCH[1]}"
            fi
            # Back online after the offline variant, the progress bar holds for
            # several seconds before the file arrives.
            if [[ $line == *"SELFTEST download.online"* ]]; then
                shot downloading
            fi
            if [[ $line == *"SELFTEST done"* || $line == *"done: $mode"* ]]; then
                waited=$timeout
            fi
        done
        sleep 0.5
        waited=$((waited + 1))
    done
    kill "$pid" 2>/dev/null || true
    xcrun simctl terminate "$device" "$bundle" 2>/dev/null || true
    rm -f "$log"
}

selftest screens 180
selftest download 120

if (( uitest )); then
    echo "-- UI tests"
    # build.sh uitest prints the test log; SHOT lines arrive on it.
    (cd "$here" && MBR_SIM_DEVICE="$device" ./build.sh uitest 2>&1) | while IFS= read -r line; do
        if [[ $line =~ SHOT\ ([a-z-]+) ]]; then
            shot "${BASH_REMATCH[1]}"
        fi
    done || true
fi

xcrun simctl status_bar "$device" clear || true
echo "done"

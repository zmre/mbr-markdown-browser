#!/usr/bin/env bash
# Build the MBR iOS app.
#
# Usage:
#   ./build.sh          - Rust staticlibs + bindings + xcframework + simulator app
#   ./build.sh run      - ...then boot a simulator, install and launch the app
#   ./build.sh device   - ...build for a device instead (signs with team
#                         X2SAY6ACE6; needs your Apple account in Xcode)
#   ./build.sh rust     - Rust staticlibs + bindings + xcframework only
#   ./build.sh uitest   - ...then run the UI tests (MBRMobileUITests) on the
#                         simulator; they print `SHOT <name>` at each state
#
# Environment:
#   MBR_SIM_DEVICE            simulator name for `run` (default: "iPhone 17")
#   MBR_RUST_PROFILE          cargo profile for the staticlibs (default: release)
#   MBR_XCODE_DEVELOPER_DIR   Xcode's Developer dir (default: /Applications/Xcode.app/...)
#   MBR_IOS_NO_SIGN           with `device`: compile and link, but do not sign
#
# Runs inside `nix develop` (re-execs itself there if the iOS rust targets are
# missing). The nix shell points DEVELOPER_DIR/SDKROOT at a nix *macOS* SDK, so
# everything iOS-side is routed to Xcode explicitly, while host-side build
# scripts and proc macros keep using the nix toolchain. Never export Xcode's
# DEVELOPER_DIR/SDKROOT globally: that breaks nix's host linker.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
MODE="${1:-build}"
XCODE_DEV="${MBR_XCODE_DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
PROFILE="${MBR_RUST_PROFILE:-release}"
TARGETS=(aarch64-apple-ios aarch64-apple-ios-sim)
LIB=libmbr_ffi.a

case "$MODE" in
    build | run | device | rust | uitest) ;;
    *)
        echo "usage: $0 [build|run|device|rust|uitest]" >&2
        exit 2
        ;;
esac

has_ios_targets() {
    local sysroot
    sysroot="$(rustc --print sysroot 2>/dev/null)" || return 1
    [[ -d "$sysroot/lib/rustlib/aarch64-apple-ios" && -d "$sysroot/lib/rustlib/aarch64-apple-ios-sim" ]]
}

if ! has_ios_targets; then
    if [[ -n "${MBR_IOS_BUILD_REEXEC:-}" ]]; then
        echo "error: iOS rust targets missing even inside nix develop (see rustToolchainDev in flake.nix)" >&2
        exit 1
    fi
    echo "iOS rust targets not in this shell; re-running inside nix develop..."
    export MBR_IOS_BUILD_REEXEC=1
    exec nix develop "$PROJECT_ROOT" -c "$0" "$@"
fi

cd "$PROJECT_ROOT"

# Cargo never sees the caller's DEVELOPER_DIR when it is Xcode's (apple/quicklook/
# build.sh has the long story): inside the nix shell the cc wrapper that links
# host build scripts carries its own SDK and refuses a conflicting
# DEVELOPER_DIR. The nix shell's own value is the one that wrapper expects, so
# only a value pointing at Xcode is dropped.
host_cargo() {
    if [[ "${DEVELOPER_DIR:-}" == /Applications/* ]]; then
        env -u DEVELOPER_DIR cargo "$@"
    else
        cargo "$@"
    fi
}

# The Rust library embeds the front-end bundle; it has to exist first.
if [[ ! -f crates/mbr-core/templates/components-js/mbr-components.min.js ]]; then
    echo "Building front-end components..."
    (cd components && bun install && bun run build)
fi

# A fixed path, not mktemp: the target linker's path is part of cargo's
# fingerprint, so a fresh directory per run would rebuild every crate.
SHIM_DIR="$PROJECT_ROOT/target/ios-shims"
mkdir -p "$SHIM_DIR"

# xcrun that always asks Xcode. cc-rs runs `xcrun --sdk iphoneos ...` to find
# the SDK, and nix's xcbuild xcrun only knows its own macOS SDK.
cat >"$SHIM_DIR/xcrun" <<EOF
#!/bin/sh
DEVELOPER_DIR="$XCODE_DEV" exec /usr/bin/xcrun "\$@"
EOF
chmod +x "$SHIM_DIR/xcrun"

# Runs cargo for one iOS target. Only the *target* side is redirected to
# Xcode; build scripts and proc macros still build for the host with nix's
# toolchain, so nothing here disturbs a desktop build's cache.
cargo_ios() {
    local target="$1"
    shift
    local sdk triple_env sdk_path
    case "$target" in
        *-sim) sdk=iphonesimulator ;;
        *) sdk=iphoneos ;;
    esac
    sdk_path="$(DEVELOPER_DIR="$XCODE_DEV" /usr/bin/xcrun --sdk "$sdk" --show-sdk-path)"
    triple_env="${target//-/_}"

    # Apple clang turns the shell's MACOSX_DEPLOYMENT_TARGET into
    # -mmacos-version-min, which clashes with the iOS target triple, and the
    # nix SDKROOT is a macOS SDK. The target compiler gets neither.
    cat >"$SHIM_DIR/clang-$target" <<EOF
#!/bin/sh
unset MACOSX_DEPLOYMENT_TARGET
export SDKROOT="$sdk_path"
DEVELOPER_DIR="$XCODE_DEV" exec /usr/bin/clang "\$@"
EOF
    chmod +x "$SHIM_DIR/clang-$target"

    (
        export PATH="$SHIM_DIR:$PATH"
        export "CC_${triple_env}=$SHIM_DIR/clang-$target"
        export "AR_${triple_env}=/usr/bin/ar"
        # Some dependencies (slug, via tera) declare a cdylib crate-type, so
        # rustc links for the target even though only the staticlib ships.
        export "CARGO_TARGET_$(echo "$triple_env" | tr '[:lower:]' '[:upper:]')_LINKER=$SHIM_DIR/clang-$target"
        export IPHONEOS_DEPLOYMENT_TARGET=26.0
        # The nix shell's RUSTFLAGS carry macOS -weak_framework link args
        # (QTKit, OpenGL, ...) that do not exist on iOS. With --target they
        # only ever applied to target artifacts, so host builds are unaffected.
        unset RUSTFLAGS
        host_cargo "$@" --target "$target"
    )
}

profile_dir() {
    if [[ "$PROFILE" == "dev" ]]; then echo debug; else echo "$PROFILE"; fi
}

for target in "${TARGETS[@]}"; do
    echo "Building Rust staticlib for $target ($PROFILE)..."
    start=$(date +%s)
    cargo_ios "$target" build --profile "$PROFILE" -p mbr-ffi --features mobile --lib
    echo "  $target: $(($(date +%s) - start))s, $(du -h "target/$target/$(profile_dir)/$LIB" | cut -f1)"
done

# Bindings from the device library, with the workspace root's generator (a
# host binary). Written to a scratch directory first and copied over only when
# they differ, so an unchanged API leaves the checked-in files (and Xcode's
# incremental build) alone.
echo "Generating Swift bindings..."
GENERATED="$SCRIPT_DIR/Generated"
SCRATCH="$PROJECT_ROOT/target/ios-bindings"
rm -rf "$SCRATCH"
host_cargo run --quiet --no-default-features --features ffi --bin uniffi-bindgen -- \
    generate --library "target/aarch64-apple-ios/$(profile_dir)/$LIB" \
    --language swift --out-dir "$SCRATCH"
mkdir -p "$GENERATED"
for file in mbr.swift mbrFFI.h mbrFFI.modulemap; do
    if ! cmp -s "$SCRATCH/$file" "$GENERATED/$file"; then
        cp "$SCRATCH/$file" "$GENERATED/$file"
        echo "  updated Generated/$file (commit it: git add -f apple/ios/Generated)"
    fi
done

echo "Creating MbrFFI.xcframework..."
HEADERS="$SCRIPT_DIR/build/headers"
rm -rf "$HEADERS" "$SCRIPT_DIR/MbrFFI.xcframework"
mkdir -p "$HEADERS"
cp "$GENERATED/mbrFFI.h" "$HEADERS/"
# Inside an xcframework the module map must be named module.modulemap.
cp "$GENERATED/mbrFFI.modulemap" "$HEADERS/module.modulemap"
DEVELOPER_DIR="$XCODE_DEV" /usr/bin/xcodebuild -create-xcframework \
    -library "target/aarch64-apple-ios/$(profile_dir)/$LIB" -headers "$HEADERS" \
    -library "target/aarch64-apple-ios-sim/$(profile_dir)/$LIB" -headers "$HEADERS" \
    -output "$SCRIPT_DIR/MbrFFI.xcframework" >/dev/null

[[ "$MODE" == "rust" ]] && exit 0

# The app's version is the workspace's.
VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)"

cd "$SCRIPT_DIR"
echo "Generating Xcode project..."
xcodegen generate --quiet

# xcodebuild with the nix shell scrubbed out of its environment: it must use
# Xcode's SDKs, clang and ld, never the nix wrappers earlier on PATH.
xcodebuild_clean() {
    env -i HOME="$HOME" USER="$USER" TMPDIR="${TMPDIR:-/tmp}" \
        PATH=/usr/bin:/bin:/usr/sbin:/sbin \
        DEVELOPER_DIR="$XCODE_DEV" \
        /usr/bin/xcodebuild "$@"
}

# A fixed derived-data directory per checkout, as apple/quicklook/build.sh
# does: Xcode's default is keyed by a hash of the project path, so every
# worktree gets its own and "the" build is whichever sorts first.
DERIVED="$PROJECT_ROOT/target/ios-derived-data"
if [[ "$MODE" == "device" ]]; then
    echo "Building for device..."
    # MBR_IOS_NO_SIGN=1 compiles and links without signing, for a machine
    # whose Xcode has no account for the team.
    signing=(-allowProvisioningUpdates)
    if [[ -n "${MBR_IOS_NO_SIGN:-}" ]]; then
        signing=(CODE_SIGNING_ALLOWED=NO)
    fi
    # MBR_IOS_DEVICE=<udid> (from `xcrun devicectl list devices`) builds for
    # that phone and lets Xcode register it with the team: a development
    # profile needs at least one registered device, and the generic
    # destination never registers one.
    destination='generic/platform=iOS'
    if [[ -n "${MBR_IOS_DEVICE:-}" ]]; then
        destination="id=$MBR_IOS_DEVICE"
        [[ -z "${MBR_IOS_NO_SIGN:-}" ]] && signing+=(-allowProvisioningDeviceRegistration)
    fi
    xcodebuild_clean -project MBRMobile.xcodeproj -scheme MBRMobile \
        -configuration Release -destination "$destination" \
        -derivedDataPath "$DERIVED" "${signing[@]}" \
        MARKETING_VERSION="$VERSION" build
    echo "App: $DERIVED/Build/Products/Release-iphoneos/MBR.app"
    echo "Install with: xcrun devicectl device install app --device <id> <that path>"
    exit 0
fi

if [[ "$MODE" == "uitest" ]]; then
    SIM="${MBR_SIM_DEVICE:-iPhone 17}"
    echo "Running the UI tests on $SIM..."
    xcodebuild_clean -project MBRMobile.xcodeproj -scheme MBRMobile \
        -configuration Debug -destination "platform=iOS Simulator,name=$SIM" \
        -derivedDataPath "$DERIVED" MARKETING_VERSION="$VERSION" test
    exit $?
fi

echo "Building for simulator..."
xcodebuild_clean -project MBRMobile.xcodeproj -scheme MBRMobile \
    -configuration Debug -destination 'generic/platform=iOS Simulator' \
    -derivedDataPath "$DERIVED" MARKETING_VERSION="$VERSION" build
APP="$DERIVED/Build/Products/Debug-iphonesimulator/MBR.app"
echo "App: $APP"

if [[ "$MODE" == "run" ]]; then
    SIM="${MBR_SIM_DEVICE:-iPhone 17}"
    simctl() { DEVELOPER_DIR="$XCODE_DEV" /usr/bin/xcrun simctl "$@"; }
    simctl boot "$SIM" 2>/dev/null || true
    simctl bootstatus "$SIM" -b >/dev/null
    simctl install "$SIM" "$APP"
    echo "Launching (Ctrl-C to stop; app output follows)..."
    simctl launch --console-pty --terminate-running-process "$SIM" com.zmre.mbr.ios
fi

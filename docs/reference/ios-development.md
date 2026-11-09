---
title: Building the iOS App
description: Build and run MBR for iPhone and iPad from source — simulator, your own device, tests and screenshots
order: 8
---

# Building the iOS App

The iOS and iPadOS app lives in `apple/ios/`. It is a SwiftUI shell around
mbr's Rust core: `crates/mbr-ffi` with its `mobile` feature, built as a static
library and linked through [UniFFI](https://mozilla.github.io/uniffi-rs/). The
web front end is the desktop's, unchanged. For using the app, see
[iPhone and iPad](../modes/ios/); for how it works inside, see
`apple/ios/README.md` and `design/ios-architecture-research.md`.

## Requirements

- **A Mac with Xcode** at `/Applications/Xcode.app` and an iOS simulator
  runtime. Install the runtime with `xcodebuild -downloadPlatform iOS`, or from
  Xcode › Settings › Components.
- **The repository's nix dev shell.** Its Rust toolchain carries the
  `aarch64-apple-ios` and `aarch64-apple-ios-sim` standard libraries.
  `build.sh` re-runs itself under `nix develop` when they are missing, so you
  can call it from any shell.
- **`xcodegen`**, which generates the Xcode project from `project.yml`. It is
  in the dev shell.

The app targets iOS and iPadOS 26.

## Quick start

```bash
cd apple/ios
./build.sh run
```

This builds everything, boots the "iPhone 17" simulator, installs the app and
launches it with its log in your terminal. The first build compiles the Rust
core twice (device and simulator) and takes several minutes; later builds are
incremental.

## build.sh

| Command | What it does |
|---------|--------------|
| `./build.sh` | Rust static libraries, Swift bindings, xcframework and the simulator app |
| `./build.sh run` | The same, then boot the simulator, install and launch |
| `./build.sh device` | A Release build for a phone, signed with the team in `project.yml` |
| `./build.sh rust` | Stop after the xcframework; then open `MBRMobile.xcodeproj` in Xcode |
| `./build.sh uitest` | The UI tests (`MBRMobileUITests`) on the simulator |

| Variable | Default | Effect |
|----------|---------|--------|
| `MBR_SIM_DEVICE` | `iPhone 17` | Simulator for `run` and `uitest` |
| `MBR_IOS_DEVICE` | — | With `device`: build for this phone (UDID) and register it with the team |
| `MBR_IOS_NO_SIGN` | — | With `device`: compile and link without signing (CI-style check) |
| `MBR_RUST_PROFILE` | `release` | Cargo profile for the static libraries |
| `MBR_XCODE_DEVELOPER_DIR` | `/Applications/Xcode.app/Contents/Developer` | Which Xcode to use |

## Running on your own iPhone or iPad

1. Turn on **Developer Mode** on the device (Settings › Privacy & Security),
   and sign in to Xcode with your Apple account.
2. **Using a fork?** `project.yml` signs with team `X2SAY6ACE6` and bundle id
   `com.zmre.mbr.ios`. Change `DEVELOPMENT_TEAM` and
   `PRODUCT_BUNDLE_IDENTIFIER` (and `bundleIdPrefix`) to your own.
3. Find the device's UDID and build for it. Naming the device registers it
   with the team; the first signed build otherwise fails with "Your team has
   no devices":

   ```bash
   xcrun devicectl list devices
   MBR_IOS_DEVICE=<udid> ./build.sh device
   ```

4. Install and launch:

   ```bash
   xcrun devicectl device install app --device <udid> \
     ../../target/ios-derived-data/Build/Products/Release-iphoneos/MBR.app
   xcrun devicectl device process launch --device <udid> com.zmre.mbr.ios
   ```

   Or run `./build.sh rust`, open `MBRMobile.xcodeproj` in Xcode, pick the
   device and press Run.

5. **Logs:** in Console.app, select the device and filter on the subsystem
   `com.zmre.mbr.ios`. `apple/ios/TESTING.md` lists the lines worth reading
   and the full device checklist.

## What the build does

1. `cargo build -p mbr-ffi --features mobile --lib` for `aarch64-apple-ios`
   and `aarch64-apple-ios-sim`, giving `target/<triple>/release/libmbr_ffi.a`.
2. Swift bindings generated from the device library by the workspace's
   `uniffi-bindgen` binary, into `apple/ios/Generated/`. A file there is
   rewritten only when its content changed, and the script says so.
3. `MbrFFI.xcframework` from the two libraries.
4. `xcodegen` writes `MBRMobile.xcodeproj` from `project.yml`.
5. `xcodebuild`, with derived data in `target/ios-derived-data/`.

The project, the xcframework and the generated `Info.plist` are git-ignored.
**`Generated/` is checked in**: commit it when it changes. A common global
gitignore pattern, `*.sw*` (for vim swap files), also matches `.swift`, so add
it with:

```bash
git add -f apple/ios/Generated
```

## Checking the Rust side only

```bash
cargo check -p mbr-ffi --features mobile --lib --target aarch64-apple-ios-sim
cargo test -p mbr-ffi --features mobile
cargo clippy -p mbr-ffi --features mobile --all-targets -- -D warnings
```

The `mobile` feature compiles and is tested on every platform, so the tests
and clippy run on the host. Clippy for the iOS targets themselves is not
available in the dev shell.

## Why the Xcode shims

The nix dev shell points `DEVELOPER_DIR` and `SDKROOT` at a nix macOS SDK,
which cannot build for iOS. `build.sh` therefore routes only the *target* side
of the Rust build to Xcode — an `xcrun` shim, a target `clang` wrapper used as
the C compiler and linker, `RUSTFLAGS` unset — while build scripts and
procedural macros keep the nix toolchain. Each workaround is commented in the
script.

> [!WARNING]
> Never export Xcode's `DEVELOPER_DIR` or `SDKROOT` globally in the dev shell.
> It breaks nix's host linker for build scripts, with errors that point nowhere
> near the cause.

## Testing

**Simulator self-tests.** Launch the app with `-mbrSelfTest <mode>` and it runs
a scenario on its own sample vault, printing `SELFTEST <name>: …` lines:

```bash
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
APP=../../target/ios-derived-data/Build/Products/Debug-iphonesimulator/MBR.app
xcrun simctl install "iPhone 17" "$APP"
xcrun simctl launch --console-pty --terminate-running-process \
  "iPhone 17" com.zmre.mbr.ios -mbrSelfTest full
```

| Mode | Checks |
|------|--------|
| `full` | Rendering, chunks, search, byte ranges, a task toggle and a flashcard review written to disk, video seek, redirects |
| `single` | A single file opened from Files, then Grant Folder Access |
| `switch` | A document handed to the app while a vault is open |
| `config` | A broken `.mbr/config.toml` opens with the defaults and a notice |
| `refresh` | Edits made while the app is in the background are picked up on return |
| `download` | The Downloading page, offline, and the file arriving |
| `mbr-reload` | `.mbr/user.css` and `config.toml` edits picked up on refresh |
| `screens` | Steps through the main screens for screenshots |
| `deep` | A deep page with a long title; used by the UI tests |

The full table, with every check, is in `apple/ios/README.md`.

**UI tests.** `./build.sh uitest` runs `MBRMobileUITests`: the bar's menus,
New Note, native search, the edge swipes and the home-screen icon.

**On a device.** `apple/ios/TESTING.md` is the checklist for what a simulator
cannot show: real cloud providers, files that are not downloaded, a phone's
speed and memory.

**Swift style.** `nix build .#swiftfmt .#swiftlint-check` checks this app's
Swift (and QuickLook's).

## Screenshots

The documentation's iOS screenshots come from the simulator, using the app's
own sample vault, so no personal notes appear. Both the UI tests and the
`screens` self-test pause at each state and print a marker (`SHOT <name>` or
`SCREEN <name>`). `apple/ios/scripts/screenshots.sh` watches for those markers
and saves a `simctl io screenshot` at each:

```bash
cd apple/ios
./build.sh                          # once
scripts/screenshots.sh              # iPhone 17, light
scripts/screenshots.sh --dark       # dark appearance
MBR_SIM_DEVICE="iPad Pro 13-inch (M5)" scripts/screenshots.sh
```

The images land in `docs/images/screens/ios/`.

## Troubleshooting

| Problem | Fix |
|---------|-----|
| `Unable to find a destination` / no simulator | Install a runtime: `xcodebuild -downloadPlatform iOS` |
| "Your team has no devices" | Build once with `MBR_IOS_DEVICE=<udid>` so Xcode registers the phone |
| Signing errors on a fork | Set your own `DEVELOPMENT_TEAM` and bundle ids in `project.yml` |
| Linker errors in build scripts (`ld: library not found`) | Something exported Xcode's `DEVELOPER_DIR`/`SDKROOT` globally; start a fresh `nix develop` |
| Swift errors about missing UniFFI symbols | `Generated/` is stale: rerun `./build.sh` and commit the regenerated files |
| New Swift files never get committed | A global gitignore `*.sw*` pattern; `git add -f` them |

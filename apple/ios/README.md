# MBR for iOS and iPadOS

A SwiftUI shell around mbr's Rust core. The app links `crates/mbr-ffi` (the
`mobile` feature) as a static-library xcframework through UniFFI. It renders in
the iOS 26 SwiftUI `WebView`/`WebPage`, and serves every `mbr://localhost/...`
request in-process: a `URLSchemeHandler` hands each one to `MbrApp.handle`,
which runs it through mbr's own axum router. There is no socket and no port.
The front end is the desktop's, unchanged.

Architecture and the reasoning behind it are in `design/ios-architecture-research.md`
at the repository root. The device checklist is in [TESTING.md](TESTING.md).

## Build

Requirements:

- Xcode at `/Applications/Xcode.app` with an iOS simulator runtime
  (`xcodebuild -downloadPlatform iOS`).
- The repository's nix dev shell. The flake's `rustToolchainDev` carries the
  `aarch64-apple-ios` and `aarch64-apple-ios-sim` std, and `build.sh`
  re-execs itself under `nix develop` if they are missing.
- `xcodegen` (in the dev shell).

```bash
cd apple/ios
./build.sh          # staticlibs (release) + bindings + xcframework + simulator app
./build.sh run      # ...then boot "iPhone 17", install, and launch with console output
./build.sh device   # Release build for a device (signs with team QR9P58E333)
./build.sh rust     # Rust side only (then open MBRMobile.xcodeproj in Xcode)
MBR_IOS_NO_SIGN=1 ./build.sh device   # device build that compiles and links, unsigned
```

The Rust side is `cargo build -p mbr-ffi --features mobile --lib` for both iOS
targets, giving `target/<triple>/release/libmbr_ffi.a` (about 220 MB per arch
before dead-code stripping). The Swift bindings are generated from the device
library with the workspace's `uniffi-bindgen` binary into `Generated/`, which is
checked in. `build.sh` rewrites a file there only when its content changed, and
says so; commit it with `git add -f apple/ios/Generated` (the global gitignore's
`*.sw*` matches `.swift`).

`build.sh` sends only the *target* side of the Rust build to Xcode. Host build
scripts and proc macros keep the nix toolchain, so the desktop build cache is
untouched. Each workaround is commented in the script and listed in the research
doc §12.2:

- an `xcrun` shim (cc-rs asks `xcrun` for the iOS SDK);
- a target-clang wrapper that unsets `MACOSX_DEPLOYMENT_TARGET` and sets
  `SDKROOT`, used as `CC_<triple>` and `CARGO_TARGET_<TRIPLE>_LINKER`;
- `RUSTFLAGS` unset;
- a fixed shim directory, `target/ios-shims/`;
- `ARCHS: arm64` in `project.yml`.

Cargo never sees Xcode's `DEVELOPER_DIR`, as in `apple/quicklook/build.sh`.
`xcodebuild` runs with the nix shell scrubbed from its environment, and with a
fixed derived-data path, `target/ios-derived-data/`.

The iOS build is deliberately not a nix derivation, since it needs Xcode's iOS
SDKs. `nix build .#swiftfmt .#swiftlint-check` covers this app's Swift alongside
QuickLook's.

Build outputs (`MBRMobile.xcodeproj/`, `MbrFFI.xcframework/`, `build/`, and the
`MBRMobile/Info.plist` XcodeGen writes) are git-ignored.

## Layout

| Path | What |
|------|------|
| `project.yml` | XcodeGen project: iOS 26, team QR9P58E333, `com.zmre.mbr.ios`, a markdown document type for opening files in place, and the repository's `docs/` bundled as the sample vault |
| `MBRMobile/MBRMobileApp.swift` | App entry. Scene-phase hooks (refresh on foreground, flush on background), `onOpenURL` for documents, and the web view prewarm |
| `MBRMobile/Model/AppModel.swift` | Navigation, the open session (one at a time), lifecycle, and the dataless-files toggle |
| `MBRMobile/Model/VaultStore.swift` | Vault list in `UserDefaults`: add, rename, remove, re-pick. Plain `bookmarkData()`, resolved off the main thread and re-saved when stale. `FolderAccess` holds the security scope; `AppPaths` has the per-vault cache folders |
| `MBRMobile/Model/VaultSession.swift` | One open vault or file: `MbrApp.open`/`openFile`, timings, `refresh()` with reload, `flushIndex()`, WebContent crash recovery, and the Downloading-page watchdog |
| `MBRMobile/Web/MbrSchemeHandler.swift` | `mbr://` handler. Pull-based: each `next()` is one `MbrResponse.nextChunk()`, so media streams with Rust's bounded buffer. Logs one line per request |
| `MBRMobile/Web/WebPagePool.swift` | The `WebPage` setup (scheme handler, link policy, JS dialogs) and the prewarmed page |
| `MBRMobile/Support/FileCoordinatorBridge.swift` | mbr's `FileCoordinator`: `NSFileCoordinator` writes (`.forReplacing`, item-replacement directory plus `replaceItemAt`, with an in-place fallback), mkdir, move, remove, and download requests |
| `MBRMobile/Support/Log.swift` | Unified log (subsystem `com.zmre.mbr.ios`) plus stderr |
| `MBRMobile/Views/` | Vault list, vault screen (web view, config notice, toolbar), settings, onboarding |
| `MBRMobile/SelfTest*.swift` | Simulator self-test (below) |

## How the app uses the mobile API

| API | Where |
|-----|-------|
| `MbrApp.open(rootPath:cacheDir:vaultId:coordinator:)` | `VaultSession.open`. `cacheDir` = `Caches/vaults/<vault id>/`, so removing a vault deletes its snapshot. `vaultId` = the vault record's UUID, created with its bookmark and kept across bookmark refreshes and re-picks |
| `MbrApp.openFile(filePath:coordinator:)` | `VaultSession.openFile`, for documents from Files or the share sheet |
| `homeUrl()`, `isSingleFile()`, `rootPath()` | The first page loaded; "Grant Folder Access" |
| `configError()` | The "Configuration ignored" notice above the page |
| `handle(...)` → `MbrResponse` (`status`, `headers`, `nextChunk`) | `MbrReplyIterator` |
| `waitUntilReady()` → `OpenReport` | The `ready:` log line and the timings in the subtitle |
| `refresh()` → `RefreshSummary` | Foreground and the Refresh menu item. The page reloads when it is in `changedPages` or anything was added or removed |
| `flushIndex()` | Background, inside `beginBackgroundTask` |
| `indexCachePath()` | Settings and the `ready:` line |
| `setMaterializeDatalessFiles(_:)` | Settings → "Download files on read" (off by default, matching Rust) |
| `FileCoordinator` | `FileCoordinatorBridge`, passed to every open |

Pages must be loaded from `mbr://localhost/`. Rust derives `Host` from the URL
authority, and the edit endpoints require `localhost`.

## Simulator self-test

Launch with `-mbrSelfTest <mode>`. Results are `SELFTEST <name>: ...` lines on
stderr and in the unified log.

```bash
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer   # Apple's xcrun, not nix's
APP=../../target/ios-derived-data/Build/Products/Debug-iphonesimulator/MBR.app
xcrun simctl install "iPhone 17" "$APP"
xcrun simctl launch --console-pty --terminate-running-process "iPhone 17" com.zmre.mbr.ios -mbrSelfTest full
```

| Mode | Checks |
|------|--------|
| `full` | On the sample vault, with fixtures written to `selftest/`: open timings and `fromCache`, viewport/desktop layout, localStorage across launches, components, no live-reload socket, mermaid, lazy `import()`, `POST /.mbr/search`, `Range: bytes=0-99` → 206 + 100 bytes, a task toggle and a flashcard review through the real UI and the `FileCoordinator` (each verified on disk), `<video>` seek, and a slashless URL landing on its canonical page |
| `single` | A file from `simctl openurl file://…` (or `-mbrOpenFile <path under Documents>`): it renders, `/` lands on it, and Grant Folder Access is offered |
| `config` | A vault with a broken `.mbr/config.toml` opens, reports the error, and `errors.json` carries `config_parse_error` |
| `refresh` | Opens the sample vault and waits. Background the app, edit a file in its container, then bring it back: `refresh (foreground)` reports the change and the page reloads |
| `download` | A legacy `.pending.md.icloud` placeholder: the 503 Downloading page, whether WebKit honours `Refresh`, and the page once the real file appears |
| `screens` | Steps through page, search, deck, list and settings, writing `SCREEN <name>` to stderr for screenshots |

Run twice: the second `full` launch should report `localStorage: previous launch …`
and `open: fromCache=true`.

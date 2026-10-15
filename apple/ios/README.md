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
./build.sh device   # Release build for a device (signs with team X2SAY6ACE6)
./build.sh rust     # Rust side only (then open MBRMobile.xcodeproj in Xcode)
MBR_IOS_DEVICE=<udid> ./build.sh device   # build for that phone and register it with the team (needed once)
MBR_IOS_NO_SIGN=1 ./build.sh device   # device build that compiles and links, unsigned
./build.sh uitest   # the UI tests (MBRMobileUITests) on MBR_SIM_DEVICE (default "iPhone 17")
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
| `project.yml` | XcodeGen project: iOS 26, team X2SAY6ACE6, `com.zmre.mbr.ios`, a markdown document type for opening files in place, and the repository's `docs/` bundled as the sample vault |
| `MBRMobile/MBRMobileApp.swift` | App entry. Scene-phase hooks (refresh on foreground, flush on background), `onOpenURL` for documents, and the web view prewarm |
| `MBRMobile/Model/AppModel.swift` | Navigation, the open session (one at a time), lifecycle, and the dataless-files toggle |
| `MBRMobile/Model/VaultStore.swift` | Vault list in `UserDefaults`: add, rename, remove, re-pick. Plain `bookmarkData()`, resolved off the main thread and re-saved when stale. `FolderAccess` holds the security scope; `AppPaths` has the per-vault cache folders |
| `MBRMobile/Model/VaultSession.swift` | One open vault or file: `MbrApp.open`/`openFile`, timings, `refresh()` with reload, `flushIndex()`, WebContent crash recovery, and the Downloading-page watchdog |
| `MBRMobile/Web/MbrSchemeHandler.swift` | `mbr://` handler. Pull-based: each `next()` is one `MbrResponse.nextChunk()`, so media streams with Rust's bounded buffer. Logs one line per request |
| `MBRMobile/Web/WebPagePool.swift` | The `WebPage` setup (scheme handler, link policy, JS dialogs, the app bridge) and the prewarmed page |
| `MBRMobile/Web/AppBridge.swift` | The native half of the page ↔ app bridge: the document-start user script that marks pages embedded, the `mbrApp` script-message handler feeding `PageInfo` (title, breadcrumbs, actions), and `AppBridge.open` calling `window.mbrApp.open(name)` |
| `MBRMobile/Views/VaultToolbar.swift` | The native bar: history Back (long press: recent pages, then Vaults), the ⋯ menu (the `+` beside it is in `VaultScreen.swift`) (the page's actions, then Refresh, Home, Settings, Open Other Vault…), and the breadcrumb title menu |
| `MBRMobile/Assets.xcassets` | The app icon: one opaque 1024×1024 PNG, the macOS icon's tree cropped full-bleed |
| `MBRMobileUITests/` | UI tests for what only a tap opens (the bar's menus, the home-screen icon); each state prints `SHOT <name>` and is kept as an attachment |
| `MBRMobile/Support/FileCoordinatorBridge.swift` | mbr's `FileCoordinator`: `NSFileCoordinator` writes (`.forReplacing`, item-replacement directory plus `replaceItemAt`, with an in-place fallback), mkdir, move, remove, and download requests |
| `MBRMobile/Support/Log.swift` | Unified log (subsystem `com.zmre.mbr.ios`) plus stderr |
| `MBRMobile/Views/` | Vault list, vault screen (web view, config notice, the native bar), settings, onboarding |
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
| `refresh()` → `RefreshSummary` | Foreground and the Refresh menu item. The page reloads when it is in `changedPages`, when anything was added or removed, or when `.mbr/` changed (`configReloaded`, `templatesReloaded`, `assetsChanged`). `configError` updates the notice |
| `flushIndex()` | Background, inside `beginBackgroundTask` |
| `indexCachePath()` | Settings and the `ready:` line |
| `setMaterializeDatalessFiles(_:)` | Settings → "Download files on read" (off by default, matching Rust) |
| `FileCoordinator` | `FileCoordinatorBridge`, passed to every open |

Pages must be loaded from `mbr://localhost/`. Rust derives `Host` from the URL
authority, and the edit endpoints require `localhost`.

## Refresh and `.mbr/`

There is no file watcher on iOS. Foregrounding the app (and ⋯ → Refresh)
re-lists the vault, and also looks at the vault's `.mbr/` folder. The check is
stat-only: nothing is read unless something changed.

| Change | Result |
|--------|--------|
| `config.toml` | The vault reopens with it inside the app, keeping its index. A file that no longer parses gives the defaults plus the "Configuration ignored" notice, and fixing it clears the notice |
| a template (`*.html`) | The templates reload in place |
| `theme.css`, `user.css`, `components/*.js`, … | Nothing to reload: they are read on every request |

In every case the page on screen reloads.

**`.mbr/` files still in the cloud.** The scan skips hidden folders, so nothing
would ever ask iCloud (or another provider) for a `.mbr/` file it has not
downloaded. Opening the vault asks for every such file (`OpenReport
.mbrDownloadsRequested`), and so does each refresh until they are all here
(`RefreshSummary.mbrDownloadsRequested`). When the open asked for any, the app
refreshes again after 3, 6, 12, 30 and 60 s, stopping as soon as none are
outstanding; their arrival counts as a change, so the page reloads with them.
Meanwhile a page's request for such an asset (`/.mbr/user.css`, an SVG it
references) waits up to 3 s for it and otherwise answers `503` with
`Retry-After`, never a 500.

## The native bar and the bridge

In the app, mbr's own web header and breadcrumbs are hidden and the iOS
navigation bar takes over:

- **Title:** the page's title. Its menu (tap the title) is the breadcrumb path.
- **Back:** web history. At the vault's first page it returns to the vault
  list. A long press lists recent pages, then Vaults.
- **`+` (New Note):** beside ⋯, in the same Liquid Glass capsule (one
  `ToolbarItem` holding both: a `ToolbarItemGroup` of two folds into a single
  overflow ⋯ on an iPhone beside a long title). Opens the editor's new-note picker, defaulting to the
  current folder; the note is created and opened in the editor. Shown where
  editing is on, on listings too, and never for a file opened in place (its
  Grant Folder Access… gets a capsule of its own).
- **⋯ menu:** the page's own actions, then the app's (Refresh, Home, Settings,
  Open Other Vault…). The page's actions are Browse, Search, Edit, Info,
  Tasks, Flashcards and Slides, each listed only where it works: Edit with
  editing on, Tasks with the task browser enabled, Flashcards and Slides only
  on a deck.
- **Search (⋯ → Search):** a native sheet, not the page's modal (see
  [Native search](#native-search)).
- **While the editor is open** the bar is hidden and the back/forward swipes
  are off: the editor has its own Cancel/Save header, and Back or a stray
  edge swipe would only throw away unsaved edits.

The page's actions:

| Action | Element and method | Offered when |
|--------|--------------------|--------------|
| `browse` | `<mbr-browse>` / `<mbr-browse-single>` `open()` | always |
| `search` | `<mbr-search>` `open()` (the ⋯ menu opens the native sheet instead) | always |
| `edit` | `<mbr-editor>` `open()` | `canEdit` (editing on, a source file) |
| `info` | `<mbr-info>` `open()` | always |
| `tasks` | `<mbr-tasks>` `open()` | the task browser is enabled |
| `flashcards` | `<mbr-flashcards>` `open()` | a deck page |
| `slides` | `<mbr-slides>` `open()` | a slide document |
| `new` | `<mbr-editor>` `openNew()` | `canCreate` (editing on); the `+` button, not a ⋯ item |

No edge-swipe gestures for Browse or Info: WebKit's back/forward swipes own
both edges (a left-edge swipe is the main way back), a page's touch handlers
cannot win against the system gesture, and they would fight reveal.js swipes,
the flashcard deck's pointer handling, horizontally scrolling tables and text
selection. Browse and Info stay one tap away in ⋯.

The two halves:

| Direction | How |
|-----------|-----|
| App → page, at document start | A `WKUserScript` sets `window.__MBR_EMBEDDED__ = true` and `html.mbr-embedded`. theme.css then collapses the header (to zero height, not `display: none`, because its components host the overlays) and hides the breadcrumbs. Nothing server-side changes, so desktop, server and static pages never see it |
| Page → app, on every load | `components/src/app-bridge.ts` posts `{type: 'page', url, title, breadcrumbs, actions, editorOpen}` to the `mbrApp` script-message handler, which fills `PageInfo` |
| Page → app, editor opens/closes | `{type: 'editor', open}`, from `<mbr-editor>`'s `mbr-editor-state` event; sets `PageInfo.editorOpen` (every page message sets it again, so a new page starts with the bar back) |
| App → page, on a menu tap | `window.mbrApp.open('<action>')` calls the element's public `open()` (`openNew()` for `new`), the same path as its button and shortcut |

The action names and the message types (`"page"`, `"editor"`) are Swift
string literals (`AppAction` raw values, `PageInfo.apply`).
`app-bridge.test.ts` reads `AppBridge.swift` and checks every one, so a rename
on either side fails a test.

## Native search

⋯ → Search presents `Views/SearchSheet.swift`, a SwiftUI sheet with a
`.searchable` field activated as it appears, so the keyboard comes up with it.
The page's `<mbr-search>` modal cannot do that from the menu: WebKit raises the
software keyboard for a script `focus()` only while the user is touching the
web view, and a choice in a native menu is not that (the modal opened with a
focused input and no keyboard). The page still lists `search` as an action, and
its own button and `/` shortcut still open the web modal.

`Model/VaultSearch.swift` sends `POST /.mbr/search` through `MbrApp.handle`
(in process, like the scheme handler) with the web modal's defaults: `scope:
all` (titles, tags and content), the whole vault, markdown only, 20 results,
nothing under two characters. Each keystroke restarts a `.task(id: query)`,
which is the 200 ms debounce and the stale-answer guard in one: a cancelled
run changes nothing. Rows show the title, the folder and the content snippet
(else the description); a footnote reports `not_downloaded` notes content search
skipped, a scan still in progress, and "Showing N of M". Tapping a row (or
Return, for the first) closes the sheet and loads the page. While the field is
active iOS swaps the sheet's bar (and its Close) for the field and a system X
that only deactivates it; the sheet dismisses itself when that happens, so the
X closes it. Snippets drop grep's `Line N: ` prefix.

Known gaps: no type facets, folder picker, "current folder only" or file-type
toggle (the web modal has them); `key:value` facets typed into the field work.

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
| `download` | A legacy `.pending.md.icloud` placeholder: the Downloading page (a 503 underneath, drawn as a progress bar and the note's name), whether WebKit honours `Refresh`, and the page once the real file appears |
| `screens` | Steps through page, search, deck, the editor and the tasks overlay (both opened through the bridge), list and settings, writing `SCREEN <name>` to stderr for screenshots |
| `mbr-reload` | Opens the sample vault and waits. Background the app, edit `.mbr/user.css` and `.mbr/config.toml` in its container, then bring it back: the refresh reports a config reload and the page shows both changes (`mbr-reload.after: PASS …`) |
| `deep` | Writes and opens a page four folders down with a long title, and logs what the native bar got (`deep.bar`) and that the web header is collapsed (`deep.page`). The UI tests use it |

Run twice: the second `full` launch should report `localStorage: previous launch …`
and `open: fromCache=true`.

`./build.sh uitest` runs `MBRMobileUITests`. It launches `deep`, opens the title,
⋯ and Back menus, taps `+` (New Note), then goes to the home screen and finds the icon.
`testNativeSearchFromMenu` picks ⋯ → Search and checks the native search field
is there with the keyboard already up, types `flashcards`, waits for the
Flashcards result, taps it and checks the bar's title changed. At each state the tests print
`SHOT <name>` and pause. A script watching the output can take `xcrun simctl io <device> screenshot`
then; the same states are kept as attachments in the `.xcresult`.

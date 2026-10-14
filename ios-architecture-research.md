# mbr on iOS — Architecture Research & Options

> **Note (2026-10-14):** the spike code this document describes (§12, the
> spike's `src/mobile.rs` and the throwaway `apple/ios` app) was superseded by
> the workspace split, the `Vault` seam and the mobile core in PRs #334–#341,
> and by the real app in `apple/ios/` (see its README and TESTING.md). The
> research and decisions stand; §12.6 adds what the app's simulator self-test
> found. Device results go in §12.4.

*Date: 2026-10-07, updated 2026-10-08 after the PR0 spike (§12) · Status: reviewed; decisions recorded · Minimum OS: iOS/iPadOS 26*


---

## 0. Executive summary

**TL;DR** — Build a **thin SwiftUI shell** that links mbr's Rust core as an
**xcframework via UniFFI** (the path the QuickLook extension already takes). The
shell renders in **one WKWebView** whose requests go to a **custom URL scheme
handler** (`mbr://`), and that handler calls mbr's existing axum `Router`
**in-process** with no sockets. The front end (mermaid, KaTeX, Lit components,
flashcards, graphs) runs unchanged, because it sees the same URLs it sees today.
The one structural change mbr itself needs is a **storage abstraction (a "Vault"
trait)** under `repo`/`path_resolver`/`search`/the write handlers. That trait is
the seam for four things at once: iOS cloud-file quirks, Android's
`content://` URIs, your future encrypted-sync library, and tests. Split the
code into a Cargo workspace inside the mbr repo (`mbr-core`, `mbr-server`,
`mbr-ffi`, …), and keep the iOS app **in the same repo** under `apple/ios/`
(decided; see §6).

The hard part is **files**, not rendering. iCloud and Dropbox folders work;
**Google Drive, OneDrive and Sync.com cannot grant folder access on iOS**
(Sync.com confirmed by a device test in iA Writer). Cloud files may be
"dataless" (not downloaded), and a naïve `read()` stalls on them.

**The PR0 spike (§12) proved the stack on the iOS simulator.** The whole
crate compiles for `aarch64-apple-ios(-sim)` with no source changes. Pages,
mermaid, lazy `import()` chunks, JSON `POST /.mbr/search` and a real
`POST /.mbr/task` write all work over `mbr://`, and localStorage persists
across launches. It found two transport quirks: WebKit ignores a 301 from a
scheme handler, and `Blob` request bodies arrive empty.

| Decision                 | Recommendation                                                     | Confidence  |
|--------------------------|--------------------------------------------------------------------|-------------|
| App shell                | SwiftUI (native), not Tauri/wry/Capacitor                          | High        |
| Rust ↔ Swift             | UniFFI → xcframework (already used for QuickLook)                  | High        |
| Rendering                | WKWebView, unchanged front end                                     | High        |
| WebView ↔ Rust transport | SwiftUI `WebPage` + `URLSchemeHandler` → axum `Router::oneshot`; localhost fallback | High (spike §12) |
| File access              | Folder picker + bookmark → real POSIX path → Rust `std::fs`        | High        |
| Core seam                | `Vault` storage trait (bulk-oriented, not per-file FFI)            | High        |
| Code layout              | **Decided:** workspace in the mbr repo, app in the same repo at `apple/ios/` | Decided |
| Minimum OS               | **Decided:** iOS/iPadOS 26 (SwiftUI `WebView`/`WebPage`, async `URLSchemeHandler`, `BGContinuedProcessingTask`) | Decided |
| Persistent index         | Allowed in the app's sandboxed `Caches/` (you approved this)       | —           |
| Android                  | Same core; Kotlin shell; SAF `Vault`; localhost transport          | Medium      |

---

## 1. Requirements & constraints

**TL;DR** — Raw markdown files, any provider, offline, fast, the full front-end
feature set, and no mess in the core codebase.

From you:

| #  | Requirement                                                                        | Source        |
|----|------------------------------------------------------------------------------------|---------------|
| R1 | Work on **raw markdown files**, no import/conversion                              | stated        |
| R2 | Open folders on **iCloud Drive and other File Providers** (Sync.com in particular) | stated        |
| R3 | **Open single `.md` files from anywhere** (Files, share sheet)                     | stated, v1    |
| R4 | Keep **all front-end features** (mermaid, KaTeX, graphs, flashcards, …) → browser engine | stated  |
| R5 | **Speed**: instant render/navigation; large repos (~3k notes / 4.8k files in `~/Notes`) | CLAUDE.md |
| R6 | **No pollution**: core stays maintainable, and mobile concerns don't leak everywhere | stated     |
| R7 | v1 = read + navigate + search + **light writes** (task toggle, FSRS review, review notes) | answer |
| R8 | **Offline always**; **iPad parity**                                                | answer        |
| R9 | Android **eventually**: don't paint into a corner                                  | answer        |
| R10| Future **encrypted sync arrives as a linked library**, not as plain files on disk  | answer        |
| R11| Personal/TestFlight first, App Store later                                         | answer        |
| R12| Persistent index allowed in the app's sandboxed `Caches/` (never in the notes folder) | answer     |

R10 matters most structurally. If notes can live behind a library API rather
than a path, mbr's core **must** stop assuming `std::fs` sooner or later. iOS,
Android and the sync library all point to the same seam.

---

## 2. What mbr looks like today (relevant facts)

**TL;DR** — The render pipeline is already pure once you have a string. Every
"repo-shaped" component (scan, URL resolution, search, backlinks, writes) is
welded to `std::path` + `canonicalize`. There is **no filesystem abstraction
anywhere**, and the whole server is compiled into every build, even the
QuickLook staticlib.

### 2.1 Filesystem coupling by module

| Class                       | Modules                                                                                                                                                                          | Port impact                                  |
|-----------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|----------------------------------------------|
| **Pure** (0 FS)             | `html`, `attrs`, `tasks`, `task_query`, `wikilink(_index)`, `tag_index`, `relationships`, `link_index`, `link_transform`, `flashcards`, `contact`, `chat`, `sorting`, `page_context`, `page_errors`, `vid`, `media`, `embedded_*` | none                                         |
| **FS at edges**             | `markdown` (5 `read_to_string` entry points; pipeline is `&str` after), `templates` (Tera glob, built-in fallback), `config` (`find_root_dir`, figment file), `task_index` (1 open), `search` (content half) | small: string-in entry points                 |
| **FS deep**                 | `server` (84 hits / 7.8k prod lines), `build` (71), `repo` (26: WalkDir scan, mtimes, canonical root), `path_resolver` (22: `is_file`/`is_dir` probes, `safe_join` via `canonicalize`), `link_grep`, `link_rewrite`, `watcher` (notify) | needs the `Vault` seam                       |
| **Desktop/feature only**    | `browser`, `external_open`, `open_picker`, `macos_open`, `video_*`, `pdf_metadata` (ffmpeg/pdfium under `media-metadata`)                                                       | excluded from mobile                         |

### 2.2 Things that already help

- **UniFFI + staticlib exist.** `Cargo.toml:27-29` (`["lib","staticlib"]`),
  `ffi` feature, `src/mbr.udl`, the `quicklook/` XcodeGen project and the Nix
  `mbr-quicklook-staticlib` derivation (`flake.nix:637-714`). The Swift↔Rust
  toolchain is proven, though only for `arm64-apple-macos`.
- **The router is a value.** `Server::init` → `Server { router, … }`
  (`server.rs:1729`, `:1029`). Binding happens only in `start*`, so
  `router.oneshot(req)` works in-process today.
- **The front end is self-contained and offline.** mermaid (5.5 MB), KaTeX,
  hljs (17 langs) and reveal are all embedded. Only rare hljs languages fall
  back to a CDN (fail-soft).
- **The front end is viewport-aware.** Viewport meta is present; the sidebar
  becomes a drawer under 1024px; info/review panels go full-width; flashcards
  are fully tappable; footnote and graph hover cards turn off on coarse
  pointers.
- **Static builds use relative URLs**, so they work under any subpath.

### 2.3 Tangles that block a clean split (all small)

| Tangle                                                                  | Where                                   | Fix                                       |
|-------------------------------------------------------------------------|-----------------------------------------|-------------------------------------------|
| `errors.rs` pulls `axum::http::Error`, `notify::Error`                  | `errors.rs:55`, `:289`, `:295`          | split per-crate error enums               |
| `page_context` imports `build::relative_*` and `server::Breadcrumb`/`generate_breadcrumbs` | `page_context.rs:24,30` | move these lexical helpers into core       |
| Asset table `DEFAULT_FILES` lives in `server.rs`; used by `build`, `quicklook` | `server.rs:7498`                  | new `assets` module                       |
| `ChangeEventType` lives in `watcher` (notify)                           | `repo.rs:1267`, `task_index.rs:37`      | move enum to core                         |
| `launch_url` imports `server::MediaViewerType`                          | `launch_url.rs:13`                      | move enum                                 |
| Duplicated async/sync render pipelines with 12 positional args           | `markdown.rs:1733`, `:2302`             | `render_str` + options struct             |
| Apple deps gated `target_os = "macos"` (excludes iOS)                   | `Cargo.toml:173-197`, `build.rs:7-9`    | `target_vendor = "apple"`                 |

Each of these fixes is worth doing regardless of mobile. They make the
QuickLook build smaller too.

---

## 3. iOS file access: reality check

**TL;DR** — Once the user picks a folder, Rust gets a **real POSIX path** and
plain `std::fs` works recursively. The real problems are:

1. Some providers **can't grant folders at all** (Google Drive, OneDrive).
2. Cloud files can be **dataless**, and `open()` blocks while they download.
3. There are **no change events**.
4. **Writes** should be coordinated.

All four belong in a storage layer, never in render code.

### 3.1 Which providers allow folder access

| Location                          | Folder pick | Notes                                                                                  | Confidence  |
|-----------------------------------|-------------|----------------------------------------------------------------------------------------|-------------|
| On My iPhone / iPad               | ✅          | —                                                                                      | High        |
| iCloud Drive (`~/Notes`)          | ✅          | dataless files; `.x.icloud` stubs still seen on some versions                          | High        |
| Working Copy (git)                | ✅          | designed to be picked by other apps                                                    | High        |
| Dropbox                           | ✅ (2024+)  | outside changes don't appear until the provider refreshes                               | Medium-High |
| **Sync.com**                      | ❌          | **confirmed**: folder pick fails in iA Writer (device test); single-file open in place only (§3.8) | High        |
| Google Drive                      | ❌          | folder grayed out; file-by-file only                                                   | High        |
| OneDrive                          | ❌          | file-by-file only                                                                      | High        |
| SMB (Files "Connect to Server")   | ✅*         | broke in iOS 26.1, fixed in 26.2; atomic writes fail                                    | High        |

iOS 26 only enables "Open" for folders the app can **write** to, so read-only
shares can't be picked.

### 3.2 Access mechanics

- `UIDocumentPickerViewController(forOpeningContentTypes: [.folder])` →
  `startAccessingSecurityScopedResource()` on that exact URL. That grants
  **recursive** access to all descendants.
- Persist it with plain `bookmarkData()` (`.withSecurityScope` is macOS-only).
  On each launch, resolve it, check `isStale`, and re-save. Bookmarks
  occasionally die (provider reinstall or account change), so a **"re-pick
  folder"** path is required.
- `url.path` is a real path, so **Rust `std::fs`/`open()` just work** while
  access is held. The shell keeps access open for the session and passes the
  root path to Rust.

### 3.3 Dataless files (the speed killer)

- `readdir` returns real names (or legacy `.Name.md.icloud` stubs). `stat`
  reports the real size. **`open()`/`read()` block, possibly for seconds**,
  while the file downloads.
- **Fix:** on scanner threads call
  `setiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_THREAD, IOPOL_MATERIALIZE_DATALESS_FILES_OFF)`.
  Reads of dataless files then **fail fast with `EDEADLK`**. Record those
  files as "not downloaded" and queue them for a background download worker:
  `startDownloadingUbiquitousItem` per file for iCloud, a coordinated read for
  third-party providers.
- There's no API for "Keep Downloaded" (iOS 18). Onboarding should tell users
  to long-press the folder in Files and choose *Keep Downloaded*. That
  satisfies **R8 "offline always"** for folders the user cares about.
- To verify on device: whether a plain `read()` on iOS stalls and downloads
  (as on macOS) or fails.

### 3.4 Change detection

| Mechanism                                 | iCloud     | 3rd-party                             | Use as                     |
|-------------------------------------------|------------|---------------------------------------|----------------------------|
| `NSFilePresenter` on root                 | partial    | only after provider learns of change  | "something changed" hint   |
| `NSMetadataQuery`                         | own container only | ❌                            | don't                      |
| kqueue / DispatchSource                   | 1 dir, misses edits | unreliable                   | don't                      |
| **mtime/size diff on foreground + pull-to-refresh** | ✅ | ✅                                    | **primary**                |

That fits mbr's existing incremental model (`invalidate_file`,
`rebuild_if_built`). The `notify` watcher is simply off on iOS.

### 3.5 Writes

Coordinated `.forReplacing` write: a temp file in the item-replacement
directory, then `replaceItemAt`. Never move from `NSTemporaryDirectory()`
(cross-volume). Fall back to a coordinated non-atomic overwrite (SMB). mbr's
existing per-line `expected` optimistic concurrency (`/.mbr/task`,
`/.mbr/flashcard-review`) is **exactly right** for this. A stale read becomes a
409, not a clobber. Third-party conflicts appear as "conflicted copy" sibling
files, which the scanner just lists.

### 3.6 Background limits

Apps are suspended seconds after going to the background. `BGProcessingTask`
runs when iOS decides; `BGContinuedProcessingTask` (iOS 26) runs when the user
starts a job and is shown as a Live Activity. **Indexing must be incremental,
resumable and checkpointed**, which is where the approved `Caches/` index (R12)
comes in.

### 3.7 How other apps cope

| App           | Approach                                                       | Lesson                                                 |
|---------------|----------------------------------------------------------------|--------------------------------------------------------|
| Obsidian      | Capacitor; only its own iCloud container / local / own sync    | slow iCloud start (`NSMetadataQuery`, dataless files); no arbitrary folders |
| iA Writer     | Any picker-able folder as a "Location"                         | the model to copy; Google Drive/OneDrive file-only, "search nearly impossible" |
| Textastic     | External folders + built-in Dropbox/SFTP/WebDAV clients        | own clients route around the provider limits           |
| Ulysses       | Own Dropbox API + Files folders                                | same                                                   |
| Logseq        | iCloud graphs                                                  | evicted files mistaken for deletions, so they must be distinguished |
| Runestone/Kodex | single files in place                                        | no folder workspace on iPhone                          |

**Implication for R2:** mbr matches iA Writer for any provider that supports
folders. Google Drive and OneDrive would need a dedicated API client, which
your future sync library (R10) could replace. Out of scope for v1.

### 3.8 Sync.com specifics

**TL;DR** — **Confirmed: Sync.com does not support folder access on iOS.** A
device test (*iA Writer → Add Location → Sync*) fails to pick a folder. That
matches the desk research: Sync.com shows up in Files and supports on-demand
download and offline folders (Pro), but it isn't on Textastic's or Working
Copy's lists of folder-capable providers, its iOS app was last updated
September 2024, and it has **no public API, no WebDAV and no rclone backend**.

**Consequence for v1:** Sync.com is **single-file only** (open in place,
§3.9). Relative images and wikilinks to neighbours show placeholders. Vaults
like the family tree or health info either move to iCloud or Working Copy,
or **wait for your encrypted sync library (R10)**, which would naturally
replace Sync.com here.

This is the strongest real-world argument for doing the `Vault` seam now.
Your second-most-important storage location may only be reachable through a
library.

### 3.9 Single files (R3)

**TL;DR** — Opening a file in place (`LSSupportsOpeningDocumentsInPlace`,
`DocumentGroup`, share sheet) grants access to **that file only**, not its
siblings. Relative images, wikilinks to neighbours and backlinks can't
resolve. That is the iOS sandbox, with no workaround (macOS's
`primaryPresentedItemURL` doesn't exist on iOS).

| Entry point               | Gets                                    | mbr behaviour                                                          |
|---------------------------|-----------------------------------------|------------------------------------------------------------------------|
| Open in place (Files)     | security-scoped URL to one file; writable | render as a **single-file vault**; offer "Grant access to folder…" (picker with `directoryURL` = parent) to upgrade |
| Share sheet / extension   | copy or a short-lived URL; no write-back | read-only render                                                       |
| File inside a known vault | already covered by the vault bookmark    | open within that vault (match path prefix)                              |

**QuickLook on iOS is not a reliable win.** iOS only calls preview extensions
for types it can't preview itself. Reports say iOS 27 stopped calling custom
`.md` extensions (plain-text preview wins), and an extension would have no
sibling access anyway. **Deprioritize it.** The app's own open-in-place covers R3.

---

## 4. Shell & runtime architectures

**TL;DR** — Seven options were compared. **A (Swift shell + UniFFI + scheme
handler)** wins on iOS. **B (in-app localhost server)** is the safety net and
the likely Android transport. Tauri and wry/tao on iOS are blocked **right
now** by an iOS 27 UIScene launch crash, and they push their framework into
the desktop code. A Swift rewrite or a WASM core throws away mbr's biggest
asset.

### 4.1 Options at a glance

| Option                                   | iOS fit | Android fit | Code reuse | Core pollution | Maint. risk | Verdict              |
|------------------------------------------|---------|-------------|------------|----------------|-------------|----------------------|
| **A** SwiftUI + UniFFI + `WKURLSchemeHandler` | ★★★★★ | ★★ (use B)  | High       | Low            | Low         | **Recommended**      |
| **B** In-app localhost axum + WKWebView  | ★★★★    | ★★★★        | Highest    | Low            | Medium      | Fallback / Android   |
| **C** Tauri 2 mobile                     | ★★★     | ★★          | High       | **High**       | Med-High    | No                   |
| **D** wry + tao directly on iOS          | ★★      | ★★          | Highest    | Medium         | High        | No                   |
| **E** Swift rewrite (swift-markdown etc.)| ★★★     | ✗           | None       | n/a (fork)     | High (drift)| No                   |
| **F** Rust core → WASM in WKWebView      | ★★★     | ★★★         | Medium     | High (re-arch) | Medium      | Not for iOS          |
| **G** Capacitor + Rust plugins           | ★★★★    | ★★★         | High       | Medium (npm)   | Low-Med     | Viable runner-up     |

### 4.2 A: SwiftUI shell + Rust xcframework + scheme handler (recommended)

```
┌──────────────── iOS app (Swift, apple/ios/) ───────────────────┐
│ SwiftUI: vault list, folder picker, bookmarks, settings,       │
│          download/offline status, share/open-in               │
│ FileCoordinatorBridge: coordinated writes, downloads ◄─┐       │
│ WKWebView ── mbr://vault/... ──► MbrSchemeHandler      │       │
└──────────────────────────────────────┬─────────────────┼───────┘
                    UniFFI (async)     ▼                 │ callback trait
┌──────────────── mbr-ffi (Rust) ──────────────────────────────────┐
│ handle(request) -> response  (axum Router::oneshot, tokio rt)    │
│ open_vault(path), refresh(), download_status()                   │
├──────────────── mbr-server / mbr-core (Rust) ────────────────────┤
│ router · render · templates · search · index · Vault trait ◄────┘
└──────────────────────────────────────────────────────────────────┘
```

| Pros                                                                 | Cons                                                                  |
|----------------------------------------------------------------------|-----------------------------------------------------------------------|
| Reuses **all** of mbr: router, handlers, templates, front end         | Some WKURLSchemeHandler quirks need spikes (below)                    |
| No socket, no port, no ATS exception, no other app can reach it        | WebSockets can't be intercepted → live reload needs a JS-bridge transport |
| Swift owns what Swift is good at (pickers, bookmarks, coordination, UIScene) | Two languages to maintain (but Swift is ~1–2k lines of glue)     |
| Proven pattern (Firefox iOS, Capacitor); UniFFI already in repo        | `WKURLSchemeTask` callbacks must run on the main thread                |
| Unaffected by the tao/Tauri iOS 27 crash                              | Must hand-roll Range/206 for `<video>`                                 |

**Known scheme-handler quirks to spike first:**

| Concern                  | Status                                                                     | mbr impact                                         |
|--------------------------|----------------------------------------------------------------------------|----------------------------------------------------|
| POST bodies              | string/ArrayBuffer likely OK; **Blob/FormData/stream bodies may arrive nil** | search/task/FSRS send JSON strings (fine); upload is out of v1 |
| Streaming responses      | ✅ chunked `didReceive(data)`                                               | stream large media; never buffer >100 MB           |
| Range / `<video>`        | works if you return 206 + `Content-Range`                                  | needed only for media                              |
| WebSockets               | ❌                                                                          | `mbr-live-reload.ts` needs a pluggable transport   |
| Service workers, cookies | ❌                                                                          | mbr uses neither                                   |
| ES modules / `import()`  | ✅ with correct MIME + `Access-Control-Allow-Origin`                        | 9 lazy chunks rely on this; **spike #1**           |
| localStorage             | ✅, keyed to scheme+host; evictable                                        | review notes, FSRS UI prefs, `mbr_genealogy_chart` |

### 4.3 B: In-app localhost server

| Pros                                                                 | Cons                                                                 |
|----------------------------------------------------------------------|----------------------------------------------------------------------|
| Zero transport code: real HTTP, POST bodies, Range, **WebSocket live reload** | Listener dies while the app is suspended → detect and rebind on foreground |
| Identical to desktop `mbr -s`; also the **Android answer**             | Any process on the device can connect → per-launch token + Host/Origin checks |
| Loopback is exempt from the Local Network prompt                      | Port changes break back/forward history                               |
| `NSAllowsLocalNetworking` only                                        | Mozilla left this model for security reasons                          |

**Keep both behind one interface.** The shell asks Rust for a "base URL"
(`mbr://…` or `http://127.0.0.1:port/…?t=token`). The front end can't tell the
difference, since it already uses relative URLs and `resolveUrl()`.

### 4.4 C/D: Tauri 2 and raw wry/tao

- **Tauri 2:** on iOS it uses the same WKURLSchemeHandler (same quirks); on
  Android it **drops POST bodies**. The folder picker and bookmarks need custom
  plugins anyway (plugin-fs issues #3030, #14587). Adopting it means
  `tauri.conf` + capabilities + `gen/apple`, and Tauri replacing mbr's
  hand-tuned wry/tao/muda desktop code (`browser.rs`'s careful GTK and accel
  work). **Live blocker:** tauri-runtime-wry pins tao ^0.35, and that crashes
  at launch on the iOS 27 SDK (tauri #15719, tao #1340).
- **Raw wry/tao:** the most code shared with the desktop GUI, but the folder
  picker, bookmarks and `NSFileCoordinator` would all be written in objc2 from
  Rust. cargo-mobile2's maintainers don't support use outside Tauri. Same
  iOS 27 issue.

| Pros                                    | Cons                                                                  |
|-----------------------------------------|-----------------------------------------------------------------------|
| "All Rust"; one shell for desktop+mobile | iOS 27 launch crash today; immature mobile story                     |
| Tauri: Android shell for free            | UIKit-in-Rust is the worst place to write pickers/coordination        |
|                                         | Tauri rewrites the desktop GUI too, a large blast radius (violates R6) |

### 4.5 E: Swift port / partial port

swift-markdown (cmark-gfm) has footnotes disabled; Down is stale; Ink isn't
CommonMark. mbr's value is ~80k lines of Rust extensions (sections + attrs,
wikilinks, chat blocks, tasks, contacts, relationships, flashcards, link
transform, `data-mbr-line`…). A port forks all of it. **The only sensible
"port" is the native chrome**: vault list, settings, download status.

### 4.6 F: Rust core compiled to WASM

Feasible (pulldown-cmark ~95 KB wasm; Tera needs regex; regex-lite helps), but
it **still needs native code for files**. It turns axum routing into
JS-driven calls (a re-architecture) and buys nothing on iOS over A. It's worth
remembering only for a future pure-web/PWA "open a folder in the browser"
build.

### 4.7 G: Capacitor + Rust

A on iOS with a JS plugin layer added, plus npm/Capacitor tooling. Android
shares the Tauri POST-body limitation. Choose it only if one shell for both
mobile platforms becomes more important than a minimal toolchain.

---

## 5. The storage seam (`Vault` trait)

**TL;DR** — This is the one change to mbr's core that every mobile path needs.
**R10 (sync as a library) makes it mandatory anyway.** Design it **bulk-first**
so the desktop hot path stays as fast as today, and so iOS never crosses FFI
per file.

### 5.1 Why

| Driver                        | What it needs from storage                                         |
|-------------------------------|--------------------------------------------------------------------|
| iOS iCloud/Dropbox            | `materialized?` state, fail-fast reads, download requests, coordinated writes |
| Android SAF                   | no paths at all: `content://` tree URIs, fd-based reads             |
| Future encrypted sync (R10)   | read/list/search through a library API                              |
| Tests                         | in-memory vaults instead of `NamedTempFile` everywhere              |
| Desktop                       | unchanged speed (WalkDir + rayon + `std::fs`)                       |

### 5.2 Shape (sketch, not final API)

```rust
/// Repository-relative, normalized, never escapes the root (replaces canonicalize-containment).
pub struct VaultPath(/* … */);

pub enum Availability { Local, NotDownloaded, Unknown }

pub struct Entry { path: VaultPath, kind: EntryKind, size: u64, modified: SystemTime, availability: Availability }

pub trait Vault: Send + Sync {
    fn scan(&self, ignore: &IgnoreRules) -> Result<Vec<Entry>, VaultError>;   // bulk, one call
    fn stat(&self, p: &VaultPath) -> Result<Option<Entry>, VaultError>;      // path_resolver probes
    fn read(&self, p: &VaultPath) -> Result<Bytes, VaultError>;              // NotDownloaded is an error variant
    fn read_prefix(&self, p: &VaultPath, n: usize) -> Result<Bytes, VaultError>; // frontmatter (8 KB today)
    fn write_atomic(&self, p: &VaultPath, data: &[u8]) -> Result<(), VaultError>;
    fn request_download(&self, paths: &[VaultPath]) {}                       // default no-op
    fn local_path(&self, p: &VaultPath) -> Option<PathBuf> { None }          // fast path: ServeFile, grep, mmap
}
```

**Implementations:**

| Impl              | Where           | Notes                                                                  |
|-------------------|-----------------|------------------------------------------------------------------------|
| `LocalVault`      | desktop, iOS    | today's code moved behind the trait; on iOS sets the dataless iopolicy per scanner thread |
| `CoordinatedVault`| iOS             | wraps `LocalVault`; `write_atomic`/`request_download` call a **UniFFI foreign trait** that Swift implements with `NSFileCoordinator` |
| `SafVault`        | Android, later  | JNI to `DocumentsContract` (one query per directory); reads via fds    |
| `SyncVault`       | future          | your encrypted-sync crate                                              |
| `MemVault`        | tests           | replaces temp-dir fixtures                                             |

### 5.3 Performance guardrails

- **`local_path()` fast path.** Where a real path exists (desktop, iOS), the
  grep-searcher, `ServeFile` and rayon scans keep using it, so desktop
  benchmarks shouldn't move. Verify with the existing `repo_scan`/`search`
  criterion suites before and after.
- **No per-file FFI.** Swift is called only for writes and downloads. The scan
  is pure Rust on iOS.
- **Static dispatch where it's hot** (`Repo<V: Vault>`), with `Arc<dyn Vault>`
  only at the server-state boundary.

### 5.4 Pollution control (R6)

| Rule                                                                       | Effect                                         |
|----------------------------------------------------------------------------|------------------------------------------------|
| No `#[cfg(target_os = "ios"/"android")]` in core modules                   | platform behaviour is **injected** via traits  |
| Platform code lives only in `mbr-ffi` (and `apple/`)                       | core diffs stay platform-agnostic              |
| Features gate subsystems (`server`, `watcher`, `ssg`, `media-metadata`, `gui`) | mobile builds just leave them out          |
| Mobile-specific UX lives in the front end behind capability flags (`window.__MBR_CONFIG__`) | same pattern as `server_mode`/`gui_mode` today |

---

## 6. Code organization: separate repo vs workspace vs hybrid

**TL;DR** — **Decided: O2, everything in the mbr repo.** A Cargo workspace
inside the mbr repo, so the core, server and FFI crates evolve and test
together and CI catches FFI breakage. The **iOS app lives in the same repo
under `apple/ios/`**, and `quicklook/` moves to `apple/quicklook/` in a later
PR (PR2), so all Apple shells sit side by side. Don't start with the
workspace: first get the core building cleanly **behind features in one
crate**, then promote it to crates.

**Why not the separate app repo (O4) this doc first recommended:**

| Reason | Detail |
|--------|--------|
| Solo maintainer | O4's main benefit, independent release cadence for a separate team, doesn't apply. Two repos would mean two PRs and a version bump for every FFI change. |
| Atomic FFI + Swift changes | A change to `MbrApp`'s UniFFI surface and the Swift that calls it land in **one PR**, reviewed and tested together. There is no xcframework release step to pin. |
| Shared flake and Cachix | The iOS rust targets live in the same `flake.nix` dev shell (`rustToolchainDev`), and a future `mbr-ios-staticlib` derivation reuses crane's `cargoArtifacts` and the existing Cachix cache. |
| Precedent | `quicklook/` already ships a Swift/XcodeGen project inside this repo, built by `flake.nix`. `apple/` generalises it. |

The cost is Swift/Xcode churn in a Rust repo, which `apple/` contains to one
directory. CI cost is controlled by making any macOS iOS-build job opt-in
(PR2 decides).

### 6.1 Options

| Option                                           | Description                                                                 |
|--------------------------------------------------|-----------------------------------------------------------------------------|
| **O1** Monorepo, single crate + features          | add `ios`/`ffi` features; app lives in `ios/` like `quicklook/`             |
| **O2** Monorepo, Cargo workspace                 | `crates/mbr-core`, `mbr-server`, `mbr-cli`, `mbr-ffi`; app in `apps/ios/`   |
| **O3** Separate repo for everything mobile       | app repo depends on `mbr` as a git dependency and builds the FFI itself     |
| **O4** **Hybrid**: workspace in mbr + separate app repo | mbr repo owns all Rust incl. `mbr-ffi`; app repo consumes a released xcframework |

### 6.2 Pros & cons

| Option | Pros                                                                                                     | Cons                                                                                                    |
|--------|----------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------|
| **O1** | Least churn; no crate boundaries to design; one `cargo test`                                              | Feature-flag sprawl; easy to accidentally pull axum/notify into mobile; slow compile; nothing *enforces* the core/edge boundary |
| **O2** | Compiler-enforced layering (core can't import server); faster incremental builds; FFI and core change in one PR | Big one-time move (git blame noise, CI and Nix updates); Swift/Xcode churn mixed into a Rust repo; App Store signing and assets in mbr |
| **O3** | mbr repo untouched; app moves at its own pace; Swift tooling isolated                                    | FFI wrapper lives far from the code it wraps → silent drift; core refactors break the app with no CI signal; harder to share the `Vault` design |
| **O4** | Layering enforced; FFI breakage caught by **mbr's** CI (build the xcframework + Swift binding smoke test); app repo stays Swift-only; clean versioning (app pins `mbr-ffi` vX.Y) | Two repos to coordinate for API changes; needs a release step that produces the xcframework (Nix already does the macOS staticlib, so extend it) |

### 6.3 Proposed workspace (O4)

| Crate         | Contents                                                                                           | Deps excluded                     |
|---------------|----------------------------------------------------------------------------------------------------|-----------------------------------|
| `mbr-core`    | markdown/html/templates/config(model)/pure modules/`Vault`/`LocalVault`/repo/path_resolver/search/indexes/assets | axum, clap, notify, wry, ffmpeg |
| `mbr-server`  | axum router + handlers (over `Vault`), edit auth, live-reload channel trait                        | clap, wry                         |
| `mbr-ssg`     | `build.rs` + pagefind                                                                              | —                                 |
| `mbr` (bin)   | cli, main, watcher, browser (gui), external_open, media-metadata                                   | —                                 |
| `mbr-ffi`     | UniFFI surface: QuickLook (moved from core) + mobile `MbrApp` (open_vault, handle_request, refresh) | —                                 |

The front end (`components/`, `templates/`) stays where it is. It is embedded
by `mbr-core`'s asset table, so every shell gets the same bundle.

**Where existing pieces go:** `apple/ios/` (new, PR0) and `apple/quicklook/`
(moved from `quicklook/` in PR2). The `mbr-ffi` crate serves both.

---

## 7. Feature-by-feature on iOS

**TL;DR** — Almost everything works as-is through the in-process router. The
real work is: writes through coordination, live reload over a new transport,
search speed at 3k notes, the tasks-panel layout, and a handful of
keyboard-only features that need tap targets.

| Feature                                  | Works via router as-is? | Work needed                                                                                     |
|------------------------------------------|-------------------------|-------------------------------------------------------------------------------------------------|
| Render, nav, breadcrumbs, tags, browse   | ✅                      | —                                                                                               |
| mermaid / KaTeX / hljs / reveal / graphs | ✅ (embedded)           | spike: chunk `import()` over `mbr://`                                                            |
| `site.json`, `links.json`, backlinks     | ✅                      | lazy indexes already; persist to `Caches/` (R12)                                                |
| Search (metadata)                        | ✅                      | —                                                                                               |
| Search (content)                         | ✅ (grep per query)     | ~13 ms/276 files on desktop → maybe ~150–400 ms/3k on phone; **measure**; then an in-memory or persisted inverted index |
| Task browser, task toggle                | ✅                      | writes → `CoordinatedVault`; tasks-panel layout at <600px                                       |
| Flashcards + FSRS write                  | ✅                      | same write path; deck UI is already touch-ready                                                 |
| Review notes                             | ✅ (localStorage)       | "Add note" FAB collides with the iOS selection callout                                          |
| Edit auth (`check_edit_access`)          | ⚠                       | `ConnectInfo` is absent in-process → inject a trusted "local app" connect-info layer; treat the scheme handler as same-origin |
| Live reload (WebSocket)                  | ❌                      | pluggable transport in `mbr-live-reload.ts` (WS / native bridge); not v1                        |
| Milkdown editor, create/move/upload      | ⚠                       | out of v1 (Blob bodies through the scheme handler are unverified)                                |
| oembed                                   | ⚠                       | network fetch; keep the cache, short timeout; offline → plain links                              |
| Video/PDF media                          | ⚠                       | Range support in the handler; `media-metadata` (ffmpeg/pdfium) excluded on iOS                  |
| QuickLook previews in Files              | ⚠ (reuse `quicklook.rs`) | iOS 27 reportedly bypasses custom `.md` extensions; deprioritized (§3.9)                      |
| Single-file open in place (R3)           | ✅ (single-file vault)  | `Vault` rooted at one file; "grant folder" upgrade                                               |

**Mobile UX gaps (shared by all paths, front-end only):**

| Gap                                                     | Fix                                                        |
|---------------------------------------------------------|------------------------------------------------------------|
| `100vh` in info/page-errors/search/sidebar              | `100dvh`; `viewport-fit=cover` + `env(safe-area-inset-*)`   |
| Tasks panel two-column at 390px (≈210px for cards)      | single-column/stacked folder pane under 600px              |
| Keyboard-only: fuzzy nav (`f`/`F`/`T`), `H`/`L`, `?`    | "more" menu in nav                                         |
| Review "Add note" FAB under the iOS selection menu       | place below the selection on coarse pointers               |
| Search modal + iOS keyboard (70vh + 10vh)               | test, then use the visual viewport                         |
| iPad parity (R8)                                        | ≥1024px already gives the desktop layout; verify split view / Stage Manager widths |

---

## 8. Performance budget

**TL;DR** — The budget items are: hide WKWebView's ~0.3–0.6 s cold start,
never block on dataless files, and keep the index warm across launches in
`Caches/`.

| Phase                    | Risk                                          | Mitigation                                                        |
|--------------------------|-----------------------------------------------|-------------------------------------------------------------------|
| App launch               | WKWebView process spawn 280–580 ms            | create/preload during splash; one reused web view                  |
| Vault open (cold)        | full scan of 4.8k files; dataless stalls      | iopolicy fail-fast; frontmatter `read_prefix`; rayon; show nav as soon as the listing is ready |
| Vault open (warm)        | re-reading unchanged files                    | `Caches/` index keyed by (path, mtime, size); diff on foreground   |
| Page render              | already ms-level                              | unchanged                                                          |
| Search keystroke         | grep over 3k files                            | measure → in-memory content cache → inverted index                 |
| Memory                   | WebContent jetsam ~2 GB; app ~½ RAM           | stream media; handle `webViewWebContentProcessDidTerminate`         |
| Threads                  | tokio multi-thread + rayon global pool        | small fixed pools on mobile (config, not cfg)                      |

---

## 9. Android outlook (R9)

**TL;DR** — The same `mbr-core`/`mbr-server`, a Kotlin shell, `SafVault` via
JNI, and the **localhost transport** (B). Android's `shouldInterceptRequest`
can't see POST bodies, which is why B rather than A. Nothing in the iOS plan
blocks this, as long as the `Vault` seam exists and the transport is pluggable.

| Concern              | Answer                                                                    |
|----------------------|---------------------------------------------------------------------------|
| File access          | SAF tree URIs; `DocumentsContract` (one query per folder); fds to Rust   |
| `MANAGE_EXTERNAL_STORAGE` | possible ("document management") but a Play review risk; local storage only |
| Transport            | localhost + per-launch token                                              |
| Build                | `cargo-ndk`, UniFFI Kotlin bindings                                       |

---

## 10. Recommended path & spikes

**TL;DR** — De-risk with throwaway spikes **before** touching mbr's structure,
then refactor mbr in an order that benefits desktop first.

### 10.1 Spikes (in `apple/ios/`, PR0)

| # | Spike                                                                                      | Answers                                     | Status |
|---|--------------------------------------------------------------------------------------------|---------------------------------------------|--------|
| 1 | Hello-world SwiftUI + today's `mbr` staticlib (`--no-default-features`) for `aarch64-apple-ios(-sim)`; `Router::oneshot` behind a `URLSchemeHandler` | does the full crate even build for iOS; mermaid/chunk `import()`; JSON POST bodies; localStorage persistence | **Done on simulator** (§12) |
| 2 | Pick `~/Notes` on iCloud on a real iPhone; scan with/without the dataless iopolicy; time cold/warm | dataless behaviour on iOS, scan time at 4.8k files | Device checklist: `apple/ios/TESTING.md` (was the spike's `SPIKE.md`) |
| 3 | Same for a Dropbox / Working Copy folder                                                   | R2 viability per provider                   | Sync.com already answered (❌, §3.8); rest on device |
| 4 | Content search timing at 3k notes on device                                                | whether an index is needed for v1           | Device checklist |
| 5 | Write round-trip (task toggle) on iCloud                                                    | write path, conflict behaviour              | Works on simulator (local file); iCloud on device |

### 10.2 mbr-side refactor order (each step stands alone, desktop benefits)

1. **Untangle** (§2.3): `assets` module, move lexical helpers and enums, split
   errors, `target_vendor = "apple"`, `render_str`. No behaviour change.
2. **Feature gates**: `server`, `watcher`, `ssg`, so the QuickLook/mobile build
   drops axum/notify/pagefind where unused.
3. **`Vault` trait** with `LocalVault` only; port `repo`, `path_resolver`,
   `search`, `task_index`, and the write handlers. Benchmarks before and after.
4. **Workspace split** (§6.3), once the boundaries are proven.
5. **`mbr-ffi` mobile surface** + xcframework build in Nix/CI.
6. **Front-end mobile UX fixes** (§7). These can start at any time; they
   help phone Safari against `mbr -s` today.

### 10.3 Interim option (zero code): use mbr from your phone this week

Run `mbr -s` on the Mac and expose it with `tailscale serve`, which gives you
HTTPS plus a tailnet-only boundary (mbr has **no read auth**, so never bind
`0.0.0.0` on a LAN). Every feature works, including editing, given
`edit_token_hash`. It's a good way to dogfood the mobile UX fixes before the
app exists.

---

## 11. Open questions for you

Answered questions keep their numbers; the answers are in bold.

1. **Sync.com:** can you try *iA Writer → Library → Add Location → Sync* (or
   Textastic "Add External Folder") on your phone? If it's single-file only, OK
   to treat those vaults as "single files in v1, the encrypted sync library
   later"? **Answered: tested in iA Writer, and folder access does not work.
   Sync.com is single-file in v1 (§3.8).**
2. **Google Drive/OneDrive:** confirm these are not needed (they're impossible
   via Files).
3. **Vault switching:** several vaults (Notes, family tree, health) as a list
   in the app, one active at a time? Or cross-vault search?
4. **Health/family data on device:** any requirement that the `Caches/` index
   be encrypted at rest (iOS Data Protection class), given R10's direction?
5. **App repo name/location** and whether `quicklook/` should eventually move
   there. **Answered: no separate repo. The app lives at `apple/ios/` in this
   repo, and `quicklook/` moves to `apple/quicklook/` in PR2 (§6).**
6. **Live refresh** isn't v1, but OK to make the live-reload transport
   pluggable during the refactor?
7. **Where this doc lives:** repo root for now; move to the new app repo once
   it exists? **Answered: the repo root, committed in PR0.**
8. **Minimum OS.** **Answered: iOS/iPadOS 26.**

---

## 12. PR0 spike findings (2026-10-08)

**TL;DR** — The architecture in §4.2 works as designed. The **entire crate
compiles for iOS with zero source changes**. Every failure was toolchain
plumbing between the nix shell and Xcode. On the iOS simulator, mbr's
unmodified router serves the unmodified front end over `mbr://`. Render,
mermaid, lazy chunks, JSON POSTs, a real task-toggle write and localStorage
persistence all pass. Two transport quirks need code in the shell layer, not
in core: redirects and `Blob` bodies. How to build and run, and the device
checklist, are in `apple/ios/TESTING.md` (was the spike's `SPIKE.md`).

### 12.1 What was built

| Piece | Where | Notes |
|-------|-------|-------|
| UniFFI gate `target_os = "macos"` → `target_vendor = "apple"` (dep + build-dep) | `Cargo.toml` | objc2-app-kit / objc2-foundation stay macOS-only. `build.rs` needed no change |
| iOS rust std | `flake.nix` `rustToolchainDev` only (Darwin) | never in `rust-toolchain.toml`, so nothing new goes to Cachix |
| `MbrApp` (`open`, async `handle`, async `wait_for_scan_ms`, `root_path`) plus `set_materialize_dataless_files` / `materializes_dataless_files` | `src/mobile.rs` (behind `ffi`, proc-macro UniFFI alongside the UDL QuickLook API) | owns a tokio runtime (2 workers, 8 blocking) that shuts down in the background; uses `Server::init`/`ServerConfig` unchanged |
| Watcher off on iOS | `server.rs` (`if cfg!(target_os = "ios") { return; }` in the watcher thread) | the only core-code change; replaced by the `watcher` feature in PR2 |
| SwiftUI app + XcodeGen + `build.sh` | `apple/ios/` | `WebPage` + `URLSchemeHandler` (iOS 26 API; no WKWebView fallback needed) |

### 12.2 Compile failures for `aarch64-apple-ios[-sim]` (input to PR2)

Command: `cargo build --lib --no-default-features --features ffi --target aarch64-apple-ios-sim`
(and `aarch64-apple-ios`, release). **No Rust source needed gating to compile.**
Each failure and its fix:

| # | Failure | Cause | Fix (in `apple/ios/build.sh`) |
|---|---------|-------|-------------------------------|
| 1 | `ring` build script: `xcrun --show-sdk-path --sdk iphonesimulator` exit 255 | the nix shell's xcbuild `xcrun` only knows the nix macOS SDK | `xcrun` shim on `PATH` that runs `/usr/bin/xcrun` with `DEVELOPER_DIR=Xcode` |
| 2 | Host build scripts fail to link (`icu_*`, `syn`, `getrandom`, …) | exporting `DEVELOPER_DIR`/`SDKROOT` for Xcode globally breaks nix's host linker | change **target-side** tools only, never the global env |
| 3 | `clang: invalid argument '-mmacos-version-min=14.0' not allowed with '-mios-simulator-version-min=26.0'` | Apple clang reads the shell's `MACOSX_DEPLOYMENT_TARGET` | target-clang wrapper unsets it and sets `SDKROOT` to the iOS SDK |
| 4 | `slug` (via `tera`): `ld: building for iOS Simulator, but linking in .tbd built for macOS` | slug declares a `cdylib` crate-type, so rustc *links* for the target, using nix `cc` | `CARGO_TARGET_<TRIPLE>_LINKER` = the target-clang wrapper |
| 5 | (latent) `-weak_framework QTKit/OpenGL/...` in the shell's `RUSTFLAGS` | macOS-only frameworks | unset `RUSTFLAGS` for iOS cargo runs (with `--target` it never reached host artifacts anyway) |
| 6 | Every run rebuilt every crate | the shim dir came from `mktemp`, and the linker path is part of cargo's fingerprint | fixed shim dir `target/ios-shims/` |
| 7 | Xcode link: `found architecture 'arm64', required 'x86_64'` | the generic simulator destination builds x86_64 too | `ARCHS: arm64` in `project.yml` (as in quicklook) |

Things that **did not** break, contrary to §2.3's expectations:

- `notify` with `macos_fsevent` compiles: the feature is a no-op off macOS,
  and notify picks its kqueue backend for `target_os = "ios"`. It still should
  not *run* there (one descriptor per watched file, blind to File Provider
  changes), hence the runtime skip and PR2's `watcher` feature.
- axum, tower-http, tokio (`net`), reqwest + rustls/ring, pagefind, grep,
  image, lopdf, argon2 and rpassword all compile for iOS unchanged. PR2's
  feature gates are about **size and hygiene, not compilability**.

Pre-existing, found on the way (also on `main`):
`cargo clippy --all-targets --no-default-features --features ffi -- -D warnings`
fails with `clippy::large_const_arrays` in UniFFI's generated UDL scaffolding
(`OUT_DIR/mbr.uniffi.rs:10`, `UNIFFI_META_CONST_UDL_MBR`). An `#[allow]` on
`include_scaffolding!` is ignored ("unused attribute"). With that one lint
allowed, the ffi build is clippy-clean, including `mobile.rs`. PR2 needs a
fix, either a crate-level `cfg_attr(feature = "ffi", allow(...))` or moving
QuickLook to proc-macros, before "every CI combination builds" can include
ffi clippy.

### 12.3 Transport findings (simulator, iOS 27.0 runtime)

| Question (from §4.2) | Answer |
|----------------------|--------|
| ES modules / `import()` over a custom scheme | ✅ main bundle, mermaid's lazy script, and `import()` of a lazy chunk all load. No CORS or MIME headers needed beyond what mbr already sends, because everything is same-origin `mbr://localhost` |
| JSON string POST bodies | ✅ arrive in `URLRequest.httpBody` (search 39 B, task 96 B) |
| `Blob` POST bodies | ❌ `httpBody` **and** `httpBodyStream` are nil, so the router sees an empty body (400). Out of v1 (uploads only) |
| Redirects | ❌ **WebKit ignores a 3xx from a `URLSchemeHandler`**: it shows the empty body at the old URL. mbr's 301 to `/{path}/` matters (relative links), so `mobile.rs` turns a 3xx answering an HTML navigation into a 200 document that calls `location.replace(target)`. Fetches and subresources are untouched |
| `Content-Encoding` | WebKit would hand a gzip body to the page undecoded, so the adapter drops `Accept-Encoding` before the router's compression layer |
| Edit access (`check_edit_access`) | ✅ without weakening it. `ConnectInfo` = injected loopback; `X-MBR-Edit` comes from the front end; **WebKit sends no `Origin` (and no `Sec-Fetch-Site`) on a same-origin `mbr://` POST**, so the same-origin check takes its "no Origin" branch; `Host` is derived from the URL authority, so it must be `localhost`. A foreign `Origin` is still refused (unit test) |
| localStorage | ✅ persists across relaunch and reinstall over the same data container |
| WebSockets / live reload | not attempted; `mbr-live-reload.ts` targets `ws://localhost/.mbr/ws`, which cannot go through a scheme handler (expected, §4.2) |
| Main-thread rule | the async `URLSchemeHandler` API returns an `AsyncSequence`; the Rust call runs on a detached task, and nothing needed the main thread |

Another core bug, fixed in the adapter: a **non-canonical root** (macOS
`/tmp`, iOS `/var` → `/private/var`) made every page's `markdown_source`
`../../../private/...`. `main.rs` canonicalizes before `Config::read`; the
mobile entry point now does the same. `Config::read`'s upward root search
is also overridden: on iOS the picked folder *is* the grant, so an ancestor
`.git` must never become the root.

### 12.4 Timings

Simulator (iPhone 17, iOS 27.0) on a Mac loaded by a parallel build, release
Rust, Debug Swift. Indicative only; device numbers go here once
the checklist in `apple/ios/TESTING.md` is run.

| | Cold | Warm |
|---|---|---|
| `MbrApp.open` | 85 ms | 31 ms |
| Scan complete, 46 files (`docs/`) | 468 ms | 138 ms |
| First page `finished` (WebContent spawn + 5.5 MB mermaid parse) | 2706 ms | 704 ms |
| Request in handler (page, asset, search, task) | 0–13 ms | 0–8 ms |
| Staticlib size (`libmbr.a`, release, per arch) | 315–316 MB | — |
| App bundle (Debug) | 61 MB (55 MB linked code) | — |

Device, `~/Notes` on iCloud (to fill in): cold scan ___, warm scan ___,
with iopolicy OFF ___ / ___, content search ___, task-toggle round trip ___.

### 12.5 Decisions and inputs for later PRs

- **PR2:** the `watcher` feature replaces the `cfg!(target_os = "ios")` skip;
  fix the ffi clippy lint (§12.2); `build.sh`'s target-side env recipe is the
  template for any `mbr-ios-staticlib` nix derivation (cc-rs needs the real
  iOS SDK, so it will likely need `xcrun`/Xcode outside the sandbox or
  nixpkgs' iOS SDK).
- **PR5:** move the dataless iopolicy from the spike's **process** scope (a
  toggle, for the device A/B) to **thread** scope on scanner threads (tokio
  `on_thread_start`, rayon `start_handler`), so render-path reads of a
  dataless file can still choose to wait. Stream large responses instead of
  buffering (`to_bytes(usize::MAX)` today) and add `Range`/206 for media.
  Promote `redirect_as_document` and the `Accept-Encoding` strip into the
  transport layer for good.

### 12.6 App simulator findings (PR5b, 2026-10-14)

iPhone 17 and iPad Pro 13-inch (M5) simulators, iOS 27.0, Debug Swift with
release Rust. Sample vault = `docs/` (44 files). Indicative only.

| Question | Answer |
|----------|--------|
| Does WebKit honour `Refresh` from a scheme handler? | **Yes.** A 503 Downloading page carrying `Retry-After: 2` and `Refresh: 2` is re-requested every 2 s by WebKit itself. The app keeps a Swift watchdog that reloads only if no new request arrives, and logs which of the two happened |
| Streaming and Range | `Range: bytes=0-99` gives 206 with 100 bytes; a `<video>` loads metadata and seeks. The handler is pull-based, one `nextChunk()` per `next()` |
| Writes through the `FileCoordinator` | The task toggle and the flashcard review both go through `NSFileCoordinator` `.forReplacing` → `replaceItemAt` (4–516 ms on the simulator; the first write is the slow one) |
| Live reload | `<mbr-live-reload>` opens no socket on `mbr:`. The app reloads after `refresh()` finds a change (foreground: +0 ~1 -0 in 2 ms, reload 101 ms) |
| WebContent crash | `page.navigations` throws `webContentProcessTerminated`; the app reloads the last shown URL (673 ms) |
| `location.reload()` from script | Returns at once, and a following `import()` raced it and failed; the app awaits the native `page.reload()` instead |
| Index cache | Cold: `fromCache=false`, first page 684–1298 ms. Relaunch: `fromCache=true`, first page 558 ms. Scan/ready 3–30 ms at this size |
| Prewarm | A `WebPage` loaded with an empty document based at `mbr://localhost/` is ready 605 ms after launch, off the critical path |
| iPad | Full width (1032 CSS px) gets the desktop layout. `simctl` cannot make a Split View; that check is on the device list |
| Size | Device Release app 46 MB (39 MB executable); staticlib 222 MB per arch before dead-code stripping |

## Appendix A — Sources

Apple Developer Forums threads 691738, 775228, 766646, 773373, 685170, 756963,
785030, 816424, 127500, 805554, 650810, 85038, 733774, 801690, 795083;
`setiopolicy_np(3)`; TN3150; WWDC21 "FileProvider", WWDC25 "Meet WebKit for
SwiftUI" & background tasks; WebKit bugs 191362, 203302, 201180, 213510,
296698; tauri #13166, #11854, #15719; tao #1340; wry #1864, PR #1856;
plugins-workspace #3030; Mozilla bug 1509383; Capacitor storage docs & range
commit; iA Writer Dropbox post & iPhone library docs; Textastic external
folders manual; Working Copy manual; Ulysses external folders; Obsidian forum
(iCloud startup, Dropbox on iOS); Logseq #8915; CommonsWare SAF; Google Play
All-files policy; uniffi-bindgen-swift docs; cargo-swift; Ferrostar XCFramework
guide.

## Appendix B — Verification of this doc

- Each file:line claim in §2 was produced by read-only exploration of this
  worktree. Spot-check a few before relying on them.
- §10.1 spikes are the empirical verification of §3, §4 and §8. Numbers marked
  "measure" are deliberately not asserted.

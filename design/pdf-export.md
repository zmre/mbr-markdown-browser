# PDF export (plan)

Status: implemented, 2026-11-17; Windows/Linux pending CI verification.
macOS backed by experiments; the Windows and Linux backends are written from
source reading only (see [Verification status](#verification-status)).

## Goal

1. **CLI:** `mbr --pdf note.md` writes a paginated, vector (selectable text,
   clickable links) PDF of the note as mbr renders it, with no window, no Dock
   icon, no dialog. Scriptable: exit code and stderr say what happened.
2. **GUI:** File → **Save as PDF…** (beside Print…) asks for a file name and
   writes the same PDF silently — no print panel.
3. **Cross-platform:** macOS, Windows, Linux desktop.
4. **No new runtime dependency and no new crate:** the OS webview mbr already
   links through wry (WKWebView, WebView2, WebKitGTK) does the layout and the
   PDF. Chrome/Chromium, pandoc, Typst and pure-Rust HTML engines were ruled
   out in the earlier research (fidelity: mbr pages are finished by JS —
   mermaid, KaTeX, hljs, Lit components; or size).

Non-goals for now: headless Linux / CI servers (no display → clear message,
exit non-zero), the server (`-s`) producing PDFs over HTTP, static builds,
iOS. Revisit on demand.

## Decisions (2026-11-17)

| Question                     | Decision                                                                                                   |
|------------------------------|------------------------------------------------------------------------------------------------------------|
| Internal links in the PDF    | **Removed** (text kept). `link_base` option rewrites them onto a published site instead.                    |
| GUI accelerator              | **Cmd+Shift+P** on macOS, **Ctrl+Alt+P** on Windows/Linux (Ctrl+Shift+P is Print there).                    |
| Editor open                  | **Save as PDF… is disabled** (greyed) while the in-browser editor is open.                                  |
| Windows/Linux verification   | **CI runners** for now: Windows job runs the real export; Linux job runs it under `xvfb-run` (CI only — headless stays unsupported for users). Manual testing later. |

## Page ⇄ Rust contract (binding for both halves)

All names below are referenced from Rust string literals; each side pins
them with a test.

- **Export flag**, injected by Rust with `with_initialization_script` before
  any page script:
  `window.__MBR_EXPORT__ = {"links": "strip" | {"base": "<url ending in />"}, "paperWidthIn": <f64>, "paperHeightIn": <f64>, "margin": "<CSS margin shorthand>"}`
  (`paperWidthIn`/`paperHeightIn` already account for orientation).
- **Export stylesheet** `/.mbr/export.css` (compiled-in default, overridable
  in `.mbr/`): loaded by the page only in export mode; plus a generated
  `<style id="mbr-export-page">@page { size: Win Hin; margin: M }</style>`.
- **Ready**: `window.mbrExportReady(): Promise<{warnings: string[]}>`; the
  page starts it itself when the flag is set and posts exactly one of
  `window.ipc.postMessage("mbr:export-ready:" + JSON.stringify({warnings}))` or
  `window.ipc.postMessage("mbr:export-failed:" + message)`. Caps: 10 s for
  images, 15 s for mermaid/math/hljs settle — a cap that fires becomes a
  warning, not a failure. No `requestAnimationFrame` anywhere on this path.
- **Editor state** (GUI windows only, `isGuiMode()`): on every
  `EDITOR_STATE_EVENT` the page posts `mbr:editor-state:open` or
  `mbr:editor-state:closed`. Rust enables Save as PDF… on each new page load
  and disables it on `open`.
- Payloads are page-controlled: Rust parses them strictly, caps sizes, and
  treats warnings as text only.

## Approach in one paragraph

Load the page in a **hidden webview** from an in-process server, inject an
"export" flag before any page script runs so the components render for paper
(light theme, eager images, no idle-time deferral), wait for a page-side
**export-ready** signal over IPC, then call the platform's silent
print-to-PDF API with mbr's paper settings, verify the file, and either exit
(CLI) or toast (GUI). The GUI uses the same hidden webview pointed at the
page on screen, so the visible window is never touched.

## Experiment results (macOS, 2026-11-17)

Spike: a standalone crate (wry 0.57.1, tao 0.37.2, objc2 0.6.5,
objc2-{foundation,app-kit,web-kit} 0.3.2, mbr's `Cargo.lock`) against a running
`mbr -s`. Inspected with pdfinfo/pdftotext/pdffonts/pdftoppm and pypdf.

| #  | Question                                   | Result                                                                                                                                                                                                                                       |
|----|--------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| E1 | Paginated export from a hidden CLI window  | **Works**: `printOperationWithPrintInfo:` + `NSPrintSaveJob` + `runOperationModalForWindow:…` on a never-shown tao window; delegate fires 35–230 ms later on the main thread. Synchronous `runOperation()` **never returns and writes unbounded blank pages** (1 GB/min). `createPDF` = screen media, one tall page: unusable. |
| E1 | Dock icon / focus                          | Activation policy Accessory (or Prohibited): no Dock icon, focus not stolen. Regular: Dock icon.                                                                                                                                            |
| E2 | Output                                     | Vector, embedded fonts, real text; code highlighted, tables, images, mermaid (vector), KaTeX fonts; chrome hidden by print CSS; light even with system dark mode. Links are real `/Link` `/URI` annotations.                                   |
| E2 | Margins                                    | `NSPrintInfo` margins are **ignored**. With the theme's `@page { margin: 0 }`, pages 2+ have **no top/bottom margin** and long notes get a trailing blank page. Injecting `@page { margin: 0.6in 0.5in }` + zero `main.container` vertical padding fixes both, and the old WKWebView "top band" did **not** appear on this path. |
| E2 | Paper / orientation                        | `setPaperSize` works (Letter 612×792, A4 595.28×841.89), `setOrientation(Landscape)` works.                                                                                                                                                   |
| E3 | Readiness                                  | Exporting at load-finished prints raw mermaid source, raw TeX, no images. In a hidden window **lazy images never load** and **`requestAnimationFrame` never fires** (`visibilityState === 'hidden'`). Forcing `loading=eager` + awaiting images, fonts, mermaid `svg`, `data-mbr-math` works; hljs adds ~1.4 s on big pages (idle-time scheduling). |
| E4 | `beforeprint` / dark mode                  | `beforeprint`/`afterprint` fire during the native operation. Output light in every appearance. Mermaid's dark→light re-render on `beforeprint` worked 5/5 but is async — timing, not design.                                                     |
| E5 | Headers / footers / page numbers           | `NSPrintHeaderAndFooter = YES` draws title, date, "Page N of M" (inside the margin when the `@page` CSS above is used). CSS `@page` margin boxes (`@top-center`, `counter(page)`) do nothing.                                                  |
| E6 | Timing (release, hidden)                   | Process start → PDF: short note 0.42–0.74 s; 60-section note (13 pages) 0.75–0.82 s; `reference/configuration` (33 pages) 2.56 s, of which ~1.4 s is the hljs idle wait. Print operation itself 35–730 ms.                                       |
| E7 | Silent export of the *visible* window      | No panel, frame unchanged, scroll kept — but the live DOM is in print layout for ~200 ms and a dark-mode mermaid diagram re-renders light then dark (possible flicker). Not visually confirmed (no screen-recording permission).              |
| E8 | Robustness                                 | 5× runs stable (±10 ms). Unwritable directory → delegate `success = NO`, no file, AppKit logs that it tried to show an alert. **Missing directory → `success = YES` and no file.** Relative paths → CFURL warnings.                         |
| E9 | Several pages, one webview                 | `load_url` → ready → export, 5 pages in 2.39 s, all correct.                                                                                                                                                                                 |

Conclusions that shape the design:

- **Never** `runOperation()`; always the modal-for-window variant + delegate.
- Readiness must be **page-driven** (forced eager images, no rAF, no idle
  deferral) — a load event is not enough, and a hidden view is throttled.
- Page margins come from **export CSS**, not the platform API.
- Success flags lie: **write to a temp file in the target directory, verify
  it (exists, non-empty, starts `%PDF-`), then rename.**
- Exporting the visible webview disturbs the page; a **separate hidden
  webview** avoids it and is the same code as the CLI.

## Architecture

### Components

| Piece                     | Where                                              | What                                                                                                                                                                                                                       |
|---------------------------|----------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `pdf_export` module       | `src/pdf_export/{mod,macos,windows,linux}.rs` (root package, `gui` feature) | `PdfOptions` (paper, orientation, margins, header/footer), `PdfJob` state machine (load → ready → print → verify → rename), one `print_to_pdf(webview, &PdfOptions, tmp_path, done)` per platform. All `unsafe`/FFI lives in the platform files. |
| Export webview            | `pdf_export/mod.rs`                                | Builds a hidden window + webview with `with_initialization_script(EXPORT_FLAG_SCRIPT)`, an IPC handler for `mbr:export-ready`/`mbr:export-failed`, a page-load handler; light appearance forced (macOS `NSAppearance aqua`, WebView2 `PreferredColorScheme = Light`, GTK `prefer-dark-theme = false`). |
| CLI runner                | `src/pdf_cli.rs` (`gui` + `cli`)                   | Reads config like server mode (`cli::read_config(path, gui = false, …)` → editing off), starts the server in-process on a loopback ephemeral port, resolves the note URL with `launch_url`, runs a tao event loop (Accessory policy on macOS) with one `PdfJob`, exits with its result. |
| GUI integration           | `src/browser.rs`                                   | `Save as PDF…` menu item + `Shortcut::SaveAsPdf`; rfd save dialog (default name from the page title / note stem, `.pdf`); spawns a `PdfJob` for `webview.url()` on the running server; on completion a toast in the page (success) or an rfd message dialog (failure). One job at a time. |
| Front-end export mode     | `components/src/export-ready.ts` (main bundle) + small hooks in `mbr-mermaid.ts`, `mbr-katex.ts`, the hljs loader | When `window.__MBR_EXPORT__` is set: images `loading=eager`; mermaid renders the light theme directly (no `beforeprint` swap); hljs highlights immediately rather than at idle; live reload, find bar, review FAB etc. stay out of the way (print CSS already hides chrome). Exposes the ready promise and posts `mbr:export-ready` over `window.ipc`. |
| Export CSS                | `crates/mbr-core/templates/export.css` (served as `/.mbr/export.css`, overridable in `.mbr/`) | Injected by `export-ready.ts` only in export mode: `@page { size; margin }` from options, `main.container` vertical padding 0. Kept out of theme.css so the print panel's tuned behaviour (the top-band workaround) is untouched. |

### Export-ready contract (page ⇄ Rust)

- Rust injects, before any page script: `window.__MBR_EXPORT__ = {paper, orientation, margins, links}` (JSON of `PdfOptions`).
- Page resolves `window.mbrExportReady()` when **all** of:
  1. every `img` complete (after forcing eager), each capped (default 10 s; a stuck image is reported, not fatal);
  2. `document.fonts.ready`;
  3. `mermaidSettled()`, `mathSettled()`, and a new `hljsSettled()`;
  4. export CSS applied.
  Uses `setTimeout`, never `requestAnimationFrame` (dead in a hidden view).
- Then posts `mbr:export-ready:<json of warnings>`; on an exception, `mbr:export-failed:<message>`.
- Rust timeout (default 30 s, `--pdf-timeout-ms`): export anyway, warn on stderr.
- The global name and IPC prefixes are referenced from Rust string literals, so they are pinned by a test on each side (as `mbrCopyPage` is).

### Links

Internal links in the PDF would point at `http://127.0.0.1:<ephemeral port>/…`,
dead the moment the export ends. Policy (applied by `export-ready.ts`, chosen
by `PdfOptions::links`):

- **default `strip`**: same-origin links lose their `href` (text kept, styled
  as text); external `http(s)`/`mailto` links kept; same-page `#fragment`
  links kept as fragments (whether WebKit/Chromium/Cairo turn them into
  internal destinations is to be tested; worst case they become no-ops).
- **`base=<url>`** (`--pdf-link-base https://notes.example.com/`, config
  `pdf.link_base`): same-origin links rewritten onto that base — for repos
  also published as a static site.
- `/.mbr/outside/?path=…` links are stripped either way.

### Platform backends

**macOS (verified):** `NSPrintInfo::new()`, `setPaperSize`, `setOrientation`,
margins 0, `setJobDisposition(NSPrintSaveJob)`, `dictionary()[NSPrintJobSavingURL] =
NSURL::fileURLWithPath(absolute tmp path)`, optional `NSPrintHeaderAndFooter`;
`printOperationWithPrintInfo:`, `setShowsPrintPanel(false)`,
`setShowsProgressPanel(false)`,
`runOperationModalForWindow:delegate:didRunSelector:contextInfo:` with a
`define_class!` delegate (`printOperationDidRun:success:contextInfo:`) that
posts a `UserEvent`. Keep the operation and delegate alive until then.
New objc2-app-kit features: `NSPrintInfo`, `NSPrintOperation`, `NSPrinter`,
`NSPrintPanel`, `NSAppearance` (no new crates; objc2-web-kit is already
pulled in by wry — add it as a direct macOS dependency at the locked version).

**Windows (from source, untested):** `wry::WebViewExtWindows::webview()`
`.cast::<ICoreWebView2_7>()`, `environment().cast::<ICoreWebView2Environment6>()`,
`CreatePrintSettings()` → page width/height (inches), orientation, margins 0,
`SetShouldPrintBackgrounds(true)`, `SetShouldPrintHeaderAndFooter(opt)`;
`PrintToPdf(absolute path, settings, PrintToPdfCompletedHandler::create(…))`
— the handler posts a `UserEvent` (do **not** use `wait_for_async_operation`,
which pumps the message loop under tao). Runs on the UI thread. Direct
Windows deps `webview2-com = "0.39"` and `windows = "0.62"` (both already in
the lockfile via wry). `E_NOINTERFACE` → "WebView2 runtime too old". Hidden
WebView2 throttles like WebKit; if `IsVisible = false` misbehaves, fall back
to a shown, off-screen (`-32000,-32000`), non-activated window.

**Linux (from source, untested):** call `gtk::init()` **before** tao's
`EventLoop::new` (which panics without a display); on `Err`, print the
headless message and exit 2. Build the export webview into a
`gtk::OffscreenWindow` (`build_gtk` accepts any container) and `show_all()`
it, so the view is realized/mapped without appearing. Then
`webkit2gtk::PrintOperation::new`, `gtk::PrintSettings` with printer
`glib::dgettext(Some("gtk30"), "Print to File")` (the name is translated),
`PRINT_SETTINGS_OUTPUT_URI` = `file://…`, `OUTPUT_FILE_FORMAT` = `pdf`, paper
size, `gtk::PageSetup` with zero margins; `connect_failed` (fires before
`finished`) + `connect_finished` → `UserEvent`; `print()` (no dialog). The
EGL issue in CLAUDE.md ("EGL on non-NixOS hosts") applies to this web
process too.

### CLI

```
mbr --pdf [--output <FILE|DIR|->] [--pdf-paper letter|a4|legal|<W>x<H>mm|in]
          [--pdf-landscape|--pdf-portrait] [--pdf-margin <CSS length>]
          [--pdf-page-numbers|--no-pdf-page-numbers]
          [--pdf-link-base <URL>] [--pdf-timeout-ms <MS>] <NOTE>
```

- `--pdf` joins the `mode` ArgGroup (exclusive with `-s`/`-g`/`-b`/…);
  compiled only with `gui` (it needs the webview). Without `gui` the flag
  does not exist, like `--extract-*` without `media-metadata`.
- `--output`: today `PathBuf` with default `build`. Make it
  `Option<PathBuf>` resolved per mode (`build` for `-b`; for `--pdf`:
  absent → `./<note stem>.pdf`; an existing directory → `<dir>/<stem>.pdf`;
  `-` → PDF bytes on stdout, logs stay on stderr). Pure function
  `pdf_output_path(note, output, cwd)`, unit-tested.
- `<NOTE>`: a markdown file (or any path mbr can open; a folder exports its
  page — listing or index note — which is cheap to allow).
- Exit codes: 0 written; 1 export failed once it ran (message on stderr:
  cause, path); 2 refused before starting — anything found before the export
  window exists: bad flags, a missing note, bad config, an output folder that
  is missing or not writable, no display on Linux, no backend.
- Logging: same `-v`/`-q` as every mode; quiet by default except errors and
  warnings (stuck image, readiness timeout).
- Paper default: the platform's default (macOS shared print info / region,
  Windows Letter for en-US else A4 via locale, GTK default paper size) unless
  `pdf.paper` or the flag says otherwise.

Config (`[pdf]` table, `MBR_PDF__*`, `deny_unknown_fields` like `[slides]`):
`paper`, `orientation`, `margin`, `page_numbers`, `link_base`,
`timeout_ms`. Flags beat config (usual hierarchy), in both directions: the
on/off and orientation switches come in conflicting pairs
(`--pdf-landscape`/`--pdf-portrait`, `--pdf-page-numbers`/`--no-pdf-page-numbers`).
Applies to both CLI and GUI.

### GUI

- File menu: `Save as PDF…` after `Print…`, Cmd+Shift+P (macOS) /
  Ctrl+Alt+P (Windows, Linux). Also in the Linux keyboard-route
  table (`linux_shortcut_for`), via `perform_shortcut` like every action.
- Default file name: `<page title>.pdf` (sanitized), directory: last used,
  else Documents. rfd runs off the event loop like the Open… picker.
- Progress: a toast in the visible page ("Saving PDF…" → "Saved
  note.pdf"); failure → rfd message dialog with the cause. **v1 has no
  Reveal in Finder/Explorer button on the success toast**; adding one through
  the existing `reveal` plumbing is a follow-up. A second Save as PDF… while
  one runs only shows a toast (no error type for it).
- The job uses the visible page's current URL on the running `LiveServer`,
  so unsaved editor content would not be included — hence the item is
  disabled while the editor is open (`mbr:editor-state:*`).

## Front-end work

- `export-ready.ts` (main bundle, no custom element): reads
  `window.__MBR_EXPORT__`, forces eager images, applies export CSS +
  link policy, awaits settled promises with caps, posts IPC. Exports
  `mbrExportReady` on `window`.
- `mbr-mermaid.ts`: in export mode render with the light theme from the
  start (no dark→light swap); `mermaidSettled()` unchanged.
- hljs loader: add `hljsSettled()`; in export mode highlight immediately
  instead of `scheduleIdleTask` (saves ~1.4 s on big pages).
- `mbr-katex.ts`: nothing beyond `mathSettled()`.
- Anything that schedules work with `requestAnimationFrame` and matters on
  paper must have a `setTimeout` path in export mode (audit: genealogy charts
  near-viewport loading, calendar companion, mini graph — most are hidden by
  print CSS anyway; the relationship charts below a person note are
  near-viewport lazy and should be forced or deliberately left out).
- `<details>`/definition-list FAQ reveal: print CSS already opens them.

## Tests

| Layer              | Test                                                                                                                                                                         |
|--------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Rust unit          | `PdfOptions` from config + flags (precedence, paper parsing incl. custom sizes, bad values refused); `pdf_output_path`; Linux preflight as a pure function of (env, gtk init result); IPC message parsing (`mbr:export-ready`/`failed`, page-controlled payload treated as untrusted); temp-path + verify + rename helper (missing dir, not `%PDF-`, empty). |
| CLI parsing        | `--pdf` in the mode group; conflicts; `--output` per mode (build default unchanged — regression test).                                                                        |
| TS (vitest)        | `export-ready.ts`: eager forcing, caps, link policies (strip / base / outside), ready vs failed message, no rAF used; mermaid light-in-export; `hljsSettled`. Pin `mbrExportReady` and the IPC prefixes against the Rust literals (as `mbr-find-bar.test.ts` / copy-page do). |
| Integration (real) | `tests/pdf_export.rs`, `required-features = ["gui", "cli"]`: exports a fixture note (code, table, mermaid, math, lazy image far below the fold, internal + external link) and checks with `lopdf` (`load_mem`): starts `%PDF-`, page count, extracted text contains markers from top/bottom and from the mermaid label and the math, no `127.0.0.1` URI annotations. Runs on macOS locally and in macOS CI; Windows CI once the backend lands; skipped (with a printed reason) on Linux without a display. Gate with `MBR_PDF_TESTS=1` until CI runners are proven to allow a hidden webview. |
| Manual             | Visual check of the PNG render of the fixture per platform; dark-mode system; A4/Letter/landscape; header/footer on; a 1,800-line note (timing); GUI save into an unwritable folder (error surfaces once, no stray AppKit alert). |

## Docs and housekeeping

- `docs/features/pdf-export.md` (new; **Works in:** GUI, CLI — macOS,
  Windows, Linux desktop; not server, static builds, QuickLook, iOS) + row in
  `docs/features/index.md`'s matrix.
- `docs/reference/cli.md` (`--pdf` and its flags, exit codes, `--output`
  per mode), `docs/reference/configuration.md` (`[pdf]`), `docs/modes/gui.md`
  (menu item), `docs/reference/shortcuts.md` if an accelerator is added,
  `docs/customization/` (`.mbr/export.css` override).
- CLAUDE.md: CLI table, modules table (`pdf_export`, `pdf_cli`,
  `export-ready.ts`), Cargo Features (`--pdf` needs `gui`).
- `cargo fmt --all`, `cargo sort --workspace --grouped --no-format`, clippy
  `--all-targets` incl. `--no-default-features` and the Windows feature set,
  `cargo test --workspace`, `bun run test`.

## Phases

| Phase | Scope                                                                                     | Done when                                                                                      |
|-------|-------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------|
| 1     | Front-end export mode (`export-ready.ts`, mermaid light, `hljsSettled`, export CSS, link policy) | vitest green; manual: page with `__MBR_EXPORT__` set in a browser posts ready with everything rendered |
| 2     | `pdf_export` core + macOS backend + `mbr --pdf` CLI + `[pdf]` config                     | integration test green on macOS; docs for CLI/config                                           |
| 3     | GUI `Save as PDF…` (hidden export webview, save dialog, toast/error)                      | manual on macOS: no disturbance of the visible page; errors surface once                       |
| 4     | Windows backend                                                                           | built and run on a Windows machine; integration test on Windows CI                             |
| 5     | Linux backend + no-display preflight                                                      | run on a Linux desktop (X11 and Wayland); headless prints the message, exits 2                 |
| 6     | Docs, CLAUDE.md, feature matrix                                                           | `tests/docs_anchors.rs` green                                                                  |

Phases 4 and 5 can run in parallel with 3 once phase 2's `print_to_pdf` seam
exists.

## Verification status

| Claim                                              | macOS                   | Windows                          | Linux                                 |
|----------------------------------------------------|-------------------------|----------------------------------|---------------------------------------|
| Silent paginated export from a hidden view         | **verified** (spike)    | API confirmed from crate source  | API confirmed from crate source       |
| Vector text, embedded fonts                        | **verified**            | expected (Skia PDF)              | expected (Cairo PDF)                  |
| Clickable links                                    | **verified** (URI)      | expected                         | uncertain (Cairo link tags)           |
| `beforeprint` fires                                | **verified**            | expected                         | uncertain — not relied on             |
| Page margins from CSS `@page`                      | **verified**            | expected                         | uncertain (GtkPageSetup margins add)  |
| Paper size / orientation from the API             | **verified**            | from source                      | from source                           |
| Header/footer/page numbers                         | **verified** (fixed format) | `SetShouldPrintHeaderAndFooter` | none built in                         |
| Hidden view renders lazy images                    | no → forced eager       | likely no → forced eager         | unmapped view: no → `OffscreenWindow` |
| No-display behaviour                               | n/a                     | n/a                              | tao panics → preflight `gtk::init()`  |

## Open questions / risks

1. **Hidden-view throttling on Windows/Linux** beyond what the readiness
   script handles (timers in components). Mitigation: off-screen *shown*
   window; to be measured in phases 4–5.
2. **Same-page `#fragment` links** may not become internal PDF destinations
   on any platform; acceptable for v1.
3. **Page numbers** are only available as the platform's fixed header/footer
   (macOS, Windows; none on Linux). CSS margin boxes don't work in WebKit.
   Option `page_numbers` therefore documents per-platform behaviour.
4. **No PDF outline/bookmarks** from any backend. Possible later: post-process
   with lopdf using heading positions (needs a page map; non-trivial).
5. **Theme override repos**: a repo with its own `.mbr/theme.css` keeps its
   own print CSS; export CSS only adds `@page` margins and padding reset, and
   is itself overridable.
6. **Fonts**: the PDF embeds whatever the webview used, so it matches the
   preview, not a fixed font set — expected for "export what I see".
7. **E7 flicker** is moot with the separate export webview, but that webview
   costs a second page load (~0.3–2.5 s) per export — acceptable for an
   explicit menu action.
8. Headless Linux/CI: out of scope; candidates later are `xvfb-run`
   (documented workaround, no code) or WPE WebKit.

## Spike artifacts

Throwaway, not in the repo: `/tmp/mbr-pdf-spike/` (source `src/main.rs`,
vault, `out/` PDFs, page renders and montages, logs).

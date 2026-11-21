//! PDF export: a page as mbr renders it, printed to a paginated, vector PDF
//! by the operating system's own webview (WKWebView, WebView2, WebKitGTK).
//!
//! Used by `mbr --pdf` ([`crate::pdf_cli`]) and the GUI's **Save as PDF…**
//! ([`crate::browser`]); both run the same [`ExportSession`]:
//!
//! 1. a **hidden** window and webview load the page from mbr's own server,
//!    with [`PdfOptions::init_script`] setting `window.__MBR_EXPORT__` before
//!    any page script runs (`components/src/export-ready.ts` reads it: light
//!    theme, eager images, the `@page` size and margin, the link policy);
//! 2. the page posts `mbr:export-ready:` (or `mbr:export-failed:`) over IPC
//!    once everything that matters on paper has rendered — a load event is not
//!    enough, and a hidden view never runs `requestAnimationFrame`. Pages
//!    rendered with mbr's templates do this from the main bundle; a custom
//!    page template without it includes `/.mbr/_export_ready.html`;
//!    [`ReadySignal::Load`] signals on `load` plus fonts instead.
//!    [`PdfOptions::timeout`] exports anyway, with a warning;
//! 3. a page whose own stylesheets declare `@page { size }` reports that size
//!    in its ready message, and the sheet is printed on it
//!    ([`PdfOptions::with_page_size`]); its own `@page` margins likewise win
//!    over [`PdfOptions::margin`] (the page leaves them out of the generated
//!    rule). WebKit's print system ignores `@page { size }`, so the paper
//!    has to be set natively.
//! 4. the platform backend prints to a **temporary file in the target
//!    folder** ([`PendingOutput`]), which is then verified (exists, non-empty,
//!    starts `%PDF-`) and renamed into place — the print systems' success flags
//!    are not trustworthy (macOS reports success for a folder that is gone).
//!
//! The visible GUI window is never printed: putting a live page into print
//! layout disturbs it (measured: ~200 ms of print layout, a dark-mode diagram
//! re-rendered twice).
//!
//! # Platform seam
//!
//! Everything platform-specific (and every `unsafe`) is in `macos.rs`,
//! `windows.rs` and `linux.rs`, which each provide the same items:
//!
//! - `BACKEND_AVAILABLE` — whether `print_to_pdf` is implemented;
//! - `preflight()` — called before the event loop exists (Linux: `gtk::init`,
//!   so a missing display is a clear message rather than tao's panic);
//! - `configure_cli_event_loop(&mut EventLoop)` — the CLI's event loop (macOS:
//!   Accessory activation policy, so no Dock icon and no stolen focus);
//! - `export_window(WindowBuilder)` — the export window's platform settings
//!   (Windows: shown off-screen and never activated, because a hidden WebView2
//!   may be throttled; elsewhere unchanged, i.e. hidden);
//! - `build_webview(builder, &window)` — where the hidden webview lives (Linux
//!   builds it into a `gtk::OffscreenWindow` so it is realized and mapped
//!   without appearing);
//! - `prepare_export_view(&webview)` — force the light appearance;
//! - `default_paper()` — the paper when neither `[pdf] paper` nor a flag says;
//! - `print_to_pdf(&webview, &options, tmp_path, done) -> PrintInFlight` —
//!   start the silent print and call `done` exactly once, on the event-loop
//!   thread, when the print system is finished. `PrintInFlight` keeps whatever
//!   must live until then (macOS: the operation and its delegate).
//!
//! # From a host application
//!
//! A program that embeds mbr's server (its router merged into the host's own)
//! and owns a tao event loop exports with the same [`ExportSession`]: build
//! [`PdfOptions`], [`PendingOutput::prepare`] the target, call
//! [`ExportSession::start`] with the loop's window target and an
//! [`ExportSink`] that sends each [`ExportEvent`] back through an
//! `EventLoopProxy`, and pass every one it receives to
//! [`ExportSession::handle`] until it returns `Some`. The host's tao and wry
//! must be the versions this crate links (see the root `Cargo.toml`). See
//! docs/features/pdf-export.md, "Exporting PDFs from a host application".

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tao::dpi::LogicalSize;
use tao::event_loop::EventLoopWindowTarget;
use tao::window::{Window, WindowBuilder};
use wry::{NewWindowResponse, WebView, WebViewBuilder};

use crate::errors::PdfError;
pub use crate::pdf_config::ReadySignal;
use crate::pdf_config::{LinkBase, Margin, Orientation, Paper, PdfConfig, paper_edge_in_range};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(target_os = "windows")]
use windows as platform;

pub use platform::{BACKEND_AVAILABLE, configure_cli_event_loop, default_paper};

// ---------------------------------------------------------------------------
// Page ⇄ Rust contract (design/pdf-export.md). Each literal is also named by
// the front end (`components/src/export-ready.ts`), whose tests read these
// constants from this file, so renaming one side fails the other's tests.
// ---------------------------------------------------------------------------

/// The global the export flag is assigned to before any page script runs.
pub const EXPORT_FLAG_GLOBAL: &str = "__MBR_EXPORT__";
/// The page function that resolves once the page is ready for paper.
pub const EXPORT_READY_FUNCTION: &str = "mbrExportReady";
/// IPC prefix: the page is ready; JSON `{"warnings": [string]}` follows.
pub const IPC_EXPORT_READY_PREFIX: &str = "mbr:export-ready:";
/// IPC prefix: the page could not get ready; a message follows.
pub const IPC_EXPORT_FAILED_PREFIX: &str = "mbr:export-failed:";
/// IPC prefix (GUI windows): the in-browser editor opened or closed;
/// `open` or `closed` follows.
pub const IPC_EDITOR_STATE_PREFIX: &str = "mbr:editor-state:";
/// The id of the `<style>` holding the generated `@page` rule.
pub const EXPORT_PAGE_STYLE_ID: &str = "mbr-export-page";

/// Largest IPC message accepted from the page, in bytes. Anything bigger is
/// not a ready message this exporter sent for.
const MAX_IPC_BYTES: usize = 64 * 1024;
/// At most this many warnings are kept from a ready message.
const MAX_WARNINGS: usize = 32;
/// Each warning is cut to this many characters.
const MAX_WARNING_CHARS: usize = 300;
/// A failure message is cut to this many characters.
const MAX_FAILURE_CHARS: usize = 1000;

/// The hidden export window's size. Printing lays the page out again for the
/// paper, so this only has to be a plausible desktop viewport.
const EXPORT_VIEWPORT: LogicalSize<f64> = LogicalSize::new(1024.0, 768.0);

/// The command-line half of the options (`--pdf-*`). `None` = not given, so
/// the configuration's value stands; `Some` overrides it in either direction
/// (`--pdf-portrait` beats `orientation = "landscape"`, `--no-pdf-page-numbers`
/// beats `page_numbers = true`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PdfFlags {
    pub paper: Option<Paper>,
    pub orientation: Option<Orientation>,
    pub margin: Option<Margin>,
    pub page_numbers: Option<bool>,
    pub link_base: Option<LinkBase>,
    pub timeout_ms: Option<u64>,
    pub ready: Option<ReadySignal>,
}

/// What internal (same-origin) links become in the PDF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkPolicy {
    /// Removed; their text stays. The default: the export's server is gone
    /// the moment the export ends.
    Strip,
    /// Rewritten onto a published copy of the repository.
    Base(LinkBase),
}

/// Everything one export needs to know about paper and the page.
///
/// `paper`, `orientation` and `margin` are defaults: a page whose own
/// stylesheets declare `@page { size }` or `@page { margin }` wins
/// (docs/features/pdf-export.md, "Paper and margins set by the page").
#[derive(Debug, Clone, PartialEq)]
pub struct PdfOptions {
    pub paper: Paper,
    pub orientation: Orientation,
    pub margin: Margin,
    /// The platform's header and footer (title, date, "Page N of M").
    pub page_numbers: bool,
    pub links: LinkPolicy,
    /// How long to wait for the page's ready message before exporting anyway.
    pub timeout: Duration,
    /// What counts as ready: the page's own message, or `load` plus fonts.
    pub ready: ReadySignal,
}

impl PdfOptions {
    /// Folds the command line over the configuration (which already holds
    /// `.mbr/config.toml` under `MBR_PDF__*`): flags > environment > file.
    /// `platform_default` is asked only when neither names a paper.
    pub fn resolve(
        config: &PdfConfig,
        flags: &PdfFlags,
        platform_default: impl FnOnce() -> Paper,
    ) -> Self {
        let paper = flags
            .paper
            .or(config.paper)
            .unwrap_or_else(platform_default);
        let orientation = flags.orientation.unwrap_or(config.orientation);
        let links = match flags.link_base.as_ref().or(config.link_base.as_ref()) {
            Some(base) => LinkPolicy::Base(base.clone()),
            None => LinkPolicy::Strip,
        };
        PdfOptions {
            paper,
            orientation,
            margin: flags
                .margin
                .clone()
                .unwrap_or_else(|| config.margin.clone()),
            page_numbers: flags.page_numbers.unwrap_or(config.page_numbers),
            links,
            timeout: Duration::from_millis(flags.timeout_ms.unwrap_or(config.timeout_ms)),
            ready: flags.ready.unwrap_or(config.ready),
        }
    }

    /// These options with the sheet a page declared in its own
    /// `@page { size }` (orientation already applied, as written), which wins
    /// over [`PdfOptions::paper`] and [`PdfOptions::orientation`]. `None`
    /// leaves them as they are.
    pub fn with_page_size(&self, page: Option<PageSize>) -> PdfOptions {
        let mut options = self.clone();
        if let Some(page) = page {
            options.paper = Paper::from_dimensions_in(page.width_in, page.height_in);
            // `from_dimensions_in` names a turned standard sheet by its
            // portrait name; landscape puts the sides back as declared.
            let (width, height) = options.paper.dimensions_in();
            options.orientation = if (width > height) == (page.width_in > page.height_in) {
                Orientation::Portrait
            } else {
                Orientation::Landscape
            };
        }
        options
    }

    /// The page's width and height in inches, orientation applied: landscape
    /// swaps the paper's sides as written.
    pub fn page_size_in(&self) -> (f64, f64) {
        let (width, height) = self.paper.dimensions_in();
        match self.orientation {
            Orientation::Portrait => (width, height),
            Orientation::Landscape => (height, width),
        }
    }

    /// The sheet as print systems that take a paper size *plus* an orientation
    /// want it (AppKit, GTK): the paper's portrait sides and whether to turn
    /// it. A page wider than it is tall is a turned sheet, whether it got that
    /// way by `landscape` or by a custom paper written wide.
    pub fn sheet(&self) -> Sheet {
        let (width, height) = self.page_size_in();
        if width > height {
            Sheet {
                width_in: height,
                height_in: width,
                orientation: Orientation::Landscape,
            }
        } else {
            Sheet {
                width_in: width,
                height_in: height,
                orientation: Orientation::Portrait,
            }
        }
    }

    /// The export flag, as the JSON the page reads from `window.__MBR_EXPORT__`.
    pub fn export_flag_json(&self) -> String {
        // Four decimals: a hundredth of a millimetre, and `@page` sizes that
        // read as written (`8.5in`, not `8.499999999in`).
        let round = |inches: f64| (inches * 10_000.0).round() / 10_000.0;
        let (width, height) = self.page_size_in();
        let links = match &self.links {
            LinkPolicy::Strip => serde_json::Value::from("strip"),
            LinkPolicy::Base(base) => serde_json::json!({ "base": base.as_str() }),
        };
        serde_json::json!({
            "links": links,
            "paperWidthIn": round(width),
            "paperHeightIn": round(height),
            "margin": self.margin.as_css(),
            "ready": self.ready.as_str(),
        })
        .to_string()
    }

    /// The initialization script that sets the export flag before any page
    /// script runs. JSON is valid JavaScript, and every string in it was
    /// validated by [`crate::pdf_config`] — serde_json escapes the rest.
    ///
    /// With [`ReadySignal::Load`] it also posts the ready message itself, on
    /// `load` and then `document.fonts.ready`; the page's own scripts see
    /// `ready: "load"` and stay out of it.
    pub fn init_script(&self) -> String {
        let flag = format!("window.{EXPORT_FLAG_GLOBAL} = {};", self.export_flag_json());
        match self.ready {
            ReadySignal::Page => flag,
            ReadySignal::Load => format!("{flag}\n{}", load_ready_script()),
        }
    }
}

/// The `ready = "load"` signal: once the page has loaded and its fonts are
/// ready, post an empty ready message. No `requestAnimationFrame` (a hidden
/// view never runs one). Posted at most once.
fn load_ready_script() -> String {
    let message = serde_json::Value::from(format!("{IPC_EXPORT_READY_PREFIX}{{\"warnings\":[]}}"));
    LOAD_READY_SCRIPT.replace("READY_MESSAGE", &message.to_string())
}

/// [`load_ready_script`]'s source; `READY_MESSAGE` becomes the message as a
/// JSON string literal.
const LOAD_READY_SCRIPT: &str = r#"(function () {
  var sent = false;
  function post() {
    if (sent) return;
    sent = true;
    var ipc = window.ipc;
    if (ipc && typeof ipc.postMessage === 'function') ipc.postMessage(READY_MESSAGE);
  }
  window.addEventListener('load', function () {
    var fonts = document.fonts;
    (fonts && fonts.ready ? fonts.ready : Promise.resolve()).then(post, post);
  }, { once: true });
})();"#;

/// A sheet size a page declared in its own `@page { size }`, in inches, as
/// laid out (orientation applied).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub width_in: f64,
    pub height_in: f64,
}

/// A sheet of paper for a print system that turns it itself
/// ([`PdfOptions::sheet`]): portrait sides, `width_in <= height_in`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sheet {
    pub width_in: f64,
    pub height_in: f64,
    pub orientation: Orientation,
}

/// A message the export page posted over IPC.
#[derive(Debug, Clone, PartialEq)]
pub enum ExportMessage {
    /// Ready for paper, with anything that did not finish in time, and the
    /// sheet the page's own `@page { size }` declares, if it does.
    Ready {
        warnings: Vec<String>,
        page: Option<PageSize>,
    },
    /// The page could not prepare itself.
    Failed { message: String },
}

/// Parses an export page's IPC message. `None` for anything that is not one.
///
/// The payload is page-controlled, so it is read strictly and kept as text
/// only: size-capped, control characters removed, each warning and message
/// cut short. A ready message whose JSON does not parse still means "ready"
/// (the page is the one sending it) and carries a warning saying so.
pub fn parse_export_message(body: &str) -> Option<ExportMessage> {
    if body.len() > MAX_IPC_BYTES {
        return None;
    }
    if let Some(json) = body.strip_prefix(IPC_EXPORT_READY_PREFIX) {
        let (mut warnings, page) = match serde_json::from_str::<serde_json::Value>(json) {
            Ok(serde_json::Value::Object(map)) => {
                let warnings = match map.get("warnings") {
                    Some(serde_json::Value::Array(items)) => items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(|w| page_text(w, MAX_WARNING_CHARS))
                        .filter(|w| !w.is_empty())
                        .take(MAX_WARNINGS)
                        .collect(),
                    None => Vec::new(),
                    Some(_) => vec!["the page sent malformed export warnings".to_owned()],
                };
                (warnings, map.get("page").map(parse_page_size))
            }
            _ => (
                vec!["the page sent a malformed export-ready message".to_owned()],
                None,
            ),
        };
        let page = match page {
            Some(Some(size)) => Some(size),
            Some(None) => {
                warnings.push(
                    "the page declared an @page size mbr cannot print on; used the configured paper"
                        .to_owned(),
                );
                None
            }
            None => None,
        };
        return Some(ExportMessage::Ready { warnings, page });
    }
    if let Some(message) = body.strip_prefix(IPC_EXPORT_FAILED_PREFIX) {
        let message = page_text(message, MAX_FAILURE_CHARS);
        return Some(ExportMessage::Failed {
            message: if message.is_empty() {
                "no reason given".to_owned()
            } else {
                message
            },
        });
    }
    None
}

/// A ready message's `page`: `{"widthIn": w, "heightIn": h}`, each side a
/// paper edge mbr accepts (1–200 in). `None` for anything else.
fn parse_page_size(value: &serde_json::Value) -> Option<PageSize> {
    let side = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_f64)
            .filter(|inches| inches.is_finite() && paper_edge_in_range(*inches))
    };
    Some(PageSize {
        width_in: side("widthIn")?,
        height_in: side("heightIn")?,
    })
}

/// Whether the in-browser editor is open (GUI windows' IPC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorState {
    Open,
    Closed,
}

/// Parses `mbr:editor-state:open` / `mbr:editor-state:closed`, exactly.
pub fn parse_editor_state(body: &str) -> Option<EditorState> {
    match body.strip_prefix(IPC_EDITOR_STATE_PREFIX)? {
        "open" => Some(EditorState::Open),
        "closed" => Some(EditorState::Closed),
        _ => None,
    }
}

/// Page-supplied text made safe to print on a terminal or in a dialog:
/// control characters become spaces, runs of whitespace collapse, and the
/// result is cut to `max_chars` (with `…`).
fn page_text(raw: &str, max_chars: usize) -> String {
    let collapsed = raw
        .split(|c: char| c.is_control() || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.chars().count() <= max_chars {
        collapsed
    } else {
        let cut: String = collapsed
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect();
        format!("{cut}…")
    }
}

// ---------------------------------------------------------------------------
// Output: temp file in the target folder → verify → rename.
// ---------------------------------------------------------------------------

/// Where the PDF goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputTarget {
    /// This file (absolute), replaced if it exists.
    File(PathBuf),
    /// Standard output (`--output -`).
    Stdout,
}

impl std::fmt::Display for OutputTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputTarget::File(path) => write!(f, "{}", path.display()),
            OutputTarget::Stdout => f.write_str("standard output"),
        }
    }
}

/// A PDF on its way to its [`OutputTarget`]: the platform prints to
/// [`PendingOutput::temp_path`], [`PendingOutput::finish`] verifies it and
/// moves it into place. The temporary file is removed on drop whatever
/// happened.
#[derive(Debug)]
pub struct PendingOutput {
    target: OutputTarget,
    temp: PathBuf,
}

/// Distinguishes temporary names made in the same process and nanosecond.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl PendingOutput {
    /// Checks the target's folder exists and is writable — by creating the
    /// temporary file there — before anything is rendered, so a bad path is a
    /// clear error rather than a print system's silent success.
    ///
    /// The temporary file sits beside the target (or in the system temporary
    /// folder for standard output), so the final rename never crosses a
    /// filesystem. It is removed again before printing: the print systems
    /// create the file themselves.
    pub fn prepare(target: OutputTarget) -> Result<Self, PdfError> {
        let (dir, stem) = match &target {
            OutputTarget::File(path) => (
                path.parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
                path.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
            OutputTarget::Stdout => (std::env::temp_dir(), "stdout".to_owned()),
        };
        match std::fs::metadata(&dir) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(PdfError::OutputDirNotDirectory { dir }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(PdfError::OutputDirMissing { dir });
            }
            Err(source) => return Err(PdfError::OutputDirNotWritable { dir, source }),
        }
        let temp = create_unique_temp(&dir, &stem)?;
        // Ours, just created: nothing else can have it open.
        let _ = std::fs::remove_file(&temp);
        Ok(PendingOutput { target, temp })
    }

    /// Where the platform should write the PDF (absolute when the target is).
    pub fn temp_path(&self) -> &Path {
        &self.temp
    }

    pub fn target(&self) -> &OutputTarget {
        &self.target
    }

    /// Verifies the temporary file is a PDF and moves it to the target — or,
    /// for [`OutputTarget::Stdout`], copies it to `stdout`. Returns its size.
    pub fn finish(self, stdout: &mut dyn Write) -> Result<u64, PdfError> {
        let bytes = verify_pdf(&self.temp)?;
        match &self.target {
            OutputTarget::File(path) => {
                std::fs::rename(&self.temp, path).map_err(|source| PdfError::Finalize {
                    path: path.clone(),
                    source,
                })?;
            }
            OutputTarget::Stdout => {
                let pdf = std::fs::read(&self.temp).map_err(PdfError::Stdout)?;
                stdout
                    .write_all(&pdf)
                    .and_then(|()| stdout.flush())
                    .map_err(PdfError::Stdout)?;
            }
        }
        Ok(bytes)
    }
}

impl Drop for PendingOutput {
    fn drop(&mut self) {
        // Gone already after a successful rename; otherwise a failed or
        // abandoned export leaves nothing behind.
        let _ = std::fs::remove_file(&self.temp);
    }
}

/// Creates `<dir>/.<stem>.mbr-export-<unique>.pdf` with `create_new`, which is
/// both the uniqueness guarantee and the writability check.
fn create_unique_temp(dir: &Path, stem: &str) -> Result<PathBuf, PdfError> {
    let stem: String = stem.chars().filter(|c| !c.is_control()).take(64).collect();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let mut last_error = None;
    for _ in 0..16 {
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!(
            ".{stem}.mbr-export-{}-{nanos:x}-{n}.pdf",
            std::process::id()
        );
        let path = dir.join(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last_error = Some(e),
            Err(source) => {
                return Err(PdfError::OutputDirNotWritable {
                    dir: dir.to_path_buf(),
                    source,
                });
            }
        }
    }
    Err(PdfError::OutputDirNotWritable {
        dir: dir.to_path_buf(),
        source: last_error.unwrap_or_else(|| std::io::Error::other("no free temporary name")),
    })
}

/// Checks `path` holds a PDF: it exists, is not empty and starts `%PDF-`.
/// Returns its size.
pub fn verify_pdf(path: &Path) -> Result<u64, PdfError> {
    use std::io::Read;
    let not_written = || PdfError::NotWritten {
        path: path.to_path_buf(),
    };
    let mut file = std::fs::File::open(path).map_err(|_| not_written())?;
    let size = file.metadata().map_err(|_| not_written())?.len();
    if size == 0 {
        return Err(PdfError::Empty {
            path: path.to_path_buf(),
        });
    }
    let mut magic = [0_u8; 5];
    match file.read_exact(&mut magic) {
        Ok(()) if &magic == b"%PDF-" => Ok(size),
        _ => Err(PdfError::NotAPdf {
            path: path.to_path_buf(),
        }),
    }
}

// ---------------------------------------------------------------------------
// The job: load → ready (or timeout) → print → verify → rename.
// ---------------------------------------------------------------------------

/// What happened to an export job, from the page, the timer or the print
/// system.
#[derive(Debug, Clone, PartialEq)]
pub enum JobEvent {
    Message(ExportMessage),
    /// The readiness timeout elapsed.
    TimedOut,
    /// The print system finished: `Err` carries its reason.
    Printed(Result<(), String>),
}

/// What the caller must do next.
#[derive(Debug, PartialEq, Eq)]
pub enum JobStep {
    /// Nothing yet.
    Wait,
    /// Start printing.
    Print,
    /// The print system is done: verify and move the file into place.
    Finish,
    /// Give up.
    Fail(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    WaitingForPage,
    Printing,
    Done,
}

/// The export state machine, free of webviews and timers so every transition
/// is testable. Each step is taken at most once: a second ready message, a
/// timeout after the print started or a print result after the end are
/// ignored.
#[derive(Debug)]
pub struct PdfJob {
    phase: Phase,
    warnings: Vec<String>,
    page: Option<PageSize>,
    timeout: Duration,
}

impl PdfJob {
    pub fn new(timeout: Duration) -> Self {
        PdfJob {
            phase: Phase::WaitingForPage,
            warnings: Vec::new(),
            page: None,
            timeout,
        }
    }

    /// Warnings collected so far (stuck images, the timeout, …).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The sheet the page's ready message declared, if any.
    pub fn page_size(&self) -> Option<PageSize> {
        self.page
    }

    pub fn on_event(&mut self, event: JobEvent) -> JobStep {
        match (self.phase, event) {
            (Phase::WaitingForPage, JobEvent::Message(ExportMessage::Ready { warnings, page })) => {
                self.warnings.extend(warnings);
                self.page = page;
                self.phase = Phase::Printing;
                JobStep::Print
            }
            (Phase::WaitingForPage, JobEvent::Message(ExportMessage::Failed { message })) => {
                self.phase = Phase::Done;
                JobStep::Fail(message)
            }
            (Phase::WaitingForPage, JobEvent::TimedOut) => {
                self.warnings.push(format!(
                    "the page did not finish rendering within {:.1} s; exported it as it was",
                    self.timeout.as_secs_f64()
                ));
                self.phase = Phase::Printing;
                JobStep::Print
            }
            (Phase::Printing, JobEvent::Printed(result)) => {
                self.phase = Phase::Done;
                match result {
                    Ok(()) => JobStep::Finish,
                    Err(message) => JobStep::Fail(message),
                }
            }
            _ => JobStep::Wait,
        }
    }
}

/// Identifies one export, so a timer or callback left over from an earlier
/// export in the same GUI process cannot steer a later one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobId(u64);

static NEXT_JOB_ID: AtomicU64 = AtomicU64::new(1);

impl JobId {
    fn next() -> Self {
        JobId(NEXT_JOB_ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// A [`JobEvent`] addressed to one export. The application wraps it in its
/// own event-loop event and hands it back to [`ExportSession::handle`].
#[derive(Debug, Clone, PartialEq)]
pub struct ExportEvent {
    pub job: JobId,
    pub event: JobEvent,
}

/// What a backend's `print_to_pdf` calls, once, when the print system is
/// done: `Err` carries its reason.
pub type PrintDone = Box<dyn FnOnce(Result<(), String>)>;

/// How an export delivers events to the application's event loop (usually
/// `EventLoopProxy::send_event`). Called from the IPC handler, the timer
/// thread and the print callback.
pub type ExportSink = Arc<dyn Fn(ExportEvent) + Send + Sync>;

/// A finished export.
#[derive(Debug)]
pub struct ExportedPdf {
    pub target: OutputTarget,
    pub bytes: u64,
    pub warnings: Vec<String>,
}

/// Checks the platform can export at all, before an event loop exists:
/// a backend, and (Linux) a display.
pub fn preflight() -> Result<(), PdfError> {
    if !BACKEND_AVAILABLE {
        return Err(PdfError::Unsupported {
            platform: std::env::consts::OS,
        });
    }
    platform::preflight()
}

/// One export in flight: the hidden window and webview, the job and the
/// output. Lives on the event-loop thread.
pub struct ExportSession {
    id: JobId,
    job: PdfJob,
    options: PdfOptions,
    output: Option<PendingOutput>,
    sink: ExportSink,
    // Field order is drop order: the print operation, then the webview, then
    // the window that hosts it.
    print: Option<platform::PrintInFlight>,
    webview: WebView,
    _window: Window,
}

impl ExportSession {
    /// Opens a hidden window on `url` and starts the readiness timer.
    pub fn start<T: 'static>(
        target: &EventLoopWindowTarget<T>,
        url: &str,
        options: PdfOptions,
        output: PendingOutput,
        sink: ExportSink,
    ) -> Result<Self, PdfError> {
        let id = JobId::next();
        let window = platform::export_window(
            WindowBuilder::new()
                .with_title("mbr PDF export")
                .with_visible(false)
                .with_focused(false)
                .with_inner_size(EXPORT_VIEWPORT),
        )
        .build(target)
        .map_err(PdfError::Window)?;

        let ipc_sink = Arc::clone(&sink);
        let builder = WebViewBuilder::new()
            .with_url(url)
            .with_initialization_script_for_main_only(options.init_script(), true)
            .with_ipc_handler(move |request| {
                if let Some(message) = parse_export_message(request.body()) {
                    ipc_sink(ExportEvent {
                        job: id,
                        event: JobEvent::Message(message),
                    });
                } else {
                    tracing::debug!("Ignoring an unrecognized IPC message from the export page");
                }
            })
            // Nothing leaves the export view: no system handler, no popups.
            // http(s) stays allowed because the handler also sees `<iframe>`
            // loads (a YouTube embed), which the page is printed with.
            .with_navigation_handler(|url| export_navigation_allowed(&url))
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny);
        let webview = platform::build_webview(builder, &window)?;
        platform::prepare_export_view(&webview);

        let timer_sink = Arc::clone(&sink);
        let timeout = options.timeout;
        std::thread::spawn(move || {
            std::thread::sleep(timeout);
            timer_sink(ExportEvent {
                job: id,
                event: JobEvent::TimedOut,
            });
        });

        Ok(ExportSession {
            id,
            job: PdfJob::new(options.timeout),
            options,
            output: Some(output),
            sink,
            print: None,
            webview,
            _window: window,
        })
    }

    pub fn id(&self) -> JobId {
        self.id
    }

    /// Advances the export. `Some` once it is over, either way; `stdout`
    /// receives the PDF for [`OutputTarget::Stdout`].
    pub fn handle(
        &mut self,
        event: ExportEvent,
        stdout: &mut dyn Write,
    ) -> Option<Result<ExportedPdf, PdfError>> {
        if event.job != self.id {
            return None;
        }
        match self.job.on_event(event.event) {
            JobStep::Wait => None,
            JobStep::Print => self.print().err().map(Err),
            JobStep::Fail(message) => Some(Err(if self.print.is_some() {
                PdfError::PrintFailed { message }
            } else {
                PdfError::PageFailed { message }
            })),
            JobStep::Finish => {
                self.print = None;
                let output = self.output.take()?;
                let target = output.target().clone();
                Some(output.finish(stdout).map(|bytes| ExportedPdf {
                    target,
                    bytes,
                    warnings: self.job.warnings().to_vec(),
                }))
            }
        }
    }

    fn print(&mut self) -> Result<(), PdfError> {
        let Some(output) = self.output.as_ref() else {
            return Err(PdfError::PrintFailed {
                message: "the export was already finished".to_owned(),
            });
        };
        let sink = Arc::clone(&self.sink);
        let id = self.id;
        let done: PrintDone = Box::new(move |result: Result<(), String>| {
            sink(ExportEvent {
                job: id,
                event: JobEvent::Printed(result),
            });
        });
        let page = self.job.page_size();
        if let Some(page) = page {
            tracing::info!(
                "The page sets its own @page size ({:.2} x {:.2} in); printing on it",
                page.width_in,
                page.height_in
            );
        }
        let options = self.options.with_page_size(page);
        let in_flight = platform::print_to_pdf(&self.webview, &options, output.temp_path(), done)?;
        self.print = Some(in_flight);
        Ok(())
    }
}

/// Which navigations the hidden export webview may make: web pages (its own
/// server, and embeds the page frames) and inert schemes. Never a hand-off to
/// another application.
fn export_navigation_allowed(url: &str) -> bool {
    let scheme = url.split_once(':').map(|(s, _)| s.to_ascii_lowercase());
    matches!(
        scheme.as_deref(),
        Some("http" | "https" | "about" | "data" | "blob")
    )
}

/// What a Linux preflight concludes from the environment and `gtk::init`'s
/// result. A pure function so it is tested on every platform.
///
/// `display`/`wayland_display` are `$DISPLAY`/`$WAYLAND_DISPLAY`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn linux_preflight_decision(
    display: Option<&str>,
    wayland_display: Option<&str>,
    gtk_init: Result<(), String>,
) -> Result<(), PdfError> {
    let set = |v: Option<&str>| v.is_some_and(|v| !v.trim().is_empty());
    match gtk_init {
        Ok(()) => Ok(()),
        Err(_) if !set(display) && !set(wayland_display) => Err(PdfError::NoDisplay(
            "PDF export needs a desktop session: neither DISPLAY nor WAYLAND_DISPLAY is set. \
             On a server, run it under a virtual display (xvfb-run mbr --pdf …)."
                .to_owned(),
        )),
        Err(reason) => Err(PdfError::NoDisplay(format!(
            "PDF export could not open the display ({}): {reason}",
            [display, wayland_display]
                .into_iter()
                .flatten()
                .filter(|v| !v.trim().is_empty())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// The paper GTK names (`gtk_paper_size_get_default()`, PWG names), when it is
/// one mbr knows. A pure function so it is tested on every platform.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn paper_for_gtk_name(name: &str) -> Option<Paper> {
    match name {
        "iso_a4" => Some(Paper::A4),
        "na_letter" => Some(Paper::Letter),
        "na_legal" => Some(Paper::Legal),
        _ => None,
    }
}

/// The GTK paper name for a standard paper, so the print system gets the
/// named size (and the PDF its name) rather than a custom one.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn gtk_name_for_paper(paper: Paper) -> Option<&'static str> {
    match paper {
        Paper::A4 => Some("iso_a4"),
        Paper::Letter => Some("na_letter"),
        Paper::Legal => Some("na_legal"),
        Paper::Custom { .. } => None,
    }
}

/// A locale name written by Windows' `GetUserDefaultLocaleName` into `buffer`,
/// given the function's return value: the length **including** the
/// terminating NUL, or 0 on failure. A pure function so it is tested on every
/// platform.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn locale_from_utf16(buffer: &[u16], returned: i32) -> Option<String> {
    let len = usize::try_from(returned).ok()?.checked_sub(1)?;
    // Up to the first NUL, should the count and the string ever disagree.
    let units = buffer.get(..len)?.split(|&unit| unit == 0).next()?;
    let name = String::from_utf16(units).ok()?;
    (!name.trim().is_empty()).then_some(name)
}

#[cfg(test)]
mod tests;

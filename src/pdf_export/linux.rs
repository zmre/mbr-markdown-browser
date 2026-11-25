//! Linux backend: WebKitGTK's `PrintOperation`, printing to GTK's "Print to
//! File" printer with no dialog.
//!
//! The recipe, from webkit2gtk 2.0's and gtk 0.18's source (no spike on a
//! Linux desktop yet — CI's `xvfb-run` smoke test is the first real run):
//!
//! - [`preflight`] runs before tao's `EventLoop::new`, which panics without a
//!   display: `gtk::init()`, and on failure a clear message (exit 2).
//! - [`build_webview`] builds the webview into a `gtk::OffscreenWindow`
//!   (`WebViewBuilderExtUnix::build_gtk` accepts any container) and
//!   `show_all()`s it, so WebKit realizes and maps the view — an unmapped one
//!   never loads lazy content — without anything appearing on screen. The tao
//!   `window` stays hidden and only bounds the export's lifetime; the offscreen
//!   window, a GTK toplevel that GTK keeps alive until it is destroyed, is
//!   destroyed when the webview is.
//! - [`print_to_pdf`]: `webkit2gtk::PrintOperation` with a `gtk::PrintSettings`
//!   naming the printer `dgettext("gtk30", "Print to File")` (the name is
//!   translated; WebKit looks the printer up by it), `output-uri` = the
//!   temporary file's `file://` URI and `output-file-format` = `pdf`; a
//!   `gtk::PageSetup` with the paper ([`PdfOptions::sheet`]: portrait sides
//!   plus an orientation, as GTK wants) and **zero margins** — the page's
//!   `@page` rule sets them. `failed` fires before `finished`, so the error is
//!   kept and reported by `finished`, which calls `done` once. `print()` shows
//!   no dialog.
//! - No header/footer support: `page_numbers` is ignored, with one INFO line.
//! - [`prepare_export_view`] forces nothing: GTK's dark-theme preference is
//!   per screen, shared with the GUI's own window, and paper does not need it
//!   (theme.css's dark rules are `only screen`; mermaid renders light in export
//!   mode by itself).
//!
//! The EGL fallback in flake.nix ("EGL on non-NixOS hosts", CLAUDE.md) applies
//! to this web process too.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Once;

use gtk::glib;
use gtk::prelude::*;
use tao::event_loop::EventLoop;
use tao::window::{Window, WindowBuilder};
use webkit2gtk::{PrintOperation, PrintOperationExt};
use wry::{WebView, WebViewBuilder, WebViewBuilderExtUnix, WebViewExtUnix};

use super::{
    EXPORT_VIEWPORT, ExportOrigins, PdfOptions, PrintDone, gtk_name_for_paper,
    linux_preflight_decision, paper_for_gtk_name,
};
use crate::errors::PdfError;
use crate::pdf_config::{Orientation, Paper, paper_for_locale, paper_locale_from_env};

pub const BACKEND_AVAILABLE: bool = true;

/// GTK's translation domain, where the "Print to File" printer's name lives.
const GTK_TEXT_DOMAIN: &str = "gtk30";
/// The untranslated name of GTK's file printer (its file print backend).
const PRINT_TO_FILE: &str = "Print to File";

/// `gtk::init()` before tao touches GTK, so a missing display is a message
/// and exit code 2 instead of a panic.
pub fn preflight() -> Result<(), PdfError> {
    let display = std::env::var("DISPLAY").ok();
    let wayland = std::env::var("WAYLAND_DISPLAY").ok();
    linux_preflight_decision(
        display.as_deref(),
        wayland.as_deref(),
        gtk::init().map_err(|e| e.to_string()),
    )
}

pub fn configure_cli_event_loop<T: 'static>(_event_loop: &mut EventLoop<T>) {}

/// Unchanged: the tao window is never shown; the view lives in an offscreen
/// window ([`build_webview`]).
pub fn export_window(builder: WindowBuilder) -> WindowBuilder {
    builder
}

/// The webview, in a `gtk::OffscreenWindow` that is mapped (so WebKit lays
/// out, loads and runs the page as for a visible view) but never on screen.
///
/// Nothing beyond wry's navigation handler is needed: WebKit asks it before
/// any request is made, for frames too.
pub fn build_webview(
    builder: WebViewBuilder<'_>,
    _window: &Window,
    _origins: &ExportOrigins,
) -> Result<WebView, PdfError> {
    let host = gtk::OffscreenWindow::new();
    // Whole pixels; the viewport is a plausible desktop size, not exact.
    host.set_default_size(EXPORT_VIEWPORT.width as i32, EXPORT_VIEWPORT.height as i32);
    let webview = builder.build_gtk(&host).map_err(PdfError::WebView)?;
    host.show_all();

    // GTK owns a toplevel until it is destroyed, so the offscreen window
    // would outlive the export. wry destroys the view when the `WebView` is
    // dropped; take the host with it, from the main loop rather than inside
    // the view's own destruction.
    webview.webview().connect_destroy(move |_| {
        let host = host.clone();
        glib::idle_add_local_once(move || {
            // SAFETY: `host` is a toplevel this module created and nothing
            // else references; destroying it releases GTK's own reference.
            unsafe { host.destroy() };
        });
    });
    Ok(webview)
}

/// Nothing: the light scheme is not forced (see the module comment).
pub fn prepare_export_view(_webview: &WebView) {}

/// GTK's default paper (from the locale's `LC_PAPER`) when it is a size mbr
/// names; else Letter for US/Canadian locales, A4 elsewhere.
pub fn default_paper() -> Paper {
    // `gtk_paper_size_get_default` needs GTK initialized on this thread: the
    // CLI's preflight and the GUI's event loop both have.
    let from_gtk = gtk::is_initialized_main_thread()
        .then(gtk::PaperSize::default)
        .flatten()
        .and_then(|name| paper_for_gtk_name(name.as_str()));
    from_gtk.unwrap_or_else(|| {
        paper_locale_from_env(|name| std::env::var(name).ok())
            .map_or(Paper::A4, |locale| paper_for_locale(&locale))
    })
}

/// Keeps the print operation (and so its signal handlers) alive until
/// `finished` has fired.
pub struct PrintInFlight {
    _operation: PrintOperation,
}

/// Starts the silent print of `webview` into `path`. `done` runs on the GTK
/// main thread when WebKit is finished.
pub fn print_to_pdf(
    webview: &WebView,
    options: &PdfOptions,
    path: &Path,
    done: PrintDone,
) -> Result<PrintInFlight, PdfError> {
    let failed = |message: String| PdfError::PrintFailed { message };
    let absolute = std::path::absolute(path).map_err(|e| failed(e.to_string()))?;
    let uri = glib::filename_to_uri(&absolute, None)
        .map_err(|e| failed(format!("{}: {e}", absolute.display())))?;

    if options.page_numbers {
        static IGNORED: Once = Once::new();
        IGNORED.call_once(|| {
            tracing::info!("Page numbers are not available in PDF export on Linux; ignored");
        });
    }

    let sheet = options.sheet();
    let paper = paper_size(options.paper, sheet.width_in, sheet.height_in);
    let orientation = match sheet.orientation {
        Orientation::Portrait => gtk::PageOrientation::Portrait,
        Orientation::Landscape => gtk::PageOrientation::Landscape,
    };

    let settings = gtk::PrintSettings::new();
    let printer = glib::dgettext(Some(GTK_TEXT_DOMAIN), PRINT_TO_FILE);
    settings.set_printer(printer.as_str());
    settings.set(gtk::PRINT_SETTINGS_OUTPUT_URI.as_str(), Some(uri.as_str()));
    settings.set(gtk::PRINT_SETTINGS_OUTPUT_FILE_FORMAT.as_str(), Some("pdf"));
    settings.set_paper_size(&paper);
    settings.set_orientation(orientation);

    let setup = gtk::PageSetup::new();
    setup.set_paper_size(&paper);
    setup.set_orientation(orientation);
    // Zero: the page's `@page` rule has the margins; GTK's would add to them.
    setup.set_top_margin(0.0, gtk::Unit::Inch);
    setup.set_bottom_margin(0.0, gtk::Unit::Inch);
    setup.set_left_margin(0.0, gtk::Unit::Inch);
    setup.set_right_margin(0.0, gtk::Unit::Inch);

    let operation = PrintOperation::new(&webview.webview());
    operation.set_print_settings(&settings);
    operation.set_page_setup(&setup);

    // `failed` comes first and `finished` always follows it.
    let failure: Rc<RefCell<Option<String>>> = Rc::default();
    let recorded = Rc::clone(&failure);
    operation.connect_failed(move |_, error| {
        recorded.borrow_mut().get_or_insert_with(|| {
            format!("WebKitGTK could not print to PDF: {}", error.message())
        });
    });
    let done = RefCell::new(Some(done));
    operation.connect_finished(move |_| {
        if let Some(done) = done.borrow_mut().take() {
            done(failure.borrow_mut().take().map_or(Ok(()), Err));
        }
    });

    operation.print();
    Ok(PrintInFlight {
        _operation: operation,
    })
}

/// The GTK paper: the standard size by name, else a custom one of the sheet's
/// portrait sides.
fn paper_size(paper: Paper, width_in: f64, height_in: f64) -> gtk::PaperSize {
    match gtk_name_for_paper(paper) {
        Some(name) => gtk::PaperSize::new(Some(name)),
        None => {
            gtk::PaperSize::new_custom("mbr-custom", "Custom", width_in, height_in, gtk::Unit::Inch)
        }
    }
}

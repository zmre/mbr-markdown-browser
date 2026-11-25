//! Windows backend: WebView2's `PrintToPdf`, on the UI thread.
//!
//! The recipe, from wry 0.57's and webview2-com 0.39's source (no spike on a
//! Windows machine yet — CI's `MBR_PDF_TESTS=1` run is the first real one):
//!
//! - `ICoreWebView2Environment6::CreatePrintSettings` with the page's width
//!   and height in inches **already turned** ([`PdfOptions::page_size_in`])
//!   and the orientation left portrait, so the sides are swapped exactly once;
//!   zero margins (the page's `@page` rule sets them), scale 1, backgrounds,
//!   and Chromium's header and footer only for `page_numbers` — with the
//!   header's title set to the document's title explicitly and the footer's
//!   URL blanked: both default to things naming the export's throwaway
//!   loopback server (`http://127.0.0.1:<port>/…`) — the URL always, the title
//!   when the page has none;
//! - `ICoreWebView2_7::PrintToPdf(absolute path, settings, handler)`: the
//!   handler runs on this (the UI) thread when Chromium is done and calls
//!   `done`. Never `wait_for_async_operation`, which pumps the message loop
//!   underneath tao;
//! - `E_NOINTERFACE` on either cast means a runtime older than
//!   `PrintToPdf` (WebView2 1.0.1020): said so in the error.
//!
//! Requests leaving the exported page's origin are stopped twice. wry's
//! navigation handler is WebView2's `NavigationStarting`, whose cancel can
//! come after Chromium has already sent the request (measured in CI: a
//! script's `location.href = 'http://other/…?d=…'` reached the other server
//! although the navigation was cancelled), and which is never asked about a
//! frame's navigation. So [`build_webview`] also registers a
//! `WebResourceRequested` handler for every **document** request — top-level
//! and frame, from any source kind where the runtime supports
//! `ICoreWebView2_22` — and answers each one [`ExportOrigins`] refuses with a
//! local 403 before it is sent ([`refuse_foreign_documents`]). Subresources
//! (images, `fetch`) are deliberately not filtered, as on macOS and Linux.
//! wry registers no `WebResourceRequested` handler of its own here: it does
//! so only for custom protocols, and the export uses none.
//!
//! The export window is **shown, off-screen and never activated**
//! ([`export_window`]) rather than hidden: Chromium may throttle a view whose
//! window is hidden (timers, image decoding), and the page's readiness check
//! runs on timers. The window has no taskbar button and takes no focus.
//!
//! The light colour scheme is not forced: WebView2's preferred colour scheme
//! is a property of the *profile*, which the export view shares with the GUI's
//! own window (same user data folder), so setting it would turn the visible
//! window light too. Paper does not need it: theme.css's dark rules are all
//! `only screen`, and mermaid renders light in export mode by itself.

use std::path::Path;

use tao::dpi::PhysicalPosition;
use tao::event_loop::EventLoop;
use tao::platform::windows::WindowBuilderExtWindows;
use tao::window::{Window, WindowBuilder};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_PRINT_ORIENTATION_PORTRAIT, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT,
    COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL, ICoreWebView2_7, ICoreWebView2_22,
    ICoreWebView2Environment, ICoreWebView2Environment6, ICoreWebView2PrintSettings,
    ICoreWebView2PrintToPdfCompletedHandler, ICoreWebView2WebResourceRequestedEventArgs,
};
use webview2_com::{PrintToPdfCompletedHandler, WebResourceRequestedEventHandler, take_pwstr};
use windows::Win32::Foundation::E_NOINTERFACE;
use windows::Win32::Globalization::GetUserDefaultLocaleName;
use windows::core::{HSTRING, Interface, PWSTR};
use wry::{WebView, WebViewBuilder, WebViewExtWindows};

use super::{ExportOrigins, PdfOptions, PrintDone, locale_from_utf16};
use crate::errors::PdfError;
use crate::pdf_config::{Paper, paper_for_locale, paper_locale_from_env};

pub const BACKEND_AVAILABLE: bool = true;

/// `LOCALE_NAME_MAX_LENGTH` (winnls.h), NUL included.
const LOCALE_NAME_MAX_LENGTH: usize = 85;

/// Far outside every monitor (the coordinate Windows itself parks minimized
/// windows at), so the shown export window is never on screen.
const OFF_SCREEN: PhysicalPosition<i32> = PhysicalPosition::new(-32000, -32000);

/// Nothing to check: WebView2 needs no display server, and a missing runtime
/// is reported by the webview's own creation error.
pub fn preflight() -> Result<(), PdfError> {
    Ok(())
}

pub fn configure_cli_event_loop<T: 'static>(_event_loop: &mut EventLoop<T>) {}

/// Shown but off-screen, never activated (`with_focused(false)` makes tao show
/// it `SW_SHOWNOACTIVATE`; `with_focusable(false)` adds `WS_EX_NOACTIVATE`),
/// and without a taskbar button. See the module comment for why not hidden.
pub fn export_window(builder: WindowBuilder) -> WindowBuilder {
    builder
        .with_position(OFF_SCREEN)
        .with_focused(false)
        .with_focusable(false)
        .with_skip_taskbar(true)
        .with_visible(true)
}

/// The webview, with [`refuse_foreign_documents`] installed. Fails closed: a
/// view whose guard could not be registered is dropped, not exported from.
pub fn build_webview(
    builder: WebViewBuilder<'_>,
    window: &Window,
    origins: &ExportOrigins,
) -> Result<WebView, PdfError> {
    let webview = builder.build(window).map_err(PdfError::WebView)?;
    refuse_foreign_documents(&webview, origins.clone())
        .map_err(|e| PdfError::WebView(wry::Error::WebView2Error(e.into())))?;
    Ok(webview)
}

/// Answers every document request (top-level or frame) that `origins` does
/// not allow with a local `403 Forbidden`, so it is never sent. Registered
/// right after `build`, while the first navigation (the exported page, which
/// is allowed) is still pending: WebView2 raises its events from the message
/// loop, which has not run yet.
fn refuse_foreign_documents(
    webview: &WebView,
    origins: ExportOrigins,
) -> windows::core::Result<()> {
    let view = webview.webview();
    let environment = webview.environment();
    let every_uri = HSTRING::from("*");
    // SAFETY: `view` is a live WebView2 used on the UI thread it was created
    // on; the filter string outlives the call.
    unsafe {
        // Runtimes with `ICoreWebView2_22` raise `WebResourceRequested` only
        // for the source kinds named (wry does the same for its custom
        // protocols, so iframes and workers are covered); the older call is
        // the documents-only equivalent.
        match view.cast::<ICoreWebView2_22>() {
            Ok(view_22) => view_22.AddWebResourceRequestedFilterWithRequestSourceKinds(
                &every_uri,
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT,
                COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
            )?,
            Err(_) => view.AddWebResourceRequestedFilter(
                &every_uri,
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT,
            )?,
        }
    }

    let handler = WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else {
            return Ok(());
        };
        // An unreadable URI is refused too.
        let uri = request_uri(&args);
        if uri.as_deref().is_ok_and(|uri| origins.allows(uri)) {
            return Ok(());
        }
        let shown = uri.as_deref().unwrap_or("an unreadable URL");
        tracing::warn!("The export page tried to load {shown}; refused before it was sent");
        forbid(&environment, &args).inspect_err(|e| {
            tracing::error!("Could not refuse the export page's request for {shown}: {e}");
        })
    }));
    let mut token = 0_i64;
    // SAFETY: as above; WebView2 keeps its own reference to the handler for
    // the life of the view, and `token` is a valid out-parameter.
    unsafe { view.add_WebResourceRequested(&handler, &mut token) }
}

/// The request's URI, as WebView2 reports it.
fn request_uri(args: &ICoreWebView2WebResourceRequestedEventArgs) -> windows::core::Result<String> {
    let mut uri = PWSTR::null();
    // SAFETY: the event's arguments, on the UI thread during the event; `Uri`
    // fills a `CoTaskMemAlloc`ed string that `take_pwstr` copies and frees.
    unsafe { args.Request()?.Uri(&mut uri)? };
    Ok(take_pwstr(uri))
}

/// Sets an empty `403 Forbidden` as the request's response, which WebView2
/// then serves instead of sending the request.
fn forbid(
    environment: &ICoreWebView2Environment,
    args: &ICoreWebView2WebResourceRequestedEventArgs,
) -> windows::core::Result<()> {
    // SAFETY: the environment that created the view and the event's
    // arguments, on the UI thread during the event; the strings outlive the
    // calls, and no body stream is passed.
    unsafe {
        let response = environment.CreateWebResourceResponse(
            None,
            403,
            &HSTRING::from("Forbidden"),
            &HSTRING::new(),
        )?;
        args.SetResponse(&response)
    }
}

/// Nothing: the light scheme is not forced (see the module comment).
pub fn prepare_export_view(_webview: &WebView) {}

/// Letter for US/Canadian user locales (`GetUserDefaultLocaleName`, e.g.
/// `en-US`, `fr-CA`), A4 elsewhere; the POSIX variables only if Windows
/// cannot say.
pub fn default_paper() -> Paper {
    user_locale_name()
        .or_else(|| paper_locale_from_env(|name| std::env::var(name).ok()))
        .map_or(Paper::A4, |locale| paper_for_locale(&locale))
}

/// The user's default locale name (`en-US`), or `None` if Windows fails.
fn user_locale_name() -> Option<String> {
    let mut buffer = [0_u16; LOCALE_NAME_MAX_LENGTH];
    // SAFETY: the buffer is valid and writable for its whole length, which the
    // binding passes as the count; the function writes at most that many
    // UTF-16 units and touches nothing else.
    let returned = unsafe { GetUserDefaultLocaleName(&mut buffer) };
    locale_from_utf16(&buffer, returned)
}

/// Keeps the print settings and the completion handler alive until Chromium
/// has called the handler. (WebView2 holds its own references while the
/// operation runs; these make that independent of its implementation.)
pub struct PrintInFlight {
    _settings: ICoreWebView2PrintSettings,
    _handler: ICoreWebView2PrintToPdfCompletedHandler,
}

/// Starts the silent print of `webview` into `path`. `done` runs on this (the
/// UI) thread when WebView2 is finished.
pub fn print_to_pdf(
    webview: &WebView,
    options: &PdfOptions,
    path: &Path,
    done: PrintDone,
) -> Result<PrintInFlight, PdfError> {
    let failed = |message: String| PdfError::PrintFailed { message };
    let absolute = std::path::absolute(path).map_err(|e| failed(e.to_string()))?;

    let view = webview
        .webview()
        .cast::<ICoreWebView2_7>()
        .map_err(|e| failed(com_message("ICoreWebView2_7", &e)))?;
    let environment = webview
        .environment()
        .cast::<ICoreWebView2Environment6>()
        .map_err(|e| failed(com_message("ICoreWebView2Environment6", &e)))?;

    let title = if options.page_numbers {
        document_title(&view)
    } else {
        String::new()
    };
    let settings = print_settings(&environment, options, &title)
        .map_err(|e| failed(format!("WebView2 refused the print settings: {e}")))?;

    let handler = PrintToPdfCompletedHandler::create(Box::new(
        move |result: windows::core::Result<()>, written: bool| {
            done(match result {
                Ok(()) if written => Ok(()),
                Ok(()) => Err("WebView2 reported that printing to PDF failed".to_owned()),
                Err(e) => Err(format!("WebView2 could not print to PDF: {e}")),
            });
            Ok(())
        },
    ));

    let target = HSTRING::from(absolute.as_path());
    // SAFETY: `view` is a live WebView2 used on the UI thread it was created
    // on; the path, settings and handler are valid COM arguments, and the
    // settings and handler are kept alive by the returned `PrintInFlight`.
    unsafe { view.PrintToPdf(&target, &settings, &handler) }
        .map_err(|e| failed(format!("WebView2 could not start printing to PDF: {e}")))?;

    Ok(PrintInFlight {
        _settings: settings,
        _handler: handler,
    })
}

/// The page's `<title>`, or empty when it has none (or WebView2 cannot say).
fn document_title(view: &ICoreWebView2_7) -> String {
    let mut title = PWSTR::null();
    // SAFETY: an out-parameter WebView2 fills with a `CoTaskMemAlloc`ed
    // string, which `take_pwstr` copies and frees; on failure it stays null
    // and is not read.
    match unsafe { view.DocumentTitle(&mut title) } {
        Ok(()) if !title.is_null() => take_pwstr(title).trim().to_owned(),
        _ => String::new(),
    }
}

/// The print settings: the page's turned size in inches, portrait (so nothing
/// is turned twice), no margins, scale 1, backgrounds, and the header and
/// footer only when asked for — titled `title` (empty: no title), with no URL.
fn print_settings(
    environment: &ICoreWebView2Environment6,
    options: &PdfOptions,
    title: &str,
) -> windows::core::Result<ICoreWebView2PrintSettings> {
    let (width, height) = options.page_size_in();
    // SAFETY: plain property setters on a print-settings object the
    // environment just created, called on the UI thread; the string argument
    // outlives the call.
    unsafe {
        let settings = environment.CreatePrintSettings()?;
        settings.SetOrientation(COREWEBVIEW2_PRINT_ORIENTATION_PORTRAIT)?;
        settings.SetPageWidth(width)?;
        settings.SetPageHeight(height)?;
        settings.SetMarginTop(0.0)?;
        settings.SetMarginBottom(0.0)?;
        settings.SetMarginLeft(0.0)?;
        settings.SetMarginRight(0.0)?;
        settings.SetScaleFactor(1.0)?;
        settings.SetShouldPrintBackgrounds(true)?;
        settings.SetShouldPrintHeaderAndFooter(options.page_numbers)?;
        if options.page_numbers {
            // Both defaults can name the export's loopback server, dead once
            // the export ends: the footer is the page's URL, and the header a
            // title-less page's URL. Empty = not shown.
            settings.SetHeaderTitle(&HSTRING::from(title))?;
            settings.SetFooterUri(&HSTRING::new())?;
        }
        Ok(settings)
    }
}

/// A failed `QueryInterface`, with `E_NOINTERFACE` explained.
fn com_message(interface: &str, error: &windows::core::Error) -> String {
    if error.code() == E_NOINTERFACE {
        format!(
            "WebView2 runtime too old for PDF export (it lacks {interface}); \
             update the Microsoft Edge WebView2 Runtime"
        )
    } else {
        format!("WebView2 has no {interface}: {error}")
    }
}

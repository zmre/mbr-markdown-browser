//! macOS backend: `NSPrintOperation` from the `WKWebView`, saved to a file.
//!
//! The recipe the spike (design/pdf-export.md, E1–E9) proved:
//!
//! - `printOperationWithPrintInfo:` with an `NSPrintInfo` whose job
//!   disposition is `NSPrintSaveJob` and whose `NSPrintJobSavingURL` is an
//!   absolute file URL; no print or progress panel;
//! - run **modal for the (hidden) window** with a delegate — the synchronous
//!   `runOperation` never returns from a hidden view and writes blank pages
//!   without end;
//! - `NSPrintInfo` margins are ignored by WebKit (the page's `@page` rule
//!   sets them), so they are zeroed; paper size and orientation are honoured;
//! - `NSPrintHeaderAndFooter` draws title, date and "Page N of M".

use std::cell::RefCell;
use std::ffi::c_void;
use std::path::Path;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSPaperOrientation,
    NSPrintHeaderAndFooter, NSPrintInfo, NSPrintJobSavingURL, NSPrintOperation, NSPrintSaveJob,
};
use objc2_foundation::{NSNumber, NSSize, NSString, NSURL};
use tao::event_loop::EventLoop;
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tao::window::{Window, WindowBuilder};
use wry::{WebView, WebViewBuilder, WebViewExtMacOS};

use super::{PdfOptions, PrintDone};
use crate::errors::PdfError;
use crate::pdf_config::{Orientation, Paper};

/// Points per inch, AppKit's unit.
const POINTS_PER_INCH: f64 = 72.0;

pub const BACKEND_AVAILABLE: bool = true;

/// Nothing to check: there is always a window server for an app.
pub fn preflight() -> Result<(), PdfError> {
    Ok(())
}

/// Accessory: no Dock icon, no menu bar, focus left where it was.
pub fn configure_cli_event_loop<T: 'static>(event_loop: &mut EventLoop<T>) {
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
}

/// The export window as `mod.rs` describes it: never shown (E1: a hidden
/// window prints).
pub fn export_window(builder: WindowBuilder) -> WindowBuilder {
    builder
}

pub fn build_webview(builder: WebViewBuilder<'_>, window: &Window) -> Result<WebView, PdfError> {
    builder.build(window).map_err(PdfError::WebView)
}

/// Light appearance on the export view only (never the app: the GUI's own
/// window keeps the user's), and keep the hidden window out of the Window menu.
pub fn prepare_export_view(webview: &WebView) {
    // SAFETY: `NSAppearanceNameAqua` is an immutable `NSString` constant that
    // AppKit exports for the life of the process.
    let aqua = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameAqua });
    let window = webview.ns_window();
    window.setAppearance(aqua.as_deref());
    window.setExcludedFromWindowsMenu(true);
    webview.webview().setAppearance(aqua.as_deref());
}

/// The system's default paper (the default printer's), named when it is a
/// standard size.
pub fn default_paper() -> Paper {
    let size = NSPrintInfo::sharedPrintInfo().paperSize();
    if size.width > 0.0 && size.height > 0.0 {
        Paper::from_dimensions_in(size.width / POINTS_PER_INCH, size.height / POINTS_PER_INCH)
    } else {
        Paper::Letter
    }
}

/// The ivars of [`PrintDelegate`]: the completion, called once.
struct PrintDelegateIvars {
    done: RefCell<Option<PrintDone>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and this class
    // overrides nothing of it.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MbrPdfExportPrintDelegate"]
    #[ivars = PrintDelegateIvars]
    struct PrintDelegate;

    impl PrintDelegate {
        // SAFETY: the signature is `didRunSelector:`'s documented one,
        // `- (void)printOperationDidRun:(NSPrintOperation *)op
        //   success:(BOOL)success contextInfo:(void *)info`.
        #[unsafe(method(printOperationDidRun:success:contextInfo:))]
        fn did_run(&self, _operation: &NSPrintOperation, success: Bool, _context: *mut c_void) {
            let done = self.ivars().done.borrow_mut().take();
            if let Some(done) = done {
                done(if success.as_bool() {
                    Ok(())
                } else {
                    Err("the print system reported a failure".to_owned())
                });
            }
        }
    }
);

impl PrintDelegate {
    fn new(mtm: MainThreadMarker, done: PrintDone) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PrintDelegateIvars {
            done: RefCell::new(Some(done)),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer, called once
        // on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }
}

/// Keeps the operation and its delegate alive until the delegate has run:
/// AppKit holds neither strongly.
pub struct PrintInFlight {
    _operation: Retained<NSPrintOperation>,
    _delegate: Retained<PrintDelegate>,
}

/// Starts the silent print of `webview` into `path` (absolute). `done` runs
/// on the main thread when AppKit is finished.
pub fn print_to_pdf(
    webview: &WebView,
    options: &PdfOptions,
    path: &Path,
    done: PrintDone,
) -> Result<PrintInFlight, PdfError> {
    let failed = |message: &str| PdfError::PrintFailed {
        message: message.to_owned(),
    };
    let mtm = MainThreadMarker::new().ok_or_else(|| failed("not on the main thread"))?;
    // A relative path makes CFURL resolve against an unspecified base.
    let absolute = std::path::absolute(path).map_err(|e| failed(&e.to_string()))?;

    let info = print_info(options, &absolute);
    let wk = webview.webview();
    // SAFETY: `info` is a fully configured, owned `NSPrintInfo`; WebKit copies
    // what it needs from it.
    let operation = unsafe { wk.printOperationWithPrintInfo(&info) };
    operation.setShowsPrintPanel(false);
    operation.setShowsProgressPanel(false);
    // Without a frame the print view has no width to paginate against.
    if let Some(view) = operation.view() {
        view.setFrame(wk.bounds());
    }
    let window = wk
        .window()
        .ok_or_else(|| failed("the export webview has no window"))?;

    let delegate = PrintDelegate::new(mtm, done);
    // SAFETY: the delegate implements the selector with the signature AppKit
    // calls it with (see `did_run`), and both it and the operation are kept
    // alive by the returned `PrintInFlight` until that call. The context
    // pointer is unused.
    unsafe {
        operation.runOperationModalForWindow_delegate_didRunSelector_contextInfo(
            &window,
            Some(&*(Retained::as_ptr(&delegate) as *const AnyObject)),
            Some(sel!(printOperationDidRun:success:contextInfo:)),
            std::ptr::null_mut(),
        );
    }
    Ok(PrintInFlight {
        _operation: operation,
        _delegate: delegate,
    })
}

/// The print settings: paper, orientation, no margins (the page's `@page`
/// rule has them), save to `path`, optional header and footer.
fn print_info(options: &PdfOptions, path: &Path) -> Retained<NSPrintInfo> {
    let info = NSPrintInfo::new();
    // AppKit wants the sheet's portrait size plus an orientation; it swaps
    // the sides itself for landscape.
    let sheet = options.sheet();
    info.setPaperSize(NSSize::new(
        sheet.width_in * POINTS_PER_INCH,
        sheet.height_in * POINTS_PER_INCH,
    ));
    info.setOrientation(match sheet.orientation {
        Orientation::Portrait => NSPaperOrientation::Portrait,
        Orientation::Landscape => NSPaperOrientation::Landscape,
    });
    info.setTopMargin(0.0);
    info.setBottomMargin(0.0);
    info.setLeftMargin(0.0);
    info.setRightMargin(0.0);
    // SAFETY: `NSPrintSaveJob` is an immutable string constant exported by
    // AppKit.
    info.setJobDisposition(unsafe { NSPrintSaveJob });

    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    // SAFETY: the print info's dictionary is mutable and documented to take an
    // `NSURL` under `NSPrintJobSavingURL` and an `NSNumber` boolean under
    // `NSPrintHeaderAndFooter`; both keys are AppKit's immutable constants.
    unsafe {
        let dictionary = info.dictionary();
        let _: () = msg_send![&*dictionary, setObject: &*url, forKey: NSPrintJobSavingURL];
        if options.page_numbers {
            let yes = NSNumber::new_bool(true);
            let _: () = msg_send![&*dictionary, setObject: &*yes, forKey: NSPrintHeaderAndFooter];
        }
    }
    info
}

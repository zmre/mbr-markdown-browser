//! Which GUI navigations leave the mbr window, and how they reach the OS.
//!
//! wry installs no navigation policy of its own — its macOS delegate answers
//! `map_or(true, ...)`, so every navigation is permitted — and WKWebView,
//! unlike UIKit, never falls back to `NSWorkspace` for a scheme it cannot
//! render. Those are two faces of one bug: `https://example.com` replaced the
//! document *inside* the mbr window, while `message:`, `mailto:` and
//! `zoommtg:` links did nothing at all.
//!
//! ## Two entry points, because one of them is frame-blind
//!
//! There are two navigation policies here, and the difference between them is
//! the whole design:
//!
//! - [`decide_without_frame_info`] answers wry's navigation handler, which
//!   **cannot tell a document navigation from an `<iframe>` load**. It is
//!   therefore only allowed to act on schemes a frame can never load.
//! - [`SiteOrigin::decide`] is the full policy, including cross-origin
//!   `http(s)`. It answers wry's *new window* handler, which is only ever
//!   consulted for `window.open()`/`target="_blank"` — never for a frame — and
//!   it vets URLs the page posts over IPC ([`parse_ipc_open_request`]).
//!
//! Ordinary cross-origin link clicks are caught in the page instead, by
//! `components/src/mbr-link-enhancement.ts`, which knows exactly which frame it
//! is in. That is the piece [`decide_without_frame_info`] deliberately gives up.
//!
//! ## The URL is never rewritten
//!
//! Whatever the webview hands us is what the operating system gets, byte for
//! byte. `message://%3CCAEn…%40mail.gmail.com%3E` addresses a message by its
//! `Message-ID`, and the angle brackets are part of that identifier; decoding
//! or re-encoding the `%3C`/`%3E` produces an ID that Mail cannot find. Nothing
//! here parses and reserializes a URL — the policy only *reads* prefixes — and
//! [`apply_decision`] passes its `&str` straight through.
//!
//! ## Launching is GUI-only, and fails closed
//!
//! Deciding a URL is external is one thing; *starting an application* is
//! another, and only a GUI window may do the second. A person is sitting in
//! front of that window and clicked something. A process answering HTTP has no
//! such person, and "make the server host launch an application" is not a
//! feature — so in server and static modes links are the visiting browser's
//! business and mbr does nothing at all.
//!
//! Today no HTTP handler can reach [`open_external`]: this module and
//! `browser.rs` are both `#[cfg(feature = "gui")]`, and `launch_browser` is
//! called only from the GUI arm of `main.rs`. That is convention, though, and
//! the `gui` feature is **on by default**, so a server-mode binary still
//! contains the launcher. [`GUI_ACTIVE`] turns the convention into a runtime
//! fact: unless [`mark_gui_active`] has run, [`open_external`] refuses before it
//! touches the OS, whoever the caller is — a future route handler, a test, a
//! library consumer of this crate.

use crate::errors::ExternalOpenError;
use crate::url_path::url_scheme;
use std::sync::atomic::{AtomicBool, Ordering};

/// What the GUI webview should do with a navigation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationDecision {
    /// Navigate in the mbr window, as mbr has always done.
    Proceed,
    /// Hand the URL to the OS default handler and refuse the in-window
    /// navigation.
    OpenExternally,
    /// Refuse the navigation and do *not* hand the URL to the OS.
    Block,
}

/// The message `components/src/mbr-link-enhancement.ts` posts over wry's IPC
/// channel when it intercepts a cross-origin link click, followed by the
/// absolute URL.
///
/// Both ends of this string are load-bearing and live in different languages,
/// so neither can be changed alone. [`parse_ipc_open_request`] re-runs the full
/// policy on the payload, because anything the page can execute can post here.
pub const IPC_OPEN_EXTERNAL_PREFIX: &str = "mbr:open-external:";

/// Schemes that must never be handed to the operating system.
///
/// `html.rs` already neutralizes these in rendered markdown, but the navigation
/// handler is a second trust boundary: every navigation the webview attempts
/// arrives here, including ones that never passed through the markdown pipeline
/// (raw HTML inside a document, a redirect, a script). Handing `javascript:` or
/// `vbscript:` to `NSWorkspace`/`ShellExecuteW` would turn this fix into a way
/// to invoke arbitrary schemes, and `data:` is the usual way to smuggle a
/// payload past a scheme check.
const BLOCKED_SCHEMES: [&str; 3] = ["javascript", "vbscript", "data"];

/// Schemes naming a document the webview itself owns.
///
/// This is not an allowlist of external handlers — it is the opposite. No OS
/// handler exists for either scheme, so both stay in the window. `about:blank`
/// is what `window.open()` opens before a script writes into it, which is
/// precisely how Reveal.js opens its speaker-notes view, and a `blob:` URL can
/// only ever name something the current page created.
const IN_WINDOW_SCHEMES: [&str; 2] = ["about", "blob"];

/// The schemes a browser engine renders itself, and therefore the only ones an
/// `<iframe>` can ever load.
const WEB_SCHEMES: [&str; 2] = ["http", "https"];

/// How a URL's scheme is treated, before origin is considered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchemeClass {
    /// No RFC 3986 scheme at all.
    Absent,
    /// [`BLOCKED_SCHEMES`].
    Blocked,
    /// [`IN_WINDOW_SCHEMES`].
    InWindow,
    /// [`WEB_SCHEMES`] — renderable, and therefore frame-loadable.
    Web,
    /// Anything else: a scheme belonging to some application on this machine.
    Application,
}

/// Classifies `url` by scheme *shape*, never by a list of known applications.
///
/// Enumerating application schemes is what this codebase already refuses to do
/// in [`crate::url_path::is_external_url`], for the same reason: `zoommtg:`,
/// `x-devonthink-item:` and whatever is registered next must work without
/// anybody adding a line here.
fn classify_scheme(url: &str) -> SchemeClass {
    let Some(scheme) = url_scheme(url) else {
        return SchemeClass::Absent;
    };

    if matches_ignore_case(&BLOCKED_SCHEMES, scheme) {
        SchemeClass::Blocked
    } else if matches_ignore_case(&IN_WINDOW_SCHEMES, scheme) {
        SchemeClass::InWindow
    } else if matches_ignore_case(&WEB_SCHEMES, scheme) {
        SchemeClass::Web
    } else {
        SchemeClass::Application
    }
}

/// The policy for wry's navigation handler, which does not know which frame is
/// navigating.
///
/// # `http` and `https` ALWAYS proceed here, cross-origin included
///
/// **This is deliberate. Do not "tighten" it to compare origins.** wry's
/// navigation handler is handed nothing but a URL string:
/// `wry-0.55.1/src/wkwebview/navigation.rs` calls it straight out of
/// `decidePolicyForNavigationAction:` without ever consulting
/// `action.targetFrame.isMainFrame`, and `src/webkitgtk/mod.rs` does the same
/// from `connect_decide_policy`. WebKit calls that delegate for **subframe**
/// navigations too, so every `<iframe>` in the document arrives here looking
/// exactly like a link the user clicked.
///
/// Refusing cross-origin `http(s)` here would therefore cancel embedded content
/// and pop the system browser for it. mbr embeds YouTube cross-origin at
/// `src/media.rs:160` (`https://www.youtube-nocookie.com/embed/{id}`), so that
/// is not hypothetical: every page with a video embed would render a blank
/// frame *and* open a browser tab. Only Windows would escape, because
/// `src/webview2/mod.rs` hooks `add_NavigationStarting` on the top-level
/// `ICoreWebView2`, which fires for document navigations only.
///
/// Cross-origin `http(s)` is instead handled where the frame *is* known: the
/// click listener in `components/src/mbr-link-enhancement.ts` (GUI-only) posts
/// the resolved URL over IPC, and [`parse_ipc_open_request`] re-checks it with
/// the full [`SiteOrigin::decide`] policy before anything reaches the OS.
///
/// Application schemes need no such care: an `<iframe>` cannot navigate to
/// `mailto:` or `zoommtg:`, so acting on them here is structurally safe.
pub fn decide_without_frame_info(url: &str) -> NavigationDecision {
    match classify_scheme(url) {
        SchemeClass::Blocked => NavigationDecision::Block,
        SchemeClass::Absent | SchemeClass::InWindow | SchemeClass::Web => {
            NavigationDecision::Proceed
        }
        SchemeClass::Application => NavigationDecision::OpenExternally,
    }
}

/// The origin mbr's own server is answering on, e.g. `http://127.0.0.1:5220`.
///
/// Held as a string and compared by prefix rather than parsed into a URL type,
/// mirroring [`crate::url_path::is_external_url`]: parsing every navigation
/// would mean trusting a URL parser to agree with WKWebView about schemes like
/// `message:` that no parser normalizes the same way, and the answer we need is
/// only "does this address *our* server?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteOrigin {
    /// `None` when the server URL carried no `://`, which cannot happen for a
    /// URL mbr built itself. See [`SiteOrigin::decide`] for what that costs.
    origin: Option<String>,
}

impl SiteOrigin {
    /// Derives the origin from the URL mbr's server is reachable at.
    ///
    /// The port is part of it, and the port is not fixed: `--port` may be taken,
    /// `start_with_port_retry` moves on, and opening a new folder restarts the
    /// server somewhere else. Callers must rebuild this whenever the server URL
    /// changes or every link becomes "external" the moment the port moves.
    pub fn new(site_url: &str) -> Self {
        Self {
            origin: origin_of(site_url),
        }
    }

    /// The full policy, for callers that know a frame is not involved.
    ///
    /// Used by wry's new-window handler — `window.open()`/`target="_blank"`
    /// never describes an `<iframe>` load — and by [`parse_ipc_open_request`],
    /// where the page has already told us it intercepted a top-level click.
    /// [`decide_without_frame_info`] is the weaker rule the navigation handler
    /// has to settle for, and explains why.
    pub fn decide(&self, url: &str) -> NavigationDecision {
        match classify_scheme(url) {
            SchemeClass::Blocked => NavigationDecision::Block,
            // No scheme at all is a site-relative reference, or the empty URL
            // `window.open()` with no argument produces. Neither names an OS
            // handler.
            SchemeClass::Absent | SchemeClass::InWindow => NavigationDecision::Proceed,
            SchemeClass::Web | SchemeClass::Application => match &self.origin {
                // We could not read our own origin, so we cannot say what is
                // off site. Behave as mbr did before this handler existed
                // rather than hand unclassifiable URLs to the OS.
                None => NavigationDecision::Proceed,
                Some(origin) if covers(origin, url) => NavigationDecision::Proceed,
                Some(_) => NavigationDecision::OpenExternally,
            },
        }
    }
}

/// Validates a message posted over wry's IPC channel, returning the URL that
/// may be opened.
///
/// The payload is page-controlled: anything that can run script in the webview
/// can call `window.ipc.postMessage`, including raw HTML embedded in somebody
/// else's markdown. So the URL is re-checked here rather than trusted, and the
/// check is narrower than [`SiteOrigin::decide`] in both directions:
///
/// - it must be `http(s)`, because that is the only class the page is
///   responsible for — application schemes already work through the navigation
///   handler, and routing them through IPC as well would let a page reach
///   schemes it never had to render a link for;
/// - it must be genuinely off-origin, so a page cannot make mbr fling its own
///   URLs (or a blocked scheme) at the system.
///
/// Returns a borrow of the payload so the URL still reaches the OS byte for
/// byte, with no intermediate parse.
pub fn parse_ipc_open_request<'a>(origin: &SiteOrigin, payload: &'a str) -> Option<&'a str> {
    let url = payload.strip_prefix(IPC_OPEN_EXTERNAL_PREFIX)?;

    let is_web = matches!(classify_scheme(url), SchemeClass::Web);
    (is_web && origin.decide(url) == NavigationDecision::OpenExternally).then_some(url)
}

/// Carries out `decision` for `url`, calling `open_externally` when the answer
/// is [`NavigationDecision::OpenExternally`].
///
/// Returns whether the webview should perform the navigation itself, which is
/// what wry's navigation handler wants; the new-window handler maps the same
/// `bool` onto `Allow`/`Deny`. One function for both keeps the two from
/// drifting apart — a URL refused in-window must not be reachable by opening a
/// window for it instead.
///
/// `open_externally` is a parameter so tests can observe the string that
/// reaches the OS without one running.
pub fn apply_decision<F>(decision: NavigationDecision, url: &str, open_externally: F) -> bool
where
    F: FnOnce(&str),
{
    match decision {
        NavigationDecision::Proceed => true,
        NavigationDecision::OpenExternally => {
            open_externally(url);
            false
        }
        NavigationDecision::Block => {
            tracing::debug!("Refusing navigation to {url}: scheme is not safe to hand off");
            false
        }
    }
}

/// Whether this process is actually running a GUI window.
///
/// A one-way latch: `false` until [`mark_gui_active`], never cleared, because a
/// process either grew a window or it did not.
///
/// [`Ordering::Relaxed`] on both ends is deliberate. Nothing is published
/// *through* this flag — no other memory has to become visible alongside it —
/// so there is no release/acquire pair to establish, and the value itself is
/// atomic at any ordering. The only race a weaker ordering permits is a reader
/// observing a stale `false` shortly after `launch_browser` sets it, and that
/// direction is the safe one: a stale read refuses a launch, it never invents
/// one. A guard that can only ever err toward refusing needs no fence.
static GUI_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Records that this process is launching a GUI window, unlocking
/// [`open_external`].
///
/// Called exactly once, from [`crate::browser::launch_browser`], before the
/// WebView exists — so it is set before anything can possibly ask for a launch,
/// since every request originates in that WebView.
///
/// **No test may call this.** It is process-global and shared by every test in
/// the binary, so setting it would silently disarm
/// [`open_external`]'s guard for whatever runs afterwards. The guard's tests
/// drive [`open_external_guarded`] with an explicit flag instead, which is why
/// that inner function exists.
pub(crate) fn mark_gui_active() {
    GUI_ACTIVE.store(true, Ordering::Relaxed);
}

/// Opens `url` with the operating system's default handler for its scheme.
///
/// Refuses with [`ExternalOpenError::GuiOnly`] unless a GUI window is running in
/// this process; see the module docs for why that is a security property and not
/// a tidiness one. `pub(crate)` for the same reason: the only legitimate caller
/// is `browser.rs`, which is in this crate, and a narrower surface is one fewer
/// thing to keep true by convention.
///
/// Deliberately not a call to `open`/`xdg-open`/`rundll32`: mbr ships as a
/// single self-contained binary and shells out to nothing.
pub(crate) fn open_external(url: &str) -> Result<(), ExternalOpenError> {
    open_external_guarded(GUI_ACTIVE.load(Ordering::Relaxed), url, open_external_impl)
}

/// The guard itself, with the flag and the launcher both passed in.
///
/// Split out so the refusal is testable without touching process-global state:
/// a test that set [`GUI_ACTIVE`] could never be un-set, and would make every
/// later test in the same binary see an armed launcher. Taking `launch` as a
/// parameter is the other half — a test can prove the refusal happened *before*
/// any OS call by passing a launcher that records whether it ran, rather than
/// inferring it from an error type.
fn open_external_guarded<F>(gui_active: bool, url: &str, launch: F) -> Result<(), ExternalOpenError>
where
    F: FnOnce(&str) -> Result<(), ExternalOpenError>,
{
    if !gui_active {
        // `warn`, not `debug`: reaching this branch means something in a
        // process with no window tried to start an application, which is either
        // a bug that drifted in or somebody probing for one. Either way an
        // operator should see it in the log.
        tracing::warn!(
            "Refusing to hand {url} to the operating system: no GUI window is running in this process"
        );
        return Err(ExternalOpenError::GuiOnly {
            url: url.to_string(),
        });
    }

    launch(url)
}

/// Extracts `scheme://authority` from a URL, lowercased for case-insensitive
/// comparison.
///
/// Returns `None` when there is no `://` or the authority is empty, neither of
/// which a server URL mbr built can be.
fn origin_of(site_url: &str) -> Option<String> {
    let authority_start = site_url.find("://")? + "://".len();
    let authority_end = site_url[authority_start..]
        .find(['/', '?', '#'])
        .map_or(site_url.len(), |offset| authority_start + offset);

    (authority_end > authority_start).then(|| site_url[..authority_end].to_ascii_lowercase())
}

/// Whether `url` addresses `origin` itself rather than merely starting with its
/// text.
///
/// The delimiter check is what makes a prefix test safe. Without it
/// `http://127.0.0.1:52200/`, `http://127.0.0.1:5220.evil.example/` and
/// `http://127.0.0.1:5220@evil.example/` all "start with" the origin of a
/// server on port 5220 and would be treated as local.
fn covers(origin: &str, url: &str) -> bool {
    url.get(..origin.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(origin))
        && matches!(
            url.as_bytes().get(origin.len()).copied(),
            None | Some(b'/' | b'?' | b'#')
        )
}

/// Whether `scheme` is one of `known`, compared the way RFC 3986 says schemes
/// compare: case-insensitively.
fn matches_ignore_case(known: &[&str], scheme: &str) -> bool {
    known.iter().any(|known| scheme.eq_ignore_ascii_case(known))
}

#[cfg(target_os = "macos")]
fn open_external_impl(url: &str) -> Result<(), ExternalOpenError> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};

    // `URLWithString:` parses the string as written; an already-escaped `%3C`
    // stays `%3C`. That is the whole reason this takes the raw `&str` rather
    // than anything that has been through a URL type.
    let parsed = NSURL::URLWithString(&NSString::from_str(url)).ok_or_else(|| {
        ExternalOpenError::Malformed {
            url: url.to_string(),
        }
    })?;

    NSWorkspace::sharedWorkspace()
        .openURL(&parsed)
        .then_some(())
        .ok_or_else(|| ExternalOpenError::LaunchFailed {
            url: url.to_string(),
            reason: "no application is registered for this scheme".to_string(),
        })
}

#[cfg(target_os = "windows")]
fn open_external_impl(url: &str) -> Result<(), ExternalOpenError> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    // ShellExecuteW takes UTF-16 and, like every Win32 `W` entry point, expects
    // it nul terminated.
    let wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();

    // SAFETY: the only raw pointer we pass is `wide`, a nul-terminated buffer
    // that outlives the call, and ShellExecuteW does not retain it. A null
    // `hwnd` means "no parent window", and null `lpOperation`/`lpParameters`/
    // `lpDirectory` select the file's default verb and no arguments, all of
    // which the Win32 documentation lists as optional.
    let status = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            std::ptr::null(),
            wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };

    // ShellExecuteW returns a fake HINSTANCE: anything above 32 is success and
    // anything at or below it is one of the SE_ERR_* codes.
    if status as isize > 32 {
        Ok(())
    } else {
        Err(ExternalOpenError::LaunchFailed {
            url: url.to_string(),
            reason: format!("ShellExecuteW failed with code {}", status as isize),
        })
    }
}

#[cfg(target_os = "linux")]
fn open_external_impl(url: &str) -> Result<(), ExternalOpenError> {
    // Straight to gio rather than `gtk::show_uri_on_window`, which wants a
    // window and an event timestamp we would only invent.
    gio::AppInfo::launch_default_for_uri(url, None::<&gio::AppLaunchContext>).map_err(|e| {
        ExternalOpenError::LaunchFailed {
            url: url.to_string(),
            reason: e.to_string(),
        }
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn open_external_impl(url: &str) -> Result<(), ExternalOpenError> {
    Err(ExternalOpenError::LaunchFailed {
        url: url.to_string(),
        reason: "mbr has no system URL handler for this platform".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const SITE: &str = "http://127.0.0.1:5220/";

    fn site() -> SiteOrigin {
        SiteOrigin::new(SITE)
    }

    /// The `message:` URL from the bug report. Its angle brackets are part of
    /// the `Message-ID` and arrive already percent-encoded.
    const MESSAGE_URL: &str =
        "message://%3CCAEn-OzgEreVRuNkdnb9gdFNLXByerYCLraYJGRvjSvXw9chVMQ@mail.gmail.com%3E";

    /// Runs a decision through [`apply_decision`] and reports both halves of
    /// the outcome: what the webview was told, and what the OS was handed.
    fn run(decision: NavigationDecision, url: &str) -> (bool, Option<String>) {
        let handed_off = RefCell::new(None);
        let proceed = apply_decision(decision, url, |u| {
            *handed_off.borrow_mut() = Some(u.to_string());
        });
        (proceed, handed_off.into_inner())
    }

    /// The navigation-handler path: frame-blind.
    fn run_nav(url: &str) -> (bool, Option<String>) {
        run(decide_without_frame_info(url), url)
    }

    /// The new-window path: origin aware.
    fn run_popup(origin: &SiteOrigin, url: &str) -> (bool, Option<String>) {
        run(origin.decide(url), url)
    }

    // ==================== The frame-blind navigation policy ====================

    /// The regression this whole split exists to prevent.
    ///
    /// wry's navigation handler is also called for `<iframe>` loads (it gets a
    /// URL string and no `targetFrame`), so refusing cross-origin `http(s)`
    /// there would blank every embed *and* pop the system browser for it. mbr
    /// embeds YouTube cross-origin at `src/media.rs:160`; this is that exact
    /// URL shape.
    #[test]
    fn test_cross_origin_https_proceeds_so_iframe_embeds_are_never_cancelled() {
        let embed = "https://www.youtube-nocookie.com/embed/abc123";

        assert_eq!(
            decide_without_frame_info(embed),
            NavigationDecision::Proceed,
            "cancelling this would blank YouTube embeds in GUI mode"
        );

        let (proceed, handed_off) = run_nav(embed);
        assert!(proceed, "the embed must be allowed to load");
        assert_eq!(
            handed_off, None,
            "an embed must never open the system browser"
        );
    }

    #[test]
    fn test_decide_without_frame_info_table() {
        let cases: &[(&str, NavigationDecision)] = &[
            // Every http(s) URL proceeds, same origin or not: the handler
            // cannot tell a document navigation from a frame load.
            ("http://127.0.0.1:5220/docs/", NavigationDecision::Proceed),
            ("https://example.com/", NavigationDecision::Proceed),
            ("http://example.com/path", NavigationDecision::Proceed),
            (
                "https://www.youtube-nocookie.com/embed/abc123",
                NavigationDecision::Proceed,
            ),
            ("HTTPS://Example.COM/", NavigationDecision::Proceed),
            // Application schemes: no frame can load one, so acting is safe.
            (MESSAGE_URL, NavigationDecision::OpenExternally),
            (
                "mailto:someone@example.com",
                NavigationDecision::OpenExternally,
            ),
            ("tel:+15555550123", NavigationDecision::OpenExternally),
            (
                "zoommtg://zoom.us/join?confno=1234567890",
                NavigationDecision::OpenExternally,
            ),
            (
                "x-devonthink-item://8A3B0C1D-2E4F",
                NavigationDecision::OpenExternally,
            ),
            (
                "slack://channel?team=T1",
                NavigationDecision::OpenExternally,
            ),
            ("file:///etc/hosts", NavigationDecision::OpenExternally),
            // Script and inline-data schemes stay refused in both policies.
            ("javascript:void(0)", NavigationDecision::Block),
            ("JavaScript:alert(1)", NavigationDecision::Block),
            ("vbscript:msgbox(1)", NavigationDecision::Block),
            (
                "data:text/html;base64,PHNjcmlwdD4=",
                NavigationDecision::Block,
            ),
            // Webview-owned documents, and no scheme at all.
            ("about:blank", NavigationDecision::Proceed),
            (
                "blob:http://127.0.0.1:5220/550e8400",
                NavigationDecision::Proceed,
            ),
            ("", NavigationDecision::Proceed),
            ("/docs/guide/", NavigationDecision::Proceed),
            ("#section", NavigationDecision::Proceed),
        ];

        for (url, expected) in cases {
            assert_eq!(
                decide_without_frame_info(url),
                *expected,
                "decide_without_frame_info({url:?}) should be {expected:?}"
            );
        }
    }

    // ==================== The full, origin-aware policy ====================

    #[test]
    fn test_decide_table() {
        let site = site();
        let cases: &[(&str, NavigationDecision)] = &[
            // Our own server.
            ("http://127.0.0.1:5220/", NavigationDecision::Proceed),
            ("http://127.0.0.1:5220", NavigationDecision::Proceed),
            (
                "http://127.0.0.1:5220/docs/guide/",
                NavigationDecision::Proceed,
            ),
            // Fragments and queries are same-origin too; an in-page anchor must
            // not leave the window.
            (
                "http://127.0.0.1:5220/docs/#section",
                NavigationDecision::Proceed,
            ),
            ("http://127.0.0.1:5220/#top", NavigationDecision::Proceed),
            ("http://127.0.0.1:5220/?q=1", NavigationDecision::Proceed),
            (
                "http://127.0.0.1:5220/.mbr/site.json?v=2#x",
                NavigationDecision::Proceed,
            ),
            // Schemes and hosts compare case-insensitively.
            ("HTTP://127.0.0.1:5220/docs/", NavigationDecision::Proceed),
            // A different host, port or scheme is a different origin.
            ("https://example.com/", NavigationDecision::OpenExternally),
            (
                "http://example.com/path",
                NavigationDecision::OpenExternally,
            ),
            (
                "https://127.0.0.1:5220/",
                NavigationDecision::OpenExternally,
            ),
            ("http://127.0.0.1:5221/", NavigationDecision::OpenExternally),
            ("http://localhost:5220/", NavigationDecision::OpenExternally),
            // Prefix look-alikes: the delimiter check, not the prefix, decides.
            (
                "http://127.0.0.1:52200/",
                NavigationDecision::OpenExternally,
            ),
            (
                "http://127.0.0.1:5220.evil.example/",
                NavigationDecision::OpenExternally,
            ),
            (
                "http://127.0.0.1:5220@evil.example/",
                NavigationDecision::OpenExternally,
            ),
            // Application schemes.
            (MESSAGE_URL, NavigationDecision::OpenExternally),
            (
                "mailto:someone@example.com",
                NavigationDecision::OpenExternally,
            ),
            (
                "mailto:someone@example.com?subject=Hi%20there",
                NavigationDecision::OpenExternally,
            ),
            ("tel:+15555550123", NavigationDecision::OpenExternally),
            (
                "zoommtg://zoom.us/join?confno=1234567890",
                NavigationDecision::OpenExternally,
            ),
            (
                "x-devonthink-item://8A3B0C1D-2E4F",
                NavigationDecision::OpenExternally,
            ),
            (
                "slack://channel?team=T1&id=C1",
                NavigationDecision::OpenExternally,
            ),
            ("file:///etc/hosts", NavigationDecision::OpenExternally),
            // Script and inline-data schemes: neither in-window nor to the OS.
            ("javascript:void(0)", NavigationDecision::Block),
            ("JavaScript:alert(1)", NavigationDecision::Block),
            ("vbscript:msgbox(1)", NavigationDecision::Block),
            (
                "data:text/html;base64,PHNjcmlwdD4=",
                NavigationDecision::Block,
            ),
            ("DATA:text/html,<b>x</b>", NavigationDecision::Block),
            // The webview's own documents. `about:blank` in particular is what
            // Reveal.js opens its speaker-notes window with, which is why the
            // new-window handler must still answer Allow for it.
            ("about:blank", NavigationDecision::Proceed),
            ("about:srcdoc", NavigationDecision::Proceed),
            (
                "blob:http://127.0.0.1:5220/550e8400-e29b",
                NavigationDecision::Proceed,
            ),
            // No scheme: nothing to hand off, so the webview keeps it.
            ("", NavigationDecision::Proceed),
            ("/docs/guide/", NavigationDecision::Proceed),
            ("#section", NavigationDecision::Proceed),
        ];

        for (url, expected) in cases {
            assert_eq!(
                site.decide(url),
                *expected,
                "decide({url:?}) should be {expected:?}"
            );
        }
    }

    // ==================== Byte-for-byte hand-off ====================

    /// The core of the bug report: a `message:` URL must reach the OS with its
    /// percent-encoding untouched. A decoded `<` or a double-encoded `%253C`
    /// yields a `Message-ID` the mail client cannot find. This goes through the
    /// navigation handler, which is the path such a link actually takes.
    #[test]
    fn test_percent_encoding_reaches_the_os_verbatim() {
        let (proceed, handed_off) = run_nav(MESSAGE_URL);

        assert!(!proceed, "an external URL must not also load in-window");
        let handed_off = handed_off.expect("message: URL should reach the OS");
        assert_eq!(handed_off, MESSAGE_URL);
        assert!(
            handed_off.contains("%3C"),
            "leading %3C must not be decoded"
        );
        assert!(
            handed_off.contains("%3E"),
            "trailing %3E must not be decoded"
        );
        assert!(!handed_off.contains("%253C"), "must not be re-encoded");
    }

    /// Every shape that is not plain ASCII path text survives the hand-off
    /// unchanged, since we never parse and reserialize.
    #[test]
    fn test_external_urls_are_never_rewritten() {
        let site = site();
        let urls = [
            "https://example.com/a%20b/c?d=%26e#f%2Fg",
            "https://example.com/caf%C3%A9",
            "https://example.com/café",
            "mailto:a@b.example?subject=%5Bmbr%5D%20hi&body=one%0Atwo",
            "zoommtg://zoom.us/join?confno=1&pwd=%2Fslash%2B",
        ];

        for url in urls {
            let (proceed, handed_off) = run_popup(&site, url);
            assert!(!proceed, "{url} should not navigate in-window");
            assert_eq!(handed_off.as_deref(), Some(url), "{url} was rewritten");
        }
    }

    #[test]
    fn test_blocked_schemes_reach_neither_the_window_nor_the_os() {
        let site = site();

        for url in [
            "javascript:alert(1)",
            "vbscript:msgbox(1)",
            "data:text/html,x",
        ] {
            for (proceed, handed_off) in [run_nav(url), run_popup(&site, url)] {
                assert!(!proceed, "{url} must not navigate in-window");
                assert_eq!(handed_off, None, "{url} must never reach the OS");
            }
        }
    }

    #[test]
    fn test_same_origin_navigations_are_not_handed_off() {
        let site = site();

        for url in [
            "http://127.0.0.1:5220/",
            "http://127.0.0.1:5220/docs/#anchor",
            "about:blank",
        ] {
            for (proceed, handed_off) in [run_nav(url), run_popup(&site, url)] {
                assert!(proceed, "{url} should navigate in-window");
                assert_eq!(handed_off, None, "{url} must not reach the OS");
            }
        }
    }

    // ==================== IPC payload validation ====================

    /// The happy path the click listener in `mbr-link-enhancement.ts` produces.
    #[test]
    fn test_ipc_accepts_an_off_origin_web_url_verbatim() {
        let site = site();

        assert_eq!(
            parse_ipc_open_request(
                &site,
                "mbr:open-external:https://example.com/a%20b?c=%26d#e"
            ),
            Some("https://example.com/a%20b?c=%26d#e"),
            "the URL must survive the round trip unchanged"
        );
        assert_eq!(
            parse_ipc_open_request(&site, "mbr:open-external:http://example.com/"),
            Some("http://example.com/")
        );
    }

    /// The payload is page-controlled, so every one of these has to bounce.
    #[test]
    fn test_ipc_rejects_everything_a_hostile_page_could_post() {
        let site = site();
        let rejected = [
            // Not our message at all.
            "hello",
            "",
            "mbr:open-external",
            "open-external:https://example.com/",
            " mbr:open-external:https://example.com/",
            // Blocked schemes must not gain a second route to the OS.
            "mbr:open-external:javascript:alert(1)",
            "mbr:open-external:vbscript:msgbox(1)",
            "mbr:open-external:data:text/html;base64,PHNjcmlwdD4=",
            // Same origin: a page must not make mbr fling its own URLs at the
            // system browser.
            "mbr:open-external:http://127.0.0.1:5220/",
            "mbr:open-external:http://127.0.0.1:5220/docs/guide/",
            // Application schemes already work through the navigation handler;
            // IPC must not become a way to reach them.
            "mbr:open-external:mailto:someone@example.com",
            "mbr:open-external:zoommtg://zoom.us/join?confno=1",
            "mbr:open-external:file:///etc/passwd",
            // Webview-internal and scheme-less shapes.
            "mbr:open-external:about:blank",
            "mbr:open-external:blob:http://127.0.0.1:5220/550e8400",
            "mbr:open-external:/docs/guide/",
            "mbr:open-external:#anchor",
            "mbr:open-external:",
        ];

        for payload in rejected {
            assert_eq!(
                parse_ipc_open_request(&site, payload),
                None,
                "IPC payload {payload:?} must be refused"
            );
        }
    }

    /// Origin look-alikes are different origins, and have to be treated as such
    /// on the IPC path too rather than mistaken for our own server.
    #[test]
    fn test_ipc_is_not_fooled_by_origin_lookalikes() {
        let site = site();

        for url in [
            "http://127.0.0.1:5220.evil.example/",
            "http://127.0.0.1:5220@evil.example/",
            "http://127.0.0.1:52200/",
        ] {
            assert_eq!(
                parse_ipc_open_request(&site, &format!("{IPC_OPEN_EXTERNAL_PREFIX}{url}")),
                Some(url),
                "{url} is a different origin and must be allowed out"
            );
        }
    }

    /// With no origin to compare against we cannot prove a URL is off-site, so
    /// the page gets nothing rather than the benefit of the doubt.
    #[test]
    fn test_ipc_refuses_everything_when_the_origin_is_unknown() {
        let unknown = SiteOrigin::new("not a url");

        assert_eq!(
            parse_ipc_open_request(&unknown, "mbr:open-external:https://example.com/"),
            None
        );
    }

    // ==================== The GUI-only launcher guard ====================

    /// A stand-in for the platform launcher that records whether it ran.
    ///
    /// Every guard test uses one, so "the OS was never called" is an assertion
    /// about an observed fact rather than an inference from the error type.
    fn recording_launcher(
        seen: &RefCell<Vec<String>>,
    ) -> impl FnOnce(&str) -> Result<(), ExternalOpenError> + '_ {
        move |url| {
            seen.borrow_mut().push(url.to_string());
            Ok(())
        }
    }

    /// The security regression test.
    ///
    /// mbr must never be usable as a way to start applications on a machine
    /// that is merely *serving* markdown. No HTTP handler calls `open_external`
    /// today, but the `gui` feature is on by default, so a server-mode process
    /// links this launcher; the flag is what keeps it inert. If this test ever
    /// fails, a server can be induced to launch applications on its host.
    #[test]
    fn open_external_refuses_when_gui_is_not_running() {
        let reached_os = RefCell::new(Vec::new());

        let result = open_external_guarded(false, MESSAGE_URL, recording_launcher(&reached_os));

        assert!(
            matches!(result, Err(ExternalOpenError::GuiOnly { ref url }) if url == MESSAGE_URL),
            "a non-GUI process must refuse with GuiOnly, got {result:?}"
        );
        assert!(
            reached_os.into_inner().is_empty(),
            "the refusal must happen BEFORE the operating system is touched"
        );
    }

    /// The same refusal for every class of URL that survives the policy, so the
    /// guard cannot be mistaken for something scheme-specific. `http(s)` is in
    /// the list because that is what the IPC path hands over.
    #[test]
    fn open_external_refuses_every_url_when_gui_is_not_running() {
        for url in [
            MESSAGE_URL,
            "mailto:someone@example.com",
            "zoommtg://zoom.us/join?confno=1234567890",
            "file:///etc/passwd",
            "https://example.com/",
            "x-devonthink-item://8A3B0C1D-2E4F",
            "",
        ] {
            let reached_os = RefCell::new(Vec::new());

            let result = open_external_guarded(false, url, recording_launcher(&reached_os));

            assert!(
                matches!(result, Err(ExternalOpenError::GuiOnly { .. })),
                "{url:?} must be refused outside GUI mode, got {result:?}"
            );
            assert!(
                reached_os.into_inner().is_empty(),
                "{url:?} must not reach the operating system outside GUI mode"
            );
        }
    }

    /// The other half: the guard is a gate, not a wall. With a window running,
    /// the URL reaches the launcher unchanged — the byte-for-byte promise has to
    /// survive the guard too.
    #[test]
    fn open_external_reaches_the_launcher_verbatim_when_the_gui_is_running() {
        let reached_os = RefCell::new(Vec::new());

        let result = open_external_guarded(true, MESSAGE_URL, recording_launcher(&reached_os));

        assert!(result.is_ok(), "a GUI process may launch, got {result:?}");
        assert_eq!(
            reached_os.into_inner(),
            vec![MESSAGE_URL.to_string()],
            "the launcher must see the URL exactly as the webview gave it"
        );
    }

    /// A launcher failure is reported as itself, not swallowed or relabelled as
    /// a refusal — the two mean very different things in a log.
    #[test]
    fn open_external_surfaces_launcher_failures_unchanged() {
        let result = open_external_guarded(true, "zoommtg://zoom.us/join", |url| {
            Err(ExternalOpenError::LaunchFailed {
                url: url.to_string(),
                reason: "no application is registered for this scheme".to_string(),
            })
        });

        assert!(
            matches!(result, Err(ExternalOpenError::LaunchFailed { .. })),
            "expected the launcher's own error, got {result:?}"
        );
    }

    /// Proves the *real* entry point is wired to the latch, which the tests
    /// above deliberately bypass.
    ///
    /// Deterministic because `mark_gui_active` is called from exactly one place
    /// — `browser::launch_browser` — and no test opens a window. Its doc comment
    /// forbids calling it from a test for precisely this reason; if that ever
    /// changes, this test is the thing that notices.
    #[test]
    fn open_external_is_wired_to_the_gui_latch_and_defaults_to_refusing() {
        assert!(
            !GUI_ACTIVE.load(Ordering::Relaxed),
            "no test may call mark_gui_active(); the latch must still be closed here"
        );

        assert!(
            matches!(
                open_external("https://example.com/"),
                Err(ExternalOpenError::GuiOnly { .. })
            ),
            "open_external must consult the latch, not just open_external_guarded"
        );
    }

    /// The message an operator reads has to say *why* it was refused, since the
    /// obvious guess ("no handler for that scheme") is wrong and would send them
    /// looking at the wrong machine.
    #[test]
    fn gui_only_refusal_explains_itself() {
        let message = ExternalOpenError::GuiOnly {
            url: "https://example.com/".to_string(),
        }
        .to_string();

        assert!(message.contains("https://example.com/"), "{message}");
        assert!(message.contains("GUI-only"), "{message}");
    }

    // ==================== Origin handling ====================

    #[test]
    fn test_origin_of() {
        assert_eq!(
            origin_of("http://127.0.0.1:5220/"),
            Some("http://127.0.0.1:5220".to_string())
        );
        assert_eq!(
            origin_of("http://0.0.0.0:8080/docs/guide/"),
            Some("http://0.0.0.0:8080".to_string())
        );
        // No trailing slash, and mixed case, are both normalized.
        assert_eq!(
            origin_of("HTTP://LocalHost:5220"),
            Some("http://localhost:5220".to_string())
        );
        // Shapes a server URL can never take.
        assert_eq!(origin_of("not a url"), None);
        assert_eq!(origin_of("http:///no-authority"), None);
    }

    /// The port is not stable — `--port` may be taken and opening a new folder
    /// restarts the server — so a stale origin would send every internal link
    /// to the system browser.
    #[test]
    fn test_origin_is_rebuilt_when_the_server_moves() {
        let moved = SiteOrigin::new("http://127.0.0.1:5301/");

        assert_eq!(
            moved.decide("http://127.0.0.1:5301/docs/"),
            NavigationDecision::Proceed
        );
        assert_eq!(
            moved.decide("http://127.0.0.1:5220/docs/"),
            NavigationDecision::OpenExternally
        );
    }

    /// A server URL with no `://` cannot happen, but if it did we must not
    /// start posting every link to the operating system.
    #[test]
    fn test_unreadable_origin_keeps_navigations_in_window() {
        let unknown = SiteOrigin::new("not a url");

        assert_eq!(
            unknown.decide("https://example.com/"),
            NavigationDecision::Proceed
        );
        // Blocking still applies: it does not depend on knowing our origin.
        assert_eq!(
            unknown.decide("javascript:void(0)"),
            NavigationDecision::Block
        );
    }
}

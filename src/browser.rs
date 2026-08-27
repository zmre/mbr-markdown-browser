extern crate image;
use crate::Config;
use crate::config::MenuBarVisibility;
use crate::errors::BrowserError;
use crate::external_open::{
    SiteOrigin, apply_decision, decide_without_frame_info, mark_gui_active, open_external,
    parse_ipc_open_request,
};
use crate::server::{Server, ServerConfig};
use muda::{
    AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
use parking_lot::RwLock;
use std::path::PathBuf;
use std::sync::Arc;
use tao::{
    event::{ElementState, Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    keyboard::{Key, ModifiersState},
    window::{Icon, WindowBuilder},
};
// Only the non-Linux history arm reads raw key codes; Linux routes every
// shortcut through `linux_shortcut_for`, which matches on `logical_key`.
#[cfg(not(target_os = "linux"))]
use tao::keyboard::KeyCode;
use tokio::task::JoinHandle;
#[cfg(target_os = "linux")]
use wry::WebViewBuilderExtUnix;
use wry::{NewWindowResponse, WebViewBuilder};

// Scripts driving the GUI-only `<mbr-find-bar>` element from the native Edit menu.
//
// These are `&'static str` rather than a function returning `String` so the event loop
// never allocates to dispatch a keystroke.
//
// NOTE: a Rust line-continuation backslash strips the newline *and* all leading
// whitespace on the next line, so every JS statement below must end with `;`. There is
// no newline left for ASI to insert one.

/// Open the find bar, polling until the custom element has upgraded.
///
/// `templates/_scripts.html` loads the components bundle with `defer type="module"`, so
/// the element may not be defined yet when the first Cmd+F lands. The retry is bounded
/// (40 attempts x 25ms = 1s) and cancels itself on success. `open()` is idempotent on the
/// TS side, so a duplicate menu event cannot toggle the bar shut.
const FIND_OPEN_SCRIPT: &str = "(()=>{let n=0;const go=()=>{\
    const e=document.querySelector('mbr-find-bar');\
    if(e&&typeof e.open==='function'){e.open();return;}\
    if(++n<40)setTimeout(go,25);\
    };go();})()";

/// Advance to the next match. Single-shot: the bar must already be open to have matches.
const FIND_NEXT_SCRIPT: &str = "(()=>{\
    const e=document.querySelector('mbr-find-bar');\
    if(e&&typeof e.findNext==='function')e.findNext();\
    })()";

/// Step back to the previous match. Single-shot, as with `FIND_NEXT_SCRIPT`.
const FIND_PREV_SCRIPT: &str = "(()=>{\
    const e=document.querySelector('mbr-find-bar');\
    if(e&&typeof e.findPrevious==='function')e.findPrevious();\
    })()";

/// Custom user events for the event loop
enum UserEvent {
    MenuEvent(MenuEvent),
    FolderSelected(PathBuf),
    /// A server asked for by `Open Folder…` is listening.
    ///
    /// Carries the `JoinHandle` so the event loop — the only place that knows
    /// which server is current — can abort the one being replaced.
    ServerReady {
        handle: JoinHandle<()>,
        url: String,
    },
    /// A server asked for by `Open Folder…` never came up. The old one is still
    /// running, so the window keeps working; `path` is only for the message.
    ServerFailed {
        path: PathBuf,
    },
}

/// Context needed to launch and manage the browser window
pub struct BrowserContext {
    pub url: String,
    pub server_handle: JoinHandle<()>,
    pub config: Config,
    pub tokio_runtime: tokio::runtime::Handle,
}

/// About metadata for the application
fn about_metadata() -> AboutMetadata {
    AboutMetadata {
        name: Some("mbr".to_string()),
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        short_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        authors: Some(vec!["zmre".to_string()]),
        comments: Some("A markdown viewer and browser".to_string()),
        copyright: Some("Copyright © 2025".to_string()),
        license: Some("MIT".to_string()),
        website: Some("https://github.com/zmre/mbr".to_string()),
        website_label: Some("GitHub".to_string()),
        ..Default::default()
    }
}

/// Log (rather than panic on) a menu construction failure.
///
/// A failed append leaves that menu degraded or empty, which is a cosmetic
/// problem; crashing the GUI at startup would be far worse.
fn log_menu_result(what: &str, result: Result<(), muda::Error>) {
    if let Err(e) = result {
        tracing::error!("Failed to append {what}: {e}");
    }
}

/// Menu items for history navigation
struct HistoryMenuItems {
    back: MenuItem,
    forward: MenuItem,
}

/// Menu items for find-in-page
///
/// wry wraps a bare webview with no browser chrome, so nothing claims Cmd+F.
/// These items drive the GUI-only `<mbr-find-bar>` element via `evaluate_script`.
struct FindMenuItems {
    open: MenuItem,
    next: MenuItem,
    prev: MenuItem,
}

/// Handles to menu items needed for event matching after the menu bar is built
struct MenuHandles {
    menu_bar: Menu,
    open_item: MenuItem,
    reload_item: MenuItem,
    print_item: MenuItem,
    history_items: HistoryMenuItems,
    find_items: FindMenuItems,
    window_menu: Submenu,
}

/// Build the application menu bar with standard menus
/// On macOS, creates proper app menu with About, Services, Hide, Quit
/// On Windows/Linux, puts About in Help menu and Quit in File menu
fn build_menu_bar() -> MenuHandles {
    let menu_bar = Menu::new();

    // macOS: First menu is the app menu (named after the app)
    // Contains About, Services, Hide, Hide Others, Show All, Quit
    #[cfg(target_os = "macos")]
    let app_menu = {
        let app_menu = Submenu::new("mbr", true);
        log_menu_result(
            "app menu items",
            app_menu.append_items(&[
                &PredefinedMenuItem::about(None, Some(about_metadata())),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::services(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::quit(None),
            ]),
        );
        app_menu
    };

    // File menu
    let file_menu = Submenu::new("&File", true);

    // The "command" modifier: Cmd on macOS, Ctrl elsewhere.
    //
    // `Modifiers::SUPER` is Cmd on macOS but the *Super/Windows* key on Linux and
    // Windows, where it belongs to the desktop — a tiling compositor such as
    // Hyprland binds nearly the whole Super range, so a menu item accelerated
    // with it is not merely non-standard, it never fires. Every accelerator that
    // is Cmd-something on macOS has to make this choice; the ones below that
    // already spell it out inline predate this constant.
    #[cfg(target_os = "macos")]
    let command_modifier = Modifiers::SUPER;
    #[cfg(not(target_os = "macos"))]
    let command_modifier = Modifiers::CONTROL;

    let open_item = MenuItem::with_id(
        "open",
        "&Open...",
        true,
        Some(Accelerator::new(Some(command_modifier), Code::KeyO)),
    );

    let reload_item = MenuItem::with_id(
        "reload",
        "&Reload",
        true,
        Some(Accelerator::new(Some(command_modifier), Code::KeyR)),
    );

    // Print is Cmd+P on macOS but **Ctrl+Shift+P** elsewhere. Plain Ctrl+P is
    // "previous item" in mbr's own lists (the readline pair with Ctrl+N), and a
    // native accelerator would take it away from the page it belongs to.
    #[cfg(target_os = "macos")]
    let print_modifier = command_modifier;
    #[cfg(not(target_os = "macos"))]
    let print_modifier = Modifiers::CONTROL | Modifiers::SHIFT;

    let print_item = MenuItem::with_id(
        "print",
        "&Print…",
        true,
        Some(Accelerator::new(Some(print_modifier), Code::KeyP)),
    );

    #[cfg(target_os = "macos")]
    log_menu_result(
        "file menu items",
        file_menu.append_items(&[
            &open_item,
            &PredefinedMenuItem::separator(),
            &reload_item,
            &PredefinedMenuItem::separator(),
            &print_item,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::close_window(Some("Close Window")),
        ]),
    );

    #[cfg(not(target_os = "macos"))]
    log_menu_result(
        "file menu items",
        file_menu.append_items(&[
            &open_item,
            &PredefinedMenuItem::separator(),
            &reload_item,
            &PredefinedMenuItem::separator(),
            &print_item,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::close_window(Some("Close Window")),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ]),
    );

    // Find uses Cmd+F / Cmd+G / Shift+Cmd+G on macOS, Ctrl+F / F3 / Shift+F3 elsewhere.
    // F3 rather than Ctrl+G off macOS: Ctrl+G is already the info panel's binding.
    #[cfg(target_os = "macos")]
    let find_accelerator = Accelerator::new(Some(Modifiers::SUPER), Code::KeyF);
    #[cfg(not(target_os = "macos"))]
    let find_accelerator = Accelerator::new(Some(Modifiers::CONTROL), Code::KeyF);

    #[cfg(target_os = "macos")]
    let find_next_accelerator = Accelerator::new(Some(Modifiers::SUPER), Code::KeyG);
    #[cfg(not(target_os = "macos"))]
    let find_next_accelerator = Accelerator::new(None, Code::F3);

    #[cfg(target_os = "macos")]
    let find_prev_accelerator =
        Accelerator::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyG);
    #[cfg(not(target_os = "macos"))]
    let find_prev_accelerator = Accelerator::new(Some(Modifiers::SHIFT), Code::F3);

    let find_item = MenuItem::with_id("find", "&Find…", true, Some(find_accelerator));
    let find_next_item =
        MenuItem::with_id("find_next", "Find &Next", true, Some(find_next_accelerator));
    let find_prev_item = MenuItem::with_id(
        "find_prev",
        "Find &Previous",
        true,
        Some(find_prev_accelerator),
    );

    // Edit menu with standard clipboard operations
    let edit_menu = Submenu::new("&Edit", true);
    log_menu_result(
        "edit menu items",
        edit_menu.append_items(&[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
            &PredefinedMenuItem::separator(),
            &find_item,
            &find_next_item,
            &find_prev_item,
        ]),
    );

    // View menu
    let view_menu = Submenu::new("&View", true);
    // Cmd+Option+I on macOS, Ctrl+Shift+I elsewhere -- the inspector shortcut
    // every browser uses on that platform.
    #[cfg(target_os = "macos")]
    let devtools_modifiers = Modifiers::SUPER | Modifiers::ALT;
    #[cfg(not(target_os = "macos"))]
    let devtools_modifiers = Modifiers::CONTROL | Modifiers::SHIFT;

    let devtools_item = MenuItem::with_id(
        "devtools",
        "Toggle Developer Tools",
        true,
        Some(Accelerator::new(Some(devtools_modifiers), Code::KeyI)),
    );
    log_menu_result(
        "view menu items",
        view_menu.append_items(&[
            &PredefinedMenuItem::fullscreen(None),
            &PredefinedMenuItem::separator(),
            &devtools_item,
        ]),
    );

    // History menu with Back/Forward navigation
    let history_menu = Submenu::new("&History", true);
    // Cmd+[ / Cmd+] is the macOS convention; Alt+arrow is everyone else's, and
    // is also what the keyboard route below already answers, so the menu now
    // advertises the key that actually works there.
    #[cfg(target_os = "macos")]
    let (back_accelerator, forward_accelerator) = (
        Accelerator::new(Some(Modifiers::SUPER), Code::BracketLeft),
        Accelerator::new(Some(Modifiers::SUPER), Code::BracketRight),
    );
    #[cfg(not(target_os = "macos"))]
    let (back_accelerator, forward_accelerator) = (
        Accelerator::new(Some(Modifiers::ALT), Code::ArrowLeft),
        Accelerator::new(Some(Modifiers::ALT), Code::ArrowRight),
    );

    let back_item = MenuItem::with_id("back", "&Back", true, Some(back_accelerator));
    let forward_item = MenuItem::with_id("forward", "&Forward", true, Some(forward_accelerator));
    log_menu_result(
        "history menu items",
        history_menu.append_items(&[&back_item, &forward_item]),
    );

    let history_items = HistoryMenuItems {
        back: back_item,
        forward: forward_item,
    };

    // Window menu
    let window_menu = Submenu::new("&Window", true);
    log_menu_result(
        "window menu items",
        window_menu.append_items(&[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::bring_all_to_front(None),
        ]),
    );

    // Help menu - only needed on non-macOS for About
    #[cfg(not(target_os = "macos"))]
    let help_menu = {
        let help_menu = Submenu::new("&Help", true);
        log_menu_result(
            "help menu items",
            help_menu.append_items(&[&PredefinedMenuItem::about(None, Some(about_metadata()))]),
        );
        help_menu
    };

    // Build menu bar - order matters, especially on macOS
    #[cfg(target_os = "macos")]
    log_menu_result(
        "menus to menu bar",
        menu_bar.append_items(&[
            &app_menu,
            &file_menu,
            &edit_menu,
            &view_menu,
            &history_menu,
            &window_menu,
        ]),
    );

    #[cfg(not(target_os = "macos"))]
    log_menu_result(
        "menus to menu bar",
        menu_bar.append_items(&[
            &file_menu,
            &edit_menu,
            &view_menu,
            &history_menu,
            &window_menu,
            &help_menu,
        ]),
    );

    // On macOS, set the Window menu as the windows menu for proper window management
    #[cfg(target_os = "macos")]
    window_menu.set_as_windows_menu_for_nsapp();

    let find_items = FindMenuItems {
        open: find_item,
        next: find_next_item,
        prev: find_prev_item,
    };

    MenuHandles {
        menu_bar,
        open_item,
        reload_item,
        print_item,
        history_items,
        find_items,
        window_menu,
    }
}

/// Hand an off-site URL to the operating system's default handler.
///
/// A failure is logged rather than surfaced: the in-window navigation has
/// already been refused by the time we get here, and there is nothing useful a
/// dialog could offer for "no application claims `zoommtg:`".
fn open_with_system_handler(url: &str) {
    tracing::debug!("Opening {url} with the system default handler");
    if let Err(e) = open_external(url) {
        tracing::warn!("Could not open {url} externally: {e}");
    }
}

/// Spawn a thread to show folder picker dialog and send result via event loop proxy
fn spawn_folder_picker(proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Open Markdown Folder")
            .pick_folder()
        {
            let _ = proxy.send_event(UserEvent::FolderSelected(path));
        }
    });
}

/// Start a server for `path` and resolve once it is listening.
///
/// **Async on purpose.** This used to be a synchronous `reinit_server` that
/// spawned the server and then called `Handle::block_on` to wait for its port —
/// from the tao event-loop callback, which `main` runs *inside* `#[tokio::main]`'s
/// runtime. Blocking a runtime thread on that runtime is an immediate panic:
///
/// > Cannot start a runtime from within a runtime. This happens because a
/// > function (like `block_on`) attempted to block the current thread while the
/// > thread is being used to drive asynchronous tasks.
///
/// so **Open Folder aborted the process on every platform**, the moment a folder
/// was chosen. The port now comes back through the event loop instead
/// ([`UserEvent::ServerReady`]), which also keeps the window responsive while a
/// large repository is scanned.
async fn start_server_for(path: PathBuf) -> Result<(JoinHandle<()>, String), BrowserError> {
    let absolute_path = path.canonicalize().map_err(|e| {
        tracing::error!("Failed to canonicalize path: {e}");
        BrowserError::ServerStartFailed
    })?;

    let config = Config::read(&absolute_path).map_err(|e| {
        tracing::error!("Failed to read config: {e}");
        BrowserError::ServerStartFailed
    })?;

    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<u16>();

    let config_copy = config.clone();
    let handle = tokio::spawn(async move {
        let server_config = ServerConfig::from(&config_copy).with_gui_mode(true);
        match Server::init(server_config) {
            Ok(mut s) => {
                if let Err(e) = s.start_with_port_retry(Some(ready_tx), 10).await {
                    tracing::error!("Server error: {e}");
                }
            }
            Err(e) => {
                tracing::error!("Server init failed: {e}");
                // Dropping the sender is what tells the awaiter below that this
                // will never be ready; without it that await would hang forever.
                drop(ready_tx);
            }
        }
    });

    let port = ready_rx.await.map_err(|_| {
        // The task dropped the sender, so it has already logged the cause.
        handle.abort();
        BrowserError::ServerStartFailed
    })?;

    Ok((handle, format!("http://{}:{}/", config.host, port)))
}

/// Whether the platform shows the menu bar by default under
/// [`MenuBarVisibility::Auto`].
///
/// False on Linux and true everywhere else, because the *cost* of the bar
/// differs by platform rather than its usefulness. macOS renders the menu in
/// the system-wide bar at the top of the screen, which the window does not pay
/// for; Windows treats an in-window bar as the native convention. Only on Linux
/// is it a `GtkMenuBar` stacked above the page — chrome no other GTK app of this
/// shape has shown since the header-bar era, and under a tiling Wayland
/// compositor there is no global-menu protocol to move it to.
const MENU_BAR_AUTO_VISIBLE: bool = !cfg!(target_os = "linux");

/// Resolve `gui_menu_bar` against the platform default.
///
/// `auto` is a parameter rather than a read of [`MENU_BAR_AUTO_VISIBLE`] so the
/// mapping is testable for both platforms on either one.
fn menu_bar_starts_visible(setting: MenuBarVisibility, auto: bool) -> bool {
    match setting {
        MenuBarVisibility::Auto => auto,
        MenuBarVisibility::Always => true,
        MenuBarVisibility::Never => false,
    }
}

/// Whether F10 may reveal or dismiss the bar at runtime.
///
/// `never` means never: a user who has turned the bar off in `config.toml` did
/// not ask for a key that brings it back. `auto` and `always` both toggle, so
/// the Linux default — hidden — is still one keystroke from discoverable.
fn menu_bar_toggle_allowed(setting: MenuBarVisibility) -> bool {
    !matches!(setting, MenuBarVisibility::Never)
}

/// Show or hide the GTK menu bar attached to `window`.
///
/// Hiding the bar does **not** disable any of its actions. `muda` adds its
/// `GtkAccelGroup` to the *window* in `init_for_gtk_window` and never removes it
/// in `hide_for_gtk_window`, so Ctrl+O, Ctrl+P, Ctrl+R, Ctrl+F and the rest keep
/// firing with nothing on screen. That is the entire argument for hiding by
/// default rather than dropping the menu.
///
/// `set_no_show_all(true)` is what makes a hide durable. `gtk_widget_show_all`
/// recurses through the whole tree, so any later `show_all()` on the window —
/// tao's, a theme reload's — would undo a plain `hide()`. The flag is cleared
/// again when showing, so the bar's own `show_all()` in `muda` still works.
/// A window action reachable both from a menu item and from a key.
///
/// One enum for both routes so each action has exactly one implementation in
/// [`perform_shortcut`]. The menu route exists on every platform; the keyboard
/// route is Linux-only, for the reason given on [`linux_shortcut_for`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shortcut {
    OpenFolder,
    Reload,
    Print,
    Back,
    Forward,
    FindOpen,
    FindNext,
    FindPrev,
    Quit,
}

/// The menu-item ids that map to a [`Shortcut`].
///
/// `Quit` has no entry: it is a `PredefinedMenuItem`, which the platform
/// activates itself and which never reaches our handler. It is in `Shortcut`
/// only because the keyboard route has to cover it when the bar is hidden.
struct ShortcutIds {
    open: muda::MenuId,
    reload: muda::MenuId,
    print: muda::MenuId,
    back: muda::MenuId,
    forward: muda::MenuId,
    find: muda::MenuId,
    find_next: muda::MenuId,
    find_prev: muda::MenuId,
}

fn shortcut_for_menu_id(id: &muda::MenuId, ids: &ShortcutIds) -> Option<Shortcut> {
    let table = [
        (&ids.open, Shortcut::OpenFolder),
        (&ids.reload, Shortcut::Reload),
        (&ids.print, Shortcut::Print),
        (&ids.back, Shortcut::Back),
        (&ids.forward, Shortcut::Forward),
        (&ids.find, Shortcut::FindOpen),
        (&ids.find_next, Shortcut::FindNext),
        (&ids.find_prev, Shortcut::FindPrev),
    ];
    table
        .into_iter()
        .find(|(candidate, _)| *candidate == id)
        .map(|(_, shortcut)| shortcut)
}

/// Carry out an action, whichever route asked for it.
///
/// `evaluate_script` failures are dropped the way the menu handler always has:
/// a webview that cannot run `history.back()` has already gone, and there is
/// nothing a message could offer the user.
fn perform_shortcut(
    shortcut: Shortcut,
    webview: &wry::WebView,
    current_url: &str,
    proxy: &EventLoopProxy<UserEvent>,
    control_flow: &mut ControlFlow,
) {
    match shortcut {
        Shortcut::OpenFolder => {
            tracing::debug!("Open folder requested");
            spawn_folder_picker(proxy.clone());
        }
        Shortcut::Reload => {
            tracing::debug!("Reload requested");
            let _ = webview.load_url(current_url);
        }
        Shortcut::Print => {
            tracing::debug!("Print requested");
            if let Err(e) = webview.print() {
                tracing::error!("Print failed: {e}");
            }
        }
        Shortcut::Back => {
            tracing::debug!("History back requested");
            let _ = webview.evaluate_script("history.back()");
        }
        Shortcut::Forward => {
            tracing::debug!("History forward requested");
            let _ = webview.evaluate_script("history.forward()");
        }
        Shortcut::FindOpen => {
            tracing::debug!("Find requested");
            let _ = webview.evaluate_script(FIND_OPEN_SCRIPT);
        }
        Shortcut::FindNext => {
            tracing::debug!("Find next requested");
            let _ = webview.evaluate_script(FIND_NEXT_SCRIPT);
        }
        Shortcut::FindPrev => {
            tracing::debug!("Find previous requested");
            let _ = webview.evaluate_script(FIND_PREV_SCRIPT);
        }
        Shortcut::Quit => {
            tracing::debug!("Quit requested");
            *control_flow = ControlFlow::Exit;
        }
    }
}

/// The action a key press should perform **while the menu bar is hidden**.
///
/// This table exists because hiding a `GtkMenuBar` disables every accelerator
/// hanging off it. `gtk_menu_item_can_activate_accel` chains up the widget
/// ancestry and refuses when any ancestor is not visible, so a hidden bar takes
/// Ctrl+O, Ctrl+R, Ctrl+P, Ctrl+F, F3 and Ctrl+Q down with it — the accelerator
/// group stays attached to the window, but nothing on it will activate.
/// Confirmed by measurement, not by reading: with the bar hidden a synthesised
/// Ctrl+O produced no menu event, and the identical keystroke after F10 did.
///
/// So on Linux the keyboard has to be handled twice over, and the caller picks
/// which half is live by menu-bar visibility — never both, or every shortcut
/// would fire twice the moment the bar came back.
///
/// Pure, and takes the modifier state as a parameter, so the whole table is
/// unit-testable without a window.
#[cfg(target_os = "linux")]
fn linux_shortcut_for(key: &Key<'_>, modifiers: ModifiersState) -> Option<Shortcut> {
    // Exact comparisons, not `contains`: Ctrl+Shift+O is not Ctrl+O, and a
    // shortcut that fires on any superset would steal keys from the page.
    let ctrl = modifiers == ModifiersState::CONTROL;
    let ctrl_shift = modifiers == ModifiersState::CONTROL | ModifiersState::SHIFT;
    let alt = modifiers == ModifiersState::ALT;
    let shift = modifiers == ModifiersState::SHIFT;
    let bare = modifiers.is_empty();

    match key {
        // `logical_key` is the keyval, which Ctrl does not alter, so this is
        // still `Character("o")` with Ctrl held. Lowercased anyway: Caps Lock
        // reaches the keyval, and Ctrl+O should not depend on it.
        Key::Character(c) if ctrl => match c.to_ascii_lowercase().as_str() {
            "o" => Some(Shortcut::OpenFolder),
            "r" => Some(Shortcut::Reload),
            "f" => Some(Shortcut::FindOpen),
            // Close Window and Quit are the same thing in a one-window app, and
            // both are `PredefinedMenuItem`s, so both are lost with the bar.
            "w" | "q" => Some(Shortcut::Quit),
            _ => None,
        },
        // Shift is part of the chord *and* of the keyval, so this arrives as
        // `Character("P")`; the lowercasing below is what makes the two spellings
        // one case. Plain Ctrl+P is left to the page, where it means "previous".
        Key::Character(c) if ctrl_shift && c.eq_ignore_ascii_case("p") => Some(Shortcut::Print),
        Key::F3 if bare => Some(Shortcut::FindNext),
        Key::F3 if shift => Some(Shortcut::FindPrev),
        Key::ArrowLeft if alt => Some(Shortcut::Back),
        Key::ArrowRight if alt => Some(Shortcut::Forward),
        _ => None,
    }
}

/// Stop GTK from claiming F10 for menu-bar traversal.
///
/// `gtk-menu-bar-accel` defaults to `F10`, and a `GtkMenuBar` that is *visible*
/// consumes that key to move focus into itself. That leaves the toggle working
/// in one direction only — F10 reveals the bar, and the next F10 opens the File
/// menu instead of putting it away. Clearing the setting hands the key back, so
/// the same press means the same thing in both states.
///
/// What is given up is GTK's keyboard route *into* a visible menu bar. The bar
/// is still reachable by pointer, every item still has its own accelerator, and
/// on this application the setting is otherwise unused, so the trade is one
/// consistent toggle against a traversal shortcut nothing else here depends on.
///
/// The property is deprecated in GTK 3.10 and unwrapped by gtk-rs, hence the
/// string form; `find_property` first because `set_property` panics on a name
/// the object does not have, and a future GTK that finally drops it should
/// change nothing here.
#[cfg(target_os = "linux")]
fn release_gtk_menu_bar_accel() {
    use gtk::prelude::*;

    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    if settings.find_property("gtk-menu-bar-accel").is_some() {
        settings.set_property("gtk-menu-bar-accel", None::<String>);
    }
}

#[cfg(target_os = "linux")]
fn set_gtk_menu_bar_visible(menu_bar: &Menu, window: &tao::window::Window, visible: bool) {
    use gtk::prelude::WidgetExt;
    use tao::platform::unix::WindowExtUnix;

    let gtk_window = window.gtk_window();

    // `gtk_menubar_for_gtk_window` consumes the `Menu`; `Menu` is a handle
    // around a shared inner, so the clone is a refcount bump, not a rebuild.
    if let Some(bar) = menu_bar.clone().gtk_menubar_for_gtk_window(gtk_window) {
        bar.set_no_show_all(!visible);
    }

    let result = if visible {
        menu_bar.show_for_gtk_window(gtk_window)
    } else {
        menu_bar.hide_for_gtk_window(gtk_window)
    };
    if let Err(e) = result {
        tracing::warn!("Failed to set menu bar visibility: {e}");
    }
}

/// Launch the browser window with full context for server management
pub fn launch_browser(ctx: BrowserContext) -> Result<(), BrowserError> {
    // The only place in the codebase that arms `open_external`. Until this runs,
    // handing a URL to the operating system fails closed with
    // `ExternalOpenError::GuiOnly`, so a server-mode process — which links this
    // same code, since the `gui` feature is on by default — cannot be talked
    // into starting an application on its host no matter who calls the launcher.
    //
    // Set here rather than next to the handlers so it is unambiguously before
    // the WebView exists: every request to launch something originates in that
    // WebView, so nothing can ask before the latch is set.
    mark_gui_active();

    // Create event loop with user events for menu handling
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    // Set up menu event handler
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::MenuEvent(event));
    }));

    // Build the menu bar
    let MenuHandles {
        menu_bar,
        open_item,
        reload_item,
        print_item,
        history_items,
        find_items,
        window_menu: _window_menu,
    } = build_menu_bar();

    // Initialize menu for macOS (global app menu)
    #[cfg(target_os = "macos")]
    menu_bar.init_for_nsapp();

    // Resolved before the window exists so both the initial state and the F10
    // handler read one decision. `ctx.config` is the merged config, so an
    // `MBR_GUI_MENU_BAR` env var and `.mbr/config.toml` are already folded in.
    let menu_bar_setting = ctx.config.gui_menu_bar;
    let mut menu_bar_visible = menu_bar_starts_visible(menu_bar_setting, MENU_BAR_AUTO_VISIBLE);

    let icon = load_icon()?;
    let window = WindowBuilder::new()
        .with_title("mbr")
        .with_window_icon(Some(icon))
        .build(&event_loop)
        .map_err(BrowserError::WindowCreationFailed)?;

    // Only the Linux path consults these: macOS hangs the menu off the
    // application, not the window, and Windows has no equivalent of
    // `hide_for_hwnd` wired up here.
    #[cfg(not(target_os = "linux"))]
    let _ = (menu_bar_setting, &mut menu_bar_visible);

    // Initialize menu for Windows (per-window menu bar)
    #[cfg(target_os = "windows")]
    unsafe {
        use tao::platform::windows::WindowExtWindows;
        if let Err(e) = menu_bar.init_for_hwnd(window.hwnd()) {
            tracing::warn!("Failed to attach menu bar to window: {e}");
        }
    }

    // Initialize menu for Linux (GTK-based).
    //
    // The bar is packed into `default_vbox()` — the `GtkBox` tao puts inside the
    // window — and so is the WebView further down. That is not a preference:
    // a `GtkApplicationWindow` is a `GtkBin` and holds exactly one child, which
    // is already that box. Adding either widget to the window itself makes GTK
    // refuse the second one with
    //
    //   Gtk-WARNING: Attempting to add a widget with type WebKitWebView to a
    //   GtkApplicationWindow, but as a GtkBin subclass [it] can only contain one
    //   widget at a time; it already contains a widget of type GtkBox
    //
    // and the window then shows the menu bar with no page under it. `muda`
    // `reorder_child`s the bar to position 0, so the box orders itself.
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowExtUnix;
        if let Err(e) = menu_bar.init_for_gtk_window(window.gtk_window(), window.default_vbox()) {
            tracing::warn!("Failed to attach menu bar to window: {e}");
        }
        // `init_for_gtk_window` always `show()`s the bar, so a hidden start is a
        // hide immediately after. `set_no_show_all` is the part that makes it
        // stick: GTK's `show_all()` walks the whole widget tree, and anything
        // that calls it on the window later — tao, or a theme change — would
        // otherwise bring the bar back.
        release_gtk_menu_bar_accel();
        set_gtk_menu_bar_visible(&menu_bar, &window, menu_bar_visible);
    }

    // Shared because the policy has to follow the server: `Open Folder…`
    // restarts it, usually on a different port, and a stale origin would send
    // every internal link to the system browser.
    let site_origin = Arc::new(RwLock::new(SiteOrigin::new(&ctx.url)));
    let new_window_origin = Arc::clone(&site_origin);
    let ipc_origin = Arc::clone(&site_origin);

    let builder = WebViewBuilder::new()
        .with_devtools(true)
        .with_url(&ctx.url)
        // Without a handler wry allows every navigation, so an application
        // scheme silently did nothing: WKWebView, unlike UIKit, does not fall
        // back to NSWorkspace for a scheme it cannot render.
        //
        // `decide_without_frame_info` is the weaker of the two policies in
        // `external_open`, and its doc comment explains why at length. The short
        // version: wry hands this closure a URL and nothing else, and WebKit
        // calls it for `<iframe>` loads as well as document navigations, so it
        // deliberately lets *all* http(s) through — cancelling cross-origin
        // http(s) here would blank YouTube embeds. Clicked cross-origin links
        // are caught by the IPC handler below instead.
        .with_navigation_handler(|url| {
            apply_decision(
                decide_without_frame_info(&url),
                &url,
                open_with_system_handler,
            )
        })
        // The full origin-aware policy *is* safe here: this handler is only
        // consulted for window.open()/target="_blank", never for a frame.
        // Same-origin popups keep their linked webview so the Reveal.js
        // speaker-notes view stays in sync with its opener, while an external
        // one would otherwise open a second mbr-chrome window around somebody
        // else's site.
        //
        // The origin is copied out rather than read through a held guard: the
        // hand-off calls into AppKit/GTK, and nothing that can spin a run loop
        // should run while a lock this event loop also writes is held. One small
        // allocation per popup, which is a user action.
        .with_new_window_req_handler(move |url, _features| {
            let origin = new_window_origin.read().clone();
            if apply_decision(origin.decide(&url), &url, open_with_system_handler) {
                NewWindowResponse::Allow
            } else {
                NewWindowResponse::Deny
            }
        })
        // The other half of the external-link fix. `mbr-link-enhancement.ts`
        // runs only in GUI mode and only in the main frame, so it can tell a
        // clicked link from an embed — which the navigation handler cannot. It
        // cancels the click and posts the resolved URL here.
        //
        // The payload is page-controlled: anything that can run script in the
        // webview can post to this channel, including raw HTML in somebody
        // else's markdown. `parse_ipc_open_request` therefore re-runs the full
        // policy instead of trusting it.
        .with_ipc_handler(move |request| {
            let origin = ipc_origin.read().clone();
            match parse_ipc_open_request(&origin, request.body()) {
                Some(url) => open_with_system_handler(url),
                None => tracing::debug!("Ignoring unrecognized IPC message from the page"),
            }
        });

    #[cfg(not(target_os = "linux"))]
    let webview = builder
        .build(&window)
        .map_err(BrowserError::WebViewCreationFailed)?;
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        // Into the vbox, next to the menu bar — see the comment on
        // `init_for_gtk_window` above for why the window itself will not take it.
        // `build_gtk` recognises a `gtk::Box` and packs with
        // `pack_start(webview, true, true, 0)`, so the page expands and the menu
        // bar (packed `false, false`) keeps its natural height.
        //
        // `default_vbox()` is `None` only when the window was built with
        // `with_default_vbox(false)`, which this code never does; falling back to
        // the window keeps the match exhaustive without a panic.
        match window.default_vbox() {
            Some(vbox) => builder.build_gtk(vbox),
            None => builder.build_gtk(window.gtk_window()),
        }
        .map_err(BrowserError::WebViewCreationFailed)?
    };

    // Store menu item IDs for event matching
    let shortcut_ids = ShortcutIds {
        open: open_item.id().clone(),
        reload: reload_item.id().clone(),
        print: print_item.id().clone(),
        back: history_items.back.id().clone(),
        forward: history_items.forward.id().clone(),
        find: find_items.open.id().clone(),
        find_next: find_items.next.id().clone(),
        find_prev: find_items.prev.id().clone(),
    };

    // Track modifier state for Alt+arrow handling
    let mut modifiers = ModifiersState::empty();

    // Mutable state for server management
    let mut server_handle = ctx.server_handle;
    let mut current_url = ctx.url;
    let tokio_runtime = ctx.tokio_runtime;

    // Create proxy for folder picker
    let event_proxy = event_loop.create_proxy();

    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::UserEvent(UserEvent::MenuEvent(menu_event)) => {
                // `PredefinedMenuItem` events (quit, close, clipboard, about)
                // are activated by the platform and never arrive here.
                if let Some(shortcut) = shortcut_for_menu_id(&menu_event.id, &shortcut_ids) {
                    perform_shortcut(shortcut, &webview, &current_url, &event_proxy, control_flow);
                }
            }
            Event::UserEvent(UserEvent::FolderSelected(new_path)) => {
                tracing::info!("Switching to new folder: {}", new_path.display());

                // Hand the work to the runtime and return to the event loop
                // immediately; the result arrives as `ServerReady`/`ServerFailed`.
                // Waiting here is what used to abort the process — see
                // `start_server_for`.
                //
                // The current server is deliberately left running. It is aborted
                // only once a replacement is listening, so a folder that fails to
                // open leaves the window exactly as it was — which is what the
                // error message below has always claimed.
                let proxy = event_proxy.clone();
                tokio_runtime.spawn(async move {
                    let event = match start_server_for(new_path.clone()).await {
                        Ok((handle, url)) => UserEvent::ServerReady { handle, url },
                        Err(e) => {
                            tracing::error!("Failed to open folder: {e}");
                            UserEvent::ServerFailed { path: new_path }
                        }
                    };
                    // Fails only once the event loop is gone, which means the
                    // window is closing and nothing wants this result.
                    let _ = proxy.send_event(event);
                });
            }
            Event::UserEvent(UserEvent::ServerReady { handle, url }) => {
                server_handle.abort();
                server_handle = handle;
                current_url = url;
                // The new server rarely lands on the old port, and the
                // navigation handler compares against this.
                *site_origin.write() = SiteOrigin::new(&current_url);
                tracing::info!("Server restarted at {}", current_url);
                let _ = webview.load_url(&current_url);
            }
            Event::UserEvent(UserEvent::ServerFailed { path }) => {
                // Off-thread because a modal dialog spins its own run loop, and
                // this one is the event loop it would spin inside.
                std::thread::spawn(move || {
                    rfd::MessageDialog::new()
                        .set_level(rfd::MessageLevel::Error)
                        .set_title("Failed to Open Folder")
                        .set_description(format!(
                            "Could not open folder: {}\n\nThe current folder will remain active.",
                            path.display()
                        ))
                        .set_buttons(rfd::MessageButtons::Ok)
                        .show();
                });
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                tracing::debug!("The close button was pressed; stopping");
                *control_flow = ControlFlow::Exit
            }
            Event::WindowEvent {
                event: WindowEvent::ModifiersChanged(new_modifiers),
                ..
            } => {
                modifiers = new_modifiers;
            }
            // F10 reveals or dismisses the Linux menu bar. F10 is the GTK
            // convention for exactly this (Firefox, Nautilus, and every app that
            // ships `gtk-menu-bar-accel` unchanged), so it needs no teaching.
            //
            // Placed before the Alt+arrow arm because match arms are tried in
            // order and that arm's guard would otherwise be the only keyboard
            // branch consulted. Bare F10 only: a modified F10 belongs to the page.
            //
            // Acts on **release**, not press. WebKitGTK hands a key press to the
            // web process and, when the page leaves it unhandled, re-dispatches
            // the same press to the toplevel so window accelerators still work —
            // so a `Pressed` arm here fires *twice* for exactly the keys nothing
            // in the page claims, and a toggle would land back where it started.
            // Measured on this stack: a page-ignored key arrives as two
            // `Pressed` and one `Released`; a page-handled one as one of each.
            // The release is the event that arrives exactly once either way.
            //
            // Matched on `logical_key`, not `physical_key`. What we want is the
            // key's *meaning*, which is what a user reads off their keycap and
            // what survives a remapped layout; `physical_key` is tao's mapping of
            // the raw hardware keycode, and any input source that synthesises
            // events on a keymap of its own (a virtual keyboard, a remote-desktop
            // agent, `wtype`) fills it with whatever slot it happened to use —
            // observed reporting `Escape` for both F10 and `f`, while
            // `logical_key` stayed correct for both.
            #[cfg(target_os = "linux")]
            Event::WindowEvent {
                event:
                    WindowEvent::KeyboardInput {
                        event: key_event, ..
                    },
                ..
            } if key_event.state == ElementState::Released
                && key_event.logical_key == Key::F10
                && modifiers.is_empty()
                && menu_bar_toggle_allowed(menu_bar_setting) =>
            {
                menu_bar_visible = !menu_bar_visible;
                tracing::debug!("Menu bar toggled to visible={menu_bar_visible} via F10");
                set_gtk_menu_bar_visible(&menu_bar, &window, menu_bar_visible);
            }
            // Everything else the menu bar offers, while the menu bar is not
            // there to offer it.
            //
            // Gated on `!menu_bar_visible` because both routes are live on
            // Linux: with the bar on screen GTK's accelerator group activates
            // the menu item, and handling the key here as well would perform
            // every action twice. See `linux_shortcut_for` for why the bar's
            // visibility decides this at all.
            #[cfg(target_os = "linux")]
            Event::WindowEvent {
                event:
                    WindowEvent::KeyboardInput {
                        event: key_event, ..
                    },
                ..
            } if key_event.state == ElementState::Released
                && !menu_bar_visible
                && linux_shortcut_for(&key_event.logical_key, modifiers).is_some() =>
            {
                // The guard proved this is `Some`; the table is consulted twice
                // rather than restructured because a match guard cannot bind.
                if let Some(shortcut) = linux_shortcut_for(&key_event.logical_key, modifiers) {
                    perform_shortcut(shortcut, &webview, &current_url, &event_proxy, control_flow);
                }
            }
            // History navigation off Linux, where the menu bar is always present
            // and only the Alt+arrow pair has no accelerator of its own.
            #[cfg(not(target_os = "linux"))]
            Event::WindowEvent {
                event:
                    WindowEvent::KeyboardInput {
                        event: key_event, ..
                    },
                ..
            } if key_event.state == ElementState::Released && modifiers.alt_key() => {
                match key_event.physical_key {
                    KeyCode::ArrowLeft => {
                        perform_shortcut(
                            Shortcut::Back,
                            &webview,
                            &current_url,
                            &event_proxy,
                            control_flow,
                        );
                    }
                    KeyCode::ArrowRight => {
                        perform_shortcut(
                            Shortcut::Forward,
                            &webview,
                            &current_url,
                            &event_proxy,
                            control_flow,
                        );
                    }
                    _ => {}
                }
            }
            _ => (),
        }
    });
}

fn load_icon() -> Result<Icon, BrowserError> {
    let (icon_rgba, icon_width, icon_height) = {
        let image_bytes = include_bytes!("../mbr-icon.png");
        let image = image::load_from_memory(image_bytes)
            .map_err(|e| BrowserError::IconLoadFailed(e.to_string()))?
            .into_rgba8();
        let (width, height) = image.dimensions();
        let rgba = image.into_raw();
        (rgba, width, height)
    };
    Icon::from_rgba(icon_rgba, icon_width, icon_height).map_err(BrowserError::IconCreationFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    // `auto` is passed in rather than read from `MENU_BAR_AUTO_VISIBLE`, so both
    // platform conventions are covered no matter which host runs the suite.
    #[test]
    fn auto_follows_the_platform_convention() {
        assert!(!menu_bar_starts_visible(MenuBarVisibility::Auto, false));
        assert!(menu_bar_starts_visible(MenuBarVisibility::Auto, true));
    }

    #[test]
    fn always_and_never_ignore_the_platform() {
        for auto in [false, true] {
            assert!(menu_bar_starts_visible(MenuBarVisibility::Always, auto));
            assert!(!menu_bar_starts_visible(MenuBarVisibility::Never, auto));
        }
    }

    #[test]
    fn linux_is_the_only_platform_that_hides_by_default() {
        assert_eq!(MENU_BAR_AUTO_VISIBLE, !cfg!(target_os = "linux"));
    }

    // `never` is a decision, not a starting position: F10 must not undo it.
    #[test]
    fn only_never_refuses_the_f10_toggle() {
        assert!(menu_bar_toggle_allowed(MenuBarVisibility::Auto));
        assert!(menu_bar_toggle_allowed(MenuBarVisibility::Always));
        assert!(!menu_bar_toggle_allowed(MenuBarVisibility::Never));
    }

    // Regression test for the crash that made `Open Folder…` unusable on every
    // platform: the old synchronous `reinit_server` called `Handle::block_on`
    // from the event-loop callback, which `main` runs inside `#[tokio::main]`'s
    // runtime, and tokio aborts the process for that.
    //
    // `#[tokio::test]` reproduces the condition exactly — this body runs *on* a
    // runtime — so the old code panicked here and the new code cannot.
    #[tokio::test]
    async fn start_server_for_does_not_block_its_own_runtime() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("index.md"), "# hi\n").expect("write");

        let (handle, url) = super::start_server_for(dir.path().to_path_buf())
            .await
            .expect("server should come up for a plain markdown folder");

        assert!(
            url.starts_with("http://"),
            "expected an http url, got {url}"
        );
        // A real port, not the placeholder: the URL is built from the port the
        // server reported through the readiness channel.
        let port: u16 = url
            .rsplit(':')
            .next()
            .and_then(|p| p.trim_end_matches('/').parse().ok())
            .unwrap_or_else(|| panic!("no port in {url}"));
        assert!(port > 0);

        handle.abort();
    }

    // A path that cannot be canonicalized must come back as an error the caller
    // can turn into a dialog, not a panic and not a hang on the readiness
    // channel.
    #[tokio::test]
    async fn start_server_for_reports_a_missing_folder() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("no-such-folder");

        assert!(super::start_server_for(missing).await.is_err());
    }

    // The keyboard table only exists on Linux, and only it can reach these
    // actions while the bar is hidden -- so every entry is worth pinning.
    #[cfg(target_os = "linux")]
    mod shortcuts {
        use super::super::*;

        fn look(key: Key<'_>, modifiers: ModifiersState) -> Option<Shortcut> {
            linux_shortcut_for(&key, modifiers)
        }

        const CTRL: ModifiersState = ModifiersState::CONTROL;
        const ALT: ModifiersState = ModifiersState::ALT;
        const SHIFT: ModifiersState = ModifiersState::SHIFT;
        const NONE: ModifiersState = ModifiersState::empty();

        #[test]
        fn every_menu_shortcut_has_a_keyboard_route() {
            assert_eq!(look(Key::Character("o"), CTRL), Some(Shortcut::OpenFolder));
            assert_eq!(look(Key::Character("r"), CTRL), Some(Shortcut::Reload));
            assert_eq!(
                look(Key::Character("P"), CTRL | SHIFT),
                Some(Shortcut::Print)
            );
            assert_eq!(look(Key::Character("f"), CTRL), Some(Shortcut::FindOpen));
            assert_eq!(look(Key::F3, NONE), Some(Shortcut::FindNext));
            assert_eq!(look(Key::F3, SHIFT), Some(Shortcut::FindPrev));
            assert_eq!(look(Key::ArrowLeft, ALT), Some(Shortcut::Back));
            assert_eq!(look(Key::ArrowRight, ALT), Some(Shortcut::Forward));
        }

        // Both are `PredefinedMenuItem`s, so both stop working with the bar.
        #[test]
        fn close_and_quit_both_quit() {
            assert_eq!(look(Key::Character("w"), CTRL), Some(Shortcut::Quit));
            assert_eq!(look(Key::Character("q"), CTRL), Some(Shortcut::Quit));
        }

        // Caps Lock reaches the keyval, and Ctrl+O should not depend on it.
        #[test]
        fn character_matching_ignores_case() {
            assert_eq!(look(Key::Character("O"), CTRL), Some(Shortcut::OpenFolder));
        }

        // A superset of the modifiers is a *different* chord, and claiming it
        // would steal a key the page may want.
        // Ctrl+P is "previous item" in mbr's lists; the native menu must not
        // take it. Shift-less Ctrl+P has to reach the page.
        #[test]
        fn plain_ctrl_p_is_left_to_the_page() {
            assert_eq!(look(Key::Character("p"), CTRL), None);
            // And the print chord works whichever case the keyval carries.
            assert_eq!(
                look(Key::Character("p"), CTRL | SHIFT),
                Some(Shortcut::Print)
            );
        }

        #[test]
        fn extra_modifiers_do_not_match() {
            assert_eq!(look(Key::Character("o"), CTRL | SHIFT), None);
            assert_eq!(look(Key::ArrowLeft, ALT | CTRL), None);
            assert_eq!(look(Key::F3, ALT), None);
        }

        #[test]
        fn bare_letters_are_left_to_the_page() {
            // `o` alone is not Open; the document may bind it.
            assert_eq!(look(Key::Character("o"), NONE), None);
            assert_eq!(look(Key::Character("z"), CTRL), None);
            assert_eq!(look(Key::ArrowLeft, NONE), None);
        }

        // F10 is deliberately absent: it toggles the bar and must keep working
        // in *both* states, so it has its own ungated arm.
        #[test]
        fn f10_is_not_in_the_gated_table() {
            assert_eq!(look(Key::F10, NONE), None);
        }
    }

    // A config that pins the bar off should still start hidden after a toggle
    // check, and one that pins it on should start shown regardless of platform.
    #[test]
    fn starting_state_and_toggle_permission_agree_for_never() {
        let setting = MenuBarVisibility::Never;
        assert!(!menu_bar_starts_visible(setting, true));
        assert!(!menu_bar_toggle_allowed(setting));
    }
}

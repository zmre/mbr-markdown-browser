extern crate image;
use crate::Config;
use crate::errors::BrowserError;
use crate::server::{Server, ServerConfig};
use muda::{
    AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
use std::path::PathBuf;
use tao::{
    event::{ElementState, Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    keyboard::{KeyCode, ModifiersState},
    window::{Icon, WindowBuilder},
};
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

    let open_item = MenuItem::with_id(
        "open",
        "&Open...",
        true,
        Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyO)),
    );

    let reload_item = MenuItem::with_id(
        "reload",
        "&Reload",
        true,
        Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyR)),
    );

    // Print uses Cmd+P on macOS, Ctrl+P elsewhere
    #[cfg(target_os = "macos")]
    let print_modifier = Modifiers::SUPER;
    #[cfg(not(target_os = "macos"))]
    let print_modifier = Modifiers::CONTROL;

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
    let devtools_item = MenuItem::with_id(
        "devtools",
        "Toggle Developer Tools",
        true,
        Some(Accelerator::new(
            Some(Modifiers::SUPER | Modifiers::ALT),
            Code::KeyI,
        )),
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
    let back_item = MenuItem::with_id(
        "back",
        "&Back",
        true,
        Some(Accelerator::new(Some(Modifiers::SUPER), Code::BracketLeft)),
    );
    let forward_item = MenuItem::with_id(
        "forward",
        "&Forward",
        true,
        Some(Accelerator::new(Some(Modifiers::SUPER), Code::BracketRight)),
    );
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

/// Reinitialize the server with a new path
fn reinit_server(
    path: &std::path::Path,
    runtime: &tokio::runtime::Handle,
) -> Result<(JoinHandle<()>, String, Config), BrowserError> {
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
    let handle = runtime.spawn(async move {
        let server_config = ServerConfig::from(&config_copy).with_gui_mode(true);
        let server = Server::init(server_config);
        match server {
            Ok(mut s) => {
                if let Err(e) = s.start_with_port_retry(Some(ready_tx), 10).await {
                    tracing::error!("Server error: {e}");
                }
            }
            Err(e) => {
                tracing::error!("Server init failed: {e}");
                drop(ready_tx);
            }
        }
    });

    // Block briefly to get the port
    let port = runtime
        .block_on(ready_rx)
        .map_err(|_| BrowserError::ServerStartFailed)?;

    let url = format!("http://{}:{}/", config.host, port);
    Ok((handle, url, config))
}

/// Launch the browser window with full context for server management
pub fn launch_browser(ctx: BrowserContext) -> Result<(), BrowserError> {
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

    let icon = load_icon()?;
    let window = WindowBuilder::new()
        .with_title("mbr")
        .with_window_icon(Some(icon))
        .build(&event_loop)
        .map_err(BrowserError::WindowCreationFailed)?;

    // Initialize menu for Windows (per-window menu bar)
    #[cfg(target_os = "windows")]
    unsafe {
        use tao::platform::windows::WindowExtWindows;
        if let Err(e) = menu_bar.init_for_hwnd(window.hwnd()) {
            tracing::warn!("Failed to attach menu bar to window: {e}");
        }
    }

    // Initialize menu for Linux (GTK-based)
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowExtUnix;
        let _ = menu_bar.init_for_gtk_window(window.gtk_window(), window.default_vbox());
    }

    let builder = WebViewBuilder::new()
        .with_devtools(true)
        .with_url(&ctx.url)
        // Allow JS window.open() (e.g. Reveal.js speaker-notes view) to spawn a
        // linked webview so the popup stays in sync with the opener.
        .with_new_window_req_handler(|_url, _features| NewWindowResponse::Allow);

    #[cfg(not(target_os = "linux"))]
    let webview = builder
        .build(&window)
        .map_err(BrowserError::WebViewCreationFailed)?;
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        builder
            .build_gtk(window.gtk_window())
            .map_err(BrowserError::WebViewCreationFailed)?
    };

    // Store menu item IDs for event matching
    let open_id = open_item.id().clone();
    let reload_id = reload_item.id().clone();
    let print_id = print_item.id().clone();
    let back_id = history_items.back.id().clone();
    let forward_id = history_items.forward.id().clone();
    let find_id = find_items.open.id().clone();
    let find_next_id = find_items.next.id().clone();
    let find_prev_id = find_items.prev.id().clone();

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
                // Handle custom menu items
                if menu_event.id == open_id {
                    tracing::debug!("Open folder requested via menu");
                    spawn_folder_picker(event_proxy.clone());
                } else if menu_event.id == reload_id {
                    tracing::debug!("Reload requested via menu");
                    let _ = webview.load_url(&current_url);
                } else if menu_event.id == print_id {
                    tracing::debug!("Print requested via menu");
                    if let Err(e) = webview.print() {
                        tracing::error!("Print failed: {e}");
                    }
                } else if menu_event.id == back_id {
                    tracing::debug!("History back via menu");
                    let _ = webview.evaluate_script("history.back()");
                } else if menu_event.id == forward_id {
                    tracing::debug!("History forward via menu");
                    let _ = webview.evaluate_script("history.forward()");
                } else if menu_event.id == find_id {
                    tracing::debug!("Find requested via menu");
                    let _ = webview.evaluate_script(FIND_OPEN_SCRIPT);
                } else if menu_event.id == find_next_id {
                    tracing::debug!("Find next via menu");
                    let _ = webview.evaluate_script(FIND_NEXT_SCRIPT);
                } else if menu_event.id == find_prev_id {
                    tracing::debug!("Find previous via menu");
                    let _ = webview.evaluate_script(FIND_PREV_SCRIPT);
                }
                // Note: PredefinedMenuItem events (quit, close, etc.) are handled automatically
            }
            Event::UserEvent(UserEvent::FolderSelected(new_path)) => {
                tracing::info!("Switching to new folder: {}", new_path.display());

                // Abort current server
                server_handle.abort();

                // Reinitialize with new path
                match reinit_server(&new_path, &tokio_runtime) {
                    Ok((new_handle, new_url, _new_config)) => {
                        server_handle = new_handle;
                        current_url = new_url.clone();
                        tracing::info!("Server restarted at {}", current_url);
                        let _ = webview.load_url(&current_url);
                    }
                    Err(e) => {
                        tracing::error!("Failed to open folder: {e}");
                        // Show error dialog
                        std::thread::spawn(move || {
                            rfd::MessageDialog::new()
                                .set_level(rfd::MessageLevel::Error)
                                .set_title("Failed to Open Folder")
                                .set_description(format!(
                                    "Could not open folder: {}\n\nThe current folder will remain active.",
                                    new_path.display()
                                ))
                                .set_buttons(rfd::MessageButtons::Ok)
                                .show();
                        });
                    }
                }
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
            Event::WindowEvent {
                event:
                    WindowEvent::KeyboardInput {
                        event: key_event, ..
                    },
                ..
            } if key_event.state == ElementState::Pressed && modifiers.alt_key() => {
                // Handle Alt+Left/Right for history navigation
                match key_event.physical_key {
                    KeyCode::ArrowLeft => {
                        tracing::debug!("History back via Alt+Left");
                        let _ = webview.evaluate_script("history.back()");
                    }
                    KeyCode::ArrowRight => {
                        tracing::debug!("History forward via Alt+Right");
                        let _ = webview.evaluate_script("history.forward()");
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

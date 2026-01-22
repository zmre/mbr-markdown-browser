extern crate image;
use crate::Config;
use crate::errors::BrowserError;
use crate::server::Server;
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
use wry::WebViewBuilder;
#[cfg(target_os = "linux")]
use wry::WebViewBuilderExtUnix;

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

/// Menu items for history navigation
struct HistoryMenuItems {
    back: MenuItem,
    forward: MenuItem,
}

/// Build the application menu bar with standard menus
/// On macOS, creates proper app menu with About, Services, Hide, Quit
/// On Windows/Linux, puts About in Help menu and Quit in File menu
fn build_menu_bar() -> (Menu, MenuItem, MenuItem, HistoryMenuItems, Submenu) {
    let menu_bar = Menu::new();

    // macOS: First menu is the app menu (named after the app)
    // Contains About, Services, Hide, Hide Others, Show All, Quit
    #[cfg(target_os = "macos")]
    let app_menu = {
        let app_menu = Submenu::new("mbr", true);
        app_menu
            .append_items(&[
                &PredefinedMenuItem::about(None, Some(about_metadata())),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::services(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::quit(None),
            ])
            .expect("Failed to append app menu items");
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

    #[cfg(target_os = "macos")]
    file_menu
        .append_items(&[
            &open_item,
            &PredefinedMenuItem::separator(),
            &reload_item,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::close_window(Some("Close Window")),
        ])
        .expect("Failed to append file menu items");

    #[cfg(not(target_os = "macos"))]
    file_menu
        .append_items(&[
            &open_item,
            &PredefinedMenuItem::separator(),
            &reload_item,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::close_window(Some("Close Window")),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ])
        .expect("Failed to append file menu items");

    // Edit menu with standard clipboard operations
    let edit_menu = Submenu::new("&Edit", true);
    edit_menu
        .append_items(&[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
        ])
        .expect("Failed to append edit menu items");

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
    view_menu
        .append_items(&[
            &PredefinedMenuItem::fullscreen(None),
            &PredefinedMenuItem::separator(),
            &devtools_item,
        ])
        .expect("Failed to append view menu items");

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
    history_menu
        .append_items(&[&back_item, &forward_item])
        .expect("Failed to append history menu items");

    let history_items = HistoryMenuItems {
        back: back_item,
        forward: forward_item,
    };

    // Window menu
    let window_menu = Submenu::new("&Window", true);
    window_menu
        .append_items(&[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::bring_all_to_front(None),
        ])
        .expect("Failed to append window menu items");

    // Help menu - only needed on non-macOS for About
    #[cfg(not(target_os = "macos"))]
    let help_menu = {
        let help_menu = Submenu::new("&Help", true);
        help_menu
            .append_items(&[&PredefinedMenuItem::about(None, Some(about_metadata()))])
            .expect("Failed to append help menu items");
        help_menu
    };

    // Build menu bar - order matters, especially on macOS
    #[cfg(target_os = "macos")]
    menu_bar
        .append_items(&[
            &app_menu,
            &file_menu,
            &edit_menu,
            &view_menu,
            &history_menu,
            &window_menu,
        ])
        .expect("Failed to append menus to menu bar");

    #[cfg(not(target_os = "macos"))]
    menu_bar
        .append_items(&[
            &file_menu,
            &edit_menu,
            &view_menu,
            &history_menu,
            &window_menu,
            &help_menu,
        ])
        .expect("Failed to append menus to menu bar");

    // On macOS, set the Window menu as the windows menu for proper window management
    #[cfg(target_os = "macos")]
    window_menu.set_as_windows_menu_for_nsapp();

    (menu_bar, open_item, reload_item, history_items, window_menu)
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
        let server = Server::init(
            config_copy.ip.0,
            config_copy.port,
            config_copy.root_dir.clone(),
            &config_copy.static_folder,
            &config_copy.markdown_extensions,
            &config_copy.ignore_dirs,
            &config_copy.ignore_globs,
            &config_copy.watcher_ignore_dirs,
            &config_copy.index_file,
            config_copy.oembed_timeout_ms,
            config_copy.oembed_cache_size,
            config_copy.template_folder.clone(),
            config_copy.sort.clone(),
            true, // gui_mode: native window mode
            &config_copy.theme,
            None, // Logging already initialized
            config_copy.link_tracking,
            &config_copy.tag_sources,
            #[cfg(feature = "media-metadata")]
            config_copy.transcode,
        );
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

    let url = format!("http://{}:{}/", config.ip, port);
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
    let (menu_bar, open_item, reload_item, history_items, _window_menu) = build_menu_bar();

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
        menu_bar.init_for_hwnd(window.hwnd() as isize);
    }

    // Initialize menu for Linux (GTK-based)
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowExtUnix;
        let _ = menu_bar.init_for_gtk_window(window.gtk_window(), window.default_vbox());
    }

    let builder = WebViewBuilder::new().with_devtools(true).with_url(&ctx.url);

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
    let back_id = history_items.back.id().clone();
    let forward_id = history_items.forward.id().clone();

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
                } else if menu_event.id == back_id {
                    tracing::debug!("History back via menu");
                    let _ = webview.evaluate_script("history.back()");
                } else if menu_event.id == forward_id {
                    tracing::debug!("History forward via menu");
                    let _ = webview.evaluate_script("history.forward()");
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
            } => {
                // Handle Alt+Left/Right for history navigation
                if key_event.state == ElementState::Pressed && modifiers.alt_key() {
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
            }
            _ => (),
        }
    });
}

/// Legacy function for simple URL launch without server management
/// Kept for backwards compatibility but launch_browser is preferred
pub fn launch_url(url: &str) -> Result<(), BrowserError> {
    // Create event loop with user events for menu handling
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    // Set up menu event handler
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::MenuEvent(event));
    }));

    // Build the menu bar
    let (menu_bar, _open_item, reload_item, history_items, _window_menu) = build_menu_bar();

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
        menu_bar.init_for_hwnd(window.hwnd() as isize);
    }

    // Initialize menu for Linux (GTK-based)
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowExtUnix;
        let _ = menu_bar.init_for_gtk_window(window.gtk_window(), window.default_vbox());
    }

    let url_owned = url.to_string();
    let builder = WebViewBuilder::new()
        .with_devtools(true)
        .with_url(&url_owned);

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
    let reload_id = reload_item.id().clone();
    let back_id = history_items.back.id().clone();
    let forward_id = history_items.forward.id().clone();

    // Track modifier state for Alt+arrow handling
    let mut modifiers = ModifiersState::empty();

    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::UserEvent(UserEvent::MenuEvent(menu_event)) => {
                // Handle custom menu items
                if menu_event.id == reload_id {
                    tracing::debug!("Reload requested via menu");
                    let _ = webview.load_url(&url_owned);
                } else if menu_event.id == back_id {
                    tracing::debug!("History back via menu");
                    let _ = webview.evaluate_script("history.back()");
                } else if menu_event.id == forward_id {
                    tracing::debug!("History forward via menu");
                    let _ = webview.evaluate_script("history.forward()");
                }
                // Note: PredefinedMenuItem events (quit, close, etc.) are handled automatically
            }
            Event::UserEvent(UserEvent::FolderSelected(_)) => {
                // Not supported in legacy mode
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
            } => {
                // Handle Alt+Left/Right for history navigation
                if key_event.state == ElementState::Pressed && modifiers.alt_key() {
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

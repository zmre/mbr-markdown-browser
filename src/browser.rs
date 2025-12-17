extern crate image;
use crate::errors::BrowserError;
use tao::{
    event::{DeviceEvent, Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    platform::macos::WindowBuilderExtMacOS,
    window::{Icon, Window, WindowBuilder},
};
use wry::WebViewBuilder;

pub fn launch_url(url: &str) -> Result<(), BrowserError> {
    let event_loop = EventLoop::new();
    let icon = load_icon()?;
    let window = WindowBuilder::new()
        .with_title("mbr")
        // At present, this only does anything on Windows and Linux, so if you want to save load
        // time, you can put icon loading behind a function that returns `None` on other platforms.
        .with_window_icon(Some(icon))
        // .with_titlebar_transparent(true) // requires a feature that uses private apis
        .build(&event_loop)
        .map_err(BrowserError::WindowCreationFailed)?;

    // with_window_icon(self, window_icon: Option<Icon>) -> Self
    // window.set_title("New Window Title");
    // window_builder.with_decorations(false);

    let builder = WebViewBuilder::new().with_devtools(true).with_url(url);

    #[cfg(not(target_os = "linux"))]
    let webview = builder
        .build(&window)
        .map_err(BrowserError::WebViewCreationFailed)?;
    #[cfg(target_os = "linux")]
    let webview = builder
        .build_gtk(window.gtk_window())
        .map_err(BrowserError::WebViewCreationFailed)?;

    // webview.open_devtools();

    event_loop.run(|event, _target, control_flow| {
        // ControlFlow::Wait pauses the event loop if no events are available to process.
        // This is ideal for non-game applications that only update in response to user
        // input, and uses significantly less power/CPU time than ControlFlow::Poll.
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                println!("The close button was pressed; stopping");
                *control_flow = ControlFlow::Exit
            }
            Event::WindowEvent {
                event:
                    WindowEvent::KeyboardInput {
                        device_id,
                        event,
                        is_synthetic,
                        ..
                    },
                ..
            } => {
                println!("Window keyboard input: {:?}", &event);
            }
            Event::DeviceEvent {
                event: DeviceEvent::Key(key),
                ..
            } => {
                println!("Device keyboard input: {:?}", &key);
            }
            _ => (),
        }
    });
}

fn load_icon() -> Result<Icon, BrowserError> {
    let (icon_rgba, icon_width, icon_height) = {
        // alternatively, you can embed the icon in the binary through `include_bytes!` macro and use `image::load_from_memory`
        let image_bytes = include_bytes!("../mbr-icon.png");
        let image = image::load_from_memory(image_bytes)
            .map_err(|e| BrowserError::IconLoadFailed(e.to_string()))?
            .into_rgba8();
        // let image = image::open(path)
        //     .expect("Failed to open icon path")
        //     .into_rgba8();
        let (width, height) = image.dimensions();
        let rgba = image.into_raw();
        (rgba, width, height)
    };
    Icon::from_rgba(icon_rgba, icon_width, icon_height).map_err(BrowserError::IconCreationFailed)
}

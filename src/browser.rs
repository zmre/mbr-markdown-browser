// use winit::{
//     application::ApplicationHandler,
//     event::WindowEvent,
//     event_loop::{ActiveEventLoop, EventLoop},
//     window::{Window, WindowId},
// };
use tao::{
    event::{DeviceEvent, Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    platform::macos::WindowBuilderExtMacOS,
    window::{Window, WindowBuilder},
};
use wry::WebViewBuilder;

pub fn launch_url(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("mbr")
        .with_titlebar_transparent(true)
        .build(&event_loop)?;

    // with_window_icon(self, window_icon: Option<Icon>) -> Self
    // window.set_title("New Window Title");
    // window_builder.with_decorations(false);

    let builder = WebViewBuilder::new().with_devtools(true).with_url(url);

    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(&window)?;
    #[cfg(target_os = "linux")]
    let webview = builder.build_gtk(window.gtk_window())?;

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

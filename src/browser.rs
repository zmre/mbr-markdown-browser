use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};
use wry::WebViewBuilder;

pub enum BrowserContents {
    Url(String),
    Html(String),
}

impl Default for BrowserContents {
    fn default() -> Self {
        BrowserContents::Url("https://well.com/".to_string())
    }
}

#[derive(Default)]
pub struct Gui {
    window: Option<Window>,
    webview: Option<wry::WebView>,
    content: BrowserContents,
}

impl Gui {
    pub fn launch_url(url: &str) {
        let event_loop = EventLoop::new().unwrap();
        let mut app = Gui::default();
        app.content = BrowserContents::Url(url.to_string());
        event_loop.run_app(&mut app).unwrap();
    }
    pub fn launch_html(html: &str) {
        let event_loop = EventLoop::new().unwrap();
        let mut app = Gui::default();
        app.content = BrowserContents::Html(html.to_string());
        event_loop.run_app(&mut app).expect("error in run_app");
    }
}

impl ApplicationHandler for Gui {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = event_loop
            .create_window(Window::default_attributes())
            .unwrap();
        window.set_title("mbr");
        let webview = WebViewBuilder::new();
        let webview = match &self.content {
            BrowserContents::Url(url) => webview.with_url(url).build(&window).unwrap(),
            BrowserContents::Html(html) => webview.with_html(html).build(&window).unwrap(),
        };

        self.window = Some(window);
        self.webview = Some(webview);
    }

    // fn window_event(&mut self, _event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {}
    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        if event == WindowEvent::CloseRequested {
            println!("The close button was pressed; stopping");
            event_loop.exit();
        }
    }
}

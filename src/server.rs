use axum::{
    // extract::Request, handler::HandlerWithoutStateExt, http::StatusCode,
    extract,
    response::IntoResponse,
    routing::get,
    Router,
};
use std::net::SocketAddr;
// use tower::ServiceExt;
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::Config;

pub struct Server {
    pub router: Router,
    pub port: u16,
    pub ip: [u8; 4],
}

impl Server {
    pub fn init(ip: [u8; 4], port: u16) -> Self {
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                    format!("{}=debug,tower_http=debug", env!("CARGO_CRATE_NAME")).into()
                }),
            )
            .with(tracing_subscriber::fmt::layer())
            .init();

        let serve_dir =
            ServeDir::new("assets").not_found_service(ServeFile::new("assets/index.html"));

        let router = Router::new()
            .route("/", get(|| async { "Home" }))
            .route("/.mbr/{*path}", get(Self::handle_special))
            .route("/{*path}", get(Self::handle))
            .nest_service("/assets", serve_dir.clone())
            .fallback_service(serve_dir)
            .layer(TraceLayer::new_for_http());

        Server { router, ip, port }
    }

    pub async fn start(&self) {
        let addr = SocketAddr::from((self.ip, self.port));
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        tracing::debug!("listening on {}", listener.local_addr().unwrap());
        axum::serve(listener, self.router.clone()).await.unwrap();
    }

    async fn handle(extract::Path(path): extract::Path<String>) -> impl IntoResponse {
        tracing::debug!("got request: {}", &path);
        format!("Got {}", &path)
    }
    async fn handle_special(extract::Path(path): extract::Path<String>) -> impl IntoResponse {
        tracing::debug!("got special request: {}", &path);
        format!("Got {}", &path)
    }
}

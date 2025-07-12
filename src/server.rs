use axum::{
    body::Body,
    extract::{self, Path, Request, State},
    handler::HandlerWithoutStateExt,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use std::net::SocketAddr;

use crate::markdown;
use crate::templates;
use tower::ServiceExt;
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub struct Server {
    pub router: Router,
    pub port: u16,
    pub ip: [u8; 4],
}

#[derive(Clone)]
pub struct ServerState {
    pub base_dir: std::path::PathBuf,
    pub static_folder: String,
    pub markdown_extensions: Vec<String>,
}

impl Server {
    pub fn init<S: Into<String>, P: Into<std::path::PathBuf>>(
        ip: [u8; 4],
        port: u16,
        base_dir: P,
        static_folder: S,
        markdown_extensions: &[String],
    ) -> Self {
        let base_dir = base_dir.into();
        let static_folder = static_folder.into();
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                    format!("{}=debug,tower_http=debug", env!("CARGO_CRATE_NAME")).into()
                }),
            )
            .with(tracing_subscriber::fmt::layer())
            .init();

        async fn handle_404() -> (StatusCode, &'static str) {
            println!("non mbr 404");
            (StatusCode::NOT_FOUND, "Not found")
        }
        let handle_404_service = handle_404.into_service();

        async fn handle_mbr_404(
            extract::Path(path): extract::Path<String>,
            request: Request,
        ) -> impl IntoResponse {
            // TODO: need to fall back to builtin css/image/template stuff here
            if let Some((name, bytes)) = DEFAULT_FILES.iter().find(|(name, _)| {
                println!("Comparing path ({}) to name ({})", path, name);
                path.as_str() == *name
            }) {
                (StatusCode::OK, (*bytes).into_response())
            } else {
                println!("no default found");
                (
                    StatusCode::NOT_FOUND,
                    "404 Not found in fallback".into_response(),
                )
            }
        }
        let mbr_builtins = handle_mbr_404.into_service();

        let serve_mbr = ServeDir::new(base_dir.join(".mbr")).not_found_service(mbr_builtins);
        let serve_static_then_404 =
            ServeDir::new(base_dir.join(&static_folder)).not_found_service(handle_404_service);

        let config = ServerState {
            base_dir,
            static_folder,
            markdown_extensions: markdown_extensions.to_owned(),
        };

        let router = Router::new()
            .route("/", get(Self::home_page))
            .nest_service("/.mbr", serve_mbr)
            .route("/{*path}", get(Self::handle))
            .fallback_service(serve_static_then_404)
            .layer(TraceLayer::new_for_http())
            .with_state(config);

        Server { router, ip, port }
    }

    pub async fn start(&self) {
        let addr = SocketAddr::from((self.ip, self.port));
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        tracing::debug!("listening on {}", listener.local_addr().unwrap());
        axum::serve(listener, self.router.clone()).await.unwrap();
    }

    async fn handle(
        extract::Path(path): extract::Path<String>,
        State(config): State<ServerState>,
        req: Request<Body>,
    ) -> Result<impl IntoResponse, StatusCode> {
        tracing::debug!("got request: {}", &path);

        let candidate_path = config.base_dir.join(&path);

        // I need to look at the path, then join it to the base_dir and if there's a matching
        // file, deliver it as-is (yes, this means delivering raw markdown, too, for now)
        if candidate_path.is_file() {
            let static_service = ServeFile::new(candidate_path);
            return static_service
                .oneshot(req)
                .await
                .map(|r| r.into_response())
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
        }

        // if the candidate path is a dir, look for index.md or equiv inside it
        if candidate_path.is_dir() {
            // TODO: really need to check for each of the markdown extensions
            let index = candidate_path.join("index.md");
            if index.is_file() {
                let static_service = ServeFile::new(index);
                return static_service
                    .oneshot(req)
                    .await
                    .map(|r| r.into_response())
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
            }
        }

        // If there isn't a matching file and the path ends in a slash, then I must look to see
        // if there's a corresponding markdown file using each of the configured markdown extensions in order
        let candidate_base = {
            let s = candidate_path.to_string_lossy();
            let trimmed = s.trim_end_matches(std::path::MAIN_SEPARATOR);
            std::path::PathBuf::from(trimmed)
        };

        for ext in config.markdown_extensions.iter() {
            let mut md_path = candidate_base.clone();
            md_path.set_extension(ext);
            if md_path.is_file() {
                let html_output = markdown::render(md_path)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                let templates = templates::Templates::new(&config.base_dir)
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                let html_output = templates
                    .render_markdown(&html_output)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                return Ok(Html(html_output).into_response());
            }
        }

        Err(StatusCode::NOT_FOUND)
    }

    async fn home_page() -> impl IntoResponse {
        // TODO: look for index.{markdown extensions} then index.html then finally fall back to some hard coded html maybe with a list of markdown files in the same dir and immediate children?
        format!("Home")
    }
}

pub const DEFAULT_FILES: &[(&str, &[u8])] = &[
    ("theme.css", include_bytes!("../templates/theme.css")),
    ("user.css", include_bytes!("../templates/user.css")),
];

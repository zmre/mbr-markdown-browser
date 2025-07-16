use axum::{
    body::Body,
    extract::{self, State},
    handler::HandlerWithoutStateExt,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use std::{net::SocketAddr, path::Path};

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
    pub index_file: String,
    pub templates: crate::templates::Templates,
}

impl Server {
    pub fn init<S: Into<String>, P: Into<std::path::PathBuf>>(
        ip: [u8; 4],
        port: u16,
        base_dir: P,
        static_folder: S,
        markdown_extensions: &[String],
        index_file: S,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let base_dir = base_dir.into();
        let static_folder = static_folder.into();
        let index_file = index_file.into();

        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                    format!("{}=debug,tower_http=debug", env!("CARGO_CRATE_NAME")).into()
                }),
            )
            .with(tracing_subscriber::fmt::layer())
            .init();

        let templates = templates::Templates::new(base_dir.as_path())?;

        let config = ServerState {
            base_dir,
            static_folder,
            markdown_extensions: markdown_extensions.to_owned(),
            index_file,
            templates,
        };

        let mbr_builtins = Self::serve_default_mbr.into_service();
        let serve_mbr =
            ServeDir::new(config.base_dir.as_path().join(".mbr")).fallback(mbr_builtins);

        let router = Router::new()
            // .route("/favicon.ico", ServeFile::new())
            .route("/", get(Self::home_page))
            .nest_service("/.mbr", serve_mbr)
            .route("/{*path}", get(Self::handle))
            // .fallback_service(handle_static)
            .layer(TraceLayer::new_for_http())
            .with_state(config);

        Ok(Server { router, ip, port })
    }

    pub async fn start(&self) {
        let addr = SocketAddr::from((self.ip, self.port));
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        tracing::debug!("listening on {}", listener.local_addr().unwrap());
        axum::serve(listener, self.router.clone()).await.unwrap();
    }

    // This is the fallback if the file isn't in the runtime .mbr dir
    pub async fn serve_default_mbr(
        // extract::Path(path): extract::Path<String>,
        request: extract::Request,
    ) -> Result<impl IntoResponse, StatusCode> {
        tracing::debug!("handle_mb4_404");
        let path = request.uri().path().replace("/.mbr", "");
        if let Some((_name, bytes, mime)) = DEFAULT_FILES.iter().find(|(name, _, _)| {
            tracing::debug!("Comparing path ({}) to name ({})", path, name);
            path.as_str() == *name
        }) {
            tracing::debug!("found default");
            let resp = Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", *mime)
                .body(axum::body::Body::from(*bytes))
                .unwrap();

            println!("{:?}", &resp);
            // (StatusCode::OK, resp)
            Ok(resp.into_response())
        } else {
            tracing::debug!("no default found");
            // let resp = Response::builder()
            //     .status(StatusCode::NOT_FOUND)
            //     .body(axum::body::Body::from("404 Not Found in fallback"))
            //     .unwrap();
            // resp.into_response()
            Err(StatusCode::NOT_FOUND)
        }
    }

    async fn handle(
        extract::Path(path): extract::Path<String>,
        State(config): State<ServerState>,
        req: extract::Request<Body>,
    ) -> Result<impl IntoResponse, StatusCode> {
        tracing::debug!("handle: {}", &path);

        let candidate_path = config.base_dir.join(&path);

        // I need to look at the path, then join it to the base_dir and if there's a matching
        // file, deliver it as-is (yes, this means delivering raw markdown, too, for now)
        if candidate_path.is_file() {
            tracing::debug!("found file in root as requested");
            let static_service = ServeFile::new(candidate_path);
            return static_service
                .oneshot(req)
                .await
                .map(|r| r.into_response())
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
        }

        // if the candidate path is a dir, look for index.md or equiv inside it
        if candidate_path.is_dir() {
            let index = candidate_path.join(config.index_file);
            tracing::debug!("checking for folder with index.md in it: {:?}", &index);
            if index.is_file() {
                tracing::debug!("...found");
                return Ok(Self::markdown_to_html(&index, &config.templates)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                    .into_response());
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
                return Ok(Self::markdown_to_html(&md_path, &config.templates)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                    .into_response());
            }
        }

        let static_dir = config
            .base_dir
            .as_path()
            .join(&config.static_folder)
            .canonicalize()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let candidate_static_file = static_dir.join(&path);
        if candidate_static_file.is_file() {
            tracing::debug!("found file in static folder");
            let handle_static = ServeDir::new(static_dir);
            return handle_static
                .oneshot(req)
                .await
                .map(|r| r.into_response())
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
        }

        tracing::debug!("going with a not found code");

        Err(StatusCode::NOT_FOUND)
    }

    async fn markdown_to_html(
        md_path: &Path,
        templates: &crate::templates::Templates,
    ) -> Result<Html<String>, Box<dyn std::error::Error>> {
        let inner_html_output = markdown::render(md_path.to_path_buf()).await?;
        let full_html_output = templates.render_markdown(&inner_html_output).await?;
        Ok(Html(full_html_output))
    }

    async fn home_page() -> impl IntoResponse {
        // TODO: look for index.{markdown extensions} then index.html then finally fall back to some hard coded html maybe with a list of markdown files in the same dir and immediate children?
        tracing::debug!("home");
        format!("Home")
    }
}

pub const DEFAULT_FILES: &[(&str, &[u8], &str)] = &[
    (
        "/theme.css",
        include_bytes!("../templates/theme.css"),
        "text/css",
    ),
    (
        "/user.css",
        include_bytes!("../templates/user.css"),
        "text/css",
    ),
    (
        "/pico.min.css",
        include_bytes!("../templates/pico.min.css"),
        "text/css",
    ),
];

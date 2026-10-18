//! What the server logs, and what `errors.json` answers, for a note whose
//! bytes are not on the device (a cloud provider's dataless file).
//!
//! Its own test binary because it installs a **process-wide** capturing
//! subscriber: the server logs from tokio workers and its blocking pool, which
//! a per-thread capture (`mbr_core`'s `capture_tracing`) cannot see. Each test
//! uses note names of its own and only reads the lines that name them, so the
//! tests can share the buffer and still run in parallel.

use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use mbr_core::config::Config;
use mbr_core::vault::{Availability, MemVault, Vault};
use mbr_server::{EmbeddedStorage, Server, ServerConfig};
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone)]
struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("log buffer mutex poisoned")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for SharedBuffer {
    type Writer = SharedBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The process-wide log, at DEBUG, installed before any server (whose own
/// `try_init` then finds a subscriber already set and leaves it).
fn logs() -> &'static SharedBuffer {
    static BUFFER: OnceLock<SharedBuffer> = OnceLock::new();
    BUFFER.get_or_init(|| {
        let buffer = SharedBuffer(Arc::new(Mutex::new(Vec::new())));
        tracing_subscriber::fmt()
            .with_writer(buffer.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .init();
        buffer
    })
}

/// Every captured line that mentions `needle`.
fn lines_about(needle: &str) -> Vec<String> {
    let bytes = logs().0.lock().expect("log buffer mutex poisoned").clone();
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|line| line.contains(needle))
        .map(str::to_owned)
        .collect()
}

fn at_level(lines: &[String], level: &str) -> Vec<String> {
    let marker = format!(" {level} ");
    lines
        .iter()
        .filter(|line| line.contains(&marker))
        .cloned()
        .collect()
}

async fn server(vault: &Arc<MemVault>) -> Server {
    logs();
    let mut config = ServerConfig::from(&Config::default());
    config.base_dir = vault.root().to_path_buf();
    config.oembed_timeout_ms = 0;
    config.embedded = Some(EmbeddedStorage {
        vault: Arc::clone(vault) as Arc<dyn Vault>,
        snapshot: None,
    });
    let server = Server::init(config).expect("init");
    server.repo().wait_for_media().await;
    server
}

async fn get(server: &Server, path: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    let request = Request::get(path)
        .header(header::ACCEPT, "text/html")
        .body(Body::empty())
        .unwrap();
    let response = server.router.clone().oneshot(request).await.unwrap();
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    (
        parts.status,
        parts.headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn retrying_a_not_downloaded_page_logs_nothing_above_debug() {
    let vault = Arc::new(
        MemVault::new()
            .with_file("index.md", "# Home\n")
            .with_file("kumquat-pending.md", "# Kumquat\n"),
    );
    vault.set_availability("kumquat-pending.md", Availability::NotDownloaded);
    let server = server(&vault).await;

    // What the page's `Refresh: 2` does: the page, and its errors.json, again
    // and again. `MemVault` completes a requested download at once, so the
    // note is evicted again before every round.
    for _ in 0..3 {
        vault.set_availability("kumquat-pending.md", Availability::NotDownloaded);
        let (status, _, _) = get(&server, "/kumquat-pending/").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        vault.set_availability("kumquat-pending.md", Availability::NotDownloaded);
        let (status, _, _) = get(&server, "/kumquat-pending/errors.json").await;
        assert_eq!(status, StatusCode::OK);
    }

    let lines = lines_about("kumquat-pending");
    assert!(
        at_level(&lines, "DEBUG")
            .iter()
            .any(|line| line.contains("not downloaded yet")),
        "the state is still logged, at DEBUG: {lines:#?}"
    );
    assert_eq!(at_level(&lines, "ERROR"), Vec::<String>::new());
    assert_eq!(at_level(&lines, "WARN"), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn errors_json_for_a_not_downloaded_note_answers_cleanly() {
    let vault = Arc::new(MemVault::new().with_file("index.md", "# Home\n").with_file(
        "cloudberry-pending.md",
        "# Cloudberry\n\n[gone](missing.md)\n",
    ));
    vault.set_availability("cloudberry-pending.md", Availability::NotDownloaded);
    let server = server(&vault).await;

    let (status, headers, body) = get(&server, "/cloudberry-pending/errors.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "application/json");
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "no-store",
        "the load after the download must not reuse this answer"
    );
    let json: serde_json::Value = serde_json::from_str(&body).expect("JSON, not an HTML page");
    assert_eq!(
        json,
        serde_json::json!({
            "page_url": "/cloudberry-pending/",
            "errors": [],
            "not_downloaded": true
        })
    );

    // Once the bytes are there, the real list (its broken link included),
    // without the marker.
    vault.set_availability("cloudberry-pending.md", Availability::Local);
    let (status, _, body) = get(&server, "/cloudberry-pending/errors.json").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json.get("not_downloaded").is_none(), "{json}");
    assert_eq!(json["errors"].as_array().map(Vec::len), Some(1), "{json}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_real_render_failure_still_logs_an_error() {
    // Not UTF-8: the read fails for a reason that is not "not downloaded".
    let vault = Arc::new(
        MemVault::new()
            .with_file("index.md", "# Home\n")
            .with_file("durian-broken.md", [0xff_u8, 0xfe, 0x00, 0x23]),
    );
    let server = server(&vault).await;

    let (status, _, _) = get(&server, "/durian-broken/").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let (status, _, _) = get(&server, "/durian-broken/errors.json").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "unchanged for a real failure"
    );

    let errors = at_level(&lines_about("durian-broken"), "ERROR");
    assert!(
        errors
            .iter()
            .any(|line| line.contains("Error rendering markdown")),
        "{errors:#?}"
    );
    assert!(
        errors
            .iter()
            .any(|line| line.contains("errors.json: failed to render page")),
        "{errors:#?}"
    );
    // The request-level line for a real 5xx is still an ERROR (only 503, the
    // "retry shortly" answer, was lowered).
    assert!(
        errors.iter().any(|line| line.contains("response failed")
            && line.contains("500")
            && line.contains("uri=/durian-broken/")),
        "{errors:#?}"
    );
    // The backlink index is built by a background task, so its WARN can land
    // after the requests above return (it did on a loaded CI runner). Wait for
    // it, bounded, rather than reading the log once.
    let backlink_warned = || {
        at_level(&lines_about("durian-broken"), "WARN")
            .iter()
            .any(|line| line.contains("Backlink index: failed to read"))
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !backlink_warned() && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        backlink_warned(),
        "the backlink index still warns about a file it cannot read"
    );
}

/// The edit endpoints' read of a not-downloaded note (a task toggle after an
/// edit on another device) answers 503 and, like a page, logs it at DEBUG.
#[tokio::test(flavor = "multi_thread")]
async fn an_edit_read_of_a_not_downloaded_note_logs_nothing_above_debug() {
    let vault = Arc::new(
        MemVault::new()
            .with_file("index.md", "# Home\n")
            .with_file("tamarind-pending.md", "- [ ] one\n"),
    );
    logs();
    let mut config = ServerConfig::from(&Config::default());
    config.base_dir = vault.root().to_path_buf();
    config.oembed_timeout_ms = 0;
    config.edit_enabled = true;
    config.embedded = Some(EmbeddedStorage {
        vault: Arc::clone(&vault) as Arc<dyn Vault>,
        snapshot: None,
    });
    let server = Server::init(config).expect("init");
    server.repo().wait_for_media().await;
    vault.set_availability("tamarind-pending.md", Availability::NotDownloaded);
    vault.hold_downloads(true);

    let mut request = Request::get("/.mbr/raw/tamarind-pending.md")
        .header(header::HOST, "localhost")
        .header("X-MBR-Edit", "1")
        .body(Body::empty())
        .unwrap();
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            0,
        ))));
    let response = server.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let lines = lines_about("tamarind-pending");
    assert!(
        at_level(&lines, "DEBUG")
            .iter()
            .any(|line| line.contains("not downloaded after")),
        "{lines:#?}"
    );
    assert_eq!(at_level(&lines, "ERROR"), Vec::<String>::new());
    assert_eq!(at_level(&lines, "WARN"), Vec::<String>::new());
}

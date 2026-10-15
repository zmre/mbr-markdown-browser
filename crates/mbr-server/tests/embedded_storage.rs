//! A server over [`EmbeddedStorage`]: a caller-supplied vault with no local
//! paths (an in-memory one here), driven in-process through the router the way
//! the iOS app drives it.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use mbr_core::config::Config;
use mbr_core::vault::{Availability, MemVault, Vault, VaultPath};
use mbr_server::{EmbeddedStorage, Server, ServerConfig};
use tower::ServiceExt;

const NOTE: &str =
    "---\ntitle: Kumquat Note\ntags: [fruit]\n---\n# Hello\n\nSome searchable kumquat text.\n";

fn vault() -> Arc<MemVault> {
    let video: Vec<u8> = (0..=255u8).cycle().take(600_000).collect();
    Arc::new(
        MemVault::new()
            .with_file("note.md", NOTE)
            .with_file("cloud.md", "# Cloud\n\nkumquat in the cloud\n")
            .with_file("docs/guide.md", "# Guide\n\nSee [the note](../note.md).\n")
            .with_file("media/clip.mp4", video)
            .with_file("media/far.png", "png"),
    )
}

fn config(vault: &Arc<MemVault>, snapshot: Option<mbr_core::repo::RepoSnapshot>) -> ServerConfig {
    let mut config = ServerConfig::from(&Config::default());
    config.base_dir = vault.root().to_path_buf();
    config.oembed_timeout_ms = 0;
    config.embedded = Some(EmbeddedStorage {
        vault: Arc::clone(vault) as Arc<dyn Vault>,
        snapshot,
    });
    config
}

async fn server(vault: &Arc<MemVault>) -> Server {
    let server = Server::init(config(vault, None)).expect("init");
    server.repo().wait_for_media().await;
    server
}

async fn send(
    server: &Server,
    request: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let response = server.router.clone().oneshot(request).await.unwrap();
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    (parts.status, parts.headers, bytes.to_vec())
}

fn get(path: &str) -> Request<Body> {
    Request::get(path).body(Body::empty()).unwrap()
}

fn get_range(path: &str, range: &str) -> Request<Body> {
    Request::get(path)
        .header(header::RANGE, range)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn pages_render_from_a_vault_without_local_paths() {
    let vault = vault();
    let server = server(&vault).await;
    let (status, headers, body) = send(&server, get("/note/")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .contains("text/html")
    );
    let html = String::from_utf8(body).unwrap();
    assert!(html.contains("kumquat"), "{html}");

    let (status, _, body) = send(&server, get("/docs/guide/")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8(body).unwrap().contains("Guide"));

    let (status, _, body) = send(&server, get("/.mbr/site.json")).await;
    assert_eq!(status, StatusCode::OK);
    let site: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let urls: Vec<&str> = site["markdown_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["url_path"].as_str())
        .collect();
    assert!(
        urls.contains(&"/note/") && urls.contains(&"/docs/guide/"),
        "{urls:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn assets_stream_through_the_vault_with_ranges() {
    let vault = vault();
    let server = server(&vault).await;
    let expected = vault.contents("media/clip.mp4").unwrap();

    let (status, headers, body) = send(&server, get("/media/clip.mp4")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::ACCEPT_RANGES], "bytes");
    assert_eq!(headers[header::CONTENT_TYPE], "video/mp4");
    assert_eq!(headers[header::CONTENT_LENGTH], expected.len().to_string());
    assert!(
        !headers.contains_key(header::CONTENT_ENCODING),
        "video is never compressed"
    );
    assert_eq!(body, expected);

    let (status, headers, body) = send(&server, get_range("/media/clip.mp4", "bytes=0-99")).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        headers[header::CONTENT_RANGE],
        format!("bytes 0-99/{}", expected.len())
    );
    assert_eq!(body, expected[..100]);

    let (status, _, body) = send(&server, get_range("/media/clip.mp4", "bytes=500000-")).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(body, expected[500_000..]);

    let (status, _, body) = send(&server, get_range("/media/clip.mp4", "bytes=-50")).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(body, expected[expected.len() - 50..]);

    let (status, headers, _) =
        send(&server, get_range("/media/clip.mp4", "bytes=999999999-")).await;
    assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(
        headers[header::CONTENT_RANGE],
        format!("bytes */{}", expected.len())
    );
    let (status, _, _) = send(&server, get_range("/media/clip.mp4", "bytes=nonsense")).await;
    assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);

    let (status, _, _) = send(&server, get("/media/missing.mp4")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn not_downloaded_files_get_a_retry_answer_and_a_download_request() {
    let vault = vault();
    vault.set_availability("cloud.md", Availability::NotDownloaded);
    vault.set_availability("media/far.png", Availability::NotDownloaded);
    let server = server(&vault).await;

    // Still listed, from the listing alone.
    let (_, _, body) = send(&server, get("/.mbr/site.json")).await;
    assert!(String::from_utf8(body).unwrap().contains("/cloud/"));

    // Content search skips it and says so. (Before the page request below:
    // `MemVault` completes a requested download at once.)
    let search = Request::post("/.mbr/search")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"q":"kumquat","scope":"content"}"#))
        .unwrap();
    let (status, _, body) = send(&server, search).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["not_downloaded"], 1, "{json}");

    let (status, headers, body) = send(&server, get("/cloud/")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(headers[header::RETRY_AFTER], "2");
    assert_eq!(headers["refresh"], "2");
    assert!(String::from_utf8(body).unwrap().contains("Downloading"));

    let (status, headers, _) = send(&server, get("/media/far.png")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(headers[header::RETRY_AFTER], "2");

    let requested: Vec<String> = vault
        .download_requests()
        .iter()
        .map(|p| p.to_string())
        .collect();
    assert!(requested.contains(&"cloud.md".to_string()), "{requested:?}");
    assert!(
        requested.contains(&"media/far.png".to_string()),
        "{requested:?}"
    );

    // Once downloaded (the request above completed it), it renders.
    let (status, _, _) = send(&server, get("/cloud/")).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn refresh_applies_what_changed_in_the_vault() {
    let vault = vault();
    let server = server(&vault).await;
    assert!(server.refresh().await.is_empty(), "nothing changed yet");

    vault.insert_file("fresh.md", "---\ntitle: Fresh Arrival\n---\n");
    vault.insert_file("note.md", "---\ntitle: Renamed Title\n---\n# Hello\n");
    vault.set_modified(
        "note.md",
        SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000),
    );
    vault
        .remove_file(&VaultPath::new("docs/guide.md").unwrap())
        .unwrap();

    let diff = server.refresh().await;
    assert_eq!(
        (diff.created.len(), diff.modified.len(), diff.deleted.len()),
        (1, 1, 1),
        "{diff:?}"
    );
    let (_, _, body) = send(&server, get("/.mbr/site.json")).await;
    let site = String::from_utf8(body).unwrap();
    assert!(site.contains("Fresh Arrival"), "created note indexed");
    assert!(
        site.contains("Renamed Title"),
        "modified frontmatter re-read"
    );
    assert!(!site.contains("/docs/guide/"), "deleted note dropped");
    assert!(server.refresh().await.is_empty(), "applied once");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_snapshot_restores_without_reading_and_a_stale_one_is_ignored() {
    let vault = vault();
    let first = server(&vault).await;
    assert_eq!(first.restored_files(), None);
    let snapshot = first.repo().snapshot();
    let (_, _, scanned) = send(&first, get("/.mbr/site.json")).await;
    drop(first);

    let restored = Server::init(config(&vault, Some(snapshot.clone()))).unwrap();
    assert_eq!(restored.restored_files(), Some(snapshot.len()));
    restored.repo().wait_for_scan().await;
    let (_, _, body) = send(&restored, get("/.mbr/site.json")).await;
    assert_eq!(body, scanned, "same site.json as the scan that took it");
    assert!(restored.refresh().await.is_empty());

    // Another scan configuration: the snapshot is refused and the vault scanned.
    let mut other = config(&vault, Some(snapshot));
    other.markdown_extensions.push("markdown".into());
    let rescanned = Server::init(other).unwrap();
    assert_eq!(rescanned.restored_files(), None);
    rescanned.repo().wait_for_scan().await;
    let (_, _, body) = send(&rescanned, get("/.mbr/site.json")).await;
    assert!(String::from_utf8(body).unwrap().contains("/note/"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_large_refresh_is_applied_file_by_file_and_keeps_evicted_metadata() {
    let vault = vault();
    let server = server(&vault).await;
    // Evicted after it was indexed: still listed, frontmatter kept.
    vault.set_availability("note.md", Availability::NotDownloaded);
    // More changes than the watcher's full-rescan threshold.
    for i in 0..80 {
        vault.insert_file(
            &format!("batch/n{i:02}.md"),
            format!("---\ntitle: Batch {i}\n---\n"),
        );
    }
    vault.insert_file("media/new.png", "png");
    let diff = server.refresh().await;
    assert_eq!(diff.created.len(), 81, "{diff:?}");
    assert_eq!(diff.evicted.len(), 1);
    let (_, _, body) = send(&server, get("/.mbr/site.json")).await;
    let site = String::from_utf8(body).unwrap();
    assert!(site.contains("Batch 79"));
    assert!(
        site.contains("Kumquat Note"),
        "a rescan would have dropped the evicted note's title"
    );
    let (status, _, _) = send(&server, get("/media/new.png")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(server.refresh().await.is_empty());
}

/// A vault whose `stat` of one file fails, as `PermissionDenied`, the first
/// `failures` times: what Windows answers for a moment while another
/// request's atomic write is renaming over that file.
#[derive(Debug)]
struct FlakyStatVault {
    inner: Arc<MemVault>,
    target: VaultPath,
    failures: std::sync::Mutex<usize>,
}

impl Vault for FlakyStatVault {
    fn root(&self) -> &std::path::Path {
        self.inner.root()
    }
    fn stat(
        &self,
        path: &VaultPath,
    ) -> Result<Option<mbr_core::vault::Entry>, mbr_core::vault::VaultError> {
        if *path == self.target {
            let mut left = self.failures.lock().unwrap();
            if *left > 0 {
                *left -= 1;
                return Err(mbr_core::vault::VaultError::PermissionDenied {
                    path: path.to_string(),
                });
            }
        }
        self.inner.stat(path)
    }
    fn list_dir(
        &self,
        dir: &VaultPath,
    ) -> Result<Vec<mbr_core::vault::Entry>, mbr_core::vault::VaultError> {
        self.inner.list_dir(dir)
    }
    fn canonicalize(&self, path: &VaultPath) -> Result<VaultPath, mbr_core::vault::VaultError> {
        self.inner.canonicalize(path)
    }
    fn read(&self, path: &VaultPath) -> Result<Vec<u8>, mbr_core::vault::VaultError> {
        self.inner.read(path)
    }
    fn read_prefix(
        &self,
        path: &VaultPath,
        max_len: usize,
    ) -> Result<Vec<u8>, mbr_core::vault::VaultError> {
        self.inner.read_prefix(path, max_len)
    }
    fn write_atomic(
        &self,
        path: &VaultPath,
        bytes: &[u8],
    ) -> Result<(), mbr_core::vault::VaultError> {
        self.inner.write_atomic(path, bytes)
    }
    fn create_dir_all(&self, path: &VaultPath) -> Result<(), mbr_core::vault::VaultError> {
        self.inner.create_dir_all(path)
    }
    fn remove_file(&self, path: &VaultPath) -> Result<(), mbr_core::vault::VaultError> {
        self.inner.remove_file(path)
    }
    fn rename(&self, from: &VaultPath, to: &VaultPath) -> Result<(), mbr_core::vault::VaultError> {
        self.inner.rename(from, to)
    }
}

/// Regression for the Windows CI flake in
/// `test_concurrent_task_toggles_on_one_file_both_land`: a task toggle whose
/// path resolution races another write's rename saw the note as absent and
/// answered 404. Resolution now asks again after a short pause. Deterministic
/// on every OS: the vault fails the first two stats of the note.
#[tokio::test(flavor = "multi_thread")]
async fn an_edit_survives_a_momentary_stat_failure_on_its_note() {
    let inner = Arc::new(MemVault::new().with_file("notes.md", "# Notes\n\n- [ ] one\n"));
    let mut config = ServerConfig::from(&Config::default());
    config.base_dir = inner.root().to_path_buf();
    config.oembed_timeout_ms = 0;
    config.edit_enabled = true;
    let flaky = Arc::new(FlakyStatVault {
        inner: Arc::clone(&inner),
        target: VaultPath::new("notes.md").unwrap(),
        failures: std::sync::Mutex::new(0),
    });
    config.embedded = Some(EmbeddedStorage {
        vault: Arc::clone(&flaky) as Arc<dyn Vault>,
        snapshot: None,
    });
    let server = Server::init(config).expect("init");
    server.repo().wait_for_media().await;

    // Only now, so the scan saw the note: the next two stats fail.
    *flaky.failures.lock().unwrap() = 2;
    let mut toggle = Request::post("/.mbr/task")
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/json")
        .header("X-MBR-Edit", "1")
        .body(Body::from(
            r#"{"path":"notes.md","line":3,"expected":"- [ ] one","to":"done"}"#,
        ))
        .unwrap();
    toggle
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            0,
        ))));
    let (status, _, body) = send(&server, toggle).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let text = String::from_utf8(inner.contents("notes.md").unwrap()).unwrap();
    assert!(text.contains("- [x] one"), "{text}");
}

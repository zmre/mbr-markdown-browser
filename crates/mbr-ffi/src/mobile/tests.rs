//! `MbrApp` end to end, on the desktop: requests in through `handle`, files
//! on disk in a temp folder, writes through a recording coordinator.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::coordinated_vault::tests::RecordingCoordinator;
use super::*;

const NOTE: &str = "# Hello\n\nSome searchable kumquat text.\n\n- [ ] buy milk\n";

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    futures::executor::block_on(future)
}

/// A vault folder. The `.mbr/` marker is there so a desktop `Config::read`
/// would stop here too; the mobile open never looks upward anyway.
fn vault() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join(".mbr")).unwrap();
    std::fs::write(dir.path().join("note.md"), NOTE).unwrap();
    dir
}

fn open(dir: &Path) -> Arc<MbrApp> {
    MbrApp::open(dir.to_string_lossy().into_owned(), None, None, None).expect("open vault")
}

fn open_coordinated(dir: &Path) -> (Arc<MbrApp>, Arc<RecordingCoordinator>) {
    let coordinator = Arc::new(RecordingCoordinator::default());
    let app = MbrApp::open(
        dir.to_string_lossy().into_owned(),
        None,
        None,
        Some(Arc::clone(&coordinator) as Arc<dyn FileCoordinator>),
    )
    .expect("open vault");
    (app, coordinator)
}

fn url(path: &str) -> String {
    format!("mbr://{MOBILE_HOST}{path}")
}

struct Answer {
    status: u16,
    headers: Vec<MobileHeader>,
    body: Vec<u8>,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

fn request(
    app: &MbrApp,
    method: &str,
    path: &str,
    headers: Vec<MobileHeader>,
    body: &[u8],
) -> Answer {
    block_on(async {
        let response = app
            .handle(method.to_owned(), url(path), headers, body.to_vec())
            .await
            .expect("handle");
        Answer {
            status: response.status(),
            headers: response.headers(),
            body: response.read_all().await.expect("body"),
        }
    })
}

fn get(app: &MbrApp, path: &str) -> Answer {
    request(app, "GET", path, Vec::new(), &[])
}

fn navigate(app: &MbrApp, path: &str) -> Answer {
    request(
        app,
        "GET",
        path,
        vec![MobileHeader::new(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )],
        &[],
    )
}

fn edit_headers() -> Vec<MobileHeader> {
    vec![
        MobileHeader::new("Content-Type", "application/json"),
        MobileHeader::new("X-MBR-Edit", "1"),
        MobileHeader::new("Origin", &format!("mbr://{MOBILE_HOST}")),
    ]
}

fn toggle_milk(app: &MbrApp, origin: &str) -> Answer {
    let mut headers = edit_headers();
    headers.retain(|h| h.name != "Origin");
    headers.push(MobileHeader::new("Origin", origin));
    request(
        app,
        "POST",
        "/.mbr/task",
        headers,
        br#"{"path":"note.md","line":5,"expected":"- [ ] buy milk","to":"done"}"#,
    )
}

fn site_urls(app: &MbrApp) -> Vec<String> {
    let site: serde_json::Value =
        serde_json::from_slice(&get(app, "/.mbr/site.json").body).unwrap();
    let mut urls: Vec<String> = site["markdown_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["url_path"].as_str().map(str::to_owned))
        .collect();
    urls.sort();
    urls
}

// ---- requests --------------------------------------------------------------

#[test]
fn get_renders_a_markdown_page_with_editing_on() {
    let dir = vault();
    let app = open(dir.path());
    let page = get(&app, "/note/");
    assert_eq!(page.status, 200);
    assert!(
        page.header("content-type")
            .is_some_and(|v| v.contains("text/html"))
    );
    let html = page.text();
    assert!(html.contains("kumquat"), "page body missing note text");
    // Regression: a symlinked root (macOS `/tmp`, iOS `/var`) used to make
    // every page's source path climb out of the vault.
    assert!(
        !html.contains("../../"),
        "source path escapes the vault root"
    );
    // The #336 Host gate: `mbr://localhost` pages advertise editing.
    assert!(html.contains("editEnabled: true"), "editing not advertised");
}

#[test]
fn slashless_page_url_redirects_like_the_desktop_server() {
    let dir = vault();
    let app = open(dir.path());
    let response = get(&app, "/note");
    assert_eq!(response.status, 301);
    assert_eq!(response.header("location"), Some("/note/"));
}

#[test]
fn page_load_redirect_becomes_a_self_navigating_document() {
    let dir = vault();
    let app = open(dir.path());
    let response = navigate(&app, "/note");
    assert_eq!(response.status, 200);
    assert!(
        response.text().contains(r#"location.replace("/note/")"#),
        "no client-side redirect: {}",
        response.text()
    );
}

#[test]
fn embedded_assets_are_served_uncompressed() {
    let dir = vault();
    let app = open(dir.path());
    let response = request(
        &app,
        "GET",
        "/.mbr/components/mbr-components.min.js",
        vec![MobileHeader::new("Accept-Encoding", "gzip, deflate, br")],
        &[],
    );
    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-encoding"), None);
    assert!(!response.body.is_empty());
}

#[test]
fn post_search_reads_the_json_body() {
    let dir = vault();
    let app = open(dir.path());
    block_on(app.wait_until_ready()).expect("ready");
    let response = request(
        &app,
        "POST",
        "/.mbr/search",
        vec![MobileHeader::new("Content-Type", "application/json")],
        br#"{"q":"kumquat","scope":"all"}"#,
    );
    assert_eq!(response.status, 200);
    let json: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    let results = json["results"].as_array().expect("results array");
    assert!(
        results
            .iter()
            .any(|r| r["url_path"].as_str() == Some("/note/")),
        "search did not find the note: {json}"
    );
}

#[test]
fn task_toggle_from_the_mbr_origin_writes_through_the_coordinator() {
    let dir = vault();
    let (app, coordinator) = open_coordinated(dir.path());
    let response = toggle_milk(&app, &format!("mbr://{MOBILE_HOST}"));
    assert_eq!(response.status, 200, "{}", response.text());
    let on_disk = std::fs::read_to_string(dir.path().join("note.md")).unwrap();
    assert!(on_disk.contains("- [x] buy milk"), "not written: {on_disk}");
    let calls = coordinator.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].starts_with("write ") && calls[0].ends_with("note.md"));
}

#[test]
fn a_same_origin_post_without_an_origin_header_may_edit() {
    // WebKit sends no `Origin` on a same-origin `mbr://` POST.
    let dir = vault();
    let app = open(dir.path());
    let mut headers = edit_headers();
    headers.retain(|h| h.name != "Origin");
    let response = request(
        &app,
        "POST",
        "/.mbr/task",
        headers,
        br#"{"path":"note.md","line":5,"expected":"- [ ] buy milk","to":"done"}"#,
    );
    assert_eq!(response.status, 200, "{}", response.text());
}

#[test]
fn task_toggle_from_a_foreign_origin_is_refused() {
    let dir = vault();
    let (app, coordinator) = open_coordinated(dir.path());
    let response = toggle_milk(&app, "https://evil.example");
    assert_eq!(response.status, 403);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("note.md")).unwrap(),
        NOTE
    );
    assert!(coordinator.calls().is_empty());
}

#[test]
fn a_flashcard_review_is_written_through_the_coordinator() {
    let dir = vault();
    std::fs::write(
        dir.path().join("deck.md"),
        "---\ntype: flashcard\n---\n\nCapital of France?\n: Paris\n",
    )
    .unwrap();
    let (app, coordinator) = open_coordinated(dir.path());
    let at = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let body = serde_json::json!({
        "path": "deck.md",
        "line": 5,
        "expected": "Capital of France?",
        "rating": "good",
        "at": at,
    });
    let response = request(
        &app,
        "POST",
        "/.mbr/flashcard-review",
        edit_headers(),
        &serde_json::to_vec(&body).unwrap(),
    );
    assert_eq!(response.status, 200, "{}", response.text());
    let deck = std::fs::read_to_string(dir.path().join("deck.md")).unwrap();
    assert!(deck.contains(&format!("{at} - Good")), "{deck}");
    assert_eq!(coordinator.calls().len(), 1);
}

#[test]
fn large_assets_stream_in_chunks_and_honour_ranges() {
    let dir = vault();
    let video: Vec<u8> = (0..=255u8).cycle().take(3 * 1024 * 1024 + 7).collect();
    std::fs::write(dir.path().join("clip.mp4"), &video).unwrap();
    let app = open(dir.path());

    let chunks = block_on(async {
        let response = app
            .handle("GET".into(), url("/clip.mp4"), Vec::new(), Vec::new())
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let mut chunks = Vec::new();
        while let Some(chunk) = response.next_chunk().await.unwrap() {
            chunks.push(chunk);
        }
        chunks
    });
    assert!(chunks.len() > 1, "one {}-byte chunk", chunks[0].len());
    assert!(
        chunks.iter().all(|c| c.len() <= 1024 * 1024),
        "bounded chunks"
    );
    assert_eq!(chunks.concat(), video);

    let partial = request(
        &app,
        "GET",
        "/clip.mp4",
        vec![MobileHeader::new("Range", "bytes=0-99")],
        &[],
    );
    assert_eq!(partial.status, 206);
    assert_eq!(
        partial.header("content-range"),
        Some(format!("bytes 0-99/{}", video.len()).as_str())
    );
    assert_eq!(partial.body, video[..100]);
    let refused = request(
        &app,
        "GET",
        "/clip.mp4",
        vec![MobileHeader::new("Range", "bytes=99999999-")],
        &[],
    );
    assert_eq!(refused.status, 416);
}

// ---- opening ---------------------------------------------------------------

#[test]
fn the_root_is_the_picked_folder_even_inside_a_repository() {
    let outer = vault();
    std::fs::write(
        outer.path().join(".mbr/config.toml"),
        "index_file = \"home.md\"\n",
    )
    .unwrap();
    let inner = outer.path().join("sub");
    std::fs::create_dir(&inner).unwrap();
    std::fs::write(inner.join("inner.md"), "# Inner\n").unwrap();
    let app = open(&inner);
    assert_eq!(Path::new(&app.root_path()), inner.canonicalize().unwrap());
    assert_eq!(
        current(&app.served).pages.index_file,
        "index.md",
        "outer config not read"
    );
    assert_eq!(site_urls(&app), vec!["/inner/"]);
}

#[test]
fn a_broken_config_file_falls_back_to_the_defaults_and_is_reported() {
    for (label, toml) in [
        (
            "unparseable",
            "index_file = \"home.md\"\nthis is [not toml\n",
        ),
        (
            "invalid value",
            "index_file = \"home.md\"\ngraph_depth = 99\n",
        ),
        ("wrong type", "index_file = 7\n"),
    ] {
        let dir = vault();
        std::fs::write(dir.path().join(".mbr/config.toml"), toml).unwrap();
        // The desktop still refuses it.
        assert!(
            mbr_core::config::Config::read(dir.path()).is_err(),
            "{label}: desktop must fail fast"
        );

        let app = open(dir.path());
        let report = block_on(app.wait_until_ready()).unwrap();
        assert_eq!(
            current(&app.served).pages.index_file,
            "index.md",
            "{label}: defaults used"
        );
        let error = report.config_error.clone().expect("reported");
        let expected = match label {
            "unparseable" => "config.toml",
            "invalid value" => "graph_depth",
            _ => "index_file",
        };
        assert!(error.contains(expected), "{label}: {error}");
        assert_eq!(app.config_error(), Some(error.clone()));
        assert_eq!(site_urls(&app), vec!["/note/"], "{label}: still serves");

        let errors: serde_json::Value =
            serde_json::from_slice(&get(&app, "/note/errors.json").body).unwrap();
        let entries = errors["errors"].as_array().unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e["type"] == "config_parse_error" && e["message"] == error.as_str()),
            "{label}: {errors}"
        );
    }

    // A valid config is used, and nothing is reported.
    let dir = vault();
    std::fs::write(
        dir.path().join(".mbr/config.toml"),
        "index_file = \"home.md\"\n",
    )
    .unwrap();
    let app = open(dir.path());
    let report = block_on(app.wait_until_ready()).unwrap();
    assert_eq!(current(&app.served).pages.index_file, "home.md");
    assert_eq!(report.config_error, None);
    let errors = get(&app, "/note/errors.json").text();
    assert!(!errors.contains("config_parse_error"), "{errors}");
}

#[test]
fn open_rejects_a_missing_folder_and_a_file() {
    let result = MbrApp::open("/definitely/not/a/vault".into(), None, None, None);
    assert!(matches!(result, Err(MobileError::Open { .. })));
    let dir = vault();
    let file = dir.path().join("note.md").to_string_lossy().into_owned();
    assert!(matches!(
        MbrApp::open(file, None, None, None),
        Err(MobileError::Open { .. })
    ));
}

// ---- not downloaded ----------------------------------------------------------

#[test]
fn a_placeholder_note_is_listed_answers_downloading_and_is_requested() {
    if !cfg!(target_vendor = "apple") {
        return; // placeholders are only mapped on Apple platforms
    }
    let dir = vault();
    std::fs::write(dir.path().join(".cloud.md.icloud"), "plist").unwrap();
    let (app, coordinator) = open_coordinated(dir.path());
    block_on(app.wait_until_ready()).unwrap();
    assert_eq!(site_urls(&app), vec!["/cloud/", "/note/"]);

    let page = navigate(&app, "/cloud/");
    assert_eq!(page.status, 503);
    assert_eq!(page.header("retry-after"), Some("2"));
    assert!(page.text().contains("Downloading"));
    let downloads = coordinator.downloads.lock().unwrap().clone();
    assert_eq!(downloads.len(), 1, "{downloads:?}");
    assert!(
        downloads[0].ends_with("/cloud.md"),
        "the real name: {downloads:?}"
    );
}

// ---- the index cache and refresh ---------------------------------------------

fn open_cached(dir: &Path, cache: &Path) -> Arc<MbrApp> {
    MbrApp::open(
        dir.to_string_lossy().into_owned(),
        Some(cache.to_string_lossy().into_owned()),
        Some("test-vault".into()),
        None,
    )
    .unwrap()
}

#[test]
fn the_index_is_saved_restored_and_diffed() {
    let dir = vault();
    std::fs::create_dir(dir.path().join("docs")).unwrap();
    std::fs::write(dir.path().join("docs/a.md"), "---\ntitle: Alpha\n---\n").unwrap();
    std::fs::write(dir.path().join("docs/gone.md"), "# Gone").unwrap();
    let cache = tempfile::tempdir().unwrap();

    let cold = open_cached(dir.path(), cache.path());
    let report = block_on(cold.wait_until_ready()).unwrap();
    assert!(!report.from_cache);
    assert_eq!(report.indexed_files, 3);
    let cache_file = cold.index_cache_path().unwrap();
    assert!(Path::new(&cache_file).is_file(), "snapshot saved");
    assert!(cache_file.starts_with(&*cache.path().to_string_lossy()));
    let scanned = get(&cold, "/.mbr/site.json").body;
    drop(cold);

    // Unchanged: restored, nothing to diff, the same site.json.
    let warm = open_cached(dir.path(), cache.path());
    let report = block_on(warm.wait_until_ready()).unwrap();
    assert!(report.from_cache);
    assert_eq!(report.restored_files, 3);
    assert!(report.refresh.is_empty(), "{report:?}");
    assert_eq!(get(&warm, "/.mbr/site.json").body, scanned);
    let saved_at = std::fs::metadata(&cache_file).unwrap().modified().unwrap();
    drop(warm);

    // Changed while closed: the diff after the restore catches it.
    std::fs::write(dir.path().join("docs/a.md"), "---\ntitle: Alpha Two\n---\n").unwrap();
    filetime_bump(&dir.path().join("docs/a.md"));
    std::fs::remove_file(dir.path().join("docs/gone.md")).unwrap();
    std::fs::write(dir.path().join("new.md"), "---\ntitle: Newcomer\n---\n").unwrap();
    let warm = open_cached(dir.path(), cache.path());
    let report = block_on(warm.wait_until_ready()).unwrap();
    assert!(report.from_cache);
    assert_eq!(
        (
            report.refresh.added,
            report.refresh.changed,
            report.refresh.removed
        ),
        (1, 1, 1),
        "{report:?}"
    );
    assert_eq!(
        report.refresh.changed_pages,
        vec!["/docs/a/", "/docs/gone/", "/new/"]
    );
    let site = get(&warm, "/.mbr/site.json").text();
    assert!(site.contains("Alpha Two") && site.contains("Newcomer"));
    assert!(!site.contains("/docs/gone/"));
    assert!(
        std::fs::metadata(&cache_file).unwrap().modified().unwrap() >= saved_at,
        "the changed index was saved again"
    );
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&cache_file).unwrap()).unwrap();
    assert!(
        saved.to_string().contains("Newcomer"),
        "snapshot has the new note"
    );
}

#[test]
fn a_corrupt_or_foreign_cache_is_rebuilt_not_fatal() {
    let dir = vault();
    let cache = tempfile::tempdir().unwrap();
    let first = open_cached(dir.path(), cache.path());
    let path = first.index_cache_path().unwrap();
    block_on(first.wait_until_ready()).unwrap();
    drop(first);

    for garbage in [&b"{ truncated"[..], br#"{"format":999}"#, b""] {
        std::fs::write(&path, garbage).unwrap();
        let app = open_cached(dir.path(), cache.path());
        let report = block_on(app.wait_until_ready()).unwrap();
        assert!(!report.from_cache);
        assert_eq!(site_urls(&app), vec!["/note/"]);
        let saved = std::fs::read(&path).unwrap();
        assert!(
            mbr_core::repo::RepoSnapshot::from_bytes(&saved).is_ok(),
            "rewritten"
        );
    }
}

#[test]
fn refresh_reports_and_applies_changes() {
    let dir = vault();
    let app = open(dir.path());
    block_on(app.wait_until_ready()).unwrap();
    assert!(block_on(app.refresh()).unwrap().is_empty());

    std::fs::write(dir.path().join("later.md"), "# Later\n").unwrap();
    std::fs::write(dir.path().join("pic.png"), "png").unwrap();
    let summary = block_on(app.refresh()).unwrap();
    assert_eq!((summary.added, summary.changed, summary.removed), (2, 0, 0));
    assert_eq!(summary.changed_pages, vec!["/later/"]);
    assert!(site_urls(&app).contains(&"/later/".to_string()));
    assert_eq!(get(&app, "/pic.png").status, 200);

    std::fs::remove_file(dir.path().join("later.md")).unwrap();
    let summary = block_on(app.refresh()).unwrap();
    assert_eq!(summary.removed, 1);
    assert!(!site_urls(&app).contains(&"/later/".to_string()));
    assert!(block_on(app.refresh()).unwrap().is_empty());
}

/// Moves a file's mtime well past now, so a rewrite within the same clock
/// tick still reads as modified.
fn filetime_bump(path: &Path) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(3600))
        .unwrap();
}

// ---- single files --------------------------------------------------------------

fn single_file_folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("note.md"),
        "# Lonely\n\n![pic](pic.png)\n\n- [ ] water plants\n\nSee [other](sibling.md).\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("pic.png"), "png").unwrap();
    std::fs::write(dir.path().join("sibling.md"), "# Sibling").unwrap();
    dir
}

#[test]
fn a_single_file_renders_alone_and_home_redirects_to_it() {
    let dir = single_file_folder();
    let coordinator = Arc::new(RecordingCoordinator::default());
    let app = MbrApp::open_file(
        dir.path().join("note.md").to_string_lossy().into_owned(),
        Some(Arc::clone(&coordinator) as Arc<dyn FileCoordinator>),
    )
    .unwrap();
    assert!(app.is_single_file());
    assert_eq!(app.home_url(), "/note/");
    block_on(app.wait_until_ready()).unwrap();

    let page = get(&app, "/note/");
    assert_eq!(page.status, 200);
    assert!(page.text().contains("Lonely"));
    assert_eq!(
        site_urls(&app),
        vec!["/note/"],
        "nothing beside it is listed"
    );
    // Relative images and neighbours are simply missing.
    assert_eq!(get(&app, "/pic.png").status, 404);
    assert_eq!(get(&app, "/sibling/").status, 404);

    let home = get(&app, "/");
    assert_eq!(home.status, 302);
    assert_eq!(home.header("location"), Some("/note/"));
    assert!(
        navigate(&app, "/")
            .text()
            .contains(r#"location.replace("/note/")"#)
    );

    // Its own tasks still toggle, through the coordinator.
    let response = request(
        &app,
        "POST",
        "/.mbr/task",
        edit_headers(),
        br#"{"path":"note.md","line":5,"expected":"- [ ] water plants","to":"done"}"#,
    );
    assert_eq!(response.status, 200, "{}", response.text());
    assert_eq!(coordinator.calls().len(), 1);
}

#[test]
fn granting_the_folder_opens_it_in_full() {
    let dir = single_file_folder();
    let file = MbrApp::open_file(
        dir.path().join("note.md").to_string_lossy().into_owned(),
        None,
    )
    .unwrap();
    block_on(file.wait_until_ready()).unwrap();
    assert_eq!(site_urls(&file), vec!["/note/"]);
    drop(file);

    let folder = open(dir.path());
    assert!(!folder.is_single_file());
    assert_eq!(folder.home_url(), "/");
    block_on(folder.wait_until_ready()).unwrap();
    assert_eq!(site_urls(&folder), vec!["/note/", "/sibling/"]);
    assert_eq!(get(&folder, "/pic.png").status, 200);
}

#[test]
fn open_file_refuses_what_is_not_a_markdown_file() {
    let dir = single_file_folder();
    let picture = dir.path().join("pic.png").to_string_lossy().into_owned();
    assert!(matches!(
        MbrApp::open_file(picture, None),
        Err(MobileError::Open { .. })
    ));
    let missing = dir.path().join("missing.md").to_string_lossy().into_owned();
    assert!(matches!(
        MbrApp::open_file(missing, None),
        Err(MobileError::Open { .. })
    ));
    assert!(MbrApp::open_file(dir.path().to_string_lossy().into_owned(), None).is_err());
}

#[test]
fn a_root_index_file_opened_alone_needs_no_redirect() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.md"), "# Front page\n").unwrap();
    let app = MbrApp::open_file(
        dir.path().join("index.md").to_string_lossy().into_owned(),
        None,
    )
    .unwrap();
    assert_eq!(app.home_url(), "/");
    block_on(app.wait_until_ready()).unwrap();
    let home = get(&app, "/");
    assert_eq!(home.status, 200);
    assert!(home.text().contains("Front page"));
}

// ---- timings -------------------------------------------------------------------

/// Cold open (no snapshot: full scan, every note's frontmatter read) against
/// warm open (snapshot restored, then a stat-diff) of a large repository, and
/// the cost of a no-change refresh. A harness, not a check:
///
/// ```text
/// MBR_TIMING_REPO=/path/to/notes cargo test --release -p mbr-ffi --features mobile \
///     -- --ignored --nocapture index_timings
/// ```
#[test]
#[ignore = "timing harness; set MBR_TIMING_REPO"]
fn index_timings() {
    let Ok(repo) = std::env::var("MBR_TIMING_REPO") else {
        return;
    };
    let cache = tempfile::tempdir().unwrap();
    let runs: usize = std::env::var("MBR_TIMING_RUNS")
        .ok()
        .and_then(|r| r.parse().ok())
        .unwrap_or(5);
    for (label, keep_cache) in [("cold", false), ("warm", true)] {
        for run in 0..runs {
            if !keep_cache {
                for entry in std::fs::read_dir(cache.path()).unwrap() {
                    std::fs::remove_file(entry.unwrap().path()).unwrap();
                }
            }
            let opened = std::time::Instant::now();
            let app = open_cached(Path::new(&repo), cache.path());
            let open_ms = opened.elapsed().as_secs_f64() * 1e3;
            let site = get(&app, "/.mbr/site.json");
            let site_ms = opened.elapsed().as_secs_f64() * 1e3;
            assert_eq!(site.status, 200);
            let report = block_on(app.wait_until_ready()).unwrap();
            let refresh_started = std::time::Instant::now();
            let summary = block_on(app.refresh()).unwrap();
            let refresh_ms = refresh_started.elapsed().as_secs_f64() * 1e3;
            assert!(summary.is_empty());
            println!(
                "TIMING {label} run {run}: open {open_ms:.1} ms, first site.json {site_ms:.1} ms \
                 ({} KB), ready {} ms, files {}, from_cache {}, no-change refresh {refresh_ms:.1} ms",
                site.body.len() / 1024,
                report.ready_ms,
                report.indexed_files,
                report.from_cache,
            );
        }
    }
    let snapshot = std::fs::read_dir(cache.path())
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum::<u64>();
    println!("TIMING snapshot size {} KB", snapshot / 1024);
}

// ---- refresh: .mbr/ ----------------------------------------------------------

fn refresh(app: &MbrApp) -> RefreshSummary {
    block_on(app.refresh()).expect("refresh")
}

/// Writes `path` so its stamp certainly differs from what was there: a stat
/// fingerprint compares size and mtime, and two writes in the same instant
/// with equal sizes would look like nothing happened.
fn rewrite(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    let later = SystemTime::now() + Duration::from_secs(2);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(later)
        .unwrap();
}

#[test]
fn a_refresh_with_nothing_changed_in_mbr_reloads_nothing() {
    let dir = vault();
    std::fs::write(dir.path().join(".mbr/user.css"), "body {}").unwrap();
    let app = open(dir.path());
    block_on(app.wait_until_ready()).unwrap();
    let before = current(&app.served);
    let summary = refresh(&app);
    assert!(!summary.mbr_changed(), "{summary:?}");
    assert!(!summary.config_reloaded && !summary.templates_reloaded && !summary.assets_changed);
    assert!(Arc::ptr_eq(&before, &current(&app.served)), "no rebuild");
}

#[test]
fn a_template_edit_is_picked_up_by_refresh_without_reopening() {
    let dir = vault();
    let app = open(dir.path());
    block_on(app.wait_until_ready()).unwrap();
    let before = current(&app.served);
    assert!(!get(&app, "/note/").text().contains("TEMPLATE-ONE"));

    rewrite(
        &dir.path().join(".mbr/_footer_custom.html"),
        "<p>TEMPLATE-ONE</p>",
    );
    let summary = refresh(&app);
    assert!(summary.templates_reloaded, "{summary:?}");
    assert!(!summary.config_reloaded);
    assert!(
        Arc::ptr_eq(&before, &current(&app.served)),
        "reloaded in place"
    );
    assert!(get(&app, "/note/").text().contains("TEMPLATE-ONE"));

    rewrite(
        &dir.path().join(".mbr/_footer_custom.html"),
        "<p>TEMPLATE-TWO-EDITED</p>",
    );
    assert!(refresh(&app).templates_reloaded);
    let page = get(&app, "/note/").text();
    assert!(page.contains("TEMPLATE-TWO-EDITED") && !page.contains("TEMPLATE-ONE"));
}

#[test]
fn an_asset_edit_needs_only_a_page_reload() {
    let dir = vault();
    std::fs::write(dir.path().join(".mbr/user.css"), "body {}").unwrap();
    let app = open(dir.path());
    block_on(app.wait_until_ready()).unwrap();
    let before = current(&app.served);
    rewrite(
        &dir.path().join(".mbr/user.css"),
        "body { color: rebeccapurple; }",
    );
    let summary = refresh(&app);
    assert!(summary.assets_changed, "{summary:?}");
    assert!(!summary.config_reloaded && !summary.templates_reloaded);
    assert!(Arc::ptr_eq(&before, &current(&app.served)));
    // Read per request: the next fetch already has it.
    assert!(get(&app, "/.mbr/user.css").text().contains("rebeccapurple"));
}

#[test]
fn a_config_edit_is_picked_up_by_refresh() {
    let dir = vault();
    std::fs::write(dir.path().join("home.md"), "# Home page\n").unwrap();
    let app = open(dir.path());
    block_on(app.wait_until_ready()).unwrap();
    assert_eq!(current(&app.served).pages.index_file, "index.md");
    assert!(!get(&app, "/note/").text().contains("<title>PFX "));

    rewrite(
        &dir.path().join(".mbr/config.toml"),
        "index_file = \"home.md\"\ntitle_prefix = \"PFX \"\n",
    );
    let summary = refresh(&app);
    assert!(summary.config_reloaded, "{summary:?}");
    assert_eq!(summary.config_error, None);
    assert_eq!(current(&app.served).pages.index_file, "home.md");
    let page = get(&app, "/note/").text();
    assert!(
        page.contains("<title>PFX "),
        "the new configuration renders"
    );
    // The index came along: the vault's notes are all still there, and the
    // new index_file makes home.md the root page.
    assert!(site_urls(&app).contains(&"/note/".to_owned()));
    assert_eq!(get(&app, "/").status, 200);
}

#[test]
fn a_config_touched_but_unchanged_does_not_rebuild() {
    let dir = vault();
    std::fs::write(
        dir.path().join(".mbr/config.toml"),
        "title_prefix = \"A \"\n",
    )
    .unwrap();
    let app = open(dir.path());
    block_on(app.wait_until_ready()).unwrap();
    let before = current(&app.served);
    rewrite(
        &dir.path().join(".mbr/config.toml"),
        "title_prefix = \"A \"\n",
    );
    let summary = refresh(&app);
    assert!(!summary.config_reloaded, "{summary:?}");
    assert!(Arc::ptr_eq(&before, &current(&app.served)));
}

#[test]
fn a_config_broken_mid_session_falls_back_and_fixing_it_clears_the_error() {
    let dir = vault();
    std::fs::write(
        dir.path().join(".mbr/config.toml"),
        "title_prefix = \"OK \"\n",
    )
    .unwrap();
    let app = open(dir.path());
    block_on(app.wait_until_ready()).unwrap();
    assert_eq!(app.config_error(), None);
    assert!(get(&app, "/note/").text().contains("<title>OK "));

    rewrite(
        &dir.path().join(".mbr/config.toml"),
        "title_prefix = \"OK \"\nthis is [not toml\n",
    );
    let broken = refresh(&app);
    assert!(broken.config_reloaded, "{broken:?}");
    let error = broken.config_error.clone().expect("reported");
    assert!(error.contains("config.toml"), "{error}");
    assert_eq!(app.config_error(), Some(error.clone()));
    assert!(
        !get(&app, "/note/").text().contains("<title>OK "),
        "the defaults are in use"
    );
    let errors = get(&app, "/note/errors.json").text();
    assert!(errors.contains("config_parse_error"), "{errors}");
    assert_eq!(get(&app, "/note/").status, 200, "still serves");

    rewrite(
        &dir.path().join(".mbr/config.toml"),
        "title_prefix = \"FIXED \"\n",
    );
    let fixed = refresh(&app);
    assert!(fixed.config_reloaded);
    assert_eq!(fixed.config_error, None);
    assert_eq!(app.config_error(), None);
    assert!(get(&app, "/note/").text().contains("<title>FIXED "));
    assert!(
        !get(&app, "/note/errors.json")
            .text()
            .contains("config_parse_error")
    );
}

#[test]
fn a_file_opened_in_place_does_not_look_at_its_folder_mbr() {
    let dir = vault();
    let file = dir.path().join("note.md");
    let app = MbrApp::open_file(file.to_string_lossy().into_owned(), None).unwrap();
    block_on(app.wait_until_ready()).unwrap();
    rewrite(
        &dir.path().join(".mbr/config.toml"),
        "title_prefix = \"NO \"\n",
    );
    assert!(!refresh(&app).mbr_changed(), "outside the grant");
}

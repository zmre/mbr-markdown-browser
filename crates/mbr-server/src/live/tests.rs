use super::*;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mbr_core::config::IpArray;
use std::path::Path;

/// A repository with one note and `.mbr/config.toml` holding `toml`.
fn repo(toml: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".mbr")).unwrap();
    std::fs::write(dir.path().join("note.md"), "# Note\n\nBody.\n").unwrap();
    write_config(dir.path(), toml);
    dir
}

/// Replaces `config.toml` the way editors save: a new file renamed over it.
fn write_config(root: &Path, toml: &str) {
    let staged = root.join(".mbr/config.toml.saving");
    std::fs::write(&staged, format!("oembed_timeout_ms = 0\n{toml}")).unwrap();
    std::fs::rename(staged, root.join(CONFIG_FILE)).unwrap();
}

/// A live server reading `root`'s configuration the way `mbr -s` does.
async fn live(root: &Path) -> Arc<LiveServer> {
    let root = root.canonicalize().unwrap();
    let reader_root = root.clone();
    let read_config: ConfigReader = Arc::new(move || Config::read_at_root(&reader_root));
    let config = read_config().expect("initial configuration");
    let live = LiveServer::init(config, false, read_config).expect("init");
    live.current().repo().wait_for_media().await;
    live
}

async fn get(router: &Router, path: &str) -> (StatusCode, String) {
    let request = Request::get(path).body(Body::empty()).unwrap();
    let response = tower::ServiceExt::oneshot(router.clone(), request)
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

async fn page(router: &Router) -> String {
    let (status, body) = get(router, "/note/").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn page_errors(router: &Router) -> String {
    let (status, body) = get(router, "/note/errors.json").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unchanged_configuration_keeps_the_server() {
    let dir = repo("title_prefix = \"A: \"\n");
    let live = live(dir.path()).await;
    let before = live.current();
    // Touched, and rewritten with the same settings in another spelling.
    write_config(dir.path(), "title_prefix = 'A: '\n");

    assert_eq!(live.reload().await, ReloadOutcome::Unchanged);
    assert!(Arc::ptr_eq(&before, &live.current()), "no rebuild");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_configuration_is_served_by_the_same_router() {
    let dir = repo("title_prefix = \"Before: \"\n");
    let live = live(dir.path()).await;
    let router = live.router();
    let before = live.current();
    assert!(page(&router).await.contains("Before: "));
    let (_, default_pico) = get(&router, "/.mbr/pico.min.css").await;

    write_config(
        dir.path(),
        "title_prefix = \"After: \"\ntheme = \"amber\"\n",
    );
    assert_eq!(live.reload().await, ReloadOutcome::Applied);

    let html = page(&router).await;
    assert!(html.contains("After: "), "new title prefix served");
    assert!(!html.contains("Before: "));
    let (_, amber_pico) = get(&router, "/.mbr/pico.min.css").await;
    assert_ne!(amber_pico, default_pico, "new theme served");
    // Same scan settings: the index was carried over, not rescanned.
    assert!(Arc::ptr_eq(before.repo(), live.current().repo()));
    assert!(before.is_retired(), "the replaced server is retired");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_scan_setting_rescans() {
    let dir = repo("");
    let live = live(dir.path()).await;
    let before = live.current();

    write_config(dir.path(), "ignore_globs = [\"*.draft.md\"]\n");
    assert_eq!(live.reload().await, ReloadOutcome::Applied);

    let after = live.current();
    assert!(!Arc::ptr_eq(before.repo(), after.repo()), "a new index");
    after.repo().wait_for_media().await;
    assert_eq!(after.repo().markdown_files.len(), 1, "and it was scanned");
    // The change channel is the same one, so open sockets carry on.
    assert!(before.file_changes().same_channel(after.file_changes()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_broken_configuration_keeps_the_last_good_one_until_fixed() {
    let dir = repo("title_prefix = \"Good: \"\n");
    let live = live(dir.path()).await;
    let router = live.router();
    assert!(!page_errors(&router).await.contains("config_parse_error"));

    write_config(dir.path(), "title_prefix = [unclosed\n");
    let outcome = live.reload().await;
    assert!(
        matches!(&outcome, ReloadOutcome::Rejected { message } if message.contains("config.toml")),
        "{outcome:?}"
    );
    assert!(outcome.pages_changed());
    assert!(page(&router).await.contains("Good: "), "last good kept");
    let errors = page_errors(&router).await;
    assert!(errors.contains("config_parse_error"), "{errors}");
    assert!(
        errors.contains("configuration it loaded before"),
        "{errors}"
    );

    // The same problem again changes nothing.
    assert_eq!(live.reload().await, ReloadOutcome::Unchanged);

    write_config(dir.path(), "title_prefix = \"Fixed: \"\n");
    assert_eq!(live.reload().await, ReloadOutcome::Applied);
    assert!(page(&router).await.contains("Fixed: "));
    assert!(!page_errors(&router).await.contains("config_parse_error"));
}

#[tokio::test(flavor = "multi_thread")]
async fn fixing_a_broken_file_back_to_the_running_settings_clears_the_error() {
    let dir = repo("title_prefix = \"Same: \"\n");
    let live = live(dir.path()).await;
    let router = live.router();
    write_config(dir.path(), "not toml at all [\n");
    assert!(matches!(
        live.reload().await,
        ReloadOutcome::Rejected { .. }
    ));
    write_config(dir.path(), "title_prefix = \"Same: \"\n");
    assert_eq!(live.reload().await, ReloadOutcome::Applied);
    assert!(!page_errors(&router).await.contains("config_parse_error"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_address_needs_a_relaunch() {
    let dir = repo("port = 5301\n");
    let live = live(dir.path()).await;
    let before = live.current();
    write_config(dir.path(), "port = 5302\nhost = \"127.0.0.2\"\n");

    assert_eq!(live.reload().await, ReloadOutcome::Unchanged);
    assert!(Arc::ptr_eq(&before, &live.current()));
    assert_eq!(live.current().port, 5301);
}

#[test]
fn relaunch_only_settings_are_put_back_and_named() {
    let running = Config::default();
    let mut next = Config {
        host: IpArray([127, 0, 0, 9]),
        port: running.port + 1,
        root_dir: PathBuf::from("/somewhere/else"),
        explicit_hidden_dirs: vec![PathBuf::from(".x")],
        theme: "amber".to_string(),
        ..Config::default()
    };
    let changed = keep_relaunch_only_settings(&mut next, &running);
    assert_eq!(changed, ["host", "port", "the repository root"]);
    assert_eq!(next.host, running.host);
    assert_eq!(next.port, running.port);
    assert_eq!(next.root_dir, running.root_dir);
    assert!(next.explicit_hidden_dirs.is_empty());
    assert_eq!(next.theme, "amber", "everything else is applied");
}

#[test]
fn only_applied_and_rejected_reload_pages() {
    assert!(ReloadOutcome::Applied.pages_changed());
    assert!(
        ReloadOutcome::Rejected {
            message: String::new()
        }
        .pages_changed()
    );
    assert!(!ReloadOutcome::Unchanged.pages_changed());
    assert!(
        !ReloadOutcome::Failed {
            message: String::new()
        }
        .pages_changed()
    );
}

#[cfg(feature = "watcher")]
mod watching {
    use super::*;
    use std::time::Duration;

    #[test]
    fn only_a_change_to_the_file_itself_counts() {
        let config_path = Path::new("/repo/.mbr/config.toml");
        let event = |path: &str, event| FileChangeEvent {
            path: path.to_owned(),
            relative_path: String::new(),
            event,
        };
        assert!(is_config_change(
            &event("/repo/.mbr/config.toml", ChangeEventType::Created),
            config_path
        ));
        assert!(is_config_change(
            &event("/repo/.mbr/config.toml", ChangeEventType::Deleted),
            config_path
        ));
        assert!(!is_config_change(
            &event("/repo/.mbr/config.toml.saving", ChangeEventType::Modified),
            config_path
        ));
        assert!(!is_config_change(
            &event("/repo/notes/.mbr/config.toml", ChangeEventType::Modified),
            config_path
        ));
        // Its own announcement must not start another reload.
        assert!(!is_config_change(
            &event("/repo/.mbr/config.toml", ChangeEventType::Reloaded),
            config_path
        ));
    }

    /// An editor saving `config.toml` reloads the configuration and tells
    /// every page, once, with no restart.
    #[tokio::test(flavor = "multi_thread")]
    async fn saving_config_toml_reloads_and_tells_pages() {
        let dir = repo("title_prefix = \"Old: \"\n");
        let live = live(dir.path()).await;
        let router = live.router();
        let mut pages = live.current().file_changes().subscribe();

        // The watcher starts on a background thread; save until it sees one.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        let mut reloads = 0;
        'saving: while tokio::time::Instant::now() < deadline {
            write_config(dir.path(), "title_prefix = \"New: \"\n");
            let wait = tokio::time::Instant::now() + Duration::from_secs(2);
            while let Ok(Ok(event)) = tokio::time::timeout_at(wait, pages.recv()).await {
                if event.event == ChangeEventType::Reloaded {
                    assert_eq!(event.relative_path, ".mbr/config.toml");
                    reloads += 1;
                    break 'saving;
                }
            }
        }
        assert_eq!(reloads, 1, "pages were told");
        assert!(page(&router).await.contains("New: "));

        // Saving the same settings again tells nobody.
        write_config(dir.path(), "title_prefix = \"New: \"\n");
        let quiet = tokio::time::Instant::now() + CONFIG_DEBOUNCE * 4;
        while let Ok(Ok(event)) = tokio::time::timeout_at(quiet, pages.recv()).await {
            assert_ne!(event.event, ChangeEventType::Reloaded, "no second reload");
        }
    }
}

use std::path::Path;

use clap::Parser;
#[cfg(feature = "gui")]
use mbr::browser::{self, BrowserContext};
use mbr::{
    Config, ConfigError, MbrError, build::Builder, cli, link_transform::LinkTransformConfig,
    markdown, server, templates,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Check if the given path requires a folder picker dialog.
/// This is true when launched as an app without a valid working directory.
#[cfg(feature = "gui")]
fn needs_folder_picker(path: &Path) -> bool {
    // Try to canonicalize, fall back to the path as-is
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    #[cfg(unix)]
    {
        // On Unix, check if path is root "/" or has only one component
        canonical.components().count() <= 1
    }

    #[cfg(windows)]
    {
        // On Windows, check for root drives or system directories
        canonical.parent().is_none()
            || canonical.starts_with(r"C:\Windows")
            || canonical.starts_with(r"C:\Program Files")
            || canonical.starts_with(r"C:\Program Files (x86)")
    }
}

#[tokio::main]
async fn main() -> Result<(), MbrError> {
    // Suppress ffmpeg warnings/info messages from the metadata crate
    // These would otherwise clutter stdout/stderr when processing video files
    #[cfg(feature = "media-metadata")]
    ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Fatal);

    let args = cli::Args::parse();

    // Initialize tracing/logging based on verbosity flags
    // Use try_init to allow server to re-configure if needed (it uses tower_http logging)
    let log_filter = args.log_level_filter();
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| log_filter.into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .try_init();

    // Generate-edit-token mode: prompt for a password (or auto-generate a
    // random token), print the token and the config line, then exit. This does
    // not need a valid path or config, so handle it before any path resolution.
    if args.generate_edit_token {
        let entered = rpassword::prompt_password(
            "Enter edit password (leave blank to auto-generate a random token): ",
        )
        .unwrap_or_default();
        let token = if entered.trim().is_empty() {
            mbr::edit_auth::generate_token()
        } else {
            entered
        };
        let hash = match mbr::edit_auth::hash_token(&token) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("Failed to hash token: {e}");
                std::process::exit(1);
            }
        };
        println!(
            "\nEditing token (give this to the client; sent as `Authorization: Bearer <token>`):\n"
        );
        println!("    {token}\n");
        println!("Add these lines to your repository's .mbr/config.toml:\n");
        println!("    edit_enabled = true");
        println!("    edit_token_hash = \"{hash}\"\n");
        println!(
            "For remote editing, put mbr behind a TLS-terminating reverse proxy — the token\n\
             is sent with every request and mbr itself serves plain HTTP.\n"
        );
        std::process::exit(0);
    }

    // Determine if we're in GUI mode (no --server, --stdout, --build, --extract-video-metadata, --extract-pdf-cover flags)
    #[cfg(all(feature = "gui", feature = "media-metadata"))]
    let is_gui_mode = !args.server
        && !args.stdout
        && !args.build
        && !args.extract_video_metadata
        && !args.extract_pdf_cover;
    #[cfg(all(feature = "gui", not(feature = "media-metadata")))]
    let is_gui_mode = !args.server && !args.stdout && !args.build;
    #[cfg(not(feature = "gui"))]
    let _is_gui_mode = false;

    // Whether a folder/file picker is needed: GUI mode with no meaningful CLI
    // path (`needs_folder_picker`). No repo config is loaded yet, so the
    // picker's file filter falls back to the compiled-in default markdown
    // extensions.
    #[cfg(feature = "gui")]
    let needs_picker = is_gui_mode && needs_folder_picker(&args.path);

    // macOS: a picker-eligible launch might really be Finder asking to open a
    // specific file ("Open With → MBR", or a plain double-click once MBR.app
    // is the default handler). That only reaches this process as a
    // `tao::event::Event::Opened`, and tao can only surface it once its event
    // loop is pumping — after this point in `main`, and after a picker shown
    // here would already be on screen. So on macOS the picker-or-not decision
    // (and the server start and window that follow from it) move into the
    // event loop itself; see `browser::InitialLaunch::Deferred` for the
    // launch-ordering guarantee this relies on. `launch_browser` does not
    // return except on setup failure, same as the ordinary GUI launch below.
    #[cfg(all(feature = "gui", target_os = "macos"))]
    if needs_picker {
        return browser::launch_browser(browser::InitialLaunch::Deferred {
            tokio_runtime: tokio::runtime::Handle::current(),
        })
        .map_err(MbrError::from);
    }

    // Every other case resolves a concrete path synchronously, exactly as
    // before Finder integration existed: a real CLI path, or (off macOS,
    // where `Event::Opened` never fires and so there is nothing to race)
    // the picker dialog shown right here.
    #[cfg(all(feature = "gui", not(target_os = "macos")))]
    let input_path = if needs_picker {
        let default_markdown_extensions = Config::default().markdown_extensions;
        match mbr::open_picker::pick_file_or_folder(
            "Select a Markdown Folder or File",
            &default_markdown_extensions,
        ) {
            Some(path) => path,
            None => {
                // User cancelled - exit gracefully
                std::process::exit(0);
            }
        }
    } else {
        args.path.clone()
    };
    // `needs_picker` was handled by the early return above.
    #[cfg(all(feature = "gui", target_os = "macos"))]
    let input_path = args.path.clone();
    #[cfg(not(feature = "gui"))]
    let input_path = args.path.clone();

    let input_path_ref = Path::new(&input_path);
    let absolute_path =
        input_path_ref
            .canonicalize()
            .map_err(|e| ConfigError::CanonicalizeFailed {
                path: input_path_ref.to_path_buf(),
                source: e,
            })?;

    let is_directory = absolute_path.is_dir();

    // Apply CLI overrides (the highest-precedence configuration layer).
    // Lives in cli::apply_overrides so the wiring is testable.
    let mut config = cli::apply_overrides(Config::read(&absolute_path)?, &args)?;

    let path_relative_to_root =
        pathdiff::diff_paths(&absolute_path, &config.root_dir).ok_or_else(|| {
            ConfigError::RelativePathFailed {
                from: config.root_dir.clone(),
                to: absolute_path.clone(),
            }
        })?;

    tracing::info!(
        "Root dir: {}; File relative to root: {}",
        &config.root_dir.display(),
        &path_relative_to_root.display()
    );

    // Extract video metadata mode - extract cover/chapters/captions from video
    #[cfg(feature = "media-metadata")]
    if args.extract_video_metadata {
        if is_directory {
            eprintln!("Error: --extract-video-metadata requires a video file, not a directory.");
            eprintln!("Usage: mbr --extract-video-metadata /path/to/video.mp4");
            std::process::exit(1);
        }

        mbr::video_metadata::extract_and_save(&absolute_path)?;
        return Ok(());
    }

    // Extract PDF cover mode - extract cover images from PDFs
    #[cfg(feature = "media-metadata")]
    if args.extract_pdf_cover {
        use mbr::pdf_metadata::{extract_pdf_covers_recursive, save_cover};

        if is_directory {
            // Recursive directory mode
            let result = extract_pdf_covers_recursive(&absolute_path, |pdf_path, sidecar_path| {
                if let Some(sidecar) = sidecar_path {
                    println!(
                        "Extracting cover: {} -> {}",
                        pdf_path.display(),
                        sidecar.display()
                    );
                }
            });

            // Report failures to stderr
            for (path, error) in &result.failures {
                eprintln!("Error: {} - {}", path.display(), error);
            }

            // Print summary
            if result.failure_count > 0 && result.success_count > 0 {
                eprintln!(
                    "\u{26a0} {} PDFs failed, {} succeeded",
                    result.failure_count, result.success_count
                );
                std::process::exit(1); // Partial failure
            } else if result.failure_count > 0 && result.success_count == 0 {
                eprintln!(
                    "\u{26a0} {} PDFs failed, none succeeded",
                    result.failure_count
                );
                std::process::exit(2); // Total failure
            } else if result.success_count > 0 {
                println!("\u{2713} Created {} cover images", result.success_count);
                std::process::exit(0); // Success
            } else {
                println!("No PDF files found in directory.");
                std::process::exit(0);
            }
        } else {
            // Single file mode
            // Verify the file has a .pdf extension
            let extension = absolute_path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase());

            if extension.as_deref() != Some("pdf") {
                eprintln!(
                    "Error: {} is not a PDF file (expected .pdf extension)",
                    absolute_path.display()
                );
                std::process::exit(2);
            }

            match save_cover(&absolute_path) {
                Ok(sidecar_path) => {
                    println!(
                        "Extracting cover: {} -> {}",
                        absolute_path.display(),
                        sidecar_path.display()
                    );
                    println!("\u{2713} Created 1 cover image");
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Error: {} - {}", absolute_path.display(), e);
                    std::process::exit(2);
                }
            }
        }
    }

    if args.build {
        // Build mode - generate static site
        // Default oembed timeout to 0 (disabled) for fastest builds unless explicitly set via CLI.
        // In tests on a 3,000 note repo, oembed=1000ms took 10 minutes vs 12 seconds with oembed=0.
        if args.oembed_timeout_ms.is_none() {
            config.oembed_timeout_ms = 0;
        }

        let output_dir = if args.output.is_absolute() {
            args.output.clone()
        } else {
            std::env::current_dir()
                .map_err(ConfigError::CurrentDirFailed)?
                .join(&args.output)
        };

        tracing::info!("Building static site to: {}", output_dir.display());

        // `--skip-link-checks` short-circuits validation entirely, so
        // `broken_links` stays 0 and the `--fail-on-broken-links` gate below
        // can never fire. Asking for both is almost always a mistake in CI —
        // the build reports success over a repository nobody checked — and
        // until now it happened silently. Warn before the build rather than
        // after, so it is visible even when the build takes a while.
        // `config.skip_link_checks` (not `args`) because the value can also
        // arrive from `.mbr/config.toml` or `MBR_SKIP_LINK_CHECKS`, which is
        // the easier case to miss.
        if config.skip_link_checks && args.fail_on_broken_links {
            eprintln!(
                "Warning: --fail-on-broken-links has no effect because link checking is skipped; \
                 this build cannot fail on broken links. Drop --skip-link-checks (or the \
                 skip_link_checks config/MBR_SKIP_LINK_CHECKS setting) to enable the check."
            );
        }

        let builder = Builder::new(config, output_dir)?;
        let stats = builder.build().await?;

        if stats.broken_links > 0 {
            println!(
                "Build complete: {} markdown pages, {} section pages, {} assets linked, {} broken links in {:?}",
                stats.markdown_pages,
                stats.section_pages,
                stats.assets_linked,
                stats.broken_links,
                stats.duration
            );
        } else {
            println!(
                "Build complete: {} markdown pages, {} section pages, {} assets linked in {:?}",
                stats.markdown_pages, stats.section_pages, stats.assets_linked, stats.duration
            );
        }
        if args.fail_on_broken_links && stats.broken_links > 0 {
            eprintln!(
                "Error: {} broken internal link(s) detected; failing because --fail-on-broken-links was set.",
                stats.broken_links
            );
            std::process::exit(1);
        }
        return Ok(());
    } else if args.stdout {
        // CLI mode - render markdown to stdout (explicit -o/--stdout flag)
        if is_directory {
            eprintln!(
                "Cannot render a directory to stdout. Use -s to start a server or omit -o for GUI mode."
            );
            eprintln!("  mbr -s {}  # Start server", input_path.display());
            eprintln!("  mbr {}     # Open in GUI (default)", input_path.display());
            std::process::exit(1);
        }

        // Determine if this is an index file (which doesn't need ../ prefix for links)
        let is_index_file = input_path
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(|f| f == config.index_file);

        let link_transform_config = LinkTransformConfig {
            markdown_extensions: config.markdown_extensions.clone(),
            index_file: config.index_file.clone(),
            is_index_file,
            url_depth: None,
            // CLI stdout mode renders a single file with no repo index, so
            // body wikilinks never resolve globally; the page URL is unused.
            current_page_url: String::new(),
            markdown_page_probe: None,
        };

        // CLI mode: server_mode=false, transcode disabled (transcode is server-only).
        // Stdout/CLI mode mirrors build defaults (off unless explicitly enabled).
        let valid_tag_sources = mbr::config::tag_sources_to_set(&config.tag_sources);
        let mark_incomplete = config.mark_incomplete.unwrap_or(false);
        let render_result = markdown::render(
            input_path,
            config.root_dir.as_path(),
            config.oembed_timeout_ms,
            link_transform_config,
            false, // server_mode is false in CLI mode
            false, // transcode is disabled in CLI mode
            valid_tag_sources,
            // Hardcoded off: CLI mode writes HTML to stdout, with no page and no
            // server for a review anchor to reach.
            markdown::ReviewLines::Omit,
            mark_incomplete,
            &config.incomplete_markers,
            None, // no repo wikilink index in CLI stdout mode
        )
        .await
        .inspect_err(|e| tracing::error!("Error rendering markdown: {:?}", e))?;
        let templates =
            templates::Templates::new(&config.root_dir, config.template_folder.as_deref())
                .inspect_err(|e| tracing::error!("Error parsing template: {e}"))?;
        let html_output = templates.render_markdown(
            &render_result.html,
            render_result.frontmatter,
            std::collections::HashMap::new(),
        )?;
        println!("{}", html_output);
    } else if args.server {
        // Server mode - HTTP server only, no GUI
        let server_config = server::ServerConfig::from(&config).with_gui_mode(false);
        let server = server::Server::init(server_config)?;

        let url_path = mbr::launch_url::build_url_path(
            &path_relative_to_root,
            is_directory,
            &config.markdown_extensions,
        );
        warn_if_non_loopback_bind(&config.host);
        tracing::info!(
            "Server running at http://{}:{}/{}",
            config.host,
            config.port,
            url_path
        );

        server.start().await?;
    } else {
        // GUI mode - default when no flags specified (or explicit -g)
        #[cfg(feature = "gui")]
        {
            warn_if_non_loopback_bind(&config.host);
            let config_copy = config.clone();
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<u16>();
            let handle = tokio::spawn(async move {
                let server_config = server::ServerConfig::from(&config_copy).with_gui_mode(true);
                let server = server::Server::init(server_config);
                match server {
                    Ok(mut s) => {
                        // Try up to 10 port increments if address is in use
                        if let Err(e) = s.start_with_port_retry(Some(ready_tx), 10).await {
                            tracing::error!("Server error: {e}");
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            "Couldn't initialize the server: {e}. Try with -s for more info"
                        );
                        // Drop the sender to signal failure
                        drop(ready_tx);
                    }
                }
            });

            // Wait for server to be ready before opening browser
            let actual_port = match ready_rx.await {
                Ok(port) => port,
                Err(_) => {
                    tracing::error!("Server failed to start");
                    return Ok(());
                }
            };

            let base_url =
                url::Url::parse(format!("http://{}:{}/", config.host, actual_port).as_str())?;

            // Directory listing, media viewer or markdown page, whichever
            // `absolute_path` names — see `mbr::launch_url` for the branch.
            let url_path = mbr::launch_url::resolve_launch_url_path(
                &path_relative_to_root,
                is_directory,
                &config.markdown_extensions,
            );
            let url = base_url.join(&url_path)?;

            // Launch browser with full context for server management
            let ctx = BrowserContext {
                url: url.to_string(),
                server_handle: handle,
                config,
                tokio_runtime: tokio::runtime::Handle::current(),
            };

            browser::launch_browser(browser::InitialLaunch::Known(Box::new(ctx)))?;
            // Note: server handle is now managed by the browser context
            // It will be aborted when the browser window closes or when switching folders
        }
        #[cfg(not(feature = "gui"))]
        {
            // GUI mode not available - this shouldn't happen since is_gui_mode is always false
            // when the gui feature is disabled, but provide a clear error just in case
            tracing::error!(
                "GUI mode is not available in this build. Use -s for server mode or --stdout for stdout mode."
            );
            std::process::exit(1);
        }
    }
    Ok(())
}

/// Returns true if the given IPv4 octets represent a loopback address (127.0.0.0/8).
fn is_loopback_host(octets: [u8; 4]) -> bool {
    std::net::Ipv4Addr::from(octets).is_loopback()
}

/// Logs a security warning when the server is configured to bind to a
/// non-loopback address, since mbr has no authentication.
fn warn_if_non_loopback_bind(host: &mbr::config::IpArray) {
    if !is_loopback_host(host.0) {
        tracing::warn!(
            "Binding to {host} exposes this server beyond localhost: the entire markdown repository is readable by anyone who can reach this address, with no authentication, and expensive operations (search, video transcoding, PDF extraction) can be triggered remotely. Use --host 127.0.0.1 unless this is intended."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "gui")]
    use std::path::Path;

    #[test]
    fn test_is_loopback_host_loopback_addresses() {
        assert!(is_loopback_host([127, 0, 0, 1]));
        assert!(is_loopback_host([127, 1, 2, 3]));
    }

    #[test]
    fn test_is_loopback_host_non_loopback_addresses() {
        assert!(!is_loopback_host([0, 0, 0, 0]));
        assert!(!is_loopback_host([192, 168, 1, 10]));
        assert!(!is_loopback_host([10, 0, 0, 1]));
    }

    #[test]
    #[cfg(feature = "gui")]
    fn test_needs_folder_picker_root() {
        assert!(needs_folder_picker(Path::new("/")));
    }

    #[test]
    #[cfg(feature = "gui")]
    fn test_needs_folder_picker_normal_path() {
        // A normal path like /Users/foo should not need folder picker
        assert!(!needs_folder_picker(Path::new("/Users/foo")));
    }

    #[test]
    #[cfg(feature = "gui")]
    fn test_needs_folder_picker_current_dir() {
        // Current directory "." should not need folder picker when it resolves to a real path
        // This test depends on where it's run from
        let cwd = std::env::current_dir().unwrap();
        if cwd.components().count() > 1 {
            assert!(!needs_folder_picker(&cwd));
        }
    }
}

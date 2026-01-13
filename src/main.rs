use std::path::Path;
#[cfg(feature = "gui")]
use std::path::PathBuf;

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

/// Show a folder picker dialog and return the selected path.
/// Returns None if the user cancels.
#[cfg(feature = "gui")]
fn show_folder_picker() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Select Markdown Folder")
        .pick_folder()
}

#[tokio::main]
async fn main() -> Result<(), MbrError> {
    // Suppress ffmpeg warnings/info messages from the metadata crate
    // These would otherwise clutter stdout/stderr when processing video files
    #[cfg(feature = "media-metadata")]
    ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Error);

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

    // Determine if we're in GUI mode (no --server, --stdout, --build flags)
    #[cfg(feature = "gui")]
    let is_gui_mode = !args.server && !args.stdout && !args.build;
    #[cfg(not(feature = "gui"))]
    let _is_gui_mode = false;

    // Check if we need to show a folder picker (only in GUI mode when path is root/system dir)
    #[cfg(feature = "gui")]
    let input_path = if is_gui_mode && needs_folder_picker(&args.path) {
        match show_folder_picker() {
            Some(path) => path,
            None => {
                // User cancelled - exit gracefully
                std::process::exit(0);
            }
        }
    } else {
        args.path.clone()
    };
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

    let mut config = Config::read(&absolute_path)?;

    // Apply CLI overrides
    if let Some(timeout) = args.oembed_timeout_ms {
        config.oembed_timeout_ms = timeout;
    }
    if let Some(cache_size) = args.oembed_cache_size {
        config.oembed_cache_size = cache_size;
    }
    if let Some(ref template_folder) = args.template_folder {
        // Canonicalize and validate the template folder path
        let template_path =
            template_folder
                .canonicalize()
                .map_err(|e| ConfigError::CanonicalizeFailed {
                    path: template_folder.clone(),
                    source: e,
                })?;
        if !template_path.is_dir() {
            return Err(ConfigError::TemplateFolderNotDirectory {
                path: template_path,
            }
            .into());
        }
        config.template_folder = Some(template_path);
    }
    if let Some(port) = args.port {
        config.port = port;
    }
    if let Some(ref host) = args.host {
        let ip: std::net::IpAddr = host
            .parse()
            .map_err(|_| ConfigError::InvalidHost { host: host.clone() })?;
        match ip {
            std::net::IpAddr::V4(v4) => {
                config.ip = mbr::config::IpArray(v4.octets());
            }
            std::net::IpAddr::V6(_) => {
                return Err(ConfigError::InvalidHost { host: host.clone() }.into());
            }
        }
    }
    if let Some(ref theme) = args.theme {
        config.theme = theme.clone();
    }

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

    if args.build {
        // Build mode - generate static site
        #[cfg(target_os = "windows")]
        {
            eprintln!("Error: Static site generation is not supported on Windows");
            std::process::exit(1);
        }

        #[cfg(not(target_os = "windows"))]
        {
            let output_dir = if args.output.is_absolute() {
                args.output.clone()
            } else {
                std::env::current_dir()
                    .map_err(ConfigError::CurrentDirFailed)?
                    .join(&args.output)
            };

            tracing::info!("Building static site to: {}", output_dir.display());

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
            return Ok(());
        }
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
        };

        let (frontmatter, _headings, html_output) = markdown::render(
            input_path,
            config.root_dir.as_path(),
            config.oembed_timeout_ms,
            link_transform_config,
        )
        .await
        .inspect_err(|e| tracing::error!("Error rendering markdown: {:?}", e))?;
        let templates =
            templates::Templates::new(&config.root_dir, config.template_folder.as_deref())
                .inspect_err(|e| tracing::error!("Error parsing template: {e}"))?;
        let html_output = templates
            .render_markdown(&html_output, frontmatter, std::collections::HashMap::new())
            .await?;
        println!("{}", &html_output);
    } else if args.server {
        // Server mode - HTTP server only, no GUI
        let server = server::Server::init(
            config.ip.0,
            config.port,
            &config.root_dir,
            &config.static_folder,
            &config.markdown_extensions,
            &config.ignore_dirs,
            &config.ignore_globs,
            &config.watcher_ignore_dirs,
            &config.index_file.clone(),
            config.oembed_timeout_ms,
            config.oembed_cache_size,
            config.template_folder.clone(),
            config.sort.clone(),
            false, // gui_mode: browser access, not native window
            &config.theme,
            None, // Logging already initialized
        )?;

        let url_path = build_url_path(
            &path_relative_to_root,
            is_directory,
            &config.markdown_extensions,
        );
        tracing::info!(
            "Server running at http://{}:{}/{}",
            config.ip,
            config.port,
            url_path
        );

        server.start().await?;
    } else {
        // GUI mode - default when no flags specified (or explicit -g)
        #[cfg(feature = "gui")]
        {
            let config_copy = config.clone();
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<u16>();
            let handle = tokio::spawn(async move {
                let server = server::Server::init(
                    config_copy.ip.0,
                    config_copy.port,
                    config_copy.root_dir.clone(),
                    &config_copy.static_folder,
                    &config_copy.markdown_extensions.clone(),
                    &config_copy.ignore_dirs.clone(),
                    &config_copy.ignore_globs.clone(),
                    &config_copy.watcher_ignore_dirs.clone(),
                    &config_copy.index_file.clone(),
                    config_copy.oembed_timeout_ms,
                    config_copy.oembed_cache_size,
                    config_copy.template_folder.clone(),
                    config_copy.sort.clone(),
                    true, // gui_mode: native window mode
                    &config_copy.theme,
                    None, // Logging already initialized
                );
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

            let url = url::Url::parse(format!("http://{}:{}/", config.ip, actual_port).as_str())?;
            let url_path = build_url_path(
                &path_relative_to_root,
                is_directory,
                &config.markdown_extensions,
            );
            let url = url.join(&url_path)?;

            // Launch browser with full context for server management
            let ctx = BrowserContext {
                url: url.to_string(),
                server_handle: handle,
                config,
                tokio_runtime: tokio::runtime::Handle::current(),
            };

            browser::launch_browser(ctx)?;
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

/// Builds a URL path from a relative filesystem path.
///
/// - For directories: returns the path with a trailing slash
/// - For markdown files: replaces the extension with a trailing slash
/// - For other files: returns the path as-is
pub fn build_url_path(
    relative_path: &std::path::Path,
    is_directory: bool,
    markdown_extensions: &[String],
) -> String {
    let relative_str = relative_path.to_str().unwrap_or_default();

    if is_directory {
        if relative_str.is_empty() {
            String::new()
        } else {
            format!("{}/", relative_str)
        }
    } else {
        replace_markdown_extension_with_slash(relative_str, markdown_extensions)
    }
}

fn replace_markdown_extension_with_slash(s: &str, extensions: &[String]) -> String {
    if let Some((base, extension)) = s.rsplit_once('.') {
        match extensions
            .iter()
            .find(|cur_ext| extension == cur_ext.as_str())
        {
            Some(_) => format!("{}/", base), // one of the sought extensions is there, replace with a "/"
            None => s.to_string(), // no sought extensions found, just return input as provided
        }
    } else {
        s.to_string() // no extension, so return input as provided
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_build_url_path_root_directory() {
        let path = Path::new("");
        let extensions = vec!["md".to_string()];
        assert_eq!(build_url_path(path, true, &extensions), "");
    }

    #[test]
    fn test_build_url_path_subdirectory() {
        let path = Path::new("docs/api");
        let extensions = vec!["md".to_string()];
        assert_eq!(build_url_path(path, true, &extensions), "docs/api/");
    }

    #[test]
    fn test_build_url_path_markdown_file() {
        let path = Path::new("readme.md");
        let extensions = vec!["md".to_string()];
        assert_eq!(build_url_path(path, false, &extensions), "readme/");
    }

    #[test]
    fn test_build_url_path_markdown_file_in_subdir() {
        let path = Path::new("docs/guide.md");
        let extensions = vec!["md".to_string()];
        assert_eq!(build_url_path(path, false, &extensions), "docs/guide/");
    }

    #[test]
    fn test_build_url_path_alternate_extension() {
        let path = Path::new("notes.markdown");
        let extensions = vec!["md".to_string(), "markdown".to_string()];
        assert_eq!(build_url_path(path, false, &extensions), "notes/");
    }

    #[test]
    fn test_build_url_path_non_markdown_file() {
        let path = Path::new("image.png");
        let extensions = vec!["md".to_string()];
        assert_eq!(build_url_path(path, false, &extensions), "image.png");
    }

    #[test]
    fn test_replace_markdown_extension_with_slash() {
        let extensions = ["md".to_string()];
        assert_eq!(
            replace_markdown_extension_with_slash("test.md", &extensions),
            "test/"
        );
        assert_eq!(
            replace_markdown_extension_with_slash("test.txt", &extensions),
            "test.txt"
        );
        assert_eq!(
            replace_markdown_extension_with_slash("noext", &extensions),
            "noext"
        );
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

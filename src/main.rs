use std::path::Path;

use clap::Parser;
use mbr::{
    browser, build::Builder, cli, link_transform::LinkTransformConfig, markdown, server, templates,
    Config, ConfigError, MbrError,
};

#[tokio::main]
async fn main() -> Result<(), MbrError> {
    // Suppress ffmpeg warnings/info messages from the metadata crate
    // These would otherwise clutter stdout/stderr when processing video files
    ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Error);

    let args = cli::Args::parse();

    let input_path = Path::new(&args.path);
    let absolute_path = input_path.canonicalize().map_err(|e| {
        ConfigError::CanonicalizeFailed {
            path: input_path.to_path_buf(),
            source: e,
        }
    })?;

    let is_directory = absolute_path.is_dir();

    let mut config = Config::read(&absolute_path)?;

    // Apply CLI overrides
    if let Some(timeout) = args.oembed_timeout {
        config.oembed_timeout_ms = timeout;
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
                    .map_err(|e| ConfigError::CurrentDirFailed(e))?
                    .join(&args.output)
            };

            tracing::info!("Building static site to: {}", output_dir.display());

            let builder = Builder::new(config, output_dir)?;
            let stats = builder.build().await?;

            if stats.broken_links > 0 {
                println!(
                    "Build complete: {} markdown pages, {} section pages, {} assets linked, {} broken links in {:?}",
                    stats.markdown_pages, stats.section_pages, stats.assets_linked, stats.broken_links, stats.duration
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
            eprintln!("  mbr -s {}  # Start server", args.path.display());
            eprintln!("  mbr {}     # Open in GUI (default)", args.path.display());
            std::process::exit(1);
        }

        // Determine if this is an index file (which doesn't need ../ prefix for links)
        let is_index_file = args
            .path
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(|f| f == config.index_file);

        let link_transform_config = LinkTransformConfig {
            markdown_extensions: config.markdown_extensions.clone(),
            index_file: config.index_file.clone(),
            is_index_file,
        };

        let (frontmatter, html_output) = markdown::render(
            args.path,
            config.root_dir.as_path(),
            config.oembed_timeout_ms,
            link_transform_config,
        )
        .await
        .inspect_err(|e| tracing::error!("Error rendering markdown: {:?}", e))?;
        let templates = templates::Templates::new(&config.root_dir)
            .inspect_err(|e| tracing::error!("Error parsing template: {e}"))?;
        let html_output = templates.render_markdown(&html_output, frontmatter).await?;
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
            &config.index_file.clone(),
            config.oembed_timeout_ms,
        )?;

        let url_path = build_url_path(&path_relative_to_root, is_directory, &config.markdown_extensions);
        tracing::info!("Server running at http://{}:{}/{}", config.ip, config.port, url_path);

        server.start().await?;
    } else {
        // GUI mode - default when no flags specified (or explicit -g)
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
                &config_copy.index_file.clone(),
                config_copy.oembed_timeout_ms,
            );
            match server {
                Ok(mut s) => {
                    // Try up to 10 port increments if address is in use
                    if let Err(e) = s.start_with_port_retry(Some(ready_tx), 10).await {
                        tracing::error!("Server error: {e}");
                    }
                }
                Err(e) => {
                    tracing::error!("Couldn't initialize the server: {e}. Try with -s for more info");
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
        let url_path = build_url_path(&path_relative_to_root, is_directory, &config.markdown_extensions);
        let url = url.join(&url_path)?;

        browser::launch_url(url.as_str())?;
        handle.abort(); // after the browser window quits, we can exit the http server
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
        assert_eq!(
            replace_markdown_extension_with_slash("test.md", &vec!["md".to_string()]),
            "test/"
        );
        assert_eq!(
            replace_markdown_extension_with_slash("test.txt", &vec!["md".to_string()]),
            "test.txt"
        );
        assert_eq!(
            replace_markdown_extension_with_slash("noext", &vec!["md".to_string()]),
            "noext"
        );
    }
}

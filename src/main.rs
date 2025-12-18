use std::path::Path;

use clap::Parser;
use config::Config;

mod browser;

mod cli; // handle command line params
mod config; // handle config file and env stuff
mod errors; // centralized error types
mod html; // turn markdown into html; TODO: remove this? don't think I actually need it custom
mod markdown; // parse and process markdown
mod oembed; // handling for bare links in markdown to make auto-embeds
mod repo; // process a folder of files for navigation (and search?) purposes
mod server; // serve up local files live
mod templates; // product html wrapper
mod vid; // manage video references and html gen // process files over the whole root

pub use errors::{ConfigError, MbrError};
         // TOOD: mod static; // generate static files to be deployed -- should this somehow work in tandem with server or be a mode thereof?

#[tokio::main]
async fn main() -> Result<(), MbrError> {
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

    println!(
        "Root dir: {}; File relative to root: {}",
        &config.root_dir.display(),
        &path_relative_to_root.display()
    );

    if args.gui {
        let config_copy = config.clone();
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
                Ok(s) => {
                    if let Err(e) = s.start().await {
                        eprintln!("Server error: {e}");
                    }
                }
                Err(e) => eprintln!("Couldn't initialize the server: {e}. Try with -s for more info"),
            }
        });
        // Give the server a moment to start listening before opening the browser
        // TODO: find a better way to know when server is ready
        std::thread::sleep(std::time::Duration::from_millis(100));
        let url = url::Url::parse(format!("http://{}:{}/", config.ip, config.port,).as_str())?;

        let relative_str = path_relative_to_root.to_str().unwrap_or_default();
        let url_path = if is_directory {
            // For directories, ensure trailing slash
            if relative_str.is_empty() {
                String::new()
            } else {
                format!("{}/", relative_str)
            }
        } else {
            replace_markdown_extension_with_slash(relative_str, &config.markdown_extensions)
        };
        let url = url.join(&url_path)?;

        browser::launch_url(url.as_str())?;
        handle.abort(); // after the browser window quits, we can exit the http server
    } else if args.server {
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

        let relative_str = path_relative_to_root.to_str().unwrap_or_default();
        let url_path = if is_directory {
            if relative_str.is_empty() {
                String::new()
            } else {
                format!("{}/", relative_str)
            }
        } else {
            replace_markdown_extension_with_slash(relative_str, &config.markdown_extensions)
        };
        println!("http://{}:{}/{}", config.ip, config.port, url_path);

        server.start().await?;
    } else if is_directory {
        // CLI mode with directory - can't render a directory to stdout, suggest using -s
        eprintln!(
            "Cannot render a directory to stdout. Use -s to start a server or -g for GUI mode."
        );
        eprintln!("  mbr -s {}  # Start server", args.path.display());
        eprintln!("  mbr -g {}  # Open in GUI", args.path.display());
        std::process::exit(1);
    } else {
        // CLI mode with file - render markdown to stdout
        let (frontmatter, html_output) =
            markdown::render(args.path, config.root_dir.as_path(), config.oembed_timeout_ms)
                .await
                .inspect_err(|e| eprintln!("Error rendering markdown: {:?}", e))?;
        let templates = templates::Templates::new(&config.root_dir)
            .inspect_err(|e| eprintln!("Error parsing template: {e}"))?;
        let html_output = templates.render_markdown(&html_output, frontmatter).await?;
        println!("{}", &html_output);
    }
    Ok(())
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

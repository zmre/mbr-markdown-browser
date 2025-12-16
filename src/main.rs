use std::path::Path;

use clap::Parser;
use config::Config;

mod browser; // this could be called (should be?) GUI as it pops up a local browser window
mod cli; // handle command line params
mod config; // handle config file and env stuff
mod html; // turn markdown into html; TODO: remove this? don't think I actually need it custom
mod markdown; // parse and process markdown
mod oembed; // handling for bare links in markdown to make auto-embeds
mod repo; // process a folder of files for navigation (and search?) purposes
mod server; // serve up local files live
mod templates; // product html wrapper
mod vid; // manage video references and html gen // process files over the whole root
         // TOOD: mod static; // generate static files to be deployed -- should this somehow work in tandem with server or be a mode thereof?

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = cli::Args::parse();

    let file_absolute_path = Path::new(&args.file)
        .canonicalize()
        .expect("Failed to canonicalize path for provided folder");

    let config = Config::read(&file_absolute_path)?;

    let file_relative_to_root = pathdiff::diff_paths(file_absolute_path, &config.root_dir)
        .expect("Failed to calculate diff between CWD and file");

    println!(
        "Root dir: {}; File relative to root: {}",
        &config.root_dir.display(),
        &file_relative_to_root.display()
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
            );
            server
                .expect("Couldn't initialize the server. Try with -s for more info")
                .start()
                .await;
        });
        // Give the server a moment to start listening before opening the browser
        // TODO: find a better way to know when server is ready
        std::thread::sleep(std::time::Duration::from_millis(100));
        let url = url::Url::parse(format!("http://{}:{}/", config.ip, config.port,).as_str())?;

        let url = url.join(
            replace_markdown_extension_with_slash(
                file_relative_to_root.to_str().unwrap(),
                &config.markdown_extensions,
            )
            .as_str(),
        )?;

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
        )?;
        println!(
            "http://{}:{}/{}",
            config.ip,
            config.port,
            replace_markdown_extension_with_slash(
                file_relative_to_root.to_str().unwrap(),
                &config.markdown_extensions
            )
        );

        server.start().await;
    } else {
        let (frontmatter, html_output) = markdown::render(args.file, &config.root_dir.as_path())
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

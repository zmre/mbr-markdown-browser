use std::path::Path;

use clap::Parser;
use config::Config;

mod browser;
mod cli;
mod config;
mod markdown;
mod oembed;
mod server;
mod templates;

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
                &config_copy.index_file.clone(),
            );
            server
                .expect("Couldn't initialize the server. Try with -s for more info")
                .start()
                .await;
        });
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
        let html_output = markdown::render(args.file).await?;
        let templates = templates::Templates::new(&config.root_dir)?;
        let html_output = templates.render_markdown(&html_output).await?;
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

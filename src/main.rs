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
        .expect("Failed to canonicalize path");

    let config = Config::read(&file_absolute_path)?;

    let file_relative_to_root = pathdiff::diff_paths(file_absolute_path, &config.root_dir)
        .expect("Failed to calculate diff between CWD and file");

    println!(
        "Root dir: {}; File relative to root: {}",
        &config.root_dir.display(),
        &file_relative_to_root.display()
    );

    if args.gui {
        let handle = tokio::spawn(async move {
            let server = server::Server::init(config.ip.0, config.port);
            server.start().await;
        });
        let url = url::Url::parse(format!("http://{}:{}/", config.ip, config.port,).as_str())?;

        let url = url.join(
            file_relative_to_root
                .to_str()
                .unwrap()
                .replace(".md", "/")
                .as_str(),
        )?;

        browser::Gui::launch_url(url.as_str());
        handle.abort(); // after the browser window quits, we can exit the server
    } else if args.server {
        let server = server::Server::init(config.ip.0, config.port);
        println!(
            "http://{}:{}/{}",
            config.ip,
            config.port,
            file_relative_to_root
                .display()
                .to_string()
                .replace(".md", "/")
        );

        server.start().await;
    } else {
        let html_output = markdown::render(args.file).await?;
        let templates = templates::Templates::new();
        let html_output = templates.render_markdown(&html_output).await?;
        println!("{}", &html_output);
    }
    Ok(())
}

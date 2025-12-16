use clap::Parser;
use std::path::PathBuf;

/// Simple program to render a markdown file
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Launch GUI (otherwise print html to STDOUT)
    #[arg(short, long, conflicts_with = "server")]
    pub gui: bool,

    /// Launch server
    #[arg(short, long, conflicts_with = "gui")]
    pub server: bool,

    /// Markdown file to render
    //#[clap(short, long, default_value = "index.md")]
    pub file: PathBuf,

    /// Timeout in milliseconds for fetching oembed/OpenGraph metadata from URLs.
    /// Falls back to plain link if fetch doesn't complete in time.
    #[arg(long)]
    pub oembed_timeout: Option<u64>,
}

use clap::Parser;
use std::path::PathBuf;

/// Markdown browser and previewer
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Launch GUI window (default if no mode specified)
    #[arg(short, long, conflicts_with_all = ["server", "stdout"])]
    pub gui: bool,

    /// Launch HTTP server only (no GUI)
    #[arg(short, long, conflicts_with_all = ["gui", "stdout"])]
    pub server: bool,

    /// Render single markdown file to stdout (CLI mode)
    #[arg(short = 'o', long, conflicts_with_all = ["gui", "server"])]
    pub stdout: bool,

    /// Markdown file or folder to serve (defaults to current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Timeout in milliseconds for fetching oembed/OpenGraph metadata from URLs.
    /// Falls back to plain link if fetch doesn't complete in time.
    #[arg(long)]
    pub oembed_timeout: Option<u64>,
}

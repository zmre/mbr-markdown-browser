use clap::Parser;
use std::path::PathBuf;

/// Markdown browser and previewer
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Launch GUI window (default if no mode specified)
    #[arg(short, long, conflicts_with_all = ["server", "stdout", "build"])]
    pub gui: bool,

    /// Launch HTTP server only (no GUI)
    #[arg(short, long, conflicts_with_all = ["gui", "stdout", "build"])]
    pub server: bool,

    /// Render single markdown file to stdout (CLI mode)
    #[arg(short = 'o', long, conflicts_with_all = ["gui", "server", "build"])]
    pub stdout: bool,

    /// Build static site (generate HTML for all markdown files)
    #[arg(short, long, conflicts_with_all = ["gui", "server", "stdout"])]
    pub build: bool,

    /// Output directory for static site build (default: "build")
    #[arg(long, default_value = "build")]
    pub output: PathBuf,

    /// Markdown file or folder to serve (defaults to current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Timeout in milliseconds for fetching oembed/OpenGraph metadata from URLs.
    /// Falls back to plain link if fetch doesn't complete in time.
    #[arg(long)]
    pub oembed_timeout: Option<u64>,
}

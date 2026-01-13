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
    /// Set to 0 to disable oembed fetching entirely (uses plain links).
    #[arg(long)]
    pub oembed_timeout_ms: Option<u64>,

    /// Maximum size in bytes for the oembed cache. The cache stores fetched page
    /// metadata to avoid redundant network requests. Set to 0 to disable caching.
    /// Default: 2097152 (2MB). Accepts human-readable sizes like "2MB" or "512KB".
    #[arg(long)]
    pub oembed_cache_size: Option<usize>,

    /// Override template folder (replaces default .mbr/ and compiled defaults).
    /// Files found in this folder take precedence; missing files fall back to defaults.
    #[arg(long, value_name = "PATH")]
    pub template_folder: Option<PathBuf>,

    /// Increase logging verbosity (-v = info, -vv = debug, -vvv = trace).
    /// Default is warn level. Can also set RUST_LOG env var.
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Suppress all output except errors
    #[arg(short, long)]
    pub quiet: bool,

    /// Port to listen on when running in server mode (-s).
    /// Overrides the default port from config (default: 5200).
    #[arg(short = 'p', long, value_name = "PORT")]
    pub port: Option<u16>,

    /// Host/IP address to bind to when running in server mode (-s).
    /// Overrides the default from config (default: 127.0.0.1).
    /// Use 0.0.0.0 to listen on all interfaces.
    #[arg(long, value_name = "HOST")]
    pub host: Option<String>,

    /// Pico CSS theme to use. Overrides config file setting.
    /// Options: default, fluid, or a color name (amber, blue, cyan, fuchsia, green,
    /// grey, indigo, jade, lime, orange, pink, pumpkin, purple, red, sand, slate,
    /// violet, yellow, zinc). Prefix with "fluid." for fluid typography (e.g., fluid.amber).
    #[arg(long, value_name = "THEME")]
    pub theme: Option<String>,
}

impl Args {
    /// Get the log level filter string based on verbosity flags.
    /// Returns a filter suitable for tracing_subscriber::EnvFilter.
    pub fn log_level_filter(&self) -> String {
        let level = if self.quiet {
            "error"
        } else {
            match self.verbose {
                0 => "warn",
                1 => "info",
                2 => "debug",
                _ => "trace",
            }
        };

        // Set level for mbr crate and tower_http (for request logging)
        format!(
            "{}={},tower_http={}",
            env!("CARGO_CRATE_NAME"),
            level,
            level
        )
    }
}

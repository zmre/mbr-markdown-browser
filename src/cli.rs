use clap::Parser;
use std::path::PathBuf;

/// Markdown browser and previewer
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Launch GUI window (default if no mode specified)
    #[arg(short, long, conflicts_with_all = ["server", "stdout", "build", "extract_video_metadata"])]
    pub gui: bool,

    /// Launch HTTP server only (no GUI)
    #[arg(short, long, conflicts_with_all = ["gui", "stdout", "build", "extract_video_metadata"])]
    pub server: bool,

    /// Render single markdown file to stdout (CLI mode)
    #[arg(short = 'o', long, conflicts_with_all = ["gui", "server", "build", "extract_video_metadata"])]
    pub stdout: bool,

    /// Build static site (generate HTML for all markdown files)
    #[arg(short, long, conflicts_with_all = ["gui", "server", "stdout", "extract_video_metadata"])]
    pub build: bool,

    /// Extract video metadata (cover, chapters, captions) and save as sidecar files.
    /// Takes a video file path and generates .cover.png, .chapters.en.vtt, and
    /// .captions.en.vtt files next to it (if the video contains this data).
    #[cfg(feature = "media-metadata")]
    #[arg(long, conflicts_with_all = ["gui", "server", "stdout", "build"])]
    pub extract_video_metadata: bool,

    /// Output directory for static site build (default: "build")
    #[arg(long, default_value = "build")]
    pub output: PathBuf,

    /// Markdown file or folder to serve (defaults to current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Timeout in milliseconds for fetching oembed/OpenGraph metadata from URLs.
    /// Falls back to plain link if fetch doesn't complete in time.
    /// Set to 0 to disable oembed fetching entirely (uses plain links).
    /// Default: 500ms for server/GUI mode, 0 (disabled) for build mode.
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

    /// Number of files to process concurrently during static build (-b).
    /// Higher values use more memory but may be faster on multi-core systems.
    /// Default: auto (2x CPU cores, max 32).
    #[arg(long, value_name = "N")]
    pub build_concurrency: Option<usize>,

    /// Skip internal link validation during static build (-b).
    /// Useful for faster builds when you don't need link checking.
    #[arg(long)]
    pub skip_link_checks: bool,

    /// Disable bidirectional link tracking (backlinks).
    /// When disabled, the links.json endpoint returns 404 and no links.json files
    /// are generated during static builds.
    #[arg(long)]
    pub no_link_tracking: bool,

    /// [EXPERIMENTAL] Enable dynamic video transcoding to serve lower-resolution
    /// HLS variants (720p, 480p) for bandwidth savings. Only active in server/GUI mode.
    /// Videos are transcoded on-demand as segments and cached in memory.
    /// Feedback welcome!
    #[cfg(feature = "media-metadata")]
    #[arg(long)]
    pub transcode: bool,
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Helper to create Args with specific verbosity settings
    fn args_with_verbosity(verbose: u8, quiet: bool) -> Args {
        Args {
            gui: false,
            server: false,
            stdout: false,
            build: false,
            #[cfg(feature = "media-metadata")]
            extract_video_metadata: false,
            output: PathBuf::from("build"),
            path: PathBuf::from("."),
            oembed_timeout_ms: None,
            oembed_cache_size: None,
            template_folder: None,
            verbose,
            quiet,
            port: None,
            host: None,
            theme: None,
            build_concurrency: None,
            skip_link_checks: false,
            no_link_tracking: false,
            #[cfg(feature = "media-metadata")]
            transcode: false,
        }
    }

    #[test]
    fn test_log_level_default_is_warn() {
        let args = args_with_verbosity(0, false);
        let filter = args.log_level_filter();
        assert!(filter.contains("=warn"));
        assert!(filter.contains("tower_http=warn"));
    }

    #[test]
    fn test_log_level_verbose_once_is_info() {
        let args = args_with_verbosity(1, false);
        let filter = args.log_level_filter();
        assert!(filter.contains("=info"));
        assert!(filter.contains("tower_http=info"));
    }

    #[test]
    fn test_log_level_verbose_twice_is_debug() {
        let args = args_with_verbosity(2, false);
        let filter = args.log_level_filter();
        assert!(filter.contains("=debug"));
        assert!(filter.contains("tower_http=debug"));
    }

    #[test]
    fn test_log_level_verbose_three_times_is_trace() {
        let args = args_with_verbosity(3, false);
        let filter = args.log_level_filter();
        assert!(filter.contains("=trace"));
        assert!(filter.contains("tower_http=trace"));
    }

    #[test]
    fn test_log_level_verbose_more_than_three_is_still_trace() {
        let args = args_with_verbosity(10, false);
        let filter = args.log_level_filter();
        assert!(filter.contains("=trace"));
    }

    #[test]
    fn test_log_level_quiet_is_error() {
        let args = args_with_verbosity(0, true);
        let filter = args.log_level_filter();
        assert!(filter.contains("=error"));
        assert!(filter.contains("tower_http=error"));
    }

    #[test]
    fn test_log_level_quiet_overrides_verbose() {
        // When both quiet and verbose are set, quiet takes precedence
        let args = args_with_verbosity(3, true);
        let filter = args.log_level_filter();
        assert!(filter.contains("=error"));
    }

    #[test]
    fn test_log_level_includes_crate_name() {
        let args = args_with_verbosity(0, false);
        let filter = args.log_level_filter();
        // Should include the crate name (mbr)
        assert!(filter.contains("mbr="));
    }

    // Test CLI parsing with clap
    #[test]
    fn test_parse_default_args() {
        // Parse with no arguments (just the program name)
        let args = Args::parse_from(["mbr"]);
        assert!(!args.gui);
        assert!(!args.server);
        assert!(!args.stdout);
        assert!(!args.build);
        assert_eq!(args.path, PathBuf::from("."));
        assert_eq!(args.output, PathBuf::from("build"));
        assert_eq!(args.verbose, 0);
        assert!(!args.quiet);
    }

    #[test]
    fn test_parse_server_mode() {
        let args = Args::parse_from(["mbr", "-s"]);
        assert!(args.server);
        assert!(!args.gui);
    }

    #[test]
    fn test_parse_gui_mode() {
        let args = Args::parse_from(["mbr", "-g"]);
        assert!(args.gui);
        assert!(!args.server);
    }

    #[test]
    fn test_parse_build_mode() {
        let args = Args::parse_from(["mbr", "-b"]);
        assert!(args.build);
        assert!(!args.server);
        assert!(!args.gui);
    }

    #[test]
    fn test_parse_stdout_mode() {
        let args = Args::parse_from(["mbr", "-o"]);
        assert!(args.stdout);
    }

    #[test]
    fn test_parse_verbose_flags() {
        let args = Args::parse_from(["mbr", "-v"]);
        assert_eq!(args.verbose, 1);

        let args = Args::parse_from(["mbr", "-vv"]);
        assert_eq!(args.verbose, 2);

        let args = Args::parse_from(["mbr", "-vvv"]);
        assert_eq!(args.verbose, 3);
    }

    #[test]
    fn test_parse_quiet_flag() {
        let args = Args::parse_from(["mbr", "-q"]);
        assert!(args.quiet);
    }

    #[test]
    fn test_parse_port() {
        let args = Args::parse_from(["mbr", "-p", "8080"]);
        assert_eq!(args.port, Some(8080));
    }

    #[test]
    fn test_parse_host() {
        let args = Args::parse_from(["mbr", "--host", "0.0.0.0"]);
        assert_eq!(args.host, Some("0.0.0.0".to_string()));
    }

    #[test]
    fn test_parse_theme() {
        let args = Args::parse_from(["mbr", "--theme", "amber"]);
        assert_eq!(args.theme, Some("amber".to_string()));
    }

    #[test]
    fn test_parse_output_directory() {
        let args = Args::parse_from(["mbr", "-b", "--output", "./public"]);
        assert!(args.build);
        assert_eq!(args.output, PathBuf::from("./public"));
    }

    #[test]
    fn test_parse_path_argument() {
        let args = Args::parse_from(["mbr", "/path/to/notes"]);
        assert_eq!(args.path, PathBuf::from("/path/to/notes"));
    }

    #[test]
    fn test_parse_oembed_timeout() {
        let args = Args::parse_from(["mbr", "--oembed-timeout-ms", "1000"]);
        assert_eq!(args.oembed_timeout_ms, Some(1000));
    }

    #[test]
    fn test_parse_build_concurrency() {
        let args = Args::parse_from(["mbr", "-b", "--build-concurrency", "8"]);
        assert_eq!(args.build_concurrency, Some(8));
    }

    #[test]
    fn test_parse_skip_link_checks() {
        let args = Args::parse_from(["mbr", "-b", "--skip-link-checks"]);
        assert!(args.skip_link_checks);
    }

    #[test]
    fn test_parse_no_link_tracking() {
        let args = Args::parse_from(["mbr", "--no-link-tracking"]);
        assert!(args.no_link_tracking);
    }

    #[test]
    fn test_parse_template_folder() {
        let args = Args::parse_from(["mbr", "--template-folder", "/custom/templates"]);
        assert_eq!(
            args.template_folder,
            Some(PathBuf::from("/custom/templates"))
        );
    }
}

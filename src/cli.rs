use crate::config::{Config, IpArray};
use crate::errors::ConfigError;
use clap::{ArgGroup, Parser};
use std::path::PathBuf;

/// Markdown browser and previewer
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
// The mode flags below are mutually exclusive. This is expressed as an
// `ArgGroup` rather than per-argument `conflicts_with_all` lists because two of
// the members are `#[cfg(feature = "media-metadata")]`. A `#[cfg]`-removed
// argument is simply never added to the group, whereas naming it in
// `conflicts_with_all` left a dangling reference that tripped clap's debug
// assertions and panicked on *every* invocation of a build without that
// feature. Keeping the exclusivity in one place also means new mode flags only
// have to join the group instead of being added to every other flag's list.
#[command(group(ArgGroup::new("mode").multiple(false)))]
pub struct Args {
    /// Launch GUI window (default if no mode specified)
    #[arg(short, long, group = "mode")]
    pub gui: bool,

    /// Launch HTTP server only (no GUI)
    #[arg(short, long, group = "mode")]
    pub server: bool,

    /// Render single markdown file to stdout (CLI mode)
    #[arg(short = 'o', long, group = "mode")]
    pub stdout: bool,

    /// Build static site (generate HTML for all markdown files)
    #[arg(short, long, group = "mode")]
    pub build: bool,

    /// Extract video metadata (cover, chapters, captions) and save as sidecar files.
    /// Takes a video file path and generates .cover.jpg, .chapters.en.vtt, and
    /// .captions.en.vtt files next to it (if the video contains this data).
    #[cfg(feature = "media-metadata")]
    #[arg(long, group = "mode")]
    pub extract_video_metadata: bool,

    /// Extract cover images from PDF files and save as sidecar files.
    /// Takes a PDF file or directory path and generates {file}.cover.jpg next to each PDF.
    /// For directories, recursively processes all .pdf files.
    #[cfg(feature = "media-metadata")]
    #[arg(long, group = "mode")]
    pub extract_pdf_cover: bool,

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

    /// Exit with a non-zero status if the static build (-b) detects broken
    /// internal links. Intended for CI. Has no effect with --skip-link-checks
    /// (which skips validation entirely) or outside build mode.
    #[arg(long)]
    pub fail_on_broken_links: bool,

    /// Disable bidirectional link tracking (backlinks).
    /// When disabled, the links.json endpoint returns 404 and no links.json files
    /// are generated during static builds.
    #[arg(long)]
    pub no_link_tracking: bool,

    /// Disable typed relationship tracking (named frontmatter relationships).
    /// When disabled, relationships are omitted from links.json and site.json
    /// and not rendered in the info panel.
    #[arg(long)]
    pub no_relationship_tracking: bool,

    /// Disable the task browser (server/GUI only).
    /// When disabled, the /.mbr/tasks endpoint returns 404 and no task index
    /// is ever built. Static builds never include tasks either way.
    #[arg(long)]
    pub no_tasks: bool,

    /// Highlight blocks that start with an incomplete-marker (TK/TODO/FIXME/XXX).
    /// Default: on for server/GUI mode, off for static builds.
    #[arg(long, conflicts_with = "no_mark_incomplete")]
    pub mark_incomplete: bool,

    /// Disable highlighting of incomplete-marker blocks (TK/TODO/FIXME/XXX).
    #[arg(long, conflicts_with = "mark_incomplete")]
    pub no_mark_incomplete: bool,

    /// Text to prepend to all page titles (e.g., "My Site: ").
    #[arg(long, value_name = "TEXT")]
    pub title_prefix: Option<String>,

    /// Text to append to all page titles (e.g., " | My Site").
    #[arg(long, value_name = "TEXT")]
    pub title_suffix: Option<String>,

    /// [EXPERIMENTAL] Enable dynamic video transcoding to serve lower-resolution
    /// HLS variants (720p, 480p) for bandwidth savings. Only active in server/GUI mode.
    /// Videos are transcoded on-demand as segments and cached in memory.
    /// Feedback welcome!
    #[cfg(feature = "media-metadata")]
    #[arg(long)]
    pub transcode: bool,

    /// Enable the in-browser markdown editing endpoints (server/GUI mode only).
    /// Loopback callers may edit without a token (still CSRF-protected); remote
    /// callers require a token — see --generate-edit-token. Off by default.
    #[arg(long)]
    pub edit: bool,

    /// Generate a hashed editing token from a password (prompted; leave blank to
    /// auto-generate a random token), print the token and the `edit_token_hash`
    /// config line, then exit. Nothing is written to disk.
    #[arg(long)]
    pub generate_edit_token: bool,
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

/// Folds command-line flags into a `Config` already loaded from disk.
///
/// This is the last (highest precedence) layer of the configuration hierarchy
/// described in the README: compiled-in defaults, then `.mbr/config.toml`, then
/// `MBR_*` environment variables, then these flags.
///
/// It lives here rather than inline in `main()` so the wiring — including the
/// two error branches, which are otherwise only constructible from a real
/// process invocation — is reachable from tests.
///
/// # Errors
///
/// - `ConfigError::CanonicalizeFailed` if `--template-folder` does not exist.
/// - `ConfigError::TemplateFolderNotDirectory` if `--template-folder` is not a directory.
/// - `ConfigError::InvalidHost` if `--host` is unparseable or is IPv6 (only IPv4
///   binds are supported, since `Config::host` is a 4-octet array).
/// - Whatever `Config::validate` returns when `--edit` re-validates the config.
pub fn apply_overrides(mut config: Config, args: &Args) -> Result<Config, ConfigError> {
    if let Some(timeout) = args.oembed_timeout_ms {
        config.oembed_timeout_ms = timeout;
    }
    if let Some(cache_size) = args.oembed_cache_size {
        config.oembed_cache_size = cache_size;
    }
    if let Some(ref template_folder) = args.template_folder {
        // Canonicalize and validate the template folder path
        let template_path =
            template_folder
                .canonicalize()
                .map_err(|e| ConfigError::CanonicalizeFailed {
                    path: template_folder.clone(),
                    source: e,
                })?;
        if !template_path.is_dir() {
            return Err(ConfigError::TemplateFolderNotDirectory {
                path: template_path,
            });
        }
        config.template_folder = Some(template_path);
    }
    if let Some(port) = args.port {
        config.port = port;
    }
    if let Some(ref host) = args.host {
        let ip: std::net::IpAddr = host
            .parse()
            .map_err(|_| ConfigError::InvalidHost { host: host.clone() })?;
        match ip {
            std::net::IpAddr::V4(v4) => {
                config.host = IpArray(v4.octets());
            }
            std::net::IpAddr::V6(_) => {
                return Err(ConfigError::InvalidHost { host: host.clone() });
            }
        }
    }
    if let Some(ref theme) = args.theme {
        config.theme = theme.clone();
    }
    if let Some(concurrency) = args.build_concurrency {
        config.build_concurrency = Some(concurrency);
    }
    // Apply transcode options from CLI
    #[cfg(feature = "media-metadata")]
    if args.transcode {
        config.transcode = true;
    }
    // Apply skip_link_checks from CLI
    if args.skip_link_checks {
        config.skip_link_checks = true;
    }
    // Apply no_link_tracking from CLI
    if args.no_link_tracking {
        config.link_tracking = false;
    }
    // Apply no_relationship_tracking from CLI
    if args.no_relationship_tracking {
        config.relationship_tracking = false;
    }
    // Apply no_tasks from CLI
    if args.no_tasks {
        config.tasks_enabled = false;
    }
    // Apply mark_incomplete / no_mark_incomplete from CLI (mutually exclusive)
    if args.mark_incomplete {
        config.mark_incomplete = Some(true);
    } else if args.no_mark_incomplete {
        config.mark_incomplete = Some(false);
    }
    // Apply title_prefix and title_suffix from CLI
    if let Some(ref prefix) = args.title_prefix {
        config.title_prefix = prefix.clone();
    }
    if let Some(ref suffix) = args.title_suffix {
        config.title_suffix = suffix.clone();
    }
    // Enable in-browser editing from CLI
    if args.edit {
        config.edit_enabled = true;
        // Re-validate now that editing is enabled (e.g. non-loopback needs a token).
        config.validate()?;
    }

    Ok(config)
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
            #[cfg(feature = "media-metadata")]
            extract_pdf_cover: false,
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
            fail_on_broken_links: false,
            no_link_tracking: false,
            no_relationship_tracking: false,
            no_tasks: false,
            mark_incomplete: false,
            no_mark_incomplete: false,
            title_prefix: None,
            title_suffix: None,
            #[cfg(feature = "media-metadata")]
            transcode: false,
            edit: false,
            generate_edit_token: false,
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
    fn test_parse_fail_on_broken_links() {
        let args = Args::parse_from(["mbr", "-b", "--fail-on-broken-links"]);
        assert!(args.fail_on_broken_links);
    }

    #[test]
    fn test_parse_no_link_tracking() {
        let args = Args::parse_from(["mbr", "--no-link-tracking"]);
        assert!(args.no_link_tracking);
    }

    #[test]
    fn test_parse_no_relationship_tracking() {
        let args = Args::parse_from(["mbr", "--no-relationship-tracking"]);
        assert!(args.no_relationship_tracking);
    }

    #[test]
    fn test_parse_no_tasks() {
        let args = Args::parse_from(["mbr", "--no-tasks"]);
        assert!(args.no_tasks);
        assert!(!Args::parse_from(["mbr"]).no_tasks);
    }

    #[test]
    fn test_no_tasks_disables_the_config_flag() {
        // Default is on; only the flag turns it off.
        let mut config = Config::default();
        assert!(config.tasks_enabled);

        let args = Args::parse_from(["mbr", "--no-tasks"]);
        config = apply_overrides(config, &args).expect("overrides apply");
        assert!(!config.tasks_enabled);
    }

    #[test]
    fn test_parse_mark_incomplete() {
        let args = Args::parse_from(["mbr", "--mark-incomplete"]);
        assert!(args.mark_incomplete);
        assert!(!args.no_mark_incomplete);
    }

    #[test]
    fn test_parse_no_mark_incomplete() {
        let args = Args::parse_from(["mbr", "--no-mark-incomplete"]);
        assert!(args.no_mark_incomplete);
        assert!(!args.mark_incomplete);
    }

    #[test]
    fn test_parse_mark_incomplete_conflicts_with_no_mark_incomplete() {
        let result = Args::try_parse_from(["mbr", "--mark-incomplete", "--no-mark-incomplete"]);
        assert!(result.is_err(), "Mutually exclusive flags should error");
    }

    /// Validates the entire clap command definition: dangling `conflicts_with`
    /// / group references, duplicate ids, invalid defaults.
    ///
    /// This is clap's own self-check and it must hold under *every* feature
    /// combination. Without it, a build compiled without `media-metadata`
    /// referenced the feature-gated `extract_*` arguments in the mode flags'
    /// conflict lists and panicked inside clap on **every** invocation — a
    /// shipped-binary bug, not just a test failure.
    #[test]
    fn test_command_definition_is_valid() {
        use clap::CommandFactory;
        Args::command().debug_assert();
    }

    /// The mode flags are mutually exclusive. Only the ungated flags are used
    /// here, so this holds no matter how the crate is compiled.
    #[test]
    fn test_mode_flags_are_mutually_exclusive() {
        let modes = ["--gui", "--server", "--stdout", "--build"];
        for (i, first) in modes.iter().enumerate() {
            for second in &modes[i + 1..] {
                let result = Args::try_parse_from(["mbr", first, second]);
                assert!(
                    result.is_err(),
                    "{first} and {second} should be mutually exclusive"
                );
            }
        }
    }

    /// Each mode flag must still be accepted on its own.
    #[test]
    fn test_each_mode_flag_parses_alone() {
        for mode in ["--gui", "--server", "--stdout", "--build"] {
            assert!(
                Args::try_parse_from(["mbr", mode]).is_ok(),
                "{mode} should parse on its own"
            );
        }
    }

    /// With `media-metadata` on, the extract flags join the same exclusivity
    /// group as the other modes. This pins the behavior the `ArgGroup` replaced.
    #[cfg(feature = "media-metadata")]
    #[test]
    fn test_extract_flags_conflict_with_other_modes() {
        for extract in ["--extract-video-metadata", "--extract-pdf-cover"] {
            for mode in ["--gui", "--server", "--stdout", "--build"] {
                let result = Args::try_parse_from(["mbr", extract, mode]);
                assert!(result.is_err(), "{extract} and {mode} should conflict");
            }
        }
        let result =
            Args::try_parse_from(["mbr", "--extract-video-metadata", "--extract-pdf-cover"]);
        assert!(
            result.is_err(),
            "the two extract flags should conflict with each other"
        );
    }

    #[cfg(feature = "media-metadata")]
    #[test]
    fn test_parse_extract_pdf_cover() {
        let args = Args::parse_from(["mbr", "--extract-pdf-cover", "/path/to/pdfs"]);
        assert!(args.extract_pdf_cover);
        assert_eq!(args.path, PathBuf::from("/path/to/pdfs"));
    }

    #[test]
    fn test_parse_template_folder() {
        let args = Args::parse_from(["mbr", "--template-folder", "/custom/templates"]);
        assert_eq!(
            args.template_folder,
            Some(PathBuf::from("/custom/templates"))
        );
    }

    #[test]
    fn test_parse_title_prefix() {
        let args = Args::parse_from(["mbr", "--title-prefix", "My Site: "]);
        assert_eq!(args.title_prefix, Some("My Site: ".to_string()));
    }

    #[test]
    fn test_parse_title_suffix() {
        let args = Args::parse_from(["mbr", "--title-suffix", " | My Site"]);
        assert_eq!(args.title_suffix, Some(" | My Site".to_string()));
    }

    // ---- apply_overrides ----------------------------------------------------
    //
    // These drive the real clap parser so the tests pin the whole flag → config
    // path, not just the assignment. Before this block the override wiring was
    // inline in `main()` and unreachable: deleting the `--theme` assignment left
    // fmt/clippy/test green while `mbr -s --theme amber` served the default.

    /// Parses `argv` and applies it to a default `Config`, panicking on error.
    fn overridden(argv: &[&str]) -> Config {
        apply_overrides(Config::default(), &Args::parse_from(argv)).expect("overrides should apply")
    }

    #[test]
    fn test_apply_overrides_no_flags_changes_nothing() {
        let base = Config::default();
        let config = overridden(&["mbr"]);
        assert_eq!(config.port, base.port);
        assert_eq!(config.theme, base.theme);
        assert_eq!(config.host, base.host);
        assert_eq!(config.title_prefix, base.title_prefix);
        assert_eq!(config.title_suffix, base.title_suffix);
        assert_eq!(config.template_folder, base.template_folder);
        assert_eq!(config.build_concurrency, base.build_concurrency);
        assert_eq!(config.mark_incomplete, base.mark_incomplete);
        assert_eq!(config.skip_link_checks, base.skip_link_checks);
        assert_eq!(config.link_tracking, base.link_tracking);
        assert_eq!(config.relationship_tracking, base.relationship_tracking);
        assert_eq!(config.edit_enabled, base.edit_enabled);
    }

    #[test]
    fn test_apply_overrides_sets_port() {
        assert_eq!(overridden(&["mbr", "-s", "-p", "5299"]).port, 5299);
    }

    #[test]
    fn test_apply_overrides_sets_theme() {
        assert_eq!(
            overridden(&["mbr", "-s", "--theme", "amber"]).theme,
            "amber"
        );
    }

    #[test]
    fn test_apply_overrides_sets_title_prefix_and_suffix() {
        let config = overridden(&[
            "mbr",
            "--title-prefix",
            "My Site: ",
            "--title-suffix",
            " | Docs",
        ]);
        assert_eq!(config.title_prefix, "My Site: ");
        assert_eq!(config.title_suffix, " | Docs");
    }

    #[test]
    fn test_apply_overrides_sets_ipv4_host() {
        let config = overridden(&["mbr", "-s", "--host", "0.0.0.0"]);
        assert_eq!(config.host, IpArray([0, 0, 0, 0]));
    }

    #[test]
    fn test_apply_overrides_sets_oembed_and_build_concurrency() {
        let config = overridden(&[
            "mbr",
            "--oembed-timeout-ms",
            "1234",
            "--oembed-cache-size",
            "4096",
            "--build-concurrency",
            "8",
        ]);
        assert_eq!(config.oembed_timeout_ms, 1234);
        assert_eq!(config.oembed_cache_size, 4096);
        assert_eq!(config.build_concurrency, Some(8));
    }

    #[test]
    fn test_apply_overrides_toggle_flags() {
        assert!(overridden(&["mbr", "-b", "--skip-link-checks"]).skip_link_checks);
        assert!(!overridden(&["mbr", "--no-link-tracking"]).link_tracking);
        assert!(!overridden(&["mbr", "--no-relationship-tracking"]).relationship_tracking);
        assert_eq!(
            overridden(&["mbr", "--mark-incomplete"]).mark_incomplete,
            Some(true)
        );
        assert_eq!(
            overridden(&["mbr", "--no-mark-incomplete"]).mark_incomplete,
            Some(false)
        );
    }

    /// `--host ::1` parses as an IP but has no place to live: `Config::host` is
    /// four octets, so IPv6 is rejected rather than silently truncated.
    #[test]
    fn test_apply_overrides_rejects_ipv6_host() {
        let result = apply_overrides(
            Config::default(),
            &Args::parse_from(["mbr", "--host", "::1"]),
        );
        assert!(
            matches!(&result, Err(ConfigError::InvalidHost { host }) if host == "::1"),
            "expected InvalidHost, got {result:?}"
        );
    }

    #[test]
    fn test_apply_overrides_rejects_unparseable_host() {
        let result = apply_overrides(
            Config::default(),
            &Args::parse_from(["mbr", "--host", "not-an-ip"]),
        );
        assert!(
            matches!(&result, Err(ConfigError::InvalidHost { host }) if host == "not-an-ip"),
            "expected InvalidHost, got {result:?}"
        );
    }

    #[test]
    fn test_apply_overrides_accepts_template_folder_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let expected = dir.path().canonicalize().expect("canonicalize");
        let config = apply_overrides(
            Config::default(),
            &Args::parse_from(["mbr", "--template-folder", dir.path().to_str().unwrap()]),
        )
        .expect("a real directory should be accepted");
        assert_eq!(config.template_folder, Some(expected));
    }

    #[test]
    fn test_apply_overrides_rejects_template_folder_that_is_a_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("index.html");
        std::fs::write(&file, "<html></html>").expect("write");
        let result = apply_overrides(
            Config::default(),
            &Args::parse_from(["mbr", "--template-folder", file.to_str().unwrap()]),
        );
        assert!(
            matches!(&result, Err(ConfigError::TemplateFolderNotDirectory { .. })),
            "expected TemplateFolderNotDirectory, got {result:?}"
        );
    }

    #[test]
    fn test_apply_overrides_rejects_missing_template_folder() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("does-not-exist");
        let result = apply_overrides(
            Config::default(),
            &Args::parse_from(["mbr", "--template-folder", missing.to_str().unwrap()]),
        );
        assert!(
            matches!(&result, Err(ConfigError::CanonicalizeFailed { .. })),
            "expected CanonicalizeFailed, got {result:?}"
        );
    }

    #[test]
    fn test_apply_overrides_edit_flag_enables_editing() {
        // The default host is loopback, so enabling editing needs no token and
        // the re-`validate()` at the end of the override block succeeds.
        assert!(overridden(&["mbr", "-s", "--edit"]).edit_enabled);
    }
}

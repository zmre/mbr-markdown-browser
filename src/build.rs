//! Static site generation module for mbr.
//!
//! Generates static HTML files from markdown, creating a deployable site.

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

use percent_encoding::percent_decode_str;
use walkdir::WalkDir;

use scraper::{Html, Selector};

use std::sync::Arc;

use papaya::HashMap as ConcurrentHashMap;

use crate::{
    config::Config,
    embedded_pico,
    errors::BuildError,
    link_index::{InboundLink, OutboundLink, PageLinks, resolve_relative_url},
    link_transform::{LinkTransformConfig, make_relative_url},
    markdown,
    oembed_cache::OembedCache,
    page_context::{self, ModeFlags, PageChrome, UrlMode},
    repo::{MarkdownInfo, Repo},
    server::{
        DEFAULT_FILES, MediaViewerType, generate_breadcrumbs, get_current_dir_name,
        get_parent_path, markdown_file_to_json,
    },
    sorting::sort_files,
    templates::Templates,
};

/// Maps each directory (relative path) to its direct child markdown files (as
/// template JSON with absolute `url_path`) and the set of immediate
/// subdirectory names. Built once per build so section pages resolve their
/// children in O(children) instead of rescanning all markdown files.
type DirChildrenIndex = HashMap<PathBuf, (Vec<serde_json::Value>, HashSet<String>)>;

/// Groups markdown files into a directory→children index in a single pass.
///
/// Each file is registered as a direct child of its containing directory and
/// contributes its first-level subdirectory name to each ancestor directory.
/// The stored file JSON keeps its absolute `url_path`; callers relativize and
/// sort their own slice. Root is keyed by the empty path.
fn build_dir_children_index<'a>(files: impl Iterator<Item = &'a MarkdownInfo>) -> DirChildrenIndex {
    let mut index: DirChildrenIndex = HashMap::new();
    index.entry(PathBuf::new()).or_default();
    for info in files {
        let trimmed = info.url_path.trim_start_matches('/').trim_end_matches('/');
        if trimmed.is_empty() {
            continue;
        }
        let components: Vec<&str> = trimmed.split('/').collect();
        let n = components.len();
        // Register the file as a direct child of its containing directory.
        let containing: PathBuf = components[..n - 1].iter().collect();
        index
            .entry(containing)
            .or_default()
            .0
            .push(markdown_file_to_json(info));
        // Register the immediate subdirectory name for each ancestor.
        for i in 0..n - 1 {
            let dir: PathBuf = components[..i].iter().collect();
            index
                .entry(dir)
                .or_default()
                .1
                .insert(components[i].to_string());
        }
    }
    index
}

/// Maximum build concurrency (parallel file processing limit).
const MAX_BUILD_CONCURRENCY: usize = 32;

/// Fallback concurrency when CPU core count detection fails.
const FALLBACK_BUILD_CONCURRENCY: usize = 4;

/// Calculate the directory depth from a URL path.
///
/// Examples:
/// - "/" or "" → 0
/// - "/docs/" → 1
/// - "/docs/guide/" → 2
fn url_depth(url_path: &str) -> usize {
    url_path
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .count()
}

/// Build the relative path prefix for .mbr assets based on page depth.
///
/// Examples:
/// - depth 0 → ".mbr/"
/// - depth 1 → "../.mbr/"
/// - depth 2 → "../../.mbr/"
pub(crate) fn relative_base(depth: usize) -> String {
    if depth == 0 {
        ".mbr/".to_string()
    } else {
        format!("{}.mbr/", "../".repeat(depth))
    }
}

/// Build the relative path prefix to root based on page depth.
///
/// Examples:
/// - depth 0 → "" (empty string, already at root)
/// - depth 1 → "../"
/// - depth 2 → "../../"
pub(crate) fn relative_root(depth: usize) -> String {
    if depth == 0 {
        String::new()
    } else {
        "../".repeat(depth)
    }
}

/// Prints a progress stage message to stdout.
///
/// This bypasses the logging system to provide direct user feedback during builds.
fn print_stage(stage: &str) {
    print!("\r\x1b[K{}", stage);
    let _ = io::stdout().flush();
}

/// Prints a progress update with count/total to stdout.
///
/// Uses carriage return to update in place for a cleaner terminal experience.
fn print_progress(stage: &str, current: usize, total: usize) {
    print!("\r\x1b[K{} ({}/{})", stage, current, total);
    let _ = io::stdout().flush();
}

/// Formats a duration for display: "1.23s" or "1m 23.4s" for longer durations.
fn format_duration(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs >= 60.0 {
        format!("{:.0}m {:.1}s", (secs / 60.0).floor(), secs % 60.0)
    } else {
        format!("{:.2}s", secs)
    }
}

/// Prints a completed stage message with a newline.
fn print_stage_done(stage: &str, count: usize, duration: Option<Duration>) {
    if let Some(d) = duration {
        println!(
            "\r\x1b[K{} ... {} done ({})",
            stage,
            count,
            format_duration(d)
        );
    } else {
        println!("\r\x1b[K{} ... {} done", stage, count);
    }
}

/// Prints a completed stage message without count.
fn print_done(stage: &str, duration: Option<Duration>) {
    if let Some(d) = duration {
        println!("\r\x1b[K{} ... done ({})", stage, format_duration(d));
    } else {
        println!("\r\x1b[K{} ... done", stage);
    }
}

/// Normalize a path by resolving `.` and `..` components without requiring the path to exist.
fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                // Pop the last component if we can (and it's not also a parent dir)
                if !components.is_empty() {
                    components.pop();
                }
            }
            std::path::Component::CurDir => {
                // Skip "." components
            }
            _ => {
                components.push(component);
            }
        }
    }
    components.iter().collect()
}

/// Checks if a link target exists using a pre-built set of valid file paths.
///
/// Uses O(1) HashSet lookups instead of filesystem stat() calls.
/// Handles both direct file matches and the directory/index.html convention.
fn link_target_exists(path: &Path, valid_files: &HashSet<PathBuf>) -> bool {
    valid_files.contains(path) || valid_files.contains(&path.join("index.html"))
}

/// Records the first error observed across parallel (rayon) workers.
///
/// Wraps a `Mutex<Option<E>>`. The guarded data is plain (no invariants a
/// panicking holder could break), so a poisoned lock is recovered with
/// `PoisonError::into_inner` rather than propagating the panic.
struct FirstError<E>(std::sync::Mutex<Option<E>>);

impl<E> FirstError<E> {
    fn new() -> Self {
        Self(std::sync::Mutex::new(None))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<E>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Returns true if an error has already been recorded.
    fn is_set(&self) -> bool {
        self.lock().is_some()
    }

    /// Records `err` unless an earlier error was already recorded.
    fn record(&self, err: E) {
        let mut guard = self.lock();
        if guard.is_none() {
            *guard = Some(err);
        }
    }

    /// Consumes the tracker, yielding `Err(first_error)` if any was recorded.
    fn into_result(self) -> Result<(), E> {
        match self
            .0
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

/// Statistics from a build run.
#[derive(Debug, Default)]
pub struct BuildStats {
    pub markdown_pages: usize,
    pub section_pages: usize,
    pub tag_pages: usize,
    pub assets_linked: usize,
    pub duration: Duration,
    /// Whether Pagefind search indexing succeeded (None = not attempted)
    pub pagefind_indexed: Option<bool>,
    /// Number of broken links detected
    pub broken_links: usize,
    /// Number of pages whose YAML frontmatter failed to parse
    pub frontmatter_errors: usize,
    /// Number of links.json files written (for link tracking)
    pub link_files: usize,
}

/// A broken link detected during build.
#[derive(Debug, Clone)]
pub struct BrokenLink {
    /// The source page containing the broken link
    pub source_page: String,
    /// The broken link URL
    pub link_url: String,
}

/// Static site builder.
pub struct Builder {
    config: Config,
    templates: Templates,
    output_dir: PathBuf,
    repo: Repo,
    /// Cache for OEmbed page metadata shared across all file renders
    oembed_cache: Arc<OembedCache>,
    /// Index of outbound links per page (url_path -> (is_index_file, links)).
    /// Used for building bidirectional link tracking during static builds.
    /// `is_index_file` is needed to correctly resolve relative URLs: non-index
    /// pages drop the trailing URL segment (file stem), index pages don't.
    build_link_index: Arc<ConcurrentHashMap<String, (bool, Vec<OutboundLink>)>>,
    /// Frontmatter (YAML) parse errors collected during the parallel render
    /// pass (url_path -> error message). Summarized to stderr after the build,
    /// mirroring broken-link reporting.
    frontmatter_errors: Arc<ConcurrentHashMap<String, String>>,
}

impl Builder {
    /// Creates a new Builder instance.
    pub fn new(config: Config, output_dir: PathBuf) -> Result<Self, BuildError> {
        let templates = Templates::new(&config.root_dir, config.template_folder.as_deref())?;
        let repo = Repo::init_from_config(&config);
        let oembed_cache = Arc::new(OembedCache::new(config.oembed_cache_size));
        let build_link_index = Arc::new(ConcurrentHashMap::new());
        let frontmatter_errors = Arc::new(ConcurrentHashMap::new());

        tracing::debug!(
            "build: initialized oembed cache with {} bytes max",
            config.oembed_cache_size
        );

        Ok(Builder {
            config,
            templates,
            output_dir,
            repo,
            oembed_cache,
            build_link_index,
            frontmatter_errors,
        })
    }

    /// Builds the static site.
    pub async fn build(&self) -> Result<BuildStats, BuildError> {
        let start = Instant::now();
        let mut stats = BuildStats::default();

        // Scan repository for all files
        let stage_start = Instant::now();
        print_stage("Scanning repository...");
        self.repo.scan_all()?;
        self.repo.scan_static_folder()?;
        // Build the typed-relationship index once all note titles are known.
        if self.config.relationship_tracking {
            self.repo.build_relationship_index();
        }
        // Build the global wikilink name index (always on) so body `[[Name]]`
        // links resolve globally during the render pass and link validation.
        self.repo.build_wikilink_index();
        let file_count = self.repo.markdown_files.pin().len() + self.repo.other_files.pin().len();
        print_stage_done(
            "Scanning repository",
            file_count,
            Some(stage_start.elapsed()),
        );

        // Prepare output directory
        let stage_start = Instant::now();
        print_stage("Cleaning output directory...");
        self.prepare_output_dir()?;
        print_done("Cleaning output directory", Some(stage_start.elapsed()));

        // Render all markdown files
        stats.markdown_pages = self.render_markdown_files().await?;

        // Write links.json files (if link tracking is enabled)
        if self.config.link_tracking {
            stats.link_files = self.write_link_files().await?;
        }

        // Generate directory/section pages
        stats.section_pages = self.render_directory_pages().await?;

        // Generate tag pages (if enabled)
        if self.config.build_tag_pages {
            stats.tag_pages = self.render_tag_pages().await?;
        } else {
            println!("Generating tag pages ... skipped");
        }

        // Symlink assets (images, PDFs, etc.)
        let stage_start = Instant::now();
        print_stage("Linking assets...");
        stats.assets_linked = self.symlink_assets()?;
        print_stage_done(
            "Linking assets",
            stats.assets_linked,
            Some(stage_start.elapsed()),
        );

        // Handle static folder overlay
        let stage_start = Instant::now();
        print_stage("Processing static folder...");
        self.handle_static_folder()?;
        print_done("Processing static folder", Some(stage_start.elapsed()));

        // Handle .mbr folder (copy, write defaults, generate site.json)
        let stage_start = Instant::now();
        print_stage("Copying theme and assets...");
        self.handle_mbr_folder()?;
        print_done("Copying theme and assets", Some(stage_start.elapsed()));

        // Generate 404.html for GitHub Pages compatibility
        self.generate_404_page()?;

        // Generate media viewer pages (videos, pdfs, audio)
        self.generate_media_viewer_pages()?;

        // Validate internal links and report broken ones
        if self.config.skip_link_checks {
            println!("Validating links ... skipped");
        } else {
            let stage_start = Instant::now();
            print_stage("Validating links...");
            let broken_links = self.validate_links();
            stats.broken_links = broken_links.len();
            print_done("Validating links", Some(stage_start.elapsed()));

            if !broken_links.is_empty() {
                eprintln!(
                    "\n⚠️  Broken links detected ({} total):",
                    broken_links.len()
                );
                for link in &broken_links {
                    eprintln!("   {} → {}", link.source_page, link.link_url);
                }
                eprintln!();
            }
        }

        // Report any YAML frontmatter parse errors collected during rendering.
        // A failed parse discards the entire frontmatter (title, style, etc.),
        // so surface it the same way broken links are surfaced.
        {
            let guard = self.frontmatter_errors.pin();
            stats.frontmatter_errors = guard.len();
            if stats.frontmatter_errors > 0 {
                eprintln!(
                    "\n⚠️  Frontmatter parse errors ({} total):",
                    stats.frontmatter_errors
                );
                for (page, message) in guard.iter() {
                    eprintln!("   {page} → {message}");
                }
                eprintln!();
            }
        }

        // Run Pagefind to generate search index
        let stage_start = Instant::now();
        print_stage("Building search index...");
        stats.pagefind_indexed = Some(self.run_pagefind().await);
        if stats.pagefind_indexed == Some(true) {
            print_done("Building search index", Some(stage_start.elapsed()));
        } else {
            println!("\r\x1b[KBuilding search index ... skipped");
        }

        stats.duration = start.elapsed();
        Ok(stats)
    }

    /// Creates or cleans the output directory.
    ///
    /// Uses an atomic rename to move the old directory aside, then deletes it in a background
    /// thread. This lets the build start immediately instead of waiting for recursive removal,
    /// which is O(n) on the number of filesystem entries.
    fn prepare_output_dir(&self) -> Result<(), BuildError> {
        if self.output_dir.exists() {
            // Rename old dir aside (O(1) atomic operation), then delete in background.
            let tmp_dir = self
                .output_dir
                .with_extension(format!("old.{}", std::process::id()));
            // If a stale temp dir exists from a previous interrupted build, remove it first
            if tmp_dir.exists() {
                let _ = fs::remove_dir_all(&tmp_dir);
            }
            fs::rename(&self.output_dir, &tmp_dir).map_err(|e| BuildError::CreateDirFailed {
                path: self.output_dir.clone(),
                source: e,
            })?;
            // Spawn background thread to delete the old directory
            std::thread::spawn(move || {
                let _ = fs::remove_dir_all(&tmp_dir);
            });
        }
        fs::create_dir_all(&self.output_dir).map_err(|e| BuildError::CreateDirFailed {
            path: self.output_dir.clone(),
            source: e,
        })?;
        Ok(())
    }

    /// Returns the effective build concurrency based on config or auto-detection.
    fn get_concurrency(&self) -> usize {
        self.config.build_concurrency.unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| std::cmp::min(n.get() * 2, MAX_BUILD_CONCURRENCY))
                .unwrap_or(FALLBACK_BUILD_CONCURRENCY)
        })
    }

    /// Renders all markdown files to HTML in parallel.
    async fn render_markdown_files(&self) -> Result<usize, BuildError> {
        let stage_start = Instant::now();
        let markdown_files: Vec<_> = self
            .repo
            .markdown_files
            .pin()
            .iter()
            .map(|(path, info)| (path.clone(), info.clone()))
            .collect();

        let count = markdown_files.len();
        let concurrency = self.get_concurrency();

        tracing::info!(
            "Rendering {} markdown files with concurrency {}",
            count,
            concurrency
        );

        // Pre-build sibling index: group files by parent directory and sort each group once.
        // This turns O(n²) per-file sibling scanning into O(n log n) total.
        let sibling_index = {
            let mut index: HashMap<PathBuf, Vec<serde_json::Value>> = HashMap::new();
            for (_, info) in &markdown_files {
                let parent = info
                    .raw_path
                    .parent()
                    .unwrap_or(Path::new(""))
                    .to_path_buf();
                index
                    .entry(parent)
                    .or_default()
                    .push(markdown_file_to_json(info));
            }
            for siblings in index.values_mut() {
                sort_files(siblings, &self.config.sort);
            }
            Arc::new(index)
        };

        // Progress counter for parallel rendering
        let completed = Arc::new(AtomicUsize::new(0));
        print_progress("Rendering markdown", 0, count);

        // Use rayon for true CPU parallelism — render_single_markdown_sync does
        // zero async work (all fs ops are sync, oembed is disabled in build mode).
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(concurrency)
            .build()
            .map_err(|e| BuildError::CreateDirFailed {
                path: self.output_dir.clone(),
                source: std::io::Error::other(format!("Failed to create rayon thread pool: {}", e)),
            })?;

        // Clone Tera once before entering the rayon pool to avoid per-file
        // RwLock contention. Tera is ~KB of template AST, so this is cheap
        // compared to 44K+ lock acquisitions from competing rayon threads.
        let tera_snapshot = self.templates.tera_clone();

        let error: FirstError<BuildError> = FirstError::new();

        pool.install(|| {
            use rayon::prelude::*;
            markdown_files.par_iter().for_each(|(path, info)| {
                // Skip remaining files once an error has been recorded
                if error.is_set() {
                    return;
                }
                match self.render_single_markdown_sync(path, info, &sibling_index, &tera_snapshot) {
                    Ok(()) => {
                        let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                        // Batch progress: only flush stdout every 100 files or at completion
                        // to avoid mutex contention from 44K+ competing rayon threads.
                        if done.is_multiple_of(100) || done == count {
                            print_progress("Rendering markdown", done, count);
                        }
                    }
                    Err(e) => error.record(e),
                }
            });
        });

        error.into_result()?;

        print_stage_done("Rendering markdown", count, Some(stage_start.elapsed()));
        Ok(count)
    }

    /// Writes links.json files for all pages with bidirectional link information.
    ///
    /// This method:
    /// 1. Builds an inbound link index by inverting the outbound links
    /// 2. Writes links.json files in parallel for each page
    async fn write_link_files(&self) -> Result<usize, BuildError> {
        let stage_start = Instant::now();
        print_stage("Building link index...");

        // Step 1: Build the inbound index by inverting outbound links
        // For each outbound link from page A to page B, create an inbound link on page B from A
        // Also collect the outbound index into a regular HashMap for thread-safe parallel access
        let outbound_guard = self.build_link_index.pin();
        let mut inbound_index: HashMap<String, Vec<InboundLink>> = HashMap::new();
        let mut outbound_index: HashMap<String, Vec<OutboundLink>> = HashMap::new();

        for (source_url, (is_index_file, outbound_links)) in outbound_guard.iter() {
            // Copy outbound links to our local HashMap
            outbound_index.insert(source_url.clone(), outbound_links.clone());

            for link in outbound_links {
                // Only track internal links for inbound
                if !link.internal {
                    continue;
                }

                // Resolve the relative URL to an absolute URL based on the source page
                let target_url = resolve_relative_url(source_url, &link.to, *is_index_file);

                let inbound_link = InboundLink {
                    from: source_url.clone(),
                    text: link.text.clone(),
                    anchor: link.anchor.clone(),
                };

                inbound_index
                    .entry(target_url)
                    .or_default()
                    .push(inbound_link);
            }
        }

        // Step 2: Collect all known page URLs that can actually render backlinks.
        // We intentionally exclude inbound_index keys here — those may contain URLs
        // to static files (PDFs, images) which can't display backlinks. Creating
        // directories for them would shadow the actual file symlinks during asset linking.
        let all_page_urls: HashSet<String> = {
            let mut urls: HashSet<String> = outbound_index.keys().cloned().collect();

            // Include all markdown pages (even those without links)
            for (_, info) in self.repo.markdown_files.pin().iter() {
                urls.insert(info.url_path.clone());
            }

            // Include tag page URLs if tag pages are enabled
            if self.config.build_tag_pages {
                for tag_source in &self.config.tag_sources {
                    let source = tag_source.url_source();

                    // Add tag source index URL (e.g., "/tags/")
                    urls.insert(format!("/{}/", source));

                    // Add individual tag page URLs (e.g., "/tags/rust/")
                    for tag in self.repo.tag_index.get_all_tags(&source) {
                        urls.insert(format!("/{}/{}/", source, tag.normalized));
                    }
                }
            }

            urls
        };

        let count = all_page_urls.len();
        let concurrency = self.get_concurrency();

        // Progress counter for parallel writing
        let completed = Arc::new(AtomicUsize::new(0));
        print_progress("Writing link files", 0, count);

        let page_urls: Vec<String> = all_page_urls.into_iter().collect();

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(concurrency)
            .build()
            .map_err(|e| BuildError::CreateDirFailed {
                path: self.output_dir.clone(),
                source: std::io::Error::other(format!("Failed to create rayon thread pool: {}", e)),
            })?;

        let error: FirstError<BuildError> = FirstError::new();

        pool.install(|| {
            use rayon::prelude::*;
            page_urls.par_iter().for_each(|url_path| {
                if error.is_set() {
                    return;
                }
                match self.write_single_link_file(url_path, &outbound_index, &inbound_index) {
                    Ok(()) => {
                        let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                        if done.is_multiple_of(100) || done == count {
                            print_progress("Writing link files", done, count);
                        }
                    }
                    Err(e) => error.record(e),
                }
            });
        });

        error.into_result()?;

        print_stage_done("Writing link files", count, Some(stage_start.elapsed()));
        Ok(count)
    }

    /// Writes a single links.json file for a page.
    fn write_single_link_file(
        &self,
        url_path: &str,
        outbound_index: &HashMap<String, Vec<OutboundLink>>,
        inbound_index: &HashMap<String, Vec<InboundLink>>,
    ) -> Result<(), BuildError> {
        // Try to build tag page outbound links, or fall back to the index
        let outbound = self
            .try_build_tag_outbound(url_path)
            .unwrap_or_else(|| outbound_index.get(url_path).cloned().unwrap_or_default());
        let inbound = inbound_index.get(url_path).cloned().unwrap_or_default();

        // Typed relationships (declared + derived) for this page, if enabled.
        let relationships = if self.config.relationship_tracking {
            self.repo.relationship_index.get(url_path)
        } else {
            Vec::new()
        };

        let page_links = PageLinks {
            inbound,
            outbound,
            relationships,
        };

        // Determine output path: url_path → build/{url_path}/links.json
        let url_path_stripped = url_path.trim_start_matches('/');
        let output_path = if url_path_stripped.is_empty() || url_path == "/" {
            self.output_dir.join("links.json")
        } else {
            self.output_dir.join(url_path_stripped).join("links.json")
        };

        // Create parent directories
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|e| BuildError::CreateDirFailed {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }

        // Write JSON file
        let json = serde_json::to_string(&page_links).map_err(|e| BuildError::WriteFailed {
            path: output_path.clone(),
            source: std::io::Error::other(format!("JSON serialization failed: {}", e)),
        })?;

        fs::write(&output_path, json).map_err(|e| BuildError::WriteFailed {
            path: output_path,
            source: e,
        })?;

        Ok(())
    }

    /// Synchronous version of `render_single_markdown` for use with rayon parallelism.
    ///
    /// All I/O in the render pipeline is already synchronous (std::fs, pulldown-cmark,
    /// Tera templates, papaya concurrent HashMap). This avoids the overhead of an async
    /// runtime when oembed fetching is disabled (the default in build mode).
    ///
    /// Takes a pre-cloned `&Tera` to avoid `Arc<RwLock<Tera>>` contention when
    /// many rayon threads render in parallel.
    fn render_single_markdown_sync(
        &self,
        path: &Path,
        info: &MarkdownInfo,
        sibling_index: &HashMap<PathBuf, Vec<serde_json::Value>>,
        tera: &tera::Tera,
    ) -> Result<(), BuildError> {
        // Determine if this is an index file (which doesn't need ../ prefix for links)
        let is_index_file = path
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(|f| f == self.config.index_file);

        let link_transform_config = LinkTransformConfig {
            markdown_extensions: self.config.markdown_extensions.clone(),
            index_file: self.config.index_file.clone(),
            is_index_file,
            url_depth: Some(url_depth(&info.url_path)),
            current_page_url: info.url_path.clone(),
        };

        tracing::debug!("build: rendering {}", path.display());

        // Render markdown to HTML synchronously
        // In build mode, server_mode=false and transcode is disabled (transcode is server-only).
        // Build mode defaults `mark_incomplete=false` (off unless config/CLI override).
        let valid_tag_sources = crate::config::tag_sources_to_set(&self.config.tag_sources);
        let mark_incomplete = self.config.mark_incomplete.unwrap_or(false);
        let render_result = markdown::render_sync(
            path.to_path_buf(),
            &self.config.root_dir,
            self.config.oembed_timeout_ms,
            link_transform_config,
            Some(self.oembed_cache.clone()),
            false, // server_mode is false in build mode
            false, // transcode is disabled in build mode
            valid_tag_sources,
            mark_incomplete,
            &self.config.incomplete_markers,
            Some(self.repo.wikilink_index.clone()),
        )
        .map_err(|e| BuildError::RenderFailed {
            path: path.to_path_buf(),
            source: Box::new(crate::MbrError::Io(std::io::Error::other(e.to_string()))),
        })?;
        // Record any frontmatter parse error so it can be summarized after the
        // parallel render pass completes (mirrors broken-link reporting).
        if let Some(err) = render_result.frontmatter_error {
            self.frontmatter_errors
                .pin()
                .insert(info.url_path.clone(), err);
        }

        let mut frontmatter = render_result.frontmatter;
        let headings = render_result.headings;
        let html = render_result.html;
        let outbound_links = render_result.outbound_links;
        let has_h1 = render_result.has_h1;
        let word_count = render_result.word_count;
        let readability_counts = crate::readability::ReadabilityCounts {
            words: render_result.word_count,
            sentences: render_result.sentence_count,
            syllables: render_result.syllable_count,
        };
        let readability_scores = crate::readability::scores(&readability_counts);

        // Store outbound links in the build link index for generating links.json files
        if self.config.link_tracking && !outbound_links.is_empty() {
            self.build_link_index
                .pin()
                .insert(info.url_path.clone(), (is_index_file, outbound_links));
        }

        tracing::debug!("build: rendered {}", path.display());

        // Add markdown_source to frontmatter
        frontmatter.insert(
            "markdown_source".to_string(),
            serde_json::Value::String(info.url_path.clone()),
        );

        // Indicate static mode (no dynamic search endpoint)
        // Boolean false is falsy in Tera templates
        frontmatter.insert("server_mode".to_string(), serde_json::json!(false));

        // Calculate page depth for relative path generation
        let depth = url_depth(&info.url_path);

        // File path (relative to root) for reference
        let relative_path = path
            .strip_prefix(&self.config.root_dir)
            .unwrap_or(path)
            .to_string_lossy();

        // Modified date from file metadata
        let modified_secs = std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());

        // Prev/next sibling pages come from the pre-built index
        let parent_dir = info
            .raw_path
            .parent()
            .unwrap_or(Path::new(""))
            .to_path_buf();
        let empty_siblings = Vec::new();
        let siblings = sibling_index.get(&parent_dir).unwrap_or(&empty_siblings);

        // Build the extra context (navigation, TOC, readability, chrome) via
        // the shared builder; static builds relativize URLs to the page depth.
        let extra_context = page_context::markdown_extra_context(
            &page_context::MarkdownPageParams {
                breadcrumb_path: std::path::Path::new(&info.url_path),
                headings: &headings,
                has_h1,
                word_count,
                readability: &readability_scores,
                file_path: &relative_path,
                modified_secs,
                current_url: &info.url_path,
                siblings,
            },
            &page_context::MarkdownContextOptions {
                tag_sources: &self.config.tag_sources,
                sidebar_style: &self.config.sidebar_style,
                sidebar_max_items: self.config.sidebar_max_items,
                title_prefix: &self.config.title_prefix,
                title_suffix: &self.config.title_suffix,
            },
            &page_context::UrlMode::RelativeToDepth(depth),
        );

        // Render through template (lock-free — uses pre-cloned Tera)
        let html_output =
            Templates::render_markdown_with_tera(tera, &html, frontmatter, extra_context)?;

        // Determine output path: url_path → build/{url_path}/index.html
        let url_path = info.url_path.trim_start_matches('/');
        let output_path = if url_path.is_empty() || url_path == "/" {
            self.output_dir.join("index.html")
        } else {
            self.output_dir.join(url_path).join("index.html")
        };

        // Create parent directories
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|e| BuildError::CreateDirFailed {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }

        // Write HTML file
        fs::write(&output_path, html_output).map_err(|e| BuildError::WriteFailed {
            path: output_path,
            source: e,
        })?;

        Ok(())
    }

    /// Generates directory/section pages in parallel.
    async fn render_directory_pages(&self) -> Result<usize, BuildError> {
        let stage_start = Instant::now();
        // Collect all directories that need section pages
        let mut directories: HashSet<PathBuf> = HashSet::new();

        // Add root directory
        directories.insert(PathBuf::new());

        // Add all parent directories of markdown files
        for (_, info) in self.repo.markdown_files.pin().iter() {
            let url_path = info.url_path.trim_start_matches('/').trim_end_matches('/');
            if !url_path.is_empty() {
                let mut current = PathBuf::new();
                for component in Path::new(url_path)
                    .parent()
                    .into_iter()
                    .flat_map(|p| p.components())
                {
                    if let std::path::Component::Normal(s) = component {
                        current.push(s);
                        directories.insert(current.clone());
                    }
                }
            }
        }

        let count = directories.len();
        let concurrency = self.get_concurrency();

        tracing::info!(
            "Rendering {} directory pages with concurrency {}",
            count,
            concurrency
        );

        // Pre-build a directory→children index so each section page looks up its
        // direct child files and immediate subdirectories in O(children) instead
        // of rescanning every markdown file (was O(dirs × files)). Mirrors the
        // `sibling_index` in `render_markdown_files`. File JSON keeps its
        // absolute `url_path`; each page relativizes + sorts its own slice, so
        // output ordering/behavior is preserved.
        let dir_index = Arc::new(build_dir_children_index(
            self.repo.markdown_files.pin().iter().map(|(_, info)| info),
        ));

        // Clone Tera once before entering the rayon pool to avoid per-file lock contention
        let tera_snapshot = self.templates.tera_clone();

        // Progress counter for parallel rendering
        let completed = Arc::new(AtomicUsize::new(0));
        print_progress("Generating sections", 0, count);

        // Convert HashSet to Vec for rayon iteration
        let directories: Vec<_> = directories.into_iter().collect();

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(concurrency)
            .build()
            .map_err(|e| BuildError::CreateDirFailed {
                path: self.output_dir.clone(),
                source: std::io::Error::other(format!("Failed to create rayon thread pool: {}", e)),
            })?;

        let error: FirstError<BuildError> = FirstError::new();

        pool.install(|| {
            use rayon::prelude::*;
            directories.par_iter().for_each(|dir| {
                if error.is_set() {
                    return;
                }
                match self.render_directory_page_sync(dir, &dir_index, &tera_snapshot) {
                    Ok(()) => {
                        let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                        if done.is_multiple_of(100) || done == count {
                            print_progress("Generating sections", done, count);
                        }
                    }
                    Err(e) => error.record(e),
                }
            });
        });

        error.into_result()?;

        print_stage_done("Generating sections", count, Some(stage_start.elapsed()));
        Ok(count)
    }

    /// Renders a single directory page synchronously with a pre-cloned Tera.
    ///
    /// `dir_index` maps each directory to its direct child files (as JSON with
    /// absolute `url_path`) and immediate subdirectory names, so this avoids a
    /// full rescan of the repository per directory.
    fn render_directory_page_sync(
        &self,
        relative_dir: &Path,
        dir_index: &DirChildrenIndex,
        tera: &tera::Tera,
    ) -> Result<(), BuildError> {
        let is_root = relative_dir.as_os_str().is_empty();

        // Calculate page depth for relative path generation
        let depth = if is_root {
            0
        } else {
            relative_dir.components().count()
        };

        // Build context for template
        let mut context: HashMap<String, serde_json::Value> = HashMap::new();

        // Breadcrumbs with relative URLs
        let breadcrumbs = generate_breadcrumbs(relative_dir);
        let breadcrumbs_json =
            page_context::breadcrumbs_to_json(&breadcrumbs, &UrlMode::RelativeToDepth(depth));
        context.insert(
            "breadcrumbs".to_string(),
            serde_json::Value::Array(breadcrumbs_json),
        );

        // Current directory name
        let current_dir_name = if is_root {
            "Home".to_string()
        } else {
            get_current_dir_name(relative_dir)
        };
        context.insert(
            "current_dir_name".to_string(),
            serde_json::Value::String(current_dir_name),
        );

        // Parent path (relative)
        if let Some(parent) = get_parent_path(relative_dir) {
            let relative_parent = make_relative_url(&parent, depth);
            context.insert(
                "parent_path".to_string(),
                serde_json::Value::String(relative_parent),
            );
        }

        // Collect files in this directory
        let dir_prefix = if is_root {
            "/".to_string()
        } else {
            format!("/{}/", relative_dir.to_string_lossy())
        };

        // Look up this directory's direct child files and immediate subdirs from
        // the pre-built index (O(children) rather than a full repo rescan).
        let empty_children: (Vec<serde_json::Value>, HashSet<String>) = Default::default();
        let (dir_files, dir_subdirs) = dir_index.get(relative_dir).unwrap_or(&empty_children);

        // Relativize each file's url_path for this page's depth.
        let mut files: Vec<serde_json::Value> = dir_files
            .iter()
            .map(|file_json| {
                let mut file_json = file_json.clone();
                if let Some(obj) = file_json.as_object_mut()
                    && let Some(abs_url) = obj.get("url_path").and_then(|v| v.as_str())
                {
                    obj.insert(
                        "url_path".to_string(),
                        serde_json::Value::String(make_relative_url(abs_url, depth)),
                    );
                }
                file_json
            })
            .collect();

        // Sort files using configurable sort order
        sort_files(&mut files, &self.config.sort);

        context.insert("files".to_string(), serde_json::Value::Array(files));

        // Convert subdirs to JSON array with name and relative url_path
        let subdirs_json: Vec<serde_json::Value> = dir_subdirs
            .iter()
            .map(|name| {
                let abs_url_path = if is_root {
                    format!("/{}/", name)
                } else {
                    format!("{}{}/", dir_prefix, name)
                };
                serde_json::json!({
                    "name": name,
                    "url_path": make_relative_url(&abs_url_path, depth)
                })
            })
            .collect();
        context.insert(
            "subdirs".to_string(),
            serde_json::Value::Array(subdirs_json),
        );

        // Pass tag_sources configuration for frontend (consistent with markdown pages)
        context.insert(
            "tag_sources".to_string(),
            serde_json::json!(page_context::tag_sources_json(&self.config.tag_sources)),
        );

        // Mode flags, sidebar navigation configuration, and title affixes
        page_context::insert_page_chrome(
            &mut context,
            &PageChrome {
                mode: ModeFlags::Static { depth },
                sidebar_style: &self.config.sidebar_style,
                sidebar_max_items: self.config.sidebar_max_items,
                title_affixes: Some((&self.config.title_prefix, &self.config.title_suffix)),
            },
        );

        // Render template (lock-free — uses pre-cloned Tera)
        let template_name = if is_root { "home.html" } else { "section.html" };
        let html_output = Templates::render_template_with_tera(tera, template_name, context)?;

        // Determine output path
        let output_path = if is_root {
            self.output_dir.join("index.html")
        } else {
            self.output_dir.join(relative_dir).join("index.html")
        };

        // Only write if file doesn't exist (markdown files take precedence)
        if !output_path.exists() {
            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(parent).map_err(|e| BuildError::CreateDirFailed {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }

            fs::write(&output_path, html_output).map_err(|e| BuildError::WriteFailed {
                path: output_path,
                source: e,
            })?;
        }

        Ok(())
    }

    /// Generates tag pages in parallel.
    ///
    /// For each configured tag source, generates:
    /// - A tag source index page at `/{source}/`
    /// - Individual tag pages at `/{source}/{value}/`
    async fn render_tag_pages(&self) -> Result<usize, BuildError> {
        let stage_start = Instant::now();
        // Collect all tag pages to render (source index + individual tags)
        let mut tasks: Vec<(String, Option<String>)> = Vec::new(); // (source, Some(value)) or (source, None for index)

        for tag_source in &self.config.tag_sources {
            let source = tag_source.url_source();

            // Check if this source has any tags
            if !self.repo.tag_index.has_source(&source) {
                continue;
            }

            // Add source index page
            tasks.push((source.clone(), None));

            // Add individual tag pages
            for tag_info in self.repo.tag_index.get_all_tags(&source) {
                tasks.push((source.clone(), Some(tag_info.normalized)));
            }
        }

        if tasks.is_empty() {
            println!("Generating tag pages ... skipped (no tags)");
            return Ok(0);
        }

        let count = tasks.len();
        let concurrency = self.get_concurrency();

        tracing::info!(
            "Rendering {} tag pages with concurrency {}",
            count,
            concurrency
        );

        // Clone Tera once before entering the rayon pool to avoid per-file lock contention
        let tera_snapshot = self.templates.tera_clone();

        // Progress counter for parallel rendering
        let completed = Arc::new(AtomicUsize::new(0));
        print_progress("Generating tag pages", 0, count);

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(concurrency)
            .build()
            .map_err(|e| BuildError::CreateDirFailed {
                path: self.output_dir.clone(),
                source: std::io::Error::other(format!("Failed to create rayon thread pool: {}", e)),
            })?;

        let error: FirstError<BuildError> = FirstError::new();

        pool.install(|| {
            use rayon::prelude::*;
            tasks.par_iter().for_each(|(source, value)| {
                if error.is_set() {
                    return;
                }
                let result = if let Some(tag_value) = value {
                    self.render_single_tag_page_sync(source, tag_value, &tera_snapshot)
                } else {
                    self.render_tag_source_index_sync(source, &tera_snapshot)
                };
                match result {
                    Ok(()) => {
                        let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                        if done.is_multiple_of(100) || done == count {
                            print_progress("Generating tag pages", done, count);
                        }
                    }
                    Err(e) => error.record(e),
                }
            });
        });

        error.into_result()?;

        print_stage_done("Generating tag pages", count, Some(stage_start.elapsed()));
        Ok(count)
    }

    /// Synchronous version of tag page rendering for use with rayon.
    /// Takes a pre-cloned `&Tera` to avoid lock contention.
    fn render_single_tag_page_sync(
        &self,
        source: &str,
        value: &str,
        tera: &tera::Tera,
    ) -> Result<(), BuildError> {
        let context = self.build_single_tag_page_context(source, value);
        let context = match context {
            Some(c) => c,
            None => return Ok(()),
        };

        let html_output = Templates::render_template_with_tera(tera, "tag.html", context.0)?;

        // Write file (context.1 = output_path)
        if !context.1.exists() {
            if let Some(parent) = context.1.parent() {
                fs::create_dir_all(parent).map_err(|e| BuildError::CreateDirFailed {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }
            fs::write(&context.1, html_output).map_err(|e| BuildError::WriteFailed {
                path: context.1,
                source: e,
            })?;
        }

        Ok(())
    }

    /// Synchronous version of tag source index rendering for use with rayon.
    fn render_tag_source_index_sync(
        &self,
        source: &str,
        tera: &tera::Tera,
    ) -> Result<(), BuildError> {
        let context = self.build_tag_source_index_context(source);
        let context = match context {
            Some(c) => c,
            None => return Ok(()),
        };

        let html_output = Templates::render_template_with_tera(tera, "tag_index.html", context.0)?;

        if !context.1.exists() {
            if let Some(parent) = context.1.parent() {
                fs::create_dir_all(parent).map_err(|e| BuildError::CreateDirFailed {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }
            fs::write(&context.1, html_output).map_err(|e| BuildError::WriteFailed {
                path: context.1,
                source: e,
            })?;
        }

        Ok(())
    }

    /// Builds context and output path for a single tag page.
    /// Returns None if the tag should be skipped (empty sanitized path).
    fn build_single_tag_page_context(
        &self,
        source: &str,
        value: &str,
    ) -> Option<(HashMap<String, serde_json::Value>, PathBuf)> {
        // Labels (fallback: raw source name)
        let (singular_label, plural_label) =
            page_context::tag_labels(&self.config.tag_sources, source, source);

        let display_value = self
            .repo
            .tag_index
            .get_tag_display(source, value)
            .unwrap_or_else(|| value.to_string());

        let pages = self.repo.tag_index.get_pages(source, value);
        let url_path = format!("/{}/{}/", source, value);
        let depth = url_depth(&url_path);

        let mut context: HashMap<String, serde_json::Value> = HashMap::new();
        page_context::insert_tag_page_keys(
            &mut context,
            source,
            &display_value,
            &singular_label,
            &plural_label,
            &pages,
            &UrlMode::RelativeToDepth(depth),
        );
        page_context::insert_page_chrome(
            &mut context,
            &PageChrome {
                mode: ModeFlags::Static { depth },
                sidebar_style: &self.config.sidebar_style,
                sidebar_max_items: self.config.sidebar_max_items,
                title_affixes: Some((&self.config.title_prefix, &self.config.title_suffix)),
            },
        );

        // Tag pages have synthetic breadcrumbs (Home > plural label)
        let breadcrumbs_json = vec![
            serde_json::json!({
                "name": "Home",
                "url": make_relative_url("/", depth)
            }),
            serde_json::json!({
                "name": plural_label,
                "url": make_relative_url(&format!("/{}/", source), depth)
            }),
        ];
        context.insert(
            "breadcrumbs".to_string(),
            serde_json::Value::Array(breadcrumbs_json),
        );
        context.insert(
            "current_dir_name".to_string(),
            serde_json::Value::String(display_value),
        );

        // Sanitize source and value to prevent path traversal
        let safe_source = crate::wikilink::sanitize_path_component(source);
        let safe_value = crate::wikilink::sanitize_path_component(value);
        if safe_source.is_empty() || safe_value.is_empty() {
            tracing::warn!(
                "Skipping tag page with empty sanitized source={source:?} value={value:?}"
            );
            return None;
        }

        let output_path = self
            .output_dir
            .join(&safe_source)
            .join(&safe_value)
            .join("index.html");

        if !output_path.starts_with(&self.output_dir) {
            tracing::warn!(
                "Tag page path escaped output directory: source={source:?} value={value:?}"
            );
            return None;
        }

        Some((context, output_path))
    }

    /// Builds context and output path for a tag source index page.
    /// Returns None if the source should be skipped (empty sanitized path).
    fn build_tag_source_index_context(
        &self,
        source: &str,
    ) -> Option<(HashMap<String, serde_json::Value>, PathBuf)> {
        // Labels (fallback: raw source name)
        let (singular_label, plural_label) =
            page_context::tag_labels(&self.config.tag_sources, source, source);

        let tags = self.repo.tag_index.get_all_tags(source);
        let url_path = format!("/{}/", source);
        let depth = url_depth(&url_path);

        let mut context: HashMap<String, serde_json::Value> = HashMap::new();
        page_context::insert_tag_index_keys(
            &mut context,
            source,
            &singular_label,
            &plural_label,
            &tags,
        );
        page_context::insert_page_chrome(
            &mut context,
            &PageChrome {
                mode: ModeFlags::Static { depth },
                sidebar_style: &self.config.sidebar_style,
                sidebar_max_items: self.config.sidebar_max_items,
                title_affixes: Some((&self.config.title_prefix, &self.config.title_suffix)),
            },
        );

        // Tag source index pages have synthetic breadcrumbs (Home only)
        let breadcrumbs_json = vec![serde_json::json!({
            "name": "Home",
            "url": make_relative_url("/", depth)
        })];
        context.insert(
            "breadcrumbs".to_string(),
            serde_json::Value::Array(breadcrumbs_json),
        );
        context.insert(
            "current_dir_name".to_string(),
            serde_json::Value::String(plural_label),
        );

        let safe_source = crate::wikilink::sanitize_path_component(source);
        if safe_source.is_empty() {
            tracing::warn!("Skipping tag source index with empty sanitized source={source:?}");
            return None;
        }

        let output_path = self.output_dir.join(&safe_source).join("index.html");

        if !output_path.starts_with(&self.output_dir) {
            tracing::warn!("Tag source index path escaped output directory: source={source:?}");
            return None;
        }

        Some((context, output_path))
    }

    /// Creates symlinks for static assets.
    fn symlink_assets(&self) -> Result<usize, BuildError> {
        let other_files: Vec<_> = self
            .repo
            .other_files
            .pin()
            .iter()
            .map(|(_, info)| info.clone())
            .collect();

        let count = other_files.len();

        for file_info in other_files {
            let url_path = file_info.url_path.trim_start_matches('/');
            let output_path = self.output_dir.join(url_path);

            // Create parent directories
            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(parent).map_err(|e| BuildError::CreateDirFailed {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }

            // Calculate relative path from output location to original file
            let target = self.calculate_relative_symlink(&output_path, &file_info.raw_path)?;

            // Create symlink (skip if already exists)
            if !output_path.exists() {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&target, &output_path).map_err(|e| {
                    BuildError::SymlinkFailed {
                        target: target.clone(),
                        link: output_path.clone(),
                        source: e,
                    }
                })?;
            }
        }

        Ok(count)
    }

    /// Calculates a relative path for symlinking.
    fn calculate_relative_symlink(&self, from: &Path, to: &Path) -> Result<PathBuf, BuildError> {
        // Get the directory containing the symlink
        let from_dir = from.parent().unwrap_or(from);

        // Calculate how many levels up we need to go
        let from_components: Vec<_> = from_dir.components().collect();
        let to_components: Vec<_> = to.components().collect();

        // Find common prefix length
        let common_len = from_components
            .iter()
            .zip(to_components.iter())
            .take_while(|(a, b)| a == b)
            .count();

        // Build the relative path
        let mut relative = PathBuf::new();

        // Add "../" for each level we need to go up
        for _ in common_len..from_components.len() {
            relative.push("..");
        }

        // Add the remaining path components from target
        for component in to_components.iter().skip(common_len) {
            relative.push(component.as_os_str());
        }

        Ok(relative)
    }

    /// Handles static folder overlay.
    fn handle_static_folder(&self) -> Result<(), BuildError> {
        let static_path = self.config.root_dir.join(&self.config.static_folder);

        if !static_path.exists() || !static_path.is_dir() {
            return Ok(());
        }

        for entry in WalkDir::new(&static_path)
            .follow_links(true)
            .min_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                let relative = entry.path().strip_prefix(&static_path).map_err(|_| {
                    BuildError::CreateDirFailed {
                        path: entry.path().to_path_buf(),
                        source: std::io::Error::other("strip prefix failed"),
                    }
                })?;

                let output_path = self.output_dir.join(relative);

                // Only symlink if path doesn't already exist (asset wins over static)
                if !output_path.exists() {
                    if let Some(parent) = output_path.parent() {
                        fs::create_dir_all(parent).map_err(|e| BuildError::CreateDirFailed {
                            path: parent.to_path_buf(),
                            source: e,
                        })?;
                    }

                    let target = self.calculate_relative_symlink(&output_path, entry.path())?;

                    #[cfg(unix)]
                    std::os::unix::fs::symlink(&target, &output_path).map_err(|e| {
                        BuildError::SymlinkFailed {
                            target,
                            link: output_path,
                            source: e,
                        }
                    })?;
                }
            }
        }

        Ok(())
    }

    /// Handles .mbr folder: copy user files, write defaults, generate site.json.
    fn handle_mbr_folder(&self) -> Result<(), BuildError> {
        let mbr_output = self.output_dir.join(".mbr");

        // Step 1: Create .mbr directory
        fs::create_dir_all(&mbr_output).map_err(|e| BuildError::CreateDirFailed {
            path: mbr_output.clone(),
            source: e,
        })?;

        // Write .nojekyll so GitHub Pages serves dotfolders (like .mbr/)
        let nojekyll_path = self.output_dir.join(".nojekyll");
        fs::write(&nojekyll_path, "").map_err(|e| BuildError::WriteFailed {
            path: nojekyll_path,
            source: e,
        })?;

        // Step 2: Copy repo's .mbr folder if it exists
        let mbr_source = self.config.root_dir.join(".mbr");
        if mbr_source.exists() && mbr_source.is_dir() {
            self.copy_dir_recursive(&mbr_source, &mbr_output)?;
        }

        // Step 3: Write DEFAULT_FILES using route names (skip if file exists)
        for (route, content, _mime_type) in DEFAULT_FILES.iter() {
            // Skip empty files (like /user.css)
            if content.is_empty() {
                continue;
            }

            // Skip pico.min.css - we'll write the themed version separately
            if *route == "/pico.min.css" {
                continue;
            }

            // Strip leading / from route to get filename
            let filename = route.trim_start_matches('/');
            let output_path = mbr_output.join(filename);

            // Only write if file doesn't already exist (repo's .mbr/ wins)
            if !output_path.exists() {
                // Create parent directories for nested paths (e.g., components/mbr-components.js)
                if let Some(parent) = output_path.parent() {
                    fs::create_dir_all(parent).map_err(|e| BuildError::CreateDirFailed {
                        path: parent.to_path_buf(),
                        source: e,
                    })?;
                }

                fs::write(&output_path, content).map_err(|e| BuildError::WriteFailed {
                    path: output_path,
                    source: e,
                })?;
            }
        }

        // Step 3b: Write themed pico.min.css (only if not already present from repo's .mbr/)
        let pico_output_path = mbr_output.join("pico.min.css");
        if !pico_output_path.exists() {
            let pico_content = match embedded_pico::get_pico_css(&self.config.theme) {
                Some(content) => content,
                None => {
                    eprintln!(
                        "Warning: Invalid theme '{}'. Using default. Valid themes: {}",
                        self.config.theme,
                        embedded_pico::valid_themes_display()
                    );
                    embedded_pico::get_pico_css("default").ok_or(BuildError::MissingDefaultTheme)?
                }
            };
            fs::write(&pico_output_path, pico_content).map_err(|e| BuildError::WriteFailed {
                path: pico_output_path,
                source: e,
            })?;
        }

        // Step 4: Generate site.json with sort config and tags
        let mut response = serde_json::to_value(&self.repo)
            .map_err(|e| BuildError::RepoScan(crate::errors::RepoError::JsonSerializeFailed(e)))?;

        // Add sort config and tags to the response
        if let Some(obj) = response.as_object_mut() {
            obj.insert(
                "sort".to_string(),
                serde_json::to_value(&self.config.sort).unwrap_or(serde_json::Value::Array(vec![])),
            );

            // Add tag sources with their tags
            let mut tags_data: HashMap<String, serde_json::Value> = HashMap::new();
            for tag_source in &self.config.tag_sources {
                let source = tag_source.url_source();
                if self.repo.tag_index.has_source(&source) {
                    let tags = self.repo.tag_index.get_all_tags(&source);
                    let tags_json: Vec<serde_json::Value> = tags
                        .iter()
                        .map(|t| {
                            serde_json::json!({
                                "normalized": t.normalized,
                                "display": t.display,
                                "count": t.count,
                                "url": format!("/{}/{}/", source, t.normalized)
                            })
                        })
                        .collect();
                    tags_data.insert(
                        source,
                        serde_json::json!({
                            "label": tag_source.singular_label(),
                            "label_plural": tag_source.plural_label(),
                            "tags": tags_json
                        }),
                    );
                }
            }
            if !tags_data.is_empty() {
                obj.insert(
                    "tag_sources".to_string(),
                    serde_json::to_value(tags_data)
                        .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
                );
            }
        }

        // Add relationship_types + per-note resolved relationships (if enabled).
        if self.config.relationship_tracking {
            self.repo
                .relationship_index
                .inject_into_site_json(&mut response);
        }

        let site_json = serde_json::to_string(&response)
            .map_err(|e| BuildError::RepoScan(crate::errors::RepoError::JsonSerializeFailed(e)))?;
        let site_json_path = mbr_output.join("site.json");
        fs::write(&site_json_path, site_json).map_err(|e| BuildError::WriteFailed {
            path: site_json_path,
            source: e,
        })?;

        // Step 4b: Generate media.json with only other_files
        let media_data = serde_json::json!({
            "other_files": &self.repo.other_files,
        });
        let media_json = serde_json::to_string(&media_data)
            .map_err(|e| BuildError::RepoScan(crate::errors::RepoError::JsonSerializeFailed(e)))?;
        let media_json_path = mbr_output.join("media.json");
        fs::write(&media_json_path, media_json).map_err(|e| BuildError::WriteFailed {
            path: media_json_path,
            source: e,
        })?;

        Ok(())
    }

    /// Generates a 404.html error page at the root of the output directory.
    /// This is used by GitHub Pages and other hosts for custom 404 pages.
    fn generate_404_page(&self) -> Result<(), BuildError> {
        use std::collections::HashMap;

        let output_path = self.output_dir.join("404.html");

        // Build context for error template
        let mut context: HashMap<String, serde_json::Value> = HashMap::new();
        page_context::insert_error_keys(
            &mut context,
            404,
            "Not Found",
            Some("The requested page could not be found."),
        );

        // Static mode settings - 404.html is at root level (depth 0); error
        // pages omit title affixes
        page_context::insert_page_chrome(
            &mut context,
            &PageChrome {
                mode: ModeFlags::Static { depth: 0 },
                sidebar_style: &self.config.sidebar_style,
                sidebar_max_items: self.config.sidebar_max_items,
                title_affixes: None,
            },
        );

        // Empty breadcrumbs for error page
        context.insert(
            "breadcrumbs".to_string(),
            serde_json::Value::Array(vec![serde_json::json!({
                "name": "Home",
                "url": "./"
            })]),
        );

        let html = self.templates.render_error(context)?;

        fs::write(&output_path, html).map_err(|e| BuildError::WriteFailed {
            path: output_path,
            source: e,
        })?;

        Ok(())
    }

    /// Generates media viewer pages for videos, PDFs, and audio.
    ///
    /// Creates:
    /// - `.mbr/videos/index.html`
    /// - `.mbr/pdfs/index.html`
    /// - `.mbr/audio/index.html`
    ///
    /// These pages use client-side JavaScript to load the media based on
    /// a `?path=` query parameter. In static builds, the media viewer
    /// works entirely client-side.
    fn generate_media_viewer_pages(&self) -> Result<(), BuildError> {
        use std::collections::HashMap;

        let media_types = [
            MediaViewerType::Video,
            MediaViewerType::Pdf,
            MediaViewerType::Audio,
            MediaViewerType::Image,
        ];

        for media_type in media_types {
            // Determine output path based on media type
            let output_path = match media_type {
                MediaViewerType::Video => self.output_dir.join(".mbr/videos/index.html"),
                MediaViewerType::Pdf => self.output_dir.join(".mbr/pdfs/index.html"),
                MediaViewerType::Audio => self.output_dir.join(".mbr/audio/index.html"),
                MediaViewerType::Image => self.output_dir.join(".mbr/images/index.html"),
            };

            // Create parent directories
            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(parent).map_err(|e| BuildError::CreateDirFailed {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }

            // Build context for media viewer template
            // The page is at depth 2 from root (e.g., .mbr/videos/index.html)
            let depth = 2;
            let mut context: HashMap<String, serde_json::Value> = HashMap::new();

            // Media type specific context
            context.insert(
                "media_type".to_string(),
                serde_json::Value::String(media_type.as_str().to_string()),
            );
            context.insert(
                "title".to_string(),
                serde_json::Value::String(format!("{} Viewer", media_type.label())),
            );

            // Static mode settings, sidebar configuration, and title affixes
            page_context::insert_page_chrome(
                &mut context,
                &PageChrome {
                    mode: ModeFlags::Static { depth },
                    sidebar_style: &self.config.sidebar_style,
                    sidebar_max_items: self.config.sidebar_max_items,
                    title_affixes: Some((&self.config.title_prefix, &self.config.title_suffix)),
                },
            );

            // Breadcrumbs: Home link only (relative from depth 2)
            context.insert(
                "breadcrumbs".to_string(),
                serde_json::Value::Array(vec![serde_json::json!({
                    "name": "Home",
                    "url": "../../"
                })]),
            );

            // Parent path for back navigation (go to root)
            context.insert(
                "parent_path".to_string(),
                serde_json::Value::String("../../".to_string()),
            );

            let html = self.templates.render_media_viewer(context)?;

            fs::write(&output_path, html).map_err(|e| BuildError::WriteFailed {
                path: output_path,
                source: e,
            })?;
        }

        Ok(())
    }

    /// Runs Pagefind to generate the search index using the native Rust library.
    ///
    /// Returns true if Pagefind ran successfully, false otherwise.
    async fn run_pagefind(&self) -> bool {
        use pagefind::api::PagefindIndex;
        use pagefind::options::PagefindServiceConfig;

        // Create Pagefind index with default options
        let options = PagefindServiceConfig::builder()
            .force_language("en".to_string())
            .build();

        let mut index = match PagefindIndex::new(Some(options)) {
            Ok(idx) => idx,
            Err(e) => {
                tracing::warn!("Failed to create Pagefind index: {}", e);
                return false;
            }
        };

        // Use add_directory for parallel indexing via rayon (in pagefind's fossick_many)
        // The glob **/*.html naturally excludes .mbr/ since it has no HTML files
        let path = self.output_dir.to_string_lossy().to_string();
        let files_indexed = match index
            .add_directory(path, Some("**/*.html".to_string()))
            .await
        {
            Ok(count) => count,
            Err(e) => {
                tracing::warn!("Failed to index directory: {}", e);
                return false;
            }
        };

        if files_indexed == 0 {
            tracing::warn!("No HTML files found to index");
            return false;
        }

        // Get the generated index files
        let files = match index.get_files().await {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("Failed to get Pagefind files: {}", e);
                return false;
            }
        };

        // Write files to .mbr/pagefind/
        let pagefind_dir = self.output_dir.join(".mbr").join("pagefind");
        if let Err(e) = fs::create_dir_all(&pagefind_dir) {
            tracing::warn!("Failed to create pagefind directory: {}", e);
            return false;
        }

        for file in files {
            let file_path = pagefind_dir.join(&file.filename);

            // Create parent directories if needed
            if let Some(parent) = file_path.parent()
                && let Err(e) = fs::create_dir_all(parent)
            {
                tracing::debug!("Failed to create dir {}: {}", parent.display(), e);
                continue;
            }

            if let Err(e) = fs::write(&file_path, &file.contents) {
                tracing::debug!("Failed to write {}: {}", file_path.display(), e);
                continue;
            }
        }

        tracing::info!(
            "Pagefind search index generated: {} pages indexed",
            files_indexed
        );
        true
    }

    /// Validates internal links in all generated HTML files.
    ///
    /// Scans all HTML files for `<a href="...">` links, filters to internal links
    /// (excluding external URLs, mailto:, tel:, etc.), and checks if each link
    /// resolves to an existing file or directory in the output.
    ///
    /// Returns a list of broken links found.
    fn validate_links(&self) -> Vec<BrokenLink> {
        use rayon::prelude::*;

        // Create selector for anchor tags
        let selector = match Selector::parse("a[href]") {
            Ok(s) => s,
            Err(_) => return Vec::new(), // Should never fail with this simple selector
        };

        // Single WalkDir pass: build a HashSet of all files (for O(1) link lookups)
        // and collect HTML file paths (for parallel processing).
        let mut valid_files: HashSet<PathBuf> = HashSet::new();
        let mut html_files: Vec<PathBuf> = Vec::new();
        let mbr_prefix = self.output_dir.join(".mbr");

        for entry in WalkDir::new(&self.output_dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.into_path();
            valid_files.insert(path.clone());
            if path.extension().is_some_and(|ext| ext == "html") && !path.starts_with(&mbr_prefix) {
                html_files.push(path);
            }
        }

        // Process files in parallel: read, parse HTML, extract + validate links
        html_files
            .par_iter()
            .flat_map(|path| {
                let html_content = match fs::read_to_string(path) {
                    Ok(content) => content,
                    Err(_) => return Vec::new(),
                };

                let source_page = path
                    .strip_prefix(&self.output_dir)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();

                let document = Html::parse_document(&html_content);
                let mut broken = Vec::new();

                for element in document.select(&selector) {
                    if let Some(href) = element.value().attr("href") {
                        if href.starts_with("http://")
                            || href.starts_with("https://")
                            || href.starts_with("//")
                            || href.starts_with("mailto:")
                            || href.starts_with("tel:")
                            || href.starts_with("javascript:")
                            || href.starts_with("data:")
                            || href.starts_with("#")
                        {
                            continue;
                        }

                        if let Some(resolved) = self.resolve_link(path, href)
                            && !link_target_exists(&resolved, &valid_files)
                        {
                            broken.push(BrokenLink {
                                source_page: source_page.clone(),
                                link_url: href.to_string(),
                            });
                        }
                    }
                }

                broken
            })
            .collect()
    }

    /// Resolves a link URL relative to the source file's directory.
    ///
    /// Returns the absolute path within the output directory, or None if the link
    /// cannot be resolved.
    fn resolve_link(&self, source_file: &Path, href: &str) -> Option<PathBuf> {
        // Strip anchor from href (e.g., "/page/#section" -> "/page/")
        let href = href.split('#').next().unwrap_or(href);
        // Strip query string (e.g., "/page/?foo=bar" -> "/page/")
        let href = href.split('?').next().unwrap_or(href);

        if href.is_empty() {
            return None;
        }

        // URL-decode the href to handle percent-encoded characters (e.g., %20 -> space)
        // HTML links are percent-encoded by escape_href, but filesystem paths have literal characters
        let href = percent_decode_str(href).decode_utf8_lossy();

        if href.starts_with('/') {
            // Absolute path within site
            let path = href.trim_start_matches('/');
            Some(self.output_dir.join(path))
        } else {
            // Relative path - resolve from source file's parent directory
            let source_dir = source_file.parent()?;
            let resolved = source_dir.join(href.as_ref());
            // Normalize the path manually (handle ../ without requiring existence)
            Some(normalize_path(&resolved))
        }
    }

    /// Recursively copies a directory.
    fn copy_dir_recursive(&self, from: &Path, to: &Path) -> Result<(), BuildError> {
        for entry in WalkDir::new(from)
            .follow_links(true)
            .min_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let relative = entry
                .path()
                .strip_prefix(from)
                .map_err(|_| BuildError::CopyFailed {
                    from: entry.path().to_path_buf(),
                    to: to.to_path_buf(),
                    source: std::io::Error::other("strip prefix failed"),
                })?;

            let dest = to.join(relative);

            if entry.file_type().is_dir() {
                fs::create_dir_all(&dest).map_err(|e| BuildError::CreateDirFailed {
                    path: dest.clone(),
                    source: e,
                })?;
            } else if entry.file_type().is_file() {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).map_err(|e| BuildError::CreateDirFailed {
                        path: parent.to_path_buf(),
                        source: e,
                    })?;
                }
                fs::copy(entry.path(), &dest).map_err(|e| BuildError::CopyFailed {
                    from: entry.path().to_path_buf(),
                    to: dest.clone(),
                    source: e,
                })?;
            }
        }

        Ok(())
    }

    // ============================================================================
    // Tag page link helpers
    // ============================================================================

    /// Returns outbound links if url_path is a tag page, None otherwise.
    fn try_build_tag_outbound(&self, url_path: &str) -> Option<Vec<OutboundLink>> {
        let path = url_path.trim_matches('/');
        if path.is_empty() {
            return None;
        }

        let segments: Vec<&str> = path.split('/').collect();
        let tag_sources: Vec<String> = self
            .config
            .tag_sources
            .iter()
            .map(|ts| ts.url_source())
            .collect();

        match segments.len() {
            1 => {
                // Tag source index (e.g., "tags")
                let source = segments[0].to_lowercase();
                if tag_sources.contains(&source) {
                    Some(self.build_tag_index_outbound(&source))
                } else {
                    None
                }
            }
            2 => {
                // Tag page (e.g., "tags/rust")
                let source = segments[0].to_lowercase();
                let value = segments[1];
                if tag_sources.contains(&source) {
                    Some(self.build_tag_page_outbound(&source, value))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Builds outbound links for a tag page (e.g., /tags/rust/).
    fn build_tag_page_outbound(&self, source: &str, value: &str) -> Vec<OutboundLink> {
        let mut outbound = Vec::new();

        for page in self.repo.tag_index.get_pages(source, value) {
            outbound.push(OutboundLink {
                to: page.url_path,
                text: page.title,
                anchor: None,
                internal: true,
            });
        }

        // Link back to tag source index
        let label = self
            .config
            .tag_sources
            .iter()
            .find(|ts| ts.url_source() == source)
            .map(|ts| ts.plural_label())
            .unwrap_or_else(|| source.to_string());

        outbound.push(OutboundLink {
            to: format!("/{}/", source),
            text: label,
            anchor: None,
            internal: true,
        });

        outbound
    }

    /// Builds outbound links for a tag source index page (e.g., /tags/).
    fn build_tag_index_outbound(&self, source: &str) -> Vec<OutboundLink> {
        self.repo
            .tag_index
            .get_all_tags(source)
            .into_iter()
            .map(|tag| OutboundLink {
                to: format!("/{}/{}/", source, tag.normalized),
                text: tag.display,
                anchor: None,
                internal: true,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== FirstError Tests ====================

    #[test]
    fn test_first_error_starts_unset() {
        let error: FirstError<String> = FirstError::new();
        assert!(!error.is_set());
        assert!(error.into_result().is_ok());
    }

    #[test]
    fn test_first_error_records_only_first() {
        let error: FirstError<String> = FirstError::new();
        error.record("first".to_string());
        assert!(error.is_set());
        error.record("second".to_string());
        assert_eq!(error.into_result().unwrap_err(), "first");
    }

    #[test]
    fn test_first_error_records_across_threads() {
        let error: FirstError<usize> = FirstError::new();
        std::thread::scope(|s| {
            for i in 0..8 {
                let error = &error;
                s.spawn(move || error.record(i));
            }
        });
        // Exactly one of the recorded values must win and be reported.
        let value = error.into_result().unwrap_err();
        assert!(value < 8);
    }

    fn mk_info(url: &str) -> MarkdownInfo {
        MarkdownInfo {
            raw_path: PathBuf::from(url.trim_matches('/')),
            url_path: url.to_string(),
            frontmatter: None,
            created: 0,
            modified: 0,
            relationships: Vec::new(),
        }
    }

    /// Finding #18: the pre-built directory children index must produce the same
    /// direct-child files and immediate subdirectories, for every directory, as
    /// the previous per-directory full scan over all markdown files.
    #[test]
    fn test_build_dir_children_index_matches_full_scan() {
        let infos = vec![
            mk_info("/readme/"),
            mk_info("/docs/guide/"),
            mk_info("/docs/intro/"),
            mk_info("/docs/api/reference/"),
            mk_info("/blog/2024/post/"),
        ];

        let index = build_dir_children_index(infos.iter());

        // Every directory that appears in the index (plus root) must match the
        // old scan for both its direct child files and immediate subdirs.
        for dir in index.keys() {
            let is_root = dir.as_os_str().is_empty();
            let dir_prefix = if is_root {
                "/".to_string()
            } else {
                format!("/{}/", dir.to_string_lossy())
            };

            // Reference (old) full-scan logic.
            let mut expected_files: Vec<serde_json::Value> = Vec::new();
            let mut expected_subdirs: HashSet<String> = HashSet::new();
            for info in &infos {
                let url_path = &info.url_path;
                if url_path.starts_with(&dir_prefix) {
                    let remainder = url_path.strip_prefix(&dir_prefix).unwrap_or(url_path);
                    if !remainder.trim_end_matches('/').contains('/') {
                        expected_files.push(markdown_file_to_json(info));
                    } else if let Some(subdir) = remainder.split('/').next()
                        && !subdir.is_empty()
                    {
                        expected_subdirs.insert(subdir.to_string());
                    }
                }
            }

            let (got_files, got_subdirs) = index.get(dir).unwrap();
            // Files are order-independent here (each page sorts its own slice).
            let got_urls: HashSet<String> = got_files
                .iter()
                .map(|f| f["url_path"].as_str().unwrap().to_string())
                .collect();
            let expected_urls: HashSet<String> = expected_files
                .iter()
                .map(|f| f["url_path"].as_str().unwrap().to_string())
                .collect();
            assert_eq!(got_urls, expected_urls, "files mismatch for dir {:?}", dir);
            assert_eq!(
                got_subdirs, &expected_subdirs,
                "subdirs mismatch for dir {:?}",
                dir
            );
        }

        // Spot-check specific directories.
        assert_eq!(index[&PathBuf::from("docs")].0.len(), 2); // guide, intro
        assert!(index[&PathBuf::from("docs")].1.contains("api"));
        assert!(index[&PathBuf::new()].1.contains("docs"));
        assert!(index[&PathBuf::new()].1.contains("blog"));
        assert_eq!(index[&PathBuf::new()].0.len(), 1); // readme
    }

    #[test]
    fn test_build_stats_default() {
        let stats = BuildStats::default();
        assert_eq!(stats.markdown_pages, 0);
        assert_eq!(stats.section_pages, 0);
        assert_eq!(stats.tag_pages, 0);
        assert_eq!(stats.assets_linked, 0);
        assert_eq!(stats.pagefind_indexed, None);
        assert_eq!(stats.broken_links, 0);
    }

    #[test]
    fn test_broken_link_struct() {
        let link = BrokenLink {
            source_page: "docs/index.html".to_string(),
            link_url: "../missing/".to_string(),
        };
        assert_eq!(link.source_page, "docs/index.html");
        assert_eq!(link.link_url, "../missing/");
    }

    #[test]
    fn test_normalize_path_simple() {
        let path = PathBuf::from("/foo/bar/baz");
        assert_eq!(normalize_path(&path), PathBuf::from("/foo/bar/baz"));
    }

    #[test]
    fn test_normalize_path_with_parent() {
        let path = PathBuf::from("/foo/bar/../baz");
        assert_eq!(normalize_path(&path), PathBuf::from("/foo/baz"));
    }

    #[test]
    fn test_normalize_path_with_multiple_parents() {
        let path = PathBuf::from("/foo/bar/qux/../../baz");
        assert_eq!(normalize_path(&path), PathBuf::from("/foo/baz"));
    }

    #[test]
    fn test_normalize_path_with_current_dir() {
        let path = PathBuf::from("/foo/./bar/./baz");
        assert_eq!(normalize_path(&path), PathBuf::from("/foo/bar/baz"));
    }

    #[test]
    fn test_normalize_path_mixed() {
        let path = PathBuf::from("/foo/./bar/../baz/./qux");
        assert_eq!(normalize_path(&path), PathBuf::from("/foo/baz/qux"));
    }

    #[test]
    fn test_url_depth_root() {
        assert_eq!(url_depth("/"), 0);
        assert_eq!(url_depth(""), 0);
    }

    #[test]
    fn test_url_depth_one_level() {
        assert_eq!(url_depth("/docs/"), 1);
        assert_eq!(url_depth("docs/"), 1);
        assert_eq!(url_depth("/docs"), 1);
    }

    #[test]
    fn test_url_depth_multiple_levels() {
        assert_eq!(url_depth("/docs/guide/"), 2);
        assert_eq!(url_depth("/a/b/c/"), 3);
        assert_eq!(url_depth("/a/b/c/d/e/"), 5);
    }

    #[test]
    fn test_relative_base_at_root() {
        assert_eq!(relative_base(0), ".mbr/");
    }

    #[test]
    fn test_relative_base_one_level() {
        assert_eq!(relative_base(1), "../.mbr/");
    }

    #[test]
    fn test_relative_base_multiple_levels() {
        assert_eq!(relative_base(2), "../../.mbr/");
        assert_eq!(relative_base(3), "../../../.mbr/");
    }

    #[test]
    fn test_relative_root_at_root() {
        assert_eq!(relative_root(0), "");
    }

    #[test]
    fn test_relative_root_one_level() {
        assert_eq!(relative_root(1), "../");
    }

    #[test]
    fn test_relative_root_multiple_levels() {
        assert_eq!(relative_root(2), "../../");
        assert_eq!(relative_root(3), "../../../");
    }

    // ============================================================================
    // Link Validation Tests
    // ============================================================================

    /// Helper to create a minimal Builder for testing link-related methods.
    pub(super) fn test_builder(output_dir: PathBuf, root_dir: PathBuf) -> Builder {
        use crate::config::Config;
        use crate::oembed_cache::OembedCache;
        use crate::repo::Repo;
        use crate::templates::Templates;
        use papaya::HashMap as ConcurrentHashMap;
        use std::sync::Arc;

        let config = Config {
            root_dir: root_dir.clone(),
            ..Default::default()
        };
        let templates =
            Templates::new(&root_dir, None).expect("Failed to create templates for test");
        let repo = Repo::init_from_config(&config);
        let oembed_cache = Arc::new(OembedCache::new(1024));
        let build_link_index = Arc::new(ConcurrentHashMap::new());
        let frontmatter_errors = Arc::new(ConcurrentHashMap::new());

        Builder {
            config,
            templates,
            output_dir,
            repo,
            oembed_cache,
            build_link_index,
            frontmatter_errors,
        }
    }

    // ---------------------- resolve_link tests ----------------------

    #[test]
    fn test_resolve_link_absolute_path() {
        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path().to_path_buf();
        let root = temp.path().join("root");
        std::fs::create_dir_all(root.join(".mbr")).unwrap();

        let builder = test_builder(temp_path.clone(), root);

        // Absolute path within site
        let source = temp_path.join("docs").join("index.html");
        let result = builder.resolve_link(&source, "/readme/");

        assert_eq!(result, Some(temp_path.join("readme/")));
    }

    #[test]
    fn test_resolve_link_relative_path() {
        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path().to_path_buf();
        let root = temp.path().join("root");
        std::fs::create_dir_all(root.join(".mbr")).unwrap();

        let builder = test_builder(temp_path.clone(), root);

        // Create source directory structure
        let docs_dir = temp_path.join("docs");
        std::fs::create_dir_all(&docs_dir).unwrap();
        let source = docs_dir.join("index.html");

        // Relative path - sibling
        let result = builder.resolve_link(&source, "guide/");
        assert_eq!(result, Some(docs_dir.join("guide")));

        // Relative path - parent
        let result = builder.resolve_link(&source, "../readme/");
        assert_eq!(result, Some(temp_path.join("readme")));
    }

    #[test]
    fn test_resolve_link_with_anchor() {
        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path().to_path_buf();
        let root = temp.path().join("root");
        std::fs::create_dir_all(root.join(".mbr")).unwrap();

        let builder = test_builder(temp_path.clone(), root);
        let source = temp_path.join("index.html");

        // Anchor should be stripped
        let result = builder.resolve_link(&source, "/docs/#section");
        assert_eq!(result, Some(temp_path.join("docs/")));

        // Just anchor returns None (empty after stripping)
        let result = builder.resolve_link(&source, "#section");
        // Note: "#section" starts with "#" so it's filtered in validate_links,
        // but resolve_link strips it to empty and returns None
        assert_eq!(result, None);
    }

    #[test]
    fn test_resolve_link_with_query_string() {
        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path().to_path_buf();
        let root = temp.path().join("root");
        std::fs::create_dir_all(root.join(".mbr")).unwrap();

        let builder = test_builder(temp_path.clone(), root);
        let source = temp_path.join("index.html");

        // Query string should be stripped
        let result = builder.resolve_link(&source, "/search/?q=test");
        assert_eq!(result, Some(temp_path.join("search/")));
    }

    #[test]
    fn test_resolve_link_url_encoded() {
        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path().to_path_buf();
        let root = temp.path().join("root");
        std::fs::create_dir_all(root.join(".mbr")).unwrap();

        let builder = test_builder(temp_path.clone(), root);
        let source = temp_path.join("index.html");

        // URL-encoded spaces should be decoded
        let result = builder.resolve_link(&source, "/my%20file/");
        assert_eq!(result, Some(temp_path.join("my file/")));
    }

    #[test]
    fn test_resolve_link_empty() {
        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path().to_path_buf();
        let root = temp.path().join("root");
        std::fs::create_dir_all(root.join(".mbr")).unwrap();

        let builder = test_builder(temp_path, root);
        let source = PathBuf::from("/some/source.html");

        // Empty href returns None
        let result = builder.resolve_link(&source, "");
        assert_eq!(result, None);
    }

    // ---------------------- link_target_exists tests ----------------------

    /// Helper to build a HashSet of valid files from a directory (mirrors validate_links logic).
    fn build_valid_files(dir: &Path) -> HashSet<PathBuf> {
        WalkDir::new(dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.into_path())
            .collect()
    }

    #[test]
    fn test_link_target_exists_file() {
        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path().to_path_buf();

        // Create a file
        let file_path = temp_path.join("readme.html");
        std::fs::write(&file_path, "content").unwrap();

        let valid_files = build_valid_files(&temp_path);
        assert!(link_target_exists(&file_path, &valid_files));
    }

    #[test]
    fn test_link_target_exists_directory_with_index() {
        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path().to_path_buf();

        // Create a directory with index.html
        let dir_path = temp_path.join("docs");
        std::fs::create_dir_all(&dir_path).unwrap();
        std::fs::write(dir_path.join("index.html"), "content").unwrap();

        let valid_files = build_valid_files(&temp_path);
        assert!(link_target_exists(&dir_path, &valid_files));
    }

    #[test]
    fn test_link_target_exists_directory_without_index() {
        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path().to_path_buf();

        // Create a directory without index.html
        let dir_path = temp_path.join("docs");
        std::fs::create_dir_all(&dir_path).unwrap();

        let valid_files = build_valid_files(&temp_path);
        // Directory exists but has no index.html, so returns false
        // This is important because the link indexer creates directories
        // with just links.json for pages that don't exist
        assert!(!link_target_exists(&dir_path, &valid_files));
    }

    #[test]
    fn test_link_target_exists_missing() {
        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path().to_path_buf();

        let valid_files = build_valid_files(&temp_path);

        // Non-existent path
        let missing = temp_path.join("nonexistent");
        assert!(!link_target_exists(&missing, &valid_files));
    }

    #[test]
    fn test_link_target_exists_path_with_trailing_slash() {
        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path().to_path_buf();

        // Create a directory with index.html
        let dir_path = temp_path.join("docs");
        std::fs::create_dir_all(&dir_path).unwrap();
        std::fs::write(dir_path.join("index.html"), "content").unwrap();

        let valid_files = build_valid_files(&temp_path);

        // Path with trailing slash should check for index.html
        let path_with_slash = temp_path.join("docs/");
        assert!(link_target_exists(&path_with_slash, &valid_files));

        // Non-existent directory with trailing slash
        let missing_with_slash = temp_path.join("missing/");
        assert!(!link_target_exists(&missing_with_slash, &valid_files));
    }

    // ---------------------- validate_links tests ----------------------

    #[test]
    fn test_validate_links_skips_external() {
        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path().to_path_buf();
        let root = temp.path().join("root");
        std::fs::create_dir_all(root.join(".mbr")).unwrap();

        // Create HTML with external links
        let html_path = temp_path.join("test.html");
        std::fs::write(
            &html_path,
            r##"<html><body>
            <a href="https://example.com">External HTTPS</a>
            <a href="http://example.com">External HTTP</a>
            <a href="//cdn.example.com">Protocol-relative</a>
            <a href="mailto:test@example.com">Email</a>
            <a href="tel:+1234567890">Phone</a>
            <a href="javascript:void(0)">JavaScript</a>
            <a href="data:text/html,Hello">Data URI</a>
            <a href="#section">Anchor</a>
        </body></html>"##,
        )
        .unwrap();

        let builder = test_builder(temp_path, root);
        let broken = builder.validate_links();

        // All links should be skipped (external/special protocols)
        assert!(
            broken.is_empty(),
            "Expected no broken links, got: {:?}",
            broken
        );
    }

    #[test]
    fn test_validate_links_finds_broken() {
        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path().to_path_buf();
        let root = temp.path().join("root");
        std::fs::create_dir_all(root.join(".mbr")).unwrap();

        // Create HTML with broken internal link
        let html_path = temp_path.join("test.html");
        std::fs::write(
            &html_path,
            r#"<html><body>
            <a href="/nonexistent/">Broken link</a>
        </body></html>"#,
        )
        .unwrap();

        let builder = test_builder(temp_path, root);
        let broken = builder.validate_links();

        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].link_url, "/nonexistent/");
    }

    #[test]
    fn test_validate_links_valid_links() {
        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path().to_path_buf();
        let root = temp.path().join("root");
        std::fs::create_dir_all(root.join(".mbr")).unwrap();

        // Create target directory with index.html
        let docs_dir = temp_path.join("docs");
        std::fs::create_dir_all(&docs_dir).unwrap();
        std::fs::write(docs_dir.join("index.html"), "content").unwrap();

        // Create HTML with valid internal link
        let html_path = temp_path.join("test.html");
        std::fs::write(
            &html_path,
            r#"<html><body>
            <a href="/docs/">Valid link</a>
        </body></html>"#,
        )
        .unwrap();

        let builder = test_builder(temp_path, root);
        let broken = builder.validate_links();

        assert!(
            broken.is_empty(),
            "Expected no broken links, got: {:?}",
            broken
        );
    }

    #[test]
    fn test_validate_links_skips_mbr_directory() {
        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path().to_path_buf();
        let root = temp.path().join("root");
        std::fs::create_dir_all(root.join(".mbr")).unwrap();

        // Create .mbr directory with HTML containing broken links
        let mbr_dir = temp_path.join(".mbr");
        std::fs::create_dir_all(&mbr_dir).unwrap();
        std::fs::write(
            mbr_dir.join("pagefind-ui.html"),
            r#"<html><body><a href="/broken/">Broken</a></body></html>"#,
        )
        .unwrap();

        let builder = test_builder(temp_path, root);
        let broken = builder.validate_links();

        // Should skip .mbr directory
        assert!(broken.is_empty());
    }

    #[test]
    fn test_validate_links_relative_path() {
        let temp = tempfile::tempdir().unwrap();
        let temp_path = temp.path().to_path_buf();
        let root = temp.path().join("root");
        std::fs::create_dir_all(root.join(".mbr")).unwrap();

        // Create directory structure
        let docs_dir = temp_path.join("docs");
        std::fs::create_dir_all(&docs_dir).unwrap();

        // Create target
        let guide_dir = docs_dir.join("guide");
        std::fs::create_dir_all(&guide_dir).unwrap();
        std::fs::write(guide_dir.join("index.html"), "content").unwrap();

        // Create HTML with relative link
        std::fs::write(
            docs_dir.join("index.html"),
            r#"<html><body>
            <a href="guide/">Valid relative link</a>
            <a href="missing/">Broken relative link</a>
        </body></html>"#,
        )
        .unwrap();

        let builder = test_builder(temp_path, root);
        let broken = builder.validate_links();

        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].link_url, "missing/");
    }

    // ---------------------- calculate_relative_symlink tests ----------------------

    fn symlink_helper(from: &str, to: &str) -> PathBuf {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        std::fs::create_dir_all(&root).unwrap();
        let builder = test_builder(temp.path().to_path_buf(), root);
        builder
            .calculate_relative_symlink(Path::new(from), Path::new(to))
            .unwrap()
    }

    #[test]
    fn test_symlink_same_directory() {
        // Symlink and target in same directory
        let result = symlink_helper("/a/b/link", "/a/b/target");
        assert_eq!(result, PathBuf::from("target"));
    }

    #[test]
    fn test_symlink_parent_directory() {
        // Target is in parent directory of symlink
        let result = symlink_helper("/a/b/link", "/a/target");
        assert_eq!(result, PathBuf::from("../target"));
    }

    #[test]
    fn test_symlink_sibling_directory() {
        // Target is in sibling directory
        let result = symlink_helper("/a/b/link", "/a/c/target");
        assert_eq!(result, PathBuf::from("../c/target"));
    }

    #[test]
    fn test_symlink_deeply_nested_up() {
        // Target is many levels up
        let result = symlink_helper("/a/b/c/d/link", "/a/target");
        assert_eq!(result, PathBuf::from("../../../target"));
    }

    #[test]
    fn test_symlink_deeply_nested_both() {
        // Both symlink and target are deeply nested
        let result = symlink_helper("/a/b/c/link", "/a/x/y/z/target");
        assert_eq!(result, PathBuf::from("../../x/y/z/target"));
    }

    #[test]
    fn test_symlink_to_root_level() {
        // Target is at root level
        let result = symlink_helper("/a/b/c/link", "/target");
        assert_eq!(result, PathBuf::from("../../../target"));
    }

    #[test]
    fn test_symlink_from_root_level() {
        // Symlink is at root level (edge case)
        let result = symlink_helper("/link", "/a/b/target");
        assert_eq!(result, PathBuf::from("a/b/target"));
    }

    #[test]
    fn test_symlink_no_common_prefix() {
        // No common prefix beyond root
        let result = symlink_helper("/a/b/link", "/x/y/z/target");
        assert_eq!(result, PathBuf::from("../../x/y/z/target"));
    }

    #[test]
    fn test_symlink_same_path() {
        // Unusual: symlink pointing to itself (edge case)
        let result = symlink_helper("/a/b/link", "/a/b/link");
        assert_eq!(result, PathBuf::from("link"));
    }

    #[test]
    fn test_symlink_resolution_property() {
        // Property: from.parent().join(relative) normalized should equal to
        let from = PathBuf::from("/project/build/docs/images/link");
        let to = PathBuf::from("/project/source/assets/image.png");

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        std::fs::create_dir_all(&root).unwrap();
        let builder = test_builder(temp.path().to_path_buf(), root);
        let relative = builder.calculate_relative_symlink(&from, &to).unwrap();

        // Build the resolved path
        let from_dir = from.parent().unwrap();
        let resolved = from_dir.join(&relative);
        let normalized = normalize_path(&resolved);

        assert_eq!(normalized, to);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// Generate a reasonable path component (no special chars that break paths)
    fn path_component() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_]{0,10}".prop_map(|s| s.to_string())
    }

    /// Generate a path with 1-8 components
    fn reasonable_path() -> impl Strategy<Value = PathBuf> {
        prop::collection::vec(path_component(), 1..8).prop_map(|components| {
            let mut path = PathBuf::from("/");
            for c in components {
                path.push(c);
            }
            path
        })
    }

    proptest! {
        #[test]
        fn prop_symlink_resolution_correct(from in reasonable_path(), to in reasonable_path()) {
            // Skip if from has no parent (edge case we don't need to test)
            if from.parent().is_none() {
                return Ok(());
            }

            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().to_path_buf();
            std::fs::create_dir_all(&root).unwrap();
            let builder = super::tests::test_builder(temp.path().to_path_buf(), root);

            // Add a filename to 'from' to make it a symlink path
            let from_link = from.join("link");
            let relative = builder.calculate_relative_symlink(&from_link, &to).unwrap();

            // Resolve the path: start from from_link's parent, apply relative
            let from_dir = from_link.parent().unwrap();
            let resolved = from_dir.join(&relative);
            let normalized = normalize_path(&resolved);

            prop_assert_eq!(normalized, to.clone(),
                "from={:?}, to={:?}, relative={:?}, resolved={:?}",
                from_link, to, relative, resolved
            );
        }

        #[test]
        fn prop_symlink_no_absolute_in_result(from in reasonable_path(), to in reasonable_path()) {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().to_path_buf();
            std::fs::create_dir_all(&root).unwrap();
            let builder = super::tests::test_builder(temp.path().to_path_buf(), root);

            let from_link = from.join("link");
            let relative = builder.calculate_relative_symlink(&from_link, &to).unwrap();

            // Result should never be absolute
            prop_assert!(!relative.is_absolute(),
                "Relative symlink should not be absolute: {:?}", relative);
        }

        #[test]
        fn prop_symlink_starts_with_parent_or_component(from in reasonable_path(), to in reasonable_path()) {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().to_path_buf();
            std::fs::create_dir_all(&root).unwrap();
            let builder = super::tests::test_builder(temp.path().to_path_buf(), root);

            let from_link = from.join("link");
            let relative = builder.calculate_relative_symlink(&from_link, &to).unwrap();

            // First component should be ".." or a path component (never absolute)
            if let Some(first) = relative.components().next() {
                let is_parent_dir = matches!(first, std::path::Component::ParentDir);
                let is_normal = matches!(first, std::path::Component::Normal(_));
                prop_assert!(is_parent_dir || is_normal,
                    "First component should be '..' or normal: {:?}", first);
            }
        }
    }
}

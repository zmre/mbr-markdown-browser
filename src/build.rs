//! Static site generation module for mbr.
//!
//! Generates static HTML files from markdown, creating a deployable site.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use walkdir::WalkDir;

use scraper::{Html, Selector};

use crate::{
    config::Config,
    errors::BuildError,
    link_transform::LinkTransformConfig,
    markdown,
    repo::{MarkdownInfo, Repo},
    server::{generate_breadcrumbs, get_current_dir_name, get_parent_path, markdown_file_to_json, DEFAULT_FILES},
    templates::Templates,
};

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

/// Statistics from a build run.
#[derive(Debug, Default)]
pub struct BuildStats {
    pub markdown_pages: usize,
    pub section_pages: usize,
    pub assets_linked: usize,
    pub duration: Duration,
    /// Whether Pagefind search indexing succeeded (None = not attempted)
    pub pagefind_indexed: Option<bool>,
    /// Number of broken links detected
    pub broken_links: usize,
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
}

impl Builder {
    /// Creates a new Builder instance.
    pub fn new(config: Config, output_dir: PathBuf) -> Result<Self, BuildError> {
        let templates = Templates::new(&config.root_dir, config.template_folder.as_deref())?;
        let repo = Repo::init_from_config(&config);

        Ok(Builder {
            config,
            templates,
            output_dir,
            repo,
        })
    }

    /// Builds the static site.
    pub async fn build(&self) -> Result<BuildStats, BuildError> {
        let start = Instant::now();
        let mut stats = BuildStats::default();

        // Scan repository for all files
        self.repo.scan_all().map_err(|e| {
            crate::errors::RepoError::ScanFailed {
                path: self.config.root_dir.clone(),
                source: std::io::Error::other(e.to_string()),
            }
        })?;

        // Prepare output directory
        self.prepare_output_dir()?;

        // Render all markdown files
        stats.markdown_pages = self.render_markdown_files().await?;

        // Generate directory/section pages
        stats.section_pages = self.render_directory_pages().await?;

        // Symlink assets (images, PDFs, etc.)
        stats.assets_linked = self.symlink_assets()?;

        // Handle static folder overlay
        self.handle_static_folder()?;

        // Handle .mbr folder (copy, write defaults, generate site.json)
        self.handle_mbr_folder()?;

        // Validate internal links and report broken ones
        let broken_links = self.validate_links();
        stats.broken_links = broken_links.len();

        if !broken_links.is_empty() {
            eprintln!("\n⚠️  Broken links detected ({} total):", broken_links.len());
            for link in &broken_links {
                eprintln!("   {} → {}", link.source_page, link.link_url);
            }
            eprintln!();
        }

        // Run Pagefind to generate search index
        stats.pagefind_indexed = Some(self.run_pagefind().await);

        stats.duration = start.elapsed();
        Ok(stats)
    }

    /// Creates or cleans the output directory.
    fn prepare_output_dir(&self) -> Result<(), BuildError> {
        if self.output_dir.exists() {
            fs::remove_dir_all(&self.output_dir).map_err(|e| BuildError::CreateDirFailed {
                path: self.output_dir.clone(),
                source: e,
            })?;
        }
        fs::create_dir_all(&self.output_dir).map_err(|e| BuildError::CreateDirFailed {
            path: self.output_dir.clone(),
            source: e,
        })?;
        Ok(())
    }

    /// Renders all markdown files to HTML.
    async fn render_markdown_files(&self) -> Result<usize, BuildError> {
        let markdown_files: Vec<_> = self.repo.markdown_files.pin().iter()
            .map(|(path, info)| (path.clone(), info.clone()))
            .collect();

        let count = markdown_files.len();

        for (path, info) in markdown_files {
            self.render_single_markdown(&path, &info).await?;
        }

        Ok(count)
    }

    /// Renders a single markdown file.
    async fn render_single_markdown(&self, path: &Path, info: &MarkdownInfo) -> Result<(), BuildError> {
        // Determine if this is an index file (which doesn't need ../ prefix for links)
        let is_index_file = path
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(|f| f == self.config.index_file);

        let link_transform_config = LinkTransformConfig {
            markdown_extensions: self.config.markdown_extensions.clone(),
            index_file: self.config.index_file.clone(),
            is_index_file,
        };

        // Render markdown to HTML
        let (mut frontmatter, headings, html) = markdown::render(
            path.to_path_buf(),
            &self.config.root_dir,
            self.config.oembed_timeout_ms,
            link_transform_config,
        )
        .await
        .map_err(|e| BuildError::RenderFailed {
            path: path.to_path_buf(),
            source: Box::new(crate::MbrError::Io(std::io::Error::other(e.to_string()))),
        })?;

        // Add markdown_source to frontmatter
        frontmatter.insert(
            "markdown_source".to_string(),
            info.url_path.clone(),
        );

        // Indicate static mode (no dynamic search endpoint)
        // Empty string is falsy in Tera templates
        frontmatter.insert("server_mode".to_string(), String::new());

        // Compute breadcrumbs for the markdown file
        let url_path_for_breadcrumbs = std::path::Path::new(&info.url_path);
        let breadcrumbs = crate::server::generate_breadcrumbs(url_path_for_breadcrumbs);
        let breadcrumbs_json: Vec<_> = breadcrumbs
            .iter()
            .map(|b| serde_json::json!({"name": b.name, "url": b.url}))
            .collect();
        let current_dir_name = crate::server::get_current_dir_name(url_path_for_breadcrumbs);

        let mut extra_context = std::collections::HashMap::new();
        extra_context.insert("breadcrumbs".to_string(), serde_json::json!(breadcrumbs_json));
        extra_context.insert("current_dir_name".to_string(), serde_json::json!(current_dir_name));
        extra_context.insert("headings".to_string(), serde_json::json!(headings));

        // Render through template
        let html_output = self.templates.render_markdown(&html, frontmatter, extra_context).await?;

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

    /// Generates directory/section pages.
    async fn render_directory_pages(&self) -> Result<usize, BuildError> {
        // Collect all directories that need section pages
        let mut directories: HashSet<PathBuf> = HashSet::new();

        // Add root directory
        directories.insert(PathBuf::new());

        // Add all parent directories of markdown files
        for (_, info) in self.repo.markdown_files.pin().iter() {
            let url_path = info.url_path.trim_start_matches('/').trim_end_matches('/');
            if !url_path.is_empty() {
                let mut current = PathBuf::new();
                for component in Path::new(url_path).parent().into_iter().flat_map(|p| p.components()) {
                    if let std::path::Component::Normal(s) = component {
                        current.push(s);
                        directories.insert(current.clone());
                    }
                }
            }
        }

        let count = directories.len();

        for dir in directories {
            self.render_directory_page(&dir).await?;
        }

        Ok(count)
    }

    /// Renders a single directory page.
    async fn render_directory_page(&self, relative_dir: &Path) -> Result<(), BuildError> {
        let is_root = relative_dir.as_os_str().is_empty();

        // Build context for template
        let mut context: HashMap<String, serde_json::Value> = HashMap::new();

        // Breadcrumbs
        let breadcrumbs = generate_breadcrumbs(relative_dir);
        context.insert("breadcrumbs".to_string(), serde_json::to_value(&breadcrumbs).unwrap_or_default());

        // Current directory name
        let current_dir_name = if is_root {
            "Home".to_string()
        } else {
            get_current_dir_name(relative_dir)
        };
        context.insert("current_dir_name".to_string(), serde_json::Value::String(current_dir_name));

        // Parent path
        if let Some(parent) = get_parent_path(relative_dir) {
            context.insert("parent_path".to_string(), serde_json::Value::String(parent));
        }

        // Collect files in this directory
        let dir_prefix = if is_root {
            "/".to_string()
        } else {
            format!("/{}/", relative_dir.to_string_lossy())
        };

        let mut files: Vec<serde_json::Value> = Vec::new();
        let mut subdirs: HashSet<String> = HashSet::new();

        for (_, info) in self.repo.markdown_files.pin().iter() {
            let url_path = &info.url_path;

            // Check if this file is in the current directory
            if url_path.starts_with(&dir_prefix) {
                let remainder = url_path.strip_prefix(&dir_prefix).unwrap_or(url_path);

                // If there's no / in remainder, it's a direct child
                if !remainder.trim_end_matches('/').contains('/') {
                    files.push(markdown_file_to_json(info));
                } else if let Some(subdir) = remainder.split('/').next()
                    && !subdir.is_empty()
                {
                    subdirs.insert(subdir.to_string());
                }
            }
        }

        // Sort files by title
        files.sort_by(|a, b| {
            let title_a = a.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let title_b = b.get("title").and_then(|v| v.as_str()).unwrap_or("");
            title_a.cmp(title_b)
        });

        context.insert("files".to_string(), serde_json::Value::Array(files));

        // Convert subdirs to JSON array with name and url_path
        let subdirs_json: Vec<serde_json::Value> = subdirs
            .into_iter()
            .map(|name| {
                let url_path = if is_root {
                    format!("/{}/", name)
                } else {
                    format!("{}{}/", dir_prefix, name)
                };
                serde_json::json!({
                    "name": name,
                    "url_path": url_path
                })
            })
            .collect();
        context.insert("subdirs".to_string(), serde_json::Value::Array(subdirs_json));

        // Indicate static mode (no dynamic search endpoint)
        context.insert("server_mode".to_string(), serde_json::Value::Bool(false));

        // Render template
        let html_output = if is_root {
            self.templates.render_home(context).await?
        } else {
            self.templates.render_section(context).await?
        };

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

    /// Creates symlinks for static assets.
    fn symlink_assets(&self) -> Result<usize, BuildError> {
        let other_files: Vec<_> = self.repo.other_files.pin().iter()
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
                std::os::unix::fs::symlink(&target, &output_path).map_err(|e| BuildError::SymlinkFailed {
                    target: target.clone(),
                    link: output_path.clone(),
                    source: e,
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
        let common_len = from_components.iter()
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
                let relative = entry.path().strip_prefix(&static_path)
                    .map_err(|_| BuildError::CreateDirFailed {
                        path: entry.path().to_path_buf(),
                        source: std::io::Error::other("strip prefix failed"),
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
                    std::os::unix::fs::symlink(&target, &output_path).map_err(|e| BuildError::SymlinkFailed {
                        target,
                        link: output_path,
                        source: e,
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

        // Step 4: Generate site.json
        let site_json = self.repo.to_json().map_err(|e| BuildError::RepoScan(
            crate::errors::RepoError::JsonSerializeFailed(e)
        ))?;
        let site_json_path = mbr_output.join("site.json");
        fs::write(&site_json_path, site_json).map_err(|e| BuildError::WriteFailed {
            path: site_json_path,
            source: e,
        })?;

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

        // Walk through all HTML files in output directory
        let mut files_indexed = 0;
        for entry in WalkDir::new(&self.output_dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "html"))
        {
            let path = entry.path();

            // Skip .mbr directory
            if path.starts_with(self.output_dir.join(".mbr")) {
                continue;
            }

            // Read HTML content
            let html_content = match fs::read_to_string(path) {
                Ok(content) => content,
                Err(e) => {
                    tracing::debug!("Failed to read {}: {}", path.display(), e);
                    continue;
                }
            };

            // Calculate URL path from file path
            let relative_path = path.strip_prefix(&self.output_dir).unwrap_or(path);
            let url_path = format!("/{}", relative_path.display())
                .replace("/index.html", "/")
                .replace("\\", "/");

            // Add to index - parameters are: source_path, url, content
            // We use url (2nd param) since we have the explicit URL path
            match index.add_html_file(
                None,
                Some(url_path.clone()),
                html_content,
            ).await {
                Ok(_) => {
                    files_indexed += 1;
                    tracing::debug!("Indexed: {}", url_path);
                }
                Err(e) => {
                    tracing::debug!("Failed to index {}: {}", url_path, e);
                }
            }
        }

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

        tracing::info!("Pagefind search index generated: {} pages indexed", files_indexed);
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
        let mut broken_links = Vec::new();

        // Create selector for anchor tags
        let selector = match Selector::parse("a[href]") {
            Ok(s) => s,
            Err(_) => return broken_links, // Should never fail with this simple selector
        };

        // Walk through all HTML files in output directory
        for entry in WalkDir::new(&self.output_dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "html"))
        {
            let path = entry.path();

            // Skip .mbr directory (Pagefind UI, etc.)
            if path.starts_with(self.output_dir.join(".mbr")) {
                continue;
            }

            // Read HTML content
            let html_content = match fs::read_to_string(path) {
                Ok(content) => content,
                Err(_) => continue,
            };

            // Calculate source page path for error reporting
            let source_page = path
                .strip_prefix(&self.output_dir)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();

            // Parse HTML and find all links
            let document = Html::parse_document(&html_content);

            for element in document.select(&selector) {
                if let Some(href) = element.value().attr("href") {
                    // Skip external links and special protocols
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

                    // Resolve the link relative to the current file's directory
                    if let Some(resolved) = self.resolve_link(path, href)
                        && !self.link_target_exists(&resolved)
                    {
                        broken_links.push(BrokenLink {
                            source_page: source_page.clone(),
                            link_url: href.to_string(),
                        });
                    }
                }
            }
        }

        broken_links
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

        if href.starts_with('/') {
            // Absolute path within site
            let path = href.trim_start_matches('/');
            Some(self.output_dir.join(path))
        } else {
            // Relative path - resolve from source file's parent directory
            let source_dir = source_file.parent()?;
            let resolved = source_dir.join(href);
            // Normalize the path manually (handle ../ without requiring existence)
            Some(normalize_path(&resolved))
        }
    }

    /// Checks if a link target exists in the output directory.
    ///
    /// Handles both files and directories (checking for index.html in directories).
    fn link_target_exists(&self, path: &Path) -> bool {
        if path.exists() {
            return true;
        }

        // If path ends with /, check for index.html
        let path_str = path.to_string_lossy();
        if path_str.ends_with('/') || path_str.ends_with(std::path::MAIN_SEPARATOR) {
            return path.join("index.html").exists();
        }

        // Check if it's a directory with index.html
        if path.is_dir() {
            return path.join("index.html").exists();
        }

        // Try adding index.html for directory-style paths
        let with_index = path.join("index.html");
        if with_index.exists() {
            return true;
        }

        false
    }

    /// Recursively copies a directory.
    fn copy_dir_recursive(&self, from: &Path, to: &Path) -> Result<(), BuildError> {
        for entry in WalkDir::new(from)
            .follow_links(true)
            .min_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let relative = entry.path().strip_prefix(from)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_stats_default() {
        let stats = BuildStats::default();
        assert_eq!(stats.markdown_pages, 0);
        assert_eq!(stats.section_pages, 0);
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
}

//! Grep-based inbound link discovery for server mode.
//!
//! This module provides fast, on-demand discovery of pages that link to a given page
//! by searching through all markdown files in the repository.

use papaya::HashMap as ConcurrentHashMap;
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use walkdir::WalkDir;

use crate::link_index::InboundLink;
use crate::repo::should_ignore;

/// Result of scanning for inbound links to a page.
#[derive(Clone)]
struct InboundLinkCacheEntry {
    /// Links pointing to this page from other pages
    links: Vec<InboundLink>,
    /// When this entry was computed
    computed_at: std::time::Instant,
    /// Estimated memory size
    size_bytes: usize,
}

/// Cache for inbound link grep results.
///
/// Since grep operations can be slow for large repositories, we cache the results
/// and invalidate on a time-based basis (results become stale after a period).
pub struct InboundLinkCache {
    /// Cached grep results (target_url_path -> inbound links)
    cache: ConcurrentHashMap<String, InboundLinkCacheEntry>,
    /// Current total size in bytes
    current_size: AtomicUsize,
    /// Maximum allowed size in bytes
    max_size: usize,
    /// How long entries stay valid (in seconds)
    ttl_seconds: u64,
}

impl InboundLinkCache {
    /// Creates a new cache with the specified maximum size and TTL.
    pub fn new(max_size_bytes: usize, ttl_seconds: u64) -> Self {
        Self {
            cache: ConcurrentHashMap::new(),
            current_size: AtomicUsize::new(0),
            max_size: max_size_bytes,
            ttl_seconds,
        }
    }

    /// Gets cached inbound links for a page, if still valid.
    pub fn get(&self, url_path: &str) -> Option<Vec<InboundLink>> {
        if self.max_size == 0 {
            return None;
        }

        let guard = self.cache.pin();
        if let Some(entry) = guard.get(url_path) {
            // Check TTL
            if entry.computed_at.elapsed().as_secs() < self.ttl_seconds {
                tracing::debug!("inbound link cache hit: {}", url_path);
                return Some(entry.links.clone());
            } else {
                tracing::debug!("inbound link cache expired: {}", url_path);
            }
        }
        None
    }

    /// Inserts inbound links into the cache.
    pub fn insert(&self, url_path: String, links: Vec<InboundLink>) {
        if self.max_size == 0 {
            return;
        }

        let size_bytes = url_path.len()
            + links
                .iter()
                .map(|l| {
                    l.from.len()
                        + l.text.len()
                        + l.anchor.as_ref().map(|a| a.len()).unwrap_or(0)
                        + 32
                })
                .sum::<usize>()
            + std::mem::size_of::<InboundLinkCacheEntry>();

        let entry = InboundLinkCacheEntry {
            links,
            computed_at: Instant::now(),
            size_bytes,
        };

        self.cache.pin().insert(url_path.clone(), entry);
        let new_size = self.current_size.fetch_add(size_bytes, Ordering::Relaxed) + size_bytes;

        tracing::debug!("inbound links cached: {} ({} bytes)", url_path, size_bytes);

        // Evict if over limit
        if new_size > self.max_size {
            self.evict_oldest(new_size - self.max_size);
        }
    }

    /// Evicts oldest entries until at least `target_bytes` have been freed.
    fn evict_oldest(&self, target_bytes: usize) {
        let guard = self.cache.pin();
        let mut entries: Vec<(String, Instant, usize)> = guard
            .iter()
            .map(|(k, v)| (k.clone(), v.computed_at, v.size_bytes))
            .collect();

        // Sort by computation time (oldest first)
        entries.sort_by_key(|(_, computed_at, _)| *computed_at);

        let mut freed = 0usize;
        let mut evict_count = 0usize;

        for (url, _, size) in entries {
            if freed >= target_bytes {
                break;
            }
            if guard.remove(&url).is_some() {
                freed += size;
                evict_count += 1;
                self.current_size.fetch_sub(size, Ordering::Relaxed);
            }
        }

        if evict_count > 0 {
            tracing::debug!(
                "inbound link cache evicted {} entries ({} bytes freed)",
                evict_count,
                freed
            );
        }
    }

    /// Invalidates all cached entries (e.g., after file changes).
    pub fn invalidate_all(&self) {
        let guard = self.cache.pin();
        let keys: Vec<String> = guard.iter().map(|(k, _)| k.clone()).collect();
        for key in keys {
            guard.remove(&key);
        }
        self.current_size.store(0, Ordering::Relaxed);
        tracing::debug!("inbound link cache invalidated");
    }
}

/// Find all inbound links to a target page by grep-searching markdown files.
///
/// This scans all markdown files in the repository looking for links that point
/// to the target URL path. It extracts link text and anchor information.
///
/// # Arguments
/// * `target_url_path` - The URL path being linked to (e.g., "/docs/guide/")
/// * `root_dir` - Root directory of the markdown repository
/// * `markdown_extensions` - List of valid markdown file extensions
/// * `ignore_dirs` - Directories to skip during scanning
/// * `ignore_globs` - Glob patterns for files to ignore
///
/// # Returns
/// A vector of `InboundLink` structs representing pages that link to the target.
pub fn find_inbound_links(
    target_url_path: &str,
    root_dir: &Path,
    markdown_extensions: &[String],
    ignore_dirs: &[String],
    ignore_globs: &[String],
) -> Vec<InboundLink> {
    let start = Instant::now();
    let mut inbound_links = Vec::new();

    // Normalize target for matching
    let target_normalized = target_url_path.trim_end_matches('/');
    let target_with_slash = format!("{}/", target_normalized);

    // Extract path segments (strip leading slash for relative path matching)
    let target_segments = target_normalized.trim_start_matches('/');

    // Build regex patterns for different link syntaxes:
    // 1. Standard links: [text](url) or [text](url#anchor)
    // 2. Reference-style links: [text][ref] with [ref]: url
    // 3. Bare URLs (less common for internal links)

    // Pattern for inline links: [text](url)
    // Matches both absolute paths (/target/) and relative paths (target/, ../target/)
    let escaped_segments = regex::escape(target_segments);
    let link_pattern = format!(
        r#"\[([^\]]*)\]\((?:\.\.?/)*/?{}(?:/)?(?:#([^)]*))?\)"#,
        escaped_segments
    );

    let link_regex = match Regex::new(&link_pattern) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to compile link regex: {}", e);
            return inbound_links;
        }
    };

    // Pattern for wiki-style links: [[target]], [[target|text]], [[target#anchor]]
    // Case-insensitive to handle different casing conventions
    let wiki_pattern = format!(
        r#"(?i)\[\[(?:\.\.?/)*/?{}(?:\.md)?(?:/)?(?:#([^\]|]*))?(?:\|([^\]]*))?\]\]"#,
        escaped_segments
    );

    let wiki_regex = match Regex::new(&wiki_pattern) {
        Ok(r) => Some(r),
        Err(e) => {
            tracing::warn!("Failed to compile wiki link regex: {}", e);
            None
        }
    };

    // Walk through all markdown files
    let mut files_scanned = 0;
    for entry in WalkDir::new(root_dir)
        .follow_links(true)
        .into_iter()
        .filter_entry(|e| {
            let path = e.path();
            // Skip ignored directories
            if path.is_dir()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                return !ignore_dirs.contains(&name.to_string());
            }
            true
        })
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        // Skip non-files
        if !path.is_file() {
            continue;
        }

        // Check if it's a markdown file
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !markdown_extensions.contains(&extension) {
            continue;
        }

        // Skip ignored files
        if should_ignore(path, ignore_dirs, ignore_globs) {
            continue;
        }

        files_scanned += 1;

        // Read file content
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Compute source URL path from file path
        let source_url_path = compute_url_path(path, root_dir, markdown_extensions);

        // Skip if this is the target page itself
        if source_url_path.trim_end_matches('/') == target_normalized {
            continue;
        }

        // Search for links to the target
        for cap in link_regex.captures_iter(&content) {
            let text = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            let anchor = cap.get(2).map(|m| format!("#{}", m.as_str()));

            inbound_links.push(InboundLink {
                from: source_url_path.clone(),
                text: text.to_string(),
                anchor,
            });
        }

        // Search for wiki-style links: [[target]], [[target|display text]]
        if let Some(ref wiki_re) = wiki_regex {
            for cap in wiki_re.captures_iter(&content) {
                let anchor = cap.get(1).and_then(|m| {
                    let s = m.as_str();
                    if s.is_empty() {
                        None
                    } else {
                        Some(format!("#{}", s))
                    }
                });

                // Display text is after the pipe; if absent, use the target name
                let text = cap
                    .get(2)
                    .map(|m| m.as_str().trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        target_segments
                            .split('/')
                            .next_back()
                            .unwrap_or(target_segments)
                            .to_string()
                    });

                let link = InboundLink {
                    from: source_url_path.clone(),
                    text,
                    anchor,
                };

                if !inbound_links.contains(&link) {
                    inbound_links.push(link);
                }
            }
        }

        // Also check for reference-style links
        // [text][ref] ... [ref]: /target/path
        if content.contains(target_normalized)
            || content.contains(&target_with_slash)
            || content.contains(target_segments)
        {
            // Simple check: if the file contains the target path in a reference definition
            // Format: [ref]: /target/path or [ref]: /target/path/ or [ref]: /target/path#anchor
            let ref_pattern = format!(
                r#"\[([^\]]+)\]:\s*(?:\.\.?/)*/?{}(?:/)?(?:#\S*)?"#,
                escaped_segments
            );
            if let Ok(ref_regex) = Regex::new(&ref_pattern) {
                for cap in ref_regex.captures_iter(&content) {
                    let ref_name = cap.get(1).map(|m| m.as_str()).unwrap_or("");

                    // Now find uses of this reference: [text][ref_name]
                    let use_pattern = format!(r#"\[([^\]]*)\]\[{}\]"#, regex::escape(ref_name));
                    if let Ok(use_regex) = Regex::new(&use_pattern) {
                        for use_cap in use_regex.captures_iter(&content) {
                            let text = use_cap.get(1).map(|m| m.as_str()).unwrap_or("");

                            // Avoid duplicates
                            let link = InboundLink {
                                from: source_url_path.clone(),
                                text: text.to_string(),
                                anchor: None,
                            };
                            if !inbound_links.contains(&link) {
                                inbound_links.push(link);
                            }
                        }
                    }
                }
            }
        }
    }

    // Deduplicate inbound links by source file - if a page links to the target
    // multiple times, we only keep the first occurrence
    let mut seen_sources: HashSet<String> = HashSet::new();
    let deduplicated_links: Vec<InboundLink> = inbound_links
        .into_iter()
        .filter(|link| seen_sources.insert(link.from.clone()))
        .collect();

    tracing::debug!(
        "Scanned {} files for inbound links to {} in {:?}, found {}",
        files_scanned,
        target_url_path,
        start.elapsed(),
        deduplicated_links.len()
    );

    deduplicated_links
}

/// Computes the URL path for a markdown file.
fn compute_url_path(file_path: &Path, root_dir: &Path, markdown_extensions: &[String]) -> String {
    let relative = file_path.strip_prefix(root_dir).unwrap_or(file_path);

    let mut url_path = String::from("/");

    for component in relative.components() {
        if let std::path::Component::Normal(name) = component {
            let name_str = name.to_string_lossy();
            url_path.push_str(&name_str);
            url_path.push('/');
        }
    }

    // Remove the file extension and add trailing slash
    for ext in markdown_extensions {
        let suffix = format!(".{}/", ext);
        if url_path.ends_with(&suffix) {
            url_path = url_path[..url_path.len() - suffix.len()].to_string();
            url_path.push('/');
            break;
        }
    }

    url_path
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_compute_url_path_basic() {
        let root = Path::new("/home/user/notes");
        let file = Path::new("/home/user/notes/docs/guide.md");
        let extensions = vec!["md".to_string()];

        let url = compute_url_path(file, root, &extensions);
        assert_eq!(url, "/docs/guide/");
    }

    #[test]
    fn test_compute_url_path_nested() {
        let root = Path::new("/notes");
        let file = Path::new("/notes/a/b/c/page.md");
        let extensions = vec!["md".to_string()];

        let url = compute_url_path(file, root, &extensions);
        assert_eq!(url, "/a/b/c/page/");
    }

    #[test]
    fn test_inbound_link_cache_basic() {
        let cache = InboundLinkCache::new(1024 * 1024, 60);

        let links = vec![InboundLink {
            from: "/other/".to_string(),
            text: "Link text".to_string(),
            anchor: None,
        }];

        cache.insert("/docs/".to_string(), links.clone());

        let retrieved = cache.get("/docs/");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().len(), 1);
    }

    #[test]
    fn test_inbound_link_cache_disabled() {
        let cache = InboundLinkCache::new(0, 60);

        let links = vec![InboundLink {
            from: "/other/".to_string(),
            text: "Link".to_string(),
            anchor: None,
        }];

        cache.insert("/docs/".to_string(), links);
        assert!(cache.get("/docs/").is_none());
    }

    #[test]
    fn test_find_inbound_links_basic() {
        let temp_dir = TempDir::new().unwrap();

        // Create target file
        let target_path = temp_dir.path().join("target.md");
        fs::write(&target_path, "# Target Page\n\nThis is the target.").unwrap();

        // Create source file with link to target
        let source_path = temp_dir.path().join("source.md");
        fs::write(
            &source_path,
            "# Source Page\n\nHere is a [link to target](target/).",
        )
        .unwrap();

        let extensions = vec!["md".to_string()];
        let ignore_dirs: Vec<String> = vec![];
        let ignore_globs: Vec<String> = vec![];

        let links = find_inbound_links(
            "/target/",
            temp_dir.path(),
            &extensions,
            &ignore_dirs,
            &ignore_globs,
        );

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].from, "/source/");
        assert_eq!(links[0].text, "link to target");
    }

    #[test]
    fn test_find_inbound_links_with_anchor() {
        let temp_dir = TempDir::new().unwrap();

        // Create target file
        fs::write(temp_dir.path().join("target.md"), "# Target").unwrap();

        // Create source with anchor link
        fs::write(
            temp_dir.path().join("source.md"),
            "Link: [section link](target/#section)",
        )
        .unwrap();

        let links = find_inbound_links("/target/", temp_dir.path(), &["md".to_string()], &[], &[]);

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].anchor, Some("#section".to_string()));
    }

    #[test]
    fn test_find_inbound_links_wiki_style_basic() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("Japan.md"), "# Japan").unwrap();
        fs::write(temp_dir.path().join("source.md"), "See also: [[Japan]]").unwrap();

        let links = find_inbound_links("/Japan/", temp_dir.path(), &["md".to_string()], &[], &[]);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].from, "/source/");
        assert_eq!(links[0].text, "Japan");
    }

    #[test]
    fn test_find_inbound_links_wiki_style_with_display_text() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("Japan.md"), "# Japan").unwrap();
        fs::write(
            temp_dir.path().join("source.md"),
            "Visit [[Japan|the Land of the Rising Sun]].",
        )
        .unwrap();

        let links = find_inbound_links("/Japan/", temp_dir.path(), &["md".to_string()], &[], &[]);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].text, "the Land of the Rising Sun");
    }

    #[test]
    fn test_find_inbound_links_wiki_style_with_anchor() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("Japan.md"), "# Japan").unwrap();
        fs::write(temp_dir.path().join("source.md"), "See [[Japan#History]].").unwrap();

        let links = find_inbound_links("/Japan/", temp_dir.path(), &["md".to_string()], &[], &[]);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].anchor, Some("#History".to_string()));
    }

    #[test]
    fn test_find_inbound_links_wiki_style_case_insensitive() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("Japan.md"), "# Japan").unwrap();
        fs::write(
            temp_dir.path().join("source.md"),
            "See [[japan]] for details.",
        )
        .unwrap();

        let links = find_inbound_links("/Japan/", temp_dir.path(), &["md".to_string()], &[], &[]);
        assert_eq!(links.len(), 1);
    }

    #[test]
    fn test_find_inbound_links_mixed_markdown_and_wiki() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("target.md"), "# Target").unwrap();
        fs::write(
            temp_dir.path().join("source.md"),
            "See [standard](target/) and [[target]].",
        )
        .unwrap();

        // Even though source.md links to target via both markdown and wiki syntax,
        // we deduplicate by source file - only one inbound link per source page
        let links = find_inbound_links("/target/", temp_dir.path(), &["md".to_string()], &[], &[]);
        assert_eq!(links.len(), 1);
    }

    #[test]
    fn test_find_inbound_links_multiple_sources() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("target.md"), "# Target").unwrap();
        fs::write(temp_dir.path().join("source1.md"), "See [link](target/).").unwrap();
        fs::write(
            temp_dir.path().join("source2.md"),
            "Also see [another link](target/).",
        )
        .unwrap();

        // Two different source files linking to the same target = two inbound links
        let links = find_inbound_links("/target/", temp_dir.path(), &["md".to_string()], &[], &[]);
        assert_eq!(links.len(), 2);
    }
}

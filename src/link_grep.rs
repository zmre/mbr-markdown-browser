//! Grep-based inbound link discovery for server mode.
//!
//! This module provides fast, on-demand discovery of pages that link to a given page
//! by searching through all markdown files in the repository.
//!
//! ## Algorithm
//!
//! The key challenge is that markdown links can be:
//! - Absolute: `/a/b/c/1`
//! - Relative to current folder: `c/1`, `./c/1`
//! - Relative with parent traversal: `../b/c/1`, `../../a/b/c/1`
//!
//! To efficiently find all links to a target page, we:
//! 1. Collect all unique folder paths in the repository
//! 2. For each folder, compute which patterns could represent a link to the target
//! 3. Build an Aho-Corasick automaton per folder for fast multi-pattern matching
//! 4. Scan each file using the automaton for its folder
//! 5. Only when a match is found, extract link details with regex

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use papaya::HashMap as ConcurrentHashMap;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
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

/// Computes the relative path from a source folder to a target URL path.
///
/// Given a source folder and a target URL path (both as URL-style paths starting with `/`),
/// returns the relative path that would be used in a link.
///
/// # Examples
/// - `/a/` -> `/a/1` = `1`
/// - `/a/` -> `/a/b/1` = `b/1`
/// - `/a/b/` -> `/a/1` = `../1`
/// - `/a/` -> `/b/1` = `../b/1`
/// - `/a/b/c/` -> `/d/e/f/1` = `../../../d/e/f/1`
///
/// # Arguments
/// * `source_folder` - The folder containing the source file (e.g., `/a/b/`)
/// * `target_path` - The target URL path (e.g., `/a/b/c/1`)
///
/// # Returns
/// The relative path from source to target (e.g., `c/1` or `../1`)
fn compute_relative_path(source_folder: &str, target_path: &str) -> String {
    // Normalize: strip leading slash and any trailing slashes for comparison
    let source = source_folder.trim_start_matches('/').trim_end_matches('/');
    let target = target_path.trim_start_matches('/').trim_end_matches('/');

    // Split into segments
    let source_parts: Vec<&str> = if source.is_empty() {
        vec![]
    } else {
        source.split('/').collect()
    };
    let target_parts: Vec<&str> = if target.is_empty() {
        vec![]
    } else {
        target.split('/').collect()
    };

    // Find common prefix length
    let common_len = source_parts
        .iter()
        .zip(target_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();

    // Number of ".." needed to go up from source to common ancestor
    let ups_needed = source_parts.len() - common_len;

    // Build the relative path
    let mut result_parts: Vec<&str> = vec![".."; ups_needed];

    // Add the remaining target parts after the common prefix
    result_parts.extend(&target_parts[common_len..]);

    if result_parts.is_empty() {
        // Same directory - shouldn't happen for different files but handle it
        ".".to_string()
    } else {
        result_parts.join("/")
    }
}

/// Computes all possible link patterns that could reference the target from a given source folder.
///
/// This generates patterns for:
/// - Absolute paths: `/a/b/c/1`
/// - Relative paths without prefix: `b/c/1`
/// - Relative paths with `./`: `./b/c/1`
/// - Parent traversal: `../a/b/c/1`, `../../a/b/c/1`
///
/// For each base pattern, generates variants:
/// - Without trailing slash: `b/c/1`
/// - With trailing slash: `b/c/1/`
/// - With .md extension: `b/c/1.md`
/// - With anchor start: `b/c/1#` (to catch `b/c/1#anchor`)
///
/// # Arguments
/// * `source_folder` - The URL path of the folder containing the source file (e.g., `/docs/`)
/// * `target_url_path` - The full URL path of the target (e.g., `/a/b/c/1/`)
///
/// # Returns
/// A vector of all patterns that could be valid links to the target from this folder
fn compute_patterns_for_folder(source_folder: &str, target_url_path: &str) -> Vec<String> {
    let mut patterns = HashSet::new();

    // Normalize target (strip leading/trailing slashes for the base)
    let target_normalized = target_url_path
        .trim_start_matches('/')
        .trim_end_matches('/');

    if target_normalized.is_empty() {
        return vec![];
    }

    // 1. Absolute paths (always valid from any folder)
    let abs_path = format!("/{}", target_normalized);
    add_pattern_variants(&mut patterns, &abs_path);

    // 2. Relative path from this folder
    let relative = compute_relative_path(source_folder, target_url_path);

    // Skip if relative path is just "." (same location)
    if relative != "." {
        // Add the relative path
        add_pattern_variants(&mut patterns, &relative);

        // Add with explicit ./ prefix if it doesn't already have ../ prefix
        if !relative.starts_with("../") && !relative.starts_with("./") {
            add_pattern_variants(&mut patterns, &format!("./{}", relative));
        }
    }

    patterns.into_iter().collect()
}

/// Adds pattern variants for a base path.
///
/// For base path `a/b/c`, adds:
/// - `a/b/c`
/// - `a/b/c/`
/// - `a/b/c.md`
/// - `a/b/c#` (for anchor detection)
fn add_pattern_variants(patterns: &mut HashSet<String>, base: &str) {
    let normalized = base.trim_end_matches('/');
    patterns.insert(normalized.to_string());
    patterns.insert(format!("{}/", normalized));
    patterns.insert(format!("{}.md", normalized));
    patterns.insert(format!("{}#", normalized));
}

/// Builds a mapping from folder paths to their Aho-Corasick search patterns.
///
/// # Arguments
/// * `target_url_path` - The URL path being searched for (e.g., "/docs/guide/")
/// * `all_folders` - Set of all folder URL paths in the repository
///
/// # Returns
/// HashMap from folder URL path to patterns valid for that folder
fn build_folder_patterns(
    target_url_path: &str,
    all_folders: &HashSet<String>,
) -> HashMap<String, Vec<String>> {
    all_folders
        .iter()
        .map(|folder| {
            let patterns = compute_patterns_for_folder(folder, target_url_path);
            (folder.clone(), patterns)
        })
        .collect()
}

/// Builds a regex pattern that matches any of the given patterns in markdown link syntax.
///
/// Creates a pattern like: `\[([^\]]*)\]\((pattern1|pattern2|...)(?:#([^)]*))?\)`
fn build_extraction_regex(patterns: &[String]) -> Option<Regex> {
    if patterns.is_empty() {
        return None;
    }

    // Escape patterns for regex and join with |
    let escaped_patterns: Vec<String> = patterns
        .iter()
        .map(|p| {
            // Remove trailing slash, .md, # for the pattern base
            let base = p
                .trim_end_matches('/')
                .trim_end_matches(".md")
                .trim_end_matches('#');
            regex::escape(base)
        })
        .collect();

    // Deduplicate
    let unique_patterns: HashSet<String> = escaped_patterns.into_iter().collect();
    let pattern_alternation = unique_patterns.into_iter().collect::<Vec<_>>().join("|");

    // Build regex for inline markdown links: [text](url) or [text](url#anchor)
    let pattern = format!(
        r#"\[([^\]]*)\]\((?:{})(?:\.md)?(?:/)?(?:#([^)]*))?\)"#,
        pattern_alternation
    );

    Regex::new(&pattern).ok()
}

/// Builds a regex pattern for wiki-style links.
fn build_wiki_extraction_regex(patterns: &[String]) -> Option<Regex> {
    if patterns.is_empty() {
        return None;
    }

    // Escape patterns for regex and join with |
    let escaped_patterns: Vec<String> = patterns
        .iter()
        .map(|p| {
            let base = p
                .trim_end_matches('/')
                .trim_end_matches(".md")
                .trim_end_matches('#');
            regex::escape(base)
        })
        .collect();

    let unique_patterns: HashSet<String> = escaped_patterns.into_iter().collect();
    let pattern_alternation = unique_patterns.into_iter().collect::<Vec<_>>().join("|");

    // Build regex for wiki-style links: [[target]], [[target|text]], [[target#anchor]]
    // Case insensitive
    let pattern = format!(
        r#"(?i)\[\[(?:{})(?:\.md)?(?:/)?(?:#([^\]|]*))?(?:\|([^\]]*))?\]\]"#,
        pattern_alternation
    );

    Regex::new(&pattern).ok()
}

/// Builds a regex pattern for reference-style links.
fn build_ref_extraction_regex(patterns: &[String]) -> Option<Regex> {
    if patterns.is_empty() {
        return None;
    }

    let escaped_patterns: Vec<String> = patterns
        .iter()
        .map(|p| {
            let base = p
                .trim_end_matches('/')
                .trim_end_matches(".md")
                .trim_end_matches('#');
            regex::escape(base)
        })
        .collect();

    let unique_patterns: HashSet<String> = escaped_patterns.into_iter().collect();
    let pattern_alternation = unique_patterns.into_iter().collect::<Vec<_>>().join("|");

    // Build regex for reference-style link definitions: [ref]: url
    let pattern = format!(
        r#"\[([^\]]+)\]:\s*(?:{})(?:\.md)?(?:/)?(?:#\S*)?"#,
        pattern_alternation
    );

    Regex::new(&pattern).ok()
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
    let target_segments = target_normalized.trim_start_matches('/');

    if target_segments.is_empty() {
        return inbound_links;
    }

    // First pass: collect all unique folder paths and their files
    let mut folder_files: HashMap<String, Vec<(PathBuf, String)>> = HashMap::new();

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

        // Compute folder URL path and source URL path
        let source_url_path = compute_url_path(path, root_dir, markdown_extensions);
        let folder_url_path = get_folder_url_path(&source_url_path);

        // Skip if this is the target page itself
        if source_url_path.trim_end_matches('/') == target_normalized {
            continue;
        }

        folder_files
            .entry(folder_url_path)
            .or_default()
            .push((path.to_path_buf(), source_url_path));
    }

    // Collect all unique folders
    let all_folders: HashSet<String> = folder_files.keys().cloned().collect();

    // Build patterns for each folder
    let folder_patterns = build_folder_patterns(target_url_path, &all_folders);

    // Build Aho-Corasick automatons for each folder (case-insensitive for wiki links)
    let mut folder_automatons: HashMap<String, Option<AhoCorasick>> = HashMap::new();

    for (folder, patterns) in &folder_patterns {
        if patterns.is_empty() {
            folder_automatons.insert(folder.clone(), None);
        } else {
            // Build case-insensitive automaton to match wiki-style [[links]]
            match AhoCorasickBuilder::new()
                .ascii_case_insensitive(true)
                .match_kind(MatchKind::LeftmostFirst)
                .build(patterns)
            {
                Ok(ac) => {
                    folder_automatons.insert(folder.clone(), Some(ac));
                }
                Err(e) => {
                    tracing::warn!("Failed to build Aho-Corasick for folder {}: {}", folder, e);
                    folder_automatons.insert(folder.clone(), None);
                }
            }
        }
    }

    // Build folder-specific extraction regexes
    let mut folder_link_regexes: HashMap<String, Option<Regex>> = HashMap::new();
    let mut folder_wiki_regexes: HashMap<String, Option<Regex>> = HashMap::new();
    let mut folder_ref_regexes: HashMap<String, Option<Regex>> = HashMap::new();

    for (folder, patterns) in &folder_patterns {
        folder_link_regexes.insert(folder.clone(), build_extraction_regex(patterns));
        folder_wiki_regexes.insert(folder.clone(), build_wiki_extraction_regex(patterns));
        folder_ref_regexes.insert(folder.clone(), build_ref_extraction_regex(patterns));
    }

    let mut files_scanned = 0;

    // Scan files using folder-specific automatons
    for (folder, files) in &folder_files {
        let automaton = folder_automatons.get(folder).and_then(|a| a.as_ref());

        // Skip if no automaton (no patterns for this folder)
        let Some(ac) = automaton else {
            continue;
        };

        let link_regex = folder_link_regexes.get(folder).and_then(|r| r.as_ref());
        let wiki_regex = folder_wiki_regexes.get(folder).and_then(|r| r.as_ref());
        let ref_regex = folder_ref_regexes.get(folder).and_then(|r| r.as_ref());

        for (path, source_url_path) in files {
            files_scanned += 1;

            // Read file content
            let content = match fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Fast check with Aho-Corasick
            if !ac.is_match(&content) {
                continue;
            }

            // Found a potential match - extract details with regex
            let mut found_link = false;

            // Search for inline links
            if let Some(regex) = link_regex {
                for cap in regex.captures_iter(&content) {
                    let text = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                    let anchor = cap.get(2).map(|m| format!("#{}", m.as_str()));

                    inbound_links.push(InboundLink {
                        from: source_url_path.clone(),
                        text: text.to_string(),
                        anchor,
                    });
                    found_link = true;
                }
            }

            // Search for wiki-style links
            if let Some(regex) = wiki_regex {
                for cap in regex.captures_iter(&content) {
                    let anchor = cap.get(1).and_then(|m| {
                        let s = m.as_str();
                        if s.is_empty() {
                            None
                        } else {
                            Some(format!("#{}", s))
                        }
                    });

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
                        found_link = true;
                    }
                }
            }

            // Search for reference-style links
            if !found_link && let Some(regex) = ref_regex {
                for cap in regex.captures_iter(&content) {
                    let ref_name = cap.get(1).map(|m| m.as_str()).unwrap_or("");

                    // Find uses of this reference: [text][ref_name]
                    let use_pattern = format!(r#"\[([^\]]*)\]\[{}\]"#, regex::escape(ref_name));
                    if let Ok(use_regex) = Regex::new(&use_pattern) {
                        for use_cap in use_regex.captures_iter(&content) {
                            let text = use_cap.get(1).map(|m| m.as_str()).unwrap_or("");

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

/// Gets the folder URL path from a file URL path.
/// `/a/b/c/` -> `/a/b/`
/// `/a/` -> `/`
fn get_folder_url_path(file_url_path: &str) -> String {
    let trimmed = file_url_path.trim_end_matches('/');
    if let Some(pos) = trimmed.rfind('/') {
        format!("{}/", &trimmed[..pos])
    } else {
        "/".to_string()
    }
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

    // ========== compute_relative_path tests ==========

    #[test]
    fn test_compute_relative_path_same_directory() {
        // /a/ -> /a/1 = 1
        assert_eq!(compute_relative_path("/a/", "/a/1"), "1");
        assert_eq!(compute_relative_path("/a/", "/a/1/"), "1");
    }

    #[test]
    fn test_compute_relative_path_subdirectory() {
        // /a/ -> /a/b/1 = b/1
        assert_eq!(compute_relative_path("/a/", "/a/b/1"), "b/1");
        assert_eq!(compute_relative_path("/a/", "/a/b/c/1"), "b/c/1");
    }

    #[test]
    fn test_compute_relative_path_parent_directory() {
        // /a/b/ -> /a/1 = ../1
        assert_eq!(compute_relative_path("/a/b/", "/a/1"), "../1");
        assert_eq!(compute_relative_path("/a/b/c/", "/a/1"), "../../1");
    }

    #[test]
    fn test_compute_relative_path_sibling_directory() {
        // /a/ -> /b/1 = ../b/1
        assert_eq!(compute_relative_path("/a/", "/b/1"), "../b/1");
        assert_eq!(compute_relative_path("/a/", "/b/c/1"), "../b/c/1");
    }

    #[test]
    fn test_compute_relative_path_deep_nesting() {
        // /a/b/c/ -> /d/e/f/1 = ../../../d/e/f/1
        assert_eq!(
            compute_relative_path("/a/b/c/", "/d/e/f/1"),
            "../../../d/e/f/1"
        );
    }

    #[test]
    fn test_compute_relative_path_from_root() {
        // / -> /a/b/1 = a/b/1
        assert_eq!(compute_relative_path("/", "/a/b/1"), "a/b/1");
    }

    #[test]
    fn test_compute_relative_path_to_root_level() {
        // /a/b/ -> /1 = ../../1
        assert_eq!(compute_relative_path("/a/b/", "/1"), "../../1");
    }

    // ========== compute_patterns_for_folder tests ==========

    #[test]
    fn test_compute_patterns_for_folder_root() {
        let patterns = compute_patterns_for_folder("/", "/a/b/c/1/");

        // Should include absolute path variants
        assert!(patterns.contains(&"/a/b/c/1".to_string()));
        assert!(patterns.contains(&"/a/b/c/1/".to_string()));
        assert!(patterns.contains(&"/a/b/c/1.md".to_string()));
        assert!(patterns.contains(&"/a/b/c/1#".to_string()));

        // Should include relative path variants
        assert!(patterns.contains(&"a/b/c/1".to_string()));
        assert!(patterns.contains(&"a/b/c/1/".to_string()));
        assert!(patterns.contains(&"./a/b/c/1".to_string()));
        assert!(patterns.contains(&"./a/b/c/1/".to_string()));
    }

    #[test]
    fn test_compute_patterns_for_folder_same_directory() {
        let patterns = compute_patterns_for_folder("/a/b/", "/a/b/c/1/");

        // Should include absolute path
        assert!(patterns.contains(&"/a/b/c/1".to_string()));

        // Should include relative path (c/1)
        assert!(patterns.contains(&"c/1".to_string()));
        assert!(patterns.contains(&"./c/1".to_string()));
    }

    #[test]
    fn test_compute_patterns_for_folder_sibling() {
        let patterns = compute_patterns_for_folder("/d/", "/a/b/c/1/");

        // Should include absolute path
        assert!(patterns.contains(&"/a/b/c/1".to_string()));

        // Should include parent traversal
        assert!(patterns.contains(&"../a/b/c/1".to_string()));
        assert!(patterns.contains(&"../a/b/c/1/".to_string()));
    }

    #[test]
    fn test_compute_patterns_for_folder_deeper_sibling() {
        let patterns = compute_patterns_for_folder("/d/e/", "/a/b/c/1/");

        // Should include absolute path
        assert!(patterns.contains(&"/a/b/c/1".to_string()));

        // Should include double parent traversal
        assert!(patterns.contains(&"../../a/b/c/1".to_string()));
    }

    // ========== get_folder_url_path tests ==========

    #[test]
    fn test_get_folder_url_path_basic() {
        assert_eq!(get_folder_url_path("/a/b/c/"), "/a/b/");
        assert_eq!(get_folder_url_path("/a/"), "/");
        assert_eq!(get_folder_url_path("/a/b/"), "/a/");
    }

    // ========== compute_url_path tests ==========

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

    // ========== InboundLinkCache tests ==========

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

    // ========== find_inbound_links integration tests ==========

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

    // ========== NEW: Relative path tests (the bug fix) ==========

    #[test]
    fn test_find_inbound_links_relative_path_from_subfolder() {
        let temp_dir = TempDir::new().unwrap();

        // Create directory structure:
        // /coins/tricks/3-fly.md (target)
        // /coins/overview.md (source with relative link)
        let tricks_dir = temp_dir.path().join("coins").join("tricks");
        fs::create_dir_all(&tricks_dir).unwrap();

        fs::write(tricks_dir.join("3-fly.md"), "# 3 Fly Trick").unwrap();
        fs::write(
            temp_dir.path().join("coins").join("overview.md"),
            "Check out [3 Fly](tricks/3-fly/) for more.",
        )
        .unwrap();

        let links = find_inbound_links(
            "/coins/tricks/3-fly/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
        );

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].from, "/coins/overview/");
        assert_eq!(links[0].text, "3 Fly");
    }

    #[test]
    fn test_find_inbound_links_relative_path_with_parent_traversal() {
        let temp_dir = TempDir::new().unwrap();

        // Create directory structure:
        // /coins/tricks/3-fly.md (target)
        // /cards/overview.md (source with ../coins/tricks/3-fly link)
        let coins_tricks_dir = temp_dir.path().join("coins").join("tricks");
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&coins_tricks_dir).unwrap();
        fs::create_dir_all(&cards_dir).unwrap();

        fs::write(coins_tricks_dir.join("3-fly.md"), "# 3 Fly Trick").unwrap();
        fs::write(
            cards_dir.join("overview.md"),
            "See also [3 Fly](../coins/tricks/3-fly/) coin trick.",
        )
        .unwrap();

        let links = find_inbound_links(
            "/coins/tricks/3-fly/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
        );

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].from, "/cards/overview/");
        assert_eq!(links[0].text, "3 Fly");
    }

    #[test]
    fn test_find_inbound_links_absolute_path() {
        let temp_dir = TempDir::new().unwrap();

        // Create directory structure:
        // /coins/tricks/3-fly.md (target)
        // /cards/overview.md (source with absolute link)
        let coins_tricks_dir = temp_dir.path().join("coins").join("tricks");
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&coins_tricks_dir).unwrap();
        fs::create_dir_all(&cards_dir).unwrap();

        fs::write(coins_tricks_dir.join("3-fly.md"), "# 3 Fly Trick").unwrap();
        fs::write(
            cards_dir.join("overview.md"),
            "See also [3 Fly](/coins/tricks/3-fly/) coin trick.",
        )
        .unwrap();

        let links = find_inbound_links(
            "/coins/tricks/3-fly/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
        );

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].from, "/cards/overview/");
    }

    #[test]
    fn test_find_inbound_links_deep_relative_path() {
        let temp_dir = TempDir::new().unwrap();

        // Create directory structure:
        // /a/b/c/target.md
        // /d/e/f/source.md with link ../../../a/b/c/target
        let target_dir = temp_dir.path().join("a").join("b").join("c");
        let source_dir = temp_dir.path().join("d").join("e").join("f");
        fs::create_dir_all(&target_dir).unwrap();
        fs::create_dir_all(&source_dir).unwrap();

        fs::write(target_dir.join("target.md"), "# Target").unwrap();
        fs::write(
            source_dir.join("source.md"),
            "Link: [target](../../../a/b/c/target/)",
        )
        .unwrap();

        let links = find_inbound_links(
            "/a/b/c/target/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
        );

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].from, "/d/e/f/source/");
    }

    #[test]
    fn test_find_inbound_links_with_dot_slash_prefix() {
        let temp_dir = TempDir::new().unwrap();

        // Create directory structure with ./relative link
        let tricks_dir = temp_dir.path().join("coins").join("tricks");
        fs::create_dir_all(&tricks_dir).unwrap();

        fs::write(tricks_dir.join("3-fly.md"), "# 3 Fly").unwrap();
        fs::write(
            temp_dir.path().join("coins").join("index.md"),
            "See [3 Fly](./tricks/3-fly/) for more.",
        )
        .unwrap();

        let links = find_inbound_links(
            "/coins/tricks/3-fly/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
        );

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].from, "/coins/index/");
    }

    #[test]
    fn test_find_inbound_links_relative_with_md_extension() {
        let temp_dir = TempDir::new().unwrap();

        // Create directory structure with .md extension in link
        let tricks_dir = temp_dir.path().join("coins").join("tricks");
        fs::create_dir_all(&tricks_dir).unwrap();

        fs::write(tricks_dir.join("3-fly.md"), "# 3 Fly").unwrap();
        fs::write(
            temp_dir.path().join("coins").join("index.md"),
            "See [3 Fly](tricks/3-fly.md) for more.",
        )
        .unwrap();

        let links = find_inbound_links(
            "/coins/tricks/3-fly/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
        );

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].from, "/coins/index/");
    }
}

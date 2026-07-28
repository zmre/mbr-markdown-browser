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
//!
//! Bare `[[Name]]` wiki links do not name a path at all: the renderer resolves
//! them through [`crate::wikilink_index::WikilinkIndex`] by title, alias, or
//! filename stem. Those names are therefore added to the pattern set as well
//! (gate + wiki regex only — never the inline/reference regexes, where a name
//! is not a URL).

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use papaya::HashMap as ConcurrentHashMap;
use regex::Regex;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use walkdir::WalkDir;

use crate::link_index::{InboundLink, sort_inbound_links};
use crate::repo::{build_markdown_url_path, should_ignore};

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

        // If an entry already existed for this key, subtract its accounted size
        // first so `current_size` reflects the replacement rather than
        // ratcheting upward on every overwrite.
        let replaced_size = self
            .cache
            .pin()
            .insert(url_path.clone(), entry)
            .map_or(0, |old| old.size_bytes);
        self.current_size
            .fetch_sub(replaced_size, Ordering::Relaxed);
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

    /// Returns the current approximate size of the cache in bytes.
    #[cfg(test)]
    pub fn current_size(&self) -> usize {
        self.current_size.load(Ordering::Relaxed)
    }

    /// Returns the number of entries in the cache.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.cache.pin().len()
    }

    /// Returns true if the cache is empty.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.cache.pin().is_empty()
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
pub(crate) fn compute_relative_path(source_folder: &str, target_path: &str) -> String {
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
pub(crate) fn compute_patterns_for_folder(
    source_folder: &str,
    target_url_path: &str,
) -> Vec<String> {
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
pub(crate) fn add_pattern_variants(patterns: &mut HashSet<String>, base: &str) {
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
///
/// `names` holds the target's bare wiki names (title, aliases, filename stem).
/// They are alternatives here — and in the Aho-Corasick gate — but deliberately
/// nowhere else: `[text](Some Title)` is not a link to the target.
fn build_wiki_extraction_regex(patterns: &[String], names: &[String]) -> Option<Regex> {
    if patterns.is_empty() && names.is_empty() {
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
        .chain(names.iter().map(|n| regex::escape(n)))
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
/// Bare `[[Name]]` links to the target (by title, alias, or filename stem) are
/// found too: the target's own names are read from its frontmatter once and
/// added to the wiki pattern set, so server-mode backlinks match what the
/// renderer (and therefore build mode) resolves.
///
/// Links written inside fenced code blocks or backtick spans are ignored, as
/// the parser-derived build-mode index ignores them.
///
/// # Arguments
/// * `target_url_path` - The URL path being linked to (e.g., "/docs/guide/")
/// * `root_dir` - Root directory of the markdown repository
/// * `markdown_extensions` - List of valid markdown file extensions
/// * `ignore_dirs` - Directories to skip during scanning
/// * `ignore_globs` - Glob patterns for files to ignore
/// * `index_file` - Configured index file name (e.g., "index.md"), so the
///   reported `from` URLs are the canonical ones (`docs/index.md` → `/docs/`)
///
/// # Returns
/// A vector of `InboundLink` structs representing pages that link to the target.
pub fn find_inbound_links(
    target_url_path: &str,
    root_dir: &Path,
    markdown_extensions: &[String],
    ignore_dirs: &[String],
    ignore_globs: &[String],
    index_file: &str,
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
    // The target's own file, located during the walk so its wiki names
    // (title/aliases/stem) can be read without a second directory scan.
    let mut target_file: Option<PathBuf> = None;

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
        let (source_url_path, folder_url_path) =
            page_and_folder_urls(path, root_dir, markdown_extensions, Some(index_file));

        // Skip if this is the target page itself
        if source_url_path.trim_end_matches('/') == target_normalized {
            target_file = Some(path.to_path_buf());
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

    // The bare `[[Name]]` forms that resolve to this target. Folder-independent,
    // so they are computed once and reused by every folder's gate/wiki regex.
    let target_names = target_file
        .as_deref()
        .map(wikilink_names_for_target)
        .unwrap_or_default();

    // Build Aho-Corasick automatons for each folder (case-insensitive for wiki links)
    let mut folder_automatons: HashMap<String, Option<AhoCorasick>> = HashMap::new();

    for (folder, patterns) in &folder_patterns {
        let gate_patterns: Vec<&String> = patterns.iter().chain(target_names.iter()).collect();
        if gate_patterns.is_empty() {
            folder_automatons.insert(folder.clone(), None);
        } else {
            // Build case-insensitive automaton to match wiki-style [[links]]
            match AhoCorasickBuilder::new()
                .ascii_case_insensitive(true)
                .match_kind(MatchKind::LeftmostFirst)
                .build(&gate_patterns)
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
        folder_wiki_regexes.insert(
            folder.clone(),
            build_wiki_extraction_regex(patterns, &target_names),
        );
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

            // Only candidate files pay for code stripping. Links inside fences
            // and backtick spans are documentation, not references: the
            // parser-derived build-mode index never sees them.
            let content = strip_code_regions(&content);

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

    // Sort before deduplicating, not after. The scan above iterated
    // `folder_files`, a std `HashMap`, so the push order was randomly seeded
    // per process — which made *both* the order of these links and, because
    // the dedup keeps the first occurrence per source, *which* link survives
    // for a page that links here twice depend on that hash order. Sorting
    // first pins the surviving link to the lowest-sorting one and leaves the
    // result in the same order the builder emits, so the two modes agree.
    sort_inbound_links(&mut inbound_links);

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
pub(crate) fn get_folder_url_path(file_url_path: &str) -> String {
    let trimmed = file_url_path.trim_end_matches('/');
    if let Some(pos) = trimmed.rfind('/') {
        format!("{}/", &trimmed[..pos])
    } else {
        "/".to_string()
    }
}

/// The two URL forms a markdown file needs during a link scan.
///
/// Returns `(page_url, folder_url)`:
/// - `page_url` is the **canonical** site URL — the same value
///   [`crate::repo::build_markdown_url_path`] puts in `site.json`, so
///   `docs/index.md` collapses to `/docs/`. This is what gets reported as
///   `InboundLink::from`. Pass `index_file: None` when the caller has no index
///   file configured, which keeps the positional form.
/// - `folder_url` is the folder a *relative* link inside that file resolves
///   against, and is deliberately derived from the **positional** URL: for
///   `docs/index.md` that is `/docs/index/` → `/docs/`, whereas the canonical
///   `/docs/` would wrongly yield `/`.
pub(crate) fn page_and_folder_urls(
    file_path: &Path,
    root_dir: &Path,
    markdown_extensions: &[String],
    index_file: Option<&str>,
) -> (String, String) {
    let positional = compute_url_path(file_path, root_dir, markdown_extensions);
    let folder_url = get_folder_url_path(&positional);
    let page_url = match index_file {
        Some(index_file) => build_markdown_url_path(file_path, root_dir, index_file),
        None => positional,
    };
    (page_url, folder_url)
}

/// Computes the **positional** URL path for a markdown file: every path
/// component becomes a URL segment and the markdown extension becomes a
/// trailing slash.
///
/// This is not the canonical page URL — it does not collapse the configured
/// index file — and is only used to derive the folder that a file's relative
/// links resolve against. Use [`page_and_folder_urls`] rather than calling this
/// directly.
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

    // Remove the file extension and add trailing slash. Extensions are compared
    // case-insensitively so `NOTES.MD` and `notes.md` behave the same way (the
    // walk that feeds this function already lowercases the extension it filters
    // on). A matching tail is pure ASCII, so truncating there is always on a
    // character boundary.
    for ext in markdown_extensions {
        let suffix = format!(".{}/", ext);
        let cut = match url_path.len().checked_sub(suffix.len()) {
            Some(cut) => cut,
            None => continue,
        };
        if url_path.as_bytes()[cut..].eq_ignore_ascii_case(suffix.as_bytes()) {
            url_path.truncate(cut);
            url_path.push('/');
            break;
        }
    }

    url_path
}

/// The bare `[[Name]]` forms that resolve to `path`: its frontmatter `title`,
/// its frontmatter `aliases`, and its filename stem.
///
/// Mirrors the note inputs [`crate::wikilink_index::WikilinkIndex`] is built
/// from (`Repo::collect_note_inputs`), so grep-based backlinks see the same
/// names the renderer resolves. Names carrying wiki-link syntax (`/`, `[`, `]`,
/// `|`, `#`) are dropped: path forms are already covered by the path patterns,
/// and the rest could never appear inside a `[[…]]` target.
fn wikilink_names_for_target(path: &Path) -> Vec<String> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    let metadata = crate::markdown::extract_metadata_from_file(path)
        .map(|m| m.metadata)
        .unwrap_or_default();
    let title = metadata
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let aliases = metadata
        .get("aliases")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();

    let mut seen: HashSet<String> = HashSet::new();
    std::iter::once(stem)
        .chain(title)
        .chain(aliases)
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty() && !name.contains(['/', '[', ']', '|', '#']))
        .filter(|name| seen.insert(name.to_lowercase()))
        .collect()
}

/// Blanks out fenced code blocks and inline code spans so link extraction sees
/// only prose, matching the parser-derived (build-mode) link index.
///
/// Single pass over the lines; blanked regions keep their line structure so the
/// line-anchored reference-definition regex still behaves. Indented code blocks
/// are deliberately *not* stripped: telling one from a nested list item needs
/// full block parsing, and a wrong guess would silently drop real backlinks.
fn strip_code_regions(content: &str) -> Cow<'_, str> {
    if !content.contains('`') && !content.contains('~') {
        return Cow::Borrowed(content);
    }

    let mut out = String::with_capacity(content.len());
    // The open fence's character and length, while inside a fenced block.
    let mut fence: Option<(u8, usize)> = None;

    for line in content.lines() {
        let indent = line.len() - line.trim_start_matches(' ').len();
        // A fence may be indented by at most 3 spaces (CommonMark).
        let marker = (indent <= 3)
            .then(|| fence_marker(&line[indent..]))
            .flatten();
        match fence {
            Some((fence_char, fence_len)) => {
                // A closing fence is the same character, at least as long, and
                // carries no info string.
                let closes = marker.is_some_and(|(c, n)| {
                    c == fence_char && n >= fence_len && line[indent + n..].trim_end().is_empty()
                });
                if closes {
                    fence = None;
                }
            }
            None => match marker {
                Some((c, n)) => fence = Some((c, n)),
                None => out.push_str(&strip_inline_code(line)),
            },
        }
        out.push('\n');
    }

    Cow::Owned(out)
}

/// The fence character and run length if `line` opens or closes a code fence.
fn fence_marker(line: &str) -> Option<(u8, usize)> {
    let first = *line.as_bytes().first()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let run = line.bytes().take_while(|b| *b == first).count();
    (run >= 3).then_some((first, run))
}

/// Removes inline code spans (`` `code` ``) from a single line.
///
/// A run of N backticks opens a span that ends at the next run of exactly N
/// backticks on the same line; an unclosed run is literal text and is kept.
fn strip_inline_code(line: &str) -> Cow<'_, str> {
    if !line.contains('`') {
        return Cow::Borrowed(line);
    }
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let open_start = i;
            i += bytes[i..].iter().take_while(|b| **b == b'`').count();
            let run = i - open_start;
            match find_backtick_run(bytes, i, run) {
                // Drop the span, leaving a space so neighbouring text can't fuse.
                Some(close_start) => {
                    out.push(' ');
                    i = close_start + run;
                }
                None => out.push_str(&line[open_start..i]),
            }
        } else {
            let start = i;
            i += bytes[i..].iter().take_while(|b| **b != b'`').count();
            out.push_str(&line[start..i]);
        }
    }
    Cow::Owned(out)
}

/// Byte offset of the next run of exactly `run` backticks at or after `from`.
fn find_backtick_run(bytes: &[u8], from: usize, run: usize) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let start = i;
            i += bytes[i..].iter().take_while(|b| **b == b'`').count();
            if i - start == run {
                return Some(start);
            }
        } else {
            i += 1;
        }
    }
    None
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

    #[test]
    fn compute_url_path_strips_uppercase_extension() {
        // The walk filters on a lowercased extension, so the URL builder must
        // treat `.MD` the same as `.md` instead of leaving it in the path.
        let root = Path::new("/notes");
        let extensions = vec!["md".to_string()];

        assert_eq!(
            compute_url_path(Path::new("/notes/NOTES.MD"), root, &extensions),
            "/NOTES/"
        );
    }

    // ========== page_and_folder_urls tests ==========

    #[test]
    fn page_and_folder_urls_collapses_index_but_keeps_link_base() {
        // The canonical page URL drops the index file; the folder that the
        // page's own relative links resolve against must not.
        let root = Path::new("/notes");
        let extensions = vec!["md".to_string()];

        let (page, folder) = page_and_folder_urls(
            Path::new("/notes/docs/index.md"),
            root,
            &extensions,
            Some("index.md"),
        );
        assert_eq!(page, "/docs/");
        assert_eq!(folder, "/docs/");

        let (page, folder) = page_and_folder_urls(
            Path::new("/notes/docs/guide.md"),
            root,
            &extensions,
            Some("index.md"),
        );
        assert_eq!(page, "/docs/guide/");
        assert_eq!(folder, "/docs/");
    }

    #[test]
    fn page_and_folder_urls_without_index_file_keeps_positional_url() {
        let root = Path::new("/notes");
        let extensions = vec!["md".to_string()];

        let (page, folder) =
            page_and_folder_urls(Path::new("/notes/docs/index.md"), root, &extensions, None);
        assert_eq!(page, "/docs/index/");
        assert_eq!(folder, "/docs/");
    }

    #[test]
    fn page_and_folder_urls_matches_repo_canonical_url() {
        // The reported `from` must be byte-identical to the `url_path` the repo
        // scanner puts in site.json, or backlink hrefs need a redirect and the
        // graph splits one page into two nodes.
        let root = Path::new("/notes");
        let extensions = vec!["md".to_string()];
        for file in [
            "/notes/docs/index.md",
            "/notes/docs/guide.md",
            "/notes/index.md",
            "/notes/a/b/c/page.md",
        ] {
            let (page, _folder) =
                page_and_folder_urls(Path::new(file), root, &extensions, Some("index.md"));
            assert_eq!(
                page,
                crate::repo::build_markdown_url_path(Path::new(file), root, "index.md"),
                "canonical URL mismatch for {file}"
            );
        }
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
    fn test_inbound_link_cache_size_stable_on_overwrite() {
        // Regression for #12: overwriting an existing key must not ratchet
        // `current_size` upward.
        let cache = InboundLinkCache::new(1024 * 1024, 60);
        let links = vec![InboundLink {
            from: "/other/".to_string(),
            text: "Link text".to_string(),
            anchor: None,
        }];

        cache.insert("/docs/".to_string(), links.clone());
        let size_after_first = cache.current_size();
        assert_eq!(cache.len(), 1);

        for _ in 0..10 {
            cache.insert("/docs/".to_string(), links.clone());
        }

        assert_eq!(cache.len(), 1);
        assert_eq!(cache.current_size(), size_after_first);
    }

    #[test]
    fn test_inbound_link_cache_size_tracks_replacement_delta() {
        // Overwriting with a larger payload adjusts size by the delta only.
        let cache = InboundLinkCache::new(1024 * 1024, 60);
        let one = vec![InboundLink {
            from: "/a/".to_string(),
            text: "A".to_string(),
            anchor: None,
        }];
        let two = vec![
            InboundLink {
                from: "/a/".to_string(),
                text: "A".to_string(),
                anchor: None,
            },
            InboundLink {
                from: "/b/".to_string(),
                text: "B".to_string(),
                anchor: None,
            },
        ];

        cache.insert("/docs/".to_string(), one.clone());
        let small = cache.current_size();
        cache.insert("/docs/".to_string(), two.clone());
        let large = cache.current_size();

        assert_eq!(cache.len(), 1);
        let expected_delta = "/b/".len() + "B".len() + 32;
        assert_eq!(large - small, expected_delta);
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
            "index.md",
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

        let links = find_inbound_links(
            "/target/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
            "index.md",
        );

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].anchor, Some("#section".to_string()));
    }

    #[test]
    fn test_find_inbound_links_wiki_style_basic() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("Japan.md"), "# Japan").unwrap();
        fs::write(temp_dir.path().join("source.md"), "See also: [[Japan]]").unwrap();

        let links = find_inbound_links(
            "/Japan/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
            "index.md",
        );
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

        let links = find_inbound_links(
            "/Japan/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
            "index.md",
        );
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].text, "the Land of the Rising Sun");
    }

    #[test]
    fn test_find_inbound_links_wiki_style_with_anchor() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("Japan.md"), "# Japan").unwrap();
        fs::write(temp_dir.path().join("source.md"), "See [[Japan#History]].").unwrap();

        let links = find_inbound_links(
            "/Japan/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
            "index.md",
        );
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

        let links = find_inbound_links(
            "/Japan/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
            "index.md",
        );
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
        let links = find_inbound_links(
            "/target/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
            "index.md",
        );
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
        let links = find_inbound_links(
            "/target/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
            "index.md",
        );
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
            "index.md",
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
            "index.md",
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
            "index.md",
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
            "index.md",
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
            "index.md",
        );

        assert_eq!(links.len(), 1);
        // Canonical URL: `coins/index.md` is served at `/coins/`, not
        // `/coins/index/`. The relative `./tricks/…` link still resolves,
        // because the folder used for matching stays `/coins/`.
        assert_eq!(links[0].from, "/coins/");
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
            "index.md",
        );

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].from, "/coins/");
    }

    // ========== canonical `from` URLs (index files) ==========

    #[test]
    fn find_inbound_links_reports_index_source_with_canonical_url() {
        // A backlink from `docs/index.md` must be reported as `/docs/` — the URL
        // the page is actually served at — not the positional `/docs/index/`.
        let temp_dir = TempDir::new().unwrap();
        let docs = temp_dir.path().join("docs");
        fs::create_dir_all(&docs).unwrap();

        fs::write(temp_dir.path().join("target.md"), "# Target").unwrap();
        fs::write(docs.join("index.md"), "See [Target](../target/).").unwrap();

        let links = find_inbound_links(
            "/target/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
            "index.md",
        );

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].from, "/docs/");
    }

    #[test]
    fn find_inbound_links_skips_index_target_self_link() {
        // The target page is `docs/index.md`, served at `/docs/`. Its own
        // self-link must not be reported as an inbound link from itself.
        let temp_dir = TempDir::new().unwrap();
        let docs = temp_dir.path().join("docs");
        fs::create_dir_all(&docs).unwrap();

        fs::write(docs.join("index.md"), "Back to [me](/docs/).").unwrap();
        fs::write(
            temp_dir.path().join("other.md"),
            "See [Docs](/docs/) for more.",
        )
        .unwrap();

        let links = find_inbound_links(
            "/docs/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
            "index.md",
        );

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].from, "/other/");
    }

    #[test]
    fn find_inbound_links_handles_uppercase_markdown_extension() {
        // `.MD` passes the (lowercased) extension filter, so its URL must be
        // built the same way `.md` is.
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("target.md"), "# Target").unwrap();
        fs::write(temp_dir.path().join("SOURCE.MD"), "See [t](target/).").unwrap();

        let links = find_inbound_links(
            "/target/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
            "index.md",
        );

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].from, "/SOURCE/");
    }

    // ========== bare wiki names (title / alias / stem) ==========

    #[test]
    fn find_inbound_links_matches_title_and_alias_wikilinks() {
        // `[[Patrick Walsh]]` / `[[PW]]` resolve through the wikilink index by
        // title and alias; server-mode backlinks must see them too.
        let temp_dir = TempDir::new().unwrap();
        let people = temp_dir.path().join("people");
        let notes = temp_dir.path().join("notes");
        fs::create_dir_all(&people).unwrap();
        fs::create_dir_all(&notes).unwrap();

        fs::write(
            people.join("pw.md"),
            "---\ntitle: Patrick Walsh\naliases: [PW]\n---\n\n# Patrick Walsh\n",
        )
        .unwrap();
        fs::write(
            notes.join("family.md"),
            "See [[Patrick Walsh]] and also [[PW]].",
        )
        .unwrap();

        let links = find_inbound_links(
            "/people/pw/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
            "index.md",
        );

        // Two matches on one page dedupe to a single inbound link.
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].from, "/notes/family/");
    }

    #[test]
    fn find_inbound_links_matches_bare_stem_wikilink_from_another_folder() {
        let temp_dir = TempDir::new().unwrap();
        let people = temp_dir.path().join("people");
        let notes = temp_dir.path().join("notes");
        fs::create_dir_all(&people).unwrap();
        fs::create_dir_all(&notes).unwrap();

        fs::write(people.join("pw.md"), "# Patrick Walsh\n").unwrap();
        fs::write(notes.join("family.md"), "See [[pw]] for details.").unwrap();

        let links = find_inbound_links(
            "/people/pw/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
            "index.md",
        );

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].from, "/notes/family/");
    }

    #[test]
    fn find_inbound_links_wiki_names_do_not_match_inline_link_targets() {
        // A name is a `[[…]]` target, never a URL: `[x](Patrick Walsh)` must not
        // be reported as a backlink.
        let temp_dir = TempDir::new().unwrap();
        let people = temp_dir.path().join("people");
        let notes = temp_dir.path().join("notes");
        fs::create_dir_all(&people).unwrap();
        fs::create_dir_all(&notes).unwrap();

        fs::write(people.join("pw.md"), "---\ntitle: Patrick Walsh\n---\n").unwrap();
        fs::write(notes.join("family.md"), "See [him](Patrick Walsh).").unwrap();

        let links = find_inbound_links(
            "/people/pw/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
            "index.md",
        );

        assert!(links.is_empty(), "unexpected links: {links:?}");
    }

    // ========== code blocks / spans ==========

    #[test]
    fn find_inbound_links_ignores_links_inside_code_blocks() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("target.md"), "# Target").unwrap();

        // Backtick fence.
        fs::write(
            temp_dir.path().join("fenced.md"),
            "How to link:\n\n```markdown\n[example](target/)\n```\n",
        )
        .unwrap();
        // Tilde fence (which may itself contain backticks).
        fs::write(
            temp_dir.path().join("tilde.md"),
            "Example:\n\n~~~\n[example](/target/) and `code`\n~~~\n",
        )
        .unwrap();
        // Inline code span.
        fs::write(
            temp_dir.path().join("span.md"),
            "Write `[example](target/)` to link.",
        )
        .unwrap();
        // Wiki link inside a fence.
        fs::write(
            temp_dir.path().join("wiki_fence.md"),
            "```\n[[target]]\n```\n",
        )
        .unwrap();

        let links = find_inbound_links(
            "/target/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
            "index.md",
        );

        assert!(links.is_empty(), "unexpected links: {links:?}");
    }

    #[test]
    fn find_inbound_links_still_finds_links_around_code_blocks() {
        // Stripping code must not swallow the prose around it.
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("target.md"), "# Target").unwrap();
        fs::write(
            temp_dir.path().join("source.md"),
            "Run `cargo test` first.\n\n```sh\necho hi\n```\n\nThen see [Target](target/).",
        )
        .unwrap();

        let links = find_inbound_links(
            "/target/",
            temp_dir.path(),
            &["md".to_string()],
            &[],
            &[],
            "index.md",
        );

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].from, "/source/");
        assert_eq!(links[0].text, "Target");
    }

    #[test]
    fn strip_code_regions_blanks_fences_and_spans_only() {
        let input = "a [x](y) b\n```\n[fenced](y)\n```\nc `[span](y)` d\n~~~\n[tilde](y)\n~~~\ne\n";
        let out = strip_code_regions(input);
        assert!(out.contains("[x](y)"));
        assert!(!out.contains("[fenced](y)"));
        assert!(!out.contains("[span](y)"));
        assert!(!out.contains("[tilde](y)"));
        assert!(out.contains('c') && out.contains('d') && out.contains('e'));
    }

    #[test]
    fn strip_code_regions_keeps_unclosed_backticks_literal() {
        // An unmatched backtick is literal text in CommonMark; dropping the rest
        // of the line would lose real links.
        let out = strip_code_regions("a ` b [x](y)\n");
        assert!(out.contains("[x](y)"));
    }

    #[test]
    fn strip_code_regions_leaves_code_free_content_borrowed() {
        // Fast path: files without fences or spans must not be reallocated.
        assert!(matches!(
            strip_code_regions("plain [x](y)\n"),
            Cow::Borrowed(_)
        ));
    }

    // ========== cross-check against the build-mode (parser) link index ==========

    /// Inverts parser-derived outbound links exactly the way
    /// `Builder::write_link_files` does, and returns the inbound set for
    /// `target_url`, sorted for comparison.
    fn parser_inbound_links(
        root: &Path,
        pages: &[(&str, &str, bool)],
        target_url: &str,
    ) -> Vec<(String, String)> {
        use crate::link_index::resolve_relative_url;
        use crate::link_transform::LinkTransformConfig;

        let mut inbound: Vec<(String, String)> = Vec::new();
        for (rel_path, page_url, is_index) in pages {
            let rendered = crate::markdown::render_sync(
                root.join(rel_path),
                root,
                0,
                LinkTransformConfig {
                    markdown_extensions: vec!["md".to_string()],
                    index_file: "index.md".to_string(),
                    is_index_file: *is_index,
                    url_depth: None,
                    current_page_url: (*page_url).to_string(),
                },
                None,
                true,
                false,
                HashSet::new(),
                false,
                &[],
                None,
            )
            .expect("render");

            for link in rendered.outbound_links {
                if !link.internal || link.to.is_empty() {
                    continue;
                }
                if resolve_relative_url(page_url, &link.to, *is_index) == target_url {
                    inbound.push(((*page_url).to_string(), link.text));
                }
            }
        }
        inbound.sort();
        inbound
    }

    #[test]
    fn find_inbound_links_agrees_with_parser_derived_index() {
        // One nested fixture exercising every link shape at once: parent-relative,
        // `./`-relative, absolute, a wiki link, and a code-fenced decoy. The
        // grep-based (server) and parser-derived (build) inbound sets must match.
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();
        let docs = root.join("docs");
        let notes = root.join("notes");
        fs::create_dir_all(docs.join("sub")).unwrap();
        fs::create_dir_all(&notes).unwrap();

        fs::write(docs.join("guide.md"), "# Guide\n").unwrap();
        // ../guide/ from a nested folder
        fs::write(
            docs.join("sub").join("deep.md"),
            "Up to [Guide](../guide/).\n",
        )
        .unwrap();
        // ./guide/ from the same folder
        fs::write(docs.join("intro.md"), "See [Guide](./guide/).\n").unwrap();
        // absolute
        fs::write(notes.join("abs.md"), "See [Guide](/docs/guide/).\n").unwrap();
        // wiki link (same-folder stem)
        fs::write(docs.join("wiki.md"), "See [[guide]].\n").unwrap();
        // code-fenced decoy: must be reported by neither implementation
        fs::write(
            notes.join("decoy.md"),
            "Example syntax:\n\n```markdown\n[Guide](/docs/guide/)\n```\n",
        )
        .unwrap();

        let grep: Vec<(String, String)> = {
            let mut v: Vec<(String, String)> = find_inbound_links(
                "/docs/guide/",
                root,
                &["md".to_string()],
                &[],
                &[],
                "index.md",
            )
            .into_iter()
            .map(|l| (l.from, l.text))
            .collect();
            v.sort();
            v
        };

        let parser = parser_inbound_links(
            root,
            &[
                ("docs/sub/deep.md", "/docs/sub/deep/", false),
                ("docs/intro.md", "/docs/intro/", false),
                ("notes/abs.md", "/notes/abs/", false),
                ("docs/wiki.md", "/docs/wiki/", false),
                ("notes/decoy.md", "/notes/decoy/", false),
            ],
            "/docs/guide/",
        );

        assert_eq!(grep, parser);
        assert_eq!(grep.len(), 4, "expected four real backlinks: {grep:?}");
        assert!(
            !grep.iter().any(|(from, _)| from == "/notes/decoy/"),
            "code-fenced link must not be a backlink: {grep:?}"
        );
    }
}

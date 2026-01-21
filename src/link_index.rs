//! Link tracking module for bidirectional link management.
//!
//! Provides data structures and caching for tracking inbound and outbound links
//! between markdown pages, enabling wiki-style backlink features.

use papaya::HashMap as ConcurrentHashMap;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// An outbound link from a markdown page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutboundLink {
    /// The target URL path (e.g., "/docs/guide/")
    pub to: String,
    /// The link text displayed to users
    pub text: String,
    /// Optional anchor fragment (e.g., "#section")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    /// Whether this is an internal link (true) or external (false)
    pub internal: bool,
}

/// An inbound link pointing to a markdown page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InboundLink {
    /// The source page URL path that contains the link
    pub from: String,
    /// The link text used in the source page
    pub text: String,
    /// Optional anchor fragment targeted
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
}

/// Links data for a single page (used in API responses).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PageLinks {
    /// Links pointing to this page from other pages
    pub inbound: Vec<InboundLink>,
    /// Links from this page to other pages
    pub outbound: Vec<OutboundLink>,
}

/// Splits a URL into its path and anchor components.
///
/// # Examples
/// ```ignore
/// let (path, anchor) = split_url_anchor("/docs/guide/#section");
/// assert_eq!(path, "/docs/guide/");
/// assert_eq!(anchor, Some("#section".to_string()));
/// ```
pub fn split_url_anchor(url: &str) -> (String, Option<String>) {
    if let Some(hash_pos) = url.find('#') {
        let path = url[..hash_pos].to_string();
        let anchor = Some(url[hash_pos..].to_string());
        (path, anchor)
    } else {
        (url.to_string(), None)
    }
}

/// Determines if a URL is an internal link.
///
/// Internal links are relative paths or absolute paths starting with '/'.
/// External links start with a protocol (http://, https://, mailto:, etc.).
pub fn is_internal_link(url: &str) -> bool {
    // External links start with a protocol
    if url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("//")
        || url.starts_with("mailto:")
        || url.starts_with("tel:")
        || url.starts_with("javascript:")
        || url.starts_with("data:")
    {
        return false;
    }

    // Anchor-only links (e.g., "#section") are internal
    // Relative paths (e.g., "../page/") are internal
    // Absolute paths (e.g., "/docs/") are internal
    true
}

/// Normalizes a URL path for consistent comparison.
///
/// - Removes query strings
/// - Ensures trailing slash for directory-style paths
/// - Resolves relative path components (../, ./)
pub fn normalize_url_path(url: &str) -> String {
    // Remove query string
    let url = url.split('?').next().unwrap_or(url);

    // Remove anchor for normalization (we track it separately)
    let (path, _anchor) = split_url_anchor(url);

    // Ensure trailing slash for paths that look like directories
    // (no file extension in the last component)
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        return "/".to_string();
    }

    // Check if the last component has a file extension
    let last_component = path.rsplit('/').next().unwrap_or(path);
    if last_component.contains('.') && !last_component.starts_with('.') {
        // Looks like a file path, don't add trailing slash
        path.to_string()
    } else {
        // Directory-style path, add trailing slash
        format!("{}/", path)
    }
}

/// Resolves a relative URL against a base URL path.
///
/// Given a source page URL (e.g., "/docs/page/") and a relative link (e.g., "../other/"),
/// returns the absolute target URL (e.g., "/other/").
///
/// In mbr, URLs like "/docs/page/" correspond to files like "docs/page.md".
/// Relative links are resolved against the file's parent directory (e.g., "docs/").
///
/// # Examples
/// - resolve_relative_url("/source/", "target/") → "/target/" (sibling in root)
/// - resolve_relative_url("/docs/guide/", "intro/") → "/docs/intro/" (sibling in docs/)
/// - resolve_relative_url("/docs/guide/", "../other/") → "/other/" (up from docs/ to root)
pub fn resolve_relative_url(base_url: &str, relative_url: &str) -> String {
    // If the relative URL is already absolute, just normalize it
    if relative_url.starts_with('/') {
        let trimmed = relative_url.trim_end_matches('/');
        return if trimmed.is_empty() {
            "/".to_string()
        } else {
            format!("{}/", trimmed)
        };
    }

    // Anchor-only links stay as-is
    if relative_url.starts_with('#') {
        return relative_url.to_string();
    }

    // Split base URL into path segments
    // The base URL like "/docs/guide/" represents a FILE in directory "/docs/"
    // So we treat the last segment as the filename and start from its parent directory
    let base_segments: Vec<&str> = base_url
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    // Remove the last segment (the "filename" part) to get the parent directory
    let mut segments: Vec<&str> = if !base_segments.is_empty() {
        base_segments[..base_segments.len() - 1].to_vec()
    } else {
        vec![]
    };

    // Process each segment of the relative URL
    for part in relative_url.split('/') {
        match part {
            "" | "." => {} // Skip empty or current directory
            ".." => {
                segments.pop(); // Go up one directory
            }
            segment => {
                segments.push(segment); // Add the segment
            }
        }
    }

    // Reconstruct the absolute URL
    if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}/", segments.join("/"))
    }
}

/// Resolves all relative URLs in outbound links to absolute URLs.
///
/// This is used before caching outbound links so the frontend can use them directly.
pub fn resolve_outbound_links(base_url: &str, links: Vec<OutboundLink>) -> Vec<OutboundLink> {
    links
        .into_iter()
        .map(|mut link| {
            if link.internal && !link.to.starts_with('/') && !link.to.starts_with('#') {
                link.to = resolve_relative_url(base_url, &link.to);
            }
            link
        })
        .collect()
}

/// A cached entry for outbound links.
#[derive(Clone)]
struct LinkCacheEntry {
    /// The cached outbound links
    links: Vec<OutboundLink>,
    /// When this entry was inserted
    inserted_at: Instant,
    /// Estimated memory size in bytes
    size_bytes: usize,
}

/// Thread-safe cache for outbound links per page.
///
/// Used in server mode to cache links extracted during page renders,
/// avoiding re-parsing when the links.json endpoint is requested.
pub struct LinkCache {
    /// The underlying concurrent cache (url_path -> outbound links)
    cache: ConcurrentHashMap<String, LinkCacheEntry>,
    /// Current total size in bytes (approximate)
    current_size: AtomicUsize,
    /// Maximum allowed size in bytes
    max_size: usize,
}

impl LinkCache {
    /// Creates a new cache with the specified maximum size in bytes.
    ///
    /// Set `max_size_bytes` to 0 to disable caching.
    pub fn new(max_size_bytes: usize) -> Self {
        Self {
            cache: ConcurrentHashMap::new(),
            current_size: AtomicUsize::new(0),
            max_size: max_size_bytes,
        }
    }

    /// Retrieves cached outbound links for a page if present.
    pub fn get(&self, url_path: &str) -> Option<Vec<OutboundLink>> {
        if self.max_size == 0 {
            return None;
        }

        let guard = self.cache.pin();
        guard.get(url_path).map(|entry| {
            tracing::debug!("link cache hit: {}", url_path);
            entry.links.clone()
        })
    }

    /// Inserts outbound links into the cache.
    ///
    /// If the cache exceeds its size limit, oldest entries are evicted.
    pub fn insert(&self, url_path: String, links: Vec<OutboundLink>) {
        if self.max_size == 0 {
            return;
        }

        // Estimate size: URL + links (rough estimate based on string lengths)
        let size_bytes = url_path.len()
            + links
                .iter()
                .map(|l| {
                    l.to.len() + l.text.len() + l.anchor.as_ref().map(|a| a.len()).unwrap_or(0) + 32
                })
                .sum::<usize>()
            + std::mem::size_of::<LinkCacheEntry>();

        let entry = LinkCacheEntry {
            links,
            inserted_at: Instant::now(),
            size_bytes,
        };

        self.cache.pin().insert(url_path.clone(), entry);
        let new_size = self.current_size.fetch_add(size_bytes, Ordering::Relaxed) + size_bytes;

        tracing::debug!("link cached: {} ({} bytes)", url_path, size_bytes);

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
            .map(|(k, v)| (k.clone(), v.inserted_at, v.size_bytes))
            .collect();

        // Sort by insertion time (oldest first)
        entries.sort_by_key(|(_, inserted_at, _)| *inserted_at);

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
                "link cache evicted {} entries ({} bytes freed)",
                evict_count,
                freed
            );
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_url_anchor_with_anchor() {
        let (path, anchor) = split_url_anchor("/docs/guide/#section");
        assert_eq!(path, "/docs/guide/");
        assert_eq!(anchor, Some("#section".to_string()));
    }

    #[test]
    fn test_split_url_anchor_without_anchor() {
        let (path, anchor) = split_url_anchor("/docs/guide/");
        assert_eq!(path, "/docs/guide/");
        assert_eq!(anchor, None);
    }

    #[test]
    fn test_split_url_anchor_only_anchor() {
        let (path, anchor) = split_url_anchor("#section");
        assert_eq!(path, "");
        assert_eq!(anchor, Some("#section".to_string()));
    }

    #[test]
    fn test_is_internal_link_external_https() {
        assert!(!is_internal_link("https://example.com"));
        assert!(!is_internal_link("http://example.com"));
    }

    #[test]
    fn test_is_internal_link_external_protocols() {
        assert!(!is_internal_link("mailto:test@example.com"));
        assert!(!is_internal_link("tel:+1234567890"));
        assert!(!is_internal_link("javascript:void(0)"));
        assert!(!is_internal_link("data:text/html,<h1>Hi</h1>"));
        assert!(!is_internal_link("//cdn.example.com/script.js"));
    }

    #[test]
    fn test_is_internal_link_internal() {
        assert!(is_internal_link("/docs/guide/"));
        assert!(is_internal_link("../other-page/"));
        assert!(is_internal_link("./sibling/"));
        assert!(is_internal_link("relative-path/"));
        assert!(is_internal_link("#anchor"));
    }

    #[test]
    fn test_normalize_url_path_trailing_slash() {
        assert_eq!(normalize_url_path("/docs/guide"), "/docs/guide/");
        assert_eq!(normalize_url_path("/docs/guide/"), "/docs/guide/");
    }

    #[test]
    fn test_normalize_url_path_file() {
        assert_eq!(normalize_url_path("/images/photo.jpg"), "/images/photo.jpg");
        assert_eq!(normalize_url_path("/docs/file.pdf"), "/docs/file.pdf");
    }

    #[test]
    fn test_normalize_url_path_with_query() {
        assert_eq!(normalize_url_path("/docs/guide/?foo=bar"), "/docs/guide/");
    }

    #[test]
    fn test_normalize_url_path_root() {
        assert_eq!(normalize_url_path("/"), "/");
        assert_eq!(normalize_url_path(""), "/");
    }

    #[test]
    fn test_link_cache_insert_and_get() {
        let cache = LinkCache::new(1024 * 1024);
        let links = vec![OutboundLink {
            to: "/other/".to_string(),
            text: "Other Page".to_string(),
            anchor: None,
            internal: true,
        }];

        cache.insert("/docs/".to_string(), links.clone());

        let retrieved = cache.get("/docs/");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap(), links);
    }

    #[test]
    fn test_link_cache_miss() {
        let cache = LinkCache::new(1024 * 1024);
        assert!(cache.get("/nonexistent/").is_none());
    }

    #[test]
    fn test_link_cache_disabled() {
        let cache = LinkCache::new(0);
        let links = vec![OutboundLink {
            to: "/other/".to_string(),
            text: "Other".to_string(),
            anchor: None,
            internal: true,
        }];

        cache.insert("/docs/".to_string(), links);
        assert!(cache.get("/docs/").is_none());
    }

    #[test]
    fn test_outbound_link_serialize() {
        let link = OutboundLink {
            to: "/docs/guide/".to_string(),
            text: "Guide".to_string(),
            anchor: Some("#intro".to_string()),
            internal: true,
        };

        let json = serde_json::to_string(&link).unwrap();
        assert!(json.contains("\"to\":\"/docs/guide/\""));
        assert!(json.contains("\"text\":\"Guide\""));
        assert!(json.contains("\"anchor\":\"#intro\""));
        assert!(json.contains("\"internal\":true"));
    }

    #[test]
    fn test_outbound_link_serialize_no_anchor() {
        let link = OutboundLink {
            to: "/docs/".to_string(),
            text: "Docs".to_string(),
            anchor: None,
            internal: true,
        };

        let json = serde_json::to_string(&link).unwrap();
        // anchor should be skipped when None
        assert!(!json.contains("anchor"));
    }

    #[test]
    fn test_page_links_default() {
        let links = PageLinks::default();
        assert!(links.inbound.is_empty());
        assert!(links.outbound.is_empty());
    }
}

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

/// Sorts inbound links into a deterministic, mode-independent order.
///
/// Both producers of an inbound list build it by iterating a hash map whose
/// order is randomly seeded per process: the builder inverts `build_link_index`
/// (a `papaya::HashMap`, see `build::Builder::write_link_files`) and the
/// server's grep walks `folder_files` (a std `HashMap`, see
/// `link_grep::find_inbound_links`). Without a sort the same repository emits a
/// different `links.json` on every build, and the two modes disagree on the
/// ordering of the same page's backlinks.
///
/// The key is the struct's full contents rather than `from` alone. Today
/// `from` happens to be unique within a list — the builder's source of truth
/// dedups outbound links by target (`markdown::finalize_render`) and the
/// server's grep dedups backlinks by source — so `from` alone would sort
/// correctly. That is a property of two distant call sites, not of this type:
/// if either dedup is relaxed, `from` stops being a total order, and because
/// [`slice::sort_by`] is stable the leftover ties would silently preserve the
/// hash order this function exists to remove. Sorting on the full contents
/// costs nothing and does not depend on that invariant holding.
pub fn sort_inbound_links(links: &mut [InboundLink]) {
    links.sort_by(|a, b| (&a.from, &a.text, &a.anchor).cmp(&(&b.from, &b.text, &b.anchor)));
}

/// Links data for a single page (used in API responses).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PageLinks {
    /// Links pointing to this page from other pages
    pub inbound: Vec<InboundLink>,
    /// Links from this page to other pages
    pub outbound: Vec<OutboundLink>,
    /// Typed relationships (declared + derived) for this page. Empty when
    /// relationship tracking is disabled.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relationships: Vec<crate::relationships::ResolvedRelationship>,
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
/// Internal links are relative paths, absolute paths starting with `/`, and
/// fragment-/query-only references. Anything carrying a URL scheme (or a
/// protocol-relative `//host`) is external. The predicate itself lives in
/// [`crate::url_path::is_external_url`] so link transformation, tracking and
/// validation cannot disagree about a given href.
pub fn is_internal_link(url: &str) -> bool {
    !crate::url_path::is_external_url(url)
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
/// The `OutboundLink.to` values stored by the markdown renderer hold the
/// **original** (pre-transform) link text from the markdown source, expressed
/// relative to the source file in the markdown tree. This function converts
/// that to an absolute site URL.
///
/// Markdown-level semantics: a link like `foo.md` in `X.md` means "sibling
/// file of X.md". The base URL interpretation therefore depends on whether
/// the source page is an index file:
///
/// - **Non-index** (`docs/guide.md` → `/docs/guide/`): the last URL segment
///   is the file stem (`guide`), and siblings live in the parent directory
///   (`/docs/`). Strip the last segment before applying the relative URL.
/// - **Index** (`docs/modes/index.md` → `/modes/`): the URL already
///   represents the directory; siblings live inside it. Keep all segments.
///
/// # Examples
/// - `resolve_relative_url("/modes/", "gui/", true)` → `"/modes/gui/"`
/// - `resolve_relative_url("/docs/guide/", "intro/", false)` → `"/docs/intro/"`
/// - `resolve_relative_url("/docs/guide/", "../other/", false)` → `"/other/"`
/// - `resolve_relative_url("/source/", "../", false)` → `"/"`
pub fn resolve_relative_url(base_url: &str, relative_url: &str, is_index_file: bool) -> String {
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

    let base_segments: Vec<&str> = base_url
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    // For non-index pages, the last URL segment is the file stem — drop it
    // so relative links resolve against the parent directory. For index
    // pages, every segment is a real directory component.
    let mut segments: Vec<&str> = if is_index_file || base_segments.is_empty() {
        base_segments
    } else {
        base_segments[..base_segments.len() - 1].to_vec()
    };

    for part in relative_url.split('/') {
        match part {
            "" | "." => {} // Skip empty or current directory
            ".." => {
                segments.pop();
            }
            segment => {
                segments.push(segment);
            }
        }
    }

    if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}/", segments.join("/"))
    }
}

/// Resolves all relative URLs in outbound links to absolute URLs.
///
/// This is used before caching outbound links so the frontend can use them
/// directly. `is_index_file` indicates whether `base_url` corresponds to an
/// index markdown file; see [`resolve_relative_url`] for why this matters.
///
/// Links with an empty target are dropped. The renderer splits `#anchor` off
/// the destination before storing it, so an anchor-only link like
/// `[Top](#top)` arrives as `to: ""` with `anchor: Some("#top")` — the
/// `starts_with('#')` guard below never sees it. Such a link targets the
/// current page, not another one; resolving `""` against `base_url` used to
/// invent an outbound link to the parent directory (and, once inverted, a
/// phantom backlink on a page the author never linked to).
pub fn resolve_outbound_links(
    base_url: &str,
    links: Vec<OutboundLink>,
    is_index_file: bool,
) -> Vec<OutboundLink> {
    links
        .into_iter()
        .filter(|link| !link.to.is_empty())
        .map(|mut link| {
            if link.internal && !link.to.starts_with('/') && !link.to.starts_with('#') {
                link.to = resolve_relative_url(base_url, &link.to, is_index_file);
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

    /// Invalidates all cached entries (e.g., after a file move rewrites
    /// outbound links across many pages). Clears the map and resets the tracked
    /// size so the cache doesn't ratchet after a bulk change.
    pub fn invalidate_all(&self) {
        let guard = self.cache.pin();
        let keys: Vec<String> = guard.iter().map(|(k, _)| k.clone()).collect();
        for key in keys {
            guard.remove(&key);
        }
        self.current_size.store(0, Ordering::Relaxed);
        tracing::debug!("link cache invalidated");
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

/// One target page's backlinks, keyed by the source page URL.
///
/// A `BTreeMap` rather than a `Vec` for two reasons: withdrawing one source's
/// contribution when that file changes is O(log n) instead of a linear scan,
/// and iteration is already in `from` order, which is the order
/// [`sort_inbound_links`] produces. `from` is unique within a bucket because
/// outbound links are deduplicated by target before they get here.
type InboundBucket =
    std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<String, InboundLink>>>;

/// Repository-wide backlink index for server mode.
///
/// The server used to answer every `links.json` request by walking the entire
/// repository and grepping each markdown file for references to that one page
/// (`link_grep::find_inbound_links`). That is O(repository) per *page*, and the
/// info panel's mini graph requests many pages at once, so a large repo paid
/// for a full walk per distinct page on every cache miss.
///
/// This inverts the relationship once instead: each markdown file is parsed a
/// single time, its internal outbound links are resolved to absolute URLs, and
/// each one is recorded as a backlink on its target. Lookups are then a hash
/// hit. It is the same inversion `build::Builder::write_link_files` performs
/// for static output, which also closes the long-standing server/build
/// disagreement about which links produce backlinks — the grep matched on path
/// spelling, while both inverted indexes use the renderer's own resolution.
///
/// `contributions` is what makes incremental maintenance cheap: it remembers
/// which targets a given source page currently contributes to, so a changed
/// file's old backlinks can be withdrawn without touching any other bucket.
#[derive(Default)]
pub struct InboundIndex {
    /// target page URL -> its backlinks, keyed by source page URL.
    inbound: ConcurrentHashMap<String, InboundBucket>,
    /// source page URL -> the target URLs it currently contributes to.
    contributions: ConcurrentHashMap<String, Vec<String>>,
    /// Whether the initial full-repository pass has finished. Until it has,
    /// the index is incomplete and callers must not treat a miss as "no
    /// backlinks".
    ready: std::sync::atomic::AtomicBool,
    /// Woken when `ready` first flips. Lets callers (and tests) await the
    /// initial build instead of polling.
    ready_notify: tokio::sync::Notify,
}

impl InboundIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// True once the initial full-repository pass has completed.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// Mark the index queryable. Called after the initial pass.
    pub fn mark_ready(&self) {
        self.ready.store(true, Ordering::Release);
        self.ready_notify.notify_waiters();
    }

    /// Wait until the initial full-repository pass has finished.
    ///
    /// `notify_waiters()` stores no permit, so interest is registered *before*
    /// the flag is re-checked; otherwise a build that completes in that window
    /// would leave the caller parked forever (same pattern as
    /// `Repo::wait_for_scan`).
    pub async fn wait_until_ready(&self) {
        loop {
            let notified = self.ready_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            if self.is_ready() {
                return;
            }

            notified.await;
        }
    }

    /// Record `source_url`'s links, replacing whatever it contributed before.
    ///
    /// `links` must already be resolved to absolute URLs (see
    /// [`resolve_outbound_links`]); external links are ignored, since only an
    /// internal target can display a backlink.
    pub fn set_page_links(&self, source_url: &str, links: &[OutboundLink]) {
        self.withdraw(source_url);

        let targets: Vec<String> = links
            .iter()
            .filter(|link| link.internal && !link.to.is_empty())
            .map(|link| {
                let bucket = {
                    let guard = self.inbound.pin();
                    guard
                        .get_or_insert_with(link.to.clone(), InboundBucket::default)
                        .clone()
                };
                bucket.lock().expect("inbound bucket poisoned").insert(
                    source_url.to_string(),
                    InboundLink {
                        from: source_url.to_string(),
                        text: link.text.clone(),
                        anchor: link.anchor.clone(),
                    },
                );
                link.to.clone()
            })
            .collect();

        if targets.is_empty() {
            self.contributions.pin().remove(source_url);
        } else {
            self.contributions
                .pin()
                .insert(source_url.to_string(), targets);
        }
    }

    /// Remove every backlink contributed by `source_url` (the page was deleted).
    pub fn remove_page(&self, source_url: &str) {
        self.withdraw(source_url);
        self.contributions.pin().remove(source_url);
    }

    /// Drop `source_url`'s existing contributions from every target it fed.
    fn withdraw(&self, source_url: &str) {
        let previous = self.contributions.pin().get(source_url).cloned();
        let Some(previous) = previous else {
            return;
        };
        let guard = self.inbound.pin();
        for target in previous {
            if let Some(bucket) = guard.get(&target) {
                let mut entries = bucket.lock().expect("inbound bucket poisoned");
                entries.remove(source_url);
                // Leave the empty bucket in place: a page that lost its only
                // backlink usually gains another on the next edit, and the key
                // count is bounded by the number of link targets.
            }
        }
    }

    /// Backlinks for `target_url`, in `from` order.
    pub fn get(&self, target_url: &str) -> Vec<InboundLink> {
        let bucket = self.inbound.pin().get(target_url).cloned();
        match bucket {
            Some(bucket) => bucket
                .lock()
                .expect("inbound bucket poisoned")
                .values()
                .cloned()
                .collect(),
            None => Vec::new(),
        }
    }

    /// Drop everything, including readiness (a full rescan rebuilds it).
    pub fn clear(&self) {
        self.inbound.pin().clear();
        self.contributions.pin().clear();
        self.ready.store(false, Ordering::Release);
    }

    /// Number of pages that currently have at least one backlink recorded.
    pub fn target_count(&self) -> usize {
        self.inbound
            .pin()
            .iter()
            .filter(|(_, bucket)| !bucket.lock().expect("inbound bucket poisoned").is_empty())
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== InboundIndex ====================

    fn link_to(to: &str, text: &str) -> OutboundLink {
        OutboundLink {
            to: to.to_string(),
            text: text.to_string(),
            anchor: None,
            internal: true,
        }
    }

    #[test]
    fn inbound_index_inverts_internal_links() {
        let index = InboundIndex::new();
        index.set_page_links("/a/", &[link_to("/b/", "see B"), link_to("/c/", "see C")]);

        let b = index.get("/b/");
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].from, "/a/");
        assert_eq!(b[0].text, "see B");
        assert_eq!(index.get("/c/").len(), 1);
        assert!(index.get("/nowhere/").is_empty());
    }

    #[test]
    fn inbound_index_ignores_external_links() {
        let index = InboundIndex::new();
        let external = OutboundLink {
            to: "https://example.com/".to_string(),
            text: "out".to_string(),
            anchor: None,
            internal: false,
        };
        index.set_page_links("/a/", &[external]);

        assert!(index.get("https://example.com/").is_empty());
        assert_eq!(index.target_count(), 0);
    }

    /// Editing a page must withdraw the backlinks it no longer contains.
    /// Without `contributions` this would need a scan of every bucket, and
    /// skipping it leaves a backlink pointing at a link the author deleted.
    #[test]
    fn inbound_index_withdraws_links_removed_by_an_edit() {
        let index = InboundIndex::new();
        index.set_page_links("/a/", &[link_to("/b/", "see B"), link_to("/c/", "see C")]);
        assert_eq!(index.get("/b/").len(), 1);

        // The author deleted the link to B, kept C, and added D.
        index.set_page_links("/a/", &[link_to("/c/", "see C"), link_to("/d/", "see D")]);

        assert!(
            index.get("/b/").is_empty(),
            "a link removed by an edit must stop producing a backlink"
        );
        assert_eq!(index.get("/c/").len(), 1);
        assert_eq!(index.get("/d/").len(), 1);
    }

    #[test]
    fn inbound_index_removing_a_page_drops_only_its_own_backlinks() {
        let index = InboundIndex::new();
        index.set_page_links("/a/", &[link_to("/target/", "from A")]);
        index.set_page_links("/b/", &[link_to("/target/", "from B")]);
        assert_eq!(index.get("/target/").len(), 2);

        index.remove_page("/a/");

        let remaining = index.get("/target/");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].from, "/b/");
    }

    /// Backlinks are returned in `from` order regardless of insertion order,
    /// matching what [`sort_inbound_links`] produces for the build path.
    #[test]
    fn inbound_index_returns_backlinks_in_from_order() {
        let index = InboundIndex::new();
        index.set_page_links("/zeta/", &[link_to("/t/", "z")]);
        index.set_page_links("/alpha/", &[link_to("/t/", "a")]);
        index.set_page_links("/mid/", &[link_to("/t/", "m")]);

        let froms: Vec<String> = index.get("/t/").into_iter().map(|l| l.from).collect();
        assert_eq!(froms, vec!["/alpha/", "/mid/", "/zeta/"]);

        let mut expected = index.get("/t/");
        sort_inbound_links(&mut expected);
        assert_eq!(index.get("/t/"), expected, "must already be sorted");
    }

    #[test]
    fn inbound_index_clear_resets_readiness() {
        let index = InboundIndex::new();
        index.set_page_links("/a/", &[link_to("/b/", "see B")]);
        index.mark_ready();
        assert!(index.is_ready());

        index.clear();

        assert!(!index.is_ready());
        assert!(index.get("/b/").is_empty());
    }

    #[tokio::test]
    async fn inbound_index_wait_until_ready_returns_after_late_build() {
        let index = std::sync::Arc::new(InboundIndex::new());

        let waiting = std::sync::Arc::clone(&index);
        let waiter = tokio::spawn(async move { waiting.wait_until_ready().await });
        tokio::task::yield_now().await;

        index.mark_ready();

        tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
            .await
            .expect("waiter must be woken by mark_ready")
            .expect("waiter task panicked");
    }

    /// A waiter created after the build already finished must not block:
    /// `notify_waiters()` leaves no permit behind.
    #[tokio::test]
    async fn inbound_index_wait_until_ready_returns_immediately_when_already_ready() {
        let index = InboundIndex::new();
        index.mark_ready();

        tokio::time::timeout(std::time::Duration::from_secs(5), index.wait_until_ready())
            .await
            .expect("a waiter started after readiness must not block");
    }

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
    fn test_is_internal_link_schemes_missing_from_old_allowlist() {
        // Regression: these schemes were absent from the hand-rolled list, so
        // the links were marked internal, resolved against the page URL and
        // then reported as broken internal links.
        assert!(!is_internal_link("ftp://ftp.example.com/pub/file.zip"));
        assert!(!is_internal_link("ftps://ftp.example.com/pub/file.zip"));
        assert!(!is_internal_link("magnet:?xt=urn:btih:c12fe1c06bba"));
        assert!(!is_internal_link("sms:+15555550123"));
        assert!(!is_internal_link("callto:+15555550123"));
        assert!(!is_internal_link("blob:http://localhost:5220/550e8400"));
    }

    #[test]
    fn test_is_internal_link_windows_drive_and_colon_in_path() {
        // A drive letter and a colon in a later segment must not read as
        // schemes, or real files would be treated as off-site targets.
        assert!(is_internal_link("C:/win/path"));
        assert!(is_internal_link("docs/a:b.md"));
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
    fn test_link_cache_size_stable_on_overwrite() {
        // Regression for #12: overwriting an existing key must not ratchet
        // `current_size` upward.
        let cache = LinkCache::new(1024 * 1024);
        let links = vec![OutboundLink {
            to: "/other/".to_string(),
            text: "Other Page".to_string(),
            anchor: None,
            internal: true,
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
    fn test_link_cache_size_tracks_replacement_delta() {
        // Overwriting with a larger payload adjusts size by the delta only.
        let cache = LinkCache::new(1024 * 1024);
        let one = vec![OutboundLink {
            to: "/a/".to_string(),
            text: "A".to_string(),
            anchor: None,
            internal: true,
        }];
        let two = vec![
            OutboundLink {
                to: "/a/".to_string(),
                text: "A".to_string(),
                anchor: None,
                internal: true,
            },
            OutboundLink {
                to: "/b/".to_string(),
                text: "B".to_string(),
                anchor: None,
                internal: true,
            },
        ];

        cache.insert("/docs/".to_string(), one.clone());
        let small = cache.current_size();
        cache.insert("/docs/".to_string(), two.clone());
        let large = cache.current_size();

        assert_eq!(cache.len(), 1);
        // Growth equals a single extra link's accounted size, not the full
        // two-link entry stacked on top of the one-link entry.
        let expected_delta = "/b/".len() + "B".len() + 32;
        assert_eq!(large - small, expected_delta);
    }

    #[test]
    fn test_link_cache_invalidate_all_clears_and_resets_size() {
        let cache = LinkCache::new(1024 * 1024);
        let links = vec![OutboundLink {
            to: "/other/".to_string(),
            text: "Other".to_string(),
            anchor: None,
            internal: true,
        }];
        cache.insert("/a/".to_string(), links.clone());
        cache.insert("/b/".to_string(), links);
        assert_eq!(cache.len(), 2);
        assert!(cache.current_size() > 0);

        cache.invalidate_all();

        assert!(cache.is_empty());
        assert_eq!(cache.current_size(), 0);
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

    fn inbound(from: &str, text: &str, anchor: Option<&str>) -> InboundLink {
        InboundLink {
            from: from.to_string(),
            text: text.to_string(),
            anchor: anchor.map(str::to_string),
        }
    }

    #[test]
    fn test_sort_inbound_links_orders_by_source() {
        let mut links = vec![
            inbound("/zeta/", "Zeta", None),
            inbound("/docs/alpha/", "Alpha", None),
            inbound("/docs/beta/", "Beta", None),
        ];
        sort_inbound_links(&mut links);

        let sources: Vec<&str> = links.iter().map(|l| l.from.as_str()).collect();
        assert_eq!(sources, ["/docs/alpha/", "/docs/beta/", "/zeta/"]);
    }

    /// The whole point of the function: two lists holding the same backlinks in
    /// different (hash-seeded) orders must converge on identical output, or
    /// `links.json` churns between builds.
    #[test]
    fn test_sort_inbound_links_is_order_independent() {
        let mut forward = vec![
            inbound("/a/", "First", None),
            inbound("/b/", "Second", None),
            inbound("/c/", "Third", None),
        ];
        let mut reversed: Vec<InboundLink> = forward.iter().rev().cloned().collect();

        sort_inbound_links(&mut forward);
        sort_inbound_links(&mut reversed);

        assert_eq!(forward, reversed);
    }

    /// `from` alone is not assumed to be unique: entries sharing a source fall
    /// back to `text`, then `anchor`, so no pair is left tied and reordered by
    /// the stable sort.
    #[test]
    fn test_sort_inbound_links_breaks_ties_on_text_then_anchor() {
        let mut links = vec![
            inbound("/src/", "Beta", None),
            inbound("/src/", "Alpha", Some("#z")),
            inbound("/src/", "Alpha", Some("#a")),
            inbound("/src/", "Alpha", None),
        ];
        sort_inbound_links(&mut links);

        let keys: Vec<(&str, Option<&str>)> = links
            .iter()
            .map(|l| (l.text.as_str(), l.anchor.as_deref()))
            .collect();
        // `None` sorts before `Some` for Option, so the anchorless link leads.
        assert_eq!(
            keys,
            [
                ("Alpha", None),
                ("Alpha", Some("#a")),
                ("Alpha", Some("#z")),
                ("Beta", None),
            ]
        );
    }

    #[test]
    fn test_sort_inbound_links_handles_empty() {
        let mut links: Vec<InboundLink> = Vec::new();
        sort_inbound_links(&mut links);
        assert!(links.is_empty());
    }

    #[test]
    fn test_resolve_relative_url_index_page_sibling() {
        // docs/modes/index.md → URL /modes/ → [GUI Mode](gui/) lands at /modes/gui/
        assert_eq!(resolve_relative_url("/modes/", "gui/", true), "/modes/gui/");
        assert_eq!(
            resolve_relative_url("/reference/", "cli/", true),
            "/reference/cli/"
        );
        assert_eq!(
            resolve_relative_url("/getting-started/", "quickstart/", true),
            "/getting-started/quickstart/"
        );
    }

    #[test]
    fn test_resolve_relative_url_nonindex_sibling() {
        // docs/guide.md → URL /docs/guide/ with link [intro](intro.md) means
        // sibling file docs/intro.md, which lives at URL /docs/intro/.
        assert_eq!(
            resolve_relative_url("/docs/guide/", "intro/", false),
            "/docs/intro/"
        );
    }

    #[test]
    fn test_resolve_relative_url_nonindex_parent_traversal() {
        // docs/sub/guide.md → URL /docs/sub/guide/ with link ../other/ means
        // go up from docs/sub/ to docs/, then into other/.
        assert_eq!(
            resolve_relative_url("/docs/sub/guide/", "../other/", false),
            "/docs/other/"
        );
    }

    #[test]
    fn test_resolve_relative_url_index_parent_traversal() {
        // docs/modes/index.md with link ../other/ goes up from /modes/ to /,
        // then into other/.
        assert_eq!(
            resolve_relative_url("/modes/", "../other/", true),
            "/other/"
        );
    }

    #[test]
    fn test_resolve_relative_url_absolute_unchanged() {
        // is_index_file value doesn't matter for absolute URLs
        assert_eq!(resolve_relative_url("/modes/", "/other/", true), "/other/");
        assert_eq!(
            resolve_relative_url("/docs/guide/", "/other/", false),
            "/other/"
        );
        assert_eq!(resolve_relative_url("/modes/", "/", true), "/");
    }

    #[test]
    fn test_resolve_relative_url_anchor_only() {
        assert_eq!(
            resolve_relative_url("/modes/", "#section", true),
            "#section"
        );
    }

    #[test]
    fn test_resolve_relative_url_to_root() {
        // Migrated from build.rs when its duplicate resolver was removed.
        assert_eq!(resolve_relative_url("/source/", "../", false), "/");
        assert_eq!(resolve_relative_url("/docs/guide/", "../../", false), "/");
        assert_eq!(resolve_relative_url("/modes/", "../", true), "/");
    }

    #[test]
    fn test_resolve_outbound_links_drops_anchor_only_link() {
        // Regression: `[Top](#top)` reaches this function already split into
        // `to: ""` + `anchor: Some("#top")`, so the `starts_with('#')` guard
        // never fired and the link resolved to the parent page (`/docs/`),
        // which then showed up as a phantom backlink there.
        let links = vec![OutboundLink {
            to: String::new(),
            text: "Top".to_string(),
            anchor: Some("#top".to_string()),
            internal: true,
        }];

        let resolved = resolve_outbound_links("/docs/guide/", links, false);

        assert!(
            resolved.is_empty(),
            "anchor-only link must not become an outbound link: {resolved:?}"
        );
    }

    #[test]
    fn test_resolve_outbound_links_keeps_real_links_alongside_anchor_only() {
        let links = vec![
            OutboundLink {
                to: String::new(),
                text: "Top".to_string(),
                anchor: Some("#top".to_string()),
                internal: true,
            },
            OutboundLink {
                to: "intro/".to_string(),
                text: "Intro".to_string(),
                anchor: None,
                internal: true,
            },
        ];

        let resolved = resolve_outbound_links("/docs/guide/", links, false);

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].to, "/docs/intro/");
    }

    #[test]
    fn test_resolve_relative_url_nonindex_nested_parent_traversal() {
        // Migrated from build.rs: strip guide → /docs/, then apply ../../.
        assert_eq!(
            resolve_relative_url("/docs/guide/", "../../other/", false),
            "/other/"
        );
    }
}

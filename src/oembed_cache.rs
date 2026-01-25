//! OEmbed cache module for caching page metadata fetches.
//!
//! Provides a thread-safe, size-bounded cache for OEmbed/OpenGraph page info
//! to avoid redundant network requests when rendering multiple markdown files.

use crate::oembed::PageInfo;
use papaya::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// A cached page info entry with metadata for eviction.
#[derive(Clone)]
struct CacheEntry {
    /// The cached page information
    info: PageInfo,
    /// When this entry was inserted (for LRU eviction)
    inserted_at: Instant,
    /// Estimated memory size in bytes
    size_bytes: usize,
}

/// Thread-safe cache for OEmbed page information.
///
/// Uses a papaya concurrent HashMap for lock-free reads and a size-based
/// eviction strategy to bound memory usage.
pub struct OembedCache {
    /// The underlying concurrent cache
    cache: HashMap<String, CacheEntry>,
    /// Current total size in bytes (approximate)
    current_size: AtomicUsize,
    /// Maximum allowed size in bytes
    max_size: usize,
}

impl OembedCache {
    /// Creates a new cache with the specified maximum size in bytes.
    ///
    /// # Arguments
    ///
    /// * `max_size_bytes` - Maximum memory to use for cached entries.
    ///   Set to 0 to disable caching entirely.
    pub fn new(max_size_bytes: usize) -> Self {
        Self {
            cache: HashMap::new(),
            current_size: AtomicUsize::new(0),
            max_size: max_size_bytes,
        }
    }

    /// Retrieves cached page info for a URL if present.
    ///
    /// Returns `None` if the URL is not in the cache.
    pub fn get(&self, url: &str) -> Option<PageInfo> {
        if self.max_size == 0 {
            return None;
        }

        let guard = self.cache.pin();
        match guard.get(url) {
            Some(entry) => {
                tracing::debug!("oembed cache hit: {}", url);
                Some(entry.info.clone())
            }
            None => {
                tracing::debug!("oembed cache miss: {}", url);
                None
            }
        }
    }

    /// Inserts page info into the cache.
    ///
    /// If the cache exceeds its size limit after insertion, oldest entries
    /// are evicted until the cache is within bounds.
    pub fn insert(&self, url: String, info: PageInfo) {
        if self.max_size == 0 {
            return;
        }

        let size_bytes = info.estimated_size() + url.len() + std::mem::size_of::<CacheEntry>();

        let entry = CacheEntry {
            info,
            inserted_at: Instant::now(),
            size_bytes,
        };

        // Insert the entry
        self.cache.pin().insert(url.clone(), entry);
        // SAFETY: Relaxed ordering is acceptable here because:
        // 1. The size tracking is approximate - we don't need perfect synchronization
        // 2. Worst case: cache temporarily exceeds max_size until next eviction
        // 3. No data dependency with other operations requires acquire/release
        let new_size = self.current_size.fetch_add(size_bytes, Ordering::Relaxed) + size_bytes;

        tracing::debug!("oembed cached: {} ({} bytes)", url, size_bytes);

        // Evict if over limit
        if new_size > self.max_size {
            self.evict_oldest(new_size - self.max_size);
        }
    }

    /// Evicts oldest entries until at least `target_bytes` have been freed.
    ///
    /// Uses a simple LRU-like strategy based on insertion time.
    fn evict_oldest(&self, target_bytes: usize) {
        // Collect entries with their timestamps
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
                // Relaxed: approximate tracking, same rationale as insert
                self.current_size.fetch_sub(size, Ordering::Relaxed);
            }
        }

        if evict_count > 0 {
            tracing::debug!(
                "oembed cache evicted {} entries ({} bytes freed)",
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

    fn make_page_info(url: &str) -> PageInfo {
        PageInfo {
            url: url.to_string(),
            title: Some("Test Title".to_string()),
            description: Some("Test description".to_string()),
            image: None,
            embed_html: None,
        }
    }

    #[test]
    fn test_insert_and_retrieve() {
        let cache = OembedCache::new(1024 * 1024); // 1MB

        let url = "https://example.com/page";
        let info = make_page_info(url);

        cache.insert(url.to_string(), info.clone());

        let retrieved = cache.get(url);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().url, url);
    }

    #[test]
    fn test_cache_miss() {
        let cache = OembedCache::new(1024 * 1024);

        let retrieved = cache.get("https://nonexistent.com");
        assert!(retrieved.is_none());
    }

    #[test]
    fn test_disabled_cache() {
        let cache = OembedCache::new(0); // Disabled

        let url = "https://example.com";
        let info = make_page_info(url);

        cache.insert(url.to_string(), info);

        // Should not be cached when max_size is 0
        assert!(cache.get(url).is_none());
    }

    #[test]
    fn test_size_tracking() {
        let cache = OembedCache::new(1024 * 1024);

        assert_eq!(cache.current_size(), 0);

        let url = "https://example.com";
        let info = make_page_info(url);

        cache.insert(url.to_string(), info);

        assert!(cache.current_size() > 0);
    }

    #[test]
    fn test_eviction_on_size_limit() {
        // Small cache that can only hold a few entries
        let cache = OembedCache::new(500);

        // Insert several entries to trigger eviction
        for i in 0..10 {
            let url = format!("https://example.com/page{}", i);
            let info = make_page_info(&url);
            cache.insert(url, info);
        }

        // Cache should have evicted some entries to stay within bounds
        // The exact number depends on entry sizes
        assert!(cache.current_size() <= 600); // Allow some slack due to concurrent operations
    }

    #[test]
    fn test_multiple_entries() {
        let cache = OembedCache::new(1024 * 1024);

        let urls = vec![
            "https://example.com/page1",
            "https://example.com/page2",
            "https://example.com/page3",
        ];

        for url in &urls {
            let info = make_page_info(url);
            cache.insert(url.to_string(), info);
        }

        assert_eq!(cache.len(), 3);

        for url in &urls {
            assert!(cache.get(url).is_some());
        }
    }

    #[test]
    fn test_overwrite_existing() {
        let cache = OembedCache::new(1024 * 1024);

        let url = "https://example.com";

        let info1 = PageInfo {
            url: url.to_string(),
            title: Some("First Title".to_string()),
            ..Default::default()
        };

        let info2 = PageInfo {
            url: url.to_string(),
            title: Some("Second Title".to_string()),
            ..Default::default()
        };

        cache.insert(url.to_string(), info1);
        cache.insert(url.to_string(), info2);

        let retrieved = cache.get(url).unwrap();
        assert_eq!(retrieved.title, Some("Second Title".to_string()));
    }
}

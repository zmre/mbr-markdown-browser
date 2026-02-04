//! OEmbed cache module for caching page metadata fetches.
//!
//! Provides a thread-safe, size-bounded cache for OEmbed/OpenGraph page info
//! to avoid redundant network requests when rendering multiple markdown files.

use crate::oembed::PageInfo;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Mutex;

/// Maximum number of entries in the oembed LRU cache.
const OEMBED_CACHE_MAX_ENTRIES: usize = 10_000;

/// A cached page info entry with size tracking.
struct CacheEntry {
    /// The cached page information
    info: PageInfo,
    /// Estimated memory size in bytes
    size_bytes: usize,
}

/// Thread-safe cache for OEmbed page information.
///
/// Uses an LRU cache with size-based eviction for O(1) get/insert
/// and bounded memory usage.
pub struct OembedCache {
    /// The underlying LRU cache, protected by a mutex.
    /// Mutex is appropriate here because operations are fast (no I/O inside the lock).
    cache: Mutex<LruCache<String, CacheEntry>>,
    /// Current total size in bytes
    current_size: Mutex<usize>,
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
        // Use a generous count-based capacity since we manage eviction by size.
        let cap = NonZeroUsize::new(OEMBED_CACHE_MAX_ENTRIES).unwrap();
        Self {
            cache: Mutex::new(LruCache::new(cap)),
            current_size: Mutex::new(0),
            max_size: max_size_bytes,
        }
    }

    /// Retrieves cached page info for a URL if present.
    ///
    /// Returns `None` if the URL is not in the cache.
    /// Promotes the entry to most-recently-used on access.
    pub fn get(&self, url: &str) -> Option<PageInfo> {
        if self.max_size == 0 {
            return None;
        }

        let mut cache = self.cache.lock().unwrap();
        match cache.get(url) {
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
    /// If the cache exceeds its size limit after insertion, least-recently-used
    /// entries are evicted until the cache is within bounds.
    pub fn insert(&self, url: String, info: PageInfo) {
        if self.max_size == 0 {
            return;
        }

        let size_bytes = info.estimated_size() + url.len() + std::mem::size_of::<CacheEntry>();

        let entry = CacheEntry { info, size_bytes };

        let mut cache = self.cache.lock().unwrap();
        let mut current_size = self.current_size.lock().unwrap();

        // If overwriting an existing entry, subtract its old size
        if let Some(old) = cache.push(url.clone(), entry) {
            *current_size -= old.1.size_bytes;
        }
        *current_size += size_bytes;

        tracing::debug!("oembed cached: {} ({} bytes)", url, size_bytes);

        // Evict LRU entries until under the size limit
        while *current_size > self.max_size {
            if let Some((_evicted_key, evicted_entry)) = cache.pop_lru() {
                *current_size -= evicted_entry.size_bytes;
            } else {
                break;
            }
        }
    }

    /// Returns the current approximate size of the cache in bytes.
    #[cfg(test)]
    pub fn current_size(&self) -> usize {
        *self.current_size.lock().unwrap()
    }

    /// Returns the number of entries in the cache.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.cache.lock().unwrap().len()
    }

    /// Returns true if the cache is empty.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.cache.lock().unwrap().is_empty()
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
        assert!(cache.current_size() <= 500);
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

//! Video metadata cache module.
//!
//! Provides a thread-safe, size-bounded cache for dynamically generated
//! video metadata (cover images, chapters, captions) to avoid redundant
//! ffmpeg operations when serving the same content multiple times.

use papaya::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// Cached video metadata content.
#[derive(Clone)]
pub enum CachedMetadata {
    /// PNG cover image bytes
    Cover(Vec<u8>),
    /// WebVTT chapters content
    Chapters(String),
    /// WebVTT captions content
    Captions(String),
    /// Marker indicating this metadata is not available for this video
    NotAvailable,
}

impl CachedMetadata {
    /// Estimate the memory size of this cached entry.
    fn estimated_size(&self) -> usize {
        match self {
            CachedMetadata::Cover(bytes) => bytes.len() + std::mem::size_of::<Self>(),
            CachedMetadata::Chapters(s) | CachedMetadata::Captions(s) => {
                s.len() + std::mem::size_of::<Self>()
            }
            CachedMetadata::NotAvailable => std::mem::size_of::<Self>(),
        }
    }
}

/// A cached entry with metadata for eviction.
#[derive(Clone)]
struct CacheEntry {
    /// The cached metadata
    data: CachedMetadata,
    /// When this entry was inserted (for LRU eviction)
    inserted_at: Instant,
    /// Estimated memory size in bytes
    size_bytes: usize,
}

/// Thread-safe cache for video metadata.
///
/// Uses a papaya concurrent HashMap for lock-free reads and a size-based
/// eviction strategy to bound memory usage.
pub struct VideoMetadataCache {
    /// The underlying concurrent cache
    cache: HashMap<String, CacheEntry>,
    /// Current total size in bytes (approximate)
    current_size: AtomicUsize,
    /// Maximum allowed size in bytes
    max_size: usize,
}

impl VideoMetadataCache {
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

    /// Retrieves cached metadata for a video path and type if present.
    ///
    /// Returns `None` if the entry is not in the cache.
    pub fn get(&self, key: &str) -> Option<CachedMetadata> {
        if self.max_size == 0 {
            return None;
        }

        let guard = self.cache.pin();
        match guard.get(key) {
            Some(entry) => {
                tracing::debug!("video metadata cache hit: {}", key);
                Some(entry.data.clone())
            }
            None => {
                tracing::debug!("video metadata cache miss: {}", key);
                None
            }
        }
    }

    /// Inserts metadata into the cache.
    ///
    /// If the cache exceeds its size limit after insertion, oldest entries
    /// are evicted until the cache is within bounds.
    pub fn insert(&self, key: String, data: CachedMetadata) {
        if self.max_size == 0 {
            return;
        }

        let size_bytes = data.estimated_size() + key.len() + std::mem::size_of::<CacheEntry>();

        let entry = CacheEntry {
            data,
            inserted_at: Instant::now(),
            size_bytes,
        };

        // Insert the entry
        self.cache.pin().insert(key.clone(), entry);
        let new_size = self.current_size.fetch_add(size_bytes, Ordering::Relaxed) + size_bytes;

        tracing::debug!("video metadata cached: {} ({} bytes)", key, size_bytes);

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

        for (key, _, size) in entries {
            if freed >= target_bytes {
                break;
            }
            if guard.remove(&key).is_some() {
                freed += size;
                evict_count += 1;
                self.current_size.fetch_sub(size, Ordering::Relaxed);
            }
        }

        if evict_count > 0 {
            tracing::debug!(
                "video metadata cache evicted {} entries ({} bytes freed)",
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

/// Build a cache key for video metadata.
///
/// Combines the video path with the metadata type to create a unique key.
pub fn cache_key(video_path: &str, metadata_type: &str) -> String {
    format!("{}::{}", video_path, metadata_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_retrieve_cover() {
        let cache = VideoMetadataCache::new(1024 * 1024); // 1MB

        let key = "videos/test.mp4::cover";
        let data = CachedMetadata::Cover(vec![0x89, 0x50, 0x4E, 0x47]); // PNG magic

        cache.insert(key.to_string(), data);

        let retrieved = cache.get(key);
        assert!(retrieved.is_some());
        match retrieved.unwrap() {
            CachedMetadata::Cover(bytes) => assert_eq!(bytes.len(), 4),
            _ => panic!("Expected Cover variant"),
        }
    }

    #[test]
    fn test_insert_and_retrieve_vtt() {
        let cache = VideoMetadataCache::new(1024 * 1024);

        let key = "videos/test.mp4::chapters";
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:01:00.000\nIntro\n\n";
        let data = CachedMetadata::Chapters(vtt.to_string());

        cache.insert(key.to_string(), data);

        let retrieved = cache.get(key);
        assert!(retrieved.is_some());
        match retrieved.unwrap() {
            CachedMetadata::Chapters(s) => assert!(s.contains("WEBVTT")),
            _ => panic!("Expected Chapters variant"),
        }
    }

    #[test]
    fn test_cache_miss() {
        let cache = VideoMetadataCache::new(1024 * 1024);
        let retrieved = cache.get("nonexistent");
        assert!(retrieved.is_none());
    }

    #[test]
    fn test_disabled_cache() {
        let cache = VideoMetadataCache::new(0); // Disabled

        let key = "videos/test.mp4::cover";
        let data = CachedMetadata::Cover(vec![1, 2, 3, 4]);

        cache.insert(key.to_string(), data);

        // Should not be cached when max_size is 0
        assert!(cache.get(key).is_none());
    }

    #[test]
    fn test_not_available_marker() {
        let cache = VideoMetadataCache::new(1024 * 1024);

        let key = "videos/test.mp4::captions";
        cache.insert(key.to_string(), CachedMetadata::NotAvailable);

        let retrieved = cache.get(key);
        assert!(retrieved.is_some());
        assert!(matches!(retrieved.unwrap(), CachedMetadata::NotAvailable));
    }

    #[test]
    fn test_size_tracking() {
        let cache = VideoMetadataCache::new(1024 * 1024);

        assert_eq!(cache.current_size(), 0);

        let key = "videos/test.mp4::cover";
        let data = CachedMetadata::Cover(vec![0; 100]);

        cache.insert(key.to_string(), data);

        assert!(cache.current_size() > 100);
    }

    #[test]
    fn test_eviction_on_size_limit() {
        // Small cache
        let cache = VideoMetadataCache::new(500);

        // Insert several entries to trigger eviction
        for i in 0..10 {
            let key = format!("videos/test{}.mp4::cover", i);
            let data = CachedMetadata::Cover(vec![0; 50]);
            cache.insert(key, data);
        }

        // Cache should have evicted some entries to stay within bounds
        assert!(cache.current_size() <= 600); // Allow some slack
    }

    #[test]
    fn test_cache_key_generation() {
        let key = cache_key("videos/foo.mp4", "cover");
        assert_eq!(key, "videos/foo.mp4::cover");

        let key = cache_key("videos/Eric Jones/Metal.mp4", "chapters");
        assert_eq!(key, "videos/Eric Jones/Metal.mp4::chapters");
    }
}

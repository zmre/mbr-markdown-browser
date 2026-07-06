//! Video metadata cache module.
//!
//! Provides a thread-safe, size-bounded cache for dynamically generated
//! video metadata (cover images, chapters, captions) to avoid redundant
//! ffmpeg operations when serving the same content multiple times.

use crate::cache::{Entry, SizeBoundedMap};
use std::path::Path;
use std::time::UNIX_EPOCH;

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

/// Thread-safe cache for video metadata.
///
/// A thin wrapper around the shared [`SizeBoundedMap`]: lock-free reads via
/// papaya with size-based, oldest-first eviction at insert time.
pub struct VideoMetadataCache {
    /// The shared size-bounded concurrent cache core
    cache: SizeBoundedMap<String, CachedMetadata>,
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
            cache: SizeBoundedMap::new(max_size_bytes),
        }
    }

    /// Retrieves cached metadata for a video path and type if present.
    ///
    /// Returns `None` if the entry is not in the cache.
    pub fn get(&self, key: &str) -> Option<CachedMetadata> {
        if self.cache.is_disabled() {
            return None;
        }

        match self.cache.with_entry(key, |entry| entry.value.clone()) {
            Some(data) => {
                tracing::debug!("video metadata cache hit: {}", key);
                Some(data)
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
        if self.cache.is_disabled() {
            return;
        }

        let size_bytes = Entry::<CachedMetadata>::weigh(data.estimated_size(), key.len());
        let (_, new_size) = self.cache.insert_weighted(key.clone(), data, size_bytes);

        tracing::debug!("video metadata cached: {} ({} bytes)", key, size_bytes);

        // Evict oldest-first if over limit
        if new_size > self.cache.max_size() {
            let stats = self
                .cache
                .evict_until_freed(new_size - self.cache.max_size(), |_, entry| {
                    Some(entry.inserted_at)
                });
            if stats.evicted > 0 {
                tracing::debug!(
                    "video metadata cache evicted {} entries ({} bytes freed)",
                    stats.evicted,
                    stats.freed
                );
            }
        }
    }

    /// Returns the current approximate size of the cache in bytes.
    #[cfg(test)]
    pub fn current_size(&self) -> usize {
        self.cache.current_size()
    }

    /// Returns the number of entries in the cache.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Returns true if the cache is empty.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

/// Build a cache key for video metadata.
///
/// Combines the video path with the metadata type to create a unique key.
pub fn cache_key(video_path: &str, metadata_type: &str) -> String {
    format!("{}::{}", video_path, metadata_type)
}

/// Build a cache key for video/PDF metadata scoped to the source file's
/// modification time.
///
/// The plain [`cache_key`] keys only on the request path and metadata type, so
/// an edited source file keeps serving stale covers/chapters/captions until the
/// process restarts, and negative (`NotAvailable`) results are cached forever.
/// Folding the source file's mtime into the key means that editing the
/// underlying file produces a different key, so the stale entry (positive or
/// negative) is naturally missed and the metadata is re-extracted.
///
/// When the mtime cannot be read (missing file, unsupported platform, or a
/// pre-epoch timestamp) a stable `0` token is used so the key stays
/// deterministic.
pub fn cache_key_with_mtime(source_file: &Path, metadata_type: &str) -> String {
    format!(
        "{}::{}::mtime={}",
        source_file.display(),
        metadata_type,
        source_file_mtime_nanos(source_file)
    )
}

/// Reads the modification time of `path` as nanoseconds since the Unix epoch,
/// returning `0` when it is unavailable.
fn source_file_mtime_nanos(path: &Path) -> u128 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_nanos())
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

    #[test]
    fn test_size_accounting_stable_on_overwrite() {
        // Regression for #12: overwriting an existing key must not ratchet
        // `current_size` upward. Repeatedly inserting the same key with the
        // same payload should leave the accounted size unchanged.
        let cache = VideoMetadataCache::new(1024 * 1024);

        let key = "videos/test.mp4::cover";
        cache.insert(key.to_string(), CachedMetadata::Cover(vec![0; 100]));

        let size_after_first = cache.current_size();
        assert_eq!(cache.len(), 1);

        for _ in 0..10 {
            cache.insert(key.to_string(), CachedMetadata::Cover(vec![0; 100]));
        }

        // Still a single entry, and size did not grow with each overwrite.
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.current_size(), size_after_first);
    }

    #[test]
    fn test_size_accounting_tracks_replacement_delta() {
        // Overwriting with a larger payload should adjust `current_size` by the
        // delta, not by the full new size on top of the old.
        let cache = VideoMetadataCache::new(1024 * 1024);

        let key = "videos/test.mp4::cover";
        cache.insert(key.to_string(), CachedMetadata::Cover(vec![0; 100]));
        let small = cache.current_size();

        cache.insert(key.to_string(), CachedMetadata::Cover(vec![0; 200]));
        let large = cache.current_size();

        assert_eq!(cache.len(), 1);
        // Growth should be exactly the payload delta (100 bytes), not ~300.
        assert_eq!(large - small, 100);
    }

    #[test]
    fn test_cache_key_with_mtime_changes_on_edit() {
        // Regression for #13: editing the source file must change the key so a
        // stale entry is missed and metadata is re-extracted.
        use filetime::{FileTime, set_file_mtime};

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("clip.mp4");
        std::fs::write(&file, b"first").unwrap();

        set_file_mtime(&file, FileTime::from_unix_time(1_000_000, 0)).unwrap();
        let key_before = cache_key_with_mtime(&file, "cover");

        // Simulate an edit with a newer modification time.
        std::fs::write(&file, b"second-edit").unwrap();
        set_file_mtime(&file, FileTime::from_unix_time(2_000_000, 0)).unwrap();
        let key_after = cache_key_with_mtime(&file, "cover");

        assert_ne!(
            key_before, key_after,
            "changed mtime must produce a different cache key"
        );
    }

    #[test]
    fn test_mtime_scoped_negative_entry_misses_after_edit() {
        // A cached `NotAvailable` result stored under an mtime-scoped key must
        // not persist across edits: after the file changes, the new key misses.
        use filetime::{FileTime, set_file_mtime};

        let cache = VideoMetadataCache::new(1024 * 1024);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("clip.mp4");
        std::fs::write(&file, b"first").unwrap();
        set_file_mtime(&file, FileTime::from_unix_time(1_000_000, 0)).unwrap();

        let key_before = cache_key_with_mtime(&file, "cover");
        cache.insert(key_before.clone(), CachedMetadata::NotAvailable);
        assert!(matches!(
            cache.get(&key_before),
            Some(CachedMetadata::NotAvailable)
        ));

        // Editing the file yields a fresh key that misses the stale negative.
        std::fs::write(&file, b"second-edit").unwrap();
        set_file_mtime(&file, FileTime::from_unix_time(2_000_000, 0)).unwrap();
        let key_after = cache_key_with_mtime(&file, "cover");
        assert!(cache.get(&key_after).is_none());
    }

    #[test]
    fn test_cache_key_with_mtime_deterministic_and_stable() {
        // Same file with unchanged mtime yields the same key on repeated calls.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("clip.mp4");
        std::fs::write(&file, b"data").unwrap();

        let a = cache_key_with_mtime(&file, "chapters");
        let b = cache_key_with_mtime(&file, "chapters");
        assert_eq!(a, b);

        // Missing files fall back to the stable `0` token rather than panicking.
        let missing = dir.path().join("nope.mp4");
        let key = cache_key_with_mtime(&missing, "cover");
        assert!(key.ends_with("::cover::mtime=0"));
    }
}

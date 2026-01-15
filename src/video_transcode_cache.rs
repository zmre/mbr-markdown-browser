//! HLS transcode cache module for caching playlists and segments.
//!
//! Provides a thread-safe, size-bounded cache for HLS playlists and transcoded
//! video segments to avoid redundant transcoding operations. Handles concurrent
//! requests by ensuring only one transcode runs per segment.

use crate::video_transcode::{TranscodeError, TranscodeTarget};
use papaya::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use tokio::sync::Notify;

/// Cache key for HLS content (playlists and segments).
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum HlsCacheKey {
    /// Key for an HLS playlist (.m3u8)
    Playlist {
        path: PathBuf,
        target: TranscodeTarget,
    },
    /// Key for an HLS segment (.ts)
    Segment {
        path: PathBuf,
        target: TranscodeTarget,
        segment_index: u32,
    },
}

impl HlsCacheKey {
    /// Creates a new playlist cache key.
    pub fn playlist(path: PathBuf, target: TranscodeTarget) -> Self {
        Self::Playlist { path, target }
    }

    /// Creates a new segment cache key.
    pub fn segment(path: PathBuf, target: TranscodeTarget, segment_index: u32) -> Self {
        Self::Segment {
            path,
            target,
            segment_index,
        }
    }
}

/// State of a cache entry.
#[derive(Clone)]
pub enum HlsCacheState {
    /// Content generation is in progress - wait on the notify signal
    InProgress(Arc<Notify>),
    /// Content is ready (playlist text or segment binary data)
    Complete(Arc<Vec<u8>>),
    /// Content generation failed with an error message
    Failed(String),
}

/// A cached entry with metadata for eviction.
struct CacheEntry {
    /// The cache state
    state: HlsCacheState,
    /// When this entry was created
    created_at: Instant,
    /// Memory size in bytes (only for Complete state)
    size_bytes: usize,
}

/// Thread-safe cache for HLS playlists and segments.
///
/// Uses a papaya concurrent HashMap for lock-free reads and a size-based
/// eviction strategy to bound memory usage.
pub struct HlsCache {
    /// The underlying concurrent cache
    cache: HashMap<HlsCacheKey, CacheEntry>,
    /// Current total size in bytes (only counts Complete entries)
    current_size: AtomicUsize,
    /// Maximum allowed size in bytes
    max_size: usize,
}

impl HlsCache {
    /// Creates a new cache with the specified maximum size in bytes.
    ///
    /// # Arguments
    ///
    /// * `max_size_bytes` - Maximum memory to use for cached content.
    ///   Set to 0 to disable caching entirely.
    pub fn new(max_size_bytes: usize) -> Self {
        Self {
            cache: HashMap::new(),
            current_size: AtomicUsize::new(0),
            max_size: max_size_bytes,
        }
    }

    /// Gets the current state of a cache entry.
    ///
    /// Returns `None` if no entry exists for this key.
    pub fn get_state(&self, key: &HlsCacheKey) -> Option<HlsCacheState> {
        if self.max_size == 0 {
            return None;
        }

        let guard = self.cache.pin();
        guard.get(key).map(|entry| entry.state.clone())
    }

    /// Marks content generation as in-progress and returns the Notify to signal on completion.
    ///
    /// If generation is already in progress or complete, returns the existing state.
    /// This ensures only one generation runs per key.
    pub fn start_generation(&self, key: HlsCacheKey) -> HlsCacheStartResult {
        if self.max_size == 0 {
            return HlsCacheStartResult::CacheDisabled;
        }

        let guard = self.cache.pin();

        // Check if already exists
        if let Some(entry) = guard.get(&key) {
            return match &entry.state {
                HlsCacheState::InProgress(notify) => {
                    HlsCacheStartResult::AlreadyInProgress(notify.clone())
                }
                HlsCacheState::Complete(data) => HlsCacheStartResult::AlreadyComplete(data.clone()),
                HlsCacheState::Failed(msg) => HlsCacheStartResult::PreviouslyFailed(msg.clone()),
            };
        }

        // Start new generation
        let notify = Arc::new(Notify::new());
        let entry = CacheEntry {
            state: HlsCacheState::InProgress(notify.clone()),
            created_at: Instant::now(),
            size_bytes: 0,
        };

        guard.insert(key.clone(), entry);
        tracing::debug!("Started generation for {:?}", key);

        HlsCacheStartResult::Started(notify)
    }

    /// Marks content generation as complete and stores the result.
    ///
    /// Notifies any waiters and may trigger eviction if cache is over limit.
    pub fn complete_generation(&self, key: HlsCacheKey, data: Vec<u8>) {
        if self.max_size == 0 {
            return;
        }

        let guard = self.cache.pin();
        let size_bytes = data.len();

        // Get the existing notify to signal completion
        let notify = if let Some(entry) = guard.get(&key) {
            if let HlsCacheState::InProgress(n) = &entry.state {
                Some(n.clone())
            } else {
                None
            }
        } else {
            None
        };

        // Store the completed content
        let data = Arc::new(data);
        let entry = CacheEntry {
            state: HlsCacheState::Complete(data),
            created_at: Instant::now(),
            size_bytes,
        };

        guard.insert(key.clone(), entry);
        let new_size = self.current_size.fetch_add(size_bytes, Ordering::Relaxed) + size_bytes;

        tracing::debug!(
            "Generation complete for {:?} ({} bytes, cache size: {} bytes)",
            key,
            size_bytes,
            new_size
        );

        // Notify waiters
        if let Some(n) = notify {
            n.notify_waiters();
        }

        // Evict if over limit
        if new_size > self.max_size {
            self.evict_oldest(new_size - self.max_size);
        }
    }

    /// Marks content generation as failed with an error message.
    pub fn fail_generation(&self, key: HlsCacheKey, error: &TranscodeError) {
        if self.max_size == 0 {
            return;
        }

        let guard = self.cache.pin();

        // Get the existing notify to signal completion (even on failure)
        let notify = if let Some(entry) = guard.get(&key) {
            if let HlsCacheState::InProgress(n) = &entry.state {
                Some(n.clone())
            } else {
                None
            }
        } else {
            None
        };

        let entry = CacheEntry {
            state: HlsCacheState::Failed(error.to_string()),
            created_at: Instant::now(),
            size_bytes: 0,
        };

        guard.insert(key.clone(), entry);
        tracing::warn!("Generation failed for {:?}: {}", key, error);

        // Notify waiters (so they know to check the state)
        if let Some(n) = notify {
            n.notify_waiters();
        }
    }

    /// Clears a failed entry so it can be retried.
    #[allow(dead_code)]
    pub fn clear_failed(&self, key: &HlsCacheKey) {
        let guard = self.cache.pin();
        if let Some(entry) = guard.get(key)
            && matches!(entry.state, HlsCacheState::Failed(_))
        {
            guard.remove(key);
            tracing::debug!("Cleared failed entry for {:?}", key);
        }
    }

    /// Evicts oldest completed entries until at least `target_bytes` have been freed.
    fn evict_oldest(&self, target_bytes: usize) {
        let guard = self.cache.pin();

        // Collect only Complete entries with their timestamps
        // Prefer evicting segments over playlists (segments are larger)
        let mut entries: Vec<(HlsCacheKey, Instant, usize, bool)> = guard
            .iter()
            .filter_map(|(k, v)| {
                if matches!(v.state, HlsCacheState::Complete(_)) && v.size_bytes > 0 {
                    let is_playlist = matches!(k, HlsCacheKey::Playlist { .. });
                    Some((k.clone(), v.created_at, v.size_bytes, is_playlist))
                } else {
                    None
                }
            })
            .collect();

        // Sort: segments first (is_playlist=false), then by creation time (oldest first)
        entries.sort_by(
            |(_, time_a, _, is_playlist_a), (_, time_b, _, is_playlist_b)| {
                is_playlist_a
                    .cmp(is_playlist_b)
                    .then_with(|| time_a.cmp(time_b))
            },
        );

        let mut freed = 0usize;
        let mut evict_count = 0usize;

        for (key, _, size, _) in entries {
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
            tracing::info!(
                "HLS cache evicted {} entries ({} bytes freed)",
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

/// Result of attempting to start content generation.
pub enum HlsCacheStartResult {
    /// Generation was started - the caller should generate content and call complete/fail
    Started(Arc<Notify>),
    /// Another generation is already in progress - wait on the Notify then re-fetch state
    AlreadyInProgress(Arc<Notify>),
    /// Content already generated - use the cached data
    AlreadyComplete(Arc<Vec<u8>>),
    /// A previous generation failed - caller can retry or serve original
    PreviouslyFailed(String),
    /// Cache is disabled (max_size = 0)
    CacheDisabled,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_playlist_key(path: &str, target: TranscodeTarget) -> HlsCacheKey {
        HlsCacheKey::playlist(PathBuf::from(path), target)
    }

    fn make_segment_key(path: &str, target: TranscodeTarget, index: u32) -> HlsCacheKey {
        HlsCacheKey::segment(PathBuf::from(path), target, index)
    }

    #[test]
    fn test_start_and_complete_playlist() {
        let cache = HlsCache::new(1024 * 1024); // 1MB

        let key = make_playlist_key("/videos/test.mp4", TranscodeTarget::Resolution720p);

        // Start generation
        let result = cache.start_generation(key.clone());
        assert!(matches!(result, HlsCacheStartResult::Started(_)));

        // Should be in progress
        let state = cache.get_state(&key);
        assert!(matches!(state, Some(HlsCacheState::InProgress(_))));

        // Complete it
        let data = b"#EXTM3U\n#EXT-X-VERSION:3\n".to_vec();
        cache.complete_generation(key.clone(), data);

        // Should be complete
        let state = cache.get_state(&key);
        assert!(matches!(state, Some(HlsCacheState::Complete(_))));
    }

    #[test]
    fn test_start_and_complete_segment() {
        let cache = HlsCache::new(1024 * 1024);

        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 5);

        // Start generation
        let result = cache.start_generation(key.clone());
        assert!(matches!(result, HlsCacheStartResult::Started(_)));

        // Complete it
        let data = vec![0u8; 1000]; // Fake MPEG-TS data
        cache.complete_generation(key.clone(), data);

        // Should be complete
        let state = cache.get_state(&key);
        assert!(matches!(state, Some(HlsCacheState::Complete(_))));
    }

    #[test]
    fn test_concurrent_start_returns_in_progress() {
        let cache = HlsCache::new(1024 * 1024);

        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 0);

        // First start
        let result1 = cache.start_generation(key.clone());
        assert!(matches!(result1, HlsCacheStartResult::Started(_)));

        // Second start should return AlreadyInProgress
        let result2 = cache.start_generation(key.clone());
        assert!(matches!(result2, HlsCacheStartResult::AlreadyInProgress(_)));
    }

    #[test]
    fn test_complete_returns_already_complete() {
        let cache = HlsCache::new(1024 * 1024);

        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 0);

        // Start and complete
        cache.start_generation(key.clone());
        cache.complete_generation(key.clone(), vec![0u8; 100]);

        // Another start should return AlreadyComplete
        let result = cache.start_generation(key.clone());
        assert!(matches!(result, HlsCacheStartResult::AlreadyComplete(_)));
    }

    #[test]
    fn test_failed_generation() {
        let cache = HlsCache::new(1024 * 1024);

        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 0);

        cache.start_generation(key.clone());
        cache.fail_generation(
            key.clone(),
            &TranscodeError::TranscodeFailed("Test failure".to_string()),
        );

        // Should be failed
        let state = cache.get_state(&key);
        assert!(matches!(state, Some(HlsCacheState::Failed(_))));

        // Start again should return PreviouslyFailed
        let result = cache.start_generation(key.clone());
        assert!(matches!(result, HlsCacheStartResult::PreviouslyFailed(_)));
    }

    #[test]
    fn test_clear_failed_allows_retry() {
        let cache = HlsCache::new(1024 * 1024);

        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 0);

        cache.start_generation(key.clone());
        cache.fail_generation(
            key.clone(),
            &TranscodeError::TranscodeFailed("Test failure".to_string()),
        );

        // Clear the failure
        cache.clear_failed(&key);

        // Should be able to start again
        let result = cache.start_generation(key.clone());
        assert!(matches!(result, HlsCacheStartResult::Started(_)));
    }

    #[test]
    fn test_disabled_cache() {
        let cache = HlsCache::new(0); // Disabled

        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 0);

        let result = cache.start_generation(key.clone());
        assert!(matches!(result, HlsCacheStartResult::CacheDisabled));

        assert!(cache.get_state(&key).is_none());
    }

    #[test]
    fn test_different_resolutions_are_separate() {
        let cache = HlsCache::new(1024 * 1024);

        let key_720 = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 0);
        let key_480 = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution480p, 0);

        cache.start_generation(key_720.clone());
        cache.complete_generation(key_720.clone(), vec![0u8; 100]);

        // 480p should still be startable
        let result = cache.start_generation(key_480.clone());
        assert!(matches!(result, HlsCacheStartResult::Started(_)));
    }

    #[test]
    fn test_different_segments_are_separate() {
        let cache = HlsCache::new(1024 * 1024);

        let key_0 = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 0);
        let key_1 = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 1);

        cache.start_generation(key_0.clone());
        cache.complete_generation(key_0.clone(), vec![0u8; 100]);

        // Segment 1 should still be startable
        let result = cache.start_generation(key_1.clone());
        assert!(matches!(result, HlsCacheStartResult::Started(_)));
    }

    #[test]
    fn test_playlist_and_segment_are_separate() {
        let cache = HlsCache::new(1024 * 1024);

        let playlist_key = make_playlist_key("/videos/test.mp4", TranscodeTarget::Resolution720p);
        let segment_key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 0);

        cache.start_generation(playlist_key.clone());
        cache.complete_generation(playlist_key.clone(), b"#EXTM3U\n".to_vec());

        // Segment should still be startable
        let result = cache.start_generation(segment_key.clone());
        assert!(matches!(result, HlsCacheStartResult::Started(_)));
    }

    #[test]
    fn test_size_tracking() {
        let cache = HlsCache::new(1024 * 1024);

        assert_eq!(cache.current_size(), 0);

        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 0);
        cache.start_generation(key.clone());
        cache.complete_generation(key.clone(), vec![0u8; 500]);

        assert_eq!(cache.current_size(), 500);
    }

    #[test]
    fn test_eviction_prefers_segments_over_playlists() {
        // Small cache
        let cache = HlsCache::new(500);

        // Add a playlist (small)
        let playlist_key = make_playlist_key("/videos/test.mp4", TranscodeTarget::Resolution720p);
        cache.start_generation(playlist_key.clone());
        cache.complete_generation(playlist_key.clone(), vec![0u8; 50]);

        // Add segments until eviction is triggered
        for i in 0..10 {
            let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, i);
            cache.start_generation(key.clone());
            cache.complete_generation(key, vec![0u8; 100]);
        }

        // Playlist should still exist (segments evicted first)
        assert!(matches!(
            cache.get_state(&playlist_key),
            Some(HlsCacheState::Complete(_))
        ));
    }

    #[test]
    fn test_eviction_on_size_limit() {
        // Small cache that can only hold ~1000 bytes
        let cache = HlsCache::new(1000);

        // Insert several segments to trigger eviction
        for i in 0..10 {
            let key = make_segment_key(
                &format!("/videos/test{}.mp4", i),
                TranscodeTarget::Resolution720p,
                0,
            );
            cache.start_generation(key.clone());
            cache.complete_generation(key, vec![0u8; 200]);
        }

        // Cache should have evicted some entries to stay within bounds
        assert!(cache.current_size() <= 1200); // Allow some slack
    }
}

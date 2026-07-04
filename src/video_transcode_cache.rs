//! HLS transcode cache module for caching playlists and segments.
//!
//! Provides a thread-safe, size-bounded cache for HLS playlists and transcoded
//! video segments to avoid redundant transcoding operations. Handles concurrent
//! requests by ensuring only one transcode runs per segment.

use crate::cache::SizeBoundedMap;
use crate::video_transcode::{TranscodeError, TranscodeTarget};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;
use tokio::sync::Notify;

/// Maximum time a request will wait for an in-progress generation to complete
/// before giving up. Guards against a lost wakeup degrading into a permanent
/// hang: on timeout the caller gets `None` (retryable) rather than blocking
/// forever.
pub const HLS_WAIT_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a `Failed` cache entry is honored before it is treated as expired
/// and a retry is allowed. Prevents a single transient error from poisoning a
/// playlist until the process restarts.
const FAILED_ENTRY_TTL: Duration = Duration::from_secs(60);

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

/// Thread-safe cache for HLS playlists and segments.
///
/// A state machine (in-progress / complete / failed with TTL) layered on the
/// shared [`SizeBoundedMap`] core, which provides lock-free reads, overwrite
/// accounting, and size-based eviction. Only `Complete` entries carry weight.
pub struct HlsCache {
    /// The shared size-bounded concurrent cache core
    cache: SizeBoundedMap<HlsCacheKey, HlsCacheState>,
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
            cache: SizeBoundedMap::new(max_size_bytes),
        }
    }

    /// Gets the current state of a cache entry.
    ///
    /// Returns `None` if no entry exists for this key.
    pub fn get_state(&self, key: &HlsCacheKey) -> Option<HlsCacheState> {
        self.cache.with_entry(key, |entry| entry.value.clone())
    }

    /// Marks content generation as in-progress and returns the Notify to signal on completion.
    ///
    /// If generation is already in progress or complete, returns the existing state.
    /// This ensures only one generation runs per key.
    pub fn start_generation(&self, key: HlsCacheKey) -> HlsCacheStartResult {
        if self.cache.is_disabled() {
            return HlsCacheStartResult::CacheDisabled;
        }

        // Check if already exists
        let existing = self
            .cache
            .with_entry(&key, |entry| match &entry.value {
                HlsCacheState::InProgress(notify) => {
                    Some(HlsCacheStartResult::AlreadyInProgress(notify.clone()))
                }
                HlsCacheState::Complete(data) => {
                    Some(HlsCacheStartResult::AlreadyComplete(data.clone()))
                }
                HlsCacheState::Failed(msg) => {
                    // Only honor the failure while it is still fresh. Once the
                    // TTL elapses, fall through to start a new generation so a
                    // transient error can be retried.
                    if entry.inserted_at.elapsed() < FAILED_ENTRY_TTL {
                        Some(HlsCacheStartResult::PreviouslyFailed(msg.clone()))
                    } else {
                        tracing::debug!("Failed entry for {:?} expired; allowing retry", key);
                        None
                    }
                }
            })
            .flatten();
        if let Some(result) = existing {
            return result;
        }

        // Start new generation (weightless: only Complete entries are counted)
        let notify = Arc::new(Notify::new());
        self.cache
            .insert_weighted(key.clone(), HlsCacheState::InProgress(notify.clone()), 0);
        tracing::debug!("Started generation for {:?}", key);

        HlsCacheStartResult::Started(notify)
    }

    /// Marks content generation as complete and stores the result.
    ///
    /// Notifies any waiters and may trigger eviction if cache is over limit.
    pub fn complete_generation(&self, key: HlsCacheKey, data: Vec<u8>) {
        if self.cache.is_disabled() {
            return;
        }

        let size_bytes = data.len();

        // Store the completed content. The core subtracts any replaced
        // entry's accounted size so `current_size` does not ratchet up on
        // rewrite, and hands back the replaced state so waiters registered on
        // an in-progress generation can be notified.
        let (replaced, new_size) = self.cache.insert_weighted(
            key.clone(),
            HlsCacheState::Complete(Arc::new(data)),
            size_bytes,
        );

        tracing::debug!(
            "Generation complete for {:?} ({} bytes, cache size: {} bytes)",
            key,
            size_bytes,
            new_size
        );

        // Notify waiters
        if let Some(HlsCacheState::InProgress(n)) = replaced {
            n.notify_waiters();
        }

        // Evict if over limit
        if new_size > self.cache.max_size() {
            self.evict_oldest(new_size - self.cache.max_size());
        }
    }

    /// Marks content generation as failed with an error message.
    pub fn fail_generation(&self, key: HlsCacheKey, error: &TranscodeError) {
        if self.cache.is_disabled() {
            return;
        }

        let (replaced, _) =
            self.cache
                .insert_weighted(key.clone(), HlsCacheState::Failed(error.to_string()), 0);
        tracing::warn!("Generation failed for {:?}: {}", key, error);

        // Notify waiters (so they know to check the state)
        if let Some(HlsCacheState::InProgress(n)) = replaced {
            n.notify_waiters();
        }
    }

    /// Waits for an in-progress generation (identified by `notify`) to complete,
    /// returning the completed data if it becomes available within `timeout`.
    ///
    /// This uses the race-free tokio `Notify` pattern: interest is registered
    /// (`enable`) *before* re-checking cache state, so a completion that lands
    /// between `start_generation` and this call is never missed. A bounded
    /// timeout guards against a genuinely lost signal, degrading a hang into a
    /// retryable `None`.
    pub async fn wait_for_completion(
        &self,
        key: &HlsCacheKey,
        notify: Arc<Notify>,
        timeout: Duration,
    ) -> Option<Arc<Vec<u8>>> {
        // Register interest before re-checking so no wakeup can be lost.
        let notified = notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        // The generation may have finished before we registered above.
        if let Some(HlsCacheState::Complete(data)) = self.get_state(key) {
            return Some(data);
        }

        match tokio::time::timeout(timeout, notified).await {
            Ok(()) => match self.get_state(key) {
                Some(HlsCacheState::Complete(data)) => Some(data),
                _ => None,
            },
            Err(_) => {
                tracing::warn!("Timed out waiting for in-progress generation of {:?}", key);
                None
            }
        }
    }

    /// Evicts oldest completed entries until at least `target_bytes` have been freed.
    ///
    /// Only `Complete` entries are evictable; segments are preferred over
    /// playlists (segments are larger), oldest first within each group.
    fn evict_oldest(&self, target_bytes: usize) {
        let stats = self.cache.evict_until_freed(target_bytes, |key, entry| {
            if matches!(entry.value, HlsCacheState::Complete(_)) && entry.size_bytes > 0 {
                // Sort key: segments first (is_playlist=false), then oldest first
                let is_playlist = matches!(key, HlsCacheKey::Playlist { .. });
                Some((is_playlist, entry.inserted_at))
            } else {
                None
            }
        });

        if stats.evicted > 0 {
            tracing::info!(
                "HLS cache evicted {} entries ({} bytes freed)",
                stats.evicted,
                stats.freed
            );
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

    /// Inserts a `Failed` entry with an explicit `created_at` timestamp.
    ///
    /// Test-only helper used to exercise TTL expiry without waiting in real
    /// time (std `Instant` cannot be paused like tokio's clock).
    #[cfg(test)]
    pub fn insert_failed_for_test(&self, key: HlsCacheKey, message: &str, created_at: Instant) {
        self.cache.insert_weighted_at(
            key,
            HlsCacheState::Failed(message.to_string()),
            0,
            created_at,
        );
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
    fn test_failed_entry_within_ttl_blocks_retry() {
        let cache = HlsCache::new(1024 * 1024);

        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 0);

        cache.start_generation(key.clone());
        cache.fail_generation(
            key.clone(),
            &TranscodeError::TranscodeFailed("Test failure".to_string()),
        );

        // A fresh failure should still be honored (not retried).
        let result = cache.start_generation(key.clone());
        assert!(matches!(result, HlsCacheStartResult::PreviouslyFailed(_)));
    }

    #[test]
    fn test_failed_entry_expires_and_allows_retry() {
        let cache = HlsCache::new(1024 * 1024);

        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 0);

        // Simulate a failure that happened longer ago than the TTL.
        let stale = Instant::now()
            .checked_sub(FAILED_ENTRY_TTL + Duration::from_secs(1))
            .expect("clock far enough from epoch");
        cache.insert_failed_for_test(key.clone(), "stale failure", stale);

        // Sanity: it is stored as Failed.
        assert!(matches!(
            cache.get_state(&key),
            Some(HlsCacheState::Failed(_))
        ));

        // Expired failure should allow a fresh generation.
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

    #[test]
    fn test_size_accounting_on_overwrite() {
        let cache = HlsCache::new(1024 * 1024);

        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 0);

        cache.start_generation(key.clone());
        cache.complete_generation(key.clone(), vec![0u8; 500]);
        assert_eq!(cache.current_size(), 500);

        // Overwriting the same key must not ratchet the size up; it should
        // reflect only the latest entry, not 500 + 200.
        cache.complete_generation(key.clone(), vec![0u8; 200]);
        assert_eq!(cache.current_size(), 200);

        // Overwriting with a larger payload updates accounting upward correctly.
        cache.complete_generation(key.clone(), vec![0u8; 900]);
        assert_eq!(cache.current_size(), 900);
    }

    #[tokio::test]
    async fn test_wait_for_completion_already_done_no_lost_wakeup() {
        // Reproduces the lost-wakeup race: the generation completes (and the
        // notify fires with no registered waiters) *before* the waiter calls
        // wait_for_completion. The state re-check must return the data rather
        // than blocking forever.
        let cache = HlsCache::new(1024 * 1024);

        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 0);

        let notify = match cache.start_generation(key.clone()) {
            HlsCacheStartResult::Started(n) => n,
            _ => panic!("expected Started"),
        };

        // Complete BEFORE anyone waits — notify_waiters here reaches no one.
        cache.complete_generation(key.clone(), vec![7u8; 128]);

        // Even so, waiting must resolve immediately from the re-checked state.
        let data = cache
            .wait_for_completion(&key, notify, Duration::from_secs(5))
            .await;
        assert_eq!(data.map(|d| d.len()), Some(128));
    }

    #[tokio::test]
    async fn test_wait_for_completion_concurrent_waiters() {
        use std::sync::Arc as StdArc;

        let cache = StdArc::new(HlsCache::new(1024 * 1024));

        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 0);

        // The generating task holds the primary notify.
        let gen_notify = match cache.start_generation(key.clone()) {
            HlsCacheStartResult::Started(n) => n,
            _ => panic!("expected Started"),
        };

        // Spawn several concurrent waiters that arrive while in progress.
        let waiters: Vec<_> = (0..8)
            .map(|_| {
                let cache = cache.clone();
                let key = key.clone();
                let notify = match cache.start_generation(key.clone()) {
                    HlsCacheStartResult::AlreadyInProgress(n) => n,
                    _ => panic!("expected AlreadyInProgress"),
                };
                tokio::spawn(async move {
                    cache
                        .wait_for_completion(&key, notify, Duration::from_secs(5))
                        .await
                })
            })
            .collect();

        // Let the waiters register, then complete the generation.
        tokio::time::sleep(Duration::from_millis(20)).await;
        cache.complete_generation(key.clone(), vec![3u8; 256]);
        gen_notify.notify_waiters();

        for w in waiters {
            let data = w.await.expect("waiter task panicked");
            assert_eq!(data.map(|d| d.len()), Some(256));
        }
    }

    #[tokio::test]
    async fn test_wait_for_completion_times_out() {
        let cache = HlsCache::new(1024 * 1024);

        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 0);

        let notify = match cache.start_generation(key.clone()) {
            HlsCacheStartResult::Started(n) => n,
            _ => panic!("expected Started"),
        };

        // Never complete — the bounded wait must degrade to None, not hang.
        let data = cache
            .wait_for_completion(&key, notify, Duration::from_millis(50))
            .await;
        assert!(data.is_none());
    }
}

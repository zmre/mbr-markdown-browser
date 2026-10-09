//! HLS transcode cache module for caching playlists and segments.
//!
//! Provides a thread-safe, size-bounded cache for HLS playlists and transcoded
//! video segments to avoid redundant transcoding operations. Handles concurrent
//! requests by ensuring only one transcode runs per segment.
//!
//! ## Cancellation safety
//!
//! An in-progress marker that is never settled would wedge its key forever:
//! every later request for it would wait out [`HLS_WAIT_TIMEOUT`] and fail, with
//! no work running. That is not hypothetical — Safari's HLS loader routinely
//! abandons segment prefetches, and axum drops the request future when a client
//! disconnects, so any completion bookkeeping living in the request path gets
//! skipped.
//!
//! [`HlsCache::spawn_generation`] is therefore the only supported way to produce
//! content: it runs the work in a detached task so a disconnect cannot stop it,
//! and holds an [`InFlightGuard`] whose `Drop` withdraws the marker if the result
//! was never recorded. No client behaviour can leave a key in-progress.

use crate::cache::{Claim, SizeBoundedMap};
use crate::video_remux::RemuxPart;
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
    /// Key for one part of the stream-copy (remux) fMP4 variant.
    ///
    /// Unlike the transcode variants this is keyed by a *mtime-scoped* string
    /// (see [`crate::video_metadata_cache::cache_key_with_mtime`]) rather than
    /// the bare path, so editing the source file yields fresh keys and the
    /// playlist, init segment and media segments are re-derived together. That
    /// matters more here than for the transcode ladder because a remux
    /// segment's byte layout depends on the source's keyframe positions.
    Remux { source_key: String, part: RemuxPart },
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

    /// Creates a cache key for one part of the remux variant.
    pub fn remux(source_key: String, part: RemuxPart) -> Self {
        Self::Remux { source_key, part }
    }

    /// Whether this entry is a small, frequently needed index rather than bulk
    /// media, and so should be evicted last.
    ///
    /// Playlists and the fMP4 init segment are both tiny and required for every
    /// playback attempt; media segments are large and cheap to regenerate.
    fn is_index_entry(&self) -> bool {
        match self {
            Self::Playlist { .. } => true,
            Self::Segment { .. } => false,
            Self::Remux { part, .. } => {
                matches!(part, RemuxPart::Playlist | RemuxPart::Init)
            }
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
    ///
    /// The slot is claimed with a single compare-and-swap
    /// ([`SizeBoundedMap::claim_or`]). A `get` followed by an `insert` would
    /// not hold: papaya's pin guard is an epoch guard, not a lock, so racing
    /// callers could all observe a vacant slot, all be told to `Started`, and
    /// all transcode the same segment — with every loser's `Notify` orphaned
    /// when the winner later overwrites the entry.
    pub fn start_generation(&self, key: HlsCacheKey) -> HlsCacheStartResult {
        if self.cache.is_disabled() {
            return HlsCacheStartResult::CacheDisabled;
        }

        // Weightless: only Complete entries are counted against the budget.
        let notify = Arc::new(Notify::new());
        let claim = self.cache.claim_or(
            key.clone(),
            HlsCacheState::InProgress(notify.clone()),
            0,
            |entry| match &entry.value {
                HlsCacheState::InProgress(existing) => {
                    Some(HlsCacheStartResult::AlreadyInProgress(existing.clone()))
                }
                HlsCacheState::Complete(data) => {
                    Some(HlsCacheStartResult::AlreadyComplete(data.clone()))
                }
                // Only honor a failure while it is still fresh. Once the TTL
                // elapses the entry is replaced so a transient error can be
                // retried — by exactly one caller.
                HlsCacheState::Failed(msg) if entry.inserted_at.elapsed() < FAILED_ENTRY_TTL => {
                    Some(HlsCacheStartResult::PreviouslyFailed(msg.clone()))
                }
                HlsCacheState::Failed(_) => None,
            },
        );

        match claim {
            Claim::Retained(existing) => existing,
            Claim::Claimed => {
                tracing::debug!("Started generation for {:?}", key);
                HlsCacheStartResult::Started(notify)
            }
        }
    }

    /// Marks content generation as complete and stores the result.
    ///
    /// Notifies any waiters and may trigger eviction if cache is over limit.
    ///
    /// Returns the stored payload so the producer can serve it directly. Reading
    /// it back out of the cache instead would be a race: a large entry can be
    /// evicted by the very insertion that added it, and the caller would then
    /// have to 404 a segment it had just built successfully.
    pub fn complete_generation(&self, key: HlsCacheKey, data: Vec<u8>) -> Arc<Vec<u8>> {
        let size_bytes = data.len();
        let stored = Arc::new(data);
        if self.cache.is_disabled() {
            return stored;
        }

        // Store the completed content. The core subtracts any replaced
        // entry's accounted size so `current_size` does not ratchet up on
        // rewrite, and hands back the replaced state so waiters registered on
        // an in-progress generation can be notified.
        let (replaced, new_size) = self.cache.insert_weighted(
            key.clone(),
            HlsCacheState::Complete(stored.clone()),
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

        stored
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

    /// Withdraws an in-progress marker that will never be settled.
    ///
    /// Only removes the entry while it is still `InProgress`, so a generation
    /// that completed concurrently is never clobbered. Returns whether a marker
    /// was withdrawn.
    ///
    /// Leaving the marker in place would be the worst possible failure mode: the
    /// key looks busy forever, so every later request waits out
    /// [`HLS_WAIT_TIMEOUT`] while no work is running. Removing it instead lets
    /// the next request start cleanly.
    fn abandon_generation(&self, key: &HlsCacheKey) -> bool {
        let withdrawn = self.cache.remove_if(key.clone(), |entry| {
            matches!(entry.value, HlsCacheState::InProgress(_))
        });
        if withdrawn {
            tracing::debug!("Withdrew unsettled in-progress marker for {:?}", key);
        }
        withdrawn
    }

    /// Runs `generate` for a key already claimed via [`Self::start_generation`],
    /// settling the cache entry no matter what happens to the caller.
    ///
    /// The work runs in a **detached** task. That is the fix for a real and
    /// frequent failure: when a client disconnects mid-request (Safari's HLS
    /// loader abandons segment prefetches constantly), axum drops the request
    /// future. Bookkeeping awaited in that future would simply never run, and the
    /// key would stay in-progress forever. Dropping the returned `JoinHandle`
    /// does not abort a tokio task, so the generation still finishes and still
    /// populates the cache — the abandoned work is not even wasted, since the
    /// client's inevitable retry is then served from cache.
    ///
    /// An [`InFlightGuard`] covers the remaining gap: if the task itself is
    /// dropped (runtime shutdown) or panics before recording a result, the marker
    /// is withdrawn and waiters are woken on unwind.
    ///
    /// # Panics
    ///
    /// Must be called from within a tokio runtime.
    pub fn spawn_generation<F>(
        cache: &Arc<Self>,
        key: HlsCacheKey,
        notify: Arc<Notify>,
        generate: F,
    ) -> tokio::task::JoinHandle<Result<Arc<Vec<u8>>, TranscodeError>>
    where
        F: FnOnce() -> Result<Vec<u8>, TranscodeError> + Send + 'static,
    {
        let cache = Arc::clone(cache);
        tokio::spawn(async move {
            let mut guard = InFlightGuard {
                cache: Arc::clone(&cache),
                key: key.clone(),
                notify: Arc::clone(&notify),
                settled: false,
            };

            // ffmpeg demuxing and muxing is blocking CPU/IO work and must not
            // occupy a tokio worker thread.
            let outcome = tokio::task::spawn_blocking(generate).await;

            let result = match outcome {
                Ok(Ok(data)) => Ok(cache.complete_generation(key.clone(), data)),
                Ok(Err(error)) => {
                    cache.fail_generation(key.clone(), &error);
                    Err(error)
                }
                Err(join_error) => {
                    // The blocking closure panicked. Record it as a failure so
                    // waiters get an answer instead of timing out.
                    let error = TranscodeError::TranscodeFailed(format!(
                        "generation task did not finish: {join_error}"
                    ));
                    cache.fail_generation(key.clone(), &error);
                    Err(error)
                }
            };

            // A result is recorded, so the guard has nothing left to clean up.
            guard.settled = true;
            // `complete_generation`/`fail_generation` already wake waiters via
            // the state they replaced; this also covers the case where the
            // in-progress entry was evicted or overwritten in the meantime, so a
            // waiter can never be left asleep with no one to wake it.
            notify.notify_waiters();

            result
        })
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
    /// Only `Complete` entries are evictable; media segments are preferred over
    /// playlists and init segments (segments are larger), oldest first within
    /// each group.
    fn evict_oldest(&self, target_bytes: usize) {
        let stats = self.cache.evict_until_freed(target_bytes, |key, entry| {
            if matches!(entry.value, HlsCacheState::Complete(_)) && entry.size_bytes > 0 {
                // Sort key: bulk media first (is_index_entry=false), then oldest first
                Some((key.is_index_entry(), entry.inserted_at))
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

/// Withdraws an unsettled in-progress marker when dropped.
///
/// The invariant it enforces: a claimed key is always either settled with a
/// result or released. Without that, an interrupted generation leaves the key
/// looking permanently busy, and every later request for it waits out
/// [`HLS_WAIT_TIMEOUT`] with no work running — a silent, permanent stall rather
/// than an error anyone can see.
///
/// `Notify::notify_waiters` is synchronous, so waking waiters from `Drop` is
/// sound; nothing here awaits.
struct InFlightGuard {
    cache: Arc<HlsCache>,
    key: HlsCacheKey,
    notify: Arc<Notify>,
    /// Set once a result has been recorded, which makes the drop a no-op.
    settled: bool,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        tracing::warn!(
            "Generation of {:?} ended without a result; releasing the claim",
            self.key
        );
        self.cache.abandon_generation(&self.key);
        // Wake anyone already waiting so they retry immediately instead of
        // blocking on a claim that no longer exists.
        self.notify.notify_waiters();
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
    fn test_concurrent_start_generation_admits_one_producer() {
        // Regression: `start_generation` used to `get` then `insert`, two
        // independent lock-free operations. Racing callers could all observe a
        // vacant slot and all become producers, so the same segment was
        // transcoded several times and every loser's `Notify` was orphaned.
        use std::sync::Barrier;
        use std::sync::atomic::{AtomicUsize, Ordering};

        const THREADS: usize = 8;
        const ROUNDS: usize = 64;

        for round in 0..ROUNDS {
            let cache = HlsCache::new(1024 * 1024);
            let key = make_segment_key(
                &format!("/videos/race{round}.mp4"),
                TranscodeTarget::Resolution720p,
                0,
            );
            let barrier = Barrier::new(THREADS);
            let started = AtomicUsize::new(0);

            std::thread::scope(|scope| {
                for _ in 0..THREADS {
                    scope.spawn(|| {
                        barrier.wait();
                        if matches!(
                            cache.start_generation(key.clone()),
                            HlsCacheStartResult::Started(_)
                        ) {
                            started.fetch_add(1, Ordering::Relaxed);
                        }
                    });
                }
            });

            assert_eq!(
                started.load(Ordering::Relaxed),
                1,
                "exactly one caller may be told to generate a given key"
            );
        }
    }

    #[test]
    fn test_concurrent_start_generation_after_expired_failure_admits_one_producer() {
        // The expired-`Failed` path also replaces the entry, so it needs the
        // same atomicity: only one retry may become the producer.
        use std::sync::Barrier;
        use std::sync::atomic::{AtomicUsize, Ordering};

        const THREADS: usize = 8;
        const ROUNDS: usize = 64;

        for round in 0..ROUNDS {
            let cache = HlsCache::new(1024 * 1024);
            let key = make_segment_key(
                &format!("/videos/retry{round}.mp4"),
                TranscodeTarget::Resolution720p,
                0,
            );
            let stale = Instant::now()
                .checked_sub(FAILED_ENTRY_TTL + Duration::from_secs(1))
                .expect("clock far enough from epoch");
            cache.insert_failed_for_test(key.clone(), "stale failure", stale);

            let barrier = Barrier::new(THREADS);
            let started = AtomicUsize::new(0);

            std::thread::scope(|scope| {
                for _ in 0..THREADS {
                    scope.spawn(|| {
                        barrier.wait();
                        if matches!(
                            cache.start_generation(key.clone()),
                            HlsCacheStartResult::Started(_)
                        ) {
                            started.fetch_add(1, Ordering::Relaxed);
                        }
                    });
                }
            });

            assert_eq!(
                started.load(Ordering::Relaxed),
                1,
                "an expired failure may be retried by exactly one caller"
            );
        }
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

    /// The producer must always get its payload back, even when the entry it
    /// just inserted is immediately evicted for being over the size limit.
    /// Otherwise a large segment could 404 right after generating successfully.
    #[test]
    fn test_complete_generation_returns_payload_even_when_evicted() {
        // Cache far smaller than the payload, so the insert immediately evicts.
        let cache = HlsCache::new(64);

        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 0);
        cache.start_generation(key.clone());
        let stored = cache.complete_generation(key.clone(), vec![9u8; 4096]);

        assert_eq!(stored.len(), 4096);
        assert!(stored.iter().all(|byte| *byte == 9));
        // It really was evicted, so reading the cache back would have failed.
        assert!(!matches!(
            cache.get_state(&key),
            Some(HlsCacheState::Complete(_))
        ));
    }

    /// A disabled cache stores nothing but must still hand the payload back.
    #[test]
    fn test_complete_generation_returns_payload_when_cache_disabled() {
        let cache = HlsCache::new(0);
        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 0);

        let stored = cache.complete_generation(key.clone(), vec![1u8; 32]);
        assert_eq!(stored.len(), 32);
        assert!(cache.get_state(&key).is_none());
    }

    /// Remux parts are keyed independently of the transcode ladder and of each
    /// other, and a new mtime yields new keys so an edited file re-segments.
    #[test]
    fn test_remux_keys_are_distinct() {
        use crate::video_remux::RemuxPart;

        let cache = HlsCache::new(1024 * 1024);
        let playlist = HlsCacheKey::remux("clip::mtime=1".to_string(), RemuxPart::Playlist);
        let init = HlsCacheKey::remux("clip::mtime=1".to_string(), RemuxPart::Init);
        let segment = HlsCacheKey::remux("clip::mtime=1".to_string(), RemuxPart::Segment(0));
        let edited = HlsCacheKey::remux("clip::mtime=2".to_string(), RemuxPart::Segment(0));

        cache.start_generation(playlist.clone());
        cache.complete_generation(playlist, b"#EXTM3U\n".to_vec());

        for key in [init, segment, edited] {
            assert!(
                matches!(
                    cache.start_generation(key.clone()),
                    HlsCacheStartResult::Started(_)
                ),
                "{key:?} must not collide with the playlist entry"
            );
        }
    }

    /// Media segments are evicted before playlists and init segments, which are
    /// tiny and needed for every playback attempt.
    #[test]
    fn test_eviction_prefers_remux_segments_over_playlist_and_init() {
        use crate::video_remux::RemuxPart;

        let cache = HlsCache::new(500);

        let playlist = HlsCacheKey::remux("clip::mtime=1".to_string(), RemuxPart::Playlist);
        let init = HlsCacheKey::remux("clip::mtime=1".to_string(), RemuxPart::Init);
        cache.start_generation(playlist.clone());
        cache.complete_generation(playlist.clone(), vec![0u8; 40]);
        cache.start_generation(init.clone());
        cache.complete_generation(init.clone(), vec![0u8; 40]);

        for index in 0..10 {
            let key = HlsCacheKey::remux("clip::mtime=1".to_string(), RemuxPart::Segment(index));
            cache.start_generation(key.clone());
            cache.complete_generation(key, vec![0u8; 100]);
        }

        for key in [playlist, init] {
            assert!(
                matches!(cache.get_state(&key), Some(HlsCacheState::Complete(_))),
                "{key:?} must survive eviction of media segments"
            );
        }
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

    // -----------------------------------------------------------------
    // Cancellation safety
    // -----------------------------------------------------------------
    //
    // The invariant: no caller behaviour may leave a key in the in-progress
    // state. A leaked marker is the worst failure mode available — the key looks
    // busy forever, so every later request waits out `HLS_WAIT_TIMEOUT` while no
    // work is running, and the client sees a stall with no error to report.

    /// Dropping the handle returned by `spawn_generation` is exactly what happens
    /// when a client disconnects mid-request (axum drops the request future). The
    /// work must still finish and still populate the cache.
    #[tokio::test]
    async fn test_dropped_handle_still_completes_generation() {
        let cache = Arc::new(HlsCache::new(1024 * 1024));
        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 5);

        let notify = match cache.start_generation(key.clone()) {
            HlsCacheStartResult::Started(n) => n,
            _ => panic!("expected Started"),
        };

        // Abandon the handle immediately, before the work can finish.
        drop(HlsCache::spawn_generation(
            &cache,
            key.clone(),
            notify,
            || {
                std::thread::sleep(Duration::from_millis(50));
                Ok(vec![1u8; 64])
            },
        ));

        // A later request must be able to obtain the content promptly rather than
        // block on a marker nobody will ever settle.
        let notify = match cache.start_generation(key.clone()) {
            HlsCacheStartResult::AlreadyInProgress(n) => n,
            HlsCacheStartResult::AlreadyComplete(data) => {
                assert_eq!(data.len(), 64);
                return;
            }
            other => panic!(
                "expected AlreadyInProgress or AlreadyComplete, got {}",
                describe(&other)
            ),
        };
        let data = cache
            .wait_for_completion(&key, notify, Duration::from_secs(5))
            .await;
        assert_eq!(
            data.map(|d| d.len()),
            Some(64),
            "an abandoned request's work must still land in the cache"
        );
    }

    /// The same, but for a generation that fails: the key must end up `Failed`,
    /// never stuck in-progress.
    #[tokio::test]
    async fn test_dropped_handle_still_records_failure() {
        let cache = Arc::new(HlsCache::new(1024 * 1024));
        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 1);

        let notify = match cache.start_generation(key.clone()) {
            HlsCacheStartResult::Started(n) => n,
            _ => panic!("expected Started"),
        };
        drop(HlsCache::spawn_generation(
            &cache,
            key.clone(),
            notify,
            || {
                std::thread::sleep(Duration::from_millis(20));
                Err(TranscodeError::TranscodeFailed("nope".to_string()))
            },
        ));

        // Give the detached task time to settle the entry.
        for _ in 0..100 {
            if matches!(cache.get_state(&key), Some(HlsCacheState::Failed(_))) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "failure was never recorded; state is {}",
            cache
                .get_state(&key)
                .map_or("absent", |s| describe_state(&s))
        );
    }

    /// A generation whose closure panics must still settle the key, so waiters get
    /// an answer instead of timing out.
    #[tokio::test]
    async fn test_panicking_generation_settles_the_key() {
        let cache = Arc::new(HlsCache::new(1024 * 1024));
        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 2);

        let notify = match cache.start_generation(key.clone()) {
            HlsCacheStartResult::Started(n) => n,
            _ => panic!("expected Started"),
        };
        let handle = HlsCache::spawn_generation(&cache, key.clone(), notify, || {
            panic!("generation exploded");
        });

        let result = handle
            .await
            .expect("outer task must not propagate the panic");
        assert!(
            result.is_err(),
            "a panicking generation must report an error"
        );
        assert!(
            matches!(cache.get_state(&key), Some(HlsCacheState::Failed(_))),
            "the key must be settled, not left in-progress"
        );
    }

    /// The guard's own contract, exercised directly: an unsettled claim is
    /// withdrawn on drop so the next `start_generation` gets a clean `Started`.
    #[tokio::test]
    async fn test_unsettled_guard_releases_the_claim() {
        let cache = Arc::new(HlsCache::new(1024 * 1024));
        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 3);

        let notify = match cache.start_generation(key.clone()) {
            HlsCacheStartResult::Started(n) => n,
            _ => panic!("expected Started"),
        };
        assert!(matches!(
            cache.get_state(&key),
            Some(HlsCacheState::InProgress(_))
        ));

        drop(InFlightGuard {
            cache: Arc::clone(&cache),
            key: key.clone(),
            notify,
            settled: false,
        });

        assert!(
            cache.get_state(&key).is_none(),
            "an unsettled claim must be withdrawn"
        );
        assert!(
            matches!(
                cache.start_generation(key.clone()),
                HlsCacheStartResult::Started(_)
            ),
            "the next request must be able to claim the key"
        );
    }

    /// A settled guard must not touch the completed entry on drop.
    #[tokio::test]
    async fn test_settled_guard_leaves_content_alone() {
        let cache = Arc::new(HlsCache::new(1024 * 1024));
        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 4);

        let notify = match cache.start_generation(key.clone()) {
            HlsCacheStartResult::Started(n) => n,
            _ => panic!("expected Started"),
        };
        cache.complete_generation(key.clone(), vec![7u8; 16]);

        drop(InFlightGuard {
            cache: Arc::clone(&cache),
            key: key.clone(),
            notify,
            settled: true,
        });

        assert!(matches!(
            cache.get_state(&key),
            Some(HlsCacheState::Complete(_))
        ));
    }

    /// Withdrawing a claim must never clobber content another writer already
    /// stored for the same key.
    #[test]
    fn test_abandon_only_removes_an_in_progress_marker() {
        let cache = HlsCache::new(1024 * 1024);
        let key = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 0);

        cache.start_generation(key.clone());
        cache.complete_generation(key.clone(), vec![1u8; 32]);

        assert!(
            !cache.abandon_generation(&key),
            "a completed entry must not be withdrawn"
        );
        assert!(matches!(
            cache.get_state(&key),
            Some(HlsCacheState::Complete(_))
        ));

        // And an in-progress marker *is* withdrawn.
        let other = make_segment_key("/videos/test.mp4", TranscodeTarget::Resolution720p, 9);
        cache.start_generation(other.clone());
        assert!(cache.abandon_generation(&other));
        assert!(cache.get_state(&other).is_none());
    }

    fn describe(result: &HlsCacheStartResult) -> &'static str {
        match result {
            HlsCacheStartResult::Started(_) => "Started",
            HlsCacheStartResult::AlreadyInProgress(_) => "AlreadyInProgress",
            HlsCacheStartResult::AlreadyComplete(_) => "AlreadyComplete",
            HlsCacheStartResult::PreviouslyFailed(_) => "PreviouslyFailed",
            HlsCacheStartResult::CacheDisabled => "CacheDisabled",
        }
    }

    fn describe_state(state: &HlsCacheState) -> &'static str {
        match state {
            HlsCacheState::InProgress(_) => "InProgress",
            HlsCacheState::Complete(_) => "Complete",
            HlsCacheState::Failed(_) => "Failed",
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

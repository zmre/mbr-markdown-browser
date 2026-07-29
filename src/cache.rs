//! Shared primitives for size-bounded in-memory caches.
//!
//! The concrete caches in this crate ([`crate::oembed_cache`],
//! `video_metadata_cache`, `video_transcode_cache`) all bound memory by an
//! approximate byte budget with the same conventions:
//!
//! - A `max_size_bytes` of `0` disables the cache entirely.
//! - Each stored entry is weighed once at insert time (value estimate + key
//!   bytes + fixed per-entry overhead).
//! - Overwriting an existing key subtracts the replaced entry's accounted
//!   size first, so the running total reflects the replacement rather than
//!   ratcheting upward.
//! - When the total exceeds the budget, entries are evicted in a
//!   caller-defined priority order until enough bytes are freed.
//!
//! [`SizeBoundedMap`] implements those conventions on top of a lock-free
//! papaya [`HashMap`] with an [`AtomicUsize`] size counter, and is the shared
//! core for the papaya-backed caches. [`Entry`] carries the per-entry
//! bookkeeping and is also reused by the mutex-guarded LRU oembed cache,
//! which shares the weighing/accounting conventions but not the storage.

use papaya::{Compute, HashMap, Operation};
use std::borrow::Borrow;
use std::hash::Hash;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// A cached value together with the bookkeeping used for size accounting and
/// insertion-order eviction.
pub struct Entry<V> {
    /// The cached value.
    pub value: V,
    /// When this entry was inserted (used for oldest-first eviction and TTLs).
    pub inserted_at: Instant,
    /// Accounted memory size in bytes.
    pub size_bytes: usize,
}

impl<V> Entry<V> {
    /// Creates an entry inserted now.
    pub fn new(value: V, size_bytes: usize) -> Self {
        Self {
            value,
            inserted_at: Instant::now(),
            size_bytes,
        }
    }

    /// Total accounted size for an entry: the estimated value size plus the
    /// key's bytes plus the fixed per-entry bookkeeping overhead.
    pub fn weigh(value_size: usize, key_len: usize) -> usize {
        value_size + key_len + std::mem::size_of::<Self>()
    }
}

/// Outcome of [`SizeBoundedMap::claim_or`].
pub enum Claim<R> {
    /// The caller's value now occupies the slot.
    Claimed,
    /// An existing entry was kept; carries the value the decision closure
    /// produced for it.
    Retained(R),
}

/// Statistics from an eviction pass.
pub struct EvictionStats {
    /// Number of entries removed.
    pub evicted: usize,
    /// Total accounted bytes freed.
    pub freed: usize,
}

/// A lock-free, size-bounded concurrent map.
///
/// Reads and writes go through a papaya [`HashMap`]; the approximate total
/// size is tracked in an [`AtomicUsize`]. Eviction is approximate and happens
/// at insert time under the caller's control (see [`Self::evict_until_freed`]),
/// which keeps `get` entirely lock-free.
pub struct SizeBoundedMap<K, V> {
    /// The underlying concurrent map.
    map: HashMap<K, Entry<V>>,
    /// Current total accounted size in bytes (approximate).
    current_size: AtomicUsize,
    /// Maximum allowed size in bytes; `0` disables the cache.
    max_size: usize,
}

impl<K, V> SizeBoundedMap<K, V>
where
    K: Hash + Eq + Clone,
{
    /// Creates a new map with the specified maximum size in bytes.
    ///
    /// A `max_size_bytes` of `0` disables the cache: lookups always miss and
    /// inserts are no-ops.
    pub fn new(max_size_bytes: usize) -> Self {
        Self {
            map: HashMap::new(),
            current_size: AtomicUsize::new(0),
            max_size: max_size_bytes,
        }
    }

    /// Returns true when the cache is disabled (`max_size_bytes == 0`).
    pub fn is_disabled(&self) -> bool {
        self.max_size == 0
    }

    /// Returns the configured maximum size in bytes.
    pub fn max_size(&self) -> usize {
        self.max_size
    }

    /// Returns the current approximate accounted size in bytes.
    pub fn current_size(&self) -> usize {
        self.current_size.load(Ordering::Relaxed)
    }

    /// Subtracts `bytes` from the running total, saturating at zero.
    ///
    /// The total is maintained by several independent lock-free operations, so
    /// a duplicated or stale subtraction is possible under contention. A plain
    /// `fetch_sub` would wrap to a near-`usize::MAX` total and leave the cache
    /// permanently "over budget", evicting on every insert; saturating keeps
    /// the drift bounded and self-correcting.
    fn sub_current_size(&self, bytes: usize) {
        let _ = self
            .current_size
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_sub(bytes))
            });
    }

    /// Returns the number of entries in the map.
    pub fn len(&self) -> usize {
        self.map.pin().len()
    }

    /// Returns true if the map is empty.
    pub fn is_empty(&self) -> bool {
        self.map.pin().is_empty()
    }

    /// Applies `f` to the entry for `key`, if present.
    ///
    /// Returns `None` when the key is absent or the cache is disabled. This
    /// gives callers access to the entry's bookkeeping (e.g. `inserted_at`
    /// for TTL checks) without cloning more than they need.
    pub fn with_entry<Q, R>(&self, key: &Q, f: impl FnOnce(&Entry<V>) -> R) -> Option<R>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        if self.is_disabled() {
            return None;
        }
        self.map.pin().get(key).map(f)
    }

    /// Inserts `value` accounted at `size_bytes`, timestamped now.
    ///
    /// See [`Self::insert_weighted_at`].
    pub fn insert_weighted(&self, key: K, value: V, size_bytes: usize) -> (Option<V>, usize)
    where
        V: Clone,
    {
        self.insert_weighted_at(key, value, size_bytes, Instant::now())
    }

    /// Inserts `value` accounted at `size_bytes` with an explicit insertion
    /// timestamp (exposed for TTL tests that need to backdate entries).
    ///
    /// If an entry already existed for this key, its accounted size is
    /// subtracted first so the running total reflects the replacement rather
    /// than ratcheting upward on every overwrite.
    ///
    /// Returns the replaced value (if any) and the new approximate total
    /// size. Eviction is not performed here; callers decide if and how to
    /// evict based on the returned total (see [`Self::evict_until_freed`]).
    pub fn insert_weighted_at(
        &self,
        key: K,
        value: V,
        size_bytes: usize,
        inserted_at: Instant,
    ) -> (Option<V>, usize)
    where
        V: Clone,
    {
        if self.is_disabled() {
            return (None, 0);
        }
        let entry = Entry {
            value,
            inserted_at,
            size_bytes,
        };
        let guard = self.map.pin();
        let (replaced_value, replaced_size) = guard
            .insert(key, entry)
            .map_or((None, 0), |old| (Some(old.value.clone()), old.size_bytes));
        self.sub_current_size(replaced_size);
        let new_total = self.current_size.fetch_add(size_bytes, Ordering::Relaxed) + size_bytes;
        (replaced_value, new_total)
    }

    /// Atomically installs `value` for `key` unless an existing entry is kept.
    ///
    /// `decide` is consulted for an entry that is already present: it returns
    /// `Some(reason)` to keep that entry (nothing is written, `reason` is
    /// handed back) or `None` to replace it. A vacant slot is always claimed.
    ///
    /// The read-modify-write is a single papaya compare-and-swap, so of many
    /// concurrent callers exactly one claims a given slot. A `get` followed by
    /// an `insert` cannot promise that: every caller can observe the vacancy
    /// and every caller then claims it, which is how single-flight guards leak
    /// duplicate producers.
    ///
    /// `decide` may run more than once (the CAS retries when the entry changes
    /// underneath it), so it must be free of side effects.
    ///
    /// When the cache is disabled nothing is stored and every caller is told it
    /// claimed the slot — with no storage there is nothing to deduplicate on.
    pub fn claim_or<R>(
        &self,
        key: K,
        value: V,
        size_bytes: usize,
        decide: impl Fn(&Entry<V>) -> Option<R>,
    ) -> Claim<R>
    where
        V: Clone,
    {
        if self.is_disabled() {
            return Claim::Claimed;
        }

        let guard = self.map.pin();
        let outcome = guard.compute(key, |existing| {
            let keep = existing.and_then(|(_, entry)| decide(entry));
            match keep {
                Some(reason) => Operation::Abort(reason),
                None => Operation::Insert(Entry::new(value.clone(), size_bytes)),
            }
        });

        match outcome {
            Compute::Aborted(reason) => Claim::Retained(reason),
            Compute::Inserted(_, _) => {
                self.current_size.fetch_add(size_bytes, Ordering::Relaxed);
                Claim::Claimed
            }
            Compute::Updated {
                old: (_, replaced), ..
            } => {
                self.sub_current_size(replaced.size_bytes);
                self.current_size.fetch_add(size_bytes, Ordering::Relaxed);
                Claim::Claimed
            }
            // `Operation::Remove` is never produced above, so the map cannot
            // report a removal. Keep the accounting honest and treat the now
            // vacant slot as claimed rather than panicking on an unreachable
            // state.
            Compute::Removed(_, removed) => {
                self.sub_current_size(removed.size_bytes);
                Claim::Claimed
            }
        }
    }

    /// Removes evictable entries in ascending `priority` order until at least
    /// `target_bytes` have been freed (or no candidates remain).
    ///
    /// `priority` returns `Some(order_key)` for evictable entries — smaller
    /// keys are evicted first — and `None` for entries that must be kept.
    pub fn evict_until_freed<P: Ord>(
        &self,
        target_bytes: usize,
        priority: impl Fn(&K, &Entry<V>) -> Option<P>,
    ) -> EvictionStats {
        let guard = self.map.pin();
        let mut candidates: Vec<(K, P)> = guard
            .iter()
            .filter_map(|(k, e)| priority(k, e).map(|p| (k.clone(), p)))
            .collect();

        // Evict in ascending priority order (e.g. oldest first).
        candidates.sort_by(|(_, a), (_, b)| a.cmp(b));

        let mut freed = 0usize;
        let mut evicted = 0usize;
        for (key, _) in candidates {
            if freed >= target_bytes {
                break;
            }
            // Account the entry that was *actually* removed rather than the one
            // observed while iterating: a concurrent overwrite between the
            // snapshot and this removal replaces the entry, and subtracting the
            // stale size would leave `current_size` permanently drifted from
            // the map's real contents.
            if let Some(removed) = guard.remove(&key) {
                freed += removed.size_bytes;
                evicted += 1;
                self.sub_current_size(removed.size_bytes);
            }
        }

        EvictionStats { evicted, freed }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_with_entry_roundtrip() {
        let map: SizeBoundedMap<String, String> = SizeBoundedMap::new(1024);

        let (replaced, total) = map.insert_weighted("k".to_string(), "value".to_string(), 10);
        assert!(replaced.is_none());
        assert_eq!(total, 10);

        let value = map.with_entry("k", |e| e.value.clone());
        assert_eq!(value.as_deref(), Some("value"));
        assert_eq!(map.current_size(), 10);
        assert_eq!(map.len(), 1);
        assert!(!map.is_empty());
    }

    #[test]
    fn test_disabled_map_misses_and_ignores_inserts() {
        let map: SizeBoundedMap<String, String> = SizeBoundedMap::new(0);
        assert!(map.is_disabled());

        let (replaced, total) = map.insert_weighted("k".to_string(), "v".to_string(), 10);
        assert!(replaced.is_none());
        assert_eq!(total, 0);
        assert!(map.with_entry("k", |e| e.value.clone()).is_none());
        assert!(map.is_empty());
        assert_eq!(map.current_size(), 0);
    }

    #[test]
    fn test_overwrite_subtracts_replaced_size() {
        // The running total must reflect the replacement, not ratchet upward.
        let map: SizeBoundedMap<String, String> = SizeBoundedMap::new(1024);

        map.insert_weighted("k".to_string(), "small".to_string(), 100);
        assert_eq!(map.current_size(), 100);

        let (replaced, total) = map.insert_weighted("k".to_string(), "large".to_string(), 250);
        assert_eq!(replaced.as_deref(), Some("small"));
        assert_eq!(total, 250);
        assert_eq!(map.current_size(), 250);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_evict_until_freed_oldest_first() {
        let map: SizeBoundedMap<String, u32> = SizeBoundedMap::new(1024);

        // Insert with explicit timestamps so ordering is deterministic.
        let base = Instant::now();
        map.insert_weighted_at("old".to_string(), 1, 100, base);
        map.insert_weighted_at(
            "new".to_string(),
            2,
            100,
            base + std::time::Duration::from_secs(1),
        );

        let stats = map.evict_until_freed(100, |_, e| Some(e.inserted_at));
        assert_eq!(stats.evicted, 1);
        assert_eq!(stats.freed, 100);
        assert!(map.with_entry("old", |_| ()).is_none());
        assert!(map.with_entry("new", |_| ()).is_some());
        assert_eq!(map.current_size(), 100);
    }

    #[test]
    fn test_evict_skips_entries_without_priority() {
        let map: SizeBoundedMap<String, u32> = SizeBoundedMap::new(1024);

        map.insert_weighted("keep".to_string(), 0, 100);
        map.insert_weighted("evictable".to_string(), 1, 100);

        // Only entries with value > 0 are evictable; the target exceeds what
        // can be freed, so eviction stops when candidates run out.
        let stats = map.evict_until_freed(1000, |_, e| (e.value > 0).then_some(e.inserted_at));
        assert_eq!(stats.evicted, 1);
        assert_eq!(stats.freed, 100);
        assert!(map.with_entry("keep", |_| ()).is_some());
        assert!(map.with_entry("evictable", |_| ()).is_none());
    }

    #[test]
    fn test_evict_accounts_the_entry_actually_removed() {
        // Regression: the eviction pass used to subtract the size it snapshotted
        // while iterating. Replacing the entry between that snapshot and the
        // removal (what a concurrent insert does) then left `current_size`
        // permanently above the map's real contents.
        let map: SizeBoundedMap<String, u32> = SizeBoundedMap::new(1024);
        map.insert_weighted("k".to_string(), 1, 100);
        assert_eq!(map.current_size(), 100);

        // The priority closure runs during the snapshot pass, so overwriting
        // from inside it reproduces the interleaving deterministically.
        let stats = map.evict_until_freed(50, |_, entry| {
            map.insert_weighted("k".to_string(), 2, 900);
            Some(entry.inserted_at)
        });

        assert_eq!(stats.evicted, 1);
        assert_eq!(
            stats.freed, 900,
            "freed bytes must reflect the removed entry, not the stale snapshot"
        );
        assert!(map.is_empty());
        assert_eq!(
            map.current_size(),
            0,
            "an emptied map must account exactly zero bytes"
        );
    }

    #[test]
    fn test_current_size_subtraction_saturates_at_zero() {
        // Defence in depth: an over-subtraction must clamp instead of wrapping
        // to ~usize::MAX, which would make the cache look permanently over
        // budget and evict on every insert.
        let map: SizeBoundedMap<String, u32> = SizeBoundedMap::new(1024);
        map.insert_weighted("k".to_string(), 1, 10);

        map.sub_current_size(usize::MAX);

        assert_eq!(map.current_size(), 0);
    }

    #[test]
    fn test_claim_or_claims_vacant_slot_and_accounts_size() {
        let map: SizeBoundedMap<String, u32> = SizeBoundedMap::new(1024);

        let claim = map.claim_or("k".to_string(), 7, 40, |_| Some("occupied"));

        assert!(matches!(claim, Claim::Claimed));
        assert_eq!(map.with_entry("k", |e| e.value), Some(7));
        assert_eq!(map.current_size(), 40);
    }

    #[test]
    fn test_claim_or_retains_existing_entry() {
        let map: SizeBoundedMap<String, u32> = SizeBoundedMap::new(1024);
        map.insert_weighted("k".to_string(), 7, 40);

        let claim = map.claim_or("k".to_string(), 9, 40, |entry| Some(entry.value));

        assert!(matches!(claim, Claim::Retained(7)));
        assert_eq!(map.with_entry("k", |e| e.value), Some(7));
        assert_eq!(
            map.current_size(),
            40,
            "a retained slot must not be charged"
        );
    }

    #[test]
    fn test_claim_or_replaces_when_decision_declines_to_keep() {
        let map: SizeBoundedMap<String, u32> = SizeBoundedMap::new(1024);
        map.insert_weighted("k".to_string(), 7, 40);

        let claim = map.claim_or("k".to_string(), 9, 10, |_| None::<()>);

        assert!(matches!(claim, Claim::Claimed));
        assert_eq!(map.with_entry("k", |e| e.value), Some(9));
        assert_eq!(
            map.current_size(),
            10,
            "replacement must subtract the old size and add the new one"
        );
    }

    #[test]
    fn test_claim_or_on_disabled_map_stores_nothing() {
        let map: SizeBoundedMap<String, u32> = SizeBoundedMap::new(0);

        let claim = map.claim_or("k".to_string(), 7, 40, |_| Some("occupied"));

        assert!(matches!(claim, Claim::Claimed));
        assert!(map.is_empty());
        assert_eq!(map.current_size(), 0);
    }

    #[test]
    fn test_claim_or_admits_exactly_one_of_many_concurrent_claimants() {
        use std::sync::Barrier;

        const THREADS: usize = 8;
        const ROUNDS: usize = 64;

        for round in 0..ROUNDS {
            let map: SizeBoundedMap<String, u32> = SizeBoundedMap::new(1024 * 1024);
            let key = format!("race-{round}");
            let barrier = Barrier::new(THREADS);
            let claims = AtomicUsize::new(0);

            std::thread::scope(|scope| {
                for _ in 0..THREADS {
                    scope.spawn(|| {
                        barrier.wait();
                        if let Claim::Claimed = map.claim_or(key.clone(), 1, 0, |_| Some(())) {
                            claims.fetch_add(1, Ordering::Relaxed);
                        }
                    });
                }
            });

            assert_eq!(
                claims.load(Ordering::Relaxed),
                1,
                "exactly one claimant may win a slot"
            );
        }
    }

    #[test]
    fn test_entry_weigh_includes_key_and_overhead() {
        let size = Entry::<Vec<u8>>::weigh(100, 7);
        assert_eq!(size, 100 + 7 + std::mem::size_of::<Entry<Vec<u8>>>());
    }
}

//! A stale-while-revalidate cache for the composed montage, keyed by the served
//! list's [`ContentSignature`].
//!
//! A request either finds the montage for the current signature (a hit, served
//! immediately) or triggers a background regeneration, waits a short while for
//! it, and otherwise serves the previous (stale) montage while it finishes.
//! Deduplication of concurrent regenerations is delegated to [`moka`], whose
//! `try_get_with` coalesces same-key inits into a single run and caches only
//! successes. This layer adds only the short wait and the stale fallback on top.

use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures::future::{BoxFuture, FutureExt, Shared};
use moka::future::Cache;
use tokio::time::timeout;
use tracing::warn;

use super::selection::ContentSignature;

/// How long a request waits for a fresh montage before falling back to the stale
/// one (or nothing). Kept short so generation never blocks a request for long.
const DEFAULT_WAIT_FOR_FRESH: Duration = Duration::from_millis(200);

/// How many recent montages to retain, so a served list that oscillates between
/// a few signatures keeps re-hitting rather than regenerating.
const MONTAGE_CACHE_CAPACITY: u64 = 4;

/// The composed montage as encoded PNG bytes, shared so cache reads are cheap.
pub type MontagePng = Arc<Vec<u8>>;

/// A regeneration in flight, awaitable by many callers at once without re-running
/// or cancelling the underlying work. It resolves to the freshly generated
/// montage, or `None` if regeneration produced none (it failed and was logged,
/// so the request falls back to the stale montage).
type RefreshHandle = Shared<BoxFuture<'static, Option<MontagePng>>>;

/// A stale-while-revalidate montage cache built on moka's single-flight cache.
pub struct MontageCache {
    /// Recent montages keyed by signature; provides storage and single-flight
    /// regeneration.
    montages: Cache<ContentSignature, MontagePng>,
    /// The most recently served/generated montage, served stale while a new
    /// signature regenerates.
    last_good: Arc<Mutex<Option<MontagePng>>>,
    /// The latest background regeneration, for [`Self::await_pending`].
    latest_refresh: Arc<Mutex<Option<RefreshHandle>>>,
    wait_for_fresh: Duration,
}

/// Lock a mutex, recovering the guard even if a previous holder panicked — a
/// poisoned montage cache should degrade, not take the process down.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl MontageCache {
    pub fn new() -> Self {
        Self {
            montages: Cache::new(MONTAGE_CACHE_CAPACITY),
            last_good: Arc::new(Mutex::new(None)),
            latest_refresh: Arc::new(Mutex::new(None)),
            wait_for_fresh: DEFAULT_WAIT_FOR_FRESH,
        }
    }

    /// Serve the montage for `signature`: a hit if the cache already holds it,
    /// otherwise a background regeneration is started and waited on briefly,
    /// falling back to the stale montage (or `None` if the cache is empty).
    pub async fn get_or_revalidate<F, Fut, E>(
        &self,
        signature: ContentSignature,
        generate: F,
    ) -> Option<MontagePng>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = std::result::Result<MontagePng, E>> + Send + 'static,
        E: std::fmt::Display + Send + Sync + 'static,
    {
        if let Some(png) = self.montages.get(&signature).await {
            self.remember(&png);
            return Some(png);
        }
        let refresh = self.spawn_refresh(signature, generate);
        match timeout(self.wait_for_fresh, refresh).await {
            Ok(Some(png)) => Some(png),
            // Didn't land in time, or failed: serve whatever we last had.
            Ok(None) | Err(_) => self.stale(),
        }
    }

    /// Generate once and populate the cache, awaiting completion. A failure
    /// leaves the cache untouched (non-fatal) rather than erroring — used to warm
    /// the cache at startup.
    pub async fn warm<F, Fut, E>(&self, signature: ContentSignature, generate: F)
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = std::result::Result<MontagePng, E>>,
        E: std::fmt::Display + Send + Sync + 'static,
    {
        match self.montages.try_get_with(signature, generate()).await {
            Ok(png) => self.remember(&png),
            Err(error) => warn!(%error, "montage cache warm-up failed"),
        }
    }

    /// Await the latest background regeneration (no-op if none). For graceful
    /// shutdown and deterministic tests.
    pub async fn await_pending(&self) {
        let refresh = lock(&self.latest_refresh).clone();
        if let Some(refresh) = refresh {
            let _ = refresh.await;
        }
    }

    fn remember(&self, png: &MontagePng) {
        *lock(&self.last_good) = Some(png.clone());
    }

    fn stale(&self) -> Option<MontagePng> {
        lock(&self.last_good).clone()
    }

    /// Start a background regeneration for `signature`, returning an awaitable
    /// handle. The work runs on a spawned task, so it completes (updating the
    /// cache and last-good montage) regardless of whether the caller waits;
    /// moka coalesces concurrent regenerations of the same signature.
    fn spawn_refresh<F, Fut, E>(&self, signature: ContentSignature, generate: F) -> RefreshHandle
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = std::result::Result<MontagePng, E>> + Send + 'static,
        E: std::fmt::Display + Send + Sync + 'static,
    {
        // The spawned task is `'static` and can't borrow `&self`, so move owned
        // handles in. These are cheap shared-handle clones (moka `Cache` and
        // `Arc` are ref-counted), so the task writes through to the *same* store
        // this `MontageCache` reads from — not a copy.
        let montages = self.montages.clone();
        let last_good = self.last_good.clone();
        let generate = generate();
        let task = tokio::spawn(async move {
            // moka caches only successes, keeping a failed generation out.
            match montages.try_get_with(signature, generate).await {
                Ok(png) => {
                    *lock(&last_good) = Some(png.clone());
                    Some(png)
                }
                Err(error) => {
                    warn!(%error, "montage regeneration failed");
                    None
                }
            }
        });

        let refresh: RefreshHandle = task.map(|joined| joined.ok().flatten()).boxed().shared();
        *lock(&self.latest_refresh) = Some(refresh.clone());
        refresh
    }
}

impl Default for MontageCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// A regeneration result with a stringly error — the cache only needs the
    /// error to be `Display`.
    type Regen = std::result::Result<MontagePng, &'static str>;

    fn png(marker: u8) -> MontagePng {
        Arc::new(vec![marker; 8])
    }

    fn sig(s: &str) -> ContentSignature {
        ContentSignature::from_raw(s)
    }

    fn ok(marker: u8) -> Regen {
        Ok(png(marker))
    }

    fn fail() -> Regen {
        Err("regeneration failed")
    }

    fn never() -> Regen {
        panic!("regeneration should not run")
    }

    #[tokio::test]
    async fn matching_signature_hits_without_regenerating() {
        let cache = MontageCache::new();
        cache.warm(sig("A"), || async { ok(1) }).await;

        let regen_calls = Arc::new(AtomicUsize::new(0));
        let counter = regen_calls.clone();
        let served = cache
            .get_or_revalidate(sig("A"), || async move {
                counter.fetch_add(1, Ordering::SeqCst);
                ok(2)
            })
            .await;

        assert_eq!(served, Some(png(1)), "the cached image is served");
        assert_eq!(
            regen_calls.load(Ordering::SeqCst),
            0,
            "a hit must not regenerate"
        );
    }

    #[tokio::test]
    async fn a_miss_regenerates_and_caches_the_result() {
        let cache = MontageCache::new();

        let fresh = cache
            .get_or_revalidate(sig("A"), || async { ok(7) })
            .await;
        assert_eq!(fresh, Some(png(7)), "fresh image returned within the wait");

        let cached = cache
            .get_or_revalidate(sig("A"), || async { never() })
            .await;
        assert_eq!(cached, Some(png(7)), "second request hits the cache");
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_regeneration_serves_stale_then_updates_in_the_background() {
        // Paused time: tokio auto-advances the virtual clock, so the 200ms wait
        // fires before the (virtually) minute-long regen — deterministically.
        let cache = MontageCache::new();
        cache.warm(sig("A"), || async { ok(1) }).await;

        let served = cache
            .get_or_revalidate(sig("B"), || async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                ok(2)
            })
            .await;
        assert_eq!(served, Some(png(1)), "serves stale A while B regenerates");

        cache.await_pending().await;
        let now = cache
            .get_or_revalidate(sig("B"), || async { never() })
            .await;
        assert_eq!(now, Some(png(2)), "B is cached once its regeneration lands");
    }

    #[tokio::test(start_paused = true)]
    async fn an_empty_cache_serves_nothing_while_regenerating_then_fills() {
        let cache = MontageCache::new();

        let served = cache
            .get_or_revalidate(sig("A"), || async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                ok(9)
            })
            .await;
        assert_eq!(
            served, None,
            "nothing to serve while empty and regenerating"
        );

        cache.await_pending().await;
        let now = cache
            .get_or_revalidate(sig("A"), || async { never() })
            .await;
        assert_eq!(now, Some(png(9)), "the montage lands after regeneration");
    }

    #[tokio::test]
    async fn a_failed_regeneration_is_not_cached() {
        let cache = MontageCache::new();

        let served = cache.get_or_revalidate(sig("A"), || async { fail() }).await;
        assert_eq!(served, None, "nothing served when regeneration fails");

        let retried = cache
            .get_or_revalidate(sig("A"), || async { ok(3) })
            .await;
        assert_eq!(retried, Some(png(3)), "a later attempt can still succeed");
    }
}

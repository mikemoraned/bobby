//! A value cached against a version key — the shared "recompute only when the
//! relevant table version(s) moved" mechanism.
//!
//! It serves both **value caches** (`T` is the cached value, via
//! [`VersionedCache::get_cached_if_current`]) and **skip-if-unchanged gates**
//! (`T = ()`, via [`VersionedCache::is_cached_current`]). The caller owns
//! synchronization — wrap it in a lock for shared `&self` access, or hold it
//! behind `&mut self`.

/// A single value remembered against the version it was computed at.
pub struct VersionedCache<V, T> {
    entry: Option<(V, T)>,
}

impl<V, T> VersionedCache<V, T> {
    #[must_use]
    pub const fn new() -> Self {
        Self { entry: None }
    }
}

impl<V, T> Default for VersionedCache<V, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: PartialEq, T> VersionedCache<V, T> {
    /// The cached value, if one is stored at `version`.
    pub fn get_cached_if_current(&self, version: &V) -> Option<&T> {
        self.entry
            .as_ref()
            .and_then(|(v, value)| (v == version).then_some(value))
    }

    /// Whether a value is cached at `version` (ignores the value — for gate use,
    /// where `T = ()`).
    pub fn is_cached_current(&self, version: &V) -> bool {
        self.entry.as_ref().is_some_and(|(v, _)| v == version)
    }

    /// Cache `value` at `version`, replacing any previous entry.
    pub fn cache(&mut self, version: V, value: T) {
        self.entry = Some((version, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_cache_has_nothing() {
        let cache: VersionedCache<u64, &str> = VersionedCache::new();
        assert_eq!(cache.get_cached_if_current(&1), None);
        assert!(!cache.is_cached_current(&1));
    }

    #[test]
    fn get_returns_value_only_at_its_version() {
        let mut cache = VersionedCache::new();
        cache.cache(7u64, "seven");
        assert_eq!(cache.get_cached_if_current(&7), Some(&"seven"));
        assert_eq!(cache.get_cached_if_current(&8), None);
    }

    #[test]
    fn is_current_tracks_the_stored_version() {
        let mut cache: VersionedCache<u64, ()> = VersionedCache::new();
        assert!(!cache.is_cached_current(&3));
        cache.cache(3, ());
        assert!(cache.is_cached_current(&3));
        assert!(!cache.is_cached_current(&4));
    }

    #[test]
    fn insert_replaces_the_previous_entry() {
        let mut cache = VersionedCache::new();
        cache.cache(1u64, "a");
        cache.cache(2u64, "b");
        assert_eq!(cache.get_cached_if_current(&1), None);
        assert_eq!(cache.get_cached_if_current(&2), Some(&"b"));
    }
}

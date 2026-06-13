use std::collections::HashSet;

use skeet_store::{
    IMAGE_APPRAISAL_TABLE_NAME, SCORE_TABLE_NAME, SKEET_APPRAISAL_TABLE_NAME, Version,
};

/// Tables whose version changes should trigger a feed recompute.
///
/// The feed depends on scored images and manual appraisals; new images or
/// skeets only become visible once scored, so the images and skeets tables are
/// deliberately excluded.
pub const RELEVANT_TABLES: &[&str] = &[
    SCORE_TABLE_NAME,
    SKEET_APPRAISAL_TABLE_NAME,
    IMAGE_APPRAISAL_TABLE_NAME,
];

/// The relevant subset of a version snapshot — the table versions a feed
/// recompute actually depends on.
///
/// Two snapshots with equal relevant subsets mean nothing the feed cares about
/// has moved, so the result doubles as a version key to gate against (e.g. in a
/// [`skeet_store::VersionedCache`]).
pub fn relevant(snapshot: &HashSet<Version>) -> HashSet<Version> {
    snapshot
        .iter()
        .filter(|v| RELEVANT_TABLES.contains(&v.name.as_str()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(name: &str, tag: &str) -> Version {
        Version::new(name, tag)
    }

    #[test]
    fn keeps_only_relevant_tables() {
        let snapshot: HashSet<Version> = [
            version(SCORE_TABLE_NAME, "a"),
            version(SKEET_APPRAISAL_TABLE_NAME, "a"),
            version(IMAGE_APPRAISAL_TABLE_NAME, "a"),
            version("images", "z"),
            version("skeets", "z"),
        ]
        .into_iter()
        .collect();
        assert_eq!(relevant(&snapshot).len(), 3);
        assert!(relevant(&snapshot).iter().all(|v| v.name != "images"));
    }

    #[test]
    fn unchanged_relevant_subset_when_irrelevant_table_moves() {
        let base = [version(SCORE_TABLE_NAME, "a"), version("images", "a")];
        let moved = [version(SCORE_TABLE_NAME, "a"), version("images", "b")];
        let a: HashSet<Version> = base.into_iter().collect();
        let b: HashSet<Version> = moved.into_iter().collect();
        assert_eq!(relevant(&a), relevant(&b));
    }
}

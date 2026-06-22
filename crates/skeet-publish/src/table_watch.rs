use std::collections::HashSet;

use skeet_store::{TableName, Version};

/// Tables whose version changes should trigger a feed recompute.
///
/// The feed depends on scored images and manual appraisals; new images or
/// skeets only become visible once scored, so the images and skeets tables are
/// deliberately excluded.
pub const RELEVANT_TABLES: &[&str] = &[
    TableName::Scores.as_str(),
    TableName::SkeetAppraisal.as_str(),
    TableName::ImageAppraisal.as_str(),
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
            version(TableName::Scores.as_str(), "a"),
            version(TableName::SkeetAppraisal.as_str(), "a"),
            version(TableName::ImageAppraisal.as_str(), "a"),
            version(TableName::Images.as_str(), "z"),
            version(TableName::Validate.as_str(), "z"),
        ]
        .into_iter()
        .collect();
        assert_eq!(relevant(&snapshot).len(), 3);
        assert!(
            relevant(&snapshot)
                .iter()
                .all(|v| v.name != TableName::Images.as_str())
        );
    }

    #[test]
    fn unchanged_relevant_subset_when_irrelevant_table_moves() {
        let base = [
            version(TableName::Scores.as_str(), "a"),
            version(TableName::Images.as_str(), "a"),
        ];
        let moved = [
            version(TableName::Scores.as_str(), "a"),
            version(TableName::Images.as_str(), "b"),
        ];
        let a: HashSet<Version> = base.into_iter().collect();
        let b: HashSet<Version> = moved.into_iter().collect();
        assert_eq!(relevant(&a), relevant(&b));
    }
}

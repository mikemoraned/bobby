use std::collections::HashSet;

use tracing::instrument;

use crate::error::StoreError;
use crate::SkeetStore;

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct Version {
    pub name: String,
    pub tag: String,
}

impl Version {
    pub fn new(name: impl Into<String>, tag: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            tag: tag.into(),
        }
    }
}

impl SkeetStore {
    /// Snapshot the current version of every table, returned as a `HashSet<Version>`.
    ///
    /// `Version.tag` is an opaque string derived from each table's lancedb version,
    /// so callers can compare snapshots without coupling to the underlying
    /// representation.
    #[instrument(skip(self))]
    pub async fn version_snapshot(&self) -> Result<HashSet<Version>, StoreError> {
        let mut snapshot = HashSet::with_capacity(self.tables.len());
        for (name, table) in &self.tables {
            let tag = table.version().await?.to_string();
            snapshot.insert(Version::new(*name, tag));
        }
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{
        IMAGE_APPRAISAL_TABLE_NAME, SCORE_TABLE_NAME, SKEET_APPRAISAL_TABLE_NAME, TABLE_NAME,
        VALIDATE_TABLE_NAME,
    };
    use crate::test_utils::{make_record, open_temp_store};
    use crate::{ModelVersion, Score};

    fn names(snapshot: &HashSet<Version>) -> HashSet<String> {
        snapshot.iter().map(|v| v.name.clone()).collect()
    }

    fn tag_for(snapshot: &HashSet<Version>, name: &str) -> String {
        snapshot
            .iter()
            .find(|v| v.name == name)
            .map(|v| v.tag.clone())
            .expect("table missing from snapshot")
    }

    #[tokio::test]
    async fn version_snapshot_includes_all_known_tables() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = open_temp_store(&dir).await;

        let snapshot = store.version_snapshot().await.expect("version snapshot");

        let expected: HashSet<String> = [
            TABLE_NAME,
            SCORE_TABLE_NAME,
            SKEET_APPRAISAL_TABLE_NAME,
            IMAGE_APPRAISAL_TABLE_NAME,
            VALIDATE_TABLE_NAME,
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        assert_eq!(names(&snapshot), expected);
    }

    #[tokio::test]
    async fn version_snapshot_changes_only_for_written_table() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = open_temp_store(&dir).await;

        let before = store.version_snapshot().await.expect("snapshot before");

        let record = make_record("vsnap1", 10, 0, 0);
        store.add(&record).await.expect("add image");

        let after_image = store.version_snapshot().await.expect("snapshot after image");
        assert_ne!(
            tag_for(&before, TABLE_NAME),
            tag_for(&after_image, TABLE_NAME),
            "images table tag should change after add"
        );
        assert_eq!(
            tag_for(&before, SCORE_TABLE_NAME),
            tag_for(&after_image, SCORE_TABLE_NAME),
            "scores table tag should be unchanged"
        );

        store
            .upsert_score(
                &record.image_id,
                &Score::new(0.5).expect("valid score"),
                &ModelVersion::from("test"),
            )
            .await
            .expect("upsert score");

        let after_score = store.version_snapshot().await.expect("snapshot after score");
        assert_ne!(
            tag_for(&after_image, SCORE_TABLE_NAME),
            tag_for(&after_score, SCORE_TABLE_NAME),
            "scores table tag should change after upsert"
        );
        assert_eq!(
            tag_for(&after_image, TABLE_NAME),
            tag_for(&after_score, TABLE_NAME),
            "images table tag should be unchanged after score upsert"
        );
    }
}

use std::collections::HashSet;

use async_trait::async_trait;

use super::schema::TableName;
use crate::error::StoreError;
use crate::{SkeetStore, TableVersions, Version};

#[async_trait]
impl TableVersions for SkeetStore {
    async fn table_version(&self, table: &str) -> Result<Version, StoreError> {
        let name = TableName::from_name(table)
            .ok_or_else(|| StoreError::UnknownTable(table.to_string()))?;
        let v = self.table(name).version().await?;
        Ok(Version::new(name.as_str(), v.to_string()))
    }

    async fn version_snapshot(&self) -> Result<HashSet<Version>, StoreError> {
        let mut snapshot = HashSet::with_capacity(self.tables.len());
        for (name, t) in &self.tables {
            let v = t.version().await?;
            snapshot.insert(Version::new(name.as_str(), v.to_string()));
        }
        Ok(snapshot)
    }
}

impl SkeetStore {
    /// The numeric LanceDB version counter for each table — the gauge source for
    /// version metrics. Store-agnostic callers should prefer the opaque
    /// [`TableVersions`] port instead.
    pub async fn table_versions(&self) -> Result<Vec<(TableName, u64)>, StoreError> {
        let mut versions = Vec::with_capacity(self.tables.len());
        for (name, table) in &self.tables {
            let v = table.version().await?;
            versions.push((name, v));
        }
        Ok(versions)
    }

    /// The fragment count for each table — a LanceDB storage-maintenance signal
    /// (drives compaction scheduling and gauges). Lance-physical, so it stays off
    /// the [`TableVersions`] port.
    pub async fn fragment_counts(&self) -> Result<Vec<(TableName, u64)>, StoreError> {
        let mut counts = Vec::with_capacity(self.tables.len());
        for (name, table) in &self.tables {
            let native = table
                .as_native()
                .ok_or_else(|| StoreError::CannotGetFragmentCount {
                    table: name.to_string(),
                    reason: "table is not a native LanceDB table".to_string(),
                })?;
            let count = native.count_fragments().await?;
            counts.push((name, count as u64));
        }
        Ok(counts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{make_record, open_temp_store};
    use crate::{Images, ModelScore, ModelVersion, Score, Scores};

    fn names(snapshot: &HashSet<Version>) -> HashSet<String> {
        snapshot.iter().map(|v| v.name.clone()).collect()
    }

    fn tag_for(snapshot: &HashSet<Version>, table: TableName) -> String {
        snapshot
            .iter()
            .find(|v| v.name == table.as_str())
            .map(|v| v.tag.clone())
            .expect("table missing from snapshot")
    }

    #[tokio::test]
    async fn version_snapshot_includes_all_known_tables() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = open_temp_store(&dir).await;

        let snapshot = store.version_snapshot().await.expect("version snapshot");

        let expected: HashSet<String> = [
            TableName::Images,
            TableName::Scores,
            TableName::SkeetAppraisal,
            TableName::ImageAppraisal,
            TableName::Validate,
            TableName::PruneStats,
        ]
        .iter()
        .map(|t| t.as_str().to_string())
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

        let after_image = store
            .version_snapshot()
            .await
            .expect("snapshot after image");
        assert_ne!(
            tag_for(&before, TableName::Images),
            tag_for(&after_image, TableName::Images),
            "images table tag should change after add"
        );
        assert_eq!(
            tag_for(&before, TableName::Scores),
            tag_for(&after_image, TableName::Scores),
            "scores table tag should be unchanged"
        );

        store
            .upsert_score(
                &record.image_id,
                ModelScore {
                    score: Score::new(0.5).expect("valid score"),
                    model_version: ModelVersion::from("test"),
                },
            )
            .await
            .expect("upsert score");

        let after_score = store
            .version_snapshot()
            .await
            .expect("snapshot after score");
        assert_ne!(
            tag_for(&after_image, TableName::Scores),
            tag_for(&after_score, TableName::Scores),
            "scores table tag should change after upsert"
        );
        assert_eq!(
            tag_for(&after_image, TableName::Images),
            tag_for(&after_score, TableName::Images),
            "images table tag should be unchanged after score upsert"
        );
    }
}

use async_trait::async_trait;
use shared::{Appraisal, Appraiser, Band, ImageId, SkeetId};

use crate::StoreError;

/// One manual-appraisal table, keyed by `K` (`SkeetId` or `ImageId`). The CRUD
/// surface a consumer needs; the concrete handle stays in the adapter.
#[async_trait]
pub trait Appraisals<K: Send + Sync>: Send + Sync {
    /// Upsert the appraisal for `id`.
    async fn set(&self, id: &K, band: Band, appraiser: &Appraiser) -> Result<(), StoreError>;
    /// Remove any appraisal for `id`.
    async fn clear(&self, id: &K) -> Result<(), StoreError>;
    /// The appraisal for `id`, if one is stored.
    async fn get(&self, id: &K) -> Result<Option<Appraisal>, StoreError>;
    /// Every stored `(key, appraisal)` pair.
    async fn list_all(&self) -> Result<Vec<(K, Appraisal)>, StoreError>;
}

/// Access to the per-key appraisal tables. The seam generic and `dyn` consumers
/// use to reach appraisals without naming the concrete adapter.
pub trait AppraisalsSource: Send + Sync {
    fn skeet_appraisals(&self) -> Box<dyn Appraisals<SkeetId>>;
    fn image_appraisals(&self) -> Box<dyn Appraisals<ImageId>>;
}

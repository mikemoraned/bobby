use shared::{ImageId, SkeetId};

use crate::Appraisals;

/// Access to the per-key appraisal tables. The seam generic and `dyn` consumers
/// use to reach appraisals without naming the concrete `SkeetStore`.
pub trait AppraisalSource: Send + Sync {
    fn skeet_appraisals(&self) -> Appraisals<SkeetId>;
    fn image_appraisals(&self) -> Appraisals<ImageId>;
}

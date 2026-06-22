use crate::{Appraiser, Band};

/// A stored manual appraisal: the band assigned and who assigned it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Appraisal {
    pub band: Band,
    pub appraiser: Appraiser,
}

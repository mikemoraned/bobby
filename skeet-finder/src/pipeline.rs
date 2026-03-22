use shared::Rejection;
use skeet_store::ImageRecord;

pub enum FilterResult {
    Post { image_count: u64 },
    Classified(Box<ImageRecord>),
    Rejected(Vec<Rejection>),
}

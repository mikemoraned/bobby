use skeet_store::SkeetStore;
use tokio::sync::mpsc;
use tracing::warn;

use crate::pipeline::FilterResult;
use crate::{persistence, status};

pub async fn run(
    rx: &mut mpsc::Receiver<FilterResult>,
    store: &SkeetStore,
    fallback: Option<&SkeetStore>,
) {
    let mut status = status::Status::new(std::time::Duration::from_secs(30), 100);

    while let Some(result) = rx.recv().await {
        match result {
            FilterResult::Post { image_count } => {
                status.record_post(image_count);
            }
            FilterResult::Classified(record) => {
                if let Some(fallback_store) = fallback {
                    persistence::save_with_fallback(
                        store,
                        fallback_store,
                        &record,
                        &mut status,
                    )
                    .await;
                } else {
                    persistence::save(store, &record, &mut status).await;
                }
            }
            FilterResult::Rejected(reasons) => {
                status.record_rejected(&reasons);
            }
        }
    }

    warn!("filter stage ended, shutting down");
}

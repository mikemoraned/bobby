use async_channel::{Receiver, Sender};
use skeet_store::Images;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::persistence;
use crate::persistence::SaveOutcome;
use crate::pipeline::{self, ContentCounts, ImageMessage, StatsMessage};

/// Persist the survivors of each message, folding the resulting `saved` count
/// into the message's `ContentCounts` and forwarding it on for tallying.
///
/// `saved` is determined here — whether an item is actually persisted depends
/// on storage state (it may already exist) — so this is the stage that decides
/// it and folds it into the running tally, exactly as every other stage folds
/// the decisions it makes.
pub async fn run(
    rx: &Receiver<ImageMessage>,
    store: &impl Images,
    stats_tx: Sender<StatsMessage>,
    token: CancellationToken,
) {
    let mut saved_total: u64 = 0;

    while let Some((records, mut counts)) = pipeline::recv(rx, &token).await {
        for record in records {
            if matches!(persistence::save(store, &record).await, SaveOutcome::Saved) {
                saved_total += 1;
                counts += &ContentCounts::saved();
                info!(
                    saved = saved_total,
                    skeet_id = %record.skeet_id,
                    zone = %record.zone,
                    "saved image"
                );
            }
        }
        if pipeline::forward(&stats_tx, counts, &token).await.is_err() {
            break;
        }
    }

    warn!("save stage ended, shutting down");
}

#[cfg(test)]
mod tests {
    use skeet_store::test_utils::{make_record, open_temp_store};
    use tokio_util::sync::CancellationToken;

    use super::*;

    /// A fresh record is saved (folding `saved: 1` into its forwarded counts)
    /// while a pre-seeded record is skipped (leaving its counts untouched).
    #[tokio::test]
    async fn folds_only_fresh_saves_into_forwarded_counts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = open_temp_store(&dir).await;

        let fresh = make_record("fresh", 1, 0, 0);
        let existing = make_record("existing", 2, 0, 0);
        store.add(&existing).await.expect("seed already-exists record");

        let (in_tx, in_rx) = async_channel::bounded(16);
        let (stats_tx, stats_rx) = async_channel::bounded::<StatsMessage>(16);
        in_tx
            .send((vec![fresh], ContentCounts::post(3)))
            .await
            .expect("send fresh message");
        in_tx
            .send((vec![existing], ContentCounts::post(1)))
            .await
            .expect("send existing message");
        drop(in_tx);

        run(&in_rx, &store, stats_tx, CancellationToken::new()).await;

        let fresh_counts = stats_rx.recv().await.expect("fresh counts forwarded");
        assert_eq!(fresh_counts.posts, 1);
        assert_eq!(fresh_counts.images, 3);
        assert_eq!(fresh_counts.saved, 1);

        let existing_counts = stats_rx.recv().await.expect("existing counts forwarded");
        assert_eq!(existing_counts.posts, 1);
        assert_eq!(existing_counts.saved, 0);

        assert!(stats_rx.recv().await.is_err(), "channel closed after run");
    }
}

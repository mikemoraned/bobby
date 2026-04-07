use std::sync::atomic::Ordering;
use std::sync::Arc;

use face_detection::FaceDetector;
use shared::{ModelVersion, PruneConfig};
use tokio::sync::mpsc;
use tracing::warn;

use crate::pipeline::{ImageResult, MetaResult, PipelineCounters};

pub async fn run(
    rx: &mut mpsc::Receiver<MetaResult>,
    tx: mpsc::Sender<ImageResult>,
    http: reqwest::Client,
    detector: FaceDetector,
    prune_config: PruneConfig,
    config_version: ModelVersion,
    counters: Arc<PipelineCounters>,
) {
    while let Some(result) = rx.recv().await {
        match result {
            MetaResult::Candidate(candidate) => {
                counters.image.fetch_add(1, Ordering::Relaxed);
                let skeet_images =
                    crate::firehose::download_candidate_images(&candidate, &http).await;

                for skeet_image in skeet_images {
                    let result = match crate::classify_image(
                        skeet_image,
                        &detector,
                        &prune_config,
                        &config_version,
                    ) {
                        Ok(record) => ImageResult::Classified(Box::new(record)),
                        Err(reasons) => ImageResult::Rejected(reasons),
                    };

                    if tx.send(result).await.is_err() {
                        warn!("downstream dropped, shutting down image filter");
                        return;
                    }
                }
            }
            MetaResult::Post { image_count } => {
                if tx.send(ImageResult::Post { image_count }).await.is_err() {
                    warn!("downstream dropped, shutting down image filter");
                    return;
                }
            }
            MetaResult::Rejected(reasons) => {
                if tx.send(ImageResult::Rejected(reasons)).await.is_err() {
                    warn!("downstream dropped, shutting down image filter");
                    return;
                }
            }
        }
    }
}

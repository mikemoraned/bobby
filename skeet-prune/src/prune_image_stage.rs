use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use face_detection::FaceDetector;
use shared::{ModelVersion, PruneConfig};
use tokio::sync::mpsc;
use tracing::{debug, warn};

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

                let download_start = Instant::now();
                let skeet_images =
                    crate::firehose::download_candidate_images(&candidate, &http).await;
                let download_ms = download_start.elapsed().as_millis();

                let image_count = skeet_images.len();
                let classify_start = Instant::now();
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
                let classify_ms = classify_start.elapsed().as_millis();

                debug!(
                    download_ms,
                    classify_ms,
                    image_count,
                    "candidate processed"
                );
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

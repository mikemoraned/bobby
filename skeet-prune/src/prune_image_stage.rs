use std::sync::atomic::Ordering;
use std::sync::Arc;

use face_detection::FaceDetector;
use shared::{ModelVersion, PruneConfig};
use tokio::sync::{Mutex, mpsc};
use tracing::{info, warn};

use crate::pipeline::{ImageResult, MetaResult, PipelineCounters};

pub async fn run_workers(
    rx: mpsc::Receiver<MetaResult>,
    tx: mpsc::Sender<ImageResult>,
    http: reqwest::Client,
    prune_config: PruneConfig,
    config_version: ModelVersion,
    counters: Arc<PipelineCounters>,
    num_workers: usize,
) {
    info!(num_workers, "starting image stage workers");

    let rx = Arc::new(Mutex::new(rx));
    let mut handles = Vec::with_capacity(num_workers);

    let enable_text = prune_config.is_category_enabled(shared::RejectionCategory::Text);

    for worker_id in 0..num_workers {
        let rx = Arc::clone(&rx);
        let tx = tx.clone();
        let http = http.clone();
        let config_version = config_version.clone();
        let prune_config = prune_config.clone();
        let counters = Arc::clone(&counters);

        handles.push(tokio::spawn(async move {
            let face = FaceDetector::from_bundled_weights();
            let text = if enable_text {
                info!(worker_id, "loading text detection models");
                Some(text_detection::TextDetector::from_bundled_models())
            } else {
                None
            };
            let detectors = Detectors { face, text };
            info!(worker_id, "image worker ready");
            run_single(rx, tx, http, detectors, prune_config, config_version, counters).await;
        }));
    }

    for handle in handles {
        if let Err(e) = handle.await {
            warn!("image worker panicked: {e}");
        }
    }
}

struct Detectors {
    face: FaceDetector,
    text: Option<text_detection::TextDetector>,
}

async fn run_single(
    rx: Arc<Mutex<mpsc::Receiver<MetaResult>>>,
    tx: mpsc::Sender<ImageResult>,
    http: reqwest::Client,
    detectors: Detectors,
    prune_config: PruneConfig,
    config_version: ModelVersion,
    counters: Arc<PipelineCounters>,
) {
    loop {
        let result = rx.lock().await.recv().await;
        let Some(result) = result else { return };

        match result {
            MetaResult::Candidate(candidate) => {
                counters.image.fetch_add(1, Ordering::Relaxed);

                let skeet_images =
                    crate::firehose::download_candidate_images(&candidate, &http).await;

                for skeet_image in skeet_images {
                    let result = match crate::classify_image(
                        skeet_image,
                        &detectors.face,
                        detectors.text.as_ref(),
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

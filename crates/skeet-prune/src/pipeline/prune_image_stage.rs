use std::sync::Arc;
use std::sync::atomic::Ordering;

use async_channel::{Receiver, Sender};
use face_detection::FaceDetector;
use shared::{ModelVersion, PruneConfig};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::pipeline::{self, ImageResult, MetaResult, PipelineCounters};

// The text-detection models are compile-time-bundled assets; a load failure is an
// unrecoverable startup error for the worker, so panicking the spawned task is intended.
#[allow(clippy::expect_used)]
fn load_text_detector() -> text_detection::TextDetector {
    text_detection::TextDetector::from_bundled_models()
        .expect("failed to load text detection models")
}

/// Per-worker classification inputs, cloned into each spawned worker.
#[derive(Clone)]
pub struct ClassifyConfig {
    pub http: reqwest::Client,
    pub prune_config: PruneConfig,
    pub config_version: ModelVersion,
}

pub async fn run_workers(
    rx: Receiver<MetaResult>,
    tx: Sender<ImageResult>,
    config: ClassifyConfig,
    counters: Arc<PipelineCounters>,
    num_workers: usize,
    token: CancellationToken,
) {
    info!(num_workers, "starting image stage workers");

    let mut handles = Vec::with_capacity(num_workers);

    let enable_text = config
        .prune_config
        .is_category_enabled(shared::RejectionCategory::Text);

    for worker_id in 0..num_workers {
        let rx = rx.clone();
        let tx = tx.clone();
        let config = config.clone();
        let counters = Arc::clone(&counters);
        let token = token.clone();

        handles.push(tokio::spawn(async move {
            let face = FaceDetector::from_bundled_weights();
            let text = if enable_text {
                info!(worker_id, "loading text detection models");
                Some(load_text_detector())
            } else {
                None
            };
            let detectors = Detectors { face, text };
            info!(worker_id, "image worker ready");
            run_single(&rx, &tx, config, detectors, &counters, &token).await;
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
    rx: &Receiver<MetaResult>,
    tx: &Sender<ImageResult>,
    config: ClassifyConfig,
    detectors: Detectors,
    counters: &PipelineCounters,
    token: &CancellationToken,
) {
    let ClassifyConfig {
        http,
        prune_config,
        config_version,
    } = config;
    while let Some(result) = pipeline::recv(rx, token).await {
        match result {
            MetaResult::Candidate(candidate) => {
                counters.image.fetch_add(1, Ordering::Relaxed);

                let skeet_images =
                    crate::firehose::download_candidate_images(&candidate, &http).await;

                for skeet_image in skeet_images {
                    let result = match crate::classify::classify_image(
                        skeet_image,
                        &detectors.face,
                        detectors.text.as_ref(),
                        &prune_config,
                        &config_version,
                    ) {
                        Ok(record) => ImageResult::Classified(Box::new(record)),
                        Err(reasons) => ImageResult::Rejected(reasons),
                    };

                    if pipeline::forward(tx, result, token).await.is_err() {
                        return;
                    }
                }
            }
            MetaResult::Post { image_count } => {
                if pipeline::forward(tx, ImageResult::Post { image_count }, token)
                    .await
                    .is_err()
                {
                    return;
                }
            }
            MetaResult::Rejected(reasons) => {
                if pipeline::forward(tx, ImageResult::Rejected(reasons), token)
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
    }
}

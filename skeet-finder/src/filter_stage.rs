use face_detection::FaceDetector;
use shared::{ArchetypeConfig, ConfigVersion};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::pipeline::FilterResult;

pub async fn run(
    tx: mpsc::Sender<FilterResult>,
    http: reqwest::Client,
    detector: FaceDetector,
    text_detector: text_detection::TextDetector,
    archetype_config: ArchetypeConfig,
    config_version: ConfigVersion,
) {
    let recv_timeout = std::time::Duration::from_secs(30);

    loop {
        let receiver = match crate::firehose::connect().await {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "failed to connect to firehose, retrying");
                continue;
            }
        };
        info!("firehose connected, listening for posts...");

        loop {
            let event = match tokio::time::timeout(recv_timeout, receiver.recv_async()).await {
                Ok(Ok(event)) => event,
                Ok(Err(_)) => {
                    warn!("firehose channel closed");
                    break;
                }
                Err(_) => {
                    warn!("no message received in {recv_timeout:?}, reconnecting");
                    break;
                }
            };

            let skeet_images =
                crate::firehose::extract_skeet_images(&event, &http).await;
            let image_count = skeet_images.len() as u64;

            for skeet_image in skeet_images {
                let result = match crate::classify_image(
                    skeet_image,
                    &detector,
                    &text_detector,
                    &archetype_config,
                    &config_version,
                ) {
                    Ok(record) => FilterResult::Classified(Box::new(record)),
                    Err(reasons) => FilterResult::Rejected(reasons),
                };

                if tx.send(result).await.is_err() {
                    warn!("save stage dropped, shutting down filter");
                    return;
                }
            }

            if tx.send(FilterResult::Post { image_count }).await.is_err() {
                warn!("save stage dropped, shutting down filter");
                return;
            }
        }
    }
}

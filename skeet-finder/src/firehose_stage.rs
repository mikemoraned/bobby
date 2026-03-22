use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::firehose::SkeetCandidate;

pub async fn run(tx: mpsc::Sender<SkeetCandidate>) {
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

            if let Some(candidate) = crate::firehose::extract_skeet_candidate(&event)
                && tx.send(candidate).await.is_err()
            {
                warn!("downstream dropped, shutting down firehose");
                return;
            }
        }
    }
}

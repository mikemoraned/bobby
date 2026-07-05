//! Jetstream ingress: connecting to a public Bluesky Jetstream instance and
//! interpreting the `app.bsky.feed.post` records it streams.
//!
//! Covers the raw transport ([`connect`]) plus the record-interpretation helpers
//! that read a commit's labels, image embeds, blob CIDs, and timestamps. The
//! reconnect/cursor/backoff logic that wraps [`connect`] lives with its consumer.

use atrium_api::{
    app::bsky::{
        embed::{images::Image, record_with_media::MainMediaRefs},
        feed::post::{RecordEmbedRefs, RecordLabelsRefs},
    },
    types::{BlobRef, TypedBlobRef, Union},
};
use chrono::{DateTime, Utc};
use jetstream_oxide::{
    DefaultJetstreamEndpoints, JetstreamCompression, JetstreamConfig, JetstreamConnector,
    JetstreamReceiver, exports::Nsid,
};
use shared::BlueskyCid;
use std::time::Duration;
use tracing::{info, warn};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

const ALL_ENDPOINTS: [DefaultJetstreamEndpoints; 4] = [
    DefaultJetstreamEndpoints::USEastOne,
    DefaultJetstreamEndpoints::USEastTwo,
    DefaultJetstreamEndpoints::USWestOne,
    DefaultJetstreamEndpoints::USWestTwo,
];

/// Connect to a public Jetstream instance filtered to `app.bsky.feed.post`,
/// resuming from `cursor` (or live-tailing when `None`).
///
/// Tries the known endpoints in a shuffled order and returns the first
/// [`JetstreamReceiver`] that connects within [`CONNECT_TIMEOUT`], or an error
/// if none do.
pub async fn connect(
    cursor: Option<DateTime<Utc>>,
) -> Result<JetstreamReceiver, Box<dyn std::error::Error>> {
    info!("connecting to firehose");

    let wanted_collections = vec!["app.bsky.feed.post".parse::<Nsid>()?];

    let mut endpoints: Vec<String> = ALL_ENDPOINTS.map(Into::into).to_vec();
    fastrand::shuffle(&mut endpoints);

    for endpoint_str in &endpoints {
        info!(endpoint = %endpoint_str, "trying endpoint");

        let config = JetstreamConfig {
            endpoint: endpoint_str.clone(),
            compression: JetstreamCompression::Zstd,
            wanted_collections: wanted_collections.clone(),
            max_retries: 0,
            cursor,
            ..Default::default()
        };

        let connector = JetstreamConnector::new(config)?;
        match tokio::time::timeout(CONNECT_TIMEOUT, connector.connect()).await {
            Ok(Ok(receiver)) => {
                info!(endpoint = %endpoint_str, "connected to firehose");
                return Ok(receiver);
            }
            Ok(Err(e)) => {
                warn!(endpoint = %endpoint_str, error = %e, "connection failed");
            }
            Err(_) => {
                warn!(endpoint = %endpoint_str, "connection timed out after {:?}", CONNECT_TIMEOUT);
            }
        }
    }

    Err(format!(
        "failed to connect to any firehose endpoint after trying all {} endpoints",
        ALL_ENDPOINTS.len()
    )
    .into())
}

/// Whether a post carries a self-applied moderation label in
/// [`shared::labels::EXCLUDED_VALUES`].
pub fn has_excluded_label(labels: &Option<Union<RecordLabelsRefs>>) -> bool {
    let Some(Union::Refs(RecordLabelsRefs::ComAtprotoLabelDefsSelfLabels(self_labels))) = labels
    else {
        return false;
    };
    self_labels
        .values
        .iter()
        .any(|label| shared::labels::EXCLUDED_VALUES.contains(&label.val.as_str()))
}

/// The image blobs embedded in a post, whether embedded directly or alongside a
/// quoted record. Empty when the post has no image embed.
pub fn extract_images(embed: &Option<Union<RecordEmbedRefs>>) -> Vec<&Image> {
    let Some(Union::Refs(refs)) = embed else {
        return Vec::new();
    };
    match refs {
        RecordEmbedRefs::AppBskyEmbedImagesMain(images) => images.images.iter().collect(),
        RecordEmbedRefs::AppBskyEmbedRecordWithMediaMain(record_with_media) => {
            if let Union::Refs(MainMediaRefs::AppBskyEmbedImagesMain(images)) =
                &record_with_media.media
            {
                images.images.iter().collect()
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

/// The blob's content id as a [`BlueskyCid`], or `None` if the ref carries no
/// parseable CID.
pub fn blob_cid(blob_ref: &BlobRef) -> Option<BlueskyCid> {
    let cid_str = match blob_ref {
        BlobRef::Typed(TypedBlobRef::Blob(blob)) => blob.r#ref.0.to_string(),
        BlobRef::Untyped(untyped) => untyped.cid.clone(),
    };
    BlueskyCid::new(cid_str).ok()
}

/// The post's `createdAt` timestamp in UTC.
pub fn parse_created_at(dt: &atrium_api::types::string::Datetime) -> DateTime<Utc> {
    let fixed: &chrono::DateTime<chrono::FixedOffset> = dt.as_ref();
    fixed.with_timezone(&Utc)
}

use std::collections::HashMap;
use std::io::Cursor;
use std::time::Duration;

use async_trait::async_trait;
use image::ImageReader;
use reqwest::StatusCode;
use tracing::warn;

use super::ImageStatus;
use crate::dimensions::Dimensions;
use crate::image_url::ImageUrl;

/// Checks whether image urls still exist, measuring their dimensions in passing.
#[async_trait]
pub trait ImageProber: Send + Sync {
    /// The [`ImageStatus`] of each image url. Fail-open: an inconclusive check
    /// reports the image as still present.
    async fn probe_images(&self, urls: &[ImageUrl]) -> HashMap<ImageUrl, ImageStatus>;
}

/// Probes the real Bluesky CDN, fetching at most `concurrency` images at once.
pub struct CdnImageProber {
    client: reqwest::Client,
    concurrency: usize,
}

impl CdnImageProber {
    pub fn new(concurrency: usize) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self {
            client,
            concurrency: concurrency.max(1),
        }
    }
}

#[async_trait]
impl ImageProber for CdnImageProber {
    async fn probe_images(&self, urls: &[ImageUrl]) -> HashMap<ImageUrl, ImageStatus> {
        let client = self.client.clone();
        super::probe_bounded(urls, self.concurrency, move |url| {
            let client = client.clone();
            async move {
                let status = probe_one_image(&client, &url).await;
                (url, status)
            }
        })
        .await
    }
}

/// Whether an image url exists, from the status of a GET (`None` = transport
/// failure). Fail-open: only a definitive "not there" (`404`/`410`) is treated
/// as gone; everything else — transport failures, rate-limits, server errors —
/// is inconclusive and reported as still existing.
const fn exists_from_status(status: Option<StatusCode>) -> bool {
    !matches!(status, Some(StatusCode::NOT_FOUND | StatusCode::GONE))
}

/// GET an image, deriving its [`ImageStatus`]: a header-read GET proves existence
/// and yields dimensions in one request.
async fn probe_one_image(client: &reqwest::Client, url: &ImageUrl) -> ImageStatus {
    match client.get(url.as_str()).send().await {
        Err(e) => {
            warn!(url = %url, error = %e, "image probe failed; treating as still present");
            ImageStatus {
                exists: true,
                dimensions: None,
            }
        }
        Ok(response) => {
            let status = response.status();
            if status.is_success() {
                let dimensions = response
                    .bytes()
                    .await
                    .ok()
                    .and_then(|b| read_dimensions(&b));
                ImageStatus {
                    exists: true,
                    dimensions,
                }
            } else {
                ImageStatus {
                    exists: exists_from_status(Some(status)),
                    dimensions: None,
                }
            }
        }
    }
}

/// Read an image's pixel dimensions from its header (no full decode). `None` on
/// any decode failure.
fn read_dimensions(bytes: &[u8]) -> Option<Dimensions> {
    ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
        .map(|(width, height)| Dimensions { width, height })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_not_found_is_gone_other_statuses_fail_open() {
        assert!(exists_from_status(Some(StatusCode::OK)));
        assert!(!exists_from_status(Some(StatusCode::NOT_FOUND)));
        assert!(!exists_from_status(Some(StatusCode::GONE)));
        // Server errors and rate-limits are inconclusive → present.
        assert!(exists_from_status(Some(StatusCode::INTERNAL_SERVER_ERROR)));
        assert!(exists_from_status(Some(StatusCode::TOO_MANY_REQUESTS)));
        // A transport failure (no status) is inconclusive → present.
        assert!(exists_from_status(None));
    }
}

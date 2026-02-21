use std::fmt;

use crate::images::Cid;
use crate::jetstream::Did;

#[derive(Debug, Clone)]
pub struct ImageUrl(String);

impl fmt::Display for ImageUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

pub struct FetchedImage {
    pub image: image::DynamicImage,
    pub url: ImageUrl,
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("non-success status {status} for {url}")]
    BadStatus { status: u16, url: ImageUrl },

    #[error("failed to decode image from {url}: {source}")]
    ImageDecode {
        url: ImageUrl,
        source: image::ImageError,
    },
}

pub fn cdn_url(did: &Did, cid: &Cid) -> ImageUrl {
    ImageUrl(format!(
        "https://cdn.bsky.app/img/feed_fullsize/plain/{}/{}@jpeg",
        did.0, cid.0
    ))
}

pub async fn fetch_image(
    client: &reqwest::Client,
    did: &Did,
    cid: &Cid,
) -> Result<FetchedImage, FetchError> {
    let url = cdn_url(did, cid);
    let response = client.get(&url.0).send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(FetchError::BadStatus {
            status: status.as_u16(),
            url,
        });
    }
    let bytes = response.bytes().await?;
    let image = image::load_from_memory(&bytes).map_err(|source| FetchError::ImageDecode {
        url: url.clone(),
        source,
    })?;
    Ok(FetchedImage { image, url })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cdn_url_formats_correctly() {
        let did = Did("did:plc:abc123".to_string());
        let cid = Cid("bafkrei1234".to_string());
        let url = cdn_url(&did, &cid);
        assert_eq!(
            url.0,
            "https://cdn.bsky.app/img/feed_fullsize/plain/did:plc:abc123/bafkrei1234@jpeg"
        );
    }
}

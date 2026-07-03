//! The social-media preview image.
//!
//! A 1200×630 PNG served at [`PREVIEW_ROUTE_PATH`] and referenced by the home
//! page's Open Graph / Twitter Card meta tags so a shared link unfurls with a
//! montage of the feed.

use std::io::Cursor;

use cot::http::HeaderValue;
use cot::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use cot::response::Response;
use cot::{Body, Result};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use thiserror::Error;
use tracing::{info, instrument};

/// Standard Open Graph / Twitter `summary_large_image` size (1.91:1): unfurls
/// large on Facebook, X, LinkedIn and Slack.
pub const PREVIEW_WIDTH: u32 = 1200;
pub const PREVIEW_HEIGHT: u32 = 630;

/// The route the preview image is served at; also the path the home page's
/// `og:image` URL is built from, so route and meta tag can't drift.
pub const PREVIEW_ROUTE_PATH: &str = "/preview.png";

/// A short shared cache so scrapers and CDNs revalidate cheaply.
const PREVIEW_CACHE_CONTROL: &str = "public, max-age=60";

#[derive(Debug, Error)]
pub enum PreviewError {
    #[error("failed to encode preview PNG: {0}")]
    Encode(#[from] image::ImageError),
}

/// Encode an image as PNG bytes.
fn encode_png(image: &DynamicImage) -> std::result::Result<Vec<u8>, PreviewError> {
    let mut buffer = Cursor::new(Vec::new());
    image.write_to(&mut buffer, ImageFormat::Png)?;
    Ok(buffer.into_inner())
}

/// A solid-fill 1200×630 PNG standing in for the composed montage until dynamic
/// composition exists, so the Open Graph plumbing can be verified end-to-end.
fn placeholder_png() -> std::result::Result<Vec<u8>, PreviewError> {
    let image = RgbImage::from_pixel(PREVIEW_WIDTH, PREVIEW_HEIGHT, Rgb([24, 24, 33]));
    encode_png(&DynamicImage::ImageRgb8(image))
}

#[instrument(skip_all)]
pub async fn preview() -> Result<Response> {
    info!("serving {PREVIEW_ROUTE_PATH}");
    let png = placeholder_png()
        .map_err(|e| cot::Error::internal(format!("failed to render preview image: {e}")))?;
    let mut response = Response::new(Body::fixed(png));
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("image/png"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(PREVIEW_CACHE_CONTROL));
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_is_a_valid_png_of_the_target_size() {
        let bytes = placeholder_png().expect("render placeholder");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "PNG magic bytes");
        let decoded = image::load_from_memory(&bytes).expect("decode placeholder");
        assert_eq!(decoded.width(), PREVIEW_WIDTH);
        assert_eq!(decoded.height(), PREVIEW_HEIGHT);
    }
}

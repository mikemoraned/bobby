//! The social-media preview image.
//!
//! A 1200×630 PNG served at [`PREVIEW_ROUTE_PATH`] and referenced by the home
//! page's Open Graph / Twitter Card meta tags so a shared link unfurls with a
//! montage of the feed.

pub mod selection;
pub mod tiles;

use std::io::Cursor;

use cot::http::HeaderValue;
use cot::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use cot::response::Response;
use cot::{Body, Result};
use image::imageops::{self, FilterType};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
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

/// Number of equal-width columns the montage is laid out in — i.e. how many
/// tiles sit across the top. Every tile is scaled to the column width, so the
/// montage reads as a few images per row rather than one giant image.
const MONTAGE_COLUMNS: u32 = 4;

/// The number of rows the montage guarantees room for even when tiles are tall
/// portraits: tile heights are capped so this many rows always fit, so one tall
/// portrait can't blow out a whole row and leave the canvas half empty. Shorter
/// (landscape) tiles still let extra rows peek in below.
const MONTAGE_MIN_ROWS: u32 = 2;

/// Gutter between tiles (and around the montage edges), in pixels.
const TILE_GUTTER: u32 = 6;

/// Height of the bottom fade-to-background band, in pixels.
const FADE_HEIGHT: u32 = 220;

/// The montage background, and the colour the bottom fade resolves to.
const BACKGROUND: Rgba<u8> = Rgba([24, 24, 33, 255]);

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

/// The column width every tile is scaled to: the canvas width less the gutters
/// on each side and between columns, divided across the columns.
const fn target_tile_width(canvas_width: u32) -> u32 {
    canvas_width.saturating_sub((MONTAGE_COLUMNS + 1) * TILE_GUTTER) / MONTAGE_COLUMNS
}

/// The tallest a tile may be: the canvas height less inter-row gutters, split so
/// `MONTAGE_MIN_ROWS` rows always fit. Portraits taller than this are cropped.
const fn max_tile_height(canvas_height: u32) -> u32 {
    canvas_height.saturating_sub((MONTAGE_MIN_ROWS + 1) * TILE_GUTTER) / MONTAGE_MIN_ROWS
}

/// Scale an image to `target_width`, preserving its aspect ratio.
fn scale_to_width(image: &DynamicImage, target_width: u32) -> RgbaImage {
    let height =
        (u64::from(target_width) * u64::from(image.height()) / u64::from(image.width().max(1)))
            .max(1) as u32;
    image
        .resize_exact(target_width, height, FilterType::Lanczos3)
        .to_rgba8()
}

/// Scale to `target_width` (aspect preserved), then centre-crop the height down
/// to at most `max_height`, so an over-tall portrait fills its column without
/// dominating its row. Landscapes and squares end up shorter than the cap and
/// pass through untouched.
fn scale_and_cap(image: &DynamicImage, target_width: u32, max_height: u32) -> RgbaImage {
    let scaled = scale_to_width(image, target_width);
    if scaled.height() <= max_height {
        return scaled;
    }
    let top = (scaled.height() - max_height) / 2;
    imageops::crop_imm(&scaled, 0, top, target_width, max_height).to_image()
}

/// Where one tile is placed on the canvas: the inner (gutter-inset) top-left
/// corner and the tile's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Placement {
    tile_index: usize,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// Lay the tiles out in shelf rows: `MONTAGE_COLUMNS` tiles per row, each row as
/// tall as its tallest tile. Rows that would overflow the canvas are dropped, so
/// the output stays bounded no matter how many tiles are offered. Deterministic
/// for fixed input.
fn plan_layout(sizes: &[(u32, u32)], canvas: (u32, u32)) -> Vec<Placement> {
    let (canvas_width, canvas_height) = canvas;
    let columns = MONTAGE_COLUMNS as usize;
    let column_width = target_tile_width(canvas_width);
    let mut placements = Vec::with_capacity(sizes.len());
    let mut y = TILE_GUTTER;
    for (row_index, row) in sizes.chunks(columns).enumerate() {
        let row_height = row.iter().map(|&(_, height)| height).max().unwrap_or(0);
        if y + row_height + TILE_GUTTER > canvas_height {
            break;
        }
        for (column, &(width, height)) in row.iter().enumerate() {
            placements.push(Placement {
                tile_index: row_index * columns + column,
                x: TILE_GUTTER + column as u32 * (column_width + TILE_GUTTER),
                y,
                width,
                height,
            });
        }
        y += row_height + TILE_GUTTER;
    }
    placements
}

/// Linearly interpolate one colour channel from `from` to `to` by `t` in `[0,1]`.
fn lerp_channel(from: u8, to: u8, t: f32) -> u8 {
    f32::from(from).mul_add(1.0 - t, f32::from(to) * t).round() as u8
}

/// Fade the bottom `FADE_HEIGHT` rows into the background so the montage reads as
/// a legible "taste" of the feed rather than a hard-cropped grid.
fn apply_bottom_fade(canvas: &mut RgbaImage) {
    let (width, height) = (canvas.width(), canvas.height());
    let fade_px = FADE_HEIGHT.min(height);
    if fade_px == 0 {
        return;
    }
    let fade_start = height - fade_px;
    for y in fade_start..height {
        let t = (y - fade_start) as f32 / fade_px as f32;
        for x in 0..width {
            let px = canvas.get_pixel_mut(x, y);
            px[0] = lerp_channel(px[0], BACKGROUND[0], t);
            px[1] = lerp_channel(px[1], BACKGROUND[1], t);
            px[2] = lerp_channel(px[2], BACKGROUND[2], t);
        }
    }
}

/// Compose the tiles into a montage PNG of `size`.
///
/// Each tile is scaled to a common width (over-tall portraits centre-cropped so
/// rows stay compact), the tiles are laid out in shelf rows onto a background
/// canvas, the bottom is faded, and the result is PNG-encoded. Tiles that don't
/// fit are dropped. Deterministic for a fixed input, and does no IO — pixels in,
/// PNG bytes out.
pub fn compose(
    tiles: &[DynamicImage],
    size: (u32, u32),
) -> std::result::Result<Vec<u8>, PreviewError> {
    let (width, height) = size;
    let target_width = target_tile_width(width);
    let max_height = max_tile_height(height);
    let scaled: Vec<RgbaImage> = tiles
        .iter()
        .map(|tile| scale_and_cap(tile, target_width, max_height))
        .collect();
    let sizes: Vec<(u32, u32)> = scaled.iter().map(|t| (t.width(), t.height())).collect();

    let mut canvas = RgbaImage::from_pixel(width, height, BACKGROUND);
    for placement in plan_layout(&sizes, size) {
        let tile = &scaled[placement.tile_index];
        imageops::overlay(
            &mut canvas,
            tile,
            i64::from(placement.x),
            i64::from(placement.y),
        );
    }
    apply_bottom_fade(&mut canvas);
    encode_png(&DynamicImage::ImageRgba8(canvas))
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

    const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
    const PREVIEW_SIZE: (u32, u32) = (PREVIEW_WIDTH, PREVIEW_HEIGHT);

    /// A solid-colour tile of the given size, standing in for a fetched thumbnail.
    fn tile(width: u32, height: u32, colour: [u8; 3]) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(width, height, Rgb(colour)))
    }

    /// A spread of tiles with varied aspect ratios (portrait, landscape, square),
    /// as real thumbnails would be.
    fn varied_tiles(count: usize) -> Vec<DynamicImage> {
        let shapes = [(800, 600), (600, 800), (700, 700), (900, 500), (500, 900)];
        (0..count)
            .map(|i| {
                let (w, h) = shapes[i % shapes.len()];
                tile(w, h, [(i * 23) as u8, 90, 160])
            })
            .collect()
    }

    fn overlaps(a: &Placement, b: &Placement) -> bool {
        a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
    }

    #[test]
    fn placeholder_is_a_valid_png_of_the_target_size() {
        let bytes = placeholder_png().expect("render placeholder");
        assert_eq!(&bytes[..8], PNG_MAGIC, "PNG magic bytes");
        let decoded = image::load_from_memory(&bytes).expect("decode placeholder");
        assert_eq!(decoded.width(), PREVIEW_WIDTH);
        assert_eq!(decoded.height(), PREVIEW_HEIGHT);
    }

    #[test]
    fn compose_produces_a_png_of_the_requested_size() {
        let bytes = compose(&varied_tiles(10), PREVIEW_SIZE).expect("compose");
        assert_eq!(&bytes[..8], PNG_MAGIC, "PNG magic bytes");
        let decoded = image::load_from_memory(&bytes).expect("decode montage");
        assert_eq!((decoded.width(), decoded.height()), PREVIEW_SIZE);
    }

    #[test]
    fn compose_is_deterministic_for_a_fixed_input() {
        let tiles = varied_tiles(10);
        let first = compose(&tiles, PREVIEW_SIZE).expect("compose");
        let second = compose(&tiles, PREVIEW_SIZE).expect("compose");
        assert_eq!(first, second, "same tiles must yield identical bytes");
    }

    #[test]
    fn compose_handles_fewer_than_the_target_count() {
        let bytes = compose(&varied_tiles(3), PREVIEW_SIZE).expect("compose");
        let decoded = image::load_from_memory(&bytes).expect("decode montage");
        assert_eq!((decoded.width(), decoded.height()), PREVIEW_SIZE);
    }

    #[test]
    fn tall_portraits_are_capped_but_shorter_tiles_pass_through() {
        let width = target_tile_width(PREVIEW_WIDTH);
        let max_height = max_tile_height(PREVIEW_HEIGHT);

        // A very tall portrait is cropped to exactly the column width × cap.
        let tall = scale_and_cap(&tile(600, 1600, [10, 20, 30]), width, max_height);
        assert_eq!((tall.width(), tall.height()), (width, max_height));

        // A landscape stays shorter than the cap, aspect preserved.
        let wide = scale_and_cap(&tile(900, 500, [10, 20, 30]), width, max_height);
        assert_eq!(wide.width(), width);
        assert!(wide.height() < max_height, "landscape should not be capped");
    }

    #[test]
    fn compose_with_no_tiles_still_renders_the_background() {
        let bytes = compose(&[], PREVIEW_SIZE).expect("compose");
        let decoded = image::load_from_memory(&bytes).expect("decode montage");
        assert_eq!((decoded.width(), decoded.height()), PREVIEW_SIZE);
    }

    #[test]
    fn layout_keeps_tiles_in_bounds_unrotated_and_non_overlapping() {
        // Uniform target width (as `compose` produces), varied heights.
        let target = target_tile_width(PREVIEW_WIDTH);
        let sizes: Vec<(u32, u32)> = (0..10)
            .map(|i| (target, [200, 390, 300, 500, 250][i % 5]))
            .collect();
        let placements = plan_layout(&sizes, PREVIEW_SIZE);

        for p in &placements {
            assert!(
                p.x + p.width <= PREVIEW_WIDTH && p.y + p.height <= PREVIEW_HEIGHT,
                "placement {p:?} escapes the canvas"
            );
            assert_eq!(
                (p.width, p.height),
                sizes[p.tile_index],
                "tile {} was rotated by the packer",
                p.tile_index
            );
        }
        for (i, a) in placements.iter().enumerate() {
            for b in &placements[i + 1..] {
                assert!(!overlaps(a, b), "placements overlap: {a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn layout_fits_a_few_tiles_across_the_top_row() {
        let target = target_tile_width(PREVIEW_WIDTH);
        let sizes: Vec<(u32, u32)> = (0..10)
            .map(|i| (target, [200, 390, 300, 500, 250][i % 5]))
            .collect();
        let placements = plan_layout(&sizes, PREVIEW_SIZE);

        let top = placements.iter().map(|p| p.y).min().expect("some placement");
        let across_top = placements.iter().filter(|p| p.y == top).count();
        assert!(
            (3..=4).contains(&across_top),
            "expected 3–4 tiles across the top, got {across_top}"
        );
    }
}

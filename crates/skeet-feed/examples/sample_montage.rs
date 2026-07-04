//! Render a sample preview montage from synthetic, varied-aspect tiles so the
//! row/gutter/fade geometry can be eyeballed without live thumbnails.
//!
//! Usage: `cargo run -p skeet-feed --example sample_montage -- [OUT_PATH]`
//! (defaults to `sample-montage.png` in the current directory).

use image::{DynamicImage, Rgb, RgbImage};
use skeet_feed::preview::{PREVIEW_HEIGHT, PREVIEW_WIDTH, compose};

fn tile(width: u32, height: u32, colour: [u8; 3]) -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::from_pixel(width, height, Rgb(colour)))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A spread of portrait / landscape / square shapes and distinct colours so
    // tile edges read clearly against the dark gutters.
    let shapes = [
        (800, 600, [214, 93, 82]),
        (600, 800, [232, 168, 56]),
        (700, 700, [86, 170, 120]),
        (900, 500, [72, 132, 204]),
        (520, 780, [150, 96, 190]),
        (760, 640, [220, 120, 170]),
        (680, 760, [96, 186, 196]),
        (840, 560, [186, 176, 84]),
        (560, 720, [120, 128, 220]),
        (720, 700, [200, 140, 96]),
    ];
    let tiles: Vec<DynamicImage> = shapes
        .iter()
        .map(|&(w, h, c)| tile(w, h, c))
        .collect();

    let png = compose(&tiles, (PREVIEW_WIDTH, PREVIEW_HEIGHT))?;

    let out_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sample-montage.png".to_string());
    std::fs::write(&out_path, png)?;
    println!("wrote {out_path}");
    Ok(())
}

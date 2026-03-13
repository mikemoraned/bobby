use burn::prelude::*;
use burn::backend::NdArray;
use image::DynamicImage;

const INPUT_SIZE: u32 = 640;

type B = NdArray;

/// Resize image to 640x640 and convert to NCHW BGR float tensor.
/// Returns (tensor, scale_x, scale_y) where scale factors map model coords back to original.
pub fn image_to_tensor(image: &DynamicImage) -> (Tensor<B, 4>, f32, f32) {
    let original_width = image.width();
    let original_height = image.height();

    let resized = image.resize_exact(INPUT_SIZE, INPUT_SIZE, image::imageops::FilterType::Triangle);
    let rgb = resized.to_rgb8();

    let mut data = vec![0.0f32; 3 * (INPUT_SIZE as usize) * (INPUT_SIZE as usize)];
    let hw = (INPUT_SIZE * INPUT_SIZE) as usize;

    for (i, pixel) in rgb.pixels().enumerate() {
        // BGR order (OpenCV convention for YuNet)
        data[i] = f32::from(pixel[2]);           // B channel
        data[hw + i] = f32::from(pixel[1]);       // G channel
        data[2 * hw + i] = f32::from(pixel[0]);   // R channel
    }

    let device = Default::default();
    let tensor = Tensor::<B, 1>::from_floats(data.as_slice(), &device)
        .reshape([1, 3, INPUT_SIZE as i64, INPUT_SIZE as i64]);

    let scale_x = original_width as f32 / INPUT_SIZE as f32;
    let scale_y = original_height as f32 / INPUT_SIZE as f32;

    (tensor, scale_x, scale_y)
}

//! Bounded opaque scanner-image decoding shared by local examples.

use std::{error::Error, path::Path};

use image::{GrayImage, ImageReader};

pub(super) fn read(path: &Path, max_pixels: usize) -> Result<GrayImage, Box<dyn Error>> {
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    opaque_grayscale(reader.decode()?, max_pixels)
}

fn opaque_grayscale(
    image: image::DynamicImage,
    max_pixels: usize,
) -> Result<GrayImage, Box<dyn Error>> {
    if u64::from(image.width()) * u64::from(image.height()) > max_pixels as u64 {
        return Err("image exceeds the source-pixel budget".into());
    }
    if image.color().has_alpha() && image.to_rgba16().pixels().any(|pixel| pixel[3] != u16::MAX) {
        return Err(
            "scanner input must be opaque; transparent pixels require an explicit validity mask"
                .into(),
        );
    }
    Ok(image.into_luma8())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageBuffer, Rgba};

    #[test]
    fn transparent_hidden_pixels_are_not_interpreted_as_scanner_evidence() {
        let transparent =
            DynamicImage::ImageRgba8(ImageBuffer::from_pixel(2, 2, Rgba([0, 0, 0, 0])));
        let almost_opaque =
            DynamicImage::ImageRgba16(ImageBuffer::from_pixel(2, 2, Rgba([0, 0, 0, 65_534])));
        assert!(opaque_grayscale(transparent, 4).is_err());
        assert!(opaque_grayscale(almost_opaque, 4).is_err());
    }

    #[test]
    fn opaque_images_respect_the_pixel_budget() {
        let image =
            DynamicImage::ImageRgba8(ImageBuffer::from_pixel(2, 2, Rgba([100, 100, 100, 255])));
        assert!(opaque_grayscale(image.clone(), 3).is_err());
        assert_eq!(opaque_grayscale(image, 4).unwrap().as_raw(), &[100; 4]);
    }
}

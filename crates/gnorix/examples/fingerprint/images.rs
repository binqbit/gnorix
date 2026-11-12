//! Bounded scanner-image decoding and lossless grayscale stage export.

use std::{error::Error, path::Path};

use gnorix::biometrics::fingerprint::{Fingerprint, RidgeMap};
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

pub(super) fn save(
    root: &Path,
    source: &GrayImage,
    result: &Fingerprint,
) -> Result<(), Box<dyn Error>> {
    source.save(root.join("source.png"))?;
    save_palette(root, result)?;
    save_clarity(root, result)?;
    save_values(
        root,
        "normalized.png",
        result,
        result
            .normalized
            .iter()
            .map(|&v| ((v + 3.0) / 6.0 * 255.0).round() as u8),
    )?;
    save_values(
        root,
        "denoised.png",
        result,
        result
            .denoised
            .iter()
            .map(|&v| ((v + 3.0) / 6.0 * 255.0).round() as u8),
    )?;
    save_values(
        root,
        "coherence.png",
        result,
        result.coherence.iter().map(|&v| (v * 255.0).round() as u8),
    )?;
    save_values(
        root,
        "usable.png",
        result,
        result
            .binary
            .usable()
            .iter()
            .map(|&v| if v { 255 } else { 0 }),
    )?;
    save_map(root, "binary.png", &result.binary)?;
    save_map(root, "skeleton.png", &result.skeleton)?;
    save_map(root, "measurement-skeleton.png", &result.scale.skeleton)?;
    save_values(
        root,
        "pattern-support.png",
        result,
        result
            .scale
            .pattern_support
            .iter()
            .map(|&v| if v { 255 } else { 0 }),
    )?;
    Ok(())
}

fn save_palette(root: &Path, result: &Fingerprint) -> Result<(), Box<dyn Error>> {
    for (filename, values) in [
        ("ridge-level.png", &result.palette.ridge_gray),
        ("valley-level.png", &result.palette.valley_gray),
    ] {
        save_values(
            root,
            filename,
            result,
            values.iter().map(|v| v.round() as u8),
        )?;
    }
    save_values(
        root,
        "normalization-support.png",
        result,
        result
            .palette
            .usable
            .iter()
            .map(|&v| if v { 255 } else { 0 }),
    )
}

fn save_values(
    root: &Path,
    filename: &str,
    result: &Fingerprint,
    values: impl Iterator<Item = u8>,
) -> Result<(), Box<dyn Error>> {
    let image = GrayImage::from_raw(
        result.binary.width() as u32,
        result.binary.height() as u32,
        values.collect(),
    )
    .ok_or("invalid stage dimensions")?;
    image.save(root.join(filename))?;
    Ok(())
}

fn save_map(root: &Path, filename: &str, map: &RidgeMap) -> Result<(), Box<dyn Error>> {
    let values = map
        .ridges()
        .iter()
        .map(|&v| if v { 0 } else { 255 })
        .collect();
    GrayImage::from_raw(map.width() as u32, map.height() as u32, values)
        .ok_or("invalid ridge dimensions")?
        .save(root.join(filename))?;
    Ok(())
}

fn save_clarity(root: &Path, result: &Fingerprint) -> Result<(), Box<dyn Error>> {
    let map = &result.clarity;
    for (filename, values) in [
        ("clarity.png", &map.values),
        ("clarity-contrast.png", &map.contrast),
        ("clarity-separation.png", &map.separation),
        ("clarity-repetition.png", &map.repetition),
    ] {
        save_values(
            root,
            filename,
            result,
            values.iter().map(|&value| (value * 255.0).round() as u8),
        )?;
    }
    save_values(
        root,
        "clarity-support.png",
        result,
        map.supported
            .iter()
            .map(|&valid| if valid { 255 } else { 0 }),
    )
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

//! Fixed demonstration catalog with optional, locally imported research scans.

use std::{collections::BTreeSet, io::Read, path::Path, sync::Arc};

use image::{GrayImage, ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::Result;

const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_PNG: &[u8] =
    include_bytes!("../../../crates/gnorix/tests/assets/test-fingerprint.png");

#[derive(Clone, Serialize)]
pub struct ImageView {
    pub id: String,
    pub title: String,
    pub description: String,
    pub width: u32,
    pub height: u32,
    pub url: String,
}

pub struct GalleryImage {
    pub view: ImageView,
    pub image: GrayImage,
    pub png: Arc<[u8]>,
}

pub struct Gallery {
    images: Vec<GalleryImage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    images: Vec<ImportedImage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportedImage {
    id: String,
    title: String,
    description: String,
    source: String,
    source_sha256: String,
    pixels_sha256: String,
    width: u32,
    height: u32,
}

impl Gallery {
    pub fn load(directory: &Path) -> Result<Self> {
        let manifest: Manifest = serde_json::from_str(include_str!("../gallery.json"))?;
        Self::load_manifest(directory, manifest)
    }

    fn load_manifest(directory: &Path, manifest: Manifest) -> Result<Self> {
        validate_manifest(&manifest)?;
        let image = image::load_from_memory(DEFAULT_PNG)?.into_luma8();
        let mut images = vec![GalleryImage {
            view: image_view(
                "test".into(),
                "Test fingerprint".into(),
                "The default synthetic fingerprint used by the project.".into(),
                &image,
            ),
            image,
            png: Arc::from(DEFAULT_PNG),
        }];

        for entry in manifest.images {
            let path = directory.join(format!("{}.png", entry.id));
            let file = match std::fs::File::open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(format!("{}: {error}", path.display()).into()),
            };
            let mut bytes = Vec::new();
            file.take(MAX_IMAGE_BYTES + 1).read_to_end(&mut bytes)?;
            let image = decode_import(&entry, &bytes)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            images.push(GalleryImage {
                view: image_view(entry.id, entry.title, entry.description, &image),
                image,
                png: Arc::from(bytes),
            });
        }
        Ok(Self { images })
    }

    pub fn default_image(&self) -> &GalleryImage {
        &self.images[0]
    }

    pub fn get(&self, id: &str) -> Option<&GalleryImage> {
        self.images.iter().find(|entry| entry.view.id == id)
    }

    pub fn views(&self) -> Vec<ImageView> {
        self.images.iter().map(|entry| entry.view.clone()).collect()
    }

    #[cfg(test)]
    pub(super) fn test_catalog() -> Self {
        use image::ImageEncoder;
        let images = [("test", 0), ("other", 200)]
            .into_iter()
            .map(|(id, value)| {
                let image = GrayImage::from_pixel(2, 2, image::Luma([value]));
                let mut png = Vec::new();
                image::codecs::png::PngEncoder::new(&mut png)
                    .write_image(image.as_raw(), 2, 2, image::ExtendedColorType::L8)
                    .unwrap();
                GalleryImage {
                    view: image_view(id.into(), id.into(), "Test input".into(), &image),
                    image,
                    png: Arc::from(png),
                }
            })
            .collect();
        Self { images }
    }
}

fn image_view(id: String, title: String, description: String, image: &GrayImage) -> ImageView {
    ImageView {
        url: format!("/api/images/{id}"),
        id,
        title,
        description,
        width: image.width(),
        height: image.height(),
    }
}

fn validate_manifest(manifest: &Manifest) -> Result<()> {
    if manifest.images.len() > 5 {
        return Err("the demonstration gallery supports five curated scans".into());
    }
    let mut ids = BTreeSet::new();
    for entry in &manifest.images {
        if entry.id.is_empty()
            || entry.id == "test"
            || entry.id.len() > 80
            || !entry
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || !ids.insert(&entry.id)
            || entry.title.trim().is_empty()
            || entry.description.trim().is_empty()
            || !entry.source.starts_with("Real/")
            || !entry.source.ends_with(".BMP")
            || entry.source[5..].contains(['/', '\\'])
            || !valid_digest(&entry.source_sha256)
            || !valid_digest(&entry.pixels_sha256)
            || !(1..=4096).contains(&entry.width)
            || !(1..=4096).contains(&entry.height)
        {
            return Err("invalid curated fingerprint gallery entry".into());
        }
    }
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn decode_import(entry: &ImportedImage, bytes: &[u8]) -> Result<GrayImage> {
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err("gallery PNG exceeds the 16 MiB limit".into());
    }
    let mut reader = ImageReader::with_format(std::io::Cursor::new(bytes), ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(entry.width);
    limits.max_image_height = Some(entry.height);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode()?;
    if image.color() != image::ColorType::L8
        || (image.width(), image.height()) != (entry.width, entry.height)
    {
        return Err(
            "gallery PNG must have the declared dimensions and opaque 8-bit grayscale pixels"
                .into(),
        );
    }
    let image = image.into_luma8();
    let digest: String = Sha256::digest(image.as_raw())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if digest != entry.pixels_sha256 {
        return Err(
            "gallery pixels do not match the curated source; import the gallery again".into(),
        );
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    fn entry() -> ImportedImage {
        ImportedImage {
            id: "scan-1".into(),
            title: "Test scan".into(),
            description: "Control pixels".into(),
            source: "Real/example.BMP".into(),
            source_sha256: "0".repeat(64),
            pixels_sha256: Sha256::digest([0; 4])
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            width: 2,
            height: 2,
        }
    }

    #[test]
    fn validates_exact_pixels_and_rejects_color_alpha_and_wrong_shape() {
        for (pixels, width, height, color) in [
            (vec![0; 4], 2, 2, image::ExtendedColorType::L8),
            (vec![1; 4], 2, 2, image::ExtendedColorType::L8),
            (vec![0; 4], 1, 4, image::ExtendedColorType::L8),
            (vec![0; 12], 2, 2, image::ExtendedColorType::Rgb8),
            (vec![0; 8], 2, 2, image::ExtendedColorType::La8),
        ] {
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(&pixels, width, height, color)
                .unwrap();
            let expected = pixels == [0; 4] && width == 2 && color == image::ExtendedColorType::L8;
            assert_eq!(decode_import(&entry(), &png).is_ok(), expected);
        }
    }

    #[test]
    fn rejects_unsafe_ids_duplicate_entries_and_invalid_provenance() {
        for id in ["test", "", "../scan", "scan/file", "A", "a.png"] {
            let mut item = entry();
            item.id = id.into();
            assert!(validate_manifest(&Manifest { images: vec![item] }).is_err());
        }
        assert!(
            validate_manifest(&Manifest {
                images: vec![entry(), entry()]
            })
            .is_err()
        );
        let mut item = entry();
        item.source_sha256 = "x".repeat(64);
        assert!(validate_manifest(&Manifest { images: vec![item] }).is_err());
    }

    #[test]
    fn absent_optional_imports_keep_the_embedded_default_available() {
        // The deliberately absent parent is not created by this test.
        let missing =
            std::env::temp_dir().join(format!("gnorix-missing-gallery-{}", std::process::id()));
        let gallery = Gallery::load_manifest(
            &missing,
            Manifest {
                images: vec![entry()],
            },
        )
        .unwrap();
        assert_eq!(gallery.views().len(), 1);
        assert_eq!(gallery.default_image().view.id, "test");
        assert!(gallery.get("scan-1").is_none());
    }
}

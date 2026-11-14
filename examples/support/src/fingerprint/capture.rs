use gnorix_core::biometrics::Capture;
use image::GrayImage;

use super::Circle;
use crate::Result;

pub struct SelectedCapture {
    pub capture: Capture,
    pub offset: [f64; 2],
}

pub fn select(source: &GrayImage, circle: Option<Circle>) -> Result<SelectedCapture> {
    let Some(circle) = circle else {
        return Ok(SelectedCapture {
            capture: Capture::new(
                source.width(),
                source.height(),
                source.as_raw().clone(),
                None,
                None,
            )?,
            offset: [0.0, 0.0],
        });
    };
    validate_circle(source, circle)?;
    let left = (circle.x - circle.radius).floor().max(0.0) as u32;
    let top = (circle.y - circle.radius).floor().max(0.0) as u32;
    let right = ((circle.x + circle.radius).floor() + 1.0).min(f64::from(source.width())) as u32;
    let bottom = ((circle.y + circle.radius).floor() + 1.0).min(f64::from(source.height())) as u32;
    let width = right - left;
    let height = bottom - top;
    let mut pixels = Vec::with_capacity((width as usize) * (height as usize));
    let mut mask = Vec::with_capacity(pixels.capacity());
    for y in top..bottom {
        for x in left..right {
            pixels.push(source.get_pixel(x, y).0[0]);
            mask.push((f64::from(x) - circle.x).hypot(f64::from(y) - circle.y) <= circle.radius);
        }
    }
    if !mask.iter().any(|&usable| usable) {
        return Err("selected circle contains no observed pixels".into());
    }
    Ok(SelectedCapture {
        capture: Capture::new(width, height, pixels, Some(mask), None)?,
        offset: [f64::from(left), f64::from(top)],
    })
}

fn validate_circle(source: &GrayImage, circle: Circle) -> Result<()> {
    if !circle.x.is_finite()
        || !circle.y.is_finite()
        || !circle.radius.is_finite()
        || circle.radius < 8.0
        || circle.radius > f64::from(source.width().max(source.height())) * 2.0
        || !(0.0..f64::from(source.width())).contains(&circle.x)
        || !(0.0..f64::from(source.height())).contains(&circle.y)
    {
        return Err(
            "circle must have a finite center inside the image and a radius of at least 8 px"
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    #[test]
    fn circle_keeps_original_pixels_and_masks_only_outside() {
        let source = GrayImage::from_fn(40, 40, |x, y| Luma([(x + y) as u8]));
        let selected = select(
            &source,
            Some(Circle {
                x: 20.0,
                y: 20.0,
                radius: 10.0,
            }),
        )
        .unwrap();
        assert_eq!(selected.offset, [10.0, 10.0]);
        assert_eq!(
            (selected.capture.width(), selected.capture.height()),
            (21, 21)
        );
        assert_eq!(selected.capture.pixels()[0], 20);
        assert!(!selected.capture.valid_mask().unwrap()[0]);
        let mask = selected.capture.valid_mask().unwrap();
        for (x, y) in [(0, 10), (20, 10), (10, 0), (10, 20)] {
            assert!(mask[y * 21 + x], "tangent at ({x}, {y}) must be included");
        }
        assert!(!mask[5 * 21 + 1]);
        assert!(mask[10 * 21 + 10]);
        assert_eq!(selected.capture.pixels()[10 * 21 + 10], 40);
    }

    #[test]
    fn clipped_circle_keeps_global_offset_and_rejects_invalid_geometry() {
        let source = GrayImage::new(40, 30);
        let selected = select(
            &source,
            Some(Circle {
                x: 2.0,
                y: 3.0,
                radius: 8.0,
            }),
        )
        .unwrap();
        assert_eq!(selected.offset, [0.0, 0.0]);
        assert_eq!(
            (selected.capture.width(), selected.capture.height()),
            (11, 12)
        );
        assert!(
            select(
                &source,
                Some(Circle {
                    x: -1.0,
                    y: 2.0,
                    radius: 10.0
                })
            )
            .is_err()
        );
        assert!(
            select(
                &source,
                Some(Circle {
                    x: 2.0,
                    y: 2.0,
                    radius: f64::NAN
                })
            )
            .is_err()
        );
    }
}

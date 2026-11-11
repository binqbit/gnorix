use std::{error::Error, fmt};

use zeroize::Zeroizing;

/// An owned, single-channel 8-bit capture supplied by an acquisition adapter.
///
/// Pixels and an optional validity mask use row-major order, starting at the top
/// left. A `true` mask entry marks an observed, usable pixel; the mask does not
/// prescribe a quality threshold. Omitting it means no mask was supplied, not
/// that a quality assessment has accepted every pixel.
///
/// The optional scale is pixels per inch along both image axes. This type does
/// not infer scale, assess quality, fill missing regions, or select a sensor.
/// Pixel values are cleared when the capture is dropped and omitted from debug
/// output. The input format is a prerelease boundary, not a sensor requirement.
pub struct Capture {
    width: u32,
    height: u32,
    pixels: Zeroizing<Vec<u8>>,
    valid_mask: Option<Vec<bool>>,
    pixels_per_inch: Option<f64>,
}

impl Capture {
    /// Checks the image shape and any explicitly supplied mask and scale.
    ///
    /// The owned pixel buffer is also cleared if validation fails. Zeroization
    /// cannot clear copies retained by the caller or the acquisition device.
    pub fn new(
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        valid_mask: Option<Vec<bool>>,
        pixels_per_inch: Option<f64>,
    ) -> Result<Self, CaptureError> {
        let pixels = Zeroizing::new(pixels);
        let expected_length = pixel_count(width, height)?;

        validate_buffer_lengths(expected_length, &pixels, valid_mask.as_deref())?;
        validate_scale(pixels_per_inch)?;

        Ok(Self {
            width,
            height,
            pixels,
            valid_mask,
            pixels_per_inch,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn valid_mask(&self) -> Option<&[bool]> {
        self.valid_mask.as_deref()
    }

    pub fn pixels_per_inch(&self) -> Option<f64> {
        self.pixels_per_inch
    }
}

impl fmt::Debug for Capture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Capture")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("pixels", &"[REDACTED]")
            .field(
                "valid_mask",
                &self.valid_mask.as_ref().map(|_| "[REDACTED]"),
            )
            .field("pixels_per_inch", &self.pixels_per_inch)
            .finish()
    }
}

/// A structural input error, separate from biometric quality or matching.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureError {
    EmptyDimensions,
    DimensionsOverflow,
    PixelCountMismatch { expected: usize, actual: usize },
    MaskCountMismatch { expected: usize, actual: usize },
    InvalidPixelsPerInch,
}

impl fmt::Display for CaptureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDimensions => formatter.write_str("capture dimensions must be nonzero"),
            Self::DimensionsOverflow => {
                formatter.write_str("capture dimensions exceed the addressable pixel count")
            }
            Self::PixelCountMismatch { expected, actual } => write!(
                formatter,
                "capture requires {expected} pixels, but received {actual}"
            ),
            Self::MaskCountMismatch { expected, actual } => write!(
                formatter,
                "capture requires {expected} mask entries, but received {actual}"
            ),
            Self::InvalidPixelsPerInch => {
                formatter.write_str("pixels per inch must be finite and positive")
            }
        }
    }
}

impl Error for CaptureError {}

fn pixel_count(width: u32, height: u32) -> Result<usize, CaptureError> {
    if width == 0 || height == 0 {
        return Err(CaptureError::EmptyDimensions);
    }

    let width = usize::try_from(width).map_err(|_| CaptureError::DimensionsOverflow)?;
    let height = usize::try_from(height).map_err(|_| CaptureError::DimensionsOverflow)?;

    width
        .checked_mul(height)
        .ok_or(CaptureError::DimensionsOverflow)
}

fn validate_buffer_lengths(
    expected: usize,
    pixels: &[u8],
    valid_mask: Option<&[bool]>,
) -> Result<(), CaptureError> {
    if pixels.len() != expected {
        return Err(CaptureError::PixelCountMismatch {
            expected,
            actual: pixels.len(),
        });
    }

    if let Some(mask) = valid_mask
        && mask.len() != expected
    {
        return Err(CaptureError::MaskCountMismatch {
            expected,
            actual: mask.len(),
        });
    }

    Ok(())
}

fn validate_scale(pixels_per_inch: Option<f64>) -> Result<(), CaptureError> {
    if let Some(scale) = pixels_per_inch
        && (!scale.is_finite() || scale <= 0.0)
    {
        return Err(CaptureError::InvalidPixelsPerInch);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_explicit_capture_data() {
        let capture = Capture::new(
            2,
            2,
            vec![10, 20, 30, 40],
            Some(vec![true, false, true, false]),
            Some(500.0),
        )
        .unwrap();

        assert_eq!(capture.width(), 2);
        assert_eq!(capture.height(), 2);
        assert_eq!(capture.pixels(), &[10, 20, 30, 40]);
        assert_eq!(
            capture.valid_mask(),
            Some([true, false, true, false].as_slice())
        );
        assert_eq!(capture.pixels_per_inch(), Some(500.0));
    }

    #[test]
    fn accepts_absent_metadata_without_inventing_values() {
        let capture = Capture::new(1, 1, vec![0], None, None).unwrap();

        assert_eq!(capture.valid_mask(), None);
        assert_eq!(capture.pixels_per_inch(), None);
    }

    #[test]
    fn leaves_capture_quality_to_the_caller() {
        let capture = Capture::new(1, 1, vec![0], Some(vec![false]), None).unwrap();

        assert_eq!(capture.valid_mask(), Some([false].as_slice()));
    }

    #[test]
    fn rejects_zero_dimensions() {
        for (width, height) in [(0, 1), (1, 0), (0, 0)] {
            let result = Capture::new(width, height, vec![], None, None);

            assert_eq!(result.unwrap_err(), CaptureError::EmptyDimensions);
        }
    }

    #[test]
    fn rejects_wrong_pixel_and_mask_lengths() {
        let missing_pixels = Capture::new(2, 2, vec![0; 3], None, None);
        let extra_pixels = Capture::new(2, 2, vec![0; 5], None, None);
        let missing_mask = Capture::new(2, 2, vec![0; 4], Some(vec![true; 3]), None);
        let extra_mask = Capture::new(2, 2, vec![0; 4], Some(vec![true; 5]), None);

        assert_eq!(
            missing_pixels.unwrap_err(),
            CaptureError::PixelCountMismatch {
                expected: 4,
                actual: 3
            }
        );
        assert_eq!(
            extra_pixels.unwrap_err(),
            CaptureError::PixelCountMismatch {
                expected: 4,
                actual: 5
            }
        );
        assert_eq!(
            missing_mask.unwrap_err(),
            CaptureError::MaskCountMismatch {
                expected: 4,
                actual: 3
            }
        );
        assert_eq!(
            extra_mask.unwrap_err(),
            CaptureError::MaskCountMismatch {
                expected: 4,
                actual: 5
            }
        );
    }

    #[test]
    fn rejects_nonpositive_or_nonfinite_scale() {
        for scale in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let result = Capture::new(1, 1, vec![0], None, Some(scale));

            assert_eq!(result.unwrap_err(), CaptureError::InvalidPixelsPerInch);
        }
    }

    #[test]
    fn checks_large_dimensions_without_wrapping() {
        let result = Capture::new(u32::MAX, u32::MAX, vec![], None, None);

        match usize::try_from(u64::from(u32::MAX) * u64::from(u32::MAX)) {
            Ok(expected) => assert_eq!(
                result.unwrap_err(),
                CaptureError::PixelCountMismatch {
                    expected,
                    actual: 0
                }
            ),
            Err(_) => assert_eq!(result.unwrap_err(), CaptureError::DimensionsOverflow),
        }
    }

    #[test]
    fn debug_redacts_pixels_and_mask() {
        let capture = Capture::new(2, 1, vec![237, 153], Some(vec![true, false]), None).unwrap();
        let debug = format!("{capture:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("237"));
        assert!(!debug.contains("153"));
        assert!(!debug.contains("true"));
        assert!(!debug.contains("false"));
    }
}

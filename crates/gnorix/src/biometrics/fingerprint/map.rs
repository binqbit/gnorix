//! Binary ridge pixels and inherited boundary provenance.

use std::fmt;

use super::FingerprintError;

/// A binary ridge image with an explicit usable region.
#[derive(Clone, PartialEq, Eq)]
pub struct RidgeMap {
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) ridges: Vec<bool>,
    pub(super) usable: Vec<bool>,
    pub(super) boundary_origin: Vec<bool>,
}

impl RidgeMap {
    pub fn new(
        width: usize,
        height: usize,
        ridges: Vec<bool>,
        usable: Vec<bool>,
    ) -> Result<Self, FingerprintError> {
        let count =
            width
                .checked_mul(height)
                .filter(|&n| n > 0)
                .ok_or(FingerprintError::InvalidMap(
                    "dimensions must be nonzero and addressable",
                ))?;
        if ridges.len() != count || usable.len() != count {
            return Err(FingerprintError::InvalidMap(
                "pixel buffers do not match dimensions",
            ));
        }
        if ridges
            .iter()
            .zip(&usable)
            .any(|(&ridge, &valid)| ridge && !valid)
        {
            return Err(FingerprintError::InvalidMap("ridge pixels must be usable"));
        }
        let boundary_origin = boundary_contacts(width, height, &ridges, &usable);
        Ok(Self {
            width,
            height,
            ridges,
            usable,
            boundary_origin,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }
    pub fn ridges(&self) -> &[bool] {
        &self.ridges
    }
    pub fn usable(&self) -> &[bool] {
        &self.usable
    }
}

fn boundary_contacts(width: usize, height: usize, ridges: &[bool], usable: &[bool]) -> Vec<bool> {
    (0..ridges.len())
        .map(|index| {
            if !ridges[index] {
                return false;
            }
            let (x, y) = (index % width, index / width);
            x == 0
                || y == 0
                || x + 1 == width
                || y + 1 == height
                || ((y - 1)..=(y + 1))
                    .any(|row| ((x - 1)..=(x + 1)).any(|column| !usable[row * width + column]))
        })
        .collect()
}

impl fmt::Debug for RidgeMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RidgeMap")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("pixels", &"[REDACTED]")
            .finish()
    }
}

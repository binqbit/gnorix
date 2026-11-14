//! Image acquisition and seeded capture defects shared by example applications.
//! These adapters generate test inputs; they do not change core matching policy.

mod artifacts;
mod capture;

use serde::{Deserialize, Serialize};

pub use artifacts::apply as apply_artifacts;
pub use capture::{SelectedCapture, select};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Circle {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
}

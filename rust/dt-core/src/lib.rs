//! Shared values and validated buffers ported from `glowtube.py` and `card_image.py`.
//! This crate has no runtime side effects and no external dependencies.

pub mod constants;
mod error;
mod types;

pub use error::{Error, Result};
pub use types::{AudioFeatureFrame, AudioFeatures, CardTarget, Clock, Frame, TrackInfo};

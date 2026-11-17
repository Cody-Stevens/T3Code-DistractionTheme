//! Pixels and CSS ported from `encoder.py`, `card_image.py`, `theme_css.py` and `glowtube.py`.
//! Interfaces only: codecs and browser behavior still require phase-two implementation/measurement.

pub mod card;
pub mod css;
pub mod encode;

use dt_core::{AudioFeatureFrame, Frame, Result};

/// Port `glowtube.light`: boost HSV saturation/value, preserving hue; return CSS `#rrggbb`.
pub fn light(_rgb: [f32; 3]) -> String {
    todo!("port light")
}

/// Port `glowtube.edge_colors`: mean top fifth, right sixth, bottom fifth, left sixth, then full
/// frame, passed through `light`. Return that exact order for `c0..c3,aa` keyframe values.
pub fn edge_colors(_frame: &Frame) -> Result<[String; 5]> {
    todo!("port edge_colors")
}

/// Port `glowtube.glow_frame`: return a 128x72 RGB24 blur with bass spill, kick flash and ring.
/// Ring input is in screen pixels at 1440 px/s; preserve hues when scaling into 0..255.
/// This runs per frame inside the median <100 ms / worst <200 ms total theme-build budget.
pub fn glow_frame(_frame: &Frame, _features: AudioFeatureFrame) -> Result<Frame> {
    todo!("port glow_frame")
}

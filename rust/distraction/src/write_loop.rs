//! Theme timing from `Player.batch/until_next` and `glowtube.play`.

use crate::player::Player;
use dt_core::{Clock, Result};
use dt_render::css::BuiltTheme;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// Port `Player.build_est/offset/next_k/hold_writes_until` timing state.
#[derive(Clone, Debug)]
pub struct WriteTiming {
    /// Exponentially smoothed build time, initial 150 ms; update .7 old + .3 measured.
    pub build_estimate: Duration,
    /// Signed display offset seconds, initially .16; ctl offset adds a delta.
    pub display_offset: f64,
    /// Next six-frame-grid start, cleared after pause, seek or long quiet write.
    pub next_frame: Option<u64>,
    /// No theme writes before this monotonic instant.
    pub hold_until: Option<Instant>,
}

/// Port `Player.batch` and the main loop; owns renderer state, reader, card cache and theme writer.
pub struct WriteLoop {
    _private: (),
}

impl WriteLoop {
    /// Port `Player.__init__`'s rendering/timing initialization. Clock is audible sample time.
    pub fn new(_clock: Arc<dyn Clock>, _timing: WriteTiming) -> Result<Self> {
        todo!("initialize write loop")
    }

    /// Port `Player.batch`: predict media_time+build_est+offset, snap within two frames to grid,
    /// ask ahead pool within +/-1 frame, build eight frames (one paused, 60 for quiet hold).
    /// Return None if not ready/no frames; caller emits idle theme. CSS <=49,500 chars.
    pub fn batch(&mut self, _player: &mut Player) -> Result<Option<BuiltTheme>> {
        todo!("build theme batch")
    }

    /// Port `Player.until_next`: clamp next grid deadline delay to 0..500 ms (250 ms if paused).
    /// Command wakes may shorten the wait, but actual writes still obey >=150 ms wall-time gaps.
    pub fn until_next(&self) -> Duration {
        todo!("next audio-grid deadline")
    }

    /// Port `glowtube.play`: write complete changed CSS, smooth measured build time, acknowledge
    /// quiet requests after the 2.5 s write, hold 2.2 s and grant publish after .45 s settle.
    /// Median build <100 ms, worst <200 ms; failure logs must not contain secret data.
    pub fn run(&mut self, _player: &mut Player) -> Result<()> {
        todo!("run write loop")
    }
}

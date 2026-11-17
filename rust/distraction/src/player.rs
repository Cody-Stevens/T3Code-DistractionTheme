//! Queue/lazy-loader ownership porting `glowtube.Player`; audio output lives in dt-media.

use crate::control::ControlStatus;
use dt_core::{Error, Result, TrackInfo};
use dt_media::LoadedTrack;
use std::{sync::Arc, time::Duration};

/// Port `Track.loading/failed/pcm` state; only current and next entries retain Ready PCM.
#[derive(Clone, Debug)]
pub enum LoadState {
    /// Metadata queued but PCM not resident; cached video may still exist.
    Unloaded,
    /// The lazy-loader worker is downloading/decoding/analysing this entry.
    Loading,
    /// All PCM and per-frame features are ready for atomic publication to the callback.
    Ready(Arc<LoadedTrack>),
    /// Loading failed; callback advances past failures when another track is available.
    Failed,
}

/// Port `glowtube.Track`'s queue-owned metadata/load state.
#[derive(Clone, Debug)]
pub struct QueueEntry {
    /// Metadata survives PCM eviction so later playback can reuse the downloaded file.
    pub info: TrackInfo,
    /// Residency/failure state, changed by the lazy-loader owner.
    pub state: LoadState,
}

/// Port `Player.add` modes, including playlist insertion order.
#[derive(Clone, Copy, Debug)]
pub enum AddMode {
    /// Insert after current and start the new first entry immediately.
    Now,
    /// Insert after current without interrupting playback.
    Next,
    /// Append to end.
    End,
}

/// Port `glowtube.Player`: queue, selected track, card cache, command wake channel and workers.
/// Single orchestration owner serializes mutations; callback reads shared PCM/clock snapshots only.
pub struct Player {
    _private: (),
}

impl Player {
    /// Port `Player.__init__`: create state, enqueue URLs and start current/next-only lazy loading.
    pub fn new(_urls: Vec<String>) -> Result<Self> {
        todo!("initialize player")
    }

    /// Port `Player.add`: expand a URL asynchronously, insert entries in order, wake the write loop.
    pub fn add(&mut self, _url: String, _mode: AddMode) -> Result<()> {
        todo!("queue URL")
    }

    /// Port `Player.jump`: wrap selected queue index by signed delta and reset sample position.
    pub fn jump(&mut self, _delta: i32) -> Result<()> {
        todo!("select track")
    }

    /// Port `Player.play_next`: move zero-based upcoming offset to immediate-next position.
    pub fn play_next(&mut self, _offset: usize) -> Result<()> {
        todo!("reorder queue")
    }

    /// Port `Player.seek`: relative signed seconds, clamped to decoded sample bounds.
    pub fn seek(&mut self, _seconds: f64) -> Result<()> {
        todo!("relative seek")
    }

    /// Port `Player.seek_to`: clamp a fraction to 0..=1 and seek, invalidate video/ahead/favicon.
    pub fn seek_to(&mut self, _fraction: f64) -> Result<()> {
        todo!("fractional seek")
    }

    /// Port `Player.say`: update card subtitle until a monotonic deadline, default duration 4 s.
    pub fn say(&mut self, _message: String, _duration: Duration) {
        todo!("transient card status")
    }

    /// Port `serve`'s locked JSON snapshot; status reads must not wake rendering or mutate state.
    pub fn status(&self) -> ControlStatus {
        todo!("snapshot player state")
    }

    /// Port `play`'s finally block: stop/join audio, video, loader, input, ahead/favicon and server.
    pub fn stop(&mut self) -> Result<()> {
        todo!("stop player resources")
    }
}

/// Port `glowtube.play`: coordinate player, control/input workers and 250 ms theme write loop.
/// Phase one returns an explicit error before opening devices, binding ports or touching T3 files.
pub fn play(_urls: Vec<String>) -> Result<()> {
    Err(Error::NotImplemented("play"))
}

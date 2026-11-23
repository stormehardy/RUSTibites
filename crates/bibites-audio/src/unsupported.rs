//! The build of the music player for the systems that have none yet: every file is refused with a clear message.

use std::path::Path;

use crate::{AudioError, PlayState};

/// Nothing can be played here.
pub const AVAILABLE: bool = false;

/// A clip that cannot exist (the type is there so the callers compile).
pub struct Clip;

fn refused() -> AudioError {
    AudioError("playing music is only available on Windows for now".to_owned())
}

impl Clip {
    pub fn open(_path: &Path) -> Result<Self, AudioError> {
        Err(refused())
    }

    pub fn play(&mut self) -> Result<(), AudioError> {
        Err(refused())
    }

    pub fn pause(&mut self) -> Result<(), AudioError> {
        Err(refused())
    }

    pub fn resume(&mut self) -> Result<(), AudioError> {
        Err(refused())
    }

    pub fn set_volume(&mut self, _volume: f32) {}

    pub fn state(&self) -> PlayState {
        PlayState::Stopped
    }

    pub fn position_ms(&self) -> Option<u64> {
        None
    }

    pub fn length_ms(&self) -> Option<u64> {
        None
    }
}

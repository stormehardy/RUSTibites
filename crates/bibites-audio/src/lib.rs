//! RUSTibites Improved's music: one file at a time, played by the operating system.
//!
//! [`Clip`] opens a music file and plays, pauses, resumes it and sets its volume; its state says whether it is still playing. On Windows the file goes
//! through MCI (`winmm.dll`), the player every Windows has: it plays mp3 (and wav, wma and m4a/aac) with Windows' own codecs, so the project needs no audio
//! library. Elsewhere [`Clip::open`] says that playing is not available, and [`available`] is false.
//!
//! Everything is safe to call; the only `unsafe` is the handful of `winmm` calls in `mci.rs`. All the calls of a clip are meant to come from one thread
//! (the jukebox's own), which is also how MCI is used best.

#![deny(unsafe_code)]

use std::fmt;
use std::path::Path;

#[cfg(windows)]
mod mci;
#[cfg(windows)]
use mci as backend;

#[cfg(not(windows))]
mod unsupported;
#[cfg(not(windows))]
use unsupported as backend;

/// What went wrong with a music file (the player's own words where it has some).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioError(pub String);

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AudioError {}

/// Whether a clip is playing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    /// Sound is coming out (or the player is about to start it).
    Playing,
    /// Held by [`Clip::pause`].
    Paused,
    /// Not playing: before the first [`Clip::play`], and after the end of the file.
    Stopped,
}

/// Whether this build can play music at all (Windows).
#[must_use]
pub fn available() -> bool {
    backend::AVAILABLE
}

/// The loudness MCI is given for a volume of `0.0` (silent) to `1.0` (the file's own level): its scale is `0..=1000`.
#[must_use]
pub fn mci_volume(volume: f32) -> u32 {
    (volume.clamp(0.0, 1.0) * 1000.0).round() as u32
}

/// The state MCI names with `mode` (`playing`, `paused`, `stopped`, `seeking`, `not ready`, ...).
#[must_use]
pub fn state_of_mode(mode: &str) -> PlayState {
    match mode.trim().to_ascii_lowercase().as_str() {
        "playing" | "seeking" => PlayState::Playing,
        "paused" => PlayState::Paused,
        _ => PlayState::Stopped,
    }
}

/// One open music file.
pub struct Clip {
    inner: backend::Clip,
}

impl Clip {
    /// Opens the music file at `path` (it does not start playing).
    ///
    /// # Errors
    /// The file is missing or of a kind Windows has no decoder for, or this build cannot play music.
    pub fn open(path: &Path) -> Result<Self, AudioError> {
        Ok(Self {
            inner: backend::Clip::open(path)?,
        })
    }

    /// Starts the file from the beginning.
    ///
    /// # Errors
    /// The player refused.
    pub fn play(&mut self) -> Result<(), AudioError> {
        self.inner.play()
    }

    /// Holds the sound where it is.
    ///
    /// # Errors
    /// The player refused.
    pub fn pause(&mut self) -> Result<(), AudioError> {
        self.inner.pause()
    }

    /// Lets a paused clip go on.
    ///
    /// # Errors
    /// The player refused.
    pub fn resume(&mut self) -> Result<(), AudioError> {
        self.inner.resume()
    }

    /// Sets the loudness: `0.0` silent, `1.0` the file's own level.
    pub fn set_volume(&mut self, volume: f32) {
        self.inner.set_volume(volume);
    }

    /// Whether the clip is playing, paused or over.
    #[must_use]
    pub fn state(&self) -> PlayState {
        self.inner.state()
    }

    /// How far into the file the sound is, in milliseconds.
    #[must_use]
    pub fn position_ms(&self) -> Option<u64> {
        self.inner.position_ms()
    }

    /// How long the file is, in milliseconds.
    #[must_use]
    pub fn length_ms(&self) -> Option<u64> {
        self.inner.length_ms()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_volume_is_the_players_zero_to_a_thousand_and_stays_in_it() {
        assert_eq!(mci_volume(0.0), 0);
        assert_eq!(mci_volume(0.25), 250);
        assert_eq!(mci_volume(1.0), 1000);
        assert_eq!(mci_volume(7.0), 1000);
        assert_eq!(mci_volume(-3.0), 0);
        assert_eq!(mci_volume(f32::NAN), 0);
    }

    #[test]
    fn the_modes_the_player_names_map_to_the_three_states() {
        assert_eq!(state_of_mode("playing"), PlayState::Playing);
        assert_eq!(state_of_mode("Playing\r\n"), PlayState::Playing);
        assert_eq!(state_of_mode("seeking"), PlayState::Playing);
        assert_eq!(state_of_mode("paused"), PlayState::Paused);
        assert_eq!(state_of_mode("stopped"), PlayState::Stopped);
        assert_eq!(state_of_mode("not ready"), PlayState::Stopped);
        assert_eq!(state_of_mode(""), PlayState::Stopped);
    }

    /// The real player on a real file, silently (volume 0): `RUSTIBITES_TEST_MP3=<a short mp3> cargo test -p bibites-audio -- --ignored`.
    #[test]
    #[ignore = "needs a music file (RUSTIBITES_TEST_MP3) and a sound device"]
    fn the_system_player_plays_pauses_resumes_and_ends_a_real_file() {
        use std::time::{Duration, Instant};
        let Some(path) = std::env::var_os("RUSTIBITES_TEST_MP3") else {
            return;
        };
        let mut clip = Clip::open(Path::new(&path)).expect("the file opens");
        let length = clip.length_ms().expect("the file has a length");
        assert!(length > 0);
        assert_eq!(clip.state(), PlayState::Stopped, "not started yet");
        clip.set_volume(0.0);
        clip.play().expect("plays");
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(clip.state(), PlayState::Playing);
        assert!(clip.position_ms().is_some_and(|ms| ms > 0));
        clip.pause().expect("pauses");
        assert_eq!(clip.state(), PlayState::Paused);
        let held = clip.position_ms();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(clip.position_ms(), held, "a paused clip stands still");
        clip.resume().expect("resumes");
        assert_eq!(clip.state(), PlayState::Playing);
        let end = Instant::now() + Duration::from_millis(length + 3000);
        while clip.state() != PlayState::Stopped && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(clip.state(), PlayState::Stopped, "the file ran out");
    }

    #[test]
    fn a_file_that_is_not_there_is_an_error_and_not_a_panic() {
        let missing = std::env::temp_dir().join("bibites-audio-no-such-file.mp3");
        let error = Clip::open(&missing).err().expect("nothing to play");
        assert!(!error.0.is_empty());
    }
}

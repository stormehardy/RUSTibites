//! Windows: a music file played through MCI (`mciSendStringW` of `winmm.dll`), the string-command interface of the system's own media player.
//!
//! A file is opened under an alias of its own (`type mpegvideo` is the MCI device that plays mp3, wav, wma and m4a/aac through DirectShow's decoders),
//! and every later call names that alias. This module holds the only `unsafe` of the crate: the two `winmm` functions and nothing else.

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::path::Path;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::{AudioError, PlayState, mci_volume, state_of_mode};

/// Music can be played here.
pub const AVAILABLE: bool = true;

#[link(name = "winmm")]
unsafe extern "system" {
    /// `MCIERROR mciSendStringW(LPCWSTR command, LPWSTR return_string, UINT return_length, HWND callback)`.
    fn mciSendStringW(command: *const u16, return_string: *mut u16, return_length: u32, callback: *mut c_void) -> u32;
    /// `BOOL mciGetErrorStringW(MCIERROR error, LPWSTR buffer, UINT length)`.
    fn mciGetErrorStringW(error: u32, buffer: *mut u16, length: u32) -> i32;
}

/// Sends one MCI command and returns what it answered (most commands answer nothing).
fn send(command: &str) -> Result<String, AudioError> {
    let wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
    let mut answer = [0u16; 256];
    // SAFETY: `wide` is NUL-terminated and outlives the call; `answer` is 256 UTF-16 units and the call is told so; no callback window is given.
    let code = unsafe { mciSendStringW(wide.as_ptr(), answer.as_mut_ptr(), answer.len() as u32, null_mut()) };
    if code != 0 {
        return Err(AudioError(error_text(code)));
    }
    let length = answer.iter().position(|&unit| unit == 0).unwrap_or(answer.len());
    Ok(String::from_utf16_lossy(&answer[..length]))
}

/// The player's own words for an MCI error code.
fn error_text(code: u32) -> String {
    let mut text = [0u16; 256];
    // SAFETY: `text` is 256 UTF-16 units and the call is told so.
    let found = unsafe { mciGetErrorStringW(code, text.as_mut_ptr(), text.len() as u32) };
    let length = text.iter().position(|&unit| unit == 0).unwrap_or(text.len());
    if found == 0 || length == 0 {
        format!("the player's error {code}")
    } else {
        String::from_utf16_lossy(&text[..length])
    }
}

/// The number of clips opened so far: each gets an alias of its own.
static OPENED: AtomicU32 = AtomicU32::new(0);

/// One file open under an alias.
pub struct Clip {
    alias: String,
}

impl Clip {
    pub fn open(path: &Path) -> Result<Self, AudioError> {
        let path = path.to_str().ok_or_else(|| AudioError("the file name is not text".to_owned()))?;
        if path.contains('"') {
            return Err(AudioError("the file name has a quotation mark".to_owned()));
        }
        let alias = format!("bibitesjuke{}", OPENED.fetch_add(1, Ordering::Relaxed));
        send(&format!("open \"{path}\" type mpegvideo alias {alias}"))?;
        let clip = Self { alias };
        // Positions and lengths in milliseconds (the device's own default, said once so nothing depends on it).
        let _ = send(&format!("set {} time format milliseconds", clip.alias));
        Ok(clip)
    }

    pub fn play(&mut self) -> Result<(), AudioError> {
        send(&format!("play {}", self.alias)).map(|_| ())
    }

    pub fn pause(&mut self) -> Result<(), AudioError> {
        send(&format!("pause {}", self.alias)).map(|_| ())
    }

    pub fn resume(&mut self) -> Result<(), AudioError> {
        send(&format!("resume {}", self.alias)).map(|_| ())
    }

    pub fn set_volume(&mut self, volume: f32) {
        let _ = send(&format!("setaudio {} volume to {}", self.alias, mci_volume(volume)));
    }

    pub fn state(&self) -> PlayState {
        send(&format!("status {} mode", self.alias)).map_or(PlayState::Stopped, |mode| state_of_mode(&mode))
    }

    pub fn position_ms(&self) -> Option<u64> {
        send(&format!("status {} position", self.alias)).ok()?.trim().parse().ok()
    }

    pub fn length_ms(&self) -> Option<u64> {
        send(&format!("status {} length", self.alias)).ok()?.trim().parse().ok()
    }
}

impl Drop for Clip {
    fn drop(&mut self) {
        let _ = send(&format!("close {}", self.alias));
    }
}

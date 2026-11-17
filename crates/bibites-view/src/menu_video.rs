//! The title menu's animated background.
//!
//! The game plays `Assets/Videos/showOff.webm` (VP8, 1920x1080, 30 fps, 93.7 s, looping, audio not
//! routed) through a Unity `VideoPlayer` into a `RenderTexture` shown by the title screen's
//! `BackGround/VideoTexture` RawImage (inset 2 px) over a navy base. The scene's `Global Volume`
//! blurs it (a gaussian of about 4.4 px at 1080p, measured against the captured backgrounds) and
//! adds a vignette (corners down to about 0.7, centre about 1.09).
//!
//! The clip lives in the game's `sharedassets1.resource` (offset 0, 13,487,128 bytes: the file
//! as imported; `VideoClip.m_ExternalResources`). It is copied once from the installed game into
//! this port's data folder, and ffmpeg (when installed) decodes it in real time to raw RGBA
//! frames, which a reader thread tints with the vignette. Without the game files or ffmpeg the
//! viewer keeps the static captured frame.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Streamed resource file holding the clip, inside the game's `The Bibites_Data` folder.
const RESOURCE_FILE: &str = "sharedassets1.resource";
/// `VideoClip.m_ExternalResources.m_Size` of `showOff`.
const CLIP_SIZE: usize = 13_487_128;
/// Gaussian blur sigma of the game's post-processing at 1920 px width.
const BLUR_SIGMA_1080: f32 = 4.4;
/// Vignette: `(distance from the centre, 0 centre .. 1 corner; brightness gain)`.
const VIGNETTE: [(f32, f32); 10] = [
    (0.0, 1.09),
    (0.3, 1.085),
    (0.45, 1.047),
    (0.55, 1.023),
    (0.65, 0.983),
    (0.75, 0.903),
    (0.85, 0.846),
    (0.95, 0.715),
    (1.0, 0.68),
    (1.6, 0.4),
];

/// The newest decoded frame, shared with the reader thread.
#[derive(Default)]
struct Shared {
    latest: Mutex<Option<Arc<Vec<u8>>>>,
    counter: AtomicU64,
}

/// A running background decode.
pub struct MenuVideo {
    child: Child,
    shared: Arc<Shared>,
    width: u32,
    height: u32,
}

impl MenuVideo {
    /// Starts decoding the clip at `width` x `height`; `None` when the clip or ffmpeg is missing.
    #[must_use]
    pub fn start(width: u32, height: u32) -> Option<Self> {
        let video = locate_video()?;
        let ffmpeg = find_ffmpeg()?;
        let sigma = BLUR_SIGMA_1080 * width as f32 / 1920.0;
        let mut command = Command::new(ffmpeg);
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-re",
                "-stream_loop",
                "-1",
                "-i",
            ])
            .arg(&video)
            .args(["-an", "-vf"])
            .arg(format!(
                "scale={width}:{height}:flags=bilinear,gblur=sigma={sigma:.2}:steps=2"
            ))
            .args(["-pix_fmt", "rgba", "-f", "rawvideo", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = command.spawn().ok()?;
        let mut stdout = child.stdout.take()?;
        let shared = Arc::new(Shared::default());
        let thread_shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            let frame_len = (width * height * 4) as usize;
            let gain = vignette_gains(width, height);
            loop {
                let mut frame = vec![0_u8; frame_len];
                if stdout.read_exact(&mut frame).is_err() {
                    break;
                }
                apply_vignette(&mut frame, &gain);
                if let Ok(mut latest) = thread_shared.latest.lock() {
                    *latest = Some(Arc::new(frame));
                }
                thread_shared.counter.fetch_add(1, Ordering::Release);
            }
        });
        Some(Self {
            child,
            shared,
            width,
            height,
        })
    }

    /// Frame size in pixels.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The newest frame when it is newer than `seen` (the frame counter and its RGBA pixels).
    #[must_use]
    pub fn poll(&self, seen: u64) -> Option<(u64, Arc<Vec<u8>>)> {
        let counter = self.shared.counter.load(Ordering::Acquire);
        if counter == seen {
            return None;
        }
        let latest = self.shared.latest.lock().ok()?.clone()?;
        Some((counter, latest))
    }

    /// Whether the decoder is still running (ffmpeg could not open the clip otherwise).
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for MenuVideo {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Per-pixel brightness gain of the vignette in 1/256 units.
fn vignette_gains(width: u32, height: u32) -> Vec<u16> {
    let mut gains = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        let ny = (y as f32 + 0.5) / height as f32 * 2.0 - 1.0;
        for x in 0..width {
            let nx = (x as f32 + 0.5) / width as f32 * 2.0 - 1.0;
            let distance = (nx * nx + ny * ny).sqrt() / std::f32::consts::SQRT_2;
            let gain = interpolate(distance);
            gains.push((gain * 256.0).round().clamp(0.0, 1023.0) as u16);
        }
    }
    gains
}

fn interpolate(distance: f32) -> f32 {
    let mut previous = VIGNETTE[0];
    for point in VIGNETTE {
        if distance <= point.0 {
            let span = (point.0 - previous.0).max(1e-6);
            let t = ((distance - previous.0) / span).clamp(0.0, 1.0);
            return previous.1 + (point.1 - previous.1) * t;
        }
        previous = point;
    }
    previous.1
}

fn apply_vignette(frame: &mut [u8], gains: &[u16]) {
    for (pixel, gain) in frame.chunks_exact_mut(4).zip(gains) {
        let gain = u32::from(*gain);
        for channel in &mut pixel[..3] {
            *channel = ((u32::from(*channel) * gain) >> 8).min(255) as u8;
        }
    }
}

/// The clip on disk: an override, the committed asset, the cached copy, or a copy extracted from
/// the installed game (cached for the next start).
fn locate_video() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("RUSTIBITES_MENU_VIDEO").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    let committed = PathBuf::from("assets/game/menu_video/showOff.webm");
    if committed.is_file() {
        return Some(committed);
    }
    let cached = bibites_io::storage::StoragePaths::default_local()
        .root
        .join("menu_video")
        .join("showOff.webm");
    if cached.is_file() {
        return Some(cached);
    }
    let resource = game_install_dirs()
        .into_iter()
        .map(|dir| dir.join("The Bibites_Data").join(RESOURCE_FILE))
        .find(|path| path.is_file())?;
    let mut file = std::fs::File::open(resource).ok()?;
    let mut bytes = vec![0_u8; CLIP_SIZE];
    file.read_exact(&mut bytes).ok()?;
    // EBML header: the file starts with a Matroska / WebM container.
    if bytes[..4] != [0x1A, 0x45, 0xDF, 0xA3] {
        return None;
    }
    std::fs::create_dir_all(cached.parent()?).ok()?;
    std::fs::write(&cached, bytes).ok()?;
    Some(cached)
}

/// Places the game may be installed.
fn game_install_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("RUSTIBITES_GAME_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        dirs.push(PathBuf::from(profile).join("TheBibites"));
    }
    for drive in ['C', 'D', 'E', 'F', 'Z'] {
        dirs.push(PathBuf::from(format!(
            "{drive}:\\Program Files (x86)\\Steam\\steamapps\\common\\The Bibites"
        )));
        dirs.push(PathBuf::from(format!(
            "{drive}:\\SteamLibrary\\steamapps\\common\\The Bibites"
        )));
        dirs.push(PathBuf::from(format!("{drive}:\\Games\\The Bibites")));
    }
    dirs
}

/// An ffmpeg executable: `RUSTIBITES_FFMPEG`, the `PATH`, or the usual Windows install places.
fn find_ffmpeg() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    if let Some(path) = std::env::var_os("RUSTIBITES_FFMPEG").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let mut roots: Vec<PathBuf> = vec![
        PathBuf::from("C:\\ffmpeg\\bin"),
        PathBuf::from("C:\\Program Files\\ffmpeg\\bin"),
    ];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let packages = PathBuf::from(local).join("Microsoft\\WinGet\\Packages");
        if let Ok(read) = std::fs::read_dir(packages) {
            for package in read.flatten() {
                if !package.file_name().to_string_lossy().contains("FFmpeg") {
                    continue;
                }
                if let Ok(inner) = std::fs::read_dir(package.path()) {
                    for build in inner.flatten() {
                        roots.push(build.path().join("bin"));
                    }
                }
            }
        }
    }
    roots
        .into_iter()
        .map(|dir| dir.join(name))
        .find(|path: &PathBuf| Path::new(path).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vignette_is_brightest_in_the_middle_and_dark_in_the_corners() {
        let gains = vignette_gains(64, 36);
        let centre = gains[18 * 64 + 32];
        let corner = gains[0];
        assert!(centre > 256, "centre {centre}");
        assert!(corner < 200, "corner {corner}");
    }

    #[test]
    fn vignette_scales_without_overflow() {
        let mut frame = vec![200_u8, 100, 50, 255];
        apply_vignette(&mut frame, &[300]);
        assert_eq!(frame, vec![234, 117, 58, 255]);
    }
}

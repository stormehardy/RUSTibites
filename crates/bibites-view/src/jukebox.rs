//! RUSTibites Improved: the jukebox. The music files the player puts in the `Music` folder (mp3, and whatever else Windows can play) are played one after the
//! other while the game runs, from a Music page of the Settings screens: switch it on and off, shuffle, the volume, open the folder, skip a track.
//!
//! Nothing of this is in a save or in the simulation: it is a player beside the game. The pieces, bottom up:
//! - [`scan`] lists the music files of the folder, in the order of their names;
//! - [`Playlist`] is the order they play in (the list in order, or shuffled so that every track plays once before any plays again);
//! - [`Engine`] drives one [`Voice`] at a time through a [`Backend`]: starts the next track when one ends, holds the sound while the music is off, skips a file
//!   the player cannot open. The real backend is the system's player (`bibites_audio`); the tests use a made-up one;
//! - the player thread ([`start`]) runs an engine on the folder, and the rest of the game talks to it through [`configure`], [`next`], [`previous`] and
//!   [`status`] without ever waiting for the sound.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use bibites_audio::PlayState;

/// The folder of the music, next to the game's own files (`userdata/Music`).
pub const FOLDER_NAME: &str = "Music";
/// The kinds of file the folder is searched for: mp3 is the one meant; the others play when Windows has the decoder (it does for these).
pub const EXTENSIONS: [&str; 6] = ["mp3", "wav", "wma", "m4a", "aac", "flac"];
/// How often the player thread looks at the folder again (a file put in it joins the music within this).
const RESCAN_EVERY: Duration = Duration::from_secs(3);
/// How often the player thread looks at its track (it sleeps in between, waiting for a command).
const TICK: Duration = Duration::from_millis(250);
/// How long a track that has not been heard playing yet may be "stopped" before it counts as over (the player takes a moment to start).
const START_GRACE: Duration = Duration::from_secs(3);
/// How many tracks in a row may fail to open before the engine gives up for this turn.
const MAX_TRIES: usize = 8;
/// The most tracks "Previous" can walk back through.
const HISTORY: usize = 64;
/// The most characters of a track's name the page's line shows (the line has room for about 55 with the track count after it).
const MAX_TITLE: usize = 38;
/// The most characters of a complaint the page's line shows.
const MAX_COMPLAINT: usize = 54;
/// The file the folder is given, so a player who opens it knows what it is for.
const README: &str = "Put your music here: mp3 files (wav, wma, m4a, aac and flac too).\r\n\
They play one after the other while RUSTibites Improved runs. Files you add while the game is running join the music within a few seconds.\r\n\
Switch the music on and off, shuffle it, set the volume and skip a track on the Music page of the Settings screen.\r\n";

// ---------------------------------------------------------------------------------------------------------------------------------------------------------
// The settings
// ---------------------------------------------------------------------------------------------------------------------------------------------------------

/// What the Music page says.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    /// "Play Music".
    pub enabled: bool,
    /// "Shuffle Music".
    pub shuffle: bool,
    /// "Music Volume" as the slider has it: `0.0` to `1.0`.
    pub volume: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self { enabled: true, shuffle: true, volume: 0.5 }
    }
}

impl Config {
    /// The three settings of the Music page.
    #[must_use]
    pub fn from_settings(settings: &crate::user_settings::UserSettings) -> Self {
        Self {
            enabled: settings.flag(LABEL_PLAY),
            shuffle: settings.flag(LABEL_SHUFFLE),
            volume: settings.number(LABEL_VOLUME).clamp(0.0, 1.0),
        }
    }
}

/// The rows of the Music page that are settings (their labels are what the page's rows are bound by).
pub const LABEL_PLAY: &str = "Play Music";
/// See [`LABEL_PLAY`].
pub const LABEL_SHUFFLE: &str = "Shuffle Music";
/// See [`LABEL_PLAY`].
pub const LABEL_VOLUME: &str = "Music Volume";

/// The loudness a slider place gives: the square of it, so the lower half of the slider is not already loud (the player's own scale is by amplitude).
#[must_use]
pub fn loudness(slider: f32) -> f32 {
    let slider = slider.clamp(0.0, 1.0);
    slider * slider
}

// ---------------------------------------------------------------------------------------------------------------------------------------------------------
// The files
// ---------------------------------------------------------------------------------------------------------------------------------------------------------

/// Whether `path` names a music file by its ending.
#[must_use]
pub fn is_music_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| EXTENSIONS.iter().any(|known| known.eq_ignore_ascii_case(extension)))
}

/// The music files of `folder` and of the folders inside it (down to four levels, so an album can have a folder), in the order of their names, without regard
/// to case. A folder that is not there gives none.
#[must_use]
pub fn scan(folder: &Path) -> Vec<PathBuf> {
    fn walk(folder: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let hidden = entry.file_name().to_string_lossy().starts_with('.');
            if hidden {
                continue;
            }
            if path.is_dir() {
                if depth < 4 {
                    walk(&path, depth + 1, out);
                }
            } else if is_music_file(&path) {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(folder, 0, &mut files);
    files.sort_by_cached_key(|path| sort_key(path));
    files
}

/// What the tracks are ordered by.
fn sort_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// The name a track is shown under: its file name without the ending.
#[must_use]
pub fn title_of(path: &Path) -> String {
    path.file_stem().map_or_else(String::new, |stem| stem.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------------------------------------------------------------------------------------
// The order
// ---------------------------------------------------------------------------------------------------------------------------------------------------------

/// The order the tracks play in.
#[derive(Clone, Debug)]
pub struct Playlist {
    tracks: Vec<PathBuf>,
    current: Option<PathBuf>,
    /// The tracks played before the current one, the last one on top (what "Previous" goes back through).
    history: Vec<PathBuf>,
    /// The tracks a shuffled round has not played yet.
    bag: Vec<PathBuf>,
    rng: u64,
}

impl Playlist {
    /// An empty list; `seed` starts the shuffling.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self { tracks: Vec::new(), current: None, history: Vec::new(), bag: Vec::new(), rng: seed | 1 }
    }

    /// The tracks, in order.
    #[must_use]
    pub fn tracks(&self) -> &[PathBuf] {
        &self.tracks
    }

    /// The track that plays now (or played last); the tests ask.
    #[cfg(test)]
    #[must_use]
    pub fn current(&self) -> Option<&Path> {
        self.current.as_deref()
    }

    /// Replaces the tracks with what the folder holds now: the current track stays the current one (it is still playing when its file went), a new file
    /// joins the round that is on, a file that went leaves it. Returns whether the list changed.
    pub fn set_tracks(&mut self, tracks: Vec<PathBuf>) -> bool {
        if tracks == self.tracks {
            return false;
        }
        let known: HashSet<&PathBuf> = self.tracks.iter().collect();
        let added: Vec<PathBuf> = tracks.iter().filter(|track| !known.contains(track)).cloned().collect();
        let now: HashSet<&PathBuf> = tracks.iter().collect();
        self.bag.retain(|track| now.contains(track));
        self.history.retain(|track| now.contains(track));
        self.bag.extend(added.into_iter().filter(|track| self.current.as_ref() != Some(track)));
        self.tracks = tracks;
        true
    }

    /// The next track: the next of the list, or a random one of those this round has not played (a round plays every track once; the first of the next one
    /// is never the track that just played, while there is another).
    pub fn advance(&mut self, shuffle: bool) -> Option<PathBuf> {
        if self.tracks.is_empty() {
            return None;
        }
        let next = if shuffle { self.draw() } else { self.after_current() };
        if let Some(previous) = self.current.replace(next.clone()) {
            self.history.push(previous);
            if self.history.len() > HISTORY {
                self.history.remove(0);
            }
        }
        self.bag.retain(|track| *track != next);
        Some(next)
    }

    /// The track before: the one played before this (as far back as it is remembered), else the one before it in the list.
    pub fn back(&mut self) -> Option<PathBuf> {
        if self.tracks.is_empty() {
            return None;
        }
        let previous = loop {
            match self.history.pop() {
                Some(track) if self.tracks.contains(&track) => break track,
                Some(_) => {}
                None => break self.before_current(),
            }
        };
        self.current = Some(previous.clone());
        // Going back does not use the round up: the track plays again, so it leaves the bag if it was still in it.
        self.bag.retain(|track| *track != previous);
        Some(previous)
    }

    /// Takes `track` out of the list for good (a file the player could not open).
    pub fn drop_track(&mut self, track: &Path) {
        self.tracks.retain(|candidate| candidate != track);
        self.bag.retain(|candidate| candidate != track);
        self.history.retain(|candidate| candidate != track);
    }

    fn draw(&mut self) -> PathBuf {
        if self.bag.is_empty() {
            self.bag = self.tracks.clone();
        }
        let mut at = (self.random() % self.bag.len() as u64) as usize;
        // The track that just played does not play again at once, while there is another one to play.
        if self.bag.len() > 1 && self.current.as_ref() == Some(&self.bag[at]) {
            at = (at + 1) % self.bag.len();
        }
        self.bag.swap_remove(at)
    }

    fn after_current(&self) -> PathBuf {
        let at = self.position_after_current();
        self.tracks[at % self.tracks.len()].clone()
    }

    /// The index of the first track after the current one (the first track when there is no current one, and when it is the last).
    fn position_after_current(&self) -> usize {
        let Some(current) = &self.current else {
            return 0;
        };
        match self.tracks.iter().position(|track| track == current) {
            Some(at) => at + 1,
            // The current file left the list: the first track that sorts after it.
            None => {
                let key = sort_key(current);
                self.tracks.iter().position(|track| sort_key(track) > key).unwrap_or(0)
            }
        }
    }

    fn before_current(&self) -> PathBuf {
        let at = match self.current.as_ref().and_then(|current| self.tracks.iter().position(|track| track == current)) {
            Some(0) | None => self.tracks.len() - 1,
            Some(at) => at - 1,
        };
        self.tracks[at].clone()
    }

    /// xorshift64*: the shuffle needs luck, not quality.
    fn random(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------------------------
// The engine
// ---------------------------------------------------------------------------------------------------------------------------------------------------------

/// One track that has been opened.
pub trait Voice {
    /// Starts it.
    fn play(&mut self) -> Result<(), String>;
    /// Holds the sound.
    fn pause(&mut self) -> Result<(), String>;
    /// Lets it go on.
    fn resume(&mut self) -> Result<(), String>;
    /// `0.0` silent to `1.0` the file's own level.
    fn set_volume(&mut self, volume: f32);
    /// Playing, paused or over.
    fn state(&self) -> PlayState;
}

/// Where tracks come from.
pub trait Backend {
    /// Opens the file at `path`.
    fn open(&mut self, path: &Path) -> Result<Box<dyn Voice>, String>;
}

/// The system's player.
pub struct SystemBackend;

struct SystemVoice(bibites_audio::Clip);

impl Voice for SystemVoice {
    fn play(&mut self) -> Result<(), String> {
        self.0.play().map_err(|error| error.to_string())
    }

    fn pause(&mut self) -> Result<(), String> {
        self.0.pause().map_err(|error| error.to_string())
    }

    fn resume(&mut self) -> Result<(), String> {
        self.0.resume().map_err(|error| error.to_string())
    }

    fn set_volume(&mut self, volume: f32) {
        self.0.set_volume(volume);
    }

    fn state(&self) -> PlayState {
        self.0.state()
    }
}

impl Backend for SystemBackend {
    fn open(&mut self, path: &Path) -> Result<Box<dyn Voice>, String> {
        bibites_audio::Clip::open(path).map(|clip| Box::new(SystemVoice(clip)) as Box<dyn Voice>).map_err(|error| error.to_string())
    }
}

/// What the Music page shows of the player.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// The player thread is up (in the game window; a screenshot has none).
    pub running: bool,
    /// "Play Music" is on.
    pub enabled: bool,
    /// How many music files the folder holds.
    pub tracks: usize,
    /// The track that is playing (or held, while the music is off).
    pub playing: Option<String>,
    /// What went wrong last, if something did (a file that would not open).
    pub problem: String,
}

impl Status {
    /// The line of the Music page's "Now Playing" row.
    #[must_use]
    pub fn line(&self) -> String {
        if !self.running {
            return "Plays while the game window is open".to_owned();
        }
        if self.tracks == 0 {
            return if self.problem.is_empty() {
                "No music yet: put mp3 files in the Music folder".to_owned()
            } else {
                shorten(&self.problem, MAX_COMPLAINT)
            };
        }
        let count = if self.tracks == 1 { "1 track".to_owned() } else { format!("{} tracks", self.tracks) };
        match (&self.playing, self.enabled) {
            (_, false) => format!("Off ({count})"),
            (Some(title), true) => format!("{}  ({count})", shorten(title, MAX_TITLE)),
            (None, true) if self.problem.is_empty() => format!("Starting ({count})"),
            (None, true) => shorten(&self.problem, MAX_COMPLAINT),
        }
    }
}

/// `text` cut to `max` characters with "..." at the cut (the page's line has a width, and a file name has none).
fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max.saturating_sub(3)).collect();
    format!("{}...", kept.trim_end())
}

/// Plays the tracks of a [`Playlist`] one after the other through a [`Backend`].
pub struct Engine<B: Backend> {
    backend: B,
    playlist: Playlist,
    config: Config,
    voice: Option<Box<dyn Voice>>,
    /// The file `voice` plays.
    playing: Option<PathBuf>,
    /// When it was started, and whether it has been heard playing since.
    started: Instant,
    heard: bool,
    /// The music was switched off while it played: the voice is held and goes on when it is switched on.
    held: bool,
    /// The files the player could not open (not tried again until they change).
    bad: HashSet<PathBuf>,
    /// The folder's files as the last scan found them (the playlist is these, less the bad ones).
    scanned: Vec<PathBuf>,
    problem: String,
}

impl<B: Backend> Engine<B> {
    /// An engine with no tracks; `seed` starts the shuffling.
    pub fn new(backend: B, config: Config, seed: u64, now: Instant) -> Self {
        Self {
            backend,
            playlist: Playlist::new(seed),
            config,
            voice: None,
            playing: None,
            started: now,
            heard: false,
            held: false,
            bad: HashSet::new(),
            scanned: Vec::new(),
            problem: String::new(),
        }
    }

    /// The Music page's settings changed.
    pub fn set_config(&mut self, config: Config) {
        let volume_changed = (config.volume - self.config.volume).abs() > f32::EPSILON;
        self.config = config;
        if volume_changed && let Some(voice) = self.voice.as_mut() {
            voice.set_volume(loudness(config.volume));
        }
    }

    /// The folder holds `files` now.
    pub fn set_tracks(&mut self, files: Vec<PathBuf>) {
        let gone: Vec<PathBuf> = self.bad.iter().filter(|bad| !files.contains(bad)).cloned().collect();
        for path in gone {
            self.bad.remove(&path);
        }
        if files.is_empty() {
            self.problem.clear();
        }
        self.scanned = files;
        let playable: Vec<PathBuf> = self.scanned.iter().filter(|file| !self.bad.contains(*file)).cloned().collect();
        self.playlist.set_tracks(playable);
    }

    /// The "Next Track" button: the music goes on with the next track (nothing happens while the music is off).
    pub fn next(&mut self, now: Instant) {
        if self.config.enabled {
            self.start_track(now, Direction::Forward);
        }
    }

    /// The "Previous Track" button.
    pub fn previous(&mut self, now: Instant) {
        if self.config.enabled {
            self.start_track(now, Direction::Back);
        }
    }

    /// One look at the track: holds it while the music is off, goes on when it is on, and starts the next track when it is over.
    pub fn tick(&mut self, now: Instant) {
        if !self.config.enabled {
            if !self.held
                && let Some(voice) = self.voice.as_mut()
                && voice.state() == PlayState::Playing
                && voice.pause().is_ok()
            {
                self.held = true;
            }
            return;
        }
        if self.held {
            if let Some(voice) = self.voice.as_mut() {
                let _ = voice.resume();
            }
            self.held = false;
        }
        let Some(state) = self.voice.as_ref().map(|voice| voice.state()) else {
            self.start_track(now, Direction::Forward);
            return;
        };
        match state {
            PlayState::Playing => self.heard = true,
            // Held from outside (the player does not do that by itself): let it go on.
            PlayState::Paused => {
                if let Some(voice) = self.voice.as_mut() {
                    let _ = voice.resume();
                }
            }
            PlayState::Stopped => {
                if self.heard || now.duration_since(self.started) > START_GRACE {
                    self.start_track(now, Direction::Forward);
                }
            }
        }
    }

    /// What the Music page shows.
    #[must_use]
    pub fn status(&self) -> Status {
        Status {
            running: true,
            enabled: self.config.enabled,
            tracks: self.playlist.tracks().len(),
            playing: self.voice.as_ref().and(self.playing.as_deref()).map(title_of),
            problem: self.problem.clone(),
        }
    }

    fn start_track(&mut self, now: Instant, direction: Direction) {
        self.voice = None;
        self.playing = None;
        self.held = false;
        for _ in 0..MAX_TRIES {
            let picked = match direction {
                Direction::Forward => self.playlist.advance(self.config.shuffle),
                Direction::Back => self.playlist.back(),
            };
            // No track left to play (none in the folder, or none that opens): what went wrong stays what the page says.
            let Some(path) = picked else {
                return;
            };
            let opened = self.backend.open(&path).and_then(|mut voice| {
                voice.set_volume(loudness(self.config.volume));
                voice.play()?;
                Ok(voice)
            });
            match opened {
                Ok(voice) => {
                    self.voice = Some(voice);
                    self.playing = Some(path);
                    self.started = now;
                    self.heard = false;
                    self.problem.clear();
                    return;
                }
                Err(error) => {
                    self.problem = format!("Could not play {}: {error}", title_of(&path));
                    self.bad.insert(path.clone());
                    self.playlist.drop_track(&path);
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Direction {
    Forward,
    Back,
}

// ---------------------------------------------------------------------------------------------------------------------------------------------------------
// The player thread
// ---------------------------------------------------------------------------------------------------------------------------------------------------------

enum Command {
    Config(Config),
    Next,
    Previous,
}

/// What the game window and the player thread share.
struct Shared {
    config: Mutex<Config>,
    status: Mutex<Status>,
    sender: Mutex<Option<mpsc::Sender<Command>>>,
    folder: Mutex<Option<PathBuf>>,
}

fn shared() -> &'static Shared {
    static SHARED: OnceLock<Shared> = OnceLock::new();
    SHARED.get_or_init(|| Shared {
        config: Mutex::new(Config::default()),
        status: Mutex::new(Status::default()),
        sender: Mutex::new(None),
        folder: Mutex::new(None),
    })
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn send(command: Command) {
    if let Some(sender) = locked(&shared().sender).as_ref() {
        let _ = sender.send(command);
    }
}

/// The folder of the music for a data folder (`<data>/Music`).
#[must_use]
pub fn folder_in(data: &Path) -> PathBuf {
    data.join(FOLDER_NAME)
}

/// The folder the player thread watches, once it runs.
#[must_use]
pub fn folder() -> Option<PathBuf> {
    locked(&shared().folder).clone()
}

/// Starts the player thread on `folder` (made when it is not there, with a note in it): the music plays from now on. Only the game window calls it, so a
/// screenshot or a test never makes a sound. A second call does nothing.
pub fn start(folder: PathBuf) {
    let state = shared();
    let mut sender = locked(&state.sender);
    if sender.is_some() {
        return;
    }
    if std::fs::create_dir_all(&folder).is_ok() {
        let note = folder.join("Put your music here.txt");
        if !note.exists() && std::fs::read_dir(&folder).is_ok_and(|mut entries| entries.next().is_none()) {
            let _ = std::fs::write(note, README);
        }
    }
    *locked(&state.folder) = Some(folder.clone());
    let (tx, rx) = mpsc::channel();
    let config = *locked(&state.config);
    let spawned = std::thread::Builder::new().name("jukebox".to_owned()).spawn(move || {
        run(&folder, config, &rx, SystemBackend, |status| *locked(&shared().status) = status);
    });
    if spawned.is_ok() {
        *sender = Some(tx);
    }
}

/// The Music page's settings changed (they are kept for the player thread even before it starts).
pub fn configure(config: Config) {
    let state = shared();
    let changed = {
        let mut stored = locked(&state.config);
        let changed = *stored != config;
        *stored = config;
        changed
    };
    if changed {
        send(Command::Config(config));
    }
}

/// The "Next Track" button.
pub fn next() {
    send(Command::Next);
}

/// The "Previous Track" button.
pub fn previous() {
    send(Command::Previous);
}

/// What the player thread is doing (a stopped one, in a screenshot, is not running).
#[must_use]
pub fn status() -> Status {
    locked(&shared().status).clone()
}

/// Opens the music folder in the file manager.
///
/// # Errors
/// The folder is not known yet (the player has not started) or could not be opened.
pub fn open_folder() -> Result<(), String> {
    let folder = folder().ok_or_else(|| "The music folder is made when the game window opens".to_owned())?;
    let _ = std::fs::create_dir_all(&folder);
    // A full path with backslashes (`canonicalize` would give the `\?\` form, which Explorer does not open).
    let folder = std::path::absolute(&folder).unwrap_or(folder);
    bibites_io::storage::open_in_file_manager(&folder).map_err(|error| error.to_string())
}

/// The player thread's loop: commands in, the folder looked at every few seconds, the track looked at every tick, `publish` told what the page should show.
/// Ends when every sender of `commands` is gone.
fn run<B: Backend>(folder: &Path, config: Config, commands: &mpsc::Receiver<Command>, backend: B, mut publish: impl FnMut(Status)) {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0x9E37_79B9_7F4A_7C15, |elapsed| elapsed.as_nanos() as u64);
    let mut engine = Engine::new(backend, config, seed, Instant::now());
    let mut next_scan = Instant::now();
    loop {
        let mut disconnected = false;
        match commands.recv_timeout(TICK) {
            Ok(command) => apply(&mut engine, command),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => disconnected = true,
        }
        while let Ok(command) = commands.try_recv() {
            apply(&mut engine, command);
        }
        let now = Instant::now();
        if now >= next_scan {
            engine.set_tracks(scan(folder));
            next_scan = now + RESCAN_EVERY;
        }
        engine.tick(now);
        publish(engine.status());
        if disconnected {
            return;
        }
    }
}

fn apply<B: Backend>(engine: &mut Engine<B>, command: Command) {
    let now = Instant::now();
    match command {
        Command::Config(config) => engine.set_config(config),
        Command::Next => engine.next(now),
        Command::Previous => engine.previous(now),
    }
}

/// The text help of the rows of the Music page, by label (the game's own rows have theirs from its data).
#[must_use]
pub fn help_of(label: &str) -> Option<(&'static str, &'static str)> {
    let label = label.trim();
    [
        (LABEL_PLAY, "Plays the music files of the Music folder while the game runs. Put mp3 files in the folder (the Open Folder button shows it)."),
        (LABEL_SHUFFLE, "If enabled, the tracks play in a random order, each once before any plays again. If not, they play in the order of their names."),
        (LABEL_VOLUME, "How loud the music is. It only changes the music, not the volume of anything else on the computer."),
    ]
    .into_iter()
    .find(|(name, _)| name.eq_ignore_ascii_case(label))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(|name| PathBuf::from(format!("music/{name}.mp3"))).collect()
    }

    /// A folder of the system's temp folder that goes when the test ends.
    struct TempFolder(PathBuf);

    impl TempFolder {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("rustibites-jukebox-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_scan_finds_the_music_files_in_name_order_without_regard_to_case_and_goes_into_folders() {
        let folder = TempFolder::new("scan");
        for name in ["b song.mp3", "A song.MP3", "c.wav", "notes.txt", "cover.jpg", ".hidden.mp3"] {
            std::fs::write(folder.0.join(name), b"x").unwrap();
        }
        std::fs::create_dir_all(folder.0.join("Album")).unwrap();
        std::fs::write(folder.0.join("Album").join("01 first.mp3"), b"x").unwrap();
        let found: Vec<String> = scan(&folder.0)
            .iter()
            .map(|path| path.strip_prefix(&folder.0).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(found, ["A song.MP3", "Album/01 first.mp3", "b song.mp3", "c.wav"]);
        assert!(scan(&folder.0.join("nothing here")).is_empty());
    }

    #[test]
    fn a_track_is_shown_by_its_file_name_without_the_ending() {
        assert_eq!(title_of(Path::new("x/Some Song.mp3")), "Some Song");
        assert!(is_music_file(Path::new("a.MP3")) && is_music_file(Path::new("a.m4a")));
        assert!(!is_music_file(Path::new("a.txt")) && !is_music_file(Path::new("mp3")));
    }

    #[test]
    fn an_unshuffled_list_plays_in_order_wraps_round_and_goes_back() {
        let mut list = Playlist::new(7);
        list.set_tracks(paths(&["a", "b", "c"]));
        let order: Vec<PathBuf> = (0..4).filter_map(|_| list.advance(false)).collect();
        assert_eq!(order, paths(&["a", "b", "c", "a"]));
        // Back goes through what played, then on through the list.
        assert_eq!(list.back(), Some(paths(&["c"])[0].clone()));
        assert_eq!(list.back(), Some(paths(&["b"])[0].clone()));
        let mut fresh = Playlist::new(7);
        fresh.set_tracks(paths(&["a", "b", "c"]));
        assert_eq!(fresh.advance(false), Some(paths(&["a"])[0].clone()));
        assert_eq!(fresh.back(), Some(paths(&["c"])[0].clone()), "nothing played before the first: the last of the list");
        assert_eq!(Playlist::new(1).advance(false), None, "no tracks, no next");
        assert_eq!(Playlist::new(1).back(), None);
    }

    #[test]
    fn a_shuffled_round_plays_every_track_once_and_never_starts_with_the_one_that_just_played() {
        let mut list = Playlist::new(0xDEAD_BEEF);
        let names = ["a", "b", "c", "d", "e"];
        list.set_tracks(paths(&names));
        let mut last: Option<PathBuf> = None;
        for _ in 0..40 {
            let round: Vec<PathBuf> = (0..names.len()).filter_map(|_| list.advance(true)).collect();
            let mut sorted = round.clone();
            sorted.sort();
            assert_eq!(sorted, paths(&names), "every track once in a round: {round:?}");
            if let Some(last) = last.take() {
                assert_ne!(round[0], last, "a new round does not open with the track that closed the last");
            }
            last = round.last().cloned();
        }
        // One track alone plays again and again.
        let mut single = Playlist::new(3);
        single.set_tracks(paths(&["only"]));
        assert_eq!(single.advance(true), Some(paths(&["only"])[0].clone()));
        assert_eq!(single.advance(true), Some(paths(&["only"])[0].clone()));
    }

    #[test]
    fn a_changed_folder_keeps_the_current_track_and_a_new_file_joins_the_round() {
        let mut list = Playlist::new(11);
        list.set_tracks(paths(&["a", "b", "c"]));
        assert_eq!(list.advance(false), Some(paths(&["a"])[0].clone()));
        assert!(!list.set_tracks(paths(&["a", "b", "c"])), "the same files are no change");
        // The playing file went and one came: the current track is still the current one; the sequence goes on after it by name.
        assert!(list.set_tracks(paths(&["b", "c", "d"])));
        assert_eq!(list.current(), Some(paths(&["a"])[0].as_path()));
        assert_eq!(list.advance(false), Some(paths(&["b"])[0].clone()));
        // A shuffled round that is on takes the new file in.
        let mut shuffled = Playlist::new(5);
        shuffled.set_tracks(paths(&["a", "b"]));
        let first = shuffled.advance(true).unwrap();
        shuffled.set_tracks(paths(&["a", "b", "z"]));
        let mut rest = vec![shuffled.advance(true).unwrap(), shuffled.advance(true).unwrap()];
        rest.sort();
        let mut expected: Vec<PathBuf> = paths(&["a", "b", "z"]).into_iter().filter(|track| *track != first).collect();
        expected.sort();
        assert_eq!(rest, expected);
    }

    #[test]
    fn the_loudness_is_the_square_of_the_slider() {
        assert_eq!(loudness(0.0), 0.0);
        assert_eq!(loudness(1.0), 1.0);
        assert!((loudness(0.5) - 0.25).abs() < 1e-6);
        assert_eq!(loudness(9.0), 1.0);
        assert_eq!(loudness(-1.0), 0.0);
    }

    // --- the engine with a made-up player --------------------------------------------------------------------------------------------------------------

    #[derive(Default)]
    struct World {
        /// What was opened, started, paused and resumed, in order.
        log: RefCell<Vec<String>>,
        /// The state every voice reports (the test plays the song's part).
        state: Cell<Option<PlayState>>,
        /// File names (stems) that cannot be opened.
        broken: RefCell<HashSet<String>>,
        /// The volumes set on voices.
        volumes: RefCell<Vec<f32>>,
    }

    struct FakeBackend(Rc<World>);

    struct FakeVoice(Rc<World>, String, Cell<PlayState>);

    impl Backend for FakeBackend {
        fn open(&mut self, path: &Path) -> Result<Box<dyn Voice>, String> {
            let name = title_of(path);
            if self.0.broken.borrow().contains(&name) {
                self.0.log.borrow_mut().push(format!("refused {name}"));
                return Err("no decoder".to_owned());
            }
            self.0.log.borrow_mut().push(format!("open {name}"));
            Ok(Box::new(FakeVoice(Rc::clone(&self.0), name, Cell::new(PlayState::Stopped))))
        }
    }

    impl Voice for FakeVoice {
        fn play(&mut self) -> Result<(), String> {
            self.0.log.borrow_mut().push(format!("play {}", self.1));
            self.2.set(PlayState::Playing);
            Ok(())
        }

        fn pause(&mut self) -> Result<(), String> {
            self.0.log.borrow_mut().push(format!("pause {}", self.1));
            self.2.set(PlayState::Paused);
            Ok(())
        }

        fn resume(&mut self) -> Result<(), String> {
            self.0.log.borrow_mut().push(format!("resume {}", self.1));
            self.2.set(PlayState::Playing);
            Ok(())
        }

        fn set_volume(&mut self, volume: f32) {
            self.0.volumes.borrow_mut().push(volume);
        }

        fn state(&self) -> PlayState {
            // The test's own state, when it set one, stands in for the song running out.
            self.0.state.get().unwrap_or_else(|| self.2.get())
        }
    }

    fn start_engine(config: Config, names: &[&str]) -> (Engine<FakeBackend>, Rc<World>, Instant) {
        let world = Rc::new(World::default());
        let now = Instant::now();
        let mut engine = Engine::new(FakeBackend(Rc::clone(&world)), config, 9, now);
        engine.set_tracks(paths(names));
        (engine, world, now)
    }

    fn ordered() -> Config {
        Config { enabled: true, shuffle: false, volume: 0.5 }
    }

    fn log(world: &World) -> Vec<String> {
        world.log.borrow().clone()
    }

    #[test]
    fn the_engine_starts_the_first_track_at_the_volume_of_the_slider_and_keeps_it_playing() {
        let (mut engine, world, now) = start_engine(ordered(), &["a", "b"]);
        engine.tick(now);
        assert_eq!(log(&world), ["open a", "play a"]);
        assert_eq!(*world.volumes.borrow(), [0.25], "the slider's half is a quarter of the loudness");
        engine.tick(now + Duration::from_secs(1));
        engine.tick(now + Duration::from_secs(2));
        assert_eq!(log(&world).len(), 2, "a track that is playing is left alone");
        let status = engine.status();
        assert_eq!((status.tracks, status.playing.as_deref(), status.enabled), (2, Some("a"), true));
        assert_eq!(status.line(), "a  (2 tracks)");
    }

    #[test]
    fn the_next_track_starts_when_one_is_over_and_not_before() {
        let (mut engine, world, now) = start_engine(ordered(), &["a", "b"]);
        engine.tick(now);
        // Just started: the player may still say "stopped" for a moment; that is not the end.
        world.state.set(Some(PlayState::Stopped));
        engine.tick(now + Duration::from_millis(300));
        assert_eq!(log(&world), ["open a", "play a"], "stopped right at the start is not over");
        // Heard playing, then stopped: over.
        world.state.set(Some(PlayState::Playing));
        engine.tick(now + Duration::from_millis(600));
        world.state.set(Some(PlayState::Stopped));
        engine.tick(now + Duration::from_secs(1));
        assert_eq!(log(&world)[2..], ["open b", "play b"]);
        // A track that was never heard playing is over once the grace is out, and not before.
        world.state.set(Some(PlayState::Stopped));
        engine.tick(now + Duration::from_secs(2));
        assert_eq!(log(&world).len(), 4, "still within the grace");
        engine.tick(now + Duration::from_secs(5));
        assert_eq!(log(&world)[4..], ["open a", "play a"], "the list went round");
    }

    #[test]
    fn switching_the_music_off_holds_the_track_and_switching_it_on_goes_on_with_it() {
        let (mut engine, world, now) = start_engine(ordered(), &["a", "b"]);
        engine.tick(now);
        engine.set_config(Config { enabled: false, ..ordered() });
        engine.tick(now + Duration::from_secs(1));
        engine.tick(now + Duration::from_secs(2));
        assert_eq!(log(&world), ["open a", "play a", "pause a"], "held once");
        assert_eq!(engine.status().line(), "Off (2 tracks)");
        engine.set_config(ordered());
        engine.tick(now + Duration::from_secs(3));
        assert_eq!(log(&world), ["open a", "play a", "pause a", "resume a"]);
        // Off from the start: nothing opens.
        let (mut quiet, quiet_world, start) = start_engine(Config { enabled: false, ..ordered() }, &["a"]);
        quiet.tick(start);
        quiet.next(start);
        assert!(log(&quiet_world).is_empty());
    }

    #[test]
    fn next_and_previous_move_through_the_tracks() {
        let (mut engine, world, now) = start_engine(ordered(), &["a", "b", "c"]);
        engine.tick(now);
        engine.next(now);
        engine.next(now);
        engine.previous(now);
        let opened: Vec<String> = log(&world).into_iter().filter(|line| line.starts_with("open")).collect();
        assert_eq!(opened, ["open a", "open b", "open c", "open b"]);
    }

    #[test]
    fn a_file_that_will_not_open_is_skipped_and_named_and_not_tried_again() {
        let (mut engine, world, now) = start_engine(ordered(), &["a", "b", "c"]);
        world.broken.borrow_mut().insert("a".to_owned());
        engine.tick(now);
        assert_eq!(log(&world), ["refused a", "open b", "play b"]);
        assert_eq!(engine.status().tracks, 2, "the broken file is out of the list");
        // A scan that finds the same files again does not bring it back.
        engine.set_tracks(paths(&["a", "b", "c"]));
        assert_eq!(engine.status().tracks, 2);
        // Only broken files: the engine says what happened and stays quiet.
        let (mut hopeless, hopeless_world, start) = start_engine(ordered(), &["x", "y"]);
        hopeless_world.broken.borrow_mut().extend(["x".to_owned(), "y".to_owned()]);
        hopeless.tick(start);
        hopeless.tick(start + Duration::from_secs(1));
        assert_eq!(log(&hopeless_world), ["refused x", "refused y"], "each file is tried once");
        assert!(hopeless.status().line().starts_with("Could not play y: no decoder"), "{}", hopeless.status().line());
        // The files going from the folder clears the complaint.
        hopeless.set_tracks(Vec::new());
        assert_eq!(hopeless.status().line(), "No music yet: put mp3 files in the Music folder");
    }

    #[test]
    fn a_volume_change_reaches_the_track_that_is_playing() {
        let (mut engine, world, now) = start_engine(ordered(), &["a"]);
        engine.tick(now);
        engine.set_config(Config { volume: 1.0, ..ordered() });
        assert_eq!(*world.volumes.borrow(), [0.25, 1.0]);
        engine.set_config(Config { volume: 1.0, shuffle: true, ..ordered() });
        assert_eq!(world.volumes.borrow().len(), 2, "no change of volume, no call");
    }

    #[test]
    fn the_status_line_says_what_the_page_needs_to() {
        let idle = Status::default();
        assert_eq!(idle.line(), "Plays while the game window is open");
        let empty = Status { running: true, enabled: true, ..Status::default() };
        assert_eq!(empty.line(), "No music yet: put mp3 files in the Music folder");
        let starting = Status { tracks: 1, ..empty.clone() };
        assert_eq!(starting.line(), "Starting (1 track)");
        let playing = Status { tracks: 12, playing: Some("Song".to_owned()), ..empty.clone() };
        assert_eq!(playing.line(), "Song  (12 tracks)");
        let off = Status { enabled: false, ..playing };
        assert_eq!(off.line(), "Off (12 tracks)");
        // A long file name is cut to fit the line; a short one is left alone.
        let long = Status { tracks: 2, playing: Some("A Very Long Song Name That Goes On And On And On For Ever".to_owned()), ..empty.clone() };
        assert_eq!(long.line(), "A Very Long Song Name That Goes On...  (2 tracks)");
        assert_eq!(shorten("short", 38), "short");
        assert_eq!(shorten("exactly ten", 11), "exactly ten");
        let complaint = Status { problem: "Could not play ".to_owned() + &"x".repeat(80), ..empty.clone() };
        assert_eq!(complaint.line().chars().count(), MAX_COMPLAINT - 3 + 3);
    }

    #[test]
    fn the_settings_become_the_config_and_every_row_has_its_help() {
        let settings = crate::user_settings::UserSettings::load(None);
        let config = Config::from_settings(&settings);
        assert!(config.enabled && config.shuffle && (config.volume - 0.5).abs() < 1e-6, "{config:?}");
        for label in [LABEL_PLAY, LABEL_SHUFFLE, LABEL_VOLUME] {
            assert!(help_of(label).is_some(), "{label}");
            assert!(crate::user_settings::index_of(label).is_some(), "{label} is a setting");
        }
        assert!(help_of(" play music ").is_some());
        assert!(help_of("Fertility Color").is_none());
    }

    /// A player that plays every file for ever (it holds nothing, so it can go to the thread).
    struct EndlessBackend;

    struct EndlessVoice;

    impl Backend for EndlessBackend {
        fn open(&mut self, _path: &Path) -> Result<Box<dyn Voice>, String> {
            Ok(Box::new(EndlessVoice))
        }
    }

    impl Voice for EndlessVoice {
        fn play(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn pause(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn resume(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn set_volume(&mut self, _volume: f32) {}

        fn state(&self) -> PlayState {
            PlayState::Playing
        }
    }

    /// The whole chain on real files, silently (volume 0): the folder holds `tone one.mp3` (about 4 s) and `tone two.mp3` (about 2 s); run with
    /// `RUSTIBITES_TEST_MUSIC_FOLDER=<that folder> cargo test -p bibites-view --bins jukebox -- --ignored`.
    #[test]
    #[ignore = "needs a folder of two short mp3 files (RUSTIBITES_TEST_MUSIC_FOLDER) and a sound device"]
    fn the_real_player_thread_plays_a_real_folder_one_file_after_the_other_and_holds_it_when_switched_off() {
        let Some(folder) = std::env::var_os("RUSTIBITES_TEST_MUSIC_FOLDER").map(PathBuf::from) else {
            return;
        };
        let (commands, receiver) = mpsc::channel();
        let (statuses, seen) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            run(&folder, Config { enabled: true, shuffle: false, volume: 0.0 }, &receiver, SystemBackend, |status| {
                let _ = statuses.send(status);
            });
        });
        let mut order: Vec<String> = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(25);
        while Instant::now() < deadline && order.len() < 3 {
            if let Ok(status) = seen.recv_timeout(Duration::from_millis(200))
                && let Some(title) = status.playing
                && order.last() != Some(&title)
            {
                order.push(title);
            }
        }
        assert_eq!(order, ["tone one", "tone two", "tone one"], "the files play one after the other and the list goes round");
        // Switched off: the track is held (still named), and no other starts however long it waits.
        commands.send(Command::Config(Config { enabled: false, shuffle: false, volume: 0.0 })).unwrap();
        let held_until = Instant::now() + Duration::from_secs(3);
        let mut last = None;
        while Instant::now() < held_until {
            if let Ok(status) = seen.recv_timeout(Duration::from_millis(200)) {
                last = Some(status);
            }
        }
        let last = last.expect("statuses go on");
        assert!(!last.enabled && last.playing.as_deref() == Some("tone one"), "{last:?}");
        drop(commands);
        thread.join().expect("the thread ends");
    }

    #[test]
    fn the_player_thread_finds_the_files_of_the_folder_plays_them_and_ends_when_the_game_lets_go() {
        let folder = TempFolder::new("thread");
        std::fs::write(folder.0.join("first song.mp3"), b"x").unwrap();
        let (commands, receiver) = mpsc::channel();
        let (statuses, seen) = mpsc::channel();
        let path = folder.0.clone();
        let thread = std::thread::spawn(move || {
            run(&path, Config { enabled: true, shuffle: false, volume: 0.5 }, &receiver, EndlessBackend, |status| {
                let _ = statuses.send(status);
            });
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut playing = None;
        while Instant::now() < deadline {
            if let Ok(status) = seen.recv_timeout(Duration::from_millis(200))
                && status.playing.is_some()
            {
                playing = Some(status);
                break;
            }
        }
        let status = playing.expect("the thread started the only file of the folder");
        assert_eq!((status.running, status.tracks, status.playing.as_deref()), (true, 1, Some("first song")));
        // Commands go through, and letting go of the thread's channel ends it.
        commands.send(Command::Next).unwrap();
        commands.send(Command::Config(Config { enabled: false, shuffle: false, volume: 0.1 })).unwrap();
        drop(commands);
        thread.join().expect("the thread ends");
    }
}

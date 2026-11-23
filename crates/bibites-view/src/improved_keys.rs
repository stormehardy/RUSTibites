//! RUSTibites Improved: the keys of its own hotkeys, which the player can change. The game's hotkeys are fixed (its Hotkeys page says so); the ones this edition adds
//! are on the Extra Hotkeys page of the Settings screens, where each has a button: a click on it, then the key to use instead (Escape cancels).
//!
//! The keys are kept in a small text file, `improved_keys.txt`, in the data folder (`userdata`), one `action=key` line each. A key the game already uses (the camera
//! keys, the selection keys, the function keys it has a job for, ...) cannot be taken, and the page says what it does there; a file with such a key in it (written by
//! hand) is read as if the line were not there.
//!
//! [`Bindings`] is the whole logic (a plain value, tested on its own); the process-wide copy and the window's input reads are at the bottom of the file.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use macroquad::prelude::KeyCode;

/// The file of the keys.
pub const FILE_NAME: &str = "improved_keys.txt";
/// How long a line of news (a key taken, a key bound) stays on the page.
const NOTE_FOR: Duration = Duration::from_secs(4);
/// What the key's button says while it waits for a key.
pub const PRESS_A_KEY: &str = "Press a key...";

/// The hotkeys this edition adds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Hide (and show again) the world's objects: bibites, eggs, pellets, towers, particles. The interface stays.
    HideObjects,
}

impl Action {
    /// Every action, in the order of the page.
    pub const ALL: [Self; 1] = [Self::HideObjects];

    /// The word the file uses for the action.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::HideObjects => "hide_objects",
        }
    }

    /// What the page calls it.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::HideObjects => "Hide Objects",
        }
    }

    /// The key it has until the player changes it.
    #[must_use]
    pub fn default_key(self) -> KeyCode {
        match self {
            Self::HideObjects => KeyCode::V,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::HideObjects => 0,
        }
    }
}

/// The keys a hotkey can be: letters, digits, the function keys, Tab, the punctuation keys and the keypad's digits (the arrows, Space, Enter, Escape, the editing keys and the
/// modifiers are not for it: they are what the interface itself runs on). The name is what the page and the file show.
const KEYS: &[(KeyCode, &str)] = &[
    (KeyCode::A, "A"),
    (KeyCode::B, "B"),
    (KeyCode::C, "C"),
    (KeyCode::D, "D"),
    (KeyCode::E, "E"),
    (KeyCode::F, "F"),
    (KeyCode::G, "G"),
    (KeyCode::H, "H"),
    (KeyCode::I, "I"),
    (KeyCode::J, "J"),
    (KeyCode::K, "K"),
    (KeyCode::L, "L"),
    (KeyCode::M, "M"),
    (KeyCode::N, "N"),
    (KeyCode::O, "O"),
    (KeyCode::P, "P"),
    (KeyCode::Q, "Q"),
    (KeyCode::R, "R"),
    (KeyCode::S, "S"),
    (KeyCode::T, "T"),
    (KeyCode::U, "U"),
    (KeyCode::V, "V"),
    (KeyCode::W, "W"),
    (KeyCode::X, "X"),
    (KeyCode::Y, "Y"),
    (KeyCode::Z, "Z"),
    (KeyCode::Key0, "0"),
    (KeyCode::Key1, "1"),
    (KeyCode::Key2, "2"),
    (KeyCode::Key3, "3"),
    (KeyCode::Key4, "4"),
    (KeyCode::Key5, "5"),
    (KeyCode::Key6, "6"),
    (KeyCode::Key7, "7"),
    (KeyCode::Key8, "8"),
    (KeyCode::Key9, "9"),
    (KeyCode::F1, "F1"),
    (KeyCode::F2, "F2"),
    (KeyCode::F3, "F3"),
    (KeyCode::F4, "F4"),
    (KeyCode::F5, "F5"),
    (KeyCode::F6, "F6"),
    (KeyCode::F7, "F7"),
    (KeyCode::F8, "F8"),
    (KeyCode::F9, "F9"),
    (KeyCode::F10, "F10"),
    (KeyCode::F11, "F11"),
    (KeyCode::F12, "F12"),
    (KeyCode::Tab, "Tab"),
    (KeyCode::GraveAccent, "`"),
    (KeyCode::Comma, ","),
    (KeyCode::Period, "."),
    (KeyCode::Slash, "/"),
    (KeyCode::Semicolon, ";"),
    (KeyCode::Apostrophe, "'"),
    (KeyCode::LeftBracket, "["),
    (KeyCode::RightBracket, "]"),
    (KeyCode::Backslash, "\\"),
    (KeyCode::Kp0, "Num 0"),
    (KeyCode::Kp1, "Num 1"),
    (KeyCode::Kp2, "Num 2"),
    (KeyCode::Kp3, "Num 3"),
    (KeyCode::Kp4, "Num 4"),
    (KeyCode::Kp5, "Num 5"),
    (KeyCode::Kp6, "Num 6"),
    (KeyCode::Kp7, "Num 7"),
    (KeyCode::Kp8, "Num 8"),
    (KeyCode::Kp9, "Num 9"),
    (KeyCode::KpMultiply, "Num *"),
    (KeyCode::KpDivide, "Num /"),
];

/// What a key of [`KEYS`] is called.
#[must_use]
pub fn name_of(key: KeyCode) -> Option<&'static str> {
    KEYS.iter().find(|(code, _)| *code == key).map(|(_, name)| *name)
}

/// The key a name (as [`name_of`] gives it, in any case) stands for.
#[must_use]
pub fn key_named(name: &str) -> Option<KeyCode> {
    let name = name.trim();
    KEYS.iter().find(|(_, candidate)| candidate.eq_ignore_ascii_case(name)).map(|(code, _)| *code)
}

/// What the game does with a key, when it does something (finishing "it ..."): such a key is not for a hotkey of this edition. The list is the game's own
/// (`UserControl`, `CameraManager`, `GUIManager`, `BrainPanelCommon`) and the port's screenshot key.
#[must_use]
pub fn taken_by(key: KeyCode) -> Option<&'static str> {
    Some(match key {
        KeyCode::W | KeyCode::A | KeyCode::S | KeyCode::D => "moves the camera",
        KeyCode::R => "selects a random bibite",
        KeyCode::E => "selects a random egg",
        KeyCode::G => "selects the top generation",
        KeyCode::O => "selects the oldest bibite",
        KeyCode::Z => "selects a random zone",
        KeyCode::K => "starts the autopanner",
        KeyCode::H => "hides the interface",
        KeyCode::Key1 | KeyCode::Key2 | KeyCode::Key3 | KeyCode::Key4 => "opens a bibite panel",
        KeyCode::F1 | KeyCode::F3 | KeyCode::F4 => "sorts the brain diagram",
        KeyCode::F2 => "steps the hidden nodes",
        KeyCode::F5 => "makes the quick save",
        KeyCode::F9 => "loads the quick save",
        KeyCode::F12 => "takes a screenshot",
        _ => return None,
    })
}

/// A key that only changes what another one means: never a hotkey's key, and not an answer to "press a key".
#[must_use]
pub fn is_modifier(key: KeyCode) -> bool {
    matches!(
        key,
        KeyCode::LeftShift
            | KeyCode::RightShift
            | KeyCode::LeftControl
            | KeyCode::RightControl
            | KeyCode::LeftAlt
            | KeyCode::RightAlt
            | KeyCode::LeftSuper
            | KeyCode::RightSuper
    )
}

/// Which of the keys that went down in a frame answers a button that waits for one: Escape when it is among them, else the lowest of those that are not modifiers (so
/// the answer does not depend on the order a set gives them in).
#[must_use]
pub fn answer_of(pressed: impl IntoIterator<Item = KeyCode>) -> Option<KeyCode> {
    let mut best: Option<KeyCode> = None;
    for key in pressed {
        if key == KeyCode::Escape {
            return Some(key);
        }
        if is_modifier(key) {
            continue;
        }
        if best.is_none_or(|current| (key as u16) < (current as u16)) {
            best = Some(key);
        }
    }
    best
}

/// What [`Bindings::offer`] did with what it was offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    /// Nothing: no key was waited for, or the key was not an answer (a modifier), or a click ended the wait.
    Idle,
    /// A key answered (so the key press is the page's and nothing else's this frame). `changed`: a key is bound to an action now (the file is written).
    Taken {
        /// Whether a binding changed.
        changed: bool,
    },
}

/// The keys of the actions, what is waited for, and the line of news.
#[derive(Clone, Debug)]
pub struct Bindings {
    keys: [KeyCode; Action::ALL.len()],
    listening: Option<Action>,
    note: Option<(String, Instant)>,
}

impl Default for Bindings {
    fn default() -> Self {
        Self::new()
    }
}

impl Bindings {
    /// The default keys, nothing waited for.
    #[must_use]
    pub fn new() -> Self {
        Self { keys: Action::ALL.map(Action::default_key), listening: None, note: None }
    }

    /// The key of `action`.
    #[must_use]
    pub fn key(&self, action: Action) -> KeyCode {
        self.keys[action.index()]
    }

    /// The action whose key is waited for.
    #[must_use]
    pub fn listening(&self) -> Option<Action> {
        self.listening
    }

    /// The next key pressed becomes the key of `action`.
    pub fn listen(&mut self, action: Action) {
        self.listening = Some(action);
        self.note = None;
    }

    /// Stops waiting for a key.
    pub fn cancel(&mut self) {
        self.listening = None;
    }

    /// Every action has its default key again.
    pub fn reset(&mut self, now: Instant) {
        self.keys = Action::ALL.map(Action::default_key);
        self.listening = None;
        self.note = Some(("The default keys are back".to_owned(), now));
    }

    /// One frame's offer to the wait: `key` is the one that went down (see [`answer_of`]), `clicked` that the left button went down. A click ends the wait (the click
    /// itself is the interface's); Escape ends it too; a key the game has a job for is refused (the wait goes on); any other key of [`KEYS`] is the action's now.
    pub fn offer(&mut self, key: Option<KeyCode>, clicked: bool, now: Instant) -> Offer {
        let Some(action) = self.listening else {
            return Offer::Idle;
        };
        if clicked {
            self.listening = None;
            return Offer::Idle;
        }
        let Some(key) = key else {
            return Offer::Idle;
        };
        if is_modifier(key) {
            return Offer::Idle;
        }
        if key == KeyCode::Escape {
            self.listening = None;
            return Offer::Taken { changed: false };
        }
        let Some(name) = name_of(key) else {
            self.note = Some(("That key cannot be used".to_owned(), now));
            return Offer::Taken { changed: false };
        };
        if let Some(job) = taken_by(key) {
            self.note = Some((format!("{name} is taken: it {job}"), now));
            return Offer::Taken { changed: false };
        }
        self.keys[action.index()] = key;
        self.listening = None;
        self.note = Some((format!("{} is now {name}", action.label()), now));
        Offer::Taken { changed: true }
    }

    /// What the button of `action` says: its key, or that a key is waited for.
    #[must_use]
    pub fn button_text(&self, action: Action) -> String {
        if self.listening == Some(action) {
            PRESS_A_KEY.to_owned()
        } else {
            name_of(self.key(action)).unwrap_or("?").to_owned()
        }
    }

    /// The line of the page that says what to do or what just happened.
    #[must_use]
    pub fn state_line(&self, now: Instant) -> String {
        if let Some((text, at)) = &self.note
            && now.saturating_duration_since(*at) < NOTE_FOR
        {
            return text.clone();
        }
        if self.listening.is_some() {
            "Press the new key (Esc cancels)".to_owned()
        } else {
            "Click the key, then press another one".to_owned()
        }
    }

    /// The text of a file: `action=key` lines under a note.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut text = String::from(
            "# The keys of RUSTibites Improved's extra hotkeys. Change them on the Extra Hotkeys page of the Settings screens (or here: one action=key line each).\r\n",
        );
        for action in Action::ALL {
            text.push_str(&format!("{}={}\r\n", action.id(), name_of(self.key(action)).unwrap_or("?")));
        }
        text
    }

    /// The keys a file's text says; what it does not say, or says with a key that cannot be used, stays the default.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        let mut bindings = Self::new();
        for line in text.lines() {
            let line = line.trim_start_matches('\u{feff}').trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((id, name)) = line.split_once('=') else {
                continue;
            };
            let (Some(action), Some(key)) = (Action::ALL.into_iter().find(|action| action.id().eq_ignore_ascii_case(id.trim())), key_named(name)) else {
                continue;
            };
            if taken_by(key).is_none() {
                bindings.keys[action.index()] = key;
            }
        }
        bindings
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------------------------
// The process-wide keys and the window
// ---------------------------------------------------------------------------------------------------------------------------------------------------------

struct Shared {
    bindings: Bindings,
    path: Option<PathBuf>,
}

fn shared() -> &'static Mutex<Shared> {
    static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();
    SHARED.get_or_init(|| Mutex::new(Shared { bindings: Bindings::new(), path: None }))
}

fn locked() -> std::sync::MutexGuard<'static, Shared> {
    shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The file of a data folder.
#[must_use]
pub fn file_in(data: &Path) -> PathBuf {
    data.join(FILE_NAME)
}

/// Reads the keys of the data folder's file (a missing file: the defaults) and keeps the file for the changes to come. Only the game window calls it, so a screenshot
/// or a test never reads or writes the player's file.
pub fn load(data: &Path) {
    let path = file_in(data);
    let bindings = std::fs::read_to_string(&path).map(|text| Bindings::from_text(&text)).unwrap_or_default();
    let mut shared = locked();
    shared.bindings = bindings;
    shared.path = Some(path);
}

fn save(shared: &mut Shared, now: Instant) {
    let Some(path) = shared.path.clone() else {
        return;
    };
    if let Some(folder) = path.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    if std::fs::write(&path, shared.bindings.to_text()).is_err() {
        shared.bindings.note = Some(("The keys file could not be written".to_owned(), now));
    }
}

/// The key of `action` now (the window asks [`pressed`]; this is for the tests).
#[cfg(test)]
#[must_use]
pub fn key(action: Action) -> KeyCode {
    locked().bindings.key(action)
}

/// A click on the action's button: the next key pressed is its key.
pub fn start_listening(action: Action) {
    locked().bindings.listen(action);
}

/// The wait for a key ends (the player went to another page).
pub fn cancel_listening() {
    locked().bindings.cancel();
}

/// Told once a frame by the window loop whether the Extra Hotkeys page is on the screen: a wait for a key does not go on once it is not (another page, or the
/// Settings screen closed).
pub fn page_shown(shown: bool) {
    if !shown {
        cancel_listening();
    }
}

/// The "Reset" button: every action has its default key.
pub fn reset() {
    let now = Instant::now();
    let mut shared = locked();
    shared.bindings.reset(now);
    save(&mut shared, now);
}

/// What the button of `action` says now.
#[must_use]
pub fn button_text(action: Action) -> String {
    locked().bindings.button_text(action)
}

/// The page's line of news now.
#[must_use]
pub fn state_line() -> String {
    locked().bindings.state_line(Instant::now())
}

/// What the page says, as one string: to see whether it changed (and is to be painted again).
#[must_use]
pub fn line() -> String {
    let shared = locked();
    format!("{}|{}", shared.bindings.button_text(Action::HideObjects), shared.bindings.state_line(Instant::now()))
}

/// Offers a frame's key (see [`answer_of`]) and click to the wait for a key (see [`Bindings::offer`]); a key that is bound is written to the file. Returns whether a key
/// was taken.
pub fn offer(key: Option<KeyCode>, clicked: bool) -> bool {
    let now = Instant::now();
    let mut shared = locked();
    match shared.bindings.offer(key, clicked, now) {
        Offer::Idle => false,
        Offer::Taken { changed } => {
            if changed {
                save(&mut shared, now);
            }
            true
        }
    }
}

/// Once a frame, before anything reads the keys: while a key is waited for, the window's key presses are the page's. Returns whether a key was taken (the frame's
/// other readers of the keyboard, Escape included, then see none of it).
pub fn frame() -> bool {
    use macroquad::prelude::{MouseButton, get_keys_pressed, is_mouse_button_pressed};
    if locked().bindings.listening().is_none() {
        return false;
    }
    offer(answer_of(get_keys_pressed()), is_mouse_button_pressed(MouseButton::Left))
}

/// The tests of the process-wide keys take this first (they share one copy, and the tests of a binary run side by side).
#[cfg(test)]
pub(crate) static TEST_GLOBAL: Mutex<()> = Mutex::new(());

/// A test's own keys in the process-wide copy, with no file.
#[cfg(test)]
pub(crate) fn replace_for_test(bindings: Bindings) {
    let mut shared = locked();
    shared.bindings = bindings;
    shared.path = None;
}

/// Whether the key of `action` went down this frame (not while it is being changed, and not with Control or Alt held: Ctrl + V is the paste of a tag, and so on).
#[must_use]
pub fn pressed(action: Action) -> bool {
    use macroquad::prelude::{is_key_down, is_key_pressed};
    let (key, waiting) = {
        let shared = locked();
        (shared.bindings.key(action), shared.bindings.listening().is_some())
    };
    !waiting
        && is_key_pressed(key)
        && !is_key_down(KeyCode::LeftControl)
        && !is_key_down(KeyCode::RightControl)
        && !is_key_down(KeyCode::LeftAlt)
        && !is_key_down(KeyCode::RightAlt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> Instant {
        Instant::now()
    }

    #[test]
    fn the_hide_objects_key_is_v_until_it_is_changed() {
        let bindings = Bindings::new();
        assert_eq!(bindings.key(Action::HideObjects), KeyCode::V);
        assert_eq!(bindings.button_text(Action::HideObjects), "V");
        assert!(taken_by(KeyCode::V).is_none(), "the default must be a free key");
        assert!(name_of(KeyCode::V).is_some(), "and one a hotkey can be");
    }

    #[test]
    fn every_default_key_can_be_chosen_and_is_free() {
        for action in Action::ALL {
            let key = action.default_key();
            assert!(name_of(key).is_some() && taken_by(key).is_none(), "{action:?}");
        }
    }

    #[test]
    fn the_names_are_unique_and_found_in_any_case() {
        for (code, name) in KEYS {
            assert_eq!(key_named(name), Some(*code), "{name}");
            assert_eq!(key_named(&name.to_lowercase()), Some(*code), "{name} in lower case");
        }
        let mut names: Vec<&str> = KEYS.iter().map(|(_, name)| *name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), KEYS.len(), "two keys with one name");
        assert_eq!(key_named("  f10 "), Some(KeyCode::F10));
        assert_eq!(key_named("Space"), None);
    }

    #[test]
    fn a_click_on_the_button_then_a_free_key_binds_it() {
        let mut bindings = Bindings::new();
        bindings.listen(Action::HideObjects);
        assert_eq!(bindings.button_text(Action::HideObjects), PRESS_A_KEY);
        assert_eq!(bindings.offer(Some(KeyCode::F10), false, at()), Offer::Taken { changed: true });
        assert_eq!(bindings.key(Action::HideObjects), KeyCode::F10);
        assert_eq!(bindings.button_text(Action::HideObjects), "F10");
        assert!(bindings.listening().is_none());
        assert_eq!(bindings.state_line(at()), "Hide Objects is now F10");
    }

    #[test]
    fn nothing_is_taken_when_no_key_is_waited_for() {
        let mut bindings = Bindings::new();
        assert_eq!(bindings.offer(Some(KeyCode::F10), false, at()), Offer::Idle);
        assert_eq!(bindings.key(Action::HideObjects), KeyCode::V);
    }

    #[test]
    fn a_key_the_game_uses_is_refused_with_what_it_does_and_the_wait_goes_on() {
        let mut bindings = Bindings::new();
        bindings.listen(Action::HideObjects);
        assert_eq!(bindings.offer(Some(KeyCode::R), false, at()), Offer::Taken { changed: false });
        assert_eq!(bindings.key(Action::HideObjects), KeyCode::V, "unchanged");
        assert_eq!(bindings.listening(), Some(Action::HideObjects), "still waiting");
        assert_eq!(bindings.state_line(at()), "R is taken: it selects a random bibite");
        // Every key the game has a job for is refused, and each says what the job is.
        for key in [KeyCode::W, KeyCode::A, KeyCode::S, KeyCode::D, KeyCode::E, KeyCode::G, KeyCode::O, KeyCode::Z, KeyCode::K, KeyCode::H, KeyCode::Key1, KeyCode::Key4, KeyCode::F1, KeyCode::F2, KeyCode::F5, KeyCode::F9, KeyCode::F12] {
            assert!(taken_by(key).is_some(), "{key:?}");
            assert_eq!(bindings.offer(Some(key), false, at()), Offer::Taken { changed: false }, "{key:?}");
            assert_eq!(bindings.key(Action::HideObjects), KeyCode::V, "{key:?}");
        }
        // ... and then a free one is taken.
        assert_eq!(bindings.offer(Some(KeyCode::F6), false, at()), Offer::Taken { changed: true });
        assert_eq!(bindings.key(Action::HideObjects), KeyCode::F6);
    }

    #[test]
    fn a_key_that_is_not_a_hotkey_key_says_so_and_a_modifier_is_not_an_answer() {
        let mut bindings = Bindings::new();
        bindings.listen(Action::HideObjects);
        assert_eq!(bindings.offer(Some(KeyCode::Space), false, at()), Offer::Taken { changed: false });
        assert_eq!(bindings.state_line(at()), "That key cannot be used");
        assert_eq!(bindings.offer(Some(KeyCode::LeftControl), false, at()), Offer::Idle);
        assert_eq!(bindings.offer(None, false, at()), Offer::Idle);
        assert_eq!(bindings.listening(), Some(Action::HideObjects));
    }

    #[test]
    fn escape_and_a_click_end_the_wait_without_changing_the_key() {
        let mut bindings = Bindings::new();
        bindings.listen(Action::HideObjects);
        assert_eq!(bindings.offer(Some(KeyCode::Escape), false, at()), Offer::Taken { changed: false }, "Escape is the page's: it must not also close the screen");
        assert!(bindings.listening().is_none());
        assert_eq!(bindings.key(Action::HideObjects), KeyCode::V);
        bindings.listen(Action::HideObjects);
        assert_eq!(bindings.offer(Some(KeyCode::F6), true, at()), Offer::Idle, "a click ends the wait, and is the interface's");
        assert!(bindings.listening().is_none());
        assert_eq!(bindings.key(Action::HideObjects), KeyCode::V);
    }

    #[test]
    fn the_answer_of_a_frame_is_escape_or_the_lowest_key_that_is_not_a_modifier() {
        assert_eq!(answer_of([KeyCode::LeftControl, KeyCode::V]), Some(KeyCode::V));
        assert_eq!(answer_of([KeyCode::V, KeyCode::LeftShift]), Some(KeyCode::V));
        assert_eq!(answer_of([KeyCode::LeftControl]), None);
        assert_eq!(answer_of([KeyCode::V, KeyCode::Escape]), Some(KeyCode::Escape));
        let both = answer_of([KeyCode::B, KeyCode::A]);
        assert_eq!(both, answer_of([KeyCode::A, KeyCode::B]), "the order of a set does not matter");
        assert_eq!(both, Some(KeyCode::A));
        assert_eq!(answer_of(std::iter::empty()), None);
    }

    #[test]
    fn the_news_goes_after_a_few_seconds() {
        let mut bindings = Bindings::new();
        let start = at();
        bindings.listen(Action::HideObjects);
        let _ = bindings.offer(Some(KeyCode::R), false, start);
        assert!(bindings.state_line(start).starts_with("R is taken"));
        assert_eq!(bindings.state_line(start + NOTE_FOR + Duration::from_millis(1)), "Press the new key (Esc cancels)");
        bindings.cancel();
        assert_eq!(bindings.state_line(start + NOTE_FOR + Duration::from_millis(1)), "Click the key, then press another one");
    }

    #[test]
    fn reset_gives_the_default_keys_back() {
        let mut bindings = Bindings::new();
        bindings.listen(Action::HideObjects);
        let _ = bindings.offer(Some(KeyCode::F7), false, at());
        assert_eq!(bindings.key(Action::HideObjects), KeyCode::F7);
        bindings.reset(at());
        assert_eq!(bindings.key(Action::HideObjects), KeyCode::V);
        assert!(bindings.listening().is_none());
        assert_eq!(bindings.state_line(at()), "The default keys are back");
    }

    #[test]
    fn the_file_round_trips_and_a_bad_line_leaves_the_default() {
        let mut bindings = Bindings::new();
        bindings.listen(Action::HideObjects);
        let _ = bindings.offer(Some(KeyCode::Kp5), false, at());
        let text = bindings.to_text();
        assert!(text.contains("hide_objects=Num 5"), "{text}");
        assert_eq!(Bindings::from_text(&text).key(Action::HideObjects), KeyCode::Kp5);
        // A hand-written file: any case, spaces, a note, a BOM; a key the game uses, a key that does not exist and an unknown action change nothing.
        assert_eq!(Bindings::from_text("\u{feff}# note\r\n  Hide_Objects = f10 \r\n").key(Action::HideObjects), KeyCode::F10);
        for bad in ["hide_objects=R", "hide_objects=Space", "hide_objects=", "hide_objects", "other=F10", "=F10", ""] {
            assert_eq!(Bindings::from_text(bad).key(Action::HideObjects), KeyCode::V, "{bad:?}");
        }
    }

    #[test]
    fn the_process_wide_keys_load_save_and_read_back_in_a_data_folder() {
        let _alone = TEST_GLOBAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let folder = std::env::temp_dir().join(format!("rustibites-keys-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        // A folder without the file: the defaults, and the file is not made until a key changes.
        load(&folder);
        assert_eq!(key(Action::HideObjects), KeyCode::V);
        assert!(!file_in(&folder).exists());
        start_listening(Action::HideObjects);
        assert_eq!(button_text(Action::HideObjects), PRESS_A_KEY);
        assert!(offer(Some(KeyCode::F8), false), "a key was taken");
        assert_eq!(key(Action::HideObjects), KeyCode::F8);
        assert!(std::fs::read_to_string(file_in(&folder)).unwrap().contains("hide_objects=F8"));
        // A key that is refused is taken too (nothing else sees it), and changes nothing; no key waited for: nothing is taken.
        start_listening(Action::HideObjects);
        assert!(offer(Some(KeyCode::R), false));
        assert_eq!(key(Action::HideObjects), KeyCode::F8);
        cancel_listening();
        assert!(!offer(Some(KeyCode::F9), false));
        // The wait ends when the page is not on the screen (the player went to another page, or closed the screen), not before.
        start_listening(Action::HideObjects);
        page_shown(true);
        assert_eq!(button_text(Action::HideObjects), PRESS_A_KEY);
        page_shown(false);
        assert_eq!(button_text(Action::HideObjects), "F8");
        assert!(!offer(Some(KeyCode::F9), false), "nothing is waited for any more");
        // Another start of the window reads it.
        replace_for_test(Bindings::new());
        load(&folder);
        assert_eq!(key(Action::HideObjects), KeyCode::F8);
        reset();
        assert_eq!(key(Action::HideObjects), KeyCode::V);
        assert!(std::fs::read_to_string(file_in(&folder)).unwrap().contains("hide_objects=V"));
        assert!(line().starts_with("V|"));
        // Leave the process-wide state as a window that never started has it.
        replace_for_test(Bindings::new());
        let _ = std::fs::remove_dir_all(&folder);
    }
}

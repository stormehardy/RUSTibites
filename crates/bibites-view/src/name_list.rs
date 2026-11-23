//! RUSTibites Improved: the editable species name list. The words new species are named from are in a text file the player can edit,
//! `species_names.txt` (in the folder of the game's own files, `userdata`), and the Species Names page of the Settings screens opens it, reloads it and shows
//! what names it gives.
//!
//! A species is named by two words (`Genus epithet`): the game draws both from one list of words (the supporters' Latin names). The file holds words, one or
//! several to a line, and a line that starts with `#` is a note. `Species Names` says how they are used: only the game's list (the file is ignored), only the
//! player's words (while the file has none the game's list is used), or both (half of the names from each). The core does the naming
//! (`bibites_core::species_names`); this module reads the file, keeps the core's pool up to date and answers what the page shows.
//!
//! Nothing of this is in a save or in the simulation's numbers: a species' name is its two words, whatever list they came from.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use bibites_core::species_names::{NameMode, NamePool, sample_name, set_name_pool};

/// The file of the player's words.
pub const FILE_NAME: &str = "species_names.txt";
/// The label of the setting (a dropdown row of the page).
pub const LABEL_MODE: &str = "Species Names";
/// The setting's choices, in the order of `user_settings`' list.
pub const MODE_CHOICES: [&str; 3] = ["Game's List", "My List", "Both Lists"];
/// How often the file is looked at again (a save in an editor shows within this).
const CHECK_EVERY: Duration = Duration::from_secs(1);
/// The most words taken from the file.
const MAX_WORDS: usize = 20_000;
/// The most characters of a line the page shows.
const MAX_LINE: usize = 56;
/// What a new file says.
const TEMPLATE: &str = "# The words new species are named from. One or several words to a line; a line that starts with # is a note.\r\n\
# A species name is two words, a genus and an epithet (\"Fluffy bubbles\"): both are drawn from this list. The genus is shown with a capital letter and\r\n\
# the epithet in lower case, whatever you write. A short list runs out of fresh names quickly, so names then repeat.\r\n\
# Species Names on the Settings screen's Names page chooses what is used: the game's list, this list, or both.\r\n\
# The game looks at this file again every second, so a change you save applies to the species named from then on.\r\n\
#\r\n\
# Put your words under this line:\r\n";

/// The text help of the page's dropdown row (the title and the text), by label.
#[must_use]
pub fn help_of(label: &str) -> Option<(&'static str, &'static str)> {
    label.trim().eq_ignore_ascii_case(LABEL_MODE).then_some((
        LABEL_MODE,
        "Where new species' names come from: the game's own list of words, the words in your names file (Open File), or both, half of the names from each. Species that already have a name keep it.",
    ))
}

/// The file of a data folder.
#[must_use]
pub fn file_in(data: &Path) -> PathBuf {
    data.join(FILE_NAME)
}

/// What a file holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    /// The words, in the order of the file, each once (a word is the same word whatever its case).
    pub words: Vec<String>,
}

/// The words of a file's text: split on spaces, lines that start with `#` left out, a repeated word kept once.
#[must_use]
pub fn parse(text: &str) -> Parsed {
    let mut words: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.starts_with('#') {
            continue;
        }
        for word in line.split_whitespace() {
            if words.len() >= MAX_WORDS {
                return Parsed { words };
            }
            if seen.insert(word.to_lowercase()) {
                words.push(word.to_owned());
            }
        }
    }
    Parsed { words }
}

/// What the page says about the list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// The list has been started (in the game window; a screenshot has none).
    pub running: bool,
    /// The setting: an index of [`MODE_CHOICES`].
    pub mode: usize,
    /// How many words the file has.
    pub words: usize,
}

impl Status {
    /// The line of the page's "My List" row.
    #[must_use]
    pub fn line(&self) -> String {
        if !self.running {
            return "Read when the game window opens".to_owned();
        }
        let count = if self.words == 1 { "1 word".to_owned() } else { format!("{} words", self.words) };
        shorten(&match (self.mode, self.words) {
            (0, _) => format!("Ignored: the game's list is used ({count} in file)"),
            (_, 0) => "Empty: put words in the file (the game's list is used)".to_owned(),
            (1, _) => format!("{count}: species are named from them"),
            _ => format!("{count}: half the names are theirs"),
        })
    }
}

struct State {
    path: PathBuf,
    /// The modification time of the file when it was read (`None`: there was no file).
    modified: Option<SystemTime>,
    words: Vec<String>,
    mode: usize,
    checked: Instant,
    seed: u64,
}

fn state() -> &'static Mutex<Option<State>> {
    static STATE: OnceLock<Mutex<Option<State>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

fn locked() -> std::sync::MutexGuard<'static, Option<State>> {
    state().lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The setting the game was last told (so a `configure` before `start` is not lost).
fn pending_mode() -> &'static Mutex<usize> {
    static MODE: OnceLock<Mutex<usize>> = OnceLock::new();
    MODE.get_or_init(|| Mutex::new(1))
}

fn modified_of(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

/// Starts the list on `path` (a new file is made, with a note in it, when there is none): the game's species are named from it from now on. Only the game
/// window calls it, so a screenshot or a test never changes how species are named.
pub fn start(path: PathBuf) {
    if !path.exists() {
        if let Some(folder) = path.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let _ = std::fs::write(&path, TEMPLATE);
    }
    let mode = *pending_mode().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut slot = locked();
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0x9E37_79B9, |elapsed| elapsed.as_nanos() as u64);
    *slot = Some(State { modified: modified_of(&path), words: parse(&text).words, mode, checked: Instant::now(), seed, path });
    if let Some(state) = slot.as_ref() {
        apply(state);
    }
}

/// Makes the core's pool what the file and the setting say.
fn apply(state: &State) {
    let mode = match state.mode {
        0 => None,
        1 => Some(NameMode::Mine),
        _ => Some(NameMode::Both),
    };
    set_name_pool(mode.and_then(|mode| NamePool::new(state.words.clone(), mode)));
}

/// The setting `Species Names` is `mode` (an index of [`MODE_CHOICES`]).
pub fn configure(mode: usize) {
    let mode = mode.min(MODE_CHOICES.len() - 1);
    {
        let mut pending = pending_mode().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if *pending == mode && locked().as_ref().is_none_or(|state| state.mode == mode) {
            return;
        }
        *pending = mode;
    }
    if let Some(state) = locked().as_mut()
        && state.mode != mode
    {
        state.mode = mode;
        apply(state);
    }
}

/// Once a frame: reads the file again when it was saved since (looked at once a second).
pub fn tick() {
    let mut slot = locked();
    let Some(state) = slot.as_mut() else {
        return;
    };
    if state.checked.elapsed() < CHECK_EVERY {
        return;
    }
    state.checked = Instant::now();
    if modified_of(&state.path) != state.modified {
        read_again(state);
    }
}

/// The "Reload" button: the file is read again now.
pub fn reload() {
    if let Some(state) = locked().as_mut() {
        read_again(state);
    }
}

fn read_again(state: &mut State) {
    state.modified = modified_of(&state.path);
    state.words = parse(&std::fs::read_to_string(&state.path).unwrap_or_default()).words;
    apply(state);
}

/// What the page says about the list.
#[must_use]
pub fn status() -> Status {
    locked().as_ref().map_or_else(Status::default, |state| Status { running: true, mode: state.mode, words: state.words.len() })
}

/// The "Show New" button: other sample names.
pub fn new_samples() {
    if let Some(state) = locked().as_mut() {
        state.seed = state.seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
    }
}

/// Two names as the list in use would give them (`Species Names` as it is now), the page's "Sample Names" row.
#[must_use]
pub fn samples_line() -> String {
    let Some(seed) = locked().as_ref().map(|state| state.seed) else {
        return String::new();
    };
    let pool = bibites_core::species_names::name_pool();
    let mut x = seed | 1;
    let mut draw = || {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        ((x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32) / (1u64 << 24) as f32
    };
    let names: Vec<String> = (0..2).map(|_| sample_name(pool.as_deref(), draw(), draw())).collect();
    shorten(&names.join(",  "))
}

fn shorten(text: &str) -> String {
    if text.chars().count() <= MAX_LINE {
        return text.to_owned();
    }
    let kept: String = text.chars().take(MAX_LINE - 3).collect();
    format!("{}...", kept.trim_end())
}

/// Opens the file in the program the system has for text files.
///
/// # Errors
/// The list has not started (the file is made when the game window opens) or the system could not open it.
pub fn open_file() -> Result<(), String> {
    let path = locked().as_ref().map(|state| state.path.clone()).ok_or_else(|| "The names file is made when the game window opens".to_owned())?;
    let path = std::path::absolute(&path).unwrap_or(path);
    open_text_file(&path)
}

#[cfg(windows)]
fn open_text_file(path: &Path) -> Result<(), String> {
    // `explorer <file>` opens a file in the program the system has for its kind (a text editor for `.txt`), without a console window.
    std::process::Command::new("explorer").arg(path).spawn().map(|_| ()).map_err(|error| error.to_string())
}

#[cfg(not(windows))]
fn open_text_file(path: &Path) -> Result<(), String> {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(opener).arg(path).spawn().map(|_| ()).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of the system's temp folder that goes when the test ends.
    struct TempFolder(PathBuf);

    impl TempFolder {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("rustibites-names-{tag}-{}", std::process::id()));
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
    fn the_file_is_words_one_or_several_to_a_line_with_notes_left_out_and_each_word_once() {
        let text = "\u{feff}# a note\n\nFluffy bubbles\r\n  mossy   TINY  \n# another note: ignored words\nfluffy\nBubbles\n#\nöland-x\n";
        assert_eq!(parse(text).words, ["Fluffy", "bubbles", "mossy", "TINY", "öland-x"]);
        assert!(parse("").words.is_empty() && parse("# only notes\n#\n").words.is_empty());
        // A note is a line that STARTS with #; a word with one in it is a word.
        assert_eq!(parse("a#b\n").words, ["a#b"]);
        // The file made for a new list has notes only.
        assert!(parse(TEMPLATE).words.is_empty());
    }

    #[test]
    fn a_file_with_a_huge_number_of_words_is_cut() {
        let text: String = (0..MAX_WORDS + 50).map(|n| format!("w{n} ")).collect();
        assert_eq!(parse(&text).words.len(), MAX_WORDS);
    }

    #[test]
    fn the_page_says_what_the_list_does_in_each_mode() {
        let not_running = Status::default();
        assert_eq!(not_running.line(), "Read when the game window opens");
        let line = |mode, words| Status { running: true, mode, words }.line();
        assert_eq!(line(0, 5), "Ignored: the game's list is used (5 words in file)");
        assert_eq!(line(0, 1), "Ignored: the game's list is used (1 word in file)");
        assert_eq!(line(1, 0), "Empty: put words in the file (the game's list is used)");
        assert_eq!(line(2, 0), "Empty: put words in the file (the game's list is used)");
        assert_eq!(line(1, 12), "12 words: species are named from them");
        assert_eq!(line(2, 1), "1 word: half the names are theirs");
        assert!(line(1, 12).chars().count() <= MAX_LINE && line(0, 12345).chars().count() <= MAX_LINE);
    }

    #[test]
    fn the_modes_are_the_three_the_page_offers() {
        assert_eq!(MODE_CHOICES.len(), 3);
        assert!(crate::user_settings::index_of(LABEL_MODE).is_some(), "the setting exists");
        assert_eq!(
            crate::user_settings::UserSettings::choice_labels(crate::user_settings::index_of(LABEL_MODE).unwrap()),
            Some(&MODE_CHOICES[..])
        );
    }

    #[test]
    fn a_long_sample_line_is_cut_to_fit() {
        let long = "x".repeat(120);
        assert_eq!(shorten(&long).chars().count(), MAX_LINE);
        assert!(shorten(&long).ends_with("..."));
        assert_eq!(shorten("Fluffy bubbles"), "Fluffy bubbles");
    }

    /// The one test that starts the list: it changes the process-wide pool of the core, so it is the only one that does, and it puts it back.
    #[test]
    fn the_list_makes_the_file_reads_it_follows_the_setting_and_a_save_of_the_file_changes_the_names() {
        let folder = TempFolder::new("life");
        let path = file_in(&folder.0);
        assert!(!path.exists());
        configure(1);
        start(path.clone());
        let made = std::fs::read_to_string(&path).expect("a new file is made");
        assert!(made.starts_with("# The words new species are named from"));
        assert_eq!(status(), Status { running: true, mode: 1, words: 0 });
        assert!(bibites_core::species_names::name_pool().is_none(), "an empty list names nothing: the game's list is used");
        // Words in the file: read again at once by "Reload", and the core names from them.
        std::fs::write(&path, "# mine\nFluffy bubbles mossy\n").unwrap();
        reload();
        assert_eq!(status().words, 3);
        assert_eq!(bibites_core::species_names::name_pool().expect("a pool").len(), 3);
        let samples = samples_line();
        assert_eq!(samples.split(",  ").count(), 2, "{samples:?}");
        for name in samples.split(",  ") {
            let words: Vec<&str> = name.split(' ').collect();
            assert_eq!(words.len(), 2, "{name}");
            assert!(["fluffy", "bubbles", "mossy"].contains(&words[0].to_lowercase().as_str()) && ["fluffy", "bubbles", "mossy"].contains(&words[1]), "{name}");
        }
        // A world's first species (a fresh registry, as a new world's) is named from the player's words, both of its words, through the process-wide pool.
        {
            use bibites_core::brain::Brain;
            use bibites_core::genome::{Genome, GenomeMeta};
            let mut registry = bibites_core::species::SpeciesRegistry::with_seed(3);
            let mut meta = GenomeMeta::default();
            registry.assign(&Genome::starter(), &Brain::starter(), &mut meta, &bibites_core::settings::SimSettings::game_defaults());
            let record = &registry.records()[0];
            let mine = ["fluffy", "bubbles", "mossy"];
            assert!(mine.contains(&record.generic_name.to_lowercase().as_str()), "{}", record.generic_name);
            assert!(record.generic_name.chars().next().unwrap().is_uppercase());
            assert!(mine.contains(&record.specific_name.as_str()), "{}", record.specific_name);
        }
        // Other samples on the button (the same pool, another draw), the same ones until it is pressed.
        assert_eq!(samples_line(), samples);
        let mut changed = false;
        for _ in 0..20 {
            new_samples();
            changed |= samples_line() != samples;
        }
        assert!(changed, "new samples are other names");
        // The setting: the game's list ignores the file; both lists share.
        configure(0);
        assert!(bibites_core::species_names::name_pool().is_none());
        assert_eq!(status().line(), "Ignored: the game's list is used (3 words in file)");
        configure(2);
        assert_eq!(bibites_core::species_names::name_pool().expect("a pool").reach(), 3 + bibites_core::species_names::game_word_count());
        // A save of the file is noticed by the tick (the file's time changes; the check is once a second).
        configure(1);
        std::thread::sleep(Duration::from_millis(1100));
        std::fs::write(&path, "alpha beta gamma delta\n").unwrap();
        // Some file systems keep the time to the second: make sure it moved.
        let later = SystemTime::now() + Duration::from_secs(3);
        let _ = std::fs::OpenOptions::new().write(true).open(&path).and_then(|file| file.set_modified(later));
        tick();
        assert_eq!(status().words, 4, "the saved file was read again by the tick");
        // Put the shared pool back as it was.
        set_name_pool(None);
        *locked() = None;
        configure(1);
    }
}

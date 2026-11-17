//! Filesystem persistence for viewer/UI actions (saves, scenarios, templates,
//! user preferences). The game stores a few different kinds of state:
//!
//! * `Application.persistentDataPath/Savefiles/*.zip` (manual saves),
//! * `.../Savefiles/Autosaves/autosave_YYYYMMDDHHMMSS.zip`,
//! * `.../Scenarios/*.zip` (scenario + challenge bundles),
//! * `.../Bibites/Templates/*.bb8template`,
//! * user prefs in Unity `PlayerPrefs` (not a file we can portably read).
//!
//! RUSTibites keeps the same directory *shape* under a caller-provided root
//! (default: `RUSTIBITES_DATA_DIR` or `./userdata`), and persists user prefs as
//! JSON at `<state>/user_settings.json`. Import/export helpers copy files into or
//! out of that layout. The interactive viewer points the root at the installed
//! game's own data folder ([`StoragePaths::default_content`]) so the original game
//! lists what this port saves; the port's private files (user settings, challenge
//! progress, thumbnail cache) stay in the separate *state* folder either way.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use bibites_core::world::WorldContents;

use crate::world::{load_world_contents, save_world_contents};
use crate::{Error, Result};

/// Directory layout used by the viewer's save/load/scenario UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoragePaths {
    /// Data root containing `Savefiles/`, `Scenarios/`, `Bibites/Templates/`
    /// (the game's `Application.persistentDataPath` layout).
    pub root: PathBuf,
    /// Folder of this port's own files (`user_settings.json`,
    /// `challenge_progress.json`, the thumbnail cache, `Workshop/`): `root` itself
    /// unless `root` is the installed game's folder.
    state: PathBuf,
}

impl StoragePaths {
    /// Creates a layout rooted at `root`; directories are created on demand. The
    /// port's own files live under the same root.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            state: root.clone(),
            root,
        }
    }

    /// The same content root with the port's own files kept in `state`.
    #[must_use]
    pub fn with_state_dir(mut self, state: impl Into<PathBuf>) -> Self {
        self.state = state.into();
        self
    }

    /// Where the port's own files (user settings, challenge progress, thumbnail
    /// cache, local workshop folder) live.
    #[must_use]
    pub fn state_dir(&self) -> &Path {
        &self.state
    }

    /// Default layout: `$RUSTIBITES_DATA_DIR`, else `./userdata`.
    #[must_use]
    pub fn default_local() -> Self {
        let root = std::env::var_os("RUSTIBITES_DATA_DIR")
            .map_or_else(|| PathBuf::from("userdata"), PathBuf::from);
        Self::new(root)
    }

    /// Where the interactive viewer keeps worlds, autosaves, scenarios and bibites:
    /// `$RUSTIBITES_DATA_DIR` when set (an explicit choice, also the opt-out), else
    /// the installed game's own data folder so the original game sees them, else
    /// `./userdata`. The port's private files stay under [`Self::default_local`].
    #[must_use]
    pub fn default_content() -> Self {
        let local = Self::default_local();
        if std::env::var_os("RUSTIBITES_DATA_DIR").is_some() {
            return local;
        }
        match Self::game_root_guess() {
            Some(game) => Self::new(game).with_state_dir(local.root),
            None => local,
        }
    }

    /// The installed game's data root
    /// (`%USERPROFILE%/AppData/LocalLow/The Bibites/The Bibites`), when it exists.
    /// Only written when a caller makes it the content root ([`Self::default_content`]).
    #[must_use]
    pub fn game_root_guess() -> Option<PathBuf> {
        let profile = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
        let root = PathBuf::from(profile).join("AppData/LocalLow/The Bibites/The Bibites");
        root.is_dir().then_some(root)
    }

    #[must_use]
    /// `Savefiles/` (manual worlds).
    pub fn saves_dir(&self) -> PathBuf {
        self.root.join("Savefiles")
    }

    #[must_use]
    /// `Savefiles/Autosaves/` (timestamped autosaves).
    pub fn autosaves_dir(&self) -> PathBuf {
        self.saves_dir().join("Autosaves")
    }

    #[must_use]
    /// `Scenarios/` (scenario and challenge bundles).
    pub fn scenarios_dir(&self) -> PathBuf {
        self.root.join("Scenarios")
    }

    #[must_use]
    /// `Bibites/` (single bibite and egg saves, `SaveSystem.savedBibitePath`).
    pub fn bibites_dir(&self) -> PathBuf {
        self.root.join("Bibites")
    }

    #[must_use]
    /// `Bibites/Templates/` (`.bb8template` files).
    pub fn templates_dir(&self) -> PathBuf {
        self.bibites_dir().join("Templates")
    }

    #[must_use]
    /// `<state>/user_settings.json` (RUSTibites user preferences).
    pub fn preferences_path(&self) -> PathBuf {
        self.state.join("user_settings.json")
    }

    #[must_use]
    /// `<state>/challenge_progress.json` (best result per challenge; the game keeps
    /// these in `PlayerPrefs`, see `ChallengesProgress`).
    pub fn challenge_progress_path(&self) -> PathBuf {
        self.state.join("challenge_progress.json")
    }

    #[must_use]
    /// `<state>/achievements.json` (the achievements this port unlocked and its counters; the game keeps its own in
    /// `PlayerPrefs`, see `Achievement.Trigger`).
    pub fn achievements_path(&self) -> PathBuf {
        self.state.join("achievements.json")
    }

    /// Creates the save/auto-save/scenario/template directories if missing.
    pub fn ensure_layout(&self) -> Result<()> {
        for dir in [
            self.saves_dir(),
            self.autosaves_dir(),
            self.scenarios_dir(),
            self.templates_dir(),
        ] {
            std::fs::create_dir_all(&dir)?;
        }
        Ok(())
    }
}

/// One file shown in a save/scenario/template list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFile {
    /// File name only (`foo.zip`, `autosave_20260101000000.zip`).
    pub name: String,
    /// Absolute/relative path as returned by the directory join.
    pub path: PathBuf,
    /// File size in bytes.
    pub size: u64,
    /// Last modification time, when the filesystem exposes it.
    pub modified: Option<SystemTime>,
    /// True for `Savefiles/Autosaves/*.zip`.
    /// Enable periodic autosaves.
    pub autosave: bool,
}

/// Lists files with `ext` (without dot, case-insensitive) in `dir`, sorted by
/// modification time descending (then name), which is the game's list order.
pub fn list_files(dir: &Path, ext: &str) -> Result<Vec<StoredFile>> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(out),
        Err(error) => return Err(Error::from(error)),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() {
            continue;
        }
        let matches = path
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|actual| actual.eq_ignore_ascii_case(ext));
        if !matches {
            continue;
        }
        let metadata = entry.metadata()?;
        out.push(StoredFile {
            name: path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or_default()
                .to_string(),
            path,
            size: metadata.len(),
            modified: metadata.modified().ok(),
            autosave: false,
        });
    }
    out.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

/// Manual saves, plus auto-saves when `include_autosaves` is set (the game's
/// `SaveController.GetSaves(includeAutoSave)` behaviour). Auto-saves are marked
/// `autosave: true` even though they share the `.zip` extension.
pub fn list_saves(paths: &StoragePaths, include_autosaves: bool) -> Result<Vec<StoredFile>> {
    let mut saves = list_files(&paths.saves_dir(), "zip")?;
    if include_autosaves {
        let mut autosaves = list_files(&paths.autosaves_dir(), "zip")?;
        for save in &mut autosaves {
            save.autosave = true;
        }
        saves.extend(autosaves);
    }
    saves.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(saves)
}

/// Scenario/challenge bundles (`.zip`) in `Scenarios/`.
pub fn list_scenarios(paths: &StoragePaths) -> Result<Vec<StoredFile>> {
    list_files(&paths.scenarios_dir(), "zip")
}

/// Bibite templates (`.bb8template`) in `Bibites/Templates/`.
pub fn list_templates(paths: &StoragePaths) -> Result<Vec<StoredFile>> {
    list_files(&paths.templates_dir(), "bb8template")
}

/// Validates a user-supplied file name and returns it with the expected
/// extension. Mirrors the game's bad-character check (`SaveGamePanel`).
pub fn sanitize_file_name(name: &str, extension: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(Error::invalid_data("name is empty"));
    }
    if trimmed.chars().any(|c| {
        matches!(
            c,
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'
        )
    }) {
        return Err(Error::invalid_data("name contains an invalid character"));
    }
    if trimmed.starts_with('.') {
        return Err(Error::invalid_data("name may not start with a dot"));
    }
    let suffix = format!(".{extension}");
    if trimmed.to_ascii_lowercase().ends_with(&suffix) {
        Ok(trimmed.to_string())
    } else {
        Ok(format!("{trimmed}{suffix}"))
    }
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

/// Save-as: copies a world save into `Savefiles/<name>.zip`.
/// `overwrite=false` is the UI default; the caller confirms overwrite first.
pub fn save_as(
    source: &Path,
    paths: &StoragePaths,
    name: &str,
    overwrite: bool,
) -> Result<PathBuf> {
    let file_name = sanitize_file_name(name, "zip")?;
    let destination = paths.saves_dir().join(file_name);
    ensure_parent(&destination)?;
    if destination.exists() && !overwrite {
        return Err(Error::invalid_data("save already exists"));
    }
    std::fs::copy(source, &destination)?;
    Ok(destination)
}

/// Imports (copies) a scenario bundle into `Scenarios/`, validating that it is a
/// scenario-style zip before exposing it in the viewer list.
pub fn import_scenario(
    source: &Path,
    paths: &StoragePaths,
    name: &str,
    overwrite: bool,
) -> Result<PathBuf> {
    crate::load_scenario_bundle(source)?;
    let file_name = sanitize_file_name(name, "zip")?;
    let destination = paths.scenarios_dir().join(file_name);
    ensure_parent(&destination)?;
    if destination.exists() && !overwrite {
        return Err(Error::invalid_data("scenario already exists"));
    }
    std::fs::copy(source, &destination)?;
    Ok(destination)
}

/// Exports a save/scenario/template to an arbitrary user-selected path.
pub fn export_file(source: &Path, destination: &Path) -> Result<()> {
    ensure_parent(destination)?;
    std::fs::copy(source, destination)?;
    Ok(())
}

/// Deletes a save/scenario/template file (UI delete action).
pub fn delete_file(path: &Path) -> Result<()> {
    std::fs::remove_file(path)?;
    Ok(())
}

/// Renames a file within its directory (UI rename action; extension preserved
/// if the new name omits it).
pub fn rename_file(path: &Path, new_name: &str) -> Result<PathBuf> {
    let extension = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("zip");
    let file_name = sanitize_file_name(new_name, extension)?;
    let destination = path.with_file_name(file_name);
    if destination.exists() && destination != path {
        return Err(Error::invalid_data("destination already exists"));
    }
    std::fs::rename(path, &destination)?;
    Ok(destination)
}

/// Viewer user preferences. The game uses Unity `PlayerPrefs`; RUSTibites keeps
/// the equivalent keys in a JSON file so view-dev can persist toggles portably.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UserPreferences {
    /// Load panel's "include auto-saves" toggle.
    pub include_autosaves_in_load_panel: bool,
    /// Enable periodic autosaves.
    pub autosave: bool,
    /// Autosave cadence is wall-clock instead of sim time.
    pub autosave_real_time: bool,
    /// Minutes between autosaves.
    pub autosave_period_minutes: u32,
    /// Maximum retained autosaves (0 disables cleanup).
    pub autosave_max_files: u32,
    /// Pause the simulation after loading a save.
    pub pause_after_load: bool,
    /// Reload the world after each autosave.
    pub reload_after_autosaves: bool,
    /// Optional user override for the save directory (empty = layout default).
    /// Optional save-directory override (empty = layout default).
    pub save_path_override: String,
    /// Viewer UI scale (1.0 = default).
    pub ui_scale: f32,
    /// Fullscreen window mode.
    pub fullscreen: bool,
    /// Preferred window width.
    pub resolution_width: u32,
    /// Preferred window height.
    pub resolution_height: u32,
    /// Background style/mode key (viewer-defined; empty = default).
    pub background_mode: String,
    /// Show the world grid.
    pub show_grid: bool,
    /// Show node names in brain/lineage views.
    pub show_node_names: bool,
    /// Show the nodes' values over them in the Bibite Editor's Brain tab (`UserSettings.ShowNodeValues`, off by default).
    pub show_node_values: bool,
    /// Show pheromone spots.
    pub show_pheromones: bool,
    /// Pheromone spot opacity (0..=1).
    pub pheromone_opacity: f32,
    /// Minimum target FPS (0 = uncapped).
    pub min_fps: u32,
    /// Camera zoom (1.0 = default).
    pub camera_zoom: f32,
    /// Camera follows the selected entity.
    pub camera_follow_selection: bool,
    /// Camera pan speed multiplier.
    pub camera_pan_speed: f32,
    /// Selection outline/marker enabled.
    pub selection_outline: bool,
    /// Viewer hotkey bindings (action -> key name, e.g. `Space`, `F12`).
    #[serde(default = "default_hotkeys")]
    pub hotkeys: BTreeMap<String, String>,
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            include_autosaves_in_load_panel: false,
            autosave: true,
            // `UserSettings.AutoSaveRealTime` = true, `AutoSaveMaxFiles` = 25.
            autosave_real_time: true,
            autosave_period_minutes: 10,
            autosave_max_files: 25,
            pause_after_load: false,
            reload_after_autosaves: false,
            save_path_override: String::new(),
            ui_scale: 1.0,
            fullscreen: false,
            resolution_width: 1920,
            resolution_height: 1080,
            background_mode: String::new(),
            show_grid: false,
            // `UserSettings.ShowNodeNames` default = false.
            show_node_names: false,
            // `UserSettings.ShowNodeValues` default = false.
            show_node_values: false,
            show_pheromones: true,
            pheromone_opacity: 1.0,
            // `UserSettings.minimumFPS` default = 15.
            min_fps: 15,
            camera_zoom: 1.0,
            camera_follow_selection: false,
            camera_pan_speed: 1.0,
            selection_outline: true,
            hotkeys: default_hotkeys(),
        }
    }
}

/// Default hotkey bindings mirroring the viewer's current hardcoded keys.
#[must_use]
pub fn default_hotkeys() -> BTreeMap<String, String> {
    [
        ("pause", "Space"),
        ("step", "Right"),
        ("fit", "F"),
        ("fit_bibites", "B"),
        ("reset_view", "R"),
        ("toggle_grid", "G"),
        ("toggle_zones", "Z"),
        ("toggle_pellets", "P"),
        ("toggle_pheromones", "O"),
        ("toggle_hud", "H"),
        ("quit", "Escape"),
        ("screenshot", "F12"),
    ]
    .into_iter()
    .map(|(action, key)| (action.to_string(), key.to_string()))
    .collect()
}

impl UserPreferences {
    /// Current binding for a viewer action (if any).
    #[must_use]
    pub fn hotkey(&self, action: &str) -> Option<&str> {
        self.hotkeys.get(action).map(String::as_str)
    }

    /// Sets a binding; empty action/key is rejected.
    pub fn set_hotkey(&mut self, action: &str, key: &str) -> Result<()> {
        if action.trim().is_empty() || key.trim().is_empty() {
            return Err(Error::invalid_data("hotkey action/key must not be empty"));
        }
        self.hotkeys.insert(action.to_string(), key.to_string());
        Ok(())
    }

    /// Replaces the persisted prefs with factory defaults and returns them.
    pub fn reset(paths: &StoragePaths) -> Result<Self> {
        let defaults = Self::default();
        save_user_preferences(paths, &defaults)?;
        Ok(defaults)
    }
}

/// Loads `<root>/user_settings.json`; missing file returns defaults.
pub fn load_user_preferences(paths: &StoragePaths) -> Result<UserPreferences> {
    let path = paths.preferences_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(UserPreferences::default());
        }
        Err(error) => return Err(Error::from(error)),
    };
    serde_json::from_str(&text)
        .map_err(|error| Error::invalid_data(format!("invalid user_settings.json: {error}")))
}

/// Atomically writes `<root>/user_settings.json`.
pub fn save_user_preferences(paths: &StoragePaths, preferences: &UserPreferences) -> Result<()> {
    let path = paths.preferences_path();
    ensure_parent(&path)?;
    let text = serde_json::to_string_pretty(preferences)
        .map_err(|error| Error::invalid_data(error.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Whether `name` is an `autosave_<yyyyMMddHHmmss>.zip` file the game's autosave writes.
fn is_autosave_name(name: &str) -> bool {
    name.strip_prefix("autosave_")
        .and_then(|rest| rest.strip_suffix(".zip"))
        .is_some_and(|stamp| !stamp.is_empty() && stamp.chars().all(|c| c.is_ascii_digit()))
}

/// Removes auto-saves beyond `max_files`, newest kept (game
/// `SaveController.CleanUpAutosaveFolder`; `max_files <= 0` disables cleanup).
///
/// Only `autosave_<digits>.zip` files are ever removed (the game deletes every zip in the
/// folder): the folder can be the installed game's own, where other files may live.
pub fn cleanup_autosaves(paths: &StoragePaths, max_files: usize) -> Result<usize> {
    cleanup_autosaves_limited(paths, max_files, usize::MAX)
}

/// [`cleanup_autosaves`] that removes at most `max_removals` files per call (oldest first).
pub fn cleanup_autosaves_limited(
    paths: &StoragePaths,
    max_files: usize,
    max_removals: usize,
) -> Result<usize> {
    if max_files == 0 {
        return Ok(0);
    }
    let mut autosaves: Vec<StoredFile> = list_files(&paths.autosaves_dir(), "zip")?
        .into_iter()
        .filter(|file| is_autosave_name(&file.name))
        .collect();
    if autosaves.len() <= max_files {
        return Ok(0);
    }
    // `list_files` sorts newest first; the excess is the oldest tail.
    let excess = autosaves.split_off(max_files);
    let mut removed = 0;
    for file in excess.iter().rev().take(max_removals) {
        std::fs::remove_file(&file.path)?;
        removed += 1;
    }
    Ok(removed)
}

/// High-level viewer storage facade: maps UI actions (save-as, load, quick
/// save/load, list refresh, import/export) to io storage and save APIs. The
/// caller owns the `World`; pass `world.to_contents()` to save and
/// `World::from_contents(contents)` after loading.
#[derive(Debug, Clone)]
pub struct SaveBrowser {
    /// Directory layout used by the viewer.
    pub paths: StoragePaths,
}

impl SaveBrowser {
    /// Creates a browser over the given directory layout.
    #[must_use]
    pub fn new(paths: StoragePaths) -> Self {
        Self { paths }
    }

    /// Save current world contents under `Savefiles/<name>.zip` (save-as).
    /// `overwrite=false` matches the UI's confirmation-first behavior.
    pub fn save_world(
        &self,
        name: &str,
        contents: &WorldContents,
        overwrite: bool,
    ) -> Result<PathBuf> {
        self.paths.ensure_layout()?;
        let file_name = sanitize_file_name(name, "zip")?;
        let destination = self.paths.saves_dir().join(file_name);
        if destination.exists() && !overwrite {
            return Err(Error::invalid_data("save already exists"));
        }
        let mut bundle = crate::world::save_bundle_from_contents(contents, None)?;
        bundle.templates = self.bundled_templates(contents);
        crate::write_save_bundle(&bundle, &destination)?;
        Ok(destination)
    }

    /// `SaveSystem.AddTemplatesToArchive`: the template of every spawn row goes into the save as `templates/<file>` (the game's own templates, which
    /// every install has, are left out; a missing file is skipped).
    fn bundled_templates(&self, contents: &WorldContents) -> Vec<(String, Vec<u8>)> {
        let mut out: Vec<(String, Vec<u8>)> = Vec::new();
        for row in &contents.scene.bibite_spawns {
            if row.path.is_empty() {
                continue;
            }
            let name = Path::new(&row.path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let source = if row.is_external {
                PathBuf::from(&row.path)
            } else if name.to_ascii_lowercase().ends_with(".bb8template") {
                self.paths.templates_dir().join(&name)
            } else {
                self.paths.bibites_dir().join(&name)
            };
            if !row.is_external && name.to_ascii_lowercase().ends_with(".bb8template") {
                let official = crate::load_template(&source).is_ok_and(|loaded| loaded.template.is_official);
                if official {
                    continue;
                }
            }
            let entry = format!("templates/{name}");
            if out.iter().any(|(existing, _)| *existing == entry) {
                continue;
            }
            if let Ok(bytes) = std::fs::read(&source) {
                out.push((entry, bytes));
            }
        }
        out
    }

    /// `SaveSystem.CheckTemplatesOfArchive`: the templates of a save that the player does not have are written to their folders. Returns their names.
    pub fn restore_missing_templates(&self, save: &Path) -> Vec<String> {
        let Ok(file) = std::fs::File::open(save) else {
            return Vec::new();
        };
        let Ok(mut archive) = zip::ZipArchive::new(file) else {
            return Vec::new();
        };
        let mut added = Vec::new();
        for index in 0..archive.len() {
            let Ok(mut entry) = archive.by_index(index) else {
                continue;
            };
            let full = entry.name().replace('\\', "/");
            if !full.to_ascii_lowercase().starts_with("templates") {
                continue;
            }
            let name = full.rsplit('/').next().unwrap_or_default().to_owned();
            let lower = name.to_ascii_lowercase();
            let folder = if lower.ends_with(".bb8template") {
                self.paths.templates_dir()
            } else if lower.ends_with(".bb8") {
                self.paths.bibites_dir()
            } else {
                continue;
            };
            let target = folder.join(&name);
            if target.exists() {
                continue;
            }
            let mut bytes = Vec::new();
            if std::io::Read::read_to_end(&mut entry, &mut bytes).is_ok()
                && std::fs::create_dir_all(&folder).is_ok()
                && std::fs::write(&target, bytes).is_ok()
            {
                added.push(name);
            }
        }
        added
    }

    /// Load a save by path into world contents (load panel / quick load).
    pub fn load_world(&self, path: impl AsRef<Path>) -> Result<WorldContents> {
        load_world_contents(path, None)
    }

    /// Quick save to `Savefiles/quick.zip` (always overwrites).
    pub fn quick_save(&self, contents: &WorldContents) -> Result<PathBuf> {
        self.save_world("quick", contents, true)
    }

    /// Quick load from `Savefiles/quick.zip`; errors clearly when absent.
    pub fn quick_load(&self) -> Result<WorldContents> {
        let path = self.paths.saves_dir().join("quick.zip");
        if !path.is_file() {
            return Err(Error::invalid_data("no quick save exists"));
        }
        self.load_world(path)
    }

    /// Refreshes the load/save panel list after a save/delete/rename/import.
    pub fn refresh_saves(&self, include_autosaves: bool) -> Result<Vec<StoredFile>> {
        list_saves(&self.paths, include_autosaves)
    }

    /// Refreshes the scenario selector list.
    pub fn refresh_scenarios(&self) -> Result<Vec<StoredFile>> {
        list_scenarios(&self.paths)
    }

    /// Refreshes the template selector list.
    pub fn refresh_templates(&self) -> Result<Vec<StoredFile>> {
        list_templates(&self.paths)
    }

    /// Save-as another save file (copy) from an existing zip path.
    pub fn save_as_file(&self, source: &Path, name: &str, overwrite: bool) -> Result<PathBuf> {
        save_as(source, &self.paths, name, overwrite)
    }

    /// Delete a save/scenario/template, then the caller calls the matching
    /// `refresh_*` (or use [`Self::delete_save`] which returns the new list).
    pub fn delete_save(&self, path: &Path, include_autosaves: bool) -> Result<Vec<StoredFile>> {
        delete_file(path)?;
        self.refresh_saves(include_autosaves)
    }

    /// Rename a save, then return the refreshed save list.
    pub fn rename_save(
        &self,
        path: &Path,
        new_name: &str,
        include_autosaves: bool,
    ) -> Result<(PathBuf, Vec<StoredFile>)> {
        let renamed = rename_file(path, new_name)?;
        Ok((renamed, self.refresh_saves(include_autosaves)?))
    }

    /// Import a scenario/challenge zip and refresh the scenario list.
    pub fn import_scenario(
        &self,
        source: &Path,
        name: &str,
        overwrite: bool,
    ) -> Result<(PathBuf, Vec<StoredFile>)> {
        let path = crate::storage::import_scenario(source, &self.paths, name, overwrite)?;
        Ok((path, self.refresh_scenarios()?))
    }

    /// Import a `.bb8template` into the template folder and refresh the list.
    pub fn import_template(
        &self,
        source: &Path,
        name: &str,
        overwrite: bool,
    ) -> Result<(PathBuf, Vec<StoredFile>)> {
        let file_name = sanitize_file_name(name, "bb8template")?;
        let destination = self.paths.templates_dir().join(file_name);
        if destination.exists() && !overwrite {
            return Err(Error::invalid_data("template already exists"));
        }
        crate::load_template(source)?;
        self.paths.ensure_layout()?;
        std::fs::copy(source, &destination)?;
        Ok((destination, self.refresh_templates()?))
    }

    /// Export any save/scenario/template to a user-chosen path.
    pub fn export(&self, source: &Path, destination: &Path) -> Result<()> {
        export_file(source, destination)
    }
}

/// One save row for the load/save browser. Built from a cheap zip metadata read
/// (scene JSON + thumbnail member) rather than parsing all entities.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveListEntry {
    /// File name (`foo.zip`, `autosave_....zip`).
    pub name: String,
    /// Full save path.
    pub path: PathBuf,
    /// File size in bytes.
    pub size: u64,
    /// Last modification time.
    pub modified: Option<SystemTime>,
    /// True for `Savefiles/Autosaves/*.zip`.
    pub autosave: bool,
    /// `scene.version` when readable.
    pub version: Option<String>,
    /// `scene.simulatedTime` when readable.
    pub sim_time: Option<f64>,
    /// `scene.nBibites` (display count) when readable.
    pub population: Option<u64>,
    /// Cached extracted thumbnail (`<root>/thumbnails/<stem>.png`) when present.
    pub thumbnail: Option<PathBuf>,
}

/// Reads cheap metadata straight from a save zip (scene JSON, no entity parse).
/// Unreadable zips yield `None` fields rather than failing the whole list.
#[must_use]
pub fn read_save_metadata(path: &Path) -> (Option<String>, Option<f64>, Option<u64>) {
    let Ok(file) = std::fs::File::open(path) else {
        return (None, None, None);
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return (None, None, None);
    };
    let Ok(mut entry) = archive.by_name("scene.bb8scene") else {
        return (None, None, None);
    };
    let mut bytes = Vec::new();
    if std::io::Read::read_to_end(&mut entry, &mut bytes).is_err() {
        return (None, None, None);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return (None, None, None);
    };
    (
        json.get("version")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        json.get("simulatedTime")
            .and_then(serde_json::Value::as_f64),
        json.get("nBibites").and_then(serde_json::Value::as_u64),
    )
}

impl StoragePaths {
    /// `<state>/thumbnails/` cache for extracted save thumbnails.
    #[must_use]
    pub fn thumbnails_dir(&self) -> PathBuf {
        self.state.join("thumbnails")
    }
}

/// Save browser rows with metadata and cached thumbnail paths.
pub fn list_save_entries(
    paths: &StoragePaths,
    include_autosaves: bool,
) -> Result<Vec<SaveListEntry>> {
    list_save_entries_head(paths, include_autosaves, usize::MAX)
}

/// [`list_save_entries`] reading the zip metadata of the first `with_metadata` rows (newest first) only: the other rows keep `None` in
/// `version`, `sim_time` and `population`. Opening every zip of a folder takes seconds when it holds a thousand autosaves (the game's own
/// folder had 1,382 of them, 3.4 GB: five seconds frozen on every F5 and Load Game press).
pub fn list_save_entries_head(
    paths: &StoragePaths,
    include_autosaves: bool,
    with_metadata: usize,
) -> Result<Vec<SaveListEntry>> {
    let mut entries = Vec::new();
    for (index, file) in list_saves(paths, include_autosaves)?
        .into_iter()
        .enumerate()
    {
        let (version, sim_time, population) = if index < with_metadata {
            read_save_metadata(&file.path)
        } else {
            (None, None, None)
        };
        let thumb = paths
            .thumbnails_dir()
            .join(file.path.file_stem().unwrap_or_default())
            .with_extension("png");
        entries.push(SaveListEntry {
            name: file.name,
            path: file.path,
            size: file.size,
            modified: file.modified,
            autosave: file.autosave,
            version,
            sim_time,
            population,
            thumbnail: thumb.is_file().then_some(thumb),
        });
    }
    Ok(entries)
}

/// What `LoadGamePanel.SelectSaveItem` shows for one save.
#[derive(Debug, Clone, Default)]
pub struct SaveCard {
    /// `scene.version` (`None` when missing/unreadable).
    pub version: Option<String>,
    /// Simulated seconds.
    pub sim_time: f64,
    /// `scene.nBibites` (`None` shows `?`).
    pub n_bibites: Option<u64>,
    /// `scene.nPellets`.
    pub n_pellets: u64,
    /// `SimulationSize` of the save's settings.
    pub sim_size: f32,
    /// `img.png` bytes, when present.
    pub preview_png: Option<Vec<u8>>,
}

/// Why a save cannot be listed (`SaveItemReference.InitSaveItem` catches the exception of `ZipFile.Open` / `GetSceneOfSave`): `None` when the file opens as
/// a zip with a `scene.bb8scene` that parses, else the first line of the reason.
#[must_use]
pub fn save_problem(path: &Path) -> Option<String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) => return Some(error.to_string()),
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(archive) => archive,
        Err(error) => return Some(error.to_string()),
    };
    // `GetSceneOfSave`: `scene.bb8scene`, else `scene.json`.
    let name = if archive.by_name("scene.bb8scene").is_ok() { "scene.bb8scene" } else { "scene.json" };
    let mut entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(error) => return Some(error.to_string()),
    };
    let mut bytes = Vec::new();
    if let Err(error) = std::io::Read::read_to_end(&mut entry, &mut bytes) {
        return Some(error.to_string());
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    serde_json::from_slice::<serde_json::Value>(bytes).err().map(|error| error.to_string())
}

/// The version a save was written by (`GetVersionOfFile` over the scene): `None` when the file has no readable scene or version.
#[must_use]
pub fn save_version(path: &Path) -> Option<crate::version::Version> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let name = if archive.by_name("scene.bb8scene").is_ok() { "scene.bb8scene" } else { "scene.json" };
    let mut entry = archive.by_name(name).ok()?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut bytes).ok()?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    let scene: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    crate::version::Version::parse(scene.get("version")?.as_str()?).ok()
}

/// Reads the load-panel details of a save zip (best effort, never fails).
#[must_use]
pub fn read_save_card(path: &Path) -> SaveCard {
    let mut card = SaveCard::default();
    let Ok(file) = std::fs::File::open(path) else {
        return card;
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return card;
    };
    let mut read_json = |name: &str| -> Option<serde_json::Value> {
        let mut entry = archive.by_name(name).ok()?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes).ok()?;
        let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
        serde_json::from_slice(bytes).ok()
    };
    if let Some(scene) = read_json("scene.bb8scene") {
        card.version = scene
            .get("version")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        card.sim_time = scene
            .get("simulatedTime")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);
        card.n_bibites = scene.get("nBibites").and_then(serde_json::Value::as_u64);
        card.n_pellets = scene
            .get("nPellets")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
    }
    if let Some(settings) = read_json("settings.bb8settings") {
        let size = settings.get("SimulationSize").or_else(|| {
            settings
                .get("independents")
                .and_then(|i| i.get("SimulationSize"))
        });
        card.sim_size = size
            .and_then(|value| value.get("Value").or(Some(value)))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0) as f32;
    }
    if let Ok(mut entry) = archive.by_name("img.png") {
        let mut bytes = Vec::new();
        if std::io::Read::read_to_end(&mut entry, &mut bytes).is_ok() {
            card.preview_png = Some(bytes);
        }
    }
    card
}

/// Extracts the save's `img.png` member to the viewer thumbnail cache
/// (`<root>/thumbnails/<stem>.png`). Returns `None` when the save has no image.
pub fn extract_thumbnail(save: &Path, paths: &StoragePaths) -> Result<Option<PathBuf>> {
    let file = std::fs::File::open(save)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| Error::invalid_data(format!("invalid save zip: {error}")))?;
    let mut entry = match archive.by_name("img.png") {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => {
            return Err(Error::invalid_data(format!("invalid save zip: {error}")));
        }
    };
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut bytes)?;
    std::fs::create_dir_all(paths.thumbnails_dir())?;
    let thumb = paths
        .thumbnails_dir()
        .join(save.file_stem().unwrap_or_default())
        .with_extension("png");
    std::fs::write(&thumb, bytes)?;
    Ok(Some(thumb))
}

/// Validates an autosave timestamp (digits only) and returns
/// `autosave_<timestamp>.zip`, matching the game's
/// `autosave_yyyyMMddHHmmss` naming (the caller supplies the local timestamp).
pub fn autosave_file_name(timestamp: &str) -> Result<String> {
    if timestamp.is_empty() || !timestamp.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::invalid_data(
            "autosave timestamp must be digits (yyyyMMddHHmmss)",
        ));
    }
    Ok(format!("autosave_{timestamp}.zip"))
}

/// Scheduler inputs mirrored from the game's `AutoSave*` user settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutosaveSchedule {
    /// Master switch.
    pub enabled: bool,
    /// Wall-clock cadence (vs sim time).
    pub real_time: bool,
    /// Minutes between autosaves.
    pub period_minutes: u32,
    /// Retained autosave files (`0` = no cleanup).
    pub max_files: usize,
}

impl AutosaveSchedule {
    /// Builds the schedule from persisted user preferences.
    #[must_use]
    pub fn from_preferences(prefs: &UserPreferences) -> Self {
        Self {
            enabled: prefs.autosave,
            real_time: prefs.autosave_real_time,
            period_minutes: prefs.autosave_period_minutes.max(1),
            max_files: prefs.autosave_max_files as usize,
        }
    }

    /// Duration between autosaves.
    #[must_use]
    pub fn interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(u64::from(self.period_minutes) * 60)
    }
}

/// Most autosave files one autosave run removes. The game's `CleanUpAutosaveFolder` deletes
/// every file beyond the limit in one go, but in steady state that is exactly one per run
/// (each run adds one). The folder is the installed game's own: a setting that was misread
/// or lowered by mistake must not wipe a large autosave history in a single pass, so a
/// larger backlog is trimmed one file per autosave instead.
pub const AUTOSAVE_MAX_REMOVALS_PER_RUN: usize = 1;

impl SaveBrowser {
    /// Writes `Savefiles/Autosaves/autosave_<timestamp>.zip` and applies the game's
    /// retention limit (`AutoSaveMaxFiles` newest files stay, plus the file just written:
    /// the game cleans up *before* it writes), returning the new file's path.
    pub fn autosave_file(
        &self,
        timestamp: &str,
        contents: &WorldContents,
        schedule: AutosaveSchedule,
    ) -> Result<PathBuf> {
        self.paths.ensure_layout()?;
        let name = autosave_file_name(timestamp)?;
        let destination = self.paths.autosaves_dir().join(name);
        save_world_contents(&destination, contents)?;
        if schedule.max_files > 0 {
            cleanup_autosaves_limited(
                &self.paths,
                schedule.max_files.saturating_add(1),
                AUTOSAVE_MAX_REMOVALS_PER_RUN,
            )?;
        }
        Ok(destination)
    }

    /// [`Self::autosave_file`], then the refreshed entry list (opens every save in the
    /// folder to read its metadata: not for the per-autosave hot path).
    pub fn autosave(
        &self,
        timestamp: &str,
        contents: &WorldContents,
        schedule: AutosaveSchedule,
    ) -> Result<(PathBuf, Vec<SaveListEntry>)> {
        let destination = self.autosave_file(timestamp, contents, schedule)?;
        Ok((destination, list_save_entries(&self.paths, true)?))
    }

    /// Load rows for the save/load browser (metadata + cached thumbnails).
    pub fn refresh_save_entries(&self, include_autosaves: bool) -> Result<Vec<SaveListEntry>> {
        list_save_entries(&self.paths, include_autosaves)
    }

    /// [`Self::refresh_save_entries`] with the metadata of the first `with_metadata` rows only (see [`list_save_entries_head`]).
    pub fn refresh_save_entries_head(
        &self,
        include_autosaves: bool,
        with_metadata: usize,
    ) -> Result<Vec<SaveListEntry>> {
        list_save_entries_head(&self.paths, include_autosaves, with_metadata)
    }

    /// Save current contents, then return the refreshed browser list (the UI
    /// can repaint immediately after save/overwrite).
    pub fn save_world_with_list(
        &self,
        name: &str,
        contents: &WorldContents,
        overwrite: bool,
        include_autosaves: bool,
    ) -> Result<(PathBuf, Vec<SaveListEntry>)> {
        let path = self.save_world(name, contents, overwrite)?;
        Ok((path, self.refresh_save_entries(include_autosaves)?))
    }

    /// Delete and return the refreshed browser rows.
    pub fn delete_save_with_list(
        &self,
        path: &Path,
        include_autosaves: bool,
    ) -> Result<Vec<SaveListEntry>> {
        delete_file(path)?;
        self.refresh_save_entries(include_autosaves)
    }

    /// Rename and return the new path + refreshed browser rows.
    pub fn rename_save_with_list(
        &self,
        path: &Path,
        new_name: &str,
        include_autosaves: bool,
    ) -> Result<(PathBuf, Vec<SaveListEntry>)> {
        let renamed = rename_file(path, new_name)?;
        Ok((renamed, self.refresh_save_entries(include_autosaves)?))
    }
}

/// Opens a folder (or selects a file) in the platform file manager. Used by the
/// load/save panels' "open folder" actions; returns an error for missing paths.
pub fn open_in_file_manager(path: &Path) -> Result<()> {
    if !path.exists() {
        return Err(Error::invalid_data("path does not exist"));
    }
    #[cfg(target_os = "windows")]
    {
        let mut command = std::process::Command::new("explorer");
        if path.is_file() {
            command.arg(format!("/select,{}", path.display()));
        } else {
            command.arg(path);
        }
        command
            .spawn()
            .map_err(|error| Error::Io(error.to_string()))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|error| Error::Io(error.to_string()))?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|error| Error::Io(error.to_string()))?;
    }
    Ok(())
}

/// Updates an existing scenario bundle's `.bb8settings` from the scenario
/// editor, preserving every other zip member/template/unknown entry.
/// The caller then refreshes scenarios via [`list_scenarios`].
pub fn update_scenario_settings(path: &Path, settings: &crate::ScenarioSettingsFile) -> Result<()> {
    let mut bundle = crate::load_scenario_bundle(path)?;
    bundle.settings_file = settings.clone();
    // Scenario writer derives the recommended/effective independent block from
    // bundle.settings; keep both representations in sync for the editor.
    if let Some(independents) = settings
        .recommended
        .clone()
        .or_else(|| settings.independents.clone())
    {
        bundle.settings.independents = independents;
    }
    crate::write_scenario_bundle(&bundle, path)
}

/// Library selector entry for a bibite template/`.bb8` genome.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateListEntry {
    /// Display name (`template.name` or file stem fallback).
    pub name: String,
    /// Template description when present.
    pub description: Option<String>,
    /// Full file path.
    pub path: PathBuf,
    /// On-disk kind (`.bb8template` vs genome-only `.bb8`).
    pub kind: TemplateKind,
    /// File size in bytes.
    pub size: u64,
    /// Last modification time.
    pub modified: Option<SystemTime>,
}

/// Kind of library entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateKind {
    /// `.bb8template` (editor template with metadata/brain anchors).
    Template,
    /// Genome-only `.bb8` file.
    Genome,
}

impl TemplateKind {
    /// Stable kind label for UI rows: template or genome.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Template => "template",
            Self::Genome => "genome",
        }
    }
}

/// Lists library entries (`Bibites/Templates/*.bb8template` and `*.bb8`) with
/// name/description/path/kind, reading only JSON header metadata. Sorted by
/// name, case-insensitive.
pub fn list_template_entries(paths: &StoragePaths) -> Result<Vec<TemplateListEntry>> {
    let dir = paths.templates_dir();
    let mut entries = Vec::new();
    let read = match std::fs::read_dir(&dir) {
        Ok(read) => read,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(entries),
        Err(error) => return Err(Error::from(error)),
    };
    for item in read {
        let item = item?;
        let path = item.path();
        if !item.file_type()?.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let kind = match ext.as_str() {
            "bb8template" => TemplateKind::Template,
            "bb8" => TemplateKind::Genome,
            _ => continue,
        };
        let metadata = item.metadata()?;
        let text = std::fs::read_to_string(&path)?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let json: serde_json::Value = serde_json::from_str(text).unwrap_or(serde_json::Value::Null);
        let stem = path
            .file_stem()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default()
            .to_string();
        let name = json
            .get("name")
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .map_or_else(|| stem.clone(), str::to_string);
        let description = json
            .get("description")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        entries.push(TemplateListEntry {
            name,
            description,
            path,
            kind,
            size: metadata.len(),
            modified: metadata.modified().ok(),
        });
    }
    entries.sort_by_key(|entry| entry.name.to_lowercase());
    Ok(entries)
}

/// One row of the Bibite Library list (`BibiteTemplateSelectorPanel.FillLists`).
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryEntry {
    /// `BibiteTemplate.name` (templates: the stored name; `.bb8`: the file name).
    pub name: String,
    /// `BibiteTemplate.description`.
    pub description: String,
    /// `Version.Parse(template.version).ToString()`.
    pub version: String,
    /// `isOfficial` flag of the file.
    pub is_official: bool,
    /// Hidden brain nodes (`nodes.Length - NInputs - NOutputs`).
    pub n_hidden_nodes: usize,
    /// Synapse count.
    pub n_synapses: usize,
    /// Full file path.
    pub path: PathBuf,
    /// `.bb8template` or `.bb8`.
    pub kind: TemplateKind,
}

/// The Bibite Library list as the game builds it: every `*.bb8template` of
/// `Bibites/Templates` followed by every `*.bb8` of `Bibites/` (directory order,
/// i.e. by file name), skipping files it cannot parse or whose version is unknown.
#[must_use]
pub fn list_library_entries(paths: &StoragePaths) -> Vec<LibraryEntry> {
    const N_INPUTS_PLUS_OUTPUTS: usize =
        bibites_core::brain::N_INPUTS + bibites_core::brain::N_OUTPUTS;
    let mut out = Vec::new();
    let groups = [
        (paths.templates_dir(), "bb8template", TemplateKind::Template),
        (paths.root.join("Bibites"), "bb8", TemplateKind::Genome),
    ];
    for (dir, extension, kind) in groups {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = read
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .extension()
                        .and_then(std::ffi::OsStr::to_str)
                        .is_some_and(|ext| ext == extension)
            })
            .collect();
        files.sort_by_key(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_uppercase())
                .unwrap_or_default()
        });
        for path in files {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
            let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
                continue;
            };
            let stem = path
                .file_stem()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or_default()
                .to_owned();
            let string = |key: &str| {
                json.get(key)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            };
            let (name, description, version_text, nodes, synapses) = match kind {
                TemplateKind::Template => {
                    // `LoadState` needs both arrays; a missing one is a parse error.
                    let (Some(nodes), Some(synapses)) = (
                        json.get("nodes").and_then(serde_json::Value::as_array),
                        json.get("synapses").and_then(serde_json::Value::as_array),
                    ) else {
                        continue;
                    };
                    (
                        string("name").unwrap_or_else(|| stem.clone()),
                        string("description").unwrap_or_default(),
                        string("version").unwrap_or_else(|| crate::version::PRESENT.to_owned()),
                        nodes.len(),
                        synapses.len(),
                    )
                }
                TemplateKind::Genome => {
                    let brain = json.get("brain");
                    let count = |key: &str| {
                        brain
                            .and_then(|brain| brain.get(key))
                            .and_then(serde_json::Value::as_array)
                            .map_or(0, Vec::len)
                    };
                    (
                        stem.clone(),
                        string("desc").unwrap_or_default(),
                        string("version").unwrap_or_default(),
                        count("Nodes"),
                        count("Synapses"),
                    )
                }
            };
            let Ok(version) = crate::version::Version::parse(&version_text) else {
                continue;
            };
            out.push(LibraryEntry {
                name,
                description,
                version: version.to_string(),
                is_official: json
                    .get("isOfficial")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                n_hidden_nodes: nodes.saturating_sub(N_INPUTS_PLUS_OUTPUTS),
                n_synapses: synapses,
                path,
                kind,
            });
        }
    }
    out
}

impl SaveBrowser {
    /// Refreshes the Library/template selector rows.
    pub fn refresh_template_entries(&self) -> Result<Vec<TemplateListEntry>> {
        list_template_entries(&self.paths)
    }

    /// Refreshes scenario rows (scenario editor save flow).
    pub fn refresh_scenario_entries(&self) -> Result<Vec<StoredFile>> {
        list_scenarios(&self.paths)
    }
}

/// Builds and writes a new scenario zip from option values. `settings` is the
/// scenario `.bb8settings` (editor state; obtainable from a parsed
/// [`crate::ScenarioSettingsFile`] or a base scenario), `scenario` the core
/// [`bibites_core::world::ScenarioData`] (zones/spawn rows/tools). The scenario
/// starts with no bundled templates/image; callers can add them via the
/// existing bundle writer/import helpers.
pub fn create_scenario(
    path: &Path,
    name: &str,
    description: &str,
    settings: &crate::ScenarioSettingsFile,
    scenario: bibites_core::world::ScenarioData,
) -> Result<()> {
    use crate::bundle::{ScenarioBundle, ScenarioInfo};
    let version = crate::present_version();
    let info = ScenarioInfo {
        name: name.to_string(),
        description: description.to_string(),
        version,
        version_text: version.to_string(),
        is_official: false,
        rank: None,
        is_challenge: false,
        stars: [None, None, None],
        extra: Default::default(),
    };
    let mut effective = bibites_core::settings::SimSettings::game_defaults();
    if let Some(independents) = settings
        .recommended
        .clone()
        .or_else(|| settings.independents.clone())
    {
        effective.independents = independents;
    }
    let bundle = ScenarioBundle {
        settings: effective,
        scenario,
        settings_file: settings.clone(),
        info,
        templates: Vec::new(),
        template_bytes: Vec::new(),
        img_png: None,
        extra_entries: Vec::new(),
    };
    crate::write_scenario_bundle(&bundle, path)
}

/// Loads a scenario for a **new game**: returns the scenario's zones/spawn
/// rows/tools/settings with an explicit world seed. The caller (viewer/world)
/// then builds a fresh `World` and runs its new-game spawner; io deliberately
/// does not own simulation startup.
pub fn new_game_contents(
    scenario_path: &Path,
    seed: u64,
) -> Result<bibites_core::world::WorldContents> {
    let mut contents = crate::load_scenario_world_contents(scenario_path, None)?;
    contents.seed = seed;
    Ok(contents)
}

/// Local (no-Steam) workshop-like content browser. Files/folders live under
/// `<root>/Workshop/`; scenarios/templates can also be imported via the normal
/// `import_scenario`/`import_template` helpers (Steam Workshop is out of scope).
#[derive(Debug, Clone, PartialEq)]
pub struct WorkshopEntry {
    /// Display name (file/folder name).
    pub name: String,
    /// Full path.
    pub path: PathBuf,
    /// Kind by extension (`archive` for .zip, `template`, `genome`, `folder`,
    /// `other`).
    pub kind: String,
    /// File size in bytes (`0` for folders).
    pub size: u64,
    /// Last modification time.
    pub modified: Option<SystemTime>,
    /// Whether the entry is a directory (items copied recursively import/export).
    pub folder: bool,
}

impl StoragePaths {
    /// `<state>/Workshop/` local workshop-like content folder (the game's own
    /// `Workshop/Sharing` staging folder is not touched).
    #[must_use]
    pub fn workshop_dir(&self) -> PathBuf {
        self.state.join("Workshop")
    }
}

/// Lists local workshop items (`Workshop/*`, one level; folders shown as items).
pub fn list_workshop_entries(paths: &StoragePaths) -> Result<Vec<WorkshopEntry>> {
    let dir = paths.workshop_dir();
    let mut entries = Vec::new();
    let read = match std::fs::read_dir(&dir) {
        Ok(read) => read,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(entries),
        Err(error) => return Err(Error::from(error)),
    };
    for item in read {
        let item = item?;
        let path = item.path();
        let metadata = item.metadata()?;
        let folder = metadata.is_dir();
        let ext = path
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let kind = if folder {
            "folder"
        } else {
            match ext.as_str() {
                "zip" => "archive",
                "bb8template" => "template",
                "bb8" => "genome",
                _ => "other",
            }
        };
        entries.push(WorkshopEntry {
            name: path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or_default()
                .to_string(),
            path,
            kind: kind.to_string(),
            size: if folder { 0 } else { metadata.len() },
            modified: metadata.modified().ok(),
            folder,
        });
    }
    entries.sort_by_key(|entry| entry.name.to_lowercase());
    Ok(entries)
}

/// Copies a file or folder into `Workshop/<name>` (recursive for folders).
pub fn import_workshop_item(
    source: &Path,
    paths: &StoragePaths,
    name: &str,
    overwrite: bool,
) -> Result<PathBuf> {
    let file_name = sanitize_file_name(
        name,
        source
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or(""),
    )?;
    let destination = paths.workshop_dir().join(file_name);
    if destination.exists() && !overwrite {
        return Err(Error::invalid_data("workshop item already exists"));
    }
    paths.ensure_layout()?;
    std::fs::create_dir_all(paths.workshop_dir())?;
    copy_item(source, &destination)?;
    Ok(destination)
}

/// Exports a workshop item (file/folder) to a user-chosen path.
pub fn export_workshop_item(source: &Path, destination: &Path) -> Result<()> {
    ensure_parent(destination)?;
    copy_item(source, destination)
}

/// Deletes a workshop item (file or recursively a folder).
pub fn delete_workshop_item(path: &Path) -> Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn copy_item(source: &Path, destination: &Path) -> Result<()> {
    if source.is_dir() {
        std::fs::create_dir_all(destination)?;
        for item in std::fs::read_dir(source)? {
            let item = item?;
            copy_item(&item.path(), &destination.join(item.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(source, destination)?;
        Ok(())
    }
}

impl SaveBrowser {
    /// Refreshes the local workshop-like browser.
    pub fn refresh_workshop_entries(&self) -> Result<Vec<WorkshopEntry>> {
        list_workshop_entries(&self.paths)
    }

    /// Imports a local workshop file/folder and returns the refreshed list.
    pub fn import_workshop(
        &self,
        source: &Path,
        name: &str,
        overwrite: bool,
    ) -> Result<(PathBuf, Vec<WorkshopEntry>)> {
        let path = import_workshop_item(source, &self.paths, name, overwrite)?;
        Ok((path, self.refresh_workshop_entries()?))
    }

    /// Deletes a local workshop item and returns the refreshed list.
    pub fn delete_workshop(&self, path: &Path) -> Result<Vec<WorkshopEntry>> {
        delete_workshop_item(path)?;
        self.refresh_workshop_entries()
    }
}

/// Local workshop item metadata (`Workshop/<item>/workshop.json`), optional for
/// no-Steam local packs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WorkshopItemMetadata {
    /// Display name override (empty = item folder/file name).
    pub name: String,
    /// Description shown in the local browser.
    pub description: String,
    /// Free-form tags.
    pub tags: Vec<String>,
}

/// Metadata path for a workshop item (inside folders, sidecar for files).
#[must_use]
pub fn workshop_metadata_path(item: &Path) -> PathBuf {
    if item.is_dir() {
        item.join("workshop.json")
    } else {
        item.with_extension("workshop.json")
    }
}

/// Loads optional workshop metadata; missing file returns defaults.
pub fn load_workshop_metadata(item: &Path) -> Result<WorkshopItemMetadata> {
    let path = workshop_metadata_path(item);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(WorkshopItemMetadata::default());
        }
        Err(error) => return Err(Error::from(error)),
    };
    serde_json::from_str(&text)
        .map_err(|error| Error::invalid_data(format!("invalid workshop metadata: {error}")))
}

/// Atomically writes workshop item metadata.
pub fn save_workshop_metadata(item: &Path, metadata: &WorkshopItemMetadata) -> Result<()> {
    let path = workshop_metadata_path(item);
    ensure_parent(&path)?;
    let text = serde_json::to_string_pretty(metadata)
        .map_err(|error| Error::invalid_data(error.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Renames a local workshop item (file or folder) inside `Workshop/`.
pub fn rename_workshop_item(item: &Path, new_name: &str) -> Result<PathBuf> {
    let trimmed = new_name.trim();
    if trimmed.is_empty()
        || trimmed.chars().any(|c| {
            matches!(
                c,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'
            )
        })
    {
        return Err(Error::invalid_data("invalid workshop item name"));
    }
    let destination = item.with_file_name(trimmed);
    if destination.exists() && destination != item {
        return Err(Error::invalid_data("workshop destination already exists"));
    }
    std::fs::rename(item, &destination)?;
    Ok(destination)
}

impl SaveBrowser {
    /// Renames a local workshop item and returns refreshed rows.
    pub fn rename_workshop(
        &self,
        item: &Path,
        new_name: &str,
    ) -> Result<(PathBuf, Vec<WorkshopEntry>)> {
        let path = rename_workshop_item(item, new_name)?;
        Ok((path, self.refresh_workshop_entries()?))
    }
}

/// Stored best result of one challenge (the `PlayerPrefs` keys
/// `<key>Star|Score|ChampionName|ChampionPath|Time` of `ChallengesProgress`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StoredChallengeScore {
    /// Stars attained.
    pub stars: u8,
    /// Best score (`None` when never scored; the game stores NaN).
    pub score: Option<f32>,
    /// Champion display name.
    pub champion_name: String,
    /// Champion template path.
    pub champion_path: String,
    /// ISO-8601 UTC time of the result.
    pub time: String,
}

impl StoredChallengeScore {
    fn to_info(&self, name: &str) -> bibites_core::world::ChallengeScoreInfo {
        bibites_core::world::ChallengeScoreInfo {
            challenge_name: name.to_owned(),
            star_attained: self.stars,
            score: self.score.unwrap_or(f32::NAN),
            elapsed_s: 0.0,
            champion_name: self.champion_name.clone(),
            champion_path: self.champion_path.clone(),
        }
    }
}

/// What this port stores of the achievements: the ids that are unlocked and the counters behind some of them (`Counter`: how many bibites
/// the editor saved, `BibiteEditorSaved`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AchievementsFile {
    /// The unlocked achievements (`ACH_*`).
    #[serde(default)]
    pub unlocked: std::collections::BTreeSet<String>,
    /// Counters by name.
    #[serde(default)]
    pub counters: BTreeMap<String, u64>,
}

/// Reads `achievements.json` (nothing stored yet: empty).
pub fn read_achievements(paths: &StoragePaths) -> Result<AchievementsFile> {
    match std::fs::read_to_string(paths.achievements_path()) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|error| Error::invalid_data(format!("invalid achievements.json: {error}"))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(AchievementsFile::default()),
        Err(error) => Err(Error::from(error)),
    }
}

/// Writes `achievements.json` (through a temporary file, so a crash leaves the old one).
pub fn write_achievements(paths: &StoragePaths, file: &AchievementsFile) -> Result<()> {
    let path = paths.achievements_path();
    ensure_parent(&path)?;
    let text = serde_json::to_string_pretty(file)
        .map_err(|error| Error::invalid_data(error.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Outcome of [`record_challenge_result`].
#[derive(Debug, Clone, PartialEq)]
pub struct ChallengeRecordOutcome {
    /// The stored best before this result (`None`: never completed).
    pub previous: Option<StoredChallengeScore>,
    /// Whether this result replaced the stored best.
    pub improved: bool,
}

type ChallengeProgressFile = BTreeMap<String, StoredChallengeScore>;

fn read_challenge_progress(paths: &StoragePaths) -> Result<ChallengeProgressFile> {
    match std::fs::read_to_string(paths.challenge_progress_path()) {
        Ok(text) => serde_json::from_str(&text).map_err(|error| {
            Error::invalid_data(format!("invalid challenge_progress.json: {error}"))
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(Error::from(error)),
    }
}

/// `ChallengesProgress.GetHighScoreOfChallenge`: the stored best for a challenge.
pub fn challenge_high_score(
    paths: &StoragePaths,
    challenge_name: &str,
) -> Result<Option<StoredChallengeScore>> {
    let key = bibites_core::world::ChallengeScoreInfo::name_to_key(challenge_name);
    Ok(read_challenge_progress(paths)?.remove(&key))
}

/// The best result of a challenge as the installed game stored it in its
/// `PlayerPrefs` (`<key>Star|Score|ChampionName|ChampionPath|Time`); `None` when
/// the game never recorded one.
#[must_use]
pub fn challenge_score_from_prefs(
    prefs: &crate::prefs::GamePrefs,
    challenge_name: &str,
) -> Option<StoredChallengeScore> {
    let key = bibites_core::world::ChallengeScoreInfo::name_to_key(challenge_name);
    let stars = prefs
        .int(&format!("{key}Star"))
        .map_or(0, |value| value.clamp(0, 3) as u8);
    let score = prefs
        .float(&format!("{key}Score"))
        .map(|value| value as f32)
        .filter(|value| value.is_finite());
    if stars == 0 && score.is_none() {
        return None;
    }
    Some(StoredChallengeScore {
        stars,
        score,
        champion_name: prefs
            .text(&format!("{key}ChampionName"))
            .unwrap_or_default()
            .to_owned(),
        champion_path: prefs
            .text(&format!("{key}ChampionPath"))
            .unwrap_or_default()
            .to_owned(),
        time: prefs
            .text(&format!("{key}Time"))
            .unwrap_or_default()
            .to_owned(),
    })
}

/// The stored best from this port's own file, or (when it has none) from the
/// installed game's preferences.
pub fn challenge_high_score_with_prefs(
    paths: &StoragePaths,
    challenge_name: &str,
    prefs: &crate::prefs::GamePrefs,
    high_is_better: bool,
) -> Result<Option<StoredChallengeScore>> {
    let own = challenge_high_score(paths, challenge_name)?;
    Ok(better_stored(
        own,
        challenge_score_from_prefs(prefs, challenge_name),
        challenge_name,
        high_is_better,
    ))
}

fn better_stored(
    own: Option<StoredChallengeScore>,
    game: Option<StoredChallengeScore>,
    challenge_name: &str,
    high_is_better: bool,
) -> Option<StoredChallengeScore> {
    match (own, game) {
        (Some(own), Some(game)) => {
            let game_info = game.to_info(challenge_name);
            if game_info.beats(Some(&own.to_info(challenge_name)), high_is_better) {
                Some(game)
            } else {
                Some(own)
            }
        }
        (own, game) => own.or(game),
    }
}

/// `ChallengesProgress.TrySetHighScoreOfChallenge`: stores `info` when it beats
/// the stored best (`high_is_better` from the challenge's parameters).
pub fn record_challenge_result(
    paths: &StoragePaths,
    info: &bibites_core::world::ChallengeScoreInfo,
    high_is_better: bool,
    time_iso: &str,
) -> Result<ChallengeRecordOutcome> {
    record_challenge_result_over(paths, info, high_is_better, time_iso, None)
}

/// [`record_challenge_result`] that also treats the installed game's stored best
/// (`prefs`) as the previous result, so a result is only "new" when it beats
/// what the player already achieved in the game. Writes only to `paths`.
pub fn record_challenge_result_over(
    paths: &StoragePaths,
    info: &bibites_core::world::ChallengeScoreInfo,
    high_is_better: bool,
    time_iso: &str,
    prefs: Option<&crate::prefs::GamePrefs>,
) -> Result<ChallengeRecordOutcome> {
    let key = bibites_core::world::ChallengeScoreInfo::name_to_key(&info.challenge_name);
    let mut progress = read_challenge_progress(paths)?;
    let previous = better_stored(
        progress.get(&key).cloned(),
        prefs.and_then(|prefs| challenge_score_from_prefs(prefs, &info.challenge_name)),
        &info.challenge_name,
        high_is_better,
    );
    let previous_info = previous
        .as_ref()
        .map(|stored| stored.to_info(&info.challenge_name));
    let improved = info.beats(previous_info.as_ref(), high_is_better);
    if improved {
        progress.insert(
            key,
            StoredChallengeScore {
                stars: info.star_attained,
                score: Some(info.score),
                champion_name: info.champion_name.clone(),
                champion_path: info.champion_path.clone(),
                time: time_iso.to_owned(),
            },
        );
        let path = paths.challenge_progress_path();
        ensure_parent(&path)?;
        let text = serde_json::to_string_pretty(&progress)
            .map_err(|error| Error::invalid_data(error.to_string()))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)?;
    }
    Ok(ChallengeRecordOutcome { previous, improved })
}

//! The in-world scenario saver (`ScenarioSaver`, opened by the Save button of the settings panel, `DynamicSettingsPanel.OpenScenarioSaver`): a name, a
//! description and the picture of the world, written as `Scenarios/<name>.zip` with the world's settings, zones, zone groups, default bibites and settings
//! animators (`ScenarioSaver.SaveScenario` -> `SerializationHelper.SerializeScenario(saveRecommendations: true)`: none of the entities).
//!
//! The form is the one the title menu's scenario editor saves with (the same `ScenarioSaver` prefab: `UiRuntime::emit_saver_form`). While it is open the game
//! is paused and nothing behind it answers; Escape and the exit button close it; a name that is taken asks before it overwrites (as the editor does); a name of
//! a shipped scenario or with a character a file name cannot have cannot be saved.

use std::path::PathBuf;
use std::sync::Arc;

use crate::app::{App, ConfirmKind, FrameInput, GAME_VERSION};
use crate::save_dialog::is_invalid_file_char;
use crate::text_field::Caret;
use crate::ui::UiTexture;

/// The text field with the focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// "Scenario Name".
    Name,
    /// "Scenario Description".
    Description,
}

/// What `UiAction::ScenarioSaver` carries: the name field, the description field, Save and the exit button.
pub const ACTION_NAME: u8 = 0;
/// See [`ACTION_NAME`].
pub const ACTION_DESCRIPTION: u8 = 1;
/// See [`ACTION_NAME`].
pub const ACTION_SAVE: u8 = 2;
/// See [`ACTION_NAME`].
pub const ACTION_CLOSE: u8 = 3;

/// The open saver.
#[derive(Clone)]
pub struct ScenarioSaver {
    /// The scenario's name (the file name).
    pub name: String,
    /// The scenario's description.
    pub description: String,
    /// The field typing goes to.
    pub focus: Field,
    /// The caret of the focused field.
    pub caret: Caret,
    /// The game's pause before the saver locked it.
    pub previous_pause: bool,
    /// The picture the scenario carries (`img.png`).
    pub picture_png: Option<Vec<u8>>,
    /// The same picture as the form draws it.
    pub picture: Option<Arc<UiTexture>>,
    /// The names (lower case) of the shipped scenarios, which the saver refuses (`GameManager.defaultScenarios`).
    pub official: Vec<String>,
}

impl std::fmt::Debug for ScenarioSaver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScenarioSaver").field("name", &self.name).field("focus", &self.focus).finish_non_exhaustive()
    }
}

impl ScenarioSaver {
    /// Why the name cannot be used (`ScenarioSaver.AssertForm`): `"invalid"` for an empty name or one with a character a file name cannot have,
    /// `"official"` for the name of a shipped scenario.
    #[must_use]
    pub fn name_problem(&self) -> Option<&'static str> {
        if self.name.is_empty() || self.name.chars().any(is_invalid_file_char) {
            return Some("invalid");
        }
        let lower = self.name.to_lowercase();
        self.official.iter().any(|name| *name == lower).then_some("official")
    }

    /// The Save button is interactable.
    #[must_use]
    pub fn can_save(&self) -> bool {
        self.name_problem().is_none()
    }

    fn focused_text(&mut self) -> &mut String {
        match self.focus {
            Field::Name => &mut self.name,
            Field::Description => &mut self.description,
        }
    }

    /// Typed characters go to the focused field (control characters are dropped).
    pub fn type_text(&mut self, text: &str) {
        let mut caret = self.caret;
        caret.clamp(self.focused_text());
        caret.insert(self.focused_text(), text);
        self.caret = caret;
    }

    /// Backspace in the focused field.
    pub fn backspace(&mut self) {
        let mut caret = self.caret;
        caret.clamp(self.focused_text());
        caret.backspace(self.focused_text());
        self.caret = caret;
    }

    /// The caret keys (and Delete / Ctrl + A) in the focused field.
    pub fn keys(&mut self, keys: crate::text_field::EditKeys) {
        let mut caret = self.caret;
        crate::text_field::edit(self.focused_text(), &mut caret, "", usize::MAX, keys);
        self.caret = caret;
    }

    /// Puts the caret at the end of the field that has the focus.
    pub fn focus_field(&mut self, field: Field) {
        self.focus = field;
        let end = Caret::at_end(self.focused_text());
        self.caret = end;
    }

    /// Enter: a new line in the multi-line description; the name field has none.
    pub fn enter(&mut self) {
        if self.focus == Field::Description {
            let mut caret = self.caret;
            caret.clamp(&self.description);
            caret.insert_newline(&mut self.description);
            self.caret = caret;
        }
    }
}

impl App {
    /// `DynamicSettingsPanel.OpenScenarioSaver`: the form opens over the world, which pauses.
    pub(crate) fn open_scenario_saver(&mut self) {
        if self.scenario_saver.is_some() {
            return;
        }
        // The shipped scenarios cannot be overwritten: their names are read from the Scenarios folder.
        let official: Vec<String> = bibites_io::storage::list_scenarios(&self.storage)
            .map(|files| {
                files
                    .into_iter()
                    .filter_map(|file| match bibites_io::read_scenario_card(&file.path) {
                        Ok(Some(card)) if card.info.is_official => Some(card.info.name.to_lowercase()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        // `SceneScreenShotHandler`: the picture of the whole world as it is now.
        let image = self.world_screenshot();
        let picture_png = image.as_ref().and_then(|image| image.to_png_bytes().ok());
        let picture = image.map(|image| Arc::new(UiTexture { width: image.width, height: image.height, rgba: image.rgba }));
        self.scenario_saver = Some(ScenarioSaver {
            name: String::new(),
            description: String::new(),
            focus: Field::Name,
            caret: Caret { pos: usize::MAX, anchor: None },
            previous_pause: self.paused,
            picture_png,
            picture,
            official,
        });
        self.paused = true;
        self.accumulator = 0.0;
    }

    /// The form closes and the pause it held ends.
    pub(crate) fn close_scenario_saver(&mut self) {
        if let Some(saver) = self.scenario_saver.take() {
            self.paused = saver.previous_pause;
            self.accumulator = 0.0;
        }
        if matches!(self.confirm_dialog, Some(ConfirmKind::OverwriteScenario(_))) {
            self.confirm_dialog = None;
        }
    }

    /// A control of the form was pressed.
    pub(crate) fn scenario_saver_action(&mut self, code: u8) {
        match code {
            ACTION_NAME => {
                if let Some(saver) = self.scenario_saver.as_mut() {
                    saver.focus_field(Field::Name);
                }
            }
            ACTION_DESCRIPTION => {
                if let Some(saver) = self.scenario_saver.as_mut() {
                    saver.focus_field(Field::Description);
                }
            }
            ACTION_SAVE => self.scenario_saver_save(),
            ACTION_CLOSE => self.close_scenario_saver(),
            _ => {}
        }
    }

    /// `Scenarios/<name>.zip` of the folder the world's files go to.
    pub(crate) fn scenario_path(&self, name: &str) -> PathBuf {
        self.storage.scenarios_dir().join(format!("{name}.zip"))
    }

    /// A user scenario with the typed name exists already (the form warns under the name).
    #[must_use]
    pub fn scenario_saver_name_exists(&self) -> bool {
        self.scenario_saver
            .as_ref()
            .is_some_and(|saver| saver.can_save() && self.scenario_path(&saver.name).is_file())
    }

    /// The Save button: a name that is taken asks first (`EditorPopup::Overwrite`'s wording); a free one is written at once. A button that is not
    /// interactable does nothing.
    pub(crate) fn scenario_saver_save(&mut self) {
        let Some(saver) = self.scenario_saver.as_ref() else {
            return;
        };
        if !saver.can_save() {
            return;
        }
        let name = saver.name.clone();
        if self.scenario_path(&name).is_file() {
            self.confirm_do_not_show = false;
            self.confirm_dialog = Some(ConfirmKind::OverwriteScenario(name));
            self.last_action = "confirm: overwrite scenario?".to_owned();
        } else {
            self.scenario_saver_write(&name);
        }
    }

    /// Writes the scenario (`ScenarioSaver.SaveScenario`): the settings, zones, zone groups, default bibites and animators of the world, the templates the
    /// default bibites name (`SaveSystem.AddTemplatesToArchive`) and the picture. The form closes when it was written.
    pub(crate) fn scenario_saver_write(&mut self, name: &str) {
        let Some(saver) = self.scenario_saver.as_ref() else {
            return;
        };
        let (description, picture) = (saver.description.clone(), saver.picture_png.clone());
        if let Err(error) = self.storage.ensure_layout() {
            self.last_action = format!("save scenario failed: {error}");
            return;
        }
        let contents = self.world.to_contents();
        // The templates the default bibites are made from travel with the scenario (a template the game ships is not in the folder's list of
        // the player's own, and is found by its name).
        let mut template_bytes: Vec<(String, Vec<u8>)> = Vec::new();
        for row in &contents.scene.bibite_spawns {
            let file_name = row.path.rsplit(['/', '\\']).next().unwrap_or(&row.path).to_owned();
            if file_name.is_empty() || template_bytes.iter().any(|(entry, _)| *entry == format!("templates/{file_name}")) {
                continue;
            }
            let candidates = [PathBuf::from(&row.path), self.storage.templates_dir().join(&file_name), self.storage.bibites_dir().join(&file_name)];
            if let Some(bytes) = candidates.iter().filter(|path| path.is_file()).find_map(|path| std::fs::read(path).ok()) {
                template_bytes.push((format!("templates/{file_name}"), bytes));
            }
        }
        let path = self.scenario_path(name);
        let result = bibites_io::scenario_bundle_from_contents(&contents, name, &description, GAME_VERSION, picture, template_bytes)
            .and_then(|bundle| bibites_io::write_scenario_bundle(&bundle, &path));
        match result {
            Ok(()) => {
                self.last_action = format!("saved scenario {}", path.display());
                self.close_scenario_saver();
            }
            Err(error) => self.last_action = format!("save scenario failed: {error}"),
        }
    }

    /// Typing into the form: characters and Backspace edit the focused field, Enter breaks a line of the description, Ctrl + V pastes.
    pub(crate) fn edit_scenario_saver(&mut self, input: &FrameInput) {
        // Only an open form takes the typed characters and the clipboard; the overwrite question over it takes the keyboard too (the typed
        // characters are dropped, not held for later).
        if self.scenario_saver.is_none() {
            return;
        }
        let typed = std::mem::take(&mut self.typed);
        let pasted = if input.paste_tag { self.clipboard_in.take().unwrap_or_default() } else { String::new() };
        if self.confirm_dialog.is_some() {
            return;
        }
        let keys = self.edit_keys(input);
        let Some(saver) = self.scenario_saver.as_mut() else {
            return;
        };
        saver.type_text(&typed);
        saver.type_text(&pasted);
        if input.backspace {
            saver.backspace();
        }
        saver.keys(crate::text_field::EditKeys { backspace: false, ..keys });
        if input.enter {
            saver.enter();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LoadedWorld;
    use crate::ui::{ControlCommand, UiAction};
    use bibites_core::math::Vec2;
    use bibites_io::storage::StoragePaths;

    /// An app on a starter world whose storage is a folder of its own.
    fn app_in(name: &str) -> (App, PathBuf) {
        let dir = std::env::temp_dir().join(format!("rustibites-scenariosaver-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let world = LoadedWorld::load(None, 7, 12, 1).expect("starter world");
        let mut app = App::new(world, Vec2::new(1280.0, 800.0), None);
        app.storage = StoragePaths::new(dir.clone());
        app.user_settings_path = Some(dir.join("settings.json"));
        (app, dir)
    }

    fn idle() -> FrameInput {
        FrameInput { dt: 0.016, ..FrameInput::default() }
    }

    fn type_in(app: &mut App, text: &str) {
        app.typed = text.to_owned();
        app.handle_input(&idle());
    }

    #[test]
    fn the_save_button_opens_the_form_and_the_world_waits_for_it() {
        let (mut app, dir) = app_in("open");
        assert!(!app.paused);
        app.apply_ui_actions(&[UiAction::DynSaveScenario]);
        let saver = app.scenario_saver.as_ref().expect("the form");
        assert!(app.paused, "the world pauses while the form is open");
        assert_eq!(saver.name, "");
        assert!(!saver.can_save(), "an empty name cannot be saved");
        let picture = saver.picture.as_ref().expect("the picture of the world");
        assert_eq!((picture.width, picture.height), (400, 400));
        // Escape closes it, and the pause it held ends.
        app.handle_input(&FrameInput { quit: true, ..idle() });
        assert!(app.scenario_saver.is_none() && !app.paused);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn saving_writes_a_scenario_that_starts_a_world() {
        let (mut app, dir) = app_in("write");
        app.apply_ui_actions(&[UiAction::DynSaveScenario]);
        type_in(&mut app, "Test Oasis");
        app.apply_ui_actions(&[UiAction::ScenarioSaver(ACTION_DESCRIPTION)]);
        type_in(&mut app, "Dry lands");
        assert_eq!(app.scenario_saver.as_ref().expect("form").name, "Test Oasis");
        app.apply_ui_actions(&[UiAction::ScenarioSaver(ACTION_SAVE)]);
        let path = dir.join("Scenarios").join("Test Oasis.zip");
        assert!(path.is_file(), "{}", app.last_action);
        assert!(app.scenario_saver.is_none() && !app.paused, "the form closed");
        let bundle = bibites_io::load_scenario_bundle(&path).expect("the game's own loader reads it");
        assert_eq!(bundle.info.name, "Test Oasis");
        assert_eq!(bundle.info.description, "Dry lands");
        assert_eq!(bundle.info.version_text, GAME_VERSION);
        assert!(!bundle.info.is_official);
        let png = bundle.img_png.as_ref().expect("img.png");
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        assert_eq!(bundle.scenario.zones.len(), app.world.zones.explicit_count());
        assert_eq!(bundle.scenario.zone_groups.len(), app.world.zone_groups.len());
        assert_eq!(bundle.settings.scenario.pellet_energy, app.world.settings.scenario.pellet_energy);
        // It starts a world.
        bibites_core::world::World::new(bundle.settings, bundle.scenario, 3).expect("a world from the saved scenario");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_name_that_is_taken_asks_before_it_overwrites() {
        let (mut app, dir) = app_in("overwrite");
        app.apply_ui_actions(&[UiAction::DynSaveScenario]);
        type_in(&mut app, "Twice");
        app.apply_ui_actions(&[UiAction::ScenarioSaver(ACTION_SAVE)]);
        let path = dir.join("Scenarios").join("Twice.zip");
        assert!(path.is_file());
        std::fs::write(&path, b"old").expect("a marker");
        app.apply_ui_actions(&[UiAction::DynSaveScenario]);
        type_in(&mut app, "Twice");
        assert!(app.scenario_saver_name_exists(), "the form warns under the name");
        app.apply_ui_actions(&[UiAction::ScenarioSaver(ACTION_SAVE)]);
        assert_eq!(app.confirm_dialog, Some(ConfirmKind::OverwriteScenario("Twice".to_owned())));
        assert_eq!(std::fs::read(&path).expect("file"), b"old", "nothing is written before the answer");
        // Cancel keeps the form and the file.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::Cancel)]);
        assert!(app.scenario_saver.is_some() && app.confirm_dialog.is_none());
        assert_eq!(std::fs::read(&path).expect("file"), b"old");
        // Yes writes it and closes the form.
        app.apply_ui_actions(&[UiAction::ScenarioSaver(ACTION_SAVE)]);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::Confirm)]);
        assert!(app.scenario_saver.is_none());
        assert!(bibites_io::load_scenario_bundle(&path).is_ok(), "the old marker was replaced");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_bad_name_and_the_name_of_a_shipped_scenario_cannot_be_saved() {
        let (mut app, dir) = app_in("refused");
        app.apply_ui_actions(&[UiAction::DynSaveScenario]);
        type_in(&mut app, "a/b");
        assert_eq!(app.scenario_saver.as_ref().expect("form").name_problem(), Some("invalid"));
        app.apply_ui_actions(&[UiAction::ScenarioSaver(ACTION_SAVE)]);
        assert!(!dir.join("Scenarios").exists() || std::fs::read_dir(dir.join("Scenarios")).expect("dir").next().is_none());
        assert!(app.scenario_saver.is_some(), "the form stays");
        let saver = app.scenario_saver.as_mut().expect("form");
        saver.name = "Default".to_owned();
        saver.official = vec!["default".to_owned()];
        assert_eq!(saver.name_problem(), Some("official"));
        app.apply_ui_actions(&[UiAction::ScenarioSaver(ACTION_SAVE)]);
        assert!(app.scenario_saver.is_some() && app.confirm_dialog.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn typing_goes_to_the_focused_field_and_space_does_not_pause_the_world() {
        let (mut app, dir) = app_in("typing");
        app.apply_ui_actions(&[UiAction::DynSaveScenario]);
        // Open the form while the world runs: a space typed into the name is a character, not the pause key.
        app.paused = false;
        app.accumulator = 0.0;
        app.handle_input(&FrameInput { toggle_pause: true, ..idle() });
        assert!(!app.paused, "the pause key is ignored while the form is open");
        type_in(&mut app, "a b");
        assert_eq!(app.scenario_saver.as_ref().expect("form").name, "a b");
        app.apply_ui_actions(&[UiAction::ScenarioSaver(ACTION_DESCRIPTION)]);
        type_in(&mut app, "x");
        app.handle_input(&FrameInput { enter: true, ..idle() });
        type_in(&mut app, "y");
        assert_eq!(app.scenario_saver.as_ref().expect("form").description, "x\ny");
        let _ = std::fs::remove_dir_all(dir);
    }
}

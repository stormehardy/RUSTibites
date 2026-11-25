//! The scenario editor's side of the title menu: opening it from the New Game screen
//! (`ScenarioSelectorPanel.NewScenario` / `EditScenario`), the editor's commands (start, back,
//! save) and the keyboard input of its save form (`ScenarioSaver`).

use std::path::{Path, PathBuf};

use bibites_core::math::Vec2;

use super::{MenuAction, MenuState, MenuView};
use crate::scenario_editor::{EditorPopup, EditorState};
use crate::ui::{MenuCommand, UiAction};

/// `Path.GetInvalidFileNameChars()` on Windows (control characters are rejected separately).
const INVALID_FILE_CHARS: [char; 9] = ['"', '<', '>', '|', ':', '*', '?', '\\', '/'];

impl MenuState {
    /// The scenario draft being edited.
    #[must_use]
    pub fn editor(&self) -> Option<&EditorState> {
        self.editor.as_deref()
    }

    /// `ScenarioSelectorPanel.NewScenario` / `EditScenario`: opens the editor on the Default
    /// scenario (`new`) or on the selected one, with the New Game panel's modifiers applied.
    pub(super) fn open_editor(&mut self, new: bool) {
        let entry = if new {
            self.scenario_entries
                .iter()
                .find(|entry| entry.is_official && entry.name == "Default")
                .or_else(|| {
                    self.scenario_entries
                        .iter()
                        .find(|entry| !entry.is_challenge)
                })
        } else {
            self.selected_entry
                .and_then(|index| self.scenario_entries.get(index))
        };
        let Some(entry) = entry.cloned() else {
            self.status = "No scenario selected".to_owned();
            return;
        };
        let bundle = match bibites_io::load_scenario_bundle(&entry.path) {
            Ok(bundle) => bundle,
            Err(error) => {
                self.status = format!("Could not open {}: {error}", entry.path.display());
                return;
            }
        };
        let mut settings = bibites_io::world::scenario_effective_settings(&bundle, None);
        self.apply_newgame_modifiers(&mut settings);
        let (title, description) = if new {
            ("New Scenario".to_owned(), String::new())
        } else {
            (entry.name.clone(), entry.description.clone())
        };
        let mut editor = EditorState::new(
            entry.path.clone(),
            settings,
            bundle.scenario.clone(),
            bundle.settings_file.challenge.clone(),
            title,
            description,
        );
        editor.dyn_state.tab = editor.tab;
        self.editor = Some(Box::new(editor));
        self.view = MenuView::ScenarioEditor;
        self.dropdown = None;
        self.status.clear();
    }

    /// Opens the editor on the Default scenario (deterministic screenshots).
    pub fn open_editor_for_capture(&mut self) {
        self.open_editor(true);
    }

    /// The `+` of list `list` (deterministic screenshots).
    pub fn editor_add_for_capture(&mut self, list: u8) {
        if let Some(editor) = self.editor.as_deref_mut() {
            let _ = editor.add_item(list);
        }
    }

    /// Gives the first animator a base: choice `choice` of its "Add Setting" dropdown (deterministic screenshots).
    pub fn editor_base_for_capture(&mut self, choice: usize) {
        if let Some(editor) = self.editor.as_deref_mut() {
            let _ = editor.animator_pick(crate::ui::DropdownTarget::AnimatorBase(0), choice);
        }
    }

    /// The creator's preset (deterministic screenshots).
    pub fn editor_creator_preset_for_capture(&mut self, preset: usize) {
        if let Some(editor) = self.editor.as_deref_mut() {
            let _ = editor.animator_pick(crate::ui::DropdownTarget::AnimatorPreset, preset.min(2));
        }
    }

    /// Picks point `point` of animator `changer`'s curve (deterministic screenshots).
    pub fn editor_pick_point_for_capture(&mut self, changer: usize, point: usize) {
        if let Some(editor) = self.editor.as_deref_mut() {
            editor.animator.point = Some((changer, point));
        }
    }

    /// Leaves the editor without starting or saving (`ScenariosEditor.ActuallyClosePanel`).
    fn close_editor(&mut self) {
        self.editor = None;
        self.view = MenuView::NewGame;
    }

    /// Esc in the editor: closes the open dialog, or asks before leaving.
    pub(super) fn editor_escape(&mut self) {
        let Some(editor) = self.editor.as_deref_mut() else {
            self.back_to_main();
            return;
        };
        // Escape closes an open text field first.
        if editor.popup.is_none() && editor.cancel_edit() {
            return;
        }
        editor.popup = match editor.popup {
            None => Some(EditorPopup::ExitWarning),
            Some(EditorPopup::Overwrite) => Some(EditorPopup::Save { focus: 0 }),
            Some(_) => None,
        };
    }

    /// Commands emitted by the editor's header buttons and dialogs.
    pub(super) fn editor_command(&mut self, command: MenuCommand) -> MenuAction {
        let Some(editor) = self.editor.as_deref_mut() else {
            return MenuAction::None;
        };
        match command {
            MenuCommand::EditorStart => {
                // `PopupManager.DisplayActionWarning(startScenarioAfterEdit)` is skipped once
                // the player ticked "do not show again" (here or in the game).
                if self
                    .user_settings
                    .warning_off(crate::warnings::ActionWarning::StartScenarioAfterEdit)
                {
                    return self.editor_start();
                }
                editor.popup = Some(EditorPopup::StartWarning);
                self.confirm_checked = false;
            }
            MenuCommand::EditorBack => editor.popup = Some(EditorPopup::ExitWarning),
            MenuCommand::EditorSave => {
                editor.save_name = editor.title.clone();
                editor.save_description = editor.description.clone();
                editor.popup = Some(EditorPopup::Save { focus: 0 });
            }
            // `DefaultBibitesPanel.AddBibiteButtonPressed`: the template selector opens and the
            // chosen template becomes a new default bibite.
            MenuCommand::EditorAdd(2) => self.begin_pick(super::TemplatePick::DefaultBibite),
            MenuCommand::EditorAdd(list) => {
                if !editor.add_item(list) {
                    self.status = "This list cannot be edited yet".to_owned();
                }
            }
            // The Zone Groups list's button is wired to a method the game does not have: nothing happens.
            MenuCommand::EditorClear(1) => {
                self.status = "Remove All of the Zone Groups list does nothing in the game".to_owned();
            }
            MenuCommand::EditorClear(list) => {
                // The button is disabled while the list is empty.
                if editor.list_len(list) > 0 {
                    editor.popup = Some(EditorPopup::ClearList(list));
                }
            }
            // A default bibite and a zone group go at once (`BibiteDeleted`, `GroupDeleted`); zones ask first.
            MenuCommand::EditorDelete(list @ (1 | 2), index) => {
                editor.delete_item(list, usize::from(index));
            }
            MenuCommand::EditorDelete(list, index) => {
                editor.popup = Some(EditorPopup::DeleteItem(list, index));
            }
            MenuCommand::EditorChallenge(on) => {
                editor.challenge =
                    on.then(bibites_core::settings::ChallengeParameters::game_defaults);
            }
            MenuCommand::EditorFocus(field) => {
                if let Some(EditorPopup::Save { focus }) = editor.popup.as_mut() {
                    *focus = field.min(1);
                }
            }
            MenuCommand::EditorAnswer(yes) => {
                let popup = editor.popup.take();
                match (popup, yes) {
                    (Some(EditorPopup::ExitWarning), true) => self.close_editor(),
                    (Some(EditorPopup::StartWarning), true) => {
                        // `WarningHandle.Dismiss`: a ticked box switches the warning off for good.
                        if std::mem::take(&mut self.confirm_checked) {
                            self.user_settings.set_warning_off(
                                crate::warnings::ActionWarning::StartScenarioAfterEdit,
                                true,
                            );
                        }
                        return self.editor_start();
                    }
                    (Some(EditorPopup::Save { .. }), true) => self.editor_try_save(),
                    (Some(EditorPopup::Overwrite), true) => self.editor_save_now(),
                    (Some(EditorPopup::ClearList(list)), true) => editor.clear_list(list),
                    (Some(EditorPopup::DeleteItem(list, index)), true) => {
                        editor.delete_item(list, usize::from(index));
                    }
                    (Some(EditorPopup::Overwrite), false) => {
                        editor.popup = Some(EditorPopup::Save { focus: 0 });
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        MenuAction::None
    }

    /// Applies a click on a settings-row control; returns whether it was one.
    pub fn editor_action(&mut self, action: UiAction) -> bool {
        let tps = self.user_settings.tps_defaults();
        let Some(editor) = self.editor.as_deref_mut() else {
            return false;
        };
        editor.apply(action, tps)
    }

    /// Mouse held while dragging a slider (`point` in menu pixels at `width` x `height`).
    pub fn editor_drag(&mut self, point: Vec2, width: f32, height: f32) -> bool {
        let tps = self.user_settings.tps_defaults();
        let Some(editor) = self.editor.as_deref_mut() else {
            return false;
        };
        let scale = crate::ui::UiFrame::scaler(width, height);
        if editor.animator.drag.is_some() {
            return editor.drag_graph([point.x, point.y], scale);
        }
        if editor.caret_drag.is_some() {
            return editor.drag_caret(point.x / scale);
        }
        editor.drag_to(point.x / scale, tps)
    }

    /// Mouse released: the slider drag ends.
    pub fn editor_release(&mut self) {
        if let Some(editor) = self.editor.as_deref_mut() {
            editor.release_drags();
        }
    }

    /// Text typed into the save form.
    pub fn editor_type(&mut self, character: char) {
        let Some(editor) = self.editor.as_deref_mut() else {
            return;
        };
        let Some(EditorPopup::Save { focus }) = editor.popup else {
            editor.type_into_field(character);
            return;
        };
        if character.is_control() && !(focus == 1 && character == '\n') {
            return;
        }
        let field = if focus == 0 {
            &mut editor.save_name
        } else {
            &mut editor.save_description
        };
        field.push(character);
    }

    /// Backspace in the save form.
    pub fn editor_backspace(&mut self) {
        let Some(editor) = self.editor.as_deref_mut() else {
            return;
        };
        let Some(EditorPopup::Save { focus }) = editor.popup else {
            editor.backspace_in_field();
            return;
        };
        let field = if focus == 0 {
            &mut editor.save_name
        } else {
            &mut editor.save_description
        };
        field.pop();
    }

    /// Tab in the save form moves between the fields; Enter in the name field moves on too.
    pub fn editor_next_field(&mut self) {
        if let Some(editor) = self.editor.as_deref_mut()
            && let Some(EditorPopup::Save { focus }) = editor.popup.as_mut()
        {
            *focus = 1 - *focus;
        }
    }

    /// Enter in the save form: the name field moves on to the description, which gets a new line.
    pub fn editor_enter(&mut self) {
        let Some(editor) = self.editor.as_deref_mut() else {
            return;
        };
        match editor.popup {
            Some(EditorPopup::Save { focus: 0 }) => {
                editor.popup = Some(EditorPopup::Save { focus: 1 })
            }
            Some(EditorPopup::Save { .. }) => editor.save_description.push('\n'),
            None => editor.submit_edit(),
            _ => {}
        }
    }

    /// Whether the save form is open (the viewer then routes typing to it).
    #[must_use]
    pub fn editor_typing(&self) -> bool {
        self.editor.as_deref().is_some_and(|editor| {
            matches!(editor.popup, Some(EditorPopup::Save { .. })) || (editor.popup.is_none() && editor.field_open())
        })
    }

    /// Ctrl + V in an open text field.
    pub fn editor_paste(&mut self, text: &str) {
        if let Some(editor) = self.editor.as_deref_mut() {
            editor.paste_into_field(text);
        }
    }

    /// A frame passes; returns whether the open value field's caret changed its look.
    pub fn editor_tick(&mut self, dt: f32) -> bool {
        self.editor.as_deref_mut().is_some_and(|editor| editor.tick_field(dt))
    }

    /// A caret key in a typed value (Delete, the arrows, Home, End, Ctrl + A).
    pub fn editor_caret_key(&mut self, key: Option<crate::value_edit::CaretKey>, delete: bool, shift: bool) {
        if let Some(editor) = self.editor.as_deref_mut() {
            editor.key_in_field(key, delete, shift);
        }
    }

    /// `ScenarioSaver.AssertForm`: why the name cannot be used (`None` = fine).
    #[must_use]
    pub fn editor_name_problem(&self) -> Option<&'static str> {
        let editor = self.editor.as_deref()?;
        let name = editor.save_name.as_str();
        if name.is_empty()
            || name
                .chars()
                .any(|c| c.is_control() || INVALID_FILE_CHARS.contains(&c))
        {
            return Some("invalid");
        }
        let lower = name.to_lowercase();
        let official = self
            .scenario_entries
            .iter()
            .any(|entry| entry.is_official && entry.name.to_lowercase() == lower);
        official.then_some("official")
    }

    /// Whether a user scenario with the typed name exists already.
    #[must_use]
    pub fn editor_name_exists(&self) -> bool {
        self.editor
            .as_deref()
            .is_some_and(|editor| self.scenario_path_for(&editor.save_name).is_file())
    }

    /// `Scenarios/<name>.zip` in the folder worlds are saved to (the game's own data folder
    /// when it is installed, so the original game lists the scenario too).
    fn scenario_path_for(&self, name: &str) -> PathBuf {
        bibites_io::storage::StoragePaths::default_content()
            .scenarios_dir()
            .join(format!("{name}.zip"))
    }

    /// "Save": asks before overwriting an existing scenario.
    fn editor_try_save(&mut self) {
        if self.editor_name_problem().is_some() {
            if let Some(editor) = self.editor.as_deref_mut() {
                editor.popup = Some(EditorPopup::Save { focus: 0 });
            }
            return;
        }
        if self.editor_name_exists() {
            if let Some(editor) = self.editor.as_deref_mut() {
                editor.popup = Some(EditorPopup::Overwrite);
            }
            return;
        }
        self.editor_save_now();
    }

    /// Writes the draft to `Scenarios/<name>.zip` (`ScenarioSaver.SaveScenario`).
    fn editor_save_now(&mut self) {
        let Some(editor) = self.editor.as_deref() else {
            return;
        };
        let name = editor.save_name.clone();
        let path = self.scenario_path_for(&name);
        let result = (|| {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            write_editor_bundle(editor, &path, Some((&name, &editor.save_description)))
        })();
        let content = bibites_io::storage::StoragePaths::default_content().root;
        match result {
            Ok(()) => {
                self.status = format!("Scenario saved as {name}");
                if !self.data_roots.contains(&content) {
                    self.data_roots.push(content);
                }
                self.reload_scenarios();
                // The editor stays open (`OnSubmit` only closes the save form).
                if let Some(editor) = self.editor.as_deref_mut() {
                    editor.title = name;
                    editor.description = editor.save_description.clone();
                }
            }
            Err(error) => self.status = format!("Could not save the scenario: {error}"),
        }
    }

    /// Runs the draft (`ScenarioSelectorPanel.StartSim(customScenario: true)`).
    fn editor_start(&mut self) -> MenuAction {
        let Some(editor) = self.editor.as_deref() else {
            return MenuAction::None;
        };
        let path = std::env::temp_dir().join("rustibites-newgame-scenario.zip");
        let title = editor.title.clone();
        let description = editor.description.clone();
        match write_editor_bundle(editor, &path, Some((&title, &description))) {
            Ok(()) => {
                self.begin_loading(path.clone(), false);
                MenuAction::Start(path)
            }
            Err(error) => {
                self.status = format!("Could not start the scenario: {error}");
                MenuAction::None
            }
        }
    }
}

/// Writes the draft as a scenario bundle: the base scenario's templates and extras with the
/// draft's settings, zones, animators and challenge.
fn write_editor_bundle(
    editor: &EditorState,
    path: &Path,
    info: Option<(&str, &str)>,
) -> Result<(), String> {
    let mut bundle =
        bibites_io::load_scenario_bundle(&editor.base_path).map_err(|error| error.to_string())?;
    bundle.settings = editor.settings.clone();
    bundle.settings_file.scenario = editor.settings.scenario.clone();
    bundle.settings_file.materials = editor.settings.materials;
    bundle.settings_file.recommended = Some(editor.settings.independents.clone());
    bundle.settings_file.challenge = editor.challenge.clone();
    bundle.scenario = editor.scenario.clone();
    // The templates added for default bibites travel with the scenario
    // (`SaveSystem.AddTemplatesToArchive`).
    for (file_name, bytes) in &editor.added_templates {
        let used = bundle
            .scenario
            .bibite_spawns
            .iter()
            .any(|row| row.path == *file_name);
        let known = bundle.templates.iter().any(|(name, _)| name == file_name);
        if !used || known {
            continue;
        }
        let text = String::from_utf8_lossy(bytes);
        let loaded = bibites_io::from_json_with_period(
            &text,
            bibites_core::settings::SimSettings::game_defaults().brain_period(),
        )
        .map_err(|error| format!("template {file_name}: {error}"))?;
        bundle.templates.push((file_name.clone(), loaded));
        bundle
            .template_bytes
            .push((format!("templates/{file_name}"), bytes.clone()));
    }
    bundle.info.is_official = false;
    bundle.info.rank = None;
    bundle.info.is_challenge = editor.challenge.is_some();
    bundle.info.version_text = crate::app::GAME_VERSION.to_owned();
    if let Some((name, description)) = info {
        bundle.info.name = name.to_owned();
        bundle.info.description = if description.is_empty() {
            "No description was given for this scenario.".to_owned()
        } else {
            description.to_owned()
        };
        bundle.scenario.name = name.to_owned();
    }
    bibites_io::write_scenario_bundle(&bundle, path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use bibites_io::storage::{LibraryEntry, TemplateKind};

    use super::*;
    use crate::menu::TemplatePick;
    use crate::scenario_editor::TAB_BIBITES;

    fn workspace(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join(relative)
    }

    /// A menu with the Default scenario, one library template, and the editor open on it.
    fn editor_menu(dir: &Path) -> Option<(MenuState, PathBuf)> {
        let default = workspace("testdata/real/Default.zip");
        let template = workspace("testdata/real/templates/10_Parasite_0.6.bb8template");
        if !default.is_file() || !template.is_file() {
            eprintln!("skipping: the committed test data is absent");
            return None;
        }
        let mut menu = MenuState::new(Some(default), dir);
        menu.reload_scenarios();
        menu.templates = vec![LibraryEntry {
            name: "Parasite".to_owned(),
            description: String::new(),
            version: "0.6".to_owned(),
            is_official: true,
            n_hidden_nodes: 0,
            n_synapses: 0,
            path: template.clone(),
            kind: TemplateKind::Template,
        }];
        menu.view = MenuView::NewGame;
        menu.menu_command(MenuCommand::NewScenario);
        assert_eq!(menu.view(), MenuView::ScenarioEditor);
        assert!(menu.editor_action(UiAction::DynTab(TAB_BIBITES)));
        Some((menu, template))
    }

    #[test]
    fn add_opens_the_library_and_select_bibite_adds_a_default_bibite() {
        let dir = std::env::temp_dir().join(format!("rustibites-defbib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let Some((mut menu, template)) = editor_menu(&dir) else {
            return;
        };
        let before = menu.editor().unwrap().scenario.bibite_spawns.len();

        // Add opens the template selector in pick mode; Back returns to the editor unchanged.
        menu.menu_command(MenuCommand::EditorAdd(2));
        assert_eq!(menu.view(), MenuView::Library);
        assert_eq!(menu.pick(), Some(TemplatePick::DefaultBibite));
        menu.menu_command(MenuCommand::Back);
        assert_eq!(menu.view(), MenuView::ScenarioEditor);
        assert_eq!(menu.pick(), None);
        assert_eq!(menu.editor().unwrap().scenario.bibite_spawns.len(), before);

        // Submitting without a template says so and stays in the selector.
        menu.menu_command(MenuCommand::EditorAdd(2));
        menu.menu_command(MenuCommand::StartTemplate);
        assert_eq!(menu.view(), MenuView::Library);
        assert!(menu.status().contains("Select a template"));

        // Select Bibite hands the template to the editor, which opens the new item.
        menu.menu_command(MenuCommand::SelectTemplate(0));
        let action = menu.menu_command(MenuCommand::StartTemplate);
        assert_eq!(action, MenuAction::None);
        assert_eq!(menu.view(), MenuView::ScenarioEditor);
        let editor = menu.editor().unwrap();
        assert_eq!(editor.scenario.bibite_spawns.len(), before + 1);
        let row = editor.scenario.bibite_spawns.last().unwrap();
        assert_eq!(row.path, "10_Parasite_0.6.bb8template");
        assert_eq!((row.spawn_priority, row.minimum), (1.0, 10));
        assert!(editor.open_items.contains(&(2, before as u16)));
        assert_eq!(editor.added_templates.len(), 1);

        // The saved scenario carries the template file and the row.
        let out = dir.join("with-bibite.zip");
        write_editor_bundle(menu.editor().unwrap(), &out, None).expect("bundle written");
        let bundle = bibites_io::load_scenario_bundle(&out).expect("bundle loads");
        assert!(
            bundle
                .templates
                .iter()
                .any(|(name, _)| name == "10_Parasite_0.6.bb8template"),
            "templates: {:?}",
            bundle.templates.iter().map(|(n, _)| n).collect::<Vec<_>>()
        );
        assert_eq!(bundle.scenario.bibite_spawns.len(), before + 1);
        assert!(std::fs::metadata(&template).is_ok());

        // A default bibite is deleted at once (no confirmation), and Remove All still asks.
        menu.menu_command(MenuCommand::EditorDelete(2, before as u16));
        assert_eq!(menu.editor().unwrap().scenario.bibite_spawns.len(), before);
        assert!(menu.editor().unwrap().popup.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_editors_zone_and_bibite_dropdowns_open_lists_and_a_pick_edits_the_scenario() {
        use crate::bibite_editor::{GROWTH, TAGGING};
        use crate::ui::DropdownTarget as Target;
        use crate::zone_editor::{DISTRIBUTION, MOVEMENT};
        use bibites_core::settings::{GrowthAtSpawn, MovementType, SpawnDistribution, Tagging};
        let dir = std::env::temp_dir().join(format!("rustibites-editdrop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let Some((mut menu, _)) = editor_menu(&dir) else {
            return;
        };
        let anchor = [900.0, 400.0, 180.0, 30.0];
        // A zone item: the distribution's six options, a pick sets the zone's.
        menu.open_dropdown_list(Target::Zone(0, DISTRIBUTION), anchor);
        let list = menu.dropdown_list().expect("the zone's list opens");
        assert_eq!(list.choices.len(), 6);
        assert_eq!(list.choices[4].label, "Flat Ring");
        menu.pick_dropdown_list(4);
        assert!(menu.dropdown_list().is_none());
        assert_eq!(
            menu.editor().unwrap().scenario.zones[0].distribution,
            SpawnDistribution::FlatRing
        );
        menu.open_dropdown_list(Target::Zone(0, MOVEMENT), anchor);
        menu.pick_dropdown_list(1);
        assert_eq!(
            menu.editor().unwrap().scenario.zones[0].movement,
            MovementType::Free
        );
        // A default bibite item: its dropdowns list the game's options and a pick edits the row.
        menu.menu_command(MenuCommand::EditorAdd(2));
        menu.menu_command(MenuCommand::SelectTemplate(0));
        menu.menu_command(MenuCommand::StartTemplate);
        let row = menu.editor().unwrap().scenario.bibite_spawns.len() as u16 - 1;
        menu.open_dropdown_list(Target::Bibite(row, TAGGING), anchor);
        assert_eq!(menu.dropdown_list().expect("list").choices.len(), 4);
        menu.pick_dropdown_list(3);
        menu.open_dropdown_list(Target::Bibite(row, GROWTH), anchor);
        menu.pick_dropdown_list(0);
        let spawn = &menu.editor().unwrap().scenario.bibite_spawns[usize::from(row)];
        assert_eq!(spawn.tagging, Tagging::RandomTagging);
        assert_eq!(spawn.growth_at_spawn, GrowthAtSpawn::Egg);
        // A dropdown of an item that is gone opens nothing.
        menu.open_dropdown_list(Target::Zone(99, DISTRIBUTION), anchor);
        assert!(menu.dropdown_list().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_start_warning_has_a_do_not_show_again_box_that_skips_it_next_time() {
        use crate::warnings::ActionWarning;
        let dir = std::env::temp_dir().join(format!("rustibites-startwarn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let Some((mut menu, _)) = editor_menu(&dir) else {
            return;
        };
        // Its own settings file, and "shown" whatever the installed game remembers.
        menu.user_settings =
            crate::user_settings::UserSettings::load(Some(dir.join("settings.json")));
        menu.user_settings.reset_warnings();

        // Start asks first, with the box unticked.
        assert_eq!(
            menu.menu_command(MenuCommand::EditorStart),
            MenuAction::None
        );
        assert!(menu.start_warning_up());
        assert!(!menu.confirm_checked());
        // Cancel with the box ticked remembers nothing.
        menu.menu_command(MenuCommand::ToggleDoNotShowAgain);
        assert!(menu.confirm_checked());
        menu.menu_command(MenuCommand::EditorAnswer(false));
        assert!(!menu.start_warning_up());
        assert!(!menu.warning_off(ActionWarning::StartScenarioAfterEdit));

        // Yes with the box ticked starts the simulation and switches the warning off for good.
        menu.menu_command(MenuCommand::EditorStart);
        assert!(!menu.confirm_checked(), "a new popup starts unticked");
        menu.menu_command(MenuCommand::ToggleDoNotShowAgain);
        let action = menu.menu_command(MenuCommand::EditorAnswer(true));
        assert!(matches!(action, MenuAction::Start(_)), "{action:?}");
        assert!(menu.warning_off(ActionWarning::StartScenarioAfterEdit));
        assert!(
            crate::user_settings::UserSettings::load(Some(dir.join("settings.json")))
                .warning_off(ActionWarning::StartScenarioAfterEdit)
        );

        // The next Start goes at once.
        menu.view = MenuView::ScenarioEditor;
        let action = menu.menu_command(MenuCommand::EditorStart);
        assert!(matches!(action, MenuAction::Start(_)), "{action:?}");
        assert!(!menu.start_warning_up());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_editors_animators_names_typed_values_and_move_arrows_work() {
        use crate::dyn_panel::FieldRef;
        use crate::ui::{ControlCommand, DropdownTarget};
        use crate::zone_editor as ze;
        let dir = std::env::temp_dir().join(format!("rustibites-anim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let Some((mut menu, _)) = editor_menu(&dir) else {
            return;
        };
        let changers = |menu: &MenuState| menu.editor().unwrap().scenario.settings_changers.clone();
        // The creator opens from the list's + and OK adds an animator whose card opens.
        assert!(changers(&menu).is_empty());
        menu.menu_command(MenuCommand::EditorAdd(3));
        assert!(menu.editor().unwrap().animator.creator.open);
        assert!(menu.editor_action(UiAction::Invoke(ControlCommand::SubmitAnimatorCreator)));
        assert_eq!(changers(&menu).len(), 1);
        assert!(menu.editor().unwrap().open_items.contains(&(3, 0)));
        // A name is typed into the card's field and Enter renames it.
        let reference = ze::animator_ref(0);
        assert!(menu.editor_action(UiAction::ZoneNameEdit(reference)));
        assert!(menu.editor_typing());
        for _ in 0..40 {
            menu.editor_backspace();
        }
        for character in "Pulse".chars() {
            menu.editor_type(character);
        }
        menu.editor_enter();
        assert_eq!(changers(&menu)[0].name, "Pulse");
        assert!(!menu.editor_typing());
        // The caret moves inside a name: Home, a typed letter, Right, Delete, then Enter.
        use crate::value_edit::CaretKey;
        assert!(menu.editor_action(UiAction::ZoneNameEdit(reference)));
        menu.editor_caret_key(Some(CaretKey::Home), false, false);
        menu.editor_type('X');
        menu.editor_caret_key(Some(CaretKey::Right), false, false);
        menu.editor_caret_key(None, true, false);
        menu.editor_enter();
        assert_eq!(changers(&menu)[0].name, "XPlse");
        // Ctrl + A selects the name and a typed letter replaces it; Escape leaves the old one.
        assert!(menu.editor_action(UiAction::ZoneNameEdit(reference)));
        menu.editor_caret_key(Some(CaretKey::SelectAll), false, false);
        menu.editor_type('Q');
        menu.editor_escape();
        assert_eq!(changers(&menu)[0].name, "XPlse");
        assert!(!menu.editor_typing());
        assert!(menu.editor_action(UiAction::ZoneNameEdit(reference)));
        menu.editor_paste("ab\n");
        menu.editor_enter();
        assert_eq!(changers(&menu)[0].name, "XPlseab");
        // A press on the open name puts the caret under the pointer (10 px characters here), a drag selects, a typed letter replaces the selection.
        assert!(menu.editor_action(UiAction::ZoneNameEdit(reference)));
        {
            let editor = menu.editor.as_deref_mut().unwrap();
            editor.measure_field(&|text, _| text.chars().count() as f32 * 10.0);
            editor.pointer = crate::scenario_editor::Pointer { ctrl: false, cursor: [131.0, 5.0], rect: Some([100.0, 0.0, 300.0, 30.0]), shift: false, scale: 1.0 };
        }
        assert!(menu.editor_action(UiAction::ZoneNameEdit(reference)));
        assert_eq!(menu.editor().unwrap().name_caret.pos, 3);
        assert!(menu.editor_drag(Vec2::new(150.0, 5.0), 1920.0, 1080.0));
        {
            let editor = menu.editor.as_deref_mut().unwrap();
            assert!(editor.drag_caret(150.0));
            assert_eq!(editor.name_caret.selection(), Some((3, 5)));
        }
        menu.editor_type('Z');
        menu.editor_release();
        menu.editor_enter();
        assert_eq!(changers(&menu)[0].name, "XPlZab");
        // A typed time moves the picked point; a value field opens on a click and Enter applies it.
        menu.editor.as_deref_mut().unwrap().animator.point = Some((0, 1));
        let field = FieldRef::Zone(reference, ze::ANIM_POINT_VALUE);
        assert!(menu.editor_action(UiAction::DynEditValue(field)));
        for _ in 0..12 {
            menu.editor_backspace();
        }
        for character in "3".chars() {
            menu.editor_type(character);
        }
        menu.editor_enter();
        assert_eq!(changers(&menu)[0].points[1].y, 3.0);
        // The start slider follows a drag; the base dropdown adds a setting; Repeat flips.
        let repeat = changers(&menu)[0].repeat;
        assert!(menu.editor_action(UiAction::ZoneToggle(reference, ze::ANIM_REPEAT)));
        assert_eq!(changers(&menu)[0].repeat, !repeat);
        assert!(menu.editor_action(UiAction::ZoneSlide(reference, ze::ANIM_START, 100, 400)));
        assert!(menu.editor_drag(Vec2::new(400.0, 0.0), 1920.0, 1080.0));
        menu.editor_release();
        assert!(changers(&menu)[0].start > 0.0, "{}", changers(&menu)[0].start);
        assert!(menu.editor.as_deref_mut().unwrap().animator_pick(DropdownTarget::AnimatorBase(0), 1));
        assert_eq!(changers(&menu)[0].settings_bases.len(), 1);
        assert!(menu.editor_action(UiAction::Invoke(ControlCommand::RemoveAnimatorBase(0, 0))));
        assert!(changers(&menu)[0].settings_bases.is_empty());
        // A second animator; the move arrows reorder the cards, not the data.
        menu.menu_command(MenuCommand::EditorAdd(3));
        assert!(menu.editor_action(UiAction::Invoke(ControlCommand::SubmitAnimatorCreator)));
        assert!(menu.editor_action(UiAction::ListMove(3, 1, true)));
        let order = crate::dyn_panel::card_order(&menu.editor().unwrap().dyn_state.order, 3, 2);
        assert_eq!(order, vec![1, 0]);

        // Delete asks first; the card that goes takes its place in the order with it.
        assert!(menu.editor_action(UiAction::Invoke(ControlCommand::DeleteAnimator(0))));
        assert_eq!(menu.editor().unwrap().popup, Some(EditorPopup::DeleteItem(3, 0)));
        menu.menu_command(MenuCommand::EditorAnswer(true));
        assert_eq!(changers(&menu).len(), 1);
        assert_eq!(crate::dyn_panel::card_order(&menu.editor().unwrap().dyn_state.order, 3, 1), vec![0]);
        // A typed value in a settings row, and a typed zone name.
        let index = crate::setting_ui_data::SETTING_UI.iter().position(|row| row.field == "dragCoefficient").expect("a settings row");
        assert!(menu.editor_action(UiAction::DynEditValue(FieldRef::Setting(index as u16))));
        for _ in 0..12 {
            menu.editor_backspace();
        }
        for character in "7".chars() {
            menu.editor_type(character);
        }
        menu.editor_enter();
        assert_eq!(crate::dyn_panel::read_value(&menu.editor().unwrap().settings, &crate::setting_ui_data::SETTING_UI[index]), 7.0);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// An animator made in the editor travels with the scenario: Start (and Save) write the draft as a bundle, and the world that is made from that
    /// bundle holds the animator with its settings, as the scenario editor's list shows it.
    #[test]
    fn an_animator_made_in_the_editor_goes_into_the_scenario_that_starts() {
        use crate::ui::{ControlCommand, DropdownTarget};
        let dir = std::env::temp_dir().join(format!("rustibites-anim-start-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let Some((mut menu, _)) = editor_menu(&dir) else {
            return;
        };
        assert!(menu.editor_action(UiAction::DynTab(crate::scenario_editor::TAB_ANIMS)));
        // The `+`, a Sine preset, OK, then a setting for the animator to take over.
        assert!(menu.editor_action(UiAction::Invoke(ControlCommand::OpenAnimatorCreator)));
        menu.editor_creator_preset_for_capture(1);
        assert!(menu.editor_action(UiAction::Invoke(ControlCommand::SubmitAnimatorCreator)));
        assert!(menu.editor.as_deref_mut().unwrap().animator_pick(DropdownTarget::AnimatorBase(0), 1));
        let drafted = menu.editor().unwrap().changers().to_vec();
        assert_eq!(drafted.len(), 1);
        assert_eq!(drafted[0].settings_bases.len(), 1, "the animator holds a setting");
        // The bundle Start writes, read back the way the new world reads it.
        let path = dir.join("draft.zip");
        write_editor_bundle(menu.editor().unwrap(), &path, Some(("Pulses", ""))).expect("the draft is written");
        let bundle = bibites_io::load_scenario_bundle(&path).expect("the bundle reads back");
        assert_eq!(bundle.scenario.settings_changers.len(), 1);
        assert_eq!(bundle.scenario.settings_changers[0].name, drafted[0].name);
        assert_eq!(bundle.scenario.settings_changers[0].settings_bases, drafted[0].settings_bases);
        assert_eq!(bundle.scenario.settings_changers[0].points.len(), drafted[0].points.len());
        let contents = bibites_io::world::scenario_world_contents(&bundle);
        assert_eq!(contents.settings_changers.len(), 1, "the world is made with the animator");
        assert_eq!(contents.settings_changers[0].settings_bases, drafted[0].settings_bases);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_items_edit_their_spawn_settings_through_the_editor_actions() {
        let dir = std::env::temp_dir().join(format!("rustibites-defbib2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let Some((mut menu, _)) = editor_menu(&dir) else {
            return;
        };
        menu.menu_command(MenuCommand::EditorAdd(2));
        menu.menu_command(MenuCommand::SelectTemplate(0));
        menu.menu_command(MenuCommand::StartTemplate);
        let index = (menu.editor().unwrap().scenario.bibite_spawns.len() - 1) as u16;
        use crate::bibite_editor::{GROWTH, MINIMUM, PRIORITY, TAGGING};
        // Dropdowns advance, sliders follow a drag and reset.
        assert!(menu.editor_action(UiAction::BibiteCycle(index, TAGGING)));
        assert!(menu.editor_action(UiAction::BibiteCycle(index, GROWTH)));
        assert!(menu.editor_action(UiAction::BibiteSlide(index, MINIMUM, 100, 400)));
        assert!(menu.editor_drag(Vec2::new(100.0, 0.0), 1920.0, 1080.0));
        assert!(menu.editor_action(UiAction::BibiteSlide(index, PRIORITY, 100, 400)));
        assert!(menu.editor_drag(Vec2::new(400.0, 0.0), 1920.0, 1080.0));
        menu.editor_release();
        let row = menu
            .editor()
            .unwrap()
            .scenario
            .bibite_spawns
            .get(usize::from(index))
            .unwrap()
            .clone();
        assert_eq!(row.tagging, bibites_core::settings::Tagging::SpeciesTagging);
        assert_eq!(
            row.growth_at_spawn,
            bibites_core::settings::GrowthAtSpawn::Elder
        );
        assert_eq!(row.minimum, 0, "dragged to the left end");
        assert_eq!(row.spawn_priority, 100.0, "dragged to the right end");
        assert!(menu.editor_action(UiAction::BibiteReset(index, MINIMUM)));
        let row = &menu.editor().unwrap().scenario.bibite_spawns[usize::from(index)];
        assert_eq!(row.minimum, 10);
        let _ = std::fs::remove_dir_all(dir);
    }
}

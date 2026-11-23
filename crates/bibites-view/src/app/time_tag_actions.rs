//! RUSTibites Improved: the time tags' side of the app (`crate::time_tags` is the model): setting a tag with a name the player types, removing one, the
//! per-panel show / hide switches, and keeping the tags with the saves (`time_tags/<save name>.json` in the state folder, never inside a save).

use std::path::Path;

use bibites_io::storage::{read_time_tags, write_time_tag_view, write_time_tags};

use super::{App, FrameInput};
use crate::text_field::{self, Caret};
use crate::time_tags::{MAX_LABEL_CHARS, TagPanel, TimeTags};
use crate::ui::PanelKind;

/// `TMP_InputField.caretBlinkRate` of the game's input fields (the caret blinks like theirs).
const BLINK_RATE: f32 = 0.85;

/// The name field of a time tag while the player types it (opened by a panel's "Tag time" button).
#[derive(Clone, Debug, PartialEq)]
pub struct TimeTagEdit {
    /// The panel whose button opened it (the field is drawn there).
    pub panel: TagPanel,
    /// What is typed so far.
    pub text: String,
    /// The caret and the selection.
    pub caret: Caret,
    /// Seconds since the last key (the caret blinks from there).
    pub idle: f32,
}

impl TimeTagEdit {
    /// An empty field.
    #[must_use]
    pub fn new(panel: TagPanel) -> Self {
        Self { panel, text: String::new(), caret: Caret::at_end(""), idle: 0.0 }
    }

    /// Whether the caret is drawn now: no selection, and the first half of the blink period.
    #[must_use]
    pub fn caret_visible(&self) -> bool {
        let period = 1.0 / BLINK_RATE;
        self.caret.selection().is_none() && self.idle % period <= period / 2.0
    }
}

impl App {
    /// Whether the time tags show in `panel` (the player's switch for it).
    #[must_use]
    pub fn time_tags_shown(&self, panel: TagPanel) -> bool {
        match panel {
            TagPanel::Graphs => self.time_tag_view.graphs,
            TagPanel::Genes => self.time_tag_view.genes,
            TagPanel::Lineage => self.time_tag_view.lineage,
        }
    }

    /// Whether `panel` is on the screen now (a name field left behind by a panel that closed is dropped).
    fn time_tag_panel_visible(&self, panel: TagPanel) -> bool {
        match panel {
            TagPanel::Graphs => self.panels.is_open(PanelKind::Graphs) && !self.graphs_genes,
            TagPanel::Genes => self.panels.is_open(PanelKind::Graphs) && self.graphs_genes,
            TagPanel::Lineage => self.panels.is_open(PanelKind::Lineage),
        }
    }

    /// The show / hide button of `panel`: its own switch only, the other panels keep theirs.
    pub(crate) fn time_tags_toggle(&mut self, panel: TagPanel) {
        let shown = !self.time_tags_shown(panel);
        match panel {
            TagPanel::Graphs => self.time_tag_view.graphs = shown,
            TagPanel::Genes => self.time_tag_view.genes = shown,
            TagPanel::Lineage => self.time_tag_view.lineage = shown,
        }
        if let Err(error) = write_time_tag_view(&self.storage, &self.time_tag_view) {
            self.last_action = format!("time tag settings not saved: {error}");
            return;
        }
        self.last_action = format!("time tags {} in {}", if shown { "shown" } else { "hidden" }, panel.name());
    }

    /// The "Tag time" button of `panel`: the name field opens there; pressed again while it is open (in the same panel) it sets the tag, as Enter does.
    pub(crate) fn time_tag_open(&mut self, panel: TagPanel) {
        if self.time_tag_edit.as_ref().is_some_and(|edit| edit.panel == panel) {
            self.time_tag_commit();
            return;
        }
        // A tag is always seen where it was set: a hidden panel shows them again.
        if !self.time_tags_shown(panel) {
            self.time_tags_toggle(panel);
        }
        self.time_tag_edit = Some(TimeTagEdit::new(panel));
    }

    /// Sets the tag the open name field describes, at the world's time now (a blank name becomes "Tag N"). The field closes either way.
    pub(crate) fn time_tag_commit(&mut self) {
        let now = self.simulated_time();
        self.time_tag_set(now);
    }

    /// A click on the graph of `panel` while its name field is open: the tag is set at `seconds` of the world's clock, a moment of the past (a time before the
    /// world began or after now is not one: nothing is set and the field stays open for another try).
    pub(crate) fn time_tag_place(&mut self, panel: TagPanel, seconds: f64) {
        if self.time_tag_edit.as_ref().is_none_or(|edit| edit.panel != panel) {
            return;
        }
        let now = self.simulated_time();
        if !seconds.is_finite() || seconds < 0.0 || seconds > now + 0.5 {
            self.last_action = "time tag not set: that moment is outside the world's time".to_owned();
            return;
        }
        self.time_tag_set(seconds.min(now));
    }

    /// Sets the tag the open name field describes at `seconds`; the field closes either way.
    fn time_tag_set(&mut self, seconds: f64) {
        let Some(edit) = self.time_tag_edit.take() else {
            return;
        };
        match self.time_tags.add(seconds, &edit.text) {
            Some(id) => {
                let label = self.time_tags.get(id).map(|tag| tag.label.clone()).unwrap_or_default();
                self.last_action = format!("time tag \"{label}\" set");
                self.time_tags_persist();
            }
            None => self.last_action = format!("time tag not set: a world keeps at most {} tags", crate::time_tags::MAX_TAGS),
        }
    }

    /// A tag's remove button.
    pub(crate) fn time_tag_remove(&mut self, id: u32) {
        let label = self.time_tags.get(id).map(|tag| tag.label.clone());
        if self.time_tags.remove(id) {
            self.last_action = format!("time tag \"{}\" removed", label.unwrap_or_default());
            self.time_tags_persist();
        }
    }

    /// Escape in the name field: it closes without setting a tag. `false` when no field was open.
    pub(crate) fn time_tag_cancel(&mut self) -> bool {
        self.time_tag_edit.take().is_some()
    }

    /// Typing into the name field: characters, Backspace, Delete, the arrows, Home / End, Ctrl + A and Ctrl + V; Enter sets the tag. Called every frame
    /// (it does nothing without an open field), after the other fields took their share of what was typed.
    pub(super) fn edit_time_tag(&mut self, input: &FrameInput) {
        let Some(panel) = self.time_tag_edit.as_ref().map(|edit| edit.panel) else {
            return;
        };
        if !self.time_tag_panel_visible(panel) {
            self.time_tag_edit = None;
            return;
        }
        let keys = self.edit_keys(input);
        let mut typed = std::mem::take(&mut self.typed);
        if input.paste_tag {
            typed.push_str(&self.clipboard_in.take().unwrap_or_default());
        }
        let Some(edit) = self.time_tag_edit.as_mut() else {
            return;
        };
        edit.idle += input.dt;
        let before = (edit.text.clone(), edit.caret);
        text_field::edit(&mut edit.text, &mut edit.caret, &typed, MAX_LABEL_CHARS, keys);
        if (edit.text.clone(), edit.caret) != before {
            edit.idle = 0.0;
        }
        if input.enter {
            self.time_tag_commit();
        }
    }

    /// Stores the world's tags under the name of the save it was saved or loaded as (nothing is stored for a world that has no such name yet).
    fn time_tags_persist(&mut self) {
        let Some(key) = self.time_tags_key.clone() else {
            return;
        };
        if let Err(error) = write_time_tags(&self.storage, &key, &self.time_tags.to_file()) {
            self.last_action = format!("time tags not saved: {error}");
        }
    }

    /// A save of the world was written as `path`: the tags are stored under its name too (and the world's tags follow that name from now on).
    pub(crate) fn time_tags_after_save(&mut self, path: &Path) {
        let Some(stem) = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()) else {
            return;
        };
        self.time_tags_key = Some(stem);
        self.time_tags_persist();
        self.zone_colors_persist();
        self.gene_towers_persist();
        self.vent_towers_persist();
        self.force_towers_persist();
        self.whirlpool_towers_persist();
        self.streams_persist();
    }

    /// The world was loaded from the save `path`: its tags come back (without the ones of a later timeline, see [`TimeTags::from_file`]).
    pub(crate) fn time_tags_after_load(&mut self, path: &Path) {
        self.time_tag_edit = None;
        let Some(stem) = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()) else {
            return;
        };
        let now = self.simulated_time();
        self.time_tags = match read_time_tags(&self.storage, &stem) {
            Ok(file) => TimeTags::from_file(&file, now),
            Err(error) => {
                self.last_action = format!("time tags of {stem} not read: {error}");
                TimeTags::new()
            }
        };
        self.zone_colors_after_load(&stem);
        self.gene_towers_after_load(&stem);
        self.vent_towers_after_load(&stem);
        self.force_towers_after_load(&stem);
        self.whirlpool_towers_after_load(&stem);
        self.streams_after_load(&stem);
        self.time_tags_key = Some(stem);
    }

    /// An autosave is about to be written as `autosave_<stamp>.zip`: the world's tags are stored beside it under its name (the world's own key stays), and the
    /// tag files of autosaves that were deleted since are cleaned away.
    pub(crate) fn time_tags_beside_autosave(&mut self, stamp: &str) {
        bibites_io::storage::prune_autosave_time_tags(&self.storage);
        self.zone_colors_beside_autosave(stamp);
        self.gene_towers_beside_autosave(stamp);
        self.vent_towers_beside_autosave(stamp);
        self.force_towers_beside_autosave(stamp);
        self.whirlpool_towers_beside_autosave(stamp);
        self.streams_beside_autosave(stamp);
        if self.time_tags.is_empty() {
            return;
        }
        if let Err(error) = write_time_tags(&self.storage, &format!("autosave_{stamp}"), &self.time_tags.to_file()) {
            self.last_action = format!("time tags of the autosave not saved: {error}");
        }
    }

    /// A new world took the place of the old one (a scenario, a challenge): it has no tags and no save name yet.
    pub(crate) fn time_tags_reset(&mut self) {
        self.time_tags = TimeTags::new();
        self.time_tags_key = None;
        self.time_tag_edit = None;
        self.zone_color_popup = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LoadedWorld;
    use crate::ui::UiAction;
    use bibites_core::math::Vec2;
    use bibites_io::storage::{StoragePaths, TimeTagRecord, read_time_tag_view};

    /// A throwaway state folder (the tests must not touch the player's own).
    fn folder(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("rustibites_time_tags_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp folder");
        path
    }

    /// An app whose world has run for a while (so its clock reads something), the Graphs panel open, with a state folder of its own.
    fn app(name: &str) -> (App, std::path::PathBuf) {
        let loaded = LoadedWorld::load(None, 77, 6, 1).expect("starter world");
        let mut app = App::new(loaded, Vec2::new(1920.0, 1080.0), None);
        let root = folder(name);
        app.storage = StoragePaths::new(&root);
        app.world.run(600);
        app.panels.open(PanelKind::Graphs);
        (app, root)
    }

    fn key(app: &mut App, input: FrameInput) {
        app.handle_input(&FrameInput { dt: 0.016, ..input });
    }

    #[test]
    fn the_tag_button_opens_a_name_field_that_blocks_the_hotkeys_and_enter_sets_the_tag_at_the_worlds_time() {
        let (mut app, root) = app("flow");
        let now = app.simulated_time();
        assert!(now > 0.0);
        app.apply_ui_actions(&[UiAction::TimeTagStart(TagPanel::Graphs)]);
        assert_eq!(app.time_tag_edit.as_ref().map(|edit| edit.panel), Some(TagPanel::Graphs));
        // What is typed goes in the field; `R` (a random bibite) and the other hotkeys wait.
        app.typed = "Drought".to_owned();
        key(&mut app, FrameInput { select_random_bibite: true, ..FrameInput::default() });
        assert_eq!(app.time_tag_edit.as_ref().map(|edit| edit.text.as_str()), Some("Drought"));
        assert!(app.selected.is_none(), "the field blocks the hotkeys");
        key(&mut app, FrameInput { backspace: true, ..FrameInput::default() });
        // A space in the name is a character, not the pause key.
        let paused = app.paused;
        app.typed = "t ".to_owned();
        key(&mut app, FrameInput { toggle_pause: true, ..FrameInput::default() });
        assert_eq!(app.paused, paused, "the space typed in the name does not pause or resume the game");
        app.typed = "starts".to_owned();
        key(&mut app, FrameInput::default());
        key(&mut app, FrameInput { enter: true, ..FrameInput::default() });
        assert!(app.time_tag_edit.is_none());
        let tags = app.time_tags.all();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].label, "Drought starts");
        assert_eq!(tags[0].seconds, now, "the tag is set at the world's time now");
        assert!(app.last_action.contains("Drought starts"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_click_on_a_graph_sets_the_tag_in_the_past_with_the_name_typed_and_a_moment_outside_the_worlds_time_sets_none() {
        let (mut app, root) = app("place");
        app.world.simulated_time = 5_000.0;
        let now = app.simulated_time();
        assert_eq!(now, 5_000.0);
        // No name field is open: a click action that comes anyway sets nothing.
        app.apply_ui_actions(&[UiAction::TimeTagPlace(TagPanel::Graphs, now - 5.0)]);
        assert!(app.time_tags.is_empty());
        // Another panel's graph does not set the tag of this panel's field.
        app.apply_ui_actions(&[UiAction::TimeTagStart(TagPanel::Graphs)]);
        app.apply_ui_actions(&[UiAction::TimeTagPlace(TagPanel::Lineage, now - 5.0)]);
        assert!(app.time_tags.is_empty() && app.time_tag_edit.is_some());
        // Before the world began, in its future, or not a time: refused, and the field stays open for another try.
        for seconds in [-3.0, now + 100.0, f64::NAN, f64::INFINITY] {
            app.apply_ui_actions(&[UiAction::TimeTagPlace(TagPanel::Graphs, seconds)]);
            assert!(app.time_tags.is_empty() && app.time_tag_edit.is_some(), "{seconds}");
        }
        assert!(app.last_action.contains("outside"), "{}", app.last_action);
        // The name typed so far goes with the tag set in the past, and the field closes.
        app.typed = "Flood".to_owned();
        key(&mut app, FrameInput::default());
        app.apply_ui_actions(&[UiAction::TimeTagPlace(TagPanel::Graphs, now - 120.0)]);
        assert!(app.time_tag_edit.is_none());
        let tags = app.time_tags.all();
        assert_eq!(tags.len(), 1);
        assert_eq!((tags[0].label.as_str(), tags[0].seconds), ("Flood", now - 120.0));
        // Without a name it is "Tag N" as always; a click at the very end of the graph (now) is a moment of the world's time too.
        app.apply_ui_actions(&[UiAction::TimeTagStart(TagPanel::Graphs)]);
        app.apply_ui_actions(&[UiAction::TimeTagPlace(TagPanel::Graphs, now)]);
        assert_eq!(app.time_tags.all().iter().map(|tag| tag.label.as_str()).collect::<Vec<_>>(), ["Flood", "Tag 2"]);
        // Once the world has a save name the tag is stored at once, like every other.
        app.time_tags_after_save(std::path::Path::new("Savefiles/Placed.zip"));
        app.apply_ui_actions(&[UiAction::TimeTagStart(TagPanel::Graphs)]);
        app.apply_ui_actions(&[UiAction::TimeTagPlace(TagPanel::Graphs, now - 30.0)]);
        assert_eq!(bibites_io::storage::read_time_tags(&app.storage, "Placed").unwrap().tags.len(), 3);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_blank_name_becomes_tag_n_and_pressing_the_button_again_adds_the_tag() {
        let (mut app, root) = app("blank");
        app.apply_ui_actions(&[UiAction::TimeTagStart(TagPanel::Graphs)]);
        app.apply_ui_actions(&[UiAction::TimeTagField]);
        assert!(app.time_tag_edit.is_some(), "a press on the field keeps it open");
        app.apply_ui_actions(&[UiAction::TimeTagStart(TagPanel::Graphs)]);
        assert!(app.time_tag_edit.is_none());
        assert_eq!(app.time_tags.all().iter().map(|tag| tag.label.as_str()).collect::<Vec<_>>(), ["Tag 1"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn escape_and_a_press_on_anything_else_close_the_field_without_a_tag() {
        let (mut app, root) = app("close");
        app.apply_ui_actions(&[UiAction::TimeTagStart(TagPanel::Graphs)]);
        app.typed = "never set".to_owned();
        key(&mut app, FrameInput::default());
        key(&mut app, FrameInput { quit: true, ..FrameInput::default() });
        assert!(app.time_tag_edit.is_none());
        assert!(app.time_tags.is_empty());
        assert!(app.panels.is_open(PanelKind::Graphs), "Escape closed the field, not the panel");
        app.apply_ui_actions(&[UiAction::TimeTagStart(TagPanel::Graphs)]);
        app.apply_ui_actions(&[UiAction::SetGraphsScale(0)]);
        assert!(app.time_tag_edit.is_none());
        assert!(app.time_tags.is_empty());
        // The page changes: the field does not stay behind.
        app.apply_ui_actions(&[UiAction::TimeTagStart(TagPanel::Graphs)]);
        app.graphs_genes = true;
        key(&mut app, FrameInput::default());
        assert!(app.time_tag_edit.is_none(), "the field belongs to the data graphs page");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn each_panel_has_its_own_show_switch_that_is_stored_and_a_tag_is_seen_where_it_was_set() {
        let (mut app, root) = app("switches");
        for panel in [TagPanel::Graphs, TagPanel::Genes, TagPanel::Lineage] {
            assert!(app.time_tags_shown(panel), "shown until the player hides them");
        }
        app.apply_ui_actions(&[UiAction::TimeTagsShow(TagPanel::Graphs)]);
        assert!(!app.time_tags_shown(TagPanel::Graphs));
        assert!(app.time_tags_shown(TagPanel::Genes) && app.time_tags_shown(TagPanel::Lineage), "the other panels keep theirs");
        let stored = read_time_tag_view(&app.storage).unwrap();
        assert_eq!((stored.graphs, stored.genes, stored.lineage), (false, true, true));
        // Setting a tag in a panel that hides them shows them there again.
        app.apply_ui_actions(&[UiAction::TimeTagStart(TagPanel::Graphs)]);
        assert!(app.time_tags_shown(TagPanel::Graphs));
        app.apply_ui_actions(&[UiAction::TimeTagStart(TagPanel::Graphs)]);
        assert_eq!(app.time_tags.len(), 1);
        app.apply_ui_actions(&[UiAction::TimeTagsShow(TagPanel::Lineage)]);
        assert!(!read_time_tag_view(&app.storage).unwrap().lineage);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_tag_can_be_removed_and_removing_a_missing_one_does_nothing() {
        let (mut app, root) = app("remove");
        let first = app.time_tags.add(1.0, "one").unwrap();
        let second = app.time_tags.add(2.0, "two").unwrap();
        app.apply_ui_actions(&[UiAction::TimeTagRemove(first)]);
        assert_eq!(app.time_tags.all().iter().map(|tag| tag.id).collect::<Vec<_>>(), [second]);
        app.apply_ui_actions(&[UiAction::TimeTagRemove(first)]);
        assert_eq!(app.time_tags.len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_tags_follow_the_save_name_and_a_load_brings_them_back_without_the_ones_of_a_later_timeline() {
        let (mut app, root) = app("saves");
        let now = app.simulated_time();
        // No save name yet: nothing is stored, the tags live in the session.
        app.panels.open(PanelKind::Lineage);
        app.apply_ui_actions(&[UiAction::TimeTagStart(TagPanel::Lineage)]);
        app.typed = "Flood".to_owned();
        key(&mut app, FrameInput::default());
        key(&mut app, FrameInput { enter: true, ..FrameInput::default() });
        assert_eq!(app.time_tags.len(), 1);
        assert!(!root.join("time_tags").exists());
        // A save under a name: its tags are stored under that name, and edits go on being stored there.
        app.time_tags_after_save(std::path::Path::new("Savefiles/My save.zip"));
        let file = root.join("time_tags").join("My save.json");
        assert!(file.is_file());
        let id = app.time_tags.add(now, "Second").unwrap();
        app.time_tags_persist();
        assert_eq!(bibites_io::storage::read_time_tags(&app.storage, "My save").unwrap().tags.len(), 2);
        // A tag from after the save (the player went back to this older save) is not brought back; the others are.
        let mut stored = bibites_io::storage::read_time_tags(&app.storage, "My save").unwrap();
        stored.tags.push(TimeTagRecord { id: 99, seconds: now + 5_000.0, label: "from the future".to_owned() });
        bibites_io::storage::write_time_tags(&app.storage, "My save", &stored).unwrap();
        app.time_tags_reset();
        assert!(app.time_tags.is_empty() && app.time_tags_key.is_none());
        app.time_tags_after_load(std::path::Path::new("Savefiles/My save.zip"));
        let labels: Vec<_> = app.time_tags.all().iter().map(|tag| tag.label.as_str()).collect();
        assert_eq!(labels.len(), 2, "{labels:?}");
        assert!(labels.contains(&"Flood") && labels.contains(&"Second") && app.time_tags.get(id).is_some());
        assert_eq!(app.time_tags_key.as_deref(), Some("My save"));
        // Another save, or one that has no tags: none.
        app.time_tags_after_load(std::path::Path::new("Savefiles/Other.zip"));
        assert!(app.time_tags.is_empty());
        // A world that took the place of this one starts without tags or a name.
        app.time_tags_after_load(std::path::Path::new("Savefiles/My save.zip"));
        let loaded = LoadedWorld::load(None, 5, 3, 1).expect("starter world");
        app.adopt_world(loaded);
        assert!(app.time_tags.is_empty() && app.time_tags_key.is_none() && app.time_tag_edit.is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_autosave_gets_its_own_copy_of_the_tags_and_the_copies_of_deleted_autosaves_are_cleaned_away() {
        let (mut app, root) = app("autosave");
        app.time_tags.add(10.0, "tag");
        app.time_tags_beside_autosave("20261117010101");
        let first = root.join("time_tags").join("autosave_20261117010101.json");
        assert!(first.is_file());
        assert!(app.time_tags_key.is_none(), "an autosave does not rename the world's tags");
        // Its zip exists: the copy stays when the next autosave is made. A copy whose zip is gone goes.
        std::fs::create_dir_all(app.storage.autosaves_dir()).unwrap();
        std::fs::write(app.storage.autosaves_dir().join("autosave_20261117010101.zip"), b"zip").unwrap();
        app.time_tags_beside_autosave("20261117020202");
        assert!(first.is_file());
        std::fs::remove_file(app.storage.autosaves_dir().join("autosave_20261117010101.zip")).unwrap();
        app.time_tags_beside_autosave("20261117030303");
        assert!(!first.exists(), "its autosave was deleted");
        assert!(root.join("time_tags").join("autosave_20261117030303.json").is_file());
        // A manual save's tags are never taken for an autosave's.
        app.time_tags_after_save(std::path::Path::new("Savefiles/viewer-1.zip"));
        app.time_tags_beside_autosave("20261117040404");
        assert!(root.join("time_tags").join("viewer-1.json").is_file());
        let _ = std::fs::remove_dir_all(root);
    }
}

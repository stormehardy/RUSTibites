//! RUSTibites Improved: the Streams section of the Zones tab: adding, editing and deleting the world's streams, and keeping them beside the save (`streams/<save name>.json`
//! in the state folder; never inside a save, which stays in the game's format: the original game has no such thing).
//!
//! A stream is data of the world (`World::streams`, see `bibites_core::stream`); what the cards of the Zones tab write goes through here.

use bibites_core::math::Vec2;
use bibites_core::stream::Stream;
use bibites_io::storage::{StreamRecord, StreamsFile, read_streams, write_streams};

use super::App;
use crate::dyn_panel::FieldRef;
use crate::zone_editor as ze;

/// The list the streams are in (`DynState.open_items`, `UiAction::ZoneOpen`).
pub(crate) const STREAM_LIST: u8 = 4;

/// The stored form of a stream.
fn record_of(stream: &Stream) -> StreamRecord {
    StreamRecord {
        start_x: stream.start.x,
        start_y: stream.start.y,
        end_x: stream.end.x,
        end_y: stream.end.y,
        width: stream.width,
        strength: stream.strength,
    }
}

/// A stream from its stored form (every value is kept inside its range, whatever the file says; a value that is not a number leaves the default).
fn stream_of(record: &StreamRecord) -> Stream {
    let mut stream = Stream::new(Vec2::new(record.start_x, record.start_y), Vec2::new(record.end_x, record.end_y));
    stream.set_width(record.width);
    stream.set_strength(record.strength);
    stream
}

impl App {
    /// The `+` of the Streams list: a stream across the middle of what the camera shows, from left to right, with the default width and strength, and its card open.
    pub(crate) fn add_stream(&mut self) {
        let size = self.world.settings.independents.simulation_size;
        // A third of the view's width either side of its centre (at least 50 u), kept inside the world.
        let view_width = self.camera.screen.x / self.camera.zoom.max(crate::render::Camera::MIN_ZOOM);
        let half = (view_width / 6.0).max(50.0).min(size * 0.5);
        let centre = Vec2::new(self.camera.center.x.clamp(-size + half, size - half), self.camera.center.y.clamp(-size, size));
        let stream = Stream::new(Vec2::new(centre.x - half, centre.y), Vec2::new(centre.x + half, centre.y));
        self.world.streams.push(stream);
        let index = self.world.streams.len() - 1;
        self.dyn_panel.open_items.push((STREAM_LIST, index as u16));
        self.last_action = format!("added stream {}", index + 1);
    }

    /// A stream's delete button: the stream goes at once (as a zone group does).
    pub(crate) fn delete_stream(&mut self, index: usize) {
        if index >= self.world.streams.len() {
            return;
        }
        self.world.streams.remove(index);
        let at = index as u16;
        self.dyn_panel.open_items.retain(|item| *item != (STREAM_LIST, at));
        for item in &mut self.dyn_panel.open_items {
            if item.0 == STREAM_LIST && item.1 > at {
                item.1 -= 1;
            }
        }
        self.stream_edit_ended();
        self.last_action = format!("deleted stream {}", index + 1);
    }

    /// The Streams list's "remove all" button, once confirmed.
    pub(crate) fn clear_streams(&mut self) {
        self.world.streams.clear();
        self.dyn_panel.open_items.retain(|item| item.0 != STREAM_LIST);
        self.stream_edit_ended();
        self.last_action = "removed all streams".to_owned();
    }

    /// A slider that was being dragged or a value that was being typed may belong to a stream that is gone: both end.
    fn stream_edit_ended(&mut self) {
        let of_a_stream = |reference: u16| ze::stream_index(reference).is_some();
        if self.dyn_panel.zone_drag.is_some_and(|(reference, ..)| of_a_stream(reference)) {
            self.dyn_panel.zone_drag = None;
        }
        if self.value_edit.as_ref().is_some_and(|edit| matches!(edit.field, FieldRef::Zone(reference, _) if of_a_stream(reference))) {
            self.value_edit = None;
        }
    }

    /// A stream slider follows the cursor: `fraction` is where on its track the cursor is (0 to 1).
    pub(crate) fn stream_slide(&mut self, index: usize, field: u8, fraction: f32) {
        let size = self.world.settings.independents.simulation_size;
        let Some(info) = ze::stream_slider_info(field, size) else {
            return;
        };
        let value = crate::dyn_panel::value_range(info.mode, info.min, info.max, info.default, fraction.clamp(0.0, 1.0));
        self.stream_set(index, field, value);
    }

    /// Writes a value into a field of stream `index` (a value outside the field's range is held to it).
    pub(crate) fn stream_set(&mut self, index: usize, field: u8, value: f32) {
        let size = self.world.settings.independents.simulation_size;
        if let Some(stream) = self.world.streams.get_mut(index) {
            ze::set_stream_value(stream, field, value, size);
        }
    }

    /// What typing a value into a stream's slider follows, and the value it holds now (`None` when the stream or the field is gone).
    pub(crate) fn stream_typed_rules(&self, index: usize, field: u8) -> Option<(crate::value_edit::Rules, f32)> {
        let size = self.world.settings.independents.simulation_size;
        let info = ze::stream_slider_info(field, size)?;
        let stream = self.world.streams.get(index)?;
        Some((crate::dyn_panel::info_rules(&info, false), ze::stream_value_of(stream, field)))
    }

    /// The streams of the world, as stored.
    fn streams_file(&self) -> StreamsFile {
        StreamsFile {
            streams: self.world.streams.iter().map(record_of).collect(),
        }
    }

    /// Stores the streams under the name of the save the world was saved or loaded as (nothing is stored for a world that has no such name yet).
    pub(crate) fn streams_persist(&mut self) {
        let Some(key) = self.time_tags_key.clone() else {
            return;
        };
        if let Err(error) = write_streams(&self.storage, &key, &self.streams_file()) {
            self.last_action = format!("streams not saved: {error}");
        }
    }

    /// The world was loaded from the save `stem`: the world's streams are the ones stored for it (none stored: none).
    pub(crate) fn streams_after_load(&mut self, stem: &str) {
        self.dyn_panel.open_items.retain(|item| item.0 != STREAM_LIST);
        self.stream_edit_ended();
        match read_streams(&self.storage, stem) {
            Ok(file) => self.world.streams = file.streams.iter().map(stream_of).collect(),
            Err(error) => {
                self.world.streams.clear();
                self.last_action = format!("streams of {stem} not read: {error}");
            }
        }
    }

    /// An autosave is about to be written as `autosave_<stamp>.zip`: the streams are stored beside it under its name (the world's own key stays).
    pub(crate) fn streams_beside_autosave(&mut self, stamp: &str) {
        let file = self.streams_file();
        if file.streams.is_empty() {
            return;
        }
        if let Err(error) = write_streams(&self.storage, &format!("autosave_{stamp}"), &file) {
            self.last_action = format!("streams of the autosave not saved: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{ConfirmKind, LoadedWorld};
    use crate::ui::{ControlCommand, UiAction};
    use bibites_io::storage::StoragePaths;

    /// An app with a state folder of its own (the tests must not touch the player's).
    fn app(name: &str) -> (App, std::path::PathBuf) {
        let loaded = LoadedWorld::load(None, 77, 6, 1).expect("starter world");
        let mut app = App::new(loaded, Vec2::new(1920.0, 1080.0), None);
        let root = std::env::temp_dir().join(format!("rustibites_streams_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp folder");
        app.storage = StoragePaths::new(&root);
        (app, root)
    }

    #[test]
    fn the_plus_makes_a_stream_across_the_view_with_the_default_width_and_strength_and_opens_its_card() {
        let (mut app, root) = app("add");
        assert!(app.world.streams.is_empty());
        app.camera.center = Vec2::new(40.0, -25.0);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::AddStream)]);
        assert_eq!(app.world.streams.len(), 1);
        let stream = app.world.streams[0];
        assert_eq!(stream.start.y, -25.0);
        assert_eq!(stream.end.y, -25.0);
        assert!(stream.start.x < 40.0 && stream.end.x > 40.0, "{stream:?}: from the left of the view's centre to the right of it");
        assert_eq!((stream.width, stream.strength), (bibites_core::stream::STREAM_WIDTH_DEFAULT, bibites_core::stream::STREAM_STRENGTH_DEFAULT));
        assert!(stream.is_active());
        assert!(app.dyn_panel.open_items.contains(&(STREAM_LIST, 0)), "the new card is open");
        // A camera far outside the world: the stream is still inside it and still has a direction.
        let size = app.world.settings.independents.simulation_size;
        app.camera.center = Vec2::new(1.0e7, -1.0e7);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::AddStream)]);
        let second = app.world.streams[1];
        for value in [second.start.x, second.start.y, second.end.x, second.end.y] {
            assert!(value.abs() <= size, "{second:?}");
        }
        assert!(second.is_active(), "{second:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_cards_sliders_and_typed_values_edit_the_stream_and_keep_every_value_in_range() {
        use crate::zone_editor::{STREAM_END_X, STREAM_START_Y, STREAM_STRENGTH, STREAM_WIDTH, stream_ref};
        let (mut app, root) = app("edit");
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::AddStream)]);
        let size = app.world.settings.independents.simulation_size;
        // The slider's ends: the left end of a coordinate is -size, the right end size; the strength's are 0 and 500.
        app.zone_slide(usize::from(stream_ref(0)), STREAM_START_Y, (100.0, 500.0), 100.0 * crate::ui::UiFrame::scaler(app.camera.screen.x, app.camera.screen.y));
        assert!((app.world.streams[0].start.y + size).abs() < 1.0, "{:?}", app.world.streams[0]);
        app.stream_slide(0, STREAM_END_X, 1.0);
        assert_eq!(app.world.streams[0].end.x, size);
        app.stream_slide(0, STREAM_STRENGTH, 0.5);
        assert!((app.world.streams[0].strength - 250.0).abs() < 0.5);
        app.stream_slide(0, STREAM_STRENGTH, 7.0);
        assert_eq!(app.world.streams[0].strength, bibites_core::stream::STREAM_STRENGTH_MAX, "a fraction past the end is held to it");
        // A typed value: the rules say what the row accepts and what it holds now; a value past the ends is held to them.
        let field = crate::dyn_panel::FieldRef::Zone(stream_ref(0), STREAM_WIDTH);
        let (_, current) = app.typed_rules(field).expect("rules of a stream's width");
        assert_eq!(current, bibites_core::stream::STREAM_WIDTH_DEFAULT);
        app.apply_typed_value(field, 640.0);
        assert_eq!(app.world.streams[0].width, 640.0);
        app.apply_typed_value(field, 1.0e9);
        assert_eq!(app.world.streams[0].width, bibites_core::stream::STREAM_WIDTH_MAX);
        // A stream that is not there: nothing to edit, nothing to type into.
        assert!(app.typed_rules(crate::dyn_panel::FieldRef::Zone(stream_ref(5), STREAM_WIDTH)).is_none());
        app.apply_typed_value(crate::dyn_panel::FieldRef::Zone(stream_ref(5), STREAM_WIDTH), 50.0);
        assert_eq!(app.world.streams.len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn deleting_a_stream_closes_its_card_and_shifts_the_cards_after_it_and_remove_all_asks_first() {
        let (mut app, root) = app("delete");
        for _ in 0..3 {
            app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::AddStream)]);
        }
        app.dyn_panel.open_items.retain(|item| item.0 != STREAM_LIST);
        app.dyn_panel.open_items.extend([(STREAM_LIST, 0), (STREAM_LIST, 2), (0, 2)]);
        app.world.streams[2].set_strength(321.0);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::DeleteStream(1))]);
        assert_eq!(app.world.streams.len(), 2);
        assert_eq!(app.world.streams[1].strength, 321.0, "the third stream is the second now");
        assert!(app.dyn_panel.open_items.contains(&(STREAM_LIST, 0)) && app.dyn_panel.open_items.contains(&(STREAM_LIST, 1)));
        assert!(!app.dyn_panel.open_items.contains(&(STREAM_LIST, 2)) && app.dyn_panel.open_items.contains(&(0, 2)), "the zones' cards are not the streams'");
        // A stream that is not there: nothing happens.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::DeleteStream(9))]);
        assert_eq!(app.world.streams.len(), 2);
        // Remove all: a question first; "No" leaves them, "Yes" takes them all.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::RequestClearStreams)]);
        assert_eq!(app.confirm_dialog, Some(ConfirmKind::ClearStreams));
        assert_eq!(app.world.streams.len(), 2, "nothing goes before the answer");
        app.confirm_pending();
        assert!(app.world.streams.is_empty());
        assert!(app.dyn_panel.open_items.iter().all(|item| item.0 != STREAM_LIST) && app.dyn_panel.open_items.contains(&(0, 2)));
        // The button is disabled while the list is empty.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::RequestClearStreams)]);
        assert_eq!(app.confirm_dialog, None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_streams_are_kept_beside_the_save_come_back_with_it_and_an_autosave_gets_its_own_copy() {
        let (mut app, root) = app("persist");
        app.world.streams.push(Stream::new(Vec2::new(-300.0, 20.0), Vec2::new(400.0, -60.0)));
        let mut second = Stream::new(Vec2::new(0.0, 0.0), Vec2::new(0.0, 900.0));
        second.set_width(333.0);
        second.set_strength(42.0);
        app.world.streams.push(second);
        // No save name yet: nothing is stored.
        app.streams_persist();
        assert!(!app.storage.streams_path("My world").exists());
        app.time_tags_key = Some("My world".to_owned());
        app.streams_persist();
        assert!(app.storage.streams_path("My world").exists());
        // An autosave gets a file of its own and the world keeps its key.
        app.streams_beside_autosave("2026-11-18_10-00");
        assert!(app.storage.streams_path("autosave_2026-11-18_10-00").exists());
        assert_eq!(app.time_tags_key.as_deref(), Some("My world"));
        // Loading puts the stored streams in the world, whatever it had; a save with none stored has none.
        let kept = app.world.streams.clone();
        app.world.streams.clear();
        app.world.streams.push(Stream::new(Vec2::new(1.0, 1.0), Vec2::new(2.0, 2.0)));
        app.dyn_panel.open_items.push((STREAM_LIST, 0));
        app.streams_after_load("My world");
        assert_eq!(app.world.streams, kept);
        assert!(app.dyn_panel.open_items.iter().all(|item| item.0 != STREAM_LIST), "the cards of the old list are shut");
        app.streams_after_load("Another save");
        assert!(app.world.streams.is_empty());
        // A damaged file leaves no streams and says so; a stream whose width is out of range is held to it.
        std::fs::create_dir_all(app.storage.streams_path("Broken").parent().unwrap()).unwrap();
        std::fs::write(app.storage.streams_path("Broken"), "not json").unwrap();
        app.streams_after_load("Broken");
        assert!(app.world.streams.is_empty() && app.last_action.contains("not read"), "{}", app.last_action);
        std::fs::write(app.storage.streams_path("Wild"), r#"{"streams":[{"start_x":0,"start_y":0,"end_x":10,"end_y":0,"width":1e30,"strength":-5}]}"#).unwrap();
        app.streams_after_load("Wild");
        assert_eq!((app.world.streams[0].width, app.world.streams[0].strength), (bibites_core::stream::STREAM_WIDTH_MAX, 0.0));
        let _ = std::fs::remove_dir_all(root);
    }
}

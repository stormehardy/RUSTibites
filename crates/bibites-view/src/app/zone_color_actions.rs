//! RUSTibites Improved: giving zones colours (`crate::zone_colors`): the View panel's check box, the colour pop-up's actions (the wheel and the brightness bar follow
//! the pointer, the hex field takes typing), and keeping the colours beside the save (`zone_colors/<save name>.json` in the state folder, by the zones' names; never
//! inside a save, which stays in the game's format).

use bibites_core::math::Vec2;
use bibites_io::storage::{ZoneColorRecord, ZoneColorsFile, read_zone_colors, write_zone_colors};

use super::{App, FrameInput};
use crate::text_field::{self, Caret};
use crate::ui::UiFrame;
use crate::user_settings::{UserSettings, Value, index_of};
use crate::zone_color_ui::{self, PopupDrag, ZoneColorPopup};
use crate::zone_colors::{self, hex_of, parse_hex};

impl App {
    /// The View panel's "Individual Zone Colors" check box: the colours the zones were given show, or they do not (the zones keep them either way).
    pub(crate) fn toggle_zone_colors(&mut self) {
        self.set_zone_colors_shown(!self.scene_options.zone_colors);
    }

    /// Shows or hides the colours of the zones, and keeps the choice with the other settings, so that the next run starts as this one ended.
    fn set_zone_colors_shown(&mut self, shown: bool) {
        self.scene_options.zone_colors = shown;
        self.last_action = format!("individual zone colors {}", if shown { "shown" } else { "hidden" });
        if let Some(path) = self.user_settings_path.clone()
            && let Some(index) = index_of("Individually Coloured Zones")
        {
            UserSettings::load(Some(path)).set(index, Value::Bool(shown));
        }
    }

    /// The colour a zone without a colour of its own is drawn in (the Fertility Color).
    fn zone_base_color(&self) -> [u8; 3] {
        self.assets.as_ref().map_or([231, 154, 0], |assets| assets.background.zone_base)
    }

    /// A click on zone `zone` in the Preview: its colour pop-up opens where the pointer is, starting from the zone's colour.
    pub(crate) fn zone_color_open(&mut self, zone: usize) {
        let Some(data) = self.world.zones.zones.get(zone) else {
            return;
        };
        let start = data.tint.unwrap_or_else(|| self.zone_base_color());
        self.zone_color_popup = Some(ZoneColorPopup::new(zone, self.cursor, data.tint, start));
    }

    /// The colour being made is the zone's (the world and the Preview show it at once); with `rewrite_hex` the hex field's text follows it. A hidden set of colours
    /// is shown again, so that the choice is seen.
    fn zone_color_apply(&mut self, rewrite_hex: bool) {
        let Some(popup) = self.zone_color_popup.as_mut() else {
            return;
        };
        let rgb = popup.rgb();
        if rewrite_hex {
            popup.hex = hex_of(rgb);
            popup.caret = Caret::at_end(&popup.hex);
        }
        let zone = popup.zone;
        if let Some(data) = self.world.zones.zones.get_mut(zone) {
            data.tint = Some(rgb);
        }
        if !self.scene_options.zone_colors {
            self.set_zone_colors_shown(true);
        }
    }

    /// The pointer went down on the pop-up's wheel (or its brightness bar when `on_value`): it holds it from now on, and the keyboard leaves the hex field.
    pub(crate) fn zone_color_press(&mut self, on_value: bool) {
        if let Some(popup) = self.zone_color_popup.as_mut() {
            popup.drag = Some(if on_value { PopupDrag::Value } else { PopupDrag::Wheel });
            popup.hex_focus = false;
        }
    }

    /// The held wheel (or bar) follows the pointer (`cursor`, screen px on a canvas `canvas` px big): the wheel gives the hue by the angle from its middle and the
    /// saturation by the distance (the rim holds past it), the bar the brightness.
    pub(crate) fn zone_color_drag_to(&mut self, cursor: Vec2, canvas: Vec2) {
        let Some(popup) = self.zone_color_popup.as_mut() else {
            return;
        };
        let Some(drag) = popup.drag else {
            return;
        };
        let s = UiFrame::scaler(canvas.x, canvas.y);
        let l = zone_color_ui::layout([popup.at.x / s, popup.at.y / s], [canvas.x / s, canvas.y / s]);
        let point = [cursor.x / s, cursor.y / s];
        match drag {
            PopupDrag::Wheel => {
                let (centre, radius) = zone_color_ui::wheel_circle(&l);
                let (hue, saturation) = zone_colors::wheel_to_hs(point[0] - centre[0], point[1] - centre[1], radius);
                popup.hsv[0] = hue;
                popup.hsv[1] = saturation;
            }
            PopupDrag::Value => popup.hsv[2] = ((point[0] - l.bar[0]) / l.bar[2]).clamp(0.0, 1.0),
        }
        self.zone_color_apply(true);
    }

    /// The pointer let go: the colour is kept beside the save.
    pub(crate) fn zone_color_release(&mut self) {
        if let Some(popup) = self.zone_color_popup.as_mut() {
            popup.drag = None;
        }
        self.zone_colors_persist();
    }

    /// A click on the hex field: the keyboard is in it, with its text selected.
    pub(crate) fn zone_color_focus_hex(&mut self) {
        if let Some(popup) = self.zone_color_popup.as_mut() {
            popup.hex_focus = true;
            popup.caret.select_all(&popup.hex);
            popup.idle = 0.0;
        }
    }

    /// The colour `rgb` was written into the hex field: the wheel and the bar move to it (a grey keeps the wheel's hue where it was).
    fn zone_color_from_hex(&mut self, rgb: [u8; 3]) {
        if let Some(popup) = self.zone_color_popup.as_mut() {
            let (hue, saturation, value) = zone_colors::rgb_to_hsv(rgb);
            popup.hsv = [if saturation > 0.0 && value > 0.0 { hue } else { popup.hsv[0] }, saturation, value];
        }
    }

    /// Typing into the pop-up's hex field while the keyboard is in it: hex digits go in (upper case, at most six; anything else typed or pasted is dropped), the
    /// usual editing keys work, six digits make a colour at once, Enter takes the colour (three digits count as `RGB`) and closes the pop-up. Called every frame,
    /// before the other fields take their share of what was typed.
    pub(super) fn edit_zone_color_hex(&mut self, input: &FrameInput) {
        if !self.zone_color_popup.as_ref().is_some_and(|popup| popup.hex_focus) {
            return;
        }
        let keys = self.edit_keys(input);
        let mut typed = std::mem::take(&mut self.typed);
        if input.paste_tag {
            typed.push_str(&self.clipboard_in.take().unwrap_or_default());
        }
        let digits: String = typed.chars().filter(char::is_ascii_hexdigit).map(|c| c.to_ascii_uppercase()).collect();
        let Some(popup) = self.zone_color_popup.as_mut() else {
            return;
        };
        popup.idle += input.dt;
        let before = (popup.hex.clone(), popup.caret);
        text_field::edit(&mut popup.hex, &mut popup.caret, &digits, 6, keys);
        if (popup.hex.clone(), popup.caret) != before {
            popup.idle = 0.0;
        }
        let written = (popup.hex != before.0).then(|| popup.hex.clone());
        let on_enter = input.enter.then(|| popup.hex.clone());
        if let Some(rgb) = written.filter(|text| text.len() == 6).and_then(|text| parse_hex(&text)) {
            self.zone_color_from_hex(rgb);
            self.zone_color_apply(false);
        }
        if let Some(text) = on_enter {
            if let Some(rgb) = parse_hex(&text) {
                self.zone_color_from_hex(rgb);
                self.zone_color_apply(true);
                self.zone_color_close();
            }
        }
    }

    /// The pop-up closes and the zone keeps the colour it was given (Done, a click outside it).
    pub(crate) fn zone_color_close(&mut self) {
        if self.zone_color_popup.take().is_some() {
            self.zone_colors_persist();
        }
    }

    /// Escape: the pop-up closes and the zone has the colour it had when the pop-up opened again. `false` when no pop-up was open.
    pub(crate) fn zone_color_cancel(&mut self) -> bool {
        let Some(popup) = self.zone_color_popup.take() else {
            return false;
        };
        if let Some(data) = self.world.zones.zones.get_mut(popup.zone) {
            data.tint = popup.original;
        }
        self.zone_colors_persist();
        true
    }

    /// "Fertility Color": the colour of zone `zone` is taken away and it is drawn in the Fertility Color again. The pop-up closes.
    pub(crate) fn zone_color_clear(&mut self, zone: usize) {
        self.zone_color_popup = None;
        let Some(data) = self.world.zones.zones.get_mut(zone) else {
            return;
        };
        data.tint = None;
        self.last_action = format!("zone \"{}\" is in the Fertility Color again", data.config.name);
        self.zone_colors_persist();
    }

    /// "All different": every zone of the world gets a colour of its own (`zone_colors::spread` of the Fertility Color: the first zone keeps it).
    pub(crate) fn zone_colors_spread(&mut self) {
        self.zone_color_popup = None;
        let base = self.zone_base_color();
        for (index, zone) in self.world.zones.zones.iter_mut().enumerate() {
            zone.tint = Some(zone_colors::spread(base, index));
        }
        self.last_action = "every zone has a colour of its own".to_owned();
        if !self.scene_options.zone_colors {
            self.set_zone_colors_shown(true);
        }
        self.zone_colors_persist();
    }

    /// The colours the world's zones have, as stored.
    fn zone_colors_file(&self) -> ZoneColorsFile {
        ZoneColorsFile {
            zones: self.world.zones.tints().into_iter().map(|(name, rgb)| ZoneColorRecord { name, rgb }).collect(),
        }
    }

    /// Stores the zones' colours under the name of the save the world was saved or loaded as (nothing is stored for a world that has no such name yet).
    pub(crate) fn zone_colors_persist(&mut self) {
        let Some(key) = self.time_tags_key.clone() else {
            return;
        };
        if let Err(error) = write_zone_colors(&self.storage, &key, &self.zone_colors_file()) {
            self.last_action = format!("zone colors not saved: {error}");
        }
    }

    /// The world was loaded from the save `stem`: its zones get their colours back (by name; a zone without one keeps the Fertility Color).
    pub(crate) fn zone_colors_after_load(&mut self, stem: &str) {
        self.zone_color_popup = None;
        match read_zone_colors(&self.storage, stem) {
            Ok(file) => {
                for zone in &mut self.world.zones.zones {
                    zone.tint = file.zones.iter().find(|record| record.name == zone.config.name).map(|record| record.rgb);
                }
            }
            Err(error) => self.last_action = format!("zone colors of {stem} not read: {error}"),
        }
    }

    /// An autosave is about to be written as `autosave_<stamp>.zip`: the zones' colours are stored beside it under its name (the world's own key stays).
    pub(crate) fn zone_colors_beside_autosave(&mut self, stamp: &str) {
        let file = self.zone_colors_file();
        if file.zones.is_empty() {
            return;
        }
        if let Err(error) = write_zone_colors(&self.storage, &format!("autosave_{stamp}"), &file) {
            self.last_action = format!("zone colors of the autosave not saved: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LoadedWorld;
    use crate::ui::{OverlayKind, UiAction};
    use bibites_io::storage::StoragePaths;

    const CANVAS: Vec2 = Vec2::new(1920.0, 1080.0);

    /// An app with a state folder of its own (the tests must not touch the player's) and a settings file of its own, and two zones.
    fn app(name: &str) -> (App, std::path::PathBuf) {
        let loaded = LoadedWorld::load(None, 77, 6, 1).expect("starter world");
        let mut app = App::new(loaded, CANVAS, None);
        let root = std::env::temp_dir().join(format!("rustibites_zone_colors_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp folder");
        app.storage = StoragePaths::new(&root);
        app.user_settings_path = Some(root.join("settings.json"));
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::AddZone)]);
        assert!(app.world.zones.len() >= 2, "two zones to tell apart");
        (app, root)
    }

    fn key(app: &mut App, input: FrameInput) {
        app.handle_input(&FrameInput { dt: 0.016, ..input });
    }

    /// The wheel's middle and radius and the brightness bar's rectangle on the pop-up that is open (reference px = screen px at 1920 x 1080).
    fn parts(app: &App) -> (([f32; 2], f32), [f32; 4]) {
        let popup = app.zone_color_popup.as_ref().expect("the pop-up is open");
        let l = zone_color_ui::layout([popup.at.x, popup.at.y], [CANVAS.x, CANVAS.y]);
        (zone_color_ui::wheel_circle(&l), l.bar)
    }

    #[test]
    fn the_check_box_hides_and_shows_the_colours_and_is_kept_in_the_user_settings() {
        let (mut app, root) = app("switch");
        assert!(app.scene_options.zone_colors, "shown from the start: a zone without a colour looks as ever");
        app.apply_ui_actions(&[UiAction::ToggleOverlay(OverlayKind::ZoneColors)]);
        assert!(!app.scene_options.zone_colors);
        let settings = root.join("settings.json");
        assert!(!UserSettings::load(Some(settings.clone())).flag("Individually Coloured Zones"), "kept for the next run");
        app.apply_ui_actions(&[UiAction::ToggleOverlay(OverlayKind::ZoneColors)]);
        assert!(app.scene_options.zone_colors && UserSettings::load(Some(settings)).flag("Individually Coloured Zones"));
        // The other view switches are not touched by it.
        assert!(app.scene_options.zones);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_popup_opens_at_the_pointer_from_the_zones_colour_and_closes_on_anything_but_itself_and_escape_puts_the_colour_back() {
        let (mut app, root) = app("popup");
        app.cursor = Vec2::new(700.0, 500.0);
        // A zone that is not there opens nothing.
        app.apply_ui_actions(&[UiAction::ZoneColorOpen(99)]);
        assert!(app.zone_color_popup.is_none());
        // A zone in the Fertility Color: the pop-up starts from that colour, and opening gives the zone nothing of its own.
        app.apply_ui_actions(&[UiAction::ZoneColorOpen(1)]);
        let popup = app.zone_color_popup.clone().expect("the pop-up is open");
        assert_eq!((popup.zone, popup.at, popup.original), (1, Vec2::new(700.0, 500.0), None));
        assert_eq!(popup.rgb(), [231, 154, 0]);
        assert_eq!(popup.hex, "E79A00");
        assert!(app.world.zones.zones[1].tint.is_none());
        // Its own clicks and its panel keep it open; anything else closes it.
        app.apply_ui_actions(&[UiAction::Consume]);
        assert!(app.zone_color_popup.is_some());
        app.apply_ui_actions(&[UiAction::TogglePause]);
        assert!(app.zone_color_popup.is_none());
        // A zone with a colour: the pop-up starts from it.
        app.world.zones.zones[0].tint = Some([10, 200, 30]);
        app.apply_ui_actions(&[UiAction::ZoneColorOpen(0)]);
        assert_eq!(app.zone_color_popup.as_ref().unwrap().hex, "0AC81E");
        assert_eq!(app.zone_color_popup.as_ref().unwrap().original, Some([10, 200, 30]));
        // Escape closes it and the zone has the colour it had when the pop-up opened, whatever was made since.
        app.zone_color_popup.as_mut().unwrap().hsv = [0.7, 1.0, 0.5];
        app.zone_color_apply(true);
        assert_ne!(app.world.zones.zones[0].tint, Some([10, 200, 30]));
        key(&mut app, FrameInput { quit: true, ..FrameInput::default() });
        assert!(app.zone_color_popup.is_none());
        assert_eq!(app.world.zones.zones[0].tint, Some([10, 200, 30]));
        // The same for a zone that had none: it has none again.
        app.apply_ui_actions(&[UiAction::ZoneColorOpen(1)]);
        app.zone_color_popup.as_mut().unwrap().hsv = [0.3, 0.5, 0.9];
        app.zone_color_apply(true);
        assert!(app.world.zones.zones[1].tint.is_some());
        assert!(app.zone_color_cancel());
        assert!(app.world.zones.zones[1].tint.is_none());
        assert!(!app.zone_color_cancel(), "nothing is open");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_wheel_gives_hue_and_saturation_by_where_it_is_held_the_bar_the_brightness_and_the_zone_has_the_colour_as_it_is_made() {
        let (mut app, root) = app("wheel");
        app.cursor = Vec2::new(500.0, 300.0);
        app.apply_ui_actions(&[UiAction::ZoneColorOpen(1)]);
        let ((centre, radius), bar) = parts(&app);
        let (cx, cy) = (centre[0], centre[1]);
        // Pressing on the wheel holds it; the pointer to the right of the middle, halfway out: hue 0 (red), saturation 0.5.
        app.zone_color_press(false);
        assert_eq!(app.zone_color_popup.as_ref().unwrap().drag, Some(PopupDrag::Wheel));
        let value = app.zone_color_popup.as_ref().unwrap().hsv[2];
        app.zone_color_drag_to(Vec2::new(cx + radius * 0.5, cy), CANVAS);
        let popup = app.zone_color_popup.clone().unwrap();
        assert!(popup.hsv[0].abs() < 1e-4 && (popup.hsv[1] - 0.5).abs() < 1e-4 && (popup.hsv[2] - value).abs() < 1e-6, "{:?}", popup.hsv);
        assert_eq!(app.world.zones.zones[1].tint, Some(popup.rgb()), "the zone has the colour as it is made");
        assert_eq!(popup.hex, hex_of(popup.rgb()), "the hex field follows the wheel");
        // Straight up (a quarter turn) and far outside the rim: hue 0.25, full saturation (the rim holds).
        app.zone_color_drag_to(Vec2::new(cx, cy - radius * 3.0), CANVAS);
        let popup = app.zone_color_popup.clone().unwrap();
        assert!((popup.hsv[0] - 0.25).abs() < 1e-4 && (popup.hsv[1] - 1.0).abs() < 1e-6, "{:?}", popup.hsv);
        // The brightness bar: a third of the way along gives a third; the ends hold.
        app.zone_color_press(true);
        assert_eq!(app.zone_color_popup.as_ref().unwrap().drag, Some(PopupDrag::Value));
        app.zone_color_drag_to(Vec2::new(bar[0] + bar[2] / 3.0, bar[1]), CANVAS);
        assert!((app.zone_color_popup.as_ref().unwrap().hsv[2] - 1.0 / 3.0).abs() < 1e-4);
        app.zone_color_drag_to(Vec2::new(bar[0] - 500.0, bar[1]), CANVAS);
        assert_eq!(app.zone_color_popup.as_ref().unwrap().hsv[2], 0.0);
        app.zone_color_drag_to(Vec2::new(bar[0] + bar[2] + 500.0, bar[1]), CANVAS);
        assert_eq!(app.zone_color_popup.as_ref().unwrap().hsv[2], 1.0);
        // Letting go keeps the colour and the pop-up (a click outside or Done closes it); nothing is held any more.
        app.zone_color_release();
        assert_eq!(app.zone_color_popup.as_ref().unwrap().drag, None);
        app.zone_color_drag_to(Vec2::new(cx, cy), CANVAS);
        assert_eq!(app.zone_color_popup.as_ref().unwrap().hsv[1], 1.0, "a pointer that holds nothing moves nothing");
        app.apply_ui_actions(&[UiAction::ZoneColorClose]);
        assert!(app.zone_color_popup.is_none() && app.world.zones.zones[1].tint.is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_colour_can_be_typed_into_the_hex_field_digits_only_live_at_six_digits_and_enter_takes_it() {
        let (mut app, root) = app("hex");
        app.apply_ui_actions(&[UiAction::ZoneColorOpen(1)]);
        // The keyboard is not in the field until it is clicked: what is typed is not for it.
        app.typed = "ff".to_owned();
        key(&mut app, FrameInput::default());
        assert_eq!(app.zone_color_popup.as_ref().unwrap().hex, "E79A00");
        app.apply_ui_actions(&[UiAction::ZoneColorHexField]);
        let popup = app.zone_color_popup.as_ref().unwrap();
        assert!(popup.hex_focus && popup.caret.selection().is_some(), "the text is selected");
        // The hotkeys wait (R would select a random bibite, a space pause) and the characters that are not digits are dropped; typing replaces the selection.
        let paused = app.paused;
        app.typed = "#12 zz3".to_owned();
        key(&mut app, FrameInput { select_random_bibite: true, toggle_pause: true, ..FrameInput::default() });
        assert!(app.selected.is_none() && app.paused == paused);
        assert_eq!(app.zone_color_popup.as_ref().unwrap().hex, "123");
        assert!(app.world.zones.zones[1].tint.is_none(), "three digits are not a colour yet");
        // The sixth digit makes the colour at once: the zone, the wheel and the bar have it.
        app.typed = "abc".to_owned();
        key(&mut app, FrameInput::default());
        let popup = app.zone_color_popup.clone().unwrap();
        assert_eq!(popup.hex, "123ABC");
        assert_eq!(app.world.zones.zones[1].tint, Some([0x12, 0x3A, 0xBC]));
        assert_eq!(popup.rgb(), [0x12, 0x3A, 0xBC]);
        // A seventh digit does not go in; Backspace takes one off and the zone keeps the last whole colour.
        app.typed = "9".to_owned();
        key(&mut app, FrameInput::default());
        assert_eq!(app.zone_color_popup.as_ref().unwrap().hex, "123ABC");
        key(&mut app, FrameInput { backspace: true, ..FrameInput::default() });
        assert_eq!(app.zone_color_popup.as_ref().unwrap().hex, "123AB");
        assert_eq!(app.world.zones.zones[1].tint, Some([0x12, 0x3A, 0xBC]));
        // Enter with a whole colour takes it and closes the pop-up; with three digits it is `RGB`.
        app.typed = "C".to_owned();
        key(&mut app, FrameInput::default());
        key(&mut app, FrameInput { enter: true, ..FrameInput::default() });
        assert!(app.zone_color_popup.is_none());
        assert_eq!(app.world.zones.zones[1].tint, Some([0x12, 0x3A, 0xBC]));
        app.apply_ui_actions(&[UiAction::ZoneColorOpen(1), UiAction::ZoneColorHexField]);
        app.typed = "f80".to_owned();
        key(&mut app, FrameInput::default());
        assert_eq!(app.world.zones.zones[1].tint, Some([0x12, 0x3A, 0xBC]), "three digits do not set it live");
        key(&mut app, FrameInput { enter: true, ..FrameInput::default() });
        assert_eq!(app.world.zones.zones[1].tint, Some([255, 136, 0]));
        assert!(app.zone_color_popup.is_none());
        // Enter with two digits is not a colour: the pop-up stays and the zone keeps its colour.
        app.apply_ui_actions(&[UiAction::ZoneColorOpen(1), UiAction::ZoneColorHexField]);
        app.typed = "7F".to_owned();
        key(&mut app, FrameInput { enter: true, ..FrameInput::default() });
        assert!(app.zone_color_popup.is_some());
        assert_eq!(app.world.zones.zones[1].tint, Some([255, 136, 0]));
        // A grey keeps the wheel's hue where it was (a grey has none), and the bar follows.
        app.zone_color_popup.as_mut().unwrap().hsv = [0.6, 0.5, 0.5];
        app.apply_ui_actions(&[UiAction::ZoneColorHexField]);
        app.typed = "808080".to_owned();
        key(&mut app, FrameInput::default());
        let popup = app.zone_color_popup.clone().unwrap();
        assert!((popup.hsv[0] - 0.6).abs() < 1e-6 && popup.hsv[1] == 0.0 && (popup.hsv[2] - 128.0 / 255.0).abs() < 1e-5, "{:?}", popup.hsv);
        // Pressing the wheel takes the keyboard out of the field.
        app.zone_color_press(false);
        assert!(!app.zone_color_popup.as_ref().unwrap().hex_focus);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_fertility_color_button_takes_the_colour_away_and_all_different_colours_every_zone() {
        let (mut app, root) = app("buttons");
        app.world.zones.zones[1].tint = Some([1, 2, 3]);
        app.apply_ui_actions(&[UiAction::ZoneColorOpen(1), UiAction::ZoneColorClear(1)]);
        assert!(app.zone_color_popup.is_none() && app.world.zones.zones[1].tint.is_none());
        assert!(app.last_action.contains("Fertility Color"), "{}", app.last_action);
        app.apply_ui_actions(&[UiAction::ZoneColorOpen(0), UiAction::ZoneColorSpread]);
        assert!(app.zone_color_popup.is_none());
        let colours: Vec<_> = app.world.zones.zones.iter().map(|zone| zone.tint).collect();
        assert!(colours.iter().all(Option::is_some));
        assert_eq!(colours[0], Some([231, 154, 0]), "the first zone keeps the Fertility Color");
        assert_ne!(colours[0], colours[1]);
        // Choosing with the colours hidden shows them again.
        app.apply_ui_actions(&[UiAction::ToggleOverlay(OverlayKind::ZoneColors)]);
        assert!(!app.scene_options.zone_colors);
        app.apply_ui_actions(&[UiAction::ZoneColorOpen(0)]);
        app.zone_color_press(false);
        app.zone_color_drag_to(Vec2::new(app.cursor.x + 100.0, app.cursor.y + 100.0), CANVAS);
        assert!(app.scene_options.zone_colors, "the choice is seen");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_colours_follow_the_save_name_are_found_again_by_the_zones_names_and_autosaves_get_their_own_copy() {
        let (mut app, root) = app("saves");
        app.world.zones.zones[1].config.0.name = "Second".to_owned();
        let give = |app: &mut App, zone: usize, rgb: [u8; 3]| {
            app.apply_ui_actions(&[UiAction::ZoneColorOpen(zone as u16)]);
            let popup = app.zone_color_popup.as_mut().unwrap();
            let (h, s, v) = zone_colors::rgb_to_hsv(rgb);
            popup.hsv = [h, s, v];
            app.zone_color_apply(true);
            app.apply_ui_actions(&[UiAction::ZoneColorClose]);
        };
        // No save name yet: nothing is stored, the colours live in the session.
        give(&mut app, 1, [10, 200, 30]);
        assert!(!root.join("zone_colors").exists());
        // A save under a name stores them, and a later choice is stored at once (when the pop-up closes).
        app.time_tags_after_save(std::path::Path::new("Savefiles/My save.zip"));
        let file = root.join("zone_colors").join("My save.json");
        assert!(file.is_file());
        let stored = read_zone_colors(&app.storage, "My save").unwrap().zones;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].name, "Second");
        assert!(stored[0].rgb.iter().zip([10u8, 200, 30]).all(|(a, b)| a.abs_diff(b) <= 1), "{:?}", stored[0].rgb);
        give(&mut app, 0, [255, 0, 128]);
        assert_eq!(read_zone_colors(&app.storage, "My save").unwrap().zones.len(), 2);
        // A colour taken away leaves the file.
        app.apply_ui_actions(&[UiAction::ZoneColorClear(0)]);
        assert_eq!(read_zone_colors(&app.storage, "My save").unwrap().zones.len(), 1);
        // A load finds the zones by their names (the order does not matter); another save has none.
        let own = app.world.zones.zones[1].tint;
        for zone in &mut app.world.zones.zones {
            zone.tint = None;
        }
        app.time_tags_after_load(std::path::Path::new("Savefiles/My save.zip"));
        let tints: Vec<_> = app.world.zones.zones.iter().map(|zone| (zone.config.name.clone(), zone.tint)).collect();
        assert_eq!(tints.iter().find(|(name, _)| name == "Second").unwrap().1, own);
        assert_eq!(tints.iter().filter(|(_, tint)| tint.is_some()).count(), 1, "{tints:?}");
        app.time_tags_after_load(std::path::Path::new("Savefiles/Other.zip"));
        assert!(app.world.zones.zones.iter().all(|zone| zone.tint.is_none()));
        // An autosave gets its own copy, and the world's own name stays.
        app.time_tags_after_load(std::path::Path::new("Savefiles/My save.zip"));
        app.time_tags_beside_autosave("20261117010101");
        assert!(root.join("zone_colors").join("autosave_20261117010101.json").is_file());
        assert_eq!(app.time_tags_key.as_deref(), Some("My save"));
        let _ = std::fs::remove_dir_all(root);
    }
}

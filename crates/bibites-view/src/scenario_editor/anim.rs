//! The scenario editor's text fields and its settings animators: typing a name into a zone, zone group or animator card (`TextLineHandle`),
//! typed slider values (`SettingSlider.OpenEditPanel`), the Animation list (the creator panel, the cards, the curve graph) and the cards' move
//! arrows. The animators themselves are edited by the shared [`Session`], as in the world's Animation tab.

use bibites_core::settings::SettingsChanger;

use super::{Drag, EditorPopup, EditorState};
use crate::animator_session::{Env, Session};
use crate::dyn_panel::{self, FieldRef};
use crate::ui::{ControlCommand, DropdownTarget, UiAction};
use crate::value_edit::{Content, ValueEdit};
use crate::zone_editor::{self as ze, ZoneRef};

/// The longest name that can be typed into a card (`TextLineHandle`'s `characterLimit`, as in the world).
const NAME_MAX_CHARS: usize = 30;

/// What the pointer was doing when an action fired: the Ctrl key, the cursor in menu pixels and the pixels per reference pixel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pointer {
    /// Ctrl is held.
    pub ctrl: bool,
    /// The cursor in menu pixels.
    pub cursor: [f32; 2],
    /// The control under the cursor (reference px: x, y, width, height), when there is one.
    pub rect: Option<[f32; 4]>,
    /// Shift is held.
    pub shift: bool,
    /// Menu pixels per reference pixel.
    pub scale: f32,
}

impl EditorState {
    /// The environment an animator action works in (`scale`: pixels per reference pixel of the cursor positions it is given).
    fn animator_env(&self, scale: f32) -> Env<'_> {
        Env { settings: &self.settings, zones: &self.scenario.zones, scale }
    }

    /// The settings animators the draft holds.
    #[must_use]
    pub fn changers(&self) -> &[SettingsChanger] {
        &self.scenario.settings_changers
    }

    /// The settings an animator can take over, for the Animation list's base items.
    #[must_use]
    pub fn animator_catalog(&self) -> Vec<crate::animators::Animatable> {
        crate::animators::catalog(&self.settings, &self.scenario.zones)
    }

    /// The choices and the current one of an animator dropdown.
    #[must_use]
    pub fn animator_dropdown(&self, target: DropdownTarget) -> Option<(Vec<crate::dropdown::Choice>, usize)> {
        let env = self.animator_env(1.0);
        self.animator.dropdown_choices(&self.scenario.settings_changers, &env, target)
    }

    /// A pick in an animator dropdown's list.
    pub fn animator_pick(&mut self, target: DropdownTarget, choice: usize) -> bool {
        let env = Env { settings: &self.settings, zones: &self.scenario.zones, scale: 1.0 };
        self.animator.dropdown_pick(&mut self.scenario.settings_changers, &env, target, choice)
    }

    /// `SettingsChangerCreator.Submit`: the animator is added to the draft and its card opens.
    fn submit_creator(&mut self) {
        let changer = self.animator.creator.build(None);
        self.animator.creator.open = false;
        if changer.validate().is_ok() {
            self.scenario.settings_changers.push(changer);
            self.open_items.push((3, (self.scenario.settings_changers.len() - 1) as u16));
        }
    }

    /// Animator `index` leaves the draft (its settings are free again).
    pub(super) fn delete_animator(&mut self, index: usize) {
        if index >= self.scenario.settings_changers.len() {
            return;
        }
        self.scenario.settings_changers.remove(index);
        self.animator.removed(index);
        self.dyn_state.card_removed(3, index);
        self.name_edit = None;
        self.open_items.retain(|item| *item != (3, index as u16));
        for item in &mut self.open_items {
            if item.0 == 3 && usize::from(item.1) > index {
                item.1 -= 1;
            }
        }
    }

    /// The actions of the text fields, the animators and the move arrows; `None` when `action` is none of them.
    pub(super) fn apply_text_and_animators(&mut self, action: UiAction, pointer: Pointer) -> Option<bool> {
        // A press on anything but the open field's own controls closes the open fields without applying them.
        if !matches!(action, UiAction::ZoneNameEdit(_) | UiAction::DynEditValue(_) | UiAction::ValueEditInside) {
            self.name_edit = None;
            self.value_edit = None;
        }
        match action {
            // A press inside the open value field puts the caret under the pointer (its text starts 12 in).
            UiAction::ValueEditInside => {
                self.press_in_field(pointer);
                Some(true)
            }
            UiAction::ZoneNameEdit(reference) => {
                self.value_edit = None;
                let name = self.name_of(reference)?;
                if self.name_edit.as_ref().is_some_and(|(edited, _)| *edited == reference) {
                    self.press_in_field(pointer);
                    return Some(true);
                }
                if self.name_edit.as_ref().is_none_or(|(edited, _)| *edited != reference) {
                    self.name_caret = crate::text_field::Caret::at_end(&name);
                    self.name_edit = Some((reference, name));
                }
                Some(true)
            }
            UiAction::DynEditValue(field) => {
                // The text of the point panel's value line is the field itself: a press on it moves the caret.
                if self.value_edit.as_ref().is_some_and(|edit| edit.field == field) {
                    self.press_in_field(pointer);
                    return Some(true);
                }
                let (rules, current) = self.typed_rules(field)?;
                self.name_edit = None;
                let content = if rules.integer { Content::Integer } else { Content::Decimal };
                self.value_edit = Some(ValueEdit::open(field, current * rules.factor, content));
                Some(true)
            }
            UiAction::ListMove(list, index, up) => {
                let len = self.list_len(list);
                self.dyn_state.move_card(list, len, usize::from(index), up);
                Some(true)
            }
            UiAction::Invoke(command) => match command {
                ControlCommand::OpenAnimatorCreator => {
                    self.animator.creator.open(None);
                    Some(true)
                }
                ControlCommand::CloseAnimatorCreator => {
                    self.animator.creator.open = false;
                    Some(true)
                }
                ControlCommand::SubmitAnimatorCreator => {
                    self.submit_creator();
                    Some(true)
                }
                ControlCommand::RequestClearAnimators => {
                    if !self.scenario.settings_changers.is_empty() {
                        self.popup = Some(EditorPopup::ClearList(3));
                    }
                    Some(true)
                }
                ControlCommand::DeleteAnimator(index) => {
                    if usize::from(index) < self.scenario.settings_changers.len() {
                        self.popup = Some(EditorPopup::DeleteItem(3, index));
                    }
                    Some(true)
                }
                ControlCommand::RemoveAnimatorBase(changer, base) => {
                    Session::remove_base(&mut self.scenario.settings_changers, usize::from(changer), usize::from(base));
                    Some(true)
                }
                ControlCommand::RemoveAnimatorPoint(changer, point) => {
                    self.animator.remove_point(&mut self.scenario.settings_changers, usize::from(changer), usize::from(point));
                    Some(true)
                }
                _ => None,
            },
            UiAction::AnimGraph(changer, x0, y0, x1, y1) => {
                let env = Env { settings: &self.settings, zones: &self.scenario.zones, scale: pointer.scale.max(f32::EPSILON) };
                self.animator.graph_press(
                    &mut self.scenario.settings_changers,
                    &env,
                    usize::from(changer),
                    [f32::from(x0), f32::from(y0), f32::from(x1), f32::from(y1)],
                    pointer.cursor,
                    pointer.ctrl,
                );
                Some(true)
            }
            UiAction::ZoneToggle(reference, ze::ANIM_REPEAT) if matches!(ze::zone_ref(reference), ZoneRef::Animator(_)) => {
                if let ZoneRef::Animator(index) = ze::zone_ref(reference) {
                    Session::toggle_repeat(&mut self.scenario.settings_changers, index);
                }
                Some(true)
            }
            UiAction::ZoneReset(reference, field) if matches!(ze::zone_ref(reference), ZoneRef::Animator(_) | ZoneRef::Creator) => {
                let env = Env { settings: &self.settings, zones: &self.scenario.zones, scale: 1.0 };
                if let Some((info, _)) = self.animator.slider(&self.scenario.settings_changers, &env, reference, field) {
                    self.animator.set_value(&mut self.scenario.settings_changers, &env, reference, field, info.default, true);
                }
                Some(true)
            }
            _ => None,
        }
    }

    /// The name of the card a name field belongs to.
    fn name_of(&self, reference: u16) -> Option<String> {
        match ze::zone_ref(reference) {
            ZoneRef::Animator(index) => self.scenario.settings_changers.get(index).map(|changer| changer.name.clone()),
            ZoneRef::Group(index) => self.scenario.zone_groups.get(index).map(|group| group.name.clone()),
            ZoneRef::Zone(index) => self.scenario.zones.get(index).map(|zone| zone.name.clone()),
            _ => None,
        }
    }

    /// Enter in a name field (`TextLineHandle.FieldSubmit`): the card is renamed.
    fn submit_name(&mut self) {
        let Some((reference, name)) = self.name_edit.take() else {
            return;
        };
        match ze::zone_ref(reference) {
            ZoneRef::Animator(index) => {
                crate::animator_session::edit(&mut self.scenario.settings_changers, index, |changer| changer.name = name.clone());
            }
            ZoneRef::Group(index) => self.edit_group(index, |group| group.name = name.clone()),
            ZoneRef::Zone(index) => {
                if let Some(zone) = self.scenario.zones.get_mut(index) {
                    zone.name = name;
                }
            }
            _ => {}
        }
    }

    /// What typing a value into slider row `field` follows and the value it holds now.
    fn typed_rules(&self, field: FieldRef) -> Option<(crate::value_edit::Rules, f32)> {
        use crate::dyn_panel::{info_rules, setting_rules};
        match field {
            FieldRef::Setting(index) => {
                let row = crate::setting_ui_data::SETTING_UI.get(usize::from(index))?;
                let (mode, _) = dyn_panel::slider_info_or_default(row);
                Some((setting_rules(row, mode), dyn_panel::read_value(&self.settings, row)))
            }
            FieldRef::Zone(index, field) => {
                let size = self.settings.independents.simulation_size;
                match ze::zone_ref(index) {
                    ZoneRef::Animator(_) | ZoneRef::Creator => {
                        let env = self.animator_env(1.0);
                        let (info, value) = self.animator.slider(&self.scenario.settings_changers, &env, index, field)?;
                        Some((info_rules(&info, false), value))
                    }
                    ZoneRef::Group(group) => {
                        let group = self.scenario.zone_groups.get(group)?;
                        let info = ze::group_slider_info(field)?;
                        Some((info_rules(&info, field == ze::GROUP_COUNT), ze::group_value_of(group, field)))
                    }
                    ZoneRef::Zone(zone) => {
                        let zone = self.scenario.zones.get(zone)?;
                        let info = ze::slider_info(zone, field, size)?;
                        Some((info_rules(&info, false), ze::value_of(zone, field)))
                    }
                    ZoneRef::Template(group) => {
                        let zone = &self.scenario.zone_groups.get(group)?.template;
                        let info = ze::slider_info(zone, field, size)?;
                        Some((info_rules(&info, false), ze::value_of(zone, field)))
                    }
                }
            }
            FieldRef::Material(kind, field) => {
                let material = ze::material_kind(kind);
                let info = ze::material_slider_info(material, field)?;
                let current = *self.settings.materials.get(material);
                Some((info_rules(&info, false), ze::material_value(&current, field)))
            }
            FieldRef::Bibite(index, field) => {
                let row = self.scenario.bibite_spawns.get(usize::from(index))?;
                let info = crate::bibite_editor::slider_info(field)?;
                Some((info_rules(&info, field == crate::bibite_editor::MINIMUM), crate::bibite_editor::value_of(row, field)))
            }
            _ => None,
        }
    }

    /// Writes a typed value (already divided by the factor) into the row it was typed for, as it is.
    fn apply_typed_value(&mut self, field: FieldRef, value: f32) {
        let size = self.settings.independents.simulation_size;
        match field {
            FieldRef::Setting(index) => {
                dyn_panel::set_value_balanced(&mut self.settings, usize::from(index), value);
            }
            FieldRef::Zone(index, field) => match ze::zone_ref(index) {
                ZoneRef::Animator(_) | ZoneRef::Creator => {
                    let env = Env { settings: &self.settings, zones: &self.scenario.zones, scale: 1.0 };
                    self.animator.set_value(&mut self.scenario.settings_changers, &env, index, field, value, false);
                }
                ZoneRef::Group(group) => self.edit_group(group, |group| ze::set_group_value_unclamped(group, field, value)),
                _ => {
                    if let Some(item) = self.zone_like_mut(index) {
                        ze::set_value_unclamped(item, field, value, size);
                    }
                }
            },
            FieldRef::Material(kind, field) => {
                let material = ze::material_kind(kind);
                ze::set_material_value_unclamped(self.settings.materials.get_mut(material), field, value);
            }
            FieldRef::Bibite(index, field) => {
                if let Some(row) = self.scenario.bibite_spawns.get_mut(usize::from(index)) {
                    crate::bibite_editor::set_value_unclamped(row, field, value);
                }
            }
            _ => {}
        }
    }

    /// Enter in the open edit field: a name renames its card, a value goes into its setting (a text that does not parse changes nothing).
    pub fn submit_edit(&mut self) {
        if self.name_edit.is_some() {
            self.submit_name();
            return;
        }
        let Some(edit) = self.value_edit.take() else {
            return;
        };
        let Some((rules, _)) = self.typed_rules(edit.field) else {
            return;
        };
        if let Some(value) = rules.submitted(&edit.text) {
            self.apply_typed_value(edit.field, value);
        }
    }

    /// What the open text field is typed in: its font size, the inset of its text in the control under the pointer, and its text.
    fn field_look(&self) -> Option<(f32, f32, String)> {
        if let Some((_, text)) = &self.name_edit {
            return Some((26.0, 0.0, text.clone()));
        }
        let edit = self.value_edit.as_ref()?;
        // The point panel's value line is the text itself (18 pt); a slider row's input field insets its text by 12 (16 pt).
        let line = matches!(edit.field, FieldRef::Zone(reference, ze::ANIM_POINT_TIME | ze::ANIM_POINT_VALUE) if matches!(ze::zone_ref(reference), ZoneRef::Animator(_)));
        Some(if line { (18.0, 0.0, edit.text.clone()) } else { (16.0, 12.0, edit.text.clone()) })
    }

    /// Measures the open text field's characters (the build does this each frame it shows one).
    pub fn measure_field(&self, measure: &dyn Fn(&str, f32) -> f32) {
        let Some((size, inset, text)) = self.field_look() else {
            *self.field_edges.borrow_mut() = (0.0, Vec::new());
            return;
        };
        let mut edges = vec![0.0];
        let mut prefix = String::new();
        for character in text.chars() {
            prefix.push(character);
            edges.push(measure(&prefix, size));
        }
        *self.field_edges.borrow_mut() = (inset, edges);
    }

    /// The character boundary nearest to the reference-px x `x` in a field whose text starts at `start`.
    fn place_of(&self, start: f32, x: f32) -> usize {
        let edges = self.field_edges.borrow();
        if edges.1.is_empty() {
            return 0;
        }
        crate::text_field::nearest_edge(&edges.1, x - start)
    }

    /// A press in the open text field: the caret goes under the pointer (Shift keeps the selection's anchor) and a drag may follow.
    fn press_in_field(&mut self, pointer: Pointer) {
        let Some(rect) = pointer.rect else {
            return;
        };
        let start = rect.first().copied().unwrap_or(0.0) + self.field_edges.borrow().0;
        let x = pointer.cursor[0] / pointer.scale.max(f32::EPSILON);
        let place = self.place_of(start, x);
        if let Some(edit) = self.value_edit.as_mut() {
            edit.caret.press(place, pointer.shift);
            edit.idle = 0.0;
        } else if self.name_edit.is_some() {
            self.name_caret.press(place, pointer.shift);
        } else {
            return;
        }
        self.caret_drag = Some(start);
    }

    /// The held mouse drags the selection's end (`x` in reference px). Returns whether a text press is held.
    pub fn drag_caret(&mut self, x: f32) -> bool {
        let Some(start) = self.caret_drag else {
            return false;
        };
        let place = self.place_of(start, x);
        if let Some(edit) = self.value_edit.as_mut() {
            edit.caret.drag(place);
        } else if self.name_edit.is_some() {
            self.name_caret.drag(place);
        }
        true
    }

    /// Whether a name or a value is being typed.
    #[must_use]
    pub fn field_open(&self) -> bool {
        self.name_edit.is_some() || self.value_edit.is_some()
    }

    /// Escape: the open field closes without applying; `true` when there was one.
    pub fn cancel_edit(&mut self) -> bool {
        let open = self.field_open();
        self.name_edit = None;
        self.value_edit = None;
        open
    }

    /// A typed character goes into the open field (a name at the caret, replacing the selection).
    pub fn type_into_field(&mut self, character: char) {
        if let Some((_, text)) = self.name_edit.as_mut() {
            let mut keys = crate::text_field::EditKeys::default();
            keys.shift = false;
            crate::text_field::edit(text, &mut self.name_caret, &character.to_string(), NAME_MAX_CHARS, keys);
        } else if let Some(edit) = self.value_edit.as_mut() {
            edit.type_char(character);
        }
    }

    /// Backspace in the open field.
    pub fn backspace_in_field(&mut self) {
        if let Some((_, text)) = self.name_edit.as_mut() {
            let keys = crate::text_field::EditKeys { backspace: true, ..Default::default() };
            crate::text_field::edit(text, &mut self.name_caret, "", NAME_MAX_CHARS, keys);
        } else if let Some(edit) = self.value_edit.as_mut() {
            edit.backspace();
        }
    }

    /// Delete / Left / Right / Home / End / Ctrl + A in the open field (Shift extends the selection).
    pub fn key_in_field(&mut self, key: Option<crate::value_edit::CaretKey>, delete: bool, shift: bool) {
        use crate::value_edit::CaretKey;
        if let Some((_, text)) = self.name_edit.as_mut() {
            let keys = crate::text_field::EditKeys {
                delete,
                left: key == Some(CaretKey::Left),
                right: key == Some(CaretKey::Right),
                home: key == Some(CaretKey::Home),
                end: key == Some(CaretKey::End),
                select_all: key == Some(CaretKey::SelectAll),
                shift,
                ..Default::default()
            };
            crate::text_field::edit(text, &mut self.name_caret, "", NAME_MAX_CHARS, keys);
        } else if let Some(edit) = self.value_edit.as_mut() {
            if delete {
                edit.delete_forward();
            }
            if let Some(key) = key {
                edit.move_caret(key, shift);
            }
        }
    }

    /// Pastes text into the open field (Ctrl + V).
    pub fn paste_into_field(&mut self, text: &str) {
        if let Some((_, field)) = self.name_edit.as_mut() {
            crate::text_field::edit(field, &mut self.name_caret, text, NAME_MAX_CHARS, crate::text_field::EditKeys::default());
        } else if let Some(edit) = self.value_edit.as_mut() {
            edit.paste(text);
        }
    }

    /// A frame passes: the open value field's caret blinks on (`dt` seconds). Returns whether its look changed.
    pub fn tick_field(&mut self, dt: f32) -> bool {
        let Some(edit) = self.value_edit.as_mut() else {
            return false;
        };
        let before = edit.caret_visible();
        edit.idle += dt;
        edit.caret_visible() != before
    }

    /// The held mouse drags a point of an animator's curve (`cursor` and `scale` in menu pixels).
    pub fn drag_graph(&mut self, cursor: [f32; 2], scale: f32) -> bool {
        if self.animator.drag.is_none() {
            return false;
        }
        let env = Env { settings: &self.settings, zones: &self.scenario.zones, scale: scale.max(f32::EPSILON) };
        self.animator.graph_drag(&mut self.scenario.settings_changers, &env, cursor);
        true
    }

    /// A slider of an animator or of the creator follows the cursor (`x` in reference px).
    pub(super) fn drag_animator_slider(&mut self, zone: u16, field: u8, track: (f32, f32), x: f32) {
        let env = Env { settings: &self.settings, zones: &self.scenario.zones, scale: 1.0 };
        self.animator.slide(&mut self.scenario.settings_changers, &env, zone, field, track, x);
    }

    /// The mouse is let go: the slider and the graph point stop following it.
    pub fn release_drags(&mut self) {
        self.caret_drag = None;
        self.drag = None;
        self.animator.drag = None;
    }
}

impl Drag {
    /// Whether this drag moves a slider of an animator or of the creator panel.
    #[must_use]
    pub fn is_animator(self) -> bool {
        matches!(self, Drag::Zone(zone, ..) if matches!(ze::zone_ref(zone), ZoneRef::Animator(_) | ZoneRef::Creator))
    }
}

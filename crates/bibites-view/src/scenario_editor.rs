//! The scenario editor (`ScenariosEditor`), opened from the New Game screen.
//!
//! The editor works on a *draft* of the selected scenario: the effective settings, the zone
//! list and groups, the settings animators and the challenge definition. Its tabs are lists of
//! setting groups bound to the draft (the same row prefabs as the in-world dynamic panel, so the
//! rows are cloned from the captured editor and given live labels and values) plus the zone,
//! default-bibite, animator and challenge editors. "Start" runs the draft, "Save" writes it to a
//! scenario file, "Back" asks before throwing it away.

use std::path::PathBuf;

use bibites_core::format::FloatValueFormat;
use bibites_core::settings::{ChallengeParameters, SimSettings};
use bibites_core::world::ScenarioData;
use bibites_core::zone::{ZoneConfig, ZoneGlobals, ZoneGroupConfig};

use crate::dyn_panel::{self, DynHit, DynInput, DynState, DynTip, ListGeometry, Row, TabGroup};
use crate::ui::{MenuCommand, UiAction};
use crate::ui_runtime::Node;

/// Number of tabs: simulation, zones, energy, bibites, animations, challenge.
pub const TAB_COUNT: usize = 6;
/// Simulation tab.
pub const TAB_SIM: u8 = 0;
/// Zones tab (world parameters, zones, zone groups).
pub const TAB_ZONES: u8 = 1;
/// Energy tab.
pub const TAB_ENERGY: u8 = 2;
/// Bibites tab (default bibites, genetics).
pub const TAB_BIBITES: u8 = 3;
/// Settings animators tab.
pub const TAB_ANIMS: u8 = 4;
/// Challenge options tab.
pub const TAB_CHALLENGE: u8 = 5;

/// GameObject names of the tab panels, in tab order.
const TAB_PANELS: [&str; TAB_COUNT] = [
    "SimTab",
    "ZoneEditorPanel",
    "EnergyTab",
    "BibiteTab",
    "AnimationsPanel",
    "ChallengeParameters",
];
/// `x0, x1` of the tab buttons (`LeftSide Buttons`), reference px; `y` is [`BUTTON_Y`].
const TAB_BUTTONS: [(f32, f32); TAB_COUNT] = [
    (25.0, 95.0),
    (111.0, 181.0),
    (197.0, 267.0),
    (283.0, 353.0),
    (369.0, 439.0),
    (1046.0, 1116.0),
];
/// Top and bottom of the header buttons.
const BUTTON_Y: (f32, f32) = (12.0, 82.0);
/// Start, Back and Save buttons (`Right Side Buttons`): `x0, x1`.
const START_BUTTON: (f32, f32) = (1825.0, 1895.0);
const BACK_BUTTON: (f32, f32) = (1745.0, 1815.0);
const SAVE_BUTTON: (f32, f32) = (1630.0, 1700.0);

/// Lists with a scroll bar (the content is taller than the viewport).
const SCROLL_GEOMETRY: ListGeometry = ListGeometry {
    viewport: [26.0, 90.0, 1103.0, 1054.0],
    content_x: [41.0, 1088.0],
    scrollbar: [1103.0, 96.0, 1115.0, 1048.0],
    top_pad: 15.0,
    bottom_pad: 50.0,
};
/// Lists without one (the scroll bar hides itself and the viewport widens).
const WIDE_GEOMETRY: ListGeometry = ListGeometry {
    viewport: [26.0, 90.0, 1115.0, 1054.0],
    content_x: [41.0, 1100.0],
    scrollbar: [1103.0, 96.0, 1115.0, 1048.0],
    top_pad: 15.0,
    bottom_pad: 50.0,
};

mod anim;
pub use anim::Pointer;

/// A slider being dragged (`track` = x range in reference px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drag {
    /// Setting `SETTING_UI[.0]`.
    Setting(u16, u16, u16),
    /// Field `.1` of zone `.0`.
    Zone(u16, u8, u16, u16),
    /// Field `.1` of material `.0`.
    Material(u8, u8, u16, u16),
    /// Field `.1` of default bibite `.0`.
    Bibite(u16, u8, u16, u16),
}

/// A modal dialog over the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorPopup {
    /// "You're about to exit the Scenario Editor ..." (Back).
    ExitWarning,
    /// `ActionWarnings.startScenarioAfterEdit` (Start).
    StartWarning,
    /// The save form (`ScenarioSaver`): `focus` is the text field being typed into.
    Save {
        /// 0 = name, 1 = description.
        focus: u8,
    },
    /// "A scenario with this name already exists" (overwrite confirmation).
    Overwrite,
    /// "Delete All ..." confirmation of list `.0` (0 zones, 1 zone groups, 2 default bibites,
    /// 3 animators).
    ClearList(u8),
    /// "Delete <name>?" confirmation of item `.1` of list `.0` (0 zones, 1 zone groups).
    DeleteItem(u8, u16),
}

/// The draft being edited.
#[derive(Clone, Debug)]
pub struct EditorState {
    /// Scenario file the draft started from (templates and unknown entries come from it).
    pub base_path: PathBuf,
    /// Draft effective settings.
    pub settings: SimSettings,
    /// Draft scenario data (zones, zone groups, bibite spawns, animators).
    pub scenario: ScenarioData,
    /// Draft challenge definition (`None` = ordinary scenario).
    pub challenge: Option<ChallengeParameters>,
    /// `ScenarioSelectorPanel.scenarioTitle` (the save form's default name).
    pub title: String,
    /// `ScenarioSelectorPanel.scenarioDescription`.
    pub description: String,
    /// Template files added to the draft's default bibites (`file name`, bytes): they travel
    /// with the saved scenario like the game's `SaveSystem.AddTemplatesToArchive`.
    pub added_templates: Vec<(String, Vec<u8>)>,
    /// Selected tab.
    pub tab: u8,
    /// Scroll offset of every tab (reference px).
    pub scrolls: [f32; TAB_COUNT],
    /// Row state of the current tab (scroll, open sub-panels).
    pub dyn_state: DynState,
    /// Open modal dialog.
    pub popup: Option<EditorPopup>,
    /// Slider being dragged.
    pub drag: Option<Drag>,
    /// Zone items whose edit section is open: `(list, index)`.
    pub open_items: Vec<(u8, u16)>,
    /// Name typed into the save form.
    pub save_name: String,
    /// Description typed into the save form.
    pub save_description: String,
    /// The settings animator editor's state (the creator panel, the picked and dragged points, the graphs' frames).
    pub animator: crate::animator_session::Session,
    /// The card whose name is being typed (`ze::zone_ref` reference) and the text so far.
    pub name_edit: Option<(u16, String)>,
    /// The caret and the selection of the name being typed.
    pub name_caret: crate::text_field::Caret,
    /// The slider row whose value is being typed.
    pub value_edit: Option<crate::value_edit::ValueEdit>,
    /// The pointer of the click being applied (set by the menu before it applies the actions).
    pub pointer: Pointer,
    /// Where the characters of the open text field sit, measured by the last build: the text's inset in its control and the distance from the
    /// text's start to before each character (the end last), so a press can put the caret under the pointer.
    pub field_edges: std::cell::RefCell<(f32, Vec<f32>)>,
    /// A press in a text field is held: the text's start in reference px (a drag moves the selection's end).
    pub caret_drag: Option<f32>,
}

impl EditorState {
    /// A fresh draft of the scenario at `base_path`.
    #[must_use]
    pub fn new(
        base_path: PathBuf,
        settings: SimSettings,
        scenario: ScenarioData,
        challenge: Option<ChallengeParameters>,
        title: String,
        description: String,
    ) -> Self {
        Self {
            base_path,
            settings,
            scenario,
            challenge,
            title,
            description,
            added_templates: Vec::new(),
            tab: TAB_SIM,
            scrolls: [0.0; TAB_COUNT],
            dyn_state: DynState::default(),
            popup: None,
            drag: None,
            open_items: Vec::new(),
            save_name: String::new(),
            save_description: String::new(),
            animator: crate::animator_session::Session::default(),
            name_edit: None,
            name_caret: crate::text_field::Caret::default(),
            value_edit: None,
            pointer: Pointer::default(),
            field_edges: std::cell::RefCell::new((0.0, Vec::new())),
            caret_drag: None,
        }
    }

    /// Switches tab (`ScenariosEditor.SwitchToPanel`); every tab keeps its own scroll position.
    pub fn select_tab(&mut self, tab: u8) {
        let tab = tab.min(TAB_COUNT as u8 - 1);
        // `SettingsChangersEditor.FillPanel` closes the creator each time the panel is shown.
        if tab != self.tab {
            self.animator.creator.open = false;
        }
        self.scrolls[usize::from(self.tab)] = self.dyn_state.scroll;
        self.tab = tab;
        self.dyn_state.tab = tab;
        self.dyn_state.scroll = self.scrolls[usize::from(tab)];
    }

    /// Scrolls the current tab (wheel).
    pub fn scroll_by(&mut self, delta: f32) {
        let max = self.dyn_state.max_scroll.get();
        self.dyn_state.scroll = (self.dyn_state.scroll + delta).clamp(0.0, max.max(0.0));
    }

    /// Estimation shown under the preview: total biomass, pellet production, pellet count.
    #[must_use]
    pub fn estimation(&self) -> (f32, f32, u32) {
        estimation(&self.settings, &self.scenario)
    }

    /// Number of items in list `list` (0 zones, 1 zone groups, 2 default bibites, 3 animators).
    #[must_use]
    pub fn list_len(&self, list: u8) -> usize {
        match list {
            0 => self.scenario.zones.len(),
            1 => self.scenario.zone_groups.len(),
            2 => self.scenario.bibite_spawns.len(),
            _ => self.scenario.settings_changers.len(),
        }
    }

    /// The `+` button of a list. Zones and zone groups are added with the game's defaults
    /// (`ZonesEditorPanel.AddZone` / `AddZoneGroup`); the other lists are not editable yet.
    pub fn add_item(&mut self, list: u8) -> bool {
        use bibites_core::settings::{MaterialKind, ZoneGroupSettings, ZoneSettings};
        match list {
            0 => {
                let mut zone = ZoneSettings::game_defaults();
                zone.material = Some(MaterialKind::Plant);
                zone.name = format!("Zone {}", self.scenario.zones.len());
                self.scenario.zones.push(zone);
                self.renumber_zones(None);
                true
            }
            // `ZonesEditorPanel.AddZoneGroup`: the new group's card opens.
            1 => {
                let mut group = ZoneGroupSettings::game_defaults();
                group.name = format!("Pockets{}", self.scenario.zone_groups.len());
                group.align_count_types(self.settings.independents.simulation_size);
                self.scenario.zone_groups.push(group);
                self.open_items
                    .push((1, (self.scenario.zone_groups.len() - 1) as u16));
                true
            }
            // `SettingsChangersEditor.AddNewChanger`: the creator panel opens.
            3 => {
                self.animator.creator.open(None);
                true
            }
            _ => false,
        }
    }

    /// `DefaultBibitesPanel.AddBibite`: appends a default bibite for the template file
    /// `file_name` (`bytes` travel with the scenario) and opens it for editing.
    pub fn add_default_bibite(&mut self, file_name: &str, bytes: Vec<u8>) {
        let row = crate::bibite_editor::new_row(file_name);
        self.scenario.bibite_spawns.push(row);
        if !self
            .added_templates
            .iter()
            .any(|(name, _)| name == file_name)
        {
            self.added_templates.push((file_name.to_owned(), bytes));
        }
        // `ItemClicked`: the new item is the selected one, the others close.
        self.open_items.retain(|item| item.0 != 2);
        self.open_items
            .push((2, (self.scenario.bibite_spawns.len() - 1) as u16));
    }

    /// Name of item `index` of list `list` (zones, zone groups).
    #[must_use]
    pub fn item_name(&self, list: u8, index: usize) -> String {
        match list {
            0 => self.scenario.zones.get(index).map(|zone| zone.name.clone()),
            3 => self.scenario.settings_changers.get(index).map(|changer| changer.name.clone()),
            _ => self
                .scenario
                .zone_groups
                .get(index)
                .map(|group| group.name.clone()),
        }
        .unwrap_or_default()
    }

    /// Removes item `index` of list `list` (after the confirmation).
    pub fn delete_item(&mut self, list: u8, index: usize) {
        match list {
            0 if index < self.scenario.zones.len() => {
                let id = self.scenario.zones[index].zone_id;
                self.scenario.zones.remove(index);
                // Followers of the deleted zone lose their target; open items shift down.
                for zone in &mut self.scenario.zones {
                    if zone.target.is_some() && zone.target == id {
                        zone.target = None;
                    }
                }
                self.renumber_zones(Some(index));
                self.dyn_state.card_removed(0, index);
                self.name_edit = None;
                self.open_items.retain(|item| *item != (0, index as u16));
                for item in &mut self.open_items {
                    if item.0 == 0 && usize::from(item.1) > index {
                        item.1 -= 1;
                    }
                }
            }
            1 if index < self.scenario.zone_groups.len() => {
                self.scenario.zone_groups.remove(index);
                self.dyn_state.card_removed(1, index);
                self.name_edit = None;
                self.open_items.retain(|item| *item != (1, index as u16));
                for item in &mut self.open_items {
                    if item.0 == 1 && usize::from(item.1) > index {
                        item.1 -= 1;
                    }
                }
            }
            // `DefaultBibitesPanel.BibiteDeleted`: no confirmation.
            2 if index < self.scenario.bibite_spawns.len() => {
                self.scenario.bibite_spawns.remove(index);
                self.open_items.retain(|item| *item != (2, index as u16));
                for item in &mut self.open_items {
                    if item.0 == 2 && usize::from(item.1) > index {
                        item.1 -= 1;
                    }
                }
            }
            3 => self.delete_animator(index),
            _ => {}
        }
    }

    /// The list's "remove all" button, after the confirmation.
    pub fn clear_list(&mut self, list: u8) {
        self.open_items.retain(|item| item.0 != list);
        self.dyn_state.cards_cleared(list);
        self.name_edit = None;
        if list == 3 {
            self.animator.cleared();
        }
        match list {
            0 => self.scenario.zones.clear(),
            1 => self.scenario.zone_groups.clear(),
            2 => self.scenario.bibite_spawns.clear(),
            _ => self.scenario.settings_changers.clear(),
        }
    }

    /// Applies a click on a settings-row control of the current tab. Returns `true` when the
    /// click was one of those controls.
    pub fn apply(&mut self, action: UiAction, tps_defaults: [i32; 5]) -> bool {
        if let Some(handled) = self.apply_text_and_animators(action, self.pointer) {
            return handled;
        }
        match action {
            UiAction::DynTab(tab) => self.select_tab(tab),
            UiAction::DynToggle(index) => {
                dyn_panel::toggle_value(&mut self.settings, usize::from(index));
            }
            UiAction::DynReset(index) => {
                dyn_panel::reset_values(&mut self.settings, &[usize::from(index)], tps_defaults);
            }
            UiAction::DynSub(group) => self.dyn_state.toggle_sub(group),
            UiAction::DynResetGroup(group) => {
                let keys = dyn_panel::sub_group_keys_in(&editor_groups(self.tab), group);
                let indices: Vec<usize> = keys
                    .iter()
                    .filter_map(|key| dyn_panel::ui_of(key).map(|(index, _)| index))
                    .collect();
                dyn_panel::reset_values(&mut self.settings, &indices, tps_defaults);
            }
            UiAction::DynSlide(index, x0, x1) => {
                self.drag = Some(Drag::Setting(index, x0, x1));
            }
            UiAction::ZoneSlide(zone, field, x0, x1) => {
                self.drag = Some(Drag::Zone(zone, field, x0, x1));
            }
            UiAction::MaterialSlide(kind, field, x0, x1) => {
                self.drag = Some(Drag::Material(kind, field, x0, x1));
            }
            UiAction::MaterialToggle(kind) => {
                let material = crate::zone_editor::material_kind(kind);
                let slot = self.settings.materials.get_mut(material);
                slot.decay = !slot.decay;
            }
            UiAction::MaterialReset(kind, field) => {
                let material = crate::zone_editor::material_kind(kind);
                let slot = self.settings.materials.get_mut(material);
                crate::zone_editor::reset_material_field(slot, material, field);
            }
            UiAction::MaterialResetAll(kind) => {
                let material = crate::zone_editor::material_kind(kind);
                let defaults =
                    *bibites_core::settings::MaterialSettings::game_defaults().get(material);
                *self.settings.materials.get_mut(material) = defaults;
            }
            UiAction::ZoneOpen(list, index) => {
                if list == 2 {
                    // `DefaultBibitesPanel.ItemClicked`: only one default bibite is open, and a click on the open one does nothing.
                    self.open_items.retain(|item| item.0 != 2);
                    self.open_items.push((list, index));
                } else if let Some(at) = self.open_items.iter().position(|o| *o == (list, index)) {
                    self.open_items.remove(at);
                } else {
                    self.open_items.push((list, index));
                }
            }
            UiAction::BibiteSlide(index, field, x0, x1) => {
                self.drag = Some(Drag::Bibite(index, field, x0, x1));
            }
            UiAction::BibiteCycle(index, field) => {
                let zones = self.scenario.zones.clone();
                if let Some(row) = self.scenario.bibite_spawns.get_mut(usize::from(index)) {
                    crate::bibite_editor::cycle(row, field, &zones);
                }
            }
            // A pick in an open dropdown list (the list itself lives in the menu).
            UiAction::DropdownPick(index) => {
                let _ = index;
                return false;
            }
            UiAction::BibiteReset(index, field) => {
                if let Some(row) = self.scenario.bibite_spawns.get_mut(usize::from(index)) {
                    crate::bibite_editor::reset_value(row, field);
                }
            }
            UiAction::ZoneToggle(zone, field) => {
                let size = self.settings.independents.simulation_size;
                if let crate::zone_editor::ZoneRef::Group(group) = crate::zone_editor::zone_ref(zone) {
                    if field == crate::zone_editor::GROUP_SCALE {
                        self.edit_group(group, |group| group.scale_with_sim = !group.scale_with_sim);
                    }
                } else if let Some(item) = self.zone_like_mut(zone) {
                    crate::zone_editor::toggle(item, field);
                    crate::zone_editor::cap_position(item, size);
                }
            }
            UiAction::ZoneCycle(zone, field) => {
                let zones = self.scenario.zones.clone();
                let size = self.settings.independents.simulation_size;
                if let Some(item) = self.zone_like_mut(zone) {
                    crate::zone_editor::cycle(item, field, &zones, usize::from(zone));
                    crate::zone_editor::cap_position(item, size);
                }
            }
            UiAction::ZoneReset(zone, field) => {
                let size = self.settings.independents.simulation_size;
                if let Some(item) = self.zone_like_mut(zone) {
                    crate::zone_editor::reset(item, field, size);
                }
            }
            _ => return false,
        }
        true
    }

    /// The settings a zone index of an action addresses in the draft: a zone of the list, or the template of a zone group.
    fn zone_like_mut(&mut self, index: u16) -> Option<&mut bibites_core::settings::ZoneSettings> {
        match crate::zone_editor::zone_ref(index) {
            crate::zone_editor::ZoneRef::Zone(zone) => self.scenario.zones.get_mut(zone),
            crate::zone_editor::ZoneRef::Template(group) => {
                self.scenario.zone_groups.get_mut(group).map(|group| &mut group.template)
            }
            crate::zone_editor::ZoneRef::Group(_)
            | crate::zone_editor::ZoneRef::Animator(_)
            | crate::zone_editor::ZoneRef::Creator => None,
        }
    }

    /// A zone group's own setting changed: its two ways of counting agree again (`ZoneGroupSettings.AlignCountTypes`).
    fn edit_group(&mut self, group: usize, edit: impl FnOnce(&mut bibites_core::settings::ZoneGroupSettings)) {
        let size = self.settings.independents.simulation_size;
        if let Some(group) = self.scenario.zone_groups.get_mut(group) {
            edit(group);
            group.align_count_types(size);
        }
    }

    /// The options, the current choice and the look of the list of the editor's dropdown `target`
    /// (a zone item's or a default bibite's field).
    #[must_use]
    pub fn dropdown_choices(
        &self,
        target: crate::ui::DropdownTarget,
    ) -> Option<(Vec<crate::dropdown::Choice>, usize, crate::dropdown::Style)> {
        use crate::ui::DropdownTarget;
        match target {
            DropdownTarget::Zone(index, field) => {
                let zones = &self.scenario.zones;
                let zone = match crate::zone_editor::zone_ref(index) {
                    crate::zone_editor::ZoneRef::Zone(zone) => zones.get(zone)?,
                    crate::zone_editor::ZoneRef::Template(group) => {
                        &self.scenario.zone_groups.get(group)?.template
                    }
                    crate::zone_editor::ZoneRef::Group(_)
                    | crate::zone_editor::ZoneRef::Animator(_)
                    | crate::zone_editor::ZoneRef::Creator => return None,
                };
                Some((
                    crate::zone_editor::choices(field, zones),
                    crate::zone_editor::current_choice(zone, field, zones),
                    crate::dropdown::ROBOTO,
                ))
            }
            DropdownTarget::AnimatorPreset | DropdownTarget::AnimatorBase(_) | DropdownTarget::AnimatorInterpolation(_) => {
                let (choices, current) = self.animator_dropdown(target)?;
                Some((choices, current, crate::dropdown::ROBOTO))
            }
            DropdownTarget::Bibite(index, field) => {
                let zones = &self.scenario.zones;
                let row = self.scenario.bibite_spawns.get(usize::from(index))?;
                Some((
                    crate::bibite_editor::choices(field, zones),
                    crate::bibite_editor::current_choice(row, field, zones),
                    crate::dropdown::ROBOTO,
                ))
            }
            _ => None,
        }
    }

    /// A pick in the open list of the editor's dropdown `target`.
    pub fn pick_dropdown(&mut self, target: crate::ui::DropdownTarget, choice: usize) -> bool {
        use crate::ui::DropdownTarget;
        match target {
            DropdownTarget::Zone(index, field) => {
                let zones = self.scenario.zones.clone();
                let size = self.settings.independents.simulation_size;
                let Some(item) = self.zone_like_mut(index) else {
                    return false;
                };
                crate::zone_editor::set_choice(item, field, choice, &zones);
                crate::zone_editor::cap_position(item, size);
                true
            }
            DropdownTarget::Bibite(index, field) => {
                let zones = self.scenario.zones.clone();
                let Some(row) = self.scenario.bibite_spawns.get_mut(usize::from(index)) else {
                    return false;
                };
                crate::bibite_editor::set_choice(row, field, choice, &zones);
                true
            }
            DropdownTarget::AnimatorPreset | DropdownTarget::AnimatorBase(_) | DropdownTarget::AnimatorInterpolation(_) => {
                self.animator_pick(target, choice)
            }
            _ => false,
        }
    }

    /// Moves the slider being dragged to reference-px position `x`.
    pub fn drag_to(&mut self, x: f32, tps_defaults: [i32; 5]) -> bool {
        match self.drag {
            Some(Drag::Setting(index, x0, x1)) => dyn_panel::slide_value(
                &mut self.settings,
                usize::from(index),
                (f32::from(x0), f32::from(x1)),
                x,
                tps_defaults,
            ),
            Some(Drag::Zone(zone, field, x0, x1)) => {
                let size = self.settings.independents.simulation_size;
                let span = f32::from(x1) - f32::from(x0);
                if self.drag.is_some_and(Drag::is_animator) {
                    self.drag_animator_slider(zone, field, (f32::from(x0), f32::from(x1)), x);
                    return true;
                }
                if let crate::zone_editor::ZoneRef::Group(group) = crate::zone_editor::zone_ref(zone) {
                    let Some(info) = crate::zone_editor::group_slider_info(field) else {
                        return false;
                    };
                    if span <= 0.0 {
                        return false;
                    }
                    let value = dyn_panel::value_range(info.mode, info.min, info.max, info.default, (x - f32::from(x0)) / span);
                    self.edit_group(group, |group| crate::zone_editor::set_group_value(group, field, value));
                    return true;
                }
                let Some(item) = self.zone_like_mut(zone) else {
                    return false;
                };
                let Some(info) = crate::zone_editor::slider_info(item, field, size) else {
                    return false;
                };
                if span <= 0.0 {
                    return false;
                }
                let value = dyn_panel::value_range(
                    info.mode,
                    info.min,
                    info.max,
                    info.default,
                    (x - f32::from(x0)) / span,
                );
                crate::zone_editor::set_value(item, field, value, size);
                true
            }
            Some(Drag::Bibite(index, field, x0, x1)) => {
                let Some(info) = crate::bibite_editor::slider_info(field) else {
                    return false;
                };
                let span = f32::from(x1) - f32::from(x0);
                let Some(row) = self.scenario.bibite_spawns.get_mut(usize::from(index)) else {
                    return false;
                };
                if span <= 0.0 {
                    return false;
                }
                let value = dyn_panel::value_range(
                    info.mode,
                    info.min,
                    info.max,
                    info.default,
                    (x - f32::from(x0)) / span,
                );
                crate::bibite_editor::set_value(row, field, value);
                true
            }
            Some(Drag::Material(kind, field, x0, x1)) => {
                let material = crate::zone_editor::material_kind(kind);
                let Some(info) = crate::zone_editor::material_slider_info(material, field) else {
                    return false;
                };
                let span = f32::from(x1) - f32::from(x0);
                if span <= 0.0 {
                    return false;
                }
                let value = dyn_panel::value_range(
                    info.mode,
                    info.min,
                    info.max,
                    info.default,
                    (x - f32::from(x0)) / span,
                );
                let slot = self.settings.materials.get_mut(material);
                crate::zone_editor::set_material_value(slot, material, field, value);
                true
            }
            None => false,
        }
    }

    /// Gives the explicit zones their list-order ids (`ScenarioSettings.UpdateAllZonesList`) and
    /// keeps the targets pointing at the same zones; `removed` is the index of a zone that was
    /// just deleted (its followers lose their target).
    fn renumber_zones(&mut self, removed: Option<usize>) {
        let old: Vec<Option<u32>> = self.scenario.zones.iter().map(|z| z.zone_id).collect();
        let _ = removed;
        for (index, zone) in self.scenario.zones.iter_mut().enumerate() {
            zone.zone_id = Some(index as u32);
        }
        let new_ids: Vec<Option<u32>> = self.scenario.zones.iter().map(|z| z.zone_id).collect();
        for zone in &mut self.scenario.zones {
            zone.target = zone.target.and_then(|target| {
                old.iter()
                    .position(|id| *id == Some(target))
                    .and_then(|at| new_ids.get(at).copied().flatten())
            });
        }
    }
}

/// `ScenarioSettings.TotalBiomass / TotalGrowth / PelletNumberEstimation`: the sums over every
/// zone, including the zones the groups generate.
#[must_use]
pub fn estimation(settings: &SimSettings, scenario: &ScenarioData) -> (f32, f32, u32) {
    let globals = ZoneGlobals::from_settings(settings);
    let size = settings.independents.simulation_size;
    let mut biomass = 0.0;
    let mut growth = 0.0;
    let mut pellets = 0_u32;
    let mut add = |zone: &ZoneConfig, times: u32| {
        biomass += zone.max_biomass(&globals, size) * times as f32;
        growth += zone.total_growth(&globals, size) * times as f32;
        pellets =
            pellets.saturating_add(zone.estimated_pellets(&globals, size).saturating_mul(times));
    };
    for zone in &scenario.zones {
        add(&ZoneConfig::from_settings(zone), 1);
    }
    for group in &scenario.zone_groups {
        let config = ZoneGroupConfig::from_settings(group);
        add(
            &ZoneConfig::from_settings(&group.template),
            config.count(size),
        );
    }
    (biomass, growth, pellets)
}

/// Kind of a zone shape in the preview.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PreviewKind {
    /// A filled disc.
    Disc,
    /// An annulus whose hole is `inside` of the outer radius.
    Ring {
        /// Inside radius as a fraction of the outer one.
        inside: f32,
    },
    /// A rectangle.
    Rect,
}

/// One zone of the preview square (`ZonePreview`).
#[derive(Clone, Copy, Debug)]
pub struct PreviewShape {
    /// Centre from the middle of the square, as fractions of half its width: `x` right, `y` down.
    pub centre: [f32; 2],
    /// Width and height as fractions of the square's width.
    pub size: [f32; 2],
    /// Shape.
    pub kind: PreviewKind,
    /// Fill opacity (`Zone.UpdateMaterial`).
    pub opacity: f32,
    /// Palette factor: 0.5 for the flat distributions, 1 otherwise.
    pub palette: f32,
}

/// `f(v) = 1 - 0.9 * (invLog(v, .001, 100) - 1)^2` (`docs/design/ZONE_COLOUR.md`).
pub(crate) fn density_factor(value: f32) -> f32 {
    let value = value.clamp(0.001, 100.0).log10();
    let (min, max) = (0.001_f32.log10(), 100.0_f32.log10());
    let inverse = ((value - min) / (max - min)).clamp(0.0, 1.0);
    1.0 - 0.9 * (inverse - 1.0).powi(2)
}

/// One zone as the preview draws it (`ZonePreview` with the zone shader's `UpdateMaterial`): where it is, how
/// big, what shape, and the opacity and palette factor its fertility, biomass density and distribution give.
#[must_use]
pub fn zone_shape(config: &ZoneConfig, size: f32) -> PreviewShape {
    use bibites_core::settings::SpawnDistribution;
    let opacity = (density_factor(config.fertility) + density_factor(config.biomass_density)) * 0.5;
    let palette = match config.distribution {
        SpawnDistribution::Flat | SpawnDistribution::FlatRing | SpawnDistribution::Rect => 0.5,
        _ => 1.0,
    };
    let (kind, extent) = if config.is_rect() {
        (
            PreviewKind::Rect,
            [
                config.relative_width(size) * 0.5,
                config.relative_height(size) * 0.5,
            ],
        )
    } else {
        let diameter = config.relative_radius(size);
        let kind = if config.is_ring() {
            PreviewKind::Ring {
                inside: config.inside_radius,
            }
        } else {
            PreviewKind::Disc
        };
        (kind, [diameter, diameter])
    };
    PreviewShape {
        centre: [config.pos_x, -config.pos_y],
        size: extent,
        kind,
        opacity,
        palette,
    }
}

/// The zones of the draft as the preview draws them: the explicit zones, then each group's
/// generated zones at (deterministic) random positions.
#[must_use]
pub fn preview_shapes(settings: &SimSettings, scenario: &ScenarioData) -> Vec<PreviewShape> {
    use bibites_core::rng::{Rng, domain};
    let size = settings.independents.simulation_size;
    let shape = |config: &ZoneConfig| zone_shape(config, size);
    let mut shapes: Vec<PreviewShape> = scenario
        .zones
        .iter()
        .map(|zone| shape(&ZoneConfig::from_settings(zone)))
        .collect();
    for (index, group) in scenario.zone_groups.iter().enumerate() {
        let mut rng = Rng::derive(12_345, domain::WORLD, index as u64 + 1);
        let count = ZoneGroupConfig::from_settings(group).count(size);
        for _ in 0..count {
            let mut config = ZoneConfig::from_settings(&group.template);
            config.randomize_position(&mut rng, size);
            shapes.push(shape(&config));
        }
    }
    shapes
}

/// Formats of the three estimation lines (`BiomassPreview`'s `FloatValueTextHandle`s).
pub(crate) fn estimation_formats() -> [FloatValueFormat; 3] {
    [
        FloatValueFormat {
            prefix: "",
            units: "E",
            precision: 3,
            ..FloatValueFormat::default()
        },
        FloatValueFormat {
            prefix: "",
            units: "E/s",
            precision: 2,
            ..FloatValueFormat::default()
        },
        FloatValueFormat {
            prefix: "~",
            units: "pellets",
            precision: 1,
            ..FloatValueFormat::default()
        },
    ]
}

/// The setting groups of a tab, as `ScenariosEditor` declares them.
#[must_use]
pub fn editor_groups(tab: u8) -> Vec<TabGroup> {
    use Row::{FixedSum, Slider, Toggle};
    let linear = dyn_panel::LINEAR;
    let log = dyn_panel::log;
    let sliders = |keys: &[&'static str]| -> Vec<Row> {
        keys.iter().map(|key| Slider(key, linear)).collect()
    };
    match tab {
        TAB_SIM => {
            let toggle = |key: &'static str| Toggle(key);
            let mut physics = vec![
                Slider("dragCoefficient", linear),
                Toggle("disableDragOnHeldPellet"),
            ];
            physics.extend(sliders(&[
                "armMusclePressure",
                "forwardForceSizePower",
                "turnForceSizePower",
                "backwardFraction",
                "bibiteMassDensity",
                "stomachContentContribution",
                "bitingPressure",
                "jawMusclesSizingPower",
                "bitePeriodFactor",
                "throwingForceFactor",
                "bitingThrowForceFactor",
                "bitingDamageFactor",
                "collisionDamageConstant",
                "collisionDamageThreshold",
            ]));
            vec![
                TabGroup {
                    title: "Simulation Options",
                    rows: vec![
                        toggle("corpsesEnabled"),
                        toggle("preventMutations"),
                        Row::Sub {
                            title: "Pellet Options",
                            master: "pelletCollision",
                            rows: vec![toggle("pelletRotation"), toggle("pelletMerge")],
                        },
                        toggle("eggCollision"),
                        toggle("disableHerding"),
                        Row::Sub {
                            title: "Shade Effect",
                            master: "shadeOutsideOfBounds",
                            rows: vec![toggle("shadeAvoidance"), toggle("worldWrapping")],
                        },
                        Row::Sub {
                            title: "Red Death Bloom Settings",
                            master: "enableRedDeath",
                            rows: vec![
                                Slider("redDeathFill", linear),
                                Slider("redDeathBaseDamages", log(10.0, Some(3.0))),
                                Slider("redDeathDamageVelocity", log(10.0, Some(3.0))),
                            ],
                        },
                        toggle("preventRedPheroProduction"),
                        toggle("preventGreenPheroProduction"),
                        toggle("preventBluePheroProduction"),
                    ],
                },
                TabGroup {
                    title: "Cheat Options",
                    rows: vec![Row::Sub {
                        title: "Void Avoidance Settings",
                        master: "voidAvoidance",
                        rows: vec![Slider("voidAvoidanceDistance", linear)],
                    }],
                },
                TabGroup {
                    title: "Simulation Performance Parameters",
                    rows: vec![
                        Slider("simTPS", dyn_panel::INT),
                        Slider("brainTPS", dyn_panel::INT),
                        Slider("decayTPS", dyn_panel::INT),
                        Slider("visionLookupUpdateFactor", dyn_panel::INT),
                        Slider("visionSenseUpdateFactor", dyn_panel::INT),
                        Slider("minPelletSize", log(10.0, Some(3.5))),
                    ],
                },
                TabGroup {
                    title: "Physics Parameters",
                    rows: physics,
                },
            ]
        }
        TAB_ZONES => vec![
            TabGroup {
                title: "World Parameters",
                rows: vec![
                    Slider("SimulationSize", log(10.0, None)),
                    Slider("biomassDensity", log(10.0, Some(3.5))),
                    Slider("initialSeeding", linear),
                    Slider("pelletGrowth", log(10.0, Some(3.5))),
                    Slider("globalZoneSpeed", log(10.0, Some(6.0))),
                    Slider("pelletEnergy", log(10.0, None)),
                ],
            },
            // The captured `World Parameters` wrapper is 60 px taller than its group.
            bare(vec![Row::Gap(40.0)]),
            bare(vec![list_header("Zones Header", "Zones", 0)]),
            bare(vec![Row::Items {
                list: 0,
                template: "NoZones",
                height: 100.0,
            }]),
            bare(vec![list_header("Group Header", "Zone Groups", 1)]),
            bare(vec![Row::Items {
                list: 1,
                template: "NoGroups",
                height: 99.0,
            }]),
        ],
        TAB_ENERGY => {
            let mut balance = sliders(&[
                "storableEnergyPerArea",
                "fatSustain",
                "baseMetabolismCost",
                "energyUsageEfficiency",
                "moveMusclesCost",
                "neuronBirthCost",
                "synapseBirthCost",
                "neuronUpkeepCost",
                "synapseUpkeepCost",
                "pheromoneProductionCost",
                "pheromoneProductionStrength",
                "healRate",
                "healPowerFactor",
                "healEfficiency",
            ]);
            balance.push(Slider("corpseDecayFactor", log(10.0, Some(4.0))));
            vec![
                TabGroup {
                    title: "Bibite Aging",
                    rows: sliders(&[
                        "ageingThreshold",
                        "ageStrengthPenalties",
                        "ageMetabolismPenalties",
                    ]),
                },
                TabGroup {
                    title: "Bibite Balance",
                    rows: balance,
                },
                TabGroup {
                    title: "Materials Parameters",
                    rows: {
                        let mut rows: Vec<Row> = (0..4).map(Row::Material).collect();
                        rows.extend(sliders(&[
                            "plantAffinityPowerFactor",
                            "meatAffinityPowerFactor",
                        ]));
                        rows
                    },
                },
                TabGroup {
                    title: "Bibite Meat Balance",
                    rows: sliders(&[
                        "baseBodyGrowthCost",
                        "viewAngleAddedCost",
                        "viewRadiusAddedCost",
                        "healthPerArea",
                        "healthEnergyFactor",
                    ]),
                },
            ]
        }
        TAB_BIBITES => vec![
            bare(vec![
                list_header("Top Header", "Default Bibites", 2),
                // The default bibites; an empty list still takes its slot between the spacings.
                Row::Items {
                    list: 2,
                    template: "",
                    height: 0.0,
                },
                Row::Heading("Bibite Settings"),
            ]),
            TabGroup {
                title: "Virgin Bibite Options",
                rows: vec![
                    Slider("virginSpawnRate", linear),
                    Row::Sub {
                        title: "Limit Bibite Birth",
                        master: "limitBibiteBirth",
                        rows: vec![Slider("bibiteLimit", dyn_panel::INT)],
                    },
                ],
            },
            TabGroup {
                title: "Species Settings",
                rows: sliders(&["speciesGeneticSpan", "speciesSpanScalingFactor"]),
            },
            TabGroup {
                title: "Mutation Genes",
                rows: sliders(&[
                    "backgroundMutationChance",
                    "backgroundMutationVariance",
                    "relativeMutationShare",
                ]),
            },
            TabGroup {
                title: "",
                rows: vec![FixedSum {
                    title: "Brain Mutation Probabilities",
                    sum: 1.0,
                    rows: sliders(&["synapseMutationChance", "neuronMutationChance"]),
                }],
            },
            TabGroup {
                title: "",
                rows: vec![FixedSum {
                    title: "Synapse Mutation Probabilities",
                    sum: 1.0,
                    rows: sliders(&[
                        "synapseChangeChance",
                        "synapseFlipChance",
                        "synapseToggleChance",
                        "synapseAddChance",
                        "synapseRemoveChance",
                    ]),
                }],
            },
            TabGroup {
                title: "",
                rows: vec![FixedSum {
                    title: "Neuron Mutation Probabilities",
                    sum: 1.0,
                    rows: sliders(&[
                        "neuronDefaultChance",
                        "neuronChangeChance",
                        "neuronAddChance",
                        "neuronRemoveChance",
                    ]),
                }],
            },
        ],
        TAB_ANIMS => vec![
            bare(vec![Row::Gap(0.0)]),
            bare(vec![list_header("Animator Header", "Settings Animator", 3)]),
            bare(vec![Row::Items {
                list: 3,
                template: "NoAnimator",
                height: 100.0,
            }]),
        ],
        _ => Vec::new(),
    }
}

/// The challenge tab: an ordinary scenario offers to become a challenge; a challenge offers to
/// turn back (its condition editors are not available yet).
fn challenge_groups(is_challenge: bool) -> Vec<TabGroup> {
    let (template, button, action) = if is_challenge {
        (
            "TitleHolder",
            "TurnIntoScenarioButton",
            UiAction::Menu(MenuCommand::EditorChallenge(false)),
        )
    } else {
        (
            "NotAChallengeSection",
            "TurnIntoChallengeButton",
            UiAction::Menu(MenuCommand::EditorChallenge(true)),
        )
    };
    vec![bare(vec![
        // The holder pads its content by 20 px.
        Row::Gap(5.0),
        Row::Panel {
            template,
            buttons: vec![(button, action)],
        },
    ])]
}

/// A group without a frame of its own (each row brings its own title).
fn bare(rows: Vec<Row>) -> TabGroup {
    TabGroup { title: "", rows }
}

/// A list header whose `+` and "remove all" buttons edit list `list`
/// (0 zones, 1 zone groups, 2 default bibites, 3 animators).
fn list_header(template: &'static str, title: &'static str, list: u8) -> Row {
    Row::Header {
        template,
        title,
        add: UiAction::Menu(MenuCommand::EditorAdd(list)),
        clear: UiAction::Menu(MenuCommand::EditorClear(list)),
    }
}

/// What [`build`] needs besides the captured template.
pub struct EditorInput<'a> {
    /// The draft.
    pub state: &'a EditorState,
    /// Defaults of the five TPS settings.
    pub tps_defaults: [i32; 5],
    /// Width of a text at a font size (reference px), for the right-anchored value texts.
    pub measure: &'a dyn Fn(&str, f32) -> f32,
    /// The template library as `(file name, display name)`: the names the default bibites show.
    pub library: &'a [(String, String)],
}

/// Result of [`build`].
pub struct EditorBuild {
    /// The editor root with generated content.
    pub root: Node,
    /// Click regions.
    pub hits: Vec<DynHit>,
    /// Tooltips of the generated controls (the animator creator's rows).
    pub tips: Vec<DynTip>,
}

/// A hash of everything the editor shows, so an unchanged editor can reuse its last frame.
#[must_use]
pub fn state_key(state: &EditorState, tps_defaults: [i32; 5]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    state.tab.hash(&mut hasher);
    state.dyn_state.scroll.to_bits().hash(&mut hasher);
    state.dyn_state.open_subs.hash(&mut hasher);
    tps_defaults.hash(&mut hasher);
    for row in crate::setting_ui_data::SETTING_UI {
        dyn_panel::read_value(&state.settings, row)
            .to_bits()
            .hash(&mut hasher);
    }
    let (biomass, growth, pellets) = state.estimation();
    biomass.to_bits().hash(&mut hasher);
    growth.to_bits().hash(&mut hasher);
    pellets.hash(&mut hasher);
    // Lists and the challenge flag change what the tabs and the preview show.
    format!("{:?}{:?}", state.scenario.zones, state.scenario.zone_groups).hash(&mut hasher);
    format!("{:?}", state.scenario.bibite_spawns).hash(&mut hasher);
    state.open_items.hash(&mut hasher);
    state.dyn_state.order.hash(&mut hasher);
    format!("{:?}{:?}{:?}{:?}", state.scenario.settings_changers, state.animator, state.name_edit, state.name_caret).hash(&mut hasher);
    // (The blink phase, not the idle time: the picture only changes when the caret appears or goes.)
    format!("{:?}{}", state.value_edit.as_ref().map(|edit| (edit.field, &edit.text, edit.caret)), state.value_edit.as_ref().is_some_and(|edit| edit.caret_visible())).hash(&mut hasher);
    format!("{:?}", state.settings.materials).hash(&mut hasher);
    state.challenge.is_some().hash(&mut hasher);
    hasher.finish()
}

/// Builds the editor from the captured `ScenarioEditor` node.
#[must_use]
pub fn build(captured: &Node, input: &EditorInput<'_>) -> Option<EditorBuild> {
    let state = input.state;
    let tab = state.tab.min(TAB_COUNT as u8 - 1);
    let templates = dyn_panel::templates_of(captured)?;
    let mut root = dyn_panel::clone_chrome(captured);
    state.measure_field(input.measure);

    // Only the selected tab's panel is shown (`SwitchToPanel`).
    let wanted = TAB_PANELS[usize::from(tab)];
    if let Some(panels) = dyn_panel::find_mut(&mut root, &|n: &Node| {
        n.name == "Panels" && n.path.ends_with("/Holder/Panels")
    }) {
        panels
            .children
            .retain(|child| child.name == "LeftSide Buttons" || child.name == wanted);
    }

    let mut hits: Vec<DynHit> = Vec::new();
    // The animator creator (`SettingsChangerCreator`) hangs over the Animation tab, outside its scroll view, and is on top of everything else.
    let mut creator = None;
    let groups = if tab == TAB_CHALLENGE {
        challenge_groups(state.challenge.is_some())
    } else {
        editor_groups(tab)
    };
    let bibite_names: Vec<String> = state
        .scenario
        .bibite_spawns
        .iter()
        .map(|row| crate::bibite_editor::display_name(&row.path, input.library))
        .collect();
    // The settings an animator could take over (the Animation list's base items name them).
    let catalog = if tab == TAB_ANIMS { state.animator_catalog() } else { Vec::new() };
    crate::text_field::set_display_caret(if state.name_edit.is_some() {
        state.name_caret
    } else {
        crate::text_field::Caret { pos: usize::MAX, anchor: None }
    });
    if !groups.is_empty() {
        let dyn_input = DynInput {
            settings: &state.settings,
            state: &state.dyn_state,
            tps_defaults: input.tps_defaults,
            edit: state.value_edit.as_ref(),
            lists: Some(dyn_panel::ListsInput {
                zones: &state.scenario.zones,
                all_zones: &state.scenario.zones,
                changers: &state.scenario.settings_changers,
                catalog: &catalog,
                graph_views: &state.animator.views,
                sim_time: None,
                selected_point: state.animator.point,
                creator: Some(&state.animator.creator),
                groups: &state.scenario.zone_groups,
                streams: &[],
                open: &state.open_items,
                order: &state.dyn_state.order,
                bibites: &state.scenario.bibite_spawns,
                bibite_names: &bibite_names,
                in_sim: false,
                delete_item: |list, index| UiAction::Menu(MenuCommand::EditorDelete(list, index)),
                sim_size: state.settings.independents.simulation_size,
                name_edit: state.name_edit.as_ref().map(|(reference, text)| (*reference, text.as_str())),
            }),
        };
        let viewport_h = SCROLL_GEOMETRY.viewport[3] - SCROLL_GEOMETRY.viewport[1];
        // The scroll bar only exists when the content does not fit.
        let mut geometry = SCROLL_GEOMETRY;
        let mut list = dyn_panel::layout_list(&templates, &groups, &dyn_input, &geometry);
        if list.content_h <= viewport_h && tab != TAB_SIM && tab != TAB_ENERGY {
            geometry = WIDE_GEOMETRY;
            // The rows of a tab without a scroll bar are 12 px wider (the zones tab's own).
            let wide = dyn_panel::find(captured, &|n: &Node| {
                n.name == "ZoneEditorPanel" && n.path.contains("/Panels/")
            })
            .map_or_else(
                || templates.clone(),
                |panel| templates.clone().with_rows_of(panel),
            );
            list = dyn_panel::layout_list(&wide, &groups, &dyn_input, &geometry);
        }
        // The tabs captured without a scroll bar borrow the simulation tab's when they need one.
        let scrollbar = (geometry.content_x[1] == SCROLL_GEOMETRY.content_x[1]
            && !matches!(tab, TAB_SIM | TAB_ENERGY | TAB_BIBITES))
        .then(|| {
            dyn_panel::find(captured, &|n: &Node| {
                n.name == "Scrollbar Vertical" && n.path.contains("/SimTab/")
            })
            .cloned()
        })
        .flatten();
        if let Some(panel) = dyn_panel::find_mut(&mut root, &|n: &Node| {
            n.name == wanted && n.path.contains("/Panels/")
        }) {
            if let Some(scrollbar) = scrollbar
                && let Some(view) = dyn_panel::find_mut(panel, &|n: &Node| n.name == "Scroll View")
                && !view.children.iter().any(|c| c.name == "Scrollbar Vertical")
            {
                view.children.push(scrollbar);
            }
            dyn_panel::place_scrollbar(panel, &geometry, &list);
            dyn_panel::replace_content(panel, list.nodes);
        }
        hits.extend(list.hits);
        if tab == TAB_ANIMS
            && let Some(lists) = dyn_input.lists.as_ref()
        {
            creator = dyn_panel::editor_creator_panel(lists, dyn_input.edit);
        }
    }

    // Header buttons.
    let rect = |(x0, x1): (f32, f32)| [x0, BUTTON_Y.0, x1 - x0, BUTTON_Y.1 - BUTTON_Y.0];
    for (index, span) in TAB_BUTTONS.iter().enumerate() {
        hits.push(DynHit {
            rect: rect(*span),
            action: UiAction::DynTab(index as u8),
        });
    }
    for (span, command) in [
        (START_BUTTON, MenuCommand::EditorStart),
        (BACK_BUTTON, MenuCommand::EditorBack),
        (SAVE_BUTTON, MenuCommand::EditorSave),
    ] {
        hits.push(DynHit {
            rect: rect(span),
            action: UiAction::Menu(command),
        });
    }

    // Estimation under the preview.
    let (biomass, growth, pellets) = state.estimation();
    let formats = estimation_formats();
    for (line, (value, format)) in [
        ("Line", (biomass, &formats[0])),
        ("Line (2)", (growth, &formats[1])),
        ("Line (1)", (pellets as f32, &formats[2])),
    ] {
        let text = format.format(value);
        let prefix = "/Estimation/";
        let in_line = |n: &Node, name: &str| {
            n.name == name
                && n.path
                    .rsplit_once(prefix)
                    .is_some_and(|(_, rest)| rest.starts_with(&format!("{line}/")))
        };
        // The value sits flush right and is as wide as its text; the label takes the rest.
        let mut value_left = None;
        if let Some(info) = dyn_panel::find_mut(&mut root, &|n: &Node| in_line(n, "FloatInfoText"))
        {
            let size = info.texts.first().map_or(32.0, |t| t.font_size);
            if let Some([_, y0, x1, y1]) = dyn_panel::rect_of(info) {
                let left = x1 - (input.measure)(&text, size).ceil();
                dyn_panel::set_rect(info, left, y0, x1, y1);
                value_left = Some(left);
            }
            dyn_panel::set_text(info, &text);
        }
        if let Some(left) = value_left
            && let Some(label) =
                dyn_panel::find_mut(&mut root, &|n: &Node| in_line(n, "Text (TMP)"))
            && let Some([x0, y0, _, y1]) = dyn_panel::rect_of(label)
        {
            dyn_panel::set_rect(label, x0, y0, left, y1);
        }
    }
    let mut tips = Vec::new();
    if let Some((nodes, popup_hits, popup_tips)) = creator {
        for mut node in nodes {
            node.sibling_index = root.children.len() as i32;
            root.children.push(node);
        }
        hits.extend(popup_hits);
        tips.extend(popup_tips);
    }
    Some(EditorBuild { root, hits, tips })
}

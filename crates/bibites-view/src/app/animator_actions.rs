//! The settings animator editor's actions on the world (`SettingsChangersEditor`, `SettingsChangerCreator`, `SettingsChangerHandle`,
//! `EditableLineGraph`): the creator panel, the animators' cards, their bases and their curves.

use super::App;
use crate::animator_session::{Env, Session};
use crate::animators::{self, Animatable};
use crate::dropdown::Choice;

impl App {
    /// The simulated time now (what an animator created now starts at unless its start slider was moved).
    fn animator_now(&self) -> f32 {
        self.world.simulated_time as f32
    }

    /// The catalogue of the settings an animator can take over, with the world's explicit zones.
    pub(crate) fn animator_catalog(&self) -> Vec<Animatable> {
        let zones = self.explicit_zone_settings();
        animators::catalog(&self.world.settings, &zones)
    }

    /// The settings of the explicit zones (the ones an animator can take over).
    fn explicit_zone_settings(&self) -> Vec<bibites_core::settings::ZoneSettings> {
        let explicit = self.world.zones.explicit_count();
        self.world.zones.zones[..explicit]
            .iter()
            .map(|zone| zone.config.0.clone())
            .collect()
    }

    /// `SettingsChangersEditor.AddNewChanger`: the creator panel opens with its defaults (the start at the simulated time).
    pub(super) fn open_animator_creator(&mut self) {
        let now = self.animator_now();
        self.animator.creator.open(Some(now));
    }

    /// The creator's Cancel button (`UIPanel.ClosePanel`).
    pub(super) fn close_animator_creator(&mut self) {
        self.animator.creator.open = false;
    }

    /// The creator's OK button (`SettingsChangerCreator.Submit`, `SettingsChangersEditor.OnChangerAdded`): the animator is added and its card
    /// opens.
    pub(super) fn submit_animator_creator(&mut self) {
        let now = self.animator_now();
        let changer = self.animator.creator.build(Some(now));
        self.animator.creator.open = false;
        match self.world.add_settings_changer(changer) {
            Ok(index) => {
                self.dyn_panel.open_items.push((3, index as u16));
                self.last_action = format!("added settings animator {index}");
            }
            Err(error) => self.last_action = format!("animator not added: {error}"),
        }
    }

    /// `SettingsChangerHandle.DeleteChanger` / `SettingsChangersEditor.ChangerDeleted`: the animator goes (its settings are free again).
    pub(super) fn delete_animator(&mut self, index: usize) {
        if self.world.remove_settings_changer(index).is_none() {
            return;
        }
        if index < self.animator.views.len() {
            self.animator.views.remove(index);
        }
        let at = index as u16;
        self.dyn_panel.open_items.retain(|item| *item != (3, at));
        for item in &mut self.dyn_panel.open_items {
            if item.0 == 3 && item.1 > at {
                item.1 -= 1;
            }
        }
        self.animator.point = None;
        self.dyn_panel.card_removed(3, index);
        self.zone_name_edit = None;
        self.last_action = format!("deleted settings animator {index}");
    }

    /// `SettingsChangersEditor.ActuallyDeleteAllChangers`.
    pub(super) fn clear_animators(&mut self) {
        self.world.clear_settings_changers();
        self.animator.views.clear();
        self.dyn_panel.open_items.retain(|item| item.0 != 3);
        self.dyn_panel.cards_cleared(3);
        self.animator.point = None;
        self.zone_name_edit = None;
        self.last_action = "removed all settings animators".to_owned();
    }

    /// The Repeat toggle of animator `index`.
    pub(super) fn animator_toggle_repeat(&mut self, index: usize) {
        Session::toggle_repeat(&mut self.world.settings_changers, index);
    }

    /// `SettingBaseItem.Remove`: the animator lets go of its `base`th setting.
    pub(super) fn remove_animator_base(&mut self, index: usize, base: usize) {
        Session::remove_base(&mut self.world.settings_changers, index, base);
    }

    /// The settings the "Add Setting" dropdown offers: every one no animator has yet, in the catalogue's order.
    pub(crate) fn animator_available(&self) -> Vec<Animatable> {
        let zones = self.explicit_zone_settings();
        Session::available(&self.world.settings_changers, &Env { settings: &self.world.settings, zones: &zones, scale: 1.0 })
    }

    /// The choices of the animator dropdowns (`target`) and the current one.
    pub(super) fn animator_dropdown_choices(&self, target: crate::ui::DropdownTarget) -> Option<(Vec<Choice>, usize)> {
        let zones = self.explicit_zone_settings();
        self.animator.dropdown_choices(&self.world.settings_changers, &Env { settings: &self.world.settings, zones: &zones, scale: 1.0 }, target)
    }

    /// A pick in an animator dropdown's list; `false` when `target` is not one of them.
    pub(crate) fn animator_dropdown_pick(&mut self, target: crate::ui::DropdownTarget, choice: usize) -> bool {
        let zones = self.explicit_zone_settings();
        let env = Env { settings: &self.world.settings, zones: &zones, scale: 1.0 };
        self.animator.dropdown_pick(&mut self.world.settings_changers, &env, target, choice)
    }

    /// The slider an animator control addresses and its value.
    pub(crate) fn animator_slider(&self, reference: u16, field: u8) -> Option<(crate::zone_editor::SliderInfo, f32)> {
        let zones = self.explicit_zone_settings();
        self.animator.slider(&self.world.settings_changers, &Env { settings: &self.world.settings, zones: &zones, scale: 1.0 }, reference, field)
    }

    /// An animator slider takes `value` (as it is for a typed value, kept inside the slider's range when dragged).
    pub(crate) fn animator_set_value(&mut self, reference: u16, field: u8, value: f32, clamp: bool) {
        let zones = self.explicit_zone_settings();
        let env = Env { settings: &self.world.settings, zones: &zones, scale: 1.0 };
        self.animator.set_value(&mut self.world.settings_changers, &env, reference, field, value, clamp);
    }

    /// An animator slider follows the cursor.
    pub(super) fn animator_slide(&mut self, reference: u16, field: u8, track: (f32, f32), cursor_x: f32) {
        let scale = crate::ui::UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let zones = self.explicit_zone_settings();
        let env = Env { settings: &self.world.settings, zones: &zones, scale };
        self.animator.slide(&mut self.world.settings_changers, &env, reference, field, track, cursor_x);
    }

    /// The frame animator `index`'s graph shows: the kept one, or the curve's own (`UpdateMinMax`).
    pub(crate) fn animator_graph_view(&mut self, index: usize) -> Option<animators::GraphView> {
        self.animator.graph_view(&self.world.settings_changers, index)
    }

    /// A press on the plot of animator `index`'s graph (`rect`: its edges in reference px, `cursor` in screen px): the point under it is picked
    /// and dragged; with Ctrl the point goes (`RemovePoint`), or on empty ground a new one is added (`AddPoint`).
    pub(crate) fn animator_graph_press(&mut self, index: usize, rect: [f32; 4], cursor: crate::app::Vec2, ctrl: bool) {
        let scale = crate::ui::UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let zones = self.explicit_zone_settings();
        let env = Env { settings: &self.world.settings, zones: &zones, scale };
        if self.animator.graph_press(&mut self.world.settings_changers, &env, index, rect, [cursor.x, cursor.y], ctrl) {
            self.value_edit = None;
        }
    }

    /// The dragged point follows the cursor (`OnDragHandleMoved`).
    pub(crate) fn animator_graph_drag(&mut self, cursor: crate::app::Vec2) {
        let scale = crate::ui::UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let zones = self.explicit_zone_settings();
        let env = Env { settings: &self.world.settings, zones: &zones, scale };
        self.animator.graph_drag(&mut self.world.settings_changers, &env, [cursor.x, cursor.y]);
    }

    /// The press is let go.
    pub(crate) fn animator_graph_release(&mut self) {
        self.animator.drag = None;
    }

    /// `RemovePoint`: the first point and the last two stay.
    pub(super) fn animator_remove_point(&mut self, index: usize, point: usize) {
        self.animator.remove_point(&mut self.world.settings_changers, index, point);
    }
}

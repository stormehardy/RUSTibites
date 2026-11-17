//! The settings animator editor's state and actions, shared by the world's Animation tab and the scenario editor's Animation list
//! (`SettingsChangersEditor`, `SettingsChangerHandle`, `EditableLineGraph`): the creator panel, the selected point, the graph drag, the
//! graphs' frames, and what a slider, a dropdown or a press on the graph does to the animators (a list of [`SettingsChanger`]).

use bibites_core::settings::{SettingBase, SettingsChanger, SimSettings, ZoneSettings};

use crate::animators::{self, Animatable};
use crate::dropdown::Choice;
use crate::zone_editor::{self as ze, ZoneRef};

/// What the animator editor knows besides the animators: the creator, the picked point, the dragged point, the graphs' frames.
#[derive(Clone, Debug, Default)]
pub struct Session {
    /// The creator panel.
    pub creator: animators::Creator,
    /// The picked point of a curve: `(animator, point)`.
    pub point: Option<(usize, usize)>,
    /// The point being dragged: `(animator, point, plot rectangle in reference px)`.
    pub drag: Option<(usize, usize, [f32; 4])>,
    /// The frame each animator's graph shows (`None`: framed on the curve).
    pub views: Vec<Option<animators::GraphView>>,
}

/// The things an action needs that are not the animators themselves.
#[derive(Clone, Copy)]
pub struct Env<'a> {
    /// The settings the animators can take over.
    pub settings: &'a SimSettings,
    /// The explicit zones (their settings can be animated too).
    pub zones: &'a [ZoneSettings],
    /// Screen pixels per reference pixel.
    pub scale: f32,
}

/// Edits animator `index`; the edited animator must still be valid, otherwise nothing changes (`false`).
pub fn edit(changers: &mut [SettingsChanger], index: usize, change: impl FnOnce(&mut SettingsChanger)) -> bool {
    let Some(current) = changers.get(index) else {
        return false;
    };
    let mut changer = current.clone();
    change(&mut changer);
    if changer.validate().is_err() {
        return false;
    }
    changers[index] = changer;
    true
}

/// Whether an animator already animates the setting `key` (`ScenarioSettings.animatedSettings`).
#[must_use]
pub fn is_animated(changers: &[SettingsChanger], key: &str) -> bool {
    changers.iter().any(|changer| changer.settings_bases.iter().any(|base| base.key == key))
}

impl Session {
    /// A new animator is gone (`index` removed): the frames and the pick follow.
    pub fn removed(&mut self, index: usize) {
        if index < self.views.len() {
            self.views.remove(index);
        }
        self.point = None;
        self.drag = None;
    }

    /// Every animator is gone.
    pub fn cleared(&mut self) {
        self.views.clear();
        self.point = None;
        self.drag = None;
    }

    /// The settings the "Add Setting" dropdown offers: every one no animator has yet, in the catalogue's order.
    #[must_use]
    pub fn available(changers: &[SettingsChanger], env: &Env<'_>) -> Vec<Animatable> {
        animators::catalog(env.settings, env.zones)
            .into_iter()
            .filter(|entry| !is_animated(changers, &entry.key))
            .collect()
    }

    /// The choices of the animator dropdown `target` and the current one.
    #[must_use]
    pub fn dropdown_choices(&self, changers: &[SettingsChanger], env: &Env<'_>, target: crate::ui::DropdownTarget) -> Option<(Vec<Choice>, usize)> {
        use crate::ui::DropdownTarget;
        match target {
            DropdownTarget::AnimatorPreset => Some((
                animators::Preset::ALL.iter().map(|preset| Choice::plain(preset.label())).collect(),
                animators::Preset::ALL.iter().position(|preset| *preset == self.creator.preset).unwrap_or(0),
            )),
            // "Select Settings", then the free settings (`SettingsChangerHandle.SetOptionList`).
            DropdownTarget::AnimatorBase(_) => {
                let mut choices = vec![Choice::plain("Select Settings")];
                choices.extend(Self::available(changers, env).iter().map(|entry| Choice::plain(entry.name.clone())));
                Some((choices, 0))
            }
            DropdownTarget::AnimatorInterpolation(changer) => {
                let (_, point) = self.point.filter(|(selected, _)| *selected == usize::from(changer))?;
                let current = changers.get(usize::from(changer))?.points.get(point)?.interpolation;
                Some((
                    animators::INTERPOLATIONS.iter().map(|(_, label)| Choice::plain(*label)).collect(),
                    animators::INTERPOLATIONS.iter().position(|(mode, _)| *mode == current).unwrap_or(0),
                ))
            }
            _ => None,
        }
    }

    /// A pick in an animator dropdown's list; `false` when `target` is not one of them.
    pub fn dropdown_pick(&mut self, changers: &mut [SettingsChanger], env: &Env<'_>, target: crate::ui::DropdownTarget, choice: usize) -> bool {
        use crate::ui::DropdownTarget;
        match target {
            DropdownTarget::AnimatorPreset => {
                if let Some(preset) = animators::Preset::ALL.get(choice) {
                    self.creator.preset = *preset;
                }
            }
            // `SettingsChangerHandle.OnNewBaseSelected`: choice 0 is the caption; the others add the setting, from the value it has now.
            DropdownTarget::AnimatorBase(changer) => {
                if choice >= 1
                    && let Some(entry) = Self::available(changers, env).get(choice - 1).cloned()
                    && let Some(value) = animators::current_value(env.settings, env.zones, &entry.key)
                    && let Some(target) = changers.get_mut(usize::from(changer))
                {
                    target.settings_bases.push(SettingBase { key: entry.key, base: value });
                }
            }
            DropdownTarget::AnimatorInterpolation(changer) => {
                if let (Some((_, point)), Some((mode, _))) = (
                    self.point.filter(|(selected, _)| *selected == usize::from(changer)),
                    animators::INTERPOLATIONS.get(choice),
                ) {
                    let mode = *mode;
                    edit(changers, usize::from(changer), |changer| {
                        if let Some(point) = changer.points.get_mut(point) {
                            point.interpolation = mode;
                        }
                    });
                }
            }
            _ => return false,
        }
        true
    }

    /// The slider an animator control addresses and its value.
    #[must_use]
    pub fn slider(&self, changers: &[SettingsChanger], env: &Env<'_>, reference: u16, field: u8) -> Option<(ze::SliderInfo, f32)> {
        match ze::zone_ref(reference) {
            ZoneRef::Creator => {
                let info = animators::creator_field(field);
                Some((animators::creator_slider_info(&info), self.creator.value(field)))
            }
            ZoneRef::Animator(index) => {
                let changer = changers.get(index)?;
                // The point editor's fields: the selected point's time, value and easing.
                if matches!(field, ze::ANIM_POINT_TIME | ze::ANIM_POINT_VALUE | ze::ANIM_POINT_EASE) {
                    let (_, slot) = self.point.filter(|(selected, _)| *selected == index)?;
                    let point = changer.points.get(slot)?;
                    let linear = crate::dyn_panel::SliderMode::Linear;
                    return Some(match field {
                        ze::ANIM_POINT_TIME => (
                            ze::SliderInfo { min: 0.0, max: 1.0e9, default: 0.0, mode: linear, units: "h", precision: 2, si: false, factor: animators::HOURS },
                            point.x,
                        ),
                        ze::ANIM_POINT_VALUE => (
                            ze::SliderInfo { min: -1.0e6, max: 1.0e6, default: 1.0, mode: linear, units: "", precision: 2, si: false, factor: 1.0 },
                            point.y,
                        ),
                        _ => (
                            ze::SliderInfo { min: 1.0, max: 10.0, default: 2.0, mode: linear, units: "", precision: 1, si: false, factor: 1.0 },
                            point.ease_factor,
                        ),
                    });
                }
                if field == ze::ANIM_START {
                    let info = animators::creator_field(animators::START);
                    return Some((animators::creator_slider_info(&info), changer.start));
                }
                let base = changer.settings_bases.get(usize::from(field.checked_sub(ze::ANIM_BASE_FIRST)?))?;
                let entry = animators::catalog(env.settings, env.zones).into_iter().find(|entry| entry.key == base.key)?;
                Some((animators::base_slider_info(&entry), base.base))
            }
            _ => None,
        }
    }

    /// An animator slider takes `value` (as it is for a typed value, kept inside the slider's range when dragged).
    pub fn set_value(&mut self, changers: &mut [SettingsChanger], env: &Env<'_>, reference: u16, field: u8, value: f32, clamp: bool) {
        let Some((info, _)) = self.slider(changers, env, reference, field) else {
            return;
        };
        let value = if clamp { value.clamp(info.min, info.max) } else { value };
        match ze::zone_ref(reference) {
            ZoneRef::Creator => {
                if clamp {
                    self.creator.set(field, value);
                } else {
                    // A typed value may go past the ends, but a time is never negative and the cycle's period is never zero.
                    self.creator.set(field, value.max(info.min.min(value)));
                }
            }
            ZoneRef::Animator(index) => {
                if matches!(field, ze::ANIM_POINT_TIME | ze::ANIM_POINT_VALUE | ze::ANIM_POINT_EASE) {
                    self.set_point_field(changers, env, index, field, value);
                    return;
                }
                let slot = usize::from(field.saturating_sub(ze::ANIM_BASE_FIRST));
                edit(changers, index, |changer| {
                    if field == ze::ANIM_START {
                        changer.start = value.max(0.0);
                    } else if let Some(base) = changer.settings_bases.get_mut(slot) {
                        base.base = value;
                    }
                });
            }
            _ => {}
        }
    }

    /// An animator slider follows the cursor (`cursor_x` in screen px, `track` in reference px).
    pub fn slide(&mut self, changers: &mut [SettingsChanger], env: &Env<'_>, reference: u16, field: u8, track: (f32, f32), cursor_x: f32) {
        let span = track.1 - track.0;
        if span <= 0.0 {
            return;
        }
        let fraction = (cursor_x / env.scale - track.0) / span;
        if let Some((info, _)) = self.slider(changers, env, reference, field) {
            let value = crate::dyn_panel::value_range(info.mode, info.min, info.max, info.default, fraction);
            self.set_value(changers, env, reference, field, value, true);
        }
    }

    /// The Repeat toggle of animator `index`.
    pub fn toggle_repeat(changers: &mut [SettingsChanger], index: usize) {
        edit(changers, index, |changer| changer.repeat = !changer.repeat);
    }

    /// `SettingBaseItem.Remove`: the animator lets go of its `base`th setting.
    pub fn remove_base(changers: &mut [SettingsChanger], index: usize, base: usize) {
        edit(changers, index, |changer| {
            if base < changer.settings_bases.len() {
                changer.settings_bases.remove(base);
            }
        });
    }

    /// The frame animator `index`'s graph shows: the kept one, or the curve's own (`UpdateMinMax`).
    pub fn graph_view(&mut self, changers: &[SettingsChanger], index: usize) -> Option<animators::GraphView> {
        let changer = changers.get(index)?;
        if self.views.len() < changers.len() {
            self.views.resize(changers.len(), None);
        }
        let points = &changer.points;
        Some(*self.views[index].get_or_insert_with(|| animators::graph_view(points)))
    }

    /// `CheckBoundsForPoint`: the frame is made again when a point left it or it became far too big.
    fn check_view(&mut self, changers: &[SettingsChanger], index: usize) {
        let Some(view) = self.graph_view(changers, index) else {
            return;
        };
        let points = &changers[index].points;
        if animators::needs_reframe(&view, points) {
            self.views[index] = Some(animators::graph_view(points));
        }
    }

    /// A press on the plot of animator `index`'s graph (`rect`: its edges in reference px, `cursor` in screen px): the point under it is picked
    /// and dragged; with Ctrl the point goes (`RemovePoint`), or on empty ground a new one is added (`AddPoint`). Returns whether a point was
    /// picked for dragging (the caller then closes any open edit field).
    pub fn graph_press(&mut self, changers: &mut [SettingsChanger], env: &Env<'_>, index: usize, rect: [f32; 4], cursor: [f32; 2], ctrl: bool) -> bool {
        let Some(view) = self.graph_view(changers, index) else {
            return false;
        };
        let size = [rect[2] - rect[0], rect[3] - rect[1]];
        let local = [cursor[0] / env.scale - rect[0], rect[3] - cursor[1] / env.scale];
        let value = view.from_rect(local, size);
        let hit = changers[index].points.iter().position(|point| {
            let at = view.to_rect([point.x, point.y], size);
            (at[0] - local[0]).hypot(at[1] - local[1]) <= 10.0
        });
        match (hit, ctrl) {
            (Some(point), true) => self.remove_point(changers, index, point),
            (None, true) => {
                let mut added = None;
                edit(changers, index, |changer| added = animators::add_point(&mut changer.points, value));
                if let Some(point) = added {
                    self.point = Some((index, point));
                    self.check_view(changers, index);
                }
            }
            (Some(point), false) => {
                self.point = Some((index, point));
                self.drag = Some((index, point, rect));
                return true;
            }
            (None, false) => self.point = None,
        }
        false
    }

    /// The dragged point follows the cursor (`OnDragHandleMoved`).
    pub fn graph_drag(&mut self, changers: &mut [SettingsChanger], env: &Env<'_>, cursor: [f32; 2]) {
        let Some((index, point, rect)) = self.drag else {
            return;
        };
        let Some(view) = self.graph_view(changers, index) else {
            return;
        };
        let size = [rect[2] - rect[0], rect[3] - rect[1]];
        let local = [
            (cursor[0] / env.scale - rect[0]).clamp(0.0, size[0]),
            (rect[3] - cursor[1] / env.scale).clamp(0.0, size[1]),
        ];
        let value = view.from_rect(local, size);
        let mut now = point;
        edit(changers, index, |changer| now = animators::move_point(&mut changer.points, point, value));
        self.drag = Some((index, now, rect));
        self.point = Some((index, now));
        self.check_view(changers, index);
    }

    /// `RemovePoint`: the first point and the last two stay.
    pub fn remove_point(&mut self, changers: &mut [SettingsChanger], index: usize, point: usize) {
        let mut removed = false;
        edit(changers, index, |changer| removed = animators::remove_point(&mut changer.points, point));
        if removed {
            self.point = None;
            self.check_view(changers, index);
        }
    }

    /// A typed time or value (or the easing slider) of the selected point.
    fn set_point_field(&mut self, changers: &mut [SettingsChanger], _env: &Env<'_>, index: usize, field: u8, value: f32) {
        let Some((_, slot)) = self.point.filter(|(selected, _)| *selected == index) else {
            return;
        };
        let mut now = slot;
        edit(changers, index, |changer| {
            let Some(point) = changer.points.get(slot).copied() else {
                return;
            };
            match field {
                ze::ANIM_POINT_TIME => now = animators::move_point(&mut changer.points, slot, [value, point.y]),
                ze::ANIM_POINT_VALUE => now = animators::move_point(&mut changer.points, slot, [point.x, value]),
                _ => changer.points[slot].ease_factor = value.clamp(1.0, 10.0),
            }
        });
        self.point = Some((index, now));
        self.check_view(changers, index);
    }
}

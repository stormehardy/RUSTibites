//! The in-world dynamic settings panel (`DynamicSettingsPanel`, the pen button).
//!
//! The game builds this panel at runtime from `MenuDynamicSettingsManager`: tabs of setting
//! groups whose rows are toggles, linear/log/int sliders and master-toggle sub-groups, all
//! bound to `ScenarioSettings` / `ScenarioIndependentSettings`. The capture only has the first
//! tab with captured (frozen) values, so the panel is regenerated here: the captured chrome
//! (frame, tab buttons, scroll bar) and the captured row prefabs are cloned and repositioned
//! with live labels and values, so the look is the captured one and the contents are live.

use bibites_core::format::FloatValueFormat;
use bibites_core::settings::{MaterialKind, SettingValue};

use crate::setting_ui_data::{SETTING_UI, SettingScope, SettingUi, SettingUiKind};
use crate::ui::UiAction;
use crate::ui_runtime::Node;
use crate::value_edit::{self, Rules, ValueEdit};

mod animator_cards_ui;
mod zone_cards_ui;

/// Reference size the capture was made at.
const REFERENCE_HEIGHT: f32 = 1080.0;
/// Content viewport (`Viewport`): reference px, y down.
const VIEWPORT: [f32; 4] = [929.0, 143.0, 1886.0, 1071.0];
/// Rows span `x0..x1` inside the viewport (15 px padding).
const CONTENT_X: [f32; 2] = [944.0, 1871.0];
/// Scroll bar track (`Scrollbar Vertical`): x0, y0, x1, y1.
const SCROLLBAR: [f32; 4] = [1886.0, 163.0, 1906.0, 1051.0];
/// Row height of toggles and sliders; vertical spacing; group title height.
const ROW_H: f32 = 40.0;
const SPACING: f32 = 20.0;
const TITLE_H: f32 = 50.0;
/// Tab buttons: x of each tab, size, y.
const TAB_X: [f32; 5] = [935.0, 1000.0, 1065.0, 1130.0, 1195.0];
const TAB_Y: f32 = 80.0;
const TAB_SIZE: f32 = 60.0;
/// Panel rectangle (x0, y0, x1, y1).
pub const PANEL: [f32; 4] = [920.0, 80.0, 1920.0, 1080.0];

/// Where a scrolling list of setting groups sits on the screen (the in-world dynamic panel
/// and the scenario editor use the same row prefabs in different frames).
#[derive(Clone, Copy, Debug)]
pub struct ListGeometry {
    /// Visible area `x0, y0, x1, y1` (reference px, y down).
    pub viewport: [f32; 4],
    /// Rows span `x0..x1` (also the width of an expanded sub-panel).
    pub content_x: [f32; 2],
    /// Scroll bar track `x0, y0, x1, y1`.
    pub scrollbar: [f32; 4],
    /// Gap between the viewport top and the first group.
    pub top_pad: f32,
    /// Gap between the last group and the end of the content.
    pub bottom_pad: f32,
}

/// Geometry of the in-world dynamic settings panel.
pub const DYN_GEOMETRY: ListGeometry = ListGeometry {
    viewport: VIEWPORT,
    content_x: CONTENT_X,
    scrollbar: SCROLLBAR,
    top_pad: 15.0,
    bottom_pad: 15.0,
};

/// Geometry of the in-game panel's Animation tab (`AnimationsPanel`: its scroll view is inset 16 from the panel, the content 15 more; read from the live game).
const ANIMATION_GEOMETRY: ListGeometry = ListGeometry {
    viewport: [936.0, 150.0, 1904.0, 1064.0],
    content_x: [951.0, 1889.0],
    scrollbar: SCROLLBAR,
    top_pad: 15.0,
    bottom_pad: 50.0,
};

/// Scene path of the Zones tab's `TogglePreview` button (its tooltip is found by it).
const TOGGLE_PREVIEW_PATH: &str = "UICanvas/MainUI/Layer1/Panels/Right Panels/DynamicSettingsPanel/ZoneEditorPanel/Scroll View/Viewport/Content/TogglePreview";

/// Whether `session` is one of the generated panels (flat sprite-less images, own toggle hits).
#[must_use]
pub fn is_generated(session: &str) -> bool {
    matches!(session, "dyn_panel" | "scenario_editor")
}

/// How a slider maps its position to the setting value.
#[derive(Clone, Copy, Debug)]
pub enum SliderMode {
    /// `FloatSettingSlider`.
    Linear,
    /// `IntSettingSlider` (whole numbers).
    Int,
    /// `LogFloatSettingSlider(base, span_if_zero)`.
    Log { base: f32, span: Option<f32> },
}

/// One row (or nested group) of a tab.
#[derive(Clone, Debug)]
pub enum Row {
    /// `SettingToggle`.
    Toggle(&'static str),
    /// A slider row.
    Slider(&'static str, SliderMode),
    /// `SubSettingsGroupHandle` with a master toggle and a collapsible sub-panel.
    Sub {
        title: &'static str,
        master: &'static str,
        rows: Vec<Row>,
    },
    /// A nested `SettingsGroupHandle`.
    Group { title: &'static str, rows: Vec<Row> },
    /// `FixedSumSettingsGroupHandle`: the member values always add up to `sum`.
    FixedSum {
        title: &'static str,
        sum: f32,
        rows: Vec<Row>,
    },
    /// Header of a list editor: a `+` button, the title and a "remove all" button (zones, zone
    /// groups, default bibites, animators). `template` is the captured node to clone.
    Header {
        template: &'static str,
        title: &'static str,
        add: UiAction,
        clear: UiAction,
    },
    /// Vertical space of `.0` px.
    Gap(f32),
    /// A captured panel cloned as it is, with click regions on its named buttons.
    Panel {
        template: &'static str,
        buttons: Vec<(&'static str, UiAction)>,
    },
    /// A big section title (`Title (1)`: "Bibite Settings").
    Heading(&'static str),
    /// An empty list: a box of `height` px holding the grey placeholder line.
    EmptyList {
        template: &'static str,
        text: &'static str,
        height: f32,
    },
    /// A collapsible material block of the Energy tab (`MaterialParametersGroupHandle`):
    /// material index 0 Plant, 1 Meat, 2 Fat, 3 Armor.
    Material(u8),
    /// The items of list `.0` (0 zones, 1 zone groups) as collapsed boxes; while the list is
    /// empty it is the placeholder box instead.
    Items {
        list: u8,
        template: &'static str,
        height: f32,
    },
}

/// A top-level group of a tab (`SettingGroupPrefab`).
#[derive(Clone, Debug)]
pub struct TabGroup {
    /// Group title.
    pub title: &'static str,
    /// Rows.
    pub rows: Vec<Row>,
}

/// `FloatSettingSlider`.
pub const LINEAR: SliderMode = SliderMode::Linear;
/// `IntSettingSlider`.
pub const INT: SliderMode = SliderMode::Int;

/// `LogFloatSettingSlider(base, span)`.
#[must_use]
pub fn log(base: f32, span: Option<f32>) -> SliderMode {
    SliderMode::Log { base, span }
}

/// The groups of a tab, as `MenuDynamicSettingsManager` declares them.
#[must_use]
pub fn tab_groups(tab: u8) -> Vec<TabGroup> {
    use Row::{Group, Slider, Sub, Toggle};
    match tab {
        0 => vec![
            TabGroup {
                title: "Simulation Options",
                rows: vec![
                    Toggle("corpsesEnabled"),
                    Toggle("preventMutations"),
                    Sub {
                        title: "Pellet Options",
                        master: "pelletCollision",
                        rows: vec![Toggle("pelletRotation"), Toggle("pelletMerge")],
                    },
                    Toggle("disableHerding"),
                    Sub {
                        title: "Shade Effect",
                        master: "shadeOutsideOfBounds",
                        rows: vec![Toggle("shadeAvoidance"), Toggle("worldWrapping")],
                    },
                    Sub {
                        title: "Red Death Bloom Settings",
                        master: "enableRedDeath",
                        rows: vec![
                            Slider("redDeathFill", LINEAR),
                            Slider("redDeathBaseDamages", log(10.0, Some(3.0))),
                            Slider("redDeathDamageVelocity", log(10.0, Some(3.0))),
                        ],
                    },
                    Toggle("preventRedPheroProduction"),
                    Toggle("preventGreenPheroProduction"),
                    Toggle("preventBluePheroProduction"),
                ],
            },
            TabGroup {
                title: "Cheat Options",
                rows: vec![Sub {
                    title: "Void Avoidance Settings",
                    master: "voidAvoidance",
                    rows: vec![Slider("voidAvoidanceDistance", LINEAR)],
                }],
            },
            TabGroup {
                title: "Simulation Performance Parameters",
                rows: vec![
                    Slider("simTPS", SliderMode::Int),
                    Slider("brainTPS", SliderMode::Int),
                    Slider("decayTPS", SliderMode::Int),
                    Slider("visionLookupUpdateFactor", SliderMode::Int),
                    Slider("visionSenseUpdateFactor", SliderMode::Int),
                    Slider("minPelletSize", log(10.0, Some(3.5))),
                ],
            },
            TabGroup {
                title: "Physics Parameters",
                rows: vec![
                    Slider("dragCoefficient", LINEAR),
                    Toggle("disableDragOnHeldPellet"),
                ],
            },
        ],
        // `ZoneEditorPanel`: the "Zones Parameters" group (`MenuDynamicSettingsManager`), then the
        // same Zones and Zone Groups lists as the scenario editor's tab, now editing the live zones.
        1 => vec![
            TabGroup {
                title: "Zones Parameters",
                rows: vec![
                    Slider("biomassDensity", log(10.0, Some(3.5))),
                    Slider("pelletGrowth", log(10.0, Some(3.5))),
                    Slider("globalZoneSpeed", log(10.0, Some(6.0))),
                    Slider("pelletEnergy", log(10.0, None)),
                ],
            },
            // The wrapper of the parameters group is taller than the group (as in the editor).
            TabGroup {
                title: "",
                rows: vec![Row::Gap(40.0)],
            },
            TabGroup {
                title: "",
                rows: vec![Row::Header {
                    template: "Zones Header",
                    title: "Zones",
                    add: UiAction::Invoke(crate::ui::ControlCommand::AddZone),
                    clear: UiAction::Invoke(crate::ui::ControlCommand::RequestClearZones),
                }],
            },
            TabGroup {
                title: "",
                rows: vec![Row::Items {
                    list: 0,
                    template: "NoZones",
                    height: 100.0,
                }],
            },
            TabGroup {
                title: "",
                rows: vec![Row::Header {
                    template: "Group Header",
                    title: "Zone Groups",
                    add: UiAction::Invoke(crate::ui::ControlCommand::AddZoneGroup),
                    clear: UiAction::Invoke(crate::ui::ControlCommand::ClearZoneGroups),
                }],
            },
            TabGroup {
                title: "",
                rows: vec![Row::Items {
                    list: 1,
                    template: "NoGroups",
                    height: 99.0,
                }],
            },
        ],
        2 => vec![
            // `DefaultBibitesPanel`: the `+`, the title, "remove all", then the items.
            TabGroup {
                title: "",
                rows: vec![
                    // (The in-game panel's title is "Bibite Templates" and no "Bibite Settings" heading follows the list: both read from the
                    // live game; the scenario editor's own declaration has the other two.)
                    Row::Header {
                        template: "Top Header",
                        title: "Bibite Templates",
                        add: UiAction::Invoke(crate::ui::ControlCommand::AddDefaultBibite),
                        clear: UiAction::Invoke(
                            crate::ui::ControlCommand::RequestClearDefaultBibites,
                        ),
                    },
                    Row::Items {
                        list: 2,
                        template: "",
                        height: 0.0,
                    },
                ],
            },
            TabGroup {
                title: "Virgin Bibite Options",
                rows: vec![
                    Slider("virginSpawnRate", LINEAR),
                    Sub {
                        title: "Limit Bibite Birth",
                        master: "limitBibiteBirth",
                        rows: vec![Slider("bibiteLimit", SliderMode::Int)],
                    },
                    Slider("backgroundMutationChance", LINEAR),
                    Slider("backgroundMutationVariance", LINEAR),
                    Slider("relativeMutationShare", LINEAR),
                    Row::FixedSum {
                        title: "Brain Mutation Probabilities",
                        sum: 1.0,
                        rows: vec![
                            Slider("synapseMutationChance", LINEAR),
                            Slider("neuronMutationChance", LINEAR),
                        ],
                    },
                    Row::FixedSum {
                        title: "Synapse Mutation Probabilities",
                        sum: 1.0,
                        rows: vec![
                            Slider("synapseChangeChance", LINEAR),
                            Slider("synapseFlipChance", LINEAR),
                            Slider("synapseToggleChance", LINEAR),
                            Slider("synapseAddChance", LINEAR),
                            Slider("synapseRemoveChance", LINEAR),
                        ],
                    },
                    Row::FixedSum {
                        title: "Neuron Mutation Probabilities",
                        sum: 1.0,
                        rows: vec![
                            Slider("neuronDefaultChance", LINEAR),
                            Slider("neuronChangeChance", LINEAR),
                            Slider("neuronAddChance", LINEAR),
                            Slider("neuronRemoveChance", LINEAR),
                        ],
                    },
                ],
            },
        ],
        3 => vec![
            TabGroup {
                title: "Bibite Balance",
                rows: vec![
                    Slider("armMusclePressure", LINEAR),
                    Slider("forwardForceSizePower", LINEAR),
                    Slider("turnForceSizePower", LINEAR),
                    Slider("backwardFraction", LINEAR),
                    Slider("bitingPressure", LINEAR),
                    Slider("jawMusclesSizingPower", LINEAR),
                    Slider("bitePeriodFactor", LINEAR),
                    Slider("throwingForceFactor", LINEAR),
                    Slider("bitingThrowForceFactor", LINEAR),
                    Slider("bitingDamageFactor", LINEAR),
                    Slider("healRate", LINEAR),
                    Slider("healPowerFactor", LINEAR),
                    Slider("healEfficiency", LINEAR),
                    Slider("corpseDecayFactor", log(10.0, Some(4.0))),
                    Group {
                        title: "Energy Balance",
                        rows: vec![
                            Slider("energyUsageEfficiency", LINEAR),
                            Slider("pheromoneProductionCost", LINEAR),
                            Slider("fatSustain", LINEAR),
                            Slider("pheromoneProductionStrength", LINEAR),
                            Slider("baseMetabolismCost", LINEAR),
                            Slider("moveMusclesCost", LINEAR),
                            Slider("neuronBirthCost", LINEAR),
                            Slider("synapseBirthCost", LINEAR),
                            Slider("neuronUpkeepCost", LINEAR),
                            Slider("synapseUpkeepCost", LINEAR),
                        ],
                    },
                    Slider("storableEnergyPerArea", LINEAR),
                    Group {
                        title: "Bibite Aging",
                        rows: vec![
                            Slider("ageingThreshold", LINEAR),
                            Slider("ageStrengthPenalties", LINEAR),
                            Slider("ageMetabolismPenalties", LINEAR),
                        ],
                    },
                ],
            },
            // `MaterialParameters`: the four `MatterMaterialSettingsHandle` blocks (Plant, Meat, Fat,
            // Armor), then the two affinity sliders.
            TabGroup {
                title: "Materials Parameters",
                rows: {
                    let mut rows: Vec<Row> = (0..4).map(Row::Material).collect();
                    rows.push(Slider("plantAffinityPowerFactor", LINEAR));
                    rows.push(Slider("meatAffinityPowerFactor", LINEAR));
                    rows
                },
            },
        ],
        // `AnimationsPanel` (read from the live game): an empty `World Parameters` wrapper 60 high, the "Settings Animator" header and the list's
        // placeholder (the port keeps no settings animators, so the list is always empty and "remove all" greyed).
        4 => vec![
            TabGroup {
                title: "",
                rows: vec![Row::Gap(60.0)],
            },
            TabGroup {
                title: "",
                rows: vec![Row::Header {
                    template: "Animator Header",
                    title: "Settings Animator",
                    add: UiAction::Invoke(crate::ui::ControlCommand::OpenAnimatorCreator),
                    clear: UiAction::Invoke(crate::ui::ControlCommand::RequestClearAnimators),
                }],
            },
            TabGroup {
                title: "",
                rows: vec![Row::Items {
                    list: 3,
                    template: "NoAnimator",
                    height: 99.8,
                }],
            },
        ],
        _ => Vec::new(),
    }
}

/// Metadata of a core setting key (scenario settings first, then the independents).
#[must_use]
pub fn ui_of(key: &str) -> Option<(usize, &'static SettingUi)> {
    SETTING_UI
        .iter()
        .enumerate()
        .find(|(_, row)| row.field == key)
}

/// Current value of a setting as `f32` (`1.0` / `0.0` for booleans).
#[must_use]
pub fn read_value(settings: &bibites_core::settings::SimSettings, row: &SettingUi) -> f32 {
    let value = match row.scope {
        SettingScope::Scenario => settings.scenario.get_by_key(row.field),
        SettingScope::Independent => settings.independents.get_by_key(row.field),
    };
    match value {
        Some(SettingValue::Float(v)) => v,
        Some(SettingValue::Int(v)) => v as f32,
        Some(SettingValue::Bool(v)) => f32::from(u8::from(v)),
        None => row.default,
    }
}

/// `SettingValue` for writing `value` into `row` (rounded for integers).
#[must_use]
pub fn make_value(row: &SettingUi, value: f32) -> SettingValue {
    match row.kind {
        SettingUiKind::Float => SettingValue::Float(value),
        SettingUiKind::Int => SettingValue::Int(value.round() as i32),
        SettingUiKind::Bool => SettingValue::Bool(value >= 0.5),
    }
}

/// `FloatValueFormat` of a numeric setting (`NumericSetting.formatting`).
#[must_use]
pub fn format_of(row: &SettingUi) -> FloatValueFormat {
    FloatValueFormat {
        prefix: row.prefix,
        units: row.units,
        precision: row.precision,
        factor: row.factor,
        si: row.si,
        always_show_sign: row.always_show_sign,
        precision_is_si: false,
        is_int: row.kind == SettingUiKind::Int,
    }
}

/// Slider-space bounds `(min, max)` and the log reference value of a slider.
fn slider_space(mode: SliderMode, row: &SettingUi, default: f32) -> (f32, f32, f32) {
    match mode {
        SliderMode::Linear | SliderMode::Int => (row.min, row.max, 1.0),
        SliderMode::Log { base, span } => {
            let reference = if default.abs() < 1e-9 { 1.0 } else { default };
            let to_log = |v: f32| (v / reference).ln() / base.ln();
            let max = to_log(row.max);
            let min = if row.min <= 0.0 {
                max - span.unwrap_or(5.0)
            } else {
                to_log(row.min)
            };
            (min, max, reference)
        }
    }
}

/// Slider position `0..=1` of `value` for a slider over `min..=max` (`LogSettingSlider` when
/// `mode` is `Log`: the slider space is `log_base(value / default)`).
#[must_use]
pub fn fraction_range(mode: SliderMode, min: f32, max: f32, default: f32, value: f32) -> f32 {
    let row = SettingUi {
        min,
        max,
        ..crate::setting_ui_data::SETTING_UI[0]
    };
    fraction_of(mode, &row, default, value)
}

/// Value at slider position `fraction` for a slider over `min..=max`.
#[must_use]
pub fn value_range(mode: SliderMode, min: f32, max: f32, default: f32, fraction: f32) -> f32 {
    let row = SettingUi {
        min,
        max,
        ..crate::setting_ui_data::SETTING_UI[0]
    };
    value_at(mode, &row, default, fraction)
}

/// Slider position `0..=1` of `value`.
#[must_use]
pub fn fraction_of(mode: SliderMode, row: &SettingUi, default: f32, value: f32) -> f32 {
    let (min, max, _) = slider_space(mode, row, default);
    let position = match mode {
        SliderMode::Linear | SliderMode::Int => value,
        SliderMode::Log { base, .. } => {
            let (_, _, reference) = slider_space(mode, row, default);
            if value > 0.0 {
                (value / reference).ln() / base.ln()
            } else {
                min
            }
        }
    };
    if (max - min).abs() < 1e-9 {
        return 0.0;
    }
    ((position - min) / (max - min)).clamp(0.0, 1.0)
}

/// Setting value at slider position `fraction` (`SettingValueOfSlider`).
#[must_use]
pub fn value_at(mode: SliderMode, row: &SettingUi, default: f32, fraction: f32) -> f32 {
    let (min, max, reference) = slider_space(mode, row, default);
    let position = min + fraction.clamp(0.0, 1.0) * (max - min);
    match mode {
        SliderMode::Linear => position,
        SliderMode::Int => position.round(),
        SliderMode::Log { base, .. } => {
            if (position - min).abs() < 1e-6 {
                row.min
            } else {
                reference * base.powf(position)
            }
        }
    }
}

/// [`slider_info`] of a row, or the plain slider of its kind when no in-world tab lists it.
#[must_use]
pub fn slider_info_or_default(row: &SettingUi) -> (SliderMode, Option<(f32, Vec<&'static str>)>) {
    // A setting the in-world tabs do not list (the species settings of the Lineage panel's Settings panel) is a
    // plain slider: an int one for an int setting (`IntSettingSlider`), else a linear one (`FloatSettingSlider`).
    slider_info(row.field).unwrap_or(if row.kind == SettingUiKind::Int {
        (INT, None)
    } else {
        (LINEAR, None)
    })
}

/// Slider mode of `key` and, when it belongs to a `FixedSum` group, `(sum, all member keys)`.
pub fn slider_info(key: &str) -> Option<(SliderMode, Option<(f32, Vec<&'static str>)>)> {
    fn walk(
        rows: &[Row],
        key: &str,
        fixed: Option<(f32, Vec<&'static str>)>,
    ) -> Option<(SliderMode, Option<(f32, Vec<&'static str>)>)> {
        for row in rows {
            match row {
                Row::Slider(name, mode) if *name == key => return Some((*mode, fixed)),
                Row::Sub { rows, .. } | Row::Group { rows, .. } => {
                    if let Some(found) = walk(rows, key, None) {
                        return Some(found);
                    }
                }
                Row::FixedSum { sum, rows, .. } => {
                    let members: Vec<&'static str> = rows
                        .iter()
                        .filter_map(|row| match row {
                            Row::Slider(name, _) => Some(*name),
                            _ => None,
                        })
                        .collect();
                    if let Some(found) = walk(rows, key, Some((*sum, members))) {
                        return Some(found);
                    }
                }
                _ => {}
            }
        }
        None
    }
    let search =
        |groups: Vec<TabGroup>| groups.iter().find_map(|group| walk(&group.rows, key, None));
    (0..5).find_map(|tab| search(tab_groups(tab))).or_else(|| {
        (0..crate::scenario_editor::TAB_COUNT as u8)
            .find_map(|tab| search(crate::scenario_editor::editor_groups(tab)))
    })
}

/// Keys of the settings under sub-group `group_id` of `tab` (for "reset sub-panel").
#[must_use]
pub fn sub_group_keys(tab: u8, group_id: u16) -> Vec<&'static str> {
    sub_group_keys_in(&tab_groups(tab), group_id)
}

/// Keys of the settings under sub-group `group_id` of a list of groups.
#[must_use]
pub fn sub_group_keys_in(groups: &[TabGroup], group_id: u16) -> Vec<&'static str> {
    fn count(rows: &[Row], next: &mut u16, target: u16, out: &mut Vec<&'static str>) {
        for row in rows {
            match row {
                Row::Sub { master, rows, .. } => {
                    let id = *next;
                    *next += 1;
                    if id == target {
                        out.push(master);
                        for inner in rows {
                            if let Row::Toggle(key) | Row::Slider(key, _) = inner {
                                out.push(key);
                            }
                        }
                    }
                    count(rows, next, target, out);
                }
                Row::Group { rows, .. } | Row::FixedSum { rows, .. } => {
                    count(rows, next, target, out);
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    let mut next = 0;
    for group in groups {
        count(&group.rows, &mut next, group_id, &mut out);
    }
    out
}

/// A clickable region (reference px) and what it does.
#[derive(Clone, Copy, Debug)]
pub struct DynHit {
    /// `x, y, w, h` (reference px, y down).
    pub rect: [f32; 4],
    /// Action emitted on click.
    pub action: UiAction,
}

/// A hover region (reference px) that shows a tooltip: the name label of a settings row
/// (`SettingSlider` / `SettingToggle` set the trigger's text to the setting's name and help text).
#[derive(Clone, Copy, Debug)]
pub struct DynTip {
    /// `x, y, w, h` (reference px, y down).
    pub rect: [f32; 4],
    /// The trigger.
    pub tip: crate::tooltip::Tip,
}

/// Mutable state of the panel (kept in `App`).
#[derive(Clone, Debug, Default)]
pub struct DynState {
    /// Selected tab (0 simulation, 1 zones, 2 bibite, 3 energy, 4 animation).
    pub tab: u8,
    /// Scroll offset of the content (reference px).
    pub scroll: f32,
    /// Open sub-panels (`tab * 256 + group id`).
    pub open_subs: Vec<u32>,
    /// Slider being dragged: setting index and track x range (reference px).
    pub drag: Option<(u16, u16, u16)>,
    /// Slider of a default bibite being dragged: item, field and track x range.
    pub bibite_drag: Option<(u16, u8, u16, u16)>,
    /// Slider of a zone being dragged: item, field and track x range.
    pub zone_drag: Option<(u16, u8, u16, u16)>,
    /// Slider of a material block being dragged: material, field and track x range.
    pub material_drag: Option<(u8, u8, u16, u16)>,
    /// Items whose edit section is open: `(list, index)` (the Default Bibites list is list 2).
    pub open_items: Vec<(u8, u16)>,
    /// The order the cards of the zone, zone group and animator lists are shown in (`ReorderableItem` moves the cards, not the data): the
    /// item indices of each list, first card first. Indices that are missing or too big are repaired by [`DynState::order_of`].
    pub order: [Vec<u16>; 4],
    /// Largest useful scroll offset of the last built frame.
    pub max_scroll: std::cell::Cell<f32>,
}

/// The item indices of a list of `len` items in the order its cards are shown in: the kept order, and every index it lacks after it.
#[must_use]
pub fn card_order(order: &[Vec<u16>; 4], list: u8, len: usize) -> Vec<usize> {
    let mut seen = vec![false; len];
    let mut out = Vec::with_capacity(len);
    for index in order.get(usize::from(list)).map_or(&[][..], Vec::as_slice) {
        let index = usize::from(*index);
        if index < len && !seen[index] {
            seen[index] = true;
            out.push(index);
        }
    }
    out.extend((0..len).filter(|index| !seen[*index]));
    out
}

impl DynState {
    /// `ReorderableItem.MoveUp` / `MoveDown`: item `index` of `list` (which has `len` items) swaps places with the card before or after it
    /// (nothing at the ends).
    pub fn move_card(&mut self, list: u8, len: usize, index: usize, up: bool) {
        if usize::from(list) >= self.order.len() {
            return;
        }
        let mut order = card_order(&self.order, list, len);
        let Some(at) = order.iter().position(|item| *item == index) else {
            return;
        };
        let to = if up { at.checked_sub(1) } else { Some(at + 1).filter(|to| *to < order.len()) };
        if let Some(to) = to {
            order.swap(at, to);
            self.order[usize::from(list)] = order.into_iter().map(|item| item as u16).collect();
        }
    }

    /// Item `index` of `list` was removed: the later ones are one place nearer the front and the cards keep their order.
    pub fn card_removed(&mut self, list: u8, index: usize) {
        if let Some(order) = self.order.get_mut(usize::from(list)) {
            order.retain(|item| usize::from(*item) != index);
            for item in order.iter_mut() {
                if usize::from(*item) > index {
                    *item -= 1;
                }
            }
        }
    }

    /// Every item of `list` was removed.
    pub fn cards_cleared(&mut self, list: u8) {
        if let Some(order) = self.order.get_mut(usize::from(list)) {
            order.clear();
        }
    }

    /// Whether the sub-panel with this id is open.
    #[must_use]
    pub fn is_open(&self, group: u16) -> bool {
        self.open_subs
            .contains(&(u32::from(self.tab) * 256 + u32::from(group)))
    }

    /// Opens or closes a sub-panel.
    pub fn toggle_sub(&mut self, group: u16) {
        let key = u32::from(self.tab) * 256 + u32::from(group);
        if let Some(position) = self.open_subs.iter().position(|open| *open == key) {
            self.open_subs.remove(position);
        } else {
            self.open_subs.push(key);
        }
    }
}

/// The scenario's zone lists (the scenario editor's zones tab).
#[derive(Clone, Copy)]
pub struct ListsInput<'a> {
    /// Explicit zones.
    pub zones: &'a [bibites_core::settings::ZoneSettings],
    /// Every zone, the groups' too (`ScenarioSettings.allZones`: the zones a target or a spawn row can name).
    pub all_zones: &'a [bibites_core::settings::ZoneSettings],
    /// The settings animators (`ScenarioSettings.settingsChangers`) and the settings they could take over (names, ranges).
    pub changers: &'a [bibites_core::settings::SettingsChanger],
    /// See `changers`.
    pub catalog: &'a [crate::animators::Animatable],
    /// The graphs' frames of the animators (the corners the curves are drawn in; `None`: framed on the curve).
    pub graph_views: &'a [Option<crate::animators::GraphView>],
    /// The simulated time, in a simulation (the graph's progress marker).
    pub sim_time: Option<f32>,
    /// The selected point of an animator's curve: `(animator, point)`.
    pub selected_point: Option<(usize, usize)>,
    /// The animator creator panel, when it is open.
    pub creator: Option<&'a crate::animators::Creator>,
    /// Zone groups.
    pub groups: &'a [bibites_core::settings::ZoneGroupSettings],
    /// Items whose edit section is open: `(list, index)`.
    pub open: &'a [(u8, u16)],
    /// The order the cards are shown in (`DynState.order`).
    pub order: &'a [Vec<u16>; 4],
    /// The default bibites (`DefaultBibitesPanel`): their spawn rows ...
    pub bibites: &'a [bibites_core::settings::BibiteSpawn],
    /// ... and the name each item shows.
    pub bibite_names: &'a [String],
    /// Whether a simulation is running (a "One Time" row then shows how many it spawned).
    pub in_sim: bool,
    /// What the delete button of item `.1` of list `.0` does (the editor deletes at once, the
    /// world asks the app).
    pub delete_item: fn(u8, u16) -> UiAction,
    /// Simulation size (the absolute-size sliders' upper bound).
    pub sim_size: f32,
    /// The zone whose name is being typed and the text so far (the world's Zones list).
    pub name_edit: Option<(u16, &'a str)>,
}

/// Everything the builder needs to read from the app.
pub struct DynInput<'a> {
    /// Live settings.
    pub settings: &'a bibites_core::settings::SimSettings,
    /// Panel state.
    pub state: &'a DynState,
    /// Defaults of the five TPS settings (sim, brain, decay, vision lookup, vision senses).
    pub tps_defaults: [i32; 5],
    /// The open edit field of a slider row (`SettingSlider.OpenEditPanel`).
    pub edit: Option<&'a ValueEdit>,
    /// Zone lists, when the panel edits a scenario.
    pub lists: Option<ListsInput<'a>>,
}

/// Result of [`build`].
pub struct DynBuild {
    /// The panel root with generated content.
    pub root: Node,
    /// Click regions.
    pub hits: Vec<DynHit>,
    /// Hover regions with a tooltip.
    pub tips: Vec<DynTip>,
}

/// A hash of everything the panel shows (values, tab, scroll, open sub-panels), so an unchanged
/// panel can reuse its last frame.
#[must_use]
pub fn state_key(input: &DynInput<'_>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    input.state.tab.hash(&mut hasher);
    input.state.scroll.to_bits().hash(&mut hasher);
    input.state.open_subs.hash(&mut hasher);
    input.tps_defaults.hash(&mut hasher);
    // The open edit field: its row, its text, whether it is selected and which blink the caret is in.
    input
        .edit
        .map(|edit| {
            (
                edit.field,
                edit.text.as_str(),
                edit.caret,
                edit.caret_visible(),
            )
        })
        .hash(&mut hasher);
    for row in SETTING_UI {
        read_value(input.settings, row).to_bits().hash(&mut hasher);
    }
    // The Energy tab's material blocks show the whole material table.
    if input.state.tab == 3 {
        format!("{:?}", input.settings.materials).hash(&mut hasher);
    }
    if let Some(lists) = &input.lists {
        format!("{:?}", lists.bibites).hash(&mut hasher);
        lists.bibite_names.hash(&mut hasher);
        lists.open.hash(&mut hasher);
        lists.order.hash(&mut hasher);
        lists.in_sim.hash(&mut hasher);
        lists.name_edit.hash(&mut hasher);
        // The Animation tab: the animators, the creator, the picked point, and the progress marker (ten times a second).
        if input.state.tab == 4 {
            format!("{:?}", lists.changers).hash(&mut hasher);
            format!("{:?}", lists.creator).hash(&mut hasher);
            lists.selected_point.hash(&mut hasher);
            lists.sim_time.map(|time| (time * 10.0) as i64).hash(&mut hasher);
            format!("{:?}", lists.graph_views).hash(&mut hasher);
        }
        // The Zones tab shows every zone's settings (they also move by themselves).
        if input.state.tab == 1 {
            format!("{:?}", lists.zones).hash(&mut hasher);
            format!("{:?}", lists.groups).hash(&mut hasher);
            lists.sim_size.to_bits().hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Default value of a setting: the game's `DefaultValue`, except the TPS rows, whose defaults
/// are the player's `UserSettings.*TPSDefault` (`ScenarioSelectorPanel.InitPanel`).
#[must_use]
pub fn default_of(row: &SettingUi, tps_defaults: [i32; 5]) -> f32 {
    match row.field {
        "simTPS" => tps_defaults[0] as f32,
        "brainTPS" => tps_defaults[1] as f32,
        "decayTPS" => tps_defaults[2] as f32,
        "visionLookupUpdateFactor" => tps_defaults[3] as f32,
        "visionSenseUpdateFactor" => tps_defaults[4] as f32,
        _ => row.default,
    }
}

// ---------------------------------------------------------------------------
// Editing the settings (shared by the in-world panel and the scenario editor)
// ---------------------------------------------------------------------------

/// Writes `value` into `SETTING_UI[index]`; `false` when the setting does not exist.
pub fn write_value(
    settings: &mut bibites_core::settings::SimSettings,
    index: usize,
    value: f32,
) -> bool {
    let Some(row) = SETTING_UI.get(index) else {
        return false;
    };
    let value = make_value(row, value);
    match row.scope {
        SettingScope::Scenario => settings.scenario.set_by_key(row.field, value).is_some(),
        SettingScope::Independent => settings.independents.set_by_key(row.field, value).is_some(),
    }
}

/// Flips the toggle `SETTING_UI[index]`.
pub fn toggle_value(settings: &mut bibites_core::settings::SimSettings, index: usize) -> bool {
    let Some(row) = SETTING_UI.get(index) else {
        return false;
    };
    let current = read_value(settings, row) >= 0.5;
    write_value(settings, index, if current { 0.0 } else { 1.0 })
}

/// Resets settings to their defaults (the members of a fixed-sum group rebalance the others,
/// as `FixedSumSettingsGroupHandle.OnAMemberChange` does for every member change).
pub fn reset_values(
    settings: &mut bibites_core::settings::SimSettings,
    indices: &[usize],
    tps_defaults: [i32; 5],
) -> bool {
    let mut any = false;
    for index in indices {
        let Some(row) = SETTING_UI.get(*index) else {
            continue;
        };
        let default = default_of(row, tps_defaults);
        if write_value(settings, *index, default) {
            any = true;
            if let Some((_, Some((sum, members)))) = slider_info(row.field) {
                rebalance(settings, row.field, sum, &members);
            }
        }
    }
    any
}

/// Moves the slider of `SETTING_UI[index]` to reference-px position `x` of its `track`.
pub fn slide_value(
    settings: &mut bibites_core::settings::SimSettings,
    index: usize,
    track: (f32, f32),
    x: f32,
    tps_defaults: [i32; 5],
) -> bool {
    let Some(row) = SETTING_UI.get(index) else {
        return false;
    };
    let (mode, fixed) = slider_info_or_default(row);
    let span = track.1 - track.0;
    if span <= 0.0 {
        return false;
    }
    let default = default_of(row, tps_defaults);
    let value = value_at(mode, row, default, (x - track.0) / span);
    if !write_value(settings, index, value) {
        return false;
    }
    // `FixedSumSettingsGroupHandle.OnAMemberChange`: the others absorb the difference.
    if let Some((sum, members)) = fixed {
        rebalance(settings, row.field, sum, &members);
    }
    true
}

/// Writes a typed value into `SETTING_UI[index]`; the members of a fixed-sum group rebalance the
/// others as for any member change (`FixedSumSettingsGroupHandle.OnAMemberChange`).
pub fn set_value_balanced(
    settings: &mut bibites_core::settings::SimSettings,
    index: usize,
    value: f32,
) -> bool {
    let Some(row) = SETTING_UI.get(index) else {
        return false;
    };
    if !write_value(settings, index, value) {
        return false;
    }
    if let (_, Some((sum, members))) = slider_info_or_default(row) {
        rebalance(settings, row.field, sum, &members);
    }
    true
}

/// Keeps the members of a fixed-sum group adding up to `sum` after `changed` moved.
pub fn rebalance(
    settings: &mut bibites_core::settings::SimSettings,
    changed: &str,
    sum: f32,
    members: &[&'static str],
) {
    let rows: Vec<(usize, &SettingUi)> = members.iter().filter_map(|key| ui_of(key)).collect();
    let mut values: Vec<f32> = rows
        .iter()
        .map(|(_, row)| read_value(settings, row).clamp(row.min, row.max))
        .collect();
    let total: f32 = values.iter().sum();
    let in_bounds: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(position, (_, row))| {
            row.field != changed && values[*position] >= row.min && values[*position] <= row.max
        })
        .map(|(position, _)| position)
        .collect();
    if in_bounds.is_empty() {
        return;
    }
    let correction = (total - sum) / in_bounds.len() as f32;
    for position in in_bounds {
        values[position] -= correction;
    }
    for ((index, _), value) in rows.iter().zip(values) {
        write_value(settings, *index, value);
    }
}

// ---------------------------------------------------------------------------
// Node helpers
// ---------------------------------------------------------------------------

pub(crate) fn find<'a>(node: &'a Node, predicate: &dyn Fn(&Node) -> bool) -> Option<&'a Node> {
    if predicate(node) {
        return Some(node);
    }
    node.children
        .iter()
        .find_map(|child| find(child, predicate))
}

pub(crate) fn find_mut<'a>(
    node: &'a mut Node,
    predicate: &dyn Fn(&Node) -> bool,
) -> Option<&'a mut Node> {
    if predicate(node) {
        return Some(node);
    }
    node.children
        .iter_mut()
        .find_map(|child| find_mut(child, predicate))
}

/// Top (y down) of a node's captured rectangle.
pub(crate) fn top_of(node: &Node) -> f32 {
    node.native_rect
        .map_or(0.0, |[_, _, _, y1]| REFERENCE_HEIGHT - y1)
}

/// Moves a subtree `dy` reference px down (the capture's y axis points up).
pub(crate) fn shift(node: &mut Node, dy: f32) {
    if let Some(rect) = node.native_rect.as_mut() {
        rect[1] -= dy;
        rect[3] -= dy;
    }
    for child in &mut node.children {
        shift(child, dy);
    }
}

/// Sets the x range / y range of a node's rectangle (reference px, y down).
pub(crate) fn set_rect(node: &mut Node, x0: f32, y0: f32, x1: f32, y1: f32) {
    node.native_rect = Some([x0, REFERENCE_HEIGHT - y1, x1, REFERENCE_HEIGHT - y0]);
}

/// Rectangle `(x0, y0, x1, y1)` in reference px (y down).
pub(crate) fn rect_of(node: &Node) -> Option<[f32; 4]> {
    node.native_rect
        .map(|[x0, y0, x1, y1]| [x0, REFERENCE_HEIGHT - y1, x1, REFERENCE_HEIGHT - y0])
}

pub(crate) fn set_active(node: &mut Node, active: bool) {
    node.active = active;
    node.active_in_hierarchy = active;
    for child in &mut node.children {
        set_active(child, active);
    }
}

pub(crate) fn set_text(node: &mut Node, text: &str) {
    if let Some(component) = node.texts.first_mut() {
        component.text = text.to_owned();
    }
}

/// Sets a row label and shrinks it to fit its box on one line: the game's TMP labels auto-size (`text_sdf::fit_size`, with the font's real advances
/// when the fonts are loaded; Roboto-Bold averages about 0.49 em per character otherwise).
pub(crate) fn set_label(node: &mut Node, text: &str) {
    let available = rect_of(node).map_or(f32::MAX, |[x0, _, x1, _]| (x1 - x0 - 10.0).max(10.0));
    if let Some(component) = node.texts.first_mut() {
        component.text = text.to_owned();
        let max = component.font_size;
        if let Some(fonts) = crate::text_sdf::shared() {
            let name = component.font.as_ref().map(|font| font.name.clone());
            component.font_size = crate::text_sdf::fit_size(10.0_f32.min(max), max, available, |size| {
                fonts.measure_named(name.as_deref(), crate::ui::FontRole::Bold, text, size)
            });
        } else {
            let full = text.chars().count() as f32 * 0.49 * max;
            if full > available {
                component.font_size = (max * available / full).max(12.0);
            }
        }
    }
}

/// A copy of `node` without its children.
#[must_use]
pub fn shallow(node: &Node) -> Node {
    Node {
        name: node.name.clone(),
        path: node.path.clone(),
        active: node.active,
        active_in_hierarchy: node.active_in_hierarchy,
        sibling_index: node.sibling_index,
        rect: node.rect,
        native_rect: node.native_rect,
        images: node.images.clone(),
        texts: node.texts.clone(),
        canvas_scalers: node.canvas_scalers.clone(),
        layouts: node.layouts.clone(),
        controls: node.controls.clone(),
        children: Vec::new(),
    }
}

/// Deep clone that leaves out the captured rows (`Content` children).
pub(crate) fn clone_chrome(node: &Node) -> Node {
    let mut copy = shallow(node);
    if node.name != "Content" {
        copy.children = node.children.iter().map(clone_chrome).collect();
    }
    copy
}

/// Prefab templates cloned from a captured settings list.
#[derive(Clone)]
pub struct Templates {
    title: Node,
    toggle: Node,
    slider: Node,
    sub: Node,
    sub_panel: Option<Node>,
    /// List headers and placeholder lines by GameObject name (the list editors of the scenario
    /// editor: `Zones Header`, `NoZones`, `Top Header`, `Title (1)`, ...).
    extras: Vec<Node>,
}

/// The captured list headers, placeholder lines and blocks cloned as extras.
const EXTRAS: [&str; 11] = [
    "Zones Header",
    "Group Header",
    "Top Header",
    "Animator Header",
    "NoZones",
    "NoGroups",
    "NoAnimator",
    "Title (1)",
    "NotAChallengeSection",
    "TitleHolder",
    "MaterialParametersGroupHandle(Clone)",
];

/// Finds the row prefabs (group title, toggle, slider, sub-group) in a captured settings panel.
#[must_use]
pub fn templates_of(root: &Node) -> Option<Templates> {
    let holder = |n: &Node| n.path.ends_with("/SettingGroupPrefab(Clone)") && n.active_in_hierarchy;
    let group = find(root, &holder)?;
    let title = group
        .children
        .iter()
        .find(|child| child.name == "Text (TMP)")?
        .clone();
    let settings_holder = group
        .children
        .iter()
        .find(|child| child.name == "SettingsHolder")?;
    let toggle = settings_holder
        .children
        .iter()
        .find(|child| child.name.starts_with("SettingToggle") && child.active_in_hierarchy)?
        .clone();
    let sub = settings_holder
        .children
        .iter()
        .find(|child| {
            child.name.starts_with("SubSettingsGroupPrefab") && child.active_in_hierarchy
        })?
        .clone();
    let sub_panel = sub
        .children
        .iter()
        .find(|child| child.name == "SubPanel")
        .cloned();
    // The slider prefab lives in the performance group.
    let slider = find(root, &|n: &Node| {
        n.name.starts_with("SettingSlider") && n.active_in_hierarchy
    })?
    .clone();
    let extras = EXTRAS
        .iter()
        .filter_map(|name| {
            find(root, &|n: &Node| {
                n.name == *name && n.active_in_hierarchy && n.native_rect.is_some()
            })
            .cloned()
        })
        .collect();
    Some(Templates {
        title,
        toggle,
        slider,
        sub,
        sub_panel,
        extras,
    })
}

impl Templates {
    /// Adds the list headers and placeholder lines this capture lacks from another one (the
    /// in-world panel was captured on the Simulation tab, the editor on every tab).
    #[must_use]
    pub fn with_extras_of(mut self, root: &Node) -> Self {
        for name in EXTRAS {
            if self.extras.iter().any(|extra| extra.name == name) {
                continue;
            }
            if let Some(found) = find(root, &|n: &Node| {
                n.name == name && n.active_in_hierarchy && n.native_rect.is_some()
            }) {
                self.extras.push(found.clone());
            }
        }
        self
    }

    /// The same prefabs with the group title and slider row of another captured list (the
    /// scenario editor's tabs without a scroll bar are 12 px wider than those with one).
    #[must_use]
    pub fn with_rows_of(mut self, root: &Node) -> Self {
        let group = find(root, &|n: &Node| {
            n.path.ends_with("/SettingGroupPrefab(Clone)") && n.active_in_hierarchy
        });
        if let Some(title) = group.and_then(|group| {
            group
                .children
                .iter()
                .find(|child| child.name == "Text (TMP)")
        }) {
            self.title = title.clone();
        }
        if let Some(slider) = find(root, &|n: &Node| {
            n.name.starts_with("SettingSlider") && n.active_in_hierarchy
        }) {
            self.slider = slider.clone();
        }
        self
    }

    fn extra(&self, name: &str) -> Option<&Node> {
        self.extras.iter().find(|node| node.name == name)
    }
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    input: &'a DynInput<'a>,
    templates: Templates,
    geometry: ListGeometry,
    hits: Vec<DynHit>,
    tips: Vec<DynTip>,
    /// Row nodes of the current tab in layout order (absolute positions, scroll applied).
    nodes: Vec<Node>,
    next_group: u16,
    /// Rows must lie inside `[view_top, view_bottom]` to be emitted.
    view_top: f32,
    view_bottom: f32,
    /// Measuring only: no nodes or hits are produced.
    dry: bool,
    /// The viewport's `Mask` cuts the rows that lie partly outside it (the in-game panel: its replay clips them), so those are emitted too.
    cuts: bool,
}

impl Ctx<'_> {
    fn visible(&self, y: f32, h: f32) -> bool {
        if self.cuts {
            !self.dry && y + h > self.view_top && y < self.view_bottom
        } else {
            !self.dry && y >= self.view_top && y + h <= self.view_bottom
        }
    }

    /// Appends a generated node; the replay draws siblings in `sibling_index` order, so the
    /// order of insertion is the z-order.
    fn add(&mut self, mut node: Node) {
        node.sibling_index = self.nodes.len() as i32;
        self.nodes.push(node);
    }
}

/// Lays out one row at `y` (y down, content coordinates already scrolled); returns its height.
fn layout_row(ctx: &mut Ctx<'_>, row: &Row, y: f32) -> f32 {
    match row {
        Row::Toggle(key) => {
            layout_toggle(ctx, key, y);
            ROW_H
        }
        Row::Slider(key, mode) => {
            layout_slider(ctx, key, *mode, y);
            ROW_H
        }
        Row::Sub {
            title,
            master,
            rows,
        } => layout_sub(ctx, title, master, rows, y),
        Row::Group { title, rows } | Row::FixedSum { title, rows, .. } => {
            layout_group(ctx, title, rows, y)
        }
        Row::Header {
            template,
            title,
            add,
            clear,
        } => layout_header(ctx, template, title, *add, *clear, y),
        // (The in-game Zones tab's `World Parameters` wrapper is 60 under its 310 high group; with the content's 15 on each side of the gap row.)
        Row::Gap(height) => {
            if in_game_zones_tab(ctx.input) && *height == 40.0 {
                45.0
            } else {
                *height
            }
        }
        Row::Material(kind) => layout_material(ctx, *kind, y),
        Row::Panel { template, buttons } => layout_panel(ctx, template, buttons, y),
        Row::Items {
            list,
            template,
            height,
        } => layout_items(ctx, *list, template, *height, y),
        Row::Heading(text) => layout_heading(ctx, text, y),
        Row::EmptyList {
            template,
            text,
            height,
        } => {
            layout_placeholder(ctx, template, text, y);
            *height
        }
    }
}

/// The font, size and row height of an in-game list header's title (read from the live game): the zone lists' are Roboto-Bold 36 in a 42.2 high row,
/// the default bibites' Boston Black 36 in 46.8, the settings animators' Roboto-Bold 40 in 46.9.
fn in_sim_header_style(title: &str) -> (&'static str, f32, f32) {
    match title {
        "Bibite Templates" => ("Boston Black SDF", 36.0, 46.8),
        "Settings Animator" => ("Roboto-Bold SDF NoOutline", 40.0, 46.9),
        _ => ("Roboto-Bold SDF NoOutline", 36.0, 42.2),
    }
}

/// The in-game panel's list header (`Zones Header` / `Group Header`, read from the live game): a 40 x 40 `+` button, the title in Roboto-Bold 36
/// centred between the buttons, and a 40 x 40 "remove all" button (its icon 50 x 50) in a 42.2 high row. "Remove all" is not interactable
/// (its frame greyed by the button's disabled tint) while its list is empty.
fn layout_header_in_sim(
    ctx: &mut Ctx<'_>,
    title: &str,
    add: UiAction,
    clear: UiAction,
    empty: bool,
    y: f32,
) -> f32 {
    const BUTTON: f32 = 40.0;
    let (font, size, height) = in_sim_header_style(title);
    if !ctx.visible(y, height) {
        return height;
    }
    let (x0, x1) = (ctx.geometry.content_x[0], ctx.geometry.content_x[1]);
    let top = y + (height - BUTTON) * 0.5;
    let root_path = format!("DynamicSettingsPanel/Header/{title}");
    let mut root = sprite_node(&root_path, "base_icon", [x0, y, x1, y + height], [1.0, 1.0, 1.0, 0.0]);
    root.images.clear();
    let child = |suffix: &str| format!("{root_path}/{suffix}");
    root.children.push(sprite_node(&child("Add"), "base_icon", [x0, top, x0 + BUTTON, top + BUTTON], [1.0; 4]));
    root.children.push(sprite_node(&child("Add/icon"), "plus_icon", [x0, top, x0 + BUTTON, top + BUTTON], [1.0; 4]));
    let mut name = text_node(&child("Title"), title, size, [x0 + BUTTON, y, x1 - BUTTON, y + height]);
    if let Some(text) = name.texts.first_mut() {
        text.font = Some(crate::ui_runtime::FontRef {
            name: font.to_owned(),
            ..crate::ui_runtime::FontRef::default()
        });
        text.alignment = "Center".to_owned();
        text.color = Some(crate::ui_runtime::Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        });
    }
    root.children.push(name);
    // The disabled `ColorBlock` (200, 200, 200, 128) tints the button's frame only; the icon keeps its colour.
    let frame_tint = if empty {
        [200.0 / 255.0, 200.0 / 255.0, 200.0 / 255.0, 128.0 / 255.0]
    } else {
        [1.0; 4]
    };
    root.children.push(sprite_node(&child("Remove All"), "base_icon", [x1 - BUTTON, top, x1, top + BUTTON], frame_tint));
    root.children.push(sprite_node(
        &child("Remove All/icon"),
        "exit_icon",
        [x1 - BUTTON - 5.0, top - 5.0, x1 + 5.0, top + BUTTON + 5.0],
        [1.0; 4],
    ));
    ctx.hits.push(DynHit {
        rect: [x0, top, BUTTON, BUTTON],
        action: add,
    });
    if !empty {
        ctx.hits.push(DynHit {
            rect: [x1 - BUTTON, top, BUTTON, BUTTON],
            action: clear,
        });
    }
    ctx.add(root);
    height
}

/// A list header: `+` button, title, "remove all" button.
fn layout_header(
    ctx: &mut Ctx<'_>,
    template: &str,
    title: &str,
    add: UiAction,
    clear: UiAction,
    y: f32,
) -> f32 {
    if let Some(lists) = ctx.input.lists.as_ref().filter(|lists| lists.in_sim) {
        let empty = match title {
            "Zones" => lists.zones.is_empty(),
            "Zone Groups" => lists.groups.is_empty(),
            // (The port keeps no settings animators.)
            "Settings Animator" => lists.changers.is_empty(),
            _ => lists.bibites.is_empty(),
        };
        return layout_header_in_sim(ctx, title, add, clear, empty, y);
    }
    let Some(source) = ctx.templates.extra(template) else {
        return 0.0;
    };
    let height = rect_of(source).map_or(0.0, |[_, y0, _, y1]| y1 - y0);
    if !ctx.visible(y, height) {
        return height;
    }
    let mut node = source.clone();
    let dy = y - top_of(&node);
    shift(&mut node, dy);
    fit_to_content(ctx, &mut node);
    if let Some(label) = find_mut(&mut node, &|n: &Node| n.name == "Title") {
        set_text(label, title);
    }
    for (name_matches, action) in [
        (
            &(|n: &Node| n.name.starts_with("Add")) as &dyn Fn(&Node) -> bool,
            add,
        ),
        (&|n: &Node| n.name == "Remove All", clear),
    ] {
        if let Some(button) = find(&node, name_matches)
            && let Some([x0, y0, x1, y1]) = rect_of(button)
        {
            ctx.hits.push(DynHit {
                rect: [x0, y0, x1 - x0, y1 - y0],
                action,
            });
        }
    }
    ctx.add(node);
    height
}

/// Moves a captured node that spans `old` (x0, x1) so it fits a content area `new`: parts left
/// of the centre keep their distance to the left edge, parts right of it their distance to the
/// right edge, wide parts stretch and centred ones stay centred.
fn fit_x(node: &mut Node, old: [f32; 2], new: [f32; 2]) {
    let old_w = (old[1] - old[0]).max(1.0);
    let old_centre = (old[0] + old[1]) * 0.5;
    let new_centre = (new[0] + new[1]) * 0.5;
    fn walk(node: &mut Node, old: [f32; 2], new: [f32; 2], old_w: f32, oc: f32, nc: f32) {
        if let Some(rect) = node.native_rect.as_mut() {
            let (x0, x1) = (rect[0], rect[2]);
            let centre = (x0 + x1) * 0.5;
            if x1 - x0 > 0.6 * old_w {
                // Stretches with the area.
                rect[0] = x0 + (new[0] - old[0]);
                rect[2] = x1 + (new[1] - old[1]);
            } else {
                let dx = if centre < oc - 0.15 * old_w {
                    new[0] - old[0]
                } else if centre > oc + 0.15 * old_w {
                    new[1] - old[1]
                } else {
                    nc - oc
                };
                rect[0] = x0 + dx;
                rect[2] = x1 + dx;
            }
        }
        for child in &mut node.children {
            walk(child, old, new, old_w, oc, nc);
        }
    }
    walk(node, old, new, old_w, old_centre, new_centre);
}

/// Re-seats a captured header or heading in the list's content area.
fn fit_to_content(ctx: &Ctx<'_>, node: &mut Node) {
    let Some([x0, _, x1, _]) = rect_of(node) else {
        return;
    };
    let [n0, n1] = ctx.geometry.content_x;
    if (x0 - n0).abs() < 0.5 && (x1 - n1).abs() < 0.5 {
        return;
    }
    fit_x(node, [x0, x1], [n0, n1]);
}

/// A generated sprite node (the zone items have no captured geometry): `sprite` drawn over
/// `rect` (x0, y0, x1, y1; reference px, y down).
fn sprite_node(path: &str, sprite: &str, rect: [f32; 4], tint: [f32; 4]) -> Node {
    use crate::ui_runtime::{Border, Color, ImageComponent, SpriteRef};
    // The sliced sprites of the panels (their border and pixels per unit, read from the live game): `base_icon` 5 at 100, the 32 px frames 10 at 200.
    let sliced = match sprite {
        "base_icon" => Some((5.0, 100.0)),
        // (`base_icon_thin`: the point panel's delete button; a 3 px border at 100 per unit, from the live game's dump.)
        "base_icon_thin" => Some((3.0, 100.0)),
        "UISprite" | "Background" | "InputFieldBackground" => Some((10.0, 200.0)),
        // (`Button_UI`: the popup panels' and buttons' frame, a 30 px border; the panels draw it at 1 / 8.)
        "Button_UI" => Some((30.0, 100.0)),
        _ => None,
    };
    let mut node = Node {
        name: path.rsplit('/').next().unwrap_or(path).to_owned(),
        path: path.to_owned(),
        active: true,
        active_in_hierarchy: true,
        images: vec![ImageComponent {
            sprite: Some(SpriteRef {
                name: sprite.to_owned(),
                ..SpriteRef::default()
            }),
            color: Some(Color {
                r: tint[0],
                g: tint[1],
                b: tint[2],
                a: tint[3],
            }),
            image_type: if sliced.is_some() { "Sliced" } else { "Simple" }.to_owned(),
            runtime_sprite_border: sliced.map(|(border, _)| Border {
                x: border,
                y: border,
                z: border,
                w: border,
            }),
            sprite_ppu: Some(sliced.map_or(100.0, |(_, ppu)| ppu)),
            ..ImageComponent::default()
        }],
        ..Node::default()
    };
    set_rect(&mut node, rect[0], rect[1], rect[2], rect[3]);
    node
}

/// A generated text node.
fn text_node(path: &str, text: &str, size: f32, rect: [f32; 4]) -> Node {
    use crate::ui_runtime::{Color, FontRef, TextComponent};
    let mut node = Node {
        name: path.rsplit('/').next().unwrap_or(path).to_owned(),
        path: path.to_owned(),
        active: true,
        active_in_hierarchy: true,
        texts: vec![TextComponent {
            text: text.to_owned(),
            font: Some(FontRef {
                name: "Roboto-Bold SDF NoOutline".to_owned(),
                ..FontRef::default()
            }),
            font_size: size,
            alignment: "Left".to_owned(),
            color: Some(Color {
                r: 0.1647,
                g: 0.1647,
                b: 0.1647,
                a: 1.0,
            }),
            ..TextComponent::default()
        }],
        ..Node::default()
    };
    set_rect(&mut node, rect[0], rect[1], rect[2], rect[3]);
    node
}

/// The items of a zone list (the game's `ZoneHandle`): a light box with the name header, a delete
/// button and an open bar; an open zone shows its controls (stacked prefab rows) below the header.
fn layout_items(ctx: &mut Ctx<'_>, list: u8, template: &str, min_height: f32, y: f32) -> f32 {
    let Some(lists) = ctx.input.lists else {
        layout_placeholder(ctx, template, "", y);
        return min_height;
    };
    if list == 2 {
        return layout_bibite_items(ctx, &lists, min_height, y);
    }
    if list == 1 {
        if lists.groups.is_empty() {
            layout_placeholder(ctx, template, "", y);
            return min_height;
        }
        return zone_cards_ui::layout_groups(ctx, &lists, min_height, y);
    }
    if list == 3 {
        if lists.changers.is_empty() {
            layout_placeholder(ctx, template, "", y);
            return min_height;
        }
        return animator_cards_ui::layout_animators(ctx, &lists, min_height, y);
    }
    let count = if list == 0 {
        lists.zones.len()
    } else {
        lists.groups.len()
    };
    if count == 0 {
        layout_placeholder(ctx, template, "", y);
        return min_height;
    }
    // The in-game card (`ZoneHandle`, read from the live game) is 38.5 + 20 high and its header is the zone's name only (10 in from the edge,
    // 8 down); the scenario editor's has a "Name" caption and a 48 high header. The list ends with 60 of padding in the game.
    let in_sim = lists.in_sim;
    let header_h: f32 = if in_sim { 38.5 } else { 48.0 };
    const BAR_H: f32 = 20.0;
    const GAP: f32 = 10.0;
    // The cards of the game's list are 5 apart (`Zone List`: spacing 5).
    let card_gap = if in_sim { 5.0 } else { GAP };
    let (x0, x1) = (ctx.geometry.content_x[0], ctx.geometry.content_x[1]);
    let mut cursor = y;
    for index in card_order(lists.order, list, count) {
        let name = lists.zones[index].name.clone();
        let open = lists.open.contains(&(list, index as u16));
        // In the game an open zone is the two-column card (`Zone List` keeps its cards 5 apart).
        if open && in_sim {
            let height = zone_cards_ui::paint_open_zone(ctx, &lists, index, cursor);
            cursor += height + card_gap;
            continue;
        }
        let fields = if open {
            crate::zone_editor::fields_of(&lists.zones[index])
        } else {
            Vec::new()
        };
        let rows_h = if fields.is_empty() {
            0.0
        } else {
            fields.len() as f32 * (ROW_H + GAP) + GAP
        };
        let height = header_h + rows_h + BAR_H;
        let (view_top, view_bottom) = (ctx.view_top, ctx.view_bottom);
        if !ctx.dry && cursor + height > view_top && cursor < view_bottom {
            // (The prefab's own path from the last `(Clone)` on: the Image multipliers are keyed by it, the card's is 0.75.)
            let root_path = format!("UI/ScenarioEditor/Items/{list}/Item{index}/ZoneHandle(Clone)");
            // The box is clipped to the viewport (the game's scroll view masks it).
            let top = cursor.max(view_top);
            let bottom = (cursor + height).min(view_bottom);
            let mut root = sprite_node(&root_path, "base_icon", [x0, top, x1, bottom], [1.0; 4]);
            let child = |suffix: &str| format!("{root_path}/{suffix}");
            let name_x = if in_sim { x0 + 10.0 } else { x0 + 200.0 };
            let header_visible = cursor >= view_top && cursor + header_h <= view_bottom;
            if header_visible {
                if !in_sim {
                    root.children.push(text_node(
                        &child("label"),
                        "Name",
                        26.0,
                        [x0 + 10.0, cursor + 8.0, x0 + 180.0, cursor + 40.0],
                    ));
                }
                // The name line is a text field (`TextLineHandle`): typing shows a caret.
                let editing = lists
                    .name_edit
                    .filter(|(edited, _)| list == 0 && usize::from(*edited) == index)
                    .map(|(_, text)| text);
                root.children.push(text_node(
                    &child("name"),
                    &editing.map_or_else(|| name.clone(), crate::text_field::with_caret),
                    26.0,
                    [name_x, cursor + 8.0, x1 - 60.0, cursor + 40.0],
                ));
                // `ZoneSettingsHandle.SelectItem` shows the delete button, `CloseEditSection`
                // hides it again (a zone group's header always has its own).
                if open || list != 0 {
                    root.children.push(sprite_node(
                        &child("delete"),
                        "base_icon",
                        [x1 - 45.0, cursor + 6.0, x1 - 15.0, cursor + 36.0],
                        [1.0; 4],
                    ));
                    root.children.push(sprite_node(
                        &child("delete/icon"),
                        "exit_icon",
                        [x1 - 43.0, cursor + 8.0, x1 - 17.0, cursor + 34.0],
                        [1.0; 4],
                    ));
                    ctx.hits.push(DynHit {
                        rect: [x1 - 45.0, cursor + 6.0, 30.0, 30.0],
                        action: (lists.delete_item)(list, index as u16),
                    });
                }
            }
            // Clicking the bar (or the header of a closed item) opens / closes it.
            let toggle_open = UiAction::ZoneOpen(list, index as u16);
            let bar_top = cursor + height - BAR_H;
            if bar_top >= view_top && cursor + height <= view_bottom {
                root.children.push(sprite_node(
                    &child("open"),
                    "base_icon",
                    [x0, bar_top, x1, cursor + height],
                    [1.0; 4],
                ));
                // `OpenButton/icon`: the `Arrow` sprite, 35 x 17, centred and 1.5 under the bar's top, turned half a turn while the item is closed.
                let centre = (x0 + x1) * 0.5;
                let mut arrow = sprite_node(
                    &child("OpenButton/icon"),
                    "Arrow",
                    [centre - 17.5, bar_top + 1.5, centre + 17.5, bar_top + 18.5],
                    [1.0; 4],
                );
                if !open {
                    arrow.rect.local_rotation = crate::ui_runtime::Quaternion { x: 0.0, y: 0.0, z: 1.0, w: 0.0 };
                }
                root.children.push(arrow);
                ctx.hits.push(DynHit {
                    rect: [x0, bar_top, x1 - x0, BAR_H],
                    action: toggle_open,
                });
            }
            if !open && header_visible {
                ctx.hits.push(DynHit {
                    rect: [x0, cursor, x1 - x0 - 60.0, header_h],
                    action: toggle_open,
                });
            }
            // Clicking the name starts typing a new one (`TextLineHandle.openEdit`); the zones only.
            if list == 0 && header_visible {
                ctx.hits.push(DynHit {
                    rect: [name_x, cursor + 8.0, x1 - 60.0 - name_x, 32.0],
                    action: UiAction::ZoneNameEdit(index as u16),
                });
            }
            ctx.add(root);
            // The controls of an open zone.
            let mut row_y = cursor + header_h + GAP;
            for field in fields {
                let zone = &lists.zones[index];
                let reference = FieldRef::Zone(index as u16, field);
                match crate::zone_editor::kind_of(field) {
                    crate::zone_editor::FieldKind::Slider => {
                        if let Some(info) =
                            crate::zone_editor::slider_info(zone, field, lists.sim_size)
                        {
                            let value = crate::zone_editor::value_of(zone, field);
                            let fraction =
                                fraction_range(info.mode, info.min, info.max, info.default, value);
                            let default_fraction = fraction_range(
                                info.mode,
                                info.min,
                                info.max,
                                info.default,
                                info.default,
                            );
                            let spec = SliderSpec {
                                name: crate::zone_editor::name_of(field),
                                text: info.format().format(value),
                                fraction,
                                default_fraction,
                                field: reference,
                                format: info.format(),
                                rules: info_rules(&info, false),
                            };
                            emit_slider(ctx, &spec, row_y, true);
                        }
                    }
                    crate::zone_editor::FieldKind::Toggle => emit_toggle(
                        ctx,
                        crate::zone_editor::name_of(field),
                        crate::zone_editor::toggle_value(zone, field),
                        reference,
                        row_y,
                        true,
                    ),
                    crate::zone_editor::FieldKind::Choice => emit_choice(
                        ctx,
                        crate::zone_editor::name_of(field),
                        &crate::zone_editor::choice_label(zone, field, lists.all_zones),
                        UiAction::DropdownOpen(crate::ui::DropdownTarget::Zone(
                            index as u16,
                            field,
                        )),
                        reference,
                        row_y,
                        true,
                    ),
                }
                row_y += ROW_H + GAP;
            }
        }
        cursor += height + card_gap;
    }
    let used = cursor - card_gap - y;
    if in_sim {
        // `Zone List` / `Group List` end with 60 of padding.
        (used + 60.0).max(min_height)
    } else {
        used.max(min_height)
    }
}

/// The default bibites (`DefaultBibitesPanel`): one box per spawn row that opens into the
/// `BibiteSettingsHandle` controls (the prefab's two columns are stacked in one, like the zone
/// items). The empty list takes no room of its own.
fn layout_bibite_items(ctx: &mut Ctx<'_>, lists: &ListsInput<'_>, min_height: f32, y: f32) -> f32 {
    if lists.in_sim {
        return layout_bibite_cards_in_sim(ctx, lists, min_height, y);
    }
    let count = lists.bibites.len();
    if count == 0 {
        return min_height;
    }
    const HEADER_H: f32 = 48.0;
    const BAR_H: f32 = 20.0;
    const GAP: f32 = 10.0;
    let list = 2_u8;
    let (x0, x1) = (ctx.geometry.content_x[0], ctx.geometry.content_x[1]);
    let mut cursor = y;
    for index in 0..count {
        let row = &lists.bibites[index];
        let name = lists
            .bibite_names
            .get(index)
            .cloned()
            .unwrap_or_else(|| row.path.clone());
        let open = lists.open.contains(&(list, index as u16));
        let fields = if open {
            crate::bibite_editor::fields_of(row, lists.in_sim)
        } else {
            Vec::new()
        };
        let rows_h = if fields.is_empty() {
            0.0
        } else {
            fields.len() as f32 * (ROW_H + GAP) + GAP
        };
        let height = HEADER_H + rows_h + BAR_H;
        let (view_top, view_bottom) = (ctx.view_top, ctx.view_bottom);
        if !ctx.dry && cursor + height > view_top && cursor < view_bottom {
            let root_path = format!("UI/ScenarioEditor/Items/{list}/BibiteHandle(Clone)");
            let top = cursor.max(view_top);
            let bottom = (cursor + height).min(view_bottom);
            let mut root = sprite_node(&root_path, "base_icon", [x0, top, x1, bottom], [1.0; 4]);
            root.path = format!("{root_path}#{index}");
            let child = |suffix: &str| format!("{root_path}#{index}/{suffix}");
            let header_visible = cursor >= view_top && cursor + HEADER_H <= view_bottom;
            if header_visible {
                root.children.push(text_node(
                    &child("name"),
                    &name,
                    26.0,
                    [x0 + 20.0, cursor + 8.0, x1 - 60.0, cursor + 40.0],
                ));
                // `SelectItem` shows the delete button of the selected item.
                if open {
                    root.children.push(sprite_node(
                        &child("delete"),
                        "base_icon",
                        [x1 - 45.0, cursor + 6.0, x1 - 15.0, cursor + 36.0],
                        [1.0; 4],
                    ));
                    root.children.push(sprite_node(
                        &child("delete/icon"),
                        "exit_icon",
                        [x1 - 43.0, cursor + 8.0, x1 - 17.0, cursor + 34.0],
                        [1.0; 4],
                    ));
                    ctx.hits.push(DynHit {
                        rect: [x1 - 45.0, cursor + 6.0, 30.0, 30.0],
                        action: (lists.delete_item)(list, index as u16),
                    });
                }
            }
            // Clicking the bar (or the header of a closed item) opens / closes it.
            let toggle_open = UiAction::ZoneOpen(list, index as u16);
            let bar_top = cursor + height - BAR_H;
            if bar_top >= view_top && cursor + height <= view_bottom {
                root.children.push(sprite_node(
                    &child("open"),
                    "base_icon",
                    [x0, bar_top, x1, cursor + height],
                    [1.0; 4],
                ));
                ctx.hits.push(DynHit {
                    rect: [x0, bar_top, x1 - x0, BAR_H],
                    action: toggle_open,
                });
            }
            if !open && header_visible {
                ctx.hits.push(DynHit {
                    rect: [x0, cursor, x1 - x0 - 60.0, HEADER_H],
                    action: toggle_open,
                });
            }
            ctx.add(root);
            // The controls of an open item.
            let mut row_y = cursor + HEADER_H + GAP;
            for field in fields {
                let reference = FieldRef::Bibite(index as u16, field);
                let label = crate::bibite_editor::name_of(field);
                match crate::bibite_editor::kind_of(field) {
                    crate::bibite_editor::Kind::Slider => {
                        if let Some(info) = crate::bibite_editor::slider_info(field) {
                            let value = crate::bibite_editor::value_of(row, field);
                            let spec = SliderSpec {
                                name: label,
                                text: info.format().format(value),
                                fraction: fraction_range(
                                    info.mode,
                                    info.min,
                                    info.max,
                                    info.default,
                                    value,
                                ),
                                default_fraction: fraction_range(
                                    info.mode,
                                    info.min,
                                    info.max,
                                    info.default,
                                    info.default,
                                ),
                                field: reference,
                                format: info.format(),
                                rules: info_rules(&info, field == crate::bibite_editor::MINIMUM),
                            };
                            emit_slider(ctx, &spec, row_y, true);
                        }
                    }
                    crate::bibite_editor::Kind::Choice => emit_choice(
                        ctx,
                        label,
                        &crate::bibite_editor::choice_label(row, field, lists.all_zones),
                        UiAction::DropdownOpen(crate::ui::DropdownTarget::Bibite(
                            index as u16,
                            field,
                        )),
                        reference,
                        row_y,
                        true,
                    ),
                    // Shown, not typed (the species / custom tag has no text field here yet).
                    crate::bibite_editor::Kind::Text => emit_info(
                        ctx,
                        label,
                        &crate::bibite_editor::text_of(row, field),
                        row_y,
                    ),
                }
                row_y += ROW_H + GAP;
            }
        }
        cursor += height + GAP;
    }
    (cursor - GAP - y).max(min_height)
}

/// The prefab's name of the row of a field of a card.
fn card_row_name(field: u8) -> &'static str {
    match field {
        crate::bibite_editor::TAGGING => "tagging",
        crate::bibite_editor::SPAWN_ZONE => "spawnZone",
        crate::bibite_editor::PRIORITY => "priority",
        crate::bibite_editor::GROWTH => "growth",
        crate::bibite_editor::CUSTOM_TAG => "customTag",
        crate::bibite_editor::SPAWN_TYPE => "spawnType",
        crate::bibite_editor::MINIMUM => "minimum",
        _ => "mutations",
    }
}

/// A text node of a card: `alignment`, a grey and a size.
fn card_text(path: &str, text: &str, size: f32, rect: [f32; 4], alignment: &str, gray: f32) -> Node {
    let mut node = text_node(path, text, size, rect);
    for component in &mut node.texts {
        component.alignment = alignment.to_owned();
        component.color = Some(crate::ui_runtime::Color {
            r: gray,
            g: gray,
            b: gray,
            a: 1.0,
        });
    }
    node
}

/// One dropdown row of an open card (read from the live game): the label (20) and a 202 x 30 `UISprite` dropdown with its choice (16) and arrow.
fn card_dropdown_row(
    root: &mut Node,
    hits: &mut Vec<DynHit>,
    base: &str,
    (x, y, col_w): (f32, f32, f32),
    label: &str,
    choice: &str,
    action: UiAction,
) {
    let name_w = col_w - 15.0 - 202.1;
    root.children.push(card_text(&format!("{base}/Name"), label, 20.0, [x + 10.0, y, x + name_w, y + 30.0], "Left", 0.164_705_88));
    let dx = x + name_w + 15.0;
    root.children.push(sprite_node(&format!("{base}/Dropdown"), "UISprite", [dx, y, dx + 202.1, y + 30.0], [1.0; 4]));
    root.children.push(card_text(
        &format!("{base}/Dropdown/Label"),
        choice,
        16.0,
        [dx + 10.0, y + 6.5, dx + 177.1, y + 23.5],
        "Left",
        0.196_078_43,
    ));
    root.children.push(sprite_node(&format!("{base}/Dropdown/Arrow"), "DropdownArrow", [dx + 172.1, y, dx + 202.1, y + 30.0], [1.0; 4]));
    hits.push(DynHit {
        rect: [dx, y, 202.1, 30.0],
        action,
    });
}

/// Where the parts of a card's slider row sit, from the row's left edge: the label's width, the value text (offset, align) and the slider panel
/// (offset, width). The prefab's layout group gives the three rows different shares (read from the live game, with and without the spawned count).
struct CardSliderGeometry {
    name_w: f32,
    value_x: f32,
    /// Width of the value text's box.
    value_w: f32,
    value_align: &'static str,
    panel_x: f32,
    panel_w: f32,
}

/// One slider row of an open card: the label (20), the value (16) and a 20 high slider (a `Background` track, a white `UISprite` fill, a 20 x 20
/// `base_circle_icon` handle), which a click on its value turns into an input field.
fn card_slider_row(
    root: &mut Node,
    hits: &mut Vec<DynHit>,
    edit: Option<&ValueEdit>,
    base: &str,
    (x, y): (f32, f32),
    geometry: &CardSliderGeometry,
    spec: &SliderSpec<'_>,
) {
    let CardSliderGeometry {
        name_w,
        value_x,
        value_w,
        value_align,
        panel_x,
        panel_w,
    } = *geometry;
    root.children.push(card_text(&format!("{base}/Name"), spec.name, 20.0, [x + 10.0, y, x + name_w, y + 30.0], "Left", 0.164_705_88));
    let px = x + panel_x;
    if let Some(edit) = edit {
        // `OpenEditPanel`: the input field takes the slider's place (and the value text goes).
        let rect = [px, y, px + panel_w, y + 30.0];
        let path = format!("{base}/EditSection/InputField (TMP)");
        root.children.push(field_image(&path, Some("InputFieldBackground"), rect, [1.0; 4]));
        let area = [rect[0] + 12.0, rect[1] + 6.5, rect[2] - 10.0, rect[3] - 6.5];
        let ink = FIELD_INK;
        let before = |chars: usize| value_edit::text_width(&edit.text.chars().take(chars).collect::<String>(), 16.0);
        if let Some((from, to)) = edit.caret.selection() {
            root.children.push(field_image(
                &format!("{path}/Text Area/Selection"),
                None,
                [area[0] + before(from), area[1], area[0] + before(to), area[3]],
                [0.658_823_5, 0.807_843_2, 1.0, 0.752_941_2],
            ));
        }
        if edit.text.is_empty() {
            let mut hint = field_text(&format!("{path}/Text Area/Placeholder"), "Enter value...", area, [ink[0], ink[1], ink[2], 0.5]);
            for component in &mut hint.texts {
                component.font_size = 16.0;
            }
            root.children.push(hint);
        } else {
            let mut typed = field_text(&format!("{path}/Text Area/Text"), &edit.text, area, [ink[0], ink[1], ink[2], 1.0]);
            for component in &mut typed.texts {
                component.font_size = 16.0;
            }
            root.children.push(typed);
        }
        if edit.caret_visible() {
            let at = area[0] + before(edit.caret.pos);
            root.children.push(field_image(
                &format!("{path}/Text Area/Caret"),
                None,
                [at, area[1], at + 1.0, area[3]],
                [ink[0], ink[1], ink[2], 1.0],
            ));
        }
        hits.push(DynHit {
            rect: [rect[0], rect[1], rect[2] - rect[0], rect[3] - rect[1]],
            action: UiAction::ValueEditInside,
        });
        return;
    }
    let value_rect = [x + value_x, y + 5.0, x + value_x + value_w, y + 25.0];
    root.children.push(card_text(&format!("{base}/FloatInfoText"), &spec.text, 16.0, value_rect, value_align, 0.164_705_88));
    hits.push(DynHit {
        rect: [value_rect[0], value_rect[1], value_rect[2] - value_rect[0], value_rect[3] - value_rect[1]],
        action: UiAction::DynEditValue(spec.field),
    });
    let (track_x0, track_x1) = (px + 10.0, px + panel_w - 10.0);
    let centre = track_x0 + spec.fraction * (track_x1 - track_x0);
    root.children.push(sprite_node(
        &format!("{base}/NormalSecton/SliderPanel/Slider/Background"),
        "Background",
        [track_x0, y + 5.0, track_x1 + 5.0, y + 25.0],
        [1.0, 1.0, 1.0, 0.254_901_97],
    ));
    root.children.push(sprite_node(
        &format!("{base}/NormalSecton/SliderPanel/Slider/Fill Area/Fill"),
        "UISprite",
        [track_x0 - 5.0, y + 5.0, centre + 5.0 + 5.0 * spec.fraction, y + 25.0],
        [1.0; 4],
    ));
    root.children.push(sprite_node(
        &format!("{base}/NormalSecton/SliderPanel/Slider/Handle Slide Area/Handle"),
        "base_circle_icon",
        [centre - 10.0, y + 5.0, centre + 10.0, y + 25.0],
        [1.0; 4],
    ));
    hits.push(DynHit {
        rect: [px, y, panel_w, 30.0],
        action: spec.field.slide(track_x0.round() as u16, track_x1.round() as u16),
    });
}

/// The default bibites of the in-game panel (`BibiteSettingsHandle`, read from the live game): one card per spawn row, 5 apart. A card shows the
/// template's name; the selected one also a delete button and a two-column grid of its settings, each a 30 high row of a label and a dropdown or a
/// slider (left: tagging, spawn zone, priority, growth at spawn; right, bottom aligned: the custom tag while tagging is custom, spawn logic, target
/// count, mutate genes). One card is open at a time (`DefaultBibitesPanel.ItemClicked`: a click on a closed card selects it; the first one is
/// selected when the panel is filled): the open card is 206 high, a closed one 50.5.
fn layout_bibite_cards_in_sim(ctx: &mut Ctx<'_>, lists: &ListsInput<'_>, min_height: f32, y: f32) -> f32 {
    const PAD: f32 = 10.0;
    const OPEN_H: f32 = 206.0;
    const CLOSED_H: f32 = 50.5;
    const CARD_GAP: f32 = 5.0;
    const ROW_STEP: f32 = 37.0;
    let count = lists.bibites.len();
    if count == 0 {
        return min_height;
    }
    let list = 2_u8;
    let open_index = lists
        .open
        .iter()
        .find(|(kind, index)| *kind == list && usize::from(*index) < count)
        .map_or(0, |(_, index)| usize::from(*index));
    let (x0, x1) = (ctx.geometry.content_x[0], ctx.geometry.content_x[1]);
    let mut cursor = y;
    for index in 0..count {
        let open = index == open_index;
        let height = if open { OPEN_H } else { CLOSED_H };
        if ctx.visible(cursor, height) {
            let row = &lists.bibites[index];
            let name = lists
                .bibite_names
                .get(index)
                .cloned()
                .unwrap_or_else(|| row.path.clone());
            let root_path = format!("DynamicSettingsPanel/Bibites/Card{index}/BibiteSettingsHandle(Clone)");
            let mut root = sprite_node(&root_path, "base_icon", [x0, cursor, x1, cursor + height], [1.0; 4]);
            let child = |suffix: &str| format!("{root_path}/{suffix}");
            // The header row is 35 high with the delete button, the name's own 30.5 without it.
            let header_h = if open { 35.0 } else { 30.5 };
            let name_top = cursor + PAD + (header_h - 30.5) * 0.5;
            root.children.push(text_node(
                &child("Header/name"),
                &name,
                26.0,
                [x0 + PAD, name_top, x1 - PAD - 55.0, name_top + 30.5],
            ));
            if !open {
                ctx.hits.push(DynHit {
                    rect: [x0, cursor, x1 - x0, height],
                    action: UiAction::ZoneOpen(list, index as u16),
                });
            } else {
                let (bx, by) = (x1 - PAD - 35.0, cursor + PAD);
                root.children.push(sprite_node(&child("Header/DeleteButton"), "base_icon", [bx, by, bx + 35.0, by + 35.0], [1.0; 4]));
                root.children.push(sprite_node(&child("Header/DeleteButton/Icon"), "Delete_0", [bx, by, bx + 35.0, by + 35.0], [1.0; 4]));
                ctx.hits.push(DynHit {
                    rect: [bx, by, 35.0, 35.0],
                    action: (lists.delete_item)(list, index as u16),
                });
                let col_w = ((x1 - x0) - 2.0 * PAD - 20.0) * 0.5;
                let left_x = x0 + PAD;
                let right_x = left_x + col_w + 20.0;
                // `Edit Section` starts under the header: card padding 10 + header 35 + the section's own 10.
                let grid_top = cursor + PAD + 35.0 + 10.0;
                let fields = crate::bibite_editor::fields_of(row, lists.in_sim);
                let has = |field: u8| fields.contains(&field);
                let left = [
                    crate::bibite_editor::TAGGING,
                    crate::bibite_editor::SPAWN_ZONE,
                    crate::bibite_editor::PRIORITY,
                    crate::bibite_editor::GROWTH,
                ];
                let mut right = Vec::new();
                if has(crate::bibite_editor::CUSTOM_TAG) {
                    right.push(crate::bibite_editor::CUSTOM_TAG);
                }
                right.extend([
                    crate::bibite_editor::SPAWN_TYPE,
                    crate::bibite_editor::MINIMUM,
                    crate::bibite_editor::MUTATIONS,
                ]);
                // The right column sits at the bottom of the left one's height.
                let right_top = grid_top + (4 - right.len()) as f32 * ROW_STEP;
                let spawned = has(crate::bibite_editor::SPAWNED);
                for (column_x, column_top, column) in [(left_x, grid_top, &left[..]), (right_x, right_top, &right[..])] {
                    for (slot, field) in column.iter().copied().enumerate() {
                        let row_y = column_top + slot as f32 * ROW_STEP;
                        let reference = FieldRef::Bibite(index as u16, field);
                        let label = crate::bibite_editor::name_of(field);
                        let side = if column_x == left_x { "Left Row" } else { "Right Row" };
                        let base = child(&format!("Edit Section/Two Rows/{side}/{}", card_row_name(field)));
                        if let Some(tip) = crate::bibite_editor::help_key(field).and_then(|key| crate::tooltip::of_item("bibite", key)) {
                            ctx.tips.push(DynTip {
                                rect: [column_x, row_y, col_w - 15.0 - 202.1, 30.0],
                                tip,
                            });
                        }
                        match crate::bibite_editor::kind_of(field) {
                            crate::bibite_editor::Kind::Choice => card_dropdown_row(
                                &mut root,
                                &mut ctx.hits,
                                &base,
                                (column_x, row_y, col_w),
                                label,
                                &crate::bibite_editor::choice_label(row, field, lists.all_zones),
                                UiAction::DropdownOpen(crate::ui::DropdownTarget::Bibite(index as u16, field)),
                            ),
                            crate::bibite_editor::Kind::Slider => {
                                let Some(info) = crate::bibite_editor::slider_info(field) else {
                                    continue;
                                };
                                let value = crate::bibite_editor::value_of(row, field);
                                let spec = SliderSpec {
                                    name: label,
                                    text: info.format().format(value),
                                    fraction: fraction_range(info.mode, info.min, info.max, info.default, value),
                                    default_fraction: fraction_range(info.mode, info.min, info.max, info.default, info.default),
                                    field: reference,
                                    format: info.format(),
                                    rules: info_rules(&info, field == crate::bibite_editor::MINIMUM),
                                };
                                let geometry = if field == crate::bibite_editor::PRIORITY {
                                    CardSliderGeometry {
                                        name_w: 125.0,
                                        value_x: 144.2,
                                        value_w: 82.1,
                                        value_align: "Center",
                                        panel_x: 241.4,
                                        panel_w: 202.1,
                                    }
                                } else if spawned {
                                    CardSliderGeometry {
                                        name_w: 137.2,
                                        value_x: 204.0,
                                        value_w: 40.0,
                                        value_align: "Left",
                                        panel_x: 237.4,
                                        panel_w: 206.1,
                                    }
                                } else {
                                    CardSliderGeometry {
                                        name_w: 171.8,
                                        value_x: 186.8,
                                        value_w: 40.0,
                                        value_align: "Left",
                                        panel_x: 220.1,
                                        panel_w: 223.4,
                                    }
                                };
                                let editing = ctx.input.edit.filter(|edit| edit.field == reference);
                                card_slider_row(&mut root, &mut ctx.hits, editing, &base, (column_x, row_y), &geometry, &spec);
                                if field == crate::bibite_editor::MINIMUM && spawned && editing.is_none() {
                                    // `SpawnedSection`: how many this row has spawned, its 12 x 12 reset button and the "/" before the target.
                                    let n = crate::bibite_editor::text_of(row, crate::bibite_editor::SPAWNED);
                                    root.children.push(card_text(
                                        &format!("{base}/SpawnedSection/FloatInfoText"),
                                        &n,
                                        16.0,
                                        [column_x + 152.2, row_y + 5.6, column_x + 170.6, row_y + 24.4],
                                        "Center",
                                        0.164_705_88,
                                    ));
                                    root.children.push(sprite_node(
                                        &format!("{base}/SpawnedSection/Button"),
                                        "base_icon",
                                        [column_x + 175.6, row_y + 9.0, column_x + 187.6, row_y + 21.0],
                                        [1.0; 4],
                                    ));
                                    root.children.push(sprite_node(
                                        &format!("{base}/SpawnedSection/Button/icon"),
                                        "reset_icon",
                                        [column_x + 175.6, row_y + 9.0, column_x + 187.6, row_y + 21.0],
                                        [1.0; 4],
                                    ));
                                    root.children.push(card_text(
                                        &format!("{base}/SpawnedSection/Slash"),
                                        "/",
                                        16.0,
                                        [column_x + 192.6, row_y + 5.6, column_x + 199.1, row_y + 24.4],
                                        "Center",
                                        0.164_705_88,
                                    ));
                                }
                            }
                            crate::bibite_editor::Kind::Text => {
                                if field == crate::bibite_editor::CUSTOM_TAG {
                                    // `customTag`: the tag's input field, 10 in from the column's left edge.
                                    let rect = [column_x + 10.0, row_y, column_x + col_w, row_y + 30.0];
                                    root.children.push(sprite_node(&format!("{base}/renameField"), "InputFieldBackground", rect, [1.0; 4]));
                                    let mut tag = text_node(
                                        &format!("{base}/renameField/Text Area/Text"),
                                        &crate::bibite_editor::text_of(row, field),
                                        20.0,
                                        [rect[0] + 10.0, row_y + 7.0, rect[2] - 10.0, row_y + 24.0],
                                    );
                                    for component in &mut tag.texts {
                                        component.font = Some(crate::ui_runtime::FontRef {
                                            name: "Roboto-Regular SDF".to_owned(),
                                            ..crate::ui_runtime::FontRef::default()
                                        });
                                        component.color = Some(crate::ui_runtime::Color {
                                            r: FIELD_INK[0],
                                            g: FIELD_INK[1],
                                            b: FIELD_INK[2],
                                            a: 1.0,
                                        });
                                    }
                                    root.children.push(tag);
                                }
                                // (The spawned count is drawn with the target count's row.)
                            }
                        }
                    }
                }
            }
            ctx.add(root);
        }
        cursor += height + CARD_GAP;
    }
    (cursor - CARD_GAP - y).max(min_height)
}

/// A row that only shows a value (the label and the text, no control).
fn emit_info(ctx: &mut Ctx<'_>, name_text: &str, value: &str, y: f32) {
    if !ctx.visible(y, ROW_H) {
        return;
    }
    let mut node = ctx.templates.toggle.clone();
    let dy = y - top_of(&node);
    shift(&mut node, dy);
    if let Some(name) = find_mut(&mut node, &|n: &Node| n.name == "Name") {
        set_label(name, name_text);
    }
    let mut box_rect = [0.0_f32; 4];
    if let Some(toggle) = find_mut(&mut node, &|n: &Node| n.name == "Toggle") {
        if let Some([x0, y0, x1, y1]) = rect_of(toggle) {
            box_rect = [x0, y0, x1, y1];
        }
        set_active(toggle, false);
    }
    let [x0, y0, x1, y1] = box_rect;
    if x1 > x0 {
        let text = if value.is_empty() { "-" } else { value };
        node.children.push(text_node(
            &format!("{}/info", node.path),
            text,
            22.0,
            [x0 + 10.0, y0, x1 - 20.0, y1],
        ));
    }
    recolor_texts(&mut node, 0.1647);
    ctx.add(node);
}

/// A captured panel with click regions on its buttons.
fn layout_panel(ctx: &mut Ctx<'_>, template: &str, buttons: &[(&str, UiAction)], y: f32) -> f32 {
    let Some(source) = ctx.templates.extra(template) else {
        return 0.0;
    };
    let height = rect_of(source).map_or(0.0, |[_, y0, _, y1]| y1 - y0);
    if !ctx.visible(y, height) {
        return height;
    }
    let mut node = source.clone();
    let dy = y - top_of(&node);
    shift(&mut node, dy);
    for (name, action) in buttons {
        if let Some(button) = find(&node, &|n: &Node| n.name == *name)
            && let Some([x0, y0, x1, y1]) = rect_of(button)
        {
            ctx.hits.push(DynHit {
                rect: [x0, y0, x1 - x0, y1 - y0],
                action: *action,
            });
        }
    }
    ctx.add(node);
    height
}

/// A big section title.
fn layout_heading(ctx: &mut Ctx<'_>, text: &str, y: f32) -> f32 {
    let Some(source) = ctx.templates.extra("Title (1)") else {
        return 0.0;
    };
    let height = rect_of(source).map_or(0.0, |[_, y0, _, y1]| y1 - y0);
    if ctx.visible(y, height) {
        let mut node = source.clone();
        let dy = y - top_of(&node);
        shift(&mut node, dy);
        fit_to_content(ctx, &mut node);
        set_text(&mut node, text);
        ctx.add(node);
    }
    height
}

/// The grey "no items" line at the top of an empty list box.
fn layout_placeholder(ctx: &mut Ctx<'_>, template: &str, text: &str, y: f32) {
    let Some(source) = ctx.templates.extra(template) else {
        return;
    };
    let height = rect_of(source).map_or(0.0, |[_, y0, _, y1]| y1 - y0);
    if !ctx.visible(y, height) {
        return;
    }
    let mut node = source.clone();
    let dy = y - top_of(&node);
    shift(&mut node, dy);
    // The line was captured in the editor's content area; the world's panel is elsewhere.
    fit_to_content(ctx, &mut node);
    if !text.is_empty() {
        set_text(&mut node, text);
    }
    ctx.add(node);
}

/// Rows without a group frame (each row brings its own title).
fn layout_bare(ctx: &mut Ctx<'_>, rows: &[Row], y: f32) -> f32 {
    let mut cursor = y;
    let spacing = content_spacing(ctx.input);
    for (index, row) in rows.iter().enumerate() {
        cursor += layout_row(ctx, row, cursor);
        if index + 1 < rows.len() {
            cursor += spacing;
        }
    }
    cursor - y
}

fn layout_group(ctx: &mut Ctx<'_>, title: &str, rows: &[Row], y: f32) -> f32 {
    let mut cursor = y;
    // Title.
    if ctx.visible(cursor, TITLE_H) {
        let mut node = ctx.templates.title.clone();
        let dy = cursor - top_of(&node);
        shift(&mut node, dy);
        fit_to_content(ctx, &mut node);
        set_text(&mut node, title);
        ctx.add(node);
    }
    // Layout spacing (10) + the holder's top padding (10), measured on the capture.
    cursor += TITLE_H + 10.0 + 10.0;
    for (index, row) in rows.iter().enumerate() {
        let height = layout_row(ctx, row, cursor);
        cursor += height;
        if index + 1 < rows.len() {
            cursor += SPACING;
        }
    }
    // The holder's bottom padding.
    cursor += SPACING;
    cursor - y
}

/// What a generated row edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldRef {
    /// `SETTING_UI[n]`.
    Setting(u16),
    /// Field `.1` of zone `.0` of the scenario's zone list.
    Zone(u16, u8),
    /// Field `.1` of material `.0` (Plant, Meat, Fat, Armor).
    Material(u8, u8),
    /// Field `.1` of default bibite `.0`.
    Bibite(u16, u8),
    /// Gene `.0` (an index of the core's gene table) in the Bibite Editor.
    Gene(u8),
    /// The bias of the node the Bibite Editor's Brain tab has open (`NodeEditPanel.biasSlider`).
    BrainBias,
    /// The value of the input node the Brain tab has open (`NodeEditPanel.actSlider`).
    BrainAct,
    /// The weight of the connection the Brain tab has open (`ConnectionEditPanel.weightSlider`).
    BrainWeight,
    /// A setting of the Expanded Brain View (`ExpandedBrainParameters`): an index of `expanded_brain::Param`.
    Expanded(u8),
    /// The time-speed slider at the top of the screen (`TimeController.targetTimeScale`, a `LogFloatSettingSlider`): a click on its number types a speed.
    TimeSpeed,
}

/// The action of a click area of the settings panel moved `dx` reference px to the right: the slider tracks and the graph rectangles an action
/// carries are in the same reference px as the click area's rectangle (the panel is flush with the right edge of a canvas that is wider or narrower
/// than 1920 px, see `ui_runtime::dyn_panel_offset`), and a slider is moved by where the pointer is on its track.
#[must_use]
pub(crate) fn offset_action(action: UiAction, dx: f32) -> UiAction {
    let by = |x: u16| (f32::from(x) + dx).round().clamp(0.0, f32::from(u16::MAX)) as u16;
    match action {
        UiAction::DynSlide(index, x0, x1) => UiAction::DynSlide(index, by(x0), by(x1)),
        UiAction::ZoneSlide(index, field, x0, x1) => UiAction::ZoneSlide(index, field, by(x0), by(x1)),
        UiAction::BibiteSlide(index, field, x0, x1) => UiAction::BibiteSlide(index, field, by(x0), by(x1)),
        UiAction::MaterialSlide(kind, field, x0, x1) => UiAction::MaterialSlide(kind, field, by(x0), by(x1)),
        UiAction::AnimGraph(changer, x0, y0, x1, y1) => UiAction::AnimGraph(changer, by(x0), y0, by(x1), y1),
        other => other,
    }
}

impl FieldRef {
    fn slide(self, x0: u16, x1: u16) -> UiAction {
        use crate::brain_editor_ui::{BrainAction, Slider};
        use crate::template_editor_ui::EditorAction;
        match self {
            Self::Setting(index) => UiAction::DynSlide(index, x0, x1),
            Self::Zone(zone, field) => UiAction::ZoneSlide(zone, field, x0, x1),
            Self::Material(kind, field) => UiAction::MaterialSlide(kind, field, x0, x1),
            Self::Bibite(index, field) => UiAction::BibiteSlide(index, field, x0, x1),
            Self::Gene(gene) => UiAction::Editor(EditorAction::GeneSlide(gene, x0, x1)),
            Self::BrainBias => {
                UiAction::Editor(EditorAction::Brain(BrainAction::Slide(Slider::Bias, x0, x1)))
            }
            Self::BrainAct => {
                UiAction::Editor(EditorAction::Brain(BrainAction::Slide(Slider::Act, x0, x1)))
            }
            Self::BrainWeight => UiAction::Editor(EditorAction::Brain(BrainAction::Slide(
                Slider::Weight,
                x0,
                x1,
            ))),
            Self::Expanded(index) => {
                UiAction::ExpandedBrain(crate::expanded_brain_ui::ExAction::Slide(index, x0, x1))
            }
            // The time slider's track in the capture (reference px).
            Self::TimeSpeed => UiAction::SpeedSlide(845, 1075),
        }
    }

    fn reset(self) -> UiAction {
        match self {
            Self::Setting(index) => UiAction::DynReset(index),
            Self::Zone(zone, field) => UiAction::ZoneReset(zone, field),
            Self::Material(kind, field) => UiAction::MaterialReset(kind, field),
            Self::Bibite(index, field) => UiAction::BibiteReset(index, field),
            Self::Gene(gene) => {
                UiAction::Editor(crate::template_editor_ui::EditorAction::GeneReset(gene))
            }
            // The Brain tab's sliders have no reset button.
            Self::BrainBias | Self::BrainAct | Self::BrainWeight => UiAction::Consume,
            Self::Expanded(index) => {
                UiAction::ExpandedBrain(crate::expanded_brain_ui::ExAction::ResetParam(index))
            }
            // The time slider has no reset button (the play button sets it back to 1).
            Self::TimeSpeed => UiAction::Consume,
        }
    }

    fn toggle(self) -> UiAction {
        match self {
            Self::Setting(index) => UiAction::DynToggle(index),
            Self::Zone(zone, field) => UiAction::ZoneToggle(zone, field),
            Self::Material(kind, _) => UiAction::MaterialToggle(kind),
            Self::Bibite(index, field) => UiAction::BibiteCycle(index, field),
            Self::Gene(gene) => {
                UiAction::Editor(crate::template_editor_ui::EditorAction::GeneReset(gene))
            }
            Self::BrainBias | Self::BrainAct | Self::BrainWeight | Self::Expanded(_) | Self::TimeSpeed => {
                UiAction::Consume
            }
        }
    }
}

/// Everything a slider row shows.
struct SliderSpec<'a> {
    name: &'a str,
    text: String,
    fraction: f32,
    default_fraction: f32,
    field: FieldRef,
    /// How the value text reads (the edit field shows the typed value this way).
    format: FloatValueFormat,
    /// What typing a value into the row does.
    rules: Rules,
}

/// The typed-value rules of a `SETTING_UI` row (`NumericSetting`: `factor`, `canGoOutOfBounds`, ...).
pub(crate) fn setting_rules(row: &SettingUi, mode: SliderMode) -> Rules {
    Rules {
        factor: row.factor,
        min: row.min,
        max: row.max,
        out_of_bounds: row.out_of_bounds,
        integer: row.kind == SettingUiKind::Int,
        log: matches!(mode, SliderMode::Log { .. }),
    }
}

/// The typed-value rules of a zone, material or default bibite slider: those settings leave
/// `canGoOutOfBounds` at its default, true.
pub(crate) fn info_rules(info: &crate::zone_editor::SliderInfo, integer: bool) -> Rules {
    Rules {
        factor: info.factor,
        min: info.min,
        max: info.max,
        out_of_bounds: true,
        integer,
        log: matches!(info.mode, SliderMode::Log { .. }),
    }
}

/// Dark text for rows drawn on a light box (the zone items).
fn recolor_texts(node: &mut Node, rgb: f32) {
    for text in &mut node.texts {
        if let Some(color) = text.color.as_mut() {
            color.r = rgb;
            color.g = rgb;
            color.b = rgb;
        }
    }
    for child in &mut node.children {
        recolor_texts(child, rgb);
    }
}

fn layout_toggle(ctx: &mut Ctx<'_>, key: &str, y: f32) {
    let Some((index, row)) = ui_of(key) else {
        return;
    };
    let value = read_value(ctx.input.settings, row) >= 0.5;
    emit_toggle(
        ctx,
        row.name,
        value,
        FieldRef::Setting(index as u16),
        y,
        false,
    );
}

/// A toggle row: label and checkbox (`dark`: dark text, for rows on a light box).
fn emit_toggle(
    ctx: &mut Ctx<'_>,
    name_text: &str,
    value: bool,
    field: FieldRef,
    y: f32,
    dark: bool,
) {
    if !ctx.visible(y, ROW_H) {
        return;
    }
    let mut node = ctx.templates.toggle.clone();
    let dy = y - top_of(&node);
    shift(&mut node, dy);
    if let Some(name) = find_mut(&mut node, &|n: &Node| n.name == "Name") {
        set_label(name, name_text);
    }
    push_row_tip(ctx, &node, field);
    if let Some(check) = find_mut(&mut node, &|n: &Node| n.name == "Checkmark") {
        check.active_in_hierarchy = value;
        check.active = value;
    }
    // An animated setting's toggle is not interactable: its background (the target graphic) takes the disabled tint.
    if is_animated_field(ctx, field)
        && let Some(background) = find_mut(&mut node, &|n: &Node| n.path.ends_with("/Toggle/Background"))
    {
        tint_disabled(background);
    }
    if let Some(background) = find(&node, &|n: &Node| n.path.ends_with("/Toggle/Background")) {
        if let Some([x0, y0, x1, y1]) = rect_of(background) {
            ctx.hits.push(DynHit {
                rect: [x0, y0, x1 - x0, y1 - y0],
                action: field.toggle(),
            });
        }
    }
    if dark {
        recolor_texts(&mut node, 0.1647);
    }
    ctx.add(node);
}

/// Whether a settings animator animates the setting of `field` (the row is then not interactable).
fn is_animated_field(ctx: &Ctx<'_>, field: FieldRef) -> bool {
    let FieldRef::Setting(index) = field else {
        return false;
    };
    let (Some(lists), Some(row)) = (ctx.input.lists, SETTING_UI.get(usize::from(index))) else {
        return false;
    };
    lists.changers.iter().any(|changer| changer.settings_bases.iter().any(|base| base.key == row.field))
}

/// The ColorBlock's disabled tint (0.784 grey, half alpha) on the images of a control's target graphic.
fn tint_disabled(node: &mut Node) {
    for image in &mut node.images {
        if let Some(color) = image.color.as_mut() {
            color.r *= 0.784;
            color.g *= 0.784;
            color.b *= 0.784;
            color.a *= 0.5;
        }
    }
}

/// A dropdown row (the label and the current choice; a click opens its list).
fn emit_choice(
    ctx: &mut Ctx<'_>,
    name_text: &str,
    choice: &str,
    action: UiAction,
    field: FieldRef,
    y: f32,
    dark: bool,
) {
    if !ctx.visible(y, ROW_H) {
        return;
    }
    let mut node = ctx.templates.toggle.clone();
    let dy = y - top_of(&node);
    shift(&mut node, dy);
    if let Some(name) = find_mut(&mut node, &|n: &Node| n.name == "Name") {
        set_label(name, name_text);
    }
    push_row_tip(ctx, &node, field);
    let mut box_rect = [0.0_f32; 4];
    if let Some(toggle) = find_mut(&mut node, &|n: &Node| n.name == "Toggle") {
        if let Some([x0, y0, x1, y1]) = rect_of(toggle) {
            box_rect = [x0, y0, x1, y1];
        }
        set_active(toggle, false);
    }
    let [x0, y0, x1, y1] = box_rect;
    if x1 > x0 {
        let mut value = text_node(
            &format!("{}/choice", node.path),
            choice,
            22.0,
            [x0 + 10.0, y0, x1 - 20.0, y1],
        );
        if !dark {
            recolor_texts(&mut value, 1.0);
        }
        node.children.push(value);
        ctx.hits.push(DynHit {
            rect: [x0, y0, x1 - x0, y1 - y0],
            action,
        });
    }
    if dark {
        recolor_texts(&mut node, 0.1647);
    }
    ctx.add(node);
}

fn layout_slider(ctx: &mut Ctx<'_>, key: &str, mode: SliderMode, y: f32) {
    let Some((index, row)) = ui_of(key) else {
        return;
    };
    let default = default_of(row, ctx.input.tps_defaults);
    let value = read_value(ctx.input.settings, row);
    let mut format = format_of(row);
    if matches!(mode, SliderMode::Log { .. }) {
        // `LogSettingSlider` shows significant digits (`sliderValue.precisionIsSI = true`).
        format.precision_is_si = true;
    }
    let spec = SliderSpec {
        name: row.name,
        text: format.format(value),
        fraction: fraction_of(mode, row, default, value),
        default_fraction: fraction_of(mode, row, default, default),
        field: FieldRef::Setting(index as u16),
        rules: setting_rules(row, mode),
        format,
    };
    emit_slider(ctx, &spec, y, false);
}

/// The tooltip of a settings row (the game sets the trigger on the name label to the setting's name
/// and help text): a hover region over the label of the row `node`. Only the scenario and
/// independent settings have a help text here.
fn push_row_tip(ctx: &mut Ctx<'_>, node: &Node, field: FieldRef) {
    let tip = match field {
        FieldRef::Setting(index) => SETTING_UI
            .get(usize::from(index))
            .and_then(crate::tooltip::of_setting),
        FieldRef::Zone(index, field) => ctx
            .input
            .lists
            .and_then(|lists| lists.zones.get(usize::from(index)))
            .and_then(|zone| crate::zone_editor::help_key(zone, field))
            .and_then(|key| crate::tooltip::of_item("zone", key)),
        FieldRef::Bibite(_, field) => crate::bibite_editor::help_key(field)
            .and_then(|key| crate::tooltip::of_item("bibite", key)),
        FieldRef::Material(_, field) => crate::zone_editor::material_help_key(field)
            .and_then(|key| crate::tooltip::of_item("material", key)),
        // The Bibite Editor's rows are drawn by `template_editor_ui` and `brain_editor_ui`, with their tips.
        FieldRef::Gene(_)
        | FieldRef::BrainBias
        | FieldRef::BrainAct
        | FieldRef::BrainWeight
        | FieldRef::Expanded(_)
        | FieldRef::TimeSpeed => None,
    };
    let Some(tip) = tip else {
        return;
    };
    if let Some(name) = find(node, &|n: &Node| n.name == "Name")
        && let Some([x0, y0, x1, y1]) = rect_of(name)
    {
        ctx.tips.push(DynTip {
            rect: [x0, y0, x1 - x0, y1 - y0],
            tip,
        });
    }
}

/// A flat or sliced image generated for the edit field.
fn field_image(path: &str, sprite: Option<&str>, rect: [f32; 4], tint: [f32; 4]) -> Node {
    let mut node = match sprite {
        Some(sprite) => sprite_node(path, sprite, rect, tint),
        None => {
            let mut node = sprite_node(path, "", rect, tint);
            for image in &mut node.images {
                image.sprite = None;
            }
            node
        }
    };
    if sprite == Some("InputFieldBackground") {
        use crate::ui_runtime::Border;
        // `InputFieldBackground`: a 32 x 32 sprite with a border of 8 at 100 pixels per unit.
        for image in &mut node.images {
            image.image_type = "Sliced".to_owned();
            image.runtime_sprite_border = Some(Border {
                x: 8.0,
                y: 8.0,
                z: 8.0,
                w: 8.0,
            });
        }
    }
    node
}

/// The text of the edit field: `LiberationSans SDF` 20, left aligned, dark grey.
fn field_text(path: &str, text: &str, rect: [f32; 4], color: [f32; 4]) -> Node {
    use crate::ui_runtime::{Color, FontRef};
    let mut node = text_node(path, text, 20.0, rect);
    for component in &mut node.texts {
        component.font = Some(FontRef {
            name: "LiberationSans SDF".to_owned(),
            ..FontRef::default()
        });
        component.color = Some(Color {
            r: color[0],
            g: color[1],
            b: color[2],
            a: color[3],
        });
    }
    node
}

/// The dark grey of the field's text and caret (`Text` colour 50 / 255).
const FIELD_INK: [f32; 3] = [0.196_078_43; 3];

/// `OpenEditPanel`: the slider and its reset button (`NormalSecton`) leave the row, the three children
/// that are left share its width again with the edit section (Unity's layout group), and the
/// `InputField (TMP)` takes the slider's place: the sliced `InputFieldBackground`, the selection
/// highlight (`selectionColor`) behind the text or the blinking caret after it (1 wide), and the
/// `Enter value...` placeholder while it is empty.
fn edit_row(ctx: &mut Ctx<'_>, node: &mut Node, edit: &ValueEdit) {
    let Some([rx0, ry0, rx1, ry1]) = rect_of(node) else {
        return;
    };
    node.children.retain(|child| child.name != "NormalSecton");
    let parts = value_edit::row_parts(rx1 - rx0, true);
    let (x0, x1) = (rx0 + parts.body[0], rx0 + parts.body[1]);
    let middle = (ry0 + ry1) * 0.5;
    let (y0, y1) = (
        middle - value_edit::FIELD_HEIGHT * 0.5,
        middle + value_edit::FIELD_HEIGHT * 0.5,
    );
    let path = format!("{}/EditSection/InputField (TMP)", node.path);
    let first = node.children.len() as i32 + 10;
    let mut added: Vec<Node> = Vec::new();
    added.push(field_image(
        &path,
        Some("InputFieldBackground"),
        [x0, y0, x1, y1],
        [1.0; 4],
    ));
    // `Text Area`: 10 in from the sides, 6.5 from the top and bottom, half a pixel lower; the text
    // sits 2 in from its left edge.
    let area = [x0 + 10.0 + 2.0, y0 + 6.5 + 0.5, x1 - 10.0, y1 - 6.5 + 0.5];
    let text_x = area[0];
    let line = 23.0;
    let line_top = middle + 0.5 - line * 0.5;
    let before = |chars: usize| value_edit::text_width(&edit.text.chars().take(chars).collect::<String>(), 20.0);
    let width = before(edit.caret.pos);
    if let Some((from, to)) = edit.caret.selection() {
        added.push(field_image(
            &format!("{path}/Text Area/Selection"),
            None,
            [text_x + before(from), line_top, text_x + before(to), line_top + line],
            [0.658_823_5, 0.807_843_2, 1.0, 0.752_941_2],
        ));
    }
    let ink = [FIELD_INK[0], FIELD_INK[1], FIELD_INK[2]];
    if edit.text.is_empty() {
        added.push(field_text(
            &format!("{path}/Text Area/Placeholder"),
            "Enter value...",
            area,
            [ink[0], ink[1], ink[2], 0.5],
        ));
    } else {
        added.push(field_text(
            &format!("{path}/Text Area/Text"),
            &edit.text,
            area,
            [ink[0], ink[1], ink[2], 1.0],
        ));
    }
    if edit.caret_visible() {
        added.push(field_image(
            &format!("{path}/Text Area/Caret"),
            None,
            [
                text_x + width,
                line_top,
                text_x + width + 1.0,
                line_top + line,
            ],
            [ink[0], ink[1], ink[2], 1.0],
        ));
    }
    for (offset, mut child) in added.into_iter().enumerate() {
        child.sibling_index = first + offset as i32;
        node.children.push(child);
    }
    ctx.hits.push(DynHit {
        rect: [x0, y0, x1 - x0, y1 - y0],
        action: UiAction::ValueEditInside,
    });
}

/// A slider row with the captured prefab's geometry.
fn emit_slider(ctx: &mut Ctx<'_>, spec: &SliderSpec<'_>, y: f32, dark: bool) {
    if !ctx.visible(y, ROW_H) {
        return;
    }
    let editing = ctx.input.edit.filter(|edit| edit.field == spec.field);
    let mut node = ctx.templates.slider.clone();
    let dy = y - top_of(&node);
    shift(&mut node, dy);
    let extra = ctx.geometry.content_x[1] - CONTENT_X[1];
    if extra > 0.5 {
        widen_slider_row(&mut node, extra);
    }
    if editing.is_some() {
        // The name and the value text share the row with the edit field, not with the slider.
        let parts = rect_of(&node).map(|[x0, _, x1, _]| (x0, value_edit::row_parts(x1 - x0, true)));
        if let Some((rx0, parts)) = parts {
            for (name, range) in [("Name", parts.name), ("FloatInfoText", parts.value)] {
                if let Some(child) = find_mut(&mut node, &|n: &Node| n.name == name)
                    && let Some([_, y0, _, y1]) = rect_of(child)
                {
                    set_rect(child, rx0 + range[0], y0, rx0 + range[1], y1);
                }
            }
        }
    }
    if let Some(name) = find_mut(&mut node, &|n: &Node| n.name == "Name") {
        set_label(name, spec.name);
    }
    push_row_tip(ctx, &node, spec.field);
    // `UpdateValueDuringEdit`: the value text shows the value the typed text means.
    let shown = editing
        .and_then(|edit| spec.rules.preview(&edit.text))
        .map_or_else(|| spec.text.clone(), |value| spec.format.format(value));
    if let Some(info) = find_mut(&mut node, &|n: &Node| n.name == "FloatInfoText") {
        set_text(info, &shown);
    }
    // The value text is the row's `editButton`.
    if let Some(info) = find(&node, &|n: &Node| n.name == "FloatInfoText")
        && let Some([x0, y0, x1, y1]) = rect_of(info)
    {
        ctx.hits.push(DynHit {
            rect: [x0, y0, x1 - x0, y1 - y0],
            action: UiAction::DynEditValue(spec.field),
        });
    }
    if let Some(edit) = editing {
        if dark {
            recolor_texts(&mut node, 0.1647);
        }
        edit_row(ctx, &mut node, edit);
        ctx.add(node);
        return;
    }
    let animated = is_animated_field(ctx, spec.field);
    // Slider geometry (reference px, y down). The track is the captured `Slider` rectangle.
    let track = find(&node, &|n: &Node| n.path.ends_with("/SliderPanel/Slider"))
        .and_then(rect_of)
        .unwrap_or([1517.0, y + 10.0, 1806.0, y + 30.0]);
    let (track_x0, track_x1) = (track[0], track[2]);
    let centre = track_x0 + spec.fraction * (track_x1 - track_x0);
    if let Some(handle) = find_mut(&mut node, &|n: &Node| {
        n.path.ends_with("/Handle Slide Area/Handle")
    }) {
        if let Some([x0, y0, x1, y1]) = rect_of(handle) {
            let half = (x1 - x0) * 0.5;
            set_rect(handle, centre - half, y0, centre + half, y1);
        }
        // An animated setting's slider is not interactable: the handle (the slider's target graphic) takes the
        // ColorBlock's disabled tint.
        if animated {
            tint_disabled(handle);
        }
    }
    if let Some(fill) = find_mut(&mut node, &|n: &Node| n.path.ends_with("/Fill Area/Fill")) {
        if let Some([x0, y0, _, y1]) = rect_of(fill) {
            set_rect(fill, x0, y0, centre + 5.5, y1);
        }
    }
    let pointer_centre = track_x0 + spec.default_fraction * (track_x1 - track_x0);
    if let Some(pointer) = find_mut(&mut node, &|n: &Node| {
        n.path.ends_with("/Slider/defaultvaluepointer")
    }) {
        shift_pointer(pointer, pointer_centre);
    }
    if let Some(panel) = find(&node, &|n: &Node| {
        n.path.ends_with("/NormalSecton/SliderPanel")
    }) {
        if let Some([x0, y0, x1, y1]) = rect_of(panel) {
            ctx.hits.push(DynHit {
                rect: [x0, y0, x1 - x0, y1 - y0],
                action: spec
                    .field
                    .slide(track_x0.round() as u16, track_x1.round() as u16),
            });
        }
    }
    if let Some(reset) = find(&node, &|n: &Node| {
        n.path.ends_with("/NormalSecton/ResetButton")
    }) {
        if let Some([x0, y0, x1, y1]) = rect_of(reset) {
            ctx.hits.push(DynHit {
                rect: [x0, y0, x1 - x0, y1 - y0],
                action: spec.field.reset(),
            });
        }
    }
    if dark {
        recolor_texts(&mut node, 0.1647);
    }
    ctx.add(node);
}

/// A slider row of the content's width when the content is `extra` wider than the captured one (a tab whose scroll bar is hidden: the Zones tab): the
/// row's layout group shares the extra width out (read from the live game: the name takes half of it, the value and the slider a quarter each).
fn widen_slider_row(node: &mut Node, extra: f32) {
    fn walk(node: &mut Node, extra: f32) {
        let path = node.path.clone();
        let name = node.name.clone();
        let shift_x = |node: &mut Node, dx0: f32, dx1: f32| {
            if let Some(rect) = node.native_rect.as_mut() {
                rect[0] += dx0;
                rect[2] += dx1;
            }
        };
        if path.contains("/NormalSecton/ResetButton") {
            shift_x(node, extra, extra);
        } else if name == "Name" {
            shift_x(node, 0.0, extra * 0.5);
        } else if name == "FloatInfoText" {
            shift_x(node, extra * 0.5, extra * 0.75);
        } else if matches!(
            name.as_str(),
            "NormalSecton" | "SliderPanel" | "Slider" | "Background" | "Fill Area" | "Handle Slide Area"
        ) && path.contains("/NormalSecton")
            || name == "NormalSecton"
        {
            shift_x(node, extra * 0.75, extra);
        }
        for child in &mut node.children {
            walk(child, extra);
        }
    }
    if let Some(rect) = node.native_rect.as_mut() {
        rect[2] += extra;
    }
    for child in &mut node.children {
        walk(child, extra);
    }
}

/// Centres the default-value pointer (and its two tick marks) on `centre`.
fn shift_pointer(pointer: &mut Node, centre: f32) {
    let Some([x0, y0, x1, y1]) = rect_of(pointer) else {
        return;
    };
    let half = (x1 - x0) * 0.5;
    let dx = centre - (x0 + half);
    fn move_x(node: &mut Node, dx: f32) {
        if let Some(rect) = node.native_rect.as_mut() {
            rect[0] += dx;
            rect[2] += dx;
        }
        for child in &mut node.children {
            move_x(child, dx);
        }
    }
    let _ = (y0, y1);
    move_x(pointer, dx);
}

fn layout_sub(ctx: &mut Ctx<'_>, title: &str, master: &str, rows: &[Row], y: f32) -> f32 {
    let group_id = ctx.next_group;
    ctx.next_group += 1;
    let open = ctx.input.state.is_open(group_id);
    let master_on =
        ui_of(master).is_some_and(|(_, row)| read_value(ctx.input.settings, row) >= 0.5);
    let sub_height = 50.0;
    if ctx.visible(y, sub_height) {
        if let Some((index, row)) = ui_of(master) {
            let mut node = ctx.templates.sub.clone();
            let dy = y - top_of(&node);
            shift(&mut node, dy);
            // The prefab's inactive sub-panel is replaced by the generated one below.
            node.children.retain(|child| child.name != "SubPanel");
            if let Some(name) = find_mut(&mut node, &|n: &Node| n.name == "Name") {
                set_label(name, row.name);
            }
            push_row_tip(ctx, &node, FieldRef::Setting(index as u16));
            if let Some(check) = find_mut(&mut node, &|n: &Node| n.name == "Checkmark") {
                check.active_in_hierarchy = master_on;
                check.active = master_on;
            }
            if let Some(background) =
                find(&node, &|n: &Node| n.path.ends_with("/Toggle/Background"))
            {
                if let Some([x0, y0, x1, y1]) = rect_of(background) {
                    ctx.hits.push(DynHit {
                        rect: [x0, y0, x1 - x0, y1 - y0],
                        action: UiAction::DynToggle(index as u16),
                    });
                }
            }
            if let Some(edit) = find(&node, &|n: &Node| n.path.ends_with("/GroupEdit/Background")) {
                if let Some([x0, y0, x1, y1]) = rect_of(edit) {
                    ctx.hits.push(DynHit {
                        rect: [x0, y0, x1 - x0, y1 - y0],
                        action: UiAction::DynSub(group_id),
                    });
                }
            }
            ctx.add(node);
        }
    }
    let mut height = sub_height;
    if open && master_on {
        height += layout_sub_panel(ctx, title, rows, y + height, group_id);
    }
    height
}

/// The frame of an expanded sub-panel (`SubPanel`): background, title, reset and close buttons.
fn emit_sub_panel_shell(
    ctx: &mut Ctx<'_>,
    title: &str,
    y: f32,
    total: f32,
    close: UiAction,
    reset: UiAction,
) {
    let header_h = 60.0;
    let (x0, x1) = (ctx.geometry.content_x[0], ctx.geometry.content_x[1]);
    // The frame is clipped to the viewport (the game's scroll view masks it); the header and its
    // buttons only show while they are fully inside.
    if ctx.dry || y + total <= ctx.view_top || y >= ctx.view_bottom {
        return;
    }
    let Some(template) = ctx.templates.sub_panel.clone() else {
        return;
    };
    let header_visible = y >= ctx.view_top && y + header_h <= ctx.view_bottom;
    let mut panel = template;
    set_active(&mut panel, true);
    set_rect(
        &mut panel,
        x0,
        y.max(ctx.view_top),
        x1,
        (y + total).min(ctx.view_bottom),
    );
    // Drop the prefab's own (geometry-less) rows; header children get explicit rects.
    let mut header_children = Vec::new();
    if let Some(header) = find_mut(&mut panel, &|n: &Node| n.name == "SubSettingsHeader") {
        set_rect(header, x0, y, x1, y + header_h);
        for child in &mut header.children {
            match child.name.as_str() {
                "Title" => {
                    set_rect(child, x0 + 80.0, y + 5.0, x1 - 160.0, y + header_h - 5.0);
                    set_text(child, title);
                }
                "CloseButton" => {
                    set_rect(child, x1 - 55.0, y + 10.0, x1 - 15.0, y + 50.0);
                    for inner in &mut child.children {
                        set_rect(inner, x1 - 55.0, y + 10.0, x1 - 15.0, y + 50.0);
                    }
                    header_children.push(DynHit {
                        rect: [x1 - 55.0, y + 10.0, 40.0, 40.0],
                        action: close,
                    });
                }
                "ResetButton" => {
                    set_rect(child, x0 + 15.0, y + 10.0, x0 + 55.0, y + 50.0);
                    for inner in &mut child.children {
                        set_rect(inner, x0 + 15.0, y + 10.0, x0 + 55.0, y + 50.0);
                    }
                    header_children.push(DynHit {
                        rect: [x0 + 15.0, y + 10.0, 40.0, 40.0],
                        action: reset,
                    });
                }
                _ => {}
            }
        }
    }
    panel
        .children
        .retain(|child| header_visible && child.name == "SubSettingsHeader");
    if header_visible {
        ctx.hits.extend(header_children);
    }
    ctx.add(panel);
}

/// The expanded sub-panel (`SubPanel`: header with title / reset / close, then the rows).
fn layout_sub_panel(ctx: &mut Ctx<'_>, title: &str, rows: &[Row], y: f32, group_id: u16) -> f32 {
    let header_h = 60.0;
    let inner_h: f32 =
        rows.iter().map(|_| ROW_H).sum::<f32>() + SPACING * rows.len().saturating_sub(1) as f32;
    let total = header_h + 10.0 + inner_h + 20.0;
    emit_sub_panel_shell(
        ctx,
        title,
        y,
        total,
        UiAction::DynSub(group_id),
        UiAction::DynResetGroup(group_id),
    );
    let mut cursor = y + header_h + 10.0;
    for row in rows {
        let height = layout_row(ctx, row, cursor);
        cursor += height + SPACING;
    }
    total
}

/// A material block of the Energy tab: the collapsed row (pellet picture, title, gear) and, when
/// open, the parameter panel with the six sliders and the decay toggle with its two sliders.
fn layout_material(ctx: &mut Ctx<'_>, kind: u8, y: f32) -> f32 {
    use crate::zone_editor as ze;
    let group_id = ctx.next_group;
    ctx.next_group += 1;
    let open = ctx.input.state.is_open(group_id);
    let material = ze::material_kind(kind);
    let mut height = ROW_H;
    if ctx.visible(y, ROW_H)
        && let Some(source) = ctx.templates.extra("MaterialParametersGroupHandle(Clone)")
    {
        let mut node = source.clone();
        let dy = y - top_of(&node);
        shift(&mut node, dy);
        // The row was captured in the editor's content area; the world's panel is elsewhere.
        fit_to_content(ctx, &mut node);
        let (title, sprite) = match material {
            MaterialKind::Plant => ("Plant Material Parameters", "medium_plant"),
            MaterialKind::Meat => ("Meat Material Parameters", "medium_meat"),
            MaterialKind::Fat => ("Fat Material Parameters", "fat_pellet"),
            MaterialKind::Armor => ("Armor Material Parameters", "medium_plant"),
        };
        if let Some(label) = find_mut(&mut node, &|n: &Node| n.name == "Title1") {
            set_text(label, title);
        }
        if let Some(picture) = find_mut(&mut node, &|n: &Node| n.name == "Pellet Image")
            && let Some(image) = picture.images.first_mut()
        {
            image.sprite = Some(crate::ui_runtime::SpriteRef {
                name: sprite.to_owned(),
                ..Default::default()
            });
        }
        if let Some(gear) = find(&node, &|n: &Node| n.name == "OpenButton")
            && let Some([x0, y0, x1, y1]) = rect_of(gear)
        {
            ctx.hits.push(DynHit {
                rect: [x0, y0, x1 - x0, y1 - y0],
                action: UiAction::DynSub(group_id),
            });
        }
        // Only the row itself is drawn (the prefab's hidden panel has no geometry).
        node.children.retain(|child| child.name != "SubPanel");
        ctx.add(node);
    }
    if open {
        let decay_on = ctx.input.settings.materials.get(material).decay;
        let sliders = [
            ze::MAT_ENERGY,
            ze::MAT_MASS,
            ze::MAT_COHESION,
            ze::MAT_REACTIVITY,
            ze::MAT_MAX_EFFICIENCY,
            ze::MAT_MIN_EFFICIENCY,
        ];
        let rows = sliders.len() + 1 + if decay_on { 2 } else { 0 };
        let header_h = 60.0;
        let inner_h = rows as f32 * ROW_H + SPACING * (rows - 1) as f32;
        let total = header_h + 10.0 + inner_h + 20.0;
        let top = y + ROW_H;
        emit_sub_panel_shell(
            ctx,
            "Material Parameters",
            top,
            total,
            UiAction::DynSub(group_id),
            UiAction::MaterialResetAll(kind),
        );
        let mut cursor = top + header_h + 10.0;
        let current = *ctx.input.settings.materials.get(material);
        let emit = |ctx: &mut Ctx<'_>, field: u8, cursor: f32| {
            let Some(info) = ze::material_slider_info(material, field) else {
                return;
            };
            let value = ze::material_value(&current, field);
            let spec = SliderSpec {
                name: ze::material_name_of(field),
                text: info.format().format(value),
                fraction: fraction_range(info.mode, info.min, info.max, info.default, value),
                default_fraction: fraction_range(
                    info.mode,
                    info.min,
                    info.max,
                    info.default,
                    info.default,
                ),
                field: FieldRef::Material(kind, field),
                format: info.format(),
                rules: info_rules(&info, false),
            };
            emit_slider(ctx, &spec, cursor, false);
        };
        for field in sliders {
            emit(ctx, field, cursor);
            cursor += ROW_H + SPACING;
        }
        emit_toggle(
            ctx,
            ze::material_name_of(ze::MAT_DECAY),
            decay_on,
            FieldRef::Material(kind, ze::MAT_DECAY),
            cursor,
            false,
        );
        cursor += ROW_H + SPACING;
        if decay_on {
            for field in [ze::MAT_FRESH_TIME, ze::MAT_DECAY_RATE] {
                emit(ctx, field, cursor);
                cursor += ROW_H + SPACING;
            }
        }
        height += total + SPACING;
    }
    height
}

/// Rows generated for one scrolling list.
pub struct ListBuild {
    /// Row nodes (absolute positions, scroll applied, in z-order).
    pub nodes: Vec<Node>,
    /// Click regions of the rows.
    pub hits: Vec<DynHit>,
    /// Hover regions of the rows (their tooltips).
    pub tips: Vec<DynTip>,
    /// Total content height (reference px).
    pub content_h: f32,
    /// Largest useful scroll offset.
    pub max_scroll: f32,
    /// The scroll offset used (clamped to `0..=max_scroll`).
    pub scroll: f32,
}

/// Whether this is the Zones tab of the in-game panel (`ZoneEditorPanel`: its content lays its children out 15 apart, the other tabs' groups 20).
fn in_game_zones_tab(input: &DynInput<'_>) -> bool {
    input.state.tab == 1 && input.lists.as_ref().is_some_and(|lists| lists.in_sim)
}

/// Whether this is the Bibite tab of the in-game panel (`DefaultBibitesPanel`: 15 between its content's children, the parameter groups under the cards 20).
fn in_game_bibite_tab(input: &DynInput<'_>) -> bool {
    input.state.tab == 2 && input.lists.as_ref().is_some_and(|lists| lists.in_sim)
}

/// What separates the rows of a group without a frame: the content's spacing (15 on the Zones and Bibite tabs, 20 on the others).
fn content_spacing(input: &DynInput<'_>) -> f32 {
    if in_game_zones_tab(input) || in_game_bibite_tab(input) {
        15.0
    } else {
        SPACING
    }
}

/// The gap after group `index` of the tab: the content's spacing, and on the Bibite tab the `Bibite Parameters Holders` top padding (20) after the
/// cards and 20 (not 15) between the parameter groups.
fn gap_after_group(input: &DynInput<'_>, index: usize) -> f32 {
    if in_game_bibite_tab(input) {
        if index == 0 { 15.0 + 20.0 } else { 20.0 }
    } else {
        content_spacing(input)
    }
}

/// Lays out `groups` at the scroll offset of `input.state`.
#[must_use]
pub fn layout_list(
    templates: &Templates,
    groups: &[TabGroup],
    input: &DynInput<'_>,
    geometry: &ListGeometry,
) -> ListBuild {
    let viewport_h = geometry.viewport[3] - geometry.viewport[1];
    // First pass without scrolling to measure the content height.
    let measure = |scroll: f32, dry: bool| {
        let mut ctx = Ctx {
            input,
            templates: templates.clone(),
            geometry: *geometry,
            hits: Vec::new(),
            tips: Vec::new(),
            nodes: Vec::new(),
            next_group: 0,
            view_top: geometry.viewport[1],
            view_bottom: geometry.viewport[3],
            dry,
            cuts: input.lists.as_ref().is_some_and(|lists| lists.in_sim),
        };
        let mut y = geometry.viewport[1] + geometry.top_pad - scroll;
        let mut last_gap = 0.0;
        for (index, group) in groups.iter().enumerate() {
            last_gap = gap_after_group(input, index);
            y += if group.title.is_empty() {
                layout_bare(&mut ctx, &group.rows, y)
            } else {
                layout_group(&mut ctx, group.title, &group.rows, y)
            } + last_gap;
        }
        // The last group has no trailing gap; the content has its own bottom padding.
        let content_bottom = y - last_gap + geometry.bottom_pad + scroll;
        (ctx, content_bottom - geometry.viewport[1])
    };
    let (_, content_h) = measure(0.0, true);
    let max_scroll = (content_h - viewport_h).max(0.0);
    input.state.max_scroll.set(max_scroll);
    let scroll = input.state.scroll.clamp(0.0, max_scroll);
    let (ctx, _) = measure(scroll, false);
    ListBuild {
        nodes: ctx.nodes,
        hits: ctx.hits,
        tips: ctx.tips,
        content_h,
        max_scroll,
        scroll,
    }
}

/// Moves the scroll bar handle (`Sliding Area/Handle`) to the scroll position.
pub fn place_scrollbar(root: &mut Node, geometry: &ListGeometry, list: &ListBuild) {
    let viewport_h = geometry.viewport[3] - geometry.viewport[1];
    let scrollbar = geometry.scrollbar;
    let track_h = scrollbar[3] - scrollbar[1];
    let handle_h = if list.content_h > viewport_h {
        (track_h * viewport_h / list.content_h).clamp(30.0, track_h)
    } else {
        track_h
    };
    let travel = track_h - handle_h;
    let handle_y = scrollbar[1]
        + if list.max_scroll > 0.0 {
            travel * list.scroll / list.max_scroll
        } else {
            0.0
        };
    if let Some(handle) = find_mut(root, &|n: &Node| n.path.ends_with("/Sliding Area/Handle")) {
        set_rect(
            handle,
            scrollbar[0],
            handle_y,
            scrollbar[2],
            handle_y + handle_h,
        );
    }
}

/// Replaces the captured rows under `Content` with the generated ones.
pub fn replace_content(root: &mut Node, nodes: Vec<Node>) {
    if let Some(content) = find_mut(root, &|n: &Node| n.name == "Content") {
        content.children = nodes;
    }
}

/// Builds the panel for the app's current tab.
#[must_use]
pub fn build_with_extras(
    template_root: &Node,
    extras_root: Option<&Node>,
    input: &DynInput<'_>,
) -> Option<DynBuild> {
    let mut templates = templates_of(template_root)?;
    if let Some(extras_root) = extras_root {
        templates = templates.with_extras_of(extras_root);
    }
    let groups = tab_groups(input.state.tab);
    let in_sim = input.lists.as_ref().is_some_and(|lists| lists.in_sim);
    let mut geometry = if in_sim && input.state.tab == 4 {
        ANIMATION_GEOMETRY
    } else {
        DYN_GEOMETRY
    };
    let mut list = layout_list(&templates, &groups, input, &geometry);
    // `ScrollRect.verticalScrollbarVisibility = AutoHideAndExpandViewport`: when the content fits, the scroll bar is hidden and the viewport (so the
    // rows) is 25 wider (the Zones tab: 952 against 927).
    let fits = in_sim && list.content_h <= geometry.viewport[3] - geometry.viewport[1] + 0.5;
    if fits && input.state.tab != 4 {
        geometry.content_x[1] += 25.0;
        list = layout_list(&templates, &groups, input, &geometry);
    }
    // The Animation tab: its content is 12 narrower while the scroll bar is there (the cards are 926 wide in the game, 938 without the bar).
    if in_sim && input.state.tab == 4 && !fits {
        geometry.content_x[1] -= 12.0;
        list = layout_list(&templates, &groups, input, &geometry);
    }
    // The Zones tab's `TogglePreview` button sits in the corner of the scrolled content: 40 x 40,
    // a `base_icon` button with the `feedback_icon` on it.
    if input.state.tab == 1 && list.scroll < 0.5 {
        let [vx, vy, _, _] = DYN_GEOMETRY.viewport;
        let rect = [vx, vy, vx + 40.0, vy + 40.0];
        let mut button = sprite_node(TOGGLE_PREVIEW_PATH, "base_icon", rect, [1.0; 4]);
        button.children.push(sprite_node(
            &format!("{TOGGLE_PREVIEW_PATH}/Text"),
            "feedback_icon",
            rect,
            [1.0; 4],
        ));
        button.sibling_index = list.nodes.len() as i32;
        list.nodes.push(button);
        list.hits.push(DynHit {
            rect: [vx, vy, 40.0, 40.0],
            action: UiAction::Invoke(crate::ui::ControlCommand::ToggleZonePreview),
        });
    }

    // Clone the captured chrome (without the captured rows) and add the generated rows.
    let mut root = clone_chrome(template_root);
    // The Animation tab's own panel (`AnimationsPanel`) is the only one whose frame is drawn at 0.5 of the Image multiplier (the table is keyed by the
    // panel's path): a 6 px outline inset by 6 instead of 3 at the edge.
    if in_sim
        && input.state.tab == 4
        && let Some(panel) = find_mut(&mut root, &|n: &Node| n.name == "SimulationSettings")
    {
        panel.path = panel.path.replace("/SimulationSettings", "/AnimationsPanel");
    }
    let (view_top, view_bottom) = (geometry.viewport[1], geometry.viewport[3]);
    let cut = |rect: [f32; 4]| -> Option<[f32; 4]> {
        let (top, bottom) = (rect[1].max(view_top), (rect[1] + rect[3]).min(view_bottom));
        (bottom > top).then_some([rect[0], top, rect[2], bottom - top])
    };
    if in_sim {
        list.hits = std::mem::take(&mut list.hits)
            .into_iter()
            .filter_map(|hit| cut(hit.rect).map(|rect| DynHit { rect, ..hit }))
            .collect();
        list.tips = std::mem::take(&mut list.tips)
            .into_iter()
            .filter_map(|tip| cut(tip.rect).map(|rect| DynTip { rect, ..tip }))
            .collect();
    }
    place_scrollbar(&mut root, &geometry, &list);
    if fits && let Some(bar) = find_mut(&mut root, &|n: &Node| n.name == "Scrollbar Vertical") {
        set_active(bar, false);
    }
    let mut hits = list.hits;
    let mut tips = list.tips;
    replace_content(&mut root, list.nodes);
    // Tab buttons, save and close.
    for (index, x) in TAB_X.iter().enumerate() {
        hits.push(DynHit {
            rect: [*x, TAB_Y, TAB_SIZE, TAB_SIZE],
            action: UiAction::DynTab(index as u8),
        });
    }
    hits.push(DynHit {
        rect: [1785.0, TAB_Y, TAB_SIZE, TAB_SIZE],
        action: UiAction::DynSaveScenario,
    });
    hits.push(DynHit {
        rect: [1850.0, TAB_Y, TAB_SIZE, TAB_SIZE],
        action: UiAction::ClosePanel,
    });
    // The animator creator (`SettingsChangerCreator`) hangs over the tab, outside the scroll view.
    if input.state.tab == 4
        && let Some(lists) = input.lists.as_ref()
        && let Some((nodes, popup_hits, popup_tips)) = animator_cards_ui::creator_panel(lists, input.edit)
    {
        for mut node in nodes {
            node.sibling_index = root.children.len() as i32;
            root.children.push(node);
        }
        hits.extend(popup_hits);
        tips.extend(popup_tips);
    }
    Some(DynBuild { root, hits, tips })
}


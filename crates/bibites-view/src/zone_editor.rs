//! The fields of a zone item of the scenario editor's zones tab (`ZoneSettingsHandle`): which
//! controls it shows for a zone, their ranges and formats (`ZoneSettings.cs`), and how a slider
//! position, toggle or dropdown click changes the zone.

use bibites_core::format::FloatValueFormat;
use bibites_core::settings::{
    MaterialKind, MaterialSetting, MaterialSettings, MovementType, SpawnDistribution,
    ZoneGroupSettings, ZoneSettings,
};

use crate::dyn_panel::SliderMode;

/// Spawn material dropdown (`None`, Plant, Meat).
pub const MATERIAL: u8 = 0;
/// Spawn distribution dropdown.
pub const DISTRIBUTION: u8 = 1;
/// Fertility slider.
pub const FERTILITY: u8 = 2;
/// Biomass density slider.
pub const BIOMASS: u8 = 3;
/// Pellet size slider.
pub const PELLET_SIZE: u8 = 4;
/// Renew biomass toggle.
pub const RENEW: u8 = 5;
/// Movement dropdown.
pub const MOVEMENT: u8 = 6;
/// Target dropdown.
pub const TARGET: u8 = 7;
/// Speed factor slider.
pub const SPEED: u8 = 8;
/// "Size scales with sim" toggle.
pub const SIZE_SCALES: u8 = 9;
/// Position X slider.
pub const POS_X: u8 = 10;
/// Position Y slider.
pub const POS_Y: u8 = 11;
/// Radius slider (discs and rings).
pub const RADIUS: u8 = 12;
/// Width slider (rectangles).
pub const WIDTH: u8 = 13;
/// Height slider (rectangles).
pub const HEIGHT: u8 = 14;
/// Inside radius slider (rings).
pub const INSIDE: u8 = 15;

/// A zone index of `FieldRef::Zone` / `UiAction::Zone*` / `DropdownTarget::Zone` that is `TEMPLATE_BASE` or more addresses the template zone of
/// the zone group `index - TEMPLATE_BASE` (its fields are a zone's); `GROUP_BASE` or more addresses the group itself (the fields below).
pub const TEMPLATE_BASE: u16 = 0x8000;
/// See [`TEMPLATE_BASE`].
pub const GROUP_BASE: u16 = 0xC000;
/// `ANIMATOR_BASE` or more addresses settings animator `index - ANIMATOR_BASE` (the Animation tab's cards reuse the zone controls' actions).
pub const ANIMATOR_BASE: u16 = 0xE000;
/// The animator creator panel (its sliders are addressed by the creator's field ids).
pub const CREATOR_REF: u16 = 0xFFFF;
/// RUSTibites Improved: `STREAM_BASE` up to the creator's number addresses stream `index - STREAM_BASE` of the Zones tab's Streams list (the cards reuse the zone sliders'
/// actions). It is above the animators', so no animator list is long enough to reach it; the app asks [`stream_index`] before it asks [`zone_ref`].
pub const STREAM_BASE: u16 = 0xF000;
/// A stream's start x.
pub const STREAM_START_X: u8 = 40;
/// A stream's start y.
pub const STREAM_START_Y: u8 = 41;
/// A stream's finish x.
pub const STREAM_END_X: u8 = 42;
/// A stream's finish y.
pub const STREAM_END_Y: u8 = 43;
/// A stream's width.
pub const STREAM_WIDTH: u8 = 44;
/// A stream's strength.
pub const STREAM_STRENGTH: u8 = 45;
/// An animator card's Start Time slider.
pub const ANIM_START: u8 = 30;
/// An animator card's Repeat toggle.
pub const ANIM_REPEAT: u8 = 31;
/// The first base item's slider: base `k` is field `ANIM_BASE_FIRST + k`.
pub const ANIM_BASE_FIRST: u8 = 64;
/// The selected curve point's time (a typed value of the point editor).
pub const ANIM_POINT_TIME: u8 = 32;
/// The selected curve point's value.
pub const ANIM_POINT_VALUE: u8 = 33;
/// The selected curve point's easing slider.
pub const ANIM_POINT_EASE: u8 = 34;
/// "Count scales with sim" (`ZoneGroupSettings.scaleWithSim`).
pub const GROUP_SCALE: u8 = 20;
/// "Zones Count" slider.
pub const GROUP_COUNT: u8 = 21;
/// "Zones Density" slider.
pub const GROUP_DENSITY: u8 = 22;

/// What a zone index of an action addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoneRef {
    /// Zone `.0` of the list.
    Zone(usize),
    /// The template of zone group `.0`.
    Template(usize),
    /// Zone group `.0` itself.
    Group(usize),
    /// Settings animator `.0`.
    Animator(usize),
    /// The animator creator panel.
    Creator,
}

/// What zone index `index` of an action addresses.
#[must_use]
pub fn zone_ref(index: u16) -> ZoneRef {
    if index == CREATOR_REF {
        ZoneRef::Creator
    } else if index >= ANIMATOR_BASE {
        ZoneRef::Animator(usize::from(index - ANIMATOR_BASE))
    } else if index >= GROUP_BASE {
        ZoneRef::Group(usize::from(index - GROUP_BASE))
    } else if index >= TEMPLATE_BASE {
        ZoneRef::Template(usize::from(index - TEMPLATE_BASE))
    } else {
        ZoneRef::Zone(usize::from(index))
    }
}

/// The zone index that addresses the template of group `group`.
#[must_use]
pub fn template_ref(group: usize) -> u16 {
    TEMPLATE_BASE + group as u16
}

/// The zone index that addresses group `group` itself.
#[must_use]
pub fn group_ref(group: usize) -> u16 {
    GROUP_BASE + group as u16
}

/// The zone index that addresses settings animator `changer`.
#[must_use]
pub fn animator_ref(changer: usize) -> u16 {
    ANIMATOR_BASE + changer as u16
}

/// The zone index that addresses stream `stream`.
#[must_use]
pub fn stream_ref(stream: usize) -> u16 {
    STREAM_BASE + (stream as u16).min(CREATOR_REF - STREAM_BASE - 1)
}

/// The stream a zone index of an action addresses, `None` when it addresses something else.
#[must_use]
pub fn stream_index(index: u16) -> Option<usize> {
    (STREAM_BASE..CREATOR_REF).contains(&index).then(|| usize::from(index - STREAM_BASE))
}

/// The label of a stream's slider.
#[must_use]
pub fn stream_name_of(field: u8) -> &'static str {
    match field {
        STREAM_START_X => "Start X",
        STREAM_START_Y => "Start Y",
        STREAM_END_X => "End X",
        STREAM_END_Y => "End Y",
        STREAM_WIDTH => "Width",
        STREAM_STRENGTH => "Strength",
        _ => "",
    }
}

/// The slider of a stream's field: the coordinates run over the whole world (`-sim_size` to `sim_size`), the width over 10 to 3000 u on a log scale, the strength over 0 to
/// 500 u/s per second.
#[must_use]
pub fn stream_slider_info(field: u8, sim_size: f32) -> Option<SliderInfo> {
    use bibites_core::stream::{STREAM_STRENGTH_DEFAULT, STREAM_STRENGTH_MAX, STREAM_STRENGTH_MIN, STREAM_WIDTH_DEFAULT, STREAM_WIDTH_MAX, STREAM_WIDTH_MIN};
    Some(match field {
        STREAM_START_X | STREAM_START_Y | STREAM_END_X | STREAM_END_Y => SliderInfo {
            min: -sim_size,
            max: sim_size,
            default: 0.0,
            mode: SliderMode::Linear,
            units: " u",
            precision: 0,
            si: false,
            factor: 1.0,
        },
        STREAM_WIDTH => SliderInfo {
            min: STREAM_WIDTH_MIN,
            max: STREAM_WIDTH_MAX,
            default: STREAM_WIDTH_DEFAULT,
            mode: SliderMode::Log { base: 10.0, span: None },
            units: " u",
            precision: 0,
            si: false,
            factor: 1.0,
        },
        STREAM_STRENGTH => SliderInfo {
            min: STREAM_STRENGTH_MIN,
            max: STREAM_STRENGTH_MAX,
            default: STREAM_STRENGTH_DEFAULT,
            mode: SliderMode::Linear,
            units: " u/s\u{b2}",
            precision: 0,
            si: false,
            factor: 1.0,
        },
        _ => return None,
    })
}

/// The current value of a stream's field.
#[must_use]
pub fn stream_value_of(stream: &bibites_core::stream::Stream, field: u8) -> f32 {
    match field {
        STREAM_START_X => stream.start.x,
        STREAM_START_Y => stream.start.y,
        STREAM_END_X => stream.end.x,
        STREAM_END_Y => stream.end.y,
        STREAM_WIDTH => stream.width,
        STREAM_STRENGTH => stream.strength,
        _ => 0.0,
    }
}

/// Writes a stream's field (every value is kept inside its range: the world for a coordinate, the stream's own limits for the width and the strength).
pub fn set_stream_value(stream: &mut bibites_core::stream::Stream, field: u8, value: f32, sim_size: f32) {
    if !value.is_finite() {
        return;
    }
    let coordinate = value.clamp(-sim_size, sim_size);
    match field {
        STREAM_START_X => stream.set_start(bibites_core::math::Vec2::new(coordinate, stream.start.y)),
        STREAM_START_Y => stream.set_start(bibites_core::math::Vec2::new(stream.start.x, coordinate)),
        STREAM_END_X => stream.set_end(bibites_core::math::Vec2::new(coordinate, stream.end.y)),
        STREAM_END_Y => stream.set_end(bibites_core::math::Vec2::new(stream.end.x, coordinate)),
        STREAM_WIDTH => stream.set_width(value),
        STREAM_STRENGTH => stream.set_strength(value),
        _ => {}
    }
}

/// Slider description of a zone group's count or density (`LogIntSettingSlider(10)` over 1 to 1000, `LogFloatSettingSlider(10)` over 0.01 to 50).
#[must_use]
pub fn group_slider_info(field: u8) -> Option<SliderInfo> {
    let log = SliderMode::Log {
        base: 10.0,
        span: None,
    };
    Some(match field {
        GROUP_COUNT => SliderInfo {
            min: 1.0,
            max: 1000.0,
            default: 10.0,
            mode: log,
            units: "",
            precision: 0,
            si: false,
            factor: 1.0,
        },
        GROUP_DENSITY => SliderInfo {
            min: 0.01,
            max: 50.0,
            default: 0.66,
            mode: log,
            units: " z/ku\u{b2}",
            precision: 2,
            si: false,
            factor: 1.0,
        },
        _ => return None,
    })
}

/// Current value of a group's slider field.
#[must_use]
pub fn group_value_of(group: &ZoneGroupSettings, field: u8) -> f32 {
    match field {
        GROUP_COUNT => group.zone_count as f32,
        GROUP_DENSITY => group.zone_density,
        _ => 0.0,
    }
}

/// Writes a group's slider value as it is (a typed value may go past the slider's ends; the count is a whole number).
pub fn set_group_value_unclamped(group: &mut ZoneGroupSettings, field: u8, value: f32) {
    match field {
        GROUP_COUNT => group.zone_count = value.round() as i32,
        GROUP_DENSITY => group.zone_density = value,
        _ => {}
    }
}

/// Writes a group's slider value, kept inside the slider's range.
pub fn set_group_value(group: &mut ZoneGroupSettings, field: u8, value: f32) {
    if let Some(info) = group_slider_info(field) {
        set_group_value_unclamped(group, field, value.clamp(info.min, info.max));
    }
}

/// How a field is edited.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FieldKind {
    /// A slider.
    Slider,
    /// A checkbox.
    Toggle,
    /// A dropdown (clicking advances to the next choice).
    Choice,
}

/// The controls shown for `zone`, in order (`ZoneSettingsHandle.ShowCorrectSizeSliders`,
/// `ChangePelletType`, `ChangeMovementType`).
#[must_use]
pub fn fields_of(zone: &ZoneSettings) -> Vec<u8> {
    let mut fields = vec![MATERIAL];
    if zone.material.is_some() {
        fields.extend([DISTRIBUTION, FERTILITY, BIOMASS, PELLET_SIZE, RENEW]);
    }
    fields.push(MOVEMENT);
    match zone.movement {
        MovementType::Free => fields.push(SPEED),
        MovementType::Orbit | MovementType::Attached => fields.push(TARGET),
        _ => {}
    }
    fields.extend([SIZE_SCALES, POS_X, POS_Y]);
    if zone.distribution == SpawnDistribution::Rect {
        fields.extend([WIDTH, HEIGHT]);
    } else {
        fields.push(RADIUS);
        if matches!(
            zone.distribution,
            SpawnDistribution::Ring | SpawnDistribution::FlatRing
        ) {
            fields.push(INSIDE);
        }
    }
    fields
}

/// Kind of a field.
#[must_use]
pub fn kind_of(field: u8) -> FieldKind {
    match field {
        MATERIAL | DISTRIBUTION | MOVEMENT | TARGET => FieldKind::Choice,
        RENEW | SIZE_SCALES => FieldKind::Toggle,
        _ => FieldKind::Slider,
    }
}

/// Label of a field.
#[must_use]
pub fn name_of(field: u8) -> &'static str {
    match field {
        MATERIAL => "Spawn Pellet of Type",
        DISTRIBUTION => "Spawn distribution",
        FERTILITY => "Fertility",
        BIOMASS => "Biomass ",
        PELLET_SIZE => "Pellet Size",
        RENEW => "Renew Biomass",
        MOVEMENT => "Movement",
        TARGET => "Target",
        SPEED => "Speed Factor",
        SIZE_SCALES => "Size scales with sim",
        POS_X => "Pos X",
        POS_Y => "Pos Y",
        RADIUS => "Radius",
        WIDTH => "Width",
        HEIGHT => "Height",
        INSIDE => "Inside radius",
        _ => "",
    }
}

/// Slider description of a field: range, default, mode and number format.
#[derive(Clone, Copy, Debug)]
pub struct SliderInfo {
    /// Smallest value.
    pub min: f32,
    /// Largest value.
    pub max: f32,
    /// `DefaultValue`.
    pub default: f32,
    /// Position mapping.
    pub mode: SliderMode,
    /// Value units.
    pub units: &'static str,
    /// Digits.
    pub precision: i32,
    /// SI prefixes.
    pub si: bool,
    /// Display factor (applied when `si` is off).
    pub factor: f32,
}

impl SliderInfo {
    /// The value format (`LogSettingSlider` shows significant digits).
    #[must_use]
    pub fn format(&self) -> FloatValueFormat {
        FloatValueFormat {
            prefix: "",
            units: self.units,
            precision: self.precision,
            factor: self.factor,
            si: self.si,
            always_show_sign: false,
            // The fertility, biomass and size sliders are log sliders whose text keeps two decimals whatever the magnitude (the game's
            // template card shows "10.00x"); the others show significant digits.
            precision_is_si: matches!(self.mode, SliderMode::Log { .. }) && !(self.units == "x" && self.precision == 2 && !self.si),
            is_int: false,
        }
    }
}

/// Slider description of `field` for `zone`, `None` for the other kinds.
#[must_use]
pub fn slider_info(zone: &ZoneSettings, field: u8, sim_size: f32) -> Option<SliderInfo> {
    let log = |span: Option<f32>| SliderMode::Log { base: 10.0, span };
    let relative = zone.radius_is_relative;
    Some(match field {
        FERTILITY | BIOMASS => SliderInfo {
            min: 0.001,
            max: 100.0,
            default: 1.0,
            mode: log(None),
            units: "x",
            precision: 2,
            si: false,
            factor: 1.0,
        },
        PELLET_SIZE => SliderInfo {
            min: 0.01,
            max: 50.0,
            default: 1.0,
            mode: log(None),
            units: "x",
            precision: 2,
            si: false,
            factor: 1.0,
        },
        SPEED => SliderInfo {
            min: 0.001,
            max: 1000.0,
            default: 1.0,
            mode: log(None),
            units: "",
            precision: 2,
            si: true,
            factor: 1.0,
        },
        POS_X | POS_Y => SliderInfo {
            min: -1.0,
            max: 1.0,
            default: 0.0,
            mode: SliderMode::Linear,
            units: "",
            precision: 3,
            si: false,
            factor: 1.0,
        },
        RADIUS if relative => SliderInfo {
            min: 0.0,
            max: 1.0,
            default: 0.25,
            mode: log(Some(3.0)),
            units: "x",
            precision: 3,
            si: false,
            factor: 1.0,
        },
        RADIUS => SliderInfo {
            min: 0.0,
            max: sim_size,
            default: 1000.0,
            mode: log(Some(3.0)),
            units: "u",
            precision: 2,
            si: true,
            factor: 1.0,
        },
        WIDTH | HEIGHT if relative => SliderInfo {
            min: 0.0,
            max: 2.0,
            default: if field == WIDTH { 0.25 } else { 0.5 },
            mode: log(Some(2.0)),
            units: "x",
            precision: 3,
            si: false,
            factor: 1.0,
        },
        WIDTH | HEIGHT => SliderInfo {
            min: 0.0,
            max: sim_size * 2.0,
            default: 1000.0,
            mode: log(Some(2.0)),
            units: "u",
            precision: 2,
            si: true,
            factor: 1.0,
        },
        INSIDE => SliderInfo {
            min: 0.0,
            max: 0.999,
            default: 0.8,
            mode: SliderMode::Linear,
            units: "x",
            precision: 2,
            si: false,
            factor: 1.0,
        },
        _ => return None,
    })
}

/// Current numeric value of a slider field.
#[must_use]
pub fn value_of(zone: &ZoneSettings, field: u8) -> f32 {
    let relative = zone.radius_is_relative;
    match field {
        FERTILITY => zone.fertility,
        BIOMASS => zone.biomass_density,
        PELLET_SIZE => zone.pellet_size,
        SPEED => zone.speed,
        POS_X => zone.pos_x,
        POS_Y => zone.pos_y,
        RADIUS if relative => zone.radius_relative,
        RADIUS => zone.radius_absolute,
        WIDTH if relative => zone.width_relative,
        WIDTH => zone.width_absolute,
        HEIGHT if relative => zone.height_relative,
        HEIGHT => zone.height_absolute,
        INSIDE => zone.inside_radius,
        _ => 0.0,
    }
}

/// Writes a slider value (clamped to the field's range; positions stay inside the world like
/// `ZoneSettings.CapPosition`).
pub fn set_value(zone: &mut ZoneSettings, field: u8, value: f32, sim_size: f32) {
    let Some(info) = slider_info(zone, field, sim_size) else {
        return;
    };
    set_value_unclamped(zone, field, value.clamp(info.min, info.max), sim_size);
}

/// Writes a slider value as it is, past the slider's ends too: a typed value of a setting that
/// `canGoOutOfBounds` (the zone settings all do).
pub fn set_value_unclamped(zone: &mut ZoneSettings, field: u8, value: f32, sim_size: f32) {
    let relative = zone.radius_is_relative;
    match field {
        FERTILITY => zone.fertility = value,
        BIOMASS => zone.biomass_density = value,
        PELLET_SIZE => zone.pellet_size = value,
        SPEED => zone.speed = value,
        POS_X => zone.pos_x = value,
        POS_Y => zone.pos_y = value,
        RADIUS if relative => zone.radius_relative = value,
        RADIUS => zone.radius_absolute = value,
        WIDTH if relative => zone.width_relative = value,
        WIDTH => zone.width_absolute = value,
        HEIGHT if relative => zone.height_relative = value,
        HEIGHT => zone.height_absolute = value,
        INSIDE => zone.inside_radius = value,
        _ => {}
    }
    cap_position(zone, sim_size);
}

/// The C# field of a zone's setting behind field `field`, whose name and help text its row's
/// tooltip shows (the size fields have one for each size type; the position has none).
#[must_use]
pub fn help_key(zone: &ZoneSettings, field: u8) -> Option<&'static str> {
    let relative = zone.radius_is_relative;
    Some(match field {
        MATERIAL => "spawnMaterial",
        DISTRIBUTION => "distribution",
        FERTILITY => "fertility",
        BIOMASS => "biomassDensity",
        PELLET_SIZE => "pelletSize",
        RENEW => "renewBiomass",
        MOVEMENT => "movement",
        TARGET => "target",
        SPEED => "speed",
        SIZE_SCALES => "sizeScalesWithSim",
        RADIUS if relative => "radiusRelative",
        RADIUS => "radiusAbsolute",
        WIDTH if relative => "widthRelative",
        WIDTH => "widthAbsolute",
        HEIGHT if relative => "heightRelative",
        HEIGHT => "heightAbsolute",
        INSIDE => "insideRadius",
        _ => return None,
    })
}

/// The C# field of a material's setting behind field `field` (the Energy tab's blocks).
#[must_use]
pub fn material_help_key(field: u8) -> Option<&'static str> {
    Some(match field {
        MAT_ENERGY => "energyDensity",
        MAT_MASS => "massDensity",
        MAT_COHESION => "cohesiveness",
        MAT_REACTIVITY => "reactivity",
        MAT_MAX_EFFICIENCY => "maxEfficiency",
        MAT_MIN_EFFICIENCY => "minEfficiency",
        MAT_DECAY => "decay",
        MAT_FRESH_TIME => "freshTime",
        MAT_DECAY_RATE => "decayRate",
        _ => return None,
    })
}

/// `ZoneSettings.CapPosition`: the zone stays inside the world.
pub fn cap_position(zone: &mut ZoneSettings, sim_size: f32) {
    let config = bibites_core::zone::ZoneConfig::from_settings(zone);
    let (cap_x, cap_y) = if config.is_rect() {
        (
            (1.0 - config.relative_width(sim_size) / 2.0).clamp(0.0, 1.0),
            (1.0 - config.relative_height(sim_size) / 2.0).clamp(0.0, 1.0),
        )
    } else {
        let cap = (1.0 - config.relative_radius(sim_size)).clamp(0.0, 1.0);
        (cap, cap)
    };
    zone.pos_x = zone.pos_x.clamp(-cap_x, cap_x);
    zone.pos_y = zone.pos_y.clamp(-cap_y, cap_y);
}

/// Current state of a toggle field.
#[must_use]
pub fn toggle_value(zone: &ZoneSettings, field: u8) -> bool {
    match field {
        RENEW => zone.renew_biomass,
        SIZE_SCALES => zone.radius_is_relative,
        _ => false,
    }
}

/// Flips a toggle field.
pub fn toggle(zone: &mut ZoneSettings, field: u8) {
    match field {
        RENEW => zone.renew_biomass = !zone.renew_biomass,
        SIZE_SCALES => zone.radius_is_relative = !zone.radius_is_relative,
        _ => {}
    }
}

/// The label of a dropdown field's current choice (`zones`: every zone, for the target names).
#[must_use]
pub fn choice_label(zone: &ZoneSettings, field: u8, zones: &[ZoneSettings]) -> String {
    match field {
        MATERIAL => match zone.material {
            None => "None",
            Some(MaterialKind::Plant) => "Plant",
            Some(MaterialKind::Meat) => "Meat",
            Some(MaterialKind::Fat) => "Fat",
            Some(MaterialKind::Armor) => "Armor",
        }
        .to_owned(),
        DISTRIBUTION => match zone.distribution {
            SpawnDistribution::Flat => "Flat Circle",
            SpawnDistribution::CentricGradual => "Center to edge Circle",
            SpawnDistribution::ExteriorGradual => "Edge to center Circle",
            SpawnDistribution::Ring => "Ring",
            SpawnDistribution::FlatRing => "Flat Ring",
            SpawnDistribution::Rect => "Rectangular",
        }
        .to_owned(),
        MOVEMENT => match zone.movement {
            MovementType::None => "None",
            MovementType::Free => "Free",
            _ => "Locked to Target",
        }
        .to_owned(),
        TARGET => zone
            .target
            .and_then(|id| {
                zones
                    .iter()
                    .position(|other| other.zone_id == Some(id))
                    .map(|index| zones[index].name.clone())
            })
            .unwrap_or_else(|| "None".to_owned()),
        _ => String::new(),
    }
}

/// The spawn distributions in the game's dropdown order (`ZoneSettings.distributionChoices`).
const DISTRIBUTIONS: [(SpawnDistribution, &str, &str); 6] = [
    (
        SpawnDistribution::Flat,
        "Flat Circle",
        "Pellets will have an equal probability to spawn anywhere in the zone",
    ),
    (
        SpawnDistribution::CentricGradual,
        "Center to edge Circle",
        "Pellets have a higher probability to spawn closer to the center, with a probability of 0 at the edges.",
    ),
    (
        SpawnDistribution::ExteriorGradual,
        "Edge to center Circle",
        "Pellets have a higher probability to spawn closer to the edge, with a probability of 0 at the center.",
    ),
    (
        SpawnDistribution::Ring,
        "Ring",
        "Pellets will spawn in a ring, with a pellet density of 0 at the ring's edges",
    ),
    (
        SpawnDistribution::FlatRing,
        "Flat Ring",
        "Pellets will spawn in a ring, with a pellet density even across the ring",
    ),
    (
        SpawnDistribution::Rect,
        "Rectangular",
        "Pellets will spawn in a rectangular inside a rectangle, with a flat distribution across the rectangle",
    ),
];

/// The movement types in the game's dropdown order (`ZoneSettings.movementChoices`: the game lists
/// three of its five movement types).
const MOVEMENTS: [(MovementType, &str, &str); 3] = [
    (
        MovementType::None,
        "None",
        "The Zone will be fixed and not move.",
    ),
    (
        MovementType::Free,
        "Free",
        "The Zone will freely move around",
    ),
    (
        MovementType::Attached,
        "Locked to Target",
        "The Zone will be locked to the target Zone and move wih it.",
    ),
];

/// The spawn materials in the game's dropdown order: "None", then the physical materials
/// (`MatterMaterialManager.PhysicalMaterials`: plant and meat).
const MATERIALS: [(Option<MaterialKind>, &str); 3] = [
    (None, "None"),
    (Some(MaterialKind::Plant), "Plant"),
    (Some(MaterialKind::Meat), "Meat"),
];

/// The options of a dropdown field, in the game's order (`zones`: every zone, whose names the
/// target dropdown lists after "None" - `TargetZoneDropdown.PopulateList`, the zone's own included).
#[must_use]
pub fn choices(field: u8, zones: &[ZoneSettings]) -> Vec<crate::dropdown::Choice> {
    use crate::dropdown::Choice;
    match field {
        MATERIAL => MATERIALS
            .iter()
            .map(|(_, label)| Choice::plain(*label))
            .collect(),
        // `ChoiceSettingDropdown.OnItemHovered`: "Setting name: Choice" over the choice's text.
        DISTRIBUTION => DISTRIBUTIONS
            .iter()
            .map(|&(_, label, tip)| Choice::of_setting("Spawn distribution", label, tip))
            .collect(),
        MOVEMENT => MOVEMENTS
            .iter()
            .map(|&(_, label, tip)| Choice::of_setting("Movement", label, tip))
            .collect(),
        TARGET => std::iter::once(Choice::plain("None"))
            .chain(zones.iter().map(|zone| Choice::plain(zone.name.clone())))
            .collect(),
        _ => Vec::new(),
    }
}

/// Index of the zone's current choice of a dropdown field in [`choices`].
#[must_use]
pub fn current_choice(zone: &ZoneSettings, field: u8, zones: &[ZoneSettings]) -> usize {
    match field {
        MATERIAL => MATERIALS
            .iter()
            .position(|(material, _)| *material == zone.material)
            .unwrap_or(0),
        DISTRIBUTION => DISTRIBUTIONS
            .iter()
            .position(|(distribution, ..)| *distribution == zone.distribution)
            .unwrap_or(0),
        MOVEMENT => MOVEMENTS
            .iter()
            .position(|(movement, ..)| *movement == zone.movement)
            // `Orbit` and `Repulsed` zones (from a save) show the last entry, as the label does.
            .unwrap_or(MOVEMENTS.len() - 1),
        TARGET => zone
            .target
            .and_then(|id| zones.iter().position(|other| other.zone_id == Some(id)))
            .map_or(0, |at| at + 1),
        _ => 0,
    }
}

/// Sets a dropdown field to option `index` of [`choices`] (a pick in its list).
pub fn set_choice(zone: &mut ZoneSettings, field: u8, index: usize, zones: &[ZoneSettings]) {
    match field {
        MATERIAL => {
            if let Some((material, _)) = MATERIALS.get(index) {
                zone.material = *material;
            }
        }
        DISTRIBUTION => {
            if let Some((distribution, ..)) = DISTRIBUTIONS.get(index) {
                zone.distribution = *distribution;
            }
        }
        MOVEMENT => {
            if let Some((movement, ..)) = MOVEMENTS.get(index) {
                zone.movement = *movement;
                // `ChangeMovementType`: only attached zones keep a target.
                if !matches!(zone.movement, MovementType::Attached | MovementType::Orbit) {
                    zone.target = None;
                }
            }
        }
        TARGET => {
            zone.target = if index == 0 {
                None
            } else {
                zones.get(index - 1).and_then(|other| other.zone_id)
            };
        }
        _ => {}
    }
}

/// Advances a dropdown field to its next choice (`zones`: every zone, for the target).
pub fn cycle(zone: &mut ZoneSettings, field: u8, zones: &[ZoneSettings], own_index: usize) {
    let _ = own_index;
    let count = choices(field, zones).len();
    if count == 0 {
        return;
    }
    let next = (current_choice(zone, field, zones) + 1) % count;
    set_choice(zone, field, next, zones);
}

/// Resets a field to the game's default (`SettingHandle.ResetValue`).
pub fn reset(zone: &mut ZoneSettings, field: u8, sim_size: f32) {
    match kind_of(field) {
        FieldKind::Slider => {
            if let Some(info) = slider_info(zone, field, sim_size) {
                set_value(zone, field, info.default, sim_size);
            }
        }
        FieldKind::Toggle => match field {
            RENEW => zone.renew_biomass = false,
            _ => zone.radius_is_relative = true,
        },
        FieldKind::Choice => match field {
            MATERIAL => zone.material = Some(MaterialKind::Plant),
            DISTRIBUTION => zone.distribution = SpawnDistribution::CentricGradual,
            MOVEMENT => {
                zone.movement = MovementType::None;
                zone.target = None;
            }
            _ => zone.target = None,
        },
    }
}

// ---------------------------------------------------------------------------
// Material parameters (`MatterMaterialSettings`, the Energy tab)
// ---------------------------------------------------------------------------

/// Energy density slider.
pub const MAT_ENERGY: u8 = 0;
/// Mass density slider.
pub const MAT_MASS: u8 = 1;
/// Cohesiveness slider.
pub const MAT_COHESION: u8 = 2;
/// Base reactivity slider.
pub const MAT_REACTIVITY: u8 = 3;
/// Maximum conversion efficiency slider.
pub const MAT_MAX_EFFICIENCY: u8 = 4;
/// Minimum conversion efficiency slider.
pub const MAT_MIN_EFFICIENCY: u8 = 5;
/// Decay toggle (master of the decay parameters).
pub const MAT_DECAY: u8 = 6;
/// Freshness time slider.
pub const MAT_FRESH_TIME: u8 = 7;
/// Decay rate slider.
pub const MAT_DECAY_RATE: u8 = 8;

/// The material of kind index `kind` (Plant, Meat, Fat, Armor).
#[must_use]
pub fn material_kind(kind: u8) -> MaterialKind {
    MaterialKind::ALL[usize::from(kind).min(3)]
}

/// Label of a material field.
#[must_use]
pub fn material_name_of(field: u8) -> &'static str {
    match field {
        MAT_ENERGY => "Energy Density",
        MAT_MASS => "Mass Density",
        MAT_COHESION => "Cohesiveness",
        MAT_REACTIVITY => "Base Reactivity",
        MAT_MAX_EFFICIENCY => "Max Conversion Efficiency",
        MAT_MIN_EFFICIENCY => "Min Conversion Efficiency",
        MAT_DECAY => "Decay",
        MAT_FRESH_TIME => "Freshness Time",
        MAT_DECAY_RATE => "Decay Rate",
        _ => "",
    }
}

/// Current value of a material field (`1.0` / `0.0` for the decay toggle).
#[must_use]
pub fn material_value(material: &MaterialSetting, field: u8) -> f32 {
    match field {
        MAT_ENERGY => material.energy_density,
        MAT_MASS => material.mass_density,
        MAT_COHESION => material.cohesiveness,
        MAT_REACTIVITY => material.reactivity,
        MAT_MAX_EFFICIENCY => material.max_efficiency,
        MAT_MIN_EFFICIENCY => material.min_efficiency,
        MAT_DECAY => f32::from(u8::from(material.decay)),
        MAT_FRESH_TIME => material.fresh_time,
        MAT_DECAY_RATE => material.decay_rate,
        _ => 0.0,
    }
}

/// Slider description of a material field for `kind` (defaults differ per material).
#[must_use]
pub fn material_slider_info(kind: MaterialKind, field: u8) -> Option<SliderInfo> {
    let default = material_value(MaterialSettings::game_defaults().get(kind), field);
    let log = |base: f32, span: Option<f32>| SliderMode::Log { base, span };
    let info = |min, max, mode, units, precision, si, factor| SliderInfo {
        min,
        max,
        default,
        mode,
        units,
        precision,
        si,
        factor,
    };
    Some(match field {
        MAT_ENERGY => info(0.01, 100.0, log(10.0, None), "E/u\u{b2}", 1, true, 1.0),
        MAT_MASS => info(0.001, 1.0, log(10.0, None), "g/u\u{b2}", 1, true, 1000.0),
        MAT_COHESION => info(0.0, 1000.0, log(10.0, Some(4.0)), " N/u", 1, false, 1.0),
        MAT_REACTIVITY => info(0.0, 10.0, log(10.0, Some(4.0)), "u/s", 2, true, 1.0),
        MAT_MAX_EFFICIENCY | MAT_MIN_EFFICIENCY => {
            info(-1.0, 1.0, SliderMode::Linear, "%", 1, false, 100.0)
        }
        MAT_FRESH_TIME => info(0.0, 3600.0, log(6.0, Some(5.0)), "s", 1, false, 1.0),
        MAT_DECAY_RATE => info(0.001, 100.0, log(10.0, None), "u\u{b2}/s", 1, true, 1.0),
        _ => return None,
    })
}

/// Writes a material slider value (clamped to its range).
pub fn set_material_value(
    material: &mut MaterialSetting,
    kind: MaterialKind,
    field: u8,
    value: f32,
) {
    let Some(info) = material_slider_info(kind, field) else {
        return;
    };
    set_material_value_unclamped(material, field, value.clamp(info.min, info.max));
}

/// Writes a material slider value as it is, past the slider's ends too (a typed value).
pub fn set_material_value_unclamped(material: &mut MaterialSetting, field: u8, value: f32) {
    match field {
        MAT_ENERGY => material.energy_density = value,
        MAT_MASS => material.mass_density = value,
        MAT_COHESION => material.cohesiveness = value,
        MAT_REACTIVITY => material.reactivity = value,
        MAT_MAX_EFFICIENCY => material.max_efficiency = value,
        MAT_MIN_EFFICIENCY => material.min_efficiency = value,
        MAT_FRESH_TIME => material.fresh_time = value,
        MAT_DECAY_RATE => material.decay_rate = value,
        _ => {}
    }
}

/// Resets one field of a material to its default.
pub fn reset_material_field(material: &mut MaterialSetting, kind: MaterialKind, field: u8) {
    let defaults = *MaterialSettings::game_defaults().get(kind);
    match field {
        MAT_ENERGY => material.energy_density = defaults.energy_density,
        MAT_MASS => material.mass_density = defaults.mass_density,
        MAT_COHESION => material.cohesiveness = defaults.cohesiveness,
        MAT_REACTIVITY => material.reactivity = defaults.reactivity,
        MAT_MAX_EFFICIENCY => material.max_efficiency = defaults.max_efficiency,
        MAT_MIN_EFFICIENCY => material.min_efficiency = defaults.min_efficiency,
        MAT_DECAY => material.decay = defaults.decay,
        MAT_FRESH_TIME => material.fresh_time = defaults.fresh_time,
        MAT_DECAY_RATE => material.decay_rate = defaults.decay_rate,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_index_is_told_apart_from_the_zones_groups_animators_and_the_creator() {
        for (index, expected) in [(0, None), (TEMPLATE_BASE, None), (GROUP_BASE + 3, None), (ANIMATOR_BASE + 7, None), (CREATOR_REF, None), (STREAM_BASE, Some(0)), (STREAM_BASE + 5, Some(5))] {
            assert_eq!(stream_index(index), expected, "{index:#x}");
        }
        for stream in [0, 1, 40] {
            assert_eq!(stream_index(stream_ref(stream)), Some(stream));
        }
        assert!(STREAM_BASE > ANIMATOR_BASE && stream_ref(usize::MAX / 2) < CREATOR_REF, "a huge number does not become the creator's");
    }

    #[test]
    fn a_streams_sliders_cover_the_world_and_the_values_written_stay_in_range() {
        use bibites_core::math::Vec2;
        use bibites_core::stream::{STREAM_STRENGTH_MAX, STREAM_WIDTH_MAX, STREAM_WIDTH_MIN, Stream};
        let sim = 2000.0;
        for field in [STREAM_START_X, STREAM_START_Y, STREAM_END_X, STREAM_END_Y, STREAM_WIDTH, STREAM_STRENGTH] {
            let info = stream_slider_info(field, sim).expect("a slider");
            assert!(info.min < info.max && (info.min..=info.max).contains(&info.default), "field {field}");
            assert!(!stream_name_of(field).is_empty());
        }
        assert!(stream_slider_info(0, sim).is_none() && stream_slider_info(STREAM_STRENGTH + 1, sim).is_none());
        let coordinate = stream_slider_info(STREAM_START_X, sim).unwrap();
        assert_eq!((coordinate.min, coordinate.max), (-sim, sim));
        let mut stream = Stream::new(Vec2::new(-100.0, 0.0), Vec2::new(100.0, 0.0));
        set_stream_value(&mut stream, STREAM_START_X, 250.0, sim);
        set_stream_value(&mut stream, STREAM_START_Y, -75.0, sim);
        set_stream_value(&mut stream, STREAM_END_X, 1.0e9, sim);
        set_stream_value(&mut stream, STREAM_END_Y, -1.0e9, sim);
        assert_eq!((stream.start, stream.end), (Vec2::new(250.0, -75.0), Vec2::new(sim, -sim)));
        set_stream_value(&mut stream, STREAM_WIDTH, 1.0e9, sim);
        assert_eq!(stream.width, STREAM_WIDTH_MAX);
        set_stream_value(&mut stream, STREAM_WIDTH, 0.0, sim);
        assert_eq!(stream.width, STREAM_WIDTH_MIN);
        set_stream_value(&mut stream, STREAM_STRENGTH, 1.0e9, sim);
        assert_eq!(stream.strength, STREAM_STRENGTH_MAX);
        let before = stream;
        set_stream_value(&mut stream, STREAM_STRENGTH, f32::NAN, sim);
        set_stream_value(&mut stream, STREAM_START_X, f32::INFINITY, sim);
        assert_eq!(stream, before, "not a number: unchanged");
        for field in [STREAM_START_X, STREAM_START_Y, STREAM_END_X, STREAM_END_Y, STREAM_WIDTH, STREAM_STRENGTH] {
            let value = stream_value_of(&stream, field);
            let mut copy = stream;
            set_stream_value(&mut copy, field, value, sim);
            assert_eq!(copy, stream, "writing back what a field reads changes nothing (field {field})");
        }
    }

    fn zone(name: &str, id: u32) -> ZoneSettings {
        let mut zone = ZoneSettings::game_defaults();
        zone.name = name.to_owned();
        zone.zone_id = Some(id);
        zone
    }

    fn labels(field: u8, zones: &[ZoneSettings]) -> Vec<String> {
        choices(field, zones)
            .into_iter()
            .map(|choice| choice.label)
            .collect()
    }

    #[test]
    fn the_dropdowns_offer_the_games_options_in_the_games_order() {
        assert_eq!(labels(MATERIAL, &[]), ["None", "Plant", "Meat"]);
        assert_eq!(
            labels(DISTRIBUTION, &[]),
            [
                "Flat Circle",
                "Center to edge Circle",
                "Edge to center Circle",
                "Ring",
                "Flat Ring",
                "Rectangular"
            ]
        );
        assert_eq!(labels(MOVEMENT, &[]), ["None", "Free", "Locked to Target"]);
        // The target dropdown lists "None" and every zone, the zone's own included
        // (`TargetZoneDropdown.PopulateList`).
        let zones = [zone("A", 1), zone("B", 2)];
        assert_eq!(labels(TARGET, &zones), ["None", "A", "B"]);
        // The distribution and movement options have tooltips (`SettingChoice.tooltipText`).
        assert!(choices(DISTRIBUTION, &[]).iter().all(|c| !c.tip.is_empty()));
        assert!(choices(MOVEMENT, &[]).iter().all(|c| !c.tip.is_empty()));
        assert!(choices(MATERIAL, &[]).iter().all(|c| c.tip.is_empty()));
        // The item shows "Setting name: Choice" (`ChoiceSettingDropdown.OnItemHovered`); the material
        // and target dropdowns (other classes) have no item tooltips.
        let titles = |field| -> Vec<&'static str> {
            choices(field, &zones).iter().map(|c| c.title).collect()
        };
        assert_eq!(
            titles(DISTRIBUTION),
            [
                "Spawn distribution: Flat Circle",
                "Spawn distribution: Center to edge Circle",
                "Spawn distribution: Edge to center Circle",
                "Spawn distribution: Ring",
                "Spawn distribution: Flat Ring",
                "Spawn distribution: Rectangular",
            ]
        );
        assert_eq!(
            titles(MOVEMENT),
            [
                "Movement: None",
                "Movement: Free",
                "Movement: Locked to Target"
            ]
        );
        assert!(titles(MATERIAL).iter().all(|title| title.is_empty()));
        assert!(titles(TARGET).iter().all(|title| title.is_empty()));
        // Fields that are not dropdowns have no options.
        assert!(choices(FERTILITY, &zones).is_empty());
    }

    #[test]
    fn picking_each_option_sets_the_field_and_makes_it_the_current_one() {
        let zones = [zone("A", 1), zone("B", 2)];
        let mut edited = zones[0].clone();
        for field in [MATERIAL, DISTRIBUTION, MOVEMENT, TARGET] {
            for index in 0..choices(field, &zones).len() {
                set_choice(&mut edited, field, index, &zones);
                assert_eq!(
                    current_choice(&edited, field, &zones),
                    index,
                    "field {field}, option {index}"
                );
                // The label of the current choice is the option's.
                assert_eq!(
                    choice_label(&edited, field, &zones),
                    labels(field, &zones)[index],
                    "field {field}, option {index}"
                );
            }
        }
        // Out of range changes nothing.
        let before = edited.clone();
        set_choice(&mut edited, DISTRIBUTION, 99, &zones);
        assert_eq!(edited.distribution, before.distribution);
    }

    #[test]
    fn a_movement_that_cannot_follow_drops_the_target() {
        let zones = [zone("A", 1), zone("B", 2)];
        let mut edited = zones[0].clone();
        set_choice(&mut edited, MOVEMENT, 2, &zones);
        set_choice(&mut edited, TARGET, 2, &zones);
        assert_eq!(edited.target, Some(2));
        // `ChangeMovementType`: only attached zones keep a target.
        set_choice(&mut edited, MOVEMENT, 1, &zones);
        assert_eq!(edited.target, None);
        assert_eq!(current_choice(&edited, TARGET, &zones), 0);
    }

    #[test]
    fn a_click_still_advances_to_the_next_option_and_wraps() {
        let zones = [zone("A", 1), zone("B", 2)];
        let mut edited = zones[0].clone();
        edited.material = None;
        for expected in ["Plant", "Meat", "None"] {
            cycle(&mut edited, MATERIAL, &zones, 0);
            assert_eq!(choice_label(&edited, MATERIAL, &zones), expected);
        }
    }

    /// The texts the game's cards show (live capture of a zone and of a group's template): two decimals for the fertility, biomass and size
    /// sliders whatever the magnitude, significant digits for the radius, width and height.
    #[test]
    fn the_cards_slider_texts_follow_the_games() {
        let zone = ZoneSettings::game_defaults();
        let text = |field: u8, value: f32| slider_info(&zone, field, 5000.0).expect("a slider").format().format(value);
        assert_eq!(text(FERTILITY, 0.78), "0.78x");
        assert_eq!(text(FERTILITY, 1.0), "1.00x");
        assert_eq!(text(FERTILITY, 10.0), "10.00x");
        assert_eq!(text(BIOMASS, 10.0), "10.00x");
        assert_eq!(text(PELLET_SIZE, 1.0), "1.00x");
        let mut relative = zone.clone();
        relative.radius_is_relative = true;
        let radius = |value: f32| slider_info(&relative, RADIUS, 5000.0).expect("a slider").format().format(value);
        assert_eq!(radius(0.05), "0.05000x");
    }
}

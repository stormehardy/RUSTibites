//! The settings animator editor's own logic (`SettingsChangersEditor`, `SettingsChangerCreator`, `SettingBaseItem`, `EditableLineGraph`): the
//! three presets the creator offers, the catalogue of the settings an animator can take over, and the mathematics of the curve's graph.

use bibites_core::format::FloatValueFormat;
use bibites_core::settings::{
    IndependentSettings, InterpolationType, ScenarioSettings, SettingValue, SettingsChanger, SimSettings, SplinePoint, ZoneSettings,
};

use crate::dyn_panel::SliderMode;
use crate::setting_ui_data::{SETTING_UI, SettingScope, SettingUiKind};
use crate::zone_editor as ze;

/// Hours in a simulated second (the factor of every time setting of the editor).
pub const HOURS: f32 = 0.000_277_777_78;
/// How many vertical lines and horizontal lines the graph's grid has (21 labels along the time, 11 along the value).
pub const GRAPH_NX: usize = 21;
/// See [`GRAPH_NX`].
pub const GRAPH_NY: usize = 11;

// ---------------------------------------------------------------------------
// The creator
// ---------------------------------------------------------------------------

/// `AnimatorPresets`: what the creator panel offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// A straight run to a target value.
    Linear,
    /// A repeating cycle that starts at its offset and rises.
    Sine,
    /// A repeating cycle that starts at its peak.
    Cosine,
}

impl Preset {
    /// The presets in the dropdown's order (`SettingsChangerCreator.presetsChoices`).
    pub const ALL: [Self; 3] = [Self::Linear, Self::Sine, Self::Cosine];

    /// The dropdown's text.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Sine => "Sine",
            Self::Cosine => "Cosine",
        }
    }

    /// The help text of the choice.
    #[must_use]
    pub fn help(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Sine | Self::Cosine => "Repeating cycle",
        }
    }
}

/// A slider of the creator panel.
pub const START: u8 = 0;
/// How long the linear run takes.
pub const DURATION: u8 = 1;
/// Where the linear run ends.
pub const TARGET: u8 = 2;
/// How long a cycle takes.
pub const PERIOD: u8 = 3;
/// How far a cycle rises above and falls below its offset.
pub const AMPLITUDE: u8 = 4;
/// The value a cycle oscillates around.
pub const OFFSET: u8 = 5;

/// The sliders the creator shows for `preset`, in the panel's order.
#[must_use]
pub fn creator_fields(preset: Preset) -> &'static [u8] {
    match preset {
        Preset::Linear => &[START, DURATION, TARGET],
        Preset::Sine | Preset::Cosine => &[START, PERIOD, OFFSET, AMPLITUDE],
    }
}

/// A creator slider: its name and help text (`FloatSetting.Name` / `HelperText`), its range and how it reads.
#[derive(Clone, Copy, Debug)]
pub struct CreatorField {
    /// Label.
    pub name: &'static str,
    /// Tooltip text.
    pub help: &'static str,
    /// Smallest value.
    pub min: f32,
    /// Largest value.
    pub max: f32,
    /// `DefaultValue`.
    pub default: f32,
    /// How the slider maps its position to the value (`LogFloatSettingSlider(.., span)` or a linear one).
    pub mode: SliderMode,
    /// Units, factor and digits of the value text.
    pub units: &'static str,
    /// Display factor.
    pub factor: f32,
    /// Digits after the point.
    pub precision: i32,
}

impl CreatorField {
    /// How the value reads (plain digits: the creator's sliders show `10.0h`, not a scientific form).
    #[must_use]
    pub fn format(&self) -> FloatValueFormat {
        FloatValueFormat {
            prefix: "",
            units: self.units,
            precision: self.precision,
            factor: self.factor,
            si: false,
            always_show_sign: false,
            precision_is_si: false,
            is_int: false,
        }
    }
}

/// The description of creator slider `field`.
#[must_use]
pub fn creator_field(field: u8) -> CreatorField {
    let log = |span: f32| SliderMode::Log {
        base: 10.0,
        span: Some(span),
    };
    match field {
        START => CreatorField {
            name: "Start Time",
            help: "The time at which the settings takes effect \nWhen in sim, if not touched, the current time is used instead",
            min: 0.0,
            max: 360_000_000.0,
            default: 0.0,
            mode: log(5.0),
            units: "h",
            factor: HOURS,
            precision: 1,
        },
        DURATION => CreatorField {
            name: "Duration",
            help: "The time for reaching the target value",
            min: 3600.0,
            max: 3_600_000.0,
            default: 36_000.0,
            mode: log(4.0),
            units: "h",
            factor: HOURS,
            precision: 1,
        },
        TARGET => CreatorField {
            name: "Target Value",
            help: "The Final value for the settings",
            min: 0.0,
            max: 5.0,
            default: 1.5,
            mode: SliderMode::Linear,
            units: "",
            factor: 1.0,
            precision: 2,
        },
        PERIOD => CreatorField {
            name: "Period",
            help: "The peak value of the cycle",
            min: 3600.0,
            max: 3_600_000.0,
            default: 36_000.0,
            mode: log(4.0),
            units: "h",
            factor: HOURS,
            precision: 1,
        },
        AMPLITUDE => CreatorField {
            name: "Amplitude",
            help: "How much the value will rise above the offset",
            min: 0.0,
            max: 5.0,
            default: 0.5,
            mode: SliderMode::Linear,
            units: "",
            factor: 1.0,
            precision: 2,
        },
        _ => CreatorField {
            name: "Offset",
            help: "The value around which the cycle oscillate",
            min: -1.0,
            max: 5.0,
            default: 1.0,
            mode: SliderMode::Linear,
            units: "",
            factor: 1.0,
            precision: 2,
        },
    }
}

/// The creator panel (`SettingsChangerCreator`) and what is set in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Creator {
    /// Whether the panel is open.
    pub open: bool,
    /// The chosen preset.
    pub preset: Preset,
    /// Start time (seconds).
    pub start: f32,
    /// Whether the player moved the start slider (in a simulation an untouched start means "now").
    pub start_moved: bool,
    /// Duration of the linear run.
    pub duration: f32,
    /// Target of the linear run.
    pub target: f32,
    /// Period of a cycle.
    pub period: f32,
    /// Amplitude of a cycle.
    pub amplitude: f32,
    /// Offset of a cycle.
    pub offset: f32,
}

impl Default for Creator {
    fn default() -> Self {
        Self {
            open: false,
            preset: Preset::Linear,
            start: 0.0,
            start_moved: false,
            duration: creator_field(DURATION).default,
            target: creator_field(TARGET).default,
            period: creator_field(PERIOD).default,
            amplitude: creator_field(AMPLITUDE).default,
            offset: creator_field(OFFSET).default,
        }
    }
}

impl Creator {
    /// `OpenPanel`: every slider goes back to its default, the start to the simulated time (`sim_time`, none in the editor: 0).
    pub fn open(&mut self, sim_time: Option<f32>) {
        *self = Self {
            open: true,
            start: sim_time.unwrap_or(0.0),
            ..Self::default()
        };
    }

    /// The value of slider `field`.
    #[must_use]
    pub fn value(&self, field: u8) -> f32 {
        match field {
            START => self.start,
            DURATION => self.duration,
            TARGET => self.target,
            PERIOD => self.period,
            AMPLITUDE => self.amplitude,
            _ => self.offset,
        }
    }

    /// Sets slider `field` (kept inside its range).
    pub fn set(&mut self, field: u8, value: f32) {
        let info = creator_field(field);
        let value = value.clamp(info.min, info.max);
        match field {
            START => {
                self.start = value;
                self.start_moved = true;
            }
            DURATION => self.duration = value,
            TARGET => self.target = value,
            PERIOD => self.period = value,
            AMPLITUDE => self.amplitude = value,
            _ => self.offset = value,
        }
    }

    /// `Submit`: the animator the panel makes. In a simulation an untouched start slider means the simulated time now (`sim_time`).
    #[must_use]
    pub fn build(&self, sim_time: Option<f32>) -> SettingsChanger {
        use InterpolationType::{EaseIn, EaseInOut, EaseOut};
        let start = match sim_time {
            Some(now) if !self.start_moved => now,
            _ => self.start,
        };
        let point = |x: f32, y: f32| SplinePoint::new(x, y);
        let eased = |x: f32, y: f32, mode| SplinePoint::with_interpolation(x, y, mode, 2.0);
        let (name, points, repeat) = match self.preset {
            Preset::Linear => (
                "Linear Animator",
                vec![point(0.0, 1.0), point(self.duration, self.target)],
                false,
            ),
            Preset::Sine => (
                "Cycle Animator",
                vec![
                    point(0.0, self.offset),
                    eased(self.period / 4.0, self.offset + self.amplitude, EaseInOut),
                    eased(3.0 * self.period / 4.0, self.offset - self.amplitude, EaseInOut),
                    point(self.period, self.offset),
                ],
                true,
            ),
            Preset::Cosine => (
                "Cycle Animator",
                vec![
                    eased(0.0, self.offset + self.amplitude, EaseOut),
                    eased(self.period / 2.0, self.offset - self.amplitude, EaseInOut),
                    eased(self.period, self.offset + self.amplitude, EaseIn),
                ],
                true,
            ),
        };
        SettingsChanger {
            name: name.to_owned(),
            settings_bases: Vec::new(),
            points,
            repeat,
            start,
        }
    }
}

// ---------------------------------------------------------------------------
// The settings an animator can take over
// ---------------------------------------------------------------------------

/// A setting an animator can animate (`INumericSetting` in `ScenarioSettings.animatableSettings`).
#[derive(Clone, Debug, PartialEq)]
pub struct Animatable {
    /// The key the animator stores (`SettingsReferenceParser.GetKeyOfSetting`): a settings field, or `Zone(<id>).<field>`.
    pub key: String,
    /// `GetKeyedName()`: the name the dropdown lists (a zone's settings carry the zone's name).
    pub name: String,
    /// Smallest value.
    pub min: f32,
    /// Largest value.
    pub max: f32,
    /// `DefaultValue`.
    pub default: f32,
    /// The value's units, factor and digits.
    pub format: FloatValueFormat,
}

impl Animatable {
    /// The span of the base value's slider (`SettingBaseItem.AssignKey`: 2 decades, or twice the decades between the default and the largest
    /// value when the smallest is zero).
    #[must_use]
    pub fn slider_span(&self) -> f32 {
        if self.min.abs() < 1e-6 && self.default > 0.0 {
            let decades = (self.max / self.default).log10().round();
            if decades > 0.0 {
                return 2.0 * decades;
            }
        }
        2.0
    }
}

/// The zone settings an animator can take over: `(field, JSON key)`.
const ZONE_FIELDS: [(u8, bool, &str); 13] = [
    (ze::FERTILITY, true, "fertility"),
    (ze::BIOMASS, true, "biomassDensity"),
    (ze::PELLET_SIZE, true, "pelletSize"),
    (ze::SPEED, true, "speed"),
    (ze::POS_X, true, "posX"),
    (ze::POS_Y, true, "posY"),
    (ze::RADIUS, true, "radiusRelative"),
    (ze::RADIUS, false, "radiusAbsolute"),
    (ze::WIDTH, true, "widthRelative"),
    (ze::WIDTH, false, "widthAbsolute"),
    (ze::HEIGHT, true, "heightRelative"),
    (ze::HEIGHT, false, "heightAbsolute"),
    (ze::INSIDE, true, "insideRadius"),
];

/// Whether the core lets an animator write the scenario or independent setting `field`.
fn core_animatable(scope: SettingScope, field: &str) -> bool {
    match scope {
        SettingScope::Scenario => ScenarioSettings::SETTINGS.iter().any(|meta| meta.key == field && meta.animatable),
        SettingScope::Independent => IndependentSettings::SETTINGS.iter().any(|meta| meta.key == field && meta.animatable),
    }
}

/// Every setting an animator can take over (`ScenarioSettings.animatableSettings`): the numeric scenario and independent settings the game
/// lets it write, and the numeric settings of the explicit zones; sorted by their keyed names.
#[must_use]
pub fn catalog(settings: &SimSettings, zones: &[ZoneSettings]) -> Vec<Animatable> {
    let mut out = Vec::new();
    for row in SETTING_UI.iter().filter(|row| row.kind != SettingUiKind::Bool && core_animatable(row.scope, row.field)) {
        out.push(Animatable {
            key: row.field.to_owned(),
            name: row.name.to_owned(),
            min: row.min,
            max: row.max,
            default: row.default,
            format: crate::dyn_panel::format_of(row),
        });
    }
    let sim_size = settings.independents.simulation_size;
    for (index, zone) in zones.iter().enumerate() {
        for (field, relative, key) in ZONE_FIELDS {
            let mut view = zone.clone();
            view.radius_is_relative = relative;
            let Some(info) = ze::slider_info(&view, field, sim_size) else {
                continue;
            };
            out.push(Animatable {
                key: format!("Zone({}).{key}", zone.zone_id.unwrap_or(index as u32)),
                name: format!("Zone({}): {}", zone.name, ze::name_of(field)),
                min: info.min,
                max: info.max,
                default: info.default,
                format: info.format(),
            });
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.name.cmp(&b.name)));
    out
}

/// The value setting `key` has now (the base an animator starts from).
#[must_use]
pub fn current_value(settings: &SimSettings, zones: &[ZoneSettings], key: &str) -> Option<f32> {
    if let Some(rest) = key.strip_prefix("Zone(")
        && let Some((id, field)) = rest.split_once(").")
    {
        let id: u32 = id.parse().ok()?;
        let zone = zones.iter().find(|zone| zone.zone_id == Some(id))?;
        return Some(match field {
            "fertility" => zone.fertility,
            "biomassDensity" => zone.biomass_density,
            "pelletSize" => zone.pellet_size,
            "speed" => zone.speed,
            "posX" => zone.pos_x,
            "posY" => zone.pos_y,
            "radiusRelative" => zone.radius_relative,
            "radiusAbsolute" => zone.radius_absolute,
            "widthRelative" => zone.width_relative,
            "widthAbsolute" => zone.width_absolute,
            "heightRelative" => zone.height_relative,
            "heightAbsolute" => zone.height_absolute,
            "insideRadius" => zone.inside_radius,
            _ => return None,
        });
    }
    let row = SETTING_UI.iter().find(|row| row.field == key)?;
    let value = match row.scope {
        SettingScope::Scenario => settings.scenario.get_by_key(key),
        SettingScope::Independent => settings.independents.get_by_key(key),
    };
    match value? {
        SettingValue::Float(v) => Some(v),
        SettingValue::Int(v) => Some(v as f32),
        SettingValue::Bool(_) => None,
    }
}

/// The slider of a creator field (the types of `zone_editor`'s sliders, which the typed-value rules and the drags share).
#[must_use]
pub fn creator_slider_info(field: &CreatorField) -> ze::SliderInfo {
    ze::SliderInfo {
        min: field.min,
        max: field.max,
        default: field.default,
        mode: field.mode,
        units: field.units,
        precision: field.precision,
        si: false,
        factor: field.factor,
    }
}

/// The slider of the base value of an animated setting (`SettingBaseItem`: a log slider whose span follows the setting's range).
#[must_use]
pub fn base_slider_info(entry: &Animatable) -> ze::SliderInfo {
    ze::SliderInfo {
        min: entry.min,
        max: entry.max,
        default: entry.default,
        mode: SliderMode::Log {
            base: 10.0,
            span: Some(entry.slider_span()),
        },
        units: entry.format.units,
        precision: entry.format.precision,
        si: entry.format.si,
        factor: entry.format.factor,
    }
}

/// The name a base item shows for `key`.
#[must_use]
pub fn name_of(settings: &SimSettings, zones: &[ZoneSettings], key: &str) -> String {
    catalog(settings, zones)
        .into_iter()
        .find(|entry| entry.key == key)
        .map_or_else(|| key.to_owned(), |entry| entry.name)
}

// ---------------------------------------------------------------------------
// The graph
// ---------------------------------------------------------------------------

/// `MathfE.CeilToReadable`: the next number of the 1, 2, 2.5, 5, 10 series.
#[must_use]
pub fn ceil_to_readable(value: f32) -> f32 {
    let decade = value.log10().floor();
    let scaled = value * 10.0_f32.powf(-decade);
    if scaled > 5.0 {
        10.0_f32.powf(decade + 1.0)
    } else if scaled > 2.5 {
        5.0 * 10.0_f32.powf(decade)
    } else if scaled > 2.0 {
        2.5 * 10.0_f32.powf(decade)
    } else if scaled > 1.0 {
        2.0 * 10.0_f32.powf(decade)
    } else {
        10.0_f32.powf(decade)
    }
}

/// The part of the plane the graph shows (`EditableLineGraph.UpdateMinMax`): the corners and the spacing of the grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphView {
    /// Lower left corner (time in seconds, value).
    pub min: [f32; 2],
    /// Upper right corner.
    pub max: [f32; 2],
    /// Time between two vertical lines.
    pub x_step: f32,
    /// Value between two horizontal lines.
    pub y_step: f32,
}

/// `UpdateMinMax`: room for the curve, a third of its length more on the right, from 0 down to the lowest value, the grid on readable steps.
#[must_use]
pub fn graph_view(points: &[SplinePoint]) -> GraphView {
    let (first, last) = (points[0], points[points.len() - 1]);
    let mut min = [first.x, first.y];
    let mut max = [last.x, last.y];
    for point in points {
        min[1] = min[1].min(point.y);
        max[1] = max[1].max(point.y);
    }
    min[0] -= (max[0] - min[0]) / 3.0;
    min[0] = min[0].max(0.0);
    min[1] = min[1].min(0.0);
    max[0] += (max[0] - min[0]) / 3.0;
    max[1] *= 1.25;
    let y_step = ceil_to_readable((max[1] - min[1]) / (GRAPH_NY - 1) as f32);
    let x_step = ceil_to_readable(HOURS * (max[0] - min[0]) / (GRAPH_NX - 1) as f32) / HOURS;
    let min = [(min[0] / x_step).round() * x_step, (min[1] / y_step).round() * y_step];
    GraphView {
        min,
        max: [min[0] + x_step * (GRAPH_NX - 1) as f32, min[1] + y_step * (GRAPH_NY - 1) as f32],
        x_step,
        y_step,
    }
}

impl GraphView {
    /// Where the value `(time, value)` is in a graph `size` wide and high, from its lower left corner.
    #[must_use]
    pub fn to_rect(&self, value: [f32; 2], size: [f32; 2]) -> [f32; 2] {
        [
            (value[0] - self.min[0]) / (self.max[0] - self.min[0]) * size[0],
            (value[1] - self.min[1]) / (self.max[1] - self.min[1]) * size[1],
        ]
    }

    /// The value at a place of the graph (the other way round).
    #[must_use]
    pub fn from_rect(&self, at: [f32; 2], size: [f32; 2]) -> [f32; 2] {
        [
            self.min[0] + at[0] / size[0] * (self.max[0] - self.min[0]),
            self.min[1] + at[1] / size[1] * (self.max[1] - self.min[1]),
        ]
    }
}

/// `EditableLineGraph.CheckBoundsForPoint`: whether the frame has to be made again - a point is outside it, or it is far too big for the curve.
#[must_use]
pub fn needs_reframe(view: &GraphView, points: &[SplinePoint]) -> bool {
    if points.iter().any(|point| view.max[0] < point.x || view.max[1] < point.y || view.min[0] > point.x || view.min[1] > point.y) {
        return true;
    }
    let (first, last) = (points[0], points[points.len() - 1]);
    let (mut low, mut high) = (first.y, last.y);
    for point in points {
        low = low.min(point.y);
        high = high.max(point.y);
    }
    view.max[1] - view.min[1] > 2.5 * (high - low) || view.max[0] - view.min[0] > 2.5 * (last.x - first.x)
}

/// `MovePoint` (`EditableLineGraph`): point `index` goes to `to`. Two points may share a time (a jump: an end value, then the next point at the same
/// time and another value), and their order is what tells them apart: a dragged point hops only over the points that are strictly later (or
/// earlier) than where it lands, so it stays on the side of an equal-time point it came from. Returns where it is now.
pub fn move_point(points: &mut Vec<SplinePoint>, index: usize, to: [f32; 2]) -> usize {
    let count = points.len();
    let mut moved = points.remove(index);
    moved.x = to[0].max(0.0);
    moved.y = to[1];
    let at = if index > 0 && points[index - 1].x > moved.x {
        // Left: in front of the first earlier point that is later than the new time.
        points[..index].partition_point(|other| other.x <= moved.x)
    } else if index < count - 1 && points[index].x < moved.x {
        // Right: after the later points that are earlier than the new time.
        index + points[index..].iter().take_while(|other| other.x < moved.x).count()
    } else {
        index
    };
    points.insert(at, moved);
    at
}

/// `AddPoint`: a new point at `at`, in front of the first point that is later than its time (so after the points that already have it).
pub fn add_point(points: &mut Vec<SplinePoint>, at: [f32; 2]) -> Option<usize> {
    let x = at[0].max(0.0);
    let index = points.partition_point(|point| point.x <= x);
    points.insert(index, SplinePoint::new(x, at[1]));
    Some(index)
}

/// `RemovePoint`: the first point and the last two stay.
pub fn remove_point(points: &mut Vec<SplinePoint>, index: usize) -> bool {
    if index >= 1 && points.len() > 2 && index < points.len() {
        points.remove(index);
        true
    } else {
        false
    }
}

/// The interpolation choices of a point in the dropdown's order (`SplinePoint.interpolationChoices`).
pub const INTERPOLATIONS: [(InterpolationType, &str); 4] = [
    (InterpolationType::Linear, "Linear"),
    (InterpolationType::EaseIn, "Ease In"),
    (InterpolationType::EaseOut, "Ease Out"),
    (InterpolationType::EaseInOut, "Ease In-Out"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_presets_make_the_games_animators() {
        let mut creator = Creator::default();
        creator.open(None);
        let linear = creator.build(None);
        assert_eq!(linear.name, "Linear Animator");
        assert!(!linear.repeat);
        assert_eq!(linear.points, vec![SplinePoint::new(0.0, 1.0), SplinePoint::new(36_000.0, 1.5)]);
        creator.preset = Preset::Sine;
        let sine = creator.build(None);
        assert_eq!(sine.name, "Cycle Animator");
        assert!(sine.repeat);
        assert_eq!(sine.points.len(), 4);
        assert_eq!((sine.points[1].x, sine.points[1].y, sine.points[1].interpolation), (9000.0, 1.5, InterpolationType::EaseInOut));
        assert_eq!((sine.points[2].x, sine.points[2].y), (27_000.0, 0.5));
        assert_eq!((sine.points[3].x, sine.points[3].y), (36_000.0, 1.0));
        creator.preset = Preset::Cosine;
        let cosine = creator.build(None);
        assert_eq!(cosine.points.len(), 3);
        assert_eq!((cosine.points[0].y, cosine.points[0].interpolation), (1.5, InterpolationType::EaseOut));
        assert_eq!((cosine.points[1].x, cosine.points[1].y), (18_000.0, 0.5));
        assert_eq!(cosine.points[2].interpolation, InterpolationType::EaseIn);
        // In a simulation the start is "now" unless the slider was moved.
        creator.open(Some(7200.0));
        assert_eq!(creator.build(Some(7300.0)).start, 7300.0);
        creator.set(START, 3600.0);
        assert_eq!(creator.build(Some(7300.0)).start, 3600.0);
        assert_eq!(creator_fields(Preset::Linear), &[START, DURATION, TARGET]);
    }

    #[test]
    fn the_graph_frames_the_curve_like_the_games_does() {
        // The linear preset of the live game: 0 to 20 hours, 0 to 2.
        let view = graph_view(&[SplinePoint::new(0.0, 1.0), SplinePoint::new(36_000.0, 1.5)]);
        assert_eq!(view.min, [0.0, 0.0]);
        assert!((view.x_step - 3600.0).abs() < 1.0);
        assert!((view.max[0] - 72_000.0).abs() < 20.0);
        assert!((view.y_step - 0.2).abs() < 1e-6 && (view.max[1] - 2.0).abs() < 1e-5);
        // The point (10 h, 1.5) of an 831 x 320.14 graph sits at (415.5, 240.1).
        let at = view.to_rect([36_000.0, 1.5], [831.0, 320.14]);
        assert!((at[0] - 415.5).abs() < 0.5 && (at[1] - 240.1).abs() < 0.2);
        let back = view.from_rect(at, [831.0, 320.14]);
        assert!((back[0] - 36_000.0).abs() < 1.0 && (back[1] - 1.5).abs() < 1e-3);
        assert_eq!(ceil_to_readable(0.1875), 0.2);
        assert_eq!(ceil_to_readable(2.2), 2.5);
        assert_eq!(ceil_to_readable(7.0), 10.0);
    }

    #[test]
    fn points_move_get_added_and_removed_in_order() {
        let mut points = vec![SplinePoint::new(0.0, 1.0), SplinePoint::new(1000.0, 2.0), SplinePoint::new(2000.0, 1.0)];
        // A point dragged past its neighbour changes places with it.
        let now = move_point(&mut points, 1, [2500.0, 3.0]);
        assert_eq!(now, 2);
        assert_eq!(points.iter().map(|p| p.x).collect::<Vec<_>>(), vec![0.0, 2000.0, 2500.0]);
        assert_eq!(points[2].y, 3.0);
        // A jump: a point dragged onto a neighbour's time stays on the side it came from, and a new point goes after the ones that have the time.
        let now = move_point(&mut points, 2, [2000.0, 1.0]);
        assert_eq!(now, 2, "{points:?}");
        assert_eq!(points.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>(), vec![(0.0, 1.0), (2000.0, 1.0), (2000.0, 1.0)]);
        assert_eq!(add_point(&mut points, [1000.0, 0.5]), Some(1));
        assert_eq!(add_point(&mut points, [1000.0, 0.9]), Some(2), "after the point that has the time");
        assert!(!remove_point(&mut points, 0));
        assert!(remove_point(&mut points, 1));
        assert!(remove_point(&mut points, 1));
        // At least two points stay.
        assert!(remove_point(&mut points, 1));
        assert!(!remove_point(&mut points, 1));
        assert_eq!(points.len(), 2);
    }

    #[test]
    fn the_catalogue_lists_every_animatable_setting_sorted_and_the_explicit_zones() {
        let settings = SimSettings::game_defaults();
        let mut zone = ZoneSettings::game_defaults();
        zone.name = "World".to_owned();
        zone.zone_id = Some(0);
        let entries = catalog(&settings, &[zone.clone()]);
        assert!(entries.len() > 30, "{}", entries.len());
        assert!(entries.windows(2).all(|pair| pair[0].name.to_lowercase() <= pair[1].name.to_lowercase()));
        let fertility = entries.iter().find(|entry| entry.key == "Zone(0).fertility").expect("a zone's fertility");
        assert_eq!(fertility.name, "Zone(World): Fertility");
        assert!(entries.iter().any(|entry| entry.key == "pelletEnergy"));
        // The value an animator starts from, and the name its base item shows.
        assert_eq!(current_value(&settings, &[zone.clone()], "Zone(0).fertility"), Some(zone.fertility));
        assert!(current_value(&settings, &[zone.clone()], "pelletEnergy").is_some());
        assert_eq!(name_of(&settings, &[zone], "Zone(0).fertility"), "Zone(World): Fertility");
    }
}

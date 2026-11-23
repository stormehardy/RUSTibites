//! The panel of a selected tower or colour killer: `PheroTowerPanel`, `RadTowerPanel` and
//! `ColorKillerPanel` (`GUIManager.OnTargetChanged` opens one when a tower is selected).
//!
//! Geometry and texts come from the prefabs (read with `tools/unity/dump_ui_tree.py`, scene
//! `level2`, path ids 300 / 162 / 190): a `Button_UI` frame 10 units from the top-left corner with
//! 15 units of padding and 20 between its sections, a title line with the Remove button, then the
//! rows (25 high, 10 apart): the "follow zone" dropdown, the sliders and, for the colour killer, the
//! result strip, two switches and the progress bar. The slider ranges, value formats and the
//! effect of each control are the game's (`PheroTower`, `RadTower`, `ColorKillerPanel`).
//!
//! Not matched: the dropdown is a click-to-cycle box (the game opens a list), the value texts cannot be
//! typed (`ClickToEditHover`), the colour strip is a stand-in for the `Shader Graphs` spectrum
//! and tolerance materials, and there is no tooltip.

use bibites_core::format::FloatValueFormat;
use bibites_core::ids::{ToolId, ZoneId};
use bibites_core::math::Vec2;
use bibites_core::tools::{Tool, hue_to_rgb};
use bibites_core::world::World;

use crate::app::App;
use crate::dyn_panel::{SliderMode, fraction_range, log, value_range};
use crate::ui::{ControlCommand, Rgba, UiAction, UiAssets, UiDraw, UiFrame};

/// The "follow zone" dropdown (`TargetZoneDropdown`).
pub const ZONE: u8 = 0;
/// Pheromone tower: red strength.
pub const RED: u8 = 1;
/// Pheromone tower: green strength.
pub const GREEN: u8 = 2;
/// Pheromone tower: blue strength.
pub const BLUE: u8 = 3;
/// Pheromone tower: heading.
pub const HEADING: u8 = 4;
/// Radiation tower: mutation count rate.
pub const COUNT: u8 = 5;
/// Radiation tower: mutation intensity rate.
pub const INTENSITY: u8 = 6;
/// Radiation tower: radius.
pub const RADIUS: u8 = 7;
/// Radiation tower: taper factor.
pub const TAPER: u8 = 8;
/// Colour killer: target hue.
pub const HUE: u8 = 9;
/// Colour killer: tolerance.
pub const TOLERANCE: u8 = 10;
/// Colour killer: scan radius.
pub const SCAN_RADIUS: u8 = 11;
/// Colour killer: seconds between scans.
pub const PERIOD: u8 = 12;
/// Colour killer switch: select for body (off) or eyes (on).
pub const BODY_OR_EYES: u8 = 13;
/// Colour killer switch: kill any (off) or the most different (on) candidate.
pub const ANY_OR_MOST: u8 = 14;
/// RUSTibites Improved, gene selector: the lowest value of the range.
pub const GENE_MIN: u8 = 15;
/// Gene selector: the highest value of the range.
pub const GENE_MAX: u8 = 16;
/// Gene selector: how many bibites a cycle kills.
pub const KILLS: u8 = 17;
/// Gene selector switch: kill outside (off) or inside (on) the range.
pub const KILL_INSIDE: u8 = 18;
/// Gene selector: the "follow zone" dropdown (the colour killer's), under the gene's.
pub const FOLLOW: u8 = 19;
/// RUSTibites Improved, hydrothermal vent: the launch speed.
pub const VELOCITY: u8 = 20;
/// Vent: the size (energy) of a pellet.
pub const PELLET_SIZE: u8 = 21;
/// Vent: pellets shot per second.
pub const RATE: u8 = 22;
/// Vent: the cap on the energy of the pellets it has alive.
pub const CAP: u8 = 23;
/// Vent switch: stop shooting at the cap (off) or recycle the oldest pellets and go on (on).
pub const RECYCLE: u8 = 24;
/// Vent: the bar of how much of the cap its pellets fill (a row, not a control).
const VENT_BAR: u8 = 25;
/// Vent: the line of how many pellets it has shot (a row, not a control).
const VENT_LINE: u8 = 26;
/// Vent: how far a pellet slides.
pub const RANGE: u8 = 27;
/// RUSTibites Improved, force tower: how far it acts.
pub const FORCE_RANGE: u8 = 28;
/// Force tower: the strength (positive pulls, negative pushes).
pub const STRENGTH: u8 = 29;
/// Force tower switch: the same force all the way to the edge (off) or one that fades with the distance (on).
pub const FALLOFF: u8 = 30;
/// Force tower switch: acts on bibites.
pub const ON_BIBITES: u8 = 31;
/// Force tower switch: acts on pellets.
pub const ON_PELLETS: u8 = 32;
/// Force tower switch: acts on eggs.
pub const ON_EGGS: u8 = 33;
/// Force tower: the line of what it is doing (a row, not a control).
const FORCE_LINE: u8 = 34;
/// RUSTibites Improved, whirlpool tower: how fast it turns things (the acceleration along the circle).
pub const WHIRL_SPEED: u8 = 35;
/// Whirlpool tower: which way it turns (a button that says the way and turns it the other way when clicked).
pub const SPIN: u8 = 36;
/// Whirlpool tower: the line of what it is doing (a row, not a control).
const WHIRL_LINE: u8 = 37;

/// Panel placement (canvas units): 10 from the top-left corner.
const PANEL_X: f32 = 10.0;
const PANEL_Y: f32 = 10.0;
/// Padding inside the frame and the height of a row.
const PAD: f32 = 15.0;
const ROW_H: f32 = 25.0;
const DARK: Rgba = [28, 28, 28, 255];

/// The three panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `PheroTowerPanel`.
    Phero,
    /// `RadTowerPanel`.
    Rad,
    /// `ColorKillerPanel` ("Color Selector Editior" in the game).
    Killer,
    /// RUSTibites Improved: the gene selector's panel, the colour killer's with a gene and a range.
    Gene,
    /// RUSTibites Improved: the hydrothermal vent's panel: the follow-zone dropdown, four sliders, the recycle switch and the bar of its cap.
    Vent,
    /// RUSTibites Improved: the force tower's panel: the follow-zone dropdown, the range and strength sliders, the falloff switch, the three "acts on" switches and a line.
    Force,
    /// RUSTibites Improved: the whirlpool tower's panel: the follow-zone dropdown, the range and speed sliders, the direction button, the falloff switch, the three "acts on"
    /// switches and a line.
    Whirl,
}

impl Kind {
    /// The panel of `tool`.
    #[must_use]
    pub fn of(tool: &Tool) -> Self {
        match tool {
            Tool::Phero(_) => Self::Phero,
            Tool::Rad(_) => Self::Rad,
            Tool::Killer(_) => Self::Killer,
            Tool::Gene(_) => Self::Gene,
            Tool::Vent(_) => Self::Vent,
            Tool::Force(_) => Self::Force,
            Tool::Whirl(_) => Self::Whirl,
        }
    }

    /// The title text of the prefab (the game spells the colour killer's "Editior").
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Phero => "Pheromone Tower",
            Self::Rad => "RadTower",
            Self::Killer => "Color Selector Editior",
            Self::Gene => "Gene Selector",
            Self::Vent => "Hydrothermal Vent",
            Self::Force => "Force Tower",
            Self::Whirl => "Whirlpool Tower",
        }
    }

    /// The slider fields, top to bottom.
    #[must_use]
    pub fn sliders(self) -> &'static [u8] {
        match self {
            Self::Phero => &[RED, GREEN, BLUE, HEADING],
            Self::Rad => &[COUNT, INTENSITY, RADIUS, TAPER],
            Self::Killer => &[HUE, TOLERANCE, SCAN_RADIUS, PERIOD],
            Self::Gene => &[GENE_MIN, GENE_MAX, KILLS, SCAN_RADIUS, PERIOD],
            Self::Vent => &[VELOCITY, RANGE, PELLET_SIZE, RATE, CAP],
            Self::Force => &[FORCE_RANGE, STRENGTH],
            Self::Whirl => &[FORCE_RANGE, WHIRL_SPEED],
        }
    }

    /// Panel size (canvas units): the prefab's `VerticalLayoutGroup` result.
    #[must_use]
    fn size(self) -> Vec2 {
        match self {
            Self::Phero => Vec2::new(584.48, 275.47),
            Self::Rad => Vec2::new(555.0, 275.47),
            Self::Killer => Vec2::new(514.05, 428.5),
            Self::Gene => Vec2::new(514.05, 505.0),
            Self::Vent => Vec2::new(514.05, 395.0),
            Self::Force => Vec2::new(514.05, 360.0),
            Self::Whirl => Vec2::new(514.05, 395.0),
        }
    }

    /// Width of the Remove button (it takes the title line's spare width in the radiation panel).
    #[must_use]
    fn remove_width(self) -> f32 {
        match self {
            Self::Phero => 140.0,
            Self::Rad => 217.73,
            Self::Killer | Self::Gene | Self::Vent | Self::Force | Self::Whirl => 99.73,
        }
    }

    /// The slider and value-text boxes of a row, from the row's left edge.
    #[must_use]
    fn row_boxes(self) -> ((f32, f32), (f32, f32)) {
        match self {
            Self::Phero => ((135.0, 464.48), (479.48, 554.48)),
            Self::Rad => ((135.0, 435.0), (450.0, 525.0)),
            Self::Killer | Self::Gene | Self::Vent => ((135.0, 414.05), (429.05, 484.05)),
            // The force tower's strength reads "+250u/s²": its value box is wider (and its slider shorter) than the others'.
            Self::Force | Self::Whirl => ((135.0, 374.05), (389.05, 484.05)),
        }
    }

    /// Top of a slider's row, from the panel's top.
    #[must_use]
    fn row_top(self, field: u8) -> f32 {
        let main = if matches!(self, Self::Killer | Self::Gene | Self::Vent | Self::Force | Self::Whirl) { 65.0 } else { 65.47 };
        let offset = match (self, field) {
            (_, ZONE) => 0.0,
            (Self::Phero, RED) | (Self::Rad, COUNT) => 50.0,
            (Self::Phero, GREEN) | (Self::Rad, INTENSITY) => 85.0,
            (Self::Phero, BLUE) => 120.0,
            (Self::Phero, HEADING) | (Self::Rad, TAPER) => 170.0,
            (Self::Rad, RADIUS) => 135.0,
            (Self::Killer, HUE) => 35.0,
            (Self::Killer, TOLERANCE) => 70.0,
            (Self::Killer, BODY_OR_EYES) => 183.5,
            (Self::Killer, ANY_OR_MOST) => 218.5,
            (Self::Killer, SCAN_RADIUS) => 253.5,
            (Self::Killer, PERIOD) => 288.5,
            // The gene selector: the gene row at the top, the "follow zone" row, the two ends of the range, the strip that shows the range (140), the kills per
            // cycle, the two switches, the scan radius and the period, the progress bar (below) and the line of what it did (below that).
            (Self::Gene, FOLLOW) => 35.0,
            (Self::Gene, GENE_MIN) => 70.0,
            (Self::Gene, GENE_MAX) => 105.0,
            (Self::Gene, KILLS) => 180.0,
            (Self::Gene, KILL_INSIDE) => 215.0,
            (Self::Gene, ANY_OR_MOST) => 250.0,
            (Self::Gene, SCAN_RADIUS) => 285.0,
            (Self::Gene, PERIOD) => 320.0,
            // The hydrothermal vent: the four sliders, the recycle switch, the bar of how full its cap is and the line of how many it has shot.
            (Self::Vent, VELOCITY) => 35.0,
            (Self::Vent, RANGE) => 70.0,
            (Self::Vent, PELLET_SIZE) => 105.0,
            (Self::Vent, RATE) => 140.0,
            (Self::Vent, CAP) => 175.0,
            (Self::Vent, RECYCLE) => 215.0,
            (Self::Vent, VENT_BAR) => 255.0,
            (Self::Vent, VENT_LINE) => 290.0,
            // The force tower: the two sliders, the falloff switch, the three "acts on" switches and the line of what it is doing.
            (Self::Force, FORCE_RANGE) => 35.0,
            (Self::Force, STRENGTH) => 70.0,
            (Self::Force, FALLOFF) => 110.0,
            (Self::Force, ON_BIBITES) => 145.0,
            (Self::Force, ON_PELLETS) => 180.0,
            (Self::Force, ON_EGGS) => 215.0,
            (Self::Force, FORCE_LINE) => 255.0,
            // The whirlpool tower: the two sliders, the direction button, the falloff switch, the three "acts on" switches and the line of what it is doing.
            (Self::Whirl, FORCE_RANGE) => 35.0,
            (Self::Whirl, WHIRL_SPEED) => 70.0,
            (Self::Whirl, SPIN) => 110.0,
            (Self::Whirl, FALLOFF) => 145.0,
            (Self::Whirl, ON_BIBITES) => 180.0,
            (Self::Whirl, ON_PELLETS) => 215.0,
            (Self::Whirl, ON_EGGS) => 250.0,
            (Self::Whirl, WHIRL_LINE) => 290.0,
            _ => 0.0,
        };
        main + offset
    }
}

/// Label of a slider (the `Name` of the tower's settings; the colour killer's labels are in its
/// prefab).
#[must_use]
pub fn name_of(field: u8) -> &'static str {
    match field {
        ZONE => "Attached to Zone",
        FOLLOW => "Follow Zone:",
        RED => "Red Phero",
        GREEN => "Green Phero",
        BLUE => "Blue Phero",
        HEADING => "Heading",
        COUNT => "Count Rate",
        INTENSITY => "Intensity Rate",
        RADIUS => "Radius",
        TAPER => "Taper Factor",
        HUE => "Hue:",
        TOLERANCE => "Tolerance:",
        SCAN_RADIUS => "Radius:",
        PERIOD => "Period:",
        BODY_OR_EYES => "Select For :",
        ANY_OR_MOST => "Kill:",
        GENE_MIN => "Lowest:",
        GENE_MAX => "Highest:",
        KILLS => "Kills:",
        KILL_INSIDE => "Kill:",
        VELOCITY => "Velocity:",
        RANGE => "Range:",
        PELLET_SIZE => "Pellet Size:",
        RATE => "Shoot Rate:",
        CAP => "Pellet Cap:",
        RECYCLE => "Recycle:",
        FORCE_RANGE => "Range:",
        STRENGTH => "Strength:",
        FALLOFF => "Falloff:",
        WHIRL_SPEED => "Speed:",
        SPIN => "Direction:",
        ON_BIBITES => "Bibites:",
        ON_PELLETS => "Pellets:",
        ON_EGGS => "Eggs:",
        _ => "",
    }
}

/// How a slider maps to its value and how the value reads.
#[derive(Clone, Debug)]
pub struct SliderDef {
    /// Lowest value.
    pub min: f32,
    /// Highest value.
    pub max: f32,
    /// The setting's default (the reference of a log slider).
    pub default: f32,
    /// Linear or logarithmic (`LogFloatSettingSlider(10, ..., simple, 4)`).
    pub mode: SliderMode,
    /// The value text (`setting.formatting`, or the prefab's `FloatValueTextHandle`).
    pub format: FloatValueFormat,
}

/// The slider of `field` in a world of size `sim_size` (the `FloatSetting`s of `PheroTower` and
/// `RadTower`, the prefab's sliders and `ColorKillerPanel.FillPanel` for the colour killer).
#[must_use]
pub fn slider_def(field: u8, sim_size: f32) -> Option<SliderDef> {
    let phero = |_: u8| SliderDef {
        min: 0.0,
        max: 100.0,
        default: 0.0,
        mode: log(10.0, Some(4.0)),
        format: FloatValueFormat {
            units: "u²",
            precision: 1,
            si: true,
            ..FloatValueFormat::default()
        },
    };
    Some(match field {
        RED | GREEN | BLUE => phero(field),
        HEADING => SliderDef {
            min: 0.0,
            max: std::f32::consts::TAU,
            default: 0.0,
            mode: SliderMode::Linear,
            format: FloatValueFormat {
                precision: 2,
                si: false,
                ..FloatValueFormat::default()
            },
        },
        COUNT => SliderDef {
            min: 0.0,
            max: 0.1,
            default: 0.001,
            mode: log(10.0, Some(4.0)),
            format: FloatValueFormat {
                prefix: "+",
                units: "N/s",
                precision: 1,
                si: true,
                ..FloatValueFormat::default()
            },
        },
        INTENSITY => SliderDef {
            min: 0.0,
            max: 0.1,
            default: 0.001,
            mode: log(10.0, Some(4.0)),
            format: FloatValueFormat {
                prefix: "+",
                units: "%/s",
                precision: 1,
                factor: 100.0,
                si: false,
                ..FloatValueFormat::default()
            },
        },
        RADIUS => {
            // `RadTower.InitTower`: the radius goes up to half the world, and its default is
            // `min(250, max / 2)`.
            let max = sim_size * 0.5;
            SliderDef {
                min: 10.0,
                max,
                default: 250.0_f32.min(max * 0.5),
                mode: log(10.0, Some(4.0)),
                format: FloatValueFormat {
                    units: "u",
                    precision: 1,
                    si: true,
                    ..FloatValueFormat::default()
                },
            }
        }
        TAPER => SliderDef {
            min: 0.0,
            max: 3.0,
            default: 1.0,
            mode: SliderMode::Linear,
            format: FloatValueFormat {
                precision: 1,
                si: false,
                ..FloatValueFormat::default()
            },
        },
        HUE => SliderDef {
            min: 0.0,
            max: 1.0,
            default: 0.0,
            mode: SliderMode::Linear,
            format: FloatValueFormat {
                precision: 2,
                si: false,
                ..FloatValueFormat::default()
            },
        },
        TOLERANCE => SliderDef {
            min: 0.0,
            max: 0.5,
            default: 0.1,
            mode: SliderMode::Linear,
            format: FloatValueFormat {
                units: "%",
                precision: 1,
                factor: 100.0,
                si: false,
                ..FloatValueFormat::default()
            },
        },
        SCAN_RADIUS => SliderDef {
            min: 100.0,
            max: sim_size.max(101.0),
            default: 400.0,
            mode: SliderMode::Linear,
            format: FloatValueFormat {
                units: "u",
                precision: 0,
                si: false,
                ..FloatValueFormat::default()
            },
        },
        PERIOD => SliderDef {
            min: 5.0,
            max: 600.0,
            default: 100.0,
            mode: SliderMode::Linear,
            format: FloatValueFormat {
                units: "s",
                precision: 0,
                si: false,
                ..FloatValueFormat::default()
            },
        },
        // RUSTibites Improved, the hydrothermal vent: each over its range on a log scale (they span two to four decades).
        VELOCITY => SliderDef {
            min: bibites_core::vent::VENT_VELOCITY_MIN,
            max: bibites_core::vent::VENT_VELOCITY_MAX,
            default: bibites_core::vent::VENT_VELOCITY_DEFAULT,
            mode: log(10.0, None),
            format: FloatValueFormat {
                units: "u/s",
                precision: 0,
                si: false,
                ..FloatValueFormat::default()
            },
        },
        RANGE => SliderDef {
            min: bibites_core::vent::VENT_RANGE_MIN,
            max: bibites_core::vent::VENT_RANGE_MAX,
            default: bibites_core::vent::VENT_RANGE_DEFAULT,
            mode: log(10.0, None),
            format: FloatValueFormat {
                units: "u",
                precision: 0,
                si: false,
                ..FloatValueFormat::default()
            },
        },
        PELLET_SIZE => SliderDef {
            min: bibites_core::vent::VENT_PELLET_MIN,
            max: bibites_core::vent::VENT_PELLET_MAX,
            default: bibites_core::vent::VENT_PELLET_DEFAULT,
            mode: log(10.0, None),
            format: FloatValueFormat {
                units: "E",
                precision: 0,
                si: false,
                ..FloatValueFormat::default()
            },
        },
        RATE => SliderDef {
            min: bibites_core::vent::VENT_RATE_MIN,
            max: bibites_core::vent::VENT_RATE_MAX,
            default: bibites_core::vent::VENT_RATE_DEFAULT,
            mode: log(10.0, None),
            format: FloatValueFormat {
                units: "/s",
                precision: 1,
                si: false,
                ..FloatValueFormat::default()
            },
        },
        CAP => SliderDef {
            min: bibites_core::vent::VENT_CAP_MIN,
            max: bibites_core::vent::VENT_CAP_MAX,
            default: bibites_core::vent::VENT_CAP_DEFAULT,
            mode: log(10.0, None),
            format: FloatValueFormat {
                units: "E",
                precision: 1,
                si: true,
                ..FloatValueFormat::default()
            },
        },
        // RUSTibites Improved, the force tower: the range over its decades on a log scale; the strength linear, a push to the left of the middle and a pull to the right.
        FORCE_RANGE => SliderDef {
            min: bibites_core::force_tower::FORCE_RANGE_MIN,
            max: bibites_core::force_tower::FORCE_RANGE_MAX,
            default: bibites_core::force_tower::FORCE_RANGE_DEFAULT,
            mode: log(10.0, None),
            format: FloatValueFormat {
                units: "u",
                precision: 0,
                si: false,
                ..FloatValueFormat::default()
            },
        },
        STRENGTH => SliderDef {
            min: bibites_core::force_tower::FORCE_STRENGTH_MIN,
            max: bibites_core::force_tower::FORCE_STRENGTH_MAX,
            default: bibites_core::force_tower::FORCE_STRENGTH_DEFAULT,
            mode: SliderMode::Linear,
            format: FloatValueFormat {
                units: "u/s²",
                precision: 0,
                si: false,
                always_show_sign: true,
                ..FloatValueFormat::default()
            },
        },
        // RUSTibites Improved, the whirlpool tower: the speed, linear from none to the fastest (the way it turns is the direction button's).
        WHIRL_SPEED => SliderDef {
            min: bibites_core::whirlpool::WHIRL_SPEED_MIN,
            max: bibites_core::whirlpool::WHIRL_SPEED_MAX,
            default: bibites_core::whirlpool::WHIRL_SPEED_DEFAULT,
            mode: SliderMode::Linear,
            format: FloatValueFormat {
                units: "u/s²",
                precision: 0,
                si: false,
                ..FloatValueFormat::default()
            },
        },
        KILLS => SliderDef {
            min: 1.0,
            max: bibites_core::gene_selector::GENE_SELECTOR_MAX_KILLS as f32,
            default: 1.0,
            mode: SliderMode::Linear,
            format: FloatValueFormat {
                precision: 0,
                si: false,
                ..FloatValueFormat::default()
            },
        },
        _ => return None,
    })
}

/// The slider of `field` for `tool`: what [`slider_def`] gives, except that a gene selector's two range sliders run over the range of ITS gene, in the gene's units.
#[must_use]
pub fn slider_def_for(tool: &Tool, field: u8, sim_size: f32) -> Option<SliderDef> {
    if let (Tool::Gene(selector), GENE_MIN | GENE_MAX) = (tool, field) {
        let def = selector.def();
        let span = def.max - def.min;
        return Some(SliderDef {
            min: def.min,
            max: def.max,
            default: def.default,
            mode: SliderMode::Linear,
            format: FloatValueFormat {
                units: def.units,
                precision: if span >= 100.0 { 0 } else if span >= 10.0 { 1 } else { 2 },
                si: false,
                ..FloatValueFormat::default()
            },
        });
    }
    slider_def(field, sim_size)
}

/// Current value of a slider field of `tool`.
#[must_use]
pub fn value_of(tool: &Tool, field: u8) -> f32 {
    match (tool, field) {
        (Tool::Phero(tower), RED) => tower.red,
        (Tool::Phero(tower), GREEN) => tower.green,
        (Tool::Phero(tower), BLUE) => tower.blue,
        (Tool::Phero(tower), HEADING) => tower.heading,
        (Tool::Rad(tower), COUNT) => tower.count_rate,
        (Tool::Rad(tower), INTENSITY) => tower.intensity_rate,
        (Tool::Rad(tower), RADIUS) => tower.radius,
        (Tool::Rad(tower), TAPER) => tower.taper,
        (Tool::Killer(killer), HUE) => killer.hue,
        (Tool::Killer(killer), TOLERANCE) => killer.tolerance,
        (Tool::Killer(killer), SCAN_RADIUS) => killer.radius,
        (Tool::Killer(killer), PERIOD) => killer.period,
        (Tool::Gene(selector), GENE_MIN) => selector.min,
        (Tool::Gene(selector), GENE_MAX) => selector.max,
        (Tool::Gene(selector), KILLS) => selector.kills_per_cycle as f32,
        (Tool::Gene(selector), SCAN_RADIUS) => selector.radius,
        (Tool::Gene(selector), PERIOD) => selector.period,
        (Tool::Vent(vent), VELOCITY) => vent.velocity,
        (Tool::Vent(vent), RANGE) => vent.range,
        (Tool::Vent(vent), PELLET_SIZE) => vent.pellet_energy,
        (Tool::Vent(vent), RATE) => vent.rate,
        (Tool::Vent(vent), CAP) => vent.cap,
        (Tool::Force(tower), FORCE_RANGE) => tower.range,
        (Tool::Force(tower), STRENGTH) => tower.strength,
        (Tool::Whirl(tower), FORCE_RANGE) => tower.range,
        (Tool::Whirl(tower), WHIRL_SPEED) => tower.speed,
        _ => 0.0,
    }
}

/// Writes a slider field of `tool` (the value is already inside the slider's range).
fn write_value(tool: &mut Tool, field: u8, value: f32) {
    match (tool, field) {
        (Tool::Phero(tower), RED) => tower.red = value,
        (Tool::Phero(tower), GREEN) => tower.green = value,
        (Tool::Phero(tower), BLUE) => tower.blue = value,
        (Tool::Phero(tower), HEADING) => tower.heading = value,
        (Tool::Rad(tower), COUNT) => tower.count_rate = value,
        (Tool::Rad(tower), INTENSITY) => tower.intensity_rate = value,
        (Tool::Rad(tower), RADIUS) => tower.radius = value,
        (Tool::Rad(tower), TAPER) => tower.taper = value,
        (Tool::Killer(killer), HUE) => killer.hue = value,
        (Tool::Killer(killer), TOLERANCE) => killer.tolerance = value,
        (Tool::Killer(killer), SCAN_RADIUS) => killer.radius = value,
        (Tool::Killer(killer), PERIOD) => killer.period = value,
        (Tool::Gene(selector), GENE_MIN) => selector.set_min(value),
        (Tool::Gene(selector), GENE_MAX) => selector.set_max(value),
        (Tool::Gene(selector), KILLS) => selector.set_kills_per_cycle(value.round().max(1.0) as u32),
        (Tool::Gene(selector), SCAN_RADIUS) => selector.radius = value,
        (Tool::Gene(selector), PERIOD) => selector.period = value,
        (Tool::Vent(vent), VELOCITY) => vent.set_velocity(value),
        (Tool::Vent(vent), RANGE) => vent.set_range(value),
        (Tool::Vent(vent), PELLET_SIZE) => vent.set_pellet_energy(value),
        (Tool::Vent(vent), RATE) => vent.set_rate(value),
        (Tool::Vent(vent), CAP) => vent.set_cap(value),
        (Tool::Force(tower), FORCE_RANGE) => tower.set_range(value),
        (Tool::Force(tower), STRENGTH) => tower.set_strength(value),
        (Tool::Whirl(tower), FORCE_RANGE) => tower.set_range(value),
        (Tool::Whirl(tower), WHIRL_SPEED) => tower.set_speed(value),
        _ => {}
    }
}

/// The text of a slider's value box.
#[must_use]
pub fn value_text(tool: &Tool, field: u8, sim_size: f32) -> String {
    slider_def_for(tool, field, sim_size)
        .map(|def| def.format.format(value_of(tool, field)))
        .unwrap_or_default()
}

/// Slider position `0..=1` of a field's current value.
#[must_use]
pub fn fraction_of_tool(tool: &Tool, field: u8, sim_size: f32) -> f32 {
    slider_def_for(tool, field, sim_size).map_or(0.0, |def| {
        fraction_range(
            def.mode,
            def.min,
            def.max,
            def.default,
            value_of(tool, field),
        )
        .clamp(0.0, 1.0)
    })
}

/// Moves slider `field` of tool `id` to position `fraction` (`SettingSlider.SetValueFromSlider`).
pub fn slide(world: &mut World, id: ToolId, field: u8, fraction: f32) {
    let sim_size = world.bounds.sim_size();
    let Some(def) = world.tools.get(id).and_then(|tool| slider_def_for(tool, field, sim_size)) else {
        return;
    };
    let value = value_range(
        def.mode,
        def.min,
        def.max,
        def.default,
        fraction.clamp(0.0, 1.0),
    );
    if !value.is_finite() {
        return;
    }
    let value = value.clamp(def.min, def.max);
    let Some(tool) = world.tools.get_mut(id) else {
        return;
    };
    write_value(tool, field, value);
    // `PheroTower.UpdatePheroSpot`: the pheromone spot follows at once, even while paused.
    if matches!(world.tools.get(id), Some(Tool::Phero(_))) {
        world.step_tools(0.0);
    }
}

/// Flips a colour killer switch (`SetSelectionTarget` / `SetKillOption`) or a gene selector's (outside / inside, any / most).
pub fn flip_switch(world: &mut World, id: ToolId, field: u8) {
    match world.tools.get_mut(id) {
        Some(Tool::Killer(killer)) => match field {
            BODY_OR_EYES => killer.body_or_eye = !killer.body_or_eye,
            ANY_OR_MOST => killer.any_or_most = !killer.any_or_most,
            _ => {}
        },
        Some(Tool::Gene(selector)) => match field {
            KILL_INSIDE => selector.kill_inside = !selector.kill_inside,
            ANY_OR_MOST => selector.any_or_most = !selector.any_or_most,
            _ => {}
        },
        Some(Tool::Vent(vent)) if field == RECYCLE => vent.recycle = !vent.recycle,
        // RUSTibites Improved: the force tower's falloff and its three kinds of body.
        Some(Tool::Force(tower)) => match field {
            FALLOFF => {
                tower.falloff = match tower.falloff {
                    bibites_core::force_tower::ForceFalloff::Constant => bibites_core::force_tower::ForceFalloff::Fading,
                    bibites_core::force_tower::ForceFalloff::Fading => bibites_core::force_tower::ForceFalloff::Constant,
                };
            }
            ON_BIBITES => tower.bibites = !tower.bibites,
            ON_PELLETS => tower.pellets = !tower.pellets,
            ON_EGGS => tower.eggs = !tower.eggs,
            _ => {}
        },
        // RUSTibites Improved: the whirlpool tower's direction, falloff and its three kinds of body.
        Some(Tool::Whirl(tower)) => match field {
            SPIN => tower.spin = tower.spin.reversed(),
            FALLOFF => {
                tower.falloff = match tower.falloff {
                    bibites_core::force_tower::ForceFalloff::Constant => bibites_core::force_tower::ForceFalloff::Fading,
                    bibites_core::force_tower::ForceFalloff::Fading => bibites_core::force_tower::ForceFalloff::Constant,
                };
            }
            ON_BIBITES => tower.bibites = !tower.bibites,
            ON_PELLETS => tower.pellets = !tower.pellets,
            ON_EGGS => tower.eggs = !tower.eggs,
            _ => {}
        },
        _ => {}
    }
}

/// RUSTibites Improved: picks the gene `index` (of `GENE_DEFS`) for the gene selector `id`; its range becomes that gene's whole range.
pub fn set_gene(world: &mut World, id: ToolId, index: usize) {
    if let Some(Tool::Gene(selector)) = world.tools.get_mut(id) {
        selector.set_gene(index.min(bibites_core::genome::N_GENES - 1) as u8);
    }
}

/// The choices of the "follow zone" dropdown.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneChoices {
    /// The zone each choice attaches to; the first is "None".
    pub zones: Vec<Option<ZoneId>>,
    /// The labels shown.
    pub labels: Vec<String>,
    /// The current choice.
    pub current: usize,
}

/// `TargetZoneDropdown.PopulateList` for the towers: "None" and every zone by name.
/// `ColorKillerPanel.BuildAttachedSpawnerDropdown` for the colour killer (and, in RUSTibites Improved, the gene selector): "None" and the five
/// closest zones, each with its distance ("Zone (123u)").
#[must_use]
pub fn zone_choices(world: &World, tool: &Tool) -> ZoneChoices {
    let mut zones: Vec<Option<ZoneId>> = vec![None];
    let mut labels = vec!["None".to_owned()];
    let all = world
        .zones
        .zones
        .iter()
        .map(|zone| (zone.id, zone.config.0.name.clone(), zone.pos));
    if matches!(tool, Tool::Killer(_) | Tool::Gene(_) | Tool::Vent(_) | Tool::Force(_) | Tool::Whirl(_)) {
        let tool_pos = tool.base().pos;
        let mut by_distance: Vec<(ZoneId, String, f32)> = all
            .map(|(id, name, pos)| (id, name, pos.distance(tool_pos)))
            .collect();
        by_distance.sort_by(|a, b| a.2.total_cmp(&b.2));
        for (id, name, distance) in by_distance.into_iter().take(5) {
            zones.push(Some(id));
            labels.push(format!("{name} ({distance:.0}u)"));
        }
    } else {
        for (id, name, _) in all {
            zones.push(Some(id));
            labels.push(name);
        }
    }
    let attached = tool.base().attached_zone;
    let current = attached
        .and_then(|attached| zones.iter().position(|zone| *zone == Some(attached)))
        .unwrap_or(0);
    ZoneChoices {
        zones,
        labels,
        current,
    }
}

/// Advances the dropdown to its next choice (`SetAttachedSpawnPoint` / `SetValueOfSetting`).
pub fn cycle_zone(world: &mut World, id: ToolId) {
    let Some(tool) = world.tools.get(id).copied() else {
        return;
    };
    let choices = zone_choices(world, &tool);
    let next = (choices.current + 1) % choices.zones.len().max(1);
    set_zone(world, id, next);
}

/// Picks option `index` of the dropdown (a click on its list).
pub fn set_zone(world: &mut World, id: ToolId, index: usize) {
    let Some(tool) = world.tools.get(id).copied() else {
        return;
    };
    let choices = zone_choices(world, &tool);
    if let Some(zone) = choices.zones.get(index) {
        world.set_tool_attached_zone(id, *zone);
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// Everything the panel shows, copied from the world.
pub struct Model {
    /// The tool.
    pub tool: Tool,
    /// The dropdown.
    pub zone: ZoneChoices,
    /// Simulation size (the sliders' ranges).
    pub sim_size: f32,
    /// RUSTibites Improved: gene selectors act (the Settings switch is on).
    pub genes_active: bool,
    /// RUSTibites Improved: hydrothermal vents act (the Settings switch is on).
    pub vents_active: bool,
    /// RUSTibites Improved: force towers act (the Settings switch is on).
    pub forces_active: bool,
    /// RUSTibites Improved: whirlpool towers act (the Settings switch is on).
    pub whirls_active: bool,
}

/// The panel of the selected tool, when it is open.
#[must_use]
pub fn model(app: &App) -> Option<Model> {
    if !app.tool_panel_open {
        return None;
    }
    let id = app.selected_tool_id()?;
    let tool = *app.world.tools.get(id)?;
    Some(Model {
        zone: zone_choices(&app.world, &tool),
        sim_size: app.world.bounds.sim_size(),
        genes_active: app.improved.is_on(bibites_core::improved::Feature::GeneSelectorTower),
        vents_active: app.improved.is_on(bibites_core::improved::Feature::HydrothermalVent),
        forces_active: app.improved.is_on(bibites_core::improved::Feature::ForceTower),
        whirls_active: app.improved.is_on(bibites_core::improved::Feature::Whirlpool),
        tool,
    })
}

/// Rough advance of a run of text at `px` (the build step has no font metrics).
fn estimate_width(text: &str, px: f32) -> f32 {
    text.chars().count() as f32 * px * 0.56
}

/// The font size (at most `px`) at which `text` fits `width` units, but not below 12.
fn fit_px(text: &str, px: f32, width: f32) -> f32 {
    let wanted = estimate_width(text, px);
    if wanted > width && wanted > 0.0 {
        (px * width / wanted).max(12.0)
    } else {
        px
    }
}

#[allow(clippy::too_many_arguments)]
fn slice(
    frame: &mut UiFrame,
    ui: &UiAssets,
    sprite: &str,
    border: f32,
    dest_scale: f32,
    pos: Vec2,
    size: Vec2,
    tint: Rgba,
) {
    match ui.texture_of_alias(sprite) {
        Some(texture) => frame.draws.push(UiDraw::NineSlice {
            texture,
            border: [border; 4],
            dest_scale,
            pos,
            size,
            tint,
        }),
        None => frame.rect(pos, size, tint),
    }
}

fn icon(frame: &mut UiFrame, ui: &UiAssets, sprite: &str, pos: Vec2, size: Vec2, tint: Rgba) {
    if let Some(texture) = ui.texture_of_alias(sprite) {
        frame.draws.push(UiDraw::Image {
            texture,
            pos,
            size,
            tint,
        });
    }
}

/// Hit ids of the panel (the selection panel uses 8500..8600).
const HIT_PANEL: u32 = 8600;
const HIT_REMOVE: u32 = 8601;
const HIT_ZONE: u32 = 8602;
/// The gene selector's second dropdown, "Follow Zone:" (the ids 8604..8609 are free: no switch has a field below 13).
const HIT_FOLLOW: u32 = 8604;
const HIT_SWITCH: u32 = 8603;
const HIT_SLIDER: u32 = 8610;

/// One dropdown row of the panel: its label, the width of its box, its top, the text it shows, its hit id and the list it opens.
struct Dropdown<'a> {
    label: &'a str,
    width: f32,
    top: f32,
    text: &'a str,
    hit: u32,
    target: crate::ui::DropdownTarget,
}

/// Draws the panel of the selected tower with its hit regions.
pub fn draw(frame: &mut UiFrame, app: &App, ui: &UiAssets, width: f32, height: f32) {
    let Some(model) = model(app) else {
        return;
    };
    draw_model(frame, &model, ui, width, height);
}

/// Draws `model` (the layout and the hits; the frame builder calls this once per frame).
pub fn draw_model(frame: &mut UiFrame, model: &Model, ui: &UiAssets, width: f32, height: f32) {
    let s = UiFrame::scaler(width, height);
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    let dim = |w: f32, h: f32| Vec2::new(w * s, h * s);
    let kind = Kind::of(&model.tool);
    let size = kind.size();
    let left = PANEL_X + PAD;
    let inner_w = size.x - 2.0 * PAD;
    let right = left + inner_w;

    // Frame: `Button_UI`, colour (1, 1, 1, 0.882), border 30 at multiplier 4.
    slice(
        frame,
        ui,
        "Button_UI",
        30.0,
        0.25,
        at(PANEL_X, PANEL_Y),
        dim(size.x, size.y),
        [255, 255, 255, 225],
    );
    // The panel swallows the clicks that land on it.
    frame.hit(
        HIT_PANEL,
        at(PANEL_X, PANEL_Y),
        dim(size.x, size.y),
        UiAction::Consume,
    );

    // Title line: the title, the tower's id (not the colour killer's) and Remove.
    let title_top = PANEL_Y + PAD;
    frame.bold(kind.title(), at(left, title_top + 1.0), 26.0 * s, DARK);
    if kind != Kind::Killer {
        // `FloatValueFormat.id`: "(#31)", after the title.
        let id_text = FloatValueFormat::id().format(model.tool.save_id() as f32);
        frame.text(
            id_text,
            at(
                left + estimate_width(kind.title(), 26.0) + 14.3,
                title_top + 3.0,
            ),
            22.0 * s,
            DARK,
        );
    }
    let remove_w = kind.remove_width();
    let remove_at = at(right - remove_w, title_top);
    slice(
        frame,
        ui,
        "base_icon",
        5.0,
        1.0,
        remove_at,
        dim(remove_w, 30.0),
        [255, 255, 255, 255],
    );
    frame.text(
        "Remove",
        at(
            right - remove_w + (remove_w - estimate_width("Remove", 20.0)) * 0.5,
            title_top + 5.0,
        ),
        20.0 * s,
        DARK,
    );
    frame.hit(
        HIT_REMOVE,
        remove_at,
        dim(remove_w, 30.0),
        UiAction::Invoke(ControlCommand::RemoveTool),
    );

    // The "follow zone" row: the label and a 150-wide dropdown at the right end.
    // RUSTibites Improved: a gene selector has its gene's row (a wider box, the list of the genes) above the "follow zone" one.
    let follow_text = model.zone.labels.get(model.zone.current).map_or("None", String::as_str);
    let mut dropdowns: Vec<Dropdown> = Vec::new();
    if let Tool::Gene(selector) = &model.tool {
        dropdowns.push(Dropdown {
            label: "Gene:",
            width: 260.0,
            top: PANEL_Y + kind.row_top(ZONE),
            text: selector.def().display_name,
            hit: HIT_ZONE,
            target: crate::ui::DropdownTarget::ToolGene,
        });
        dropdowns.push(Dropdown {
            label: name_of(FOLLOW),
            width: 260.0,
            top: PANEL_Y + kind.row_top(FOLLOW),
            text: follow_text,
            hit: HIT_FOLLOW,
            target: crate::ui::DropdownTarget::ToolZone,
        });
    } else {
        dropdowns.push(Dropdown {
            label: if matches!(kind, Kind::Killer | Kind::Vent | Kind::Force | Kind::Whirl) { "Follow Zone:" } else { name_of(ZONE) },
            width: 150.0,
            top: PANEL_Y + kind.row_top(ZONE),
            text: follow_text,
            hit: HIT_ZONE,
            target: crate::ui::DropdownTarget::ToolZone,
        });
    }
    for dropdown in &dropdowns {
        let box_w = dropdown.width;
        frame.text(dropdown.label, at(left, dropdown.top + 2.0), 20.0 * s, DARK);
        let box_at = at(right - box_w, dropdown.top);
        slice(frame, ui, "UISprite", 4.0, 1.0, box_at, dim(box_w, ROW_H), [245, 245, 245, 255]);
        frame.rect(box_at, dim(box_w, 1.0), [90, 90, 90, 255]);
        frame.rect(at(right - box_w, dropdown.top + ROW_H - 1.0), dim(box_w, 1.0), [90, 90, 90, 255]);
        // A long zone name with its distance shrinks to fit the box, never under the arrow.
        let px = fit_px(dropdown.text, 18.0, box_w - 40.0);
        frame.text(dropdown.text, at(right - box_w + 9.0, dropdown.top + 3.0 + (18.0 - px) * 0.5), px * s, DARK);
        icon(frame, ui, "DropdownArrow", at(right - 25.0, dropdown.top + 2.5), dim(20.0, 20.0), [255, 255, 255, 255]);
        frame.hit(dropdown.hit, box_at, dim(box_w, ROW_H), UiAction::DropdownOpen(dropdown.target));
    }

    // The sliders.
    let (slider_box, value_box) = kind.row_boxes();
    for (index, &field) in kind.sliders().iter().enumerate() {
        let top = PANEL_Y + kind.row_top(field);
        frame.text(name_of(field), at(left, top + 2.0), 20.0 * s, DARK);
        let (x0, x1) = (left + slider_box.0, left + slider_box.1);
        // `SliderPanel`: a translucent white box (1, 1, 1, 0.392); the slider is 10 smaller.
        frame.rect(at(x0, top), dim(x1 - x0, ROW_H), [255, 255, 255, 100]);
        let (track0, track1) = (x0 + 5.0, x1 - 5.0);
        let fraction = fraction_of_tool(&model.tool, field, model.sim_size);
        let centre = track0 + fraction * (track1 - track0);
        slice(
            frame,
            ui,
            "tiled_bars_empty",
            5.0,
            1.0,
            at(track0, top + 5.0),
            dim(track1 - track0, 15.0),
            [255, 255, 255, 255],
        );
        slice(
            frame,
            ui,
            "tiled_bars_full",
            5.0,
            1.0,
            at(track0, top + 5.0),
            dim((centre + 5.5 - track0).clamp(10.0, track1 - track0), 15.0),
            [255, 255, 255, 255],
        );
        icon(
            frame,
            ui,
            "base_circle_icon",
            at(centre - 10.0, top + 2.5),
            dim(20.0, 20.0),
            [255, 255, 255, 255],
        );
        let text = value_text(&model.tool, field, model.sim_size);
        let (v0, v1) = (left + value_box.0, left + value_box.1);
        // A value wider than its 55-75 unit box ("+1.0 mN/s") shrinks to fit it.
        let px = fit_px(&text, 20.0, v1 - v0);
        frame.text(
            &text,
            at(
                v0 + ((v1 - v0) - estimate_width(&text, px)).max(0.0) * 0.5,
                top + 2.0 + (20.0 - px) * 0.5,
            ),
            px * s,
            DARK,
        );
        frame.hit(
            HIT_SLIDER + index as u32,
            at(x0, top),
            dim(x1 - x0, ROW_H),
            UiAction::ToolSlide(field, track0.round() as u16, track1.round() as u16),
        );
    }

    if let Tool::Killer(killer) = model.tool {
        draw_killer_rows(frame, ui, s, (left, right), kind, &killer);
    }
    if let Tool::Gene(selector) = model.tool {
        draw_gene_rows(frame, ui, s, (left, right), kind, &selector, model.genes_active);
    }
    if let Tool::Vent(vent) = model.tool {
        draw_vent_rows(frame, ui, s, (left, right), kind, &vent, model.vents_active);
    }
    if let Tool::Force(tower) = model.tool {
        draw_force_rows(frame, ui, s, (left, right), kind, &tower, model.forces_active);
    }
    if let Tool::Whirl(tower) = model.tool {
        draw_whirl_rows(frame, ui, s, (left, right), kind, &tower, model.whirls_active);
    }
}

/// RUSTibites Improved: switch rows (`(field, the text of the off side, the text of the on side, whether it is on)`): each a label, the two sides' words and a 50 wide switch
/// with a knob, a click area of its own; what the force and whirlpool towers' panels share.
fn draw_switch_rows(frame: &mut UiFrame, ui: &UiAssets, s: f32, left: f32, kind: Kind, rows: &[(u8, &str, &str, bool)]) {
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    let dim = |w: f32, h: f32| Vec2::new(w * s, h * s);
    for &(field, off, on, on_state) in rows {
        let top = PANEL_Y + kind.row_top(field);
        frame.text(name_of(field), at(left, top + 2.0), 20.0 * s, DARK);
        let switch_x = left + 319.05;
        frame.text(off, at(left + 304.05 - estimate_width(off, 20.0) - 6.0, top + 2.0), 20.0 * s, DARK);
        slice(frame, ui, "tiled_bars_empty", 5.0, 1.0, at(switch_x, top), dim(50.0, ROW_H), [255, 255, 255, 255]);
        let knob_x = if on_state { switch_x + 25.0 } else { switch_x };
        slice(frame, ui, "base_icon", 5.0, 1.0, at(knob_x, top), dim(25.0, ROW_H), [255, 255, 255, 255]);
        frame.text(on, at(left + 384.05 + 6.0, top + 2.0), 20.0 * s, DARK);
        frame.hit(HIT_SWITCH + u32::from(field), at(switch_x, top), dim(50.0, ROW_H), UiAction::ToolSwitch(field));
    }
}

/// RUSTibites Improved: the whirlpool tower's direction button (it says the way it turns now; a click turns it the other way), its four switches (how the speed falls off
/// with the distance, and which kinds of body it acts on) and the line of what it is doing.
fn draw_whirl_rows(
    frame: &mut UiFrame,
    ui: &UiAssets,
    s: f32,
    (left, right): (f32, f32),
    kind: Kind,
    tower: &bibites_core::whirlpool::WhirlpoolTower,
    active: bool,
) {
    use bibites_core::force_tower::ForceFalloff;
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    let dim = |w: f32, h: f32| Vec2::new(w * s, h * s);
    // The direction: a framed button as wide as the sliders and their values, with the word in it.
    let top = PANEL_Y + kind.row_top(SPIN);
    frame.text(name_of(SPIN), at(left, top + 2.0), 20.0 * s, DARK);
    let (x0, x1) = (left + 135.0, left + 484.05);
    slice(frame, ui, "base_icon", 5.0, 1.0, at(x0, top), dim(x1 - x0, ROW_H), [255, 255, 255, 255]);
    let word = match tower.spin {
        bibites_core::whirlpool::Spin::Clockwise => "Clockwise",
        bibites_core::whirlpool::Spin::Counterclockwise => "Counterclockwise",
    };
    frame.text(word, at(x0 + ((x1 - x0) - estimate_width(word, 20.0)) * 0.5, top + 2.0), 20.0 * s, DARK);
    frame.hit(HIT_SWITCH + u32::from(SPIN), at(x0, top), dim(x1 - x0, ROW_H), UiAction::ToolSwitch(SPIN));
    draw_switch_rows(
        frame,
        ui,
        s,
        left,
        kind,
        &[
            (FALLOFF, "Constant", "Fading", tower.falloff == ForceFalloff::Fading),
            (ON_BIBITES, "Off", "On", tower.bibites),
            (ON_PELLETS, "Off", "On", tower.pellets),
            (ON_EGGS, "Off", "On", tower.eggs),
        ],
    );
    let line = PANEL_Y + kind.row_top(WHIRL_LINE);
    let _ = right;
    if !active {
        frame.text("Switched off: Settings > Towers & Streams", at(left, line + 3.0), 18.0 * s, [170, 40, 30, 255]);
    } else {
        frame.text(whirl_line(tower), at(left, line + 2.0), 20.0 * s, DARK);
    }
}

/// What the whirlpool tower's line says: what it is doing now, or why it does nothing.
#[must_use]
pub fn whirl_line(tower: &bibites_core::whirlpool::WhirlpoolTower) -> String {
    if tower.speed <= 0.0 {
        "No whirl: the speed is 0".to_owned()
    } else if !(tower.bibites || tower.pellets || tower.eggs) {
        "Acts on nothing: every switch is off".to_owned()
    } else if tower.affected == 0 {
        "Nothing in its range".to_owned()
    } else {
        format!("Turning {} {}", tower.affected, if tower.affected == 1 { "body" } else { "bodies" })
    }
}

/// RUSTibites Improved: the force tower's four switches (how the force falls off with the distance, and which kinds of body it acts on) and the line of what it is
/// doing: whom it is pulling or pushing now, or why it does nothing.
fn draw_force_rows(
    frame: &mut UiFrame,
    ui: &UiAssets,
    s: f32,
    (left, right): (f32, f32),
    kind: Kind,
    tower: &bibites_core::force_tower::ForceTower,
    active: bool,
) {
    use bibites_core::force_tower::ForceFalloff;
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    draw_switch_rows(
        frame,
        ui,
        s,
        left,
        kind,
        &[
            (FALLOFF, "Constant", "Fading", tower.falloff == ForceFalloff::Fading),
            (ON_BIBITES, "Off", "On", tower.bibites),
            (ON_PELLETS, "Off", "On", tower.pellets),
            (ON_EGGS, "Off", "On", tower.eggs),
        ],
    );
    let line = PANEL_Y + kind.row_top(FORCE_LINE);
    let _ = right;
    if !active {
        frame.text("Switched off: Settings > Towers & Streams", at(left, line + 3.0), 18.0 * s, [170, 40, 30, 255]);
    } else {
        frame.text(force_line(tower), at(left, line + 2.0), 20.0 * s, DARK);
    }
}

/// What the force tower's line says: what it is doing now, or why it does nothing.
#[must_use]
pub fn force_line(tower: &bibites_core::force_tower::ForceTower) -> String {
    if tower.strength == 0.0 {
        "No force: the strength is 0".to_owned()
    } else if !(tower.bibites || tower.pellets || tower.eggs) {
        "Acts on nothing: every switch is off".to_owned()
    } else if tower.affected == 0 {
        "Nothing in its range".to_owned()
    } else {
        let verb = if tower.strength > 0.0 { "Pulling" } else { "Pushing" };
        format!("{verb} {} {}", tower.affected, if tower.affected == 1 { "body" } else { "bodies" })
    }
}

/// The text of an energy in the vent panel ("20.0 kE").
fn energy_text(energy: f32) -> String {
    FloatValueFormat { units: "E", precision: 1, si: true, ..FloatValueFormat::default() }.format(energy)
}

/// RUSTibites Improved: the hydrothermal vent's Recycle switch, the bar that shows how much of its cap its pellets fill, and the line of how many it has shot.
fn draw_vent_rows(
    frame: &mut UiFrame,
    ui: &UiAssets,
    s: f32,
    (left, right): (f32, f32),
    kind: Kind,
    vent: &bibites_core::vent::HydrothermalVent,
    active: bool,
) {
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    let dim = |w: f32, h: f32| Vec2::new(w * s, h * s);

    // The switch (the colour killer's): Off stops shooting at the cap, On recycles the oldest pellets and goes on.
    let top = PANEL_Y + kind.row_top(RECYCLE);
    frame.text(name_of(RECYCLE), at(left, top + 2.0), 20.0 * s, DARK);
    let switch_x = left + 319.05;
    frame.text("Off", at(left + 304.05 - estimate_width("Off", 20.0) - 6.0, top + 2.0), 20.0 * s, DARK);
    slice(frame, ui, "tiled_bars_empty", 5.0, 1.0, at(switch_x, top), dim(50.0, ROW_H), [255, 255, 255, 255]);
    let knob_x = if vent.recycle { switch_x + 25.0 } else { switch_x };
    slice(frame, ui, "base_icon", 5.0, 1.0, at(knob_x, top), dim(25.0, ROW_H), [255, 255, 255, 255]);
    frame.text("On", at(left + 384.05 + 6.0, top + 2.0), 20.0 * s, DARK);
    frame.hit(HIT_SWITCH + u32::from(RECYCLE), at(switch_x, top), dim(50.0, ROW_H), UiAction::ToolSwitch(RECYCLE));

    // "Filled:" and the bar of its pellets' energy against the cap (the colour killer's progress bar), with the figures on it.
    let top = PANEL_Y + kind.row_top(VENT_BAR);
    frame.text("Filled:", at(left, top + 2.0), 20.0 * s, DARK);
    let (bar0, bar1) = (left + 120.0, right);
    slice(frame, ui, "tiled_bars_1", 5.0, 1.0, at(bar0, top), dim(bar1 - bar0, ROW_H), [255, 255, 255, 255]);
    let ratio = if vent.cap > 0.0 { (vent.biomass / vent.cap).clamp(0.0, 1.0) } else { 0.0 };
    slice(frame, ui, "tiled_bars_0", 5.0, 1.0, at(bar0, top), dim(8.0 + ratio * (bar1 - bar0 - 8.0), ROW_H), [255, 199, 0, 255]);
    let text = format!("{} pellets, {} of {}", vent.alive, energy_text(vent.biomass), energy_text(vent.cap));
    let px = fit_px(&text, 20.0, bar1 - bar0 - 12.0);
    frame.text(&text, at(bar0 + ((bar1 - bar0) - estimate_width(&text, px)).max(0.0) * 0.5, top + 2.0 + (20.0 - px) * 0.5), px * s, DARK);

    // What it has done, and, when the Settings switch is off, that it does nothing.
    let line = PANEL_Y + kind.row_top(VENT_LINE);
    frame.text(format!("Shot: {}", vent.shot), at(left, line + 2.0), 20.0 * s, DARK);
    if !active {
        let note = "Switched off: Settings > Towers & Streams";
        frame.text(note, at(right - estimate_width(note, 18.0), line + 3.0), 18.0 * s, [170, 40, 30, 255]);
    }
}

/// RUSTibites Improved: the gene selector's strip that shows its range, the two switches, the progress bar and the line of what it did.
fn draw_gene_rows(
    frame: &mut UiFrame,
    ui: &UiAssets,
    s: f32,
    (left, right): (f32, f32),
    kind: Kind,
    selector: &bibites_core::gene_selector::GeneSelector,
    active: bool,
) {
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    let dim = |w: f32, h: f32| Vec2::new(w * s, h * s);
    let width = right - left;
    let def = selector.def();
    let span = (def.max - def.min).max(f32::EPSILON);
    let fraction = |value: f32| ((value - def.min) / span).clamp(0.0, 1.0);

    // "Range:" and the strip: the gene's whole range with the part that is KEPT lit (kill outside) or the part that is killed in red (kill inside).
    let strip_top = PANEL_Y + 65.0 + 140.0;
    frame.text("Range:", at(left, strip_top + 2.0), 20.0 * s, DARK);
    slice(frame, ui, "base_icon", 5.0, 1.0, at(left + 120.0, strip_top), dim(width - 120.0, 28.5), [255, 255, 255, 255]);
    let bar_x = left + 123.0;
    let bar_w = width - 126.0;
    let (low, high) = (fraction(selector.min), fraction(selector.max));
    let (inside, outside): (Rgba, Rgba) = if selector.kill_inside { ([200, 60, 50, 255], [95, 95, 95, 255]) } else { ([90, 210, 110, 255], [200, 60, 50, 255]) };
    frame.rect(at(bar_x, strip_top + 6.0), dim(bar_w, 16.5), outside);
    frame.rect(at(bar_x + low * bar_w, strip_top + 6.0), dim(((high - low) * bar_w).max(2.0), 16.5), inside);

    // The two switches: outside / inside, any / most.
    for (field, off, on, on_state) in [
        (KILL_INSIDE, "Outside", "Inside", selector.kill_inside),
        (ANY_OR_MOST, "Any", "Most", selector.any_or_most),
    ] {
        let top = PANEL_Y + kind.row_top(field);
        frame.text(if field == KILL_INSIDE { "Kill:" } else { "Pick:" }, at(left, top + 2.0), 20.0 * s, DARK);
        let switch_x = left + 319.05;
        frame.text(off, at(left + 304.05 - estimate_width(off, 20.0) - 6.0, top + 2.0), 20.0 * s, DARK);
        slice(frame, ui, "tiled_bars_empty", 5.0, 1.0, at(switch_x, top), dim(50.0, ROW_H), [255, 255, 255, 255]);
        let knob_x = if on_state { switch_x + 25.0 } else { switch_x };
        slice(frame, ui, "base_icon", 5.0, 1.0, at(knob_x, top), dim(25.0, ROW_H), [255, 255, 255, 255]);
        frame.text(on, at(left + 384.05 + 6.0, top + 2.0), 20.0 * s, DARK);
        frame.hit(HIT_SWITCH + u32::from(field), at(switch_x, top), dim(50.0, ROW_H), UiAction::ToolSwitch(field));
    }

    // The progress bar towards the next cycle (the colour killer's) and, under it, what the tower has done.
    let top = PANEL_Y + 65.0 + 355.0;
    frame.text("Progess:", at(left, top + 2.0), 20.0 * s, DARK);
    let (bar0, bar1) = (left + 120.0, right);
    slice(frame, ui, "tiled_bars_1", 5.0, 1.0, at(bar0, top), dim(bar1 - bar0, ROW_H), [255, 255, 255, 255]);
    let ratio = if selector.period > 0.0 { (selector.progress / selector.period).clamp(0.0, 1.0) } else { 0.0 };
    slice(frame, ui, "tiled_bars_0", 5.0, 1.0, at(bar0, top), dim(8.0 + ratio * (bar1 - bar0 - 8.0), ROW_H), [255, 199, 0, 255]);
    let progress = FloatValueFormat { units: "s", precision: 1, si: false, ..FloatValueFormat::default() }.format(selector.progress);
    frame.text(&progress, at(bar0 + ((bar1 - bar0) - estimate_width(&progress, 20.0)) * 0.5, top + 2.0), 20.0 * s, DARK);
    let line = top + 35.0;
    frame.text(format!("Killed: {}", selector.killed), at(left, line + 2.0), 20.0 * s, DARK);
    if !active {
        let note = "Switched off: Settings > Towers & Streams";
        frame.text(note, at(right - estimate_width(note, 18.0), line + 3.0), 18.0 * s, [170, 40, 30, 255]);
    }
}

/// The colour killer's result strip, switches and progress bar.
fn draw_killer_rows(
    frame: &mut UiFrame,
    ui: &UiAssets,
    s: f32,
    (left, right): (f32, f32),
    kind: Kind,
    killer: &bibites_core::tools::ColorKiller,
) {
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    let dim = |w: f32, h: f32| Vec2::new(w * s, h * s);
    let width = right - left;

    // "Result:" and the strip: a `base_icon` box holding the hue spectrum centred on the target
    // hue and, under it, the tolerance band (the game draws both with shader materials).
    let result_top = PANEL_Y + 65.0 + 105.0;
    frame.text("Result:", at(left, result_top + 2.0), 20.0 * s, DARK);
    let strip_top = result_top + 35.0;
    slice(
        frame,
        ui,
        "base_icon",
        5.0,
        1.0,
        at(left, strip_top),
        dim(width, 28.5),
        [255, 255, 255, 255],
    );
    let bar_x = left + 3.0;
    let bar_w = width - 6.0;
    const SEGMENTS: usize = 64;
    for segment in 0..SEGMENTS {
        let u = (segment as f32 + 0.5) / SEGMENTS as f32;
        let rgb = hue_to_rgb((killer.hue + u - 0.5).rem_euclid(1.0));
        frame.rect(
            at(
                bar_x + bar_w * segment as f32 / SEGMENTS as f32,
                strip_top + 3.0,
            ),
            dim(bar_w / SEGMENTS as f32 + 0.6, 15.0),
            [
                (rgb[0] * 255.0).round() as u8,
                (rgb[1] * 255.0).round() as u8,
                (rgb[2] * 255.0).round() as u8,
                255,
            ],
        );
    }
    // The band of hues the killer spares (`tolerance` either side of the target).
    frame.rect(
        at(bar_x, strip_top + 18.0),
        dim(bar_w, 7.5),
        [60, 60, 60, 255],
    );
    let band = (2.0 * killer.tolerance).clamp(0.0, 1.0) * bar_w;
    frame.rect(
        at(bar_x + (bar_w - band) * 0.5, strip_top + 18.0),
        dim(band, 7.5),
        [235, 235, 235, 255],
    );

    // The two switches: `Choice1`, the switch, `Choice 2`.
    for (field, off, on, on_state) in [
        (BODY_OR_EYES, "Body", "Eyes", killer.body_or_eye),
        (ANY_OR_MOST, "Any", "Most", killer.any_or_most),
    ] {
        let top = PANEL_Y + kind.row_top(field);
        frame.text(name_of(field), at(left, top + 2.0), 20.0 * s, DARK);
        // `Choice1` is 169 wide from the label column, the switch 50, `Choice 2` the rest.
        let switch_x = left + 319.05;
        frame.text(
            off,
            at(left + 304.05 - estimate_width(off, 20.0) - 6.0, top + 2.0),
            20.0 * s,
            DARK,
        );
        slice(
            frame,
            ui,
            "tiled_bars_empty",
            5.0,
            1.0,
            at(switch_x, top),
            dim(50.0, ROW_H),
            [255, 255, 255, 255],
        );
        let knob_x = if on_state { switch_x + 25.0 } else { switch_x };
        slice(
            frame,
            ui,
            "base_icon",
            5.0,
            1.0,
            at(knob_x, top),
            dim(25.0, ROW_H),
            [255, 255, 255, 255],
        );
        frame.text(on, at(left + 384.05 + 6.0, top + 2.0), 20.0 * s, DARK);
        frame.hit(
            HIT_SWITCH + u32::from(field),
            at(switch_x, top),
            dim(50.0, ROW_H),
            UiAction::ToolSwitch(field),
        );
    }

    // "Progess:" (the game's spelling) and the bar filling towards the next scan.
    let top = PANEL_Y + 65.0 + 323.5;
    frame.text("Progess:", at(left, top + 2.0), 20.0 * s, DARK);
    let (bar0, bar1) = (left + 120.0, right);
    slice(
        frame,
        ui,
        "tiled_bars_1",
        5.0,
        1.0,
        at(bar0, top),
        dim(bar1 - bar0, ROW_H),
        [255, 255, 255, 255],
    );
    // `InfoSliderHandle.ScaledX`: a cap plus the ratio of the way to the period.
    let ratio = if killer.period > 0.0 {
        (killer.progress / killer.period).clamp(0.0, 1.0)
    } else {
        0.0
    };
    slice(
        frame,
        ui,
        "tiled_bars_0",
        5.0,
        1.0,
        at(bar0, top),
        dim(8.0 + ratio * (bar1 - bar0 - 8.0), ROW_H),
        [255, 199, 0, 255],
    );
    let text = FloatValueFormat {
        units: "s",
        precision: 1,
        si: false,
        ..FloatValueFormat::default()
    }
    .format(killer.progress);
    frame.text(
        &text,
        at(
            bar0 + ((bar1 - bar0) - estimate_width(&text, 20.0)) * 0.5,
            top + 2.0,
        ),
        20.0 * s,
        DARK,
    );
}

/// The panel's outer rectangle (canvas units), for the tests of the hit regions.
#[cfg(test)]
#[must_use]
pub fn panel_rect(tool: &Tool) -> [f32; 4] {
    let size = Kind::of(tool).size();
    [PANEL_X, PANEL_Y, PANEL_X + size.x, PANEL_Y + size.y]
}

#[cfg(test)]
mod tests {
    use super::*;
    use bibites_core::tools::{ColorKiller, PheroTower, RadTower};

    fn phero() -> Tool {
        Tool::Phero(PheroTower::new(31, Vec2::ZERO))
    }

    fn rad() -> Tool {
        Tool::Rad(RadTower::new(31, Vec2::ZERO, 5000.0))
    }

    fn killer() -> Tool {
        Tool::Killer(ColorKiller::new(7, Vec2::ZERO))
    }

    #[test]
    fn each_panel_has_the_prefabs_size_title_and_sliders() {
        assert_eq!(Kind::of(&phero()).title(), "Pheromone Tower");
        assert_eq!(Kind::of(&rad()).title(), "RadTower");
        assert_eq!(Kind::of(&killer()).title(), "Color Selector Editior");
        assert_eq!(Kind::Phero.sliders(), &[RED, GREEN, BLUE, HEADING]);
        assert_eq!(Kind::Rad.sliders(), &[COUNT, INTENSITY, RADIUS, TAPER]);
        assert_eq!(
            Kind::Killer.sliders(),
            &[HUE, TOLERANCE, SCAN_RADIUS, PERIOD]
        );
        assert_eq!(panel_rect(&killer()), [10.0, 10.0, 524.05, 438.5]);
    }

    #[test]
    fn values_read_like_the_games_formats() {
        // Fresh towers: `PheroTower` strengths 0 u², heading 0; `RadTower` +1.0 mN/s, +0.1 %/s.
        assert_eq!(value_text(&phero(), RED, 5000.0), "0.0 u²");
        assert_eq!(value_text(&phero(), HEADING, 5000.0), "0.00");
        assert_eq!(value_text(&rad(), COUNT, 5000.0), "+1.0 mN/s");
        assert_eq!(value_text(&rad(), INTENSITY, 5000.0), "+0.1%/s");
        assert_eq!(value_text(&rad(), RADIUS, 5000.0), "250.0 u");
        assert_eq!(value_text(&rad(), TAPER, 5000.0), "1.0");
        // The colour killer's prefab values: hue 0, tolerance 0.1, radius 400, period 100.
        assert_eq!(value_text(&killer(), HUE, 5000.0), "0.00");
        assert_eq!(value_text(&killer(), TOLERANCE, 5000.0), "10.0%");
        assert_eq!(value_text(&killer(), SCAN_RADIUS, 5000.0), "400u");
        assert_eq!(value_text(&killer(), PERIOD, 5000.0), "100s");
        assert_eq!(FloatValueFormat::id().format(31.0), "(#31)");
    }

    #[test]
    fn slider_positions_round_trip_for_every_field() {
        let sim = 5000.0;
        for (tool, fields) in [
            (phero(), Kind::Phero.sliders()),
            (rad(), Kind::Rad.sliders()),
            (killer(), Kind::Killer.sliders()),
        ] {
            for &field in fields {
                let def = slider_def(field, sim).expect("slider");
                for fraction in [0.0_f32, 0.25, 0.5, 0.9, 1.0] {
                    let value = value_range(def.mode, def.min, def.max, def.default, fraction);
                    let back = fraction_range(def.mode, def.min, def.max, def.default, value);
                    assert!(
                        (back - fraction).abs() < 1e-3,
                        "field {field} fraction {fraction} -> {value} -> {back}"
                    );
                    assert!(
                        value >= def.min - 1e-4 && value <= def.max * 1.0001 + 1e-4,
                        "field {field} fraction {fraction}: {value} outside {}..{}",
                        def.min,
                        def.max
                    );
                }
                // The current value of a fresh tool sits inside the slider.
                let at = fraction_of_tool(&tool, field, sim);
                assert!((0.0..=1.0).contains(&at), "field {field} at {at}");
            }
        }
    }

    /// The frame of a model drawn at the reference size (scale 1).
    fn frame_of(tool: Tool) -> UiFrame {
        let model = Model {
            zone: ZoneChoices {
                zones: vec![None],
                labels: vec!["None".to_owned()],
                current: 0,
            },
            sim_size: 5000.0,
            genes_active: true,
            vents_active: true,
            forces_active: true,
            whirls_active: true,
            tool,
        };
        let mut frame = UiFrame::default();
        draw_model(&mut frame, &model, &UiAssets::empty(), 1920.0, 1080.0);
        frame
    }

    fn gene() -> Tool {
        Tool::Gene(bibites_core::gene_selector::GeneSelector::new(9, Vec2::ZERO))
    }

    fn force() -> Tool {
        Tool::Force(bibites_core::force_tower::ForceTower::new(11, Vec2::ZERO))
    }

    #[test]
    fn the_force_panel_has_its_dropdown_two_sliders_four_switches_and_remove_each_with_a_click_area_of_its_own_and_reads_in_its_units() {
        let tool = force();
        assert_eq!(Kind::of(&tool).title(), "Force Tower");
        assert_eq!(Kind::Force.sliders(), &[FORCE_RANGE, STRENGTH]);
        let frame = frame_of(tool);
        let [x0, y0, x1, y1] = panel_rect(&tool);
        assert_eq!(frame.action_at(Vec2::new(x0 + 3.0, y1 - 3.0)), Some(UiAction::Consume), "the panel swallows the clicks that are not a control");
        assert_eq!(frame.action_at(Vec2::new(x1 + 5.0, y0 + 40.0)), None);
        let actions: Vec<UiAction> = frame.actions.iter().map(|(_, action)| *action).collect();
        assert!(actions.contains(&UiAction::Invoke(ControlCommand::RemoveTool)));
        assert!(actions.contains(&UiAction::DropdownOpen(crate::ui::DropdownTarget::ToolZone)), "the follow-zone dropdown");
        for &field in Kind::Force.sliders() {
            assert!(actions.iter().any(|action| matches!(action, UiAction::ToolSlide(f, _, _) if *f == field)), "a slider for field {field}");
        }
        for field in [FALLOFF, ON_BIBITES, ON_PELLETS, ON_EGGS] {
            assert!(actions.contains(&UiAction::ToolSwitch(field)), "a switch for field {field}");
        }
        let mut ids: Vec<u32> = frame.actions.iter().map(|(id, _)| *id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "the click areas of the force panel have distinct ids");
        // A new tower: a pull of 100 that reaches 300 u.
        assert_eq!(value_text(&tool, FORCE_RANGE, 5000.0), "300u");
        let strength = value_text(&tool, STRENGTH, 5000.0);
        assert!(strength.starts_with("+100") && strength.contains("u/s"), "{strength}");
        let mut pushing = tower_of(tool);
        pushing.set_strength(-250.0);
        let text = value_text(&Tool::Force(pushing), STRENGTH, 5000.0);
        assert!(text.starts_with("-250"), "{text}");
    }

    fn whirl() -> Tool {
        Tool::Whirl(bibites_core::whirlpool::WhirlpoolTower::new(12, Vec2::ZERO))
    }

    /// The whirlpool tower of a `Tool`.
    fn whirl_of(tool: Tool) -> bibites_core::whirlpool::WhirlpoolTower {
        match tool {
            Tool::Whirl(tower) => tower,
            _ => panic!("a whirlpool tower"),
        }
    }

    #[test]
    fn the_whirl_panel_has_its_dropdown_two_sliders_the_direction_button_four_switches_and_remove_each_with_a_click_area_of_its_own_and_reads_in_its_units() {
        let tool = whirl();
        assert_eq!(Kind::of(&tool).title(), "Whirlpool Tower");
        assert_eq!(Kind::Whirl.sliders(), &[FORCE_RANGE, WHIRL_SPEED]);
        let frame = frame_of(tool);
        let [x0, y0, x1, y1] = panel_rect(&tool);
        assert_eq!(frame.action_at(Vec2::new(x0 + 3.0, y1 - 3.0)), Some(UiAction::Consume), "the panel swallows the clicks that are not a control");
        assert_eq!(frame.action_at(Vec2::new(x1 + 5.0, y0 + 40.0)), None);
        let actions: Vec<UiAction> = frame.actions.iter().map(|(_, action)| *action).collect();
        assert!(actions.contains(&UiAction::Invoke(ControlCommand::RemoveTool)));
        assert!(actions.contains(&UiAction::DropdownOpen(crate::ui::DropdownTarget::ToolZone)), "the follow-zone dropdown");
        for &field in Kind::Whirl.sliders() {
            assert!(actions.iter().any(|action| matches!(action, UiAction::ToolSlide(f, _, _) if *f == field)), "a slider for field {field}");
        }
        for field in [SPIN, FALLOFF, ON_BIBITES, ON_PELLETS, ON_EGGS] {
            assert!(actions.contains(&UiAction::ToolSwitch(field)), "a switch for field {field}");
        }
        let mut ids: Vec<u32> = frame.actions.iter().map(|(id, _)| *id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "the click areas of the whirlpool panel have distinct ids");
        // The rows do not overlap: top to bottom, with the line below the last switch, inside the panel.
        let order = [FORCE_RANGE, WHIRL_SPEED, SPIN, FALLOFF, ON_BIBITES, ON_PELLETS, ON_EGGS, WHIRL_LINE];
        let tops: Vec<f32> = order.iter().map(|field| Kind::Whirl.row_top(*field)).collect();
        assert!(tops.windows(2).all(|pair| pair[1] - pair[0] >= 35.0), "{tops:?}");
        assert!(tops.last().copied().unwrap_or(0.0) + 25.0 <= Kind::Whirl.size().y, "the line is inside the panel");
        // A new tower: a speed of 100 that reaches 300 u.
        assert_eq!(value_text(&tool, FORCE_RANGE, 5000.0), "300u");
        let speed = value_text(&tool, WHIRL_SPEED, 5000.0);
        assert!(speed.starts_with("100") && speed.contains("u/s") && !speed.starts_with('+'), "{speed}");
    }

    #[test]
    fn the_whirl_panel_says_the_way_it_turns_and_the_direction_button_flips_it() {
        let texts = |tool: Tool| {
            let model = Model {
                zone: ZoneChoices { zones: vec![None], labels: vec!["None".to_owned()], current: 0 },
                sim_size: 5000.0,
                genes_active: true,
                vents_active: true,
                forces_active: true,
                whirls_active: true,
                tool,
            };
            let mut frame = UiFrame::default();
            draw_model(&mut frame, &model, &UiAssets::empty(), 1920.0, 1080.0);
            frame.draws.iter().filter_map(|draw| if let UiDraw::Text { text, .. } = draw { Some(text.clone()) } else { None }).collect::<Vec<String>>()
        };
        let counter = texts(whirl());
        assert!(counter.iter().any(|text| text == "Counterclockwise") && !counter.iter().any(|text| text == "Clockwise"), "{counter:?}");
        let mut turned = whirl_of(whirl());
        turned.spin = turned.spin.reversed();
        let clock = texts(Tool::Whirl(turned));
        assert!(clock.iter().any(|text| text == "Clockwise") && !clock.iter().any(|text| text == "Counterclockwise"), "{clock:?}");
        for label in ["Follow Zone:", "Range:", "Speed:", "Direction:", "Falloff:", "Bibites:", "Pellets:", "Eggs:", "Constant", "Fading"] {
            assert!(counter.iter().any(|text| text == label), "{label}: {counter:?}");
        }
        // The panel's own flip changes the tower in a world.
        let mut world = crate::app::LoadedWorld::load(None, 77, 6, 1).expect("starter world").world;
        let id = world.place_whirlpool_at(Vec2::ZERO);
        flip_switch(&mut world, id, SPIN);
        assert_eq!(whirl_of(*world.tools.get(id).expect("tool")).spin, bibites_core::whirlpool::Spin::Clockwise);
        flip_switch(&mut world, id, SPIN);
        assert_eq!(whirl_of(*world.tools.get(id).expect("tool")).spin, bibites_core::whirlpool::Spin::Counterclockwise);
    }

    #[test]
    fn the_speed_slider_runs_from_none_to_the_fastest_and_the_whirl_panels_line_says_what_the_tower_is_doing_or_why_it_does_nothing() {
        let def = slider_def(WHIRL_SPEED, 5000.0).expect("slider");
        let at = |fraction: f32| value_range(def.mode, def.min, def.max, def.default, fraction);
        assert!(at(0.0).abs() < 1e-3 && (at(1.0) - bibites_core::whirlpool::WHIRL_SPEED_MAX).abs() < 1e-3);
        let mut tower = whirl_of(whirl());
        assert_eq!(whirl_line(&tower), "Nothing in its range");
        tower.affected = 1;
        assert_eq!(whirl_line(&tower), "Turning 1 body");
        tower.affected = 12;
        assert_eq!(whirl_line(&tower), "Turning 12 bodies");
        tower.set_speed(0.0);
        assert_eq!(whirl_line(&tower), "No whirl: the speed is 0");
        tower.set_speed(40.0);
        (tower.bibites, tower.pellets, tower.eggs) = (false, false, false);
        assert_eq!(whirl_line(&tower), "Acts on nothing: every switch is off");
        // Switched off in the Settings: the panel says so instead.
        let model = Model {
            zone: ZoneChoices { zones: vec![None], labels: vec!["None".to_owned()], current: 0 },
            sim_size: 5000.0,
            genes_active: true,
            vents_active: true,
            forces_active: true,
            whirls_active: false,
            tool: whirl(),
        };
        let mut frame = UiFrame::default();
        draw_model(&mut frame, &model, &UiAssets::empty(), 1920.0, 1080.0);
        let shown: Vec<String> = frame.draws.iter().filter_map(|draw| if let UiDraw::Text { text, .. } = draw { Some(text.clone()) } else { None }).collect();
        assert!(shown.iter().any(|text| text.contains("Switched off: Settings > Towers & Streams")), "{shown:?}");
    }

    /// The force tower of a `Tool`.
    fn tower_of(tool: Tool) -> bibites_core::force_tower::ForceTower {
        match tool {
            Tool::Force(tower) => tower,
            _ => panic!("a force tower"),
        }
    }

    #[test]
    fn the_strength_slider_is_a_straight_line_from_the_strongest_push_through_nothing_to_the_strongest_pull() {
        let def = slider_def(STRENGTH, 5000.0).expect("slider");
        let at = |fraction: f32| value_range(def.mode, def.min, def.max, def.default, fraction);
        assert!((at(0.0) - bibites_core::force_tower::FORCE_STRENGTH_MIN).abs() < 1e-3);
        assert!(at(0.5).abs() < 1e-3, "the middle is no force");
        assert!((at(1.0) - bibites_core::force_tower::FORCE_STRENGTH_MAX).abs() < 1e-3);
        // The default (a pull of 100) sits right of the middle, and the range reads back from its fraction.
        assert!(fraction_of_tool(&force(), STRENGTH, 5000.0) > 0.5);
        for field in [FORCE_RANGE, STRENGTH] {
            let def = slider_def(field, 5000.0).expect("slider");
            for fraction in [0.0_f32, 0.25, 0.5, 0.9, 1.0] {
                let value = value_range(def.mode, def.min, def.max, def.default, fraction);
                let back = fraction_range(def.mode, def.min, def.max, def.default, value);
                assert!((back - fraction).abs() < 1e-3, "field {field} fraction {fraction} -> {value} -> {back}");
            }
        }
    }

    #[test]
    fn the_force_panels_line_says_what_the_tower_is_doing_or_why_it_does_nothing() {
        let mut tower = tower_of(force());
        assert_eq!(force_line(&tower), "Nothing in its range");
        tower.affected = 1;
        assert_eq!(force_line(&tower), "Pulling 1 body");
        tower.affected = 12;
        tower.set_strength(-40.0);
        assert_eq!(force_line(&tower), "Pushing 12 bodies");
        tower.set_strength(0.0);
        assert_eq!(force_line(&tower), "No force: the strength is 0");
        tower.set_strength(40.0);
        (tower.bibites, tower.pellets, tower.eggs) = (false, false, false);
        assert_eq!(force_line(&tower), "Acts on nothing: every switch is off");
    }

    #[test]
    fn the_gene_selector_panel_has_its_gene_dropdown_range_sliders_switches_and_remove_and_reads_in_the_genes_units() {
        let tool = gene();
        assert_eq!(Kind::of(&tool).title(), "Gene Selector");
        assert_eq!(Kind::Gene.sliders(), &[GENE_MIN, GENE_MAX, KILLS, SCAN_RADIUS, PERIOD]);
        let frame = frame_of(tool);
        let [x0, y0, x1, y1] = panel_rect(&tool);
        assert_eq!(frame.action_at(Vec2::new(x0 + 3.0, y1 - 3.0)), Some(UiAction::Consume), "the panel swallows the clicks that are not a control");
        assert_eq!(frame.action_at(Vec2::new(x1 + 5.0, y0 + 40.0)), None);
        let actions: Vec<UiAction> = frame.actions.iter().map(|(_, action)| *action).collect();
        assert!(actions.contains(&UiAction::Invoke(ControlCommand::RemoveTool)));
        assert!(actions.contains(&UiAction::DropdownOpen(crate::ui::DropdownTarget::ToolGene)), "the gene dropdown");
        assert!(actions.contains(&UiAction::DropdownOpen(crate::ui::DropdownTarget::ToolZone)), "the follow-zone dropdown, as the colour killer has it");
        // Every control has a click area of its own (no two share an id).
        let mut ids: Vec<u32> = frame.actions.iter().map(|(id, _)| *id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "the click areas of the gene panel have distinct ids");
        for &field in Kind::Gene.sliders() {
            assert!(actions.iter().any(|action| matches!(action, UiAction::ToolSlide(f, _, _) if *f == field)), "a slider for field {field}");
        }
        assert!(actions.contains(&UiAction::ToolSwitch(KILL_INSIDE)) && actions.contains(&UiAction::ToolSwitch(ANY_OR_MOST)));
        // A new tower: Diet over its whole range, one kill a cycle, the colour killer's radius and period; the values read in the gene's own units.
        let def = bibites_core::genome::GENE_DEFS[bibites_core::genome::Gene::Diet as usize];
        let format = slider_def_for(&tool, GENE_MIN, 5000.0).expect("the range slider").format;
        assert_eq!(value_text(&tool, GENE_MIN, 5000.0), format.format(def.min));
        assert_eq!(value_text(&tool, GENE_MAX, 5000.0), format.format(def.max));
        assert_eq!(value_text(&tool, KILLS, 5000.0), "1");
        assert_eq!(value_text(&tool, SCAN_RADIUS, 5000.0), "400u");
        assert_eq!(value_text(&tool, PERIOD, 5000.0), "100s");
        assert_eq!((fraction_of_tool(&tool, GENE_MIN, 5000.0), fraction_of_tool(&tool, GENE_MAX, 5000.0)), (0.0, 1.0));
        // The two range sliders follow the gene: another gene, another scale.
        let Tool::Gene(mut selector) = tool else { unreachable!() };
        selector.set_gene(bibites_core::genome::Gene::SizeRatio as u8);
        let size = bibites_core::genome::GENE_DEFS[bibites_core::genome::Gene::SizeRatio as usize];
        let def_for = slider_def_for(&Tool::Gene(selector), GENE_MAX, 5000.0).expect("slider");
        assert_eq!((def_for.min, def_for.max), (size.min, size.max));
    }

    fn vent() -> Tool {
        Tool::Vent(bibites_core::vent::HydrothermalVent::new(12, Vec2::ZERO, 200.0))
    }

    #[test]
    fn the_vent_panel_has_a_follow_zone_dropdown_four_sliders_a_recycle_switch_and_remove_and_reads_in_the_vents_units() {
        let tool = vent();
        assert_eq!(Kind::of(&tool).title(), "Hydrothermal Vent");
        assert_eq!(Kind::Vent.sliders(), &[VELOCITY, RANGE, PELLET_SIZE, RATE, CAP]);
        let frame = frame_of(tool);
        let [x0, y0, x1, y1] = panel_rect(&tool);
        assert_eq!(frame.action_at(Vec2::new(x0 + 3.0, y1 - 3.0)), Some(UiAction::Consume), "the panel swallows the clicks that are not a control");
        assert_eq!(frame.action_at(Vec2::new(x1 + 5.0, y0 + 40.0)), None);
        let actions: Vec<UiAction> = frame.actions.iter().map(|(_, action)| *action).collect();
        assert!(actions.contains(&UiAction::Invoke(ControlCommand::RemoveTool)));
        assert!(actions.contains(&UiAction::DropdownOpen(crate::ui::DropdownTarget::ToolZone)), "the vent can follow a zone");
        assert!(actions.contains(&UiAction::ToolSwitch(RECYCLE)));
        for &field in Kind::Vent.sliders() {
            assert!(actions.iter().any(|action| matches!(action, UiAction::ToolSlide(f, _, _) if *f == field)), "a slider for field {field}");
        }
        let mut ids: Vec<u32> = frame.actions.iter().map(|(id, _)| *id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "the click areas of the vent panel have distinct ids");
        // Every row is inside the panel.
        for field in [VELOCITY, RANGE, PELLET_SIZE, RATE, CAP, RECYCLE, VENT_BAR, VENT_LINE] {
            assert!(PANEL_Y + Kind::Vent.row_top(field) + ROW_H <= y1 - 10.0, "row {field} inside the panel");
        }
        // A new vent: the default velocity, the world's pellet size, two shots a second and a cap of twenty thousand.
        assert_eq!(value_text(&tool, VELOCITY, 5000.0), "250u/s");
        assert_eq!(value_text(&tool, RANGE, 5000.0), "200u");
        assert_eq!(value_text(&tool, PELLET_SIZE, 5000.0), "200E");
        assert_eq!(value_text(&tool, RATE, 5000.0), "2.0/s");
        let cap = value_text(&tool, CAP, 5000.0);
        assert!(cap.starts_with("20.0") && cap.ends_with("kE"), "{cap}");
    }

    #[test]
    fn the_vent_sliders_run_over_their_ranges_and_their_positions_round_trip() {
        let tool = vent();
        for (field, min, max) in [
            (VELOCITY, bibites_core::vent::VENT_VELOCITY_MIN, bibites_core::vent::VENT_VELOCITY_MAX),
            (RANGE, bibites_core::vent::VENT_RANGE_MIN, bibites_core::vent::VENT_RANGE_MAX),
            (PELLET_SIZE, bibites_core::vent::VENT_PELLET_MIN, bibites_core::vent::VENT_PELLET_MAX),
            (RATE, bibites_core::vent::VENT_RATE_MIN, bibites_core::vent::VENT_RATE_MAX),
            (CAP, bibites_core::vent::VENT_CAP_MIN, bibites_core::vent::VENT_CAP_MAX),
        ] {
            let def = slider_def_for(&tool, field, 5000.0).expect("a slider");
            assert_eq!((def.min, def.max), (min, max));
            assert!((value_range(def.mode, def.min, def.max, def.default, 0.0) - min).abs() < 1e-3 * min, "field {field}: the left end is the lowest value");
            assert!((value_range(def.mode, def.min, def.max, def.default, 1.0) - max).abs() < 1e-3 * max, "field {field}: the right end is the highest value");
            for fraction in [0.1_f32, 0.35, 0.5, 0.8] {
                let value = value_range(def.mode, def.min, def.max, def.default, fraction);
                let back = fraction_range(def.mode, def.min, def.max, def.default, value);
                assert!((back - fraction).abs() < 1e-3, "field {field}: {fraction} -> {value} -> {back}");
            }
            // The new vent's value is somewhere inside the slider.
            assert!((0.0..=1.0).contains(&fraction_of_tool(&tool, field, 5000.0)), "field {field}");
        }
        // The values go through the setters: a slider cannot put a vent outside its ranges.
        let mut edited = vent();
        write_value(&mut edited, RATE, 1.0e9);
        write_value(&mut edited, CAP, -1.0);
        let Tool::Vent(after) = edited else { unreachable!() };
        assert_eq!((after.rate, after.cap), (bibites_core::vent::VENT_RATE_MAX, bibites_core::vent::VENT_CAP_MIN));
    }

    #[test]
    fn the_vent_panel_shows_that_it_is_switched_off_and_how_full_its_cap_is() {
        let texts = |vents_active: bool, tool: Tool| {
            let model = Model {
                zone: ZoneChoices { zones: vec![None], labels: vec!["None".to_owned()], current: 0 },
                sim_size: 5000.0,
                genes_active: true,
                vents_active,
                forces_active: true,
                whirls_active: true,
                tool,
            };
            let mut frame = UiFrame::default();
            draw_model(&mut frame, &model, &UiAssets::empty(), 1920.0, 1080.0);
            frame.draws.iter().filter_map(|draw| if let UiDraw::Text { text, .. } = draw { Some(text.clone()) } else { None }).collect::<Vec<String>>()
        };
        assert!(!texts(true, vent()).iter().any(|text| text.contains("Switched off")));
        assert!(texts(false, vent()).iter().any(|text| text.contains("Switched off")));
        let Tool::Vent(mut full) = vent() else { unreachable!() };
        full.alive = 37;
        full.biomass = 7400.0;
        full.shot = 412;
        let shown = texts(true, Tool::Vent(full));
        assert!(shown.iter().any(|text| text.contains("37 pellets") && text.contains("7.4 kE") && text.contains("20.0 kE")), "{shown:?}");
        assert!(shown.iter().any(|text| text == "Shot: 412"), "{shown:?}");
    }

    #[test]
    fn the_force_panel_shows_that_it_is_switched_off_instead_of_what_it_is_doing() {
        let texts = |forces_active: bool| {
            let model = Model {
                zone: ZoneChoices { zones: vec![None], labels: vec!["None".to_owned()], current: 0 },
                sim_size: 5000.0,
                genes_active: true,
                vents_active: true,
                forces_active,
                whirls_active: true,
                tool: force(),
            };
            let mut frame = UiFrame::default();
            draw_model(&mut frame, &model, &UiAssets::empty(), 1920.0, 1080.0);
            frame.draws.iter().filter_map(|draw| if let UiDraw::Text { text, .. } = draw { Some(text.clone()) } else { None }).collect::<Vec<String>>()
        };
        let on = texts(true);
        assert!(on.iter().any(|text| text == "Nothing in its range") && !on.iter().any(|text| text.contains("Switched off")), "{on:?}");
        let off = texts(false);
        assert!(off.iter().any(|text| text.contains("Switched off: Settings > Towers & Streams")) && !off.iter().any(|text| text == "Nothing in its range"), "{off:?}");
        for label in ["Follow Zone:", "Range:", "Strength:", "Falloff:", "Bibites:", "Pellets:", "Eggs:", "Constant", "Fading"] {
            assert!(on.iter().any(|text| text == label), "{label}: {on:?}");
        }
    }

    #[test]
    fn the_gene_selector_panel_shows_that_it_is_switched_off() {
        let model = |genes_active| Model {
            zone: ZoneChoices { zones: vec![None], labels: vec!["None".to_owned()], current: 0 },
            sim_size: 5000.0,
            genes_active,
            vents_active: genes_active,
            forces_active: genes_active,
            whirls_active: true,
            tool: gene(),
        };
        let texts = |model: &Model| {
            let mut frame = UiFrame::default();
            draw_model(&mut frame, model, &UiAssets::empty(), 1920.0, 1080.0);
            frame.draws.iter().filter_map(|draw| if let UiDraw::Text { text, .. } = draw { Some(text.clone()) } else { None }).collect::<Vec<String>>()
        };
        assert!(!texts(&model(true)).iter().any(|text| text.contains("Switched off")));
        assert!(texts(&model(false)).iter().any(|text| text.contains("Switched off")));
    }

    #[test]
    fn the_panel_swallows_clicks_and_has_a_remove_button_a_dropdown_and_its_sliders() {
        for tool in [phero(), rad(), killer()] {
            let frame = frame_of(tool);
            let [x0, y0, x1, y1] = panel_rect(&tool);
            // Anywhere on the panel that is not a control the click is consumed.
            assert_eq!(
                frame.action_at(Vec2::new(x0 + 3.0, y1 - 3.0)),
                Some(UiAction::Consume)
            );
            assert_eq!(frame.action_at(Vec2::new(x1 + 5.0, y0 + 40.0)), None);
            let actions: Vec<UiAction> = frame.actions.iter().map(|(_, action)| *action).collect();
            assert!(actions.contains(&UiAction::Invoke(ControlCommand::RemoveTool)));
            assert!(actions.contains(&UiAction::DropdownOpen(crate::ui::DropdownTarget::ToolZone)));
            let kind = Kind::of(&tool);
            for &field in kind.sliders() {
                let slide = actions
                    .iter()
                    .find(|action| matches!(action, UiAction::ToolSlide(f, _, _) if *f == field));
                assert!(slide.is_some(), "{kind:?} has no slider for field {field}");
            }
            assert_eq!(
                actions
                    .iter()
                    .filter(|action| matches!(action, UiAction::ToolSwitch(_)))
                    .count(),
                if kind == Kind::Killer { 2 } else { 0 }
            );
        }
    }

    #[test]
    fn a_slider_hit_reports_the_track_the_cursor_is_measured_against() {
        let frame = frame_of(phero());
        let [x0, y0, ..] = panel_rect(&phero());
        // The first slider row: 65.47 + 50 below the panel's top; its track starts 15 + 135 + 5
        // units from the panel's left edge and runs 319.48 units.
        let row_y = y0 + 65.47 + 50.0 + 12.0;
        let track0 = x0 + 15.0 + 135.0 + 5.0;
        let Some(UiAction::ToolSlide(field, a, b)) =
            frame.action_at(Vec2::new(track0 + 40.0, row_y))
        else {
            panic!("no slider under the cursor");
        };
        assert_eq!(field, RED);
        assert_eq!(f32::from(a), track0.round());
        assert_eq!(f32::from(b), (track0 + 319.48).round());
        // The "Remove" button sits at the right end of the title line.
        let remove = frame.action_at(Vec2::new(x0 + 15.0 + 554.48 - 20.0, y0 + 15.0 + 12.0));
        assert_eq!(remove, Some(UiAction::Invoke(ControlCommand::RemoveTool)));
    }

    #[test]
    fn the_colour_killer_panel_is_taller_and_has_its_two_switches() {
        let frame = frame_of(killer());
        let [x0, y0, ..] = panel_rect(&killer());
        // "Select For :" row: 65 + 183.5 below the top; the switch is 319 units from the content.
        let switch = frame.action_at(Vec2::new(
            x0 + 15.0 + 319.05 + 10.0,
            y0 + 65.0 + 183.5 + 10.0,
        ));
        assert_eq!(switch, Some(UiAction::ToolSwitch(BODY_OR_EYES)));
        let switch = frame.action_at(Vec2::new(
            x0 + 15.0 + 319.05 + 10.0,
            y0 + 65.0 + 218.5 + 10.0,
        ));
        assert_eq!(switch, Some(UiAction::ToolSwitch(ANY_OR_MOST)));
    }

    #[test]
    fn the_logarithmic_sliders_span_four_decades_up_to_their_maximum() {
        let red = slider_def(RED, 5000.0).expect("red");
        // Slider space is log10(value / 1) from -2 to 2, so the left end reads the setting's
        // minimum (0) and the next values start at 0.01.
        assert_eq!(
            value_range(red.mode, red.min, red.max, red.default, 0.0),
            0.0
        );
        let near = value_range(red.mode, red.min, red.max, red.default, 0.01);
        assert!(near > 0.0 && near < 0.02, "{near}");
        let mid = value_range(red.mode, red.min, red.max, red.default, 0.5);
        assert!((mid - 1.0).abs() < 1e-3, "{mid}");
        assert!((value_range(red.mode, red.min, red.max, red.default, 1.0) - 100.0).abs() < 1e-2);
    }
}

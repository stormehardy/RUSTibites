//! The Position tab of the species panels (`SpeciesDistributionPanel`): a time slider over the species' history log,
//! the map of the zones as they were at that moment (`SimZonePreviewer`, `ZonePreview`) with the ellipse where the
//! species was (`SpeciesDistributionItem`), and the zones the ellipse overlaps (`ZoneMatch`).
//!
//! Prefab values (level2, `Species PosPanel`): a 420 wide `base_icon` panel (0.84 grey) over a vertical layout with
//! padding 10 / 10 / 5 / 10 and spacing 5: the title "Species Distribution over time" (Roboto-Bold 18, 21.1 high); the
//! slider row (400 x 30: a 315 wide `BasicSlider` and the 90 wide time text, Roboto-Bold 18, flush right); the map, a
//! 400 x 400 navy (0, 0, 0.106) square with the 300 x 300 preview square in its middle (the zones' parent, so a
//! point at `posX` is `150 posX` from its middle); and the body, 5 down, with "Possible Zone Presence: " and the
//! lines of the zones (Roboto-Bold 13, black, 15.24 apart). The panel is as tall as its layout asks (536.81 for two
//! zone lines).
//!
//! The slider (`Slider` right to left, whole numbers, 295 x 20 inside its 315 x 30 `Background` panel) runs from
//! the point after the species' disappearance (or `-1`, "Now", while it is alive) to its apparition, the newest point
//! at the right. Like the game, the panel is filled when a species is selected and again for every slider move:
//! what it shows is that moment's snapshot, not a live view.
//!
//! Each zone carries its labels (`ZonePreview`: the name, the biomass as `125.66 kE` and the pellet count as `~26 pellets`, stacked over its
//! middle), here and in the Zones tab's preview ([`emit_zone_labels`]).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bibites_core::format::FloatValueFormat;
use bibites_core::ids::SpeciesId;
use bibites_core::math::Vec2;
use bibites_core::species::SpeciesListEntry;
use bibites_core::species_log::{SpeciesDataPoint, SpeciesLog};
use bibites_core::zone::{ZoneConfig, ZoneData};
use bibites_core::zone_log::ZoneDataPoint;

use super::{BLACK, Drawer, FRAME_GREY, INK, OVERLAY, WHITE};
use crate::app::App;
use crate::scenario_editor::PreviewShape;
use crate::time_format::{TimeFormat, Timescale};
use crate::ui::{Rgba, UiAction, UiDraw, UiFrame, UiTexture};

/// Hit ids of the slider's track and handle (8950 - 8999 belong to the species panels).
const HIT_TRACK: u32 = 8996;
const HIT_HANDLE: u32 = 8997;
/// The 13 px lines' pitch (`Line` is 15.24 high).
const LINE: f32 = 13.0 * 1.1719;
/// The map and the preview square in it.
const MAP: f32 = 400.0;
const PREVIEW: f32 = 300.0;
/// The map's and the preview's colour: (0, 0, 0.106).
const NAVY: Rgba = [0, 0, 27, 255];
/// The disclaimer's cover: (0.39, 0.39, 0.39) at 0.561.
const COVER: Rgba = [99, 99, 99, 143];
/// The species' ellipse: white at 0.239.
const ELLIPSE: Rgba = [255, 255, 255, 61];
/// The disclaimer text's box and font size.
const DISCLAIMER_BOX: [f32; 2] = [241.0, 78.4];
const DISCLAIMER_PX: f32 = 16.7;
const DISCLAIMER: &str =
    "The species was either too scarce or not present \n\nDistribution can't be displayed";

/// Where the pieces of the tab are (reference px, `x, y, w, h`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// The panel.
    pub panel: [f32; 4],
    /// The title's line.
    pub title: [f32; 4],
    /// The slider's `Background` panel.
    pub slider_panel: [f32; 4],
    /// The slider itself: the panel inset by 10 and 5.
    pub slider: [f32; 4],
    /// The time text.
    pub time_text: [f32; 4],
    /// The map.
    pub map: [f32; 4],
    /// The preview square in the middle of the map.
    pub preview: [f32; 4],
    /// "Possible Zone Presence: ".
    pub caption: [f32; 4],
    /// The zone lines.
    pub zones: [f32; 4],
}

/// The tab's layout when the zone text has `zone_lines` lines and the time text is `time_width` wide: the vertical
/// layout puts the pieces under each other and the panel is as tall as they are; in the slider row the time text
/// takes its width (at least 90) off the end of the 405 and the slider panel the rest.
#[must_use]
pub fn layout(zone_lines: usize, time_width: f32) -> Layout {
    let (x, y) = (OVERLAY.x + 10.0, OVERLAY.y);
    let text_w = time_width.max(90.0);
    let row = y + 31.1;
    let map_top = y + 66.1;
    let body = y + 471.1;
    let zones_h = LINE * zone_lines.max(1) as f32;
    let height = 471.1 + (5.0 + LINE + 5.0 + zones_h) + 10.0;
    Layout {
        panel: [OVERLAY.x, OVERLAY.y, 420.0, height],
        title: [x, y + 5.0, 242.96, 21.1],
        slider_panel: [x, row, 405.0 - text_w, 30.0],
        slider: [x + 10.0, row + 5.0, 385.0 - text_w, 20.0],
        time_text: [x + 405.0 - text_w, row + 4.45, text_w, 21.1],
        map: [x, map_top, MAP, MAP],
        preview: [
            x + (MAP - PREVIEW) / 2.0,
            map_top + (MAP - PREVIEW) / 2.0,
            PREVIEW,
            PREVIEW,
        ],
        caption: [x, body + 5.0, 400.0, LINE],
        zones: [x, body + 5.0 + LINE + 5.0, 400.0, zones_h],
    }
}

// ------------------------------------------------------------------------------------------- the slider

/// `Slider.ClampValue` for whole numbers: into `[min, max]` (the minimum wins when the range is upside down).
#[must_use]
pub fn clamp_value(value: i32, min: i32, max: i32) -> i32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// The slider's range for a species (`FillPanel`): from the point after its disappearance (`-1`, "Now", while it
/// is alive) to its apparition.
#[must_use]
pub fn slider_range(log: &SpeciesLog) -> (i32, i32) {
    let disappearance = log.total_index_of_disappearance();
    let min = if disappearance < 0 {
        -1
    } else {
        disappearance + 1
    };
    (min, log.total_index_of_apparition())
}

/// `Mathf.InverseLerp(min, max, value)`.
fn normalized(min: i32, max: i32, value: i32) -> f32 {
    if min == max {
        return 0.0;
    }
    ((value - min) as f32 / (max - min) as f32).clamp(0.0, 1.0)
}

/// The handle's middle (reference px): the slider runs right to left, so the minimum (the newest point) is at the
/// right end.
#[must_use]
pub fn handle_x(slider: [f32; 4], min: i32, max: i32, value: i32) -> f32 {
    slider[0] + (1.0 - normalized(min, max, value)) * slider[2]
}

/// `Slider.UpdateDrag` for this slider: the value under the cursor at `cursor_x` (reference px) between the track's
/// ends `x0` and `x1`, with the cursor `grab` px off the handle's middle where it was grabbed.
#[must_use]
pub fn value_at(x0: f32, x1: f32, min: i32, max: i32, cursor_x: f32, grab: f32) -> i32 {
    let width = x1 - x0;
    if width <= 0.0 {
        return clamp_value(min, min, max);
    }
    let along = ((cursor_x - grab - x0) / width).clamp(0.0, 1.0);
    let value = min as f32 + (max - min) as f32 * (1.0 - along);
    // `Mathf.Round` rounds halves to the even number.
    clamp_value(value.round_ties_even() as i32, min, max)
}

/// The slider's text (`LogLikeTimeSpanSlider.UpdateTime`): "Now", or how long ago the point was logged.
#[must_use]
pub fn time_text(value: i32) -> String {
    if value < 0 {
        return "Now".to_owned();
    }
    format!(
        "{} ago",
        TimeFormat::of_species_point(value).formatted(1, "", true, false, Timescale::Minutes)
    )
}

// ---------------------------------------------------------------------------------- the distribution

/// The species' ellipse (`SpeciesDistributionItem.UpdatePoint`) on the preview square.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ellipse {
    /// `anchoredPosition`: from the middle of the square, in canvas units, y up.
    pub centre: [f32; 2],
    /// `sizeDelta`: the width and the height before the rotation.
    pub size: [f32; 2],
    /// `localRotation` in radians, counter-clockwise.
    pub angle: f32,
    /// What `ZoneMatch` reads: the mean position as a fraction of the simulation size.
    pub centre_fraction: [f32; 2],
    /// The ellipse's two axes (`xAxis`, `yAxis`) as fractions of the simulation size.
    pub x_axis: [f32; 2],
    /// See `x_axis`.
    pub y_axis: [f32; 2],
}

/// `Mathf.Clamp`: a NaN stays a NaN.
fn mathf_clamp(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// `SpeciesDistributionItem.UpdatePoint`: the ellipse of the standard deviations and the correlation of a point,
/// on a square `half` canvas units from its middle to its edge, for a simulation `simulation_size` big.
#[must_use]
pub fn ellipse(point: &SpeciesDataPoint, simulation_size: f32, half: f32) -> Ellipse {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};
    let sim = simulation_size;
    let sx = mathf_clamp(point.pos_std_dev[0], sim * 0.03, sim);
    let sy = mathf_clamp(point.pos_std_dev[1], sim * 0.03, sim);
    let mut rho = mathf_clamp(point.pos_cor, -0.999, 0.999);
    if rho == 0.0 || rho.is_nan() {
        rho = 1e-7;
    }
    let mut angle = FRAC_PI_4 - 0.5 * ((sx * sx - sy * sy) / (2.0 * rho * sx * sy)).atan();
    if (rho >= 0.0 && sy >= sx) || (rho < 0.0 && sy < sx) {
        angle -= FRAC_PI_2;
    }
    let mut tall = (sy * sy - rho * sx * sy * angle.tan()).sqrt();
    let wide = sx * sy / tall * (1.0 - rho * rho).sqrt() / sim;
    tall /= sim;
    let (sin, cos) = angle.sin_cos();
    let x_len = wide.max(0.03).max(0.2 * tall);
    let y_len = tall.max(0.03).max(0.2 * wide);
    let pos = point.avg_pos;
    let length = (pos[0] * pos[0] + pos[1] * pos[1]).sqrt();
    // The ellipse stays on the map: no further than 1.5 simulation sizes from the middle, its spread included.
    let reach = (length / sim).min(1.5 - sx.max(sy) / sim);
    let direction = if length > 0.0 {
        [pos[0] / length, pos[1] / length]
    } else {
        [0.0, 0.0]
    };
    Ellipse {
        centre: [half * reach * direction[0], half * reach * direction[1]],
        size: [2.0 * half * wide, 2.0 * half * tall],
        angle,
        centre_fraction: [pos[0] / sim, pos[1] / sim],
        x_axis: [x_len * cos, x_len * sin],
        y_axis: [-y_len * sin, y_len * cos],
    }
}

/// `ZoneMatch`'s `NormalInt`.
fn normal_int(z: f32) -> f32 {
    1.0 / (1.0 + (-1.67 * z).exp())
}

/// `ZoneMatch.Match`.
fn matching(d: f32, r: f32) -> f32 {
    normal_int(r - d) / normal_int(r)
}

/// `new ZoneMatch(zone, ellipse...).overlap`: how much of the zone the species' ellipse covers, from the zone's
/// current position and size.
#[must_use]
pub fn zone_overlap(zone: &ZoneConfig, simulation_size: f32, ellipse: &Ellipse) -> f32 {
    let dx = zone.pos_x - ellipse.centre_fraction[0];
    let dy = zone.pos_y - ellipse.centre_fraction[1];
    let radius = if zone.is_rect() {
        (zone.relative_height(simulation_size) + zone.relative_width(simulation_size)) / 2.0
    } else {
        zone.relative_radius(simulation_size)
    };
    let dot = |axis: [f32; 2]| axis[0] * dx + axis[1] * dy;
    let square = |axis: [f32; 2]| axis[0] * axis[0] + axis[1] * axis[1];
    let x_off = (dot(ellipse.x_axis) / square(ellipse.x_axis)).abs();
    let y_off = (dot(ellipse.y_axis) / square(ellipse.y_axis)).abs();
    let x_ratio = radius / square(ellipse.x_axis).sqrt();
    let y_ratio = radius / square(ellipse.y_axis).sqrt();
    let mut overlap = matching(x_off, x_ratio) * matching(y_off, y_ratio);
    if x_ratio > 1.0 {
        overlap *= (x_off * (1.0 - x_ratio) / (1.0 + x_ratio * x_ratio)).exp();
    }
    if y_ratio > 1.0 {
        overlap *= (y_off * (1.0 - y_ratio) / (1.0 + y_ratio * y_ratio)).exp();
    }
    overlap
}

/// `ZonePreview.UpdatePoint`: the point of a zone at slider value `time` - the zone as it is now for the newest
/// points (`timeIndex < 1`) and when the log does not reach back that far, else the logged point.
#[must_use]
pub fn zone_point(zone: &ZoneData, simulation_size: f32, time: i32) -> ZoneDataPoint {
    if time < 1 || time > zone.log.total_index_of_apparition() {
        zone.log_point(simulation_size)
    } else {
        zone.log.at(time as usize)
    }
}

/// `ZonePreview.UpdatePos` / `UpdateSize`: the shape a zone has on the preview square at a logged point, with the
/// opacity and colour the zone's current settings give it.
#[must_use]
pub fn zone_shape(zone: &ZoneData, point: &ZoneDataPoint, simulation_size: f32) -> PreviewShape {
    let mut shape = crate::scenario_editor::zone_shape(&zone.config, simulation_size);
    let relative = point.relative_pos();
    shape.centre = [relative[0], -relative[1]];
    // The point's radius is half the larger side (or the radius) over two: 65535 is the log's 32767 doubled.
    let num = f32::from(point.radius) / 65535.0;
    shape.size = if zone.config.is_rect() {
        let (w, h) = (
            zone.config.relative_width(simulation_size),
            zone.config.relative_height(simulation_size),
        );
        if w > h {
            [2.0 * num, 2.0 * num * h / w]
        } else {
            [2.0 * num * w / h, 2.0 * num]
        }
    } else {
        [2.0 * num, 2.0 * num]
    };
    shape
}

/// The labels a zone's preview carries (`ZonePreview`: `nameLabel`, `biomassLabel`, `numberPelletsLabel`), stacked over its middle.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneLabel {
    /// The zone's middle on the preview square, as its shape has it (fractions of half the square: `x` right, `y` down).
    pub centre: [f32; 2],
    /// `zoneName`.
    pub name: String,
    /// `maxBiomass` and `estimatedPellets` (shown when the zone spawns a material).
    pub counts: Option<(f32, u32)>,
    /// The labels' alpha: the fertility's factor (`UpdateMaterial`).
    pub alpha: f32,
}

impl ZoneLabel {
    /// The labels of `zone` at `centre` (its shape's) in a world of `sim` size.
    #[must_use]
    pub fn of(
        zone: &bibites_core::zone::ZoneData,
        globals: &bibites_core::zone::ZoneGlobals,
        sim: f32,
        centre: [f32; 2],
    ) -> Self {
        Self {
            centre,
            name: zone.config.name.clone(),
            counts: zone.config.material.is_some().then(|| {
                (
                    zone.config.max_biomass(globals, sim),
                    zone.config.estimated_pellets(globals, sim),
                )
            }),
            alpha: crate::scenario_editor::density_factor(zone.config.fertility),
        }
    }
}

/// The zones' labels (`ZonePreview`'s `holder`: the name (Roboto-Bold 14), the biomass (14, `125.66 kE`) and the pellet count (12, `~26
/// pellets`) stacked over the zone's middle, 16.41, 16.41 and 14.07 high, centred), white at the fertility's alpha. `square` is the preview
/// square (reference px, `x, y, w, h`), `s` the scale, `measure` the width of a text in bold at a size.
pub(crate) fn emit_zone_labels(
    frame: &mut UiFrame,
    labels: &[ZoneLabel],
    square: [f32; 4],
    s: f32,
    measure: &dyn Fn(&str, f32) -> f32,
) {
    let (mid_x, mid_y) = (square[0] + square[2] * 0.5, square[1] + square[3] * 0.5);
    let energy = FloatValueFormat {
        prefix: "",
        units: "E",
        precision: 2,
        ..FloatValueFormat::default()
    };
    let count = FloatValueFormat {
        prefix: "~",
        units: "pellets",
        precision: 2,
        is_int: true,
        ..FloatValueFormat::default()
    };
    for label in labels {
        let centre = [
            mid_x + label.centre[0] * square[2] * 0.5,
            mid_y + label.centre[1] * square[3] * 0.5,
        ];
        let colour: Rgba = [255, 255, 255, (label.alpha.clamp(0.0, 1.0) * 255.0).round() as u8];
        let mut line = |text: &str, px: f32, top: f32| {
            let width = measure(text, px * s);
            frame.bold(
                text.to_owned(),
                Vec2::new(centre[0] * s - width * 0.5, (centre[1] + top) * s),
                px * s,
                colour,
            );
        };
        line(&label.name, 14.0, -23.445);
        if let Some((biomass, pellets)) = label.counts {
            line(energy.format(biomass).trim(), 14.0, -7.035);
            line(count.format(pellets as f32).trim(), 12.0, 9.375);
        }
    }
}

/// What the tab shows (`UpdateSelectedTime`).
#[derive(Clone, Debug)]
pub struct View {
    /// The zones that were present, in the world's order (later ones over earlier ones).
    pub shapes: Vec<PreviewShape>,
    /// Their labels, one per shape.
    pub labels: Vec<ZoneLabel>,
    /// The species was around: three or more bibites with a spread (`point.count > 2 && posStdDev.x > 0`).
    pub around: bool,
    /// Where the species was (when it was around).
    pub ellipse: Option<Ellipse>,
    /// The lines of "Possible Zone Presence": up to five zones the ellipse overlaps by more than 0.15, the best
    /// first, or "None" / "Unknown".
    pub lines: Vec<String>,
}

/// `SpeciesDistributionPanel.UpdateSelectedTime` for slider value `time`.
#[must_use]
pub fn build_view(app: &App, id: SpeciesId, time: i32) -> Option<View> {
    let log = app.world.species.log(id)?;
    let sim = app.world.settings.independents.simulation_size;
    let point = if time < 0 {
        app.world.species_point_now(id)
    } else {
        log.at(time as usize)
    };
    let zones = app.world.zones.zones();
    let mut shapes = Vec::with_capacity(zones.len());
    let mut labels = Vec::with_capacity(zones.len());
    for zone in zones {
        let zone_point = zone_point(zone, sim, time);
        if zone_point.present() {
            let shape = zone_shape(zone, &zone_point, sim);
            labels.push(ZoneLabel::of(zone, &app.world.zones.globals, sim, shape.centre));
            shapes.push(shape);
        }
    }
    let around = point.count > 2 && point.pos_std_dev[0] > 0.0;
    if !around {
        return Some(View {
            shapes,
            labels,
            around,
            ellipse: None,
            lines: vec!["Unknown".to_owned()],
        });
    }
    let ellipse = ellipse(&point, sim, PREVIEW / 2.0);
    let mut matches: Vec<(f32, &str)> = zones
        .iter()
        .map(|zone| {
            (
                zone_overlap(&zone.config, sim, &ellipse),
                zone.config.name.as_str(),
            )
        })
        .filter(|(overlap, _)| *overlap > 0.15)
        .collect();
    // `OrderByDescending` is stable.
    matches.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    // `{2:F0}` rounds halves away from zero, as `f32::round` does.
    let lines: Vec<String> = matches
        .iter()
        .take(5)
        .map(|(overlap, name)| format!("-{name} ({}%)", (overlap * 100.0).round() as i64))
        .collect();
    Some(View {
        shapes,
        labels,
        around,
        ellipse: Some(ellipse),
        lines: if lines.is_empty() {
            vec!["None".to_owned()]
        } else {
            lines
        },
    })
}

// ------------------------------------------------------------------------------------------ app state

/// What the tab was filled with.
#[derive(Clone, Debug)]
pub struct State {
    /// The species the panel shows.
    pub species: SpeciesId,
    /// The slider's range, set when the species was selected (`SetMinMax`).
    pub min: i32,
    /// See `min`.
    pub max: i32,
    /// The picture of the slider's value.
    pub view: View,
    /// Tells a new picture from the last one, for the drawing cache.
    serial: u64,
}

/// The slider is held: its track and where in the handle it was grabbed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drag {
    /// The track's ends (reference px).
    pub x0: f32,
    /// See `x0`.
    pub x1: f32,
    /// How far the cursor was from the handle's middle when it was grabbed.
    pub grab: f32,
}

static SERIAL: AtomicU64 = AtomicU64::new(1);

impl App {
    /// `SpeciesDistributionPanel.SelectSpecies` / `FillPanel`: the slider takes the species' range, keeping its
    /// value (clamped into the range), and the panel is filled for that moment.
    pub fn position_fill(&mut self, id: SpeciesId) {
        let Some(log) = self.world.species.log(id) else {
            self.position = None;
            return;
        };
        let (min, max) = slider_range(log);
        self.position_value = clamp_value(self.position_value, min, max);
        self.position_refresh(id, min, max);
    }

    fn position_refresh(&mut self, id: SpeciesId, min: i32, max: i32) {
        self.position = build_view(self, id, self.position_value).map(|view| State {
            species: id,
            min,
            max,
            view,
            serial: SERIAL.fetch_add(1, Ordering::Relaxed),
        });
    }

    /// The slider moved to `value` (`LogLikeTimeSpanSlider.UpdateTime` -> `UpdateSelectedTime`): the panel is
    /// filled for that moment again.
    pub fn position_set_value(&mut self, value: i32) {
        let Some(state) = self.position.as_ref() else {
            return;
        };
        let (id, min, max) = (state.species, state.min, state.max);
        let value = clamp_value(value, min, max);
        if value == self.position_value {
            return;
        }
        self.position_value = value;
        self.position_refresh(id, min, max);
    }

    /// The slider was pressed: on its handle (the grab offset keeps the value where it is) or on its track (the
    /// value jumps to the cursor, `OnPointerDown`).
    pub fn position_press(&mut self, on_handle: bool, track: (f32, f32), cursor_x: f32) {
        let Some(state) = self.position.as_ref() else {
            return;
        };
        let scale = UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let x = cursor_x / scale;
        let grab = if on_handle {
            let slider = [track.0, 0.0, track.1 - track.0, 0.0];
            x - handle_x(slider, state.min, state.max, self.position_value)
        } else {
            0.0
        };
        self.position_drag = Some(Drag {
            x0: track.0,
            x1: track.1,
            grab,
        });
        self.position_slide(cursor_x);
    }

    /// The held slider follows the cursor (`Slider.UpdateDrag`).
    pub fn position_slide(&mut self, cursor_x: f32) {
        let (Some(drag), Some(state)) = (self.position_drag, self.position.as_ref()) else {
            return;
        };
        let scale = UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let value = value_at(
            drag.x0,
            drag.x1,
            state.min,
            state.max,
            cursor_x / scale,
            drag.grab,
        );
        self.position_set_value(value);
    }
}

// ----------------------------------------------------------------------------------------- the drawing

/// The pictures made from a [`View`] at one screen scale: the zones and the ellipse. The tab is redrawn every
/// frame, and the window uploads a picture once per image, so the same ones are handed out until the view or the
/// scale changes.
struct Pictures {
    serial: u64,
    scale_bits: u32,
    draws: Vec<UiDraw>,
}

static PICTURES: Mutex<Option<Pictures>> = Mutex::new(None);

/// The ellipse as pixels: white at 0.239, with a soft edge, rotated, centred at `centre` (screen px).
fn ellipse_draw(ellipse: &Ellipse, centre: Vec2, scale: f32) -> Option<UiDraw> {
    let values = [
        ellipse.centre[0],
        ellipse.centre[1],
        ellipse.size[0],
        ellipse.size[1],
        ellipse.angle,
    ];
    if values.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let (a, b) = (
        (ellipse.size[0] * scale * 0.5).max(0.5),
        (ellipse.size[1] * scale * 0.5).max(0.5),
    );
    let (sin, cos) = ellipse.angle.sin_cos();
    let half_w = ((a * cos).powi(2) + (b * sin).powi(2)).sqrt();
    let half_h = ((a * sin).powi(2) + (b * cos).powi(2)).sqrt();
    let width = (half_w * 2.0).ceil() as u32 + 2;
    let height = (half_h * 2.0).ceil() as u32 + 2;
    let edge = a.min(b);
    let mut rgba = vec![0_u8; (width * height * 4) as usize];
    for py in 0..height {
        for px in 0..width {
            // From the middle, y up, then turned back into the ellipse's own axes.
            let x = px as f32 + 0.5 - width as f32 * 0.5;
            let y = height as f32 * 0.5 - (py as f32 + 0.5);
            let u = x * cos + y * sin;
            let v = -x * sin + y * cos;
            let distance = ((u / a).powi(2) + (v / b).powi(2)).sqrt();
            let coverage = ((1.0 - distance) * edge).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let at = ((py * width + px) * 4) as usize;
            rgba[at..at + 3].copy_from_slice(&ELLIPSE[..3]);
            rgba[at + 3] = (f32::from(ELLIPSE[3]) * coverage).round() as u8;
        }
    }
    Some(UiDraw::Pixels {
        texture: Arc::new(UiTexture {
            width,
            height,
            rgba,
        }),
        pos: Vec2::new(
            centre.x + ellipse.centre[0] * scale - width as f32 * 0.5,
            centre.y - ellipse.centre[1] * scale - height as f32 * 0.5,
        ),
        size: Vec2::new(width as f32, height as f32),
    })
}

impl Drawer<'_> {
    /// The Position tab.
    pub(super) fn position(&self, frame: &mut UiFrame, app: &App, entry: &SpeciesListEntry) {
        let s = self.s;
        let state = app
            .position
            .as_ref()
            .filter(|state| state.species == entry.id);
        let lines = state.map_or(1, |state| state.view.lines.len());
        let (min, max) = state.map_or((0, 0), |state| (state.min, state.max));
        let value = clamp_value(app.position_value, min, max);
        let time = time_text(value);
        let layout = layout(lines, self.measure(&time, 18.0 * s, true) / s);
        self.slice(frame, "base_icon", 5.0, 1.0, layout.panel, FRAME_GREY);
        frame.hit(
            super::HIT_PANEL,
            self.rect_pos(layout.panel),
            self.rect_size(layout.panel),
            UiAction::Consume,
        );
        frame.bold(
            "Species Distribution over time",
            self.at(layout.title[0], layout.title[1]),
            18.0 * s,
            INK,
        );
        self.time_slider(frame, &layout, (min, max), value, state.is_some());
        self.right_text(frame, layout.time_text, &time, 18.0, INK);
        // The map: the navy square with the preview square in its middle.
        frame.rect(self.rect_pos(layout.map), self.rect_size(layout.map), NAVY);
        frame.rect(
            self.rect_pos(layout.preview),
            self.rect_size(layout.preview),
            NAVY,
        );
        if let Some(state) = state {
            self.map_pictures(frame, app, state, &layout);
            self.zone_labels(frame, &state.view.labels, &layout);
            if !state.view.around {
                self.disclaimer(frame, &layout);
            }
        }
        frame.bold(
            "Possible Zone Presence: ",
            self.at(layout.caption[0], layout.caption[1]),
            13.0 * s,
            BLACK,
        );
        if let Some(state) = state {
            for (index, line) in state.view.lines.iter().enumerate() {
                frame.bold(
                    line.clone(),
                    self.at(layout.zones[0], layout.zones[1] + index as f32 * LINE),
                    13.0 * s,
                    BLACK,
                );
            }
        }
    }

    /// The slider: its dim panel, the track, the fill from the handle to the right end and the handle.
    fn time_slider(
        &self,
        frame: &mut UiFrame,
        layout: &Layout,
        (min, max): (i32, i32),
        value: i32,
        live: bool,
    ) {
        let slider = layout.slider;
        self.slice(
            frame,
            "Background",
            10.0,
            0.5,
            layout.slider_panel,
            [255, 255, 255, 100],
        );
        // The track and the fill's area reach 5 to the left of the slider.
        let track = [slider[0] - 5.0, slider[1], slider[2] + 5.0, slider[3]];
        self.slice(frame, "Background", 10.0, 1.0, track, [255, 255, 255, 65]);
        let along = 1.0 - normalized(min, max, value);
        let left = track[0] + along * track[2] - 5.0;
        let right = track[0] + track[2] + 5.0;
        self.slice(
            frame,
            "UISprite",
            10.0,
            1.0,
            [left, slider[1], right - left, slider[3]],
            WHITE,
        );
        let centre = handle_x(slider, min, max, value);
        let handle = [centre - 10.0, slider[1], 20.0, slider[3]];
        self.icon(frame, "base_circle_icon", handle, WHITE);
        if !live {
            return;
        }
        let ends = (
            slider[0].round().clamp(0.0, 65535.0) as u16,
            (slider[0] + slider[2]).round().clamp(0.0, 65535.0) as u16,
        );
        // The handle sticks out of the track at the ends, so it has its own hit, on top.
        frame.hit(
            HIT_HANDLE,
            self.rect_pos(handle),
            self.rect_size(handle),
            UiAction::PositionSlideHandle(ends.0, ends.1),
        );
        frame.hit(
            HIT_TRACK,
            self.rect_pos(track),
            self.rect_size(track),
            UiAction::PositionSlide(ends.0, ends.1),
        );
    }

    /// The zones and the species' ellipse on the preview square (made once per view and scale).
    fn map_pictures(&self, frame: &mut UiFrame, app: &App, state: &State, layout: &Layout) {
        let s = self.s;
        let cached = PICTURES.lock().ok().and_then(|cache| {
            cache
                .as_ref()
                .filter(|pictures| {
                    pictures.serial == state.serial && pictures.scale_bits == s.to_bits()
                })
                .map(|pictures| pictures.draws.clone())
        });
        if let Some(draws) = cached {
            frame.draws.extend(draws);
            return;
        }
        let before = frame.draws.len();
        let base = app
            .assets
            .as_ref()
            .map_or([231, 154, 0], |assets| assets.background.zone_base);
        let preview = layout.preview;
        crate::ui_runtime::emit_zone_preview(
            frame,
            &state.view.shapes,
            [base[0], base[1], base[2], 255],
            [
                preview[0],
                preview[1],
                preview[0] + preview[2],
                preview[1] + preview[3],
            ],
            s,
        );
        if let Some(ellipse) = state.view.ellipse.as_ref() {
            let centre = Vec2::new(
                (preview[0] + preview[2] * 0.5) * s,
                (preview[1] + preview[3] * 0.5) * s,
            );
            if let Some(draw) = ellipse_draw(ellipse, centre, s) {
                frame.draws.push(draw);
            }
        }
        if let Ok(mut cache) = PICTURES.lock() {
            *cache = Some(Pictures {
                serial: state.serial,
                scale_bits: s.to_bits(),
                draws: frame.draws[before..].to_vec(),
            });
        }
    }

    /// The zones' labels over the map.
    fn zone_labels(&self, frame: &mut UiFrame, labels: &[ZoneLabel], layout: &Layout) {
        emit_zone_labels(frame, labels, layout.preview, self.s, &|text, px| {
            self.measure(text, px, true)
        });
    }

    /// The cover over the map and its text, while the species was not around.
    fn disclaimer(&self, frame: &mut UiFrame, layout: &Layout) {
        let s = self.s;
        frame.rect(self.rect_pos(layout.map), self.rect_size(layout.map), COVER);
        let px = DISCLAIMER_PX * s;
        let room = DISCLAIMER_BOX[0] * s;
        let mut lines: Vec<String> = Vec::new();
        for paragraph in DISCLAIMER.split('\n') {
            let mut line = String::new();
            for word in paragraph.split_whitespace() {
                let candidate = if line.is_empty() {
                    word.to_owned()
                } else {
                    format!("{line} {word}")
                };
                if !line.is_empty() && self.measure(&candidate, px, true) > room {
                    lines.push(std::mem::take(&mut line));
                    line = word.to_owned();
                } else {
                    line = candidate;
                }
            }
            lines.push(line);
        }
        let middle = layout.map[0] + layout.map[2] * 0.5;
        let top = layout.map[1] + (layout.map[3] - DISCLAIMER_BOX[1]) * 0.5;
        for (index, line) in lines.iter().enumerate() {
            let width = self.measure(line, px, true);
            frame.bold(
                line.clone(),
                Vec2::new(
                    middle * s - width * 0.5,
                    (top + index as f32 * DISCLAIMER_PX * 1.1719) * s,
                ),
                px,
                WHITE,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LoadedWorld;

    fn point(count: u16, pos: [f32; 2], spread: [f32; 2], cor: f32) -> SpeciesDataPoint {
        SpeciesDataPoint {
            count,
            energy: 100.0,
            avg_pos: pos,
            pos_std_dev: spread,
            pos_cor: cor,
        }
    }

    /// An app with a world, the Lineage panel open, and a species with a history of twenty logs: ten bibites
    /// spread over 0.2 by 0.1 of the world around its middle (the world's zone logged twenty times too).
    fn app_with_history() -> (App, SpeciesId) {
        let loaded = LoadedWorld::load(None, 41, 10, 1).expect("starter world");
        let mut app = App::new(loaded, Vec2::new(1920.0, 1080.0), None);
        app.lineage_options = Some(crate::lineage_tree::TreeSettings {
            hide_extinct: false,
            ..crate::lineage_tree::TreeSettings::default()
        });
        app.world.run(200);
        let (genome, brain) = {
            let (_, record) = app.world.bibites.iter().next().expect("a bibite");
            (record.genome.clone(), record.brain.clone())
        };
        let id = app
            .world
            .species
            .add_template_species("Zzpos zzspecies", "", &genome, &brain);
        let sim = app.world.settings.independents.simulation_size;
        {
            let log = &mut app.world.species.get_mut(id).expect("species").stats.log;
            for _ in 0..20 {
                log.push(point(10, [0.0, 0.0], [0.2 * sim, 0.1 * sim], 0.0));
            }
        }
        for _ in 0..20 {
            app.world.zones.log_all();
        }
        app.refresh_lineage();
        app.panels.open(crate::ui::PanelKind::Lineage);
        (app, id)
    }

    #[test]
    fn the_panel_is_as_tall_as_its_lines() {
        // The prefab's two lines: 536.81 high; the body's text is the only thing that changes.
        assert!((layout(2, 0.0).panel[3] - 536.81).abs() < 0.02);
        assert!((layout(1, 0.0).panel[3] - (536.81 - LINE)).abs() < 0.05);
        assert!((layout(5, 0.0).panel[3] - (536.81 + 3.0 * LINE)).abs() < 0.05);
        let layout = layout(2, 0.0);
        assert_eq!(layout.panel[..2], [15.0, 50.0]);
        // The pieces sit under each other, 5 apart, 10 in from the panel's sides.
        assert_eq!(layout.title[0], 25.0);
        assert!((layout.slider_panel[1] - (layout.title[1] + 21.1 + 5.0)).abs() < 1e-3);
        assert!((layout.map[1] - (layout.slider_panel[1] + 30.0 + 5.0)).abs() < 1e-3);
        assert_eq!(layout.map[2..], [400.0, 400.0]);
        // The preview square is in the middle of the map.
        assert!((layout.preview[0] - layout.map[0] - 50.0).abs() < 1e-3);
        assert!((layout.preview[1] - layout.map[1] - 50.0).abs() < 1e-3);
        assert_eq!(layout.preview[2..], [300.0, 300.0]);
        assert!((layout.caption[1] - (layout.map[1] + 400.0 + 5.0 + 5.0)).abs() < 1e-3);
        // The time text ends 405 from the row's left, flush right, and is 90 wide at least: the slider's panel is the
        // prefab's 315 and its slider 295.
        assert_eq!(layout.time_text[0] + layout.time_text[2], 25.0 + 405.0);
        assert_eq!(layout.time_text[2], 90.0);
        assert_eq!((layout.slider_panel[2], layout.slider[2]), (315.0, 295.0));
        // A longer text takes its room from the slider.
        let wide = super::layout(2, 110.0);
        assert_eq!(wide.time_text[2], 110.0);
        assert_eq!((wide.slider_panel[2], wide.slider[2]), (295.0, 275.0));
        assert_eq!(
            wide.time_text[0],
            wide.slider_panel[0] + wide.slider_panel[2]
        );
        // A shorter one does not give it any.
        assert_eq!(super::layout(2, 40.0).slider_panel[2], 315.0);
    }

    #[test]
    fn the_slider_runs_right_to_left_over_whole_numbers() {
        let slider = [35.0, 0.0, 295.0, 20.0];
        // The newest point (the minimum) is at the right end, the oldest at the left.
        assert_eq!(handle_x(slider, -1, 30, -1), 330.0);
        assert_eq!(handle_x(slider, -1, 30, 30), 35.0);
        assert!((handle_x(slider, 0, 10, 5) - 182.5).abs() < 1e-3);
        // A range of one point has the handle at the right.
        assert_eq!(handle_x(slider, 3, 3, 3), 330.0);
        // The cursor picks the value, clamped to the ends.
        assert_eq!(value_at(35.0, 330.0, -1, 30, 400.0, 0.0), -1);
        assert_eq!(value_at(35.0, 330.0, -1, 30, 0.0, 0.0), 30);
        assert_eq!(value_at(35.0, 330.0, 0, 10, 182.5, 0.0), 5);
        // Grabbed 4 px right of the handle's middle: the handle stays where it was.
        let at = handle_x(slider, 0, 10, 7);
        assert_eq!(value_at(35.0, 330.0, 0, 10, at + 4.0, 4.0), 7);
        // Halves go to the even number (`Mathf.Round`): 0.5 of a step is 0, 1.5 is 2 (8 px a step, exact in floats).
        assert_eq!(value_at(0.0, 64.0, 0, 8, 60.0, 0.0), 0);
        assert_eq!(value_at(0.0, 64.0, 0, 8, 52.0, 0.0), 2);
        // The range of a species: alive, from "Now"; gone, from after its disappearance.
        assert_eq!(clamp_value(5, -1, 30), 5);
        assert_eq!(clamp_value(-1, 4, 30), 4);
        assert_eq!(clamp_value(99, -1, 30), 30);
    }

    #[test]
    fn the_time_text_says_how_long_ago() {
        assert_eq!(time_text(-1), "Now");
        assert_eq!(time_text(0), "5m ago");
        // Whole hours still say their minutes.
        assert_eq!(time_text(11), "1h0m ago");
        // The first point of the second scale: 70 minutes.
        assert_eq!(time_text(12), "1h10m ago");
    }

    #[test]
    fn an_uncorrelated_spread_makes_an_axis_aligned_ellipse() {
        let sim = 1000.0;
        let wide = ellipse(&point(10, [0.0, 0.0], [200.0, 100.0], 0.0), sim, 150.0);
        assert!(wide.angle.abs() < 1e-5, "{}", wide.angle);
        assert!(
            (wide.size[0] - 2.0 * 150.0 * 0.2).abs() < 1e-3,
            "{:?}",
            wide.size
        );
        assert!(
            (wide.size[1] - 2.0 * 150.0 * 0.1).abs() < 1e-3,
            "{:?}",
            wide.size
        );
        let tall = ellipse(&point(10, [0.0, 0.0], [100.0, 200.0], 0.0), sim, 150.0);
        assert!(tall.angle.abs() < 1e-5, "{}", tall.angle);
        assert!((tall.size[0] - 2.0 * 150.0 * 0.1).abs() < 1e-3);
        assert!((tall.size[1] - 2.0 * 150.0 * 0.2).abs() < 1e-3);
        // Equal spreads are a circle.
        let round = ellipse(&point(10, [0.0, 0.0], [100.0, 100.0], 0.0), sim, 150.0);
        assert!(
            (round.size[0] - round.size[1]).abs() < 1e-2,
            "{:?}",
            round.size
        );
        assert!((round.size[0] - 30.0).abs() < 1e-2);
    }

    #[test]
    fn a_positive_correlation_stretches_the_ellipse_up_and_to_the_right() {
        let sim = 1000.0;
        let up = ellipse(&point(10, [0.0, 0.0], [100.0, 100.0], 0.9), sim, 150.0);
        // The rotated `up` axis points to the upper right and is the long one.
        assert!(up.y_axis[0] > 0.0 && up.y_axis[1] > 0.0, "{:?}", up.y_axis);
        assert!(up.size[1] > up.size[0], "{:?}", up.size);
        let down = ellipse(&point(10, [0.0, 0.0], [100.0, 100.0], -0.9), sim, 150.0);
        assert!(
            down.y_axis[0] < 0.0 && down.y_axis[1] > 0.0,
            "{:?}",
            down.y_axis
        );
        // The ellipse of a species at the edge of the world is kept on the map: 1.5 sizes from the middle at most.
        let far = ellipse(&point(10, [5000.0, 0.0], [100.0, 100.0], 0.0), sim, 150.0);
        assert!(
            (far.centre[0] - 150.0 * (1.5 - 0.1)).abs() < 1e-2,
            "{:?}",
            far.centre
        );
        assert_eq!(far.centre[1], 0.0);
        // The spread is limited to 3% .. 100% of the world.
        let tiny = ellipse(&point(10, [0.0, 0.0], [1.0, 1.0], 0.0), sim, 150.0);
        assert!(
            (tiny.size[0] - 2.0 * 150.0 * 0.03).abs() < 1e-2,
            "{:?}",
            tiny.size
        );
    }

    /// A zone of `radius` (relative) at `x` (relative) on the middle line.
    fn zone_at(x: f32, radius: f32) -> ZoneConfig {
        let mut zone = ZoneConfig::default_zone();
        zone.radius_is_relative = true;
        zone.radius_relative = radius;
        zone.pos_x = x;
        zone.pos_y = 0.0;
        zone
    }

    #[test]
    fn a_zone_under_the_ellipse_overlaps_it_and_a_far_one_does_not() {
        let sim = 1000.0;
        // Spreads 200 by 100, uncorrelated: the axes are 0.2 and 0.1 of the world along x and y.
        let ellipse = ellipse(&point(10, [0.0, 0.0], [200.0, 100.0], 0.0), sim, 150.0);
        assert!((zone_overlap(&zone_at(0.0, 0.2), sim, &ellipse) - 1.0).abs() < 1e-3);
        // A zone as big as the axis, a whole axis away: matched by `NormalInt(0) / NormalInt(1)` = 0.594 along x
        // and fully along y.
        let half = zone_overlap(&zone_at(0.2, 0.2), sim, &ellipse);
        assert!(
            (half - 0.5 / (1.0 / (1.0 + (-1.67_f32).exp()))).abs() < 1e-3,
            "{half}"
        );
        assert!((half - 0.594).abs() < 0.001, "{half}");
        // Far away there is nothing.
        assert!(zone_overlap(&zone_at(0.9, 0.2), sim, &ellipse) < 0.15);
        // A zone much bigger than the ellipse that has it inside counts in full.
        let big = zone_overlap(&zone_at(0.0, 0.5), sim, &ellipse);
        assert!((big - 1.0).abs() < 1e-3, "{big}");
        // A rectangle counts by the mean of its sides.
        let mut rect = zone_at(0.0, 0.2);
        rect.distribution = bibites_core::settings::SpawnDistribution::Rect;
        rect.radius_is_relative = true;
        rect.width_relative = 0.3;
        rect.height_relative = 0.1;
        assert!((zone_overlap(&rect, sim, &ellipse) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn the_tab_is_filled_for_a_moment_of_the_species_history() {
        let (mut app, id) = app_with_history();
        app.position_value = 0;
        app.position_fill(id);
        let state = app.position.as_ref().expect("filled");
        // Alive and logged 20 times: from "Now" back to the oldest point (the first log is 12 + 8/2 = 16 points old).
        assert_eq!(state.min, -1);
        assert_eq!(
            state.max,
            app.world
                .species
                .log(id)
                .unwrap()
                .total_index_of_apparition()
        );
        assert_eq!(app.position_value, 0, "the value is kept");
        let view = &state.view;
        assert!(view.around, "ten bibites with a spread");
        let ellipse = view.ellipse.expect("the species was around");
        assert!(ellipse.size[0] > 0.0 && ellipse.size[1] > 0.0);
        // The world's zone was logged (present), so it is on the map, as big as the world.
        assert_eq!(view.shapes.len(), app.world.zones.zones().len());
        assert!(!view.lines.is_empty());
        // The slider moves to a point and the panel is filled for it again.
        let serial = state.serial;
        app.position_set_value(5);
        assert_eq!(app.position_value, 5);
        assert_ne!(app.position.as_ref().unwrap().serial, serial);
        // A value outside the range is clamped, and the same value is not a new picture.
        app.position_set_value(10_000);
        let max = app.position.as_ref().unwrap().max;
        assert_eq!(app.position_value, max);
        let serial = app.position.as_ref().unwrap().serial;
        app.position_set_value(10_000);
        assert_eq!(app.position.as_ref().unwrap().serial, serial);
    }

    #[test]
    fn a_species_that_was_too_scarce_shows_unknown() {
        let (mut app, id) = app_with_history();
        {
            let log = &mut app.world.species.get_mut(id).unwrap().stats.log;
            // Two bibites: not enough to say where the species was.
            log.push(point(2, [0.0, 0.0], [50.0, 50.0], 0.0));
        }
        app.position_value = 0;
        app.position_fill(id);
        let view = &app.position.as_ref().unwrap().view;
        assert!(!view.around);
        assert!(view.ellipse.is_none());
        assert_eq!(view.lines, vec!["Unknown".to_owned()]);
        // The zones show under the disclaimer all the same.
        assert!(!view.shapes.is_empty());
    }

    #[test]
    fn the_zones_are_as_they_were_logged_and_as_they_are_for_the_newest_points() {
        let (mut app, id) = app_with_history();
        let sim = app.world.settings.independents.simulation_size;
        // The zone moved after it was logged: the older points still have it where it was.
        let logged = app.world.zones.zones()[0].log.at(4).pos_x;
        assert_ne!(logged, (0.5_f32 * 32767.0) as i16);
        app.world.zones.zones_mut()[0].config.pos_x = 0.5;
        app.world.zones.log_all();
        let zone = &app.world.zones.zones()[0];
        let old = zone_point(zone, sim, 4);
        assert_eq!(old.pos_x, logged, "logged where it was");
        let newest = zone_point(zone, sim, 0);
        assert_eq!(
            newest.pos_x,
            (0.5_f32 * 32767.0) as i16,
            "index 0 is the zone as it is now"
        );
        let now = zone_point(zone, sim, -1);
        assert_eq!(now, newest);
        // Past the end of the log: the zone as it is now.
        assert_eq!(zone_point(zone, sim, 10_000), newest);
        // The shape follows the point and the zone's own settings.
        let shape = zone_shape(zone, &old, sim);
        assert_eq!(shape.centre[0], f32::from(logged) / 32767.0);
        app.position_value = 4;
        app.position_fill(id);
        let view = &app.position.as_ref().unwrap().view;
        assert_eq!(view.shapes[0].centre[0], f32::from(logged) / 32767.0);
    }

    #[test]
    fn the_overlap_text_lists_the_best_five_zones() {
        let (mut app, id) = app_with_history();
        // The world's own zone is somewhere else; six small zones lie east of the species' middle, 0.1 of the world
        // apart, so each overlaps its ellipse (axes 0.2 and 0.1) less than the one before.
        {
            let zone = &mut app.world.zones.zones_mut()[0];
            zone.config.name = "Home".to_owned();
            zone.config.pos_x = 0.9;
            zone.config.pos_y = 0.0;
            zone.config.radius_relative = 0.05;
        }
        for k in 0..6 {
            let mut config = app.world.zones.zones()[0].config.clone();
            config.name = format!("Zone {k}");
            config.pos_x = 0.1 * k as f32;
            app.world.zones.add_zone(config);
        }
        app.position_value = 0;
        app.position_fill(id);
        let view = &app.position.as_ref().unwrap().view;
        // `NormalInt(0.25 - d) / NormalInt(0.25)` along x for d = 0, 0.5, 1, 1.5, 2, 2.5 axes: 100, 66, 37, 18, 8
        // and 4 percent; the last two are not above 15.
        assert_eq!(
            view.lines,
            vec![
                "-Zone 0 (100%)".to_owned(),
                "-Zone 1 (66%)".to_owned(),
                "-Zone 2 (37%)".to_owned(),
                "-Zone 3 (18%)".to_owned(),
            ]
        );
        // The panel is as tall as the four lines ask.
        assert!((layout(view.lines.len(), 0.0).panel[3] - (536.81 + 2.0 * LINE)).abs() < 0.05);
        // With nothing around, the text says so.
        for zone in app.world.zones.zones_mut() {
            zone.config.pos_x = -0.9;
        }
        app.position_set_value(1);
        let view = &app.position.as_ref().unwrap().view;
        assert_eq!(view.lines, vec!["None".to_owned()]);
    }

    #[test]
    fn the_tab_draws_its_slider_texts_and_hits() {
        let (mut app, id) = app_with_history();
        app.lineage_tab = 3;
        app.select_species_info(Some(id));
        app.position_value = 0;
        app.position_fill(id);
        let mut frame = UiFrame::new(1.0);
        crate::species_panel::emit(
            &mut frame,
            &app,
            &crate::ui::UiAssets::empty(),
            None,
            1920.0,
            1080.0,
        );
        let texts: Vec<String> = frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Text { text, .. } => Some(text.trim().to_owned()),
                _ => None,
            })
            .collect();
        for expected in [
            "Species Distribution over time",
            "5m ago",
            "Possible Zone Presence:",
        ] {
            assert!(
                texts.iter().any(|text| text == expected),
                "{expected}: {texts:?}"
            );
        }
        // The slider has a handle and a track to press.
        let actions: Vec<UiAction> = frame.actions.iter().map(|(_, action)| *action).collect();
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, UiAction::PositionSlideHandle(..)))
        );
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, UiAction::PositionSlide(..)))
        );
        // The ellipse is a picture on the map.
        assert!(
            frame
                .draws
                .iter()
                .any(|draw| matches!(draw, UiDraw::Pixels { .. }))
        );
        // Drawn again at the same scale, the pictures are the same ones (the window uploads them once).
        let mut again = UiFrame::new(1.0);
        crate::species_panel::emit(
            &mut again,
            &app,
            &crate::ui::UiAssets::empty(),
            None,
            1920.0,
            1080.0,
        );
        let address = |frame: &UiFrame| -> Vec<usize> {
            frame
                .draws
                .iter()
                .filter_map(|draw| match draw {
                    UiDraw::Pixels { texture, .. } => Some(Arc::as_ptr(texture) as usize),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(address(&frame), address(&again));
    }

    #[test]
    fn pressing_and_dragging_the_slider_moves_the_value() {
        let (mut app, id) = app_with_history();
        app.position_value = 3;
        app.position_fill(id);
        let (min, max) = {
            let state = app.position.as_ref().unwrap();
            (state.min, state.max)
        };
        let layout = layout(1, 0.0);
        let scale = UiFrame::scaler(app.camera.screen.x, app.camera.screen.y);
        let track = (layout.slider[0], layout.slider[0] + layout.slider[2]);
        // A press on the track jumps to the cursor: the left end is the oldest point.
        app.position_press(false, track, (track.0 - 4.0) * scale);
        assert_eq!(app.position_value, max);
        // Dragging to the right end goes to "Now".
        app.position_slide((track.1 + 20.0) * scale);
        assert_eq!(app.position_value, min);
        // A press on the handle does not move it.
        let at = handle_x(layout.slider, min, max, app.position_value);
        app.position_press(true, track, (at + 3.0) * scale);
        assert_eq!(app.position_value, min);
        assert_eq!(app.position_drag.map(|drag| drag.grab), Some(3.0));
    }
}

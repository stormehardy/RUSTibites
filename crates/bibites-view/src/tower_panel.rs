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
}

impl Kind {
    /// The panel of `tool`.
    #[must_use]
    pub fn of(tool: &Tool) -> Self {
        match tool {
            Tool::Phero(_) => Self::Phero,
            Tool::Rad(_) => Self::Rad,
            Tool::Killer(_) => Self::Killer,
        }
    }

    /// The title text of the prefab (the game spells the colour killer's "Editior").
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Phero => "Pheromone Tower",
            Self::Rad => "RadTower",
            Self::Killer => "Color Selector Editior",
        }
    }

    /// The slider fields, top to bottom.
    #[must_use]
    pub fn sliders(self) -> &'static [u8] {
        match self {
            Self::Phero => &[RED, GREEN, BLUE, HEADING],
            Self::Rad => &[COUNT, INTENSITY, RADIUS, TAPER],
            Self::Killer => &[HUE, TOLERANCE, SCAN_RADIUS, PERIOD],
        }
    }

    /// Panel size (canvas units): the prefab's `VerticalLayoutGroup` result.
    #[must_use]
    fn size(self) -> Vec2 {
        match self {
            Self::Phero => Vec2::new(584.48, 275.47),
            Self::Rad => Vec2::new(555.0, 275.47),
            Self::Killer => Vec2::new(514.05, 428.5),
        }
    }

    /// Width of the Remove button (it takes the title line's spare width in the radiation panel).
    #[must_use]
    fn remove_width(self) -> f32 {
        match self {
            Self::Phero => 140.0,
            Self::Rad => 217.73,
            Self::Killer => 99.73,
        }
    }

    /// The slider and value-text boxes of a row, from the row's left edge.
    #[must_use]
    fn row_boxes(self) -> ((f32, f32), (f32, f32)) {
        match self {
            Self::Phero => ((135.0, 464.48), (479.48, 554.48)),
            Self::Rad => ((135.0, 435.0), (450.0, 525.0)),
            Self::Killer => ((135.0, 414.05), (429.05, 484.05)),
        }
    }

    /// Top of a slider's row, from the panel's top.
    #[must_use]
    fn row_top(self, field: u8) -> f32 {
        let main = if self == Self::Killer { 65.0 } else { 65.47 };
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
        _ => return None,
    })
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
        _ => {}
    }
}

/// The text of a slider's value box.
#[must_use]
pub fn value_text(tool: &Tool, field: u8, sim_size: f32) -> String {
    slider_def(field, sim_size)
        .map(|def| def.format.format(value_of(tool, field)))
        .unwrap_or_default()
}

/// Slider position `0..=1` of a field's current value.
#[must_use]
pub fn fraction_of_tool(tool: &Tool, field: u8, sim_size: f32) -> f32 {
    slider_def(field, sim_size).map_or(0.0, |def| {
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
    let Some(def) = slider_def(field, sim_size) else {
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

/// Flips a colour killer switch (`SetSelectionTarget` / `SetKillOption`).
pub fn flip_switch(world: &mut World, id: ToolId, field: u8) {
    if let Some(Tool::Killer(killer)) = world.tools.get_mut(id) {
        match field {
            BODY_OR_EYES => killer.body_or_eye = !killer.body_or_eye,
            ANY_OR_MOST => killer.any_or_most = !killer.any_or_most,
            _ => {}
        }
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
/// `ColorKillerPanel.BuildAttachedSpawnerDropdown` for the colour killer: "None" and the five
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
    if let Tool::Killer(killer) = tool {
        let mut by_distance: Vec<(ZoneId, String, f32)> = all
            .map(|(id, name, pos)| (id, name, pos.distance(killer.base.pos)))
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
const HIT_SWITCH: u32 = 8603;
const HIT_SLIDER: u32 = 8610;

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
    let zone_top = PANEL_Y + kind.row_top(ZONE);
    let zone_label = if kind == Kind::Killer {
        "Follow Zone:"
    } else {
        name_of(ZONE)
    };
    frame.text(zone_label, at(left, zone_top + 2.0), 20.0 * s, DARK);
    let box_at = at(right - 150.0, zone_top);
    slice(
        frame,
        ui,
        "UISprite",
        4.0,
        1.0,
        box_at,
        dim(150.0, ROW_H),
        [245, 245, 245, 255],
    );
    frame.rect(box_at, dim(150.0, 1.0), [90, 90, 90, 255]);
    frame.rect(
        at(right - 150.0, zone_top + ROW_H - 1.0),
        dim(150.0, 1.0),
        [90, 90, 90, 255],
    );
    frame.text(
        model
            .zone
            .labels
            .get(model.zone.current)
            .map_or("None", String::as_str),
        at(right - 150.0 + 9.0, zone_top + 3.0),
        18.0 * s,
        DARK,
    );
    icon(
        frame,
        ui,
        "DropdownArrow",
        at(right - 25.0, zone_top + 2.5),
        dim(20.0, 20.0),
        [255, 255, 255, 255],
    );
    frame.hit(
        HIT_ZONE,
        box_at,
        dim(150.0, ROW_H),
        UiAction::DropdownOpen(crate::ui::DropdownTarget::ToolZone),
    );

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
            tool,
        };
        let mut frame = UiFrame::default();
        draw_model(&mut frame, &model, &UiAssets::empty(), 1920.0, 1080.0);
        frame
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

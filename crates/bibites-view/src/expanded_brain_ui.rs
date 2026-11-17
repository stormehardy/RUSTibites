//! The Expanded Brain View's screen and its pointer (`ExpandedBrainPanel` and its Top Bar and `BrainViewSettingsPanel`, drawn from the
//! prefab): the selected bibite's brain on a zoomable canvas, its force simulation, the highlight of what a neuron reaches, and the
//! settings of both graph types.
//!
//! Prefab values (level2 `ExpandedBrainPanel`, sharedassets2 `ExUINeuron` / `UISynaps`): the panel fills the screen with a 0.22 grey
//! `ExpandedBrainScroll` (a clamped `ScrollRect` whose content, the `BrainHolder`, is panned with the right button and zoomed with the
//! wheel); the 80-high Top Bar has the settings button at the left and, from the right, Exit, Help, Reset and Start / Stop, each 60 x 60 on a
//! `base_icon` (ppum 0.75), 10 apart; the settings panel is 600 x 800 at (10, 10) (`Background` 0.736 grey, a 25-point title, a close
//! `X`, a `Long_UI` view with a scroll view of 535 and a column padded 20 / 20 / 20 / 35 with 20 between its children: the graph-type
//! button, "Toggle Disabled Synapse Visibility" and the parameter group of the graph type). A neuron is the prefab's 35 x 35 at scale 1.7
//! (a `UINeuron` at 1.286 in the inspector), its synapses strips of `synapse_0` as in the Brain tabs, thickness 10 to 16.
//!
//! What the game does and this does too: the hover text over a neuron is the prefab's "0.00" (`ExUINeuron.OnPointerEnter` never sets
//! `isHovered`, so its text is never updated); the names above the neurons are never shown; a click on a neuron highlights what it reaches
//! (`BFS`), a click on anything else of the canvas (a synapse too) takes the highlight away; a hovered synapse is drawn on top of the others
//! with its weight above its middle; a dragged neuron goes to the cursor and stays still for the simulation. The two buttons of the
//! settings that the prefab leaves inactive ("Reset Branch Selection", "Toggle Trim Branches") are not there.

use std::sync::{Arc, Mutex};

use bibites_brainsort::N_INPUTS;
use bibites_core::math::Vec2;

use crate::app::{App, FrameInput};
use crate::dyn_panel::{FieldRef, LINEAR, fraction_range, value_range};
use crate::expanded_brain::{
    ExpandedBrain, GraphType, LINEAR_PARAMS, Param, SPHERICAL_PARAMS, Viewport, default_params,
};
use crate::lineage_settings::SliderSpec;
use crate::species_panel::Drawer;
use crate::species_panel::brain::{
    Mark, Strip, icon_name, neuron_colour, node_tip, pattern, rasterise, synapse_colour,
};
use crate::tooltip::{self, Tip, TipRect};
use crate::ui::{FontRole, Rgba, UiAction, UiDraw, UiFrame, UiId, UiTexture};
use crate::value_edit::Rules;

/// What a control of the Expanded Brain View does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExAction {
    /// The scroll view behind everything: a left press that is let go on it is a click (`resetBFS`), the right button pans it.
    Canvas,
    /// A neuron (an index of the model's neurons): a click highlights what it reaches, a drag moves it.
    Neuron(u16),
    /// `ToggleBrainSettings` (the gear).
    Settings,
    /// The settings panel's close `X` (`UIPanel.ClosePanel`).
    CloseSettings,
    /// `ExitButton`: `OpenBibitePanel(BrainPanel)`.
    Exit,
    /// `HelpButton`: `GameManager.brainHelp`.
    Help,
    /// `ResetButton`: `ExpandedBrainPanel.reset`.
    Reset,
    /// `StartButton`: `StartForce`.
    Start,
    /// `StopButton`: `HaltForce`.
    Stop,
    /// The graph-type button (`ToggleGraphType`).
    ToggleGraph,
    /// `ToggleDisabledButton` (`DisableSynapse`).
    ToggleDisabled,
    /// A parameter's slider: the setting's index and the track's ends in reference px.
    Slide(u8, u16, u16),
    /// A parameter's reset button.
    ResetParam(u8),
    /// A part of the panel that takes the press.
    Consume,
}

/// What a held press of the pointer is doing in the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExDrag {
    /// A neuron is pressed: it follows the cursor once the press has moved 10 px (`ExUINeuron.OnDrag`), else it is clicked.
    Neuron {
        /// The neuron (an index of the model's neurons).
        index: usize,
        /// Where the press began (window px).
        start: [f32; 2],
        /// The press has moved far enough to be a drag.
        moved: bool,
    },
    /// The left button is down on the canvas: let go on it, it is a click.
    Canvas,
    /// The right button pans the canvas (`ExpandedBrainScroll.OnDrag`, which begins after 10 px).
    Pan {
        /// Where the press began (window px).
        start: [f32; 2],
        /// Once the drag has begun: where the cursor was then (window px) and where the canvas was (canvas units).
        from: Option<[f32; 4]>,
    },
    /// A parameter's slider is held (the setting, the track's ends).
    Slider(u8, u16, u16),
}

/// The view's own state (what is edited is the [`ExpandedBrain`]).
#[derive(Clone, Debug, Default)]
pub struct ExpandedView {
    /// The settings panel shows.
    pub settings_open: bool,
    /// The press that is being held.
    pub drag: Option<ExDrag>,
    /// The right button was down last frame.
    pub secondary_down: bool,
}

/// The open panel.
#[derive(Clone, Debug)]
pub struct ExpandedSession {
    /// The neurons, the synapses and the simulation.
    pub model: ExpandedBrain,
    /// The view.
    pub view: ExpandedView,
}

// ---------------------------------------------------------------------------------------- the geometry

const SCENE: &str = "UICanvas/MainUI/Layer2/ExpandedBrainPanel/";
/// The scroll view's colour (0.22).
const BACKGROUND: Rgba = [56, 56, 56, 255];
const WHITE: Rgba = [255, 255, 255, 255];
/// A top bar button.
const BUTTON: f32 = 60.0;
/// The top bar buttons' top edge (the bar is 80 high).
const BUTTON_Y: f32 = 10.0;
/// The settings panel (`BrainViewSettingsPanel`).
pub const SETTINGS: [f32; 4] = [10.0, 10.0, 600.0, 800.0];
/// The settings panel's white view (`SettingsView`, inset 10 at the sides and 50 / 10 above and below), and the scroll view in it.
const SETTINGS_VIEW: [f32; 4] = [20.0, 60.0, 580.0, 740.0];
/// Its column: the scroll view's viewport is 45 narrower than the view; the column is padded by 20.
const COLUMN_X: f32 = 40.0;
const COLUMN_W: f32 = 495.0;
const COLUMN_TOP: f32 = 90.4;
/// A neuron: the prefab's 35 x 35 at scale 1.7, its icon inset by 8 and its sign 11 x 11.
const NODE: f32 = 35.0 * 1.7;
const ICON: f32 = 19.0 * 1.7;
const SIGN: f32 = 11.0 * 1.7;
/// Hit ids of the view.
const HIT_PANEL: UiId = 49_000;
const HIT_CANVAS: UiId = 49_001;
const HIT_BUTTONS: UiId = 49_010;
const HIT_SETTINGS: UiId = 49_100;
const HIT_NEURON: UiId = 920_000;

/// The top bar's buttons in reference px for a window `width` wide (in reference px): the gear at the left, then from the right Exit,
/// Help, Reset and Start / Stop (`HorizontalLayoutGroup` with padding 10, spacing 10, reversed at the right).
#[must_use]
pub fn top_buttons(width: f32) -> [[f32; 4]; 5] {
    let right = |slot: f32| [width - 10.0 - BUTTON - slot * (BUTTON + 10.0), BUTTON_Y, BUTTON, BUTTON];
    [
        [10.0, BUTTON_Y, BUTTON, BUTTON],
        right(0.0),
        right(1.0),
        right(2.0),
        right(3.0),
    ]
}

/// The parameters of the graph type, in the order of its group.
#[must_use]
pub fn group_of(graph: GraphType) -> &'static [Param] {
    match graph {
        GraphType::Linear => &LINEAR_PARAMS,
        GraphType::Circular => &SPHERICAL_PARAMS,
    }
}

/// The settings column laid out (`x, y, w, h` in reference px): the graph-type button, the toggle for disabled synapses, the group's
/// title and its rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    /// The graph-type button (60 high).
    pub graph_button: [f32; 4],
    /// "Toggle Disabled Synapse Visibility".
    pub disabled_button: [f32; 4],
    /// The group's title (50 high).
    pub title: [f32; 4],
    /// The rows (40 high, 20 apart).
    pub rows: Vec<[f32; 4]>,
}

/// The column for a group of `rows` parameters.
#[must_use]
pub fn column(rows: usize) -> Column {
    let mut y = COLUMN_TOP;
    let graph_button = [COLUMN_X, y, COLUMN_W, 60.0];
    y += 60.0 + 20.0;
    let disabled_button = [COLUMN_X, y, COLUMN_W, 60.0];
    y += 60.0 + 20.0;
    let title = [COLUMN_X, y, COLUMN_W, 50.0];
    // The holder: 10 under the title and its own 10 of padding, then a row and 20 for each.
    y += 50.0 + 10.0 + 10.0;
    let rows = (0..rows)
        .map(|index| [COLUMN_X, y + index as f32 * 60.0, COLUMN_W, 40.0])
        .collect();
    Column {
        graph_button,
        disabled_button,
        title,
        rows,
    }
}

// ---------------------------------------------------------------------------------------- the picture

/// The synapses' picture, kept while nothing about them changes.
struct Cached {
    key: u64,
    draw: Option<UiDraw>,
}

static SYNAPSES: Mutex<Option<Cached>> = Mutex::new(None);

/// A hash of everything the picture of the synapses depends on.
fn picture_key(strips: &[Strip], window: [f32; 2], scale: f32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for strip in strips {
        for value in [
            strip.from[0],
            strip.from[1],
            strip.to[0],
            strip.to[1],
            strip.thickness,
        ] {
            value.to_bits().hash(&mut hasher);
        }
        strip.tint.hash(&mut hasher);
    }
    for value in window.into_iter().chain([scale]) {
        value.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

/// Where a neuron is on the screen in reference px (the middle of its circle).
fn place(model: &ExpandedBrain, local: [f32; 2], view: Viewport) -> [f32; 2] {
    let at = model.to_screen(local, view);
    [at[0] / view.ui, at[1] / view.ui]
}

/// How far `point` is from the segment `a`-`b`.
fn distance_to_segment(point: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length_squared = dx * dx + dy * dy;
    let t = if length_squared > 0.0 {
        (((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / length_squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (cx, cy) = (a[0] + t * dx, a[1] + t * dy);
    ((point[0] - cx).powi(2) + (point[1] - cy).powi(2)).sqrt()
}

/// The synapse under `cursor` (reference px): the one drawn on top of the others, which is the one that was made first (`UISynaps`
/// are put first among their siblings as they are made).
#[must_use]
pub fn synapse_at(model: &ExpandedBrain, view: Viewport, cursor: [f32; 2]) -> Option<usize> {
    model.synapses.iter().position(|synapse| {
        if synapse.hidden {
            return false;
        }
        let from = place(model, model.neurons[synapse.node_in].pos, view);
        let to = place(model, model.neurons[synapse.node_out].pos, view);
        // The strip's rectangle is as thick as the synapse; a hair-thin one is still catchable.
        distance_to_segment(cursor, from, to) <= (synapse.thickness * model.scale * 0.5).max(2.0)
    })
}

/// The neuron under `cursor` (reference px): the last of them is the topmost.
#[must_use]
pub fn neuron_at(model: &ExpandedBrain, view: Viewport, cursor: [f32; 2]) -> Option<usize> {
    let half = NODE * model.scale / 2.0;
    model.neurons.iter().rposition(|neuron| {
        let at = place(model, neuron.pos, view);
        (cursor[0] - at[0]).abs() <= half && (cursor[1] - at[1]).abs() <= half
    })
}

/// The synapses as strips in reference px, bottom first (the last made is at the bottom; the hovered one goes on top).
fn strips_of(model: &ExpandedBrain, view: Viewport, hovered: Option<usize>) -> Vec<Strip> {
    let strip = |index: usize| -> Option<Strip> {
        let synapse = &model.synapses[index];
        if synapse.hidden {
            return None;
        }
        let from = place(model, model.neurons[synapse.node_in].pos, view);
        let to = place(model, model.neurons[synapse.node_out].pos, view);
        Some(Strip {
            // The picture's coordinates are `x` right, `y` up.
            from: [from[0], -from[1]],
            to: [to[0], -to[1]],
            thickness: synapse.thickness * model.scale,
            tint: synapse_colour(synapse.weight, synapse.alpha(), synapse.enabled),
            halo: 0.0,
        })
    };
    let mut strips: Vec<Strip> = (0..model.synapses.len())
        .rev()
        .filter(|&index| Some(index) != hovered)
        .filter_map(strip)
        .collect();
    if let Some(hovered) = hovered.and_then(strip) {
        strips.push(hovered);
    }
    strips
}

impl Drawer<'_> {
    /// The picture of the synapses: one raster of the part of the window they cover.
    fn synapse_picture(&self, frame: &mut UiFrame, strips: &[Strip], window: [f32; 2]) {
        let s = self.s;
        let key = picture_key(strips, window, s);
        let cached = SYNAPSES.lock().ok().and_then(|cache| {
            cache
                .as_ref()
                .filter(|cached| cached.key == key)
                .map(|cached| cached.draw.clone())
        });
        let draw = if let Some(draw) = cached {
            draw
        } else {
            let draw = self.rasterise_synapses(strips, window);
            if let Ok(mut cache) = SYNAPSES.lock() {
                *cache = Some(Cached {
                    key,
                    draw: draw.clone(),
                });
            }
            draw
        };
        if let Some(draw) = draw {
            frame.draws.push(draw);
        }
    }

    /// The raster: the box the strips cover inside the window, in device pixels.
    fn rasterise_synapses(&self, strips: &[Strip], window: [f32; 2]) -> Option<UiDraw> {
        let s = self.s;
        let (mut low, mut high) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
        for strip in strips {
            let reach = strip.thickness * 0.5 + 3.0;
            for point in [strip.from, strip.to] {
                // The strips' `y` is up: the window's is down.
                let (x, y) = (point[0], -point[1]);
                low = [low[0].min(x - reach), low[1].min(y - reach)];
                high = [high[0].max(x + reach), high[1].max(y + reach)];
            }
        }
        let origin = [low[0].max(0.0).floor(), low[1].max(0.0).floor()];
        let end = [high[0].min(window[0]).ceil(), high[1].min(window[1]).ceil()];
        if strips.is_empty() || end[0] <= origin[0] || end[1] <= origin[1] {
            return None;
        }
        let size = [
            ((end[0] - origin[0]) * s).ceil().max(1.0) as u32,
            ((end[1] - origin[1]) * s).ceil().max(1.0) as u32,
        ];
        let rgba = rasterise(strips, [0.0, 0.0], origin, size, s, &pattern(self.ui));
        Some(UiDraw::Pixels {
            texture: Arc::new(UiTexture {
                width: size[0],
                height: size[1],
                rgba,
            }),
            pos: Vec2::new(origin[0] * s, origin[1] * s),
            size: Vec2::new(size[0] as f32, size[1] as f32),
        })
    }
}

// ---------------------------------------------------------------------------------------- the drawing

/// Draws the view over the whole window.
pub fn draw(
    frame: &mut UiFrame,
    d: &Drawer<'_>,
    app: &App,
    session: &ExpandedSession,
    width: f32,
    height: f32,
) {
    let s = d.s;
    // The panel covers the screen: what the panels under it have to say does not show through.
    frame.tips.clear();
    frame.rect(Vec2::ZERO, Vec2::new(width, height), BACKGROUND);
    frame.hit(
        HIT_PANEL,
        Vec2::ZERO,
        Vec2::new(width, height),
        UiAction::Consume,
    );
    frame.hit(
        HIT_CANVAS,
        Vec2::ZERO,
        Vec2::new(width, height),
        UiAction::ExpandedBrain(ExAction::Canvas),
    );
    let view = Viewport {
        size: [width, height],
        ui: s,
    };
    // The pointer is over the graph only where the top bar and the settings panel (drawn over it) are not.
    let cursor = [d.cursor.x / s, d.cursor.y / s];
    let inside = |rect: [f32; 4]| {
        cursor[0] >= rect[0]
            && cursor[0] < rect[0] + rect[2]
            && cursor[1] >= rect[1]
            && cursor[1] < rect[1] + rect[3]
    };
    let covered = top_buttons(width / s).into_iter().any(inside)
        || (session.view.settings_open && inside(SETTINGS));
    graph(frame, d, session, view, covered);
    top_bar(frame, d, session, width / s);
    if session.view.settings_open {
        settings_panel(frame, d, app, session);
    }
}

/// The synapses and the neurons; `covered` is whether something of the panel above them is under the pointer.
fn graph(
    frame: &mut UiFrame,
    d: &Drawer<'_>,
    session: &ExpandedSession,
    view: Viewport,
    covered: bool,
) {
    let s = d.s;
    let model = &session.model;
    let window = [view.size[0] / s, view.size[1] / s];
    let cursor = [d.cursor.x / s, d.cursor.y / s];
    let over_graph = !covered;
    let neuron_under = if over_graph {
        neuron_at(model, view, cursor)
    } else {
        None
    };
    let synapse_under = if over_graph && neuron_under.is_none() {
        synapse_at(model, view, cursor)
    } else {
        None
    };
    d.synapse_picture(frame, &strips_of(model, view, synapse_under), window);
    let side = NODE * model.scale;
    for (index, neuron) in model.neurons.iter().enumerate() {
        let middle = place(model, neuron.pos, view);
        let rect = [middle[0] - side / 2.0, middle[1] - side / 2.0, side, side];
        if rect[0] > window[0] || rect[1] > window[1] || rect[0] + side < 0.0 || rect[1] + side < 0.0 {
            continue;
        }
        let alpha = neuron.alpha();
        let byte = (alpha * 255.0).round() as u8;
        let mut tint = neuron_colour(neuron.value);
        tint[3] = byte;
        d.icon(frame, "small_circle_base_icon", rect, tint);
        let icon = ICON * model.scale;
        d.icon(
            frame,
            icon_name(neuron.index, neuron.node.node_type),
            [middle[0] - icon / 2.0, middle[1] - icon / 2.0, icon, icon],
            [255, 255, 255, byte],
        );
        let bias = neuron.node.base_activation;
        if neuron.index > N_INPUTS && bias.abs() > 0.0 {
            let sign = if bias > 0.0 { "plus" } else { "minus" };
            // `SetDesc` gives the sign the alpha of the bias (`sqrt(|bias|)`); `SetAlphaLow` replaces it by 1 or 0.02.
            let sign_alpha = if neuron.touched {
                alpha
            } else {
                bias.abs().clamp(0.0, 1.0).sqrt()
            };
            let size = SIGN * model.scale;
            d.icon(
                frame,
                sign,
                [rect[0] + side - size, rect[1], size, size],
                synapse_colour(bias, sign_alpha, true),
            );
        }
        frame.hit(
            HIT_NEURON + index as UiId,
            d.at(rect[0], rect[1]),
            d.at(rect[2], rect[3]),
            UiAction::ExpandedBrain(ExAction::Neuron(index as u16)),
        );
    }
    // The hovered neuron: its text (the prefab's "0.00" while it is not dimmed) over it, and its tooltip.
    if let Some(index) = neuron_under {
        let neuron = &model.neurons[index];
        let middle = place(model, neuron.pos, view);
        let rect = [middle[0] - side / 2.0, middle[1] - side / 2.0, side, side];
        if !neuron.low {
            let px = 20.0 * 1.7 * model.scale * s;
            let text = "0.00";
            let width = d.measure(text, px, false);
            frame.draws.push(UiDraw::Text {
                font: Some("LiberationSans SDF".to_owned()),
                text: text.to_owned(),
                pos: Vec2::new(middle[0] * s - width / 2.0, (rect[1] - 0.0) * s - px * 1.17),
                px,
                color: WHITE,
                role: FontRole::Body,
            });
        }
        if let Some((title, text)) = node_tip(neuron.index, &neuron.node, Mark::None) {
            frame.tips.push(TipRect {
                pos: d.at(rect[0], rect[1]),
                size: d.at(rect[2], rect[3]),
                tip: Tip {
                    path: "",
                    title,
                    content: tooltip::intern(text),
                    ui: true,
                    instant: false,
                },
            });
        }
    } else if let Some(index) = synapse_under {
        // A synapse under the pointer shows its weight over its middle (`UISynaps.OnPointerEnter`: the hover holder).
        let synapse = &model.synapses[index];
        let from = place(model, model.neurons[synapse.node_in].pos, view);
        let to = place(model, model.neurons[synapse.node_out].pos, view);
        let text = crate::value_edit::dotnet_float_string(synapse.weight);
        let px = 16.0 * model.scale * s;
        let width = d.measure(&text, px, false);
        let middle = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
        frame.draws.push(UiDraw::Text {
            font: Some("LiberationSans SDF".to_owned()),
            text,
            pos: Vec2::new(
                middle[0] * s - width / 2.0,
                (middle[1] - synapse.thickness * model.scale * 0.5) * s - px * 1.17,
            ),
            px,
            color: WHITE,
            role: FontRole::Body,
        });
    }
}

/// The top bar: the gear, and Exit, Help, Reset and Start or Stop.
fn top_bar(frame: &mut UiFrame, d: &Drawer<'_>, session: &ExpandedSession, width: f32) {
    let rects = top_buttons(width);
    let running = session.model.running;
    let buttons: [(&str, ExAction, &str, bool); 5] = [
        (
            "settings_icon",
            ExAction::Settings,
            "Top Bar/TopLeft/SettingsButton",
            true,
        ),
        (
            "exit_icon",
            ExAction::Exit,
            "Top Bar/TopRight/ExitButton",
            true,
        ),
        (
            "help_icon",
            ExAction::Help,
            "Top Bar/TopRight/HelpButton",
            true,
        ),
        (
            "reset_icon",
            ExAction::Reset,
            "Top Bar/TopRight/ResetButton",
            true,
        ),
        // Start and Stop take the same place: `StartForce` swaps them.
        if running {
            (
                "pause_icon",
                ExAction::Stop,
                "Top Bar/TopRight/StopButton",
                true,
            )
        } else {
            (
                "resume_icon",
                ExAction::Start,
                "Top Bar/TopRight/StartButton",
                true,
            )
        },
    ];
    for (slot, (icon, action, path, _)) in buttons.iter().enumerate() {
        let rect = rects[slot];
        d.slice(frame, "base_icon", 5.0, 1.0 / 0.75, rect, WHITE);
        d.icon(frame, icon, rect, WHITE);
        frame.hit(
            HIT_BUTTONS + slot as UiId,
            d.at(rect[0], rect[1]),
            d.at(rect[2], rect[3]),
            UiAction::ExpandedBrain(*action),
        );
        tip(frame, d, rect, path);
    }
}

/// A tooltip over `rect` for the control at `path` under the panel.
fn tip(frame: &mut UiFrame, d: &Drawer<'_>, rect: [f32; 4], path: &str) {
    if let Some(tip) = tooltip::find(&format!("{SCENE}{path}")) {
        frame.tips.push(TipRect {
            pos: d.at(rect[0], rect[1]),
            size: d.at(rect[2], rect[3]),
            tip,
        });
    }
}

/// The rules of typing a value into a parameter's row: the setting's own range, which it may leave (`canGoOutOfBounds` stays on).
#[must_use]
pub fn rules_of(param: Param) -> Rules {
    Rules {
        factor: 1.0,
        min: param.min(),
        max: param.max(),
        out_of_bounds: true,
        integer: false,
        log: false,
    }
}

/// How a parameter's value reads (`FloatSetting.formatting`: its precision, SI prefixes on).
#[must_use]
pub fn format_of(param: Param) -> bibites_core::format::FloatValueFormat {
    bibites_core::format::FloatValueFormat {
        precision: param.precision(),
        ..bibites_core::format::FloatValueFormat::default()
    }
}

/// The settings panel (`BrainViewSettingsPanel`).
fn settings_panel(frame: &mut UiFrame, d: &Drawer<'_>, app: &App, session: &ExpandedSession) {
    let s = d.s;
    let model = &session.model;
    frame.hit(
        HIT_SETTINGS,
        d.at(SETTINGS[0], SETTINGS[1]),
        d.at(SETTINGS[2], SETTINGS[3]),
        UiAction::ExpandedBrain(ExAction::Consume),
    );
    d.slice(frame, "Background", 10.0, 1.0 / 0.54, SETTINGS, [188, 188, 188, 255]);
    // The title: LiberationSans 25, left and at the bottom of its 25-high rectangle.
    let title = "Brain Inspector Settings";
    frame.text(
        title.to_owned(),
        d.at(SETTINGS[0] + 25.0, SETTINGS[1] + 15.0 + 25.1 - 25.0 * 1.17),
        25.0 * s,
        WHITE,
    );
    // The close `X`.
    let close = [SETTINGS[0] + SETTINGS[2] - 10.0 - 30.0, SETTINGS[1] + 10.0, 30.0, 30.0];
    d.icon(frame, "1727490", close, WHITE);
    frame.hit(
        HIT_SETTINGS + 1,
        d.at(close[0], close[1]),
        d.at(close[2], close[3]),
        UiAction::ExpandedBrain(ExAction::CloseSettings),
    );
    // The view and the scroll view in it: two white `Long_UI`s.
    d.slice(frame, "Long_UI", 70.0, 0.2, SETTINGS_VIEW, WHITE);
    d.slice(
        frame,
        "Long_UI",
        70.0,
        0.2,
        [SETTINGS_VIEW[0], SETTINGS_VIEW[1] + 10.4, SETTINGS_VIEW[2], SETTINGS_VIEW[3] - 20.8],
        WHITE,
    );
    let group = group_of(model.graph);
    let layout = column(group.len());
    // The graph-type button names what it switches to.
    let (graph_label, graph_path) = match model.graph {
        GraphType::Linear => (
            "Switch To Circular Graph",
            "BrainViewSettingsPanel/SettingsView/BrainSettings/Viewport/Content/TurnGraphToCircular",
        ),
        GraphType::Circular => (
            "Switch To Linear Graph",
            "BrainViewSettingsPanel/SettingsView/BrainSettings/Viewport/Content/TurnGraphToLinear",
        ),
    };
    let button = |frame: &mut UiFrame, rect: [f32; 4], label: &str, action: ExAction, slot: UiId, path: &str| {
        d.slice(frame, "Button_UI", 30.0, 1.0 / 8.0, rect, [222, 223, 222, 255]);
        let px = 18.0 * s;
        let width = d.fitted(label, px, false, (rect[2] - 10.0) * s);
        let size = width.min(px);
        let text_width = d.measure(label, size, false);
        frame.text(
            label.to_owned(),
            Vec2::new(
                (rect[0] + rect[2] / 2.0) * s - text_width / 2.0,
                (rect[1] + rect[3] / 2.0) * s - size * 0.585,
            ),
            size,
            [50, 50, 50, 255],
        );
        frame.hit(
            HIT_SETTINGS + slot,
            d.at(rect[0], rect[1]),
            d.at(rect[2], rect[3]),
            UiAction::ExpandedBrain(action),
        );
        tip(frame, d, rect, path);
    };
    button(frame, layout.graph_button, graph_label, ExAction::ToggleGraph, 2, graph_path);
    button(
        frame,
        layout.disabled_button,
        "Toggle Disabled Synapse Visibility",
        ExAction::ToggleDisabled,
        3,
        "BrainViewSettingsPanel/SettingsView/BrainSettings/Viewport/Content/ToggleDisabledButton",
    );
    // The group: its title (Boston Black 36, centred) and a row for each parameter.
    let title = match model.graph {
        GraphType::Linear => "Linear Graph Parameters",
        GraphType::Circular => "Spherical Graph Parameters",
    };
    let size = 36.0 * s;
    let fitted = d.fitted(title, size, true, layout.title[2] * s);
    let text_width = d.measure(title, fitted, true);
    frame.draws.push(UiDraw::Text {
        font: Some("Boston Black SDF".to_owned()),
        text: title.to_owned(),
        pos: Vec2::new(
            (layout.title[0] + layout.title[2] / 2.0) * s - text_width / 2.0,
            (layout.title[1] + layout.title[3] / 2.0) * s - fitted * 0.585,
        ),
        px: fitted,
        color: WHITE,
        role: FontRole::Bold,
    });
    let everything = [0.0, 0.0, 100_000.0, 100_000.0];
    let place = |rect: [f32; 4]| [rect[0] * s, rect[1] * s, rect[2] * s, rect[3] * s];
    let mut next_hit = HIT_SETTINGS + 10;
    for (param, row) in group.iter().zip(&layout.rows) {
        let value = model.param(*param);
        let field = FieldRef::Expanded(param.index() as u8);
        let editing = app.value_edit.as_ref().filter(|edit| edit.field == field);
        let spec = SliderSpec {
            name: param.name(),
            help: param.help(),
            field,
            value,
            format: format_of(*param),
            rules: rules_of(*param),
            fraction: fraction_range(LINEAR, param.min(), param.max(), param.default_value(), value)
                .clamp(0.0, 1.0),
            default_fraction: fraction_range(
                LINEAR,
                param.min(),
                param.max(),
                param.default_value(),
                param.default_value(),
            )
            .clamp(0.0, 1.0),
            reset: UiAction::ExpandedBrain(ExAction::ResetParam(param.index() as u8)),
            lineage: false,
        };
        let index = param.index() as u8;
        d.slider_cells(
            frame,
            &spec,
            editing,
            *row,
            0.0,
            &|from, to| UiAction::ExpandedBrain(ExAction::Slide(index, from, to)),
            &place,
            everything,
            &mut next_hit,
        );
    }
}

// ---------------------------------------------------------------------------------------- the interaction

impl App {
    /// The window's size and the UI scale as the model's maths want them.
    fn expanded_viewport(&self) -> Viewport {
        Viewport {
            size: [self.camera.screen.x, self.camera.screen.y],
            ui: UiFrame::scaler(self.camera.screen.x, self.camera.screen.y),
        }
    }

    /// `GUIManager.OpenBibitePanel(ExpendedBrainPanel)` (the Brain panel's Expanded View button): the selected bibite's brain opens
    /// on the whole window and the game pauses; the other bibite panels close, `UserControl.AllowControl` is off (the hotkeys, the
    /// camera and the selection leave the world alone).
    pub fn open_expanded_brain(&mut self) {
        if self.expanded_brain.is_some() {
            return;
        }
        self.brain_sorts += 1;
        let seed = self.brain_sorts.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let view = self
            .selection
            .as_ref()
            .map(|panel| panel.target)
            .and_then(|target| self.target_brain(target))
            .map(bibites_core::brain::Brain::graph_view);
        let mut model = ExpandedBrain::open(
            view.as_ref(),
            default_params(),
            self.expanded_graph,
            self.expanded_pan,
            &mut bibites_brainsort::Rng::new(seed),
        );
        let viewport = self.expanded_viewport();
        model.pan = model.clamped(model.pan, viewport);
        self.expanded_brain = Some(ExpandedSession {
            model,
            view: ExpandedView {
                settings_open: self.expanded_settings_open,
                ..ExpandedView::default()
            },
        });
        // The inspector's panel closes (`CloseAllPanels`); the Expanded View is the newest of the Escape stack.
        self.inspector_open = false;
        self.value_edit = None;
        self.sync_escape_stack();
        self.panels.close(crate::ui::PanelKind::Inspector);
        self.panels.open_once(crate::ui::PanelKind::ExpandedBrain);
    }

    /// `ExpandedBrainPanel.ClosePanel`: the neurons go, the settings go back to their defaults, the graph type and the canvas' place stay.
    pub fn close_expanded_brain(&mut self) {
        if let Some(session) = self.expanded_brain.take() {
            self.expanded_graph = session.model.graph;
            self.expanded_pan = session.model.pan;
            self.expanded_settings_open = session.view.settings_open;
        }
        self.value_edit = None;
        self.panels.close(crate::ui::PanelKind::ExpandedBrain);
    }

    /// `ExitButton` (`OpenBibitePanel(BrainPanel)`): the view closes and the inspector's Brain panel opens.
    pub fn expanded_brain_exit(&mut self) {
        self.close_expanded_brain();
        self.open_bibite_panel(crate::ui::InspectorTab::Brain, true);
    }

    /// `reset()`: `ClosePanel(); OpenPanel();` (new random places, the zoom of a new view; the graph type, the canvas' place and the open
    /// settings panel stay; the settings are the defaults again).
    pub fn expanded_brain_reset(&mut self) {
        self.close_expanded_brain();
        self.open_expanded_brain();
    }

    /// Escape: a typed field closes first (the caller), then the settings panel (`UIPanel`, the newest entry of the stack), then the
    /// view itself (`BibitePanel.Escape`: `ResetPanelToNone`, no panel opens in its place). Returns whether the view was open.
    pub fn expanded_brain_escape(&mut self) -> bool {
        let Some(session) = self.expanded_brain.as_mut() else {
            return false;
        };
        // The settings panel's entry is above the view's unless the view was closed and opened again with the panel still open (the entry then
        // stays below it): the view goes first, the panel stays open and its entry stays in the stack.
        let above = self.panels.position(crate::ui::PanelKind::ExpandedSettings) > self.panels.position(crate::ui::PanelKind::ExpandedBrain);
        if session.view.settings_open && above {
            session.view.settings_open = false;
            self.panels.close(crate::ui::PanelKind::ExpandedSettings);
            session.view.drag = None;
        } else {
            self.close_expanded_brain();
        }
        true
    }

    /// A control of the view was clicked (the canvas, the neurons and the sliders are held: [`Self::route_expanded_pointer`]).
    pub fn apply_expanded_action(&mut self, action: ExAction) {
        match action {
            ExAction::Exit => self.expanded_brain_exit(),
            ExAction::Reset => self.expanded_brain_reset(),
            ExAction::Help => {
                // `GameManager.brainHelp`.
                self.invoke_command(crate::ui::ControlCommand::OpenBrainHelp);
            }
            _ => {
                let Some(session) = self.expanded_brain.as_mut() else {
                    return;
                };
                match action {
                    ExAction::Settings => {
                        session.view.settings_open = !session.view.settings_open;
                        // `TogglePanel`: opening adds an entry on top of the stack, closing takes it out.
                        if session.view.settings_open {
                            self.panels.open_once(crate::ui::PanelKind::ExpandedSettings);
                        } else {
                            self.panels.close(crate::ui::PanelKind::ExpandedSettings);
                        }
                    }
                    ExAction::CloseSettings => {
                        session.view.settings_open = false;
                        self.panels.close(crate::ui::PanelKind::ExpandedSettings);
                    }
                    ExAction::Start => session.model.set_running(true),
                    ExAction::Stop => session.model.set_running(false),
                    ExAction::ToggleGraph => session.model.toggle_graph(),
                    ExAction::ToggleDisabled => session.model.toggle_disabled(),
                    ExAction::ResetParam(index) => {
                        if let Some(param) = Param::from_index(usize::from(index)) {
                            session.model.set_param(param, param.default_value());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// One frame of the view: the simulation runs with the real frame time (`Time.unscaledDeltaTime`; a long stall counts as a tenth of a
    /// second so a hitch does not throw the neurons about).
    pub fn expanded_brain_frame(&mut self, dt: f32) {
        let viewport = self.expanded_viewport();
        if let Some(session) = self.expanded_brain.as_mut() {
            session.model.step(dt.clamp(0.0, 0.1), viewport);
        }
    }

    /// The wheel over the view: `Zoom(Input.GetAxis("Mouse ScrollWheel"))`, a tenth of the zoom a notch.
    pub fn expanded_brain_wheel(&mut self, cursor: Vec2, wheel: f32) {
        let viewport = self.expanded_viewport();
        if let Some(session) = self.expanded_brain.as_mut() {
            session
                .model
                .zoom(wheel * 0.1, [cursor.x, cursor.y], viewport);
        }
    }

    /// (Headless script) The canvas moved to a place (canvas units), kept inside its limits.
    pub fn expanded_brain_pan_to(&mut self, pan: [f32; 2]) {
        let viewport = self.expanded_viewport();
        if let Some(session) = self.expanded_brain.as_mut() {
            session.model.pan = session.model.clamped(pan, viewport);
        }
    }

    /// (Headless script) A neuron dragged to a window point and let go.
    pub fn expanded_brain_drag_neuron(&mut self, neuron: usize, to: Vec2) {
        let viewport = self.expanded_viewport();
        if let Some(session) = self.expanded_brain.as_mut() {
            session.model.drag_neuron(neuron, [to.x, to.y], viewport);
            session.model.release_neuron(neuron);
        }
    }

    /// A parameter's slider follows the cursor (`SettingSlider.SetValueFromSlider`: a linear map of the track).
    fn expanded_slide(&mut self, param: u8, track: (f32, f32), cursor_x: f32) {
        let scale = UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let span = track.1 - track.0;
        let (Some(param), Some(session)) = (
            Param::from_index(usize::from(param)),
            self.expanded_brain.as_mut(),
        ) else {
            return;
        };
        if span <= 0.0 {
            return;
        }
        let fraction = ((cursor_x / scale - track.0) / span).clamp(0.0, 1.0);
        let value = value_range(LINEAR, param.min(), param.max(), param.default_value(), fraction);
        session.model.set_param(param, value);
    }

    /// The rules of typing a value into a parameter's row and the value it holds now.
    #[must_use]
    pub fn expanded_typed_rules(&self, field: FieldRef) -> Option<(Rules, f32)> {
        let FieldRef::Expanded(index) = field else {
            return None;
        };
        let param = Param::from_index(usize::from(index))?;
        let session = self.expanded_brain.as_ref()?;
        Some((rules_of(param), session.model.param(param)))
    }

    /// A typed value of a parameter's row.
    pub fn expanded_apply_value(&mut self, field: FieldRef, value: f32) {
        let FieldRef::Expanded(index) = field else {
            return;
        };
        if let (Some(param), Some(session)) = (
            Param::from_index(usize::from(index)),
            self.expanded_brain.as_mut(),
        ) {
            session.model.set_param(param, value);
        }
    }

    /// The right button's edge: `true` when it went down this frame; a release ends a pan.
    fn expanded_secondary_edge(&mut self, down: bool) -> bool {
        let Some(session) = self.expanded_brain.as_mut() else {
            return false;
        };
        let view = &mut session.view;
        let edge = down && !view.secondary_down;
        view.secondary_down = down;
        if !down && matches!(view.drag, Some(ExDrag::Pan { .. })) {
            view.drag = None;
        }
        edge
    }

    /// The view's share of a frame's pointer routing: presses on the canvas, a neuron and a slider start to be held, and a held press
    /// follows the cursor until it is let go. Returns `true` when the pointer was taken (the mouse fields of `input` are then
    /// cleared). `secondary` is the right button, which the caller keeps out of the frame's input.
    pub fn route_expanded_pointer(
        &mut self,
        frame: &UiFrame,
        input: &mut FrameInput,
        secondary: bool,
    ) -> bool {
        if self.expanded_brain.is_none() {
            return false;
        }
        let cursor = input.cursor;
        let action = match frame.action_at(cursor) {
            Some(UiAction::ExpandedBrain(action)) => Some(action),
            _ => None,
        };
        let right_edge = self.expanded_secondary_edge(secondary);
        let at = [cursor.x, cursor.y];
        if right_edge && action == Some(ExAction::Canvas) {
            // The right button pans only from the scroll view itself (a neuron or a button takes it).
            if let Some(session) = self.expanded_brain.as_mut() {
                session.view.drag = Some(ExDrag::Pan {
                    start: at,
                    from: None,
                });
            }
        }
        if input.primary_pressed {
            self.value_edit = None;
            if let Some(session) = self.expanded_brain.as_mut() {
                session.view.drag = match action {
                    Some(ExAction::Canvas) => Some(ExDrag::Canvas),
                    Some(ExAction::Neuron(index)) => Some(ExDrag::Neuron {
                        index: usize::from(index),
                        start: at,
                        moved: false,
                    }),
                    Some(ExAction::Slide(param, x0, x1)) => Some(ExDrag::Slider(param, x0, x1)),
                    _ => session.view.drag,
                };
            }
        }
        let Some(drag) = self
            .expanded_brain
            .as_ref()
            .and_then(|session| session.view.drag)
        else {
            return false;
        };
        let right = matches!(drag, ExDrag::Pan { .. });
        let down = if right { secondary } else { input.primary_down };
        if down {
            self.expanded_drag_to(drag, cursor);
        }
        if !down || (!right && input.primary_released) {
            self.expanded_release(drag, action);
        }
        input.primary_pressed = false;
        input.primary_released = false;
        input.primary_down = false;
        input.wheel = 0.0;
        true
    }

    /// The held press follows the cursor (window px).
    fn expanded_drag_to(&mut self, drag: ExDrag, cursor: Vec2) {
        let viewport = self.expanded_viewport();
        let scale = viewport.ui;
        let at = [cursor.x, cursor.y];
        // `EventSystem.pixelDragThreshold`.
        let far = |start: [f32; 2]| (at[0] - start[0]).hypot(at[1] - start[1]) >= 10.0;
        let Some(session) = self.expanded_brain.as_mut() else {
            return;
        };
        match drag {
            ExDrag::Neuron {
                index,
                start,
                moved,
            } => {
                if moved || far(start) {
                    session.view.drag = Some(ExDrag::Neuron {
                        index,
                        start,
                        moved: true,
                    });
                    session.model.drag_neuron(index, at, viewport);
                }
            }
            ExDrag::Canvas => {}
            ExDrag::Pan { start, from } => {
                let from = match from {
                    Some(from) => from,
                    None if far(start) => {
                        // `ScrollRect.OnBeginDrag`: the cursor and the canvas where the threshold was passed.
                        [at[0], at[1], session.model.pan[0], session.model.pan[1]]
                    }
                    None => return,
                };
                // `ScrollRect.OnDrag` (clamped): where the canvas began plus the cursor's whole way, kept inside its limits.
                let pan = session.model.clamped(
                    [
                        from[2] + (at[0] - from[0]) / scale,
                        from[3] - (at[1] - from[1]) / scale,
                    ],
                    viewport,
                );
                session.model.pan = pan;
                session.view.drag = Some(ExDrag::Pan {
                    start,
                    from: Some(from),
                });
            }
            ExDrag::Slider(param, x0, x1) => {
                self.expanded_slide(param, (f32::from(x0), f32::from(x1)), cursor.x);
            }
        }
    }

    /// The held press was let go: a press on a neuron that did not move is a click (the highlight), one on the canvas takes it away.
    fn expanded_release(&mut self, drag: ExDrag, action: Option<ExAction>) {
        let Some(session) = self.expanded_brain.as_mut() else {
            return;
        };
        session.view.drag = None;
        match drag {
            ExDrag::Neuron {
                index, moved: false, ..
            } if action == Some(ExAction::Neuron(index as u16)) => {
                if let Some(node) = session.model.neurons.get(index).map(|neuron| neuron.index) {
                    session.model.bfs(node);
                }
            }
            ExDrag::Neuron { index, .. } => session.model.release_neuron(index),
            // `ExpandedBrainScroll.OnPointerClick`: a left click that ends on the canvas.
            ExDrag::Canvas if action == Some(ExAction::Canvas) => session.model.reset_highlight(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LoadedWorld;
    use crate::expanded_brain::Kind;
    use crate::ui::{ControlCommand, InspectorTab, PanelKind};

    /// An app on the starter world with the first bibite selected and the Brain panel the chosen inspector panel.
    fn app_with_bibite() -> App {
        let loaded = LoadedWorld::load(None, 31, 8, 1).expect("starter world");
        let mut app = App::new(loaded, Vec2::new(1920.0, 1080.0), None);
        assert!(app.select_first());
        app.open_bibite_panel(InspectorTab::Brain, false);
        app
    }

    fn open_view(app: &mut App) {
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenExpandedBrain)]);
        assert!(app.expanded_brain.is_some(), "the view opened");
    }

    fn frame_of(app: &App) -> UiFrame {
        let mut frame = UiFrame::new(1.0);
        let session = app.expanded_brain.as_ref().expect("open");
        let assets = crate::ui::UiAssets::empty();
        let drawer = Drawer::new(1.0, &assets, None, app.cursor);
        draw(&mut frame, &drawer, app, session, 1920.0, 1080.0);
        frame
    }

    fn actions_of(frame: &UiFrame) -> Vec<ExAction> {
        frame
            .actions
            .iter()
            .filter_map(|(_, action)| match action {
                UiAction::ExpandedBrain(action) => Some(*action),
                _ => None,
            })
            .collect()
    }

    fn model(app: &App) -> &ExpandedBrain {
        &app.expanded_brain.as_ref().expect("open").model
    }

    fn viewport() -> Viewport {
        Viewport {
            size: [1920.0, 1080.0],
            ui: 1.0,
        }
    }

    fn texts(frame: &UiFrame) -> Vec<String> {
        frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// A press of the left button at `at` and its release, through the window's routing.
    fn click(app: &mut App, at: Vec2) {
        app.cursor = at;
        let frame = frame_of(app);
        let mut input = FrameInput {
            cursor: at,
            primary_pressed: true,
            primary_down: true,
            ..FrameInput::default()
        };
        app.route_expanded_pointer(&frame, &mut input, false);
        let mut input = FrameInput {
            cursor: at,
            primary_released: true,
            ..FrameInput::default()
        };
        app.route_expanded_pointer(&frame, &mut input, false);
    }

    #[test]
    fn the_brain_panels_button_opens_the_view_over_the_game_and_pauses_it() {
        let mut app = app_with_bibite();
        assert!(app.inspector_visible());
        open_view(&mut app);
        // The inspector's panel closed and the view is on the Escape stack.
        assert!(!app.inspector_open);
        assert!(app.panels.is_open(PanelKind::ExpandedBrain));
        // `TogglePauseGame("ExpendedBrainPanel")`.
        app.paused = false;
        app.advance(0.05);
        assert!(app.paused, "the game pauses while the view is open");
        // `UserControl.AllowControl = false`: Space does not unpause.
        app.handle_input(&FrameInput {
            toggle_pause: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(app.paused);
        // Exit opens the Brain panel again and the game goes on.
        app.apply_ui_actions(&[UiAction::ExpandedBrain(ExAction::Exit)]);
        assert!(app.expanded_brain.is_none());
        assert!(app.inspector_open && app.inspector_tab == InspectorTab::Brain);
        app.advance(0.05);
        assert!(!app.paused, "the pause of the view ended with it");
    }

    #[test]
    fn a_view_closed_with_its_settings_open_leaves_an_escape_entry_that_swallows_one_press() {
        let mut app = app_with_bibite();
        open_view(&mut app);
        app.apply_ui_actions(&[UiAction::ExpandedBrain(ExAction::Settings)]);
        app.apply_ui_actions(&[UiAction::ExpandedBrain(ExAction::Exit)]);
        assert!(app.expanded_brain.is_none() && app.panels.is_open(crate::ui::PanelKind::ExpandedSettings));
        let escape = FrameInput { quit: true, dt: 0.016, ..FrameInput::default() };
        // The Brain panel the exit opened is newer than the entry, the selection older: the entry is spent before the selection goes.
        let mut presses = 0;
        while app.panels.is_open(crate::ui::PanelKind::ExpandedSettings) && presses < 4 {
            app.handle_input(&escape);
            presses += 1;
        }
        assert_eq!(presses, 2, "the Brain panel closes, then the left-over entry takes a press");
        assert!(app.selected.is_some(), "the selection is still there");
        assert!(!app.panels.is_open(crate::ui::PanelKind::Escape), "no pause menu opened");
        app.handle_input(&escape);
        assert!(app.selected.is_none());
        app.handle_input(&escape);
        assert!(app.panels.is_open(crate::ui::PanelKind::Escape), "the next press opens it");
    }

    #[test]
    fn escape_closes_the_settings_first_then_the_view_without_opening_a_panel() {
        let mut app = app_with_bibite();
        open_view(&mut app);
        app.apply_ui_actions(&[UiAction::ExpandedBrain(ExAction::Settings)]);
        assert!(app.expanded_brain.as_ref().expect("open").view.settings_open);
        app.handle_input(&FrameInput {
            quit: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(app.expanded_brain.is_some());
        assert!(!app.expanded_brain.as_ref().expect("open").view.settings_open);
        app.handle_input(&FrameInput {
            quit: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(app.expanded_brain.is_none());
        assert!(
            !app.inspector_open,
            "Escape is `ResetPanelToNone`: no panel opens in its place"
        );
        assert!(!app.panels.is_open(PanelKind::Escape));
    }

    #[test]
    fn the_top_bar_has_the_five_buttons_and_start_becomes_stop() {
        let mut app = app_with_bibite();
        open_view(&mut app);
        let actions = actions_of(&frame_of(&app));
        for wanted in [
            ExAction::Settings,
            ExAction::Exit,
            ExAction::Help,
            ExAction::Reset,
            ExAction::Start,
        ] {
            assert!(actions.contains(&wanted), "{wanted:?}");
        }
        assert!(!actions.contains(&ExAction::Stop));
        // The buttons are 60 x 60, from the right edge 10 in and 10 apart.
        let rects = top_buttons(1920.0);
        assert_eq!(rects[0], [10.0, 10.0, 60.0, 60.0]);
        assert_eq!(rects[1], [1850.0, 10.0, 60.0, 60.0]);
        assert_eq!(rects[4][0], 1920.0 - 10.0 - 60.0 - 3.0 * 70.0);
        app.apply_ui_actions(&[UiAction::ExpandedBrain(ExAction::Start)]);
        assert!(model(&app).running);
        let actions = actions_of(&frame_of(&app));
        assert!(actions.contains(&ExAction::Stop) && !actions.contains(&ExAction::Start));
    }

    #[test]
    fn the_settings_panel_lists_the_graph_types_parameters_and_swaps_with_it() {
        let mut app = app_with_bibite();
        open_view(&mut app);
        assert!(!actions_of(&frame_of(&app)).contains(&ExAction::ToggleGraph));
        app.apply_ui_actions(&[UiAction::ExpandedBrain(ExAction::Settings)]);
        let slides = |app: &App| -> Vec<u8> {
            actions_of(&frame_of(app))
                .into_iter()
                .filter_map(|action| match action {
                    ExAction::Slide(param, ..) => Some(param),
                    _ => None,
                })
                .collect()
        };
        let linear: Vec<u8> = LINEAR_PARAMS
            .iter()
            .map(|param| param.index() as u8)
            .collect();
        assert_eq!(slides(&app), linear);
        let actions = actions_of(&frame_of(&app));
        assert!(
            actions.contains(&ExAction::ToggleGraph) && actions.contains(&ExAction::ToggleDisabled)
        );
        assert_eq!(
            actions
                .iter()
                .filter(|action| matches!(action, ExAction::ResetParam(_)))
                .count(),
            6
        );
        // The graph-type button swaps the group (`ToggleGraphType`, the holders' `SetActive`).
        app.apply_ui_actions(&[UiAction::ExpandedBrain(ExAction::ToggleGraph)]);
        assert_eq!(model(&app).graph, GraphType::Circular);
        let spherical: Vec<u8> = SPHERICAL_PARAMS
            .iter()
            .map(|param| param.index() as u8)
            .collect();
        assert_eq!(slides(&app), spherical);
        // The column: the buttons 60 high and 20 apart, then the group's title and its rows.
        let layout = column(6);
        assert_eq!(layout.graph_button, [40.0, 90.4, 495.0, 60.0]);
        assert_eq!(layout.disabled_button[1], 170.4);
        assert_eq!(layout.title[1], 250.4);
        assert_eq!(layout.rows[0][1], 250.4 + 70.0);
        assert_eq!(layout.rows[1][1] - layout.rows[0][1], 60.0);
    }

    #[test]
    fn a_slider_follows_the_cursor_its_reset_restores_the_default_and_a_typed_value_may_leave_the_range()
     {
        let mut app = app_with_bibite();
        open_view(&mut app);
        app.apply_ui_actions(&[UiAction::ExpandedBrain(ExAction::Settings)]);
        let frame = frame_of(&app);
        let attraction = Param::LinearSynapseAttraction;
        let (index, x0, x1) = actions_of(&frame)
            .into_iter()
            .find_map(|action| match action {
                ExAction::Slide(index, x0, x1) if usize::from(index) == attraction.index() => {
                    Some((index, x0, x1))
                }
                _ => None,
            })
            .expect("the slider");
        assert_eq!(model(&app).param(attraction), 1.5);
        // A press on the track and a drag to past its right end: the maximum (5); past its left end: the minimum (0.1).
        let row = column(6).rows[0];
        let y = row[1] + 20.0;
        let mut input = FrameInput {
            cursor: Vec2::new(f32::from(x0) + 10.0, y),
            primary_pressed: true,
            primary_down: true,
            ..FrameInput::default()
        };
        app.cursor = input.cursor;
        assert!(app.route_expanded_pointer(&frame, &mut input, false));
        let mut input = FrameInput {
            cursor: Vec2::new(f32::from(x1) + 50.0, y),
            primary_down: true,
            ..FrameInput::default()
        };
        assert!(app.route_expanded_pointer(&frame, &mut input, false));
        assert!((model(&app).param(attraction) - 5.0).abs() < 1e-4);
        let mut input = FrameInput {
            cursor: Vec2::new(f32::from(x0) - 50.0, y),
            primary_down: true,
            ..FrameInput::default()
        };
        app.route_expanded_pointer(&frame, &mut input, false);
        assert!((model(&app).param(attraction) - 0.1).abs() < 1e-4);
        let mut input = FrameInput {
            cursor: Vec2::new(f32::from(x0) - 50.0, y),
            primary_released: true,
            ..FrameInput::default()
        };
        app.route_expanded_pointer(&frame, &mut input, false);
        assert!(
            app.expanded_brain
                .as_ref()
                .expect("open")
                .view
                .drag
                .is_none()
        );
        // The reset button.
        app.apply_ui_actions(&[UiAction::ExpandedBrain(ExAction::ResetParam(index))]);
        assert_eq!(model(&app).param(attraction), 1.5);
        // A typed value (the value text opens the field): the setting's own range, which it may leave.
        let (rules, now) = app
            .expanded_typed_rules(FieldRef::Expanded(index))
            .expect("rules");
        assert_eq!(now, 1.5);
        assert_eq!((rules.min, rules.max, rules.out_of_bounds), (0.1, 5.0, true));
        assert_eq!(rules.submitted("50"), Some(50.0));
        app.expanded_apply_value(FieldRef::Expanded(index), 50.0);
        assert_eq!(model(&app).param(attraction), 50.0);
        // The name's tooltip is the setting's help.
        assert!(attraction.help().starts_with("How strongly nodes are pulled"));
    }

    #[test]
    fn closing_puts_the_settings_back_and_keeps_the_graph_type_and_the_canvas_place() {
        let mut app = app_with_bibite();
        open_view(&mut app);
        app.apply_ui_actions(&[UiAction::ExpandedBrain(ExAction::ToggleGraph)]);
        app.expanded_apply_value(
            FieldRef::Expanded(Param::GravityToCenter.index() as u8),
            120.0,
        );
        app.expanded_brain_pan_to([30.0, -20.0]);
        let pan = model(&app).pan;
        app.apply_ui_actions(&[UiAction::ExpandedBrain(ExAction::Settings)]);
        app.close_expanded_brain();
        assert!(app.expanded_brain.is_none() && !app.panels.is_open(PanelKind::ExpandedBrain));
        open_view(&mut app);
        let panel = model(&app);
        assert_eq!(panel.graph, GraphType::Circular, "`graphType` stays");
        assert_eq!(
            panel.pan,
            panel.clamped(pan, viewport()),
            "the holder keeps its place"
        );
        assert_eq!(
            panel.param(Param::GravityToCenter),
            90.0,
            "`ResetValue` on close"
        );
        assert!(
            app.expanded_brain.as_ref().expect("open").view.settings_open,
            "the settings panel is a child with its own flag"
        );
        // Reset is a close and an open: new neurons, the defaults, and the rest stays.
        app.expanded_apply_value(
            FieldRef::Expanded(Param::GravityToCenter.index() as u8),
            120.0,
        );
        app.apply_ui_actions(&[UiAction::ExpandedBrain(ExAction::Reset)]);
        assert!(app.expanded_brain.is_some());
        assert_eq!(model(&app).param(Param::GravityToCenter), 90.0);
        assert_eq!(model(&app).graph, GraphType::Circular);
    }

    #[test]
    fn a_click_on_a_neuron_highlights_what_it_reaches_and_one_on_the_background_takes_it_away() {
        let mut app = app_with_bibite();
        open_view(&mut app);
        let view = viewport();
        // The first input neuron.
        let first = model(&app)
            .neurons
            .iter()
            .position(|neuron| neuron.kind == Kind::Input)
            .expect("an input");
        let at = place(model(&app), model(&app).neurons[first].pos, view);
        click(&mut app, Vec2::new(at[0], at[1]));
        let panel = model(&app);
        assert!(!panel.neurons[first].low, "the clicked neuron is reachable");
        assert!(
            panel.neurons.iter().any(|neuron| neuron.low),
            "what it does not reach dims"
        );
        assert!(
            panel
                .synapses
                .iter()
                .any(|synapse| synapse.enabled && synapse.low)
        );
        // A click on the background (a place with no neuron): the highlight goes.
        click(&mut app, Vec2::new(5.0, 900.0));
        assert!(model(&app).neurons.iter().all(|neuron| !neuron.low));
        assert!(model(&app).synapses.iter().all(|synapse| !synapse.low));
    }

    #[test]
    fn a_neuron_dragged_follows_the_cursor_and_is_not_clicked() {
        let mut app = app_with_bibite();
        open_view(&mut app);
        let view = viewport();
        let first = 0;
        let at = place(model(&app), model(&app).neurons[first].pos, view);
        let start = Vec2::new(at[0], at[1]);
        app.cursor = start;
        let frame = frame_of(&app);
        let mut input = FrameInput {
            cursor: start,
            primary_pressed: true,
            primary_down: true,
            ..FrameInput::default()
        };
        assert!(app.route_expanded_pointer(&frame, &mut input, false));
        // Far enough it is a drag and the neuron goes to the cursor.
        let target = Vec2::new(start.x + 120.0, start.y + 80.0);
        let mut input = FrameInput {
            cursor: target,
            primary_down: true,
            ..FrameInput::default()
        };
        app.route_expanded_pointer(&frame, &mut input, false);
        let dragged = place(model(&app), model(&app).neurons[first].pos, view);
        assert!(
            (dragged[0] - target.x).abs() < 0.5 && (dragged[1] - target.y).abs() < 0.5,
            "{dragged:?}"
        );
        assert!(model(&app).neurons[first].dragged);
        let mut input = FrameInput {
            cursor: target,
            primary_released: true,
            ..FrameInput::default()
        };
        app.route_expanded_pointer(&frame, &mut input, false);
        assert!(!model(&app).neurons[first].dragged);
        assert!(
            model(&app).neurons.iter().all(|neuron| !neuron.low),
            "a drag is not a click"
        );
    }

    #[test]
    fn the_wheel_zooms_and_the_right_button_pans_after_ten_pixels() {
        let mut app = app_with_bibite();
        open_view(&mut app);
        let scale = model(&app).scale;
        app.expanded_brain_wheel(Vec2::new(960.0, 540.0), 1.0);
        assert!((model(&app).scale - scale * 1.1).abs() < 1e-5);
        // The right button on the background: nothing moves within the threshold, then the canvas follows the cursor's whole way.
        let start = Vec2::new(300.0, 900.0);
        app.cursor = start;
        let frame = frame_of(&app);
        let pan = model(&app).pan;
        let mut input = FrameInput {
            cursor: start,
            ..FrameInput::default()
        };
        assert!(app.route_expanded_pointer(&frame, &mut input, true));
        let mut input = FrameInput {
            cursor: Vec2::new(start.x + 4.0, start.y),
            ..FrameInput::default()
        };
        app.route_expanded_pointer(&frame, &mut input, true);
        assert_eq!(model(&app).pan, pan);
        let mut input = FrameInput {
            cursor: Vec2::new(start.x + 20.0, start.y),
            ..FrameInput::default()
        };
        app.route_expanded_pointer(&frame, &mut input, true);
        let mut input = FrameInput {
            cursor: Vec2::new(start.x + 50.0, start.y),
            ..FrameInput::default()
        };
        app.route_expanded_pointer(&frame, &mut input, true);
        let moved = model(&app).pan;
        assert!(
            (moved[0] - pan[0] - 30.0).abs() < 0.01 && moved[1] == pan[1],
            "{moved:?} from {pan:?}"
        );
        // The button let go ends the pan.
        let mut input = FrameInput {
            cursor: Vec2::new(start.x + 50.0, start.y),
            ..FrameInput::default()
        };
        app.route_expanded_pointer(&frame, &mut input, false);
        assert!(
            app.expanded_brain
                .as_ref()
                .expect("open")
                .view
                .drag
                .is_none()
        );
        // A right press on a neuron does not pan.
        let neuron = place(model(&app), model(&app).neurons[0].pos, viewport());
        let on = Vec2::new(neuron[0], neuron[1]);
        app.cursor = on;
        let frame = frame_of(&app);
        let mut input = FrameInput {
            cursor: on,
            ..FrameInput::default()
        };
        app.route_expanded_pointer(&frame, &mut input, true);
        assert!(
            app.expanded_brain
                .as_ref()
                .expect("open")
                .view
                .drag
                .is_none()
        );
    }

    #[test]
    fn the_simulation_runs_only_after_start_and_moves_with_real_time() {
        let mut app = app_with_bibite();
        open_view(&mut app);
        let positions = |app: &App| -> Vec<[f32; 2]> {
            model(app).neurons.iter().map(|neuron| neuron.pos).collect()
        };
        let before = positions(&app);
        app.expanded_brain_frame(0.016);
        assert_eq!(before, positions(&app), "it waits for Start");
        app.apply_ui_actions(&[UiAction::ExpandedBrain(ExAction::ToggleGraph)]);
        app.apply_ui_actions(&[UiAction::ExpandedBrain(ExAction::Start)]);
        for _ in 0..30 {
            app.expanded_brain_frame(0.016);
        }
        assert!(
            before != positions(&app),
            "the circular graph moves the neurons"
        );
        app.apply_ui_actions(&[UiAction::ExpandedBrain(ExAction::Stop)]);
        assert!(!model(&app).running);
    }

    #[test]
    fn the_hover_shows_the_prefabs_text_over_a_neuron_and_the_weight_over_a_synapse() {
        let mut app = app_with_bibite();
        open_view(&mut app);
        let view = viewport();
        let neuron = place(model(&app), model(&app).neurons[0].pos, view);
        app.cursor = Vec2::new(neuron[0], neuron[1]);
        let frame = frame_of(&app);
        assert!(
            texts(&frame).contains(&"0.00".to_owned()),
            "{:?}",
            texts(&frame)
        );
        assert!(!frame.tips.is_empty(), "the neuron's tooltip");
        // The middle of a synapse (away from the neurons at its ends).
        let (middle, weight) = {
            let panel = model(&app);
            let synapse = &panel.synapses[0];
            let from = place(panel, panel.neurons[synapse.node_in].pos, view);
            let to = place(panel, panel.neurons[synapse.node_out].pos, view);
            (
                [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0],
                crate::value_edit::dotnet_float_string(synapse.weight),
            )
        };
        app.cursor = Vec2::new(middle[0], middle[1]);
        assert_eq!(synapse_at(model(&app), view, middle), Some(0));
        assert!(neuron_at(model(&app), view, middle).is_none());
        let frame = frame_of(&app);
        assert!(
            texts(&frame).contains(&weight),
            "{:?} for {weight}",
            texts(&frame)
        );
    }
}

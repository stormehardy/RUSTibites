//! The Bibite Editor's Brain tab screen (`BibiteBrainEditor` and its panels, drawn from the prefab): the toolbar (step, play, show values,
//! keep positions, sort, help), the canvas (a `ScrollRect` of connections and nodes that pans with the right button and zooms with the
//! wheel), its scroll bars, the panels that edit a node or a connection, the add-node panel and the help.
//!
//! Positions are the 1920 x 1080 reference's, read from the prefab (the toolbar's 14 padding, 10 spacing and 50 pt buttons; the scroll
//! view inset by 16; the edit panels' 10 padding and 10 spacing; the settings sliders' 5 / 3 inset and 20 pt handle) and checked against
//! the blurred reference screenshot. A node is the prefab's 35 x 35 at scale 1.286, a connection a strip of the `synapse_0` tile as thick
//! as its weight makes it.

use std::sync::{Arc, Mutex};

use bibites_brainsort::Archetype;
use bibites_core::brain::{N_INPUTS, NodeType};
use bibites_core::math::Vec2;

use crate::app::{App, FrameInput, HoverPanel};
use crate::brain_editor::{
    BASE_NODES, BrainEdit, EdgeKey, EditEdge, EditNode, MAX_ZOOM, MIN_ZOOM, NodeKey, Selected,
    base_name,
};
use crate::dyn_panel::FieldRef;
use crate::node_info_data::{self, NodeInfo};
use crate::species_panel::Drawer;
use crate::species_panel::brain::{
    HIGHLIGHT, ICON, NODE, SIGN, Strip, icon_name, neuron_colour, pattern, rasterise,
    synapse_colour, thickness,
};
use crate::template_editor_ui::{EditorAction, EditorSession, LIST, Tab};
use crate::tooltip::{Tip, TipRect};
use crate::ui::{DropdownTarget, Rgba, UiAction, UiDraw, UiFrame, UiId, UiTexture};
use crate::value_edit::{Share, distribute};

/// What a control of the Brain tab does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BrainAction {
    /// `StepButton` (`OneBrainStep`).
    Step,
    /// `PlayButton` (`PlayBrain(true)`) and `PauseButton` (`PlayBrain(false)`).
    Play(bool),
    /// The eye toggle (`UserSettings.ShowNodeValues`).
    ToggleValues,
    /// `SaveAnchors` (`SetNodePositionSaveMode(true)`).
    SaveAnchors,
    /// `ResortButton` (`ReSortBrain`).
    Resort,
    /// `InfoButton` (`OnHoverShow.ShowElement`): the help shows at once.
    ShowInfo,
    /// The canvas: a press starts a node drag, a click or a pan (`editor_brain_press`).
    Canvas,
    /// A scroll bar of the canvas (`true`: the horizontal one).
    Bar(bool),
    /// The add-node panel's scroll bar.
    AddBar,
    /// The node panel's "add connection" buttons (`NodeEditPanel.AddConnection(from)`: `true` starts at this node).
    AddConnection(bool),
    /// The delete buttons (`RemoveNode`, `RemoveConnection`).
    DeleteSelected,
    /// The connection panel's "add node" buttons (`SplitWithNode(remainIngoing)`).
    Split(bool),
    /// The connection panel's "Enabled" toggle.
    ToggleEnabled,
    /// A slider of an edit panel pressed (which, the track's ends in reference px).
    Slide(Slider, u16, u16),
    /// The node panel's label field.
    FocusLabel,
    /// The add-node panel's filter field.
    FocusFilter,
    /// An `ExpandCloseButton` of the add-node panel (0 inputs, 1 outputs, 2 hidden).
    ToggleSection(u8),
    /// An item of the add-node panel (`AddNode`): the section (0 input, 1 output, 2 hidden) and the item's index in its list (the
    /// output's number, or the hidden node's `NodeType`).
    AddNode(u8, u16),
    /// Part of a panel: it takes the press.
    Consume,
}

/// The sliders of the edit panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slider {
    /// `biasSlider`.
    Bias,
    /// `actSlider` (an input node's value for the simulator).
    Act,
    /// The connection's weight.
    Weight,
}

/// What a held press of the pointer is doing in the Brain tab.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BrainDrag {
    /// A node is pressed: it follows the cursor once the press has moved 10 px (`NodeEditor.OnDrag`), else it is clicked.
    Node {
        /// The node.
        key: NodeKey,
        /// Where the press began (window px).
        start: [f32; 2],
        /// The press has moved far enough to be a drag.
        moved: bool,
    },
    /// A connection is pressed: a click selects it.
    Edge {
        /// The connection.
        key: EdgeKey,
        /// Where the press began (window px).
        start: [f32; 2],
        /// The press moved too far to be a click.
        moved: bool,
    },
    /// The right button pans the canvas (`BrainEditorScroll.OnDrag`, which begins after 10 px).
    Pan {
        /// Where the press began (window px).
        start: [f32; 2],
        /// Once the drag has begun (`OnBeginDrag`): where the cursor was then (window px) and where the canvas was (reference px).
        /// The canvas is always that place plus how far the cursor has gone since, kept inside its limits: not a step on from where
        /// it is, so a drag past an end has to come back past the point it left the canvas at before the canvas moves again.
        from: Option<[f32; 4]>,
    },
    /// A slider of an edit panel is held.
    Slider(Slider, u16, u16),
    /// A scroll bar's handle is held (`true`: the horizontal one), at this distance from the handle's start.
    Bar(bool, f32),
    /// The add-node panel's scroll bar handle is held, at this distance from the handle's top.
    AddBar(f32),
}

/// The Brain tab's screen state: the view of the canvas and the panels (what is edited is the [`BrainEdit`]).
#[derive(Clone, Debug)]
pub struct BrainView {
    /// `currentScale`.
    pub zoom: f32,
    /// `brainRT.anchoredPosition`: how far the canvas is moved from the middle of the viewport (`y` up).
    pub pan: [f32; 2],
    /// Where the open panel hangs from, in the editor's own space (reference px from its bottom left, `y` up).
    pub panel_at: [f32; 2],
    /// The add-node panel's filter.
    pub filter: String,
    /// The filter field has the focus.
    pub filter_focus: bool,
    /// The node panel's label field has the focus.
    pub label_focus: bool,
    /// The node whose label field took the focus, and its text then (`TMP_InputField.m_OriginalText`: Escape puts it back).
    pub label_original: Option<(NodeKey, String)>,
    /// The sections of the add-node panel that are open (inputs, outputs, hidden).
    pub sections: [bool; 3],
    /// How far the add-node panel's list is scrolled.
    pub add_scroll: f32,
    /// The help (`OnHoverShow` on the info button, kept by the panel).
    pub info: HoverPanel,
    /// How far the help is scrolled.
    pub info_scroll: f32,
    /// The press that is being held.
    pub drag: Option<BrainDrag>,
    /// The right button was down last frame (a press is an edge).
    pub secondary_down: bool,
    /// Shift is held (`Input.GetKey(304 / 303)` at a node's click).
    pub shift: bool,
    /// Seconds since the last key, for the caret.
    pub idle: f32,
}

impl Default for BrainView {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: [0.0, 0.0],
            panel_at: [0.0, 0.0],
            filter: String::new(),
            filter_focus: false,
            label_focus: false,
            label_original: None,
            sections: [true, true, true],
            add_scroll: 0.0,
            info: HoverPanel::default(),
            info_scroll: 0.0,
            drag: None,
            secondary_down: false,
            shift: false,
            idle: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------------------- the geometry

const WHITE: Rgba = [255, 255, 255, 255];
/// The panels' labels (`TextMeshProUGUI` of `Name`, `Label`: (42, 42, 42)) and what is typed into their fields ((50, 50, 50)).
const LABEL: Rgba = [42, 42, 42, 255];
const FIELD: Rgba = [50, 50, 50, 255];
/// The small notes of the node panel (the "can't change" line (81, 81, 81), the non-linear warning (178, 65, 0)).
const NOTE: Rgba = [81, 81, 81, 255];
const WARNING: Rgba = [178, 65, 0, 255];
/// The tab's prefab path, for the tooltips of its controls.
const TIP_BASE: &str =
    "UICanvas/MainUI/Layer2/BibiteSelector/BibiteEditor/Panel/Holder/Panels/BrainTab/";

/// The toolbar (`BrainToolbar`): the tab's top 78.
pub const TOOLBAR: [f32; 4] = [LIST[0], LIST[1], LIST[2], 78.0];
/// The brain editor (`BrainEditor`): the rest of the tab, the panels' space (their pivots count from its bottom left).
pub const EDITOR: [f32; 4] = [LIST[0], LIST[1] + 78.0, LIST[2], LIST[3] - 78.0];
/// The scroll view (inset 16 on the sides and at the bottom).
const SCROLL_VIEW: [f32; 4] = [EDITOR[0] + 16.0, EDITOR[1], EDITOR[2] - 32.0, EDITOR[3] - 16.0];
/// A scroll bar's thickness (`Scrollbar` 12), which is also its sliding area's inset.
const BAR: f32 = 12.0;
/// A toolbar button.
const BUTTON: f32 = 50.0;
const BUTTON_Y: f32 = TOOLBAR[1] + 14.0;
/// The notice over the preview: where its middle is.
const NOTICE_Y: f32 = 101.0;
/// Hit ids of the Brain tab.
const HIT_CANVAS: UiId = 48_000;
const HIT_TOOLBAR: UiId = 48_001;
const HIT_BARS: UiId = 48_010;
const HIT_PANEL: UiId = 48_020;
const HIT_BUTTONS: UiId = 48_021;
const HIT_SLIDERS: UiId = 48_030;
const HIT_LABEL: UiId = 48_040;
const HIT_FUNCTION: UiId = 48_041;
const HIT_ENABLED: UiId = 48_042;
const HIT_FILTER: UiId = 48_050;
const HIT_SECTIONS: UiId = 48_051;
const HIT_ADD_BAR: UiId = 48_055;
const HIT_INFO: UiId = 48_090;
const HIT_ITEMS: UiId = 48_100;

/// The toolbar's buttons' left edges: the step and the play (or pause) button on the left; on the right the eye, the keep-positions
/// button (gone once positions are kept: the others stay put and the eye moves 100 right), the sort button and the help.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Toolbar {
    /// `StepButton`.
    pub step: f32,
    /// `PlayButton` / `PauseButton`.
    pub play: f32,
    /// The `Visibility` toggle.
    pub eye: f32,
    /// `SaveAnchors`, while the positions are not kept.
    pub keep: Option<f32>,
    /// `ResortButton`.
    pub sort: f32,
    /// `InfoButton`.
    pub info: f32,
}

/// The toolbar for a canvas whose positions are (`keeping`) or are not saved.
#[must_use]
pub fn toolbar_layout(keeping: bool) -> Toolbar {
    let right = TOOLBAR[0] + TOOLBAR[2] - 14.0;
    let info = right - BUTTON;
    let sort = info - 100.0;
    let keep = (!keeping).then_some(sort - 100.0);
    let eye = keep.unwrap_or(sort) - 100.0;
    Toolbar {
        step: TOOLBAR[0] + 14.0,
        play: TOOLBAR[0] + 74.0,
        eye,
        keep,
        sort,
        info,
    }
}

/// The canvas: where the content is on the screen, and the viewport it shows through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Canvas {
    /// `Viewport`: what the content shows through (reference px).
    pub viewport: [f32; 4],
    /// A horizontal and a vertical bar show (the scroll view hides them when the content fits: `AutoHideAndExpandViewport`).
    pub bars: (bool, bool),
    /// The content's size (`brainRT.sizeDelta`).
    pub content: [f32; 2],
    /// `currentScale`.
    pub zoom: f32,
    /// `brainRT.anchoredPosition`, kept inside what the content allows.
    pub pan: [f32; 2],
}

impl Canvas {
    /// The canvas of a brain at a zoom and pan.
    #[must_use]
    pub fn new(content: [f32; 2], zoom: f32, pan: [f32; 2]) -> Self {
        let full = [SCROLL_VIEW[2], SCROLL_VIEW[3]];
        let (width, height) = (content[0] * zoom, content[1] * zoom);
        // A bar that shows takes 12 of the other direction, which may then need its bar too.
        let mut bars = (width > full[0], height > full[1]);
        bars.0 = width > full[0] - if bars.1 { BAR } else { 0.0 };
        bars.1 = height > full[1] - if bars.0 { BAR } else { 0.0 };
        let viewport = [
            SCROLL_VIEW[0],
            SCROLL_VIEW[1],
            full[0] - if bars.1 { BAR } else { 0.0 },
            full[1] - if bars.0 { BAR } else { 0.0 },
        ];
        let mut canvas = Self {
            viewport,
            bars,
            content,
            zoom,
            pan,
        };
        canvas.pan = canvas.clamped(pan);
        canvas
    }

    /// How far the content may be moved either way (`ScrollRect` clamped: the content never leaves a gap, and stays in the middle
    /// when it is smaller than the viewport).
    #[must_use]
    pub fn reach(&self) -> [f32; 2] {
        [
            ((self.content[0] * self.zoom - self.viewport[2]) / 2.0).max(0.0),
            ((self.content[1] * self.zoom - self.viewport[3]) / 2.0).max(0.0),
        ]
    }

    /// `pan` kept inside what the content allows.
    #[must_use]
    pub fn clamped(&self, pan: [f32; 2]) -> [f32; 2] {
        let reach = self.reach();
        [
            pan[0].clamp(-reach[0], reach[0]),
            pan[1].clamp(-reach[1], reach[1]),
        ]
    }

    /// The content's middle on the screen.
    #[must_use]
    pub fn center(&self) -> [f32; 2] {
        [
            self.viewport[0] + self.viewport[2] / 2.0 + self.pan[0],
            self.viewport[1] + self.viewport[3] / 2.0 - self.pan[1],
        ]
    }

    /// Where a node's place is on the screen (`y` of the place is negative downwards).
    #[must_use]
    pub fn to_screen(self, place: [f32; 2]) -> [f32; 2] {
        let center = self.center();
        [
            center[0] + (place[0] - self.content[0] / 2.0) * self.zoom,
            center[1] + (-place[1] - self.content[1] / 2.0) * self.zoom,
        ]
    }

    /// `MouseToNodePos`: the place a point of the screen is, in a node's terms.
    #[must_use]
    pub fn to_place(self, screen: [f32; 2]) -> [f32; 2] {
        let center = self.center();
        [
            (screen[0] - center[0]) / self.zoom + self.content[0] / 2.0,
            -((screen[1] - center[1]) / self.zoom + self.content[1] / 2.0),
        ]
    }

    /// `Zoom`: the zoom after `increment` (a part of the scale), and the pan that keeps the place under `cursor` where it was.
    #[must_use]
    pub fn zoomed(&self, increment: f32, cursor: [f32; 2]) -> (f32, [f32; 2]) {
        let center = self.center();
        // `MousePosInGraph`: the cursor in the content's own units, `y` up.
        let under = [
            (cursor[0] - center[0]) / self.zoom,
            -(cursor[1] - center[1]) / self.zoom,
        ];
        let change = (increment * self.zoom).clamp(MIN_ZOOM - self.zoom, MAX_ZOOM - self.zoom);
        let pan = [
            self.pan[0] - under[0] * change,
            self.pan[1] - under[1] * change,
        ];
        (self.zoom + change, pan)
    }

    /// Whether the point is over the viewport.
    #[must_use]
    pub fn contains(&self, point: [f32; 2]) -> bool {
        inside(point, self.viewport)
    }

    /// A scroll bar's track and handle (`true`: the horizontal bar). The handle is `size` of the sliding area (the track less 12) plus
    /// its 12 of overhang; its place is the scroll position.
    #[must_use]
    pub fn bar(&self, horizontal: bool) -> ([f32; 4], [f32; 4]) {
        let reach = self.reach();
        if horizontal {
            let track = [
                SCROLL_VIEW[0],
                SCROLL_VIEW[1] + SCROLL_VIEW[3] - BAR,
                SCROLL_VIEW[2] - BAR,
                BAR,
            ];
            let size = (self.viewport[2] / (self.content[0] * self.zoom)).min(1.0);
            let area = track[2] - BAR;
            // The content moves right when `pan.x` grows, which shows its left: the handle goes left.
            let along = if reach[0] > 0.0 {
                (reach[0] - self.pan[0]) / (2.0 * reach[0])
            } else {
                0.0
            };
            let handle = [
                track[0] + along * (1.0 - size) * area,
                track[1],
                size * area + BAR,
                BAR,
            ];
            (track, handle)
        } else {
            let track = [
                SCROLL_VIEW[0] + SCROLL_VIEW[2] - BAR,
                SCROLL_VIEW[1],
                BAR,
                SCROLL_VIEW[3] - BAR,
            ];
            let size = (self.viewport[3] / (self.content[1] * self.zoom)).min(1.0);
            let area = track[3] - BAR;
            // The content moves up when `pan.y` grows, which shows its bottom: the handle goes down.
            let along = if reach[1] > 0.0 {
                (reach[1] + self.pan[1]) / (2.0 * reach[1])
            } else {
                0.0
            };
            let handle = [
                track[0],
                track[1] + along * (1.0 - size) * area,
                BAR,
                size * area + BAR,
            ];
            (track, handle)
        }
    }

    /// The pan that puts a scroll bar's handle at `along` (0 at the start of its travel, 1 at the end) of it.
    #[must_use]
    pub fn pan_for_bar(&self, horizontal: bool, along: f32) -> [f32; 2] {
        let reach = self.reach();
        let along = along.clamp(0.0, 1.0);
        if horizontal {
            [reach[0] - 2.0 * reach[0] * along, self.pan[1]]
        } else {
            [self.pan[0], -reach[1] + 2.0 * reach[1] * along]
        }
    }
}

/// Whether `point` is in `rect`.
fn inside(point: [f32; 2], rect: [f32; 4]) -> bool {
    point[0] >= rect[0]
        && point[0] < rect[0] + rect[2]
        && point[1] >= rect[1]
        && point[1] < rect[1] + rect[3]
}

/// The node or connection under a point of the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Picked {
    /// A node.
    Node(NodeKey),
    /// A connection.
    Edge(EdgeKey),
}

/// What is under `screen`: a node first (the later ones over the earlier ones), else a connection whose strip the point is on.
#[must_use]
pub fn pick(edit: &BrainEdit, canvas: &Canvas, screen: [f32; 2]) -> Option<Picked> {
    if !canvas.contains(screen) {
        return None;
    }
    let radius = NODE / 2.0 * canvas.zoom;
    for node in edit.nodes.iter().rev() {
        let at = canvas.to_screen(node.pos);
        let (dx, dy) = (screen[0] - at[0], screen[1] - at[1]);
        if dx * dx + dy * dy <= radius * radius {
            return Some(Picked::Node(node.key));
        }
    }
    for edge in edit.edges.iter().rev() {
        let Some((from, to)) = edit.ends_of(edge.key) else {
            continue;
        };
        let (a, b) = (canvas.to_screen(from.pos), canvas.to_screen(to.pos));
        let reach = (thickness(edge.weight) * 0.5 * canvas.zoom).max(3.0);
        if distance_to_segment(screen, a, b) <= reach {
            return Some(Picked::Edge(edge.key));
        }
    }
    None
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

// ---------------------------------------------------------------------------------------- the panels' geometry

/// The width of the node panel (10 padding, the 140 label, 20 spacing, the 200 field, 10 padding).
const NODE_PANEL_WIDTH: f32 = 380.0;
/// The height of a label row (the field's 18.76 text and its 6 of margin), of a function dropdown and of a slider row.
const LABEL_ROW: f32 = 24.76;
const FUNCTION_ROW: f32 = 25.0;
const SLIDER_ROW: f32 = 26.0;
/// The small notes' heights: one line of 10 pt, and two.
const NOTE_ONE: f32 = 11.72;
const NOTE_TWO: f32 = 23.44;

/// The size of the node panel for this node (`NodeEditPanel`: 10 padding and spacing, the buttons' row 30 high, the label 24.76, the
/// function 25 with its notes, the sliders 26).
#[must_use]
pub fn node_panel_size(node: &EditNode) -> [f32; 2] {
    let hidden = node.archetype() == Archetype::Hidden;
    let mut function = FUNCTION_ROW;
    if !hidden {
        function += 5.0 + NOTE_ONE;
    }
    if node.kind.is_non_linear() {
        function += 5.0 + NOTE_TWO;
    }
    let mut handles = LABEL_ROW + 10.0 + function;
    if node.kind.has_bias() {
        handles += 10.0 + SLIDER_ROW;
    }
    if node.kind == NodeType::Input {
        handles += 10.0 + SLIDER_ROW;
    }
    [NODE_PANEL_WIDTH, 10.0 + 30.0 + 10.0 + handles + 10.0]
}

/// The size of the connection panel (buttons, weight, toggle: 5 + 35 + 10 + 30 + 10 + 30 + 10 = 130).
pub const EDGE_PANEL: [f32; 2] = [298.77, 130.0];
/// The size of the add-node panel.
pub const ADD_PANEL: [f32; 2] = [200.0, 250.0];

/// Where a panel of `size` is on the screen when it hangs from `at` (the editor's own space, from its bottom left): an edit panel's
/// pivot goes from 75 right of the point at the editor's left to 75 left of it at the right, and 75 below it in the upper half, above
/// it in the lower; the add-node panel's the other way round (`SelectElement`, `RequestAddNode`).
#[must_use]
pub fn panel_rect(size: [f32; 2], at: [f32; 2], add_node: bool) -> [f32; 4] {
    let (width, height) = (size[0], size[1]);
    let along = at[0] / EDITOR[2];
    let across = at[1] / EDITOR[3];
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let pivot = if add_node {
        [
            if along > 0.5 {
                1.0 + 75.0 / width
            } else {
                -75.0 / width
            },
            lerp(-75.0, height + 75.0, across) / height,
        ]
    } else {
        [
            lerp(-75.0, width + 75.0, along) / width,
            if across > 0.5 {
                1.0 + 75.0 / height
            } else {
                -75.0 / height
            },
        ]
    };
    let left = at[0] - pivot[0] * width;
    let bottom = at[1] - pivot[1] * height;
    [
        EDITOR[0] + left,
        EDITOR[1] + EDITOR[3] - (bottom + height),
        width,
        height,
    ]
}

/// A point of the screen in the editor's own space (from its bottom left, `y` up).
#[must_use]
pub fn to_editor_space(screen: [f32; 2]) -> [f32; 2] {
    [screen[0] - EDITOR[0], EDITOR[1] + EDITOR[3] - screen[1]]
}

/// The rectangle of the open edit or add panel, if there is one.
#[must_use]
pub fn open_panel_rect(edit: &BrainEdit, view: &BrainView) -> Option<[f32; 4]> {
    if edit.add_mode {
        return Some(panel_rect(ADD_PANEL, view.panel_at, true));
    }
    match edit.selected? {
        Selected::Node(key) => Some(panel_rect(
            node_panel_size(edit.node(key)?),
            view.panel_at,
            false,
        )),
        Selected::Edge(_) => Some(panel_rect(EDGE_PANEL, view.panel_at, false)),
    }
}

/// The slider ranges of the panels: `(minimum, maximum)` of the track, and whether a typed value may leave it
/// (`CheckTypeForBiasSlider`, `SelectNode`).
#[must_use]
pub fn slider_range(slider: Slider, node: Option<&EditNode>) -> (f32, f32, bool) {
    match slider {
        Slider::Weight => (-10.0, 10.0, true),
        Slider::Bias => match node.map(|node| node.kind) {
            Some(NodeType::Inhibitory) => (0.0001, 10.0, false),
            Some(NodeType::SoftLatch) => (0.0001, 1000.0, false),
            _ => (-5.0, 5.0, true),
        },
        Slider::Act => {
            let info =
                node.and_then(|node| node_info_data::of_node(node.index, node.kind as usize));
            let (low, high) = info.map_or((-5.0, 5.0), |info| {
                (info.min_output.max(-5.0), info.max_output.min(5.0))
            });
            (low, high, true)
        }
    }
}

/// The value of a slider at `cursor_x` on the track `x0..x1`.
#[must_use]
pub fn slider_value(
    slider: Slider,
    node: Option<&EditNode>,
    x0: f32,
    x1: f32,
    cursor_x: f32,
) -> f32 {
    let (low, high, _) = slider_range(slider, node);
    if x1 <= x0 {
        return low;
    }
    let along = ((cursor_x - x0) / (x1 - x0)).clamp(0.0, 1.0);
    low + (high - low) * along
}

/// Where a value sits on a slider's track, `0..=1`.
fn fraction(slider: Slider, node: Option<&EditNode>, value: f32) -> f32 {
    let (low, high, _) = slider_range(slider, node);
    if high > low {
        ((value - low) / (high - low)).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// `NodeTypeSelection.nodeTypeChoices`: the options of the activation function dropdown, in the game's order.
pub const FUNCTION_CHOICES: [(NodeType, &str); 13] = [
    (NodeType::Linear, "Linear"),
    (NodeType::Abs, "Abs"),
    (NodeType::ReLu, "ReLU"),
    (NodeType::Mult, "Multiplicative"),
    (NodeType::Sigmoid, "Sigmoid"),
    (NodeType::TanH, "TanH"),
    (NodeType::Sine, "Sinus"),
    (NodeType::Gaussian, "Gaussian"),
    (NodeType::Differential, "Differential"),
    (NodeType::Inhibitory, "Inhibitory"),
    (NodeType::Integrator, "Integrator"),
    (NodeType::Latch, "Latch"),
    (NodeType::SoftLatch, "Soft Latch"),
];

/// The name the activation function dropdown shows for a function (an input's `Input` is not a choice: its name as it is).
fn function_name(kind: NodeType) -> &'static str {
    FUNCTION_CHOICES
        .iter()
        .find(|(choice, _)| *choice == kind)
        .map_or_else(|| kind.name(), |(_, name)| *name)
}

/// The settings' help texts, shown on the names of the panel's rows (`SettingSlider.UpdateText`).
const BIAS_HELP: &str = "The node's default activation. \nIt behaves as a constant activation of the node and determines the baseline input aside from any other incoming connection.\n\nSome nodes activation functions, like the Inhibitor function, use the bias differently, but most just add it to their total activation.";
const ACT_HELP: &str = "The input node's value\nThis is just for the simulator, and allows you to test out how the network will react to given senses.";
const LABEL_HELP: &str = "Identifies the node\nCan only be modified for hidden nodes";
const FUNCTION_HELP: &str = "A node's output is equal the sum of it's inputs and bias, passed trough the activation function.\n\nNon-linear nodes might have different use for biases";
const WEIGHT_HELP: &str = "The connection's strength is used when propagating information.\nIt's multiplied to the input node's value before propagating it to the output's node activation.";
const ENABLED_HELP: &str = "A disabled connection will not propagate its input.\nThe use is that re-enabling it is a likelier mutation then re-evolving the connection completely.";

/// A setting's help as a tooltip on its name.
fn help(title: &'static str, content: &'static str) -> Tip {
    Tip {
        path: "",
        title,
        content,
        ui: true,
        instant: false,
    }
}

/// `NodeInformation.tooltipText`.
fn tooltip_text(info: Option<&'static NodeInfo>) -> String {
    let Some(info) = info else {
        return String::new();
    };
    format!(
        "{}{}\n\n{}",
        if info.is_linear {
            ""
        } else {
            "!NONLINEAR! the same input won't always result in the same output\n\n"
        },
        info.desc,
        info.range_text()
    )
}

/// `NodeEditor.UpdateTooltip`: the node's name over what the game says about its function, its range and its bias; `None` for a node
/// without a name (the game disables its trigger).
#[must_use]
pub fn node_tip(node: &EditNode) -> Option<Tip> {
    if node.desc.is_empty() {
        return None;
    }
    let archetype = node.archetype();
    let mut text = if archetype == Archetype::Hidden {
        format!(
            "Function: {}\n{}",
            node.kind.name(),
            tooltip_text(node_info_data::of_function(node.kind as usize))
        )
    } else {
        tooltip_text(node_info_data::of_node(node.index, node.kind as usize))
    };
    if archetype != Archetype::Input {
        text.push_str(&format!("\n\nDefault Activation: {:.4}", node.bias));
        if archetype == Archetype::Output {
            text.push_str(&format!("\nFunction: {}", node.kind.name()));
        }
    }
    Some(Tip {
        path: "",
        title: crate::tooltip::intern(node.desc.clone()),
        content: crate::tooltip::intern(text),
        ui: true,
        instant: false,
    })
}

// ---------------------------------------------------------------------------------------- the add-node panel's lists

/// What the add-node panel lists in one of its three sections.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    /// The header's title.
    pub title: &'static str,
    /// The items that match the filter: the index `AddNode` takes and the item's name. Their number is `nInputs`, `nOutputs` or
    /// `nHidden`: counted even while the section is not offered.
    pub items: Vec<(u16, String)>,
    /// The section shows (it has items, and is not the kind a waiting connection cannot take).
    pub shown: bool,
}

/// The add-node panel's three sections for a filter (`AddNodePanel.Filter`, `AddNodeItem.Filter`): the inputs that are not on the canvas,
/// the outputs that are not, and the functions a hidden node can have (`NodeType` order, enum names), each the ones whose name has the
/// filter in it, whatever its case.
#[must_use]
pub fn add_sections(edit: &BrainEdit, filter: &str) -> [Section; 3] {
    let wanted =
        |name: &str| filter.is_empty() || name.to_lowercase().contains(&filter.to_lowercase());
    let inputs: Vec<(u16, String)> = edit
        .available(Archetype::Input)
        .into_iter()
        .map(|index| (index as u16, base_name(index)))
        .filter(|(_, name)| wanted(name))
        .collect();
    let outputs: Vec<(u16, String)> = edit
        .available(Archetype::Output)
        .into_iter()
        .map(|index| ((index - N_INPUTS) as u16, base_name(index)))
        .filter(|(_, name)| wanted(name))
        .collect();
    let hidden: Vec<(u16, String)> = (1..NodeType::COUNT)
        .filter_map(|number| NodeType::from_u8(number as u8))
        .map(|kind| (kind as u16, kind.name().to_owned()))
        .filter(|(_, name)| wanted(name))
        .collect();
    [
        Section {
            title: "input nodes",
            shown: !inputs.is_empty() && edit.add_hides != Archetype::Input,
            items: inputs,
        },
        Section {
            title: "output nodes",
            shown: !outputs.is_empty() && edit.add_hides != Archetype::Output,
            items: outputs,
        },
        // Nothing hides the hidden nodes: `unavailableArchetype` is never `Hidden` for a waiting connection.
        Section {
            title: "hidden nodes",
            shown: !hidden.is_empty(),
            items: hidden,
        },
    ]
}

/// The height of a section header and of an item (`Button`: the 18.76 text, 1 of padding above it and 2 below).
const ROW: f32 = 21.76;
/// The add-node panel's filter field.
const FILTER_H: f32 = 31.76;

/// The list's viewport and its scroll bar track in the add-node panel at `panel`.
fn add_viewport(panel: [f32; 4]) -> ([f32; 4], [f32; 4]) {
    let view = [
        panel[0] + 3.0,
        panel[1] + FILTER_H,
        184.0,
        panel[3] - FILTER_H - 3.0,
    ];
    let bar = [
        panel[0] + 187.0,
        panel[1] + FILTER_H - 1.0,
        BAR,
        panel[3] - FILTER_H + 1.0,
    ];
    (view, bar)
}

/// The height of the add-node panel's list: the headers of the sections that show and the items under the open ones.
fn add_content_height(sections: &[Section; 3], open: [bool; 3]) -> f32 {
    sections
        .iter()
        .zip(open)
        .filter(|(section, _)| section.shown)
        .map(|(section, open)| {
            ROW + if open {
                ROW * section.items.len() as f32
            } else {
                0.0
            }
        })
        .sum()
}

// ---------------------------------------------------------------------------------------- the drawing

/// `ConnectionEditor.UpdateColor`: green for a positive weight, red for a negative one, and a quarter as strong when it is disabled.
fn strip_tint(edge: &EditEdge) -> Rgba {
    let tint = synapse_colour(edge.weight, 1.0, true);
    if edge.enabled {
        tint
    } else {
        [tint[0], tint[1], tint[2], 64]
    }
}

/// Text whose line is centred `centre` px down, from `x` (reference px, `size` in reference px).
fn put(
    frame: &mut UiFrame,
    d: &Drawer<'_>,
    text: &str,
    x: f32,
    centre: f32,
    size: f32,
    colour: Rgba,
) {
    let px = size * d.s;
    frame.text(
        text.to_owned(),
        Vec2::new(x * d.s, centre * d.s - 0.6 * px),
        px,
        colour,
    );
}

/// Text centred on `x` and `centre`.
fn put_centred(
    frame: &mut UiFrame,
    d: &Drawer<'_>,
    text: &str,
    x: f32,
    centre: f32,
    size: f32,
    colour: Rgba,
) {
    let width = d.measure(text, size * d.s, false) / d.s;
    put(frame, d, text, x - width / 2.0, centre, size, colour);
}

/// Text ending at `x`.
fn put_right(
    frame: &mut UiFrame,
    d: &Drawer<'_>,
    text: &str,
    x: f32,
    centre: f32,
    size: f32,
    colour: Rgba,
) {
    let width = d.measure(text, size * d.s, false) / d.s;
    put(frame, d, text, x - width, centre, size, colour);
}

/// A sprite cut to a clip rectangle (the renderers have no clip): the part of `rect` inside `clip`.
fn clipped_icon(
    frame: &mut UiFrame,
    d: &Drawer<'_>,
    sprite: &str,
    rect: [f32; 4],
    tint: Rgba,
    clip: [f32; 4],
) {
    let Some(texture) = d.ui.texture_of_alias(sprite) else {
        return;
    };
    let Some(size) = d
        .ui
        .texture(texture)
        .map(|texture| (texture.width as f32, texture.height as f32))
    else {
        return;
    };
    let x0 = rect[0].max(clip[0]);
    let y0 = rect[1].max(clip[1]);
    let x1 = (rect[0] + rect[2]).min(clip[0] + clip[2]);
    let y1 = (rect[1] + rect[3]).min(clip[1] + clip[3]);
    if x1 <= x0 || y1 <= y0 || rect[2] <= 0.0 || rect[3] <= 0.0 {
        return;
    }
    if x0 == rect[0] && y0 == rect[1] && x1 == rect[0] + rect[2] && y1 == rect[1] + rect[3] {
        d.icon(frame, sprite, rect, tint);
        return;
    }
    frame.draws.push(UiDraw::ImageRegion {
        texture,
        region: [
            (x0 - rect[0]) / rect[2] * size.0,
            (y0 - rect[1]) / rect[3] * size.1,
            (x1 - x0) / rect[2] * size.0,
            (y1 - y0) / rect[3] * size.1,
        ],
        pos: d.at(x0, y0),
        size: d.at(x1 - x0, y1 - y0),
        tint,
    });
}

/// A filled circle of one colour with a soft edge, drawn a few pixels bigger than a node behind it: the ring `NodeEditor.SetHighlight`
/// (selected) and `RefreshValidity` (invalid) make the node's shader draw. One texture per colour is kept.
fn halo_texture(colour: Rgba) -> Arc<UiTexture> {
    static HALOS: Mutex<Vec<(Rgba, Arc<UiTexture>)>> = Mutex::new(Vec::new());
    const SIDE: u32 = 64;
    let make = || {
        let mut rgba = Vec::with_capacity((SIDE * SIDE * 4) as usize);
        for y in 0..SIDE {
            for x in 0..SIDE {
                let dx = (x as f32 + 0.5) / SIDE as f32 * 2.0 - 1.0;
                let dy = (y as f32 + 0.5) / SIDE as f32 * 2.0 - 1.0;
                let inside = ((1.0 - dx.hypot(dy)) * SIDE as f32 / 2.0).clamp(0.0, 1.0);
                rgba.extend_from_slice(&[
                    colour[0],
                    colour[1],
                    colour[2],
                    (inside * f32::from(colour[3])) as u8,
                ]);
            }
        }
        Arc::new(UiTexture {
            width: SIDE,
            height: SIDE,
            rgba,
        })
    };
    let Ok(mut halos) = HALOS.lock() else {
        return make();
    };
    if let Some((_, texture)) = halos.iter().find(|(kept, _)| *kept == colour) {
        return Arc::clone(texture);
    }
    let texture = make();
    halos.push((colour, Arc::clone(&texture)));
    texture
}

/// A raw-pixel picture stretched over `rect` and cut to a clip rectangle.
fn clipped_pixels(
    frame: &mut UiFrame,
    d: &Drawer<'_>,
    texture: &Arc<UiTexture>,
    rect: [f32; 4],
    clip: [f32; 4],
) {
    let x0 = rect[0].max(clip[0]);
    let y0 = rect[1].max(clip[1]);
    let x1 = (rect[0] + rect[2]).min(clip[0] + clip[2]);
    let y1 = (rect[1] + rect[3]).min(clip[1] + clip[3]);
    if x1 <= x0 || y1 <= y0 || rect[2] <= 0.0 || rect[3] <= 0.0 {
        return;
    }
    if x0 == rect[0] && y0 == rect[1] && x1 == rect[0] + rect[2] && y1 == rect[1] + rect[3] {
        frame.draws.push(UiDraw::Pixels {
            texture: Arc::clone(texture),
            pos: d.at(rect[0], rect[1]),
            size: d.at(rect[2], rect[3]),
        });
        return;
    }
    let (width, height) = (texture.width as f32, texture.height as f32);
    frame.draws.push(UiDraw::PixelsRegion {
        texture: Arc::clone(texture),
        region: [
            (x0 - rect[0]) / rect[2] * width,
            (y0 - rect[1]) / rect[3] * height,
            (x1 - x0) / rect[2] * width,
            (y1 - y0) / rect[3] * height,
        ],
        pos: d.at(x0, y0),
        size: d.at(x1 - x0, y1 - y0),
    });
}

/// The connections' picture, kept while nothing about them changes.
struct Cached {
    key: u64,
    draw: UiDraw,
}

static CONNECTIONS: Mutex<Option<Cached>> = Mutex::new(None);

/// A hash of everything the picture of the connections depends on.
fn picture_key(strips: &[Strip], viewport: [f32; 4], scale: f32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for strip in strips {
        for value in [
            strip.from[0],
            strip.from[1],
            strip.to[0],
            strip.to[1],
            strip.thickness,
            strip.halo,
        ] {
            value.to_bits().hash(&mut hasher);
        }
        strip.tint.hash(&mut hasher);
    }
    for value in viewport.into_iter().chain([scale]) {
        value.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

/// Draws the Brain tab.
pub fn draw(frame: &mut UiFrame, d: &Drawer<'_>, app: &App, session: &EditorSession) {
    let edit = &session.brain;
    let view = &session.view.brain;
    d.slice(frame, "gray_panel", 10.0, 1.0 / 0.5, LIST, WHITE);
    toolbar(frame, d, app, session);
    let canvas = Canvas::new(edit.content, view.zoom, view.pan);
    canvas_content(frame, d, app, edit, view, &canvas);
    scroll_bars(frame, d, &canvas);
    panels(frame, d, app, session);
    if view.info.open {
        info_panel(frame, d, view);
    }
}

/// A tooltip over `rect` for the control at `path` under the tab.
fn tip(frame: &mut UiFrame, d: &Drawer<'_>, rect: [f32; 4], path: &str) {
    if let Some(tip) = crate::tooltip::find(&format!("{TIP_BASE}{path}")) {
        frame.tips.push(TipRect {
            pos: d.at(rect[0], rect[1]),
            size: d.at(rect[2], rect[3]),
            tip,
        });
    }
}

/// The toolbar: the simulator's buttons on the left, the display and layout buttons on the right.
fn toolbar(frame: &mut UiFrame, d: &Drawer<'_>, app: &App, session: &EditorSession) {
    let edit = &session.brain;
    d.slice(frame, "gray_panel", 10.0, 1.0 / 0.5, TOOLBAR, WHITE);
    let layout = toolbar_layout(edit.save_anchors);
    let button = |frame: &mut UiFrame, x: f32, enabled: bool| -> [f32; 4] {
        let rect = [x, BUTTON_Y, BUTTON, BUTTON];
        let tint: Rgba = if enabled {
            WHITE
        } else {
            [140, 140, 140, 255]
        };
        d.slice(frame, "base_icon", 5.0, 1.0, rect, tint);
        rect
    };
    let hit = |frame: &mut UiFrame, slot: UiId, rect: [f32; 4], action: BrainAction| {
        frame.hit(
            HIT_TOOLBAR + slot,
            d.at(rect[0], rect[1]),
            d.at(rect[2], rect[3]),
            UiAction::Editor(EditorAction::Brain(action)),
        );
    };
    // Step, and Play or Pause (they take the same place: `PlayBrain` swaps them).
    let step = button(frame, layout.step, true);
    d.icon(frame, "step_icon_0", step, WHITE);
    hit(frame, 0, step, BrainAction::Step);
    tip(frame, d, step, "BrainToolbar/SimButtons/StepButton");
    let play = button(frame, layout.play, true);
    if edit.playing {
        d.icon(frame, "pause_icon", play, WHITE);
        hit(frame, 1, play, BrainAction::Play(false));
        tip(frame, d, play, "BrainToolbar/SimButtons/PauseButton");
    } else {
        d.icon(frame, "resume_icon", play, WHITE);
        hit(frame, 1, play, BrainAction::Play(true));
        tip(frame, d, play, "BrainToolbar/SimButtons/PlayButton");
    }
    // The eye (`SettingToggleTwoIcons`: the open eye while values show, the crossed one while they do not).
    let eye = button(frame, layout.eye, true);
    d.icon(
        frame,
        if app.prefs.show_node_values {
            "eye_icon_0"
        } else {
            "no_eye_icon_0"
        },
        eye,
        WHITE,
    );
    hit(frame, 2, eye, BrainAction::ToggleValues);
    tip(frame, d, eye, "BrainToolbar/UtilityButtons/Visibility");
    // Keep the positions: the brain at the bottom left, a pencil at the top right.
    if let Some(x) = layout.keep {
        let keep = button(frame, x, true);
        d.icon(
            frame,
            "brain_icon",
            [keep[0] + 2.0, keep[1] + 15.0, 35.0, 35.0],
            [89, 89, 89, 255],
        );
        d.icon(
            frame,
            "rename_icon",
            [keep[0] + 15.0, keep[1], 35.0, 35.0],
            [36, 36, 36, 255],
        );
        hit(frame, 3, keep, BrainAction::SaveAnchors);
        tip(frame, d, keep, "BrainToolbar/UtilityButtons/SaveAnchors");
    }
    // Sort: the brain inside a circle of arrows, greyed while the brain is not valid.
    let sort = button(frame, layout.sort, edit.is_valid());
    d.icon(
        frame,
        "reset_icon",
        [sort[0] - 5.0, sort[1] - 5.0, 60.0, 60.0],
        WHITE,
    );
    d.icon(
        frame,
        "brain_icon",
        [sort[0] + 5.0, sort[1] + 5.0, 40.0, 40.0],
        [89, 89, 89, 255],
    );
    if edit.is_valid() {
        hit(frame, 4, sort, BrainAction::Resort);
    }
    tip(frame, d, sort, "BrainToolbar/UtilityButtons/ResortButton");
    // Help: the panel shows while the pointer rests on it.
    let info = button(frame, layout.info, true);
    d.icon(frame, "help_icon", info, WHITE);
    hit(frame, 5, info, BrainAction::ShowInfo);
    tip(frame, d, info, "BrainToolbar/UtilityButtons/InfoButton");
}

/// The strips of the connections on the canvas, in the order they were made (later ones over earlier ones); a connection that waits
/// for its end follows the cursor (`ConnectionEditor.UpdatePosition`), or stays where it was while the add-node panel is open.
fn strips_of(edit: &BrainEdit, canvas: &Canvas, cursor: [f32; 2]) -> Vec<Strip> {
    let loose = if edit.add_mode {
        canvas.to_screen(edit.add_pos)
    } else {
        cursor
    };
    let mut strips = Vec::with_capacity(edit.edges.len());
    for edge in &edit.edges {
        let place = |end: Option<NodeKey>| {
            end.and_then(|key| edit.node(key))
                .map_or(loose, |node| canvas.to_screen(node.pos))
        };
        let (from, to) = (place(edge.from), place(edge.to));
        let selected = edit.selected == Some(Selected::Edge(edge.key));
        strips.push(Strip {
            // The picture's holder is the screen's top left, and its `y` goes up.
            from: [from[0], -from[1]],
            to: [to[0], -to[1]],
            thickness: thickness(edge.weight) * canvas.zoom,
            tint: strip_tint(edge),
            halo: if selected { 1.0 } else { 0.0 },
        });
    }
    strips
}

/// The canvas: the connections as one picture, the nodes over them.
fn canvas_content(
    frame: &mut UiFrame,
    d: &Drawer<'_>,
    app: &App,
    edit: &BrainEdit,
    view: &BrainView,
    canvas: &Canvas,
) {
    let s = d.s;
    let viewport = canvas.viewport;
    // The canvas takes the pointer (the window loop picks what is under it).
    frame.hit(
        HIT_CANVAS,
        d.at(viewport[0], viewport[1]),
        d.at(viewport[2], viewport[3]),
        UiAction::Editor(EditorAction::Brain(BrainAction::Canvas)),
    );
    let cursor = [d.cursor.x / s, d.cursor.y / s];
    // The connections.
    let strips = strips_of(edit, canvas, cursor);
    let key = picture_key(&strips, viewport, s);
    let cached = CONNECTIONS.lock().ok().and_then(|cache| {
        cache
            .as_ref()
            .filter(|cache| cache.key == key)
            .map(|cache| cache.draw.clone())
    });
    let draw = cached.unwrap_or_else(|| {
        let size = [
            (viewport[2] * s).ceil().max(1.0) as u32,
            (viewport[3] * s).ceil().max(1.0) as u32,
        ];
        let rgba = rasterise(
            &strips,
            [0.0, 0.0],
            [viewport[0], viewport[1]],
            size,
            s,
            &pattern(d.ui),
        );
        let draw = UiDraw::Pixels {
            texture: Arc::new(UiTexture {
                width: size[0],
                height: size[1],
                rgba,
            }),
            pos: d.at(viewport[0], viewport[1]),
            size: Vec2::new(size[0] as f32, size[1] as f32),
        };
        if let Ok(mut cache) = CONNECTIONS.lock() {
            *cache = Some(Cached {
                key,
                draw: draw.clone(),
            });
        }
        draw
    });
    frame.draws.push(draw);
    // The nodes.
    let dragging = matches!(
        view.drag,
        Some(BrainDrag::Node { moved: true, .. } | BrainDrag::Pan { .. })
    );
    // A node under an open panel or the help is not the pointer's.
    let covered = open_panel_rect(edit, view).is_some_and(|panel| inside(cursor, panel))
        || (view.info.open && inside(cursor, EDITOR));
    let hovered = match pick(edit, canvas, cursor) {
        Some(Picked::Node(key)) if !dragging && !covered => Some(key),
        _ => None,
    };
    for node in &edit.nodes {
        let at = canvas.to_screen(node.pos);
        let size = NODE * canvas.zoom;
        let rect = [at[0] - size / 2.0, at[1] - size / 2.0, size, size];
        let reach = size / 2.0 + 24.0 * canvas.zoom;
        if at[0] + reach < viewport[0]
            || at[0] - reach > viewport[0] + viewport[2]
            || at[1] + reach < viewport[1]
            || at[1] - reach > viewport[1] + viewport[3]
        {
            continue;
        }
        let selected = edit.selected == Some(Selected::Node(node.key));
        if selected || !node.valid {
            let grow = 3.0 * canvas.zoom;
            let halo = [
                rect[0] - grow,
                rect[1] - grow,
                size + 2.0 * grow,
                size + 2.0 * grow,
            ];
            let colour: Rgba = if selected {
                HIGHLIGHT
            } else {
                [255, 40, 40, 255]
            };
            clipped_pixels(frame, d, &halo_texture(colour), halo, viewport);
        }
        clipped_icon(
            frame,
            d,
            "small_circle_base_icon",
            rect,
            neuron_colour(node.value),
            viewport,
        );
        let icon = ICON * canvas.zoom;
        clipped_icon(
            frame,
            d,
            icon_name(node.index, node.kind),
            [at[0] - icon / 2.0, at[1] - icon / 2.0, icon, icon],
            WHITE,
            viewport,
        );
        // The bias' sign at the top right.
        if node.bias != 0.0 {
            let sign = SIGN * canvas.zoom;
            let alpha = node.bias.abs().clamp(0.0, 1.0).sqrt();
            clipped_icon(
                frame,
                d,
                if node.bias > 0.0 { "plus" } else { "minus" },
                [rect[0] + size - sign, rect[1], sign, sign],
                synapse_colour(node.bias, alpha, true),
                viewport,
            );
        }
        // The value above it (`Text (TMP)`: 14 pt at (0, 24.39) under the node's own scale).
        if app.prefs.show_node_values {
            let text = value_format().format(node.value);
            let px = 14.0 * 1.285_71 * canvas.zoom;
            let centre = at[1] - 24.39 * 1.285_71 * canvas.zoom;
            let width = d.measure(&text, px * s, false) / s;
            let left = at[0] - width / 2.0;
            if left >= viewport[0]
                && left + width <= viewport[0] + viewport[2]
                && centre - px / 2.0 >= viewport[1]
                && centre + px / 2.0 <= viewport[1] + viewport[3]
            {
                put(frame, d, &text, left, centre, px, WHITE);
            }
        }
        if hovered == Some(node.key)
            && let Some(tip) = node_tip(node)
        {
            frame.tips.push(TipRect {
                pos: d.at(rect[0], rect[1]),
                size: d.at(rect[2], rect[3]),
                tip,
            });
        }
    }
}

/// `FloatValueFormat { precision = 2, alwaysShowSign, SI = false }`: a node's value and an edit panel's bias.
fn value_format() -> bibites_core::format::FloatValueFormat {
    bibites_core::format::FloatValueFormat {
        precision: 2,
        always_show_sign: true,
        si: false,
        ..bibites_core::format::FloatValueFormat::default()
    }
}

/// The scroll bars: a track and a handle in proportion, while the content is bigger than the viewport.
fn scroll_bars(frame: &mut UiFrame, d: &Drawer<'_>, canvas: &Canvas) {
    for (horizontal, shown) in [(false, canvas.bars.1), (true, canvas.bars.0)] {
        if !shown {
            continue;
        }
        let (track, handle) = canvas.bar(horizontal);
        d.slice(frame, "scroll", 3.0, 1.0, track, [82, 82, 82, 255]);
        d.slice(frame, "base_icon", 5.0, 1.0, handle, WHITE);
        frame.hit(
            HIT_BARS + u32::from(horizontal),
            d.at(track[0], track[1]),
            d.at(track[2], track[3]),
            UiAction::Editor(EditorAction::Brain(BrainAction::Bar(horizontal))),
        );
    }
}

/// The track and the knob of a slider panel (`SliderPanel` over `Slider`): the panel's own `Background` at 0.392, the slider inset 5 and
/// 3 with its `Background` at 0.255, the fill (`UISprite`) from the panel's left edge to 5 past the handle's middle, the default
/// value's two marks (3 x 6, above and below), and the 20 wide handle.
fn slider(frame: &mut UiFrame, d: &Drawer<'_>, panel: [f32; 4], value: f32, default: f32) {
    d.slice(frame, "Background", 10.0, 1.0, panel, [255, 255, 255, 100]);
    let area = [
        panel[0] + 5.0,
        panel[1] + 3.0,
        panel[2] - 10.0,
        panel[3] - 6.0,
    ];
    d.slice(
        frame,
        "Background",
        10.0,
        1.0 / 0.5,
        [area[0], area[1], area[2] + 5.0, area[3]],
        [255, 255, 255, 65],
    );
    d.slice(
        frame,
        "UISprite",
        10.0,
        1.0 / 0.5,
        [
            panel[0],
            area[1],
            10.0 + (area[2] + 5.0) * value.clamp(0.0, 1.0),
            area[3],
        ],
        WHITE,
    );
    let mark = area[0] + area[2] * default.clamp(0.0, 1.0);
    frame.rect(
        d.at(mark - 1.5, area[1] - 6.0),
        d.at(3.0, 6.0),
        [56, 56, 56, 255],
    );
    frame.rect(
        d.at(mark - 1.5, area[1] + area[3]),
        d.at(3.0, 6.0),
        [56, 56, 56, 255],
    );
    let knob = [
        area[0] + area[2] * value.clamp(0.0, 1.0) - 10.0,
        area[1],
        20.0,
        area[3],
    ];
    d.icon(frame, "base_circle_icon", knob, WHITE);
}

/// What a slider row shows besides its name: the value text, which opens its edit field when pressed, or that field while it is typed.
struct Cell<'a> {
    /// The row's setting.
    field: FieldRef,
    /// Where the value text ends (the slider's left edge less the 15 of spacing).
    right: f32,
    /// Where the edit field goes.
    edit: [f32; 4],
    /// The row's middle.
    centre: f32,
    /// The value as text.
    text: &'a str,
    /// The hit id of the text or the field.
    id: UiId,
}

/// A slider row's value text or, while it is typed, its edit field; true while the field shows.
fn value_cell(frame: &mut UiFrame, d: &Drawer<'_>, app: &App, cell: &Cell<'_>) -> bool {
    if let Some(edit) = app
        .value_edit
        .as_ref()
        .filter(|edit| edit.field == cell.field)
    {
        d.slice(frame, "InputFieldBackground", 4.0, 1.0, cell.edit, WHITE);
        let shown = format!(
            "{}{}",
            edit.text,
            if edit.caret_visible() { "|" } else { "" }
        );
        put(frame, d, &shown, cell.edit[0] + 10.0, cell.centre, 16.0, FIELD);
        frame.hit(
            cell.id,
            d.at(cell.edit[0], cell.edit[1]),
            d.at(cell.edit[2], cell.edit[3]),
            UiAction::ValueEditInside,
        );
        return true;
    }
    put_right(frame, d, cell.text, cell.right, cell.centre, 16.0, LABEL);
    let width = d.measure(cell.text, 16.0 * d.s, false) / d.s;
    frame.hit(
        cell.id,
        d.at(cell.right - width, cell.centre - 13.0),
        d.at(width, 26.0),
        UiAction::DynEditValue(cell.field),
    );
    false
}

/// The slider row of an edit panel: the name (with its help as a tooltip), the value text, and the slider with its 200 (or 150) wide
/// panel at the right end of the row.
struct SliderRow<'a> {
    /// Which slider.
    which: Slider,
    /// The node it is about (for its range), if it is a node's.
    node: Option<&'a EditNode>,
    /// The value.
    value: f32,
    /// The row: `x, y, w, h`.
    row: [f32; 4],
    /// The slider panel's width.
    slider_width: f32,
    /// The setting's name and help.
    name: &'static str,
    helper: &'static str,
    /// The value as text.
    text: String,
    /// The first of the two hit ids (the value, the slider).
    id: UiId,
}

fn slider_row(frame: &mut UiFrame, d: &Drawer<'_>, app: &App, r: &SliderRow<'_>) {
    let row = r.row;
    let centre = row[1] + row[3] / 2.0;
    put(frame, d, r.name, row[0], centre, 16.0, LABEL);
    let name_width = d.measure(r.name, 16.0 * d.s, false) / d.s;
    frame.tips.push(TipRect {
        pos: d.at(row[0], row[1]),
        size: d.at(name_width.max(30.0), row[3]),
        tip: help(r.name, r.helper),
    });
    let field = match r.which {
        Slider::Bias => FieldRef::BrainBias,
        Slider::Act => FieldRef::BrainAct,
        Slider::Weight => FieldRef::BrainWeight,
    };
    // While the value is typed the edit section replaces the value and the slider: it takes what the name leaves of the row, by
    // the layout group's shares (the name's minimum 84, or its own text; the section's 100 to 200, flexible 1).
    let name_share = if r.which == Slider::Weight { 40.0 } else { 84.0 };
    let widths = distribute(
        row[2],
        15.0,
        &[
            Share {
                min: name_share,
                preferred: name_share,
                flexible: 1.0,
            },
            Share {
                min: 100.0,
                preferred: 200.0,
                flexible: 1.0,
            },
        ],
    );
    let panel = [
        row[0] + row[2] - r.slider_width,
        row[1],
        r.slider_width,
        row[3],
    ];
    let cell = Cell {
        field,
        right: panel[0] - 15.0,
        edit: [row[0] + widths[0] + 15.0, centre - 15.0, widths[1], 30.0],
        centre,
        text: &r.text,
        id: r.id,
    };
    if value_cell(frame, d, app, &cell) {
        return;
    }
    slider(
        frame,
        d,
        panel,
        fraction(r.which, r.node, r.value),
        fraction(r.which, r.node, 0.0),
    );
    let (x0, x1) = (panel[0] + 5.0, panel[0] + panel[2] - 5.0);
    frame.hit(
        r.id + 1,
        d.at(panel[0], panel[1]),
        d.at(panel[2], panel[3]),
        UiAction::Editor(EditorAction::Brain(BrainAction::Slide(
            r.which, x0 as u16, x1 as u16,
        ))),
    );
}

/// The panel that is open: the one of the selected node or connection, or the add-node panel.
fn panels(frame: &mut UiFrame, d: &Drawer<'_>, app: &App, session: &EditorSession) {
    let edit = &session.brain;
    if edit.add_mode {
        add_panel(frame, d, session);
        return;
    }
    match edit.selected {
        Some(Selected::Node(key)) => {
            if let Some(node) = edit.node(key) {
                node_panel(frame, d, app, session, node);
            }
        }
        Some(Selected::Edge(key)) => {
            if let Some(edge) = edit.edge(key) {
                edge_panel(frame, d, app, session, edge);
            }
        }
        None => {}
    }
}

/// One of an edit panel's three 35 square buttons: its icon, what it does, its tooltip's name, and whether it works.
type PanelButton<'a> = (&'a str, BrainAction, &'a str, bool);

/// The buttons of an edit panel: two on the left and `delete` at the right end.
fn panel_buttons(
    frame: &mut UiFrame,
    d: &Drawer<'_>,
    panel: [f32; 4],
    buttons: [PanelButton<'_>; 3],
    path: &str,
) {
    let xs = [panel[0] + 5.0, panel[0] + 55.0, panel[0] + panel[2] - 40.0];
    for (index, ((icon, action, name, enabled), x)) in buttons.into_iter().zip(xs).enumerate() {
        let rect = [x, panel[1] + 5.0, 35.0, 35.0];
        let tint: Rgba = if enabled {
            WHITE
        } else {
            [140, 140, 140, 255]
        };
        d.slice(frame, "base_icon", 5.0, 1.0, rect, tint);
        d.icon(frame, icon, rect, tint);
        if enabled {
            frame.hit(
                HIT_BUTTONS + index as UiId,
                d.at(rect[0], rect[1]),
                d.at(rect[2], rect[3]),
                UiAction::Editor(EditorAction::Brain(action)),
            );
        }
        tip(frame, d, rect, &format!("BrainEditor/{path}/buttons/{name}"));
    }
}

/// `ConnectionEditPanel`: the add-node buttons, the delete button, the weight and whether it is enabled.
fn edge_panel(
    frame: &mut UiFrame,
    d: &Drawer<'_>,
    app: &App,
    session: &EditorSession,
    edge: &EditEdge,
) {
    let view = &session.view.brain;
    let panel = panel_rect(EDGE_PANEL, view.panel_at, false);
    d.slice(frame, "base_icon", 5.0, 1.0, panel, WHITE);
    frame.hit(
        HIT_PANEL,
        d.at(panel[0], panel[1]),
        d.at(panel[2], panel[3]),
        UiAction::Editor(EditorAction::Brain(BrainAction::Consume)),
    );
    panel_buttons(
        frame,
        d,
        panel,
        [
            (
                "addNodeBefore_0",
                BrainAction::Split(false),
                "AddNodeBefore",
                true,
            ),
            (
                "addNodeAfter_0",
                BrainAction::Split(true),
                "AddNodeAfter",
                true,
            ),
            ("Delete_0", BrainAction::DeleteSelected, "delete", true),
        ],
        "ConnectionEditPanel",
    );
    // The weight.
    slider_row(
        frame,
        d,
        app,
        &SliderRow {
            which: Slider::Weight,
            node: None,
            value: edge.weight,
            row: [panel[0] + 10.0, panel[1] + 50.0, panel[2] - 20.0, 30.0],
            slider_width: 150.0,
            name: "Weight",
            helper: WEIGHT_HELP,
            text: format!("{:.2}", edge.weight),
            id: HIT_SLIDERS + 4,
        },
    );
    // Enabled: a 30 square box in the middle of the toggle's 150.
    let row = [panel[0] + 10.0, panel[1] + 90.0, panel[2] - 20.0, 30.0];
    put(frame, d, "Enabled", row[0], row[1] + 15.0, 16.0, LABEL);
    frame.tips.push(TipRect {
        pos: d.at(row[0], row[1]),
        size: d.at(60.0, 30.0),
        tip: help("Enabled", ENABLED_HELP),
    });
    let toggle = [row[0] + row[2] - 150.0, row[1], 150.0, 30.0];
    let boxed = [toggle[0] + 60.0, toggle[1], 30.0, 30.0];
    d.slice(frame, "base_icon", 5.0, 1.0, boxed, WHITE);
    if edge.enabled {
        d.icon(frame, "checkmark_icon", boxed, WHITE);
    }
    frame.hit(
        HIT_ENABLED,
        d.at(toggle[0], toggle[1]),
        d.at(toggle[2], toggle[3]),
        UiAction::Editor(EditorAction::Brain(BrainAction::ToggleEnabled)),
    );
}

/// `NodeEditPanel`: the connection and delete buttons, the label, the activation function, and the bias (or an input's value).
fn node_panel(
    frame: &mut UiFrame,
    d: &Drawer<'_>,
    app: &App,
    session: &EditorSession,
    node: &EditNode,
) {
    let view = &session.view.brain;
    let size = node_panel_size(node);
    let panel = panel_rect(size, view.panel_at, false);
    d.slice(frame, "base_icon", 5.0, 1.0, panel, WHITE);
    frame.hit(
        HIT_PANEL,
        d.at(panel[0], panel[1]),
        d.at(panel[2], panel[3]),
        UiAction::Editor(EditorAction::Brain(BrainAction::Consume)),
    );
    let archetype = node.archetype();
    let hidden = archetype == Archetype::Hidden;
    // The buttons: a connection into the node (not for an input), one out of it (not for an output), and delete.
    panel_buttons(
        frame,
        d,
        panel,
        [
            (
                "add In Connection_0",
                BrainAction::AddConnection(false),
                "AddInConection",
                archetype != Archetype::Input,
            ),
            (
                "add Out Connection_0",
                BrainAction::AddConnection(true),
                "AddOutConenction",
                archetype != Archetype::Output,
            ),
            ("Delete_0", BrainAction::DeleteSelected, "delete", true),
        ],
        "nodeEditPanel",
    );
    let (x, right) = (panel[0] + 10.0, panel[0] + panel[2] - 10.0);
    let mut top = panel[1] + 50.0;
    let greyed: Rgba = [200, 200, 200, 255];
    // The label: a text field only a hidden node can use.
    let centre = top + LABEL_ROW / 2.0;
    put(frame, d, "Label", x, centre, 16.0, LABEL);
    frame.tips.push(TipRect {
        pos: d.at(x, top),
        size: d.at(40.0, LABEL_ROW),
        tip: help("Label", LABEL_HELP),
    });
    let field = [panel[0] + 170.0, top, 200.0, LABEL_ROW];
    d.slice(
        frame,
        "InputFieldBackground",
        4.0,
        1.0,
        field,
        if hidden { WHITE } else { greyed },
    );
    let caret = view.label_focus && (view.idle / 0.85) as u32 % 2 == 0;
    let shown = if caret {
        crate::text_field::with_caret(&node.desc)
    } else {
        node.desc.clone()
    };
    put(frame, d, &shown, field[0] + 10.0, centre, 16.0, FIELD);
    if hidden {
        frame.hit(
            HIT_LABEL,
            d.at(field[0], field[1]),
            d.at(field[2], field[3]),
            UiAction::Editor(EditorAction::Brain(BrainAction::FocusLabel)),
        );
    }
    top += LABEL_ROW + 10.0;
    // The activation function: its name, and a dropdown only a hidden node can use.
    let centre = top + FUNCTION_ROW / 2.0;
    put(frame, d, "Activation Function", x, centre, 16.0, LABEL);
    frame.tips.push(TipRect {
        pos: d.at(x, top),
        size: d.at(150.0, FUNCTION_ROW),
        tip: help("Activation Function", FUNCTION_HELP),
    });
    let dropdown = [panel[0] + 170.0, top, 200.0, FUNCTION_ROW];
    let tint = if hidden { [245, 245, 245, 255] } else { greyed };
    d.slice(frame, "UISprite", 4.0, 1.0, dropdown, tint);
    put(
        frame,
        d,
        function_name(node.kind),
        dropdown[0] + 10.0,
        centre,
        16.0,
        FIELD,
    );
    d.icon(
        frame,
        "DropdownArrow",
        [dropdown[0] + dropdown[2] - 25.0, dropdown[1] + 2.5, 20.0, 20.0],
        WHITE,
    );
    if hidden {
        frame.hit(
            HIT_FUNCTION,
            d.at(dropdown[0], dropdown[1]),
            d.at(dropdown[2], dropdown[3]),
            UiAction::DropdownOpen(DropdownTarget::NodeFunction),
        );
    }
    top += FUNCTION_ROW;
    let middle = panel[0] + panel[2] / 2.0;
    if !hidden {
        top += 5.0;
        put_centred(
            frame,
            d,
            "*You can't change an input's or output's label and activation function*",
            middle,
            top + NOTE_ONE / 2.0,
            10.0,
            NOTE,
        );
        top += NOTE_ONE;
    }
    if node.kind.is_non_linear() {
        top += 5.0;
        for (line, text) in [
            "!The activation function is non-linear!",
            "The node's output isn't the result of the input alone",
        ]
        .into_iter()
        .enumerate()
        {
            let centre = top + NOTE_ONE * (line as f32 + 0.5);
            put_centred(frame, d, text, middle, centre, 10.0, WARNING);
        }
        top += NOTE_TWO;
    }
    // The bias (not for an input or an integrator), and an input's value for the simulator.
    if node.kind.has_bias() {
        top += 10.0;
        slider_row(
            frame,
            d,
            app,
            &SliderRow {
                which: Slider::Bias,
                node: Some(node),
                value: session.brain.panel_bias,
                row: [x, top, right - x, SLIDER_ROW],
                slider_width: 200.0,
                name: "Bias",
                helper: BIAS_HELP,
                text: value_format().format(session.brain.panel_bias),
                id: HIT_SLIDERS,
            },
        );
        top += SLIDER_ROW;
    }
    if node.kind == NodeType::Input {
        top += 10.0;
        slider_row(
            frame,
            d,
            app,
            &SliderRow {
                which: Slider::Act,
                node: Some(node),
                value: node.value,
                row: [x, top, right - x, SLIDER_ROW],
                slider_width: 200.0,
                name: "Value",
                helper: ACT_HELP,
                text: value_format().format(node.value),
                id: HIT_SLIDERS + 2,
            },
        );
    }
}

/// `AddNodePanel`: the filter and the three sections of what can be added.
fn add_panel(frame: &mut UiFrame, d: &Drawer<'_>, session: &EditorSession) {
    let edit = &session.brain;
    let view = &session.view.brain;
    let panel = panel_rect(ADD_PANEL, view.panel_at, true);
    d.slice(frame, "base_icon", 5.0, 1.0, panel, WHITE);
    frame.hit(
        HIT_PANEL,
        d.at(panel[0], panel[1]),
        d.at(panel[2], panel[3]),
        UiAction::Editor(EditorAction::Brain(BrainAction::Consume)),
    );
    // The filter field.
    let field = [panel[0], panel[1], panel[2], FILTER_H];
    d.slice(frame, "base_icon", 5.0, 1.0, field, WHITE);
    let centre = field[1] + FILTER_H / 2.0;
    if view.filter.is_empty() && !view.filter_focus {
        put(
            frame,
            d,
            "Filter...",
            field[0] + 10.0,
            centre,
            16.0,
            [50, 50, 50, 128],
        );
    } else {
        let caret = view.filter_focus && (view.idle / 0.85) as u32 % 2 == 0;
        let shown = if caret {
            crate::text_field::with_caret(&view.filter)
        } else {
            view.filter.clone()
        };
        put(frame, d, &shown, field[0] + 10.0, centre, 16.0, FIELD);
    }
    frame.hit(
        HIT_FILTER,
        d.at(field[0], field[1]),
        d.at(field[2], field[3]),
        UiAction::Editor(EditorAction::Brain(BrainAction::FocusFilter)),
    );
    // The list: a header for each section that shows, with the items under it while it is open.
    let sections = add_sections(edit, &view.filter);
    let (viewport, bar) = add_viewport(panel);
    let content = add_content_height(&sections, view.sections);
    let scroll = view.add_scroll.clamp(0.0, (content - viewport[3]).max(0.0));
    let cursor = [d.cursor.x / d.s, d.cursor.y / d.s];
    let mut y = viewport[1] - scroll;
    for (number, section) in sections.iter().enumerate() {
        if !section.shown {
            continue;
        }
        let header = [viewport[0], y, viewport[2], ROW];
        row_background(frame, d, header, viewport, WHITE);
        if row_visible(header, viewport) {
            let centre = y + ROW / 2.0;
            put_right(
                frame,
                d,
                &section.items.len().to_string(),
                header[0] + 38.0,
                centre,
                16.0,
                [48, 48, 48, 255],
            );
            put(
                frame,
                d,
                section.title,
                header[0] + 48.0,
                centre,
                16.0,
                [38, 38, 38, 255],
            );
            d.icon(
                frame,
                "Arrow",
                [header[0] + header[2] - 28.0, y + 2.0, 20.0, 17.0],
                WHITE,
            );
            frame.hit(
                HIT_SECTIONS + number as UiId,
                d.at(header[0], header[1]),
                d.at(header[2], header[3]),
                UiAction::Editor(EditorAction::Brain(BrainAction::ToggleSection(
                    number as u8,
                ))),
            );
        }
        y += ROW;
        if !view.sections[number] {
            continue;
        }
        for (at, (index, name)) in section.items.iter().enumerate() {
            let item = [viewport[0], y, viewport[2], ROW];
            let over = inside(cursor, item) && inside(cursor, viewport);
            let tint = if over { [225, 235, 255, 255] } else { WHITE };
            row_background(frame, d, item, viewport, tint);
            if row_visible(item, viewport) {
                let sprite = match number {
                    0 => icon_name(usize::from(*index), NodeType::Linear),
                    1 => icon_name(usize::from(*index) + N_INPUTS, NodeType::Linear),
                    _ => icon_name(
                        BASE_NODES,
                        NodeType::from_u8(*index as u8).unwrap_or(NodeType::Linear),
                    ),
                };
                d.icon(frame, sprite, [item[0] + 3.0, y + 1.5, 18.0, 18.0], WHITE);
                put(frame, d, name, item[0] + 28.0, y + ROW / 2.0, 14.0, FIELD);
                frame.hit(
                    HIT_ITEMS + number as UiId * 100 + at as UiId,
                    d.at(item[0], item[1]),
                    d.at(item[2], item[3]),
                    UiAction::Editor(EditorAction::Brain(BrainAction::AddNode(
                        number as u8,
                        *index,
                    ))),
                );
            }
            y += ROW;
        }
    }
    // The list's scroll bar, while it does not fit.
    if content > viewport[3] {
        d.slice(frame, "scroll", 3.0, 1.0, bar, [82, 82, 82, 255]);
        let handle = add_handle(bar, viewport[3], content, scroll);
        d.slice(frame, "base_icon", 5.0, 1.0, handle, WHITE);
        frame.hit(
            HIT_ADD_BAR,
            d.at(bar[0], bar[1]),
            d.at(bar[2], bar[3]),
            UiAction::Editor(EditorAction::Brain(BrainAction::AddBar)),
        );
    }
}

/// The add-node list's scroll bar handle (the sliding area is the track less 12, the handle overhangs it by 12).
fn add_handle(bar: [f32; 4], viewport_h: f32, content: f32, scroll: f32) -> [f32; 4] {
    let size = (viewport_h / content).min(1.0);
    let area = bar[3] - BAR;
    let room = (content - viewport_h).max(1.0);
    let along = (scroll / room).clamp(0.0, 1.0);
    [
        bar[0],
        bar[1] + along * (1.0 - size) * area,
        BAR,
        size * area + BAR,
    ]
}

/// Whether a row of the add-node list is whole inside its viewport.
fn row_visible(row: [f32; 4], viewport: [f32; 4]) -> bool {
    row[1] >= viewport[1] - 0.01 && row[1] + row[3] <= viewport[1] + viewport[3] + 0.01
}

/// A row's background: the panel's sprite while the row is whole, a flat strip cut to the viewport while it is not.
fn row_background(
    frame: &mut UiFrame,
    d: &Drawer<'_>,
    row: [f32; 4],
    viewport: [f32; 4],
    tint: Rgba,
) {
    if row_visible(row, viewport) {
        d.slice(frame, "base_icon", 5.0, 1.0, row, tint);
        return;
    }
    let y0 = row[1].max(viewport[1]);
    let y1 = (row[1] + row[3]).min(viewport[1] + viewport[3]);
    if y1 > y0 {
        frame.rect(d.at(row[0], y0), d.at(row[2], y1 - y0), tint);
    }
}

/// The help (`Brain Info Panel`): what the editor is, its keys, and how a brain works. It scrolls with the wheel.
fn info_panel(frame: &mut UiFrame, d: &Drawer<'_>, view: &BrainView) {
    let panel = EDITOR;
    // `Long_UI` (a border of 70 pixels at 500 pixels per unit) over the whole editor, and the scroll view inset by 10 in it, whose
    // content has 200 of padding at its sides and 16 above.
    d.slice(frame, "Long_UI", 70.0, 0.2, panel, WHITE);
    frame.hit(
        HIT_INFO,
        d.at(panel[0], panel[1]),
        d.at(panel[2], panel[3]),
        UiAction::Editor(EditorAction::Brain(BrainAction::Consume)),
    );
    let scroll_view = [panel[0] + 10.0, panel[1] + 10.0, panel[2] - 20.0, panel[3] - 20.0];
    let x = scroll_view[0] + 200.0;
    let width = scroll_view[2] - 9.0 - 400.0;
    let mut y = scroll_view[1] + 16.0 - view.info_scroll;
    for (text, size, centred, gap) in INFO_TEXTS {
        let lines = crate::tooltip::wrap(text, width * d.s, &|line: &str| {
            d.measure(line, size * d.s, false)
        });
        for line in lines {
            let line_h = size * 1.1719;
            // A line shows when it is whole inside the scroll view (the renderers have no clip).
            if y >= scroll_view[1] && y + line_h <= scroll_view[1] + scroll_view[3] {
                if centred {
                    put_centred(frame, d, &line, x + width / 2.0, y + line_h / 2.0, size, WHITE);
                } else {
                    put(frame, d, &line, x, y + line_h / 2.0, size, WHITE);
                }
            }
            y += line_h;
        }
        y += gap;
    }
}

/// How far the help may scroll: its text's height less the panel's, by a rough width of the characters (the window has the real ones).
#[must_use]
pub fn info_scroll_max() -> f32 {
    let width = EDITOR[2] - 20.0 - 9.0 - 400.0;
    let mut y = 32.0;
    for (text, size, _, gap) in INFO_TEXTS {
        let lines = crate::tooltip::wrap(text, width, &|line: &str| {
            line.chars().count() as f32 * size * 0.52
        });
        y += lines.len() as f32 * size * 1.1719 + gap;
    }
    (y - (EDITOR[3] - 20.0)).max(0.0)
}

/// The help's texts (`Brain Info Panel`): the text, its size, whether it is centred, and the space under it.
const INFO_TEXTS: [(&str, f32, bool, f32); 13] = [
    ("Brain Editor", 50.22, true, 8.0),
    ("Edit a bibite's brain to your heart's content!\nSimply click on an element of the brain to edit it", 24.0, false, 16.0),
    ("Keyboard shortcuts:", 36.0, false, 4.0),
    ("[SHIFT-Click Node] => Create a new Connection from that node\n   Simply click on the node you want to link to the previous one\n   *Output nodes can't have outgoing connections, so to it instead*\n\n[SPACE] => Create a node at your mouse position\n   This will open a dropdown with all avalaible choice and a search bar\n   *you can do this even when you're making a new connection*\n\n[DELETE] => Delete selected element\n   *will also remove all connections to and from a removed node*\n\n[ESCAPE] => Unselect  element, cancel node or connection placement", 24.0, false, 16.0),
    ("Brain Basics and Functionning:", 36.0, false, 4.0),
    ("Every brain tick, 4 steps execute one after the other:\n\n1 - Recording Senses ", 28.0, false, 2.0),
    ("The senses of the bibites are recorded and writen as the according \n", 24.0, false, 8.0),
    ("2 - Propagating Activations", 28.0, false, 2.0),
    ("Connections take the last tick's output from their outgoing node, multiply it to their Weight, and add it to their ingoing node's input. That's called the activation (node.output * connection.weight)", 24.0, false, 8.0),
    ("3 - Node Activation", 28.0, false, 2.0),
    ("Nodes collect all the activations they received, including their Bias (default activation), and (for most activation functions) sums them together. \nThis is called a node's Total Activation.\nThis total activation is then passed through the node's Activation Function (as the activation function is literally a function f(x) ) to produce an output", 24.0, false, 4.0),
    ("*most nodes activation functions are linear (i.e. the same total activation will always produce the same output), but some depend on other values like the node's last output, and are very useful to allow memories and adaptation to evolve and develop *", 18.0, false, 8.0),
    ("4 - Taking actions\nThe output nodes' final output value is taken and distributed to the different bibite's systems to influence behavior.\nAll of this means that information propagates one node at a time. A direct connection between an input and output node won't add any delay, but every hidden node the information has to go through before reaching an output will add one tick of delay", 24.0, false, 8.0),
];

/// The notice at the top of the preview while the Brain tab is open (`positionSavedMessages`): "*Nodes placement WILL (NOT) be saved with
/// template*".
pub fn saved_notice(frame: &mut UiFrame, d: &Drawer<'_>, keeping: bool, preview: [f32; 4]) {
    let parts: [(&str, Rgba); 3] = [
        ("*Nodes placement", WHITE),
        if keeping {
            ("WILL", [10, 217, 0, 255])
        } else {
            ("WILL NOT", [255, 0, 2, 255])
        },
        ("be saved with template*", WHITE),
    ];
    // Three text objects in a row, a few pixels apart.
    let gap = 6.0;
    let widths: Vec<f32> = parts
        .iter()
        .map(|(text, _)| d.measure(text, 20.0 * d.s, false) / d.s)
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + gap * 2.0;
    let mut x = preview[0] + preview[2] / 2.0 - total / 2.0;
    for ((text, colour), width) in parts.iter().zip(widths) {
        put(frame, d, text, x, NOTICE_Y, 20.0, *colour);
        x += width + gap;
    }
}

// ---------------------------------------------------------------------------------------- the interaction

impl App {
    /// The scale from the window's pixels to the reference's (the editor is drawn at the UI scaler's scale).
    fn editor_brain_scale(&self) -> f32 {
        UiFrame::scaler(self.camera.screen.x, self.camera.screen.y)
    }

    /// The canvas of the open editor.
    fn editor_brain_canvas(&self) -> Option<Canvas> {
        let session = self.bibite_editor.as_ref()?;
        Some(Canvas::new(
            session.brain.content,
            session.view.brain.zoom,
            session.view.brain.pan,
        ))
    }

    /// A control of the Brain tab was clicked (the canvas, the sliders and the bars are held: [`Self::route_brain_pointer`]).
    pub fn editor_brain_action(&mut self, action: BrainAction) {
        let canvas = self.editor_brain_canvas();
        let Some(session) = self.bibite_editor.as_mut() else {
            return;
        };
        // A press on one of the tab's controls takes the keyboard from the name and description fields.
        session.view.focus = None;
        let (edit, view) = (&mut session.brain, &mut session.view.brain);
        match action {
            BrainAction::Step => edit.one_step(),
            BrainAction::Play(on) => edit.play(on),
            BrainAction::ToggleValues => {
                self.prefs.show_node_values = !self.prefs.show_node_values;
                if let Err(error) =
                    bibites_io::storage::save_user_preferences(&self.storage, &self.prefs)
                {
                    self.last_action = format!("could not save the preferences: {error}");
                }
            }
            BrainAction::SaveAnchors => edit.set_save_anchors(true),
            BrainAction::Resort => {
                if edit.is_valid() {
                    edit.resort();
                    // `SizePanelAndPlaceBrain`: `brainRT.anchoredPosition = Vector2.zero`.
                    view.pan = [0.0, 0.0];
                    view.label_focus = false;
                }
            }
            BrainAction::ShowInfo => view.info.show(),
            BrainAction::AddConnection(from) => {
                if let Some(Selected::Node(key)) = edit.selected {
                    if from {
                        edit.new_connection(Some(key), None);
                    } else {
                        edit.new_connection(None, Some(key));
                    }
                    view.label_focus = false;
                }
            }
            BrainAction::DeleteSelected => {
                delete_selected(edit);
                view.label_focus = false;
            }
            BrainAction::Split(remain_ingoing) => {
                if let Some(Selected::Edge(key)) = edit.selected {
                    edit.split_connection(key, remain_ingoing);
                    hang_panel_on_selected_node(edit, view, canvas.as_ref());
                }
            }
            BrainAction::ToggleEnabled => {
                if let Some(Selected::Edge(key)) = edit.selected {
                    let enabled = edit.edge(key).is_some_and(|edge| edge.enabled);
                    edit.edit_enabled(key, !enabled);
                }
            }
            BrainAction::FocusLabel => {
                if let Some(Selected::Node(key)) = edit.selected
                    && let Some(node) = edit.node(key)
                {
                    if !view.label_focus {
                        view.label_original = Some((key, node.desc.clone()));
                    }
                    view.label_focus = true;
                    view.filter_focus = false;
                    view.idle = 0.0;
                }
            }
            BrainAction::FocusFilter => {
                view.filter_focus = true;
                view.label_focus = false;
                view.idle = 0.0;
            }
            BrainAction::ToggleSection(section) => {
                let open = &mut view.sections[usize::from(section).min(2)];
                *open = !*open;
            }
            BrainAction::AddNode(section, index) => {
                let shift = view.shift;
                match section {
                    0 => edit.add_node_holding(usize::from(index), Archetype::Input, shift),
                    1 => edit.add_node_holding(
                        usize::from(index) + N_INPUTS,
                        Archetype::Output,
                        shift,
                    ),
                    _ => edit.add_node_holding(usize::from(index), Archetype::Hidden, shift),
                }
                view.filter.clear();
                view.filter_focus = false;
                hang_panel_on_selected_node(edit, view, canvas.as_ref());
            }
            BrainAction::Slide(..)
            | BrainAction::Canvas
            | BrainAction::Bar(_)
            | BrainAction::AddBar
            | BrainAction::Consume => {}
        }
    }

    /// A press on the canvas (`cursor` in window px): the left button on a node starts a drag or a click, on a connection a click; the
    /// right button on anything but a node starts to pan.
    pub fn editor_brain_press(&mut self, cursor: Vec2, secondary: bool) {
        let scale = self.editor_brain_scale();
        let point = [cursor.x / scale, cursor.y / scale];
        let Some(canvas) = self.editor_brain_canvas() else {
            return;
        };
        let Some(session) = self.bibite_editor.as_mut() else {
            return;
        };
        session.view.focus = None;
        let (edit, view) = (&session.brain, &mut session.view.brain);
        view.label_focus = false;
        view.filter_focus = false;
        let picked = pick(edit, &canvas, point);
        let at = [cursor.x, cursor.y];
        view.drag = if secondary {
            // A node takes the right button's drag too (`NodeEditor.OnDrag` ignores it): the canvas does not pan then.
            match picked {
                Some(Picked::Node(_)) => None,
                _ => Some(BrainDrag::Pan {
                    start: at,
                    from: None,
                }),
            }
        } else {
            match picked {
                Some(Picked::Node(key)) => Some(BrainDrag::Node {
                    key,
                    start: at,
                    moved: false,
                }),
                Some(Picked::Edge(key)) => Some(BrainDrag::Edge {
                    key,
                    start: at,
                    moved: false,
                }),
                None => None,
            }
        };
    }

    /// A press on a slider or a scroll bar starts to be held (`action` is the control that was pressed, `cursor` in window px).
    pub fn editor_brain_hold(&mut self, action: BrainAction, cursor: Vec2) {
        let scale = self.editor_brain_scale();
        let point = [cursor.x / scale, cursor.y / scale];
        let canvas = self.editor_brain_canvas();
        let Some(session) = self.bibite_editor.as_mut() else {
            return;
        };
        session.view.focus = None;
        let (edit, view) = (&mut session.brain, &mut session.view.brain);
        view.label_focus = false;
        view.filter_focus = false;
        match action {
            BrainAction::Slide(slider, x0, x1) => {
                view.drag = Some(BrainDrag::Slider(slider, x0, x1));
            }
            BrainAction::Bar(horizontal) => {
                let Some(canvas) = canvas else {
                    return;
                };
                let (_, handle) = canvas.bar(horizontal);
                let start = if horizontal { handle[0] } else { handle[1] };
                let at = if horizontal { point[0] } else { point[1] };
                let length = if horizontal { handle[2] } else { handle[3] };
                // On the handle it is grabbed where it was pressed; on the track it jumps to be centred on the press.
                let grab = if at >= start && at <= start + length {
                    at - start
                } else {
                    length / 2.0
                };
                view.drag = Some(BrainDrag::Bar(horizontal, grab));
            }
            BrainAction::AddBar => {
                let panel = panel_rect(ADD_PANEL, view.panel_at, true);
                let (viewport, bar) = add_viewport(panel);
                let sections = add_sections(edit, &view.filter);
                let content = add_content_height(&sections, view.sections);
                let handle = add_handle(bar, viewport[3], content, view.add_scroll);
                let grab = if point[1] >= handle[1] && point[1] <= handle[1] + handle[3] {
                    point[1] - handle[1]
                } else {
                    handle[3] / 2.0
                };
                view.drag = Some(BrainDrag::AddBar(grab));
            }
            _ => {}
        }
    }

    /// Whether a press of the Brain tab is being held, and whether the right button holds it (a pan).
    #[must_use]
    pub fn editor_brain_dragging(&self) -> Option<bool> {
        let session = self.bibite_editor.as_ref()?;
        session
            .view
            .brain
            .drag
            .map(|drag| matches!(drag, BrainDrag::Pan { .. }))
    }

    /// The held press follows the cursor (window px).
    pub fn editor_brain_drag_to(&mut self, cursor: Vec2) {
        let scale = self.editor_brain_scale();
        let point = [cursor.x / scale, cursor.y / scale];
        let at = [cursor.x, cursor.y];
        let Some(canvas) = self.editor_brain_canvas() else {
            return;
        };
        let Some(session) = self.bibite_editor.as_mut() else {
            return;
        };
        let (edit, view) = (&mut session.brain, &mut session.view.brain);
        let Some(drag) = view.drag else {
            return;
        };
        // `EventSystem.pixelDragThreshold`.
        let far = |start: [f32; 2]| (at[0] - start[0]).hypot(at[1] - start[1]) >= 10.0;
        match drag {
            BrainDrag::Node { key, start, moved } => {
                if moved || far(start) {
                    view.drag = Some(BrainDrag::Node {
                        key,
                        start,
                        moved: true,
                    });
                    edit.drag_node(key, canvas.to_place(point));
                }
            }
            BrainDrag::Edge { key, start, moved } => {
                if !moved && far(start) {
                    view.drag = Some(BrainDrag::Edge {
                        key,
                        start,
                        moved: true,
                    });
                }
            }
            BrainDrag::Pan { start, from } => {
                let from = match from {
                    Some(from) => from,
                    None if far(start) => {
                        // `ScrollRect.OnBeginDrag` takes the cursor, and the content, where the threshold was passed.
                        [at[0], at[1], canvas.pan[0], canvas.pan[1]]
                    }
                    None => return,
                };
                // `ScrollRect.OnDrag` (clamped): the content's start plus the cursor's whole way since, then kept inside its limits.
                let pan = canvas.clamped([
                    from[2] + (at[0] - from[0]) / scale,
                    from[3] - (at[1] - from[1]) / scale,
                ]);
                if pan != canvas.pan {
                    view.pan = pan;
                    // `OnPanelMoved`: the content moved, so the selection and the add-node panel go.
                    edit.deselect();
                    edit.close_add_node_panel();
                }
                view.drag = Some(BrainDrag::Pan {
                    start,
                    from: Some(from),
                });
            }
            BrainDrag::Slider(slider, x0, x1) => {
                let selected = edit.selected;
                let node = match selected {
                    Some(Selected::Node(key)) => edit.node(key).cloned(),
                    _ => None,
                };
                let value = slider_value(
                    slider,
                    node.as_ref(),
                    f32::from(x0),
                    f32::from(x1),
                    point[0],
                );
                match (slider, selected) {
                    (Slider::Bias, Some(Selected::Node(key))) => edit.edit_bias(key, value),
                    (Slider::Act, Some(Selected::Node(key))) => edit.edit_act(key, value),
                    (Slider::Weight, Some(Selected::Edge(key))) => edit.edit_weight(key, value),
                    _ => {}
                }
            }
            BrainDrag::Bar(horizontal, grab) => {
                let (track, handle) = canvas.bar(horizontal);
                let (start, length, along_track) = if horizontal {
                    (track[0], handle[2], point[0])
                } else {
                    (track[1], handle[3], point[1])
                };
                let travel = (if horizontal { track[2] } else { track[3] }) - length;
                let along = if travel > 0.0 {
                    (along_track - grab - start) / travel
                } else {
                    0.0
                };
                let pan = canvas.pan_for_bar(horizontal, along);
                if pan != canvas.pan {
                    view.pan = canvas.clamped(pan);
                    edit.deselect();
                    edit.close_add_node_panel();
                }
            }
            BrainDrag::AddBar(grab) => {
                let panel = panel_rect(ADD_PANEL, view.panel_at, true);
                let (viewport, bar) = add_viewport(panel);
                let sections = add_sections(edit, &view.filter);
                let content = add_content_height(&sections, view.sections);
                let size = (viewport[3] / content).min(1.0);
                let travel = (bar[3] - BAR) * (1.0 - size);
                let along = if travel > 0.0 {
                    (point[1] - grab - bar[1]) / travel
                } else {
                    0.0
                };
                view.add_scroll = along.clamp(0.0, 1.0) * (content - viewport[3]).max(0.0);
            }
        }
    }

    /// The held press was let go (window px): a press on a node or a connection that did not move is a click.
    pub fn editor_brain_release(&mut self, cursor: Vec2) {
        let scale = self.editor_brain_scale();
        let point = [cursor.x / scale, cursor.y / scale];
        let Some(canvas) = self.editor_brain_canvas() else {
            return;
        };
        let Some(session) = self.bibite_editor.as_mut() else {
            return;
        };
        let (edit, view) = (&mut session.brain, &mut session.view.brain);
        let Some(drag) = view.drag.take() else {
            return;
        };
        // A click needs the press and the release on the same thing (`pointerPress == pointerUpHandler`).
        let under = pick(edit, &canvas, point);
        match drag {
            BrainDrag::Node {
                key, moved: false, ..
            } if under == Some(Picked::Node(key)) => {
                view.panel_at = to_editor_space(point);
                view.label_focus = false;
                edit.click_node(key, view.shift);
            }
            BrainDrag::Edge {
                key, moved: false, ..
            } if under == Some(Picked::Edge(key)) => {
                view.panel_at = to_editor_space(point);
                view.label_focus = false;
                edit.click_edge(key);
            }
            _ => {}
        }
    }

    /// The right button's edge: `true` when it went down this frame; a release ends a pan.
    pub fn editor_brain_secondary_edge(&mut self, down: bool) -> bool {
        let Some(session) = self.bibite_editor.as_mut() else {
            return false;
        };
        let view = &mut session.view.brain;
        let edge = down && !view.secondary_down;
        view.secondary_down = down;
        if !down && matches!(view.drag, Some(BrainDrag::Pan { .. })) {
            view.drag = None;
        }
        edge
    }

    /// The Brain tab's share of a frame's pointer routing: presses on the canvas, the sliders and the scroll bars start to be held,
    /// and a held press follows the cursor until it is let go. Returns `true` when the pointer was taken (the mouse fields of `input`
    /// are then cleared). `secondary` is the right button, which the editor's caller keeps out of the frame's input.
    pub fn route_brain_pointer(
        &mut self,
        frame: &UiFrame,
        input: &mut FrameInput,
        secondary: bool,
    ) -> bool {
        let Some(session) = self.bibite_editor.as_ref() else {
            return false;
        };
        if session.view.tab != Tab::Brain {
            return false;
        }
        let cursor = input.cursor;
        let action = match frame.action_at(cursor) {
            Some(UiAction::Editor(EditorAction::Brain(action))) => Some(action),
            _ => None,
        };
        let right_edge = self.editor_brain_secondary_edge(secondary);
        if right_edge && action == Some(BrainAction::Canvas) {
            self.editor_brain_press(cursor, true);
        }
        if input.primary_pressed {
            match action {
                Some(BrainAction::Canvas) => {
                    self.value_edit = None;
                    self.editor_brain_press(cursor, false);
                }
                Some(
                    held @ (BrainAction::Slide(..) | BrainAction::Bar(_) | BrainAction::AddBar),
                ) => {
                    self.value_edit = None;
                    self.editor_brain_hold(held, cursor);
                }
                _ => {}
            }
        }
        let Some(right) = self.editor_brain_dragging() else {
            return false;
        };
        let down = if right {
            secondary
        } else {
            input.primary_down
        };
        if down {
            self.editor_brain_drag_to(cursor);
        }
        if !down || (!right && input.primary_released) {
            self.editor_brain_release(cursor);
        }
        input.primary_pressed = false;
        input.primary_released = false;
        input.primary_down = false;
        input.wheel = 0.0;
        true
    }

    /// The wheel over the editor: over the canvas it zooms about the cursor (`Zoom`), unless something is selected or the add-node
    /// panel or the help is open; over the add-node list or the help it scrolls them by 25 px a notch. Returns whether the Brain
    /// tab took it.
    pub fn editor_brain_wheel(&mut self, cursor: Vec2, wheel: f32) -> bool {
        let scale = self.editor_brain_scale();
        let point = [cursor.x / scale, cursor.y / scale];
        let Some(canvas) = self.editor_brain_canvas() else {
            return false;
        };
        let typing = self.name_or_description_focused();
        let Some(session) = self.bibite_editor.as_mut() else {
            return false;
        };
        if session.view.tab != Tab::Brain || wheel == 0.0 {
            return false;
        }
        let (edit, view) = (&mut session.brain, &mut session.view.brain);
        if view.info.open && inside(point, EDITOR) {
            view.info_scroll = (view.info_scroll - wheel * 25.0).clamp(0.0, info_scroll_max());
            return true;
        }
        if edit.add_mode {
            let panel = panel_rect(ADD_PANEL, view.panel_at, true);
            if inside(point, panel) {
                let (viewport, _) = add_viewport(panel);
                let sections = add_sections(edit, &view.filter);
                let content = add_content_height(&sections, view.sections);
                view.add_scroll =
                    (view.add_scroll - wheel * 25.0).clamp(0.0, (content - viewport[3]).max(0.0));
                return true;
            }
        }
        if inside(point, EDITOR) && !typing && edit.selected.is_none() && !edit.add_mode {
            let (zoom, pan) = canvas.zoomed(wheel * 0.1, point);
            let mut next = Canvas::new(edit.content, zoom, pan);
            next.pan = next.clamped(pan);
            view.zoom = zoom;
            view.pan = next.pan;
            // The scroll rect sees its content move: the selection and the add-node panel go.
            edit.deselect();
            edit.close_add_node_panel();
        }
        inside(point, EDITOR)
    }

    /// Whether the name or the description field has the focus (`BibiteEditorPanel.editingText`).
    fn name_or_description_focused(&self) -> bool {
        self.bibite_editor
            .as_ref()
            .is_some_and(|session| session.view.focus.is_some())
    }

    /// One frame of the Brain tab's keys and clock (the raw input): key `1` steps the simulator, Space opens or closes the add-node
    /// panel over the pointer, Delete removes what is selected, the add-node panel's Enter adds the one thing left on its list, and
    /// the help shows while the pointer is on its button or on it.
    pub fn editor_brain_frame(&mut self, input: &FrameInput) {
        let scale = self.editor_brain_scale();
        let cursor = [input.cursor.x / scale, input.cursor.y / scale];
        let Some(canvas) = self.editor_brain_canvas() else {
            return;
        };
        let editing_text = self.name_or_description_focused();
        let Some(session) = self.bibite_editor.as_mut() else {
            return;
        };
        let (edit, view) = (&mut session.brain, &mut session.view.brain);
        view.shift = input.shift_down;
        view.idle += input.dt;
        if session.view.tab != Tab::Brain {
            return;
        }
        view.pan = canvas.pan;
        // The simulator runs whatever the pointer does (`UpdatePanel`).
        edit.tick(input.dt, input.digit == Some(1), input.digit1_down);
        // A field whose panel is gone has no focus.
        if edit.selected.is_none() {
            view.label_focus = false;
        }
        if !edit.add_mode {
            view.filter_focus = false;
        }
        // The help: the pointer on its button, or on it once it shows (`OnHoverShow`, `OnHoverKeep`).
        let layout = toolbar_layout(edit.save_anchors);
        let over = inside(cursor, [layout.info, BUTTON_Y, BUTTON, BUTTON])
            || (view.info.open && inside(cursor, EDITOR));
        view.info.update(over, input.dt);
        if !view.info.open {
            view.info_scroll = 0.0;
        }
        // Delete (`NodeEditPanel.Update`, `ConnectionEditPanel.Update`: while their panel shows, whatever has the keyboard).
        if input.kill_target {
            delete_selected(edit);
        }
        // Space: nothing selected, the pointer over the editor, the name and description not being typed.
        if input.toggle_pause && inside(cursor, EDITOR) && !editing_text && edit.selected.is_none()
        {
            if edit.add_mode {
                edit.close_add_node_panel();
            } else if !view.info.open {
                edit.request_add_node(canvas.to_place(cursor));
                view.panel_at = to_editor_space(cursor);
                view.filter.clear();
                view.filter_focus = true;
                view.label_focus = false;
                view.add_scroll = 0.0;
                // The key that opened the panel is not the first character of its filter.
                self.typed.clear();
            }
        }
        // `AddNodePanel.Update`: Enter with one item left on the list adds it.
        if edit.add_mode && input.enter {
            let sections = add_sections(edit, &view.filter);
            let total: usize = sections.iter().map(|section| section.items.len()).sum();
            if total == 1 {
                for (number, section) in sections.iter().enumerate() {
                    if let Some((index, _)) = section.items.first() {
                        self.editor_brain_action(BrainAction::AddNode(number as u8, *index));
                        return;
                    }
                }
            }
        }
    }

    /// Escape in the Brain tab: a field with the focus gives it up (`TMP_InputField` cancels: the label has its text from before it
    /// was edited again), and in the same press the newest thing of the editor is undone (`UINavigationManager` pops its stack on
    /// `GetKeyDown(Escape)` whatever has the keyboard; the help is not on it: it goes when the pointer leaves it). Returns whether
    /// something was.
    pub fn editor_brain_escape(&mut self) -> bool {
        let Some(session) = self.bibite_editor.as_mut() else {
            return false;
        };
        if session.view.tab != Tab::Brain {
            return false;
        }
        let (edit, view) = (&mut session.brain, &mut session.view.brain);
        let mut did = false;
        if view.label_focus {
            view.label_focus = false;
            if let Some((key, original)) = view.label_original.take() {
                edit.edit_label(key, &original);
            }
            did = true;
        }
        if view.filter_focus {
            view.filter_focus = false;
            did = true;
        }
        edit.escape() || did
    }

    /// Characters typed into the Brain tab's fields (the node's label, the add-node filter), Backspace and Ctrl + V. Returns whether
    /// one of them has the focus (the others leave the characters alone).
    pub fn editor_brain_type(&mut self, input: &FrameInput) -> bool {
        let focused = self.bibite_editor.as_ref().is_some_and(|session| {
            session.view.brain.label_focus || session.view.brain.filter_focus
        });
        if !focused {
            return false;
        }
        let typed = std::mem::take(&mut self.typed);
        let pasted = if input.paste_tag {
            self.clipboard_in.take().unwrap_or_default()
        } else {
            String::new()
        };
        let keys = self.edit_keys(input);
        let typed = format!("{typed}{pasted}");
        let Some(session) = self.bibite_editor.as_mut() else {
            return false;
        };
        let (edit, view) = (&mut session.brain, &mut session.view.brain);
        let owner = if view.label_focus { 6 } else { 7 };
        let mut changed = false;
        let caret = &mut self.edit_caret;
        let owner_now = &mut self.edit_caret_owner;
        let mut edited = |text: &mut String| {
            if *owner_now != owner {
                *owner_now = owner;
                *caret = crate::text_field::Caret::at_end(text);
            }
            let before = text.clone();
            crate::text_field::edit(text, caret, &typed, usize::MAX, keys);
            changed = *text != before;
        };
        if view.label_focus {
            if let Some(Selected::Node(key)) = edit.selected
                && let Some(node) = edit.node(key)
            {
                let mut text = node.desc.clone();
                edited(&mut text);
                if changed {
                    edit.edit_label(key, &text);
                }
            }
            if input.enter {
                view.label_focus = false;
            }
        } else {
            let mut text = std::mem::take(&mut view.filter);
            edited(&mut text);
            view.filter = text;
            if changed {
                view.add_scroll = 0.0;
            }
        }
        if changed {
            view.idle = 0.0;
        }
        true
    }

    /// The rules of a typed value of the panels' sliders, and what it holds now.
    #[must_use]
    pub fn editor_brain_typed_rules(&self, field: FieldRef) -> Option<(crate::value_edit::Rules, f32)> {
        let session = self.bibite_editor.as_ref()?;
        let edit = &session.brain;
        let node = match edit.selected {
            Some(Selected::Node(key)) => edit.node(key),
            _ => None,
        };
        // The value is the setting's own (the panel's bias: see `BrainEdit::panel_bias`), and its range the setting's: the value
        // setting of an input keeps -5 ..= 5 whatever the slider's track is (`actSlider.SetMinMax` is the slider's alone).
        let (value, (min, max, out_of_bounds)) = match field {
            FieldRef::BrainBias => {
                node?;
                (edit.panel_bias, slider_range(Slider::Bias, node))
            }
            FieldRef::BrainAct => (node?.value, (-5.0, 5.0, true)),
            FieldRef::BrainWeight => match edit.selected {
                Some(Selected::Edge(key)) => {
                    (edit.edge(key)?.weight, slider_range(Slider::Weight, None))
                }
                _ => return None,
            },
            _ => return None,
        };
        Some((
            crate::value_edit::Rules {
                factor: 1.0,
                min,
                max,
                out_of_bounds,
                integer: false,
                log: false,
            },
            value,
        ))
    }

    /// A typed value of one of the panels' sliders (already divided by the factor).
    pub fn editor_brain_apply_value(&mut self, field: FieldRef, value: f32) {
        let Some(session) = self.bibite_editor.as_mut() else {
            return;
        };
        let edit = &mut session.brain;
        match (field, edit.selected) {
            (FieldRef::BrainBias, Some(Selected::Node(key))) => edit.edit_bias(key, value),
            (FieldRef::BrainAct, Some(Selected::Node(key))) => edit.edit_act(key, value),
            (FieldRef::BrainWeight, Some(Selected::Edge(key))) => edit.edit_weight(key, value),
            _ => {}
        }
    }

    /// The options of the activation function dropdown (`ChoiceSettingDropdown`: each with the function's tooltip, shown as
    /// "Activation Function: label" while the pointer rests on it) and the one the open node has.
    #[must_use]
    pub fn editor_brain_function_choices(&self) -> Option<(Vec<crate::dropdown::Choice>, usize)> {
        let session = self.bibite_editor.as_ref()?;
        let Some(Selected::Node(key)) = session.brain.selected else {
            return None;
        };
        let node = session.brain.node(key)?;
        let choices = FUNCTION_CHOICES
            .iter()
            .map(|(kind, name)| {
                let tip = crate::tooltip::intern(tooltip_text(node_info_data::of_function(
                    *kind as usize,
                )));
                crate::dropdown::Choice::of_setting("Activation Function", *name, tip)
            })
            .collect();
        let current = FUNCTION_CHOICES
            .iter()
            .position(|(kind, _)| *kind == node.kind)
            .unwrap_or(0);
        Some((choices, current))
    }

    /// A pick of the activation function dropdown: the open hidden node takes the function.
    pub fn editor_brain_pick_function(&mut self, index: usize) {
        let Some(session) = self.bibite_editor.as_mut() else {
            return;
        };
        let Some(Selected::Node(key)) = session.brain.selected else {
            return;
        };
        if let Some((kind, _)) = FUNCTION_CHOICES.get(index) {
            session.brain.edit_function(key, *kind);
        }
    }

    /// `PrepareForTemplateSave` and `SaveTemplate`: the template takes the brain as the editor has it, and the positions it keeps.
    pub fn editor_brain_into_template(&mut self) -> Result<(), String> {
        let Some(session) = self.bibite_editor.as_mut() else {
            return Ok(());
        };
        let (nodes, synapses, anchors) = session.brain.prepare_for_template_save();
        // `SaveTemplate` keeps the brain as it is: no repair of a floating hidden node or a soft latch's bias.
        let brain = bibites_core::brain::Brain::from_records_unrepaired(&nodes, &synapses)
            .map_err(|error| error.to_string())?;
        session.model.brain = brain;
        session.model.anchors = anchors
            .into_iter()
            .map(|(innovation, (x, y))| {
                (
                    innovation,
                    bibites_io::model::AnchorJson {
                        x,
                        y,
                        ..bibites_io::model::AnchorJson::default()
                    },
                )
            })
            .collect();
        if session.brain.any_change {
            session.model.any_change = true;
        }
        Ok(())
    }
}

/// `NodeEditPanel.RemoveNode` / `ConnectionEditPanel.RemoveConnection` for what is selected.
fn delete_selected(edit: &mut BrainEdit) {
    match edit.selected {
        Some(Selected::Node(key)) => edit.delete_node(key),
        Some(Selected::Edge(key)) => edit.delete_connection(key),
        None => {}
    }
}

/// `SelectElement(node, null)`: the panel of the node that was just added hangs where the node is.
fn hang_panel_on_selected_node(edit: &BrainEdit, view: &mut BrainView, canvas: Option<&Canvas>) {
    view.label_focus = false;
    let (Some(Selected::Node(key)), Some(canvas)) = (edit.selected, canvas) else {
        return;
    };
    if let Some(node) = edit.node(key) {
        view.panel_at = to_editor_space(canvas.to_screen(node.pos));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LoadedWorld;
    use std::collections::BTreeMap;

    fn blank_edit() -> BrainEdit {
        let brain = crate::template_editor::blank_brain();
        BrainEdit::fill(
            brain.to_node_records(),
            &brain.to_synapse_records(),
            &BTreeMap::new(),
        )
    }

    /// An app with the Bibite Editor open on a new template and its Brain tab showing, over a throwaway preferences folder.
    fn app_on_brain_tab(tag: &str) -> App {
        let loaded = LoadedWorld::load(None, 31, 8, 1).expect("starter world");
        let mut app = App::new(loaded, Vec2::new(1920.0, 1080.0), None);
        let dir = std::env::temp_dir().join(format!(
            "rustibites_brain_ui_{tag}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        app.storage = bibites_io::storage::StoragePaths::new(dir);
        let model = crate::template_editor::TemplateEditor::new_empty(
            crate::template_editor::blank_brain(),
            Vec::new(),
            |_| false,
        );
        app.open_template_editor(model, true);
        let session = app.bibite_editor.as_mut().expect("open");
        session.view.tab = Tab::Brain;
        app
    }

    /// Puts node `index` of `archetype` on the canvas where the window point `at` is, and selects it (the add-node panel's pick).
    fn put_node(app: &mut App, index: usize, archetype: Archetype, at: [f32; 2]) -> NodeKey {
        let canvas = app.editor_brain_canvas().expect("canvas");
        let session = app.bibite_editor.as_mut().expect("open");
        session.brain.request_add_node(canvas.to_place(at));
        session.brain.add_node(index, archetype);
        match session.brain.selected {
            Some(Selected::Node(key)) => key,
            _ => panic!("the new node is selected"),
        }
    }

    fn frame_of(app: &App) -> UiFrame {
        let mut frame = UiFrame::new(1.0);
        crate::template_editor_ui::draw(
            &mut frame,
            app,
            &crate::ui::UiAssets::empty(),
            None,
            1920.0,
            1080.0,
        );
        frame
    }

    #[test]
    fn the_toolbar_keeps_its_places_and_the_eye_moves_once_positions_are_kept() {
        let loose = toolbar_layout(false);
        assert_eq!(loose.step, 26.0);
        assert_eq!(loose.play, 86.0);
        assert_eq!(loose.eye, 976.0);
        assert_eq!(loose.keep, Some(1076.0));
        assert_eq!(loose.sort, 1176.0);
        assert_eq!(loose.info, 1276.0);
        let kept = toolbar_layout(true);
        assert_eq!(kept.keep, None);
        assert_eq!(kept.eye, 1076.0, "the eye takes the keep button's place");
        assert_eq!((kept.sort, kept.info), (1176.0, 1276.0));
    }

    #[test]
    fn the_canvas_shows_bars_in_proportion_to_what_does_not_fit() {
        // The reference's brain on a 1920 x 1080 window: 2016 x 1296 of content in a 1296 x 896 scroll view.
        let canvas = Canvas::new([2016.0, 1296.0], 1.0, [0.0, 0.0]);
        assert_eq!(canvas.bars, (true, true));
        assert_eq!(canvas.viewport, [28.0, 156.0, 1284.0, 884.0]);
        let (_, handle) = canvas.bar(true);
        // 1284 / 2016 = 0.637 of the sliding area, plus the handle's 12.
        assert!(
            (handle[2] - (1284.0 / 2016.0 * 1272.0 + 12.0)).abs() < 0.01,
            "{handle:?}"
        );
        let (_, handle) = canvas.bar(false);
        assert!(
            (handle[3] - (884.0 / 1296.0 * 872.0 + 12.0)).abs() < 0.01,
            "{handle:?}"
        );
        // Zoomed out far enough it all fits: no bars, nothing to scroll, and the pan is zero.
        let small = Canvas::new([2016.0, 1296.0], 0.5, [300.0, -40.0]);
        assert_eq!(small.bars, (false, false));
        assert_eq!(small.pan, [0.0, 0.0]);
    }

    #[test]
    fn places_and_screen_points_convert_both_ways_and_the_wheel_keeps_the_point_under_the_cursor() {
        let canvas = Canvas::new([2016.0, 1296.0], 1.0, [10.0, -20.0]);
        for place in [[0.0, 0.0], [376.0, -300.0], [1000.0, -700.0]] {
            let back = canvas.to_place(canvas.to_screen(place));
            assert!((back[0] - place[0]).abs() < 1e-3 && (back[1] - place[1]).abs() < 1e-3);
        }
        let cursor = [500.0, 400.0];
        let before = canvas.to_place(cursor);
        let (zoom, pan) = canvas.zoomed(0.1, cursor);
        assert!((zoom - 1.1).abs() < 1e-6);
        let next = Canvas::new([2016.0, 1296.0], zoom, pan);
        let after = next.to_place(cursor);
        assert!((after[0] - before[0]).abs() < 0.05 && (after[1] - before[1]).abs() < 0.05);
        // The zoom stays between the game's limits.
        assert_eq!(canvas.zoomed(100.0, cursor).0, MAX_ZOOM);
        assert!((canvas.zoomed(-100.0, cursor).0 - MIN_ZOOM).abs() < 1e-6);
    }

    #[test]
    fn a_bar_handle_dragged_to_an_end_pans_the_content_to_that_end() {
        let canvas = Canvas::new([2016.0, 1296.0], 1.0, [0.0, 0.0]);
        let reach = canvas.reach();
        // The handle at the left shows the content's left edge: the content has moved right by its reach.
        assert_eq!(canvas.pan_for_bar(true, 0.0)[0], reach[0]);
        assert_eq!(canvas.pan_for_bar(true, 1.0)[0], -reach[0]);
        // The handle at the top shows the content's top: the content has moved down.
        assert_eq!(canvas.pan_for_bar(false, 0.0)[1], -reach[1]);
        assert_eq!(canvas.pan_for_bar(false, 1.0)[1], reach[1]);
        // And the handle follows the pan the other way round.
        let moved = Canvas::new([2016.0, 1296.0], 1.0, [reach[0], -reach[1]]);
        let (track, handle) = moved.bar(true);
        assert_eq!(handle[0], track[0], "at the left end");
        let (track, handle) = moved.bar(false);
        assert_eq!(handle[1], track[1], "at the top");
    }

    #[test]
    fn the_node_under_a_point_is_picked_before_a_connection_and_the_later_node_wins() {
        let mut edit = blank_edit();
        let canvas = Canvas::new(edit.content, 1.0, [0.0, 0.0]);
        let output = edit
            .nodes
            .iter()
            .find(|node| node.archetype() == Archetype::Output)
            .map(|node| node.key)
            .expect("an output");
        edit.request_add_node(canvas.to_place([300.0, 400.0]));
        edit.add_node(0, Archetype::Input);
        let Some(Selected::Node(input)) = edit.selected else {
            panic!("the new input is selected");
        };
        edit.new_connection(Some(input), None);
        edit.click_node(output, false);
        assert_eq!(edit.edges.len(), 1);
        let from = canvas.to_screen(edit.node(input).expect("input").pos);
        let to = canvas.to_screen(edit.node(output).expect("output").pos);
        assert_eq!(pick(&edit, &canvas, from), Some(Picked::Node(input)));
        if canvas.contains(to) {
            assert_eq!(pick(&edit, &canvas, to), Some(Picked::Node(output)));
        }
        let middle = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
        if canvas.contains(middle) {
            assert!(matches!(pick(&edit, &canvas, middle), Some(Picked::Edge(_))));
        }
        // Outside the viewport nothing is picked, even where a node would be.
        assert_eq!(pick(&edit, &canvas, [5.0, 5.0]), None);
    }

    #[test]
    fn the_edit_panels_hang_from_the_click_the_way_the_game_places_them() {
        // A click in the middle of the editor's upper half: the panel is centred on it, and its bottom edge sits 75 below the point.
        let middle = [EDITOR[2] / 2.0, EDITOR[3] * 0.75];
        let rect = panel_rect(EDGE_PANEL, middle, false);
        assert!((rect[0] + rect[2] / 2.0 - (EDITOR[0] + middle[0])).abs() < 0.01);
        let point_screen_y = EDITOR[1] + EDITOR[3] - middle[1];
        assert!((rect[1] - (point_screen_y + 75.0)).abs() < 0.01, "{rect:?}");
        // At the left edge the panel starts 75 right of the point's x, at the right edge it ends 75 left of it.
        let left = panel_rect(EDGE_PANEL, [0.0, 100.0], false);
        assert!((left[0] - (EDITOR[0] + 75.0)).abs() < 0.01);
        let right = panel_rect(EDGE_PANEL, [EDITOR[2], 100.0], false);
        assert!((right[0] + right[2] - (EDITOR[0] + EDITOR[2] - 75.0)).abs() < 0.01);
        // The add-node panel goes to the side of the point instead.
        let add = panel_rect(ADD_PANEL, [EDITOR[2] * 0.25, EDITOR[3] / 2.0], true);
        assert!(
            (add[0] - (EDITOR[0] + EDITOR[2] * 0.25 + 75.0)).abs() < 0.01,
            "{add:?}"
        );
    }

    #[test]
    fn the_node_panel_is_as_tall_as_its_rows() {
        let mut edit = blank_edit();
        edit.request_add_node([300.0, -300.0]);
        edit.add_node(0, Archetype::Input);
        let Some(Selected::Node(key)) = edit.selected else {
            panic!("the input is selected");
        };
        // An input (no bias, but its value and the "can't change" note): 10 + 30 + 10 + (24.76 + 10 + 25 + 5 + 11.72 + 10 + 26) + 10.
        let input = node_panel_size(edit.node(key).expect("input"));
        assert!((input[1] - 172.48).abs() < 0.01, "{input:?}");
        assert_eq!(input[0], 380.0);
        // An output of the sigmoid function: the note and the bias.
        let output = edit
            .nodes
            .iter()
            .find(|node| node.archetype() == Archetype::Output)
            .expect("an output");
        let output = node_panel_size(output);
        assert!((output[1] - 172.48).abs() < 0.01, "{output:?}");
        // A hidden node of a non-linear function with a bias: no note, the warning and the bias row.
        edit.request_add_node([400.0, -300.0]);
        edit.add_node(NodeType::SoftLatch as usize, Archetype::Hidden);
        let Some(Selected::Node(key)) = edit.selected else {
            panic!("the hidden node is selected");
        };
        let hidden = node_panel_size(edit.node(key).expect("hidden"));
        // 10 + 30 + 10 + (24.76 + 10 + (25 + 5 + 23.44) + 10 + 26) + 10.
        assert!((hidden[1] - 184.2).abs() < 0.01, "{hidden:?}");
    }

    #[test]
    fn the_sliders_map_the_cursor_to_their_ranges() {
        assert_eq!(slider_value(Slider::Weight, None, 100.0, 200.0, 150.0), 0.0);
        assert_eq!(
            slider_value(Slider::Weight, None, 100.0, 200.0, 300.0),
            10.0
        );
        assert_eq!(
            slider_value(Slider::Weight, None, 100.0, 200.0, -50.0),
            -10.0
        );
        // A bias slider keeps to its function's range: an inhibitory node's is 0.0001 to 10, a soft latch's up to 1000.
        let mut edit = blank_edit();
        edit.add_node(NodeType::Inhibitory as usize, Archetype::Hidden);
        let Some(Selected::Node(key)) = edit.selected else {
            panic!("selected");
        };
        let node = edit.node(key).expect("node").clone();
        assert_eq!(
            slider_range(Slider::Bias, Some(&node)),
            (0.0001, 10.0, false)
        );
        assert_eq!(
            slider_value(Slider::Bias, Some(&node), 0.0, 100.0, 100.0),
            10.0
        );
        // An input's value slider takes its sense's range, within -5..=5.
        let mut inputs = blank_edit();
        inputs.request_add_node([300.0, -300.0]);
        inputs.add_node(0, Archetype::Input);
        let Some(Selected::Node(energy)) = inputs.selected else {
            panic!("the energy ratio is selected");
        };
        let energy = inputs.node(energy).expect("energy ratio").clone();
        assert_eq!(slider_range(Slider::Act, Some(&energy)), (0.0, 1.0, true));
    }

    #[test]
    fn the_add_node_panel_lists_what_is_not_on_the_canvas_and_filters_by_name() {
        let edit = blank_edit();
        let all = add_sections(&edit, "");
        // The blank brain has every input off the canvas, its outputs that are active on it.
        assert_eq!(all[0].items.len(), N_INPUTS);
        assert!(all[1].items.len() < 15, "the active outputs are on the canvas");
        assert_eq!(
            all[2].items.len(),
            NodeType::COUNT - 1,
            "every function but Input"
        );
        assert!(all.iter().all(|section| section.shown));
        // The filter is a case-insensitive part of the name.
        let filtered = add_sections(&edit, "SIGMOID");
        assert_eq!(filtered[0].items.len(), 0);
        assert_eq!(
            filtered[2].items,
            vec![(NodeType::Sigmoid as u16, "Sigmoid".to_owned())]
        );
        // A hidden function's index is its `NodeType`.
        assert_eq!(all[2].items[0].0, 1);
        // While a connection waits for its start the outputs are not offered, though they are counted.
        let mut waiting = blank_edit();
        let output = waiting
            .nodes
            .iter()
            .find(|node| node.archetype() == Archetype::Output)
            .map(|node| node.key)
            .expect("output");
        waiting.new_connection(None, Some(output));
        waiting.request_add_node([100.0, -100.0]);
        assert_eq!(waiting.add_hides, Archetype::Output);
        let offered = add_sections(&waiting, "");
        assert!(offered[0].shown && offered[2].shown);
        assert!(!offered[1].shown, "the kind the connection cannot take is hidden");
        assert!(!offered[1].items.is_empty(), "but still counted");
    }

    #[test]
    fn a_click_on_a_node_selects_it_and_a_press_that_moves_drags_it() {
        let mut app = app_on_brain_tab("click");
        let key = put_node(&mut app, 0, Archetype::Input, [700.0, 500.0]);
        // Escape takes the selection back.
        assert!(app.editor_brain_escape());
        assert!(app.bibite_editor.as_ref().expect("open").brain.selected.is_none());
        // A real click: press on the node, release on it.
        let canvas = app.editor_brain_canvas().expect("canvas");
        let place = app.bibite_editor.as_ref().expect("open").brain.node(key).expect("node").pos;
        let at = canvas.to_screen(place);
        let cursor = Vec2::new(at[0], at[1]);
        app.editor_brain_press(cursor, false);
        assert_eq!(app.editor_brain_dragging(), Some(false));
        app.editor_brain_release(cursor);
        let session = app.bibite_editor.as_ref().expect("open");
        assert_eq!(session.brain.selected, Some(Selected::Node(key)));
        let panel = open_panel_rect(&session.brain, &session.view.brain).expect("a panel");
        assert!(panel[2] > 0.0);
        // A press that moves more than 10 px is a drag: the node follows the cursor and nothing is selected afterwards.
        app.bibite_editor.as_mut().expect("open").brain.deselect();
        app.editor_brain_press(cursor, false);
        app.editor_brain_drag_to(Vec2::new(cursor.x + 5.0, cursor.y));
        assert_eq!(
            app.bibite_editor.as_ref().expect("open").brain.node(key).expect("node").pos,
            place,
            "five pixels are still a click"
        );
        let end = Vec2::new(cursor.x + 40.0, cursor.y + 30.0);
        app.editor_brain_drag_to(end);
        app.editor_brain_release(end);
        let session = app.bibite_editor.as_ref().expect("open");
        assert!(session.brain.selected.is_none(), "a drag does not click");
        let moved = session.brain.node(key).expect("node").pos;
        assert!(
            (moved[0] - place[0] - 40.0).abs() < 0.5 && (moved[1] - place[1] + 30.0).abs() < 0.5,
            "{moved:?} from {place:?}"
        );
    }

    #[test]
    fn the_right_button_pans_the_canvas_after_ten_pixels_and_a_node_does_not_take_it() {
        let mut app = app_on_brain_tab("pan");
        let canvas = app.editor_brain_canvas().expect("canvas");
        assert!(canvas.reach()[0] > 0.0);
        let empty = Vec2::new(canvas.viewport[0] + 5.0, canvas.viewport[1] + 5.0);
        app.editor_brain_press(empty, true);
        assert_eq!(app.editor_brain_dragging(), Some(true));
        // Within the threshold nothing moves.
        app.editor_brain_drag_to(Vec2::new(empty.x + 4.0, empty.y));
        assert_eq!(app.editor_brain_canvas().expect("canvas").pan, canvas.pan);
        // Past it the canvas follows from where the threshold was passed.
        app.editor_brain_drag_to(Vec2::new(empty.x + 20.0, empty.y));
        app.editor_brain_drag_to(Vec2::new(empty.x + 50.0, empty.y));
        let pan = app.editor_brain_canvas().expect("canvas").pan;
        assert!((pan[0] - canvas.pan[0] - 30.0).abs() < 0.01, "{pan:?}");
        app.editor_brain_release(Vec2::new(empty.x + 50.0, empty.y));
        // A node under the press keeps the pan from starting.
        let canvas = app.editor_brain_canvas().expect("canvas");
        let key = put_node(&mut app, 0, Archetype::Input, [600.0, 500.0]);
        let place = app.bibite_editor.as_ref().expect("open").brain.node(key).expect("node").pos;
        let at = canvas.to_screen(place);
        app.editor_brain_press(Vec2::new(at[0], at[1]), true);
        assert_eq!(app.editor_brain_dragging(), None);
    }

    #[test]
    fn the_wheel_zooms_unless_something_is_selected_and_scrolls_the_lists() {
        let mut app = app_on_brain_tab("wheel");
        let cursor = Vec2::new(600.0, 600.0);
        assert!(app.editor_brain_wheel(cursor, 1.0));
        let zoom = app.bibite_editor.as_ref().expect("open").view.brain.zoom;
        assert!((zoom - 1.1).abs() < 1e-5, "{zoom}");
        // With a node selected the wheel is left alone (the game's `elementSelected`).
        put_node(&mut app, 0, Archetype::Input, [600.0, 500.0]);
        app.editor_brain_wheel(cursor, 1.0);
        assert!((app.bibite_editor.as_ref().expect("open").view.brain.zoom - zoom).abs() < 1e-6);
        // The add-node panel scrolls its list by 25 a notch instead.
        let session = app.bibite_editor.as_mut().expect("open");
        session.brain.deselect();
        session.brain.request_add_node([500.0, -300.0]);
        session.view.brain.panel_at = to_editor_space([600.0, 600.0]);
        let panel = open_panel_rect(&session.brain, &session.view.brain).expect("panel");
        let inside_panel = Vec2::new(panel[0] + 50.0, panel[1] + 100.0);
        assert!(app.editor_brain_wheel(inside_panel, -1.0));
        let view = &app.bibite_editor.as_ref().expect("open").view.brain;
        assert_eq!(view.add_scroll, 25.0);
        assert!((view.zoom - zoom).abs() < 1e-6, "no zoom under the panel");
    }

    #[test]
    fn the_tab_draws_its_toolbar_canvas_and_the_panel_of_the_selection() {
        let mut app = app_on_brain_tab("draw");
        let frame = frame_of(&app);
        let ids: Vec<UiId> = frame.hits.iter().map(|(id, _, _)| *id).collect();
        for id in [
            HIT_CANVAS,
            HIT_TOOLBAR,
            HIT_TOOLBAR + 1,
            HIT_TOOLBAR + 2,
            HIT_TOOLBAR + 3,
            HIT_TOOLBAR + 4,
            HIT_TOOLBAR + 5,
        ] {
            assert!(ids.contains(&id), "hit {id} is drawn");
        }
        // A selected hidden node shows its panel with the label, the dropdown and the bias slider.
        put_node(&mut app, NodeType::Sigmoid as usize, Archetype::Hidden, [700.0, 500.0]);
        let frame = frame_of(&app);
        let actions: Vec<UiAction> = frame.actions.iter().map(|(_, action)| *action).collect();
        assert!(actions.contains(&UiAction::DropdownOpen(DropdownTarget::NodeFunction)));
        assert!(actions.contains(&UiAction::Editor(EditorAction::Brain(
            BrainAction::FocusLabel
        ))));
        assert!(actions.contains(&UiAction::Editor(EditorAction::Brain(
            BrainAction::DeleteSelected
        ))));
        assert!(actions.iter().any(|action| matches!(
            action,
            UiAction::Editor(EditorAction::Brain(BrainAction::Slide(Slider::Bias, ..)))
        )));
    }

    #[test]
    fn the_function_dropdown_changes_a_hidden_nodes_function_and_its_bias() {
        let mut app = app_on_brain_tab("function");
        put_node(&mut app, NodeType::Sigmoid as usize, Archetype::Hidden, [700.0, 500.0]);
        let (choices, current) = app.editor_brain_function_choices().expect("choices");
        assert_eq!(choices.len(), 13);
        assert_eq!(choices[current].label, "Sigmoid");
        assert_eq!(choices[0].title, "Activation Function: Linear");
        let soft = FUNCTION_CHOICES
            .iter()
            .position(|(kind, _)| *kind == NodeType::SoftLatch)
            .expect("soft latch");
        app.editor_brain_pick_function(soft);
        let session = app.bibite_editor.as_ref().expect("open");
        let Some(Selected::Node(key)) = session.brain.selected else {
            panic!("selected");
        };
        let node = session.brain.node(key).expect("node");
        assert_eq!(node.kind, NodeType::SoftLatch);
        // `EditFunction` gives the function's own bias (5), then `CheckTypeForBiasSlider` forces the slider into the soft latch's
        // range with the value the panel had (0 of the sigmoid node that was selected), which edits the bias again.
        assert_eq!(node.bias, 0.0001);
        // A function whose slider is not forced keeps its own bias; the panel's bias row goes on showing (and typing from) the old one.
        let mult = FUNCTION_CHOICES
            .iter()
            .position(|(kind, _)| *kind == NodeType::Mult)
            .expect("mult");
        app.editor_brain_pick_function(mult);
        let session = app.bibite_editor.as_ref().expect("open");
        assert_eq!(session.brain.node(key).expect("node").bias, 1.0);
        let (rules, shown) = app
            .editor_brain_typed_rules(FieldRef::BrainBias)
            .expect("rules");
        assert_eq!(shown, 0.0001, "the panel's own value");
        assert_eq!(
            (rules.min, rules.max, rules.out_of_bounds),
            (-5.0, 5.0, true)
        );
    }

    #[test]
    fn typing_goes_to_the_focused_field_and_enter_adds_the_single_match() {
        let mut app = app_on_brain_tab("typing");
        let mut input = FrameInput {
            cursor: Vec2::new(600.0, 600.0),
            toggle_pause: true,
            ..FrameInput::default()
        };
        // The character of the key that opens the panel is in the frame's typed text too.
        app.typed = " ".to_owned();
        app.editor_brain_frame(&input);
        let session = app.bibite_editor.as_ref().expect("open");
        assert!(
            session.brain.add_mode && session.view.brain.filter_focus,
            "Space opens the add-node panel with the filter focused"
        );
        assert!(app.typed.is_empty(), "and is not the filter's first character");
        app.typed = "tanh".to_owned();
        input = FrameInput::default();
        assert!(app.editor_brain_type(&input));
        assert_eq!(
            app.bibite_editor.as_ref().expect("open").view.brain.filter,
            "tanh"
        );
        // One hidden function matches "tanh" and nothing else does: Enter adds it.
        input.enter = true;
        app.editor_brain_frame(&input);
        let session = app.bibite_editor.as_ref().expect("open");
        assert!(!session.brain.add_mode, "the panel closed");
        let Some(Selected::Node(key)) = session.brain.selected else {
            panic!("the new node is selected");
        };
        assert_eq!(session.brain.node(key).expect("node").kind, NodeType::TanH);
        // Space again with the node selected does nothing (`elementSelected`).
        let again = FrameInput {
            cursor: Vec2::new(600.0, 600.0),
            toggle_pause: true,
            ..FrameInput::default()
        };
        app.editor_brain_frame(&again);
        assert!(!app.bibite_editor.as_ref().expect("open").brain.add_mode);
        // Delete removes the selected node.
        let delete = FrameInput {
            kill_target: true,
            ..FrameInput::default()
        };
        app.editor_brain_frame(&delete);
        assert!(app.bibite_editor.as_ref().expect("open").brain.selected.is_none());
    }

    #[test]
    fn typed_values_follow_the_sliders_rules() {
        let mut app = app_on_brain_tab("values");
        put_node(&mut app, NodeType::Inhibitory as usize, Archetype::Hidden, [700.0, 500.0]);
        let (rules, value) = app.editor_brain_typed_rules(FieldRef::BrainBias).expect("rules");
        assert_eq!(value, 1.0);
        assert_eq!(
            (rules.min, rules.max, rules.out_of_bounds),
            (0.0001, 10.0, false)
        );
        assert_eq!(
            rules.submitted("50"),
            Some(10.0),
            "an inhibitory bias cannot leave its range"
        );
        app.editor_brain_apply_value(FieldRef::BrainBias, 3.5);
        let session = app.bibite_editor.as_ref().expect("open");
        let Some(Selected::Node(key)) = session.brain.selected else {
            panic!("selected");
        };
        assert_eq!(session.brain.node(key).expect("node").bias, 3.5);
        assert!(
            app.editor_brain_typed_rules(FieldRef::BrainWeight).is_none(),
            "no connection is selected"
        );
        // An input's value: the slider's track is the input's own range, but a typed value has the setting's -5 ..= 5 and may leave it.
        let input = put_node(&mut app, 0, Archetype::Input, [700.0, 400.0]);
        let node = app
            .bibite_editor
            .as_ref()
            .expect("open")
            .brain
            .node(input)
            .cloned();
        let (track_low, track_high, _) = slider_range(Slider::Act, node.as_ref());
        let (rules, _) = app.editor_brain_typed_rules(FieldRef::BrainAct).expect("rules");
        assert_eq!(
            (rules.min, rules.max, rules.out_of_bounds),
            (-5.0, 5.0, true)
        );
        assert!(
            track_low > -5.0 || track_high < 5.0,
            "an input whose range is narrower than -5 ..= 5 ({track_low}, {track_high})"
        );
        assert_eq!(rules.submitted("50"), Some(50.0));
    }

    #[test]
    fn escape_gives_up_the_label_field_and_undoes_the_newest_thing_in_one_press() {
        let mut app = app_on_brain_tab("escape");
        let key = put_node(&mut app, NodeType::Linear as usize, Archetype::Hidden, [700.0, 500.0]);
        app.editor_brain_action(BrainAction::FocusLabel);
        assert!(app.bibite_editor.as_ref().expect("open").view.brain.label_focus);
        app.typed = " two".to_owned();
        assert!(app.editor_brain_type(&FrameInput::default()));
        let label = |app: &App| {
            app.bibite_editor
                .as_ref()
                .expect("open")
                .brain
                .node(key)
                .expect("node")
                .desc
                .clone()
        };
        assert_eq!(label(&app), "Hidden Node two");
        // `TMP_InputField` cancels: the text from before; and `UINavigationManager` pops the selection from its stack at once.
        assert!(app.editor_brain_escape());
        let session = app.bibite_editor.as_ref().expect("open");
        assert!(!session.view.brain.label_focus);
        assert!(session.brain.selected.is_none());
        assert_eq!(label(&app), "Hidden Node");
        // Enter keeps what was typed, and the node stays selected.
        app.bibite_editor
            .as_mut()
            .expect("open")
            .brain
            .click_node(key, false);
        app.editor_brain_action(BrainAction::FocusLabel);
        app.typed = "!".to_owned();
        app.editor_brain_type(&FrameInput {
            enter: true,
            ..FrameInput::default()
        });
        assert_eq!(label(&app), "Hidden Node!");
        let session = app.bibite_editor.as_ref().expect("open");
        assert!(!session.view.brain.label_focus);
        assert_eq!(session.brain.selected, Some(Selected::Node(key)));
    }

    #[test]
    fn pressing_anything_but_a_text_field_takes_the_keyboard_off_the_fields() {
        let mut app = app_on_brain_tab("focus");
        put_node(&mut app, NodeType::Linear as usize, Archetype::Hidden, [700.0, 500.0]);
        let focus = UiAction::Editor(EditorAction::Brain(BrainAction::FocusLabel));
        app.apply_ui_actions(&[focus]);
        assert!(app.bibite_editor.as_ref().expect("open").view.brain.label_focus);
        app.apply_ui_actions(&[focus]);
        assert!(
            app.bibite_editor.as_ref().expect("open").view.brain.label_focus,
            "a press on the field itself keeps the focus"
        );
        app.apply_ui_actions(&[UiAction::Consume]);
        assert!(!app.bibite_editor.as_ref().expect("open").view.brain.label_focus);
    }

    #[test]
    fn a_pan_follows_the_cursors_whole_way_from_where_it_began_inside_the_limits() {
        let mut app = app_on_brain_tab("panorigin");
        let canvas = app.editor_brain_canvas().expect("canvas");
        let limit = canvas.clamped([1.0e6, canvas.pan[1]]);
        assert!(limit[0] > canvas.pan[0], "the canvas can be moved right");
        let empty = Vec2::new(canvas.viewport[0] + 5.0, canvas.viewport[1] + 5.0);
        app.editor_brain_press(empty, true);
        // The drag begins 20 px on; the cursor then goes far past the end of the canvas...
        app.editor_brain_drag_to(Vec2::new(empty.x + 20.0, empty.y));
        let way = 100_000.0;
        app.editor_brain_drag_to(Vec2::new(empty.x + 20.0 + way, empty.y));
        assert_eq!(app.editor_brain_canvas().expect("canvas").pan[0], limit[0]);
        // ...and a little back does not move it: the content is where it started plus the whole way, kept inside its limits.
        app.editor_brain_drag_to(Vec2::new(empty.x + 20.0 + way - 50.0, empty.y));
        assert_eq!(app.editor_brain_canvas().expect("canvas").pan[0], limit[0]);
        // Back at the start of the drag the canvas is where it began; before it, it moves the other way.
        app.editor_brain_drag_to(Vec2::new(empty.x + 20.0, empty.y));
        assert!((app.editor_brain_canvas().expect("canvas").pan[0] - canvas.pan[0]).abs() < 0.01);
        app.editor_brain_drag_to(Vec2::new(empty.x + 10.0, empty.y));
        let pan = app.editor_brain_canvas().expect("canvas").pan[0];
        assert_eq!(
            pan,
            canvas.clamped([canvas.pan[0] - 10.0, canvas.pan[1]])[0]
        );
    }

    #[test]
    fn the_template_takes_the_edited_brain_and_its_kept_positions() {
        let mut app = app_on_brain_tab("template");
        let input = put_node(&mut app, 0, Archetype::Input, [700.0, 500.0]);
        let session = app.bibite_editor.as_mut().expect("open");
        let output = session
            .brain
            .nodes
            .iter()
            .find(|node| node.archetype() == Archetype::Output)
            .map(|node| node.key)
            .expect("output");
        session.brain.new_connection(Some(input), None);
        session.brain.click_node(output, false);
        assert!(session.brain.is_valid());
        session.brain.set_save_anchors(true);
        app.editor_brain_into_template().expect("the brain builds");
        let model = &app.bibite_editor.as_ref().expect("open").model;
        assert_eq!(model.brain.synapse_count(), 1);
        assert!(!model.anchors.is_empty(), "the kept positions are saved");
        assert!(model.any_change);
    }

    #[test]
    fn the_notice_over_the_preview_says_what_happens_to_the_positions() {
        let assets = crate::ui::UiAssets::empty();
        let d = Drawer::new(1.0, &assets, None, Vec2::ZERO);
        let mut frame = UiFrame::new(1.0);
        saved_notice(&mut frame, &d, false, [1360.0, 78.0, 548.0, 392.0]);
        let texts: Vec<&str> = frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            texts,
            ["*Nodes placement", "WILL NOT", "be saved with template*"]
        );
        let mut kept = UiFrame::new(1.0);
        saved_notice(&mut kept, &d, true, [1360.0, 78.0, 548.0, 392.0]);
        assert!(kept.draws.iter().any(|draw| matches!(
            draw,
            UiDraw::Text { text, color, .. } if text == "WILL" && *color == [10, 217, 0, 255]
        )));
    }
}

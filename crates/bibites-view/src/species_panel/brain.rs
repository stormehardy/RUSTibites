//! The Brain tab of the species panels (`SpeciesBrainPanel`, a `BrainPanelCommon`): the template brain of the
//! species as a diagram, its sizes, how far it is from the parent's and from a compared species' brain, and the
//! differences with that one highlighted.
//!
//! Prefab values (level2, `SpeciesBrainPanel`; `UINeuron` and `UISynaps` from sharedassets2): a `base_icon`
//! panel (0.84 grey) over a vertical layout with padding 10 / 10 / 8 / 10; the 101.7 high header (the name,
//! Roboto-Bold 20; the "Parent species:" and "Compared species:" rows of the Genes tab; the info row "Brain
//! Diagram:" with "Nodes :" and "Connections:" counts, Roboto-Bold 18); the body (padding 25, the holder the
//! nodes and synapses live in, at least 300 high); the bottom line ("Highlight differences" toggle, Roboto-Bold 16,
//! and the Sort, Save pos. and Reset pos. buttons 80 / 80 / 85 wide, LiberationSans 14). The panel is as wide as
//! its widest row (445.17 + 20) or the holder (`width` + 70), and as tall as its parts.
//!
//! A neuron is 35 x 35 scaled by 1.28571 (45 px): the `small_circle_base_icon` tinted by its value, an icon inset
//! by 8, a plus or minus (11 x 11) at the top right for a node with a bias, and its name (LiberationSans 14,
//! white) above it when "Show Node Names" is on. A synapse is a strip from its source node's middle to its target's,
//! as long as the distance and `10 ..= 16` thick by its weight (`(e^(|w|/2) - 1) / (1 + e^(|w|/2)) 6 + 10`),
//! tiled with the `synapse_0` sprite (12 x 7, one tile as high as the strip) and tinted green for a positive and
//! red for a negative weight, white at 0.25 when disabled. Dragging a neuron moves it inside the holder.
//!
//! Not the game's: the synapse shader's animation, the highlight of a difference (a yellow halo of the amount the
//! game gives the shader), the weight text of a hovered synapse is not turned along the strip, the random numbers
//! of the sorter (`bibites-brainsort`'s generator is seeded by the species) and the nodes' value on hover (the
//! species panel does not show it either).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bibites_brainsort::{Archetype, N_INPUTS, N_OUTPUTS, Rng};
use bibites_core::brain::NodeType;
use bibites_core::ids::SpeciesId;
use bibites_core::math::Vec2;
use bibites_core::species::SpeciesListEntry;

use super::{BLACK, Drawer, FRAME_GREY, HeaderRects, INK, OVERLAY, WHITE};
use crate::app::App;
use crate::brain_layout::{Diagram, Graph, START_HOLDER_WIDTH};
use crate::tooltip::{self, Tip, TipRect};
use crate::ui::{FontRole, Rgba, UiAction, UiAssets, UiDraw, UiFrame, UiTexture};

/// Hit ids of the toggle and the three buttons (8950 - 8999 belong to the species panels) and of the nodes.
const HIT_TOGGLE: u32 = 8992;
const HIT_SORT: u32 = 8993;
const HIT_NODE: u32 = 900_000;
/// The panel's smallest inner width: the bottom line's.
const MIN_INNER: f32 = 445.17;
/// The holder's smallest height (`Holder` is 300 high).
const MIN_HOLDER: f32 = 300.0;
/// A neuron: 35 x 35 scaled by 1.28571.
pub(crate) const NODE: f32 = 35.0 * 1.285_71;
/// The part of a neuron the icon takes: 19 of 35.
pub(crate) const ICON: f32 = 19.0 * 1.285_71;
/// The plus and minus on a neuron.
pub(crate) const SIGN: f32 = 11.0 * 1.285_71;
/// The yellow of a highlighted difference.
pub(crate) const HIGHLIGHT: Rgba = [251, 255, 0, 255];
/// The icons of the 48 inputs and outputs, then of the 13 activation functions (`BrainIconHolder`).
const NODE_ICONS: [&str; 48] = [
    "input_energy_ratio",
    "input_maturity",
    "input_life_ratio",
    "input_fullness",
    "input_speed",
    "input_rotation_speed",
    "input_is_grabbing_objects",
    "input_attacked_damage",
    "input_eggstored",
    "input_bb_conc_weight",
    "input_bb_conc_angle",
    "input_n_visible_bb",
    "input_plant_conc_weight",
    "input_plant_conc_angle",
    "input_n_visible_plant",
    "input_meat_conc_weight",
    "input_meat_conc_angle",
    "input_n_visible_meat",
    "input_closest_bb_r",
    "input_closest_bb_g",
    "input_closest_bb_b",
    "input_tic",
    "input_minute",
    "input_time_alive",
    "input_pherosense1",
    "input_pherosense2",
    "input_pherosense3",
    "input_phero1_angle",
    "input_phero2_angle",
    "input_phero3_angle",
    "input_phero1_heading",
    "input_phero2_heading",
    "input_phero3_heading",
    "output_accelerate",
    "output_rotate",
    "output_herding",
    "output_eggProduction",
    "output_want2lay",
    "output_want2eat",
    "output_digestion",
    "output_grab",
    "output_clk_reset",
    "output_pher_out1",
    "output_pher_out2",
    "output_pher_out3",
    "output_want2grow",
    "output_want2heal",
    "output_want2attack",
];
/// `brainFunctionsIcons[type - 1]`.
const FUNCTION_ICONS: [&str; 13] = [
    "hidden_sigmoid",
    "hidden_linear",
    "hidden_tanh",
    "hidden_sine",
    "hidden_relu",
    "hidden_gaussian",
    "hidden_latch",
    "hidden_diff",
    "hidden_abs",
    "hidden_mult",
    "integrator",
    "hidden_adaptor2",
    "hidden_softlatch_0",
];
/// `synapse_0` (12 x 7) as alpha, for when the sprite is not loaded: a bar with a notch like a `>` in it.
const SYNAPSE_PATTERN: [[u8; 12]; 7] = [
    [0; 12],
    [255, 255, 255, 255, 0, 0, 255, 255, 255, 255, 255, 255],
    [255, 255, 255, 255, 255, 0, 0, 255, 255, 255, 255, 255],
    [255, 255, 255, 255, 255, 255, 0, 0, 255, 255, 255, 255],
    [255, 255, 255, 255, 255, 0, 0, 255, 255, 255, 255, 255],
    [255, 255, 255, 255, 0, 0, 255, 255, 255, 255, 255, 255],
    [0; 12],
];

/// The icon of node `index` of a brain whose hidden node has `function`.
#[must_use]
pub fn icon_name(index: usize, function: NodeType) -> &'static str {
    if index < NODE_ICONS.len() {
        NODE_ICONS[index]
    } else {
        FUNCTION_ICONS[(function as usize).clamp(1, FUNCTION_ICONS.len()) - 1]
    }
}

// ------------------------------------------------------------------------------------------- geometry

/// Where the pieces of the tab are (reference px, `x, y, w, h`).
#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    /// The panel.
    pub panel: [f32; 4],
    /// The name and the parent and compared rows.
    pub header: HeaderRects,
    /// "Brain Diagram:".
    pub info_title: [f32; 4],
    /// "Nodes :" and its count.
    pub nodes_title: [f32; 4],
    /// See `nodes_title`.
    pub nodes_value: [f32; 4],
    /// "Connections: " and its count.
    pub connections_title: [f32; 4],
    /// See `connections_title`.
    pub connections_value: [f32; 4],
    /// The holder the nodes live in (its top left is the nodes' origin).
    pub holder: [f32; 4],
    /// The "Highlight differences" toggle's box and its text.
    pub toggle_box: [f32; 4],
    /// See `toggle_box`.
    pub toggle_text: [f32; 4],
    /// The Sort, Save pos. and Reset pos. buttons.
    pub buttons: [[f32; 4]; 3],
}

/// The tab's layout for a holder `holder_width` wide and `height` high (the layout's `height`), with the counts
/// `nodes_width` and `connections_width` wide.
#[must_use]
pub fn geometry(
    holder_width: f32,
    height: f32,
    nodes_width: f32,
    connections_width: f32,
) -> Geometry {
    geometry_for(holder_width, height, nodes_width, connections_width, true)
}

/// [`geometry`] for a species with or without a parent: a root species' parent row has no button (just "None"), so it is 16.41 high instead
/// of 20.41 and everything under it is 4 higher (the game's header is 97.7 high then).
#[must_use]
pub fn geometry_for(
    holder_width: f32,
    height: f32,
    nodes_width: f32,
    connections_width: f32,
    has_parent: bool,
) -> Geometry {
    let drop = if has_parent { 0.0 } else { 4.0 };
    let (px, py) = (OVERLAY.x, OVERLAY.y);
    let holder_width = holder_width.max(MIN_INNER - 50.0);
    let inner = holder_width + 50.0;
    let (x, top) = (px + 10.0, py + 8.0);
    let holder_height = height.max(MIN_HOLDER);
    let body = top + 101.7 - drop;
    let bottom = body + 50.0 + holder_height;
    let right = x + inner;
    let row = |at: f32| top + at;
    let info = row(73.26 - drop) + 5.0;
    let sign = |at: f32| [at, info + (23.44 - 21.1) / 2.0, 0.0, 21.1];
    let nodes_x = x + 153.52;
    let nodes_title = [nodes_x, sign(0.0)[1], 61.31, 21.1];
    let connections_w = 106.42 + 5.0 + connections_width;
    let connections_x = right - connections_w;
    let mut connections_title = sign(connections_x);
    connections_title[2] = 106.42;
    let button = |left: f32, width: f32| [left, bottom, width, 20.01];
    let reset = button(right - 85.0, 85.0);
    let save = button(reset[0] - 5.0 - 80.0, 80.0);
    let sort = button(save[0] - 5.0 - 80.0, 80.0);
    Geometry {
        panel: [
            px,
            py,
            inner + 20.0,
            8.0 + 101.7 - drop + 50.0 + holder_height + 20.01 + 10.0,
        ],
        header: HeaderRects {
            name: [x, top, inner, 23.44],
            parent_title: [x, row(26.44), 150.0, 20.41 - drop],
            parent_button: [
                x + 150.0,
                row(26.44),
                if has_parent {
                    inner - 5.0 - 150.0 - 80.0
                } else {
                    inner - 5.0 - 150.0
                },
                20.41 - drop,
            ],
            parent_distance: [right - 5.0 - 80.0, row(26.44), 80.0, 20.41],
            compared_title: [x, row(49.85 - drop), 150.0, 20.41],
            compared_dropdown: [x + 150.0, row(49.85 - drop), inner - 5.0 - 150.0 - 80.0, 20.41],
            compared_distance: [right - 5.0 - 80.0, row(49.85 - drop), 80.0, 20.41],
        },
        info_title: [x, info, 153.52, 23.44],
        nodes_title,
        nodes_value: [nodes_x + 61.31 + 5.0, nodes_title[1], nodes_width, 21.1],
        connections_title,
        connections_value: [
            connections_x + 106.42 + 5.0,
            connections_title[1],
            connections_width,
            21.1,
        ],
        holder: [x + 25.0, body + 25.0, holder_width, holder_height],
        toggle_box: [x, bottom, 20.0, 20.01],
        toggle_text: [x + 30.0, bottom, 150.17, 20.01],
        buttons: [sort, save, reset],
    }
}

// --------------------------------------------------------------------------------------------- state

/// What a difference with the compared species does to a synapse or a node.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Mark {
    /// Nothing.
    #[default]
    None,
    /// The compared species lacks it (`SetNew`).
    New {
        /// `_HighlightAmount`.
        amount: f32,
        /// `1 / nMaxConnections`.
        distance: f32,
        /// What it contributes to speciation: the distance over the species span.
        contribution: f32,
    },
    /// Enabled in one and disabled in the other (`SetToggled`).
    Toggled {
        /// `_HighlightAmount`.
        amount: f32,
        /// `1 / nCommonConnections`.
        distance: f32,
        /// What it contributes to speciation.
        contribution: f32,
    },
    /// A weight or a bias that differs (`SetDiff`).
    Changed {
        /// `_HighlightAmount`: `sqrt(|delta| / maxDelta)`.
        amount: f32,
        /// How much the compared species' value is off (`+` when it is higher).
        delta: f32,
        /// The distance this adds.
        distance: f32,
        /// What it contributes to speciation.
        contribution: f32,
    },
}

impl Mark {
    /// The highlight's amount (0 for none).
    #[must_use]
    pub fn amount(&self) -> f32 {
        match *self {
            Self::None => 0.0,
            Self::New { amount, .. }
            | Self::Toggled { amount, .. }
            | Self::Changed { amount, .. } => amount,
        }
    }

    /// The text of the tooltip a marked synapse has: its title and its lines.
    #[must_use]
    pub fn synapse_tip(&self) -> Option<(&'static str, String)> {
        match *self {
            Self::None => None,
            Self::New {
                distance,
                contribution,
                ..
            } => Some((
                "New from compared",
                format!(
                    "This is a new Connection\nResulting Genetic Distance: {distance:.3}\nContribution to speciation: +{:.1}%",
                    100.0 * contribution
                ),
            )),
            Self::Toggled {
                distance,
                contribution,
                ..
            } => Some((
                "Toggled from compared",
                format!(
                    "Connection was Toggled (either disabled or re-enabled)\nResulting Genetic Distance: {distance:.3}\nContribution to speciation: +{:.1}%",
                    100.0 * contribution
                ),
            )),
            Self::Changed {
                delta,
                distance,
                contribution,
                ..
            } => Some((
                "Weight Difference from compared",
                format!(
                    "Delta: {}{}\nResulting Genetic Distance: {distance:.3}\nContribution to speciation: +{:.1}%",
                    if delta > 0.0 { "+" } else { "" },
                    crate::value_edit::dotnet_float_string(delta),
                    100.0 * contribution
                ),
            )),
        }
    }
}

/// The marks of the differences with the compared species, per synapse and per node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marks {
    /// By the synapse's index in the brain.
    pub synapses: Vec<Mark>,
    /// By the node's index in the brain.
    pub nodes: Vec<Mark>,
}

/// What the tab was filled with.
#[derive(Clone, Debug)]
pub struct State {
    /// The species the panel shows.
    pub species: SpeciesId,
    /// The layout.
    pub diagram: Diagram,
    /// The differences with the compared species (empty while they are not shown).
    pub marks: Marks,
    /// Tells a new picture from the last one, for the drawing cache.
    serial: u64,
}

/// A neuron is held: which, and where in it the cursor took it (reference px from its middle).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drag {
    /// The node's index.
    pub node: usize,
    /// The cursor's offset from the node's middle when it was grabbed.
    pub grab: [f32; 2],
}

static SERIAL: AtomicU64 = AtomicU64::new(1);

/// A number no layout had before (a layout's picture is cached under it).
pub(crate) fn next_serial() -> u64 {
    SERIAL.fetch_add(1, Ordering::Relaxed)
}

impl App {
    /// `SpeciesBrainPanel.SelectSpecies` / `FillPanel`: the layout of the species' template brain (by the saved
    /// positions when there are enough of them, else by the sorter) and the differences with the compared species.
    pub fn brain_fill(&mut self, id: SpeciesId) {
        let Some(record) = self.world.species.get(id) else {
            self.brain = None;
            return;
        };
        let graph = Graph::of(&record.template.brain.graph_view());
        let scale = UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        // `(1080 - 350 totalScale) / UIScale`, the UI scale being the user's (1 by default).
        let max_height = 1080.0 - 350.0 * scale;
        let anchors = self.brain_anchors.get(&id);
        let mut rng = Rng::new(u64::from(id.value()) + 1);
        let diagram = Diagram::fill(
            graph,
            anchors,
            self.brain_holder_width,
            max_height,
            &mut rng,
        );
        // The holder keeps the width it was given: it only ever grows (`holder.preferredWidth = width`).
        self.brain_holder_width = diagram.width.max(START_HOLDER_WIDTH);
        self.brain = Some(State {
            species: id,
            diagram,
            marks: Marks::default(),
            serial: SERIAL.fetch_add(1, Ordering::Relaxed),
        });
        self.brain_update_marks();
    }

    /// `CallShowDifferences`: the marks of the differences with the compared species, when they are shown.
    pub fn brain_update_marks(&mut self) {
        let Some(state) = self.brain.as_ref() else {
            return;
        };
        let marks = if self.show_brain_differences {
            self.brain_marks(state.species)
        } else {
            Marks::default()
        };
        if let Some(state) = self.brain.as_mut() {
            state.marks = marks;
            state.serial = SERIAL.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// `SpeciesBrainPanel.CallShowDifferences` -> `ShowDifferences`: how the species' brain differs from the
    /// compared species' (the differences are the compared species' own view of the shown species).
    fn brain_marks(&self, id: SpeciesId) -> Marks {
        let (Some(target), Some(compared)) = (
            self.world.species.get(id),
            self.species_compared
                .and_then(|compared| self.world.species.get(compared)),
        ) else {
            return Marks::default();
        };
        marks_between(
            &compared.template.brain,
            &target.template.brain,
            self.world.species.current_used_span(&self.world.settings),
        )
    }

    /// Whether species `id` has a parent (a root species' Brain header is 4 shorter).
    pub(crate) fn brain_has_parent(&self, id: SpeciesId) -> bool {
        self.lineage_species
            .iter()
            .find(|entry| entry.id == id)
            .is_some_and(|entry| entry.parent.is_some())
    }

    /// The Sort button (`SortN()`).
    pub fn brain_sort(&mut self) {
        self.brain_sort_n(1);
    }

    /// `SortN(n)`: the sorter runs `n` more times (with new random numbers) and the nodes are placed by it.
    pub fn brain_sort_n(&mut self, times: usize) {
        let Some(state) = self.brain.as_mut() else {
            return;
        };
        self.brain_sorts += 1;
        let seed = (u64::from(state.species.value()) + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ self.brain_sorts;
        state.diagram.sort_n(times, &mut Rng::new(seed));
        state.serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    }

    /// `SortHiddenOnce` (F2 held): one step of the hidden nodes' physics of `dt` seconds.
    pub fn brain_sort_hidden(&mut self, dt: f32, scalar: f32) {
        let Some(state) = self.brain.as_mut() else {
            return;
        };
        state.diagram.sort_hidden_once(dt, scalar);
        state.serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    }

    /// The Save pos. button (`SaveNodePositions`): the nodes' places are kept in the species' template.
    pub fn brain_save_positions(&mut self) {
        let Some(state) = self.brain.as_ref() else {
            return;
        };
        let anchors = self.brain_anchors.entry(state.species).or_default();
        state.diagram.save_positions(anchors);
    }

    /// The Reset pos. button (`ResetNodePositions`): the saved places are forgotten and the panel is filled again.
    pub fn brain_reset_positions(&mut self) {
        let Some(state) = self.brain.as_ref() else {
            return;
        };
        let id = state.species;
        self.brain_anchors.remove(&id);
        self.brain_fill(id);
    }

    /// The "Highlight differences" toggle (`UpdateShowDifferences`).
    pub fn brain_toggle_differences(&mut self) {
        self.show_brain_differences = !self.show_brain_differences;
        self.brain_update_marks();
        // The setting is shared (`UserSettings.showBrainDifferences`): the inspector's Brain tab follows it too.
        self.inspector_brain_update_marks();
    }

    /// A neuron was pressed: it follows the cursor from now on (`UINeuron.OnDrag`).
    pub fn brain_press_node(&mut self, node: usize, cursor: Vec2) {
        let Some(state) = self.brain.as_ref() else {
            return;
        };
        let Some(at) = state.diagram.position(node) else {
            return;
        };
        let scale = UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let holder = geometry_for(
            self.brain_holder_width,
            state.diagram.height,
            0.0,
            0.0,
            self.brain_has_parent(state.species),
        )
        .holder;
        let (x, y) = (cursor.x / scale, cursor.y / scale);
        self.brain_drag = Some(Drag {
            node,
            grab: [x - (holder[0] + at[0]), y - (holder[1] - at[1])],
        });
    }

    /// The held neuron follows the cursor, kept inside the holder, snapped to its step with Shift.
    pub fn brain_drag_to(&mut self, cursor: Vec2, snap: bool) {
        let has_parent = self
            .brain
            .as_ref()
            .is_some_and(|state| self.brain_has_parent(state.species));
        let (Some(drag), Some(state)) = (self.brain_drag, self.brain.as_mut()) else {
            return;
        };
        let scale = UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let holder = geometry_for(
            self.brain_holder_width,
            state.diagram.height,
            0.0,
            0.0,
            has_parent,
        )
        .holder;
        let (x, y) = (
            cursor.x / scale - drag.grab[0],
            cursor.y / scale - drag.grab[1],
        );
        // The node's middle is at `holder + (at.x, -at.y)`: the game keeps the cursor's own place in the holder.
        let at = [x - holder[0], holder[1] - y];
        let size = [holder[2], holder[3]];
        state.diagram.drag_node(drag.node, at, size, snap);
        state.serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    }
}

/// `BrainPanelCommon.ShowDifferences` (`is1` false): how the brain `shown` differs from `compared`, as the marks of its synapses and
/// nodes. `span` is the used species span (`GlobalLineageManager.usedSpeciesSpan`).
pub(crate) fn marks_between(
    compared: &bibites_core::brain::Brain,
    shown: &bibites_core::brain::Brain,
    span: f32,
) -> Marks {
    let diff = compared.differences(shown);
    let view = shown.graph_view();
    let synapses = &view.synapses;
    let mut marks = Marks {
        synapses: vec![Mark::None; synapses.len()],
        nodes: vec![Mark::None; view.nodes.len()],
    };
    // The largest distance of a changed weight (the game divides by it whatever the unit).
    let max_delta = diff
        .connection_deltas
        .iter()
        .map(|delta| delta.distance.abs())
        .fold(0.0_f32, f32::max);
    let n_max = diff.max_connections.max(1) as f32;
    let n_common = diff.common_connections.max(1) as f32;
    for (index, synapse) in synapses.iter().enumerate() {
        let inov = synapse.innovation;
        if diff.disjoint_individual.contains(&inov) {
            marks.synapses[index] = Mark::New {
                amount: 1.0,
                distance: 1.0 / n_max,
                contribution: 1.0 / n_max / span,
            };
        } else if diff.toggled.contains(&inov) {
            marks.synapses[index] = Mark::Toggled {
                amount: 0.75,
                distance: 1.0 / n_common,
                contribution: 1.0 / n_common / span,
            };
        } else if let Some(delta) = diff
            .connection_deltas
            .iter()
            .find(|delta| delta.template_index == index && delta.innovation == inov)
        {
            // The game compares the index in the compared brain with this one's.
            marks.synapses[index] = Mark::Changed {
                amount: (delta.delta.abs() / max_delta).sqrt(),
                delta: -delta.delta,
                distance: delta.distance,
                contribution: delta.distance / span,
            };
        }
    }
    if !diff.activation_deltas.is_empty() {
        for (index, node) in view.nodes.iter().enumerate() {
            if let Some(delta) = diff
                .activation_deltas
                .iter()
                .find(|delta| delta.innovation == node.innovation)
            {
                marks.nodes[index] = Mark::Changed {
                    amount: (delta.delta.abs() / max_delta).sqrt(),
                    delta: delta.delta,
                    distance: delta.distance,
                    contribution: delta.distance / span,
                };
            }
        }
    }
    marks
}

// ----------------------------------------------------------------------------------------- the pictures

/// The synapses as one picture of the holder and a margin: made once per layout and scale, handed out again until
/// either changes (the window uploads a picture once per image).
struct Strips {
    serial: u64,
    scale_bits: u32,
    draw: Option<UiDraw>,
}

/// One cached picture per diagram that can be on screen: the species panel's and the inspector's.
static STRIPS: [Mutex<Option<Strips>>; 2] = [Mutex::new(None), Mutex::new(None)];

/// A synapse's thickness (`UISynaps.SetSynapseThickness`).
#[must_use]
pub fn thickness(weight: f32) -> f32 {
    let clamped = weight.abs().clamp(0.0, 25.0);
    let e = (clamped / 2.0).exp();
    (e - 1.0) / (1.0 + e) * 6.0 + 10.0
}

/// `UISynaps.GetColorGradiant`: green for a positive weight, red for a negative one, white at 0.25 when disabled.
#[must_use]
pub fn synapse_colour(weight: f32, alpha: f32, enabled: bool) -> Rgba {
    if !enabled {
        return [255, 255, 255, 64];
    }
    // `(exp(v) - 1) / (1 + exp(v))`, kept finite for the huge weights a mutation can leave.
    let e = weight.clamp(-20.0, 20.0).exp();
    let num = (e - 1.0) / (1.0 + e);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let a = byte(alpha);
    if weight >= 0.0 {
        [byte(1.0 - num), 255, byte(1.0 - num), a]
    } else {
        [255, byte(num + 1.0), byte(num + 1.0), a]
    }
}

/// `UINeuron.GetColorGradiant`: the tint of a neuron by its value.
#[must_use]
pub fn neuron_colour(value: f32) -> Rgba {
    let e = (2.5 * value).clamp(-20.0, 20.0).exp();
    let num = (e - 1.0) / (1.0 + e);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if value >= 0.0 {
        [byte(1.0 - num), 255, byte(1.0 - num), 255]
    } else {
        [255, byte(num + 1.0), byte(num + 1.0), 255]
    }
}

/// The tiles' alpha (`synapse_0`): the sprite when it is loaded, else the pattern of its pixels.
pub(crate) fn pattern(ui: &UiAssets) -> Vec<u8> {
    if let Some(texture) = ui
        .texture_of_alias("synapse_0")
        .and_then(|slot| ui.texture(slot))
        && texture.width == 12
        && texture.height == 7
    {
        return texture.rgba.chunks(4).map(|pixel| pixel[3]).collect();
    }
    SYNAPSE_PATTERN.iter().flatten().copied().collect()
}

/// One synapse as the picture needs it: both ends in holder coordinates, how thick, its tint and a halo.
pub(crate) struct Strip {
    pub(crate) from: [f32; 2],
    pub(crate) to: [f32; 2],
    pub(crate) thickness: f32,
    pub(crate) tint: Rgba,
    pub(crate) halo: f32,
}

/// The synapses of a diagram as strips, in the brain's order (later ones over earlier ones).
fn strips(
    diagram: &Diagram,
    marks: &Marks,
    view: &bibites_core::brain::BrainGraphView,
) -> Vec<Strip> {
    let mut out = Vec::with_capacity(view.synapses.len());
    for (index, synapse) in view.synapses.iter().enumerate() {
        let (Some(from), Some(to)) = (
            diagram.position(synapse.node_in),
            diagram.position(synapse.node_out),
        ) else {
            continue;
        };
        out.push(Strip {
            from,
            to,
            thickness: thickness(synapse.weight),
            tint: synapse_colour(synapse.weight, 1.0, synapse.enabled),
            halo: marks.synapses.get(index).map_or(0.0, Mark::amount),
        });
    }
    out
}

/// Draws strips into a picture whose top left is `origin` (reference px) and that is `size` px at `scale`: each
/// pixel inside a strip takes the tint times the tile's alpha (the strip's own axis along its direction, the tile a
/// `thickness` high and `12 / 7` of that wide), composed in order.
pub(crate) fn rasterise(
    strips: &[Strip],
    holder: [f32; 2],
    origin: [f32; 2],
    size: [u32; 2],
    scale: f32,
    tiles: &[u8],
) -> Vec<u8> {
    let mut rgba = vec![0_u8; (size[0] * size[1] * 4) as usize];
    // The 12 x 7 tile's alpha at a texel: the texture is point sampled and clamped, and each tile of the strip is a quad of its own with the
    // sprite's whole 0 ..= 1 range, so a neighbour past the tile's edge is the edge texel.
    let texel = |x: i32, y: i32| -> f32 { f32::from(tiles[(y.clamp(0, 6) * 12 + x.clamp(0, 11)) as usize]) / 255.0 };
    for strip in strips {
        let delta = [strip.to[0] - strip.from[0], strip.to[1] - strip.from[1]];
        let length = (delta[0] * delta[0] + delta[1] * delta[1]).sqrt();
        if length < 1e-3 {
            continue;
        }
        let direction = [delta[0] / length, delta[1] / length];
        let up = [-direction[1], direction[0]];
        let reach = strip.thickness * 0.5;
        let tile_width = 12.0 * strip.thickness / 7.0;
        // The pixels the strip can touch: the box of its four corners.
        let corners = [
            (strip.from, -reach),
            (strip.from, reach),
            (strip.to, -reach),
            (strip.to, reach),
        ]
        .map(|(point, across)| [point[0] + up[0] * across, point[1] + up[1] * across]);
        let to_pixel = |local: [f32; 2]| -> [f32; 2] {
            [
                (holder[0] + local[0] - origin[0]) * scale,
                (holder[1] - local[1] - origin[1]) * scale,
            ]
        };
        let pixels = corners.map(to_pixel);
        let min_x = pixels
            .iter()
            .map(|p| p[0])
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as u32;
        let max_x = (pixels
            .iter()
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil() as i64)
            .clamp(0, i64::from(size[0])) as u32;
        let min_y = pixels
            .iter()
            .map(|p| p[1])
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as u32;
        let max_y = (pixels
            .iter()
            .map(|p| p[1])
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil() as i64)
            .clamp(0, i64::from(size[1])) as u32;
        // A strip entirely outside the picture has nothing to draw.
        if min_x >= max_x || min_y >= max_y {
            continue;
        }
        for py in min_y..max_y {
            // The run of this row the strip's rectangle can touch: `along` and `across` are linear in the pixel's `x`, so each bounds
            // an interval of it (a long diagonal strip would otherwise test every pixel of its box).
            let ly = holder[1] - (origin[1] + (py as f32 + 0.5) / scale);
            let ry = ly - strip.from[1];
            let (mut low, mut high) = (f32::NEG_INFINITY, f32::INFINITY);
            // `a x + b` has to stay in `lo ..= hi`.
            let mut bound = |a: f32, b: f32, lo: f32, hi: f32| {
                if a.abs() < 1e-9 {
                    if b < lo || b > hi {
                        low = f32::INFINITY;
                    }
                } else {
                    let (x0, x1) = ((lo - b) / a, (hi - b) / a);
                    low = low.max(x0.min(x1));
                    high = high.min(x0.max(x1));
                }
            };
            bound(
                direction[0],
                -direction[0] * strip.from[0] + ry * direction[1],
                0.0,
                length,
            );
            bound(
                up[0],
                -up[0] * strip.from[0] + ry * up[1],
                -reach,
                reach,
            );
            if low > high {
                continue;
            }
            // Back to pixels, a pixel wider on both sides so rounding cannot drop one the strip covers.
            let pixel_x = |lx: f32| (lx + holder[0] - origin[0]) * scale - 0.5;
            let from_x = (pixel_x(low).floor() as i64 - 1).clamp(i64::from(min_x), i64::from(max_x)) as u32;
            let end_x = (pixel_x(high).ceil() as i64 + 1).clamp(i64::from(min_x), i64::from(max_x)) as u32;
            for px in from_x..end_x {
                // The pixel's middle in holder coordinates (y up).
                let lx = origin[0] + (px as f32 + 0.5) / scale - holder[0];
                let (rx, ry) = (lx - strip.from[0], ly - strip.from[1]);
                let along = rx * direction[0] + ry * direction[1];
                let across = rx * up[0] + ry * up[1];
                if along < 0.0 || along > length || across.abs() > reach {
                    continue;
                }
                // The pixel's texel in its tile (`synapseActivation`, `_MainTex` point sampled).
                let u = (along / tile_width).rem_euclid(1.0);
                let v = ((strip.thickness * 0.5 - across) / strip.thickness).clamp(0.0, 1.0);
                let (tx, ty) = (((u * 12.0) as i32).min(11), ((v * 7.0) as i32).min(6));
                let centre = texel(tx, ty);
                let mut colour = [
                    f32::from(strip.tint[0]) / 255.0,
                    f32::from(strip.tint[1]) / 255.0,
                    f32::from(strip.tint[2]) / 255.0,
                ];
                let mut coverage = centre;
                if strip.halo > 0.0 {
                    // `_Selected`: the sprite's alpha edge, one texel out (`1 - prod(1 - alpha of the four neighbours) - alpha`), is mixed toward
                    // blue and made as opaque as `_HighlightAmount` says: `alpha = sat(a + sat(outline H + a))`.
                    let around = [
                        texel(tx - 1, ty),
                        texel(tx + 1, ty),
                        texel(tx, ty - 1),
                        texel(tx, ty + 1),
                    ];
                    let outline = 1.0 - around.iter().map(|a| 1.0 - a).product::<f32>() - centre;
                    let weight = outline.clamp(0.0, 1.0);
                    for (channel, blue) in colour.iter_mut().zip([0.0, 0.011_904, 1.0]) {
                        *channel += (blue - *channel) * weight;
                    }
                    coverage = (centre + (outline * strip.halo.clamp(0.0, 1.0) + centre).clamp(0.0, 1.0)).clamp(0.0, 1.0);
                }
                coverage *= f32::from(strip.tint[3]) / 255.0;
                let source = [
                    colour[0] * coverage,
                    colour[1] * coverage,
                    colour[2] * coverage,
                    coverage,
                ];
                if source[3] <= 0.0 {
                    continue;
                }
                let at = ((py * size[0] + px) * 4) as usize;
                let keep = 1.0 - source[3];
                for channel in 0..4 {
                    let before = f32::from(rgba[at + channel]) / 255.0;
                    rgba[at + channel] =
                        ((source[channel] + before * keep).clamp(0.0, 1.0) * 255.0).round() as u8;
                }
            }
        }
    }
    // The buffer holds premultiplied colour; the window wants straight alpha.
    for pixel in rgba.chunks_exact_mut(4) {
        if pixel[3] > 0 && pixel[3] < 255 {
            let alpha = f32::from(pixel[3]) / 255.0;
            for value in &mut pixel[..3] {
                *value = (f32::from(*value) / alpha).round().min(255.0) as u8;
            }
        }
    }
    rgba
}

// ----------------------------------------------------------------------------------------- the drawing

/// What a diagram is drawn from: the layout, the marks, the brain's view and where the holder is.
pub(crate) struct DiagramDraw<'a> {
    /// Which cached picture of the synapses is this diagram's (one per diagram on screen).
    pub slot: usize,
    /// Tells a new layout from the last one, for the picture cache.
    pub serial: u64,
    /// The layout.
    pub diagram: &'a Diagram,
    /// The marks of the differences.
    pub marks: &'a Marks,
    /// The brain.
    pub view: &'a bibites_core::brain::BrainGraphView,
    /// The holder (`NeuronHolder`): reference px, `x, y, w, h`; the nodes' origin is its top left.
    pub holder: [f32; 4],
    /// *Show Node Names*.
    pub show_names: bool,
    /// Neurons are tinted by the brain's live values (the inspector's `UINeuron.SetValue` / `Update`), not by 0.
    pub live: bool,
    /// `UINeuron.showValueOnHover`: the neuron under the pointer shows its value over it. `BrainPanelCommon.InitPanel` switches it off for the
    /// neurons it creates and the species panel keeps that, but the inspector's `BrainPanel.InitPanel` then replaces the pool with one that does
    /// not (the prefab's default is on), so only the inspector's neurons show a value on hover.
    pub value_on_hover: bool,
    /// The first hit id of the neurons.
    pub hit_base: u32,
    /// The action of pressing a neuron.
    pub node_action: fn(u16) -> UiAction,
    /// How a neuron's name is written.
    pub label: Label,
}

/// How a neuron's name (`NameText`: LiberationSans SDF 14 in a holder that follows the neuron's scale) is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Label {
    /// The neuron's scale the text is under: 1.28571 (the whole neuron is scaled, its name holder with it).
    pub scale: f32,
    /// The colour: the labels are white TMP text, which the `LiberationSans SDF` material (a black face) draws black, on the species panel as in
    /// the inspector (a live capture of the game's Brain tab: the names are dark).
    pub color: Rgba,
    /// The heavy face the game's outlined labels come out in.
    pub heavy: bool,
}

impl Drawer<'_> {
    /// The Brain tab.
    pub(super) fn brain(&self, frame: &mut UiFrame, app: &App, entry: &SpeciesListEntry) {
        let s = self.s;
        let state = app.brain.as_ref().filter(|state| state.species == entry.id);
        let record = app.world.species.get(entry.id);
        let int = |value: usize| {
            bibites_core::format::FloatValueFormat {
                is_int: true,
                ..bibites_core::format::FloatValueFormat::default()
            }
            .format(value as f32)
        };
        let (nodes_text, connections_text) = state.map_or_else(
            || (String::new(), String::new()),
            |state| {
                (
                    int(state.diagram.used.len()),
                    int(state.diagram.graph.synapses.len()),
                )
            },
        );
        let nodes_width = self.measure(&nodes_text, 18.0 * s, true) / s;
        let connections_width = self.measure(&connections_text, 18.0 * s, true) / s;
        let height = state.map_or(MIN_HOLDER, |state| state.diagram.height);
        let geometry = geometry_for(
            app.brain_holder_width,
            height,
            nodes_width,
            connections_width,
            entry.parent.is_some(),
        );
        self.slice(frame, "base_icon", 5.0, 1.0, geometry.panel, FRAME_GREY);
        frame.hit(
            super::HIT_PANEL,
            self.rect_pos(geometry.panel),
            self.rect_size(geometry.panel),
            UiAction::Consume,
        );
        // The name and the two rows (`header`); the distances are the brains'.
        self.species_header(
            frame,
            app,
            entry,
            &geometry.header,
            &|target, other| {
                target
                    .template
                    .brain_distance(&other.template.brain, &app.world.settings)
            },
            "Overlay/SpeciesPanels/SpeciesBrainPanel/header",
        );
        // The info row.
        frame.bold(
            "Brain Diagram:",
            self.at(geometry.info_title[0], geometry.info_title[1] + 1.0),
            20.0 * s,
            INK,
        );
        frame.bold(
            "Nodes :",
            self.at(geometry.nodes_title[0], geometry.nodes_title[1]),
            18.0 * s,
            INK,
        );
        frame.bold(
            nodes_text,
            self.at(geometry.nodes_value[0], geometry.nodes_value[1]),
            18.0 * s,
            BLACK,
        );
        frame.bold(
            "Connections: ",
            self.at(geometry.connections_title[0], geometry.connections_title[1]),
            18.0 * s,
            INK,
        );
        frame.bold(
            connections_text,
            self.at(geometry.connections_value[0], geometry.connections_value[1]),
            18.0 * s,
            BLACK,
        );
        if let (Some(state), Some(record)) = (state, record) {
            self.diagram(
                frame,
                app,
                state,
                &record.template.brain.graph_view(),
                &geometry,
            );
        }
        self.brain_bottom(frame, app, &geometry);
    }

    /// The bottom line: the toggle and the three buttons.
    fn brain_bottom(&self, frame: &mut UiFrame, app: &App, geometry: &Geometry) {
        let s = self.s;
        // The toggle: a `base_icon` box (border at multiplier 2) with a check mark.
        self.slice(frame, "base_icon", 5.0, 0.5, geometry.toggle_box, WHITE);
        if app.show_brain_differences {
            self.icon(frame, "checkmark_icon", geometry.toggle_box, WHITE);
        }
        frame.bold(
            "Highlight differences",
            self.at(
                geometry.toggle_text[0],
                geometry.toggle_text[1] + (geometry.toggle_text[3] - 16.0 * 1.1719) * 0.5,
            ),
            16.0 * s,
            INK,
        );
        let toggle = [
            geometry.toggle_box[0],
            geometry.toggle_box[1],
            180.17,
            geometry.toggle_box[3],
        ];
        frame.hit(
            HIT_TOGGLE,
            self.rect_pos(toggle),
            self.rect_size(toggle),
            UiAction::BrainToggleDifferences,
        );
        let labels = ["Sort ", "Save pos.", "Reset pos."];
        let actions = [
            UiAction::BrainSort,
            UiAction::BrainSavePositions,
            UiAction::BrainResetPositions,
        ];
        let paths = ["SortButton", "SavePosButtons", "ResetPosButton"];
        for (index, rect) in geometry.buttons.iter().enumerate() {
            self.slice(frame, "base_icon", 5.0, 1.0 / 1.5, *rect, WHITE);
            // `LiberationSans SDF`, bold style (the asset has no bold face: TMP widens the regular one), black like all its text.
            let font = format!("LiberationSans SDF{}", crate::text_sdf::BOLD_SUFFIX);
            let width = self.metrics.map_or(labels[index].chars().count() as f32 * 14.0 * s * 0.6, |fonts| {
                fonts.measure_named(Some(&font), FontRole::Body, labels[index], 14.0 * s)
            });
            frame.draws.push(UiDraw::Text {
                font: Some(font),
                text: labels[index].to_owned(),
                pos: Vec2::new(
                    (rect[0] + rect[2] * 0.5) * s - width * 0.5,
                    (rect[1] + (rect[3] - 14.0 * 1.1719) * 0.5) * s,
                ),
                px: 14.0 * s,
                color: BLACK,
                role: FontRole::Body,
            });
            frame.hit(
                HIT_SORT + index as u32,
                self.rect_pos(*rect),
                self.rect_size(*rect),
                actions[index],
            );
            self.tip(
                frame,
                *rect,
                &format!(
                    "Overlay/SpeciesPanels/SpeciesBrainPanel/BottomLine/{}",
                    paths[index]
                ),
            );
        }
    }

    /// The diagram in its holder: the synapses as one picture, the neurons over them.
    fn diagram(
        &self,
        frame: &mut UiFrame,
        app: &App,
        state: &State,
        view: &bibites_core::brain::BrainGraphView,
        geometry: &Geometry,
    ) {
        self.draw_diagram(
            frame,
            &DiagramDraw {
                slot: 0,
                serial: state.serial,
                diagram: &state.diagram,
                marks: &state.marks,
                view,
                holder: geometry.holder,
                show_names: app.prefs.show_node_names,
                live: false,
                // `SpeciesBrainPanel` sets `showValueOnHover = false` on its neurons: only the tooltip shows over them.
                value_on_hover: false,
                hit_base: HIT_NODE,
                node_action: UiAction::BrainNode,
                label: Label {
                    // The neuron is scaled by 1.28571 and its name holder is one of its children.
                    scale: 1.285_71,
                    color: BLACK,
                    heavy: false,
                },
            },
        );
    }

    /// A diagram in its holder (`BrainPanelCommon`'s `NeuronHolder` and `SynapseHolder`): the synapses as one picture, the neurons
    /// over them, the tooltips of the neurons and the weight of the synapse under the pointer.
    pub(crate) fn draw_diagram(&self, frame: &mut UiFrame, d: &DiagramDraw<'_>) {
        let s = self.s;
        let (diagram, marks, view, holder) = (d.diagram, d.marks, d.view, d.holder);
        // The picture covers the holder and a margin for the neurons' size.
        let margin = 40.0;
        let origin = [holder[0] - margin, holder[1] - margin];
        let extent = [holder[2] + 2.0 * margin, holder[3] + 2.0 * margin];
        let cached = STRIPS[d.slot].lock().ok().and_then(|cache| {
            cache
                .as_ref()
                .filter(|strips| strips.serial == d.serial && strips.scale_bits == s.to_bits())
                .map(|strips| strips.draw.clone())
        });
        let draw = if let Some(draw) = cached {
            draw
        } else {
            let size = [
                (extent[0] * s).ceil().max(1.0) as u32,
                (extent[1] * s).ceil().max(1.0) as u32,
            ];
            let tiles = pattern(self.ui);
            let rgba = rasterise(
                &strips(diagram, marks, view),
                [holder[0], holder[1]],
                origin,
                size,
                s,
                &tiles,
            );
            let draw = Some(UiDraw::Pixels {
                texture: Arc::new(UiTexture {
                    width: size[0],
                    height: size[1],
                    rgba,
                }),
                pos: Vec2::new(origin[0] * s, origin[1] * s),
                size: Vec2::new(size[0] as f32, size[1] as f32),
            });
            if let Ok(mut cache) = STRIPS[d.slot].lock() {
                *cache = Some(Strips {
                    serial: d.serial,
                    scale_bits: s.to_bits(),
                    draw: draw.clone(),
                });
            }
            draw
        };
        if let Some(draw) = draw {
            frame.draws.push(draw);
        }
        // The neurons, in the order of the diagram's nodes.
        let names = d.show_names;
        let cursor_ref = Vec2::new(self.cursor.x / s, self.cursor.y / s);
        let mut over_node = false;
        // The neuron the pointer is on is the topmost one under it, the last drawn (`OnPointerEnter` reaches only that one).
        let value_node = d
            .value_on_hover
            .then(|| {
                diagram.used.iter().rev().copied().find(|&index| {
                    view.nodes.get(index).is_some()
                        && diagram.position(index).is_some_and(|at| {
                            let (x, y) = (holder[0] + at[0] - NODE / 2.0, holder[1] - at[1] - NODE / 2.0);
                            cursor_ref.x >= x && cursor_ref.x < x + NODE && cursor_ref.y >= y && cursor_ref.y < y + NODE
                        })
                })
            })
            .flatten();
        for &index in &diagram.used {
            let (Some(at), Some(node)) = (diagram.position(index), view.nodes.get(index)) else {
                continue;
            };
            let middle = [holder[0] + at[0], holder[1] - at[1]];
            let rect = [middle[0] - NODE / 2.0, middle[1] - NODE / 2.0, NODE, NODE];
            let mark = marks.nodes.get(index).copied().unwrap_or_default();
            if mark != Mark::None {
                // `UINeuron.SetDiff` selects the neuron whatever the amount: `nodeActivation` draws its blue outline.
                self.icon(frame, "small_circle_base_icon#selected", rect, WHITE);
            }
            self.icon(
                frame,
                "small_circle_base_icon",
                rect,
                neuron_colour(if d.live { node.value } else { 0.0 }),
            );
            let icon = [middle[0] - ICON / 2.0, middle[1] - ICON / 2.0, ICON, ICON];
            self.icon(frame, icon_name(index, node.node_type), icon, WHITE);
            if index > N_INPUTS && node.base_activation.abs() > 0.0 {
                let sign = if node.base_activation > 0.0 {
                    "plus"
                } else {
                    "minus"
                };
                let alpha = node.base_activation.abs().clamp(0.0, 1.0).sqrt();
                self.icon(
                    frame,
                    sign,
                    [rect[0] + NODE - SIGN, rect[1], SIGN, SIGN],
                    synapse_colour(node.base_activation, alpha, true),
                );
            }
            // The neuron's text holder (`Text Holder`: a `HorizontalLayoutGroup` with spacing 5 and centred children, fitted to its contents, its
            // bottom middle on the neuron's top middle) holds the name (`NameText`, 14) and, while the pointer is on the neuron, its value
            // (`ActivationText`, 16): the two sit side by side, centred together, and the shorter one is centred in the height of the taller.
            let name = names.then(|| {
                if index < N_INPUTS + N_OUTPUTS {
                    node.label.clone()
                } else {
                    node.node_type.name().to_owned()
                }
            });
            let value = (value_node == Some(index)).then(|| neuron_value_text(if d.live { node.value } else { 0.0 }));
            if name.is_some() || value.is_some() {
                let scale = d.label.scale;
                let (name_px, value_px) = (14.0 * scale * s, 16.0 * scale * s);
                let name_width = name.as_deref().map_or(0.0, |text| self.measure(text, name_px, d.label.heavy));
                let value_width = value.as_deref().map_or(0.0, |text| self.measure(text, value_px, d.label.heavy));
                let gap = if name.is_some() && value.is_some() { 5.0 * scale * s } else { 0.0 };
                let left = middle[0] * s - (name_width + gap + value_width) * 0.5;
                let (name_height, value_height) = (15.65 * scale, 17.88 * scale);
                let holder_height = match (name.is_some(), value.is_some()) {
                    (true, true) => name_height.max(value_height),
                    (true, false) => name_height,
                    _ => value_height,
                };
                let top = rect[1] - holder_height;
                let font = if d.label.heavy {
                    "Roboto-Bold SDF NoOutline"
                } else {
                    "LiberationSans SDF"
                };
                let role = if d.label.heavy { FontRole::Bold } else { FontRole::Body };
                if let Some(text) = name {
                    frame.draws.push(UiDraw::Text {
                        font: Some(font.to_owned()),
                        text,
                        pos: Vec2::new(left, (top + (holder_height - name_height) * 0.5) * s),
                        px: name_px,
                        color: d.label.color,
                        role,
                    });
                }
                if let Some(text) = value {
                    frame.draws.push(UiDraw::Text {
                        font: Some(font.to_owned()),
                        text,
                        pos: Vec2::new(left + name_width + gap, (top + (holder_height - value_height) * 0.5) * s),
                        px: value_px,
                        color: d.label.color,
                        role,
                    });
                }
            }
            frame.hit(
                d.hit_base + index as u32,
                self.rect_pos(rect),
                self.rect_size(rect),
                (d.node_action)(index as u16),
            );
            if cursor_ref.x >= rect[0]
                && cursor_ref.x < rect[0] + rect[2]
                && cursor_ref.y >= rect[1]
                && cursor_ref.y < rect[1] + rect[3]
            {
                over_node = true;
            }
            if let Some((title, text)) = node_tip(index, node, mark) {
                self.push_tip(
                    frame,
                    rect,
                    Tip {
                        path: "",
                        title,
                        content: tooltip::intern(text),
                        ui: true,
                        instant: false,
                    },
                );
            }
        }
        if over_node {
            return;
        }
        // A synapse under the pointer shows its weight over its middle, and what differs about it.
        let local = [cursor_ref.x - holder[0], holder[1] - cursor_ref.y];
        let hovered = view.synapses.iter().enumerate().rev().find(|(_, synapse)| {
            let (Some(from), Some(to)) = (
                diagram.position(synapse.node_in),
                diagram.position(synapse.node_out),
            ) else {
                return false;
            };
            distance_to_segment(local, from, to) <= (thickness(synapse.weight) * 0.5).max(3.0)
        });
        let Some((index, synapse)) = hovered else {
            return;
        };
        let (Some(from), Some(to)) = (
            diagram.position(synapse.node_in),
            diagram.position(synapse.node_out),
        ) else {
            return;
        };
        let text = crate::value_edit::dotnet_float_string(synapse.weight);
        let middle = [
            holder[0] + (from[0] + to[0]) * 0.5,
            holder[1] - (from[1] + to[1]) * 0.5,
        ];
        let px = 16.0 * s;
        let width = self.measure(&text, px, false);
        frame.draws.push(UiDraw::Text {
            font: Some("LiberationSans SDF".to_owned()),
            text,
            pos: Vec2::new(
                middle[0] * s - width * 0.5,
                (middle[1] - thickness(synapse.weight) * 0.5 - 17.88) * s,
            ),
            px,
            color: WHITE,
            role: FontRole::Body,
        });
        if let Some((title, text)) = marks.synapses.get(index).and_then(Mark::synapse_tip) {
            let at = self.cursor;
            frame.tips.push(TipRect {
                pos: Vec2::new(at.x - 2.0, at.y - 2.0),
                size: Vec2::new(4.0, 4.0),
                tip: Tip {
                    path: "",
                    title,
                    content: tooltip::intern(text),
                    ui: true,
                    instant: false,
                },
            });
        }
    }
}

/// What a neuron shows over itself while the pointer is on it (`UINeuron.OnPointerEnter` / `Update`): `(Mathf.Round(100f * value) / 100f).ToString()`,
/// the value to two places in .NET's shortest form (`Mathf.Round` rounds half to even; a tiny negative value reads `-0`).
pub(crate) fn neuron_value_text(value: f32) -> String {
    crate::value_edit::dotnet_float_string((100.0 * value).round_ties_even() / 100.0)
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

/// A neuron's tooltip (`UINeuron.SetDesc`): its name over what the game says about its function, range and bias,
/// and what differs from the compared species. `None` for a node without a name (the game disables its trigger).
pub(crate) fn node_tip(
    index: usize,
    node: &bibites_core::brain::BrainNodeView,
    mark: Mark,
) -> Option<(&'static str, String)> {
    let archetype = Archetype::of_index(index);
    let function = node.node_type as usize;
    let info = crate::node_info_data::of_node(index, function);
    let mut text = match archetype {
        Archetype::Hidden => {
            let (name, linear, desc, range) = info.map_or(
                (String::new(), true, String::new(), String::new()),
                |info| {
                    (
                        info.name.to_owned(),
                        info.is_linear,
                        info.desc.to_owned(),
                        info.range_text(),
                    )
                },
            );
            format!(
                "Function: {name} {}\n{desc}\n{range}",
                if linear { "" } else { "(nonlinear)" }
            )
        }
        _ => info.map_or_else(String::new, |info| info.desc.to_owned()),
    };
    if archetype != Archetype::Input {
        if archetype == Archetype::Output {
            text.push_str(&format!(
                "\nFunction: {}\n{}",
                node.node_type.name(),
                info.map_or_else(String::new, |info| info.range_text())
            ));
        }
        text.push_str(&format!(
            "\n\nDefault Activation: {:.4}",
            node.base_activation
        ));
        if archetype != Archetype::Output {
            text.push_str(&format!("\nInnovation: {}", node.innovation));
        }
    }
    if let Mark::Changed {
        delta,
        distance,
        contribution,
        ..
    } = mark
    {
        text.push_str(&format!(
            "\n\nDelta: {}{}\nResulting Genetic Distance: {distance:.3}\nContribution to speciation: +{:.1}%",
            if delta > 0.0 { "+" } else { "" },
            crate::value_edit::dotnet_float_string(delta),
            100.0 * contribution
        ));
    }
    let title = if node.label.is_empty() {
        return None;
    } else {
        tooltip::intern(node.label.clone())
    };
    Some((title, text))
}

/// The saved positions of every species (`template.nodeAnchors`), by the species.
pub type Anchors = std::collections::HashMap<SpeciesId, BTreeMap<i64, (f32, f32)>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LoadedWorld;

    /// An app with a species whose template brain is the starter brain plus a few more nodes, the Lineage panel
    /// open and the Brain tab selected.
    fn app_with_brain() -> (App, SpeciesId) {
        let loaded = LoadedWorld::load(None, 41, 10, 1).expect("starter world");
        let mut app = App::new(loaded, Vec2::new(1920.0, 1080.0), None);
        app.lineage_options = Some(crate::lineage_tree::TreeSettings {
            hide_extinct: false,
            ..crate::lineage_tree::TreeSettings::default()
        });
        app.world.run(100);
        let (genome, brain) = {
            let (_, record) = app.world.bibites.iter().next().expect("a bibite");
            (record.genome.clone(), record.brain.clone())
        };
        let id = app
            .world
            .species
            .add_template_species("Zzbrain zzspecies", "", &genome, &brain);
        app.refresh_lineage();
        app.panels.open(crate::ui::PanelKind::Lineage);
        app.lineage_tab = 2;
        app.select_species_info(Some(id));
        (app, id)
    }

    #[test]
    fn the_panel_is_as_big_as_its_parts() {
        // The prefab's: a 300 high holder of the narrowest width.
        let g = geometry(START_HOLDER_WIDTH, 100.0, 10.33, 30.99);
        assert!((g.panel[2] - 465.17).abs() < 0.01, "{:?}", g.panel);
        assert!((g.panel[3] - 489.71).abs() < 0.02, "{:?}", g.panel);
        assert_eq!(g.panel[..2], [15.0, 50.0]);
        // The holder is 25 inside the body, under the 101.7 high header.
        assert!(
            (g.holder[0] - 50.0).abs() < 1e-3 && (g.holder[1] - 184.7).abs() < 1e-3,
            "{:?}",
            g.holder
        );
        assert_eq!((g.holder[2], g.holder[3]), (395.17, 300.0));
        // The bottom line is 20 high under the body, the buttons at its right end.
        let bottom = g.buttons[0][1];
        assert!((bottom - 509.7).abs() < 1e-3, "{bottom}");
        assert!((g.buttons[2][0] + g.buttons[2][2] - (25.0 + 445.17)).abs() < 1e-3);
        assert_eq!(
            (g.buttons[0][2], g.buttons[1][2], g.buttons[2][2]),
            (80.0, 80.0, 85.0)
        );
        // The rows of the header are the Genes tab's: 26.44 and 49.85 down.
        assert!((g.header.parent_title[1] - (58.0 + 26.44)).abs() < 1e-3);
        assert!((g.header.compared_title[1] - (58.0 + 49.85)).abs() < 1e-3);
        // The parent button takes what the title and the distance leave: the prefab's 210.17.
        assert!(
            (g.header.parent_button[2] - 210.17).abs() < 0.01,
            "{:?}",
            g.header.parent_button
        );
        // A taller or wider diagram grows the panel.
        let big = geometry(520.0, 600.0, 10.0, 30.0);
        assert!((big.panel[2] - (520.0 + 70.0)).abs() < 1e-3);
        assert!((big.panel[3] - (8.0 + 101.7 + 50.0 + 600.0 + 20.01 + 10.0)).abs() < 1e-2);
        assert!((big.header.parent_button[2] - (520.0 + 50.0 - 235.0)).abs() < 1e-3);
    }

    #[test]
    fn a_synapse_is_thicker_and_greener_the_stronger_it_is() {
        assert!((thickness(0.0) - 10.0).abs() < 1e-4);
        assert!(thickness(1.0) > thickness(0.5));
        assert!(thickness(100.0) <= 16.0 + 1e-3 && thickness(100.0) > 15.9);
        assert_eq!(thickness(-2.0), thickness(2.0));
        // Green for a positive weight, red for a negative one, white at a quarter when disabled.
        let green = synapse_colour(2.0, 1.0, true);
        assert!(
            green[1] == 255 && green[0] < 255 && green[0] == green[2],
            "{green:?}"
        );
        let red = synapse_colour(-2.0, 1.0, true);
        assert!(red[0] == 255 && red[1] < 255 && red[1] == red[2], "{red:?}");
        assert_eq!(synapse_colour(0.0, 1.0, true), [255, 255, 255, 255]);
        assert_eq!(synapse_colour(3.0, 1.0, false), [255, 255, 255, 64]);
        // A weight of any size keeps its colours finite.
        assert_eq!(synapse_colour(1e30, 1.0, true)[3], 255);
        // A neuron at 0 is white, at a high value green, at a low one red.
        assert_eq!(neuron_colour(0.0), [255, 255, 255, 255]);
        assert!(neuron_colour(1.0)[0] < 100 && neuron_colour(-1.0)[1] < 100);
    }

    #[test]
    fn a_node_has_the_icon_of_its_index_or_its_function() {
        assert_eq!(icon_name(0, NodeType::Input), "input_energy_ratio");
        assert_eq!(icon_name(32, NodeType::Input), "input_phero3_heading");
        assert_eq!(icon_name(33, NodeType::Sigmoid), "output_accelerate");
        assert_eq!(icon_name(47, NodeType::Sigmoid), "output_want2attack");
        assert_eq!(icon_name(48, NodeType::Sigmoid), "hidden_sigmoid");
        assert_eq!(icon_name(60, NodeType::Differential), "hidden_diff");
        assert_eq!(icon_name(61, NodeType::Integrator), "integrator");
        assert_eq!(icon_name(62, NodeType::SoftLatch), "hidden_softlatch_0");
    }

    #[test]
    fn the_strips_are_drawn_tiled_and_coloured_along_their_line() {
        let tiles: Vec<u8> = SYNAPSE_PATTERN.iter().flatten().copied().collect();
        // A positive synapse along the top of a small holder: 10 px thick, at scale 1.
        let strip = Strip {
            from: [0.0, -10.0],
            to: [60.0, -10.0],
            thickness: 10.0,
            tint: synapse_colour(0.0, 1.0, true),
            halo: 0.0,
        };
        let rgba = rasterise(&[strip], [0.0, 0.0], [0.0, 0.0], [64, 24], 1.0, &tiles);
        let alpha = |x: u32, y: u32| rgba[((y * 64 + x) * 4 + 3) as usize];
        // The line is at y = 10 (down from the holder's top): the middle rows are solid where the tile is, the
        // outer rows of the strip (the tile's blank top and bottom) are clear.
        let column: Vec<u8> = (0..24).map(|y| alpha(2, y)).collect();
        assert!(column[10] > 200, "{column:?}");
        assert_eq!(column[4], 0, "above the strip");
        assert_eq!(column[16], 0, "below the strip");
        // Along the strip the notch of the tile comes back every 12 / 7 thicknesses (17.14 px).
        let row: Vec<u8> = (0..60).map(|x| alpha(x, 10)).collect();
        let gaps = row.windows(2).filter(|w| w[0] > 128 && w[1] <= 128).count();
        assert!(gaps >= 2, "the notch repeats: {row:?}");
        // Nothing beyond the ends.
        assert_eq!(alpha(62, 10), 0);
        // A disabled synapse is a quarter as strong.
        let faint = Strip {
            from: [0.0, -10.0],
            to: [60.0, -10.0],
            thickness: 10.0,
            tint: synapse_colour(0.0, 1.0, false),
            halo: 0.0,
        };
        let rgba = rasterise(&[faint], [0.0, 0.0], [0.0, 0.0], [64, 24], 1.0, &tiles);
        assert!(rgba[((10 * 64 + 2) * 4 + 3) as usize] <= 70);
    }

    #[test]
    fn a_diagonal_strip_follows_its_direction() {
        let tiles: Vec<u8> = SYNAPSE_PATTERN.iter().flatten().copied().collect();
        let strip = Strip {
            from: [0.0, 0.0],
            to: [40.0, -40.0],
            thickness: 10.0,
            tint: synapse_colour(1.0, 1.0, true),
            halo: 0.0,
        };
        let rgba = rasterise(&[strip], [0.0, 0.0], [0.0, 0.0], [48, 48], 1.0, &tiles);
        let alpha = |x: u32, y: u32| rgba[((y * 48 + x) * 4 + 3) as usize];
        // On the diagonal (y down equals x) there is paint, far from it none.
        let painted = (4..36).filter(|&k| alpha(k, k) > 100).count();
        assert!(painted > 10, "{painted}");
        assert_eq!(alpha(40, 5), 0);
        assert_eq!(alpha(5, 40), 0);
    }

    #[test]
    fn the_tab_fills_for_a_species_and_keeps_the_holder_widening() {
        let (mut app, id) = app_with_brain();
        let state = app.brain.as_ref().expect("filled at selection");
        assert_eq!(state.species, id);
        assert!(!state.diagram.used.is_empty());
        assert!(state.diagram.width >= 400.0);
        let before = app.brain_holder_width;
        assert_eq!(before, state.diagram.width.max(START_HOLDER_WIDTH));
        // Filling again keeps the width at least where it was.
        app.brain_fill(id);
        assert!(app.brain_holder_width >= before);
    }

    #[test]
    fn sorting_saving_resetting_and_dragging_move_the_nodes() {
        let (mut app, id) = app_with_brain();
        let node = *app
            .brain
            .as_ref()
            .unwrap()
            .diagram
            .used
            .first()
            .expect("a node");
        let at = app.brain.as_ref().unwrap().diagram.position(node).unwrap();
        // Sort keeps the nodes drawn.
        app.brain_sort();
        let drawn = |app: &App| {
            app.brain
                .as_ref()
                .unwrap()
                .diagram
                .positions
                .iter()
                .filter(|slot| slot.is_some())
                .count()
        };
        let count = drawn(&app);
        assert_eq!(count, app.brain.as_ref().unwrap().diagram.used.len());
        // Drag a node: the cursor takes it, it follows within the holder.
        let scale = UiFrame::scaler(app.camera.screen.x, app.camera.screen.y);
        let g = geometry(
            app.brain_holder_width,
            app.brain.as_ref().unwrap().diagram.height,
            0.0,
            0.0,
        );
        let at = app
            .brain
            .as_ref()
            .unwrap()
            .diagram
            .position(node)
            .unwrap_or(at);
        let middle = Vec2::new((g.holder[0] + at[0]) * scale, (g.holder[1] - at[1]) * scale);
        app.brain_press_node(node, middle);
        assert_eq!(app.brain_drag.map(|drag| drag.node), Some(node));
        let target = Vec2::new((g.holder[0] + 120.0) * scale, (g.holder[1] + 60.0) * scale);
        app.brain_drag_to(target, false);
        let moved = app.brain.as_ref().unwrap().diagram.position(node).unwrap();
        assert!(
            (moved[0] - 120.0).abs() < 1.0 && (moved[1] + 60.0).abs() < 1.0,
            "{moved:?}"
        );
        // Save pos. keeps the places, Reset pos. forgets them again.
        app.brain_save_positions();
        // (The outputs nothing reaches are not saved: `SaveNodePositions` skips them.)
        let kept = {
            let diagram = &app.brain.as_ref().unwrap().diagram;
            diagram
                .used
                .iter()
                .filter(|&&index| {
                    Archetype::of_index(index) != Archetype::Output || diagram.n_in[index] > 0
                })
                .count()
        };
        assert!(kept > 0);
        assert_eq!(
            app.brain_anchors.get(&id).map(|anchors| anchors.len()),
            Some(kept)
        );
        app.brain_reset_positions();
        assert!(!app.brain_anchors.contains_key(&id));
        // The panel is filled again with the places that were saved: the game only uses them when there are more
        // than three fifths of the nodes' worth (`anchors.Count > 3 * usedIndexes.Count / 5`), and this brain has
        // more outputs that nothing reaches (never saved) than that allows, so the sorter lays it out again.
        app.brain_press_node(node, middle);
        app.brain_drag_to(target, false);
        app.brain_save_positions();
        app.brain_drag = None;
        app.brain_fill(id);
        let saved = app
            .brain_anchors
            .get(&id)
            .map_or(0, |anchors| anchors.len());
        let used = app.brain.as_ref().unwrap().diagram.used.len();
        assert!(saved > 0 && saved <= 3 * used / 5, "{saved} of {used}");
        assert_eq!(
            app.brain.as_ref().unwrap().diagram.position(node).unwrap()[0],
            0.0,
            "an input is back in its column"
        );
    }

    #[test]
    fn the_differences_with_a_compared_species_are_marked_when_shown() {
        let (mut app, id) = app_with_brain();
        // A second species whose brain lacks a synapse the first has and weighs another one differently.
        let (genome, mut brain) = {
            let record = app.world.species.get(id).unwrap();
            (
                record.template.genome.clone(),
                record.template.brain.clone(),
            )
        };
        let other_brain = {
            let view = brain.graph_view();
            assert!(!view.synapses.is_empty(), "the starter brain has synapses");
            brain.set_synapse_weight(0, view.synapses[0].weight + 1.0);
            brain.clone()
        };
        let other =
            app.world
                .species
                .add_template_species("Zzother zzspecies", "", &genome, &other_brain);
        app.refresh_lineage();
        app.species_compared = Some(other);
        // Hidden by default; the toggle shows them.
        app.brain_update_marks();
        assert_eq!(app.brain.as_ref().unwrap().marks, Marks::default());
        app.brain_toggle_differences();
        assert!(app.show_brain_differences);
        let marks = &app.brain.as_ref().unwrap().marks;
        let marked = marks
            .synapses
            .iter()
            .filter(|mark| **mark != Mark::None)
            .count();
        assert!(
            marked >= 1,
            "the changed weight is marked: {:?}",
            marks.synapses
        );
        let first = marks.synapses[0];
        match first {
            Mark::Changed { delta, amount, .. } => {
                assert!(
                    (delta - -1.0).abs() < 1e-5 || (delta - 1.0).abs() < 1e-5,
                    "{delta}"
                );
                // `sqrt(|delta| / maxDelta)` with the largest *distance* as the divisor: not capped at 1.
                assert!(amount > 0.0, "{amount}");
            }
            other => panic!("{other:?}"),
        }
        let (title, text) = first.synapse_tip().expect("a tooltip");
        assert_eq!(title, "Weight Difference from compared");
        assert!(text.starts_with("Delta: "), "{text}");
        // Turning them off clears the marks.
        app.brain_toggle_differences();
        assert_eq!(app.brain.as_ref().unwrap().marks, Marks::default());
    }

    #[test]
    fn a_neuron_of_the_species_tab_shows_no_value_under_the_pointer() {
        // `SpeciesBrainPanel` sets `showValueOnHover = false` on its neurons (only the inspector's tab shows a value over the neuron
        // under the pointer, see `inspector_brain`): the pointer on a neuron adds its tooltip and no text.
        let (mut app, _) = app_with_brain();
        let state = app.brain.as_ref().expect("the tab was filled");
        let g = geometry(app.brain_holder_width, state.diagram.height, 0.0, 0.0);
        let node = *state.diagram.used.first().expect("a node");
        let at = state.diagram.position(node).expect("placed");
        let middle = Vec2::new(g.holder[0] + at[0], g.holder[1] - at[1]);
        let frame_with = |app: &mut App, cursor: Vec2| {
            app.cursor = cursor;
            let mut frame = UiFrame::new(1.0);
            crate::species_panel::emit(&mut frame, app, &UiAssets::empty(), None, 1920.0, 1080.0);
            frame
        };
        let texts = |frame: &UiFrame| -> Vec<String> {
            frame
                .draws
                .iter()
                .filter_map(|draw| match draw {
                    UiDraw::Text { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect()
        };
        let away = frame_with(&mut app, Vec2::new(-50.0, -50.0));
        let over = frame_with(&mut app, middle);
        assert!(
            over.tips.iter().any(|rect| rect.contains(middle)),
            "the pointer is on a neuron: it has its tooltip"
        );
        assert_eq!(texts(&away), texts(&over), "and the neuron shows no value");
    }

    #[test]
    fn the_tab_draws_its_header_diagram_and_buttons() {
        let (app, _) = app_with_brain();
        let mut frame = UiFrame::new(1.0);
        crate::species_panel::emit(&mut frame, &app, &UiAssets::empty(), None, 1920.0, 1080.0);
        let texts: Vec<String> = frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Text { text, .. } => Some(text.trim().to_owned()),
                _ => None,
            })
            .collect();
        for expected in [
            "Parent species:",
            "Compared species:",
            "Brain Diagram:",
            "Nodes :",
            "Connections:",
            "Highlight differences",
            "Sort",
            "Save pos.",
            "Reset pos.",
        ] {
            assert!(
                texts.iter().any(|text| text == expected),
                "{expected}: {texts:?}"
            );
        }
        let actions: Vec<UiAction> = frame.actions.iter().map(|(_, action)| *action).collect();
        for wanted in [
            UiAction::BrainSort,
            UiAction::BrainSavePositions,
            UiAction::BrainResetPositions,
            UiAction::BrainToggleDifferences,
        ] {
            assert!(actions.contains(&wanted), "{wanted:?}");
        }
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, UiAction::BrainNode(_)))
        );
        // The synapses are one picture, the same on the next frame.
        let mut again = UiFrame::new(1.0);
        crate::species_panel::emit(&mut again, &app, &UiAssets::empty(), None, 1920.0, 1080.0);
        let pictures = |frame: &UiFrame| -> Vec<usize> {
            frame
                .draws
                .iter()
                .filter_map(|draw| match draw {
                    UiDraw::Pixels { texture, .. } => Some(Arc::as_ptr(texture) as usize),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(pictures(&frame).len(), 1);
        assert_eq!(pictures(&frame), pictures(&again));
    }
}

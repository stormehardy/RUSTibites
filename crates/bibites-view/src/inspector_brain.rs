//! The inspector's Brain tab (`BrainPanel`, a `BrainPanelCommon` like the species panel's): the brain of the selected bibite or egg as
//! a diagram laid out by the game's `BrainSorter`, in a panel that grows with it.
//!
//! `FillBrainPanel` runs when the target is set: the nodes go where the sorter puts them (or where the species' saved positions say,
//! when there are enough of them), the holder gets `preferredWidth = width` and `minHeight = height`, and the panel (a vertical
//! layout with a content-size fitter, pivot at its top left corner) grows right and down with it. The diagram is the species panel's
//! (`species_panel::brain`): neurons tinted by their value, which here is the live value of the brain (`BrainPanel.UpdatePanel`
//! sets `n.SetValue(brain.Nodes[n.index].Value)` every frame), synapses as tiled strips, a neuron's tooltip, the weight of the
//! synapse under the pointer; a neuron can be dragged (Shift snaps it to its step). *Highlight differences* marks what differs from
//! the species' template (`species.BrainDifferencesToIndividual`), Sort (and F1 / F3 / F4, F2 held) sorts again.
//!
//! Prefab values (level2 `BrainPanel`, the native capture): panel padding 15, pivot top left, 690 x 502.44 at the captured state;
//! `Body` pads its holder by 40; the holder's `LayoutElement` is `min_height 300`, `preferred_width 580`, `preferred_height 75`, and
//! sits 65 right and 197 below the top left corner of the screen. The Expanded View button (`ExpandedBrainPanel`, a force-directed
//! graph with its own settings) opens `expanded_brain_ui`.

use bibites_brainsort::Rng;
use bibites_core::brain::Brain;
use bibites_core::math::Vec2;

use crate::app::{App, SelectionTarget};
use crate::brain_layout::{Diagram, Graph};
use crate::panel_layout::PanelLayout;
use crate::species_panel::brain::{DiagramDraw, Label, Marks, marks_between, next_serial};
use crate::ui::{InspectorTab, UiAction, UiAssets, UiFrame};
use crate::unity_layout::{Elem, LNode};

/// The holder's width in the prefab (`LayoutElement.preferredWidth`): it only ever grows from it.
pub const START_HOLDER_WIDTH: f32 = 580.0;
/// `LayoutElement.preferredHeight` of the holder: with a smaller `minHeight` this is its height.
const HOLDER_PREFERRED_HEIGHT: f32 = 75.0;
/// The holder's top left corner on the screen (reference px, y down), which the panel's anchoring keeps.
const HOLDER_ORIGIN: [f32; 2] = [65.0, 197.0];
/// The holder's height in the captured state (`minHeight`).
const CAPTURED_HEIGHT: f32 = 300.0;
/// The first hit id of the neurons (the species panel's start at 900 000).
const HIT_NODE: u32 = 910_000;

/// What the tab was filled with.
#[derive(Clone, Debug)]
pub struct State {
    /// The bibite or egg whose brain this is.
    pub target: SelectionTarget,
    /// The layout.
    pub diagram: Diagram,
    /// The differences with the species' template (empty while they are not shown).
    pub marks: Marks,
    /// Tells a new picture from the last one, for the drawing cache.
    serial: u64,
}

impl State {
    /// The number of the layout's picture: it changes with every sort, drag and new marks.
    #[cfg(test)]
    #[must_use]
    pub fn serial(&self) -> u64 {
        self.serial
    }
}

/// A neuron is held: which, and where in it the cursor took it (reference px from its middle).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drag {
    /// The node's index.
    pub node: usize,
    /// The cursor's offset from the node's middle when it was grabbed.
    pub grab: [f32; 2],
}

/// The size of the holder, which moves the rest of the panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    /// The holder's width (`preferredWidth`).
    pub width: f32,
    /// The holder's height.
    pub height: f32,
}

impl Geometry {
    /// The holder of a diagram: `minHeight = height` against a preferred height of 75.
    #[must_use]
    pub fn of(diagram: &Diagram) -> Self {
        Self {
            width: diagram.width.max(START_HOLDER_WIDTH),
            height: diagram.height.max(HOLDER_PREFERRED_HEIGHT),
        }
    }

    /// The holder rectangle (reference px, `x, y, w, h`; y down).
    #[must_use]
    pub fn holder(&self) -> [f32; 4] {
        [HOLDER_ORIGIN[0], HOLDER_ORIGIN[1], self.width, self.height]
    }
}

impl App {
    /// The brain of the bibite or egg `target`.
    pub(crate) fn target_brain(&self, target: SelectionTarget) -> Option<&Brain> {
        match target {
            SelectionTarget::Bibite(id) => self.world.bibites.get(id).map(|record| &record.brain),
            SelectionTarget::Egg(id) => self.world.eggs.get(id).map(|egg| &egg.brain),
        }
    }

    /// The species of the bibite or egg `target`.
    fn target_species(&self, target: SelectionTarget) -> Option<bibites_core::ids::SpeciesId> {
        match target {
            SelectionTarget::Bibite(id) => self
                .world
                .bibites
                .get(id)
                .and_then(|record| record.meta.species),
            SelectionTarget::Egg(id) => self.world.eggs.get(id).and_then(|egg| egg.meta.species),
        }
    }

    /// Keeps the tab's layout in step with the target (`BibitePanel.SetTarget` fills every panel when the target is set; the layout is
    /// made when the tab shows, then kept while the target stays, also while the tab is closed and opened again).
    pub fn sync_inspector_brain(&mut self) {
        let target = self.selection.as_ref().map(|panel| panel.target);
        if self.inspector_brain.as_ref().map(|state| state.target) != target {
            self.inspector_brain = None;
            self.inspector_brain_drag = None;
        }
        if self.inspector_visible()
            && self.inspector_tab == InspectorTab::Brain
            && self.inspector_brain.is_none()
            && let Some(target) = target
        {
            self.inspector_brain_fill(target);
        }
    }

    /// `BrainPanel.FillPanel` / `FillBrainPanel`: the layout of the target's brain (by the species' saved positions when there are
    /// enough of them, else by the sorter) and the differences with the species' template.
    fn inspector_brain_fill(&mut self, target: SelectionTarget) {
        let Some(brain) = self.target_brain(target) else {
            self.inspector_brain = None;
            return;
        };
        let graph = Graph::of(&brain.graph_view());
        let scale = UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        // `(1080 - 350 totalScale) / UIScale`, the UI scale being the user's (1 by default).
        let max_height = 1080.0 - 350.0 * scale;
        let anchors = self
            .target_species(target)
            .and_then(|species| self.brain_anchors.get(&species));
        self.brain_sorts += 1;
        let mut rng = Rng::new(self.brain_sorts.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let diagram = Diagram::fill(
            graph,
            anchors,
            self.inspector_holder_width,
            max_height,
            &mut rng,
        );
        // The holder keeps the width it was given: it only ever grows (`holder.preferredWidth = width`).
        self.inspector_holder_width = diagram.width.max(START_HOLDER_WIDTH);
        self.inspector_brain = Some(State {
            target,
            diagram,
            marks: Marks::default(),
            serial: next_serial(),
        });
        self.inspector_brain_update_marks();
    }

    /// `CallShowDifferences`: the marks of the differences with the species' template, when they are shown.
    pub fn inspector_brain_update_marks(&mut self) {
        let Some(target) = self.inspector_brain.as_ref().map(|state| state.target) else {
            return;
        };
        let marks = if self.show_brain_differences {
            self.inspector_brain_marks(target)
        } else {
            Marks::default()
        };
        if let Some(state) = self.inspector_brain.as_mut() {
            state.marks = marks;
            state.serial = next_serial();
        }
    }

    /// `BrainPanel.CallShowDifferences`: `species.BrainDifferencesToIndividual(brain)`, the individual being the shown brain.
    fn inspector_brain_marks(&self, target: SelectionTarget) -> Marks {
        let (Some(brain), Some(species)) = (
            self.target_brain(target),
            self.target_species(target)
                .and_then(|species| self.world.species.get(species)),
        ) else {
            return Marks::default();
        };
        marks_between(
            &species.template.brain,
            brain,
            self.world.species.current_used_span(&self.world.settings),
        )
    }

    /// `SortN(n)`: the sorter runs `n` more times (with new random numbers) and the nodes are placed by it.
    pub fn inspector_brain_sort_n(&mut self, times: usize) {
        self.brain_sorts += 1;
        let seed = self.brain_sorts.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let Some(state) = self.inspector_brain.as_mut() else {
            return;
        };
        state.diagram.sort_n(times, &mut Rng::new(seed));
        state.serial = next_serial();
    }

    /// `SortHiddenOnce` (F2 held): one step of the hidden nodes' physics of `dt` seconds.
    pub fn inspector_brain_sort_hidden(&mut self, dt: f32, scalar: f32) {
        let Some(state) = self.inspector_brain.as_mut() else {
            return;
        };
        state.diagram.sort_hidden_once(dt, scalar);
        state.serial = next_serial();
    }

    /// The holder rectangle of the filled tab.
    fn inspector_holder(&self) -> Option<[f32; 4]> {
        self.inspector_brain
            .as_ref()
            .map(|state| Geometry::of(&state.diagram).holder())
    }

    /// A neuron was pressed: it follows the cursor from now on (`UINeuron.OnDrag`).
    pub fn inspector_brain_press_node(&mut self, node: usize, cursor: Vec2) {
        let (Some(state), Some(holder)) = (self.inspector_brain.as_ref(), self.inspector_holder())
        else {
            return;
        };
        let Some(at) = state.diagram.position(node) else {
            return;
        };
        let scale = UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let (x, y) = (cursor.x / scale, cursor.y / scale);
        self.inspector_brain_drag = Some(Drag {
            node,
            grab: [x - (holder[0] + at[0]), y - (holder[1] - at[1])],
        });
    }

    /// The held neuron follows the cursor, kept inside the holder, snapped to its step with Shift.
    pub fn inspector_brain_drag_to(&mut self, cursor: Vec2, snap: bool) {
        let (Some(drag), Some(holder)) = (self.inspector_brain_drag, self.inspector_holder())
        else {
            return;
        };
        let scale = UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let (x, y) = (
            cursor.x / scale - drag.grab[0],
            cursor.y / scale - drag.grab[1],
        );
        // The node's middle is at `holder + (at.x, -at.y)`: the game keeps the cursor's own place in the holder.
        let at = [x - holder[0], holder[1] - y];
        if let Some(state) = self.inspector_brain.as_mut() {
            state
                .diagram
                .drag_node(drag.node, at, [holder[2], holder[3]], snap);
            state.serial = next_serial();
        }
    }

    /// The numbers the info row shows: the nodes the diagram draws (`usedIndexes.Count`) and every connection.
    #[must_use]
    pub fn inspector_brain_counts(&self) -> Option<(usize, usize)> {
        let state = self.inspector_brain.as_ref()?;
        (Some(state.target) == self.selection.as_ref().map(|panel| panel.target))
            .then_some((state.diagram.used.len(), state.diagram.graph.synapses.len()))
    }
}

/// The Brain panel laid out for the target: the holder takes the diagram's size (`holder.preferredWidth = width`, `holder.minHeight =
/// height`, over the prefab's `LayoutElement`, which the capture shows already set for its own diagram) and the species row its name's
/// width; `text` gives the text a node shows now.
#[must_use]
pub fn panel_layout(
    capture: &crate::ui_runtime::Node,
    state: &State,
    text: &dyn Fn(&str, &crate::ui_runtime::Node) -> Option<String>,
    fonts: Option<&crate::text_sdf::SdfFonts>,
) -> PanelLayout {
    let holder_of = |geometry: Geometry| {
        move |rel: &str, _: &LNode| match rel {
            "BrainPanel/Body/Holder" => Some(Elem {
                ignore: false,
                min: [-1.0, geometry.height],
                pref: [geometry.width, HOLDER_PREFERRED_HEIGHT],
                flex: [-1.0, -1.0],
            }),
            // The header's species box is as wide as its content in the game (the spacer beside it takes the whole surplus, though its
            // `SpeciesText` has a flexible width of 1: the captured row is 267 wide for "Basic bibite", the live one 320.8 for "Seneca
            // octavius", both exactly their content), so it takes no share of the row's surplus.
            "BrainPanel/Top Section/First Row/SpeciesHolder" => Some(Elem {
                ignore: false,
                min: [-1.0, -1.0],
                pref: [-1.0, -1.0],
                flex: [0.0, -1.0],
            }),
            _ => None,
        }
    };
    let captured = holder_of(Geometry {
        width: START_HOLDER_WIDTH,
        height: CAPTURED_HEIGHT,
    });
    let now = holder_of(Geometry::of(&state.diagram));
    PanelLayout::new(
        capture,
        &crate::brain_panel_layout_data::BRAIN_PANEL,
        text,
        Some(&captured),
        Some(&now),
        fonts,
    )
}

/// Draws the diagram in the holder (`NeuronHolder` / `SynapseHolder`) of the captured panel moved by its geometry. `holder_origin` is
/// the composed top left corner of the holder in reference px.
pub fn draw(
    frame: &mut UiFrame,
    app: &App,
    ui: &UiAssets,
    metrics: Option<&crate::text_sdf::SdfFonts>,
    scale: f32,
) -> bool {
    let Some(state) = app.inspector_brain.as_ref() else {
        return false;
    };
    if Some(state.target) != app.selection.as_ref().map(|panel| panel.target) {
        return false;
    }
    let Some(brain) = (match state.target {
        SelectionTarget::Bibite(id) => app.world.bibites.get(id).map(|record| &record.brain),
        SelectionTarget::Egg(id) => app.world.eggs.get(id).map(|egg| &egg.brain),
    }) else {
        return false;
    };
    let view = brain.graph_view();
    let drawer = crate::species_panel::Drawer::new(scale, ui, metrics, app.cursor);
    drawer.draw_diagram(
        frame,
        &DiagramDraw {
            slot: 1,
            serial: state.serial,
            diagram: &state.diagram,
            marks: &state.marks,
            view: &view,
            holder: Geometry::of(&state.diagram).holder(),
            show_names: app.prefs.show_node_names,
            live: true,
            // `BrainPanel.InitPanel` replaces the pool `BrainPanelCommon` made (its neurons had `showValueOnHover = false`) with one whose
            // neurons keep the prefab's default, on: the neuron under the pointer shows its live value.
            value_on_hover: true,
            hit_base: HIT_NODE,
            node_action: UiAction::InspectorBrainNode,
            // The neuron is scaled 1.28571 in the panel, its name with it; the captured white labels draw dark (`emit_texts`).
            label: Label {
                scale: 1.285_71,
                color: [33, 34, 33, 255],
                heavy: true,
            },
        },
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_holder_is_the_captured_one_for_the_captured_diagram_and_grows_from_there() {
        assert_eq!(
            Geometry {
                width: START_HOLDER_WIDTH,
                height: CAPTURED_HEIGHT,
            }
            .holder(),
            [65.0, 197.0, 580.0, 300.0]
        );
    }

    #[test]
    fn a_small_diagram_gives_the_holder_its_minimum_height_not_the_captured_one() {
        // `holder.minHeight = height` replaces the prefab's 300; the preferred height of 75 is the floor.
        let (graph, mut rng) = (
            Graph {
                nodes: Vec::new(),
                synapses: Vec::new(),
            },
            Rng::new(7),
        );
        let diagram = Diagram::fill(graph, None, START_HOLDER_WIDTH, 730.0, &mut rng);
        let g = Geometry::of(&diagram);
        assert!(g.height >= HOLDER_PREFERRED_HEIGHT);
        assert!(g.width >= START_HOLDER_WIDTH);
    }

    /// An app on the starter world, run until its brains have live values, with the first bibite selected and the Brain tab shown.
    fn app_with_the_brain_tab() -> App {
        let loaded = crate::app::LoadedWorld::load(None, 31, 8, 1).expect("starter world");
        let mut app = App::new(loaded, Vec2::new(1920.0, 1080.0), None);
        app.world.run(60);
        assert!(app.select_first());
        app.open_bibite_panel(InspectorTab::Brain, false);
        app.advance(0.0);
        app
    }

    /// The texts of the diagram drawn for the pointer at `cursor` (screen px): `(text, x, y, px)` of each.
    fn diagram_texts(app: &mut App, cursor: Vec2, scale: f32) -> Vec<(String, f32, f32, f32)> {
        app.cursor = cursor;
        let mut frame = UiFrame::new(scale);
        assert!(draw(&mut frame, app, &UiAssets::empty(), None, scale), "the tab draws its diagram");
        frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                crate::ui::UiDraw::Text { text, pos, px, .. } => Some((text.clone(), pos.x, pos.y, *px)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_neuron_under_the_pointer_shows_its_live_value_over_it_as_the_games_inspector_does() {
        let mut app = app_with_the_brain_tab();
        let scale = UiFrame::scaler(app.camera.screen.x, app.camera.screen.y);
        let (shown, middle) = {
            let state = app.inspector_brain.as_ref().expect("the tab was filled");
            let holder = Geometry::of(&state.diagram).holder();
            let view = app.target_brain(state.target).expect("the brain").graph_view();
            // The used neuron with the largest value, so that the text is not the "0" of a neuron at rest.
            let (index, value) = state
                .diagram
                .used
                .iter()
                .filter_map(|&index| view.nodes.get(index).map(|node| (index, node.value)))
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                .expect("a neuron");
            let at = state.diagram.position(index).expect("placed");
            (
                crate::species_panel::brain::neuron_value_text(value),
                Vec2::new((holder[0] + at[0]) * scale, (holder[1] - at[1]) * scale),
            )
        };
        assert_ne!(shown, "0", "the brain has run");
        let top = middle.y - crate::species_panel::brain::NODE * 0.5 * scale;
        // Names are not on by default, so a value is the only text a hovered neuron has.
        assert!(!app.prefs.show_node_names);
        let away = diagram_texts(&mut app, Vec2::new(-50.0, -50.0), scale);
        assert!(away.iter().all(|text| text.0 != shown), "no value while the pointer is elsewhere: {away:?}");
        let near = diagram_texts(&mut app, middle, scale);
        let (text, x, y, px) = near.iter().find(|text| text.0 == shown).cloned().expect("the value of the neuron under the pointer");
        // 16 pt in the neuron's scale (1.28571), centred over it, its box (17.88 high) standing on the neuron's top edge.
        assert!((px - 16.0 * 1.285_71 * scale).abs() < 1e-3, "{px}");
        let width = text.chars().count() as f32 * px * 0.56;
        assert!((x + width * 0.5 - middle.x).abs() < 0.6, "centred: {x} + {width} / 2 vs {}", middle.x);
        assert!((y + 17.88 * 1.285_71 * scale - top).abs() < 0.6, "on the top edge: {y} vs {top}");
        // With the names on, the name and the value share the holder: side by side, 5 apart, centred together, the name standing a
        // little higher than it does alone (the holder is as high as the taller text and the shorter one is centred in it).
        app.prefs.show_node_names = true;
        let alone = diagram_texts(&mut app, Vec2::new(-50.0, -50.0), scale);
        let estimate = |text: &str, px: f32| text.chars().count() as f32 * px * 0.56;
        let name_px = 14.0 * 1.285_71 * scale;
        let name = alone
            .iter()
            .find(|text| {
                (text.1 + estimate(&text.0, name_px) * 0.5 - middle.x).abs() < 0.6
                    && (text.2 + 15.65 * 1.285_71 * scale - top).abs() < 0.6
            })
            .expect("the neuron's own name, centred over it")
            .0
            .clone();
        let both = diagram_texts(&mut app, middle, scale);
        let (name_width, value_width) = (estimate(&name, name_px), estimate(&shown, 16.0 * 1.285_71 * scale));
        let gap = 5.0 * 1.285_71 * scale;
        let left = middle.x - (name_width + gap + value_width) * 0.5;
        let at = |wanted: &str, x: f32, y: f32| both.iter().any(|text| text.0 == wanted && (text.1 - x).abs() < 0.6 && (text.2 - y).abs() < 0.6);
        let holder_top = top - 17.88 * 1.285_71 * scale;
        assert!(at(&name, left, holder_top + (17.88 - 15.65) * 0.5 * 1.285_71 * scale), "the name moved left and up: {both:?}");
        assert!(at(&shown, left + name_width + gap, holder_top), "the value right of it: {both:?}");
    }

    #[test]
    fn a_neurons_value_reads_like_the_games_rounded_float() {
        use crate::species_panel::brain::neuron_value_text;
        assert_eq!(neuron_value_text(0.0), "0");
        assert_eq!(neuron_value_text(1.0), "1");
        assert_eq!(neuron_value_text(0.5321), "0.53");
        assert_eq!(neuron_value_text(-0.2), "-0.2");
        assert_eq!(neuron_value_text(0.126), "0.13");
        // `Mathf.Round` rounds half to even, and a tiny negative value is `-0` (.NET Core prints negative zero's sign).
        assert_eq!(neuron_value_text(0.125), "0.12");
        assert_eq!(neuron_value_text(-0.001), "-0");
        assert_eq!(neuron_value_text(12.349), "12.35");
    }
}

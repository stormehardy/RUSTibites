//! The Expanded Brain View's model (`ExpandedBrainPanel`, `ExUINeuron`, `UISynaps`, `ExpandedBrainParameters`): the selected bibite's brain
//! as a graph of neurons and synapses on a huge, zoomable canvas, with a force simulation that moves the neurons about (linear: inputs and
//! outputs keep two columns and swap places, hidden neurons float between them; circular: everything is pulled to the middle), and the
//! highlight of what is reachable from a clicked neuron (`BFS`).
//!
//! Everything here is the C#'s, quirks included, because the numbers are what the player sees:
//!
//! * The canvas (`BrainHolder`) is `panelWidth` x `panelHeight` units: the brain's window (`nMax x graphWidthStep` by `nMax x
//!   graphHeightStep`) with 20 000 of padding left and right and 10 000 above and below, so the graph sits in the middle of it. A neuron's
//!   `localPosition` is from the holder's top left corner (`x` right, `y` up, so `y <= 0`).
//! * The simulation runs once per frame with `Time.unscaledDeltaTime`: `Accelerate` adds to a velocity, `Move` adds the velocity to the
//!   neuron's WORLD position (screen pixels: the local position moves by `velocity * dt / (UI scale * zoom)`), or in the linear graph
//!   to the `hiddenPosition` of an input or output (in local units, not divided by anything). Friction is `1 - friction * dt`.
//! * The wheel zoom (`Zoom`) converts `mousePosition - screenCentre` as if it were a screen point, which puts the fixed point of the
//!   zoom half a screen down and to the left of the cursor; the change of the canvas' position uses the unclamped zoom step.
//! * Clicking a neuron highlights the neurons and synapses reachable from it through hidden neurons (`BFS`, `RemoveBranches`,
//!   `drawGraph`): the rest drop to alpha 0.02 (neurons) and 0.01 (enabled synapses).
//!
//! The hidden neurons' first places are random (`Random.Range(0.2, 0.8)`): this port draws them with its own generator.

use bibites_brainsort::{N_INPUTS, N_OUTPUTS, Rng};
use bibites_core::brain::{BrainGraphView, BrainNodeView};

/// `leftPadding` and `rightPadding`.
pub const SIDE_PADDING: f32 = 20_000.0;
/// `topPadding` and `bottomPadding`.
pub const VERTICAL_PADDING: f32 = 10_000.0;
/// `minScale` and `maxScale`.
pub const MIN_SCALE: f32 = 0.1;
/// `maxScale`.
pub const MAX_SCALE: f32 = 5.0;
/// The most a pair of neurons or a neuron and the middle repel each other (`Mathf.Min(100, ...)`).
const MAX_PUSH: f32 = 100.0;

/// `GraphType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GraphType {
    /// The two columns with the hidden neurons between them.
    #[default]
    Linear,
    /// Everything around the middle.
    Circular,
}

/// What `ExUINeuron.type` says about a neuron (`NodeType.Input`, `Output`, `Hidden`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// One of the 33 inputs.
    Input,
    /// One of the 15 outputs.
    Output,
    /// A hidden neuron.
    Hidden,
}

/// One setting of `ExpandedBrainParameters` (a `FloatSetting`). The numbers are the indices of the values array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Param {
    /// `attractionToCenter`, "Gravity to Center".
    GravityToCenter = 0,
    /// `synapseAttraction`.
    SynapseAttraction = 1,
    /// `repulsionFromCenter`.
    RepulsionFromCenter = 2,
    /// `neuronRepulsion`.
    NeuronRepulsion = 3,
    /// `circularFriction`.
    CircularFriction = 4,
    /// `graphWidthStep`.
    GraphWidth = 5,
    /// `graphHeightStep`, "Height Step".
    HeightStep = 6,
    /// `linearSynapseAttraction`.
    LinearSynapseAttraction = 7,
    /// `linearNeuronRepulsion`.
    LinearNeuronRepulsion = 8,
    /// `linearFriction`.
    LinearFriction = 9,
    /// `switchingThreshold`.
    SwitchingThreshold = 10,
}

/// The number of settings.
pub const PARAMS: usize = 11;

/// The rows of "Linear Graph Parameters" (`LinearGraphParameters.Settings`), in the order the game lists them.
pub const LINEAR_PARAMS: [Param; 6] = [
    Param::LinearSynapseAttraction,
    Param::LinearNeuronRepulsion,
    Param::GraphWidth,
    Param::HeightStep,
    Param::LinearFriction,
    Param::SwitchingThreshold,
];

/// The rows of "Spherical Graph Parameters".
pub const SPHERICAL_PARAMS: [Param; 5] = [
    Param::SynapseAttraction,
    Param::NeuronRepulsion,
    Param::GravityToCenter,
    Param::RepulsionFromCenter,
    Param::CircularFriction,
];

impl Param {
    /// The setting an index of the values array is.
    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        [
            Self::GravityToCenter,
            Self::SynapseAttraction,
            Self::RepulsionFromCenter,
            Self::NeuronRepulsion,
            Self::CircularFriction,
            Self::GraphWidth,
            Self::HeightStep,
            Self::LinearSynapseAttraction,
            Self::LinearNeuronRepulsion,
            Self::LinearFriction,
            Self::SwitchingThreshold,
        ]
        .get(index)
        .copied()
    }

    /// The index in the values array.
    #[must_use]
    pub fn index(self) -> usize {
        self as usize
    }

    /// `Name`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::GravityToCenter => "Gravity to Center",
            Self::SynapseAttraction | Self::LinearSynapseAttraction => "Synaptic Attraction",
            Self::RepulsionFromCenter => "Repulsion from Center",
            Self::NeuronRepulsion | Self::LinearNeuronRepulsion => "Neuron Repulsion",
            Self::CircularFriction | Self::LinearFriction => "Friction",
            Self::GraphWidth => "Graph Width",
            Self::HeightStep => "Height Step",
            Self::SwitchingThreshold => "Switching Threshold",
        }
    }

    /// `HelperText`, the tooltip of the row's name.
    #[must_use]
    pub fn help(self) -> &'static str {
        match self {
            Self::GravityToCenter => {
                "How attracted each node is to the center of the panel.  A higher value will increase the force at which nodes are pulled to the center"
            }
            Self::SynapseAttraction | Self::LinearSynapseAttraction => {
                "How strongly nodes are pulled toward each other if they are connected by a synaptic connection."
            }
            Self::RepulsionFromCenter => {
                "How strongly input and output nodes are repulsed from the center.  Since this does not apply to hidden nodes this is a good tool to create separation between inner and outer nodes"
            }
            Self::NeuronRepulsion | Self::LinearNeuronRepulsion => {
                "How strongly neurons repulse one another."
            }
            Self::CircularFriction | Self::LinearFriction => {
                "This is the friction applied to all nodes as they move.  A higher friction will slow nodes down to a steady state faster but may cause the graph to settle for less optimized layouts"
            }
            Self::GraphWidth => {
                "The distance between the column of input neurons and the column of output neurons"
            }
            Self::HeightStep => "The step size between rows of input and output nodes",
            Self::SwitchingThreshold => {
                "The threshold of force required before input and output nodes will swap position.  Too low of a value will be unstable while too high will prevent the graph from finding an optimized layout"
            }
        }
    }

    /// `DefaultValue`.
    #[must_use]
    pub fn default_value(self) -> f32 {
        match self {
            Self::GravityToCenter => 90.0,
            Self::SynapseAttraction => 0.3,
            Self::RepulsionFromCenter => 1500.0,
            Self::NeuronRepulsion | Self::LinearNeuronRepulsion => 1000.0,
            Self::CircularFriction | Self::LinearFriction => 1.0,
            Self::GraphWidth => 200.0,
            Self::HeightStep => 150.0,
            Self::LinearSynapseAttraction => 1.5,
            Self::SwitchingThreshold => 0.5,
        }
    }

    /// `minValue`.
    #[must_use]
    pub fn min(self) -> f32 {
        match self {
            Self::GravityToCenter => 10.0,
            Self::SynapseAttraction => 0.01,
            Self::RepulsionFromCenter => 1.0,
            Self::NeuronRepulsion | Self::LinearNeuronRepulsion => 20.0,
            Self::CircularFriction | Self::LinearFriction => 0.1,
            Self::GraphWidth | Self::HeightStep => 50.0,
            Self::LinearSynapseAttraction => 0.1,
            Self::SwitchingThreshold => -0.75,
        }
    }

    /// `maxValue`.
    #[must_use]
    pub fn max(self) -> f32 {
        match self {
            Self::GravityToCenter => 150.0,
            Self::SynapseAttraction => 2.0,
            Self::RepulsionFromCenter => 15_000.0,
            Self::NeuronRepulsion => 5000.0,
            Self::LinearNeuronRepulsion => 4000.0,
            Self::CircularFriction => 5.0,
            Self::LinearFriction => 3.0,
            Self::GraphWidth => 1000.0,
            Self::HeightStep => 500.0,
            Self::LinearSynapseAttraction => 5.0,
            Self::SwitchingThreshold => 2.0,
        }
    }

    /// `precision`.
    #[must_use]
    pub fn precision(self) -> i32 {
        match self {
            Self::RepulsionFromCenter
            | Self::NeuronRepulsion
            | Self::GraphWidth
            | Self::HeightStep
            | Self::LinearNeuronRepulsion => 1,
            _ => 2,
        }
    }
}

/// The values of every setting at their defaults (`ResetValue`).
#[must_use]
pub fn default_params() -> [f32; PARAMS] {
    let mut values = [0.0; PARAMS];
    for (index, value) in values.iter_mut().enumerate() {
        if let Some(param) = Param::from_index(index) {
            *value = param.default_value();
        }
    }
    values
}

/// An `ExUINeuron`.
#[derive(Clone, Debug)]
pub struct Neuron {
    /// The node's index in the brain (`ExUINeuron.index`).
    pub index: usize,
    /// `type`.
    pub kind: Kind,
    /// `rank`: the place in its column (an input or an output; 0 for a hidden neuron).
    pub rank: usize,
    /// `transform.localPosition` in the neuron holder: `x` from its left edge, `y` up from its top edge (so `y <= 0`).
    pub pos: [f32; 2],
    /// `velocity`, in world pixels a second.
    pub velocity: [f32; 2],
    /// `hiddenPosition`: where an input or output of the linear graph would be, in local units.
    pub hidden_pos: [f32; 2],
    /// `beingDragged`.
    pub dragged: bool,
    /// `hidden` (`SetAlphaLow(true)`): the neuron is dimmed.
    pub low: bool,
    /// `SetAlphaLow` has been called once (it sets the sign icon's alpha to 1 or 0.02, replacing the one `SetDesc` gave it).
    pub touched: bool,
    /// `SetValue(Nodes[i].LastOutput)`: what tints it.
    pub value: f32,
    /// The node as the brain has it (its tooltip, its icon, its bias).
    pub node: BrainNodeView,
    /// The synapses that end here (indices of [`ExpandedBrain::synapses`]).
    pub in_synapses: Vec<usize>,
    /// The synapses that start here.
    pub out_synapses: Vec<usize>,
}

/// A `UISynaps`.
#[derive(Clone, Debug)]
pub struct Synapse {
    /// The synapse's index in the brain (`UISynaps.index`).
    pub index: usize,
    /// The neuron it starts at (an index of [`ExpandedBrain::neurons`]).
    pub node_in: usize,
    /// The neuron it ends at.
    pub node_out: usize,
    /// `value`.
    pub weight: f32,
    /// `!disabled`.
    pub enabled: bool,
    /// `thickness` (10 ..= 16, by the weight).
    pub thickness: f32,
    /// `setAlphaLow(true)`: dimmed to alpha 0.01 (an enabled synapse only).
    pub low: bool,
    /// `hidden` (`ToggleShowDisabled`): a disabled synapse that is not shown.
    pub hidden: bool,
}

/// What `RectTransformUtility` and the canvas scale make of the window: its size in pixels and the UI's scale (`totalUIScale`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// The window's size in pixels.
    pub size: [f32; 2],
    /// `UserSettings.totalUIScale`: how many pixels a canvas unit is.
    pub ui: f32,
}

/// The panel.
#[derive(Clone, Debug)]
pub struct ExpandedBrain {
    /// The neurons that have a connection, in the brain's order (`ExUINeurons`).
    pub neurons: Vec<Neuron>,
    /// The synapses, in the brain's order (`UISynapses`).
    pub synapses: Vec<Synapse>,
    /// The values of `ExpandedBrainParameters`.
    pub params: [f32; PARAMS],
    /// `graphType`.
    pub graph: GraphType,
    /// `runForceGraph`.
    pub running: bool,
    /// `currentScale`.
    pub scale: f32,
    /// `BrainHolder.anchoredPosition` (`y` up), in canvas units.
    pub pan: [f32; 2],
    /// `panelWidth`, `panelHeight`: the size of the holder.
    pub panel: [f32; 2],
    window: [f32; 2],
    n_in: usize,
    n_out: usize,
    n_max: usize,
    input_step: f32,
    output_step: f32,
    /// A brain was there to show (`brain != null && brain.isReady`).
    pub ready: bool,
}

/// `Vector2.normalized`: zero for a vector that is too short to have a direction.
fn normalized(v: [f32; 2]) -> [f32; 2] {
    let length = v[0].hypot(v[1]);
    if length > 1e-5 {
        [v[0] / length, v[1] / length]
    } else {
        [0.0, 0.0]
    }
}

fn length(v: [f32; 2]) -> f32 {
    v[0].hypot(v[1])
}

impl ExpandedBrain {
    /// `OpenPanel`: a neuron for every node that has a connection, put in its column (inputs left, outputs right, hidden neurons at a
    /// random place between them), a synapse for every connection.
    #[must_use]
    pub fn open(
        view: Option<&BrainGraphView>,
        params: [f32; PARAMS],
        graph: GraphType,
        pan: [f32; 2],
        rng: &mut Rng,
    ) -> Self {
        let mut panel = Self {
            neurons: Vec::new(),
            synapses: Vec::new(),
            params,
            graph,
            running: false,
            scale: 0.5,
            pan,
            panel: [0.0, 0.0],
            window: [0.0, 0.0],
            n_in: 0,
            n_out: 0,
            n_max: 0,
            input_step: 0.0,
            output_step: 0.0,
            ready: false,
        };
        let Some(view) = view else {
            return panel;
        };
        panel.ready = true;
        // `ComputeActiveNeurons`: the degrees ignore a node's loop on itself.
        let nodes = view.nodes.len();
        let mut degree_in = vec![0_usize; nodes];
        let mut degree_out = vec![0_usize; nodes];
        for synapse in &view.synapses {
            if synapse.node_in != synapse.node_out
                && synapse.node_in < nodes
                && synapse.node_out < nodes
            {
                degree_out[synapse.node_in] += 1;
                degree_in[synapse.node_out] += 1;
            }
        }
        let base = N_INPUTS + N_OUTPUTS;
        let hidden = nodes.saturating_sub(base);
        panel.n_in = (0..N_INPUTS.min(nodes))
            .filter(|&node| degree_out[node] > 0)
            .count();
        panel.n_out = (0..base.min(nodes))
            .filter(|&node| degree_in[node] > 0)
            .count();
        panel.n_max = panel.n_in.max(panel.n_out);
        let n_max = panel.n_max as f32;
        let (width_step, height_step) = (
            panel.param(Param::GraphWidth),
            panel.param(Param::HeightStep),
        );
        panel.window = [n_max * width_step, n_max * height_step];
        panel.panel = [
            panel.window[0] + 2.0 * SIDE_PADDING,
            panel.window[1] + 2.0 * VERTICAL_PADDING,
        ];
        panel.scale = (5.0 / n_max).clamp(MIN_SCALE, MAX_SCALE);
        panel.input_step = panel.window[1] / (panel.n_in.saturating_sub(1).max(1) as f32);
        panel.output_step = panel.window[1] / (panel.n_out.saturating_sub(1).max(1) as f32);
        let hidden_step = panel.window[1] / (hidden.max(1) as f32);
        let (mut inputs, mut outputs, mut hiddens) = (0_usize, 0_usize, 1_usize);
        for (node, data) in view.nodes.iter().enumerate() {
            if degree_in[node] == 0 && degree_out[node] == 0 {
                continue;
            }
            let (kind, rank, pos) = if node < N_INPUTS {
                let rank = inputs;
                inputs += 1;
                (
                    Kind::Input,
                    rank,
                    [
                        SIDE_PADDING,
                        -VERTICAL_PADDING - rank as f32 * panel.input_step,
                    ],
                )
            } else if node < base {
                let rank = outputs;
                outputs += 1;
                (
                    Kind::Output,
                    rank,
                    [
                        SIDE_PADDING + panel.window[0],
                        -VERTICAL_PADDING - rank as f32 * panel.output_step,
                    ],
                )
            } else {
                let at = hiddens;
                hiddens += 1;
                (
                    Kind::Hidden,
                    0,
                    [
                        SIDE_PADDING + panel.window[0] * rng.range(0.2, 0.8),
                        -VERTICAL_PADDING - at as f32 * hidden_step,
                    ],
                )
            };
            panel.neurons.push(Neuron {
                index: node,
                kind,
                rank,
                pos,
                velocity: [0.0, 0.0],
                // `Start()`: the place it was made at.
                hidden_pos: pos,
                dragged: false,
                low: false,
                touched: false,
                value: data.last_output,
                node: data.clone(),
                in_synapses: Vec::new(),
                out_synapses: Vec::new(),
            });
        }
        for (position, synapse) in view.synapses.iter().enumerate() {
            let find = |node: usize| panel.neurons.iter().position(|n| n.index == node);
            // The game takes `ExUINeurons[FindIndex(..)]`: a node with nothing but a loop on itself has no neuron, and it fails there.
            let (Some(node_in), Some(node_out)) = (find(synapse.node_in), find(synapse.node_out))
            else {
                continue;
            };
            let at = panel.synapses.len();
            panel.synapses.push(Synapse {
                index: position,
                node_in,
                node_out,
                weight: synapse.weight,
                enabled: synapse.enabled,
                thickness: synapse_thickness(synapse.weight),
                low: false,
                hidden: false,
            });
            panel.neurons[node_in].out_synapses.push(at);
            panel.neurons[node_out].in_synapses.push(at);
        }
        panel
    }

    /// A setting's value.
    #[must_use]
    pub fn param(&self, param: Param) -> f32 {
        self.params[param.index()]
    }

    /// A setting takes a value as it is (`SetValue`: these settings may go out of their range).
    pub fn set_param(&mut self, param: Param, value: f32) {
        self.params[param.index()] = value;
    }

    /// The index of the neuron of a node of the brain.
    #[must_use]
    pub fn neuron_of(&self, node: usize) -> Option<usize> {
        self.neurons.iter().position(|neuron| neuron.index == node)
    }

    /// The viewport's centre and the unit conversions of the holder as the canvas draws it: where a neuron's local position is in the
    /// window's pixels (`y` down).
    #[must_use]
    pub fn to_screen(&self, local: [f32; 2], view: Viewport) -> [f32; 2] {
        let k = self.scale * view.ui;
        [
            view.size[0] / 2.0 + self.pan[0] * view.ui + (local[0] - self.panel[0] / 2.0) * k,
            view.size[1] / 2.0 - self.pan[1] * view.ui - (self.panel[1] / 2.0 + local[1]) * k,
        ]
    }

    /// The other way: the local position under a window pixel (`y` down), as `ExUINeuron.OnDrag` works it out (not kept inside the
    /// holder: see [`Self::drag_neuron`]).
    #[must_use]
    pub fn to_local(&self, screen: [f32; 2], view: Viewport) -> [f32; 2] {
        let k = self.scale * view.ui;
        [
            (screen[0] - view.size[0] / 2.0 - self.pan[0] * view.ui) / k + self.panel[0] / 2.0,
            -((screen[1] - view.size[1] / 2.0 + self.pan[1] * view.ui) / k + self.panel[1] / 2.0),
        ]
    }

    /// How far the holder can be moved from the middle (`ScrollRect` clamped, the holder being the content and bigger than the window):
    /// the pan's limits in canvas units.
    #[must_use]
    pub fn reach(&self, view: Viewport) -> [f32; 2] {
        let k = self.scale * view.ui;
        [
            ((self.panel[0] * k - view.size[0]) / 2.0).max(0.0) / view.ui,
            ((self.panel[1] * k - view.size[1]) / 2.0).max(0.0) / view.ui,
        ]
    }

    /// The pan kept inside the content's limits.
    #[must_use]
    pub fn clamped(&self, pan: [f32; 2], view: Viewport) -> [f32; 2] {
        let reach = self.reach(view);
        [
            pan[0].clamp(-reach[0], reach[0]),
            pan[1].clamp(-reach[1], reach[1]),
        ]
    }

    /// `Zoom(increment)` with the pointer at `mouse` (window pixels, `y` down). The point the game converts is
    /// `mousePosition - screenCentre` taken as a screen point, so the zoom keeps still the point half a window down and left of the
    /// cursor; the canvas moves by the unclamped step times the point's local position; the scroll rect then keeps it inside its limits.
    pub fn zoom(&mut self, increment: f32, mouse: [f32; 2], view: Viewport) {
        if increment == 0.0 {
            return;
        }
        // The mouse position in Unity's screen space (`y` up), less the centre.
        let up = [mouse[0] - view.size[0] / 2.0, view.size[1] - mouse[1] - view.size[1] / 2.0];
        let k = self.scale * view.ui;
        // `ScreenPointToLocalPointInRectangle`: the point from the holder's pivot, in its own units.
        let local = [
            (up[0] - (view.size[0] / 2.0 + self.pan[0] * view.ui)) / k,
            (up[1] - (view.size[1] / 2.0 + self.pan[1] * view.ui)) / k,
        ];
        let step = increment * self.scale;
        self.scale = (self.scale + step).clamp(MIN_SCALE, MAX_SCALE);
        self.pan = [self.pan[0] - step * local[0], self.pan[1] - step * local[1]];
        self.pan = self.clamped(self.pan, view);
    }

    /// `ExUINeuron.OnDrag` (the left button): the neuron goes to the cursor, kept inside the holder, and is held still by the
    /// simulation while it is dragged.
    pub fn drag_neuron(&mut self, neuron: usize, cursor: [f32; 2], view: Viewport) {
        let local = self.to_local(cursor, view);
        let (width, height) = (self.panel[0], self.panel[1]);
        if let Some(neuron) = self.neurons.get_mut(neuron) {
            neuron.dragged = true;
            neuron.pos = [local[0].clamp(0.0, width), local[1].clamp(-height, 0.0)];
        }
    }

    /// `OnEndDrag`.
    pub fn release_neuron(&mut self, neuron: usize) {
        if let Some(neuron) = self.neurons.get_mut(neuron) {
            neuron.dragged = false;
        }
    }

    /// `StartForce`, `HaltForce`.
    pub fn set_running(&mut self, running: bool) {
        self.running = running;
    }

    /// `ToggleGraphType` (its holder swap is the UI's).
    pub fn toggle_graph(&mut self) {
        self.graph = match self.graph {
            GraphType::Linear => GraphType::Circular,
            GraphType::Circular => GraphType::Linear,
        };
    }

    // ---------------------------------------------------------------------------------------------- the simulation

    /// `UpdatePanel` while the force graph runs: the forces of the graph type, then every neuron moves.
    pub fn step(&mut self, dt: f32, view: Viewport) {
        if !self.running {
            return;
        }
        match self.graph {
            GraphType::Linear => self.apply_linear_force(dt),
            GraphType::Circular => self.apply_spherical_force(dt),
        }
        let unit = (self.scale * view.ui).max(f32::EPSILON);
        let (circular_friction, linear_friction) = (
            self.param(Param::CircularFriction),
            self.param(Param::LinearFriction),
        );
        let graph = self.graph;
        for neuron in &mut self.neurons {
            if neuron.dragged {
                continue;
            }
            let (friction, in_hidden_position) = match graph {
                GraphType::Circular => (circular_friction, false),
                GraphType::Linear => (linear_friction, neuron.kind != Kind::Hidden),
            };
            if in_hidden_position {
                neuron.hidden_pos[0] += neuron.velocity[0] * dt;
                neuron.hidden_pos[1] += neuron.velocity[1] * dt;
            } else {
                // `transform.position += velocity * dt`: the world moves, the local position by that over the scales.
                neuron.pos[0] += neuron.velocity[0] * dt / unit;
                neuron.pos[1] += neuron.velocity[1] * dt / unit;
            }
            neuron.velocity[0] *= 1.0 - friction * dt;
            neuron.velocity[1] *= 1.0 - friction * dt;
        }
    }

    /// `ExUINeuron.Accelerate`.
    fn accelerate(&mut self, neuron: usize, acceleration: [f32; 2], dt: f32) {
        let neuron = &mut self.neurons[neuron];
        if !neuron.dragged {
            neuron.velocity[0] += acceleration[0] * dt;
            neuron.velocity[1] += acceleration[1] * dt;
        }
    }

    /// The push of one neuron on another: `Min(100, repulsion / (distance x 0.04)^2)` along the line between them.
    fn push(repulsion: f32, from: [f32; 2], to: [f32; 2]) -> [f32; 2] {
        let delta = [from[0] - to[0], from[1] - to[1]];
        let distance = length(delta);
        let force = (repulsion / (distance * 0.04).powi(2)).min(MAX_PUSH);
        let direction = normalized(delta);
        [force * direction[0], force * direction[1]]
    }

    /// `ApplySynapticForce`: a synapse that is not disabled pulls its two ends together.
    fn apply_synaptic_force(&mut self, attraction: f32, dt: f32) {
        for at in 0..self.synapses.len() {
            let synapse = &self.synapses[at];
            if !synapse.enabled {
                continue;
            }
            let (from, to) = (synapse.node_in, synapse.node_out);
            let delta = [
                self.neurons[from].pos[0] - self.neurons[to].pos[0],
                self.neurons[from].pos[1] - self.neurons[to].pos[1],
            ];
            let force = length(delta).sqrt() * (synapse.thickness / 6.5) * attraction;
            let direction = normalized(delta);
            self.accelerate(from, [-force * direction[0], -force * direction[1]], dt);
            self.accelerate(to, [force * direction[0], force * direction[1]], dt);
        }
    }

    /// `ApplySphericalForceGraph`: everything is pulled to the middle of the holder, inputs and outputs pushed from it, neurons push each
    /// other, synapses pull their ends together.
    fn apply_spherical_force(&mut self, dt: f32) {
        let centre = [self.panel[0] * 0.5, self.panel[1] * -0.5];
        let (attraction, repulsion_from_centre, neuron_repulsion) = (
            self.param(Param::GravityToCenter),
            self.param(Param::RepulsionFromCenter),
            self.param(Param::NeuronRepulsion),
        );
        for at in 0..self.neurons.len() {
            let (pos, kind, index) = {
                let neuron = &self.neurons[at];
                (neuron.pos, neuron.kind, neuron.index)
            };
            let to_centre = [centre[0] - pos[0], centre[1] - pos[1]];
            let force = length(to_centre).sqrt() * attraction / 46.0;
            let direction = normalized(to_centre);
            self.accelerate(at, [force * direction[0], force * direction[1]], dt);
            if kind != Kind::Hidden {
                let away = [pos[0] - centre[0], pos[1] - centre[1]];
                let force =
                    (repulsion_from_centre / (length(away) * 0.004).powi(2)).min(MAX_PUSH);
                let direction = normalized(away);
                self.accelerate(at, [force * direction[0], force * direction[1]], dt);
            }
            for other in 0..self.neurons.len() {
                if self.neurons[other].index == index {
                    continue;
                }
                let push = Self::push(neuron_repulsion, pos, self.neurons[other].pos);
                self.accelerate(at, push, dt);
            }
        }
        let attraction = self.param(Param::SynapseAttraction);
        self.apply_synaptic_force(attraction, dt);
    }

    /// `ApplyLinearForceGraph`: neurons push each other (an input or an output from its `hiddenPosition`), inputs and outputs trade
    /// places in their column when the push on their `hiddenPosition` is strong enough, and the columns are laid out by rank.
    fn apply_linear_force(&mut self, dt: f32) {
        let repulsion = self.param(Param::LinearNeuronRepulsion);
        for at in 0..self.neurons.len() {
            for other in 0..self.neurons.len() {
                if self.neurons[at].index == self.neurons[other].index {
                    continue;
                }
                let from = if self.neurons[at].kind == Kind::Hidden {
                    self.neurons[at].pos
                } else {
                    self.neurons[at].hidden_pos
                };
                let push = Self::push(repulsion, from, self.neurons[other].pos);
                self.accelerate(at, push, dt);
            }
        }
        let threshold = self.param(Param::SwitchingThreshold);
        let height_step = self.param(Param::HeightStep);
        let width = self.n_max as f32 * self.param(Param::GraphWidth);
        self.window[0] = width;
        self.input_step = self.n_max as f32 * height_step / (self.n_in as f32 - 1.0).max(1.0);
        self.arrange_column(Kind::Input, SIDE_PADDING, threshold);
        self.output_step = self.n_max as f32 * height_step / (self.n_out as f32 - 1.0).max(1.0);
        self.arrange_column(Kind::Output, SIDE_PADDING + width, threshold);
        let attraction = self.param(Param::LinearSynapseAttraction);
        self.apply_synaptic_force(attraction, dt);
    }

    /// The input or output loop of `ApplyLinearForceGraph`: a neuron whose next neighbour's `hiddenPosition` is higher by more than
    /// `outputStep x switchingThreshold` (the outputs' step for the inputs too: the game's) trades ranks with it; every neuron is put at its
    /// rank's place, and one whose rank changed takes the place as its `hiddenPosition` and the loop skips the neuron after it.
    fn arrange_column(&mut self, kind: Kind, x: f32, threshold: f32) {
        let members: Vec<usize> = (0..self.neurons.len())
            .filter(|&at| self.neurons[at].kind == kind)
            .collect();
        let (count, step) = if kind == Kind::Input {
            (self.n_in, self.input_step)
        } else {
            (self.n_out, self.output_step)
        };
        let mut i = 0;
        while i < members.len() {
            let at = members[i];
            let current = self.neurons[at].rank;
            if current + 1 < count
                && let Some(next) = members
                    .iter()
                    .copied()
                    .find(|&other| self.neurons[other].rank == current + 1)
                && self.neurons[next].hidden_pos[1] - self.neurons[at].hidden_pos[1]
                    > self.output_step * threshold
            {
                self.neurons[at].rank = current + 1;
                self.neurons[next].rank = current;
                let place = [x, -VERTICAL_PADDING - current as f32 * step];
                self.neurons[next].pos = place;
                self.neurons[next].hidden_pos = place;
                self.neurons[next].velocity = [0.0, 0.0];
                self.neurons[at].velocity = [0.0, 0.0];
            }
            let rank = self.neurons[at].rank;
            self.neurons[at].pos = [x, -VERTICAL_PADDING - rank as f32 * step];
            if rank != current {
                self.neurons[at].hidden_pos = self.neurons[at].pos;
                i += 1;
            }
            i += 1;
        }
    }

    // ---------------------------------------------------------------------------------------------- the highlight

    /// `BFS(sourceNode)`: what a click on a neuron highlights.
    pub fn bfs(&mut self, source: usize) {
        let Some(source_at) = self.neuron_of(source) else {
            return;
        };
        let source_kind = self.neurons[source_at].kind;
        let mut next_nodes: Vec<usize> = vec![source];
        let mut next_synapses: Vec<usize> = Vec::new();
        let mut reachable_nodes: Vec<usize> = Vec::new();
        let mut reachable_synapses: Vec<usize> = Vec::new();
        while !next_nodes.is_empty() || !next_synapses.is_empty() {
            for &node in &next_nodes {
                let Some(at) = self.neuron_of(node) else {
                    continue;
                };
                let neuron = &self.neurons[at];
                if node == source || neuron.kind == Kind::Hidden {
                    reachable_nodes.push(node);
                    for &synapse in neuron.in_synapses.iter().chain(&neuron.out_synapses) {
                        let index = self.synapses[synapse].index;
                        if !reachable_synapses.contains(&index) && !next_synapses.contains(&index) {
                            next_synapses.push(index);
                        }
                    }
                } else if neuron.kind != source_kind {
                    reachable_nodes.push(node);
                }
            }
            next_nodes.clear();
            for &index in &next_synapses {
                let Some(synapse) = self.synapse_of(index) else {
                    continue;
                };
                if synapse.enabled {
                    reachable_synapses.push(index);
                    for end in [synapse.node_in, synapse.node_out] {
                        let node = self.neurons[end].index;
                        if !reachable_nodes.contains(&node) && !next_nodes.contains(&node) {
                            next_nodes.push(node);
                        }
                    }
                }
            }
            next_synapses.clear();
        }
        self.remove_branches(source, &mut reachable_nodes, &mut reachable_synapses);
        self.draw_graph(&reachable_nodes, &reachable_synapses);
    }

    /// The synapse with an index of the brain.
    fn synapse_of(&self, index: usize) -> Option<&Synapse> {
        self.synapses.iter().find(|synapse| synapse.index == index)
    }

    /// `RemoveBranches`: hidden neurons that have lost all their reachable synapses in or out are dropped, then the synapses with a dropped
    /// end (and the disabled ones), until nothing changes.
    fn remove_branches(&self, source: usize, nodes: &mut Vec<usize>, synapses: &mut Vec<usize>) {
        let mut settled = false;
        while !settled {
            settled = true;
            let mut remove: Vec<usize> = Vec::new();
            for &node in nodes.iter() {
                let Some(at) = self.neuron_of(node) else {
                    continue;
                };
                let neuron = &self.neurons[at];
                if neuron.kind != Kind::Hidden || node == source {
                    continue;
                }
                let reaches = |list: &[usize]| {
                    list.iter()
                        .any(|&synapse| synapses.contains(&self.synapses[synapse].index))
                };
                if !reaches(&neuron.out_synapses) {
                    remove.push(node);
                    settled = false;
                }
                if !reaches(&neuron.in_synapses) {
                    remove.push(node);
                    settled = false;
                }
            }
            nodes.retain(|node| !remove.contains(node));
            let mut remove: Vec<usize> = Vec::new();
            for &index in synapses.iter() {
                let Some(synapse) = self.synapse_of(index) else {
                    continue;
                };
                if synapse.enabled {
                    if !nodes.contains(&self.neurons[synapse.node_out].index) {
                        remove.push(index);
                        settled = false;
                    }
                    if !nodes.contains(&self.neurons[synapse.node_in].index) {
                        remove.push(index);
                        settled = false;
                    }
                } else {
                    remove.push(index);
                    settled = false;
                }
            }
            synapses.retain(|index| !remove.contains(index));
        }
    }

    /// `drawGraph`: everything dims, the reachable neurons and synapses do not.
    fn draw_graph(&mut self, nodes: &[usize], synapses: &[usize]) {
        for neuron in &mut self.neurons {
            neuron.set_alpha_low(true);
        }
        for synapse in &mut self.synapses {
            synapse.set_alpha_low(true);
        }
        for &node in nodes {
            if let Some(at) = self.neuron_of(node) {
                self.neurons[at].set_alpha_low(false);
            }
        }
        for &index in synapses {
            if let Some(at) = self.synapses.iter().position(|synapse| synapse.index == index) {
                self.synapses[at].set_alpha_low(false);
            }
        }
    }

    /// `resetBFS`: nothing is dimmed.
    pub fn reset_highlight(&mut self) {
        for neuron in &mut self.neurons {
            neuron.set_alpha_low(false);
        }
        for synapse in &mut self.synapses {
            synapse.set_alpha_low(false);
        }
    }

    /// `DisableSynapse`: the disabled synapses show or hide (each toggles its own flag).
    pub fn toggle_disabled(&mut self) {
        for synapse in &mut self.synapses {
            if !synapse.enabled {
                synapse.hidden = !synapse.hidden;
            }
        }
    }
}

impl Neuron {
    /// `UINeuron.SetAlphaLow`: alpha 0.02 when dimmed, 1 otherwise.
    fn set_alpha_low(&mut self, low: bool) {
        self.low = low;
        self.touched = true;
    }

    /// `alphaFloat`.
    #[must_use]
    pub fn alpha(&self) -> f32 {
        if self.low { 0.02 } else { 1.0 }
    }
}

impl Synapse {
    /// `UISynaps.setAlphaLow`: only an enabled synapse dims (to 0.01) or comes back.
    fn set_alpha_low(&mut self, low: bool) {
        if self.enabled {
            self.low = low;
        }
    }

    /// The alpha of the synapse's tint: 0.01 when dimmed, the 0.25 of a disabled one, else 1.
    #[must_use]
    pub fn alpha(&self) -> f32 {
        if !self.enabled {
            0.25
        } else if self.low {
            0.01
        } else {
            1.0
        }
    }
}

/// `UISynaps.SetSynapseThickness`.
#[must_use]
pub fn synapse_thickness(weight: f32) -> f32 {
    let clamped = weight.abs().clamp(0.0, 25.0);
    let e = (clamped / 2.0).exp();
    (e - 1.0) / (1.0 + e) * 6.0 + 10.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use bibites_core::brain::{Brain, BrainSynapseView, NodeType};

    fn node(index: usize, last_output: f32) -> BrainNodeView {
        BrainNodeView {
            index,
            node_type: if index >= N_INPUTS + N_OUTPUTS || index >= N_INPUTS {
                NodeType::Sigmoid
            } else {
                NodeType::Input
            },
            innovation: index as i64,
            label: format!("node {index}"),
            base_activation: 0.0,
            value: last_output,
            last_input: 0.0,
            last_output,
        }
    }

    fn synapse(index: usize, from: usize, to: usize, weight: f32, enabled: bool) -> BrainSynapseView {
        BrainSynapseView {
            index,
            innovation: 100 + index as i64,
            node_in: from,
            node_out: to,
            weight,
            enabled,
        }
    }

    /// Inputs 0 and 1, outputs 33 and 34, hidden nodes 48 and 49: 0 -> 48 -> 33, 1 -> 48, 0 -> 34 (disabled), 1 -> 49 -> 34, and 49 -> 49
    /// (a loop does not count for a degree).
    fn small_view() -> BrainGraphView {
        let nodes: Vec<BrainNodeView> = (0..50).map(|index| node(index, 0.0)).collect();
        BrainGraphView {
            nodes,
            synapses: vec![
                synapse(0, 0, 48, 1.0, true),
                synapse(1, 48, 33, 2.0, true),
                synapse(2, 1, 48, -1.5, true),
                synapse(3, 0, 34, 0.5, false),
                synapse(4, 1, 49, 1.0, true),
                synapse(5, 49, 34, 1.0, true),
                synapse(6, 49, 49, 1.0, true),
            ],
            outputs: [0.0; N_OUTPUTS],
        }
    }

    fn viewport() -> Viewport {
        Viewport {
            size: [1920.0, 1080.0],
            ui: 1.0,
        }
    }

    fn open(view: &BrainGraphView) -> ExpandedBrain {
        ExpandedBrain::open(
            Some(view),
            default_params(),
            GraphType::Linear,
            [0.0, 0.0],
            &mut Rng::new(7),
        )
    }

    #[test]
    fn the_defaults_are_the_settings_of_expanded_brain_parameters() {
        let params = default_params();
        assert_eq!(params[Param::GravityToCenter.index()], 90.0);
        assert_eq!(params[Param::SynapseAttraction.index()], 0.3);
        assert_eq!(params[Param::LinearSynapseAttraction.index()], 1.5);
        assert_eq!(params[Param::SwitchingThreshold.index()], 0.5);
        for index in 0..PARAMS {
            let param = Param::from_index(index).expect("a setting");
            assert_eq!(param.index(), index);
            assert!(param.min() < param.max(), "{param:?}");
            assert!(
                (param.min()..=param.max()).contains(&param.default_value()),
                "{param:?}"
            );
            assert!(!param.name().is_empty() && !param.help().is_empty());
        }
        assert!(Param::from_index(PARAMS).is_none());
        // The two groups list the game's rows.
        assert_eq!(LINEAR_PARAMS.len(), 6);
        assert_eq!(SPHERICAL_PARAMS.len(), 5);
    }

    #[test]
    fn opening_puts_inputs_left_outputs_right_and_the_hidden_between() {
        let panel = open(&small_view());
        // Two active inputs, two active outputs, two hidden: the 49 with a loop still has a connection in and out.
        assert_eq!((panel.n_in, panel.n_out, panel.n_max), (2, 2, 2));
        assert_eq!(panel.neurons.len(), 6);
        // The window is nMax x (200, 150); the holder has the paddings around it.
        assert_eq!(panel.window, [400.0, 300.0]);
        assert_eq!(panel.panel, [400.0 + 40_000.0, 300.0 + 20_000.0]);
        // `5 / nMax`.
        assert_eq!(panel.scale, 2.5);
        let input = &panel.neurons[panel.neuron_of(1).expect("input 1")];
        assert_eq!(
            (input.kind, input.rank, input.pos),
            (Kind::Input, 1, [20_000.0, -10_000.0 - 300.0])
        );
        let output = &panel.neurons[panel.neuron_of(33).expect("output")];
        assert_eq!(output.pos, [20_400.0, -10_000.0]);
        for index in [48, 49] {
            let hidden = &panel.neurons[panel.neuron_of(index).expect("hidden")];
            assert_eq!(hidden.kind, Kind::Hidden);
            assert!(hidden.pos[0] >= 20_000.0 + 0.2 * 400.0 && hidden.pos[0] <= 20_000.0 + 0.8 * 400.0);
            assert_eq!(hidden.hidden_pos, hidden.pos);
        }
        // Every synapse has two ends, and the neurons know theirs.
        assert_eq!(panel.synapses.len(), 7);
        let hidden = panel.neuron_of(48).expect("hidden");
        assert_eq!(panel.neurons[hidden].in_synapses.len(), 2);
        assert_eq!(panel.neurons[hidden].out_synapses.len(), 1);
        // Thickness: a weight 0 synapse is 10 thick; a big one tends to 16.
        assert!((synapse_thickness(0.0) - 10.0).abs() < 1e-6);
        assert!(synapse_thickness(100.0) < 16.0 && synapse_thickness(100.0) > 15.9);
    }

    #[test]
    fn a_real_brain_opens_with_its_active_nodes_and_every_synapse() {
        let brain = Brain::starter();
        let view = brain.graph_view();
        let panel = open(&view);
        assert!(panel.ready);
        assert_eq!(panel.synapses.len(), view.synapses.len());
        assert!(panel.neurons.len() >= 2);
        // The zoom fits the graph: 5 / nMax, between the limits.
        assert!(panel.scale >= MIN_SCALE && panel.scale <= MAX_SCALE);
        let closed = ExpandedBrain::open(
            None,
            default_params(),
            GraphType::Linear,
            [0.0, 0.0],
            &mut Rng::new(1),
        );
        assert!(!closed.ready && closed.neurons.is_empty());
    }

    #[test]
    fn the_wheel_zooms_about_a_point_half_a_window_from_the_cursor_and_clamps_the_scale_not_the_step() {
        let mut panel = open(&small_view());
        let view = viewport();
        panel.scale = 1.0;
        panel.pan = [0.0, 0.0];
        // The fixed point is the cursor less (W/2, H/2) (and the canvas then moves by step x local): with the cursor at the centre,
        // that is the window's bottom left corner, so zooming in moves the content away from it.
        let before = panel.to_screen([20_200.0, -10_150.0], view);
        panel.zoom(0.1, [960.0, 540.0], view);
        assert!((panel.scale - 1.1).abs() < 1e-6);
        let after = panel.to_screen([20_200.0, -10_150.0], view);
        // The point under (0, 0) of the screen's up space (the bottom left, i.e. window y = 1080) did not move.
        let corner_before = {
            let mut copy = open(&small_view());
            copy.scale = 1.0;
            copy.pan = [0.0, 0.0];
            copy.to_local([0.0, 1080.0], view)
        };
        let still = panel.to_screen(corner_before, view);
        assert!(still[0].abs() < 0.5 && (still[1] - 1080.0).abs() < 0.5, "{still:?}");
        assert!(before != after);
        // At the limit the scale stays but the canvas still moves by the unclamped step.
        panel.scale = MAX_SCALE;
        panel.pan = [0.0, 0.0];
        panel.zoom(0.1, [960.0, 540.0], view);
        assert_eq!(panel.scale, MAX_SCALE);
        // No wheel, nothing.
        let (scale, pan) = (panel.scale, panel.pan);
        panel.zoom(0.0, [0.0, 0.0], view);
        assert_eq!((panel.scale, panel.pan), (scale, pan));
    }

    #[test]
    fn the_canvas_cannot_leave_its_content() {
        let mut panel = open(&small_view());
        let view = viewport();
        panel.scale = 0.1;
        let reach = panel.reach(view);
        assert!(reach[0] > 0.0 && reach[1] > 0.0);
        let far = panel.clamped([1.0e9, -1.0e9], view);
        assert_eq!(far, [reach[0], -reach[1]]);
        // A zoom out keeps it inside the (smaller) content at once.
        panel.scale = 5.0;
        panel.pan = [panel.reach(view)[0], 0.0];
        panel.zoom(-0.9, [500.0, 500.0], view);
        assert!(panel.pan[0] <= panel.reach(view)[0] + 1e-3);
    }

    #[test]
    fn screen_and_local_positions_convert_both_ways() {
        let mut panel = open(&small_view());
        let view = Viewport {
            size: [1600.0, 900.0],
            ui: 0.8,
        };
        panel.scale = 1.7;
        panel.pan = [123.0, -45.0];
        let local = [20_345.0, -10_222.0];
        let screen = panel.to_screen(local, view);
        let back = panel.to_local(screen, view);
        assert!((back[0] - local[0]).abs() < 0.05 && (back[1] - local[1]).abs() < 0.05, "{back:?}");
        // The graph's window is centred in the middle of the window with no pan.
        panel.pan = [0.0, 0.0];
        let middle = panel.to_screen([panel.panel[0] / 2.0, -panel.panel[1] / 2.0], view);
        assert!((middle[0] - 800.0).abs() < 1e-2 && (middle[1] - 450.0).abs() < 1e-2);
    }

    #[test]
    fn a_dragged_neuron_goes_to_the_cursor_and_is_held_still_by_the_simulation() {
        let mut panel = open(&small_view());
        let view = viewport();
        let at = panel.neuron_of(48).expect("hidden");
        let cursor = [700.0, 300.0];
        panel.drag_neuron(at, cursor, view);
        assert!(panel.neurons[at].dragged);
        let screen = panel.to_screen(panel.neurons[at].pos, view);
        assert!((screen[0] - cursor[0]).abs() < 0.1 && (screen[1] - cursor[1]).abs() < 0.1);
        let held = panel.neurons[at].pos;
        panel.set_running(true);
        panel.graph = GraphType::Circular;
        panel.step(0.016, view);
        assert_eq!(panel.neurons[at].pos, held, "it does not move while it is dragged");
        assert_eq!(panel.neurons[at].velocity, [0.0, 0.0], "and gathers no speed");
        let other = panel.neuron_of(49).expect("other");
        assert!(panel.neurons[other].velocity != [0.0, 0.0] || panel.neurons[other].pos != panel.neurons[other].hidden_pos);
        panel.release_neuron(at);
        assert!(!panel.neurons[at].dragged);
        // The holder keeps it: far outside the window it is clamped to the holder's rectangle.
        panel.drag_neuron(at, [-1.0e7, -1.0e7], view);
        assert_eq!(panel.neurons[at].pos[0], 0.0);
        assert_eq!(panel.neurons[at].pos[1], 0.0);
    }

    #[test]
    fn nothing_moves_until_the_force_graph_runs() {
        let mut panel = open(&small_view());
        let before: Vec<[f32; 2]> = panel.neurons.iter().map(|n| n.pos).collect();
        panel.step(0.016, viewport());
        let after: Vec<[f32; 2]> = panel.neurons.iter().map(|n| n.pos).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn the_circular_graph_pulls_a_neuron_to_the_middle_and_pushes_the_outer_ones_out() {
        let mut panel = open(&small_view());
        panel.graph = GraphType::Circular;
        panel.set_running(true);
        let centre = [panel.panel[0] * 0.5, panel.panel[1] * -0.5];
        // A lone hidden neuron far from the middle (the others far away too: their pushes are small) accelerates towards it.
        let at = panel.neuron_of(48).expect("hidden");
        panel.neurons[at].pos = [centre[0] + 5000.0, centre[1] + 3000.0];
        for neuron in &mut panel.neurons {
            neuron.velocity = [0.0, 0.0];
        }
        panel.apply_spherical_force(0.01);
        let v = panel.neurons[at].velocity;
        assert!(v[0] < 0.0 && v[1] < 0.0, "{v:?}");
        // An input is pushed from the middle: it has the repulsion from the centre as well (here the pull wins at distance, but the
        // sum differs from a hidden neuron at the same place).
        let input = panel.neuron_of(0).expect("input");
        let hidden = panel.neuron_of(49).expect("hidden");
        let place = [centre[0] + 50.0, centre[1]];
        panel.neurons[input].pos = place;
        panel.neurons[hidden].pos = place;
        for neuron in &mut panel.neurons {
            neuron.velocity = [0.0, 0.0];
        }
        panel.apply_spherical_force(0.01);
        assert!(
            panel.neurons[input].velocity[0] > panel.neurons[hidden].velocity[0],
            "the input is pushed out: {:?} vs {:?}",
            panel.neurons[input].velocity,
            panel.neurons[hidden].velocity
        );
    }

    #[test]
    fn a_step_moves_the_world_position_by_the_velocity_over_the_scales() {
        let mut panel = open(&small_view());
        panel.graph = GraphType::Circular;
        // One neuron alone, with no force on it.
        panel.synapses.clear();
        panel.neurons.truncate(1);
        panel.params[Param::GravityToCenter.index()] = 0.0;
        panel.params[Param::RepulsionFromCenter.index()] = 0.0;
        panel.scale = 0.5;
        panel.neurons[0].velocity = [100.0, -50.0];
        let before = panel.neurons[0].pos;
        panel.set_running(true);
        panel.step(
            0.1,
            Viewport {
                size: [1920.0, 1080.0],
                ui: 2.0,
            },
        );
        // 100 px/s for 0.1 s is 10 px, which is 10 units over the UI scale 2 and the zoom 0.5.
        let moved = [
            panel.neurons[0].pos[0] - before[0],
            panel.neurons[0].pos[1] - before[1],
        ];
        assert!((moved[0] - 10.0).abs() < 0.01, "{moved:?}");
        assert!((moved[1] + 5.0).abs() < 0.01, "{moved:?}");
        // Friction: `1 - friction x dt`.
        assert!((panel.neurons[0].velocity[0] - 90.0).abs() < 0.01);
    }

    #[test]
    fn inputs_trade_places_when_the_push_on_their_hidden_position_passes_the_threshold() {
        let mut panel = open(&small_view());
        panel.graph = GraphType::Linear;
        panel.set_running(true);
        let (a, b) = (
            panel.neuron_of(0).expect("input 0"),
            panel.neuron_of(1).expect("input 1"),
        );
        assert_eq!((panel.neurons[a].rank, panel.neurons[b].rank), (0, 1));
        // 1 is below 0 in the column; its hidden position is higher than 0's by more than outputStep x threshold (300 x 0.5, the output
        // step the panel was opened with: the inputs' loop runs before the game updates it): they trade.
        panel.neurons[a].hidden_pos[1] = -10_400.0;
        panel.neurons[b].hidden_pos[1] = -10_000.0;
        for neuron in &mut panel.neurons {
            neuron.velocity = [0.0, 0.0];
        }
        panel.apply_linear_force(0.001);
        assert_eq!((panel.neurons[a].rank, panel.neurons[b].rank), (1, 0));
        // Positions follow the ranks (the step is nMax x height step over (inputs - 1)).
        let step = 2.0 * 150.0 / 1.0;
        assert_eq!(panel.neurons[b].pos, [SIDE_PADDING, -VERTICAL_PADDING]);
        assert_eq!(panel.neurons[a].pos, [SIDE_PADDING, -VERTICAL_PADDING - step]);
        // The one that changed rank takes its place as its hidden position too.
        assert_eq!(panel.neurons[a].hidden_pos, panel.neurons[a].pos);
    }

    #[test]
    fn a_linear_step_moves_hidden_positions_of_inputs_and_the_local_position_of_hidden_neurons() {
        let mut panel = open(&small_view());
        panel.graph = GraphType::Linear;
        panel.set_running(true);
        let input = panel.neuron_of(0).expect("input");
        let hidden = panel.neuron_of(48).expect("hidden");
        let (input_pos, hidden_pos) = (panel.neurons[input].pos, panel.neurons[hidden].pos);
        panel.neurons[input].velocity = [50.0, 0.0];
        panel.neurons[hidden].velocity = [50.0, 0.0];
        panel.step(0.02, viewport());
        // The input stays in its column; its hidden position took the move; the hidden neuron moved.
        assert_eq!(panel.neurons[input].pos[0], input_pos[0]);
        assert!(panel.neurons[input].hidden_pos[0] != input_pos[0]);
        assert!(panel.neurons[hidden].pos[0] != hidden_pos[0]);
    }

    #[test]
    fn a_click_highlights_what_is_reachable_through_hidden_neurons() {
        let mut panel = open(&small_view());
        // From input 0: its synapses to 48 (hidden) and to 34 (disabled: not followed); 48 reaches 33 and input 1 through its input
        // synapse (the same type as the source: not highlighted).
        panel.bfs(0);
        let low = |panel: &ExpandedBrain, node: usize| {
            panel.neurons[panel.neuron_of(node).expect("neuron")].low
        };
        assert!(!low(&panel, 0) && !low(&panel, 48) && !low(&panel, 33));
        assert!(low(&panel, 1), "an input of the same type is not reachable");
        assert!(low(&panel, 34), "the output behind a disabled synapse is not");
        assert!(low(&panel, 49));
        let synapse = |panel: &ExpandedBrain, index: usize| panel.synapses[index].low;
        assert!(!synapse(&panel, 0) && !synapse(&panel, 1));
        assert!(synapse(&panel, 2), "1 -> 48: its other end is not highlighted");
        assert!(!synapse(&panel, 3), "a disabled synapse never dims (it has its own alpha)");
        assert!(synapse(&panel, 4) && synapse(&panel, 5));
        assert_eq!(panel.synapses[3].alpha(), 0.25);
        assert_eq!(panel.synapses[4].alpha(), 0.01);
        assert_eq!(panel.neurons[panel.neuron_of(49).expect("n")].alpha(), 0.02);
        // The background click puts everything back.
        panel.reset_highlight();
        assert!(panel.neurons.iter().all(|n| !n.low));
        assert!(panel.synapses.iter().all(|s| !s.low));
        // From a hidden neuron: its inputs and outputs are reachable ends.
        panel.bfs(48);
        assert!(!low(&panel, 0) && !low(&panel, 1) && !low(&panel, 33));
        assert!(low(&panel, 49));
    }

    #[test]
    fn disabled_synapses_toggle_their_visibility() {
        let mut panel = open(&small_view());
        assert!(panel.synapses.iter().all(|s| !s.hidden));
        panel.toggle_disabled();
        let hidden: Vec<usize> = panel
            .synapses
            .iter()
            .filter(|s| s.hidden)
            .map(|s| s.index)
            .collect();
        assert_eq!(hidden, vec![3]);
        panel.toggle_disabled();
        assert!(panel.synapses.iter().all(|s| !s.hidden));
    }
}

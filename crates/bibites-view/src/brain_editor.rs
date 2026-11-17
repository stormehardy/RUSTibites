//! The Bibite Editor's Brain tab, without its drawing (`BibiteBrainEditor`, `NodeEditor`, `ConnectionEditor`, `NodeEditPanel`,
//! `ConnectionEditPanel`, `AddNodePanel`): the nodes and connections that are on the canvas, where they are, what is selected, the
//! edits the panels make, the little simulator behind the play / step buttons, and what the template is saved with.
//!
//! The editor works on the template's node list (the 33 inputs, the 15 outputs, then the hidden nodes) and its synapses. Only the nodes
//! the layout calls used are on the canvas (`BrainSorter.UsedIndicesOfGroups`: the connected ones, and the outputs that are active
//! without a connection); the other inputs and outputs are what the add-node panel offers. Positions are the game's `anchoredPosition`
//! of a node inside its holder: `x` from the left edge, `y` from the top edge and negative downwards, in canvas units.

use std::collections::BTreeMap;

use bibites_brainsort::{Archetype, Layout, Rng, SortNode, SortSynapse, make_groups};
use bibites_core::brain::{
    FIRST_HIDDEN, N_INPUTS, NodeRecord, NodeType, SynapseRecord, execute_neuron,
};

/// The inputs and the outputs: the nodes every brain has (`NInputs + NOutputs`).
pub const BASE_NODES: usize = FIRST_HIDDEN;
/// `BibiteBrainEditor.brainPeriod`: seconds between two steps of the simulator, and the period its nodes execute with.
pub const BRAIN_PERIOD: f32 = 0.1;
/// The window the tests size the canvas by (`Screen.width`, `Screen.height`).
#[cfg(test)]
const SCREEN: [f32; 2] = [1920.0, 1080.0];
/// `stepW`: the width one column of nodes asks for.
const STEP_W: f32 = 150.0;
/// `targetStepV`: the height a group of weight 1 asks for.
const TARGET_STEP_V: f32 = 75.0;
/// `minScale`, `maxScale`.
pub const MIN_ZOOM: f32 = 0.1;
/// See [`MIN_ZOOM`].
pub const MAX_ZOOM: f32 = 5.0;
/// How near the left or right edge of the brain a dragged node sticks to it (`MousePosToStickyNodePos`).
const STICKY: f32 = 10.0;

/// What a template is saved with: its nodes, its connections and the positions it keeps (by node innovation).
pub type TemplateParts = (Vec<NodeRecord>, Vec<SynapseRecord>, BTreeMap<i64, (f32, f32)>);

/// A node's identity on the canvas: stable while its index changes.
pub type NodeKey = u32;
/// A connection's identity on the canvas.
pub type EdgeKey = u32;

/// One node on the canvas (`NodeEditor`).
#[derive(Clone, Debug, PartialEq)]
pub struct EditNode {
    /// Its identity.
    pub key: NodeKey,
    /// `node.Index`: the inputs, then the outputs, then the hidden nodes.
    pub index: usize,
    /// `node.Inov`.
    pub inov: i64,
    /// `node.Type`.
    pub kind: NodeType,
    /// `node.Desc`: a hidden node's label.
    pub desc: String,
    /// `node.baseActivation`: the bias.
    pub bias: f32,
    /// `node.Value`.
    pub value: f32,
    /// `node.LastInput`.
    pub last_input: f32,
    /// `node.LastOutput`.
    pub last_output: f32,
    /// Where it is on the canvas.
    pub pos: [f32; 2],
    /// `NodeEditor.activation`: what its connections have added up this step.
    pub activation: f32,
    /// `NodeEditor.valid`: an input with no connection out, or a hidden node without one in and one out, is not.
    pub valid: bool,
    /// The connections that end here (`ingoings`).
    pub ingoing: Vec<EdgeKey>,
    /// The connections that start here (`outgoings`).
    pub outgoing: Vec<EdgeKey>,
}

impl EditNode {
    /// `node.archetype`.
    #[must_use]
    pub fn archetype(&self) -> Archetype {
        Archetype::of_index(self.index)
    }

    /// The record the template keeps (`Node.SaveForTemplate`: no running state).
    fn record(&self) -> NodeRecord {
        NodeRecord {
            node_type: self.kind,
            index: self.index as u32,
            innovation: self.inov,
            desc: (self.archetype() == Archetype::Hidden).then(|| self.desc.clone()),
            base_activation: self.bias,
            value: 0.0,
            last_input: 0.0,
            last_output: 0.0,
        }
    }

    /// `NodeEditor.ExecuteNode`: the node's function of what it collected, then the collection starts again.
    fn execute(&mut self) {
        if self.archetype() == Archetype::Input {
            return;
        }
        let (output, last_input) = execute_neuron(
            self.kind,
            self.bias,
            self.activation,
            self.last_output,
            self.last_input,
            BRAIN_PERIOD,
        );
        self.value = output;
        self.last_output = output;
        self.last_input = last_input;
        self.activation = if self.kind == NodeType::Mult {
            1.0
        } else {
            0.0
        };
    }

    /// `NodeEditor.RefreshValidity`.
    fn refresh_validity(&mut self) {
        self.valid = match self.archetype() {
            Archetype::Input => !self.outgoing.is_empty(),
            Archetype::Hidden => !self.outgoing.is_empty() && !self.ingoing.is_empty(),
            Archetype::Output => true,
        };
    }
}

/// One connection on the canvas (`ConnectionEditor`).
#[derive(Clone, Debug, PartialEq)]
pub struct EditEdge {
    /// Its identity.
    pub key: EdgeKey,
    /// `connection.Inov`.
    pub inov: i64,
    /// `connection.Weight`.
    pub weight: f32,
    /// `connection.En`.
    pub enabled: bool,
    /// The node it starts at (`nodeIn`); `None` while the new connection waits for one.
    pub from: Option<NodeKey>,
    /// The node it ends at (`nodeOut`); `None` while the new connection waits for one.
    pub to: Option<NodeKey>,
    /// `missingArchetype`: what the end that is missing may be (`Input`: the start, `Output`: the end).
    pub missing: Archetype,
}

/// What the edit panels are about (`BibiteBrainEditor`'s selection).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selected {
    /// A node (`NodeEditPanel`).
    Node(NodeKey),
    /// A connection (`ConnectionEditPanel`).
    Edge(EdgeKey),
}

/// What Escape undoes (`selectAction`, `createConnectionAction`, `addNodeAction`), newest last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Escapable {
    /// The selection goes (`Deselect`).
    Select,
    /// The new connection is dropped (`StopConnecting`).
    CreateConnection,
    /// The add-node panel closes (`CloseAddNodePanel`).
    AddNode,
}

/// The sizes the game derives from the groups (`RefreshDerivedValues`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// `width`: the brain's width.
    pub width: f32,
    /// `height`: the brain's height.
    pub height: f32,
    /// `stepI`: the step of the input column, negative.
    pub step_i: f32,
    /// `stepO`: the step of the output column, negative.
    pub step_o: f32,
    /// `stepG`: the step between the rows of outputs nothing reaches, negative.
    pub step_g: f32,
    /// `nG`: how many groups of outputs nothing reaches.
    pub n_g: f32,
}

/// The Brain tab: the canvas, the selection and the simulator.
#[derive(Clone, Debug)]
pub struct BrainEdit {
    /// The nodes on the canvas (base nodes in the order they were put there, then the hidden ones as they came).
    pub nodes: Vec<EditNode>,
    /// The connections, oldest first (later ones draw over earlier ones).
    pub edges: Vec<EditEdge>,
    /// `nodes` of the game: the node list the editor was filled with, rewritten by [`Self::rebuild_lists`].
    list: Vec<NodeRecord>,
    /// `synapses` of the game.
    synapse_list: Vec<SynapseRecord>,
    next_node: NodeKey,
    next_edge: EdgeKey,
    /// The window the canvas is sized by (`Screen.width`, `Screen.height`).
    screen: [f32; 2],
    /// The anchors the template came with, by node (`anchors`).
    anchors: BTreeMap<NodeKey, [f32; 2]>,
    /// `saveAnchors`: the template keeps where the nodes are.
    pub save_anchors: bool,
    /// `anyChange`.
    pub any_change: bool,
    /// The layout's numbers.
    pub metrics: Metrics,
    /// `brainRT.sizeDelta`: the canvas.
    pub content: [f32; 2],
    /// `usedIndices`, as of the last fill or anchor save (the game does not refresh it elsewhere).
    used: Vec<usize>,
    sorter: Layout,
    rng: Rng,
    /// The element the edit panel is about.
    pub selected: Option<Selected>,
    /// The connection that is being made (`tempConnection`), when [`Self::connecting`].
    pub temp: Option<EdgeKey>,
    /// `connectingMode`.
    pub connecting: bool,
    /// `addNodeMode`: the add-node panel is open.
    pub add_mode: bool,
    /// `addPos`: where the next added node goes.
    pub add_pos: [f32; 2],
    /// What the add-node panel does not offer while a connection waits for an end (`unavailableArchetype`).
    pub add_hides: Archetype,
    /// What Escape undoes, newest last (`UINavigationManager`'s entries of this editor).
    pub escapes: Vec<Escapable>,
    /// `play`: the simulator steps by itself.
    pub playing: bool,
    progress: f32,
    /// What the add-node panel offers of the 48 base nodes (`AddNodePanel.availableInputs`, `availableOutputs`): the ones that are not on the
    /// canvas, and an output that "deleted" stays on it (`DeleteNode` lists it again before keeping it).
    offered: Vec<bool>,
    /// The node panel's own bias (`NodeEditPanel.bias.val`): set when a node is selected and by the bias slider, and left alone when the
    /// function changes (`EditFunction` sets the node's bias, not this), so it can be out of step with the node.
    pub panel_bias: f32,
}

impl BrainEdit {
    /// `FillEditor` for a 1920 x 1080 window: the editor on a template's nodes (`list`: at least the 48 base nodes) and synapses, with
    /// the positions it saved (`anchors`, by node innovation).
    #[cfg(test)]
    #[must_use]
    pub fn fill(
        list: Vec<NodeRecord>,
        synapses: &[SynapseRecord],
        anchors: &BTreeMap<i64, (f32, f32)>,
    ) -> Self {
        Self::fill_for_screen(list, synapses, anchors, SCREEN)
    }

    /// [`Self::fill`] for a window of `screen` pixels (the canvas is sized by `Screen.width` and `Screen.height`).
    #[must_use]
    pub fn fill_for_screen(
        list: Vec<NodeRecord>,
        synapses: &[SynapseRecord],
        anchors: &BTreeMap<i64, (f32, f32)>,
        screen: [f32; 2],
    ) -> Self {
        let mut edit = Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            list,
            synapse_list: synapses.to_vec(),
            next_node: 0,
            next_edge: 0,
            screen,
            anchors: BTreeMap::new(),
            save_anchors: false,
            any_change: false,
            metrics: Metrics {
                width: 0.0,
                height: 0.0,
                step_i: -1.0,
                step_o: -1.0,
                step_g: -1.0,
                n_g: 0.0,
            },
            content: [0.0, 0.0],
            used: Vec::new(),
            sorter: make_groups(&[], &[]),
            rng: Rng::new(0x5EED_B8A1),
            selected: None,
            temp: None,
            connecting: false,
            add_mode: false,
            add_pos: [0.0, 0.0],
            add_hides: Archetype::Hidden,
            escapes: Vec::new(),
            playing: false,
            progress: 0.0,
            offered: vec![true; BASE_NODES],
            panel_bias: 0.0,
        };
        edit.sorter = edit.make_sorter();
        edit.used = edit.sorter.used_indices_of_groups();
        for index in edit.used.clone() {
            let Some(record) = edit.list.get(index).cloned() else {
                continue;
            };
            let key = edit.new_node(&record);
            if index < BASE_NODES {
                edit.offered[index] = false;
            }
            if let Some(anchor) = anchors.get(&record.innovation) {
                edit.anchors.insert(key, [anchor.0, anchor.1]);
            }
        }
        // `newAnchors.Count > usedIndices.Count / 2`.
        edit.save_anchors = anchors.len() > edit.used.len() / 2;
        for synapse in synapses {
            let (Some(from), Some(to)) = (
                edit.key_of_index(synapse.node_in as usize),
                edit.key_of_index(synapse.node_out as usize),
            ) else {
                continue;
            };
            let key = edit.next_edge;
            edit.next_edge += 1;
            edit.edges.push(EditEdge {
                key,
                inov: synapse.innovation,
                weight: synapse.weight,
                enabled: synapse.enabled,
                from: None,
                to: None,
                missing: Archetype::Hidden,
            });
            edit.attach(key, Some(from), Some(to));
        }
        edit.size_panel_and_place_brain();
        edit
    }

    // ------------------------------------------------------------------------------ lookups

    /// The node with this key.
    #[must_use]
    pub fn node(&self, key: NodeKey) -> Option<&EditNode> {
        self.nodes.iter().find(|node| node.key == key)
    }

    fn node_mut(&mut self, key: NodeKey) -> Option<&mut EditNode> {
        self.nodes.iter_mut().find(|node| node.key == key)
    }

    /// The connection with this key.
    #[must_use]
    pub fn edge(&self, key: EdgeKey) -> Option<&EditEdge> {
        self.edges.iter().find(|edge| edge.key == key)
    }

    fn edge_mut(&mut self, key: EdgeKey) -> Option<&mut EditEdge> {
        self.edges.iter_mut().find(|edge| edge.key == key)
    }

    /// `GetNode(index)`: the base node of that index, or the first hidden node that has it.
    fn key_of_index(&self, index: usize) -> Option<NodeKey> {
        self.nodes
            .iter()
            .find(|node| node.index == index)
            .map(|node| node.key)
    }

    /// The hidden nodes on the canvas (`hiddenNodes.activeCount`).
    #[must_use]
    pub fn hidden_count(&self) -> usize {
        self.nodes
            .iter()
            .filter(|node| node.archetype() == Archetype::Hidden)
            .count()
    }

    /// The inputs and outputs the add-node panel offers: those that are not on the canvas, and an output that was deleted but stayed
    /// (`SetIndexAvailability`).
    #[must_use]
    pub fn available(&self, archetype: Archetype) -> Vec<usize> {
        (0..BASE_NODES)
            .filter(|&index| Archetype::of_index(index) == archetype)
            .filter(|&index| self.offered[index])
            .collect()
    }

    /// `valid`: every node on the canvas is (`CheckAllValid`). The Save buttons and Resort need it.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.nodes.iter().all(|node| node.valid)
    }

    /// The nodes a connection runs between, if both ends are there.
    #[must_use]
    pub fn ends_of(&self, key: EdgeKey) -> Option<(&EditNode, &EditNode)> {
        let edge = self.edge(key)?;
        Some((self.node(edge.from?)?, self.node(edge.to?)?))
    }

    // ------------------------------------------------------------------------------ nodes and connections

    /// `AssignNode` on a fresh `NodeEditor`: the node as the record has it, Inhibitory and SoftLatch nodes with no bias given 0.1,
    /// executed once so it has the value its bias makes.
    fn new_node(&mut self, record: &NodeRecord) -> NodeKey {
        let key = self.next_node;
        self.next_node += 1;
        let mut bias = record.base_activation;
        // `Mathf.Approximately(node.baseActivation, 0f)`.
        if matches!(record.node_type, NodeType::Inhibitory | NodeType::SoftLatch) && bias == 0.0 {
            bias = 0.1;
        }
        let index = record.index as usize;
        let desc = if index < BASE_NODES {
            base_name(index)
        } else {
            record.desc.clone().unwrap_or_default()
        };
        let mut node = EditNode {
            key,
            index,
            inov: record.innovation,
            kind: record.node_type,
            desc,
            bias,
            value: record.value,
            last_input: record.last_input,
            last_output: record.last_output,
            pos: [0.0, 0.0],
            activation: 0.0,
            valid: true,
            ingoing: Vec::new(),
            outgoing: Vec::new(),
        };
        // The nodes of a template start from rest.
        node.value = 0.0;
        node.last_input = 0.0;
        node.last_output = 0.0;
        node.activation = if node.kind == NodeType::Mult {
            1.0
        } else {
            0.0
        };
        node.execute();
        node.refresh_validity();
        self.nodes.push(node);
        key
    }

    /// `AssignNodes` / `AssignMissingNode` / `ReassignNode`: the ends of a connection, and the connections they know.
    fn attach(&mut self, edge: EdgeKey, from: Option<NodeKey>, to: Option<NodeKey>) {
        if let Some(from) = from {
            if let Some(node) = self.node_mut(from) {
                if !node.outgoing.contains(&edge) {
                    node.outgoing.push(edge);
                }
                node.refresh_validity();
            }
        }
        if let Some(to) = to {
            if let Some(node) = self.node_mut(to) {
                if !node.ingoing.contains(&edge) {
                    node.ingoing.push(edge);
                }
                node.refresh_validity();
            }
        }
        let missing = match (from, to) {
            (None, _) => Archetype::Input,
            (_, None) => Archetype::Output,
            _ => Archetype::Hidden,
        };
        if let Some(edge) = self.edge_mut(edge) {
            if from.is_some() {
                edge.from = from;
            }
            if to.is_some() {
                edge.to = to;
            }
            edge.missing = missing;
        }
    }

    /// `ConnectionEditor.Retire`: the connection goes, and the nodes forget it.
    fn retire_edge(&mut self, key: EdgeKey) {
        let Some(position) = self.edges.iter().position(|edge| edge.key == key) else {
            return;
        };
        let edge = self.edges.remove(position);
        for node in &mut self.nodes {
            if node.outgoing.contains(&edge.key) || node.ingoing.contains(&edge.key) {
                node.outgoing.retain(|other| *other != edge.key);
                node.ingoing.retain(|other| *other != edge.key);
                node.refresh_validity();
            }
        }
        if self.temp == Some(key) {
            self.temp = None;
        }
    }

    // ------------------------------------------------------------------------------ layout

    /// The sorter's view of what is on the canvas, from the node list and the synapse list.
    fn make_sorter(&self) -> Layout {
        let nodes: Vec<SortNode> = self
            .list
            .iter()
            .map(|node| {
                SortNode::new(
                    node.index as usize,
                    node.innovation,
                    node.node_type == NodeType::Sigmoid,
                    node.base_activation,
                )
            })
            .collect();
        let synapses: Vec<SortSynapse> = self
            .synapse_list
            .iter()
            .map(|synapse| {
                SortSynapse::new(
                    synapse.innovation,
                    synapse.node_in as usize,
                    synapse.node_out as usize,
                    synapse.weight,
                    synapse.enabled,
                )
            })
            .collect();
        make_groups(&nodes, &synapses)
    }

    /// `RefreshDerivedValues`.
    fn refresh_derived_values(&mut self) {
        let sorter = &self.sorter;
        let inputless: Vec<usize> = (0..sorter.groups.len())
            .filter(|&group| {
                sorter.groups[group].inputs.len() + sorter.groups[group].hidden.len() == 0
            })
            .collect();
        let n_g = inputless.len() as f32;
        let inputless_outputs: usize = inputless
            .iter()
            .map(|&group| sorter.groups[group].outputs.len())
            .sum();
        let used_inputs = self.used.iter().filter(|&&index| index < N_INPUTS).count();
        let used_outputs = self
            .used
            .iter()
            .filter(|&&index| index < BASE_NODES)
            .count()
            .saturating_sub(used_inputs)
            .saturating_sub(inputless_outputs);
        let rows: f32 = sorter
            .groups
            .iter()
            .map(|group| group.weight() + 1.0)
            .sum::<f32>()
            - 1.0;
        let mut height = TARGET_STEP_V * rows;
        // `nodeGroups.Max(g => g.maxDepth)`: the groups keep no paths, so it is 0 (the game throws on no groups at all).
        let mut max_depth = (0..sorter.groups.len())
            .map(|group| sorter.group_max_depth(group) as f32)
            .fold(0.0_f32, f32::max);
        if inputless.len() > 1 {
            let most = inputless
                .iter()
                .map(|&group| sorter.groups[group].outputs.len())
                .max()
                .unwrap_or(0) as f32;
            max_depth = max_depth.max(most / 2.0);
        }
        let width = height.max(STEP_W * (max_depth + 1.0));
        height = height.max(width / 2.0);
        self.metrics = Metrics {
            width,
            height,
            step_i: -height / used_inputs.saturating_sub(1).max(1) as f32,
            step_o: -height / used_outputs.saturating_sub(1).max(1) as f32,
            step_g: if rows != 0.0 { -height / rows } else { 0.0 },
            n_g,
        };
    }

    /// Where the brain starts inside the canvas (`(brainRT.sizeDelta - (width, height)) / 2`, `y` negated).
    fn origin(&self) -> [f32; 2] {
        [
            (self.content[0] - self.metrics.width) / 2.0,
            -(self.content[1] - self.metrics.height) / 2.0,
        ]
    }

    /// `SizePanelAndPlaceBrain`.
    fn size_panel_and_place_brain(&mut self) {
        self.refresh_derived_values();
        self.content = [
            (1.5 * self.screen[0] * 0.7).max(2.0 * self.metrics.width),
            (1.5 * self.screen[1] * 0.8).max(2.0 * self.metrics.height),
        ];
        if self.save_anchors {
            self.place_nodes_according_to_anchors();
        } else {
            self.sorter.sort(3, None, &mut self.rng);
            self.place_nodes_according_to_sort();
        }
    }

    /// `PlaceNodesAccordingToSort`.
    fn place_nodes_according_to_sort(&mut self) {
        let weights: Vec<f32> = self
            .sorter
            .groups
            .iter()
            .map(|group| group.weight())
            .collect();
        let sum: f32 = weights.iter().sum();
        let rows: f32 = weights.iter().map(|weight| weight + 1.0).sum();
        // (A single group of weight 0 divides by zero in the game and leaves its nodes nowhere; it has no gap here.)
        let gap = if rows > 1.0 {
            self.metrics.height / (rows - 1.0)
        } else {
            0.0
        };
        let sum = if sum < 0.1 { 1.0 } else { sum };
        let mut start = 0.0_f32;
        for group in self.sorter.groups_by_descending_weight() {
            let share = (self.metrics.height - gap * (self.sorter.groups.len() as f32 - 1.0))
                * weights[group]
                / sum;
            self.sorter.set_start_end(group, start, share);
            for node in self.sorter.groups[group].nodes.clone() {
                let step = if self.sorter.nodes[node].archetype() == Archetype::Input {
                    self.metrics.step_i
                } else {
                    self.metrics.step_o
                };
                let at = self.sorter.nodes[node].group_pos;
                self.sorter.nodes[node].panel_pos =
                    bibites_brainsort::Vec2::new(at.x, (start - share * at.y) / step);
            }
            start -= share + gap;
        }
        let origin = self.origin();
        let width = self.metrics.width;
        for group in self.sorter.groups_by_descending_weight() {
            for node in self.sorter.groups[group].nodes.clone() {
                let helper = &self.sorter.nodes[node];
                let step = if helper.archetype() == Archetype::Input {
                    self.metrics.step_i
                } else {
                    self.metrics.step_o
                };
                let (index, at) = (helper.index(), helper.panel_pos);
                if let Some(key) = self.key_of_index(index)
                    && let Some(node) = self.node_mut(key)
                {
                    node.pos = [origin[0] + at.x * width, origin[1] + at.y * step];
                }
            }
        }
    }

    /// `PlaceNodesAccordingToAnchors`: the nodes that have an anchor go by it, the others are put in line (inputs, outputs) or where
    /// the nodes they connect to are (hidden), and the outputs nothing reaches lie along the bottom.
    fn place_nodes_according_to_anchors(&mut self) {
        let metrics = self.metrics;
        let origin = self.origin();
        let inputless: Vec<usize> = (0..self.sorter.groups.len())
            .filter(|&group| {
                self.sorter.groups[group].inputs.len() + self.sorter.groups[group].hidden.len() == 0
            })
            .collect();
        let inputless_inovs: Vec<i64> = inputless
            .iter()
            .flat_map(|&group| self.sorter.groups[group].outputs.iter().copied())
            .map(|node| self.sorter.nodes[node].inov())
            .collect();
        let mut anchors = self.anchors.clone();
        let archetype_of = |this: &Self, key: NodeKey| this.node(key).map(EditNode::archetype);
        let input_anchors: Vec<(NodeKey, [f32; 2])> = anchors
            .iter()
            .filter(|(key, _)| archetype_of(self, **key) == Some(Archetype::Input))
            .map(|(key, anchor)| (*key, *anchor))
            .collect();
        let output_anchors: Vec<(NodeKey, [f32; 2])> = anchors
            .iter()
            .filter(|(key, _)| archetype_of(self, **key) == Some(Archetype::Output))
            .map(|(key, anchor)| (*key, *anchor))
            .collect();
        for node in &self.nodes {
            if inputless_inovs.contains(&node.inov) && node.archetype() == Archetype::Output {
                anchors.remove(&node.key);
            }
        }
        let highest = |list: &[(NodeKey, [f32; 2])]| {
            list.iter()
                .map(|(_, anchor)| anchor[1])
                .fold(f32::NEG_INFINITY, f32::max)
        };
        let (input_top, output_top) = (highest(&input_anchors), highest(&output_anchors));
        let hidden_top = if anchors.len() <= input_anchors.len() + output_anchors.len() {
            1.0
        } else {
            anchors
                .iter()
                .filter(|(key, _)| archetype_of(self, **key) == Some(Archetype::Hidden))
                .map(|(_, anchor)| anchor[1])
                .fold(f32::NEG_INFINITY, f32::max)
        };
        let group_share = 1.0 + metrics.n_g * metrics.step_g / metrics.height;
        let squeeze = |count: usize, top: f32| {
            if top.is_finite() && top != 0.0 {
                1.0_f32.min((count as f32 - 1.0) / top)
            } else {
                1.0
            }
        };
        let input_squeeze = squeeze(input_anchors.len(), input_top);
        let output_squeeze = squeeze(output_anchors.len(), output_top);
        let hidden_squeeze = group_share
            * if hidden_top.is_finite() && hidden_top != 0.0 {
                1.0_f32.min(1.0 / hidden_top)
            } else {
                1.0
            };
        let (mut placed_inputs, mut placed_outputs) = (0_u32, 0_u32);
        let mut placed: Vec<NodeKey> = Vec::new();
        for (key, anchor) in &anchors {
            let Some(archetype) = archetype_of(self, *key) else {
                continue;
            };
            let at = match archetype {
                Archetype::Input => {
                    placed_inputs += 1;
                    [
                        origin[0] + anchor[0] * metrics.width,
                        origin[1] + anchor[1] * input_squeeze * metrics.step_i,
                    ]
                }
                Archetype::Output => {
                    placed_outputs += 1;
                    [
                        origin[0] + anchor[0] * metrics.width,
                        origin[1] + anchor[1] * output_squeeze * metrics.step_o,
                    ]
                }
                Archetype::Hidden => [
                    origin[0] + anchor[0] * metrics.width,
                    origin[1] + anchor[1] * -metrics.height * hidden_squeeze,
                ],
            };
            if let Some(node) = self.node_mut(*key) {
                node.pos = at;
            }
            placed.push(*key);
        }
        // The rest, by index.
        let mut rest: Vec<(usize, NodeKey)> = self
            .nodes
            .iter()
            .filter(|node| !placed.contains(&node.key))
            .map(|node| (node.index, node.key))
            .collect();
        rest.sort_unstable();
        for (_, key) in rest {
            let Some(node) = self.node(key) else {
                continue;
            };
            let at = match node.archetype() {
                Archetype::Input => {
                    placed_inputs += 1;
                    [
                        origin[0],
                        origin[1] + (placed_inputs - 1) as f32 * metrics.step_i,
                    ]
                }
                Archetype::Output => {
                    placed_outputs += 1;
                    [
                        origin[0] + metrics.width,
                        origin[1] + (placed_outputs - 1) as f32 * metrics.step_o,
                    ]
                }
                Archetype::Hidden => {
                    // The mean of its neighbours' places by the connections' strength; the game adds the brain's origin to a
                    // mean of places that already have it.
                    let (mut sum, mut weights) = ([0.0_f32; 2], 1e-6_f32);
                    for edge_key in node.ingoing.iter().chain(node.outgoing.iter()) {
                        let Some(edge) = self.edge(*edge_key) else {
                            continue;
                        };
                        let other = if edge.from == Some(key) {
                            edge.to
                        } else {
                            edge.from
                        };
                        let Some(other) = other.and_then(|other| self.node(other)) else {
                            continue;
                        };
                        let weight = edge.weight.abs();
                        sum[0] += other.pos[0] * weight;
                        sum[1] += other.pos[1] * weight;
                        weights += weight;
                    }
                    [origin[0] + sum[0] / weights, origin[1] + sum[1] / weights]
                }
            };
            if let Some(node) = self.node_mut(key) {
                node.pos = at;
            }
        }
        if metrics.n_g > 0.0 {
            let mut y = -metrics.height * group_share;
            for group in inputless {
                y += metrics.step_g;
                let nodes = self.sorter.groups[group].nodes.clone();
                let step = 1.0 / (nodes.len() as f32 + 1.0);
                let mut along = 0.0;
                for node in nodes {
                    along += step;
                    let index = self.sorter.nodes[node].index();
                    if let Some(key) = self.key_of_index(index)
                        && let Some(node) = self.node_mut(key)
                    {
                        node.pos = [origin[0] + along * metrics.width, origin[1] + y];
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------------------ the lists

    /// `RebuildLists`: the hidden nodes get their places in the list (and the connections the indices that go with them), the nodes
    /// and connections their innovation numbers, and the simulation stops.
    pub fn rebuild_lists(&mut self) {
        self.play(false);
        self.list.truncate(BASE_NODES);
        for index in 0..BASE_NODES {
            if let Some(key) = self.key_of_index(index) {
                if let Some(node) = self.node_mut(key) {
                    node.inov = index as i64 + 1;
                    node.last_input = 0.0;
                    node.last_output = 0.0;
                    node.value = 0.0;
                    let record = node.record();
                    self.list[index] = record;
                }
            }
        }
        let hidden: Vec<NodeKey> = self
            .nodes
            .iter()
            .filter(|node| node.archetype() == Archetype::Hidden)
            .map(|node| node.key)
            .collect();
        for (next, key) in (BASE_NODES..).zip(hidden) {
            if let Some(node) = self.node_mut(key) {
                node.index = next;
                node.inov = next as i64 + 1;
                node.last_input = 0.0;
                node.last_output = 0.0;
                node.value = 0.0;
                let record = node.record();
                self.list.push(record);
            }
        }
        self.synapse_list.clear();
        let mut inov = 1_i64;
        let ends: Vec<(EdgeKey, Option<usize>, Option<usize>)> = self
            .edges
            .iter()
            .map(|edge| {
                (
                    edge.key,
                    edge.from
                        .and_then(|key| self.node(key))
                        .map(|node| node.index),
                    edge.to
                        .and_then(|key| self.node(key))
                        .map(|node| node.index),
                )
            })
            .collect();
        for (key, from, to) in ends {
            if let Some(edge) = self.edge_mut(key) {
                edge.inov = inov;
                inov += 1;
                // A connection still waiting for an end is not saved.
                if let (Some(from), Some(to)) = (from, to) {
                    let record = SynapseRecord {
                        innovation: edge.inov,
                        node_in: from as u32,
                        node_out: to as u32,
                        weight: edge.weight,
                        enabled: edge.enabled,
                    };
                    self.synapse_list.push(record);
                }
            }
        }
    }

    /// `ReSortBrain`: the nodes are put where the sorter wants them, and the simulation starts from rest.
    pub fn resort(&mut self) {
        self.set_save_anchors(false);
        self.rebuild_lists();
        self.sorter = self.make_sorter();
        self.size_panel_and_place_brain();
        for node in &mut self.nodes {
            node.last_output = 0.0;
            node.last_input = 0.0;
            node.value = 0.0;
            node.activation = if node.kind == NodeType::Mult {
                1.0
            } else {
                0.0
            };
            node.execute();
        }
    }

    /// `SetNodePositionSaveMode`.
    pub fn set_save_anchors(&mut self, on: bool) {
        self.save_anchors = on;
    }

    /// `SaveAnchors`: where the nodes are, as fractions of the brain (`anchors` by innovation).
    fn compute_anchors(&mut self) -> BTreeMap<i64, (f32, f32)> {
        self.sorter = self.make_sorter();
        self.used = self.sorter.used_indices_of_groups();
        self.refresh_derived_values();
        let metrics = self.metrics;
        // Unconnected outputs are not placed by anyone.
        let counted: Vec<&EditNode> = self
            .nodes
            .iter()
            .filter(|node| node.archetype() != Archetype::Output || !node.ingoing.is_empty())
            .collect();
        let mut anchors = BTreeMap::new();
        if counted.is_empty() {
            return anchors;
        }
        let min_x = counted
            .iter()
            .map(|node| node.pos[0])
            .fold(f32::INFINITY, f32::min);
        let max_x = counted
            .iter()
            .map(|node| node.pos[0])
            .fold(f32::NEG_INFINITY, f32::max);
        // `yMin` is the highest of them (the largest `y`, they are negative downwards).
        let top = counted
            .iter()
            .map(|node| node.pos[1])
            .fold(f32::NEG_INFINITY, f32::max);
        let width = max_x - min_x;
        for node in counted {
            let step = match node.archetype() {
                Archetype::Input => metrics.step_i,
                Archetype::Output => metrics.step_o,
                Archetype::Hidden => -(metrics.height + metrics.n_g * metrics.step_g),
            };
            let at = (
                if width != 0.0 {
                    (node.pos[0] - min_x) / width
                } else {
                    0.0
                },
                (node.pos[1] - top) / step,
            );
            anchors.insert(node.inov, at);
        }
        anchors
    }

    /// `PrepareForTemplateSave`: the lists as the template is saved with, and the anchors when they are kept.
    #[must_use]
    pub fn prepare_for_template_save(&mut self) -> TemplateParts {
        self.rebuild_lists();
        let anchors = if self.save_anchors {
            self.compute_anchors()
        } else {
            BTreeMap::new()
        };
        (self.list.clone(), self.synapse_list.clone(), anchors)
    }

    // ------------------------------------------------------------------------------ selection

    fn push_escape(&mut self, entry: Escapable) {
        self.escapes.retain(|other| *other != entry);
        self.escapes.push(entry);
    }

    fn remove_escape(&mut self, entry: Escapable) {
        self.escapes.retain(|other| *other != entry);
    }

    /// `Deselect`.
    pub fn deselect(&mut self) {
        self.selected = None;
        self.remove_escape(Escapable::Select);
    }

    /// `SelectElement(NodeEditor)`: while a connection is being made the node is its other end (when it can be); with Shift held a
    /// connection starts at the node (at an output, it ends there); else the node is selected.
    pub fn click_node(&mut self, key: NodeKey, shift: bool) {
        let Some(node) = self.node(key) else {
            return;
        };
        let archetype = node.archetype();
        if self.connecting {
            let Some(temp) = self.temp else {
                return;
            };
            let missing = self.edge(temp).map(|edge| edge.missing);
            if archetype == Archetype::Hidden || missing == Some(archetype) {
                let (from, to) = self
                    .edge(temp)
                    .map(|edge| (edge.from, edge.to))
                    .unwrap_or((None, None));
                if from.is_none() {
                    self.attach(temp, Some(key), None);
                } else if to.is_none() {
                    self.attach(temp, None, Some(key));
                }
                self.temp = None;
                if self.add_mode {
                    self.close_add_node_panel();
                }
                // `StopConnecting` with no temporary connection left only ends the mode.
                self.connecting = false;
                self.remove_escape(Escapable::CreateConnection);
                self.any_change = true;
            }
            return;
        }
        if shift {
            if archetype == Archetype::Output {
                self.new_connection(None, Some(key));
            } else {
                self.new_connection(Some(key), None);
            }
            return;
        }
        self.select_node(key);
    }

    /// `NodeEditPanel.SelectNode`: the node is the selection, and the panel's bias is the node's.
    fn select_node(&mut self, key: NodeKey) {
        let bias = self.node(key).map_or(0.0, |node| node.bias);
        self.selected = Some(Selected::Node(key));
        self.panel_bias = bias;
        self.push_escape(Escapable::Select);
    }

    /// `SelectElement(ConnectionEditor)`: nothing happens while a connection is being made.
    pub fn click_edge(&mut self, key: EdgeKey) {
        if self.connecting || self.edge(key).is_none() {
            return;
        }
        self.selected = Some(Selected::Edge(key));
        self.push_escape(Escapable::Select);
    }

    /// `CreateNewConnectionFromToNode`: a connection from `from` (or to `to`) that follows the cursor until a node is clicked.
    pub fn new_connection(&mut self, from: Option<NodeKey>, to: Option<NodeKey>) {
        let key = self.next_edge;
        self.next_edge += 1;
        self.edges.push(EditEdge {
            key,
            inov: 0,
            weight: 1.0,
            enabled: true,
            from: None,
            to: None,
            missing: Archetype::Hidden,
        });
        self.attach(key, from, to);
        self.temp = Some(key);
        self.deselect();
        self.push_escape(Escapable::CreateConnection);
        self.connecting = true;
    }

    /// `StopConnecting`: the connection that was being made is dropped.
    pub fn stop_connecting(&mut self) {
        if !self.connecting {
            return;
        }
        self.remove_escape(Escapable::CreateConnection);
        if let Some(temp) = self.temp.take() {
            self.retire_edge(temp);
            self.deselect();
            self.any_change = true;
        }
        self.connecting = false;
    }

    /// `RequestAddNode(unavailableType)`: the add-node panel opens for a node at `pos`. While a connection waits for its start it
    /// does not offer outputs (the game passes the other archetype), while one waits for its end no inputs.
    pub fn request_add_node(&mut self, pos: [f32; 2]) {
        self.add_pos = pos;
        self.add_hides = match self
            .temp
            .and_then(|temp| self.edge(temp))
            .map(|edge| edge.missing)
        {
            Some(Archetype::Input) if self.connecting => Archetype::Output,
            Some(Archetype::Output) if self.connecting => Archetype::Input,
            _ => Archetype::Hidden,
        };
        self.add_mode = true;
        self.push_escape(Escapable::AddNode);
    }

    /// `CloseAddNodePanel`.
    pub fn close_add_node_panel(&mut self) {
        self.remove_escape(Escapable::AddNode);
        self.add_mode = false;
    }

    /// What Escape does here (`UINavigationManager` pops the newest entry); `false` when there is nothing of this editor to undo.
    pub fn escape(&mut self) -> bool {
        match self.escapes.pop() {
            Some(Escapable::Select) => {
                self.deselect();
                true
            }
            Some(Escapable::CreateConnection) => {
                self.escapes.push(Escapable::CreateConnection);
                self.stop_connecting();
                true
            }
            Some(Escapable::AddNode) => {
                self.escapes.push(Escapable::AddNode);
                self.close_add_node_panel();
                true
            }
            None => false,
        }
    }

    // ------------------------------------------------------------------------------ edits

    /// `NodeEditor.OnDrag`: the node goes where the cursor is, stuck to the brain's left and right edge when it is near one.
    pub fn drag_node(&mut self, key: NodeKey, cursor: [f32; 2]) {
        let origin = self.origin();
        let width = self.metrics.width;
        let mut x = cursor[0] - origin[0];
        if x > -STICKY && x < STICKY {
            x = 0.0;
        } else if x > width - STICKY && x < width + STICKY {
            x = width;
        }
        if let Some(node) = self.node_mut(key) {
            node.pos = [x + origin[0], cursor[1]];
        }
        self.deselect();
    }

    /// `NodeEditor.EditLabel`: only a hidden node has one to change.
    pub fn edit_label(&mut self, key: NodeKey, text: &str) {
        if let Some(node) = self.node_mut(key)
            && node.archetype() == Archetype::Hidden
        {
            node.desc = text.to_owned();
        }
    }

    /// `NodeEditor.EditBias`: nodes whose function has a bias; an Inhibitory node's stays in 0.0001 ..= 10, a SoftLatch's in
    /// 0.0001 ..= 1000.
    pub fn edit_bias(&mut self, key: NodeKey, value: f32) {
        if let Some(node) = self.node_mut(key)
            && node.kind.has_bias()
        {
            let value = match node.kind {
                NodeType::Inhibitory => value.clamp(0.0001, 10.0),
                NodeType::SoftLatch => value.clamp(0.0001, 1000.0),
                _ => value,
            };
            node.bias = value;
            node.execute();
            // The slider and a typed value keep the panel's own bias in range already (`canGoOutOfBounds` is off for those functions).
            self.panel_bias = value;
        }
    }

    /// `NodeEditor.EditAct`: what an input node says to the simulator.
    pub fn edit_act(&mut self, key: NodeKey, value: f32) {
        if let Some(node) = self.node_mut(key)
            && node.kind == NodeType::Input
        {
            node.last_output = value;
            node.value = value;
        }
    }

    /// `NodeEditor.EditFunction`: a hidden node takes another function (and that function's own bias), then `CheckTypeForBiasSlider` gives
    /// the bias slider the function's range: for an Inhibitory or a SoftLatch node the slider's value is forced into it, which edits the
    /// bias again with the panel's own value (that of the node as it was selected or last moved, not the new default) when that is out of it.
    pub fn edit_function(&mut self, key: NodeKey, kind: NodeType) {
        let changed = match self.node_mut(key) {
            Some(node) if node.archetype() == Archetype::Hidden => {
                node.kind = kind;
                node.bias = default_bias_of(kind);
                node.activation = if kind == NodeType::Mult { 1.0 } else { 0.0 };
                node.execute();
                true
            }
            _ => false,
        };
        if !changed {
            return;
        }
        let range = match kind {
            NodeType::Inhibitory => Some((0.0001, 10.0)),
            NodeType::SoftLatch => Some((0.0001, 1000.0)),
            _ => None,
        };
        if let Some((low, high)) = range {
            let forced = self.panel_bias.clamp(low, high);
            if forced != self.panel_bias {
                self.edit_bias(key, forced);
            }
        }
    }

    /// `ConnectionEditor.EditWeight`.
    pub fn edit_weight(&mut self, key: EdgeKey, value: f32) {
        if let Some(edge) = self.edge_mut(key) {
            edge.weight = value;
        }
    }

    /// `ConnectionEditor.EditEnabled`.
    pub fn edit_enabled(&mut self, key: EdgeKey, enabled: bool) {
        if let Some(edge) = self.edge_mut(key) {
            edge.enabled = enabled;
        }
    }

    /// `SplitConnectionInNewNode`: a hidden node (Linear, no bias) goes in the middle of the connection, which keeps its weight and
    /// ends at the new node (`remain_ingoing`) or starts from it; the other half is a new connection of weight 1.
    pub fn split_connection(&mut self, key: EdgeKey, remain_ingoing: bool) {
        let Some((from, to)) = self.edge(key).map(|edge| (edge.from, edge.to)) else {
            return;
        };
        let (Some(from), Some(to)) = (from, to) else {
            return;
        };
        let middle = match (self.node(from), self.node(to)) {
            (Some(a), Some(b)) => [(a.pos[0] + b.pos[0]) / 2.0, (a.pos[1] + b.pos[1]) / 2.0],
            _ => return,
        };
        let new_key = self.add_hidden(NodeType::Linear, 0.0, middle);
        let half = self.next_edge;
        self.next_edge += 1;
        self.edges.push(EditEdge {
            key: half,
            inov: 0,
            weight: 1.0,
            enabled: true,
            from: None,
            to: None,
            missing: Archetype::Hidden,
        });
        if remain_ingoing {
            self.attach(half, Some(new_key), Some(to));
            self.reassign(key, new_key, false);
        } else {
            self.attach(half, Some(from), Some(new_key));
            self.reassign(key, new_key, true);
        }
        self.select_node(new_key);
        self.any_change = true;
    }

    /// `ConnectionEditor.ReassignNode`.
    fn reassign(&mut self, edge: EdgeKey, node: NodeKey, input: bool) {
        let Some(old) = self
            .edge(edge)
            .map(|edge| if input { edge.from } else { edge.to })
        else {
            return;
        };
        if let Some(old) = old.and_then(|old| self.node_mut(old)) {
            if input {
                old.outgoing.retain(|other| *other != edge);
            } else {
                old.ingoing.retain(|other| *other != edge);
            }
            old.refresh_validity();
        }
        if input {
            if let Some(edge) = self.edge_mut(edge) {
                edge.from = None;
            }
            self.attach(edge, Some(node), None);
        } else {
            if let Some(edge) = self.edge_mut(edge) {
                edge.to = None;
            }
            self.attach(edge, None, Some(node));
        }
        // Both ends are there again.
        if let Some(edge) = self.edge_mut(edge) {
            edge.missing = Archetype::Hidden;
        }
    }

    /// A new hidden node at `pos` (`Index = NInputs + NOutputs + hiddenNodes.activeCount`, `Desc = "Hidden Node"`).
    fn add_hidden(&mut self, kind: NodeType, bias: f32, pos: [f32; 2]) -> NodeKey {
        let record = NodeRecord {
            node_type: kind,
            index: (BASE_NODES + self.hidden_count()) as u32,
            innovation: 0,
            desc: Some("Hidden Node".to_owned()),
            base_activation: bias,
            value: 0.0,
            last_input: 0.0,
            last_output: 0.0,
        };
        let key = self.new_node(&record);
        if let Some(node) = self.node_mut(key) {
            node.pos = pos;
        }
        key
    }

    /// [`Self::add_node_holding`] without Shift (the tests' way to put a node on the canvas).
    #[cfg(test)]
    pub fn add_node(&mut self, index: usize, archetype: Archetype) {
        self.add_node_holding(index, archetype, false);
    }

    /// `AddNode`: a base node (`index`: its place in the node list) or a hidden node of function `index` goes on the canvas at the
    /// place the panel was asked for, and is then selected as a click would select it (`SelectElement(node, null)`): while a connection
    /// waits for its end the new node ends it (when it can), and with Shift held it starts a connection instead.
    pub fn add_node_holding(&mut self, index: usize, archetype: Archetype, shift: bool) {
        let pos = self.add_pos;
        let key = if archetype == Archetype::Hidden {
            let kind = NodeType::from_u8(index as u8).unwrap_or(NodeType::Linear);
            self.add_hidden(kind, default_bias_of(kind), pos)
        } else if let Some(record) = self.list.get(index).cloned() {
            // `baseNodes.GetItemWithKey(index)`: the node that is there (`AssignNode(nodes[index])` is the same object: it is only
            // moved), or a new one made from the list.
            let key = match self.key_of_index(index) {
                Some(key) => key,
                None => self.new_node(&record),
            };
            if let Some(offered) = self.offered.get_mut(index) {
                *offered = false;
            }
            if let Some(node) = self.node_mut(key) {
                node.pos = pos;
            }
            key
        } else {
            return;
        };
        self.close_add_node_panel();
        self.click_node(key, shift);
        self.any_change = true;
    }

    /// `DeleteConnection`.
    pub fn delete_connection(&mut self, key: EdgeKey) {
        self.retire_edge(key);
        self.deselect();
        self.any_change = true;
    }

    /// `DeleteNode`: the node and its connections go; an output goes back to its function's own bias, and stays when it is a
    /// sigmoid (the layout would put it back: an active output without a connection is shown).
    pub fn delete_node(&mut self, key: NodeKey) {
        let Some(node) = self.node(key) else {
            return;
        };
        let (archetype, kind, index) = (node.archetype(), node.kind, node.index);
        let connections: Vec<EdgeKey> = node
            .ingoing
            .iter()
            .chain(node.outgoing.iter())
            .copied()
            .collect();
        for edge in connections {
            self.retire_edge(edge);
        }
        self.any_change = true;
        // `SetIndexAvailability(..., true)` comes first: an output that stays is offered again.
        if index < BASE_NODES {
            self.offered[index] = true;
        }
        if archetype == Archetype::Output {
            self.edit_bias(key, default_bias_of(kind));
            // The editor's node is the list's node in the game, so its reset is the list's too (the next time it is added).
            let record = self.node(key).map(EditNode::record);
            if let (Some(record), Some(slot)) = (record, self.list.get_mut(index)) {
                *slot = record;
            }
            if kind == NodeType::Sigmoid {
                self.deselect();
                return;
            }
        }
        self.nodes.retain(|node| node.key != key);
        self.anchors.remove(&key);
        self.deselect();
    }

    // ------------------------------------------------------------------------------ the simulator

    /// `PlayBrain`.
    pub fn play(&mut self, on: bool) {
        self.progress = 0.0;
        self.playing = on;
    }

    /// `StepBrain`: every connection hands what its start said last time, times its weight, to its end, then every node executes.
    pub fn step(&mut self) {
        let mut adds: Vec<(NodeKey, f32, bool)> = Vec::new();
        for edge in &self.edges {
            let (Some(from), Some(to)) = (edge.from, edge.to) else {
                continue;
            };
            if !edge.enabled {
                continue;
            }
            let (Some(a), Some(b)) = (self.node(from), self.node(to)) else {
                continue;
            };
            adds.push((to, a.last_output * edge.weight, b.kind == NodeType::Mult));
        }
        for (to, amount, multiplies) in adds {
            if let Some(node) = self.node_mut(to) {
                if multiplies {
                    node.activation *= amount;
                } else {
                    node.activation += amount;
                }
            }
        }
        // Each node executes on what it collected alone, so the order does not matter.
        for node in &mut self.nodes {
            node.execute();
        }
    }

    /// `OneBrainStep`: the simulation stops and takes one step.
    pub fn one_step(&mut self) {
        self.play(false);
        self.step();
    }

    /// `UpdatePanel`: key `1` (when it goes down: one step; held: steps by the period) and the play button step the simulator.
    pub fn tick(&mut self, dt: f32, key_pressed: bool, key_held: bool) {
        if key_pressed {
            self.one_step();
        } else if key_held || self.playing {
            self.progress += dt;
            if self.progress >= BRAIN_PERIOD {
                self.progress -= BRAIN_PERIOD;
                self.step();
            }
        }
    }
}

/// `NEATBrain.DefaultBaseActivationOfType`.
#[must_use]
pub fn default_bias_of(kind: NodeType) -> f32 {
    match kind {
        NodeType::Mult | NodeType::Inhibitory => 1.0,
        NodeType::SoftLatch => 5.0,
        _ => 0.0,
    }
}

/// The name of a base node (`Enum.GetName(typeof(Inputs / Outputs), ...)`).
#[must_use]
pub fn base_name(index: usize) -> String {
    use bibites_core::brain::{Input, Output};
    if index < N_INPUTS {
        Input::from_index(index).map_or_else(String::new, |input| input.name().to_owned())
    } else {
        Output::from_index(index - N_INPUTS)
            .map_or_else(String::new, |output| output.name().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bibites_core::brain::Brain;

    fn editing(brain: &Brain) -> BrainEdit {
        BrainEdit::fill(
            brain.to_node_records(),
            &brain.to_synapse_records(),
            &BTreeMap::new(),
        )
    }

    fn starter() -> BrainEdit {
        editing(&Brain::starter())
    }

    fn blank() -> BrainEdit {
        editing(&crate::template_editor::blank_brain())
    }

    fn output_node(edit: &BrainEdit) -> NodeKey {
        edit.nodes
            .iter()
            .find(|node| node.archetype() == Archetype::Output)
            .expect("an output on the canvas")
            .key
    }

    /// An input node of that index put on the canvas, selected, as the add-node panel does.
    fn add_input(edit: &mut BrainEdit, index: usize) -> NodeKey {
        edit.request_add_node([300.0, -300.0]);
        edit.add_node(index, Archetype::Input);
        match edit.selected {
            Some(Selected::Node(key)) => key,
            other => panic!("the new node is selected, not {other:?}"),
        }
    }

    #[test]
    fn a_templates_brain_puts_its_used_nodes_on_the_canvas() {
        let edit = starter();
        // The three connections of the starter brain and what they join, plus the outputs that are active without one.
        assert_eq!(edit.edges.len(), 3);
        for index in [3, 12, 13, 33, 34, 39] {
            assert!(
                edit.nodes.iter().any(|node| node.index == index),
                "node {index} is used"
            );
        }
        assert_eq!(edit.nodes.len(), edit.used.len());
        assert!(
            edit.nodes.len() < BASE_NODES,
            "the unused inputs are not there"
        );
        assert!(edit.is_valid());
        assert!(!edit.save_anchors && !edit.any_change);
        // Every node has a place on the canvas, inside it.
        for node in &edit.nodes {
            assert!(
                node.pos[0].is_finite() && node.pos[1].is_finite(),
                "{node:?}"
            );
            assert!(
                node.pos[0] >= 0.0 && node.pos[0] <= edit.content[0],
                "{node:?}"
            );
            assert!(
                node.pos[1] <= 0.0 && node.pos[1] >= -edit.content[1],
                "{node:?}"
            );
        }
        // The inputs are on the left of the outputs.
        let leftmost_output = edit
            .nodes
            .iter()
            .filter(|node| node.archetype() == Archetype::Output)
            .map(|node| node.pos[0])
            .fold(f32::INFINITY, f32::min);
        let rightmost_input = edit
            .nodes
            .iter()
            .filter(|node| node.archetype() == Archetype::Input)
            .map(|node| node.pos[0])
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(rightmost_input < leftmost_output);
        // What is not on the canvas is what the add-node panel offers.
        assert_eq!(edit.available(Archetype::Input).len(), 33 - 3);
        assert!(
            edit.available(Archetype::Output)
                .iter()
                .all(|&index| (33..48).contains(&index))
        );
        // Executed once, a node has the value its bias makes: the sigmoid of 0.4535929 for Accelerate (a TanH output: tanh).
        let accelerate = edit.nodes.iter().find(|node| node.index == 33).unwrap();
        assert!(
            (accelerate.value - 0.4535929_f32.tanh()).abs() < 1e-5,
            "{}",
            accelerate.value
        );
    }

    #[test]
    fn a_new_templates_brain_shows_the_outputs_that_are_active_and_offers_the_rest() {
        let edit = blank();
        assert!(edit.edges.is_empty());
        assert!(
            edit.nodes
                .iter()
                .all(|node| node.archetype() == Archetype::Output)
        );
        assert!(
            !edit.nodes.is_empty(),
            "the sigmoid outputs are active without a connection"
        );
        assert!(
            edit.nodes
                .iter()
                .all(|node| node.kind == NodeType::Sigmoid || node.bias != 0.0)
        );
        assert_eq!(edit.available(Archetype::Input).len(), 33);
        assert_eq!(
            edit.available(Archetype::Output).len() + edit.nodes.len(),
            15,
            "every output is on the canvas or on offer"
        );
        assert!(edit.is_valid(), "outputs need nothing");
    }

    #[test]
    fn an_input_is_added_connected_and_simulated() {
        let mut edit = blank();
        let input = add_input(&mut edit, 0);
        let other = add_input(&mut edit, 1);
        assert!(edit.any_change);
        assert_eq!(edit.node(input).unwrap().pos, [300.0, -300.0]);
        assert!(!edit.add_mode, "the panel closed");
        // An input with no connection out is not valid, and the Save buttons wait for it.
        assert!(!edit.node(input).unwrap().valid && !edit.is_valid());
        // Shift-click starts a connection from it, which needs an end.
        edit.click_node(input, true);
        assert!(edit.connecting);
        let temp = edit.temp.expect("the connection being made");
        assert_eq!(edit.edge(temp).unwrap().missing, Archetype::Output);
        assert!(
            edit.node(input).unwrap().valid,
            "it has a connection out now"
        );
        // Another input is not an end for it; an output is.
        edit.click_node(other, false);
        assert!(edit.connecting, "an input cannot be the end");
        let output = output_node(&edit);
        edit.click_node(output, false);
        assert!(!edit.connecting && edit.temp.is_none());
        assert_eq!(edit.edges.len(), 1);
        let edge = &edit.edges[0];
        assert_eq!((edge.from, edge.to), (Some(input), Some(output)));
        assert_eq!((edge.weight, edge.enabled), (1.0, true));
        // The simulator: the input says 1, one step later the output has the function of 1 + its bias.
        edit.edit_act(input, 1.0);
        edit.step();
        let out = edit.node(output).unwrap();
        let (expected, _) = execute_neuron(out.kind, out.bias, 1.0, 0.0, 0.0, BRAIN_PERIOD);
        assert!(
            (out.value - expected).abs() < 1e-5,
            "{} {}",
            out.value,
            expected
        );
        // A disabled connection carries nothing.
        let key = edit.edges[0].key;
        edit.edit_enabled(key, false);
        edit.step();
        let quiet = edit.node(output).unwrap();
        let (rest, _) = execute_neuron(quiet.kind, quiet.bias, 0.0, expected, 0.0, BRAIN_PERIOD);
        assert!(
            (quiet.value - rest).abs() < 1e-4,
            "{} {}",
            quiet.value,
            rest
        );
    }

    #[test]
    fn a_connection_is_split_by_a_new_hidden_node() {
        let mut edit = starter();
        let before = edit.edges.len();
        let edge = edit.edges[0].clone();
        edit.split_connection(edge.key, true);
        assert_eq!(edit.edges.len(), before + 1);
        let hidden = edit
            .nodes
            .iter()
            .find(|node| node.archetype() == Archetype::Hidden)
            .expect("the new node")
            .clone();
        assert_eq!(
            (hidden.kind, hidden.bias, hidden.desc.as_str()),
            (NodeType::Linear, 0.0, "Hidden Node")
        );
        assert_eq!(edit.selected, Some(Selected::Node(hidden.key)));
        assert!(hidden.valid, "one in, one out");
        // "Add node after": the connection keeps its weight and now ends at the new node; the other half is new, of weight 1.
        let kept = edit.edge(edge.key).unwrap();
        assert_eq!(
            (kept.from, kept.to, kept.weight),
            (edge.from, Some(hidden.key), edge.weight)
        );
        let half = edit.edges.last().unwrap();
        assert_eq!(
            (half.from, half.to, half.weight),
            (Some(hidden.key), edge.to, 1.0)
        );
        // "Add node before" does it the other way round.
        let second = edit.edges[1].clone();
        edit.split_connection(second.key, false);
        let kept = edit.edge(second.key).unwrap();
        assert_eq!(kept.to, second.to);
        assert!(kept.from != second.from);
        assert_eq!(edit.hidden_count(), 2);
        assert!(edit.is_valid());
    }

    #[test]
    fn deleting_a_node_takes_its_connections_and_a_sigmoid_output_stays() {
        let mut edit = starter();
        let edge = edit.edges[0].key;
        edit.split_connection(edge, true);
        let Some(Selected::Node(hidden)) = edit.selected else {
            panic!("selected");
        };
        edit.delete_node(hidden);
        assert_eq!(edit.hidden_count(), 0);
        assert_eq!(edit.edges.len(), 2, "both halves went with it");
        assert!(edit.selected.is_none());
        // The input and the output of the connection that is gone are still there; the input is no longer valid.
        assert!(!edit.is_valid());
        // An output with a function whose bias is zero leaves the canvas; a sigmoid one stays (it would come back).
        let mut blank = blank();
        let sigmoid = output_node(&blank);
        let count = blank.nodes.len();
        blank.delete_node(sigmoid);
        assert_eq!(blank.nodes.len(), count);
        let mut starter = starter();
        let rotate = starter
            .nodes
            .iter()
            .find(|node| node.index == 34)
            .unwrap()
            .key;
        let rotating = starter
            .nodes
            .iter()
            .find(|node| node.index == 34)
            .unwrap()
            .kind;
        let count = starter.nodes.len();
        starter.delete_node(rotate);
        let expected = if rotating == NodeType::Sigmoid {
            count
        } else {
            count - 1
        };
        assert_eq!(starter.nodes.len(), expected);
        assert_eq!(starter.edges.len(), 2, "its connection went");
    }

    #[test]
    fn the_template_is_saved_with_compact_hidden_indices_and_fresh_innovations() {
        let mut edit = blank();
        let input = add_input(&mut edit, 3);
        let output = output_node(&edit);
        edit.new_connection(Some(input), None);
        edit.click_node(output, false);
        let edge = edit.edges[0].key;
        edit.split_connection(edge, true);
        edit.split_connection(edge, true);
        assert_eq!(edit.hidden_count(), 2);
        let (nodes, synapses, anchors) = edit.prepare_for_template_save();
        assert!(anchors.is_empty(), "positions are not kept unless asked");
        assert_eq!(nodes.len(), BASE_NODES + 2);
        for (at, node) in nodes.iter().enumerate() {
            assert_eq!(node.index as usize, at);
            assert_eq!(node.innovation, at as i64 + 1);
        }
        assert_eq!(nodes[BASE_NODES].desc.as_deref(), Some("Hidden Node"));
        assert_eq!(nodes[3].desc, None, "a base node's name is its index's");
        assert_eq!(synapses.len(), 3);
        for (at, synapse) in synapses.iter().enumerate() {
            assert_eq!(synapse.innovation, at as i64 + 1);
            assert!(
                (synapse.node_in as usize) < nodes.len()
                    && (synapse.node_out as usize) < nodes.len()
            );
        }
        // A chain input -> hidden -> hidden -> output.
        let path = synapses
            .iter()
            .map(|synapse| (synapse.node_in, synapse.node_out))
            .collect::<std::collections::BTreeSet<_>>();
        assert!(
            path.iter().any(|&(from, _)| from == 3),
            "the input starts a connection"
        );
        // The core takes it as a brain.
        let brain = Brain::from_records(&nodes, &synapses).expect("a brain");
        assert_eq!(brain.hidden_count(), 2);
        assert_eq!(brain.synapse_count(), 3);
    }

    #[test]
    fn a_connection_that_still_waits_for_its_end_is_not_saved() {
        let mut edit = blank();
        let input = add_input(&mut edit, 5);
        edit.click_node(input, true);
        assert!(edit.connecting);
        let (_, synapses, _) = edit.prepare_for_template_save();
        assert!(synapses.is_empty());
        // Escape drops it.
        assert!(edit.escape());
        assert!(!edit.connecting && edit.edges.is_empty());
    }

    #[test]
    fn escape_undoes_the_newest_thing_first() {
        let mut edit = starter();
        let node = edit.nodes[0].key;
        edit.click_node(node, false);
        assert_eq!(edit.selected, Some(Selected::Node(node)));
        edit.request_add_node([10.0, -10.0]);
        assert!(edit.add_mode);
        assert_eq!(edit.escapes, [Escapable::Select, Escapable::AddNode]);
        assert!(edit.escape());
        assert!(!edit.add_mode && edit.selected.is_some());
        assert!(edit.escape());
        assert!(edit.selected.is_none());
        assert!(
            !edit.escape(),
            "nothing of the brain editor is left to undo"
        );
    }

    #[test]
    fn a_dragged_node_sticks_to_the_edges_and_the_selection_goes() {
        let mut edit = starter();
        let node = edit.nodes[0].key;
        edit.click_node(node, false);
        let origin = edit.origin();
        edit.drag_node(node, [origin[0] + 6.0, -123.0]);
        let moved = edit.node(node).unwrap();
        assert_eq!(
            moved.pos,
            [origin[0], -123.0],
            "near the left edge it sticks"
        );
        assert!(edit.selected.is_none());
        edit.drag_node(node, [origin[0] + edit.metrics.width - 9.0, -50.0]);
        assert_eq!(
            edit.node(node).unwrap().pos[0],
            origin[0] + edit.metrics.width
        );
        edit.drag_node(node, [origin[0] + 40.0, -50.0]);
        assert_eq!(edit.node(node).unwrap().pos[0], origin[0] + 40.0);
    }

    #[test]
    fn resorting_puts_the_nodes_back_and_stops_keeping_positions() {
        let mut edit = starter();
        let node = edit.nodes[0].key;
        let before = edit.node(node).unwrap().pos;
        edit.drag_node(node, [before[0] + 77.0, before[1] - 33.0]);
        edit.set_save_anchors(true);
        edit.resort();
        assert!(!edit.save_anchors);
        let after = edit.node(node).unwrap().pos;
        assert!(
            (after[0] - before[0]).abs() < 1.0 && (after[1] - before[1]).abs() < 1.0,
            "{before:?} {after:?}"
        );
    }

    #[test]
    fn saved_positions_are_kept_as_fractions_and_lay_the_nodes_out_again() {
        let mut edit = starter();
        edit.set_save_anchors(true);
        let (nodes, synapses, anchors) = edit.prepare_for_template_save();
        assert!(!anchors.is_empty());
        for (x, y) in anchors.values() {
            assert!((0.0..=1.0).contains(x), "{x}");
            assert!(y.is_finite());
        }
        // The game keeps them only when there are more than half as many as nodes that are used (the outputs nothing reaches have
        // none), which the starter brain, with its many such outputs, does not reach.
        let again = BrainEdit::fill(nodes, &synapses, &anchors);
        assert_eq!(again.save_anchors, anchors.len() > again.used.len() / 2);
        assert!(again.is_valid());
        // A brain whose nodes are mostly connected does: its positions come back as the fractions say.
        let mut edit = blank();
        let output = output_node(&edit);
        for index in 0..8 {
            let input = add_input(&mut edit, index);
            edit.click_node(input, true);
            edit.click_node(output, false);
            let at = [edit.origin()[0] + 25.0, -40.0 - 30.0 * index as f32];
            edit.drag_node(input, at);
        }
        edit.set_save_anchors(true);
        let (nodes, synapses, anchors) = edit.prepare_for_template_save();
        assert_eq!(
            anchors.len(),
            9,
            "the eight inputs and the output they reach"
        );
        let again = BrainEdit::fill(nodes, &synapses, &anchors);
        assert!(again.save_anchors);
        let origin = again.origin();
        for node in again
            .nodes
            .iter()
            .filter(|node| node.archetype() == Archetype::Input)
        {
            // Every input was at the same distance from the brain's left edge: the same fraction (here 0 of the width).
            assert!((node.pos[0] - origin[0]).abs() < 1e-3, "{:?}", node.pos);
        }
        assert!(again.is_valid());
    }

    #[test]
    fn the_simulator_steps_by_its_period_and_by_the_key() {
        let mut edit = starter();
        let rotate_input = edit.nodes.iter().find(|node| node.index == 13).unwrap().key;
        edit.edit_act(rotate_input, 1.0);
        let rotate = edit.nodes.iter().find(|node| node.index == 34).unwrap().key;
        let rest = edit.node(rotate).unwrap().value;
        // Playing steps once a period, not once a frame.
        edit.play(true);
        edit.tick(0.05, false, false);
        assert_eq!(edit.node(rotate).unwrap().value, rest);
        edit.tick(0.06, false, false);
        assert_ne!(edit.node(rotate).unwrap().value, rest);
        // The key's first press is one step and stops the simulation.
        edit.tick(0.0, true, true);
        assert!(!edit.playing);
        // Edits: the bias of an Inhibitory node stays in its range, and an input's bias is not one.
        let hidden = {
            edit.request_add_node([0.0, 0.0]);
            edit.add_node(NodeType::Inhibitory as usize, Archetype::Hidden);
            let Some(Selected::Node(key)) = edit.selected else {
                panic!("selected");
            };
            key
        };
        assert_eq!(edit.node(hidden).unwrap().bias, 1.0, "its own default");
        edit.edit_bias(hidden, 50.0);
        assert_eq!(edit.node(hidden).unwrap().bias, 10.0);
        edit.edit_bias(hidden, -3.0);
        assert_eq!(edit.node(hidden).unwrap().bias, 0.0001);
        edit.edit_function(hidden, NodeType::SoftLatch);
        assert_eq!(
            edit.node(hidden).unwrap().bias,
            5.0,
            "the panel's bias (0.0001) is in the soft latch's range: the function's own bias stays"
        );
        edit.edit_label(hidden, "memory");
        assert_eq!(edit.node(hidden).unwrap().desc, "memory");
        edit.edit_label(rotate, "not allowed");
        assert_ne!(edit.node(rotate).unwrap().desc, "not allowed");
        edit.edit_bias(rotate_input, 2.0);
        assert_eq!(edit.node(rotate_input).unwrap().bias, 0.0);
    }

    /// A hidden node of `kind` on the canvas, selected.
    fn hidden_node(edit: &mut BrainEdit, kind: NodeType) -> NodeKey {
        edit.request_add_node([0.0, 0.0]);
        edit.add_node(kind as usize, Archetype::Hidden);
        let Some(Selected::Node(key)) = edit.selected else {
            panic!("the new node is selected");
        };
        key
    }

    #[test]
    fn a_new_function_forces_the_panels_bias_into_its_range() {
        // `NodeEditPanel.CheckTypeForBiasSlider`: the bias slider is forced into an Inhibitory or a SoftLatch range with the value the
        // panel holds (the node's when it was selected or last edited from the panel, not the new function's own bias).
        let mut edit = blank();
        let hidden = hidden_node(&mut edit, NodeType::Sigmoid);
        assert_eq!(edit.panel_bias, 0.0, "the selected node's bias");
        edit.edit_function(hidden, NodeType::Inhibitory);
        assert_eq!(edit.node(hidden).unwrap().bias, 0.0001);
        assert_eq!(edit.panel_bias, 0.0001);
        // A function whose slider is not forced keeps its own bias, and the panel goes on showing the old value.
        edit.edit_function(hidden, NodeType::Mult);
        assert_eq!(edit.node(hidden).unwrap().bias, 1.0);
        assert_eq!(edit.panel_bias, 0.0001);
        // A panel bias in the new range is left alone, so the function's own bias stands.
        edit.edit_function(hidden, NodeType::Sine);
        edit.edit_bias(hidden, 500.0);
        assert_eq!(
            (edit.node(hidden).unwrap().bias, edit.panel_bias),
            (500.0, 500.0)
        );
        edit.edit_function(hidden, NodeType::SoftLatch);
        assert_eq!(edit.node(hidden).unwrap().bias, 5.0);
        assert_eq!(edit.panel_bias, 500.0);
        // 500 is out of an Inhibitory node's range: the slider takes 10, which is the node's bias too.
        edit.edit_function(hidden, NodeType::Inhibitory);
        assert_eq!(
            (edit.node(hidden).unwrap().bias, edit.panel_bias),
            (10.0, 10.0)
        );
        // Selecting a node again gives the panel that node's bias.
        edit.deselect();
        edit.click_node(hidden, false);
        assert_eq!(edit.panel_bias, 10.0);
    }

    #[test]
    fn a_node_added_while_a_connection_waits_for_its_end_is_that_end() {
        let mut edit = blank();
        let input = add_input(&mut edit, 0);
        edit.click_node(input, true);
        assert!(edit.connecting);
        // The panel offers no input then (the connection has its start); an output from it ends the connection (`SelectElement`).
        edit.request_add_node([400.0, -300.0]);
        assert_eq!(edit.add_hides, Archetype::Input);
        let index = edit.available(Archetype::Output)[0];
        edit.add_node(index, Archetype::Output);
        assert!(!edit.connecting && edit.temp.is_none() && !edit.add_mode);
        assert_eq!(edit.edges.len(), 1);
        let output = edit.key_of_index(index).expect("the output");
        assert_eq!(
            (edit.edges[0].from, edit.edges[0].to),
            (Some(input), Some(output))
        );
        assert!(
            edit.selected.is_none(),
            "a connection that is made is not selected"
        );
        // A hidden node can be the end of a connection from an input too.
        let mut edit = blank();
        let input = add_input(&mut edit, 0);
        edit.click_node(input, true);
        edit.request_add_node([400.0, -300.0]);
        edit.add_node(NodeType::Linear as usize, Archetype::Hidden);
        assert!(!edit.connecting);
        assert_eq!(edit.hidden_count(), 1);
        assert!(edit.edges[0].to.is_some());
    }

    #[test]
    fn a_node_added_with_shift_held_starts_a_connection() {
        let mut edit = blank();
        edit.request_add_node([300.0, -300.0]);
        edit.add_node_holding(0, Archetype::Input, true);
        assert!(edit.connecting && edit.selected.is_none());
        let temp = edit.temp.expect("the connection being made");
        assert_eq!(edit.edge(temp).unwrap().missing, Archetype::Output);
        assert!(edit.any_change);
    }

    #[test]
    fn an_output_that_stays_after_it_is_deleted_is_offered_again() {
        let mut edit = blank();
        let output = edit
            .nodes
            .iter()
            .find(|node| node.archetype() == Archetype::Output && node.kind == NodeType::Sigmoid)
            .expect("a sigmoid output on the canvas")
            .key;
        let index = edit.node(output).unwrap().index;
        assert!(!edit.available(Archetype::Output).contains(&index));
        edit.delete_node(output);
        assert!(edit.node(output).is_some(), "a sigmoid output stays");
        assert!(
            edit.available(Archetype::Output).contains(&index),
            "and the add-node panel offers it again"
        );
        // Adding it moves the node that is there instead of making another.
        let count = edit.nodes.len();
        edit.request_add_node([500.0, -200.0]);
        edit.add_node(index, Archetype::Output);
        assert_eq!(edit.nodes.len(), count);
        assert_eq!(edit.node(output).unwrap().pos, [500.0, -200.0]);
        assert!(!edit.available(Archetype::Output).contains(&index));
        // An input that is deleted is offered again too, and its node is gone from the canvas.
        let input = add_input(&mut edit, 4);
        assert!(!edit.available(Archetype::Input).contains(&4));
        edit.delete_node(input);
        assert!(edit.node(input).is_none());
        assert!(edit.available(Archetype::Input).contains(&4));
    }

    #[test]
    fn an_output_that_is_deleted_goes_back_to_its_default_bias_for_the_next_time_it_is_added() {
        let mut edit = starter();
        // An output with a bias of its own: the starter brain's Accelerate.
        let accelerate = edit.nodes.iter().find(|node| node.index == 33).unwrap().key;
        let kind = edit.node(accelerate).unwrap().kind;
        edit.edit_bias(accelerate, 2.0);
        edit.delete_node(accelerate);
        if kind == NodeType::Sigmoid {
            // A sigmoid output stays where it is (with its default bias).
            assert!(edit.node(accelerate).is_some());
            return;
        }
        assert!(
            edit.node(accelerate).is_none(),
            "an output that is not a sigmoid leaves the canvas"
        );
        edit.request_add_node([250.0, -250.0]);
        edit.add_node(33, Archetype::Output);
        let again = edit.key_of_index(33).expect("on the canvas again");
        assert_eq!(
            edit.node(again).unwrap().bias,
            default_bias_of(kind),
            "the bias it was reset to, not the 2 it had"
        );
    }

    #[test]
    fn a_split_selects_the_new_node_with_its_panel_bias() {
        let mut edit = starter();
        edit.panel_bias = 9.0;
        let edge = edit.edges[0].key;
        edit.split_connection(edge, true);
        assert_eq!(edit.panel_bias, 0.0, "the new node is Linear, with no bias");
    }
}

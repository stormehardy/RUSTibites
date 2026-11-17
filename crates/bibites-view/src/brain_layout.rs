//! The layout of a brain diagram: `BrainPanelCommon.FillBrainPanel` with `BrainSorter` (the `bibites-brainsort`
//! crate), `PlaceNodesAccordingToSort`, `PlaceNodesAccordingToAnchors`, `SaveNodePositions` and `SortN`.
//!
//! Positions are in the node holder's own space, as the game's `localPosition`s: `x` from the holder's left edge
//! (0 ..= `width`), `y` from its top edge, negative downwards (0 ..= `-height`). The inputs sit on the left and the
//! outputs on the right, one column each; the hidden nodes are placed by the sorter between them; outputs nothing
//! reaches lie along the bottom.

use std::collections::BTreeMap;

use bibites_brainsort::{
    Archetype, Layout as Sorter, N_INPUTS, N_OUTPUTS, Rng, SortNode, SortSynapse, Vec2, make_groups,
};
use bibites_core::brain::{BrainGraphView, NodeType};

/// `BrainPanelCommon.TargetVerticalStep`: the height a group of weight 1 asks for.
pub const VERTICAL_STEP: f32 = 75.0;

/// What the layout reads of a brain: the nodes and the synapses, in the brain's own order.
#[derive(Clone, Debug, PartialEq)]
pub struct Graph {
    /// Every node (`NEATBrain.Node[]`).
    pub nodes: Vec<SortNode>,
    /// Every synapse (`NEATBrain.Synaps[]`).
    pub synapses: Vec<SortSynapse>,
}

impl Graph {
    /// The graph of a brain's read-only view.
    #[must_use]
    pub fn of(view: &BrainGraphView) -> Self {
        Self {
            nodes: view
                .nodes
                .iter()
                .map(|node| {
                    SortNode::new(
                        node.index,
                        node.innovation,
                        node.node_type == NodeType::Sigmoid,
                        node.base_activation,
                    )
                })
                .collect(),
            synapses: view
                .synapses
                .iter()
                .map(|synapse| {
                    SortSynapse::new(
                        synapse.innovation,
                        synapse.node_in,
                        synapse.node_out,
                        synapse.weight,
                        synapse.enabled,
                    )
                })
                .collect(),
        }
    }
}

/// A diagram's layout: the sorter's state, the sizes the game derives from the groups and where every used node
/// is.
#[derive(Clone, Debug)]
pub struct Diagram {
    /// The sorter, kept to sort again (`SortN`).
    pub sorter: Sorter,
    /// The brain.
    pub graph: Graph,
    /// `usedIndexes`: the nodes that take part, group by group.
    pub used: Vec<usize>,
    /// `nG`: the groups that have only outputs (what nothing reaches), lying along the bottom.
    pub inputless_groups: Vec<usize>,
    /// The holder's width (`width`).
    pub width: f32,
    /// The height the groups share (`height`).
    pub height: f32,
    /// The vertical step of the input column (`stepI`), negative.
    pub step_i: f32,
    /// The vertical step of the output column (`stepO`), negative.
    pub step_o: f32,
    /// The vertical step between the inputless output rows (`stepG`), negative.
    pub step_g: f32,
    /// Where each node is (indexed by the node's index), `None` for a node that is not drawn.
    pub positions: Vec<Option<[f32; 2]>>,
    /// How many synapses end at each node (`Node.NIn`, which the species panel counts itself).
    pub n_in: Vec<u32>,
}

/// The holder width the game's layout starts from (the prefab's `Holder` is 395.17 wide).
pub const START_HOLDER_WIDTH: f32 = 395.17;

impl Diagram {
    /// `FillBrainPanel`: the groups, the sizes and the places of the nodes. `anchors` are the template's saved
    /// positions (`nodeAnchors`, keyed by node innovation: fractions of the holder), `holder_width` the width the
    /// holder had (`holderRT.rect.width`) and `max_height` the cap `(1080 - 350 totalScale) / UIScale`.
    #[must_use]
    pub fn fill(
        graph: Graph,
        anchors: Option<&BTreeMap<i64, (f32, f32)>>,
        holder_width: f32,
        max_height: f32,
        rng: &mut Rng,
    ) -> Self {
        let sorter = make_groups(&graph.nodes, &graph.synapses);
        let inputless_groups: Vec<usize> = sorter
            .groups
            .iter()
            .enumerate()
            .filter(|(_, group)| group.inputs.len() + group.hidden.len() == 0)
            .map(|(index, _)| index)
            .collect();
        let n_g = inputless_groups.len() as f32;
        let inputless_outputs: usize = inputless_groups
            .iter()
            .map(|&group| sorter.groups[group].outputs.len())
            .sum();
        let used = sorter.used_indices_of_groups();
        let used_inputs = used.iter().filter(|&&index| index < N_INPUTS).count();
        let used_outputs = used
            .iter()
            .filter(|&&index| index < N_INPUTS + N_OUTPUTS)
            .count()
            .saturating_sub(used_inputs)
            .saturating_sub(inputless_outputs);
        let hidden_nodes = graph.nodes.len().saturating_sub(N_INPUTS + N_OUTPUTS);
        let groups_height: f32 = sorter
            .groups
            .iter()
            .map(|group| group.weight() + 1.0)
            .sum::<f32>()
            - 1.0;
        let height = (groups_height * VERTICAL_STEP).min(max_height);
        let step_g = -height / groups_height;
        let wanted = 400.0 + 150.0 * (1.0 - 2.0_f32.powf(-(hidden_nodes as f32) / 5.0));
        let width = wanted.max(holder_width);
        let room = (height + n_g * step_g).max(1.0);
        let step_i = -room / used_inputs.saturating_sub(1).max(1) as f32;
        let step_o = -room / used_outputs.saturating_sub(1).max(1) as f32;
        let mut n_in = vec![0_u32; graph.nodes.len()];
        for synapse in &graph.synapses {
            if let Some(count) = n_in.get_mut(synapse.node_out) {
                *count += 1;
            }
        }
        let mut diagram = Self {
            positions: vec![None; graph.nodes.len()],
            sorter,
            graph,
            used,
            inputless_groups,
            width,
            height,
            step_i,
            step_o,
            step_g,
            n_in,
        };
        // `anchors.Count > 3 * usedIndexes.Count / 5`: enough saved positions to lay the nodes out by them.
        if let Some(anchors) = anchors
            && anchors.len() > 3 * diagram.used.len() / 5
        {
            diagram.place_according_to_anchors(anchors);
        } else {
            // `BrainSorter.Sort(nodeGroups, 3); PlaceNodesAccordingToSort();`
            diagram.sorter.sort(3, None, rng);
            diagram.place_according_to_sort();
        }
        diagram
    }

    /// `SortN(n)`: sorts again and places the nodes by the sort (when there is more than one group).
    pub fn sort_n(&mut self, n: usize, rng: &mut Rng) {
        if self.sorter.groups.len() > 1 && n >= 1 {
            self.sorter.sort(n, None, rng);
            self.place_according_to_sort();
        }
    }

    /// `SortHiddenOnce`: one step of the hidden nodes' physics in every group that has some (`dt` is the frame
    /// time, `scalar` 1, or 0 with Ctrl held).
    pub fn sort_hidden_once(&mut self, dt: f32, scalar: f32) {
        if self.sorter.groups.len() <= 1 {
            return;
        }
        for group in 0..self.sorter.groups.len() {
            if !self.sorter.groups[group].hidden.is_empty() {
                self.sorter.optimize_hidden_placement(group, dt, scalar);
            }
        }
        self.place_according_to_sort();
    }

    /// The vertical step of a node (`stepI` for an input, `stepO` otherwise).
    #[must_use]
    pub fn step_of(&self, index: usize) -> f32 {
        if Archetype::of_index(index) == Archetype::Input {
            self.step_i
        } else {
            self.step_o
        }
    }

    /// `PlaceNodesAccordingToSort`: every group gets a share of the height by its weight, heaviest first from the
    /// top, and its nodes are put where the sorter left them within it.
    fn place_according_to_sort(&mut self) {
        let weights: Vec<f32> = self
            .sorter
            .groups
            .iter()
            .map(|group| group.weight())
            .collect();
        let sum: f32 = weights.iter().sum();
        let sum_with_gaps: f32 = weights.iter().map(|weight| weight + 1.0).sum();
        let gap = self.height / (sum_with_gaps - 1.0);
        let sum = if sum < 0.1 { 1.0 } else { sum };
        let mut start = 0.0_f32;
        for group in self.sorter.groups_by_descending_weight() {
            let share = (self.height - gap * (self.sorter.groups.len() as f32 - 1.0))
                * weights[group]
                / sum;
            self.sorter.set_start_end(group, start, share);
            let nodes = self.sorter.groups[group].nodes.clone();
            for node in nodes {
                let step = if self.sorter.nodes[node].archetype() == Archetype::Input {
                    self.step_i
                } else {
                    self.step_o
                };
                let pos = self.sorter.nodes[node].group_pos;
                self.sorter.nodes[node].panel_pos =
                    Vec2::new(pos.x, (start - share * pos.y) / step);
            }
            start -= share + gap;
        }
        self.update_placement();
    }

    /// `UpdatePlacement`: the panel positions as holder coordinates.
    fn update_placement(&mut self) {
        for group in self.sorter.groups_by_descending_weight() {
            for &node in &self.sorter.groups[group].nodes {
                let helper = &self.sorter.nodes[node];
                let step = if helper.archetype() == Archetype::Input {
                    self.step_i
                } else {
                    self.step_o
                };
                let index = helper.index();
                if let Some(slot) = self.positions.get_mut(index) {
                    *slot = Some([helper.panel_pos.x * self.width, helper.panel_pos.y * step]);
                }
            }
        }
    }

    /// `PlaceNodesAccordingToAnchors`: the nodes go where the template's saved positions (fractions of the
    /// holder, keyed by innovation) put them; the nodes without one are put in line (inputs and outputs) or at the
    /// mean of their neighbours weighted by the connections (hidden), and the groups of outputs nothing reaches lie
    /// along the bottom.
    fn place_according_to_anchors(&mut self, anchors: &BTreeMap<i64, (f32, f32)>) {
        let mut anchors = anchors.clone();
        let inputless_nodes: Vec<usize> = self
            .inputless_groups
            .iter()
            .flat_map(|&group| self.sorter.groups[group].outputs.iter().copied())
            .collect();
        let inputless_indices: Vec<usize> = inputless_nodes
            .iter()
            .map(|&node| self.sorter.nodes[node].index())
            .collect();
        let inputless_inovs: Vec<i64> = inputless_nodes
            .iter()
            .map(|&node| self.sorter.nodes[node].inov())
            .collect();
        let inov_of = |index: usize| self.graph.nodes[index].inov;
        let mut left: Vec<usize> = self
            .used
            .iter()
            .copied()
            .filter(|index| !inputless_indices.contains(index))
            .collect();
        let input_inovs: Vec<i64> = self
            .used
            .iter()
            .filter(|&&index| Archetype::of_index(index) == Archetype::Input)
            .map(|&index| inov_of(index))
            .collect();
        let output_inovs: Vec<i64> = self
            .used
            .iter()
            .filter(|&&index| Archetype::of_index(index) == Archetype::Output)
            .map(|&index| inov_of(index))
            .filter(|inov| !inputless_inovs.contains(inov))
            .collect();
        for inov in &inputless_inovs {
            anchors.remove(inov);
        }
        let max_y = |wanted: &dyn Fn(i64) -> bool| -> f32 {
            anchors
                .iter()
                .filter(|(inov, _)| wanted(**inov))
                .map(|(_, anchor)| anchor.1)
                .fold(f32::NEG_INFINITY, f32::max)
        };
        let input_max = max_y(&|inov| input_inovs.contains(&inov));
        let output_max = max_y(&|inov| output_inovs.contains(&inov));
        let hidden_max = if anchors.len() <= input_inovs.len() + output_inovs.len() {
            1.0
        } else {
            max_y(&|inov| !input_inovs.contains(&inov) && !output_inovs.contains(&inov))
        };
        let n_g = self.inputless_groups.len() as f32;
        let group_share = 1.0 + n_g * self.step_g / self.height;
        // (The game throws when there is no anchor for a column; the column is left as it is here.)
        let squeeze = |count: usize, highest: f32| -> f32 {
            if highest.is_finite() {
                1.0_f32.min((count as f32 - 1.0) / highest)
            } else {
                1.0
            }
        };
        let input_squeeze = squeeze(input_inovs.len(), input_max);
        let output_squeeze = squeeze(output_inovs.len(), output_max);
        let hidden_squeeze = group_share * 1.0_f32.min(1.0 / hidden_max);
        let mut placed_inputs = 0_u32;
        let mut placed_outputs = 0_u32;
        self.positions = vec![None; self.graph.nodes.len()];
        // By height, the highest anchors first (a stable sort, as `orderby`).
        let mut ordered: Vec<(i64, (f32, f32))> = anchors.iter().map(|(k, v)| (*k, *v)).collect();
        ordered.sort_by(|a, b| a.1.1.total_cmp(&b.1.1));
        for (inov, anchor) in ordered {
            let Some(index) = self
                .used
                .iter()
                .copied()
                .find(|&index| inov_of(index) == inov)
            else {
                continue;
            };
            match Archetype::of_index(index) {
                Archetype::Input => {
                    self.positions[index] = Some([
                        anchor.0 * self.width,
                        anchor.1 * input_squeeze * self.step_i,
                    ]);
                    placed_inputs += 1;
                }
                Archetype::Output => {
                    if self.n_in[index] > 0 {
                        self.positions[index] = Some([
                            anchor.0 * self.width,
                            anchor.1 * output_squeeze * self.step_o,
                        ]);
                        placed_outputs += 1;
                    }
                }
                Archetype::Hidden => {
                    self.positions[index] = Some([
                        anchor.0 * self.width,
                        anchor.1 * -self.height * hidden_squeeze,
                    ]);
                }
            }
            left.retain(|&other| other != index);
        }
        left.sort_unstable();
        for index in left {
            match Archetype::of_index(index) {
                Archetype::Input => {
                    self.positions[index] = Some([0.0, placed_inputs as f32 * self.step_i]);
                    placed_inputs += 1;
                }
                Archetype::Output => {
                    self.positions[index] = Some([self.width, placed_outputs as f32 * self.step_o]);
                    placed_outputs += 1;
                }
                Archetype::Hidden => {
                    let mut sum = [0.0_f32, 0.0];
                    let mut weights = 1e-6_f32;
                    for synapse in &self.graph.synapses {
                        let other = if synapse.node_in == index {
                            synapse.node_out
                        } else if synapse.node_out == index {
                            synapse.node_in
                        } else {
                            continue;
                        };
                        let weight = synapse.weight.abs();
                        let at = match self.positions.get(other).copied().flatten() {
                            Some(at) => at,
                            None => [
                                if synapse.node_in == index {
                                    self.width
                                } else {
                                    0.0
                                },
                                -self.height / 2.0,
                            ],
                        };
                        sum[0] += at[0] * weight;
                        sum[1] += at[1] * weight;
                        weights += weight;
                    }
                    self.positions[index] = Some([sum[0] / weights, sum[1] / weights]);
                }
            }
        }
        if n_g > 0.0 {
            let mut y = -self.height * group_share;
            for &group in &self.inputless_groups {
                y += self.step_g;
                for &node in &self.sorter.groups[group].nodes {
                    let helper = &self.sorter.nodes[node];
                    self.positions[helper.index()] = Some([helper.group_pos.x * self.width, y]);
                }
            }
        }
    }

    /// `SaveNodePositions`: the positions of the nodes as fractions of the holder, by innovation, written into
    /// `anchors` (which keeps what it has for nodes that are not drawn now).
    pub fn save_positions(&self, anchors: &mut BTreeMap<i64, (f32, f32)>) {
        for &index in &self.used {
            let Some(at) = self.positions.get(index).copied().flatten() else {
                continue;
            };
            let step = match Archetype::of_index(index) {
                Archetype::Input => self.step_i,
                Archetype::Output => {
                    if self.n_in[index] == 0 {
                        continue;
                    }
                    self.step_o
                }
                Archetype::Hidden => {
                    -(self.height + self.inputless_groups.len() as f32 * self.step_g)
                }
            };
            anchors.insert(
                self.graph.nodes[index].inov,
                (at[0] / self.width, at[1] / step),
            );
        }
    }

    /// Holder coordinates of node `index`.
    #[must_use]
    pub fn position(&self, index: usize) -> Option<[f32; 2]> {
        self.positions.get(index).copied().flatten()
    }

    /// Puts node `index` at `at`, kept inside the holder (`UINeuron.OnDrag`): `x` within `0 ..= holder_width`
    /// and `y` within `-holder_height ..= 0`, rounded to the node's step with Shift held.
    pub fn drag_node(&mut self, index: usize, at: [f32; 2], holder: [f32; 2], snap: bool) {
        if self.positions.get(index).copied().flatten().is_none() {
            return;
        }
        let x = at[0].min(holder[0]).max(0.0);
        let mut y = at[1].min(0.0).max(-holder[1]);
        if snap {
            let step = self.step_of(index);
            if step != 0.0 {
                y = (y / step).round_ties_even() * step;
            }
        }
        self.positions[index] = Some([x, y]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A brain with `hidden` hidden nodes in a chain from input 0 to output 0, plus the given extra synapses
    /// `(from, to)`.
    fn chain(hidden: usize, extra: &[(usize, usize)]) -> Graph {
        let total = N_INPUTS + N_OUTPUTS + hidden;
        let nodes: Vec<SortNode> = (0..total)
            .map(|i| SortNode::new(i, i as i64 + 1, false, 0.0))
            .collect();
        let mut synapses = Vec::new();
        let first_hidden = N_INPUTS + N_OUTPUTS;
        let mut previous = 0;
        for k in 0..hidden {
            synapses.push(SortSynapse::new(
                synapses.len() as i64 + 1,
                previous,
                first_hidden + k,
                1.0,
                true,
            ));
            previous = first_hidden + k;
        }
        synapses.push(SortSynapse::new(
            synapses.len() as i64 + 1,
            previous,
            N_INPUTS,
            1.0,
            true,
        ));
        for &(from, to) in extra {
            synapses.push(SortSynapse::new(
                synapses.len() as i64 + 1,
                from,
                to,
                0.5,
                true,
            ));
        }
        Graph { nodes, synapses }
    }

    fn fill(graph: Graph, anchors: Option<&BTreeMap<i64, (f32, f32)>>) -> Diagram {
        Diagram::fill(graph, anchors, START_HOLDER_WIDTH, 730.0, &mut Rng::new(7))
    }

    #[test]
    fn the_inputs_are_on_the_left_and_the_outputs_on_the_right() {
        let diagram = fill(chain(2, &[(1, N_INPUTS + 1), (2, N_INPUTS + 1)]), None);
        assert_eq!(
            diagram.used.len(),
            3 + 2 + 2,
            "inputs 0, 1, 2; outputs 0, 1; two hidden"
        );
        for index in 0..3 {
            let at = diagram.position(index).expect("an input");
            assert_eq!(at[0], 0.0, "input {index}");
            assert!(at[1] <= 0.0);
        }
        for index in [N_INPUTS, N_INPUTS + 1] {
            let at = diagram.position(index).expect("an output");
            assert_eq!(at[0], diagram.width);
            assert!(at[1] <= 0.0 && at[1] >= -diagram.height - 1.0);
        }
        // The hidden nodes are between the columns.
        for index in [N_INPUTS + N_OUTPUTS, N_INPUTS + N_OUTPUTS + 1] {
            let at = diagram.position(index).expect("a hidden node");
            assert!(at[0] > 0.0 && at[0] < diagram.width, "{at:?}");
        }
        // Nodes that nothing connects are not drawn.
        assert!(diagram.position(20).is_none());
    }

    #[test]
    fn the_sizes_follow_the_groups_and_the_hidden_count() {
        let few = fill(chain(1, &[]), None);
        let many = fill(chain(20, &[]), None);
        // `400 + 150 (1 - 2^(-hidden / 5))`: more hidden nodes widen the holder up to 550.
        assert!(
            few.width >= 400.0 && few.width < many.width,
            "{} {}",
            few.width,
            many.width
        );
        assert!(many.width <= 550.0);
        assert!((few.width - (400.0 + 150.0 * (1.0 - 2.0_f32.powf(-0.2)))).abs() < 1e-3);
        // The height is 75 for every unit of weight (and the gaps between the groups), capped.
        assert!(few.height <= 730.0 && many.height <= 730.0);
        assert!(few.step_g < 0.0);
        // The width ratchets: a holder that was wide stays wide.
        let wide = Diagram::fill(chain(1, &[]), None, 520.0, 730.0, &mut Rng::new(7));
        assert_eq!(wide.width, 520.0);
        // A smaller cap shortens the diagram.
        let squat = Diagram::fill(
            chain(6, &[]),
            None,
            START_HOLDER_WIDTH,
            100.0,
            &mut Rng::new(7),
        );
        assert!(squat.height <= 100.0);
    }

    #[test]
    fn a_layout_is_the_same_for_the_same_seed_and_the_sorter_can_run_again() {
        let one = fill(chain(5, &[(3, N_INPUTS + 2), (4, N_INPUTS + 3)]), None);
        let two = fill(chain(5, &[(3, N_INPUTS + 2), (4, N_INPUTS + 3)]), None);
        assert_eq!(one.positions, two.positions);
        let mut again = one.clone();
        again.sort_n(8, &mut Rng::new(11));
        // The same nodes are drawn, still in their columns.
        for (a, b) in one.positions.iter().zip(&again.positions) {
            assert_eq!(a.is_some(), b.is_some());
        }
        for index in 0..N_INPUTS {
            if let Some(at) = again.position(index) {
                assert_eq!(at[0], 0.0, "inputs stay in the left column");
            }
        }
    }

    #[test]
    fn outputs_nothing_reaches_lie_along_the_bottom() {
        // Output 5 feeds output 6, and no input reaches either: a group of outputs without inputs.
        let graph = chain(1, &[(N_INPUTS + 5, N_INPUTS + 6)]);
        let diagram = fill(graph, None);
        assert!(!diagram.inputless_groups.is_empty());
        let main = diagram.position(N_INPUTS).expect("the chain's output");
        for index in [N_INPUTS + 5, N_INPUTS + 6] {
            let at = diagram
                .position(index)
                .expect("an output of the bottom group");
            assert!(at[1] < main[1], "{index} {at:?} is below {main:?}");
        }
        // The outputs of that group do not count for the output column's step.
        assert!(diagram.step_g < 0.0);
    }

    #[test]
    fn saved_positions_put_the_nodes_back_where_they_were() {
        let graph = chain(3, &[(1, N_INPUTS + 1), (2, N_INPUTS + 1)]);
        let diagram = fill(graph.clone(), None);
        let mut anchors = BTreeMap::new();
        diagram.save_positions(&mut anchors);
        assert_eq!(anchors.len(), diagram.used.len());
        // A diagram filled with them has every node where it was.
        let restored = fill(graph.clone(), Some(&anchors));
        for &index in &diagram.used {
            let (was, is) = (
                diagram.position(index).unwrap(),
                restored.position(index).unwrap(),
            );
            assert!(
                (was[0] - is[0]).abs() < 0.5 && (was[1] - is[1]).abs() < 0.5,
                "node {index}: {was:?} became {is:?}"
            );
        }
        // A node that was moved comes back where it was moved to (across; the height squeezes into the group).
        let hidden = N_INPUTS + N_OUTPUTS + 1;
        let mut moved = diagram.clone();
        moved.drag_node(hidden, [111.0, -30.0], [moved.width, 300.0], false);
        let mut anchors = BTreeMap::new();
        moved.save_positions(&mut anchors);
        let restored = fill(graph, Some(&anchors));
        let at = restored.position(hidden).unwrap();
        assert!((at[0] - 111.0).abs() < 0.5, "{at:?}");
        assert!((at[1] - -30.0).abs() < 0.5, "{at:?}");
        // Fewer saved positions than three fifths of the nodes are not used: the sorter lays the nodes out.
        let mut few = BTreeMap::new();
        few.insert(1_i64, (0.5, 0.5));
        let sorted = fill(chain(3, &[]), Some(&few));
        assert_eq!(sorted.position(0).unwrap()[0], 0.0);
    }

    #[test]
    fn a_dragged_node_stays_in_the_holder_and_snaps_with_shift() {
        let mut diagram = fill(chain(2, &[]), None);
        let holder = [diagram.width, 300.0];
        diagram.drag_node(1, [-40.0, 90.0], holder, false);
        assert_eq!(diagram.position(1), None, "node 1 is not drawn");
        diagram.drag_node(0, [-40.0, 90.0], holder, false);
        assert_eq!(diagram.position(0), Some([0.0, 0.0]));
        diagram.drag_node(0, [diagram.width + 80.0, -999.0], holder, false);
        assert_eq!(diagram.position(0), Some([diagram.width, -300.0]));
        // With Shift the height snaps to the node's step.
        diagram.drag_node(0, [30.0, -123.0], holder, true);
        let step = diagram.step_i;
        let y = diagram.position(0).unwrap()[1];
        assert!(((y / step) - (y / step).round()).abs() < 1e-4, "{y} {step}");
    }

    #[test]
    fn the_graph_is_made_from_a_brains_view() {
        use bibites_core::brain::Brain;
        let brain = Brain::starter();
        let view = brain.graph_view();
        let graph = Graph::of(&view);
        assert_eq!(graph.nodes.len(), view.nodes.len());
        assert_eq!(graph.synapses.len(), view.synapses.len());
        let diagram = Diagram::fill(graph, None, START_HOLDER_WIDTH, 730.0, &mut Rng::new(3));
        assert!(!diagram.used.is_empty());
        assert!(diagram.width >= 400.0);
        for &index in &diagram.used {
            let at = diagram.position(index).expect("a used node has a place");
            assert!(at[0].is_finite() && at[1].is_finite(), "{index} {at:?}");
        }
    }
}

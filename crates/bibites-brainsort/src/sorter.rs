//! `BrainSorter`: `MakeGroups`, `UsedIndicesOfGroups`, `Sort`, `ComputeLocalMinima` and
//! `OptimizeHiddenPlacement`, operating on the arenas of a [`Layout`].

use std::collections::{HashMap, HashSet, VecDeque};

use crate::group::NodeGroup;
use crate::math::{self, compare_f32, remove_first};
use crate::nodes::{ConnectionSortHelper, HiddenNodeAffinity, NodeSortHelper};
use crate::path::{InputOutputAffinity, InputOutputPath};
use crate::rng::Rng;
use crate::types::{
    AffinityId, Archetype, ConnId, MAX_NODE_PATH, N_INPUTS, N_OUTPUTS, NodeId, PathId, SortNode,
    SortSynapse, Tuning,
};
use crate::vec2::Vec2;

/// Number of `OptimizeHiddenPlacement` rounds `Sort` runs per group (`int num3 = 20`).
const HIDDEN_OPTIMIZATION_ROUNDS: i32 = 20;

/// The result of [`make_groups`]: the arenas of every helper object the C# `MakeGroups` creates,
/// plus the resulting list of [`NodeGroup`]s.
///
/// Every `*Id` stored in a group, affinity, path or helper indexes into the matching `Vec`
/// here. The fields are public so a consumer can read positions and write `panel_pos`
/// while iterating groups (borrowing `groups` and `nodes` as separate fields).
#[derive(Clone, Debug, Default)]
pub struct Layout {
    /// The `BrainSorter` statics this layout sorts with (defaults = the game's values).
    pub tuning: Tuning,
    /// One [`NodeSortHelper`] per node that takes part in the diagram (the C# `dictionary`
    /// values), in the order the C# `list` discovers them.
    pub nodes: Vec<NodeSortHelper>,
    /// One [`ConnectionSortHelper`] per input synapse, in input order (so `ConnId` ==
    /// position in the `connections` slice passed to [`make_groups`]).
    pub connections: Vec<ConnectionSortHelper>,
    /// Every [`InputOutputAffinity`] created, in creation order.
    pub affinities: Vec<InputOutputAffinity>,
    /// The finished (blocked or done) [`InputOutputPath`]s, in finishing order.
    pub paths: Vec<InputOutputPath>,
    /// The `List<NodeGroup>` that `MakeGroups` returns, in the same order.
    pub groups: Vec<NodeGroup>,
    /// The C# `dictionary` keys: position in the input `nodes` slice -> arena id.
    node_ids: Vec<Option<NodeId>>,
}

/// `BrainSorter.MakeGroups(nodes, connections)`: splits a brain into [`NodeGroup`]s.
///
/// `nodes` is indexed by position (the C# does `nodes[index]`), so for a real brain
/// `nodes[i].index == i`. If a synapse (or an active output node) refers to a position outside
/// `nodes`, the C# would throw `ArgumentOutOfRangeException`; this port returns an empty
/// [`Layout`] (no groups) instead.
///
/// The result is not yet sorted: input/output nodes sit on the group's left/right edge at even
/// spacing (a lone input or output has `y = NaN`, as in the C#), hidden nodes are at the origin.
/// Call [`Layout::sort`] to lay them out.
pub fn make_groups(nodes: &[SortNode], connections: &[SortSynapse]) -> Layout {
    build_layout(nodes, connections).unwrap_or_default()
}

/// Looks up the C# `dictionary[index]`.
fn lookup(dictionary: &[Option<NodeId>], index: usize) -> Option<NodeId> {
    dictionary.get(index).copied().flatten()
}

/// The body of `MakeGroups`; `None` stands for "the C# would have thrown".
fn build_layout(sort_nodes: &[SortNode], sort_connections: &[SortSynapse]) -> Option<Layout> {
    // `list`: indices of every node that takes part, in discovery order (connection endpoints
    // first, then the inputless outputs). `list2`: output nodes without any synapse that are
    // nevertheless active (sigmoid, or with a non-zero base activation).
    let mut list: Vec<usize> = Vec::new();
    let mut list2: Vec<usize> = Vec::new();
    for connection in sort_connections {
        if !list.contains(&connection.node_in) {
            list.push(connection.node_in);
        }
        if !list.contains(&connection.node_out) {
            list.push(connection.node_out);
        }
    }
    for node in sort_nodes {
        if node.archetype == Archetype::Output
            && (node.is_sigmoid || math::abs(node.base_activation) != 0.0)
            && !list.contains(&node.index)
        {
            list2.push(node.index);
            list.push(node.index);
        }
    }

    // `dictionary`: index -> NodeSortHelper (`nodes[item2]` indexes by position).
    let mut dictionary: Vec<Option<NodeId>> = vec![None; sort_nodes.len()];
    let mut nodes: Vec<NodeSortHelper> = Vec::with_capacity(list.len());
    for &item in &list {
        let source = sort_nodes.get(item)?;
        *dictionary.get_mut(item)? = Some(nodes.len());
        nodes.push(NodeSortHelper::new(*source));
    }
    let mut connections: Vec<ConnectionSortHelper> = Vec::with_capacity(sort_connections.len());
    for connection in sort_connections {
        let from = lookup(&dictionary, connection.node_in)?;
        let to = lookup(&dictionary, connection.node_out)?;
        let id: ConnId = connections.len();
        connections.push(ConnectionSortHelper::new(*connection, from, to));
        nodes[from].output_connections.push(id);
        nodes[to].input_connections.push(id);
    }

    // Walk every input towards the outputs and every output towards the inputs.
    // `paths` keeps the finished ones; `done_paths` is the C# `list4`, `blocked_paths` `list5`.
    let mut paths: Vec<InputOutputPath> = Vec::new();
    let mut done_paths: Vec<PathId> = Vec::new();
    let mut blocked_paths: Vec<PathId> = Vec::new();
    // `list3`: the live paths. The head is always the path being extended; forks are appended.
    let mut live: VecDeque<InputOutputPath> = VecDeque::new();
    for &item in &list {
        if item >= N_INPUTS + N_OUTPUTS {
            continue;
        }
        let toward_outputs = item < N_INPUTS;
        live.push_back(InputOutputPath::new(
            lookup(&dictionary, item)?,
            toward_outputs,
        ));
        while !live.is_empty() {
            let head_next = live[0].next_node;
            let count = if toward_outputs {
                nodes[head_next].output_connections.len()
            } else {
                nodes[head_next].input_connections.len()
            };
            if count == 0 {
                live.pop_front();
                continue;
            }
            for num in (0..count).rev() {
                // The last connection (num == 0) advances the head path itself; every other one
                // advances a fork of it, unless too many paths are already alive.
                let slot = if num > 0 {
                    if live.len() > MAX_NODE_PATH {
                        continue;
                    }
                    let fork = live[0].duplicate();
                    live.push_back(fork);
                    live.len() - 1
                } else {
                    0
                };
                let path = &mut live[slot];
                let node = &nodes[path.next_node];
                let outgoing = if toward_outputs {
                    &node.output_connections
                } else {
                    &node.input_connections
                };
                let next_connection = *outgoing.get(num)?;
                if path.advance_and_check_done(next_connection, &mut nodes, &connections) {
                    if let Some(finished) = live.remove(slot) {
                        let (blocked, done) = (finished.blocked, finished.done);
                        let id = paths.len();
                        paths.push(finished);
                        if blocked {
                            blocked_paths.push(id);
                        }
                        if done {
                            done_paths.push(id);
                        }
                    }
                }
            }
        }
    }

    // One affinity per (source, destination) pair of the finished paths; further paths with the
    // same endpoints are collapsed into it.
    let mut affinities: Vec<InputOutputAffinity> = Vec::new();
    let mut by_endpoints: HashMap<(NodeId, NodeId), AffinityId> = HashMap::new();
    for &path_id in &done_paths {
        let key = (paths[path_id].source_node, paths[path_id].next_node);
        if let Some(&existing) = by_endpoints.get(&key) {
            affinities[existing].collapse_path_into(path_id, &paths[path_id]);
        } else {
            let id = InputOutputAffinity::create(path_id, &paths, &mut nodes, &mut affinities);
            by_endpoints.insert(key, id);
        }
    }

    // Group seeds: every connected input/output node, lowest index first.
    let mut seeds: Vec<usize> = list
        .iter()
        .copied()
        .filter(|&i| i < N_INPUTS + N_OUTPUTS)
        .filter(|i| !list2.contains(i))
        .collect();
    seeds.sort_unstable();

    // Grow one group at a time by following affinities from input to output and back.
    let mut groups: Vec<NodeGroup> = Vec::new();
    while let Some(&seed_index) = seeds.first() {
        let mut group = NodeGroup::new();
        // `list7` / `list8`: inputs / outputs whose affinities still have to be followed.
        let mut pending_inputs: Vec<NodeId> = Vec::new();
        let mut pending_outputs: Vec<NodeId> = Vec::new();
        let seed = lookup(&dictionary, seed_index)?;
        if seed_index < N_INPUTS {
            pending_inputs.push(seed);
            group.inputs.push(seed);
        } else {
            if seed_index >= N_INPUTS + N_OUTPUTS {
                break;
            }
            pending_outputs.push(seed);
            group.outputs.push(seed);
        }
        seeds.remove(0);
        while !pending_inputs.is_empty() || !pending_outputs.is_empty() {
            for &input in &pending_inputs {
                for &affinity_id in &nodes[input].affinities {
                    if group.input_output_affinities.contains(&affinity_id) {
                        continue;
                    }
                    group.input_output_affinities.push(affinity_id);
                    let Some(output) = affinities[affinity_id].node_out else {
                        continue;
                    };
                    if !group.outputs.contains(&output) {
                        group.outputs.push(output);
                        pending_outputs.push(output);
                        remove_first(&mut seeds, &nodes[output].index());
                    }
                }
            }
            pending_inputs.clear();
            for &output in &pending_outputs {
                for &affinity_id in &nodes[output].affinities {
                    if group.input_output_affinities.contains(&affinity_id) {
                        continue;
                    }
                    group.input_output_affinities.push(affinity_id);
                    let Some(input) = affinities[affinity_id].node_in else {
                        continue;
                    };
                    if !group.inputs.contains(&input) {
                        group.inputs.push(input);
                        pending_inputs.push(input);
                        remove_first(&mut seeds, &nodes[input].index());
                    }
                }
            }
            pending_outputs.clear();
        }
        // `foreach (item7 in nodeGroup.hidden) list6.Remove(..)`: `hidden` is still empty here
        // (it is filled by `FinalizeInternalStructure`), so this is a no-op in the C# too.
        for &hidden in &group.hidden {
            remove_first(&mut seeds, &nodes[hidden].index());
        }
        groups.push(group);
    }

    // Hidden nodes that no group's affinities mention.
    let grouped_hidden: HashSet<usize> = groups
        .iter()
        .flat_map(|g| g.input_output_affinities.iter())
        .flat_map(|&a| affinities[a].hidden_nodes.iter())
        .map(|&h| nodes[h].index())
        .collect();
    let non_grouped_hidden: Vec<usize> = list
        .iter()
        .copied()
        .filter(|&i| i >= N_INPUTS + N_OUTPUTS)
        .filter(|i| !grouped_hidden.contains(i))
        .collect();
    let mut still_ungrouped = non_grouped_hidden.clone();
    // Attach them through the blocked paths that passed through them.
    for &path_id in &blocked_paths {
        let path = &paths[path_id];
        let path_ungrouped: Vec<NodeId> = path
            .visited_nodes
            .iter()
            .filter(|h| non_grouped_hidden.contains(&nodes[h.node].index()))
            .map(|h| h.node)
            .collect();
        if path_ungrouped.is_empty() {
            continue;
        }
        let mut target = groups.iter().position(|g| {
            if path.toward_outputs {
                g.inputs.contains(&path.source_node)
            } else {
                g.outputs.contains(&path.source_node)
            }
        });
        if target.is_none() {
            target = groups.iter().position(|g| {
                g.input_output_affinities.iter().any(|&a| {
                    affinities[a]
                        .hidden_nodes
                        .iter()
                        .any(|h| path_ungrouped.contains(h))
                })
            });
            if target.is_none() {
                groups.push(NodeGroup::new());
                target = Some(groups.len() - 1);
            }
        }
        let group = &mut groups[target?];
        let affinity_id = InputOutputAffinity::create(path_id, &paths, &mut nodes, &mut affinities);
        group.input_output_affinities.push(affinity_id);
        if path.toward_outputs && !group.inputs.contains(&path.source_node) {
            group.inputs.push(path.source_node);
        } else if !path.toward_outputs && !group.outputs.contains(&path.source_node) {
            group.outputs.push(path.source_node);
        }
        for &hidden in &affinities[affinity_id].hidden_nodes {
            remove_first(&mut still_ungrouped, &nodes[hidden].index());
        }
        if still_ungrouped.is_empty() {
            break;
        }
    }

    // Outputs with no synapse get groups of their own: one, or two interleaved when there are
    // more than five.
    if list2.len() > 5 {
        let mut first = NodeGroup {
            is_inputless_outputs_group: true,
            ..NodeGroup::new()
        };
        let mut second = NodeGroup {
            is_inputless_outputs_group: true,
            ..NodeGroup::new()
        };
        for (i, &index) in list2.iter().enumerate() {
            let output = lookup(&dictionary, index)?;
            if i % 2 == 0 {
                first.outputs.push(output);
            } else {
                second.outputs.push(output);
            }
        }
        groups.push(first);
        groups.push(second);
    } else if !list2.is_empty() {
        let mut only = NodeGroup {
            is_inputless_outputs_group: true,
            ..NodeGroup::new()
        };
        for &index in &list2 {
            only.outputs.push(lookup(&dictionary, index)?);
        }
        groups.push(only);
    }

    for group in &mut groups {
        group.finalize_internal_structure(&mut nodes, &affinities);
    }

    Some(Layout {
        tuning: Tuning::default(),
        nodes,
        connections,
        affinities,
        paths,
        groups,
        node_ids: dictionary,
    })
}

impl Layout {
    /// The arena id of the node with this index (the C# `dictionary[index]`), if it takes part
    /// in the diagram. For a real brain the index is the node's `Index`.
    pub fn node_id_by_index(&self, index: usize) -> Option<NodeId> {
        lookup(&self.node_ids, index)
    }

    /// `BrainSorter.UsedIndicesOfGroups(groups)`: the distinct `Index` of every node of every
    /// group, group by group, in each group's `nodes` order.
    pub fn used_indices_of_groups(&self) -> Vec<usize> {
        let mut list: Vec<usize> = Vec::new();
        for group in &self.groups {
            for &node in &group.nodes {
                let index = self.nodes[node].index();
                if !list.contains(&index) {
                    list.push(index);
                }
            }
        }
        list
    }

    /// `NodeGroup.SetStartEnd(start, height)` for `groups[group_index]` (does nothing when the
    /// index is out of range).
    pub fn set_start_end(&mut self, group_index: usize, start: f32, height: f32) {
        if let Some(group) = self.groups.get_mut(group_index) {
            group.set_start_end(start, height);
        }
    }

    /// `NodeGroup.maxDepth`: the largest `totalDepth` among the group's `paths`, or 0 when it
    /// has none (which is always the case, see [`NodeGroup::paths`]). 0 for an invalid index.
    pub fn group_max_depth(&self, group_index: usize) -> i32 {
        let Some(group) = self.groups.get(group_index) else {
            return 0;
        };
        group
            .paths
            .iter()
            .filter_map(|&p| self.paths.get(p))
            .map(|p| p.total_depth)
            .max()
            .unwrap_or(0)
    }

    /// The group indices in the order of the consumer's `nodeGroups.OrderByDescending(g => g.weight)`:
    /// heaviest first, ties keeping their original order (LINQ's sort is stable).
    pub fn groups_by_descending_weight(&self) -> Vec<usize> {
        let weights: Vec<f32> = self.groups.iter().map(NodeGroup::weight).collect();
        let mut order: Vec<usize> = (0..self.groups.len()).collect();
        order.sort_by(|&a, &b| compare_f32(weights[b], weights[a]));
        order
    }

    /// `BrainSorter.Sort(nodeGroups, iteration, nodesToIgnore)`.
    ///
    /// Runs `max(iterations, 1)` ordering passes over every group's inputs and outputs, then
    /// places the hidden nodes: an initial weighted guess from their affinities with some
    /// random vertical jitter, followed by 20 rounds of [`Layout::optimize_hidden_placement`].
    ///
    /// * `nodes_to_ignore` is accepted for signature parity and ignored: the C# evaluates and
    ///   discards it (and does not forward it when it recurses).
    /// * `rng` replaces `UnityEngine.Random`; the order and number of draws is the C# one, the
    ///   numbers themselves are not (see the crate docs).
    pub fn sort(&mut self, iterations: usize, nodes_to_ignore: Option<&[usize]>, rng: &mut Rng) {
        let _ = nodes_to_ignore;
        for _ in 0..iterations.max(1) {
            self.sort_pass();
        }
        for group_index in 0..self.groups.len() {
            self.place_hidden_nodes(group_index, rng);
        }
    }

    /// One recursion level of `Sort` before the hidden placement: for every group, order the
    /// inputs and then the outputs by their accumulated `tempPos.y`.
    fn sort_pass(&mut self) {
        let Layout {
            nodes,
            affinities,
            groups,
            ..
        } = self;
        for group in groups.iter() {
            order_and_place(nodes, affinities, &group.inputs);
            order_and_place(nodes, affinities, &group.outputs);
        }
    }

    /// The hidden-node half of `Sort` for one group: the initial guess, then 20 relaxation
    /// rounds with a shrinking step and a vanishing connection push.
    fn place_hidden_nodes(&mut self, group_index: usize, rng: &mut Rng) {
        self.initial_hidden_placement(group_index, rng);
        for round in 0..HIDDEN_OPTIMIZATION_ROUNDS {
            self.optimize_hidden_placement(
                group_index,
                math::pow(0.6, round as f32 / 2.0 + 0.5),
                math::pow(0.5, (2 * (HIDDEN_OPTIMIZATION_ROUNDS - round)) as f32),
            );
        }
    }

    /// The first loop of the hidden stage of `Sort`: every hidden node of the group starts at
    /// the mean of its partners' positions (the keys of its `hiddenAffinitiesToInputsOutputs`,
    /// in insertion order), weighted by `(closeness, sqrt(1 + affinity))` per axis, each partner
    /// jittered vertically by `initialPlacementTemp * Random.Range(-1, 1) / (outputs + 4)`. A
    /// node without any partner goes to a random point of the disc of radius 0.125 around
    /// `(0.5, 0.5)`. One random draw per partner, or one disc point, in that order.
    fn initial_hidden_placement(&mut self, group_index: usize, rng: &mut Rng) {
        let Some(group) = self.groups.get(group_index) else {
            return;
        };
        let hidden = group.hidden.clone();
        let output_count = group.outputs.len() as i32;
        for &node in &hidden {
            let mut val = Vec2::ZERO;
            self.nodes[node].group_pos = Vec2::ZERO;
            let entries: Vec<(NodeId, HiddenNodeAffinity)> =
                self.nodes[node].hidden_affinities_to_inputs_outputs.clone();
            for (key, affinity) in entries {
                let val2 = Vec2::new(affinity.closeness, math::sqrt(1.0 + affinity.affinity));
                let jitter = rng.range(-1.0, 1.0) / (output_count + 4) as f32;
                let guess = (self.nodes[key].group_pos
                    + self.tuning.initial_placement_temp * Vec2::new(0.0, jitter))
                    * val2;
                self.nodes[node].group_pos += guess;
                val += val2;
            }
            if val.sqr_magnitude() > 0.0 {
                self.nodes[node].group_pos /= val;
            } else {
                self.nodes[node].group_pos = (Vec2::ONE + rng.inside_unit_circle() * 0.25) / 2.0;
            }
        }
    }

    /// `BrainSorter.OptimizeHiddenPlacement(group, dT, connectionsPushingScalar)` for
    /// `groups[group_index]` (the C# defaults are `dT = 1`, `connectionsPushingScalar = 1`).
    ///
    /// One physics-like relaxation step for the group's hidden nodes: connections pull their
    /// endpoints together and push the other hidden nodes sideways, hidden nodes repel each
    /// other, then the result is rescaled to fit the box `[0.15, 0.85] x [0.05, 0.95]`.
    /// Does nothing when the group has no hidden nodes or the index is out of range.
    pub fn optimize_hidden_placement(
        &mut self,
        group_index: usize,
        dt: f32,
        connections_pushing_scalar: f32,
    ) {
        let Layout {
            tuning,
            nodes,
            connections,
            groups,
            ..
        } = self;
        if let Some(group) = groups.get(group_index) {
            optimize_group(
                group,
                nodes,
                connections,
                tuning,
                dt,
                connections_pushing_scalar,
            );
        }
    }
}

/// `nodeGroup.inputs.OrderBy(n => n.tempPos.y)`: the ids in ascending `tempPos.y`, ties (and
/// NaNs, which compare equal and smallest) keeping their original order.
fn ordered_by_temp_pos_y(nodes: &[NodeSortHelper], ids: &[NodeId]) -> Vec<NodeId> {
    let mut ordered = ids.to_vec();
    ordered.sort_by(|&a, &b| compare_f32(nodes[a].temp_pos.y, nodes[b].temp_pos.y));
    ordered
}

/// The body of the per-group loop of `Sort` for one list (`inputs`, or `outputs`): update every
/// node's `tempPos` with `ComputeLocalMinima`, then hand out evenly spaced `groupPos.y` slots in
/// `tempPos.y` order.
fn order_and_place(
    nodes: &mut [NodeSortHelper],
    affinities: &[InputOutputAffinity],
    list: &[NodeId],
) {
    let count = list.len();
    let step = 1.0 / (count as i64 - 1) as f32;
    for &node in list {
        compute_local_minima(nodes, affinities, node, list);
    }
    let mut slot = 0.0f32;
    for node in ordered_by_temp_pos_y(nodes, list) {
        nodes[node].group_pos.y = if count > 1 { slot } else { 0.5 };
        slot += step;
    }
}

/// `BrainSorter.ComputeLocalMinima(node, sameNodes)`: nudges `node.tempPos.y` towards the
/// affinity-weighted mean position of its partners while the nodes of the same list push each
/// other apart.
fn compute_local_minima(
    nodes: &mut [NodeSortHelper],
    affinities: &[InputOutputAffinity],
    node: NodeId,
    same_nodes: &[NodeId],
) {
    let archetype = nodes[node].archetype();
    if archetype == Archetype::Output && nodes[node].affinities.is_empty() {
        return;
    }
    let y = nodes[node].group_pos.y;
    let mut num = 0.0f32;
    let mut num2 = 0.0f32;
    for &affinity_id in &nodes[node].affinities {
        let affinity = &affinities[affinity_id];
        let partner = if archetype == Archetype::Input {
            affinity.node_out
        } else {
            affinity.node_in
        };
        if let Some(partner) = partner {
            num += 2.0 * affinity.affinity * (nodes[partner].group_pos.y - y);
            num2 += 2.0 * affinity.affinity;
        }
    }
    // `num3` (the summed affinities) is accumulated and divided by `sameNodes.Count - 1` in the
    // C#, but the quotient is discarded, so it has no effect and is not computed here.
    let num4 = same_nodes.len() as i32 + 1;
    let neg_num4 = num4.wrapping_neg() as f32;
    let num4_squared = num4.wrapping_mul(num4) as f32;
    for &same in same_nodes {
        if same != node {
            let num5 = nodes[same].group_pos.y - y;
            num -= num4 as f32 * math::sign(num5) * math::exp(neg_num4 * math::abs(num5));
            num2 += num4_squared * math::exp(neg_num4 * math::abs(num5));
        }
    }
    nodes[node].temp_pos.y += num / num2;
}

/// The body of `BrainSorter.OptimizeHiddenPlacement` for one group.
fn optimize_group(
    group: &NodeGroup,
    nodes: &mut [NodeSortHelper],
    connections: &[ConnectionSortHelper],
    tuning: &Tuning,
    dt: f32,
    connections_pushing_scalar: f32,
) {
    if group.hidden.is_empty() {
        return;
    }
    for &hidden in &group.hidden {
        let node = &mut nodes[hidden];
        node.temp_pos = node.group_pos;
        node.temp_vel = Vec2::ZERO;
        node.temp_acc = Vec2::ZERO;
    }
    let num = connections_pushing_scalar / group.connections.len() as f32;
    let mut num2 = 0.0f32;
    for &id in &group.connections {
        let connection = &connections[id];
        if nodes[connection.node_in].archetype() == Archetype::Hidden
            || nodes[connection.node_out].archetype() == Archetype::Hidden
        {
            num2 += math::sqrt(math::min(math::abs(connection.value()), 10.0) + 2.5)
                * (if connection.enabled() {
                    1.0
                } else {
                    tuning.disabled_factor
                });
        }
    }
    // `int num3 = Mathf.Max(group.inputs.Count, group.outputs.Count)`, used as `(float)(-num3)`
    // and `(float)(num3 * num3)`.
    let num3 = group.inputs.len().max(group.outputs.len()) as i32;
    let neg_num3 = num3.wrapping_neg() as f32;
    let num3_squared = num3.wrapping_mul(num3) as f32;
    for &id in &group.connections {
        let connection = &connections[id];
        let (node_in, node_out) = (connection.node_in, connection.node_out);
        let num4 = tuning.enabled_factor
            * math::sqrt(math::abs(connection.value()) + 2.5)
            * (if connection.enabled() {
                1.0
            } else {
                tuning.disabled_factor
            })
            / num2;
        let val2 = nodes[node_out].group_pos - nodes[node_in].group_pos;
        let normalized = val2.rotate(std::f32::consts::PI / 2.0).normalized();
        if nodes[node_in].archetype() == Archetype::Hidden {
            nodes[node_in].temp_vel += (0.0 - num4) * val2;
            nodes[node_in].temp_acc += num4 * val2.normalized();
        }
        if nodes[node_out].archetype() == Archetype::Hidden {
            nodes[node_out].temp_vel -= (0.0 - num4) * val2;
            nodes[node_out].temp_acc -= num4 * val2.normalized();
        }
        for &other in &group.hidden {
            if other != node_in && other != node_out {
                let val3 = nodes[other].group_pos
                    - (nodes[node_in].group_pos + nodes[node_out].group_pos) / 2.0;
                let num5 = 2.0
                    * math::abs(Vec2::dot(val3, val2.normalized()) / (val2.magnitude() + 0.0001));
                let num6 = Vec2::dot(val3, normalized) / val3.magnitude();
                let num7 = math::abs(num6) * math::max(0.5, 2.0 * num5 - 0.5);
                let num8 = neg_num3 * math::exp(neg_num3 * num7);
                let num9 = num3_squared * math::exp(neg_num3 * num7);
                let push = math::sign(num6) * tuning.connection_push * num;
                nodes[other].temp_vel += normalized * (push * num8);
                nodes[other].temp_acc += normalized * (push * num9);
            }
        }
    }
    if group.hidden.len() > 1 {
        let hidden_count = group.hidden.len() as i32;
        let num10 = 1.0 / hidden_count.wrapping_mul(hidden_count).wrapping_sub(1) as f32;
        let mut remaining = group.nodes.clone();
        for &node in &group.nodes {
            remove_first(&mut remaining, &node);
            for &other in &remaining {
                let val4 = nodes[other].group_pos - nodes[node].group_pos;
                let magnitude = val4.magnitude();
                let num11 = neg_num3 * math::exp(neg_num3 * magnitude);
                let num12 = num3_squared * math::exp(neg_num3 * magnitude);
                let node_is_hidden = nodes[node].archetype() == Archetype::Hidden;
                let other_is_hidden = nodes[other].archetype() == Archetype::Hidden;
                if node_is_hidden {
                    let factor = if other_is_hidden {
                        num10
                    } else {
                        tuning.non_hidden_repulsion_factor
                    };
                    nodes[node].temp_vel -= val4.normalized() * (num11 * factor);
                    nodes[node].temp_acc -= val4.normalized() * (num12 * factor);
                }
                if other_is_hidden {
                    let factor = if node_is_hidden {
                        num10
                    } else {
                        tuning.non_hidden_repulsion_factor
                    };
                    nodes[other].temp_vel += val4.normalized() * (num11 * factor);
                    nodes[other].temp_acc += val4.normalized() * (num12 * factor);
                }
            }
        }
    }
    let low = Vec2::new(0.15, 0.05);
    let high = Vec2::new(0.85, 0.95);
    let mut min = low;
    let mut max = high;
    for &hidden in &group.hidden {
        let node = &mut nodes[hidden];
        node.temp_pos -= dt * node.temp_vel / node.temp_acc.magnitude();
        min = Vec2::new(
            math::min(min.x, node.temp_pos.x),
            math::min(min.y, node.temp_pos.y),
        );
        max = Vec2::new(
            math::max(max.x, node.temp_pos.x),
            math::max(max.y, node.temp_pos.y),
        );
    }
    for &hidden in &group.hidden {
        let node = &mut nodes[hidden];
        node.group_pos = (node.temp_pos - min) / (max - min) * (high - low) + low;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_nodes(n_hidden: usize) -> Vec<SortNode> {
        (0..N_INPUTS + N_OUTPUTS + n_hidden)
            .map(|i| SortNode::new(i, i as i64 + 1, false, 0.0))
            .collect()
    }

    fn syn(a: usize, b: usize, w: f32) -> SortSynapse {
        SortSynapse::new(0, a, b, w, true)
    }

    /// `OrderBy(n => n.tempPos.y)`: ascending, stable, and NaN keys sort first (as equals).
    #[test]
    fn ordering_follows_linq_order_by_on_floats() {
        let mut nodes: Vec<NodeSortHelper> = (0..6)
            .map(|i| NodeSortHelper::new(SortNode::new(i, i as i64, false, 0.0)))
            .collect();
        let keys = [f32::NAN, 3.0, f32::NAN, -1.0, 3.0, -0.0];
        for (node, key) in nodes.iter_mut().zip(keys) {
            node.temp_pos.y = key;
        }
        let ids = [0usize, 1, 2, 3, 4, 5];
        assert_eq!(ordered_by_temp_pos_y(&nodes, &ids), vec![0, 2, 3, 5, 1, 4]);
        // a sub-list keeps the order it was given for equal keys
        assert_eq!(
            ordered_by_temp_pos_y(&nodes, &[4, 1, 2, 0]),
            vec![2, 0, 4, 1]
        );
        // +0 and -0 are equal
        nodes[0].temp_pos.y = 0.0;
        nodes[5].temp_pos.y = -0.0;
        assert_eq!(ordered_by_temp_pos_y(&nodes, &[5, 0]), vec![5, 0]);
        assert_eq!(ordered_by_temp_pos_y(&nodes, &[0, 5]), vec![0, 5]);
    }

    /// I(0, .5) -(2)-> H -(3)-> O(1, .5): H has two partners, inserted source first, both with
    /// closeness 2 and affinities 2 and 3, so the weights are (2, sqrt(3)) and (2, sqrt(4)).
    /// Draws: one per partner, in that order, each scaled by `initialPlacementTemp / (outputs + 4)`.
    ///   x = (0 * 2 + 1 * 2) / (2 + 2) = 0.5
    ///   y = ((.5 + .1 r1 / 5) * sqrt(3) + (.5 + .1 r2 / 5) * 2) / (sqrt(3) + 2)
    #[test]
    fn initial_placement_is_the_weighted_mean_of_the_jittered_partners() {
        let mut layout = make_groups(&base_nodes(1), &[syn(0, 48, 2.0), syn(48, 33, 3.0)]);
        let input = layout.node_id_by_index(0);
        let output = layout.node_id_by_index(33);
        let hidden = layout.node_id_by_index(48);
        let (Some(input), Some(output), Some(hidden)) = (input, output, hidden) else {
            panic!("nodes missing");
        };
        layout.nodes[input].group_pos = Vec2::new(0.0, 0.5);
        layout.nodes[output].group_pos = Vec2::new(1.0, 0.5);
        layout.initial_hidden_placement(0, &mut Rng::new(5));

        let mut replay = Rng::new(5);
        let r1 = f64::from(replay.range(-1.0, 1.0));
        let r2 = f64::from(replay.range(-1.0, 1.0));
        let (w1, w2) = (3.0f64.sqrt(), 4.0f64.sqrt());
        let expected_y = ((0.5 + 0.1 * r1 / 5.0) * w1 + (0.5 + 0.1 * r2 / 5.0) * w2) / (w1 + w2);
        let p = layout.nodes[hidden].group_pos;
        assert!((f64::from(p.x) - 0.5).abs() < 1e-6, "{p:?}");
        assert!(
            (f64::from(p.y) - expected_y).abs() < 1e-5,
            "{} vs {expected_y}",
            p.y
        );
    }

    /// Hidden nodes that no finished path reached have no partners: each gets
    /// `(one + insideUnitCircle * 0.25) / 2`, consuming one disc point apiece, in group order.
    #[test]
    fn partnerless_hidden_nodes_get_random_points_near_the_middle() {
        let syns = [
            syn(0, 33, 1.0),
            syn(0, 48, 2.0),
            syn(48, 49, 3.0),
            syn(49, 48, 4.0),
        ];
        let mut layout = make_groups(&base_nodes(2), &syns);
        assert_eq!(layout.groups[0].hidden.len(), 2);
        layout.initial_hidden_placement(0, &mut Rng::new(77));
        let mut replay = Rng::new(77);
        for &hidden in &layout.groups[0].hidden {
            let expected = (Vec2::ONE + replay.inside_unit_circle() * 0.25) / 2.0;
            assert_eq!(layout.nodes[hidden].group_pos, expected);
            let p = layout.nodes[hidden].group_pos;
            assert!((0.375..=0.625).contains(&p.x) && (0.375..=0.625).contains(&p.y));
        }
    }
}

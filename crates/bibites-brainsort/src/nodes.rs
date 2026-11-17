//! `NodeSortHelper`, `ConnectionSortHelper`, `HiddenNodeAffinity` and `HiddenDepthInPath`.

use crate::math;
use crate::types::{AffinityId, Archetype, ConnId, NodeId, SortNode, SortSynapse};
use crate::vec2::Vec2;

/// `HiddenNodeAffinity`: how strongly, and how closely, one hidden node is tied to one
/// input/output node (the key of [`NodeSortHelper::hidden_affinities_to_inputs_outputs`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HiddenNodeAffinity {
    /// `HiddenNodeAffinity.affinity`: running mean of the affinities of all paths seen so far.
    pub affinity: f32,
    /// `HiddenNodeAffinity.closeness`: the smallest closeness of any path seen so far.
    pub closeness: f32,
    /// `HiddenNodeAffinity.n`: how many paths have been averaged into `affinity` (starts at 1).
    pub n: i32,
}

impl HiddenNodeAffinity {
    /// `new HiddenNodeAffinity(affinity, closeness)`.
    pub fn new(affinity: f32, closeness: f32) -> Self {
        HiddenNodeAffinity {
            affinity,
            closeness,
            n: 1,
        }
    }

    /// `HiddenNodeAffinity.AddAffinity(affinityToAdd, newCloseness)`.
    pub fn add_affinity(&mut self, affinity_to_add: f32, new_closeness: f32) {
        self.affinity = (self.n as f32 * self.affinity + affinity_to_add) / (self.n + 1) as f32;
        self.closeness = math::min(self.closeness, new_closeness);
        self.n += 1;
    }
}

/// `HiddenDepthInPath`: a hidden node a path went through, with the path's depth and
/// accumulated affinity *at the moment the node was reached*.
///
/// The C# struct also stores a reference to its owning path. That reference always equals the
/// path whose `visitedNodes` list holds the entry (the copy constructor of `InputOutputPath`
/// re-points it), so the port omits it and passes the owning path's fields to
/// [`NodeSortHelper::add_hidden_affinities_of_path`] instead.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HiddenDepthInPath {
    /// `HiddenDepthInPath.node`.
    pub node: NodeId,
    /// `HiddenDepthInPath.depth`: the path's `totalDepth` when the node was reached.
    pub depth: i32,
    /// `HiddenDepthInPath.accumulatedAffinity`: the path's `accumulatedAffinity` when the node
    /// was reached.
    pub accumulated_affinity: f32,
}

impl HiddenDepthInPath {
    /// `new HiddenDepthInPath(path, node)` with `depth = path.totalDepth` and
    /// `accumulatedAffinity = path.accumulatedAffinity` passed explicitly.
    pub fn new(node: NodeId, depth: i32, accumulated_affinity: f32) -> Self {
        HiddenDepthInPath {
            node,
            depth,
            accumulated_affinity,
        }
    }
}

/// `ConnectionSortHelper`: one synapse plus the two [`NodeSortHelper`]s it joins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConnectionSortHelper {
    /// `ConnectionSortHelper.connection`.
    pub connection: SortSynapse,
    /// `ConnectionSortHelper.nodeIn`.
    pub node_in: NodeId,
    /// `ConnectionSortHelper.nodeOut`.
    pub node_out: NodeId,
}

impl ConnectionSortHelper {
    /// `new ConnectionSortHelper(conn, from, to)`.
    pub fn new(conn: SortSynapse, from: NodeId, to: NodeId) -> Self {
        ConnectionSortHelper {
            connection: conn,
            node_in: from,
            node_out: to,
        }
    }

    /// `ConnectionSortHelper.indexIn` (`connection.NodeIn`).
    pub fn index_in(&self) -> usize {
        self.connection.node_in
    }

    /// `ConnectionSortHelper.indexOut` (`connection.NodeOut`).
    pub fn index_out(&self) -> usize {
        self.connection.node_out
    }

    /// `ConnectionSortHelper.value` (`connection.Weight`).
    pub fn value(&self) -> f32 {
        self.connection.weight
    }

    /// `ConnectionSortHelper.enabled` (`connection.En`).
    pub fn enabled(&self) -> bool {
        self.connection.enabled
    }
}

/// `NodeSortHelper`: the sorter's working record for one brain node.
///
/// The C# `Dictionary<NodeSortHelper, HiddenNodeAffinity>` is kept as an insertion-ordered
/// `Vec` of pairs: .NET enumerates a dictionary without removals in insertion order, and the
/// initial hidden-node placement in `Sort` is sensitive to that order (float accumulation and
/// the order of random draws).
#[derive(Clone, Debug)]
pub struct NodeSortHelper {
    /// `NodeSortHelper.node`.
    pub node: SortNode,
    /// `NodeSortHelper.groupPos`: position inside its group, both coordinates in `[0, 1]`
    /// (x = 0 for inputs, 1 for outputs; y is the vertical slot).
    pub group_pos: Vec2,
    /// `NodeSortHelper.panelPos`: written by the consumer when it scales groups onto the panel.
    pub panel_pos: Vec2,
    /// `NodeSortHelper.tempPos`: accumulated ordering key (inputs/outputs) or scratch position
    /// (hidden nodes).
    pub temp_pos: Vec2,
    /// `NodeSortHelper.tempVel`.
    pub temp_vel: Vec2,
    /// `NodeSortHelper.tempAcc`.
    pub temp_acc: Vec2,
    /// `NodeSortHelper.inputConnections`, in synapse order.
    pub input_connections: Vec<ConnId>,
    /// `NodeSortHelper.outputConnections`, in synapse order.
    pub output_connections: Vec<ConnId>,
    /// `NodeSortHelper.affinities`, in creation order.
    pub affinities: Vec<AffinityId>,
    /// `NodeSortHelper.hiddenAffinitiesToInputsOutputs`, in insertion order.
    pub hidden_affinities_to_inputs_outputs: Vec<(NodeId, HiddenNodeAffinity)>,
}

impl NodeSortHelper {
    /// `new NodeSortHelper(refNode)`: `groupPos` starts at `(1, 0)` for outputs, `(0, 0)` otherwise.
    pub fn new(ref_node: SortNode) -> Self {
        let x = if ref_node.archetype == Archetype::Output {
            1.0
        } else {
            0.0
        };
        NodeSortHelper {
            node: ref_node,
            group_pos: Vec2::new(x, 0.0),
            panel_pos: Vec2::ZERO,
            temp_pos: Vec2::ZERO,
            temp_vel: Vec2::ZERO,
            temp_acc: Vec2::ZERO,
            input_connections: Vec::new(),
            output_connections: Vec::new(),
            affinities: Vec::new(),
            hidden_affinities_to_inputs_outputs: Vec::new(),
        }
    }

    /// `NodeSortHelper.index` (`node.Index`).
    pub fn index(&self) -> usize {
        self.node.index
    }

    /// `NodeSortHelper.inov` (`node.Inov`).
    pub fn inov(&self) -> i64 {
        self.node.inov
    }

    /// `NodeSortHelper.archetype` (`node.archetype`).
    pub fn archetype(&self) -> Archetype {
        self.node.archetype
    }

    /// `NodeSortHelper.connections`: `inputConnections.Union(outputConnections)`, i.e. the
    /// distinct connections, input side first.
    pub fn connections(&self) -> Vec<ConnId> {
        let mut union: Vec<ConnId> = Vec::new();
        for &c in self
            .input_connections
            .iter()
            .chain(&self.output_connections)
        {
            if !union.contains(&c) {
                union.push(c);
            }
        }
        union
    }

    /// Looks a key up in `hiddenAffinitiesToInputsOutputs` (`TryGetValue`).
    pub fn hidden_affinity(&self, key: NodeId) -> Option<&HiddenNodeAffinity> {
        self.hidden_affinities_to_inputs_outputs
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v)
    }

    /// `hiddenAffinitiesToInputsOutputs.TryGetValue(key) ? value.AddAffinity(..) : Add(key, new ..)`.
    fn upsert_hidden_affinity(&mut self, key: NodeId, affinity: f32, closeness: f32) {
        match self
            .hidden_affinities_to_inputs_outputs
            .iter_mut()
            .find(|(k, _)| *k == key)
        {
            Some((_, value)) => value.add_affinity(affinity, closeness),
            None => self
                .hidden_affinities_to_inputs_outputs
                .push((key, HiddenNodeAffinity::new(affinity, closeness))),
        }
    }

    /// `NodeSortHelper.AddHiddenAffinitiesOfPath(HiddenDepthInPath depth, int totalDepth)`,
    /// called on the hidden node `depth.node` when a path finishes.
    ///
    /// `depth.path.sourceNode`, `depth.path.nextNode` and `depth.path.accumulatedAffinity` of
    /// the C# are the owning path's `source_node`, `next_node` and `accumulated_affinity`
    /// (see [`HiddenDepthInPath`]).
    pub fn add_hidden_affinities_of_path(
        &mut self,
        depth: &HiddenDepthInPath,
        source_node: NodeId,
        next_node: NodeId,
        path_accumulated_affinity: f32,
        total_depth: i32,
    ) {
        let accumulated_affinity = depth.accumulated_affinity;
        let num = path_accumulated_affinity / accumulated_affinity;
        let num2 = depth.depth as f32;
        let total = total_depth as f32;
        self.upsert_hidden_affinity(source_node, accumulated_affinity, total / num2);
        self.upsert_hidden_affinity(next_node, num, total / (total - num2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hidden_helper() -> NodeSortHelper {
        NodeSortHelper::new(SortNode::new(48, 100, false, 0.0))
    }

    #[test]
    fn new_helper_starts_at_the_column_of_its_archetype() {
        assert_eq!(
            NodeSortHelper::new(SortNode::new(3, 4, false, 0.0)).group_pos,
            Vec2::new(0.0, 0.0)
        );
        assert_eq!(
            NodeSortHelper::new(SortNode::new(40, 41, true, 0.0)).group_pos,
            Vec2::new(1.0, 0.0)
        );
        assert_eq!(hidden_helper().group_pos, Vec2::new(0.0, 0.0));
        assert_eq!(hidden_helper().inov(), 100);
        assert_eq!(hidden_helper().index(), 48);
        assert_eq!(hidden_helper().archetype(), Archetype::Hidden);
    }

    #[test]
    fn add_affinity_averages_and_keeps_the_smallest_closeness() {
        let mut a = HiddenNodeAffinity::new(2.0, 3.0);
        assert_eq!(a.n, 1);
        a.add_affinity(5.0, 4.0);
        assert_eq!(a.n, 2);
        assert_eq!(a.affinity, 3.5);
        assert_eq!(a.closeness, 3.0);
        a.add_affinity(2.0, 1.5);
        assert_eq!(a.n, 3);
        assert_eq!(a.affinity, (2.0 * 3.5 + 2.0) / 3.0);
        assert_eq!(a.closeness, 1.5);
    }

    #[test]
    fn hidden_affinities_of_a_path_use_depth_and_total_depth() {
        // Path source 7 -> hidden (depth 1, affinity 2) -> ... -> next 9, total depth 3,
        // final accumulated affinity 24.
        let mut h = hidden_helper();
        let depth = HiddenDepthInPath::new(48, 1, 2.0);
        h.add_hidden_affinities_of_path(&depth, 7, 9, 24.0, 3);
        let to_source = h.hidden_affinity(7).copied();
        let to_next = h.hidden_affinity(9).copied();
        // source: (accumulatedAffinity, totalDepth / depth) = (2, 3)
        assert_eq!(
            to_source,
            Some(HiddenNodeAffinity {
                affinity: 2.0,
                closeness: 3.0,
                n: 1
            })
        );
        // next: (pathAffinity / accumulatedAffinity, totalDepth / (totalDepth - depth)) = (12, 1.5)
        assert_eq!(
            to_next,
            Some(HiddenNodeAffinity {
                affinity: 12.0,
                closeness: 1.5,
                n: 1
            })
        );
        // Insertion order: source first, then next.
        assert_eq!(h.hidden_affinities_to_inputs_outputs[0].0, 7);
        assert_eq!(h.hidden_affinities_to_inputs_outputs[1].0, 9);
        // A second path through the same endpoints averages into the same entries.
        let depth2 = HiddenDepthInPath::new(48, 2, 6.0);
        h.add_hidden_affinities_of_path(&depth2, 7, 9, 24.0, 3);
        let to_source = h
            .hidden_affinity(7)
            .copied()
            .unwrap_or(HiddenNodeAffinity::new(0.0, 0.0));
        assert_eq!(to_source.n, 2);
        assert_eq!(to_source.affinity, (2.0 + 6.0) / 2.0);
        assert_eq!(to_source.closeness, 1.5); // min(3, 3/2)
        assert_eq!(h.hidden_affinities_to_inputs_outputs.len(), 2);
    }

    #[test]
    fn connections_property_is_a_distinct_union_input_side_first() {
        let mut n = hidden_helper();
        n.input_connections = vec![4, 2];
        n.output_connections = vec![2, 9];
        assert_eq!(n.connections(), vec![4, 2, 9]);
    }

    #[test]
    fn connection_helper_exposes_the_synapse_fields() {
        let c = ConnectionSortHelper::new(SortSynapse::new(5, 1, 40, -0.75, false), 0, 1);
        assert_eq!((c.index_in(), c.index_out()), (1, 40));
        assert_eq!(c.value(), -0.75);
        assert!(!c.enabled());
        assert_eq!((c.node_in, c.node_out), (0, 1));
    }
}

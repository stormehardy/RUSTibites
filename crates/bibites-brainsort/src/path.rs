//! `InputOutputPath` and `InputOutputAffinity`.

use crate::math;
use crate::nodes::{ConnectionSortHelper, HiddenDepthInPath, NodeSortHelper};
use crate::types::{AffinityId, Archetype, ConnId, NodeId, PathId};

/// `InputOutputPath`: one walk from an input (towards the outputs) or from an output (towards
/// the inputs) through the hidden nodes, built one synapse at a time.
///
/// Fields that are `private` in the C# (`blockedNodes`, `recursivity`) have read-only
/// accessors. Paths still being walked are owned by the walk loop in
/// [`make_groups`](crate::make_groups); only finished (blocked or done) paths are stored, in
/// [`Layout::paths`](crate::Layout::paths).
#[derive(Clone, Debug)]
pub struct InputOutputPath {
    /// `sourceNode`: the input/output node the walk started at.
    pub source_node: NodeId,
    /// `nextNode`: the node the walk has reached.
    pub next_node: NodeId,
    /// `visitedNodes`: the intermediate nodes passed so far, in visiting order.
    pub visited_nodes: Vec<HiddenDepthInPath>,
    blocked_nodes: Vec<NodeId>,
    /// `connections`: the distinct synapses walked so far, in walking order.
    pub connections: Vec<ConnId>,
    /// `accumulatedAffinity`: product of the walked synapses' weights (`|w|` if enabled,
    /// `sqrt(|w|)` if disabled).
    pub accumulated_affinity: f32,
    /// `blocked`: the walk reached a node it had already passed twice.
    pub blocked: bool,
    /// `done`: the walk reached an output (towards outputs) or an input (towards inputs).
    pub done: bool,
    /// `totalDepth`: number of distinct synapses walked.
    pub total_depth: i32,
    /// `towardOutputs`: walking from an input to the outputs (`true`) or the reverse (`false`).
    pub toward_outputs: bool,
    recursivity: i32,
}

impl InputOutputPath {
    /// `new InputOutputPath(start, directionIsTowardOutputs)`.
    pub fn new(start: NodeId, direction_is_toward_outputs: bool) -> Self {
        InputOutputPath {
            source_node: start,
            next_node: start,
            visited_nodes: Vec::new(),
            blocked_nodes: Vec::new(),
            connections: Vec::new(),
            accumulated_affinity: 1.0,
            blocked: false,
            done: false,
            total_depth: 0,
            toward_outputs: direction_is_toward_outputs,
            recursivity: 0,
        }
    }

    /// `new InputOutputPath(pathToDuplicate)`: the copy constructor (lists are copied, and
    /// `done` / `blocked` are reset to `false`).
    pub fn duplicate(&self) -> Self {
        let mut copy = self.clone();
        copy.done = false;
        copy.blocked = false;
        copy
    }

    /// `blockedNodes` (private in the C#): nodes this walk has passed twice.
    pub fn blocked_nodes(&self) -> &[NodeId] {
        &self.blocked_nodes
    }

    /// `recursivity` (private in the C#): how many times the walk re-entered a visited node.
    pub fn recursivity(&self) -> i32 {
        self.recursivity
    }

    /// `normalizedAffinity`: `Mathf.Pow(Mathf.Abs(accumulatedAffinity), 1f / (1 + recursivity))`.
    pub fn normalized_affinity(&self) -> f32 {
        math::pow(
            math::abs(self.accumulated_affinity),
            1.0 / (1 + self.recursivity) as f32,
        )
    }

    /// `AdvanceAndCheckDone(nextConnection)`: walks one more synapse. Returns `true` when the
    /// walk has ended (it is `blocked` and/or `done`).
    ///
    /// When the walk is `done`, every visited node has the walk's affinities registered on it
    /// ([`NodeSortHelper::add_hidden_affinities_of_path`]), which is why the node arena is
    /// borrowed mutably.
    pub fn advance_and_check_done(
        &mut self,
        next_connection: ConnId,
        nodes: &mut [NodeSortHelper],
        connections: &[ConnectionSortHelper],
    ) -> bool {
        let connection = &connections[next_connection];
        self.accumulated_affinity *= if connection.enabled() {
            math::abs(connection.value())
        } else {
            math::sqrt(math::abs(connection.value()))
        };
        self.next_node = if self.toward_outputs {
            connection.node_out
        } else {
            connection.node_in
        };
        if !self.connections.contains(&next_connection) {
            self.connections.push(next_connection);
            self.total_depth += 1;
        }
        self.blocked = self.blocked_nodes.contains(&self.next_node);
        // `(NodeArchetype)(towardOutputs ? 1 : 0)`: Output when walking towards the outputs,
        // Input otherwise.
        let wanted = if self.toward_outputs {
            Archetype::Output
        } else {
            Archetype::Input
        };
        self.done = nodes[self.next_node].archetype() == wanted;
        if self.done {
            for visited in &self.visited_nodes {
                nodes[visited.node].add_hidden_affinities_of_path(
                    visited,
                    self.source_node,
                    self.next_node,
                    self.accumulated_affinity,
                    self.total_depth,
                );
            }
        }
        if self.blocked || self.done {
            return true;
        }
        let node_to_add = self.next_node;
        if self.visited_nodes.iter().any(|n| n.node == node_to_add) {
            self.blocked_nodes.push(self.next_node);
            self.recursivity += 1;
        } else {
            self.visited_nodes.push(HiddenDepthInPath::new(
                self.next_node,
                self.total_depth,
                self.accumulated_affinity,
            ));
        }
        false
    }
}

/// `InputOutputAffinity`: the tie between one input and one output (found by one or more
/// paths between them), with the hidden nodes and synapses those paths used.
///
/// `node_in` / `node_out` are `None` where the C# holds `null`, which happens for the affinity
/// built from a *blocked* path (its far end was never reached).
#[derive(Clone, Debug)]
pub struct InputOutputAffinity {
    /// `affinity`: the sum of the normalised affinities of the collapsed paths.
    pub affinity: f32,
    /// `nodeIn`: the input end (`null` for a blocked path walking towards the inputs).
    pub node_in: Option<NodeId>,
    /// `nodeOut`: the output end (`null` for a blocked path walking towards the outputs).
    pub node_out: Option<NodeId>,
    /// `hiddenNodes`: distinct hidden nodes of the collapsed paths, in discovery order.
    pub hidden_nodes: Vec<NodeId>,
    /// `connections`: distinct synapses of the collapsed paths, in discovery order.
    pub connections: Vec<ConnId>,
    /// `paths`: the collapsed paths (ids into [`Layout::paths`](crate::Layout::paths)).
    pub paths: Vec<PathId>,
}

impl InputOutputAffinity {
    /// `new InputOutputAffinity(path)`: builds the affinity from `paths[path_id]`, appends it to
    /// `affinities`, registers it on its non-null end nodes (`nodeIn?.affinities.Add(this)`,
    /// `nodeOut?.affinities.Add(this)`) and returns its id.
    pub fn create(
        path_id: PathId,
        paths: &[InputOutputPath],
        nodes: &mut [NodeSortHelper],
        affinities: &mut Vec<InputOutputAffinity>,
    ) -> AffinityId {
        let path = &paths[path_id];
        let node_in = if path.toward_outputs {
            Some(path.source_node)
        } else if path.blocked {
            None
        } else {
            Some(path.next_node)
        };
        let node_out = if !path.toward_outputs {
            Some(path.source_node)
        } else if path.blocked {
            None
        } else {
            Some(path.next_node)
        };
        let id = affinities.len();
        affinities.push(InputOutputAffinity {
            affinity: path.normalized_affinity(),
            node_in,
            node_out,
            hidden_nodes: path.visited_nodes.iter().map(|n| n.node).collect(),
            connections: path.connections.clone(),
            paths: vec![path_id],
        });
        if let Some(n) = node_in {
            nodes[n].affinities.push(id);
        }
        if let Some(n) = node_out {
            nodes[n].affinities.push(id);
        }
        id
    }

    /// `CollapsePathInto(otherPath)`: merges another path with the same endpoints into this
    /// affinity (adds its affinity, and any hidden node / synapse not yet listed).
    pub fn collapse_path_into(&mut self, other_id: PathId, other: &InputOutputPath) {
        self.affinity += other.normalized_affinity();
        self.paths.push(other_id);
        for visited in &other.visited_nodes {
            if !self.hidden_nodes.contains(&visited.node) {
                self.hidden_nodes.push(visited.node);
            }
        }
        for connection in &other.connections {
            if !self.connections.contains(connection) {
                self.connections.push(*connection);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SortNode, SortSynapse};

    /// Arena for a small hand-made graph: input 0, hidden 48 and 49, output 33.
    fn arena(
        conns: &[(usize, usize, f32, bool)],
    ) -> (Vec<NodeSortHelper>, Vec<ConnectionSortHelper>) {
        let ids = [0usize, 33, 48, 49];
        let mut nodes: Vec<NodeSortHelper> = ids
            .iter()
            .map(|&i| NodeSortHelper::new(SortNode::new(i, i as i64, false, 0.0)))
            .collect();
        let pos = |idx: usize| ids.iter().position(|&i| i == idx).unwrap_or(0);
        let mut connections = Vec::new();
        for (k, &(a, b, w, en)) in conns.iter().enumerate() {
            let (na, nb) = (pos(a), pos(b));
            connections.push(ConnectionSortHelper::new(
                SortSynapse::new(k as i64, a, b, w, en),
                na,
                nb,
            ));
            nodes[na].output_connections.push(k);
            nodes[nb].input_connections.push(k);
        }
        (nodes, connections)
    }

    #[test]
    fn enabled_weights_multiply_and_disabled_ones_use_the_square_root() {
        let (mut nodes, conns) = arena(&[(0, 33, -4.0, true), (0, 33, -4.0, false)]);
        let mut p = InputOutputPath::new(0, true);
        assert!(p.advance_and_check_done(0, &mut nodes, &conns));
        assert_eq!(p.accumulated_affinity, 4.0);
        assert!(p.done && !p.blocked);
        assert_eq!(p.next_node, 1);
        assert_eq!(p.total_depth, 1);
        let mut q = InputOutputPath::new(0, true);
        assert!(q.advance_and_check_done(1, &mut nodes, &conns));
        assert_eq!(q.accumulated_affinity, 2.0);
    }

    #[test]
    fn walking_a_chain_records_hidden_nodes_with_their_depth() {
        // 0 -> 48 (w 2) -> 49 (w 3) -> 33 (w 4)
        let (mut nodes, conns) =
            arena(&[(0, 48, 2.0, true), (48, 49, 3.0, true), (49, 33, 4.0, true)]);
        let mut p = InputOutputPath::new(0, true);
        assert!(!p.advance_and_check_done(0, &mut nodes, &conns));
        assert!(!p.advance_and_check_done(1, &mut nodes, &conns));
        assert_eq!(
            p.visited_nodes,
            vec![
                HiddenDepthInPath::new(2, 1, 2.0),
                HiddenDepthInPath::new(3, 2, 6.0)
            ]
        );
        assert!(p.advance_and_check_done(2, &mut nodes, &conns));
        assert_eq!(p.accumulated_affinity, 24.0);
        assert_eq!(p.total_depth, 3);
        assert_eq!(p.normalized_affinity(), 24.0);
        // The finished walk registered itself on both hidden nodes (see the hand trace in
        // `tests::two_hidden_nodes_in_a_chain_get_depth_based_closeness`).
        assert_eq!(
            nodes[2]
                .hidden_affinity(0)
                .map(|a| (a.affinity, a.closeness)),
            Some((2.0, 3.0))
        );
        assert_eq!(
            nodes[3]
                .hidden_affinity(1)
                .map(|a| (a.affinity, a.closeness)),
            Some((4.0, 3.0))
        );
    }

    #[test]
    fn a_node_entered_a_second_time_is_blocked_and_the_third_time_ends_the_walk() {
        // 0 -> 48, 48 -> 48 (self loop), 48 -> 33
        let (mut nodes, conns) =
            arena(&[(0, 48, 2.0, true), (48, 48, 3.0, true), (48, 33, 5.0, true)]);
        let mut p = InputOutputPath::new(0, true);
        assert!(!p.advance_and_check_done(0, &mut nodes, &conns)); // reach 48
        assert_eq!(p.recursivity(), 0);
        assert!(!p.advance_and_check_done(1, &mut nodes, &conns)); // 48 again: now blocked-listed
        assert_eq!(p.recursivity(), 1);
        assert_eq!(p.blocked_nodes(), &[2]);
        assert_eq!(p.visited_nodes.len(), 1);
        assert_eq!(p.total_depth, 2);
        // sqrt(|acc|) because one recursion happened: acc = 2 * 3 = 6
        assert_eq!(p.normalized_affinity(), math::pow(6.0, 0.5));
        // The same synapse again does not add depth, and the node is now blocked: walk ends.
        assert!(p.advance_and_check_done(1, &mut nodes, &conns));
        assert!(p.blocked && !p.done);
        assert_eq!(p.total_depth, 2);
        assert_eq!(p.connections, vec![0, 1]);
    }

    #[test]
    fn duplicate_copies_lists_and_clears_the_finished_flags() {
        let (mut nodes, conns) = arena(&[(0, 48, 2.0, true), (48, 33, 5.0, true)]);
        let mut p = InputOutputPath::new(0, true);
        assert!(!p.advance_and_check_done(0, &mut nodes, &conns));
        let mut c = p.duplicate();
        assert_eq!(c.visited_nodes, p.visited_nodes);
        assert_eq!(c.connections, p.connections);
        assert_eq!(c.accumulated_affinity, 2.0);
        // The copy is independent: finishing it leaves the original untouched.
        assert!(c.advance_and_check_done(1, &mut nodes, &conns));
        assert!(c.done);
        assert!(!p.done);
        assert_eq!(p.connections, vec![0]);
        // A finished path duplicates as unfinished.
        let again = c.duplicate();
        assert!(!again.done && !again.blocked);
    }

    #[test]
    fn affinity_registers_on_both_ends_and_collapse_merges_paths() {
        let (mut nodes, conns) =
            arena(&[(0, 48, 2.0, true), (48, 33, 5.0, true), (0, 33, 3.0, true)]);
        let mut paths = Vec::new();
        // Path A through the hidden node, path B direct; both go 0 -> 33.
        let mut a = InputOutputPath::new(0, true);
        assert!(!a.advance_and_check_done(0, &mut nodes, &conns));
        assert!(a.advance_and_check_done(1, &mut nodes, &conns));
        let mut b = InputOutputPath::new(0, true);
        assert!(b.advance_and_check_done(2, &mut nodes, &conns));
        paths.push(a);
        paths.push(b);
        let mut affinities = Vec::new();
        let id = InputOutputAffinity::create(0, &paths, &mut nodes, &mut affinities);
        assert_eq!(id, 0);
        assert_eq!(nodes[0].affinities, vec![0]);
        assert_eq!(nodes[1].affinities, vec![0]);
        assert_eq!(affinities[0].node_in, Some(0));
        assert_eq!(affinities[0].node_out, Some(1));
        assert_eq!(affinities[0].affinity, 10.0);
        assert_eq!(affinities[0].hidden_nodes, vec![2]);
        assert_eq!(affinities[0].connections, vec![0, 1]);
        affinities[0].collapse_path_into(1, &paths[1]);
        assert_eq!(affinities[0].affinity, 13.0);
        assert_eq!(affinities[0].paths, vec![0, 1]);
        assert_eq!(affinities[0].hidden_nodes, vec![2]);
        assert_eq!(affinities[0].connections, vec![0, 1, 2]);
    }

    #[test]
    fn blocked_path_affinity_has_one_null_end() {
        let (mut nodes, conns) = arena(&[(0, 48, 2.0, true), (48, 48, 3.0, true)]);
        let mut p = InputOutputPath::new(0, true);
        assert!(!p.advance_and_check_done(0, &mut nodes, &conns));
        assert!(!p.advance_and_check_done(1, &mut nodes, &conns));
        assert!(p.advance_and_check_done(1, &mut nodes, &conns));
        assert!(p.blocked);
        let paths = vec![p];
        let mut affinities = Vec::new();
        InputOutputAffinity::create(0, &paths, &mut nodes, &mut affinities);
        assert_eq!(affinities[0].node_in, Some(0));
        assert_eq!(affinities[0].node_out, None);
        assert_eq!(nodes[0].affinities, vec![0]);
        assert!(nodes[1].affinities.is_empty());
        // Walking towards the inputs mirrors it.
        let (mut nodes, conns) = arena(&[(48, 33, 2.0, true), (48, 48, 3.0, true)]);
        let mut p = InputOutputPath::new(1, false);
        assert!(!p.advance_and_check_done(0, &mut nodes, &conns));
        assert!(!p.advance_and_check_done(1, &mut nodes, &conns));
        assert!(p.advance_and_check_done(1, &mut nodes, &conns));
        let paths = vec![p];
        let mut affinities = Vec::new();
        InputOutputAffinity::create(0, &paths, &mut nodes, &mut affinities);
        assert_eq!(affinities[0].node_in, None);
        assert_eq!(affinities[0].node_out, Some(1));
    }
}

//! Input types (what the sorter reads from a brain), the sorter's tuning statics, and the
//! arena id aliases used by every other module.

/// Number of brain input nodes: `NEATBrain.NInputs` (`Enum.GetValues(typeof(Inputs)).Length`,
/// the `Inputs` enum runs `EnergyRatio ..= Phero3Heading`). Equal to `bibites_core::brain::N_INPUTS`.
pub const N_INPUTS: usize = 33;

/// Number of brain output nodes: `NEATBrain.NOutputs` (`Enum.GetValues(typeof(Outputs)).Length`,
/// the `Outputs` enum runs `Accelerate ..= Want2Attack`). Equal to `bibites_core::brain::N_OUTPUTS`.
pub const N_OUTPUTS: usize = 15;

/// `BrainSorter.MaxNodePath`: while exploring input/output paths in
/// [`make_groups`](crate::make_groups), a branch is only forked while the number of live paths is
/// **not greater than** this value. (The C# declares the constant but writes the literal `250`
/// at the use site; both mean the same thing.)
pub const MAX_NODE_PATH: usize = 250;

/// Index of a [`NodeSortHelper`](crate::NodeSortHelper) in [`Layout::nodes`](crate::Layout::nodes).
///
/// The C# holds `NodeSortHelper` object references; the port holds arena indices, which give
/// the same identity semantics (two ids are equal exactly when the C# references were).
pub type NodeId = usize;

/// Index of a [`ConnectionSortHelper`](crate::ConnectionSortHelper) in
/// [`Layout::connections`](crate::Layout::connections). Because one helper is created per
/// input synapse, in order, a `ConnId` is also the position of that synapse in the
/// `connections` slice given to [`make_groups`](crate::make_groups).
pub type ConnId = usize;

/// Index of an [`InputOutputAffinity`](crate::InputOutputAffinity) in
/// [`Layout::affinities`](crate::Layout::affinities).
pub type AffinityId = usize;

/// Index of an [`InputOutputPath`](crate::InputOutputPath) in
/// [`Layout::paths`](crate::Layout::paths). Only finished paths (blocked or done) are kept.
pub type PathId = usize;

/// `NEATBrain.NodeArchetype`. The discriminants match the C# enum (`Input = 0`, `Output = 1`,
/// `Hidden = 2`), which the C# relies on in `InputOutputPath.AdvanceAndCheckDone`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Archetype {
    /// An input node (`Index < N_INPUTS`).
    Input = 0,
    /// An output node (`N_INPUTS <= Index < N_INPUTS + N_OUTPUTS`).
    Output = 1,
    /// A hidden node (`Index >= N_INPUTS + N_OUTPUTS`).
    Hidden = 2,
}

impl Archetype {
    /// The `NEATBrain.Node.archetype` getter: the archetype is derived from the node's `Index`.
    pub fn of_index(index: usize) -> Archetype {
        if index < N_INPUTS {
            Archetype::Input
        } else if index >= N_INPUTS + N_OUTPUTS {
            Archetype::Hidden
        } else {
            Archetype::Output
        }
    }
}

/// The fields of `NEATBrain.Node` that the sorter reads.
///
/// In the C# `archetype` is a computed property of `Index`; build nodes with [`SortNode::new`]
/// to keep the two consistent (the sorter trusts the field).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SortNode {
    /// `Node.Index`: the node's position in the brain (inputs, then outputs, then hidden).
    pub index: usize,
    /// `Node.Inov`: the node's innovation number.
    pub inov: i64,
    /// `Node.archetype`, see [`Archetype::of_index`].
    pub archetype: Archetype,
    /// `Node.Type == NodeType.Sigmoid`.
    pub is_sigmoid: bool,
    /// `Node.baseActivation`.
    pub base_activation: f32,
}

impl SortNode {
    /// Builds a node whose `archetype` is derived from `index`, as the C# getter does.
    pub fn new(index: usize, inov: i64, is_sigmoid: bool, base_activation: f32) -> Self {
        SortNode {
            index,
            inov,
            archetype: Archetype::of_index(index),
            is_sigmoid,
            base_activation,
        }
    }
}

/// The fields of `NEATBrain.Synaps` that the sorter reads (`Inov` is carried along but not read
/// by the C# sorter).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SortSynapse {
    /// `Synaps.NodeIn`: index of the source node.
    pub node_in: usize,
    /// `Synaps.NodeOut`: index of the destination node.
    pub node_out: usize,
    /// `Synaps.Weight`.
    pub weight: f32,
    /// `Synaps.En`: whether the synapse is enabled.
    pub enabled: bool,
    /// `Synaps.Inov`.
    pub inov: i64,
}

impl SortSynapse {
    /// Same argument order as the C# `Synaps(long inov, int nodeI, int nodeO, float weight, bool en)`.
    pub fn new(inov: i64, node_in: usize, node_out: usize, weight: f32, enabled: bool) -> Self {
        SortSynapse {
            node_in,
            node_out,
            weight,
            enabled,
            inov,
        }
    }
}

/// The mutable statics of `BrainSorter`, gathered so a [`Layout`](crate::Layout) can carry them.
/// [`Tuning::default`] holds the values the game ships with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    /// `BrainSorter.enabledFactor` (`1f`): weight multiplier of an enabled connection's pull.
    pub enabled_factor: f32,
    /// `BrainSorter.disabledFactor` (`0.5f`): multiplier applied to disabled connections.
    pub disabled_factor: f32,
    /// `BrainSorter.initialPlacementTemp` (`0.1f`): amplitude of the random vertical jitter in
    /// the initial hidden-node placement.
    pub initial_placement_temp: f32,
    /// `BrainSorter.connectionPush` (`1f`): strength with which connections push other hidden
    /// nodes sideways.
    pub connection_push: f32,
    /// `BrainSorter.repulsionFactor` (`1f`). Declared by the C# but never read; kept for parity.
    pub repulsion_factor: f32,
    /// `BrainSorter.nonHiddenRepulsionFactor` (`0f`): repulsion between a hidden node and an
    /// input/output node.
    pub non_hidden_repulsion_factor: f32,
}

impl Tuning {
    /// The values of the C# static field initialisers.
    pub const DEFAULT: Tuning = Tuning {
        enabled_factor: 1.0,
        disabled_factor: 0.5,
        initial_placement_temp: 0.1,
        connection_push: 1.0,
        repulsion_factor: 1.0,
        non_hidden_repulsion_factor: 0.0,
    };
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archetype_follows_the_index_thresholds() {
        assert_eq!(Archetype::of_index(0), Archetype::Input);
        assert_eq!(Archetype::of_index(N_INPUTS - 1), Archetype::Input);
        assert_eq!(Archetype::of_index(N_INPUTS), Archetype::Output);
        assert_eq!(
            Archetype::of_index(N_INPUTS + N_OUTPUTS - 1),
            Archetype::Output
        );
        assert_eq!(Archetype::of_index(N_INPUTS + N_OUTPUTS), Archetype::Hidden);
        assert_eq!(Archetype::of_index(1000), Archetype::Hidden);
    }

    #[test]
    fn archetype_discriminants_match_the_csharp_enum() {
        assert_eq!(Archetype::Input as i32, 0);
        assert_eq!(Archetype::Output as i32, 1);
        assert_eq!(Archetype::Hidden as i32, 2);
    }

    #[test]
    fn node_and_synapse_constructors_fill_the_fields() {
        let n = SortNode::new(40, 41, true, 0.25);
        assert_eq!(n.archetype, Archetype::Output);
        assert!(n.is_sigmoid);
        let s = SortSynapse::new(9, 1, 2, -0.5, false);
        assert_eq!(
            (s.inov, s.node_in, s.node_out, s.weight, s.enabled),
            (9, 1, 2, -0.5, false)
        );
    }

    #[test]
    fn tuning_defaults_are_the_shipped_statics() {
        let t = Tuning::default();
        assert_eq!(t.enabled_factor, 1.0);
        assert_eq!(t.disabled_factor, 0.5);
        assert_eq!(t.initial_placement_temp, 0.1);
        assert_eq!(t.connection_push, 1.0);
        assert_eq!(t.repulsion_factor, 1.0);
        assert_eq!(t.non_hidden_repulsion_factor, 0.0);
        assert_eq!(MAX_NODE_PATH, 250);
    }
}

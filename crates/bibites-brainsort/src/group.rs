//! `NodeGroup`: a connected cluster of inputs, outputs and hidden nodes laid out together.

use crate::math;
use crate::nodes::NodeSortHelper;
use crate::path::InputOutputAffinity;
use crate::types::{AffinityId, ConnId, NodeId, PathId};
use crate::vec2::Vec2;

/// `NodeGroup`: one cluster of the brain diagram.
///
/// All node/synapse/affinity lists hold arena ids into the owning [`Layout`](crate::Layout)
/// (`Layout::nodes`, `Layout::connections`, `Layout::affinities`, `Layout::paths`), in exactly
/// the order the C# `List`s hold their references.
#[derive(Clone, Debug, Default)]
pub struct NodeGroup {
    /// `groupStart`: top of the group's band, set by the consumer through [`NodeGroup::set_start_end`].
    pub group_start: f32,
    /// `groupHeight`: height of the group's band, set through [`NodeGroup::set_start_end`].
    pub group_height: f32,
    /// `inputs`.
    pub inputs: Vec<NodeId>,
    /// `outputs`.
    pub outputs: Vec<NodeId>,
    /// `hidden` (filled by `FinalizeInternalStructure` from the affinities' hidden nodes).
    pub hidden: Vec<NodeId>,
    /// `nodes`: `inputs`, then `outputs`, then `hidden` (concatenated by `FinalizeInternalStructure`).
    pub nodes: Vec<NodeId>,
    /// `connections`: distinct synapses of the group's affinities.
    pub connections: Vec<ConnId>,
    /// `inputOutputAffinities`.
    pub input_output_affinities: Vec<AffinityId>,
    /// `paths`. The C# declares this list but nothing ever adds to it, so it stays empty (and
    /// [`Layout::group_max_depth`](crate::Layout::group_max_depth) is always 0).
    pub paths: Vec<PathId>,
    /// `isInputlessOutputsGroup`: the group only holds unconnected-but-active output nodes.
    pub is_inputless_outputs_group: bool,
}

impl NodeGroup {
    /// `new NodeGroup()`.
    pub fn new() -> Self {
        NodeGroup::default()
    }

    /// `NodeGroup.size`: `inputs.Count + outputs.Count`.
    pub fn size(&self) -> usize {
        self.inputs.len() + self.outputs.len()
    }

    /// `NodeGroup.weight`: how much vertical room the group deserves,
    /// `(inputs > 0 ? max(inputs, outputs) - 1 : 0) + hidden / sqrt(2 + hidden + inputs + outputs)`.
    pub fn weight(&self) -> f32 {
        let inputs = self.inputs.len();
        let outputs = self.outputs.len();
        let hidden = self.hidden.len();
        let spread = if inputs > 0 {
            // `(float)(Mathf.Max(inputs.Count, outputs.Count) - 1)`
            (inputs.max(outputs) as i64 - 1) as f32
        } else {
            0.0
        };
        spread + hidden as f32 / math::sqrt(2.0 + hidden as f32 + inputs as f32 + outputs as f32)
    }

    /// `NodeGroup.SetStartEnd(start, height)`.
    pub fn set_start_end(&mut self, start: f32, height: f32) {
        self.group_start = start;
        self.group_height = height;
    }

    /// `NodeGroup.FinalizeInternalStructure()`: collects the hidden nodes and synapses from the
    /// group's affinities, builds `nodes`, and lays inputs out on the left edge and outputs on
    /// the right edge (or along the bottom edge for an inputless-outputs group).
    ///
    /// With a single input (or output) the C# divides `0 / (1 - 1)` and stores NaN as its `y`;
    /// that is reproduced here (the first [`Layout::sort`](crate::Layout::sort) pass overwrites it).
    pub(crate) fn finalize_internal_structure(
        &mut self,
        nodes: &mut [NodeSortHelper],
        affinities: &[InputOutputAffinity],
    ) {
        for &affinity in &self.input_output_affinities {
            let affinity = &affinities[affinity];
            for &hidden_node in &affinity.hidden_nodes {
                if !self.hidden.contains(&hidden_node) {
                    self.hidden.push(hidden_node);
                }
            }
            for &connection in &affinity.connections {
                if !self.connections.contains(&connection) {
                    self.connections.push(connection);
                }
            }
        }
        self.nodes.extend_from_slice(&self.inputs);
        self.nodes.extend_from_slice(&self.outputs);
        self.nodes.extend_from_slice(&self.hidden);
        let input_count = self.inputs.len() as f32;
        for (i, &input) in self.inputs.iter().enumerate() {
            nodes[input].group_pos = Vec2::new(0.0, i as f32 / (input_count - 1.0));
        }
        let output_count = self.outputs.len() as f32;
        if self.is_inputless_outputs_group {
            for (j, &output) in self.outputs.iter().enumerate() {
                nodes[output].group_pos = Vec2::new((j + 1) as f32 / output_count, 1.0);
            }
        } else {
            for (k, &output) in self.outputs.iter().enumerate() {
                nodes[output].group_pos = Vec2::new(1.0, k as f32 / (output_count - 1.0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(inputs: usize, outputs: usize, hidden: usize) -> NodeGroup {
        NodeGroup {
            inputs: (0..inputs).collect(),
            outputs: (0..outputs).collect(),
            hidden: (0..hidden).collect(),
            ..NodeGroup::default()
        }
    }

    #[test]
    fn weight_follows_the_csharp_formula() {
        // inputs > 0: (max(2, 1) - 1) + 3 / sqrt(2 + 3 + 2 + 1)
        let w = group(2, 1, 3).weight();
        assert_eq!(w, 1.0 + 3.0 / math::sqrt(8.0));
        // one input, one output, no hidden nodes: 0.
        assert_eq!(group(1, 1, 0).weight(), 0.0);
        // no inputs: the spread term is 0 whatever the outputs are.
        assert_eq!(group(0, 7, 0).weight(), 0.0);
        // outputs dominate when there are more of them.
        assert_eq!(group(2, 5, 0).weight(), 4.0);
    }

    #[test]
    fn size_and_set_start_end() {
        let mut g = group(3, 2, 9);
        assert_eq!(g.size(), 5);
        g.set_start_end(-4.5, 20.0);
        assert_eq!((g.group_start, g.group_height), (-4.5, 20.0));
    }
}

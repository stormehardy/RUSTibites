//! # bibites-brainsort
//!
//! A dependency-free, pure-Rust port of the brain-diagram layout algorithm of
//! [The Bibites](https://thebibites.com/): the decompiled C# classes `BrainSorter`, `NodeGroup`,
//! `NodeSortHelper`, `ConnectionSortHelper`, `InputOutputPath`, `InputOutputAffinity`,
//! `HiddenDepthInPath` and `HiddenNodeAffinity` (folder `ScriptHelpers`), as driven by
//! `BrainPanelCommon` / `BibiteBrainEditor` (`FillBrainPanel`, `PlaceNodesAccordingToSort`,
//! `SortN`, `SortHiddenOnce`).
//!
//! ## The one deliberate difference: randomness
//!
//! The C# calls `UnityEngine.Random.Range(-1f, 1f)` and `UnityEngine.Random.insideUnitCircle`
//! while it places the hidden nodes in `BrainSorter.Sort`. Those are replaced by an explicit,
//! seedable generator ([`Rng`], xorshift64*) that is passed to [`Layout::sort`]. The **order and
//! number of draws** is exactly the C# one, so the same seed always gives the same layout, but
//! the drawn numbers are not Unity's: the initial jitter of the hidden nodes (and with it the
//! exact final hidden-node positions) can differ from the game's. Everything else, including
//! the float operation order, NaN behaviour of `Mathf`/`Vector2`, LINQ's stable `OrderBy` and the
//! iteration order of every `List`/`Dictionary`, is reproduced as written.
//!
//! ## Shape of the port
//!
//! The C# helper objects point at each other (`NodeSortHelper` <-> `InputOutputAffinity` <->
//! `InputOutputPath` <-> `NodeGroup`) and the algorithm relies on reference identity
//! (`List.Contains`, `List.Remove`, dictionary keys). The port keeps every helper in an arena
//! `Vec` owned by a [`Layout`], and objects refer to each other by index
//! ([`NodeId`], [`ConnId`], [`AffinityId`], [`PathId`]); index equality is reference identity.
//!
//! | C# | Rust |
//! |----|------|
//! | `BrainSorter.MakeGroups(nodes, connections)` | [`make_groups`] |
//! | `BrainSorter.UsedIndicesOfGroups(groups)` | [`Layout::used_indices_of_groups`] |
//! | `BrainSorter.Sort(groups, iteration, nodesToIgnore)` | [`Layout::sort`] |
//! | `BrainSorter.OptimizeHiddenPlacement(group, dT, scalar)` | [`Layout::optimize_hidden_placement`] |
//! | `NodeGroup.SetStartEnd(start, height)` | [`Layout::set_start_end`] / [`NodeGroup::set_start_end`] |
//! | `NodeGroup.weight` | [`NodeGroup::weight`] |
//! | `BrainSorter` statics, `MaxNodePath` | [`Tuning`] (`Layout::tuning`), [`MAX_NODE_PATH`] |
//! | `NEATBrain.Node` / `NEATBrain.Synaps` (the fields read) | [`SortNode`] / [`SortSynapse`] |
//! | `NEATBrain.NodeArchetype`, `NInputs`, `NOutputs` | [`Archetype`], [`N_INPUTS`], [`N_OUTPUTS`] |
//! | `UnityEngine.Vector2` | [`Vec2`] |
//!
//! ## Typical use (what `FillBrainPanel` does)
//!
//! ```
//! use bibites_brainsort::{make_groups, Rng, SortNode, SortSynapse, N_INPUTS, N_OUTPUTS};
//!
//! // A brain with one hidden node: input 0 -> hidden -> output 0.
//! let hidden = N_INPUTS + N_OUTPUTS;
//! let nodes: Vec<SortNode> = (0..=hidden).map(|i| SortNode::new(i, i as i64 + 1, false, 0.0)).collect();
//! let synapses = [
//!     SortSynapse::new(1, 0, hidden, 1.5, true),
//!     SortSynapse::new(2, hidden, N_INPUTS, -0.8, true),
//! ];
//!
//! let mut layout = make_groups(&nodes, &synapses);
//! let used = layout.used_indices_of_groups();
//! assert_eq!(used, vec![0, N_INPUTS, hidden]);
//!
//! layout.sort(3, None, &mut Rng::new(1));
//! for group in &layout.groups {
//!     for &node in &group.nodes {
//!         let helper = &layout.nodes[node];
//!         // group_pos is in [0, 1] x [0, 1]; the consumer scales it onto the panel.
//!         assert!(helper.group_pos.x.is_finite() && helper.group_pos.y.is_finite());
//!     }
//! }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod group;
mod math;
mod nodes;
mod path;
mod rng;
mod sorter;
mod types;
mod vec2;

#[cfg(test)]
mod tests;

pub use group::NodeGroup;
pub use nodes::{ConnectionSortHelper, HiddenDepthInPath, HiddenNodeAffinity, NodeSortHelper};
pub use path::{InputOutputAffinity, InputOutputPath};
pub use rng::Rng;
pub use sorter::{Layout, make_groups};
pub use types::{
    AffinityId, Archetype, ConnId, MAX_NODE_PATH, N_INPUTS, N_OUTPUTS, NodeId, PathId, SortNode,
    SortSynapse, Tuning,
};
pub use vec2::Vec2;

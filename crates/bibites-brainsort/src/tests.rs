//! Behavioural tests of the whole port. Every expectation was derived by tracing the C#
//! (`BrainSorter.MakeGroups`, `Sort`, ...) by hand, not by running this implementation; the
//! traces are summarised in the comments next to each test.

use crate::{Archetype, Layout, N_INPUTS, N_OUTPUTS, NodeId, Rng, SortNode, SortSynapse, Vec2};
use crate::{HiddenNodeAffinity, make_groups};

/// First hidden node index.
const H0: usize = N_INPUTS + N_OUTPUTS;
/// Index of output node 0 ("Accelerate").
const O0: usize = N_INPUTS;

/// A full node list as a real brain has it (`nodes[i].index == i`); outputs are plain linear
/// nodes, so unconnected outputs are *not* "inputless outputs".
fn base_nodes(n_hidden: usize) -> Vec<SortNode> {
    (0..N_INPUTS + N_OUTPUTS + n_hidden)
        .map(|i| SortNode::new(i, i as i64 + 1, false, 0.0))
        .collect()
}

fn syn(node_in: usize, node_out: usize, weight: f32) -> SortSynapse {
    SortSynapse::new(0, node_in, node_out, weight, true)
}

/// Node `Index` of every id in the list.
fn indices(layout: &Layout, ids: &[NodeId]) -> Vec<usize> {
    ids.iter().map(|&n| layout.nodes[n].index()).collect()
}

fn id_of(layout: &Layout, index: usize) -> NodeId {
    layout
        .node_id_by_index(index)
        .expect("node takes part in the diagram")
}

fn pos(layout: &Layout, index: usize) -> Vec2 {
    layout.nodes[id_of(layout, index)].group_pos
}

fn set_pos(layout: &mut Layout, index: usize, p: Vec2) {
    let id = id_of(layout, index);
    layout.nodes[id].group_pos = p;
}

fn assert_close(actual: f32, expected: f32, eps: f32) {
    assert!(
        (actual - expected).abs() <= eps,
        "{actual} is not within {eps} of {expected}"
    );
}

fn bits(layout: &Layout) -> Vec<(u32, u32)> {
    layout
        .nodes
        .iter()
        .map(|n| (n.group_pos.x.to_bits(), n.group_pos.y.to_bits()))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// (1)-(4): the group structure
// ---------------------------------------------------------------------------------------------

/// Trace: list = [0, 33]; forward path 0->33 is done, so is the reverse path 33->0. The two have
/// different (source, next) keys, so there are two affinities, both registered on both nodes.
/// The seed 0 pulls in output 33 through them; nothing else exists.
#[test]
fn single_connection_is_one_group_with_one_input_and_one_output() {
    let layout = make_groups(&base_nodes(0), &[syn(0, O0, 1.0)]);
    assert_eq!(layout.groups.len(), 1);
    let g = &layout.groups[0];
    assert_eq!(indices(&layout, &g.inputs), vec![0]);
    assert_eq!(indices(&layout, &g.outputs), vec![O0]);
    assert!(g.hidden.is_empty());
    assert_eq!(indices(&layout, &g.nodes), vec![0, O0]);
    assert_eq!(g.connections, vec![0]);
    assert!(!g.is_inputless_outputs_group);
    assert_eq!(g.weight(), 0.0);
    assert_eq!(layout.affinities.len(), 2);
    assert_eq!(layout.affinities[0].affinity, 1.0);
    assert_eq!(layout.paths.len(), 2);
    assert_eq!(layout.nodes.len(), 2);
    assert_eq!(layout.connections.len(), 1);
}

/// `FinalizeInternalStructure` puts a lone input at `(0, 0/(1-1))` and a lone output at
/// `(1, 0/(1-1))`, i.e. NaN heights; the first sorting pass then assigns `y = 0.5`.
#[test]
fn lone_input_and_output_start_with_nan_height_and_sort_to_the_middle() {
    let mut layout = make_groups(&base_nodes(0), &[syn(0, O0, 1.0)]);
    assert_eq!(pos(&layout, 0).x, 0.0);
    assert!(pos(&layout, 0).y.is_nan());
    assert_eq!(pos(&layout, O0).x, 1.0);
    assert!(pos(&layout, O0).y.is_nan());
    layout.sort(1, None, &mut Rng::new(1));
    assert_eq!(pos(&layout, 0), Vec2::new(0.0, 0.5));
    assert_eq!(pos(&layout, O0), Vec2::new(1.0, 0.5));
}

/// Two chains 0->33 and 1->34 share nothing, so the seeds 0 and 1 each start their own group
/// (groups come out in order of their lowest seed index, not in synapse order).
#[test]
fn two_disjoint_chains_are_two_groups() {
    let layout = make_groups(&base_nodes(0), &[syn(1, O0 + 1, 1.0), syn(0, O0, 1.0)]);
    assert_eq!(layout.groups.len(), 2);
    assert_eq!(indices(&layout, &layout.groups[0].inputs), vec![0]);
    assert_eq!(indices(&layout, &layout.groups[0].outputs), vec![O0]);
    assert_eq!(indices(&layout, &layout.groups[1].inputs), vec![1]);
    assert_eq!(indices(&layout, &layout.groups[1].outputs), vec![O0 + 1]);
    // The helper arena follows the C# `list`: connection endpoints in order of discovery.
    assert_eq!(
        indices(&layout, &(0..layout.nodes.len()).collect::<Vec<_>>()),
        vec![1, 34, 0, 33]
    );
}

/// Seed 0 reaches output 33 through its affinities; 33's affinities then reach input 1.
#[test]
fn two_inputs_feeding_one_output_share_a_group() {
    let layout = make_groups(&base_nodes(0), &[syn(0, O0, 1.0), syn(1, O0, 1.0)]);
    assert_eq!(layout.groups.len(), 1);
    let g = &layout.groups[0];
    assert_eq!(indices(&layout, &g.inputs), vec![0, 1]);
    assert_eq!(indices(&layout, &g.outputs), vec![O0]);
    assert!(g.hidden.is_empty());
    // weight = (max(2, 1) - 1) + 0 / sqrt(...)
    assert_eq!(g.weight(), 1.0);
    assert_eq!(g.connections.len(), 2);
}

/// 0 -(2)-> 48 -(3)-> 33. The forward walk visits 48 at depth 1 with accumulated affinity 2; the
/// reverse walk visits it at depth 1 with accumulated affinity 3. Total depth is 2 both ways.
/// Hidden affinities of 48: towards the input (affinity 2, closeness 2/1) and towards the output
/// (affinity 3, closeness 2/(2-1)), each fed twice (n = 2, same values).
#[test]
fn hidden_node_between_input_and_output_belongs_to_the_group() {
    let layout = make_groups(&base_nodes(1), &[syn(0, H0, 2.0), syn(H0, O0, 3.0)]);
    assert_eq!(layout.groups.len(), 1);
    let g = &layout.groups[0];
    assert_eq!(indices(&layout, &g.inputs), vec![0]);
    assert_eq!(indices(&layout, &g.outputs), vec![O0]);
    assert_eq!(indices(&layout, &g.hidden), vec![H0]);
    assert_eq!(indices(&layout, &g.nodes), vec![0, O0, H0]);
    assert_eq!(g.connections, vec![0, 1]);
    assert_eq!(g.weight(), 1.0 / 5.0f32.sqrt());

    let hidden = &layout.nodes[id_of(&layout, H0)];
    assert_eq!(
        hidden.hidden_affinities_to_inputs_outputs,
        vec![
            (
                id_of(&layout, 0),
                HiddenNodeAffinity {
                    affinity: 2.0,
                    closeness: 2.0,
                    n: 2
                }
            ),
            (
                id_of(&layout, O0),
                HiddenNodeAffinity {
                    affinity: 3.0,
                    closeness: 2.0,
                    n: 2
                }
            ),
        ]
    );
    // The two walks have different endpoints keys: forward 0->33 is |2*3| = 6, reverse likewise.
    assert_eq!(layout.affinities.len(), 2);
    assert_eq!(layout.affinities[0].affinity, 6.0);
    assert_eq!(layout.affinities[1].affinity, 6.0);
    assert_eq!(layout.affinities[0].hidden_nodes, vec![id_of(&layout, H0)]);
}

/// 0 -(2)-> 48 -(3)-> 49 -(4)-> 33: closeness is `totalDepth / depth` towards the source and
/// `totalDepth / (totalDepth - depth)` towards the destination.
/// Forward: 48 has (affinity 2, closeness 3) to the input and (24/2 = 12, 3/2) to the output;
/// 49 has (6, 3/2) and (24/6 = 4, 3). The reverse walk contributes the very same numbers.
#[test]
fn two_hidden_nodes_in_a_chain_get_depth_based_closeness() {
    let layout = make_groups(
        &base_nodes(2),
        &[syn(0, H0, 2.0), syn(H0, H0 + 1, 3.0), syn(H0 + 1, O0, 4.0)],
    );
    assert_eq!(layout.groups.len(), 1);
    assert_eq!(indices(&layout, &layout.groups[0].hidden), vec![H0, H0 + 1]);
    let (input, output) = (id_of(&layout, 0), id_of(&layout, O0));
    let first = &layout.nodes[id_of(&layout, H0)].hidden_affinities_to_inputs_outputs;
    let second = &layout.nodes[id_of(&layout, H0 + 1)].hidden_affinities_to_inputs_outputs;
    let expect = |list: &Vec<(NodeId, HiddenNodeAffinity)>, key, affinity, closeness| {
        let entry = list.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
        assert_eq!(
            entry,
            Some(HiddenNodeAffinity {
                affinity,
                closeness,
                n: 2
            })
        );
    };
    expect(first, input, 2.0, 3.0);
    expect(first, output, 12.0, 1.5);
    expect(second, input, 6.0, 1.5);
    expect(second, output, 4.0, 3.0);
    assert_eq!(
        first[0].0, input,
        "the source is inserted before the destination"
    );
}

/// Disabled synapses still define groups, and a disabled synapse contributes `sqrt(|w|)`.
#[test]
fn disabled_synapse_still_groups_and_uses_the_square_root_of_its_weight() {
    let layout = make_groups(&base_nodes(0), &[SortSynapse::new(0, 0, O0, -4.0, false)]);
    assert_eq!(layout.groups.len(), 1);
    assert_eq!(layout.affinities[0].affinity, 2.0);
}

// ---------------------------------------------------------------------------------------------
// (5): outputs without synapses
// ---------------------------------------------------------------------------------------------

/// `list2`: Output archetype, (Sigmoid or non-zero base activation), not in any synapse. They go
/// into one `isInputlessOutputsGroup` group in `nodes` order, laid out along the bottom edge
/// at `((j + 1) / n, 1)`.
#[test]
fn active_outputs_without_synapses_form_an_inputless_group() {
    let mut nodes = base_nodes(0);
    nodes[O0].is_sigmoid = true; // sigmoid output
    nodes[O0 + 2].base_activation = -0.5; // non-zero base activation
    nodes[O0 + 4].base_activation = 0.0; // plain output: ignored
    nodes[3].is_sigmoid = true; // an input: ignored (not an Output archetype)
    nodes[H0 - 1].is_sigmoid = true; // the last output
    let layout = make_groups(&nodes, &[]);
    assert_eq!(layout.groups.len(), 1);
    let g = &layout.groups[0];
    assert!(g.is_inputless_outputs_group);
    assert!(g.inputs.is_empty() && g.hidden.is_empty());
    assert_eq!(indices(&layout, &g.outputs), vec![O0, O0 + 2, H0 - 1]);
    assert_eq!(indices(&layout, &g.nodes), vec![O0, O0 + 2, H0 - 1]);
    assert_eq!(g.weight(), 0.0);
    assert_eq!(pos(&layout, O0), Vec2::new(1.0 / 3.0, 1.0));
    assert_eq!(pos(&layout, O0 + 2), Vec2::new(2.0 / 3.0, 1.0));
    assert_eq!(pos(&layout, H0 - 1), Vec2::new(1.0, 1.0));
    assert_eq!(layout.used_indices_of_groups(), vec![O0, O0 + 2, H0 - 1]);
    assert!(layout.connections.is_empty() && layout.paths.is_empty());
}

#[test]
fn a_connected_sigmoid_output_is_not_an_inputless_output() {
    let mut nodes = base_nodes(0);
    nodes[O0].is_sigmoid = true;
    let layout = make_groups(&nodes, &[syn(0, O0, 1.0)]);
    assert_eq!(layout.groups.len(), 1);
    assert!(!layout.groups[0].is_inputless_outputs_group);
}

/// More than five inputless outputs are dealt alternately into two groups (list order: even
/// positions to the first group, odd to the second); five or fewer stay in one.
#[test]
fn many_inputless_outputs_are_split_in_two_interleaved_groups() {
    let mut nodes = base_nodes(0);
    for node in &mut nodes[O0..O0 + 5] {
        node.is_sigmoid = true;
    }
    let five = make_groups(&nodes, &[]);
    assert_eq!(five.groups.len(), 1);
    assert_eq!(five.groups[0].outputs.len(), 5);

    nodes[O0 + 5].is_sigmoid = true;
    let six = make_groups(&nodes, &[]);
    assert_eq!(six.groups.len(), 2);
    assert_eq!(
        indices(&six, &six.groups[0].outputs),
        vec![O0, O0 + 2, O0 + 4]
    );
    assert_eq!(
        indices(&six, &six.groups[1].outputs),
        vec![O0 + 1, O0 + 3, O0 + 5]
    );
    assert!(six.groups.iter().all(|g| g.is_inputless_outputs_group));
}

/// The default brain (every output a sigmoid, no synapses) shows as two inputless groups of
/// 8 and 7 outputs.
#[test]
fn default_outputs_without_a_brain_split_eight_and_seven() {
    let mut nodes = base_nodes(0);
    for node in &mut nodes[O0..H0] {
        node.is_sigmoid = true;
    }
    let layout = make_groups(&nodes, &[]);
    assert_eq!(layout.groups.len(), 2);
    assert_eq!(layout.groups[0].outputs.len(), 8);
    assert_eq!(layout.groups[1].outputs.len(), 7);
    assert_eq!(layout.used_indices_of_groups().len(), N_OUTPUTS);
}

/// Inputless groups get their `y` from the sorting pass (slots `k/(n-1)`, or 0.5 for a single
/// output), overwriting the bottom-edge `y = 1`; `x` keeps `(j + 1) / n`.
#[test]
fn sort_spreads_the_inputless_outputs_vertically() {
    let mut nodes = base_nodes(0);
    for node in &mut nodes[O0..O0 + 3] {
        node.is_sigmoid = true;
    }
    let mut layout = make_groups(&nodes, &[]);
    layout.sort(3, None, &mut Rng::new(3));
    assert_eq!(pos(&layout, O0), Vec2::new(1.0 / 3.0, 0.0));
    assert_eq!(pos(&layout, O0 + 1), Vec2::new(2.0 / 3.0, 0.5));
    assert_eq!(pos(&layout, O0 + 2), Vec2::new(1.0, 1.0));
}

// ---------------------------------------------------------------------------------------------
// (6): used indices
// ---------------------------------------------------------------------------------------------

/// Group A = {0, 1} -> 33; group B = {2} -> 48 -> 34. Node 5 has no synapse and is not used.
/// `UsedIndicesOfGroups` lists each group's `nodes` (inputs, outputs, hidden) in group order.
#[test]
fn used_indices_cover_every_grouped_node_exactly_once() {
    let syns = [
        syn(0, O0, 1.0),
        syn(1, O0, 1.0),
        syn(2, H0, 1.0),
        syn(H0, O0 + 1, 1.0),
    ];
    let layout = make_groups(&base_nodes(1), &syns);
    assert_eq!(layout.groups.len(), 2);
    let used = layout.used_indices_of_groups();
    assert_eq!(used, vec![0, 1, O0, 2, O0 + 1, H0]);
    // every grouped node once
    let mut from_groups: Vec<usize> = layout
        .groups
        .iter()
        .flat_map(|g| indices(&layout, &g.nodes))
        .collect();
    from_groups.sort_unstable();
    let mut sorted = used.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, from_groups);
    sorted.dedup();
    assert_eq!(sorted.len(), used.len());
    assert!(!used.contains(&5));
}

/// A node listed in two groups would be reported once (`if (!list.Contains(..))`).
#[test]
fn used_indices_deduplicate_nodes_shared_between_groups() {
    let mut layout = make_groups(&base_nodes(0), &[syn(0, O0, 1.0), syn(1, O0 + 1, 1.0)]);
    let shared = layout.groups[0].nodes[0];
    layout.groups[1].nodes.push(shared);
    assert_eq!(layout.used_indices_of_groups(), vec![0, O0, 1, O0 + 1]);
}

// ---------------------------------------------------------------------------------------------
// (7): sorting
// ---------------------------------------------------------------------------------------------

/// Synapses 0:I0->O1(5), 1:I1->O0(5), 2:I0->O0(.5), 3:I1->O1(.5), nodes I0=0, I1=1, O0=33,
/// O1=34. Hand trace of pass 1 (`ComputeLocalMinima`, inputs first, then outputs against the
/// *new* input heights):
///   I0: y=0, pulled by affinity 10 towards O1 (y=1) and 1 towards O0 (y=0), pushed down by I1:
///       num = 20 - 3e^-3, den = 22 + 9e^-3  =>  +0.884      I1 likewise => -0.884
///   so inputs order [I1, I0]: I1.y = 0, I0.y = 1.
///   O0: y=0, partners I0 (y=1, affinity 1) and I1 (y=0, affinity 10): num = 2 - 3e^-3 => +0.082
///   O1: partners I0 (y=1, 10) and I1 (y=0, 1), y=1: num = -2 + 3e^-3                  => -0.082
///   so outputs order [O1, O0]: O1.y = 0, O0.y = 1.
#[test]
fn first_sorting_pass_orders_inputs_and_outputs_by_their_pull() {
    let syns = [
        syn(0, O0 + 1, 5.0),
        syn(1, O0, 5.0),
        syn(0, O0, 0.5),
        syn(1, O0 + 1, 0.5),
    ];
    let mut layout = make_groups(&base_nodes(0), &syns);
    assert_eq!(layout.groups.len(), 1);
    assert_eq!(indices(&layout, &layout.groups[0].inputs), vec![0, 1]);
    assert_eq!(
        indices(&layout, &layout.groups[0].outputs),
        vec![O0, O0 + 1]
    );
    assert_eq!(layout.affinities.len(), 8);
    // before sorting: even spacing in list order
    assert_eq!(pos(&layout, 0), Vec2::new(0.0, 0.0));
    assert_eq!(pos(&layout, 1), Vec2::new(0.0, 1.0));
    assert_eq!(pos(&layout, O0), Vec2::new(1.0, 0.0));
    assert_eq!(pos(&layout, O0 + 1), Vec2::new(1.0, 1.0));

    layout.sort(1, None, &mut Rng::new(1));
    assert_eq!(pos(&layout, 1), Vec2::new(0.0, 0.0));
    assert_eq!(pos(&layout, 0), Vec2::new(0.0, 1.0));
    assert_eq!(pos(&layout, O0 + 1), Vec2::new(1.0, 0.0));
    assert_eq!(pos(&layout, O0), Vec2::new(1.0, 1.0));
    // The accumulated ordering keys are what the C# stores in tempPos.y:
    //   inputs  +-(20 - 3e^-3) / (22 + 9e^-3),   outputs  +-(2 - 3e^-3) / (22 + 9e^-3).
    let e3 = (-3.0f64).exp();
    let input_key = ((20.0 - 3.0 * e3) / (22.0 + 9.0 * e3)) as f32;
    let output_key = ((2.0 - 3.0 * e3) / (22.0 + 9.0 * e3)) as f32;
    assert_close(layout.nodes[id_of(&layout, 0)].temp_pos.y, input_key, 1e-5);
    assert_close(layout.nodes[id_of(&layout, 1)].temp_pos.y, -input_key, 1e-5);
    assert_close(
        layout.nodes[id_of(&layout, O0)].temp_pos.y,
        output_key,
        1e-5,
    );
    assert_close(
        layout.nodes[id_of(&layout, O0 + 1)].temp_pos.y,
        -output_key,
        1e-5,
    );
}

/// With a single output its initial height is `0/(1-1) = NaN`, so every input's `tempPos.y`
/// becomes NaN in the first pass (and stays NaN: `NaN + x = NaN`). LINQ's `OrderBy` treats
/// NaN keys as equal, so the inputs keep the group's order for ever.
///
/// Group order trace: list = [2, 33, 0, 1]; walking from 33 towards the inputs forks over the
/// synapses last-to-first, so the reverse affinities are created for inputs 1, 0, 2 in that
/// order, after the forward ones for 2 (first), then 0 and 1. Seed 0 reaches output 33, whose
/// affinities (creation order) name the inputs 2 (forward), 1, 0, 2, 0, 1: group inputs [0, 2, 1].
#[test]
fn nan_ordering_keys_keep_the_original_order() {
    let syns = [syn(2, O0, 9.0), syn(0, O0, 1.0), syn(1, O0, 5.0)];
    let mut layout = make_groups(&base_nodes(0), &syns);
    assert_eq!(indices(&layout, &layout.groups[0].inputs), vec![0, 2, 1]);
    layout.sort(4, None, &mut Rng::new(2));
    for (slot, index) in [0usize, 2, 1].into_iter().enumerate() {
        assert_eq!(pos(&layout, index).y, slot as f32 / 2.0);
        assert!(layout.nodes[id_of(&layout, index)].temp_pos.y.is_nan());
    }
    assert_eq!(pos(&layout, O0), Vec2::new(1.0, 0.5));
}

/// A brain with several groups and hidden nodes of different shapes, plus two active outputs
/// (indices 41 and 42) that have no synapse.
fn rich_graph() -> (Vec<SortNode>, Vec<SortSynapse>) {
    let mut nodes = base_nodes(5);
    nodes[O0 + 8].is_sigmoid = true;
    nodes[O0 + 9].base_activation = 0.3;
    let syns = vec![
        syn(0, H0, 0.8),
        syn(1, H0, -0.6),
        syn(1, H0 + 1, 1.2),
        syn(2, H0 + 1, 0.5),
        syn(H0, H0 + 2, 1.1),
        syn(H0 + 1, H0 + 2, -0.9),
        syn(H0 + 1, H0 + 3, 0.7),
        syn(H0 + 2, O0, 1.3),
        syn(H0 + 3, O0 + 1, -1.4),
        syn(H0, O0 + 1, 0.4),
        syn(0, O0, -0.3),
        SortSynapse::new(0, 7, H0 + 4, 2.0, false),
        syn(H0 + 4, O0 + 5, 1.0),
    ];
    (nodes, syns)
}

fn sorted_rich(seed: u64, iterations: usize) -> Layout {
    let (nodes, syns) = rich_graph();
    let mut layout = make_groups(&nodes, &syns);
    layout.sort(iterations, None, &mut Rng::new(seed));
    layout
}

/// Same seed -> bit-identical layout; different seeds -> different hidden-node placement.
#[test]
fn sort_is_deterministic_per_seed_and_depends_on_the_seed() {
    let a = bits(&sorted_rich(11, 3));
    assert_eq!(a, bits(&sorted_rich(11, 3)));
    let others: Vec<Vec<(u32, u32)>> = [12u64, 13, 14]
        .iter()
        .map(|&s| bits(&sorted_rich(s, 3)))
        .collect();
    assert!(
        others.iter().any(|o| *o != a),
        "seeds 12..=14 all reproduced seed 11"
    );
    // The inputs/outputs ordering involves no randomness: only hidden nodes move with the seed.
    let (first, second) = (sorted_rich(11, 3), sorted_rich(12, 3));
    for (n1, n2) in first.nodes.iter().zip(&second.nodes) {
        if n1.archetype() != Archetype::Hidden {
            assert_eq!(n1.group_pos, n2.group_pos);
        }
    }
}

/// Every position is finite after sorting, inputs/outputs sit in their columns, and each hidden
/// node of a group lies inside the box `OptimizeHiddenPlacement` rescales into
/// (`[0.15, 0.85] x [0.05, 0.95]`, up to rounding).
#[test]
fn sorted_positions_are_finite_and_hidden_nodes_stay_in_the_box() {
    for seed in 0..20u64 {
        for iterations in [1usize, 3, 8] {
            let layout = sorted_rich(seed, iterations);
            assert!(layout.groups.len() >= 3);
            for group in &layout.groups {
                for &id in &group.nodes {
                    let node = &layout.nodes[id];
                    let p = node.group_pos;
                    assert!(p.x.is_finite() && p.y.is_finite(), "seed {seed}: {p:?}");
                    match node.archetype() {
                        Archetype::Input => assert_eq!(p.x, 0.0),
                        Archetype::Output if !group.is_inputless_outputs_group => {
                            assert_eq!(p.x, 1.0)
                        }
                        Archetype::Output => assert!(p.x > 0.0 && p.x <= 1.0),
                        Archetype::Hidden => {
                            assert!((0.15 - 1e-4..=0.85 + 1e-4).contains(&p.x), "{p:?}");
                            assert!((0.05 - 1e-4..=0.95 + 1e-4).contains(&p.y), "{p:?}");
                        }
                    }
                    if node.archetype() != Archetype::Hidden {
                        assert!((0.0..=1.0).contains(&p.y), "{p:?}");
                    }
                }
            }
        }
    }
}

/// The group structure of the rich graph, traced by hand: the connected component of inputs
/// {0,1,2} reaching outputs {33, 34}; input 7 -> hidden 52 -> output 38 (disabled first synapse,
/// still a group); and the inputless group of the outputs 41 and 42.
#[test]
fn rich_graph_has_the_expected_groups() {
    let (nodes, syns) = rich_graph();
    let layout = make_groups(&nodes, &syns);
    assert_eq!(layout.groups.len(), 3);
    assert!(layout.groups[2].is_inputless_outputs_group);
    assert_eq!(
        indices(&layout, &layout.groups[2].outputs),
        vec![O0 + 8, O0 + 9]
    );
    let main = &layout.groups[0];
    assert_eq!(indices(&layout, &main.inputs), vec![0, 1, 2]);
    assert_eq!(indices(&layout, &main.outputs), vec![O0, O0 + 1]);
    let mut hidden = indices(&layout, &main.hidden);
    hidden.sort_unstable();
    assert_eq!(hidden, vec![H0, H0 + 1, H0 + 2, H0 + 3]);
    let side = &layout.groups[1];
    assert_eq!(indices(&layout, &side.inputs), vec![7]);
    assert_eq!(indices(&layout, &side.outputs), vec![O0 + 5]);
    assert_eq!(indices(&layout, &side.hidden), vec![H0 + 4]);
    // the heavier group sorts first when ordered by weight
    assert!(main.weight() > side.weight());
    assert!(side.weight() > layout.groups[2].weight());
    assert_eq!(layout.groups_by_descending_weight(), vec![0, 1, 2]);
}

#[test]
fn groups_by_descending_weight_is_stable_for_equal_weights() {
    let layout = make_groups(
        &base_nodes(0),
        &[syn(0, O0, 1.0), syn(1, O0 + 1, 1.0), syn(2, O0 + 2, 1.0)],
    );
    assert_eq!(layout.groups_by_descending_weight(), vec![0, 1, 2]);
}

// ---------------------------------------------------------------------------------------------
// OptimizeHiddenPlacement
// ---------------------------------------------------------------------------------------------

/// One hidden node H between I=(0, .5) and O=(1, .5), H=(.5, .3), both weights 1.5, dT = 1,
/// push scalar 0. By hand: num2 = 2 * sqrt(1.5 + 2.5) = 4, so num4 = 0.5 for both synapses.
///   I->H: val2 = H - I = (.5, -.2): vel -= -0.5 * val2 = (.25, -.1); acc -= 0.5 * unit(val2)
///   H->O: val2 = O - H = (.5, .2):  vel += -0.5 * val2 = (-.25, -.1); acc += 0.5 * unit(val2)
///   vel = (0, -.2), acc = (0, 0.5 * 2 * .2 / .5385) = (0, .3714)
///   tempPos = H - 1 * vel / |acc| = (.5, .3 + .5385) = (.5, .8385), inside the box already.
#[test]
fn single_hidden_node_step_matches_the_hand_computation() {
    let mut layout = make_groups(&base_nodes(1), &[syn(0, H0, 1.5), syn(H0, O0, 1.5)]);
    set_pos(&mut layout, 0, Vec2::new(0.0, 0.5));
    set_pos(&mut layout, O0, Vec2::new(1.0, 0.5));
    set_pos(&mut layout, H0, Vec2::new(0.5, 0.3));
    layout.optimize_hidden_placement(0, 1.0, 0.0);
    let p = pos(&layout, H0);
    assert_close(p.x, 0.5, 1e-5);
    assert_close(p.y, 0.838_516, 1e-4);
    // inputs and outputs are never moved by the optimiser
    assert_eq!(pos(&layout, 0), Vec2::new(0.0, 0.5));
    assert_eq!(pos(&layout, O0), Vec2::new(1.0, 0.5));
}

/// Same setup as above with a huge step (dT = 5): the node is thrown to y = .3 + 5 * .5385,
/// far above the box, so it becomes the maximum of the set and the final rescale
/// `(tempPos - min) / (max - min) * (hi - lo) + lo` maps it exactly onto the top edge, y = 0.95
/// (x = .5 stays inside and is mapped to itself).
#[test]
fn rescale_maps_the_extreme_node_onto_the_box_edge() {
    let mut layout = make_groups(&base_nodes(1), &[syn(0, H0, 1.5), syn(H0, O0, 1.5)]);
    set_pos(&mut layout, 0, Vec2::new(0.0, 0.5));
    set_pos(&mut layout, O0, Vec2::new(1.0, 0.5));
    set_pos(&mut layout, H0, Vec2::new(0.5, 0.3));
    layout.optimize_hidden_placement(0, 5.0, 0.0);
    let p = pos(&layout, H0);
    assert_close(p.x, 0.5, 1e-5);
    assert_close(p.y, 0.95, 1e-5);
}

/// With two hidden nodes and every force switched on, a large step still ends inside the box.
#[test]
fn several_hidden_nodes_always_end_inside_the_box() {
    let syns = [syn(0, H0, 1.0), syn(H0, H0 + 1, 1.0), syn(H0 + 1, O0, 1.0)];
    for dt in [0.01f32, 0.5, 1.0, 5.0, 50.0] {
        let mut layout = make_groups(&base_nodes(2), &syns);
        for (index, p) in [(0, Vec2::new(0.0, 0.5)), (O0, Vec2::new(1.0, 0.5))] {
            set_pos(&mut layout, index, p);
        }
        set_pos(&mut layout, H0, Vec2::new(0.3, 0.2));
        set_pos(&mut layout, H0 + 1, Vec2::new(0.7, 0.8));
        layout.optimize_hidden_placement(0, dt, 1.0);
        for h in [H0, H0 + 1] {
            let p = pos(&layout, h);
            assert!((0.15 - 1e-4..=0.85 + 1e-4).contains(&p.x), "dt {dt}: {p:?}");
            assert!((0.05 - 1e-4..=0.95 + 1e-4).contains(&p.y), "dt {dt}: {p:?}");
        }
    }
}

#[test]
fn optimizing_a_group_without_hidden_nodes_changes_nothing() {
    let mut layout = make_groups(&base_nodes(0), &[syn(0, O0, 1.0)]);
    layout.sort(2, None, &mut Rng::new(9));
    let before = bits(&layout);
    layout.optimize_hidden_placement(0, 1.0, 1.0);
    layout.optimize_hidden_placement(99, 1.0, 1.0); // out of range: ignored
    assert_eq!(before, bits(&layout));
}

// ---------------------------------------------------------------------------------------------
// (8): empty input
// ---------------------------------------------------------------------------------------------

#[test]
fn empty_graph_has_no_groups() {
    let layout = make_groups(&[], &[]);
    assert!(layout.groups.is_empty());
    assert!(layout.used_indices_of_groups().is_empty());
    assert!(layout.groups_by_descending_weight().is_empty());

    // a full node list with no synapses and no active outputs is empty too
    let mut layout = make_groups(&base_nodes(4), &[]);
    assert!(layout.groups.is_empty());
    assert!(layout.nodes.is_empty());
    layout.sort(3, None, &mut Rng::new(1));
    layout.optimize_hidden_placement(0, 1.0, 1.0);
    layout.set_start_end(0, 1.0, 2.0);
    assert!(layout.groups.is_empty());
}

/// The C# would throw on `nodes[index]`; the port answers with an empty layout.
#[test]
fn synapse_outside_the_node_list_gives_an_empty_layout() {
    let layout = make_groups(&base_nodes(0), &[syn(0, 500, 1.0)]);
    assert!(layout.groups.is_empty());
    let layout = make_groups(&base_nodes(0), &[syn(0, O0, 1.0), syn(9999, O0, 1.0)]);
    assert!(layout.groups.is_empty());
    assert_eq!(layout.node_id_by_index(0), None);
}

// ---------------------------------------------------------------------------------------------
// (9): the MaxNodePath cut-off
// ---------------------------------------------------------------------------------------------

/// Input 0 fans out to `fan` hidden nodes that all feed output 33. The walk from the input forks
/// once per synapse, from the last one down to the first, but only while the number of live
/// paths is *not more than* 250 before the fork: forks for synapses `fan-1 ..= fan-250` are made
/// (250 forks), the rest are skipped, and the original path takes synapse 0. So at most 251
/// branches are explored, always including synapse 0 and the highest-numbered ones. The reverse
/// walk from the output is symmetric. Each explored branch finishes on the very next step.
fn fan_out(fan: usize) -> Layout {
    let mut syns: Vec<SortSynapse> = (0..fan).map(|j| syn(0, H0 + j, 1.0)).collect();
    syns.extend((0..fan).map(|j| syn(H0 + j, O0, 1.0)));
    make_groups(&base_nodes(fan), &syns)
}

fn explored(layout: &Layout) -> Vec<usize> {
    indices(layout, &layout.groups[0].hidden)
        .into_iter()
        .map(|i| i - H0)
        .collect()
}

#[test]
fn fan_out_larger_than_the_cap_explores_exactly_251_branches() {
    let layout = fan_out(300);
    assert_eq!(layout.groups.len(), 1);
    let g = &layout.groups[0];
    assert_eq!(indices(&layout, &g.inputs), vec![0]);
    assert_eq!(indices(&layout, &g.outputs), vec![O0]);
    // Finished order: the original (synapse 0) first, then the forks, last created first.
    let mut expected = vec![0usize];
    expected.extend((50..300).rev());
    assert_eq!(explored(&layout), expected);
    assert_eq!(g.hidden.len(), 251);
    // 251 forward + 251 reverse paths, each through one hidden node and two synapses.
    assert_eq!(layout.paths.len(), 502);
    assert_eq!(g.connections.len(), 502);
    assert_eq!(layout.used_indices_of_groups().len(), 253);
    // the skipped hidden nodes (synapses 1..=49) never join a group
    assert!(!layout.used_indices_of_groups().contains(&(H0 + 1)));
    assert!(!layout.used_indices_of_groups().contains(&(H0 + 49)));
    assert!(layout.used_indices_of_groups().contains(&(H0 + 50)));
}

/// 251 branches is exactly what the cap allows: nothing is dropped. At 252 the single synapse
/// number 1 (the last fork that would have been made) is skipped.
#[test]
fn the_cap_is_a_strict_greater_than_on_the_number_of_live_paths() {
    let all = fan_out(251);
    assert_eq!(explored(&all).len(), 251);
    let mut sorted = explored(&all);
    sorted.sort_unstable();
    assert_eq!(sorted, (0..251).collect::<Vec<_>>());

    let one_over = fan_out(252);
    let mut sorted = explored(&one_over);
    sorted.sort_unstable();
    let mut expected: Vec<usize> = (0..252).collect();
    expected.retain(|&j| j != 1);
    assert_eq!(sorted, expected);
}

/// A dense layered network has astronomically many paths; the walk must still end quickly and
/// produce a sane group.
#[test]
fn dense_layered_network_terminates() {
    let layers = 7usize;
    let width = 6usize;
    let mut syns = Vec::new();
    let hidden = |layer: usize, k: usize| H0 + layer * width + k;
    for k in 0..width {
        syns.push(syn(0, hidden(0, k), 1.0 + k as f32 * 0.1));
    }
    for layer in 0..layers - 1 {
        for a in 0..width {
            for b in 0..width {
                syns.push(syn(
                    hidden(layer, a),
                    hidden(layer + 1, b),
                    0.5 + (a + b) as f32 * 0.05,
                ));
            }
        }
    }
    for k in 0..width {
        syns.push(syn(hidden(layers - 1, k), O0, 0.9));
    }
    let mut layout = make_groups(&base_nodes(layers * width), &syns);
    assert_eq!(layout.groups.len(), 1);
    let g = &layout.groups[0];
    assert_eq!(indices(&layout, &g.inputs), vec![0]);
    assert_eq!(indices(&layout, &g.outputs), vec![O0]);
    assert!(!g.hidden.is_empty() && g.hidden.len() <= layers * width);
    layout.sort(3, None, &mut Rng::new(4));
    for &id in &layout.groups[0].nodes {
        let p = layout.nodes[id].group_pos;
        assert!(p.x.is_finite() && p.y.is_finite(), "{p:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// Cycles and blocked paths
// ---------------------------------------------------------------------------------------------

/// 0 -(2)-> 48, 48 -(3)-> 48 (self loop), 48 -(5)-> 33. Traced by hand (see the comments in
/// `path.rs`'s tests for the walk rules):
///   forward  F1: 0,48,33 (acc 10)             done        F2: 0,48,48,33 (acc 30, recursivity 1) done
///   forward  P : 0,48,48,48 (acc 18)          blocked
///   reverse  R : 33,48,0 (acc 10)             done        S: 33,48,48,0 (acc 30, recursivity 1)  done
///   reverse  U : 33,48,48,48 (acc 45)         blocked
/// F1 and F2 collapse into one affinity 10 + sqrt(30), R and S into another. Hidden affinities
/// of 48 are the running means of the four done paths.
#[test]
fn self_loop_walks_are_blocked_and_merged_as_traced() {
    let layout = make_groups(
        &base_nodes(1),
        &[syn(0, H0, 2.0), syn(H0, H0, 3.0), syn(H0, O0, 5.0)],
    );
    assert_eq!(layout.groups.len(), 1);
    let g = &layout.groups[0];
    assert_eq!(indices(&layout, &g.hidden), vec![H0]);
    assert_eq!(g.connections, vec![0, 2, 1]);
    assert_eq!(layout.paths.len(), 6);
    assert_eq!(
        layout
            .paths
            .iter()
            .map(|p| (p.done, p.blocked, p.total_depth))
            .collect::<Vec<_>>(),
        vec![
            (true, false, 2),
            (true, false, 3),
            (false, true, 2),
            (true, false, 2),
            (false, true, 2),
            (true, false, 3),
        ]
    );
    assert_eq!(layout.affinities.len(), 2);
    let expected = 10.0 + 30.0f32.sqrt();
    assert_close(layout.affinities[0].affinity, expected, 1e-4);
    assert_close(layout.affinities[1].affinity, expected, 1e-4);
    assert_eq!(layout.affinities[0].paths, vec![0, 1]);
    assert_eq!(layout.affinities[1].paths, vec![3, 5]);

    let entries = &layout.nodes[id_of(&layout, H0)].hidden_affinities_to_inputs_outputs;
    assert_eq!(entries.len(), 2);
    let (input, a) = entries[0];
    let (output, b) = entries[1];
    assert_eq!((input, output), (id_of(&layout, 0), id_of(&layout, O0)));
    assert_eq!((a.n, b.n), (4, 4));
    assert_close(a.affinity, 3.0, 1e-5);
    assert_close(a.closeness, 1.5, 1e-6);
    assert_close(b.affinity, 7.5, 1e-4);
    assert_close(b.closeness, 1.5, 1e-6);
}

/// A hidden cycle (48 <-> 49) that never reaches an output is only on a blocked walk. Nothing
/// groups it through a done path, so the blocked walk's affinity is attached to the group that
/// holds its source (the input 0), adding the two hidden nodes and the cycle's synapses.
#[test]
fn hidden_cycle_without_exit_is_attached_through_its_blocked_path() {
    let syns = [
        syn(0, O0, 1.0),
        syn(0, H0, 2.0),
        syn(H0, H0 + 1, 3.0),
        syn(H0 + 1, H0, 4.0),
    ];
    let mut layout = make_groups(&base_nodes(2), &syns);
    assert_eq!(layout.groups.len(), 1);
    let g = &layout.groups[0];
    assert_eq!(indices(&layout, &g.inputs), vec![0]);
    assert_eq!(indices(&layout, &g.outputs), vec![O0]);
    assert_eq!(indices(&layout, &g.hidden), vec![H0, H0 + 1]);
    assert_eq!(g.connections, vec![0, 1, 2, 3]);
    assert_eq!(layout.affinities.len(), 3);
    let blocked = &layout.affinities[2];
    assert_eq!(blocked.node_in, Some(id_of(&layout, 0)));
    assert_eq!(blocked.node_out, None);
    // accumulated 2 * 3 * 4 * 3 * 4 = 288 with recursivity 2: 288^(1/3)
    assert_close(blocked.affinity, 288.0f32.cbrt(), 1e-4);
    assert_eq!(layout.used_indices_of_groups(), vec![0, O0, H0, H0 + 1]);
    for h in [H0, H0 + 1] {
        assert!(
            layout.nodes[id_of(&layout, h)]
                .hidden_affinities_to_inputs_outputs
                .is_empty()
        );
    }
    // Hidden nodes with no affinities are placed at random inside a small disc and then
    // optimised; the result must be finite and inside the box.
    layout.sort(3, None, &mut Rng::new(21));
    for h in [H0, H0 + 1] {
        let p = pos(&layout, h);
        assert!((0.15 - 1e-4..=0.85 + 1e-4).contains(&p.x), "{p:?}");
        assert!((0.05 - 1e-4..=0.95 + 1e-4).contains(&p.y), "{p:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// API conveniences and plumbing
// ---------------------------------------------------------------------------------------------

#[test]
fn synapse_ids_are_positions_in_the_input_slice() {
    let syns = [syn(1, O0 + 1, 0.5), syn(0, O0, 1.5), syn(0, O0 + 1, -2.0)];
    let layout = make_groups(&base_nodes(0), &syns);
    assert_eq!(layout.connections.len(), 3);
    for (k, s) in syns.iter().enumerate() {
        assert_eq!(layout.connections[k].connection, *s);
        assert_eq!(layout.connections[k].index_in(), s.node_in);
        assert_eq!(layout.connections[k].index_out(), s.node_out);
    }
    assert_eq!(layout.connections[2].value(), -2.0);
}

#[test]
fn set_start_end_updates_the_group_band() {
    let mut layout = make_groups(&base_nodes(0), &[syn(0, O0, 1.0), syn(1, O0 + 1, 1.0)]);
    layout.set_start_end(1, -10.0, 33.0);
    assert_eq!(
        (layout.groups[1].group_start, layout.groups[1].group_height),
        (-10.0, 33.0)
    );
    assert_eq!(
        (layout.groups[0].group_start, layout.groups[0].group_height),
        (0.0, 0.0)
    );
    layout.set_start_end(7, 1.0, 1.0); // out of range: ignored
    assert_eq!(
        layout.group_max_depth(0),
        0,
        "the C# never fills NodeGroup.paths"
    );
    assert_eq!(layout.group_max_depth(9), 0);
}

/// What `PlaceNodesAccordingToSort` does with the result: heaviest groups first, a band per
/// group, `panelPos = (groupPos.x, (start - height * groupPos.y) / step)`.
#[test]
fn consumer_loop_compiles_and_reads_naturally() {
    let (nodes, syns) = rich_graph();
    let mut layout = make_groups(&nodes, &syns);
    layout.sort(3, None, &mut Rng::new(8));
    let (height, step_i, step_o) = (600.0f32, -40.0f32, -40.0f32);
    let total: f32 = layout.groups.iter().map(|g| g.weight()).sum();
    let mut start = 0.0f32;
    for group_index in layout.groups_by_descending_weight() {
        let band = height * layout.groups[group_index].weight() / total;
        layout.set_start_end(group_index, start, band);
        let ids = layout.groups[group_index].nodes.clone();
        for id in ids {
            let node = &mut layout.nodes[id];
            let step = if node.archetype() == Archetype::Input {
                step_i
            } else {
                step_o
            };
            node.panel_pos = Vec2::new(node.group_pos.x, (start - band * node.group_pos.y) / step);
            assert!(node.panel_pos.y.is_finite());
        }
        start -= band + 10.0;
    }
    assert!(layout.groups[0].group_height > 0.0);
}

// ---------------------------------------------------------------------------------------------
// Robustness on random brains
// ---------------------------------------------------------------------------------------------

fn pick(rng: &mut Rng, n: usize) -> usize {
    (rng.next_u64() % n as u64) as usize
}

/// A random brain. `valid` brains follow the NEAT rules of the game (synapses run from an
/// input or hidden node to an output or hidden node); the others connect arbitrary nodes, which
/// also produces synapses into inputs and out of outputs, self loops and duplicates.
fn random_brain(
    rng: &mut Rng,
    n_hidden: usize,
    n_synapses: usize,
    valid: bool,
) -> (Vec<SortNode>, Vec<SortSynapse>) {
    let mut nodes = base_nodes(n_hidden);
    for node in &mut nodes[O0..H0] {
        node.is_sigmoid = pick(rng, 3) == 0; // some inputless outputs
    }
    let total = nodes.len();
    let mut syns = Vec::new();
    for k in 0..n_synapses {
        let (a, b) = if valid {
            let a = pick(rng, N_INPUTS + n_hidden);
            let a = if a < N_INPUTS { a } else { H0 + (a - N_INPUTS) };
            (a, O0 + pick(rng, N_OUTPUTS + n_hidden))
        } else {
            (pick(rng, total), pick(rng, total))
        };
        let weight = (rng.range(-3.0, 3.0) * 1000.0).round() / 1000.0;
        let weight = if weight == 0.0 { 0.5 } else { weight };
        syns.push(SortSynapse::new(k as i64, a, b, weight, pick(rng, 5) != 0));
    }
    (nodes, syns)
}

fn check_structure(layout: &Layout) {
    for group in &layout.groups {
        let mut concat = group.inputs.clone();
        concat.extend(&group.outputs);
        concat.extend(&group.hidden);
        assert_eq!(group.nodes, concat);
        assert!(group.nodes.iter().all(|&n| n < layout.nodes.len()));
        assert!(
            group
                .connections
                .iter()
                .all(|&c| c < layout.connections.len())
        );
        assert!(
            group
                .input_output_affinities
                .iter()
                .all(|&a| a < layout.affinities.len())
        );
    }
    for affinity in &layout.affinities {
        assert!(affinity.paths.iter().all(|&p| p < layout.paths.len()));
        assert!(
            affinity
                .hidden_nodes
                .iter()
                .all(|&n| n < layout.nodes.len())
        );
        assert!(
            affinity
                .connections
                .iter()
                .all(|&c| c < layout.connections.len())
        );
    }
    let used = layout.used_indices_of_groups();
    let mut unique = used.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), used.len());
}

#[test]
fn random_brains_never_panic_and_keep_their_invariants() {
    let mut rng = Rng::new(0x0B1B_17E5);
    let mut layouts_with_hidden = 0;
    for case in 0..60 {
        let valid = case % 3 != 2;
        let n_hidden = pick(&mut rng, 25);
        let n_synapses = 1 + pick(&mut rng, 140);
        let (nodes, syns) = random_brain(&mut rng, n_hidden, n_synapses, valid);
        let mut layout = make_groups(&nodes, &syns);
        check_structure(&layout);
        if valid {
            // Strict NEAT brains: archetypes are what the lists say, and every connected input
            // or output node is shown in some group.
            let used = layout.used_indices_of_groups();
            for s in &syns {
                for index in [s.node_in, s.node_out] {
                    if index < H0 {
                        assert!(used.contains(&index), "case {case}: node {index} missing");
                    }
                }
            }
            for group in &layout.groups {
                for &n in &group.inputs {
                    assert_eq!(layout.nodes[n].archetype(), Archetype::Input);
                }
                for &n in &group.outputs {
                    assert_eq!(layout.nodes[n].archetype(), Archetype::Output);
                }
                for &n in &group.hidden {
                    assert_eq!(layout.nodes[n].archetype(), Archetype::Hidden);
                }
            }
        }
        layouts_with_hidden += layout.groups.iter().any(|g| !g.hidden.is_empty()) as usize;
        layout.sort(3, None, &mut Rng::new(case));
        check_structure(&layout);
        // a few extra relaxation steps, as SortHiddenOnce does each frame
        for gi in 0..layout.groups.len() {
            layout.optimize_hidden_placement(gi, 1.0 / 60.0, 1.0);
        }
        let order = layout.groups_by_descending_weight();
        assert_eq!(order.len(), layout.groups.len());
    }
    assert!(
        layouts_with_hidden > 20,
        "only {layouts_with_hidden} layouts had hidden nodes"
    );
}

/// Zero, NaN and infinite weights make the C# produce NaN/inf positions (`0 / 0` affinities, ...);
/// the port reproduces float semantics and must neither panic nor hang on them.
#[test]
fn degenerate_weights_do_not_panic() {
    let weights = [
        0.0f32,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::MIN_POSITIVE,
        1e30,
    ];
    for (k, &w) in weights.iter().enumerate() {
        let syns = [
            syn(0, H0, w),
            syn(1, H0, 1.0),
            syn(H0, H0 + 1, w),
            syn(H0 + 1, O0, 2.0),
            syn(H0, O0 + 1, w),
            syn(H0 + 1, H0, 0.5),
        ];
        let mut layout = make_groups(&base_nodes(2), &syns);
        check_structure(&layout);
        layout.sort(3, None, &mut Rng::new(k as u64));
        layout.optimize_hidden_placement(0, 0.5, 1.0);
        check_structure(&layout);
        assert_eq!(
            layout.groups_by_descending_weight().len(),
            layout.groups.len()
        );
    }
}

/// With strictly positive finite weights on NEAT-shaped brains, sorting leaves every position finite.
#[test]
fn sorted_random_brains_have_finite_positions() {
    let mut rng = Rng::new(31337);
    let mut checked = 0;
    for case in 0..40 {
        let n_hidden = 1 + pick(&mut rng, 12);
        let n_synapses = 10 + pick(&mut rng, 60);
        let (nodes, syns) = random_brain(&mut rng, n_hidden, n_synapses, true);
        let mut layout = make_groups(&nodes, &syns);
        layout.sort(3, None, &mut Rng::new(case));
        for group in &layout.groups {
            for &id in &group.nodes {
                let p = layout.nodes[id].group_pos;
                assert!(p.x.is_finite() && p.y.is_finite(), "case {case}: {p:?}");
                checked += 1;
            }
        }
    }
    assert!(checked > 200);
}

// ---------------------------------------------------------------------------------------------
// Cross-check against a second, literal transliteration of OptimizeHiddenPlacement
// ---------------------------------------------------------------------------------------------

/// A deliberately naive f64 re-statement of the C# `OptimizeHiddenPlacement`, written straight
/// from the decompiled text over plain arrays (no arenas, no helper types), used to cross-check
/// the f32 port on realistic layouts. Differences are rounding only (f32 vs f64).
mod reference {
    #[derive(Clone, Copy, Debug)]
    pub struct V(pub f64, pub f64);

    fn add(a: V, b: V) -> V {
        V(a.0 + b.0, a.1 + b.1)
    }
    fn sub(a: V, b: V) -> V {
        V(a.0 - b.0, a.1 - b.1)
    }
    fn mul(a: V, s: f64) -> V {
        V(a.0 * s, a.1 * s)
    }
    fn dot(a: V, b: V) -> f64 {
        a.0 * b.0 + a.1 * b.1
    }
    fn mag(a: V) -> f64 {
        (a.0 * a.0 + a.1 * a.1).sqrt()
    }
    fn norm(a: V) -> V {
        let m = mag(a);
        if m > 1e-5 {
            V(a.0 / m, a.1 / m)
        } else {
            V(0.0, 0.0)
        }
    }
    fn sign(x: f64) -> f64 {
        if x >= 0.0 { 1.0 } else { -1.0 }
    }

    pub struct Group<'a> {
        /// group position of every node, indexed by node id
        pub pos: &'a [V],
        pub is_hidden: &'a [bool],
        pub inputs: usize,
        pub outputs: usize,
        /// `group.nodes` (ids, in order)
        pub nodes: &'a [usize],
        pub hidden: &'a [usize],
        /// `group.connections`: (node_in, node_out, weight, enabled)
        pub connections: &'a [(usize, usize, f64, bool)],
    }

    /// Returns the new group position of every hidden node (other entries are copied).
    pub fn optimize(g: &Group, dt: f64, scalar: f64) -> Vec<V> {
        let n = g.pos.len();
        let mut vel = vec![V(0.0, 0.0); n];
        let mut acc = vec![V(0.0, 0.0); n];
        let mut temp = g.pos.to_vec();
        let disabled = 0.5;
        let num = scalar / g.connections.len() as f64;
        let mut num2 = 0.0;
        for &(a, b, w, en) in g.connections {
            if g.is_hidden[a] || g.is_hidden[b] {
                num2 += (w.abs().min(10.0) + 2.5).sqrt() * if en { 1.0 } else { disabled };
            }
        }
        let num3 = g.inputs.max(g.outputs) as f64;
        for &(a, b, w, en) in g.connections {
            let num4 = 1.0 * (w.abs() + 2.5).sqrt() * if en { 1.0 } else { disabled } / num2;
            let val2 = sub(g.pos[b], g.pos[a]);
            let rotated = V(-val2.1, val2.0); // Rotate(pi/2)
            let normalized = norm(rotated);
            if g.is_hidden[a] {
                vel[a] = add(vel[a], mul(val2, -num4));
                acc[a] = add(acc[a], mul(norm(val2), num4));
            }
            if g.is_hidden[b] {
                vel[b] = sub(vel[b], mul(val2, -num4));
                acc[b] = sub(acc[b], mul(norm(val2), num4));
            }
            for &h in g.hidden {
                if h != a && h != b {
                    let mid = V(
                        (g.pos[a].0 + g.pos[b].0) / 2.0,
                        (g.pos[a].1 + g.pos[b].1) / 2.0,
                    );
                    let val3 = sub(g.pos[h], mid);
                    let num5 = 2.0 * (dot(val3, norm(val2)) / (mag(val2) + 0.0001)).abs();
                    let num6 = dot(val3, normalized) / mag(val3);
                    let num7 = num6.abs() * f64::max(0.5, 2.0 * num5 - 0.5);
                    let num8 = -num3 * (-num3 * num7).exp();
                    let num9 = num3 * num3 * (-num3 * num7).exp();
                    vel[h] = add(vel[h], mul(normalized, sign(num6) * 1.0 * num * num8));
                    acc[h] = add(acc[h], mul(normalized, sign(num6) * 1.0 * num * num9));
                }
            }
        }
        if g.hidden.len() > 1 {
            let num10 = 1.0 / ((g.hidden.len() * g.hidden.len()) as f64 - 1.0);
            let mut list = g.nodes.to_vec();
            for &node in g.nodes {
                if let Some(p) = list.iter().position(|&x| x == node) {
                    list.remove(p);
                }
                for &item in &list {
                    let d = sub(g.pos[item], g.pos[node]);
                    let m = mag(d);
                    let num11 = -num3 * (-num3 * m).exp();
                    let num12 = num3 * num3 * (-num3 * m).exp();
                    let (flag, flag2) = (g.is_hidden[node], g.is_hidden[item]);
                    if flag {
                        let k = if flag2 { num10 } else { 0.0 };
                        vel[node] = sub(vel[node], mul(norm(d), num11 * k));
                        acc[node] = sub(acc[node], mul(norm(d), num12 * k));
                    }
                    if flag2 {
                        let k = if flag { num10 } else { 0.0 };
                        vel[item] = add(vel[item], mul(norm(d), num11 * k));
                        acc[item] = add(acc[item], mul(norm(d), num12 * k));
                    }
                }
            }
        }
        let (lo, hi) = (V(0.15, 0.05), V(0.85, 0.95));
        let (mut min, mut max) = (lo, hi);
        for &h in g.hidden {
            let step = mul(vel[h], dt / mag(acc[h]));
            temp[h] = sub(temp[h], step);
            min = V(min.0.min(temp[h].0), min.1.min(temp[h].1));
            max = V(max.0.max(temp[h].0), max.1.max(temp[h].1));
        }
        let mut out = g.pos.to_vec();
        for &h in g.hidden {
            out[h] = V(
                (temp[h].0 - min.0) / (max.0 - min.0) * (hi.0 - lo.0) + lo.0,
                (temp[h].1 - min.1) / (max.1 - min.1) * (hi.1 - lo.1) + lo.1,
            );
        }
        out
    }
}

/// On sorted layouts of the rich graph, a single optimiser step with assorted (dT, push)
/// values agrees with the naive f64 version to rounding error.
#[test]
fn optimiser_step_agrees_with_the_naive_reference() {
    let (nodes, syns) = rich_graph();
    let mut checked = 0;
    let mut max_err = 0.0f64;
    for seed in 0..12u64 {
        let mut layout = make_groups(&nodes, &syns);
        layout.sort(2, None, &mut Rng::new(seed));
        for (dt, scalar) in [(1.0f32, 1.0f32), (0.37, 0.8), (0.1, 0.0), (0.02, 0.003)] {
            for gi in 0..layout.groups.len() {
                let group = layout.groups[gi].clone();
                if group.hidden.is_empty() {
                    continue;
                }
                let pos: Vec<reference::V> = layout
                    .nodes
                    .iter()
                    .map(|n| reference::V(f64::from(n.group_pos.x), f64::from(n.group_pos.y)))
                    .collect();
                let is_hidden: Vec<bool> = layout
                    .nodes
                    .iter()
                    .map(|n| n.archetype() == Archetype::Hidden)
                    .collect();
                let connections: Vec<(usize, usize, f64, bool)> = group
                    .connections
                    .iter()
                    .map(|&c| {
                        let c = &layout.connections[c];
                        (c.node_in, c.node_out, f64::from(c.value()), c.enabled())
                    })
                    .collect();
                let expected = reference::optimize(
                    &reference::Group {
                        pos: &pos,
                        is_hidden: &is_hidden,
                        inputs: group.inputs.len(),
                        outputs: group.outputs.len(),
                        nodes: &group.nodes,
                        hidden: &group.hidden,
                        connections: &connections,
                    },
                    f64::from(dt),
                    f64::from(scalar),
                );
                let mut stepped = layout.clone();
                stepped.optimize_hidden_placement(gi, dt, scalar);
                for &h in &group.hidden {
                    let got = stepped.nodes[h].group_pos;
                    let want = expected[h];
                    let err = (f64::from(got.x) - want.0)
                        .abs()
                        .max((f64::from(got.y) - want.1).abs());
                    // Measured: ~2e-6 (f32 rounding) over these 240 positions.
                    assert!(
                        err < 1e-4,
                        "seed {seed} group {gi} dt {dt} push {scalar}: {got:?} vs {want:?}"
                    );
                    max_err = max_err.max(err);
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 100, "only {checked} node positions were compared");
    assert!(max_err < 1e-4, "max deviation {max_err}");
}

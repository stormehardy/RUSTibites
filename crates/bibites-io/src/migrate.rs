//! Version migrations for genomes and brains.
//!
//! Ports of the game's `BibiteUpdater.UpdateGenesToPresentVersion` and
//! `BrainUpdater.UpdateElementsFromVersion` (`Here/ScriptHelpers/`), following
//! `docs/spec/04-formats-settings.md` sections 4.7/8.4. All thresholds are
//! **alpha** versions: a 4-int `new Version(0,6,0,15)` in C# means `0.6a15`.
//!
//! One deliberate deviation: the game's `0.6a13` step uses `Random.Range(1,14)`
//! to choose a replacement input for disabled synapses that originated from
//! node 0. That is non-deterministic; we derive a stable replacement from the
//! synapse's position instead (documented so round-trip output is reproducible).

use bibites_core::brain::{N_INPUTS, N_OUTPUTS, NodeRecord, NodeType, SynapseRecord};
use bibites_core::genome::{ALL_GENES, Gene, Genome, N_GENES};
use bibites_core::{Error, Result};

use crate::model::GenesRepr;
use crate::version::Version;

/// One structural node change (`NodeChanges` in `BrainUpdater`).
#[derive(Clone, Copy, Debug)]
struct NodeChanges {
    position: usize,
    amount: usize,
}

impl NodeChanges {
    const fn new(position: usize, amount: usize) -> Self {
        Self { position, amount }
    }
}

/// What a [`BrainChange`] does after its structural edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Update {
    None,
    /// 0.6a5: `EggProduction.baseActivation = 0.2`; mint innovations for
    /// synapses whose innovation is 0.
    EggProductionBias,
    /// 0.6a15: `Want2Eat` bias 0 becomes 2.5.
    WantToEatBias,
    /// 0.6a16: `Want2Eat` becomes `TanH` with affine bias/weight transform.
    WantToEatTanH,
    /// 0.6a13 (applied after all other changes): fold the enabled "Constant"
    /// input-0 synapses into their targets, disable the rest.
    RemoveConstantInput,
}

/// One entry of `BrainUpdater.VersionChanges` (spec 04 4.7).
#[derive(Clone, Copy, Debug)]
struct BrainChange {
    version: Version,
    added_inputs: &'static [NodeChanges],
    removed_inputs: &'static [NodeChanges],
    added_outputs: &'static [NodeChanges],
    removed_outputs: &'static [NodeChanges],
    update: Update,
}

impl BrainChange {
    fn any_changes(&self) -> bool {
        self.update != Update::None
            || !self.added_inputs.is_empty()
            || !self.removed_inputs.is_empty()
            || !self.added_outputs.is_empty()
            || !self.removed_outputs.is_empty()
    }
}

const fn v(major: u32, minor: u32, patch: u32, alpha: u32) -> Version {
    Version::new(major, minor, patch, 0, alpha)
}

/// `BrainUpdater.VersionChanges`, newest first (the game's list order).
const BRAIN_CHANGES: &[BrainChange] = &[
    BrainChange {
        version: v(0, 6, 2, 1),
        added_inputs: &[NodeChanges::new(5, 1)],
        removed_inputs: &[],
        added_outputs: &[],
        removed_outputs: &[],
        update: Update::None,
    },
    BrainChange {
        version: v(0, 6, 0, 18),
        added_inputs: &[],
        removed_inputs: &[NodeChanges::new(32, 1)],
        added_outputs: &[],
        removed_outputs: &[NodeChanges::new(15, 1)],
        update: Update::None,
    },
    BrainChange {
        version: v(0, 6, 0, 16),
        added_inputs: &[],
        removed_inputs: &[],
        added_outputs: &[],
        removed_outputs: &[],
        update: Update::WantToEatTanH,
    },
    BrainChange {
        version: v(0, 6, 0, 15),
        added_inputs: &[],
        removed_inputs: &[],
        added_outputs: &[],
        removed_outputs: &[],
        update: Update::WantToEatBias,
    },
    BrainChange {
        version: v(0, 6, 0, 13),
        added_inputs: &[],
        removed_inputs: &[NodeChanges::new(0, 1)],
        added_outputs: &[],
        removed_outputs: &[],
        update: Update::RemoveConstantInput,
    },
    BrainChange {
        version: v(0, 6, 0, 5),
        added_inputs: &[NodeChanges::new(8, 1)],
        removed_inputs: &[],
        added_outputs: &[NodeChanges::new(3, 1)],
        removed_outputs: &[],
        update: Update::EggProductionBias,
    },
    BrainChange {
        version: v(0, 4, 0, 3),
        added_inputs: &[NodeChanges::new(28, 3), NodeChanges::new(4, 1)],
        removed_inputs: &[],
        added_outputs: &[],
        removed_outputs: &[],
        update: Update::None,
    },
    BrainChange {
        version: v(0, 3, 0, 0),
        added_inputs: &[],
        removed_inputs: &[],
        added_outputs: &[],
        removed_outputs: &[],
        update: Update::None,
    },
];

/// Returns whether a brain from `from` needs any migration
/// (`BrainUpdater.ChangesSinceVersion`).
#[must_use]
pub fn brain_needs_migration(from: Version) -> bool {
    BRAIN_CHANGES
        .iter()
        .any(|change| change.version > from && change.any_changes())
}

/// Applies the game's brain migrations in place.
///
/// Returns `true` when at least one migration was applied (and therefore the
/// base-node type/description table must be normalised by the caller).
pub fn migrate_brain(
    from: Version,
    nodes: &mut Vec<NodeRecord>,
    synapses: &mut Vec<SynapseRecord>,
) -> Result<bool> {
    if !brain_needs_migration(from) {
        return Ok(false);
    }

    // ---- Pass 1: index map, newest change first (game lines 209-249) ------
    let mut index_map: Vec<i64> = (0..(N_INPUTS + N_OUTPUTS) as i64).collect();
    let mut num = N_INPUTS as i64;
    let mut num2 = N_OUTPUTS as i64;
    let mut num3 = 0i64;

    let mut newer: Vec<&BrainChange> = BRAIN_CHANGES
        .iter()
        .filter(|change| change.version > from && change.any_changes())
        .collect();
    newer.sort_by_key(|change| std::cmp::Reverse(change.version));

    for change in &newer {
        for node_change in change.added_outputs {
            remove_range(
                &mut index_map,
                (num + node_change.position as i64) as usize,
                node_change.amount,
            );
            num2 -= node_change.amount as i64;
            num3 += node_change.amount as i64;
        }
        for node_change in change.added_inputs {
            remove_range(&mut index_map, node_change.position, node_change.amount);
            num -= node_change.amount as i64;
            num3 += node_change.amount as i64;
        }
        for node_change in change.removed_inputs {
            for _ in 0..node_change.amount {
                index_map.insert(node_change.position, -1);
            }
            num += node_change.amount as i64;
            num3 -= node_change.amount as i64;
        }
        for node_change in change.removed_outputs {
            let at = (num + node_change.position as i64) as usize;
            for _ in 0..node_change.amount {
                index_map.insert(at, -1);
            }
            num2 += node_change.amount as i64;
            num3 -= node_change.amount as i64;
        }
    }

    let base_count = (num + num2) as usize;
    for hidden in base_count..nodes.len() {
        index_map.push(hidden as i64 + num3);
    }

    // ---- Pass 2: structural edits, oldest change first (game lines 250-289) --
    let mut oldest: Vec<&BrainChange> = newer.clone();
    oldest.sort_by_key(|change| change.version);
    let mut prog_index_map = index_map.clone();
    let mut pending_end_updates: Vec<Update> = Vec::new();

    for change in &oldest {
        let mut removed_outputs = change.removed_outputs.to_vec();
        removed_outputs.sort_by_key(|node_change| std::cmp::Reverse(node_change.position));
        for node_change in removed_outputs {
            let at = (num + node_change.position as i64) as usize;
            drain_range(nodes, at, node_change.amount)?;
            drain_range(&mut prog_index_map, at, node_change.amount)?;
            num2 -= node_change.amount as i64;
        }
        let mut removed_inputs = change.removed_inputs.to_vec();
        removed_inputs.sort_by_key(|node_change| std::cmp::Reverse(node_change.position));
        for node_change in removed_inputs {
            drain_range(nodes, node_change.position, node_change.amount)?;
            drain_range(
                &mut prog_index_map,
                node_change.position,
                node_change.amount,
            )?;
            num -= node_change.amount as i64;
        }
        let mut added_inputs = change.added_inputs.to_vec();
        added_inputs.sort_by_key(|node_change| node_change.position);
        for node_change in added_inputs {
            for offset in 0..node_change.amount {
                nodes.insert(node_change.position, placeholder_node());
                prog_index_map.insert(
                    node_change.position,
                    node_change.position as i64 + offset as i64,
                );
            }
            num += node_change.amount as i64;
        }
        let mut added_outputs = change.added_outputs.to_vec();
        added_outputs.sort_by_key(|node_change| node_change.position);
        for node_change in added_outputs {
            let at = (num + node_change.position as i64) as usize;
            for offset in 0..node_change.amount {
                nodes.insert(at, placeholder_node());
                prog_index_map.insert(at, at as i64 + offset as i64);
            }
            num2 += node_change.amount as i64;
        }

        match change.update {
            Update::None => {}
            Update::EggProductionBias => apply_egg_production_bias(nodes, synapses)?,
            Update::WantToEatBias => apply_want_to_eat_bias(nodes)?,
            Update::WantToEatTanH => apply_want_to_eat_tanh(nodes, synapses)?,
            Update::RemoveConstantInput => pending_end_updates.push(Update::RemoveConstantInput),
        }
    }

    // Game lines 294-300: `updateAtEnd` handlers (0.6a13), before reindexing.
    for update in pending_end_updates {
        if update == Update::RemoveConstantInput {
            apply_remove_constant_input(nodes, synapses, &index_map)?;
        }
    }

    // Game lines 301-314: reindex nodes and reset base innovations.
    for (index, node) in nodes.iter_mut().enumerate() {
        node.index = index as u32;
        if index < N_INPUTS + N_OUTPUTS {
            node.innovation = index as i64 + 1;
        } else {
            node.innovation += num3;
        }
    }

    // Game lines 290-293: count check.
    if num != N_INPUTS as i64 || num2 != N_OUTPUTS as i64 {
        return Err(Error::invalid_data(format!(
            "brain migration from {from} produced {num} inputs / {num2} outputs (expected {N_INPUTS}/{N_OUTPUTS})"
        )));
    }

    // Game lines 315-326: drop synapses to removed nodes and remap endpoints.
    let mut remapped = Vec::with_capacity(synapses.len());
    for mut synapse in synapses.drain(..) {
        let new_in = index_map
            .get(synapse.node_in as usize)
            .copied()
            .unwrap_or(-1);
        let new_out = index_map
            .get(synapse.node_out as usize)
            .copied()
            .unwrap_or(-1);
        if new_in < 0 || new_out < 0 {
            continue;
        }
        synapse.node_in = new_in as u32;
        synapse.node_out = new_out as u32;
        remapped.push(synapse);
    }
    *synapses = remapped;

    Ok(true)
}

fn remove_range(values: &mut Vec<i64>, at: usize, amount: usize) {
    if at <= values.len() {
        let end = (at + amount).min(values.len());
        values.drain(at..end);
    }
}

fn drain_range<T>(values: &mut Vec<T>, at: usize, amount: usize) -> Result<()> {
    if at + amount > values.len() {
        return Err(Error::invalid_data(format!(
            "brain migration node range {at}..{} is out of bounds ({} nodes)",
            at + amount,
            values.len()
        )));
    }
    values.drain(at..at + amount);
    Ok(())
}

fn placeholder_node() -> NodeRecord {
    NodeRecord {
        node_type: NodeType::Sigmoid,
        index: 0,
        innovation: 0,
        desc: None,
        base_activation: 0.0,
        value: 0.0,
        last_input: 0.0,
        last_output: 0.0,
    }
}

fn apply_egg_production_bias(
    nodes: &mut [NodeRecord],
    synapses: &mut [SynapseRecord],
) -> Result<()> {
    // Game `UpdateTo0_6_0_5`: node 37 (EggProduction) activation 0.2; synapses
    // with innovation 0 get a fresh per-(in,out) innovation starting at 1.
    let node = nodes.get_mut(37).ok_or_else(|| {
        Error::invalid_data("0.6a5 brain migration: node 37 (EggProduction) is missing")
    })?;
    node.base_activation = 0.2;
    let mut next = 1i64;
    for synapse in synapses.iter_mut() {
        if synapse.innovation == 0 {
            synapse.innovation = next;
            next += 1;
        }
    }
    Ok(())
}

fn apply_want_to_eat_bias(nodes: &mut [NodeRecord]) -> Result<()> {
    let node = nodes
        .get_mut(38)
        .ok_or_else(|| Error::invalid_data("0.6a15 brain migration: node 38 is missing"))?;
    if node.base_activation == 0.0 {
        node.base_activation = 2.5;
    }
    Ok(())
}

fn apply_want_to_eat_tanh(nodes: &mut [NodeRecord], synapses: &mut [SynapseRecord]) -> Result<()> {
    let node = nodes
        .get_mut(38)
        .ok_or_else(|| Error::invalid_data("0.6a16 brain migration: node 38 is missing"))?;
    node.node_type = NodeType::TanH;
    node.base_activation = 0.28 * node.base_activation + 0.528;
    for synapse in synapses.iter_mut() {
        if synapse.node_out == 38 {
            synapse.weight *= 0.28;
        }
    }
    Ok(())
}

fn apply_remove_constant_input(
    nodes: &mut [NodeRecord],
    synapses: &mut [SynapseRecord],
    index_map: &[i64],
) -> Result<()> {
    // Enabled synapses out of node 0 fold their weight into the target bias.
    for synapse in synapses.iter().filter(|s| s.node_in == 0 && s.enabled) {
        let target = index_map
            .get(synapse.node_out as usize)
            .copied()
            .unwrap_or(-1);
        if target >= 0 {
            if let Some(node) = nodes.get_mut(target as usize) {
                node.base_activation += synapse.weight;
            }
        }
    }
    // All remaining node-0 synapses are disabled and re-pointed to a
    // deterministic input in 1..=13 (the game uses Random.Range(1,14)).
    for (position, synapse) in synapses.iter_mut().enumerate() {
        if synapse.node_in == 0 {
            synapse.node_in = deterministic_input(position);
            synapse.enabled = false;
        }
    }
    // Mult nodes with activation 0 become 1.
    for node in nodes.iter_mut() {
        if node.node_type == NodeType::Mult && node.base_activation == 0.0 {
            node.base_activation = 1.0;
        }
    }
    Ok(())
}

fn deterministic_input(position: usize) -> u32 {
    // Game: Random.Range(1, 14) -> 1..=13. Deterministic substitute (documented).
    1 + (position as u32 % 13)
}

/// Normalises the first `N_INPUTS + N_OUTPUTS` nodes exactly like
/// `NEATBrain.SetBaseNeuronsTypeAndDesc` (game lines 1029-1056).
pub fn normalize_base_nodes(nodes: &mut [NodeRecord]) {
    use bibites_core::brain::{Input, Output};
    for (index, node) in nodes.iter_mut().take(N_INPUTS).enumerate() {
        if let Some(input) = Input::from_index(index) {
            node.node_type = NodeType::Input;
            node.desc = Some(input.name().to_string());
            node.index = index as u32;
        }
    }
    for (output_index, node) in nodes.iter_mut().skip(N_INPUTS).take(N_OUTPUTS).enumerate() {
        if let Some(output) = Output::from_index(output_index) {
            node.node_type = output_base_type(output_index);
            node.desc = Some(output.name().to_string());
            node.index = (N_INPUTS + output_index) as u32;
        }
    }
    for (index, node) in nodes.iter_mut().enumerate().skip(N_INPUTS + N_OUTPUTS) {
        node.index = index as u32;
    }
}

/// Base activation type of each output (game `SetBaseNeuronsTypeAndDesc`).
fn output_base_type(index: usize) -> NodeType {
    match index {
        0..=3 | 5 | 7 => NodeType::TanH,
        9..=11 => NodeType::ReLu,
        _ => NodeType::Sigmoid,
    }
}

/// One old-layout gene index mapped to a current index, or -1 for removed genes
/// (`BibiteUpdater.to06a15GenesMap`).
const TO_0_6A15_GENES_MAP: [i32; 34] = [
    0, 1, 2, 3, 4, 5, 6, 7, -1, -1, 8, 9, 10, 11, 12, 13, 14, 15, 16, -1, 17, 18, 19, 20, 21, 22,
    23, 24, 25, 26, 27, 28, 33, 34,
];

/// Port of `BibiteUpdater.UpdateGenesToPresentVersion` (spec 04 4.7).
///
/// `remap_needed` is the game's `geneRemapingNecessary` flag (`version < 0.6a1`):
/// it enables the 34-gene remap and the 0.6a7 default-overwrite quirk.
/// `is_saved` is true for live-entity files (save bundle; the `.bb8` template
/// loader also passes true) and false for `.bb8template`; it changes how
/// Strength/Defence are sourced for pre-0.6a1 files.
pub fn migrate_genes(
    from: Version,
    repr: &GenesRepr,
    remap_needed: bool,
    is_saved: bool,
) -> Result<[f32; N_GENES]> {
    let alpha_0_6a1 = v(0, 6, 0, 1);
    let alpha_0_6a7 = v(0, 6, 0, 7);
    let alpha_0_6a15 = v(0, 6, 0, 15);
    let mut genes = [0.0_f32; 35];
    let mut strength: Option<f32> = None;
    let mut defence: Option<f32> = None;

    match repr {
        GenesRepr::Object(map) => {
            for gene in ALL_GENES {
                genes[gene.index()] = map
                    .get(gene.def().name)
                    .and_then(serde_json::Value::as_f64)
                    .map_or(gene.def().default, |value| value as f32);
            }
            strength = map
                .get("Strength")
                .and_then(serde_json::Value::as_f64)
                .map(|value| value as f32);
            defence = map
                .get("Defence")
                .and_then(serde_json::Value::as_f64)
                .map(|value| value as f32);
        }
        GenesRepr::Array(values) => {
            if remap_needed {
                if values.len() != 34 {
                    return Err(Error::unsupported(format!(
                        "pre-0.6a1 gene array has {} values (expected 34)",
                        values.len()
                    )));
                }
                if is_saved {
                    // The game's `.bb8`/save loaders expect named genes for old
                    // entities; an array there means the file was never
                    // loadable by the installed game either.
                    return Err(Error::unsupported(
                        "pre-0.6a1 gene arrays are only defined for templates",
                    ));
                }
                for (old, target) in TO_0_6A15_GENES_MAP.iter().enumerate() {
                    if *target >= 0 {
                        genes[*target as usize] = values[old];
                    }
                }
            } else if values.len() == 35 {
                genes.copy_from_slice(values);
            } else {
                return Err(Error::unsupported(format!(
                    "gene array has {} values (expected 35)",
                    values.len()
                )));
            }
        }
    }

    // 0.6a7 quirk (preserved on purpose): six slots get new-gene defaults.
    if from < alpha_0_6a7 && remap_needed {
        genes[28] = Gene::EyeOffset.def().default;
        genes[29] = Gene::StomachWAG.def().default;
        genes[30] = Gene::WombWAG.def().default;
        genes[31] = Gene::FatWAG.def().default;
        genes[32] = Gene::FatStorageThreshold.def().default;
        genes[33] = Gene::FatStorageDeadband.def().default;
    }

    if from < alpha_0_6a15 {
        // Strength/Defence: named values for >=0.6a1 or saved entities, old
        // array slots 8/9 for pre-0.6a1 templates.
        let (strength, defence) = if from < alpha_0_6a1 && !is_saved {
            (
                strength.or_else(|| Some(genes[8])).unwrap_or(5.0),
                defence.or_else(|| Some(genes[9])).unwrap_or(0.0),
            )
        } else {
            (strength.unwrap_or(5.0), defence.unwrap_or(0.0))
        };
        let mut array = genes;
        array[28] *= 2.5;
        array[27] *= 2.5;
        array[26] *= 2.5;
        array[31] = 0.0;
        array[29] = 0.0;
        array[32] = 0.0;
        array[30] = 0.0;
        let total: f32 = (26..=32).map(|index| array[index].max(0.0)).sum();
        let share_mouth = 0.01_f32.min((strength / 35.0).powi(2));
        let share_armor = (defence / 35.0).powi(2);
        let share_move = 0.05;
        let share_throat = 0.1;
        let rest = 1.0 - (share_mouth + share_armor + share_move + share_throat);
        if total > 0.0 && rest.abs() > f32::EPSILON {
            array[31] = total * share_mouth / rest;
            array[29] = total * share_armor / rest;
            array[32] = total * share_move / rest;
            array[30] = total * share_throat / rest;
        }
        genes = array;
    }

    Ok(genes)
}

/// Convenience: migrate and wrap in a core [`bibites_core::genome::Genome`] (values preserved
/// exactly; the game clamps entity genes later via `CapGenes`).
#[must_use]
pub fn genome_from_genes(genes: [f32; N_GENES]) -> Genome {
    Genome { genes }
}

//! Cross-language parity digests and capture schema constants.
//!
//! The Rust engine must emit captures in exactly the schema and byte layout the
//! Python ground-truth harness uses, so that `tools/parity/compare.py` can
//! diff a game capture against a Rust capture. All definitions here mirror
//! `tools/parity/parity_common.py` and `docs/PARITY.md` §6.3:
//!
//! * numbers are hashed as raw little-endian bits (f32/i32/bool), never as
//!   decimal text;
//! * a digest is `sha256(bytes).hexdigest()[:16]`;
//! * nodes are sorted by `Index`, synapses keep their in-brain order.
//!
//! Changing any layout here silently breaks parity comparisons — update the
//! Python schema first, then this file, and bump the capture schema version.

use bibites_core::brain::Brain;
use bibites_core::genome::{Genome, N_GENES};
use sha2::{Digest, Sha256};

/// Number of hex characters kept from each SHA-256 digest.
pub const DIGEST_CHARS: usize = 16;

/// Capture manifest schema identifier.
pub const CAPTURE_SCHEMA: &str = "rustibites.parity.capture.v1";
/// Per-tick record schema identifier.
pub const TICK_SCHEMA: &str = "rustibites.parity.tick.v1";
/// Event record schema identifier.
pub const EVENT_SCHEMA: &str = "rustibites.parity.event.v1";

/// The game's `BibiteGenes.Genes` enum order (what the bridge reports as
/// `geneNames`). Kept in lock-step with [`bibites_core::genome::GENE_DEFS`] by a unit test.
pub const GENE_NAMES: [&str; N_GENES] = [
    "LayTime",
    "BroodTime",
    "HatchTime",
    "SizeRatio",
    "SpeedRatio",
    "ColorR",
    "ColorG",
    "ColorB",
    "MutationAmountSigma",
    "AverageMutationNumber",
    "BrainMutationSigma",
    "BrainAverageMutation",
    "ViewAngle",
    "ViewRadius",
    "ClockSpeed",
    "PheroSense",
    "Diet",
    "HerdSeparationWeight",
    "HerdAlignmentWeight",
    "HerdCohesionWeight",
    "HerdVelocityWeight",
    "HerdSeparationDistance",
    "GrowthScale",
    "GrowthMaturityFactor",
    "GrowthMaturityExponent",
    "EyeOffset",
    "StomachWAG",
    "WombWAG",
    "FatWAG",
    "ArmorWAG",
    "ThroatWAG",
    "MouthMusclesWAG",
    "MoveMusclesWAG",
    "FatStorageThreshold",
    "FatStorageDeadband",
];

#[inline]
fn f32_bits(value: f32) -> [u8; 4] {
    value.to_bits().to_le_bytes()
}

#[inline]
fn i32_bits(value: i32) -> [u8; 4] {
    value.to_le_bytes()
}

#[inline]
fn bool_bit(value: bool) -> u8 {
    u8::from(value)
}

fn hex16(hasher: Sha256) -> String {
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex.truncate(DIGEST_CHARS);
    hex
}

/// Digest of the 35 gene values as f32 bit patterns, in gene-enum order.
pub fn genes_digest(genome: &Genome) -> String {
    let mut hasher = Sha256::new();
    for &gene in genome.values().iter() {
        hasher.update(f32_bits(gene));
    }
    hex16(hasher)
}

/// Digest of topology + full node runtime state + every synapse.
pub fn brain_digest(brain: &Brain) -> String {
    let mut nodes = brain.to_node_records();
    nodes.sort_by_key(|node| node.index);
    let synapses = brain.to_synapse_records();

    let mut hasher = Sha256::new();
    hasher.update(i32_bits(nodes.len() as i32));
    for node in &nodes {
        hasher.update(i32_bits(node.node_type as u8 as i32));
        hasher.update(i32_bits(node.index as i32));
        hasher.update(i32_bits(node.innovation as i32));
        for value in [
            node.base_activation,
            node.value,
            node.last_input,
            node.last_output,
        ] {
            hasher.update(f32_bits(value));
        }
    }
    hasher.update(i32_bits(synapses.len() as i32));
    for synapse in &synapses {
        hasher.update(i32_bits(synapse.innovation as i32));
        hasher.update(i32_bits(synapse.node_in as i32));
        hasher.update(i32_bits(synapse.node_out as i32));
        hasher.update(f32_bits(synapse.weight));
        hasher.update([bool_bit(synapse.enabled)]);
    }
    hex16(hasher)
}

/// Digest of the 33 input node values (the sensor readings as the brain sees
/// them).
pub fn sense_digest(brain: &Brain) -> String {
    let mut nodes = brain.to_node_records();
    nodes.sort_by_key(|node| node.index);
    let mut hasher = Sha256::new();
    let inputs: Vec<_> = nodes.iter().filter(|node| node.index < 33).collect();
    hasher.update(i32_bits(inputs.len() as i32));
    for node in inputs {
        hasher.update(i32_bits(node.index as i32));
        hasher.update(f32_bits(node.value));
    }
    hex16(hasher)
}

/// Digest of the 15 output nodes' `Value`/`LastInput`/`LastOutput` (motor
/// output).
pub fn output_digest(brain: &Brain) -> String {
    let mut nodes = brain.to_node_records();
    nodes.sort_by_key(|node| node.index);
    let mut hasher = Sha256::new();
    let outputs: Vec<_> = nodes
        .iter()
        .filter(|node| (33..48).contains(&node.index))
        .collect();
    hasher.update(i32_bits(outputs.len() as i32));
    for node in outputs {
        hasher.update(i32_bits(node.index as i32));
        for value in [node.value, node.last_input, node.last_output] {
            hasher.update(f32_bits(value));
        }
    }
    hex16(hasher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bibites_core::brain::{BrainInputs, NodeType};
    use bibites_core::genome::GENE_DEFS;

    #[test]
    fn schema_strings_match_python_harness() {
        assert_eq!(CAPTURE_SCHEMA, "rustibites.parity.capture.v1");
        assert_eq!(TICK_SCHEMA, "rustibites.parity.tick.v1");
        assert_eq!(EVENT_SCHEMA, "rustibites.parity.event.v1");
        assert_eq!(DIGEST_CHARS, 16);
    }

    #[test]
    fn gene_names_match_core_gene_order() {
        assert_eq!(N_GENES, GENE_DEFS.len());
        for (index, def) in GENE_DEFS.iter().enumerate() {
            assert_eq!(
                GENE_NAMES[index], def.name,
                "gene name drift at index {index}"
            );
        }
    }

    #[test]
    fn sha16_matches_python_prefix() {
        // hashlib.sha256(b"abc").hexdigest()[:16]
        let mut hasher = Sha256::new();
        hasher.update(b"abc");
        assert_eq!(hex16(hasher), "ba7816bf8f01cfea");
    }

    #[test]
    fn genes_digest_of_zero_genome_matches_python() {
        // sha256(140 zero bytes).hexdigest()[:16]
        let genome = Genome {
            genes: [0.0; N_GENES],
        };
        assert_eq!(genes_digest(&genome), "24045c10c12a89f4");
    }

    #[test]
    fn digests_are_deterministic_and_well_formed() {
        let brain = Brain::starter();
        let genes = Genome::starter();
        let genes_digest_a = genes_digest(&genes);
        assert_eq!(genes_digest_a, genes_digest(&genes));
        assert_eq!(genes_digest_a.len(), DIGEST_CHARS);
        assert!(genes_digest_a.chars().all(|c| c.is_ascii_hexdigit()));

        for digest in [
            brain_digest(&brain),
            sense_digest(&brain),
            output_digest(&brain),
        ] {
            assert_eq!(digest.len(), DIGEST_CHARS);
            assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn different_brains_have_different_digests() {
        let mut first = Brain::starter();
        let mut second = Brain::starter();
        // Step one of them with non-zero inputs so runtime state diverges.
        let mut inputs = BrainInputs::new();
        inputs.set(bibites_core::brain::Input::EnergyRatio, 0.75);
        let _ = first.step(&inputs, 1.0 / 15.0);
        let _ = second.step(&BrainInputs::new(), 1.0 / 15.0);
        assert_ne!(brain_digest(&first), brain_digest(&second));
        assert!(
            first
                .to_node_records()
                .iter()
                .any(|n| n.node_type != NodeType::Input || n.value != 0.0)
        );
    }
}

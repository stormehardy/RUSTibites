//! NEAT neural network: node/connection model, CSR execution, mutation, distance.
//!
//! Faithful port of `Here/SimulationScripts.BibiteScripts/NEATBrain.cs`; the
//! authoritative write-up with line-by-line references is
//! `docs/spec/01-brain-genetics.md`.
//!
//! # Execution model
//!
//! Every neuron reads the **previous brain tick's** output (`LastOutput` in the
//! game), so the update is synchronous: evaluation order does not affect the
//! result, only the fixed `f32` summation order does. [`crate::brain::Brain`] groups synapses
//! by destination in the game's "first appearance in the synapse array" order
//! and evaluates each destination exactly once per [`crate::brain::Brain::step`].
//!
//! Inputs are one brain tick stale by construction: [`crate::brain::Brain::step`] shifts the
//! previous input values into their `LastOutput` slots before writing the new
//! values, mirroring the game's `UpdateSenses` ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¾ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ `Thinker` order.
//!
//! Every node output is clamped to `[-1024, 1024]` (`NEATBrain.cs:622-639`).

use crate::error::{Error, Result};
use crate::genome::{self, Genome, GenomeSettings, MutationFactor};
use crate::math::{clamp, exp, gaussian, relu, sigmoid, sin, tanh};
use crate::rng::Rng;

/// Number of input nodes (`NEATBrain.NInputs`, `NEATBrain.cs:243`).
pub const N_INPUTS: usize = 33;
/// Number of output nodes (`NEATBrain.NOutputs`, `NEATBrain.cs:245`).
pub const N_OUTPUTS: usize = 15;
/// Index of the first output node in the node array (`33`).
pub const FIRST_OUTPUT: usize = N_INPUTS;
/// Index of the first hidden node in the node array (`48`).
pub const FIRST_HIDDEN: usize = N_INPUTS + N_OUTPUTS;
/// Output clamp applied by `ExecuteNeuron` (`NEATBrain.cs:639`).
pub const OUTPUT_CLAMP: f32 = 1024.0;
/// Node index of the `Herding` output (`33 + 2`, `NEATBrain.cs:694`).
pub const HERDING_OUTPUT_NODE: usize = FIRST_OUTPUT + 2;
/// The node `ComputeProcessingStructure` leaves out when herding is off (`NEATBrain.cs:515`: `herdingEnabled || Synapses[i].NodeOut != 2`). The author meant the
/// Herding output, but the line compares with the literal `2`, which is an INPUT node (`LifeRatio`) that no synapse ends in: with herding off the game still
/// computes the Herding output when synapses end in it (found 2026-11-19 against the original game: its value moves tick by tick, `brain_vs_game` in
/// `crates/bibites-cli/examples`). The port copies the literal. (`AddSynapse`, `:694`, does compare with `NInputs + 2`, and does not create such a synapse.)
pub const HERDING_SKIPPED_NODE: usize = 2;
/// Default brain period (s) at the game's default settings: `brain_tps/sim_tps`
/// = `2/30` (`docs/spec/01-brain-genetics.md` ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â§3.1).
pub const DEFAULT_BRAIN_PERIOD: f32 = 2.0 / 30.0;
/// Bit of an [`Input`] inside [`Brain::used_input_mask`].
#[inline]
pub const fn input_bit(input: Input) -> u64 {
    1_u64 << (input as u64)
}
/// Bit of an [`Output`] inside [`Brain::used_output_mask`].
#[inline]
pub const fn output_bit(output: Output) -> u16 {
    1_u16 << (output as u16)
}
/// Inputs 9,10,11 (seen bibites) and 18,19,20 (closest bibite colour) ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â the
/// bibite-vision category (`NEATBrain.StartUsedSystems`, `NEATBrain.cs:533-551`).
pub const VISION_BIBITE_INPUT_MASK: u64 = input_bit(Input::BibiteCloseness)
    | input_bit(Input::BibiteAngle)
    | input_bit(Input::NBibites)
    | input_bit(Input::RedBibite)
    | input_bit(Input::GreenBibite)
    | input_bit(Input::BlueBibite);
/// Inputs 12,13,14 ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â the plant-vision category.
pub const VISION_PLANT_INPUT_MASK: u64 =
    input_bit(Input::PlantCloseness) | input_bit(Input::PlantAngle) | input_bit(Input::NPlants);
/// Inputs 15,16,17 ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â the meat-vision category.
pub const VISION_MEAT_INPUT_MASK: u64 =
    input_bit(Input::MeatCloseness) | input_bit(Input::MeatAngle) | input_bit(Input::NMeats);
/// Inputs 24..=32 ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â all pheromone channels.
pub const PHEROMONE_INPUT_MASK: u64 = input_bit(Input::PheroSense1)
    | input_bit(Input::PheroSense2)
    | input_bit(Input::PheroSense3)
    | input_bit(Input::Phero1Angle)
    | input_bit(Input::Phero2Angle)
    | input_bit(Input::Phero3Angle)
    | input_bit(Input::Phero1Heading)
    | input_bit(Input::Phero2Heading)
    | input_bit(Input::Phero3Heading);
/// Inputs 0..=8 ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â body-internal senses.
pub const BODY_INPUT_MASK: u64 = input_bit(Input::EnergyRatio)
    | input_bit(Input::Maturity)
    | input_bit(Input::LifeRatio)
    | input_bit(Input::Fullness)
    | input_bit(Input::Speed)
    | input_bit(Input::RotationSpeed)
    | input_bit(Input::IsGrabbing)
    | input_bit(Input::AttackedDamage)
    | input_bit(Input::EggStored);
/// Inputs 21,22,23 ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â internal clock channels.
pub const CLOCK_INPUT_MASK: u64 =
    input_bit(Input::Tic) | input_bit(Input::Minute) | input_bit(Input::TimeAlive);

/// Defensive node cap. The game has no cap; this protects the Rust engine from
/// pathological mutation growth (documented deviation).
pub const MAX_NODES: usize = 4096;
/// Defensive synapse cap; see [`MAX_NODES`].
pub const MAX_SYNAPSES: usize = 65_536;

/// Activation function type of a neuron, in the game's `NodeType` order
/// (`NEATBrain.cs:70-86`).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeType {
    /// Raw pass-through (input nodes).
    Input = 0,
    /// `1 / (1 + exp(-x))`.
    Sigmoid = 1,
    /// `x`.
    Linear = 2,
    /// `tanh(x)`.
    TanH = 3,
    /// `sin(x)`.
    Sine = 4,
    /// `max(x, 0)`.
    ReLu = 5,
    /// `1 / (1 + xÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â²)`.
    Gaussian = 6,
    /// Two-level switch with memory (spec ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â§2.2).
    Latch = 7,
    /// `(x - lastInput) / period`.
    Differential = 8,
    /// `abs(x)`.
    Abs = 9,
    /// Product of inputs, bias as a multiplier.
    Mult = 10,
    /// `lastOutput + x * period` (no bias).
    Integrator = 11,
    /// `x - lastInput + lastOutput * exp(-bias * period)`.
    Inhibitory = 12,
    /// Saturating hysteresis latch.
    SoftLatch = 13,
}

impl NodeType {
    /// Number of node types (`NEATBrain.NNodesTypes = 14`).
    pub const COUNT: usize = 14;

    /// Maps the game's integer node type to a [`NodeType`].
    pub fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::Input,
            1 => Self::Sigmoid,
            2 => Self::Linear,
            3 => Self::TanH,
            4 => Self::Sine,
            5 => Self::ReLu,
            6 => Self::Gaussian,
            7 => Self::Latch,
            8 => Self::Differential,
            9 => Self::Abs,
            10 => Self::Mult,
            11 => Self::Integrator,
            12 => Self::Inhibitory,
            13 => Self::SoftLatch,
            _ => return None,
        })
    }

    /// The game's `NodeType.ToString()` name (`"Sigmoid"`, ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¦).
    pub fn name(self) -> &'static str {
        match self {
            Self::Input => "Input",
            Self::Sigmoid => "Sigmoid",
            Self::Linear => "Linear",
            Self::TanH => "TanH",
            Self::Sine => "Sine",
            Self::ReLu => "ReLu",
            Self::Gaussian => "Gaussian",
            Self::Latch => "Latch",
            Self::Differential => "Differential",
            Self::Abs => "Abs",
            Self::Mult => "Mult",
            Self::Integrator => "Integrator",
            Self::Inhibitory => "Inhibitory",
            Self::SoftLatch => "SoftLatch",
        }
    }

    /// `NEATBrain.TypeHasBias`: false for `Input` and `Integrator`
    /// (`NEATBrain.cs:285-292`).
    pub fn has_bias(self) -> bool {
        !matches!(self, Self::Input | Self::Integrator)
    }

    /// `NEATBrain.TypeUsesBiasDifferently`: `Integrator`, `Mult`, `Inhibitory`,
    /// `SoftLatch` (`NEATBrain.cs:294-301`).
    pub fn uses_bias_differently(self) -> bool {
        matches!(
            self,
            Self::Integrator | Self::Mult | Self::Inhibitory | Self::SoftLatch
        )
    }

    /// `NEATBrain.TypeIsNonLinear`: `Differential`, `Inhibitory`, `Integrator`,
    /// `Latch`, `SoftLatch` (`NEATBrain.cs:303-310`). Used by the node editor to
    /// show its nonlinear disclaimer.
    #[must_use]
    pub fn is_non_linear(self) -> bool {
        matches!(
            self,
            Self::Differential
                | Self::Inhibitory
                | Self::Integrator
                | Self::Latch
                | Self::SoftLatch
        )
    }
}

/// Brain inputs, in the game's `NEATBrain.Inputs` order (`NEATBrain.cs:14-49`).
#[repr(usize)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Input {
    /// Energy / max energy.
    EnergyRatio = 0,
    /// Raw growth value.
    Maturity = 1,
    /// Health / max health.
    LifeRatio = 2,
    /// Stomach fullness.
    Fullness = 3,
    /// Forward speed / body length.
    Speed = 4,
    /// Angular speed in revolutions per second.
    RotationSpeed = 5,
    /// Number of held objects.
    IsGrabbing = 6,
    /// Normalized recent damage.
    AttackedDamage = 7,
    /// Completed eggs stored.
    EggStored = 8,
    /// Seen bibites closeness.
    BibiteCloseness = 9,
    /// Seen bibites angle.
    BibiteAngle = 10,
    /// Seen bibites count / 4.
    NBibites = 11,
    /// Seen plants closeness.
    PlantCloseness = 12,
    /// Seen plants angle.
    PlantAngle = 13,
    /// Seen plants count / 4.
    NPlants = 14,
    /// Seen meat closeness.
    MeatCloseness = 15,
    /// Seen meat angle.
    MeatAngle = 16,
    /// Seen meat count / 4.
    NMeats = 17,
    /// Closest bibite red gene.
    RedBibite = 18,
    /// Closest bibite green gene.
    GreenBibite = 19,
    /// Closest bibite blue gene.
    BlueBibite = 20,
    /// Clock square wave (0/1).
    Tic = 21,
    /// Minutes since clock reset.
    Minute = 22,
    /// Minutes alive.
    TimeAlive = 23,
    /// Red pheromone strength, `tanh(sum/5)`.
    PheroSense1 = 24,
    /// Green pheromone strength.
    PheroSense2 = 25,
    /// Blue pheromone strength.
    PheroSense3 = 26,
    /// Red pheromone angle.
    Phero1Angle = 27,
    /// Green pheromone angle.
    Phero2Angle = 28,
    /// Blue pheromone angle.
    Phero3Angle = 29,
    /// Red pheromone heading.
    Phero1Heading = 30,
    /// Green pheromone heading.
    Phero2Heading = 31,
    /// Blue pheromone heading.
    Phero3Heading = 32,
}

/// All 33 inputs in enum order.
pub const ALL_INPUTS: [Input; N_INPUTS] = {
    use Input::*;
    [
        EnergyRatio,
        Maturity,
        LifeRatio,
        Fullness,
        Speed,
        RotationSpeed,
        IsGrabbing,
        AttackedDamage,
        EggStored,
        BibiteCloseness,
        BibiteAngle,
        NBibites,
        PlantCloseness,
        PlantAngle,
        NPlants,
        MeatCloseness,
        MeatAngle,
        NMeats,
        RedBibite,
        GreenBibite,
        BlueBibite,
        Tic,
        Minute,
        TimeAlive,
        PheroSense1,
        PheroSense2,
        PheroSense3,
        Phero1Angle,
        Phero2Angle,
        Phero3Angle,
        Phero1Heading,
        Phero2Heading,
        Phero3Heading,
    ]
};

impl Input {
    /// Index into [`BrainInputs`].
    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }

    /// The game's enum name (`"EnergyRatio"`, ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¦).
    pub fn name(self) -> &'static str {
        match self {
            Self::EnergyRatio => "EnergyRatio",
            Self::Maturity => "Maturity",
            Self::LifeRatio => "LifeRatio",
            Self::Fullness => "Fullness",
            Self::Speed => "Speed",
            Self::RotationSpeed => "RotationSpeed",
            Self::IsGrabbing => "IsGrabbing",
            Self::AttackedDamage => "AttackedDamage",
            Self::EggStored => "EggStored",
            Self::BibiteCloseness => "BibiteCloseness",
            Self::BibiteAngle => "BibiteAngle",
            Self::NBibites => "NBibites",
            Self::PlantCloseness => "PlantCloseness",
            Self::PlantAngle => "PlantAngle",
            Self::NPlants => "NPlants",
            Self::MeatCloseness => "MeatCloseness",
            Self::MeatAngle => "MeatAngle",
            Self::NMeats => "NMeats",
            Self::RedBibite => "RedBibite",
            Self::GreenBibite => "GreenBibite",
            Self::BlueBibite => "BlueBibite",
            Self::Tic => "Tic",
            Self::Minute => "Minute",
            Self::TimeAlive => "TimeAlive",
            Self::PheroSense1 => "PheroSense1",
            Self::PheroSense2 => "PheroSense2",
            Self::PheroSense3 => "PheroSense3",
            Self::Phero1Angle => "Phero1Angle",
            Self::Phero2Angle => "Phero2Angle",
            Self::Phero3Angle => "Phero3Angle",
            Self::Phero1Heading => "Phero1Heading",
            Self::Phero2Heading => "Phero2Heading",
            Self::Phero3Heading => "Phero3Heading",
        }
    }

    /// Maps an input index to an [`Input`].
    #[inline]
    pub fn from_index(index: usize) -> Option<Self> {
        ALL_INPUTS.get(index).copied()
    }
}

/// Brain outputs, in the game's `NEATBrain.Outputs` order (`NEATBrain.cs:51-68`).
#[repr(usize)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Output {
    /// Forward acceleration.
    Accelerate = 0,
    /// Turn torque (positive = counter-clockwise / left; spec ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â§0).
    Rotate = 1,
    /// Herding blend.
    Herding = 2,
    /// Egg organ activity.
    EggProduction = 3,
    /// Lay stored eggs if >= 0.25.
    Want2Lay = 4,
    /// Mouth opening / swallow intent.
    Want2Eat = 5,
    /// Stomach acid level.
    Digestion = 6,
    /// Grab strength.
    Grab = 7,
    /// Reset internal clock if > 0.75.
    ClkReset = 8,
    /// Red pheromone production.
    PhereOut1 = 9,
    /// Green pheromone production.
    PhereOut2 = 10,
    /// Blue pheromone production.
    PhereOut3 = 11,
    /// Growth multiplier.
    Want2Grow = 12,
    /// Healing multiplier.
    Want2Heal = 13,
    /// Bite strength.
    Want2Attack = 14,
}

/// All 15 outputs in enum order.
pub const ALL_OUTPUTS: [Output; N_OUTPUTS] = {
    use Output::*;
    [
        Accelerate,
        Rotate,
        Herding,
        EggProduction,
        Want2Lay,
        Want2Eat,
        Digestion,
        Grab,
        ClkReset,
        PhereOut1,
        PhereOut2,
        PhereOut3,
        Want2Grow,
        Want2Heal,
        Want2Attack,
    ]
};

impl Output {
    /// Index into [`BrainOutputs`].
    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }

    /// The game's enum name (`"Accelerate"`, ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¦).
    pub fn name(self) -> &'static str {
        match self {
            Self::Accelerate => "Accelerate",
            Self::Rotate => "Rotate",
            Self::Herding => "Herding",
            Self::EggProduction => "EggProduction",
            Self::Want2Lay => "Want2Lay",
            Self::Want2Eat => "Want2Eat",
            Self::Digestion => "Digestion",
            Self::Grab => "Grab",
            Self::ClkReset => "ClkReset",
            Self::PhereOut1 => "PhereOut1",
            Self::PhereOut2 => "PhereOut2",
            Self::PhereOut3 => "PhereOut3",
            Self::Want2Grow => "Want2Grow",
            Self::Want2Heal => "Want2Heal",
            Self::Want2Attack => "Want2Attack",
        }
    }

    /// Maps an output index to an [`Output`].
    #[inline]
    pub fn from_index(index: usize) -> Option<Self> {
        ALL_OUTPUTS.get(index).copied()
    }

    /// The fixed activation type the game assigns to this output in
    /// `NEATBrain.SetBaseNeuronsTypeAndDesc` (`NEATBrain.cs:1029-1056`).
    pub fn node_type(self) -> NodeType {
        match self {
            Self::Accelerate
            | Self::Rotate
            | Self::Herding
            | Self::EggProduction
            | Self::Want2Eat
            | Self::Grab => NodeType::TanH,
            Self::PhereOut1 | Self::PhereOut2 | Self::PhereOut3 => NodeType::ReLu,
            Self::Want2Lay
            | Self::Digestion
            | Self::ClkReset
            | Self::Want2Grow
            | Self::Want2Heal
            | Self::Want2Attack => NodeType::Sigmoid,
        }
    }
}

/// World-provided senses consumed by the brain (21 of the 33 inputs).
///
/// The world's `SenseOutputs` (`docs/INTERFACES.md` ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â§6) implements this trait;
/// keeping the trait in `brain.rs` avoids a core dependency from `brain` to
/// `world`/`senses`.
pub trait WorldSenseSource {
    /// 0..1: weighted-average bibite closeness.
    fn bibite_closeness(&self) -> f32;
    /// Normalized angle to the weighted-average bibite.
    fn bibite_angle(&self) -> f32;
    /// Number of seen bibites divided by four.
    fn n_bibites(&self) -> f32;
    /// 0..1: weighted-average plant closeness.
    fn plant_closeness(&self) -> f32;
    /// Normalized angle to the weighted-average plant pellet.
    fn plant_angle(&self) -> f32;
    /// Number of seen plant pellets divided by four.
    fn n_plants(&self) -> f32;
    /// 0..1: weighted-average meat closeness.
    fn meat_closeness(&self) -> f32;
    /// Normalized angle to the weighted-average meat.
    fn meat_angle(&self) -> f32;
    /// Number of seen meat objects divided by four.
    fn n_meats(&self) -> f32;
    /// Red gene of the closest seen bibite.
    fn red_bibite(&self) -> f32;
    /// Green gene of the closest seen bibite.
    fn green_bibite(&self) -> f32;
    /// Blue gene of the closest seen bibite.
    fn blue_bibite(&self) -> f32;
    /// `tanh(pheroSum1 / 5)`.
    fn phero_sense_1(&self) -> f32;
    /// `tanh(pheroSum2 / 5)`.
    fn phero_sense_2(&self) -> f32;
    /// `tanh(pheroSum3 / 5)`.
    fn phero_sense_3(&self) -> f32;
    /// Normalized red pheromone angle.
    fn phero_1_angle(&self) -> f32;
    /// Normalized green pheromone angle.
    fn phero_2_angle(&self) -> f32;
    /// Normalized blue pheromone angle.
    fn phero_3_angle(&self) -> f32;
    /// Normalized red pheromone heading.
    fn phero_1_heading(&self) -> f32;
    /// Normalized green pheromone heading.
    fn phero_2_heading(&self) -> f32;
    /// Normalized blue pheromone heading.
    fn phero_3_heading(&self) -> f32;
}

/// Body-provided internal senses consumed by the brain (12 of the 33 inputs).
pub trait InternalSenseSource {
    /// Energy / max energy.
    fn energy_ratio(&self) -> f32;
    /// Raw growth value (`BibiteGrowth.growth`).
    fn maturity(&self) -> f32;
    /// Health / max health.
    fn life_ratio(&self) -> f32;
    /// Stomach fullness.
    fn fullness(&self) -> f32;
    /// Signed forward speed / body length.
    fn speed(&self) -> f32;
    /// Angular velocity in revolutions per second.
    fn rotation_speed(&self) -> f32;
    /// Number of held objects.
    fn is_grabbing(&self) -> f32;
    /// Normalized recent attack damage.
    fn attacked_damage(&self) -> f32;
    /// Completed eggs stored.
    fn egg_stored(&self) -> f32;
    /// Clock square wave 0/1.
    fn tic(&self) -> f32;
    /// Minutes since the last clock reset.
    fn minute(&self) -> f32;
    /// Minutes since birth.
    fn time_alive(&self) -> f32;
}

/// One brain update's input vector.
#[derive(Clone, Debug)]
pub struct BrainInputs {
    values: [f32; N_INPUTS],
}

impl Default for BrainInputs {
    fn default() -> Self {
        Self {
            values: [0.0; N_INPUTS],
        }
    }
}

impl BrainInputs {
    /// All-zero inputs.
    pub fn new() -> Self {
        Self::default()
    }

    /// Writes one input by name.
    #[inline]
    pub fn set(&mut self, input: Input, value: f32) {
        self.values[input.index()] = value;
    }

    /// Reads one input by name.
    #[inline]
    pub fn get(&self, input: Input) -> f32 {
        self.values[input.index()]
    }

    /// Raw input values (io/testing).
    #[inline]
    pub fn values(&self) -> &[f32; N_INPUTS] {
        &self.values
    }

    /// Fills the 12 body-internal inputs.
    pub fn write_internal<S: InternalSenseSource + ?Sized>(&mut self, state: &S) {
        self.set(Input::EnergyRatio, state.energy_ratio());
        self.set(Input::Maturity, state.maturity());
        self.set(Input::LifeRatio, state.life_ratio());
        self.set(Input::Fullness, state.fullness());
        self.set(Input::Speed, state.speed());
        self.set(Input::RotationSpeed, state.rotation_speed());
        self.set(Input::IsGrabbing, state.is_grabbing());
        self.set(Input::AttackedDamage, state.attacked_damage());
        self.set(Input::EggStored, state.egg_stored());
        self.set(Input::Tic, state.tic());
        self.set(Input::Minute, state.minute());
        self.set(Input::TimeAlive, state.time_alive());
    }

    /// Fills the 21 world-derived inputs (vision + pheromone).
    pub fn write_world_senses<S: WorldSenseSource + ?Sized>(&mut self, senses: &S) {
        self.set(Input::BibiteCloseness, senses.bibite_closeness());
        self.set(Input::BibiteAngle, senses.bibite_angle());
        self.set(Input::NBibites, senses.n_bibites());
        self.set(Input::PlantCloseness, senses.plant_closeness());
        self.set(Input::PlantAngle, senses.plant_angle());
        self.set(Input::NPlants, senses.n_plants());
        self.set(Input::MeatCloseness, senses.meat_closeness());
        self.set(Input::MeatAngle, senses.meat_angle());
        self.set(Input::NMeats, senses.n_meats());
        self.set(Input::RedBibite, senses.red_bibite());
        self.set(Input::GreenBibite, senses.green_bibite());
        self.set(Input::BlueBibite, senses.blue_bibite());
        self.set(Input::PheroSense1, senses.phero_sense_1());
        self.set(Input::PheroSense2, senses.phero_sense_2());
        self.set(Input::PheroSense3, senses.phero_sense_3());
        self.set(Input::Phero1Angle, senses.phero_1_angle());
        self.set(Input::Phero2Angle, senses.phero_2_angle());
        self.set(Input::Phero3Angle, senses.phero_3_angle());
        self.set(Input::Phero1Heading, senses.phero_1_heading());
        self.set(Input::Phero2Heading, senses.phero_2_heading());
        self.set(Input::Phero3Heading, senses.phero_3_heading());
    }
}

/// One brain update's output vector.
#[derive(Clone, Debug)]
pub struct BrainOutputs {
    values: [f32; N_OUTPUTS],
}

impl Default for BrainOutputs {
    fn default() -> Self {
        Self {
            values: [0.0; N_OUTPUTS],
        }
    }
}

impl BrainOutputs {
    /// Reads one output by name.
    #[inline]
    pub fn get(&self, output: Output) -> f32 {
        self.values[output.index()]
    }

    /// Raw output values (io/testing).
    #[inline]
    pub fn values(&self) -> &[f32; N_OUTPUTS] {
        &self.values
    }
}

/// Counts used by the body to charge brain upkeep.
///
/// The game's `brainUpkeepCost = nSynaps * synapseUpkeepCost + nHidden *
/// neuronUpkeepCost` (`NEATBrain.cs:283`) is a **per-second** rate; the body
/// multiplies by the settings and `dt` (`BibiteBody.cs:603`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BrainUpkeep {
    /// Number of synapses (enabled or not ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â all cost upkeep).
    pub synapses: u32,
    /// Number of hidden neurons (inputs/outputs are free).
    pub hidden: u32,
}

impl BrainUpkeep {
    /// Energy per second at the given upkeep settings.
    #[inline]
    pub fn energy_per_second(&self, synapse_upkeep: f32, neuron_upkeep: f32) -> f32 {
        // `(float)nSynaps * synapseUpkeepCost + (float)nHidden * neuronUpkeepCost` is one expression of the game's `double` arithmetic, rounded once as the property
        // returns it (see `activation_function`).
        (f64::from(self.synapses as f32) * f64::from(synapse_upkeep)
            + f64::from(self.hidden as f32) * f64::from(neuron_upkeep)) as f32
    }
}

/// Runtime state of one node (`Value`, `LastInput`, `LastOutput` in the game),
/// used by the parity harness to capture/restore exact brain state
/// (`docs/PARITY.md` ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â§6.3 brain digests).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NodeRuntimeState {
    /// `Node.Value`: current output.
    pub value: f32,
    /// `Node.LastInput`: previous full activation `x`.
    pub last_input: f32,
    /// `Node.LastOutput`: value readable by the network this tick.
    pub last_output: f32,
}

/// Serialized node record (the `.bb8` node object, or the `.bb8template` subset
/// when `value`/`last_input`/`last_output` are zero and `desc` is `None`).
///
/// See `docs/spec/01-brain-genetics.md` ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â§1.2 and ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â§10.1 for the JSON mapping.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeRecord {
    /// Activation function.
    pub node_type: NodeType,
    /// Node array position (informational; constructors use slice order).
    pub index: u32,
    /// Innovation number.
    pub innovation: i64,
    /// Custom description. `None` for base nodes (name derived from index).
    pub desc: Option<String>,
    /// Bias / default activation.
    pub base_activation: f32,
    /// Current output (`Node.Value`).
    pub value: f32,
    /// Previous full activation (`Node.LastInput`).
    pub last_input: f32,
    /// Previous output (`Node.LastOutput`).
    pub last_output: f32,
}

impl NodeRecord {
    /// Creates a record in the `.bb8template` shape (no runtime state).
    pub fn template(
        node_type: NodeType,
        index: u32,
        innovation: i64,
        base_activation: f32,
    ) -> Self {
        Self {
            node_type,
            index,
            innovation,
            desc: None,
            base_activation,
            value: 0.0,
            last_input: 0.0,
            last_output: 0.0,
        }
    }
}

/// Serialized synapse record (`Synaps` in the game, `NEATBrain.cs:172-183`).
#[derive(Clone, Debug, PartialEq)]
pub struct SynapseRecord {
    /// Innovation number (primary key for cross-genome alignment).
    pub innovation: i64,
    /// Source node index.
    pub node_in: u32,
    /// Destination node index.
    pub node_out: u32,
    /// Connection weight.
    pub weight: f32,
    /// Whether the connection transmits.
    pub enabled: bool,
}

/// Read-only node row for brain-graph panels.
#[derive(Clone, Debug, PartialEq)]
pub struct BrainNodeView {
    /// Array index (stable for this graph view).
    pub index: usize,
    /// Activation function.
    pub node_type: NodeType,
    /// Innovation number.
    pub innovation: i64,
    /// Input/output enum name for base nodes, custom hidden description otherwise.
    pub label: String,
    /// Bias / default activation.
    pub base_activation: f32,
    /// Current runtime value (`Node.Value`).
    pub value: f32,
    /// Previous full activation.
    pub last_input: f32,
    /// Previous output.
    pub last_output: f32,
}

/// Read-only synapse row for brain-graph panels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrainSynapseView {
    /// Array index in this graph view.
    pub index: usize,
    /// Innovation number.
    pub innovation: i64,
    /// Source node index.
    pub node_in: usize,
    /// Destination node index.
    pub node_out: usize,
    /// Connection weight.
    pub weight: f32,
    /// Whether the connection is enabled.
    pub enabled: bool,
}

/// Complete read-only snapshot for a brain-graph panel.
#[derive(Clone, Debug, PartialEq)]
pub struct BrainGraphView {
    /// Nodes in the graph's array order.
    pub nodes: Vec<BrainNodeView>,
    /// Synapses in template order.
    pub synapses: Vec<BrainSynapseView>,
    /// Current output values (input values live in the caller's [`BrainInputs`]).
    pub outputs: [f32; N_OUTPUTS],
}

/// Draft-editor validation result.
///
/// Unlike [`Brain::validate`] this permits disconnected/floating hidden nodes
/// while the user is drawing the graph. Errors describe editor-breaking
/// structure; `floating_hidden` is advisory and does not make the draft invalid.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrainEditorValidation {
    /// Structural errors (base-node types, endpoints, self/duplicate links).
    pub errors: Vec<String>,
    /// Hidden node indices with no incoming or no outgoing synapse.
    pub floating_hidden: Vec<usize>,
}

impl BrainEditorValidation {
    /// True when no blocking structural error was found.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Settings needed by brain mutation.
///
/// Like [`crate::genome::GenomeSettings`] this decouples `brain.rs` from fmt-dev's
/// `SimSettings`; when `settings.rs` lands, implement `BrainSettings` for it.
/// Values are scenario settings from `Here/SettingScripts/ScenarioSettings.cs`.
pub trait BrainSettings {
    /// `synapseMutationChance` (default 0.6).
    fn synapse_mutation_chance(&self) -> f32;
    /// `synapseAddChance` (default 0.1).
    fn synapse_add_chance(&self) -> f32;
    /// `synapseChangeChance` (default 0.75).
    fn synapse_change_chance(&self) -> f32;
    /// `synapseFlipChance` (default 0.025).
    fn synapse_flip_chance(&self) -> f32;
    /// `synapseToggleChance` (default 0.025).
    fn synapse_toggle_chance(&self) -> f32;
    /// `synapseRemoveChance` (default 0.1).
    fn synapse_remove_chance(&self) -> f32;
    /// `neuronDefaultChance` (default 0.7).
    fn neuron_default_chance(&self) -> f32;
    /// `neuronChangeChance` (default 0.1).
    fn neuron_change_chance(&self) -> f32;
    /// `neuronAddChance` (default 0.1).
    fn neuron_add_chance(&self) -> f32;
    /// `neuronRemoveChance` (default 0.1).
    fn neuron_remove_chance(&self) -> f32;
    /// `disableHerding` (default false): hides the Herding output and prevents
    /// new synapses from targeting it.
    fn disable_herding(&self) -> bool;
}

/// Captured game defaults for [`BrainSettings`].
///
/// Temporary shim until fmt-dev's `SimSettings` implements [`BrainSettings`];
/// production code must not depend on it (same policy as
/// `genome::DefaultGenomeSettings`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DefaultBrainSettings {
    /// `synapseMutationChance`.
    pub synapse_mutation_chance: f32,
    /// `synapseAddChance`.
    pub synapse_add_chance: f32,
    /// `synapseChangeChance`.
    pub synapse_change_chance: f32,
    /// `synapseFlipChance`.
    pub synapse_flip_chance: f32,
    /// `synapseToggleChance`.
    pub synapse_toggle_chance: f32,
    /// `synapseRemoveChance`.
    pub synapse_remove_chance: f32,
    /// `neuronDefaultChance`.
    pub neuron_default_chance: f32,
    /// `neuronChangeChance`.
    pub neuron_change_chance: f32,
    /// `neuronAddChance`.
    pub neuron_add_chance: f32,
    /// `neuronRemoveChance`.
    pub neuron_remove_chance: f32,
    /// `disableHerding`.
    pub disable_herding: bool,
}

impl DefaultBrainSettings {
    /// Captured 0.6.3.1 defaults (`ScenarioSettings.cs:56-219,257-263`).
    pub fn game_defaults() -> Self {
        Self {
            synapse_mutation_chance: 0.6,
            synapse_add_chance: 0.1,
            synapse_change_chance: 0.75,
            synapse_flip_chance: 0.025,
            synapse_toggle_chance: 0.025,
            synapse_remove_chance: 0.1,
            neuron_default_chance: 0.7,
            neuron_change_chance: 0.1,
            neuron_add_chance: 0.1,
            neuron_remove_chance: 0.1,
            disable_herding: false,
        }
    }
}

impl BrainSettings for DefaultBrainSettings {
    fn synapse_mutation_chance(&self) -> f32 {
        self.synapse_mutation_chance
    }
    fn synapse_add_chance(&self) -> f32 {
        self.synapse_add_chance
    }
    fn synapse_change_chance(&self) -> f32 {
        self.synapse_change_chance
    }
    fn synapse_flip_chance(&self) -> f32 {
        self.synapse_flip_chance
    }
    fn synapse_toggle_chance(&self) -> f32 {
        self.synapse_toggle_chance
    }
    fn synapse_remove_chance(&self) -> f32 {
        self.synapse_remove_chance
    }
    fn neuron_default_chance(&self) -> f32 {
        self.neuron_default_chance
    }
    fn neuron_change_chance(&self) -> f32 {
        self.neuron_change_chance
    }
    fn neuron_add_chance(&self) -> f32 {
        self.neuron_add_chance
    }
    fn neuron_remove_chance(&self) -> f32 {
        self.neuron_remove_chance
    }
    fn disable_herding(&self) -> bool {
        self.disable_herding
    }
}

/// Production seam: the aggregate simulation settings implement the brain's
/// mutation settings view (`docs/INTERFACES.md` ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â§4). Field paths are maintained
/// by fmt-dev's `settings.rs`.
impl BrainSettings for crate::settings::SimSettings {
    fn synapse_mutation_chance(&self) -> f32 {
        self.scenario.synapse_mutation_chance
    }
    fn synapse_add_chance(&self) -> f32 {
        self.scenario.synapse_add_chance
    }
    fn synapse_change_chance(&self) -> f32 {
        self.scenario.synapse_change_chance
    }
    fn synapse_flip_chance(&self) -> f32 {
        self.scenario.synapse_flip_chance
    }
    fn synapse_toggle_chance(&self) -> f32 {
        self.scenario.synapse_toggle_chance
    }
    fn synapse_remove_chance(&self) -> f32 {
        self.scenario.synapse_remove_chance
    }
    fn neuron_default_chance(&self) -> f32 {
        self.scenario.neuron_default_chance
    }
    fn neuron_change_chance(&self) -> f32 {
        self.scenario.neuron_change_chance
    }
    fn neuron_add_chance(&self) -> f32 {
        self.scenario.neuron_add_chance
    }
    fn neuron_remove_chance(&self) -> f32 {
        self.scenario.neuron_remove_chance
    }
    fn disable_herding(&self) -> bool {
        self.scenario.disable_herding
    }
}

/// The activation math shared by all nodes (`NEATBrain.ExecuteNeuron`,
/// `NEATBrain.cs:612-641`, plus `SoftLatchFunc`, `:1105-1133`).
///
/// Returns `(output, last_input)` where `output` is clamped to
/// `[-1024, 1024]` and `last_input` is the full activation `x` stored as
/// `Node.LastInput` (after bias handling).
#[inline]
pub(crate) fn activation(
    kind: NodeType,
    bias: f32,
    raw_input: f32,
    prev_out: f32,
    prev_in: f32,
    period: f32,
) -> (f32, f32) {
    // `if (!TypeUsesBiasDifferently(Type)) input += baseActivation; else if (Type == Mult) input *= baseActivation;` (`input` is a `float` parameter: the result is
    // stored as a `float`).
    let x = if kind.uses_bias_differently() {
        if kind == NodeType::Mult {
            raw_input * bias
        } else {
            raw_input
        }
    } else {
        raw_input + bias
    };
    (
        clamp(activation_function(kind, bias, x, prev_out, prev_in, period), -OUTPUT_CLAMP, OUTPUT_CLAMP),
        x,
    )
}

/// The `switch` of `ExecuteNeuron` on the node's full activation `x` (before the `Mathf.Clamp`).
///
/// The game runs on Mono, which does the arithmetic of a C# expression in `double` and rounds to `float` only where the value is stored in a variable or field,
/// passed to a method (`Mathf.Exp`, `Mathf.Pow`, `Mathf.Clamp`) or returned. Every expression below that holds more than one operation is therefore written as
/// ONE `double` expression with the roundings at those places; `f32` steps give 1 ulp differences that the nodes after then carry on (checked against the
/// original game with `tools/braincheck/game_diff`: Sigmoid 4,593, Gaussian 2,605, Integrator 653, Inhibitory 1,306 and SoftLatch 658 nodes, all bit-exact now;
/// with `f32` steps 3,841, 1,768, 649, 1,281 and 631 of them were). A single operation is the same either way (`double` holds the exact result of one
/// `f32` operation, and rounding that once is the `f32` operation).
#[inline]
fn activation_function(kind: NodeType, bias: f32, x: f32, prev_out: f32, prev_in: f32, period: f32) -> f32 {
    match kind {
        NodeType::Input | NodeType::Linear | NodeType::Mult => x,
        NodeType::Sigmoid => sigmoid(x),
        NodeType::TanH => tanh(x),
        NodeType::Sine => sin(x),
        NodeType::ReLu => relu(x),
        NodeType::Gaussian => gaussian(x),
        NodeType::Latch => {
            if x >= 1.0 {
                1.0
            } else if x <= 0.0 {
                0.0
            } else {
                prev_out
            }
        }
        // `(input - node.LastInput) / period`
        NodeType::Differential => ((f64::from(x) - f64::from(prev_in)) / f64::from(period)) as f32,
        NodeType::Abs => x.abs(),
        // `node.LastOutput + input * period`
        NodeType::Integrator => (f64::from(prev_out) + f64::from(x) * f64::from(period)) as f32,
        // `input - node.LastInput + node.LastOutput * Mathf.Exp((0f - node.baseActivation) * period)`: the `Exp` takes and returns a float.
        NodeType::Inhibitory => {
            ((f64::from(x) - f64::from(prev_in)) + f64::from(prev_out) * f64::from(exp(-bias * period))) as f32
        }
        NodeType::SoftLatch => soft_latch(bias, x, prev_out, prev_in),
    }
}

/// The activation functions alone, for the game-vs-port test (`brain_vs_game --acts`): `x` is the node's FULL activation (what the game keeps as `LastInput`: the sum
/// with the bias already added or multiplied in), and the result is the node's new output, clamped as `ExecuteNeuron` clamps it, and `x` again.
#[doc(hidden)]
#[must_use]
pub fn execute_neuron_from_activation(
    kind: NodeType,
    bias: f32,
    x: f32,
    prev_out: f32,
    prev_in: f32,
    period: f32,
) -> (f32, f32) {
    (
        clamp(activation_function(kind, bias, x, prev_out, prev_in, period), -OUTPUT_CLAMP, OUTPUT_CLAMP),
        x,
    )
}

/// `NEATBrain.ExecuteNeuron` on plain numbers, for a simulator that steps nodes one by one (the Bibite Editor's brain tab): returns
/// the node's new output (also its `Value` and `LastOutput`, clamped to +-1024) and the full activation it keeps as `LastInput`.
#[must_use]
pub fn execute_neuron(
    kind: NodeType,
    bias: f32,
    input: f32,
    last_output: f32,
    last_input: f32,
    period: f32,
) -> (f32, f32) {
    activation(kind, bias, input, last_output, last_input, period)
}

/// `NEATBrain.SoftLatchFunc` (`NEATBrain.cs:1105-1133`), exact branch structure and the game's evaluation (see [`activation_function`]): the `Mathf.Exp` calls
/// take and return floats, `num3` is a float local, the rest of each `return` expression is one `double` expression.
#[inline]
pub(crate) fn soft_latch(bias: f32, x: f32, last_output: f32, last_input: f32) -> f32 {
    if x >= 0.999 {
        return 1.0;
    }
    if x <= 0.001 {
        return 0.0;
    }
    let mut li = last_input;
    if li >= 0.999 {
        li = 1.0;
    }
    if li <= 0.001 {
        li = 0.0;
    }
    let lo = f64::from(last_output);
    if x < li {
        // `lastOutput / (1f - Mathf.Exp((0f - baseActivation) * num)) * (1f - Mathf.Exp((0f - baseActivation) * x))`
        let a = f64::from(exp(-bias * li));
        let c = f64::from(exp(-bias * x));
        return (lo / (1.0 - a) * (1.0 - c)) as f32;
    }
    // `baseActivation * (num - 1f)` is one double expression handed to `Mathf.Exp` as a float.
    let e1 = f64::from(exp((f64::from(bias) * (f64::from(li) - 1.0)) as f32));
    let num3 = ((lo - e1) / (1.0 - e1)) as f32; // a float local
    let e2 = f64::from(exp((f64::from(bias) * (f64::from(x) - 1.0)) as f32));
    (f64::from(num3) + (1.0 - f64::from(num3)) * e2) as f32
}

/// One node of the compiled plan that has incoming synapses: its edges are `plan_edges[begin..end]`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PlanGroup {
    target: u32,
    begin: u32,
    end: u32,
    bias: f32,
    kind: NodeType,
}

/// One enabled synapse of the compiled plan.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PlanEdge {
    source: u32,
    weight: f32,
}

/// A NEAT brain: node/synapse SoA plus a CSR execution plan.
///
/// Construct with [`Brain::starter`], [`Brain::from_records`] (runtime snapshot
/// semantics: state fields honoured) or [`Brain::from_template_records`]
/// (template semantics: state zeroed and bias-initialised like the game's
/// `FinalizeEditing`).
#[derive(Clone, Debug, PartialEq)]
#[repr(C)]
pub struct Brain {
    /// `Node.Value`: current output.
    value: Vec<f32>,
    /// `Node.LastOutput`: value readable by the network this tick.
    last_out: Vec<f32>,
    /// `Node.LastInput`: previous full activation `x`.
    last_in: Vec<f32>,
    /// The compiled execution plan: one record for each node that has incoming synapses, in the order of `targets` (what
    /// `step` reads instead of `targets`, `kind`, `bias`, `offsets`, `target_syns`, `syn_weight` and `syn_in`: two small
    /// arrays instead of seven scattered ones). Rebuilt with the CSR plan; an edit of a kind, a bias or a weight marks
    /// `plan_dirty`.
    plan_groups: Vec<PlanGroup>,
    /// The edges of the compiled plan: source node and weight of each enabled synapse, grouped by target, in CSR order.
    plan_edges: Vec<PlanEdge>,
    /// Cached bitmask of inputs with at least one outgoing synapse (enabled or
    /// not ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â the game's `Node.NOut > 0`, `NEATBrain.cs:467-507`).
    used_input_mask: u64,
    /// Set when topology changed and the CSR plan/masks need a rebuild before
    /// the next step (`Brain::ensure_plan`).
    plan_dirty: bool,
    /// Cached bitmask of outputs with at least one incoming synapse.
    used_output_mask: u16,
    herding_enabled: bool,

    kind: Vec<NodeType>,
    bias: Vec<f32>,
    innovation: Vec<i64>,
    custom_desc: Vec<Option<Box<str>>>,
    syn_in: Vec<u32>,
    syn_out: Vec<u32>,
    syn_weight: Vec<f32>,
    syn_enabled: Vec<bool>,
    syn_innovation: Vec<i64>,
    targets: Vec<u32>,
    offsets: Vec<u32>,
    target_syns: Vec<u32>,
    in_degree: Vec<u16>,
    out_degree: Vec<u16>,
    node_max_innovation: i64,
    connection_max_innovation: i64,
    /// Innovation numbers of synapses disabled by an `AddNeuron` split during
    /// the current mutation pass. The game's `disabledSynapses` candidate list
    /// is only built at copy time / rebuilt after removals and toggled, so
    /// these are not removal candidates until then (`NEATBrain.cs:391-398,
    /// 758-763, 996-1003`). Always empty outside [`Brain::mutate`].
    split_disabled: Vec<i64>,
}

impl Brain {
    /// The official "Basic bibite" starter brain: 48 nodes, 3 enabled synapses,
    /// captured from `testdata/real/templates_current/Basic_bibite_0.6.3.1.bb8template`.
    ///
    /// Synapses: `Fullness(3) -> Digestion(39) w=4.07295752`,
    /// `PlantAngle(13) -> Rotate(34) w=1.15925133`,
    /// `PlantCloseness(12) -> Accelerate(33) w=-0.401356`.
    /// Output biases: `Accelerate=0.4535929`, `EggProduction=0.2`,
    /// `Want2Eat=1.228`, `Digestion=-2.06689548`; all other biases 0.
    pub fn starter() -> Self {
        Self::starter_with_period(DEFAULT_BRAIN_PERIOD)
    }

    /// [`Brain::starter`] with an explicit period for the initial activation.
    pub fn starter_with_period(period: f32) -> Self {
        let mut nodes = Vec::with_capacity(FIRST_HIDDEN);
        for (i, input) in ALL_INPUTS.iter().enumerate() {
            nodes.push(NodeRecord {
                node_type: NodeType::Input,
                index: i as u32,
                innovation: i as i64 + 1,
                desc: Some(input.name().to_string()),
                base_activation: 0.0,
                value: 0.0,
                last_input: 0.0,
                last_output: 0.0,
            });
        }
        let biases = [
            0.453_592_9_f32, // Accelerate
            0.0,             // Rotate
            0.0,             // Herding
            0.2,             // EggProduction
            0.0,             // Want2Lay
            1.228,           // Want2Eat
            -2.066_895_5,    // Digestion
            0.0,             // Grab
            0.0,             // ClkReset
            0.0,             // PhereOut1
            0.0,             // PhereOut2
            0.0,             // PhereOut3
            0.0,             // Want2Grow
            0.0,             // Want2Heal
            0.0,             // Want2Attack
        ];
        for (i, output) in ALL_OUTPUTS.iter().enumerate() {
            let index = FIRST_OUTPUT + i;
            nodes.push(NodeRecord {
                node_type: output.node_type(),
                index: index as u32,
                innovation: index as i64 + 1,
                desc: Some(output.name().to_string()),
                base_activation: biases[i],
                value: 0.0,
                last_input: 0.0,
                last_output: 0.0,
            });
        }
        let synapses = [
            SynapseRecord {
                innovation: 1,
                node_in: Input::Fullness.index() as u32,
                node_out: FIRST_OUTPUT as u32 + Output::Digestion.index() as u32,
                weight: 4.072_957_5,
                enabled: true,
            },
            SynapseRecord {
                innovation: 2,
                node_in: Input::PlantAngle.index() as u32,
                node_out: FIRST_OUTPUT as u32 + Output::Rotate.index() as u32,
                weight: 1.159_251_3,
                enabled: true,
            },
            SynapseRecord {
                innovation: 3,
                node_in: Input::PlantCloseness.index() as u32,
                node_out: FIRST_OUTPUT as u32 + Output::Accelerate.index() as u32,
                weight: -0.401_356,
                enabled: true,
            },
        ];
        let mut brain = Self::from_template_records(&nodes, &synapses, period)
            .expect("starter brain records are valid");
        brain.node_max_innovation = FIRST_HIDDEN as i64 + 1; // 49; next hidden gets 50
        brain.connection_max_innovation = 4; // next new synapse gets Inov 4
        brain
    }

    /// Builds a brain from plain records, keeping their runtime state
    /// (`value`/`last_input`/`last_output`) ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â the `.bb8`/save-bundle shape.
    ///
    /// CSR and degree counters are rebuilt and floating hidden nodes are
    /// repaired (game `ComputeActiveNeurons`).
    pub fn from_records(nodes: &[NodeRecord], synapses: &[SynapseRecord]) -> Result<Self> {
        let mut brain = Self::build(nodes, synapses, false)?;
        brain.repair_floating_hidden();
        Ok(brain)
    }

    /// A node with no enabled incoming synapse is never run (`brainTargets` holds only the nodes some enabled synapse ends in), so in the game it holds what `FinalizeEditing`
    /// gave it for ever: `f(0 + bias)` (measured on the original game's own saves: 4,881 such nodes of 622 evolved bibites, none holds anything else). A save the port wrote
    /// while a child kept its parent's running node values (before [`Brain::reset_runtime_state`]) can hold the parent's last output in such a node, a constant pheromone or
    /// herding output with the synapse disabled and no bias; this puts those back. A value that already is `f(bias)` (every value a game save holds, to a rounding) is left alone.
    ///
    /// The world calls this when it loads a bibite from a save; [`Brain::from_records`] itself keeps the records' node state exactly (the replay of the game's brain ticks and the
    /// editor depend on that).
    pub fn heal_unfed_nodes(&mut self) {
        let mut fed = vec![false; self.kind.len()];
        for (index, &enabled) in self.syn_enabled.iter().enumerate() {
            if enabled && let Some(slot) = fed.get_mut(self.syn_out[index] as usize) {
                *slot = true;
            }
        }
        for i in FIRST_OUTPUT..self.kind.len() {
            if fed[i] {
                continue;
            }
            let (fresh, last_in) = activation(self.kind[i], self.bias[i], 0.0, 0.0, 0.0, 1.0);
            let tolerance = 1e-4 * fresh.abs().max(1.0);
            let close = |held: f32| (held - fresh).abs() <= tolerance || held.to_bits() == fresh.to_bits();
            if !close(self.value[i]) || !close(self.last_out[i]) {
                self.value[i] = fresh;
                self.last_out[i] = fresh;
                self.last_in[i] = last_in;
            }
        }
    }

    /// Builds a brain from records exactly as they are: no repair of a floating hidden node and no clamp of a soft latch's bias.
    /// This is what the Bibite Editor saves (`BibiteEditorPanel.SaveTemplate` keeps the edited nodes and connections; the repair is
    /// `ComputeActiveNeurons`'s, and happens when a brain is loaded to run).
    pub fn from_records_unrepaired(
        nodes: &[NodeRecord],
        synapses: &[SynapseRecord],
    ) -> Result<Self> {
        Self::build(nodes, synapses, false)
    }

    /// Builds a brain from template records: runtime state is discarded, then
    /// every non-input node is executed once with `input = 0` and `period`,
    /// exactly like `NEATBrain.FinalizeEditing` (`NEATBrain.cs:453-460`).
    pub fn from_template_records(
        nodes: &[NodeRecord],
        synapses: &[SynapseRecord],
        period: f32,
    ) -> Result<Self> {
        let mut brain = Self::build(nodes, synapses, true)?;
        brain.repair_floating_hidden();
        brain.initialize_states(period);
        Ok(brain)
    }

    fn build(nodes: &[NodeRecord], synapses: &[SynapseRecord], zero_state: bool) -> Result<Self> {
        if nodes.len() < FIRST_HIDDEN {
            return Err(Error::invalid_data(format!(
                "brain has {} nodes, expected at least {FIRST_HIDDEN}",
                nodes.len()
            )));
        }
        if nodes.len() > MAX_NODES {
            return Err(Error::invalid_data(format!(
                "brain has {} nodes, cap is {MAX_NODES}",
                nodes.len()
            )));
        }
        if synapses.len() > MAX_SYNAPSES {
            return Err(Error::invalid_data(format!(
                "brain has {} synapses, cap is {MAX_SYNAPSES}",
                synapses.len()
            )));
        }
        let n = nodes.len();
        let mut kind = Vec::with_capacity(n);
        let mut bias = Vec::with_capacity(n);
        let mut innovation = Vec::with_capacity(n);
        let mut custom_desc: Vec<Option<Box<str>>> = Vec::with_capacity(n);
        let mut value = vec![0.0_f32; n];
        let mut last_out = vec![0.0_f32; n];
        let mut last_in = vec![0.0_f32; n];
        let mut node_max = FIRST_HIDDEN as i64 + 1;
        for (i, node) in nodes.iter().enumerate() {
            if i < N_INPUTS && node.node_type != NodeType::Input {
                return Err(Error::invalid_data(format!(
                    "node {i} is in the input range but has type {}",
                    node.node_type.name()
                )));
            }
            if (N_INPUTS..FIRST_HIDDEN).contains(&i) && node.node_type == NodeType::Input {
                return Err(Error::invalid_data(format!(
                    "output node {i} has Input activation"
                )));
            }
            kind.push(node.node_type);
            bias.push(node.base_activation);
            innovation.push(node.innovation);
            custom_desc.push(if i < FIRST_HIDDEN {
                None
            } else {
                node.desc.as_deref().map(Box::from)
            });
            if !zero_state {
                value[i] = node.value;
                last_out[i] = node.last_output;
                last_in[i] = node.last_input;
            }
            node_max = node_max.max(node.innovation);
        }
        let mut syn_in = Vec::with_capacity(synapses.len());
        let mut syn_out = Vec::with_capacity(synapses.len());
        let mut syn_weight = Vec::with_capacity(synapses.len());
        let mut syn_enabled = Vec::with_capacity(synapses.len());
        let mut syn_innovation = Vec::with_capacity(synapses.len());
        let mut conn_max = 1_i64;
        for (i, syn) in synapses.iter().enumerate() {
            let a = syn.node_in as usize;
            let b = syn.node_out as usize;
            if a >= n || b >= n {
                return Err(Error::invalid_data(format!(
                    "synapse {i} endpoints ({a}->{b}) out of range for {n} nodes"
                )));
            }
            syn_in.push(syn.node_in);
            syn_out.push(syn.node_out);
            syn_weight.push(syn.weight);
            syn_enabled.push(syn.enabled);
            syn_innovation.push(syn.innovation);
            conn_max = conn_max.max(syn.innovation + 1);
        }
        let mut brain = Self {
            kind,
            bias,
            innovation,
            custom_desc,
            value,
            last_out,
            last_in,
            syn_in,
            syn_out,
            syn_weight,
            syn_enabled,
            syn_innovation,
            targets: Vec::new(),
            offsets: Vec::new(),
            target_syns: Vec::new(),
            plan_groups: Vec::new(),
            plan_edges: Vec::new(),
            in_degree: vec![0; n],
            out_degree: vec![0; n],
            node_max_innovation: node_max,
            connection_max_innovation: conn_max,
            herding_enabled: true,
            used_input_mask: 0,
            used_output_mask: 0,
            plan_dirty: false,
            split_disabled: Vec::new(),
        };
        brain.recompute_degrees();
        brain.rebuild_csr();
        Ok(brain)
    }

    /// Number of nodes.
    #[inline]
    pub fn node_count(&self) -> usize {
        self.kind.len()
    }

    /// Number of synapses (enabled or not).
    #[inline]
    pub fn synapse_count(&self) -> usize {
        self.syn_in.len()
    }

    /// Number of hidden nodes.
    #[inline]
    pub fn hidden_count(&self) -> usize {
        self.kind.len() - FIRST_HIDDEN
    }

    /// Upkeep counters for the body.
    #[inline]
    pub fn upkeep(&self) -> BrainUpkeep {
        BrainUpkeep {
            synapses: self.synapse_count() as u32,
            hidden: self.hidden_count() as u32,
        }
    }

    /// Next hidden-node innovation number (game `nodeMaxInov`).
    #[inline]
    pub fn node_max_innovation(&self) -> i64 {
        self.node_max_innovation
    }

    /// Next connection innovation number (game `connectionMaxInov`).
    #[inline]
    pub fn connection_max_innovation(&self) -> i64 {
        self.connection_max_innovation
    }

    /// Seeds the innovation counters from world/lineage state (io load).
    pub fn set_innovations(&mut self, node_max: i64, connection_max: i64) {
        self.node_max_innovation = node_max.max(FIRST_HIDDEN as i64 + 1);
        self.connection_max_innovation = connection_max.max(1);
    }

    /// Enables/disables herding for CSR construction (game `herdingEnabled`).
    pub fn set_herding_enabled(&mut self, enabled: bool) {
        if self.herding_enabled != enabled {
            self.herding_enabled = enabled;
            self.plan_dirty = true;
        }
    }

    /// Whether herding synapses are included in execution.
    #[inline]
    pub fn herding_enabled(&self) -> bool {
        self.herding_enabled
    }

    /// Whether the CSR execution plan needs rebuilding before the next
    /// [`crate::brain::Brain::step`] (set by topology or herding changes).
    ///
    /// [`crate::brain::Brain::step`] rebuilds lazily, but callers running brains on worker
    /// threads (e.g. the body's parallel brain pass) can check this first and
    /// call [`Brain::rebuild`] on the owning thread to keep the workers
    /// allocation-free.
    #[inline]
    pub fn plan_dirty(&self) -> bool {
        self.plan_dirty
    }

    /// Rebuilds the CSR execution plan (and usage masks) after topology or
    /// herding changes.
    pub fn rebuild(&mut self) {
        self.recompute_degrees();
        self.rebuild_csr();
        self.plan_dirty = false;
    }

    fn ensure_plan(&mut self) {
        if self.plan_dirty {
            self.rebuild();
        }
    }

    // -----------------------------------------------------------------------
    // Sensing-usage masks (game `ComputeActiveNeurons`/`StartUsedSystems`)
    // -----------------------------------------------------------------------

    /// Bitmask of inputs with at least one outgoing synapse (enabled or not) ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â
    /// the game's `Node.NOut > 0` test used to decide which senses to compute
    /// (`NEATBrain.cs:467-551`, spec ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â§4.3).
    ///
    /// Valid after any public mutation (mutations rebuild the plan) and after
    /// load; call [`Brain::rebuild`] if you edit topology through internal
    /// helpers in tests.
    #[inline]
    pub fn used_input_mask(&self) -> u64 {
        self.used_input_mask
    }

    /// Whether an input has at least one outgoing synapse (`Node.NOut > 0`).
    #[inline]
    pub fn uses_input(&self, input: Input) -> bool {
        self.used_input_mask & input_bit(input) != 0
    }

    /// Bitmask of outputs with at least one incoming synapse (enabled or not).
    #[inline]
    pub fn used_output_mask(&self) -> u16 {
        self.used_output_mask
    }

    /// Whether an output has at least one incoming synapse (`Node.NIn > 0`).
    #[inline]
    pub fn uses_output(&self, output: Output) -> bool {
        self.used_output_mask & output_bit(output) != 0
    }

    /// `StartUsedSystems` bibite-vision rule: the bibite category is computed if
    /// any bibite input is wired **or** the Herding output is wired/biased while
    /// herding is enabled (`NEATBrain.cs:542`).
    pub fn needs_bibite_vision(&self) -> bool {
        let herding_used = self.herding_enabled
            && (self.uses_output(Output::Herding) || self.bias[HERDING_OUTPUT_NODE] != 0.0);
        herding_used || self.used_input_mask & VISION_BIBITE_INPUT_MASK != 0
    }

    /// Plant-vision category is needed if any plant input is wired
    /// (`NEATBrain.cs:543`).
    #[inline]
    pub fn needs_plant_vision(&self) -> bool {
        self.used_input_mask & VISION_PLANT_INPUT_MASK != 0
    }

    /// Meat-vision category is needed if any meat input is wired
    /// (`NEATBrain.cs:544`).
    #[inline]
    pub fn needs_meat_vision(&self) -> bool {
        self.used_input_mask & VISION_MEAT_INPUT_MASK != 0
    }

    /// Pheromone sensing is needed if any pheromone input is wired
    /// (`StartPherosensing`, `NEATBrain.cs:535-538`).
    #[inline]
    pub fn needs_pheromone_sensing(&self) -> bool {
        self.used_input_mask & PHEROMONE_INPUT_MASK != 0
    }

    // -----------------------------------------------------------------------
    // Parity accessors (docs/PARITY.md ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â§6.3)
    // -----------------------------------------------------------------------

    /// Number of inputs (constant 33).
    #[inline]
    pub fn input_count(&self) -> usize {
        N_INPUTS
    }

    /// Number of outputs (constant 15).
    #[inline]
    pub fn output_count(&self) -> usize {
        N_OUTPUTS
    }

    /// Reads a node's runtime state (`Value`/`LastInput`/`LastOutput`).
    #[inline]
    pub fn node_state(&self, index: usize) -> Option<NodeRuntimeState> {
        if index >= self.kind.len() {
            return None;
        }
        Some(NodeRuntimeState {
            value: self.value[index],
            last_input: self.last_in[index],
            last_output: self.last_out[index],
        })
    }

    /// Restores a node's runtime state (save loading / parity replay). Returns
    /// false if the index is out of range.
    #[inline]
    pub fn set_node_state(&mut self, index: usize, state: NodeRuntimeState) -> bool {
        if index >= self.kind.len() {
            return false;
        }
        self.value[index] = state.value;
        self.last_in[index] = state.last_input;
        self.last_out[index] = state.last_output;
        true
    }

    /// Current `Node.Value` of a node, or `None` if out of range.
    #[inline]
    pub fn node_value(&self, index: usize) -> Option<f32> {
        self.value.get(index).copied()
    }

    /// Copies all output node values in [`Output`] order (bulk read for the
    /// body organ pass; one array copy instead of N indexed calls).
    #[inline]
    pub fn output_values_copy(&self) -> [f32; N_OUTPUTS] {
        let mut values = [0.0f32; N_OUTPUTS];
        values.copy_from_slice(&self.value[FIRST_OUTPUT..FIRST_OUTPUT + N_OUTPUTS]);
        values
    }

    /// Current value of an output node (parity `output_digest`).
    #[inline]
    pub fn output_value(&self, output: Output) -> f32 {
        self.value[FIRST_OUTPUT + output.index()]
    }

    /// Sets an output node's `Value` (parity replay/debug). Note this does not
    /// update `LastOutput`; use [`Brain::set_node_state`] for exact restoration.
    #[inline]
    pub fn set_output_value(&mut self, output: Output, value: f32) {
        self.value[FIRST_OUTPUT + output.index()] = value;
    }

    /// Current value of an input node (parity `sense_digest`).
    #[inline]
    pub fn input_value(&self, input: Input) -> f32 {
        self.value[input.index()]
    }

    /// Bias (`baseActivation`) of an output node.
    #[inline]
    pub fn output_bias(&self, output: Output) -> f32 {
        self.bias[FIRST_OUTPUT + output.index()]
    }

    /// Sets an output node's bias (debug/editor).
    #[inline]
    pub fn set_output_bias(&mut self, output: Output, bias: f32) {
        self.bias[FIRST_OUTPUT + output.index()] = bias;
        self.plan_dirty = true;
    }

    /// Adds an empty hidden node for the brain editor.
    ///
    /// The node is disconnected (like the game's `BibiteBrainEditor.AddNode`
    /// before the user draws connections), gets the next node innovation and a
    /// default bias (`Mult`/`Inhibitory` = 1, `SoftLatch` = 5, otherwise 0; game
    /// `NEATBrain.DefaultBaseActivationOfType`). Returns the new array index, or
    /// `None` for an input type or when the node cap is reached.
    pub fn add_hidden_node(&mut self, node_type: NodeType, label: Option<&str>) -> Option<usize> {
        if node_type == NodeType::Input || self.kind.len() >= MAX_NODES {
            return None;
        }
        let index = self.kind.len();
        let bias = match node_type {
            NodeType::Mult | NodeType::Inhibitory => 1.0,
            NodeType::SoftLatch => 5.0,
            _ => 0.0,
        };
        let desc = label
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Hidden{}", self.hidden_count()));
        self.kind.push(node_type);
        self.bias.push(bias);
        self.innovation.push(self.node_max_innovation);
        self.node_max_innovation += 1;
        self.custom_desc.push(Some(desc.into_boxed_str()));
        self.value.push(0.0);
        self.last_out.push(0.0);
        self.last_in.push(0.0);
        self.in_degree.push(0);
        self.out_degree.push(0);
        self.plan_dirty = true;
        self.rebuild();
        Some(index)
    }

    /// Removes a hidden node and every synapse touching it, renumbering later
    /// nodes and synapse endpoints. Inputs/outputs cannot be removed. Returns
    /// false for a base/out-of-range index.
    pub fn remove_hidden_node(&mut self, index: usize) -> bool {
        if index < FIRST_HIDDEN || index >= self.kind.len() {
            return false;
        }
        self.remove_node_compact(index);
        true
    }

    /// Adds an explicit editor connection.
    ///
    /// Validation mirrors the editor controls: endpoints must exist, a synapse
    /// may not target an input, outputs may not be sources, self-connections are
    /// rejected, and duplicate endpoint pairs are rejected. Returns the new
    /// synapse array index. Weight/enabled are editor values, not mutations.
    pub fn add_synapse_connection(
        &mut self,
        node_in: usize,
        node_out: usize,
        weight: f32,
        enabled: bool,
    ) -> Option<usize> {
        if node_in >= self.kind.len()
            || node_out >= self.kind.len()
            || node_out < FIRST_OUTPUT
            || node_in == node_out
            || (FIRST_OUTPUT..FIRST_HIDDEN).contains(&node_in)
            || self.find_synapse(node_in as u32, node_out as u32).is_some()
            || self.synapse_count() >= MAX_SYNAPSES
        {
            return None;
        }
        Some(self.push_synapse(node_in as u32, node_out as u32, weight, enabled))
    }

    /// Removes one explicit editor connection without the game's mutation
    /// cascade. Returns false for an out-of-range index.
    pub fn remove_synapse_at(&mut self, index: usize) -> bool {
        if index >= self.synapse_count() {
            return false;
        }
        self.compact_synapses_where(|s, _, _| s == index);
        self.recompute_degrees();
        self.rebuild_csr();
        self.plan_dirty = false;
        true
    }

    /// Sets one synapse weight for the editor. Returns false if out of range.
    pub fn set_synapse_weight(&mut self, index: usize, weight: f32) -> bool {
        let Some(slot) = self.syn_weight.get_mut(index) else {
            return false;
        };
        *slot = weight;
        self.plan_dirty = true;
        true
    }

    /// Sets one synapse enabled flag for the editor and refreshes the execution
    /// plan on the next step.
    pub fn set_synapse_enabled(&mut self, index: usize, enabled: bool) -> bool {
        let Some(slot) = self.syn_enabled.get_mut(index) else {
            return false;
        };
        *slot = enabled;
        self.plan_dirty = true;
        true
    }

    /// Sets any node bias for the editor (base outputs included; use
    /// [`Brain::set_output_bias`] for the typed output accessor).
    pub fn set_node_bias_at(&mut self, index: usize, bias: f32) -> bool {
        let Some(slot) = self.bias.get_mut(index) else {
            return false;
        };
        *slot = bias;
        self.plan_dirty = true;
        true
    }

    /// Sets a hidden node's activation type. Inputs/outputs keep their fixed
    /// types, mirroring the node editor. Returns false for a base/out-of-range
    /// index or [`NodeType::Input`].
    pub fn set_hidden_node_type(&mut self, index: usize, node_type: NodeType) -> bool {
        if index < FIRST_HIDDEN || index >= self.kind.len() || node_type == NodeType::Input {
            return false;
        }
        self.kind[index] = node_type;
        self.plan_dirty = true;
        true
    }

    /// Sets a hidden node's display description. Returns false for a
    /// base/out-of-range index.
    pub fn set_hidden_node_label(&mut self, index: usize, label: &str) -> bool {
        if index < FIRST_HIDDEN || index >= self.kind.len() {
            return false;
        }
        self.custom_desc[index] = Some(label.to_owned().into_boxed_str());
        true
    }

    /// Node type at an index.
    #[inline]
    pub fn node_type(&self, index: usize) -> Option<NodeType> {
        self.kind.get(index).copied()
    }

    /// Innovation number of a node.
    #[inline]
    pub fn node_innovation(&self, index: usize) -> Option<i64> {
        self.innovation.get(index).copied()
    }

    /// Read-only brain digest input: `(node_count, synapse_count,
    /// enabled_synapse_count, hidden_count)`.
    #[inline]
    pub fn digest_counts(&self) -> (usize, usize, usize, usize) {
        (
            self.node_count(),
            self.synapse_count(),
            self.syn_enabled.iter().filter(|enabled| **enabled).count(),
            self.hidden_count(),
        )
    }

    /// Exports nodes as plain records (`.bb8` runtime shape).
    pub fn to_node_records(&self) -> Vec<NodeRecord> {
        (0..self.node_count())
            .map(|i| NodeRecord {
                node_type: self.kind[i],
                index: i as u32,
                innovation: self.innovation[i],
                desc: self.node_desc(i).map(str::to_string),
                base_activation: self.bias[i],
                value: self.value[i],
                last_input: self.last_in[i],
                last_output: self.last_out[i],
            })
            .collect()
    }

    /// Exports synapses as plain records.
    pub fn to_synapse_records(&self) -> Vec<SynapseRecord> {
        (0..self.synapse_count())
            .map(|i| SynapseRecord {
                innovation: self.syn_innovation[i],
                node_in: self.syn_in[i],
                node_out: self.syn_out[i],
                weight: self.syn_weight[i],
                enabled: self.syn_enabled[i],
            })
            .collect()
    }

    /// Complete read-only graph snapshot for brain panels.
    ///
    /// This is a copy: the viewer can hold it while the simulation continues.
    #[must_use]
    pub fn graph_view(&self) -> BrainGraphView {
        let outputs = self.output_values_copy();
        let nodes = (0..self.node_count())
            .map(|i| BrainNodeView {
                index: i,
                node_type: self.kind[i],
                innovation: self.innovation[i],
                label: self.node_desc(i).unwrap_or("").to_string(),
                base_activation: self.bias[i],
                value: self.value[i],
                last_input: self.last_in[i],
                last_output: self.last_out[i],
            })
            .collect();
        let synapses = (0..self.synapse_count())
            .map(|i| BrainSynapseView {
                index: i,
                innovation: self.syn_innovation[i],
                node_in: self.syn_in[i] as usize,
                node_out: self.syn_out[i] as usize,
                weight: self.syn_weight[i],
                enabled: self.syn_enabled[i],
            })
            .collect();
        BrainGraphView {
            nodes,
            synapses,
            outputs,
        }
    }

    /// Node description as serialized by the game: input/output enum names for
    /// base nodes, stored custom description for hidden nodes.
    pub fn node_desc(&self, index: usize) -> Option<&str> {
        if index < N_INPUTS {
            Some(Input::from_index(index)?.name())
        } else if index < FIRST_HIDDEN {
            Some(Output::from_index(index - FIRST_OUTPUT)?.name())
        } else {
            self.custom_desc[index].as_deref()
        }
    }

    /// Runs one brain tick. `period` is `brain_tps / sim_tps` seconds.
    pub fn step(&mut self, inputs: &BrainInputs, period: f32) -> BrainOutputs {
        self.ensure_plan();
        // `UpdateSenses`: previous input values become readable by the network,
        // then are replaced by the fresh samples.
        for i in 0..N_INPUTS {
            self.last_out[i] = self.value[i];
        }
        self.value[..N_INPUTS].copy_from_slice(inputs.values());

        #[cfg(test)]
        self.assert_plan_is_current();
        for group in &self.plan_groups {
            let target = group.target as usize;
            let kind = group.kind;
            let mut acc = if kind == NodeType::Mult { 1.0_f32 } else { 0.0 };
            for edge in &self.plan_edges[group.begin as usize..group.end as usize] {
                let term = edge.weight * self.last_out[edge.source as usize];
                if kind == NodeType::Mult {
                    acc *= term;
                } else {
                    acc += term;
                }
            }
            let (out, last_in) = activation(
                kind,
                group.bias,
                acc,
                self.last_out[target],
                self.last_in[target],
                period,
            );
            self.value[target] = out;
            self.last_out[target] = out;
            self.last_in[target] = last_in;
        }

        let mut values = [0.0_f32; N_OUTPUTS];
        values.copy_from_slice(&self.value[FIRST_OUTPUT..FIRST_HIDDEN]);
        BrainOutputs { values }
    }

    /// Steps a batch of **independent** brains with pre-assembled inputs, in
    /// caller order (brain-side half of the parallel brain-tick design).
    ///
    /// Intended body-side integration (see the task-42 hand-off and
    /// `docs/INTERFACES.md` ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â§4):
    ///
    /// 1. phase 5a (`World::step_sensing`) has already written every record's
    ///    world senses; the body then assembles each due record's
    ///    [`BrainInputs`] into one dense buffer, in dense slot order (assembly
    ///    reads only that record + immutable snapshots, so it is order-independent);
    /// 2. the body splits its record slice into disjoint `&mut` chunks with
    ///    `split_at_mut`, collects `&mut Brain` references per chunk, and calls
    ///    this helper from its worker pool;
    /// 3. output buffers are written in caller order and the body applies them
    ///    to organs in the same dense order as the serial loop.
    ///
    /// The result is bit-identical to calling [`crate::brain::Brain::step`] serially: each
    /// brain is fully independent (no shared state; `step` draws no RNG; all
    /// maths is per-brain f32), the pre-assembled inputs are identical, and
    /// every cross-brain effect (energy, position, collisions, births) is
    /// applied after the join in dense slot order.
    ///
    /// # Panics
    ///
    /// Panics if `brains`, `inputs` and `outputs` do not have equal lengths.
    pub fn step_batch(
        brains: &mut [&mut Brain],
        inputs: &[BrainInputs],
        period: f32,
        outputs: &mut [BrainOutputs],
    ) {
        assert_eq!(
            brains.len(),
            inputs.len(),
            "step_batch: brains/inputs length mismatch"
        );
        assert_eq!(
            brains.len(),
            outputs.len(),
            "step_batch: brains/outputs length mismatch"
        );
        for (index, brain) in brains.iter_mut().enumerate() {
            outputs[index] = brain.step(&inputs[index], period);
        }
    }

    /// Executes every non-input node once with `input = 0` (game
    /// `FinalizeEditing`, `NEATBrain.cs:453-460`).
    fn initialize_states(&mut self, period: f32) {
        for i in FIRST_OUTPUT..self.kind.len() {
            let (out, last_in) = activation(self.kind[i], self.bias[i], 0.0, 0.0, 0.0, period);
            self.value[i] = out;
            self.last_out[i] = out;
            self.last_in[i] = last_in;
        }
    }

    /// The state a brain has when the game has just built it for a new bibite or egg (`NEATBrain.CopyBrain(..)` then `FinalizeEditing()`): `CopyNode` copies only a node's
    /// structure (type, innovation, bias, ...), never its `Value`, `LastInput` or `LastOutput`, so the new node array starts at zero, and `FinalizeEditing` then runs every
    /// non-input node once with input 0 (`Value = f(0 + bias)`).
    ///
    /// A node with no enabled incoming synapse is never run again, so it rests at this value for ever: a child that kept its parent's live node values (a clone of the
    /// running brain) held the parent's last output in such a node for its whole life, for example a constant pheromone or herding output with the synapse disabled
    /// and no bias. Call this on every brain copied from a parent, an egg or a template; not on a brain loaded from a save (a save keeps its node values).
    pub fn reset_runtime_state(&mut self, period: f32) {
        for i in 0..FIRST_OUTPUT.min(self.kind.len()) {
            self.value[i] = 0.0;
            self.last_in[i] = 0.0;
            self.last_out[i] = 0.0;
        }
        self.initialize_states(period);
    }

    /// Re-runs the game's `FinalizeEditing`/`ResumeBrain` state initialisation
    /// on a loaded brain, without touching input node state.
    ///
    /// Runtime saves normally preserve every node's `Value`/`LastInput`/
    /// `LastOutput`. If a load applies a structural `BrainUpdater` migration
    /// (old save version), the migrated node array no longer has meaningful
    /// runtime values for displaced output/hidden nodes; the game re-executes
    /// all non-input nodes once with zero input. This method is that step.
    /// `period` is `brain_tps / sim_tps` (for migrated Default saves, 2/30).
    pub fn initialize_runtime_states(&mut self, period: f32) {
        self.initialize_states(period);
    }

    /// Removes hidden nodes with no incoming or no outgoing synapse, like the
    /// game's `ComputeActiveNeurons` repair path (`NEATBrain.cs:488-495`).
    ///
    /// Also performs the first half of that method: every `SoftLatch` node's
    /// bias is clamped to `[0.1, 100]` (`NEATBrain.cs:475-478`,
    /// `Mathf.Clamp`, which leaves NaN untouched).
    pub fn repair_floating_hidden(&mut self) {
        self.clamp_soft_latch_biases();
        self.repair_floating_hidden_impl();
    }

    /// `ComputeActiveNeurons` SoftLatch bias repair (`NEATBrain.cs:475-478`).
    fn clamp_soft_latch_biases(&mut self) {
        let mut changed = false;
        for (kind, bias) in self.kind.iter().zip(self.bias.iter_mut()) {
            if *kind == NodeType::SoftLatch {
                // Unity `Mathf.Clamp`: NaN passes through (as `f32::clamp` does).
                let clamped = (*bias).clamp(0.1, 100.0);
                changed |= clamped.to_bits() != bias.to_bits();
                *bias = clamped;
            }
        }
        if changed && !self.plan_dirty {
            self.compile_plan();
        }
    }

    /// [`Brain::repair_floating_hidden`] returning whether anything was removed
    /// (lets the mutation hot path skip a full rebuild when topology is intact).
    fn repair_floating_hidden_impl(&mut self) -> bool {
        let mut removed = false;
        loop {
            let floating = (FIRST_HIDDEN..self.kind.len())
                .find(|i| self.in_degree[*i] < 1 || self.out_degree[*i] < 1);
            let Some(index) = floating else { break };
            self.remove_node_compact(index);
            removed = true;
        }
        removed
    }

    /// Removes one hidden node and every synapse touching it, renumbering.
    fn remove_node_compact(&mut self, index: usize) {
        debug_assert!(index >= FIRST_HIDDEN);
        self.kind.remove(index);
        self.bias.remove(index);
        self.innovation.remove(index);
        self.custom_desc.remove(index);
        self.value.remove(index);
        self.last_out.remove(index);
        self.last_in.remove(index);
        self.in_degree.remove(index);
        self.out_degree.remove(index);
        mutation_counters::add_node_removed(1);

        self.compact_synapses_where(|_, a, b| a as usize == index || b as usize == index);
        for s in 0..self.syn_in.len() {
            if self.syn_in[s] as usize > index {
                self.syn_in[s] -= 1;
            }
            if self.syn_out[s] as usize > index {
                self.syn_out[s] -= 1;
            }
        }
        self.recompute_degrees();
        self.rebuild_csr();
        self.plan_dirty = false;
    }

    fn recompute_degrees(&mut self) {
        let n = self.kind.len();
        self.in_degree.clear();
        self.in_degree.resize(n, 0);
        self.out_degree.clear();
        self.out_degree.resize(n, 0);
        self.used_input_mask = 0;
        self.used_output_mask = 0;
        for s in 0..self.syn_in.len() {
            let a = self.syn_in[s] as usize;
            let b = self.syn_out[s] as usize;
            if a != b {
                self.out_degree[a] += 1;
                self.in_degree[b] += 1;
                if a < N_INPUTS {
                    self.used_input_mask |= 1_u64 << (a as u64);
                }
                if (FIRST_OUTPUT..FIRST_HIDDEN).contains(&b) {
                    self.used_output_mask |= 1_u16 << ((b - FIRST_OUTPUT) as u16);
                }
            }
        }
    }

    fn rebuild_csr(&mut self) {
        let n = self.kind.len();
        self.targets.clear();
        self.offsets.clear();
        self.target_syns.clear();
        // One scratch allocation instead of three (`slot`, `counts`, `cursor`);
        // these are hot on the birth path (every structural mutation).
        let mut scratch = vec![0_u32; n * 3];
        let (slot, rest) = scratch.split_at_mut(n);
        slot.fill(u32::MAX);
        let (counts, cursor) = rest.split_at_mut(n);
        for s in 0..self.syn_in.len() {
            if !self.syn_enabled[s] {
                continue;
            }
            let out = self.syn_out[s] as usize;
            if !self.herding_enabled && out == HERDING_SKIPPED_NODE {
                continue;
            }
            if slot[out] == u32::MAX {
                slot[out] = self.targets.len() as u32;
                self.targets.push(out as u32);
            }
            counts[out] += 1;
        }
        self.offsets.reserve(self.targets.len() + 1);
        self.offsets.push(0);
        let mut acc = 0_u32;
        for t in &self.targets {
            acc += counts[*t as usize];
            self.offsets.push(acc);
        }
        self.target_syns.resize(acc as usize, 0);
        cursor[..self.targets.len()].copy_from_slice(&self.offsets[..self.targets.len()]);
        for s in 0..self.syn_in.len() {
            if !self.syn_enabled[s] {
                continue;
            }
            let out = self.syn_out[s] as usize;
            if !self.herding_enabled && out == HERDING_SKIPPED_NODE {
                continue;
            }
            let g = slot[out] as usize;
            self.target_syns[cursor[g] as usize] = s as u32;
            cursor[g] += 1;
        }
        self.compile_plan();
    }

    /// Compiles `plan_groups` and `plan_edges` from the CSR plan and the current kinds, biases and weights.
    fn compile_plan(&mut self) {
        let (groups, edges) = self.compiled_plan();
        self.plan_groups = groups;
        self.plan_edges = edges;
    }

    /// The compiled plan as the CSR plan and the current kinds, biases and weights give it.
    fn compiled_plan(&self) -> (Vec<PlanGroup>, Vec<PlanEdge>) {
        let mut groups = Vec::with_capacity(self.targets.len());
        let mut edges = Vec::with_capacity(self.target_syns.len());
        for (g, &target) in self.targets.iter().enumerate() {
            let begin = edges.len() as u32;
            for k in self.offsets[g] as usize..self.offsets[g + 1] as usize {
                let s = self.target_syns[k] as usize;
                edges.push(PlanEdge {
                    source: self.syn_in[s],
                    weight: self.syn_weight[s],
                });
            }
            groups.push(PlanGroup {
                target,
                begin,
                end: edges.len() as u32,
                bias: self.bias[target as usize],
                kind: self.kind[target as usize],
            });
        }
        (groups, edges)
    }

    /// Tests: the compiled plan a step runs is the one the masters give (it was kept current by every edit).
    #[cfg(test)]
    fn assert_plan_is_current(&self) {
        if self.plan_dirty {
            return;
        }
        let (groups, edges) = self.compiled_plan();
        let same = groups.len() == self.plan_groups.len()
            && edges.len() == self.plan_edges.len()
            && groups.iter().zip(&self.plan_groups).all(|(a, b)| {
                a.target == b.target
                    && a.begin == b.begin
                    && a.end == b.end
                    && a.kind == b.kind
                    && a.bias.to_bits() == b.bias.to_bits()
            })
            && edges
                .iter()
                .zip(&self.plan_edges)
                .all(|(a, b)| a.source == b.source && a.weight.to_bits() == b.weight.to_bits());
        assert!(
            same,
            "the compiled plan of a brain is stale: an edit of a bias, a kind or a weight did not mark it"
        );
    }

    /// Validates a draft brain for the editor.
    ///
    /// Base input/output archetypes are fixed, endpoints must be non-self,
    /// unique, and oriented input/hidden → hidden/output; hidden nodes may be
    /// left floating while editing. Returns advisory [`BrainEditorValidation`]
    /// rather than an error so the viewer can show inline warnings.
    #[must_use]
    pub fn validate_editor(&self) -> BrainEditorValidation {
        let mut errors = Vec::new();
        if self.kind.len() < FIRST_HIDDEN {
            errors.push(format!(
                "brain has {} nodes; at least {FIRST_HIDDEN} base nodes are required",
                self.kind.len()
            ));
        }
        for index in 0..self.kind.len().min(N_INPUTS) {
            if self.kind[index] != NodeType::Input {
                errors.push(format!("input node {index} must have type Input"));
            }
        }
        for index in FIRST_OUTPUT..self.kind.len().min(FIRST_HIDDEN) {
            if self.kind[index] == NodeType::Input {
                errors.push(format!("output node {index} cannot have type Input"));
            }
        }
        for index in FIRST_HIDDEN..self.kind.len() {
            if self.kind[index] == NodeType::Input {
                errors.push(format!("hidden node {index} cannot have type Input"));
            }
        }

        let mut in_degree = vec![0_u32; self.kind.len()];
        let mut out_degree = vec![0_u32; self.kind.len()];
        for s in 0..self.synapse_count() {
            let node_in = self.syn_in[s] as usize;
            let node_out = self.syn_out[s] as usize;
            if node_in >= self.kind.len() || node_out >= self.kind.len() {
                errors.push(format!("synapse {s} endpoints are out of range"));
                continue;
            }
            if node_in == node_out {
                errors.push(format!("synapse {s} connects node {node_in} to itself"));
            }
            if node_out < FIRST_OUTPUT {
                errors.push(format!("synapse {s} targets input node {node_out}"));
            }
            if (FIRST_OUTPUT..FIRST_HIDDEN).contains(&node_in) {
                errors.push(format!("synapse {s} starts at output node {node_in}"));
            }
            for earlier in 0..s {
                if self.syn_in[earlier] == self.syn_in[s]
                    && self.syn_out[earlier] == self.syn_out[s]
                {
                    errors.push(format!(
                        "synapse {s} duplicates endpoints of synapse {earlier}"
                    ));
                    break;
                }
            }
            if node_in != node_out {
                in_degree[node_out] += 1;
                out_degree[node_in] += 1;
            }
        }
        let floating_hidden = (FIRST_HIDDEN..self.kind.len())
            .filter(|index| in_degree[*index] == 0 || out_degree[*index] == 0)
            .collect();
        BrainEditorValidation {
            errors,
            floating_hidden,
        }
    }

    /// Convenience bool for [`Brain::validate_editor`].
    #[must_use]
    pub fn editor_is_valid(&self) -> bool {
        self.validate_editor().is_valid()
    }

    /// Sanity-checks structural invariants (io boundary and tests).
    pub fn validate(&self) -> Result<()> {
        if self.kind.len() < FIRST_HIDDEN {
            return Err(Error::invalid_data("brain has fewer than 48 nodes"));
        }
        if self.kind.len() != 48 + self.hidden_count() {
            return Err(Error::invalid_data("node count/hidden count mismatch"));
        }
        for s in 0..self.synapse_count() {
            if self.syn_in[s] as usize >= self.kind.len()
                || self.syn_out[s] as usize >= self.kind.len()
            {
                return Err(Error::invalid_data(format!(
                    "synapse {s} endpoints out of range"
                )));
            }
        }
        for i in FIRST_HIDDEN..self.kind.len() {
            if self.in_degree[i] < 1 || self.out_degree[i] < 1 {
                return Err(Error::invalid_data(format!("hidden node {i} is floating")));
            }
        }
        if !self.offsets.is_empty()
            && self.offsets.last().copied().unwrap_or(0) as usize != self.target_syns.len()
        {
            return Err(Error::internal("CSR offsets do not cover target synapses"));
        }
        Ok(())
    }

    /// Mutates the brain in place with the game's two-level mutation algorithm
    /// (`NEATBrain.cs:643-961`).
    ///
    /// Count is `Poisson(BrainAverageMutation + backgroundMutationChance +
    /// factor.probabilityIncrease)`; each event is a synapse mutation with
    /// probability `synapseMutationChance`, otherwise a neuron mutation.
    /// Deterministic given `rng`.
    pub fn mutate<S>(&mut self, rng: &mut Rng, s: &S, genome: &Genome, factor: &MutationFactor)
    where
        S: BrainSettings + GenomeSettings,
    {
        let herding_changed = self.herding_enabled != !s.disable_herding();
        self.herding_enabled = !s.disable_herding();
        if s.prevent_mutations() {
            // A settings animator may have flipped `disableHerding`; keep the
            // lazy plan contract (`step` rebuilds) instead of leaving it stale.
            if herding_changed {
                self.plan_dirty = true;
            }
            return;
        }
        let lambda = genome.gene(genome::Gene::BrainAverageMutation)
            + s.background_mutation_chance()
            + factor.probability_increase;
        let sigma = genome.gene(genome::Gene::BrainMutationSigma)
            + s.background_mutation_variance()
            + factor.intensity_increase;
        let count = genome::poisson(rng, f64::from(lambda.max(0.0)));
        let relative_share = s.relative_mutation_share();
        let nodes_before = self.node_count();
        let synapses_before = self.synapse_count();
        self.split_disabled.clear();
        for _ in 0..count {
            mutation_counters::add_brain_event(1);
            if rng.f32() < s.synapse_mutation_chance() {
                self.mutate_synapses(rng, s, sigma, relative_share);
            } else {
                self.mutate_neurons(rng, s, sigma, relative_share);
            }
        }
        self.split_disabled.clear();
        // The repair scan only looks at hidden-node degrees (cheap), while the
        // full rebuild re-derives degrees/masks and reallocates the CSR. Most
        // mutations (weight/bias/type) leave topology untouched, so skip the
        // rebuild entirely in that case.
        let repaired = self.repair_floating_hidden_impl();
        let topology_changed = repaired
            || self.node_count() != nodes_before
            || self.synapse_count() != synapses_before;
        if topology_changed {
            self.rebuild();
        } else if herding_changed || self.plan_dirty {
            self.rebuild_csr();
            self.plan_dirty = false;
        } else if count > 0 {
            // A weight, a bias or a kind may have changed: the plan copies them, the CSR plan does not care.
            self.compile_plan();
        }
    }

    fn mutate_synapses<S>(&mut self, rng: &mut Rng, s: &S, sigma: f32, relative_share: f32)
    where
        S: BrainSettings,
    {
        let mut u = rng.f32();
        if u < s.synapse_add_chance() {
            self.add_synapse(rng);
            return;
        }
        u -= s.synapse_add_chance();
        if u < s.synapse_change_chance() {
            self.change_synapse_strength(rng, sigma, relative_share);
            return;
        }
        u -= s.synapse_change_chance();
        if u < s.synapse_flip_chance() {
            self.flip_synapse(rng);
            return;
        }
        u -= s.synapse_flip_chance();
        if u < s.synapse_toggle_chance() {
            self.toggle_synapse(rng);
            return;
        }
        u -= s.synapse_toggle_chance();
        if u < s.synapse_remove_chance() {
            self.remove_disabled_synapse(rng);
        }
    }

    fn mutate_neurons<S>(&mut self, rng: &mut Rng, s: &S, sigma: f32, relative_share: f32)
    where
        S: BrainSettings,
    {
        let mut u = rng.f32();
        if u < s.neuron_default_chance() {
            self.change_default_act(rng, sigma, relative_share);
            return;
        }
        u -= s.neuron_default_chance();
        if u < s.neuron_change_chance() {
            self.change_neuron_type(rng);
            return;
        }
        u -= s.neuron_change_chance();
        if u < s.neuron_add_chance() {
            self.add_neuron(rng);
            return;
        }
        // Decompiled quirk (`NEATBrain.cs:835-843`, spec ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â§8.6 / Q9): the removal
        // chance is subtracted before the test; the final `else` is dead code.
        u -= s.neuron_remove_chance();
        if u < s.neuron_remove_chance() {
            self.remove_neuron(rng);
        }
    }

    /// Adds a random synapse (`NEATBrain.AddSynapse`, `:680-719`).
    ///
    /// Distribution quirks preserved: input picks use `[0, N_INPUTS-1)` (input
    /// 32 is never chosen, Q10); with herding disabled the destination is
    /// re-rolled until it is not the `Herding` output.
    fn add_synapse(&mut self, rng: &mut Rng) {
        if self.synapse_count() >= MAX_SYNAPSES || self.kind.len() >= MAX_NODES {
            return;
        }
        let n_hidden = self.hidden_count();
        let roll = rng.usize_below(N_INPUTS + n_hidden);
        let node_in = if roll < N_INPUTS {
            rng.usize_below(N_INPUTS - 1) as u32
        } else {
            (FIRST_HIDDEN + rng.usize_below(n_hidden)) as u32
        };
        let mut node_out = (FIRST_OUTPUT + rng.usize_below(N_OUTPUTS + n_hidden)) as u32;
        if !self.herding_enabled {
            let mut guard = 0;
            while node_out as usize == HERDING_OUTPUT_NODE && guard < 64 {
                node_out = (FIRST_OUTPUT + rng.usize_below(N_OUTPUTS + n_hidden)) as u32;
                guard += 1;
            }
            if node_out as usize == HERDING_OUTPUT_NODE {
                return;
            }
        }
        let strength = rng.normal_with(0.0, 0.5);
        if let Some(existing) = self.find_synapse(node_in, node_out) {
            self.syn_weight[existing] += strength;
            return;
        }
        self.push_synapse(node_in, node_out, strength, true);
    }

    fn find_synapse(&self, node_in: u32, node_out: u32) -> Option<usize> {
        self.syn_in
            .iter()
            .zip(self.syn_out.iter())
            .position(|(a, b)| *a == node_in && *b == node_out)
    }

    fn push_synapse(&mut self, node_in: u32, node_out: u32, weight: f32, enabled: bool) -> usize {
        mutation_counters::add_synapse_added(1);
        let index = self.syn_in.len();
        self.syn_in.push(node_in);
        self.syn_out.push(node_out);
        self.syn_weight.push(weight);
        self.syn_enabled.push(enabled);
        let inov = self.connection_max_innovation;
        self.connection_max_innovation += 1;
        self.syn_innovation.push(inov);
        if node_in != node_out {
            self.out_degree[node_in as usize] += 1;
            self.in_degree[node_out as usize] += 1;
            if (node_in as usize) < N_INPUTS {
                self.used_input_mask |= 1_u64 << (node_in as u64);
            }
            if (FIRST_OUTPUT..FIRST_HIDDEN).contains(&(node_out as usize)) {
                self.used_output_mask |= 1_u16 << ((node_out - FIRST_OUTPUT as u32) as u16);
            }
        }
        self.plan_dirty = true;
        index
    }

    /// `NEATBrain.ChangeSynapseStrength` (`:721-732`).
    fn change_synapse_strength(&mut self, rng: &mut Rng, sigma: f32, relative_share: f32) {
        if self.synapse_count() == 0 {
            self.add_synapse(rng);
            if self.synapse_count() == 0 {
                return;
            }
        }
        let index = rng.usize_below(self.synapse_count());
        let delta = genome::next_mutation_value(
            rng,
            self.syn_weight[index],
            sigma,
            1.0,
            false,
            relative_share,
        );
        self.syn_weight[index] += delta;
    }

    /// `NEATBrain.FlipSynapse` (`:734-745`).
    fn flip_synapse(&mut self, rng: &mut Rng) {
        if self.synapse_count() == 0 {
            self.add_synapse(rng);
            if self.synapse_count() == 0 {
                return;
            }
        }
        let index = rng.usize_below(self.synapse_count());
        self.syn_weight[index] *= -1.0;
    }

    /// `NEATBrain.ToggleSynapseEnabled` (`:747-767`).
    fn toggle_synapse(&mut self, rng: &mut Rng) {
        if self.synapse_count() == 0 {
            self.add_synapse(rng);
            if self.synapse_count() == 0 {
                return;
            }
        }
        let index = rng.usize_below(self.synapse_count());
        self.syn_enabled[index] = !self.syn_enabled[index];
        // A toggled synapse follows the list's add/remove rule (cs:758-763).
        let innovation = self.syn_innovation[index];
        self.split_disabled.retain(|split| *split != innovation);
        mutation_counters::add_synapse_toggled(1);
        // Enabling state changes the CSR plan (masks count all synapses either
        // way, matching the game's Node.NIn/NOut).
        self.plan_dirty = true;
    }

    /// `NEATBrain.RemoveDisabledSynapse` (`:769-780`): remove a random disabled
    /// synapse, or toggle a random synapse when none are disabled.
    fn remove_disabled_synapse(&mut self, rng: &mut Rng) {
        let is_candidate = |brain: &Self, index: usize| {
            !brain.syn_enabled[index]
                && !brain.split_disabled.contains(&brain.syn_innovation[index])
        };
        let disabled_count = (0..self.synapse_count())
            .filter(|index| is_candidate(self, *index))
            .count();
        if disabled_count == 0 {
            self.toggle_synapse(rng);
            return;
        }
        // Same RNG draw as the game's `disabledSynapses` list pick, without
        // materialising the list.
        let pick = rng.usize_below(disabled_count);
        let victim = (0..self.synapse_count())
            .filter(|index| is_candidate(self, *index))
            .nth(pick)
            .expect("pick is below the disabled count");
        self.remove_synapse(victim);
    }

    /// `NEATBrain.ChangeDefaultAct` (`:916-935`): mutate a bias of a random
    /// non-input node (outputs included); Integrator targets are re-rolled.
    fn change_default_act(&mut self, rng: &mut Rng, sigma: f32, relative_share: f32) {
        let mut target = FIRST_OUTPUT + rng.usize_below(N_OUTPUTS + self.hidden_count());
        let mut guard = 0;
        while self.kind[target] == NodeType::Integrator && guard < 1024 {
            target = FIRST_OUTPUT + rng.usize_below(N_OUTPUTS + self.hidden_count());
            guard += 1;
        }
        if self.kind[target] == NodeType::Integrator {
            return;
        }
        let kind = self.kind[target];
        let delta = genome::next_mutation_value(
            rng,
            self.bias[target],
            sigma,
            1.0,
            kind == NodeType::Inhibitory,
            relative_share,
        );
        let mut value = self.bias[target] + delta;
        match kind {
            NodeType::Inhibitory => value = clamp(value, 0.0001, 10.0),
            NodeType::SoftLatch => value = clamp(value, 0.1, 100.0),
            _ => {}
        }
        self.bias[target] = value;
    }

    /// `NEATBrain.ChangeNeuronType` (`:937-961`): retype a random hidden node.
    fn change_neuron_type(&mut self, rng: &mut Rng) {
        if self.hidden_count() == 0 {
            self.add_neuron(rng);
            return;
        }
        let index = FIRST_HIDDEN + rng.usize_below(self.hidden_count());
        let kind = NodeType::from_u8((1 + rng.usize_below(NodeType::COUNT - 1)) as u8)
            .expect("node type in 1..=13");
        self.kind[index] = kind;
        match kind {
            NodeType::Inhibitory | NodeType::Mult => {
                self.bias[index] = clamp(self.bias[index].abs(), 0.1, 1.0);
            }
            NodeType::Integrator => self.bias[index] = 0.0,
            NodeType::SoftLatch => {
                self.bias[index] = clamp(self.bias[index], 0.1, 100.0);
            }
            _ => {}
        }
    }

    /// `NEATBrain.AddNeuron` (`:877-914`): split a random synapse, insert a new
    /// hidden node of a random type, disable the split and add two synapses.
    fn add_neuron(&mut self, rng: &mut Rng) {
        if self.synapse_count() == 0 {
            self.add_synapse(rng);
            if self.synapse_count() == 0 {
                return;
            }
        }
        if self.kind.len() >= MAX_NODES || self.synapse_count() + 2 > MAX_SYNAPSES {
            return;
        }
        let split = rng.usize_below(self.synapse_count());
        // RNG order matches the game: type roll, bias roll, split-side roll.
        let kind = NodeType::from_u8((1 + rng.usize_below(NodeType::COUNT - 1)) as u8)
            .expect("node type in 1..=13");
        let default_bias = match kind {
            NodeType::Inhibitory => 10.0_f32.powf(rng.f32_range(-2.0, 1.0)),
            NodeType::Mult => 1.0,
            NodeType::SoftLatch => 10.0_f32.powf(rng.f32_range(-1.0, 1.0)),
            _ => 0.0,
        };
        let node_index = self.kind.len();
        let inov = self.node_max_innovation;
        self.node_max_innovation += 1;
        mutation_counters::add_node_added(1);
        let desc = format!("Hidden{}", self.hidden_count());
        self.kind.push(kind);
        self.bias.push(default_bias);
        self.innovation.push(inov);
        self.custom_desc.push(Some(desc.into_boxed_str()));
        self.value.push(0.0);
        self.last_out.push(0.0);
        self.last_in.push(0.0);
        self.in_degree.push(0);
        self.out_degree.push(0);

        let split_in = self.syn_in[split];
        let split_out = self.syn_out[split];
        let split_weight = self.syn_weight[split];
        self.syn_enabled[split] = false;
        self.split_disabled.push(self.syn_innovation[split]);

        let flag = rng.f32() >= 0.5;
        let (in_weight, out_weight) = if flag {
            (split_weight, 1.0)
        } else {
            (1.0, split_weight)
        };
        self.push_synapse(split_in, node_index as u32, in_weight, true);
        self.push_synapse(node_index as u32, split_out, out_weight, true);
    }

    /// `NEATBrain.RemoveNeuron` (`:846-875`) + `FinishRemovalProcess`
    /// (`:963-1004`).
    fn remove_neuron(&mut self, rng: &mut Rng) {
        if self.hidden_count() == 0 {
            return;
        }
        let target = FIRST_HIDDEN + rng.usize_below(self.hidden_count());
        self.remove_node_compact(target);
    }

    /// `NEATBrain.RemoveSynapse` (`:782-813`) + `FinishRemovalProcess`.
    ///
    /// Reproduces the game's cascade: a hidden endpoint left without incoming or
    /// outgoing synapses is removed too (with the `> NInputs+NOutputs` strict
    /// comparison quirk that exempts node index 48).
    fn remove_synapse(&mut self, victim: usize) {
        if self.synapse_count() == 0 || victim >= self.synapse_count() {
            return;
        }
        // `FinishRemovalProcess` rebuilds `disabledSynapses` from every
        // disabled synapse, split ones included (cs:996-1003).
        self.split_disabled.clear();
        let node_in = self.syn_in[victim] as usize;
        let node_out = self.syn_out[victim] as usize;
        let self_loop = node_in == node_out;
        // Floating endpoints under the game's `> FIRST_HIDDEN` quirk, computed
        // from the maintained degree counters minus the victim's contribution
        // (the game scans the remaining connections; the result is identical).
        let mut floating = [0_usize; 2];
        let mut n_floating = 0_usize;
        for endpoint in [node_in, node_out] {
            if endpoint > FIRST_HIDDEN {
                let contributes_out = !self_loop && node_in == endpoint;
                let contributes_in = !self_loop && node_out == endpoint;
                let deg_out =
                    self.out_degree[endpoint].saturating_sub(if contributes_out { 1 } else { 0 });
                let deg_in =
                    self.in_degree[endpoint].saturating_sub(if contributes_in { 1 } else { 0 });
                if (deg_in < 1 || deg_out < 1) && !floating[..n_floating].contains(&endpoint) {
                    floating[n_floating] = endpoint;
                    n_floating += 1;
                }
            }
        }
        self.compact_synapses_where(|s, a, b| {
            if s == victim {
                return true;
            }
            let (a, b) = (a as usize, b as usize);
            floating[..n_floating].contains(&a) || floating[..n_floating].contains(&b)
        });
        self.recompute_degrees();
        self.rebuild_csr();
        self.plan_dirty = false;
        self.repair_floating_hidden_impl();
    }

    /// Removes every synapse selected by `remove(read_index, node_in, node_out)`
    /// by compacting the five synapse arrays in place (no temporary `Vec`s).
    /// Leaves degrees/CSR stale: the caller must finish with
    /// `recompute_degrees()` + `rebuild_csr()` (or `rebuild()`).
    fn compact_synapses_where(&mut self, mut remove: impl FnMut(usize, u32, u32) -> bool) {
        let len = self.syn_in.len();
        let mut write = 0_usize;
        for read in 0..len {
            let a = self.syn_in[read];
            let b = self.syn_out[read];
            if remove(read, a, b) {
                continue;
            }
            if write != read {
                self.syn_in[write] = a;
                self.syn_out[write] = b;
                self.syn_weight[write] = self.syn_weight[read];
                self.syn_enabled[write] = self.syn_enabled[read];
                self.syn_innovation[write] = self.syn_innovation[read];
            }
            write += 1;
        }
        mutation_counters::add_synapse_removed((len - write) as u64);
        self.syn_in.truncate(write);
        self.syn_out.truncate(write);
        self.syn_weight.truncate(write);
        self.syn_enabled.truncate(write);
        self.syn_innovation.truncate(write);
    }

    /// Genetic distance between two brains, as used for speciation
    /// (`docs/spec/01-brain-genetics.md` ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â§9.1).
    ///
    /// `self` plays the role of the species template and `other` the individual,
    /// so the metric is asymmetric when weights differ (the weight-distance
    /// denominator uses the template's weight). Arrays must be sorted by
    /// innovation number (the game's serialization invariant).
    pub fn distance<S>(&self, other: &Brain, _settings: &S) -> f32 {
        debug_assert!(self.syn_innovation.windows(2).all(|w| w[0] <= w[1]));
        debug_assert!(other.syn_innovation.windows(2).all(|w| w[0] <= w[1]));

        let n_max = self.synapse_count().max(other.synapse_count());
        let mut common = 0_u32;
        let mut disjoint = 0_u32;
        let mut toggled = 0_f32;
        let mut connection_distance = 0_f32;
        let (mut i, mut j) = (0_usize, 0_usize);
        while i < self.synapse_count() && j < other.synapse_count() {
            if self.syn_innovation[i] == other.syn_innovation[j] {
                common += 1;
                let delta = self.syn_weight[i] - other.syn_weight[j];
                if delta != 0.0 {
                    connection_distance += (delta / (self.syn_weight[i].abs() + 1.0)).abs();
                }
                if self.syn_enabled[i] != other.syn_enabled[j] {
                    toggled += 1.0;
                }
                i += 1;
                j += 1;
            } else if self.syn_innovation[i] > other.syn_innovation[j] {
                disjoint += 1;
                j += 1;
            } else {
                disjoint += 1;
                i += 1;
            }
        }
        disjoint += (self.synapse_count() - i) as u32 + (other.synapse_count() - j) as u32;

        let mut biased_common = 0_u32;
        let mut activation_distance = 0_f32;
        let (mut i, mut j) = (0_usize, 0_usize);
        while i < self.kind.len() && j < other.kind.len() {
            if self.innovation[i] == other.innovation[j] {
                if self.bias[i].abs() > 0.0 || other.bias[j].abs() > 0.0 {
                    biased_common += 1;
                    let delta = self.bias[i] - other.bias[j];
                    if delta != 0.0 {
                        activation_distance += delta.abs();
                    }
                }
                i += 1;
                j += 1;
            } else if self.innovation[i] > other.innovation[j] {
                j += 1;
            } else {
                i += 1;
            }
        }

        disjoint as f32 / n_max.max(1) as f32
            + toggled / common.max(1) as f32
            + connection_distance / (common.max(1) as f32).sqrt()
            + activation_distance / biased_common.max(1) as f32
    }

    /// Read-only graph difference used by brain/species comparison panels
    /// (`Species.BrainDifferencesToIndividual`). `self` is the template and
    /// `other` the individual; the returned indices refer to the template's
    /// node/synapse arrays and are safe for UI highlighting.
    pub fn differences(&self, other: &Brain) -> BrainDiff {
        let n_max = self.synapse_count().max(other.synapse_count());
        let mut common = 0_u32;
        let mut disjoint = 0_u32;
        let mut toggled_count = 0_f32;
        let mut connection_deltas = Vec::new();
        let mut disjoint_template = Vec::new();
        let mut disjoint_individual = Vec::new();
        let mut toggled = Vec::new();
        let (mut i, mut j) = (0_usize, 0_usize);
        while i < self.synapse_count() && j < other.synapse_count() {
            if self.syn_innovation[i] == other.syn_innovation[j] {
                common += 1;
                let delta = self.syn_weight[i] - other.syn_weight[j];
                let distance = (delta / (self.syn_weight[i].abs() + 1.0)).abs();
                if delta != 0.0 {
                    connection_deltas.push(BrainConnectionDelta {
                        template_index: i,
                        innovation: self.syn_innovation[i],
                        delta,
                        distance,
                    });
                }
                if self.syn_enabled[i] != other.syn_enabled[j] {
                    toggled_count += 1.0;
                    toggled.push(self.syn_innovation[i]);
                }
                i += 1;
                j += 1;
            } else if self.syn_innovation[i] > other.syn_innovation[j] {
                disjoint += 1;
                if !disjoint_individual.contains(&other.syn_innovation[j]) {
                    disjoint_individual.push(other.syn_innovation[j]);
                }
                j += 1;
            } else {
                disjoint += 1;
                if !disjoint_template.contains(&self.syn_innovation[i]) {
                    disjoint_template.push(self.syn_innovation[i]);
                }
                i += 1;
            }
        }
        for innovation in &self.syn_innovation[i..] {
            disjoint += 1;
            if !disjoint_template.contains(innovation) {
                disjoint_template.push(*innovation);
            }
        }
        for innovation in &other.syn_innovation[j..] {
            disjoint += 1;
            if !disjoint_individual.contains(innovation) {
                disjoint_individual.push(*innovation);
            }
        }

        let mut biased_common = 0_u32;
        let mut activation_deltas = Vec::new();
        let (mut i, mut j) = (0_usize, 0_usize);
        while i < self.kind.len() && j < other.kind.len() {
            if self.innovation[i] == other.innovation[j] {
                if self.bias[i].abs() > 0.0 || other.bias[j].abs() > 0.0 {
                    biased_common += 1;
                    let delta = self.bias[i] - other.bias[j];
                    if delta != 0.0 {
                        activation_deltas.push(BrainActivationDelta {
                            template_index: i,
                            innovation: self.innovation[i],
                            delta,
                            distance: 0.0,
                        });
                    }
                }
                i += 1;
                j += 1;
            } else if self.innovation[i] > other.innovation[j] {
                j += 1;
            } else {
                i += 1;
            }
        }
        for delta in &mut activation_deltas {
            delta.distance = delta.delta.abs() / biased_common.max(1) as f32;
        }
        let inv_sqrt_common = 1.0 / (common.max(1) as f32).sqrt();
        for delta in &mut connection_deltas {
            delta.distance *= inv_sqrt_common;
        }

        BrainDiff {
            common_connections: common,
            disjoint_connections: disjoint,
            toggled_connections: toggled_count,
            max_connections: n_max,
            connection_deltas,
            activation_deltas,
            disjoint_template,
            disjoint_individual,
            toggled,
        }
    }
}

/// One changed common synapse in a [`Brain::differences`] result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrainConnectionDelta {
    /// Index in the template's synapse array (UI highlight target).
    pub template_index: usize,
    /// Synapse innovation number.
    pub innovation: i64,
    /// `template - individual` weight.
    pub delta: f32,
    /// Distance contribution for the total metric.
    pub distance: f32,
}

/// One changed common node bias in a [`Brain::differences`] result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrainActivationDelta {
    /// Index in the template's node array (UI highlight target).
    pub template_index: usize,
    /// Node innovation number.
    pub innovation: i64,
    /// `template - individual` bias.
    pub delta: f32,
    /// Distance contribution for the total metric.
    pub distance: f32,
}

/// Read-only brain comparison (`Species.BrainDifferencesToIndividual`).
///
/// UI panels can highlight the listed innovations/indices without mutating
/// either brain. `total_distance()` reproduces [`Brain::distance`] semantics.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrainDiff {
    /// Synapses present in both brains.
    pub common_connections: u32,
    /// Synapses present in only one brain (including tail entries).
    pub disjoint_connections: u32,
    /// Common synapses whose enabled flag differs.
    pub toggled_connections: f32,
    /// `max(template.synapse_count, individual.synapse_count)`.
    pub max_connections: usize,
    /// Changed common synapse weights.
    pub connection_deltas: Vec<BrainConnectionDelta>,
    /// Changed common node biases.
    pub activation_deltas: Vec<BrainActivationDelta>,
    /// Template-only synapse innovations.
    pub disjoint_template: Vec<i64>,
    /// Individual-only synapse innovations.
    pub disjoint_individual: Vec<i64>,
    /// Toggled synapse innovations.
    pub toggled: Vec<i64>,
}

impl BrainDiff {
    /// Distance portion of the speciation metric for this diff.
    #[must_use]
    pub fn total_distance(&self) -> f32 {
        let connection_distance: f32 = self.connection_deltas.iter().map(|d| d.distance).sum();
        let activation_distance: f32 = self.activation_deltas.iter().map(|d| d.distance).sum();
        self.disjoint_connections as f32 / self.max_connections.max(1) as f32
            + self.toggled_connections / self.common_connections.max(1) as f32
            + connection_distance
            + activation_distance
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::{DefaultGenomeSettings, Genome, GenomeSettings, MutationFactor};

    /// Combined settings shim for tests: the eventual `SimSettings` will
    /// implement both traits; until then the two default shims are glued here.
    struct TestSettings {
        brain: DefaultBrainSettings,
        genome: DefaultGenomeSettings,
    }

    impl TestSettings {
        fn new() -> Self {
            Self {
                brain: DefaultBrainSettings::game_defaults(),
                genome: DefaultGenomeSettings::game_defaults(),
            }
        }
    }

    impl BrainSettings for TestSettings {
        fn synapse_mutation_chance(&self) -> f32 {
            self.brain.synapse_mutation_chance()
        }
        fn synapse_add_chance(&self) -> f32 {
            self.brain.synapse_add_chance()
        }
        fn synapse_change_chance(&self) -> f32 {
            self.brain.synapse_change_chance()
        }
        fn synapse_flip_chance(&self) -> f32 {
            self.brain.synapse_flip_chance()
        }
        fn synapse_toggle_chance(&self) -> f32 {
            self.brain.synapse_toggle_chance()
        }
        fn synapse_remove_chance(&self) -> f32 {
            self.brain.synapse_remove_chance()
        }
        fn neuron_default_chance(&self) -> f32 {
            self.brain.neuron_default_chance()
        }
        fn neuron_change_chance(&self) -> f32 {
            self.brain.neuron_change_chance()
        }
        fn neuron_add_chance(&self) -> f32 {
            self.brain.neuron_add_chance()
        }
        fn neuron_remove_chance(&self) -> f32 {
            self.brain.neuron_remove_chance()
        }
        fn disable_herding(&self) -> bool {
            self.brain.disable_herding()
        }
    }

    impl GenomeSettings for TestSettings {
        fn background_mutation_chance(&self) -> f32 {
            self.genome.background_mutation_chance()
        }
        fn background_mutation_variance(&self) -> f32 {
            self.genome.background_mutation_variance()
        }
        fn relative_mutation_share(&self) -> f32 {
            self.genome.relative_mutation_share()
        }
        fn prevent_mutations(&self) -> bool {
            self.genome.prevent_mutations()
        }
        fn energy_usage_efficiency(&self) -> f32 {
            self.genome.energy_usage_efficiency()
        }
        fn base_body_growth_cost(&self) -> f32 {
            self.genome.base_body_growth_cost()
        }
        fn view_angle_added_cost(&self) -> f32 {
            self.genome.view_angle_added_cost()
        }
        fn view_radius_added_cost(&self) -> f32 {
            self.genome.view_radius_added_cost()
        }
        fn armor_energy_density(&self) -> f32 {
            self.genome.armor_energy_density()
        }
        fn storable_energy_per_area(&self) -> f32 {
            self.genome.storable_energy_per_area()
        }
        fn health_energy_factor(&self) -> f32 {
            self.genome.health_energy_factor()
        }
    }

    pub(super) fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-5, "expected {b}, got {a}");
    }

    fn starter_with_hidden(
        node_type: NodeType,
        bias: f32,
        incoming: &[(usize, f32)],
        outgoing: &[(usize, f32)],
    ) -> Brain {
        let base = Brain::starter();
        let mut nodes = base.to_node_records();
        let mut synapses = base.to_synapse_records();
        let hidden = nodes.len() as u32;
        nodes.push(NodeRecord {
            node_type,
            index: hidden,
            innovation: hidden as i64 + 1,
            desc: Some("Hidden0".to_string()),
            base_activation: bias,
            value: 0.0,
            last_input: 0.0,
            last_output: 0.0,
        });
        let mut inov = 100;
        for (input, weight) in incoming {
            synapses.push(SynapseRecord {
                innovation: inov,
                node_in: *input as u32,
                node_out: hidden,
                weight: *weight,
                enabled: true,
            });
            inov += 1;
        }
        for (output, weight) in outgoing {
            synapses.push(SynapseRecord {
                innovation: inov,
                node_in: hidden,
                node_out: (FIRST_OUTPUT + output) as u32,
                weight: *weight,
                enabled: true,
            });
            inov += 1;
        }
        Brain::from_template_records(&nodes, &synapses, 0.5).expect("valid test brain")
    }

    #[test]
    fn enums_match_game_order() {
        assert_eq!(ALL_INPUTS.len(), 33);
        assert_eq!(ALL_OUTPUTS.len(), 15);
        for (i, input) in ALL_INPUTS.iter().enumerate() {
            assert_eq!(input.index(), i);
            assert_eq!(Input::from_index(i), Some(*input));
            assert!(!input.name().is_empty());
        }
        for (i, output) in ALL_OUTPUTS.iter().enumerate() {
            assert_eq!(output.index(), i);
            assert_eq!(Output::from_index(i), Some(*output));
            assert!(!output.name().is_empty());
        }
        assert_eq!(Input::RotationSpeed.index(), 5);
        assert_eq!(Output::Want2Attack.index(), 14);
        assert_eq!(Input::from_index(33), None);
        assert_eq!(Output::from_index(15), None);
        assert_eq!(NodeType::from_u8(0), Some(NodeType::Input));
        assert_eq!(NodeType::from_u8(13), Some(NodeType::SoftLatch));
        assert_eq!(NodeType::from_u8(14), None);
    }

    #[test]
    fn node_type_bias_rules() {
        assert!(!NodeType::Input.has_bias());
        assert!(!NodeType::Integrator.has_bias());
        assert!(NodeType::Sigmoid.has_bias());
        assert!(NodeType::Mult.has_bias());
        for kind in [
            NodeType::Integrator,
            NodeType::Mult,
            NodeType::Inhibitory,
            NodeType::SoftLatch,
        ] {
            assert!(kind.uses_bias_differently(), "{kind:?}");
        }
        assert!(!NodeType::TanH.uses_bias_differently());
    }

    #[test]
    fn stateless_activations() {
        let period = 0.1;
        approx(
            activation(NodeType::Linear, 0.0, 0.5, 0.0, 0.0, period).0,
            0.5,
        );
        approx(
            activation(NodeType::Linear, 0.5, 0.5, 0.0, 0.0, period).0,
            1.0,
        );
        approx(
            activation(NodeType::Sigmoid, 0.0, 0.5, 0.0, 0.0, period).0,
            sigmoid(0.5),
        );
        approx(
            activation(NodeType::TanH, 0.5, 0.0, 0.0, 0.0, period).0,
            tanh(0.5),
        );
        approx(
            activation(NodeType::Sine, 0.0, 0.5, 0.0, 0.0, period).0,
            0.5_f32.sin(),
        );
        approx(
            activation(NodeType::ReLu, 0.0, -2.0, 0.0, 0.0, period).0,
            0.0,
        );
        approx(
            activation(NodeType::Abs, 0.0, -2.0, 0.0, 0.0, period).0,
            2.0,
        );
        approx(
            activation(NodeType::Gaussian, 0.0, 2.0, 0.0, 0.0, period).0,
            1.0 / 5.0,
        );
        // `ExecuteNeuron` adds the bias for every type but the four that use it differently (`!TypeUsesBiasDifferently`, `NEATBrain.cs:614`), the Input type
        // included (an input node's bias is 0 and a brain never executes it).
        approx(
            activation(NodeType::Input, 5.0, 0.25, 0.0, 0.0, period).0,
            5.25,
        );
    }

    /// The game does the arithmetic of one C# expression in double and rounds to float only at a store, a call argument and a return (see
    /// `activation_function`): these are cases where float steps give another float (found with `brain_vs_game` against the original game, 2026-11-19).
    /// A save written while children kept their parents' running node values can hold a stale value in a node nothing feeds: healing puts it back to `f(bias)` and leaves
    /// everything else (a fed node, a value that already is `f(bias)`) alone.
    #[test]
    fn loading_heals_a_stale_value_in_an_unfed_node() {
        let base = Brain::starter();
        let nodes = base.to_node_records();
        let synapses = base.to_synapse_records();
        let unfed: Vec<usize> = (FIRST_OUTPUT..FIRST_HIDDEN).filter(|&i| !synapses.iter().any(|s| s.enabled && s.node_out as usize == i)).collect();
        assert!(unfed.len() >= 3, "the starter brain leaves outputs unfed: {unfed:?}");
        let (a, b, c) = (unfed[0], unfed[1], unfed[2]);
        let mut stale = nodes.clone();
        for index in [a, b] {
            stale[index].value = 5.0;
            stale[index].last_output = 5.0;
        }
        let mut loaded = Brain::from_records(&stale, &synapses).expect("brain");
        assert_eq!(loaded.node_value(a), Some(5.0), "from_records keeps the records' state exactly");
        loaded.heal_unfed_nodes();
        for index in [a, b] {
            assert_eq!(loaded.node_value(index).map(f32::to_bits), Some(nodes[index].value.to_bits()), "node {index} is back to f(bias)");
        }
        assert_eq!(loaded.node_value(c).map(f32::to_bits), Some(nodes[c].value.to_bits()), "a value that already is f(bias) is left alone");
        // a fed node keeps whatever it holds
        let fed = (0..nodes.len()).filter(|&i| i >= FIRST_OUTPUT).find(|&i| synapses.iter().any(|s| s.enabled && s.node_out as usize == i)).expect("a fed node");
        let mut running = nodes.clone();
        running[fed].value = 0.3;
        running[fed].last_output = 0.3;
        let mut loaded = Brain::from_records(&running, &synapses).expect("brain");
        loaded.heal_unfed_nodes();
        assert_eq!(loaded.node_value(fed), Some(0.3));
    }

    /// A child brain starts fresh (`CopyBrain` + `FinalizeEditing`): a node that nothing feeds rests at `f(bias)`, whatever its parent's node held when the copy was made.
    #[test]
    fn a_reset_brain_has_no_stale_node_values() {
        let base = Brain::starter();
        let nodes = base.to_node_records();
        let mut synapses = base.to_synapse_records();
        let phere = FIRST_OUTPUT + Output::PhereOut1.index();
        let feed = synapses.len();
        synapses.push(SynapseRecord {
            innovation: 40,
            node_in: Input::EnergyRatio.index() as u32,
            node_out: phere as u32,
            weight: 2.0,
            enabled: true,
        });
        let mut parent = Brain::from_template_records(&nodes, &synapses, 0.1).expect("brain");
        let mut inputs = BrainInputs::new();
        inputs.set(Input::EnergyRatio, 0.8);
        for _ in 0..3 {
            let _ = parent.step(&inputs, 0.1);
        }
        let running = parent.output_value(Output::PhereOut1);
        assert!(running > 1.0, "the parent emits pheromone: {running}");
        // The child is a clone whose synapse into PhereOut1 ends up disabled (a mutation, or an edit in the Bibite Editor): the clone still holds the parent's value, and nothing
        // runs the node any more.
        let mut child = parent.clone();
        assert!(child.set_synapse_enabled(feed, false));
        for _ in 0..3 {
            let _ = child.step(&inputs, 0.1);
        }
        assert_eq!(child.output_value(Output::PhereOut1), running, "the stale value is kept (what the reset is for)");
        // The reset: every node is what a freshly built brain holds.
        child.reset_runtime_state(0.1);
        let fresh = Brain::from_template_records(&nodes, &synapses, 0.1).expect("brain");
        assert_eq!(child.output_value(Output::PhereOut1), 0.0);
        for i in 0..child.node_count() {
            let (a, b) = (child.node_state(i).expect("node"), fresh.node_state(i).expect("node"));
            if i < FIRST_OUTPUT {
                assert_eq!((a.value, a.last_input, a.last_output), (0.0, 0.0, 0.0), "input node {i}");
            } else if i != phere {
                assert_eq!((a.value.to_bits(), a.last_input.to_bits(), a.last_output.to_bits()), (b.value.to_bits(), b.last_input.to_bits(), b.last_output.to_bits()), "node {i}");
            }
        }
        for _ in 0..3 {
            let _ = child.step(&inputs, 0.1);
        }
        assert_eq!(child.output_value(Output::PhereOut1), 0.0, "nothing feeds it, no bias: it stays at 0");
    }

    #[test]
    fn expressions_are_evaluated_the_games_way() {
        // Sigmoid at x = 11.674102 (bits 0x413ac91f): the game gave 0x3f7fff71, float steps 0x3f7fff72.
        assert_eq!(sigmoid(f32::from_bits(0x413a_c91f)).to_bits(), 0x3f7f_ff71);
        assert_eq!(sigmoid(f32::from_bits(0x3bc6_2ba7)).to_bits(), 0x3f00_6316);
        assert_eq!(sigmoid(f32::from_bits(0x4043_407b)).to_bits(), 0x3f74_6eec);
        // Gaussian at x = -1.1747949 / 0.44233978 / -2.8215325 / -0.06625184: the game 0x3ed71cf6 / 0x3f561b5d / 0x3de48b75 / 0x3f7ee199.
        for (x, game) in [(0xbf96_5fae_u32, 0x3ed7_1cf6_u32), (0x3ee2_7a5c, 0x3f56_1b5d), (0xc034_93fd, 0x3de4_8b75), (0xbd87_af0b, 0x3f7e_e199)] {
            assert_eq!(gaussian(f32::from_bits(x)).to_bits(), game, "gaussian({x:08x})");
        }
        // Inhibitory (bias 0.10207629, period 0.13333334, x 0.11302659, last input 0.082166806, last output -0.016380552): the game 0x3c70db0d, float steps 0x3c70db0c.
        let out = activation_function(NodeType::Inhibitory, 0.102_076_29, 0.113_026_59, -0.016_380_552, 0.082_166_806, 0.133_333_34);
        assert_eq!(out.to_bits(), 0x3c70_db0d, "inhibitory gave {out:e}");
    }

    /// With herding off the game still computes the Herding output: `ComputeProcessingStructure` skips `NodeOut != 2` (an input node), not node 35.
    #[test]
    fn herding_off_still_runs_the_herding_output() {
        let base = Brain::starter();
        let nodes = base.to_node_records();
        let mut synapses = base.to_synapse_records();
        synapses.push(SynapseRecord {
            innovation: 40,
            node_in: Input::EnergyRatio.index() as u32,
            node_out: HERDING_OUTPUT_NODE as u32,
            weight: 1.0,
            enabled: true,
        });
        let mut on = Brain::from_template_records(&nodes, &synapses, 0.1).expect("brain");
        let mut off = on.clone();
        off.set_herding_enabled(false);
        let mut inputs = BrainInputs::new();
        inputs.set(Input::EnergyRatio, 0.75);
        // (the synapses read the inputs of the tick before: two steps)
        for _ in 0..2 {
            let _ = on.step(&inputs, 0.1);
            let _ = off.step(&inputs, 0.1);
        }
        assert_eq!(
            on.output_value(Output::Herding).to_bits(),
            off.output_value(Output::Herding).to_bits(),
            "the Herding output must not depend on the herding switch"
        );
        assert_ne!(off.node_value(HERDING_OUTPUT_NODE), Some(0.0));
    }

    #[test]
    fn output_clamp_applies() {
        approx(
            activation(NodeType::Linear, 0.0, 2000.0, 0.0, 0.0, 0.1).0,
            OUTPUT_CLAMP,
        );
        approx(
            activation(NodeType::Linear, 0.0, -2000.0, 0.0, 0.0, 0.1).0,
            -OUTPUT_CLAMP,
        );
        // Integrator is unbounded except for the clamp.
        approx(
            activation(NodeType::Integrator, 0.0, 1000.0, 1000.0, 0.0, 1.0).0,
            OUTPUT_CLAMP,
        );
    }

    #[test]
    fn stateful_activations() {
        let period = 0.2;
        // Latch: thresholds inclusive, otherwise holds previous output.
        approx(
            activation(NodeType::Latch, 0.0, 0.5, 0.7, 0.0, period).0,
            0.7,
        );
        approx(
            activation(NodeType::Latch, 0.0, 1.0, 0.7, 0.0, period).0,
            1.0,
        );
        approx(
            activation(NodeType::Latch, 0.0, 0.0, 0.7, 0.0, period).0,
            0.0,
        );
        // Latch bias is added normally (not a "bias differently" type).
        approx(
            activation(NodeType::Latch, 0.6, 0.5, 0.0, 0.0, period).0,
            1.0,
        );
        // Differential: (x - lastInput) / period, last_input is x.
        let (out, last_in) = activation(NodeType::Differential, 0.0, 1.0, 0.0, 0.5, period);
        approx(out, (1.0 - 0.5) / period);
        approx(last_in, 1.0);
        // Integrator: ignores additive bias, Euler accumulation.
        approx(
            activation(NodeType::Integrator, 5.0, 2.0, 1.0, 0.0, period).0,
            1.0 + 2.0 * period,
        );
        // Inhibitory: x - lastInput + lastOutput * exp(-bias * period).
        approx(
            activation(NodeType::Inhibitory, 0.5, 1.0, 2.0, 0.5, period).0,
            0.5 + 2.0 * (-0.5 * period).exp(),
        );
        // SoftLatch saturation and hysteresis branch.
        approx(
            activation(NodeType::SoftLatch, 5.0, 0.999, 0.0, 0.0, period).0,
            1.0,
        );
        approx(
            activation(NodeType::SoftLatch, 5.0, 0.001, 1.0, 0.0, period).0,
            0.0,
        );
        let expected = 0.8 / (1.0 - (-2.0_f32 * 0.5).exp()) * (1.0 - (-2.0_f32 * 0.2).exp());
        approx(
            activation(NodeType::SoftLatch, 2.0, 0.2, 0.8, 0.5, period).0,
            expected,
        );
    }

    #[test]
    fn starter_structure_and_upkeep() {
        let brain = Brain::starter();
        assert_eq!(brain.node_count(), 48);
        assert_eq!(brain.synapse_count(), 3);
        assert_eq!(brain.hidden_count(), 0);
        assert_eq!(
            brain.upkeep(),
            BrainUpkeep {
                synapses: 3,
                hidden: 0
            }
        );
        approx(
            brain.upkeep().energy_per_second(0.00025, 0.001),
            3.0 * 0.00025,
        );
        assert_eq!(brain.node_max_innovation(), 49);
        assert_eq!(brain.connection_max_innovation(), 4);
        brain.validate().expect("starter is valid");
    }

    #[test]
    fn starter_bias_only_outputs() {
        let mut brain = Brain::starter();
        let outs = brain.step(&BrainInputs::default(), 0.5);
        approx(outs.get(Output::Accelerate), tanh(0.453_592_9));
        approx(outs.get(Output::Rotate), 0.0);
        approx(outs.get(Output::Herding), 0.0);
        approx(outs.get(Output::EggProduction), tanh(0.2));
        approx(outs.get(Output::Want2Lay), 0.5);
        approx(outs.get(Output::Want2Eat), tanh(1.228));
        approx(outs.get(Output::Digestion), sigmoid(-2.066_895_5));
        approx(outs.get(Output::Grab), 0.0);
        approx(outs.get(Output::ClkReset), 0.5);
        approx(outs.get(Output::PhereOut1), 0.0);
        approx(outs.get(Output::PhereOut2), 0.0);
        approx(outs.get(Output::PhereOut3), 0.0);
        approx(outs.get(Output::Want2Grow), 0.5);
        approx(outs.get(Output::Want2Heal), 0.5);
        approx(outs.get(Output::Want2Attack), 0.5);
    }

    #[test]
    fn inputs_are_one_brain_tick_stale() {
        let mut brain = Brain::starter();
        let mut inputs = BrainInputs::new();
        inputs.set(Input::PlantAngle, 1.0);
        inputs.set(Input::PlantCloseness, 1.0);
        inputs.set(Input::Fullness, 1.0);
        // Tick 1 sees the initial (zero) input history.
        let outs1 = brain.step(&inputs, 0.5);
        approx(outs1.get(Output::Rotate), 0.0);
        approx(outs1.get(Output::Accelerate), tanh(0.453_592_9));
        approx(outs1.get(Output::Digestion), sigmoid(-2.066_895_5));
        // Tick 2 sees tick 1's inputs.
        let outs2 = brain.step(&inputs, 0.5);
        approx(outs2.get(Output::Rotate), tanh(1.159_251_3));
        approx(outs2.get(Output::Accelerate), tanh(0.453_592_9 - 0.401_356));
        approx(
            outs2.get(Output::Digestion),
            sigmoid(-2.066_895_5 + 4.072_957_5),
        );
    }

    #[test]
    fn integrator_hidden_node_accumulates() {
        let mut brain = starter_with_hidden(
            NodeType::Integrator,
            0.0,
            &[(Input::Fullness.index(), 1.0)],
            &[(Output::Accelerate.index(), 1.0)],
        );
        let mut inputs = BrainInputs::new();
        inputs.set(Input::Fullness, 1.0);
        let _ = brain.step(&inputs, 0.5);
        let hidden_after_1 = brain.to_node_records()[48].last_output;
        approx(hidden_after_1, 0.0);
        let _ = brain.step(&inputs, 0.5);
        let hidden_after_2 = brain.to_node_records()[48].last_output;
        approx(hidden_after_2, 0.5);
        let _ = brain.step(&inputs, 0.5);
        let hidden_after_3 = brain.to_node_records()[48].last_output;
        approx(hidden_after_3, 1.0);
    }

    #[test]
    fn mult_hidden_node_multiplies_inputs_and_bias() {
        let mut brain = starter_with_hidden(
            NodeType::Mult,
            2.0,
            &[
                (Input::Fullness.index(), 1.0),
                (Input::PlantCloseness.index(), 1.0),
            ],
            &[(Output::Accelerate.index(), 1.0)],
        );
        let mut inputs = BrainInputs::new();
        inputs.set(Input::Fullness, 3.0);
        inputs.set(Input::PlantCloseness, 4.0);
        let _ = brain.step(&inputs, 0.5);
        let _ = brain.step(&inputs, 0.5);
        let records = brain.to_node_records();
        approx(records[48].value, 2.0 * 3.0 * 4.0);
    }

    #[test]
    fn records_roundtrip_preserves_state() {
        let mut a = Brain::starter();
        let inputs = BrainInputs::new();
        let _ = a.step(&inputs, 0.5);
        let b =
            Brain::from_records(&a.to_node_records(), &a.to_synapse_records()).expect("round-trip");
        b.validate().expect("valid");
        assert_eq!(a.to_node_records(), b.to_node_records());
        assert_eq!(a.to_synapse_records(), b.to_synapse_records());
        let mut a2 = a.clone();
        let mut b2 = b;
        let mut sense = BrainInputs::new();
        sense.set(Input::PlantAngle, 0.3);
        let out_a = a2.step(&sense, 0.5);
        let out_b = b2.step(&sense, 0.5);
        for output in ALL_OUTPUTS {
            assert_eq!(out_a.get(output), out_b.get(output), "{output:?}");
        }
    }

    #[test]
    fn template_construction_repairs_floating_hidden() {
        let base = Brain::starter();
        let mut nodes = base.to_node_records();
        nodes.push(NodeRecord::template(NodeType::TanH, 48, 49, 0.0));
        let mut synapses = base.to_synapse_records();
        synapses.push(SynapseRecord {
            innovation: 10,
            node_in: 48,
            node_out: FIRST_OUTPUT as u32,
            weight: 1.0,
            enabled: true,
        });
        let brain =
            Brain::from_template_records(&nodes, &synapses, 0.5).expect("valid after repair");
        assert_eq!(brain.node_count(), 48);
        assert_eq!(brain.hidden_count(), 0);
    }

    #[test]
    fn constructors_reject_malformed_brains() {
        assert!(Brain::from_template_records(&[], &[], 0.5).is_err());
        let in_ = vec![NodeRecord::template(NodeType::Input, 0, 1, 0.0); 32];
        assert!(Brain::from_template_records(&in_, &[], 0.5).is_err());
        let mut nodes = Brain::starter().to_node_records();
        nodes[0] = NodeRecord::template(NodeType::TanH, 0, 1, 0.0);
        assert!(Brain::from_template_records(&nodes, &[], 0.5).is_err());
        let nodes = Brain::starter().to_node_records();
        let bad = [SynapseRecord {
            innovation: 1,
            node_in: 0,
            node_out: 999,
            weight: 1.0,
            enabled: true,
        }];
        assert!(Brain::from_template_records(&nodes, &bad, 0.5).is_err());
    }

    #[test]
    fn mutate_is_deterministic() {
        let settings = TestSettings::new();
        let genome = Genome::starter();
        let factor = MutationFactor {
            probability_increase: 4.0,
            intensity_increase: 0.2,
        };
        let mut a = Brain::starter();
        let mut b = Brain::starter();
        a.mutate(&mut Rng::from_seed(42), &settings, &genome, &factor);
        b.mutate(&mut Rng::from_seed(42), &settings, &genome, &factor);
        assert_eq!(a.to_node_records(), b.to_node_records());
        assert_eq!(a.to_synapse_records(), b.to_synapse_records());
    }

    /// A brain steps as a brain rebuilt from its own records does: three steps on different inputs give the same bits.
    fn steps_like_a_rebuilt_brain(brain: &mut Brain, what: &str) {
        let mut fresh = Brain::from_records(&brain.to_node_records(), &brain.to_synapse_records())
            .expect("a brain's own records rebuild it");
        fresh.set_herding_enabled(brain.herding_enabled());
        for round in 0..3 {
            let mut inputs = BrainInputs::new();
            inputs.set(Input::Fullness, 0.3 + 0.2 * round as f32);
            inputs.set(Input::PlantCloseness, 0.9 - 0.2 * round as f32);
            inputs.set(Input::PlantAngle, -0.5 + 0.4 * round as f32);
            let ours = brain.step(&inputs, DEFAULT_BRAIN_PERIOD);
            let theirs = fresh.step(&inputs, DEFAULT_BRAIN_PERIOD);
            assert!(
                ours.values()
                    .iter()
                    .zip(theirs.values())
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "{what}: round {round}: {:?} against {:?}",
                ours.values(),
                theirs.values()
            );
        }
    }

    /// The compiled plan copies weights, biases and kinds: every edit of one of them, in the editor or by mutation, reaches the
    /// next step.
    #[test]
    fn edits_and_mutations_reach_the_next_step() {
        let mut brain = Brain::starter();
        steps_like_a_rebuilt_brain(&mut brain, "starter");
        assert!(brain.set_synapse_weight(0, 3.25));
        steps_like_a_rebuilt_brain(&mut brain, "a weight");
        assert!(brain.set_node_bias_at(FIRST_OUTPUT + Output::Accelerate.index(), -0.75));
        steps_like_a_rebuilt_brain(&mut brain, "a bias");
        brain.set_output_bias(Output::Digestion, 1.5);
        steps_like_a_rebuilt_brain(&mut brain, "an output bias");
        let hidden = brain
            .add_hidden_node(NodeType::TanH, Some("h"))
            .expect("a hidden node fits");
        assert!(
            brain
                .add_synapse_connection(Input::PlantCloseness.index(), hidden, 0.8, true)
                .is_some()
        );
        assert!(
            brain
                .add_synapse_connection(hidden, FIRST_OUTPUT + Output::Rotate.index(), -1.1, true)
                .is_some()
        );
        steps_like_a_rebuilt_brain(&mut brain, "a hidden node");
        assert!(brain.set_hidden_node_type(hidden, NodeType::Sigmoid));
        steps_like_a_rebuilt_brain(&mut brain, "a node type");
        assert!(brain.set_hidden_node_type(hidden, NodeType::Mult));
        steps_like_a_rebuilt_brain(&mut brain, "a multiplier");
        assert!(brain.set_synapse_enabled(1, false));
        steps_like_a_rebuilt_brain(&mut brain, "a disabled synapse");
        assert!(brain.set_synapse_enabled(1, true));
        steps_like_a_rebuilt_brain(&mut brain, "an enabled synapse");
        brain.set_herding_enabled(false);
        steps_like_a_rebuilt_brain(&mut brain, "herding off");
        // A loaded brain whose soft latch has a bias outside the range the game clamps it to.
        let mut nodes = Brain::starter().to_node_records();
        let mut synapses = Brain::starter().to_synapse_records();
        nodes.push(NodeRecord::template(
            NodeType::SoftLatch,
            FIRST_HIDDEN as u32,
            50,
            0.0,
        ));
        synapses.push(SynapseRecord {
            innovation: 4,
            node_in: Input::PlantCloseness.index() as u32,
            node_out: FIRST_HIDDEN as u32,
            weight: 1.3,
            enabled: true,
        });
        synapses.push(SynapseRecord {
            innovation: 5,
            node_in: FIRST_HIDDEN as u32,
            node_out: (FIRST_OUTPUT + Output::Rotate.index()) as u32,
            weight: 0.9,
            enabled: true,
        });
        let mut latched =
            Brain::from_records(&nodes, &synapses).expect("a brain with a soft latch");
        assert!(latched.to_node_records()[FIRST_HIDDEN].base_activation >= 0.1);
        steps_like_a_rebuilt_brain(&mut latched, "a soft latch with a bias out of range");
        let settings = TestSettings::new();
        let genome = Genome::starter();
        let factor = MutationFactor {
            probability_increase: 6.0,
            intensity_increase: 0.3,
        };
        let mut rng = Rng::from_seed(2024);
        for round in 0..120 {
            brain.mutate(&mut rng, &settings, &genome, &factor);
            steps_like_a_rebuilt_brain(&mut brain, &format!("mutation {round}"));
        }
    }

    #[test]
    fn mutate_keeps_brain_valid() {
        let settings = TestSettings::new();
        let genome = Genome::starter();
        let factor = MutationFactor {
            probability_increase: 50.0,
            intensity_increase: 0.2,
        };
        let mut brain = Brain::starter();
        let mut rng = Rng::from_seed(7);
        for _ in 0..200 {
            brain.mutate(&mut rng, &settings, &genome, &factor);
            brain.validate().expect("brain valid after mutation");
            assert!(brain.node_count() <= MAX_NODES);
            assert!(brain.synapse_count() <= MAX_SYNAPSES);
            let mut inputs = BrainInputs::new();
            inputs.set(Input::Fullness, 0.5);
            let outs = brain.step(&inputs, DEFAULT_BRAIN_PERIOD);
            assert!(outs.values().iter().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn disable_herding_blocks_herding_synapses() {
        let mut settings = TestSettings::new();
        settings.brain.disable_herding = true;
        let genome = Genome::starter();
        let factor = MutationFactor {
            probability_increase: 100.0,
            intensity_increase: 0.2,
        };
        let mut brain = Brain::starter();
        let mut rng = Rng::from_seed(99);
        for _ in 0..50 {
            brain.mutate(&mut rng, &settings, &genome, &factor);
        }
        assert!(!brain.herding_enabled());
        for syn in brain.to_synapse_records() {
            assert_ne!(
                syn.node_out as usize, HERDING_OUTPUT_NODE,
                "synapse targeting Herding with herding disabled"
            );
        }
    }

    #[test]
    fn distance_zer_for_identical_brains() {
        let brain = Brain::starter();
        approx(brain.distance(&brain.clone(), &()), 0.0);
    }

    #[test]
    fn distance_weight_change_uses_template_weight_denominator() {
        let brain = Brain::starter();
        let mut nodes = brain.to_node_records();
        let mut synapses = brain.to_synapse_records();
        // Change the first synapse weight by +1.
        let template_weight = synapses[0].weight;
        synapses[0].weight += 1.0;
        nodes.truncate(FIRST_HIDDEN);
        let other = Brain::from_template_records(&nodes, &synapses, 0.5).expect("valid");
        let expected = (1.0 / (template_weight.abs() + 1.0)).abs() / (3.0_f32).sqrt();
        approx(brain.distance(&other, &()), expected);
    }

    #[test]
    fn distance_counts_toggled_synapses() {
        let brain = Brain::starter();
        let nodes = brain.to_node_records();
        let mut synapses = brain.to_synapse_records();
        synapses[0].enabled = false;
        let other = Brain::from_template_records(&nodes, &synapses, 0.5).expect("valid");
        approx(brain.distance(&other, &()), 1.0 / 3.0);
    }

    #[test]
    fn distance_includes_disjoint_and_bias_terms() {
        let a = Brain::starter();
        let mut nodes = a.to_node_records();
        // Add a hidden node with a bias and two synapses (present only in `b`).
        nodes.push(NodeRecord {
            node_type: NodeType::TanH,
            index: 48,
            innovation: 100,
            desc: Some("Hidden0".to_string()),
            base_activation: 0.5,
            value: 0.0,
            last_input: 0.0,
            last_output: 0.0,
        });
        let mut synapses = a.to_synapse_records();
        synapses.push(SynapseRecord {
            innovation: 200,
            node_in: 0,
            node_out: 48,
            weight: 1.0,
            enabled: true,
        });
        synapses.push(SynapseRecord {
            innovation: 201,
            node_in: 48,
            node_out: FIRST_OUTPUT as u32,
            weight: 1.0,
            enabled: true,
        });
        let b = Brain::from_template_records(&nodes, &synapses, 0.5).expect("valid");
        let distance = b.distance(&a, &());
        // b vs a: two extra (disjoint) synapses over max(5,3) connections.
        // Node bias deltas only count nodes matched by innovation, so the new
        // hidden node's bias does not contribute (it has no counterpart).
        approx(distance, 2.0 / 5.0);
        approx(a.distance(&b, &()), 2.0 / 5.0);
    }

    // -----------------------------------------------------------------------
    // task-17: usage masks, compiled-plan reference check, parity accessors
    // -----------------------------------------------------------------------

    fn brute_force_masks(brain: &Brain) -> (u64, u16) {
        let mut inputs = 0_u64;
        let mut outputs = 0_u16;
        for syn in brain.to_synapse_records() {
            let a = syn.node_in as usize;
            let b = syn.node_out as usize;
            if a != b {
                if a < N_INPUTS {
                    inputs |= 1_u64 << (a as u64);
                }
                if (FIRST_OUTPUT..FIRST_HIDDEN).contains(&b) {
                    outputs |= 1_u16 << ((b - FIRST_OUTPUT) as u16);
                }
            }
        }
        (inputs, outputs)
    }

    #[test]
    fn starter_usage_masks_match_game_laziness() {
        let brain = Brain::starter();
        let expected_inputs = input_bit(Input::Fullness)
            | input_bit(Input::PlantCloseness)
            | input_bit(Input::PlantAngle);
        assert_eq!(brain.used_input_mask(), expected_inputs);
        let expected_outputs = output_bit(Output::Accelerate)
            | output_bit(Output::Rotate)
            | output_bit(Output::Digestion);
        assert_eq!(brain.used_output_mask(), expected_outputs);
        assert!(brain.uses_input(Input::PlantAngle));
        assert!(!brain.uses_input(Input::EnergyRatio));
        assert!(brain.uses_output(Output::Digestion));
        assert!(!brain.uses_output(Output::Want2Attack));
        // Game StartUsedSystems: bibites/meat/phero off, plants on.
        assert!(!brain.needs_bibite_vision());
        assert!(brain.needs_plant_vision());
        assert!(!brain.needs_meat_vision());
        assert!(!brain.needs_pheromone_sensing());
    }

    #[test]
    fn usage_masks_follow_topology_changes() {
        // Add a pheromone input synapse and wire the Herding output.
        let base = Brain::starter();
        let nodes = base.to_node_records();
        let mut synapses = base.to_synapse_records();
        synapses.push(SynapseRecord {
            innovation: 10,
            node_in: Input::PheroSense1.index() as u32,
            node_out: (FIRST_OUTPUT + Output::Accelerate.index()) as u32,
            weight: 1.0,
            enabled: false, // disabled synapses still count in the game's masks
        });
        synapses.push(SynapseRecord {
            innovation: 11,
            node_in: Input::PlantAngle.index() as u32,
            node_out: HERDING_OUTPUT_NODE as u32,
            weight: 1.0,
            enabled: true,
        });
        let brain =
            Brain::from_template_records(&nodes, &synapses, 0.5).expect("valid template brain");
        assert!(brain.uses_input(Input::PheroSense1));
        assert!(brain.needs_pheromone_sensing());
        assert!(brain.uses_output(Output::Herding));
        assert!(brain.needs_bibite_vision());
        // Clearing herding removes the bibite-vision dependency again.
        let mut brain = brain;
        brain.set_herding_enabled(false);
        assert!(!brain.needs_bibite_vision());
        // Rebuild from records without the extra synapses.
        let mut nodes = base.to_node_records();
        nodes.truncate(FIRST_HIDDEN);
        let synapses = base.to_synapse_records();
        let brain = Brain::from_template_records(&nodes, &synapses, 0.5).expect("valid");
        assert!(!brain.needs_bibite_vision());
        assert!(!brain.needs_pheromone_sensing());
    }

    #[test]
    fn usage_masks_stay_valid_after_mutations() {
        let settings = TestSettings::new();
        let genome = Genome::starter();
        let factor = MutationFactor {
            probability_increase: 4.0,
            intensity_increase: 0.1,
        };
        for seed in 0..100_u64 {
            let mut brain = Brain::starter();
            let mut rng = Rng::from_seed(seed);
            for _ in 0..10 {
                brain.mutate(&mut rng, &settings, &genome, &factor);
                let (mask_in, mask_out) = brute_force_masks(&brain);
                assert_eq!(brain.used_input_mask(), mask_in, "seed {seed}");
                assert_eq!(brain.used_output_mask(), mask_out, "seed {seed}");
                assert_eq!(
                    brain.needs_pheromone_sensing(),
                    mask_in & PHEROMONE_INPUT_MASK != 0
                );
                assert_eq!(
                    brain.needs_plant_vision(),
                    mask_in & VISION_PLANT_INPUT_MASK != 0
                );
                assert_eq!(
                    brain.needs_meat_vision(),
                    mask_in & VISION_MEAT_INPUT_MASK != 0
                );
            }
        }
    }

    #[test]
    fn parity_accessors_read_write_state() {
        let mut brain = Brain::starter();
        assert_eq!(brain.input_count(), N_INPUTS);
        assert_eq!(brain.output_count(), N_OUTPUTS);
        assert_eq!(brain.node_count(), 48);
        assert_eq!(brain.synapse_count(), 3);
        assert_eq!(brain.hidden_count(), 0);
        let (nodes, synapses, enabled, hidden) = brain.digest_counts();
        assert_eq!((nodes, synapses, enabled, hidden), (48, 3, 3, 0));
        assert_eq!(brain.node_type(0), Some(NodeType::Input));
        assert_eq!(brain.node_type(FIRST_OUTPUT), Some(NodeType::TanH));
        assert_eq!(brain.node_innovation(0), Some(1));
        assert_eq!(brain.node_type(9999), None);
        approx(brain.input_value(Input::PlantAngle), 0.0);
        approx(brain.output_value(Output::Digestion), sigmoid(-2.066_895_5));
        approx(brain.output_bias(Output::Accelerate), 0.453_592_9);
        brain.set_output_bias(Output::Rotate, 0.25);
        approx(brain.output_bias(Output::Rotate), 0.25);
        let state = NodeRuntimeState {
            value: 0.5,
            last_input: 0.25,
            last_output: 0.75,
        };
        assert!(brain.set_node_state(FIRST_OUTPUT + 3, state));
        assert_eq!(brain.node_state(FIRST_OUTPUT + 3), Some(state));
        assert!(!brain.set_node_state(9999, state));
        assert_eq!(brain.node_state(9999), None);
        brain.set_output_value(Output::Rotate, 0.125);
        approx(brain.output_value(Output::Rotate), 0.125);
    }

    /// Independent reference interpretation of one brain tick: builds the
    /// destination groups by scanning the synapse array and sums in synapse order
    /// (no precompiled CSR), then returns the outputs and full node state.
    /// Mirrors `NEATBrain.Thinker`.
    fn reference_step(
        brain: &Brain,
        inputs: &BrainInputs,
        period: f32,
    ) -> (BrainOutputs, Vec<NodeRuntimeState>) {
        let mut value = brain.value.clone();
        let mut last_out = brain.last_out.clone();
        let mut last_in = brain.last_in.clone();
        last_out[..N_INPUTS].copy_from_slice(&value[..N_INPUTS]);
        value[..N_INPUTS].copy_from_slice(&inputs.values()[..]);
        let mut targets: Vec<usize> = Vec::new();
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for s in 0..brain.synapse_count() {
            if !brain.syn_enabled[s] {
                continue;
            }
            let out = brain.syn_out[s] as usize;
            if !brain.herding_enabled && out == HERDING_SKIPPED_NODE {
                continue;
            }
            match targets.iter().position(|target| *target == out) {
                Some(group) => groups[group].push(s),
                None => {
                    targets.push(out);
                    groups.push(vec![s]);
                }
            }
        }
        for (group, target) in targets.iter().enumerate() {
            let target = *target;
            let kind = brain.kind[target];
            let mut acc = if kind == NodeType::Mult { 1.0_f32 } else { 0.0 };
            for s in &groups[group] {
                let term = brain.syn_weight[*s] * last_out[brain.syn_in[*s] as usize];
                if kind == NodeType::Mult {
                    acc *= term;
                } else {
                    acc += term;
                }
            }
            let (out, last_input) = activation(
                kind,
                brain.bias[target],
                acc,
                last_out[target],
                last_in[target],
                period,
            );
            value[target] = out;
            last_out[target] = out;
            last_in[target] = last_input;
        }
        let mut values = [0.0_f32; N_OUTPUTS];
        values.copy_from_slice(&value[FIRST_OUTPUT..FIRST_HIDDEN]);
        let states = (0..brain.node_count())
            .map(|i| NodeRuntimeState {
                value: value[i],
                last_input: last_in[i],
                last_output: last_out[i],
            })
            .collect();
        (BrainOutputs { values }, states)
    }

    fn random_inputs(seed: u64) -> BrainInputs {
        let mut rng = Rng::from_seed(seed);
        let mut inputs = BrainInputs::new();
        for input in ALL_INPUTS {
            inputs.set(input, rng.f32_range(-2.0, 2.0));
        }
        inputs
    }

    #[test]
    fn compiled_step_matches_reference_interpreter() {
        // 1000 randomized brains ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬ÃƒÂ¢Ã¢â‚¬Å¾Ã‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â 5 ticks, comparing every output and every
        // node's Value/LastInput/LastOutput bitwise against the reference.
        let settings = TestSettings::new();
        let genome = Genome::starter();
        let factor = MutationFactor {
            probability_increase: 2.0,
            intensity_increase: 0.1,
        };
        let period = 2.0 / 30.0;
        for seed in 0..1_000_u64 {
            let mut brain = Brain::starter();
            brain.mutate(
                &mut Rng::from_seed(seed.wrapping_mul(3).wrapping_add(1)),
                &settings,
                &genome,
                &factor,
            );
            for tick in 0..5_u64 {
                let inputs = random_inputs(seed.wrapping_mul(17).wrapping_add(tick));
                let mut engine = brain.clone();
                let (expected_outputs, expected_states) = reference_step(&engine, &inputs, period);
                let actual_outputs = engine.step(&inputs, period);
                for output in ALL_OUTPUTS {
                    assert_eq!(
                        actual_outputs.get(output).to_bits(),
                        expected_outputs.get(output).to_bits(),
                        "seed {seed} tick {tick} output {output:?}"
                    );
                }
                for (i, expected_state) in expected_states.iter().enumerate() {
                    let actual_state = engine.node_state(i).expect("node in range");
                    assert_eq!(
                        actual_state.value.to_bits(),
                        expected_state.value.to_bits(),
                        "seed {seed} tick {tick} node {i} value"
                    );
                    assert_eq!(
                        actual_state.last_input.to_bits(),
                        expected_state.last_input.to_bits(),
                        "seed {seed} tick {tick} node {i} last_input"
                    );
                    assert_eq!(
                        actual_state.last_output.to_bits(),
                        expected_state.last_output.to_bits(),
                        "seed {seed} tick {tick} node {i} last_output"
                    );
                }
                brain = engine;
            }
        }
    }

    #[test]
    fn step_is_deterministic_over_many_ticks() {
        let mut a = Brain::starter();
        let mut b = Brain::starter();
        let mut rng = Rng::from_seed(1234);
        for _ in 0..200 {
            let mut inputs = BrainInputs::new();
            for input in ALL_INPUTS {
                inputs.set(input, rng.f32_range(-1.0, 1.0));
            }
            let out_a = a.step(&inputs, 2.0 / 30.0);
            let out_b = b.step(&inputs, 2.0 / 30.0);
            for output in ALL_OUTPUTS {
                assert_eq!(
                    out_a.get(output).to_bits(),
                    out_b.get(output).to_bits(),
                    "{output:?}"
                );
            }
        }
    }

    #[test]
    fn extreme_inputs_stay_finite_and_clamped() {
        let mut brain = starter_with_hidden(
            NodeType::Integrator,
            0.0,
            &[(Input::Fullness.index(), 1.0)],
            &[(Output::Accelerate.index(), 1.0)],
        );
        for tick in 0..2_000 {
            let mut inputs = BrainInputs::new();
            let magnitude = 1.0e6;
            for input in ALL_INPUTS {
                inputs.set(input, magnitude);
            }
            let outputs = brain.step(&inputs, 2.0 / 30.0);
            for output in ALL_OUTPUTS {
                let value = outputs.get(output);
                assert!(value.is_finite(), "tick {tick} {output:?} {value}");
                assert!(value.abs() <= OUTPUT_CLAMP);
            }
        }
    }

    #[test]
    fn step_plan_rebuilds_after_topology_change() {
        let mut brain = Brain::starter();
        assert!(!brain.plan_dirty());
        brain.plan_dirty = true; // simulate an un-rebuilt topology edit
        assert!(brain.plan_dirty());
        let outputs = brain.step(&BrainInputs::new(), 2.0 / 30.0);
        assert!(!brain.plan_dirty(), "step must rebuild a dirty plan");
        assert!(outputs.values().iter().all(|value| value.is_finite()));
        let before = outputs.get(Output::Digestion);
        let outputs = brain.step(&BrainInputs::new(), 2.0 / 30.0);
        approx(outputs.get(Output::Digestion), before);
    }

    #[test]
    fn timing_report_for_starter_and_grown_brain() {
        use std::time::Instant;
        let period = 2.0f32 / 30.0;
        let inputs = BrainInputs::new();
        let mut starter = Brain::starter();
        let iterations = 200_000_u32;
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = starter.step(&inputs, period);
        }
        let elapsed = start.elapsed();
        let ns_per_step = elapsed.as_nanos() as f64 / f64::from(iterations);
        println!(
            "brain step timing: starter (48 nodes/3 synapses) = {ns_per_step:.1} ns/step \
             ({:.2} M steps/s)",
            1_000.0 / ns_per_step
        );

        // Grow a ~50-node brain via mutation, then time it.
        let settings = TestSettings::new();
        let genome = Genome::starter();
        let factor = MutationFactor {
            probability_increase: 200.0,
            intensity_increase: 0.2,
        };
        let mut grown = Brain::starter();
        let mut rng = Rng::from_seed(7);
        while grown.node_count() < 64 {
            grown.mutate(&mut rng, &settings, &genome, &factor);
        }
        let iterations = 20_000_u32;
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = grown.step(&inputs, period);
        }
        let elapsed = start.elapsed();
        let ns_per_step = elapsed.as_nanos() as f64 / f64::from(iterations);
        println!(
            "brain step timing: grown ({} nodes/{} synapses, {} hidden) = {ns_per_step:.1} ns/step \
             ({:.2} M steps/s)",
            grown.node_count(),
            grown.synapse_count(),
            grown.hidden_count(),
            1_000.0 / ns_per_step
        );
    }

    #[test]
    fn step_batch_is_bit_identical_to_serial() {
        let settings = TestSettings::new();
        let mut rng = Rng::from_seed(0x5EED_0042);
        let mut brains: Vec<Brain> = Vec::new();
        for index in 0..32_usize {
            let mut brain = Brain::starter();
            for _ in 0..(index % 7) {
                brain.mutate(
                    &mut rng,
                    &settings,
                    &Genome::starter(),
                    &MutationFactor::default(),
                );
            }
            brains.push(brain);
        }
        let mut inputs: Vec<BrainInputs> = Vec::new();
        for _ in 0..brains.len() {
            let mut brain_inputs = BrainInputs::new();
            brain_inputs.set(Input::EnergyRatio, rng.f32_range(0.0, 1.0));
            brain_inputs.set(Input::Fullness, rng.f32_range(0.0, 1.0));
            brain_inputs.set(Input::PlantAngle, rng.f32_range(-1.0, 1.0));
            brain_inputs.set(Input::PlantCloseness, rng.f32_range(0.0, 1.0));
            brain_inputs.set(Input::TimeAlive, rng.f32_range(0.0, 500.0));
            inputs.push(brain_inputs);
        }

        let mut serial = brains.clone();
        let serial_outputs: Vec<BrainOutputs> = serial
            .iter_mut()
            .zip(&inputs)
            .map(|(brain, brain_inputs)| brain.step(brain_inputs, 1.0 / 15.0))
            .collect();

        let count = brains.len();
        let mut brain_refs: Vec<&mut Brain> = brains.iter_mut().collect();
        let mut batch_outputs = vec![BrainOutputs::default(); count];
        Brain::step_batch(&mut brain_refs, &inputs, 1.0 / 15.0, &mut batch_outputs);

        for (serial_out, batch_out) in serial_outputs.iter().zip(&batch_outputs) {
            for output in ALL_OUTPUTS {
                assert_eq!(
                    serial_out.get(output).to_bits(),
                    batch_out.get(output).to_bits(),
                    "output {output:?} differs between serial and batch step"
                );
            }
        }
        assert_eq!(
            serial, brains,
            "brain runtime state differs after batch step"
        );
    }
}

/// Brain-mutation instrumentation for the evolution benchmark (task-48).
///
/// Always compiled (a few relaxed atomics on the birth path).
pub mod mutation_counters {
    use std::sync::atomic::{AtomicU64, Ordering};

    static BRAIN_EVENTS: AtomicU64 = AtomicU64::new(0);
    static SYNAPSES_ADDED: AtomicU64 = AtomicU64::new(0);
    static SYNAPSES_REMOVED: AtomicU64 = AtomicU64::new(0);
    static SYNAPSES_TOGGLED: AtomicU64 = AtomicU64::new(0);
    static NODES_ADDED: AtomicU64 = AtomicU64::new(0);
    static NODES_REMOVED: AtomicU64 = AtomicU64::new(0);

    /// Adds `count` brain mutation events (one Poisson event each).
    pub fn add_brain_event(count: u64) {
        BRAIN_EVENTS.fetch_add(count, Ordering::Relaxed);
    }
    /// Adds `count` structurally added synapses.
    pub fn add_synapse_added(count: u64) {
        SYNAPSES_ADDED.fetch_add(count, Ordering::Relaxed);
    }
    /// Adds `count` structurally removed synapses.
    pub fn add_synapse_removed(count: u64) {
        SYNAPSES_REMOVED.fetch_add(count, Ordering::Relaxed);
    }
    /// Adds `count` synapse enable/disable toggles.
    pub fn add_synapse_toggled(count: u64) {
        SYNAPSES_TOGGLED.fetch_add(count, Ordering::Relaxed);
    }
    /// Adds `count` structurally added hidden nodes.
    pub fn add_node_added(count: u64) {
        NODES_ADDED.fetch_add(count, Ordering::Relaxed);
    }
    /// Adds `count` structurally removed hidden nodes.
    pub fn add_node_removed(count: u64) {
        NODES_REMOVED.fetch_add(count, Ordering::Relaxed);
    }

    /// `(events, synapses_added, synapses_removed, synapses_toggled, nodes_added, nodes_removed)`.
    pub fn snapshot() -> (u64, u64, u64, u64, u64, u64) {
        (
            BRAIN_EVENTS.load(Ordering::Relaxed),
            SYNAPSES_ADDED.load(Ordering::Relaxed),
            SYNAPSES_REMOVED.load(Ordering::Relaxed),
            SYNAPSES_TOGGLED.load(Ordering::Relaxed),
            NODES_ADDED.load(Ordering::Relaxed),
            NODES_REMOVED.load(Ordering::Relaxed),
        )
    }

    /// Resets all instrumentation counters.
    pub fn reset() {
        for counter in [
            &BRAIN_EVENTS,
            &SYNAPSES_ADDED,
            &SYNAPSES_REMOVED,
            &SYNAPSES_TOGGLED,
            &NODES_ADDED,
            &NODES_REMOVED,
        ] {
            counter.store(0, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod eval_order_tests {
    use super::*;

    fn node(index: usize, kind: NodeType) -> NodeRecord {
        NodeRecord {
            node_type: kind,
            index: index as u32,
            innovation: index as i64 + 1,
            desc: None,
            base_activation: 0.0,
            value: 10.0 + index as f32,
            last_input: 20.0 + index as f32,
            last_output: 30.0 + index as f32,
        }
    }

    fn hidden_fixture() -> (Vec<NodeRecord>, Vec<SynapseRecord>) {
        let mut nodes = Vec::with_capacity(51);
        for index in 0..N_INPUTS {
            nodes.push(node(index, NodeType::Input));
        }
        for index in N_INPUTS..N_INPUTS + N_OUTPUTS {
            nodes.push(node(index, NodeType::Sigmoid));
        }
        nodes.push(node(48, NodeType::Linear));
        nodes.push(node(49, NodeType::TanH));
        nodes.push(node(50, NodeType::Sigmoid));

        let synapses = vec![
            SynapseRecord {
                innovation: 1_001,
                node_in: 0,
                node_out: 50,
                weight: 1.0,
                enabled: true,
            },
            SynapseRecord {
                innovation: 1_002,
                node_in: 1,
                node_out: 34,
                weight: 1.0,
                enabled: true,
            },
            SynapseRecord {
                innovation: 1_003,
                node_in: 50,
                node_out: 49,
                weight: 1.0,
                enabled: true,
            },
            SynapseRecord {
                innovation: 1_004,
                node_in: 49,
                node_out: 34,
                weight: 1.0,
                enabled: true,
            },
            SynapseRecord {
                innovation: 1_005,
                node_in: 48,
                node_out: 50,
                weight: 1.0,
                enabled: true,
            },
            // Disabled: counts for floating-node repair (game NIn/NOut include
            // disabled synapses) but must not create a CSR target.
            SynapseRecord {
                innovation: 1_006,
                node_in: 2,
                node_out: 48,
                weight: 1.0,
                enabled: false,
            },
        ];
        (nodes, synapses)
    }

    /// The game's `ComputeProcessingStructure` orders `brainTargets` by the
    /// **first appearance of the destination in the saved synapse array**
    /// (`NEATBrain.cs:509-531`), not by node index, innovation or a topological
    /// sort. Incoming links inside a target keep ascending synapse-array order.
    /// Loading must therefore preserve the `.bb8` node and synapse array order.
    #[test]
    fn hidden_node_target_order_is_first_synapse_appearance() {
        let (nodes, synapses) = hidden_fixture();
        let brain = Brain::from_records(&nodes, &synapses).expect("fixture loads");

        // Targets: 50 (synapse 0), 34 (synapse 1), 49 (synapse 2). Node 48 has
        // only a disabled incoming synapse, so it is never a target (game rule).
        assert_eq!(brain.targets, vec![50, 34, 49]);
        assert_eq!(brain.offsets, vec![0, 2, 4, 5]);
        assert_eq!(brain.target_syns, vec![0, 4, 1, 3, 2]);

        // Saved order, not innovation/index order, is what the plan follows.
        let synapse_innovations: Vec<i64> = brain
            .to_synapse_records()
            .iter()
            .map(|synapse| synapse.innovation)
            .collect();
        assert_eq!(
            synapse_innovations,
            synapses
                .iter()
                .map(|synapse| synapse.innovation)
                .collect::<Vec<_>>()
        );
    }

    /// Load preserves all saved `Value`/`LastInput`/`LastOutput` state
    /// (`SaveSystem.LoadBibite` -> `SerializationHelper.DeserializeObject` then
    /// `ResumeBrain`, which only recomputes degrees/targets and does not
    /// re-execute neurons when there is no floating hidden node).
    #[test]
    fn from_records_preserves_game_runtime_state() {
        let (nodes, synapses) = hidden_fixture();
        let brain = Brain::from_records(&nodes, &synapses).expect("fixture loads");
        let records = brain.to_node_records();
        for (index, record) in records.iter().enumerate() {
            assert_eq!(record.value, nodes[index].value);
            assert_eq!(record.last_input, nodes[index].last_input);
            assert_eq!(record.last_output, nodes[index].last_output);
        }
        assert_eq!(brain.node_count(), 51);
        assert_eq!(brain.hidden_count(), 3);
    }

    /// `ComputeActiveNeurons` clamps SoftLatch bias to [0.1, 100] on load
    /// (`NEATBrain.cs:475-478`); other node types are left untouched.
    #[test]
    fn load_clamps_soft_latch_bias_only() {
        let (mut nodes, synapses) = hidden_fixture();
        let hidden: Vec<usize> = (FIRST_HIDDEN..nodes.len()).collect();
        nodes[hidden[0]].node_type = NodeType::SoftLatch;
        nodes[hidden[0]].base_activation = 1000.0;
        nodes[hidden[1]].node_type = NodeType::SoftLatch;
        nodes[hidden[1]].base_activation = -3.0;
        nodes[hidden[2]].node_type = NodeType::Sigmoid;
        nodes[hidden[2]].base_activation = 1000.0;
        let brain = Brain::from_records(&nodes, &synapses).expect("fixture loads");
        let records = brain.to_node_records();
        assert_eq!(records[hidden[0]].base_activation, 100.0);
        assert_eq!(records[hidden[1]].base_activation, 0.1);
        assert_eq!(records[hidden[2]].base_activation, 1000.0);
    }

    /// The editor's brain is stored as it is edited: the repair of `from_records` (a hidden node without a connection in or out
    /// goes, a soft latch's bias is clamped to 0.1 ..= 100) is not applied by `from_records_unrepaired`.
    #[test]
    fn unrepaired_records_keep_a_floating_node_and_a_large_soft_latch_bias() {
        let (mut nodes, mut synapses) = hidden_fixture();
        // Nothing comes into node 48 any more.
        synapses.retain(|synapse| synapse.innovation != 1_006);
        nodes[49].node_type = NodeType::SoftLatch;
        nodes[49].base_activation = 1000.0;

        let repaired = Brain::from_records(&nodes, &synapses).expect("fixture loads");
        assert_eq!(repaired.node_count(), 50, "the floating node is removed");
        assert_eq!(repaired.to_node_records()[48].base_activation, 100.0);

        let raw = Brain::from_records_unrepaired(&nodes, &synapses).expect("fixture loads");
        assert_eq!(raw.node_count(), 51);
        assert_eq!(raw.to_node_records()[49].base_activation, 1000.0);
        assert_eq!(raw.to_synapse_records().len(), synapses.len());
    }

    /// Template construction is the only game path that executes neurons on
    /// load (`FinalizeEditing`, `NEATBrain.cs:453-460`, index order); it must
    /// remain distinct from the live-save path above.
    #[test]
    fn template_records_still_initialize_states() {
        let (nodes, synapses) = hidden_fixture();
        let brain = Brain::from_template_records(&nodes, &synapses, DEFAULT_BRAIN_PERIOD)
            .expect("fixture loads");
        let records = brain.to_node_records();
        // Inputs are not executed by game FinalizeEditing; outputs/hidden are.
        assert_eq!(records[33].value, 0.5);
        assert!(records[48].value.is_finite());
        assert_ne!(records[48].last_input, nodes[48].last_input);
    }
}

#[cfg(test)]
mod feeding_diagnostic {
    use super::*;
    use crate::body::{BibiteSpawn, SpawnVitals};
    use crate::genome::GenomeMeta;
    use crate::ids::BibiteId;
    use crate::ids::PelletId;
    use crate::math::Vec2;
    use crate::settings::SimSettings;
    use crate::world::{ScenarioData, World};

    fn world(seed: u64) -> World {
        let mut settings = SimSettings::game_defaults();
        settings.independents.virgin_spawn_rate = 0.0;
        let mut world = World::new(settings, ScenarioData::with_default_zone_group(), seed)
            .expect("diagnostic world");
        let _ = world.seed_scenario_world();
        world
    }

    fn spawn_runtime(world: &mut World, brain: Brain, pos: Vec2) -> BibiteId {
        let spawn = BibiteSpawn {
            pos,
            rot: 0.0,
            vel: Vec2::ZERO,
            ang_vel: 0.0,
            genome: Genome::starter(),
            meta: GenomeMeta::default(),
            brain,
            vitals: SpawnVitals::Adult,
            energy: None,
            health: None,
        };
        let id = world.bibites.spawn(spawn).expect("spawn runtime bibite");
        world.register_bibite(id);

        id
    }

    fn first_pellet(world: &World) -> PelletId {
        world.pellets.ids().next().expect("scenario plant pellet")
    }

    fn snapshot(world: &mut World, id: BibiteId, pellet: PelletId) -> String {
        let Some(record) = world.bibites.get(id) else {
            return "MISSING BIBITE".to_string();
        };
        let mut inputs = BrainInputs::new();
        inputs.write_internal(record);
        inputs.write_world_senses(&record.senses);
        let outputs: Vec<String> = ALL_OUTPUTS
            .iter()
            .map(|output| {
                format!(
                    "{}={:.4}",
                    output.name(),
                    record.brain.output_value(*output)
                )
            })
            .collect();
        let amount = world
            .pellets
            .get(pellet)
            .map_or(f32::NAN, |view| view.amount);
        format!(
            "pos=({:.3},{:.3}) rot={:.3} ready={} born={} energy={:.2} space={:.4} mouth_open={:.4} can_swallow={} pellet={:.3} \
             plant_in=closeness {:.4} angle {:.4} n {:.4} held_in={} n_in_mouth={} bite={:.3} \
             desire_eat={:.3} desire_grab={:.3} contacts={} held={} outputs[{}]",
            record.pos.x,
            record.pos.y,
            record.rot,
            record.is_ready,
            record.born,
            record.body.energy.amount,
            record.stomach.available_space(),
            record.mouth.mouth_opening,
            record.mouth.can_swallow,
            amount,
            inputs.get(Input::PlantCloseness),
            inputs.get(Input::PlantAngle),
            inputs.get(Input::NPlants),
            inputs.get(Input::Fullness),
            record.mouth.n_in_mouth,
            record.mouth.bite_progress,
            record.mouth.desire_eat,
            record.mouth.desire_grab,
            record.n_mouth_targets,
            record.n_held_targets,
            outputs.join(","),
        )
    }

    /// Diagnostic for task-85/87: never-created-vs-loaded feeding.
    #[test]
    #[ignore = "task-87 diagnostic: run with --ignored --nocapture"]
    fn template_vs_loaded_feeding_probe() {
        // Use a scenario-grown plant pellet (the bug is in scenario feeding);
        // place the bibite 5 u behind it, facing +x.
        let template_seed_world = world(0xF00D);
        let plant = first_pellet(&template_seed_world);
        let plant_pos = template_seed_world
            .pellets
            .pos(plant)
            .expect("pellet position");
        let spawn_pos = plant_pos - Vec2::new(0.0, 5.0);
        drop(template_seed_world);

        // (a) runtime/template path: brain built from template records (the
        // scenario spawner path), then spawned through `BibiteStore::spawn`.
        let template_brain = {
            let starter = Brain::starter();
            Brain::from_template_records(
                &starter.to_node_records(),
                &starter.to_synapse_records(),
                DEFAULT_BRAIN_PERIOD,
            )
            .expect("template brain")
        };
        let mut template_world = world(0xF00D);
        let template_id = spawn_runtime(&mut template_world, template_brain, spawn_pos);
        let template_pellet = first_pellet(&template_world);

        // (b) save-loaded path for the same genome/brain: round-trip the
        // runtime record through BibiteSave and `spawn_loaded_bibite`.
        let mut source_world = world(0xF00D);
        let source_brain = {
            let starter = Brain::starter();
            Brain::from_records(&starter.to_node_records(), &starter.to_synapse_records())
                .expect("saved brain")
        };
        let source_id = spawn_runtime(&mut source_world, source_brain, spawn_pos);
        let save = source_world
            .bibites
            .to_save(source_id)
            .expect("serialize runtime bibite");
        let mut loaded_world = world(0xF00D);
        let loaded_id = loaded_world
            .spawn_loaded_bibite(save)
            .expect("load saved bibite");
        let loaded_pellet = first_pellet(&loaded_world);

        println!("=== task-87 feeding probe: runtime/template vs save-loaded ===");
        println!("template id {:?}, loaded id {:?}", template_id, loaded_id);
        for tick in 0..=1000_u64 {
            if matches!(tick, 0 | 10 | 25 | 50 | 100 | 200 | 400 | 600 | 800 | 1000) {
                println!("\n-- tick {tick} --");
                println!(
                    "template: {}",
                    snapshot(&mut template_world, template_id, template_pellet)
                );
                println!(
                    "loaded  : {}",
                    snapshot(&mut loaded_world, loaded_id, loaded_pellet)
                );
            }
            template_world.tick();
            loaded_world.tick();
        }
        println!(
            "\nfinal template: {}",
            snapshot(&mut template_world, template_id, template_pellet)
        );
        println!(
            "final loaded  : {}",
            snapshot(&mut loaded_world, loaded_id, loaded_pellet)
        );
    }
}

// ---------------------------------------------------------------------------
// Brain runtime checkpoint (task-108): per-entity state that the game save
// does not persist in a way the world can restore bit-identically.
// ---------------------------------------------------------------------------

const BRAIN_CHECKPOINT_MAGIC: [u8; 8] = *b"BRCHK001";
const BRAIN_CHECKPOINT_VERSION: u16 = 1;
const BRAIN_CHECKPOINT_HEADER: usize = 8 + 2 + 2 + 4;

/// Per-entity brain state that is not restored from the game save: node runtime
/// values, innovation counters, the effective herding flag and the lazy-plan
/// dirty bit.
///
/// Topology (node/synapse arrays) comes from the save; `topology_hash` guards
/// against applying a checkpoint to a different brain. The CSR plan itself is
/// derived and is rebuilt on restore. Brain cadence counters and the world RNG
/// are not part of a brain: the body store owns cadence and the world owns RNG.
#[derive(Clone, Debug, PartialEq)]
pub struct BrainCheckpoint {
    /// Number of nodes in the brain this checkpoint belongs to.
    pub node_count: u32,
    /// Number of synapses in the brain this checkpoint belongs to.
    pub synapse_count: u32,
    /// FNV-1a fingerprint of the immutable topology (node types/innovations/
    /// biases + synapse endpoints/weights/enabled/innovations).
    pub topology_hash: u64,
    /// `Node.Value`, indexed by node position.
    pub value: Vec<f32>,
    /// `Node.LastOutput`, indexed by node position.
    pub last_output: Vec<f32>,
    /// `Node.LastInput`, indexed by node position.
    pub last_input: Vec<f32>,
    /// Next hidden-node innovation (`nodeMaxInov`).
    pub node_max_innovation: i64,
    /// Next connection innovation (`connectionMaxInov`).
    pub connection_max_innovation: i64,
    /// `herdingEnabled` used to build the CSR plan.
    pub herding_enabled: bool,
    /// Lazy-plan dirty flag at capture time.
    pub plan_dirty: bool,
}

impl Brain {
    /// Captures the runtime checkpoint state for this brain (task-108).
    pub fn capture_checkpoint(&self) -> BrainCheckpoint {
        BrainCheckpoint {
            node_count: self.kind.len() as u32,
            synapse_count: self.syn_in.len() as u32,
            topology_hash: self.topology_hash(),
            value: self.value.clone(),
            last_output: self.last_out.clone(),
            last_input: self.last_in.clone(),
            node_max_innovation: self.node_max_innovation,
            connection_max_innovation: self.connection_max_innovation,
            herding_enabled: self.herding_enabled,
            plan_dirty: self.plan_dirty,
        }
    }

    /// Restores a [`BrainCheckpoint`] captured from the same brain topology.
    ///
    /// Validates counts, value lengths and the topology fingerprint before
    /// mutating, then rebuilds the derived CSR plan. Returns `Err` on a mismatch
    /// (wrong brain, corrupt checkpoint); the brain is untouched in that case.
    pub fn restore_checkpoint(&mut self, checkpoint: BrainCheckpoint) -> crate::error::Result<()> {
        if checkpoint.node_count as usize != self.kind.len() {
            return Err(Error::invalid_data(format!(
                "brain checkpoint has {} nodes, brain has {}",
                checkpoint.node_count,
                self.kind.len()
            )));
        }
        if checkpoint.synapse_count as usize != self.syn_in.len() {
            return Err(Error::invalid_data(format!(
                "brain checkpoint has {} synapses, brain has {}",
                checkpoint.synapse_count,
                self.syn_in.len()
            )));
        }
        if checkpoint.topology_hash != self.topology_hash() {
            return Err(Error::invalid_data(
                "brain checkpoint topology does not match this brain",
            ));
        }
        let n = self.kind.len();
        if checkpoint.value.len() != n
            || checkpoint.last_output.len() != n
            || checkpoint.last_input.len() != n
        {
            return Err(Error::invalid_data(
                "brain checkpoint runtime-vector length mismatch",
            ));
        }
        self.value = checkpoint.value;
        self.last_out = checkpoint.last_output;
        self.last_in = checkpoint.last_input;
        self.node_max_innovation = checkpoint.node_max_innovation;
        self.connection_max_innovation = checkpoint.connection_max_innovation;
        self.herding_enabled = checkpoint.herding_enabled;
        // Cached degrees/usage masks are derived from the (restored) topology;
        // recompute them before building the CSR so sensing/plan decisions are
        // identical to the captured brain.
        self.recompute_degrees();
        self.rebuild_csr();
        self.plan_dirty = checkpoint.plan_dirty;
        Ok(())
    }

    fn topology_hash(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        let mut mix = |value: u64| {
            hash ^= value;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        };
        mix(self.kind.len() as u64);
        mix(self.syn_in.len() as u64);
        for i in 0..self.kind.len() {
            mix(self.kind[i] as u64);
            mix(self.innovation[i] as u64);
            mix(u64::from(self.bias[i].to_bits()));
        }
        for s in 0..self.syn_in.len() {
            mix(self.syn_innovation[s] as u64);
            mix(u64::from(self.syn_in[s]));
            mix(u64::from(self.syn_out[s]));
            mix(u64::from(self.syn_weight[s].to_bits()));
            mix(u64::from(self.syn_enabled[s]));
        }
        hash
    }
}

impl BrainCheckpoint {
    /// Encodes the checkpoint as deterministic little-endian bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut payload = Vec::new();
        put_u32(&mut payload, self.node_count);
        put_u32(&mut payload, self.synapse_count);
        put_u64(&mut payload, self.topology_hash);
        payload.push(u8::from(self.plan_dirty));
        payload.push(u8::from(self.herding_enabled));
        put_i64(&mut payload, self.node_max_innovation);
        put_i64(&mut payload, self.connection_max_innovation);
        for &value in &self.value {
            put_u32(&mut payload, value.to_bits());
        }
        for &value in &self.last_output {
            put_u32(&mut payload, value.to_bits());
        }
        for &value in &self.last_input {
            put_u32(&mut payload, value.to_bits());
        }
        let checksum = fnv1a(&payload);
        let mut out = Vec::with_capacity(BRAIN_CHECKPOINT_HEADER + payload.len() + 8);
        out.extend_from_slice(&BRAIN_CHECKPOINT_MAGIC);
        out.extend_from_slice(&BRAIN_CHECKPOINT_VERSION.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&payload);
        out.extend_from_slice(&checksum.to_le_bytes());
        out
    }

    /// Decodes bytes produced by [`BrainCheckpoint::encode`].
    pub fn decode(bytes: &[u8]) -> crate::error::Result<Self> {
        let mut r = BrainCheckpointReader::new(bytes);
        if r.take(8)? != BRAIN_CHECKPOINT_MAGIC {
            return Err(Error::invalid_data("brain checkpoint magic mismatch"));
        }
        let version = r.u16()?;
        if version != BRAIN_CHECKPOINT_VERSION {
            return Err(Error::invalid_data(format!(
                "unsupported brain checkpoint version {version}"
            )));
        }
        let flags = r.u16()?;
        if flags != 0 {
            return Err(Error::invalid_data(
                "brain checkpoint has unsupported flags",
            ));
        }
        let payload_len = r.u32()? as usize;
        let payload = r.take(payload_len)?;
        let checksum = r.u64()?;
        if checksum != fnv1a(payload) {
            return Err(Error::invalid_data("brain checkpoint checksum mismatch"));
        }
        let mut p = BrainCheckpointReader::new(payload);
        let node_count = p.u32()?;
        let synapse_count = p.u32()?;
        if node_count as usize > MAX_NODES {
            return Err(Error::invalid_data(format!(
                "brain checkpoint node count {node_count} exceeds {MAX_NODES}"
            )));
        }
        if synapse_count as usize > MAX_SYNAPSES {
            return Err(Error::invalid_data(format!(
                "brain checkpoint synapse count {synapse_count} exceeds {MAX_SYNAPSES}"
            )));
        }
        let topology_hash = p.u64()?;
        let plan_dirty = match p.u8()? {
            0 => false,
            1 => true,
            _ => return Err(Error::invalid_data("invalid brain checkpoint plan flag")),
        };
        let herding_enabled = match p.u8()? {
            0 => false,
            1 => true,
            _ => return Err(Error::invalid_data("invalid brain checkpoint herding flag")),
        };
        let node_max_innovation = p.i64()?;
        let connection_max_innovation = p.i64()?;
        let n = node_count as usize;
        let mut value = Vec::with_capacity(n);
        for _ in 0..n {
            value.push(f32::from_bits(p.u32()?));
        }
        let mut last_output = Vec::with_capacity(n);
        for _ in 0..n {
            last_output.push(f32::from_bits(p.u32()?));
        }
        let mut last_input = Vec::with_capacity(n);
        for _ in 0..n {
            last_input.push(f32::from_bits(p.u32()?));
        }
        if !p.finished() {
            return Err(Error::invalid_data("brain checkpoint has trailing payload"));
        }
        if !r.finished() {
            return Err(Error::invalid_data("brain checkpoint has trailing bytes"));
        }
        Ok(Self {
            node_count,
            synapse_count,
            topology_hash,
            value,
            last_output,
            last_input,
            node_max_innovation,
            connection_max_innovation,
            herding_enabled,
            plan_dirty,
        })
    }
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

struct BrainCheckpointReader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> BrainCheckpointReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }
    fn take(&mut self, len: usize) -> crate::error::Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(len)
            .ok_or_else(|| Error::invalid_data("brain checkpoint length overflow"))?;
        if end > self.data.len() {
            return Err(Error::invalid_data("brain checkpoint is truncated"));
        }
        let result = &self.data[self.position..end];
        self.position = end;
        Ok(result)
    }
    fn u8(&mut self) -> crate::error::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> crate::error::Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> crate::error::Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> crate::error::Result<u64> {
        let b = self.take(8)?;
        let mut bytes = [0_u8; 8];
        bytes.copy_from_slice(b);
        Ok(u64::from_le_bytes(bytes))
    }
    fn i64(&mut self) -> crate::error::Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().map_err(
            |_| Error::invalid_data("brain checkpoint is truncated"),
        )?))
    }
    fn finished(&self) -> bool {
        self.position == self.data.len()
    }
}

#[cfg(test)]
mod brain_checkpoint_tests {
    use super::*;

    fn with_inputs() -> BrainInputs {
        let mut inputs = BrainInputs::new();
        inputs.set(Input::EnergyRatio, 0.75);
        inputs.set(Input::PlantCloseness, 0.5);
        inputs.set(Input::PlantAngle, -0.25);
        inputs.set(Input::Maturity, 1.2);
        inputs
    }

    #[test]
    fn capture_encode_decode_restore_roundtrip_and_step_equality() {
        let mut original = Brain::starter();
        // Seed plausible runtime state and exercise all three arrays.
        for i in 0..original.value.len() {
            original.value[i] = i as f32 * 0.125;
            original.last_out[i] = i as f32 * 0.25;
            original.last_in[i] = i as f32 * -0.5;
        }
        original.node_max_innovation = 1234;
        original.connection_max_innovation = 5678;
        original.plan_dirty = false;
        let checkpoint = original.capture_checkpoint();
        let bytes = checkpoint.encode();
        let decoded = BrainCheckpoint::decode(&bytes).expect("decode");
        assert_eq!(decoded, checkpoint);
        assert_eq!(decoded.encode(), bytes);

        let mut restored = original.clone();
        restored.value.fill(9.0);
        restored.last_out.fill(9.0);
        restored.last_in.fill(9.0);
        restored.restore_checkpoint(decoded).expect("restore");
        assert_eq!(restored, original);

        let inputs = with_inputs();
        let period = DEFAULT_BRAIN_PERIOD;
        let mut a = original.clone();
        let mut b = restored;
        let out_a = a.step(&inputs, period);
        let out_b = b.step(&inputs, period);
        for (x, y) in out_a.values().iter().zip(out_b.values()) {
            assert_eq!(x.to_bits(), y.to_bits(), "output bits differ");
        }
        assert_eq!(a.to_node_records(), b.to_node_records());
        assert_eq!(a.to_synapse_records(), b.to_synapse_records());
    }

    #[test]
    fn restore_rejects_wrong_topology_and_does_not_mutate() {
        let original = Brain::starter();
        let before = original.capture_checkpoint();
        let mut checkpoint = before.clone();
        checkpoint.node_count += 1;
        let mut brain = original.clone();
        assert!(brain.restore_checkpoint(checkpoint.clone()).is_err());
        assert_eq!(brain.capture_checkpoint(), before);
        checkpoint = before.clone();
        checkpoint.topology_hash ^= 1;
        assert!(brain.restore_checkpoint(checkpoint).is_err());
        assert_eq!(brain.capture_checkpoint(), before);
    }

    #[test]
    fn plan_dirty_flag_is_preserved_and_step_still_rebuilds() {
        let mut original = Brain::starter();
        original.plan_dirty = true;
        original.herding_enabled = false;
        let checkpoint = original.capture_checkpoint();
        let decoded = BrainCheckpoint::decode(&checkpoint.encode()).expect("decode");
        assert!(decoded.plan_dirty);
        assert!(!decoded.herding_enabled);
        let mut restored = Brain::starter();
        restored.restore_checkpoint(decoded).expect("restore");
        assert!(restored.plan_dirty());
        assert!(!restored.herding_enabled());
        let out = restored.step(&with_inputs(), DEFAULT_BRAIN_PERIOD);
        assert!(out.values().iter().all(|value| value.is_finite()));
        assert!(!restored.plan_dirty());
    }

    #[test]
    fn decode_rejects_malformed_checkpoints() {
        let bytes = Brain::starter().capture_checkpoint().encode();
        let mut bad = bytes.clone();
        bad[0] ^= 0xFF;
        assert!(BrainCheckpoint::decode(&bad).is_err());
        for cut in 0..bytes.len() {
            assert!(BrainCheckpoint::decode(&bytes[..cut]).is_err(), "cut {cut}");
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(BrainCheckpoint::decode(&trailing).is_err());
        let mut bad = bytes.clone();
        let len = bad.len();
        bad[len - 1] ^= 1;
        assert!(BrainCheckpoint::decode(&bad).is_err());
        // Oversized declared node count (header/payload parse) must fail, not allocate.
        let mut oversized = bytes.clone();
        let payload_start = 16 + 8; // magic/version/flags/len + topology header position
        if oversized.len() > payload_start + 8 {
            oversized[payload_start..payload_start + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(BrainCheckpoint::decode(&oversized).is_err());
        }
    }
}

#[cfg(test)]
mod brain_checkpoint_grown_tests {
    use super::*;

    fn grown_brain() -> Brain {
        let starter = Brain::starter();
        let mut nodes = starter.to_node_records();
        // Two extra hidden nodes: 48 is fed by input 0 and feeds 49; 49 feeds
        // output 33. Both have in/out degree, so no floating-node repair.
        nodes.push(NodeRecord {
            node_type: NodeType::TanH,
            index: 48,
            innovation: 100,
            desc: None,
            base_activation: 0.1,
            value: 0.0,
            last_input: 0.0,
            last_output: 0.0,
        });
        nodes.push(NodeRecord {
            node_type: NodeType::Sigmoid,
            index: 49,
            innovation: 101,
            desc: None,
            base_activation: -0.2,
            value: 0.0,
            last_input: 0.0,
            last_output: 0.0,
        });
        let mut synapses = starter.to_synapse_records();
        synapses.push(SynapseRecord {
            innovation: 1000,
            node_in: 0,
            node_out: 48,
            weight: 0.75,
            enabled: true,
        });
        synapses.push(SynapseRecord {
            innovation: 1001,
            node_in: 48,
            node_out: 49,
            weight: -0.5,
            enabled: true,
        });
        synapses.push(SynapseRecord {
            innovation: 1002,
            node_in: 49,
            node_out: 33,
            weight: 0.25,
            enabled: true,
        });
        let mut brain = Brain::from_records(&nodes, &synapses).expect("grown brain");
        for i in 0..brain.value.len() {
            brain.value[i] = (i as f32).sin();
            brain.last_out[i] = (i as f32).cos();
            brain.last_in[i] = (i as f32) * 0.01;
        }
        brain.node_max_innovation = 555;
        brain.connection_max_innovation = 777;
        brain
    }

    #[test]
    fn grown_brain_roundtrip_and_step_outputs_bit_equal() {
        let original = grown_brain();
        assert!(original.hidden_count() >= 2);
        let checkpoint = original.capture_checkpoint();
        let decoded = BrainCheckpoint::decode(&checkpoint.encode()).expect("decode");
        assert_eq!(decoded, checkpoint);
        let mut restored = grown_brain();
        restored.value.fill(123.0);
        restored.last_out.fill(123.0);
        restored.last_in.fill(123.0);
        restored.restore_checkpoint(decoded).expect("restore");
        assert_eq!(restored, original);

        let mut inputs = BrainInputs::new();
        inputs.set(Input::EnergyRatio, 0.4);
        inputs.set(Input::PlantCloseness, 0.9);
        inputs.set(Input::PlantAngle, 0.2);
        inputs.set(Input::Speed, 1.5);
        let period = DEFAULT_BRAIN_PERIOD;
        let mut a = original.clone();
        let mut b = restored;
        let out_a = a.step(&inputs, period);
        let out_b = b.step(&inputs, period);
        for (x, y) in out_a.values().iter().zip(out_b.values()) {
            assert_eq!(x.to_bits(), y.to_bits(), "grown output bits differ");
        }
        assert_eq!(a.to_node_records(), b.to_node_records());
        // Second step (plan clean path) also exact.
        let out_a = a.step(&inputs, period);
        let out_b = b.step(&inputs, period);
        for (x, y) in out_a.values().iter().zip(out_b.values()) {
            assert_eq!(x.to_bits(), y.to_bits(), "grown second-step bits differ");
        }
    }
    #[test]
    fn differences_expose_template_indices_and_match_distance() {
        let template = Brain::starter();
        let empty = template.differences(&template);
        assert_eq!(empty.total_distance(), 0.0);
        assert!(empty.connection_deltas.is_empty());
        assert!(empty.activation_deltas.is_empty());

        let mut nodes = template.to_node_records();
        nodes[39].base_activation += 0.5;
        let mut synapses = template.to_synapse_records();
        let first_innovation = synapses[0].innovation;
        synapses[0].weight += 1.0;
        synapses[0].enabled = !synapses[0].enabled;
        let individual = Brain::from_records(&nodes, &synapses).expect("valid brain");
        let diff = template.differences(&individual);
        assert!(
            diff.activation_deltas
                .iter()
                .any(|d| d.template_index == 39)
        );
        assert!(diff.connection_deltas.iter().any(|d| d.template_index == 0));
        assert!(diff.toggled.contains(&first_innovation));
        assert!((diff.total_distance() - template.distance(&individual, &())).abs() < 1e-5);
    }
}

#[cfg(test)]
mod brain_graph_view_tests {
    use super::*;

    #[test]
    fn graph_view_exposes_nodes_synapses_and_outputs() {
        let brain = Brain::starter();
        let view = brain.graph_view();
        assert_eq!(view.nodes.len(), brain.node_count());
        assert_eq!(view.synapses.len(), brain.synapse_count());
        assert_eq!(view.outputs, brain.output_values_copy());
        assert_eq!(view.nodes[0].label, "EnergyRatio");
        assert_eq!(view.nodes[33].label, "Accelerate");
        assert_eq!(view.nodes[33].node_type, NodeType::TanH);
        assert_eq!(view.synapses[0].node_in as u32, 3);
        assert_eq!(view.synapses[0].node_out as u32, 39);
        assert!(view.synapses[0].enabled);
        // The view is a copy: mutating the brain afterwards does not change it.
        let before = view.nodes[33].value;
        let mut brain = brain;
        brain.set_output_value(Output::Accelerate, before + 0.25);
        assert_eq!(view.nodes[33].value, before);
    }
}

#[cfg(test)]
mod editor_api_tests {
    use super::*;

    #[test]
    fn editor_add_edit_and_remove_graph_parts() {
        let mut brain = Brain::starter();
        let base_nodes = brain.node_count();
        let base_synapses = brain.synapse_count();
        let hidden = brain
            .add_hidden_node(NodeType::TanH, Some("Editor hidden"))
            .expect("hidden node");
        assert_eq!(hidden, base_nodes);
        assert_eq!(brain.node_count(), base_nodes + 1);
        assert_eq!(brain.node_desc(hidden), Some("Editor hidden"));
        assert_eq!(brain.node_type(hidden), Some(NodeType::TanH));
        let draft = brain.validate_editor();
        assert!(
            draft.is_valid(),
            "disconnected hidden nodes are allowed: {draft:?}"
        );
        assert_eq!(draft.floating_hidden, vec![hidden]);

        let input_to_hidden = brain
            .add_synapse_connection(0, hidden, 0.5, true)
            .expect("input -> hidden");
        let hidden_to_output = brain
            .add_synapse_connection(hidden, FIRST_OUTPUT, 0.25, true)
            .expect("hidden -> output");
        assert!(brain.validate_editor().floating_hidden.is_empty());
        assert_eq!(brain.synapse_count(), base_synapses + 2);
        assert!(brain.set_synapse_weight(input_to_hidden, 2.0));
        assert!(brain.set_synapse_enabled(hidden_to_output, false));
        assert!(brain.set_node_bias_at(hidden, 0.75));
        assert!(brain.set_hidden_node_type(hidden, NodeType::Linear));
        assert!(brain.set_hidden_node_label(hidden, "Renamed"));
        assert_eq!(brain.node_desc(hidden), Some("Renamed"));

        // Duplicate endpoints, self connections, output sources and input
        // destinations are rejected.
        assert!(brain.add_synapse_connection(0, hidden, 1.0, true).is_none());
        assert!(brain.add_synapse_connection(0, 0, 1.0, true).is_none());
        assert!(
            brain
                .add_synapse_connection(FIRST_OUTPUT, hidden, 1.0, true)
                .is_none()
        );
        assert!(brain.add_synapse_connection(hidden, 0, 1.0, true).is_none());

        let mut inputs = BrainInputs::new();
        inputs.set(Input::EnergyRatio, 1.0);
        brain.step(&inputs, 0.5);

        assert!(brain.remove_synapse_at(hidden_to_output));
        assert_eq!(brain.synapse_count(), base_synapses + 1);
        assert!(brain.remove_hidden_node(hidden));
        assert_eq!(brain.node_count(), base_nodes);
        assert_eq!(brain.synapse_count(), base_synapses);
        assert!(!brain.remove_hidden_node(hidden));
        assert!(!brain.remove_synapse_at(999));
    }
}

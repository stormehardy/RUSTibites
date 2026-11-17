//! Parity capture emitter (`rustibites.parity.*` schema).
//!
//! Emits the Rust half of the game-vs-Rust capture format defined by
//! `docs/PARITY.md` §6 and `tools/parity/parity_common.py`, so that
//! `tools/parity/compare.py` can diff a ground-truth game capture against a
//! Rust capture.
//!
//! Output is a plain (uncompressed) capture directory — accepted by the harness
//! (`parity_common.open_capture`):
//!
//! ```text
//! <capture>/
//!   manifest.json      rustibites.parity.capture.v1  (pretty JSON)
//!   ticks.jsonl        rustibites.parity.tick.v1     (one record per sampled tick)
//!   events.jsonl       rustibites.parity.event.v1    (optional birth/death log)
//! ```
//!
//! Writers are streaming and deterministic: struct field order is the JSON key
//! order, and `finish()` writes `manifest.json` last with per-file
//! `{name, sha256, bytes, records}` entries.
//!
//! # Wiring (Lead/engine side)
//!
//! `run --capture DIR` should call, per sampled tick:
//!
//! 1. `World::sample_stats()` → build [`TickRecord`] (fields from the stats
//!    sample; widen `f32` values with `f64::from(value)`, never `as f64` on an
//!    `f64` computed from text);
//! 2. fill entity snapshots from `BibiteStore`/`EggStore` using
//!    [`ParityEntityRecord::set_brain`] / [`ParityEntityRecord::set_genes`]
//!    (digests come from [`crate::parity`], not from this module);
//! 3. `writer.push_tick(record)`.
//!
//! Births/deaths: `writer.push_event(EventRecord { ... })`.
//!
//! At end of run: `writer.finish()` (writes manifest) after filling
//! `manifest.inputs`/`manifest.sim`/`manifest.world`/`counts_*`; settings come
//! from `bibites-io::to_bridge_settings_json(&SimSettings)` and the hash from
//! [`settings_hash`].
//!
//! The aggregate [`TickRecord`] fields mirror `docs/PARITY.md` §6.1 exactly;
//! optional groups are omitted (never `null`) when the Rust engine does not
//! model them yet, and `compare.py` reports those gaps explicitly.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::parity::{
    self, CAPTURE_SCHEMA, EVENT_SCHEMA, GENE_NAMES, TICK_SCHEMA, brain_digest, genes_digest,
    output_digest, sense_digest,
};
use bibites_core::body::{BibiteRecord, DeathCause, bibite_id_from_tag, pellet_id_from_tag};
use bibites_core::brain::Output;
use bibites_core::brain::{ALL_OUTPUTS, Brain};
use bibites_core::genome::Genome;
use bibites_core::ids::BibiteId;
use bibites_core::sim::StatsSample;
use bibites_core::world::World;

/// Number of hex characters kept by the Python harness digests.
const DIGEST_CHARS: usize = parity::DIGEST_CHARS;

/// Serde helper for `[f64; 35]` fields (`serde` only derives arrays up to 32).
///
/// Serializes exactly as a JSON array of 35 numbers; deserializing enforces the
/// length so a malformed record fails loudly.
mod gene_array_serde {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(value: &[f64; 35], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value.as_slice().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[f64; 35], D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = Vec::<f64>::deserialize(deserializer)?;
        let length = values.len();
        values
            .try_into()
            .map_err(|_: Vec<f64>| D::Error::invalid_length(length, &"an array of 35 gene values"))
    }
}

// ---------------------------------------------------------------------------
// settings_hash — must match parity_common.settings_hash byte-for-byte
// ---------------------------------------------------------------------------

/// SHA-256 over the bridge `/settings` JSON, encoded bit-exactly like
/// `parity_common.settings_hash` (docs/PARITY.md §6.3).
///
/// For each scope in `("scenario", "independent")`:
///
/// ```text
/// scope_bytes, "{",
///   for each key in sorted order: key_bytes, "=", tag, payload, ";",
/// "}"
/// ```
///
/// with tags/payloads: `b` + one byte (bool, 0/1), `i` + `i32` little-endian,
/// `f` + `f32` little-endian. The result is the full 64-character lowercase hex
/// digest (not truncated).
///
/// `settings` is the object produced by `bibites-io::to_bridge_settings_json`;
/// it must contain only `bool`, integer and float values in the two scopes.
/// Missing scopes hash as empty objects (matching the Python `or {}`).
/// Unsupported JSON values (null/string/array/object) are a programming error:
/// they are debug-asserted and encoded as an `f32` `0.0` so release builds stay
/// deterministic rather than panicking.
pub fn settings_hash(settings: &serde_json::Value) -> String {
    let mut hasher = Sha256::new();
    for scope in ["scenario", "independent"] {
        hasher.update(scope.as_bytes());
        hasher.update(b"{");
        let values = settings.get(scope).and_then(serde_json::Value::as_object);
        if let Some(values) = values {
            let mut keys: Vec<&String> = values.keys().collect();
            keys.sort_unstable();
            for key in keys {
                let value = &values[key.as_str()];
                hasher.update(key.as_bytes());
                hasher.update(b"=");
                if let Some(flag) = value.as_bool() {
                    hasher.update(b"b");
                    hasher.update([u8::from(flag)]);
                } else if let Some(integer) = value.as_i64() {
                    // Game integer settings are i32 (docs/spec/04); the Python
                    // harness packs them as `<i`, so out-of-range values are a
                    // capture-side error.
                    debug_assert!(
                        i32::try_from(integer).is_ok(),
                        "settings_hash: integer setting {key}={integer} does not fit i32"
                    );
                    hasher.update(b"i");
                    hasher.update((integer as i32).to_le_bytes());
                } else if let Some(float) = value.as_f64() {
                    hasher.update(b"f");
                    hasher.update((float as f32).to_le_bytes());
                } else {
                    debug_assert!(false, "settings_hash: unsupported value for {key}: {value}");
                    hasher.update(b"f");
                    hasher.update(0.0_f32.to_le_bytes());
                }
                hasher.update(b";");
            }
        }
        hasher.update(b"}");
    }
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

// ---------------------------------------------------------------------------
// Record groups (docs/PARITY.md §6.1)
// ---------------------------------------------------------------------------

/// Population counts (`counts`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Counts {
    /// Living bibites.
    pub bibites: u32,
    /// Incubating eggs.
    pub eggs: u32,
    /// Corpses.
    pub corpses: u32,
}

/// Pellet counters and biomass (`pellets`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Pellets {
    /// Plant pellets.
    pub plant_n: u32,
    /// Plant biomass (energy units).
    pub plant_biomass: f64,
    /// Meat pellets (incl. corpses turned to meat).
    pub meat_n: u32,
    /// Meat biomass (energy units).
    pub meat_biomass: f64,
    /// Free (unclaimed) biomass.
    pub free_biomass: f64,
    /// Biomass bound in bibites.
    pub bibite_biomass: f64,
}

/// Living-population vital means (`vitals`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vitals {
    /// Mean energy.
    pub energy_mean: f64,
    /// Mean health.
    pub health_mean: f64,
    /// Mean age (s).
    pub age_mean: f64,
    /// Maximum age (s).
    pub age_max: f64,
}

/// Generation statistics (`generation`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GenerationStats {
    /// Highest generation present.
    pub gen_max: u32,
    /// Mean generation.
    pub gen_mean: f64,
}

/// Active-species statistics (`species`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SpeciesStats {
    /// Number of active species.
    pub n: u32,
    /// Per-species population, keyed by decimal species id.
    pub counts: BTreeMap<String, u32>,
}

/// Brain-size statistics (`brain`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BrainStats {
    /// Mean node count.
    pub nodes_mean: f64,
    /// Maximum node count.
    pub nodes_max: u32,
    /// Mean synapse count.
    pub syn_mean: f64,
    /// Maximum synapse count.
    pub syn_max: u32,
}

/// Per-gene population statistics (`genes`), in game gene-enum order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenesStats {
    /// Mean of each of the 35 genes.
    #[serde(with = "gene_array_serde")]
    pub mean: [f64; 35],
    /// Standard deviation of each of the 35 genes.
    #[serde(with = "gene_array_serde")]
    pub std: [f64; 35],
}

impl Default for GenesStats {
    fn default() -> Self {
        Self {
            mean: [0.0; 35],
            std: [0.0; 35],
        }
    }
}

/// Birth/death deltas between samples (`lifecycle`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Lifecycle {
    /// Births since the previous sample.
    pub births: i64,
    /// Deaths since the previous sample.
    pub deaths: i64,
}

/// Measured behaviour statistics (`behavior`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Behavior {
    /// Mean speed (body lengths/s).
    pub speed_mean: f64,
    /// Maximum speed.
    pub speed_max: f64,
    /// Fraction of the population moving.
    pub moving_frac: f64,
    /// Mean travel rate.
    pub travel_rate_mean: f64,
    /// Mean bite rate.
    pub bite_rate_mean: f64,
    /// Mean damage dealt rate.
    pub damage_dealt_rate_mean: f64,
    /// Mean egg-laying rate.
    pub egg_rate_mean: f64,
    /// Mean activation of each of the 15 brain outputs, keyed by node name.
    pub outputs: BTreeMap<String, f64>,
}

/// Body statistics (`body`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BodyStats {
    /// Mean size (d2 size).
    pub size_mean: f64,
    /// Mean area.
    pub area_mean: f64,
    /// Mean mass.
    pub mass_mean: f64,
    /// Mean maximum energy.
    pub max_energy_mean: f64,
    /// Mean maximum health.
    pub max_health_mean: f64,
    /// Mean metabolism rate.
    pub metabolism_mean: f64,
}

/// Tag statistics (`tags`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TagStats {
    /// Number of tagged bibites.
    pub n: u32,
    /// Tagged biomass.
    pub biomass: f64,
}

// ---------------------------------------------------------------------------
// Entity record (docs/PARITY.md §6.2)
// ---------------------------------------------------------------------------

/// One per-entity snapshot inside a [`TickRecord`] (`entities[]`).
///
/// Digests use the cross-language definitions from [`crate::parity`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ParityEntityRecord {
    /// `"bibite"` or `"egg"`.
    pub kind: String,
    /// Stable save id for bibites; eggs have none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    /// Species id (`genes.speciesID`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub species: Option<u32>,
    /// Generation (`genes.gen`).
    pub generation: u32,
    /// Lineage tag.
    pub tag: String,
    /// World x position.
    pub x: f64,
    /// World y position.
    pub y: f64,
    /// x velocity.
    pub vx: f64,
    /// y velocity.
    pub vy: f64,
    /// Rotation in degrees.
    pub rot: f64,
    /// Energy.
    pub energy: f64,
    /// Maximum energy (population table for the game; derived for Rust).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_energy: Option<f64>,
    /// Health.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<f64>,
    /// Maximum health.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_health: Option<f64>,
    /// Age in seconds (`None` for eggs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_s: Option<f64>,
    /// d2 size (`None` for eggs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// Node count.
    pub nodes: u32,
    /// Synapse count.
    pub synapses: u32,
    /// Digest of the 35 gene values (`None` when the genome is unavailable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genes_digest: Option<String>,
    /// Digest of the 33 input node values.
    pub sense_digest: String,
    /// Digest of the 15 output nodes' Value/LastInput/LastOutput.
    pub output_digest: String,
    /// Digest of full brain topology + node runtime state + synapses.
    pub brain_digest: String,
    /// Optional embedded gene values (only when a capture wants them; exactly 35).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genes: Option<Vec<f64>>,
}

impl ParityEntityRecord {
    /// Creates a zeroed entity record of the given kind (`"bibite"`/`"egg"`).
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            ..Self::default()
        }
    }

    /// Fills the brain-derived fields (node/synapse counts and all
    /// sense/output/brain digests) from a live [`bibites_core::brain::Brain`].
    pub fn set_brain(&mut self, brain: &Brain) {
        self.nodes = brain.node_count() as u32;
        self.synapses = brain.synapse_count() as u32;
        self.sense_digest = sense_digest(brain);
        self.output_digest = output_digest(brain);
        self.brain_digest = brain_digest(brain);
    }

    /// Fills the gene digest, and optionally the embedded gene values (as
    /// `f64`-widened `f32`s, matching the game's JSON doubles).
    pub fn set_genes(&mut self, genome: &Genome, embed: bool) {
        self.genes_digest = Some(genes_digest(genome));
        if embed {
            self.genes = Some(
                genome
                    .values()
                    .iter()
                    .map(|value| f64::from(*value))
                    .collect(),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Tick record (docs/PARITY.md §6.1) and event record (§6)
// ---------------------------------------------------------------------------

/// One sampled-tick record (`ticks.jsonl`, `rustibites.parity.tick.v1`).
///
/// Field order matches the schema table; optional groups are omitted when
/// [`None`]. All floats are `f64` holding `f32` values widened via
/// `f64::from(...)` so JSON doubles match the game capture bit-for-bit.
/// Per-bibite sample of the optional `rustibites.parity.feeding.v1` stream
/// (subset of the game's parity feeding plugin; used by tools/parity/feeding_ref.py).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FeedingBibite {
    /// External stable id.
    pub id: i64,
    /// Energy pool.
    pub energy: f64,
    /// Health pool.
    pub health: f64,
    /// Body d2 size.
    pub d2_size: f64,
    /// World position.
    pub pos: FeedingPos,
    /// Total stomach matter (u²).
    pub stomach_total: f64,
    /// Stomach matter by material (`plant`/`meat`, u²).
    pub stomach_materials: BTreeMap<String, f64>,
    /// Number of held pellets.
    pub n_held: usize,
    /// Held pellet descriptors (currently emitted empty; ownership/amount is
    /// available from the pellet stream in the game plugin).
    pub held_pellets: Vec<serde_json::Value>,
    /// Bite timer progress (s).
    pub bite_progress: f64,
    /// Bite period (s).
    pub bite_period: f64,
    /// Legacy bibite-bite counter (first bites).
    pub bibites_bitten: u32,
    /// Cumulative bite damage dealt.
    pub damage_dealt: f64,
    /// Objects currently inside the mouth trigger (`BibiteMouth.nHeld`/in-mouth).
    pub n_in_mouth: u8,
    /// Cumulative ticks with at least one mouth target.
    pub mouth_contact_ticks: u64,
    /// Cumulative ticks with a plant pellet inside the mouth trigger.
    pub plant_contact_ticks: u64,
    /// Cumulative no-contact → contact transitions.
    pub mouth_contact_enter_events: u64,
    /// Cumulative energy accepted (`BodyState::gain_energy`), death-budget aid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_gained_e: Option<f64>,
    /// Cumulative energy spent (`BodyState::use_energy`), death-budget aid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_spent_e: Option<f64>,
    /// Cumulative matter dissolved by digestion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stomach_digested_u2: Option<f64>,
    /// Cumulative energy gained from digestion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stomach_energy_gain_e: Option<f64>,
    /// Cumulative living seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_seconds: Option<f64>,
    /// Growth state variable (`growth`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub growth: Option<f64>,
    /// Fat reserve amount (u²).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fat_amount: Option<f64>,
    /// Cumulative plant matter ingested (u²).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plant_consumed_u2: Option<f64>,
    /// Cumulative meat matter ingested (u²).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meat_consumed_u2: Option<f64>,
    /// Eggs laid by this bibite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eggs_laid: Option<u32>,
    /// Estimated cumulative egg energy invested (`n_laid * egg_energy`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egg_energy_spent_estimate: Option<f64>,
    /// Last applied growth value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_growth: Option<f64>,
    /// Plant pellet targets currently in the mouth trigger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n_plant_targets: Option<u32>,
    /// Bibite targets currently in the mouth trigger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n_bibite_targets: Option<u32>,
}

/// Position object inside a feeding sample.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FeedingPos {
    /// World x.
    pub x: f64,
    /// World y.
    pub y: f64,
}

/// Cumulative world conservation totals + stock at one sample (task-99).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FeedingConservation {
    /// Initial zone-seeding energy (plant).
    pub seeded_plant_energy: f64,
    /// Initial zone-seeding energy (meat).
    pub seeded_meat_energy: f64,
    /// Zone-produced energy (plant).
    pub produced_plant_energy: f64,
    /// Zone-produced energy (meat).
    pub produced_meat_energy: f64,
    /// Recycled energy (plant).
    pub recycled_plant_energy: f64,
    /// Recycled energy (meat).
    pub recycled_meat_energy: f64,
    /// Decayed energy (plant).
    pub decayed_plant_energy: f64,
    /// Decayed energy (meat).
    pub decayed_meat_energy: f64,
    /// Culled energy (plant).
    pub culled_plant_energy: f64,
    /// Culled energy (meat).
    pub culled_meat_energy: f64,
    /// Explicitly removed energy (plant).
    pub removed_plant_energy: f64,
    /// Explicitly removed energy (meat).
    pub removed_meat_energy: f64,
    /// Consumed plant matter (u²).
    pub consumed_plant_u2: f64,
    /// Consumed meat matter (u²).
    pub consumed_meat_u2: f64,
    /// Consumed plant energy (E).
    pub consumed_plant_energy: f64,
    /// Consumed meat energy (E).
    pub consumed_meat_energy: f64,
    /// Live plant pellet energy at this sample.
    pub stock_plant_energy: f64,
    /// Live meat pellet energy at this sample.
    pub stock_meat_energy: f64,
    /// True while a conservation term (created meat) is not yet tracked.
    pub terms_missing: bool,
}

/// Optional `rustibites.parity.feeding.v1` sample (one JSONL row per tick).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedingSample {
    /// Schema id.
    pub schema: String,
    /// Simulation tick.
    pub tick: i64,
    /// Simulation time (s).
    pub t: f64,
    /// Per-bibite samples.
    pub bibites: Vec<FeedingBibite>,
    /// World pellet totals.
    pub totals: FeedingTotals,
    /// Runtime feeding counters (Rust engine; cumulative).
    pub counters: BTreeMap<String, f64>,
    /// Material properties used by the energy proxy.
    pub material_properties: BTreeMap<String, FeedingMaterial>,
    /// World conservation terms/stock (task-99; cumulative, runtime-only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conservation: Option<FeedingConservation>,
}

/// Pellet stock totals for one feeding sample.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FeedingTotals {
    /// Live pellets.
    pub pellets_n: usize,
    /// Plant pellet matter (u²).
    pub plant_amount: f64,
    /// Meat pellet matter (u²).
    pub meat_amount: f64,
}

/// Physics-material properties used by `feeding_ref.py`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FeedingMaterial {
    /// Energy per u².
    pub energy_density: f64,
    /// Mass per u².
    pub mass_density: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TickRecord {
    /// `rustibites.parity.tick.v1`.
    pub schema: String,
    /// `"rust"` (or `"game"` for game captures).
    pub source: String,
    /// Absolute simulation tick since the base save state (tick 0 = base).
    pub tick: i64,
    /// Simulated time in seconds.
    pub t: f64,
    /// Simulation ticks per second.
    pub sim_tps: u32,
    /// Brain update factor (ticks per brain update).
    pub brain_tps: u32,
    /// `settings_hash` of the capture's effective settings.
    pub settings_hash: String,
    /// True for the start snapshot (entities/counts only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial: Option<bool>,
    /// Population counts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counts: Option<Counts>,
    /// Pellet counters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pellets: Option<Pellets>,
    /// Vital means.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vitals: Option<Vitals>,
    /// Generation stats.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<GenerationStats>,
    /// Species stats.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub species: Option<SpeciesStats>,
    /// Brain-size stats.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brain: Option<BrainStats>,
    /// Gene stats.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genes: Option<GenesStats>,
    /// Birth/death deltas.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<Lifecycle>,
    /// Behaviour stats.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behavior: Option<Behavior>,
    /// Body stats.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<BodyStats>,
    /// Tag stats.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<BTreeMap<String, TagStats>>,
    /// Game-native metrics (`metric.*` sampler columns).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<BTreeMap<String, f64>>,
    /// Per-entity snapshots (present at snapshot ticks).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<ParityEntityRecord>,
}

impl TickRecord {
    /// Creates the mandatory tick-record fields.
    pub fn new(
        source: impl Into<String>,
        tick: i64,
        t: f64,
        sim_tps: u32,
        brain_tps: u32,
        settings_hash: impl Into<String>,
    ) -> Self {
        Self {
            schema: TICK_SCHEMA.to_string(),
            source: source.into(),
            tick,
            t,
            sim_tps,
            brain_tps,
            settings_hash: settings_hash.into(),
            partial: None,
            counts: None,
            pellets: None,
            vitals: None,
            generation: None,
            species: None,
            brain: None,
            genes: None,
            lifecycle: None,
            behavior: None,
            body: None,
            tags: None,
            metrics: None,
            entities: Vec::new(),
        }
    }

    /// Convenience constructor for a Rust capture tick.
    pub fn rust(tick: i64, t: f64, sim_tps: u32, brain_tps: u32, settings_hash: &str) -> Self {
        Self::new("rust", tick, t, sim_tps, brain_tps, settings_hash)
    }
}

/// One birth/death record (`events.jsonl`, `rustibites.parity.event.v1`).
///
/// The game capture passes through the bridge's event fields; comparison only
/// relies on `tick`, `kind` and `id`, so the rest are optional and skipped when
/// unknown.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EventRecord {
    /// `rustibites.parity.event.v1`.
    pub schema: String,
    /// Simulation tick of the event.
    pub tick: i64,
    /// `"initial"`, `"birth"` or `"death"` (plus any future game kinds).
    pub kind: String,
    /// Event time in seconds (when the provider reports it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t: Option<f64>,
    /// Bibite/egg id involved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    /// Species id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub species: Option<u32>,
    /// Lineage tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Generation (`gen` in the bridge payload).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<u32>,
    /// First parent id (bridge event key `p1`; 0 when unknown).
    #[serde(rename = "p1", skip_serializing_if = "Option::is_none")]
    pub p1: Option<i64>,
    /// Second parent id (bridge event key `p2`; 0 when unknown).
    #[serde(rename = "p2", skip_serializing_if = "Option::is_none")]
    pub p2: Option<i64>,
    /// Event x position.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    /// Event y position.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    /// Age in seconds at the event (`ageS` in the bridge payload).
    #[serde(rename = "ageS", skip_serializing_if = "Option::is_none")]
    pub age_s: Option<f64>,
    /// Absolute birth time in seconds (`bornAt` on death events).
    #[serde(rename = "bornAt", skip_serializing_if = "Option::is_none")]
    pub born_at: Option<f64>,
    /// Whether the entity was part of the initial population (death events).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial: Option<bool>,
    /// Energy at death.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy: Option<f64>,
    /// Maximum energy at death.
    #[serde(rename = "maxEnergy", skip_serializing_if = "Option::is_none")]
    pub max_energy: Option<f64>,
    /// Health at death.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<f64>,
    /// Maximum health at death.
    #[serde(rename = "maxHealth", skip_serializing_if = "Option::is_none")]
    pub max_health: Option<f64>,
    /// Damage taken.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub damage: Option<f64>,
    /// Number of attacks received.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attacked: Option<u32>,
    /// Total distance travelled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub travel: Option<f64>,
    /// Number of bites delivered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bites: Option<u32>,
    /// Damage dealt to other bibites.
    #[serde(rename = "damageDealt", skip_serializing_if = "Option::is_none")]
    pub damage_dealt: Option<f64>,
    /// Eggs laid.
    #[serde(rename = "eggsLaid", skip_serializing_if = "Option::is_none")]
    pub eggs_laid: Option<u32>,
    /// Death cause string (`"starved"`, `"killed"`, `"other"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
}

impl EventRecord {
    /// Creates an event record with the mandatory schema/tick/kind fields.
    pub fn new(tick: i64, kind: impl Into<String>) -> Self {
        Self {
            schema: EVENT_SCHEMA.to_string(),
            tick,
            kind: kind.into(),
            ..Self::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Canonical record digest (parity_common.record_digest)
// ---------------------------------------------------------------------------

/// Digest of one tick record's **aggregate** telemetry
/// (`parity_common.record_digest`): SHA-256 over canonical JSON (sorted keys,
/// compact separators, no ASCII escaping), first 16 hex chars.
///
/// Entity snapshots and comparison-only bookkeeping keys (`wall`, `capture`,
/// `pass`, `entities`) are excluded, exactly like the Python harness, because
/// paused snapshots between game/Rust runs may land on slightly different
/// ticks. Used by `--repeat`/determinism CI.
///
/// Canonical form is produced by [`canonical_record_json`] (keys sorted at every
/// nesting level — `serde_json`'s map order depends on workspace features, so it
/// is not relied upon).
pub fn record_digest(record: &TickRecord) -> Result<String, serde_json::Error> {
    Ok(sha16(canonical_record_json(record)?.as_bytes()))
}

/// Canonical JSON text used by [`record_digest`] (useful for debugging diffs).
///
/// Matches `parity_common.canonical_json` for the record types in this module:
/// recursively sorted object keys, `,`/`:` separators, UTF-8 strings emitted
/// unescaped (control characters escaped). Arrays keep their order.
pub fn canonical_record_json(record: &TickRecord) -> Result<String, serde_json::Error> {
    let mut value = serde_json::to_value(record)?;
    if let serde_json::Value::Object(ref mut object) = value {
        for key in ["wall", "capture", "pass", "entities"] {
            object.remove(key);
        }
    }
    let mut output = String::new();
    write_canonical_json(&value, &mut output);
    Ok(output)
}

fn write_canonical_json(value: &serde_json::Value, output: &mut String) {
    use serde_json::Value;
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(flag) => output.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => output.push_str(&number.to_string()),
        Value::String(text) => {
            // `serde_json` writes a valid JSON string; non-ASCII stays raw,
            // matching Python's `ensure_ascii=False`.
            match serde_json::to_string(text) {
                Ok(encoded) => output.push_str(&encoded),
                Err(_) => output.push_str("\"\""),
            }
        }
        Value::Array(items) => {
            output.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_canonical_json(item, output);
            }
            output.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable();
            output.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                match serde_json::to_string(key) {
                    Ok(encoded) => output.push_str(&encoded),
                    Err(_) => output.push_str("\"\""),
                }
                output.push(':');
                write_canonical_json(&map[key], output);
            }
            output.push('}');
        }
    }
}

fn sha16(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex.truncate(DIGEST_CHARS);
    hex
}

// ---------------------------------------------------------------------------
// Manifest (docs/PARITY.md §6.4)
// ---------------------------------------------------------------------------

/// `{path, sha256}` input file reference.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FileRef {
    /// Path as given on the command line (may be absolute).
    pub path: String,
    /// SHA-256 of the file bytes.
    pub sha256: String,
}

/// Base save reference (`inputs.base`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BaseRef {
    /// Path of the base save (zip).
    pub path: String,
    /// SHA-256 of the save file.
    pub sha256: String,
    /// Simulated time stored in the save (`scene.bb8scene.simulatedTime`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sim_time: Option<f64>,
}

/// Requested settings files/scopes (`inputs.settings_requested`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SettingsRequested {
    /// Scenario-scope settings requested from the base/scenario.
    pub scenario: serde_json::Value,
    /// Independent-scope settings requested (scenario-independent file).
    pub independent: serde_json::Value,
}

/// Manifest `inputs` block: everything that identifies the run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Inputs {
    /// Scenario reference, if the capture started from a scenario.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scenario: Option<FileRef>,
    /// Base save reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<BaseRef>,
    /// Save input (opaque: path or object as provided by the harness).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub save: Option<serde_json::Value>,
    /// RNG seed (`--seed`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// Which RNG parts were seeded (`/lab/seed` response for the game).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seeded_parts: Option<serde_json::Value>,
    /// Settings requested to be applied to the base world.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings_requested: Option<SettingsRequested>,
    /// The 14 independent values stored in the save (game captures).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub independents_in_save: Option<serde_json::Value>,
    /// Settings file used, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings_file: Option<String>,
}

/// Engine provenance (`manifest.engine`). Rust fills name/version/git/threads;
/// the game fills game/bridge/unity versions — each side skips unknown fields.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EngineInfo {
    /// `"RUSTibites"` or the game name.
    pub name: String,
    /// Rust crate version / game version.
    pub version: String,
    /// Git commit of the build (Rust).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_commit: Option<String>,
    /// Worker threads used (Rust).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threads: Option<u32>,
    /// Game version (game captures).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_version: Option<String>,
    /// Parity bridge version (game captures).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bridge_version: Option<String>,
    /// Unity version (game captures).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unity_version: Option<String>,
}

impl EngineInfo {
    /// Rust engine provenance block.
    pub fn rust(
        version: impl Into<String>,
        git_commit: Option<String>,
        threads: Option<u32>,
    ) -> Self {
        Self {
            name: "RUSTibites".to_string(),
            version: version.into(),
            git_commit,
            threads,
            game_version: None,
            bridge_version: None,
            unity_version: None,
        }
    }
}

/// `manifest.sim`: tick-rate/timing/progress block.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SimInfo {
    /// Simulation ticks per second.
    pub sim_tps: u32,
    /// Brain update factor.
    pub brain_tps: u32,
    /// Fixed timestep (s) = `1 / sim_tps`.
    pub dt: f64,
    /// Base-state simulation time (s).
    pub base_time: f64,
    /// Simulation time of the first captured tick (s).
    pub start_t: f64,
    /// Tick index of the first captured tick.
    pub start_tick: i64,
    /// Tick index of the last captured tick.
    pub end_tick: i64,
    /// Number of captured ticks.
    pub ticks: i64,
    /// Wall-clock duration of the run (s).
    pub wall_s: f64,
    /// Achieved simulation ticks per wall second.
    pub achieved_ticks_per_s: f64,
    /// Achieved speed multiplier (`achieved_ticks_per_s * dt`).
    pub achieved_speed: f64,
    /// Ticks finished past the nominal end (game pause overshoot).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overshoot_ticks: Vec<i64>,
    /// Segment durations for paused/segmented captures.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<serde_json::Value>,
}

/// `manifest.world`: initial world description.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldInfo {
    /// Simulation square size (world units).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub simulation_size: Option<f64>,
    /// Zone descriptions (game shape passes through).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zones: Vec<serde_json::Value>,
}

/// `manifest.plugin`: parity plugin availability (game captures).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PluginInfo {
    /// Whether the plugin was reachable.
    pub available: bool,
    /// Plugin version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// `Time.fixedDeltaTime` was pinned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dt_pinned: Option<bool>,
    /// The world was paused exactly at the saved state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused_at_load: Option<bool>,
    /// Independent settings were preloaded before the world started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preloaded_settings: Option<bool>,
}

/// One entry of `manifest.files`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FileInfo {
    /// File name inside the capture directory.
    pub name: String,
    /// SHA-256 of the file bytes.
    pub sha256: String,
    /// File size in bytes.
    pub bytes: u64,
    /// Number of JSONL records.
    pub records: u64,
}

/// `manifest.files`: written capture files and their integrity metadata.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FilesInfo {
    /// `ticks.jsonl`.
    pub ticks: FileInfo,
    /// `events.jsonl` (absent when no events were recorded).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events: Option<FileInfo>,
}

/// Capture manifest (`manifest.json`, `rustibites.parity.capture.v1`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// `rustibites.parity.capture.v1`.
    pub schema: String,
    /// `"capture"`.
    pub kind: String,
    /// `"rust"` (or `"game"`).
    pub source: String,
    /// Capture name.
    pub capture: String,
    /// Pass index (repeats of the same run).
    pub pass: u32,
    /// ISO-8601 UTC creation time.
    pub created_at: String,
    /// Engine provenance.
    pub engine: EngineInfo,
    /// Gene order used by `genes_digest`/`genes.mean/std` (game enum order).
    pub gene_names: Vec<String>,
    /// Run inputs/provenance.
    pub inputs: Inputs,
    /// Effective settings (same shape as the bridge `/settings` JSON).
    pub settings: serde_json::Value,
    /// [`settings_hash`] of `settings`.
    pub settings_hash: String,
    /// Timing and progress.
    pub sim: SimInfo,
    /// Initial world description.
    pub world: WorldInfo,
    /// Counts at the first captured tick.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counts_start: Option<Counts>,
    /// Counts at the last captured tick.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counts_end: Option<Counts>,
    /// Parity plugin state (game captures; Rust sets `available: false`).
    pub plugin: PluginInfo,
    /// Non-fatal drift/limitations; empty = clean capture.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub taint: Vec<String>,
    /// Files written by [`CaptureWriter::finish`] (absent before finishing).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<FilesInfo>,
}

impl Manifest {
    /// Creates a Rust manifest with the mandatory identity fields.
    pub fn new_rust(capture: impl Into<String>, pass: u32, version: impl Into<String>) -> Self {
        Self {
            schema: CAPTURE_SCHEMA.to_string(),
            kind: "capture".to_string(),
            source: "rust".to_string(),
            capture: capture.into(),
            pass,
            created_at: utc_now_iso8601(),
            engine: EngineInfo::rust(version, None, None),
            gene_names: GENE_NAMES.iter().map(|name| (*name).to_string()).collect(),
            inputs: Inputs::default(),
            settings: serde_json::Value::Object(serde_json::Map::new()),
            settings_hash: String::new(),
            sim: SimInfo::default(),
            world: WorldInfo::default(),
            counts_start: None,
            counts_end: None,
            plugin: PluginInfo {
                available: false,
                ..PluginInfo::default()
            },
            taint: Vec::new(),
            files: None,
        }
    }

    /// Sets `settings` and recomputes [`Manifest::settings_hash`].
    pub fn set_settings(&mut self, settings: serde_json::Value) {
        self.settings_hash = settings_hash(&settings);
        self.settings = settings;
    }

    /// Appends a non-fatal taint note (drift/limitation).
    pub fn add_taint(&mut self, note: impl Into<String>) {
        self.taint.push(note.into());
    }
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// Streaming writer for a plain (uncompressed) capture directory.
///
/// Files are written as compact JSONL (one object per line). The manifest is
/// written last by [`CaptureWriter::finish`], which also fills
/// `manifest.files` with per-file name/sha256/bytes/records.
pub struct CaptureWriter {
    dir: PathBuf,
    manifest: Manifest,
    ticks: BufWriter<File>,
    events: Option<BufWriter<File>>,
    ticks_path: PathBuf,
    events_path: PathBuf,
    tick_records: u64,
    event_records: u64,
    finished: bool,
}

impl CaptureWriter {
    /// Creates `dir` (recursively) and opens `ticks.jsonl` for writing.
    ///
    /// `events.jsonl` is created lazily on the first [`CaptureWriter::push_event`].
    pub fn new(dir: impl AsRef<Path>, manifest: Manifest) -> io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let ticks_path = dir.join("ticks.jsonl");
        let events_path = dir.join("events.jsonl");
        let ticks = BufWriter::new(File::create(&ticks_path)?);
        Ok(Self {
            dir,
            manifest,
            ticks,
            events: None,
            ticks_path,
            events_path,
            tick_records: 0,
            event_records: 0,
            finished: false,
        })
    }

    /// Capture directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Manifest accumulated so far (mutable for wiring metadata before
    /// [`CaptureWriter::finish`]).
    pub fn manifest_mut(&mut self) -> &mut Manifest {
        &mut self.manifest
    }

    /// Number of tick records written.
    pub fn tick_records(&self) -> u64 {
        self.tick_records
    }

    /// Number of event records written.
    pub fn event_records(&self) -> u64 {
        self.event_records
    }

    /// Appends one tick record as a compact JSON line.
    pub fn push_tick(&mut self, record: &TickRecord) -> io::Result<()> {
        write_jsonl(&mut self.ticks, record)?;
        self.tick_records += 1;
        Ok(())
    }

    /// Appends one birth/death event as a compact JSON line, creating
    /// `events.jsonl` on first use.
    pub fn push_event(&mut self, record: &EventRecord) -> io::Result<()> {
        if self.events.is_none() {
            self.events = Some(BufWriter::new(File::create(&self.events_path)?));
        }
        let events = self
            .events
            .as_mut()
            .expect("events writer created just above");
        write_jsonl(events, record)?;
        self.event_records += 1;
        Ok(())
    }

    /// Flushes both streams, removes an empty `events.jsonl`, computes file
    /// hashes/sizes/record counts into `manifest.files`, and writes
    /// `manifest.json`. Returns the manifest path.
    pub fn finish(mut self) -> io::Result<PathBuf> {
        self.ticks.flush()?;
        if let Some(events) = self.events.as_mut() {
            events.flush()?;
        }
        self.finished = true;
        let ticks_info = file_info(&self.ticks_path, self.tick_records)?;
        let events_info = if self.event_records > 0 {
            Some(file_info(&self.events_path, self.event_records)?)
        } else {
            // No events recorded: never leave an empty events.jsonl behind.
            if self.events_path.exists() {
                std::fs::remove_file(&self.events_path)?;
            }
            None
        };
        self.manifest.files = Some(FilesInfo {
            ticks: ticks_info,
            events: events_info,
        });

        // The manifest is written last, atomically via a temp file + rename.
        std::fs::create_dir_all(&self.dir)?;
        let manifest_path = self.dir.join("manifest.json");
        let temp_path = self.dir.join("manifest.json.tmp");
        let text = serde_json::to_string_pretty(&self.manifest)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        {
            let mut file = File::create(&temp_path)?;
            file.write_all(text.as_bytes())?;
            file.write_all(b"\n")?;
            file.flush()?;
        }
        std::fs::rename(&temp_path, &manifest_path)?;
        Ok(manifest_path)
    }
}

fn write_jsonl<T: Serialize>(writer: &mut BufWriter<File>, record: &T) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, record)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    writer.write_all(b"\n")
}

fn file_info(path: &Path, records: u64) -> io::Result<FileInfo> {
    let metadata = std::fs::metadata(path)?;
    let mut hasher = Sha256::new();
    let mut reader = BufReader::new(File::open(path)?);
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let mut sha256 = String::with_capacity(64);
    for byte in hasher.finalize() {
        use std::fmt::Write as _;
        let _ = write!(sha256, "{byte:02x}");
    }
    Ok(FileInfo {
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        sha256,
        bytes: metadata.len(),
        records,
    })
}

// ---------------------------------------------------------------------------
// ISO-8601 UTC timestamp helper (manifest.created_at)
// ---------------------------------------------------------------------------

/// Returns the current UTC time as `YYYY-MM-DDTHH:MM:SSZ`.
///
/// Capture metadata only; the simulation itself never reads the wall clock.
pub fn utc_now_iso8601() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 → (Y, M, D).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365; // [0, 399]
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100); // [0, 365]
    let mp = (5 * day_of_year + 2) / 153; // [0, 11]
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = year + i64::from(month <= 2);
    (year, month, day)
}

/// Widen an `f32` simulation value into the `f64` JSON number the capture
/// schema expects (exact: `f32` → `f64` conversion is lossless).
#[inline]
pub fn widen(value: f32) -> f64 {
    f64::from(value)
}

// ---------------------------------------------------------------------------
// World -> capture wiring (`run --capture DIR`)
// ---------------------------------------------------------------------------

/// Runs `world` for `ticks` ticks and writes a parity capture directory.
///
/// Emits the pre-tick snapshot as record `tick 0` (like the game capture:
/// `partial: true`, start counts/vitals/species/genes/brain + entities), then
/// one full record after each `world.tick()`, for `ticks + 1` records total
/// (`0..=ticks`). Entity snapshots are included at `snapshot_ticks` (tick 0 is
/// the pre-tick state; other values are post-tick states). Initial-population
/// `initial` events are emitted at tick 1, matching the game's event stream.
///
/// `base_path` is recorded in `manifest.inputs.base` (path + SHA-256 + base sim
/// time) and is re-read for the stable save `body.id`s used as entity ids.
///
/// Settings/settings_hash come from `bibites_io::to_bridge_settings_json`, so
/// the capture compares meta exactly with the game manifest.
///
/// # Errors
///
/// Returns a human-readable error for filesystem/JSON failures; simulation
/// errors (invalid world) must be handled by the caller before calling this.
/// One live bibite snapshot taken at the start of a tick, used to detect
/// deaths/births after the tick and to fill death-event fields.
#[derive(Clone)]
struct BibiteSnapshot {
    id: BibiteId,
    external: i64,
    species: Option<u32>,
    tag: Option<String>,
    generation: u32,
    p1: i64,
    p2: i64,
    x: f64,
    y: f64,
    age_s: f64,
    born_at: f64,
    energy: f64,
    max_energy: f64,
    health: f64,
    max_health: f64,
    damage: f64,
    attacked: u32,
    travel: f64,
    bites: u32,
    damage_dealt: f64,
    eggs_laid: u32,
    initial: bool,
}

/// Assigns a stable external capture id to a bibite born during the run.
fn external_id(id_map: &mut BTreeMap<BibiteId, i64>, next_fallback: &mut i64, id: BibiteId) -> i64 {
    *id_map.entry(id).or_insert_with(|| {
        *next_fallback -= 1;
        *next_fallback
    })
}

/// External ids of a bibite's parents (`0` when unknown, matching the game).
fn parent_ids(record: &BibiteRecord, id_map: &BTreeMap<BibiteId, i64>) -> (i64, i64) {
    let p1 = record
        .meta
        .parent1
        .and_then(|parent| id_map.get(&parent).copied())
        .unwrap_or(0);
    let p2 = record
        .meta
        .parent2
        .and_then(|parent| id_map.get(&parent).copied())
        .unwrap_or(0);
    (p1, p2)
}

/// Snapshots the current live (alive, non-corpse) population.
fn snapshot_live(
    world: &World,
    id_map: &mut BTreeMap<BibiteId, i64>,
    next_fallback: &mut i64,
    initial_ids: &BTreeSet<i64>,
) -> Vec<BibiteSnapshot> {
    let mut snapshots = Vec::new();
    for (id, record) in world.bibites.iter() {
        if record.body.dead || record.is_corpse {
            continue;
        }
        let external = external_id(id_map, next_fallback, id);
        let (p1, p2) = parent_ids(record, id_map);
        let age_s = f64::from(record.clock.time_alive);
        snapshots.push(BibiteSnapshot {
            id,
            external,
            species: record.meta.species.map(|species| species.value()),
            tag: record.meta.species_tag.clone(),
            generation: record.meta.generation,
            p1,
            p2,
            x: widen(record.pos.x),
            y: widen(record.pos.y),
            age_s,
            born_at: world.simulated_time - age_s,
            energy: widen(record.body.energy.amount),
            max_energy: widen(record.body.energy.max_amount),
            health: widen(record.body.health.amount),
            max_health: widen(record.body.health.max_amount),
            damage: widen(record.body.total_damage_suffered),
            attacked: record.body.times_attacked,
            travel: widen(record.propulsion.total_travel),
            bites: record.mouth.bibites_bitten,
            damage_dealt: widen(record.mouth.total_damage_dealt),
            eggs_laid: record.egg_layer.n_laid,
            initial: initial_ids.contains(&external),
        });
    }
    snapshots
}

/// Builds a `birth` event for a newly appeared bibite.
fn birth_event(
    tick: i64,
    t: f64,
    record: &BibiteRecord,
    external: i64,
    p1: i64,
    p2: i64,
) -> EventRecord {
    let mut event = EventRecord::new(tick, "birth");
    event.t = Some(t);
    event.id = Some(external);
    event.species = record.meta.species.map(|species| species.value());
    event.tag = record.meta.species_tag.clone();
    event.generation = Some(record.meta.generation);
    event.p1 = Some(p1);
    event.p2 = Some(p2);
    event.x = Some(widen(record.pos.x));
    event.y = Some(widen(record.pos.y));
    event.age_s = Some(0.0);
    event
}

/// Infers the game's death-cause string from the pre-death snapshot.
fn infer_death_cause(snapshot: &BibiteSnapshot) -> &'static str {
    if snapshot.energy <= 0.0 {
        "starved"
    } else if snapshot.attacked > 0 || snapshot.health < snapshot.max_health * 0.999 {
        "killed"
    } else {
        "other"
    }
}

/// The capture's cause string of a death the simulation recorded (`BibiteRecord::death_cause`): what the world did, not a guess from the last
/// snapshot (a bibite hurt earlier that starved a moment later is "starved"; the guess called it "killed" and over-counted kills: 53 of the
/// game's 1387 "starved" deaths on Branching Desert 2 have the guess's "killed" shape).
fn recorded_death_cause(cause: DeathCause) -> &'static str {
    match cause {
        DeathCause::Starvation => "starved",
        DeathCause::Bitten | DeathCause::Swallowed | DeathCause::Collision => "killed",
        DeathCause::Hazard | DeathCause::Removed => "other",
    }
}

/// Builds a `death` event from the last live snapshot of the bibite (`recorded`: the cause the simulation kept, when it still has the record).
fn death_event(tick: i64, t: f64, snapshot: &BibiteSnapshot, recorded: Option<DeathCause>) -> EventRecord {
    let mut event = EventRecord::new(tick, "death");
    event.t = Some(t);
    event.id = Some(snapshot.external);
    event.species = snapshot.species;
    event.tag = snapshot.tag.clone();
    event.generation = Some(snapshot.generation);
    event.p1 = Some(snapshot.p1);
    event.p2 = Some(snapshot.p2);
    event.x = Some(snapshot.x);
    event.y = Some(snapshot.y);
    event.age_s = Some(snapshot.age_s);
    event.born_at = Some(snapshot.born_at);
    event.initial = Some(snapshot.initial);
    event.energy = Some(snapshot.energy);
    event.max_energy = Some(snapshot.max_energy);
    event.health = Some(snapshot.health);
    event.max_health = Some(snapshot.max_health);
    event.damage = Some(snapshot.damage);
    event.attacked = Some(snapshot.attacked);
    event.travel = Some(snapshot.travel);
    event.bites = Some(snapshot.bites);
    event.damage_dealt = Some(snapshot.damage_dealt);
    event.eggs_laid = Some(snapshot.eggs_laid);
    event.cause = Some(recorded.map_or_else(|| infer_death_cause(snapshot), recorded_death_cause).to_string());
    event
}

/// One per-bibite state sample in the optional debug trace
/// (`trace.jsonl`, enabled with `RUSTIBITES_TRACE=1`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceEntity {
    /// External stable id (same mapping as feeding/events).
    pub id: i64,
    /// World x.
    pub x: f64,
    /// World y.
    pub y: f64,
    /// Rotation in degrees (matches game transform/ticks schema).
    pub rot_deg: f64,
    /// Velocity x.
    pub vel_x: f64,
    /// Velocity y.
    pub vel_y: f64,
    /// Angular velocity in degrees/second.
    pub ang_vel_deg: f64,
    /// Energy pool.
    pub energy: f64,
    /// Health pool.
    pub health: f64,
    /// Number of mouth targets.
    pub n_in_mouth: u8,
    /// Centre distance to the nearest plant pellet (-1 when none).
    pub nearest_plant_distance: f64,
    /// Radius of the nearest plant pellet (0 when none).
    pub nearest_plant_radius: f64,
    /// Brain `Rotate` output.
    pub rotate: f64,
    /// Brain `Accelerate` output.
    pub accelerate: f64,
    /// Solid contacts touching this bibite in the last physics step.
    pub contact_count: u16,
    /// Sum of |normal impulse| of those solid contacts.
    pub contact_impulse: f64,
    /// Solid contacts with another bibite.
    pub contact_bibite: u16,
    /// Solid contacts with a pellet.
    pub contact_pellet: u16,
    /// Solid contacts with an egg.
    pub contact_egg: u16,
    /// Solid contacts on Body/Tail/Mouth colliders this step.
    pub contact_collider: [u16; 3],
    /// New solid contacts this step (OnCollisionEnter2D analogue).
    pub contact_enter_count: u16,
    /// Sum of |normal impulse| of new solid contacts.
    pub contact_enter_impulse: f64,
}

/// One debug-trace tick (`rustibites.parity.trace.v1`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceSample {
    /// Schema id.
    pub schema: String,
    /// `"rust"`.
    pub source: String,
    /// Simulation tick.
    pub tick: u64,
    /// Simulated time.
    pub t: f64,
    /// Per-bibite samples in dense order.
    pub entities: Vec<TraceEntity>,
}

fn trace_sample(world: &World, id_map: &BTreeMap<BibiteId, i64>, tick: u64, t: f64) -> TraceSample {
    let mut entities = Vec::new();
    for (id, record) in world.bibites.iter() {
        let mut nearest_distance = f64::INFINITY;
        let mut nearest_radius = 0.0;
        for pellet_id in world.pellets.ids() {
            let Some(view) = world.pellets.get(pellet_id) else {
                continue;
            };
            if !matches!(view.material, bibites_core::pellet::MaterialKind::Plant) {
                continue;
            }
            let distance = f64::from(record.pos.distance(view.pos));
            if distance < nearest_distance {
                nearest_distance = distance;
                nearest_radius = f64::from(view.radius);
            }
        }
        entities.push(TraceEntity {
            id: id_map
                .get(&id)
                .copied()
                .unwrap_or(-(1_000_000_000 + entities.len() as i64)),
            x: widen(record.pos.x),
            y: widen(record.pos.y),
            rot_deg: widen(record.rot.to_degrees()),
            vel_x: widen(record.vel.x),
            vel_y: widen(record.vel.y),
            ang_vel_deg: widen(record.ang_vel.to_degrees()),
            energy: widen(record.body.energy.amount),
            health: widen(record.body.health.amount),
            n_in_mouth: record.n_mouth_targets,
            nearest_plant_distance: if nearest_distance.is_finite() {
                nearest_distance
            } else {
                -1.0
            },
            nearest_plant_radius: nearest_radius,
            rotate: widen(record.brain.output_value(Output::Rotate)),
            accelerate: widen(record.brain.output_value(Output::Accelerate)),
            contact_count: record.last_contact_count,
            contact_impulse: f64::from(record.last_contact_impulse),
            contact_bibite: record.last_contact_bibite,
            contact_pellet: record.last_contact_pellet,
            contact_egg: record.last_contact_egg,
            contact_collider: record.last_contact_collider,
            contact_enter_count: record.last_contact_enter_count,
            contact_enter_impulse: f64::from(record.last_contact_enter_impulse),
        });
    }
    TraceSample {
        schema: "rustibites.parity.trace.v1".to_string(),
        source: "rust".to_string(),
        tick,
        t,
        entities,
    }
}

pub fn write_world_capture(
    world: &mut World,
    dir: &Path,
    base_path: &Path,
    seed: u64,
    ticks: u64,
    snapshot_ticks: &[u64],
) -> Result<PathBuf, String> {
    let settings_json = bibites_io::to_bridge_settings_json(&world.settings);
    let hash = settings_hash(&settings_json);
    let start_time = world.simulated_time;
    let start_tick = world.clock.tick();
    let dt = f64::from(world.settings.fixed_dt());
    let sim_tps = world.settings.sim_tps;
    let brain_tps = world.settings.brain_tps;

    let capture_name = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "rust-capture".to_string());
    let mut manifest = Manifest::new_rust(capture_name, 0, env!("CARGO_PKG_VERSION"));
    if let Some(commit) = option_env!("RUSTIBITES_GIT_COMMIT") {
        manifest.engine.git_commit = Some(commit.to_string());
    }
    manifest.inputs.base = Some(BaseRef {
        path: base_path.display().to_string(),
        sha256: file_sha256(base_path).map_err(|error| {
            format!("could not hash base save {}: {error}", base_path.display())
        })?,
        sim_time: Some(start_time),
    });
    manifest.inputs.seed = Some(seed);
    manifest.set_settings(settings_json);
    manifest.sim = SimInfo {
        sim_tps,
        brain_tps,
        dt,
        base_time: start_time,
        start_t: start_time,
        start_tick: start_tick as i64,
        end_tick: start_tick as i64 + ticks as i64,
        ticks: ticks as i64,
        wall_s: 0.0,
        achieved_ticks_per_s: 0.0,
        achieved_speed: 0.0,
        overshoot_ticks: Vec::new(),
        segments: Vec::new(),
    };
    manifest.world = world_info(world);

    let (save_ids, save_parents) = load_save_identities(base_path);
    let mut id_map: BTreeMap<BibiteId, i64> = world
        .bibites
        .iter()
        .enumerate()
        .map(|(index, (id, _))| {
            let save_id = save_ids
                .get(index)
                .copied()
                .map(i64::from)
                .unwrap_or(-(1_000_000_000 + index as i64));
            (id, save_id)
        })
        .collect();
    let parent_map: BTreeMap<BibiteId, (i64, i64)> = world
        .bibites
        .iter()
        .enumerate()
        .map(|(index, (id, _))| {
            let (p1, p2) = save_parents
                .get(index)
                .copied()
                .map(|(a, b)| (i64::from(a.unwrap_or(0)), i64::from(b.unwrap_or(0))))
                .unwrap_or((0, 0));
            (id, (p1, p2))
        })
        .collect();
    // Fallback ids for bibites born during the run (stable only within a capture).
    let mut next_fallback = -(2_000_000_000_i64);
    let initial_ids: BTreeSet<i64> = id_map.values().copied().collect();
    let mut known_alive: BTreeSet<BibiteId> = world
        .bibites
        .iter()
        .filter(|(_, record)| !record.body.dead && !record.is_corpse)
        .map(|(id, _)| id)
        .collect();
    let mut dead_logged: BTreeSet<BibiteId> = BTreeSet::new();

    let mut writer = CaptureWriter::new(dir, manifest)
        .map_err(|error| format!("could not create capture: {error}"))?;

    let mut initial_events: Vec<EventRecord> = Vec::new();
    let start_counts: Counts;
    {
        let start_sample = world.sample_stats();
        start_counts = counts_from(&start_sample);
        let mut tick0 =
            tick_record_from_sample(&start_sample, &hash, start_time, sim_tps, brain_tps, true);
        tick0.partial = Some(true);
        if snapshot_ticks.contains(&0) || snapshot_ticks.is_empty() {
            tick0.entities = collect_entities(world, &id_map, &mut next_fallback);
        }
        // The game bridge logs one `initial` event per loaded bibite at the first
        // tick; tick/kind/id are what compare.py matches on, the rest is context.
        for (id, record) in world.bibites.iter() {
            if record.is_corpse || record.body.dead {
                continue;
            }
            let mut event = EventRecord::new(1, "initial");
            event.id = id_map.get(&id).copied();
            event.species = record.meta.species.map(|species| species.value());
            event.generation = Some(record.meta.generation);
            event.tag = Some(record.meta.species_tag.clone().unwrap_or_default());
            let (p1, p2) = parent_map.get(&id).copied().unwrap_or((0, 0));
            event.p1 = Some(p1);
            event.p2 = Some(p2);
            event.x = Some(widen(record.pos.x));
            event.y = Some(widen(record.pos.y));
            event.age_s = Some(f64::from(record.clock.time_alive));
            initial_events.push(event);
        }
        writer
            .push_tick(&tick0)
            .map_err(|error| format!("could not write capture tick 0: {error}"))?;
    }
    let mut end_counts = start_counts;

    let feeding_path = dir.join("feeding.jsonl");
    let mut feeding_writer = BufWriter::new(File::create(&feeding_path).map_err(|error| {
        format!(
            "could not create feeding capture {}: {error}",
            feeding_path.display()
        )
    })?);
    let mut trace_writer = if std::env::var_os("RUSTIBITES_TRACE").is_some() {
        let trace_path = dir.join("trace.jsonl");
        Some(BufWriter::new(File::create(&trace_path).map_err(
            |error| {
                format!(
                    "could not create trace capture {}: {error}",
                    trace_path.display()
                )
            },
        )?))
    } else {
        None
    };
    let no_feeding = std::env::var_os("RUSTIBITES_NO_FEEDING").is_some();
    let wall_start = Instant::now();
    for step in 1..=ticks {
        let prev_alive = snapshot_live(world, &mut id_map, &mut next_fallback, &initial_ids);
        world.tick();
        let sample = world.sample_stats();
        end_counts = counts_from(&sample);
        let event_tick = sample.tick as i64;
        let event_t = world.simulated_time;
        // Births: alive ids never seen before (hatched/spawned this tick).
        let mut newborn: Vec<(BibiteId, i64)> = Vec::new();
        for (id, record) in world.bibites.iter() {
            if record.body.dead || record.is_corpse || known_alive.contains(&id) {
                continue;
            }
            let external = external_id(&mut id_map, &mut next_fallback, id);
            newborn.push((id, external));
        }
        for (id, _) in &newborn {
            known_alive.insert(*id);
        }
        let mut lifecycle_events: Vec<EventRecord> = Vec::new();
        for (id, external) in &newborn {
            if let Some(record) = world.bibites.get(*id) {
                let (p1, p2) = parent_ids(record, &id_map);
                lifecycle_events.push(birth_event(event_tick, event_t, record, *external, p1, p2));
            }
        }
        // Deaths: previous live bibites that are now dead/corpse/removed.
        for snapshot in &prev_alive {
            let dead = match world.bibites.get(snapshot.id) {
                Some(record) => record.body.dead || record.is_corpse,
                None => true,
            };
            if dead && !dead_logged.contains(&snapshot.id) {
                dead_logged.insert(snapshot.id);
                let recorded = world.bibites.get(snapshot.id).and_then(|record| record.death_cause);
                lifecycle_events.push(death_event(event_tick, event_t, snapshot, recorded));
            }
        }
        let mut record = tick_record_from_sample(
            &sample,
            &hash,
            world.simulated_time,
            sim_tps,
            brain_tps,
            false,
        );
        if snapshot_ticks.contains(&sample.tick) {
            record.entities = collect_entities(world, &id_map, &mut next_fallback);
        }
        writer
            .push_tick(&record)
            .map_err(|error| format!("could not write capture tick {}: {error}", sample.tick))?;
        if step == 1 {
            for event in &mut initial_events {
                event.t = Some(world.simulated_time);
                writer
                    .push_event(event)
                    .map_err(|error| format!("could not write capture event: {error}"))?;
            }
        }
        // `RUSTIBITES_NO_FEEDING=1` skips the per-bibite feeding stream (it is gigabytes for a big world).
        if !no_feeding {
            let feeding = feeding_sample(&mut *world, &id_map, event_tick, event_t);
            write_jsonl(&mut feeding_writer, &feeding)
                .map_err(|error| format!("could not write feeding sample: {error}"))?;
        }
        if let Some(writer) = trace_writer.as_mut() {
            let trace = trace_sample(world, &id_map, sample.tick, event_t);
            write_jsonl(writer, &trace)
                .map_err(|error| format!("could not write trace sample: {error}"))?;
        }
        for event in &lifecycle_events {
            writer
                .push_event(event)
                .map_err(|error| format!("could not write capture event: {error}"))?;
        }
    }
    let wall_s = wall_start.elapsed().as_secs_f64();

    {
        let manifest = writer.manifest_mut();
        manifest.counts_start = Some(start_counts);
        manifest.counts_end = Some(end_counts);
        manifest.sim.wall_s = wall_s;
        manifest.sim.achieved_ticks_per_s = if wall_s > 0.0 {
            ticks as f64 / wall_s
        } else {
            0.0
        };
        manifest.sim.achieved_speed = manifest.sim.achieved_ticks_per_s * dt;
    }
    if let Some(mut writer) = trace_writer.take() {
        writer
            .flush()
            .map_err(|error| format!("could not flush trace capture: {error}"))?;
    }
    feeding_writer
        .flush()
        .map_err(|error| format!("could not flush feeding capture: {error}"))?;
    writer
        .finish()
        .map_err(|error| format!("could not finish capture: {error}"))
}

/// One tick record (aggregate groups) built from a [`bibites_core::sim::StatsSample`].
fn tick_record_from_sample(
    sample: &StatsSample,
    settings_hash: &str,
    t: f64,
    sim_tps: u32,
    brain_tps: u32,
    partial: bool,
) -> TickRecord {
    let mut record = TickRecord::rust(sample.tick as i64, t, sim_tps, brain_tps, settings_hash);
    if partial {
        record.partial = Some(true);
    }
    record.counts = Some(counts_from(sample));
    record.vitals = Some(Vitals {
        energy_mean: widen(sample.vitals.energy_mean),
        health_mean: widen(sample.vitals.health_mean),
        age_mean: widen(sample.vitals.age_mean),
        age_max: widen(sample.vitals.age_max),
    });
    record.species = Some(SpeciesStats {
        n: sample.species.n,
        counts: sample
            .species
            .counts
            .iter()
            .map(|(id, count)| (id.to_string(), *count))
            .collect(),
    });
    record.genes = Some(GenesStats {
        mean: std::array::from_fn(|index| widen(sample.genes.mean[index])),
        std: std::array::from_fn(|index| widen(sample.genes.std[index])),
    });
    record.brain = Some(BrainStats {
        nodes_mean: widen(sample.brain.nodes_mean),
        nodes_max: sample.brain.nodes_max,
        syn_mean: widen(sample.brain.syn_mean),
        syn_max: sample.brain.syn_max,
    });
    // The pre-tick (partial) record only carries the start-state groups, like
    // the game capture; everything else is filled on post-tick records.
    if record.partial.is_some() {
        return record;
    }
    record.pellets = Some(Pellets {
        plant_n: sample.pellets.plant_n,
        plant_biomass: widen(sample.pellets.plant_biomass),
        meat_n: sample.pellets.meat_n,
        meat_biomass: widen(sample.pellets.meat_biomass),
        free_biomass: widen(sample.pellets.free_biomass),
        bibite_biomass: widen(sample.pellets.bibite_biomass),
    });
    record.generation = Some(GenerationStats {
        gen_max: sample.generation.gen_max,
        gen_mean: widen(sample.generation.gen_mean),
    });
    record.lifecycle = Some(Lifecycle {
        births: sample.lifecycle.births as i64,
        deaths: sample.lifecycle.deaths as i64,
    });
    let mut outputs = BTreeMap::new();
    for output in ALL_OUTPUTS {
        outputs.insert(
            output.name().to_string(),
            widen(sample.behavior.outputs[output.index()]),
        );
    }
    record.behavior = Some(Behavior {
        speed_mean: widen(sample.behavior.speed_mean),
        speed_max: widen(sample.behavior.speed_max),
        moving_frac: widen(sample.behavior.moving_frac),
        travel_rate_mean: widen(sample.behavior.travel_rate_mean),
        bite_rate_mean: widen(sample.behavior.bite_rate_mean),
        damage_dealt_rate_mean: widen(sample.behavior.damage_dealt_rate_mean),
        egg_rate_mean: widen(sample.behavior.egg_rate_mean),
        outputs,
    });
    record.body = Some(BodyStats {
        size_mean: widen(sample.body.size_mean),
        area_mean: widen(sample.body.area_mean),
        mass_mean: widen(sample.body.mass_mean),
        max_energy_mean: widen(sample.body.max_energy_mean),
        max_health_mean: widen(sample.body.max_health_mean),
        metabolism_mean: widen(sample.body.metabolism_mean),
    });
    record.tags = Some(
        sample
            .tags
            .iter()
            .map(|tag| {
                (
                    tag.tag.clone(),
                    TagStats {
                        n: tag.n,
                        biomass: widen(tag.biomass),
                    },
                )
            })
            .collect(),
    );
    if !sample.metrics.is_empty() {
        record.metrics = Some(
            sample
                .metrics
                .iter()
                .map(|(label, value)| (label.clone(), widen(*value)))
                .collect(),
        );
    }
    record
}

fn counts_from(sample: &StatsSample) -> Counts {
    Counts {
        bibites: sample.counts.bibites,
        eggs: sample.counts.eggs,
        corpses: sample.counts.corpses,
    }
}

fn world_info(world: &World) -> WorldInfo {
    let zones = world
        .zones
        .zones
        .iter()
        .enumerate()
        .map(|(index, zone)| {
            let settings = &zone.config;
            serde_json::json!({
                "name": settings.name,
                "id": settings.zone_id.unwrap_or(index as u32),
                "material": settings.material.map(|material| material.name()).unwrap_or("None"),
                "distribution": settings.distribution.name(),
                "fertility": settings.fertility,
                "biomassDensity": settings.biomass_density,
                "pelletSize": settings.pellet_size,
                "movement": settings.movement.name(),
                "speed": settings.speed,
                "posX": settings.pos_x,
                "posY": settings.pos_y,
                "radiusIsRelative": settings.radius_is_relative,
                "width": settings.width_relative,
                "height": settings.height_relative,
                "worldX": zone.pos.x,
                "worldY": zone.pos.y,
            })
        })
        .collect();
    WorldInfo {
        simulation_size: Some(f64::from(world.bounds.sim_size())),
        zones,
    }
}

/// Save `body.id`s plus `(parent1, parent2)` ids, in save order.
type SaveIdentities = (Vec<i32>, Vec<(Option<i32>, Option<i32>)>);

/// Stable save `body.id`s and parent ids, in save order (empty when the input is
/// a scenario or cannot be re-read; entity ids then fall back to minus-indices).
fn load_save_identities(base_path: &Path) -> SaveIdentities {
    match bibites_io::bundle::load_save_bundle(base_path) {
        Ok(bundle) => (bundle.bibite_ids, bundle.bibite_parents),
        Err(_) => (Vec::new(), Vec::new()),
    }
}

/// Snapshots every living bibite and egg (dense store order) as entity records.
fn collect_entities(
    world: &World,
    id_map: &BTreeMap<BibiteId, i64>,
    next_fallback: &mut i64,
) -> Vec<ParityEntityRecord> {
    let mut entities = Vec::new();
    for (id, record) in world.bibites.iter() {
        if record.is_corpse || record.body.dead {
            continue;
        }
        let entity_id = match id_map.get(&id) {
            Some(entity_id) => *entity_id,
            None => {
                *next_fallback -= 1;
                *next_fallback
            }
        };
        entities.push(entity_from_bibite(record, entity_id));
    }
    for egg_id in world.eggs.ids() {
        if let Some(egg) = world.eggs.get(egg_id) {
            entities.push(entity_from_egg(egg));
        }
    }
    entities
}

/// Converts a core rotation (radians) to the capture schema's degrees
/// (`transform.rotation` in the game capture). Core/physics stay radians;
/// saves keep the io pass-through degrees.
fn capture_rotation(radians: f32) -> f64 {
    widen(radians.to_degrees())
}

/// Builds one optional `rustibites.parity.feeding.v1` sample from the live
/// world (engine snapshot; consumed by `tools/parity/feeding_ref.py`).
fn feeding_sample(
    world: &mut World,
    id_map: &BTreeMap<BibiteId, i64>,
    tick: i64,
    t: f64,
) -> FeedingSample {
    let mut bibites = Vec::new();
    let mut counters = BTreeMap::new();
    let mut live_seconds = 0.0f64;
    for (id, record) in world.bibites.iter() {
        if record.body.dead || record.is_corpse {
            continue;
        }
        let external = id_map.get(&id).copied().unwrap_or(0);
        let mut stomach_materials: BTreeMap<String, f64> = BTreeMap::new();
        for content in record.stomach.contents() {
            let key = match content.material {
                bibites_core::organs::StomachMaterial::Plant => "plant",
                bibites_core::organs::StomachMaterial::Meat => "meat",
            };
            *stomach_materials.entry(key.to_owned()).or_insert(0.0) += f64::from(content.amount);
        }
        bibites.push(FeedingBibite {
            id: external,
            energy: widen(record.body.energy.amount),
            health: widen(record.body.health.amount),
            d2_size: widen(record.body.geometry.d2_size),
            pos: FeedingPos {
                x: widen(record.pos.x),
                y: widen(record.pos.y),
            },
            stomach_total: widen(record.stomach.total_amount()),
            stomach_materials,
            n_held: record.n_held_targets as usize,
            held_pellets: Vec::new(),
            bite_progress: widen(record.mouth.bite_progress),
            bite_period: widen(record.mouth_derived.bite_period),
            bibites_bitten: record.mouth.bibites_bitten,
            damage_dealt: widen(record.mouth.total_damage_dealt),
            n_in_mouth: record.n_mouth_targets,
            mouth_contact_ticks: record.feeding.mouth_contact_ticks,
            plant_contact_ticks: record.feeding.plant_contact_ticks,
            mouth_contact_enter_events: record.feeding.mouth_contact_enter_events,
            energy_gained_e: Some(record.feeding.energy_gained_e),
            energy_spent_e: Some(record.feeding.energy_spent_e),
            stomach_digested_u2: Some(record.feeding.stomach_digested_u2),
            stomach_energy_gain_e: Some(record.feeding.stomach_energy_gain_e),
            live_seconds: Some(record.feeding.live_seconds),
            growth: Some(widen(record.growth.growth)),
            fat_amount: Some(widen(record.fat.amount)),
            plant_consumed_u2: Some(record.feeding.plant_consumed_u2),
            meat_consumed_u2: Some(record.feeding.meat_consumed_u2),
            eggs_laid: Some(record.egg_layer.n_laid),
            egg_energy_spent_estimate: Some(
                f64::from(record.egg_layer.n_laid) * widen(record.egg_layer.egg_energy),
            ),
            applied_growth: Some(widen(record.growth.applied_growth)),
            n_plant_targets: Some(
                record.mouth_targets[..record.n_mouth_targets as usize]
                    .iter()
                    .filter(|tag| pellet_id_from_tag(**tag).is_some())
                    .count() as u32,
            ),
            n_bibite_targets: Some(
                record.mouth_targets[..record.n_mouth_targets as usize]
                    .iter()
                    .filter(|tag| bibite_id_from_tag(**tag).is_some())
                    .count() as u32,
            ),
        });
        let feeding = record.feeding;
        for (name, value) in [
            ("plant_bite_events", feeding.plant_bite_events as f64),
            ("meat_bite_events", feeding.meat_bite_events as f64),
            ("bibite_bite_events", feeding.bibite_bite_events as f64),
            ("plant_swallow_events", feeding.plant_swallow_events as f64),
            ("meat_swallow_events", feeding.meat_swallow_events as f64),
            (
                "bibite_swallow_events",
                feeding.bibite_swallow_events as f64,
            ),
            ("plant_consumed_u2", feeding.plant_consumed_u2),
            ("meat_consumed_u2", feeding.meat_consumed_u2),
            ("bibite_consumed_u2", feeding.bibite_consumed_u2),
            ("plant_ripped_u2", feeding.plant_ripped_u2),
            ("meat_ripped_u2", feeding.meat_ripped_u2),
            ("bibite_damage_dealt", feeding.bibite_damage_dealt),
            ("bibite_kills", feeding.bibite_kills as f64),
            ("mouth_contact_ticks", feeding.mouth_contact_ticks as f64),
            ("plant_contact_ticks", feeding.plant_contact_ticks as f64),
            (
                "mouth_contact_enter_events",
                feeding.mouth_contact_enter_events as f64,
            ),
            ("stomach_digested_u2", feeding.stomach_digested_u2),
            ("stomach_energy_gain_e", feeding.stomach_energy_gain_e),
            ("energy_gained_e", feeding.energy_gained_e),
            ("energy_spent_e", feeding.energy_spent_e),
        ] {
            *counters.entry(name.to_owned()).or_insert(0.0) += value;
        }
        live_seconds += feeding.live_seconds;
    }
    counters.insert("live_seconds".to_owned(), live_seconds);
    let mut totals = FeedingTotals::default();
    for pellet_id in world.pellets.ids() {
        if let Some(view) = world.pellets.get(pellet_id) {
            totals.pellets_n += 1;
            match view.material {
                bibites_core::pellet::MaterialKind::Plant => {
                    totals.plant_amount += f64::from(view.amount)
                }
                bibites_core::pellet::MaterialKind::Meat => {
                    totals.meat_amount += f64::from(view.amount)
                }
                _ => {}
            }
        }
    }
    let mut material_properties = BTreeMap::new();
    for (name, params) in [
        ("plant", &world.settings.materials.plant),
        ("meat", &world.settings.materials.meat),
    ] {
        material_properties.insert(
            name.to_owned(),
            FeedingMaterial {
                energy_density: f64::from(params.energy_density),
                mass_density: f64::from(params.mass_density),
            },
        );
    }
    let snapshot = world.conservation_snapshot();
    let totals_c = snapshot.totals;
    let conservation = Some(FeedingConservation {
        seeded_plant_energy: totals_c.seeded_plant_energy,
        seeded_meat_energy: totals_c.seeded_meat_energy,
        produced_plant_energy: totals_c.produced_plant_energy,
        produced_meat_energy: totals_c.produced_meat_energy,
        recycled_plant_energy: totals_c.recycled_plant_energy,
        recycled_meat_energy: totals_c.recycled_meat_energy,
        decayed_plant_energy: totals_c.decayed_plant_energy,
        decayed_meat_energy: totals_c.decayed_meat_energy,
        culled_plant_energy: totals_c.culled_plant_energy,
        culled_meat_energy: totals_c.culled_meat_energy,
        removed_plant_energy: totals_c.removed_plant_energy,
        removed_meat_energy: totals_c.removed_meat_energy,
        consumed_plant_u2: totals_c.consumed_plant_u2,
        consumed_meat_u2: totals_c.consumed_meat_u2,
        consumed_plant_energy: totals_c.consumed_plant_energy,
        consumed_meat_energy: totals_c.consumed_meat_energy,
        stock_plant_energy: snapshot.stock_plant_energy,
        stock_meat_energy: snapshot.stock_meat_energy,
        terms_missing: snapshot.terms_missing,
    });
    FeedingSample {
        schema: "rustibites.parity.feeding.v1".to_string(),
        tick,
        t,
        bibites,
        totals,
        counters,
        material_properties,
        conservation,
    }
}

fn entity_from_bibite(record: &bibites_core::body::BibiteRecord, id: i64) -> ParityEntityRecord {
    let mut entity = ParityEntityRecord::new("bibite");
    entity.id = Some(id);
    entity.species = record.meta.species.map(|species| species.value());
    entity.generation = record.meta.generation;
    entity.tag = record.meta.species_tag.clone().unwrap_or_default();
    entity.x = widen(record.pos.x);
    entity.y = widen(record.pos.y);
    entity.vx = widen(record.vel.x);
    entity.vy = widen(record.vel.y);
    entity.rot = capture_rotation(record.rot);
    entity.energy = widen(record.body.energy.amount);
    entity.max_energy = Some(widen(record.body.geometry.max_energy));
    entity.health = Some(widen(record.body.health.amount));
    entity.max_health = Some(widen(record.body.geometry.max_health));
    entity.age_s = Some(widen(record.clock.time_alive));
    entity.size = Some(widen(record.body.geometry.d1_size));
    entity.set_brain(&record.brain);
    entity.set_genes(&record.genome, true);
    entity
}

fn entity_from_egg(egg: &bibites_core::body::EggRecord) -> ParityEntityRecord {
    let mut entity = ParityEntityRecord::new("egg");
    entity.species = egg.meta.species.map(|species| species.value());
    entity.generation = egg.meta.generation;
    entity.tag = egg.meta.species_tag.clone().unwrap_or_default();
    entity.x = widen(egg.pos.x);
    entity.y = widen(egg.pos.y);
    entity.vx = widen(egg.vel.x);
    entity.vy = widen(egg.vel.y);
    entity.rot = capture_rotation(egg.rot);
    entity.energy = widen(egg.hatch.energy);
    entity.set_brain(&egg.brain);
    entity.set_genes(&egg.genome, true);
    entity
}

fn file_sha256(path: &Path) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut reader = BufReader::new(File::open(path)?);
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_death_event_carries_the_cause_the_world_recorded_not_a_guess() {
        // Hurt earlier, then starved: the last snapshot looks "killed" (health below the maximum, bitten before) but the world says starvation.
        let snapshot = BibiteSnapshot {
            id: BibiteId::from_handle(bibites_core::arena::Handle::new(0, 0)),
            external: 1,
            species: None,
            tag: None,
            generation: 1,
            p1: 0,
            p2: 0,
            x: 0.0,
            y: 0.0,
            age_s: 10.0,
            born_at: 0.0,
            energy: 0.037,
            max_energy: 100.0,
            health: 0.1,
            max_health: 200.0,
            damage: 50.0,
            attacked: 2,
            travel: 0.0,
            bites: 0,
            damage_dealt: 0.0,
            eggs_laid: 0,
            initial: false,
        };
        assert_eq!(death_event(1, 1.0, &snapshot, None).cause.as_deref(), Some("killed"), "the old guess, kept for a record that is gone");
        for (cause, name) in [
            (DeathCause::Starvation, "starved"),
            (DeathCause::Bitten, "killed"),
            (DeathCause::Swallowed, "killed"),
            (DeathCause::Collision, "killed"),
            (DeathCause::Hazard, "other"),
            (DeathCause::Removed, "other"),
        ] {
            assert_eq!(death_event(1, 1.0, &snapshot, Some(cause)).cause.as_deref(), Some(name));
        }
    }
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "rustibites_capture_{tag}_{}_{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn sample_tick(tick: i64) -> TickRecord {
        let mut record = TickRecord::rust(tick, tick as f64 / 30.0, 30, 2, "0123456789abcdef");
        record.counts = Some(Counts {
            bibites: 3,
            eggs: 1,
            corpses: 0,
        });
        record.pellets = Some(Pellets {
            plant_n: 10,
            plant_biomass: 1.5,
            meat_n: 2,
            meat_biomass: 0.25,
            free_biomass: 0.0,
            bibite_biomass: 12.5,
        });
        record
    }

    // ---- settings_hash vectors (generated once with Python parity_common) ----

    #[test]
    fn settings_hash_empty_scopes_matches_python() {
        let empty = serde_json::json!({ "scenario": {}, "independent": {} });
        assert_eq!(
            settings_hash(&empty),
            "1be941896852b9e5bd2502425daf17f2380b38a9725b52208f0cff0c5528ae86"
        );
        // Missing scopes are identical to empty scopes (`or {}` in Python).
        assert_eq!(settings_hash(&serde_json::json!({})), settings_hash(&empty));
    }

    #[test]
    fn settings_hash_bool_int_float_matches_python() {
        let settings = serde_json::json!({
            "scenario": { "aBool": true, "zInt": -5, "mFloat": 1.5 },
            "independent": {}
        });
        assert_eq!(
            settings_hash(&settings),
            "fd7e000395c7041eb7b3b29b2fe6c9741280a6d75089c89598937617d9001d56"
        );
        let settings = serde_json::json!({
            "scenario": { "simTPS": 30 },
            "independent": { "x": false, "ratio": -0.25 }
        });
        assert_eq!(
            settings_hash(&settings),
            "24b49632e1818db029088d0937ce5679d97819ce5d3a27d31e39b419f039ffd4"
        );
    }

    #[test]
    fn settings_hash_documents_byte_layout() {
        // Manual reconstruction of the pinned empty-scope layout:
        //   b"scenario{" + b"}" + b"independent{" + b"}"
        let mut hasher = Sha256::new();
        hasher.update(b"scenario{");
        hasher.update(b"}");
        hasher.update(b"independent{");
        hasher.update(b"}");
        let mut hex = String::new();
        for byte in hasher.finalize() {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
        }
        assert_eq!(
            hex,
            "1be941896852b9e5bd2502425daf17f2380b38a9725b52208f0cff0c5528ae86"
        );
    }

    // ---- record_digest vector (generated once with Python parity_common) -----

    fn python_vector_record() -> TickRecord {
        let mut record = TickRecord::rust(12, 0.4, 30, 2, "0123456789abcdef");
        record.partial = Some(false);
        record.counts = Some(Counts {
            bibites: 3,
            eggs: 1,
            corpses: 0,
        });
        record.pellets = Some(Pellets {
            plant_n: 10,
            plant_biomass: 1.5,
            meat_n: 2,
            meat_biomass: 0.25,
            free_biomass: 0.0,
            bibite_biomass: 12.5,
        });
        record.vitals = Some(Vitals {
            energy_mean: 3.5,
            health_mean: 4.0,
            age_mean: 5.25,
            age_max: 9.0,
        });
        record.generation = Some(GenerationStats {
            gen_max: 7,
            gen_mean: 3.5,
        });
        let mut species_counts = BTreeMap::new();
        species_counts.insert("1".to_string(), 2);
        species_counts.insert("3".to_string(), 1);
        record.species = Some(SpeciesStats {
            n: 2,
            counts: species_counts,
        });
        record.brain = Some(BrainStats {
            nodes_mean: 48.0,
            nodes_max: 48,
            syn_mean: 3.0,
            syn_max: 3,
        });
        record.genes = Some(GenesStats {
            mean: [0.1; 35],
            std: [0.01; 35],
        });
        record.lifecycle = Some(Lifecycle {
            births: 4,
            deaths: 2,
        });
        let mut outputs = BTreeMap::new();
        outputs.insert("Accelerate".to_string(), 0.5);
        outputs.insert("Rotate".to_string(), -0.1);
        record.behavior = Some(Behavior {
            speed_mean: 0.5,
            speed_max: 1.0,
            moving_frac: 0.75,
            travel_rate_mean: 2.0,
            bite_rate_mean: 0.0,
            damage_dealt_rate_mean: 0.0,
            egg_rate_mean: 0.1,
            outputs,
        });
        record.body = Some(BodyStats {
            size_mean: 2.0,
            area_mean: 4.0,
            mass_mean: 0.5,
            max_energy_mean: 6.0,
            max_health_mean: 5.0,
            metabolism_mean: 0.001,
        });
        let mut tags = BTreeMap::new();
        tags.insert("tagA".to_string(), TagStats { n: 2, biomass: 3.5 });
        record.tags = Some(tags);
        let mut metrics = BTreeMap::new();
        metrics.insert("pop".to_string(), 3.0);
        record.metrics = Some(metrics);
        record
    }

    #[test]
    fn record_digest_matches_python_vector() {
        let record = python_vector_record();
        assert_eq!(
            record_digest(&record).expect("record serializes"),
            "9bee71be6d26f38b"
        );
    }

    #[test]
    fn record_digest_ignores_entities() {
        let plain = python_vector_record();
        let mut with_entities = plain.clone();
        let mut entity = ParityEntityRecord::new("bibite");
        entity.set_brain(&Brain::starter());
        with_entities.entities.push(entity);
        assert_eq!(
            record_digest(&plain).expect("serialize"),
            record_digest(&with_entities).expect("serialize")
        );
        // Sanity: entity content is present in the serialized record.
        let json = serde_json::to_value(&with_entities).expect("serialize");
        assert_eq!(json["entities"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn record_digest_changes_with_aggregates() {
        let plain = python_vector_record();
        let mut changed = plain.clone();
        changed.counts.as_mut().expect("counts").bibites = 4;
        assert_ne!(
            record_digest(&plain).expect("serialize"),
            record_digest(&changed).expect("serialize")
        );
    }

    // ---- schema shape snapshots ----

    #[test]
    fn tick_record_key_order_and_optional_omission() {
        let record = TickRecord::rust(0, 0.0, 30, 2, "hash");
        let text = serde_json::to_string(&record).expect("serialize");
        assert_eq!(
            text,
            r#"{"schema":"rustibites.parity.tick.v1","source":"rust","tick":0,"t":0.0,"sim_tps":30,"brain_tps":2,"settings_hash":"hash"}"#
        );
        let value = serde_json::to_value(&record).expect("serialize");
        let object = value.as_object().expect("object");
        let keys: Vec<&String> = object.keys().collect();
        assert_eq!(
            keys,
            [
                "schema",
                "source",
                "tick",
                "t",
                "sim_tps",
                "brain_tps",
                "settings_hash"
            ]
        );
        assert!(!object.contains_key("entities"));
        assert!(!object.contains_key("partial"));
    }

    #[test]
    fn entity_record_serializes_schema_fields() {
        let mut entity = ParityEntityRecord::new("bibite");
        entity.id = Some(7);
        entity.species = Some(3);
        entity.generation = 12;
        entity.tag = "tag".to_string();
        entity.set_brain(&Brain::starter());
        entity.set_genes(&Genome::starter(), false);
        let value = serde_json::to_value(&entity).expect("serialize");
        assert_eq!(value["kind"], "bibite");
        assert_eq!(value["id"], 7);
        assert_eq!(value["species"], 3);
        assert_eq!(value["nodes"], 48);
        assert_eq!(value["synapses"], 3);
        assert_eq!(value["genes_digest"], genes_digest(&Genome::starter()));
        assert!(
            value.get("genes").is_none(),
            "genes omitted unless embedded"
        );
        assert_eq!(value["sense_digest"].as_str().map(str::len), Some(16));

        entity.set_genes(&Genome::starter(), true);
        let value = serde_json::to_value(&entity).expect("serialize");
        assert_eq!(value["genes"].as_array().map(Vec::len), Some(35));
    }

    #[test]
    fn manifest_serializes_rust_shape() {
        let mut manifest = Manifest::new_rust("smoke", 0, "0.1.0");
        manifest.set_settings(serde_json::json!({
            "scenario": { "simTPS": 30 },
            "independent": {}
        }));
        let value = serde_json::to_value(&manifest).expect("serialize");
        let object = value.as_object().expect("object");
        for key in [
            "schema",
            "kind",
            "source",
            "capture",
            "pass",
            "created_at",
            "engine",
            "gene_names",
            "inputs",
            "settings",
            "settings_hash",
            "sim",
            "world",
            "plugin",
        ] {
            assert!(object.contains_key(key), "missing manifest key {key}");
        }
        assert_eq!(value["schema"], CAPTURE_SCHEMA);
        assert_eq!(value["kind"], "capture");
        assert_eq!(value["source"], "rust");
        assert_eq!(value["engine"]["name"], "RUSTibites");
        assert_eq!(value["plugin"]["available"], false);
        assert!(value["plugin"].get("version").is_none());
        assert_eq!(
            value["gene_names"].as_array().map(Vec::len),
            Some(GENE_NAMES.len())
        );
        assert!(
            object.get("files").is_none(),
            "files written only by finish()"
        );
        assert!(object.get("taint").is_none(), "empty taint omitted");
    }

    #[test]
    fn utc_timestamp_has_expected_shape() {
        let stamp = utc_now_iso8601();
        assert_eq!(stamp.len(), 20, "{stamp}");
        assert!(stamp.ends_with('Z'));
        assert_eq!(&stamp[4..5], "-");
        assert_eq!(&stamp[7..8], "-");
        assert_eq!(&stamp[10..11], "T");
        assert_eq!(&stamp[13..14], ":");
        assert_eq!(&stamp[16..17], ":");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_000), (2022, 1, 8));
    }

    // ---- writer round-trip ----

    #[test]
    fn writer_round_trip_with_events() {
        let dir = temp_dir("writer_events");
        let mut manifest = Manifest::new_rust("smoke", 0, "0.1.0");
        manifest.set_settings(serde_json::json!({ "scenario": {}, "independent": {} }));
        let mut writer = CaptureWriter::new(&dir, manifest).expect("writer");
        writer.push_tick(&sample_tick(0)).expect("tick 0");
        writer.push_tick(&sample_tick(1)).expect("tick 1");
        let mut birth = EventRecord::new(1, "birth");
        birth.id = Some(42);
        writer.push_event(&birth).expect("event");
        assert_eq!(writer.tick_records(), 2);
        assert_eq!(writer.event_records(), 1);
        let manifest_path = writer.finish().expect("finish");
        assert_eq!(manifest_path, dir.join("manifest.json"));

        let manifest_text = std::fs::read_to_string(&manifest_path).expect("manifest");
        let manifest_json: serde_json::Value =
            serde_json::from_str(&manifest_text).expect("manifest json");
        assert_eq!(manifest_json["files"]["ticks"]["name"], "ticks.jsonl");
        assert_eq!(manifest_json["files"]["ticks"]["records"], 2);
        assert_eq!(manifest_json["files"]["events"]["name"], "events.jsonl");
        assert_eq!(manifest_json["files"]["events"]["records"], 1);

        let ticks_text = std::fs::read_to_string(dir.join("ticks.jsonl")).expect("ticks");
        let lines: Vec<&str> = ticks_text.lines().collect();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value = serde_json::from_str(lines[0]).expect("tick json");
        assert_eq!(first["tick"], 0);
        assert_eq!(first["source"], "rust");
        let second: serde_json::Value = serde_json::from_str(lines[1]).expect("tick json");
        assert_eq!(second["tick"], 1);

        // Manifest hash/bytes must match the actual files.
        for (entry, file) in [
            (&manifest_json["files"]["ticks"], "ticks.jsonl"),
            (&manifest_json["files"]["events"], "events.jsonl"),
        ] {
            let bytes = std::fs::read(dir.join(file)).expect("read capture file");
            assert_eq!(entry["bytes"].as_u64(), Some(bytes.len() as u64));
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let mut sha = String::new();
            for byte in hasher.finalize() {
                use std::fmt::Write as _;
                let _ = write!(sha, "{byte:02x}");
            }
            assert_eq!(entry["sha256"].as_str(), Some(sha.as_str()));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writer_sample_tick_digests_match_python() {
        // Hashes produced by parity_common.record_digest for the exact records
        // written by `writer_round_trip_with_events` (generated once).
        assert_eq!(
            record_digest(&sample_tick(0)).expect("digest"),
            "c8b381e0df59dd28"
        );
        assert_eq!(
            record_digest(&sample_tick(1)).expect("digest"),
            "7dd800b8c0ca9f1f"
        );
    }

    #[test]
    fn writer_without_events_omits_events_file() {
        let dir = temp_dir("writer_no_events");
        let manifest = Manifest::new_rust("no_events", 0, "0.1.0");
        let mut writer = CaptureWriter::new(&dir, manifest).expect("writer");
        writer.push_tick(&sample_tick(0)).expect("tick");
        writer.finish().expect("finish");
        assert!(!dir.join("events.jsonl").exists());
        let manifest_json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("manifest.json")).expect("manifest"),
        )
        .expect("manifest json");
        assert!(manifest_json["files"].get("events").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_mut_can_be_updated_before_finish() {
        let dir = temp_dir("writer_manifest_mut");
        let manifest = Manifest::new_rust("mutable", 0, "0.1.0");
        let mut writer = CaptureWriter::new(&dir, manifest).expect("writer");
        writer.push_tick(&sample_tick(0)).expect("tick");
        {
            let manifest = writer.manifest_mut();
            manifest.sim.sim_tps = 30;
            manifest.sim.brain_tps = 2;
            manifest.sim.dt = 1.0 / 30.0;
            manifest.sim.ticks = 1;
            manifest.counts_start = Some(Counts::default());
            manifest.add_taint("unit-test");
        }
        writer.finish().expect("finish");
        let manifest_json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("manifest.json")).expect("manifest"),
        )
        .expect("manifest json");
        assert_eq!(manifest_json["taint"][0], "unit-test");
        assert_eq!(manifest_json["sim"]["ticks"], 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn capture_rotation_is_degrees() {
        let degrees = capture_rotation(std::f32::consts::FRAC_PI_2);
        assert!((degrees - 90.0).abs() < 1e-4, "expected 90, got {degrees}");
        assert!((capture_rotation(-std::f32::consts::PI) + 180.0).abs() < 1e-4);
        assert_eq!(capture_rotation(0.0), 0.0);
    }

    #[test]
    fn widen_is_lossless() {
        for value in [0.0_f32, 0.1, -2.5, 1024.0, f32::MIN_POSITIVE] {
            assert_eq!(widen(value), f64::from(value));
        }
    }

    // ---- Canonical Default-scenario validation (docs/TESTING_POLICY.md) ----

    fn default_paths() -> Option<(PathBuf, PathBuf)> {
        let root = workspace_root();
        let base = root.join("testdata/real/Default.zip");
        let template = root.join("testdata/real/templates/5_Basic_bibite_0.6.bb8template");
        if base.is_file() && template.is_file() {
            Some((base, template))
        } else {
            eprintln!(
                "skipping Default capture test: {} / {} missing",
                base.display(),
                template.display()
            );
            None
        }
    }

    fn default_world(base: &Path, template_path: &Path) -> World {
        let bundle = bibites_io::load_scenario_bundle(base).expect("load Default scenario");
        let mut world =
            World::new(bundle.settings, bundle.scenario, 12_345).expect("build Default world");
        let _ = world.seed_scenario_world();
        let loaded = bibites_io::load_template(template_path).expect("load Basic template");
        let name = loaded
            .template
            .name
            .clone()
            .unwrap_or_else(|| "Basic bibite".to_string());
        world.set_templates(vec![bibites_core::world::TemplateSource::new(
            name,
            loaded.genome,
            loaded.meta,
            loaded.brain,
        )]);
        let _ = world.start_fresh_scenario_spawner();
        world
    }

    #[test]
    fn default_capture_lifecycle_and_feeding_stream_600() {
        let Some((base, template_path)) = default_paths() else {
            return;
        };
        let mut world = default_world(&base, &template_path);
        let dir = temp_dir("default_capture_600");
        write_world_capture(&mut world, &dir, &base, 12_345, 600, &[0, 600])
            .expect("write Default capture");
        let ticks: Vec<serde_json::Value> = std::fs::read_to_string(dir.join("ticks.jsonl"))
            .expect("ticks")
            .lines()
            .map(|line| serde_json::from_str(line).expect("tick json"))
            .collect();
        let last = ticks.last().expect("last tick");
        assert_eq!(last["counts"]["bibites"], 10);
        assert_eq!(last["counts"]["corpses"], 0);
        // Faithful Body/Mouth 90° part transforms slightly change the
        // scenario-start feeding timing (new collider geometry): 447 plants.
        // Virgin spawns now run the row's `randomizeGenes` mutation pass
        // (`WorldObjectsSpawner.cs:152-175`), which shifts the RNG stream and the
        // starting genes and randomised newborn sense phases: 448 plants, energy mean 108.49 (was 447 / 115.41).
        assert_eq!(last["pellets"]["plant_n"], 448);
        let energy = last["vitals"]["energy_mean"].as_f64().expect("energy");
        // Faithful Body/Mouth 90° prefab transforms retime scenario-start
        // intake; new Rust baseline 115.41 (old vertical-collider baseline
        // was 114.71). Parity task-133 confirms the transform is authoritative.
        assert!(
            (energy - 108.49).abs() < 0.5,
            "Default 600-tick energy mean {energy} differs from updated baseline 108.49"
        );
        let events: Vec<serde_json::Value> = std::fs::read_to_string(dir.join("events.jsonl"))
            .expect("events")
            .lines()
            .map(|line| serde_json::from_str(line).expect("event json"))
            .collect();
        assert_eq!(
            events
                .iter()
                .filter(|event| event["kind"] == "initial")
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event["kind"] == "birth")
                .count(),
            9
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event["kind"] == "death")
                .count(),
            0
        );
        // Feeding stream is the primary evidence; Default has low but nonzero
        // intake once the spawner population starts grazing.
        let feeding: Vec<serde_json::Value> = std::fs::read_to_string(dir.join("feeding.jsonl"))
            .expect("feeding stream")
            .lines()
            .map(|line| serde_json::from_str(line).expect("feeding json"))
            .collect();
        assert_eq!(feeding.len(), 600);
        assert_eq!(feeding[0]["schema"], "rustibites.parity.feeding.v1");
        let counters = feeding.last().expect("last feeding")["counters"]
            .as_object()
            .expect("counters");
        assert!(counters["live_seconds"].as_f64().unwrap_or(0.0) > 0.0);
        assert!(counters["plant_bite_events"].as_f64().unwrap_or(0.0) > 0.0);
        assert!(counters["plant_consumed_u2"].as_f64().unwrap_or(0.0) > 0.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Historical real-fixture regressions (gitignored testdata; skipped
    // when absent). The canonical validation world is `testdata/real/Default.zip`
    // (docs/TESTING_POLICY.md); Ring/lol are kept only as documented regression
    // fixtures (evolved-brain / specific-save behaviour), not as primary baselines.

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .to_path_buf()
    }

    fn ring_paths() -> Option<(PathBuf, PathBuf)> {
        let root = workspace_root();
        let base = root.join("testdata/parity/bases/ring_parity.zip");
        let game_manifest =
            root.join("testdata/parity/captures/ring_seed12345/pass-0/manifest.json");
        if base.is_file() && game_manifest.is_file() {
            Some((base, game_manifest))
        } else {
            eprintln!(
                "skipping Ring parity test: {} / {} missing",
                base.display(),
                game_manifest.display()
            );
            None
        }
    }

    fn ring_world(base: &Path) -> World {
        let mut contents = bibites_io::load_world_contents(base, None).expect("load ring save");
        contents.seed = 12_345;
        World::from_contents(contents).expect("build ring world")
    }

    #[test]
    fn ring_settings_hash_matches_game_capture() {
        let Some((base, game_manifest)) = ring_paths() else {
            return;
        };
        let world = ring_world(&base);
        let settings = bibites_io::to_bridge_settings_json(&world.settings);
        let hash = settings_hash(&settings);
        let game: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&game_manifest).expect("game manifest"))
                .expect("game manifest json");
        let game_hash = game["settings_hash"].as_str().unwrap_or_default();
        assert_eq!(
            hash,
            game_hash,
            "settings JSON differs from the game capture; rust={} game={}",
            serde_json::to_string(&settings).unwrap_or_default(),
            serde_json::to_string(&game["settings"]).unwrap_or_default()
        );
    }

    #[test]
    fn ring_capture_smoke() {
        let Some((base, game_manifest)) = ring_paths() else {
            return;
        };
        let dir = workspace_root().join("testdata/parity/rust_ring");
        let _ = std::fs::remove_dir_all(&dir);
        let mut world = ring_world(&base);
        let manifest_path = write_world_capture(&mut world, &dir, &base, 12_345, 300, &[0, 300])
            .expect("write ring capture");
        assert!(manifest_path.is_file());

        let ticks_text = std::fs::read_to_string(dir.join("ticks.jsonl")).expect("ticks jsonl");
        let lines: Vec<&str> = ticks_text.lines().collect();
        assert_eq!(lines.len(), 301, "expected tick 0..=300");
        let first: serde_json::Value = serde_json::from_str(lines[0]).expect("tick 0");
        assert_eq!(first["tick"], 0);
        assert_eq!(first["partial"], true);
        let last: serde_json::Value = serde_json::from_str(lines[300]).expect("tick 300");
        assert_eq!(last["tick"], 300);
        assert_eq!(last["counts"]["bibites"], 1);

        let manifest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("manifest.json")).expect("manifest"),
        )
        .expect("manifest json");
        let game: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&game_manifest).expect("game manifest"))
                .expect("game manifest json");
        assert_eq!(manifest["schema"], CAPTURE_SCHEMA);
        assert_eq!(manifest["source"], "rust");
        assert_eq!(manifest["sim"]["sim_tps"], game["sim"]["sim_tps"]);
        assert_eq!(manifest["sim"]["brain_tps"], game["sim"]["brain_tps"]);
        assert_eq!(
            manifest["settings_hash"], game["settings_hash"],
            "settings hash must match the game capture"
        );
        assert_eq!(manifest["gene_names"].as_array().map(Vec::len), Some(35));
        assert_eq!(manifest["files"]["ticks"]["records"], 301);
        // Entity snapshots at both requested ticks.
        let start: serde_json::Value = serde_json::from_str(lines[0]).expect("tick 0");
        let end: serde_json::Value = serde_json::from_str(lines[300]).expect("tick 300");
        assert_eq!(start["entities"].as_array().map(Vec::len), Some(1));
        assert_eq!(end["entities"].as_array().map(Vec::len), Some(1));
        assert_eq!(end["entities"][0]["kind"], "bibite");
        let end_id = end["entities"][0]["id"].as_i64().unwrap_or_default();
        if end_id != -124_909_645 {
            // Known io gap: `bibite_ids` is 0 because `parse_bibite` reads the
            // body id from `Extra` instead of the typed `Bb8Body.id` field.
            eprintln!("warning: entity id {end_id} != game -124909645 (bibites-io body.id gap)");
        }
        println!("wrote capture: {}", manifest_path.display());
    }

    /// Historical real-save regression (lol, evolved brain): independent
    /// determinism check for world-dev's thread-parallel sensing: capture the
    /// same real save with 1 vs 8 threads and require the entire
    /// per-tick stream (aggregates + per-entity brain/sense/output digests) to
    /// be byte-identical.
    #[test]
    fn threaded_sensing_capture_is_bit_identical() {
        let base = workspace_root().join("testdata/real/saves/lol.zip");
        if !base.is_file() {
            eprintln!("skipping threaded capture test: {} missing", base.display());
            return;
        }
        let ticks = 40_u64;
        let snapshots: Vec<u64> = (0..=ticks).collect();
        let run = |threads: usize, dir: &Path| {
            let mut contents = bibites_io::load_world_contents(&base, None).expect("load lol save");
            contents.seed = 12_345;
            let mut world = World::from_contents(contents).expect("build lol world");
            world.set_threads(threads);
            assert_eq!(world.threads(), threads);
            let live = world
                .bibites
                .iter()
                .filter(|(_, record)| !record.body.dead && !record.is_corpse)
                .count();
            assert!(
                live > 1,
                "parallel sensing needs >1 bibite; real save has {live}"
            );
            write_world_capture(&mut world, dir, &base, 12_345, ticks, &snapshots)
                .expect("write capture");
        };
        let dir1 = temp_dir("threads_sensing_1");
        let dir8 = temp_dir("threads_sensing_8");
        run(1, &dir1);
        run(8, &dir8);

        let ticks1 = std::fs::read(dir1.join("ticks.jsonl")).expect("serial ticks");
        let ticks8 = std::fs::read(dir8.join("ticks.jsonl")).expect("parallel ticks");
        assert_eq!(
            ticks1, ticks8,
            "1-thread vs 8-thread sensing produced different tick streams"
        );
        let events1 = std::fs::read(dir1.join("events.jsonl")).expect("serial events");
        let events8 = std::fs::read(dir8.join("events.jsonl")).expect("parallel events");
        assert_eq!(events1, events8, "1/8-thread event streams differ");

        // Confirm snapshots really carry per-entity digests for every tick.
        let first_line = ticks1
            .split(|byte| *byte == b'\n')
            .find(|line| !line.is_empty())
            .expect("first tick line");
        let first: serde_json::Value = serde_json::from_slice(first_line).expect("tick 0");
        let entities = first["entities"].as_array().expect("entities");
        assert!(
            entities.len() > 1,
            "expected >1 per-entity snapshot, got {}",
            entities.len()
        );
        for entity in entities {
            assert!(
                entity["brain_digest"]
                    .as_str()
                    .is_some_and(|d| d.len() == 16),
                "missing brain_digest in entity snapshot: {entity}"
            );
            assert!(
                entity["sense_digest"]
                    .as_str()
                    .is_some_and(|d| d.len() == 16),
                "missing sense_digest in entity snapshot: {entity}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir1);
        let _ = std::fs::remove_dir_all(&dir8);
    }
    #[test]
    fn event_schema_key_order_matches_game() {
        let mut event = EventRecord::new(33, "death");
        event.t = Some(1.0);
        event.id = Some(7);
        event.species = Some(2583);
        event.tag = Some("Bibite1".to_string());
        event.generation = Some(626);
        event.p1 = Some(8);
        event.p2 = Some(0);
        event.x = Some(1.0);
        event.y = Some(2.0);
        event.age_s = Some(3.0);
        event.born_at = Some(4.0);
        event.initial = Some(false);
        event.energy = Some(5.0);
        event.max_energy = Some(6.0);
        event.health = Some(7.0);
        event.max_health = Some(8.0);
        event.damage = Some(9.0);
        event.attacked = Some(1);
        event.travel = Some(10.0);
        event.bites = Some(2);
        event.damage_dealt = Some(11.0);
        event.eggs_laid = Some(3);
        event.cause = Some("killed".to_string());
        let text = serde_json::to_string(&event).expect("event json");
        let keys = [
            "schema",
            "tick",
            "kind",
            "t",
            "id",
            "species",
            "tag",
            "generation",
            "p1",
            "p2",
            "x",
            "y",
            "ageS",
            "bornAt",
            "initial",
            "energy",
            "maxEnergy",
            "health",
            "maxHealth",
            "damage",
            "attacked",
            "travel",
            "bites",
            "damageDealt",
            "eggsLaid",
            "cause",
        ];
        let mut cursor = 0usize;
        for key in keys {
            let needle = format!("\"{key}\"");
            let found = text[cursor..]
                .find(&needle)
                .unwrap_or_else(|| panic!("missing key {key} in {text}"));
            cursor += found + needle.len();
        }
        assert!(
            !text.contains("null"),
            "optional fields must be omitted: {text}"
        );
    }

    /// Historical evolved-save regression helper (not the canonical world; the
    /// Default scenario is the validation baseline per docs/TESTING_POLICY.md).
    fn lol_base() -> Option<PathBuf> {
        let base = workspace_root().join("testdata/real/saves/lol.zip");
        if base.is_file() {
            Some(base)
        } else {
            eprintln!(
                "skipping lol capture event test: {} missing",
                base.display()
            );
            None
        }
    }

    #[test]
    fn lol_capture_logs_birth_and_death_lifecycle_events() {
        let Some(base) = lol_base() else {
            return;
        };
        let mut contents = bibites_io::load_world_contents(&base, None).expect("load lol save");
        contents.seed = 12_345;
        let mut world = World::from_contents(contents).expect("build lol world");
        let dir = temp_dir("lol_lifecycle_events");
        write_world_capture(&mut world, &dir, &base, 12_345, 300, &[0, 300])
            .expect("write lol capture");
        let events_text = std::fs::read_to_string(dir.join("events.jsonl")).expect("events");
        let events: Vec<serde_json::Value> = events_text
            .lines()
            .map(|line| serde_json::from_str(line).expect("event json"))
            .collect();
        assert!(!events.is_empty(), "lol capture emitted no events");
        // initial events come first and match the starting live population.
        let initial_count = events
            .iter()
            .take_while(|event| event["kind"] == "initial")
            .count();
        assert!(initial_count > 0, "no initial events");
        let births = events
            .iter()
            .filter(|event| event["kind"] == "birth")
            .count();
        let deaths = events
            .iter()
            .filter(|event| event["kind"] == "death")
            .count();
        assert!(
            births > 0,
            "lol capture has no birth events: {initial_count} initial, {deaths} deaths"
        );
        assert!(deaths > 0, "lol capture has no death events");
        // Every lifecycle event is attributed to a sampling tick in range and
        // carries an external id; birth ids never collide with initial ids.
        let initial_ids: BTreeSet<i64> = events
            .iter()
            .filter(|event| event["kind"] == "initial")
            .filter_map(|event| event["id"].as_i64())
            .collect();
        let mut birth_ids = BTreeSet::new();
        for event in &events {
            let kind = event["kind"].as_str().unwrap_or_default();
            if kind == "initial" {
                continue;
            }
            let tick = event["tick"].as_i64().expect("event tick");
            assert!(
                (1..=300).contains(&tick),
                "event tick out of range: {event}"
            );
            let id = event["id"].as_i64().expect("lifecycle event id");
            if kind == "birth" {
                assert!(
                    !initial_ids.contains(&id),
                    "birth reused initial id: {event}"
                );
                assert!(birth_ids.insert(id), "duplicate birth for id {id}");
            }
            assert!(
                kind == "birth" || kind == "death",
                "unexpected event kind: {event}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

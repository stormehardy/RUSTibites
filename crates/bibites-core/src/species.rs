//! Species registry, genetic distance and lineage bookkeeping.
//!
//! Port of `Here/SimulationScripts.BibiteScripts/Species.cs` and
//! `Here/ManagementScripts/GlobalLineageManager.cs`; the authoritative write-up
//! with source references is `docs/spec/01-brain-genetics.md` ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â§9.
//!
//! Speciation is **bookkeeping only** ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â it never feeds back into bibite
//! behaviour. Assignment compares a bibite's genome+brain against each species'
//! template snapshot:
//!
//! ```text
//! distance = ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â½ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â£ |ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â½ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Âgene| / gene_max + brain_distance(template_brain, individual_brain)
//! ```
//!
//! and adopts/stays/founds species following the game's two-step `is_mutant`
//! state machine. See [`crate::species::SpeciesRegistry::assign`].
//!
//! Unlike the game (which aliases the founder's live gene/brain arrays into the
//! species template), this registry takes a deep snapshot. That is behaviourally
//! equivalent because distance only reads immutable gene/brain fields, and it
//! keeps the Rust data model free of aliasing (documented deviation).

use crate::brain::{Brain, NodeRecord, SynapseRecord};
use crate::error::{Error, Result};
use crate::genome::{GENE_DEFS, Gene, Genome, GenomeMeta, N_GENES};
use crate::ids::SpeciesId;
use crate::latin_names::LATIN_NAMES;
use crate::rng::{Rng, RngState};
use crate::settings::SimSettings;
use crate::species_log::{SpeciesDataPoint, SpeciesLog};

/// The species are logged every 5 simulated minutes: `SerialSpeciesConfig.acquisitionFactor` is 5 with the
/// acquisition in minutes, and `DataLogger.FixedUpdate` calls `LogSpeciesInfo` when its minute counter is a
/// multiple of it.
pub const SPECIES_LOG_PERIOD_SECONDS: f32 = 300.0;
/// `GlobalLineageManager.nRecords` stops counting here (`LogSpeciesInfo`).
pub const MAX_RECORDS: u64 = 10_000;
/// The first 20 active species do not increase the used genetic span
/// (`GlobalLineageManager.UpdateEffectiveSpeciesSpan`).
pub const SPECIES_SPAN_FREE_COUNT: usize = 20;

/// Deep snapshot of a species' defining genome and brain.
///
/// The game stores the founder's live arrays by reference
/// (`BibiteTemplate.cs:148-202`); Rust snapshots instead (documented deviation).
#[derive(Clone, Debug, PartialEq)]
pub struct TemplateSnapshot {
    /// Genome of the founder.
    pub genome: Genome,
    /// Brain of the founder, including its CSR. Its runtime state is never used
    /// by distance, but snapshotting it keeps the record self-contained.
    pub brain: Brain,
}

impl TemplateSnapshot {
    /// Snapshots a live genome+brain pair.
    pub fn from_live(genome: &Genome, brain: &Brain) -> Self {
        Self {
            genome: genome.clone(),
            brain: brain.clone(),
        }
    }

    /// Rebuilds a snapshot from serialized records (template semantics: runtime
    /// state is discarded and bias-initialised, like the game's template copy).
    pub fn from_records(
        genes: [f32; N_GENES],
        nodes: &[NodeRecord],
        synapses: &[SynapseRecord],
        period: f32,
    ) -> Result<Self> {
        Ok(Self {
            genome: Genome::from_values(genes),
            brain: Brain::from_template_records(nodes, synapses, period)?,
        })
    }

    /// Gene-distance component to an individual:
    /// `ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â½ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â£ |template - individual| / GENE_DEFS[i].max` (`Species.cs:288-297`).
    pub fn gene_distance(&self, genome: &Genome) -> f32 {
        let mut distance = 0.0;
        for (i, def) in GENE_DEFS.iter().enumerate() {
            distance += (self.genome.genes[i] - genome.genes[i]).abs() / def.max;
        }
        distance
    }

    /// Brain-distance component to an individual
    /// (`Species.BrainDistanceToIndividual`, `Species.cs:160-286`).
    pub fn brain_distance(&self, brain: &Brain, s: &SimSettings) -> f32 {
        self.brain.distance(brain, s)
    }

    /// Combined genetic distance to an individual
    /// (`Species.GeneticDistanceToIndividual`, `Species.cs:155-158`).
    pub fn distance(&self, genome: &Genome, brain: &Brain, s: &SimSettings) -> f32 {
        self.gene_distance(genome) + self.brain_distance(brain, s)
    }
}

/// Species record, mirroring the game's `Species` fields that are serialized.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeciesRecord {
    /// Persistent species id (`Species.speciesID`).
    pub id: SpeciesId,
    /// Simulated time of creation (`Species.timeCreation`).
    pub time_creation: f64,
    /// Generation of the founding specimen (`generationOfFirstSpecimen`).
    pub generation_of_first_specimen: u32,
    /// Never pruned when true (`Species.favorite`).
    pub favorite: bool,
    /// Genus (`Species.genericName`).
    pub generic_name: String,
    /// Species epithet (`Species.specificName`).
    pub specific_name: String,
    /// Free-form description.
    pub description: String,
    /// Parent species (`Species.parentSpecies`), `None` for root species.
    pub parent: Option<SpeciesId>,
    /// Root of the lineage tree (`Species.rootSpecies`), equals `id` for a root.
    pub root: SpeciesId,
    /// Child species in insertion order (`Species.children`).
    pub children: Vec<SpeciesId>,
    /// Genome+brain snapshot (`Species.template`).
    pub template: TemplateSnapshot,
    /// Live aggregate statistics.
    pub stats: SpeciesStats,
}

impl SpeciesRecord {
    /// The game's `name => genericName + " " + specificName` (`Species.cs:85`).
    pub fn name(&self) -> String {
        format!("{} {}", self.generic_name, self.specific_name)
    }

    /// A species is absolutely kept from pruning if it is a favourite or has no
    /// parent (`Species.absoluteKeep`, `Species.cs:69-79`).
    pub fn absolute_keep(&self) -> bool {
        self.favorite || self.parent.is_none()
    }

    /// No live/incubating members remain. Unlike the game save this predicate
    /// is derived from the current aggregate count, not a historical
    /// disappearance index.
    #[must_use]
    pub fn is_extinct(&self) -> bool {
        self.stats.count == 0
    }
}

/// Aggregate live statistics for a species.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpeciesStats {
    /// Number of living bibites plus incubating eggs in this species.
    pub count: u32,
    /// Summed energy of members (`Species.energy`, `Species.cs:83`).
    pub energy: f32,
    /// The multi-resolution history of the species (`Species.speciesData`): what the lineage tree draws and
    /// what pruning counts.
    pub log: SpeciesLog,
}

impl SpeciesStats {
    /// Statistics of a species that has no history yet; `is_template` is true for a species made from a
    /// template (the game's `Species(BibiteTemplate)`), whose log keeps a compressed point alive as soon as
    /// one of its points was.
    #[must_use]
    pub fn new(is_template: bool) -> Self {
        Self {
            count: 0,
            energy: 0.0,
            log: SpeciesLog::new(is_template),
        }
    }

    /// `speciesData.nPointsPresent`, used by pruning.
    #[must_use]
    pub fn n_points_present(&self) -> u32 {
        self.log.n_points_present()
    }

    /// `speciesData.totalIndexOfApparition`: the oldest point the species is present in.
    #[must_use]
    pub fn total_index_of_apparition(&self) -> i32 {
        self.log.total_index_of_apparition()
    }

    /// `speciesData.totalIndexOfDisappearance`: the newest point the species is gone from, `-1` while it is
    /// present in the newest one.
    #[must_use]
    pub fn total_index_of_disappearance(&self) -> i32 {
        self.log.total_index_of_disappearance()
    }
}

/// Serialized template shape used by `speciesData.json`/save species sections.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeciesTemplateRecord {
    /// Template/game version string, preserved for round-trip.
    pub version: Option<String>,
    /// Brain node records (template shape).
    pub nodes: Vec<NodeRecord>,
    /// Brain synapse records.
    pub synapses: Vec<SynapseRecord>,
    /// The 35 gene values.
    pub genes: [f32; N_GENES],
}

/// Plain record for `speciesData.json` / the save bundle's species section.
///
/// Field names mirror the game JSON (spec 04); `bibites-io` maps the snake_case
/// names to these fields. No serde in core.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeciesRecordIo {
    /// `speciesID`.
    pub species_id: u32,
    /// `timeCreation`.
    pub time_creation: f64,
    /// `generationOfFirstSpecimen`.
    pub generation_of_first_specimen: u32,
    /// `favorite`.
    pub favorite: bool,
    /// `genericName`.
    pub generic_name: String,
    /// `specificName`.
    pub specific_name: String,
    /// `description`.
    pub description: String,
    /// `parentID` (absent for root species).
    pub parent_id: Option<u32>,
    /// `template`.
    pub template: SpeciesTemplateRecord,
}

/// Lineage-wide record shape for `speciesData.json`/save species section.
///
/// Mirrors `GlobalLineageManager.SaveState` (`GlobalLineageManager.cs:361-379`):
/// recorded species, active species ids, global innovation counters,
/// `nextSpeciesID`, and logging statistics. `data.bin` history is kept opaque by
/// `bibites-io` for M1.
#[derive(Clone, Debug, PartialEq)]
pub struct LineageRecordIo {
    /// `recordedSpecies` in the game's recorded order.
    pub recorded_species: Vec<SpeciesRecordIo>,
    /// `activeSpeciesList` (ids, active order).
    pub active_species: Vec<u32>,
    /// `nodeMaxInnovation`.
    pub node_max_innovation: i64,
    /// `connectionMaxInnovation`.
    pub connection_max_innovation: i64,
    /// `nextSpeciesID`.
    pub next_species_id: u32,
    /// `maxEnergy` statistic.
    pub max_energy: f32,
    /// `maxCount` statistic.
    pub max_count: f32,
    /// `logCounter`.
    pub log_counter: u64,
    /// `nRecords`.
    pub n_records: u64,
}

/// Complete mutable state of a [`SpeciesRegistry`] for checkpointing (task-75
/// P2, encoded by the world in `SimSnapshot` extension `0x0104`).
///
/// The game save persists lineage records but not registry RNG/counters, so a
/// chunked resume needs this struct restored after the normal save load.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeciesCheckpoint {
    /// Recorded species, in the registry's recorded order.
    pub records: Vec<SpeciesRecord>,
    /// Active species ids in the game's `activeSpecies` list order.
    pub active: Vec<SpeciesId>,
    /// Next species id allocator (`nextSpeciesID`).
    pub next_id: u32,
    /// Next node innovation (`nodeMaxInnovation`).
    pub node_max_innovation: i64,
    /// Next connection innovation (`connectionMaxInnovation`).
    pub connection_max_innovation: i64,
    /// Species-lifetime simulated time.
    pub time: f64,
    /// Species history logging accumulator.
    pub log_progress: f32,
    /// Species history log counter.
    pub log_counter: u64,
    /// Number of species history records.
    pub n_records: u64,
    /// Maximum recorded species energy.
    pub max_energy: f32,
    /// Maximum recorded species count.
    pub max_count: f32,
    /// Species history logging period (seconds).
    pub log_period_seconds: f32,
    /// Pruning point limit.
    pub prune_point_limit: i32,
    /// Name-generation RNG position.
    pub name_rng: RngState,
    /// Cached effective genetic span.
    pub cached_span: f32,
    /// Active count the cached span was computed for (`usize::MAX` = unset).
    pub cached_span_active_count: usize,
}

const SPECIES_CHECKPOINT_MAGIC: [u8; 8] = *b"SPCHK001";
/// 2: the species history is a [`SpeciesLog`] (1 kept a bounded list of (time, count, energy) samples).
const SPECIES_CHECKPOINT_VERSION: u16 = 2;
const SPECIES_CHECKPOINT_MAX_RECORDS: usize = 100_000;
const SPECIES_CHECKPOINT_MAX_ITEMS: usize = 1_000_000;
const SPECIES_CHECKPOINT_MAX_STRING: usize = 1_048_576;

impl SpeciesCheckpoint {
    /// Encodes this checkpoint as dependency-free little-endian bytes for
    /// `SimSnapshot` extension `0x0104` (task-75 P2).
    ///
    /// Deterministic: the same checkpoint always produces the same bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = SpeciesWriter::default();
        w.bytes(&SPECIES_CHECKPOINT_MAGIC);
        w.u16(SPECIES_CHECKPOINT_VERSION);
        w.u32(self.records.len() as u32);
        for record in &self.records {
            w.u32(record.id.value());
            w.f64(record.time_creation);
            w.u32(record.generation_of_first_specimen);
            w.u8(u8::from(record.favorite));
            w.string(&record.generic_name);
            w.string(&record.specific_name);
            w.string(&record.description);
            match record.parent {
                Some(parent) => {
                    w.u8(1);
                    w.u32(parent.value());
                }
                None => w.u8(0),
            }
            w.u32(record.root.value());
            w.u32(record.children.len() as u32);
            for child in &record.children {
                w.u32(child.value());
            }
            let genes = record.template.genome.values();
            for gene in genes {
                w.f32(*gene);
            }
            let nodes = record.template.brain.to_node_records();
            w.u32(nodes.len() as u32);
            for node in &nodes {
                w.u8(node.node_type as u8);
                w.u32(node.index);
                w.i64(node.innovation);
                match &node.desc {
                    Some(desc) => {
                        w.u8(1);
                        w.string(desc);
                    }
                    None => w.u8(0),
                }
                w.f32(node.base_activation);
                w.f32(node.value);
                w.f32(node.last_input);
                w.f32(node.last_output);
            }
            let synapses = record.template.brain.to_synapse_records();
            w.u32(synapses.len() as u32);
            for synapse in &synapses {
                w.i64(synapse.innovation);
                w.u32(synapse.node_in);
                w.u32(synapse.node_out);
                w.f32(synapse.weight);
                w.u8(u8::from(synapse.enabled));
            }
            w.u32(record.stats.count);
            w.f32(record.stats.energy);
            let log = record.stats.log.encode_exact();
            w.u32(log.len() as u32);
            w.bytes(&log);
        }
        w.u32(self.active.len() as u32);
        for id in &self.active {
            w.u32(id.value());
        }
        w.u32(self.next_id);
        w.i64(self.node_max_innovation);
        w.i64(self.connection_max_innovation);
        w.f64(self.time);
        w.f32(self.log_progress);
        w.u64(self.log_counter);
        w.u64(self.n_records);
        w.f32(self.max_energy);
        w.f32(self.max_count);
        w.f32(self.log_period_seconds);
        w.i32(self.prune_point_limit);
        w.u64(self.name_rng.seed);
        for word in self.name_rng.words {
            w.u64(word);
        }
        w.f32(self.cached_span);
        w.u64(self.cached_span_active_count as u64);
        w.into_bytes()
    }

    /// Decodes bytes produced by [`SpeciesCheckpoint::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = SpeciesReader::new(bytes);
        if r.take(8)? != SPECIES_CHECKPOINT_MAGIC {
            return Err(Error::invalid_data("species checkpoint magic mismatch"));
        }
        let version = r.u16()?;
        if version != 1 && version != SPECIES_CHECKPOINT_VERSION {
            return Err(Error::invalid_data(format!(
                "unsupported species checkpoint version {version}"
            )));
        }
        let records_len = r.len("record count", SPECIES_CHECKPOINT_MAX_RECORDS)?;
        let mut records = Vec::with_capacity(records_len);
        for _ in 0..records_len {
            let id = SpeciesId::new(r.u32()?);
            let time_creation = r.f64()?;
            let generation_of_first_specimen = r.u32()?;
            let favorite = r.bool()?;
            let generic_name = r.string(SPECIES_CHECKPOINT_MAX_STRING)?;
            let specific_name = r.string(SPECIES_CHECKPOINT_MAX_STRING)?;
            let description = r.string(SPECIES_CHECKPOINT_MAX_STRING)?;
            let parent = match r.u8()? {
                0 => None,
                1 => Some(SpeciesId::new(r.u32()?)),
                _ => {
                    return Err(Error::invalid_data(
                        "invalid species checkpoint parent flag",
                    ));
                }
            };
            let root = SpeciesId::new(r.u32()?);
            let children_len = r.len("child count", SPECIES_CHECKPOINT_MAX_ITEMS)?;
            let mut children = Vec::with_capacity(children_len);
            for _ in 0..children_len {
                children.push(SpeciesId::new(r.u32()?));
            }
            let mut genes = [0.0_f32; N_GENES];
            for gene in &mut genes {
                *gene = r.f32()?;
            }
            let nodes_len = r.len("node count", SPECIES_CHECKPOINT_MAX_ITEMS)?;
            let mut nodes = Vec::with_capacity(nodes_len);
            for _ in 0..nodes_len {
                let raw_type = r.u8()?;
                let node_type = crate::brain::NodeType::from_u8(raw_type).ok_or_else(|| {
                    Error::invalid_data(format!("invalid species template node type {raw_type}"))
                })?;
                let index = r.u32()?;
                let innovation = r.i64()?;
                let desc = match r.u8()? {
                    0 => None,
                    1 => Some(r.string(SPECIES_CHECKPOINT_MAX_STRING)?),
                    _ => {
                        return Err(Error::invalid_data(
                            "invalid species checkpoint node desc flag",
                        ));
                    }
                };
                nodes.push(NodeRecord {
                    node_type,
                    index,
                    innovation,
                    desc,
                    base_activation: r.f32()?,
                    value: r.f32()?,
                    last_input: r.f32()?,
                    last_output: r.f32()?,
                });
            }
            let synapses_len = r.len("synapse count", SPECIES_CHECKPOINT_MAX_ITEMS)?;
            let mut synapses = Vec::with_capacity(synapses_len);
            for _ in 0..synapses_len {
                synapses.push(SynapseRecord {
                    innovation: r.i64()?,
                    node_in: r.u32()?,
                    node_out: r.u32()?,
                    weight: r.f32()?,
                    enabled: r.bool()?,
                });
            }
            let brain = Brain::from_records(&nodes, &synapses)?;
            let template = TemplateSnapshot {
                genome: Genome::from_values(genes),
                brain,
            };
            let count = r.u32()?;
            let energy = r.f32()?;
            let log = if version == 1 {
                // The first format kept a bounded list of (time, count, energy) samples.
                let _n_points_present = r.u32()?;
                let _total_index = r.u64()?;
                let history_len = r.len("history count", SPECIES_CHECKPOINT_MAX_ITEMS)?;
                let mut log = SpeciesLog::new(false);
                for _ in 0..history_len {
                    let _time = r.f64()?;
                    let sample_count = r.u32()?;
                    let sample_energy = r.f32()?;
                    log.push(SpeciesDataPoint::from_counts(sample_count, sample_energy));
                }
                log
            } else {
                let len = r.len("species log size", SPECIES_CHECKPOINT_MAX_STRING)?;
                SpeciesLog::decode_exact(r.take(len)?)?
            };
            let stats = SpeciesStats { count, energy, log };
            records.push(SpeciesRecord {
                id,
                time_creation,
                generation_of_first_specimen,
                favorite,
                generic_name,
                specific_name,
                description,
                parent,
                root,
                children,
                template,
                stats,
            });
        }
        let active_len = r.len("active species count", SPECIES_CHECKPOINT_MAX_ITEMS)?;
        let mut active = Vec::with_capacity(active_len);
        for _ in 0..active_len {
            active.push(SpeciesId::new(r.u32()?));
        }
        let next_id = r.u32()?;
        let node_max_innovation = r.i64()?;
        let connection_max_innovation = r.i64()?;
        let time = r.f64()?;
        let log_progress = r.f32()?;
        let log_counter = r.u64()?;
        let n_records = r.u64()?;
        let max_energy = r.f32()?;
        let max_count = r.f32()?;
        let log_period_seconds = r.f32()?;
        let prune_point_limit = r.i32()?;
        let seed = r.u64()?;
        let mut words = [0_u64; 4];
        for word in &mut words {
            *word = r.u64()?;
        }
        let name_rng = RngState::new(seed, words);
        if !name_rng.is_valid() {
            return Err(Error::invalid_data("invalid species checkpoint RNG state"));
        }
        let cached_span = r.f32()?;
        let cached_span_active_count = usize::try_from(r.u64()?)
            .map_err(|_| Error::invalid_data("species cached_span_active_count overflows usize"))?;
        if !r.finished() {
            return Err(Error::invalid_data("species checkpoint has trailing bytes"));
        }
        Ok(Self {
            records,
            active,
            next_id,
            node_max_innovation,
            connection_max_innovation,
            time,
            log_progress,
            log_counter,
            n_records,
            max_energy,
            max_count,
            log_period_seconds,
            prune_point_limit,
            name_rng,
            cached_span,
            cached_span_active_count,
        })
    }
}

#[derive(Default)]
struct SpeciesWriter {
    bytes: Vec<u8>,
}

impl SpeciesWriter {
    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }
    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn i64(&mut self, value: i64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn f32(&mut self, value: f32) {
        self.u32(value.to_bits());
    }
    fn f64(&mut self, value: f64) {
        self.u64(value.to_bits());
    }
    fn bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }
    fn string(&mut self, value: &str) {
        self.u32(value.len() as u32);
        self.bytes(value.as_bytes());
    }
    fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

struct SpeciesReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> SpeciesReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(len)
            .ok_or_else(|| Error::invalid_data("species checkpoint length overflow"))?;
        if end > self.bytes.len() {
            return Err(Error::invalid_data("species checkpoint is truncated"));
        }
        let result = &self.bytes[self.position..end];
        self.position = end;
        Ok(result)
    }
    fn len(&mut self, what: &str, limit: usize) -> Result<usize> {
        let value = self.u32()? as usize;
        if value > limit {
            return Err(Error::invalid_data(format!(
                "species checkpoint {what} exceeds limit {limit}"
            )));
        }
        Ok(value)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        let mut bytes = [0_u8; 8];
        bytes.copy_from_slice(b);
        Ok(u64::from_le_bytes(bytes))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().map_err(
            |_| Error::invalid_data("species checkpoint is truncated"),
        )?))
    }
    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().map_err(
            |_| Error::invalid_data("species checkpoint is truncated"),
        )?))
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.u64()?))
    }
    fn bool(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::invalid_data("invalid species checkpoint bool")),
        }
    }
    fn string(&mut self, limit: usize) -> Result<String> {
        let len = self.len("string length", limit)?;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| Error::invalid_data("invalid UTF-8 in species checkpoint string"))
    }
    fn finished(&self) -> bool {
        self.position == self.bytes.len()
    }
}

/// One read-only row for lineage/species list panels.
///
/// Strings are copied so the viewer does not borrow the registry while it
/// sorts/filters. `active`/`extinct` use the same predicates as the UI filters;
/// `n_points_present` and the two total indices are where the species is present in its history log (what
/// the lineage tree's rows are drawn from).
#[derive(Clone, Debug, PartialEq)]
pub struct SpeciesListEntry {
    /// Persistent species id.
    pub id: SpeciesId,
    /// Display name (`genericName + " " + specificName`).
    pub name: String,
    /// Generic name.
    pub generic_name: String,
    /// Specific epithet.
    pub specific_name: String,
    /// Free-form description.
    pub description: String,
    /// Parent species (`None` for a root).
    pub parent: Option<SpeciesId>,
    /// Root of the lineage.
    pub root: SpeciesId,
    /// Child species in insertion order.
    pub children: Vec<SpeciesId>,
    /// Live + incubating member count.
    pub count: u32,
    /// Summed member energy.
    pub energy: f32,
    /// Favourite flag.
    pub favorite: bool,
    /// Present in the active-species list.
    pub active: bool,
    /// Count is zero.
    pub extinct: bool,
    /// Creation time in simulated seconds.
    pub time_creation: f64,
    /// Generation of the founding specimen.
    pub generation_of_first_specimen: u32,
    /// `speciesData.nPointsPresent`.
    pub n_points_present: u32,
    /// `speciesData.totalIndexOfApparition`: the oldest point the species is present in.
    pub total_index_of_apparition: i32,
    /// `speciesData.totalIndexOfDisappearance`: the newest point it is gone from, `-1` while alive.
    pub total_index_of_disappearance: i32,
}

/// One parent → child edge in a lineage tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineageEdge {
    /// Parent species.
    pub parent: SpeciesId,
    /// Child species.
    pub child: SpeciesId,
}

/// One layout-ready node in a lineage tree.
///
/// `depth` is the generation rank (root = 0); `order` is the record index, so a
/// viewer can lay nodes out by `(depth, order)` or use `nodes` directly.
/// `subtree_*` counts include this node. `history` is the retained bounded
/// distribution samples (not the game's compressed per-rung arrays).
#[derive(Clone, Debug, PartialEq)]
pub struct LineageNode {
    /// Species id.
    pub id: SpeciesId,
    /// Parent species.
    pub parent: Option<SpeciesId>,
    /// Root of the lineage.
    pub root: SpeciesId,
    /// Generation depth from the root (root = 0).
    pub depth: usize,
    /// Record/insertion order within the registry.
    pub order: usize,
    /// Number of direct children.
    pub child_count: usize,
    /// Live + incubating member count.
    pub member_count: u32,
    /// Summed member energy.
    pub energy: f32,
    /// Number of species in this node's subtree, including itself.
    pub subtree_species_count: usize,
    /// Summed member count in this node's subtree, including itself.
    pub subtree_member_count: u64,
    /// Generation of the founding specimen.
    pub generation_of_first_specimen: u32,
    /// Favourite flag.
    pub favorite: bool,
    /// Present in the active-species list.
    pub active: bool,
    /// Current count is zero.
    pub extinct: bool,
    /// `speciesData.nPointsPresent`.
    pub n_points_present: u32,
    /// `speciesData.totalIndexOfApparition`: the oldest point the species is present in.
    pub total_index_of_apparition: i32,
    /// `speciesData.totalIndexOfDisappearance`: the newest point it is gone from, `-1` while alive.
    pub total_index_of_disappearance: i32,
}

/// Layout-ready lineage tree for UI rendering.
///
/// `roots` and `edges` describe the graph; `nodes` carries depth/order/counts
/// and the available distribution history. `max_depth` is the largest node
/// depth (0 for an empty tree).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LineageTree {
    /// Records with no parent, in record order.
    pub roots: Vec<SpeciesId>,
    /// Every parent → child edge in record/child-insertion order.
    pub edges: Vec<LineageEdge>,
    /// One node per species record, in record order.
    pub nodes: Vec<LineageNode>,
    /// Largest node depth (0 for an empty tree).
    pub max_depth: usize,
}

/// Read-only selection/navigation snapshot for one species.
///
/// `ancestors` lists parent upward (nearest first); `path_from_root` includes
/// the root, every ancestor, and the selected species. `descendants` is
/// depth-first child order excluding the selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineageSelection {
    /// Selected species.
    pub selected: SpeciesId,
    /// Parent, if the selection is not a root.
    pub parent: Option<SpeciesId>,
    /// Root of the lineage.
    pub root: SpeciesId,
    /// Direct children.
    pub children: Vec<SpeciesId>,
    /// Parent-upward ancestors (nearest first).
    pub ancestors: Vec<SpeciesId>,
    /// Depth-first descendants (excluding `selected`).
    pub descendants: Vec<SpeciesId>,
    /// Root-to-selected navigation path.
    pub path_from_root: Vec<SpeciesId>,
}

fn compute_subtree_counts(
    records: &[SpeciesRecord],
    index: usize,
    visiting: &mut [bool],
    out: &mut [(usize, u64)],
) -> (usize, u64) {
    if visiting[index] {
        return (0, 0);
    }
    visiting[index] = true;
    let record = &records[index];
    let mut species_count = 1;
    let mut member_count = u64::from(record.stats.count);
    for child in &record.children {
        if let Some(child_index) = records.iter().position(|r| r.id == *child) {
            let (child_species, child_members) =
                compute_subtree_counts(records, child_index, visiting, out);
            species_count += child_species;
            member_count += child_members;
        }
    }
    visiting[index] = false;
    out[index] = (species_count, member_count);
    (species_count, member_count)
}

/// The species/lineage registry owned by `World`.
///
/// Construction: [`SpeciesRegistry::new`] (fresh deterministic registry) or
/// [`SpeciesRegistry::from_records`] (load `speciesData.json`/save section).
#[derive(Clone, Debug)]
pub struct SpeciesRegistry {
    records: Vec<SpeciesRecord>,
    /// Active species ids in the game's `activeSpecies` list order.
    active: Vec<SpeciesId>,
    next_id: u32,
    node_max_innovation: i64,
    connection_max_innovation: i64,
    time: f64,
    log_progress: f32,
    log_counter: u64,
    n_records: u64,
    max_energy: f32,
    max_count: f32,
    log_period_seconds: f32,
    prune_point_limit: i32,
    name_rng: Rng,
    /// RUSTibites Improved: the words this registry names species from instead of the process-wide pool (`species_names::name_pool`); the tests use it so that
    /// they do not touch the shared one. Not saved, not part of any checkpoint.
    own_name_pool: Option<std::sync::Arc<crate::species_names::NamePool>>,
    cached_span: f32,
    cached_span_active_count: usize,
    /// Prunes the world has not applied to its members yet (`removed`, `survivor`).
    merges: Vec<(SpeciesId, SpeciesId)>,
}

impl Default for SpeciesRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SpeciesRegistry {
    /// Creates an empty registry with the game defaults (`speciesGeneticSpan`
    /// 1.5, `speciesSpanScalingFactor` 0.05, prune limit 2). Names use a
    /// deterministic RNG seeded with 0; use [`SpeciesRegistry::with_seed`] for a
    /// different name stream.
    pub fn new() -> Self {
        Self::with_seed(0)
    }

    /// Creates an empty registry with a specific name-generation seed.
    pub fn with_seed(seed: u64) -> Self {
        Self {
            records: Vec::new(),
            active: Vec::new(),
            next_id: 1,
            node_max_innovation: (crate::brain::FIRST_HIDDEN + 1) as i64,
            connection_max_innovation: 1,
            time: 0.0,
            log_progress: 0.0,
            log_counter: 0,
            n_records: 0,
            max_energy: 0.0,
            max_count: 0.0,
            log_period_seconds: SPECIES_LOG_PERIOD_SECONDS,
            prune_point_limit: 2,
            name_rng: Rng::from_seed(seed),
            own_name_pool: None,
            cached_span: 0.0,
            cached_span_active_count: usize::MAX,
            merges: Vec::new(),
        }
    }

    /// Captures every mutable field of this registry for checkpointing
    /// (task-75 P2). Restore it with [`SpeciesRegistry::restore_checkpoint`].
    pub fn capture_checkpoint(&self) -> SpeciesCheckpoint {
        SpeciesCheckpoint {
            records: self.records.clone(),
            active: self.active.clone(),
            next_id: self.next_id,
            node_max_innovation: self.node_max_innovation,
            connection_max_innovation: self.connection_max_innovation,
            time: self.time,
            log_progress: self.log_progress,
            log_counter: self.log_counter,
            n_records: self.n_records,
            max_energy: self.max_energy,
            max_count: self.max_count,
            log_period_seconds: self.log_period_seconds,
            prune_point_limit: self.prune_point_limit,
            name_rng: self.name_rng.state(),
            cached_span: self.cached_span,
            cached_span_active_count: self.cached_span_active_count,
        }
    }

    /// Restores a [`SpeciesCheckpoint`] captured by
    /// [`SpeciesRegistry::capture_checkpoint`].
    ///
    /// Fails before mutating this registry if the saved name RNG state is
    /// invalid (the all-zero xoshiro state).
    pub fn restore_checkpoint(&mut self, checkpoint: SpeciesCheckpoint) -> Result<()> {
        let Some(name_rng) = Rng::from_state(checkpoint.name_rng) else {
            return Err(Error::invalid_data(
                "species checkpoint has an invalid name RNG state",
            ));
        };
        self.records = checkpoint.records;
        self.active = checkpoint.active;
        self.next_id = checkpoint.next_id;
        self.node_max_innovation = checkpoint.node_max_innovation;
        self.connection_max_innovation = checkpoint.connection_max_innovation;
        self.time = checkpoint.time;
        self.log_progress = checkpoint.log_progress;
        self.log_counter = checkpoint.log_counter;
        self.n_records = checkpoint.n_records;
        self.max_energy = checkpoint.max_energy;
        self.max_count = checkpoint.max_count;
        self.log_period_seconds = checkpoint.log_period_seconds;
        self.prune_point_limit = checkpoint.prune_point_limit;
        self.name_rng = name_rng;
        self.cached_span = checkpoint.cached_span;
        self.cached_span_active_count = checkpoint.cached_span_active_count;
        self.merges.clear();
        Ok(())
    }

    /// Sets the pruning limit (`speciesPrunePointLimit`, default 2).
    pub fn set_prune_point_limit(&mut self, limit: i32) {
        self.prune_point_limit = limit.max(1);
    }

    /// Sets the species-history logging period in seconds (default 60).
    pub fn set_log_period_seconds(&mut self, seconds: f32) {
        self.log_period_seconds = seconds.max(1.0);
    }

    /// Simulated time in seconds (advanced by [`SpeciesRegistry::tick_stats`]).
    pub fn time(&self) -> f64 {
        self.time
    }

    /// Number of recorded species (including prunable history-only records).
    pub fn species_count(&self) -> usize {
        self.records.len()
    }

    /// Number of active species (`activeSpecies.Count`).
    pub fn active_species_count(&self) -> usize {
        self.active.len()
    }

    /// Active species ids in the game's list order.
    pub fn active_species(&self) -> &[SpeciesId] {
        &self.active
    }

    /// Global next hidden-node innovation counter (`nodeMaxInnovation`).
    pub fn node_max_innovation(&self) -> i64 {
        self.node_max_innovation
    }

    /// Global next connection innovation counter (`connectionMaxInnovation`).
    pub fn connection_max_innovation(&self) -> i64 {
        self.connection_max_innovation
    }

    /// Next species id (`Species.speciesMaxID`).
    pub fn next_species_id(&self) -> u32 {
        self.next_id
    }

    /// Merges a brain's innovation counters into the registry (called by the
    /// world after brain mutation; the game keeps them as global statics in
    /// `NEATBrain.cs:251-253`).
    pub fn merge_brain_innovations(&mut self, node_max: i64, connection_max: i64) {
        self.node_max_innovation = self.node_max_innovation.max(node_max);
        self.connection_max_innovation = self.connection_max_innovation.max(connection_max);
    }

    /// Looks up a species by id.
    pub fn get(&self, id: SpeciesId) -> Option<&SpeciesRecord> {
        self.records.iter().find(|record| record.id == id)
    }

    /// Mutable lookup (used by the world for energy/count updates).
    pub fn get_mut(&mut self, id: SpeciesId) -> Option<&mut SpeciesRecord> {
        self.records.iter_mut().find(|record| record.id == id)
    }

    /// Iterates all recorded species in the game's recorded order.
    pub fn records(&self) -> &[SpeciesRecord] {
        &self.records
    }

    /// Whether a species is in the active list.
    pub fn is_active(&self, id: SpeciesId) -> bool {
        self.active.contains(&id)
    }

    /// The species' parent id, if any.
    pub fn parent_of(&self, id: SpeciesId) -> Option<SpeciesId> {
        self.get(id).and_then(|record| record.parent)
    }

    /// Iterates a species' ancestors from parent upward.
    pub fn ancestors(&self, id: SpeciesId) -> Vec<SpeciesId> {
        let mut result = Vec::new();
        let mut current = self.parent_of(id);
        while let Some(parent) = current {
            result.push(parent);
            current = self.parent_of(parent);
        }
        result
    }

    /// Number of species in the subtree rooted at `id` (including itself).
    pub fn descendant_species_count(&self, id: SpeciesId) -> usize {
        let mut total = 1;
        if let Some(record) = self.get(id) {
            for child in &record.children {
                total += self.descendant_species_count(*child);
            }
        }
        total
    }

    /// Descendant species ids in game child-insertion order, depth-first and
    /// excluding `id` itself. Read-only helper for lineage tree/list panels.
    pub fn descendants_of(&self, id: SpeciesId) -> Vec<SpeciesId> {
        let mut result = Vec::new();
        self.collect_descendants(id, &mut result);
        result
    }

    fn collect_descendants(&self, id: SpeciesId, out: &mut Vec<SpeciesId>) {
        if let Some(record) = self.get(id) {
            for child in &record.children {
                out.push(*child);
                self.collect_descendants(*child, out);
            }
        }
    }

    /// All root species (records whose parent is `None`), in record order.
    #[must_use]
    pub fn roots(&self) -> Vec<SpeciesId> {
        self.records
            .iter()
            .filter(|record| record.parent.is_none())
            .map(|record| record.id)
            .collect()
    }

    /// Parent edge for every record, in record order. `None` marks a root;
    /// suitable for building a lineage-tree edge list directly.
    #[must_use]
    pub fn lineage_edges(&self) -> Vec<(SpeciesId, Option<SpeciesId>)> {
        self.records
            .iter()
            .map(|record| (record.id, record.parent))
            .collect()
    }

    /// Species records with no live/incubating members. This is the
    /// core-side extinction predicate; the game's historical disappearance
    /// index is not currently modelled (see `docs/UI_ACTIONS_BRAIN.md`).
    #[must_use]
    pub fn extinct_species(&self) -> Vec<SpeciesId> {
        self.records
            .iter()
            .filter(|record| record.stats.count == 0)
            .map(|record| record.id)
            .collect()
    }

    /// Viewer-friendly species list, in record order (game lineage order).
    #[must_use]
    pub fn species_list(&self) -> Vec<SpeciesListEntry> {
        self.records
            .iter()
            .map(|record| SpeciesListEntry {
                id: record.id,
                name: record.name(),
                generic_name: record.generic_name.clone(),
                specific_name: record.specific_name.clone(),
                description: record.description.clone(),
                parent: record.parent,
                root: record.root,
                children: record.children.clone(),
                count: record.stats.count,
                energy: record.stats.energy,
                favorite: record.favorite,
                active: self.active.contains(&record.id),
                extinct: record.stats.count == 0,
                time_creation: record.time_creation,
                generation_of_first_specimen: record.generation_of_first_specimen,
                n_points_present: record.stats.n_points_present(),
                total_index_of_apparition: record.stats.total_index_of_apparition(),
                total_index_of_disappearance: record.stats.total_index_of_disappearance(),
            })
            .collect()
    }

    /// Plain lineage tree with roots and parent→child edges.
    #[must_use]
    pub fn lineage_tree(&self) -> LineageTree {
        let mut roots = Vec::new();
        let mut edges = Vec::new();
        let mut depths: Vec<Option<usize>> = vec![None; self.records.len()];
        for (index, record) in self.records.iter().enumerate() {
            if record.parent.is_none() {
                roots.push(record.id);
                depths[index] = Some(0);
            }
            for child in &record.children {
                edges.push(LineageEdge {
                    parent: record.id,
                    child: *child,
                });
            }
        }
        // Resolve generation depth parent-first; records are normally already
        // ordered this way, but the loop is robust to insertion order.
        loop {
            let mut changed = false;
            for index in 0..self.records.len() {
                if depths[index].is_some() {
                    continue;
                }
                let Some(parent) = self.records[index].parent else {
                    continue;
                };
                let Some(parent_index) = self.records.iter().position(|r| r.id == parent) else {
                    continue;
                };
                if let Some(parent_depth) = depths[parent_index] {
                    depths[index] = Some(parent_depth + 1);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for depth in &mut depths {
            depth.get_or_insert(0);
        }

        let mut subtree: Vec<(usize, u64)> = vec![(1, 0); self.records.len()];
        let mut visiting = vec![false; self.records.len()];
        for index in 0..self.records.len() {
            compute_subtree_counts(&self.records, index, &mut visiting, &mut subtree);
        }

        let mut nodes = Vec::with_capacity(self.records.len());
        let mut max_depth = 0;
        for (index, record) in self.records.iter().enumerate() {
            let depth = depths[index].unwrap_or(0);
            max_depth = max_depth.max(depth);
            nodes.push(LineageNode {
                id: record.id,
                parent: record.parent,
                root: record.root,
                depth,
                order: index,
                child_count: record.children.len(),
                member_count: record.stats.count,
                energy: record.stats.energy,
                subtree_species_count: subtree[index].0,
                subtree_member_count: subtree[index].1,
                generation_of_first_specimen: record.generation_of_first_specimen,
                favorite: record.favorite,
                active: self.active.contains(&record.id),
                extinct: record.stats.count == 0,
                n_points_present: record.stats.n_points_present(),
                total_index_of_apparition: record.stats.total_index_of_apparition(),
                total_index_of_disappearance: record.stats.total_index_of_disappearance(),
            });
        }
        LineageTree {
            roots,
            edges,
            nodes,
            max_depth,
        }
    }

    /// The species' history log (`Species.speciesData`).
    #[must_use]
    pub fn log(&self, id: SpeciesId) -> Option<&SpeciesLog> {
        self.get(id).map(|record| &record.stats.log)
    }

    /// Replaces a species' history log (loading the species section of `data.bin`).
    pub fn set_log(&mut self, id: SpeciesId, log: SpeciesLog) {
        if let Some(record) = self.get_mut(id) {
            record.stats.log = log;
        }
    }

    /// Selection/navigation snapshot for a species panel or ancestral-lineage
    /// control. Returns `None` for an unknown id.
    #[must_use]
    pub fn selected_lineage(&self, id: SpeciesId) -> Option<LineageSelection> {
        let record = self.get(id)?;
        let ancestors = self.ancestors(id);
        let mut path_from_root: Vec<SpeciesId> = ancestors.iter().rev().copied().collect();
        path_from_root.push(id);
        Some(LineageSelection {
            selected: id,
            parent: record.parent,
            root: record.root,
            children: record.children.clone(),
            ancestors,
            descendants: self.descendants_of(id),
            path_from_root,
        })
    }

    /// Marks/unmarks a species as a favourite (`Species.favorite`).
    pub fn set_favorite(&mut self, id: SpeciesId, favorite: bool) {
        if let Some(record) = self.get_mut(id) {
            record.favorite = favorite;
        }
    }

    /// `SpeciesInfoPanel.AssertSpeciesFormat`: a species name is a capitalised genus and a
    /// lower-case epithet, one space apart (`^[A-Z][a-z]+ [a-z]+$`).
    #[must_use]
    pub fn is_valid_species_name(text: &str) -> bool {
        let Some((genus, epithet)) = text.split_once(' ') else {
            return false;
        };
        let mut genus_chars = genus.chars();
        genus_chars.next().is_some_and(|c| c.is_ascii_uppercase())
            && genus.chars().count() >= 2
            && genus_chars.all(|c| c.is_ascii_lowercase())
            && !epithet.is_empty()
            && epithet.chars().all(|c| c.is_ascii_lowercase())
    }

    /// `SpeciesInfoPanel.FinishEditing`: renames a species to `text` ("Genus epithet"). Returns
    /// `false` (and changes nothing) when the text does not have that form or the species is gone.
    pub fn rename(&mut self, id: SpeciesId, text: &str) -> bool {
        if !Self::is_valid_species_name(text) {
            return false;
        }
        let Some((genus, epithet)) = text.split_once(' ') else {
            return false;
        };
        let (genus, epithet) = (genus.to_owned(), epithet.to_owned());
        match self.get_mut(id) {
            Some(record) => {
                record.generic_name = genus;
                record.specific_name = epithet;
                true
            }
            None => false,
        }
    }

    /// `SpeciesInfoPanel.OnNotesEdit`: the species' free-form description.
    pub fn set_description(&mut self, id: SpeciesId, text: &str) {
        if let Some(record) = self.get_mut(id) {
            record.description = text.to_owned();
        }
    }

    /// Whether the info panel's Prune button is enabled for the species (`SpeciesInfoPanel.FillPanel`:
    /// `!absoluteKeep && count == 0 && speciesData.totalIndexOfDisappearance > 0`).
    #[must_use]
    pub fn can_prune(&self, id: SpeciesId) -> bool {
        self.get(id).is_some_and(|record| {
            !record.absolute_keep()
                && record.stats.count == 0
                && record.stats.total_index_of_disappearance() > 0
        })
    }

    /// `Species.GetDescendantCount`: the members of the species and of all its descendants.
    #[must_use]
    pub fn alive_descendant_count(&self, id: SpeciesId) -> u32 {
        let Some(record) = self.get(id) else {
            return 0;
        };
        record.stats.count
            + record
                .children
                .iter()
                .map(|child| self.alive_descendant_count(*child))
                .sum::<u32>()
    }

    /// Used genetic span (`usedSpeciesSpan`): `speciesGeneticSpan +
    /// max(activeCount - 20, 0) * speciesSpanScalingFactor`
    /// (`GlobalLineageManager.cs:73-79`).
    pub fn used_span(&mut self, s: &SimSettings) -> f32 {
        // The game recomputes it when the species count or either setting changes (`OnChange` ->
        // `UpdateEffectiveSpeciesSpan`), so the settings panel's sliders act at once: the value is
        // computed from the current settings (two multiplications), the cache only records it.
        if self.cached_span_active_count == usize::MAX {
            self.update_span_count();
        }
        let active_count = self.cached_span_active_count;
        let base = s.independents.species_genetic_span;
        let scaling = s.independents.species_span_scaling_factor;
        self.cached_span =
            base + (active_count.saturating_sub(SPECIES_SPAN_FREE_COUNT)) as f32 * scaling;
        self.cached_span_active_count = active_count;
        self.cached_span
    }

    /// Raises `maxEnergy` to the most energy all the species held together at any point of their logged histories. A save the port wrote
    /// before it kept `maxEnergy` had 0 there (and the game's lineage mesh divides by it), and the value a game save stores is kept when it is
    /// larger (the compressed older points are averages, so what the logs give is never above the true maximum).
    pub fn raise_max_energy_from_history(&mut self) {
        let newest_oldest = self
            .records
            .iter()
            .map(|record| record.stats.log.total_index_of_apparition())
            .max()
            .unwrap_or(-1);
        let mut best = 0.0_f32;
        for index in 0..=newest_oldest.max(-1) {
            let total: f32 = self
                .records
                .iter()
                .map(|record| record.stats.log.at(index as usize).energy)
                .sum();
            best = best.max(total);
        }
        self.max_energy = self.max_energy.max(best);
    }

    /// `GlobalLineageManager.maxEnergy`: the most energy all the species had together at one of the logs (the
    /// lineage mesh view scales its width by how much of it is there).
    #[must_use]
    pub fn max_energy(&self) -> f32 {
        self.max_energy
    }

    /// `usedSpeciesSpan` without touching the cache (for read-only callers, the species panel's
    /// distances): the same `speciesGeneticSpan + max(activeCount - 20, 0) * scaling`.
    #[must_use]
    pub fn current_used_span(&self, s: &SimSettings) -> f32 {
        let base = s.independents.species_genetic_span;
        let scaling = s.independents.species_span_scaling_factor;
        let count = if self.cached_span_active_count == usize::MAX { self.active.len() } else { self.cached_span_active_count };
        base + (count.saturating_sub(SPECIES_SPAN_FREE_COUNT)) as f32 * scaling
    }

    /// Game `CheckNewSpecies` (`GlobalLineageManager.cs:194-249`).
    ///
    /// The state machine, in order:
    /// 1. no inherited species ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¾Ãƒâ€šÃ‚Â¢ adopt the **first** active species within the
    ///    used span (list order), else found a new root species;
    /// 2. inherited species and distance `< usedSpan` ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¾Ãƒâ€šÃ‚Â¢ stay;
    /// 3. otherwise try active child species within `usedSpan/2` (insertion
    ///    order, last match wins), adopt if found;
    /// 4. otherwise the two-step mutant rule: a first outlier only sets
    ///    `meta.is_mutant`; a second consecutive outlier founds a new child
    ///    species whose template is the founder individual (see
    ///    [`SpeciesRegistry::assign_with_founder`]).
    ///
    /// `meta.species` and `meta.is_mutant` are updated in place. The founder for
    /// a new species defaults to the individual being assigned; pass the actual
    /// parent via [`SpeciesRegistry::assign_with_founder`] to match the game
    /// (`GlobalLineageManager.cs:235-237`).
    pub fn assign(
        &mut self,
        genome: &Genome,
        brain: &Brain,
        meta: &mut GenomeMeta,
        s: &SimSettings,
    ) -> SpeciesId {
        self.assign_with_founder(genome, brain, meta, None, s)
    }

    /// [`crate::species::SpeciesRegistry::assign`] with an explicit founder `(genome, brain)`
    /// pair for newly created species.
    pub fn assign_with_founder(
        &mut self,
        genome: &Genome,
        brain: &Brain,
        meta: &mut GenomeMeta,
        founder: Option<(&Genome, &Brain)>,
        s: &SimSettings,
    ) -> SpeciesId {
        // Runtime settings animators may change these; read them fresh.
        self.prune_point_limit = s.independents.species_prune_point_limit.max(1);
        let span = self.used_span(s);

        let inherited = meta.species.filter(|id| self.get(*id).is_some());
        let Some(current) = inherited else {
            let adopted = self
                .active
                .iter()
                .copied()
                .find(|id| self.distance_to(*id, genome, brain, s) <= span);
            if let Some(id) = adopted {
                meta.species = Some(id);
                return id;
            }
            let id = self.create_species(genome, brain, meta, None);
            meta.species = Some(id);
            return id;
        };

        if self.distance_to(current, genome, brain, s) < span {
            meta.species = Some(current);
            return current;
        }

        // Try an active child species within half the span. Game note: scan in
        // children insertion order and let the last match win (no early break).
        let half = span * 0.5;
        let mut candidate: Option<SpeciesId> = None;
        if let Some(parent) = self.get(current) {
            for &child in &parent.children {
                if !self.is_active(child) {
                    continue;
                }
                if let Some(record) = self.get(child) {
                    if record.stats.count >= 1 && record.template.distance(genome, brain, s) <= half
                    {
                        candidate = Some(child);
                    }
                }
            }
        }
        if let Some(child) = candidate {
            meta.species = Some(child);
            return child;
        }

        // Two-step mutant rule.
        if !meta.is_mutant {
            meta.is_mutant = true;
            meta.species = Some(current);
            return current;
        }
        meta.is_mutant = false;
        let (founder_genome, founder_brain) = founder.unwrap_or((genome, brain));
        let id = self.create_species(founder_genome, founder_brain, meta, Some(current));
        // The game moves the founder individual from the old species into the new
        // one (`GlobalLineageManager.cs:236-244`); mirror the aggregate counts.
        if let Some(old) = self.get_mut(current) {
            old.stats.count = old.stats.count.saturating_sub(1);
        }
        if let Some(new) = self.get_mut(id) {
            new.stats.count = new.stats.count.saturating_add(1);
        }
        meta.species = Some(id);
        id
    }

    /// Adds a pre-existing species from a scenario/template (game
    /// `CreateNewSpecies(BibiteTemplate)`, `GlobalLineageManager.cs:313-320`).
    pub fn add_template_species(
        &mut self,
        name: &str,
        description: &str,
        genome: &Genome,
        brain: &Brain,
    ) -> SpeciesId {
        let (generic, specific) = split_species_name(name);
        let id = SpeciesId::new(self.next_id);
        self.next_id += 1;
        let record = SpeciesRecord {
            id,
            time_creation: self.time,
            generation_of_first_specimen: 0,
            favorite: false,
            generic_name: generic,
            specific_name: specific,
            description: description.to_string(),
            parent: None,
            root: id,
            children: Vec::new(),
            template: TemplateSnapshot::from_live(genome, brain),
            // `new Species(BibiteTemplate)`: its log keeps what one point lived.
            stats: SpeciesStats::new(true),
        };
        self.records.push(record);
        self.active.push(id);
        self.update_span_count();
        id
    }

    /// `GlobalLineageManager.CheckSpeciesOfEgg` (an egg that resumes hatching after a load): the first active species whose distance to the egg is
    /// within the used span, else the nearest one (`None` when no species is active).
    pub fn check_species_of_egg(&mut self, genome: &Genome, brain: &Brain, s: &SimSettings) -> Option<SpeciesId> {
        let span = self.used_span(s);
        let mut best: Option<(f32, SpeciesId)> = None;
        for id in self.active.clone() {
            let distance = self.distance_to(id, genome, brain, s);
            if best.is_none_or(|(nearest, _)| distance < nearest) {
                best = Some((distance, id));
            }
            if distance <= span {
                return Some(id);
            }
        }
        best.map(|(_, id)| id)
    }

    /// `GlobalLineageManager.FindSpeciesFromTemplate`: a template that names a recorded species gets that species (put back into the active list at
    /// its place in the recorded order when it was not active; `only_look_up` leaves the list alone), any other template founds a new one.
    pub fn find_species_from_template(
        &mut self,
        name: &str,
        description: &str,
        genome: &Genome,
        brain: &Brain,
        only_look_up: bool,
    ) -> SpeciesId {
        let found = if name.is_empty() { None } else { self.records.iter().find(|record| record.name() == name).map(|record| record.id) };
        let Some(id) = found else {
            return self.add_template_species(if name.is_empty() { "Zz zzspecies" } else { name }, description, genome, brain);
        };
        if only_look_up || self.active.contains(&id) {
            return id;
        }
        let recorded = self.index_of(id).unwrap_or(usize::MAX);
        let place = self.active.iter().position(|other| self.index_of(*other).is_some_and(|index| index > recorded));
        match place {
            Some(at) => self.active.insert(at, id),
            None => self.active.push(id),
        }
        self.update_span_count();
        id
    }

    /// `UpdateEffectiveSpeciesSpan`'s read of `activeSpecies.Count`: the game calls it when a species is created or found from a template and when a
    /// span setting changes, not when `LogSpeciesInfo` drops the empty species, so the span stays as it was until the next of those events.
    fn update_span_count(&mut self) {
        self.cached_span_active_count = self.active.len();
    }

    /// `UpdateEffectiveSpeciesSpan` for the callers outside the registry: `SpeciesPanel.OpenPanel` calls it when the Lineage panel opens, so the
    /// span takes the number of species that are active then (the species that died out since it was last read left the list at the species log
    /// and make it smaller) and judges the births that follow. An edit of `speciesGeneticSpan` or `speciesSpanScalingFactor` calls it as well
    /// (their `OnChange`, `GlobalLineageManager.Awake`): `World::apply_settings`.
    pub fn refresh_span(&mut self) {
        self.update_span_count();
    }

    fn create_species(
        &mut self,
        genome: &Genome,
        brain: &Brain,
        meta: &GenomeMeta,
        parent: Option<SpeciesId>,
    ) -> SpeciesId {
        let id = SpeciesId::new(self.next_id);
        self.next_id += 1;
        let (generic_name, specific_name) = match parent {
            Some(parent_id) => {
                let parent_generic = self
                    .get(parent_id)
                    .map(|record| record.generic_name.clone())
                    .unwrap_or_default();
                self.generate_new_name(&parent_generic, parent_id)
            }
            None => {
                let generic = self.random_unused_genus();
                let specific = self.random_unused_specific(Some(&generic));
                (generic, specific)
            }
        };
        let root = parent
            .and_then(|parent_id| self.get(parent_id))
            .map_or(id, |record| record.root);
        let record = SpeciesRecord {
            id,
            time_creation: self.time,
            generation_of_first_specimen: meta.generation,
            favorite: false,
            generic_name,
            specific_name,
            description: String::new(),
            parent,
            root,
            children: Vec::new(),
            template: TemplateSnapshot::from_live(genome, brain),
            stats: SpeciesStats::default(),
        };
        if let Some(parent_id) = parent {
            if let Some(parent_index) = self.index_of(parent_id) {
                self.records[parent_index].children.push(id);
                // Game inserts a child species immediately after its parent in
                // recordedSpecies and in the active list's recorded ordering.
                self.records.insert(parent_index + 1, record);
                let active_pos = self
                    .active
                    .iter()
                    .position(|active| {
                        self.index_of(*active)
                            .is_some_and(|index| index > parent_index + 1)
                    })
                    .unwrap_or(self.active.len());
                self.active.insert(active_pos, id);
                self.update_span_count();
                return id;
            }
        }
        self.records.push(record);
        self.active.push(id);
        self.update_span_count();
        id
    }

    /// Genetic distance from a species' template to an individual.
    pub fn distance_to(
        &self,
        species: SpeciesId,
        genome: &Genome,
        brain: &Brain,
        s: &SimSettings,
    ) -> f32 {
        match self.get(species) {
            Some(record) => record.template.distance(genome, brain, s),
            None => f32::INFINITY,
        }
    }

    /// Records a birth in a species (`Species.AddToSpecies`).
    pub fn on_birth(&mut self, child: SpeciesId, _parent: SpeciesId, _generation: u32) {
        if let Some(record) = self.get_mut(child) {
            record.stats.count = record.stats.count.saturating_add(1);
        }
    }

    /// Records a death/removal from a species (`Species.RemoveFromSpecies`).
    pub fn on_death(&mut self, species: SpeciesId) {
        if let Some(record) = self.get_mut(species) {
            record.stats.count = record.stats.count.saturating_sub(1);
        }
    }

    /// Recomputes per-species live member counts from an authoritative live-id
    /// stream and drops species with no live members from the active list.
    ///
    /// Intended as a safety net for the game's incremental `AddToSpecies` /
    /// `RemoveFromSpecies` bookkeeping: a death that removes the body record
    /// before event processing cannot call [`Self::on_death`] with the old
    /// record, so the caller can pass the live bibites' species ids at a safe
    /// boundary (for example when statistics are sampled). The live-species
    /// statistics themselves already use the per-record species; this method
    /// reconciles the registry aggregates and active list with them.
    pub fn reconcile_live_counts<I>(&mut self, live: I) -> usize
    where
        I: IntoIterator<Item = SpeciesId>,
    {
        for record in &mut self.records {
            record.stats.count = 0;
        }
        for species in live {
            if let Some(record) = self.get_mut(species) {
                record.stats.count = record.stats.count.saturating_add(1);
            }
        }
        let before = self.active.len();
        let mut active: Vec<SpeciesId> = self
            .active
            .iter()
            .copied()
            .filter(|id| self.get(*id).is_some_and(|record| record.stats.count > 0))
            .collect();
        // A reconciled live species must be active even if a previous reconcile
        // pruned it (robustness against orderings where births were observed
        // before the next tick-boundary sample). Preserve existing active order,
        // then append newly-live records in recorded-species order.
        for record in &self.records {
            if record.stats.count > 0 && !active.contains(&record.id) {
                active.push(record.id);
            }
        }
        self.active = active;
        self.update_span_count();
        before.saturating_sub(self.active.len())
    }

    /// Number of species-log records taken so far (`GlobalLineageManager.nRecords`).
    #[must_use]
    pub fn n_records(&self) -> u64 {
        self.n_records
    }

    /// Overrides the record counter (tests / loading a save with its history).
    pub fn set_n_records(&mut self, records: u64) {
        self.n_records = records;
    }

    /// Recounts member totals from the authoritative live stream **without**
    /// touching the active list: the game removes extinct species from
    /// `activeSpecies` only inside `LogSpeciesInfo` (`GlobalLineageManager.cs:94-95`)
    /// and never re-adds them.
    pub fn reconcile_counts<I>(&mut self, live: I)
    where
        I: IntoIterator<Item = SpeciesId>,
    {
        for record in &mut self.records {
            record.stats.count = 0;
        }
        for species in live {
            if let Some(record) = self.get_mut(species) {
                record.stats.count = record.stats.count.saturating_add(1);
            }
        }
    }

    /// [`SpeciesRegistry::reconcile_counts`] for members already counted by species: `counts` lists each species with
    /// its number of members (a species it does not list has none). One look-up per species where the other takes one per
    /// member, and a look-up is a linear search through every species ever recorded.
    pub fn reconcile_counts_by_species<I>(&mut self, counts: I)
    where
        I: IntoIterator<Item = (SpeciesId, u32)>,
    {
        for record in &mut self.records {
            record.stats.count = 0;
        }
        for (species, members) in counts {
            if let Some(record) = self.get_mut(species) {
                record.stats.count = record.stats.count.saturating_add(members);
            }
        }
    }

    /// Adds/removes aggregate member energy (the world passes per-bibite deltas;
    /// game `Species.energy` sums live members).
    pub fn add_energy(&mut self, species: SpeciesId, delta: f32) {
        if let Some(record) = self.get_mut(species) {
            record.stats.energy += delta;
        }
    }

    /// Advances the registry clock and says whether a log is due (the progress passed the logging
    /// period). The world then collects the samples ([`SpeciesDataPoint::from_members`]) and calls
    /// [`SpeciesRegistry::log_stats_with`].
    pub fn advance_clock(&mut self, dt: f32) -> bool {
        self.time += f64::from(dt);
        self.log_progress += dt;
        if self.log_progress >= self.log_period_seconds {
            self.log_progress -= self.log_period_seconds;
            return true;
        }
        false
    }

    /// Advances the clock and logs from the member counts when due (no positions are known here, which is
    /// what tests and callers without a world get; the world uses [`SpeciesRegistry::advance_clock`]).
    pub fn tick_stats(&mut self, dt: f32) {
        if self.advance_clock(dt) {
            self.log_stats();
        }
    }

    /// [`SpeciesRegistry::log_stats_with`] with a sample made of the member count and energy.
    pub fn log_stats(&mut self) {
        self.log_stats_with(&[]);
    }

    /// `GlobalLineageManager.LogSpeciesInfo`: every recorded species logs its sample (`samples`; a species
    /// without one logs from its count and energy), the maxima grow, `SpeciesTreeCleanup` prunes, then the
    /// species without members leave the active list.
    pub fn log_stats_with(&mut self, samples: &[(SpeciesId, SpeciesDataPoint)]) {
        self.log_counter += 1;
        if !self.records.is_empty() && self.n_records < MAX_RECORDS {
            self.n_records += 1;
        }
        let total_count: u64 = self
            .records
            .iter()
            .map(|record| u64::from(record.stats.count))
            .sum();
        // `maxEnergy = Max(maxEnergy, activeSpecies.Sum(s => s.energy))`, where `Species.energy` is what its members hold: the samples
        // of this log (the registry's own `stats.energy` is never kept up by the world, so summing it left `maxEnergy` at 0 in every
        // saved world - and the game's lineage mesh divides by it: its shapes came out as NaN).
        let active: std::collections::HashSet<SpeciesId> = self.active.iter().copied().collect();
        let total_energy: f32 = self
            .records
            .iter()
            .filter(|record| active.contains(&record.id))
            .map(|record| {
                samples
                    .iter()
                    .find(|(id, _)| *id == record.id)
                    .map_or(record.stats.energy, |(_, point)| point.energy)
            })
            .sum();
        self.max_count = self.max_count.max(total_count as f32);
        self.max_energy = self.max_energy.max(total_energy);
        for record in &mut self.records {
            let point = samples.iter().find(|(id, _)| *id == record.id).map_or_else(
                || SpeciesDataPoint::from_counts(record.stats.count, record.stats.energy),
                |(_, point)| *point,
            );
            record.stats.log.push(point);
        }
        // `LogSpeciesInfo` order: tree cleanup first (extinct species are still
        // in `activeSpecies`), then drop the empty ones.
        self.species_tree_cleanup();
        let empty: Vec<SpeciesId> = self
            .active
            .iter()
            .copied()
            .filter(|id| self.get(*id).is_none_or(|record| record.stats.count == 0))
            .collect();
        self.active.retain(|id| !empty.contains(id));
    }

    /// `GlobalLineageManager.SpeciesTreeCleanup`: the species that are not active and have at most
    /// `speciesPrunePointLimit` present points are pruned. A species that is kept for good (a favourite or a
    /// root) first absorbs the history of its children and siblings, the least relevant first, until it has
    /// more points than the limit; one that still has none is pruned too.
    pub fn species_tree_cleanup(&mut self) {
        let limit = self.prune_point_limit.max(1) as u32;
        let mut to_prune: Vec<SpeciesId> = Vec::new();
        let mut kept: Vec<SpeciesId> = Vec::new();
        for record in &self.records {
            if self.active.contains(&record.id) || record.stats.n_points_present() > limit {
                continue;
            }
            if record.absolute_keep() {
                kept.push(record.id);
            } else {
                to_prune.push(record.id);
            }
        }
        // `OrderByDescending(totalIndexOfApparition)` is stable.
        kept.sort_by_key(|id| {
            std::cmp::Reverse(
                self.get(*id)
                    .map_or(i32::MIN, |record| record.stats.total_index_of_apparition()),
            )
        });
        for item in kept {
            let Some(record) = self.get(item) else {
                continue;
            };
            let mut candidates: Vec<SpeciesId> = record.children.clone();
            if let Some(parent) = record.parent.and_then(|parent| self.get(parent)) {
                candidates.extend(parent.children.iter().copied());
                if parent.children.is_empty() {
                    candidates.push(parent.id);
                }
            }
            let start = record.stats.total_index_of_apparition();
            candidates.retain(|id| self.get(*id).is_some_and(|other| !other.absolute_keep()));
            // `OrderBy((children + 1) * nPointsPresent * (|start - totalIndexOfApparition| + 1))`, stable.
            candidates.sort_by_key(|id| {
                self.get(*id).map_or(i64::MAX, |other| {
                    (other.children.len() as i64 + 1)
                        * i64::from(other.stats.n_points_present())
                        * (i64::from((start - other.stats.total_index_of_apparition()).abs()) + 1)
                })
            });
            let mut queue = candidates.into_iter();
            while self
                .get(item)
                .is_some_and(|record| record.stats.n_points_present() <= limit)
            {
                let Some(species) = queue.next() else {
                    break;
                };
                self.prune_species(species, true, Some(item));
                to_prune.retain(|id| *id != species);
            }
            if self
                .get(item)
                .is_some_and(|record| record.stats.n_points_present() < 1)
            {
                to_prune.push(item);
            }
        }
        for id in to_prune {
            self.prune_species(id, false, None);
        }
    }

    /// Removes a species from the lineage tree, re-parenting its children and
    /// merging its history into `merge_into` (default: parent or largest child).
    /// Returns the surviving species id when statistics were merged. A
    /// favourite or root species is left alone unless a target is given.
    pub fn prune_from_tree(
        &mut self,
        species: SpeciesId,
        merge_into: Option<SpeciesId>,
    ) -> Option<SpeciesId> {
        if merge_into.is_none() {
            if let Some(record) = self.get(species) {
                if record.absolute_keep() {
                    return Some(species);
                }
            }
        }
        self.prune_species(species, merge_into.is_some(), merge_into)
    }

    /// `GlobalLineageManager.PruneSpeciesFromTree`: without `force` a species with more present points than
    /// the limit stays (and is returned).
    fn prune_species(
        &mut self,
        species: SpeciesId,
        force: bool,
        merge_into: Option<SpeciesId>,
    ) -> Option<SpeciesId> {
        let limit = self.prune_point_limit.max(1) as u32;
        let record = self.get(species)?;
        if !force && record.stats.n_points_present() > limit {
            return Some(species);
        }
        let index = self.index_of(species)?;
        let removed = self.records.remove(index);
        self.active.retain(|id| *id != species);
        let parent = removed.parent;
        let children = removed.children.clone();
        if let Some(parent_id) = parent {
            if let Some(parent_record) = self.get_mut(parent_id) {
                parent_record.children.retain(|id| *id != species);
                for child in &children {
                    if !parent_record.children.contains(child) {
                        parent_record.children.push(*child);
                    }
                }
            }
            if let Some(parent_root) = self.get(parent_id).map(|record| record.root) {
                for child in &children {
                    if let Some(child_record) = self.get_mut(*child) {
                        child_record.parent = parent;
                        child_record.root = parent_root;
                    }
                }
            }
        } else {
            for child in &children {
                if let Some(child_record) = self.get_mut(*child) {
                    child_record.parent = None;
                    child_record.root = *child;
                }
            }
        }
        // `speciesToMergeInto ?? parent ?? children.OrderByDescending(nPointsPresent).FirstOrDefault()`: the
        // first of the children with the most points.
        let largest_child = || {
            let mut best: Option<(SpeciesId, u32)> = None;
            for child in &children {
                let points = self
                    .get(*child)
                    .map_or(0, |record| record.stats.n_points_present());
                if best.is_none_or(|(_, most)| points > most) {
                    best = Some((*child, points));
                }
            }
            best.map(|(id, _)| id)
        };
        let target = merge_into.or(parent).or_else(largest_child);
        let target_id = target?;
        let target_record = self.get_mut(target_id)?;
        target_record.stats.log.incorporate(&removed.stats.log);
        target_record.stats.count += removed.stats.count;
        target_record.stats.energy += removed.stats.energy;
        self.merges.push((species, target_id));
        Some(target_id)
    }

    /// The prunes since the last call as `(removed, survivor)`: the world moves the bibites and eggs of the
    /// removed species to the survivor (`PruneSpeciesFromTree` reassigns them).
    pub fn take_merges(&mut self) -> Vec<(SpeciesId, SpeciesId)> {
        std::mem::take(&mut self.merges)
    }

    /// Converts the registry to the plain record shape used by `bibites-io`.
    pub fn to_records(&self) -> LineageRecordIo {
        LineageRecordIo {
            recorded_species: self.records.iter().map(species_to_io).collect(),
            active_species: self.active.iter().map(|id| id.value()).collect(),
            node_max_innovation: self.node_max_innovation,
            connection_max_innovation: self.connection_max_innovation,
            next_species_id: self.next_id,
            max_energy: self.max_energy,
            max_count: self.max_count,
            log_counter: self.log_counter,
            n_records: self.n_records,
        }
    }

    /// Rebuilds a registry from plain records (`speciesData.json` / save species
    /// section). `period` initialises template brains' node state (distance only
    /// reads immutable fields, so it does not affect speciation).
    pub fn from_records(record: LineageRecordIo, period: f32) -> Result<Self> {
        let mut registry = Self::new();
        registry.node_max_innovation = record.node_max_innovation;
        registry.connection_max_innovation = record.connection_max_innovation;
        registry.next_id = record.next_species_id.max(1);
        registry.max_energy = record.max_energy;
        registry.max_count = record.max_count;
        registry.log_counter = record.log_counter;
        registry.n_records = record.n_records;
        for io in record.recorded_species {
            let genes = io.template.genes;
            let template = TemplateSnapshot::from_records(
                genes,
                &io.template.nodes,
                &io.template.synapses,
                period,
            )
            .map_err(|error| {
                Error::invalid_data(format!("species {} template: {error}", io.species_id))
            })?;
            let id = SpeciesId::new(io.species_id);
            registry.next_id = registry.next_id.max(io.species_id.saturating_add(1));
            registry.records.push(SpeciesRecord {
                id,
                time_creation: io.time_creation,
                generation_of_first_specimen: io.generation_of_first_specimen,
                favorite: io.favorite,
                generic_name: io.generic_name,
                specific_name: io.specific_name,
                description: io.description,
                parent: io.parent_id.map(SpeciesId::new),
                root: id,
                children: Vec::new(),
                template,
                stats: SpeciesStats::default(),
            });
        }
        for io in record.active_species {
            let id = SpeciesId::new(io);
            if registry.get(id).is_some() {
                registry.active.push(id);
            }
        }
        // Re-link the lineage tree by parentID (game LoadState).
        let ids: Vec<SpeciesId> = registry.records.iter().map(|record| record.id).collect();
        for id in ids {
            let parent_id = registry.get(id).and_then(|record| record.parent);
            if let Some(parent_id) = parent_id {
                if let Some(parent_index) = registry.index_of(parent_id) {
                    if !registry.records[parent_index].children.contains(&id) {
                        registry.records[parent_index].children.push(id);
                    }
                    let root = registry.records[parent_index].root;
                    if let Some(record) = registry.get_mut(id) {
                        record.root = root;
                    }
                } else if let Some(record) = registry.get_mut(id) {
                    record.parent = None;
                    record.root = id;
                }
            }
        }
        Ok(registry)
    }

    // -----------------------------------------------------------------------
    // Name generation (`Here/Utility/SpeciesNameGenerator.cs`)
    // -----------------------------------------------------------------------

    /// Game `GenerateNewName`: inherit the parent genus unless five consecutive
    /// ancestors share it (the parent counts as the first: `parentSpecies` is where the game starts), then generate a fresh specific epithet.
    fn generate_new_name(&mut self, parent_generic: &str, parent: SpeciesId) -> (String, String) {
        let mut generic = parent_generic.to_string();
        if generic.is_empty() {
            generic = self.random_unused_genus();
        } else {
            let mut same = 0;
            let mut current = Some(parent);
            while let Some(id) = current {
                let ancestor_generic = self
                    .get(id)
                    .map(|record| record.generic_name.clone())
                    .unwrap_or_default();
                if ancestor_generic != generic || same > 5 {
                    break;
                }
                same += 1;
                current = self.parent_of(id);
            }
            if same >= 5 {
                generic = self.random_unused_genus();
            }
        }
        let specific = self.random_unused_specific(Some(&generic));
        (generic, specific)
    }

    /// The words a name is drawn from now: this registry's own pool, else the player's (`species_names::name_pool`), else (`None`) the game's list.
    fn name_pool(&self) -> Option<std::sync::Arc<crate::species_names::NamePool>> {
        self.own_name_pool.clone().or_else(crate::species_names::name_pool)
    }

    /// Makes this registry name its species from `pool` (and not from the process-wide one) from now on; `None` takes its own pool away.
    pub fn set_own_name_pool(&mut self, pool: Option<crate::species_names::NamePool>) {
        self.own_name_pool = pool.map(std::sync::Arc::new);
    }

    /// How many different words the name generator can draw from (the loops that look for an unused name count by it).
    fn name_reach(&self) -> usize {
        self.name_pool().map_or(LATIN_NAMES.len(), |pool| pool.reach())
    }

    fn random_unused_genus(&mut self) -> String {
        let mut attempts = self.name_reach() * 2;
        loop {
            let name = self.weighted_random_patron(4.0);
            if !self
                .records
                .iter()
                .any(|record| record.generic_name == name)
                || attempts == 0
            {
                // `name.Capitalize()` lowers the whole word first, then raises its first letter.
                return capitalize(&name.to_lowercase());
            }
            attempts -= 1;
        }
    }

    fn random_unused_specific(&mut self, genus: Option<&str>) -> String {
        let mut attempts = self.name_reach() / 2;
        loop {
            let name = self.weighted_random_patron(1.5).to_lowercase();
            let used = self.records.iter().any(|record| {
                record.specific_name == name
                    && genus.is_none_or(|genus| record.generic_name == genus)
            });
            if !used || attempts == 0 {
                return name;
            }
            attempts -= 1;
        }
    }

    /// Game `WeightedRandomPatron`: `1 - U^weight` picks a word of the game's list
    /// (`latinNames`: the supporters' Latin names, the biggest pledges first). The
    /// number is near 1 for most draws (U^4 for a genus, U^1.5 for an epithet), so the
    /// words at the END of the list are the common ones and the first are rare.
    fn weighted_random_patron(&mut self, weight: f32) -> String {
        // One draw of the name stream, as ever; RUSTibites Improved's pool of the player's words (when there is one) turns it into a word of its own.
        let draw = self.name_rng.f32();
        if let Some(pool) = self.name_pool() {
            return pool.pick(draw, weight);
        }
        let sample = 1.0 - draw.powf(weight);
        let len = LATIN_NAMES.len();
        let index = ((len as f32 * sample).floor() as usize).min(len - 1);
        LATIN_NAMES[index].to_string()
    }

    fn index_of(&self, id: SpeciesId) -> Option<usize> {
        self.records.iter().position(|record| record.id == id)
    }
}

fn species_to_io(record: &SpeciesRecord) -> SpeciesRecordIo {
    SpeciesRecordIo {
        species_id: record.id.value(),
        time_creation: record.time_creation,
        generation_of_first_specimen: record.generation_of_first_specimen,
        favorite: record.favorite,
        generic_name: record.generic_name.clone(),
        specific_name: record.specific_name.clone(),
        description: record.description.clone(),
        parent_id: record.parent.map(|id| id.value()),
        template: SpeciesTemplateRecord {
            version: None,
            nodes: record.template.brain.to_node_records(),
            synapses: record.template.brain.to_synapse_records(),
            genes: *record.template.genome.values(),
        },
    }
}

/// Splits a stored species name into genus/epithet like `Species.cs:94-98`.
fn split_species_name(name: &str) -> (String, String) {
    let mut parts = name.split(' ');
    let generic = parts.next().unwrap_or_default();
    let specific = parts.next().unwrap_or(generic);
    // `Species(BibiteTemplate)`: `array[0].Capitalize()` (which lowers the rest) and `array[1].ToLower()`.
    (capitalize(&generic.to_lowercase()), specific.to_lowercase())
}

/// `StringExtend.ToSpeciesFormat`: the first word capitalised and the second (the first again when there is only one) in lower case.
#[must_use]
pub fn to_species_format(name: &str) -> String {
    let (generic, specific) = split_species_name(name);
    format!("{generic} {specific}")
}

/// `.Capitalize()` used by the name generator.
fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The gene-distance denominator for a gene, i.e. the editor maximum
/// (`docs/spec/01-brain-genetics.md` ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â§9.1). Exposed so `bibites-io` can share
/// validation of `speciesData.json` templates.
pub fn gene_distance_denominator(gene: Gene) -> f32 {
    gene.def().max
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::Genome;

    pub(super) fn settings() -> SimSettings {
        SimSettings::game_defaults()
    }

    pub(super) fn pair() -> (Genome, Brain) {
        (Genome::starter(), Brain::starter())
    }

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
    }

    pub(super) fn outlier(genome: &mut Genome) {
        // Move several genes far outside the species span (~1.5).
        genome.set_gene(Gene::LayTime, 45.0);
        genome.set_gene(Gene::ViewRadius, 10.0);
        genome.set_gene(Gene::Diet, 1.0);
        genome.set_gene(Gene::StomachWAG, 8.0);
        genome.set_gene(Gene::ColorR, 1.0);
    }

    #[test]
    fn template_snapshot_distance_is_zero_for_self() {
        let (genome, brain) = pair();
        let snapshot = TemplateSnapshot::from_live(&genome, &brain);
        let s = settings();
        approx(snapshot.distance(&genome, &brain, &s), 0.0);
        approx(snapshot.gene_distance(&genome), 0.0);
        approx(snapshot.brain_distance(&brain, &s), 0.0);
    }

    #[test]
    fn gene_distance_uses_editor_max_denominator() {
        let snapshot = TemplateSnapshot::from_live(&Genome::starter(), &Brain::starter());
        let mut genome = Genome::starter();
        genome.set_gene(Gene::LayTime, 25.0);
        approx(snapshot.gene_distance(&genome), 10.0 / 51.0);
        let mut genome = Genome::starter();
        genome.set_gene(Gene::StomachWAG, 4.0);
        approx(snapshot.gene_distance(&genome), 2.0 / 9.0);
    }

    #[test]
    fn assign_creates_root_then_stays() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let mut meta = GenomeMeta::default();
        let id = registry.assign(&genome, &brain, &mut meta, &s);
        assert_eq!(meta.species, Some(id));
        assert_eq!(registry.species_count(), 1);
        assert_eq!(registry.active_species_count(), 1);
        assert_eq!(registry.get(id).unwrap().parent, None);
        let mut meta2 = GenomeMeta::default();
        let id2 = registry.assign(&genome, &brain, &mut meta2, &s);
        assert_eq!(id2, id);
        assert_eq!(meta2.species, Some(id));
        assert_eq!(registry.species_count(), 1);
    }

    #[test]
    fn lineage_traversal_and_extinction_helpers() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let mut meta = GenomeMeta::default();
        let root = registry.assign(&genome, &brain, &mut meta, &s);
        let mut stray = Genome::starter();
        outlier(&mut stray);
        let mut stray_meta = GenomeMeta {
            species: Some(root),
            ..GenomeMeta::default()
        };
        let _ = registry.assign(&stray, &brain, &mut stray_meta, &s);
        let child = registry.assign(&stray, &brain, &mut stray_meta, &s);
        assert_ne!(child, root);
        registry.on_birth(root, root, 0);
        assert_eq!(registry.roots(), vec![root]);
        assert_eq!(registry.descendants_of(root), vec![child]);
        assert!(registry.descendants_of(child).is_empty());
        let edges = registry.lineage_edges();
        assert!(edges.contains(&(root, None)));
        assert!(edges.contains(&(child, Some(root))));
        assert!(!registry.get(child).unwrap().is_extinct());
        registry.on_death(child);
        assert_eq!(registry.get(child).unwrap().stats.count, 0);
        assert!(registry.get(child).unwrap().is_extinct());
        assert_eq!(registry.extinct_species(), vec![child]);
    }

    #[test]
    fn reconcile_live_counts_drops_extinct_children_and_recounts_root() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let mut meta = GenomeMeta::default();
        let root = registry.assign(&genome, &brain, &mut meta, &s);
        let mut stray = Genome::starter();
        outlier(&mut stray);
        let mut stray_meta = GenomeMeta {
            species: Some(root),
            ..GenomeMeta::default()
        };
        let _ = registry.assign(&stray, &brain, &mut stray_meta, &s);
        let child = registry.assign(&stray, &brain, &mut stray_meta, &s);
        assert_ne!(child, root);
        registry.on_birth(root, root, 0);
        registry.on_birth(root, root, 0);
        registry.on_birth(root, root, 0);
        registry.on_birth(child, root, 0);
        assert_eq!(registry.get(root).unwrap().stats.count, 3);
        assert_eq!(registry.get(child).unwrap().stats.count, 2);
        let removed = registry.reconcile_live_counts([root, root, root]);
        assert!(removed >= 1, "extinct child should leave the active list");
        assert_eq!(registry.get(root).unwrap().stats.count, 3);
        assert_eq!(registry.get(child).unwrap().stats.count, 0);
        assert_eq!(registry.active_species(), &[root]);
    }

    #[test]
    fn reconcile_long_membership_stream_matches_live_counts() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let mut meta = GenomeMeta::default();
        let root = registry.assign(&genome, &brain, &mut meta, &s);
        let mut stray = Genome::starter();
        outlier(&mut stray);
        let mut stray_meta = GenomeMeta {
            species: Some(root),
            ..GenomeMeta::default()
        };
        let _ = registry.assign(&stray, &brain, &mut stray_meta, &s);
        let child = registry.assign(&stray, &brain, &mut stray_meta, &s);
        let mut live = vec![root, root, child, root];
        let mut state = 0x1234_5678_u64;
        for _step in 0..2_000 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            match (state >> 62) % 4 {
                0 => {
                    let species = if (state >> 3) & 1 == 0 { root } else { child };
                    registry.on_birth(species, root, 0);
                    live.push(species);
                }
                1 if live.len() > 1 => {
                    // Swallowed/unborn death: the body record is removed before
                    // event processing, so the incremental on_death is skipped.
                    let index = ((state >> 8) as usize) % live.len();
                    live.remove(index);
                }
                2 if live.len() > 1 => {
                    // Ordinary death still reaches on_death.
                    let index = ((state >> 16) as usize) % live.len();
                    let species = live.remove(index);
                    registry.on_death(species);
                }
                _ => {}
            }
            registry.reconcile_live_counts(live.iter().copied());
            for id in registry.active_species() {
                let expected = live.iter().filter(|species| **species == *id).count() as u32;
                assert_eq!(registry.get(*id).unwrap().stats.count, expected);
            }
            assert!(
                registry
                    .active_species()
                    .iter()
                    .all(|id| { live.iter().any(|species| species == id) })
            );
        }
        registry.reconcile_live_counts(live.iter().copied());
        for record in registry.records() {
            let expected = live.iter().filter(|species| **species == record.id).count() as u32;
            assert_eq!(record.stats.count, expected);
        }
        assert!(live.iter().all(|id| registry.active_species().contains(id)));
    }

    #[test]
    fn assign_is_deterministic_for_same_inputs() {
        let s = settings();
        let mut a = SpeciesRegistry::new();
        let mut b = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let mut meta_a = GenomeMeta::default();
        let mut meta_b = GenomeMeta::default();
        assert_eq!(
            a.assign(&genome, &brain, &mut meta_a, &s),
            b.assign(&genome, &brain, &mut meta_b, &s)
        );
        let names_a: Vec<String> = a.records().iter().map(|r| r.name()).collect();
        let names_b: Vec<String> = b.records().iter().map(|r| r.name()).collect();
        assert_eq!(names_a, names_b);
    }

    #[test]
    fn two_step_mutant_rule_founds_new_species() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let mut meta = GenomeMeta::default();
        let root = registry.assign(&genome, &brain, &mut meta, &s);

        let mut stray_genome = genome.clone();
        outlier(&mut stray_genome);
        let mut stray_meta = GenomeMeta {
            species: Some(root),
            ..Default::default()
        };
        // First outlier: flagged, stays.
        assert_eq!(
            registry.assign(&stray_genome, &brain, &mut stray_meta, &s),
            root
        );
        assert!(stray_meta.is_mutant);
        // Second outlier: founds a child species.
        let child = registry.assign(&stray_genome, &brain, &mut stray_meta, &s);
        assert_ne!(child, root);
        assert!(!stray_meta.is_mutant);
        assert_eq!(stray_meta.species, Some(child));
        let child_record = registry.get(child).unwrap();
        assert_eq!(child_record.parent, Some(root));
        assert_eq!(child_record.root, root);
        assert!(registry.get(root).unwrap().children.contains(&child));
        assert_eq!(registry.species_count(), 2);
    }

    #[test]
    fn assign_stays_within_span() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let mut meta = GenomeMeta::default();
        let root = registry.assign(&genome, &brain, &mut meta, &s);
        let mut close = genome.clone();
        close.set_gene(Gene::LayTime, 15.2);
        let mut close_meta = GenomeMeta {
            species: Some(root),
            ..Default::default()
        };
        assert_eq!(registry.assign(&close, &brain, &mut close_meta, &s), root);
        assert!(!close_meta.is_mutant);
    }

    #[test]
    fn assign_adopts_child_species_within_half_span() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let mut meta = GenomeMeta::default();
        let root = registry.assign(&genome, &brain, &mut meta, &s);
        // A child species whose template is far from the root.
        let mut stray = genome.clone();
        outlier(&mut stray);
        let mut stray_meta = GenomeMeta::default();
        let child = registry.create_species_for_test(&stray, &brain, &mut stray_meta, Some(root));
        registry.on_birth(child, root, 1);
        // The stray individual is > span from the root, so the child-species
        // adoption path (distance <= span/2) must pick the child.
        let mut close_meta = GenomeMeta {
            species: Some(root),
            ..Default::default()
        };
        assert_eq!(registry.assign(&stray, &brain, &mut close_meta, &s), child);
        assert!(!close_meta.is_mutant);
    }

    #[test]
    fn used_span_scales_past_twenty_active_species() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let mut meta = GenomeMeta::default();
        registry.assign(&genome, &brain, &mut meta, &s);
        approx(registry.used_span(&s), 1.5);
        for _ in 0..25 {
            let id = SpeciesId::new(registry.next_id);
            registry.next_id += 1;
            registry.records.push(SpeciesRecord {
                id,
                time_creation: 0.0,
                generation_of_first_specimen: 0,
                favorite: false,
                generic_name: "Genus".to_string(),
                specific_name: "species".to_string(),
                description: String::new(),
                parent: None,
                root: id,
                children: Vec::new(),
                template: TemplateSnapshot::from_live(&genome, &brain),
                stats: SpeciesStats::default(),
            });
            registry.active.push(id);
            registry.update_span_count();
        }
        approx(registry.used_span(&s), 1.5 + 6.0 * 0.05);
    }

    #[test]
    fn an_egg_takes_the_first_species_within_the_span_else_the_nearest() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        assert_eq!(registry.check_species_of_egg(&genome, &brain, &s), None);
        let near = registry.add_template_species("Zza zzone", "", &genome, &brain);
        let mut far = genome.clone();
        outlier(&mut far);
        let other = registry.add_template_species("Zzb zztwo", "", &far, &brain);
        assert_eq!(registry.check_species_of_egg(&genome, &brain, &s), Some(near));
        assert_eq!(registry.check_species_of_egg(&far, &brain, &s), Some(other));
    }

    #[test]
    fn a_template_finds_its_recorded_species_or_founds_one() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let first = registry.find_species_from_template("Zza zzone", "", &genome, &brain, false);
        assert_eq!(registry.find_species_from_template("Zza zzone", "", &genome, &brain, false), first);
        registry.active.clear();
        assert_eq!(registry.find_species_from_template("Zza zzone", "", &genome, &brain, true), first);
        assert!(registry.active.is_empty());
        assert_eq!(registry.find_species_from_template("Zza zzone", "", &genome, &brain, false), first);
        assert_eq!(registry.active, vec![first]);
        let _ = s;
    }

    #[test]
    fn birth_and_death_track_counts() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let mut meta = GenomeMeta::default();
        let id = registry.assign(&genome, &brain, &mut meta, &s);
        registry.on_birth(id, id, 2);
        registry.on_birth(id, id, 2);
        assert_eq!(registry.get(id).unwrap().stats.count, 2);
        registry.on_death(id);
        assert_eq!(registry.get(id).unwrap().stats.count, 1);
        registry.add_energy(id, 12.5);
        approx(registry.get(id).unwrap().stats.energy, 12.5);
    }

    #[test]
    fn log_stats_pushes_history_and_keeps_roots() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let mut meta = GenomeMeta::default();
        let id = registry.assign(&genome, &brain, &mut meta, &s);
        registry.on_birth(id, id, 1);
        registry.set_log_period_seconds(1.0);
        registry.tick_stats(1.0);
        let record = registry.get(id).unwrap();
        assert_eq!(record.stats.n_points_present(), 1);
        assert_eq!(record.stats.total_index_of_apparition(), 0);
        assert_eq!(record.stats.total_index_of_disappearance(), -1, "alive");
        assert_eq!(record.stats.log.point(0, 0).count, 1);
        registry.on_death(id);
        for _ in 0..4 {
            registry.tick_stats(1.0);
        }
        assert!(!registry.is_active(id));
        assert!(registry.get(id).is_some(), "root species are never pruned");
    }

    #[test]
    fn non_root_stale_species_are_pruned() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let mut meta = GenomeMeta::default();
        let root = registry.assign(&genome, &brain, &mut meta, &s);
        let mut child_meta = GenomeMeta::default();
        let child = registry.create_species_for_test(&genome, &brain, &mut child_meta, Some(root));
        // The root has a member, as a root has in a running world (a kept species without any point is
        // pruned too, which `a_kept_species_with_no_point_is_pruned` shows).
        registry.on_birth(root, root, 0);
        registry.on_birth(child, root, 1);
        registry.set_prune_point_limit(2);
        registry.on_death(child);
        // `LogSpeciesInfo` cleans up first and only then drops empty species
        // from the active list, so the first log deactivates the extinct child
        // and the next one prunes it.
        registry.log_stats();
        assert!(registry.get(child).is_some());
        assert!(!registry.is_active(child));
        registry.log_stats();
        assert!(registry.get(child).is_none());
        assert!(registry.get(root).is_some());
    }

    #[test]
    fn the_log_follows_a_species_as_it_lives_and_dies() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let genome = Genome::starter();
        let brain = Brain::starter();
        let mut meta = GenomeMeta::default();
        let id = registry.assign(&genome, &brain, &mut meta, &s);
        // A present species (has a member) is present in every point it was logged in.
        registry.on_birth(id, id, 0);
        for _ in 0..20 {
            registry.log_stats();
        }
        let log = registry.log(id).expect("log");
        assert!(log.alive());
        assert_eq!(
            log.total_index_of_apparition(),
            15,
            "twenty logs: twelve points and four compressed pairs"
        );
        let node = registry
            .lineage_tree()
            .nodes
            .into_iter()
            .find(|node| node.id == id)
            .unwrap();
        assert_eq!(node.total_index_of_disappearance, -1);
        assert_eq!(node.n_points_present, log.n_points_present());
        // It dies: its disappearance moves off the newest point.
        registry.on_death(id);
        registry.log_stats();
        registry.log_stats();
        let log = registry.log(id).expect("a root species is kept");
        assert!(!log.alive());
        assert_eq!(log.total_index_of_disappearance(), 1);
    }

    #[test]
    fn a_kept_species_with_few_points_absorbs_its_siblings_as_the_game_does() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = (Genome::starter(), Brain::starter());
        let mut meta = GenomeMeta::default();
        let root = registry.assign(&genome, &brain, &mut meta, &s);
        let mut a_meta = GenomeMeta::default();
        let favourite = registry.create_species_for_test(&genome, &brain, &mut a_meta, Some(root));
        let mut b_meta = GenomeMeta::default();
        let sibling = registry.create_species_for_test(&genome, &brain, &mut b_meta, Some(root));
        registry.set_favorite(favourite, true);
        registry.set_prune_point_limit(3);
        registry.on_birth(root, root, 0);
        registry.on_birth(favourite, root, 0);
        registry.on_birth(sibling, root, 0);
        // The favourite lives for two logs, dies, and is left with two points (the limit is three).
        registry.log_stats();
        registry.log_stats();
        registry.on_death(favourite);
        registry.log_stats();
        assert_eq!(registry.get(favourite).unwrap().stats.n_points_present(), 2);
        assert!(registry.take_merges().is_empty());
        // The next log: the favourite is not active any more and has too few points, so the species
        // around it are merged into it, the least relevant first, until it has more (the game does this
        // to species that are still alive too).
        registry.log_stats();
        assert!(
            registry.get(sibling).is_none(),
            "the sibling was merged away"
        );
        assert_eq!(registry.take_merges(), vec![(sibling, favourite)]);
        let favourite_record = registry.get(favourite).expect("a favourite stays");
        assert_eq!(
            favourite_record.stats.count, 1,
            "the sibling's member came with it"
        );
        assert!(favourite_record.stats.n_points_present() > 3);
        assert!(
            favourite_record.stats.log.alive(),
            "its history now reaches the newest point"
        );
        assert!(registry.get(root).is_some());
    }

    #[test]
    fn a_kept_species_with_no_point_is_pruned() {
        // `SpeciesTreeCleanup`: even a root or a favourite that is not active and still has no present point
        // after absorbing its neighbours is pruned.
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let root = registry.add_template_species("Never lived", "", &genome, &brain);
        registry.set_prune_point_limit(2);
        // Its first log leaves the active list (no member), the second prunes it.
        registry.log_stats();
        assert!(registry.get(root).is_some());
        registry.log_stats();
        assert!(registry.get(root).is_none());
    }

    #[test]
    fn favorite_species_are_kept() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let mut meta = GenomeMeta::default();
        let root = registry.assign(&genome, &brain, &mut meta, &s);
        let mut child_meta = GenomeMeta::default();
        let child = registry.create_species_for_test(&genome, &brain, &mut child_meta, Some(root));
        registry.set_favorite(child, true);
        registry.set_prune_point_limit(2);
        registry.on_death(child);
        registry.log_stats();
        assert!(registry.get(child).is_some());
    }

    #[test]
    fn lineage_accessors() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let mut meta = GenomeMeta::default();
        let root = registry.assign(&genome, &brain, &mut meta, &s);
        let mut a_meta = GenomeMeta::default();
        let a = registry.create_species_for_test(&genome, &brain, &mut a_meta, Some(root));
        let mut b_meta = GenomeMeta::default();
        let b = registry.create_species_for_test(&genome, &brain, &mut b_meta, Some(a));
        assert_eq!(registry.parent_of(a), Some(root));
        assert_eq!(registry.parent_of(b), Some(a));
        assert_eq!(registry.get(b).unwrap().root, root);
        assert_eq!(registry.ancestors(b), vec![a, root]);
        assert_eq!(registry.descendant_species_count(root), 3);
        assert_eq!(registry.get(a).unwrap().children, vec![b]);
    }

    #[test]
    fn records_roundtrip_preserves_lineage() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let mut meta = GenomeMeta::default();
        let root = registry.assign(&genome, &brain, &mut meta, &s);
        let mut child_meta = GenomeMeta::default();
        let child = registry.create_species_for_test(&genome, &brain, &mut child_meta, Some(root));
        registry.on_birth(root, root, 1);
        registry.on_birth(child, root, 2);
        registry.merge_brain_innovations(80, 12);
        registry.next_id = 50;
        let before = registry.distance_to(root, &genome, &brain, &s);
        let io = registry.to_records();
        let restored = SpeciesRegistry::from_records(io, s.brain_period()).expect("restore");
        assert_eq!(restored.species_count(), 2);
        assert_eq!(restored.parent_of(child), Some(root));
        assert_eq!(restored.get(child).unwrap().root, root);
        assert_eq!(restored.active_species_count(), 2);
        assert_eq!(restored.node_max_innovation(), 80);
        assert_eq!(restored.connection_max_innovation(), 12);
        assert_eq!(restored.next_species_id(), 50);
        approx(restored.distance_to(root, &genome, &brain, &s), before);
    }

    #[test]
    fn from_records_relinks_missing_parent_as_root() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let mut meta = GenomeMeta::default();
        registry.assign(&genome, &brain, &mut meta, &s);
        let mut io = registry.to_records();
        io.recorded_species[0].parent_id = Some(999);
        let restored = SpeciesRegistry::from_records(io, s.brain_period()).expect("restore");
        let record = &restored.records()[0];
        assert_eq!(record.parent, None);
        assert_eq!(record.root, record.id);
    }

    #[test]
    fn add_template_species_parses_names() {
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let id = registry.add_template_species("Whale whale", "desc", &genome, &brain);
        let record = registry.get(id).unwrap();
        assert_eq!(record.generic_name, "Whale");
        assert_eq!(record.specific_name, "whale");
        assert_eq!(record.name(), "Whale whale");
        assert_eq!(record.description, "desc");
        assert_eq!(registry.active_species_count(), 1);
    }

    #[test]
    fn species_names_follow_the_games_format() {
        for good in ["Whale whale", "Ab cd", "Renata aguilera"] {
            assert!(SpeciesRegistry::is_valid_species_name(good), "{good}");
        }
        for bad in [
            "",
            "whale whale",
            "Whale Whale",
            "Whale",
            "Whale  whale",
            "Whale whale ",
            " Whale whale",
            "Whale whale whale",
            "W whale",
            "Whale w4le",
            "Whal3 whale",
            "Whale whal\u{e9}",
        ] {
            assert!(!SpeciesRegistry::is_valid_species_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_species_is_renamed_and_annotated_and_a_bad_name_changes_nothing() {
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let id = registry.add_template_species("Whale whale", "", &genome, &brain);
        assert!(registry.rename(id, "Orca grandis"));
        let record = registry.get(id).unwrap();
        assert_eq!(
            (record.generic_name.as_str(), record.specific_name.as_str()),
            ("Orca", "grandis")
        );
        assert_eq!(record.name(), "Orca grandis");
        assert!(!registry.rename(id, "orca grandis"));
        assert!(!registry.rename(id, "Orca"));
        assert_eq!(registry.get(id).unwrap().name(), "Orca grandis");
        assert!(!registry.rename(SpeciesId::new(99_999), "Orca grandis"));
        registry.set_description(id, "A big one.\nLikes plants.");
        assert_eq!(
            registry.get(id).unwrap().description,
            "A big one.\nLikes plants."
        );
    }

    #[test]
    fn prune_needs_an_extinct_species_that_is_not_kept_and_descendants_are_counted() {
        let mut registry = SpeciesRegistry::new();
        let (genome, brain) = pair();
        let mut meta = GenomeMeta::default();
        let root = registry.add_template_species("Root species", "", &genome, &brain);
        let child = registry.create_species_for_test(&genome, &brain, &mut meta, Some(root));
        let mut meta = GenomeMeta::default();
        let grandchild = registry.create_species_for_test(&genome, &brain, &mut meta, Some(child));
        for (id, count) in [(root, 2), (child, 3), (grandchild, 0)] {
            registry.get_mut(id).unwrap().stats.count = count;
        }
        // A root is always kept; a species with members is not extinct.
        assert!(!registry.can_prune(root));
        assert!(!registry.can_prune(child));
        // The grandchild has no member, but its log does not show it gone yet (`totalIndexOfDisappearance`).
        assert!(!registry.can_prune(grandchild));
        // It lived for two logs and has been gone for two: it can go, unless it is a favourite.
        for point in [
            SpeciesDataPoint::from_counts(1, 5.0),
            SpeciesDataPoint::from_counts(1, 5.0),
            SpeciesDataPoint::default(),
            SpeciesDataPoint::default(),
        ] {
            registry.get_mut(grandchild).unwrap().stats.log.push(point);
        }
        assert!(registry.can_prune(grandchild));
        registry.set_favorite(grandchild, true);
        assert!(!registry.can_prune(grandchild));
        registry.set_favorite(grandchild, false);
        // Members of a species and of everything below it.
        assert_eq!(registry.alive_descendant_count(root), 5);
        assert_eq!(registry.alive_descendant_count(child), 3);
        assert_eq!(registry.alive_descendant_count(grandchild), 0);
        assert_eq!(registry.alive_descendant_count(SpeciesId::new(99_999)), 0);
        // Pruning the grandchild leaves the others in place.
        assert!(registry.prune_from_tree(grandchild, None).is_some());
        assert!(registry.get(grandchild).is_none());
        assert_eq!(registry.alive_descendant_count(root), 5);
    }

    #[test]
    fn the_used_span_follows_the_settings_at_once() {
        // The Lineage panel's sliders change `speciesGeneticSpan` and its scaling while the species count stays.
        let mut registry = SpeciesRegistry::new();
        let mut s = settings();
        let (genome, brain) = pair();
        for n in 0..25 {
            registry.add_template_species(
                &format!("Zz{} zzspecies", "a".repeat(n + 1)),
                "",
                &genome,
                &brain,
            );
        }
        let before = registry.used_span(&s);
        s.independents.species_genetic_span += 1.0;
        approx(registry.used_span(&s), before + 1.0);
        s.independents.species_span_scaling_factor += 0.1;
        approx(registry.used_span(&s), before + 1.0 + 5.0 * 0.1);
    }

    #[test]
    fn the_read_only_span_matches_the_cached_one_and_grows_with_the_active_species() {
        let mut registry = SpeciesRegistry::new();
        let s = settings();
        let (genome, brain) = pair();
        let base = s.independents.species_genetic_span;
        assert_eq!(registry.current_used_span(&s), base);
        for n in 0..25 {
            registry.add_template_species(
                &format!("Zz{} zzspecies", "a".repeat(n + 1)),
                "",
                &genome,
                &brain,
            );
        }
        let expected = base
            + (registry.active_species_count().saturating_sub(20)) as f32
                * s.independents.species_span_scaling_factor;
        assert!((registry.current_used_span(&s) - expected).abs() < 1e-6);
        assert!((registry.used_span(&s) - registry.current_used_span(&s)).abs() < 1e-6);
    }

    #[test]
    fn the_span_keeps_counting_the_species_that_died_out_until_it_is_refreshed() {
        // `LogSpeciesInfo` drops the species without members from the active list and does not call `UpdateEffectiveSpeciesSpan`: the span the
        // births are judged by keeps the count it was computed with, until a species is created (or found from a template) or
        // `SpeciesPanel.OpenPanel` (the Lineage panel opening) refreshes it.
        let mut registry = SpeciesRegistry::new();
        let s = settings();
        let (genome, brain) = pair();
        let ids: Vec<SpeciesId> = (0..25)
            .map(|n| registry.add_template_species(&format!("Zz{} zzspecies", "a".repeat(n + 1)), "", &genome, &brain))
            .collect();
        let base = s.independents.species_genetic_span;
        let scaling = s.independents.species_span_scaling_factor;
        approx(registry.current_used_span(&s), base + 5.0 * scaling);
        // 22 of the 25 have a member; the three that have none leave the active list at the log.
        registry.reconcile_counts_by_species(ids.iter().take(22).map(|id| (*id, 1_u32)));
        registry.log_stats_with(&[]);
        assert_eq!(registry.active_species_count(), 22);
        approx(registry.current_used_span(&s), base + 5.0 * scaling);
        approx(registry.used_span(&s), base + 5.0 * scaling);
        registry.refresh_span();
        approx(registry.current_used_span(&s), base + 2.0 * scaling);
        approx(registry.used_span(&s), base + 2.0 * scaling);
    }

    #[test]
    fn gene_distance_denominator_helper_matches_table() {
        assert_eq!(gene_distance_denominator(Gene::LayTime), 51.0);
        approx(gene_distance_denominator(Gene::StomachWAG), 9.0);
    }

    #[test]
    fn a_genus_is_kept_for_five_species_of_a_line_and_the_sixth_gets_a_new_one() {
        // `SpeciesNameGenerator.GenerateNewName` counts from `species.parentSpecies` (the parent, which already shares the genus the child copied) up: five of
        // them in a row (the parent and four ancestors) make the child take a new genus. So a root and its next four descendants share a genus, the sixth does not.
        let mut registry = SpeciesRegistry::with_seed(11);
        let (genome, brain) = pair();
        let meta = GenomeMeta::default();
        let mut line = vec![registry.create_species(&genome, &brain, &meta, None)];
        for _ in 0..7 {
            let parent = *line.last().expect("the line");
            line.push(registry.create_species(&genome, &brain, &meta, Some(parent)));
        }
        let genus: Vec<String> = line.iter().map(|id| registry.get(*id).expect("record").generic_name.clone()).collect();
        assert!(genus[..5].iter().all(|name| *name == genus[0]), "the root and its four descendants share a genus: {genus:?}");
        assert_ne!(genus[5], genus[4], "the sixth species of the line takes a new genus: {genus:?}");
        // The new genus starts a count of its own: the species that follow keep it.
        assert!(genus[5..].iter().all(|name| *name == genus[5]), "{genus:?}");
    }

    #[test]
    fn name_generation_is_deterministic_and_unique_genus() {
        let mut a = SpeciesRegistry::with_seed(42);
        let mut b = SpeciesRegistry::with_seed(42);
        let (genome, brain) = pair();
        let mut meta = GenomeMeta::default();
        a.assign(&genome, &brain, &mut meta, &settings());
        let mut meta = GenomeMeta::default();
        b.assign(&genome, &brain, &mut meta, &settings());
        assert_eq!(a.records()[0].name(), b.records()[0].name());
        assert!(!a.records()[0].generic_name.is_empty());
        assert!(!a.records()[0].specific_name.is_empty());
    }

    #[test]
    fn a_registry_with_the_players_own_words_names_its_species_from_them_and_one_without_names_them_as_the_game_does() {
        use crate::species_names::{NameMode, NamePool};
        let mine = ["fluffy", "bubbles", "mossy", "tiny"];
        let pool = |mode| NamePool::new(mine.iter().map(|word| (*word).to_owned()).collect(), mode);
        // Only the player's words: both words of every name are theirs (the genus with its first letter raised, the epithet as it is).
        let mut registry = SpeciesRegistry::with_seed(7);
        registry.set_own_name_pool(pool(NameMode::Mine));
        for _ in 0..200 {
            let generic = registry.random_unused_genus();
            let specific = registry.random_unused_specific(Some(&generic));
            assert!(mine.iter().any(|word| capitalize(word) == generic), "{generic:?}");
            assert!(mine.contains(&specific.as_str()), "{specific:?}");
        }
        // Both lists: some of each.
        let mut both = SpeciesRegistry::with_seed(7);
        both.set_own_name_pool(pool(NameMode::Both));
        let names: Vec<String> = (0..400).map(|_| both.random_unused_specific(None)).collect();
        let theirs = names.iter().filter(|name| mine.contains(&name.as_str())).count();
        assert!((150..=250).contains(&theirs), "{theirs} of 400 from the player's words");
        assert!(names.iter().any(|name| LATIN_NAMES.iter().any(|word| word.to_lowercase() == *name)));
        // The name stream is used the same with a pool as without one: after the same number of names the next draw is the same number.
        let mut plain = SpeciesRegistry::with_seed(7);
        let mut with_pool = SpeciesRegistry::with_seed(7);
        with_pool.set_own_name_pool(pool(NameMode::Mine));
        for _ in 0..10 {
            let _ = plain.weighted_random_patron(1.5);
            let _ = with_pool.weighted_random_patron(1.5);
        }
        assert_eq!(plain.name_rng.next_u64(), with_pool.name_rng.next_u64());
        // Taking the pool away gives the game's names again, the same ones as a registry that never had one.
        with_pool.set_own_name_pool(None);
        let mut never = SpeciesRegistry::with_seed(11);
        let mut taken = SpeciesRegistry::with_seed(11);
        taken.set_own_name_pool(pool(NameMode::Mine));
        taken.set_own_name_pool(None);
        for _ in 0..50 {
            assert_eq!(never.random_unused_genus(), taken.random_unused_genus());
        }
    }

    #[test]
    fn species_are_named_after_the_games_own_latin_words() {
        // `SpeciesNameGenerator.latinNames`: the words of the game's `patreon_supporters` asset (the four the source names, then the
        // supporters', the biggest pledges first), exactly as the game has them: some end in a space.
        assert!(LATIN_NAMES.len() > 800, "{}", LATIN_NAMES.len());
        assert_eq!(LATIN_NAMES[0], "leo", "the creator's pledge is `int.MaxValue`");
        for built_in in ["borgpolisi", "aentekus", "mayzi"] {
            assert!(LATIN_NAMES.contains(&built_in), "{built_in}");
        }
        assert!(LATIN_NAMES.iter().any(|word| word.ends_with(' ')), "the trailing spaces of the game's data are kept");
        let mut registry = SpeciesRegistry::with_seed(7);
        let mut last_half = 0;
        for _ in 0..2_000 {
            let generic = registry.random_unused_genus();
            let specific = registry.random_unused_specific(Some(&generic));
            // The genus is a word with its first letter raised, the epithet a word as it is.
            assert!(LATIN_NAMES.iter().any(|word| capitalize(word) == generic), "{generic:?}");
            assert!(LATIN_NAMES.contains(&specific.as_str()), "{specific:?}");
            last_half += usize::from(LATIN_NAMES[LATIN_NAMES.len() / 2..].iter().any(|word| capitalize(word) == generic));
        }
        // `1 - U^4` is near 1 for most draws: a genus comes from the end of the list far more often than from its start
        // (P(second half) = 0.5^(1/4) = 0.84).
        assert!((1_500..=1_850).contains(&last_half), "{last_half} of 2000 genera from the second half of the list");
    }

    /// Test-only helper to create a child species directly (assignment cannot be
    /// driven to produce a specific child shape reliably).
    impl SpeciesRegistry {
        fn create_species_for_test(
            &mut self,
            genome: &Genome,
            brain: &Brain,
            meta: &mut GenomeMeta,
            parent: Option<SpeciesId>,
        ) -> SpeciesId {
            let id = self.create_species(genome, brain, meta, parent);
            meta.species = Some(id);
            id
        }
    }

    /// Birth-path microbench (task-42). Prints ns/op; no hard assertions so it
    /// is a measurement/report, not a gate. Run with:
    /// `cargo test --release -p bibites-core species::tests::birth_path_microbench -- --nocapture --test-threads=1`
    #[test]
    fn birth_path_microbench() {
        use crate::genome::MutationFactor;
        use std::hint::black_box;
        use std::time::Instant;

        const OPS: u32 = 4_000;

        fn measure(mut op: impl FnMut()) -> f64 {
            for _ in 0..64 {
                op(); // warm-up (also grows Vec capacities where applicable)
            }
            let mut best = f64::INFINITY;
            for _batch in 0..5 {
                let start = Instant::now();
                for _ in 0..OPS {
                    op();
                }
                let ns = start.elapsed().as_nanos() as f64 / f64::from(OPS);
                best = best.min(ns);
            }
            best
        }

        let mut settings = SimSettings::game_defaults();
        settings.scenario.background_mutation_chance = 0.0;
        settings.scenario.background_mutation_variance = 0.0;
        let no_mutations = {
            let mut s = settings.clone();
            s.scenario.prevent_mutations = true;
            s
        };
        let genome = Genome::starter();
        let brain = Brain::starter();
        let meta = GenomeMeta::default();
        let factor = MutationFactor::default();
        let mut rng = Rng::derive(0x0B1_7A5, 42, 7);

        let child_plain = measure(|| {
            let (child, child_meta) =
                black_box(&genome).child(black_box(&meta), &mut rng, &no_mutations, &factor);
            black_box((child, child_meta));
        });
        let child_mutating = measure(|| {
            let (child, child_meta) =
                black_box(&genome).child(black_box(&meta), &mut rng, &settings, &factor);
            black_box((child, child_meta));
        });
        let brain_clone = measure(|| {
            black_box(black_box(&brain).clone());
        });
        let rebuild = {
            let mut copy = brain.clone();
            measure(|| {
                copy.rebuild();
                black_box(&copy);
            })
        };
        let mutate_small = measure(|| {
            let mut copy = black_box(&brain).clone();
            copy.mutate(&mut rng, &settings, &genome, &factor);
            black_box(copy);
        });
        // Mutate only (pre-cloned pool, so clone/drop of the pool is outside the
        // timed region; the per-op drop of the mutated copy stays inside, as in
        // the clone+mutate row).
        let mutate_only = {
            let mut best = f64::INFINITY;
            for _batch in 0..5 {
                let mut pool: Vec<Brain> = (0..OPS).map(|_| brain.clone()).collect();
                let start = Instant::now();
                while let Some(mut copy) = pool.pop() {
                    copy.mutate(&mut rng, &settings, &genome, &factor);
                    black_box(copy);
                }
                best = best.min(start.elapsed().as_nanos() as f64 / f64::from(OPS));
            }
            best
        };

        // Grow a large brain (hidden nodes + synapses) for the heavy cases.
        let mut grow_settings = settings.clone();
        grow_settings.scenario.background_mutation_chance = 3.0;
        grow_settings.scenario.synapse_mutation_chance = 0.0;
        grow_settings.scenario.neuron_default_chance = 0.0;
        grow_settings.scenario.neuron_change_chance = 0.0;
        grow_settings.scenario.neuron_add_chance = 1.0;
        grow_settings.scenario.neuron_remove_chance = 0.0;
        let mut large = brain.clone();
        for _ in 0..80 {
            large.mutate(&mut rng, &grow_settings, &genome, &factor);
        }
        let large_state = (
            large.node_count(),
            large.synapse_count(),
            large.hidden_count(),
        );
        let mutate_large = measure(|| {
            let mut copy = black_box(&large).clone();
            copy.mutate(&mut rng, &settings, &genome, &factor);
            black_box(copy);
        });
        let mutate_large_only = {
            const LARGE_OPS: u32 = 512;
            let mut best = f64::INFINITY;
            for _batch in 0..5 {
                let mut pool: Vec<Brain> = (0..LARGE_OPS).map(|_| large.clone()).collect();
                let start = Instant::now();
                while let Some(mut copy) = pool.pop() {
                    copy.mutate(&mut rng, &settings, &genome, &factor);
                    black_box(copy);
                }
                best = best.min(start.elapsed().as_nanos() as f64 / f64::from(LARGE_OPS));
            }
            best
        };

        let mut registry = SpeciesRegistry::with_seed(7);
        let mut registry_meta = GenomeMeta::default();
        registry.create_species_for_test(&genome, &brain, &mut registry_meta, None);
        let assign = measure(|| {
            let mut meta = registry_meta.clone();
            black_box(registry.assign(black_box(&genome), black_box(&brain), &mut meta, &settings));
        });

        let mut rows = vec![
            ("Genome::child (no mutations)".to_string(), child_plain),
            ("Genome::child (mutating)".to_string(), child_mutating),
            ("Brain::clone (48n/3s)".to_string(), brain_clone),
            ("Brain::rebuild (48n/3s)".to_string(), rebuild),
            ("clone + Brain::mutate (48n/3s)".to_string(), mutate_small),
            ("Brain::mutate only (48n/3s)".to_string(), mutate_only),
            (
                format!(
                    "clone + Brain::mutate ({}n/{}s/{}h)",
                    large_state.0, large_state.1, large_state.2
                ),
                mutate_large,
            ),
            (
                format!(
                    "Brain::mutate only ({}n/{}s/{}h)",
                    large_state.0, large_state.1, large_state.2
                ),
                mutate_large_only,
            ),
            ("SpeciesRegistry::assign (stay)".to_string(), assign),
        ];
        println!("birth-path microbench ({OPS} ops, release):");
        println!("  {:<42} {:>10}", "operation", "ns/op");
        for (name, ns) in rows.drain(..) {
            println!("  {name:<42} {ns:>10.1}");
        }
    }

    // -------------------------------------------------------------------
    // Evolution benchmark (task-48)
    // -------------------------------------------------------------------

    /// One deterministic evolution run's metrics.
    #[allow(dead_code)]
    #[derive(Clone, Debug)]
    struct EvolutionReport {
        initial_population: usize,
        ticks: u64,
        seed: u64,
        wall_secs: f64,
        sim_secs: f64,
        ticks_per_second: f64,
        realtime_factor: f64,
        final_population: u32,
        min_population: u32,
        births: u64,
        deaths: u64,
        gen_max: u32,
        gen_mean: f64,
        species_active: usize,
        species_records: usize,
        gene_mutations: u64,
        brain_events: u64,
        synapses_added: u64,
        synapses_removed: u64,
        synapses_toggled: u64,
        nodes_added: u64,
        nodes_removed: u64,
        living_genomes: usize,
        changed_genomes: usize,
        distinct_genomes: usize,
        population_curve: Vec<(u64, u32)>,
        nan_entities: u64,
        fingerprint: u64,
    }

    fn build_evolution_world(initial_population: usize, seed: u64) -> crate::world::World {
        use crate::body::{BibiteSpawn, SpawnVitals};
        use crate::math::Vec2;
        use crate::world::{ScenarioData, World};

        // Tuned synthetic default ecology (same as
        // `crates/bibites-core/examples/evolution_bench.rs`): richer/larger
        // plant zones, founders spawned inside zones so they start near food.
        // The 1k/2k/5k founders still starve down after the first generations;
        // the benchmark reports that honestly.
        let mut settings = SimSettings::game_defaults();
        settings.scenario.pellet_energy = 2_000.0;
        settings.scenario.initial_seeding = 2.0;
        settings.independents.biomass_density = 0.02;
        settings.independents.pellet_growth = 0.0005;
        let mut scenario = ScenarioData::with_default_zone_group();
        for group in &mut scenario.zone_groups {
            group.zone_count = 50;
            group.template.radius_relative = 0.1;
            group.template.fertility = 2.0;
            group.template.biomass_density = 2.0;
        }
        let mut world = World::new(settings, scenario, seed).expect("evolution world");
        let _ = world.seed_scenario_world();
        for _ in 0..initial_population {
            let mut rng = world.rng.fork(0xB1B1_7E50);
            let zone = &world.zones.zones[rng.usize_below(world.zones.zones.len())];
            let radius = zone.radius * 0.9;
            let (dx, dy) = loop {
                let x = rng.f32_range(-1.0, 1.0);
                let y = rng.f32_range(-1.0, 1.0);
                if x * x + y * y <= 1.0 {
                    break (x * radius, y * radius);
                }
            };
            let pos = Vec2::new(zone.pos.x + dx, zone.pos.y + dy);
            let spawn = BibiteSpawn {
                pos,
                rot: rng.f32_range(0.0, std::f32::consts::TAU),
                vel: Vec2::ZERO,
                ang_vel: 0.0,
                genome: Genome::starter(),
                meta: GenomeMeta::default(),
                brain: Brain::starter(),
                vitals: SpawnVitals::Adult,
                energy: None,
                health: None,
            };
            let id = world.bibites.spawn(spawn).expect("spawn starter bibite");
            world.register_bibite(id);
        }
        world
    }

    fn run_evolution(initial_population: usize, ticks: u64, seed: u64) -> EvolutionReport {
        use crate::sim::state_fingerprint;
        use std::time::Instant;

        let mut world = build_evolution_world(initial_population, seed);
        // Counters reset after starter spawn (those are not mutations).
        crate::genome::mutation_counters::reset();
        crate::brain::mutation_counters::reset();
        let births_start = world.stats().births_total;
        let deaths_start = world.stats().deaths_total;

        let sample_every = (ticks / 20).max(1);
        let initial_sample = world.sample_stats();
        let mut population_curve = vec![(0_u64, initial_sample.counts.bibites)];
        let mut nan_entities = 0_u64;
        let start = Instant::now();
        for tick in 1..=ticks {
            world.tick();
            if tick % sample_every == 0 {
                let sample = world.sample_stats();
                population_curve.push((tick, sample.counts.bibites));
            }
            for (_, record) in world.bibites.iter() {
                if record.is_corpse || record.body.dead {
                    continue;
                }
                let bad = [
                    record.pos.x,
                    record.pos.y,
                    record.vel.x,
                    record.vel.y,
                    record.rot,
                    record.ang_vel,
                    record.body.energy.amount,
                    record.body.health.amount,
                ]
                .iter()
                .any(|value| !value.is_finite());
                if bad {
                    nan_entities += 1;
                }
            }
        }
        let wall_secs = start.elapsed().as_secs_f64();
        let final_sample = world.sample_stats();
        let sim_secs = ticks as f64 / f64::from(world.settings.sim_tps);

        let mut living_genomes = 0_usize;
        let mut changed_genomes = 0_usize;
        let mut distinct = std::collections::BTreeSet::new();
        let starter = Genome::starter();
        for (_, record) in world.bibites.iter() {
            if record.is_corpse || record.body.dead {
                continue;
            }
            living_genomes += 1;
            let mut hash = 0xcbf2_9ce4_8422_2325_u64;
            let mut changed = false;
            for (index, &gene) in record.genome.values().iter().enumerate() {
                if gene.to_bits() != starter.genes[index].to_bits() {
                    changed = true;
                }
                hash ^= u64::from(gene.to_bits());
                hash = hash.wrapping_mul(0x1000_0000_01b3);
            }
            distinct.insert(hash);
            if changed {
                changed_genomes += 1;
            }
        }
        let brain = crate::brain::mutation_counters::snapshot();
        EvolutionReport {
            initial_population,
            ticks,
            seed,
            wall_secs,
            sim_secs,
            ticks_per_second: ticks as f64 / wall_secs.max(f64::MIN_POSITIVE),
            realtime_factor: sim_secs / wall_secs.max(f64::MIN_POSITIVE),
            final_population: final_sample.counts.bibites,
            min_population: population_curve.iter().map(|(_, n)| *n).min().unwrap_or(0),
            births: world.stats().births_total - births_start,
            deaths: world.stats().deaths_total - deaths_start,
            gen_max: final_sample.generation.gen_max,
            gen_mean: f64::from(final_sample.generation.gen_mean),
            species_active: world.species.active_species_count(),
            species_records: world.species.records().len(),
            gene_mutations: crate::genome::mutation_counters::gene_mutations(),
            brain_events: brain.0,
            synapses_added: brain.1,
            synapses_removed: brain.2,
            synapses_toggled: brain.3,
            nodes_added: brain.4,
            nodes_removed: brain.5,
            living_genomes,
            changed_genomes,
            distinct_genomes: distinct.len(),
            population_curve,
            nan_entities,
            fingerprint: state_fingerprint(&world),
        }
    }

    /// Small deterministic evolution regression (ignored by default).
    ///
    /// Pins the exact end-state fingerprint for a fixed seed/settings/population,
    /// so evolution-path changes that alter results are caught. The expected
    /// value is generated from the same harness (see test output).
    #[test]
    #[ignore = "deterministic evolution fingerprint regression"]
    fn evolution_regression_fingerprint() {
        let report = run_evolution(64, 12_000, 0xE701_9999);
        println!(
            "evolution regression: fp {:#x}, births {}, deaths {}, gen {}, species {}, gene_mut {}, brain_ev {}",
            report.fingerprint,
            report.births,
            report.deaths,
            report.gen_max,
            report.species_records,
            report.gene_mutations,
            report.brain_events
        );
        assert_eq!(report.nan_entities, 0);
        assert_eq!(
            report.fingerprint, EVOLUTION_REGRESSION_FINGERPRINT,
            "evolution end-state diverged from the pinned fingerprint"
        );
    }

    /// Fingerprint generated by `evolution_regression_fingerprint` (12 000 ticks, 64 starters, seed 0xE701_9999). Pinned
    /// again when the physics x order became a total order (bodies whose boxes start at exactly the same x are ordered by
    /// body index instead of by the accident of the sort algorithm; the owner agreed on 2026-11-05): this world has such
    /// ties between bodies that touch, so its end state moved (births 20 -> 22). Before that: 0x5a1c_a5ce_dd15_88ac, pinned
    /// at commit 6766f25 (the old 0xaaf5_6a43_094b_6fde had gone stale with the parity work). Pinned again (0x34a3_75d3_c70c_e0aa before) when the
    /// physics became the game's soft step measured against the original: ten substeps, force and damping per slice, speculative contacts,
    /// warm starting, no speed cap at 100, two-point manifolds for capsules side by side, contacts found from further away for fast approaches.
    /// Pinned again (0x2aa4_d29d_274c_eb94 before) when bodies began to turn about their centre of mass (Unity's automatic one, from the colliders' areas).
    /// Pinned again (0x0b36_97b7_e237_72f3 before) when the muscles' force began to follow the body's heading slice by slice.
    /// Pinned again (0x05f4_6cb2_1cac_c401 before) when a tick became the game's ten physics steps for the bodies that touch (the contacts found again at every step).
    /// Pinned again (0x776e_0f04_d33c_7173 before) when the contact events came from the fine steps (the velocities of the step that first saw the contact).
    /// Pinned again (0x05e3_cf90_3457_d3ed before) with the grab as a weld joint, the bibites' boxes holding their colliders where they are (the mouth was missing) and
    /// the contacts' softness measured against deep overlaps (hertz at the cap, damping ratio 1, no limit on the push).
    /// Pinned again (0x48f3_ea70_493a_aed3 before) when the bibites' spatial-hash entries began to follow the bibites (before, an entry stayed where its bibite was
    /// spawned or loaded, so the others stopped seeing a bibite that had moved).
    /// Pinned again (0x5879_c7f2_fd9e_395c before) when the physics became the game at 1x: one step of `1 / simTPS` per tick (measured against the original on a 1200-bibite save)
    /// and no mouth contact grace.
    /// Pinned again (0x2cab_d7bc_8449_03cf before) when the contacts of the one-step tick became Box2D 2.4's (velocity iterations, positions integrated with the solved velocities,
    /// Baumgarte position iterations, contacts found when the shapes touch, restitution inside the velocity solve).
    /// Pinned again (0xc2bc_ff31_8241_1f0e before) on 2026-11-13: the test is ignored and had not been run since the contact physics, the pellet friction and the grabs of
    /// 2026-11-11 changed what a population does; the code of the day before the glitch fixes gave this very value, and so do the fixes (a lethal spend that stands,
    /// a hold that is checked against its prey's generation, a dead holder that lets go, poses that are numbers: none of it happens in this run).
    /// Pinned again (0xc40a_a2b2_b958_ec7c before) on 2026-11-19 when the brain's arithmetic became the game's (a C# expression is done in double and rounded to float only at
    /// a store, a call argument and a return: Sigmoid, Gaussian, Integrator, Inhibitory, SoftLatch; the Herding output is computed with herding off; `tanh(pheroSum / 5)` in double;
    /// measured bit for bit against the original game, `tools/braincheck/game_diff`): the nodes after a node that was one ulp off carry it on, so a population moves.
    /// Pinned again (0x160b_d186_45b3_8cc5 before) on 2026-11-19 when the senses became the game's, measured against the original game (`sense_vs_game`): Unity's vector maths in double
    /// (`magnitude`, `SignedAngle` by `acos`, `normalized`), the field-of-view weights as one double expression, `Transform.up` from the quaternion, the pheromone sense one tick late,
    /// the cadence counters subtracting the period, the growth and upkeep expressions in double: the population moves (births 25 -> 20, deaths 41 -> 44).
    /// Pinned again (0xa818_5c92_2bf4_bf31 before) on 2026-11-19 when a child's brain started fresh as in the game (`CopyBrain` copies only the structure and `FinalizeEditing` runs every
    /// non-input node once, so a node nothing feeds holds `f(bias)`; the port cloned the parent's running node values, which stayed for ever in an unfed node): births 20 -> 20, deaths 44 -> 43.
    const EVOLUTION_REGRESSION_FINGERPRINT: u64 = 0xf292_be2d_a8ad_d122;

    /// An evolving population (births, deaths, mutations, species changes, eaten and rotting pellets) with every parallel
    /// stage forced on gives the same checkpoint, byte for byte, as the serial run.
    #[test]
    fn evolution_is_bit_identical_in_parallel() {
        let build = |threads: usize| {
            let mut world = build_evolution_world(48, 0xE701_0077);
            world.set_parallel_gates(crate::sim::ParallelGates::EAGER);
            world.set_threads(threads);
            world
        };
        let encode = |world: &crate::world::World| {
            world
                .capture_checkpoint_state()
                .expect("checkpoint")
                .encode()
                .expect("encoded checkpoint")
        };
        let (mut serial, mut parallel) = (build(1), build(8));
        for round in 1..=6_u64 {
            serial.run(400);
            parallel.run(400);
            assert!(
                encode(&serial) == encode(&parallel),
                "the parallel run differs from the serial one after {} ticks",
                round * 400
            );
        }
        // (A population of 48 fed from the start takes longer than these 2,400 ticks to start dying or breeding: the run has to show that the whole
        // checkpoint, however much moved, comes out the same in parallel.)
        assert!(!serial.bibites.is_empty());
    }

    /// Fast non-ignored harness smoke test: two identical seeded runs agree.
    #[test]
    fn evolution_harness_is_deterministic() {
        let a = run_evolution(32, 800, 0xE701_0002);
        let b = run_evolution(32, 800, 0xE701_0002);
        // Mutation counters are process-global (test builds); only compare the
        // simulation state here, they are asserted in the single-threaded
        // benchmark/regression runs.
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(a.births, b.births);
        assert_eq!(a.deaths, b.deaths);
        assert_eq!(a.nan_entities, 0);
    }
}

/// Golden bit-identity probe for the birth/mutation path (task-42).
///
/// Folds 64 generations of `Genome::child` + `Brain::clone`/`mutate` +
/// `SpeciesRegistry::assign_with_founder` into one FNV-1a hash over every gene,
/// node state, synapse record, meta flag and species record. The expected value
/// was generated from the pre-optimization HEAD revision
/// (`872663d`), so this test proves the birth-path optimizations preserve
/// bit-identical results for the same seed/settings.
#[cfg(test)]
mod birth_path_golden {
    use super::*;
    use crate::brain::Brain;
    use crate::genome::{Genome, GenomeMeta, MutationFactor};
    use crate::rng::Rng;

    /// The vector without the species' names (see the loop over `registry.records()`); 0x8dbf_8f7e_8145_a174 while the names were in it.
    const EXPECTED_HASH: u64 = 0x1deb_4e72_9188_fe19;

    fn mix(hash: &mut u64, value: u64) {
        *hash ^= value;
        *hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    fn mixf(hash: &mut u64, value: f32) {
        mix(hash, u64::from(value.to_bits()));
    }
    fn mix_str(hash: &mut u64, value: &str) {
        mix(hash, value.len() as u64);
        for byte in value.bytes() {
            mix(hash, u64::from(byte));
        }
    }

    #[test]
    fn birth_path_is_bit_identical_to_pre_optimization_golden() {
        let mut settings = SimSettings::game_defaults();
        settings.scenario.pellet_energy = 2_000.0;
        settings.scenario.initial_seeding = 2.0;
        settings.independents.biomass_density = 0.02;
        settings.independents.pellet_growth = 0.0005;
        settings.scenario.pellet_collision = false;
        let factor = MutationFactor::default();
        let mut genome = Genome::starter();
        let mut brain = Brain::starter();
        let mut meta = GenomeMeta::default();
        let mut registry = SpeciesRegistry::with_seed(99);
        let mut rng = Rng::derive(0x5EED_1234, 0xABCD_0001, 7);
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for _ in 0..64_u32 {
            let (child_genome, mut child_meta) = genome.child(&meta, &mut rng, &settings, &factor);
            let mut child_brain = brain.clone();
            child_brain.mutate(&mut rng, &settings, &child_genome, &factor);
            registry.assign_with_founder(
                &child_genome,
                &child_brain,
                &mut child_meta,
                Some((&genome, &brain)),
                &settings,
            );
            for &gene in child_genome.values().iter() {
                mixf(&mut hash, gene);
            }
            mix(&mut hash, u64::from(child_meta.generation));
            mix(
                &mut hash,
                child_meta
                    .species
                    .map_or(u64::MAX, |id| u64::from(id.value())),
            );
            for flag in [
                child_meta.is_mutant,
                child_meta.is_rad,
                child_meta.is_rain,
                child_meta.is_killer,
                child_meta.is_source,
                child_meta.deux_parent,
            ] {
                mix(&mut hash, u64::from(flag));
            }
            if let Some(tag) = &child_meta.species_tag {
                mix_str(&mut hash, tag);
            }
            for node in child_brain.to_node_records() {
                mix(&mut hash, u64::from(node.node_type as u8));
                mix(&mut hash, u64::from(node.index));
                mix(&mut hash, node.innovation as u64);
                mixf(&mut hash, node.base_activation);
                mixf(&mut hash, node.value);
                mixf(&mut hash, node.last_input);
                mixf(&mut hash, node.last_output);
                if let Some(desc) = &node.desc {
                    mix_str(&mut hash, desc);
                }
            }
            for synapse in child_brain.to_synapse_records() {
                mix(&mut hash, synapse.innovation as u64);
                mix(&mut hash, u64::from(synapse.node_in));
                mix(&mut hash, u64::from(synapse.node_out));
                mixf(&mut hash, synapse.weight);
                mix(&mut hash, u64::from(synapse.enabled));
            }
            mix(&mut hash, child_brain.node_count() as u64);
            mix(&mut hash, child_brain.synapse_count() as u64);
            mix(&mut hash, child_brain.hidden_count() as u64);
            genome = child_genome;
            brain = child_brain;
            meta = child_meta;
        }
        for record in registry.records() {
            mix(&mut hash, u64::from(record.id.value()));
            mix(&mut hash, u64::from(record.stats.count));
            mix(
                &mut hash,
                record.parent.map_or(u64::MAX, |id| u64::from(id.value())),
            );
            for &child in &record.children {
                mix(&mut hash, u64::from(child.value()));
            }
            // The species' name is not part of the vector: it is made of the game's own word list (`latin_names.rs`, regenerated when the game
            // updates) and has its own test. (With the old placeholder list put back, the vector WITH the names was bit-identical to the old
            // golden 0x8dbf_8f7e_8145_a174 on 2026-11-15: the change of the list moved nothing else.)
            for &gene in record.template.genome.values().iter() {
                mixf(&mut hash, gene);
            }
        }
        assert_eq!(
            hash,
            EXPECTED_HASH,
            "birth path diverged from the pre-optimization golden vector \
             (gen={} species={:?} nodes={} synapses={} hidden={} registry={})",
            meta.generation,
            meta.species,
            brain.node_count(),
            brain.synapse_count(),
            brain.hidden_count(),
            registry.records().len()
        );
    }
}

#[cfg(test)]
mod checkpoint_tests {
    use super::*;

    fn sample_registry() -> SpeciesRegistry {
        let id = SpeciesId::new(3);
        let record = SpeciesRecord {
            id,
            time_creation: 1.5,
            generation_of_first_specimen: 4,
            favorite: true,
            generic_name: "Generic".to_string(),
            specific_name: "Species".to_string(),
            description: "checkpoint test".to_string(),
            parent: None,
            root: id,
            children: vec![SpeciesId::new(4)],
            template: TemplateSnapshot::from_live(&Genome::starter(), &Brain::starter()),
            stats: {
                let mut stats = SpeciesStats::new(true);
                stats.count = 2;
                stats.energy = 12.5;
                for count in [2, 2, 1, 0, 0, 3] {
                    stats.log.push(SpeciesDataPoint::from_counts(
                        count,
                        f32::from(count as u16) * 6.25,
                    ));
                }
                stats
            },
        };
        let mut registry = SpeciesRegistry::with_seed(0x5EED);
        registry.records.push(record);
        registry.active.push(id);
        registry.next_id = 7;
        registry.node_max_innovation = 55;
        registry.connection_max_innovation = 66;
        registry.time = 3.5;
        registry.log_progress = 0.25;
        registry.log_counter = 11;
        registry.n_records = 3;
        registry.max_energy = 12.5;
        registry.max_count = 2.0;
        registry.log_period_seconds = 60.0;
        registry.prune_point_limit = 4;
        registry.cached_span = 1.75;
        registry.cached_span_active_count = 1;
        registry
    }

    #[test]
    fn a_log_raises_max_energy_to_the_most_the_active_species_held_together() {
        let mut registry = sample_registry();
        assert_eq!(registry.max_energy(), 12.5);
        let id = registry.records[0].id;
        // The registry's own energy is stale (12.5), the members' is in the sample.
        let point = |energy: f32| (id, SpeciesDataPoint::from_counts(2, energy));
        registry.log_stats_with(&[point(900.0)]);
        assert_eq!(registry.max_energy(), 900.0, "grows to the sample");
        registry.log_stats_with(&[point(400.0)]);
        assert_eq!(registry.max_energy(), 900.0, "never shrinks");
    }

    #[test]
    fn max_energy_is_rebuilt_from_the_histories_of_a_save_that_stored_none() {
        let mut registry = sample_registry();
        registry.max_energy = 0.0;
        // The sample species' log has points of 12.5, 12.5, 6.25, 0, 0, 18.75 (newest first in `at`): the most together is 18.75.
        registry.raise_max_energy_from_history();
        assert!((registry.max_energy() - 18.75).abs() < 1e-3, "{}", registry.max_energy());
        // A stored value above what the logs give stays (a game save).
        registry.max_energy = 500.0;
        registry.raise_max_energy_from_history();
        assert_eq!(registry.max_energy(), 500.0);
    }

    #[test]
    fn capture_restore_roundtrip_preserves_registry_and_name_stream() {
        let mut original = sample_registry();
        let checkpoint = original.capture_checkpoint();
        let bytes = checkpoint.encode();
        let decoded = SpeciesCheckpoint::decode(&bytes).expect("decode");
        assert_eq!(decoded, checkpoint);

        let mut restored = SpeciesRegistry::new();
        restored.restore_checkpoint(decoded).expect("restore");
        assert_eq!(restored.capture_checkpoint(), original.capture_checkpoint());
        assert_eq!(restored.records().len(), 1);
        assert_eq!(restored.active_species(), &[SpeciesId::new(3)]);
        // Name-generation stream continues at the saved position.
        assert_eq!(original.name_rng.next_u64(), restored.name_rng.next_u64());
    }

    #[test]
    fn checkpoint_decode_rejects_corruption() {
        let mut bytes = sample_registry().capture_checkpoint().encode();
        let mut bad = bytes.clone();
        bad[0] ^= 0xFF;
        assert!(SpeciesCheckpoint::decode(&bad).is_err());
        for cut in 0..bytes.len() {
            assert!(
                SpeciesCheckpoint::decode(&bytes[..cut]).is_err(),
                "cut {cut}"
            );
        }
        bytes.push(0);
        assert!(SpeciesCheckpoint::decode(&bytes).is_err());
    }
}

#[cfg(test)]
mod species_view_tests {
    use super::*;
    use crate::genome::{Gene, Genome};
    use crate::settings::SimSettings;

    fn settings() -> SimSettings {
        SimSettings::game_defaults()
    }

    fn outlier(genome: &mut Genome) {
        genome.set_gene(Gene::LayTime, 45.0);
        genome.set_gene(Gene::ViewRadius, 10.0);
        genome.set_gene(Gene::Diet, 1.0);
        genome.set_gene(Gene::StomachWAG, 8.0);
        genome.set_gene(Gene::ColorR, 1.0);
    }

    #[test]
    fn species_and_lineage_views_are_self_contained() {
        let s = settings();
        let mut registry = SpeciesRegistry::new();
        let genome = Genome::starter();
        let brain = Brain::starter();
        let mut meta = GenomeMeta::default();
        let root = registry.assign(&genome, &brain, &mut meta, &s);
        let mut stray = Genome::starter();
        outlier(&mut stray);
        let mut stray_meta = GenomeMeta {
            species: Some(root),
            ..GenomeMeta::default()
        };
        let _ = registry.assign(&stray, &brain, &mut stray_meta, &s);
        let child = registry.assign(&stray, &brain, &mut stray_meta, &s);
        registry.on_birth(root, root, 0);
        registry.on_birth(root, root, 0);
        registry.log_stats();

        let list = registry.species_list();
        assert_eq!(list.len(), 2);
        let child_entry = list.iter().find(|entry| entry.id == child).unwrap();
        assert_eq!(child_entry.parent, Some(root));
        assert_eq!(child_entry.root, root);
        assert!(!child_entry.extinct);

        let tree = registry.lineage_tree();
        assert_eq!(tree.roots, vec![root]);
        assert_eq!(tree.max_depth, 1);
        assert_eq!(tree.nodes.len(), 2);
        assert!(
            tree.edges
                .iter()
                .any(|edge| edge.parent == root && edge.child == child)
        );
        let root_node = tree.nodes.iter().find(|node| node.id == root).unwrap();
        assert_eq!(root_node.depth, 0);
        assert_eq!(root_node.order, 0);
        assert_eq!(root_node.child_count, 1);
        assert_eq!(root_node.subtree_species_count, 2);
        assert!(root_node.subtree_member_count >= 3);
        assert_eq!(root_node.n_points_present, 1);
        assert_eq!(root_node.total_index_of_apparition, 0);
        assert_eq!(root_node.total_index_of_disappearance, -1);
        assert_eq!(registry.log(root).unwrap().n_points_present(), 1);
        let child_node = tree.nodes.iter().find(|node| node.id == child).unwrap();
        assert_eq!(child_node.parent, Some(root));
        assert_eq!(child_node.depth, 1);
        assert_eq!(child_node.order, 1);
        assert_eq!(child_node.child_count, 0);
        assert_eq!(child_node.subtree_species_count, 1);
        assert_eq!(child_node.n_points_present, 1);

        let root_sel = registry.selected_lineage(root).unwrap();
        assert_eq!(root_sel.selected, root);
        assert_eq!(root_sel.parent, None);
        assert_eq!(root_sel.children, vec![child]);
        assert_eq!(root_sel.descendants, vec![child]);
        assert_eq!(root_sel.path_from_root, vec![root]);

        let child_sel = registry.selected_lineage(child).unwrap();
        assert_eq!(child_sel.parent, Some(root));
        assert_eq!(child_sel.ancestors, vec![root]);
        assert_eq!(child_sel.path_from_root, vec![root, child]);
        assert_eq!(child_sel.root, root);
        assert!(child_sel.children.is_empty());
        assert!(registry.selected_lineage(SpeciesId::new(9_999)).is_none());
    }
}

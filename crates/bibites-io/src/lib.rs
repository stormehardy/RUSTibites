//! # bibites-io
//!
//! Reader and writer for The Bibites file formats, so RUSTibites can import real
//! worlds/genomes **and** export files the original game can load again.
//!
//! It implements every format the installed 0.6.3.1 game reads/writes:
//! `.bb8`/`.bb8template` genomes and templates (with the game's version
//! migrations), `.bb8settings` (scenario/independent/material scopes), save
//! `.zip` bundles, scenario `.zip` bundles and the `WorldContents` bridge that
//! hands plain records to `bibites-core`. See `docs/spec/04-formats-settings.md`
//! and `docs/API.md`.
//!
//! ## Round-trip guarantees
//!
//! * Unknown JSON fields are preserved (`extra` maps in [`model`]).
//! * Numbers are written as shortest round-trip `f32` (serde_json); the game's
//!   reader parses them back to the same f32 values.
//! * Migrations are applied on load exactly like the game's `BibiteUpdater` /
//!   `BrainUpdater` (with one documented deterministic substitute for the
//!   game's unseeded `Random.Range` in the 0.6a13 step); the writer always emits
//!   the present version ([`version::PRESENT`], `0.6.3.1`).
//! * The `.bb8` writer emits a genome-only file (genes + brain + version). The
//!   0.6.3.1 game consumes `.bb8` files as template sources only (it never
//!   loads their body/clock into a world), so this is loadable; full entity
//!   snapshots come with the save-bundle writer (I4).
//!
//! No serde types leak into `bibites-core`; conversion goes through the plain
//! `NodeRecord`/`SynapseRecord`/`Genome`/`GenomeMeta` surfaces from brain-dev.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod bundle;
pub mod checkpoint;
pub mod model;
pub mod prefs;
pub mod settings;
pub mod storage;
pub mod version;
pub mod world;

mod migrate;

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};

use bibites_core::brain::{Brain, Input, NodeRecord, NodeType, Output, SynapseRecord};
use bibites_core::genome::{ALL_GENES, Genome, GenomeMeta};
use bibites_core::ids::SpeciesId;
use bibites_core::settings::SimSettings;
use bibites_core::{Error, Result};

pub use bundle::{
    SaveBundle, ScenarioBundle, ScenarioCard, ScenarioInfo, load_save_bundle, load_scenario_bundle,
    read_scenario_card, write_save_bundle, write_scenario_bundle,
};
pub use checkpoint::{
    BB8STATE_HEADER_LEN, BB8STATE_MAGIC, BB8STATE_V2_CHUNK, BB8STATE_VERSION, BB8STATE_VERSION_V2,
    checkpoint_path, read_checkpoint_state, read_checkpoint_state_checked,
    read_checkpoint_state_v2, write_checkpoint_state, write_checkpoint_state_v2,
    write_checkpoint_state_with_seed,
};
pub use settings::{
    ScenarioSettingsFile, SettingsMode, apply_recommended, load_settings_file, parse_independents,
    parse_scenario_settings, parse_settings_json, resolve_effective_settings,
    serialize_save_settings, serialize_scenario_settings, to_bridge_settings_json,
    write_settings_file,
};
pub use version::Version;
pub use world::{
    load_scenario_world_contents, load_world_contents, save_bundle_from_contents,
    save_world_contents, scenario_bundle_from_contents, scenario_world_contents,
    world_contents_from_save,
};

use crate::model::{
    AnchorJson, Bb8File, Bb8Genes, BrainJson, Extra, GenesRepr, NodeJson, SynapseJson, TemplateFile,
};

/// Default number of brain inputs (33), re-exported for tests/inspect code.
pub const N_INPUTS: usize = bibites_core::brain::N_INPUTS;
/// Default number of brain outputs (15).
pub const N_OUTPUTS: usize = bibites_core::brain::N_OUTPUTS;

/// Makes a file so that a failure part-way leaves the file that was there as it was: `write` writes the whole new file to a name beside `path`, which then
/// takes `path`'s place. Writing into `path` itself empties the old file first, so a save that overwrote another (the quick save, an overwrite the
/// player confirmed) lost both when the disk filled up or the program was killed half-way.
pub(crate) fn write_replacing<T>(path: &Path, write: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
    let mut beside = path.as_os_str().to_owned();
    beside.push(".part");
    let beside = std::path::PathBuf::from(beside);
    let made = match write(&beside) {
        Ok(made) => made,
        Err(error) => {
            let _ = std::fs::remove_file(&beside);
            return Err(error);
        }
    };
    // What was written reaches the disk before it takes the old file's place (after a power cut the new name could otherwise hold data that never was).
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&beside) {
        let _ = file.sync_all();
    }
    if std::fs::rename(&beside, path).is_err() {
        // The old file may be held open by another program, which refuses the swap: the finished file is copied over it, as the in-place write did.
        let copied = std::fs::copy(&beside, path);
        let _ = std::fs::remove_file(&beside);
        copied.map_err(Error::from)?;
    }
    Ok(made)
}

/// [`write_replacing`] for a file whose bytes are at hand (`std::fs::write`, without emptying the old file first).
pub(crate) fn write_bytes_replacing(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> Result<()> {
    write_replacing(path.as_ref(), |beside| std::fs::write(beside, bytes.as_ref()).map_err(Error::from))
}

/// File-level template metadata that is not part of the simulation meta.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TemplateMeta {
    /// Display name (`name`).
    pub name: Option<String>,
    /// Species display name (`speciesName`); the game recomputes it from
    /// `name` on load.
    pub species_name: Option<String>,
    /// Free-form description.
    pub description: Option<String>,
    /// Shipped-template flag (`isOfficial`).
    pub is_official: bool,
    /// Brain-editor node anchors keyed by innovation number.
    pub node_anchors: BTreeMap<i64, AnchorJson>,
}

impl TemplateMeta {
    /// Returns the saved editor position for a node innovation.
    #[must_use]
    pub fn node_anchor(&self, innovation: i64) -> Option<(f32, f32)> {
        self.node_anchors.get(&innovation).map(AnchorJson::position)
    }

    /// Saves/updates a node's editor position. `(x, y)` is stored verbatim in
    /// the template file's `nodeAnchors` map keyed by node innovation.
    pub fn set_node_anchor(&mut self, innovation: i64, x: f32, y: f32) {
        self.node_anchors.insert(innovation, AnchorJson::new(x, y));
    }

    /// Removes a saved node position. Returns true if one existed.
    pub fn remove_node_anchor(&mut self, innovation: i64) -> bool {
        self.node_anchors.remove(&innovation).is_some()
    }

    /// Drops all saved node positions (e.g. before auto-layout).
    pub fn clear_node_anchors(&mut self) {
        self.node_anchors.clear();
    }
}

/// Which on-disk shape a [`LoadedGenome`] came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceShape {
    /// `.bb8template` (genome + brain template; no runtime state).
    Template,
    /// Genome-only `.bb8` export (genes + brain + version).
    GenomeOnlyBb8,
    /// Living bibite `.bb8` (transform/body/clock present).
    LivingBb8,
    /// Egg `.bb8` (egg section present).
    EggBb8,
}

/// A genome loaded from a `.bb8` or `.bb8template` file, migrated to the
/// present version.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedGenome {
    /// The 35 genes.
    pub genome: Genome,
    /// Generation, species tag, parent ids and easter-egg flags.
    pub meta: GenomeMeta,
    /// The NEAT brain (template semantics for `.bb8template`, runtime-state
    /// semantics for `.bb8`).
    pub brain: Brain,
    /// Version of the file that was loaded.
    pub version: Version,
    /// File-level template metadata (names, anchors, official flag).
    pub template: TemplateMeta,
    /// Shape the file had.
    pub source: SourceShape,
    /// `isReady` flag from a live-entity file (`true` for templates).
    pub is_ready: bool,
    /// Raw `parent1` runtime id (only meaningful for save-bundle entities;
    /// preserved for round-trip, not resolvable to a core handle in I1).
    pub parent1: Option<i32>,
    /// Raw `parent2` runtime id.
    pub parent2: Option<i32>,
    /// Unknown top-level JSON fields, preserved for round-trip.
    pub extra: Extra,
    /// Unknown fields of each brain node (same order as the loaded node array),
    /// preserved for round-trip.
    pub node_extras: Vec<Extra>,
    /// Unknown fields of each synapse (same order as the loaded synapse array).
    pub synapse_extras: Vec<Extra>,
    /// Unknown fields of the `brain` wrapper object.
    pub brain_extra: Extra,
}

/// Small summary used by the CLI `rustibites info` command.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectSummary {
    /// File shape detected.
    pub shape: SourceShape,
    /// Version string from the file.
    pub version: String,
    /// Number of genes (always 35 for a supported file).
    pub gene_count: usize,
    /// Number of nodes (33 + 15 + hidden).
    pub node_count: usize,
    /// Number of synapses.
    pub synapse_count: usize,
    /// Input count (33 for a present-version brain).
    pub input_count: usize,
    /// Output count (15).
    pub output_count: usize,
    /// Hidden node count.
    pub hidden_count: usize,
    /// Species tag, if any.
    pub species_tag: Option<String>,
    /// Generation, if present.
    pub generation: u32,
}

/// Detects whether a JSON document is a template or a `.bb8` entity.
#[must_use]
pub fn detect_shape(json: &str) -> Option<SourceShape> {
    let value: Value = serde_json::from_str(json).ok()?;
    let object = value.as_object()?;
    if object.contains_key("brain") {
        if object.contains_key("egg") {
            return Some(SourceShape::EggBb8);
        }
        if object.contains_key("body") || object.contains_key("clock") {
            return Some(SourceShape::LivingBb8);
        }
        return Some(SourceShape::GenomeOnlyBb8);
    }
    if object.contains_key("nodes") && object.contains_key("genes") {
        return Some(SourceShape::Template);
    }
    None
}

/// Loads and migrates a `.bb8` genome/entity file.
pub fn load_genome(path: impl AsRef<Path>) -> Result<LoadedGenome> {
    let text = std::fs::read_to_string(path).map_err(Error::from)?;
    from_json(&text)
}

/// Loads and migrates a `.bb8template` file (brain finalised with the default
/// game brain period; use [`load_template_with_period`] for a running world).
pub fn load_template(path: impl AsRef<Path>) -> Result<LoadedGenome> {
    let period = SimSettings::game_defaults().brain_period();
    // The game tolerates shifted node indexes, -1 endpoints and short old
    // gene arrays, so the template path (scenario spawner/CLI) is tolerant by
    // default; use rom_json/rom_json_with_period for strict validation.
    load_template_with_period_mode(path, period, true)
}

/// Tolerant variant of [`load_template`]: repairs the node-index/duplicate-
/// innovation quirks the real game tolerates (repair decisions are mirrored
/// from the game's brain updater; see spec 04 section 4).
pub fn load_template_tolerant(path: impl AsRef<Path>) -> Result<LoadedGenome> {
    let period = SimSettings::game_defaults().brain_period();
    load_template_with_period_mode(path, period, true)
}

/// Loads a `.bb8template` with an explicit brain period
/// (`brain_tps / sim_tps`, e.g. `2/30`).
pub fn load_template_with_period(path: impl AsRef<Path>, period: f32) -> Result<LoadedGenome> {
    load_template_with_period_mode(path, period, true)
}

fn load_template_with_period_mode(
    path: impl AsRef<Path>,
    period: f32,
    tolerant: bool,
) -> Result<LoadedGenome> {
    let text = std::fs::read_to_string(path).map_err(Error::from)?;
    from_json_with_period_mode(&text, period, tolerant)
}

/// Parses a `.bb8`/`.bb8template` JSON document (auto-detected), using the
/// default game brain period for templates.
pub fn from_json(json: &str) -> Result<LoadedGenome> {
    from_json_with_period(json, SimSettings::game_defaults().brain_period())
}

/// Parses a `.bb8`/`.bb8template` JSON document with an explicit template brain
/// period.
pub fn from_json_with_period(json: &str, period: f32) -> Result<LoadedGenome> {
    from_json_with_period_mode(json, period, false)
}

/// Tolerant parse (same repairs as [`load_template_tolerant`]).
pub fn from_json_tolerant(json: &str) -> Result<LoadedGenome> {
    let period = SimSettings::game_defaults().brain_period();
    from_json_with_period_mode(json, period, true)
}

fn from_json_with_period_mode(json: &str, period: f32, tolerant: bool) -> Result<LoadedGenome> {
    // The game writes some JSON files with a UTF-8 BOM (spec 04 section 1.1);
    // serde_json rejects it, so strip it before parsing.
    let json = json.strip_prefix('\u{feff}').unwrap_or(json);
    let shape = detect_shape(json).ok_or_else(|| {
        Error::invalid_data("JSON is neither a .bb8 entity nor a .bb8template (no brain/genes)")
    })?;
    match shape {
        SourceShape::Template => {
            let file: TemplateFile = serde_json::from_str(json).map_err(|error| {
                Error::invalid_data(format!("invalid .bb8template JSON: {error}"))
            })?;
            template_from_file(file, period, tolerant)
        }
        _ => {
            let file: Bb8File = serde_json::from_str(json)
                .map_err(|error| Error::invalid_data(format!("invalid .bb8 JSON: {error}")))?;
            bb8_from_file(file, shape, tolerant, period)
        }
    }
}

/// Serializes a template from core data.
pub fn template_to_json(
    genome: &Genome,
    meta: &GenomeMeta,
    brain: &Brain,
    template: &TemplateMeta,
) -> Result<String> {
    let file = template_from_core(genome, meta, brain, template)?;
    serde_json::to_string(&file)
        .map_err(|error| Error::invalid_data(format!("cannot serialize template: {error}")))
}

/// Writes a `.bb8template` indented when `pretty` (`UserSettings.FormatBibiteJSON`, as `SaveSystem.SaveTemplate` does).
pub fn save_template_pretty(
    path: impl AsRef<Path>,
    genome: &Genome,
    meta: &GenomeMeta,
    brain: &Brain,
    template: &TemplateMeta,
    pretty: bool,
) -> Result<()> {
    let file = template_from_core(genome, meta, brain, template)?;
    let text = if pretty {
        serde_json::to_string_pretty(&file)
    } else {
        serde_json::to_string(&file)
    }
    .map_err(|error| Error::invalid_data(format!("cannot serialize template: {error}")))?;
    write_bytes_replacing(path, text)
}

/// Serializes a genome-only `.bb8` from core data.
pub fn genome_to_json(genome: &Genome, meta: &GenomeMeta, brain: &Brain) -> Result<String> {
    let file = bb8_from_core(genome, meta, brain);
    serde_json::to_string(&file)
        .map_err(|error| Error::invalid_data(format!("cannot serialize .bb8: {error}")))
}

/// Writes a `.bb8template` file.
pub fn save_template(
    path: impl AsRef<Path>,
    genome: &Genome,
    meta: &GenomeMeta,
    brain: &Brain,
    template: &TemplateMeta,
) -> Result<()> {
    let text = template_to_json(genome, meta, brain, template)?;
    write_bytes_replacing(path, text)
}

/// Writes one live-world bibite snapshot as a standalone game `.bb8` object
/// (Stats panel SAVE button). Mirrors the save-bundle entity writer: no
/// synthetic parent/child ids, body id 0.
pub fn save_bibite_entity(
    path: impl AsRef<Path>,
    save: &bibites_core::body::BibiteSave,
) -> Result<()> {
    let json = bundle::bibite_to_json(save, &Extra::default(), (None, None), &[], 0);
    let bytes =
        serde_json::to_vec(&json).map_err(|error| Error::invalid_data(error.to_string()))?;
    write_bytes_replacing(path, bytes)
}

/// Writes one live-world egg snapshot as a standalone game `.bb8` object.
pub fn save_egg_entity(path: impl AsRef<Path>, save: &bibites_core::body::EggSave) -> Result<()> {
    let json = bundle::egg_to_json(save, &Extra::default(), (None, None), 0);
    let bytes =
        serde_json::to_vec(&json).map_err(|error| Error::invalid_data(error.to_string()))?;
    write_bytes_replacing(path, bytes)
}

/// Writes a core [`bibites_core::world::TemplateSource`] as a `.bb8template`
/// (Stats panel "save as template" / template export).
pub fn save_template_source(
    path: impl AsRef<Path>,
    source: &bibites_core::world::TemplateSource,
) -> Result<()> {
    let template = TemplateMeta {
        name: Some(source.name.clone()),
        ..TemplateMeta::default()
    };
    save_template(path, &source.genome, &source.meta, &source.brain, &template)
}

/// Writes `value` like `JObject.ToString(Formatting)`: indented when `pretty`
/// (`UserSettings.FormatBibiteJSON`), compact otherwise.
fn write_json_value(path: &Path, value: &Value, pretty: bool) -> Result<()> {
    let text = if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
    .map_err(|error| Error::invalid_data(error.to_string()))?;
    write_bytes_replacing(path, text)
}

/// Adds the keys `SaveSystem.SaveBibite` / `SaveEgg` put next to the entity: the game `version`
/// and the player's `desc`.
fn with_version_and_description(mut json: Value, version: &str, description: &str) -> Value {
    if let Value::Object(map) = &mut json {
        map.insert("version".to_owned(), Value::from(version));
        map.insert("desc".to_owned(), Value::from(description));
    }
    json
}

/// `SaveSystem.SaveBibite`: a standalone `.bb8` with the entity, the game `version` and the
/// player's description, indented when `pretty` (`FormatBibiteJSON`).
pub fn save_bibite_entity_described(
    path: impl AsRef<Path>,
    save: &bibites_core::body::BibiteSave,
    version: &str,
    description: &str,
    pretty: bool,
) -> Result<()> {
    let json = bundle::bibite_to_json(save, &Extra::default(), (None, None), &[], 0);
    write_json_value(
        path.as_ref(),
        &with_version_and_description(json, version, description),
        pretty,
    )
}

/// `SaveSystem.SaveEgg`: like [`save_bibite_entity_described`] for an egg.
pub fn save_egg_entity_described(
    path: impl AsRef<Path>,
    save: &bibites_core::body::EggSave,
    version: &str,
    description: &str,
    pretty: bool,
) -> Result<()> {
    let json = bundle::egg_to_json(save, &Extra::default(), (None, None), 0);
    write_json_value(
        path.as_ref(),
        &with_version_and_description(json, version, description),
        pretty,
    )
}

/// `SaveSystem.SaveTemplate`: a `.bb8template` named `source.name` with the player's
/// description, indented when `pretty` (`FormatBibiteJSON`).
pub fn save_template_source_described(
    path: impl AsRef<Path>,
    source: &bibites_core::world::TemplateSource,
    description: &str,
    pretty: bool,
) -> Result<()> {
    let template = TemplateMeta {
        name: Some(source.name.clone()),
        description: Some(description.to_owned()),
        ..TemplateMeta::default()
    };
    let file = template_from_core(&source.genome, &source.meta, &source.brain, &template)?;
    let text = if pretty {
        serde_json::to_string_pretty(&file)
    } else {
        serde_json::to_string(&file)
    }
    .map_err(|error| Error::invalid_data(format!("cannot serialize template: {error}")))?;
    write_bytes_replacing(path, text)
}

/// Writes a genome-only `.bb8` file (loadable by the game as a template source).
pub fn save_genome(
    path: impl AsRef<Path>,
    genome: &Genome,
    meta: &GenomeMeta,
    brain: &Brain,
) -> Result<()> {
    let text = genome_to_json(genome, meta, brain)?;
    write_bytes_replacing(path, text)
}

impl LoadedGenome {
    /// `BibiteTemplate.speciesName` as the game's constructor sets it (`file_name` is the file's name): a `.bb8template` has its `name` in species format (the file's name
    /// when it carries none), a `.bb8` the name it stores, else its file name in species format.
    #[must_use]
    pub fn game_species_name(&self, file_name: &str) -> String {
        let stem = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name).trim_end_matches(".bb8template").trim_end_matches(".bb8");
        match self.source {
            SourceShape::Template => to_species_format(self.template.name.as_deref().filter(|name| !name.is_empty()).unwrap_or(stem)),
            _ => self
                .template
                .species_name
                .clone()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| to_species_format(stem)),
        }
    }

    /// Builds the official starter `.bb8template` in memory (Basic bibite
    /// genome + starter brain, empty meta). The name is both the file/display
    /// name and the source of the generated species name; the result can be
    /// edited through its public `genome`/`brain`/`template` fields and saved
    /// with [`LoadedGenome::to_json`] or [`save_template`].
    #[must_use]
    pub fn starter_template(name: impl Into<String>) -> Self {
        let name = name.into();
        let brain = Brain::starter();
        let node_count = brain.node_count();
        let synapse_count = brain.synapse_count();
        let template = TemplateMeta {
            name: Some(name.clone()),
            species_name: Some(to_species_format(&name)),
            description: None,
            is_official: false,
            node_anchors: BTreeMap::new(),
        };
        Self {
            genome: Genome::starter(),
            meta: GenomeMeta::default(),
            brain,
            version: version::present(),
            template,
            source: SourceShape::Template,
            is_ready: true,
            parent1: None,
            parent2: None,
            extra: Extra::new(),
            node_extras: vec![Extra::new(); node_count],
            synapse_extras: vec![Extra::new(); synapse_count],
            brain_extra: Extra::new(),
        }
    }

    /// Returns the saved editor position for a node innovation.
    #[must_use]
    pub fn node_anchor(&self, innovation: i64) -> Option<(f32, f32)> {
        self.template.node_anchor(innovation)
    }

    /// Saves/updates a node's editor position in this template's metadata.
    pub fn set_node_anchor(&mut self, innovation: i64, x: f32, y: f32) {
        self.template.set_node_anchor(innovation, x, y);
    }

    /// Serializes this loaded genome back to JSON in the present version,
    /// preserving unknown fields captured on load.
    ///
    /// Templates are written as `.bb8template`; `.bb8` sources are re-emitted as
    /// genome-only `.bb8` files (see the module docs).
    pub fn to_json(&self) -> Result<String> {
        match self.source {
            SourceShape::Template => {
                let mut nodes = nodes_to_json(&self.brain, false);
                apply_node_extras(&mut nodes, &self.node_extras);
                let mut synapses = synapses_to_json(&self.brain);
                apply_synapse_extras(&mut synapses, &self.synapse_extras);
                let file = TemplateFile {
                    name: self.template.name.clone(),
                    species_name: self.template.species_name.clone(),
                    description: self.template.description.clone(),
                    generation: Some(self.meta.generation),
                    version: version::PRESENT.to_string(),
                    is_official: self.template.is_official,
                    nodes,
                    synapses,
                    genes: Some(GenesRepr::Object(genes_to_json(&self.genome))),
                    node_anchors: self.template.node_anchors.clone(),
                    extra: self.extra.clone(),
                };
                serde_json::to_string(&file).map_err(|error| {
                    Error::invalid_data(format!("cannot serialize template: {error}"))
                })
            }
            _ => {
                let mut file = bb8_from_core(&self.genome, &self.meta, &self.brain);
                file.extra = self.extra.clone();
                file.genes.parent1 = self.parent1;
                file.genes.parent2 = self.parent2;
                file.genes.is_ready = Some(self.is_ready);
                apply_node_extras(&mut file.brain.nodes, &self.node_extras);
                apply_synapse_extras(&mut file.brain.synapses, &self.synapse_extras);
                file.brain.extra = self.brain_extra.clone();
                serde_json::to_string(&file)
                    .map_err(|error| Error::invalid_data(format!("cannot serialize .bb8: {error}")))
            }
        }
    }
}

/// Returns a summary without keeping the brain.
pub fn inspect(path: impl AsRef<Path>) -> Result<InspectSummary> {
    let loaded = load_genome(path)?;
    Ok(InspectSummary {
        shape: loaded.source,
        version: loaded.version.to_string(),
        gene_count: loaded.genome.genes.len(),
        node_count: loaded.brain.node_count(),
        synapse_count: loaded.brain.synapse_count(),
        input_count: N_INPUTS,
        output_count: N_OUTPUTS,
        hidden_count: loaded.brain.node_count() - N_INPUTS - N_OUTPUTS,
        species_tag: loaded.meta.species_tag.clone(),
        generation: loaded.meta.generation,
    })
}

// ---------------------------------------------------------------------------
// Template conversion
// ---------------------------------------------------------------------------

fn template_from_file(file: TemplateFile, period: f32, tolerant: bool) -> Result<LoadedGenome> {
    let version = Version::parse(&file.version)?;
    ensure_supported(version)?;
    let genes_repr = file
        .genes
        .as_ref()
        .ok_or_else(|| Error::invalid_data("template has no genes"))?;
    let remap_needed = version < Version::new(0, 6, 0, 0, 1);
    let genes_repr = if tolerant {
        pad_short_gene_array(genes_repr)
    } else {
        genes_repr.clone()
    };
    let genes = migrate::migrate_genes(version, &genes_repr, remap_needed, false)?;
    let node_extras: Vec<Extra> = file.nodes.iter().map(|node| node.extra.clone()).collect();
    let synapse_extras: Vec<Extra> = file
        .synapses
        .iter()
        .map(|synapse| synapse.extra.clone())
        .collect();
    let (mut nodes, mut synapses) = records_from_json(&file.nodes, &file.synapses, tolerant)?;
    if migrate::migrate_brain(version, &mut nodes, &mut synapses)? {
        migrate::normalize_base_nodes(&mut nodes);
    }
    validate_records(&nodes, &synapses, tolerant)?;
    let brain = Brain::from_template_records(&nodes, &synapses, period)
        .map_err(|error| Error::invalid_data(format!("invalid template brain: {error}")))?;
    let template = TemplateMeta {
        name: file.name,
        species_name: file.species_name,
        description: file.description,
        is_official: file.is_official,
        node_anchors: file.node_anchors,
    };
    let meta = GenomeMeta {
        generation: file.generation.unwrap_or(0),
        ..GenomeMeta::default()
    };
    Ok(LoadedGenome {
        genome: migrate::genome_from_genes(genes),
        meta,
        brain,
        version,
        template,
        source: SourceShape::Template,
        is_ready: true,
        parent1: None,
        parent2: None,
        extra: file.extra.clone(),
        node_extras,
        synapse_extras,
        brain_extra: Extra::new(),
    })
}

fn template_from_core(
    genome: &Genome,
    meta: &GenomeMeta,
    brain: &Brain,
    template: &TemplateMeta,
) -> Result<TemplateFile> {
    let name = template.name.clone();
    let species_name = template
        .species_name
        .clone()
        .or_else(|| name.as_deref().map(to_species_format));
    Ok(TemplateFile {
        name,
        species_name,
        description: template.description.clone(),
        generation: Some(meta.generation),
        version: version::PRESENT.to_string(),
        is_official: template.is_official,
        nodes: nodes_to_json(brain, false),
        synapses: synapses_to_json(brain),
        genes: Some(GenesRepr::Object(genes_to_json(genome))),
        node_anchors: template.node_anchors.clone(),
        extra: Extra::new(),
    })
}

// ---------------------------------------------------------------------------
// .bb8 conversion
// ---------------------------------------------------------------------------

fn bb8_from_file(
    file: Bb8File,
    shape: SourceShape,
    tolerant: bool,
    period: f32,
) -> Result<LoadedGenome> {
    loaded_from_bb8(&file, shape, tolerant, period)
}

/// Builds a [`LoadedGenome`] from an already-parsed `.bb8` entity without a
/// second JSON serialization/parse (the save bundle parses each entity's JSON
/// once; reparsing it through `to_string` was a measured hotspot).
pub(crate) fn loaded_from_bb8(
    file: &Bb8File,
    shape: SourceShape,
    tolerant: bool,
    period: f32,
) -> Result<LoadedGenome> {
    let version_text = file
        .version
        .as_deref()
        .ok_or_else(|| Error::invalid_data(".bb8 file has no version"))?;
    let version = Version::parse(version_text)?;
    ensure_supported(version)?;
    let genes_repr = file
        .genes
        .genes
        .as_ref()
        .ok_or_else(|| Error::invalid_data(".bb8 file has no genes.genes object"))?;
    let remap_needed = version < Version::new(0, 6, 0, 0, 1);
    let genes_repr = if tolerant {
        pad_short_gene_array(genes_repr)
    } else {
        genes_repr.clone()
    };
    let genes = migrate::migrate_genes(version, &genes_repr, remap_needed, true)?;
    let node_extras: Vec<Extra> = file
        .brain
        .nodes
        .iter()
        .map(|node| node.extra.clone())
        .collect();
    let synapse_extras: Vec<Extra> = file
        .brain
        .synapses
        .iter()
        .map(|synapse| synapse.extra.clone())
        .collect();
    let brain_extra = file.brain.extra.clone();
    let (mut nodes, mut synapses) =
        records_from_json(&file.brain.nodes, &file.brain.synapses, tolerant)?;
    let migrated = migrate::migrate_brain(version, &mut nodes, &mut synapses)?;
    if migrated {
        migrate::normalize_base_nodes(&mut nodes);
    }
    validate_records(&nodes, &synapses, tolerant)?;
    let mut brain = Brain::from_records(&nodes, &synapses)
        .map_err(|error| Error::invalid_data(format!("invalid .bb8 brain: {error}")))?;
    if migrated {
        // A structural BrainUpdater migration moved output/hidden nodes and
        // leaves old runtime values attached to their old positions. The game
        // re-executes every non-input node once with zero input
        // (FinalizeEditing/ResumeBrain) after migrating; do the same so the
        // first observed outputs (e.g. Digestion) match the native client.
        brain.initialize_runtime_states(period);
    }

    let species_id = file.genes.species_id.and_then(|id| {
        u32::try_from(id)
            .ok()
            .filter(|id| *id > 0)
            .map(SpeciesId::new)
    });
    let meta = GenomeMeta {
        generation: file.genes.generation.unwrap_or(0),
        species: species_id,
        species_tag: file.genes.tag.clone(),
        is_mutant: file.genes.is_mutant.unwrap_or(false),
        is_rad: file.genes.is_rad.unwrap_or(false),
        is_rain: file.genes.is_rain.unwrap_or(false),
        is_killer: file.genes.is_killer.unwrap_or(false),
        is_source: file.genes.is_source.unwrap_or(false),
        deux_parent: false,
        parent1: None,
        parent2: None,
    };
    Ok(LoadedGenome {
        genome: migrate::genome_from_genes(genes),
        meta,
        brain,
        version,
        template: TemplateMeta::default(),
        source: shape,
        is_ready: file.genes.is_ready.unwrap_or(true),
        parent1: file.genes.parent1,
        parent2: file.genes.parent2,
        extra: file.extra.clone(),
        node_extras,
        synapse_extras,
        brain_extra,
    })
}

fn bb8_from_core(genome: &Genome, meta: &GenomeMeta, brain: &Brain) -> Bb8File {
    Bb8File {
        // Genome-only export: the game consumes `.bb8` files as template
        // sources (genes + brain); body/clock snapshots are added by I4.
        transform: None,
        rb2d: None,
        genes: Bb8Genes {
            tag: meta.species_tag.clone(),
            species_id: meta.species.map(|id| i64::from(id.value())),
            is_ready: Some(true),
            generation: Some(meta.generation),
            is_mutant: flag(meta.is_mutant),
            is_rad: flag(meta.is_rad),
            is_rain: flag(meta.is_rain),
            is_killer: flag(meta.is_killer),
            is_source: flag(meta.is_source),
            parent1: None,
            parent2: None,
            genes: Some(GenesRepr::Object(genes_to_json(genome))),
            extra: Extra::new(),
        },
        body: None,
        clock: None,
        egg: None,
        brain: BrainJson {
            is_ready: Some(true),
            nodes: nodes_to_json(brain, true),
            synapses: synapses_to_json(brain),
            extra: Extra::new(),
        },
        version: Some(version::PRESENT.to_string()),
        desc: None,
        extra: Extra::new(),
    }
}

fn flag(value: bool) -> Option<bool> {
    if value { Some(true) } else { None }
}

// ---------------------------------------------------------------------------
// Record <-> JSON helpers
// ---------------------------------------------------------------------------

fn records_from_json(
    nodes: &[NodeJson],
    synapses: &[SynapseJson],
    tolerant: bool,
) -> Result<(Vec<NodeRecord>, Vec<SynapseRecord>)> {
    if nodes.is_empty() {
        return Err(Error::invalid_data("brain has no nodes"));
    }
    let mut records = Vec::with_capacity(nodes.len());
    let mut declared_to_position = std::collections::BTreeMap::new();
    for (position, node) in nodes.iter().enumerate() {
        if let Some(index) = node.index {
            declared_to_position
                .entry(i64::from(index))
                .or_insert(position);
        }
    }
    for (position, node) in nodes.iter().enumerate() {
        let raw_type = node
            .node_type
            .ok_or_else(|| Error::invalid_data(format!("node {position} has no Type")))?;
        let node_type = NodeType::from_u8(raw_type).ok_or_else(|| {
            Error::invalid_data(format!("node {position} has unknown Type {raw_type}"))
        })?;
        if let Some(index) = node.index
            && index as usize != position
            && !tolerant
        {
            return Err(Error::invalid_data(format!(
                "node {position} declares Index {index} (must equal array position)"
            )));
        }
        let innovation = node
            .innovation
            .ok_or_else(|| Error::invalid_data(format!("node {position} has no Inov")))?;
        let desc = if position < N_INPUTS + N_OUTPUTS {
            None
        } else {
            node.desc.clone()
        };
        records.push(NodeRecord {
            node_type,
            index: position as u32,
            innovation,
            desc,
            base_activation: node.base_activation.unwrap_or(0.0),
            value: node.value.unwrap_or(0.0),
            last_input: node.last_input.unwrap_or(0.0),
            last_output: node.last_output.unwrap_or(0.0),
        });
    }
    let mut synapse_records = Vec::with_capacity(synapses.len());
    for (position, synapse) in synapses.iter().enumerate() {
        if tolerant && (synapse.node_in == u32::MAX || synapse.node_out == u32::MAX) {
            // The game loads -1 endpoint synapses but they can never fire;
            // drop them (strict mode still reports the out-of-range index).
            continue;
        }
        let mut node_in = synapse.node_in;
        let mut node_out = synapse.node_out;
        if node_in as usize >= nodes.len() || node_out as usize >= nodes.len() {
            if tolerant {
                // The game's load accepts files whose `Index` fields are shifted
                // relative to array positions; map declared indices back to the
                // array positions used by the rebuilt brain.
                if let Some(mapped) = declared_to_position.get(&i64::from(node_in)) {
                    node_in = *mapped as u32;
                }
                if let Some(mapped) = declared_to_position.get(&i64::from(node_out)) {
                    node_out = *mapped as u32;
                }
            }
            if node_in as usize >= nodes.len() || node_out as usize >= nodes.len() {
                return Err(Error::invalid_data(format!(
                    "synapse {position} endpoints ({} -> {}) out of range for {} nodes",
                    synapse.node_in,
                    synapse.node_out,
                    nodes.len()
                )));
            }
        }
        synapse_records.push(SynapseRecord {
            innovation: synapse.innovation,
            node_in,
            node_out,
            weight: synapse.weight,
            enabled: synapse.enabled,
        });
    }
    Ok((records, synapse_records))
}

/// The game's template updater allocates the full NGene array and fills the
/// leading values from older short arrays (e.g. the 28-value 0.5-era layout),
/// then the migration fills/remaps the rest. Tolerant mode mirrors that by
/// zero-padding short arrays to 34 before the gene migration runs.
fn pad_short_gene_array(repr: &GenesRepr) -> GenesRepr {
    match repr {
        GenesRepr::Array(values) if values.len() < 34 => {
            let mut padded = vec![0.0_f32; 34];
            for (slot, value) in padded.iter_mut().zip(values.iter()) {
                *slot = *value;
            }
            GenesRepr::Array(padded)
        }
        other => other.clone(),
    }
}

fn validate_records(
    nodes: &[NodeRecord],
    synapses: &[SynapseRecord],
    tolerant: bool,
) -> Result<()> {
    if nodes.len() < N_INPUTS + N_OUTPUTS {
        return Err(Error::invalid_data(format!(
            "brain has {} nodes (expected at least {})",
            nodes.len(),
            N_INPUTS + N_OUTPUTS
        )));
    }
    let mut seen = std::collections::BTreeSet::new();
    for synapse in synapses {
        if synapse.innovation > 0 && !seen.insert(synapse.innovation) && !tolerant {
            return Err(Error::invalid_data(format!(
                "duplicate synapse innovation {}",
                synapse.innovation
            )));
        }
    }
    Ok(())
}

fn nodes_to_json(brain: &Brain, runtime: bool) -> Vec<NodeJson> {
    brain
        .to_node_records()
        .into_iter()
        .enumerate()
        .map(|(position, record)| {
            let desc = brain
                .node_desc(position)
                .map(ToString::to_string)
                .or_else(|| base_node_name(position));
            NodeJson {
                node_type: Some(record.node_type as u8),
                index: Some(position as u32),
                innovation: Some(record.innovation),
                desc,
                base_activation: Some(record.base_activation),
                type_name: runtime.then(|| record.node_type.name().to_string()),
                value: runtime.then_some(record.value),
                last_input: runtime.then_some(record.last_input),
                last_output: runtime.then_some(record.last_output),
                archetype: runtime.then_some(archetype(position)),
                extra: Extra::new(),
            }
        })
        .collect()
}

fn synapses_to_json(brain: &Brain) -> Vec<SynapseJson> {
    brain
        .to_synapse_records()
        .into_iter()
        .map(|record| SynapseJson {
            innovation: record.innovation,
            node_in: record.node_in,
            node_out: record.node_out,
            weight: record.weight,
            enabled: record.enabled,
            extra: Extra::new(),
        })
        .collect()
}

fn apply_node_extras(nodes: &mut [NodeJson], extras: &[Extra]) {
    for (node, extra) in nodes.iter_mut().zip(extras) {
        for (key, value) in extra {
            node.extra
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
    }
}

fn apply_synapse_extras(synapses: &mut [SynapseJson], extras: &[Extra]) {
    for (synapse, extra) in synapses.iter_mut().zip(extras) {
        for (key, value) in extra {
            synapse
                .extra
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
    }
}

fn base_node_name(position: usize) -> Option<String> {
    if position < N_INPUTS {
        Input::from_index(position).map(|input| input.name().to_string())
    } else if position < N_INPUTS + N_OUTPUTS {
        Output::from_index(position - N_INPUTS).map(|output| output.name().to_string())
    } else {
        Some(format!("Hidden{}", position - N_INPUTS - N_OUTPUTS))
    }
}

fn archetype(position: usize) -> u32 {
    if position < N_INPUTS {
        0
    } else if position < N_INPUTS + N_OUTPUTS {
        1
    } else {
        2
    }
}

fn genes_to_json(genome: &Genome) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    for gene in ALL_GENES {
        map.insert(
            gene.def().name.to_string(),
            Value::from(f64::from(genome.genes[gene.index()])),
        );
    }
    map
}

/// The game's `StringExtend.ToSpeciesFormat`: capitalise the first word and
/// lower-case the second (or repeat the first word).
#[must_use]
pub fn to_species_format(name: &str) -> String {
    let mut words = name.split_whitespace();
    let first = words.next().unwrap_or("");
    let second = words.next().unwrap_or(first);
    format!("{} {}", capitalise(first), second.to_lowercase())
}

fn capitalise(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str().to_lowercase()),
        None => String::new(),
    }
}

fn ensure_supported(version: Version) -> Result<()> {
    // The game itself refuses pre-0.4 files; we additionally only implement
    // migrations from the 0.5 era onwards (spec 04 tier T2).
    let floor = Version::new(0, 5, 0, 0, 0);
    if version < floor {
        return Err(Error::unsupported(format!(
            "game version {version} is older than the supported .bb8 migration floor 0.5"
        )));
    }
    // Newer formats (0.6.3.3+, 0.6.4a1 folder saves, ...) are out of scope; fail
    // closed rather than applying stale migrations to unknown fields.
    let present = version::present();
    if version > present {
        return Err(Error::unsupported(format!(
            "game version {version} is newer than the supported present version {present}"
        )));
    }
    Ok(())
}

/// Returns the present game version (`0.6.3.1`).
#[must_use]
pub fn present_version() -> Version {
    version::present()
}

/// Builds a JSON object from `key: value` pairs (helper for tests/other slices).
#[must_use]
pub fn json_object(entries: &[(&str, Value)]) -> Value {
    let mut map = serde_json::Map::new();
    for (key, value) in entries {
        map.insert((*key).to_string(), value.clone());
    }
    json!(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bibites_core::genome::Genome;

    fn starter() -> (Genome, GenomeMeta, Brain) {
        (Genome::defaults(), GenomeMeta::default(), Brain::starter())
    }

    #[test]
    fn genome_json_round_trips_numbers_bitwise() {
        let (genome, mut meta, brain) = starter();
        meta.generation = 7;
        meta.species_tag = Some("Test tag".to_string());
        let text = genome_to_json(&genome, &meta, &brain).unwrap();
        let loaded = from_json(&text).unwrap();
        assert_eq!(loaded.genome, genome);
        assert_eq!(loaded.brain, brain);
        assert_eq!(loaded.meta.generation, 7);
        assert_eq!(loaded.meta.species_tag.as_deref(), Some("Test tag"));
        assert_eq!(loaded.version, version::present());
        assert_eq!(loaded.source, SourceShape::GenomeOnlyBb8);
    }

    #[test]
    fn template_round_trip() {
        let (genome, meta, brain) = starter();
        let template = TemplateMeta {
            name: Some("Sample".to_string()),
            species_name: Some("Sample sample".to_string()),
            description: Some("desc".to_string()),
            is_official: true,
            node_anchors: [(
                1,
                AnchorJson {
                    x: 0.0,
                    y: 1.0,
                    extra: Extra::new(),
                },
            )]
            .into_iter()
            .collect(),
        };
        let text = template_to_json(&genome, &meta, &brain, &template).unwrap();
        assert!(text.contains("\"nodeAnchors\""));
        assert!(!text.contains("\"body\""));
        let loaded = from_json(&text).unwrap();
        assert_eq!(loaded.source, SourceShape::Template);
        assert_eq!(loaded.template, template);
        assert_eq!(loaded.genome, genome);
        assert_eq!(loaded.brain, brain);
        assert_eq!(loaded.version, version::present());
        // Writing the loaded file again is byte-stable.
        let text2 = template_to_json(
            &loaded.genome,
            &loaded.meta,
            &loaded.brain,
            &loaded.template,
        )
        .unwrap();
        assert_eq!(text, text2);
    }

    #[test]
    fn inspect_summary_fields() {
        let (genome, meta, brain) = starter();
        let dir = std::env::temp_dir();
        let path = dir.join("rustibites_i1_inspect.bb8");
        save_genome(&path, &genome, &meta, &brain).unwrap();
        let summary = inspect(&path).unwrap();
        assert_eq!(summary.gene_count, 35);
        assert_eq!(summary.node_count, N_INPUTS + N_OUTPUTS);
        assert_eq!(summary.synapse_count, 3);
        assert_eq!(summary.input_count, 33);
        assert_eq!(summary.output_count, 15);
        assert_eq!(summary.hidden_count, 0);
        assert_eq!(summary.version, "0.6.3.1");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn detect_shape_works() {
        let (genome, meta, brain) = starter();
        let text = genome_to_json(&genome, &meta, &brain).unwrap();
        assert_eq!(detect_shape(&text), Some(SourceShape::GenomeOnlyBb8));
        let template = template_to_json(&genome, &meta, &brain, &TemplateMeta::default()).unwrap();
        assert_eq!(detect_shape(&template), Some(SourceShape::Template));
        assert_eq!(detect_shape("{}"), None);
        assert_eq!(detect_shape("not json"), None);
    }

    #[test]
    fn malformed_inputs_are_errors() {
        assert!(from_json("not json").is_err());
        assert!(from_json("{}").is_err());
        assert!(from_json(r#"{"nodes":[],"genes":{},"version":"0.6.3.1"}"#).is_err());
        assert!(from_json(r#"{"nodes":[],"synapses":[],"genes":{},"version":"nope"}"#).is_err());
        assert!(from_json(r#"{"nodes":[],"synapses":[],"genes":{}}"#).is_err());
        assert!(
            from_json(
                "{\"nodes\":[{\"Type\":0}],\"synapses\":[],\"genes\":{},\"version\":\"0.6.3.1\"}"
            )
            .is_err()
        );
    }

    #[test]
    fn species_format_matches_the_game() {
        assert_eq!(to_species_format("sample"), "Sample sample");
        assert_eq!(to_species_format("two Words"), "Two words");
        assert_eq!(to_species_format("Single"), "Single single");
    }

    #[test]
    fn version_floor_is_enforced() {
        let (genome, meta, brain) = starter();
        let text = {
            let mut file = bb8_from_core(&genome, &meta, &brain);
            file.version = Some("0.3".to_string());
            serde_json::to_string(&file).unwrap()
        };
        assert!(from_json(&text).is_err());
    }
    #[test]
    fn migrated_save_reinitializes_brain_outputs_like_game() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/parity/default/rust_seed_600.zip");
        if !path.is_file() {
            return;
        }
        let bundle = load_save_bundle(&path).expect("load migrated save");
        let brain = &bundle.bibites[0].brain;
        assert!(
            (brain.output_value(Output::Digestion) - 0.773_468_3).abs() < 1e-5,
            "migrated save must re-execute non-input nodes: Digestion={}",
            brain.output_value(Output::Digestion)
        );
    }
}

#[cfg(test)]
mod starter_template_tests {
    use super::*;
    use bibites_core::genome::Gene;

    #[test]
    fn starter_template_is_editable_and_round_trips() {
        let mut template = LoadedGenome::starter_template("My Bibite");
        assert_eq!(template.source, SourceShape::Template);
        assert_eq!(template.template.name.as_deref(), Some("My Bibite"));
        assert_eq!(template.template.species_name.as_deref(), Some("My bibite"));
        assert_eq!(template.genome, Genome::starter());
        assert_eq!(template.brain.node_count(), Brain::starter().node_count());

        template.genome.set_gene(Gene::LayTime, 29.0);
        template.set_node_anchor(33, 10.0, 20.0);
        assert_eq!(template.node_anchor(33), Some((10.0, 20.0)));
        assert_eq!(template.template.node_anchors.len(), 1);
        let hidden = template
            .brain
            .add_hidden_node(bibites_core::brain::NodeType::TanH, Some("Library"))
            .expect("add hidden");
        assert!(template.brain.node_desc(hidden) == Some("Library"));
        // Connected hidden nodes survive template load; a floating one is
        // intentionally repaired away by template semantics.
        template
            .brain
            .add_synapse_connection(0, hidden, 1.0, true)
            .expect("input -> hidden");
        template
            .brain
            .add_synapse_connection(hidden, bibites_core::brain::FIRST_OUTPUT, 1.0, true)
            .expect("hidden -> output");

        let json = template.to_json().expect("serialize starter template");
        assert!(json.contains("\"nodes\""));
        let reloaded = from_json(&json).expect("reload starter template");
        assert_eq!(reloaded.genome, template.genome);
        assert_eq!(reloaded.template.name, template.template.name);
        assert_eq!(reloaded.node_anchor(33), Some((10.0, 20.0)));
        assert_eq!(reloaded.brain.node_count(), template.brain.node_count());
    }

    #[test]
    fn described_saves_carry_the_description_and_indent_on_request() {
        use bibites_core::brain::Brain;
        use bibites_core::genome::{Genome, GenomeMeta};
        use bibites_core::world::TemplateSource;
        let source = TemplateSource::new(
            "Wolf",
            Genome::starter(),
            GenomeMeta::default(),
            Brain::starter(),
        );
        let dir = std::env::temp_dir().join(format!("rustibites-described-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let compact = dir.join("compact.bb8template");
        let pretty = dir.join("pretty.bb8template");
        save_template_source_described(&compact, &source, "a hunter", false).expect("compact");
        save_template_source_described(&pretty, &source, "a hunter", true).expect("pretty");
        let compact_text = std::fs::read_to_string(&compact).expect("read");
        let pretty_text = std::fs::read_to_string(&pretty).expect("read");
        // Indented output has line breaks; compact has none and is the shorter one.
        assert!(!compact_text.contains('\n'));
        assert!(pretty_text.contains('\n') && pretty_text.len() > compact_text.len());
        // Both carry the name and description and load back as the same template.
        for path in [&compact, &pretty] {
            let json: Value =
                serde_json::from_str(&std::fs::read_to_string(path).expect("read")).expect("json");
            assert_eq!(json["name"], "Wolf");
            assert_eq!(json["description"], "a hunter");
            let loaded = load_template(path).expect("loads");
            assert_eq!(loaded.genome.values(), source.genome.values());
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod write_replacing_tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rustibites-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        dir
    }

    fn beside(path: &Path) -> std::path::PathBuf {
        let mut name = path.as_os_str().to_owned();
        name.push(".part");
        std::path::PathBuf::from(name)
    }

    #[test]
    fn a_write_that_fails_part_way_leaves_the_old_file_as_it_was() {
        let dir = scratch("replacing-failed");
        let path = dir.join("save.zip");
        std::fs::write(&path, b"the old save").expect("old");
        let failed: Result<()> = write_replacing(&path, |at| {
            std::fs::write(at, b"half of the new sa").map_err(Error::from)?;
            Err(Error::invalid_data("the disk is full"))
        });
        assert!(failed.is_err());
        assert_eq!(std::fs::read(&path).expect("the old file"), b"the old save");
        assert!(!beside(&path).exists(), "what was half written is removed");
        // A failure before anything was written is the same.
        let early: Result<()> = write_replacing(&path, |_| Err(Error::invalid_data("cannot serialize")));
        assert!(early.is_err());
        assert_eq!(std::fs::read(&path).expect("the old file"), b"the old save");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_whole_write_takes_the_old_files_place_and_leaves_nothing_beside() {
        let dir = scratch("replacing-whole");
        let path = dir.join("save.zip");
        // No old file: the new one is made. A leftover half file from a killed run is written over.
        std::fs::write(beside(&path), b"half of a file").expect("leftover");
        let made = write_replacing(&path, |at| std::fs::write(at, b"first").map_err(Error::from).map(|()| 7)).expect("written");
        assert_eq!(made, 7, "what the writer answers is handed back");
        assert_eq!(std::fs::read(&path).expect("file"), b"first");
        assert!(!beside(&path).exists());
        // An old file: replaced whole.
        write_bytes_replacing(&path, b"second, and longer than the first").expect("replaced");
        assert_eq!(std::fs::read(&path).expect("file"), b"second, and longer than the first");
        write_bytes_replacing(&path, "3rd").expect("replaced");
        assert_eq!(std::fs::read(&path).expect("file"), b"3rd", "a shorter file leaves no tail of the old one");
        assert!(!beside(&path).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_save_bundle_written_over_another_file_loads_back_and_leaves_nothing_beside() {
        let dir = scratch("replacing-bundle");
        let path = dir.join("quick.zip");
        std::fs::write(&path, b"not a zip at all").expect("old");
        let world = bibites_core::world::World::new(
            bibites_core::settings::SimSettings::game_defaults(),
            bibites_core::world::ScenarioData::default(),
            3,
        )
        .expect("world");
        let mut contents = world.to_contents();
        // (A file needs the game version it was made with: the viewer sets it before every save.)
        contents.scene.version = "0.6.3.1".to_owned();
        let bundle = save_bundle_from_contents(&contents, None).expect("bundle");
        write_save_bundle(&bundle, &path).expect("saved over the old file");
        assert!(!beside(&path).exists());
        let loaded = load_save_bundle(&path).expect("the new file is a save");
        assert_eq!(loaded.bibites.len(), bundle.bibites.len());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

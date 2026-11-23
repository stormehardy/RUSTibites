//! Scenario and save `.zip` bundles.
//!
//! Spec: `docs/spec/04-formats-settings.md` sections 5 (save bundle), 6
//! (scenario bundle), 8.2; `docs/INTERFACES.md` section 8.
//!
//! * [`load_scenario_bundle`] reads `scenario.info` + settings + bundled
//!   templates and builds a core [`ScenarioData`] (zones/zone-groups/spawn
//!   rows) plus effective [`bibites_core::settings::SimSettings`] with the `recommended` partial
//!   semantics.
//! * [`load_save_bundle`] reads every section of a save: settings, scene,
//!   pellets, pheromones, `bibites/*` / `eggs/*`, species lineage, opaque
//!   `data.bin`, templates and unknown entries.
//! * [`write_save_bundle`] / [`write_scenario_bundle`] emit zips the original
//!   game can load (Deflate, UTF-8 JSON entries with BOM, forward-slash names,
//!   deterministic timestamps).
//!
//! Unknown JSON fields are preserved (`extra` maps / raw values), unknown zip
//! entries are kept verbatim. Scene/pellet/pheromone sections are kept as raw
//! JSON values plus parsed summary fields so that no data is lost before the
//! World load path (I4) consumes them; bibites and eggs are converted to the
//! core plain records from `bibites_core::body`.
//!
//! ## Deliberate I3 simplifications (reported to the Lead)
//!
//! * `data.bin` is opaque (`Vec<u8>`) in both directions.
//! * Template entries are kept as raw bytes and also parsed into
//!   [`crate::LoadedGenome`] when they are `.bb8template`/`.bb8` files.
//! * Raw runtime entity ids that cannot be represented by generational core
//!   ids (`genes.parent1/2`, `eggLayer.children`, `egg.id`) are preserved in
//!   side vectors and re-emitted on write.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use serde_json::{Map, Value};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use bibites_core::body::{BibiteSave, EggSave, StomachContentSave};

/// The game stores at most two stomach content slots (`StomachContent[2]`).
const STOMACH_SLOTS_IO: usize = 2;
use bibites_core::brain::{Brain, NodeType};
use bibites_core::genome::{Gene, Genome};
use bibites_core::math::Vec2;
use bibites_core::organs::egg_required_energy;
use bibites_core::settings::{MaterialKind, SimSettings};
use bibites_core::species::{LineageRecordIo, SpeciesRecordIo, SpeciesTemplateRecord};
use bibites_core::world::ScenarioData;
use bibites_core::{Error, Result};

use crate::model::{AnchorJson, Bb8File, Extra, GenesRepr, NodeJson, SynapseJson};
use crate::settings::{
    ScenarioSettingsFile, SettingsMode, parse_scenario_settings, resolve_effective_settings,
};
use crate::{LoadedGenome, TemplateMeta, Version};

/// `scenario.info` metadata (spec 04 section 6.2).
#[derive(Clone, Debug, PartialEq)]
pub struct ScenarioInfo {
    /// Scenario title.
    pub name: String,
    /// Description.
    pub description: String,
    /// Version string (parsed for validation; preserved verbatim on write).
    pub version: Version,
    /// Raw version string as written in the file.
    pub version_text: String,
    /// Shipped-scenario flag.
    pub is_official: bool,
    /// Sort rank of official scenarios.
    pub rank: Option<i32>,
    /// Challenge flag.
    pub is_challenge: bool,
    /// Human-readable star objectives (challenges only).
    pub stars: [Option<String>; 3],
    /// Unknown metadata keys, preserved.
    pub extra: Extra,
}

impl ScenarioInfo {
    fn from_json(value: &Value) -> Result<Self> {
        let object = value
            .as_object()
            .ok_or_else(|| Error::invalid_data("scenario.info is not an object"))?;
        let version_text = object
            .get("version")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::invalid_data("scenario.info has no version"))?
            .to_string();
        let version = Version::parse(&version_text)?;
        let mut extra = Extra::new();
        for (key, value) in object {
            if !matches!(
                key.as_str(),
                "name"
                    | "desc"
                    | "version"
                    | "isOfficial"
                    | "rank"
                    | "isChallenge"
                    | "star1"
                    | "star2"
                    | "star3"
            ) {
                extra.insert(key.clone(), value.clone());
            }
        }
        Ok(Self {
            name: object
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            description: object
                .get("desc")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            version,
            version_text,
            is_official: object
                .get("isOfficial")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            rank: object
                .get("rank")
                .and_then(Value::as_i64)
                .map(|rank| rank as i32),
            is_challenge: object
                .get("isChallenge")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            stars: [
                object
                    .get("star1")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                object
                    .get("star2")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                object
                    .get("star3")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            ],
            extra,
        })
    }

    fn to_json(&self) -> Value {
        let mut object = Map::new();
        object.insert("name".to_string(), Value::from(self.name.clone()));
        object.insert("desc".to_string(), Value::from(self.description.clone()));
        object.insert(
            "version".to_string(),
            Value::from(self.version_text.clone()),
        );
        if self.is_official {
            object.insert("isOfficial".to_string(), Value::Bool(true));
        }
        if let Some(rank) = self.rank {
            object.insert("rank".to_string(), Value::from(i64::from(rank)));
        }
        if self.is_challenge {
            object.insert("isChallenge".to_string(), Value::Bool(true));
            for (index, star) in self.stars.iter().enumerate() {
                if let Some(text) = star {
                    object.insert(format!("star{}", index + 1), Value::from(text.clone()));
                }
            }
        }
        for (key, value) in &self.extra {
            object.entry(key.clone()).or_insert_with(|| value.clone());
        }
        Value::Object(object)
    }
}

/// A loaded scenario bundle.
pub struct ScenarioBundle {
    /// Effective settings of a new world started from this scenario.
    pub settings: SimSettings,
    /// Core scenario data (zones, groups, spawn rows, tools).
    pub scenario: ScenarioData,
    /// Parsed settings file (retains challenge, animators and extras).
    pub settings_file: ScenarioSettingsFile,
    /// Scenario metadata.
    pub info: ScenarioInfo,
    /// Bundled templates parsed into memory (`name -> genome`).
    pub templates: Vec<(String, LoadedGenome)>,
    /// Bundled template bytes (for writing/copying, keyed by zip entry name).
    pub template_bytes: Vec<(String, Vec<u8>)>,
    /// Optional preview image bytes.
    pub img_png: Option<Vec<u8>>,
    /// Unrecognized zip entries, preserved verbatim.
    pub extra_entries: Vec<(String, Vec<u8>)>,
}

/// A loaded save bundle (plain data; no World dependency in I3).
pub struct SaveBundle {
    /// Effective settings (save `independents` precedence).
    pub settings: SimSettings,
    /// Parsed save settings file (retains extras).
    pub settings_file: ScenarioSettingsFile,
    /// Raw `scene.bb8scene` JSON (version/seed/time/tools preserved).
    pub scene: Value,
    /// Parsed scene version.
    pub version: Version,
    /// Raw `vars.bb8scene`, when present.
    pub vars: Option<Value>,
    /// Raw `pellets.bb8scene`, when present.
    pub pellets: Option<Value>,
    /// Raw `pheromones.bb8scene`, when present.
    pub pheromones: Option<Value>,
    /// Living bibites/corpses (`bibites/*.bb8`).
    pub bibites: Vec<BibiteSave>,
    /// Per-bibite raw runtime body ids (`body.id`), preserved for write-back.
    pub bibite_ids: Vec<i32>,
    /// Per-bibite preserved unknown JSON fields.
    pub bibite_extras: Vec<Extra>,
    /// Per-bibite raw runtime parent ids `(parent1, parent2)`.
    pub bibite_parents: Vec<(Option<i32>, Option<i32>)>,
    /// Per-bibite raw `eggLayer.children` ids.
    pub bibite_children: Vec<Vec<i32>>,
    /// Eggs (`eggs/*.bb8`).
    pub eggs: Vec<EggSave>,
    /// Per-egg preserved unknown JSON fields.
    pub egg_extras: Vec<Extra>,
    /// Per-egg raw ids.
    pub egg_ids: Vec<i32>,
    /// Per-egg raw runtime parent ids `(parent1, parent2)` (`genes.parent1` / `genes.parent2`).
    pub egg_parents: Vec<(Option<i32>, Option<i32>)>,
    /// Species lineage (`speciesData.json`), when present.
    pub species: Option<LineageRecordIo>,
    /// Raw species JSON when parsing into records failed/unavailable.
    pub species_raw: Option<Value>,
    /// Opaque statistics history (`data.bin`).
    pub data_bin: Option<Vec<u8>>,
    /// Bundled template bytes keyed by entry name.
    pub templates: Vec<(String, Vec<u8>)>,
    /// Optional screenshot bytes.
    pub img_png: Option<Vec<u8>>,
    /// Unrecognized zip entries, preserved verbatim.
    pub unknown_entries: Vec<(String, Vec<u8>)>,
}

// ---------------------------------------------------------------------------
// Zip helpers
// ---------------------------------------------------------------------------

fn normalize_entry_name(name: &str) -> String {
    name.replace('\\', "/")
}

fn read_entry_bytes(
    archive: &mut ZipArchive<std::fs::File>,
    name: &str,
) -> Result<Option<Vec<u8>>> {
    let target = normalize_entry_name(name);
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| Error::invalid_data(format!("zip entry error: {error}")))?;
        if normalize_entry_name(entry.name()) != target {
            continue;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| Error::Io(error.to_string()))?;
        return Ok(Some(bytes));
    }
    Ok(None)
}

fn entry_names(archive: &mut ZipArchive<std::fs::File>) -> Result<Vec<String>> {
    let mut names = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| Error::invalid_data(format!("zip entry error: {error}")))?;
        names.push(normalize_entry_name(entry.name()));
    }
    Ok(names)
}

fn read_entry(archive: &mut ZipArchive<std::fs::File>, index: usize) -> Result<Vec<u8>> {
    let mut entry = archive
        .by_index(index)
        .map_err(|error| Error::invalid_data(format!("zip entry error: {error}")))?;
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut bytes)
        .map_err(|error| Error::Io(error.to_string()))?;
    Ok(bytes)
}

fn parse_json_bytes(bytes: &[u8], name: &str) -> Result<Value> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| Error::invalid_data(format!("{name} is not UTF-8: {error}")))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    serde_json::from_str(text)
        .map_err(|error| Error::invalid_data(format!("invalid JSON in {name}: {error}")))
}

fn to_json_bytes(value: &Value, bom: bool) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|error| Error::invalid_data(format!("cannot serialize JSON: {error}")))?;
    if bom {
        let mut out = Vec::with_capacity(bytes.len() + 3);
        out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        out.append(&mut bytes);
        Ok(out)
    } else {
        Ok(bytes)
    }
}

fn write_zip(path: &Path, entries: &[(String, Vec<u8>)]) -> Result<()> {
    crate::write_replacing(path, |beside| write_zip_in_place(beside, entries))
}

/// [`write_zip`] straight into `path`.
fn write_zip_in_place(path: &Path, entries: &[(String, Vec<u8>)]) -> Result<()> {
    let file = std::fs::File::create(path).map_err(Error::from)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    for (name, bytes) in entries {
        zip.start_file(normalize_entry_name(name), options)
            .map_err(|error| Error::invalid_data(format!("zip write error: {error}")))?;
        zip.write_all(bytes)
            .map_err(|error| Error::Io(error.to_string()))?;
    }
    zip.finish()
        .map_err(|error| Error::invalid_data(format!("zip finish error: {error}")))?;
    Ok(())
}

fn open_zip(path: &Path) -> Result<ZipArchive<std::fs::File>> {
    let file = std::fs::File::open(path).map_err(Error::from)?;
    ZipArchive::new(file).map_err(|error| Error::invalid_data(format!("invalid zip: {error}")))
}

// ---------------------------------------------------------------------------
// Scenario bundles
// ---------------------------------------------------------------------------

/// Lightweight scenario list entry (`ScenarioItemReference.InitScenarioItem` plus
/// the preview/description `ScenarioSelectorPanel.SelectScenarioItem` shows).
#[derive(Debug, Clone)]
pub struct ScenarioCard {
    /// `scenario.info`.
    pub info: ScenarioInfo,
    /// First `.png` entry of the archive (the preview image), when present.
    pub preview_png: Option<Vec<u8>>,
}

/// Reads only `scenario.info` and the preview image of a scenario archive.
///
/// Returns `Ok(None)` for scenarios older than 0.5.1, which the game's list hides.
pub fn read_scenario_card(path: impl AsRef<Path>) -> Result<Option<ScenarioCard>> {
    let mut archive = open_zip(path.as_ref())?;
    let info_bytes = read_entry_bytes(&mut archive, "scenario.info")?
        .ok_or_else(|| Error::invalid_data("scenario bundle has no scenario.info"))?;
    let info = ScenarioInfo::from_json(&parse_json_bytes(&info_bytes, "scenario.info")?)?;
    if Version::parse("0.5.1").is_ok_and(|floor| crate::version::before(info.version, floor)) {
        return Ok(None);
    }
    let mut preview_png = None;
    for index in 0..archive.len() {
        let is_png = archive
            .by_index(index)
            .map(|entry| entry.name().to_ascii_lowercase().ends_with(".png"))
            .unwrap_or(false);
        if is_png {
            preview_png = Some(read_entry(&mut archive, index)?);
            break;
        }
    }
    Ok(Some(ScenarioCard { info, preview_png }))
}

/// The numeric values a scenario's `settings.bb8settings` defines for `keys`
/// (`None` where the file does not mention the setting; official 0.6 scenarios do
/// not define the TPS settings, so the New Game panel's own values apply).
pub fn scenario_defined_numbers(path: impl AsRef<Path>, keys: &[&str]) -> Result<Vec<Option<f64>>> {
    fn find(value: &Value, key: &str) -> Option<f64> {
        match value {
            Value::Object(map) => {
                if let Some(found) = map.get(key) {
                    if let Some(number) = found.as_f64() {
                        return Some(number);
                    }
                    if let Some(number) = found.get("Value").and_then(Value::as_f64) {
                        return Some(number);
                    }
                }
                map.values().find_map(|child| find(child, key))
            }
            Value::Array(items) => items.iter().find_map(|child| find(child, key)),
            _ => None,
        }
    }
    let mut archive = open_zip(path.as_ref())?;
    let bytes = read_entry_bytes(&mut archive, "settings.bb8settings")?
        .ok_or_else(|| Error::invalid_data("scenario bundle has no settings.bb8settings"))?;
    let json = parse_json_bytes(&bytes, "settings.bb8settings")?;
    Ok(keys.iter().map(|key| find(&json, key)).collect())
}

/// Loads a scenario `.zip` into core scenario data and effective settings.
pub fn load_scenario_bundle(path: impl AsRef<Path>) -> Result<ScenarioBundle> {
    let path = path.as_ref();
    let mut archive = open_zip(path)?;

    let info_bytes = read_entry_bytes(&mut archive, "scenario.info")?
        .ok_or_else(|| Error::invalid_data("scenario bundle has no scenario.info"))?;
    let info = ScenarioInfo::from_json(&parse_json_bytes(&info_bytes, "scenario.info")?)?;

    let settings_bytes = read_entry_bytes(&mut archive, "settings.bb8settings")?
        .ok_or_else(|| Error::invalid_data("scenario bundle has no settings.bb8settings"))?;
    let settings_text = std::str::from_utf8(&settings_bytes)
        .map_err(|error| Error::invalid_data(format!("settings not UTF-8: {error}")))?;
    let settings_file = parse_scenario_settings(settings_text)?;

    // Zones/spawns come from the settings file; 0.6.3.1 scenarios contain no
    // initial pellets or tools (0.6.3.3 "scenario with starting state" bundles
    // are newer format and are rejected by the version floor below).
    let recommended = settings_file.recommended.clone();
    let settings = resolve_effective_settings(
        settings_file.scenario.clone(),
        settings_file.materials,
        None,
        None,
        recommended,
    );
    let scenario = ScenarioData {
        name: info.name.clone(),
        version: info.version_text.clone(),
        zones: settings_file.zones.clone(),
        zone_groups: settings_file.zone_groups.clone(),
        bibite_spawns: settings_file.spawns.clone(),
        settings_changers: settings_file.changers.clone(),
        initial_pellets: Vec::new(),
        rad_towers: Vec::new(),
        phero_towers: Vec::new(),
        color_killers: Vec::new(),
        tags: Vec::new(),
    };

    let names = entry_names(&mut archive)?;
    let mut templates = Vec::new();
    let mut template_bytes = Vec::new();
    let mut img_png = None;
    let mut extra_entries = Vec::new();
    for (index, raw_name) in names.iter().enumerate() {
        let lower = raw_name.to_ascii_lowercase();
        let bytes = read_entry(&mut archive, index)?;
        if lower == "img.png" || lower.ends_with("/img.png") {
            img_png = Some(bytes);
            continue;
        }
        if raw_name == "scenario.info"
            || raw_name == "settings.bb8settings"
            || lower == "settings.json"
        {
            continue;
        }
        if lower.ends_with(".bb8template") || lower.ends_with(".bb8") {
            let text = std::str::from_utf8(&bytes).map_err(|error| {
                Error::invalid_data(format!("template {raw_name} is not UTF-8: {error}"))
            })?;
            let loaded =
                crate::from_json_with_period(text, SimSettings::game_defaults().brain_period())?;
            let name = raw_name.rsplit('/').next().unwrap_or(raw_name).to_string();
            templates.push((name, loaded));
            template_bytes.push((raw_name.clone(), bytes));
            continue;
        }
        extra_entries.push((raw_name.clone(), bytes));
    }

    Ok(ScenarioBundle {
        settings,
        scenario,
        settings_file,
        info,
        templates,
        template_bytes,
        img_png,
        extra_entries,
    })
}

/// Writes a scenario bundle the game can load.
pub fn write_scenario_bundle(bundle: &ScenarioBundle, path: impl AsRef<Path>) -> Result<()> {
    let mut file = bundle.settings_file.clone();
    file.zones = bundle.scenario.zones.clone();
    file.zone_groups = bundle.scenario.zone_groups.clone();
    file.spawns = bundle.scenario.bibite_spawns.clone();
    file.changers = bundle.scenario.settings_changers.clone();
    file.recommended = Some(bundle.settings.independents.clone());
    let mut entries = Vec::new();
    entries.push((
        "settings.bb8settings".to_string(),
        to_json_bytes(
            &serde_json::from_str(&file.to_json(SettingsMode::Scenario)?)
                .map_err(|error| Error::invalid_data(format!("settings JSON error: {error}")))?,
            true,
        )?,
    ));
    entries.push((
        "scenario.info".to_string(),
        to_json_bytes(&bundle.info.to_json(), true)?,
    ));
    if let Some(img) = &bundle.img_png {
        entries.push(("img.png".to_string(), img.clone()));
    }
    for (name, bytes) in &bundle.template_bytes {
        entries.push((name.clone(), bytes.clone()));
    }
    for (name, bytes) in &bundle.extra_entries {
        entries.push((name.clone(), bytes.clone()));
    }
    write_zip(path.as_ref(), &entries)
}

// ---------------------------------------------------------------------------
// Save bundles
// ---------------------------------------------------------------------------

/// Loads a save `.zip` into a plain [`SaveBundle`].
pub fn load_save_bundle(path: impl AsRef<Path>) -> Result<SaveBundle> {
    let path = path.as_ref();
    let mut archive = open_zip(path)?;

    let settings_bytes = read_entry_bytes(&mut archive, "settings.bb8settings")?
        .or(read_entry_bytes(&mut archive, "settings.json")?)
        .ok_or_else(|| Error::invalid_data("save bundle has no settings section"))?;
    let settings_text = std::str::from_utf8(&settings_bytes)
        .map_err(|error| Error::invalid_data(format!("settings not UTF-8: {error}")))?;
    let settings_file = parse_scenario_settings(settings_text)?;
    let settings = resolve_effective_settings(
        settings_file.scenario.clone(),
        settings_file.materials,
        settings_file.independents.clone(),
        None,
        settings_file.recommended.clone(),
    );

    let scene_bytes = read_entry_bytes(&mut archive, "scene.bb8scene")?
        .or(read_entry_bytes(&mut archive, "scene.json")?)
        .ok_or_else(|| Error::invalid_data("save bundle has no scene section"))?;
    let scene = parse_json_bytes(&scene_bytes, "scene.bb8scene")?;
    let version_text = scene
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::invalid_data("scene has no version"))?;
    let version = Version::parse(version_text)?;
    if version < Version::new(0, 5, 0, 0, 0) {
        return Err(Error::unsupported(format!(
            "save version {version} is older than the supported floor 0.5"
        )));
    }

    let vars = read_entry_bytes(&mut archive, "vars.bb8scene")?
        .map(|bytes| parse_json_bytes(&bytes, "vars.bb8scene"))
        .transpose()?;
    let pellets = read_entry_bytes(&mut archive, "pellets.bb8scene")?
        .map(|bytes| parse_json_bytes(&bytes, "pellets.bb8scene"))
        .transpose()?;
    let pheromones = read_entry_bytes(&mut archive, "pheromones.bb8scene")?
        .map(|bytes| parse_json_bytes(&bytes, "pheromones.bb8scene"))
        .transpose()?;
    let data_bin = read_entry_bytes(&mut archive, "data.bin")?;
    let img_png = read_entry_bytes(&mut archive, "img.png")?;
    let species_raw = read_entry_bytes(&mut archive, "speciesData.json")?
        .map(|bytes| parse_json_bytes(&bytes, "speciesData.json"))
        .transpose()?;

    let names = entry_names(&mut archive)?;
    let mut bibites = Vec::new();
    let mut bibite_ids = Vec::new();
    let mut bibite_extras = Vec::new();
    let mut bibite_parents = Vec::new();
    let mut bibite_children = Vec::new();
    let mut eggs = Vec::new();
    let mut egg_extras = Vec::new();
    let mut egg_ids = Vec::new();
    let mut egg_parents = Vec::new();
    let mut templates = Vec::new();
    let mut unknown_entries = Vec::new();
    let mut species_bytes = None;
    let mut left_out_bibites = 0usize;
    let mut left_out_eggs = 0usize;

    for (index, name) in names.iter().enumerate() {
        let lower = name.to_ascii_lowercase();
        if name == "settings.bb8settings"
            || name == "scene.bb8scene"
            || name == "vars.bb8scene"
            || name == "pellets.bb8scene"
            || name == "pheromones.bb8scene"
            || name == "data.bin"
            || name == "img.png"
            || lower == "settings.json"
            || lower == "scene.json"
        {
            continue;
        }
        let bytes = read_entry(&mut archive, index)?;
        if lower == "speciesdata.json" {
            species_bytes = Some(bytes);
            continue;
        }
        if lower.starts_with("bibites/") || lower.starts_with("bibytes/") {
            // A save written by a build before the NaN fix can hold a quarter of a million bibites of NaN position (the glitch save of
            // 2026-11-13): they are left out without the parse (the position is in the first bytes of the file).
            if has_a_null_position(&bytes) {
                left_out_bibites += 1;
                continue;
            }
            let value = parse_json_bytes(&bytes, name)?;
            let (save, extra, parents, children, body_id) =
                parse_bibite(&value, version_text, &settings)?;
            // A bibite whose position, size, health or energy is not a number (saved as `null`) cannot be put back in a world: a NaN
            // position stays NaN for good, and a size that is not a size is no body. Builds before 2026-11-13 let such bibites live
            // and breed (the glitch save of that day had 257,000); they are left out.
            if !bibite_numbers_are_usable(&save) {
                left_out_bibites += 1;
                continue;
            }
            bibites.push(save);
            bibite_ids.push(body_id);
            bibite_extras.push(extra);
            bibite_parents.push(parents);
            bibite_children.push(children);
            continue;
        }
        if lower.starts_with("eggs/") {
            if has_a_null_position(&bytes) {
                left_out_eggs += 1;
                continue;
            }
            let value = parse_json_bytes(&bytes, name)?;
            let (save, extra, parents, id) = parse_egg(&value, version_text, &settings)?;
            if !(save.pos.x.is_finite() && save.pos.y.is_finite() && save.energy.is_finite()) {
                left_out_eggs += 1;
                continue;
            }
            eggs.push(save);
            egg_extras.push(extra);
            egg_parents.push(parents);
            egg_ids.push(id);
            continue;
        }
        if lower.starts_with("templates/")
            || lower.ends_with(".bb8template")
            || lower.ends_with(".bb8")
        {
            templates.push((name.clone(), bytes));
            continue;
        }
        unknown_entries.push((name.clone(), bytes));
    }

    if left_out_bibites > 0 || left_out_eggs > 0 {
        eprintln!(
            "bibites-io: left out {left_out_bibites} bibites and {left_out_eggs} eggs whose position, size, health or energy was not a number (written by a build before the NaN fix)"
        );
    }
    let species = species_raw.as_ref().map(parse_lineage).transpose()?;
    let _ = species_bytes;

    Ok(SaveBundle {
        settings,
        settings_file,
        scene,
        version,
        vars,
        pellets,
        pheromones,
        bibites,
        bibite_ids,
        bibite_extras,
        bibite_parents,
        bibite_children,
        eggs,
        egg_extras,
        egg_ids,
        egg_parents,
        species,
        species_raw: None,
        data_bin,
        templates,
        img_png,
        unknown_entries,
    })
}

/// Writes a save `.zip` the game can load.
///
/// Entries are written to the zip **as they are produced** (one serialized
/// entity/section at a time) instead of collecting every JSON byte string in a
/// Vec first. The bytes, order and deflate level are unchanged, so outputs stay
/// byte-identical; this only removes the all-entries memory spike before the
/// deflate stage.
///
/// The zip is written to a file beside `path`, which takes `path`'s place when it is whole: a save that overwrites another (the quick save, an overwrite
/// the player confirmed) keeps the old one when the new one cannot be finished (a full disk, a failure while it is written, a killed program).
pub fn write_save_bundle(bundle: &SaveBundle, path: impl AsRef<Path>) -> Result<()> {
    crate::write_replacing(path.as_ref(), |beside| write_save_bundle_in_place(bundle, beside))
}

/// [`write_save_bundle`] straight into `path`.
fn write_save_bundle_in_place(bundle: &SaveBundle, path: impl AsRef<Path>) -> Result<()> {
    let file = std::fs::File::create(path.as_ref()).map_err(Error::from)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());

    let put = |zip: &mut ZipWriter<std::fs::File>, name: &str, bytes: &[u8]| -> Result<()> {
        zip.start_file(normalize_entry_name(name), options)
            .map_err(|error| Error::invalid_data(format!("zip write error: {error}")))?;
        zip.write_all(bytes)
            .map_err(|error| Error::Io(error.to_string()))
    };

    let settings_json = serde_json::from_str(&bundle.settings_file.to_json(SettingsMode::Save)?)
        .map_err(|error: serde_json::Error| Error::invalid_data(error.to_string()))?;
    put(
        &mut zip,
        "settings.bb8settings",
        &to_json_bytes(&settings_json, true)?,
    )?;
    if let Some(species) = &bundle.species {
        put(
            &mut zip,
            "speciesData.json",
            &to_json_bytes(&lineage_to_json(species), true)?,
        )?;
    }
    put(
        &mut zip,
        "scene.bb8scene",
        &to_json_bytes(&bundle.scene, true)?,
    )?;
    if let Some(vars) = &bundle.vars {
        put(&mut zip, "vars.bb8scene", &to_json_bytes(vars, true)?)?;
    }
    if let Some(pellets) = &bundle.pellets {
        put(&mut zip, "pellets.bb8scene", &to_json_bytes(pellets, true)?)?;
    }
    if let Some(pheromones) = &bundle.pheromones {
        put(
            &mut zip,
            "pheromones.bb8scene",
            &to_json_bytes(pheromones, true)?,
        )?;
    }
    if let Some(data) = &bundle.data_bin {
        put(&mut zip, "data.bin", data)?;
    }
    if let Some(img) = &bundle.img_png {
        put(&mut zip, "img.png", img)?;
    }
    // Hybrid bounded batches: serialize `SAVE_BATCH` entities on a small thread
    // pool, write them in order, then drop the batch. Peak memory stays bounded
    // (one batch, not the whole world) while JSON serialization uses all cores.
    const SAVE_BATCH: usize = 256;
    for batch_start in (0..bundle.bibites.len()).step_by(SAVE_BATCH) {
        let batch_end = (batch_start + SAVE_BATCH).min(bundle.bibites.len());
        let batch = parallel_entity_batch(batch_start, batch_end, |index| {
            let extra = bundle.bibite_extras.get(index).cloned().unwrap_or_default();
            let parents = bundle
                .bibite_parents
                .get(index)
                .copied()
                .unwrap_or_default();
            let children = bundle
                .bibite_children
                .get(index)
                .cloned()
                .unwrap_or_default();
            let body_id = bundle
                .bibite_ids
                .get(index)
                .copied()
                .unwrap_or(index as i32);
            let json = bibite_to_json(&bundle.bibites[index], &extra, parents, &children, body_id);
            Ok((
                format!("bibites/bibite_{index}.bb8"),
                to_json_bytes(&json, true)?,
            ))
        })?;
        for (name, bytes) in batch {
            put(&mut zip, &name, &bytes)?;
        }
    }
    for batch_start in (0..bundle.eggs.len()).step_by(SAVE_BATCH) {
        let batch_end = (batch_start + SAVE_BATCH).min(bundle.eggs.len());
        let batch = parallel_entity_batch(batch_start, batch_end, |index| {
            let extra = bundle.egg_extras.get(index).cloned().unwrap_or_default();
            let id = bundle.egg_ids.get(index).copied().unwrap_or(0);
            let parents = bundle.egg_parents.get(index).copied().unwrap_or_default();
            let json = egg_to_json(&bundle.eggs[index], &extra, parents, id);
            Ok((format!("eggs/egg_{index}.bb8"), to_json_bytes(&json, true)?))
        })?;
        for (name, bytes) in batch {
            put(&mut zip, &name, &bytes)?;
        }
    }
    for (name, bytes) in &bundle.templates {
        put(&mut zip, name, bytes)?;
    }
    for (name, bytes) in &bundle.unknown_entries {
        put(&mut zip, name, bytes)?;
    }
    zip.finish()
        .map_err(|error| Error::invalid_data(format!("zip finish error: {error}")))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Entity conversion
// ---------------------------------------------------------------------------

type ParsedBibite = (BibiteSave, Extra, (Option<i32>, Option<i32>), Vec<i32>, i32);

/// Whether a bibite or egg file starts with a position written as `null` (`"transform":{"position":[null,null]`: a NaN a build before the NaN
/// fix wrote). Only looks at the first bytes of the file; a file that spells its JSON with spaces is parsed as usual.
fn has_a_null_position(bytes: &[u8]) -> bool {
    const NEEDLE: &[u8] = b"\"position\":[null";
    let head = &bytes[..bytes.len().min(256)];
    head.windows(NEEDLE.len()).any(|window| window == NEEDLE)
}

/// Whether the numbers a world needs of a loaded bibite are numbers (see the loader's loop).
fn bibite_numbers_are_usable(save: &BibiteSave) -> bool {
    save.pos.x.is_finite()
        && save.pos.y.is_finite()
        && save.d2_size.is_finite()
        && save.d2_size > 0.0
        && save.health.is_finite()
        && save.energy.is_finite()
}

fn parse_bibite(value: &Value, version_text: &str, settings: &SimSettings) -> Result<ParsedBibite> {
    let value = with_version(value, version_text)?;
    let file: Bb8File = serde_json::from_value(value.clone())
        .map_err(|error| Error::invalid_data(format!("invalid bibite JSON: {error}")))?;
    let loaded = crate::loaded_from_bb8(
        &file,
        crate::SourceShape::LivingBb8,
        true,
        settings.brain_period(),
    )?;
    let body = file.body.clone().unwrap_or_default();
    let clock = file.clock.clone().unwrap_or_default();
    let mut extra = file.extra.clone();

    // Mouth/stomach/phero/eggLayer/control are preserved in `body.extra`.
    let body_extra = body.extra.clone();
    let stomach = body_extra
        .get("stomach")
        .and_then(|stomach| stomach.get("content"))
        .and_then(Value::as_array)
        .map(|content| {
            let mut slots = [StomachContentSave {
                material: 0,
                amount: 0.0,
                average_chunk_amount: 0.0,
            }; STOMACH_SLOTS_IO];
            for (index, item) in content.iter().take(STOMACH_SLOTS_IO).enumerate() {
                slots[index] = StomachContentSave {
                    material: match item.get("material").and_then(Value::as_str) {
                        Some("Meat") => 1,
                        _ => 0,
                    },
                    amount: item.get("amount").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                    average_chunk_amount: item
                        .get("averageChunkAmount")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0) as f32,
                };
            }
            slots
        })
        .unwrap_or(
            [StomachContentSave {
                material: 0,
                amount: 0.0,
                average_chunk_amount: 0.0,
            }; STOMACH_SLOTS_IO],
        );

    let n_stomach = body_extra
        .get("stomach")
        .and_then(|stomach| stomach.get("content"))
        .and_then(Value::as_array)
        .map(|content| content.len().min(STOMACH_SLOTS_IO) as u8)
        .unwrap_or(0);

    let body_value = Value::Object(body_extra.clone());
    let get = |path: &[&str]| -> f32 { nested_f32(&body_value, path) };

    let children = body_extra
        .get("eggLayer")
        .and_then(|layer| layer.get("children"))
        .and_then(Value::as_array)
        .map(|children| {
            children
                .iter()
                .filter_map(Value::as_i64)
                .map(|id| id as i32)
                .collect()
        })
        .unwrap_or_default();

    let growth = {
        let size_ratio = loaded.genome.gene(Gene::SizeRatio);
        if size_ratio > 0.0 {
            // `BibiteBody.LoadState`: `growth.growth = d2Size / (SizeRatio * SizeRatio)`: the square and the quotient are one double expression, rounded once.
            (f64::from(body.d2_size.unwrap_or(0.0)) / (f64::from(size_ratio) * f64::from(size_ratio))) as f32
        } else {
            0.0
        }
    };
    // ody.id is a typed field (Bb8Body.id), not an unknown extra; the game
    // writes signed runtime ids (e.g. -124909645), so read the typed value.
    let body_id = body.id.unwrap_or(0);
    let _ = settings;
    let save = BibiteSave {
        pos: Vec2::new(
            file.transform
                .as_ref()
                .and_then(|transform| transform.position.map(|position| position[0]))
                .or_else(|| file.rb2d.as_ref().and_then(|body| body.px))
                .unwrap_or(0.0),
            file.transform
                .as_ref()
                .and_then(|transform| transform.position.map(|position| position[1]))
                .or_else(|| file.rb2d.as_ref().and_then(|body| body.py))
                .unwrap_or(0.0),
        ),
        vel: Vec2::new(
            file.rb2d.as_ref().and_then(|body| body.vx).unwrap_or(0.0),
            file.rb2d.as_ref().and_then(|body| body.vy).unwrap_or(0.0),
        ),
        // Pass-through: BibiteSave.rot is the game transform rotation in
        // DEGREES (Unity 	ransform.rotation.z). Core spawn_loaded converts
        // deg→rad; io must not convert here.
        rot: file
            .transform
            .as_ref()
            .and_then(|transform| transform.rotation)
            .or_else(|| file.rb2d.as_ref().and_then(|body| body.r))
            .unwrap_or(0.0),
        ang_vel: 0.0,
        genome: loaded.genome,
        meta: loaded.meta,
        brain: loaded.brain,
        energy: body.energy.unwrap_or(0.0),
        health: body.health.unwrap_or(0.0),
        d2_size: body.d2_size.unwrap_or(0.0),
        dying: body.dying.unwrap_or(false),
        dead: body.dead.unwrap_or(false),
        attacked_dmg: body_extra
            .get("attackedDmg")
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as f32,
        times_attacked: body_extra
            .get("timesAttacked")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        total_damage_suffered: get(&["totalDamageSuffered"]),
        brain_ticks: get(&["brainTicksCount"]),
        vision_lookup: get(&["visionLookupCount"]),
        vision_sensing: get(&["visionSensingCount"]),
        corpse_energy_offset: get(&["corpseEnergyOffset"]),
        growth,
        stomach,
        n_stomach,
        bite_progress: get(&["mouth", "biteProgress"]),
        attacked_last_frame: body_extra
            .get("mouth")
            .and_then(|mouth| mouth.get("attackedLastFrame"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        bibites_bitten: get(&["mouth", "bibitesBitten"]) as u32,
        total_damage_dealt: get(&["mouth", "totalDamageDealt"]),
        total_murders: get(&["mouth", "totalMurders"]) as u32,
        murdered_area: get(&["mouth", "murderedArea"]),
        egg_progress: get(&["eggLayer", "eggProgress"]),
        n_eggs_laid: get(&["eggLayer", "nEggsLaid"]) as u32,
        children: Vec::new(),
        fat_amount: body_extra
            .get("fatReservesAmount")
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as f32,
        phero_progress: get(&["phero", "progress"]),
        tic: clock.tic.unwrap_or(0) as u8,
        tic_progress: clock.tic_progress.unwrap_or(0.0),
        time_alive: clock.time_alive.unwrap_or(0.0),
        chrono_time: clock.chrono_time.unwrap_or(0.0),
        total_travel: get(&["control", "totalTravel"]),
    };
    // Unknown top-level fields are kept together with the entity.
    for (key, value) in &file.extra {
        extra.insert(key.clone(), value.clone());
    }
    Ok((
        save,
        extra,
        (file.genes.parent1, file.genes.parent2),
        children,
        body_id,
    ))
}

fn parse_egg(
    value: &Value,
    version_text: &str,
    settings: &SimSettings,
) -> Result<(EggSave, Extra, (Option<i32>, Option<i32>), i32)> {
    let value = with_version(value, version_text)?;
    let file: Bb8File = serde_json::from_value(value.clone())
        .map_err(|error| Error::invalid_data(format!("invalid egg JSON: {error}")))?;
    let loaded = crate::loaded_from_bb8(
        &file,
        crate::SourceShape::LivingBb8,
        true,
        settings.brain_period(),
    )?;
    let egg = file.egg.clone().unwrap_or_default();
    // Only `hatchProgress`, `energy` and `id` are serialized by the game; the
    // remaining EggSave fields are derived exactly like `EggStore::spawn`.
    let size_ratio = loaded.genome.gene(Gene::SizeRatio);
    let brain_upkeep = loaded.brain.upkeep().energy_per_second(
        settings.scenario.synapse_upkeep_cost,
        settings.scenario.neuron_upkeep_cost,
    );
    let total_required_energy = egg_required_energy(
        loaded.genome.body_energy_at_birth(settings),
        brain_upkeep,
        size_ratio,
    );
    let hatch_time = loaded.genome.gene(Gene::HatchTime);
    let size_gene = loaded.genome.d1_size_at_birth();
    let energy = egg.energy.unwrap_or(0.0);
    let save = EggSave {
        pos: Vec2::new(
            file.transform
                .as_ref()
                .and_then(|transform| transform.position.map(|position| position[0]))
                .unwrap_or(0.0),
            file.transform
                .as_ref()
                .and_then(|transform| transform.position.map(|position| position[1]))
                .unwrap_or(0.0),
        ),
        vel: Vec2::new(0.0, 0.0),
        // Pass-through degrees (game transform rotation); core converts rad on load.
        rot: file
            .transform
            .as_ref()
            .and_then(|transform| transform.rotation)
            .unwrap_or(0.0),
        ang_vel: 0.0,
        genome: loaded.genome,
        meta: loaded.meta,
        brain: loaded.brain,
        energy,
        hatch_progress: egg.hatch_progress.unwrap_or(0.0),
        hatch_time,
        total_required_energy,
        is_hatching: egg.hatch_progress.unwrap_or(0.0) > 0.0,
        aborting: false,
        birth_orientation: Vec2::new(0.0, -1.0),
        size_gene,
    };
    Ok((
        save,
        file.extra,
        (loaded.parent1, loaded.parent2),
        egg.id.unwrap_or(0),
    ))
}

fn with_version(value: &Value, version_text: &str) -> Result<Value> {
    let mut object = value
        .as_object()
        .cloned()
        .ok_or_else(|| Error::invalid_data("entity is not an object"))?;
    object
        .entry("version".to_string())
        .or_insert_with(|| Value::from(version_text));
    Ok(Value::Object(object))
}

pub(crate) fn bibite_to_json(
    save: &BibiteSave,
    extra: &Extra,
    parents: (Option<i32>, Option<i32>),
    children: &[i32],
    body_id: i32,
) -> Value {
    let mut genes = Map::new();
    genes.insert("isReady".to_string(), Value::Bool(true));
    genes.insert(
        "gen".to_string(),
        Value::from(i64::from(save.meta.generation)),
    );
    if let Some(tag) = &save.meta.species_tag {
        genes.insert("tag".to_string(), Value::from(tag.clone()));
    }
    if let Some(species) = save.meta.species {
        genes.insert(
            "speciesID".to_string(),
            Value::from(i64::from(species.value())),
        );
    }
    if let Some(parent) = parents.0 {
        genes.insert("parent1".to_string(), Value::from(i64::from(parent)));
    }
    if let Some(parent) = parents.1 {
        genes.insert("parent2".to_string(), Value::from(i64::from(parent)));
    }
    if save.meta.is_mutant {
        genes.insert("isMutant".to_string(), Value::Bool(true));
    }
    if save.meta.is_rad {
        genes.insert("isRad".to_string(), Value::Bool(true));
    }
    if save.meta.is_rain {
        genes.insert("isRain".to_string(), Value::Bool(true));
    }
    if save.meta.is_killer {
        genes.insert("isKiller".to_string(), Value::Bool(true));
    }
    if save.meta.is_source {
        genes.insert("isSource".to_string(), Value::Bool(true));
    }
    genes.insert("genes".to_string(), genes_to_json(&save.genome));

    let mut stomach_content = Vec::new();
    for slot in save.stomach.iter().take(save.n_stomach as usize) {
        stomach_content.push(serde_json::json!({
            "material": if slot.material == 1 { "Meat" } else { "Plant" },
            "amount": f64::from(slot.amount),
            "averageChunkAmount": f64::from(slot.average_chunk_amount),
        }));
    }
    let body = serde_json::json!({
        "mouth": {
            "attackedLastFrame": save.attacked_last_frame,
            "biteProgress": f64::from(save.bite_progress),
            "bibitesBitten": i64::from(save.bibites_bitten),
            "totalDamageDealt": f64::from(save.total_damage_dealt),
            "totalMurders": i64::from(save.total_murders),
            "murderedArea": f64::from(save.murdered_area),
        },
        "stomach": {"content": stomach_content},
        "phero": {"progress": f64::from(save.phero_progress)},
        "health": f64::from(save.health),
        "energy": f64::from(save.energy),
        "eggLayer": {"eggProgress": f64::from(save.egg_progress), "nEggsLaid": i64::from(save.n_eggs_laid), "children": children},
        "control": {"totalTravel": f64::from(save.total_travel)},
        "fatReservesAmount": f64::from(save.fat_amount),
        "id": body_id,
        "d2Size": f64::from(save.d2_size),
        "dying": save.dying,
        "dead": save.dead,
        "attackedDmg": f64::from(save.attacked_dmg),
        "timesAttacked": i64::from(save.times_attacked),
        "totalDamageSuffered": f64::from(save.total_damage_suffered),
        "brainTicksCount": f64::from(save.brain_ticks),
        "visionLookupCount": f64::from(save.vision_lookup),
        "visionSensingCount": f64::from(save.vision_sensing),
        "corpseEnergyOffset": f64::from(save.corpse_energy_offset),
    });
    let mut root = Map::new();
    // save.rot is already in degrees (the game's transform.rotation.z shape).
    root.insert(
        "transform".to_string(),
        serde_json::json!({"position": [f64::from(save.pos.x), f64::from(save.pos.y)], "rotation": f64::from(save.rot), "scale": 1.0}),
    );
    root.insert(
        "rb2d".to_string(),
        serde_json::json!({"px": f64::from(save.pos.x), "py": f64::from(save.pos.y), "vx": f64::from(save.vel.x), "vy": f64::from(save.vel.y), "r": f64::from(save.rot)}),
    );
    root.insert("genes".to_string(), Value::Object(genes));
    root.insert("body".to_string(), body);
    root.insert(
        "clock".to_string(),
        serde_json::json!({"tic": i64::from(save.tic), "ticProgress": f64::from(save.tic_progress), "timeAlive": f64::from(save.time_alive), "chronoTime": f64::from(save.chrono_time)}),
    );
    root.insert(
        "brain".to_string(),
        brain_to_json(&save.brain, &save.genome),
    );
    for (key, value) in extra {
        root.entry(key.clone()).or_insert_with(|| value.clone());
    }
    Value::Object(root)
}

pub(crate) fn egg_to_json(
    save: &EggSave,
    extra: &Extra,
    parents: (Option<i32>, Option<i32>),
    id: i32,
) -> Value {
    let mut genes = Map::new();
    genes.insert("isReady".to_string(), Value::Bool(true));
    genes.insert(
        "gen".to_string(),
        Value::from(i64::from(save.meta.generation)),
    );
    if let Some(tag) = &save.meta.species_tag {
        genes.insert("tag".to_string(), Value::from(tag.clone()));
    }
    if let Some(species) = save.meta.species {
        genes.insert(
            "speciesID".to_string(),
            Value::from(i64::from(species.value())),
        );
    }
    if let Some(parent) = parents.0 {
        genes.insert("parent1".to_string(), Value::from(i64::from(parent)));
    }
    if let Some(parent) = parents.1 {
        genes.insert("parent2".to_string(), Value::from(i64::from(parent)));
    }
    genes.insert("genes".to_string(), genes_to_json(&save.genome));
    let mut root = Map::new();
    // save.rot is already in degrees (the game's transform.rotation.z shape).
    root.insert(
        "transform".to_string(),
        serde_json::json!({"position": [f64::from(save.pos.x), f64::from(save.pos.y)], "rotation": f64::from(save.rot), "scale": 1.0}),
    );
    root.insert("genes".to_string(), Value::Object(genes));
    root.insert(
        "egg".to_string(),
        serde_json::json!({"hatchProgress": f64::from(save.hatch_progress), "energy": f64::from(save.energy), "id": id}),
    );
    root.insert(
        "brain".to_string(),
        brain_to_json(&save.brain, &save.genome),
    );
    for (key, value) in extra {
        root.entry(key.clone()).or_insert_with(|| value.clone());
    }
    Value::Object(root)
}

fn brain_to_json(brain: &Brain, genome: &Genome) -> Value {
    let nodes = brain
        .to_node_records()
        .into_iter()
        .enumerate()
        .map(|(index, record)| NodeJson {
            node_type: Some(record.node_type as u8),
            index: Some(index as u32),
            innovation: Some(record.innovation),
            desc: brain.node_desc(index).map(str::to_string),
            base_activation: Some(record.base_activation),
            type_name: Some(record.node_type.name().to_string()),
            value: Some(record.value),
            last_input: Some(record.last_input),
            last_output: Some(record.last_output),
            archetype: Some(if index < 33 {
                0
            } else if index < 48 {
                1
            } else {
                2
            }),
            extra: Extra::new(),
        })
        .collect::<Vec<_>>();
    let synapses = brain
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
        .collect::<Vec<_>>();
    let _ = genome;
    let mut root = Map::new();
    root.insert("isReady".to_string(), Value::Bool(true));
    root.insert(
        "Nodes".to_string(),
        serde_json::to_value(nodes).unwrap_or(Value::Null),
    );
    root.insert(
        "Synapses".to_string(),
        serde_json::to_value(synapses).unwrap_or(Value::Null),
    );
    Value::Object(root)
}

fn genes_to_json(genome: &Genome) -> Value {
    let mut map = Map::new();
    for gene in bibites_core::genome::ALL_GENES {
        map.insert(
            gene.def().name.to_string(),
            Value::from(f64::from(genome.genes[gene.index()])),
        );
    }
    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Species lineage
// ---------------------------------------------------------------------------

fn parse_lineage(value: &Value) -> Result<LineageRecordIo> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::invalid_data("speciesData.json is not an object"))?;
    let mut recorded = Vec::new();
    for entry in object
        .get("recordedSpecies")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        recorded.push(parse_species(entry)?);
    }
    Ok(LineageRecordIo {
        recorded_species: recorded,
        active_species: object
            .get("activeSpeciesList")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_u64)
                    .map(|id| id as u32)
                    .collect()
            })
            .unwrap_or_default(),
        node_max_innovation: object
            .get("nodeMaxInnovation")
            .and_then(Value::as_i64)
            .unwrap_or(49),
        connection_max_innovation: object
            .get("connectionMaxInnovation")
            .and_then(Value::as_i64)
            .unwrap_or(1),
        next_species_id: object
            .get("nextSpeciesID")
            .and_then(Value::as_u64)
            .unwrap_or(1) as u32,
        max_energy: object
            .get("maxEnergy")
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as f32,
        max_count: object
            .get("maxCount")
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as f32,
        log_counter: object
            .get("logCounter")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        n_records: object.get("nRecords").and_then(Value::as_u64).unwrap_or(0),
    })
}

fn parse_species(value: &Value) -> Result<SpeciesRecordIo> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::invalid_data("species entry is not an object"))?;
    let template_value = object
        .get("template")
        .ok_or_else(|| Error::invalid_data("species has no template"))?;
    let template_object = template_value
        .as_object()
        .ok_or_else(|| Error::invalid_data("species template is not an object"))?;
    let nodes: Vec<NodeJson> = serde_json::from_value(
        template_object
            .get("nodes")
            .cloned()
            .unwrap_or_else(|| Value::Array(vec![])),
    )
    .map_err(|error| Error::invalid_data(format!("species nodes: {error}")))?;
    let synapses: Vec<SynapseJson> = serde_json::from_value(
        template_object
            .get("synapses")
            .cloned()
            .unwrap_or_else(|| Value::Array(vec![])),
    )
    .map_err(|error| Error::invalid_data(format!("species synapses: {error}")))?;
    let nodes = template_records_from_json(nodes)?;
    let synapses = synapse_records_from_json(synapses)?;
    let genes = match template_object.get("genes") {
        Some(genes) => genes_from_template_repr(genes)?,
        None => [0.0; 35],
    };
    Ok(SpeciesRecordIo {
        species_id: object.get("speciesID").and_then(Value::as_u64).unwrap_or(0) as u32,
        time_creation: object
            .get("timeCreation")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        generation_of_first_specimen: object
            .get("generationOfFirstSpecimen")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        favorite: object
            .get("favorite")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        generic_name: object
            .get("genericName")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        specific_name: object
            .get("specificName")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        description: object
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        parent_id: object
            .get("parentID")
            .and_then(Value::as_u64)
            .map(|id| id as u32),
        template: SpeciesTemplateRecord {
            version: template_object
                .get("version")
                .and_then(Value::as_str)
                .map(str::to_string),
            nodes,
            synapses,
            genes,
        },
    })
}

fn lineage_to_json(lineage: &LineageRecordIo) -> Value {
    let mut object = Map::new();
    object.insert(
        "recordedSpecies".to_string(),
        Value::Array(
            lineage
                .recorded_species
                .iter()
                .map(species_to_json)
                .collect(),
        ),
    );
    object.insert(
        "activeSpeciesList".to_string(),
        Value::Array(
            lineage
                .active_species
                .iter()
                .map(|id| Value::from(i64::from(*id)))
                .collect(),
        ),
    );
    object.insert(
        "nodeMaxInnovation".to_string(),
        Value::from(lineage.node_max_innovation),
    );
    object.insert(
        "connectionMaxInnovation".to_string(),
        Value::from(lineage.connection_max_innovation),
    );
    object.insert(
        "nextSpeciesID".to_string(),
        Value::from(i64::from(lineage.next_species_id)),
    );
    object.insert(
        "maxEnergy".to_string(),
        Value::from(f64::from(lineage.max_energy)),
    );
    object.insert(
        "maxCount".to_string(),
        Value::from(f64::from(lineage.max_count)),
    );
    object.insert("logCounter".to_string(), Value::from(lineage.log_counter));
    object.insert("nRecords".to_string(), Value::from(lineage.n_records));
    object.insert("usedSpanText".to_string(), Value::Object(Map::new()));
    Value::Object(object)
}

fn species_to_json(species: &SpeciesRecordIo) -> Value {
    let mut object = Map::new();
    object.insert(
        "speciesID".to_string(),
        Value::from(i64::from(species.species_id)),
    );
    object.insert(
        "timeCreation".to_string(),
        Value::from(species.time_creation),
    );
    object.insert(
        "generationOfFirstSpecimen".to_string(),
        Value::from(i64::from(species.generation_of_first_specimen)),
    );
    object.insert("favorite".to_string(), Value::Bool(species.favorite));
    object.insert(
        "genericName".to_string(),
        Value::from(species.generic_name.clone()),
    );
    object.insert(
        "specificName".to_string(),
        Value::from(species.specific_name.clone()),
    );
    object.insert(
        "description".to_string(),
        Value::from(species.description.clone()),
    );
    if let Some(parent) = species.parent_id {
        object.insert("parentID".to_string(), Value::from(i64::from(parent)));
    }
    let mut template = Map::new();
    if let Some(version) = &species.template.version {
        template.insert("version".to_string(), Value::from(version.clone()));
    }
    template.insert(
        "nodes".to_string(),
        serde_json::to_value(nodes_json_for_species(&species.template.nodes))
            .unwrap_or(Value::Null),
    );
    template.insert(
        "synapses".to_string(),
        serde_json::to_value(synapses_json_for_species(&species.template.synapses))
            .unwrap_or(Value::Null),
    );
    template.insert(
        "genes".to_string(),
        genes_to_json(&Genome {
            genes: species.template.genes,
        }),
    );
    object.insert("template".to_string(), Value::Object(template));
    Value::Object(object)
}

fn nodes_json_for_species(nodes: &[bibites_core::brain::NodeRecord]) -> Vec<NodeJson> {
    nodes
        .iter()
        .map(|node| NodeJson {
            node_type: Some(node.node_type as u8),
            index: Some(node.index),
            innovation: Some(node.innovation),
            desc: node.desc.clone(),
            base_activation: Some(node.base_activation),
            type_name: None,
            value: None,
            last_input: None,
            last_output: None,
            archetype: None,
            extra: Extra::new(),
        })
        .collect()
}

fn synapses_json_for_species(synapses: &[bibites_core::brain::SynapseRecord]) -> Vec<SynapseJson> {
    synapses
        .iter()
        .map(|synapse| SynapseJson {
            innovation: synapse.innovation,
            node_in: synapse.node_in,
            node_out: synapse.node_out,
            weight: synapse.weight,
            enabled: synapse.enabled,
            extra: Extra::new(),
        })
        .collect()
}

/// Converts a template node list coming from a species record.
pub(crate) fn template_records_from_json(
    nodes: Vec<NodeJson>,
) -> Result<Vec<bibites_core::brain::NodeRecord>> {
    let mut records = Vec::with_capacity(nodes.len());
    for (position, node) in nodes.into_iter().enumerate() {
        let raw_type = node
            .node_type
            .ok_or_else(|| Error::invalid_data(format!("species node {position} has no Type")))?;
        let node_type = NodeType::from_u8(raw_type)
            .ok_or_else(|| Error::invalid_data("species node has unknown Type"))?;
        records.push(bibites_core::brain::NodeRecord {
            node_type,
            index: position as u32,
            innovation: node.innovation.unwrap_or(position as i64 + 1),
            desc: node.desc,
            base_activation: node.base_activation.unwrap_or(0.0),
            value: 0.0,
            last_input: 0.0,
            last_output: 0.0,
        });
    }
    Ok(records)
}

/// Converts a template synapse list coming from a species record.
pub(crate) fn synapse_records_from_json(
    synapses: Vec<SynapseJson>,
) -> Result<Vec<bibites_core::brain::SynapseRecord>> {
    Ok(synapses
        .into_iter()
        .map(|synapse| bibites_core::brain::SynapseRecord {
            innovation: synapse.innovation,
            node_in: synapse.node_in,
            node_out: synapse.node_out,
            weight: synapse.weight,
            enabled: synapse.enabled,
        })
        .collect())
}

/// Parses a flat species-template gene object/array into 35 values.
pub(crate) fn genes_from_template_repr(value: &Value) -> Result<[f32; 35]> {
    use bibites_core::genome::{ALL_GENES, Gene, N_GENES};
    let mut genes = [0.0_f32; N_GENES];
    match &value {
        Value::Object(map) => {
            for gene in ALL_GENES {
                genes[gene.index()] = map
                    .get(gene.def().name)
                    .and_then(Value::as_f64)
                    .map_or(gene.def().default, |value| value as f32);
            }
        }
        Value::Array(values) => {
            if values.len() != N_GENES {
                return Err(Error::unsupported(format!(
                    "species template gene array has {} values (expected 35)",
                    values.len()
                )));
            }
            for (index, value) in values.iter().enumerate() {
                genes[index] = value.as_f64().unwrap_or(0.0) as f32;
            }
        }
        _ => return Err(Error::invalid_data("species template genes are malformed")),
    }
    let _ = Gene::LayTime;
    Ok(genes)
}

/// Serializes one bounded batch of entities on up to 8 threads; results are
/// returned in ascending index order so zip entries stay deterministic.
fn parallel_entity_batch<F>(
    start: usize,
    end: usize,
    serialize: F,
) -> Result<Vec<(String, Vec<u8>)>>
where
    F: Fn(usize) -> Result<(String, Vec<u8>)> + Sync,
{
    let total = end - start;
    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(8)
        .min(total);
    if workers <= 1 {
        return (start..end).map(&serialize).collect();
    }
    let chunk = total.div_ceil(workers);
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers);
        for worker in 0..workers {
            let chunk_start = start + worker * chunk;
            if chunk_start >= end {
                break;
            }
            let chunk_end = (chunk_start + chunk).min(end);
            let serialize = &serialize;
            handles.push(scope.spawn(move || -> Result<Vec<(String, Vec<u8>)>> {
                (chunk_start..chunk_end).map(serialize).collect()
            }));
        }
        let mut entries = Vec::with_capacity(total);
        for handle in handles {
            entries.extend(
                handle
                    .join()
                    .map_err(|_| Error::invalid_data("entity JSON worker panicked"))??,
            );
        }
        Ok(entries)
    })
}

fn nested_f32(value: &Value, path: &[&str]) -> f32 {
    let mut current = Some(value);
    for key in path {
        current = current.and_then(|value| value.get(*key));
    }
    current.and_then(Value::as_f64).unwrap_or(0.0) as f32
}

// Suppress unused-import warnings for helpers that exist for later slices.
#[allow(dead_code)]
fn _unused(
    _: MaterialKind,
    _: TemplateMeta,
    _: AnchorJson,
    _: Bb8File,
    _: GenesRepr,
    _: BTreeMap<String, Value>,
) {
}

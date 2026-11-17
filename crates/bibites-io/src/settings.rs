//! `.bb8settings` files: the three setting scopes, scenario/zone/spawn objects,
//! settings animators, challenge trees, effective-settings precedence and the
//! parity bridge `/settings` JSON.
//!
//! Spec: `docs/spec/04-formats-settings.md` sections 2, 3, 6, 7 and 9.2.
//!
//! ## Two file shapes
//!
//! * **Save settings**: 78 scenario scalars at the top level, `materials`,
//!   `zones`, `bibites`, `settingsChangers`, `zoneGroups`, and an
//!   `independents` object with the 14 scenario-independent settings.
//! * **Scenario settings**: same but with `recommended` instead of
//!   `independents` (and optional `challenge`).
//!
//! Each scalar setting is wrapped by Unity's `JsonUtility`:
//! `{"interactableChanged":{"m_PersistentCalls":{"m_Calls":[]}},"Value":x}`.
//! Readers are key-based; writers emit the game's canonical order and preserve
//! unknown top-level keys, unknown keys inside scalar wrappers, and unknown
//! material/zone keys via `extra` maps.
//!
//! ## Precedence (Lead decision 2026-10-29)
//!
//! Loading a save: save `independents` > CLI override > defaults. Starting a new
//! world from a scenario: apply its `recommended` block (missing keys reset to
//! their defaults) unless a CLI override is supplied (reproducibility wins).
//! [`resolve_effective_settings`] implements this.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use bibites_core::settings::{
    BibiteSpawn, ChallengeParameters, Comparator, ConditionGroup, ConditionGroupLogic,
    ConditionNode, GrowthAtSpawn, IndependentSettings, MaterialKind, MaterialSetting,
    MaterialSettings, MovementType, RandomizeGenes, ScenarioSettings, SettingKind, SettingValue,
    SettingsChanger, SimCondition, SimMetric, SimSettings, SpawnDistribution, SpawnType,
    SplinePoint, Tagging, ZoneGroupSettings, ZoneSettings,
};
use bibites_core::{Error, Result};

use crate::model::Extra;

/// Which top-level settings-file shape to write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsMode {
    /// Save shape: includes `independents`.
    Save,
    /// Scenario shape: includes `recommended`.
    Scenario,
}

/// A parsed `.bb8settings` file (save or scenario shape) plus preserved extras.
#[derive(Clone, Debug, PartialEq)]
pub struct ScenarioSettingsFile {
    /// The 78 scenario settings.
    pub scenario: ScenarioSettings,
    /// 14 independent settings from a save's `independents` object.
    pub independents: Option<IndependentSettings>,
    /// 14 independent settings from a scenario's `recommended` object.
    pub recommended: Option<IndependentSettings>,
    /// The four material parameter sets.
    pub materials: MaterialSettings,
    /// Zone list.
    pub zones: Vec<ZoneSettings>,
    /// Zone groups.
    pub zone_groups: Vec<ZoneGroupSettings>,
    /// Spawn rows.
    pub spawns: Vec<BibiteSpawn>,
    /// Settings animators.
    pub changers: Vec<SettingsChanger>,
    /// Challenge definition, when present.
    pub challenge: Option<ChallengeParameters>,
    /// Unknown top-level keys, preserved verbatim.
    pub extra: Extra,
    /// Unknown keys inside each scalar setting wrapper, keyed by setting key.
    pub scalar_extras: BTreeMap<String, Extra>,
    /// Unknown keys inside the four material objects.
    pub material_extras: [Extra; 4],
    /// Unknown keys inside each zone object.
    pub zone_extras: Vec<Extra>,
    /// Setting keys whose imported numeric value was clamped to its range.
    pub clamped_fields: Vec<String>,
}

impl ScenarioSettingsFile {
    /// Effective settings for a run: `independents` if present (save), else
    /// `recommended` (new world from scenario), else the code defaults.
    #[must_use]
    pub fn effective_settings(&self) -> SimSettings {
        let independents = self
            .independents
            .clone()
            .or_else(|| self.recommended.clone())
            .unwrap_or_default();
        SimSettings::from_parts(self.scenario.clone(), independents, self.materials)
    }

    /// Serializes this file back to JSON, preserving extras.
    pub fn to_json(&self, mode: SettingsMode) -> Result<String> {
        let mut root = Map::new();
        write_scalar_scope(&mut root, &self.scenario, &self.scalar_extras);
        root.insert(
            "materials".to_string(),
            materials_to_json(&self.materials, &self.material_extras),
        );
        let mut zones = Vec::with_capacity(self.zones.len());
        for (index, zone) in self.zones.iter().enumerate() {
            let mut object = zone_to_json(zone, Some(index as u32));
            if let Some(extra) = self.zone_extras.get(index) {
                merge_extra(&mut object, extra);
            }
            zones.push(Value::Object(object));
        }
        root.insert("zones".to_string(), Value::Array(zones));
        let mut spawns = Vec::with_capacity(self.spawns.len());
        for spawn in &self.spawns {
            spawns.push(Value::Object(spawn_to_json(spawn)));
        }
        root.insert("bibites".to_string(), Value::Array(spawns));
        let mut changers = Vec::with_capacity(self.changers.len());
        for changer in &self.changers {
            changers.push(Value::Object(changer_to_json(changer)));
        }
        root.insert("settingsChangers".to_string(), Value::Array(changers));
        if let Some(challenge) = &self.challenge {
            root.insert(
                "challenge".to_string(),
                Value::Object(challenge_to_json(challenge)),
            );
        }
        let mut groups = Vec::with_capacity(self.zone_groups.len());
        for group in &self.zone_groups {
            groups.push(Value::Object(zone_group_to_json(group)));
        }
        root.insert("zoneGroups".to_string(), Value::Array(groups));
        match mode {
            SettingsMode::Save => {
                let independents = self
                    .independents
                    .clone()
                    .or_else(|| self.recommended.clone())
                    .unwrap_or_default();
                root.insert(
                    "independents".to_string(),
                    independents_to_json(&independents),
                );
            }
            SettingsMode::Scenario => {
                let recommended = self
                    .recommended
                    .clone()
                    .or_else(|| self.independents.clone())
                    .unwrap_or_default();
                root.insert(
                    "recommended".to_string(),
                    independents_to_json(&recommended),
                );
            }
        }
        for (key, value) in &self.extra {
            root.entry(key.clone()).or_insert_with(|| value.clone());
        }
        serde_json::to_string(&Value::Object(root))
            .map_err(|error| Error::invalid_data(format!("cannot serialize settings: {error}")))
    }
}

/// Parses a save- or scenario-shaped `.bb8settings` JSON document.
pub fn parse_settings_json(json: &str) -> Result<ScenarioSettingsFile> {
    let json = json.strip_prefix('\u{feff}').unwrap_or(json);
    let value: Value = serde_json::from_str(json)
        .map_err(|error| Error::invalid_data(format!("invalid .bb8settings JSON: {error}")))?;
    let root = value
        .as_object()
        .ok_or_else(|| Error::invalid_data(".bb8settings root is not an object"))?;

    let scenario_keys: BTreeSet<&'static str> = ScenarioSettings::SETTINGS
        .iter()
        .map(|meta| meta.key)
        .collect();
    let independent_keys: BTreeSet<&'static str> = IndependentSettings::SETTINGS
        .iter()
        .map(|meta| meta.key)
        .collect();
    const RESERVED: [&str; 11] = [
        "materials",
        "zones",
        "bibites",
        "settingsChangers",
        "zoneGroups",
        "independents",
        "recommended",
        "challenge",
        "interactableChanged",
        "Value",
        "challengeParameters",
    ];

    let mut file = ScenarioSettingsFile {
        scenario: ScenarioSettings::default(),
        independents: None,
        recommended: None,
        materials: MaterialSettings::default(),
        zones: Vec::new(),
        zone_groups: Vec::new(),
        spawns: Vec::new(),
        changers: Vec::new(),
        challenge: None,
        extra: Extra::new(),
        scalar_extras: BTreeMap::new(),
        material_extras: std::array::from_fn(|_| Extra::new()),
        zone_extras: Vec::new(),
        clamped_fields: Vec::new(),
    };

    for (key, value) in root {
        if scenario_keys.contains(key.as_str()) {
            parse_scalar_into(&mut file, key, value)?;
        } else if key == "materials" {
            parse_materials(&mut file, value)?;
        } else if key == "zones" {
            parse_zones(&mut file, value)?;
        } else if key == "zoneGroups" {
            parse_zone_groups(&mut file, value)?;
        } else if key == "bibites" {
            parse_spawns(&mut file, value)?;
        } else if key == "settingsChangers" {
            parse_changers(&mut file, value)?;
        } else if key == "challenge" || key == "challengeParameters" {
            file.challenge = Some(parse_challenge(value)?);
        } else if key == "independents" || key == "recommended" {
            let independents = parse_independents(value)?;
            if key == "independents" {
                file.independents = Some(independents);
            } else {
                file.recommended = Some(independents);
            }
        } else if !RESERVED.contains(&key.as_str()) && !independent_keys.contains(key.as_str()) {
            file.extra.insert(key.clone(), value.clone());
        }
    }
    // Legacy flat `.bb8settings` files keep the independent settings at the top
    // level; collect them if no nested block was present.
    if file.independents.is_none() && file.recommended.is_none() {
        let has_flat = IndependentSettings::SETTINGS
            .iter()
            .any(|meta| root.contains_key(meta.key));
        if has_flat {
            file.independents = Some(parse_independents(&Value::Object(root.clone()))?);
        }
    }
    Ok(file)
}

/// Loads a standalone `.bb8settings` file into effective settings.
pub fn load_settings_file(path: impl AsRef<std::path::Path>) -> Result<SimSettings> {
    let text = std::fs::read_to_string(path).map_err(Error::from)?;
    Ok(parse_settings_json(&text)?.effective_settings())
}

/// Parses a scenario `settings.bb8settings` JSON document.
pub fn parse_scenario_settings(json: &str) -> Result<ScenarioSettingsFile> {
    parse_settings_json(json)
}

/// Parses an `independents`/`recommended` object. Missing keys take their game
/// defaults (the game resets absent `recommended` keys to defaults).
pub fn parse_independents(value: &Value) -> Result<IndependentSettings> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::invalid_data("independents/recommended is not an object"))?;
    let mut settings = IndependentSettings::default();
    for meta in IndependentSettings::SETTINGS {
        if let Some(raw) = object.get(meta.key) {
            let (inner, _extra) = split_wrapper(raw, meta.key)?;
            let parsed = parse_setting_value(&inner, meta.kind, meta.key)?;
            if meta.clamped && is_out_of_range(parsed, meta.min, meta.max) {
                settings.set_raw(meta.key, meta.clamp_value(parsed));
            } else {
                settings.set_raw(meta.key, parsed);
            }
        }
    }
    Ok(settings)
}

/// Applies a scenario `recommended` object over the current independent values.
///
/// Missing keys reset to their game defaults (spec 4 section 3.2.1); returns the
/// number of values that had to be clamped on import.
pub fn apply_recommended(settings: &mut SimSettings, recommended: &Value) -> Result<usize> {
    let independents = parse_independents(recommended)?;
    let mut clamped = 0usize;
    for meta in IndependentSettings::SETTINGS {
        let Some(value) = independents.get_by_key(meta.key) else {
            continue;
        };
        let applied = if meta.clamped && is_out_of_range(value, meta.min, meta.max) {
            clamped += 1;
            meta.clamp_value(value)
        } else {
            value
        };
        settings.independents.set_raw(meta.key, applied);
    }
    settings.sync_rates_from_independents();
    Ok(clamped)
}

/// Resolves the effective independent settings for a run.
///
/// Precedence: `save_independents` (save load) > `cli_override` (reproducibility)
/// > `scenario_recommended` (new world from scenario) > code defaults.
#[must_use]
pub fn resolve_effective_settings(
    scenario: ScenarioSettings,
    materials: MaterialSettings,
    save_independents: Option<IndependentSettings>,
    cli_override: Option<IndependentSettings>,
    scenario_recommended: Option<IndependentSettings>,
) -> SimSettings {
    let independents = save_independents
        .or(cli_override)
        .or(scenario_recommended)
        .unwrap_or_default();
    SimSettings::from_parts(scenario, independents, materials)
}

/// Serializes the 78 scenario + 14 independent + material scopes as a save-shaped
/// `.bb8settings` document.
pub fn serialize_save_settings(settings: &SimSettings) -> Result<String> {
    ScenarioSettingsFile {
        scenario: settings.scenario.clone(),
        independents: Some(settings.independents.clone()),
        recommended: None,
        materials: settings.materials,
        zones: Vec::new(),
        zone_groups: Vec::new(),
        spawns: Vec::new(),
        changers: Vec::new(),
        challenge: None,
        extra: Extra::new(),
        scalar_extras: BTreeMap::new(),
        material_extras: std::array::from_fn(|_| Extra::new()),
        zone_extras: Vec::new(),
        clamped_fields: Vec::new(),
    }
    .to_json(SettingsMode::Save)
}

/// Serializes the three scopes as a scenario-shaped `.bb8settings` document
/// (`recommended` instead of `independents`).
pub fn serialize_scenario_settings(settings: &SimSettings) -> Result<String> {
    ScenarioSettingsFile {
        scenario: settings.scenario.clone(),
        independents: None,
        recommended: Some(settings.independents.clone()),
        materials: settings.materials,
        zones: Vec::new(),
        zone_groups: Vec::new(),
        spawns: Vec::new(),
        changers: Vec::new(),
        challenge: None,
        extra: Extra::new(),
        scalar_extras: BTreeMap::new(),
        material_extras: std::array::from_fn(|_| Extra::new()),
        zone_extras: Vec::new(),
        clamped_fields: Vec::new(),
    }
    .to_json(SettingsMode::Scenario)
}

/// Writes a save-shaped `.bb8settings` file (no BOM; save bundles add it).
pub fn write_settings_file(
    path: impl AsRef<std::path::Path>,
    settings: &SimSettings,
) -> Result<()> {
    let text = serialize_save_settings(settings)?;
    crate::write_bytes_replacing(path, text)
}

/// Builds the parity bridge `/settings` JSON: `{"scenario":{...},"independent":{...}}`
/// with the game's exact setting names and native JSON bool/int/float tags (the
/// CLI's `settings_hash` encoder uses those types; spec 04 section 8/parity docs
/// section 6.4).
#[must_use]
pub fn to_bridge_settings_json(settings: &SimSettings) -> Value {
    let mut scenario = Map::new();
    for meta in ScenarioSettings::SETTINGS {
        if let Some(value) = settings.scenario.get_by_key(meta.key) {
            scenario.insert(meta.key.to_string(), setting_value_to_json(value));
        }
    }
    let mut independent = Map::new();
    for meta in IndependentSettings::SETTINGS {
        if let Some(value) = settings.independents.get_by_key(meta.key) {
            independent.insert(meta.key.to_string(), setting_value_to_json(value));
        }
    }
    let mut root = Map::new();
    root.insert("scenario".to_string(), Value::Object(scenario));
    root.insert("independent".to_string(), Value::Object(independent));
    Value::Object(root)
}

// ---------------------------------------------------------------------------
// Scalar scopes
// ---------------------------------------------------------------------------

fn parse_scalar_into(file: &mut ScenarioSettingsFile, key: &str, raw: &Value) -> Result<()> {
    let meta = ScenarioSettings::meta(key)
        .ok_or_else(|| Error::invalid_data(format!("unknown scenario setting {key}")))?;
    let (value, extra) = split_wrapper(raw, meta.key)?;
    let parsed = parse_setting_value(&value, meta.kind, meta.key)?;
    if meta.clamped && is_out_of_range(parsed, meta.min, meta.max) {
        file.clamped_fields.push(meta.key.to_string());
        file.scenario.set_raw(meta.key, meta.clamp_value(parsed));
    } else {
        file.scenario.set_raw(meta.key, parsed);
    }
    if !extra.is_empty() {
        file.scalar_extras.insert(meta.key.to_string(), extra);
    }
    Ok(())
}

fn split_wrapper(raw: &Value, key: &str) -> Result<(Value, Extra)> {
    if let Some(object) = raw.as_object() {
        let value = object
            .get("Value")
            .cloned()
            .ok_or_else(|| Error::invalid_data(format!("setting {key} has no Value")))?;
        let mut extra = Extra::new();
        for (name, value) in object {
            if name != "Value" && name != "interactableChanged" {
                extra.insert(name.clone(), value.clone());
            }
        }
        return Ok((value, extra));
    }
    // Some legacy writers emit the bare scalar.
    Ok((raw.clone(), Extra::new()))
}

fn parse_setting_value(raw: &Value, kind: SettingKind, key: &str) -> Result<SettingValue> {
    match kind {
        SettingKind::Bool => raw
            .as_bool()
            .map(SettingValue::Bool)
            .ok_or_else(|| Error::invalid_data(format!("setting {key} is not a bool"))),
        SettingKind::Int => raw
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .map(SettingValue::Int)
            .ok_or_else(|| Error::invalid_data(format!("setting {key} is not an integer"))),
        SettingKind::Float => raw
            .as_f64()
            .map(|value| SettingValue::Float(value as f32))
            .ok_or_else(|| Error::invalid_data(format!("setting {key} is not a number"))),
    }
}

fn is_out_of_range(value: SettingValue, min: Option<f64>, max: Option<f64>) -> bool {
    let value = value.as_f64();
    min.is_some_and(|min| value < min) || max.is_some_and(|max| value > max)
}

fn write_scalar_scope(
    root: &mut Map<String, Value>,
    settings: &ScenarioSettings,
    extras: &BTreeMap<String, Extra>,
) {
    for meta in ScenarioSettings::SETTINGS {
        let Some(value) = settings.get_by_key(meta.key) else {
            continue;
        };
        let mut wrapper = Map::new();
        wrapper.insert(
            "interactableChanged".to_string(),
            serde_json::json!({"m_PersistentCalls": {"m_Calls": []}}),
        );
        wrapper.insert("Value".to_string(), setting_value_to_json(value));
        if let Some(extra) = extras.get(meta.key) {
            merge_extra(&mut wrapper, extra);
        }
        root.insert(meta.key.to_string(), Value::Object(wrapper));
    }
}

fn setting_value_to_json(value: SettingValue) -> Value {
    match value {
        SettingValue::Float(value) => Value::from(f64::from(value)),
        SettingValue::Int(value) => Value::from(i64::from(value)),
        SettingValue::Bool(value) => Value::Bool(value),
    }
}

// ---------------------------------------------------------------------------
// Independent settings
// ---------------------------------------------------------------------------

fn independents_to_json(settings: &IndependentSettings) -> Value {
    let mut object = Map::new();
    for meta in IndependentSettings::SETTINGS {
        if let Some(value) = settings.get_by_key(meta.key) {
            object.insert(
                meta.key.to_string(),
                serde_json::json!({
                    "interactableChanged": {"m_PersistentCalls": {"m_Calls": []}},
                    "Value": setting_value_to_json(value),
                }),
            );
        }
    }
    Value::Object(object)
}

// ---------------------------------------------------------------------------
// Materials
// ---------------------------------------------------------------------------

fn parse_materials(file: &mut ScenarioSettingsFile, value: &Value) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::invalid_data("materials is not an object"))?;
    for (index, kind) in MaterialKind::ALL.into_iter().enumerate() {
        let Some(material_value) = object.get(kind.settings_key()) else {
            continue;
        };
        let material_object = material_value.as_object().ok_or_else(|| {
            Error::invalid_data(format!("{} is not an object", kind.settings_key()))
        })?;
        let setting = file.materials.get_mut(kind);
        for meta in MaterialSetting::FIELDS {
            let Some(raw) = material_object.get(meta.key) else {
                continue;
            };
            let (inner, extra) = split_wrapper(raw, meta.key)?;
            let parsed = parse_setting_value(&inner, meta.kind, meta.key)?;
            setting.set_raw(meta.key, parsed);
            if !extra.is_empty() {
                file.material_extras[index].insert(meta.key.to_string(), Value::Object(extra));
            }
        }
        for (key, raw) in material_object {
            if MaterialSetting::meta(key).is_none() {
                file.material_extras[index].insert(key.clone(), raw.clone());
            }
        }
    }
    Ok(())
}

fn materials_to_json(materials: &MaterialSettings, extras: &[Extra; 4]) -> Value {
    let mut root = Map::new();
    for (index, kind) in MaterialKind::ALL.into_iter().enumerate() {
        let material = materials.get(kind);
        let mut object = Map::new();
        for meta in MaterialSetting::FIELDS {
            if (meta.key == "freshTime" || meta.key == "decayRate") && !material.decay {
                continue;
            }
            if let Some(value) = material.get_by_key(meta.key) {
                object.insert(meta.key.to_string(), setting_value_to_json(value));
            }
        }
        if let Some(extra) = extras.get(index) {
            merge_extra(&mut object, extra);
        }
        root.insert(kind.settings_key().to_string(), Value::Object(object));
    }
    Value::Object(root)
}

// ---------------------------------------------------------------------------
// Zones
// ---------------------------------------------------------------------------

fn parse_zones(file: &mut ScenarioSettingsFile, value: &Value) -> Result<()> {
    let array = value
        .as_array()
        .ok_or_else(|| Error::invalid_data("zones is not an array"))?;
    for (index, raw) in array.iter().enumerate() {
        let (zone, extra) = zone_from_json(raw, index as u32)?;
        file.zones.push(zone);
        file.zone_extras.push(extra);
    }
    Ok(())
}

fn zone_from_json(raw: &Value, index: u32) -> Result<(ZoneSettings, Extra)> {
    let object = raw
        .as_object()
        .ok_or_else(|| Error::invalid_data(format!("zone {index} is not an object")))?;
    let mut zone = ZoneSettings {
        zone_id: object
            .get("id")
            .and_then(Value::as_u64)
            .map(|id| id as u32)
            .or(Some(index)),
        ..ZoneSettings::default()
    };
    if let Some(name) = object.get("name").and_then(Value::as_str) {
        zone.name = name.to_string();
    }
    if let Some(material) = object.get("material").and_then(Value::as_str) {
        zone.material = MaterialKind::from_name(material);
    }
    if let Some(distribution) = object.get("distribution").and_then(Value::as_str) {
        zone.distribution = SpawnDistribution::from_name(distribution)
            .ok_or_else(|| Error::invalid_data(format!("zone {index}: unknown distribution")))?;
    }
    zone.fertility = get_f32(object, "fertility", zone.fertility);
    zone.biomass_density = get_f32(object, "biomassDensity", zone.biomass_density);
    zone.pellet_size = get_f32(object, "pelletSize", zone.pellet_size);
    if let Some(value) = object.get("renewBiomass").and_then(Value::as_bool) {
        zone.renew_biomass = value;
    }
    if let Some(movement) = object.get("movement").and_then(Value::as_str) {
        zone.movement = MovementType::from_name(movement)
            .ok_or_else(|| Error::invalid_data(format!("zone {index}: unknown movement")))?;
    }
    if let Some(target) = object.get("target").and_then(Value::as_u64) {
        zone.target = Some(target as u32);
    }
    zone.speed = get_f32(object, "speed", zone.speed);
    zone.pos_x = get_f32(object, "posX", zone.pos_x);
    zone.pos_y = get_f32(object, "posY", zone.pos_y);
    if let Some(relative) = object.get("radiusIsRelative").and_then(Value::as_bool) {
        zone.radius_is_relative = relative;
    }
    zone.inside_radius = get_f32(object, "insideRadius", zone.inside_radius);
    // `ZoneSettings.SaveState/LoadState` (cs:711-720): `radius`/`width`/`height`
    // hold the relative value when `radiusIsRelative`, the absolute one
    // otherwise. Only the field that was written is touched; the other
    // variant is derived (relative x 4000, the reference simulation size).
    if zone.radius_is_relative {
        zone.radius_relative = get_f32(object, "radius", zone.radius_relative);
        zone.radius_absolute = zone.radius_relative * 4000.0;
        zone.width_relative = get_f32(object, "width", zone.width_relative);
        zone.height_relative = get_f32(object, "height", zone.height_relative);
        zone.width_absolute = zone.width_relative * 4000.0;
        zone.height_absolute = zone.height_relative * 4000.0;
    } else {
        zone.radius_absolute = get_f32(object, "radius", zone.radius_absolute);
        zone.width_absolute = get_f32(object, "width", zone.width_absolute);
        zone.height_absolute = get_f32(object, "height", zone.height_absolute);
    }
    let mut extra = Extra::new();
    for (key, value) in object {
        if !ZONE_KEYS.contains(&key.as_str()) {
            extra.insert(key.clone(), value.clone());
        }
    }
    zone.clamp_to_ranges();
    Ok((zone, extra))
}

const ZONE_KEYS: [&str; 19] = [
    "name",
    "id",
    "material",
    "distribution",
    "fertility",
    "biomassDensity",
    "pelletSize",
    "renewBiomass",
    "movement",
    "target",
    "speed",
    "posX",
    "posY",
    "radiusIsRelative",
    "radius",
    "insideRadius",
    "width",
    "height",
    "zoneID",
];

fn zone_to_json(zone: &ZoneSettings, id: Option<u32>) -> Map<String, Value> {
    let mut object = Map::new();
    if !zone.name.is_empty() {
        object.insert("name".to_string(), Value::from(zone.name.clone()));
    }
    if let Some(id) = id.or(zone.zone_id) {
        object.insert("id".to_string(), Value::from(i64::from(id)));
    }
    if let Some(material) = zone.material {
        object.insert("material".to_string(), Value::from(material.name()));
        object.insert(
            "distribution".to_string(),
            Value::from(zone.distribution.name()),
        );
        object.insert(
            "fertility".to_string(),
            Value::from(f64::from(zone.fertility)),
        );
        object.insert(
            "biomassDensity".to_string(),
            Value::from(f64::from(zone.biomass_density)),
        );
        object.insert(
            "pelletSize".to_string(),
            Value::from(f64::from(zone.pellet_size)),
        );
    }
    if zone.renew_biomass {
        object.insert("renewBiomass".to_string(), Value::Bool(true));
    }
    object.insert("movement".to_string(), Value::from(zone.movement.name()));
    if let Some(target) = zone.target {
        object.insert("target".to_string(), Value::from(i64::from(target)));
    }
    object.insert("speed".to_string(), Value::from(f64::from(zone.speed)));
    object.insert("posX".to_string(), Value::from(f64::from(zone.pos_x)));
    object.insert("posY".to_string(), Value::from(f64::from(zone.pos_y)));
    object.insert(
        "radiusIsRelative".to_string(),
        Value::Bool(zone.radius_is_relative),
    );
    if zone.distribution.is_rect() {
        let (width, height) = if zone.radius_is_relative {
            (zone.width_relative, zone.height_relative)
        } else {
            (zone.width_absolute, zone.height_absolute)
        };
        object.insert("width".to_string(), Value::from(f64::from(width)));
        object.insert("height".to_string(), Value::from(f64::from(height)));
    } else {
        let radius = if zone.radius_is_relative {
            zone.radius_relative
        } else {
            zone.radius_absolute
        };
        object.insert("radius".to_string(), Value::from(f64::from(radius)));
        if zone.distribution.is_ring() {
            object.insert(
                "insideRadius".to_string(),
                Value::from(f64::from(zone.inside_radius)),
            );
        }
    }
    object
}

fn get_f32(object: &Map<String, Value>, key: &str, default: f32) -> f32 {
    object
        .get(key)
        .and_then(Value::as_f64)
        .map_or(default, |value| value as f32)
}

// ---------------------------------------------------------------------------
// Zone groups, spawn rows, animators, challenge
// ---------------------------------------------------------------------------

fn parse_zone_groups(file: &mut ScenarioSettingsFile, value: &Value) -> Result<()> {
    let array = value
        .as_array()
        .ok_or_else(|| Error::invalid_data("zoneGroups is not an array"))?;
    for raw in array {
        let group = zone_group_from_json(raw)?;
        file.zone_groups.push(group);
    }
    Ok(())
}

fn zone_group_from_json(raw: &Value) -> Result<ZoneGroupSettings> {
    let object = raw
        .as_object()
        .ok_or_else(|| Error::invalid_data("zone group is not an object"))?;
    let mut group = ZoneGroupSettings::default();
    if let Some(name) = object.get("name").and_then(Value::as_str) {
        group.name = name.to_string();
    }
    if let Some(scale) = object.get("scaleWithSim").and_then(Value::as_bool) {
        group.scale_with_sim = scale;
    }
    if let Some(count) = object.get("count") {
        if group.scale_with_sim {
            group.zone_density = count
                .as_f64()
                .ok_or_else(|| Error::invalid_data("zone group count is not a number"))?
                as f32;
        } else {
            group.zone_count = count
                .as_i64()
                .and_then(|value| i32::try_from(value).ok())
                .ok_or_else(|| Error::invalid_data("zone group count is not an integer"))?;
        }
    }
    if let Some(template) = object.get("template") {
        group.template = zone_from_json(template, 0)?.0;
    }
    if let Some(zones) = object.get("zones").and_then(Value::as_array) {
        for (index, zone) in zones.iter().enumerate() {
            group.zones.push(zone_from_json(zone, index as u32)?.0);
        }
    }
    Ok(group)
}

fn zone_group_to_json(group: &ZoneGroupSettings) -> Map<String, Value> {
    let mut object = Map::new();
    object.insert("name".to_string(), Value::from(group.name.clone()));
    object.insert(
        "scaleWithSim".to_string(),
        Value::Bool(group.scale_with_sim),
    );
    if group.scale_with_sim {
        object.insert(
            "count".to_string(),
            Value::from(f64::from(group.zone_density)),
        );
    } else {
        object.insert(
            "count".to_string(),
            Value::from(i64::from(group.zone_count)),
        );
    }
    object.insert(
        "template".to_string(),
        Value::Object(zone_to_json(&group.template, None)),
    );
    if !group.zones.is_empty() {
        let zones = group
            .zones
            .iter()
            .enumerate()
            .map(|(index, zone)| Value::Object(zone_to_json(zone, Some(index as u32))))
            .collect();
        object.insert("zones".to_string(), Value::Array(zones));
    }
    object
}

fn parse_spawns(file: &mut ScenarioSettingsFile, value: &Value) -> Result<()> {
    let array = value
        .as_array()
        .ok_or_else(|| Error::invalid_data("bibites is not an array"))?;
    for raw in array {
        let object = raw
            .as_object()
            .ok_or_else(|| Error::invalid_data("spawn row is not an object"))?;
        let mut spawn = BibiteSpawn::game_defaults();
        if let Some(path) = object.get("path").and_then(Value::as_str) {
            spawn.path = path.to_string();
        }
        spawn.spawn_priority = get_f32(object, "spawnPriority", spawn.spawn_priority);
        if let Some(minimum) = object.get("minimum").and_then(Value::as_i64) {
            spawn.minimum = minimum as i32;
        }
        if let Some(mode) = object.get("randomizeGenes").and_then(Value::as_str) {
            spawn.randomize_genes = RandomizeGenes::from_name(mode)
                .ok_or_else(|| Error::invalid_data("unknown randomizeGenes"))?;
        }
        if let Some(growth) = object.get("growthAtSpawn").and_then(Value::as_str) {
            spawn.growth_at_spawn = GrowthAtSpawn::from_name(growth)
                .ok_or_else(|| Error::invalid_data("unknown growthAtSpawn"))?;
        }
        if let Some(tagging) = object.get("tagging").and_then(Value::as_str) {
            spawn.tagging = Tagging::from_name(tagging)
                .ok_or_else(|| Error::invalid_data("unknown tagging"))?;
        }
        if let Some(spawn_type) = object.get("spawnType").and_then(Value::as_str) {
            spawn.spawn_type = SpawnType::from_name(spawn_type)
                .ok_or_else(|| Error::invalid_data("unknown spawnType"))?;
        }
        if let Some(tag) = object.get("tag").and_then(Value::as_str) {
            spawn.tag = tag.to_string();
        }
        if let Some(spawned) = object.get("totalSpawned").and_then(Value::as_i64) {
            spawn.total_spawned = spawned as i32;
        }
        if let Some(external) = object.get("isExternal").and_then(Value::as_bool) {
            spawn.is_external = external;
        }
        if let Some(zone) = object.get("spawnZone").and_then(Value::as_u64) {
            spawn.spawn_zone = Some(zone as u32);
        }
        file.spawns.push(spawn);
    }
    Ok(())
}

fn spawn_to_json(spawn: &BibiteSpawn) -> Map<String, Value> {
    let mut object = Map::new();
    object.insert("path".to_string(), Value::from(spawn.path.clone()));
    object.insert(
        "spawnPriority".to_string(),
        Value::from(f64::from(spawn.spawn_priority)),
    );
    object.insert("minimum".to_string(), Value::from(i64::from(spawn.minimum)));
    object.insert(
        "randomizeGenes".to_string(),
        Value::from(spawn.randomize_genes.name()),
    );
    object.insert(
        "growthAtSpawn".to_string(),
        Value::from(spawn.growth_at_spawn.name()),
    );
    object.insert("tagging".to_string(), Value::from(spawn.tagging.name()));
    object.insert(
        "spawnType".to_string(),
        Value::from(spawn.spawn_type.name()),
    );
    if spawn.tagging == Tagging::CustomTagging {
        object.insert("tag".to_string(), Value::from(spawn.tag.clone()));
    }
    if spawn.total_spawned > 0 {
        object.insert(
            "totalSpawned".to_string(),
            Value::from(i64::from(spawn.total_spawned)),
        );
    }
    if spawn.is_external {
        object.insert("isExternal".to_string(), Value::Bool(true));
    }
    if let Some(zone) = spawn.spawn_zone {
        object.insert("spawnZone".to_string(), Value::from(i64::from(zone)));
    }
    object
}

fn parse_changers(file: &mut ScenarioSettingsFile, value: &Value) -> Result<()> {
    let array = value
        .as_array()
        .ok_or_else(|| Error::invalid_data("settingsChangers is not an array"))?;
    for raw in array {
        let object = raw
            .as_object()
            .ok_or_else(|| Error::invalid_data("settings changer is not an object"))?;
        let mut changer = SettingsChanger::game_defaults();
        if let Some(name) = object.get("name").and_then(Value::as_str) {
            changer.name = name.to_string();
        }
        if let Some(bases) = object.get("settingsBases").and_then(Value::as_object) {
            for (key, base) in bases {
                changer
                    .settings_bases
                    .push(bibites_core::settings::SettingBase {
                        key: key.clone(),
                        base: base.as_f64().ok_or_else(|| {
                            Error::invalid_data("settingsBases value is not a number")
                        })? as f32,
                    });
            }
        }
        let x = object.get("t").and_then(Value::as_array);
        let y = object.get("y").and_then(Value::as_array);
        let d = object.get("d").and_then(Value::as_array);
        let f = object.get("f").and_then(Value::as_array);
        if let (Some(x), Some(y), Some(d), Some(f)) = (x, y, d, f) {
            if x.len() != y.len() || x.len() != d.len() || x.len() != f.len() {
                return Err(Error::invalid_data(
                    "settings changer arrays differ in length",
                ));
            }
            for index in 0..x.len() {
                let interpolation = d[index]
                    .as_str()
                    .and_then(bibites_core::settings::InterpolationType::from_name)
                    .ok_or_else(|| Error::invalid_data("unknown interpolation type"))?;
                changer.points.push(SplinePoint::with_interpolation(
                    x[index].as_f64().unwrap_or(0.0) as f32,
                    y[index].as_f64().unwrap_or(0.0) as f32,
                    interpolation,
                    f[index].as_f64().unwrap_or(2.0) as f32,
                ));
            }
        }
        if let Some(repeat) = object.get("repeat").and_then(Value::as_bool) {
            changer.repeat = repeat;
        }
        changer.start = get_f32(object, "start", changer.start);
        file.changers.push(changer);
    }
    Ok(())
}

fn changer_to_json(changer: &SettingsChanger) -> Map<String, Value> {
    let mut object = Map::new();
    object.insert("name".to_string(), Value::from(changer.name.clone()));
    let mut bases = Map::new();
    for base in &changer.settings_bases {
        bases.insert(base.key.clone(), Value::from(f64::from(base.base)));
    }
    object.insert("settingsBases".to_string(), Value::Object(bases));
    object.insert(
        "t".to_string(),
        Value::Array(
            changer
                .points
                .iter()
                .map(|point| Value::from(f64::from(point.x)))
                .collect(),
        ),
    );
    object.insert(
        "y".to_string(),
        Value::Array(
            changer
                .points
                .iter()
                .map(|point| Value::from(f64::from(point.y)))
                .collect(),
        ),
    );
    object.insert(
        "d".to_string(),
        Value::Array(
            changer
                .points
                .iter()
                .map(|point| Value::from(point.interpolation.name()))
                .collect(),
        ),
    );
    object.insert(
        "f".to_string(),
        Value::Array(
            changer
                .points
                .iter()
                .map(|point| Value::from(f64::from(point.ease_factor)))
                .collect(),
        ),
    );
    object.insert("repeat".to_string(), Value::Bool(changer.repeat));
    object.insert("start".to_string(), Value::from(f64::from(changer.start)));
    object
}

fn parse_challenge(value: &Value) -> Result<ChallengeParameters> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::invalid_data("challenge is not an object"))?;
    let mut challenge = ChallengeParameters::game_defaults();
    if let Some(exit) = object.get("exit") {
        challenge.exit_condition = parse_condition_group(exit)?;
    }
    if let Some(metric) = object.get("scoringMetric").and_then(Value::as_str) {
        challenge.scoring_metric.metric = SimMetric::from_name(metric)
            .ok_or_else(|| Error::invalid_data("unknown scoringMetric"))?;
    }
    if let Some(argument) = object.get("scoringArg").and_then(Value::as_str) {
        challenge.scoring_metric.argument = argument.to_string();
    }
    if let Some(high) = object.get("scoringHigh").and_then(Value::as_bool) {
        challenge.high_score_is_better = high;
    }
    for (key, index) in [("oneStar", 0), ("twoStar", 1), ("threeStar", 2)] {
        if let Some(raw) = object
            .get(key)
            .or_else(|| object.get(&format!("star{}", index + 1)))
        {
            let group = parse_condition_group(raw)?;
            match index {
                0 => challenge.one_star_condition = group,
                1 => challenge.two_star_condition = group,
                _ => challenge.three_star_condition = group,
            }
        }
    }
    if let Some(champion) = object.get("champion") {
        // Champion rows use the spawn-row keys minus `path`.
        let mut row = Value::Object(champion.as_object().cloned().unwrap_or_default());
        if let Some(object) = row.as_object_mut() {
            object
                .entry("path".to_string())
                .or_insert_with(|| Value::from(String::new()));
        }
        let wrapper = Value::Array(vec![row]);
        let mut file = ScenarioSettingsFile {
            scenario: ScenarioSettings::default(),
            independents: None,
            recommended: None,
            materials: MaterialSettings::default(),
            zones: Vec::new(),
            zone_groups: Vec::new(),
            spawns: Vec::new(),
            changers: Vec::new(),
            challenge: None,
            extra: Extra::new(),
            scalar_extras: BTreeMap::new(),
            material_extras: std::array::from_fn(|_| Extra::new()),
            zone_extras: Vec::new(),
            clamped_fields: Vec::new(),
        };
        parse_spawns(&mut file, &wrapper)?;
        if let Some(spawn) = file.spawns.into_iter().next() {
            challenge.champion = spawn;
        }
        // `BibiteSettings.LoadForChampion` (cs:250-272) forces custom tagging
        // with the tag "Champion" (the champion tag is never written).
        challenge.champion.tagging = bibites_core::settings::Tagging::CustomTagging;
        challenge.champion.tag = "Champion".to_owned();
    }
    challenge.exit_condition.simplify();
    challenge.one_star_condition.simplify();
    challenge.two_star_condition.simplify();
    challenge.three_star_condition.simplify();
    Ok(challenge)
}

fn challenge_to_json(challenge: &ChallengeParameters) -> Map<String, Value> {
    let mut object = Map::new();
    object.insert(
        "exit".to_string(),
        Value::Object(condition_group_to_json(&challenge.exit_condition)),
    );
    object.insert(
        "scoringMetric".to_string(),
        Value::from(challenge.scoring_metric.metric.name()),
    );
    if challenge.scoring_metric.metric.has_argument() {
        object.insert(
            "scoringArg".to_string(),
            Value::from(challenge.scoring_metric.argument.clone()),
        );
    }
    object.insert(
        "scoringHigh".to_string(),
        Value::Bool(challenge.high_score_is_better),
    );
    object.insert(
        "star1".to_string(),
        Value::Object(condition_group_to_json(&challenge.one_star_condition)),
    );
    object.insert(
        "star2".to_string(),
        Value::Object(condition_group_to_json(&challenge.two_star_condition)),
    );
    object.insert(
        "star3".to_string(),
        Value::Object(condition_group_to_json(&challenge.three_star_condition)),
    );
    let mut champion = spawn_to_json(&challenge.champion);
    champion.remove("path");
    object.insert("champion".to_string(), Value::Object(champion));
    object
}

fn parse_condition_group(value: &Value) -> Result<ConditionGroup> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::invalid_data("condition group is not an object"))?;
    let mut group = ConditionGroup::new(ConditionGroupLogic::And);
    if let Some(logic) = object.get("logic").and_then(Value::as_str) {
        group.logic = ConditionGroupLogic::from_name(logic)
            .ok_or_else(|| Error::invalid_data("unknown condition logic"))?;
    }
    if let Some(subs) = object.get("subs").and_then(Value::as_array) {
        for sub in subs {
            if sub.get("logic").is_some() {
                group
                    .sub_conditions
                    .push(ConditionNode::Group(parse_condition_group(sub)?));
            } else {
                group
                    .sub_conditions
                    .push(ConditionNode::Sim(parse_sim_condition(sub)?));
            }
        }
    }
    Ok(group)
}

fn parse_sim_condition(value: &Value) -> Result<SimCondition> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::invalid_data("condition is not an object"))?;
    let metric = |key: &str| -> Result<SimMetric> {
        object
            .get(key)
            .and_then(Value::as_str)
            .and_then(SimMetric::from_name)
            .ok_or_else(|| Error::invalid_data(format!("condition missing {key}")))
    };
    let argument = |key: &str| -> String {
        object
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let comparator = object
        .get("comp")
        .and_then(Value::as_str)
        .and_then(Comparator::from_name)
        .ok_or_else(|| Error::invalid_data("condition missing comp"))?;
    Ok(SimCondition::new(
        metric("metric1")?,
        argument("arg1"),
        comparator,
        metric("metric2")?,
        argument("arg2"),
    ))
}

fn condition_group_to_json(group: &ConditionGroup) -> Map<String, Value> {
    let mut object = Map::new();
    object.insert("logic".to_string(), Value::from(group.logic.name()));
    let mut subs = Vec::new();
    for node in &group.sub_conditions {
        match node {
            ConditionNode::Sim(condition) => {
                let mut object = Map::new();
                object.insert(
                    "metric1".to_string(),
                    Value::from(condition.left.metric.name()),
                );
                object.insert(
                    "arg1".to_string(),
                    Value::from(condition.left.argument.clone()),
                );
                object.insert("comp".to_string(), Value::from(condition.comparator.name()));
                object.insert(
                    "metric2".to_string(),
                    Value::from(condition.right.metric.name()),
                );
                object.insert(
                    "arg2".to_string(),
                    Value::from(condition.right.argument.clone()),
                );
                subs.push(Value::Object(object));
            }
            ConditionNode::Group(group) => subs.push(Value::Object(condition_group_to_json(group))),
        }
    }
    object.insert("subs".to_string(), Value::Array(subs));
    object
}

fn merge_extra(object: &mut Map<String, Value>, extra: &Extra) {
    for (key, value) in extra {
        object.entry(key.clone()).or_insert_with(|| value.clone());
    }
}

#[cfg(test)]
mod zone_size_tests {
    use super::*;

    /// `ZoneSettings.SaveState/LoadState` (cs:711-720): the `radius`/`width`/
    /// `height` keys carry the relative value when `radiusIsRelative` and the
    /// absolute one otherwise; absolute sizes must survive a round trip.
    #[test]
    fn absolute_zone_sizes_round_trip() {
        let raw = serde_json::json!({
            "id": 0,
            "name": "box",
            "radiusIsRelative": false,
            "width": 640.0,
            "height": 320.0,
            "distribution": "Rect",
        });
        let (zone, _extra) = zone_from_json(&raw, 0).expect("zone parses");
        assert!(!zone.radius_is_relative);
        assert!(
            (zone.width_absolute - 640.0).abs() < 1e-3,
            "{}",
            zone.width_absolute
        );
        assert!((zone.height_absolute - 320.0).abs() < 1e-3);
        let written = zone_to_json(&zone, Some(0));
        let width = written.get("width").and_then(Value::as_f64).unwrap();
        let height = written.get("height").and_then(Value::as_f64).unwrap();
        assert!((width - 640.0).abs() < 1e-3, "{width}");
        assert!((height - 320.0).abs() < 1e-3, "{height}");
        assert_eq!(written.get("radiusIsRelative"), Some(&Value::Bool(false)));
    }
}

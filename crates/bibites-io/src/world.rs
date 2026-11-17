//! Save/scenario bundle ↔ core [`WorldContents`] conversion (IO I4a).
//!
//! `bibites-io` keeps raw JSON for scene/pellets/pheromones in [`SaveBundle`];
//! this module maps those sections into the core plain records that
//! `World::from_contents`/`World::to_contents` consume, and back. The mapping
//! quirks mirror the game loader (spec 04 §5.3-§5.5):
//!
//! * pellets: position/material/amount/owner bucket/fresh time/rot only;
//!   velocity and rotation are dropped (pellet respawns at rest, identity rot);
//!   `matterDecay.timeAlive` → `fresh_time_remaining = material.freshTime − timeAlive`
//!   for decaying pellets (Meat), `None` otherwise; `rotAmount` carried over.
//! * pheromones: `heading` is lost by the game serializer (`{}` → `Vec2::ZERO`);
//!   strengths keep their sign; spots load as dissipating bibite spots.
//! * tools: rad/phero towers keep their serialized `id`; phero tower runtime
//!   `spot` ids are not serialized (set `None` on load).
//! * scene: name/version/simulatedTime/tags/spawn rows; no red-death state.
//! * species/data.bin pass through unchanged; raw entity ids stay in the
//!   [`SaveBundle`] side vectors (io is the canonical raw-id layer).

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use bibites_core::Handle;
use bibites_core::ids::ZoneId;
use bibites_core::math::Vec2;
use bibites_core::pellet::MaterialKind as PelletMaterial;
use bibites_core::settings::{MaterialKind, SimSettings};
use bibites_core::species::LineageRecordIo;
use bibites_core::tools::{ColorKiller, PheroTower, RadTower, TowerBase};
use bibites_core::world::{PelletRecord, PheromoneRecord, WorldContents, WorldScene};
use bibites_core::{Error, Result};

use crate::bundle::{SaveBundle, ScenarioBundle};
use crate::model::Extra;
use crate::settings::{ScenarioSettingsFile, SettingsMode, resolve_effective_settings};

/// Loads a save `.zip` into core [`WorldContents`].
///
/// `settings_override` replaces the save's effective settings (CLI
/// `--settings/--independents`); the serialized save settings are otherwise kept.
/// `WorldContents.seed` defaults to 0 — the CLI must set its capture seed before
/// calling `World::from_contents`.
pub fn load_world_contents(
    path: impl AsRef<std::path::Path>,
    settings_override: Option<&SimSettings>,
) -> Result<WorldContents> {
    let bundle = crate::load_save_bundle(path)?;
    let mut contents = world_contents_from_save(&bundle)?;
    if let Some(settings) = settings_override {
        contents.settings = settings.clone();
    }
    Ok(contents)
}

/// Writes core [`WorldContents`] as a game-loadable save `.zip`.
pub fn save_world_contents(
    path: impl AsRef<std::path::Path>,
    contents: &WorldContents,
) -> Result<()> {
    let bundle = save_bundle_from_contents(contents, None)?;
    crate::write_save_bundle(&bundle, path)
}

/// Loads a scenario `.zip` into [`WorldContents`] (for `run <scenario.zip>`).
///
/// The returned contents carry the scenario's zones/zone-groups/spawn rows and
/// animators, empty entity/pellet/pheromone lists (a fresh world is built with
/// `World::new` + `spawn_initial_population`, not `from_contents`), and seed 0.
pub fn load_scenario_world_contents(
    path: impl AsRef<std::path::Path>,
    settings_override: Option<&SimSettings>,
) -> Result<WorldContents> {
    let scenario = crate::load_scenario_bundle(path)?;
    let mut contents = scenario_world_contents(&scenario);
    if let Some(settings) = settings_override {
        contents.settings = settings.clone();
    }
    Ok(contents)
}

/// Converts a loaded scenario bundle into [`WorldContents`] shell.
#[must_use]
pub fn scenario_world_contents(scenario: &ScenarioBundle) -> WorldContents {
    WorldContents {
        settings: scenario.settings.clone(),
        seed: 0,
        scene: WorldScene {
            name: scenario.info.name.clone(),
            version: scenario.info.version_text.clone(),
            simulated_time: 0.0,
            tags: Vec::new(),
            bibite_spawns: scenario.scenario.bibite_spawns.clone(),
        },
        zones: scenario.scenario.zones.clone(),
        zone_groups: scenario.scenario.zone_groups.clone(),
        settings_changers: scenario.scenario.settings_changers.clone(),
        rad_towers: scenario.scenario.rad_towers.clone(),
        phero_towers: scenario.scenario.phero_towers.clone(),
        color_killers: scenario.scenario.color_killers.clone(),
        pellets: Vec::new(),
        pheromones: Vec::new(),
        bibites: Vec::new(),
        eggs: Vec::new(),
        bibite_parents: Vec::new(),
        egg_parents: Vec::new(),
        species: None,
        data_bin: Vec::new(),
        img_png: None,
    }
}

/// Maps a parsed [`SaveBundle`] to core [`WorldContents`].
pub fn world_contents_from_save(bundle: &SaveBundle) -> Result<WorldContents> {
    let scene = &bundle.scene;
    let world_scene = WorldScene {
        name: scene
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        version: scene
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        simulated_time: scene
            .get("simulatedTime")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        tags: scene
            .get("tags")
            .and_then(Value::as_array)
            .map(|tags| {
                tags.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        bibite_spawns: bundle.settings_file.spawns.clone(),
    };
    // The world keeps the explicit zones and the groups apart (a group keeps the zones it generated). `ScenarioSettings.UpdateAllZonesList`
    // numbers the explicit zones first and then each group's saved zones: the zone ids and the pellets' owner indices refer to this order.
    // Groups saved without zones are regenerated on load (`DeserializeScenario`: `if (zoneGroup.zones.Count < 1) GenerateZones()`).
    let explicit_zones = bundle.settings_file.zones.clone();
    let zone_groups = bundle.settings_file.zone_groups.clone();
    let mut zones = explicit_zones.clone();
    for group in &zone_groups {
        zones.extend(group.zones.iter().cloned());
    }
    let pellets = bundle
        .pellets
        .as_ref()
        .map(|pellets| parse_pellets(pellets, &zones, &bundle.settings))
        .transpose()?
        .unwrap_or_default();
    let pheromones = bundle
        .pheromones
        .as_ref()
        .map(parse_pheromones)
        .transpose()?
        .unwrap_or_default();
    let zones_for_killers = zones.clone();
    let (bibite_parents, egg_parents) = resolve_parent_links(bundle);
    let rad_towers = parse_rad_towers(scene, &zones)?;
    let phero_towers = parse_phero_towers(scene, &zones)?;
    Ok(WorldContents {
        settings: bundle.settings.clone(),
        seed: 0,
        scene: world_scene,
        zones: explicit_zones,
        zone_groups,
        settings_changers: bundle.settings_file.changers.clone(),
        rad_towers,
        phero_towers,
        color_killers: parse_color_killers(scene, &zones_for_killers)?,
        pellets,
        pheromones,
        bibites: bundle.bibites.clone(),
        eggs: bundle.eggs.clone(),
        bibite_parents,
        egg_parents,
        species: bundle.species.clone(),
        data_bin: bundle.data_bin.clone().unwrap_or_default(),
        img_png: None,
    })
}

/// `SaveSystem.LoadWorld`'s id resolution: a bibite's or egg's `parent1` is looked up among the bibites' own ids (0 means none,
/// the first bibite with that id wins), and `parent2` only when the first was found. Returns the positions in the bundle's lists.
fn resolve_parent_links(bundle: &SaveBundle) -> (Vec<[Option<u32>; 2]>, Vec<[Option<u32>; 2]>) {
    let mut slot_of: std::collections::HashMap<i32, u32> =
        std::collections::HashMap::with_capacity(bundle.bibite_ids.len());
    for (slot, id) in bundle.bibite_ids.iter().enumerate() {
        slot_of.entry(*id).or_insert(slot as u32);
    }
    let resolve = |(first, second): (Option<i32>, Option<i32>)| -> [Option<u32>; 2] {
        let find = |raw: Option<i32>| {
            raw.filter(|raw| *raw != 0)
                .and_then(|raw| slot_of.get(&raw).copied())
        };
        let first = find(first);
        let second = first.and_then(|_| find(second));
        [first, second]
    };
    let bibites = (0..bundle.bibites.len())
        .map(|index| {
            resolve(
                bundle
                    .bibite_parents
                    .get(index)
                    .copied()
                    .unwrap_or_default(),
            )
        })
        .collect();
    let eggs = (0..bundle.eggs.len())
        .map(|index| resolve(bundle.egg_parents.get(index).copied().unwrap_or_default()))
        .collect();
    (bibites, eggs)
}

/// Builds a [`SaveBundle`] from [`WorldContents`] for writing.
///
/// `base` (an input bundle) preserves data that `WorldContents` cannot model
/// (templates, screenshot, unknown entries, `vars`, settings/scene extras, and
/// raw entity ids when the entity counts still match). Pass `None` for a fresh
/// save from a live world.
pub fn save_bundle_from_contents(
    contents: &WorldContents,
    base: Option<&SaveBundle>,
) -> Result<SaveBundle> {
    let mut settings_file = match base {
        Some(base) => base.settings_file.clone(),
        None => empty_settings_file(contents.settings.clone()),
    };
    settings_file.scenario = contents.settings.scenario.clone();
    settings_file.materials = contents.settings.materials;
    settings_file.independents = Some(contents.settings.independents.clone());
    settings_file.zones = contents.zones.clone();
    settings_file.zone_groups = contents.zone_groups.clone();
    settings_file.spawns = contents.scene.bibite_spawns.clone();
    settings_file.changers = contents.settings_changers.clone();

    let scene = scene_to_json(contents, base.map(|bundle| &bundle.scene))?;
    let pellets = pellets_to_json(contents);
    let pheromones = pheromones_to_json(contents);

    // Raw runtime ids (`BibiteID.id`): those of the bundle this world was loaded from while its lists still line up, otherwise 1, 2, ...
    // for the bibites and then the eggs (they share one id space in the game and 0 means none).
    let bibite_count = contents.bibites.len();
    let base_bibites = base.filter(|base| base.bibites.len() == bibite_count);
    let base_eggs = base.filter(|base| base.eggs.len() == contents.eggs.len());
    let bibite_ids: Vec<i32> = (0..bibite_count)
        .map(|index| base_bibites.map_or(index as i32 + 1, |base| base.bibite_ids[index]))
        .collect();
    let egg_ids: Vec<i32> = (0..contents.eggs.len())
        .map(|index| {
            base_eggs.map_or((bibite_count + index) as i32 + 1, |base| {
                base.egg_ids[index]
            })
        })
        .collect();
    let bibite_extras: Vec<Extra> = (0..bibite_count)
        .map(|index| base_bibites.map_or_else(Extra::new, |base| base.bibite_extras[index].clone()))
        .collect();
    let egg_extras: Vec<Extra> = (0..contents.eggs.len())
        .map(|index| base_eggs.map_or_else(Extra::new, |base| base.egg_extras[index].clone()))
        .collect();
    // The links of the world as raw ids; a world that does not say (`bibite_parents` empty) keeps the bundle's own.
    let linked = contents.bibite_parents.len() == bibite_count
        && contents.egg_parents.len() == contents.eggs.len();
    let raw_of = |slot: Option<u32>| slot.and_then(|slot| bibite_ids.get(slot as usize).copied());
    let raw_parents = |slots: &[Option<u32>; 2]| (raw_of(slots[0]), raw_of(slots[1]));
    let (bibite_parents, egg_parents, bibite_children) = if linked {
        let mut children: Vec<Vec<i32>> = vec![Vec::new(); bibite_count];
        // `eggLayer.children`: the living offspring and the eggs that are not hatched yet.
        for (index, slots) in contents.bibite_parents.iter().enumerate() {
            if let Some(parent) = slots[0]
                && !contents.bibites[index].dead
                && let Some(list) = children.get_mut(parent as usize)
            {
                list.push(bibite_ids[index]);
            }
        }
        for (index, slots) in contents.egg_parents.iter().enumerate() {
            if let Some(parent) = slots[0]
                && let Some(list) = children.get_mut(parent as usize)
            {
                list.push(egg_ids[index]);
            }
        }
        (
            contents.bibite_parents.iter().map(raw_parents).collect(),
            contents.egg_parents.iter().map(raw_parents).collect(),
            children,
        )
    } else {
        (
            (0..bibite_count)
                .map(|index| base_bibites.map_or((None, None), |base| base.bibite_parents[index]))
                .collect(),
            (0..contents.eggs.len())
                .map(|index| {
                    base_eggs
                        .and_then(|base| base.egg_parents.get(index).copied())
                        .unwrap_or_default()
                })
                .collect(),
            (0..bibite_count)
                .map(|index| {
                    base_bibites.map_or_else(Vec::new, |base| base.bibite_children[index].clone())
                })
                .collect(),
        )
    };

    Ok(SaveBundle {
        settings: contents.settings.clone(),
        settings_file,
        scene,
        version: crate::Version::parse(&contents.scene.version)?,
        vars: base.and_then(|bundle| bundle.vars.clone()),
        pellets: Some(pellets),
        pheromones: Some(pheromones),
        bibites: contents.bibites.clone(),
        bibite_ids,
        bibite_extras,
        bibite_parents,
        bibite_children,
        eggs: contents.eggs.clone(),
        egg_extras,
        egg_ids,
        egg_parents,
        species: contents.species.clone(),
        species_raw: None,
        data_bin: Some(contents.data_bin.clone()),
        templates: base
            .map(|bundle| bundle.templates.clone())
            .unwrap_or_default(),
        // A fresh screenshot of the world when the window made one, else the loaded save's.
        img_png: contents
            .img_png
            .clone()
            .or_else(|| base.and_then(|bundle| bundle.img_png.clone())),
        unknown_entries: base
            .map(|bundle| bundle.unknown_entries.clone())
            .unwrap_or_default(),
    })
}

fn empty_settings_file(settings: SimSettings) -> ScenarioSettingsFile {
    ScenarioSettingsFile {
        scenario: settings.scenario,
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
}

// ---------------------------------------------------------------------------
// Pellets
// ---------------------------------------------------------------------------

fn parse_pellets(
    value: &Value,
    zones: &[bibites_core::settings::ZoneSettings],
    settings: &SimSettings,
) -> Result<Vec<PelletRecord>> {
    let mut out = Vec::new();
    let groups = value
        .get("pellets")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::invalid_data("pellets.bb8scene has no pellets array"))?;
    for group in groups {
        let bucket = group
            .get("zone")
            .and_then(Value::as_str)
            .unwrap_or("free pellets");
        let owner_zone = zones
            .iter()
            .position(|zone| zone.name == bucket)
            .map(|index| index as u32);
        for pellet in group
            .get("pellets")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let amount = pellet
                .get("pellet")
                .and_then(|pellet| pellet.get("amount"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as f32;
            let material = match pellet
                .get("pellet")
                .and_then(|pellet| pellet.get("material"))
                .and_then(Value::as_str)
                .unwrap_or("Plant")
            {
                "Meat" => PelletMaterial::Meat,
                "Fat" => PelletMaterial::Fat,
                "Armor" => PelletMaterial::Armor,
                _ => PelletMaterial::Plant,
            };
            let pos = pellet
                .get("transform")
                .and_then(|transform| transform.get("position"))
                .and_then(Value::as_array)
                .map(|position| {
                    Vec2::new(
                        position.first().and_then(Value::as_f64).unwrap_or(0.0) as f32,
                        position.get(1).and_then(Value::as_f64).unwrap_or(0.0) as f32,
                    )
                })
                .unwrap_or(Vec2::ZERO);
            // `SaveSystem.LoadPellet` (cs:536-553): entry-level `matterDecay`,
            // legacy `meatRot`; the nested spelling is kept for saves written
            // by earlier RUSTibites builds.
            let decay = pellet
                .get("matterDecay")
                .or_else(|| pellet.get("meatRot"))
                .or_else(|| {
                    pellet
                        .get("pellet")
                        .and_then(|pellet| pellet.get("matterDecay"))
                });
            let (fresh_time_remaining, rot_amount) = match decay {
                Some(decay) => {
                    let time_alive = decay
                        .get("timeAlive")
                        .and_then(Value::as_f64)
                        .map_or(0.0, |value| value as f32);
                    let fresh = material_settings_fresh_time(settings, material);
                    (
                        if fresh > 0.0 {
                            Some(fresh - time_alive)
                        } else {
                            Some(-time_alive)
                        },
                        decay
                            .get("rotAmount")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0) as f32,
                    )
                }
                None => (None, 0.0),
            };
            out.push(PelletRecord {
                material,
                amount,
                pos,
                owner_zone,
                held: false,
                fresh_time_remaining,
                rot_amount,
            });
        }
    }
    Ok(out)
}

fn material_settings_fresh_time(settings: &SimSettings, material: PelletMaterial) -> f32 {
    let kind = match material {
        PelletMaterial::Plant => MaterialKind::Plant,
        PelletMaterial::Meat => MaterialKind::Meat,
        PelletMaterial::Fat => MaterialKind::Fat,
        PelletMaterial::Armor => MaterialKind::Armor,
    };
    settings.materials.get(kind).fresh_time
}

fn pellets_to_json(contents: &WorldContents) -> Value {
    let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
    for pellet in &contents.pellets {
        let bucket = pellet
            .owner_zone
            .and_then(|index| contents.zones.get(index as usize))
            .map(|zone| zone.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "free pellets".to_string());
        let mut entry = serde_json::json!({
            "transform": {
                "position": [f64::from(pellet.pos.x), f64::from(pellet.pos.y)],
                "rotation": 0.0,
                "scale": 1.0,
            },
            "rb2d": {
                "px": f64::from(pellet.pos.x),
                "py": f64::from(pellet.pos.y),
                "vx": 0.0,
                "vy": 0.0,
                "r": 0.0,
            },
            "pellet": pellet_entry_json(pellet, &contents.settings),
        });
        // `SaveSystem.SavePellet` (cs:396-407): `matterDecay` is a sibling of
        // `"pellet"`, not nested inside it.
        let decay = entry
            .get_mut("pellet")
            .and_then(Value::as_object_mut)
            .and_then(|object| object.remove("matterDecay"));
        if let (Some(decay), Some(object)) = (decay, entry.as_object_mut()) {
            object.insert("matterDecay".to_string(), decay);
        }
        match groups.iter_mut().find(|(name, _)| *name == bucket) {
            Some((_, entries)) => entries.push(entry),
            None => groups.push((bucket, vec![entry])),
        }
    }
    let groups: Vec<Value> = groups
        .into_iter()
        .map(|(zone, pellets)| serde_json::json!({"zone": zone, "pellets": pellets}))
        .collect();
    serde_json::json!({"pellets": groups})
}

fn pellet_entry_json(pellet: &PelletRecord, settings: &SimSettings) -> Value {
    let material_name = match pellet.material {
        PelletMaterial::Plant => "Plant",
        PelletMaterial::Meat => "Meat",
        PelletMaterial::Fat => "Fat",
        PelletMaterial::Armor => "Armor",
    };
    let mut object = Map::new();
    object.insert("material".to_string(), Value::from(material_name));
    object.insert("amount".to_string(), Value::from(f64::from(pellet.amount)));
    if let Some(remaining) = pellet.fresh_time_remaining {
        let fresh = material_settings_fresh_time(settings, pellet.material);
        let time_alive = fresh - remaining;
        object.insert(
            "matterDecay".to_string(),
            serde_json::json!({
                "timeAlive": f64::from(time_alive),
                "rotAmount": f64::from(pellet.rot_amount),
            }),
        );
    }
    Value::Object(object)
}

// ---------------------------------------------------------------------------
// Pheromones
// ---------------------------------------------------------------------------

fn parse_pheromones(value: &Value) -> Result<Vec<PheromoneRecord>> {
    let mut out = Vec::new();
    for spot in value
        .get("pheromones")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let pos = spot
            .get("transform")
            .and_then(|transform| transform.get("position"))
            .and_then(Value::as_array)
            .map(|position| {
                Vec2::new(
                    position.first().and_then(Value::as_f64).unwrap_or(0.0) as f32,
                    position.get(1).and_then(Value::as_f64).unwrap_or(0.0) as f32,
                )
            })
            .unwrap_or(Vec2::ZERO);
        let phero = spot.get("phero");
        let strength = [
            phero
                .and_then(|phero| phero.get("Rstrength"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as f32,
            phero
                .and_then(|phero| phero.get("Gstrength"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as f32,
            phero
                .and_then(|phero| phero.get("Bstrength"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as f32,
        ];
        out.push(PheromoneRecord {
            pos,
            heading: Vec2::ZERO,
            strength,
            dissipate: true,
            owner_tool: None,
        });
    }
    Ok(out)
}

fn pheromones_to_json(contents: &WorldContents) -> Value {
    let spots: Vec<Value> = contents
        .pheromones
        .iter()
        .map(|spot| {
            serde_json::json!({
                "transform": {
                    "position": [f64::from(spot.pos.x), f64::from(spot.pos.y)],
                    "rotation": 0.0,
                    "scale": 1.0,
                },
                "phero": {
                    "heading": {},
                    "Rstrength": f64::from(spot.strength[0]),
                    "Gstrength": f64::from(spot.strength[1]),
                    "Bstrength": f64::from(spot.strength[2]),
                    "nR": 0.0,
                    "nG": 0.0,
                    "nB": 0.0,
                },
            })
        })
        .collect();
    serde_json::json!({"pheromones": spots})
}

// ---------------------------------------------------------------------------
// Scene + tools
// ---------------------------------------------------------------------------

/// Maps a serialized zone id to the loaded zone list index (runtime ids are
/// list indexes in the rebuilt world). Prefers an exact zone_id match and
/// falls back to the raw value when it is a valid list index.
fn zone_id_for_save_id(zones: &[bibites_core::settings::ZoneSettings], raw: u64) -> Option<ZoneId> {
    let raw = raw as u32;
    let index = zones
        .iter()
        .position(|zone| zone.zone_id == Some(raw))
        .map(|index| index as u32)
        .or_else(|| (raw as usize).lt(&zones.len()).then_some(raw))?;
    Some(ZoneId::from_handle(Handle::new(index, 0)))
}

fn parse_rad_towers(
    scene: &Value,
    zones: &[bibites_core::settings::ZoneSettings],
) -> Result<Vec<RadTower>> {
    let mut towers = Vec::new();
    for tower in scene
        .get("radTowers")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let mut base = TowerBase::new(
            tower.get("id").and_then(Value::as_u64).unwrap_or(0),
            Vec2::new(
                tower.get("x").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                tower.get("y").and_then(Value::as_f64).unwrap_or(0.0) as f32,
            ),
        );
        if let Some(raw) = tower.get("attachedZone").and_then(Value::as_u64) {
            base.attached_zone = zone_id_for_save_id(zones, raw);
        }
        towers.push(RadTower {
            base,
            count_rate: tower.get("count").and_then(Value::as_f64).unwrap_or(0.001) as f32,
            intensity_rate: tower
                .get("intensity")
                .and_then(Value::as_f64)
                .unwrap_or(0.001) as f32,
            radius: tower.get("radius").and_then(Value::as_f64).unwrap_or(500.0) as f32,
            taper: tower.get("taper").and_then(Value::as_f64).unwrap_or(1.0) as f32,
            progress: 0.0,
        });
    }
    Ok(towers)
}

fn parse_phero_towers(
    scene: &Value,
    zones: &[bibites_core::settings::ZoneSettings],
) -> Result<Vec<PheroTower>> {
    let mut towers = Vec::new();
    for tower in scene
        .get("pheroTowers")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let mut base = TowerBase::new(
            tower.get("id").and_then(Value::as_u64).unwrap_or(0),
            Vec2::new(
                tower.get("x").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                tower.get("y").and_then(Value::as_f64).unwrap_or(0.0) as f32,
            ),
        );
        if let Some(raw) = tower.get("attachedZone").and_then(Value::as_u64) {
            base.attached_zone = zone_id_for_save_id(zones, raw);
        }
        towers.push(PheroTower {
            base,
            red: tower.get("r").and_then(Value::as_f64).unwrap_or(0.0) as f32,
            green: tower.get("g").and_then(Value::as_f64).unwrap_or(0.0) as f32,
            blue: tower.get("b").and_then(Value::as_f64).unwrap_or(0.0) as f32,
            heading: tower.get("dir").and_then(Value::as_f64).unwrap_or(0.0) as f32,
            spot: None,
        });
    }
    Ok(towers)
}

fn parse_color_killers(
    scene: &Value,
    zones: &[bibites_core::settings::ZoneSettings],
) -> Result<Vec<ColorKiller>> {
    let mut killers = Vec::new();
    for killer in scene
        .get("colorSelectors")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let pos = killer
            .get("transform")
            .and_then(|transform| transform.get("position"))
            .and_then(Value::as_array)
            .map(|position| {
                Vec2::new(
                    position.first().and_then(Value::as_f64).unwrap_or(0.0) as f32,
                    position.get(1).and_then(Value::as_f64).unwrap_or(0.0) as f32,
                )
            })
            .unwrap_or(Vec2::ZERO);
        let mut color = ColorKiller::new(0, pos);
        // `ColorKiller.LoadState` (cs:289-293): `followingZone` is the zone id.
        if let Some(raw) = killer.get("followingZone").and_then(Value::as_u64) {
            color.base.attached_zone = zone_id_for_save_id(zones, raw);
        }
        color.hue = killer.get("hue").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        color.tolerance = killer
            .get("tolerance")
            .and_then(Value::as_f64)
            .unwrap_or(0.05) as f32;
        color.body_or_eye = killer
            .get("bodyOrEye")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        color.any_or_most = killer
            .get("anyOrMost")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        color.radius = killer
            .get("radius")
            .and_then(Value::as_f64)
            .unwrap_or(1000.0) as f32;
        color.period = killer.get("period").and_then(Value::as_f64).unwrap_or(60.0) as f32;
        color.progress = killer
            .get("progress")
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as f32;
        killers.push(color);
    }
    Ok(killers)
}

fn rad_towers_to_json(contents: &WorldContents) -> Value {
    let towers: Vec<Value> = contents
        .rad_towers
        .iter()
        .map(|tower| {
            let mut object = Map::new();
            if let Some(zone) = tower.base.attached_zone {
                object.insert(
                    "attachedZone".to_string(),
                    Value::from(i64::from(zone.handle().slot())),
                );
            }
            object.insert("id".to_string(), Value::from(tower.base.save_id));
            object.insert("x".to_string(), Value::from(f64::from(tower.base.pos.x)));
            object.insert("y".to_string(), Value::from(f64::from(tower.base.pos.y)));
            object.insert(
                "count".to_string(),
                Value::from(f64::from(tower.count_rate)),
            );
            object.insert(
                "intensity".to_string(),
                Value::from(f64::from(tower.intensity_rate)),
            );
            object.insert("radius".to_string(), Value::from(f64::from(tower.radius)));
            object.insert("taper".to_string(), Value::from(f64::from(tower.taper)));
            Value::Object(object)
        })
        .collect();
    Value::Array(towers)
}

fn phero_towers_to_json(contents: &WorldContents) -> Value {
    let towers: Vec<Value> = contents
        .phero_towers
        .iter()
        .map(|tower| {
            let mut object = Map::new();
            if let Some(zone) = tower.base.attached_zone {
                object.insert(
                    "attachedZone".to_string(),
                    Value::from(i64::from(zone.handle().slot())),
                );
            }
            object.insert("id".to_string(), Value::from(tower.base.save_id));
            object.insert("x".to_string(), Value::from(f64::from(tower.base.pos.x)));
            object.insert("y".to_string(), Value::from(f64::from(tower.base.pos.y)));
            object.insert("r".to_string(), Value::from(f64::from(tower.red)));
            object.insert("g".to_string(), Value::from(f64::from(tower.green)));
            object.insert("b".to_string(), Value::from(f64::from(tower.blue)));
            object.insert("dir".to_string(), Value::from(f64::from(tower.heading)));
            Value::Object(object)
        })
        .collect();
    Value::Array(towers)
}

fn color_killers_to_json(contents: &WorldContents) -> Value {
    let killers: Vec<Value> = contents
        .color_killers
        .iter()
        .map(|killer| {
            let mut object = Map::new();
            if let Some(zone) = killer.base.attached_zone {
                object.insert(
                    "followingZone".to_string(),
                    Value::from(i64::from(zone.handle().slot())),
                );
            }
            object.insert(
                "transform".to_string(),
                serde_json::json!({
                    "position": [f64::from(killer.base.pos.x), f64::from(killer.base.pos.y)],
                    "rotation": 0.0,
                    "scale": 1.0,
                }),
            );
            object.insert("hue".to_string(), Value::from(f64::from(killer.hue)));
            object.insert(
                "tolerance".to_string(),
                Value::from(f64::from(killer.tolerance)),
            );
            object.insert("bodyOrEye".to_string(), Value::Bool(killer.body_or_eye));
            object.insert("anyOrMost".to_string(), Value::Bool(killer.any_or_most));
            object.insert("radius".to_string(), Value::from(f64::from(killer.radius)));
            object.insert("period".to_string(), Value::from(f64::from(killer.period)));
            object.insert(
                "progress".to_string(),
                Value::from(f64::from(killer.progress)),
            );
            Value::Object(object)
        })
        .collect();
    Value::Array(killers)
}

fn scene_to_json(contents: &WorldContents, base: Option<&Value>) -> Result<Value> {
    let mut object = match base {
        Some(Value::Object(object)) => object.clone(),
        Some(_) => {
            return Err(Error::invalid_data("base scene is not an object"));
        }
        None => Map::new(),
    };
    object.insert(
        "version".to_string(),
        Value::from(contents.scene.version.clone()),
    );
    object.insert(
        "nPellets".to_string(),
        Value::from(contents.pellets.len() as i64),
    );
    object.insert(
        "nBibites".to_string(),
        // `SaveSystem.cs:230`: bibites plus eggs.
        Value::from((contents.bibites.len() + contents.eggs.len()) as i64),
    );
    object.insert(
        "simulatedTime".to_string(),
        Value::from(contents.scene.simulated_time),
    );
    object.insert(
        "colorSelectors".to_string(),
        color_killers_to_json(contents),
    );
    object.insert("pheroTowers".to_string(), phero_towers_to_json(contents));
    object.insert("radTowers".to_string(), rad_towers_to_json(contents));
    if !contents.scene.name.is_empty() {
        object.insert("name".to_string(), Value::from(contents.scene.name.clone()));
    }
    if !contents.scene.tags.is_empty() {
        object.insert(
            "tags".to_string(),
            Value::Array(
                contents
                    .scene
                    .tags
                    .iter()
                    .cloned()
                    .map(Value::from)
                    .collect(),
            ),
        );
    }
    Ok(Value::Object(object))
}

// Re-exported helpers used by callers that build settings files directly.
/// Serializes a settings file for a save (used by the CLI save path).
pub fn serialize_save_settings_file(file: &ScenarioSettingsFile) -> Result<String> {
    file.to_json(SettingsMode::Save)
}

/// Resolves settings for a scenario with the I2 precedence helper.
#[must_use]
pub fn scenario_effective_settings(
    scenario: &ScenarioBundle,
    override_settings: Option<&SimSettings>,
) -> SimSettings {
    if let Some(settings) = override_settings {
        return settings.clone();
    }
    resolve_effective_settings(
        scenario.settings_file.scenario.clone(),
        scenario.settings_file.materials,
        None,
        None,
        scenario.settings_file.recommended.clone(),
    )
}

/// Species lineage accessor for callers that only have the raw bundle.
#[must_use]
pub fn lineage_of(bundle: &SaveBundle) -> Option<&LineageRecordIo> {
    bundle.species.as_ref()
}

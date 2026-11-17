//! Integration tests for bibites-io phase I4a (`SaveBundle` ↔ core `WorldContents`).

// NOTE: non-Default real files used here are deliberate format/version coverage,
// not the canonical test baseline (see docs/TESTING_POLICY.md); tests skip if
// testdata/ is absent.
use std::path::{Path, PathBuf};

use bibites_core::world::World;
use bibites_io::{
    load_scenario_world_contents, load_world_contents, save_world_contents,
    world_contents_from_save,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bundles")
}

fn real_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/real")
}

fn temp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("rustibites_io_i4a");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn assert_bibite_subset(
    first: &bibites_core::body::BibiteSave,
    second: &bibites_core::body::BibiteSave,
) {
    assert_eq!(first.pos, second.pos);
    assert_eq!(first.vel, second.vel);
    assert_eq!(first.rot, second.rot);
    assert_eq!(first.genome, second.genome);
    assert_eq!(first.meta, second.meta);
    assert_eq!(first.brain, second.brain);
    assert_eq!(first.energy, second.energy);
    assert_eq!(first.health, second.health);
    assert_eq!(first.d2_size, second.d2_size);
    assert_eq!(first.growth, second.growth);
    assert_eq!(first.stomach, second.stomach);
    assert_eq!(first.n_stomach, second.n_stomach);
    assert_eq!(first.egg_progress, second.egg_progress);
    assert_eq!(first.total_travel, second.total_travel);
}

/// Where two byte strings differ, in a few words (a failing assertion on whole blocks would print thousands of numbers).
fn first_difference(first: &[u8], second: &[u8]) -> String {
    match first.iter().zip(second).position(|(a, b)| a != b) {
        Some(at) => format!(
            "lengths {} and {}, first difference at byte {at} ({} against {})",
            first.len(),
            second.len(),
            first[at],
            second[at]
        ),
        None if first.len() == second.len() => "equal".to_owned(),
        None => format!(
            "lengths {} and {}, one is the start of the other",
            first.len(),
            second.len()
        ),
    }
}

/// The elements of a `DataLogger` block: the 8 bytes of progress and minutes, then each stream behind its length.
fn logger_elements(block: &[u8]) -> Vec<&[u8]> {
    let mut elements = Vec::new();
    let mut at = 8;
    while at + 4 <= block.len() {
        let length = i32::from_le_bytes(block[at..at + 4].try_into().unwrap()) as usize;
        elements.push(&block[at + 4..at + 4 + length]);
        at += 4 + length;
    }
    elements
}

/// A tracker block read and written again by the core. The game gives each estimator a slot as large as its summary was before
/// it compressed itself (the compressed summary fills the start of the slot, the rest stays zero), so blocks of the same content can
/// differ in length.
fn canonical_tracker(block: &[u8]) -> Vec<u8> {
    let mut logger = bibites_core::datalog::DataLogger::new();
    let (births, deaths) = logger.load_tracker_bin(block).expect("a tracker block");
    logger.save_tracker_bin(births, deaths)
}

/// The tracker block in a few words: the pending births and deaths, the roll counter and the size of each estimator.
fn tracker_summary(block: &[u8]) -> String {
    let int = |at: usize| i32::from_le_bytes(block[at..at + 4].try_into().unwrap());
    let mut text = format!(
        "births {}, deaths {}, roll counter {}, estimators",
        int(0),
        int(4),
        int(12)
    );
    let mut at = 16;
    while at + 4 <= block.len() {
        let length = int(at) as usize;
        let n = int(at + 4);
        let tuples = i16::from_le_bytes(block[at + 8..at + 10].try_into().unwrap());
        text += &format!(" [{length} bytes, n {n}, {tuples} tuples]");
        at += 4 + length;
    }
    text
}

/// Whether two `data.bin` files hold the same history: every block is the same bytes, except that the tracker block may differ in the
/// padding of its estimators' slots (see [`canonical_tracker`]); a file that is not a list of blocks is compared as it is.
fn same_history(first: &[u8], second: &[u8]) -> bool {
    use bibites_core::data_bin::{TRACKER_BLOCK, split_blocks};
    let (Some(a), Some(b)) = (split_blocks(first), split_blocks(second)) else {
        return first == second;
    };
    a.len() == b.len()
        && a.iter().zip(&b).enumerate().all(|(index, (x, y))| {
            if index == TRACKER_BLOCK && x.len() >= 12 && y.len() >= 12 {
                canonical_tracker(x) == canonical_tracker(y)
            } else {
                x == y
            }
        })
}

#[test]
fn synthetic_save_to_contents_round_trips() {
    let path = fixtures().join("save_min.zip");
    let contents = load_world_contents(&path, None).unwrap();
    assert_eq!(contents.zones.len(), 1);
    assert!(contents.zone_groups.is_empty());
    assert_eq!(contents.bibites.len(), 1);
    assert_eq!(contents.eggs.len(), 1);
    assert!(contents.settings.sim_tps > 0);
    assert_eq!(contents.data_bin, vec![1, 2, 3, 4, 5]);
    assert!(contents.pellets.is_empty());

    let out = temp_path("synthetic_world.zip");
    save_world_contents(&out, &contents).unwrap();
    let reloaded = load_world_contents(&out, None).unwrap();
    assert_eq!(reloaded.settings, contents.settings);
    assert_eq!(reloaded.scene.version, contents.scene.version);
    assert_eq!(reloaded.scene.simulated_time, contents.scene.simulated_time);
    assert_eq!(reloaded.zones, contents.zones);
    assert_eq!(reloaded.bibites.len(), contents.bibites.len());
    assert_eq!(reloaded.eggs.len(), contents.eggs.len());
    assert_eq!(reloaded.data_bin, contents.data_bin);
    let _ = std::fs::remove_file(out);
}

#[test]
fn real_lol_save_to_world_and_back() {
    let path = real_dir().join("saves/lol.zip");
    if !path.exists() {
        eprintln!("skipping: {} absent", path.display());
        return;
    }
    let contents = load_world_contents(&path, None).unwrap();
    assert_eq!(contents.bibites.len(), 35);
    assert_eq!(contents.eggs.len(), 4);
    assert!(contents.settings.sim_tps > 0);
    assert!(!contents.zones.is_empty());
    assert!(!contents.pellets.is_empty());
    assert!(!contents.pheromones.is_empty());
    assert!(!contents.data_bin.is_empty());
    assert!(contents.species.is_some());

    // Core conversion itself works for a real save (I4 boundary).
    let world = World::from_contents(contents.clone()).unwrap();
    assert_eq!(world.bibites.len(), 35);
    assert_eq!(world.eggs.len(), 4);
    // The species logs of the game's `data.bin` are in the registry: the lineage tree reads where each
    // species is present.
    let registry = world.species_registry();
    assert_eq!(registry.records().len(), 26);
    let with_history = registry
        .records()
        .iter()
        .filter(|record| record.stats.log.n_points_present() > 0)
        .count();
    assert!(with_history >= 20, "{with_history} species have a history");
    assert!(
        registry
            .records()
            .iter()
            .any(|record| record.stats.log.alive()),
        "some species are present in the newest point"
    );
    assert!(
        registry
            .records()
            .iter()
            .any(|record| record.stats.total_index_of_apparition() > 12),
        "the long lived ones reach the compressed scales"
    );
    let world_contents = world.to_contents();
    assert!(
        same_history(&world_contents.data_bin, &contents.data_bin),
        "the history is written back as it was loaded"
    );
    assert_eq!(world_contents.bibites.len(), contents.bibites.len());
    assert_eq!(world_contents.eggs.len(), contents.eggs.len());

    let out = temp_path("lol_world.zip");
    save_world_contents(&out, &contents).unwrap();
    let reloaded = load_world_contents(&out, None).unwrap();
    assert_eq!(reloaded.settings, contents.settings);
    assert_eq!(reloaded.scene.version, contents.scene.version);
    assert_eq!(reloaded.scene.simulated_time, contents.scene.simulated_time);
    assert_eq!(reloaded.zones.len(), contents.zones.len());
    assert_eq!(reloaded.settings_changers, contents.settings_changers);
    assert_eq!(reloaded.pellets, contents.pellets);
    assert_eq!(reloaded.pheromones, contents.pheromones);
    assert_eq!(reloaded.data_bin, contents.data_bin);
    assert_eq!(reloaded.rad_towers.len(), contents.rad_towers.len());
    assert_eq!(reloaded.phero_towers.len(), contents.phero_towers.len());
    assert_eq!(reloaded.color_killers.len(), contents.color_killers.len());
    assert_eq!(reloaded.bibites.len(), contents.bibites.len());
    assert_eq!(reloaded.eggs.len(), contents.eggs.len());
    for (first, second) in reloaded.bibites.iter().zip(contents.bibites.iter()) {
        assert_bibite_subset(first, second);
    }
    for (first, second) in reloaded.eggs.iter().zip(contents.eggs.iter()) {
        assert_eq!(first.pos, second.pos);
        assert_eq!(first.genome, second.genome);
        assert_eq!(first.brain, second.brain);
        assert_eq!(first.energy, second.energy);
        assert_eq!(first.hatch_progress, second.hatch_progress);
        assert_eq!(first.hatch_time, second.hatch_time);
        assert_eq!(first.total_required_energy, second.total_required_energy);
    }
    assert_eq!(reloaded.species.is_some(), contents.species.is_some());
    let _ = std::fs::remove_file(out);
    eprintln!(
        "exercised lol.zip: {} bibites, {} eggs, {} pellets, {} pheromones, {} zones",
        contents.bibites.len(),
        contents.eggs.len(),
        contents.pellets.len(),
        contents.pheromones.len(),
        contents.zones.len()
    );
}

#[test]
fn real_lol_save_parent_links_survive_the_world_and_the_file() {
    let path = real_dir().join("saves/lol.zip");
    if !path.exists() {
        eprintln!("skipping: {} absent", path.display());
        return;
    }
    let contents = load_world_contents(&path, None).unwrap();
    // `SaveSystem.LoadWorld`: a first parent is looked up among the bibites; 29 of the 35 bibites and all 4 eggs have one there,
    // and none of them has a second parent that was found.
    let linked =
        |slots: &[[Option<u32>; 2]]| slots.iter().filter(|slots| slots[0].is_some()).count();
    assert_eq!(contents.bibite_parents.len(), 35);
    assert_eq!(contents.egg_parents.len(), 4);
    assert_eq!(linked(&contents.bibite_parents), 29);
    assert_eq!(linked(&contents.egg_parents), 4);
    assert!(
        contents
            .bibite_parents
            .iter()
            .chain(&contents.egg_parents)
            .flatten()
            .flatten()
            .all(|slot| (*slot as usize) < contents.bibites.len())
    );

    // The world gives them the ids the bibites were made with, and the contents of that world say the same again.
    let world = World::from_contents(contents.clone()).unwrap();
    let ids: Vec<_> = world.bibites.ids().collect();
    for (index, slots) in contents.bibite_parents.iter().enumerate() {
        let meta = &world.bibites.get(ids[index]).unwrap().meta;
        assert_eq!(meta.parent1, slots[0].map(|slot| ids[slot as usize]));
    }
    let egg_ids: Vec<_> = world.eggs.ids().collect();
    for (index, slots) in contents.egg_parents.iter().enumerate() {
        let meta = &world.eggs.get(egg_ids[index]).unwrap().meta;
        assert_eq!(meta.parent1, slots[0].map(|slot| ids[slot as usize]));
    }
    let back = world.to_contents();
    assert_eq!(back.bibite_parents, contents.bibite_parents);
    assert_eq!(back.egg_parents, contents.egg_parents);

    // And through a file: the raw parent ids are written, and each bibite's children are the living bibites and the eggs it is the
    // first parent of.
    let out = temp_path("lol_links.zip");
    save_world_contents(&out, &back).unwrap();
    let again = load_world_contents(&out, None).unwrap();
    assert_eq!(again.bibite_parents, contents.bibite_parents);
    assert_eq!(again.egg_parents, contents.egg_parents);
    let bundle = bibites_io::load_save_bundle(&out).unwrap();
    let raw_children = |slot: usize| -> usize {
        let living = back
            .bibite_parents
            .iter()
            .zip(&back.bibites)
            .filter(|(slots, bibite)| slots[0] == Some(slot as u32) && !bibite.dead)
            .count();
        let eggs = back
            .egg_parents
            .iter()
            .filter(|slots| slots[0] == Some(slot as u32))
            .count();
        living + eggs
    };
    for slot in 0..bundle.bibites.len() {
        assert_eq!(
            bundle.bibite_children[slot].len(),
            raw_children(slot),
            "bibite {slot}"
        );
    }
    // The ids are unique, shared between the bibites and the eggs, and never 0 (0 means none).
    let all: std::collections::BTreeSet<i32> = bundle
        .bibite_ids
        .iter()
        .chain(&bundle.egg_ids)
        .copied()
        .collect();
    assert_eq!(all.len(), bundle.bibite_ids.len() + bundle.egg_ids.len());
    assert!(!all.contains(&0));
    let _ = std::fs::remove_file(out);
}

#[test]
fn real_graph_history_loads_and_saves_as_the_game_wrote_it() {
    use bibites_core::data_bin::{
        LOGGER_BLOCK, SPECIES_BLOCK, TRACKER_BLOCK, ZONE_BLOCK, has_logger_history, split_blocks,
    };
    // `RUSTIBITES_REAL_SAVES` points at another folder of saves (read only).
    let folder = std::env::var_os("RUSTIBITES_REAL_SAVES")
        .map_or_else(|| real_dir().join("saves"), PathBuf::from);
    let Ok(saves) = std::fs::read_dir(&folder) else {
        eprintln!("skipping: {} absent", folder.display());
        return;
    };
    let mut checked = 0;
    for path in saves
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "zip"))
    {
        let Ok(contents) = load_world_contents(&path, None) else {
            continue;
        };
        if !has_logger_history(&contents.scene.version) {
            continue;
        }
        let Some(blocks) = split_blocks(&contents.data_bin) else {
            continue;
        };
        if blocks.len() < 2 {
            continue;
        }
        let world = World::from_contents(contents.clone()).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // The history the game wrote is in the world's logger.
        let points: Vec<Vec<usize>> = world
            .data_logger
            .groups
            .iter()
            .map(|group| {
                (0..group.history.n_scales())
                    .map(|scale| group.history.n_data_at_scale(scale))
                    .collect()
            })
            .collect();
        eprintln!("{name}: points per scale of each group: {points:?}");
        let genes = &world.data_logger.genes;
        let genes_per_scale: Vec<usize> = (0..genes.n_scales())
            .map(|scale| genes.n_data_at_scale(scale))
            .collect();
        eprintln!(
            "{name}: genes stream: {} points, per scale {genes_per_scale:?}",
            genes.n_data()
        );
        if name == "lol.zip" {
            // The hourly gene distributions of the real save: 20 points, the first two scales' 6 and the third's 8.
            assert_eq!(genes_per_scale, [6, 6, 8, 0, 0, 0, 0]);
            assert_eq!(genes.n_data(), 20);
            let newest = genes.get(0).expect("the newest point");
            let sum: u32 = newest[0].data.iter().map(|n| u32::from(*n)).sum();
            assert_eq!(sum, 45, "45 bibites and eggs when the last hour was logged");
        }
        // Saved again, every block is the one the game wrote, down to the byte.
        let saved = world.to_contents().data_bin;
        let again = split_blocks(&saved).expect("a data.bin");
        let mut differences = Vec::new();
        for (label, index) in [
            ("logger", LOGGER_BLOCK),
            ("tracker", TRACKER_BLOCK),
            ("species", SPECIES_BLOCK),
            ("zones", ZONE_BLOCK),
        ] {
            let (Some(original), Some(rewritten)) = (blocks.get(index), again.get(index)) else {
                continue;
            };
            let same = if index == TRACKER_BLOCK {
                canonical_tracker(original) == canonical_tracker(rewritten)
            } else {
                original == rewritten
            };
            if !same {
                let mut note = format!("{label}: {}", first_difference(original, rewritten));
                if index == TRACKER_BLOCK {
                    note += &format!(
                        "
  game:  {}
  saved: {}",
                        tracker_summary(original),
                        tracker_summary(rewritten)
                    );
                }
                if index == LOGGER_BLOCK {
                    let (a, b) = (logger_elements(original), logger_elements(rewritten));
                    for (element, (x, y)) in a.iter().zip(&b).enumerate() {
                        if x != y {
                            note += &format!("; stream {element}: {}", first_difference(x, y));
                        }
                    }
                }
                differences.push(note);
            }
        }
        assert!(differences.is_empty(), "{name}: {differences:?}");
        checked += 1;
    }
    eprintln!("{checked} saves with a Graphs history checked");
}

#[test]
fn real_zone_logs_load_and_encode_as_the_game_wrote_them() {
    // `RUSTIBITES_REAL_SAVES` points at another folder of saves (read only).
    let folder = std::env::var_os("RUSTIBITES_REAL_SAVES")
        .map_or_else(|| real_dir().join("saves"), PathBuf::from);
    let Ok(saves) = std::fs::read_dir(&folder) else {
        eprintln!("skipping: {} absent", folder.display());
        return;
    };
    for path in saves
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "zip"))
    {
        let Ok(contents) = load_world_contents(&path, None) else {
            eprintln!("{}: not a save this build loads", path.display());
            continue;
        };
        let world = World::from_contents(contents.clone()).unwrap();
        let Some(blocks) = bibites_core::data_bin::split_blocks(&contents.data_bin) else {
            continue;
        };
        let Some(block) = blocks.get(bibites_core::data_bin::ZONE_BLOCK) else {
            continue;
        };
        let entries = bibites_core::data_bin::zone_entries(block);
        let mut with_history = 0;
        for (id, bytes) in &entries {
            let zone = world
                .zones
                .zones
                .iter()
                .find(|zone| zone.config.runtime_zone_id() as i32 == *id)
                .unwrap_or_else(|| panic!("{}: zone {id} is in the world", path.display()));
            // The log reads and writes as the game's bytes: the encoder is the game's format.
            assert_eq!(
                &zone.log.save_state_bin(),
                bytes,
                "{}: zone {id}",
                path.display()
            );
            if zone.log.n_points_present() > 0 {
                with_history += 1;
            }
        }
        eprintln!(
            "{}: {} zone logs, {with_history} with a history",
            path.display(),
            entries.len()
        );
        assert!(
            same_history(&world.to_contents().data_bin, &contents.data_bin),
            "{}: the history is written back as it was loaded",
            path.display()
        );
    }
}

#[test]
fn real_ring_base_save_to_contents() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/parity/bases/ring.base.zip");
    if !path.exists() {
        eprintln!("skipping: {} absent", path.display());
        return;
    }
    let contents = load_world_contents(&path, None).unwrap();
    assert_eq!(contents.bibites.len(), 1);
    assert!(contents.data_bin.len() == 339 || contents.data_bin.len() == 340);
    let out = temp_path("ring_base_world.zip");
    save_world_contents(&out, &contents).unwrap();
    let reloaded = load_world_contents(&out, None).unwrap();
    assert_eq!(reloaded.bibites.len(), 1);
    assert_eq!(reloaded.data_bin, contents.data_bin);
    assert_eq!(reloaded.pellets, contents.pellets);
    assert_eq!(reloaded.scene.simulated_time, contents.scene.simulated_time);
    let _ = std::fs::remove_file(out);
}

#[test]
fn zone_groups_keep_their_zones_through_a_save_file() {
    use bibites_core::settings::{ZoneGroupSettings, ZoneSettings};
    use bibites_core::world::ScenarioData;
    let mut scenario = ScenarioData::default();
    let mut explicit = ZoneSettings::game_defaults();
    explicit.name = "World".to_owned();
    scenario.zones.push(explicit);
    for (index, count) in [3, 2].into_iter().enumerate() {
        let mut group = ZoneGroupSettings::game_defaults();
        group.name = format!("Pockets{index}");
        group.zone_count = count;
        group.template.fertility = 3.0 + index as f32;
        scenario.zone_groups.push(group);
    }
    let mut world =
        World::new(bibites_core::settings::SimSettings::game_defaults(), scenario, 11).unwrap();
    world
        .edit_zone_group(1, |group| group.name = "Lakes".to_owned())
        .unwrap();
    let mut contents = world.to_contents();
    assert_eq!(contents.zones.len(), 1);
    assert_eq!(contents.zone_groups.len(), 2);
    assert_eq!(contents.all_zones().len(), 6);
    // (A file needs the game version it was made with.)
    contents.scene.version = load_world_contents(&fixtures().join("save_min.zip"), None)
        .unwrap()
        .scene
        .version;

    let out = temp_path("zone_groups_world.zip");
    save_world_contents(&out, &contents).unwrap();
    let reloaded = load_world_contents(&out, None).unwrap();
    assert_eq!(reloaded.zones.len(), 1);
    assert_eq!(reloaded.zones[0].name, "World");
    assert_eq!(reloaded.zone_groups.len(), 2);
    assert_eq!(reloaded.zone_groups[1].name, "Lakes");
    assert_eq!(reloaded.zone_groups[1].template.fertility, 4.0);
    assert_eq!(reloaded.zone_groups[0].zones.len(), 3);
    assert_eq!(reloaded.zone_groups[1].zones.len(), 2);
    assert_eq!(reloaded.zone_groups[1].zones[1].name, "Lakes_1");
    for (saved, loaded) in contents.zone_groups.iter().zip(&reloaded.zone_groups) {
        for (a, b) in saved.zones.iter().zip(&loaded.zones) {
            assert_eq!((a.pos_x, a.pos_y), (b.pos_x, b.pos_y));
        }
    }
    // The world that comes back has the groups: the editor can edit them, the zones are the saved ones.
    let again = World::from_contents(reloaded).unwrap();
    assert_eq!(again.zone_groups.len(), 2);
    assert_eq!(again.zones.zones.len(), 6);
    assert_eq!(again.zones.explicit_count(), 1);
    assert_eq!(again.zones.group_span(1), (4, 2));
    let _ = std::fs::remove_file(out);
}

#[test]
fn scenario_to_contents_shell() {
    let path = real_dir().join("Ring.zip");
    if !path.exists() {
        eprintln!("skipping: {} absent", path.display());
        return;
    }
    let contents = load_scenario_world_contents(&path, None).unwrap();
    assert_eq!(contents.scene.name, "Ring");
    assert_eq!(contents.settings.independents.simulation_size, 50_000.0);
    assert!(!contents.zones.is_empty());
    assert!(!contents.scene.bibite_spawns.is_empty());
    assert!(contents.bibites.is_empty());
    assert!(contents.pellets.is_empty());
    assert!(contents.data_bin.is_empty());
}

#[test]
fn settings_override_is_applied() {
    let path = fixtures().join("save_min.zip");
    let mut settings = bibites_core::settings::SimSettings::game_defaults();
    settings.independents.simulation_size = 5_000.0;
    let contents = load_world_contents(&path, Some(&settings)).unwrap();
    assert_eq!(contents.settings.independents.simulation_size, 5_000.0);
}

#[test]
fn errors_are_rejected() {
    assert!(load_world_contents(fixtures().join("bad.zip"), None).is_err());
    assert!(load_world_contents(fixtures().join("missing_info.zip"), None).is_err());
    let _ = world_contents_from_save;
}

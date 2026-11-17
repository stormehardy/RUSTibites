//! Integration tests for bibites-io phase I3 (scenario and save `.zip` bundles).

// NOTE: non-Default real files used here are deliberate format/version coverage,
// not the canonical test baseline (see docs/TESTING_POLICY.md); tests skip if
// testdata/ is absent.
use std::path::{Path, PathBuf};

use bibites_io::{
    load_save_bundle, load_scenario_bundle, write_save_bundle, write_scenario_bundle,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bundles")
}

fn real_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/real")
}

fn temp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("rustibites_io_i3");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn scenario_fixture_round_trips() {
    let bundle = load_scenario_bundle(fixtures().join("scenario_min.zip")).unwrap();
    assert_eq!(bundle.info.name, "Fixture Scenario");
    assert_eq!(bundle.settings.independents.simulation_size, 2000.0);
    assert_eq!(bundle.settings.sim_tps, 30);
    assert_eq!(bundle.scenario.name, "Fixture Scenario");
    assert_eq!(bundle.scenario.zones.len(), 1);
    assert_eq!(bundle.scenario.bibite_spawns.len(), 1);
    assert_eq!(bundle.templates.len(), 1);
    assert_eq!(bundle.templates[0].0, "Basic.bb8template");
    assert!(bundle.img_png.is_some());

    let path = temp_path("scenario_round_trip.zip");
    write_scenario_bundle(&bundle, &path).unwrap();
    let reloaded = load_scenario_bundle(&path).unwrap();
    assert_eq!(reloaded.info, bundle.info);
    assert_eq!(reloaded.settings, bundle.settings);
    assert_eq!(reloaded.scenario.name, bundle.scenario.name);
    assert_eq!(reloaded.scenario.zones.len(), bundle.scenario.zones.len());
    assert_eq!(
        reloaded.scenario.bibite_spawns.len(),
        bundle.scenario.bibite_spawns.len()
    );
    assert_eq!(reloaded.templates.len(), bundle.templates.len());
    let _ = std::fs::remove_file(path);
}

#[test]
fn save_fixture_round_trips() {
    let bundle = load_save_bundle(fixtures().join("save_min.zip")).unwrap();
    assert_eq!(bundle.version.to_string(), "0.6.3.1");
    assert_eq!(bundle.bibites.len(), 1);
    assert_eq!(bundle.eggs.len(), 1);
    assert_eq!(bundle.data_bin.as_deref(), Some(&[1, 2, 3, 4, 5][..]));
    assert_eq!(
        bundle
            .scene
            .get("simulatedTime")
            .and_then(|value| value.as_f64()),
        Some(12.5)
    );
    assert!(bundle.species.is_some());

    let path = temp_path("save_round_trip.zip");
    write_save_bundle(&bundle, &path).unwrap();
    let reloaded = load_save_bundle(&path).unwrap();
    assert_eq!(reloaded.version, bundle.version);
    assert!(reloaded.scene == bundle.scene, "scene differs");
    let pellets_a = serde_json::to_string(bundle.pellets.as_ref().unwrap()).unwrap();
    let pellets_b = serde_json::to_string(reloaded.pellets.as_ref().unwrap()).unwrap();
    if pellets_a != pellets_b {
        let index = pellets_a
            .bytes()
            .zip(pellets_b.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        let start = index.saturating_sub(60);
        let a = &pellets_a[start..(index + 60).min(pellets_a.len())];
        let b = &pellets_b[start..(index + 60).min(pellets_b.len())];
        panic!(
            "pellets differ (len {} vs {}) at {index}:\nA {a}\nB {b}",
            pellets_a.len(),
            pellets_b.len()
        );
    }
    assert!(
        reloaded.pheromones == bundle.pheromones,
        "pheromones differ"
    );
    assert_eq!(reloaded.data_bin, bundle.data_bin);
    assert_eq!(reloaded.bibites.len(), bundle.bibites.len());
    assert_eq!(reloaded.eggs.len(), bundle.eggs.len());
    let (first, second) = (&bundle.bibites[0], &reloaded.bibites[0]);
    assert_eq!(first.genome, second.genome);
    assert_eq!(first.brain, second.brain);
    assert_eq!(first.energy, second.energy);
    assert_eq!(first.health, second.health);
    let _ = std::fs::remove_file(path);
}

#[test]
fn real_scenarios_if_present() {
    let mut exercised = 0;
    for name in ["Ring.zip", "Default.zip", "RadTower.zip"] {
        let path = real_dir().join(name);
        if !path.exists() {
            eprintln!("skipping real scenario {} (absent)", path.display());
            continue;
        }
        let bundle = load_scenario_bundle(&path).unwrap();
        assert_eq!(
            bundle.scenario.name,
            path.file_stem().unwrap().to_str().unwrap()
        );
        assert!(!bundle.scenario.zones.is_empty(), "{name}: zones");
        let out = temp_path(&format!("{name}.round_trip.zip"));
        write_scenario_bundle(&bundle, &out).unwrap();
        let reloaded = load_scenario_bundle(&out).unwrap();
        assert_eq!(reloaded.settings, bundle.settings, "{name}: settings");
        assert_eq!(reloaded.scenario.name, bundle.scenario.name, "{name}: name");
        assert_eq!(
            reloaded.scenario.zones.len(),
            bundle.scenario.zones.len(),
            "{name}: zone count"
        );
        assert_eq!(
            reloaded.scenario.bibite_spawns.len(),
            bundle.scenario.bibite_spawns.len(),
            "{name}: spawn count"
        );
        assert_eq!(reloaded.info, bundle.info, "{name}: info");
        let _ = std::fs::remove_file(out);
        exercised += 1;
        eprintln!("exercised real scenario {name}");
    }
    if exercised == 0 {
        eprintln!("skipping real scenario tests: no scenario zips present");
    }
}

#[test]
fn real_save_if_present() {
    let candidates = [
        real_dir().join("saves/lol.zip"),
        real_dir().join("saves/ozymandias .zip"),
        real_dir().join("small_save.zip"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/parity/bases/ring.base.zip"),
    ];
    let Some(path) = candidates.iter().find(|path| path.exists()) else {
        eprintln!("skipping real save test: no save zip present");
        return;
    };
    let bundle = load_save_bundle(path).unwrap();
    assert!(
        !bundle.bibites.is_empty(),
        "real save should contain bibites"
    );
    assert!(bundle.data_bin.is_some());

    // Full persisted-field coverage: every BibiteSave/EggSave field that the
    // game serializes (or derives on load) must survive load -> save -> load.
    for bibite in &bundle.bibites {
        assert!(bibite.growth >= 0.0);
        if bibite.d2_size > 0.0 {
            assert!(bibite.growth > 0.0, "growth must be derived from d2Size");
        }
    }
    for egg in &bundle.eggs {
        assert!(egg.hatch_time > 0.0);
        assert!(egg.total_required_energy > 0.0);
        assert!(egg.size_gene > 0.0);
    }

    let out = temp_path("real_save_round_trip.zip");
    write_save_bundle(&bundle, &out).unwrap();
    let reloaded = load_save_bundle(&out).unwrap();
    assert_eq!(reloaded.version, bundle.version);
    assert!(reloaded.scene == bundle.scene, "scene differs");
    let pellets_a = serde_json::to_string(bundle.pellets.as_ref().unwrap()).unwrap();
    let pellets_b = serde_json::to_string(reloaded.pellets.as_ref().unwrap()).unwrap();
    if pellets_a != pellets_b {
        let index = pellets_a
            .bytes()
            .zip(pellets_b.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        let start = index.saturating_sub(60);
        let a = &pellets_a[start..(index + 60).min(pellets_a.len())];
        let b = &pellets_b[start..(index + 60).min(pellets_b.len())];
        panic!(
            "pellets differ (len {} vs {}) at {index}:\nA {a}\nB {b}",
            pellets_a.len(),
            pellets_b.len()
        );
    }
    assert!(
        reloaded.pheromones == bundle.pheromones,
        "pheromones differ"
    );
    assert_eq!(reloaded.data_bin, bundle.data_bin);
    assert_eq!(reloaded.bibites.len(), bundle.bibites.len());
    assert_eq!(reloaded.eggs.len(), bundle.eggs.len());
    assert_eq!(reloaded.bibite_ids, bundle.bibite_ids);
    assert_eq!(reloaded.bibite_children, bundle.bibite_children);
    assert_eq!(reloaded.bibite_parents, bundle.bibite_parents);
    assert_eq!(reloaded.egg_ids, bundle.egg_ids);
    for (index, (first, second)) in bundle
        .bibites
        .iter()
        .zip(reloaded.bibites.iter())
        .enumerate()
    {
        assert_eq!(first.pos, second.pos, "bibite {index} pos");
        assert_eq!(first.vel, second.vel, "bibite {index} vel");
        assert_eq!(first.rot, second.rot, "bibite {index} rot");
        assert_eq!(first.genome, second.genome, "bibite {index} genome");
        assert_eq!(first.meta, second.meta, "bibite {index} meta");
        assert_eq!(first.brain, second.brain, "bibite {index} brain");
        assert_eq!(first.energy, second.energy, "bibite {index} energy");
        assert_eq!(first.health, second.health, "bibite {index} health");
        assert_eq!(first.d2_size, second.d2_size, "bibite {index} d2Size");
        assert_eq!(first.growth, second.growth, "bibite {index} growth");
        assert_eq!(first.dying, second.dying);
        assert_eq!(first.dead, second.dead);
        assert_eq!(first.attacked_dmg, second.attacked_dmg);
        assert_eq!(first.times_attacked, second.times_attacked);
        assert_eq!(first.total_damage_suffered, second.total_damage_suffered);
        assert_eq!(first.brain_ticks, second.brain_ticks);
        assert_eq!(first.vision_lookup, second.vision_lookup);
        assert_eq!(first.vision_sensing, second.vision_sensing);
        assert_eq!(first.corpse_energy_offset, second.corpse_energy_offset);
        assert_eq!(first.n_stomach, second.n_stomach);
        for slot in 0..first.n_stomach as usize {
            assert_eq!(first.stomach[slot].material, second.stomach[slot].material);
            assert_eq!(first.stomach[slot].amount, second.stomach[slot].amount);
            assert_eq!(
                first.stomach[slot].average_chunk_amount,
                second.stomach[slot].average_chunk_amount
            );
        }
        assert_eq!(first.bite_progress, second.bite_progress);
        assert_eq!(first.attacked_last_frame, second.attacked_last_frame);
        assert_eq!(first.bibites_bitten, second.bibites_bitten);
        assert_eq!(first.total_damage_dealt, second.total_damage_dealt);
        assert_eq!(first.total_murders, second.total_murders);
        assert_eq!(first.murdered_area, second.murdered_area);
        assert_eq!(first.egg_progress, second.egg_progress);
        assert_eq!(first.n_eggs_laid, second.n_eggs_laid);
        assert_eq!(first.fat_amount, second.fat_amount);
        assert_eq!(first.phero_progress, second.phero_progress);
        assert_eq!(first.tic, second.tic);
        assert_eq!(first.tic_progress, second.tic_progress);
        assert_eq!(first.time_alive, second.time_alive);
        assert_eq!(first.chrono_time, second.chrono_time);
        assert_eq!(first.total_travel, second.total_travel);
    }
    for (index, (first, second)) in bundle.eggs.iter().zip(reloaded.eggs.iter()).enumerate() {
        assert_eq!(first.pos, second.pos, "egg {index} pos");
        assert_eq!(first.vel, second.vel, "egg {index} vel");
        assert_eq!(first.rot, second.rot, "egg {index} rot");
        assert_eq!(first.genome, second.genome, "egg {index} genome");
        assert_eq!(first.meta, second.meta, "egg {index} meta");
        assert_eq!(first.brain, second.brain, "egg {index} brain");
        assert_eq!(first.energy, second.energy, "egg {index} energy");
        assert_eq!(first.hatch_progress, second.hatch_progress);
        assert_eq!(first.hatch_time, second.hatch_time);
        assert_eq!(first.total_required_energy, second.total_required_energy);
        assert_eq!(first.is_hatching, second.is_hatching);
        assert_eq!(first.aborting, second.aborting);
        assert_eq!(first.birth_orientation, second.birth_orientation);
        assert_eq!(first.size_gene, second.size_gene);
    }
    let _ = std::fs::remove_file(out);
    eprintln!(
        "exercised real save {} ({} bibites, {} eggs)",
        path.display(),
        bundle.bibites.len(),
        bundle.eggs.len()
    );
}

#[test]
fn real_save_runtime_ids_match_raw_json() {
    let path = real_dir().join("saves/lol.zip");
    if !path.exists() {
        eprintln!("skipping: {} absent", path.display());
        return;
    }
    let bundle = load_save_bundle(&path).unwrap();
    assert_eq!(bundle.bibite_ids.len(), bundle.bibites.len());
    assert_eq!(bundle.egg_ids.len(), bundle.eggs.len());

    // Previously `body.id` was read from the unknown-field map and every id was 0.
    let nonzero = bundle.bibite_ids.iter().filter(|id| **id != 0).count();
    let unique = bundle
        .bibite_ids
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    assert!(
        nonzero >= bundle.bibite_ids.len() * 3 / 4,
        "ids: {:?}",
        bundle.bibite_ids
    );
    assert!(
        unique >= bundle.bibite_ids.len() * 3 / 4,
        "ids: {:?}",
        bundle.bibite_ids
    );

    // Cross-check against the raw JSON in the zip (entry order = save order).
    let file = std::fs::File::open(&path).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    let mut raw_bibite_ids = Vec::new();
    let mut raw_egg_ids = Vec::new();
    let mut raw_parents = Vec::new();
    let mut raw_children = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        let name = entry.name().replace('\\', "/").to_ascii_lowercase();
        if (!name.starts_with("bibites/") && !name.starts_with("eggs/")) || !name.ends_with(".bb8")
        {
            continue;
        }
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        if name.starts_with("bibites/") {
            raw_bibite_ids.push(value["body"]["id"].as_i64().unwrap_or(0) as i32);
            raw_parents.push((
                value["genes"]["parent1"].as_i64().map(|id| id as i32),
                value["genes"]["parent2"].as_i64().map(|id| id as i32),
            ));
            raw_children.push(
                value["body"]["eggLayer"]["children"]
                    .as_array()
                    .map(|children| {
                        children
                            .iter()
                            .filter_map(serde_json::Value::as_i64)
                            .map(|id| id as i32)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
            );
        } else {
            raw_egg_ids.push(value["egg"]["id"].as_i64().unwrap_or(0) as i32);
        }
    }
    assert_eq!(bundle.bibite_ids, raw_bibite_ids);
    assert_eq!(bundle.egg_ids, raw_egg_ids);
    assert_eq!(bundle.bibite_parents, raw_parents);
    assert_eq!(bundle.bibite_children, raw_children);
    eprintln!(
        "lol runtime ids: {} bibites ({} unique, first {:?}), eggs {:?}",
        bundle.bibite_ids.len(),
        unique,
        &bundle.bibite_ids[..bundle.bibite_ids.len().min(5)],
        bundle.egg_ids
    );
}

#[test]
fn error_cases_are_rejected() {
    assert!(load_scenario_bundle(fixtures().join("bad.zip")).is_err());
    assert!(load_scenario_bundle(fixtures().join("missing_info.zip")).is_err());
    assert!(load_save_bundle(fixtures().join("bad.zip")).is_err());
}

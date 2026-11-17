//! Save-section coverage: every available real save is loaded, written and
//! reloaded, comparing each modelled section field-by-field.

use std::path::{Path, PathBuf};

use bibites_core::body::{BibiteSave, EggSave};
use bibites_io::{
    load_save_bundle, save_bundle_from_contents, world_contents_from_save, write_save_bundle,
};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn saves() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in [
        workspace().join("testdata/real/saves"),
        workspace().join("testdata/parity/bases"),
    ] {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("zip")
                    && !path.to_string_lossy().contains("ring.base")
                {
                    out.push(path);
                }
            }
        }
    }
    if let Some(base) = Some(workspace().join("testdata/parity/bases/ring.base.zip")) {
        if base.is_file() {
            out.push(base);
        }
    }
    out
}

fn compare_bibite(first: &BibiteSave, second: &BibiteSave) {
    assert_eq!(first.pos, second.pos);
    assert_eq!(first.vel, second.vel);
    assert_eq!(first.rot, second.rot, "bibite rot (degrees)");
    assert_eq!(first.ang_vel, second.ang_vel);
    assert_eq!(first.genome, second.genome);
    assert_eq!(first.meta, second.meta);
    assert_eq!(first.brain, second.brain);
    assert_eq!(first.energy, second.energy);
    assert_eq!(first.health, second.health);
    assert_eq!(first.d2_size, second.d2_size);
    assert_eq!(first.dying, second.dying);
    assert_eq!(first.dead, second.dead);
    assert_eq!(first.attacked_dmg, second.attacked_dmg);
    assert_eq!(first.times_attacked, second.times_attacked);
    assert_eq!(first.total_damage_suffered, second.total_damage_suffered);
    assert_eq!(first.brain_ticks, second.brain_ticks);
    assert_eq!(first.vision_lookup, second.vision_lookup);
    assert_eq!(first.vision_sensing, second.vision_sensing);
    assert_eq!(first.corpse_energy_offset, second.corpse_energy_offset);
    assert_eq!(first.growth, second.growth);
    assert_eq!(first.stomach, second.stomach);
    assert_eq!(first.n_stomach, second.n_stomach);
    assert_eq!(first.bite_progress, second.bite_progress);
    assert_eq!(first.attacked_last_frame, second.attacked_last_frame);
    assert_eq!(first.bibites_bitten, second.bibites_bitten);
    assert_eq!(first.total_damage_dealt, second.total_damage_dealt);
    assert_eq!(first.total_murders, second.total_murders);
    assert_eq!(first.murdered_area, second.murdered_area);
    assert_eq!(first.egg_progress, second.egg_progress);
    assert_eq!(first.n_eggs_laid, second.n_eggs_laid);
    assert_eq!(first.children, second.children);
    assert_eq!(first.fat_amount, second.fat_amount);
    assert_eq!(first.phero_progress, second.phero_progress);
    assert_eq!(first.tic, second.tic);
    assert_eq!(first.tic_progress, second.tic_progress);
    assert_eq!(first.time_alive, second.time_alive);
    assert_eq!(first.chrono_time, second.chrono_time);
    assert_eq!(first.total_travel, second.total_travel);
}

fn compare_egg(first: &EggSave, second: &EggSave) {
    assert_eq!(first.pos, second.pos);
    assert_eq!(first.vel, second.vel);
    assert_eq!(first.rot, second.rot, "egg rot (degrees)");
    assert_eq!(first.ang_vel, second.ang_vel);
    assert_eq!(first.genome, second.genome);
    assert_eq!(first.meta, second.meta);
    assert_eq!(first.brain, second.brain);
    assert_eq!(first.energy, second.energy);
    assert_eq!(first.hatch_progress, second.hatch_progress);
    assert_eq!(first.hatch_time, second.hatch_time);
    assert_eq!(first.total_required_energy, second.total_required_energy);
    assert_eq!(first.is_hatching, second.is_hatching);
    assert_eq!(first.aborting, second.aborting);
    assert_eq!(first.birth_orientation, second.birth_orientation);
    assert_eq!(first.size_gene, second.size_gene);
}

#[test]
fn every_available_save_round_trips_every_section() {
    let mut checked = 0usize;
    for path in saves() {
        let bundle = match load_save_bundle(&path) {
            Ok(bundle) => bundle,
            Err(error) => panic!("{} must load: {error}", path.display()),
        };
        let out = std::env::temp_dir().join(format!(
            "rustibites_coverage_{}_{}.zip",
            std::process::id(),
            checked
        ));
        write_save_bundle(&bundle, &out).unwrap();
        let reloaded = load_save_bundle(&out).unwrap();
        assert_eq!(
            reloaded.settings,
            bundle.settings,
            "{} settings",
            path.display()
        );
        assert_eq!(reloaded.settings_file, bundle.settings_file);
        assert_eq!(reloaded.scene, bundle.scene);
        assert_eq!(reloaded.vars, bundle.vars);
        assert_eq!(reloaded.pellets, bundle.pellets);
        assert_eq!(reloaded.pheromones, bundle.pheromones);
        assert_eq!(reloaded.data_bin, bundle.data_bin);
        assert_eq!(reloaded.species, bundle.species);
        assert_eq!(reloaded.templates, bundle.templates);
        assert_eq!(reloaded.img_png, bundle.img_png);
        assert_eq!(reloaded.unknown_entries, bundle.unknown_entries);
        assert_eq!(reloaded.bibite_ids, bundle.bibite_ids);
        assert_eq!(reloaded.bibite_parents, bundle.bibite_parents);
        assert_eq!(reloaded.bibite_children, bundle.bibite_children);
        assert_eq!(reloaded.egg_ids, bundle.egg_ids);
        assert_eq!(reloaded.bibites.len(), bundle.bibites.len());
        assert_eq!(reloaded.eggs.len(), bundle.eggs.len());
        for (first, second) in bundle.bibites.iter().zip(reloaded.bibites.iter()) {
            compare_bibite(first, second);
        }
        for (first, second) in bundle.eggs.iter().zip(reloaded.eggs.iter()) {
            compare_egg(first, second);
        }
        let first = world_contents_from_save(&bundle).unwrap();
        let second = world_contents_from_save(&reloaded).unwrap();
        assert_eq!(first.zones, second.zones);
        assert_eq!(first.rad_towers, second.rad_towers);
        assert_eq!(first.phero_towers, second.phero_towers);
        assert_eq!(first.color_killers, second.color_killers);
        assert_eq!(first.pellets, second.pellets);
        assert_eq!(first.pheromones, second.pheromones);
        let _ = std::fs::remove_file(out);
        checked += 1;
    }
    if checked == 0 {
        eprintln!("skipping save coverage: no save fixtures present");
    } else {
        eprintln!("save coverage: {checked} save(s) round-tripped every section");
    }
}

#[test]
fn tools_and_scene_sections_round_trip() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bundles/save_min.zip");
    let mut bundle = load_save_bundle(&fixture).unwrap();
    let scene = bundle.scene.as_object_mut().unwrap();
    scene.insert("name".into(), serde_json::json!("CoverageTools"));
    scene.insert(
        "radTowers".into(),
        serde_json::json!([{"id": 3, "attachedZone": 0, "x": 10.0, "y": 20.0, "count": 0.01, "intensity": 0.02, "radius": 400.0, "taper": 1.0}]),
    );
    scene.insert(
        "pheroTowers".into(),
        serde_json::json!([{"id": 4, "attachedZone": 0, "x": 5.0, "y": 6.0, "r": 0.1, "g": 0.2, "b": 0.3, "dir": 0.5}]),
    );
    scene.insert(
        "colorSelectors".into(),
        serde_json::json!([{"transform": {"position": [1.0, 2.0], "rotation": 0.0, "scale": 1.0}, "hue": 0.4, "tolerance": 0.05, "bodyOrEye": true, "anyOrMost": false, "radius": 800.0, "period": 60.0, "progress": 0.0}]),
    );
    bundle.pellets = Some(
        serde_json::json!({"pellets": [{"zone": "World", "pellets": [
            {"transform": {"position": [1.0, 2.0], "rotation": 0.0, "scale": 1.0},
             "rb2d": {"px": 1.0, "py": 2.0, "vx": 0.0, "vy": 0.0, "r": 0.0},
             "pellet": {"material": "Plant", "amount": 5.0}},
            {"transform": {"position": [3.0, 4.0], "rotation": 0.0, "scale": 1.0},
             "rb2d": {"px": 3.0, "py": 4.0, "vx": 0.0, "vy": 0.0, "r": 0.0},
             "pellet": {"material": "Meat", "amount": 7.0, "matterDecay": {"timeAlive": 100.0, "rotAmount": 0.25}}}
        ]}]}),
    );
    bundle.pheromones = Some(serde_json::json!({"pheromones": [
        {"transform": {"position": [1.0, 2.0], "rotation": 0.0, "scale": 1.0},
         "phero": {"heading": {}, "Rstrength": -0.5, "Gstrength": 0.25, "Bstrength": 1.5,
                   "nR": 0.0, "nG": 0.0, "nB": 0.0}}
    ]}));
    let out = std::env::temp_dir().join(format!("rustibites_tools_{}.zip", std::process::id()));
    write_save_bundle(&bundle, &out).unwrap();
    let reloaded = load_save_bundle(&out).unwrap();
    assert_eq!(reloaded.scene, bundle.scene);
    let contents = world_contents_from_save(&reloaded).unwrap();
    assert_eq!(contents.rad_towers.len(), 1);
    assert_eq!(contents.rad_towers[0].base.save_id, 3);
    assert_eq!(contents.rad_towers[0].radius, 400.0);
    assert_eq!(contents.phero_towers.len(), 1);
    assert_eq!(contents.phero_towers[0].base.save_id, 4);
    assert_eq!(contents.phero_towers[0].red, 0.1);
    assert_eq!(contents.color_killers.len(), 1);
    assert_eq!(contents.color_killers[0].hue, 0.4);
    assert!(contents.color_killers[0].body_or_eye);
    // Attached-zone mapping: serialized zone id 0 -> zone-list index 0 (gen-0 id).
    assert!(contents.rad_towers[0].base.attached_zone.is_some());
    assert!(contents.phero_towers[0].base.attached_zone.is_some());
    // Pellet decay/rotation fields and pheromone flags/owner.
    assert_eq!(contents.pellets.len(), 2);
    assert_eq!(contents.pellets[0].owner_zone, Some(0));
    assert!(contents.pellets[0].fresh_time_remaining.is_none());
    let fresh = contents.pellets[1]
        .fresh_time_remaining
        .expect("meat decays");
    assert!((fresh - 1900.0).abs() < 1e-3, "fresh remaining {fresh}");
    assert_eq!(contents.pellets[1].rot_amount, 0.25);
    assert_eq!(contents.pheromones.len(), 1);
    assert_eq!(contents.pheromones[0].strength, [-0.5, 0.25, 1.5]);
    assert!(contents.pheromones[0].dissipate);
    assert!(contents.pheromones[0].owner_tool.is_none());
    // Color killers have no serialized id in the game format: save_id is 0.
    let rebuilt = save_bundle_from_contents(&contents, Some(&reloaded)).unwrap();
    write_save_bundle(&rebuilt, &out).unwrap();
    let again = world_contents_from_save(&load_save_bundle(&out).unwrap()).unwrap();
    assert_eq!(again.rad_towers, contents.rad_towers);
    assert_eq!(again.phero_towers, contents.phero_towers);
    assert_eq!(again.color_killers, contents.color_killers);
    assert_eq!(again.pellets, contents.pellets);
    assert_eq!(again.pheromones, contents.pheromones);
    let _ = std::fs::remove_file(out);
}

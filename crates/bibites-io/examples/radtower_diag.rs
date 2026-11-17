//! Diagnostic for the RadTower.zip scenario→save→reload fingerprint difference
//! (task-74 follow-up). Builds the scenario world like the CLI (`World::new` +
//! `seed_scenario_world` + templates + fresh spawner) and the save world like
//! the CLI (`load_world_contents` + `World::from_contents`), then compares the
//! per-entity identity/state that `state_fingerprint` mixes.

use std::path::{Path, PathBuf};

use bibites_core::sim::state_fingerprint;
use bibites_core::world::{TemplateSource, World};
use bibites_io::{load_scenario_bundle, load_template, load_world_contents};

fn user_templates() -> Vec<TemplateSource> {
    let Some(profile) = std::env::var_os("USERPROFILE") else {
        return Vec::new();
    };
    let dir =
        PathBuf::from(profile).join("AppData/LocalLow/The Bibites/The Bibites/Bibites/Templates");
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("bb8template") {
                if let Ok(loaded) = load_template(&path) {
                    out.push(TemplateSource::new(
                        path.file_name().unwrap().to_string_lossy().to_string(),
                        loaded.genome,
                        loaded.meta,
                        loaded.brain,
                    ));
                }
            }
        }
    }
    out
}

fn fnv(mut hash: u64, value: u64) -> u64 {
    hash ^= value;
    hash.wrapping_mul(0x0000_0100_0000_01b3)
}

fn pellet_multiset(world: &World) -> Vec<(u32, u32, u32)> {
    let mut out: Vec<(u32, u32, u32)> = world
        .pellets
        .ids()
        .filter_map(|id| world.pellets.get(id))
        .map(|view| {
            (
                view.pos.x.to_bits(),
                view.pos.y.to_bits(),
                view.amount.to_bits(),
            )
        })
        .collect();
    out.sort_unstable();
    out
}

fn dump(label: &str, world: &World) {
    println!(
        "{label}: fp={} bibites={} eggs={} pellets={} pheromones={}",
        state_fingerprint(world),
        world.bibites.len(),
        world.eggs.len(),
        world.pellets.len(),
        world.pheromones.len()
    );
    for (id, record) in world.bibites.iter() {
        println!(
            "  bibite id {}v{} pos {:?} energy {}",
            id.handle().slot(),
            id.handle().generation(),
            record.pos,
            record.body.energy.amount
        );
    }
    let mut pellet_hash = 0xcbf2_9ce4_8422_2325u64;
    for id in world.pellets.ids() {
        let Some(view) = world.pellets.get(id) else {
            continue;
        };
        pellet_hash = fnv(pellet_hash, u64::from(id.handle().slot()));
        pellet_hash = fnv(pellet_hash, u64::from(id.handle().generation()));
        pellet_hash = fnv(pellet_hash, u64::from(view.pos.x.to_bits()));
        pellet_hash = fnv(pellet_hash, u64::from(view.pos.y.to_bits()));
        pellet_hash = fnv(pellet_hash, u64::from(view.amount.to_bits()));
    }
    println!("  pellet-order hash {pellet_hash:#x}");
    let multiset = pellet_multiset(world);
    println!(
        "  pellet multiset first {:?} last {:?}",
        multiset.first(),
        multiset.last()
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scenario_path = Path::new("testdata/real/RadTower.zip");
    let save_path = Path::new("target/compat/rt/rad1.zip");
    if !save_path.is_file() {
        eprintln!("run the RadTower repro first: {}", save_path.display());
        return Ok(());
    }
    let bundle = load_scenario_bundle(scenario_path)?;
    let mut scenario_world = World::new(bundle.settings.clone(), bundle.scenario.clone(), 1)?;
    let _ = scenario_world.seed_scenario_world();
    scenario_world.set_templates(user_templates());
    let spawned = scenario_world.start_fresh_scenario_spawner();
    println!("scenario spawned {spawned}");
    dump("scenario", &scenario_world);
    let scenario_multiset = pellet_multiset(&scenario_world);

    let mut contents = load_world_contents(save_path, None)?;
    contents.seed = 1;
    let save_world = World::from_contents(contents)?;
    dump("save", &save_world);
    let save_multiset = pellet_multiset(&save_world);
    println!(
        "pellet multisets equal: {}",
        scenario_multiset == save_multiset
    );
    Ok(())
}

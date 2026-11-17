//! Generates the game-load verification fixture pack (testdata/parity/compat).
//!
//! Usage: cargo run -p bibites-io --example compat_fixtures -- <out_dir>
//!
//! The pack exercises every writer surface that could differ from the game:
//! real save entities/eggs/corpses/species, synthetic tools + pheromones,
//! synthetic zones/templates, scenarios, and a save with a `.bb8state` sidecar.
//! `tools/roundtrip/gen_compat_pack.ps1` also adds a CLI-generated 0.6
//! scenario-start save and writes `manifest.json` (sha256 + expected counts).

use std::path::{Path, PathBuf};

use bibites_core::sim::SimSnapshot;
use bibites_io::{
    LoadedGenome, load_save_bundle, load_scenario_bundle, write_checkpoint_state_with_seed,
    write_save_bundle, write_scenario_bundle,
};

fn real(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "testdata/parity/compat".to_string()),
    );
    std::fs::create_dir_all(&out)?;

    // 1) Real save: 35 bibites incl. corpses, 4 eggs, species, pellets/pheromones.
    let lol = load_save_bundle(real("testdata/real/saves/lol.zip"))?;
    write_save_bundle(&lol, out.join("rust_lol_save.zip"))?;

    // 2) Same save + sibling checkpoint sidecar (game must ignore the sidecar).
    let simulated_time = lol
        .scene
        .get("simulatedTime")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let snapshot = SimSnapshot {
        tick: simulated_time as u64,
        simulated_time,
        sim_tps: lol.settings.sim_tps,
        ..SimSnapshot::default()
    };
    let state_save = out.join("rust_state_save.zip");
    write_save_bundle(&lol, &state_save)?;
    write_checkpoint_state_with_seed(&state_save, &snapshot, 12345)?;

    // 3) Tools + pheromones on the known-good real save (game-valid entities).
    let mut tools = load_save_bundle(real("testdata/real/saves/lol.zip"))?;
    {
        let scene = tools.scene.as_object_mut().expect("scene object");
        scene.insert("name".into(), serde_json::json!("Rust tools"));
        scene.insert(
            "radTowers".into(),
            serde_json::json!([{"id": 7, "x": 100.0, "y": -50.0, "count": 0.01, "intensity": 0.02, "radius": 400.0, "taper": 1.0}]),
        );
        scene.insert(
            "pheroTowers".into(),
            serde_json::json!([{"id": 8, "x": 5.0, "y": 6.0, "r": 0.2, "g": 0.1, "b": 0.4, "dir": 0.5}]),
        );
        scene.insert(
            "colorSelectors".into(),
            serde_json::json!([{"transform": {"position": [1.0, 2.0], "rotation": 0.0, "scale": 1.0}, "hue": 0.4, "tolerance": 0.05, "bodyOrEye": true, "anyOrMost": false, "radius": 800.0, "period": 60.0, "progress": 0.0}]),
        );
    }
    tools.pheromones = Some(serde_json::json!({"pheromones": [
        {"transform": {"position": [1.0, 2.0], "rotation": 0.0, "scale": 1.0},
         "phero": {"heading": {}, "Rstrength": -0.5, "Gstrength": 0.25, "Bstrength": 1.5,
                   "nR": 0.0, "nG": 0.0, "nB": 0.0}},
        {"transform": {"position": [-3.0, 4.0], "rotation": 0.0, "scale": 1.0},
         "phero": {"heading": {}, "Rstrength": 0.0, "Gstrength": -1.0, "Bstrength": 0.0,
                   "nR": 0.0, "nG": 0.0, "nB": 0.0}}
    ]}));
    write_save_bundle(&tools, out.join("rust_tools_phero_save.zip"))?;

    // 4) Eggs: the game-validated real save already carries four eggs; re-emit it
    // under its own fixture name (no synthetic entity states).
    let mut eggs = load_save_bundle(real("testdata/real/saves/lol.zip"))?;
    eggs.scene
        .as_object_mut()
        .unwrap()
        .insert("name".into(), serde_json::json!("Rust eggs"));
    write_save_bundle(&eggs, out.join("rust_eggs_save.zip"))?;

    // 5) Corpses: the real save contains 14 corpse entries; re-emit unchanged so
    // the game's corpse load path is exercised without fabricated dead state.
    let mut corpses = load_save_bundle(real("testdata/real/saves/lol.zip"))?;
    corpses
        .scene
        .as_object_mut()
        .unwrap()
        .insert("name".into(), serde_json::json!("Rust corpses"));
    write_save_bundle(&corpses, out.join("rust_corpses_save.zip"))?;

    // 6) Real scenarios re-emitted by the Rust writer (canonical Default + Ring).
    let default = load_scenario_bundle(real("testdata/real/Default.zip"))?;
    write_scenario_bundle(&default, out.join("rust_default_scenario.zip"))?;
    let ring = load_scenario_bundle(real("testdata/real/Ring.zip"))?;
    write_scenario_bundle(&ring, out.join("rust_ring_scenario.zip"))?;

    // A template the game can import/convert directly.
    let template: LoadedGenome = bibites_io::load_template(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/template_current.bb8template"),
    )?;
    bibites_io::save_template(
        out.join("rust_template.bb8template"),
        &template.genome,
        &template.meta,
        &template.brain,
        &template.template,
    )?;

    // Keep a compact human-readable inventory next to the manifest.
    let mut inventory = Vec::new();
    for entry in std::fs::read_dir(&out)? {
        let path = entry?.path();
        if path.is_file() {
            inventory.push(format!(
                "{} {}",
                path.file_name().unwrap().to_string_lossy(),
                std::fs::metadata(&path)?.len()
            ));
        }
    }
    inventory.sort();
    std::fs::write(out.join("INVENTORY.txt"), inventory.join("\n"))?;
    println!("wrote compat fixtures to {}", out.display());
    Ok(())
}

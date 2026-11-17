//! Writes a Rust-authored scenario `.zip` for the game round-trip acceptance
//! check (task-43): load a real scenario through `bibites-io`, then re-emit it
//! with `write_scenario_bundle` (zones, spawn rows, settings, animators and
//! unknown entries preserved).
//!
//! Usage: `cargo run -p bibites-io --example export_scenario -- <in.zip> <out.zip>`

use std::path::Path;

use bibites_io::{load_scenario_bundle, write_scenario_bundle};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "testdata/real/Default.zip".to_string());
    let output = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "testdata/roundtrip/rust_scenario.zip".to_string());
    let bundle = load_scenario_bundle(Path::new(&input))?;
    write_scenario_bundle(&bundle, Path::new(&output))?;
    println!(
        "wrote {output}: name={:?} version={} zones={} zone_groups={} spawns={} changers={} templates={} bytes={}",
        bundle.info.name,
        bundle.info.version_text,
        bundle.scenario.zones.len(),
        bundle.scenario.zone_groups.len(),
        bundle.scenario.bibite_spawns.len(),
        bundle.scenario.settings_changers.len(),
        bundle.templates.len(),
        std::fs::metadata(&output)?.len()
    );
    Ok(())
}

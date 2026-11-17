//! Writes a `.bb8template` via the RUSTibites io writer for the game round-trip
//! acceptance check (`task-34`): load the bundled synthetic fixture and save it
//! back at the present version.
//!
//! Usage: `cargo run -p bibites-io --example export_template -- <out.bb8template>`

use std::path::Path;

use bibites_io::{load_template, save_template};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "rust_template.bb8template".to_string());
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/template_current.bb8template");
    let loaded = load_template(&source)?;
    save_template(
        &out,
        &loaded.genome,
        &loaded.meta,
        &loaded.brain,
        &loaded.template,
    )?;
    println!(
        "wrote {out} (version {}, {} genes, {} nodes, {} synapses)",
        loaded.version,
        loaded.genome.genes.len(),
        loaded.brain.node_count(),
        loaded.brain.synapse_count()
    );
    Ok(())
}

//! Real-world template compatibility (task-44): the game tolerates node-index
//! shifts, `-1` synapse endpoints and short pre-0.6a1 gene arrays; the tolerant
//! reader repairs them instead of rejecting the file.

use std::path::{Path, PathBuf};

use bibites_io::{load_template, load_template_tolerant};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn tolerant_accepts_index_mismatch_and_negative_endpoints() {
    let index = fixtures().join("bad_index.bb8template");
    assert!(load_template_tolerant(&index).is_ok());
    // Default template loading is tolerant (CLI/spawner path).
    assert!(load_template(&index).is_ok());

    let negative = fixtures().join("template_negative_endpoints.bb8template");
    let loaded = load_template_tolerant(&negative).unwrap();
    assert_eq!(loaded.brain.node_count(), 48);
    let synapses = loaded.brain.to_synapse_records();
    assert!(
        synapses.iter().all(
            |synapse| (synapse.node_in as usize) < loaded.brain.node_count()
                && (synapse.node_out as usize) < loaded.brain.node_count()
        ),
        "negative endpoint synapse must be dropped/remapped"
    );
}

#[test]
fn tolerant_pads_short_old_gene_arrays() {
    let path = fixtures().join("template_short_genes_0.5.bb8template");
    let loaded = load_template_tolerant(&path).unwrap();
    assert_eq!(loaded.genome.genes.len(), 35);
    assert!(loaded.genome.genes.iter().all(|gene| gene.is_finite()));
}

#[test]
fn real_user_template_folder_if_present() {
    let dir =
        Path::new(r"C:\Users\Storme\AppData\LocalLow\The Bibites\The Bibites\Bibites\Templates");
    if !dir.is_dir() {
        eprintln!(
            "skipping real user template folder (absent): {}",
            dir.display()
        );
        return;
    }
    let mut total = 0;
    let mut loaded = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("bb8template") {
            continue;
        }
        total += 1;
        match load_template_tolerant(&path) {
            Ok(genome) => {
                assert_eq!(genome.genome.genes.len(), 35);
                loaded += 1;
            }
            Err(error) => panic!("tolerant load failed for {}: {error}", path.display()),
        }
    }
    eprintln!("real user templates: {loaded}/{total} loadable (tolerant)");
    assert!(
        total >= 100,
        "expected the real user folder to be populated"
    );
    assert_eq!(loaded, total, "all real user templates must load");
}

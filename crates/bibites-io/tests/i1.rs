//! Integration tests for bibites-io phase I1 (`.bb8` / `.bb8template`).
//!
//! Synthetic fixtures live in `tests/fixtures/` so CI needs no game data. Real
//! files under `testdata/real/` (gitignored) are exercised when present and
//! skipped cleanly otherwise.

use std::path::{Path, PathBuf};

use bibites_core::brain::{N_INPUTS, N_OUTPUTS, NodeType};
use bibites_core::genome::{Genome, GenomeMeta};
use bibites_io::{
    LoadedGenome, N_INPUTS as IO_INPUTS, N_OUTPUTS as IO_OUTPUTS, SourceShape, TemplateMeta,
    from_json, inspect, load_genome, load_template, save_genome, save_template, template_to_json,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(fixtures().join(name)).expect("fixture must exist")
}

fn real_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/real")
}

fn assert_present_brain(loaded: &LoadedGenome, context: &str) {
    assert!(
        loaded.brain.node_count() >= N_INPUTS + N_OUTPUTS,
        "{context}: node count"
    );
    let records = loaded.brain.to_node_records();
    for (index, record) in records.iter().take(N_INPUTS).enumerate() {
        assert_eq!(
            record.node_type,
            NodeType::Input,
            "{context}: node {index} is not an input"
        );
    }
    for (index, record) in records.iter().skip(N_INPUTS).take(N_OUTPUTS).enumerate() {
        assert_ne!(
            record.node_type,
            NodeType::Input,
            "{context}: output {index} has Input activation"
        );
    }
}

#[test]
fn current_template_fixture_loads_and_round_trips() {
    let loaded = load_template(fixtures().join("template_current.bb8template")).unwrap();
    assert_eq!(loaded.source, SourceShape::Template);
    assert_eq!(loaded.version.to_string(), "0.6.3.1");
    assert_eq!(loaded.genome.genes.len(), 35);
    assert_eq!(loaded.meta.generation, 12);
    assert_eq!(loaded.template.name.as_deref(), Some("Fixture Sample"));
    assert_eq!(
        loaded.template.description.as_deref(),
        Some("synthetic current-version template")
    );
    assert_eq!(loaded.template.node_anchors.len(), 2);
    assert_present_brain(&loaded, "current template");

    let text = loaded.to_json().unwrap();
    let reloaded = from_json(&text).unwrap();
    assert_eq!(reloaded.genome, loaded.genome);
    assert_eq!(reloaded.brain, loaded.brain);
    assert_eq!(reloaded.meta, loaded.meta);
    assert_eq!(reloaded.version, loaded.version);
    // Bytes are stable when re-writing the reloaded file.
    assert_eq!(reloaded.to_json().unwrap(), text);
}

#[test]
fn living_bb8_fixture_loads_and_round_trips() {
    let loaded = load_genome(fixtures().join("living.bb8")).unwrap();
    assert_eq!(loaded.source, SourceShape::LivingBb8);
    assert!(loaded.is_ready);
    assert_eq!(loaded.parent1, Some(123));
    assert_eq!(loaded.meta.generation, 5);
    assert_eq!(loaded.meta.species.map(|species| species.value()), Some(42));
    assert_eq!(loaded.meta.species_tag.as_deref(), Some("Fixture"));
    assert_present_brain(&loaded, "living fixture");

    let text = loaded.to_json().unwrap();
    let reloaded = from_json(&text).unwrap();
    assert_eq!(reloaded.genome, loaded.genome);
    assert_eq!(reloaded.brain, loaded.brain);
    assert_eq!(reloaded.parent1, loaded.parent1);
}

#[test]
fn legacy_0_6a6_fixture_migrates() {
    let loaded = load_template(fixtures().join("template_0.6a6.bb8template")).unwrap();
    assert_eq!(loaded.version.to_string(), "0.6.0a6");
    assert_present_brain(&loaded, "0.6a6 template");
    let records = loaded.brain.to_node_records();
    // 0.6a16 migration makes Want2Eat (index 38) a TanH node.
    assert_eq!(records[38].node_type, NodeType::TanH);
    // 0.6a13 migration folds the enabled Constant-input synapse (w=2.0)
    // into EggProduction, which ends at index 36; all old node-0 synapses are
    // disabled and re-pointed away from input 0.
    let output = &records[36];
    assert!((output.base_activation - 2.0).abs() < 1e-6);
    assert!(
        !loaded
            .brain
            .to_synapse_records()
            .iter()
            .any(|s| s.node_in == 0 && s.enabled),
        "old Constant-input synapses must be disabled"
    );
    // All organs get a positive share from the Strength/Defence conversion.
    assert!(loaded.genome.genes[26] > 0.0);
}

#[test]
fn legacy_0_5_fixture_migrates() {
    let loaded = load_template(fixtures().join("template_0.5.bb8template")).unwrap();
    assert_present_brain(&loaded, "0.5 template");
    assert_eq!(loaded.genome.genes.len(), 35);
    // 34-array remap + Strength/Defence organ conversion produces a finite
    // genome with positive WAG values.
    assert!(loaded.genome.genes.iter().all(|gene| gene.is_finite()));
    assert!(loaded.genome.genes[26] > 0.0);
    // Round-trip after migration is stable.
    let text = loaded.to_json().unwrap();
    let reloaded = from_json(&text).unwrap();
    assert_eq!(reloaded.genome, loaded.genome);
    assert_eq!(reloaded.brain, loaded.brain);
}

#[test]
fn unknown_fields_are_preserved() {
    let text = read_fixture("template_current.bb8template");
    let injected = text.replacen('{', "{\n  \"unknownTopLevel\": 42,", 1);
    let injected = injected.replacen(
        "\"Type\": 0,",
        "\"Type\": 0,\n      \"unknownNodeField\": 7,",
        1,
    );
    let loaded = from_json(&injected).unwrap();
    let output = loaded.to_json().unwrap();
    assert!(
        output.contains("unknownTopLevel"),
        "unknown field stripped: {output}"
    );
    assert!(
        output.contains("unknownNodeField"),
        "unknown node field stripped: {output}"
    );
}

#[test]
fn error_fixtures_are_rejected() {
    for name in [
        "truncated.json",
        "bad_version.bb8template",
        "bad_index.bb8template",
        "duplicate_innovation.bb8template",
    ] {
        let text = read_fixture(name);
        assert!(
            from_json(&text).is_err(),
            "fixture {name} should be rejected"
        );
    }
}

#[test]
fn file_api_and_inspect_work() {
    let dir = std::env::temp_dir().join("rustibites_io_i1");
    std::fs::create_dir_all(&dir).unwrap();
    let template = TemplateMeta {
        name: Some("Written".to_string()),
        ..TemplateMeta::default()
    };
    let genome = Genome::defaults();
    let meta = GenomeMeta {
        generation: 1,
        ..GenomeMeta::default()
    };
    let brain = bibites_core::brain::Brain::starter();
    let template_path = dir.join("written.bb8template");
    save_template(&template_path, &genome, &meta, &brain, &template).unwrap();
    let loaded = load_template(&template_path).unwrap();
    assert_eq!(loaded.genome, genome);
    assert_eq!(loaded.brain, brain);
    assert_eq!(loaded.template.name.as_deref(), Some("Written"));

    let genome_path = dir.join("written.bb8");
    save_genome(&genome_path, &genome, &meta, &brain).unwrap();
    let loaded = load_genome(&genome_path).unwrap();
    assert_eq!(loaded.genome, genome);
    let summary = inspect(&genome_path).unwrap();
    assert_eq!(summary.gene_count, 35);
    assert_eq!(summary.node_count, IO_INPUTS + IO_OUTPUTS);
    assert_eq!(summary.version, "0.6.3.1");
    let template_text = template_to_json(&genome, &meta, &brain, &template).unwrap();
    assert!(template_text.contains("\"name\":\"Written\""));
    assert!(template_text.contains("\"version\":\"0.6.3.1\""));
}

#[test]
fn real_files_if_present() {
    let real = real_dir();
    if !real.exists() {
        eprintln!("skipping real-file tests: {} not present", real.display());
        return;
    }
    let mut exercised = 0usize;
    // Current-version templates: strict invariants.
    let current = real.join("templates_current");
    if current.is_dir() {
        for entry in std::fs::read_dir(&current).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("bb8template") {
                continue;
            }
            let loaded = load_template(&path)
                .unwrap_or_else(|error| panic!("failed to load {}: {error}", path.display()));
            assert_present_brain(&loaded, &path.display().to_string());
            let text = loaded.to_json().unwrap();
            let reloaded = from_json(&text).unwrap();
            assert_eq!(reloaded.genome, loaded.genome);
            assert_eq!(reloaded.brain, loaded.brain);
            exercised += 1;
            eprintln!("exercised current template {}", path.display());
        }
    }
    // Older-era templates: must either load (with migration) or be rejected as
    // explicitly unsupported; silently dropping data is a bug.
    let legacy = real.join("templates");
    if legacy.is_dir() {
        for entry in std::fs::read_dir(&legacy).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("bb8template") {
                continue;
            }
            match load_template(&path) {
                Ok(loaded) => {
                    assert_present_brain(&loaded, &path.display().to_string());
                    exercised += 1;
                    eprintln!("exercised legacy template {}", path.display());
                }
                Err(error) => {
                    eprintln!("legacy template {} unsupported: {error}", path.display());
                }
            }
        }
    }
    let bibites = real.join("bibites");
    if bibites.is_dir() {
        for entry in std::fs::read_dir(&bibites).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("bb8") {
                continue;
            }
            let loaded = load_genome(&path)
                .unwrap_or_else(|error| panic!("failed to load {}: {error}", path.display()));
            assert_present_brain(&loaded, &path.display().to_string());
            exercised += 1;
            eprintln!("exercised bibite {}", path.display());
        }
    }
    assert!(
        exercised > 0,
        "no real fixtures exercised under {}",
        real.display()
    );
    eprintln!("real-file tests exercised {exercised} files");
}

//! Corruption/fuzz robustness (task-55): malformed inputs must return errors,
//! never panic. Deterministic, fast, and skips game-data-dependent parts when
//! `testdata/` is absent.

// NOTE: non-Default real files used here are deliberate format/version coverage,
// not the canonical test baseline (see docs/TESTING_POLICY.md); tests skip if
// testdata/ is absent.
use std::path::{Path, PathBuf};

use bibites_io::{
    load_genome, load_save_bundle, load_scenario_bundle, load_template, parse_settings_json,
};

fn real(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/real")
        .join(name)
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() as usize) % n
        }
    }
}

fn mutated(original: &[u8], rng: &mut Lcg, mode: u32) -> Vec<u8> {
    let mut bytes = original.to_vec();
    if bytes.is_empty() {
        return b"{}".to_vec();
    }
    match mode % 6 {
        0 => bytes.truncate(rng.below(bytes.len())),
        1 => {
            for _ in 0..(1 + rng.below(4)) {
                let at = rng.below(bytes.len());
                bytes[at] ^= 1 << rng.below(8);
            }
        }
        2 => {
            let at = rng.below(bytes.len());
            bytes.insert(at, (rng.below(256)) as u8);
        }
        3 => {
            let at = rng.below(bytes.len());
            bytes.remove(at);
        }
        4 => bytes = b"not json at all \0 \xff".to_vec(),
        _ => {
            // wrong/truncated JSON object: keep a JSON-ish prefix only
            let text = String::from_utf8_lossy(&bytes)
                .chars()
                .take(32)
                .collect::<String>();
            bytes = format!("{{{text}").into_bytes();
        }
    }
    bytes
}

#[test]
fn mutated_files_never_panic() {
    let mut samples = Vec::new();
    let candidates = [
        real("Ring.zip"),
        real("NestEgg.zip"),
        real("saves/lol.zip"),
        real("templates/1_Scorpion_0.6.bb8template"),
        real("templates_current/Basic_bibite_0.6.3.1.bb8template"),
        real("speciesData.json"),
        real("Storme's Super Sparse.bb8settings"),
    ];
    for path in candidates {
        if path.is_file() {
            samples.push(std::fs::read(&path).unwrap());
        }
    }
    if samples.is_empty() {
        eprintln!("skipping fuzz samples: testdata real files absent");
        return;
    }
    let mut rng = Lcg(0x5eed_1234_abcd_0001);
    for (index, original) in samples.iter().enumerate() {
        let is_zip = original.starts_with(b"PK");
        for iteration in 0..24 {
            let bytes = mutated(original, &mut rng, iteration as u32 + index as u32);
            let dir = std::env::temp_dir().join("rustibites_io_fuzz");
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(format!("fuzz_{index}_{iteration}.bin"));
            std::fs::write(&path, &bytes).unwrap();
            if is_zip {
                let _ = load_save_bundle(&path);
                let _ = load_scenario_bundle(&path);
            } else {
                let text = String::from_utf8_lossy(&bytes);
                let _ = parse_settings_json(&text);
                let _ = load_template(&path);
                let _ = load_genome(&path);
            }
            let _ = std::fs::remove_file(path);
        }
    }
    // JSON edge cases that must be errors, not panics.
    let big_number = format!("{{\"x\":{}}}", "9".repeat(2000));
    let deep_nesting = "[".repeat(500);
    for text in [
        "",
        "{",
        "[]",
        "null",
        "{\"a\":",
        "{\"genes\":{}}",
        "{\"nodes\":[],\"synapses\":[],\"genes\":{}}",
        big_number.as_str(),
        deep_nesting.as_str(),
        "{\"version\":\"0.6.3.1\",\"nodes\":[],\"synapses\":[],\"genes\":[]}",
    ] {
        let _ = parse_settings_json(text);
        let _ = bibites_io::from_json(text);
    }
}

#[test]
fn zip_bomb_like_entries_are_bounded() {
    use std::io::Write;
    let dir = std::env::temp_dir().join("rustibites_io_fuzz");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("many_entries.zip");
    let file = std::fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    for index in 0..200 {
        zip.start_file(format!("bibites/bibite_{index}.bb8"), options)
            .unwrap();
        zip.write_all(b"{\"version\":\"0.6.3.1\"}").unwrap();
    }
    zip.finish().unwrap();
    // Must return an error (missing settings/scene), not panic or hang.
    assert!(load_save_bundle(&path).is_err());
    let _ = std::fs::remove_file(path);
}

#[test]
fn valid_real_files_load_and_round_trip() {
    let mut checked = 0;
    if let Ok(entries) = std::fs::read_dir(real("saves")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("zip") {
                continue;
            }
            let bundle = load_save_bundle(&path).expect("real save must load");
            let out = std::env::temp_dir()
                .join("rustibites_io_fuzz")
                .join("roundtrip_save.zip");
            std::fs::create_dir_all(out.parent().unwrap()).unwrap();
            bibites_io::write_save_bundle(&bundle, &out).unwrap();
            let reloaded = load_save_bundle(&out).expect("written save must reload");
            assert_eq!(reloaded.bibites.len(), bundle.bibites.len());
            assert_eq!(reloaded.eggs.len(), bundle.eggs.len());
            assert_eq!(reloaded.data_bin, bundle.data_bin);
            assert_eq!(reloaded.scene, bundle.scene);
            let _ = std::fs::remove_file(out);
            checked += 1;
        }
    }
    for name in ["Ring.zip", "Default.zip", "RadTower.zip"] {
        let path = real(name);
        if !path.is_file() {
            continue;
        }
        let bundle = load_scenario_bundle(&path).expect("real scenario must load");
        let out = std::env::temp_dir()
            .join("rustibites_io_fuzz")
            .join("roundtrip_scn.zip");
        bibites_io::write_scenario_bundle(&bundle, &out).unwrap();
        let reloaded = load_scenario_bundle(&out).expect("written scenario must reload");
        assert_eq!(reloaded.info, bundle.info);
        assert_eq!(reloaded.scenario.zones.len(), bundle.scenario.zones.len());
        let _ = std::fs::remove_file(out);
        checked += 1;
    }
    for dir in [real("templates"), real("templates_current")] {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("bb8template") {
                    continue;
                }
                if let Ok(loaded) = load_template(&path) {
                    assert_eq!(loaded.genome.genes.len(), 35);
                    checked += 1;
                }
            }
        }
    }
    if checked == 0 {
        eprintln!("skipping valid-file round-trip: testdata absent");
    } else {
        eprintln!("valid real-file round-trips checked: {checked}");
    }
}

#[test]
fn truncated_zip_bytes_never_panic() {
    let mut sources = Vec::new();
    for path in [real("saves/lol.zip"), real("Default.zip"), real("Ring.zip")] {
        if path.is_file() {
            sources.push(std::fs::read(&path).unwrap());
        }
    }
    if sources.is_empty() {
        eprintln!("skipping truncated-zip fuzz: testdata absent");
        return;
    }
    let dir = std::env::temp_dir().join(format!("rustibites_io_trunc_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (source_index, bytes) in sources.iter().enumerate() {
        for fraction in [0u8, 1, 10, 25, 50, 75, 90, 99] {
            let cut = bytes.len() * usize::from(fraction) / 100;
            let path = dir.join(format!("cut_{source_index}_{fraction}.zip"));
            std::fs::write(&path, &bytes[..cut]).unwrap();
            let _ = load_save_bundle(&path);
            let _ = load_scenario_bundle(&path);
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn scenario_templates_resolve_against_spawn_rows() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bundles/scenario_min.zip");
    let bundle = load_scenario_bundle(&path).unwrap();
    assert_eq!(bundle.templates.len(), 1);
    let template_name = &bundle.templates[0].0;
    assert_eq!(template_name, "Basic.bb8template");
    assert!(
        bundle
            .scenario
            .bibite_spawns
            .iter()
            .any(|row| row.path == *template_name || row.path.ends_with(template_name.as_str())),
        "spawn row must reference the bundled template"
    );
}

#[test]
fn version_edges_round_trip_or_fail_clearly() {
    // Older supported templates: migrate, re-emit at present version, reload.
    for name in ["template_0.5.bb8template", "template_0.6a6.bb8template"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let loaded = load_template(&path).expect("old template must migrate");
        let text = loaded.to_json().expect("re-emit old template");
        let reloaded = bibites_io::from_json(&text).expect("present-version template must reload");
        assert_eq!(reloaded.genome, loaded.genome);
        assert_eq!(reloaded.brain, loaded.brain);
        assert_eq!(reloaded.meta, loaded.meta);
        assert_eq!(reloaded.version, bibites_io::present_version());
    }
    // Newer-than-present and malformed versions are rejected, not mis-migrated.
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/template_current.bb8template"),
    )
    .unwrap();
    for version in ["9.9.9", "0.6.3.1.2", "not-a-version", ""] {
        let text = source.replacen("\"0.6.3.1\"", &format!("\"{version}\""), 1);
        let error = bibites_io::from_json(&text).expect_err("must reject");
        let message = error.to_string();
        assert!(
            message.contains("version") || message.contains("unsupported"),
            "unexpected error for {version:?}: {message}"
        );
    }
}

//! Integration tests for bibites-io phase I2 (`.bb8settings` + scenario settings
//! + precedence + bridge JSON).
//!
//! Synthetic fixtures live under `tests/fixtures/settings/`; real files under
//! `testdata/real/` are used when present and skipped otherwise.

use std::path::{Path, PathBuf};

use bibites_core::settings::{
    IndependentSettings, MaterialSettings, ScenarioSettings, SimSettings,
};
use bibites_io::settings::{
    SettingsMode, apply_recommended, load_settings_file, parse_scenario_settings,
    parse_settings_json, resolve_effective_settings, serialize_save_settings,
    serialize_scenario_settings, to_bridge_settings_json, write_settings_file,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/settings")
}

fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(fixtures().join(name)).expect("fixture must exist")
}

fn real_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/real")
}

#[test]
fn save_fixture_parses_all_scopes() {
    let file = parse_settings_json(&read_fixture("save_settings.bb8settings")).unwrap();
    assert_eq!(file.scenario, ScenarioSettings::game_defaults());
    assert_eq!(
        file.independents,
        Some(IndependentSettings::game_defaults())
    );
    assert_eq!(file.materials, MaterialSettings::game_defaults());
    assert_eq!(file.zones.len(), 3);
    assert_eq!(file.zones[0].name, "World");
    assert_eq!(file.zones[1].distribution.name(), "Ring");
    assert_eq!(file.zones[1].inside_radius, 0.5);
    assert!(file.zones[1].renew_biomass);
    assert_eq!(file.spawns.len(), 2);
    assert_eq!(file.spawns[0].path, "Fixture.bb8template");
    assert_eq!(file.spawns[0].spawn_priority, 1.5);
    assert_eq!(file.spawns[0].tag, "Fixture");
    assert_eq!(file.spawns[0].spawn_zone, Some(0));
    assert!(file.spawns[1].is_external);
    assert_eq!(file.changers.len(), 1);
    assert_eq!(file.changers[0].name, "Heat");
    assert_eq!(file.changers[0].points.len(), 2);
    assert_eq!(file.zone_groups.len(), 1);
    assert_eq!(file.zone_groups[0].zone_count, 4);
    assert_eq!(file.zone_groups[0].zones.len(), 1);
    let challenge = file.challenge.as_ref().expect("challenge parsed");
    assert_eq!(challenge.exit_condition.count_conditions(), 1);
    assert_eq!(challenge.three_star_condition.count_conditions(), 1);
    assert_eq!(challenge.scoring_metric.metric.name(), "TagCount");
    assert_eq!(challenge.scoring_metric.argument, "Champion");
    assert!(challenge.high_score_is_better);
    // Unknown top-level key and unknown wrapper key are preserved.
    assert!(file.extra.contains_key("futureUnknown"));
    assert!(
        file.scalar_extras
            .get("backgroundMutationChance")
            .is_some_and(|extra| extra.contains_key("unknownWrapperKey"))
    );
}

#[test]
fn save_fixture_round_trips_through_json() {
    let file = parse_settings_json(&read_fixture("save_settings.bb8settings")).unwrap();
    let text = file.to_json(SettingsMode::Save).unwrap();
    let reloaded = parse_settings_json(&text).unwrap();
    assert_eq!(reloaded, file);
    assert!(text.contains("futureUnknown"));
    assert!(text.contains("unknownWrapperKey"));
    assert!(text.contains("futureZoneKey"));
    // Effective settings are stable.
    assert_eq!(reloaded.effective_settings(), file.effective_settings());
}

#[test]
fn scenario_fixture_uses_recommended() {
    let file = parse_scenario_settings(&read_fixture("scenario_settings.bb8settings")).unwrap();
    assert!(file.recommended.is_some());
    assert!(file.independents.is_none());
    let settings = file.effective_settings();
    assert_eq!(settings.independents.simulation_size, 50_000.0);
    assert_eq!(settings.sim_tps, 15);
    // A partial `recommended` block resets absent values to defaults.
    assert_eq!(
        settings.independents.biomass_density,
        IndependentSettings::game_defaults().biomass_density
    );
    let text = file.to_json(SettingsMode::Scenario).unwrap();
    let reloaded = parse_scenario_settings(&text).unwrap();
    assert_eq!(reloaded, file);
    assert!(!text.contains("\"independents\""));
    assert!(text.contains("\"recommended\""));
}

#[test]
fn minimal_file_resolves_to_game_defaults() {
    let file = parse_settings_json(&read_fixture("minimal.bb8settings")).unwrap();
    assert_eq!(file.effective_settings(), SimSettings::game_defaults());
}

#[test]
fn legacy_flat_independents_are_read() {
    let file = parse_settings_json(&read_fixture("legacy_flat.bb8settings")).unwrap();
    let independents = file.independents.clone().expect("flat independents");
    assert_eq!(independents.simulation_size, 2000.0);
    assert_eq!(independents.sim_tps, 30);
    // Unknown legacy top-level keys are preserved.
    assert!(file.extra.contains_key("legacyColorR"));
    let text = file.to_json(SettingsMode::Save).unwrap();
    assert!(text.contains("legacyColorR"));
}

#[test]
fn precedence_and_recommended_semantics() {
    let baseline = SimSettings::game_defaults();
    let mut save = IndependentSettings::game_defaults();
    save.simulation_size = 20_000.0;
    let mut cli = IndependentSettings::game_defaults();
    cli.simulation_size = 5_000.0;
    let mut recommended = IndependentSettings::game_defaults();
    recommended.simulation_size = 50_000.0;

    // Save wins over CLI and recommended.
    let resolved = resolve_effective_settings(
        baseline.scenario.clone(),
        baseline.materials,
        Some(save.clone()),
        Some(cli.clone()),
        Some(recommended.clone()),
    );
    assert_eq!(resolved.independents.simulation_size, 20_000.0);
    // CLI wins over recommended when there is no save.
    let resolved = resolve_effective_settings(
        baseline.scenario.clone(),
        baseline.materials,
        None,
        Some(cli),
        Some(recommended),
    );
    assert_eq!(resolved.independents.simulation_size, 5_000.0);
    // New world from scenario uses recommended.
    let resolved = resolve_effective_settings(
        baseline.scenario.clone(),
        baseline.materials,
        None,
        None,
        Some(save),
    );
    assert_eq!(resolved.independents.simulation_size, 20_000.0);

    // Partial recommended application resets missing keys to defaults.
    let mut settings = SimSettings::game_defaults();
    settings.independents.biomass_density = 0.5;
    settings.independents.simulation_size = 123.0;
    let partial = serde_json::json!({
        "SimulationSize": {"interactableChanged": {}, "Value": 50000.0},
        "simTPS": {"Value": 15}
    });
    let clamped = apply_recommended(&mut settings, &partial).unwrap();
    assert_eq!(clamped, 0);
    assert_eq!(settings.independents.simulation_size, 50_000.0);
    assert_eq!(settings.sim_tps, 15);
    assert_eq!(
        settings.independents.biomass_density,
        IndependentSettings::game_defaults().biomass_density
    );
}

#[test]
fn bridge_json_shape_and_types() {
    let settings = SimSettings::game_defaults();
    let bridge = to_bridge_settings_json(&settings);
    let scenario = bridge["scenario"].as_object().expect("scenario object");
    let independent = bridge["independent"]
        .as_object()
        .expect("independent object");
    assert_eq!(scenario.len(), 78);
    assert_eq!(independent.len(), 14);
    // Exact game names are present.
    for key in [
        "backgroundMutationChance",
        "pelletEnergy",
        "globalZoneSpeed",
    ] {
        assert!(scenario.contains_key(key), "missing {key}");
    }
    for key in [
        "SimulationSize",
        "simTPS",
        "brainTPS",
        "visionLookupUpdateFactor",
    ] {
        assert!(independent.contains_key(key), "missing {key}");
    }
    // Tags: bool stays bool, int stays int, float stays float-number.
    assert!(scenario["pelletCollision"].is_boolean());
    assert!(scenario["backgroundMutationChance"].is_number());
    assert!(independent["simTPS"].is_i64(), "simTPS must be an int");
    assert!(independent["SimulationSize"].is_number());
}

#[test]
fn out_of_range_values_follow_game_setter_clamp_rules() {
    let file = parse_settings_json(&read_fixture("out_of_range.bb8settings")).unwrap();
    // `minPelletSize` is hard-clamped by the game's setter (canGoOutOfBounds=false).
    assert!(file.clamped_fields.iter().any(|key| key == "minPelletSize"));
    assert_eq!(file.scenario.min_pellet_size, 10.0);
    // `pelletEnergy` is not hard-clamped: the game's JSON load preserves values.
    assert!(!file.clamped_fields.iter().any(|key| key == "pelletEnergy"));
    assert_eq!(file.scenario.pellet_energy, 99_999.0);
}

#[test]
fn error_cases_are_rejected() {
    assert!(parse_settings_json(&read_fixture("truncated.bb8settings")).is_err());
    assert!(parse_settings_json(&read_fixture("wrong_type.bb8settings")).is_err());
    assert!(parse_settings_json("[]").is_err());
}

#[test]
fn serialize_and_write_from_sim_settings() {
    let settings = SimSettings::game_defaults();
    let save_text = serialize_save_settings(&settings).unwrap();
    let parsed = parse_settings_json(&save_text).unwrap();
    assert_eq!(parsed.effective_settings(), settings);
    assert!(save_text.contains("\"independents\""));

    let scenario_text = serialize_scenario_settings(&settings).unwrap();
    let parsed = parse_scenario_settings(&scenario_text).unwrap();
    assert!(scenario_text.contains("\"recommended\""));
    assert!(!scenario_text.contains("\"independents\""));
    assert_eq!(parsed.effective_settings(), settings);

    let dir = std::env::temp_dir().join("rustibites_io_i2");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("written.bb8settings");
    write_settings_file(&path, &settings).unwrap();
    let loaded = load_settings_file(&path).unwrap();
    assert_eq!(loaded, settings);
    let _ = std::fs::remove_file(path);
}

#[test]
fn real_legacy_settings_if_present() {
    let path = real_dir().join("Storme's Super Sparse.bb8settings");
    if !path.exists() {
        eprintln!(
            "skipping real settings test: {} not present",
            path.display()
        );
        return;
    }
    let text = std::fs::read_to_string(&path).unwrap();
    let file = parse_settings_json(&text).unwrap();
    // Legacy 0.5-era names are unknown to the current model and preserved.
    assert!(
        !file.extra.is_empty(),
        "legacy keys should be preserved as extras"
    );
    let effective = file.effective_settings();
    assert_eq!(effective.sim_tps, 30);
    // Round-trip keeps the unknown legacy keys.
    let text_out = file.to_json(SettingsMode::Save).unwrap();
    assert!(text_out.contains("mutationChance") || text_out.contains("colorR"));
    eprintln!("exercised real legacy settings {}", path.display());
}

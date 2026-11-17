//! Settings compatibility/round-trip audit (new development wave).
//!
//! * every scenario/independent/material setting survives save- and
//!   scenario-shaped `.bb8settings` serialization;
//! * the bridge `/settings` JSON has exactly the game's key sets and JSON types;
//! * unknown keys are preserved.

use std::path::{Path, PathBuf};

use bibites_core::settings::{
    IndependentSettings, MaterialKind, MaterialSetting, MaterialSettings, ScenarioSettings,
    SettingKind, SettingValue, SimSettings,
};
use bibites_io::settings::{
    SettingsMode, parse_settings_json, serialize_save_settings, serialize_scenario_settings,
    to_bridge_settings_json,
};

fn altered(meta: &'static bibites_core::settings::SettingMeta) -> SettingValue {
    match meta.kind {
        SettingKind::Bool => {
            let current = matches!(meta.default, SettingValue::Bool(true));
            SettingValue::Bool(!current)
        }
        SettingKind::Int => {
            let default = match meta.default {
                SettingValue::Int(value) => value,
                other => other.as_f64() as i32,
            };
            let mut value = default.saturating_add(1);
            if let Some(max) = meta.max {
                if f64::from(value) > max {
                    value = default.saturating_sub(1);
                }
            }
            SettingValue::Int(value)
        }
        SettingKind::Float => {
            let default = match meta.default {
                SettingValue::Float(value) => value,
                other => other.as_f64() as f32,
            };
            let value = match (meta.min, meta.max) {
                (Some(min), Some(max)) => ((min + max) * 0.5) as f32,
                _ => (default * 1.05).max(default + 0.5),
            };
            SettingValue::Float(value)
        }
    }
}

fn modified_settings() -> SimSettings {
    let mut settings = SimSettings::game_defaults();
    for meta in ScenarioSettings::SETTINGS {
        settings.scenario.set_raw(meta.key, altered(meta));
    }
    for meta in IndependentSettings::SETTINGS {
        settings.independents.set_raw(meta.key, altered(meta));
    }
    for kind in MaterialKind::ALL {
        let material = settings.materials.get_mut(kind);
        for meta in MaterialSetting::FIELDS {
            // freshTime/decayRate are only written for decaying materials.
            if matches!(meta.key, "freshTime" | "decayRate") && !material.decay {
                continue;
            }
            material.set_raw(meta.key, altered(meta));
        }
    }
    settings
}

fn assert_scene_equal(expected: &SimSettings, actual: &SimSettings) {
    for meta in ScenarioSettings::SETTINGS {
        assert_eq!(
            expected.scenario.get_by_key(meta.key),
            actual.scenario.get_by_key(meta.key),
            "scenario {}",
            meta.key
        );
    }
    for meta in IndependentSettings::SETTINGS {
        assert_eq!(
            expected.independents.get_by_key(meta.key),
            actual.independents.get_by_key(meta.key),
            "independent {}",
            meta.key
        );
    }
    for kind in MaterialKind::ALL {
        for meta in MaterialSetting::FIELDS {
            if matches!(meta.key, "freshTime" | "decayRate") && !expected.materials.get(kind).decay
            {
                continue;
            }
            assert_eq!(
                expected.materials.get(kind).get_by_key(meta.key),
                actual.materials.get(kind).get_by_key(meta.key),
                "material {} {}",
                kind.name(),
                meta.key
            );
        }
    }
}

#[test]
fn every_setting_round_trips_through_save_and_scenario_files() {
    let expected = modified_settings();
    for (mode, text) in [
        (
            SettingsMode::Save,
            serialize_save_settings(&expected).unwrap(),
        ),
        (
            SettingsMode::Scenario,
            serialize_scenario_settings(&expected).unwrap(),
        ),
    ] {
        let parsed = parse_settings_json(&text).unwrap();
        let mut actual = parsed.effective_settings();
        if mode == SettingsMode::Scenario {
            // A scenario file's recommended block is partial in real files; ours
            // writes all 14 values, but absent ones reset to defaults.
            actual.independents = parsed.recommended.clone().unwrap_or_default();
        }
        assert_scene_equal(&expected, &actual);
    }
}

#[test]
fn bridge_settings_keys_and_types_match_the_game_capture() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/parity/captures/ring_seed12345/pass-0/manifest.json");
    let bridge = to_bridge_settings_json(&SimSettings::game_defaults());
    let scenario = bridge["scenario"].as_object().unwrap();
    let independent = bridge["independent"].as_object().unwrap();
    assert_eq!(scenario.len(), 78);
    assert_eq!(independent.len(), 14);
    for meta in ScenarioSettings::SETTINGS {
        let value = &scenario[meta.key];
        assert_eq!(
            value.is_boolean(),
            meta.kind == SettingKind::Bool,
            "{}",
            meta.key
        );
        assert_eq!(
            value.is_i64(),
            meta.kind == SettingKind::Int,
            "{}",
            meta.key
        );
        assert_eq!(
            value.is_number(),
            meta.kind != SettingKind::Bool,
            "{}",
            meta.key
        );
    }
    if !manifest.is_file() {
        eprintln!("skipping game-manifest key comparison: capture absent");
        return;
    }
    let game: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest).unwrap()).unwrap();
    for scope in ["scenario", "independent"] {
        let game_keys: std::collections::BTreeSet<&String> = game["settings"][scope]
            .as_object()
            .unwrap()
            .keys()
            .collect();
        let io_keys: std::collections::BTreeSet<&String> =
            bridge[scope].as_object().unwrap().keys().collect();
        assert_eq!(
            game_keys, io_keys,
            "{scope} key set differs from the game capture"
        );
    }
}

#[test]
fn unknown_settings_keys_survive_round_trip() {
    let settings = SimSettings::game_defaults();
    let mut text = serialize_save_settings(&settings).unwrap();
    text = text.replacen('{', "{\n \"futureTopLevel\": {\"x\": 1},", 1);
    let parsed = parse_settings_json(&text).unwrap();
    assert!(parsed.extra.contains_key("futureTopLevel"));
    let out = parsed.to_json(SettingsMode::Save).unwrap();
    assert!(out.contains("futureTopLevel"));
    let reloaded = parse_settings_json(&out).unwrap();
    assert_eq!(reloaded.effective_settings(), settings);
}

#[test]
fn material_defaults_are_stable() {
    let defaults = MaterialSettings::game_defaults();
    assert_eq!(defaults.get(MaterialKind::Plant).cohesiveness, 10.0);
    assert_eq!(defaults.get(MaterialKind::Meat).fresh_time, 2000.0);
    assert_eq!(defaults.get(MaterialKind::Fat).energy_density, 30.0);
    assert_eq!(defaults.get(MaterialKind::Armor).mass_density, 0.05);
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/settings/save_settings.bb8settings");
    let parsed = parse_settings_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(parsed.materials, defaults);
}

#[test]
fn setting_metadata_is_internally_consistent() {
    use bibites_core::settings::ScenarioSettings;
    let mut keys = std::collections::BTreeSet::new();
    for meta in ScenarioSettings::SETTINGS {
        assert!(keys.insert(meta.key), "duplicate scenario key {}", meta.key);
        assert!(!meta.label.is_empty(), "{} has no label", meta.key);
        assert_eq!(meta.default.kind(), meta.kind, "{} default kind", meta.key);
        if meta.kind == SettingKind::Bool {
            assert!(
                meta.min.is_none() && meta.max.is_none(),
                "{} bool range",
                meta.key
            );
        } else if let (Some(min), Some(max)) = (meta.min, meta.max) {
            assert!(min <= max, "{} range inverted", meta.key);
            let value = meta.default.as_f64();
            assert!(
                (min..=max).contains(&value) || !meta.clamped,
                "{} default {value} outside clamped range {min}..={max}",
                meta.key
            );
        }
    }
    assert_eq!(ScenarioSettings::SETTINGS.len(), 78);
    assert_eq!(IndependentSettings::SETTINGS.len(), 14);
    for meta in MaterialSetting::FIELDS {
        assert_eq!(meta.default.kind(), meta.kind, "material {}", meta.key);
    }
    assert_eq!(MaterialSetting::FIELDS.len(), 9);
    assert_eq!(MaterialKind::ALL.len(), 4);
    // Defaults must serialize to bridge JSON with exactly the metadata defaults.
    let bridge = to_bridge_settings_json(&SimSettings::game_defaults());
    for meta in ScenarioSettings::SETTINGS {
        let value = &bridge["scenario"][meta.key];
        match meta.default {
            SettingValue::Bool(expected) => assert_eq!(value.as_bool(), Some(expected)),
            SettingValue::Int(expected) => assert_eq!(value.as_i64(), Some(i64::from(expected))),
            SettingValue::Float(expected) => {
                assert_eq!(value.as_f64().map(|v| v as f32), Some(expected));
            }
        }
    }
    for meta in IndependentSettings::SETTINGS {
        let value = &bridge["independent"][meta.key];
        match meta.default {
            SettingValue::Bool(expected) => assert_eq!(value.as_bool(), Some(expected)),
            SettingValue::Int(expected) => assert_eq!(value.as_i64(), Some(i64::from(expected))),
            SettingValue::Float(expected) => {
                assert_eq!(value.as_f64().map(|v| v as f32), Some(expected));
            }
        }
    }
}

#[test]
fn malformed_settings_json_never_panics() {
    for text in [
        "",
        "{",
        "}",
        "[]",
        "null",
        "42",
        "{\"backgroundMutationChance\": {\"Value\": \"not a number\"}}",
        "{\"backgroundMutationChance\": {\"Value\": true}}",
        "{\"pelletCollision\": 5}",
        "{\"SimulationSize\": {\"Value\": -1}}",
        "{\"future\": {\"deep\": [[[[1]]]]}}",
        "{\"backgroundMutationChance\": {\"Value\": 1.5}, \"backgroundMutationChance\": {\"Value\": 2.5}}",
        "{\"materials\": [], \"zones\": {}, \"bibites\": 3}",
        "\u{feff}{\"backgroundMutationChance\": {\"Value\": 1.5}}",
    ] {
        let _ = parse_settings_json(text);
    }
}

#[test]
fn material_ranges_and_conditional_fields_are_stable() {
    let settings = SimSettings::game_defaults();
    let text = serialize_save_settings(&settings).unwrap();
    let parsed = parse_settings_json(&text).unwrap();
    // Plant/Fat/Armor do not decay: no freshTime/decayRate in their file object.
    let raw: serde_json::Value = serde_json::from_str(&text).unwrap();
    for name in ["PlantSettings", "FatSettings", "ArmorSettings"] {
        assert!(
            raw["materials"][name].get("freshTime").is_none(),
            "{name} freshTime"
        );
        assert!(
            raw["materials"][name].get("decayRate").is_none(),
            "{name} decayRate"
        );
    }
    assert!(raw["materials"]["MeatSettings"].get("freshTime").is_some());
    assert_eq!(parsed.materials, settings.materials);
}

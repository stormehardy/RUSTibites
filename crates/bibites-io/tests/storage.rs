//! Filesystem persistence used by the viewer save/load/scenario UI.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use bibites_io::storage::{
    StoragePaths, UserPreferences, cleanup_autosaves, delete_file, export_file, import_scenario,
    list_saves, list_scenarios, list_templates, load_user_preferences, rename_file,
    sanitize_file_name, save_as, save_user_preferences,
};

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "rustibites_storage_{label}_{}_{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn save_fixture() -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/real/saves/lol.zip");
    path.is_file().then_some(path)
}

fn scenario_fixture() -> Option<PathBuf> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bundles/scenario_min.zip");
    path.is_file().then_some(path)
}

#[test]
fn save_as_list_and_name_validation() {
    let Some(fixture) = save_fixture() else {
        return;
    };
    let root = temp_root("save_as");
    let paths = StoragePaths::new(&root);
    paths.ensure_layout().unwrap();
    assert!(list_saves(&paths, true).unwrap().is_empty());

    let one = save_as(&fixture, &paths, "first", false).unwrap();
    assert!(one.is_file());
    assert_eq!(one.extension().unwrap(), "zip");
    let listed = list_saves(&paths, false).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "first.zip");
    assert!(!listed[0].autosave);

    // Overwrite is opt-in (the UI asks first).
    assert!(save_as(&fixture, &paths, "first", false).is_err());
    save_as(&fixture, &paths, "first", true).unwrap();
    assert_eq!(list_saves(&paths, false).unwrap().len(), 1);

    assert!(sanitize_file_name("", "zip").is_err());
    assert!(sanitize_file_name("a/b", "zip").is_err());
    assert!(sanitize_file_name(".hidden", "zip").is_err());
    assert_eq!(sanitize_file_name("ok", "zip").unwrap(), "ok.zip");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn autosaves_sorted_separately_and_cleanup_keeps_newest() {
    let Some(fixture) = save_fixture() else {
        return;
    };
    let root = temp_root("autosave");
    let paths = StoragePaths::new(&root);
    paths.ensure_layout().unwrap();
    for index in 0..5 {
        let name = format!("autosave_2026010100000{index}.zip");
        std::fs::copy(&fixture, paths.autosaves_dir().join(name)).unwrap();
    }
    let saves = list_saves(&paths, true).unwrap();
    assert_eq!(saves.len(), 5);
    assert!(saves.iter().all(|save| save.autosave));
    assert_eq!(cleanup_autosaves(&paths, 2).unwrap(), 3);
    assert_eq!(list_saves(&paths, true).unwrap().len(), 2);
    let _ = std::fs::remove_dir_all(root);
}

/// Sets a file's modification time to `hours_ago` hours before now.
fn set_age(path: &Path, hours_ago: u64) {
    let when = SystemTime::now() - std::time::Duration::from_secs(hours_ago * 3600);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

fn autosave_names(paths: &StoragePaths) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(paths.autosaves_dir())
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("autosave_"))
        .collect();
    names.sort();
    names
}

#[test]
fn autosave_keeps_the_limit_plus_the_new_file_like_the_game() {
    let Some(fixture) = save_fixture() else {
        return;
    };
    let root = temp_root("autosave_limit");
    let browser = bibites_io::storage::SaveBrowser::new(StoragePaths::new(&root));
    let contents = bibites_io::load_world_contents(&fixture, None).unwrap();
    browser.paths.ensure_layout().unwrap();
    let schedule = bibites_io::storage::AutosaveSchedule {
        enabled: true,
        real_time: true,
        period_minutes: 1,
        max_files: 3,
    };
    // Three old autosaves (oldest first). The game cleans up to the limit *before* it writes, so
    // in steady state `max_files` + the file just written stay.
    for index in 0..3 {
        let path = browser
            .paths
            .autosaves_dir()
            .join(format!("autosave_2026010100000{index}.zip"));
        std::fs::copy(&fixture, &path).unwrap();
        set_age(&path, 10 - index);
    }
    // Below the limit + 1: nothing is removed.
    let first = browser
        .autosave_file("20260101010000", &contents, schedule)
        .unwrap();
    assert!(first.is_file());
    assert_eq!(autosave_names(&browser.paths).len(), 4);
    // At the limit + 1 each run removes the oldest and adds the new one: 4 stay.
    let second = browser
        .autosave_file("20260101010100", &contents, schedule)
        .unwrap();
    assert!(second.is_file());
    let names = autosave_names(&browser.paths);
    assert_eq!(names.len(), 4, "{names:?}");
    assert!(
        !names.contains(&"autosave_20260101000000.zip".to_owned()),
        "the oldest goes first: {names:?}"
    );
    // A limit of 0 disables the cleanup entirely.
    let unlimited = bibites_io::storage::AutosaveSchedule {
        max_files: 0,
        ..schedule
    };
    browser
        .autosave_file("20260101010200", &contents, unlimited)
        .unwrap();
    assert_eq!(autosave_names(&browser.paths).len(), 5);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn autosave_cleanup_never_removes_more_than_one_file_or_foreign_files() {
    let Some(fixture) = save_fixture() else {
        return;
    };
    let root = temp_root("autosave_guard");
    let browser = bibites_io::storage::SaveBrowser::new(StoragePaths::new(&root));
    let contents = bibites_io::load_world_contents(&fixture, None).unwrap();
    browser.paths.ensure_layout().unwrap();
    // A big history (the game's own folder can hold over a thousand), oldest first, plus a zip
    // that is not an autosave and is older than all of them.
    for index in 0..9 {
        let path = browser
            .paths
            .autosaves_dir()
            .join(format!("autosave_2026010100000{index}.zip"));
        std::fs::copy(&fixture, &path).unwrap();
        set_age(&path, 20 - index);
    }
    let foreign = browser.paths.autosaves_dir().join("keep_me.zip");
    std::fs::copy(&fixture, &foreign).unwrap();
    set_age(&foreign, 100);
    // A misread limit of 1 must not wipe the history in one pass.
    let schedule = bibites_io::storage::AutosaveSchedule {
        enabled: true,
        real_time: true,
        period_minutes: 1,
        max_files: 1,
    };
    browser
        .autosave_file("20260101020000", &contents, schedule)
        .unwrap();
    // 9 old + the new one = 10, minus one removed.
    assert_eq!(autosave_names(&browser.paths).len(), 9);
    assert!(foreign.is_file(), "a file that is not an autosave stays");
    // The uncapped helper still follows the game (everything beyond the newest N, autosaves only).
    assert_eq!(cleanup_autosaves(&browser.paths, 2).unwrap(), 7);
    assert_eq!(autosave_names(&browser.paths).len(), 2);
    assert!(foreign.is_file());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn private_files_live_in_the_state_folder_not_the_content_root() {
    let content = temp_root("content");
    let state = temp_root("state");
    let paths = StoragePaths::new(&content).with_state_dir(&state);
    assert_eq!(paths.saves_dir(), content.join("Savefiles"));
    assert_eq!(paths.bibites_dir(), content.join("Bibites"));
    assert_eq!(
        paths.templates_dir(),
        content.join("Bibites").join("Templates")
    );
    assert_eq!(paths.preferences_path(), state.join("user_settings.json"));
    assert_eq!(
        paths.challenge_progress_path(),
        state.join("challenge_progress.json")
    );
    assert_eq!(paths.thumbnails_dir(), state.join("thumbnails"));
    assert_eq!(paths.workshop_dir(), state.join("Workshop"));
    save_user_preferences(&paths, &UserPreferences::default()).unwrap();
    assert!(state.join("user_settings.json").is_file());
    assert!(!content.join("user_settings.json").exists());
    // Without a separate state folder everything shares the root, as before.
    let single = StoragePaths::new(&content);
    assert_eq!(single.state_dir(), content.as_path());
    assert_eq!(
        single.preferences_path(),
        content.join("user_settings.json")
    );
    let _ = std::fs::remove_dir_all(content);
    let _ = std::fs::remove_dir_all(state);
}

#[test]
fn content_root_is_the_games_folder_when_installed_and_not_overridden() {
    if std::env::var_os("RUSTIBITES_DATA_DIR").is_some() {
        return;
    }
    let content = StoragePaths::default_content();
    match StoragePaths::game_root_guess() {
        Some(game) => {
            assert_eq!(content.root, game);
            assert_eq!(
                content.state_dir(),
                StoragePaths::default_local().root.as_path()
            );
        }
        None => assert_eq!(content, StoragePaths::default_local()),
    }
}

#[test]
fn scenario_import_validates_then_lists() {
    let Some(fixture) = scenario_fixture() else {
        return;
    };
    let root = temp_root("scenario");
    let paths = StoragePaths::new(&root);
    paths.ensure_layout().unwrap();
    assert!(import_scenario(&fixture, &paths, "ring", false).is_ok());
    let listed = list_scenarios(&paths).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "ring.zip");
    // A save is not a scenario bundle.
    if let Some(save) = save_fixture() {
        assert!(import_scenario(&save, &paths, "bad", false).is_err());
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn export_rename_delete_template_listing() {
    let Some(fixture) = save_fixture() else {
        return;
    };
    let root = temp_root("files");
    let paths = StoragePaths::new(&root);
    paths.ensure_layout().unwrap();
    let saved = save_as(&fixture, &paths, "original", false).unwrap();
    let exported = root.join("exported.zip");
    export_file(&saved, &exported).unwrap();
    assert!(exported.is_file());
    let renamed = rename_file(&saved, "renamed").unwrap();
    assert_eq!(renamed.file_name().unwrap(), "renamed.zip");
    assert!(!saved.exists());
    delete_file(&renamed).unwrap();
    assert!(list_saves(&paths, false).unwrap().is_empty());

    // Template listing only sees .bb8template files.
    std::fs::write(paths.templates_dir().join("demo.bb8template"), b"{}").unwrap();
    std::fs::write(paths.templates_dir().join("ignore.txt"), b"x").unwrap();
    let templates = list_templates(&paths).unwrap();
    assert_eq!(templates.len(), 1);
    assert_eq!(templates[0].name, "demo.bb8template");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn user_preferences_round_trip_with_defaults() {
    let root = temp_root("prefs");
    let paths = StoragePaths::new(&root);
    assert_eq!(
        load_user_preferences(&paths).unwrap(),
        UserPreferences::default()
    );
    let prefs = UserPreferences {
        include_autosaves_in_load_panel: true,
        autosave_period_minutes: 5,
        autosave_max_files: 3,
        pause_after_load: true,
        ..UserPreferences::default()
    };
    save_user_preferences(&paths, &prefs).unwrap();
    assert_eq!(load_user_preferences(&paths).unwrap(), prefs);
    // Unknown/missing fields fall back to defaults.
    std::fs::write(paths.preferences_path(), br#"{"autosave":false}"#).unwrap();
    let loaded = load_user_preferences(&paths).unwrap();
    assert!(!loaded.autosave);
    assert_eq!(loaded.autosave_period_minutes, 10);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn save_browser_save_load_quick_slot_and_fingerprint() {
    let Some(fixture) = save_fixture() else {
        return;
    };
    let root = temp_root("browser");
    let browser = bibites_io::storage::SaveBrowser::new(StoragePaths::new(&root));
    let contents = bibites_io::load_world_contents(&fixture, None).unwrap();
    let original = bibites_core::world::World::from_contents(contents.clone()).unwrap();
    let original_fp = bibites_core::sim::state_fingerprint(&original);

    let saved = browser
        .save_world("browser_save", &contents, false)
        .unwrap();
    assert!(saved.is_file());
    assert!(
        browser
            .save_world("browser_save", &contents, false)
            .is_err()
    );
    let loaded = browser.load_world(&saved).unwrap();
    let reloaded = bibites_core::world::World::from_contents(loaded).unwrap();
    assert_eq!(original_fp, bibites_core::sim::state_fingerprint(&reloaded));

    // Quick save/load slot.
    let quick = browser.quick_save(&contents).unwrap();
    assert!(quick.ends_with("quick.zip"));
    let quick_contents = browser.quick_load().unwrap();
    let quick_world = bibites_core::world::World::from_contents(quick_contents).unwrap();
    assert_eq!(
        original_fp,
        bibites_core::sim::state_fingerprint(&quick_world)
    );

    // Refresh/delete/rename keep the list coherent.
    assert_eq!(browser.refresh_saves(false).unwrap().len(), 2);
    let (renamed, list) = browser.rename_save(&saved, "renamed", false).unwrap();
    assert_eq!(list.len(), 2);
    assert!(list.iter().any(|file| file.path == renamed));
    let after_delete = browser.delete_save(&quick, false).unwrap();
    assert_eq!(after_delete.len(), 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn save_list_entries_metadata_thumbnails_and_autosave() {
    let Some(fixture) = save_fixture() else {
        return;
    };
    let root = temp_root("entries");
    let browser = bibites_io::storage::SaveBrowser::new(StoragePaths::new(&root));
    let contents = bibites_io::load_world_contents(&fixture, None).unwrap();
    let (saved, entries) = browser
        .save_world_with_list("meta", &contents, false, true)
        .unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.name, "meta.zip");
    assert_eq!(entry.path, saved);
    assert!(entry.size > 0);
    assert!(entry.modified.is_some());
    assert!(!entry.autosave);
    assert!(entry.version.is_some());
    assert!(entry.sim_time.is_some());
    assert!(entry.population.is_some());

    // Thumbnail extraction (fixture save has img.png).
    let thumb = bibites_io::storage::extract_thumbnail(&saved, &browser.paths).unwrap();
    let entries = browser.refresh_save_entries(false).unwrap();
    assert_eq!(entries[0].thumbnail.is_some(), thumb.is_some());

    // Autosave naming + retention + refreshed rows.
    let schedule =
        bibites_io::storage::AutosaveSchedule::from_preferences(&UserPreferences::default());
    let (auto_path, entries) = browser
        .autosave("20260101000000", &contents, schedule)
        .unwrap();
    assert!(auto_path.ends_with("autosave_20260101000000.zip"));
    assert!(entries.iter().any(|entry| entry.autosave));
    assert!(bibites_io::storage::autosave_file_name("nope").is_err());

    // Overwrite/delete/rename refreshed lists.
    let (path, entries) = browser
        .save_world_with_list("meta", &contents, true, true)
        .unwrap();
    assert_eq!(path.file_name().unwrap(), "meta.zip");
    assert!(entries.iter().any(|entry| entry.name == "meta.zip"));
    let (renamed, entries) = browser
        .rename_save_with_list(&path, "renamed", true)
        .unwrap();
    assert_eq!(renamed.file_name().unwrap(), "renamed.zip");
    assert!(entries.iter().any(|entry| entry.name == "renamed.zip"));
    let entries = browser.delete_save_with_list(&renamed, true).unwrap();
    assert!(entries.iter().all(|entry| entry.name != "renamed.zip"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_long_list_of_saves_reads_the_metadata_of_its_first_rows_only() {
    let Some(fixture) = save_fixture() else {
        return;
    };
    let root = temp_root("entries_head");
    let browser = bibites_io::storage::SaveBrowser::new(StoragePaths::new(&root));
    let contents = bibites_io::load_world_contents(&fixture, None).unwrap();
    for name in ["old", "middle", "new"] {
        browser.save_world(name, &contents, false).unwrap();
        // Distinct modification times: the list is newest first.
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    let all = browser.refresh_save_entries(false).unwrap();
    assert_eq!(all.len(), 3);
    assert!(all.iter().all(|entry| entry.version.is_some()));
    let head = browser.refresh_save_entries_head(false, 2).unwrap();
    // The same rows in the same order; the metadata of the first two, none for the rest.
    assert_eq!(
        head.iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        all.iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(&head[..2], &all[..2]);
    assert!(
        head[2].version.is_none() && head[2].sim_time.is_none() && head[2].population.is_none()
    );
    assert_eq!(head[2].size, all[2].size);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn scenario_settings_update_preserves_bundle_and_file_manager_errors() {
    let Some(fixture) = scenario_fixture() else {
        return;
    };
    let root = temp_root("scenario_settings");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("scenario.zip");
    std::fs::copy(&fixture, &path).unwrap();
    let before = bibites_io::load_scenario_bundle(&path).unwrap();
    let mut settings = before.settings_file.clone();
    let mut independents = settings.recommended.clone().unwrap_or_default();
    independents.sim_tps = 42;
    settings.recommended = Some(independents.clone());
    settings.independents = Some(independents);
    bibites_io::storage::update_scenario_settings(&path, &settings).unwrap();
    let after = bibites_io::load_scenario_bundle(&path).unwrap();
    assert_eq!(after.settings.independents.sim_tps, 42);
    // Non-settings members survive the in-place rewrite.
    assert_eq!(after.info, before.info);
    assert_eq!(after.template_bytes.len(), before.template_bytes.len());
    assert!(bibites_io::storage::open_in_file_manager(&root.join("missing")).is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn template_entries_expose_library_metadata() {
    let root = temp_root("library");
    let paths = StoragePaths::new(&root);
    paths.ensure_layout().unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/template_current.bb8template");
    if fixture.is_file() {
        std::fs::copy(&fixture, paths.templates_dir().join("demo.bb8template")).unwrap();
    } else {
        std::fs::write(
            paths.templates_dir().join("demo.bb8template"),
            br#"{"name":"Demo","description":"A demo","version":"0.6.3.1","genes":{"genes":{}},"nodes":[],"synapses":[]}"#,
        )
        .unwrap();
    }
    std::fs::write(
        paths.templates_dir().join("genome.bb8"),
        br#"{"name":"GenomeOnly","version":"0.6.3.1","genes":{"genes":{}},"nodes":[],"synapses":[]}"#,
    )
    .unwrap();
    let entries = bibites_io::storage::list_template_entries(&paths).unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| entry.size > 0));
    assert!(entries.iter().all(|entry| entry.path.is_file()));
    let template = entries
        .iter()
        .find(|entry| entry.kind.as_str() == "template")
        .unwrap();
    assert!(!template.name.is_empty());
    let genome = entries
        .iter()
        .find(|entry| entry.kind.as_str() == "genome")
        .unwrap();
    assert_eq!(genome.name, "GenomeOnly");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn create_scenario_and_new_game_contents_round_trip() {
    let Some(fixture) = scenario_fixture() else {
        return;
    };
    let root = temp_root("new_game");
    std::fs::create_dir_all(&root).unwrap();
    let base = bibites_io::load_scenario_bundle(&fixture).unwrap();
    let path = root.join("new_scenario.zip");
    bibites_io::storage::create_scenario(
        &path,
        "My New Scenario",
        "created from options",
        &base.settings_file,
        base.scenario.clone(),
    )
    .unwrap();
    let saved = bibites_io::load_scenario_bundle(&path).unwrap();
    assert_eq!(saved.info.name, "My New Scenario");
    assert_eq!(saved.info.description, "created from options");
    assert!(!saved.info.is_official);
    let contents = bibites_io::storage::new_game_contents(&path, 1234).unwrap();
    assert_eq!(contents.seed, 1234);
    assert_eq!(contents.zones.len(), saved.scenario.zones.len());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn viewer_preferences_cover_ui_visual_and_camera_fields_and_reset() {
    let root = temp_root("prefs_ui");
    let paths = StoragePaths::new(&root);
    let custom = UserPreferences {
        ui_scale: 1.25,
        fullscreen: true,
        resolution_width: 2560,
        resolution_height: 1440,
        background_mode: "space".into(),
        show_grid: true,
        show_node_names: false,
        show_pheromones: false,
        pheromone_opacity: 0.5,
        min_fps: 60,
        camera_zoom: 2.0,
        camera_follow_selection: true,
        camera_pan_speed: 1.5,
        selection_outline: false,
        include_autosaves_in_load_panel: true,
        autosave_period_minutes: 5,
        ..UserPreferences::default()
    };
    save_user_preferences(&paths, &custom).unwrap();
    assert_eq!(load_user_preferences(&paths).unwrap(), custom);
    let reset = UserPreferences::reset(&paths).unwrap();
    assert_eq!(reset, UserPreferences::default());
    assert_eq!(
        load_user_preferences(&paths).unwrap(),
        UserPreferences::default()
    );
    // Missing viewer fields fall back to defaults.
    std::fs::write(paths.preferences_path(), br#"{"ui_scale":2.0}"#).unwrap();
    let loaded = load_user_preferences(&paths).unwrap();
    assert_eq!(loaded.ui_scale, 2.0);
    assert_eq!(loaded.min_fps, 15);
    assert!(!loaded.show_node_names);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn local_workshop_browser_import_list_export_delete() {
    let root = temp_root("workshop");
    let paths = StoragePaths::new(&root);
    let source_dir = root.join("incoming/MyItem");
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::write(source_dir.join("readme.txt"), b"hello").unwrap();
    std::fs::write(root.join("incoming/pack.zip"), b"zipdata").unwrap();

    let (imported, entries) = bibites_io::storage::SaveBrowser::new(paths.clone())
        .import_workshop(&source_dir, "MyItem", false)
        .unwrap();
    assert!(imported.is_dir());
    assert!(
        entries
            .iter()
            .any(|entry| entry.name == "MyItem" && entry.folder)
    );
    assert!(entries.iter().any(|entry| entry.name == "MyItem"));
    let file = root.join("incoming/pack.zip");
    let (pack, entries) = bibites_io::storage::SaveBrowser::new(paths.clone())
        .import_workshop(&file, "pack", false)
        .unwrap();
    assert!(pack.is_file());
    assert!(
        entries
            .iter()
            .any(|entry| entry.name == "pack.zip" && entry.kind == "archive")
    );
    // Overwrite policy.
    assert!(
        bibites_io::storage::SaveBrowser::new(paths.clone())
            .import_workshop(&file, "pack", false)
            .is_err()
    );
    // Export + delete.
    let exported = root.join("out/MyItem");
    bibites_io::storage::export_workshop_item(&imported, &exported).unwrap();
    assert!(exported.join("readme.txt").is_file());
    let entries = bibites_io::storage::SaveBrowser::new(paths.clone())
        .delete_workshop(&imported)
        .unwrap();
    assert!(entries.iter().all(|entry| entry.name != "MyItem"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn hotkeys_defaults_override_reset_and_workshop_metadata() {
    let root = temp_root("hotkeys");
    let paths = StoragePaths::new(&root);
    let defaults = UserPreferences::default();
    assert_eq!(defaults.hotkey("pause"), Some("Space"));
    assert_eq!(defaults.hotkey("screenshot"), Some("F12"));
    assert_eq!(defaults.hotkeys.len(), 12);

    let mut custom = defaults.clone();
    custom.set_hotkey("pause", "K").unwrap();
    save_user_preferences(&paths, &custom).unwrap();
    let loaded = load_user_preferences(&paths).unwrap();
    assert_eq!(loaded.hotkey("pause"), Some("K"));
    let reset = UserPreferences::reset(&paths).unwrap();
    assert_eq!(reset.hotkey("pause"), Some("Space"));

    // Workshop metadata + rename.
    std::fs::create_dir_all(paths.workshop_dir().join("item")).unwrap();
    let item = paths.workshop_dir().join("item");
    let metadata = bibites_io::storage::WorkshopItemMetadata {
        name: "Display".into(),
        description: "Desc".into(),
        tags: vec!["tag".into()],
    };
    bibites_io::storage::save_workshop_metadata(&item, &metadata).unwrap();
    assert_eq!(
        bibites_io::storage::load_workshop_metadata(&item).unwrap(),
        metadata
    );
    let browser = bibites_io::storage::SaveBrowser::new(paths.clone());
    let (renamed, rows) = browser.rename_workshop(&item, "renamed").unwrap();
    assert_eq!(renamed.file_name().unwrap(), "renamed");
    assert!(rows.iter().any(|entry| entry.name == "renamed"));
    let _ = std::fs::remove_dir_all(root);
}

/// `ChallengesProgress.TrySetHighScoreOfChallenge` persisted across runs: stars
/// beat score, ties do not replace, and the previous best is reported.
#[test]
fn challenge_best_results_persist_with_the_games_rule() {
    use bibites_core::world::ChallengeScoreInfo;
    use bibites_io::storage::{challenge_high_score, record_challenge_result};

    let root = temp_root("challenge-progress");
    let paths = StoragePaths::new(root.clone());
    let result = |stars: u8, score: f32| ChallengeScoreInfo {
        challenge_name: "Big Lake".to_owned(),
        star_attained: stars,
        score,
        elapsed_s: 0.0,
        champion_name: "champ".to_owned(),
        champion_path: "champ.bb8template".to_owned(),
    };
    assert!(challenge_high_score(&paths, "Big Lake").unwrap().is_none());

    let first =
        record_challenge_result(&paths, &result(2, 100.0), true, "2026-11-03T20:00:00Z").unwrap();
    assert!(first.improved && first.previous.is_none());

    // Fewer stars with a much better score: not stored.
    let worse = record_challenge_result(&paths, &result(1, 1_000.0), true, "t").unwrap();
    assert!(!worse.improved);
    assert_eq!(worse.previous.as_ref().unwrap().stars, 2);

    // Tie: not stored. Strictly better: stored, previous reported.
    assert!(
        !record_challenge_result(&paths, &result(2, 100.0), true, "t")
            .unwrap()
            .improved
    );
    let better =
        record_challenge_result(&paths, &result(2, 150.0), true, "2026-11-03T21:00:00Z").unwrap();
    assert!(better.improved);
    assert_eq!(better.previous.unwrap().score, Some(100.0));

    // The key ignores spaces and case, and the result survives a reload.
    let stored = challenge_high_score(&paths, "bigLAKE").unwrap().unwrap();
    assert_eq!(stored.score, Some(150.0));
    assert_eq!(stored.time, "2026-11-03T21:00:00Z");
    let _ = std::fs::remove_dir_all(root);
}

/// The installed game's own `PlayerPrefs` count as the previous best, and are
/// shown when this port has no result of its own.
#[test]
fn challenge_best_reads_the_games_player_prefs() {
    use bibites_core::world::ChallengeScoreInfo;
    use bibites_io::prefs::GamePrefs;
    use bibites_io::storage::{
        challenge_high_score_with_prefs, challenge_score_from_prefs, record_challenge_result_over,
    };

    let prefs = GamePrefs::parse(
        "i\tdownwiththebasicsStar\t2\nf\tdownwiththebasicsScore\t2548.35009765625\n\
         s\tdownwiththebasicsChampionName\tDarwin's Demon\ni\tapocalypseStar\t0\nf\tapocalypseScore\tNaN\n",
    );
    let game = challenge_score_from_prefs(&prefs, "Down With The Basics").unwrap();
    assert_eq!(game.stars, 2);
    assert_eq!(game.score, Some(2548.35));
    assert_eq!(game.champion_name, "Darwin's Demon");
    // A challenge the game never completed has no stored result.
    assert!(challenge_score_from_prefs(&prefs, "Apocalypse").is_none());

    let root = temp_root("challenge-prefs");
    let paths = StoragePaths::new(root.clone());
    let shown = challenge_high_score_with_prefs(&paths, "Down With The Basics", &prefs, true)
        .unwrap()
        .unwrap();
    assert_eq!(shown.stars, 2);

    let result = |stars: u8, score: f32| ChallengeScoreInfo {
        challenge_name: "Down With The Basics".to_owned(),
        star_attained: stars,
        score,
        elapsed_s: 0.0,
        champion_name: "c".to_owned(),
        champion_path: "c".to_owned(),
    };
    // Worse than the game's best: not "improved", and nothing is written.
    let worse =
        record_challenge_result_over(&paths, &result(1, 9_999.0), true, "t", Some(&prefs)).unwrap();
    assert!(!worse.improved);
    assert_eq!(worse.previous.unwrap().stars, 2);
    assert!(!paths.challenge_progress_path().exists());
    // Better than it (the game's rule also needs a higher score): stored in this port's own file.
    let better =
        record_challenge_result_over(&paths, &result(3, 3_000.0), true, "t", Some(&prefs)).unwrap();
    assert!(better.improved);
    assert!(paths.challenge_progress_path().exists());
    let _ = std::fs::remove_dir_all(root);
}

/// `BibiteTemplateSelectorPanel.FillLists`: templates (by file name) then saved `.bb8`
/// files; hidden nodes are `nodes - 48`; unparseable files are skipped.
#[test]
fn library_entries_follow_the_games_list_rules() {
    use bibites_io::storage::{TemplateKind, list_library_entries};

    let root = temp_root("library");
    let paths = StoragePaths::new(root.clone());
    let templates = paths.templates_dir();
    std::fs::create_dir_all(&templates).unwrap();
    let nodes = |count: usize| vec![serde_json::json!({"Type": 0}); count];
    let write = |dir: &Path, file: &str, json: serde_json::Value| {
        std::fs::write(dir.join(file), json.to_string()).unwrap();
    };
    write(
        &templates,
        "b file.bb8template",
        serde_json::json!({"name": "Zed", "description": "last by name", "version": "0.6.3.1",
            "nodes": nodes(50), "synapses": [1, 2, 3]}),
    );
    write(
        &templates,
        "A file.bb8template",
        serde_json::json!({"name": "Alpha", "nodes": nodes(48), "synapses": []}),
    );
    // No `nodes`/`synapses`: `LoadState` fails, the game skips it.
    write(
        &templates,
        "broken.bb8template",
        serde_json::json!({"name": "Broken"}),
    );
    // Unknown version: skipped as well.
    write(
        &templates,
        "future.bb8template",
        serde_json::json!({"name": "Odd", "version": "not a version", "nodes": nodes(48), "synapses": []}),
    );
    let saved = root.join("Bibites");
    write(
        &saved,
        "Saved one.bb8",
        serde_json::json!({"desc": "from a bibite", "version": "0.6.3.1", "isOfficial": true,
            "brain": {"Nodes": nodes(49), "Synapses": [1]}}),
    );

    let entries = list_library_entries(&paths);
    let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    // Directory order (by file name, case-insensitive), templates first.
    assert_eq!(names, ["Alpha", "Zed", "Saved one"]);
    assert_eq!(entries[0].n_hidden_nodes, 0);
    assert_eq!(entries[0].version, "0.6.3.1"); // missing version -> present
    assert_eq!(entries[1].n_hidden_nodes, 2);
    assert_eq!(entries[1].n_synapses, 3);
    assert_eq!(entries[1].description, "last by name");
    assert_eq!(entries[2].kind, TemplateKind::Genome);
    assert!(entries[2].is_official);
    assert_eq!(entries[2].n_hidden_nodes, 1);
    assert_eq!(entries[2].description, "from a bibite");
    let _ = std::fs::remove_dir_all(root);
}

/// Official 0.6 scenarios do not define the TPS settings; scenarios saved by the editor do.
#[test]
fn scenario_defined_numbers_reads_nested_values() {
    use bibites_io::bundle::scenario_defined_numbers;
    use std::io::Write;

    let root = temp_root("defined");
    let path = root.join("s.zip");
    let file = std::fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file(
        "settings.bb8settings",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(
        br#"{"recommended":{"simTPS":{"Value":40},"visionLookupUpdateFactor":{"Value":23.0}},"zones":[]}"#,
    )
    .unwrap();
    zip.finish().unwrap();
    let found =
        scenario_defined_numbers(&path, &["simTPS", "brainTPS", "visionLookupUpdateFactor"])
            .unwrap();
    assert_eq!(found, vec![Some(40.0), None, Some(23.0)]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_achievements_file_keeps_what_was_unlocked_and_the_counters() {
    let state = temp_root("achievements");
    let paths = StoragePaths::new(&state);
    assert_eq!(paths.achievements_path(), state.join("achievements.json"));
    // Nothing stored yet: nothing unlocked.
    let empty = bibites_io::storage::read_achievements(&paths).unwrap();
    assert!(empty.unlocked.is_empty() && empty.counters.is_empty());
    let mut file = empty;
    file.unlocked.insert("ACH_SIM_7DAYS".to_owned());
    file.counters.insert("BibiteEditorSaved".to_owned(), 7);
    bibites_io::storage::write_achievements(&paths, &file).unwrap();
    assert!(state.join("achievements.json").is_file());
    assert!(!state.join("achievements.json.tmp").exists());
    let again = bibites_io::storage::read_achievements(&paths).unwrap();
    assert_eq!(again, file);
    // A file that is not JSON is an error, not an empty file.
    std::fs::write(paths.achievements_path(), "not json").unwrap();
    assert!(bibites_io::storage::read_achievements(&paths).is_err());
    let _ = std::fs::remove_dir_all(state);
}

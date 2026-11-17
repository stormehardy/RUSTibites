# UI actions ↔ io/storage mapping

Survey of the native game's save/load/scenario/settings/file-management UI
(`Here/`, read-only source) and the RUSTibites io/storage surface that backs the
viewer. New io APIs live in `crates/bibites-io/src/storage.rs`
(`bibites_io::storage`), tests in `crates/bibites-io/tests/storage.rs`.

## Native data locations

| Kind | Native path (`Application.persistentDataPath` = `%LOCALAPPDATA%Low/The Bibites/The Bibites`) | Source |
|---|---|---|
| Manual saves | `Savefiles/*.zip` | `SaveController.SavePath`, `SaveGamePanel` |
| Autosaves | `Savefiles/Autosaves/autosave_yyyyMMddHHmmss.zip`, newest `AutoSaveMaxFiles` kept | `SaveController.AutoSave/CleanUpAutosaveFolder` |
| Quick save/load | `Savefiles/quick.zip` | `SaveController` |
| Scenarios/challenges | `Scenarios/*.zip` (bundles: settings, scenario.info, templates, img) | `ScenarioSelectorPanel` |
| Bibite templates | `Bibites/Templates/*.bb8template` | `SaveSystem.bibiteTemplatePath` |
| Save screenshot | `img.png` inside the save zip; temp `tempScreenshot.png` for export | `ScreenShotHandler` |
| User settings | Unity `PlayerPrefs` (not a portable file) | `UserSettings` (`Bool/Int/ChoiceUserSetting`) |
| Challenge progress | Unity `PlayerPrefs` (`<challenge>Star/Score/ChampionName/Time`) | `ChallengesProgress` |

RUSTibites keeps the same directory shape under a caller-provided root
(`StoragePaths`). The interactive viewer points that root at the **game's own data
folder** (`StoragePaths::default_content()`: `$RUSTIBITES_DATA_DIR` when set, else the
installed game's `%USERPROFILE%\AppData\LocalLow\The Bibites\The Bibites`, else
`./userdata`), so manual saves, `quick.zip`, autosaves, scenarios saved from the editor,
templates and single-bibite/egg files (`Bibites/`) are visible to the original game.
The port's private files (`user_settings.json`, `challenge_progress.json`, the
thumbnail cache, the local `Workshop/` folder, the menu video cache) stay in
`./userdata` (`StoragePaths::state_dir`). The CLI and the headless screenshot runs
never write to the game's folder. Import helpers read an explicit source path and copy
into the active layout.

Autosave details that follow the game (`SaveController.AutoSave`): names use the
machine's local time (`autosave_yyyyMMddHHmmss`), none are written during challenges,
and `AutoSaveMaxFiles` (read from the game's own settings) newest files stay plus the
one just written. Deliberate safety deviations because the folder holds the player's real
history: only `autosave_<digits>.zip` files are ever removed, and at most one per autosave
(`AUTOSAVE_MAX_REMOVALS_PER_RUN`), so a misread or lowered limit trims a backlog one file
at a time instead of deleting it in one pass.

## Action → API map

| Native UI action | Native mechanism | RUSTibites API |
|---|---|---|
| Load panel: refresh list incl. auto-saves toggle | `SaveController.GetSaves(includeAutoSave)`, `FillList` | `storage::list_saves(&paths, include_autosaves)` (`StoredFile { name, path, size, modified, autosave }`) |
| Load panel: select save, show name/date/version/thumbnail | read `scene.bb8scene`, `img.png` | `load_save_bundle(path)` (scene/settings/entities) + zip `img.png` raw bytes |
| Load selected save | `SaveSystem.LoadGame` | `load_save_bundle` → `world_contents_from_save` → viewer `World` |
| Delete selected save | `File.Delete(selectedSaveInfo.FullName)` | `storage::delete_file(path)` |
| Open save folder | `OpenSaveFolder` → shell | platform open-folder in viewer (use `paths.saves_dir()`) |
| Save panel: list existing saves | `SaveController.GetSaves` | `storage::list_saves(&paths, false)` |
| Save: name validation / overwrite indicator | `containsABadCharacter`, `File.Exists` | `storage::sanitize_file_name(name, "zip")`, check path exists |
| Save / save-as (new name) | `SaveController.Instance.SaveWorld(Savefiles/<name>.zip)` with screenshot/template flags | `save_world_contents(&contents, path)` (or `save_bundle_from_contents` + `write_save_bundle`), screenshot `img.png`/`include_templates` fields already modelled; `storage::save_as(source_zip, &paths, name, overwrite)` for duplicating an existing save |
| Save: overwrite confirmation | `AskOverwritePreviousSave` | caller passes `overwrite=false`, then `true` after UI confirm |
| Save: delete selected save | `DeleteSelectedSave` | `storage::delete_file` |
| Scenario selector: refresh list | `ScenarioSelectorPanel.ReloadList` (`Directory.GetFiles(...,"*.zip")`) | `storage::list_scenarios(&paths)` |
| Scenario: select, read metadata/isChallenge/isOfficial, thumbnail | `ScenarioItemReference.InitScenarioItem`, `scenario.info`, `img.png` | `load_scenario_bundle(path)` (settings/scenario/templates/img), `ScenarioSettingsFile` fields |
| Scenario: start new world | `GameManager.StartGame(scenarioPath)` | `load_scenario_world_contents` + viewer `World::from_contents` |
| Scenario: import user zip | no dedicated button (manual copy to `Scenarios/`; official re-import on startup) | `storage::import_scenario(source, &paths, name, overwrite)` (validates scenario zip first) |
| Challenge list/start | same `Scenarios/` zips with `isChallenge=true` | `list_scenarios` + `ScenarioSettingsFile.is_challenge` |
| Templates list/select | `BibiteTemplateSelectorPanel` lists `Bibites/Templates` | `storage::list_templates(&paths)` + `load_template` |
| Export/copy save out | native OS copy | `storage::export_file(source, destination)` |
| Rename save/scenario/template | native file rename | `storage::rename_file(path, new_name)` |
| Auto-save schedule + retention | `SaveController.AutoSave`, `AutoSavePeriod/MaxFiles/RealTime` | viewer timer (sim/wall-clock) calling `save_world_contents` into `autosaves_dir`; `storage::cleanup_autosaves(&paths, max_files)` |
| User settings persistence | `PlayerPrefs` via `*UserSetting` classes | `storage::UserPreferences` + `load_user_preferences` / `save_user_preferences` (JSON; fields mirror IncludeAutoSaves, AutoSave, period, max files, PauseAfterLoad, ReloadAfterAutosaves) |
| Settings save/load (simulation) | settings embedded in save/scenario `.bb8settings`; independent settings exported | existing `bibites_io::settings` (`parse_settings_json`/`to_bridge_settings_json`, `ScenarioSettingsFile`, `SimSettings`) |
| Screenshot into save | `ScreenShotHandler.SaveScreenshotToZip` → `img.png` | viewer captures image and sets `SaveBundle.img_png` (round-trip tested) |
| Export screenshot/image | `SaveScreenshotToImage`/`tempScreenshot.png` | viewer writes PNG itself (no io API needed) |

## Gap flags for the viewer (RUSTibites support)

Implemented in io now (needs view-dev wiring):
- save-as, delete, rename, import scenario, export/copy file, list refresh
  (`list_saves/list_scenarios/list_templates`), layout creation
  (`ensure_layout`), auto-save pruning (`cleanup_autosaves`), user prefs
  persistence (JSON).

Still missing / viewer-side:
- **Load/Save/Scenario panels themselves** in `bibites-view` (list widgets,
  thumbnails/date/version preview, overwrite/delete dialogs, open-folder button).
- **Save thumbnails/screenshots**: io supports `img.png`, but the viewer has no
  screenshot capture path yet.
- **Auto-save scheduler** (period, real-time vs sim-time, reload-after-autosave)
  and quick-save/quick-load shortcuts.
- **Scenario/challenge import UX**: no file picker/drag-drop; Workshop/Steam
  subscriptions are explicitly out of scope.
- **Challenge progress / stars / champion records** are `PlayerPrefs` only;
  not ported (would need a `ChallengeProgress` JSON format; no native file).
- **User prefs**: field mirror exists, but no settings UI wiring yet; native
  `PlayerPrefs` values cannot be imported losslessly (registry), only recreated
  from defaults/UI.
- **Multi-user / save-path override**: `UserPreferences.save_path_override` is
  recorded but `StoragePaths` currently takes an explicit root from the viewer.
- **List caching**: refresh is an explicit re-scan; no filesystem watcher.

## Compatibility notes

- Importing native saves/scenarios uses the existing bundle loaders, so version
  migrations and `.bb8state` sidecars are unaffected; exporting does not modify
  the source file.
- Game-load compatibility of generated saves is unchanged (storage layer only
  copies/renames; it never rewrites a zip).
- Default/Ring fixtures remain the canonical round-trip tests.

## Tests

`crates/bibites-io/tests/storage.rs` covers: save-as + overwrite policy + name
validation, auto-save listing/ordering + retention cleanup, scenario import
validation, export/rename/delete, template listing, and user-preferences
round-trip with default fallback. Absent `testdata/` fixtures skip cleanly.

## Viewer facade usage

ibites_io::storage::SaveBrowser is the high-level API for UiAction::Invoke:

`ust
let browser = SaveBrowser::new(StoragePaths::default_local());
let contents = world.to_contents();

// SaveWorld / Save As
let path = browser.save_world(name, &contents, overwrite)?; // Savefiles/<name>.zip
let saves = browser.refresh_saves(include_autosaves)?;     // OpenSavePanel list
// QuickSave / QuickLoad
browser.quick_save(&contents)?;                            // Savefiles/quick.zip
let loaded = browser.quick_load()?;                        // -> World::from_contents(loaded)
// OpenLoadPanel
let contents = browser.load_world(&selected_path)?;
let world = World::from_contents(contents)?;
// Delete/rename/import/export
let saves = browser.delete_save(&path, false)?;
let (new_path, saves) = browser.rename_save(&path, new_name, false)?;
let (path, scenarios) = browser.import_scenario(&src, name, false)?;
let (path, templates) = browser.import_template(&src, name, false)?;
browser.export(&source, &destination)?;
`

efresh_saves/scenarios/templates return the updated sorted lists, so the
panel can refresh immediately after any mutation. Existing
load_world_contents/save_world_contents remain available for lower-level use.

## Save browser rows + autosave helpers

- storage::list_save_entries(&paths, include_autosaves) / SaveBrowser::refresh_save_entries(..)
  return SaveListEntry { name, path, size, modified, autosave, version,
  sim_time, population, thumbnail }. Metadata is read cheaply from the save zip
  (scene.bb8scene fields + cached img.png thumbnail), not by parsing entities.
- storage::extract_thumbnail(save, &paths) caches img.png to
  <root>/thumbnails/<stem>.png and SaveListEntry.thumbnail points at it when present.
- Autosave: storage::AutosaveSchedule::from_preferences(&prefs),
  storage::autosave_file_name(timestamp) (utosave_<digits>.zip; caller supplies
  local yyyyMMddHHmmss), SaveBrowser::autosave(timestamp, &contents, schedule)
  writing under Savefiles/Autosaves/ and enforcing retention.
- Save/overwrite/delete/rename can return refreshed rows directly:
  save_world_with_list, delete_save_with_list, ename_save_with_list;
  save_as, quick_save, quick_load unchanged.
- Settings persistence: UserPreferences covers save-path override + autosave
  options via load_user_preferences/save_user_preferences (atomic JSON).

## View integration helpers (file-open, scenario settings edit)

- `storage::open_in_file_manager(path)` opens a folder (or selects a file) in the
  platform file manager; the load/save panels can map "Open save folder" to
  `open_in_file_manager(&paths.saves_dir())`.
- `storage::update_scenario_settings(path, &ScenarioSettingsFile)` rewrites an
  existing scenario bundle in place with edited settings while preserving
  `scenario.info`, templates, `img.png` and unknown zip entries. It syncs the
  effective independent block used by the scenario writer
  (`settings.recommended`/`settings.independents` -> `bundle.settings.independents`)
  and updates `ScenarioSettingsFile.recommended`, so `sim_tps` and other
  independent edits survive a save/reload. Refresh the scenario list with
  `SaveBrowser::refresh_scenarios()` afterwards.
- `SaveListEntry` rows, thumbnails (`extract_thumbnail`) and autosave hooks are
  described above; the viewer only needs to render rows and call
  `extract_thumbnail` lazily for visible entries.

## Scenario/library save tranche

- Scenario editor: edit settings, then `storage::update_scenario_settings(path, &settings)`
  (in-place, preserves templates/img/info/unknown entries and syncs
  `recommended`/`independents` into the effective settings) and refresh with
  `SaveBrowser::refresh_scenario_entries()`.
- Library selector: `storage::list_template_entries(&paths)` returns
  `TemplateListEntry { name, description, path, kind, size, modified }`, where
  `kind` is `template` (`.bb8template`) or `genome` (`.bb8`); metadata is read
  from the JSON header (`name`/`description`, file-stem fallback).
  Convenience: `SaveBrowser::refresh_template_entries()`.

## New Game / scenario editor flow

```rust
// Option values: optionally start from a parsed settings file (or a scenario's
// settings_file), plus core ScenarioData (zones/spawn rows/tools).
let settings: ScenarioSettingsFile = /* editor state / parsed */;
let scenario_data: ScenarioData = /* default zone+spawn row or editor state */;
bibites_io::storage::create_scenario(
    &scenario_path,
    "My New Scenario",
    "created from options",
    &settings,
    scenario_data,
)?;
// Start a new game: load scenario zones/settings/spawn rows with an explicit seed.
let contents = bibites_io::storage::new_game_contents(&scenario_path, seed)?;
let world = /* viewer/world new-game spawner: World::new + spawn_initial_population */;
```

- `create_scenario` writes a normal scenario `.zip` (`settings.bb8settings`,
  `scenario.info`, no bundled templates/image), with name/description/version and
  the effective independent block from `settings.recommended`/`independents`.
  Existing bundle writers/import helpers can add templates/image afterwards.
- `new_game_contents(path, seed)` returns the scenario's `WorldContents` with the
  requested seed; **simulation startup stays with world/view** (fresh `World` +
  new-game spawner) to avoid io owning spawner behavior.
- Viewer edits to an existing scenario: `update_scenario_settings` (in-place,
  preserves all members) then `refresh_scenario_entries()`; creation + edit
  round-trips are covered by storage tests (10 tests).

## Full viewer Settings prefs

`storage::UserPreferences` now covers viewer-only controls (kept separate from
simulation `SimSettings`); persisted atomically as `<root>/user_settings.json`
with `#[serde(default)]` fallback and `UserPreferences::reset(&paths)`.
Groups:

- **Autosave:** `autosave`, `autosave_real_time`, `autosave_period_minutes`,
  `autosave_max_files`, `reload_after_autosaves`.
- **Load/save UI:** `include_autosaves_in_load_panel`, `pause_after_load`,
  `save_path_override`.
- **Window/UI:** `ui_scale`, `fullscreen`, `resolution_width`,
  `resolution_height`.
- **View visuals:** `background_mode`, `show_grid`, `show_node_names`,
  `show_pheromones`, `pheromone_opacity`.
- **Performance/camera/selection:** `min_fps`, `camera_zoom`,
  `camera_follow_selection`, `camera_pan_speed`, `selection_outline`.

Mapping rule for view: controls that affect rendering/UI/camera belong here;
controls that affect the simulation (`simTPS`, sizes, mutation, materials, zones,
etc.) belong in `ScenarioSettingsFile`/`SimSettings` and are saved inside the
simulation settings/scenario zip, not in `UserPreferences`. Captured view
controls should set these fields and call `save_user_preferences`; the Settings
"Reset" button uses `UserPreferences::reset`. Tests cover round-trip, defaults
fallback for missing fields, and reset.

## Local Workshop file browser (no Steam)

Local workshop-like content lives under `<root>/Workshop/` and is browsed with:

- `storage::list_workshop_entries(&paths)` -> `WorkshopEntry { name, path, kind,
  size, modified, folder }`; `kind` is `archive` (`.zip`), `template`
  (`.bb8template`), `genome` (`.bb8`), `folder`, or `other` (one level; folders
  are items).
- `storage::import_workshop_item(source, &paths, name, overwrite)` copies a file
  or folder (recursive) into `Workshop/<name>`;
  `export_workshop_item(source, destination)` and
  `delete_workshop_item(path)` (recursive for folders).
- `SaveBrowser::{refresh_workshop_entries, import_workshop, delete_workshop}`
  return refreshed rows after mutations.

Reuse: typed content still goes through the normal paths — scenarios via
`import_scenario`/`list_scenarios` (validated bundles), templates via
`import_template`/`list_template_entries`; the Workshop folder is for arbitrary
local user content/packs. Steam Workshop subscriptions/sharing are explicitly
out of scope (no Steam API); imports/exports are plain local filesystem copies.

## Hotkeys + local workshop metadata (final UI polish)

- `UserPreferences.hotkeys: BTreeMap<String, String>` (action -> key name) with
  serde default = `default_hotkeys()` mirroring the viewer's current hardcoded
  keys: pause=Space, step=Right, fit=F, fit_bibites=B, reset_view=R,
  toggle_grid=G, toggle_zones=Z, toggle_pellets=P, toggle_pheromones=O,
  toggle_hud=H, quit=Escape, screenshot=F12. (Historic list: the window loop
  binds none of the layer toggles; the game's `UserControl` uses Z for "select
  a random zone", R / G / O / E for bibites and eggs, K for the auto-panner.) Helpers: `hotkey(action)`,
  `set_hotkey(action, key)` (rejects empty); `UserPreferences::reset` restores
  defaults. Persisted atomically with the rest of the prefs JSON; missing/old
  files get the default map.
- Local workshop metadata for no-Steam packs: `WorkshopItemMetadata { name,
  description, tags }`, `workshop_metadata_path(item)` (inside folders
  `workshop.json`, files sidecar `.workshop.json`), atomic
  `save_workshop_metadata` / `load_workshop_metadata` (missing = defaults), and
  `rename_workshop_item` / `SaveBrowser::rename_workshop` (returns refreshed
  rows).

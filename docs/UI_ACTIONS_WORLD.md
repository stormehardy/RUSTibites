# UI actions → world/core API map

Status: survey/design note, 2026-11-02. Sources are the reconstructed game
scripts under `Here/` (UI panels, `WorldObjectsSpawner`, `ScenarioSettings`,
`SettingsChanger`, `SaveSystem`). This note covers **world-owned** actions; body
selection/stat panels are mapped in `docs/UI_ACTIONS_BODY.md`.

## Implemented core action APIs (viewer-callable)

| UI action | Here source | Core API |
|---|---|---|
| Pellet placer, single click (material, size factor) | `SimulationScripts/PelletPlacer.cs::PlacementEvent` | `World::spawn_pellet_with_energy(material, pos, energy)` — spawns, registers hash, adds production ledger. `MaterialKind::{Plant,Meat}`; energy = `scenario.pelletEnergy * sizeFactor`. |
| Pellet placer, pile (1–10, ±25 % energy jitter) | same | `World::spawn_pellet_pile(material, pos, count, energyPerPellet, pileRadius)` — deterministic `world.rng`, bounded to 1000, returns ids. |
| Place rad tower | `WorldObjectsSpawner.GenerateRadTower` / `GUIManager` | `World::place_rad_tower_at(pos, attachedZone)` — defaults via `RadTower::new(saveId,pos,simSize)`; snaps to attached zone. |
| Place pheromone tower | `GeneratePheroTower` | `World::place_phero_tower_at(pos, attachedZone)` — creates persistent phero spot/hash immediately. |
| Place colour killer | `GenerateColorKiller` | `World::place_color_killer_at(pos, attachedZone)` — `ColorKiller::new` defaults. |
| Destroy selected tower / colour killer | `RadTowerPanel.DestroyTarget`, `PheroTowerPanel.DestroyTarget`, `ColorKillerPanel.DestroyColorKiller` | `World::remove_tool(id)` — removes phero spot/hash too; later `ToolId`s shift down (serialized `save_id` is stable). |
| Attach tower to spawner/zone | `ColorKillerPanel.SetAttachedSpawnPoint`, tower panels | `World::set_tool_attached_zone(id, Option<ZoneId>)`; then `World::step_tools(0.0)` for phero spots if mutated directly. |
| Edit tower properties (hue, tolerance, body/eye, any/most, period, radius, strengths) | `ColorKillerPanel`, `RadTowerPanel`, `PheroTowerPanel` | `World::tools.get_mut(id)` → public `Tool::{Rad,Phero,Killer}` fields; body reads them on the next tool phase. |
| Kill target/selection (alive) | `GUIManager.TryKillTarget`, `SelectionPanel.KillSelection`, `BibiteStatsPanel.KillCurrentBibite` | `World::kill_bibite(id)` — queues Hazard death; resolution/meat next body tick. Viewer loops selection. |
| Abort egg | `GUIManager.TryKillTarget` (Egg case), `SelectionPanel` | `World::abort_egg(id)` — sets `hatch.aborting`; egg phase removes it and spawns meat. |
| Spawn bibite from template | `WorldObjectsSpawner.SpawnBibiteFromTemplate` | `World::spawn_bibite_by_template_path(path, growth, tagging, tag, pos)`; existing `spawn_bibite_from_template` for an in-memory template. |
| Spawner row editor (add/clear rows) | spawner/settings editor | `World::add_bibite_spawn(row)`, `World::set_bibite_spawns(rows)` (validates; resets progress). `start_fresh_scenario_spawner` / `step_spawner` drive it. |
| Settings animator add/delete/clear | `UIScripts/SettingsChangerCreator.cs`, `SettingsChangerHandle` | `World::add_settings_changer`, `remove_settings_changer`, `clear_settings_changers`; phase 2 evaluates splines/keys. |
| Global biomass/fertility/spawn-rate/species/material/decay settings | `ScenarioIndependentSettings`, `ScenarioSettings`, material editors | `World::apply_settings(newSimSettings)` (safe fields rebuilt); convenience `set_independent_setting(key,value)` / `set_scenario_setting(key,value)` with game JSON keys. |
| Simulation size / simTPS change | Scenario settings UI | **Rejected by `apply_settings`**; these are prebuilt into bounds/clock/hash. Viewer should rebuild the world (new `World::new`) after applying them. |
| Grid/shade rendering toggles (grid colours/steps, pheromone display, camera) | `UserSettings`, `BackgroundManager`, `VisibilityPanel` | **Viewer-only** (`UserSettings`), no core API needed. |
| Save / load / new game / autosave | `ManagementScripts/SaveSystem.cs`, `SaveController.cs` | Core exposes `World::to_contents()` / `World::from_contents`, `capture_checkpoint_state` / `restore_checkpoint_state`; file/zip/slot IO lives in `bibites-io` and the CLI/viewer. No UI slot-management API in core. |
| Challenge mode/parameters/scores | `ChallengeManager`, `ChallengeParameters`, `ChallengesProgress` | **Missing in RUSTibites core**; no challenge state/objectives/scoring. |
| Zone editor: add / edit / remove zone | Scenario zone editor / `ZoneManager` | `World::add_zone(ZoneSettings) -> ZoneId`, `World::apply_zone_settings(ZoneId, ZoneSettings)`, `World::remove_zone(ZoneId) -> Result<usize>`. Add gets a fresh per-zone RNG stream; edit refreshes derived area/biomass and snaps towers; remove despawns owned pellets, detaches/remaps attached towers and clears spawn rows targeting the removed file id. All old `ZoneId`s become stale after add/remove. |
| Explode already-dead corpse to meat | `GUIManager.TryKillTarget` dead branch | **Missing**; only live `kill_bibite` is exposed. Needs a body-store corpse-burst action (body-owned). |
| Spawn egg by template (viewer spawn-as-egg) | `SpawnBibiteAs` egg branch | **Partial**: body `EggStore::spawn`/world spawner rows exist; no single `World::spawn_egg_from_template` action yet (template/hatch-energy/brain assignment rules needed). |
| Manual life-cycle/stat edits (energy/health/age/gene edits, tags) | `BibiteStatsPanel`, gene/brain editors | **Missing in world**; body/brain-owned mutation APIs (body-dev `docs/UI_ACTIONS_BODY.md`). |

## Viewer usage snippet (first toolbar tranche)

```rust
// cursor world position; template is loaded into world.templates by io/view
let ids = world.spawn_pellet_pile(MaterialKind::Plant, cursor, 5, 200.0, 40.0);
let zone = world.zone_at(cursor);                    // Option<ZoneId>
let rad = world.place_rad_tower_at(cursor, zone);    // ToolId
let phero = world.place_phero_tower_at(cursor, zone);
let killer = world.place_color_killer_at(cursor, zone);
// edit selected killer:
if let Some(crate::tools::Tool::Killer(k)) = world.tools.get_mut(killer) { k.hue = 0.5; }
// spawn/select/kill:
let id = world.spawn_bibite_by_template_path("starter", GrowthAtSpawn::Adult,
                                             Tagging::NoTagging, "", cursor)?;
world.kill_bibite(id);
world.remove_tool(rad);
// Settings panel: clone, edit, apply; simSize/simTPS need a new world.
let mut s = world.settings.clone();
s.independents.set_raw("biomassDensity", SettingValue::Float(0.01));
world.apply_settings(s)?;
```

## Captured Settings pages (Graphics / Gameplay / Hotkeys)

These three captured pages are built from `Here/SettingScripts/UserSettings.cs` and
`Here/UIScripts.SettingHandles/UserSettingsManager.cs`. They are **viewer/UserSettings**,
not `SimSettings`; there is no simulation state to map or persist in core. The
viewer owns their values and their on-disk user-settings file.

| UI group (captured) | Fields | Rust core mapping |
|---|---|---|
| Graphics → display | Fullscreen, Automatically Detect Screen Resolution, Screen Resolution, UI Scale | none — viewer/window |
| Graphics → rendering | Pheromones Strength, Procedural Size, Fertility Color, Background Color, Show Grid + Grid Color/Step/Width ×2, Texture Color, Show Experimental Background, Show Node Names | none — viewer render/UserSettings |
| Gameplay → save | Autosave, Autosave Period, Real-time autosave, AutoSave Max Files, Reload After Autosaves, Pause After Load | viewer/io; save slots and autosave scheduling are not core |
| Gameplay → easter eggs | Allow Easter Eggs, Easter Eggs Probability Period | none — viewer/meta event, not the Red Death sim event |
| Gameplay → controls/camera | Minimum FPS, Ignore FPS minimum when unfocused, Allow Arrow-Key camera, Open Zone Editor on select, Selection Radius, Reselect on Corpses/Reselection Delay | viewer input/camera |
| Gameplay → reporting | Analytics Consent, Cloud Diagnostic Consent | viewer/platform |
| Hotkeys | camera/selection/simulation hotkeys (hardcoded `KeyCode` checks in `UserControl`/`GUIManager`/`CameraManager`; no rebinding store in captured scripts) | viewer input map; no core setting |

Simulation settings that **do** map to `SimSettings` are surfaced in the scenario
editor / in-world settings panel, not these pages: `SimulationSize`, biomass
density/fertility, spawn limit/rate, TPS/brain/decay/vision factors, species
settings → `World::apply_settings`/`set_independent_setting` (with simSize/simTPS
structural rejection); shade/void/wrap/pheromone/red-death scenario flags →
`set_scenario_setting` / `World::red_death_mut`; materials → `apply_settings`;
zones → `apply_zone_settings`; tool properties → typed setters below.

### Typed tool property setters

| Panel control | API |
|---|---|
| Pellet material / amount / count / radius | `spawn_pellet_with_amount(material, pos, amount_u2)`, `spawn_pellet_with_energy`, `spawn_pellet_pile(...)` |
| Rad: count, intensity, radius, taper, zone | `World::set_rad_tower_params(id, count_rate, intensity_rate, radius, taper)`; `set_tool_attached_zone` |
| Phero: red/green/blue strength, heading, zone | `World::set_phero_tower_params(id, [r,g,b], heading)`; `set_tool_attached_zone` |
| Killer: hue, tolerance, body/eye, any/most, radius, period, zone | `World::set_color_killer_params(id, hue, tolerance, body_or_eye, any_or_most, radius, period)`; `set_tool_attached_zone` |

## Scenario persistence

Scenario/zone edits are persisted by the normal io path: `World::to_contents()`
serializes `settings` (including applied global/scenario values), zone configs
(added/removed/reconfigured), spawn rows and tools; `World::from_contents()`
reconstructs them. `SettingsChanger`s are part of `ScenarioData.settings_changers`
and round-trip through the scenario JSON. File/zip/slot handling remains in
`bibites-io`/viewer.

### Scenario editor persistence (exact io integration)

`World::scenario_settings_snapshot() -> WorldScenarioSettings` exposes
`settings`, `zones`, `spawn_rows`, `settings_changers` for the editor. To persist
settings-only edits without dropping unknown keys/challenge/groups, start from
the parsed scenario file and copy the snapshot into it:

```rust
let bundle = bibites_io::load_scenario_bundle(&path)?;   // preserves extras
let mut file = bundle.settings_file;
let snap = world.scenario_settings_snapshot();           // after apply_* calls
file.scenario = snap.settings.scenario.clone();
file.recommended = Some(snap.settings.independents.clone());
file.materials = snap.settings.materials;
file.zones = snap.zones.clone();
file.spawns = snap.spawn_rows.clone();
file.changers = snap.settings_changers.clone();
bibites_io::storage::update_scenario_settings(&path, &file)?;
// settings-only helper with a full SimSettings:
let text = bibites_io::serialize_scenario_settings(&snap.settings)?;
```

`update_scenario_settings` writes the `.bb8settings` member and refreshes the
bundle's effective settings; callers should then `list_scenarios`/reload. For
zone/tool scene edits, rebuild the `ScenarioBundle.scenario` (`ScenarioData`) from
the world snapshot and use `bibites_io::write_scenario_bundle` so `scene.bb8scene`,
`zoneData.bb8zoneData` and templates stay consistent; coordinate with fmt-dev.

## New Game options → settings / generation

`ScenarioSelectorPanel` dropdowns map to these `SimSettings` keys (game JSON
names; use `set_raw`):

| New Game option | Key / mapping |
|---|---|
| Simulation size | `independents.SimulationSize` (Float, u half-size) |
| Density | `independents.biomassDensity` (Float) |
| Fertility | `independents.pelletGrowth` (Float) |
| TPS group | `independents.simTPS`, `brainTPS`, `decayTPS`, `visionLookupUpdateFactor`, `visionSenseUpdateFactor` (Int) |
| Limit Bibites: No | `limitBibiteBirth = false`, `bibiteLimit = 50` |
| Limit 50/100/200/500 | `limitBibiteBirth = true`, `bibiteLimit = 50/100/200/500` |
| Radioactivity None | `scenario.backgroundMutationChance = 0.0`, `backgroundMutationVariance = 0.0` |
| Lower | `0.15 / 0.01` |
| Low | `0.5 / 0.015` |
| Normal | `1.0 / 0.025` |
| Severe | `2.0 / 0.075` |
| Extreme | `4.0 / 0.15` |

All of these are structural for a live world (`SimulationSize`, `simTPS` are
rejected by `apply_settings`); apply them to a **new** world:

```rust
let bundle = bibites_io::load_scenario_bundle(&base_scenario_path)?; // e.g. official Default
let mut settings = bibites_io::scenario_effective_settings(&bundle, None);
settings.independents.set_raw("SimulationSize", SettingValue::Float(size));
settings.independents.set_raw("biomassDensity", SettingValue::Float(density));
settings.independents.set_raw("pelletGrowth", SettingValue::Float(fertility));
settings.independents.set_raw("simTPS", SettingValue::Int(tps));
settings.scenario.set_raw("backgroundMutationChance", SettingValue::Float(chance));
settings.scenario.set_raw("backgroundMutationVariance", SettingValue::Float(variance));
// optional: settings.independents.set_raw("limitBibiteBirth", SettingValue::Bool(b));
let templates = bundle.templates.iter().map(|(name, loaded)| {
    TemplateSource::new(name.clone(), loaded.genome.clone(),
                        loaded.meta.clone(), loaded.brain.clone())
}).collect::<Vec<_>>();
let world = World::new_fresh_scenario(settings.clone(), bundle.scenario.clone(), seed, templates)?;
```

`World::new_fresh_scenario` seeds zone pellets and starts the virgin spawner.
To save the generated scenario, mutate the loaded bundle and write it back:

```rust
let mut bundle = bundle;
bundle.settings = settings.clone();
bundle.settings_file.scenario = settings.scenario.clone();
bundle.settings_file.recommended = Some(settings.independents.clone());
bundle.settings_file.materials = settings.materials;
bibites_io::write_scenario_bundle(&bundle, &new_scenario_path)?;
```

## In-world Dynamics/Settings (19 captured controls)

Apply through `World::set_scenario_setting(key, SettingValue)` or
`World::set_independent_setting(key, SettingValue)`; both clone settings and
call `apply_settings` (which syncs broadcast rates and Red Death bloom state).
`simTPS` is structural (rebuild/new world).

| Control | Key | API / notes |
|---|---|---|
| Corpses (toggle) | `corpsesEnabled` Bool | live scenario setter |
| Prevent Mutations (toggle) | `preventMutations` Bool | live; used by mutation path |
| Pellet Collision (toggle) | `pelletCollision` Bool | live setter; **Rust pellet physics currently always collides** (behavior gap) |
| Disable Herding (toggle) | `disableHerding` Bool | live; body herding gate |
| Add Shaded Bounds (toggle) | `shadeOutsideOfBounds` Bool | live; body shade logic |
| The Red Death Bloom (toggle) | `enableRedDeath` Bool | live; also updates `World::red_death()` bloom |
| Prevent Red Pheromones (toggle) | `preventRedPheroProduction` Bool | live; body phero production |
| Prevent Green Pheromones | `preventGreenPheroProduction` Bool | live |
| Prevent Blue Pheromones | `preventBluePheroProduction` Bool | live |
| Void-No-Mo' (toggle) | `voidAvoidance` Bool | live; body void steering |
| Disable Drag On Held Pellet | `disableDragOnHeldPellet` Bool | live setter; **held-pellet drag currently not modelled** (body gap) |
| Ticks Per Second (slider) | `simTPS` Int | **rebuild/new world** (apply_settings rejects) |
| Brain Update Factor | `brainTPS` Int | live independent setter; body brain cadence |
| Decay Update Factor | `decayTPS` Int | live; rebuilds decay throttle |
| Vision Lookup Factor | `visionLookupUpdateFactor` Int | live |
| Vision Sensing Factor | `visionSenseUpdateFactor` Int | live |
| Minimum Pellet Size | `minPelletSize` Float | live scenario setter |
| Drag Coefficient | `dragCoefficient` Float | live scenario setter |
| Top-centre speed slider (X 0.00) | — | **viewer time-scale only**; not `SimSettings` |

Pellet Rotation/Merge are not on this captured page; if exposed later they map to
`pelletRotation`/`pelletMerge` (stored, but Rust does not yet model rotation or
`try_merge`; documented behavior gaps).

## Brain-editor node anchors

`TemplateSource` now carries `node_anchors: BTreeMap<i64, (f32, f32)>`:
```rust
let anchors = loaded.template.node_anchors.iter()
    .map(|(innovation, anchor)| (*innovation, anchor.position()))
    .collect();
let template = TemplateSource::new(name, genome, meta, brain)
    .with_node_anchors(anchors);
let pos = template.node_anchor(innovation); // Option<(f32,f32)>
```
io's `TemplateMeta::node_anchors` stays the file-level source; copy it into the
core template when loading for the brain editor and back into `TemplateMeta`
when saving a template.

## Challenge sessions (`ChallengeManager`/`ChallengeTrackerPanel`)

Core state is in `world.rs`:

```rust
pub enum ChallengeState { Running, Ended }
pub enum ChallengeOutcome { Running, Completed, Failed }
pub struct ChallengeSession {
    pub parameters: ChallengeParameters,
    pub challenge_name, description: String,
    pub star_descriptions: [String; 3],
    pub champion_name, champion_path: String,
    pub started_time, elapsed_s: f64,
    pub score: f32, pub stars: u8,
    pub state: ChallengeState, pub outcome: ChallengeOutcome,
}
impl ChallengeSession {
    pub fn new(parameters, name, description, champion_name, champion_path,
               started_time: f64) -> Self;
    pub fn update(&mut self, world: &World);            // evaluate now
    pub fn tick(&mut self, world: &World, dt: f32) -> bool; // 1 s cadence
    pub fn score_info(&self) -> ChallengeScoreInfo;
}
pub struct ChallengeScoreInfo { challenge_name, star_attained, score,
    elapsed_s, champion_name, champion_path }
impl ChallengeScoreInfo { pub fn beats(&self, other: Option<&Self>,
    high_is_better: bool) -> bool }
```

World evaluation helpers:
- `World::metric_value(SimMetric, argument: &str) -> f32` — species count/biomass,
  tag count/biomass (live bibites + incubating eggs), material count/biomass,
  time, constant.
- `World::challenge_metric(&SimulationMetric) -> f32`.
- `World::challenge_stars(&ChallengeParameters) -> u8` (3/2/1/0).
- `World::tag_summary("Champion") -> Option<TagSummary>` for champion tracking.

Start flow (champion row must be inserted before the spawner starts):
```rust
let bundle = bibites_io::load_scenario_bundle(&challenge_scenario_path)?;
let challenge = bundle.settings_file.challenge.clone()
    .ok_or("not a challenge")?;
// TemplateSource.name must match challenge.champion.path (case/stem normalized by io)
let templates = /* build Vec<TemplateSource> from bundle.templates */;
let mut world = World::new(settings, bundle.scenario.clone(), seed)?;
world.set_templates(templates);
world.add_bibite_spawn(challenge.champion.clone())?; // tagging CustomTagging/Champion
let _ = world.seed_scenario_world();
world.start_fresh_scenario_spawner();                 // spawns champion row minimum
let mut session = ChallengeSession::new(
    challenge, name, desc, champion_name, champion_path, world.simulated_time);
// each tick after world.tick():
if session.tick(&world, dt) { /* ended: state/outcome/stars/score ready */ }
let info = session.score_info(); // compare with previous high score via beats()
```
The game checks once per simulated second; `tick` accumulates dt and `update`
evaluates exit condition, score and stars. Ended with 0 stars → `Failed`;
≥1 star → `Completed`; tracker/popup bind `stars`, `score`, `elapsed_s`.
`highScoreIsBetter` is in `ChallengeParameters::high_score_is_better`; pass it
to `ChallengeScoreInfo::beats`.

Challenge parameters round-trip through `ScenarioSettingsFile.challenge` in io;
no core save format change. Continuing as a scenario simply drops the session
state (the world continues).

### Challenge parameters editor

`ChallengeParameters` is public and directly editable:
- `exit_condition: ConditionGroup`
- `scoring_metric: SimulationMetric` (metric + argument)
- `high_score_is_better: bool`
- `one_star_condition`, `two_star_condition`, `three_star_condition: ConditionGroup`
- `champion: BibiteSpawn`

Defaults: `ChallengeParameters::game_defaults()`/`Default` = exit Time ≥ 3600 s,
score = TagCount("Champion") higher-is-better, stars at 50/100/200, champion row
`CustomTagging`/`Champion`. Validation: `parameters.validate()` validates every
condition group, metric and champion row.

Star-threshold editor helpers:
```rust
let raw = params.star_thresholds();       // [Option<f32>;3] for simple leaf conditions
params.set_star_thresholds([10.0, 20.0, 30.0])?; // keeps each left metric/comparator
// composite condition trees are supported directly via ConditionGroup API;
// helpers return None/Err rather than flattening them.
```
Runtime star computation is ordered 3→2→1 (`World::challenge_stars`): the
highest satisfied condition wins, else 0. The tracker preview can evaluate the
same conditions with `ConditionGroup::evaluate` and
`World::metric_value`/`challenge_metric`; high-score comparison uses
`high_score_is_better`.

Challenge display metadata (`challengeName`, description, star description
strings) is scenario/view metadata, not part of core `ChallengeParameters`;
view stores it alongside the loaded settings file. Champion row insertion order
is confirmed in the start-flow snippet above: add the row before seeding and
`start_fresh_scenario_spawner`.

## Lineage/history source

No separate world history is needed: `World::species_registry()` exposes the
retained species registry, and brain-dev's read-only views
(`SpeciesRegistry::species_list()`, `lineage_tree()`, `selected_lineage()`) plus
`SpeciesRecord::history` are the world-side source for lineage distribution.

## Semantics / notes## Semantics / notes

- `spawn_pellet_with_energy` and piles count as **produced** energy in the
  conservation ledger; they are free `owner_zone = None` pellets unless the
  caller uses `spawn_pellet` with an owner zone.
- Tool placement uses `ToolStore::next_save_id()` (max + 1); removal reindexes
  `ToolId`s, while save/load identity uses `save_id`. Viewers should refresh
  held ids after removal.
- `apply_settings` intentionally accepts only settings that can be rebuilt
  in-place: zone globals/zone refresh, material table/recount, decay cadence,
  spawn limits/rates, brain/vision rates, species settings. `simulationSize` and
  `simTPS` are structural and must be applied by constructing a new world.
- Settings animations are runtime state; adding/removing changers during a run
  does not rewrite base values (the game's `RequestAnimateSetting` semantics).
- Save/load UI should operate through io; core checkpoints already cover
  deterministic resume. The viewer must handle slot metadata/previews itself.

## Tests added

`world::tests::ui_*` cover pellet pile hash/ledger, tower place/attach/remove
(phero spot cleanup), kill/abort actions, safe settings apply + structural
rejection, spawner rows, and settings-changer add/remove validation.

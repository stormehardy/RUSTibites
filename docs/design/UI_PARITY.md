# UI parity inventory and core-UI plan

**Status:** design / inventory (docs-only). **Owner:** spec-dev (task-78). **Consumers:** view-dev
(viewer implementation), parity-dev (screenshots + asset extraction, task-76), Lead (scope/priority
decisions).
**Goal:** make RUSTibites' viewer visually 1:1 with The Bibites 0.6.3.1, **core UI included**,
starting after the in-world sprite renderer lands.

**Sources.** The installed game's decompiled UI scripts (`Here/UIScripts*/**`,
`Here/ManagementScripts*`, `Here/SteamIntegrations/**`, `Here/SimulationScripts.Rendering`) and the
current viewer (`crates/bibites-view/{main,app,render}.rs`, `docs/VIEWER.md`). Asset *names* have to
come from Unity prefabs/scenes and are therefore `[P]` (prefab-only) until parity-dev's
`assets/game/manifest.json` exists; this document lists the logical assets to require in that
manifest.

**Confidence tags:** `[C]` verified in decompiled C#; `[P]` prefab/scene-serialized (not in scripts);
`[SCR]` must be captured from the running game to pin layout/looks.

---

## 0. Inventory at a glance

| Measure | Count |
|---|---|
| UI source files under `Here/UIScripts*/**` | **240** (29,844 lines) |
| …of those, user-visible panel/window classes | **31** `UIPanel` subclasses + **5** `BibitePanel` subclasses |
| Additional MonoBehaviours/UI components in those dirs | **96** |
| Management/UI controllers | 10 (`GUIManager`, `UserControl`, `CameraManager`, `TimeController`, `PopupManager`, `UINavigationManager`, `TooltipSystem`, `MenuInitializer`, `RightmostPanelsManager`, `NotificationManager`) |
| Steam/menu-only UI files | 11 (`WorkshopItemsPanel`, `EditSharedItemPanel`, `WorkshopItem(Reference)`, `AchievementManager`, …) |
| User-visible surfaces catalogued here | **78** (18 shell/menu, 12 in-world HUD, 7 inspect, 9 analytics, 14 editors/tools, 10 dialogs/tooltips, 8 overlays; widgets counted separately) |
| User settings displayed in the settings menu | 56 (`Here/SettingScripts/UserSettings.cs`) |
| Scenario settings displayed in the scenario editor | 78 scalars + 4 materials + zones/groups/spawn rows/changers/challenge (`docs/spec/04-formats-settings.md`) |

**Architecture in the game:** uGUI (Canvas + RectTransform) with TextMeshPro text. Every panel
derives from `UIPanel` (`Here/UIScripts/UIPanel.cs`): `InitPanel/OpenPanel/ClosePanel/FillPanel/
UpdatePanel/ResetState`, an `escapable` flag feeding `UINavigationManager`'s ESC stack, and
`onOpen/onClose` events. Input gating is global: `UserControl.AllowControl`,
`UserControl.AllowSelection`, and a keyboard-block stack (`SetKeyboardBlockFromSource`). Time has a
string-keyed pause-source stack (`TimeController.Pauses`). Canvas scaling: `UIScaler` →
`scaleFactor = ScreenResolutionFactor * UIScale`, where auto factor = `min(h/1080, w/1920)`.
`UIPrefabsHolder` stores the shared dynamic prefabs (slider, toggle, dropdown, setting group, colour
picker, text line, condition group) and three cursor textures (edit/add/remove) `[C]`.

The in-world canvas and the main-menu canvas are separate scenes; the menu is loaded first
(`AppInitializer` → `MenuInitializer` → title screen → `MainUI`), the simulation scene replaces it
after "Continue"/"Load"/scenario start (`GameManager.StartGame`).

---

## 1. Shell / menus

Layout for every shell screen is prefab-serialized `[P]`; capture at 1920×1080 with auto UI scale
for exact anchors. All buttons/text use the shared UI atlas + TMP font.

| ID | Screen | Purpose | Key controls / fields | Data source | Priority |
|---|---|---|---|---|---|
| M1 | **Title screen** | First-launch splash; fades to main menu after ~3 s | logo/background art, version (`VersionDisplay`) | `UserSettings.FirstTimeBeingOpened`, `Application.version` | P2 (cosmetic) |
| M2 | **Main menu** | Entry point | Continue (hidden if no saves), New, Load (hidden if no saves), Scenario/Challenge browser, Workshop (Steam only), Achievements, Options, Quit; Steam user avatar+name section when Steam is active `[C]` (`MenuInitializer`) | `SaveController.AnySave()`, `SteamManager` state | **P2** (needed for full parity; not core in-world) |
| M3 | **New options** | Chooser popup | "New Simulation", "New Challenge", "Create New Bibite" (`NewOptionsSelectPanel`) | — | P2 |
| M4 | **Scenario browser** | Scenario/challenge list with preview + metadata | scenario tiles (name/desc/official/rank/stars), 400×400 preview image, challenge high-score + champion preview (gene strip); world size/density/fertility/radioactivity/limit dropdowns; `simTPS` sub-panel; "start" button `[C]` (`ScenarioSelectorPanel`, 671 L) | scenario `.zip` bundle (`scenario.info`, `img.png`, settings), `ChallengesProgress` (PlayerPrefs — out of M1) | **P2** |
| M5 | **Scenario saver** | Save current settings as a scenario | name/description, screenshot preview (captured 400×400), save target folder | `ScenarioSaver`, settings model | P2 |
| M6 | **Load game** | Pick and load a save | save list (name/date/version/auto-saves toggle), 400×400 preview, info (sim time, sim size, nBibites, nPellets), compatibility badges (compatible/unknown/incompatible), Load button `[C]` (`LoadGamePanel`) | `Savefiles/*.zip` + `scene.bb8scene` metadata; `bibites_io::load_world_contents` | **P2** |
| M7 | **Save game** | Save the current world | save list, new-name field (invalid-char validation, overwrite indicator), preview, info rows, include-templates toggle, Save `[C]` (`SaveGamePanel`) | `bibites_io::save_world_contents` | **P2** |
| M8 | **User settings** | Player/UI options | 3 tabs: **Graphics** (fullscreen, auto-resolution, resolution factor, UI scale, pheromone strength, procedural size, UI colours: fertility/background/grid/texture, grid steps/widths, node names) · **Game** (autosave on/period/real-time/reload-after-autosave/pause-after-load, save paths, include autosaves, warn before delete, selection radius, reselect-on-corpses/delay, TPS defaults, easter eggs, species span prefs, prevent sleep, min FPS, allow arrow-key camera, open zone editor on select) · **Hotkeys** (key-binding table) `[C]` (`UserSettingsManager`, 56 settings) | `UserSettings` model | P2 |
| M9 | **Workshop browser/editor** | Steam workshop items: subscribed/shared/saved, upload/edit | item list + thumbnails, subscribe/share/delete, metadata editor `[C]` | Steam API (Steam-only) | **non-goal** |
| M10 | **Achievements** | Achievement gallery | item grid (icon, name, desc, achieved state, secret icons) `[C]` | `AchievementManager` (Steam + local) | **non-goal / P3** |
| M11 | **Loading panel** | Startup/progress splash | progress bar + status text, fade in/out | asset loading progress | P3 (cosmetic) |
| M12 | **Welcome/consent popups** | First-run notices, cloud/analytics consent | text + Refuse/Allow `[C]` (`MenuInitializer`) | `UserSettings` consent flags | **non-goal** (M1 has no telemetry) |
| M13 | **Scenario editor** (full-screen) | Edit every scenario setting before saving/starting | accordion of setting groups: Simulation Options, Energy Balance, World Parameters, Genetics, Materials, Zones (+zone groups & previews), Bibites (spawn table), Settings Changers, Challenge parameters, TPS panel `[C]` (`ScenariosEditor` 531 L + `MenuDynamicSettingsManager` 309 L) | full `SimSettings` model (`settings.rs`) | P2/P3 |
| M14 | **Bibite template browser** | Pick a template for spawn table / editor / champion | template list (search/filter chrome from prefab), gene-strip preview, delete, workshop buttons, "create new" → editor `[C]` (`BibiteTemplateSelectorPanel` 398 L) | `Bibites/Templates/*.bb8template`, `Bibites/*.bb8` | P2 |
| M15 | **Bibite genome editor** | Create/edit a template | name/description (+warnings: invalid/duplicate), 35 gene sliders with preview of the bibite (procedural body/plant/meat/eye/arm sprites), tabs Genes/Brain, Save&Continue / Save&Submit `[C]` (`BibiteEditorPanel` 286 L, `BibiteEditorPreview`, `ProceduralGenePreviewer`) | `Genome` + `Brain` (`genome.rs`/`brain.rs`) | **P3** |
| M16 | **Brain editor** (inside genome editor) | Drag/connect NEAT nodes | node graph canvas (pan/zoom), node type/activation dropdown, add/remove node & connection panels, sliders for bias/weight, linear/spherical layout parameters `[C]` (`BibiteBrainEditor` 1220 L, `NodeEditor` 372, `NodeEditPanel`, `AddNodePanel`, `ConnectionEditor`, `ExpandedBrainParameters`) | `Brain` records; editor-only layout (`BrainSorter` etc. are UI-only) | **P3** |
| M17 | **Save bibite dialog** | Save the selected creature/template | name, template/save-as choice `[C]` (`SaveBibiteDialogHandle`) | `BibiteGenes.SaveState` | P3 |
| M18 | **Challenge end popup** | Result screen for a challenge run | score, stars, new-record flag, retry `[C]` (`ChallengeEndPopup`, `ChallengeTrackerPanel`) | `ChallengeParameters` + metrics | P3 (challenges are scenario-editor/P2 feature) |

## 2. In-world HUD and controls

The in-world UI is one screen with anchored panels; exact rects are `[P]`/`[SCR]`.

| ID | Surface | Purpose | Key fields / controls | Data source | Anchor | Priority |
|---|---|---|---|---|---|---|
| W1 | **Time/status readout** | Playback state | `Time HH:MM:SS`, `FPS / TPS`, `sim/real: xN.NN` `[C]` (`GUIManager.UpdateTime`) | `TimeKeeper` (`crates/bibites-view/src/time_keeper.rs`): `world.simulated_time()` in whole seconds; fps, tps and sim/real are **measured** over 0.5 s windows of real time (frames drawn, ticks that ran) and stay as they were while paused (a world loaded paused reads `0 tps`, `x0.00`) | top-left | **P0** |
| W2 | **Time speed + pause** | Playback control | log slider 0.1×–100× (display `X n.nn`), pause/play buttons; multiple pause sources | `TimeController` → viewer tick accumulator + `paused` state; the engine time scale follows the slider's target and is lowered by the minimum-FPS governor (`CheckMinFPS`, `time_keeper::min_fps_scale`), and a frame's ticks stop when the next would not fit `1 / minimum FPS` (`App::advance`) | top-left (adjacent to W1) `[P]` | **P0** |
| W3 | **Top-left button holder** | Context buttons for selected target | open-biology button, lay-egg, kill, tag/copy-tag, etc. (buttons become visible per `UITargetType`) `[C]` (`GUIManager`, `TopLeftButtonHolder`) | `BibiteRecord`/`EggRecord`, player actions | top-left beneath W1/W2 | **P0** |
| W4 | **Rightmost panels** | Side-panel host | tabs/buttons for **Dynamic Settings**, **Statistics**, **Ancestral Lineage**; exactly one open at a time `[C]` (`RightmostPanelsManager`) | settings / stats / lineage | right screen edge | **P0** (stats) / P1 (lineage) |
| W5 | **Quick statistics** | Compact live world stats (default-open; same component as A2) | count + energy wedges/pie charts (`InformationPanel`, quick instance) | `SimStats` sample, pellet counts, species counts | right, under W4 | **P0** |
| W6 | **Visibility toggles** | Rendering/overlay options | pheromones, fertility, ruler, status bars, bibite FOV, bibite targets, grid `[C]` (`VisibilityPanel`) | `UserSettings` | right/bottom panel `[P]` | **P0** |
| W7 | **Selected-target status bars** | Health/energy bars above selected bibite/egg | health/energy fill + `cur / max` text, scales with camera zoom, offset above target `[C]` (`StatusBarsReference`) | `BodyState.health/energy`, egg energies | above selected entity | **P0** |
| W8 | **Selection rectangle + highlight** | Drag multi-select / click select | rectangle overlay; selected all entities highlighted with highlight material `[C]` (`UserControl`) | `world.bibites/pellets/eggs` + camera transforms | full-screen overlay | **P0** |
| W9 | **Multi-selection panel** | Act on a rubber-band selection | entity counts + biomass per type (bibites/eggs/plants/meats), removable chips, species breakdown with per-species toggles, lay-egg/kill buttons `[C]` (`SelectionPanel`) | stores + `species` registry | centre-left modal-ish panel `[P]` | P1 |
| W10 | **Escape menu** | In-game menu overlay | Resume, Save, Load, Options, Main menu, Quit; pauses the sim + blocks input `[C]` (`EscapePanel`) | — | centre modal `[P]` | **P1** |
| W11 | **Notifications/toasts** | Transient messages (autosave, errors, achievements) | message + rank/time `[C]` (`NotificationManager`, `BaseNotification`) | events | top-right/toast area `[P]` | P1 |
| W12 | **Autopan/cinematic indicator** | Auto-panning camera target | selection of autopan entity; "auto pan" state `[C]` (`UserControl` K key, `CameraManager.followTarget`) | — | HUD indicator | P2 |

## 3. Bibite / egg inspection panels

Five `BibitePanel`s share the target/`FillPanel` lifecycle; they are opened by clicking a bibite
and via hotkeys 1–4 (`GUIManager`).

| ID | Panel | Purpose / controls | Data source | Priority |
|---|---|---|---|---|
| B1 | **Stats** | Species + tag editing (copy/paste tag), generation, length/size, mass, speed, time alive, travelled, eggs laid, alive children, bites dealt, damage dealt/received, times attacked; egg variant (size/mass/total energy/generation/progress/stored-energy sliders); lay/revive buttons; parent line `[C]` (`BibiteStatsPanel`, 348 L) | `BibiteRecord` body/meta/clock/mouth/egg_layer; species registry | **P0** |
| B2 | **Genes** | 35 gene rows vs species template with per-gene `difference from species` contribution colouring; species gene-distance; link to lineage `[C]` (`GenesPanel` + `GeneLineInfo`) | `Genome`, `SpeciesRegistry` template snapshot | **P0** (cheap: data already in core) |
| B3 | **Biology** | Largest panel (1478 L): organ schematic image (texture + masks), health slider + max/energy-density/heal data; energy slider with per-organ consumption rate sliders (digest/move/growth/metabolism/pheromone/heal/fat/fat-sustain/brain/egg) + sort toggle; growth/maturity/age sliders + details; body details (armor/muscles/fat/mass/areas); stomach contents (pellet sprites with material/amount); mouth/egg/fat/armor organ details; phero organ `[C]` | `BodyState`, `organs.rs` state, per-tick energy spend; some rate breakdowns (per-organ E/s) may need a read-only view API | **P1** |
| B4 | **Brain (simple)** | Node graph of the bibite's brain with live node values, species template positions (`nodeAnchors`), brain-distance to species, node names toggle, differences highlighting `[C]` (`BrainPanel` + `BrainPanelCommon` 520 L + `UINeuron`/`UISynaps`) | `Brain` node records + `SpeciesRegistry` template anchors | **P1** |
| B5 | **Expanded brain** | Full-screen-ish force-directed brain view: pan/zoom, node/synapse rendering, linear vs spherical layout parameter panels, start/stop simulation of layout, node/synapse selection `[C]` (`ExpandedBrainPanel` 821 L + `ExUINeuron`/`UISynaps`) | same + editor-only layout algorithm (not simulation) | **P3** |
| B6 | **Egg stats** | (variant of B1) hatch progress + stored/free energy + abort button `[C]` | `EggRecord` | P1 |
| B7 | **Species info** | Species name/description/tags, counts/energy, distribution mini-map, template preview; tabs for info/genes/brain/distribution `[C]` (`SpeciesInfoPanel`, `SpeciesGenesPanel`, `SpeciesBrainPanel`, `SpeciesDistributionPanel`, `SpeciesTabManager`) | `SpeciesRegistry` records + lineage history | P1 |

## 4. Statistics, lineage and graphs

| ID | Surface | Purpose / controls | Data source | Priority |
|---|---|---|---|---|
| A1 | **Statistics panel** (rightmost) | Full `InformationPanel`: count pie chart (`Bibites/Eggs/Plants/Meats` + total) and energy pie chart (`Free/Bibites/Eggs/Plants/Meats` + total), numeric rows; auto-refresh `[C]` (`InformationPanel` 295 L uses `WedgeInfoHandle`, `PieChartHandle`) — **note:** `StatisticsPanel`/`QuickStatisticsPanel` have no script class in the decompiled source; they are GameObjects carrying the `InformationPanel` component (two instances) `[INF]` | `world.bibites.len`, eggs, `pellets.counts`, `SimStats` biomasses | **P0** (quick) / P1 (full) |
| A2 | **Quick stats panel** | Compact copy of A1 (default open in-world) `[C]` | same | **P0** |
| A3 | **Graphs panel** | Historical data explorer: stream-group dropdown (`DataLogger.parallelDataStreams`), curve visibility toggles, line graph with legend/tooltip/zoom; gene-analysis bar graph with gene dropdown + log-like time slider `[C]` (`GraphsPanel` 114 L, `DataStreamLinesGraph` 248 L, `BarGraph`, `SingleLineGraph`, `EditableLineGraph` 507 L) | `DataLogger` streams (60 s sampling; 300 s species/zone; 3600 s genes) — **not yet ported**; viewer needs a history sampler | P1 |
| A4 | **Ancestral lineage panel** (rightmost) | Scroll of the selected species' ancestors + lineage graph `[C]` (`AncestralLineageScroll`, `LineageMeshPanel` 400 L) | `SpeciesRegistry` ancestors + template snapshots | P1 |
| A5 | **Lineage window** | Species tree: pan/zoom graph, species nodes with counts/energy over time, links to parent/children, rung legend, species panel, settings (time span/zoom) `[C]` (`LineageWindow` 260 L, `LineageTreePanel` 388 L, `LineageMesh` 768 L, `LineageSpeciesNode` 410 L, `SpeciesPanel`, `SpeciesColumn`, `LineagePanelSettingsManager`) | species registry + saved `speciesData.json` histories | P2 (big) |
| A6 | **Species distribution** | Mini-map of species positions/colours over time | `SpeciesDataPoint` histories | P2 |
| A7 | **Challenge tracker** | Current challenge label/description, exit condition text, live score, high/low-is-better, 3 star slots + descriptions, retry `[C]` (`ChallengeTrackerPanel` 102 L, `SimConditionText`, `ChallengeParametersHandle`) | `ChallengeParameters` + metric evaluation; challenge persistence is PlayerPrefs/Steam | P3 |
| A8 | **Achievement popup** | Unlock notification `[C]` (`AchievementPopupHandle`) | Steam/local achievements | non-goal |
| A9 | **Ruler / grid / fertility overlays** | Scale ruler ticks; grid; fertility tint/heat overlay `[P]` rendering (`RulerHandle`, `BackgroundManager`, `FieldOfViewDisplay`, `WeightDisplay`) | world camera + zones | **P0** (grid) / P1 |

## 5. Editors and world tools

| ID | Surface | Purpose / controls | Data source | Priority |
|---|---|---|---|---|
| E1 | **Dynamic settings panel** (rightmost) | In-world live settings editor, tabs: **Simulation**, **Zones**, **Bibites**, **Energy** (+ scenario saver button, zone preview) `[C]` (`DynamicSettingsPanel`, `DynamicSettingsTabs`, `MenuDynamicSettingsManager`) | `SimSettings` (scenario/zone/spawn tables) | P2 |
| E2 | **Zone settings editor** | Per-zone rows + groups, add/remove/duplicate, zone previews, delete-all; each zone: name/material/distribution/fertility/biomassDensity/pelletSize/renewBiomass/movement/target/speed/pos/radius/insideRadius/width/height `[C]` (`ZonesEditorPanel` 276 L + `ZoneSettingsHandle` 500 L, `ZoneGroupHandle`, `ZonePreview`) | `ZoneSettings`, `ZoneGroupSettings` | P2 |
| E3 | **Settings changers editor** | Scenario animators: list/create/edit (spline, time span, repeat, start, settings bases, condition groups); `SettingsChangerCreator` `[C]` (254+168 L) | `SettingsChanger`, `SimCondition`/metric model | P3 |
| E4 | **Challenge parameters editor** | Exit condition group + scoring metric + star thresholds + champion row `[C]` (`ChallengeParametersHandle`) | `ChallengeParameters` | P3 |
| E5 | **Materials editor** | Plant/Meat/Fat/Armor parameters (9 fields each) with preview pellet sprites `[C]` (`MatterMaterialSettingsHandle`, `MaterialParametersReference`) | `MaterialSettings` | P2 |
| E6 | **Default bibites / spawn table** | Spawn rows: template, priority, minimum, spawn type, mutation mode, growth-at-spawn, tagging/tag, zone; add/delete `[C]` (`DefaultBibitesPanel`, `BibiteSettingsHandle`) | `BibiteSpawn` rows + templates | P2 |
| E7 | **Rad tower panel** | selected rad tower: attached zone, count rate, intensity, radius, taper, id, destroy `[C]` (`RadTowerPanel`) | `ToolStore`/`RadTower` | P1 |
| E8 | **Phero tower panel** | attached zone, r/g/b strengths, heading, id, destroy `[C]` (`PheroTowerPanel`) | `PheroTower` | P1 |
| E9 | **Colour killer panel** | attached zone/spawn, hue slider, tolerance, body/eyes toggle, any/most toggle, radius (≤ simulation size), period, live progress bar, colour-spectrum + tolerance bars `[C]` (`ColorKillerPanel` 249 L) | `ColorKiller` | P1 |
| E10 | **Pedestal/tool placement** | Place rad/phero towers, colour killers, pellets, zones, bibites/eggs; cursor icons (edit/add/remove) `[C]` (`BibitePlacer`, `UserControl`/`Place` mode, `UIPrefabsHolder` cursors) | tool stores + spawn APIs | P2 |
| E11 | **Developer-mode panel** | FPS/CPU/population/debug toggles `[C]` (`Utility.DeveloperMode/PanelDeveloperModeMaster`) | — | non-goal |
| E12 | **Escape/exit dialog** | already W10 | — | P1 |
| E13 | **Scenario saver** | M5 | — | P2 |
| E14 | **Text/numeric field editors** | Shared inline editors used by E1–E6 (slider, log slider, text field, colour picker, dropdown, toggle, sub-group master toggle, condition-group editor) `[C]` (`UIScripts.SettingHandles/**`, 42 files) | settings metadata (`settings.rs::SETTINGS`) | P2 |

## 6. Dialogs, popups, tooltips and notifications

| ID | Surface | Purpose / behaviour | Priority |
|---|---|---|---|
| D1 | **Tooltip** | Cursor-follow tooltip with header + body; shown on hover for settings/buttons/scenario metadata/pellets; fade-in 0.25 s unscaled `[C]` (`TooltipSystem`, `TooltipHandle`, `TooltipTrigger`, `OnHoverShow`, `FollowLikeTooltip`) | **P0** (baseline) |
| D2 | **Pellet tooltip** | Material, amount/energy, size for hovered pellet `[C]` (`PelletTooltipHandle`, `PelletTooltipTrigger`) | P1 |
| D3 | **Graph tooltip** | Value readout at cursor on graph lines `[C]` (`GraphTooltipTrigger`) | P2 |
| D4 | **Error popup** | Red error dialog, single OK, stacks in `PopupManager` `[C]` (`ErrorPopupHandle`, `PopupManager` 372 L) | P1 |
| D5 | **Warning popup** | Warning + OK/Cancel `[C]` (`WarningHandle`) | P1 |
| D6 | **Choice/dialog popup** | Two-action modal (e.g. overwrite?/retry?) `[C]` (`ChoicePopupHandle`, `DialogHandle`, `DialogGroupHandle`) | P1 |
| D7 | **Challenge end popup** | M18 | P3 |
| D8 | **Achievement popup** | A8 | non-goal |
| D9 | **Save-bibite dialog** | M17 | P3 |
| D10 | **Loading popup / screen** | M11 | P3 |

## 7. Common widgets (build once, reuse everywhere)

From `UIScripts.SettingHandles`, `UIScripts.UIReferences`, `UIScripts.InfoHandles` (≈80 component
files): standard slider (linear/log/int/float, default-pointer, value text), text field + numeric
field, toggle, two-icon toggle, dropdown (enum/numeric/material/target-zone/toggles with helper
tooltip), tabs, setting groups & collapsible sub-groups, colour picker, pie/wedge chart, progress
bar, bar/line graph + axes + legend, scroll list pooling, item rows (save/scenario/species/bibite),
star icons, button (normal/hover/pressed/disabled), close X, cursor sprites `[C]`. These are the
first UI code to build; every panel composes them.

## 8. In-world overlays (renderer, not uGUI)

| ID | Overlay | Content | Priority |
|---|---|---|---|
| O1 | Grid + axes | optional grid (2 step sizes, colours, line widths), world axes | **P0** |
| O2 | Fertility/zone tint | zone fill/outline, fertility heat overlay | **P0** (zone outline already in viewer) |
| O3 | Selection ring/highlight | tinted ring around selected bibite, highlight on multi-select | **P0** |
| O4 | Status bars | W7 | **P0** |
| O5 | FOV cone + weights | for selected bibite: FOV cone, seen-entity lines/weights (`FieldOfViewDisplay`, `WeightDisplay`) | P1 |
| O6 | Ruler | scale ruler with ticks/labels (`RulerHandle`) | P1 |
| O7 | Pheromone particles | emitted by spots; intensity from strengths | P1 |
| O8 | Red-death bloom ring / shade band | world-edge visuals | P2 |

---

## 9. Core-UI subset recommendation (what ships first)

Screens that make the viewer feel like the game without the editor/menu mountain. Each phase is
shippable and screenshot-diffable.

**Lead decisions (2026-10-30) — applied to this plan:**

1. **Scope cut confirmed:** in-world **P0/P0b/P1 is "core UI"** and ships first; menus/settings are
   **P2**; editors **P3**. The viewer must be usable/faithful in-world before shell menus.
2. **Read-only first:** P0/P0b/P1 allow only select / pause / speed / camera / overlay toggles. No
   world mutation (kill, lay egg, tag, tool placement, live settings/tower edits) — those become a
   later **sandbox-tools phase** gated on determinism + parity review. Panels render read-only state;
   action buttons are hidden/disabled until then.
3. **Graphs/lineage data:** use a **viewer-side ~60 s ring-buffer sampler**; do **not** port the
   game's `DataLogger` history streams into core for M1 (future work; exact history persistence, if
   ever needed, goes through the checkpoint sidecar, not the game save).
4. **Challenge/achievement persistence:** out of scope for M1 (render-only if trivial, otherwise
   skip).
5. **Fallbacks kept:** the current simple HUD stays as `--simple-hud`; the procedural capsule
   renderer stays as `--no-game-assets` so the viewer works without extracted art.

### P0 — "watch and inspect" (≈2–3 weeks after in-world sprites; read-only)
HUD (W1, W2, W3 read-only), rightmost quick stats (W5), visibility toggles (W6), status bars (W7),
selection rect/highlight (W8), grid (O1), zone/fertility (O2), selection ring (O3), bibite **Stats**
(B1) and **Genes** (B2) panels, basic tooltips (D1), pause/speed/step hotkeys.
**Acceptance:** 1080p screenshot diff of HUD + Stats/Genes panels vs the game within a few pixels of
layout; all values match core state; clicking/pan does not perturb determinism.

### P0b — "full inspection" (≈2–3 weeks; read-only)
Biology panel (B3; needs per-organ energy-rate read API), Brain simple (B4), egg variant (B6), full
statistics (A1), multi-selection (W9, select-only), escape menu (W10, navigation only), tool panels
(E7–E9) **read-only display** (no editing until the sandbox phase), pellet tooltip (D2),
error/warning/choice dialogs (D4–D6), notifications (W11).
**Acceptance:** every in-world panel reachable by the game's hotkeys/buttons renders with real data
and cannot mutate the world.

### P1 — "analyse" (≈2–3 weeks; read-only)
Graphs (A3; 60 s viewer-side sampler + gene bars), ancestral lineage (A4), species panels (B7),
brain-distance/differences view (B4 extras), ruler/FOV/pheromone overlays (O5–O7).
**Acceptance:** graphs/lineage show the same aggregate curves as `SimStats`/registry (not the game's
exact rolling histories).

### P2 — "menus and world editing" (≈3–4 weeks)
Main menu + load/save with preview images (M2, M6, M7), scenario browser (M4), user settings (M8),
dynamic settings + zone/materials/spawn editors (E1, E2, E5, E6), tool placement (E10), scenario
saver (M5), scenario editor (M13), template browser (M14), lineage window (A5/A6).
**Acceptance:** start a world from a scenario entirely through the UI; save/load round-trip through
the panels; settings edits mutate `SimSettings` live and persist through the save path (editing here
is intentional — the read-only gate applies to the in-world viewer, not the menu editor).

### P3 — "authoring" (≈4–6 weeks)
Genome editor (M15), brain editor (M16), expanded brain (B5), settings changers (E3), challenge
tracker/parameters/end (A7, E4, M18), save-bibite dialog (M17), title/loading polish (M1, M11).
**Acceptance:** create a bibite/brain, save as template, spawn it in a scenario.

**Sandbox-tools phase (after P1, before P2 editing polish):** enable world mutations in the viewer
(kill/lay/tag/tool placement/settings edits) behind a flag, with a determinism note and parity
review; core already supports these through store/tick APIs.

**Non-goals for visual M1:** Steam workshop, Steam achievements/cloud, analytics/consent dialogs,
localisation, developer-mode panel, UI sound effects (can be added later), exact TMP text effects
and LeanTween animations (visual parity target is static layout + state, animations P3 polish).

---

## 10. Implementation mapping to `bibites-view`

Current crate (alpha): `main.rs` (macroquad window/loop/input), `app.rs` (loaded world, camera modes,
selection, HUD data aggregation, speed/pause/step, screenshot), `render.rs` (world scene →
primitives, software rasteriser + macroquad backend), `assets.rs` (manifest loading, game
ProceduralSprites CSV/sheet layout, gene→sprite-index formulas, colour/tint helpers) and `sprites.rs`
(layered body/mouth/eyes/arms/exoskeleton, eggs, pellets; both backends draw sprites). Flags:
`--assets/--no-assets` (game art vs procedural fallback), `--ortho-size`, `--compare-ref`. The UI
additions should not disturb the simulation path: **UI only reads `World` between ticks and mutates
it only through explicit player actions.**

```
crates/bibites-view/src/
  main.rs                  window loop (unchanged); draws world then UI, routes input UI-first
  app.rs                   App { world, camera, selection, speed/pause, ui: UiState, input }
  render.rs                world rendering (unchanged)
  ui/
    mod.rs                 UiState { open_panels, focused, esc_stack, tooltip, toasts, ui_scale }
    theme.rs               palette, font handles, 9-slice sprite lookup, layout constants
    atlas.rs               assets/game/manifest.json → sprite handles (+ rect fallback)
    input.rs               hotkeys, mouse capture (UI-over check), ESC stack, pause-source stack
    widgets.rs             slider/toggle/dropdown/tabs/button/text-field/pie/graph primitives
    hud.rs                 W1/W2/W3/W6/W7/W12 (always-on overlays in screen space)
    panels/
      stats.rs             B1 (+ B6 egg variant)
      genes.rs             B2
      biology.rs           B3
      brain.rs             B4 (B5 later)
      selection.rs         W9
      quick_stats.rs       A1/A2 (pie/wedge charts)
      graphs.rs            A3 (history sampler + line/bar graphs)
      lineage.rs           A4/A5/A6
      settings.rs          E1/E2/E5/E6 (generated from settings metadata)
      tools.rs             E7/E8/E9
      escape.rs            W10
      load_save.rs         M6/M7 (+ 400×400 preview render)
    dialogs.rs             D4–D6, D9, notifications W11
    sprites.rs             logical UI-sprite names → atlas entries
```

**Retained vs immediate:** use **immediate-mode drawing** (macroquad) for layout/state, with
**retained caches** only for expensive derived data — graph series, lineage layout, brain node
positions, settings editor widget tree. This matches the game's model: panels refill on open/target
change (`FillPanel`) and only light `UpdatePanel` work runs per frame; the UI never owns simulation
state.

**Text.** The game uses TextMeshPro. Extract the TMP font assets (2 Font objects) with parity-dev and
convert to TTF (or use the SDF atlas if feasible); fallback to macroquad's default font until then.
Reference sizes derive from `UIScaler`: `scale = min(h/1080, w/1920) * uiScale` at a 1080p design
canvas; store layout constants at 1080p and multiply.

**Sprites.** All panel chrome/button/toggle/slider/icon sprites are prefab-serialized `[P]`; the
manifest from task-76 exposes them under `assets/game/ui/...` with 9-slice borders and pivots.
**The authoritative logical asset list, manifest aliases and current coverage are in
`tools/assets/ui_expectations.json` + [`docs/ASSETS_UI.md`](../ASSETS_UI.md)** (242 expectations;
215 matched in the first extraction; only the colour-killer spectrum material outstanding).
Headline groups: shared 9-slice chrome (`ui/panel/*_frame` → `UISprite`/`gray_panel*`/`Long_UI`),
`ui/button/button` → `Button_UI`, sliders → `bars`/`tiled_bars*`, fonts →
`Roboto-Regular`/`Roboto-Bold`, brain icons (`ui/icon/brain_input/00..32`,
`brain_output/00..14`, `brain_function/00..12`) and achievements (`ui/icon/achievement/ACH_*`).
Colour-tinted materials: colour-killer spectrum/tolerance bars (`_hue`/`_width` shader
equivalents). Read 9-slice borders/pivots from the manifest, not hardcoded constants.

**Layout anchors.** Prefab rects are `[P]`; start with the game's grouping: HUD cluster top-left,
rightmost panel stack right edge, bibite panel tabs/panels bottom/centre (capture D2/D3 to pin),
modals centre, tooltips cursor-following, selection rectangle screen-space. Keep every rect in
`theme.rs` so screenshot-diff iterations are cheap.

**Input.** Reproduce the game's map (Appendix B): `H` UI toggle, `Space` pause, `1`–`4` bibite
panels, click = select nearest within `selectionRadius*zoom`, drag = rubber band, `K/Z/R/G/O/E`
camera/target selection, `Insert` lay egg, `Delete` kill, `F5` quick save, `Ctrl+C/V` tag copy/paste,
`Esc` = close top escapable panel or open escape menu, right-drag pan, wheel/middle-drag and
`+/-` zoom, `PageUp/PageDown` pan speed, `Home` reset camera, arrow-key pan (optional). UI consumes
pointer first; a modal/panel blocks world selection and hotkeys exactly like
`UserControl.AllowControl`/`KeyboardControlBlockStack`.

**Determinism and player actions.** The viewer must remain bit-identical to headless for the same
seed/inputs. **M1 viewer is read-only** (Lead decision 2026-10-30): only selection, pause/speed,
camera and overlay toggles are allowed; UI reads snapshots (`BibiteView`/records/`SimStats`/stores)
between ticks and never mutates the world. World-changing actions (kill/lay egg/tag/tool
placement/settings edits) are deferred to a separate **sandbox-tools phase**; when that lands they
are explicit commands applied at a tick boundary (never from render code) and recorded as "player
actions" for deterministic captures.

**Fallbacks (Lead decision):** `--simple-hud` keeps the current lightweight HUD when the themed UI
is unavailable; `--no-game-assets` keeps the procedural capsule renderer (no extracted sprites) so
the viewer always runs. Both should be selectable at runtime, not compile-time.

**Data gaps to close with owners (read-only view APIs):**
1. Per-organ energy-flow rates for the Biology panel (game exposes `EnergyFlowRate`, per-organ
   consumption); needs a `World`/`BibiteView` read API (~small, body-dev).
2. Graphs/lineage history: **decided — viewer-side ~60 s ring-buffer sampler**, not a core
   `DataLogger` port. Sample counts/biomass/ages/deaths/egg-counts (60 s), species/zone aggregates
   (300 s) and gene means (3600 s) from `SimStats`/registry snapshots; note in docs that curves are
   equivalent-in-spirit, not byte-equal to the game's history streams. Porting the game streams is
   future work and would go through the checkpoint sidecar, not the game save format.
3. Save/load panel previews need an offscreen 400×400 world screenshot (software rasteriser already
   exists in `render.rs`; add a 400×400 camera render) — P2 only.
4. Challenge/high-score/achievement persistence is PlayerPrefs/Steam: **out of scope for M1**
   (render-only if trivial, otherwise skip the surfaces).
5. Tooltips' helper texts are in C# `Setting` definitions, not in save files. `settings.rs` carries
   labels/units but not every `HelperText`; port the helper strings (docs-only table) if tooltips
   must match word-for-word — P0b dependency for tooltip content; layout can ship with labels first.

---

## 11. Screenshot / reference needs (for parity-dev, task-76)

Requested captures (game 0.6.3.1, 1920×1080 window, auto UI scale, default user settings unless
stated; store under `assets/game/reference/ui/`):

| # | Filename | Setup |
|---|---|---|
| 1 | `ui/01_title.png` | first launch (title splash) |
| 2 | `ui/02_main_menu.png` | main menu, ≥1 save present |
| 3 | `ui/03_main_menu_new.png` | "New" chooser popup open |
| 4 | `ui/04_scenario_browser.png` | scenario list + selected scenario preview (Default) |
| 5 | `ui/05_challenge_browser.png` | challenges list + champion/high-score section |
| 6 | `ui/06_load_game.png` | load panel with saves + preview + compatibility badges |
| 7 | `ui/07_save_game.png` | save panel, name field focused/empty + overwrite indicator |
| 8 | `ui/08_settings_graphics.png` | options → Graphics tab |
| 9 | `ui/09_settings_game.png` | options → Game tab |
| 10 | `ui/10_settings_hotkeys.png` | options → Hotkeys tab |
| 11 | `ui/11_scenario_editor_sim.png` | scenario editor → Simulation Options expanded |
| 12 | `ui/12_scenario_editor_zones.png` | scenario editor → Zones tab with previews |
| 13 | `ui/13_scenario_editor_bibites.png` | spawn-table rows expanded (E6 detail) |
| 14 | `ui/14_scenario_editor_materials.png` | materials group |
| 15 | `ui/15_template_browser.png` | template picker list + gene-strip preview |
| 16 | `ui/16_genome_editor_genes.png` | bibite editor, Genes tab (one slider focused) |
| 17 | `ui/17_brain_editor.png` | brain editor tab with a loaded brain + layout params |
| 18 | `ui/18_hud_default.png` | Ring/Default world running, nothing selected (HUD + rightmost quick stats) |
| 19 | `ui/19_hud_selected_bibite.png` | bibite selected + Stats panel (status bars visible) |
| 20 | `ui/20_panel_genes.png` | Genes panel open for the selected bibite |
| 21 | `ui/21_panel_biology.png` | Biology panel open (scroll top + energy details expanded) |
| 22 | `ui/22_panel_brain.png` | Brain panel open |
| 23 | `ui/23_panel_stats_full.png` | rightmost Statistics panel open |
| 24 | `ui/24_panel_graphs_history.png` | Graphs → historical data stream + legend |
| 25 | `ui/25_panel_graphs_genes.png` | Graphs → gene analysis bar graph |
| 26 | `ui/26_lineage_window.png` | lineage tree window with a grown species tree (canonical: grow a `Default.zip` run; historical `lol.zip` fixture is opt-in only, `docs/TESTING_POLICY.md`) |
| 27 | `ui/27_species_panel.png` | species info panel for a selected species |
| 28 | `ui/28_selection_multi.png` | rubber-band selection panel (several bibites selected) |
| 29 | `ui/29_tool_panels.png` | rad tower panel + colour-killer panel (one capture each: `29a`, `29b`) |
| 30 | `ui/30_escape_menu.png` | escape menu overlay |
| 31 | `ui/31_quick_stats.png` | quick statistics wedges/pie close-up |
| 32 | `ui/32_tooltip_setting.png` | hover a setting with its tooltip visible |
| 33 | `ui/33_fov_overlay.png` | selected bibite with FOV cone/weights and ruler toggled on |
| 34 | `ui/34_def_popup.png` | warning/choice popup example |
| 35 | `ui/35_loading.png` | loading progress panel |

**Asset extraction requests (same task):** export **all UI sprites from level scenes /
`globalgamemanagers`** (panels, buttons, sliders, toggles, tabs, icons, stars, cursors, graph chrome,
pie/wedge art, title/menu backgrounds) plus the **TMP font assets** and the **brain input/activation
icons** (`BrainIconHolder.brainIcons[33]`, `brainFunctionsIcons[13]`) and achievement icons. Record
each sprite's `rect/pivot/PPU/9-slice borders` in the manifest — 9-slice borders are required to
reproduce panel corners. Materials for the colour-killer spectrum/tolerance gradients should be
exported or their shader colours documented.

**Extraction status (2026-10-30): the first batch landed** — `assets/game/manifest.json` now has 475
UI entries (266 unique UI sprite names, 22 fonts) and the in-world sprite manifest. The expectation
checker reports **215/242 UI expectations matched by alias** (the game's own asset names differ from
our logical names; `tools/assets/ui_expectations.json` carries the alias mapping). Remaining:
`ui/material/color_spectrum` (hue gradient shader/colour ramp) and optional items (14 achievement
icons the game itself lacks, title-screen art). Details and follow-ups:
[`docs/ASSETS_UI.md`](../ASSETS_UI.md).

---

## 12. Effort estimates (person-days, one dev familiar with macroquad + this engine)

| Phase | Scope | Estimate | Main risks |
|---|---|---|---|
| UI foundation | `ui/` skeleton, atlas/theme/font, widgets, input/ESC/pause stacks, debug layout overlay | **5–7** | font conversion, 9-slice fidelity |
| P0 | HUD, quick stats, selection/status bars, grid/overlays, Stats+Genes panels, tooltips | **6–9** | layout capture iteration |
| P0b | Biology (needs rate API), Brain simple, egg variant, full stats, multi-select, escape, tool panels, dialogs | **8–12** | per-organ energy-rate API, graph/layout data |
| P1 | Graphs (+viewer-side 60 s sampler), ancestral lineage, species panels, FOV/ruler/phero overlays | **10–15** | sampler fidelity, lineage layout |
| P2 | Menus, load/save+previews, scenario browser, user settings, dynamic settings/zones/materials/spawn editors, tool placement, scenario saver/editor, template browser, lineage window | **15–22** | editing APIs + live settings application, 400×400 previews |
| P3 | Genome editor, brain editor, expanded brain, changers/challenge editors, save-bibite, polish | **20–30** | drag-graph UX, editor-only layout algorithms |
| Verification | screenshot-diff harness (per screen), interaction tests, determinism checks | **5–8** | stable captures in headless CI |

**Bottom line:** the core in-world UI (P0 + P0b) is **≈14–21 person-days** after asset extraction,
plus **5–7** for the shared UI foundation; a "visually 1:1 including menus/editors" goal is
**≈60–90 person-days**. Everything is gated on parity-dev's UI sprite/font extraction and screenshot
set (task-76); without those only approximate rectangles/colours are possible.

---

## 13. Decisions (Lead, 2026-10-30) and remaining implementation questions

**Decided:**

| # | Decision |
|---|---|
| 1 | **Scope:** in-world P0/P0b/P1 = "core UI", ships first; menus/settings P2; editors P3. Viewer must be faithful in-world before shell menus. |
| 2 | **DataLogger:** viewer-side ~60 s ring-buffer sampler for graphs/lineage; do **not** port the game history streams into core for M1 (future work / checkpoint sidecar if exact persistence is ever needed). |
| 3 | **Player actions:** read-only viewer first (select/pause/speed/camera/overlay toggles only) so the viewer can never diverge the sim; kill/lay/tag/place/settings edits = later sandbox-tools phase, gated on determinism + parity review. |
| 4 | **Challenge/achievement persistence:** out of scope for M1 (render-only if trivial, else skip). |
| 5 | **Fallbacks:** keep the current simple HUD as `--simple-hud` and the procedural renderer as `--no-game-assets`. |
| 6 | **UI sounds/LeanTween animations:** out of scope for static visual parity (later polish). |

**Remaining implementation questions:**

1. **Save/load panel previews:** offscreen 400×400 render for a first pass, or placeholder? (Affects
   only P2.)
2. **Font licensing:** extracted TMP fonts are game assets — include under the same
   `assets/game/` provenance/license note as parity-dev's extraction (not MIT). Confirm wording.
3. **Tooltip helper-text port:** do we port the C# `HelperText` strings verbatim (largest content
   lift for tooltips) or ship labels/units only in P0 and add helper text later?
4. **Tool panels (E7–E9) read-only display in P0b** — confirm showing values without controls is
   useful enough, or defer those surfaces to the sandbox phase.

---

## Appendix A — `Here/UIScripts` file → inventory map

| File group | Lines | Inventory IDs |
|---|---|---|
| `UIScripts/*.cs` (89 files) — panels, widgets, tooltips, editors | 13,163 | M2/M4/M6/M7/M10/M13/M14/M15/M16/M17/M18, B1/B2/B4/B5/B7, W3/W10/W12, E1/E3/E4/E7/E8/E9/E10/E11, D1–D9 |
| `UIScripts.UIPanels/*.cs` (9 files) | 1,856 | B3 (BiologyPanel 1478), B6/egg, E1 (DynamicSettingsPanel/tabs), W6 (VisibilityPanel), A4 (AncestralLineageScroll), B5 scroll |
| `UIScripts.UIReferences/*.cs` (41 files) | 3,796 | A3 (GraphsPanel), M4 (ScenarioItemReference), M6/M7 (SaveItemReference), D1/D2/D4–D6, graph widgets; note `InformationPanel` (A1) and `StatusBarsReference` (W7) are in `UIScripts/` |
| `UIScripts.UIReferences.Graphs/*.cs` (8 files) | 1,215 | A3 graph implementations |
| `UIScripts.UIReferences.LineagePanel/*.cs` (13 files) | 2,487 | A4/A5/A6, D8, B7 (species panel) |
| `UIScripts.SettingHandles/*.cs` (42 files) | 5,423 | E1/E2/E5/E6/E14, M8 (UserSettingsManager), M13 (ScenariosEditor + MenuDynamicSettingsManager), M4 dropdowns |
| `UIScripts.SettingHandles.SettingDisplays/*.cs` (4) | 56 | E14 value displays |
| `UIScripts.SettingHandles.References/*.cs` (11) | 518 | E14 widget references |
| `UIScripts.InfoHandles/*.cs` (21 files) | 1,247 | B1/B3 value handles, A1 pie/wedge, E5, challenge text |
| `UIScripts.CustomGraphics/*.cs` (2) | 83 | mask shader for chip art |
| `ManagementScripts/*` UI controllers | ~3,200 | W1–W12, D1–D6, M1–M3, M11, A9 |
| `ManagementScripts.SceneManagers/*` | ~500 | scenario start/menu scene wiring; Patreon splash (non-goal) |
| `SteamIntegrations/*` UI (11 files) | ~3,900 | M9, M10, D8 — non-goal |
| `SimulationScripts.Rendering/*` overlays (3 files) | 438 | O5, O6 (visual overlays) |

## Appendix B — Keyboard / mouse map (verified in `Here/ManagementScripts`)

*Read-only viewer (M1) uses only the navigation/selection subset. Action keys marked † are deferred
to the sandbox-tools phase (Lead decision 2026-10-30); they remain listed for eventual parity.*

| Input | Action | Source |
|---|---|---|
| `H` | toggle whole in-world UI canvas | `UserControl.Update` |
| `Space` | pause/resume | `UserControl` → `TimeController.TogglePauseGame` |
| `1` | bibite Stats panel | `GUIManager.Update` |
| `2` | bibite Genes panel | `GUIManager.Update` |
| `3` | Biology panel (bibite) / Brain panel (egg) | `GUIManager.Update` |
| `4` | Brain panel (bibite) | `GUIManager.Update` |
| `Esc` | close topmost escapable panel, else open escape menu | `GUIManager`, `UINavigationManager`, `UIPanel` |
| left click | select nearest bibite around cursor (radius = `SelectionRadius × orthographicSize`) | `UserControl` |
| left drag (>0.15 s) | rubber-band multi-select (bibites, eggs, pellets) | `UserControl` |
| right drag | pan camera | `CameraManager` |
| middle drag / wheel | zoom camera | `CameraManager` |
| `+`/`=` , `-` | zoom in/out | `CameraManager` |
| `PageUp`/`PageDown` | camera pan-speed scale | `CameraManager` |
| `Home` | reset camera + deselect | `CameraManager` |
| arrow keys (optional) | pan camera (`UserSettings.allowArrowKeyCameraControl`) | `CameraManager` |
| `K` | select auto-panner | `UserControl` |
| `Z` | select random zone | `UserControl` |
| `R` | select random bibite (Ctrl held: repeat on death) | `UserControl` |
| `G` | select highest-generation bibite | `UserControl` |
| `O` | select oldest bibite | `UserControl` |
| `E` | select random egg | `UserControl` |
| `Insert` † | lay egg (target or multi-selection) | `UserControl` |
| `Delete` † | kill target or multi-selection | `UserControl` |
| `F5` † | quick save | `UserControl` |
| `Ctrl+C` / `Ctrl+V` † | copy family tag / paste tag onto target | `GUIManager` |
| `F12` (viewer) | screenshot | current viewer |

## Appendix C — Panel enums (authoritative order)

```
BibitePanels     { None, StatsPanel, GenesPanel, BrainPanel, ExpendedBrainPanel, BiologyPanel }
RightmostPanels  { DynamicSettingsPanel, StatisticsPanel, AncestralLineagePanel }
DynamicSettingsTabs { SimulationTab, ZonesTab, BibitesTab, EnergyTab }
UITargetType     { None, Bibite, Egg, Zone, ColorSelector, PheroTower, RadTower }
GraphType        (simple vs expanded brain layout)
```

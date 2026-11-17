# UI functional parity — every user-facing game UI surface vs the RUSTibites viewer

**Purpose.** The user reports that many viewer buttons do nothing. This document is the ground-truth
inventory of **every user-facing UI surface and interaction in the game** and what the current
`bibites-view` implementation actually does. It is the work matrix for the UI rounds that follow; it is
not a visual-diff document (`VIEWER.md` covers geometry/IoU).

**Implementation status snapshot (`8c30538`; viewer 96/96, core 457; 0.2.3 released (SHA-256 `5e33e6aa856c0b60b5d501d3ba50f9b1f33c818d1414eb8819089e571263e4e4`)):** **Works:**
title menu; New Game; Library; Settings pages + **hotkey rebinding**; save dialogs/dynamic rows; in-world
Dynamics/Settings values; lineage graph + species tabs; KILL dialog; scenario editor; egg TAG; gene/brain
editors incl paging/mutate/anchors/template save; challenge selector/champion/embedded params; **challenge
tracker/end popup**; **challenge parameters/star editor**; Workshop local browser + **local edit/online
status**; zone APIs; local achievements. **Closure criteria met.** **Unavoidable limits:** exact editor skin/
visual fidelity, unavailable game rung history, champion pre-seed ordering, Workshop metadata follow-ups.
**Out of scope:** Steam achievements/Workshop online/leaderboards/overlay, audio/UI sounds, online services.
**0.2.2 historical; 0.2.3 released (SHA-256 `5e33e6aa856c0b60b5d501d3ba50f9b1f33c818d1414eb8819089e571263e4e4`).** **Works = game-observable effect.**

**Ground truth:** `Here/` decompiled C# (655 files), especially `Here/UIScripts/**` (~87 scripts),
`Here/UIScripts.UIPanels/**`, `Here/UIScripts.SettingHandles/**`, `Here/ManagementScripts/**`,
`Here/SettingScripts/**`, `Here/SimulationScripts/**`, `Here/SteamIntegrations/**`.

**Viewer references:** `crates/bibites-view/src/{main,menu,app,hud,ui,ui_runtime}.rs`,
`docs/VIEWER.md`, runtime captures under `assets/game/reference/` and `tools/ui/`.

**Status key**

| Status | Meaning |
|---|---|
| **works** | Same interaction produces the same **game-observable effect** — expected panel/dialog/state change — not merely a hit rectangle or click. |
| **partial** | Some of the surface exists; controls, data, or edge cases are missing/only rendered. |
| **inert** | The control is drawn/found but its action is not wired (this is the user-visible “button does nothing” class). |
| **missing** | No viewer surface/action exists. |
| **n/a** | Purely visual/decorative or intentionally out of scope (audio/Steam). |

**Node paths.** `Here/` C# scripts contain serialized field references, not full hierarchy paths; Unity
scene YAML is not in `Here/`. Parity's `assets/game/reference/functional_ui/` capture now supplies
24 sessions with full control paths, active/enabled state, and a **165-label → path map**
(`label_path_manifest.json`), including the 10 main-menu routes (`UI/NewGameScenarios`,
`UI/LoadGamePanel`, `UI/SettingsPanel`, `UI/BibiteSelector`, `UI/WelcomeScreen/{AchievementsPanel,
ToolsPanel,CreditsPanel}`, `UI/SettingsPanel/{Graphics,Gameplay,Controls}`). Paths below use that manifest
where captured; remaining scene-bound-only panels are marked with their serialized reference.

---

## 1. Ranked summary — what to fix first

| Rank | Surface cluster | Why it matters | Current viewer state |
|---:|---|---|---|
| P0 | **In-game selection + inspector (Stats/Genes/Biology/Brain)** | Core moment-to-moment loop; user clicks a bibite and expects all panels. | Tabs **work** (exact runtime sessions), KILL/LAY/SAVE/EDIT/REVIVE actions work, and the lineage graph panel works; Species views/expanded editors deferred. |
| P0 | **Playback/HUD controls** (pause/play/step/speed/overlay toggles) | First thing a user touches. | Pause/step/speed and G/Z/P/O/H keys **work**; every active widget is now hit-tested via the generic registry; unmapped commands/values can still no-op. |
| P0 | **Main menu rows + Settings** | Entry point; dead rows are immediately visible. | **Works:** all 9 rows navigate; New/Load lists load; Library selector flow; Settings applies/persists overlay+TPS subset; Achievements/Tools/Credits info. **Partial:** New Game apply behavior, remaining value widgets, editor flows. |
| P1 | **Save Game panel + autosave** | Users must be able to save progress. | **Works (2026-11-06, the prefab's panel: `save_panel.rs`, see `PARITY_REMAINING.md`):** name field with live validation, the list, the world's picture and counts, overwrite / delete questions, a fresh `img.png` in every save; Save World/Quick Save. |
| P1 | **New Game / scenario selector + challenge/template choice** | Scenario choice for new games. | **Works:** local scenarios, option rows, Create Scenario & Start generation tested; challenge/template-editor flow still deferred. |
| P1 | **Settings categories applied to the world** | Full settings surface expected. | **Works (dropdown subset):** captured dropdowns open and scene-option choices apply; menu overlay/HUD/TPS persist; other setting values deferred. |
| P1 | **World tools: pellet placer, Rad/Phero towers, ColorKiller** | Main sandbox/tools interactions. | **Works (placement):** tool palette selects placer/tower/killer and click placement mutates the world; property panel value edits/delete still partial. |
| P2 | **Zones/scenario editor + settings changers** | Scenario authoring and live parameter editing. | **Works (core):** scenario editor working copy/apply/save + zone add/remove tested; full editor polish/settings changers partial. |
| P2 | **Lineage/Species panels** | Long-run evolution inspection. | **Works (lineage graph):** LineageTree graph/nodes/selection; Species UI deferred. |
| P2 | **Bibite editor / brain editor / template selector** | Custom bibite creation and library. | **Works (basic editors):** Library selector, gene edit/select/randomize/mutate, brain add/remove node/synapse + validate; deferred skin/scroll/drag/anchors/mutate settings/template save. |
| P3 | **Challenges, achievements, Bibite Library, workshop, Additional Tools, Credits** | Secondary menus. | **Partial:** Library selector works; Achievements/Additional Tools/Credits informational; challenges/workshop missing. |
| P3 | **Shared widgets: dropdowns, sliders, toggles, tooltips, popups, escape stack** | Everything above is built from these. | **Partial:** generic hit registry/action framework, panel stack, popups (KILL dialog) and captured-session renderer landed; dropdown/toggle/slider proposals tested, but apply path and tooltips remain partial. |

---

## 1.1 Captured functional UI sessions (`4159253`) — paths now available

Parity captured **24 new sessions** without disturbing the existing runtime sessions. Evidence:
`assets/game/reference/functional_ui/` (`manifest.json`, `label_path_manifest.json`,
`*_full/overlay/_world_1920x1080.png`). `manifest.json` lists every control (path/name/active/
enabled/interactable/is_on/value); `label_path_manifest.json` maps **165 visible labels** to
session+path and includes the 10 main-menu label routes.

| Session | Game surface | Key path / label routes | Viewer status |
|---|---|---|---|
| `main_menu_functional_title` | title + main rows + New Game/Scenario selector background | all title/NewGame controls; 748 controls, 4358 nodes | works (all rows; subset actions) |
| `main_menu_new_game` | New Game/Scenario selector (`UI/NewGameScenarios`) | `menu_labels["New Game"]` | works (scenario picker; options registered) |
| `main_menu_load_game` | Load panel (`UI/LoadGamePanel`) | `menu_labels["Load"]` | works (simple list/load) |
| `main_menu_settings` | Settings categories (`UI/SettingsPanel`) | `menu_labels["Settings"]` | works (scene subset+persist) |
| `main_menu_settings_graphics` | Settings/Graphics (`UI/SettingsPanel/Graphics`) | `menu_labels["Settings/Graphics"]` | missing (captured; not rendered) |
| `main_menu_settings_gameplay` | Settings/Gameplay (`UI/SettingsPanel/Gameplay`) | `menu_labels["Settings/Gameplay"]` | missing (captured; not rendered) |
| `main_menu_settings_hotkeys` | Settings/Hotkeys (`UI/SettingsPanel/Controls`) | `menu_labels["Settings/Hotkeys"]` | missing (captured; not rendered) |
| `main_menu_bibite_library` | Bibite library (`UI/BibiteSelector`) | `menu_labels["Bibite Library"]` | works (selector flow) |
| `main_menu_achievements` | Achievements (`UI/WelcomeScreen/AchievementsPanel`) | `menu_labels["Achievements"]` | works (informational) |
| `main_menu_additional_tools` | Additional Tools (`UI/WelcomeScreen/ToolsPanel`) | `menu_labels["Additional Tools"]` | works (informational/link list) |
| `main_menu_credits` | Credits (`UI/WelcomeScreen/CreditsPanel`) | `menu_labels["Credits"]` | works (informational) |
| `main_menu_new_panel` | New options/panel variant | new-panel controls | missing |
| `main_menu_workshop` | Workshop panel | workshop controls | n/a Steam |
| `main_menu_scenario_editor` | Scenario editor/settings tuner | scenario editor controls | missing |
| `main_menu_bibite_editor_genes` | Bibite editor Genes tab | 1181 controls, 6352 nodes | missing |
| `main_menu_bibite_editor_brain` | Bibite editor Brain tab | 1181 controls, 6352 nodes | missing |
| `main_world_toolbar` | In-world toolbar | toolbar controls | works (toolbar dispatch) |
| `main_world_tool_placer` | Pellet placer + tool frame | 944 controls, 7874 nodes | works (panel + click placement/property values) |
| `main_world_tool_user_actions` | Additional Tools/user actions | kill-half/remove-half controls | partial (panel/info; action coverage partial) |
| `main_world_tool_view` | View/overlay tools | display toggles | partial (panel; overlay toggles work) |
| `main_world_dynamic_parameters` | Live simulation parameters | settings handles | partial (panel; value widgets partial) |
| `main_world_lineage` | Lineage/species side panel | lineage tree controls | works (graph panel/node selection) |
| `main_world_information` | Information/statistics panel | info/graph controls | works (stats/info panel) |
| `main_world_settings_panel` | In-world settings panel | settings controls | partial (panel; value widgets partial) |

Capture note: hidden-only plugin activate/press routes; sessions include inactive/disabled controls in the
manifest, so “active” must be checked before treating a label/path as reachable. All existing sessions were
preserved.

## 1.2 Behavioral ground truth (`bf8a3fd`)

`assets/game/reference/functional_ui/behavioral_reference.json` records **34 real click actions** on the
hidden game: clicked node paths, before/after counts/tools/zones, active panels, new labels/texts and
dialogs. `manifest.json` links the behavior file; `label_path_manifest.json` supplies the paths. Several
menu labels had **0 hits after panel resets** (Bibite Library, Additional Tools, Load); their functional
sessions still cover the panels, so status must be judged by **panel/dialog state**, not click count.
**Refined path presses (`c23ccd4`):** `behavioral_reference.json` now has a `refined_actions` list of 8
**path/name-pressed** menu rows and inspector actions after dismissing the KILL confirmation:
`ButtonsPanel/Editor` → `UI/BibiteSelector/SelectionPanel` (Create New/Template Bibites/Accept + template
names); `Section/More` → `UI/WelcomeScreen/ToolsPanel`; `ButtonsPanel/LoadSim` →
`UI/LoadGamePanel` + `SelectedPanel`; KILL(clean)/KILL dismiss and clean Lay Egg/Save/Edit states. This
resolves the earlier zero-hit labels (Library/Additional Tools/Load) — judge them by the opened
panel/dialog, not the original label hit count. Some inspector follow-up label hits are false after dialog
dismissal; use the clean/path actions for those rows.

Key confirmed effects:

| Game click | Confirmed behavioral effect | Viewer status |
|---|---|---|
| `menu:Settings` | Opens `UI/SettingsPanel` (+ Graphics selector/slider panels) | **works (subset/persist)** — menu settings view opens; captured panel; values subset only |
| `menu:Achievements` | Opens `UI/WelcomeScreen/AchievementsPanel` | **partial informational** |
| `menu:Credits` | Opens `UI/WelcomeScreen/CreditsPanel` (+ credit sections) | **works** — scrolls (wheel 35, live bar), opens scrolled 622.96 as the prefab stores it, the five social buttons open `URLManager`'s pages; the Patreon simulation scene is not available |
| `menu:Bibite Library` (path `Editor`) | Opens `UI/BibiteSelector/SelectionPanel` with template list + Create New/Accept | **partial** — viewer Library view is a local template list, not the game template selection panel |
| `menu:Additional Tools` (path `More`) | Opens `UI/WelcomeScreen/ToolsPanel` with external-tool links/dialogs | **works** — scrolls (wheel 35, live bar), wrapped paragraphs, the wiki and Discord links open; the Patreon simulation and the sprite editor are other scenes of the game (not available) |
| `menu:Load` (path `LoadSim`) | Opens `UI/LoadGamePanel` + `SelectedPanel` | **works (simple load)** — viewer lists/loads saves without preview/version/delete |
| `world:ViewButton` | Opens `VisibilityPanel` (Show Fertility/Pheromones) | **missing** visibility panel; only overlay/via keys |
| `world:DynamicParametersButton` | Opens `DynamicSettingsPanel` (Simulation Options/Corpses controls) | **partial** panel/session renders; `SetValue` edits unbound |
| `world:AncestralLineageButton` | Opens ancestral lineage/species panel | **partial** lineage session renders; species binding missing |
| `world:InformationPanelButton` | Opens `UI/MainLayer2/StatsPanel` | **partial** HUD counts only; info panel not mapped |
| `placer:PlacePellets` | Opens `PlacePelletPanel` + Material slider; pellet placement mode | **partial** tool panel renders; placement unbound |
| `placer:PheroTower` / `placer:RadTower` | Change active tool state; next world click creates tower (`pheroTowers`/`radTowers` grows) | **missing** tool state/placement actions |
| `inspector:Kill` (clean/dismiss) | Opens **“Killing half the population … Are you sure???”** confirmation (`KILL`/`SAVE`); dismiss clears it; follow-up label hits may be false after dismissal | **missing** confirmation dialog/action |
| speed slider (`UICanvas/.../SliderPanel/Slider`) | `/d/slider_set` ground-truth speed change | **partial** `SetSpeedSlider` action exists; slider binding incomplete |
| inspector Lay/Egg/Save/Edit/dismiss | Opens corresponding panels/dialogs; some label hits depend on selection state | **partial/missing** corresponding action/dialogs |

These effects define the acceptance bar for “works”: a viewer control must open the same panel/dialog or
change the same world/settings state, not just be drawn/hit-testable.

## 1.3 Control behavior ground truth (`bc217f4`)

`assets/game/reference/functional_ui/control_behavior.json` (linked from `manifest.json`) captures exact
control values/options and before→after effects; parity also exposes `/d/control_dump` and
`/d/control_set`. This is the implementation contract for the deferred value-level controls.

**Settings:** 8 Toggle/Slider/Dropdown actions with paths and label contexts. Examples: two toggles
`false→true` (Show Node Names/Grid group), two `true→false` (Fullscreen/Auto resolution), Pheromones
Strength **0.1→0.5** (10%→50%), Procedural Size dropdown options exactly **`Genetic / Maturity / Both`**
(index 0→1), UI Scale options **`Smaller (0.5) / Small (0.75) / Normal (1.0) / Large (1.15) / Larger
(1.25)`** (reset to 0). Dropdown options are exact; label contexts are nearest atlas names/values.

**Load/Save browser:** Load panel texts `Include autosaves`, `New Game`, `Version:`, `Last edit:`,
`Informations`, `Sim Time/Size`, `Number of Bibites/Pellets`, `Load`; buttons Load/Delete/Close. Save panel
`New Save`, `Enter name…`, `Save`; buttons Save/Delete/Close. Hidden lab had only the sample/New Game slot,
so **no overwrite/delete dialog was triggered**; captured images/buttons are the contract.

**Placer properties:** `PlacePelletPanel` texts `Material`, `Pellet Size Factor`, `Pellet Number`, material
dropdown options exactly **`Plant` / `Meat`** (value 0), pellet-size log slider value 0 (min -1/max 3),
pellet-count slider value 1 (min 1/max 10, whole numbers), plus `CancelPelletPlacement`.

**KILL dialog:** full text `Killing half the population`, warning `You're about to kill half of all bibites
(each … 50% chance … Are you sure???)`, buttons **`Cancel` / `YES`**; popup path
`UICanvas/MainUI/TopLayer/Popuholder`. **Inspector Lay/tag gaps:** `LayBibiteEggButton` was not found in the
active StatsPanel in this pass (counts unchanged) and `EditTagButton` was not found — label/path gaps
documented, not assumed.

Current viewer: the settings/save-load value widgets, KILL confirmation dialog, egg save and tag edit remain
**deferred/partial**; this section is the exact target, not a claim they are wired.

## 1.4 New tranche (`f4d1467` + APIs `c95f98f`/`3094a27`/`abf047e`)

Current `cargo test -p bibites-view` is **72 passed / 0 failed** on this tranche. What is actually
implemented/tested, judged by game-observable effect:

- **KILL dialog — WORKS.** `ConfirmKind`/`RequestKill`/`RequestKillHalf`/`Confirm`/`Cancel` with the game
  wording rendered as a modal (`Kill this bibite? Are you sure???` / `Killing half the population…`); test
  `kill_half_dialog_confirms_and_cancels`.
- **Settings value controls — PARTIAL.** Captured toggle/slider/dropdown metadata + pointer proposals are
  tested (`settings_value_metadata_and_pointer_proposals`); `apply_control_value` applies only
  grid/pheromone/pellet/zone/HUD/TPS keys today. Other captured values (colors, resolution, fullscreen,
  procedural size, most gameplay values) are not applied yet.
- **Save/load browser — PARTIAL.** `bibites_io::storage::SaveBrowser` rows, thumbnails, autosave, rename and
  delete are tested (`save_browser_save_load_quick_slot_and_fingerprint`,
  `save_list_entries_metadata_thumbnails_and_autosave`); viewer `SaveAs/LoadSelectedDelete/Rename` commands
  exist. The captured browser panel's row/thumbnail controls are not yet bound end-to-end in the viewer.
- **Tool property widgets — REPLACED (2026-11-04).** The invented "TOOL" box and `AdjustTool` are gone: the pellet
  placer has the game's own panel (`placer_panel.rs`) and the towers / colour killer theirs (`tower_panel.rs`); see
  `PARITY_REMAINING.md`.
- **Egg save/tag — PARTIAL.** `SaveEggs` and `SetTag` commands work (`tag_and_egg_save_commands_work`); the
  in-panel tag edit field/refresh UI is not rendered/bound.
- **Library selector — PARTIAL.** Menu Library lists local templates; the game `UI/BibiteSelector/
  SelectionPanel` selector (preview/template accept/edit) is not fully wired.
- **Lineage drawing — DEFERRED.** Panel/session + lineage layout/data APIs exist; no graph rendering yet.
- **Genome/brain editor APIs — APIs PRESENT, UI DEFERRED.** `.bb8`/`.bb8template` read/write/export and
  graph/lineage view APIs exist; gene/brain editor screens and mutation UI are not implemented.

## 1.5 Library / New Game / Scenario editor inventory (`54d4b50`)

12 additional sessions + overlays: `library_selector_flow`, `library_create_new`,
`library_editor_genes_flow`, `library_editor_brain_flow`, `library_editor_brain_after_add`,
`newgame_scenario_options`, `newgame_new_scenario_button`, `newgame_new_challenge_button`,
`scenario_editor_flow`, `scenario_editor_zones`, `scenario_editor_ChallengeOptions`,
`scenario_editor_sim`. `library_newgame_behavior.json` links control paths/options/label contexts.

**Exact New Game dropdown options** (paths under
`UI/NewGameScenarios/ScenarioSelector/.../ParametersPanel/Content/Options`):

| Setting | Options (captured index) |
|---|---|
| Limit Bibites | `Fifty / One Hundred / Two Hundred / Five Hundred / No` (No=4) |
| Radioactivity | `None / Lower / Low / Normal / Severe / Extreme` (Normal=3) |
| Global Biomass Density | `Barren / Sparser / Sparse / Low / Normal / High / Dense / Denser / Packed` (Normal=4) |
| Global Fertility | `Sterile / Arid / Low / Normal / High / Lush / Overgrown` (Normal=3) |
| Simulation Size | `Smallest / Small / Normal / Bigger / Large / Larger / Enormous / Gigantic` (Normal=2) |

**Known library/newgame paths:** `UI/BibiteSelector/.../LeftSide Buttons/CreateNew`, `.../Templates`,
`.../Preview/Right Side Buttons/Edit Button`; New Game
`UI/NewGameScenarios/.../ParametersPanel/Content/Bottom/StartButton`; New Scenario
`UI/NewGameScenarios/.../Scenario Panel/Button holder/New Scenario Button`.

**Viewer status:** New Game **works as a basic local scenario picker** (no dropdown option UI yet); Library
is **partial** (local template list; captured selector/CreateNew/Templates/Edit paths not wired);
gene/brain editor sessions are captured but **editor screens remain deferred**; scenario editor sessions are
captured but **no viewer scenario editor**. Gap: randomize and brain add-node/synapse button labels were not
exposed by the control dump (inventory captured, clicks unresolved).

## 1.6 Recovered crash WIP (`a321c3d`) — implemented/tested vs deferred

Core gate: **448 core tests**, clippy/fmt clean at `a321c3d`. This tranche adds data/API/actions; it does
**not** complete the editor screens.

- **Genome randomize/mutate + brain-editor graph ops — IMPLEMENTED/TESTED (core APIs).** Tests:
  `genome::tests::randomize_is_seeded_and_respects_ranges`,
  `genome::tests::random_color_only_touches_color_genes`,
  `brain::tests::editor_add_edit_and_remove_graph_parts`. Viewer gene/brain **editor screens remain
  deferred**.
- **Zone add/remove/persistence — IMPLEMENTED/TESTED (core/world).** Test
  `world::tests::ui_zone_add_remove_and_persistence`; viewer zone-editor UI remains partial.
- **Dynamic save rows — IMPLEMENTED/TESTED (viewer + IO).** `SaveBrowser` rows/thumbnails/autosave/rename/
  delete/quick-slot covered by `bibites-io` tests; viewer HUD renders dynamic rows with `SelectSaveRow`,
  Load / Delete actions (the Save Game panel is `save_panel.rs` since 2026-11-06, with its overwrite / delete questions and thumbnail).
- **Dropdown selection — IMPLEMENTED/TESTED.** Captured dropdown opens, option rows emit
  `PickDropdownChoice`; test `tool_panel_adjustments_and_dropdown_picks_apply` applies a scene-option choice.
  Exact settings options beyond scene toggles/TPS stay partial.
- **Tool-panel adjustments — REPLACED (2026-11-04).** `AdjustTool` was removed with the invented "TOOL" box; the
  pellet placer's material / size / count (`PelletMaterial`, `PelletSlide`) and the tower panels' controls are tested
  in `app` and `placer_panel` / `tower_panel`.
- **Editor regressions — TESTED (core).** Same brain/genome tests above; viewer editor interaction remains
  the deferred part.

**Still deferred:** lineage graph drawing; full gene/brain editor screens; Library selector flow; New Game
option dropdowns UI; Save overwrite/delete dialogs. **Historical package note:** this tranche predated the
0.2.0 package; authoritative 0.2.0 values are in 1.10.

### 1.6.1 Authoritative editor/save/new-game paths (`b607160`)

`gap_flows.json`, `save_dialogs.json`, refreshed `library_newgame_behavior.json`/manifest resolve the
previously missing paths:

- **Library selector:** CreateNew `UI/BibiteSelector/SelectionPanel/Holder/Panels/LeftSide Buttons/CreateNew`;
  Edit `UI/BibiteSelector/SelectionPanel/Holder/Preview/Right Side Buttons/Edit Button`;
  Templates remains `.../LeftSide Buttons/Templates`.
- **Bibite editor:** `UI/BibiteSelector/BibiteEditor/Panel/Buttons/{Genes,Brain,SaveAndContinue,Submit}`;
  gene sliders edited via `/d/control_set`.
- **Save panel:** name input + `SaveButton` paths and the test save lifecycle recorded in
  `save_dialogs.json`; New Game option lists + `StartButton` path retained.
- **Editor gaps:** randomize and brain add-node/synapse buttons are **not present in the active editor UI**
  (documented absence, not an unresolved control dump).

These paths are inventory/contract for view work; they do **not** make the editor screens implemented. The
viewer status remains: Library selector flow deferred, gene/brain editor screens deferred, Save
overwrite/delete dialogs deferred.

## 1.7 Release-candidate state (`1505f92`)

Gate: **HEAD `1505f92`; viewer 77/77; core 449; io/cli green; clippy/fmt clean.** Release viewer/CLI rebuilt
in `target/release`, desktop shortcut verified to launch the new viewer, and headless
`--menu --screenshot` works. Newly **works (game-observable effect):**

- **Library selector** — real menu flow with Create New / template list / Accept / Edit; test
  `menu::tests::library_selector_rows_and_actions`.
- **Dynamic save rows** — SaveBrowser rows/actions rendered; row ids tested
  (`ui_runtime::tests::dynamic_row_ids_are_stable_and_distinct`); Load/SaveAs/Delete/Rename; overwrite/delete
  dialogs deferred.
- **Dropdown value selection** — captured dropdown opens, option rects/choices tested; **New Game option
  registration** (Limit Bibites/Radioactivity/Biomass Density/Fertility/Size) hit-testable
  (`ui_runtime::tests::library_and_newgame_controls_are_hit_testable_with_dropdown_values`); full New Game
  apply behavior is next tranche.
- **Tool property panels** — material/count/energy/radius/hue/attach-zone adjustments tested
  (`tool_panel_adjustments_and_dropdown_picks_apply`).
- **Lineage graph** — `LineageTree` panel draws nodes and selects/inspects
  (`lineage_panel_refreshes_and_selects_nodes`).
- **KILL dialog** — game wording, Yes/No (`kill_half_dialog_confirms_and_cancels`).
- **Zone add/remove APIs** — core/world `ui_zone_add_remove_and_persistence`; viewer zone-editor actions
  still partial.
- Genome randomize/mutate + brain add/edit/remove APIs remain tested; **full editor screens deferred**.

**Historical next steps at this checkpoint:** New Game behavior + scenario editor (both landed later). The
0.2.0 package and certification are recorded in 1.10.

## 1.8 Scenario editor core / New Game behavior / egg TAG (`fe833d1`,`15def58`,`d634037`,`452a965`)

Gate verified here on the current worktree: **viewer 80/80, core 450, `cargo fmt --check` clean**. These are
now **works** (game-observable effect):

- **Scenario editor core** — opens a settings working copy, apply/save flow, zone add/remove; test
  `scenario_editor_apply_zone_and_dirty_flow`.
- **New Game behavior** — option rows cycle and **Create Scenario & Start** generates a fresh scenario from
  the selected options; test `newgame_options_cycle_and_generate_a_world`.
- **Egg inspector** — egg selection gives the game's inspector (Stats with the egg section, Genes, Brain; the tag is edited in the Stats panel);
  the old "Set TAG / SAVE egg" box is gone (see `PARITY_REMAINING.md`, "Egg inspector"); test `egg_tag_field_sets_and_clears` covers the commands.

**Deferred after this point:** gene/brain editor polish (see 1.9), exact lineage rung data, challenges/Steam,
and any TAG edit beyond the egg inspector. **0.2.0 is released; see 1.10 for authoritative hashes.**

## 1.9 Gene/brain editor screens (`8f344be`) — basic editing works

Gate verified here: **viewer 82/82**. The Library editor screens now provide observable editing:

- **Genome editor** — opens on the selected bibite/starter, lists genes, adjusts selected gene, **Randomize**
  and **Mutate** change the working genome; test `gene_editor_edits_and_randomizes`.
- **Brain editor** — opens the working brain, lists nodes, **Add Node / Add Synapse / Remove Node** mutate the
  graph, **Validate** reports editor validation; test `brain_editor_adds_nodes_synapses_and_validates`.
- Panel entry/draw/hit paths exist for GeneEditor/BrainEditor, and **Save** is present as a UI action.

**Deferred polish (documented, not works):** scrolling beyond 14 genes; exact editor skin; node drag/anchors;
mutate-settings wiring; template save/continue from the editor (editor `Save` currently reports deferred).
These do not block the basic editor screen claim.

## 1.10 Historical 0.2.0 package (`603646b` snapshot)

| field | value |
|---|---|
| artifact | `target/release-package/RUSTibites-0.2.0-windows-x64.zip` |
| size | **124,051,485 bytes** (118.31 MB) |
| SHA-256 | `a6cc1396a126cdd5ecb89bbf990c0244b782d250dec57589806209f01b895341` |
| packaged VERSION | RUSTibites 0.2.0, snapshot commit `603646b`, built 2026-11-03T05:50:33Z, rustc 1.99.1 |
| `rustibites.exe` | 2,252,288 B, SHA-256 `D00B7CBA5B9E2F8CA99BDDB9D498547CC98E25A6BAF3343AA12828A38EDFBC0A` |
| `rustibites-view.exe` | 3,036,672 B, SHA-256 `D458538D6DAFD18C20D64210EEC59D584D90017C1CF651DD3ADA10B87202CB52` |

Bench verified the package; Lead independently re-extracted it and confirmed packaged `--menu --screenshot`
hashes `E533D78A…` and packaged CLI Default 600 t exits 0 (1→10 bibites). **Packaged docs may lag the
external zip hash** because packaging embeds docs before the final self-referential hash update;
`SHA256SUMS.txt`/this table are authoritative. 0.1.0 (`b201c1b7`, 38,496,196 B) remains historical.

## 1.11 Complete settings inventory (`db5504d`)

`settings_controls.json` extends `control_behavior.json` with every active settings control and reset path.
All values below are captured hidden-only; session overlays are listed in `manifest.json`.

| Page/session | Value controls | Texts / buttons | Reset paths | Notes |
|---|---|---|---|---|
| `menu_graphics` (`GraphicsButton`) | **9** — 5 Toggle, 1 Slider, 3 Dropdown | 39 / 26 | 7 | Exact dropdowns: `Genetic/Maturity/Both`, UI scale `Smaller…Larger`, resolution `720p/768p/900p/1080p/1440p/4K` |
| `menu_gameplay` (`Gameplay Button`) | **16** — 11 Toggle, 4 Slider, 1 Dropdown | 44 / 25 | 6 | Autosave dropdown `One Minute…Ten Hours` |
| `menu_hotkeys` (`Controls Button`) | **0** value controls | 27 / 15 keybind buttons | 1 | Keybinds are buttons/input fields, not toggle/slider/dropdown |
| `world_dynamic_parameters` (`DynamicParametersButton`) | **19** — 11 Toggle, 8 Slider | 32 / 35 | 7 | Includes simulation options + top-centre speed slider |
| `world_settings` (`SettingsButton`) | **19** — 11 Toggle, 8 Slider | 32 / 35 | 7 | Same in-world settings surface via Settings button |

Each control records path, current value (`is_on`/value/selected index), options, and label context; each
reset path points at the per-setting ResetButton. This is the implementation contract for the P0 settings
gap in [`UI_REMAINING_GAPS.md`](UI_REMAINING_GAPS.md), not evidence that the viewer binds those values yet.

## 1.12 Settings pages implemented (`abc92d9`)

Gate declared at clean `abc92d9`: **viewer 84/84**, runtime atlas refreshed to **49 sessions / 2,252
controls** (`tools/ui/ui_runtime_atlas.index.json` session count and `viewer_control_ids.json` control count).
Settings pages are now **works**:

- **Graphics / Gameplay / Hotkeys** pages bind captured controls to viewer `UserPreferences` (fullscreen,
  resolution, UI scale, grid/pheromone/background, autosave, min FPS, pause/reload, selection/camera, etc.),
  with page switching and preference persistence; test `settings_pages_map_values_and_persist_preferences`.
- Hotkey page renders captured hotkey text blocks; keybinds remain button/input fields, not value widgets.

**Next/in-progress — not works until view commits land:** in-world **DynamicParameters/Settings value
binding** (19 controls each; panels render/keyword-dispatch only) and **editor polish** (gene scroll >14,
exact skin, node drag/anchors, mutate-settings wiring, template save/continue). Save overwrite/delete dialogs
landed separately in §1.13. **0.2.1 is released (§1.15); 0.2.0 is historical.**

## 1.13 Destructive save dialogs implemented (`9e1d632`)

Gate verified here: **viewer 85/85**. Save-browser **overwrite** and **delete** now require confirmation:

- `ConfirmKind::{OverwriteSave(name), DeleteSave(path)}` feed the same modal/HUD path as KILL; Cancel leaves
  the save untouched, Confirm performs save-as/overwrite or delete.
- Test `destructive_save_dialogs_confirm_and_cancel` and the `save_panel` tests; manual paths are the Save Game panel's Save over an existing name and
  `Delete` in the Load/Save browser.
- This closes the save-dialog item in [`UI_REMAINING_GAPS.md`](UI_REMAINING_GAPS.md). Thumbnail/preview and
  full browser polish remain lower-priority detail.

**Still next/in-progress, not works:** in-world **DynamicParameters/Settings value binding** (19 controls
each; panels/keyword-dispatch only) and **editor polish** (gene scroll >14, exact skin, node drag/anchors,
mutate-settings wiring, template save/continue). **0.2.1 is released (§1.15); 0.2.0 is historical.**

## 1.14 0.2.1 tranche works (`74c4aef`)

Gate verified here: **viewer 86/86, core 453**. Newly **works** (game-observable effect):

- **Settings pages** — Graphics/Gameplay/Hotkeys bind to `UserPreferences` with persistence.
- **Save overwrite/delete dialogs** — `ConfirmKind::{OverwriteSave,DeleteSave}`; confirm/cancel tested.
- **In-world Dynamics/Settings value binding** — captured labels map to `ScenarioSettings`/`IndependentSettings`
  and `World::apply_settings`; test applies Bibite Limit and Global Biomass Density live
  (`world_settings_controls_apply_live_and_gene_pages_scroll`). Structural fields report “staged” rather than
  silently applying.
- **Gene paging + mutate wiring** — gene editor pages by 14 (`ScrollGene`), and Mutate calls
  `Genome::mutate_gene` with `SimSettings`/`MutationFactor`; same test covers paging, editor tests cover mutate.
- **Scenario editor core / egg TAG / gene+brain editor screens** already work (1.8/1.9); this tranche retains
  them.

**Remaining after 0.2.1:** editor template save/continue, exact editor skin, node drag/anchors; exact lineage
rung/distribution + Species; secondary panels/challenges (see `UI_REMAINING_GAPS.md`). **0.2.1 is released
(§1.15); 0.2.0 is historical.**

## 1.15 Historical 0.2.1 package (`86239a8` snapshot)

| field | value |
|---|---|
| artifact | `target/release-package/RUSTibites-0.2.1-windows-x64.zip` |
| size | **134,539,435 bytes** (128.31 MB) |
| SHA-256 | `55fdfce6df3fcc897132c32e8b0cbaf18f287c2a8283fca2c3ab9e0089ae7fd4` |
| packaged VERSION | RUSTibites 0.2.1, snapshot `86239a8`, built 2026-11-03T07:08:26Z, rustc 1.99.1 |
| `rustibites.exe` | 2,253,312 B, SHA-256 `7E00391CBCFF0006650110FC093CB6706385090B58E0C7FDF6AB37B3095D228C` |
| `rustibites-view.exe` | 3,081,728 B, SHA-256 `AC756621BEFE418DE1DBCB091C8C8881986E8CBA5C86F6D59700F4C3DFEAEFFE` |

Gate at package HEAD: **viewer 87/87, core 453**. Packaged `--menu --screenshot` = `E533D78A…`, packaged CLI
Default600 exit0 (1→10), atlas 49 sessions. Adds Settings pages, destructive save dialogs, in-world
Dynamics/Settings value binding, gene paging, mutate wiring, scenario editor/egg TAG/basic editors, plus
secondary-panel improvements (local achievement progress, abort-eggs utility; `86239a8`). Packaged docs may
lag the external zip hash; `SHA256SUMS.txt`/this table are authoritative. 0.2.0 table in 1.10 and 0.1.0
values are historical.

## 1.16 Final polish — closed in 0.2.3

All four final-polish items landed in the `8c30538` tree and are covered by tests:

- **Hotkey rebinding** — capture/assign/reset/persist through `UserPreferences`;
  `hotkeys_rebind_persist_and_workshop_edit_status`.
- **Workshop local edit + online status** — local rename/edit status and explicit “Steam/online not
  available” status (`hotkeys_rebind_persist_and_workshop_edit_status`); Steam Workshop remains out of scope.
- **Challenge parameters/star editor** — supported timer/star thresholds edit and apply on challenge start
  (`challenge_parameters_editor_adjusts_and_applies_to_start`).
- **Pre-seed champion ordering** — follow-up limit; champion spawn/start works but pre-seed ordering was not
  captured from the game.

**Closure criteria met:** all planned implementable UI work has landed; remaining limits are documented as
unavoidable/out-of-scope below. **0.2.3 package released (authoritative).**

## 1.17 Historical 0.2.2 package (`7158aad` snapshot)

| field | value |
|---|---|
| artifact | `target/release-package/RUSTibites-0.2.2-windows-x64.zip` |
| size | **155,568,731 bytes** (148.36 MB) |
| SHA-256 | `f7d9851638719847e20dd05db3f7c8e62026247a189f9ecf87e98048ee74b36c` |
| packaged VERSION | RUSTibites 0.2.2, snapshot `7158aad`, built 2026-11-03 |
| `rustibites.exe` | 2,253,824 B, SHA-256 `898A218A91779B5556EC534170390733E30A1217D2910E27FDA7E5BCCAFADB4C` |
| `rustibites-view.exe` | 3,168,768 B, SHA-256 `CA2A6D4684218C5CE65B767B6C3291F359E988BAD2C776DABD3BFEBD36F42B50` |

Gate at package HEAD: **viewer 94/94, core 457, io/cli green, clippy/fmt clean**. Packaged menu deterministic
(`E533D78A…`), packaged CLI Default600 exit0 (1→10), **60** runtime sessions. 0.2.2 adds challenge
tracker/end popup, challenge selector/champion picker + embedded params, Workshop local browser,
species/lineage tabs, editor anchors/template save, plus 0.2.1 features. Packaged docs may lag the external
hash; `SHA256SUMS.txt`/this table authoritative. 0.2.1 in 1.15 and 0.2.0 in 1.10 are historical.

**Final polish next (not works until view commits):** hotkey rebinding, Workshop edit controls,
challenge parameters/star-description editor, pre-seed champion ordering. **Unavoidable limits:** exact
editor skin (visual fidelity), unavailable game rung history (source data not captured), Steam/audio/online
out of scope.

## 1.18 0.2.3 closure (released)

`8c30538` is the final closure tree: **viewer 96/96**, core **457** verified locally. 0.2.3 includes the
challenge parameters/star editor, hotkey rebinding/persistence, Workshop local edit/online-status, plus all
0.2.2 features (challenge tracker/end/selector/champion/embedded params, Workshop browser, species/lineage
tabs, editor anchors/template save, Settings/save/in-world values).

**Functional closure criteria met.** All planned implementable panels/actions are landed; **full UI + visual parity remains open on the visual axis** (SDF/menu blur/editor skin), see §1.19. Remaining items are documented **unavoidable limits** (exact editor skin/visual fidelity, unavailable game rung history, champion pre-seed
ordering, Workshop metadata follow-ups) and **out of scope** (Steam achievements/Workshop online/leaderboards/
overlay, audio/UI sounds, online services). **0.2.2 historical; 0.2.3 released (156,609,848 B / `5e33e6aa856c0b60b5d501d3ba50f9b1f33c818d1414eb8819089e571263e4e4`).**

## 1.19 Conjoint visual + functional parity requirement

**Explicit requirement:** visual parity and full UI functionality are a **single acceptance target**. A pixel
metric does not establish functional parity, and a working/hit-tested control does not establish visual parity.
Both axes must be tracked and reported together before claiming complete UI parity or closing the goal.

| Surface | Functional status | Inert controls | Visual reference | Visual status |
|---|---|---|---|---|
| Title/menu rows | works | none known after 0.2.3; deferred menu blur/TMP only | native 60-session UI-on-black refs + menu background | menu blur/TMP SDF open |
| Settings pages | works/prefs | value-level widgets done; exact skin open | native UI-on-black/controls refs | SDF/TMP open |
| In-world HUD/toolbar/panels | works | deferred panel polish only | native in-world UI-only refs (partial/full-alpha) | SDF/tune against native in-world |
| Inspector tabs | works | none known | corrected per-tab native UI-only refs | SDF open |
| Library/New Game/scenario editor | works | none known | native UI-on-black refs | menu blur/SDF open |
| Gene/brain editors | works basic | deferred anchors/skin | native? no exact editor capture | exact editor skin open |
| Challenge/Workshop/lineage | works | metadata follow-ups | native UI-on-black/in-world refs | SDF/menu blur open |
| Save browser/KILL dialogs | works | overwrite/delete dialogs work | old refs only | no native dialog ref |

**Tracking rule:** [`UI_REMAINING_GAPS.md`](UI_REMAINING_GAPS.md) and this matrix must list functional coverage
**and** inert controls alongside visual metrics/reference coverage. `UI_VISUAL_REFERENCE_MAP.md` tracks 60/60
native refs and pixel-authority; it is not a functional-parity claim.

## 2. Main menu and title flow

Game entry: `MenuInitializer.Start` (MenuInitializer.cs) loads settings/sprites, hides/shows Steam-only
rows (`hideWithSteam`, `showWithSteam`, `continueButton`, `loadButton`, `achievementButton`), then
`MenuManager.SetActiveAllPanels`; `MenuManager` simply toggles panels. `MenuManager` has no action logic —
the row targets are scene-bound Unity `Button.onClick` targets.

| ID | Surface / interaction | Game script · class | What it does · data edited | Control node / ref | Viewer status | Priority / deps |
|---|---|---|---|---|---|---|
| M1 | Continue | `ManagementScripts/GameManager.cs` · `GameManager.Continue`; `MenuInitializer.continueButton` | Loads latest save; hides button if `SaveController.AnySave()` is false. Reads save index. | `main_menu_functional_title` control set (Continue has no label→path entry); serialized `MenuInitializer.continueButton`; viewer runtime row | **works** — Continue loads the first save; all 9 rows navigate; latest/autosave selection polish limited | P0; needs save index |
| M2 | New Game | `UIScripts/NewOptionsSelectPanel.cs` · `NewOptionsSelectPanel.NewSimulation` | Opens `ScenarioSelectorPanel`, closes new-options panel; enters new simulation flow. | session `main_menu_new_game`, path `UI/NewGameScenarios`; viewer row | **works** — New opens the functional scenario picker and starts the selected scenario | P0; needs scenario selector |
| M3 | Load Game | `UIScripts/LoadGamePanel.cs` · `LoadGamePanel` | Opens save list (autosave filter, preview, version flags, load/delete). | session `main_menu_load_game`, path `UI/LoadGamePanel`; refs `savesHolder`, `saveItemPrefab`, `loadButton` | **works** — Load list/selected save loads; previews/version/delete still deferred | P0; metadata + delete |
| M4 | Bibite Library | `UIScripts/DefaultBibitesPanel.cs`, `BibiteTemplateSelectorPanel.cs` | Lists saved/default bibites and templates; add/delete/edit/select. | session `main_menu_bibite_library`, path `UI/BibiteSelector`; refs `bibiteItemsHolder`, `templatesHolder` | **works (Library selector flow)** — Create New/templates/Accept/Edit tested; full library mutation actions partial | P1; template store |
| M5 | Settings | `UIScripts.UIPanels/DynamicSettingsPanel.cs`, `SettingsChangersEditor.cs`, `UIScripts.SettingHandles/**` | Opens settings tabs (Simulation/Zones/Bibites/Energy), applies `ScenarioSettings`/`UserSettings`; animator changers. | session `main_menu_settings`, path `UI/SettingsPanel`; submenus `UI/SettingsPanel/{Graphics,Gameplay,Controls}`; viewer Settings rows | **works (subset)** — menu settings apply/persist; exact game value options now captured in 1.3, value-level widgets still deferred | P0; settings API + UI widgets |
| M6 | Achievements | `UIScripts/AchievementPanel.cs`, `SteamIntegrations/AchievementManager.cs` | Achievement grid/icons from Steam; click/hover details. | session `main_menu_achievements`, path `UI/WelcomeScreen/AchievementsPanel`; refs `achievementItemPrefab`, `holder` | **works (local progress)** — Achievements panel renders 7 locally-derived progress rows; Steam unlock/sync out of scope | P3 |
| M7 | Additional Tools | `UIScripts/UserActions.cs` (KillHalfBibitesPressed, RemoveHalfPelletsPressed); utility panels | Opens tool dialogs/actions (kill half, remove half, dev tools). | session `main_menu_additional_tools`, path `UI/WelcomeScreen/ToolsPanel`; in-world `main_world_tool_user_actions` | **works (informational)** — Additional Tools links/list; external links not actionable | P2 |
| M8 | Credits | scene/`VersionDisplay.cs` (version string) + credits UI | Shows credits/version/links; version text from `VersionDisplay`. | session `main_menu_credits`, path `UI/WelcomeScreen/CreditsPanel`; title session also shows version/credits text | **works (informational)** — credits list | P3 |
| M9 | Quit | `GameManager`/scene | Quits application. | viewer row | **works** | P0 |
| M10 | Loading panel | `UIScripts.UIReferences/LoadingPanel.cs` (`SetActive`, `UpdateProgress`) | Startup/loading progress and world-open transition. | viewer loading panel/menu Loading view | **partial** — startup/menu loading works; no progress phases/settings load | P2 |
| M11 | Welcome popup | `MenuInitializer.welcomePopup` (`firstLoad && AppInitializer.firstTimeEver`) | First-time welcome/popup. | scene-only | **missing** | P3 |
| M12 | Steam/user section | `MenuInitializer.steamUserSection`, `steamUsername`, avatar; `SteamManager` | Shows Steam name/avatar/workshop entry. | scene-only | **n/a** (Steam intentionally out of scope) | P3 |
| M13 | Title/main-menu visual/row geometry | `MenuManager`, captured runtime `main_menu_title`/`main_menu_settings` | Row layout/title/version/credits. | viewer runtime sessions | **works** (exact committed title/menu layout; dark backdrop for missing art) | — |

### New Game / New Options selector

| ID | Surface / interaction | Game script · class | What it does · data edited | Node / ref | Viewer status | Priority / deps |
|---|---|---|---|---|---|---|
| N1 | New options choice panel | `NewOptionsSelectPanel` | Buttons: New Simulation, New Challenge, Create New Bibite; opens selector/editor. | refs `scenarioPanel`, `selector` | **missing** | P1 |
| N2 | Scenario selector list + preview | `ScenarioSelectorPanel` | Scans scenario folder, shows official/workshop/selected scenario, preview image/description, challenge list, start button. Reads `ScenarioIndependentSettings` defaults. | refs `scenarioItemPrefab`, `scenariosHolder`, `selectedItem`, `challengesHolder`; scene-bound | **works** — scenario picker + option rows + Create Scenario & Start generation tested | P1; scenario scan IO |
| N3 | New-game world dropdowns | `ScenarioSelectorPanel.sizeDropdown` / `densityDropdown` / `fertilityDropdown` (`FloatSettingDropdown`) | Edits `ScenarioIndependentSettings` SimulationSize, biomassDensity, pelletGrowth before start. | settings handles; scene-bound | **works** — option rows cycle and generate a fresh scenario; exact options retained | P1; settings write |
| N4 | New-game TPS sliders | `ScenarioSelectorPanel` `simTPSSlider`/`brainTPSSlider`/`decayTPSSlider`/vision lookup/sensing sliders (`LogIntSettingSlider`) | Sets sim/brain/decay/vision TPS at new-game start; feeds `ScenarioSettings`. | setting refs; scene-bound | **works** — option/TPS metadata applied by New Game generation; tests cover generation | P1; settings/TPS |
| N5 | Challenge selection + champion | `ScenarioSelectorPanel.challengesHolder`, `selectedChampion`, `startChallengeButton`, `ChallengeParametersHandle` | Picks challenge/champion template, starts challenge run. | scene-bound; challenge panel refs | **works** — challenge selector + champion template picker tested | P2; challenge core |
| N6 | Start scenario / challenge | `ScenarioSelectorPanel` start button → scene load | Starts selected scenario/challenge. | scene button | **works** — Create Scenario & Start generates/starts selected scenario | P1 |
| N7 | Bibite template choice for new game | `BibiteTemplateSelectorPanel` (OpenWithCallBacks) | Selects starting template for scenario/challenge; callbacks. | refs `templatesHolder`, `savedHolder`, `submitButton` | **missing** — template picker callbacks not wired | P2 |

---

## 3. Load / Save panels

| ID | Surface / interaction | Game script · class | What it does · data edited | Node / ref | Viewer status | Priority / deps |
|---|---|---|---|---|---|---|
| L1 | Open Load panel / list | `LoadGamePanel.OpenPanel/FillList` | Opens panel, lists saves in `SaveController`/save path, autosave toggle, item cards. | refs `savesHolder`, `saveItemPrefab`, `includeAutoSaves`; viewer `MenuView::LoadGame` | **partial** — viewer lists only `.zip` files, no autosave toggle/cards | P0 |
| L2 | Select save / preview | `LoadGamePanel.SelectSaveItem` | Fills name/date/version, preview image, world counts; sets selected item. | refs `saveInfoContainer`, `noPreviewDisclaimer`, `compatibleVersionIndicator` | **partial** — no preview/info/selection detail beyond row highlight | P1 |
| L3 | Load selected save | `LoadGamePanel.LoadSelectedSave` | Starts load of selected path; restrictions. | `loadButton` | **works** (row activation) | P0 |
| L4 | Delete save | `LoadGamePanel.AskDeleteSelectedSave`/`DeleteSelectedSave` | Confirmation then file delete; refreshes list. | scene buttons/dialog | **missing** | P1; file ops/confirm UI |
| L5 | Version compatibility flags | `LoadGamePanel` unknown/incompatible/compatible indicators + `VersionTracker` | Flags save version vs current; blocks/permits load. | serialized GameObjects | **missing** | P1; version metadata |
| L6 | Autosave filter | `LoadGamePanel.includeAutoSaves` toggle + `includeAutoSavesInLoadPanel` | Includes/excludes autosaves. | toggle ref | **missing** | P1 |
| S1 | Open Save panel | `SaveGamePanel.OpenPanel/FillList` | Opens save dialog, lists saves, name field, overwrite indicator, templates toggle. | refs `saveName`, `saveItemPrefab`, `savesHolder`, `existIndicator`, `includeTemplatesToggle` | **works (dynamic rows/actions + destructive dialogs)** — save rows/actions + overwrite/delete confirmations tested | P0; save writer |
| S2 | Save-as / Save | `SaveGamePanel.Save`/`OnSaveNameChanged` | Writes save at name/path; closes panel; overwrite prompt. | `saveButton`; scene-bound | **works (the prefab's panel, `save_panel.rs`)** — overwrite / Delete questions, live name validation, picture and counts tested | P0; IO save API, UI name field/confirm |
| S3 | Overwrite confirm | `SaveGamePanel.AskOverwritePreviousSave` | Confirmation popup before overwrite. | dialog prefab/scene | **missing** | P1 |
| S4 | Delete save from Save panel | `SaveGamePanel.AskDeleteSelectedSave`/`DeleteSelectedSave` | Deletes selected save file. | scene buttons/dialog | **missing** | P1 |
| S5 | Save templates toggle | `SaveGamePanel.includeTemplatesToggle` | Include template files in list. | toggle | **missing** | P2 |
| S6 | Autosave settings | `UserSettings.AutoSave`, `AutoSavePeriod`, `AutoSaveRealTime`, `AutoSaveMaxFiles`, `SavePath` | Autosave cadence/path/retention. | UserSettings panel handles | **missing** | P2; engine autosave |

---

## 4. Settings categories and widgets

Game settings are split into **scenario/simulation settings** (`ScenarioSettings`, edited in-game and
saved to `.bb8settings`/scenario) and **user/application settings** (`UserSettings`, prefs). The UI is
generated by `MenuDynamicSettingsManager`/`DynamicSettingsPanel` + setting handles; not all tabs are
hard-coded scenes.

| ID | Surface / interaction | Game script · class | What it does · data edited | Node / ref | Viewer status | Priority / deps |
|---|---|---|---|---|---|---|
| SET1 | Dynamic settings panel tabs | `UIScripts.UIPanels/DynamicSettingsPanel.cs`, `DynamicSettingsTabs.cs` | Tabs: **Simulation, Zones, Bibites, Energy**; open/close, scenario saver, zones preview, settings changers. | captured `main_world_dynamic_parameters`/`main_world_settings_panel`; ref `DynamicSettingsPanel`; tab enum | **works (dropdown selection subset)** — captured dropdown choices apply scene options; remaining value widgets partial | P1 |
| SET2 | Simulation settings list | `UIScripts.SettingHandles/MenuDynamicSettingsManager.cs`, `SettingsListHandle` + `ScenarioSettings` | World sim params (mutation, collisions, mutation, red death, shading, seeding, TPS, etc.) applied live/saved. | generated handles; scene-bound holder | **works (in-world binding)** — captured Dynamics/Settings controls map to Scenario/IndependentSettings + apply_settings; structural fields staged | P1; settings get/set API |
| SET3 | Zones tab / zone settings | `ZonesEditorPanel` + `ZoneSettingsHandle`, `ZoneGroupHandle`, `TargetZoneDropdown` | Add/delete zones/groups, radius/pos/color/fertility, labels, previews. | refs `zoneGroupPrefab`, `zoneItemPrefab`, `zonePreviewHolder`; rightmost panel | **partly works (2026-11-04)** — the Zones list, add / delete / delete-all with the game's confirmations, live zone editing through `apply_zone_settings` and the Z key are in (`docs/PARITY_REMAINING.md`, "In-world zone editor"); the zone groups, the preview panel, the name text field and the reorder buttons are not | P2; zone API |
| SET4 | Bibites tab | `DynamicSettingsPanel` + `BibiteSettingsHandle`/`BibiteSettings` | Default bibite/template genes, growth, tag/species, settings. | generated; scene holder | **works (shared in-world binding)** — Bibites-tab controls use the same captured label→setting path; tab-specific polish partial | P2; bibite settings |
| SET5 | Energy tab | `DynamicSettingsPanel` + `MatterMaterialSettingsHandle`/`MatterMaterialSettings` | Material energies, pellet energy/size mix, conversion. | generated; scene holder | **works (shared in-world binding)** — Energy/material controls use the same captured label→setting path; tab-specific polish partial | P2; material settings |
| SET6 | Settings changers (animators) | `SettingsChangersEditor`, `SettingsChangerCreator`, `SettingsChangerHandle` | Create/delete animated setting changes (target, rate, duration) used by scenarios. | refs `holder`, `settingsChangerHandlePrefab`, `creatorPanel` | **missing** | P3; scenario save |
| SET7 | User settings visual/gameplay | `UserSettings` + `UserSettingsManager`, `SettingToggle/Slider/Dropdown/ColorPicker` | Grid colors/steps, fullscreen, UI scale, FOV/targets, tooltips, selection radius, autosave, camera controls. | user settings panel; generated handles | **works (Settings pages + hotkey rebinding)** — prefs values and capture/assign/reset/persist hotkeys tested | P1 |
| SET8 | Color picker / colour settings | `SettingsColorPicker`, `ColorUserSetting`/`ColorSetting` | Edit RGB/alpha and apply. | generated handle | **next/in-progress** — colour pickers still not bound | P2 |
| SET9 | Scenario saver | `ScenarioSaver`, `DynamicSettingsPanel.OpenScenarioSaver` | Saves current scenario/settings with screenshot and name. | scene ref; settings panel | **works (core)** — scenario editor working copy/apply/save/zones tested; full editor UI polish partial | P2; scenario writer |
| SET10 | Animator presets | `UIScripts/AnimatorPresets.cs` | Preset setting animations for scenario/challenge. | scene list | **missing** | P3 |

---

## 5. In-game HUD, playback, camera, selection

Core loop code: `ManagementScripts/GUIManager.cs` (panels, shortcuts, target context),
`ManagementScripts/UserControl.cs` (selection, mouse, auto-pan, multi-select), `TimeKeeper`/`UserControl`
for pause/speed, `CameraManager` for camera.

| ID | Surface / interaction | Game script · class | What it does · data edited | Node / ref | Viewer status | Priority / deps |
|---|---|---|---|---|---|---|
| H1 | Time/playback HUD | `GUIManager.timeText`/`UpdateTime`, `TimeKeeper` | Displays time; pause/play, step, speed controls. | viewer `default_world` runtime session + mapped `TogglePause`/`SetSpeed`/`Step` | **works** — keyboard space, 1–8, right-arrow step; mapped HUD buttons work; unmapped drawn buttons inert | P0 |
| H2 | Pause/play/step/speed buttons | `TimeKeeper`, `UIScripts/UpdtSpeedSlider`, `UpdtSliderValue` | Toggle pause, single tick, set simulation speed; slider updates label. | runtime HUD nodes; viewer `UiAction` set | **partial** — pause/speed/step paths exist; runtime speed slider not fully interactive | P0 |
| H3 | HUD overlay toggles | `UserSettings.ShowPheromones`, `ShowGrid`, status bars, etc.; scene toggles | Show/hide grid, zones, pellets, pheromones, HUD/status bars, FOV. | viewer keys G/Z/P/O/H + HUD action mapping | **partial** — keys work; captured runtime toggles now get generic hits, but `SetValue` does not change settings | P0 |
| H4 | Camera pan/zoom/fit/reset | `CameraManager`, `GUIManager` shortcuts | Pan/zoom, fit world/bibites, reset, auto-pan/follow. | viewer mouse/wheel/F/B/R | **partial** — pan/zoom/fit/reset work; follow/auto-pan and camera hotkeys missing | P1 |
| H5 | Click select nearest bibite | `UserControl.SelectClosestBibiteAroundCursor`/`SelectTarget` | Ray/radius selection; selection panel opens; reselect on corpse. | viewer click path | **works** (nearest bibite; no eggs/pellets selection) | P0 |
| H6 | Box/multi-select | `UserControl.SelectObjectsInRectangle`, `onMultipleItemsSelected`; `SelectionPanel` | Rubber-band select bibites/pellets/eggs/meat; context list; lay eggs, kill/delete, tag. | ref `selectionRect`; `SelectionPanel` | **missing** | P2 |
| H7 | Keyboard select shortcuts | `UserControl` keys: H/K/Z/R/G/O/E/1–4 etc.; random/oldest/high-gen/tag/species/newborn | Select random/high-gen/oldest/egg, random tag/species, follow; shortcuts. | key handling in `UserControl` | **missing** (viewer only click + tabs keys) | P2 |
| H8 | Tag copy/paste | `GUIManager` Ctrl+C/Ctrl+V → `CopyTagOfTarget`/`TagTarget`; `TagsManager` | Copy species tag to clipboard, tag target bibite/egg. | key handling; no node | **missing** | P2 |
| H9 | Lay egg / kill target | `GUIManager.LayEggIfBibite/TryKillTarget`, `BibiteStatsPanel.LayEggFromCurrentBibite`; Ctrl/Shift shortcuts | Force egg lay, kill selected bibite/egg. | viewer Stats panel draws but no action | **inert/missing** | P1 |
| H10 | Selection panel | `UIScripts/SelectionPanel.cs` | Multi-select summary, species/tag sections, lay eggs, kill/delete, tag; entity-type filtering. | ref `SelectionPanel` | **missing** | P2 |
| H11 | Escape pause/menu panel | `EscapePanel` (`UIPanel`), `UINavigationManager` escape stack | Esc opens pause/escape menu (resume/save/load/settings/quit); nested panel escape. | `escapePanel`; viewer Esc behavior | **partial/missing** — menu Esc works; in-game escape panel absent | P1 |
| H12 | Info/statistics/quick stats/graphs | `InformationPanel`, `RightmostPanelsManager` (`StatisticsPanel`, `QuickStatisticsPanel`, `graphsPanel`) | Population/counts/biomass/energy sections, tags/species lists, graphs. | captured `main_world_information`; rightmost panel refs; viewer HUD has basic counts | **partial** — basic HUD counts only; graphs/rightmost panels missing | P1 |
| H13 | Status bars | `UIScripts/StatusBarsReference`, `UserControl` | Selected bibite health/energy/age bars. | scene bars | **partial** — Stats tab values; status-bar overlay not implemented | P2 |
| H14 | Field-of-view/target display | `SimulationScripts.Rendering/FieldOfViewDisplay`, `UserSettings.ShowBibiteFOW/ShowBibiteTargets` | Draw selected bibite FOV cone/target weights. | scene display | **missing** | P2 |
| H15 | Tooltips | `ManagementScripts/TooltipSystem`, `TooltipTrigger`, `FollowLikeTooltip`, `PelletTooltipTrigger` | Hover title/content tooltips for UI/pellets; graph tooltips. | tooltip prefab/scene; viewer has none | **missing** | P2 |
| H16 | HUD hit testing / runtime widgets | `GUIManager`, captured runtime UI | Every clickable button in the HUD should map to action. | viewer runtime sessions + `UiAction` mapping | **works (audited subset)** — generic dispatch for settings/lineage/library/help/close, toolbar Placer/PheroTower/KillHalf/Grid/Pheromones, inspector KILL/LAY/SAVE/EDIT/REVIVE; remaining keyword fallback/inert | P0 (user complaint) |

---

## 6. Inspector tabs and species/lineage

| ID | Surface / interaction | Game script · class | What it does · data edited | Node / ref | Viewer status | Priority / deps |
|---|---|---|---|---|---|---|
| T1 | Stats tab | `UIScripts/BibiteStatsPanel.cs`, `GUIManager.StatsPanel` | Live stats, age/energy/health, name/tag, lay egg, follow, select species, kill; KILL path opens half-population confirmation/dismiss. | viewer `default_world_inspector` session; tab `Stats` | **works (display+actions)** — KILL/LAY/SAVE/EDIT/REVIVE + **egg TAG Set/Clear/refresh**; bibite inline TAG beyond egg partial | P0 |
| T2 | Genes tab | `UIScripts/GenesPanel.cs` | 35 genes, values/sliders? (display), lineage/species link, species tag. | viewer Genes runtime session | **works (display)** — exact runtime panel; species link/edits partial/missing | P0 |
| T3 | Biology tab | `UIScripts.UIPanels/BiologyPanel.cs` | Organ/energy-flow schematic, sliders/values, energy details, digestion/move rates, sort toggle. | viewer Biology runtime session | **works (display)** — exact runtime panel; expanded value edits partial/missing | P0 |
| T4 | Brain tab | `UIScripts/BrainPanel.cs`, `ExpandedBrainPanel.cs`, `BrainPanelCommon` | Brain nodes/synapses, values toggles, expanded brain window, differences display. | viewer Brain runtime session | **works (display)** — exact runtime panel; expanded brain editor/window missing | P0 |
| T5 | Expanded brain view | `ExpandedBrainPanel`, `ExpandedBrainScroll` | Full-screen brain graph, zoom/pan, node/connection inspection. | scene panel | **missing** | P2 |
| T6 | Lineage panel | `UIScripts.UIReferences.LineagePanel/LineageTreePanel`, `LineageMeshPanel`, `LineageWindow`, `SpeciesColumn` | Ancestry graph/tree, species nodes, select/favorite/extinct filters, open lineage for species. | captured `main_world_lineage`; refs `LineageMeshPanel.lineageHolder` etc. | **partly works (2026-11-04)** — the panel is the prefab's (top bar, tab row, Info panel) with rename, notes, favourite, save, place, select-random, prune (with its warning) and parent / root navigation, see `PARITY_REMAINING.md` "Species panel"; the tree is drawn from the species logs (layout, zoom, scroll, selection, tooltips; 2026-11-05); the mesh view, the Brain / Position tabs and the settings panel are not done | P2; lineage history |
| T7 | Species tabs | `SpeciesTabManager`, `SpeciesInfoPanel`, `SpeciesGenesPanel`, `SpeciesBrainPanel`, `SpeciesDistributionPanel` | Species Info/Genes/Brain/Distribution, compare species, select parent/root, favorite, rename/notes, prune, save/place species. | tabs refs; `SpeciesInfoPanel` | **works** (Info, Genes, Brain and Position tabs and the lineage mesh view, 2026-11-05) — the Brain tab's expanded window is missing | P2; species/history |
| T8 | Ancestral lineage scroll/side panel | `AncestralLineageScroll`, `RightmostPanelsManager.AncestralLineagePanel` | Scroll/zoom ancestry side panel; select ancestor. | refs `lineageWindow`; scene | **missing** | P3 |
| T9 | Graphs panel | `GraphsPanel`, `GraphTooltipTrigger`, `GraphType` | Time-series graphs (population/energy/species etc.) with tooltips. | `graphsPanel`; scene | **partly works (2026-11-04)** — the seven data groups with real Data / Curves lists, curve toggles, the legend and the live newest point; the hover indicator (`GraphTooltipTrigger`) and the Gene Analysis page (dropdown, 20 bars with tooltips, time slider) work since 2026-11-06 | P3 |

---

## 7. World tools and scene editing

| ID | Surface / interaction | Game script · class | What it does · data edited | Node / ref | Viewer status | Priority / deps |
|---|---|---|---|---|---|---|
| W1 | Pellet placer | `SimulationScripts/PelletPlacer.cs` (UIPanel) | Panel: pellet size/count sliders, material dropdown; drag/click world to spawn pellets/piles; no selection while open. | captured `main_world_tool_placer`; `sizeRef`, `countRef`, `materialDropdown`; viewer none | **works (2026-11-04)** — the game's own `PlacePelletPanel` (material, size factor, number, close), selection off while open; see `PARITY_REMAINING.md` | P1; world pellet spawn API |
| W2 | Rad tower placement/edit | `RadTowerPanel.cs`, `RadTower.cs`, `WorldObjectsSpawner.GenerateRadTower` | Place RadTower; edit intensity/count/radius/taper/zone; delete. | `RadTowerPanel` refs; world object select | **works (property adjustments/placement)** — tower placement + property adjustments; edit/delete dialogs partial | P1; world object API |
| W3 | Phero tower placement/edit | `PheroTowerPanel.cs`, `PheroTower.cs` | Place pheromone tower; edit RGB/heading/zone; delete. | `PheroTowerPanel` | **partial** — PheroTower toolbar dispatch exists; full tower panel edits deferred | P1 |
| W4 | ColorKiller placement/edit | `UIScripts/ColorKillerPanel.cs`, `SimulationScripts/ColorKiller.cs` | Place/edit color killer: spawner attachment, hue/tolerance/radius/period, body-or-eyes, any-or-most; delete. | `ColorKillerPanel` refs/sliders/toggles | **works (property adjustments/placement)** — killer placement + tool property adjustments; ColorKiller detail edits partial | P1; world object API |
| W5 | Place selected object/tool mode | `GUIManager` target-type dispatch, `WorldObjectsSpawner`; scene tool buttons | Click world to place selected tool object; select existing tool. | scene buttons + target tags `Tower`/`ColorKiller` | **works** — tool mode/cursor and click placement | P1 |
| W6 | Kill half / remove half | `UIScripts/UserActions.cs` | Confirm dialog then kill half bibites/remove half pellets. | `UserActions` methods; Additional Tools | **missing** | P2 |
| W7 | Zones editor | `UIScripts/ZonesEditorPanel.cs`, `ZoneSettingsHandle`, `ZoneGroupHandle` | Add/delete zones/groups, edit zone radius/pos/color/fertility, zone groups, previews; select zone in world. | refs `zoneGroupHolder`, `zoneItemsHolder`, `zonePreviewHolder`; `dynamicSettingsPanel` | **works (core)** — scenario editor zone add/remove + persistence tested | P2; zone API |
| W8 | Scenario editor / saver | `ScenarioSaver`, `DynamicSettingsPanel.OpenScenarioSaver`; `ScenariosEditor` (SettingHandles) | Save scenario/settings with screenshot; edit scenario settings list. | scene panels | **missing** | P2; scenario writer |
| W9 | Settings changers editor | `SettingsChangersEditor`, `SettingsChangerCreator` | Add/delete animated setting changes for scenarios/challenges. | scene refs | **missing** | P3 |
| W10 | Zone preview / labels toggle | `ZonesEditorPanel.SetShowLabels`, `SimZonePreviewer`, `ZonePreview` | Show/hide zone labels/preview images. | scene toggles | **missing** | P3 |

---

## 8. Bibite editor, brain editor, template/library

| ID | Surface / interaction | Game script · class | What it does · data edited | Node / ref | Viewer status | Priority / deps |
|---|---|---|---|---|---|---|
| E1 | Bibite editor (genes tab) | `BibiteEditorPanel.cs`, `genesHolder`, `BibiteEditorPreview`, `BibiteGrowthGraph` | Edit 35 genes, name/species desc, preview/growth graph; validate name; save/continue/save-and-submit. | refs `genesHolder`, `genesTab`, `brainTab`, `nameField`, `descField` | **works (basic+anchors/template save)** — gene editing/paging/mutate + anchors/template save tested; exact editor skin is visual limit | P2; genome write/templates |
| E2 | Bibite brain editor | `BibiteBrainEditor.cs`, `NodeEditor`, `ConnectionEditor`, `AddNodePanel`, `NodeEditPanel`, `BrainEditorScroll` | Node/connection editing, add/remove node, move/pan/zoom, auto-sort, place manually, show values, play/stop brain preview, save positions; validates brain. | refs `nodePrefab`, `connectionPrefab`, `nodeHolder`, `manualPlaceButton`, `autoSortButton`, `playButton` | **works (basic)** — node/synapse add/remove/validate; node drag/anchors, exact skin deferred | P2; brain editor/model write |
| E3 | Brain node/connection edit subpanels | `AddNodePanel`, `NodeEditPanel`, `ConnectionEditPanel`, `AddNodeItem`, `ConnectionEditor` | Choose neuron type/activation, edit bias/values, add synapse source/target/weight. | prefab panels; scene-bound | **missing** | P2 |
| E4 | Template selector | `BibiteTemplateSelectorPanel` | Browse/select saved/templates/workshop bibites; callbacks; open editor/new/save; workshop actions. | `templatesHolder`, `savedHolder`, `submitButton` | **works (selector flow)** — real Library selector Create New/templates/Accept/Edit; editor screens deferred | P2; templates/IO |
| E5 | Bibite editor sprite preview/name/desc | `BibiteEditorPreview`, `ProceduralGenePreviewer`, `BibitePanelChoice` | Live procedural preview, gene sliders; name/desc inputs; warnings. | scene refs | **missing** | P3 |
| E6 | Default bibites/library list | `DefaultBibitesPanel` | List/add/delete default bibites; templates; add button. | `bibiteItemsHolder`, `bibiteItemPrefab` | **missing** | P2 |
| E7 | Bibite stats panel (editor/read-only) | `BibiteStatsPanel` | Display/edit? stats/name/tag, lay egg, kill; used as inspector. | viewer Stats runtime session | **partial** — read-only display; actions missing | P1 |
| E8 | Brain compare/differences | `BrainPanelCommon`, `SpeciesBrainPanel` `showBrainDifferences` | Compare two brains/species, highlight differences. | scene toggle/panels | **missing** | P3 |

---

## 9. Challenges, achievements, library/workshop

| ID | Surface / interaction | Game script · class | What it does · data edited | Node / ref | Viewer status | Priority / deps |
|---|---|---|---|---|---|---|
| C1 | Challenge tracker | `ChallengeTrackerPanel.cs` | Shows challenge objectives/stars/progress; retry/replay. | refs from challenge manager; scene panel | **works** — challenge tracker/end popup verified at 0.2.2 gate | P2; challenge state |
| C2 | Challenge parameters/star descriptions | `ChallengeParametersHandle.cs`, `ChallengeParameters`, `ChallengeScoreInfo` | Edits/displays star thresholds/descriptions, scoring metric. | scene refs | **works (supported subset)** — timer/star parameter editor applies at challenge start; exact star descriptions/metadata remain a follow-up | P3 |
| C3 | Challenge end popup | `UIScripts.UIReferences/ChallengeEndPopup.cs` | Result popup, continue/retry/next; stars. | scene popup | **works** — challenge end popup verified at 0.2.2 gate | P2 |
| C4 | Achievements panel | `AchievementPanel.cs`, `SteamIntegrations/AchievementManager` | Achievement list/icons/progress, hover details. | `achievementItemPrefab`, `holder` | **works (local progress)** — local achievement rows/progress from world state; Steam achievements out of scope | P3 |
| C5 | Workshop items | `SteamIntegrations/WorkshopItemsPanel`, `EditSharedItemPanel`, `SteamWorkshopManager` | Browse/subscribe/share workshop saves/scenarios/bibites. | scene panels | **works (local edit/online status)** — local rename/edit status + explicit Steam/online unavailable; full Steam Workshop out of scope; metadata follow-ups remain | P3 |
| C6 | Credits/version info | `VersionDisplay`, credits scene | Version text, credits, links. | viewer title runtime session displays version/credits text | **partial** (text) | P3 |
| C7 | Popup/dialog system | `PopupManager`, `ActionWarnings`, `EscapePanel` | Confirmation/choice dialogs used by delete/overwrite/kill-half; escape stack. | prefab/scene; no general viewer popup | **works (KILL + destructive save dialogs, action warnings)** — KILL and save overwrite/delete confirmations tested; the delete-save and scenario-launch confirmations are the game's action warnings (do-not-show-again checkbox, persisted flags, Reset Warnings); generic/tooltip popup polish partial | P1 (shared) |

---

## 10. Shared widgets, controls and input

| ID | Surface / interaction | Game script · class | What it does · data edited | Node / ref | Viewer status | Priority / deps |
|---|---|---|---|---|---|---|
| U1 | Dropdowns | `MyDropdown`, `RealDropdown`, `DropdownItem`, `TogglesDropdown`, `EnumDropdown`, `FloatSettingDropdown`, `IntSettingDropdown`, `TargetZoneDropdown`, `MatterMaterialDropdown` | Open list, choose option, update setting/callback; multi-select toggles; zone/material choices. | `dropdown.rs` (2026-11-04): one list for every dropdown the port has (zone / default-bibite / placer / tower / Settings / New Game, the captured controls without a target, the Graphs panel's `RealDropdown` and multi-select `TogglesDropdown`); see `docs/PARITY_REMAINING.md`, "Dropdown lists" | **works** — lists, picks, scrolling, flipping, close on a click elsewhere / Escape / pointer leaving (custom dropdowns), multi-select; the dropdowns of panels that do not exist yet are missing | P1 (foundation) |
| U2 | Sliders | `SettingSlider`, `FloatSettingSlider`, `IntSettingSlider`, `LogFloatSettingSlider`, `LogIntSettingSlider`, `SliderDragHandler`, `UpdtSliderValue`, `UpdtSpeedSlider` | Drag value, edit text, default pointer, live update of setting. | runtime speed slider drawn; viewer only speed keys | **partial/inert** | P1 |
| U3 | Toggles | `SettingToggle`, `Toggle`, `ObjectToggle`, `TogglesDropdown`, `SubSettingsToggleMaster` | Toggle bool setting/object visibility; master/sub-setting dependencies. | viewer runtime toggles mostly inert | **inert/missing** | P1 |
| U4 | Colour picker | `SettingsColorPicker` | Pick RGB/alpha and apply. | scene panel | **missing** | P2 |
| U5 | Tooltips | `TooltipTrigger`, `TooltipSystem`, `FollowLikeTooltip`, `PelletTooltipTrigger`, `GraphTooltipTrigger`, `TooltipHandle` | Hover title/content; graph/pellet tooltips. | viewer none | **missing** | P2 |
| U6 | Escape/back navigation | `UIPanel.Escape`, `UINavigationManager` escape stack, `EscapePanel` | Esc closes top panel/back; nested popup handling. | viewer menu Esc/back; in-game modal stack missing | **partial** | P1 |
| U7 | Pointer/hover events | `ChildItemHoverEvent`, `OnHoverShow`, `OnHoverKeep`, `ClickToEditHover`, `FollowLikeTooltip` | Hover reveal/keep, click-to-edit name/values. | toolbar hover (`OnHoverShow`), tooltips, the zone name field and the dynamic panel's value fields (`value_edit.rs`, 2026-11-04) work; the edit cursor, the other panels' value fields are not done | **partial** | P2 |
| U8 | List/reorder widgets | `ReorderableItem`, `GridLayoutFlexibleSetter`, `RectTransformChangeHandler`, `RightmostPanelChoice`, `BibitePanelChoice` | Reorder lists, responsive layout, choose rightmost/bibite panel. | scene | **missing/n/a** | P3 |

### Input/controls currently in the viewer

| Input | Viewer behavior | Game equivalent |
|---|---|---|
| Left click | select nearest bibite | `UserControl` closest-bibite around cursor |
| Drag | pan camera | camera manager |
| Wheel | zoom | camera manager |
| `F`/`B`/`R` | fit world / fit bibites / reset | camera shortcuts |
| Space | pause/resume | `UserControl`/TimeKeeper |
| `1`–`8` | speed | TimeKeeper/speed slider |
| Right arrow | step when paused | step control |
| `G`,`Z`,`P`,`O`,`H` | toggle grid/zones/pellets/pheromones/HUD | UserSettings toggles |
| click tab | switch Stats/Genes/Biology/Brain | `TabsManager` |
| `Esc` | menu back/exit only | escape panel/stack (in-game missing) |

Missing game shortcuts include random/oldest/high-gen/random egg, random tag/species, follow, Ctrl+C/V
tag copy/paste, Ctrl/Shift lay-egg and kill, arrow-key camera, and tool placement.

### Runtime control-node coverage (why buttons appeared dead, updated for `fb6f221`)

The node-level audit is [`UI_CONTROL_AUDIT.md`](UI_CONTROL_AUDIT.md). The original audit covers **145 active
controls / 9 sessions**; parity's `4159253` adds **24 functional sessions and 165 labelled paths**. The
framework has since moved well past the audit's six-action description:

- `UiAction` now includes `TogglePause`, `Step`, `SetSpeed`, `SetSpeedSlider`, `ToggleOverlay`, `SetTab`,
  `CloseInspector`, **`OpenPanel`/`ClosePanel`/`TogglePanel`**, **`SetValue`**, **`Invoke(ControlCommand)`**,
  **`MenuRow`**, **`ActivateControl(id)`**, `Quit`, `ToggleHelp`. `PanelKind` covers HUD/tabs/settings/
  library/achievements/tools/credits/load/save/scenario/lineage/species/help/escape.
- The title menu has a functional view state machine (`main_menu_*` rows), not only a hard-coded five-row map:
  Continue/New/Load/Bibite Library/Settings/Achievements/Additional Tools/Credits/Quit all navigate; settings
  persistence/apply and reset are implemented.
- `003a69d`/`fb6f221` introduced stable control ids (`tools/ui/viewer_control_ids.json`) and generic hits:
  **every active runtime control** now emits `ActivateControl(id)` (asserted by
  `ui_runtime::tests::runtime_control_registry_and_generic_hits_cover_every_active_control`), and captured
  panel sessions render for Settings/Library/Achievements/Tools/Credits/Lineage and in-world tool/settings
  panels.
- Dispatch is still only as good as the bound action: `main::dispatch_runtime_control` substring-maps common
  keys to panels/commands; panel opening works, but **`SaveWorld`/`QuickSave`/`OpenSavePanel`/`QuickLoad`/
  `SpawnBibite`/`KillSelected`/`SelectTool`/`SetValue` remain placeholders** in `App::invoke_command`. So a
  click can now be hit-tested and open a panel, but the actual edit/save/placement mutation may still no-op.

**Current dead-button buckets:**

| Class | Examples | Status |
|---|---|---|
| Unbound command | Save/Save As, QuickSave/Load, spawn/kill, tool selection, setting value edits | hit-test works; action no-op |
| Rendered-only panel | Settings/information/dynamic parameters/tool panels/lineage | captured layout renders; child edits/actions mostly unbound |
| Missing surface | Save panel UI, editors, challenges, scenario/zone editors, graphics/gameplay/hotkeys settings | no implementation |
| Title-menu informational rows | Library/Achievements/Additional Tools/Credits | open and list items; entries are informational, not game data/actions |

The curated fallback HUD (`--no-runtime-ui`) has 14 wired controls (pause/step/speed±/five overlay toggles/four
tabs/close); the exact runtime path now covers those plus generic panel hits.

## 11. Dependencies on core APIs (what each cluster needs)

| Cluster | Core/IO API dependency | Current status |
|---|---|---|
| Main menu/load/continue | Save index + metadata (`SaveController`, version tracking), save dir scan | scan/list exists in viewer; no metadata/version/autosave |
| Save panel/autosave | `bibites_io` world save writer/settings/template writer, save path/name, overwrite/delete, autosave loop | CLI save exists; viewer UI/autosave missing |
| Scenario selector/new game | scenario/save discovery, `ScenarioSettings`/`ScenarioIndependentSettings`, scenario name/description/preview, TPS settings | loaders exist; selector/settings-write missing |
| Settings panel | `SimSettings`/`UserSettings` get/set, settings metadata (name/min/max/precision/default/choices/helper), apply live and save | core settings exist; generic setting widgets/apply path missing |
| Tools (pellet/towers/ColorKiller) | spawn/remove world objects, zone-target dropdown data, material list, selection/target dispatch, serialization | world sim/IO types exist; viewer spawn/edit UI missing |
| Zones/scenario editor | `ZoneSettings`/`ZoneGroupSettings`, zone geometry/material, scenario serializer/screenshots | zone data exists; editor missing |
| Bibite/brain editor | genome/template writer, brain node/synapse model + validation, template store, preview renderer | genome/brain loaders exist; write/editor missing |
| Lineage/Species panels | lineage history (`DataLogger`/lineage), species registry/relations, charts | lineage data partially modelled; viewer panels missing |
| Challenges/achievements | challenge manager/scoring, Steam/achievement state | challenge core partial; UI missing |
| Shared widgets/popups | generic UI action dispatch, dropdown/slider/toggle/dialog primitives, escape stack | minimal `UiAction`; generic widgets missing |
| Runtime HUD buttons | map captured runtime node → action; unknown nodes should be marked disabled, not silently inert | only small `UiAction` subset mapped; rest inert |

---

## 12. Ranked remaining gaps after 0.2.0

The authoritative ranked gap list is [`UI_REMAINING_GAPS.md`](UI_REMAINING_GAPS.md). It splits
**implementable** work from genuinely **out-of-scope** Steam/audio features:

- **P0:** settings pages (Graphics/Gameplay/Hotkeys + in-game values), save overwrite/delete dialogs/browser
  detail, editor polish (scroll >14 genes, node anchors/drag, mutate settings, template save/continue).
- **P1:** captured secondary panels still inert (Achievements local progress, Additional Tools utilities,
  Credits actions, Workshop local files), local challenge flows, exact lineage rung/distribution + Species.
- **P2:** scenario-editor polish, popup/input/tooltip system and TAG beyond egg, template/challenge selectors.
- **P3:** external links/version actions, visual/editor skin fidelity, local workshop file browser.
- **Out of scope:** Steam achievements/Workshop/leaderboards/overlay, audio/UI sounds, online services;
  their local analogues remain implementable and are listed separately.

Keep this matrix’s per-row statuses in sync with that document as each tranche lands.

## 12.1 How to verify the newly working subset (headless)

Run from the repo root with extracted `assets/game/` present. Use the released 0.2.0 package or a clean
current HEAD; these commands cover the working subset, not the gaps in section 12.

```powershell
# 1. Build the viewer
cargo build --release -p bibites-view

# 2. Interactive title menu: every row now navigates
#    Continue / New / Load / Bibite Library / Settings / Achievements / Additional Tools / Credits / Quit
target\release\rustibites-view.exe --menu --saves-dir testdata\real\saves

# 3. Deterministic main-menu screenshot (needs assets/game)
target\release\rustibites-view.exe --menu --screenshot target\ui-main-menu.png --width 1920 --height 1080

# 4. Settings apply/persist: Settings -> toggle Grid/Zones/Pellets/Pheromones/HUD/TPS
#    -> Apply & Back; then inspect:
Get-Content target\viewer-menu-settings.txt
#    (settings are re-loaded on the next --menu run; Reset Defaults restores)

# 5. Load panel: choose Load, select a listed .zip; New: choose New, select a scenario
#    (the list/selection/loading transition are functional; save-write is still missing)

# 6. Headless HUD + selected-bibite panel; exercise one real hit-tested click
target\release\rustibites-view.exe testdata\real\Default.zip --screenshot target\ui-world.png `
  --select-first --tab stats --width 1920 --height 1080
target\release\rustibites-view.exe testdata\real\Default.zip --screenshot target\ui-click.png `
  --click 960,540 --width 1920 --height 1080
```

Headless regression tests for the statuses above:

```powershell
cargo test -p bibites-view menu::tests::menu_views_navigate_and_back
cargo test -p bibites-view menu::tests::settings_persist_and_reload
cargo test -p bibites-view menu::tests::panel_state_stack_modal_and_escape
cargo test -p bibites-view ui::tests::ui_hit_testing_and_actions
cargo test -p bibites-view ui_runtime::tests::runtime_control_registry_and_generic_hits_cover_every_active_control
# full suite (run only on a compiling tree):
cargo test -p bibites-view
```

Expected results for this head: the works listed in §1.12/§1.10/§1.9 (plus **Settings pages with prefs**,
menus, Library, New Game generation, save browser rows/actions, lineage graph, KILL dialog, scenario editor
core, egg TAG, basic gene/brain editors, tool panels). In-world Dynamics/Settings values, save dialogs, editor
polish, secondary panels/challenges and exact lineage/Species data remain next/in-progress per
[`UI_REMAINING_GAPS.md`](UI_REMAINING_GAPS.md). Screenshot
mode renders the curated HUD and menu; functional-panel screenshots require the interactive window and
extracted assets.

### Verification commands for the current working buttons

0.2.3 closure tree gate is **viewer 96/96, core 457 at `8c30538`**; 0.2.2 package was 94/94/457. Commands below
are the per-button checks; only mark a row works when the relevant effect is green:

| Button/action | Verification command | Expected game-observable effect |
|---|---|---|
| All 9 title rows navigate | `cargo test -p bibites-view menu_views_navigate_and_back` | Menu view changes; rows 0–8 map to views/Quit |
| New Game list | `cargo test -p bibites-view new_view_lists_scenarios` | Scenario list populated; activation starts loading |
| Load/Continue | `cargo test -p bibites-view load_view_back_returns_to_main` + `default_scenario_activation_starts_loading` | Save/scenario selected and loaded; Back returns |
| Settings apply/persist | `cargo test -p bibites-view settings_persist_and_reload` | Settings file persists; reload returns same values |
| Panel stack/Escape | `cargo test -p bibites-view panel_state_stack_modal_and_escape` | Open/close stack and escape behave |
| Tool palette placement | `cargo test -p bibites-view tool_placement_spawns_pellets_and_kills_with_killer` | Pellet spawn/killer kill changes world |
| KILL / Apply/Reset settings | `cargo test -p bibites-view kill_and_apply_settings_commands_are_wired` | KILL removes selection; settings commands change state/persist |
| Save/quick save+load | `cargo test -p bibites-view quick_save_and_load_round_trip_through_ui_commands` | Save file written; load restores world |
| Spawn bibite | `cargo test -p bibites-view spawn_bibite_command_adds_a_bibite` | Population increases |
| Runtime registry/panels | `cargo test -p bibites-view runtime_control_registry_and_generic_hits_cover_every_active_control` + `functional_sessions_parse_have_rects_labels_and_known_bindings` | Every active control has a hit; captured functional sessions parse/bind |
| KILL confirmation dialog | `cargo test -p bibites-view kill_half_dialog_confirms_and_cancels` | Yes kills half; Cancel leaves population unchanged |
| Settings value proposals | `cargo test -p bibites-view settings_value_metadata_and_pointer_proposals` | Captured toggles/sliders/dropdowns emit typed values; apply path currently limited |
| Tool property widgets | `cargo test -p bibites-view tool_settings_drive_pellet_pile_and_killer_tower` | Material/count/energy/radius/hue change placement output |
| Egg save + tag | `cargo test -p bibites-view tag_and_egg_save_commands_work` | Egg entity written; tag applied to selection |
| Save browser IO | `cargo test -p bibites-io save_browser_save_load_quick_slot_and_fingerprint` + `save_list_entries_metadata_thumbnails_and_autosave` | Rows/thumbnails/autosave/rename/delete/quick-slot round-trip |
| Library selector | `cargo test -p bibites-view library_selector_rows_and_actions` | Create New/templates/Accept/Edit rows select/start correctly |
| New Game option registration | `cargo test -p bibites-view library_and_newgame_controls_are_hit_testable_with_dropdown_values` | New-game dropdowns carry ≥5 options, option rects, and choices |
| Dynamic save row ids | `cargo test -p bibites-view dynamic_row_ids_are_stable_and_distinct` | Save rows have stable distinct hit ids |
| Lineage graph | `cargo test -p bibites-view lineage_panel_refreshes_and_selects_nodes` | Panel opens, tree refreshes, node selection/inspection works |
| Zone add/remove APIs | `cargo test -p bibites-core ui_zone_add_remove_and_persistence` | Add/remove persists through scenario state |
| Genome/brain editor APIs | `cargo test -p bibites-core randomize_is_seeded_and_respects_ranges` + `editor_add_edit_and_remove_graph_parts` | Seeded randomize + add/edit/remove graph ops |
| Menu screenshot | `target\\release\\rustibites-view.exe --menu --screenshot target\\menu-main.png` | Deterministic 1920x1080 menu frame |

**0.2.0 status:** KILL dialog, dynamic save rows, dropdown selection, tool panels, lineage graph, Library
selector, New Game generation, scenario editor core, egg TAG and basic gene/brain editors are **works**.
Remaining defects/polish are ranked in [`UI_REMAINING_GAPS.md`](UI_REMAINING_GAPS.md); Steam/audio are out of
scope. Per the works rule, “works” means the same game-observable panel/dialog/state effect, not merely a
hit or a passing API test.

## 13. Verification plan

- For each row marked **works/partial**, add a viewer test that synthesizes the click/key through the real
  hit-testing path (`--click` / headless UI frame) and asserts the observable world/settings/selection change.
- For each **missing**, land the panel + action and a headless screenshot/A-B against the runtime session
  if captured (title/menu already captured; inspector tabs captured; other panels need capture or a
  curated layout with an explicit “not game-exact” note).
- Update this matrix in the same commit as each UI work package; keep `VIEWER.md` for visual fidelity and
  this document for interaction/function.

**Adversarial warning:** this inventory is exhaustive at the level of scripts/classes and known surfaces.
Unity scene wiring (`Button.onClick` targets) is not in `Here/`, but `functional_ui/label_path_manifest.json`
now supplies exact paths for 24 captured sessions / 165 labels; manifests include **inactive controls**, so
“label/path exists” is not the same as “reachable in the current panel.” Panels outside the 24 captures still
need scene/runtime recovery before claiming exact layout. Until then, a control may be present in code but
unreachable, or present in the scene but not referenced in a script — the matrix marks those as
scene-bound/unknown rather than asserting a path.

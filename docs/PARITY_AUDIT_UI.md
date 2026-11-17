# UI source-level parity audit (C# `Here/UIScripts*` vs `crates/bibites-view`) - 2026-11-03

Derived from the decompiled C# and the Rust viewer code only (docs claiming "functional closure" are NOT reliable:
this audit found the inspector/editor panels are curated placeholders, not ports). Items are agent claims; verify
against the C# before implementing.

## A. In-world inspector and editors

Root cause: native Unity UI trees are replayed from captures with static text; `ui_runtime.rs::live_text`
(~2135-2162) only binds `/timeText` and the speed slider. The only live per-bibite data is the hand-written
`hud::draw_selection_panel` (hud.rs ~1670-1935).

**P0 (function missing)**
1. Stats panel: 13 live values missing (generation, length, size, mass, speed, time alive, travelled, eggs laid,
   alive children, bibites bitten, damage dealt, times attacked, damage received); `BibiteStatsPanel.cs:304-316`,
   formatted via `ScientificFormat` (SI prefixes + units).
2. Stats buttons: revive, lay egg, save, edit (opens editor), parent1/2 select, random child, kill/explode.
   Rust `EditSelected` only saves a template.
3. Biology panel: ~90 live fields (`BiologyPanel.cs:637-1078`) vs 6 static rows.
4. Kill semantics: C# `KillCurrentBibite` (:211) kills if alive, explodes if dead, aborts the selected egg.
   Rust `AbortEggs` aborts ALL eggs; no corpse-explode path.
5. Egg inspector: generation, size, mass, total energy, hatch-progress and free-energy sliders missing. (Done 2026-11-06, see `PARITY_REMAINING.md`.)
6. Species tag edit: inline text input, TagsManager add/remove, challenge/dead guards.
7. Traits (isRad/isRain/isSource/isKiller) read-out missing.
8. Gene editor: typed values, name/description with validation, Save/SaveAndContinue/SaveAndSubmit gating, spawn/apply
   path. Rust nudges +-1% and always saves `viewer-editor.bb8template`.
9. Brain editor: node drag/zoom, NodeEditPanel (label, function, bias bounds per type, input activation, add in/out
   connection), ConnectionEditPanel (weight, enabled, split, remove), Delete/Space/'1' hotkeys, multi-select,
   connect mode, AddNodePanel type picker, step/play, auto-sort, show-values. Rust adds a fixed Sigmoid node and a
   hard-wired 0->last synapse.
10. Brain inspector tab: live node graph with values/names, species-template diff, brain distance.
11. Genes inspector tab: full gene list with real names, species diff column, distance, tooltips.
12. Species info actions: rename (validated), notes, favourite, prune, save, place, random member, parent jump.
13. Multi-selection panel (lay eggs, kill, delete, tag, per-species toggles, Ctrl+V).

**P1**: number formatting (ScientificFormat), selection header labels, dead/corpse panel switching.
**P2**: generic popup layouts, Rust-invented editor buttons (Add/Remove/Anchor/Validate), custom tab strip.

## C. Main UI, toolbar, tools, camera, hotkeys (agent claims)

**P0**: camera follow of selection absent; mouse model wrong (C#: right-drag pan, ctrl+right slide, middle zoom,
left click <0.15 s selects within `orthoSize*SelectionRadius`, left-drag draws a selection rectangle, objects are
draggable; Rust pans on left-drag with a fixed pixel radius); no rectangle multi-select / Selection panel; no Escape/pause
menu (Esc quits); hotkey map differs (H, K, Z, R, G, O, E, F5, Insert, Delete, Home, PgUp/PgDn, +/-, WASD/arrows, 1-4
open inspector tabs, Ctrl+C/V, Ctrl+Z/Shift+Z undo of settings); phero tower tool and tower/colour-killer panels absent;
pellet placer differs (size factor 0.1-1000 x pelletEnergy, +-25% jitter, count 1-10, pile radius, hold-to-paint);
template bibite placer flow absent; remove-half-pellets action absent; Visibility panel lacks ruler/status bars/FOW/targets.

**P1**: speed control (C# log slider 0.1-100, pause-source stack, Play force-plays; Rust integer 1-8); Save/Load panels
do not pause; "LoadButton" mapped to QuickLoad; save-as lacks validation; button mis-wirings: LayBibiteEggButton->SpawnBibite,
KillHalf->KillSelected, DynamicParametersButton->Settings, Reset Warnings->ResetSettings (fixed: it resets the action warnings, `ResetAllWarnings`), StartButton only opens picker;
kill-half should be random 50% incl. eggs; New Game landmark tables differ (size 1000..15000, biomass 0.00075..0.1,
fertility 0..1e-4); scenario selector thin; Continue should use last save; selection gaps (skip dead on click, reselect on
corpse, zone editor on select); zoom clamp [5, max(250, 2*SimSize)]; challenge gating of controls.

**P2**: Step command lacks overlay refresh; HUD text format; F12 screenshot is a Rust invention; no popup types
(error/dialog/choice/action-warning); default pheromone/grid visibility from UserSettings; H hides the whole canvas.

Correction found while verifying: "AbortEggs aborts all eggs" is the *correct* behaviour of the toolbar user action
(`UserActions.cs:42`); only the inspector Kill button also needs to abort the selected egg.

## D. Lineage, graphs, world rendering (agent claims; shader internals unverifiable - not decompiled)

**P0**: lineage tree is a placeholder (C# `LineageTreePanel/SpeciesColumn/LineageWindow`: column packing, log-like time axis,
zoom/pan, previews, hideExtinct/showFavorites/scaleImportant, needs >=5 records, pauses game); `LineageMesh` data view absent (done 2026-11-05: see PARITY_REMAINING);
`GraphsPanel` absent (26x11 grid lines, y-max rule `10^floor(log10 m)*ceil(m/10^floor)`, multiple scale depths, curve toggles,
tooltip, parallel data-stream dropdown, genes `BarGraph` with time slider); bibite view cone (`FieldOfViewDisplay`) and target
weight labels (`WeightDisplay`) absent - the white selection ring in Rust does not exist in the game; camera follow absent;
out-of-bounds "microscope" shade absent (starts 1.5*SimSize, fades over 1000, black clear colour); per-bibite shader effects
(rainbow, virus/radio, producer, reaper, hurt flash `(1-healthRatio)^8`, decays dt/0.5) absent; arm/mouth animation
(`ProceduralSpriteController`, mouth = Want2Eat, arm springs) absent; blood/pheromone particles absent.

**P1**: grid default should be OFF, widths 1/5, alpha fade from ortho size; background default (0,0,0.19); zone colour
default FertilityColor (11,128,0) not (231,154,0) and gradient by `_PelletSpawnCoefficient`; pheromones default ON as particles;
camera controls (right-drag pan, ctrl+RMB slide, MMB zoom, keys =,-,Home,PgUp/PgDn, smoothed arrows; zoom about screen centre);
eye/corpse tint (no black lerp; saturation scaled by 1-decay); invented selected-bibite health/fat rings.

**P2**: Fat/Armor pellets skipped in render; egg fallback core/outline invented; layering unverified; tail `bend_row` 0.55 has no
C# source; left-arm offset was an empirical fudge (replaced 2026-11-14 by the pivot convention, `sprites::drawn_pivot`); body size index source.

## B. Settings UI (agent claims)
(See below; this section replaced the earlier placeholder.)

## Progress log (implemented after the audits; type-checked only for viewer code)

- `bibites_core::format` - port of `ScientificFormat` / `FloatValueFormat` (unit tested). Used by the Stats tab and Graphs.
- Stats tab now shows the 12 values of `BibiteStatsPanel.UpdateBibiteStats` (alive-children not tracked yet; units are guesses).
- `bibites_core::datalog` - port of `LogLikeArrayParallel` + `DataLogger` biomass/count streams (60 s cadence), unit tested;
  `World::data_logger` (not in checkpoints / fingerprint, so history restarts on resume).
- Graphs panel (`hud::draw_graphs`): 26x11 grid, y-axis rule, 2 streams, scale selector. Opened from the toolbar
  `InformationPanelButton` (the real Information panel - pie charts of counts/energy - is still not implemented).
  Missing vs C#: tooltip/nearest-point, curve toggles, other streams (age, brain size, eggs, birth/death), genes bar graph,
  pause-while-open, scale buttons enabled only when `DepthHasData`.
- Wiring fixes: KillHalf, LayBibiteEggButton, LoadButton; Kill explodes dead bodies; kill-half is random 50% incl. eggs;
  New Game landmark tables; zoom clamp uses world size; camera follow; click selection skips dead bibites.

## Progress log 2 (viewer changes are type-checked only)

- Escape now pops the top panel, then deselects, then opens a pause menu (Resume/Save/Load/Settings/Quit) that pauses the game and
  restores the previous pause state on close (`EscapePanel.SetRestrictions`); it no longer quits the app.
- Hotkeys follow `UserControl.Update`/`CameraManager.Update`: R random bibite, G highest generation, O oldest, E random egg, F5 quick
  save, Insert lay egg, Delete kill/explode, Home deselect + camera to origin, `=`/`-`/numpad zoom (+0.25/-0.2). The old G/Z/O/R overlay
  toggles were removed from the keyboard (grid/zones/pheromones are Visibility-panel settings). The viewer-only extras (1-8 speed
  presets, `.` step, F/B fit, P pellets) are no longer bound: the game has no such keys, and its digits 1-4 open the inspector panels.
- Graphs panel has six streams (biomass, counts, age, brain size, eggs laid, births/deaths) from `bibites_core::datalog`.
- Selected bibite shows its view-cone border (radius `ViewRadius*d1Size`, angle `ViewAngle`) instead of the invented white ring
  (the real game draws a shader cone; filled cone, target weight labels, status bars and ruler are still missing).
- Pellet placer follows `PelletPlacer.cs` (size factor, jitter, pile radius, hold-to-paint).
- Not done: speed slider 0.1-100 (native slider is not wired to any control), right/middle-drag camera, rectangle multi-select,
  Information panel (pie charts), tooltips/curve toggles on graphs.

## Progress log 3 (viewer changes are type-checked only; never run)

- Information panel (toolbar `InformationPanelButton`): counts, energy breakdown (free/bibites/eggs/plants/meats with the game's
  wedge colours) and top-5 active species as proportion bars instead of pie charts, with a button to the Graphs panel.
- Inspector tabs: Stats shows the 12 `UpdateBibiteStats` values; Genes lists all 35 genes by `GENE_DISPLAY_NAMES`; Biology shows ~23
  organ/energy rows; Brain shows network size plus all 15 outputs and 30 inputs with live values; the wheel scrolls the lists
  while the cursor is over the inspector; digits 1-4 select Stats/Genes/Biology/Brain when a bibite is selected.
- World ruler (`RulerHandle`): bar kept between 100-200 px with the game's stepping rule, label in world units; toggled from the
  Visibility `RulerToggle`.
- Right-mouse drag pans the camera and drops follow; save names containing `Path.GetInvalidFileNameChars()` are rejected.
- Still missing (largest): brain editor interactions, gene editor value entry/validation, species panel actions, multi-selection,
  lineage tree/mesh, filled view cone + target weights + status bars, arm/mouth animation, particles, microscope shade,
  speed slider 0.1-100, challenge runtime (fresh scenario world, persisted scores).

## B. Settings UI audit (agent claims; fixed items marked)

Core finding: the Rust viewer has no real `UserSettings` model. `UserPreferences` (`bibites-io/src/storage.rs`) is an invented ~22-field
struct and `App::set_pref_by_key` maps captured Unity controls to it by *substring-matching the label*; most stored prefs are never read.
- P0 prefs not applied at startup (grid default OFF / pheromones ON in the game) - **grid fixed** (`App::new`), pheromones left off
  because pheromones are drawn as flat discs, not particles; `pheromonesStrength` (default 0.1, particle density) not honoured;
  `minimumFPS` (default 15) not honoured; `ScreenResolutionFactor`/`UIScale` replaced by invented resolution presets;
  **autosave now runs** (`App::update_autosave`, wall-clock or sim-time per `AutoSaveRealTime`, `Autosaves/autosave_yyyyMMddHHmmss.zip`,
  retention `AutoSaveMaxFiles`) but the period is still a free integer instead of the 1/5/10/30/60/300/600 enum;
  **`PauseAfterLoad` now honoured** on load paths; hotkey rebinding is an invented feature that main.rs never reads (the game has a
  static Hotkeys page); ~25 user settings absent (FertilityColor, BackgroundColor, grid colours/steps/widths, ShowRuler (now an
  app flag), ShowStatusBars, ShowBibiteFOW/Targets, SelectionRadius, ReselectOnCorpses, allowArrowKeyCameraControl, ...);
  dynamic-parameter controls are applied by fuzzy label matching (risk of writing the wrong key); setting value labels are not refreshed.
- P1 **defaults fixed:** `autosave_real_time` true, `autosave_max_files` 25, `show_node_names` false, `min_fps` 15
  (`UserSettings.cs:45,50-59,327,490`). Still different: `fullscreen` (game true), `pheromone_opacity` 1.0 vs strength 0.1.
  `SettingMeta` lacks factor/precision/landmarks/units strings (e.g. biomassDensity factor 1000 " E/u2"; probabilities x100 "%"),
  log-slider mappings not modelled, dropdowns not enum-backed, `set_pref_by_key` substring routing is brittle.
- P2 new-game default TPS prefs, SpeciesSpan prefs, hidden settings, tooltips (`HelperText`) absent.

## Review of the viewer diff (independent, read-only) - fixed

Hit-id collisions between panels that can be open together (Graphs/Information/Escape/species/editor widgets now use 8000+; the brain-node
hits moved to 5000+; the editor close X and a new button had shared id 530); full-screen `Consume` blockers behind the pause menu,
Information and Graphs panels so clicks cannot reach widgets/world underneath; loading from the pause menu no longer has `PauseAfterLoad`
overridden when the menu closes; new hotkeys are ignored while rebinding, with a dialog or the pause menu open, and Insert/Delete/F5 are
disabled during challenges (`UserControl.cs:114-180`); Delete aborts a selected egg and leaves a killed bibite selected as a corpse (so a
second Delete explodes it, `TryKillTarget`); random egg selection skips aborting eggs; Reset Settings keeps the grid off; single-step ticks
feed the challenge/autosave clocks; camera follow no longer fights zoom-to-cursor; the stale `LoadButton` mapping test was updated.
Unverified because the viewer tests cannot be run here: all of the above.

## Which UI path is actually used (important when reading everything above)

`main.rs::build_ui_frame` builds the in-world UI one of two ways:
- **Native path** (`themed.runtime` is `Some`, i.e. whenever `assets/game/ui_runtime_native.json.gz` loads - the normal case): the frame is
  `RuntimeUi::build_frame`, a replay of the captured Unity UI trees (`default_world`, `default_world_inspector|genes|biology|brain`, ...),
  plus `hud::draw_selection_panel` for tabs without a session and now `hud::draw_extra_panels`.
- **Curated fallback** (`hud::build`): hand-written panels, used only when the native assets are missing.

Consequences, stated plainly:
- The native captured text is *frozen* unless `ui_runtime.rs::live_text` rewrites it. As of this pass it rewrites: time/speed (before),
  and now the Stats values (species, generation, length, size, mass, speed, time alive, distance, eggs laid, alive children, bitten,
  damage dealt, times attacked, damage suffered), the health/energy status bars, Biology health/energy values, Brain node/connection
  counts and species, Genes species and every gene `value` (rows are matched to genes by their `geneName`; the difference column shows the signed flat
  difference from the species template genome, empty when equal, as in `GeneLineInfo.SetGene`; the species-distance percentages on the Genes and Brain panels are `distance / usedSpeciesSpan` as in `GenesPanel.cs:66` / `BrainPanel.cs:38`). Formats come from the capture (`15.61 u`, `124.79 u2`, `3.15 g`, `0.0HP`, `%.3f`).
- Native ruler follows the Visibility toggle and the game's stepping rule unless a headless render pins a value.
- `draw_extra_panels` (Information, Graphs, pause menu) is drawn over both paths.
- NOT visible in the native path (they live only in `hud::build`): the curated Stats/Genes/Biology/Brain row lists, the species
  Favorite / Select-random buttons, the brain-editor Weight/Bias/Function/Link buttons, the HUD ruler. They only appear if the native
  assets are absent. Moving them into the native path would need new bindings on the captured Library/Lineage sessions.

## Later fixes (type-checked / core-tested only)

- `OpenQuickStats` (the right-hand "Info" button) now opens the Information panel instead of the Stats tab; the Stats tag label is bound.
- LAY button / Insert lay one egg regardless of progress and without spending energy (`BibiteEggLayingOrgan.LayEgg` via
  `GUIManager.LayEggIfBibite`); `BibiteStore::lay_single_egg` has a unit test.
- Space no longer unpauses the game behind the pause menu / dialogs / hotkey rebinding; opening Save/Load/Settings from the pause menu
  closes it and keeps the game paused; a gene row's value can no longer show the previous row's gene.
- Third independent review of the native text bindings: every `live_inspector_text` path was checked against the capture and all 35
  gene names match `GENE_DISPLAY_NAMES` one for one. Known gaps: Egg stats paths are not bound (`selection` is `None` for eggs), UI "Step"
  clicks are not fed to the challenge/autosave clocks.

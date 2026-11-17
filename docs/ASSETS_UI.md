# UI asset expectations and coverage

**Status:** living document (docs/tools only, task-79). **Owner:** spec-dev. **Consumers:**
parity-dev (asset extraction, task-76), view-dev (UI atlas/theme), Lead (visual-parity scope).

This document explains `tools/assets/ui_expectations.json`: the authoritative list of UI
sprites/fonts/icons/materials the viewer needs to reproduce The Bibites' UI 1:1, the naming scheme,
how to check it against parity-dev's manifest, and the current coverage gaps.

Related: [`design/UI_PARITY.md`](design/UI_PARITY.md) (UI inventory/plan), `assets/game/manifest.json`
(parity-dev extraction output), `tools/assets/README.md` (extraction pipeline).

---

## 1. Files

| File | Purpose |
|---|---|
| `tools/assets/ui_expectations.json` | **Generated catalogue**: 242 UI assets with `logical` name, usage, priority, surfaces (UI_PARITY IDs), variants, 9-slice flag/pivot, expected manifest aliases, source `Here/*.cs` references, plus `field_inventory` (237 C# graphic fields) and scan evidence. |
| `tools/assets/build_ui_expectations.py` | Regenerates the JSON: curated catalogue (from the UI inventory) + scan of the decompiled C# + manifest alias table. Run after changing the inventory or renaming assets. |
| `tools/assets/check_ui_coverage.py` | Compares the expectations with `assets/game/manifest.json`; reports extracted / renamed / missing / optional / runtime, and unclaimed extracted UI assets. |

Commands (from the repo root):

```powershell
python tools/assets/build_ui_expectations.py            # regenerate JSON
python tools/assets/check_ui_coverage.py                # coverage vs assets/game/manifest.json
python tools/assets/check_ui_coverage.py --json         # machine-readable report
python tools/assets/check_ui_coverage.py --strict       # exit 1 if required items are missing
python tools/assets/check_ui_coverage.py --manifest target/manifest-snapshot.json  # pinned snapshot
```

If `assets/game/manifest.json` does not exist (extraction not run yet), the checker prints the
expected-vs-actual plan and exits **0** — safe for CI before extraction lands.

## 2. How the list is built

1. **Curated logical catalogue** — one entry per UI asset the viewer needs, derived from the UI
   inventory in `docs/design/UI_PARITY.md` (§1–§8, §10–§11): panel frames/headers, shared widgets
   (buttons, toggles, sliders, dropdowns, inputs, tabs, scrollbars, progress bars), graph/chart
   chrome, HUD art (time/speed/pause/status bars/selection), tooltips/popups/notifications, menus,
   cursors, lineage connectors, brain icons (33 inputs + 15 outputs + 13 activation functions),
   achievement icons, fonts, materials, and procedural-preview placeholders.
2. **C# scan** — every `Image` / `SpriteRenderer` / `Sprite` / `TextMeshProUGUI` / `TMP_FontAsset`
   field under `Here/UIScripts*/**` (+ UI controllers, Steam UI, rendering overlays, background
   manager) is recorded in `field_inventory` with `file:line:field`, a heuristic logical name and
   `confidence: prefab|heuristic|runtime`. Code-level assignments (brain icons, achievement icons,
   cursors, lineage sprites, procedural previews, colour-killer materials, selection highlight) are
   attached to the matching expectation as `source` references.
3. **Manifest aliases** — the 2026-10-30 extraction surfaced the game's real asset names. Because
   the game reuses a handful of 9-slice textures for many panels, most logical chrome entries map to
   a shared alias (e.g. every `ui/panel/*_frame` → `UISprite`/`Background`/`Long_UI`/`gray_panel*`;
   `ui/button/button` → `Button_UI`; `ui/slider/slider_track` → `bars`/`tiled_bars_1`; fonts →
   `Roboto-Regular`/`Roboto-Bold`/…). Brain/achievement icons map by the game's own naming
   (`input_*`, `output_*`, `hidden_*`, `ACH_*`).

**Important:** `prefab_only: true` means the sprite is wired through a Unity prefab in the level
scenes, not through C#. The decompiled scripts reveal the field names and roles but not the sprite
asset. `field_inventory` lists all such fields; their `expected_logical` is a heuristic starting
point for parity-dev's prefab-level matching/extraction. Do not treat a heuristic name as a
requirement to rename the real asset unless it is also a curated catalogue entry.

## 3. Naming scheme

Logical names live under `assets/game/` (the manifest's root) and follow
`ui/<usage>/<logical_name>`:

```
ui/panel/bibite_stats_frame        ui/button/button                  ui/font/tmp_regular
ui/icon/brain_input/00             ui/icon/brain_function/12        ui/hud/statusbar_health_fill
ui/overlay/selection_rect          ui/cursor/cursor_edit            ui/material/fov_cone
ui/graph/graph_grid                ui/progress/progress_fill        ui/icon/achievement/ACH_*_0
```

Conventions: lowercase `snake_case`, no spaces; PNG for sprites, TTF/OTF for fonts (or the TMP SDF
atlas if conversion is lossy); 9-slice borders and pivot/PPU are metadata in
`assets/game/manifest.json`, never encoded in the name; brain icon groups nest exactly one level with
two-digit indices; in-world UI-ish overlays use the same `ui/` tree.

## 4. Current coverage (manifest snapshot 2026-10-30, while parity-dev re-extracts)

| Metric | Value |
|---|---|
| Expectations | **242** (P0 37, P0b 76, P1 44, P2 28, P3 57) |
| Extracted & matched | **215/242** via manifest aliases (no exact logical-name matches yet — the game's own names are used, see §2.3) |
| Required missing | **1** — `ui/material/color_spectrum` (colour-killer hue gradient: needs shader/colour documentation, not a sprite) |
| Optional missing | **16** — 14 achievement icons the game itself ships no art for (`AchievementManager` logs "missing full icon set") + 2 title-screen art entries (scene-only art; screenshot fallback acceptable) |
| Runtime-generated (no PNG) | **10** — procedural previews (body/mouth/eyes/arms/exoskeleton/plant/meat/pellet/egg) and the FOV target indicator |
| 9-slice entries | **66** (panel frames/headers, buttons, toggles, sliders, dropdowns, inputs, tabs, scrollbars, progress, graph lines, separator) |
| C# graphic fields scanned | **237** (236 prefab-only, 1 runtime/render-texture) |
| Unclaimed extracted UI names | **158** — extra sprites available to view-dev (tool icons, social/help icons, placeholders); informational, not blockers |

Notes:
* The extraction is in flight; counts were measured against a pinned snapshot of
  `assets/game/manifest.json` (391 KB, 475 UI entries, 266 unique UI sprite names, 22 fonts). Re-run
  the checker after parity-dev's final pass.
* The game's extracted UI sprites already carry Unity `border` (9-slice) metadata, e.g. `UISprite`,
  `Background`, `InputFieldBackground`, `UIMask` = 10 px; `Button_UI` = 30 px; `Long_UI` = 70 px;
  `base_icon` = 5 px; `tiled_bars_1` = 4/5 px. The viewer should read borders from the manifest, not
  hardcode them.
* Pivot expectations: fill/progress entries expect left-centre `[0, 0.5]` (Image.filled); knobs and
  icons centre `[0.5, 0.5]`; everything else centre unless the manifest says otherwise.

## 5. Prefab-only extraction needs (for parity-dev)

These are the sprite *roles* that exist only in Unity prefabs; C# gives us field names but not
assets. The extraction must scan `level0..level4` / `globalgamemanagers` prefabs and map them:

| Group | C# evidence (field_inventory) | Notes |
|---|---|---|
| Panel backgrounds/headers | 31 `UIPanel` subclasses' serialized `Image` fields; `UIPrefabsHolder` prefabs | Most panels reuse `UISprite`/`gray_panel*`/`Long_UI`; per-panel frames in the JSON are logical roles, not distinct textures |
| Buttons/toggles/sliders/dropdowns/inputs/tabs/scrollbars | `UIPrefabsHolder.{SettingSliderPrefab, BoolSettingPrefab, SettingDropdownPrefab, SettingGroupPrefab, SubSettingsGroupPrefab, SubSettingsToggleMasterPrefab, ColorPickerPrefab, TextLinePrefab, ConditionGroupPrefab}` | Shared widget prefabs; extracting these prefabs yields the exact sprites + borders/pivots |
| Graph chrome | `GraphsPanel`, `DataStreamLinesGraph`, `SingleLineGraph`, `EditableLineGraph`, `BarGraph` serialized `Image`s | Lines may be `LineRenderer` materials rather than sprites |
| HUD status bars | `StatusBarsReference.{healthBar, energyBar}` | Already extracted as `health_filled`/`energy_filled` |
| Selection/FOV overlays | `UserControl.{highlightMaterial, selectionRect}`, `FieldOfViewDisplay`, `WeightDisplay` | Materials/shaders, not PNGs |
| Tooltips/popups/notifications | `TooltipSystem`/`PopupManager` prefabs | Reuse `UISprite`/`gray_panel*` |
| Tool panels (rad/phero/colour killer) | `RadTowerPanel`, `PheroTowerPanel`, `ColorKillerPanel` widget references | Mostly shared widgets + colour gradients |

## 6. Known gaps and follow-ups

1. **Colour-killer hue spectrum material** (`ui/material/color_spectrum`): not in the manifest.
   The panel uses a gradient/shader for the hue bar (`ColorKillerPanel.colorSpectrum`) and the
   tolerance band (`tolerance` material, present). Ask parity-dev to extract the material or record
   its shader name + colour ramp so the viewer can reproduce the gradient.
2. **Title-screen art** (`ui/menu/title_logo`, `title_background`): optional; lives in scene/level
   data, not the UI sprite batch. A screenshot fallback is acceptable for P3.
3. **Achievement icons**: 14 optional (game ships no full icon set); non-goal for M1.
4. **Unclaimed extras**: 158 extracted UI names are not yet mapped to logical expectations. They are
   mostly icons the prefab UI uses (settings/save/delete/rename/tool/social icons). view-dev can use
   them from the manifest directly; add curated entries if a panel needs a specific one.
5. **Fonts**: the extraction provides TTF files (`Roboto-Regular`, `Roboto-Bold`, `Roboto-*`,
   `LiberationSans`, `CONSOLA`, `Inconsolata-SemiBold`, `PerfectDOSVGA437`) and TMP SDF atlas
   textures. Confirm which TMP font asset the in-world HUD/prefabs actually use before converting
   (view-dev to validate against screenshots).

## 7. Maintenance

* After parity-dev changes the extraction: `python tools/assets/build_ui_expectations.py` (if the
  inventory changed) and re-run the checker; update the alias table for renamed assets.
* When view-dev needs a new UI asset: add the entry to `build_ui_expectations.py` (curated section),
  regenerate, re-run coverage, and (for prefab-only art) add the role to §5 so parity-dev extracts it.
* When a new extraction batch lands: pin a snapshot if the file is being regenerated concurrently
  (`--manifest target/manifest-snapshot.json`) and report counts in this doc.
* Do not edit `assets/game/` from this task; it is parity-dev's output. Do not edit `Here/`.

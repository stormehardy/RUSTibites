# UI asset map for the viewer

**Status:** implementation-ready (task-84). **Owner:** spec-dev. **Consumer:** view-dev (UI layer).
**Upstream owners:** parity-dev (owns `assets/game/` — this task never writes there), spec-dev
(expectations catalogue, `docs/ASSETS_UI.md`).

This document explains `tools/ui/ui_atlas.json`, the single mapping view-dev uses to load UI artwork
without re-deriving names: **logical UI id → extracted file, kind, 9-slice borders/pivot, tint/draw
defaults, fonts, brain-icon index tables, materials and procedural fallbacks.**

```powershell
python tools/ui/build_ui_atlas.py       # regenerate ui_atlas.json (deterministic)
python tools/ui/check_ui_atlas.py       # validate: coverage, files, borders, icons, fonts
python tools/ui/check_ui_atlas.py --strict   # gate: exit 1 unless fully valid
```

Current result: **242 entries, 216 mapped, 10 runtime-procedural, 16 known-optional; 0 required
gaps; 216 files checked, 0 missing; strict checker passes (exit 0).**

Related: [`../ASSETS_UI.md`](../ASSETS_UI.md) (expectation catalogue + extraction coverage),
[`UI_PARITY.md`](UI_PARITY.md) (UI inventory/plan), `assets/game/manifest.json` (parity-dev output),
`assets/game/prefabs.json` and `assets/game/reference/` (in-world prefabs + reference screenshots).

---

## 1. Schema (version 1)

Top-level keys of `tools/ui/ui_atlas.json`:

| Key | Content |
|---|---|
| `schema_version` | `1` — bump only on breaking schema changes |
| `generated_by` | `tools/ui/build_ui_atlas.py` |
| `sources` | input paths + manifest schema/stats + prefabs/reference paths |
| `reference` | `assets/game/reference/` screenshot frames (file/width/height) for screenshot-diffing |
| `naming` | atlas-key/file/kind/priority conventions |
| `counts` | entries, by kind/priority/status, mapped, M1-core count, needs_prefab, inferred borders, gap counts |
| `atlas` | **map** `logical id → record` (see below), sorted by logical id |
| `missing_required` | core-priority entries that could not be resolved (currently empty) |
| `optional_missing` | known optional art (14 achievement icons the game lacks, 2 title-scene art entries) |
| `needs_prefab` | logical ids whose exact 9-slice border/font size needs the runtime prefab dump |
| `inferred_borders` | 9-slice records whose border was inferred (subset of `needs_prefab`) |
| `fonts` | `roles` (body/bold/mono_numeric/display/fallback), `files` (all extracted TTFs), `tmp_atlas_textures`, `substitution_rules` |
| `brain_icons` | `inputs[33]`, `outputs[15]`, `activations[13]` — ordered index→filename tables |
| `materials` | resolved UI materials (`logical → {name, shader, file, colors, floats}`) |
| `available_ui_materials` | all manifest `ui.materials` entries (color_spectrum, tolerance, field_of_view, skin_recolorizer, eye_recolorizer) |
| `procedural` | runtime-generated UI art (no PNG; use the in-world procedural sprites) |
| `implement_first` | ordered P0/P0b/P1 checklist for view-dev (see §8) |

### Atlas record fields

| Field | Meaning |
|---|---|
| `logical` | stable UI id (same as the expectations catalogue) |
| `id` | dotted form of `logical` |
| `priority` | `P0`/`P0b`/`P1` = M1 core UI; `P2`/`P3` deferred |
| `usage` | logical role (`panel`, `button`, `icon`, `hud`, `overlay`, `material`, `font`, …) |
| `surfaces` | UI_PARITY surface ids (e.g. `["B1"]`, `["W7"]`) |
| `kind` | `sprite` \| `nine_slice` \| `icon` \| `font` \| `material` \| `procedural` |
| `status` | `mapped` (file resolved) \| `runtime` (generated at runtime) \| `unmatched` (known-optional) |
| `file` | repo-relative extracted file (PNG/JSON) or `null` |
| `manifest_name` / `source_asset` / `sheet` | provenance from parity-dev's manifest |
| `rect`, `natural_size_px` | sprite rect / natural pixel size (for source atlas) |
| `pivot` | `[x, y]` from Unity metadata (default `[0.5, 0.5]`) |
| `border` | 9-slice border `[left, bottom, right, top]` in source pixels, or `null` |
| `border_inferred` | `true` when no Unity border existed and the default was inferred |
| `nine_slice` | whether the asset is drawn as a 9-slice |
| `needs_prefab` | metadata (exact border/size) still requires the runtime prefab dump |
| `tintable`, `default_tint` | per-asset tint guidance (`[r,g,b,a]` 0–255) |
| `default_draw_size_px` | inferred draw size at 1080p (`null` for stretched 9-slice/panels) |
| `size_inferred` | draw size is a default, not the prefab layout rect |
| `aliases` | logical id + manifest candidate names used for resolution |
| `shared_with` | other logical ids intentionally reusing the same texture (shared chrome) |
| `m1_core_ui` | `priority ∈ {P0, P0b, P1}` |
| `font_role` | for `kind: font` — which `fonts.roles` entry to use |
| `material` | for `kind: material` — resolved material metadata |

Minimal Rust-ish consumption:

```rust
let atlas: UiAtlas = serde_json::from_str(&std::fs::read_to_string("tools/ui/ui_atlas.json")?)?;
let btn = &atlas.atlas["ui/button/button"];          // kind=nine_slice, file, border, pivot
let tex = textures.load(&btn.file.unwrap())?;        // existing asset/texture loader
// 9-slice draw: split by border [l,b,r,t], stretch the middle; tint with default_tint.
let body = &atlas.fonts.roles["body"];
let icon_file = &atlas.brain_icons.inputs[0].file;   // input_energy_ratio
```

## 2. Coverage and gaps

| Metric | Value |
|---|---|
| Logical entries | **242** (P0 37, P0b 76, P1 44, P2 28, P3 57) |
| M1 core UI entries | **157** |
| Mapped to extracted files | **216** |
| Runtime-procedural (no PNG) | **10** |
| Known-optional (game/scene-only art) | **16** (14 achievement icons the game itself lacks + title logo/background) |
| Required gaps | **0** |
| Kinds | nine_slice 66, icon 120, sprite 33, material 10, procedural 10, font 3 |
| Files verified on disk | 216 checked, **0 missing** |
| 9-slice records | 66 (63 with Unity border metadata, 3 inferred) |
| Shared-alias records | 86 (many panels reuse one texture — see §3) |
| `needs_prefab` | 22 (3 fonts + 3 inferred borders + 16 optional) |
| Brain icons | inputs 33/33, outputs 15/15, activations 13/13 |
| Materials | 10 resolved (incl. `color_spectrum` → `rainbowMaterial`) |

**Known gaps / caveats** (all documented in `assets/game/manifest.json` or expected):
1. `ui/icon/achievement/ACH_{BIBITE_100CHILDREN,BIBITE_1DAY,BIBITE_EDITOR_100,BIBITE_EINSTEIN,SIM_1FPS,SIM_7DAYS,USER_SNAP}_{0,1}` — the game ships no icon set for these achievements; skip or show a locked placeholder.
2. `ui/menu/title_logo`, `ui/menu/title_background` — scene-only art; screenshot fallback is acceptable (P3).
3. Shader **names** are stripped by Unity in this build; material JSON carries the serialized
   properties (`_hue`, `_offset`, `_width`, …). Reimplement effects from properties (e.g.
   `rainbowMaterial` = hue ramp; tolerance = gradient band), not from the shader name.
4. UI **prefab layout** (rects, exact TMP sizes, per-panel anchors) is not in the C# or the current
   manifest: `needs_prefab` and `size_inferred` flag everything affected. A runtime MonoBehaviour
   prefab dump is the future fix; until then use the reference screenshots + inferred defaults.
5. Graph/pie art may be generated meshes/Lines rather than sprites; the map lists the chrome
   (`Line`, `Square`, `Circle`, `rond`, `tiled_bars*`) and marks the ring segment as a possible
   generated mesh.

## 3. Alias / shared-chrome rationale

The game reuses a handful of 9-slice textures and icons across many panels. The atlas keeps the
game's own asset names (`file`/`manifest_name`) and maps **many logical ids to one file**, recording
`shared_with` so view-dev loads each texture once and reuses it. Most-shared files:

| Texture | Logical users |
|---|---|
| `sprites/UISprite.png` (9-slice, border 10) | 32 (most panel frames, version plate) |
| `sprites/gray_panel_head2.png` | 19 (panel headers) |
| `sprites/Line.png` | 5 (separator, graph grid/axis, selection rect) |
| `sprites/Circle.png` | 4 (slider knob, graph point, chart segment) |
| `sprites/Button_UI.png` | 3 (button, small button, tabs) |
| `sprites/Arrow.png` | 3 (arrow icons) |
| `sprites/Checkmark.png` | 3 (toggle/checkmark) |
| `sprites/tiled_bars_full.png` | 3 (progress fill, loading fill, slider fill) |

**Rule:** never duplicate a texture per panel; load once by `file` and draw per logical id using its
border/pivot/tint. Per-panel distinct artwork does not exist in the extracted UI batch — this is the
game's actual reuse pattern. Do not rename files in `assets/game/`; if a mapping is wrong, fix the
alias table in `tools/assets/build_ui_expectations.py` and regenerate.

## 4. Fonts: roles, files, substitution rules

`fonts.roles` (all role mappings inferred from font/SDF names; exact prefab usage needs the runtime
dump — flagged `needs_prefab`):

| Role | TTF | TMP SDF atlas texture | Reference size @1080p |
|---|---|---|---|
| `body` (default UI text) | `assets/game/fonts/Roboto-Regular.ttf` | `…/Roboto-Regular_SDF_Atlas.png` | 16 px |
| `bold` (headings/labels) | `assets/game/fonts/Roboto-Bold.ttf` | `…/Roboto-Bold_SDF_Atlas.png` | 20 px |
| `mono_numeric` (time/counters/node values) | `Inconsolata-SemiBold.ttf` (candidates: `PerfectDOSVGA437.ttf`, `CONSOLA.ttf`) | `…/joystix_monospace_SDF_Atlas.png` | 16 px |
| `display` (title/display) | — | `…/Boston_Black_SDF_Atlas.png` | 20 px |
| `fallback` (Latin fallback) | `assets/game/fonts/LiberationSans.ttf` | `…/LiberationSans_SDF_Atlas.png` | 20 px |

Rules (also in `fonts.substitution_rules`):
1. Prefer the **TMP SDF atlas** for the game's exact TMP look (distance-field glyph quads), sized at
   logical px × UI scale.
2. Fallback: load the TTFs and render at logical sizes; scale factor =
   `min(viewport_h/1080, viewport_w/1920) × UIScaler.ui_scale`.
3. Reference sizes (inferred): body 16, bold/heading 20, small 14, mono 16. Validate against
   `assets/game/reference/*.png`; if the prefab sizes differ, update `reference_size_px`.
4. Monospace role is a candidate set (Inconsolata-SemiBold / PerfectDOSVGA437 / CONSOLA / joystix
   SDF); lock it after screenshot comparison of the time/counter text.
5. If a TMP atlas is unusable in the renderer, use the TTF with letter-spacing/scale compensation
   and record the visual delta.

All extracted font files are listed in `fonts.files` (22 entries; Roboto family, LiberationSans,
CONSOLA, Inconsolata-SemiBold, PerfectDOSVGA437).

## 5. Brain icon index → filename tables

Source: `Here/BrainIconHolder.cs` — `brainIcons[0..32]` = inputs, `brainIcons[33..47]` = outputs,
`brainFunctionsIcons[0..12]` = activation functions (used by `UINeuron`, `AddNodeItem`, `NodeEditor`).
All 61 files resolve in the current extraction.

### Inputs (33)

| Idx | Input | Extracted file |
|---:|---|---|
| 00 | EnergyRatio | `input_energy_ratio` |
| 01 | Maturity | `input_maturity` |
| 02 | LifeRatio | `input_life_ratio` |
| 03 | Fullness | `input_fullness` |
| 04 | Speed | `input_speed` |
| 05 | RotationSpeed | `input_rotation_speed` |
| 06 | IsGrabbing | `input_is_grabbing_objects` |
| 07 | AttackedDamage | `input_attacked_damage` |
| 08 | EggStored | `input_eggstored` |
| 09 | BibiteCloseness | `input_bb_conc_weight` |
| 10 | BibiteAngle | `input_bb_conc_angle` |
| 11 | NBibites | `input_n_visible_bb` |
| 12 | PlantCloseness | `input_plant_conc_weight` |
| 13 | PlantAngle | `input_plant_conc_angle` |
| 14 | NPlants | `input_n_visible_plant` |
| 15 | MeatCloseness | `input_meat_conc_weight` |
| 16 | MeatAngle | `input_meat_conc_angle` |
| 17 | NMeats | `input_n_visible_meat` |
| 18 | RedBibite | `input_closest_bb_r` |
| 19 | GreenBibite | `input_closest_bb_g` |
| 20 | BlueBibite | `input_closest_bb_b` |
| 21 | Tic | `input_tic` |
| 22 | Minute | `input_minute` |
| 23 | TimeAlive | `input_time_alive` |
| 24 | PheroSense1 | `input_pherosense1` |
| 25 | PheroSense2 | `input_pherosense2` |
| 26 | PheroSense3 | `input_pherosense3` |
| 27 | Phero1Angle | `input_phero1_angle` |
| 28 | Phero2Angle | `input_phero2_angle` |
| 29 | Phero3Angle | `input_phero3_angle` |
| 30 | Phero1Heading | `input_phero1_heading` |
| 31 | Phero2Heading | `input_phero2_heading` |
| 32 | Phero3Heading | `input_phero3_heading` |

### Outputs (15)

| Idx | Output | Extracted file |
|---:|---|---|
| 00 | Accelerate | `output_accelerate` |
| 01 | Rotate | `output_rotate` |
| 02 | Herding | `output_herding` |
| 03 | EggProduction | `output_eggProduction` |
| 04 | Want2Lay | `output_want2lay` |
| 05 | Want2Eat | `output_want2eat` |
| 06 | Digestion | `output_digestion` |
| 07 | Grab | `output_grab` |
| 08 | ClkReset | `output_clk_reset` |
| 09 | PhereOut1 | `output_pher_out1` |
| 10 | PhereOut2 | `output_pher_out2` |
| 11 | PhereOut3 | `output_pher_out3` |
| 12 | Want2Grow | `output_want2grow` |
| 13 | Want2Heal | `output_want2heal` |
| 14 | Want2Attack | `output_want2attack` |

### Activation functions (13)

| Idx | Activation | Extracted file |
|---:|---|---|
| 00 | Sigmoid | `hidden_sigmoid` |
| 01 | Linear | `hidden_linear` |
| 02 | TanH | `hidden_tanh` |
| 03 | Sine | `hidden_sine` |
| 04 | ReLu | `hidden_relu` |
| 05 | Gaussian | `hidden_gaussian` |
| 06 | Latch | `hidden_latch` |
| 07 | Differential | `hidden_diff` |
| 08 | Abs | `hidden_abs` |
| 09 | Mult | `hidden_mult` |
| 10 | Integrator | `integrator` |
| 11 | Inhibitory | `hidden_adaptor2` (game's legacy name) |
| 12 | SoftLatch | `hidden_softlatch` |

Full paths are in `atlas["ui/icon/brain_*"]` and `brain_icons.*[*].file`.

## 6. 9-slice usage notes

* `border` order is Unity's `[left, bottom, right, top]` in **source pixels**; the middle region is
  stretched, corners kept 1:1. `natural_size_px` is the source rect size (e.g. `Button_UI` 360×360
  with 30 px border; `UISprite` 32×32 with 10 px border).
* 63 of 66 nine-slice records use Unity-provided borders (from `manifest` sprite `border`). Three
  records had no border metadata and use inferred defaults, flagged `border_inferred` +
  `needs_prefab`: `ui/graph/graph_bar_fill`, `ui/scroll/scroll_handle`, `ui/toggle/toggle`.
* Inferred defaults by usage (until the runtime prefab dump): panel/tooltip/input/tabs/dropdown 10,
  button 30, toggle 8, slider/progress/scroll 4, popup 12, notification 8, graph 2, menu 10.
* Pivots: fills/progress expect left-centre `[0, 0.5]` (Image.filled); knobs/icons centre
  `[0.5, 0.5]`. The atlas carries `pivot` per record where Unity had one.
* Panels: `ui/panel/*_frame` are **not** distinct textures; draw them with the shared texture in
  `file` and the border. Panel headers use `gray_panel_head2` (shared by 19 panels).
* Selection rectangle is a tinted 1×1/`Line` border, not a frame texture.

## 7. Materials and procedural UI art

Resolved materials (`atlas.materials`): `ui/material/color_spectrum` → `rainbowMaterial` (hue ramp,
floats `_hue`, `_offset`), `tolerance_band` → `tolerance` (`_width`), `fov_cone` → `FieldOfViewMat`,
`selection_highlight_material` → `highlight`, plus `ui_egg`, `node_activation`, `synapse_activation`,
`lineage_graph`, `stomach_acid`, `internals_preview`. Each record has `shader` (when known), `file`
(JSON of serialized properties), and a filtered `floats`/`colors` view. **Shader names are often
stripped by Unity — reimplement from properties.**

Runtime-procedural UI art (no PNG; reuse the in-world procedural sprites):
body/mouth/exoskeleton/eye/arm/plant/meat/pellet/egg previews and the FOV target indicator. See
`atlas.procedural` and `atlas["ui/preview/procedural_*"]`.

## 8. What view-dev should implement first (checklist)

Derived from `implement_first` in the atlas. Each step lists exact logical ids.

**Phase P0 — in-world HUD/read-only core UI**
1. **Atlas loader + 9-slice renderer** — panel frames (`ui/panel/hud_time_frame`,
   `hud_ecology_frame`), buttons (`ui/button/button`, `button_icon`, `close_x`), tooltip/popup frames.
   Load textures by `file` once; draw 9-slice using `border`; tint with `default_tint`.
2. **Fonts** — `ui/font/tmp_regular`, `tmp_bold`, `tmp_sdf_atlas` + `fonts.roles`; UIScaler scaling
   and the TTF fallback rule.
3. **HUD** — `ui/hud/time_panel_bg`, `pause_icon`, `play_icon`, `step_icon`, `statusbar_frame`,
   `statusbar_health_fill`, `statusbar_energy_fill`, `rightmost_tabs`, `visibility_toggle_icons`.
   Status fills tint from ratios; speed slider reuses the shared slider widgets.
4. **Core widgets** — buttons, toggles, sliders, dropdowns, progress, inputs, tabs, scroll, separator.
5. **Selection/overlay + core icons** — `ui/overlay/selection_rect`, checkmark/arrow/warning/error/
   info/star icons, highlight material.
6. **Panels** — bibite Stats + Genes frames/headers, tooltip frame (widgets + flat labels).

**Phase P0b — full inspection (read-only)**
1. **Biology** — frame/header + `ui/material/stomach_acid`, `internals_preview` (organ schematic) +
   stomach pellet widgets.
2. **Brain simple** — frame + `node_activation`/`synapse_activation` materials + the three icon
   index tables (§5); node positions from `Brain` records/species template anchors.
3. **Statistics / multi-selection / escape / tool panels** — stats frame + chart rings, selection
   frame, escape frame, tool-panel frame; colour-killer `color_spectrum` + `tolerance` gradients;
   rad/phero towers reuse shared widgets.
4. **Dialogs + notifications** — popup/notification/tooltip frames (shared chrome; UI pause/ESC
   stacks are state work, not assets).

**Phase P1 — analysis overlays**
1. **Graphs + lineage** — graphs frame, graph grid/axis, lineage elbow/junction connectors; graph
   series come from the viewer-side 60 s sampler (see `UI_PARITY.md` §10).
2. **Species/ancestral lineage** — frames/headers; `lineage_graph` material.
3. **FOV/ruler/pheromone overlays** — runtime target indicators (procedural), `ruler` chrome,
   `fov_cone` material.

**Before starting**, view-dev should confirm with parity-dev/Lead: the TMP SDF atlas can be decoded by
the renderer (else TTF fallback), and whether the `needs_prefab` layout/size values are acceptable as
inferred defaults until a runtime prefab dump exists.

## 9. Regenerate / update

```powershell
python tools/ui/build_ui_atlas.py     # deterministic; reads expectations + manifest only
python tools/ui/check_ui_atlas.py --strict
```

* **Do not edit `assets/game/`** (parity-dev's output) or `tools/assets/ui_expectations.json`
  directly (generated by `tools/assets/build_ui_expectations.py`).
* If an alias is wrong: update the alias table in `tools/assets/build_ui_expectations.py`, regenerate
  expectations, then the atlas; re-run both checkers.
* If parity-dev's manifest changes: regenerate the atlas (schema guard in the checker), and update
  §2 counts here if the numbers move.
* If view-dev needs a new UI asset: add it to the expectation catalogue first (so parity-dev can
  extract it), then regenerate the atlas; add the logical id to the relevant implement-first step.
* The atlas is generated from data only (no timestamps, sorted keys) — diffs are reviewable and the
  file is safe to commit; `check_ui_atlas.py --strict` is the gate.

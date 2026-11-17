# UI_LAYOUT — extracted game UI layout trees (task-96)

Asset-file-only extraction of The Bibites' UI hierarchy into `assets/game/ui_layout.json` (tracked,
versioned `rustibites.assets.ui_layout.v1`). **No game launch / no visible window** was used. Tool:
`tools/assets/ui_layout_dump.py`; regenerate with:

```powershell
python tools\assets\ui_layout_dump.py            # writes assets/game/ui_layout.json
python tools\assets\ui_layout_dump.py --out target\ui_layout_check.json   # determinism check
```

The dump is deterministic (two runs produce byte-identical JSON; current sha256
`b4542c9cd4e63229226c696c5ebfe8b9004aa31ec1d85667c3f80de7af560e68`, 14.6 MB, 7051 RectTransform
nodes, 24,511 components, 140 canvases across level1–4 + sharedassets0–2).

## What is exact

* **RectTransform geometry**: `anchor_min`, `anchor_max`, `anchored_position`, `size_delta`, `pivot`,
  `local_position`, `local_rotation`, `local_scale`, active flag, layer/tag, sibling order and the full
  parent/child tree per canvas/prefab root.
* **Canvas** fields: render mode, sorting order, override sorting, pixel perfect, plane distance,
  target display.
* **Component class names** for every component, including MonoBehaviours (resolved through the
  `m_Script` MonoScript pointer even though the build strips MonoBehaviour typetrees): e.g.
  `UnityEngine.UI.Image` (2756), `UnityEngine.UI.LayoutElement` (2338), `TMPro.TextMeshProUGUI` (1826),
  `HorizontalLayoutGroup` (988), `Button` (630), `VerticalLayoutGroup` (501), `ContentSizeFitter` (242),
  `RectMask2D` (119), `TMP_InputField` (107), `Scrollbar` (100), `ScrollRect` (93), `Toggle` (82),
  `Slider` (81), `CanvasScaler` (56), `AspectRatioFitter` (52), `TMP_Dropdown` (48), plus the game's
  `UIScripts.*` panel/handle scripts.
* **Canvas roots** (140; names include `Title Screen`, `UI`, `LoadingPanel`, `TooltipCanvas`,
  `Panel Holder`, `BibiteEditor`, `WorkshopItemsPanel`, `TopLayer`, `Layer2`, `PopupHolder`, …) with
  source asset file, path id, active state and their level root.
* **UIScaler formula** (from `Here/Utility/UIScaler.cs`, recorded in the JSON): 
  `CanvasScaler.scaleFactor = totalScale = ScreenResolutionFactor * UIScale`;
  automatic `ScreenResolutionFactor = min(Screen.height / 1080, Screen.width / 1920)`; `UIScale` is a
  user setting (default 1.0).

## What is NOT readable statically (known gap)

The shipped build has **no Unity script typetrees**, so MonoBehaviour *serialized fields* are not
readable by UnityPy (the object bytes are present but field layout is unknown). That means the following
are **not** in `ui_layout.json` and must be filled from another source:

| field group | examples | source / fallback |
|---|---|---|
| Image | `sprite`, `color`, `type` (simple/sliced/tiled/filled), `fillAmount`, `fillMethod`, `preserveAspect` | runtime plugin UI dump (recommended) or `tools/ui/ui_atlas.json` + Here/ defaults |
| TMP/Text | font asset/role, fontSize, alignment, color, `enableWordWrapping`, margins | `tools/ui/ui_atlas.json` font roles + Here/ default sizes; runtime dump for exact values |
| CanvasScaler | `referenceResolution`, `screenMatchMode`, `matchWidthOrHeight`, `scaleFactor` (set at runtime by UIScaler) | UIScaler formula above; per-canvas values need runtime dump |
| Layout groups | padding/spacing/child-alignment/control flags | Here/ defaults; runtime dump for exact serialized values |
| UIScript references | button handlers, panel fields, setting handles | Here/ source + runtime dump |

`ui_layout.json.gaps` records these explicitly. The geometry is exact and already sufficient to match
panel placement/sizes/hierarchy; sprite/font/colour values are the remaining gap.

**Recommended follow-up:** a transient runtime plugin UI dump (same reflection approach as ParityDump's
`/d/feeding`) walking `Canvas`/`RectTransform` at runtime, reading Image/TMP/CanvasScaler/LayoutGroup
fields via reflection and sprouting `/d/ui_layout`. That yields the missing serialized values without any
visible window; join with `assets/game/manifest.json` sprites and `tools/ui/ui_atlas.json` font roles.

## How view-dev should consume it

1. Load `assets/game/ui_layout.json`; pick `files[<scene/asset>]` and walk `trees` (one root per
   canvas/prefab hierarchy). Node keys: `name`, `active`, `rect{...}`, `components[]`, `children[]`,
   `source_file`, `path_id`, `is_canvas_root`.
2. Build the same hierarchy in the viewer and apply `rect` exactly (anchors/pivot/anchoredPosition/
   sizeDelta drive layout relative to the parent RectTransform; localScale/rotation for authored
   scale/rotations).
3. For each component, use `script` (e.g. `UnityEngine.UI.Image`, `TMPro.TextMeshProUGUI`,
   `UnityEngine.UI.HorizontalLayoutGroup`) to choose behaviour. Where serialized fields are missing,
   resolve sprites via `assets/game/manifest.json` / `tools/ui/ui_atlas.json` logical names, fonts via
   `tools/ui/ui_atlas.json` roles (body/bold/mono), and colours/sizes from Here/ defaults or the
   runtime-dump follow-up.
4. Canvas roots with their `Canvas` fields give display/sorting order; `uiscaler` in the JSON gives the
   formula for the 1920×1080 reference scaling.

## Validation

* Deterministic: rerun produces identical sha256 (checked twice; see command above).
* Complete hierarchy walk: 7051 RectTransform GameObjects across the 7 UI-bearing asset files
  (level1 1590, level2 3234, level3 60, level4 8, sharedassets0 658, sharedassets1 1328,
  sharedassets2 173); all roots included (level1 9, level2 13, level3 8, level4 2, sharedassets0 6,
  sharedassets1 9, sharedassets2 8); max depth 21 (level2), 20 (level1).
* Every component was attributed to a class (0 unresolved scripts); 12,250 MonoBehaviour components
  are field-unreadable by design (documented gap), 2,756 Image and 1,826 TMP text components are named
  so their sprite/font fields can be joined from the atlas.
* Sprite/font resolution: **not resolvable from static serialized fields** (see gap table); all
  referenced logical sprite names are available in `assets/game/manifest.json`/`tools/ui/ui_atlas.json`
  for name-based joining, and `coverage.json` reports 0 required UI gaps in the atlas.

See `docs/ASSETS.md`, `docs/design/UI_PARITY.md` (visual contract) and `tools/ui/ui_atlas.json`
(242 logical UI assets → extracted files/fonts/9-slice) for the companion data.


---

## Runtime UI field dump (task-102) — fills the MonoBehaviour field gap

The asset-file dump cannot read MonoBehaviour serialized fields (Unity strips script typetrees), so the
transient ParityDump plugin now exposes `GET /d/ui`, which walks the **live** Canvas hierarchies in the
hidden-window game and reads the managed fields by reflection. Generator:
`python tools/assets/capture_ui_runtime.py --repeat 2` (hidden BibitesAPI lab; plugin deployed
transiently and removed afterwards). It captures the **main menu** and a fresh **Default.zip world**
(two hidden sessions for a determinism check). Raw output: `assets/game/ui_runtime.json.gz` (~1.7 MB
gzipped; uncompressed dump is regenerable and intentionally not tracked), index/metadata:
`assets/game/ui_runtime.index.json`.

**Coverage (current run)** — 12,232 UI nodes: main menu 26 canvases / 4,358 nodes; Default world 41
canvases / 7,874 nodes (active and inactive panels, including title screen, HUD, inspector panels,
tooltips, controls). Components serialized: **17,687 total** — 3,645 Image/RawImage fields, 3,182
TMP/Text fields, 72 CanvasScaler blocks, 7,046 LayoutGroup/LayoutElement/ContentSizeFitter records,
2,077 selectable/control/canvas components. Each node keeps RectTransform geometry for the ui_layout
join, component class, sprite name/rect/border/color/type/fill, font name/size/alignment/color/text,
CanvasScaler reference resolution/match/scaleFactor, layout paddings/spacing, active state and sibling
order.

**Determinism:** two hidden sessions produce structurally identical dumps. `determinism.structural_equal
= true`; the comparison ignores exactly these volatile runtime values (listed in the index/report):
`createdAtUtc`, `text` (clock/FPS/population etc.), `fill_amount`, `value`, `alpha`, `is_on`,
`scale_factor`, `anchored_position`, `local_position`, `size_delta`, `local_rotation`. Everything
structural (paths/hierarchy/component types/sprite+font names/colors/anchors/pivot/layout paddings/
CanvasScaler reference/match) is identical. Authored geometry remains authoritative in `ui_layout.json`;
use the runtime dump for the fields that file lacks.

### Consumer file for view-dev

`tools/ui/ui_runtime_atlas.json.gz` (~2.0 MB gzipped, schema
`rustibites.assets.ui_runtime_atlas.v1`) is the **joined consumer file**: runtime nodes with
Image/TMP/CanvasScaler/layouts **resolved against `assets/game/manifest.json` + `tools/ui/ui_atlas.json`**.
Metadata/index (including counts, missing names, capture/determinism): `tools/ui/ui_runtime_atlas.index.json`.
Read it with `gzip.open(path, "rt")` + `json.loads`.

Schema: `sessions.<main_menu|default_world>.canvases[]` → node
`{name, path, active, active_in_hierarchy, sibling_index, rect{anchor_min,anchor_max,anchored_position,
size_delta,pivot,local_position,local_rotation,local_scale}, images[], texts[], canvas_scalers[],
layouts[], controls[], children[]}`.
- `images[]`: `{sprite:{name, logical, file, manifest{rect,pivot,pixels_per_unit,border}, kind, border,
  default_tint}, color, image_type, fill_method, fill_amount, preserve_aspect, runtime_sprite_border}`.
- `texts[]`: `{text, font{name, role, logical, file}, font_size, alignment, color, word_wrap, margin,
  font_style, rich_text}`.
- `canvas_scalers[]`: reference_resolution/screen_match_mode/match_width_or_height/scale_factor.
- `layouts[]`: raw LayoutGroup/LayoutElement/ContentSizeFitter fields (padding, spacing, child
  alignment/control/expand, min/preferred/flexible sizes).
- `controls[]`: Button/Toggle/Slider/Scrollbar/ScrollRect/Selectable/Canvas/CanvasGroup fields.
- `stats.missing_sprites` / `stats.missing_fonts` list unresolved names.

**Join gaps in this run:** 1 missing sprite (`Video`) and 8 missing font names
(`Boston Black SDF`, `LiberationSans SDF`, `LiberationSans SDF - Fallback`,
`Roboto-Bold SDF BlackOutline`, `Roboto-Bold SDF NoOutline`, `Roboto-Bold SDF WhiteOutline`,
`Roboto-Regular SDF`, `UnityEngine.UI.FontData`) — these are TMP SDF variants/temporary objects that do
not map 1:1 to the atlas font roles; the atlas base roles (Roboto-Regular/Bold, Inconsolata, etc.) cover
the rendered text. All other sprite names resolved to manifest/atlas files.

### Sample difference vs `ui_atlas.json`

Asset files gave logical roles but no runtime values. Runtime now supplies, e.g. for the button/panel
pieces from the atlas (`ui/button/button → Button_UI`, panel aliases → `UISprite`/`gray_panel*`): exact
`sprite` object name, runtime `color`, `image_type`/`fill_*`, and the live `CanvasScaler`
reference resolution/match plus `scale_factor` at the hidden window's screen size; atlas keeps the
9-slice border (runtime sprite border also recorded and cross-checkable). Font roles from the atlas now
come with the exact TMP font asset name, size, alignment and color per text node.


### Runtime rect semantics + Canvas render modes (verified from the plugin code/dump)

* **Rect fields are local RectTransform values**, not canvas-space/world-space: `anchor_min`/`anchor_max`
  are normalized anchors in the parent rect, `anchored_position`/`size_delta` are the RectTransform
  layout values relative to those anchors, `pivot` is the normalized pivot, and
  `local_position`/`local_rotation`/`local_scale` are Unity transform-local values. Compose down the
  hierarchy (parent rect + parent localScale/rotation), then apply the canvas scale factor. For layout,
  prefer anchors+pivot+anchoredPosition+sizeDelta over localPosition (localPosition already includes
  the anchor/pivot offset; using both double-counts).
* **Canvas render modes** (from the `Canvas` component now captured with property fallbacks):
  - main menu: `Title Screen`/`LoadingPanel` = ScreenSpaceOverlay (UI Camera), `BackGround` =
    ScreenSpaceCamera (Main Camera), `UI` = ScreenSpaceCamera (UI Camera); 43 nested WorldSpace
    canvases (`UI/Popuholder` sorting 100, `UI/ScenarioEditor/Saver` 5, `UI/BibiteSelector/BibiteEditor`
    4, `UI/WorkshopItemsPanel` 4, `UI/TooltipCanvas` 150, dropdown Templates, Help Pop-ups, graph
    indicators; plane distance 100, no camera ref).
  - Default world: **no Overlay**; 5 ScreenSpaceCamera canvases all bound to `UICamera`:
    `UICanvas` (sorting 0), `UICanvas/MainUI/TopLayer` (20), `.../TopLayer/Popuholder` (100),
    `.../TopLayer/TooltipCanvas` (150), `UICanvas/MainUI/Layer2` (5); 78 WorldSpace nested canvases
    (Help Pop-ups sorting 11, dropdown Templates 16, graph indicators 25) plus in-world
    `WeightDisplayHolder` (sorting -1, WorldSpace). The atlas/index lists every mode/path; view-dev can
    skip the nested WorldSpace Help Pop-up/Template layers or treat them as tooltip overlays.
* Camera references captured: main-menu `Main Camera`/`UI Camera`; Default-world `UICamera` (names from
  Canvas.worldCamera). Plane distance is 100 for all observed canvases.

### Canonical Default in-world reference frames (view-dev visual regression)

`assets/game/reference/default/` (hidden-window bridge screenshots, no plugin, scenario `POST /newgame`
of `testdata/real/Default.zip`, paused at simulatedTime 0.25 s, counts = 1 bibite):
`full_world.png` 1920×1080 fit (camera 0,0 / orthoSize 2080), `population.png` 1920×1080
(0,0/120), `zone_pellets_eggs.png` 1920×1080 (0,0/230), `bibite_closeup.png` 1280×720
(camera 1018.39941,−471.2087 / orthoSize 12). `reference.json` + README record camera centre/ortho
size/world state; the parent `assets/game/reference/` lol frames are historical diagnostics.


### UICamera projection fields (ScreenSpaceCamera canvases) + save-based Default frames

The runtime dump now has top-level `cameras[]` per session: name, enabled, orthographic,
orthographicSize, position, rotation, pixelRect, rect, aspect, near/far clip, depth, background colour,
culling mask. `tools/ui/ui_runtime_atlas.json.gz` exposes them under `sessions.<session>.cameras`; each
Canvas `controls[]` still has render_mode + camera name. This is what maps the 5 Default-world
ScreenSpaceCamera canvases (`UICanvas`, `UICanvas/MainUI/Layer2`, `.../TopLayer`,
`.../TopLayer/Popuholder`, `.../TopLayer/TooltipCanvas`, all `UICamera`) and the main-menu
`BackGround`→Main Camera / `UI`→UI Camera canvases to screen space. Hidden reference screen was
1920×1080; UICamera orthographic size 540 at z −100 (Main Camera size 400 at z −855; ScreenShoter 2000).

Save-based Default frames: `assets/game/reference/default_save/` (from canonical Rust save
`testdata/parity/default/rust_seed_600.zip`, 10 bibites loaded by the hidden game and paused) —
`full_world.png` fit 1920×1080, `population.png` ortho 120, `zone_pellets_eggs.png` ortho 230,
`bibite_closeup.png` 1280×720 at the selected bibite (camera centres/ortho in `reference.json`/README).
This is the pixel-comparable set; the parent `default/` frames are scenario-newgame visuals, and the
lol frame set is historical.

Validation: `python tools/ui/validate_ui_camera.py` checks every Default-world ScreenSpaceCamera canvas
has render_mode/camera/planeDistance/sorting and that its camera has orthographic, orthographic_size,
position, rotation, pixel_rect, rect, aspect, near/far clip, depth and culling_mask. Last run: 5/5
canvases bound to UICamera, planeDistance 100, no missing fields (no game launch needed).


### Save-state runtime sessions (task-121)

`assets/game/ui_runtime.json.gz` / `tools/ui/ui_runtime_atlas.json.gz` publish save-state sessions in
addition to `main_menu`: `default_world` (Rust save `rust_seed_600.zip` loaded/paused t=20 s, fit camera,
1920×1080, UI scale 1; default HUD with `200u` ruler and active Info/quick-stats), `default_world_inspector`
(bibite selected via game StatsButton, panel animation settled, expanded Stats body), and
`default_world_newgame` (old t=0.25 dump). `ui_save_capture.json` + `reference.json` record camera/state/
selected id/panel. Limitation: `RulerHandle` draws the ruler bracket procedurally (script/mesh), not UI
Image components, so the runtime dump records Ruler node rects but no bracket sprite manifests; use the
`hud_savestate_overlay_1920x1080.png` alpha reference or draw it procedurally. Overlays:
`hud_savestate_{,overlay_}1920x1080.png`, `hud_inspector_expanded_{,overlay_}1920x1080.png`.

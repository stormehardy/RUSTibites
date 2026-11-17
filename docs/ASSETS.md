# ASSETS — extracted game art for visual parity (task-76)

Goal (user decision): the viewer must render **1:1 with the game** for in-world and core UI, using the
locally installed game's art. Assets are extracted read-only from
`The Bibites_Data` (game 0.6.3.1, Unity 6000.0.44f1) with UnityPy 1.25.4 and live **inside this repo**
at `assets/game/` (tracked). Entry point: `tools/assets/README.md`; index: `assets/game/manifest.json`.

**Licensing:** the repository code is MIT; the extracted art is **not** MIT and remains the property of
The Bibites' creator. It is included for this personal/fan build without warranty, with attribution and
a takedown note (`assets/game/LICENSE-ASSETS.txt`). The viewer must keep working if `assets/game/` is
removed (procedural fallback). Reference screenshots are game art too and covered by the same terms.

## What was extracted (current build)

| item | count | notes |
|---|---|---|
| Texture2D sheets | 398 | world procedural part sheets + object textures + UI atlases; lossless PNG with alpha |
| Sprite objects | 344 | decoded PNGs **plus exact `rect`, `pivot`, `pixels_per_unit`, 9-slice `border`** from the Sprite asset |
| procedural TextAssets | 5 | `parameters`, `bibites_parts`, `bibites_sizes`, `egg_sizes`, `pellet_sizes` → `assets/game/text/*.csv`, logical keys in `manifest.text_files` (avoids the Windows `parts`/`Parts` case collision) |
| fonts | 22 | all 22 Font objects exported as TTF (+ metrics/atlas names); includes Roboto family, Inconsolata-SemiBold, PerfectDOSVGA437, CONSOLA |
| materials | 84 | serialized colours/floats/textures per material, e.g. `ProceduralBibiteMaterial`, `recolorizerShader`, `ProceduralEyeMaterial`, `backgroundMat`, `radioZone`, `FieldOfViewMat`, `rainbowMaterial` |
| in-world prefabs | 10 roots | `bibite`, `egg`, `plant pellet`, `meat pellet`, `Zone`, `Radiation Tower`, `ColorKiller`, `PheroTower`, `FieldOfViewDisplay`, `background`: Transform local position/rotation/scale, SpriteRenderer sorting order/layer/colour/flip + resolved sprite names |
| UI | 237 sprites / 240 textures | from `sharedassets0-3.assets` + `resources.assets`; **21 sprites carry 9-slice borders**; UI material aliases include `color_spectrum` → `rainbowMaterial` (Shader Graphs/Rainbow, `_hue`/`_offset`) |
| reference frames | 4 | `assets/game/reference/` (full world, population, zone/pellets/eggs, bibite close-up) + `reference.json`/README with camera centre, orthographic size, world time/counts for each |
| review sheets | 3 | `assets/game/contact_sheets/` |
| total size | ~13 MB | well under the 100 MB budget |

**Coverage** (`assets/game/coverage.json`): in-world procedural **94.1%** (24/24 part sheets,
7/9 object sprite textures, parameters present) — meets the ≥90% gate. UI: 235 sprites / 240 textures /
21 nine-slice borders exposed; spec-dev's `tools/assets/check_ui_coverage.py --strict` reports
**0 required gaps** after the colour-spectrum material alias was added (215/242 matched at their last
run before the alias; their `ui_expectations.json` aliases map to our names).

## Manifest shape (for view-dev)

`assets/game/manifest.json` keys: `textures{name:{file,width,height,source_asset}}`,
`sprites{name:{file,sheet,rect,pivot,pixels_per_unit,border}}`, `sheets{sheet:[…parts]}`,
`text_files{name:{logical,file}}`, `procedural{parameters,sizes,egg_sizes,pellet_sizes,parts}`,
`ui{<assetfile>:{sprites,textures},materials:{color_spectrum,…}}`, `fonts[]`, `materials[]`,
`prefabs{roots,file}`, `reference{frames}`, `shader_resolution`, `stats`, `errors`.
Textures are also exported as raw sheets so a uniform-grid loader (`parts` rows/cols) works even without
the decoded per-frame sprite PNGs.

## Known gaps / proxies

* **Shader names**: Unity strips Shader asset names; materials have `shader_<PathID>` + serialized
  properties. The skin tint shader properties (`_FromHue`, `_VminAtS0`, `_VmaxAtS0`, `_BodyWidth`, …) are
  present in `materials/ProceduralBibiteMaterial.json` / `recolorizerShader.json`; the *name* needs a
  runtime plugin dump (follow-up). For tinting, use the serialized property values + the shipped
  `recolorizerShader`/`eyesRecolorizerShader` material rows.
* **Prefab material PPtrs** are external in this build (renderer material name null): use the material
  files by role (skin/eye/zone/background/FOV).
* **MonoBehaviour-only prefab components** (bibite AI, zone logic) are unreadable without script
  typetrees; `prefabs.json` records the readable Transform/SpriteRenderer hierarchy (child local
  transforms, sorting order/layer, flips, resolved sprites), which is what the layer composition needs.
* **Missing art in the build**: `medium_plant`/`medium_meat` pellet textures (game appears to reuse
  another material for medium pellets); some achievement icons and the title-screen logo/background do
  not exist as assets (spec-dev optional gaps; screenshots fallback).
* 22 font-atlas textures are not PNG-exportable by UnityPy; the TTF bytes are exported (text rendering
  needs the TTF, not the atlas).

## Reference frames (hidden-window policy)

`tools/assets/capture_reference.py` starts the game **hidden** through the BibitesAPI lab harness; the
bridge renders `Camera.main` to an offscreen RenderTexture (`GET /screenshot`), so **no visible window is
opened**. Frames record camera centre + orthographic size + world state (save `testdata/real/saves/lol.zip`,
`simulatedTime=469983.78125 s`, counts `{22,13,4}`, no seed on save load, paused) in
`assets/game/reference/README.md` and `reference.json`. The game is stopped in `finally`; no plugin is
deployed by this pipeline and no processes/listeners remain after the run.

## Regenerate

```powershell
python tools\assets\extract.py
python tools\assets\prefab_dump.py
python tools\assets\contact_sheets.py
python tools\assets\coverage.py
python tools\assets\capture_reference.py          # hidden game; bridge offscreen screenshots
python tools\assets\check_ui_coverage.py --strict # spec-dev UI expectation gate
```

See also `docs/ASSETS_UI.md` (spec-dev UI expectation list and prefab role notes) and
`docs/HEADLESS_POLICY.md`.


## Cell extraction / alias audit (task-137)

`tools/assets/check_sprite_cells.py` audits every unique exported cell PNG against its sheet using both
bottom-left (correct Unity) and top-left crop conventions and writes `assets/game/coverage_sprite_cells.json`.
Current result: **107 unique cell PNGs; 1 fully transparent** (`normal_exoskeleton_0.png`, live key
`normal_exoskeleton_00`); its sheet region is transparent in **both** crop orientations
(0 pixels), so this is not a crop-index bug. **0 transparent-mismatch** cells (a transparent export whose
sheet region has art). No per-cell PNG was changed by the explicit y-flip crop; UnityPy's sprite export
already matched the bottom-left sheet convention for these cells. `extract.py` now crops explicitly from the
sheet with the bottom-left rect conversion for future robustness, and adds padded runtime aliases
(`normal_body_0` ↔ `normal_body_00`, `normal_mouth_0` ↔ `normal_mouth_00`,
`normal_exoskeleton_0` ↔ `normal_exoskeleton_00`) so live SpriteRenderer names resolve directly.

Live `Texture2D` region proof (from `assets/game/reference/default_save/bibite_layers.json`, selected bibite):
Exoskeleton `normal_exoskeleton_00` **0/144** non-transparent pixels (blank by design); Eyes
`normal_eyes_21` 8/72; Body `normal_body_00` 65/99; Mouth `normal_mouth_00` 24/45; arms `normal_arms_10`
4/27 each. Thus the selected bibite's visible exo plates are not from the Exoskeleton layer. Separately,
live names `normal_eyes_21` and `normal_arms_10` are not present as extracted Sprite assets (runtime/
procedural variants), so viewers should keep a fallback for those names; this is a naming/data gap distinct
from the cell crop.

Current extracted total: ~35.9 MB (398 textures, 344 Sprite objects, plus UI/material/font assets).


## Runtime procedural sprite-name formula (task-137 follow-up)

Live bibite part sprites are named by `ProceduralSpriteManager.LoadAndSliceSpriteSheet` as
`<size>_<part>_<column><row>` (single digits, column first, row second), e.g. `normal_arms_10` =
normal_arms column 1 / row 0, `normal_eyes_21` = normal_eyes column 2 / row 1, `normal_exoskeleton_00` =
column 0 / row 0. These are **runtime-generated sprites over the shipped sheets**, not separate Sprite
assets, which is why `normal_eyes_21` / `normal_arms_10` are absent from `manifest.sprites`.

Cell geometry: sheet `nRows`/`nColumns` come from `assets/game/text/bibites_parts.csv`; cell width/height =
texture size / columns/rows; Unity sprite rect is `(x = col*cellW, y = row*cellH, w=cellW, h=cellH)` with
bottom-left origin. The live capture (`assets/game/reference/default_save/bibite_layers.json`) already has
`sprite_rect`, `sprite_pivot` (pixels), `pixels_per_unit`, `sprite_texture` and `sprite_texture_size` for
the runtime names. Examples from selected bibite:
`normal_eyes_21` rect (16,9,8,9) pivot (3,4.5); `normal_arms_10` rect (9,0,9,3) pivot (9,0);
`normal_body_00` rect (0,0,11,9) pivot (11,4.5); `normal_mouth_00` rect (0,0,5,9) pivot (0,4.5);
`normal_exoskeleton_00` rect (0,0,16,9) pivot (11,4.5) (this exoskeleton cell is genuinely blank; visible
plates come from Body).

**Pivot + pellet-scale follow-ups (tasks 138–139).** Runtime-named eye/arm cells fall back to the
nearest non-empty sheet cell when the formula cell is empty (`GameAssets::sprite_alpha_nonzero`).
Extraction-manifest pivots are already top-left-normalised (`normal_arms_10` → `(1.0, 0.0)`, not
`(1,1)`), which puts the arm quads on the correct side of their anchors. Pellet render scale is
`sqrt(amount / 20)` (`NormalAmountAtScale1 = 20`, authored radius 2.5 at scale 1), not
`sqrt(radius²/20)`; the reference pellet at (-6.628, 837.456) now matches the game's 188×187 px blob.

**Task-143/145 Body-layer result.** The atomic per-layer capture
(`closeup_single/layers/Body_2_alpha.png`) shows the game Body layer's visible alpha is a single
connected `(521,261,897,662)` = 376×401 px shape with a bottom-right tail extension. Extracted
`normal_body` cells (all three inflate rows) are half-discs with the same 65/99 alpha count as the live
texture region but only reach y≈605; the Body rect/texture/transform/bounds match the live capture and
the other five SpriteRenderers match separately. task-144 proved the live Body texture/cell is
pixel-identical to our extracted art; task-145's alpha diff (ours 74,262 px at `(521,261,882,605)` vs
game 77,307 px at `(521,261,897,662)`, IoU 0.9597, game-only 3,081 px at `(734,606,896,661)`) isolates
the difference to a tail-shaped extension. `prefabs.json` has a depth-1 **`Tail` node with a local
transform but no `sprite_renderer`** (mesh/deformed tail); the viewer has no tail-mesh renderer and the
extraction contains no tail mesh/texture. That is the remaining limitation — not Body art, pivot,
scale, or cell selection; no different tier/global stretch is substituted.


## Live Body sprite identity (task-144)

Atomic hidden closeup dump: Body SpriteRenderer sprite **`normal_body_00`**, texture **`normal_body`**
11×27, sprite rect (0,0,11,9), pivot (11,4.5), PPU 1; live full texture is **pixel-equal** to
`assets/game/sheets/normal_body.png`, and the live cell is **pixel-equal** to
`assets/game/sprites/normal_body_0.png` (65 non-transparent pixels; per-row cell alpha 93/77/65 for the
three rows). So the Body art is already present/extracted correctly; the remaining closeup bottom
difference is not missing Body art. Small refs: `closeup_single/body_texture_live.png`,
`body_cell_live.png`, `body_sprite_ref.json`; raw dump under `testdata/parity/default/body_sprite_dump/`.


## Tail deformation diagnostic (task-145)

There is no Tail mesh. The game bends the Body sprite in a shader (`ProceduralBibiteMaterial`, properties
`_TailAngle`/`_MiddleSegmentAngle` written by `ProceduralSpriteController`) using the Tail transform as a
chain node only. Live material dump: `assets/game/tail/procedural_bibite_material.json`; diagnostic sweep
(no usable float response in hidden capture, documented limitation): 
`assets/game/reference/default_save/tail_sweep/sweep.json` + alpha masks. Raw frames are gitignored.

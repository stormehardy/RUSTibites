> **Public repository: the pictures and fonts listed below are NOT here.** This folder holds only the extracted metadata (JSON / compressed JSON /
> CSV). `tools/release/extract_art.py` builds the pictures and fonts from your own copy of The Bibites into this folder, and `.gitignore` keeps them
> out of the repository. The inventory below describes a full local extraction.

# assets/game — extracted The Bibites art for the RUSTibites viewer

Extracted (read-only) from the locally installed **The Bibites 0.6.3.1 / Unity 6000.0.44f1** with
`tools/assets/extract.py` (UnityPy 1.25.4). This directory is **tracked in the repo** because the user
asked for 1:1 in-world/UI visuals; it is game art, **not MIT-licensed** — read `LICENSE-ASSETS.txt`.
The viewer must keep working if this directory is deleted (procedural fallback renderer).

## Inventory (current extraction)

| kind | count | location |
|---|---|---|
| Texture2D sheets (game art) | 398 (117 world + UI files) | `sheets/*.png`, `ui/<assetfile>/sheets/` |
| Sprite objects decoded to PNG | 344 (107 world + 237 UI) | `sprites/*.png`, `ui/<assetfile>/sprites/` |
| TextAssets (procedural CSVs) | 5 art inputs of 46 total | `text/{parameters,bibites_parts,bibites_sizes,egg_sizes,pellet_sizes}.csv` |
| Fonts | 22 (all exported .ttf; includes Roboto/Inconsolata/PerfectDOSVGA437/CONSOLA) | `fonts/*.ttf` |
| Materials | 84 (serialized colours/floats) | `materials/*.json` |
| In-world prefab roots | 10 (bibite, egg, plant/meat pellet, Zone, towers, ColorKiller, FOV, background) | `prefabs.json` |
| Reference in-world frames | 4 (+ notes/coords) | `reference/*.png`, `reference/README.md` |
| Contact sheets for review | 3 | `contact_sheets/*.png` |

Total size is ~13 MB. `manifest.json` is the authoritative index: `textures`, `sprites` (with exact
`rect`/`pivot`/`pixels_per_unit`/9-slice `border`), `text_files` (logical keys like
`ProceduralSprites/Bibites/parts`), `procedural` (parsed CSV maps), `sheets` (sheet → decoded parts),
`ui` (per-asset-file sprites/textures + 9-slice + UI material aliases incl
`ui.materials.color_spectrum` → `rainbowMaterial`), `fonts`, `materials`, `prefabs`, `reference`,
`coverage.json` (94.1% in-world; spec-dev's UI checker: 0 required gaps).

## How the viewer should use it

1. Read `manifest.json`; resolve logical names/aliases (`text_files`, `ui.materials`, `parts`).
2. Draw procedural bibites/pellets/eggs from `sheets` + `parts` rows/cols (uniform grid; parts CSV uses
   comma decimals in the shipped build). Prefer exact `sprites` rect/pivot/PPU when present.
3. Compose in-world prefab layers from `prefabs.json` child transforms (local position/scale), sorting
   order/layer and flips; tint via `materials/*.json` (skin `recolorizerShader`/`ProceduralBibiteMaterial`,
   eye `eyesRecolorizerShader`, background `backgroundMat`, zone `radioZone`, FOV `FieldOfViewMat`).
4. UI sprites are in `ui/<assetfile>/sprites/` with 9-slice borders in the manifest; fonts in `fonts/`.

## Known gaps

* Unity stripped Shader asset names: materials record `shader_<PathID>` (`shader_resolution` in the
  manifest) plus properties. Shader *names* need a runtime plugin dump; property values are present.
* Prefab SpriteRenderer material PPtrs are external in the shipped build (material name null); use the
  material files by role.
* 22 font-atlas textures are not PNG-exportable by UnityPy (`image_export_failed`); the `.ttf` bytes are
  exported, which is what text rendering needs.
* MonoBehaviour-only components (bibite AI, zone logic) are unreadable without script typetrees;
  `prefabs.json` records the readable Transform/SpriteRenderer hierarchy.
* Missing game art: `medium_plant`/`medium_meat` pellet textures are not in the build (the game appears
  to reuse another material for medium pellets); some achievement icons/title art do not ship as assets
  (spec-dev's optional list).

## Regenerate

```powershell
python tools\assets\extract.py          # assets + manifest.json
python tools\assets\prefab_dump.py       # prefabs.json (UnityPy in-world prefab hierarchy)
python tools\assets\contact_sheets.py    # contact_sheets/*.png
python tools\assets\coverage.py          # coverage.json
python tools\assets\capture_reference.py # reference frames (hidden game; bridge /screenshot)
python tools\assets\check_ui_coverage.py --strict   # spec-dev UI expectation check
```

Licensing/provenance: see `LICENSE-ASSETS.txt`. No game files are modified; the game install and AppData
are only read.

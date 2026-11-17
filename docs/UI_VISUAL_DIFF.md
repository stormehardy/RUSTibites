# UI visual-parity A/B harness

`tools/ui/visual_diff.py` compares the viewer's exact-session UI render with the game reference PNGs captured
under `assets/game/reference/functional_ui/`. It never launches the game.

> **Scope:** this harness measures the **visual axis only**. Visual parity and full UI functionality are one
> acceptance target: pair results with [`UI_FUNCTIONAL_PARITY.md`](UI_FUNCTIONAL_PARITY.md) §1.19 and the
> functional coverage/inert-control status; pixel metrics alone do not establish functional parity.

> **Reference-quality caveat (2026-11-03):** the existing `functional_ui/*_1920x1080.png` refs were
> **upscaled from a 320x240 hidden-game window**, so they are **not pixel-authoritative**. Treat IoU / MAE /
> exact-% as layout/state proxies only; **do not claim pixel parity against these refs**. Native-1920 refs
> reveal **menu-only soft-focus/blur + TMP SDF text** (in-world HUD is crisp). Native `default/`,
> `default_save/` and the native probe are a separate, pixel-authoritative family for captured states.
>
> **Native layout evidence (`3eb2a0f`/`fc2b2ac`):** native **60-session** 1920 `/d/ui` atlas
> (`assets/game/ui_runtime_native.json.gz`) + **213/213** mapped sprites + `ui_geometry.json`; title
> v2 verified title panel **[1374.7,132.5,1870,848.6]**, GenesPanel **[10,104.4,457.5,1000]**; every active
> node has native `screen_bbox`/`world_corners`. Old `ui_runtime.json.gz` (320×240) is historical only. Bulk
> native recapture is complete; a layout-group-free composer can consume the v2 atlas/UI-on-black refs.
>
> **Rendering progress:** TMP font inventory landed (`85e08ea`; 6 fonts/6 atlases under
> `assets/game/ui_fonts/`), and native 1920 in-world UI-only refs landed (`4c0a493`: toolbar/placer/
> dynamic/lineage/inspector/settings). View has started SDF text; **tune against those native in-world refs
> first**, then menu blur, then per-session mask/text diff. Native atlas now covers all 60 sessions; corrected per-tab inspector refs have distinct hashes/alpha, and the HUD toolbar ref matches `default_world`.
> **Menu UI-only isolation remains blocked:** Title Screen/BackGround exclusion is opaque;
> screenBlocker/fullscreen suppression is alpha0; menu refs are native **UI-on-black diagnostics** and a
> shared native **menu background-only** frame (`menu_background/`, frozen `showOff` VideoClip → RenderTexture
> `Video` 1920x1080) can be used for compositing. Use UI-on-black for layout/state, not transparent UI-only alpha.

## Canonical run (viewer)

```powershell
python tools/ui/visual_diff.py `
  --reference-dir assets/game/reference/functional_ui_native/menu_ui `
  --reference-map docs/UI_VISUAL_REFERENCE_MAP.harness.json `
  --viewer target/release/rustibites-view.exe `
  --world testdata/parity/default/rust_seed_600.zip `
  --out-dir target/ui_visual_diff `
  --min-iou 0.98 --max-mae 3 --min-exact-pct 90
```

For each session the harness invokes:

```text
rustibites-view [--world PATH] [--viewer-args ...] --ui-session NAME --ui-only \
  --screenshot <out-dir>/renders/NAME.png --width 1920 --height 1080
```

`--viewer-args` is inserted before the UI flags (e.g. `--assets assets/game --runtime-ui`). The viewer's
exact-session CLI (`--ui-session`) is supplied by view-dev; until it lands, use one of the offline modes below.


## Native references and rescale-blur flagging

The original `assets/game/reference/functional_ui/*_1920x1080.png` files came from a **320x240 hidden-window
capture upscaled to 1920x1080**. They are historical references, **not pixel-authoritative**. Pass the
native-1920 recapture root so it is tried before the legacy directory:

```powershell
python tools/ui/visual_diff.py `
  --native-reference-dir assets/game/reference/functional_ui_native `
  --reference-dir assets/game/reference/functional_ui_native/menu_ui `
  --viewer target/release/rustibites-view.exe `
  --world testdata/parity/default/rust_seed_600.zip `
  --sessions-file tools/ui/results/visual_parity/sessions.json `
  --out-dir target/ui_visual_diff `
  --min-iou 0.98 --max-mae 3 --min-exact-pct 90
```

Per-session rows now carry `native_reference`, `pixel_authoritative`, `ref_sharpness`, and
`rescale_blur_suspect`. Legacy refs are always non-authoritative; a native ref also gets
`rescale_blur_suspect=true` when its grayscale edge sharpness is below `--ref-sharpness-min` (default `1.0`).
The leaderboard shows a `ref` column (`native`/`legacy`/`suspect`), and the aggregate reports
`legacy_references`/`rescale_blur_suspects`. Use `--require-native-references` to make the harness fail while
legacy or blur-suspect references remain; use the default advisory mode while native recaptures are landing.

### Native reference map schema

`--reference-map` accepts either a flat `{runtime_session: path}` object or the shared generated form:

```json
{
  "schema": "rustibites.ui.visual_reference_map.v1",
  "sessions": {
    "main_world_toolbar": "assets/game/reference/functional_ui_native/in_world/native_world_toolbar.png"
  }
}
```

The runner prefers parity's canonical
`assets/game/reference/functional_ui_native/in_world/session_map.json` (13 native aliases; unknown aliases are
filtered to the 60 runtime sessions), falls back to `in_world/manifest.json`, and accepts
`--native-reference-map` to override. `main_world_tool_placer_pellets` is intentionally dropped because the
runtime atlas has no such session. Menu sessions remain legacy/upscaled until
native menu UI-only isolation lands, so they stay flagged non-authoritative.

## UI-on-black menu references

Parity's native menu captures under
`assets/game/reference/functional_ui_native/menu_ui/` are opaque UI-on-black frames. Compare them by
flattening the viewer's transparent UI-only render onto black first:

```powershell
python tools/ui/visual_diff.py `
  --only main_menu_title --only main_menu_settings `
  --native-reference-dir assets/game/reference/functional_ui_native `
  --reference-map <menu-session-map.json> `
  --render-background black --diagnostics ...
```

Because both images are then fully opaque, `alpha_iou` is 1.0 and is not a useful gate for these frames; use
`rgb_mae_opaque`, `exact_pixel_pct`, bbox offsets, and the diagnostics/sharpness ratio instead. These menu
refs are **first-pass UI-on-black verification**; bulk native menu node recapture and the canonical menu
session map are still pending, so do not publish a native menu pixel gate yet.

## Native 60-session UI-on-black mode

Parity's bulk native runtime manifest
(`assets/game/reference/functional_ui_native/menu_ui/native_runtime_manifest.json`) maps all 60 runtime
sessions to native 1920 UI-on-black PNGs. Use the runner's one-flag mode:

```powershell
python tools/ui/run_visual_parity.py --native-ui-on-black --diagnostics `
  --viewer-args "--ui-text sdf" `
  --results-dir tools/ui/results/visual_parity_native_ui_on_black
```

It auto-maps all 60 refs and sets `--render-background black`. Flattening makes both alpha masks opaque, so
`alpha_iou` is not a gate; use RGB MAE, exact-pixel %, bboxes, and diagnostics. This is a native-atlas
diagnostic baseline, not a release pixel gate until layout/sprites/SDF alignment is complete. The canonical
transparent in-world `session_map.json` mode remains available for alpha-mask A/B.

## Full-frame menu background composition

Parity's frozen menu background (`functional_ui_native/menu_background/`) can be composited by the viewer for
full-frame A/B instead of UI-on-black:

```powershell
python tools/ui/visual_diff.py `
  --only main_menu_title `
  --viewer-compose `
  --viewer-args "--ui-text sdf --ui-background assets/game/reference/functional_ui_native/menu_background/menu_background_title_1920x1080.png" `
  --reference-map <title-frozen-map.json> ...
```

`--viewer-compose` omits `--ui-only` so the viewer emits the composited frame. Use the frozen full title
reference (`menu_background/frozen_full/title_full_frozen_1920x1080.png`); settings full composite is not
available yet (press-state capture issue), so settings remains UI-only + background. These are diagnostic
until native menu recapture is finalized.

## Font/text diagnostics

`--diagnostics` adds per-session `ref_diagnostics`, `render_diagnostics`, and `sharpness_ratio`. Each
diagnostic includes alpha-pixel count/bbox, unique opaque colors, edge-pixel count/density, and mean
grayscale edge sharpness. The ratio separates a blurry reference (legacy upscale or menu post-process) from
viewer text weight/SDF differences; it is recorded alongside the alpha-IoU/RGB metrics, not used as a
threshold by itself. Native menu references still carry the game's TMP SDF text, so expect a small
`sharpness_ratio` delta even with correct glyph placement.

## 60-session deterministic matrix

`tools/ui/run_visual_parity.py` wraps this runner for bench/CI:

```powershell
cargo build --release -p bibites-view
python tools/ui/run_visual_parity.py --native-reference-dir assets/game/reference/functional_ui_native
python tools/ui/run_visual_parity.py --strict --require-native-references  # CI gate once native refs exist
```

It reads the 60 session names from `tools/ui/viewer_control_ids.json`, renders all sessions through
`--ui-session ... --ui-only` at 1920x1080, invokes `visual_diff.py` with fixed thresholds, and writes the
compact summary to `tools/ui/results/visual_parity/` (`sessions.json`, `results.json`, `results.csv`,
`leaderboard.md`, `manifest.json`). Full renders/diffs stay under `target/ui_visual_diff` (gitignored).
`--repeat 2` verifies byte-identical renders; `--only`/`--limit` are available for focused runs.

## Offline / CI modes

- `--renders-dir DIR` — skip the viewer and compare `DIR/<session>.png` (or
  `DIR/<session>_overlay_1920x1080.png`). This is the deterministic mode for bench-dev's 60-session matrix.
- `--dry-run` — write the plan and empty-result report without rendering.
- `--self-test` — synthetic identity/shift metric check (no files required).
- `--only NAME` (repeatable), `--glob REGEX`, `--limit N`, `--sessions-file sessions.json`.

## Session and reference mapping

1. `--sessions-file` JSON array (or `sessions` key), else `--only`; otherwise all runtime UI sessions plus
   all `*_overlay_1920x1080.png` under the reference dir.
2. Reference per session, in order: `--reference-map` entry (`docs/UI_VISUAL_REFERENCE_MAP.harness.json` is the ready map), `<session>_overlay_1920x1080.png`,
   `<session>_full_1920x1080.png`, `<session>_1920x1080.png`.
3. Missing references are listed under `missing_references` and skipped unless `--allow-missing`.

## Metrics (per session)

- `alpha_iou` — IoU of alpha>0 masks.
- `rgb_mae_opaque` — mean absolute RGB error over pixels opaque in **both** images (falls back to the union
  if there is no jointly opaque pixel).
- `exact_pixel_pct` — % of jointly opaque pixels with exactly equal RGB.
- `bbox_ref`/`bbox_render` — alpha bounding boxes `[x0,y0,x1,y1]`; `bbox_offset` gives render−reference
  `dx0,dy0,dx1,dy1,dwidth,dheight`.
- `alpha_pixels_ref`/`alpha_pixels_render`, `opaque_compared_pixels`.
- `diffs/<session>.png` — deterministic heatmap: amplified RGB delta, red pixels for alpha-mask mismatch.

Thresholds (`--min-iou`, `--max-mae`, `--min-exact-pct`) are optional; a session that misses one gets
`threshold_fail` and the process exits 1. Render failures are recorded per session (not fatal to the rest).

## Outputs

- `results.json` — schema `rustibites.ui.visual_diff.v1`: config, per-session metrics, aggregate, missing refs.
- `results.csv` — one row per session with the same fields (dict/list fields JSON-encoded).
- `leaderboard.md` — worst-first by `alpha_iou` then RGB MAE, with aggregate summary.
- `diffs/*.png` — heatmaps.
- `renders/*.png` — rendered or copied inputs.

Deterministic: sessions are sorted, no timestamps are written, and heatmaps are fixed-size RGB PNGs.

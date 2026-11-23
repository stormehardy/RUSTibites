# RUSTibites viewer (alpha)

A windowed 2D viewer for the headless engine: load a real save/scenario/genome,
watch the world tick, pan/zoom, pause/step, change speed and inspect bibites.
The viewer is a thin client on `bibites-core` + `bibites-io` and **never perturbs
the simulation RNG**, so watching a world is bit-identical to running it
headless with the same seed.

*(Screenshots are not part of the public repository: they show the game's art.)*

## Status (2026-10-30, alpha)

Working: loading saves/scenarios/genomes (and a generated fallback world),
fixed-timestep simulation, pan/zoom/fit, pause/step, 1×–8× speed, click-to-select
with a tabbed bibite info panel (Stats/Genes/Biology/Brain tabs, clickable tab
strip; `--tab` sets the initial tab), HUD (time/TPS/population/species/FPS),
layer toggles (G/Z/P/O/H), F12 screenshots in the window, and headless PNG
screenshots (pure software rasteriser, no display needed).

The renderer is **game-faithful when the extracted art is present**: the viewer
auto-detects `assets/game/` (sprite sheets, fonts, materials, UI), draws layered
bibite sprites, and falls back to the built-in procedural capsules when the
folder is absent/invalid or `--no-assets` is passed.

Known alpha limitations:

- **Tail bend is implemented in the software path (task-148).** The game bends the Body sprite with a
  shader (`ProceduralSpriteController` drives `_MiddleSegmentAngle`/`_TailAngle` on
  `ProceduralBibiteMaterial`); there is no Tail mesh/mesh renderer. The viewer simulates the deterministic
  trailing-spring angles (`tail_mesh.rs`) and warps Body rows in the software renderer. Headless closeup:
  tail-region alpha 3,128 px vs target ~3,081; entity bbox 450×586 vs game 438×586 (height exact; width
  +12 px from the tail silhouette). The macroquad window path still has a `TODO(tail)` GPU mesh, and
  `_TailAngle` sweep fitting remains; details below. The tail springs are only advanced by a manual Step
  (`App::step_tails`), not by playback (`App::advance`), so playback and `--frames N` draw the tails as they were
  at load.
- **SDF/TMP text:** TTF glyphs are still rasterised by fontdue, but every
  `UiDraw::Text` run gets the game TMP-like ~1 px outline as five offset copies
  (HUD **0.9890 alpha IoU overall**; playback speed-label **0.9680**, above the
  0.95 target; ruler 0.9154; top-right 0.9977; tools 1.000). Remaining delta is
  speed-label thickness/TMP SDF raster quality; a true SDF-atlas glyph pass
  remains future work.
- **Runtime-UI: HUD and inspector tabs are exact.** The `default_world` save-state
  HUD, expanded Stats and Genes/Biology/Brain inspector sessions are parsed/A-B'd
  (IoU 0.9920 / 0.9887 / 0.9815 / 0.9877); in-window clicks route through the
  runtime UI hits (pause/play, tabs, quick stats), and selecting a bibite renders
  the exact session for the active tab, falling back to the curated overlay when
  a session is absent. Keyboard controls work. The title/main menu uses the exact
  runtime layout from `main_menu_title`/`main_menu_settings`; menu background art
  was not captured, so a dark backdrop sits behind the exact node geometry.
  Still missing: full load/save/settings/scenario/editor screens.
- **The window recolours the bibites on the GPU (default; `--no-gpu-hue-shader` turns it
  off):** the skin / eye / egg sprites go through the game's skin shader maths
  (`skin.rs`). Without the GPU material (the flag, or a shader the driver did not take) the
  window draws each sprite from a copy of its cell that the same maths has recoloured on
  the processor (`Painting::Recoloured`, one copy per cell and tint, kept): before 2026-11-14
  the blue art was only multiplied with the colour, so every bibite looked blue and a bibite
  with all three pigments at 1 black all over (the GPU program itself also drew that one black
  until 2026-11-16: its `rgb2hsv` divided by an epsilon that a card loses for a colour of exactly
  (0, 0, 0); the shader has no epsilon now). `--screenshot` uses the software rasteriser,
  which has the same maths. Checked in the real window on 2026-11-05 (see `PARITY_REMAINING.md`, "Bibite
  colours"); headless tests cover the maths, the flag and byte-identical flag-on/off
  screenshots.
- **Level of detail when zoomed out (2026-11-16).** How much of a thing is drawn follows how big it is on the
  screen. A bibite whose body is under 2.5 px long is ONE dot (its body sprite's mean texel through the skin maths,
  with the reaper's darkening and the hurt flash; `sprites::bibite_dot_colour`), under 6 px it is its body sprite
  alone (no mouth, eyes, arms or exoskeleton, and none of the software-picture patterns), above that all its parts;
  the selected bibite is always in full. A pellet whose sprite would be under 2 px across, and a pheromone or blood
  particle under 1.5 px, is a dot too. Dots live in `Scene::dots` (20 bytes each) and are referred to by
  `Primitive::Dots { first, count }` runs, so a world with a hundred thousand pellets no longer writes (sorts,
  clones for the simulation thread) a hundred-byte sprite primitive for each; both renderers draw a dot as a square
  of a whole number of output pixels, one at the least, on the pixel its middle is in. The microscope shade (the
  black outside 1.5 x the simulation's size, visible from any "whole world in view" zoom on) is a GPU material like
  the zones' (`GpuZoneMaterial`; the software picture, a quarter of the window's size and a new texture every frame,
  stays as the fallback): it was the per-frame cost that made zooming out slow. Thresholds: `render::BIBITE_DOT_PIXELS`
  and its neighbours.
- The ICO window icon is used only when the platform honours it. Controls and
  camera behaviour are still being polished by the viewer owner.

## Run

```powershell
# interactive window, real Ring scenario
cargo run --release -p bibites-view -- testdata\real\Default.zip

# historical fixture (opt-in; not the default baseline): a real save
# historical fixture (opt-in; not the default baseline)
cargo run --release -p bibites-view -- testdata\real\saves\lol.zip

# a genome/template (spawns from that genome)
cargo run --release -p bibites-view -- testdata\real\templates_current\Basic_bibite_0.6.3.1.bb8template

# generated starter world (200 bibites), no game data needed
cargo run --release -p bibites-view
```

The built binary is `target\release\rustibites-view.exe`.

## Command-line options

| Option | Meaning |
|---|---|
| `PATH` | save/scenario `.zip`, or `.bb8`/`.bb8template` genome; omit for a generated world |
| `--seed N` | RNG seed for a generated world (default 12345) |
| `--bibites N` | population of the generated fallback world (default 200) |
| `--threads N` | simulation worker threads, `0` = automatic (default); with more than one the world **times its own ticks**, inline and on the pool at four levels of eagerness (how many of the tick's stages are worth a hand-off to the pool), keeps the fastest and looks at the others now and then (where the pool pays, and how eagerly, depends on the machine); a world under about 300 bibites is not timed and ticks on one thread; the startup `threads:` line and the `pool:` lines of `logs/last_run.log` say what it found (`RUSTIBITES_GATES=fixed` puts the switch at about 1,200 bibites with no measuring, `always` puts every stage of every world on the pool, which is slower for worlds under about 1,300 bibites on a machine with twelve threads); `1` = the serial tick |
| `--sim-thread` | (window; also `RUSTIBITES_SIM_THREAD=1`) tick the world on a thread of its own while you only watch the camera, so the window keeps its frame rate when a tick is long; anything that needs the world (a click, a panel, a selection, the pause...) takes it back first. Off by default; see `docs/ARCHITECTURE.md` |
| `--width N` / `--height N` | window/screenshot size (default 1280×800) |
| `--fit bibites\|world` | initial camera: bibite-bbox view (default; 500-unit max extent, densest-cluster anchor) or whole simulation square |
| `--center X,Y` | initial camera centre in world units |
| `--zoom N` | initial zoom in screen pixels per world unit |
| `--ortho-size N` | Unity orthographic size (half visible world height) for reference-frame matching |
| `--assets DIR` | game asset root (default `assets/game`) |
| `--no-assets` | force the built-in procedural renderer |
| `--compare-ref PNG` | draw a side-by-side comparison against a game frame |
| `--select-first` | select the first living bibite after loading |
| `--center-on-selected` | centre the camera on the selected bibite after sim |
| `--tab stats\|genes\|biology\|brain` | initial info-panel tab |
| `--click X,Y` | headless UI verification: synthesize a click at logical pixel (X,Y) through the real hit-testing path (screenshot mode) |
| `--only-layer NAME` | debug: render only one bibite layer (`body`, `mouth`, `eyes`, `arms`, `right-arm`, `left-arm`, `exoskeleton`); also suppresses the HUD in the capture |
| `--dump-layer-bbox` | debug: print each sprite layer's atlas rect/pivot/flip/rotation/size/anchor and analytic screen-space alpha bbox for the selected/first bibite |
| `--runtime-ui` | force the committed exact game-UI layout (default: on when the runtime dump is present and themed UI is enabled) |
| `--no-runtime-ui` | keep the curated HUD layout even when the runtime dump is available |
| `--ui-only` | render only the UI draw list into a transparent PNG (`--screenshot` HUD A/B; requires themed UI) |
| `--gpu-hue-shader` / `--no-gpu-hue-shader` | recolour the skin / eye / egg sprites in the **window** with the GPU skin shader (**default on**; off recolours them on the processor, same colours, slower with many bibites; `--screenshot` always uses the software rasteriser, same maths) |
| `--menu` / `--no-menu` | force or bypass the game-style title menu; with no PATH an interactive window shows the menu by default (`--no-menu` keeps the old generated-world behaviour) |
| `--saves-dir DIR` | directory scanned by **Continue** / **Load Game** (default `saves`, then `testdata/real/saves`) |
| `--simple-hud` | compact HUD (time/TPS/population only) |
| `--screenshot PATH` | render one frame **headlessly** to PNG and exit |
| `--frames N` | frames simulated before the screenshot (default 60) |
| `-h`, `--help` | usage |

The GPU material is the game's skin shader (`skin.rs`, read from the game's compiled pixel
program): hue = texel hue - 0.55 + colour hue, saturation = art saturation × colour saturation,
value = art value × (1 - art saturation × (1 - shade)) with `shade` from the colour's saturation and
value and the shader globals `VminAtS0` / `VmaxAtS0` (0 / 0.75). The tint is the vertex colour and the
globals are one uniform set per frame, so sprites still batch per atlas. The software rasteriser has
the same maths and is what `--screenshot` uses; the flag only affects the interactive window.
Checked in a real window on 2026-11-05 (grey levels and eye colours of the game's capture to one
level); headless tests cover the maths (a Rust port of the GLSL against the software path), the
flag and byte-identical flag-on/off screenshots.

### Title/main menu

An interactive window opened with no PATH (the desktop shortcut) shows a game-style title menu
before any world is loaded. It uses the exact game menu geometry/labels/sprites from the committed
`main_menu_title` runtime session (`Continue`, `New`, `Load`, `Bibite Library`, `Settings`,
`Achievements`, `Additional Tools`, `Credits`, `Quit`) and the game fonts; a dark backdrop stands in
for the menu background art that the hidden-lab capture could not expose. Functional rows are
**Continue (latest save)**, **New Game (Default.zip)**, **Load Game** (save list), **Settings**
(minimum overlay with Grid/Pellets/Pheromones toggles), and **Quit**; arrow keys/Enter/Esc and mouse
clicks navigate (mouse hits use the runtime node rects). A chosen world shows a loading panel
(software-rendered like the menu) before entering the normal viewer. `--menu --screenshot PATH`
renders the menu deterministically without a window or a loaded world; a plain `--screenshot PATH`
still captures the world directly. Release builds use the Windows GUI subsystem so the desktop
shortcut does not flash a console (headless scripts should launch with `Start-Process -Wait`).

The title screen carries one line the game does not have: "RUSTibites Faithful" ("RUSTibites Improved" on
the `improved` branch), a white title card in the top left corner (`menu/tagline.rs`: Boston Black SDF in the version
label's Bold style, 66 px at 1080p, scaled with the UI). It is drawn over the captured frame
and takes no clicks; it is the port's own branding and has no counterpart in the game's title screen.

Desktop shortcut target: `target\release\rustibites-view.exe` with **no arguments**, working
directory `Z:\RUSTibites`; the shortcut's icon is `assets/icon/rustibites.ico`, the window's icon is read at run time from `assets/icon/rustibites_256.png`, and the exe file itself carries the same icon as a resource (`crates/bibites-view/build.rs` compiles `assets/icon/rustibites_view.rc` with the Windows SDK's `rc.exe`; without the SDK the build still succeeds, with a warning, and the exe has the default icon). (`--no-menu` restores the old
argument-less generated-world behaviour; the CLI remains a separate optional shortcut.)

Screenshot examples:

```powershell
target\release\rustibites-view.exe testdata\real\Default.zip `
    --screenshot target\viewer-ring.png --frames 90 --fit bibites
target\release\rustibites-view.exe --screenshot target\viewer-starter.png --fit world
```

Screenshots use a software rasteriser, so they work on machines without a
display/GPU (CI-friendly) and are deterministic for a given world/seed.

HUD-only A/B against the committed game save-state overlay (the `--click`
pauses the viewer at 1920×1080 so it matches the reference's paused t=20 s
state; `--ui-only` keeps the transparent alpha channel):

```powershell
target\release\rustibites-view.exe testdata\parity\default\rust_seed_600.zip `
    --screenshot target\runtime-hud-overlay.png --ui-only `
    --width 1920 --height 1080 --frames 0 --center 0,0 --ortho-size 2080 `
    --click 739,34
```

### Runtime UI layout (`--runtime-ui`)

When `tools/ui/ui_runtime_atlas.json.gz` is present (committed, from the hidden
Default-world capture), the viewer can render the game's **exact UI hierarchy**
instead of the curated HUD. The joined dump carries the RectTransform tree plus
resolved Image/TMP text/CanvasScaler/layout/control fields; `ui_runtime.rs`
composes the local RectTransforms down the hierarchy into the same `UiFrame`
draw list the curated HUD uses, so the macroquad window and headless
screenshots share the same pixels.

- The gzip dump is decompressed once into `target/ui_runtime_cache/` (keyed by
  source size + mtime); `default_world` (save-state HUD), the expanded Stats
  session and the Genes/Biology/Brain inspector sessions are parsed (menu
  sessions are used by the title screen). Other legacy captures are ignored.
- ScreenSpaceCamera canvases are approximated as camera-centred at the captured
  1920×1080 reference resolution. The Default world has no Screen Space Overlay
  canvases: five canvases are ScreenSpaceCamera bound to `UICamera`. The capture
  records the UI camera as orthographic size 540 at 1920×1080, i.e. exactly
  1 canvas unit = 1 px with the camera at (960, 540), so the screen-centred
  composition reproduces that projection; the composer body stays swappable via
  `RectSemantics` for save-based captures (which can place the camera
  differently).
- Active nodes are drawn with sibling z-order; the known live labels
  (time/playback counter and speed label) are replaced with viewer values.
  Multi-line TMP labels are split into one draw per line (the software text
  path ignores embedded `\n`). Runtime button rects (`PauseButton`/`PlayButton`,
  quick-stats) feed the normal UI interaction path. Selecting a bibite renders
  the session for the active tab (Stats/Genes/Biology/Brain) instead of the
  curated panel; missing sessions fall back to the curated overlay.
- The runtime dump's joined sprite ids are not always trustworthy: the
  authoritative PNG path is the sprite's `manifest.file`, so the loader decodes
  any referenced PNGs the UI atlas skipped (18 across the two sessions,
  including circular pause/play buttons, slider track/knob and top-right tool
  icons). Without this, those chrome pieces render blank/wrong.
- The HUD ruler is procedural in the game (`RulerHandle` script/mesh), so the
  dump has no bracket sprites: the viewer composes the bracket from the Ruler
  node rects, the measured value from
  `assets/game/reference/default_save/ui_save_capture.json` (`ruler.value`,
  `200u`) and the live camera scale. The default HUD shows `200` above the
  bracket plus the two-bar `u` on the line; the zoomed-in inspector HUD uses
  the combined `200u` label instead. Text uses offset-copy outline emulation
  for the game's SDF-outline alpha coverage.
- RectTransform composition follows Unity's anchor-reference rule
  (`anchor_min + (anchor_max-anchor_min)*pivot`), not the anchor-centre; the
  earlier centre approximation shifted stretched, pivot-(0,1) panels
  (`Layer1`/`Right Panels`) off-screen by half the screen, which hid the
  top-right `Info`/quick-stats button. Both cases are pinned by unit tests.
- `--ui-only` renders the same draw list into a transparent RGBA PNG, which is
  the HUD A/B input. Against the committed save-state game overlay
  (`assets/game/reference/default_save/hud_savestate_overlay_1920x1080.png`,
  save `rust_seed_600.zip` at t=20 s, paused, 1920×1080, UI scale 1):
  **alpha-mask IoU 0.9890** overall, 0.9977 for the top-right panel/button
  block, 1.000 for the bottom-left tool buttons, **0.9680 for the playback
  chrome** (speed-label advances aligned), and **0.9154 for the ruler region**
  (procedural bracket + measured `200u` value). The runtime overlay is
  deterministic (two runs hash-identical).
- Inspector A/B: expanded Stats **0.9920**; captured Genes/Biology/Brain
  overlays score **0.9887 / 0.9815 / 0.9877 alpha IoU** respectively with the
  new runtime sessions (selected bibite, panel geometry and controls match;
  residual is text/icon raster quality). The curated inspector overlay is
  skipped whenever the selected tab has a runtime session.
- **TMP text:** the viewer rasterises fontdue glyphs and emulates the TMP ~1 px
  outline with five offset copies; glyph positions and all panel/button rects
  match. Residual differences are true SDF-atlas raster quality (especially
  the small monospace labels), not layout.
- **Remaining release-rendering deltas (known):** the gameplay HUD is still
  software-rasterised with TTF glyphs; the biggest remaining delta is
  speed-label glyph **thickness/anti-aliasing** (playback subregion alpha IoU
  0.7935 even though the character starts now match; overall playback IoU
  0.9680). Genes/Biology/Brain inspector overlays are 0.9815-0.9887, with the
  residual in small TMP text/icons rather than panel geometry. The tail is a
  software per-row affine approximation (3,128 vs 3,081 px; width +12) and the
  macroquad window path still lacks a GPU tail mesh. The title menu uses the
  exact runtime layout but a dark backdrop where the hidden-lab capture exposed
  no menu background art. The window's GPU skin shader was checked in a real
  window on 2026-11-05.
- The curated fallback HUD has alpha-IoU 0.0 against the game save-state layout
  (as expected), and `--simple-hud` screenshots remain deterministic.
- Fallbacks: absent/unreadable dump → curated HUD; `--simple-hud` → built-in
  text HUD; `--no-runtime-ui` → curated HUD even when the dump exists.
- **Click-area ids.** Every click area carries a `UiId` (a bare `u32`) and
  `UiFrame::interact` answers a click with *every* action registered under the
  id of the topmost area under the pointer (`action_at`, the pressed look and
  the slider presses go by id as well), so two panels that can be on screen
  together must not share one: a click on the Save Bibite dialog once also
  closed the Information panel behind it. Each panel owns a block (the list is
  in the `UiId` doc, `ui.rs`; the lists that grow with the world's content —
  lineage shapes, neurons, tags — start far apart). A panel that hangs over
  another asserts `UiFrame::shared_ids()` (test-only) is empty in a test that
  draws both and clicks through `interact`
  (`app::sprite_renderer_tests::a_click_on_the_save_dialog_fires_nothing_of_…`).
- Fresh-scenario spawn semantics match the CLI/game: `Default.zip` starts with
  **1 bibite** and the fresh spawner grows it to the scenario minimum (10) over
  ~9 s (verified headless: pop 1 at 0.25 s, pop 10 at 600 ticks).

## Controls

| Input | Action |
|---|---|
| drag | pan camera |
| mouse wheel | zoom |
| `F` | fit world |
| `B` | fit bibites |
| `R` | reset view (fit bibites) |
| `space` | pause / resume |
| `1`–`8` | simulation speed 1×–8× |
| right arrow | single simulation tick (when paused) |
| click | select the nearest bibite |
| click a tab (Stats/Genes/Biology/Brain) | switch the info-panel tab |
| `G` / `Z` / `P` / `O` / `H` | toggle grid / zones / pellets / pheromones / HUD |
| `F12` | save a timestamped PNG under `target/` (window mode) |
| `Esc` | deselect, then quit |

The HUD shows simulated time, TPS, population/species counts and FPS; the
selected-bibite panel shows energy/health/age, a gene summary and the
Stats/Genes/Biology/Brain tab content. The themed HUD is hit-tested before
camera/selection controls in both window and headless `--click` paths; a click
that presses/releases on a UI widget applies its action and never pans/zooms/
selects the world, and wheel-over-UI is consumed by the UI. Keyboard controls
and `--tab` are unaffected.

**Wheel zoom (task-139/151, commit `8104046`):** follows the game camera rule
`ratio = wheelDelta/10 * scrollSpeed`, ortho size `size *= 1 - ratio` clamped to
`[5, max(250, 2*simulationSize)]`; the viewer smooths the target over ~0.12 s
and keeps the cursor anchor fixed. `scrollSpeed = 1.0` is confirmed from the live
`CameraManager` dump (`assets/game/reference/camera/live_camera_controls.json`).
Both arm layers now match the game bboxes (headless `--only-layer`):
right arm 142x106 at (759,238)-(900,343) vs game (759,238)-(899,342); left arm
89x150 at (495,460)-(583,609) vs game (495,460)-(582,608). The left arm needed
a body-local anchor correction of (-3.002,-0.016) d1 derived from clean
isolated-layer captures (the extracted prefab transform lacks it); pellet scale
and arm pivot were fixed in task-139. **2026-11-14:** that correction was only
right for the normal tier. It was the arm's thickness (3 units there) that the
pivot convention of an unflipped sprite cost the Left Arm, so large bibites had
a left arm hovering off the body and huge ones one lying on it; the game has no
such offset (its renderer bounds are the prefab's transforms, the pivot and
Unity's flips) and the constant is gone (`sprites::drawn_pivot`, see
[`PARITY_REMAINING.md`](PARITY_REMAINING.md)).
Control coverage (every captured button/control path, hit/action status,
inert controls and the recommended generic action registry) is audited in
[`UI_CONTROL_AUDIT.md`](UI_CONTROL_AUDIT.md); the full game UI-surface inventory
(and what is missing vs inert) is [`UI_FUNCTIONAL_PARITY.md`](UI_FUNCTIONAL_PARITY.md).

## Game art assets and licensing

No picture or font of the game is in the repository. `tools/release/extract_art.py` builds them from your own copy of the game into `assets/game/`
(ignored by git), so the viewer can match the game's look. It is optional: without them (or with `--no-assets`) the viewer uses its procedural
renderer. **Licensing:** the RUSTibites code is MIT/Apache-2.0; the metadata extracted from the game that the repository holds (layouts, sprite
rectangles, parameter tables: `assets/game/*.json`, `*.gz`, `*.csv`) remains the property of The Bibites' original creator. The project is not
affiliated with or endorsed by the original creator. See `assets/game/README.md` and `assets/game/LICENSE-ASSETS.txt`.

## Screenshot gallery

Not part of the public repository (the screenshots show the game's art). The release checks are in [`VIEWER_RELEASE_CHECKLIST.md`](VIEWER_RELEASE_CHECKLIST.md).

## Assets and launch helper

- `assets/viewer/rustibites-icon.png` — 256×256 icon (copy of the project icon).
- `assets/viewer/rustibites.ico` — Windows icon (copy of the project icon).
- Both copies and `assets/icon/` are written by `scripts/make_project_icon.py --png <256 px picture>` (the icon since 2026-11-17 is a hand-edited picture; without `--png` the script re-tints the game's icon instead).
- `tools/viewer/launch.ps1` — builds if needed, launches the viewer with good
  defaults, and in screenshot mode verifies the produced PNG is non-trivial and
  has the expected dimensions:

  ```powershell
  pwsh -File tools\viewer\launch.ps1                       # Headless screenshot + verify
  pwsh -File tools\viewer\launch.ps1 -Interactive          # Open the window
  pwsh -File tools\viewer\launch.ps1 -InputPath testdata\real\Default.zip
  ```

## Troubleshooting

| Symptom | Fix |
|---|---|
| Window does not open / black window | Update GPU drivers; try `--width 960 --height 600`; on very old GPUs use the software screenshot mode to confirm the world loads |
| HUD/controls feel tiny on a HiDPI display | The viewer uses logical pixels; use the OS DPI setting or a larger `--width/--height` |
| `--screenshot` fails to write | Use a writable path (e.g. `target\viewer.png`); the folder must exist or be creatable |
| Screenshot has no text | Expected: the software rasteriser draws geometry only; HUD text appears in the interactive window |
| No display on a server/CI | Use `--screenshot` (headless) or run the engine CLI |
| `file not found` | `testdata/` is gitignored; copy a real save/scenario/template there, or omit the path to get a generated world |

## Roadmap

- Minimap and whole-world density overlay.
- Charts: population/biomass/species over time.
- Full load/save/settings/scenario/editor screens and a file picker (title menu + loading panel
  already exist; exact runtime menu art background is still missing).
- Finish the macroquad Tail GPU mesh; the software tail warp is calibrated (3,128 vs 3,081 px) but a real `_TailAngle` deformation capture is still needed (the 55-frame sweep exposed no live props).
- Sprite artwork and richer organ visualisation.
- Integrate into the main CLI (`rustibites view <path>`) once the alpha stabilises.


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

**Runtime-named sprite cells fall back to the nearest non-empty sheet cell (task-138).**
`GameAssets::sprite_alpha_nonzero` counts atlas pixels with alpha > 8 for a `SpriteRef`; the
`eye_sprite` / `arm_sprite` selectors first prefer the live-captured normal-tier cells (eyes col 2 /
row 1; arms col 1 / row 0), then the formula cell, then candidate cells ordered by Manhattan distance.
The first cell with non-empty alpha is returned, so a runtime sprite name without a manifest entry
cannot leave a fully transparent quad. On the canonical closeup (`rust_seed_600.zip`, camera
(-5.360003, 831.0031), ortho size 12) the formula cell already equals the live captured rect
(eyes `[16,9,8,9]`, arms `[9,0,9,3]`), so the eye/arm cells themselves are unchanged by the fallback.

**Pellet scale + arm pivot parity (task-139).** Two independent closeup mismatches were fixed:
1. **Pellet scale:** `pellet_sprite` used `sqrt(radius²/20)`; the game's authored pellet radius is
   `2.5·sqrt(amount/20)` at scale 1 (`NormalAmountAtScale1 = 20`), so the scale is now
   `sqrt(amount/20)` (`pellet_render_scale`). The reference pellet at world `(-6.628, 837.456)`,
   amount ≈31.12 u², now measures **188×187 px** (game 188×187 px; 23.3k vs 23.5k non-zone pixels),
   versus 105×105 px before.
2. **Arm pivot:** the extraction manifest stores SpriteRenderer pivots normalised from the top-left,
   matching the renderer, so `sheet_pivot_v1` no longer applies `1 - y`; `legacy_pivot` arms use
   `(1, 0)` (live `normal_arms_10` pivot (9,0) px). The previously misplaced right-arm component now
   matches the game's separate arm at (830–899, 269–342) px.

With pellet scale + arm pivot fixed our **full-frame** closeup bbox is **436×530 px**; the game's
full-frame blob (including the same plant pellet) is 438×586.

**Task-141 consistent-target comparison (the full-frame target was wrong).** `closeup_single/` now
provides an atomically consistent reference: selected runtime id **−1920841852**, exact camera/ortho,
genome, and a **visible-alpha bbox of 441×485 px** at `[459,177,900,662]` measured from the
selected-entity-moved diff. That bbox **excludes the static plant pellet** (the old 438×586 blob
included it). Matching the metric on our side (render `rust_seed_600.zip` with pellets removed, same
camera/ortho, headless):
- our entity-only union: **438×424 px** (`[461,181,899,605]`), i.e. width is 3 px short and height is
  **61 px short at the bottom**, not 45 px taller. The apparent extra height in `436×530` is the
  static pellet (top y=74), which the target metric excludes.
- arm components match the game exactly: RightArm `[830,269,899,342]`, Left Arm `[513,535,582,608]`;
  Body/Mouth layer world AABBs from the consistent capture match our rendered layer AABBs; the live
  dump has exactly six layers (no omitted tail/part), and extracted `normal_body` alpha is 65/99,
  identical to live `normal_body_00`.
- The remaining game-only extent is the bottom of the body composite (`y≈662` vs our `y≈605`); it is
  not explained by a different cell/variant, pivot, omitted layer, or the spawner bibite. With the
  now-consistent id/genome this is documented as an **extracted-art/live-renderer limitation**; we do
  not apply a global scale/offset hack. Closing the last ~61 px would need per-layer visible-alpha
  bounds for Body/Mouth that the live capture does not expose (it records texture-region counts and
  world bounds, not per-layer alpha pixel bboxes).

**Task-143 Body-layer isolation (conclusive extracted-art limitation).** The atomic per-layer capture
`closeup_single/layers/Body_2_alpha.png` gives the game Body layer a single connected visible-alpha
bbox of **`(521,261,897,662)` = 376×401 px, 77,307 px**. Our extracted `normal_body` art is a
half-disc with a left point (65/99 alpha pixels, matching the live texture-region count); rendering
the body layer alone with each of the three extracted body rows (0/1/2) produces
`x521–882/885, y261–605` — the top/left match exactly, but the game art has ~57 px more
**body/tail silhouette at the bottom-right** that no extracted body cell contains. Checks: Body
sprite rect/texture (`normal_body_00`, 11×9) and transform/bounds match the atomic capture; arm,
eye, mouth components match; all six layers are present (no extra renderer/tail child); the pellet is
outside this layer and already pixel-exact. This is therefore a genuine **extracted-art vs live
Body-sprite variant** difference for this genome/load, not a crop/scale/pivot/cell-selection bug.
We stop here rather than substitute a different body tier or apply a global stretch; reproducing the
last 57 px needs the exact live Body texture variant (`Body_2.png`/`Body_2_alpha.png` are kept as
reference) added to the extraction pipeline.

**Task-145 reconciliation: the extra pixels are the Body-sprite tail bend, not Body art/pivot.**
task-144 proved the live `normal_body` texture/cell is pixel-identical to our extracted art, and our
Body quad prints exactly `rect=[0,18,11,9]`, pivot `(1.0,0.5)`, world size `11×9`, `local_scale=1.0`,
`d1=1.1496`, rotation 129.98°, matching the live SpriteRenderer rect/pivot/PPU/scale. An alpha-mask
pixel diff between our Body-only render and `Body_2_alpha.png` gives:
- ours **74,262 px** bbox `(521,261,882,605)`; game **77,307 px** bbox `(521,261,897,662)`
- intersection 74,226, IoU **0.9597**; **game-only 3,081 px** in `(734,606,896,661)`; ours-only 36 px.
So our render is essentially the game body **minus a bottom-right tail-shaped extension**, not a
different cell/scale/pivot.

**Mechanism (confirmed by code + asset scan):** the prefab's depth-1 `Tail` node is a transform (its
`Collider2D` is used by `UserControl` for selection bounds) with **no SpriteRenderer and no mesh**.
A scan of `Here/**.cs` finds no `MeshRenderer`/`MeshFilter`/`SkinnedMeshRenderer`/`TailRenderer` for
the bibite, and `prefabs.json` has no Tail mesh filter/renderer or tail texture. The visible tail is a
**Body-sprite shader deformation**: `ProceduralSpriteController` (on the Body SpriteRenderer) sets
`_MiddleSegmentAngle`/`_TailAngle` (plus `_ArmAngles`/`_MouthOpenning`/`_Size`) on
`ProceduralBibiteMaterial`; segment/tail distances are `4.5·d1`/`7.5·d1`, max angles 7.5°/25°, with
dt spring/relaxation. At task-145 time the viewer drew Body as a rigid quad and did not apply that warp,
so the remaining 57 px was a **missing tail-bend deformation**, not missing Body art or a mesh-extraction
bug; task-148 has since implemented the software row warp (see the implementation note below).

**Implementation (task-148):** the software renderer now warps Body rows below `bend_row = 0.55` with
the per-row `TailState` angles and a stretched tail region (headless closeup: tail-region alpha 3,128 px
vs target ~3,081; entity bbox 450×586 vs game 438×586, height exact, width +12). The macroquad window
still has `TODO(tail)` for the GPU sprite mesh; the software rasteriser is authoritative. Remaining work
is `_TailAngle`/`_MiddleSegmentAngle` sweep fitting and the GPU path; no new art assets are needed.
Evidence: `assets/game/tail/` + `tail_sweep/sweep.json`, `tail_mesh.rs` tests, and the closeup
`body-alpha-diff.png`; details in `docs/ASSETS.md` (parity-owned).

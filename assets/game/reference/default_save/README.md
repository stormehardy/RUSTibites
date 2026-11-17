# Reference in-world frames (The Bibites 0.6.3.1 / Unity 6000.0.44f1)

Captured with **no visible window**: the game ran through the BibitesAPI lab harness with `window="hidden"`; the bridge renders the main camera into an offscreen RenderTexture (`GET /screenshot`) and returns a PNG. No BepInEx plugin was installed for these captures.

- input: `Z:\RUSTibites\testdata\parity\default\rust_seed_600.zip` (save_load)
- world at capture: simulatedTime = 20.000 s, paused = True, counts = {"bibites": 11, "corpses": 0, "eggs": 0}
- captured (UTC): 2026-10-30T12:16:04+00:00

| frame | size | camera x, y | orthographic size | note |
|---|---|---|---|---|
| [![full_world.png](full_world.png)](full_world.png) | 1920x1080 | 0.0, 0.0 | 2080.0 | fit=true whole world (1920x1080) |
| [![population.png](population.png)](population.png) | 1920x1080 | 0.0, 0.0 | 120.0 | planted zone at orthoSize 120 (mixed population, pellets, zone ring) |
| [![zone_pellets_eggs.png](zone_pellets_eggs.png)](zone_pellets_eggs.png) | 1920x1080 | 0.0, 0.0 | 230.0 | zone + pellets + eggs at orthoSize 230 |
| [![bibite_closeup.png](bibite_closeup.png)](bibite_closeup.png) | 1280x720 | -5.360003, 831.0031 | 12.0 | one living bibite at orthoSize 12 (sprite layers/tint; camera on its position) |

The world camera is orthographic; `orthographic size` is half the visible height in world units. To reproduce a frame, load the same save (read-only from `testdata/real/saves/`) and call `POST /camera {{x, y, size}}` (or `fit: true`) before `GET /screenshot?w=&h=`.


## UI-inclusive HUD frames (1920x1080, same save/state)

Captured with the transient ParityDump plugin: hidden game loads `rust_seed_600.zip`, paused at save time
20 s; the bridge renders the main-camera world, the plugin renders the ScreenSpaceCamera UI to a
transparent UICamera-projected RenderTexture, and PIL composites them. Files:
- `hud_default_1920x1080.png` — full frame, default in-world HUD (playback chrome top-centre, top-right
  time/buttons, bottom-left tool buttons, ruler), no selection.
- `hud_overlay_1920x1080.png` — **UI-only alpha overlay** (same HUD, transparent world) for HUD-only A/B.
- `hud_inspector_1920x1080.png` — selected-bibite frame (Stats panel top bar: OpenData/OpenGenes/Exit
  tabs visible; panel body not expanded in this paused capture).
- `hud_inspector_overlay_1920x1080.png` — UI-only alpha overlay of the inspector state.
Metadata/capture state/selected id/panel plus the counts note are in `hud_capture.json` and
`reference.json` (`hud_frames`, `hud_counts_note`).

**Counts discrepancy:** the save contains 10 bibites; `/load` in the game triggers the fresh spawner
(virginSpawnRate=1 in the save) which adds one virgin bibite (11 total) at a random id/position. The
viewer loading the save correctly sees 10; this is game-load spawner behaviour, not a viewer bug. Use
`rust_seed_600_nospawn.zip` for scene pixel diffs or exclude the added runtime id.


## Save-state runtime-UI dump + optimized HUD overlays (task-121)

`assets/game/ui_runtime.json.gz` now publishes save-state sessions (merged into the normal atlas):
- `default_world` — save loaded, paused t=20 s, fit camera, 1920×1080, UI scale 1.0; ruler HUD state
  (`200u` displayed), Info/quick-stats button and all default HUD chrome active. 7,874 nodes.
- `default_world_inspector` — one bibite selected via the game's real `StatsButton`, panel animation
  settled while paused, Stats body expanded (Species/State/Stats panel + Kill/Lay/Save/Edit buttons).
  8,025 nodes; selected entity/camera/panel recorded in `ui_save_capture.json`.
- `default_world_newgame` — previous t=0.25 newgame dump, kept for reference.
The t=20 s runtime dump has no Ruler Image sprites/text: `RulerHandle` draws the bracket procedurally
(script/mesh), so the dump records the Ruler node rects but not bracket sprites; measured value is `200u`
in the frames/overlay. HUD frames:
`hud_savestate_1920x1080.png` + `hud_savestate_overlay_1920x1080.png` (UI-only alpha) and
`hud_inspector_expanded_1920x1080.png` + `hud_inspector_expanded_overlay_1920x1080.png`.


## Per-SpriteRenderer bibite layers (task-135)

`bibite_layers.json` (schema `rustibites.assets.bibite_layers.v1`) captures every live bibite's renderer
stack in the save state (`rust_seed_600.zip`, paused t=20 s, hidden lab). **Selected id: `2076547059`**
(nearest the canonical closeup camera). Also `varied_sample: true` on two additional ids for arm/tail
comparisons. Each bibite has `body_position/rotation/scale` and `layers[]`; each layer has:
`path`, `name`, `sorting_order`, `sorting_layer`, `enabled`, `color`, `flip_x/y`, `draw_mode`,
`world_position/rotation`, `lossy_scale`, `local_position/rotation/scale`, `sprite`, `sprite_rect`,
`sprite_pivot`, `pixels_per_unit`, `bounds_center/size` (SpriteRenderer world AABB), `world_corners`
(sprite-rect corners transformed to world, pivot-aware), plus composited `composited_bbox`
`{x0,y0,x1,y1,w,h,cx,cy}` union of all layer bounds.

Reference closeup camera: 1280×720, orthographic size **12**, centre
`(-5.360003, 831.0031)`, so **30 px per world unit**:
`x_px = 640 + (x - cam.x)*30`; `y_px_up = 360 + (y - cam.y)*30`; image y (top-left origin)
`= 720 - y_px_up`. The selected bbox in pixels is precomputed in `selected_bbox_pixels`; use the same
formula for individual layer corners. Note: the game load of this save adds the virgin spawner bibite, so
11 live bibites are present; the save itself has 10 (as documented in the HUD counts note).


Task-137 addendum: each layer now also carries live `sprite_texture`, `sprite_texture_size`,
`sprite_region_nontransparent` / `sprite_region_pixels` (read from the live `Texture2D` at the sprite rect).
The selected bibite's Exoskeleton layer (`normal_exoskeleton_00`) is **genuinely transparent** (0/144 live
pixels; sheet cell 0,0 is blank in both crop orientations), so the visible body plates come from another
layer (Body), not a missing exoskeleton cell. See `docs/ASSETS.md` for the full sprite-cell audit.

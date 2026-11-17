# Reference in-world frames (historical lol set)

> Canonical Default.zip frames are in `default/`. — The Bibites 0.6.3.1 (Unity 6000.0.44f1)

Captured **without opening a visible game window**. The game ran through the BibitesAPI lab harness
with `window="hidden"`; BibitesBridge renders `Camera.main` into an offscreen RenderTexture and returns
PNG (`GET /screenshot`). **No BepInEx plugin was installed** for these captures; the hidden game process
was started and stopped by `tools/assets/capture_reference.py`, and no game/API process is left running.

Source world: `testdata/real/saves/lol.zip` (read-only), loaded via `POST /load`, then paused.
No RNG seed is applied on a save load (`seed: null`): the reference is the exact visual state of the
game after loading that save. World state at capture:
`simulatedTime = 469983.78125 s` (base tick 0 for this load), `sim_tps = 15`,
counts `{22 living bibites, 13 corpses, 4 eggs}`, paused = true.

| frame | size (px) | camera centre (x, y) | orthographic size (half-height, world units) | what it shows |
|---|---|---|---|---|
| [full_world.png](full_world.png) | 1920×1080 | (0.0, 0.0) | 2080.0 | `fit=true`: whole 4000×4000 world, zones/pellets/bibites |
| [population.png](population.png) | 1920×1080 | (0.0, 0.0) | 120.0 | the planted zone with the mixed population and pellet field |
| [zone_pellets_eggs.png](zone_pellets_eggs.png) | 1920×1080 | (0.0, 0.0) | 230.0 | zone ring, plant/meat pellets, eggs and corpses at close range |
| [bibite_closeup.png](bibite_closeup.png) | 1280×720 | (67.68129, −97.96953) | 12.0 | a single living bibite at high zoom: body/mouth/eyes/arms/exoskeleton layers and tint |

`reference.json` records the same values programmatically:
`frames[].camera.{x,y,size}`, `frames[].width/height`, `world.simulated_time_s`, `world.counts`,
`world.sim_tps`, `save`, and the capture method. To reproduce a frame: load the same save read-only,
`POST /camera {"x":…,"y":…,"size":…}` (or `{"fit":true}`), then `GET /screenshot?w=…&h=…`. The camera is
orthographic; "orthographic size" is half the visible height in world units.

The old user AppData screenshots (`bibite_*.png`) were not used or copied; these frames are the
authoritative reference for the viewer's visual comparison. Reference frames are game art too: they are
covered by `../LICENSE-ASSETS.txt`, not MIT.

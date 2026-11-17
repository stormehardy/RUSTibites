# Reference in-world frames (The Bibites 0.6.3.1 / Unity 6000.0.44f1)

Captured with **no visible window**: the game ran through the BibitesAPI lab harness with `window="hidden"`; the bridge renders the main camera into an offscreen RenderTexture (`GET /screenshot`) and returns a PNG. No BepInEx plugin was installed for these captures.

- input: `Z:\RUSTibites\testdata\real\Default.zip` (scenario_newgame)
- world at capture: simulatedTime = 0.250 s, paused = True, counts = {"bibites": 1, "corpses": 0, "eggs": 0}
- captured (UTC): 2026-10-30T12:00:56+00:00

| frame | size | camera x, y | orthographic size | note |
|---|---|---|---|---|
| [![full_world.png](full_world.png)](full_world.png) | 1920x1080 | 0.0, 0.0 | 2080.0 | fit=true whole world (1920x1080) |
| [![population.png](population.png)](population.png) | 1920x1080 | 0.0, 0.0 | 120.0 | planted zone at orthoSize 120 (mixed population, pellets, zone ring) |
| [![zone_pellets_eggs.png](zone_pellets_eggs.png)](zone_pellets_eggs.png) | 1920x1080 | 0.0, 0.0 | 230.0 | zone + pellets + eggs at orthoSize 230 |
| [![bibite_closeup.png](bibite_closeup.png)](bibite_closeup.png) | 1280x720 | 1018.39941, -471.2087 | 12.0 | one living bibite at orthoSize 12 (sprite layers/tint; camera on its position) |

The world camera is orthographic; `orthographic size` is half the visible height in world units. To reproduce a frame, start the same Default.zip scenario (`POST /newgame`) and call `POST /camera {{x, y, size}}` (or `fit: true`) before `GET /screenshot?w=&h=`.

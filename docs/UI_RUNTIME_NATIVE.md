# Native 1920 runtime UI atlas + sprite/geometry manifests

Captured by a standalone hidden `The Bibites.exe -screen-width 1920 -screen-height 1080 -window-mode windowed`
(bypasses BibitesAPI's 320x240 hidden launcher), BepInEx bridge/plugin.

- `assets/game/ui_runtime_native.json.gz` + `ui_runtime_native.index.json`: **60 sessions**, all `/d/ui` dumps
  with `screen=1920x1080`, same session names as the old atlas; old `ui_runtime.json.gz` remains historical.
- `assets/game/reference/functional_ui_native/sprite_manifest.json` + `sprites/`: **213/213 referenced
  native Image/RawImage sprite names mapped** to PNGs (UnityPy Sprite/Texture2D extraction; textures/rects).
- `assets/game/reference/functional_ui_native/menu_ui/`: native UI-on-black menu refs for 60 sessions
  (`*_ui_on_black_1920x1080.png`, BackGround/Title Screen hidden) + `native_runtime_manifest.json`.
- `assets/game/reference/functional_ui_native/ui_geometry.json`: active Canvas roots screen bboxes,
  renderMode/worldCamera/sorting, RectTransform size/pivot/anchors/localScale/position/world corners, and
  CanvasScaler mode/reference/scale (ConstantPixelSize 800×600 scale1 on menu roots; LoadingPanel
  ScreenSpaceOverlay 1920×1080 bbox).
- Screenshot geometry: title menu panel bbox ≈ [1366,64,1877,900]; old 320 atlas rendered the same panel at
  x≈860–1230. Native node raw ButtonsPanel rect recorded in `menu_ui/manifest.json`.
- Caveat: `ui_geometry.title_keys` is empty in this pass (key-rect enumeration miss), but root Canvas data and
  screenshot bbox are present; the bulk node dumps are complete.


## v2: final composed screen rects per active node

`assets/game/ui_runtime_native.json.gz` is now schema **`rustibites.assets.ui_runtime.v2`**: every
`active_in_hierarchy` node carries `screen_bbox` (`x0,y0,x1,y1,w,h,cx,cy` in native 1920 screen pixels) and
`world_corners`, computed from the live RectTransform + Canvas worldCamera. The composer can place nodes/sprites
from these authoritative rects and bypass Unity layout groups.

Verified examples: `UI/WelcomeScreen/Main Holder/ButtonsPanel` = **[1374.7, 132.5, 1870.0, 848.6]** (matches the
native title screenshot ~1366–1877); `default_world_genes` `GenesPanel` = **[10.0, 104.4, 457.5, 1000.0]**.
All 60 sessions were recaptured; index schema v2; old 320 atlas remains historical.

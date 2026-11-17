# Toolbar / top-right button theme inventory

**Artifact:** `assets/game/reference/functional_ui_native/in_world/button_theme/button_theme.json`
**Schema:** `rustibites.ui.button_theme.v1`
**Source:** standalone hidden native-1920 game `/d/button_theme`
**Raw:** `button_theme_raw.json`; **sprites:** `.../button_theme/sprites/`

## Coverage

The inventory covers **7 active toolbar/top-right buttons**:

`DynamicParametersButton`, `InformationPanelButton`, `UserActions`, `SettingsButton`,
`AncestralLineageButton`, `ViewButton`, `PlacerButton`.

For each button it records:

- `Button.colors` for **normal / highlighted / pressed / selected / disabled** (+ `colorMultiplier`,
  `fadeDuration`);
- `targetGraphic`/`target_type`;
- `Image.color`, sprite, texture, sprite rect (35x35) and pivot;
- material/shader/material colour;
- `CanvasRenderer` colour;
- ancestor `CanvasGroup` alpha/interactable/raycast state.

All seven use `Image` + `base_icon` (35x35, pivot 17.5/17.5), `Default UI Material` / `UI/Default`, and
alpha 1 inside the current canvas.

## Theme values are neutral

The captured tint fields are **neutral white** (normal `1,1,1,1`; highlighted/selected `0.9608`;
pressed/disabled `0.7843`; disabled alpha `0.50196`; fade `0.1`). The **native dark toolbar look is not
produced by these Button/Image material tints**; do not darken buttons by inventing theme colours. It must
come from the rendered icon/sprite content, the native **menu background** (frozen blurred `showOff` VideoClip
→ RenderTexture `Video` 1920x1080), or another composition/shader layer not covered by `Button.colors`.

## Missing extracted sprites dumped

These active icon sprites were missing from the extracted UI assets and are now dumped at 35x35:

| Sprite | File | Source / path id |
|---|---|---|
| `base_icon` | `sprites/base_icon.png` | sharedassets0 / 67 |
| `energy_icon` | `sprites/energy_icon.png` | sharedassets1 / 34 |
| `rename_icon` | `sprites/rename_icon.png` | sharedassets1 / 36 |
| `feedback_icon` | `sprites/feedback_icon.png` | sharedassets1 / 69 |
| `simulation_icon` | `sprites/simulation_icon.png` | sharedassets1 / 71 |
| `settings_icon` | `sprites/settings_icon.png` | sharedassets1 / 72 |
| `stats_icon` | `sprites/stats_icon.png` | sharedassets2 / 24 |
| `luca_import_tool` | `sprites/luca_import_tool.png` | sharedassets2 / 47 |

## Use in visual work

- Load the neutral theme fields for button tint/state animation; load `base_icon` and the other dumped sprites
  as icon textures.
- Compare button masks/layout against the native in-world UI-only refs (toolbar/top-right) before judging colour.
- **Do not claim the dark native look is explained by `Button.colors`**; that is an open rendering-composition
  issue. The old upscaled UI refs are not pixel-authoritative.

Machine-readable summary is in
[`UI_VISUAL_REFERENCE_MAP.json`](UI_VISUAL_REFERENCE_MAP.json) under `button_theme`.

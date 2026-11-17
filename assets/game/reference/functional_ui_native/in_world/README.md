# Native 1920 in-world UI-only refs

Standalone hidden native-1920 run; loaded `rust_seed_600.zip` paused; `/d/ui_overlay`.

| session | alpha fraction | unique colors | file |
|---|---|---|---|
| native_world_toolbar | 0.021 | 363 | assets/game/reference/functional_ui_native/in_world/native_world_toolbar.png |
| native_world_placer | 0.025 | 367 | assets/game/reference/functional_ui_native/in_world/native_world_placer.png |
| native_world_user_actions | 0.027 | 368 | assets/game/reference/functional_ui_native/in_world/native_world_user_actions.png |
| native_world_view | 0.057 | 368 | assets/game/reference/functional_ui_native/in_world/native_world_view.png |
| native_world_dynamic | 0.523 | 431 | assets/game/reference/functional_ui_native/in_world/native_world_dynamic.png |
| native_world_lineage | 0.103 | 468 | assets/game/reference/functional_ui_native/in_world/native_world_lineage.png |
| native_world_information | 1.000 | 412 | assets/game/reference/functional_ui_native/in_world/native_world_information.png |
| native_world_settings | 1.000 | 412 | assets/game/reference/functional_ui_native/in_world/native_world_settings.png |
| native_world_placer_pellets | 1.000 | 412 | assets/game/reference/functional_ui_native/in_world/native_world_placer_pellets.png |
| native_inspector_stats | 1.000 | 412 | assets/game/reference/functional_ui_native/in_world/native_inspector_stats.png |
| native_inspector_genes | 1.000 | 412 | assets/game/reference/functional_ui_native/in_world/native_inspector_genes.png |
| native_inspector_biology | 1.000 | 412 | assets/game/reference/functional_ui_native/in_world/native_inspector_biology.png |
| native_inspector_brain | 1.000 | 412 | assets/game/reference/functional_ui_native/in_world/native_inspector_brain.png |

Toolbar/placer/dynamic/lineage refs are transparent/partial; Stats/Genes/Biology/Brain/Information/Settings include the game's full-panel backdrop (alpha 1), not world background. Menu challenge/workshop/settings UI-only remains blocked by the Title Screen/BackGround composition.


## Button theme / missing toolbar sprites

`button_theme/button_theme.json` dumps the native toolbar/top-right theme data read-only:
Button.colors (normal/highlighted/pressed/selected/disabled + multiplier/fade), targetGraphic path,
Image.color, sprite name/rect/pivot, texture name/size, material/shader/color, CanvasRenderer color, and
ancestor CanvasGroup alpha. All observed tints are neutral white (normalColor/Image.color/material all 1,1,1,1;
CanvasGroup alpha 1), so the native dark button appearance is not a runtime color tint in these fields.

Active toolbar sprites **all missing from extracted assets**: `base_icon`, `settings_icon`, `stats_icon`,
`feedback_icon`, `simulation_icon`, `energy_icon`, `luca_import_tool`, `rename_icon`. Their native texture PNGs
are dumped under `button_theme/sprites/` (35×35 each, source asset/path IDs recorded in the JSON).


## Correct per-tab inspector refs (supersedes old native_inspector_*)

`inspector/native_inspector_{stats,genes,biology,brain}_ui_only_1920x1080.png` are native 1920 UI-only captures
with the StatsPanel/GenesPanel/BiologyPanel/BrainPanel activated directly by path after selecting a bibite.
They have distinct hashes/alpha coverage (stats 136k, genes 218k, biology 91k, brain 117k). The old
`native_inspector_*` files showed the same Stats page and were removed. `native_world_toolbar_hud_1920x1080.png`
captures the inspector-closed HUD/toolbar state matching `default_world`.

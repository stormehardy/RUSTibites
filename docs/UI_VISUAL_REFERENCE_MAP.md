# UI visual reference map (runtime sessions and in-world captures)

**Generated:** 2026-11-03  
**Runtime atlas:** native 1920 atlas, 60 sessions (`assets/game/ui_runtime_native.index.json`); old `assets/game/ui_runtime.json.gz` is historical only.  
**Reference roots:** `assets/game/reference/functional_ui_native/{menu_ui,in_world,button_theme,sprites}/`, `assets/game/reference/functional_ui/`, `default_save/`, `default/`, `main_menu/`

**Purpose:** bind every captured runtime session and in-world frame to game reference PNG(s), with dimensions/alpha/background/state, and record native-layout evidence + remaining limits. This is docs/harness contract; it does not render/compare itself.

> **Functional-parity companion:** visual parity and full UI functionality are one acceptance target. This
> map/pixel-reference work does **not** establish functional parity; the functional coverage/inert-control
> inventory is in [`UI_FUNCTIONAL_PARITY.md`](UI_FUNCTIONAL_PARITY.md) and the conjoint requirement is §1.19.
> Report visual metrics and functional status together; neither alone closes the goal.

> **Reference-quality status:** old `functional_ui/*_1920x1080.png` refs and old `ui_runtime.json.gz` are **historical**: upscaled 320x240 / 320x240 layout, not pixel-authoritative. Native 1920 atlas v2 covers all 60 sessions with per-active-node screen_bbox/world_corners; UI-on-black menu refs are opaque diagnostics, in-world refs transparent/partial/full-panel. **Menu transparent UI-only isolation remains blocked.** Do not claim pixel parity against old refs.

## Coverage

- **Sessions mapped:** 60
- **Usable reference:** 60
- **Partial:** 0
- **Missing usable reference:** 0
- **Machine-readable map:** [`UI_VISUAL_REFERENCE_MAP.json`](UI_VISUAL_REFERENCE_MAP.json)
- **Harness-compatible simple map:** [`UI_VISUAL_REFERENCE_MAP.harness.json`](UI_VISUAL_REFERENCE_MAP.harness.json) (`session -> primary reference`)

## Reference formats and backgrounds

| Reference family | Dimensions | Background / state |
|---|---|---|
| `functional_ui_native/menu_ui/*_ui_on_black_1920x1080.png` | 1920x1080, opaque | Native menu UI-on-black diagnostic refs (alpha 1, black background); use for layout/state, **not** transparent UI-only alpha. |
| `functional_ui_native/in_world/*_native_1920x1080.png` | 1920x1080, transparent/partial/full | Native in-world UI-only refs; toolbar/placer/dynamic/lineage are partial alpha; some panels include full-panel backdrop alpha 1. |
| `functional_ui_native/probe_*` | 1920x1080 | Native title/settings full frames (opaque) and world UI-only probe. |
| `functional_ui_native/button_theme/sprites/*` | 35x35 | Dumped toolbar/top-right icon sprites; neutral theme tints do not explain dark look. |
| old `functional_ui/*_1920x1080.png` | 1920x1080 upscaled | **Historical, not pixel-authoritative**; layout/state fallback only. |
| `default_save/hud_*` / `default/*` / `main_menu/*` | 1920x1080 (closeups 1280x720) | Native legacy/default refs; `main_menu` loading/settings are identical loading-screen captures, not title/settings. |

## Session map

| Runtime session | Status | Primary native reference | Refs | Role/alpha | Captured state / note |
|---|---|---|---|---|---|
| `main_menu` | usable | [main_menu_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/main_menu_ui_on_black_1920x1080.png) 1920x1080 | 3 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [25, 76, 1869, 1054]) |
| `default_world_newgame` | usable | [default_world_newgame_native_1920x1080.png](../assets/game/reference/functional_ui_native/in_world/default_world_newgame_native_1920x1080.png) 1920x1080 | 3 | native-in-world/transparent | native in-world UI-only (alpha fraction 0.021, nonblack bbox [10, 10, 1909, 1069]) |
| `default_world` | usable | [default_world_native_1920x1080.png](../assets/game/reference/functional_ui_native/in_world/default_world_native_1920x1080.png) 1920x1080 | 3 | native-in-world/transparent | native in-world UI-only (alpha fraction 0.021, nonblack bbox [10, 10, 1909, 1069]) |
| `default_world_inspector` | usable | [native_inspector_stats_ui_only_1920x1080.png](../assets/game/reference/functional_ui_native/in_world/inspector/native_inspector_stats_ui_only_1920x1080.png) 1920x1080 | 6 | native-inspector-stats/transparent | native stats inspector UI-only (alpha fraction 0.145, 465 colors) |
| `main_menu_title` | usable | [main_menu_title_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/main_menu_title_ui_on_black_1920x1080.png) 1920x1080 | 3 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [25, 76, 1869, 1054]) |
| `main_menu_settings` | usable | [main_menu_settings_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/main_menu_settings_ui_on_black_1920x1080.png) 1920x1080 | 4 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `default_world_genes` | usable | [native_inspector_genes_ui_only_1920x1080.png](../assets/game/reference/functional_ui_native/in_world/inspector/native_inspector_genes_ui_only_1920x1080.png) 1920x1080 | 5 | native-inspector-genes/transparent | native genes inspector UI-only (alpha fraction 0.220, 665 colors) |
| `default_world_biology` | usable | [native_inspector_biology_ui_only_1920x1080.png](../assets/game/reference/functional_ui_native/in_world/inspector/native_inspector_biology_ui_only_1920x1080.png) 1920x1080 | 5 | native-inspector-biology/transparent | native biology inspector UI-only (alpha fraction 0.121, 959 colors) |
| `default_world_brain` | usable | [native_inspector_brain_ui_only_1920x1080.png](../assets/game/reference/functional_ui_native/in_world/inspector/native_inspector_brain_ui_only_1920x1080.png) 1920x1080 | 5 | native-inspector-brain/transparent | native brain inspector UI-only (alpha fraction 0.194, 687 colors) |
| `main_menu_functional_title` | usable | [main_menu_functional_title_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/main_menu_functional_title_ui_on_black_1920x1080.png) 1920x1080 | 3 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [25, 76, 1869, 1054]) |
| `main_menu_load_game` | usable | [main_menu_load_game_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/main_menu_load_game_ui_on_black_1920x1080.png) 1920x1080 | 3 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `main_menu_new_game` | usable | [main_menu_new_game_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/main_menu_new_game_ui_on_black_1920x1080.png) 1920x1080 | 6 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `main_menu_bibite_library` | usable | [main_menu_bibite_library_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/main_menu_bibite_library_ui_on_black_1920x1080.png) 1920x1080 | 4 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `main_menu_achievements` | usable | [main_menu_achievements_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/main_menu_achievements_ui_on_black_1920x1080.png) 1920x1080 | 3 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1076]) |
| `main_menu_additional_tools` | usable | [main_menu_additional_tools_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/main_menu_additional_tools_ui_on_black_1920x1080.png) 1920x1080 | 3 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [25, 10, 1909, 1069]) |
| `main_menu_credits` | usable | [main_menu_credits_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/main_menu_credits_ui_on_black_1920x1080.png) 1920x1080 | 3 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [25, 10, 1909, 1069]) |
| `main_menu_new_panel` | usable | [main_menu_new_panel_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/main_menu_new_panel_ui_on_black_1920x1080.png) 1920x1080 | 3 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `main_menu_workshop` | usable | [main_menu_workshop_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/main_menu_workshop_ui_on_black_1920x1080.png) 1920x1080 | 4 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `main_menu_scenario_editor` | usable | [main_menu_scenario_editor_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/main_menu_scenario_editor_ui_on_black_1920x1080.png) 1920x1080 | 7 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `main_menu_settings_graphics` | usable | [main_menu_settings_graphics_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/main_menu_settings_graphics_ui_on_black_1920x1080.png) 1920x1080 | 5 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `main_menu_settings_gameplay` | usable | [main_menu_settings_gameplay_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/main_menu_settings_gameplay_ui_on_black_1920x1080.png) 1920x1080 | 4 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `main_menu_settings_hotkeys` | usable | [main_menu_settings_hotkeys_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/main_menu_settings_hotkeys_ui_on_black_1920x1080.png) 1920x1080 | 4 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `main_menu_bibite_editor_genes` | usable | [main_menu_bibite_editor_genes_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/main_menu_bibite_editor_genes_ui_on_black_1920x1080.png) 1920x1080 | 4 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [25, 76, 1869, 1054]) |
| `main_menu_bibite_editor_brain` | usable | [main_menu_bibite_editor_brain_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/main_menu_bibite_editor_brain_ui_on_black_1920x1080.png) 1920x1080 | 5 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [25, 76, 1869, 1054]) |
| `main_world_toolbar` | usable | [native_world_toolbar_hud_1920x1080.png](../assets/game/reference/functional_ui_native/in_world/native_world_toolbar_hud_1920x1080.png) 1920x1080 | 7 | native-hud/transparent | native HUD/toolbar inspector-closed (alpha fraction 0.029, 421 colors) |
| `main_world_tool_placer` | usable | [native_world_placer.png](../assets/game/reference/functional_ui_native/in_world/native_world_placer.png) 1920x1080 | 8 | native-in-world/transparent | native in-world UI-only native_world_placer (alpha fraction 0.025, 367 colors) |
| `main_world_tool_user_actions` | usable | [native_world_user_actions.png](../assets/game/reference/functional_ui_native/in_world/native_world_user_actions.png) 1920x1080 | 7 | native-in-world/transparent | native in-world UI-only native_world_user_actions (alpha fraction 0.027, 368 colors) |
| `main_world_tool_view` | usable | [native_world_view.png](../assets/game/reference/functional_ui_native/in_world/native_world_view.png) 1920x1080 | 7 | native-in-world/transparent | native in-world UI-only native_world_view (alpha fraction 0.057, 368 colors) |
| `main_world_dynamic_parameters` | usable | [native_world_dynamic.png](../assets/game/reference/functional_ui_native/in_world/native_world_dynamic.png) 1920x1080 | 7 | native-in-world/transparent | native in-world UI-only native_world_dynamic (alpha fraction 0.523, 431 colors) |
| `main_world_lineage` | usable | [native_world_lineage.png](../assets/game/reference/functional_ui_native/in_world/native_world_lineage.png) 1920x1080 | 7 | native-in-world/transparent | native in-world UI-only native_world_lineage (alpha fraction 0.103, 468 colors) |
| `main_world_information` | usable | [native_world_information.png](../assets/game/reference/functional_ui_native/in_world/native_world_information.png) 1920x1080 | 7 | native-in-world/opaque | native in-world UI-only native_world_information (alpha fraction 1.000, 412 colors) |
| `main_world_settings_panel` | usable | [native_world_settings.png](../assets/game/reference/functional_ui_native/in_world/native_world_settings.png) 1920x1080 | 7 | native-in-world/opaque | native in-world UI-only native_world_settings (alpha fraction 1.000, 412 colors) |
| `library_selector_flow` | usable | [library_selector_flow_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/library_selector_flow_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `library_create_new` | usable | [library_create_new_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/library_create_new_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `library_editor_genes_flow` | usable | [library_editor_genes_flow_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/library_editor_genes_flow_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [25, 76, 1869, 1054]) |
| `library_editor_brain_flow` | usable | [library_editor_brain_flow_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/library_editor_brain_flow_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [25, 76, 1869, 1054]) |
| `library_editor_brain_after_add` | usable | [library_editor_brain_after_add_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/library_editor_brain_after_add_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [25, 76, 1869, 1054]) |
| `newgame_scenario_options` | usable | [newgame_scenario_options_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/newgame_scenario_options_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `newgame_new_scenario_button` | usable | [newgame_new_scenario_button_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/newgame_new_scenario_button_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `newgame_new_challenge_button` | usable | [newgame_new_challenge_button_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/newgame_new_challenge_button_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `scenario_editor_flow` | usable | [scenario_editor_flow_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/scenario_editor_flow_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `scenario_editor_zones` | usable | [scenario_editor_zones_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/scenario_editor_zones_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `scenario_editor_ChallengeOptions` | usable | [scenario_editor_ChallengeOptions_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/scenario_editor_ChallengeOptions_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `scenario_editor_sim` | usable | [scenario_editor_sim_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/scenario_editor_sim_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `settings_page_graphics_controls` | usable | [settings_page_graphics_controls_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/settings_page_graphics_controls_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `settings_page_gameplay_controls` | usable | [settings_page_gameplay_controls_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/settings_page_gameplay_controls_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `settings_page_hotkeys_controls` | usable | [settings_page_hotkeys_controls_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/settings_page_hotkeys_controls_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `world_dynamic_parameters_controls` | usable | [world_dynamic_parameters_controls_native_1920x1080.png](../assets/game/reference/functional_ui_native/in_world/world_dynamic_parameters_controls_native_1920x1080.png) 1920x1080 | 2 | native-in-world/transparent | native in-world UI-only (alpha fraction 0.714, nonblack bbox [10, 10, 1919, 1079]) |
| `world_settings_controls` | usable | [world_settings_controls_native_1920x1080.png](../assets/game/reference/functional_ui_native/in_world/world_settings_controls_native_1920x1080.png) 1920x1080 | 2 | native-in-world/transparent | native in-world UI-only (alpha fraction 0.714, nonblack bbox [10, 10, 1919, 1079]) |
| `challenge_selector` | usable | [challenge_selector_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/challenge_selector_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `challenge_scenario_panel` | usable | [challenge_scenario_panel_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/challenge_scenario_panel_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `challenge_champion_selection` | usable | [challenge_champion_selection_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/challenge_champion_selection_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [25, 76, 1869, 1054]) |
| `challenge_parameters_editor` | usable | [challenge_parameters_editor_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/challenge_parameters_editor_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [25, 76, 1869, 1054]) |
| `workshop_panel` | usable | [workshop_panel_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/workshop_panel_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [0, 0, 1919, 1079]) |
| `workshop_edit_shared` | usable | [workshop_edit_shared_ui_on_black_1920x1080.png](../assets/game/reference/functional_ui_native/menu_ui/workshop_edit_shared_ui_on_black_1920x1080.png) 1920x1080 | 2 | native-ui-on-black/opaque | native UI-on-black diagnostic (alpha fraction 1.000, nonblack bbox [25, 76, 1869, 1054]) |
| `lineage_species_panel` | usable | [lineage_species_panel_native_1920x1080.png](../assets/game/reference/functional_ui_native/in_world/lineage_species_panel_native_1920x1080.png) 1920x1080 | 2 | native-in-world/transparent | native in-world UI-only (alpha fraction 0.275, nonblack bbox [10, 10, 1909, 1069]) |
| `lineage_species_tabs` | usable | [lineage_species_tabs_native_1920x1080.png](../assets/game/reference/functional_ui_native/in_world/lineage_species_tabs_native_1920x1080.png) 1920x1080 | 2 | native-in-world/transparent | native in-world UI-only (alpha fraction 0.275, nonblack bbox [10, 10, 1909, 1069]) |
| `lineage_species_info` | usable | [lineage_species_info_native_1920x1080.png](../assets/game/reference/functional_ui_native/in_world/lineage_species_info_native_1920x1080.png) 1920x1080 | 2 | native-in-world/transparent | native in-world UI-only (alpha fraction 0.275, nonblack bbox [10, 10, 1909, 1069]) |
| `lineage_species_genes` | usable | [lineage_species_genes_native_1920x1080.png](../assets/game/reference/functional_ui_native/in_world/lineage_species_genes_native_1920x1080.png) 1920x1080 | 2 | native-in-world/transparent | native in-world UI-only (alpha fraction 0.275, nonblack bbox [10, 10, 1909, 1069]) |
| `lineage_species_brain` | usable | [lineage_species_brain_native_1920x1080.png](../assets/game/reference/functional_ui_native/in_world/lineage_species_brain_native_1920x1080.png) 1920x1080 | 2 | native-in-world/transparent | native in-world UI-only (alpha fraction 0.275, nonblack bbox [10, 10, 1909, 1069]) |

## Native runtime layout (`3eb2a0f` / `fc2b2ac`)

Native 1920 `/d/ui` atlas **v2** + index: 60 sessions, every active node carries native **`screen_bbox` + `world_corners`**; sprite manifest **213/213** mapped; root Canvas semantics `ConstantPixelSize` 800×600/scale1. **Verified v2 rects: title panel [1374.7,132.5,1870,848.6], GenesPanel [10,104.4,457.5,1000].** A layout-group-free composer can consume this atlas. Old atlas/layout historical.

| Native layout session | Nodes | Canvases |
|---|---:|---:|
| `main_menu` | 4358 | 26 |
| `main_menu_title` | 4358 | 26 |
| `main_menu_functional_title` | 4358 | 26 |
| `main_menu_load_game` | 4358 | 26 |
| `main_menu_new_game` | 4358 | 26 |
| `main_menu_bibite_library` | 4358 | 26 |
| `main_menu_achievements` | 4378 | 26 |
| `main_menu_additional_tools` | 4378 | 26 |
| `main_menu_credits` | 4378 | 26 |
| `main_menu_new_panel` | 4378 | 26 |
| `main_menu_workshop` | 4378 | 26 |
| `main_menu_scenario_editor` | 4378 | 26 |
| `main_menu_settings` | 5423 | 60 |
| `main_menu_settings_graphics` | 5423 | 60 |
| `main_menu_settings_gameplay` | 5423 | 60 |
| `main_menu_settings_hotkeys` | 5423 | 60 |
| `settings_page_graphics_controls` | 5423 | 60 |
| `settings_page_gameplay_controls` | 5423 | 60 |
| `settings_page_hotkeys_controls` | 5423 | 60 |
| `main_menu_bibite_editor_genes` | 5423 | 60 |
| `main_menu_bibite_editor_brain` | 5606 | 60 |
| `library_selector_flow` | 6351 | 60 |
| `library_create_new` | 6351 | 60 |
| `library_editor_genes_flow` | 6351 | 60 |
| `library_editor_brain_flow` | 6351 | 60 |
| `library_editor_brain_after_add` | 6351 | 60 |
| `newgame_scenario_options` | 6351 | 60 |
| `newgame_new_scenario_button` | 6351 | 60 |
| `newgame_new_challenge_button` | 6351 | 60 |
| `scenario_editor_flow` | 6351 | 60 |
| `scenario_editor_zones` | 6351 | 60 |
| `scenario_editor_ChallengeOptions` | 6351 | 60 |
| `scenario_editor_sim` | 6351 | 60 |
| `challenge_selector` | 6351 | 60 |
| `challenge_scenario_panel` | 6351 | 60 |
| `challenge_champion_selection` | 6351 | 60 |
| `challenge_parameters_editor` | 6351 | 60 |
| `workshop_panel` | 6351 | 60 |
| `workshop_edit_shared` | 6351 | 60 |
| `default_world_newgame` | 7874 | 41 |
| `default_world` | 7874 | 41 |
| `default_world_inspector` | 7974 | 41 |
| `default_world_genes` | 7974 | 41 |
| `default_world_biology` | 7974 | 41 |
| `default_world_brain` | 7974 | 41 |
| `main_world_toolbar` | 7974 | 41 |
| `main_world_tool_placer` | 7974 | 41 |
| `main_world_tool_user_actions` | 7974 | 41 |
| `main_world_tool_view` | 7974 | 41 |
| `main_world_dynamic_parameters` | 7974 | 41 |
| `main_world_lineage` | 7981 | 41 |
| `main_world_information` | 8259 | 41 |
| `main_world_settings_panel` | 8259 | 41 |
| `world_dynamic_parameters_controls` | 8259 | 41 |
| `world_settings_controls` | 8259 | 41 |
| `lineage_species_panel` | 8266 | 41 |
| `lineage_species_tabs` | 8273 | 41 |
| `lineage_species_info` | 8280 | 41 |
| `lineage_species_genes` | 8287 | 41 |
| `lineage_species_brain` | 8294 | 41 |

Bulk 60-session native recapture: complete; UI-on-black refs are opaque diagnostics, menu transparent UI-only isolation still blocked.

## Native in-world refs (canonical session_map; corrected inspector tabs)

| Native session | Dimensions | Alpha fraction | Unique colors | SHA-256 prefix | Source |
|---|---:|---:|---:|---|---|
| `main_world_toolbar` | 1920x1080 | 0.029 | 421 | 0f90805bf375 | [png](../assets/game/reference/functional_ui_native/in_world/native_world_toolbar_hud_1920x1080.png) |
| `main_world_tool_placer` | 1920x1080 | 0.025 | — | — | [png](../assets/game/reference/functional_ui_native/in_world/native_world_placer.png) |
| `main_world_tool_user_actions` | 1920x1080 | 0.027 | — | — | [png](../assets/game/reference/functional_ui_native/in_world/native_world_user_actions.png) |
| `main_world_tool_view` | 1920x1080 | 0.057 | — | — | [png](../assets/game/reference/functional_ui_native/in_world/native_world_view.png) |
| `main_world_dynamic_parameters` | 1920x1080 | 0.523 | — | — | [png](../assets/game/reference/functional_ui_native/in_world/native_world_dynamic.png) |
| `main_world_lineage` | 1920x1080 | 0.103 | — | — | [png](../assets/game/reference/functional_ui_native/in_world/native_world_lineage.png) |
| `main_world_information` | 1920x1080 | 1.000 | — | — | [png](../assets/game/reference/functional_ui_native/in_world/native_world_information.png) |
| `main_world_settings_panel` | 1920x1080 | 1.000 | — | — | [png](../assets/game/reference/functional_ui_native/in_world/native_world_settings.png) |
| `main_world_tool_placer_pellets` | 1920x1080 | 1.000 | — | — | [png](../assets/game/reference/functional_ui_native/in_world/native_world_placer_pellets.png) |
| `default_world_inspector` | 1920x1080 | 0.145 | 465 | f1cd9fe735f3 | [png](../assets/game/reference/functional_ui_native/in_world/inspector/native_inspector_stats_ui_only_1920x1080.png) |
| `default_world_genes` | 1920x1080 | 0.220 | 665 | 82f8d04ed82a | [png](../assets/game/reference/functional_ui_native/in_world/inspector/native_inspector_genes_ui_only_1920x1080.png) |
| `default_world_biology` | 1920x1080 | 0.121 | 959 | 4561fe70128f | [png](../assets/game/reference/functional_ui_native/in_world/inspector/native_inspector_biology_ui_only_1920x1080.png) |
| `default_world_brain` | 1920x1080 | 0.194 | 687 | ce79f0e6337f | [png](../assets/game/reference/functional_ui_native/in_world/inspector/native_inspector_brain_ui_only_1920x1080.png) |

## Native UI-on-black menu refs

Native menu refs are under `assets/game/reference/functional_ui_native/menu_ui/` (`native_runtime_manifest.json`); all are **opaque UI-on-black** diagnostics (alpha fraction 1.0), not transparent UI-only captures. Menu transparent isolation remains blocked.

## Native menu background (landed `568ba01`)

Shared frozen title/settings background-only frame (`menu_background/`, schema `rustibites.ui.native_menu_background.v1`): 1920x1080, 45,429 colors; UI/Title Screen/screenBlocker hidden, BackGround retained, frozen with timeScale=0 + VideoPlayer.Pause(). Video: `BackGround/Video Player`, clip `showOff` -> RenderTexture `Video` 1920x1080, frame 52/2812. Use for compositing native UI-on-black menu refs; dark menu look is background/composition, not button tints.

## TMP font inventory (landed `85e08ea`)

`assets/game/ui_fonts/fonts_manifest.json` (`rustibites.assets.tmp_fonts.v1`): 6 TMP fonts / 6 atlases, per-font face/glyph/character/kerning/material JSON. View has started SDF text; tune against native in-world UI-only refs first.

| Font | Glyphs | Atlas | Atlas size |
|---|---:|---|---|
| Boston Black SDF | 97 | `atlas_Boston_Black_SDF_Atlas.png` | 512x512 |
| LiberationSans SDF | 250 | `atlas_LiberationSans_SDF_Atlas.png` | 1024x1024 |
| LiberationSans SDF - Fallback | 0 | `atlas_LiberationSans_SDF_Atlas.png` | 1024x1024 |
| Roboto-Bold SDF NoOutline | 112 | `atlas_Roboto-Bold_Atlas.png` | 1024x1024 |
| Roboto-Bold SDF WhiteOutline | 98 | `atlas_Roboto-Bold_SDF_Atlas.png` | 512x512 |
| Roboto-Regular SDF | 98 | `atlas_Roboto-Regular_SDF_Atlas.png` | 512x512 |

## Button theme inventory (landed `ab5d07d`)

`button_theme.json` (`rustibites.ui.button_theme.v1`): 7 toolbar/top-right buttons, neutral white tints; native dark look is **not** explained by tints. 8 missing icon sprites dumped 35×35 under `button_theme/sprites/`. See [`UI_BUTTON_THEME.md`](UI_BUTTON_THEME.md).

## Legacy in-world capture inventory

| Reference | Dimensions | Pixel auth. | Kind | State |
|---|---|---|---|---|
| [full_world.png](../assets/game/reference/default/full_world.png) | 1920x1080, opaque | yes | world | historical lol save; fit whole 4000x4000 world; paused, 22 bibites/13 corpses/4 eggs, sim_tps 15 |
| [population.png](../assets/game/reference/default/population.png) | 1920x1080, opaque | yes | world | historical lol save; population/planted-zone camera (centre 0,0 size 120) |
| [zone_pellets_eggs.png](../assets/game/reference/default/zone_pellets_eggs.png) | 1920x1080, opaque | yes | world | historical lol save; zone ring/pellets/eggs close camera (size 230) |
| [bibite_closeup.png](../assets/game/reference/default/bibite_closeup.png) | 1280x720, opaque | yes | world | historical lol save; single-bibite closeup (1280x720, size 12) |
| [hud_default_1920x1080.png](../assets/game/reference/default_save/hud_default_1920x1080.png) | 1920x1080, opaque | yes | hud | default-save HUD/world baseline |
| [hud_overlay_1920x1080.png](../assets/game/reference/default_save/hud_overlay_1920x1080.png) | 1920x1080, transparent | yes | hud-overlay | transparent HUD overlay, default-save state |
| [hud_savestate_1920x1080.png](../assets/game/reference/default_save/hud_savestate_1920x1080.png) | 1920x1080, opaque | yes | hud-save | default-save saved-state HUD |
| [hud_savestate_overlay_1920x1080.png](../assets/game/reference/default_save/hud_savestate_overlay_1920x1080.png) | 1920x1080, transparent | yes | hud-overlay | transparent HUD overlay, saved state |
| [hud_inspector_1920x1080.png](../assets/game/reference/default_save/hud_inspector_1920x1080.png) | 1920x1080, opaque | yes | hud-inspector | default-save inspector |
| [hud_inspector_overlay_1920x1080.png](../assets/game/reference/default_save/hud_inspector_overlay_1920x1080.png) | 1920x1080, transparent | yes | hud-overlay | transparent inspector overlay |
| [hud_inspector_expanded_1920x1080.png](../assets/game/reference/default_save/hud_inspector_expanded_1920x1080.png) | 1920x1080, opaque | yes | hud-inspector | expanded inspector |
| [hud_inspector_expanded_overlay_1920x1080.png](../assets/game/reference/default_save/hud_inspector_expanded_overlay_1920x1080.png) | 1920x1080, transparent | yes | hud-overlay | transparent expanded-inspector overlay |
| [full_world.png](../assets/game/reference/default_save/full_world.png) | 1920x1080, opaque | yes | world | default-save full-world render |
| [population.png](../assets/game/reference/default_save/population.png) | 1920x1080, opaque | yes | world | default-save population render |
| [zone_pellets_eggs.png](../assets/game/reference/default_save/zone_pellets_eggs.png) | 1920x1080, opaque | yes | world | default-save zone/pellets/eggs render |
| [bibite_closeup.png](../assets/game/reference/default_save/bibite_closeup.png) | 1280x720, opaque | yes | world | default-save bibite closeup (1280x720) |
| [loading_screen_1920x1080.png](../assets/game/reference/main_menu/loading_screen_1920x1080.png) | 1920x1080, opaque | yes | menu-loading | game loading screen; not a title-menu reference |
| [settings_screen_1920x1080.png](../assets/game/reference/main_menu/settings_screen_1920x1080.png) | 1920x1080, opaque | yes | menu-loading | byte-identical to loading_screen; not a settings reference |
| [hotkeys_readonly_flow_overlay_1920x1080.png](../assets/game/reference/functional_ui/hotkeys_readonly_flow_overlay_1920x1080.png) | 1920x1080, opaque | no | flow-overlay | hotkey page read-only state; supplemental for settings_page_hotkeys_controls |
| [workshop_edit_last_overlay_1920x1080.png](../assets/game/reference/functional_ui/workshop_edit_last_overlay_1920x1080.png) | 1920x1080, opaque | no | flow-overlay | workshop edit state after change; supplemental for workshop_edit_shared |

## Wrong-state / unsafe references

Do not use as primary visual truth:

- functional_ui/main_menu_achievements_{full,overlay}: show Bibite Library
- functional_ui/main_menu_additional_tools_{full,overlay}: show Bibite Library
- functional_ui/main_menu_credits_{full,overlay}: show Bibite Library
- functional_ui/main_menu_new_panel_{full,overlay}: show Bibite Library
- functional_ui/main_menu_new_game_{full,overlay}: show Graphics Settings
- functional_ui/main_menu_scenario_editor_{full,overlay}: show Steam Workshop
- functional_ui/main_menu_settings_graphics_{full,overlay}: show Steam Workshop
- functional_ui/main_menu_settings_gameplay_{full,overlay}: show Steam Workshop
- functional_ui/main_menu_settings_hotkeys_{full,overlay}: show Steam Workshop
- functional_ui_native/probe_world_full_native_1920x1080.png: fully transparent/empty capture; unusable
- main_menu/loading_screen and settings_screen are identical loading-screen captures
- functional_ui_native/menu_ui_only_native_1920x1080.png: alpha0/blank menu UI-only attempt
- functional_ui_native/settings_ui_only_native_1920x1080.png: alpha0/blank settings UI-only attempt
- functional_ui_native/probe_world_full_native_1920x1080.png: fully transparent/empty capture

## Native findings and rendering plan

**Findings:** native menu refs show **soft-focus/blur + TMP SDF text**; in-world HUD is crisp. Native geometry differs from old atlas (title panel ~1366..1877 vs ~860..1230).

**Plan:**

1. **TMP font metadata — DONE (`85e08ea`)**: 6 fonts/6 atlases under `assets/game/ui_fonts/`.
2. **SDF text renderer — IN PROGRESS**: tune glyph metrics/outline against native in-world UI-only refs first.
3. **Menu blur/composite pass — PENDING**: composite native UI-on-black over the shared native menu background frame (frozen `showOff` VideoClip RenderTexture); transparent UI-only isolation still blocked.
4. **Per-session mask/text diff — PENDING**: after native layout/schema verification; no pixel parity against old refs.

## Parity-harness contract

- Read `UI_VISUAL_REFERENCE_MAP.json` (or the simple harness map).
- Iterate `sessions[]`; use `primary_reference`.
- Skip nothing: all 60 sessions now have native refs; menu UI-on-black is layout/state truth, not alpha UI-only.
- Use native in-world refs for panel/SDF tuning; old refs are fallback only.
- `tools/ui/visual_diff.py --reference-map docs/UI_VISUAL_REFERENCE_MAP.harness.json` consumes the simple map.

## Known capture caveats

- Old `functional_ui` 1920 refs and old `ui_runtime.json.gz` are upscaled/320x240 historical; do not claim pixel parity.
- Native menu UI-on-black refs are opaque; transparent menu UI-only isolation remains blocked.
- Native in-world toolbar/placer/dynamic/lineage refs are partial-alpha; inspector/settings/info panels include full-panel backdrop.
- Button theme fields are neutral white; dark native look is not from tint values.

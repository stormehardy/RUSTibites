# UI visual parity leaderboard (worst first)

- sessions: 60, rendered: 60, missing refs: 0
- mean alpha IoU: 0.7719037583590533; median: 1.0
- mean RGB MAE (joint opaque): 87.19623390346003; mean exact-pixel %: 20.050709332529983
- legacy refs: 0; rescale-blur suspects: 0

| # | session | alpha IoU | RGB MAE | exact % | bbox d(x0,y0,x1,y1) | ref | threshold |
|---|---|---|---|---|---|---|---|
| 1 | default_world_newgame | 0.0212 | 31.485 | 69.36 | -10,-10,10,10 | native | iou<0.98 mae>3.0 exact<90.0 |
| 2 | default_world | 0.0212 | 31.483 | 69.37 | -10,-10,10,10 | native | iou<0.98 mae>3.0 exact<90.0 |
| 3 | default_world_biology | 0.1206 | 57.954 | 46.51 | -10,-10,10,10 | native | iou<0.98 mae>3.0 exact<90.0 |
| 4 | default_world_inspector | 0.1454 | 36.160 | 67.93 | -10,-10,10,10 | native | iou<0.98 mae>3.0 exact<90.0 |
| 5 | main_world_toolbar | 0.1937 | 162.899 | 7.88 | -4,-10,10,10 | native | iou<0.98 mae>3.0 exact<90.0 |
| 6 | default_world_brain | 0.1937 | 27.314 | 74.52 | -4,-10,10,10 | native | iou<0.98 mae>3.0 exact<90.0 |
| 7 | main_world_tool_placer | 0.1976 | 160.977 | 8.96 | -4,-10,10,10 | native | iou<0.98 mae>3.0 exact<90.0 |
| 8 | main_world_tool_user_actions | 0.1991 | 160.058 | 9.46 | -4,-10,10,10 | native | iou<0.98 mae>3.0 exact<90.0 |
| 9 | default_world_genes | 0.2196 | 29.681 | 73.18 | -10,-10,10,10 | native | iou<0.98 mae>3.0 exact<90.0 |
| 10 | main_world_tool_view | 0.2298 | 139.328 | 19.12 | -4,-10,10,10 | native | iou<0.98 mae>3.0 exact<90.0 |
| 11 | lineage_species_brain | 0.2750 | 147.469 | 15.96 | -4,-10,10,10 | native | iou<0.98 mae>3.0 exact<90.0 |
| 12 | lineage_species_genes | 0.2750 | 147.469 | 15.96 | -4,-10,10,10 | native | iou<0.98 mae>3.0 exact<90.0 |
| 13 | lineage_species_info | 0.2750 | 147.469 | 15.96 | -4,-10,10,10 | native | iou<0.98 mae>3.0 exact<90.0 |
| 14 | lineage_species_panel | 0.2750 | 147.469 | 15.96 | -4,-10,10,10 | native | iou<0.98 mae>3.0 exact<90.0 |
| 15 | lineage_species_tabs | 0.2750 | 147.469 | 15.96 | -4,-10,10,10 | native | iou<0.98 mae>3.0 exact<90.0 |
| 16 | main_world_lineage | 0.2750 | 125.024 | 26.41 | -4,-10,10,10 | native | iou<0.98 mae>3.0 exact<90.0 |
| 17 | main_world_dynamic_parameters | 0.6946 | 149.700 | 10.19 | -4,-10,0,0 | native | iou<0.98 mae>3.0 exact<90.0 |
| 18 | world_dynamic_parameters_controls | 0.7138 | 155.061 | 5.92 | -4,-10,0,0 | native | iou<0.98 mae>3.0 exact<90.0 |
| 19 | world_settings_controls | 0.7138 | 155.061 | 5.92 | -4,-10,0,0 | native | iou<0.98 mae>3.0 exact<90.0 |
| 20 | workshop_edit_shared | 1.0000 | 174.842 | 10.89 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 21 | main_menu_bibite_editor_brain | 1.0000 | 173.747 | 7.67 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 22 | main_menu_bibite_editor_genes | 1.0000 | 173.411 | 7.93 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 23 | main_menu_scenario_editor | 1.0000 | 171.660 | 0.54 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 24 | main_menu_settings_gameplay | 1.0000 | 134.228 | 16.55 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 25 | main_menu_settings_graphics | 1.0000 | 133.175 | 17.07 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 26 | main_menu_settings_hotkeys | 1.0000 | 131.273 | 16.09 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 27 | workshop_panel | 1.0000 | 128.846 | 17.31 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 28 | main_menu_workshop | 1.0000 | 128.338 | 17.30 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 29 | library_create_new | 1.0000 | 103.210 | 0.54 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 30 | library_selector_flow | 1.0000 | 103.210 | 0.54 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 31 | challenge_parameters_editor | 1.0000 | 98.422 | 13.51 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 32 | challenge_champion_selection | 1.0000 | 98.404 | 13.53 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 33 | newgame_new_challenge_button | 1.0000 | 83.520 | 0.91 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 34 | newgame_scenario_options | 1.0000 | 81.899 | 1.27 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 35 | main_world_information | 1.0000 | 79.819 | 3.77 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 36 | main_world_settings_panel | 1.0000 | 79.819 | 3.77 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 37 | library_editor_genes_flow | 1.0000 | 74.583 | 5.58 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 38 | library_editor_brain_after_add | 1.0000 | 71.327 | 5.18 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 39 | library_editor_brain_flow | 1.0000 | 71.327 | 5.18 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 40 | scenario_editor_zones | 1.0000 | 57.992 | 0.65 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 41 | newgame_new_scenario_button | 1.0000 | 57.885 | 0.57 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 42 | scenario_editor_flow | 1.0000 | 57.885 | 0.57 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 43 | scenario_editor_sim | 1.0000 | 57.885 | 0.57 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 44 | scenario_editor_ChallengeOptions | 1.0000 | 54.951 | 0.56 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 45 | main_menu_new_game | 1.0000 | 52.141 | 23.37 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 46 | main_menu_new_panel | 1.0000 | 50.890 | 0.00 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 47 | main_menu | 1.0000 | 44.831 | 73.42 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 48 | main_menu_achievements | 1.0000 | 44.358 | 0.06 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 49 | main_menu_additional_tools | 1.0000 | 44.122 | 3.22 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 50 | main_menu_credits | 1.0000 | 40.985 | 2.81 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 51 | settings_page_graphics_controls | 1.0000 | 30.633 | 19.35 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 52 | settings_page_gameplay_controls | 1.0000 | 30.584 | 19.48 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 53 | main_menu_settings | 1.0000 | 26.139 | 20.72 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 54 | settings_page_hotkeys_controls | 1.0000 | 20.843 | 19.55 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 55 | challenge_scenario_panel | 1.0000 | 20.115 | 37.30 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 56 | challenge_selector | 1.0000 | 20.115 | 37.30 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 57 | main_menu_bibite_library | 1.0000 | 17.888 | 1.58 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 58 | main_menu_load_game | 1.0000 | 15.865 | 26.17 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 59 | main_menu_functional_title | 1.0000 | 15.539 | 88.06 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 60 | main_menu_title | 1.0000 | 15.539 | 88.06 | 0,0,0,0 | native | mae>3.0 exact<90.0 |

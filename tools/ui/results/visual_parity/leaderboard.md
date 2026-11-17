# UI visual parity leaderboard (worst first)

- sessions: 60, rendered: 60, missing refs: 8
- mean alpha IoU: 0.8963500401514987; median: 0.9482744984567901
- mean RGB MAE (joint opaque): 64.43085368858463; mean exact-pixel %: 12.669996371049267
- legacy refs: 52; rescale-blur suspects: 52

| # | session | alpha IoU | RGB MAE | exact % | bbox d(x0,y0,x1,y1) | ref | threshold |
|---|---|---|---|---|---|---|---|
| 1 | main_world_information | 0.1408 | 93.819 | 15.18 | 10,10,0,-10 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 2 | main_world_settings_panel | 0.1408 | 93.819 | 15.18 | 10,10,0,-10 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 3 | main_menu_functional_title | 0.2342 | 74.748 | 0.01 | 25,33,-50,-25 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 4 | main_menu_load_game | 0.8531 | 23.757 | 0.32 | 25,33,-45,-25 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 5 | settings_page_graphics_controls | 0.8538 | 30.631 | 0.15 | 25,33,-45,-25 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 6 | settings_page_hotkeys_controls | 0.8555 | 40.894 | 0.06 | 25,33,-45,-17 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 7 | settings_page_gameplay_controls | 0.8624 | 30.947 | 0.05 | 25,33,-45,-6 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 8 | main_menu_settings | 0.8762 | 43.055 | 0.00 | 25,33,-45,-25 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 9 | main_world_toolbar | 0.8953 | 61.653 | 57.02 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 10 | main_world_tool_placer | 0.9062 | 65.746 | 51.16 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 11 | main_world_tool_user_actions | 0.9106 | 63.741 | 53.02 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 12 | challenge_champion_selection | 0.9160 | 28.870 | 0.29 | 10,10,0,-10 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 13 | challenge_parameters_editor | 0.9160 | 28.846 | 0.29 | 10,10,-2,-10 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 14 | challenge_scenario_panel | 0.9198 | 30.307 | 0.04 | 10,10,-1,-10 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 15 | challenge_selector | 0.9198 | 30.307 | 0.04 | 10,10,-1,-10 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 16 | workshop_panel | 0.9297 | 145.210 | 0.01 | 0,10,0,-10 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 17 | library_create_new | 0.9364 | 38.525 | 0.34 | 10,13,-10,-2 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 18 | library_selector_flow | 0.9364 | 38.525 | 0.34 | 10,13,-10,-2 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 19 | lineage_species_brain | 0.9385 | 56.360 | 60.71 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 20 | lineage_species_genes | 0.9385 | 56.360 | 60.71 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 21 | lineage_species_info | 0.9385 | 56.360 | 60.71 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 22 | lineage_species_panel | 0.9385 | 56.360 | 60.71 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 23 | lineage_species_tabs | 0.9385 | 56.360 | 60.71 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 24 | workshop_edit_shared | 0.9443 | 130.959 | 0.13 | 0,10,0,-10 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 25 | main_menu_new_game | 0.9482 | 42.204 | 0.13 | 10,10,-1,-10 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 26 | library_editor_brain_after_add | 0.9483 | 29.856 | 0.07 | 10,13,-10,-2 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 27 | library_editor_brain_flow | 0.9483 | 29.856 | 0.07 | 10,13,-10,-2 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 28 | library_editor_genes_flow | 0.9495 | 33.338 | 0.13 | 10,13,-10,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 29 | scenario_editor_ChallengeOptions | 0.9549 | 30.799 | 0.07 | 10,12,-10,-2 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 30 | scenario_editor_zones | 0.9549 | 30.799 | 0.07 | 10,12,-10,-2 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 31 | newgame_new_challenge_button | 0.9553 | 30.898 | 0.07 | 10,12,-10,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 32 | newgame_new_scenario_button | 0.9553 | 30.898 | 0.07 | 10,12,-10,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 33 | scenario_editor_flow | 0.9553 | 30.898 | 0.07 | 10,12,-10,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 34 | scenario_editor_sim | 0.9553 | 30.898 | 0.07 | 10,12,-10,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 35 | main_world_tool_view | 0.9567 | 33.676 | 70.23 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 36 | newgame_scenario_options | 0.9567 | 30.864 | 0.07 | 10,10,-1,-2 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 37 | main_world_lineage | 0.9572 | 42.944 | 66.62 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 38 | main_menu_bibite_library | 0.9580 | 20.682 | 0.11 | 10,10,-1,-10 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 39 | main_menu_achievements | 0.9587 | 20.794 | 0.11 | 10,0,0,-10 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 40 | world_dynamic_parameters_controls | 0.9675 | 153.926 | 5.92 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 41 | world_settings_controls | 0.9675 | 153.926 | 5.92 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 42 | main_menu_additional_tools | 0.9680 | 22.079 | 0.11 | 10,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 43 | main_world_dynamic_parameters | 0.9692 | 144.540 | 10.87 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 44 | main_menu_credits | 0.9726 | 23.213 | 0.11 | 10,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 45 | main_menu_new_panel | 0.9727 | 23.225 | 0.11 | 10,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 46 | main_menu_workshop | 0.9730 | 135.489 | 0.03 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 47 | main_menu_scenario_editor | 0.9778 | 135.748 | 0.03 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 48 | main_menu_settings_graphics | 0.9778 | 135.748 | 0.03 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 49 | main_menu_settings_hotkeys | 0.9778 | 135.748 | 0.03 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 50 | main_menu_settings_gameplay | 0.9778 | 135.732 | 0.03 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 51 | main_menu_bibite_editor_brain | 0.9779 | 157.594 | 0.29 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |
| 52 | main_menu_bibite_editor_genes | 0.9782 | 147.873 | 0.25 | 0,0,0,0 | legacy | iou<0.98 mae>3.0 exact<90.0 |

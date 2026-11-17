# UI visual parity leaderboard (worst first)

- sessions: 12, rendered: 12, missing refs: 0
- mean alpha IoU: 0.5135977062968219; median: 0.3418123070987654
- mean RGB MAE (joint opaque): 50.60298260004388; mean exact-pixel %: 37.744629859140666
- legacy refs: 0; rescale-blur suspects: 0

| # | session | alpha IoU | RGB MAE | exact % | bbox d(x0,y0,x1,y1) | ref | threshold |
|---|---|---|---|---|---|---|---|
| 1 | main_world_toolbar | 0.1069 | 36.097 | 68.51 | 0,0,0,0 | native | iou<0.98 mae>3.0 exact<90.0 |
| 2 | main_world_tool_placer | 0.1242 | 36.754 | 68.80 | 0,0,0,0 | native | iou<0.98 mae>3.0 exact<90.0 |
| 3 | main_world_tool_user_actions | 0.1310 | 38.858 | 67.99 | 0,0,0,0 | native | iou<0.98 mae>3.0 exact<90.0 |
| 4 | main_world_lineage | 0.1778 | 21.772 | 74.52 | 0,0,0,0 | native | iou<0.98 mae>3.0 exact<90.0 |
| 5 | main_world_tool_view | 0.2467 | 20.454 | 75.43 | 0,0,0,0 | native | iou<0.98 mae>3.0 exact<90.0 |
| 6 | main_world_information | 0.3418 | 105.542 | 11.82 | 1,1,-1,-10 | native | iou<0.98 mae>3.0 exact<90.0 |
| 7 | main_world_settings_panel | 0.3418 | 105.542 | 11.82 | 1,1,-1,-10 | native | iou<0.98 mae>3.0 exact<90.0 |
| 8 | main_world_dynamic_parameters | 0.7357 | 34.049 | 13.48 | 0,0,0,0 | native | iou<0.98 mae>3.0 exact<90.0 |
| 9 | default_world_biology | 0.9858 | 65.109 | 21.70 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 10 | default_world_inspector | 0.9882 | 51.564 | 14.36 | 0,0,0,0 | native | mae>3.0 exact<90.0 |
| 11 | default_world_brain | 0.9910 | 44.151 | 13.46 | 6,0,0,0 | native | mae>3.0 exact<90.0 |
| 12 | default_world_genes | 0.9922 | 47.342 | 11.03 | 0,0,0,0 | native | mae>3.0 exact<90.0 |

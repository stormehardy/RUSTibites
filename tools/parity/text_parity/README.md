# Text parity measurements (2026-11-11)

Offline comparisons of the port's text with the real game's captured UI pictures (no game needed). The scripts keep
the paths of the session that wrote them (`C:\Temp\...`, a black 1920x1080 `black.png`); edit the top of each one.

- `text_sweep.ps1 <exe> <tag>`: renders every `menu_ui` reference session (`--ui-session`) and prints the mean pixel difference (`sweep_one.py`); `sweep_cmp.py a.txt b.txt` compares two runs (watch the sum).
- `text_outliers.py <tag>`: per single-line text block, ink / centroid / extent against the reference, outliers and the median ink ratio.
- `text_ink.py render ref`: the same for one picture. `extent.py`, `crop_two.py`, `cropob.py`: extents and side-by-side crops (reference over render).
- `over_black.py ref render out_ref out_render`, `dark_ink.py`: the inspector's `native_inspector_*_ui_only` references (premultiplied: the RGB over black is the picture) and dark-on-light label ink.

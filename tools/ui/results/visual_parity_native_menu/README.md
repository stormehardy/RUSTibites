# Native menu UI-on-black visual diagnostic

Diagnostic against parity's native 1920 **UI-on-black** menu captures in
`assets/game/reference/functional_ui_native/menu_ui/` (title/settings). Progress, not a release pixel gate.

- sessions: `main_menu_title`, `main_menu_settings`
- references: 2 native menu UI-on-black PNGs (no legacy refs)
- render mode: current viewer (`91de667`) filtered menu `--ui-session --ui-only --ui-text sdf`, flattened over
  black (`--render-background black`); view reports 2px sampling residual
- alpha masks: both opaque after flattening, so `alpha_iou` is 1.0 and not a gate; use RGB MAE / exact-pixel
  / bbox / diagnostics
- latest per-session: title **RGB MAE 9.13, exact 91.73%**; settings **RGB MAE 21.06, exact 20.87%**
  (mean RGB MAE 15.10, mean exact 56.30%, 2/2 threshold failures while settings text/layout aligns)

Regenerate:

```powershell
python tools/ui/run_visual_parity.py --only main_menu_title --only main_menu_settings `
  --native-ui-on-black --diagnostics --viewer-args "--ui-text sdf" `
  --results-dir tools/ui/results/visual_parity_native_menu
```

Parity's native title/settings background capture (`568ba01`,
`functional_ui_native/menu_background/menu_background_title_1920x1080.png`) can be passed as
`--viewer-args "--ui-text sdf --ui-background <png>"` for composited-background A/B; the UI-on-black numbers
above remain the content-focused diagnostic. Do not treat these as a release pixel gate until the native menu
matrix and canonical menu map are finalized.

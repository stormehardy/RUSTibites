# UI control audit — runtime atlases vs viewer action coverage

**Scope:** every captured UI control node in `tools/ui/ui_runtime_atlas.json.gz`
(all 9 sessions), compared with the viewer's current hit rectangles/actions in
`ui_runtime.rs`, `hud.rs`, `ui.rs` and `menu.rs`. Audit date 2026-11-02, current
tree; no implementation changes in this pass. The companion game-surface
inventory is [`UI_FUNCTIONAL_PARITY.md`](UI_FUNCTIONAL_PARITY.md). **Update (`4159253`):** 24 additional
functional-UI sessions (165 labelled paths) were captured under `assets/game/reference/functional_ui/`; the
145-control/9-session audit below is the original runtime-atlas set and has not been recalculated for them.
**Update (native atlas, 2026-11-03):** `tools/ui/viewer_control_ids.json` is now **schema v5**, regenerated
from the native 1920 atlas via the viewer's authoritative dump:
`rustibites-view --ui-controls --ui-controls-out target/viewer_controls_native.json`, then
`python tools/ui/build_control_ids.py` (or `--check` in CI). It contains **1493 controls / 60 native sessions**
with native rects, stable FNV ids (`control_id(session,path)`), and parsed value/action metadata. The old
2501-control/legacy-atlas registry is historical.
**Update (`3de45a8`):** the six-variant/allow-list description below is the historical baseline; the current
framework has the expanded `UiAction`/`ControlCommand`/`PanelKind` set, an audited control-id registry
(`tools/ui/viewer_control_ids.json`) and generic `ActivateControl` hits for all active controls. Treat the
tables below as the original audit, not the current action surface.

The dump contains **145 active control nodes** across the sessions (only active
nodes count; the same UI hierarchy also contains ~1,100 inactive controls per
world session, mostly hidden panels/settings prefabs). A control is **drawn**
when its captured node/subtree has active images/texts and its session is used
by the viewer; **hit** only exists when the viewer emits a hit rectangle.

## Current action surface

`UiAction` has six variants: `TogglePause`, `Step`, `SetSpeed(u32)`,
`ToggleOverlay(OverlayKind)`, `SetTab(InspectorTab)`, `CloseInspector`.
`App::apply_ui_actions` handles all six.

Runtime hit mapping is currently a hard-coded **name allow-list** in
`ui_runtime.rs::hit_action`:

| Runtime node name | Emitted action | Notes |
|---|---|---|
| `PauseButton`, `PlayButton` | `TogglePause` | works |
| `OpenQuickStats` | `SetTab(InspectorTab::Stats)` | opens Stats tab, not a quick-stats panel |
| `SettingsButton` | `SetTab(InspectorTab::Stats)` | **semantically wrong placeholder**; does not open Settings |
| everything else | none | visible button is inert |

The title menu has a separate hard-coded row map in `menu.rs` (`menu_button_rects`
+ `hit_row`): Continue, New Game, Load Game, Settings, Quit. All other captured
title/settings controls are inert. The `main_menu_settings` session is not
rendered at all (the Settings view uses the curated fallback overlay), so its
34 controls have no hit path.

The curated fallback HUD (`hud.rs`, used with `--no-runtime-ui` or when runtime
data is absent) has 14 controls wired: pause, step, speed -, speed +, grid,
zones, pellets, pheromones, HUD, four inspector tabs, and close-inspector.
Those actions are not available from the exact runtime layout for most controls.

## Coverage summary

| Session | Capture/viewer use | Active controls | Runtime hits | Menu hits | Inert visible |
| --- | --- | ---: | ---: | ---: | ---: |
| `main_menu` | legacy ignored | 1 | 0 | 0 | 1 |
| `default_world_newgame` | legacy/ignored | 12 | 4 | 0 | 8 |
| `default_world` | pause/default HUD | 12 | 4 | 0 | 8 |
| `default_world_inspector` | selected Stats | 22 | 4 | 0 | 18 |
| `main_menu_title` | title Main view | 10 | 0 | 5 | 5 |
| `main_menu_settings` | **not used** (curated Settings overlay) | 34 | 0 | 2* | 32 |
| `default_world_genes` | selected Genes | 17 | 4 | 0 | 13 |
| `default_world_biology` | selected Biology | 16 | 4 | 0 | 12 |
| `default_world_brain` | selected Brain | 21 | 4 | 0 | 17 |

\* The two "menu hits" are zero-size `GroupEdit` helper nodes counted by the
path-matching appendix; the settings session is not built by the viewer, so all
34 controls are effectively inert. The captured Settings view has **zero
usable controls** in the viewer today.

## Inert-but-visible controls (world sessions)

These controls are drawn in the exact runtime HUD but have no hit rectangle or
action. Grouped by panel (full paths/labels/rects are in the appendix):

- **Bottom-left tools:** `PlacerButton`, `UserActions`, `ViewButton` (3 buttons;
  50x50 each). Need tool menus (place pellet/meat/zone/tower, user actions,
  view/filter menu).
- **Top-right buttons:** `InformationPanelButton`, `DynamicParametersButton`,
  `AncestralLineageButton` (3 buttons; 60x60). `SettingsButton` is hit but maps
  to the Stats tab rather than opening Settings.
- **Top-center:** slider `TopCenter/sliderHolder/BasicSlider/.../Slider` is a
  `UnityEngine.UI.Slider` (no hit/drag/value action); `FloatInfoText` (`X 0.00`)
  is a Button but inert. Speed is keyboard-only in the runtime layout.
- **Selected-bibite tabs:** `StatsButton`, `GeneButton`, `BiologyButton`,
  `BrainButton` are all inert in the exact runtime layout; tabs only work via
  the curated HUD or keyboard/`--tab`.
- **Stats panel actions:** `KILL`, `LAY`, `SAVE`, `EDIT` (4 buttons) inert;
  `Alive Children/Button` ("0") and the species label holder are inert.
- **Quick stats:** `OpenQuickStats` hit exists but only switches to the Stats
  tab; no quick-stats overlay exists.
- **Genes/Biology/Brain sessions:** same shared chrome inert; their active tab
  buttons/panel controls (17/16/21 active nodes) are inert. Scrollrects,
  toggles and sliders in these panels have no value binding.

## Inert-but-visible controls (menu/settings)

- `main_menu_title` (10 active buttons): Continue/New/Load/Settings/Quit are
  mapped by `MenuState`; **Patreon Holder**, **Bibite Library** (`Editor`),
  **Achievements**, **Additional Tools** (`Section/More`) and **Credits** are
  inert. The Patreon button is an external-link candidate (non-goal unless
  browser launch is accepted).
- `main_menu_settings` (34 active controls): the viewer does not use this
  session; the curated Settings view only has three local toggles + Back. The
  captured session contains Graphics/Gameplay/Hotkeys selectors, close/reset
  buttons, toggles, sliders, dropdowns, color pickers and a scrollbar — all
  inert and mostly needing new settings state/persistence.
- `main_menu` legacy capture has one loading-slider control (ignored).

## Missing controls / state support

The following functional areas are missing from the viewer's state model, not
just from hit testing:

1. **Panel/modal stack:** no generic visibility/state for Settings, Placer,
   User actions, View, Information, Dynamic Parameters, Ancestral Lineage,
   Quick Stats, Bibite Library, Achievements, Tools, Credits.
2. **Settings model:** no binding for toggles/sliders/dropdowns/color pickers,
   no apply/reset/persistence; captured settings pages (Graphics, Gameplay,
   Hotkeys) are not rendered.
3. **World tool actions:** no placement tools, no kill/lay/save/edit actions,
   no alive-children navigation.
4. **Speed slider:** no slider drag/value commit path; runtime speed text is
   display-only.
5. **Tab buttons:** runtime layout lacks four tab hits (curated only).
6. **Scroll views:** no scrolling/drag support for settings, library, credits,
   achievements, or inspector sub-panels.
7. **Menu subviews:** New Game scenario picker, save list beyond the curated
   rows, settings pages, library/achievements/tools/credits informational
   views.

## Recommended action-mapping scheme

Replace the name allow-list with a **generic runtime control map + action
registry**:

1. **Stable control id:** `ControlId = stable_hash(session, path)` (or the
   captured node path itself while debugging). Every active node with
   `Button`/`Selectable`/`Toggle`/`Slider`/`InputField`/`Scrollbar`/`ScrollRect`
   gets exactly one entry, regardless of whether it is currently mapped.
2. **Runtime control record:** `{id, session, path, label, rect, kind,
   state_binding}`. Emit hit rects for all `Button`/`Selectable`/`Toggle` nodes
   (and drag/value rects for Slider/Scrollbar/ScrollRect) in painter order.
3. **Command registry:** map `ControlId` (preferred) or
   `(panel_key, node_name)` fallback to a typed `UiCommand`:
   `TogglePause`, `Step`, `SetSpeed`, `OpenPanel(PanelId)`, `ClosePanel`,
   `SetTab`, `ToggleOption(SettingId)`, `SetValue(SettingId, f32)`,
   `InvokeAction(ActionId)`, `Scroll(delta)`, `Quit`, ...
   Unknown ids produce a logged `UnmappedControl` event (not silent inert).
4. **Dispatch:** `UiFrame::interact` returns `ControlEvent {id, kind, value}`
   and a single `App::apply_control_event` dispatches through the registry.
   This also handles menu buttons generically instead of the current row-index
   map, while keeping keyboard shortcuts as an alternative path.
5. **Panel state:** add a `PanelState` map keyed by panel path/name with
   visibility/active tab/scroll offset; registry commands open/close panels.
   Settings values need a typed settings model with apply/reset/persist.
6. **Migration:** keep the existing four runtime mappings and curated HUD as
   fallbacks while migrating panel-by-panel; compare generated hit maps with
   the captured active node list to detect misses.

## Phasing recommendation

- **P0 (framework, low risk):** generic hit map/ids/action registry; wire
  Pause/Play, four inspector tabs, Settings open, Quick Stats, menu rows,
  close/back; replace the wrong `SettingsButton -> SetTab(Stats)` mapping.
- **P1 (world chrome):** bottom-left tool/view menus, top-right information/
  dynamic-parameters/lineage/settings panels, speed slider drag/value,
  selected-bibite tabs and KILL/LAY/SAVE/EDIT/alive-children actions.
- **P2 (menu/settings):** use `main_menu_settings` session, settings pages with
  typed values/apply/reset/persistence, scenario picker, save list, library/
  achievements/tools/credits informational views.
- **P3 (deep panels):** scroll views and inspector sub-panel controls
  (Genes/Biology/Brain toggles, sliders, color pickers, editor), plus Steam/
  Workshop rows as explicit non-goals.

## Appendix — full per-session control inventory

Generated from the committed atlas (current tree). Rect is the captured
RectTransform `(anchored_position) size_delta pivot`; it is not rescaled to the
window, and runtime composition may offset it. "Runtime hit" is the current
`ui_runtime.rs` mapping; "Menu/action" is the current `menu.rs` path/row map.

### `main_menu`


**Slider**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `LoadingPanel/LoadingFooter/LoadingBar/Slider` | UnityEngine.UI.Slider | Slider | (0,0) 0x0 piv(0.50,0.50) | yes | - | - |

### `default_world_newgame`


**PlacerButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/PlacerButton` | UnityEngine.UI.Button | PlacerButton | (35,-35) 50x50 piv(0.50,0.50) | yes | - | - |

**UserActions**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/UserActions` | UnityEngine.UI.Button | UserActions | (95,-35) 50x50 piv(0.50,0.50) | yes | - | - |

**ViewButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/ViewButton` | UnityEngine.UI.Button | ViewButton | (155,-35) 50x50 piv(0.50,0.50) | yes | - | - |

**InformationPanelButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/InformationPanelButton` | UnityEngine.UI.Button | InformationPanelButton | (181,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**DynamicParametersButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/DynamicParametersButton` | UnityEngine.UI.Button | DynamicParametersButton | (246,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**AncestralLineageButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/AncestralLineageButton` | UnityEngine.UI.Button | AncestralLineageButton | (311,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**SettingsButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/SettingsButton` | UnityEngine.UI.Button | SettingsButton | (376,-40) 60x60 piv(0.50,0.50) | yes | SetTab(Stats) | - |

**TopCenter**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/PauseButton` | UnityEngine.UI.Button | PauseButton | (50,-35) 50x50 piv(0.50,0.50) | yes | TogglePause | - |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/PlayButton` | UnityEngine.UI.Button | PlayButton | (110,-35) 50x50 piv(0.50,0.50) | yes | TogglePause | - |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/sliderHolder/BasicSlider/NormalSecton/SliderPanel/Slider` | UnityEngine.UI.Slider | Slider | (0,0) -20x0 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/sliderHolder/BasicSlider/FloatInfoText` | UnityEngine.UI.Button | X 0.00 | (330,-25) 130x35 piv(0.50,0.50) | yes | - | - |

**Layer1/Right Panels/QuickStatsHolder**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer1/Panels/Right Panels/QuickStatsHolder/OpenButtons/OpenQuickStats` | UnityEngine.UI.Button | Info / + | (0,0) 82x30 piv(1.00,1.00) | yes | SetTab(Stats) | - |

### `default_world`


**PlacerButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/PlacerButton` | UnityEngine.UI.Button | PlacerButton | (35,-35) 50x50 piv(0.50,0.50) | yes | - | - |

**UserActions**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/UserActions` | UnityEngine.UI.Button | UserActions | (95,-35) 50x50 piv(0.50,0.50) | yes | - | - |

**ViewButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/ViewButton` | UnityEngine.UI.Button | ViewButton | (155,-35) 50x50 piv(0.50,0.50) | yes | - | - |

**InformationPanelButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/InformationPanelButton` | UnityEngine.UI.Button | InformationPanelButton | (181,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**DynamicParametersButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/DynamicParametersButton` | UnityEngine.UI.Button | DynamicParametersButton | (246,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**AncestralLineageButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/AncestralLineageButton` | UnityEngine.UI.Button | AncestralLineageButton | (311,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**SettingsButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/SettingsButton` | UnityEngine.UI.Button | SettingsButton | (376,-40) 60x60 piv(0.50,0.50) | yes | SetTab(Stats) | - |

**TopCenter**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/PauseButton` | UnityEngine.UI.Button | PauseButton | (50,-35) 50x50 piv(0.50,0.50) | yes | TogglePause | - |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/PlayButton` | UnityEngine.UI.Button | PlayButton | (110,-35) 50x50 piv(0.50,0.50) | yes | TogglePause | - |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/sliderHolder/BasicSlider/NormalSecton/SliderPanel/Slider` | UnityEngine.UI.Slider | Slider | (0,0) -20x0 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/sliderHolder/BasicSlider/FloatInfoText` | UnityEngine.UI.Button | X 0.00 | (330,-25) 130x35 piv(0.50,0.50) | yes | - | - |

**Layer1/Right Panels/QuickStatsHolder**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer1/Panels/Right Panels/QuickStatsHolder/OpenButtons/OpenQuickStats` | UnityEngine.UI.Button | Info / + | (0,0) 82x30 piv(1.00,1.00) | yes | SetTab(Stats) | - |

### `default_world_inspector`


**PlacerButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/PlacerButton` | UnityEngine.UI.Button | PlacerButton | (35,-35) 50x50 piv(0.50,0.50) | yes | - | - |

**UserActions**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/UserActions` | UnityEngine.UI.Button | UserActions | (95,-35) 50x50 piv(0.50,0.50) | yes | - | - |

**ViewButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/ViewButton` | UnityEngine.UI.Button | ViewButton | (155,-35) 50x50 piv(0.50,0.50) | yes | - | - |

**InformationPanelButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/InformationPanelButton` | UnityEngine.UI.Button | InformationPanelButton | (204,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**DynamicParametersButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/DynamicParametersButton` | UnityEngine.UI.Button | DynamicParametersButton | (269,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**AncestralLineageButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/AncestralLineageButton` | UnityEngine.UI.Button | AncestralLineageButton | (334,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**SettingsButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/SettingsButton` | UnityEngine.UI.Button | SettingsButton | (399,-40) 60x60 piv(0.50,0.50) | yes | SetTab(Stats) | - |

**TopCenter**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/PauseButton` | UnityEngine.UI.Button | PauseButton | (50,-35) 50x50 piv(0.50,0.50) | yes | TogglePause | - |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/PlayButton` | UnityEngine.UI.Button | PlayButton | (110,-35) 50x50 piv(0.50,0.50) | yes | TogglePause | - |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/sliderHolder/BasicSlider/NormalSecton/SliderPanel/Slider` | UnityEngine.UI.Slider | Slider | (0,0) -20x0 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/sliderHolder/BasicSlider/FloatInfoText` | UnityEngine.UI.Button | X 0.00 | (330,-25) 130x35 piv(0.50,0.50) | yes | - | - |

**BibiteButtons**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer1/Panels/BibiteButtons/StatsButton` | UnityEngine.UI.Button | StatsButton | (40,-40) 60x60 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibiteButtons/GeneButton` | UnityEngine.UI.Button | GeneButton | (110,-40) 60x60 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibiteButtons/BiologyButton` | UnityEngine.UI.Button | BiologyButton | (180,-40) 60x60 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibiteButtons/BrainButton` | UnityEngine.UI.Button | BrainButton | (250,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**BibitePanels/StatsPanel**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer1/Panels/BibitePanels/StatsPanel/MainSection/TagHolder/Label Holder` | UnityEngine.UI.Button | None | (207,-15) 299x30 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibitePanels/StatsPanel/MainSection/BibiteStats/Alive Children/Button ` | UnityEngine.UI.Button | 0 | (323,-13) 57x26 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibitePanels/StatsPanel/ButtonHolder/KillBibiteButton` | UnityEngine.UI.Button | KILL | (60,0) 60x60 piv(1.00,1.00) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibitePanels/StatsPanel/ButtonHolder/LayBibiteEggButton` | UnityEngine.UI.Button | LAY | (60,-90) 60x60 piv(1.00,1.00) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibitePanels/StatsPanel/ButtonHolder/Save` | UnityEngine.UI.Button | SAVE | (60,-180) 60x60 piv(1.00,1.00) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibitePanels/StatsPanel/ButtonHolder/Edit` | UnityEngine.UI.Button | EDIT | (60,-270) 60x60 piv(1.00,1.00) | yes | - | - |

**Layer1/Right Panels/QuickStatsHolder**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer1/Panels/Right Panels/QuickStatsHolder/OpenButtons/OpenQuickStats` | UnityEngine.UI.Button | Info / + | (0,0) 82x30 piv(1.00,1.00) | yes | SetTab(Stats) | - |

### `main_menu_title`


**Welcome/Patreon Holder**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UI/WelcomeScreen/Patreon Holder/Button` | UnityEngine.UI.Button | This is all possible thanks to the amazing patrons supporting the project on Patreon  Help | (45,-45) 90x90 piv(0.50,0.50) | yes | - | - |

**ButtonsPanel**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UI/WelcomeScreen/Main Holder/ButtonsPanel/Continue` | UnityEngine.UI.Button | Continue | (248,-57) 435x54 piv(0.50,0.50) | yes | - | Continue -> Start latest save |
| `UI/WelcomeScreen/Main Holder/ButtonsPanel/NewSim` | UnityEngine.UI.Button | New | (248,-131) 435x54 piv(0.50,0.50) | yes | - | New Game -> Start Default.zip |
| `UI/WelcomeScreen/Main Holder/ButtonsPanel/LoadSim` | UnityEngine.UI.Button | Load | (248,-205) 435x54 piv(0.50,0.50) | yes | - | Load Game -> LoadGame view |
| `UI/WelcomeScreen/Main Holder/ButtonsPanel/Editor` | UnityEngine.UI.Button | Bibite Library | (248,-279) 435x54 piv(0.50,0.50) | yes | - | - |
| `UI/WelcomeScreen/Main Holder/ButtonsPanel/Settings` | UnityEngine.UI.Button | Settings | (248,-353) 435x54 piv(0.50,0.50) | yes | - | Settings -> Settings view |
| `UI/WelcomeScreen/Main Holder/ButtonsPanel/Section/Achievements` | UnityEngine.UI.Button | Achievements | (218,-32) 395x54 piv(0.50,0.50) | yes | - | - |
| `UI/WelcomeScreen/Main Holder/ButtonsPanel/Section/More` | UnityEngine.UI.Button | Additional Tools | (218,-106) 395x54 piv(0.50,0.50) | yes | - | - |
| `UI/WelcomeScreen/Main Holder/ButtonsPanel/Section/Credits` | UnityEngine.UI.Button | Credits | (218,-180) 395x54 piv(0.50,0.50) | yes | - | - |
| `UI/WelcomeScreen/Main Holder/ButtonsPanel/Quit` | UnityEngine.UI.Button | Quit | (248,-659) 435x54 piv(0.50,0.50) | yes | - | Quit -> Quit |

### `main_menu_settings`


**Welcome/Patreon Holder**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UI/WelcomeScreen/Patreon Holder/Button` | UnityEngine.UI.Button | This is all possible thanks to the amazing patrons supporting the project on Patreon  Help | (45,-45) 90x90 piv(0.50,0.50) | yes | - | - |

**ButtonsPanel**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UI/WelcomeScreen/Main Holder/ButtonsPanel/Continue` | UnityEngine.UI.Button | Continue | (248,-57) 435x54 piv(0.50,0.50) | yes | - | - |
| `UI/WelcomeScreen/Main Holder/ButtonsPanel/NewSim` | UnityEngine.UI.Button | New | (248,-131) 435x54 piv(0.50,0.50) | yes | - | - |
| `UI/WelcomeScreen/Main Holder/ButtonsPanel/LoadSim` | UnityEngine.UI.Button | Load | (248,-205) 435x54 piv(0.50,0.50) | yes | - | - |
| `UI/WelcomeScreen/Main Holder/ButtonsPanel/Editor` | UnityEngine.UI.Button | Bibite Library | (248,-279) 435x54 piv(0.50,0.50) | yes | - | - |
| `UI/WelcomeScreen/Main Holder/ButtonsPanel/Settings` | UnityEngine.UI.Button | Settings | (248,-353) 435x54 piv(0.50,0.50) | yes | - | - |
| `UI/WelcomeScreen/Main Holder/ButtonsPanel/Section/Achievements` | UnityEngine.UI.Button | Achievements | (218,-32) 395x54 piv(0.50,0.50) | yes | - | - |
| `UI/WelcomeScreen/Main Holder/ButtonsPanel/Section/More` | UnityEngine.UI.Button | Additional Tools | (218,-106) 395x54 piv(0.50,0.50) | yes | - | - |
| `UI/WelcomeScreen/Main Holder/ButtonsPanel/Section/Credits` | UnityEngine.UI.Button | Credits | (218,-180) 395x54 piv(0.50,0.50) | yes | - | - |
| `UI/WelcomeScreen/Main Holder/ButtonsPanel/Quit` | UnityEngine.UI.Button | Quit | (248,-659) 435x54 piv(0.50,0.50) | yes | - | - |

**SettingsPanel**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View` | UnityEngine.UI.ScrollRect | Graphics Settings / Fullscreen / Automatically Detect Screen Resolution / Screen Resolutio | (0,0) -70x-70 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View/Viewport/Content/GraphicsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingToggle(Clone)/Toggle` | UnityEngine.UI.Toggle | Toggle | (932,-20) 350x40 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View/Viewport/Content/GraphicsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingToggle(Clone)/Toggle` | UnityEngine.UI.Toggle | Toggle | (932,-20) 350x40 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View/Viewport/Content/GraphicsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingDropdown(Clone)/ResetButton` | UnityEngine.UI.Button | ResetButton | (1087,-20) 40x40 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View/Viewport/Content/GraphicsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingDropdown(Clone)/ResetButton` | UnityEngine.UI.Button | ResetButton | (1087,-20) 40x40 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View/Viewport/Content/GraphicsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingSlider(Clone)/FloatInfoText` | UnityEngine.UI.Button | 10% | (566,-20) 234x40 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View/Viewport/Content/GraphicsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingSlider(Clone)/NormalSecton/SliderPanel/Slider` | UnityEngine.UI.Slider | Slider | (0,0) -20x-20 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View/Viewport/Content/GraphicsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingSlider(Clone)/NormalSecton/ResetButton` | UnityEngine.UI.Button | ResetButton | (389,-20) 40x40 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View/Viewport/Content/GraphicsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingDropdown(Clone)/ResetButton` | UnityEngine.UI.Button | ResetButton | (1087,-20) 40x40 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View/Viewport/Content/GraphicsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingGroupPrefab(Clone)/SettingsHolder/ColorPicker(Clone)/ColorDisplay` | UnityEngine.UI.Button | #E79B00FF | (902,-20) 300x40 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View/Viewport/Content/GraphicsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingGroupPrefab(Clone)/SettingsHolder/ColorPicker(Clone)/ResetButton` | UnityEngine.UI.Button | ResetButton | (1087,-20) 40x40 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View/Viewport/Content/GraphicsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingGroupPrefab(Clone)/SettingsHolder/ColorPicker(Clone)/ColorDisplay` | UnityEngine.UI.Button | #000000 | (902,-20) 300x40 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View/Viewport/Content/GraphicsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingGroupPrefab(Clone)/SettingsHolder/ColorPicker(Clone)/ResetButton` | UnityEngine.UI.Button | ResetButton | (1087,-20) 40x40 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View/Viewport/Content/GraphicsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SubSettingsGroupPrefab(Clone)/ToggleHolder/SubSettingMasterTogglePrefab(Clone)/Toggle` | UnityEngine.UI.Toggle | Toggle | (932,-25) 350x40 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View/Viewport/Content/GraphicsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SubSettingsGroupPrefab(Clone)/ToggleHolder/SubSettingMasterTogglePrefab(Clone)/GroupEdit` | UnityEngine.UI.Button | GroupEdit | (-25,0) 0x0 piv(1.00,0.50) | yes | - | zero-size helper |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View/Viewport/Content/GraphicsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SubSettingsGroupPrefab(Clone)/ToggleHolder/SubSettingMasterTogglePrefab(Clone)/Toggle` | UnityEngine.UI.Toggle | Toggle | (932,-25) 350x40 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View/Viewport/Content/GraphicsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SubSettingsGroupPrefab(Clone)/ToggleHolder/SubSettingMasterTogglePrefab(Clone)/GroupEdit` | UnityEngine.UI.Button | GroupEdit | (-25,0) 0x0 piv(1.00,0.50) | yes | - | zero-size helper |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View/Viewport/Content/GraphicsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingGroupPrefab(Clone)/SettingsHolder/SettingToggle(Clone)/Toggle` | UnityEngine.UI.Toggle | Toggle | (932,-20) 350x40 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/PagesPanels/Graphics/Scroll View/Scrollbar Vertical` | UnityEngine.UI.Scrollbar | Scrollbar Vertical | (0,0) 20x-17 piv(1.00,1.00) | yes | - | - |
| `UI/SettingsPanel/SelectorPanel/List/CloseButton` | UnityEngine.UI.Button | CloseButton | (-20,-20) 50x50 piv(1.00,1.00) | yes | - | - |
| `UI/SettingsPanel/SelectorPanel/List/GraphicsButton` | UnityEngine.UI.Button | Graphic Settings | (258,-120) 476x80 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/SelectorPanel/List/Gameplay Button` | UnityEngine.UI.Button | Game Settings | (258,-220) 476x80 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/SelectorPanel/List/Controls Button` | UnityEngine.UI.Button | Hotkeys | (258,-320) 476x80 piv(0.50,0.50) | yes | - | - |
| `UI/SettingsPanel/SelectorPanel/List/Reset Warnings` | UnityEngine.UI.Button | Reset Warnings | (258,-930) 476x80 piv(0.50,0.50) | yes | - | - |

### `default_world_genes`


**PlacerButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/PlacerButton` | UnityEngine.UI.Button | PlacerButton | (35,-35) 50x50 piv(0.50,0.50) | yes | - | - |

**UserActions**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/UserActions` | UnityEngine.UI.Button | UserActions | (95,-35) 50x50 piv(0.50,0.50) | yes | - | - |

**ViewButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/ViewButton` | UnityEngine.UI.Button | ViewButton | (155,-35) 50x50 piv(0.50,0.50) | yes | - | - |

**InformationPanelButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/InformationPanelButton` | UnityEngine.UI.Button | InformationPanelButton | (204,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**DynamicParametersButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/DynamicParametersButton` | UnityEngine.UI.Button | DynamicParametersButton | (269,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**AncestralLineageButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/AncestralLineageButton` | UnityEngine.UI.Button | AncestralLineageButton | (334,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**SettingsButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/SettingsButton` | UnityEngine.UI.Button | SettingsButton | (399,-40) 60x60 piv(0.50,0.50) | yes | SetTab(Stats) | - |

**TopCenter**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/PauseButton` | UnityEngine.UI.Button | PauseButton | (50,-35) 50x50 piv(0.50,0.50) | yes | TogglePause | - |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/PlayButton` | UnityEngine.UI.Button | PlayButton | (110,-35) 50x50 piv(0.50,0.50) | yes | TogglePause | - |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/sliderHolder/BasicSlider/NormalSecton/SliderPanel/Slider` | UnityEngine.UI.Slider | Slider | (0,0) -20x0 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/sliderHolder/BasicSlider/FloatInfoText` | UnityEngine.UI.Button | X 0.00 | (330,-25) 130x35 piv(0.50,0.50) | yes | - | - |

**BibiteButtons**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer1/Panels/BibiteButtons/StatsButton` | UnityEngine.UI.Button | StatsButton | (40,-40) 60x60 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibiteButtons/GeneButton` | UnityEngine.UI.Button | GeneButton | (110,-40) 60x60 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibiteButtons/BiologyButton` | UnityEngine.UI.Button | BiologyButton | (180,-40) 60x60 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibiteButtons/BrainButton` | UnityEngine.UI.Button | BrainButton | (250,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**BibitePanels/GenesPanel**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer1/Panels/BibitePanels/GenesPanel/SpeciesHolder` | UnityEngine.UI.Button | Species: / Basic bibite / ( / +6% / ) | (224,-32) 417x34 piv(0.50,0.50) | yes | - | - |

**Layer1/Right Panels/QuickStatsHolder**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer1/Panels/Right Panels/QuickStatsHolder/OpenButtons/OpenQuickStats` | UnityEngine.UI.Button | Info / + | (0,0) 82x30 piv(1.00,1.00) | yes | SetTab(Stats) | - |

### `default_world_biology`


**PlacerButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/PlacerButton` | UnityEngine.UI.Button | PlacerButton | (35,-35) 50x50 piv(0.50,0.50) | yes | - | - |

**UserActions**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/UserActions` | UnityEngine.UI.Button | UserActions | (95,-35) 50x50 piv(0.50,0.50) | yes | - | - |

**ViewButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/ViewButton` | UnityEngine.UI.Button | ViewButton | (155,-35) 50x50 piv(0.50,0.50) | yes | - | - |

**InformationPanelButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/InformationPanelButton` | UnityEngine.UI.Button | InformationPanelButton | (204,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**DynamicParametersButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/DynamicParametersButton` | UnityEngine.UI.Button | DynamicParametersButton | (269,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**AncestralLineageButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/AncestralLineageButton` | UnityEngine.UI.Button | AncestralLineageButton | (334,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**SettingsButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/SettingsButton` | UnityEngine.UI.Button | SettingsButton | (399,-40) 60x60 piv(0.50,0.50) | yes | SetTab(Stats) | - |

**TopCenter**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/PauseButton` | UnityEngine.UI.Button | PauseButton | (50,-35) 50x50 piv(0.50,0.50) | yes | TogglePause | - |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/PlayButton` | UnityEngine.UI.Button | PlayButton | (110,-35) 50x50 piv(0.50,0.50) | yes | TogglePause | - |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/sliderHolder/BasicSlider/NormalSecton/SliderPanel/Slider` | UnityEngine.UI.Slider | Slider | (0,0) -20x0 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/sliderHolder/BasicSlider/FloatInfoText` | UnityEngine.UI.Button | X 0.00 | (330,-25) 130x35 piv(0.50,0.50) | yes | - | - |

**BibiteButtons**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer1/Panels/BibiteButtons/StatsButton` | UnityEngine.UI.Button | StatsButton | (40,-40) 60x60 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibiteButtons/GeneButton` | UnityEngine.UI.Button | GeneButton | (110,-40) 60x60 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibiteButtons/BiologyButton` | UnityEngine.UI.Button | BiologyButton | (180,-40) 60x60 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibiteButtons/BrainButton` | UnityEngine.UI.Button | BrainButton | (250,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**Layer1/Right Panels/QuickStatsHolder**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer1/Panels/Right Panels/QuickStatsHolder/OpenButtons/OpenQuickStats` | UnityEngine.UI.Button | Info / + | (0,0) 82x30 piv(1.00,1.00) | yes | SetTab(Stats) | - |

### `default_world_brain`


**PlacerButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/PlacerButton` | UnityEngine.UI.Button | PlacerButton | (35,-35) 50x50 piv(0.50,0.50) | yes | - | - |

**UserActions**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/UserActions` | UnityEngine.UI.Button | UserActions | (95,-35) 50x50 piv(0.50,0.50) | yes | - | - |

**ViewButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/ViewButton` | UnityEngine.UI.Button | ViewButton | (155,-35) 50x50 piv(0.50,0.50) | yes | - | - |

**InformationPanelButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/InformationPanelButton` | UnityEngine.UI.Button | InformationPanelButton | (204,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**DynamicParametersButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/DynamicParametersButton` | UnityEngine.UI.Button | DynamicParametersButton | (269,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**AncestralLineageButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/AncestralLineageButton` | UnityEngine.UI.Button | AncestralLineageButton | (334,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**SettingsButton**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/UI Buttons/TopRight/SettingsButton` | UnityEngine.UI.Button | SettingsButton | (399,-40) 60x60 piv(0.50,0.50) | yes | SetTab(Stats) | - |

**TopCenter**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/PauseButton` | UnityEngine.UI.Button | PauseButton | (50,-35) 50x50 piv(0.50,0.50) | yes | TogglePause | - |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/PlayButton` | UnityEngine.UI.Button | PlayButton | (110,-35) 50x50 piv(0.50,0.50) | yes | TogglePause | - |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/sliderHolder/BasicSlider/NormalSecton/SliderPanel/Slider` | UnityEngine.UI.Slider | Slider | (0,0) -20x0 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer0/Controls/TopCenter/sliderHolder/BasicSlider/FloatInfoText` | UnityEngine.UI.Button | X 0.00 | (330,-25) 130x35 piv(0.50,0.50) | yes | - | - |

**BibiteButtons**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer1/Panels/BibiteButtons/StatsButton` | UnityEngine.UI.Button | StatsButton | (40,-40) 60x60 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibiteButtons/GeneButton` | UnityEngine.UI.Button | GeneButton | (110,-40) 60x60 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibiteButtons/BiologyButton` | UnityEngine.UI.Button | BiologyButton | (180,-40) 60x60 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibiteButtons/BrainButton` | UnityEngine.UI.Button | BrainButton | (250,-40) 60x60 piv(0.50,0.50) | yes | - | - |

**BibitePanels/BrainPanel**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer1/Panels/BibitePanels/BrainPanel/Top Section/First Row/SpeciesHolder` | UnityEngine.UI.Button | Species: / Basic bibite / ( / +0% / ) | (134,-17) 267x34 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibitePanels/BrainPanel/Top Section/First Row/BrainHelpButton` | UnityEngine.UI.Button | Help | (660,0) 60x36 piv(1.00,1.00) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibitePanels/BrainPanel/BottomLine/BrainDiffToggle` | UnityEngine.UI.Toggle | Highlight differences | (102,-18) 204x25 piv(0.50,0.50) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibitePanels/BrainPanel/BottomLine/SortButton` | UnityEngine.UI.Button | Sort | (470,0) 80x30 piv(1.00,1.00) | yes | - | - |
| `UICanvas/MainUI/Layer1/Panels/BibitePanels/BrainPanel/BottomLine/ExpandedViewerButton` | UnityEngine.UI.Button | Expanded View | (660,0) 175x30 piv(1.00,1.00) | yes | - | - |

**Layer1/Right Panels/QuickStatsHolder**

| Control path | Type | Label | Captured rect | Drawn | Runtime hit | Menu/action |
| --- | --- | --- | --- | --- | --- | --- |
| `UICanvas/MainUI/Layer1/Panels/Right Panels/QuickStatsHolder/OpenButtons/OpenQuickStats` | UnityEngine.UI.Button | Info / + | (0,0) 82x30 piv(1.00,1.00) | yes | SetTab(Stats) | - |

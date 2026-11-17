# UI/function parity — closure status and limits

> **Superseded (2026-11-03).** The "functional closure" claimed below did not survive a source-level audit: the in-world inspector,
> gene/brain editors, species/lineage panels, graphs and many hotkeys/controls were placeholders or missing relative to the decompiled
> `Here/UIScripts*`. See [`PARITY_AUDIT_UI.md`](PARITY_AUDIT_UI.md) for the verified gap list and what has since been implemented.

**Closure tree:** `8c30538` (final polish). **Gate:** viewer **96/96**, core **457** verified locally.
**Release state:** **0.2.3 released (156,609,848 B / `5e33e6aa856c0b60b5d501d3ba50f9b1f33c818d1414eb8819089e571263e4e4`)**; **0.2.2** (`7158aad`, 155,568,731 B / `f7d98516…`) and
all earlier packages are historical. **Functional closure criteria met; full visual + functional parity is not yet closed** because the visual axis (TMP SDF/menu blur/editor skin) remains open. See [UI_FUNCTIONAL_PARITY §1.19](UI_FUNCTIONAL_PARITY.md#119-conjoint-visual--functional-parity-requirement).

The authoritative panel/action inventory and per-surface statuses are in
[`UI_FUNCTIONAL_PARITY.md`](UI_FUNCTIONAL_PARITY.md) (especially §1.16–§1.18). This page records the closure
decision, the unavoidable limits, and the explicitly out-of-scope items.

## Conjoint requirement

Visual parity and full UI functionality are a **single acceptance target**. Functional coverage/inert controls and visual reference/pixel metrics must be tracked together; neither alone closes the goal. Current state: functional closure met, visual parity still open (SDF/menu blur/editor skin).

## Closed by the final polish

- **Challenge parameters/star editor** — supported timer/star thresholds edit and apply on challenge start
  (`challenge_parameters_editor_adjusts_and_applies_to_start`).
- **Hotkey rebinding** — capture/assign/reset/persist through `UserPreferences`
  (`hotkeys_rebind_persist_and_workshop_edit_status`).
- **Workshop local edit + online status** — local rename/edit status and explicit “Steam/online not
  available” status; Steam Workshop remains out of scope.
- **Earlier 0.2.x closure work** — Settings pages, save dialogs, in-world Dynamics/Settings values, gene
  paging/mutate, scenario editor, egg TAG, editors, challenge tracker/end popup/selector/champion/embedded
  params, Workshop local browser, species/lineage tabs, editor anchors/template save.

All implementable items previously ranked in this document are now either **closed** or listed below as
**unavoidable limits** / **out of scope**. No further implementable panel/action gap is identified.

## Unavoidable limits

| Limit | Why it remains |
|---|---|
| **Exact editor skin / visual fidelity** | The functional editor panels/actions work; matching the game's exact skin/TMP raster fidelity is visual polish, not a functional effect, and is not fully reproducible from captures. |
| **Unavailable game rung history** | The original lineage rung/history data was not captured from the game; exact historical rungs cannot be reconstructed. Current lineage graph/species tabs and captured distributions are the documented limit. |
| **Champion pre-seed ordering** | Champion spawn/start and selection work, but pre-seed ordering was not captured/bound from the game. |
| **Workshop metadata follow-ups** | Local list/export/delete/rename/edit status and explicit online-unavailable status work; deeper Steam metadata/edit controls are follow-ups. Steam Workshop itself is out of scope. |

## Out of scope

| Item | Why out of scope |
|---|---|
| **Steam achievements** unlock/sync/progress | Requires Steamworks SDK/app id/user service; local progress/list is implemented. |
| **Steam Workshop** browse/subscribe/upload/share | Steam service/authentication; local browser/edit status is implemented. |
| **Steam leaderboards / online champions / overlay** | Online identity/service; local challenge scoring/championship is implemented. |
| **Audio / UI sounds** | No audio subsystem; sound does not change panel/dialog/world state and is outside functional UI parity. |
| **Online/multiplayer services** | No server/identity layer; single-player UI scope only. |

## Evidence and next action

- Matrix statuses: [`UI_FUNCTIONAL_PARITY.md`](UI_FUNCTIONAL_PARITY.md) §1.18 (0.2.3 closure tree) and
  §1.17 (0.2.2 historical package).
- **0.2.3 size/SHA/exe hashes are recorded in README/INSTALL/RELEASE_CERTIFICATION/**
  VIEWER_RELEASE_CHECKLIST/COMPATIBILITY, the goal can close with the limits above documented.
- Every “works” item retains the same evidence standard: an executable test or headless click/state
  assertion showing the **game-observable effect**, not merely a hit rectangle or rendered panel.

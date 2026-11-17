# Viewer release checklist

Small, repeatable release-readiness checklist for `bibites-view`. All checks are
headless; do **not** open a GUI window unless a windowed run is explicitly
approved (see `docs/HEADLESS_POLICY.md`).

## 1. Build and gates

```powershell
cargo build --release -p bibites-view
cargo test    -p bibites-view
cargo clippy  -p bibites-view --all-targets -- -D warnings
cargo fmt --check -p bibites-view
```

Expected at this checkpoint: **82/82 viewer tests**, **450 core tests**, clippy/fmt green.

## 2. Release-package payload

Build the package with the viewer already built:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\release\package.ps1 -SkipBuild
```

The portable folder/zip must contain:

- `rustibites-view.exe` (built from the viewer HEAD being released)
- `assets/` (game fonts, UI sprites/sheets, reference data; required for the
  exact runtime HUD/menu and game-art bibites)
- `tools/ui/ui_atlas.json`
- `tools/ui/ui_runtime_atlas.json.gz`
- `tools/ui/ui_runtime_atlas.index.json`

`ui_runtime_atlas.json.gz` must contain **all** runtime sessions used by the
viewer:

```
default_world
default_world_inspector
default_world_genes
default_world_biology
default_world_brain
main_menu_title
main_menu_settings
```

(plus legacy captures such as `main_menu`/`default_world_newgame`, which may be
present but are not required). The package script copies `tools/ui` and
`assets` wholesale, so no extra files are needed beyond rebuilding the viewer
and re-running the package script.

`docs/VIEWER_RELEASE_CHECKLIST.md` is now included in the portable package's
`docs/` folder alongside USER_GUIDE/INSTALL/ARCHITECTURE/DEVIATIONS/PARITY/
INTERFACES (added to the copy list in `tools/release/package.ps1`).

## 3. Fresh-extraction smoke test

Extract the zip to a directory **outside the repository** and run from that
directory (the viewer discovers `tools/ui` relative to the working directory):

```powershell
Expand-Archive target\release-package\RUSTibites-<version>-windows-x64.zip $env:TEMP\rustibites-fresh -Force
Start-Process -FilePath "$env:TEMP\rustibites-fresh\rustibites-view.exe" `
  -WorkingDirectory "$env:TEMP\rustibites-fresh" `
  -ArgumentList '--menu','--screenshot',"$env:TEMP\rustibites-fresh\menu.png",'--width','1920','--height','1080' `
  -Wait
```

Expected: `menu.png` written, stderr reports game assets + `ui: runtime layout
from tools/ui\ui_runtime_atlas.json.gz` and decoded runtime sprites (36 at this
checkpoint). Because release builds use the Windows GUI subsystem, scripts must
use `Start-Process -Wait`; a direct `&` invocation returns before the file is
written.

For a tab smoke test, copy a save into the extraction's `testdata/parity/default/`
and repeat with `--tab genes --select-first --ui-only ...`; the packaged tab
sessions should render (Genes overlay SHA-256 `23F3566B...` for
`rust_seed_600.zip` at the canonical camera).

**0.2.3 functional-UI release (2026-11-03, commit `c1d25e0`):** `RUSTibites-0.2.3-windows-x64.zip`
**156,609,848 B** (149.35 MB), SHA-256 `5e33e6aa856c0b60b5d501d3ba50f9b1f33c818d1414eb8819089e571263e4e4`.
Packaged `rustibites.exe` **2,253,824 B**, SHA-256
`BC5F2910C9B24D3F55D2C4C6F59799E06510BC09FA38A422C278C19F6013E823`; packaged
`rustibites-view.exe` **3,205,120 B**, SHA-256
`73EE956A23174F01CB707DA467CBE8435A0260F125E8B19F826ED2994CE61858`.
Adds challenge parameters/star-description editor, persisted hotkey rebinding, Workshop edit statuses and
the 0.2.2 challenge/Workshop/species/lineage features plus 0.2.1 Settings/save-dialog/in-world values.
Gate at package HEAD: **viewer 96/96, core/io/cli green, clippy/fmt clean**.
Fresh-extraction smoke passed: menu deterministic (`E533D78A...`), packaged CLI Default 600 exit 0
(1→10), and **60** runtime sessions in the packaged atlas.

**Historical 0.2.2 functional-UI release (2026-11-03, commit `7158aad`):** `RUSTibites-0.2.2-windows-x64.zip`
**155,568,731 B** (148.36 MB), SHA-256 `f7d9851638719847e20dd05db3f7c8e62026247a189f9ecf87e98048ee74b36c`.
Packaged `rustibites.exe` **2,253,824 B**, SHA-256
`898A218A91779B5556EC534170390733E30A1217D2910E27FDA7E5BCCAFADB4C`; packaged
`rustibites-view.exe` **3,168,768 B**, SHA-256
`CA2A6D4684218C5CE65B767B6C3291F359E988BAD2C776DABD3BFEBD36F42B50`.
Adds challenge tracker/end popup, challenge selector/champion picker + embedded params, Workshop local
browser, species/lineage tabs, plus all 0.2.1 Settings/save-dialog/in-world value features.
Gate at package HEAD: **viewer 94/94, core 457, io/cli green, clippy/fmt clean**.
Fresh-extraction smoke passed: menu deterministic (`E533D78A...`), packaged CLI Default 600 exit 0
(1→10), and **60** runtime sessions in the packaged atlas.
Deferred: star-description/challenge-params editor, pre-seed champion ordering, exact editor skin, hotkey
rebinding, Workshop edit controls, exact rung data; Steam/audio/online out of scope.

**Historical 0.2.1 functional-UI release (2026-11-03, commit `86239a8`):** `RUSTibites-0.2.1-windows-x64.zip`
**134,539,435 B** (128.31 MB), SHA-256 `55fdfce6df3fcc897132c32e8b0cbaf18f287c2a8283fca2c3ab9e0089ae7fd4`.
Packaged `rustibites.exe` **2,253,312 B**, SHA-256
`7E00391CBCFF0006650110FC093CB6706385090B58E0C7FDF6AB37B3095D228C`; packaged
`rustibites-view.exe` **3,081,728 B**, SHA-256
`AC756621BEFE418DE1DBCB091C8C8881986E8CBA5C86F6D59700F4C3DFEAEFFE`.
Adds Settings pages with persisted prefs, destructive save dialogs, in-world Dynamics/Settings value
binding, gene paging, mutate wiring, scenario editor/egg TAG and basic gene/brain editors.
Gate at package HEAD: **viewer 87/87, core 453, io/cli green, clippy/fmt clean**.
Fresh-extraction smoke passed: menu `--menu --screenshot` deterministic at 1920x1080
(SHA-256 `E533D78AC5FA41471D17FB01DF3ED80AFC66263127AA62709978F17CAC970DF5`, byte-identical repeat),
packaged CLI `run testdata/real/Default.zip --ticks 600 --seed 12345 --stats json --quiet` exit 0
(1→10 bibites), and the packaged runtime atlas contains all **49** captured sessions.
Remaining: editor template save/continue, exact editor skin, node drag/anchors, exact lineage
rung/distribution, secondary panels/challenges; Steam/audio out of scope.

**Historical 0.2.0 functional-UI release (2026-11-03, commit `603646b`):** `RUSTibites-0.2.0-windows-x64.zip`
**124,051,485 B** (118.31 MB), SHA-256 `a6cc1396a126cdd5ecb89bbf990c0244b782d250dec57589806209f01b895341`.
Packaged `rustibites.exe` **2,252,288 B**, SHA-256
`D00B7CBA5B9E2F8CA99BDDB9D498547CC98E25A6BAF3343AA12828A38EDFBC0A`; packaged
`rustibites-view.exe` **3,036,672 B**, SHA-256
`D458538D6DAFD18C20D64210EEC59D584D90017C1CF651DD3ADA10B87202CB52`.
Fresh-extraction smoke passed: menu `--menu --screenshot` deterministic at 1920x1080
(SHA-256 `E533D78AC5FA41471D17FB01DF3ED80AFC66263127AA62709978F17CAC970DF5`, byte-identical repeat),
packaged CLI `run testdata/real/Default.zip --ticks 600 --seed 12345 --stats json --quiet` exit 0
(1→10 bibites), and the packaged runtime atlas contains all 44 captured sessions.
Packaged docs may lag the external zip hash because packaging embeds them before the final self-referential
hash update; `SHA256SUMS.txt`/this table are authoritative. The 0.1.0 artifact remains frozen historical.

0.2.0 historical gate checks:

- Full workspace gate green: viewer **82/82**, core **450**, io/cli green, clippy/fmt clean at the package
  HEAD.
- All 9 title-menu rows navigate; Library selector works; New Game scenario picker + option registration;
  dynamic save/load browser rows and actions; lineage graph panel; KILL/KILL-half confirmation dialog;
  tool-property panels; New Game option behavior/generation; egg TAG inspector; scenario-editor core
  (settings working copy/apply/save, zones); gene editor (edit/select/randomize/mutate); brain editor
  (add/remove node/synapse + validate).
- Deterministic headless `--menu --screenshot` and fresh-extraction smoke test against the packaged exes.
- **Deferred polish:** gene scroll >14, exact editor skin, node drag/anchors, mutate-settings wiring,
  template save/continue, exact lineage rung data, challenges/Steam, TAG beyond the egg inspector.

**0.2.1 is the current historical release; 0.2.0 remains a frozen historical package.**

**Historical 0.1.0 freeze package (2026-11-01):** **38,496,196 B**, SHA-256
`b201c1b73631f77f04d6032a6fad07ebb5bbcce7678ea03e8e8df3bb310d8479`
(`SHA256SUMS.txt` matches). Packaged `rustibites.exe`
`818B759504E2172108459334CAE79668120BA1FEE924E5A4C756B819EE43B858`,
`rustibites-view.exe`
`3AA23B129B818A8B69C4D596E577C7B93E8E71B260F3C78DDF52E4C23389ED15`.
Interim hash `92bd1a9a...` is retired.

## 4. Deterministic headless captures

```powershell
# Menu (twice, compare SHA-256)
--menu --screenshot target\menu-1.png --width 1920 --height 1080
--menu --screenshot target\menu-2.png --width 1920 --height 1080

# Inspector tabs (twice each; rust_seed_600 paused, canonical selected bibite)
<save> --ui-only --screenshot target\tab.png --width 1920 --height 1080 `
  --frames 0 --center 905.651062,-556.5388 --ortho-size 400 `
  --select-first --tab genes|biology|brain --click 739,34
```

Observed at this checkpoint (byte-identical repeats):

| Capture | SHA-256 |
|---|---|
| menu | `E533D78AC5FA41471D17FB01DF3ED80AFC66263127AA62709978F17CAC970DF5` |
| genes | `23F3566B7C02F6488F254C8C3391653087C74D285437DC9C4AFDEC4267204EA8` |
| biology | `BC8B327A986DA5CDCCBF7CCEBF685DF265A131499669876CBCFA90B22F7BEDF4` |
| brain | `6AE896EBDBE71F49FCFA170396857437D7ACFA48BB78128A63182004BE57376E` |

Canonical docs images are generated with the same binary/camera settings and
kept in `docs/images/viewer*.png` in the private working copy (closeup/population/zone/world,
Genes/Biology/Brain tabs, title menu); they are not in the public repository (they show the game's art).

## 5. 0.2.0 build procedure (performed 2026-11-03)

The 0.1.0 freeze package above is historical. The 0.2.0 package was built as follows:

1. Full workspace gate green at package HEAD `603646b` (viewer 82/82, core 450, io/cli green,
   clippy/fmt clean); version bumped to `0.2.0` in `Cargo.toml`/`Cargo.lock` before packaging.
2. Rebuilt **both** release exes from that commit:
   `cargo build --release -p bibites-cli -p bibites-view`.
3. Ran `powershell -NoProfile -ExecutionPolicy Bypass -File
   tools\release\package.ps1` (no `-SkipBuild`); packaged
   `docs/VIEWER_RELEASE_CHECKLIST.md` is present.
4. Repeated the fresh-extraction smoke test in section 3 and the deterministic
   menu capture in section 4 against the packaged exes.
5. Recorded the zip size/SHA-256 and packaged exe hashes above; the interim
   `92bd1a9a…` hash is retired. `docs/INSTALL.md`, certification and README are
   updated from this checklist's values by the spec/release owner.

## 5b. X32 thread-pool status (in 0.2.0 candidate)

X32 (lazy pool + serial fallback below gates, `1339d35`) is **PASS** on the
Lead's idle definitive thread A/B (`tools/bench/results/x32-lead-ab/`):
5k 4t/8t 0.981/1.000x serial; 10k 1.000/0.958x; 20k 0.955/1.080x; 4-5% noise
band, 8t helps 20k, fingerprints deterministic, 20k gated paths exercised.
This evidence is carried by the 0.2.0 candidate; the historical 0.1.0 package
remains frozen at `b201c1b7...` and is not rebuilt.

## 6. Known remaining rendering deltas

These are accepted deltas, documented in `docs/VIEWER.md`; they are not
release blockers for the current fan-build:

- speed-label glyph **thickness/anti-aliasing** (playback subregion IoU 0.7935;
  overall playback 0.9680, target met)
- inspector tab residual 0.9815–0.9887 (small TMP text/icons, not geometry)
- software tail approximation (3,128 vs 3,081 px; width +12) and no macroquad
  GPU tail mesh
- exact menu layout with a dark backdrop where the hidden-lab capture exposed no
  menu background art
- the window's GPU skin shader was checked in a real window on 2026-11-05 (the basic
  bibite's grey levels and cyan eyes match the game's capture); the shader's special effects
  (virus, rainbow, producer, reaper, hurt flash) are not modelled

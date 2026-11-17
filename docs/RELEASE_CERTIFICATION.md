# Release certification — RUSTibites 0.2.3 (0.2.2/0.2.1/0.2.0/0.1.0 historical)

Certified from a **fresh extraction** of the release zip (not the development build). All commands run
headless/CLI; no game window, no game/plugin process.

## 0.2.3 functional-UI package — verified (authoritative)

| field | value |
|---|---|
| artifact | `target/release-package/RUSTibites-0.2.3-windows-x64.zip` |
| size | 156,609,848 bytes (149.35 MB) |
| SHA-256 | `5e33e6aa856c0b60b5d501d3ba50f9b1f33c818d1414eb8819089e571263e4e4` (matches `SHA256SUMS.txt`) |
| packaged VERSION | RUSTibites 0.2.3 (windows-x64, portable), snapshot `8c30538` (package commit `c1d25e0`) |
| `rustibites.exe` | 2,253,824 B, SHA-256 `BC5F2910C9B24D3F55D2C4C6F59799E06510BC09FA38A422C278C19F6013E823` |
| `rustibites-view.exe` | 3,205,120 B, SHA-256 `73EE956A23174F01CB707DA467CBE8435A0260F125E8B19F826ED2994CE61858` |

Adds challenge parameters/star editor, hotkey rebinding, Workshop local edit/online-status, plus 0.2.2 features.
Gate at package HEAD: **viewer 96/96**, core/io/cli green, clippy/fmt clean. Fresh extraction: packaged menu
deterministic (`E533D78A…`), packaged CLI Default600 exit0 (1→10), **60 runtime sessions**. Packaged docs may
lag the external hash; `SHA256SUMS.txt`/this table authoritative. **Closure criteria met.** Unavoidable limits:
exact editor skin, unavailable rung history, champion pre-seed ordering, Workshop metadata follow-ups;
Steam/audio/online out of scope.

## 0.2.2 functional-UI package — verified (historical)

| field | value |
|---|---|
| artifact | `target/release-package/RUSTibites-0.2.2-windows-x64.zip` |
| size | 155,568,731 bytes (148.36 MB) |
| SHA-256 | `f7d9851638719847e20dd05db3f7c8e62026247a189f9ecf87e98048ee74b36c` (matches `SHA256SUMS.txt`) |
| packaged VERSION | RUSTibites 0.2.2 (windows-x64, portable), snapshot commit `7158aad` |
| `rustibites.exe` | 2,253,824 B, SHA-256 `898A218A91779B5556EC534170390733E30A1217D2910E27FDA7E5BCCAFADB4C` |
| `rustibites-view.exe` | 3,168,768 B, SHA-256 `CA2A6D4684218C5CE65B767B6C3291F359E988BAD2C776DABD3BFEBD36F42B50` |

Adds challenge tracker/end popup, challenge selector/champion picker + embedded params, Workshop local
browser, species/lineage tabs, editor anchors/template save, plus 0.2.1 features. Gate at package HEAD:
**viewer 94/94, core 457, io/cli green, clippy/fmt clean**. Packaged menu deterministic (`E533D78A…`),
packaged CLI Default600 exit0 (1→10), 60 sessions. Packaged docs may lag the external hash;
`SHA256SUMS.txt`/this table authoritative. Final polish: hotkey rebinding, Workshop edit controls, challenge
star/params editor, pre-seed champion ordering; exact editor skin and unavailable rung history documented as
limits. Steam/audio/online out of scope.

## 0.2.1 functional-UI package — verified (historical)

| field | value |
|---|---|
| artifact | `target/release-package/RUSTibites-0.2.1-windows-x64.zip` |
| size | 134,539,435 bytes (128.31 MB) |
| SHA-256 | `55fdfce6df3fcc897132c32e8b0cbaf18f287c2a8283fca2c3ab9e0089ae7fd4` (matches `SHA256SUMS.txt`) |
| packaged VERSION | RUSTibites 0.2.1 (windows-x64, portable), snapshot commit `86239a8`, built 2026-11-03T07:08:26Z, rustc 1.99.1 |
| `rustibites.exe` | 2,253,312 B, SHA-256 `7E00391CBCFF0006650110FC093CB6706385090B58E0C7FDF6AB37B3095D228C` |
| `rustibites-view.exe` | 3,081,728 B, SHA-256 `AC756621BEFE418DE1DBCB091C8C8881986E8CBA5C86F6D59700F4C3DFEAEFFE` |

Adds Settings pages with persisted prefs, save overwrite/delete dialogs, in-world Dynamics/Settings value
binding, gene paging, mutate wiring, scenario editor/egg TAG and basic gene/brain editors. Gate at package
HEAD: **viewer 87/87, core 453, io/cli green, clippy/fmt clean**. Fresh-extraction smoke: menu
`--menu --screenshot` deterministic (`E533D78A…`), packaged CLI Default 600 t exits 0 (1→10). Packaged docs
may lag the external zip hash; `SHA256SUMS.txt`/this table are authoritative. Still deferred: editor template
save/continue, exact editor skin, node drag/anchors, exact lineage rung/distribution, secondary
panels/challenges; Steam/audio out of scope.

## 0.2.0 functional-UI package — verified (historical)

| field | value |
|---|---|
| artifact | `target/release-package/RUSTibites-0.2.0-windows-x64.zip` |
| size | 124,051,485 bytes (118.31 MB) |
| SHA-256 | `a6cc1396a126cdd5ecb89bbf990c0244b782d250dec57589806209f01b895341` (matches `SHA256SUMS.txt`) |
| packaged VERSION | RUSTibites 0.2.0 (windows-x64, portable), snapshot commit `603646b`, built 2026-11-03T05:50:33Z, rustc 1.99.1 |
| `rustibites.exe` | 2,252,288 B, SHA-256 `D00B7CBA5B9E2F8CA99BDDB9D498547CC98E25A6BAF3343AA12828A38EDFBC0A` |
| `rustibites-view.exe` | 3,036,672 B, SHA-256 `D458538D6DAFD18C20D64210EEC59D584D90017C1CF651DD3ADA10B87202CB52` |

**Independent verification:** Lead re-extracted the package; packaged `--menu --screenshot` hashes
`E533D78A…`, and packaged CLI `run testdata/real/Default.zip --ticks 600 --seed 12345` exits 0 with 1→10
bibites. **Packaged docs may lag the external zip hash** because packaging embeds docs before the final
self-referential hash update; `SHA256SUMS.txt`/this table are authoritative.

**0.2.0 deferred polish:** gene scroll >14, exact editor skin, node drag/anchors, mutate-settings wiring,
template save/continue, exact lineage rung data, challenges/Steam, TAG beyond the egg inspector.

**0.2.2 package:** historical verified package above; **0.2.3 verified package above**.

## 0.1.0 certified artifact (historical)

| field | value |
|---|---|
| artifact | `target/release-package/RUSTibites-0.1.0-windows-x64.zip` |
| size | 38,496,196 bytes |
| SHA-256 | `b201c1b73631f77f04d6032a6fad07ebb5bbcce7678ea03e8e8df3bb310d8479` (matches `SHA256SUMS.txt`) |
| packaged VERSION | RUSTibites 0.1.0 (windows-x64, portable), commit `903f1b2`, built 2026-11-01T10:49:49Z, rustc 1.99.1 |
| `rustibites.exe` SHA-256 | `818b759504e2172108459334cae79668120ba1fee924e5a4c756b819ee43b858` |
| `rustibites-view.exe` SHA-256 | `3aa23b129b818a8b69c4d596e577c7b93e8e71b260f3c78ddf52e4c23389ed15` |

Extracted to a clean directory; all below commands use the extracted `rustibites.exe` /
`rustibites-view.exe`.

## Packaged CLI — canonical Default

| test | command | result |
|---|---|---|
| Default 600 t | `rustibites.exe run testdata/real/Default.zip --ticks 600 --seed 12345 --stats json --quiet` | exit 0; 1→10 bibites; `state_fingerprint=5821404756245751725` |
| Default 600 t capture | `run testdata/real/Default.zip --ticks 600 --seed 12345 --capture <dir> --quiet` | exit 0; `counts_end.bibites=10`; `plant_n=447`; `vitals.energy_mean=115.4122`; conservation fields present: `produced_plant_energy=1679.472`, `consumed_plant_energy=218.0879`, `stock_plant_energy=95771.192`, meat/decay/recycle/cull/removed all 0.0, `terms_missing=true` (Default materials have decay disabled; expected) |
| Default 3000 t | `run testdata/real/Default.zip --ticks 3000 --seed 12345 --stats json --quiet` | exit 0; 1→10; `state_fingerprint=9684289852194376735` |
| Default 3000 t capture | `run ... --ticks 3000 --capture <dir>` | exit 0; `counts_end.bibites=10`; `plant_n=466`; `energy_mean=116.4984`; `produced_plant_energy=9607.790`, `consumed_plant_energy=4717.888`, `stock_plant_energy=99199.710` |

## `--save` + checkpoint sidecar resume

| test | command | result |
|---|---|---|
| single 600 t | `run Default.zip --ticks 600 --seed 12345 --stats json --quiet` | `state_fingerprint=5821404756245751725` |
| first half | `run Default.zip --ticks 300 --seed 12345 --save half.zip --stats json --quiet` | `state_fingerprint=18427327640304057206`; wrote `half.zip` (43,313 B) + `half.zip.bb8state` (177,771 B) |
| resume second half | `run half.zip --ticks 300 --seed 12345 --stats json --quiet` | `state_fingerprint=5821404756245751725` — **bit-exact equal to single 600 t** |

## Feeding / contact parity vs game §10.4.1

Packaged Rust capture on the game-written identical-base fixture
(`testdata/parity/default/game_scenario_seed_600_nospawn.zip`), compared with the game captures in §10.4.1:

| metric | game 600 | package Rust 600 | game 3000 | package Rust 3000 |
|---|---|---|---|---|
| contacters | 8/10 | **8/10** | 8/10 | **8/10** |
| contact ticks | 2998 | 3103 | 10774 | 11741 |
| enter events | 10 | 10 | 45 | 26 |
| fed bibites | 8 | **8** | 8 | **8** |
| intake events | 68 | 71 | 249 | 263 |
| consumed u² | 216.36 | 230.24 (+6.4%) | 822.95 | 868.33 (+5.5%) |
| u² per bite | 3.182 | 3.243 | 3.305 | 3.302 |
| median idle gap | 7 | 7 | 9 | 9 |
| first intake tick | 6 | 4 | 4 | 4 |

This reproduces the §10.4.1 official dev-build numbers exactly (contacters 8/10, fed 8/8, intake 68/71 and
249/263, consumed +5–6%). No packaged-vs-dev deviation was observed.

## Packaged viewer (headless screenshots)

Commands (from the extracted package root, so `assets/game` and `tools/ui` resolve):

```
rustibites-view.exe testdata/real/Default.zip --screenshot out-assets.png --frames 60 --width 1920 --height 1080 --runtime-ui
rustibites-view.exe testdata/real/Default.zip --screenshot out-noassets.png --frames 60 --width 1920 --height 1080 --no-assets
```

| test | result |
|---|---|
| with assets | 1920×1080 PNG; log: `game assets: 33 atlases, 4 bibite size tiers`, `ui: runtime layout from tools/ui/ui_runtime_atlas.json.gz (7874 nodes ...)`, `runtime panel model: 8 core panels`, `runtime inspector session loaded`; SHA-256 `a7560f2ce839f98dbe47315d1513d9d65f8b25930766fc1c58a0b9b06d4242ee` |
| with assets, second run | identical SHA-256 `500bfb6d…` (deterministic) |
| `--no-assets` | procedural fallback renderer; runtime HUD still loaded (`ui: runtime layout ... 7874 nodes`); 1920×1080 PNG SHA-256 `b3862477f333d0c1a637945b5fc0b383319f7766eab19880c0b0c3a27c113bfa` |
| `--no-assets`, second run | identical SHA-256 `b25a4173…` (deterministic) |

## Deviations / caveats

- **0.2.2 package hashes** are historical; **0.2.3 values are authoritative above**. 0.2.1/0.2.0/0.1.0 values below are
  historical. The `docs/INSTALL.md` embedded inside the zip is captured by the packaging pass
  before the final self-referential zip-hash update, so its embedded hash can lag the final zip hash; the
  external values above are authoritative (bench-dev task-131 note; artifact unchanged).
- The 0.1.0 package was built at VERSION commit `903f1b2`; the certification ran only the packaged binaries and
  they reproduce the current dev §10.4.1 results exactly. No packaged-vs-dev behavioural deviation found.
- Default conservation `terms_missing=true` is expected: Default material decay is disabled, so
  decayed/recycled/culled/removed are exactly 0.0; produced/consumed/stock terms are present and populated.
- The packaged viewer without `--assets` uses the procedural fallback for in-world sprites but still loads
  the runtime HUD from the packaged `tools/ui` data (as designed); screenshots are deterministic in both
  modes.
- Zip SHA-256 matches the shipped `SHA256SUMS.txt`; installer is portable, no dependencies/registry writes
  (see `docs/INSTALL.md`).

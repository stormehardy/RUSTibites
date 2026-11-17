# Compatibility matrix: real The Bibites artifacts

This document records what RUSTibites can load and round-trip from real game data,
measured empirically against every artifact on the reference machine. The matrix
is produced by [`tools/compat/scan.py`](../tools/compat/scan.py); raw results are
written under `target/compat/` (gitignored, temp-only).

The game is **never launched** and the source roots are opened **read-only**.
Every write (round-trip saves, reports) goes under `target/compat/`.

**Final status (2026-10-30):** compatibility matrix complete on the reference
machine — **2,443 artifacts / 2,429 unique / 4.29 GB** discovered, 342 unique
artifacts tested in the representative run (all repo fixtures + stratified
samples), modern 0.6.0–0.6.4 flat formats load and semantically round-trip; one
known scenario pellet/seed round-trip gap remains documented in the unexplained
section below. **Current 0.2.2 artifact:** **`RUSTibites-0.2.2-windows-x64.zip`**, 148.36 MB (155,568,731 bytes), SHA-256 `f7d9851638719847e20dd05db3f7c8e62026247a189f9ecf87e98048ee74b36c`.
**Historical 0.2.1 artifact:** **`RUSTibites-0.2.1-windows-x64.zip`**, 128.31 MB
(134,539,435 bytes), SHA-256 `55fdfce6df3fcc897132c32e8b0cbaf18f287c2a8283fca2c3ab9e0089ae7fd4`.
**Historical 0.2.0 artifact:** **`RUSTibites-0.2.0-windows-x64.zip`**, 118.31 MB
(124,051,485 bytes), SHA-256 `a6cc1396a126cdd5ecb89bbf990c0244b782d250dec57589806209f01b895341`.
Packaged docs may lag this external hash; `SHA256SUMS.txt` is authoritative.
**Historical 0.1.0:** `RUSTibites-0.1.0-windows-x64.zip`, 36.71 MB (38,496,196 bytes), SHA-256
`b201c1b7…` (full hash in `INSTALL.md`). Checkpoint/resume is closed for the **1k–100k** scale
(single == chunked; see `design/CHECKPOINT_STATE.md`).

**Current package:** **`RUSTibites-0.2.2-windows-x64.zip`** adds challenge tracker/end popup/selector/champion picker + embedded params, Workshop local browser, species/lineage tabs, editor anchors/template save, plus 0.2.1 features. **0.2.1, 0.2.0 and 0.1.0 are historical.**

**Settings audit (2026-10-30):** all **78 scenario + 14 independent + 36 material**
settings round-trip exactly through both save- and scenario-shaped
`.bb8settings`; bridge `/settings` key sets/types match the game; defaults and
migration behaviour are unchanged, so the frozen package's defaults are
unaffected. Rust-written saves remain game-loadable; `.bb8state` is a sibling
sidecar and is never written inside the game zip
(`crates/bibites-io/tests/settings_audit.rs`).

**Section fidelity matrix:** see [`SAVE_COVERAGE.md`](SAVE_COVERAGE.md) for the
per-section load->save->reload matrix, the deliberately non-modelled game
fields, and the version rules (newer than 0.6.3.1 and 0.6.4a1 folder saves are
rejected as `Unsupported`).

**Release:** **0.2.2** (released) adds challenge tracker/end popup/selector/champion picker + embedded params,
Workshop local browser, species/lineage tabs, editor anchors/template save, plus 0.2.1 features;
0.2.1/0.2.0/0.1.0 are historical. **Final polish:** star-description/challenge-params editor, pre-seed
champion ordering, hotkey rebinding, Workshop edit controls. **Documented limits:** exact editor skin,
unavailable game rung history; Steam/audio/online out of scope. Load/round-trip compatibility and format
rules are unchanged; the 0.2.2 hash above is authoritative.

### Loaded-save brain migration rule (version-0.6 saves)

When a version-0.6 save's brain migration adds/shifts nodes, the game
**re-executes non-input nodes after load**; cached runtime values from the
pre-migration layout are not valid. Rust must reinitialize runtime states only
when migration actually happens (fix `ed03856`), keeping no stale shifted values.

Concrete example: migrated `Digestion` must read its `Want2Eat` input through the
migrated node's re-executed output — **sigmoid `0.7734683`** — not the stale
pre-migration tanh `0.841998`; a regression asserts the `0.7734683` value.
Modern flat saves that need no migration are unaffected.

## How to reproduce

```powershell
cargo build --release --bin rustibites

# Fast, representative matrix (this document's numbers; ~1-2 min):
python tools/compat/scan.py --no-build --jobs 4 --sample-per-group 40

# Full metadata for every artifact (still sampled per kind/version for CLI work):
python tools/compat/scan.py --no-build --jobs 4 --sample-per-group 0
```

Common options:

| Option | Meaning |
|---|---|
| `--root <dir>` | extra read-only source root (repeatable; defaults: `testdata/real`, `%USERPROFILE%\AppData\LocalLow\The Bibites`, common Steam install) |
| `--limit N` | test only N unique artifacts (metadata for all) |
| `--sample-per-group N` | test up to N unique artifacts per `(kind, version-bucket)`; affects CLI load/round-trip columns only |
| `--max-roundtrip-bytes N` | skip save/reload above N bytes (default 8 MiB; avoids multi-GB autosave churn) |
| `--jobs N` | parallel CLI calls (default 8; use 4 on busy machines) |
| `--keep-temp` | keep `target/compat/roundtrip/` outputs for inspection |
| `--no-build` | use `target/release/rustibites.exe` as-is |

The script copies the CLI to `target/compat/rustibites.exe` before testing so a
concurrent rebuild in `target/release/` cannot race the scan.

Outputs:

- `target/compat/report.json` — full per-artifact rows + totals/buckets.
- `target/compat/report.csv` — the same rows for spreadsheet/diff use.
- `target/compat/unexpected.txt` — failures that are **not** explained by
  `docs/DEVIATIONS.md`.

## Method

For every artifact:

- **Type/version/size/SHA-256** are read directly from the file (zip sections
  `scene.bb8scene`, `scenario.info`, `sim.info`, or the `.bb8`/`.bb8template`
  top-level `version`). Version buckets are `0.5.x`, `0.6.0-0.6.2`, `0.6.3.x`,
  `0.6.4+`, `pre-0.6a1 legacy/game data`, `unknown`.
- **Load** = `rustibites check <file>` exit 0. For genomes/settings `check` also
  performs the structural/serialize→parse round-trip.
- **Round-trip** for `.zip` saves/scenarios = `run <file> --ticks 0 --seed 1
  --stats json --save OUT`, then reload `OUT` (`check`) and run it again to
  compare `state_fingerprint`. Equal fingerprints = modeled state survived
  load→write→reload. Zip bytes are intentionally not byte-stable (JSON field
  order/image/timestamps), so the fingerprint is the semantic check.
- Files extracted from save folders (`.../saves/.../bibites/*.bb8`,
  `.../eggs/*.bb8`) are marked **save-entity / not-applicable**: they are not
  standalone game artifacts and have no own loader.
- Duplicate artifacts are deduplicated by SHA-256; coverage counts keep both the
  file count and the tested-unique count.

The scan is stratified for user libraries with thousands of autosaves: **all**
artifacts get metadata, all `testdata/real` fixtures are tested, then up to
`--sample-per-group` unique artifacts per `(kind, bucket)` get CLI load/round-trip
work. The raw `report.json` therefore has three states for untested sampled rows:
`load_ok=null`, `roundtrip=not-run`.

## Coverage (reference machine, 2026-10-30)

Roots found:

| Root | Artifacts |
|---|---|
| `testdata/real` (repo fixtures) | 29 |
| `%USERPROFILE%\AppData\LocalLow\The Bibites` | 2,414 |
| Steam install (`Program Files (x86)\...\The Bibites`) | 0 loadable artifacts |

Totals: **2,443 artifacts / 2,429 unique / 4.29 GB**, 28 duplicates.
Tested in the representative run: **342 unique artifacts** (all repo fixtures +
up to 40 per kind/version bucket).

| Type | Files | Tested | Load OK | Load fail (known) | Round-trip OK |
|---|---:|---:|---:|---:|---:|
| `.zip` save | 1,820 | 115 | 85 | 30 pre-0.6a1 | 85 |
| `.zip` scenario | 48 | 48 | 44 | 4 pre-0.6a1 | 42 |
| `.bb8` living genome | 70 | 52 | 10 | 42 pre-0.6a1 | 10 |
| `.bb8template` | 249 | 87 | 83 | 4 pre-0.6a1 | 83 |
| `.bb8` save-entity (internal) | 245 | 0 (40 n/a sampled) | n/a | n/a | n/a |
| `.bb8settings` | 11 | 11 | 11 | 0 | 11 |

(Load results are folded across duplicate copies; 313 artifact rows have a load
result out of 2,443, plus 2,130 not-applicable/untested sampled rows — all files
still have metadata. The round-trip count is lower because one scenario fails.)

Grouped by format/version:

| Kind | Version bucket | Files | Tested | Load OK | Load fail | RT OK |
|---|---|---|---:|---:|---:|---:|
| save | 0.6.3.x | 1,424 | 44 | 44 | 0 | 44 |
| save | 0.6.4+ | 365 | 40 | 40 | 0 | 40 |
| save | 0.5.x | 24 | 24 | 1 | 23 (D14) | 1 |
| save | pre-0.6a1 game data | 7 | 7 | 0 | 7 (D14) | 0 |
| scenario | 0.6.3.x | 23 | 23 | 23 | 0 | 21 |
| scenario | 0.6.0-0.6.2 | 16 | 16 | 16 | 0 | 16 |
| scenario | 0.5.x | 7 | 7 | 3 | 4 (D14) | 3 |
| scenario | 0.6.4+ | 2 | 2 | 2 | 0 | 2 |
| genome | 0.6.3.x / 0.6.4+ | 8 | 8 | 8 | 0 | 8 |
| genome | 0.6.0-0.6.2 | 2 | 2 | 2 | 0 | 2 |
| genome | 0.5.x | 60 | 42 | 0 | 42 (D14) | 0 |
| template | 0.6.3.x | 211 | 49 | 49 | 0 | 49 |
| template | 0.6.0-0.6.2 | 24 | 24 | 24 | 0 | 24 |
| template | pre-0.6a1 legacy | 14 | 14 | 10 | 4 (D14) | 10 |
| settings | n/a | 11 | 11 | 11 | 0 | 11 |

Coverage summary:

- **Modern flat saves/scenarios/templates/settings (.bb8settings)** load and
  semantically round-trip: 0.6.0–0.6.4 all pass in the sampled set.
- The observed 0.6.4a1 files are flat scene-bundle zips and load fine; the D13
  unsupported case is specifically the **folder-save layout**
  (`sim.info` + `saves/_00.zip`), which does not appear as a standalone `.zip`
  artifact in this user library (only extracted internal
  `.../saves/.../bibites|eggs/*.bb8`, which are not standalone artifacts).
- Every failure except one is the documented pre-0.6a1 migration floor (D14):
  old 28-gene templates/genomes and 0.4/0.5-era saves return a clear
  `unsupported: game version ... older than ... migration floor` error.

## Known-unsupported vs unexplained

Known-unsupported (never silently corrupts; clear error):

| Case | Status | Reference |
|---|---|---|
| pre-0.6a1 genomes / 28-gene templates | `Unsupported` migration-floor error | DEVIATIONS **D14** |
| 0.4/0.5-era saves and scenarios | `Unsupported` migration-floor error | DEVIATIONS **D14** |
| 0.6.3.3+/0.6.4a1 folder-save layout | `Unsupported`/not a loadable scenario/save | DEVIATIONS **D13** |
| corrupt/truncated zip | parse error with file name | classified as known corrupt |
| save-folder internal `bibites/*.bb8`, `eggs/*.bb8` | not standalone artifacts; no standalone loader | methodology |

**Unexplained (1 artifact, duplicate of the same file):**

- `testdata/real/RadTower.zip` (and the identical AppData `Scenarios\RadTower.zip`)
  parses and runs from the scenario, but scenario→save→reload changes
  `state_fingerprint` (`1188015547238468980 -> 8424153768765744504`). Since the
  digest covers live bibites/pellets/eggs, this is a pellet/seed round-trip gap,
  not tower state. Repro:

  ```powershell
  target\release\rustibites.exe run testdata\real\RadTower.zip --ticks 0 --seed 1 --stats json --save target\compat\rt\rad1.zip --quiet
  target\release\rustibites.exe run target\compat\rt\rad1.zip --ticks 0 --seed 1 --stats json --save target\compat\rt\rad2.zip --quiet
  ```

  Reported to fmt-dev (io owner). No fixture was copied into
  `crates/bibites-io/tests/fixtures/` by this scan (that is outside its write
  scope); the tiny `testdata/real/RadTower.zip` is the repro input.

No other unexpected load or round-trip failures were found in the sampled set.

## Limitations

- `state_fingerprint` checks modeled entity state (live bibite slot/gen,
  position, velocity, energy, health; pellet position/amount; egg ids) — not
  towers, zones, settings, brain runtime or phero spots. A green round-trip here
  does not prove byte-identical saves; separate io tests cover scene/tower/species
  equality.
- The scan samples CLI work for large user libraries (thousands of autosaves);
  metadata covers 100% of files. Run with `--sample-per-group 0` for full CLI
  coverage (slow, tens of minutes; 80 MB autosaves are skipped for full
  save/reload unless `--max-roundtrip-bytes` is raised).
- Saves above `--max-roundtrip-bytes` are load-checked but marked
  `skipped-large` for write/reload.
- Real game data is gitignored; CI cannot run this matrix (it uses skip-if-absent
  rules, see `docs/CI.md`).
- Old pre-0.6a1 artifacts are intentionally not migrated; no attempt is made to
  guess 28-gene semantics (D14).
- When an old save triggers a structural `BrainUpdater` migration, io rebuilds
  the brain and re-executes every non-input node once with zero input (game
  `FinalizeEditing`/`ResumeBrain` semantics); otherwise saved runtime node state
  is preserved exactly. Regression:
  `bibites-io::tests::migrated_save_reinitializes_brain_outputs_like_game`
  (`rust_seed_600.zip`, Digestion=0.7734683). The re-execute period is threaded
  from the save's effective settings (`settings.brain_period()`), not hardcoded;
  no non-default-TPS fixture is added because the re-executed nodes start with
  zero input/state, so period-sensitive Differential/Integrator activations are
  zero and cannot expose the period in an observable value without a synthetic
  stateful hidden node. The binding is covered by code-path review; add a
  synthetic fixture if such a save becomes available.

## Reporting an incompatible file

1. Keep the original untouched; copy it to `target/compat/cases/` (gitignored).
2. Run `python tools/compat/scan.py --no-build --root target/compat/cases` and
   keep the generated row from `report.json` / `report.csv` plus the error text.
3. Include: RUSTibites commit, OS, game version (from `scene.bb8scene` /
   template `version`), file type and size, the exact CLI command, and the
   `unexpected.txt` entry.
4. If it is a new failure class, open an issue and link `docs/DEVIATIONS.md`;
   if it is a game asset, **do not attach it publicly** — share privately with
   the maintainers or provide a minimized synthetic fixture.

### Rust-writer game-load fixture pack (2026-10-30)

A dedicated **game-load verification pack** now exists for Rust-written
artifacts: `testdata/parity/compat/` (gitignored), generated by
`tools/roundtrip/gen_compat_pack.ps1`. It covers real/ synthetic saves (tools,
eggs, corpses, pheromones, species logs, 0.6 scenario-start save, checkpoint
sidecar) plus Rust-written scenarios/templates; `manifest.json` carries exact
SHA-256, expected counts/entity ids and load/re-save commands.
`tools/roundtrip/load_compat_pack.py` drives the hidden BibitesAPI lab game to
load each fixture, re-save it and report rejections. See
[`SAVE_COVERAGE.md`](SAVE_COVERAGE.md#game-load-verification-fixture-pack).

### Game-load compat-pack status (load-accepted, 2026-11-01)

All **11 pack entries** (9 save/scenario/template fixtures + 2 `.bb8state`
sidecars) are accepted by the game with the expected counts/fields: save
fixtures load **22 living / 13 corpses / 4 eggs** or **1 living / 0 eggs**;
Default/Ring scenarios start with **1 living**; the template converter accepts
`rust_template.bb8template` (`converter kind=template`). No rejections or dropped
expected fields were observed. Evidence report:
`assets/game/reference/compat/game_report_2026_11_01.json` (top-level `fixtures`
array fixed in `d78c120`; report lists the 9 base fixtures, with parity's 11
including the two sidecar entries).

**Re-save harness limitation:** every harness `save_world` re-save attempt still
fails with a bridge connection close (`BridgeUnavailable [WinError 10054]`) even
after 2 s settle, up to 2 retries and a ping; the lifecycle watchdog exits
**124**. This is a harness/bridge lifecycle issue, **not fixture rejection**, and
**not release-blocking**. Load acceptance is closed; a future native/manual
in-game re-save retest (or a bridge that survives post-load re-save) is the
follow-up.

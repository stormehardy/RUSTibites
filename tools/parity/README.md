# tools/parity — ground-truth capture and comparison

Reproducible per-tick telemetry from the **real installed game** (The Bibites 0.6.3.1 + BibitesBridge
0.7.0), and a diff tool for RUSTibites runs. The full contract (schema, exact-vs-approximate fields,
tolerances, determinism findings) is in [`docs/PARITY.md`](../../docs/PARITY.md) — read that first.

```
capture.py      headless game -> ticks.jsonl.gz + events.jsonl.gz + manifest.json
compare.py      diff two captures, per-system metrics, --report human|json, CI exit codes
parity_common.py  schema constants, digests, record builders, capture reader (shared by both)
dump_prefabs.py build+deploy the BepInEx plugin, dump prefab/serialized constants, remove it again
exp_behavior.py  behavioural micro-experiments: ColorKiller polarity, collision-damage sign, pheromone
                 decay/heading (bridge-only; evidence under testdata/parity/experiments/)
probe.py        interactive bridge probe (route semantics, per-tick sampler, snapshot coverage)
plugin/         the parity BepInEx plugin source (ParityDump, unique GUID)
schema/         capture-v1.schema.json (machine-readable)
```

## Quickstart

```powershell
# 0) (once per machine) build + deploy the parity plugin and dump the prefab constants.
#    This starts a hidden game, writes testdata/parity/prefab_dump.json, stops the game and
#    REMOVES the plugin from the game install again.
python tools\parity\dump_prefabs.py

# 1) capture 300 ticks from a base world created from Ring.zip, twice (determinism evidence).
#    The first run also creates the base save + its .base.json provenance.
python tools\parity\capture.py --scenario testdata\real\Ring.zip `
    --base testdata\parity\bases\ring_parity.zip `
    --ticks 300 --seed 12345 --speed 50 --snapshot-every 50 --repeat 2 `
    --out testdata\parity\captures\ring_seed12345

# 2) compare the two passes: exit 0 / bit-identical with the plugin installed.
#    NOTE: deterministic captures need --speed 1 (the game is bit-reproducible only at 1x; at higher
#    speeds several fixed steps share a frame and the run drifts - the capture is marked taint).
python tools\parity\compare.py testdata\parity\captures\ring_seed12345\pass-0 `
    testdata\parity\captures\ring_seed12345\pass-1 --profile exact

# 3) compare a RUSTibites capture (schema v1) against the game capture
python tools\parity\compare.py testdata\parity\captures\ring_seed12345\pass-0 `
    target\rust-capture --profile default --report human --out report.json
```

`capture.py` always stops the game it started, and refuses to touch a game it did not start
(`--attach` is the explicit, interactive-only override). The game is started **hidden** (real graphics
device, no window, no taskbar button) through `bibites_api.lab.Lab.ensure_game()`; lab mode disables
autosaves so the player's `Autosaves/` folder is never written.

## Requirements

* Python 3.11 with the BibitesAPI package (`BIBITES_API_HOME` overrides the default
  `C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI`).
* The installed game with BepInEx + BibitesBridge 0.7.0.
* The parity plugin for deterministic captures and the prefab dump (built/deployed by
  `dump_prefabs.py`; `capture.py` auto-detects it on port 17801 and records `plugin.*` in the manifest).
* `dotnet` SDK only for building the plugin.

Without the plugin `capture.py` still runs (`--no-plugin`), but records `taint` entries: the game is
**not** bit-reproducible by itself, so such a capture is a single observation, not ground truth.

## What a capture contains

`testdata/parity/` is gitignored (game data/captures are not redistributable). A capture directory is:

```
manifest.json       schema, engine versions, inputs (scenario/base/settings/seed), sim range,
                    throughput/overshoot per segment, taint[], file hashes
ticks.jsonl.gz      one JSON object per sampled tick (counts, pellets, vitals, species, generation,
                    brain size, gene mean/std, behaviour incl. all 15 brain output means, optional
                    per-entity snapshots at pause boundaries)
events.jsonl.gz     birth/death log (exact to a simulation tick)
determinism.json    present when --repeat > 1: tick-by-tick digest equality between passes
capture.json        summary of the capture set
```

Per-entity snapshots (`entities[]` on a tick record) carry position, velocity components, energy,
health, age, size, species/generation/tag, brain size, and four cross-language digests: genes, senses
(33 input node values), brain IO (15 output nodes) and full brain state. Digests are computed over raw
little-endian f32 bit patterns, never over decimal text, so Python/C#/Rust agree exactly. Definitions
and the mandatory gene order are in `docs/PARITY.md` §6.3.

## Regenerating fixtures

* Base worlds live in `testdata/parity/bases/`; a base is a **save zip** plus `<base>.base.json`
  provenance. Recreate with `capture.py --scenario <scenario.zip> --base <new base zip> --ticks 1`
  and delete the old captures: a new base is a different fixture (its hash is in every manifest).
* The Ring fixture uses a newly created world, `virginSpawnRate=0` in the save's `independents`
  block, and the effective settings frozen in the provenance. Saves carry their independent settings
  (`settings.bb8settings["independents"]`, verified for 0.6.3.1); scenarios only carry a `recommended`
  list, so a new world takes the game's global values — capture records whichever applies.
* Prefab constants: re-run `dump_prefabs.py` after a game update (`prefab_dump.json` is gitignored).
* No capture or game file is committed: `testdata/` is in `.gitignore` by project policy.

## Rules

1. Never modify the game install, the player's saves (`AppData/LocalLow/The Bibites/...`) or the
   BibitesAPI project. The only transient change is the parity plugin DLL under
   `<game>/BepInEx/plugins/ParityDump/`, deployed and removed by `dump_prefabs.py` (and by the
   documented manual steps). If a run is interrupted, delete that folder.
2. Never capture from a game you did not start (`--attach` is for debugging only).
3. Always stop the game/API processes you started (`capture.py`/`dump_prefabs.py` do it in `finally:`).
4. Keep captures small: 200–500 ticks, snapshot every 25–100 ticks. They are gitignored; only
   scripts/docs/schema are committed.

## CI

```powershell
python tools/parity/compare.py --self-test                       # tool self-test, no game
python tools/parity/compare.py <game> <game> --profile exact     # replay determinism gate
python tools/parity/compare.py <game> <rust> --profile default   # milestone-1 parity gate
```

`compare.py` exits `0` when every gated system passes (`default`: exact systems 100%, ecology curves
within 10%, energy >= 95% bit-exact; positions/velocities are reported but not gated until the physics
solver is tuned), `1` on a gate failure and `2` on usage/IO errors.


## Canonical Default baseline (2026-10-30, docs/TESTING_POLICY.md)

Canonical world: `testdata/real/Default.zip`. Ring/lol examples above are historical diagnostics.

```powershell
python tools\parity\capture.py --scenario testdata\real\Default.zip --ticks 600 --seed 12345 `
    --speed 1 --feeding --set simTPS=30 --out testdata\parity\default\game_600
target\release\rustibites.exe run testdata\real\Default.zip --ticks 600 --seed 12345 `
    --capture testdata\parity\rust_default_600 --quiet
python tools\parity\compare.py testdata\parity\default\game_600 testdata\parity\rust_default_600 `
    --profile default --report json --out testdata\parity\default\compare_600.json
python tools\parity\default_baseline.py --game testdata\parity\default\game_600 `
    --rust testdata\parity\rust_default_600 --ticks 600
```

`--set simTPS=30` must be pinned before newgame (the game global may be another TPS); compare.py now has
`feeding`/`conservation` sections. Default-600 currently fails `feeding_rates` (game ~0.46 u²/live-s vs
Rust 0.117 — the task-93 target); game exact caller-path conservation is numerically exact at 600t
(residual −3.8e−06 E) and 0.246% at 3000t.


## Runtime UI field dump (task-102)

The transient ParityDump plugin exposes `GET /d/ui`: a hidden-window walk of live Canvas hierarchies
returning Image/TMP/CanvasScaler/LayoutGroup managed fields (asset files cannot read these without
typetrees). Capture with `python tools/assets/capture_ui_runtime.py --repeat 2` (deploy the plugin
first; it is removed after the run), join with `python tools/assets/ui_runtime_join.py`; outputs are
`assets/game/ui_runtime.json.gz` + index and the view-dev consumer
`tools/ui/ui_runtime_atlas.json.gz` + index. No visible window; deterministic structurally (volatile
HUD/animation values ignored, listed in the index).

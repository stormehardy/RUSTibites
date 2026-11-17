# RUSTibites benchmark harness (`tools/bench`)

Reproducible throughput and phase-timing measurements for the headless
RUSTibites engine. The numbers these scripts produce are what
[`docs/BENCHMARKS.md`](../../docs/BENCHMARKS.md) and
[`docs/ARCHITECTURE.md` §10](../../docs/ARCHITECTURE.md#10-performance-targets-and-measurement)
quote; this directory is the tooling, not the results.

```
tools/bench/
  bench_common.ps1     shared helpers (process execution, env/load probes, parsing)
  env.ps1              machine/build metadata capture (CPU, RAM, rustc, git, profile, binary hash)
  run_matrix.ps1       populations x threads matrix -> runs.csv / summary.csv / matrix.md / matrix.json
  run_phases.ps1       per-phase breakdown from `bench` human output -> phases.csv / phases.md
  run_cluster.ps1      dense-pile (clustered save world + spread control + optional core probe)
  make_cluster_world.py generator for the deterministic cluster worlds (Python, stdlib only)
  scenarios/cluster/   tracked inputs for the cluster world (settings/template/scenario.info)
  results/             generated artifacts (checked in; every file is regenerable)
    final/             published final matrix (fixed binary)
    final-phases/      published phase capture (same fixed binary)
    final-100k/        published 100k one-tick memory/runnability sanity
    cluster/           published dense-cluster capture
    quick/             earlier quick matrix (historical)
    phases/            earlier phase capture (historical)
    full/              full 1k-100k throughput matrix (when run)
```

Everything here is read-only with respect to `crates/`, the root `Cargo.toml`
and the CLI: the scripts only run the already-built release binary
(`target/release/rustibites[.exe]`); pass `-Build` to (re)build it with
`cargo build --release -p bibites-cli`. A missing binary is an error with
instructions rather than an implicit rebuild, so a shared machine is never
disturbed by a surprise LTO build. Do not benchmark while the machine is
compiling or while another agent/benchmark is running — the scripts record
concurrent CPU load so polluted runs are identifiable after the fact.

## Quick start

Windows PowerShell 5.1 and PowerShell 7+ on Windows/macOS/Linux are supported;
no PowerShell modules are required.

```powershell
# 1. Quick matrix: 1k/2k/5k/10k/20k x 1/4/8 threads, 30 ticks, 1 repetition (~20-40 s).
powershell -NoProfile -ExecutionPolicy Bypass -File tools\bench\run_matrix.ps1 -Mode quick

# 2. Published final matrix: freeze the binary first, then 120 ticks, best-of-3,
#    idle-machine guarded. See docs/BENCHMARKS.md#reproducing-the-latest-numbers.
$sha = (Get-FileHash target\release\rustibites.exe -Algorithm SHA256).Hash
Copy-Item target\release\rustibites.exe "target\bench-final\rustibites-$($sha.Substring(0,8)).exe"
powershell -NoProfile -ExecutionPolicy Bypass -File tools\bench\run_matrix.ps1 `
    -Exe "target\bench-final\rustibites-$($sha.Substring(0,8)).exe" `
    -Pops 1000,2000,5000,10000,20000 -Threads 1,4,8 -Ticks 120 -Repeat 3 `
    -MaxLoadPct 45 -LoadWaitSec 120 -DeterminismCheck `
    -OutputDir tools\bench\results\final -UpdateDocs

# 3. Phase breakdown: 1k/2k/5k/10k serial, 300 ticks (same fixed binary via -Exe).
powershell -NoProfile -ExecutionPolicy Bypass -File tools\bench\run_phases.ps1 `
    -Exe "target\bench-final\rustibites-$($sha.Substring(0,8)).exe" `
    -Pops 1000,2000,5000,10000 -Threads 1 -Ticks 300 `
    -OutputDir tools\bench\results\final-phases -UpdateDocs

# 4. Dense-pile mode: 2,000 bibites in 100x100 and 200x200 boxes + spread control,
#    plus the physics-only core probe (Python 3.8+ needed to generate worlds).
powershell -NoProfile -ExecutionPolicy Bypass -File tools\bench\run_cluster.ps1 `
    -Pops 2000 -BoxSizes 100,200 -Ticks 10 -Repeat 2 -CoreProbe

# 5. Environment only.
powershell -NoProfile -ExecutionPolicy Bypass -File tools\bench\env.ps1
```

On PowerShell 7 (`pwsh`) use `pwsh -File ...` instead of `powershell ...`.

## What is measured

`rustibites bench --bibites N --ticks T --threads X --repeat R --json` prints one
JSON object per repetition:

```json
{"label":"synthetic 2000 bibites [rep 1]","ticks":60,"bibites_start":2000,
 "bibites_end":2000,"wall_secs":0.087,"sim_secs":2.0,"ticks_per_second":692.0,
 "realtime_factor":23.1,"bibite_updates_per_second":1383692.0,"threads":1,
 "peak_bytes":null,"state_fingerprint":...}
```

Key semantics (also recorded in every `environment.json` / `matrix.md`):

| Field | Meaning |
|---|---|
| `wall_secs` | wall time of `world.run(ticks)` **only** — world construction, scenario seeding and synthetic spawn are excluded |
| `ticks_per_second` | `ticks / wall_secs` |
| `realtime_factor` | `sim_secs / wall_secs`; 30 simulated TPS by default, so 1× RT = 30 ticks/s |
| `bibite_updates_per_second` | `ticks_per_second * bibites_start` — the primary throughput number |
| `state_fingerprint` | FNV-1a over entity state; same seed ⇒ same fingerprint, so it doubles as a determinism check |
| `peak_bytes` | always `null` today; the harness samples process peak working set instead |
| `threads` | worker threads the engine actually used (`--threads 0` = automatic) |

The matrix harness adds, per process: `process_wall_secs` (startup + setup +
ticks), `peak_ws_mb`, and load samples (`cpu_load_pct_before/after`,
`rustc_procs_before/after`, `free_gb_before/after`).

The synthetic benchmark world scales its area as `2000 * sqrt(N/2000)`, keeping
bibite/pellet/zone density constant at the 2,000-bibite reference. Each
repetition uses a different deterministic seed (`0x5255535400000000 xor
repetition_index`), so `state_fingerprint` must be identical across thread counts
for the same repetition index.

## Matrix modes and flags

`run_matrix.ps1` defaults come from `-Mode`; any explicit parameter wins.

| Flag | Quick | Full | Notes |
|---|---|---|---|
| `-Pops` | 1000,2000,5000,10000,20000 | + 50000,100000 | synthetic populations |
| `-Threads` | 1,4,8 | 1,4,8,12 | explicit thread counts |
| `-Ticks` | 30 | 60 | ticks per run |
| `-Repeat` | 1 | 3 | processes per cell (best/median reported) |
| `-TimeoutSec` | 180 | 900 | per-process wall-clock limit |

Other useful flags:

| Flag | Effect |
|---|---|
| `-Scenario <path>` | benchmark a real scenario/save instead of the synthetic world; population comes from the scenario. Canonical real world: `testdata/real/Default.zip` (testing policy); other saves are historical fixtures and must be labelled by world |
| `-BudgetSec N` | stop launching new cells after N seconds and mark the rest `skipped_budget` (bounded runtime on shared machines) |
| `-MinFreeGB N` | skip a cell when free physical memory drops below N GB (default 0.5) |
| `-MaxLoadPct N`, `-LoadWaitSec N` | idle-machine guard: before each cell, wait up to N seconds for CPU load ≤ N% with no `rustc`/`cargo` and no other `rustibites` process, then mark the cell `skipped_load` (default off; useful on shared hosts) |
| `-DeterminismCheck` | rerun two populations in separate processes and compare `state_fingerprint` to the matrix runs |
| `-OutputDir <dir>` | artifact directory (default `tools/bench/results/<mode>`) |
| `-RunLabel <text>` | label recorded in `config.json` / Markdown |
| `-Build` | force `cargo build --release -p bibites-cli` before measuring |
| `-UpdateDocs` | replace the block between `<!-- BEGIN BENCH:MATRIX -->` / `<!-- END BENCH:MATRIX -->` in `docs/BENCHMARKS.md` with the generated tables |
| `-Quiet` | suppress per-run progress lines |

Exit codes: `0` = at least one cell succeeded, `2` = no cell succeeded
(also configuration/build errors). Individual failed/timeout/OOM cells do not
abort the matrix; they are recorded with `status` in `runs.csv`/`summary.csv` so
the 100k population can be probed safely.

### Dense-cluster mode (`run_cluster.ps1`)

The spread matrix above is the normal-case story. A dense pile is a different
regime: the physics broadphase goes quadratic-ish in contacts, so 2,000 bibites
packed into a 100×100 box cost ~200 ms/tick instead of ~2 ms. `run_cluster.ps1`
tracks that cliff end-to-end:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\bench\run_cluster.ps1 `
    -Pops 2000 -BoxSizes 100,200 -Ticks 10 -Repeat 2 -CoreProbe
```

- **World generation** (`make_cluster_world.py`, Python 3.8+ stdlib only): builds
  a minimal one-zone scenario from tracked files in `scenarios/cluster/` (no
  `testdata/` and no game install), runs it once with a fixed seed for one
  deterministic starter bibite, then clones that bibite N times onto a
  deterministic jittered lattice inside a `box × box` square. The resulting save
  zip is cached under `target/bench-worlds/cluster/` (gitignored) and keyed by
  N, box, seed, scenario and engine binary SHA-256; its `content_sha256` is
  recorded in `cluster.csv`/`config.json`.
- **Measurements**: for each N × box it runs `bench --scenario <cluster world>`
  (whole engine) and once per population/threads a synthetic `bench --bibites N`
  spread control, both parsed for phase tables. `-CoreProbe` also runs the
  physics-only ignored audit `body::perf_probe_clustered_and_spread` (starter
  genome, 100×100 cluster box) — that is the canonical fast-iteration metric for
  the dense-cluster physics work.
- **Artifacts**: `results/cluster/cluster.csv`, `cluster_summary.csv`,
  `cluster.md` (whole-engine table, cluster-vs-spread cliff table, core-probe
  table, determinism), `cluster.json`, plus `core_probe.csv`/`core_probe.log` and
  `determinism.json` when requested.

Flags: `-Pops`, `-BoxSizes`, `-Threads`, `-Ticks`, `-Repeat`, `-Seed` (default
12345), `-CoreProbe`, `-NoSpread`, `-ForceWorlds`, `-DeterminismCheck`,
`-MaxLoadPct`/`-LoadWaitSec`, `-TimeoutSec`, `-OutputDir`, `-Exe`, `-Build`.
Keep `-Ticks` small (default 10): dense worlds are slow by design, and runtime
scales fast with population/box density.

### Machine-noise discipline

- Check for concurrent work first: `Get-Process rustc,cargo -ErrorAction SilentlyContinue`
  and `(Get-CimInstance Win32_Processor).LoadPercentage`.
- Use `-Repeat 1` while other agents are active; use `-Repeat 3` (best-of-3)
  only on a quiet machine. The scripts always record the observed CPU load.
- On shared hosts, `-MaxLoadPct 40 -LoadWaitSec 120` makes the matrix wait for an
  idle window per cell instead of silently publishing polluted numbers.
- Do not run a 100k cell while someone else is compiling: the process needs
  ~0.5 GB and the numbers are only valid on an idle machine.
- Compare runs by **binary SHA-256** (in `environment.json`), not by source
  state: the tree is frequently dirty during the performance phase and
  `target/release` may be built from uncommitted code.

## Artifacts

`run_matrix.ps1` writes (all UTF-8, locale-independent formatting):

| File | Contents |
|---|---|
| `runs.csv` | one row per process/repetition: raw engine fields + process wall, peak WS, load samples, status |
| `summary.csv` | one row per population × threads cell: best/median wall and throughput, speedup vs serial, peak WS, fingerprint |
| `matrix.md` | Markdown tables: ticks/s (with ×RT) per population × threads, thread scaling, per-cell detail |
| `matrix.json` | config + aggregated cells + fingerprint/determinism checks |
| `config.json` | exact invocation parameters and paths |
| `environment.json` / `environment.txt` | OS/CPU/RAM, rustc/cargo, git HEAD + dirty state, release profile, binary SHA-256 |

`run_phases.ps1` writes `phases.csv` (one column pair per engine phase),
`phases.md` (compact table, dominant phases + `other`), `phases_raw.txt` (raw
`bench` output per run) plus `config.json` / `environment.json`.

`run_cluster.ps1` writes `cluster.csv` (one row per process: whole-engine phases
+ world content SHA-256), `cluster_summary.csv` (best/median per cell),
`cluster.md` (whole-engine table, cluster-vs-spread cliff table, core-probe
table, determinism), `cluster.json`, plus `core_probe.csv`/`core_probe.log` with
`-CoreProbe` and `determinism.json` with `-DeterminismCheck`.

## Adding scenarios

Per [`docs/TESTING_POLICY.md`](../../docs/TESTING_POLICY.md) the canonical real
world is **`testdata/real/Default.zip`** (the game's Default scenario). Other real
saves/scenarios (`Ring.zip`, `lol.zip`, …) are historical fixtures: use them only
for an explicit regression and keep the world name next to the numbers. Synthetic
`bench --bibites N` worlds remain allowed for throughput work.

```powershell
# Canonical real scenario: Default.zip, 600 ticks, threads 1/8, best-of-2.
powershell -NoProfile -ExecutionPolicy Bypass -File tools\bench\run_matrix.ps1 `
    -Scenario testdata\real\Default.zip -Threads 1,8 -Ticks 600 -Repeat 2 `
    -OutputDir tools\bench\results\default
```

With `-Scenario` the population axis is skipped and the engine-reported
`bibites_start` is recorded per row (the scenario/world label is carried into the
CSV/Markdown). To compare engines/builds, keep ticks, scenario and repetition
count identical and compare like-for-like cells.

The dense-cluster world is not a hand-maintained save: its tracked inputs live in
`scenarios/cluster/` (minimal settings, one spawn row, a bundled genome template)
and `make_cluster_world.py` turns them into a save for `bench --scenario`. The
generated world is cached under `target/bench-worlds/cluster/`, so bump the
scenario files if you need a different ecology/body plan and the cache key
(scenario content SHA) will regenerate it automatically.

## Per-phase timings

The engine always collects `World::phase_timings()` (sub-µs overhead); only the
human `bench` output prints them. Run one process directly to inspect them:

```powershell
target\release\rustibites.exe bench --bibites 2000 --ticks 500 --threads 1 --repeat 1
```

Each repetition prints a `phase / ms per tick / share` table after its summary
line. Phase names come from `bibites_core::sim::Phase::ALL`
(`crates/bibites-core/src/sim.rs`); `tools/bench/run_phases.ps1` parses them into
CSV. To add a timer: add the `Phase` variant in `sim.rs`, wrap the section with
the existing timing guard, and extend `$script:BenchPhaseNames` in
`bench_common.ps1` so the CSV keeps a stable column order.

## Portability (Linux/macOS)

The scripts use PowerShell core cmdlets and .NET APIs only; they need
PowerShell 7+ on non-Windows hosts (`pwsh -File tools/bench/run_matrix.ps1 ...`).
CPU/RAM/load probes read `/proc` on Linux and use CIM on Windows. The binary path
is `target/release/rustibites` on non-Windows. Artifacts are identical in schema,
so tables from Linux and Windows can be compared side by side (keep the reference
machine column in mind).

## Troubleshooting

| Symptom | Fix |
|---|---|
| `benchmark binary not found` | `cargo build --release -p bibites-cli`, or pass `-Build` |
| All cells `timeout` | raise `-TimeoutSec`; check you are not benchmarking a huge population on a loaded machine |
| `peak_ws_mb` is low / missing | very short runs can finish between 50 ms samples; the engine field is `null` by design (harness-sampled) |
| Numbers much worse than `docs/BENCHMARKS.md` | check `runs.csv` load columns and the binary SHA-256 in `environment.json`; re-run on an idle machine |
| `-UpdateDocs` throws "markers not found" | restore the two HTML comment markers in `docs/BENCHMARKS.md` |

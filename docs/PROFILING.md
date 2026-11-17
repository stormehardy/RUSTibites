# Profiling

How to find out where a RUSTibites tick spends its time, what tooling actually
exists on the reference machine, and how to record before/after numbers that
other people can compare.

This is the "how to investigate" companion to
[`BENCHMARKS.md`](BENCHMARKS.md) (throughput numbers) and
[`PERF_DIAGNOSIS.md`](PERF_DIAGNOSIS.md) (the historical bottleneck analysis).
The runnable tooling is [`tools/profiling/`](../tools/profiling/README.md).

## TL;DR: pick a tier

| You want to know | Use | Cost |
|---|---|---|
| Which phase dominates the tick at population N / thread count X | **Tier 1:** `rustibites bench` human output (`World::phase_timings()`, always on) or `tools/profiling/profile.ps1 -Action bench-phases` | seconds |
| A section-level breakdown + per-subsystem probes without a profiler | **Tier 2:** `profile.ps1 -Action phase-audit` (all ignored `perf_*` tests) and `-Action micro-probes` | 2–90 s (one-time release test compile) |
| A specific function-level hotspot / flamegraph | **Tier 3:** sampling profiler. **Not installed on this machine**; see [Sampling profilers](#tier-3-sampling-profilers-available-gaps) for what works, what needs approval | minutes |
| A symbolised binary for an external profiler/debugger | `profile.ps1 -Action build-symbols` | ~2–4 min LTO build |
| To add a new timer/counter | crate-owner change (Lead approval); steps in [Adding instrumentation](#adding-instrumentation-crate-owner-change) | code review |

Start with Tier 1/2. On the current engine they already show **physics + body
= ~85–90% of the tick**, so a profiler is only needed to divide that *within*
physics/body.

## Tier 1: phase timings (always on, no setup)

`World` times every top-level tick section into `PhaseTimings` on every tick
(sub-µs overhead). Definition: `crates/bibites-core/src/sim.rs:324` (`enum
Phase`), `:362` (`PHASE_COUNT = 17`), `:366` (`Phase::ALL` order), `:388`
(`Phase::name`), `:417` (`PhaseTimings`). The `step` marks each phase right after
the work: `crates/bibites-core/src/world.rs:2640`–`:2691` (e.g.
`mark_phase(Phase::Sensing, start)` at `:2654`, `Phase::Body` `:2657`,
`Phase::Physics` `:2684`, `Phase::HashSync` `:2691`).

Human `bench` output prints the mean per phase after every repetition
(`crates/bibites-cli/src/engine.rs:766`); JSON output only carries totals:

```powershell
# One process, full phase table per repetition.
target\release\rustibites.exe bench --bibites 2000 --ticks 500 --threads 1 --repeat 1

# Same thing for a population x threads matrix, parsed into CSV/Markdown:
powershell -NoProfile -ExecutionPolicy Bypass -File tools\profiling\profile.ps1 -Action bench-phases `
    -Pops 1000,2000,5000,10000,20000 -Threads 1,8 -Ticks 120
# -> tools/profiling/results/bench-phases/phases.csv / phases.md / phases_raw.txt
```

> The capture checked in under `tools/profiling/results/bench-phases/` was taken
> while other agents were active (`cpu_load_pct ≈ 100` in its
> `environment.json`), so **absolute ms/tick values are inflated**; the
> *relative shares* are what to read. Same caveat for everything under
> `tools/profiling/results/`.

Representative shares from that capture (binary `BE9AA615…`, 120 ticks/run,
machine under load):

| Population | Threads | ms/tick | physics | body | sensing | rest |
|---|---|---|---|---|---|---|
| 1,000 | 1 | 1.025 | 39.7% | 41.0% | 18.2% | ~1% |
| 2,000 | 1 | 2.545 | 39.0% | 47.1% | 12.9% | ~1% |
| 2,000 | 8 | 2.066 | 43.3% | 49.8% | 5.8% | ~1% |
| 10,000 | 1 | 25.000 | 35.3% | 56.2% | 7.4% | ~1% |
| 20,000 | 8 | 43.478 | 47.8% | 46.8% | 3.6% | ~2% |

Reading it:

- `sum(phases)` ≈ `ms/tick` = `1000 / ticks/s`. Units are **ms per tick**, means
  over all measured ticks.
- **physics + body dominate everywhere**; `sensing` is 3–18% and shrinks with
  worker threads (the only meaningfully parallel phase); deaths/hash-sync/decay/
  zones are sub-1% at these sizes.
- If a phase's share changes by more than a couple of points after a change,
  it is worth a direct audit below; absolute ms differ by machine/build/load, so
  always report binary SHA-256 + load with the numbers (see
  [Reporting template](#reporting-template)).

## Tier 2: ignored audits (`perf_*` tests, no profiler needed)

`bibites-core` ships manual audits behind `#[ignore]`, all compiled into the
release test binary. Run them all with:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\profiling\profile.ps1 -Action phase-audit
# == cargo test --release -p bibites-core --lib -- --ignored --nocapture --test-threads=1 perf_
# -> tools/profiling/results/phase_audit.log + phase_audit.meta.json
```

| Test | Source | Prints |
|---|---|---|
| `world::tests::perf_phase_audit` | `crates/bibites-core/src/world.rs:5843` | Phase table for N=500, N=2000, N=2000@8 threads; 5000-bibite sensing/events/hash-sync micro-measurements |
| `world::tests::perf_scale_sizes` | `world.rs:5813` | `size_of` for `BibiteRecord`, `Brain`, `SenseOutputs`, `SteeringInputs`, hash `Entry`, `QueryHit`, records, starter brain node/synapse counts |
| `world::tests::perf_sensing_tick_profile` | `world.rs:5502` | due-tick cadence, per-due-bibite FOV µs, hash take/restore, prefilter, serial vs 8-thread sensing |
| `world::tests::perf_sensing_breakdown` | `world.rs:5666` | forced-due sensing, herding-off/no-phero variants, query-only pass, sort-128-hit buffers, executor pool activations |
| `body::perf_probe_clustered_and_spread` | `crates/bibites-core/src/body.rs:5133` | `step_physics` on spread 2000/5000 and clustered 2000/4000: elapsed, candidate pairs, contacts, triggers |
| `body::perf_probe_world_phase_table` | `body.rs:5207` | body-only world phase table |
| `sim::tests::perf_parallel_executor_overhead` | `crates/bibites-core/src/sim.rs:1073` | thread-pool activation cost vs serial for tiny chunk jobs |

Run a single audit while iterating (compiles once, then seconds):

```powershell
cargo test --release -p bibites-core --lib -- --ignored --nocapture --exact world::tests::perf_phase_audit
cargo test --release -p bibites-core --lib -- --ignored --nocapture --exact body::perf_probe_clustered_and_spread
```

### What the current audits say

From `tools/profiling/results/phase_audit.log` (captured 2026-10-30 with the
machine under load; compare shapes/deltas, not absolute ms):

- **Phase split** (N=2000, 1 thread): body ≈1.55 ms, physics ≈1.14 ms, sensing
  ≈0.04 ms in that run's cadence, total ≈2.76 ms/tick → body+physics ≈97% of
  measured phase time. N=2000 @ 8 threads cut the sensing component but not the
  tick much (Amdahl: physics/body serial).
- **Memory/sizes**: `BibiteRecord = 1704 B`, `Brain = 440 B`,
  `SenseOutputs = 96 B`, `SteeringInputs = 32 B` (hash `Entry`/`QueryHit` are
  small fixed records). Brain still owns ~18 separate `Vec` allocations
  (≈2.6 KB/brain, see ARCHITECTURE §10 task-48) — a locality/allocation target
  for the brain owner.
- **Physics broadphase cliff**: spread 2,000 = ~1.1–1.75 ms with 109 candidate
  pairs; clustered 2,000 = **221–393 ms** with 145,227 pairs; clustered 4,000 =
  **0.85–1.52 s** with 577,764 pairs (pairs ≈ 4× for 2× population). Spread
  5,000 = 3.6–6.9 ms / 699 pairs. The ranges are build/load variance — the
  newest tracked capture is in `tools/bench/results/cluster/core_probe.csv`;
  either way this is why clustered worlds need the grid fallback /
  density-aware broadphase. Reproduce the probe with
  `body::perf_probe_clustered_and_spread`.
- **Sensing**: per-due-bibite FOV ≈0.27–0.28 µs; query-only ≈1.1–1.3 ms per
  2,000-bibite sweep; "sort-128-hit buffers" measured 5.3 ms in the audit (worth
  a look); forced-due sensing was 3.4 ms serial vs 1.3 ms on 8 threads (≈2.7×)
  in one run and 8.4 ms in another under heavier load — sensing scales but is
  noisy, and end-to-end gain is capped by the serial 85–90%.
- **Executor**: pool activation for 8 × 1-item chunks = **34 µs/call** vs
  **0.8 µs** serial. Chunk sizes must amortise the activation cost; this is the
  per-tick overhead any parallel pass-B work must beat.

### Dense-pile end-to-end (`run_cluster.ps1`)

The core probe above isolates `BibiteStore::step_physics`. To see the same cliff
in a **whole engine tick** (physics + body + hash-sync + ...) on a deterministic
world, use the bench harness's cluster mode:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\bench\run_cluster.ps1 `
    -Pops 2000 -BoxSizes 100,200 -Ticks 10 -Repeat 2 -CoreProbe
# -> tools/bench/results/cluster/cluster.md, cluster.csv, core_probe.csv
```

It generates a save world with N deterministic bibites on a jittered lattice
inside a `box × box` square (tracked inputs under `tools/bench/scenarios/cluster/`,
no `testdata/` needed), runs it for a few ticks, and records phase means. Latest
tracked capture (2026-10-30T08:29Z, binary `4F267FF9…`, same fixed binary as the
final matrix):

| Case | Population | Box | Physics ms/tick | Whole tick ms | Physics share |
|---|---|---|---|---|---|
| cluster | 2,000 | 100×100 | 94.18 | 90.91–100 | ~94–99% |
| cluster | 2,000 | 200×200 | 21.14 | 22.73 | ~93% |
| spread control | 2,000 | — | 0.72 | 1.40 | ~51% |
| core probe `cluster2000` | 2,000 | 100×100 | 159.09 | — | 100% (physics only) |
| core probe `cluster4000` | 4,000 | 100×100 | 627.24 | — | 100% (physics only) |

Reading: the dense box makes physics ~131× slower than the spread world at the
same population, and the box area (not population alone) drives it (100→200 box
is 4× area and ~4.5× faster). The canonical task-82 target (**cluster2000 physics
< 50 ms**) is measured on the physics-only core probe with the starter genome;
the whole-engine rows use the bundled fixture genome and are for end-to-end
regression tracking, not for target sign-off. This capture is already >25% better
than the previous binary snapshot (`63435014…`: 159.09 vs 221.53 ms probe;
~94 vs ~198 ms whole-engine physics). See
[`BENCHMARKS.md`](BENCHMARKS.md#dense-cluster-benchmark) for the published table
and per-world content SHA-256 details.

## Tier 3: sampling profilers (available/gaps)

Run `profile.ps1` (default action `doctor`) for a live report; the reference
machine (2026-10-30) looks like this:

| Tool | Status | Notes |
|---|---|---|
| Windows Performance Recorder (`wpr`) | **present** | ETW CPU/Heap/etc. capture, but kernel sessions need an **elevated shell** (non-elevated fails with `0xc5585011`). |
| `tracerpt` | present | Converts `.etl` to CSV/XML; flat events, not a flamegraph. |
| Windows Performance Analyzer (`wpa`), `xperf` | **missing** | No stack/symbol analysis of `.etl` here. Installing Windows Performance Toolkit (from the Windows SDK) requires Lead approval. |
| `samply`, `perf`, `cargo-flamegraph`, PerfView, `procdump`, `dotnet-trace` | **missing** | No flamegraph sampler installed. `cargo install samply` / `cargo install flamegraph` needs approval (network + build). |
| Visual Studio | **BuildTools only** | No IDE profiling/diagnostics hub. |

What works today without approvals:

1. **Tier 1 + Tier 2** above (the practical default).
2. **ETW capture from an elevated shell**, then analyse on a machine with WPA
   (or via PerfView) — the trace format is portable. `profile.ps1 -Action
   etw-capture` wraps this:
   ```powershell
   # elevated PowerShell:
   powershell -NoProfile -ExecutionPolicy Bypass -File tools\profiling\profile.ps1 -Action etw-capture -Pops 2000 -Ticks 400
   # equivalent manual steps:
   wpr -start CPU -filemode
   target\release\rustibites.exe bench --bibites 2000 --ticks 400 --threads 1 --repeat 1
   wpr -stop tools\profiling\results\cpu.etl
   tracerpt tools\profiling\results\cpu.etl -o tools\profiling\results\cpu.csv -of CSV -y
   ```
   Then open `cpu.etl` in WPA → "CPU Usage (Sampled)" (or PerfView). With the
   default release binary the process will show up but symbol resolution inside
   `rustibites.exe` needs the symbol-friendly build below.
3. **Build with symbols without editing Cargo.toml** (isolated target dir):
   ```powershell
   powershell -NoProfile -ExecutionPolicy Bypass -File tools\profiling\profile.ps1 -Action build-symbols
   # == cargo build --release -p bibites-cli --target-dir target/profiling `
   #      --config profile.release.debug=true --config profile.release.strip=false
   ```
   The root manifest is Lead-owned and stays untouched; these are Cargo config
   overrides. Env-var equivalent:
   `$env:CARGO_PROFILE_RELEASE_DEBUG='2'; $env:CARGO_PROFILE_RELEASE_STRIP='false'`.
   The build uses `target/profiling` so it does not disturb the normal
   `target/release` binary, but it is a full LTO build and roughly doubles
   target disk usage (measured on the reference machine: **2 m 18 s**, 1.88 MB
   exe + 22.7 MB `.pdb`, exe SHA-256 `8B8D6BF4…`; recorded in
   `tools/profiling/results/symbol-build.json`). The default `[profile.release]`
   strips debug info (`Cargo.toml` `strip = "debuginfo"`), so the normal exe is
   not symbol-friendly even though a stale `.pdb` may sit next to it.

If a sampling tool is approved later, the workflow is the same: build with
`-Action build-symbols`, run `bench`/`run` under the profiler, filter to the
`rustibites` process, ignore the first iterations (allocation/caching warm-up),
and report the symbol build's SHA-256 with the capture.

## Adding instrumentation (crate-owner change)

`tools/profiling` cannot add timers by itself: `crates/` is owned by the
subsystem owners and root `Cargo.toml` is Lead-owned. The exact hooks:

- **New top-level phase**: add a variant to `Phase` (`sim.rs:324`), bump
  `PHASE_COUNT` and the `ALL` array order (`sim.rs:362`/`:366`), add the
  `name()` arm (`sim.rs:388`), then wrap the section in `World::step` with
  `self.mark_phase(Phase::X, start)` like `world.rs:2654`–`:2691`. The CLI phase
  table and `tools/bench/run_phases.ps1` pick it up automatically; for the CSV
  column order also extend `$script:BenchPhaseNames` in
  `tools/bench/bench_common.ps1` (unknown phases are still in the raw table).
- **Nested timing** (e.g. brain pass vs organ loop *inside* body): either add
  new `Phase` variants (changes tick-order semantics only for counting) or add a
  manual `#[ignore]` probe test with `Instant` around the section, following the
  `perf_*` pattern; the probe keeps non-profiling builds unaffected.
- **Allocation counting**: a test-only `#[global_allocator]` counting allocator
  (owner-approved, cfg(test)) or a `dhat` dev-dependency (needs a manifest
  change/Lead approval). Not available externally today.
- **Coverage/PGO** (`cargo llvm-cov`, `-Cprofile-generate`) needs `llvm-tools`
  + LLVM coverage support and is out of scope for this harness.

Ask the owning agent (body/brain/world/io) for instrumentation changes; this
task's scope is only `tools/profiling/` + this document.

## Current hotspots (as of the last audit)

- **physics + body = ~85–90%** of the tick at every measured population; within
  them the known candidates are the per-record organ loop (body) and the
  broadphase/narrowphase (physics). Phase tables cannot split inside a section —
  that is what a sampling profiler or a new nested timer would add.
- **Clustered worlds are catastrophic** in physics (latest tracked probe on the
  final binary: spread 2k 1.10 ms vs cluster 2k **159.09 ms** vs cluster 4k
  **627.24 ms**; earlier binary snapshot: 221.5 ms / 849 ms; an older
  loaded-machine audit saw 1.75 / 393 / 1,520 ms). The `run_cluster.ps1`
  whole-engine mode shows the same cliff at **~94 ms physics** for 2,000 bibites
  in a 100×100 box. The broadphase fallback is the known fix target (task-82:
  cluster2000 physics < 50 ms). The spread throughput benchmarks do not cover
  this; use the cluster artifacts for worst-case planning.
- **Sensing** is the only phase that parallelises well (forced-due ~2.7× at 8
  threads in one clean-ish run) but is only 3–18% of the tick; per-due FOV is
  cheap (≈0.28 µs/bibite), and query/sort overheads are the remaining cost.
- **Parallel-executor activation** is ~34 µs for eight 1-item jobs vs ~0.8 µs
  serial: parallel work must batch chunks to win; relevant for the planned
  parallel body pass-B.
- **Memory/locality**: `BibiteRecord` is 1.7 KB and each brain owns ~18 small
  Vecs (~2.6 KB + allocations); ARCHITECTURE §10/task-48 has the full audit.
  This is a cache/locality opportunity, not a separate phase.

## Reporting template

Copy this into the PR/issue (and keep the artifacts under
`tools/profiling/results/` or the branch):

```markdown
### Profiling: <change> — before/after

- Date (UTC): <2026-…>, host: <machine>, load during run: <cpu_load_pct range>
- Git: <sha> (dirty: <n paths>), rustc: <version>
- Binary (before/after SHA-256): <…> / <…>
- Command: `profile.ps1 -Action phase-audit` (+ `-Action bench-phases -Pops …`;
  `tools/bench/run_cluster.ps1 -CoreProbe` for the dense-pile regression)
- Artifacts: <paths or branch>

| Population (threads) | before ms/tick | after ms/tick | delta | physics before→after | body before→after | sensing before→after |
|---|---|---|---|---|---|---|
| 2,000 (1) | | | | | | |
| 10,000 (1) | | | | | | |

Notes: <Amdahl/thread effects, parity/gates run, noise caveats, what was NOT
covered (e.g. clustered physics)>.
```

Rules for trustworthy numbers: same binary build/commit, same tick counts,
same scenario (synthetic density-scaled world unless stated), idle machine or
recorded load, and best-of-N for wall time with medians reported too. Never
compare the absolute ms from a build with different `[profile.release]`/LTO
settings, or from a run with a background `cargo build`.

## Artifacts checked in

| File | What | Caveat |
|---|---|---|
| `tools/profiling/results/doctor.txt/.json` | Tool inventory + binary/PDB status on the reference machine | point-in-time |
| `tools/profiling/results/phase_audit.log/.meta.json` | Full ignored `perf_*` audit output, command, duration, git/load | captured with `cpu_load_pct = 100`; read shares |
| `tools/profiling/results/micro_probes_*.log` | Body broadphase, executor, sensing probes | same load caveat |
| `tools/profiling/results/bench-phases/` | Phase tables for 1k–20k x 1/8 threads | same load caveat |
| `tools/bench/results/cluster/` (owned by the bench harness) | Dense-pile whole-engine + core-probe capture, cluster-vs-spread ratios, world SHA-256, determinism | captured on a shared machine; compare binary SHA-256 |
| `tools/profiling/results/symbol-build.json` | Symbolised profiling exe + PDB paths/SHA | local path, machine-specific |
| `tools/profiling/results/etw-capture-blocked.txt` | What WPR needs (elevation) when run from a normal shell | point-in-time |

## Known gaps (things this harness cannot do here)

- No symbolicated stack sampling without an elevated shell **and** a `.etl`
  analyzer (WPA/PerfView not installed); `tracerpt` CSV gives event-level, not
  stack-level, attribution.
- No flamegraphs (`samply`/`cargo-flamegraph` not installed; installs need Lead
  approval).
- Phase timers are section-level only; nested attribution requires crate edits.
- No allocation/memory profiling beyond `size_of` and process peak working set
  (see `BENCHMARKS.md`).
- All numbers here are from a shared machine: treat single-run absolute values
  as ±30%, use medians/repeats, and keep the load samples with the artifacts.

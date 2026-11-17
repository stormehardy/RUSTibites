# RUSTibites — Architecture

RUSTibites is an open-source, readable, high-performance reimplementation of
[The Bibites](https://thebibites.com/) in Rust. The reference implementation is
the decompiled Unity/C# source in `Here/`; behaviour is reverse-engineered from
that source and validated against the installed game.

This document is the engineering contract for the project: module boundaries,
data layout, determinism rules, fidelity policy, and the validation strategy.
It is intentionally stable — change it via the Lead, not ad hoc.

---

## 1. Product definition

**Milestone 1 (current): a headless simulation engine + CLI benchmark.**

- Load a real Bibites world/scenario or genome.
- Run the full simulation (brains, bodies, organs, ecology, evolution).
- Export files the original game can load again (full `.bb8` round-trip).
- Report deterministic statistics, and run far faster than real time.

**Non-goals for milestone 1:** rendering, UI, sound, Steam integration,
modding API, multiplayer. A viewer is a later milestone and must not slow down
the headless core.

### Success criteria

1. `rustibites run world.zip --ticks N` produces a sensible world; populations
   survive, reproduce, evolve, and die across long runs.
2. Round-trip: a world/genome written by RUSTibites loads in the original game,
   and vice versa.
3. Behavioural parity: with identical inputs (scenario, seed, settings), core
   statistics track the original game closely (see §9 for tolerances).
4. Throughput: large populations (10k–100k bibites) simulated many times faster
   than real time on a 6c/12t desktop (targets in §10).

---

## 2. Crate graph

```
bibites-core   (no external dependencies, no I/O, no threads-of-its-own)
    ▲
    │
bibites-io     (file formats: .bb8 genomes, .bb8settings, saves, scenarios)
    ▲
    │
bibites-cli    (argument parsing, headless runner, benchmark harness)
```

- **`bibites-core`** — pure simulation. Must remain `#![forbid(unsafe_code)]`
  and dependency-free. It may use `std` threads internally via an explicit
  runtime configuration, but never spawns threads implicitly at import time.
- **`bibites-io`** — serde/zip based readers and writers. Depends on
  `bibites-core`; core never depends on io.
- **`bibites-cli`** — UX only. All simulation logic belongs in core.
- **`bibites-view`** (later) — optional viewer, depends on core+io.

Adding an external dependency to `bibites-core` requires justification: the
simulation should stay auditable and portable. `bibites-io` may use `serde`,
`serde_json`, and a zip/flate crate.

---

## 3. Module ownership

Write scopes are enforced socially via the shared task board. One owner per
module; cross-module changes go through the Lead.

| Area | Files (in `crates/bibites-core/src/`) | Owner | Spec |
|---|---|---|---|
| Foundation | `math.rs`, `rng.rs`, `arena.rs`, `ids.rs`, `error.rs`, `clock.rs`, `lib.rs` | Lead | this doc |
| Brain & genetics | `genome.rs`, `brain.rs`, `species.rs` | brain-dev | `docs/spec/01-brain-genetics.md` |
| Body & organs | `body.rs`, `organs.rs`, `physics.rs` | body-dev | `docs/spec/02-body-organs-physics.md` |
| World & ecology | `world.rs`, `zone.rs`, `pellet.rs`, `pheromone.rs`, `senses.rs`, `tools.rs`, `sim.rs` | world-dev | `docs/spec/03-world-ecology.md` |
| Settings | `settings.rs` | fmt-dev | `docs/spec/04-formats-settings.md` |
| Formats | `crates/bibites-io/**` | fmt-dev | `docs/spec/04-formats-settings.md` |
| CLI & parity | `crates/bibites-cli/**`, `tools/parity/**` | Lead | this doc |

Modules may grow into directories (`organs/mouth.rs`, …) by replacing the
placeholder file with a `mod.rs`; the owner controls that subtree.

---

## 4. Data layout and hot-loop rules

The original game is object-oriented Unity C#: one `MonoBehaviour` per organ,
`List<>`s of components, per-tick delegate dispatch. We deliberately do not
mirror that. The Rust core uses:

- **Structure-of-arrays** for per-entity state that is touched every tick
  (position, velocity, energy, brain CSR, organ scalars). Arrays are indexed by
  a stable slot id.
- **Generational arenas** ([`Arena`](../crates/bibites-core/src/arena.rs)) for
  entity identity: stable handles, O(1) insert/remove, dense iteration, stale
  handle detection.
- **Free lists / swap-remove** for transient objects (pellets, pheromone
  spots). Pellets are numerous but short-lived; they live in chunked/SoA
  storage with a spatial hash rather than in individual heap objects.
- **No allocation in the tick loop.** Scratch buffers (query results, contact
  lists, sort keys) are owned by the simulation and reused; capacity grows on
  demand and is reported in stats.
- **`f32` everywhere**, matching the game. `f64` only for reporting
  (wall-clock timers, accumulators that must not drift over days).
- **Branch-light inner loops**, no dynamic dispatch, no `Rc`/`RefCell` in the
  simulation. Collections are indexed by plain `usize`/`u32` slot ids.

Rationale: the bottleneck in an evolution sim is the per-tick cost of thousands
to hundreds of thousands of agents, dominated by sensing, brain evaluation, and
collision queries. SoA + flat arrays + no allocations gives the compiler
autovectorisable loops and lets us parallelise by entity range.

---

## 5. Determinism and randomness

A run is fully determined by `(seed, scenario file, settings file, tick count)`.

- One authoritative random source: [`Rng`](../crates/bibites-core/src/rng.rs)
  (xoshiro256\*\*, seeded via SplitMix64).
- Simulation code must not use `std::time`, `SystemTime`, thread-local RNG, or
  hash-map iteration order for anything that affects state.
- Per-entity streams come from `Rng::derive(seed, domain, entity)` or
  `Rng::fork(domain)`, so parallel work is order-independent.
- Parallel mode must produce identical results to serial mode. If a system
  cannot meet that bar, it gets an explicitly documented ordered reduction.
- `HashMap` is allowed only for lookups whose iteration order is never
  observed; use `BTreeMap`/sorted vectors where order matters.

---

## 6. Simulation pipeline

Fixed timestep: `dt = 1 / sim_tps`, scenario-independent default `sim_tps = 30`
(`ScenarioIndependentSettings.simTPS`; scenarios may override it, e.g. 15 or 40).
There is no interpolation in the headless engine. Brain updates happen every
`brain_tps` simulation ticks (default 2), so stateful neurons receive
`brain_period = brain_tps / sim_tps` seconds (default `2/30`). Each tick is a
single `World::tick()` call executing systems in a fixed order:

```
1.  clock / settings animators (time-driven scenario mutations)
2.  world tools (towers, colour killers, events)
3.  zone spawning & plant biomass growth (pellet creation/decay)
4.  sensing: vision (FOV) + pheromone sensing   [parallelisable]
5.  brain ticks (due brains only)               [parallelisable]
6.  organ updates: clock, growth, stomach, fat, armour, egg, ...
7.  propulsion intent -> forces/torques
8.  physics integration + collision resolution  [custom solver]
9.  eating / attacking / grabbing resolution
10. reproduction, egg laying & hatching
11. death, corpse creation, cleanup
12. statistics, lineage & species bookkeeping (throttled where the game does)
```

The exact order and per-system sub-order must match `docs/spec/03-world-ecology.md`
(the authoritative reverse-engineered order). Items are marked parallelisable
only when each entity's work is independent and order-independent; the Lead
must sign off before enabling.

**Frame-rate independence:** anything the original updates in `Update()` rather
than `FixedUpdate()` is modelling/UI-only unless the spec proves it feeds the
simulation. Where the game is frame-rate dependent, we reproduce the
configured-TPS-equivalent behaviour and document the deviation.

---

## 7. Physics (intentional divergence)

Unity uses Box2D (`Rigidbody2D`, `CapsuleCollider2D`, contact solver). We do
**not** port Box2D; we implement a compact 2D solver tuned to this game:

- Shapes: circles for pellets, capsules/circles for bibite bodies, static
  rectangles for zone bounds where needed.
- Broadphase: uniform spatial hash grid (cell size ~ typical interaction
  radius), rebuilt incrementally per tick; reused buffers.
- Narrowphase: circle–circle, circle–capsule, capsule–capsule.
- Resolution: impulse-based with layered collision matrix, mass/inertia from
  the body spec, fixed iteration count, deterministic contact ordering (sorted
  by entity id).
- Bibite movement is applied as forces/torques exactly as the game's
  `BibitePropulsion` does, then integrated semi-implicitly.
- Grabbing/attacking/damage use the spec's formula on contact, not Box2D's
  contact impulses.

**Fidelity contract:** signature behaviours must match — reaching plants,
being pushed by collisions, mouth contact causing bites, mass/size affecting
acceleration and turning. Exact contact-level trajectories may diverge; parity
tests therefore compare statistical/behavioural invariants first, and
per-entity trajectories only where physics is not involved (brains, senses,
energy accounting).

---

## 8. File formats and round-trip

`bibites-io` models every format as serde types, preserving unknown fields so
that a load→save round-trip does not strip data written by the game.

- `.bb8` — genome/bibite template: genes + brain JSON. Import and export.
- `.bb8settings` — scenario-independent + scenario + zone settings.
- save `.zip` bundle — world sections (scene, pellets, bibites, species,
  settings, zones, ...) plus binary sections where the game uses them.
- scenarios `.zip` — settings, zones, initial pellets, tags, recommended list.
- `speciesData.json` — lineage/species browser data.

Requirements: canonical field ordering where the game requires it, float
formatting compatible with the original parser, version migrations applied on
load and written at the present version on save. A Rust-written file must load
in the original game (verified in `tools/parity`).

### Round-trip acceptance (2026-10-30)

All three Rust-written artifact classes have now been started by the real
0.6.3.1 game through the headless BibitesAPI lab (`tools/roundtrip/`):

- **save** (`tools/roundtrip/load_rust_save.py`): `rust_save.zip` (from
  `lol.zip` + 100 ticks) loads, `inSimulation=true`, counts `{bibites:24,
  corpses:14, eggs:1}` exactly match the CLI run, `data.bin` survives
  load→save→load, 0 new `Player.log` exceptions; artifact
  `testdata/roundtrip/acceptance.json`.
- **template** (`crates/bibites-io/examples/export_template.rs`): the game's own
  `/bibite/upgrade` converter accepts `rust_template.bb8template`
  (`fromVersion 0.6.3.1`, `updated=false`).
- **scenario** (`tools/roundtrip/start_rust_scenario.py`): `Ring.zip` re-emitted
  by `write_scenario_bundle` starts a new world (`new_game`): `inSimulation`,
  4 zones (equal to the original Ring run), spawner output `1` bibite (equal to
  the original run), paused at `simulatedTime≈0.267`, runs a further second to
  `t≈1.267`, 0 new exceptions; artifact
  `testdata/roundtrip/scenario_acceptance.json`. Pellet seeding is compared statistically (initial planting uses unseeded UnityEngine.Random): seeded amount within 2%, pellet/bucket counts within 3σ (floor 10), per-zone buckets via /pellets/select; per-pellet identity is not compared.

The same `LiveRuntime`+`Lab.ensure_game(window="hidden")` recipe as the parity
harness is used; the lab is non-destructive and always stopped.

---

## 9. Validation strategy

Three layers, from cheapest to strongest:

1. **Unit tests in core** — formulas, gene tables, brain activations, energy
   accounting, mutation ranges, arena invariants.
2. **Golden tests** — parser/serializer round-trips on real files from
   `C:\Users\Storme\AppData\LocalLow\The Bibites\The Bibites`; fixed scenarios
   run N ticks with a fixed seed and compare against checked-in digests.
3. **Ground-truth parity** — the installed game is instrumented with a small
   BepInEx plugin (`tools/parity/plugin`) that removes the two frame-rate
   couplings (pinned `fixedDeltaTime`, pause inside load-completion) and dumps
   per-tick telemetry (aggregate series + periodic per-entity snapshots) as
   gzip JSONL with a manifest. `tools/parity/compare.py` diffs two captures
   with per-system gates and three profiles (`exact`/`default`/`loose`).
   Full design, schema and measured determinism: [`PARITY.md`](PARITY.md).
   - **Formula-exact systems** (brain evaluation given identical inputs, senses,
     energy accounting, gene-derived values): expected to match to `f32`
     rounding. Mutation *outcomes* cannot match bit-for-bit: the original draws
     mutation randomness from unseeded `System.Random` / global
     `UnityEngine.Random`, so mutation is validated statistically.
   - **Physics-involved systems** (positions, contacts): compared via
     distributions and invariants, not exact trajectories, unless/until the
     solver is tuned to parity.
   - **Ecology**: population curves, biomass, extinction events over long runs.

   Captures without the plugin (bridge-only) are single observations, not
   reproducible ground truth and must only be compared with the `loose`
   profile.

Parity captures are stored outside the repo (they can be large); only small
fixtures and the schema are checked in.

### Current parity status (2026-10-30)

**Final status: parity closed on the canonical Default world.** The official
post-fix capture (body `55cda1f` / task-106, commit `2fb2ad4`) closes the
acquisition/feeding gap: contacters 8/10 on both engines, fed bibites 8/8 both,
intake 68 vs 71 (600 t) and 249 vs 263 (3000 t), consumed 216.36 vs 230.24 and
822.95 vs 868.33 u², u²/bite 3.182 vs 3.243 and 3.305 vs 3.302, median idle gap
7/7 and 9/9. The residual is +6.4 % / +5.5 % Rust over-consumption, inside the
single-pass/default 10 % band (`docs/PARITY_ROUND2.md` §10.4.1). The earlier Ring
comparison (base `ring_parity.zip`, 300 ticks at 15 TPS) remains a historical
fixture; policy now starts from `testdata/real/Default.zip`:

- identical: `settings_hash`, 35-gene order, tick coverage, tick-0 entity
  `genes/sense/output/brain` digests, entity genes/energy at both snapshots,
  species/generation/tag/node/synapse counts, all 15 brain output means,
  moving/bite/damage/egg-rate curves;
- within tolerance: pellet/plant biomass (unseeded game pellet RNG is
  statistical by design), vitals energy/health; conservation is gated on both
  sides (game `harmony_exact`, Rust `counters`) and passes at 600/3000 t;
- expected divergence (intentional): entity positions/velocities, accumulated
  energy at later ticks, and `entity_brain` digests once trajectories separate
  (custom solver vs Box2D, see §7).

A Rust-written save loads and runs in the real game with matching counts and no
new log exceptions, and a Rust-written `.bb8template` is accepted by the game's
own converter.

**Checkpoint/resume is closed for the 120 / 1k / 2k / 5k / 10k scale and the
100k evolution run:** those Default/synthetic runs resume bit-exactly for both
2× and 4× chunked splits (final fingerprints match single runs); the 100k
single-run fingerprint `8688404363908086484` matches the 50×2,000 chunked run
(10k `1750137736440958460`; fingerprints are tree-specific) after the dead-slot
pellet/pheromone generation fix (`8c799ec`). Real saves
restore byte-identically after task-125/126, with only a benign lazy egg
`plan_dirty` flag noted in `docs/design/CHECKPOINT_STATE.md`.

**Viewer/HUD parity:** alpha-mask IoU **0.9890** overall, **0.9680** for the
playback speed label (target met), **0.9154** ruler and **0.9920** expanded Stats;
runtime Genes/Biology/Brain inspector tabs **0.9887 / 0.9815 / 0.9877**
(`docs/VIEWER.md`); remaining visual delta is speed-label thickness/TMP SDF
raster quality. Final portable release **24.02 MB**, SHA-256
`836c136c…` (full hash in `docs/INSTALL.md`).

**Settings/format audit (2026-10-30):** all **78 scenario + 14 independent + 36
material** settings round-trip exactly through both save- and scenario-shaped
`.bb8settings`; the bridge `/settings` JSON key sets/types match the game; defaults
and migration behaviour are unchanged (no frozen-package/doc-default impact).
Rust-written saves remain game-loadable, and `.bb8state` is a sibling sidecar,
never inside the game zip. Details: `crates/bibites-io/tests/settings_audit.rs`;
the per-section load/save/reload fidelity matrix and version rules are in
[`SAVE_COVERAGE.md`](SAVE_COVERAGE.md) (newer than 0.6.3.1 and 0.6.4a1 folder
saves are rejected as `Unsupported`).

---

## 10. Performance targets and measurement

Benchmark harness: `rustibites bench` runs a fixed scenario for N ticks,
reporting:

- wall time, simulation ticks/second, real-time factor (`sim_time / wall_time`)
- bibite-updates/second (the primary throughput number)
- peak resident memory, allocation count in the hot loop (instrumented build)
- per-phase timing breakdown (`World::phase_timings()`, always-on, sub-µs
  overhead; printed by the human bench output)

Reference machine: Ryzen 5 3600 (6c/12t), 16 GB, release build.

**Measured baseline (final matrix, task-131, 2026-10-30):** clean-tree release
binary `3CCB231ECC3C9027B83CE998E6BB88FAFA4224D27874547B386F94EA84709D1D`
(2,168,320 bytes, built 2026-10-30T14:38:42Z from a clean worktree at git
`2fb2ad4`; the main tree only had non-Rust docs/results dirty at capture, and
none of those affect the binary). Bench binary from
`cargo build --release -p bibites-cli -p bibites-view`; synthetic
density-scaled world, 30 TPS, **120 ticks/run, best-of-3, idle-machine guarded**
(`-MaxLoadPct 45`, pre-run CPU load 2–8%, all 15 cells ok), serial/parallel
fingerprints identical. Artifacts: `tools/bench/results/final/`
(`environment.json`, `summary.csv`, `runs.csv`, `binary-provenance.json`).
**The final matrix remains authoritative for the current wave:** work since
task-131 is viewer/docs/assets-only (Tail limitation, GPU hue flag, runtime-UI
wave), with no engine/perf change, so there is no new throughput delta to quote
from HEAD `bbf6701`; the measured commit remains `2fb2ad4`.

**Canonical Default scenario (final, task-131):** **16,676 ticks/s serial
(555.9× real time)** — 8 threads 16,117 ticks/s, median 16,293, 14.8 MB peak WS;
the real-scenario path is dominated by startup (see
[`BENCHMARKS.md` §Canonical scenario](BENCHMARKS.md#canonical-scenario-defaultzip)),
and the full 1k–20k synthetic matrix is below.

| Population | Serial (ticks/s) | 4 threads | 8 threads | Serial bibite-updates/s | Peak WS |
|---|---|---|---|---|---|
| 1,000 | **1,483** (≈49.4× real-time) | 1,361 | 1,356 | 1.48M | 10.8 MB |
| 2,000 | **715** (≈23.8× real-time) | 649 | 652 | 1.43M | 17.1 MB |
| 5,000 | **220** (≈7.3× real-time) | 216 | 220 | 1.10M | 34.7 MB |
| 10,000 | **94** (≈3.1× real-time) | 94 | 95 | 0.94M | 64.7 MB |
| 20,000 | **41** (≈1.4× real-time) | 42 | 42 | 0.83M | 125.4 MB |

These are best-of-3 values; medians are within a few percent (1,487 / 723 / 217 /
95 / 41 ticks/s serial at 1k/2k/5k/10k/20k). Against the task-92 published table
(`4F267FF9…`: 1,445 / 680 / 200 / 90 / 42), the new clean-HEAD serial numbers are
**+2.6% / +5.1% / +10.0% / +4.4% / −1.6%**; peak WS grew to 10.8 / 17.1 / 34.7 /
64.7 / 125.4 MB (+8% / +14% / +10% / +12% / +14%) from the body/checkpoint/FOV
changes since that build. A provisional matrix on the previous clean HEAD
(`67472e5`) measured 1,438 / 680 / 194–231 / 94 / 42–43 best-by-population; the
final deltas are within the shared-machine noise floor (±5–10%), so there is no
measurable rotation-fix performance effect. **Historical note:** the earlier
PERF-round-3 baseline (`f984d50`, 2026-10-30) was 1,595 / 764 / 253 / 109 ticks/s
serial at 1k/2k/5k/10k with ≈1.0× thread scaling; it was a different build and
measurement protocol (4-tick isolated-target averages). The current table is the
reference; full reconciliation is in
[`BENCHMARKS.md` §Comparison](BENCHMARKS.md#comparison-with-architecture-10).

Per-tick phase split (serial, final phase capture, 300 ticks/run,
`tools/bench/results/final-phases/`):

| Population | ms/tick | physics | body | sensing |
|---|---|---|---|---|
| 1,000 | 0.726 | 0.388 (54%) | 0.251 (35%) | 0.079 (11%) |
| 2,000 | 1.429 | 0.794 (56%) | 0.500 (35%) | 0.123 (9%) |
| 5,000 | 4.739 | 2.395 (51%) | 1.974 (42%) | 0.327 (7%) |
| 10,000 | 10.753 | 5.568 (52%) | 4.370 (41%) | 0.710 (7%) |

The sensing share dropped (19/13/9/8% → 11/9/7/7%) after the FOV split; physics
and body still dominate (~85–91% of the tick). Phases are single 300-tick runs,
so totals run a few percent above the best-of-3 matrix cells.

The physical broadphase is a body-merged AABB sweep with an x-order that is a
total order (start of the box, then body index) carried from the last tick and
repaired by insertion sort, or sorted from scratch on the pool when the bodies
changed (grid fallback for clustered worlds), cached per-record collider shapes refreshed only when body size changes,
and analytic capsule/box AABBs. Sensing pre-filters due bibites before moving the
spatial hash (most ticks are no-op), reuses per-worker scratch and result
buffers, keeps the hash's deterministic unsorted FOV order, and rejects
candidates with a cheap cross/dot cone test before the exact atan2 edge solve.
Per-due-bibite FOV is ≈0.28 µs. Serial throughput improved ≈2.4× over the first
working engine (2,000 bibites: 4.47 ms → ≈1.09 ms per tick). **Historical
fixture note:** the `Ring.zip` save (pellet-heavy) ran ≈1,490 ticks/s (99×
real-time) on the earlier round-3 build — that is a historical Ring measurement,
not the canonical baseline (the canonical real world is
`testdata/real/Default.zip` per `docs/TESTING_POLICY.md`). The final capture is
synthetic-world only and did not re-measure the Ring save. **Definitive Lead thread A/B (X32):** explicit threads are no longer
regressed — 5k serial206.6/4t202.7 (0.981)/8t206.5 (1.00); 10k90.3/90.3 (1.00)/
86.5 (0.958); 20k36.87/35.2 (0.955)/39.8 (1.08); all within 4–5% noise, 8t
helps 20k, fingerprints deterministic. Physics+body remain largely serial, so
the 1.8× body pass-B/organ-loop work is a future optimization, not a regression.

**Parallel tick and resting pellets (2026-11-05).** A persistent rayon pool (`sim::ThreadPool`, `ParallelGates`
for the size from which each stage uses it) runs body pass A, the brain inputs and batches, the physics bridge's body
building, the AABBs, the poses, the narrowphase, the broadphase sweep (cut by first entry) and the x-order sort; every
stage cuts its data by index and joins the cuts in order, so results are bit-identical whatever the pool size (tests
compare whole checkpoints byte for byte, serial against 3/4/8 threads, with every gate at 1). Body pass B runs on the
pool for the bibites that touch nothing but themselves (`pass_b_heavy_reasons` is zero: nothing in the mouth or held, no
egg to lay, no pheromone to emit, no vomit); the others keep their dense-order turns and read the targets that come after
them from snapshots taken before the pool ran. Pellets at rest (their pose came out of a step bit for bit as it went in,
nobody holds them) are kept out of the physics step in a grid of sleepers (`pellet_sleep`); a body's box that overlaps a
sleeper promotes it into the step, and any write to a pellet through the store wakes it, so the world is identical to the
one the full step gives (checked byte for byte, `RUSTIBITES_GATES=nosleep` for the reference). The pellet spatial-hash sync
visits only the pellets the store marked since the last sync. See [BENCHMARKS.md](BENCHMARKS.md#parallel-tick-and-resting-pellets-2026-11-05).

**The pool sized to the work, then measured (2026-11-15; the owner: "make multithreading ALWAYS run", then, shown what that cost, "I want it" to the version that sizes the pool to the work, then "the policy should be good for all machines").** The sizes from which a stage used the pool
(`ParallelGates`: 3,000 records, 4,096 bodies, 20,000 due bibites for sensing, ...) were wrong in both directions: too high for the stages that pay at a few hundred bibites (the narrowphase, the bibites' body build), and the
default put every stage of every world on the pool the first time it was tried, which made worlds under about 1,300 bibites several times slower (a fixed hand-off per stage, some twenty a tick). A world with more than one thread
(`World::set_threads`; the viewer's and the CLI's `--threads`, `0` = all cores, is the default) runs each tick one of two ways. *Inline*: on the calling thread, and the world holds **no pool handle** during it
(`World::pool_wanted`; every stage reads `ParallelExecutor::Serial`), so no stage can pay a hand-off, and the stages around it do not pay for data it left in other cores' caches. *On the pool*: the whole tick runs on one of the
pool's workers (`World::tick`: the stages are calls from a worker, not hand-offs from the calling thread, which blocked and was woken some twenty times a tick) and every stage
that can use the pool is on it, a worker for every quarter of the stage's size up to the pool (`ParallelGates::workers_for`, `chunk_len`), except the stages that measurably lose in worlds of a few thousand bodies, which keep a size of their own
(the cheap per-body physics stages and the bridge's work after the step 3,000 bodies, the x order sorted from scratch 4,096, the pair sort 8,192, the scans of the summaries 8,192; the three sizes that were literals in `physics.rs` -
the sorts from 4,096 bodies, the islands of the fine steps from 256 held bodies and of the soft step from 512 constraints - are gates too; islands are handed out eight at a time and need sixteen of them). **Which way a tick runs is measured, not written** (the owner: "the policy should be good for all machines"). Two things depend on the machine (its cores, how fast the OS wakes one, what else is running) as much as on the world: the
size from which the pool pays at all (about 1,200 bibites on the 6-core / 12-thread machine the sizes were measured on; from a few hundred on an emulated pool of two to six workers) and how eagerly the stages of a pooled tick should use it (on 2 to 6 workers every stage from one
item beat the sizes above by 6-19 % at 500-2,000 bibites; on 12 it loses 2.5x at 200 bibites). So a world with `ParallelGates::adaptive` (the default) owns a `PoolTuner` (`pool_tuner.rs`, a state machine without a clock: `World::choose_tick_arm` asks it how the next tick
runs and whether to time it, `World::tick_in` runs the tick that way and tells it the time) that chooses among five ways: inline, and the pool at four *levels* (`ParallelGates::pooled_level`: the stage sizes above times 1, 0.3, 0.08, and every size at 1). The incumbent way runs and
its mean tick time is kept (a moving average without the hiccups); every other way is looked at for a block of 12 to 100 ticks now and then (a probe, dropped early when it is clearly worse), the nearest first, each way waiting twice as long after each loss (300 ticks, 600, up to 8,192:
about 1 % of the ticks at most for a settled world); a way that wins is not believed until it wins a second look (the incumbent is measured again from scratch in between: timings are noisy in one direction, another program takes a core); the pool takes over from inline only with a
mean under 90 % of the inline mean (it also takes cores from everything else on the machine), inline takes back within 5 %, and between levels a more eager one has to be 4 % better; the measuring starts again when the world's size changes by 41 %; a world under a quarter of the first guess
(300 bibites and 750 bodies: a tick of it takes a fraction of a millisecond) is never timed or probed. `pool_bibites` / `pool_bodies` are only the first guess and the floor; with `adaptive` off they are a plain switch. `RUSTIBITES_GATES=fixed|pooled:<level>|always|eager|nosleep|scaled:<percent>`
picks other policies in the viewer and the CLI (`pooled:<level>` = one level for every world, to measure the levels one by one); `--threads 1` is the serial tick. Results do not change: serial and pool runs are bit-identical, whichever way and level each tick runs (whole checkpoints compared
byte for byte, with every size at 1, with the sizes a world starts with, and in worlds that change way and level from tick to tick).
In the window the frame's ticks run on the tick helper thread (`tick_overlap`) whatever the world's size: the 0.5 ms floor (`OVERLAP_MIN_TICK_SECS`) is gone, and only the first frame after a world is built, before a tick has
been timed, runs its own ticks. `--sim-thread` stays opt-in (it changes what the window can do while it runs). See [BENCHMARKS.md](BENCHMARKS.md#multithreading-sized-to-the-work-2026-11-15).

**Scans over the records and the bridge's sweep (2026-11-05).** What the physics bridge does to each bibite after the step (the contact counters of the step and the tags it touched, the pose and the
rigid-body cache written back, the mouth's target list from the triggers, the prune) is one function over one record, `after_step`; the step's contact and trigger events are cut into one row of items per
bibite body in the order of the events (`BridgeRows`), so the pool can sweep the records by slot range and every record still sees its events in the order the serial loops gave them. The store logs the
bibites it removes (`BibiteStore::drain_removed`) and the spatial hash drops exactly those entries, in handle-slot order, instead of asking every entry whether its record exists (a checkpoint restore, which
the log does not cover, takes the full pass); `kill` and a restored checkpoint raise `kills_queued`, without which the scans for queued deaths are skipped; and the scans that must look at every record (rad
bibites, the species recount, the sensing prefilter) go through `BibiteStore::filter_map_dense_into`, the dense list cut into stretches whose answers are joined in order. Tests compare each of these with the
code it replaced, on a copy of the records or of the hash. On a pool-sized world the body tick's pass A, the brain steps and the pool's share of pass B are one sweep over blocks of records
(`BibiteStore::fused_body_tick`): each reads and writes only its own record, so a worker takes its stretch of records through all three; the state a dense-order turn reads of a target that comes after it is taken in
the sweep, by the worker that has the target's record, before the target's own turn, and the turns that touch the world's stores follow in dense order (`pass_b_dense`). The serial path keeps the passes apart and is
the reference the pooled one is compared with. In a world with enough pellets for the resting-pellet layer (2,048), the pass that puts back into the step the sleepers a body overlaps computes the box of every body and asks the
grid of sleepers about it; the pool does that in stretches (`PelletSleep::overlapping` only reads the grid) and the sleepers are marked afterwards in the order the plain loop marks them in (`promote_slots`), and the grid
keeps an occupancy bit per cell. The bibites that hold something are found by a pooled scan (`BibiteStore::holders`). See [BENCHMARKS.md](BENCHMARKS.md#scans-over-the-records-the-bridge-sweep-and-the-narrowphase-2026-11-05-evening).

**The physics bridge and the lines of a record (2026-11-05, late night).** At 100,000 bibites the tick is limited by the cache lines each pass pulls through memory (27 GB/s), so the passes over the records are written for the lines
they touch: `BibiteRecord`'s first four lines are what the bridge needs (line 0: `derived` and the pose, line 1: forces, id, contact counters, flags and the sense throttle, lines 2-3: the colliders), and the bridge is built on one
invariant, the physics cache. The end of a bibite's turn (`pass_b_live`, light or dense-order) calls `BibiteRecord::refresh_physics_cache`: the rigid body (computed from the organs the turn has just been through) and the geometry
copies go to `derived`, and `phys_stamp` is set to the epoch of the body tick (`BibiteStore.epoch`, a new one at the start of every `tick_inner`). `build_bibite_physics_body` takes the rigid body from `derived` when the stamp is
current, and the sweep after the step (`after_step`) skips writing `derived` and syncing the geometry; every mutable access to a record that is not a turn (`get_mut`, `for_each_mut`, `set_settings`) clears the stamp, so a record that
anything touched between its turn and the build is handled the old way. Tests compare a stamped cache with a fresh rigid body bit for bit at every build, and the old loops of the sweep run on a copy of the records and are compared with
the new one. The build makes each bibite's box together with its body (`PhysicsScratch::boxes_for_next_step` / `boxes_ready`; the promotion pass for resting pellets and the step reuse them); the step does not write the bibites'
final poses back (`skip_write_back_of`; the sweep after the step reads `PhysicsScratch::solved`); the sweep lists the bibites that have mouth targets for the next body tick (`mouth_candidates`, a superset by construction: only that
sweep gives a bibite targets); and the broadphase tests the candidates of a body's x range on four columns of the boxes, eight at a time. The brain steps from a compiled plan (`Brain::plan_groups`, `plan_edges`: kind, bias and
weight copied in CSR order; the editor's setters and the mutations mark it, `compile_plan` rebuilds it). The probes and measurements, what did not help (boxing the records, reading ahead) and what is left are in
[BENCHMARKS.md](BENCHMARKS.md#100000-bibites-toward-12x-2026-11-05-late-night).

**The record's run, the summaries and the prefetch hints (2026-11-05, night).** `BibiteRecord` is `repr(C, align(64))` and its fields are placed for the lines the passes touch: lines 0-1 (pose, forces, ids, contact counters, the sense throttle) and 2-3 (colliders) are what the physics bridge needs; then, from a line boundary, one run of 640 bytes holds every organ the turn of a bibite touches each tick, the ones written every tick first (see `docs/BENCHMARKS.md`, "second round"); the lists of contacts and targets, the brain, the genome and the metadata follow. The scans that only ask a bibite's species, rad flag, liveness, readiness or pheromone sensing read `BibiteStore`'s `SlotSummary` array (eight bytes a slot, refreshed lazily for the slots `get_mut` / `insert_record` / `remove` listed, all of them after a restore or `for_each_mut`; a test build checks every summary against a fresh one), and `dense_pos` gives the position of a slot in the dense list in one step (a removal, the rank of the body tick). The dense-order turns, which run on one thread each starting on a cold record, hint the caches for the records three and six turns ahead through the `bibites-prefetch` crate (a single `_mm_prefetch`; the first and only `unsafe` the simulation's crates depend on, behind the default `prefetch` feature of `bibites-core`, off with `--no-default-features` without any change of result). The same crate holds the broadphase sweep's eight-lane box test in SSE2 (the compiler vectorized the equivalent loop in some builds and not in others, a 0.8 ms swing). The physics bridge keeps the bibites' `PhysicsBody`s from one parallel step to the next (`BibiteScratch::kept_bibite_bodies`): `PhysicsBody` is `repr(C, align(64))` with the pose, velocities, mass and damping in its first line, and a body is updated in place while it carries the bibite's tag and the generation of the record's colliders (`BibiteRecord::colliders_gen`, renewed by `refresh_colliders`), so the 108 bytes of colliders are copied only when a bibite grows; the same build pass lists the bibites that hold something. The resting-pellet layer starts only when the pellets are at least a quarter of the bibites. The sensing jobs keep their result buffers from tick to tick and `BibiteStore::apply_senses_chunks` applies them where they are.

**The viewer's simulation thread (2026-11-05; `--sim-thread` or `RUSTIBITES_SIM_THREAD=1`, off by default).** In the window's loop the thread that draws also ticks the world, and the HUD, the picking and the display list read
the world in every frame, so a frame cannot be shorter than a tick (12 ms at 20,000 bibites). `sim_thread.rs` lets the window not need the world while the player only watches. After `LIGHT_FRAMES_BEFORE_AWAY` (20) frames in which
nothing but the camera and the toggles of what is drawn was used (`App::light_input`: no click, no key that acts on the world; `App::watching_only`: no panel, selection, tool, placer, pause, challenge or toolbar panel open) the
world moves to a worker thread (`SimThread`) and `App.world` becomes a husk, an empty world with the same settings. The worker ticks the world at the speed the window sends (`App::engine_speed`: the slider's target as the
minimum-FPS governor leaves it), in bursts of owed ticks that end when the window asks for the world back, and publishes `Snapshot`s: the display list built for a camera 1.25 times as wide as the window's (so the window can pan and
zoom between two snapshots), the HUD statistics (every 200 ms), the simulated time and the tick, and the answer to the one question the window asks of the world in every frame, what lies under the pointer for the pellet
tooltip (`SceneRequest.hover` / `Snapshot.hover`). A snapshot costs the worker tick time, so it is made only when something changed (a tick ran, or the window asked for something else) and it is paid for: with the time the worker has slept (at most one every 16 ms), or, for a world
that leaves it no time to sleep, once six times what the last took has passed (a seventh of its time at the most), so a world running flat out keeps its speed and the window gets fewer snapshots. The
window draws the newest snapshot with its own camera at its own frame rate (`App::drawn_scene`, `App::advance_light`). A frame that needs the world takes it back (`App::ensure_world_home`, at most one tick of waiting): a click, a key
that acts on the world, a panel or selection or the pause, an autosave that fell due, a resize with the view fitted to the bibites, something that opens during the frame (`App::settle_world`). The ticks the worker ran are counted for
the time panel's ticks per second and the autosave's progress, and the auto-panner (which moves with every tick of the world) takes its steps for them when the world is back. The simulation is the same code: the worker calls
`World::tick`, and a test compares a world that went away and came back with one run headless, fingerprint for fingerprint; `every_input_is_either_a_camera_input_or_needs_the_world` stops compiling when `FrameInput` gets a
field nobody has classified, so a new key cannot act on the husk, and `the_runtime_ui_of_an_app_that_only_watches_does_not_read_the_world` builds the default UI frame with the real world and with the husk and compares them,
so a part of the default UI that starts reading the world's content (the pellet tooltip did) is found. The thread only starts when the window draws the game's runtime UI; the curated fallback HUD reads the world every frame.

**Dense-cluster regime (task-82 / task-131, 2026-10-30).** Spread worlds scale
linearly, but a dense pile is the physics worst case. The final clean-HEAD
whole-engine cluster capture (`bench` on the deterministic clustered save world,
10 ticks, best-of-2) measures **38.46 ms/tick (26 ticks/s)** for 2,000 bibites in
a 100×100 box and **20.00 ms/tick (50 ticks/s)** for a 200×200 box versus
**1.38 ms/tick (726 ticks/s)** spread — 2.4× faster whole-engine than the earlier
task-92 capture (90.91 ms). The physics-only core probe measures
**46.6 ms** for cluster2000 (single release test run, starter genome,
`step_physics` only) versus **1.24 ms** spread2000 and **123.1 ms** for
cluster4000; the earlier task-82 probe was 41.4–43.5 ms (three-run clean window),
so the probe is within the shared-machine noise band and still clears the
**<50 ms** task-82 target. The improvement comes from collider pose/AABB caches,
closed-form capsule/box narrowphase, dense-only parallel narrowphase with
deterministic pair-order merge, narrow output/merge preallocation, and an
AABB-coverage grid fast path. For the synthetic dense profile (>512 bodies in the
grid path) a **deterministic per-body candidate cap of 32** applies (31,747 pairs /
71,712 contacts in the final probe); normal/spread populations are uncapped and
bit-identical to the uncapped result. This is a documented benchmark
approximation, not a general physical claim. See
[`BENCHMARKS.md` §Dense-cluster](BENCHMARKS.md#dense-cluster-benchmark).

**Checkpoint rotation representation (task-117).** `BibiteSave`/`EggSave`
serialize the game-facing rotation in transform degrees, and
radian→degree→radian is not bit-exact (e.g. `1.927486062` → `1.927486181`,
1 ULP at f32). `BibiteCheckpointRecord`/`EggCheckpointRecord` therefore also
carry raw `rot_radians`, restored after `spawn_loaded`; the game save format
keeps degrees. This removes the synthetic split-resume tick-1 rotation/velocity
drift.

**100k-population stress (2026-10-30, task-48).** `prepare_synthetic` scales
the world area as `simulation_size = 2000·sqrt(N/2000)` (20k→6325, 50k→10000,
100k→14142) so density matches the 2,000 reference and zone/pellet density stays
constant. Commands:

```powershell
cargo run --release -- bench --bibites 100000 --ticks 30 --threads 1
# peak working set (CLI reports peak_bytes=null):
$exe = (Resolve-Path 'target\release\rustibites.exe').Path
$p = Start-Process $exe -ArgumentList 'bench','--bibites','100000','--ticks','30','--threads','1','--repeat','1' -PassThru -NoNewWindow
while (-not $p.HasExited) { $p.Refresh(); $p.PeakWorkingSet64 / 1MB; Start-Sleep -Milliseconds 100 }
```

| Population | World half-size | Spawn+init | Peak WS | Serial | 8/12 threads | Bibite-updates/s |
|---|---|---|---|---|---|---|
| 20,000 | 6,325 | ≈0.3 s | 102–107 MB | 35–44 tps (1.2–1.4× RT) | 38 / 35–44 tps | 0.69–0.88M |
| 50,000 | 10,000 | ≈0.5 s | 235–254 MB | 14–18 tps (0.5–0.6× RT) | 14 / 13–18 tps | 0.67–0.90M |
| 100,000 | 14,142 | ≈0.5–1.0 s | 462–480 MB | 6–9 tps (0.2–0.3× RT) | 6–7 / 6–9 tps | 0.60–0.85M |

(Ranges are across repeated runs on the shared reference machine; ±10–20%.)
100k is **runnable with stable memory** (peak ≈480 MB after 30 ticks,
≈4.6–5.1 KB/bibite; no growth beyond population/egg churn) but is **not
≥20× real-time**: physics ≈68–73 ms/tick (50–57%), body ≈46–56 ms (39%), sensing
≈2–13 ms (2–9%), deaths/hash-sync ≈2.5 ms. The 5× population from 20k→100k
costs 5.0–5.4× tick time at fixed density, i.e. **linear, no hidden O(n²)**;
per-bibite throughput is nearly flat (0.69M → 0.64M updates/s, ~1.1× decline,
mostly cache/population churn). Memory sizes from `perf_scale_sizes`:
`BibiteRecord` ≈1,592–1,704 B across builds, `Brain` 440 B + ~2.6 KB of
per-brain `Vec` arrays (18 separate allocations), `SenseOutputs` 96 B, hash
`Entry` 32 B. The main
remaining memory/locality opportunity is compacting those 18 per-brain Vecs
(body/brain-owned). Remaining limit: serial physics/body floor — the 20k world
is only 1.2–1.4× real-time, so 100k at ≥20× needs parallel physics/body plus a
several-× per-bibite reduction, not just more threads. Threads add ~0–10%
because physics/body are serial and sensing is 2–9% of the tick.

**Final 100k one-tick sanity (task-131, clean binary `3CCB231E…`, 2026-10-30,
synthetic density-scaled bench world).**
`rustibites bench --bibites 100000 --ticks 1 --threads 1/4/8` completed at
**494.6–512.7 MB peak working set** (≈4.95–5.13 KB/bibite; +7–11% vs the task-92
463.7–463.8 MB capture with the larger body/checkpoint/FOV state), 1.06–1.09 s
process wall (spawn + one tick), 0.140–0.141 s engine time for the single tick,
0.71–0.72M bibite-updates/s, no timeouts/OOM; thread count had no measurable
effect in one tick. This is a memory/runnability check, not a throughput claim —
use the task-48 30-tick table above for historical steady-state 100k throughput.
Artifacts: `tools/bench/results/final-100k/`.

**Follow-up (body+brain, not yet done):** compact the per-brain allocation set
(18 separate `Vec`s / ≈2.6 KB per starter brain, plus scattered
`BibiteRecord`-adjacent allocations) into a shared/CSR-friendly layout; this is
the remaining 100k memory-locality and spawn-time win and needs body-dev +
brain-dev coordination. The world-owned persistent pool is exposed to body work
via `World::executor()` -> `sim::ParallelExecutor` (`parallel_chunks` /
`parallel_chunks_with` / `parallel_map`, owned payloads, chunk-order results,
serial fallback; ≈31 µs per 8-chunk activation), so body pass-B can be
parallelised without per-tick `std::thread::scope` spawns.

### Evolution throughput (task-47, 2026-10-30)

The user-facing question is "how much faster can I evolve than the game?", not
just ticks/s. The benchmark is a seeded, deterministic, release-mode example:

```powershell
cargo run --release -p bibites-core --example evolution_bench
# or custom population:ticks pairs
cargo run --release -p bibites-core --example evolution_bench -- 1000:20000 1000:50000
```

World: synthetic default ecology with mutation + speciation on, tuned for a
founder population (`pelletGrowth` at max, 50 zones, 2× zone biomass/fertility,
pellet energy 2000; founders spawned **inside** zones so they start near food).
Deterministic for a fixed seed; per-run `state_fingerprint` is printed. Test-build
harness (`cargo test --release -p bibites-core evolution_regression_fingerprint
-- --ignored -- --nocapture`) pins the 12k-tick/64-founder fingerprint
(`0xaaf56a43094b6fde`) so evolution-path changes are caught.

Metrics: `births/s = births / wall_s`; `gen/sim-h` and `gen/wall-h` use the
**max generation reached** divided by simulated/wall hours; "species" is
`active/records`; mutation columns are gene/brain mutation events and structural
synapse/node adds/removes/toggles (instrumented counters, relaxed atomics on the
birth path only).

| start pop | ticks | wall | ticks/s (RT×) | births | deaths | final pop (min) | gen max/mean | species | gene/brain mutations | gen/sim-h | gen/wall-h |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1,000 | 20,000 | 22.9 s | 872 (29.1×) | 52 | 1,037 | 15 (15) | 1 / 0.27 | 1/1 | 197 / 123 | 5.4 | 157.0 |
| 2,000 | 20,000 | 45.4 s | 441 (14.7×) | 130 | 2,097 | 33 (33) | 1 / 0.30 | 1/1 | 525 / 360 | 5.4 | 79.3 |
| 5,000 | 20,000 | 207.0 s | 97 (3.2×) | 188 | 5,104 | 84 (84) | 1 / 0.31 | 1/1 | 691 / 468 | 5.4 | 17.4 |
| 1,000 | 50,000 | 36.1 s | 1,385 (46.2×) | 57 | 1,056 | 1 (1) | 1 / 1.00 | 2/2 | 228 / 168 | 2.2 | 99.7 |

Interpretation (honest status): evolution **does run** — births occur after the
~11k-tick egg cycle, children have mutated genomes (e.g. 3–25 changed of 15–84
survivors, distinct genome hashes), mutation counters are non-zero, species get
assigned and speciation was observed in the 50k run (2 records) and transiently
in real-save runs. But dense synthetic founder populations **collapse toward a
few survivors** after the first generations: the current feeding/energy balance
does not sustain 1k–5k founders (likely body feeding/energy calibration, not a
benchmark bug), and the synthetic runs only reach generation 1, so
generations/hour is an extrapolation rather than a long steady-state rate.
Real-save check (corrected after the generation fix, task-72; `rustibites run
testdata/real/saves/lol.zip --ticks 50000 --seed 12345 --threads 8 --capture
testdata/evolve/lol_50k`): 35→24 bibites, 17.3 s wall (2,887 ticks/s, 192× RT),
**592 births / 590 deaths**, `gen_max` **630 → 637 monotonic** (no drops;
`gen_mean` 627.4 → 636.8), species 1. The earlier 630→1 reading was a real bug:
body `lay_eggs` seeded child `GenomeMeta::default()` (generation/species/flags
zeroed) instead of the parent's meta, so every egg was generation 1; body-dev
fixed it (task-73) and generation now inherits/increments. Generation stats are
documented as aggregates over **living bibites** (eggs/corpses excluded, matching
the game's living-population tracker), so losing the oldest high-generation
individual can lower the max legitimately — but it no longer resets to 1.
Corrected rates for this 50k run (0.926 simulated hours): **7.56 descendant
generations/sim-h** from the loaded 630 baseline (4.84 if divided by the save's
absolute 131.5 h lineage age), and 639 births/sim-h. The synthetic founder runs above still need re-measurement with the fixed
inheritance. The long-run checkpointed runner exists (`tools/evolve/`) and
`docs/EVOLUTION_RESULTS.md` reports the first experiments; checkpoint/resume is
now bit-exact for 1k–10k, and a full evolution-validation pass is an active
development-wave item (canonical Default baseline plus historical `lol` scale).

Targets (milestone 1, after optimisation):

| Population | Target |
|---|---|
| 1,000 bibites | ≥ 200× real-time on 1 thread; ≥ 1,000× on 12 threads |
| 10,000 bibites | ≥ 60× real-time on 12 threads |
| 100,000 bibites | runnable, stable memory, ≥ 20× real-time |

Current gap (final matrix, clean-tree binary `3CCB231E…`, 2026-10-30): 1,000
bibites runs at ~49× real-time serial vs the 200× target (~4× away); 10,000 at
8 threads is 3.2× real-time vs the 60×/12-thread target (~19× away; 12 threads
were not re-measured in the final capture); 100,000 steady-state is 0.2–0.3×
real-time vs the ≥20× target (~70–100× away) while already needing
≈495–513 MB for the one-tick sanity. The next wins are parallel physics/body,
further dense-cluster physics work (cluster2000 probe is 46.6 ms, under the
<50 ms task-82 target but still ~38× the spread cost), and sensing/brain
parallel efficiency.

Optimisation order (measure before each step): data layout → spatial hash →
brain CSR → parallel sensing/brains → SIMD-friendly math → avoidance of
allocations and virtual calls. Approximations (e.g. LUT activations) are
off by default and only added behind a `--fast-math` flag with documented error.
Current bottlenecks and the diagnosis history live in
[`PERF_DIAGNOSIS.md`](PERF_DIAGNOSIS.md).

---

## 11. CLI surface

```
rustibites run   <world.zip | scenario.zip | genome.bb8> [--ticks N] [--seed N]
                 [--threads N] [--settings PATH] [--save out.zip]
                 [--capture DIR] [--stats json|csv|human] [--quiet]
rustibites bench [--scenario PATH] [--bibites N] [--ticks N] [--threads N]
                 [--repeat N] [--json]
rustibites info  <file>        # inspect a genome/world/scenario/settings file
rustibites check <file>        # validate + round-trip a genome/settings/bundle
rustibites braincheck <json>   # emit Rust brain outputs for tools/braincheck
```

Statistics for milestone 1: time, tick, population, births, deaths, species
count, mean energy/health/age, biomasses, behaviour curves, brain sizes,
gene means/std, and throughput; `--stats json` includes a deterministic
`state_fingerprint` for same-seed checks.

### Current CLI status (2026-10-30)

Working end-to-end: `run` on real saves and scenarios (via io `WorldContents`),
`--settings` independence override, `--save` (game-loadable), `--capture`
(game-vs-Rust parity JSONL), `bench` with per-phase timings and thread scaling,
`info`/`check` for genomes/settings/scenarios/saves, `braincheck`.
Remaining CLI work: richer `info` summaries, scenario-template folder
resolution (`--templates`), and the final benchmark suite for docs/CI.

---

## 12. Development workflow

1. **Spec first.** Each subsystem has a written spec in `docs/spec/` extracted
   from `Here/`, with source references. Implementation follows the spec.
2. **Task board.** Work is claimed from the shared task board with explicit
   write scopes. Do not edit another owner's files.
3. **Small, reviewable increments.** Each system lands with unit tests and a
   short parity/behaviour note.
4. **Integration gate (Lead):** `cargo test --workspace`, `cargo clippy
   --workspace --all-targets -- -D warnings`, `cargo fmt --check`, plus the
   golden/parity checks, before a milestone is called done.
5. **Docs stay true.** If the implementation diverges from the spec, update the
   spec in the same change and record the reason.

---

## 13. Roadmap

| Phase | Deliverable |
|---|---|
| 0 | Spec extraction for brain/genetics, body/organs/physics, world/ecology, formats/settings |
| 1 | Core foundation + genome/brain with unit tests; `.bb8` genome import |
| 2 | Body, organs, energy accounting, custom physics |
| 3 | World/ecology, sensing, scenario loading, headless run + stats |
| 4 | Full save/scenario round-trip; parity harness vs installed game |
| 5 | Optimisation to throughput targets; benchmark suite |
| 6 | Optional viewer (`bibites-view`) and quality-of-life CLI |

---

## 14. Fidelity deviations (running list)

The authoritative, detailed list lives in [`DEVIATIONS.md`](DEVIATIONS.md).
Summary of the permanent decisions:

| Area | Deviation | Rationale / status |
|---|---|---|
| Physics | Custom solver instead of Unity Box2D | speed and portability; behavioural parity targeted |
| Rendering | No rendering in the headless engine; the optional viewer is a thin client | headless-first design |
| Viewer rendering | Body-sprite **tail bend** not visible yet (game is a shader deformation, no Tail mesh; angles simulated/tested, warp pending); text is plain TTF (no TMP SDF outline); exact runtime HUD layout not final | documented viewer-alpha limitations; no substitute art/stretch |
| Frame-rate coupling | Reproduce configured-TPS-equivalent behaviour | deterministic headless runs |
| Randomness | Deterministic `Rng` replaces unseeded game RNGs | reproducibility; mutation parity statistical |
| `data.bin` | Opaque pass-through in M1 | round-trip safety |
| 0.6.4a1 folder saves | Unsupported | not in the decompiled build |

Add entries here and in `DEVIATIONS.md` whenever an intentional divergence is
introduced.

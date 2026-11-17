# SoA `BibiteStore` migration — scoping and go/no-go

Status: **decision-ready scoping document, no implementation**. Owner: body-dev.
Date: 2026-10-30. Branch state: post PERF round 3/4 (serial micro-optimisations
landed; parallel pass-B rejected as not profitable in the current AoS layout).

Read with [`../ARCHITECTURE.md`](../ARCHITECTURE.md) §10 and
[`../PERF_DIAGNOSIS.md`](../PERF_DIAGNOSIS.md).

---

## 0. Executive summary

**Headline recommendation: NO-GO for a full `BibiteStore` SoA rewrite for the
current milestone. GO for a bounded "SoA-lite" phase (physics mirror + body
pass-B hot arrays + brain `Vec` consolidation) only if a 1.5–2× serial/10k win
is a hard requirement.**

- The 2,000-bibite serial tick is now ~1.05–1.09 ms (≈950 ticks/s, ≈32×
  real-time): physics ≈0.56 ms (53%), body ≈0.47 ms (45%), sensing ≈0.03 ms.
  The engine scales linearly to 100k (≈117 ms/tick, 6–9 ticks/s, 0.2–0.3×
  real-time), so there is **no complexity bug left to fix** — only per-bibite
  constant factors and serial phases.
- The milestone targets are far beyond the current constant factors:
  * 1,000 @ **200×** real-time single-thread needs **3.8×** (6000 ticks/s =
    0.167 ms/tick vs current 0.627 ms).
  * 10,000 @ **60×** on 12 threads needs **16.5×** (1800 ticks/s = 0.56 ms/tick
    vs 9.2 ms serial, 8.7 ms @8 threads).
  * 100,000 @ **20×** needs **~70×** (600 ticks/s = 1.67 ms/tick vs 117 ms).
- A full SoA rewrite is a **30–50 agent-day** data-layout migration touching
  `body.rs`, `organs.rs`, `brain.rs`, `world.rs`, `physics.rs` and the io save
  paths. Realistic single-thread gain from locality alone is **1.3–2×**, not
  3.8–16×.
- Therefore: full SoA is **not sufficient for 100k @20×** and is **high risk for
  the 1k/10k targets**; a bounded SoA-lite phase (**10–15 agent-days**, expected
  1.3–1.6× at 2k/10k) is the only piece with a defensible cost/benefit ratio.
  The 100k @20× target should be re-scoped (or explicitly accepted as
  unreachable in this architecture) before committing engineering time.

---

## 1. Current layout and measured cost model

### 1.1 Storage today (AoS)

`BibiteStore` is a generational slot vector of whole records plus a dense list:

```text
records: Vec<Option<BibiteRecord>>   // 1,592 B per live record
generations: Vec<u32>, free: Vec<u32>, dense: Vec<u32>
```

A `BibiteRecord` contains, inline: `id`, pose (`pos/vel/rot/ang_vel`), `genome`
(35 f32 genes + metadata), `brain` (440 B inline + **18 separate `Vec`s ≈2.6 KB
heap** for topology/state), `BodyState` (energy/health pools + `BodyGeometry` +
`BodyParams`), `Stomach`, `FatOrgan`, `ArmorOrgan`, `EggLayer`, `MouthState`,
`InternalClock`, `PheromoneOrgan`, `PropulsionState`, derived caches, senses,
throttle, mutation factor, steering, pending force/torque, cached colliders.

Measured per-bibite working set at 100k: **≈4.6–5.1 KB/bibite**
(`BibiteRecord` 1,592 B, `Brain` 440 B + ~2.6 KB heap, `SenseOutputs` 96 B,
hash entry 32 B, plus churn).

### 1.2 Hot-loop access pattern

Per tick, three hot passes sweep every live record, each by a different
discipline:

| Phase | What it touches | Current cost @2k | @100k (task-48) |
|---|---|---:|---:|
| Pass A (energy/cadence) | `body.energy`, `metabolism`, `derived.brain_upkeep`, `throttle`, `senses`, maybe `events/deaths` | ~0.02 ms | included in body |
| Brain batch | `brain.*` (18 Vecs), 15 output node values; `BrainInputs` assembly from `body/clock/stomach/senses` | ~0.25 ms avg (every other tick) | included in body |
| Pass B (organs/movement/mouth/growth/clock/kinematics) | `stomach`, `fat`, `egg_layer`, `mouth`, `growth`, `clock`, `propulsion`, `body` energy/health, `brain` output values, `steering`, `genome` (herd gene, rate), cached colliders on growth | ~0.33 ms | included in body |
| Physics mirror build | pose + mass/inertia/damping + cached colliders → `Vec<PhysicsBody>`; then AABB/broadphase/narrowphase/solver/write-back | ~0.56 ms total (build ~0.12, entry/AABB+broadphase ~0.20, narrowphase ~0.18, solver/writeback ~0.06) | physics ≈68–73 ms (50–57%) |

The dominant cost is **pointer chasing and cache misses**, not arithmetic:

- Every organ update loads a different 64–128 B region of a 1.6 KB record;
  `brain` arrays live in 18 separate heap allocations reached through the
  record, so brain stepping and input assembly are effectively random-access.
- The physics mirror reconstructs per-body data (mass/damping/colliders) from
  these scattered records every tick, then stores it in a separate contiguous
  `Vec<PhysicsBody>` — a full copy of the hot physics state per tick.
- Parallelism is blocked because a worker would need exclusive `&mut` to whole
  1.6 KB records (scoped threads: measured no gain at 2k; persistent pool needs
  owned `'static` payloads → 1.4–2 MB/tick of copy-in/apply-back).
- Scaling 20k→100k is 5× population for 5.0–5.4× tick time (linear), with
  per-bibite throughput declining only ~1.1× (0.69M→0.64M updates/s) — i.e. the
  AoS layout is directionally fine, just **~4–70× too slow per bibite** for the
  milestone targets.

### 1.3 Where the time goes (2,000, serial, quiet machine)

| Component | ms/tick | Share | Notes |
|---|---:|---:|---|
| physics | 0.56 | 53% | build 0.12, broadphase 0.20, narrow 0.18, solver/writeback 0.06 |
| body | 0.47 | 45% | pass B 0.33, brain batch ~0.125 avg, assembly ~0.05, pass A ~0.015 |
| sensing | 0.03 | 3% | already optimised (task-41) |
| total | 1.05 | 100% | ≈950 ticks/s |

At 100k the split is physics 50–57%, body 39%, sensing 2–9%.

---

## 2. Target metric math (what the numbers actually require)

Reference: 30 TPS, so `ticks/s = 30 × real-time factor`.

| Target | ticks/s needed | ms/tick budget | Current | Speed-up needed |
|---|---:|---:|---:|---:|
| 1,000 @ 200× single thread | 6,000 | 0.167 | 0.627 ms | **3.8×** |
| 10,000 @ 60× on 12 threads | 1,800 | 0.556 | 9.17 ms serial / 8.7 @8t | **16.5×** |
| 100,000 @ 20× (any threads) | 600 | 1.667 | 117 ms | **70×** |

Amdahl ceiling if 90% of the tick were perfectly parallel on 12 threads:
`1 / (0.1 + 0.9/12) ≈ 5.7×`. Even with a **12× serial SoA speed-up** and
perfect parallel scaling of physics/body, 100k @20× is not reachable without
algorithmic changes (LOD/sleeping/approximation) or many more cores.

Conclusion: **SoA alone cannot meet the published 100k target**; it is at best
one factor in the 1k/10k goals.

---

## 3. Proposed target layout (SoA hot / AoS cold)

Principle: split `BibiteRecord` into **hot arrays accessed by every pass** and
**cold AoS blobs accessed per birth/death/save/brain-mutation**.

### 3.1 Hot arrays (store-owned, dense slot indexing)

All arrays are `Vec<T>` indexed by **slot** (`u32`), parallel to the existing
generations/free/dense lists (or by dense index with a slot↔dense map; slot
indexing keeps `BibiteId` stable and avoids remapping on death).

| Array group | Fields | Type/size | Used by |
|---|---|---|---|
| Pose | `pos`, `vel`, `rot`, `ang_vel` | 4×`f32` = 16 B | body, physics, senses |
| Vitals | `energy`, `health`, `max_energy`, `max_health` | 4×f32 = 16 B | pass A, pass B, physics mass |
| Size/body | `d1`, `d2`, `base_area`, `real_area`, `body_length`, `inflate` | 6×f32 = 24 B | pass B, physics, geometry |
| Organs scalar | `fat_amount`, `fat_inflate`, `egg_progress`, `egg_energy`, `stomach_total`, `armor_thickness/strength` | ~8×f32 = 32 B | pass B, physics mass |
| Mouth | `bite_progress`, `desire_*`, `can_swallow`, `opening`, `n_held`, held tags `[u64;4]` | ~28 B | pass B, physics sensors |
| Clock/cadence | `time_alive`, `chrono`, `tic`, `tic_progress`, brain/lookup/sense counters | ~32 B | pass A/B |
| Brain outputs | `[f32; 15]` last node values | 60 B | pass B, parity digests |
| Steering | `has_herd`, `sum_dir`, `sep_proj` (+ reserved vectors) | 24 B | pass B movement |
| Physics cache | `mass`, `inertia`, `inv_mass`, `inv_inertia`, `lin_damp`, `ang_damp`, `aabb` | ~40 B | physics mirror |
| Colliders | 3× (shape kind/size/offset/friction/bounce) = ~120 B | physics |
| Effects ring | per-record pending bites/swallows/egg-lay/death flags | 8–16 B | pass B/merge |

Hot payload ≈ **400–500 B/record** (vs 1,592 B inline + 3 KB heap). It is
contiguous, array-per-field, and trivially chunkable for the pool.

### 3.2 Cold AoS (kept as records, accessed rarely)

- `Genome` (35 f32 + meta) — read for movement herd gene/rate, mutation, io.
  Keep per-slot `Vec<Box<Genome>>`/`Option<Genome>`; or a separate dense genome
  arena referenced by index. Access frequency is ~1–2 fields/tick; consider a
  tiny hot mirror (`metabolic_rate`, `herd_velocity_weight`, size gene) to avoid
  touching the cold blob in the hot loop.
- `Brain` topology (18 Vecs / 2.6 KB) — **must** be consolidated separately
  into CSR shared arrays (see §3.4); do not leave it in the cold blob.
- `Senses` (96 B) — written by world sensing once per due brain tick; hot but
  only on due ticks; keep in a compact array.
- ids/genome meta/species/lineage/children, save-only fields — cold AoS.

### 3.3 Handle/ID mapping

Keep the existing contract exactly:

- `Handle { slot: u32, generation: u32 }`; `BibiteId` unchanged; dense list and
  free list unchanged.
- SoA arrays are indexed by **slot**; `records[slot]` remains the cold blob.
- Removing a bibite clears hot arrays at the slot (or marks a `live: BitVec`
  generation owner) and bumps the generation. Dense iteration order must stay
  identical to today (dense slot list), so all system order and parity are
  preserved.
- Serialization (`BibiteSave`) is assembled from hot arrays + cold blob in the
  same field order; no save format change.

### 3.4 Brain storage (separate sub-project)

Consolidate the 18 per-brain Vec allocations into:

- one shared `Vec<Node>`-like arena per brain or a global CSR: `node_kind`,
  `node_bias`, `node_value`, `node_last_input/last_output`, `in_degree`,
  `out_degree`;
- `syn_in`, `syn_out`, `syn_weight`, `syn_enabled`, `syn_innovation` in a
  single CSR block per brain with a documented ownership/allocation policy
  (e.g. `brain_arena: Vec<f32>` + per-brain `{offset,len}` views, or one
  compact `Box<[T]>` per brain).

This is a brain-dev-owned refactor with its own determinism fixtures
(`braincheck`, mutation bit-identity, parity digests). It gives the largest
single memory win (≈2.6 KB → ≤0.8 KB per brain), better spawn time, and makes
brain-batch/SoA access cache-friendly. It can land **independently** of the
store layout.

### 3.5 Physics mirror

With SoA hot arrays, build a **direct** physics mirror into reused arrays
(pose/mass/inertia/damping/collider shape+offset+aabb) rather than a
`Vec<PhysicsBody>` of copied structs; the solver operates on these arrays. This
removes the per-tick 0.12 ms mirror build copy and makes narrowphase
parallelisable per body range (fixed candidate order + per-chunk contact
buffers + serial merge). Expected physics gain 1.3–1.6× serial; more with
parallel narrowphase if it beats activation overhead.

---

## 4. Migration phases, gates, effort

Each phase is independently shippable behind a feature flag with a
**bit-identical fingerprint/capture gate**. Suggested flag:
`cfg(feature = "soa_store")` or a runtime `StoreLayout::{Aos,Soa}` for
side-by-side oracle runs; parity harness compares per-tick state fingerprints
and the Ring capture byte-for-byte.

| Phase | Scope | Gate | Effort (agent-days) | Gain (est.) | Risk |
|---|---|---|---:|---|---|
| P0 | Baseline: add hot-array stats + per-phase timers + fingerprint oracle harness; record 2k/10k/100k baselines | same-seed fingerprints stable | 2–3 | 0 | low |
| P1 | Pose/vitals/size/clock/cadence + physics cached mass/AABB arrays; physics mirror reads them. Keep organs/genome/brain AoS | fingerprint equality; Ring default exit 0 | 8–12 | physics 1.2–1.4×, body ~1.1× (better locality for pass A/physics) | medium |
| P2 | Organ hot arrays (stomach/fat/egg/mouth/growth/clock/pending force) + pass-B loops over arrays; effects ring; serial first | fingerprint + capture equality; serial body ≤0.35 ms @2k | 8–12 | body 1.3–1.6× serial | medium-high (parity drift) |
| P3 | Brain consolidation (CSR/arena) + brain batch on compact arrays | braincheck/mutation/parity digests exact | 10–15 | brain step/assembly 1.1–1.4×; memory −60% per brain; spawn faster | high (brain-dev) |
| P4 | Parallel pass-B over hot arrays via `ParallelExecutor::parallel_chunks_with` (owned compact payload, apply-back in dense order); parallel narrowphase if measurable | serial-vs-parallel fingerprint identical; 8t ≥1.15× at 2k | 8–12 | body 2–4× @8t on pass-B, physics 1.2–1.5× with parallel narrowphase | medium-high |
| P5 | Cleanup: remove feature flag, delete AoS hot fields, io/`BibiteSave` adapters, docs | full gates + parity + 100k stress | 5–8 | — | medium |
| **Total** | | | **41–62 agent-days** (≈8–12 weeks single dev; 4–6 weeks for two devs with P1/P2 split) | |

### 4.1 Bounded alternative (recommended if any SoA work happens)

- **SoA-lite**: P0 + P1 + brain consolidation (P3) + `PhysicsBody` direct arrays,
  **no full organ/store rewrite, no parallel pass-B**. Effort **12–18
  agent-days**. Expected **1.2–1.5×** at 2k/10k serial, **~1.1–1.2×** at 100k,
  memory −20–30%. Low/medium risk; preserves all current gates.
- This is the only slice with a defensible cost/benefit ratio for the current
  milestone.

### 4.2 Acceptance gates for every phase

1. `cargo test --workspace`, core clippy `-D warnings`, fmt clean.
2. Same-seed state fingerprint identical to the AoS baseline (1000/2000 bibites,
   100 ticks).
3. Default capture (`testdata/real/Default.zip`, 600 ticks) default-profile compare exit 0
   and per-tick entity digests byte-identical; Ring capture (`ring_parity.zip`, 300 ticks)
   retained only as a historical-fixture regression (`docs/TESTING_POLICY.md`).
4. Brain fixtures: `braincheck` + mutation determinism unchanged (when brain
   storage changes).
5. No allocation growth in the steady tick (instrumented allocator count).
6. 100k stress: peak WS and tick time reported; no OOM.

---

## 5. Cost/benefit and 200× feasibility

| Scenario | Expected with full SoA | Expected with SoA-lite |
|---|---|---|
| 2,000 serial tick | 0.55–0.7 ms (1.5–1.9×) | 0.8–0.95 ms (1.1–1.3×) |
| 10,000 serial | 6–8 ms (1.3–1.6×) | 8–9 ms (1.1–1.2×) |
| 10,000 @12t | + parallel pass-B/physics maybe 2–4× on 80–90% of tick → 3–5× overall over serial | ~1.0–1.2× (threads still limited) |
| 100,000 | ~50–70 ms (1.7–2.3×), 0.3–0.6× RT | ~90–110 ms |
| 1,000 @200× single-thread (need 3.8×) | ~2× → still ~2× short | no |
| 10,000 @60×/12t (need 16.5×) | SoA + perfect parallel ~6–8× → still ~2× short | no |
| 100,000 @20× (need 70×) | no (needs algorithmic LOD/sleeping or ~100+ cores) | no |

**Go/no-go criteria:**

- **GO (full)** only if: (a) the 1k@200×/10k@60× targets are hard contractual
  requirements, (b) two engineers can spend 4–6 weeks, (c) parallel physics/body
  and a re-scoped 100k target are accepted, and (d) the team accepts ~30–50
  agent-days of layout churn across body/brain/physics/io.
- **GO (SoA-lite)** if a 1.3–1.5× serial/2k–10k improvement is required at low
  risk: 12–18 agent-days.
- **NO-GO (full)** for the current milestone if the 100k @20× target stays as
  written: SoA does not close a 70× gap, and the risk/effort is better spent on
  the remaining correctness/parity/io workstreams. Re-scope 100k target (e.g.
  "stable memory, ≥0.3× real-time, linear scaling") and revisit if the product
  requires it.

---

## 6. Smaller intermediate wins (do these separately, low risk)

1. **Brain `Vec` consolidation** (P3 alone): 2.6 KB → ≤0.8 KB/brain, fewer
  allocations, faster spawn/mutation; 10–15 agent-days, brain-dev owned, no
  store layout change. This is the highest-value independent item.
2. **Record shrink**: move save-only fields (children Vec, save caches) out of
   the hot record; target `BibiteRecord` ≤1.2 KB. 2–3 days; small locality win.
3. **Physics mirror direct arrays** without store SoA: keep `PhysicsBody` but
   build its arrays from compact per-record caches (already partially done via
   collider cache/AABB); 3–5 days; ~1.1–1.2× physics.
4. **Sensing/thread pool**: already near the practical floor (2–9% of tick);
   no SoA dependency.

---

## 7. Open questions for Lead

1. Is 100k @20× a hard requirement or an aspirational target? If hard, SoA is
   not the answer — we need an LOD/sleeping/approximation design and likely a
   target revision.
2. Which target matters for milestone 1: 1k @200× single-thread, 10k @60× on
   12 threads, or "best throughput at 2k" (~950 tps today)? The answer changes
   whether full SoA + parallel is worth 30–50 agent-days.
3. Do we accept a two-engine transition (AoS vs SoA behind a flag) during
   migration, with parity oracle runs in CI? This is required for safe phases
   P1–P2.
4. Can brain-dev own P3 (brain CSR) in parallel with P1/P2? It is on the
   critical path for memory and for the 10k target.
5. Determinism acceptance: fingerprints are `f32`-exact today; SoA changes
   summation/evaluation order? Arrays preserve per-record order, so `f32`
   results should be bit-identical if the same operations are applied in the
   same per-record order. Confirm that CI will gate on bit-identical
   fingerprints (not just MAE).

---

## 8. Recommendation (final)

**NO-GO for a full `BibiteStore` SoA migration now.** The 100k @20× target is
~70× away and not closable by locality; the 1k/10k targets need ~4–16×, while
full SoA realistically buys 1.5–2× for 30–50 agent-days of high-risk churn.

**If optimisation continues, do the bounded SoA-lite slice (12–18 agent-days:
physics direct arrays + brain `Vec` consolidation + record shrink).** It is
compatible with the current APIs/gates, delivers a defensible 1.2–1.5× at
2k/10k, and keeps the option open for a later full migration if targets demand
it.

Revisit this decision only after: (a) targets are re-scoped/confirmed, (b) the
remaining correctness/parity workstreams land, and (c) a prototype P1 shows a
measured ≥1.25× physics win on the reference machine.

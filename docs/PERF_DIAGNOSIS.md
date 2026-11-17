# RUSTibites performance diagnosis: superlinear `bench` scaling

Status: **root cause identified and measured**.
Snapshot: 2026-10-30. Measured sources: `physics.rs` last touched by commit
`6c9d5c1`, `rng.rs` unchanged, CLI `bench` path at `0bf0f13`. `world.rs` and
`body.rs` were being edited by other agents during the investigation; the
instrumented copy was taken at ~00:32 from the then-current working tree. This
diagnosis never modified `crates/` (the instrumented copy lived under
gitignored `testdata/perf/` and was deleted afterwards).

Machine/toolchain for the measurements: Windows, Ryzen 5 3600, 16 GiB RAM,
`rustc 1.99.1`, release build (the instrumented copy disabled fat LTO to keep
the edit/build loop short; the upstream `[profile.release]` uses
`lto = "fat"`).

---

## TL;DR

1. **The benchmark does not spawn bibites uniformly.** `spawn_one` calls
   `world.rng.fork(0xB1B1_7E50)` with the same constant stream tag on every
   spawn, and `Rng::fork` is position-independent. Every starter bibite
   therefore gets the *identical* position and rotation. Logged for every run:
   `pos = (754.823242, -590.421021)` for all N (1000/2000/3000/4000 checked).
2. With all colliders coincident, the sweep-and-prune's early `break`
   (`physics.rs:1322`) never fires, so candidate generation degenerates to
   all-pairs: **9·C(N,2)** candidate collider pairs, **4·C(N,2)** contacts and
   **2·C(N,2)** trigger events. Measured exactly at N = 500/1000/2000/3000/4000.
3. Time is quadratic in N with a constant per-pair cost:
   `T_tick ≈ 0.55 µs × 9·C(N,2) ≈ 2.5·10⁻⁶·N² s`. Measured N=3000:
   22.45 s (report says 23.5 s), N=4000: 38.6 s (report says 36.5 s).
4. The **20 GiB** failure is not a `usize`/`u32` wrap and not explicit capacity
   arithmetic. It is `Vec<ContactEvent>` doubling to capacity 2²⁸:
   `268,435,456 × size_of::<ContactEvent>() = 268,435,456 × 80 =
   21,474,836,480 B = 5·2³²`. Contacts = 4·C(N,2) crosses the previous
   capacity 2²⁷ at **N ≥ 8193**; at N=8000 the four scratch buffers already
   total **27.5 GiB** (contacts alone request 10 GiB), which OOMs a 32 GiB
   host.
5. **Exact fix location:** `crates/bibites-cli/src/engine.rs`, `spawn_one`
   (~line 333): derive a per-spawn RNG stream instead of re-forking the same
   tag. With an indexed stream, measured N=4000 drops from **39.0 s/tick to
   23 ms for 3 ticks** (≈1700×), and candidate pairs drop from 71,982,000 to
   2,060. Physics hardening (grid broadphase + buffer discipline) is still
   recommended for robustness, but it is not what makes this benchmark
   quadratic.

---

## 1. Reproduction and reported numbers

Upstream repro (no edits):

```powershell
cargo build --release -p bibites-cli
target\release\rustibites.exe bench --bibites 2000 --ticks 300 --json
target\release\rustibites.exe bench --bibites 3000 --ticks 1   --json
target\release\rustibites.exe bench --bibites 4000 --ticks 1   --json
# ...5000, 6000, 8000, 10000; watch for "memory allocation of ... failed"
```

Reported (from the task) and measured with instrumentation:

| N | reported | measured tick 1 | measured candidates | measured contacts | measured triggers |
|---:|---:|---:|---:|---:|---:|
| 500 | 2.1 s / 600 ticks | 0.542 s (tick 1) | 1,122,750 | 499,000 | 249,500 |
| 1000 | — | 2.401 s | 4,495,500 | 1,998,000 | 999,000 |
| 2000 | 16.1 s / 300 ticks | 8.871 s | 17,991,000 | 7,996,000 | 3,998,000 |
| 3000 | 23.5 s / tick | 22.451 s | 40,486,500 | 17,994,000 | 8,997,000 |
| 4000 | 36.5 s / tick | 38.627 s | 71,982,000 | 31,992,000 | 15,996,000 |
| 5000 | 127 s / tick | not run (16 GiB host, ~12.3 GiB scratch) | 112,477,500 (model) | 49,990,000 | 24,995,000 |
| 6000 | 157 s / tick | not run (~24.5 GiB scratch) | 161,973,000 (model) | 71,988,000 | 35,994,000 |
| 8000 | OOM / stall | not run | 287,964,000 (model) | 127,984,000 | 63,992,000 |
| 10000 | 20 GiB alloc failure | not run | 449,955,000 (model) | 199,980,000 | 99,990,000 |

"Measured tick 1" is the instrumented release binary's in-`step` total
(`total=` field) plus the surrounding tick; it agrees with the reported
numbers within normal LTO/clock variance. The 500/2000 rows are per-run wall
times for the requested multi-tick runs.

## 2. How this was measured

To keep `crates/` untouched, the workspace sources (`Cargo.toml`,
`Cargo.lock`, `crates/`) were copied to `testdata/perf/ws/` (gitignored),
instrumented there, and built with
`CARGO_TARGET_DIR=testdata/perf/target`. The real workspace was never built
from modified sources and no game run was performed (only `bench`).

Instrumentation added in the copy:

* `physics.rs::step`: `Instant` marks around each phase, then one stderr line
  per step with `entries/pairs/constraints/contacts/triggers` **lengths and
  capacities**, `size_of` for each record, and phase milliseconds.
* `engine.rs::spawn_one`: log the first positions; optional
  `RB_UNIQUE_SPAWN=1` replaces the constant fork tag with an indexed tag
  (`tag ^ index·0x9E3779B97F4A7C15`) — this is the "fixed" variant used for the
  A/B experiment. The spawn arithmetic is unchanged when the env var is unset
  (the copy additionally logs the first six spawns to stderr).
* `RB_PHYS_DIAG=1` enables the physics diagnostic line.
* `[profile.release] lto = false`, `codegen-units = 16` in the copy only, for
  build speed. Absolute times are therefore within a few percent of, not
  identical to, the fat-LTO release.

Representative commands:

```powershell
$env:RB_PHYS_DIAG = '1'
testdata\perf\target\release\rustibites.exe bench --bibites 3000 --ticks 1 --repeat 1 --json
$env:RB_UNIQUE_SPAWN = '1'   # A/B: unique spawn streams
testdata\perf\target\release\rustibites.exe bench --bibites 3000 --ticks 3 --repeat 1 --json
```

## 3. Root cause R1 — every bibite spawns at the same point

`crates/bibites-cli/src/engine.rs`:

```rust
// spawn_one(), ~line 333 in the current tree (was 274 before commit 0bf0f13)
let mut rng = world.rng.fork(0xB1B1_7E50);
let min = world.bounds.min();
let max = world.bounds.max();
let margin = (max - min).length() * 0.05;
let low = min + Vec2::splat(margin);
let high = max - Vec2::splat(margin);
let pos = Vec2::new(rng.f32_range(low.x, high.x), rng.f32_range(low.y, high.y));
```

`crates/bibites-core/src/rng.rs:77-79`:

```rust
pub fn fork(&self, stream: u64) -> Self {
    Self::from_seed(mix_seed(self.seed, stream))
}
```

`fork` depends only on the parent **seed** and the `stream` tag — deliberately
position-independent (`rng.rs` docs). Since `spawn_one` passes the same
constant tag and `world.rng` is never advanced, every spawn constructs the same
child RNG and draws the same three values. The generated position/rotation log
proves it:

```
[spawn 0] pos=(754.823242, -590.421021)
[spawn 1] pos=(754.823242, -590.421021)
...
[spawn 5] pos=(754.823242, -590.421021)   (all logged spawns identical)
```

The logs of every coincident run (N = 500, 1000, 2000, 3000, 4000) contain six
logged spawn positions each, and each log has exactly **one distinct position**
(`distinct=1`). The unique-stream runs have six distinct positions.

With `RB_UNIQUE_SPAWN=1` (indexed tag) the same run gives, for example:

```
[spawn 0] pos=(754.823242, -590.421021)
[spawn 1] pos=(249.886841, -717.129150)
[spawn 2] pos=(-102.309448, -1068.053467)
```

`spawn_scenario_rows` calls the same `spawn_one` in a loop, so scenario loads
are affected too. `prepare_genome` spawns one bibite and is not.

Why this is the root cause and not merely a benchmark artifact:

* The world's `SpatialHash` also puts all N bibites in one cell
  (`sync_bibite_hash`), so FOV/pheromone queries return N-1 neighbours for every
  bibite until the pile disperses.
* The physics pile is a genuine all-pairs input: every collider AABB overlaps
  every other, so no broadphase can avoid the work. The solver then spends
  several ticks slowly pushing the pile apart (only 3 position iterations ×
  `maxLinearCorrection` 0.2 per contact per tick), which is why multi-tick
  benchmarks stay slow: at N=2000 the second tick still had 105,728 candidate
  pairs vs 17,991,000 on tick 1, and the count decays only gradually.

## 4. Why physics goes quadratic (R2) — the exact loops

`crates/bibites-core/src/physics.rs::step`, broadphase (current lines
1316-1335):

```rust
scratch
    .entries
    .sort_unstable_by(|a, b| a.aabb.min.x.total_cmp(&b.aabb.min.x));   // 1316-1318
for i in 0..scratch.entries.len() {                                    // 1319
    let first = scratch.entries[i];
    for second in &scratch.entries[i + 1..] {                          // 1321
        if second.aabb.min.x > first.aabb.max.x {                      // 1322
            break;                                                     // 1322
        }
        if !first.aabb.overlaps(&second.aabb) || first.body == second.body {
            continue;                                                  // 1325
        }
        scratch.pairs.push(ColliderPair { ... });                      // 1328
    }
}
```

This is a valid 1-D (x-axis only) sweep-and-prune **for non-degenerate data**:
entries are sorted by AABB `min.x`, and the inner loop stops once the next AABB
starts after the current one ends. With all bibites at one point, every AABB
has the same x-range, the break never executes, and the loop is Θ(C²)
(C ≈ 3N colliders). It also does not use the y axis to prune, so even uniform
data costs O(C²·w/W) x-overlap iterations (w ≈ collider width, W ≈ world
width); in the benchmark W ≈ 2000 units, so w/W ≈ 0.006 — a small constant,
but still quadratic in form.

Downstream, still inside `step`:

| Location | Work | Complexity with coincident bodies |
|---|---|---|
| `physics.rs:1337-1358` | deterministic pair sort; comparator indexes `bodies[a.body_a]` etc. and compares `(tag, tag, collider, collider, ...)` | Θ(P log P), P = 9·C(N,2) |
| `physics.rs:1361-1456` | narrowphase per candidate, `collide_with_skin` → `collide`; contact/constraint/trigger pushes | Θ(P) |
| `physics.rs:1424` | `scratch.contacts.push(ContactEvent { ... })` | 4·C(N,2) pushes |
| `physics.rs:1437` | `scratch.constraints.push(ContactConstraint { ... })` | 4·C(N,2) pushes |
| `physics.rs:1462-1492` | 5 velocity iterations over all constraints | Θ(5P) |
| `physics.rs:1510-1533` | 3 position iterations over all constraints | Θ(3P) |
| `physics.rs:1543-1551` | writeback over bodies | Θ(N) |

### Pair-count model (from the code, for the benchmark setup)

Every bibite body has exactly 3 colliders (`body.rs:1964` `colliders() ->
[Collider; 3]`: body capsule, tail capsule, mouth sensor box; `MAX_COLLIDERS`
is 4 but only 3 are added by `step_physics` at `body.rs:4018`,
`for collider in record.colliders()`). Once two bodies' AABBs
overlap, the double loop considers all 3×3 ordered collider combinations:

```
candidates(N) = 9 · C(N,2) = 4.5·N·(N-1)
contacts(N)   = 4 · C(N,2)     (body-body, body-tail, tail-body, tail-tail;
                                 the 4 solid-solid combinations)
triggers(N)   = 2 · C(N,2)     (each mouth box vs the other's tail capsule)
```

The other 3 of the 9 combinations produce no manifold: mouth-vs-body
capsule-box has a centre-line distance of ≈2.52·d1 against a radius+skin of
2.51·d1 (a +0.01·d1 separation at rest), so `capsule_box` rejects it; and
mouth-vs-mouth is box-box, which `collide` explicitly returns `None` for
(`physics.rs:1013`). They are not *duplicate* pairs — each is a distinct
collider combination — but they are 9× the body-pair count, and at least 1/9
(the box-box mouth-mouth pair) is structurally wasted; in the rest pose all 3
are wasted. Measured counts match the formulas exactly at every N
tested, which also proves that pellet-pellet and bibite-pellet pairs contribute
nothing here: the default ecology seeded only 92 pellets, all far from the
spawn point (bodies = N + 92, entries = 3N + 92, pairs = 9·C(N,2) exactly).

Pellet-pellet pairs *are* enabled by the layer matrix (`(9,9)` in
`ENABLED_PAIRS`, `physics.rs:138`) and are generated when pellets overlap; they
only become quadratic if pellets pile up (e.g. repeated meat piles at one
corpse position). They are not a factor in this benchmark.

### Measured phase breakdown (tick 1, coincident)

| N | scan ms | entry sort ms | pair sort ms | narrowphase ms | velocity solve ms | position ms | total ms |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 500 | 9.9 | 0.0 | 90.8 | 401.5 | 22.2 | 17.0 | 541.5 |
| 1000 | 47.9 | 0.0 | 523.4 | 1671.7 | 88.2 | 69.9 | 2401.3 |
| 2000 | 170.4 | 0.0 | 1745.8 | 6309.5 | 354.6 | 290.8 | 8871.3 |
| 3000 | 342.1 | 0.0 | 4120.2 | 15375.4 | 1423.4 | 1189.1 | 22450.7 |
| 4000 | 779.9 | 0.1 | 8268.8 | 26607.5 | 1700.6 | 1269.7 | 38627.2 |

Per candidate pair the cost is essentially constant: ≈0.37 µs narrowphase,
≈0.10 µs pair sort, ≈0.02 µs scan, ≈0.06 µs solver/position, ≈0.55 µs total.
That is why the wall time tracks 9·C(N,2) so closely:

```
T_tick ≈ 0.55 µs · 4.5·N² ≈ 2.5·10⁻⁶·N² seconds
N=3000 → 22.3 s (measured 22.45)
N=4000 → 39.6 s (measured 38.63)
```

The model under-predicts the reported 5000/6000 numbers (61.9 s / 89.1 s vs
127 s / 157 s). Those runs allocate 12.3 GiB / 24.5 GiB of scratch; commit charge,
page faults and possible paging dominate beyond the ~4000 point on ordinary
machines, so the simple per-pair model is a lower bound there.

## 5. The 20 GiB allocation (R3)

There is **no capacity arithmetic in the physics hot path**. `PhysicsScratch`
starts empty (`physics.rs:1163-1170`) and `step` only calls `clear()`
(`physics.rs:1277`), which keeps capacity but does not reserve. The four
buffers grow through `Vec::push` at `physics.rs:1328` (pairs), `1424`
(contacts), `1437` (constraints) and `1380` (triggers). Rust's `RawVec` doubles
capacity when full, so capacities are powers of two and capacities persist
across ticks.

`PhysicsScratch::with_capacity` (`physics.rs:1179`) exists but has **no callers**
in the workspace, so its `pairs` argument cannot overflow today. `ColliderEntry.body`
is a `u32` cast (`physics.rs:1310`) and would wrap only above 4.29 B bodies.
Neither is involved.

The failure number identifies the buffer exactly. Measured record sizes from
the instrumented run:

```
sizes[e=24 p=12 c=64 ct=80 tr=56]
      ColliderEntry=24, ColliderPair=12, ContactConstraint=64, ContactEvent=80,
      TriggerEvent=56
```

`80 B × 2²⁸ = 21,474,836,480 B = 5·2³² = 20 GiB`, i.e. the next doubling of
`Vec<ContactEvent>` once `contacts.len()` exceeds 2²⁷ = 134,217,728. Since
contacts = 4·C(N,2) = 2N(N−1):

```
2N(N−1) > 2²⁷  ⇔  N(N−1) > 67,108,864  ⇔  N ≥ 8193
```

At N=8192 contacts = 134,201,344 (just under 2²⁷); at N=8193 contacts =
134,234,112, so the next push requests 2²⁸ elements and the allocator is asked
for exactly 20 GiB. At N=10000 contacts = 199,980,000, also 2²⁸ capacity, so
the same request occurs. This is why the message is *specifically*
`21474836480` and why it only appears at the very large populations.

The complete capacity table (measured up to N=4000, calculated above; MiB =
capacity × element size):

| N | pairs cap | constraints cap | contacts cap | triggers cap | total |
|---:|---:|---:|---:|---:|---:|
| 1000 | 2²³ = 96 MiB | 2²¹ = 128 MiB | 2²¹ = 160 MiB | 2²⁰ = 56 MiB | 440 MiB |
| 2000 | 2²⁵ = 384 | 2²³ = 512 | 2²³ = 640 | 2²² = 224 | 1,760 MiB |
| 3000 | 2²⁶ = 768 | 2²⁵ = 2,048 | 2²⁵ = 2,560 | 2²⁴ = 896 | 6,272 MiB (6.1 GiB) |
| 4000 | 2²⁷ = 1,536 | 2²⁵ = 2,048 | 2²⁵ = 2,560 | 2²⁴ = 896 | 7,040 MiB (6.9 GiB) |
| 5000 | 2²⁷ = 1,536 | 2²⁶ = 4,096 | 2²⁶ = 5,120 | 2²⁵ = 1,792 | 12,544 MiB (12.3 GiB) |
| 6000 | 2²⁸ = 3,072 | 2²⁷ = 8,192 | 2²⁷ = 10,240 | 2²⁶ = 3,584 | 25,088 MiB (24.5 GiB) |
| 8000 | 2²⁹ = 6,144 | 2²⁷ = 8,192 | 2²⁷ = 10,240 | 2²⁶ = 3,584 | 28,160 MiB (**27.5 GiB**) |
| 8193 | 2²⁹ = 6,144 | 2²⁸ = 16,384 | 2²⁸ = **20,480** | 2²⁷ = 7,168 | 50,176 MiB (**49 GiB**) |
| 10000 | 2²⁹ = 6,144 | 2²⁸ = 16,384 | 2²⁸ = 20,480 | 2²⁷ = 7,168 | 50,176 MiB |

Nuance worth recording: the measured *resident* set is smaller than the
capacity sum because `Vec` growth allocates capacity but only `len` elements
are written. At N=3000 the capacity total is 6.1 GiB but sampled peak working
set was 4,280 MB (touched elements ≈ 3.4 GiB plus world state). The analytical
touched set at N=4000 is ≈6.2 GiB against a 6.9 GiB capacity total. Windows
commit charge, however, tracks the full allocation, which is what the allocator
fails on.

**Why the report says N=8000 also produced the 20 GiB message:** strictly by
the arithmetic above, N=8000 requests 10 GiB for contacts (2²⁷ capacity), not
20 GiB; at 8000 the *aggregate* 27.5 GiB is what exceeds a 32 GiB host. The
exact 20 GiB request starts at N=8193. If a run at exactly 8000 really prints
`21474836480`, one of the assumptions is off by a little: extra contacts from
pellet/bibite overlaps (~6.2 M extra contacts would be needed — impossible with
92 pellets), a different collider count, or the population was actually ≥8193
(e.g. a bench that adds seeded bibites to the requested N). The confirming
measurement is a one-line `eprintln!("{}", scratch.candidate_pair_count())`
plus a contacts-length print at N=8000 on a ≥64 GiB machine: the model predicts
127,984,000 contacts. On this 16 GiB host the run cannot get that far.

## 6. A/B proof: fixing the spawn stream removes the quadratic

Same binary, same N, only `RB_UNIQUE_SPAWN` differs:

| N | mode | candidates tick 1 | contacts | wall time | tick-1 physics |
|---:|---|---:|---:|---:|---:|
| 500 | coincident | 1,122,750 | 499,000 | 1.092 s / 100 ticks | 0.542 s |
| 500 | unique | 32 | 14 | 0.067 s / 100 ticks | 0.0002 s |
| 2000 | coincident | 17,991,000 | 7,996,000 | 9.153 s / 3 ticks | 8.871 s |
| 2000 | unique | 529 | 189 | 0.0068 s / 3 ticks | 0.0014 s |
| 3000 | coincident | 40,486,500 | 17,994,000 | 22.704 s / 1 tick | 22.451 s |
| 3000 | unique | 1,161 | 409 | 0.0174 s / 3 ticks | 0.0030 s |
| 4000 | coincident | 71,982,000 | 31,992,000 | 39.002 s / 1 tick | 38.627 s |
| 4000 | unique | 2,060 | 736 | 0.0230 s / 3 ticks | 0.0053 s |

At N=4000 the first tick is ≈7,300× faster and the 3-tick run ≈1,700× faster.
Candidate pairs become a small O(N)-ish quantity (32 → 2,060 over an 8×
population increase; the residual growth is the x-only sweep's O(N²·w/W), which
a grid removes), and per-tick time drops from tens of seconds to microseconds.

This is the decisive experiment: a change *only* to the RNG stream tag, with
physics untouched, eliminates the superlinearity. The solver is where the time
is spent, but the spawner is why it is spent.

## 7. Secondary findings

### 7.1 Non-physics superlinearity in `World::tick`

`World::tick` (`world.rs:2241`) is now phase-instrumented by a concurrent
change (`Phase::*`, `mark_phase`, `World::phase_timings()` at `world.rs:2310`);
the numbers below predate that instrumentation.

* **`step_sensing`** (`world.rs:2454`): one `hash.query_circle` per bibite for
  pellets and bibites (`2503`, `2529`). With all bibites in one hash cell every
  query returns N−1 bibites, so FOV accumulation is O(N²) on sense-due ticks.
  Measured contribution at N=3000 was <1% of the tick (physics was 22.45 s of
  22.70 s), because sensing is throttled; after the spawn fix it is negligible,
  but it is still quadratic for genuinely dense groups and should be watched.
* **Held-pellet pinning** in the `step_physics` bridge (`body.rs:4113`):
  `pellet_ids.iter().position(...)` inside a per-held-target loop is O(N·P).
  It becomes significant only when many bibites hold pellets (a feeding pile).
* **Egg abort lookup** (`world.rs:2676`): `egg_positions.iter().find(...)`
  inside the aborted-egg loop is O(E·A).
* **Per-tick clones** in `BibiteStore::tick` (`body.rs:3180-3194`):
  `ids`, `bites`, `swallows`, `deaths` are cloned each tick (O(N) allocation
  churn, not superlinear).
* **Hash reconciliation** (`world.rs:2933` `sync_all_hashes`): four
  retain+update passes per tick, O(N + pellets + eggs + pheromones); correct,
  but it is unconditional work every tick.
* **Collision damage fan-out** (`body.rs:4230-4254`,
  `for contact in scratch.physics.contacts()`): every solid contact event
  applies `collision_damage` to both bodies. A coincident bibite pair produces
  4 solid contacts, i.e. up to 8 damage applications per body pair. This is
  plausibly faithful to Unity's per-collider `OnCollisionEnter2D` callbacks,
  but it is worth a parity check — it is a semantics question, not the scaling
  bug.

### 7.2 The broadphase's own deficiencies (fix even after R1)

* It is a 1-D x sweep: no y pruning, so uniform data still costs O(C²·w/W).
* It allocates pairs for layer combinations that the narrowphase later discards
  (`collision_allowed` is checked at `physics.rs:1364`, after the sort).
* Box-box candidates are always generated and always return `None`
  (`physics.rs:1013`); for bibites that is the mouth-mouth pair, 1/9 of the
  coincident candidates.
* The pair sort comparator performs two random `bodies[...]` lookups per
  comparison (`physics.rs:1337-1358`); storing the body tag in `ColliderPair`
  would make it a cheap integer/`u128` sort.
* `capsule_box` (`physics.rs:913-958`) runs a fixed 32-step ternary search per
  call; it is called for every mouth-vs-capsule candidate and is a large part
  of the 0.37 µs/pair narrowphase cost.

### 7.3 A mouth-trigger correctness bug found incidentally

At rest, the mouth box's near face (offset +2.38·d1, half extent 1.36·d1) is
≈2.52·d1 from the body capsule's axis (offset −1.5·d1). With the capsule radius
2.5·d1 grown by the 0.01 contact skin, the separation is ≈+0.01·d1, just
outside `capsule_box`'s 1e-4 acceptance threshold. A resting mouth therefore
does not reliably report the body directly in front of it; in the all-coincident
pose the 2 measured triggers per body pair are consistent with mouth-vs-tail
(one per direction), while mouth-vs-body is rejected by that margin. This is a
separate correctness issue; do not silently change it while fixing performance,
because it changes bite/grab behavior and parity.

## 8. Proposed fixes

### F1 (required, minimal) — unique RNG stream per spawn

`crates/bibites-cli/src/engine.rs`, `spawn_one` (~line 333) and its callers.
Sketch:

```rust
// spawn_synthetic_bibites / spawn_scenario_rows: pass a unique index
fn spawn_synthetic_bibites(world: &mut World, count: usize) -> Result<usize, String> {
    for index in 0..count {
        spawn_one(world, ..., index as u64)?;
    }
    Ok(count)
}

fn spawn_one(
    world: &mut World,
    // ... existing args ...
    spawn_index: u64,
) -> Result<(), String> {
    // was: let mut rng = world.rng.fork(0xB1B1_7E50);  // same stream every time
    let mut rng = world.rng.fork(
        0xB1B1_7E50 ^ spawn_index.wrapping_mul(0x9E37_79B9_7F4A_7C15),
    );
    // ... unchanged ...
}
```

Equivalent and slightly cleaner: `Rng::derive(world.rng.seed(), 0xB1B1_7E50,
spawn_index)` (both `derive` and `seed()` are public). Either way the world's
own RNG stream is not advanced, so other subsystems' streams are unchanged.

Regression test to add (CLI or core): spawn 64 starter bibites and assert that
(i) not all positions are equal, (ii) the minimum pairwise distance is > 0,
(iii) `bibites_start == 64`. Also assert that scenario-row spawns are distinct.

Expected effect: measured above — N=4000 tick 1: 38.6 s → ~5 ms; 3-tick run
39.0 s → 23 ms. This single change fixes the reported benchmark.

### F2 (recommended) — physics-local uniform grid broadphase

Replace `physics.rs:1316-1335` (sweep) with a grid over the collider AABBs.
No public API change is needed; derive the grid extent from the entries.
Pseudocode:

```rust
const CELL: f32 = 16.0;        // > max AABB extent (body capsule ≈ 9, mouth ≈ 7)
const MAX_CELLS: usize = 1 << 20;
let lo = entries.iter().map(|e| e.aabb.min).fold(Vec2::splat(f32::INFINITY), Vec2::min);
let hi = entries.iter().map(|e| e.aabb.max).fold(Vec2::splat(f32::NEG_INFINITY), Vec2::max);
let nx = (((hi.x - lo.x) / CELL).ceil() as usize + 1).clamp(1, 1024);
let ny = (((hi.y - lo.y) / CELL).ceil() as usize + 1).clamp(1, 1024);
if nx * ny > MAX_CELLS {
    // pathological extents: keep the current sweep as a fallback
} else {
    // counting sort: cell_start[cell] → prefix sums → scatter entry indices
    // for each cell c (row-major), for each entry i in c:
    //   for each of the 9 neighbouring cells (including c):
    //     for each entry j in that cell:
    //       if j <= i { continue }                  // emit each pair once
    //       if !aabb_overlap || same body { continue }
    //       if !collision_allowed(layer_a, layer_b) { continue }   // moved earlier
    //       if shape_box(a) && shape_box(b) { continue }           // always None
    //       pairs.push(...)
}
scratch.pairs.sort_unstable_by(/* same deterministic key as today */);
```

Notes:
* Store `tag`, `layer` and a 2-bit shape kind in `ColliderEntry` so the inner
  loop does not dereference `bodies` and can prune early.
* Keep the final deterministic sort (or emit in `(cell, body tag)` order) so
  solver order and determinism are unchanged.
* Expected cost: O(C + P) for uniform density. At N=10,000, C ≈ 30,000 and
  P ≈ 2,000-3,000 — trivial. A genuinely coincident pile still produces
  O(C²) pairs because the bodies truly overlap; the grid is not a substitute
  for F1.
* Validate with the existing `broadphase_contacts_match_brute_force` test plus
  a new randomized test that compares the grid pair set to the old sweep on
  10,000 random bodies (same contacts/triggers, same order after sort).

### F3 (recommended) — buffer discipline and a guarded capacity

* Reserve once per body count instead of growing inside the loop:
  `scratch.pairs.reserve(colliders * 4)`, `contacts.reserve(colliders * 2)`,
  `constraints.reserve(colliders * 2)`, `triggers.reserve(colliders)` using
  `checked_mul` (there is no wrap today, but a future caller such as
  `PhysicsScratch::with_capacity(colliders * colliders)` would overflow).
* Add a deterministic sanity cap: if `pairs.len() > 64 * entries.len()` (or
  `contacts.len() > 32 * entries.len()`), print a one-line diagnostic with the
  counts and either fall back to a coarser cell size or return an error from a
  future fallible `step`. Fail fast beats a 20 GiB allocation attempt. Do not
  silently drop contacts.
* `clear()` already retains capacity; consider `shrink_to` only when the pair
  count has been far below capacity for many ticks, so a one-off pile does not
  pin 6-27 GiB for the rest of the run.

### F4 (optional, constant factor) — cheaper capsule-box

Replace the 32-step ternary search in `physics.rs:913-958` with a closed-form
segment-vs-oriented-box distance, or reduce to ~12 iterations with the same
`1e-4` tolerance. This is a 2-3× narrowphase improvement on mouth-vs-capsule
candidates; it does not change complexity.

### Suggested order

1. F1 (one function, one test) — fixes the benchmark.
2. Re-baseline with `tools/perf/bench_scaling.ps1` and compare against the
   table in §5; confirm N=8000 now runs in O(N).
3. F3 (cheap, prevents a future 20 GiB surprise), then F2 (grid), then F4.
4. Add a `--spawn=cluster` (or scenario) stress case if dense-pile performance
   is a product requirement; do not use it as the default benchmark.

Expected complexity after F1+F2+F3: O(N) bodies + O(C + P) broadphase +
O(P log P) deterministic sort, with P ≈ O(N) at the default density; per-tick
work becomes dominated by sensing and the O(N) hash updates. Dense piles remain
Θ(P) with P = k·N²/2 by nature; the guard in F3 bounds memory.

## 9. Residual risks and open questions

1. **N=8000 vs N=8193.** The exact 20 GiB request is predicted at N≥8193; at
   N=8000 the model predicts a 10 GiB contacts request and a 27.5 GiB aggregate.
   The confirming measurement is `contacts.len()`/capacity at N=8000 on a
   ≥64 GiB host. Until then, treat the "8000" datapoint as "the 27.5 GiB
   aggregate plus the rest of the process exhausts a 32 GiB host", not as proof
   of the 20 GiB threshold.
2. **Dense real scenarios.** Fixing the benchmark spawner does not make a
   genuine 8,000-bibite pile cheap; no broadphase can avoid the inherent
   overlaps. F3's guard and/or a per-body contact budget is needed if such
   scenarios are expected.
3. **Determinism/baselines.** F1 changes every synthetic benchmark trajectory
   (and any recorded state fingerprints/baselines). Scenario runs use explicit
   spawn rows and are unaffected. Re-baseline perf and determinism CI after the
   change.
4. **Collision damage multiplicity.** Four solid contacts per coincident body
   pair produce up to 8 damage applications; verify against the Unity capture
   before "deduplicating" contacts. Deduplicating body pairs for damage would
   change behavior, while deduplicating *broadphase* work is safe.
5. **Mouth trigger gap** (§7.3) is a correctness bug adjacent to the hot path;
   fixing it will change trigger counts and therefore physics-adjacent results.
6. **Concurrent edits.** `world.rs` is being actively changed (phase timings
   and save/load work). Current line numbers will drift; anchor by function
   name. Per-phase confirmation should use the new
   `World::phase_timings()`/`Phase::Physics` rather than the old line numbers.
7. **Build profile.** Measurements used an LTO-disabled copy; upstream fat LTO
   changes absolute times by a few percent but not the quadratic shape.
8. **Allocator dependence.** Power-of-two growth is a Rust `RawVec`
   implementation detail; explicit reserve (F3) makes the capacity (and thus
   the failure point) deterministic across toolchains.

## 10. Appendix — exact diagnostics and artifacts

Instrumented scratch line at N=4000 (one tick):

```
[phys] bodies=4092 entries=12092 pairs=71982000 constraints=31992000
       contacts=31992000 triggers=15996000
       caps[e=16384 p=134217728 c=33554432 t=16777216]
       MB[p=1536.0 c=2048.0 ct=2560.0 tr=896.0]
       ms[copy=0.1 vel=0.0 ents=0.5 esort=0.1 scan=779.9 psort=8268.8
          narrow=26607.5 vsolve=1700.6 pos=1269.7 total=38627.2]
       sizes[e=24 p=12 c=64 ct=80 tr=56]
```

`bodies=4092` = 4000 bibites + 92 pellets; `entries=12092` = 3·4000 + 92.
`pairs=71,982,000 = 9·C(4000,2)`; `contacts=31,992,000 = 4·C(4000,2)`;
`triggers=15,996,000 = 2·C(4000,2)`.

Sweep tool (checked in): `tools/perf/bench_scaling.ps1`

```powershell
# full default sweep, one tick per population, CSV next to the script
./tools/perf/bench_scaling.ps1

# short sweep, five ticks, custom output
./tools/perf/bench_scaling.ps1 -N 500,1000,2000,3000 -Ticks 5 -OutFile .\scaling.csv
```

Output columns: `n, ticks, repeat, status, exit_code, process_wall_secs,
report_wall_secs, ticks_per_second, bibite_updates_per_second, bibites_start,
bibites_end, peak_ws_sampled_mb, free_gb_before, error`. Status distinguishes
`ok`, `oom` (with the exact byte count from the allocator), `timeout`, `error`
and `skipped_low_mem`.

The instrumented copy (`testdata/perf/ws`, `testdata/perf/target`) was deleted
after the measurements above were recorded; `testdata/` is gitignored. No file
under `crates/` was modified by this diagnosis.

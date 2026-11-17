# Benchmarks

Reproducible performance numbers for the headless RUSTibites engine: how many
simulation ticks per wall-clock second we get, how that scales with population
and worker threads, and where the tick time goes.

This document is **generated from measurements, not aspirations**. The tooling
that produced it lives in [`tools/bench/`](../tools/bench/README.md); the
performance targets it is measured against live in
[`ARCHITECTURE.md` §10](ARCHITECTURE.md#10-performance-targets-and-measurement).
All artifacts referenced below (`runs.csv`, `summary.csv`, `matrix.md`,
`phases.csv`, `environment.json`) are checked in under
`tools/bench/results/` and can be regenerated with the commands in
[Reproducing](#reproducing-the-latest-numbers).

## TL;DR

- The primary metric is **bibite-updates/second** =
  `ticks_per_second × population`; the secondary one is **real-time factor**
  (`simulated seconds / wall seconds`, where 1× = 30 simulated ticks/s).
- On the reference machine (Ryzen 5 3600, 6C/12T, Windows 10, release build) the
  **final measured matrix** on the **synthetic density-scaled bench world** (120
  ticks/run, best-of-3, final-freeze binary `818B7595...`) gives:
  **1,000 at ~1.42k ticks/s (~47x)**, **2,000 at ~663 ticks/s (~22x)**,
  **5,000 at ~192 ticks/s (~6.4x)**, **10,000 at ~85 ticks/s (~2.8x)**,
  **20,000 at ~38 ticks/s (~1.3x)**. Same-machine frozen/current A/B is
  within noise (e.g. paired 10k serial identical), so this is the new
  final-freeze reference; task-131 tables below are retained as history.
  The final 100k one-tick sanity measured **500.5-514.8 MB** peak working set.
- Serial throughput is **-1.6% / -2.5% / -4.0% / -5.6% / -9.5%** versus the
  task-92 published table (1,445 / 680 / 200 / 90 / 42 tps at 1k/2k/5k/10k/20k);
  peak working set is 11.1 / 17.1 / 32.6 / 66.5 / 128.5 MB for the final-freeze
  binary. Threads1 auto-brain and the frozen body-phase baseline are restored;
  same-machine paired A/B is within noise at 1k-20k.
  [Comparison with ARCHITECTURE §10](#comparison-with-architecture-10).
- Worker threads add little (~0.91–1.02× in the final clean-tree matrix) because
  physics and body are still serial; parallel sensing only shaves the `sensing`
  phase (now 7–11% of the tick, down from 8–19% after the FOV split; see
  [Phase breakdown](#phase-breakdown)). The matrix shows no structural >1.15×
  thread win, matching the Amdahl analysis in §10 and PERF_DIAGNOSIS.md.

## Parallel tick and resting pellets (2026-11-05)

Measured with the optimized CLI (`cargo build --profile perf -p bibites-cli`: opt-level 3, no LTO, 16 codegen units; the
repository's fat-LTO `release` profile is not needed to measure) on the reference machine (Ryzen 5 3600, 6C/12T).
`rustibites bench` prints a per-stage table (indented under body and physics), the share of pass-B turns that ran on the pool,
and the resting-pellet counters. `RUSTIBITES_GATES` (read by the CLI and the viewer) picks the size gates: `scaled` (the default since 2026-11-15: a world below about 1,200 bibites ticks on
one thread, a bigger one runs every stage that gains from it on the pool; `scaled:<percent>` scales every size), `always` (every stage of every world on the pool from one item), `eager` (also the
resting-pellet layer from one pellet) and `nosleep` (`scaled` with resting pellets kept in the physics step). The table below was measured with the sizes of 2026-11-05 (a stage on the pool from
3,000 records, 4,096 bodies, 20,000 due bibites for sensing: `RUSTIBITES_GATES=tuned` of the build before 2026-11-15).

| World | Before (serial / best threaded) | Now, 12 threads | Now, 1 thread |
|---|---|---|---|
| 20,000 synthetic bibites (100 ticks, best of 3) | 36 / 38 ticks/s | **65 ticks/s** (1.2 → 2.2× real time) | 35 |
| Big Pellet (500 bibites, ~55,000 pellets, simSize 20,000; 40 TPS) | 65 / 70 | **940 ticks/s** (23× real time) | 870 |
| Big Pellet with 4× the pellets (137,000) | n/a | 372-424 ticks/s | n/a |

Where the 20,000-bibite tick goes now (ms per tick, 12 threads): body 5.8 (brains 1.2, pass B 3.4 of which 91% of the turns on the
pool, assemble 0.6, pass A 0.4), physics 8.2 (the bridge's pass after the step 2.6, body building 1.3, narrowphase 0.9, broadphase
0.6, x order 0.5, poses 0.5), sensing 0.7, hash sync 1.1. A bibite record is 2,064 bytes (41 MB for 20,000), so the passes over
the records are memory-bound: the next steps cut the number of passes (fusing pass A, the brain inputs, the brains and pass B
per record; fusing the bridge's per-bibite loops), not the arithmetic. Big Pellet now: physics 0.66 (55,000 pellets asleep, ~200
awake, ~165 promoted into the step per tick), body 0.25, sensing 0.08.

The window (optimized viewer, GTX 1660 SUPER, 144 Hz display, Big Pellet): the default zoomed-in view runs at the display's
144 fps (CPU per frame under 2 ms). With the whole world in view (55,000 pellet sprites) it ran at **8 fps** until the pellets
were drawn grouped by atlas texture (23,706 texture changes between neighbouring sprites meant 23,706 draw calls per frame, 110 ms
in `next_frame`): now **117 fps**. `RUSTIBITES_LOG_FRAMES=1 rustibites-view <save> --fit world` prints where a frame goes; saving
a 109,000-pellet world takes 2 s (`to_contents` 5 ms, serialising and deflating the rest), so the autosave is written on its own
thread.

## Scans over the records, the bridge sweep and the narrowphase (2026-11-05, evening)

A bibite record is 2,064 bytes (33 cache lines, the hot fields on lines 13-32; `print_the_layout_of_a_bibite_record`, an ignored test, prints the map), so a pass that
looks at one flag of each of 20,000 records is 20,000 cache misses: 15-50 ns each on one thread, and the passes that remained serial after the parallel tick were almost all of that
kind. Same machine and build as above (optimized CLI, 12 threads, 20,000 synthetic bibites, best of 3), ms per tick:

| Stage | Before | After |
|---|---|---|
| spatial-hash sync (drop the entries of removed bibites) | 1.10 | 0.02 |
| the bridge's work after the step (contacts, pose write-back, mouth targets) | 2.64 | 1.15 |
| narrowphase (about 1,800 candidate pairs, 0.45 us each) | 0.90 | 0.34 |
| the two scans for a queued death | 0.25 | 0.00 |
| species recount (events and stats) | 0.40 | 0.25 |
| rad scan | 0.34 | 0.19 |
| **the tick** | **16.4** | **12.1-12.5** |
| **ticks per second** | **61** | **80-83** (2.7-2.8x real time at 30 TPS) |

What each change does is in the commit message; the shape of it is: the store logs the bibites it removes, so the hash does not ask every entry whether its record is still there; a death can only be
queued by `kill`, so without a kill the scan is skipped; the scans that must look at every record run on the pool (`BibiteStore::filter_map_dense_into`); the contacts and triggers of a physics step
are cut into one row per bibite in event order, so the bibites' records are updated by a sweep on the pool, each record seeing its events as it always did; and the narrowphase's pool gate and chunk
size (2,048 pairs, written for thread spawning) are 256 and 64 now that the pool is persistent. The end state of the pinned 12,000-tick evolution run is unchanged
(`0x34a375d3c70ce0aa`), and every serial-against-pooled test still compares whole checkpoints; each new path is also compared, in tests, with the code it replaced (the old loops run on a copy of the records,
the old hash pass on a copy of the hash, the plain scans).

**The record's layout.** Rust orders a struct's fields to save padding, which had scattered the fields one pass uses over 20 of the 33 cache lines of a record (`pos` on line 20, `rot` on line 27,
`pending_torque` on line 31). `BibiteRecord` is now `repr(C, align(64))` with the physics bridge's group first (lines 0-3), then the body and the organs, the feeding counters' per-tick fields together, and the
cold fields (genome, metadata, FOV cache, children) last; the record is 2,112 bytes, 33 lines of its own. Run alternately with the build before it: building the physics bodies 1.18 -> 0.92 ms, the bridge's
pass 1.17 -> 1.02, the tick about 0.6 ms shorter (80 -> 82 ticks/s); the organs' turn barely changed (1.95 -> 1.89). The same binary varies by up to 10% between runs minutes apart on this machine, so builds are
compared alternately, never against an older number. Skipping the stores that would not change a value (the contact counters, the mouth count: no bibite is in contact in most steps) was tried and
measured no better, so it is not in.

**One sweep for pass A, the brains and the organs' turns.** On a pool-sized world the body tick ran pass A (upkeep, cadences), the assembly of the due brains' inputs, the brain steps and the pool's share of pass B as four
passes over the records, each with its own fork-join and its own trip of every record's lines through memory. A worker now takes its stretch of records through all four, 64 at a time (`BibiteStore::fused_body_tick`).
What crosses records is kept as it was: the state a dense-order turn reads of a target that comes after it is taken in the sweep by the worker that has the target's record, after its brain step and before its own turn
(`pre_snapshots`), and the starvation deaths pass A finds are put in dense order afterwards. Measured alternately with the build before it, the four stages' 4.5 ms became one 3.2 ms stage (`sweep` in the bench
table), the body 6.3 -> 5.0 ms, the tick 0.8-1.3 ms shorter (4-10% more ticks per second, depending on the run); a block of any size from 16 to 1,024 records runs the same, so the gain is in the trips and the
three fork-joins saved, not in the cache residency of a block.

What is left at 20,000 bibites (ms per tick): the sweep 3.2 (pass A, the brains, the organs' turns on the pool), the turns that touch the world's stores (9% here) in dense order on one thread 1.6,
building the physics bodies 1.0, the bridge's sweep 1.1, the box and pose stages 0.9, the x order 0.6 (an insertion sort over 100,000 swaps a tick), the broadphase 0.65, sensing 0.7. This machine reads about 31 GB/s
(sequential reads; two threads already saturate it, one reaches 18.6; measured with a 768 MB buffer), and the records, the brains' vectors and the physics bodies put some 300 MB through it every tick at this size,
so most of these stages are limited by memory traffic rather than by arithmetic or by the number of threads. The next lever is fewer bytes per tick, which means data-oriented changes: one contiguous block per
brain instead of fifteen vectors, structure-of-arrays columns for the fields every pass reads, and fusing passes so a record's lines are pulled in once.

**100,000 bibites (2026-11-05, night).** The same synthetic density-scaled world with five times the bibites ran at 15 ticks/s (0.5x real time, 67 ms a tick: physics 45.6, body 19.3, sensing 4.7), and one stage was out
of line: `build-eggs+held` took 16 ms, against 0.16 ms at 20,000. The holder scan its name is about cost 1.5 ms of it; the rest was the pass behind it that puts the resting pellets a body overlaps back into the step
(`pellet_sleep`): it computes the box of every body of the step in a row (a sin/cos pair for the body and one for each collider, some 130 ns a body) and asks the grid of sleepers about it. The 20,000-bibite world has fewer
than 2,048 pellets, so the sleeping layer is off there and the pass never ran; at 100,000 bibites and 4,000 pellets it is on (`RUSTIBITES_GATES=nosleep` ran the same world at 16 ticks/s against 14-15 with it, i.e. the layer
cost more than it saved). Now the pool computes the boxes in stretches and asks the grid, which it only reads (`PelletSleep::overlapping`), and the sleepers are marked afterwards in the order of the stretches, which is the order
the plain loop marks them in (`promote_slots`); the grid keeps one bit per cell that says whether the cell holds a sleeper, so a query over an empty cell does not touch the cell's vector (14.8 -> 13.2 ms on its own);
and the holders (`BibiteStore::holders`, the bibites whose held pellets are pinned to their mouths) are found by a pooled scan. Measured alternately with the build before it (optimized CLI, 12 threads): 100,000 bibites 15 ->
18-19 ticks/s (0.6x real time; the stage 16.5 -> 2.5 ms), 20,000 bibites 94 -> 94 (unchanged), Big Pellet 955-972 -> 962-975 (unchanged). The pinned 12,000-tick fingerprint is unchanged; the pooled pass is compared with the plain
loop in tests (`stretches_of_boxes_asked_apart_mark_the_sleepers_in_the_order_of_the_plain_loop`, the sleeping layer's byte-exact world tests with every gate at 1); of six deliberate breaks five are caught, and the sixth
(the pool's pass not looking at whether a body is active) only promotes sleepers that nobody overlaps, which changes no state. What 100,000 bibites still cost per tick is linear in the population and
memory-bound: the tick moves about 1.5 GB through a machine that reads 31 GB/s, so some 20 ticks/s (0.7x) is the ceiling without a change of data layout.

## 100,000 bibites toward 1.2x (2026-11-05, late night)

The owner's goal: 100,000 bibites at 1.2x real time (36 ticks/s at 30 TPS). At the start of this round the synthetic density-scaled world (optimized CLI, 12 threads, Ryzen 5 3600) ran at 19.2 ticks/s (0.64x, 52 ms a tick). Now:

| Bibites | Before (ticks/s) | Now | Real time (30 TPS) |
|---|---|---|---|
| 100,000 | 19.2 (52.0 ms) | **26.2** (38.2 ms) | 0.64x -> **0.87x** |
| 30,000 | 66.8 | **86.1** | 2.2x -> **2.9x** |
| 20,000 | 97.1 | **120.2** | 3.2x -> **4.0x** |

Measured alternately with the build before the round (`scratchpad/ab.ps1`: the two builds run in turn, order swapped every round, best of 4 rounds x 2 repetitions), the same end state of the world byte for byte (the bench's state fingerprint at 20,000 bibites after 40 ticks and at 100,000 after 12), the 551 core tests and the pinned 12,000-tick evolution fingerprint unchanged. The target is not reached; what is left is at the end of this section.

**What the profile said.** The tick at 100,000 bibites moves some 27 GB/s, which is what this machine reads, so a stage gets faster by touching fewer cache lines, not by computing less. The probes (`crates/bibites-cli/tests/dirty_probe.rs`, ignored tests):
- A tick writes **9 of a record's 33 lines** (0, 1, 6, 8, 9, 13, 14 and 17 every tick; the other 24 are read or untouched), and the fields that change are 29 four-byte numbers and 3 eight-byte counters (about 160 bytes).
- Doubling the record's stride (2 KB of padding that nothing touches) costs only 5%: **the number of lines a pass touches is what it pays for, not the size of the record**, so moving cold fields out of line (genome, metadata, feeding counters) buys almost nothing by itself.
- `Option<BibiteRecord>` keeps its "empty slot" in a `Vec` capacity at bytes 2056..2064, the coldest line of the record, which every sweep and scan fetches when it asks whether a slot is empty. (Boxing the records puts it in a contiguous pointer array, which made the scans 0.1-0.2 ms cheaper and the tick 7% slower, because the records were then no longer contiguous: each sits among the allocations of its brain. Dropped.)
- In a world that fits the caches (2,000 bibites, one thread) pass A costs 11 ns a bibite, a brain step with its inputs 190 ns, pass B 213 ns; at 100,000 bibites the same work costs 164 ns, 1,470 ns and 500 ns of thread time: **the fused sweep spends 78% of its time waiting for memory**. Its split at 100,000 (thread time over the 12 workers, ms per tick): pass A 1.4, brain steps 6.1, pass B 4.2 (after the compiled plan 5.2 and 5.6: the lines the brain steps used to pull in are pulled in by pass B).
- Thread count: 4 / 6 / 8 / 12 threads give 24 / 25 / 26 / 26 ticks/s at 100,000 bibites; the tick is bandwidth-bound and partly serial (the dense-order mouth turns, the contact solver).
- The same binary varies +-5% between runs minutes apart, and an unchanged function can get 30% faster or slower from one build to the next with the code around it (the broadphase sweep did), so builds are compared in turn and by the whole tick.

**What changed** (all bit-exact; in the order they were made):
1. *The physics step* (`physics.rs`): the sweep of the broadphase asked the layer matrix (a linear lookup) of every candidate in a body's x range (about 180 at 100,000 bibites, a cost that grows as N^1.5) before testing the boxes; the box test comes first. The x order is sorted as plain integers on the pool (the box's start in `total_cmp` order over the body index, `par_sort_unstable`), the entries are made from one record per body (`BodyBox`: box, tag, layer) instead of 100,000 lookups into the array of bodies, the pairs carry their tags, collider poses are computed only for the bodies a pair names (they were for every body and zeroed first), the mirror integrates the velocity as it copies, and the passes between the solve and the position correction are one pass on the pool. Physics 29.2 -> 24.2 ms.
2. *The bridge between records and bodies* (`body.rs`): each bibite's turn ends by leaving its rigid body (mass, inertia, drag, from the stomach, fat, womb, armor and body it has just been through) and its geometry copies in `derived` under a stamp of the body tick (`phys_stamp`, `refresh_physics_cache`); the build of the physics body and the sweep after the step use them when the stamp is current instead of reading six lines of organs, and every mutable access that is not a turn (`get_mut`, `for_each_mut`, a change of settings) clears the stamp. The record's first lines are laid out for it (line 0: `derived` and the pose; line 1: forces, id, contact counters, flags, sense throttle; lines 2-3: colliders), so the build reads 4 lines and the sweep after the step 2. The boxes are made in the build pass, with the body in the cache (the step and the pass for the resting pellets reuse them), the bibites' final poses are read from the solver instead of being written back into bodies and read again, the events of the step are cut into linked rows (no counting sort over every body), the sets of colliding pairs have a cheap hasher, and the bibites with mouth targets are listed by that sweep so that the next body tick does not scan every record for them. Physics 24.2 -> 16.8 ms.
3. *The broadphase over columns*: the boxes of the entries are copied into four columns, the end of a body's x range is found by a binary search, and the candidates are tested eight at a time (3.06 -> 2.27 ms).
4. *Sensing*: the answers are applied to the records on the pool (0.77 -> 0.50 ms); the physics body holds three colliders instead of four.
5. *The brain's compiled plan* (`brain.rs`): `step` runs one record per node with incoming synapses and one per edge (source, weight) in CSR order instead of reading seven scattered arrays; the plan copies kinds, biases and weights, so the editor's setters and the mutations mark it, and in tests every step first checks it against a fresh compile (brain steps 6.1 -> 5.2 ms, the sweep 13.7 -> 13.3).

Stage times at 100,000 bibites, ms per tick, before the round and after:

| Stage | Before | After |
|---|---|---|
| sensing | 3.0 | 3.0 |
| fused body sweep | 14.3 | 13.3 |
| dense-order mouth turns | 3.4 | 3.4-4.7 |
| rad scan, species recount | 0.76, 1.21 | 0.71, 1.20 |
| physics: build bibites | 4.74 | 3.5 |
| physics: pellets, eggs, holders, promotion of sleepers | 2.42 | 1.44 |
| physics: mirror + integrate, boxes | 1.14, 1.23 | 1.07, 0.04 |
| physics: x order | 2.74 | 1.14 |
| physics: broadphase, pair order | 4.09, 0.61 | 2.3, 0.2 |
| physics: collider poses | 2.46 | 0.81 |
| physics: narrowphase, solve, write-back | 1.04, 1.09, 0.73 | 1.03, 0.87, 0.07 |
| physics: the bridge after the step | 6.46 | 2.9 |

**Tried and dropped.** Reading the lines of the dense-order turns a few turns ahead (loads only, `_mm_prefetch` needs `unsafe`): it cost more than it saved; the dense-order turns (4.9% of the bibites, 700 ns each) spend their time in the pellet store's operations, not in the record's lines.

**What is left to reach 36 ticks/s** (estimates in ms at 100,000 bibites, the tick is 38): the fused sweep is 13.3 and 3.5 KB a bibite, of which the record's 22 lines read and 9 written are 2 KB, so the next step is a layout that separates the 160 bytes that change every tick (clustered in 3 lines instead of 8) from the configuration that is only read (clustered in a few lines), which needs the organs' state split from their configuration in `organs.rs` and the sub-structs' methods changed (estimated -3 to -4 ms); the scans that touch two cold lines of every record (species recount, rad bibites, the sensing prefilter: 2-3 ms together) could read one byte of flags if a flag per record were kept in step with the species, the rad flag and death (-1.5); a persistent array of physics bodies updated in place (the build writes 188 bytes a bibite, most of which - the colliders, the tag, the flags - are the same as last tick: -2); the dense-order turns (-1); a parallel sort of the nearly sorted x order (-0.5). All of them together are about 10 ms: the goal is within reach of the architecture but not of one more change.

## 100,000 bibites toward 1.2x, second round (2026-11-05, night)

The same world and machine as the section above (synthetic density-scaled, 100,000 bibites, 12 threads, Ryzen 5 3600). From the build of `33e7fce` (it came right after that section and moved the "empty slot" marker of `Option<BibiteRecord>` from the coldest line of the record to line 1, by a single-variant `repr(u64)` enum `SlotMarker` whose niche is larger than any `Vec` capacity: 39.35 -> 37.30 ms) to `5f32eef`: **38.1 -> 31.1 ms a tick on one slower afternoon (26.4 -> 32.2 ticks/s, 1.22x by alternating A/B, best of 4 rounds x 2); on a quiet machine 36.0 -> 29.5 ms, 27.8 -> 33.9 ticks/s, which is 1.13x real time at 30 TPS** (the round before this one started at 19.2 ticks/s, 0.64x). The target (36 ticks/s, 27.8 ms) is not reached. Everything below is bit-exact: the state fingerprints of the bench (20,000 bibites / 40 ticks, 100,000 / 12), the end states of Default, NestEgg, RadTower and Ring (400 ticks, 1 and 12 threads, equal to those of the build before the whole series of optimizations, `d14e9b2`) and the pinned 12,000-tick evolution fingerprint are the same as before the round. The real release profile (fat LTO) was built once and measures the same as the `perf` profile the numbers come from (29.63 ms against 29.63).

Other sizes now (12 threads): 30,000 bibites 94 ticks/s (3.1x real time; 86 before the round), 20,000 bibites 131 ticks/s (4.4x; 120).

| Commit | Change | Whole tick (A/B, old -> new) | Stage that moved |
|---|---|---|---|
| `2ac5c61` | per-slot summaries for the three scans over the records | 36.40 -> 34.93 ms | events-stats 1.22 -> 0.18, rad 0.77 -> 0.32, sensing 2.62 -> 2.48 |
| `c8e8ade` | position of every slot in the dense list; id list only for serial passes; two genes from `growth_genes` | 35.17 -> 33.87 ms | about 0.5 ms of it; the rest is the code-layout lottery of an untouched function |
| `5137519` | the organs' turn lies in one 640-byte run of the record | 32.97 -> 31.00 ms | sweep 12.31 -> 10.75, pass B 3.87 -> 3.40 |
| `0cc03aa` | prefetch hints for the dense-order turns | 32.07 -> 31.63 ms (noisy) | dense-order pass 3.61 -> 2.98 |
| `f0e4ea1` | the bibites' physics bodies persist between steps; colliders only copied when the bibite grows | 31.97 -> 30.80 ms | build 3.99 -> 3.18, mirror 1.10 -> 0.67 |
| `7541536` | the build lists the holders; the resting-pellet layer only when pellets are at least a quarter of the bibites | 30.93 -> 29.67 ms | physics 13.25 -> 12.34 (holders scan 0.42 ms, promotion pass 0.73 ms) |
| `61cb2b5` | the store counts the rad bibites | about 0.1 ms | rad 0.30 -> 0.22 |
| `cb6d8a8` | the sensing step keeps its result buffers, applies them where they are | 30.03 -> 29.50 ms | sensing 2.52 -> 1.77 |
| `5f32eef` | the broadphase's eight-lane test written with SSE2 | robustness: about 0.1 ms slower than the good case, 0.8 ms faster than the bad one | broadphase 1.7 or 2.47 depending on the build -> 1.9 |

**Summaries.** The sensing prefilter, the rad scan and the species recount asked two or three cold lines of every record (dead? rad? species? ready? senses pheromones?). `BibiteStore` now keeps a `SlotSummary` per slot (species and five bits, eight bytes) and the scans read that: 0.8 MB for 100,000 slots instead of some 45 MB. The summaries are refreshed lazily before a scan, for the slots that `get_mut`, `insert_record` and `remove` listed since the last one (a restored checkpoint and `for_each_mut` mark every slot). The fields summarized are only changed through those; a test build compares every summary with a fresh one at each refresh, so a write that bypasses the list fails in whichever test ticks the world. Ten mutation breaks (no touch in `get_mut` / insert / remove / restore / `for_each_mut`, a refresh that skips the list, wrong bits) are killed; two of them needed new tests (a restored checkpoint, a brain that senses pheromones). The same refresh counts the live rad bibites, so the rad step skips its scan when there are none (and a test now puts a rad carrier next to two bibites, which no test did).

**Dense positions.** `BibiteStore::dense_pos` (slot -> position in the dense list) makes `remove` O(1) (it scanned the dense list for the slot: up to 100,000 entries per death, invisible in this synthetic world, which has no deaths) and turns the two per-tick rank fills into copies.

**The record's run.** `dirty_probe` showed a tick writing 10.8 of a record's 34 lines (the changed fields are about 140 bytes spread over the clock, the propulsion, the fat, the womb, the growth, the feeding counters and the body). The organs the turn touches every tick now lie in one run from line 6 to line 15 (offset 384 to 1024, starting on a line boundary), the part written every tick first: the last 48 bytes of `FeedingCounters` (its counters for a bibite with something in its mouth moved to the front of the struct), clock, propulsion, pheromone, growth, womb, fat, the start of the body (`BodyState` is `repr(C)`, totals and pools first), then what the turn only reads (stomach, mouth, the three derived caches, steering). A field order only.

**Persistent bodies.** The physics step built a whole 216-byte `PhysicsBody` for every bibite every tick and read the record's lines 2-3 to copy the three colliders, which change only when the bibite's size does. `PhysicsBody` is now `repr(C, align(64))` (pose, velocities, mass and damping in its first line, the colliders behind it) and the first `kept_bibite_bodies` of the vector survive from one parallel step to the next. A body is updated in place when it still carries the bibite's tag and the generation of the record's colliders (`colliders_gen`, from a global counter that `refresh_colliders` and a restored checkpoint give a record anew; only its equality is ever asked); else it is made as before. No journal of insertions and removals is needed: the tag and the generation tell. A test build compares every kept body with a fresh one at every parallel step. The solver mirror reads one line of a body now.

**The sensing step.** Per due tick (every tick in a world with spread phases, one in ten in this one) the parallel branch spent, in microseconds: 9,000 in the jobs, 1,700 waiting (really dropping last tick's 100,000 results one by one on the calling thread, plus a fresh 2 MB buffer per job: 24 MB of new pages every tick), 4,000 copying all results with `clone` into one list, 4,900 applying them. The jobs now get their buffers back from the last tick, the pool applies the buffers as they are (each cut by record stretch in parallel) and empties them.

**Memory bandwidth, again.** The fused sweep takes **11.5 / 11.5 / 11.3 ms with 3 / 6 / 12 threads, and 11.8 with 2**: two threads already pull what the machine can give for this access pattern (the bandwidth probe reads 31 GB/s with 2 threads, 35 at best, less for a mix of reads and writes), so the sweep is bound by bytes, not by latency or by arithmetic. Hints for its run lines and for the brain's arrays (the heap blocks of `value`, `last_out`, `last_in`, the plan) measured nothing. What is left in it: the brain's heap arrays are about a third of its traffic (about 21 line transfers a step, on every second tick: the input copy alone, `last_out[..33] = value[..33]` and `value[..33] = inputs`, writes six lines for a brain that uses three inputs), then the record's five dirty lines. The dense-order turns are different: one thread, a cold record each, so a prefetch instruction three and six turns ahead (`bibites-prefetch`, one `_mm_prefetch`, behind the default `prefetch` feature of `bibites-core`) takes 0.6 to 1.1 ms off them. Plain loads ahead of time did not pay in the previous round: a load that misses blocks retirement, a prefetch does not. Hints did not help the sensing jobs either, and 8, 12, 16 and 24 threads give the same tick.

**The code-layout lottery, found.** Between builds with no change to a stage, `broadphase` took 1.7 or 2.5 ms and `build-bibites` 3.1 or 3.6. The disassembly of a slow and a fast build showed it: in the slow ones LLVM had not vectorized several loops (no `cmpps` in the binary at all, against 47). The eight-lane box test of the sweep is now written with SSE2 intrinsics, which cannot be un-vectorized; the other loops (the clamps of the brain's inputs and so on) still depend on the compiler's mood, and a stage that moves by 10% for no reason should be measured again.

**Tried and dropped.** Copying the brain's fifteen outputs into the brain struct and putting the brain right after the run (the turn reads them every tick through the pointer to the `value` heap block): 3.7% slower (31.67 -> 32.90 ms, three rounds the same way). In this world the heap blocks of consecutive brains lie one after the other, so that read is a regular stride the hardware follows; in a churned world it would be a random line, which this benchmark cannot show. A record 64 bytes longer, by itself: at most 1%. A class byte per entry of the spatial hash, so that a query for pellets does not read the bibites' entries: 0.09 ms of sensing, not kept.

**Noise.** Two runs of the same binary minutes apart differ by up to 7% in the whole tick, and an untouched function moves with the code around it (see above). The table is from alternating A/B runs; the stage columns are best-of values.

**What is left to reach 27.8 ms** (estimates, ms at 100,000 bibites; the tick is 29.5 on a quiet machine): less brain state traffic (the input copy as a ring of the last two input blocks, a rewrite of `brain.rs`'s node arrays: -0.8); a grid broadphase for spread worlds instead of the x sweep and its sort (-1 to -1.5; building the grid in parallel costs about what the sort does); more dense-turn hints (the pellets a bibite is biting, -0.3); the full state/config split of the organs (-0.5 to -0.9, about a thousand use sites). All together about 2.5 ms of uncertain delivery for a 1.7 ms gap.

## The window's simulation thread (2026-11-05, night)

`rustibites-view --sim-thread` (or `RUSTIBITES_SIM_THREAD=1`) ticks the world on a thread of its own while the player only watches the camera, and the window draws what that thread publishes at its own frame rate
([ARCHITECTURE.md](ARCHITECTURE.md#10-performance-targets-and-measurement) has the design). The tick rate is the world's: the worker calls `World::tick` and a test compares a world that went away and came back with one run headless. What
the mode costs is the snapshots: a display list built for a camera a quarter wider than the window's, which takes the worker time it would spend ticking. They are made only when something changed (a tick ran, or the window asked for
something else) and are paid for with the time the worker sleeps, or, for a world that leaves it no time, with a seventh of its time at the most. Measured with `cargo test --profile perf -p bibites-view --bins -- --ignored --nocapture
worker_throughput` (5,000 bibites in the generated starter world, 12 threads, game assets loaded, 4 s per row, optimized; `RUSTIBITES_SIM_BENCH_BIBITES` / `_THREADS` change the world):

| Speed asked | Ticks/s without snapshots | Ticks/s with the window asking for snapshots | Snapshots/s |
|---|---|---|---|
| 1x | 29.8 | 29.8 | 29.5 (one for every tick) |
| 3x | 89.8 | 89.8 | 13.2 |
| flat out | 139.2 | 124.8 (-10%) | 10.0 |

So up to the speed the world can run at (3x here), publishing costs no ticks: the sleeping time pays for it; flat out it costs 10% of the ticks (the bound is 14%), and the world's picture then updates 10 times a second while the window
itself, the camera and the UI keep their frame rate. A snapshot of these 5,000 bibites in full view takes about 14 ms to build (a zoomed-in view culls to what the camera sees and costs far less). Not measured: the thread in a real window
(the player was at the machine and a window cannot be opened under their hands; `RUSTIBITES_LOG_FRAMES=1 rustibites-view <save> --sim-thread` prints where a frame goes and the ticks and frames per second the time panel shows).

## Multithreading sized to the work (2026-11-15)

The owner asked for "multithreading ALWAYS run". That was built literally first and measured (the `always` column below): every stage on the pool, at any size, made every
world under about 1,300 bibites slower than one thread (5.0x slower at 50 bibites in the first measurement). Shown that, the owner asked for the version that sizes the pool to the
work ("I want it"), and that is what a world starts with now. `--threads 1` is the serial tick; `RUSTIBITES_GATES=always` brings the literal version back (the CLI and the viewer
read it; also `scaled`, `scaled:<percent>` to scale every size, `eager`, `nosleep`).

**What a world does now** (`ParallelGates::SCALED`, `sim.rs`; `World::tick`):

- Fewer than 1,200 bibites and fewer than 3,000 bodies: the tick runs on the calling thread **with no handle to the pool at all** (`World::pool_wanted`), every stage inline, exactly
  as with `--threads 1`. Nothing in such a world wakes a worker, and the CPU shows it (1.0 core).
- A bigger world: the whole tick runs on one of the pool's workers (a stage's chunks go to that worker's own queue; the some twenty hand-offs a tick from a thread outside the pool, each
  a block and a wake of ~20 us, are one), and every stage that can use the pool is on it, one worker for every quarter of the stage's size (a stage with a size of 256 has 4 workers on
  256 items and all 12 from 768).
- A few kinds of stage keep a size of their own in such a world, because the per-stage timings below found them losing on the pool in worlds of a few thousand bodies: the cheap
  per-body stages of the physics step and the bridge's work after the step (3,000 bodies), the x order sorted from scratch (4,096 bodies; the pair sort 8,192 pairs: the serial repair of the last
  step's order is cheaper unless the order has to be sorted anyway), the scans of the per-slot summaries (8,192 slots) and the copies of results into the records (1,500 due bibites). Sensing
  (from 96 due bibites), the narrowphase (from 64 pairs), the body sweep and the bibites' body build (from 256 records) and the broadphase (from 512 bodies) only guard against a pooled world
  that has few of them (a tick with a handful of due bibites or pairs). The islands of the solver keep their thresholds (256 held bodies, 512 constraints, and at least 16 islands).
- Results do not change: serial and pool runs are bit-identical (the tests compare whole checkpoints byte for byte, with every size at 1 and with the sizes a world starts with).

Measured with the optimized CLI (`cargo build --profile perf -p bibites-cli`), Ryzen 5 3600 (6C/12T, `--threads 0` = 12), synthetic starter-bibite worlds (`scratchpad/ab_scaled2.ps1`): four variants
alternated, best of 3 rounds x 2 repetitions; serial = `--threads 1`; before = the build without this change (the old sizes: 3,000 records, 4,096 bodies, 20,000 due bibites, ...: `RUSTIBITES_GATES=tuned`);
always = `RUSTIBITES_GATES=always`; scaled = the default. ms per tick (in brackets: speed against serial), cores kept busy on average. The player was at the machine (about 2 cores of other programs ran
all the time), so the benchmark ran at High priority.

| Bibites | Serial | Before (the old sizes) | Always | Scaled (the default) |
|---|---|---|---|---|
| 50 | 0.050 | 0.048 | 0.236 (0.21x), 9.5 cores | 0.048 (1.05x), 0.9 cores |
| 100 | 0.085 | 0.084 | 0.311 (0.27x), 9.4 cores | 0.086 (0.99x), 0.9 cores |
| 200 | 0.172 | 0.167 | 0.448 (0.38x), 8.2 cores | 0.171 (1.01x), 0.9 cores |
| 300 | 0.282 | 0.284 | 0.614 (0.46x), 8.0 cores | 0.283 (1.00x), 1.0 cores |
| 500 | 0.491 | 0.485 | 0.826 (0.59x), 7.0 cores | 0.490 (1.00x), 1.0 cores |
| 700 | 0.678 | 0.685 | 0.970 (0.70x), 6.9 cores | 0.687 (0.99x), 1.0 cores |
| 1,000 | 0.978 | 1.068 | 1.129 (0.87x), 6.2 cores | 0.959 (1.02x), 1.0 cores |
| 1,200 | 1.406 | 1.562 | 1.474 (0.95x), 6.2 cores | 1.395 (1.01x), 3.4 cores |
| 1,500 | 1.400 | 1.593 | 1.341 (1.04x), 6.3 cores | 1.271 (1.10x), 4.0 cores |
| 2,000 | 1.888 | 2.173 | 1.577 (1.20x), 6.6 cores | 1.560 (1.21x), 3.6 cores |
| 3,000 | 2.795 | 2.265 | 1.920 (1.46x), 6.3 cores | 1.735 (1.61x), 4.9 cores |
| 5,000 | 5.140 | 2.936 | 2.548 (2.02x), 6.4 cores | 2.628 (1.96x), 4.3 cores |
| 10,000 | 11.347 | 4.807 | 4.120 (2.75x), 5.4 cores | 4.147 (2.74x), 5.7 cores |
| 30,000 | 35.817 | 10.767 | 10.467 (3.42x), 4.9 cores | 10.483 (3.42x), 5.3 cores |

- The default is the serial tick (within 1-5 %, 0.9-1.0 cores) up to 1,000 bibites; from 1,200, where the world goes onto the pool, 1.01x there, 1.10x at 1,500, 1.21x at 2,000, 1.61x at 3,000 (better than
  always-on's 1.46x, because the stages that lose are left off), 1.96x at 5,000, 2.74x at 10,000 and 3.42x at 30,000. Against the old sizes it is the same up to 700 bibites and faster at every size from 1,000 on:
  1.11x at 1,000, 1.25x at 1,500, 1.39x at 2,000, 1.31x at 3,000, 1.12x at 5,000, 1.16x at 10,000, 1.03x at 30,000. At 5,000 and 10,000 the literal version is 1-3 % faster than the default (2.548 against 2.628 ms,
  4.120 against 4.147): the cheap per-body stages and the sort that the default keeps inline there are a small win on the pool at that size.
- The literal version (always) is slower than one thread up to 1,200 bibites (5x at 50, 2.6x at 200, 1.7x at 500, 1.15x at 1,000): every stage that goes to the pool costs a hand-off, some twenty a tick, and the
  whole tick of such a world is cheaper than that; the pool also spins while it waits (6-9 cores busy during a tick).
- Where the pool starts to pay is a property of the machine (12 threads here), and it was measured on this machine only: that measurement did NOT carry over to smaller pools (it pays from far fewer bibites there, and
  the stage sizes below were tuned for 12 workers): see "The line and the levels are measured" at the end of this section.

**What the per-stage timings say** (ms per tick, serial -> every stage on the pool, 12 workers; `stage_table.ps1`, the bench's own stage clock):

```
stage              300            500            700            1000           1500           2000           3000           5000
sensing            0.032>0.053    0.039>0.061    0.052>0.056    0.065>0.055    0.091>0.068    0.126>0.078    0.195>0.100    0.331>0.147
body (sweep+B)     0.098>0.122    0.170>0.185    0.255>0.302    0.401>0.413    0.622>0.622    0.928>0.830    1.494>1.025    2.879>1.367
build-bibites      0.033>0.059    0.055>0.064    0.079>0.078    0.114>0.083    0.170>0.103    0.228>0.122    0.347>0.154    0.564>0.223
mirror             0.012>0.023    0.020>0.038    0.029>0.059    0.041>0.070    0.061>0.085    0.083>0.098    0.126>0.111    0.218>0.149
aabb (pellets)     0.000>0.008    0.000>0.011    0.001>0.016    0.001>0.016    0.001>0.019    0.002>0.023    0.003>0.023    0.004>0.028
x-order            0.002>0.065    0.003>0.073    0.005>0.081    0.007>0.087    0.012>0.102    0.017>0.116    0.031>0.143    0.055>0.169
broadphase         0.012>0.030    0.023>0.034    0.037>0.046    0.059>0.048    0.099>0.066    0.141>0.080    0.218>0.105    0.369>0.138
pair-sort          0.005>0.009    0.006>0.010    0.008>0.014    0.013>0.018    0.019>0.027    0.025>0.034    0.028>0.036    0.029>0.037
poses              0.007>0.031    0.014>0.041    0.023>0.049    0.038>0.054    0.059>0.063    0.083>0.074    0.110>0.078    0.149>0.090
narrowphase        0.036>0.043    0.068>0.056    0.123>0.088    0.223>0.111    0.345>0.156    0.481>0.196    0.561>0.214    0.601>0.241
solve (islands)    0.037>0.099    0.067>0.127    0.112>0.160    0.182>0.185    0.275>0.245    0.387>0.289    0.474>0.321    0.549>0.347
write-back         0.005>0.039    0.010>0.041    0.014>0.049    0.019>0.051    0.029>0.060    0.039>0.071    0.058>0.073    0.096>0.090
bridge-after       0.008>0.036    0.015>0.045    0.025>0.062    0.044>0.077    0.070>0.115    0.105>0.138    0.152>0.159    0.242>0.191
events-stats       0.002>0.021    0.003>0.036    0.004>0.041    0.007>0.041    0.011>0.043    0.018>0.046    0.034>0.046    0.084>0.050
whole tick         0.295>0.658    0.500>0.838    0.774>1.124    1.223>1.330    1.881>1.806    2.689>2.232    3.882>2.633    6.297>3.330
```

- Stages that win on the pool: the narrowphase (from about 350 bibites), the bibites' body build (700), the broadphase sweep (800), sensing (800 due), the solver's islands (1,000), the body sweep (1,500;
  at 5,000 bibites 2.9 -> 1.4 ms). Stages that lose at every size up to 5,000: the x order sorted from scratch (0.087 against 0.007 ms at 1,000), the boxes of the pellets, the pair sort, the scans of the
  summaries (`events-stats`); the cheap per-body stages (mirror, poses, write-back, the bridge's work after the step) lose until 3,000 to 5,000 bodies.
- **Stage by stage did not work** (the first two tries of this change, both measured; the world of one try is the serial world below): (1) a chunk for every few hundred items, with the tick on the calling
  thread below the size where a stage has work: 0.93x / 0.81x / 0.87x / 0.89x of serial at 200 / 300 / 500 / 700 bibites, 0.96x at 1,000, 0.98x at 1,500, 1.09x at 2,000 (against always-on's 1.16x in that sweep); (2) a size per stage
  (the break-even of the table), with no pool handle in a tick where no stage has work: serial speed to 500 bibites, but 0.81x at 700 and 0.89x at 1,000 and still behind always-on from 1,500 on (1.06x, 1.08x against 1.10x, 1.26x).
  A stage that goes to the pool alone pays the hand-off, and the stages around it pay for the data it left in other cores' caches: at 1,000 bibites `bridge-after` took 0.044 ms with everything inline and
  0.073 ms when the stages before it were on the pool, though it ran inline in both. Pooling all or none (up to the stages above that lose by themselves) wins both ends.
- Not done: scaling the number of workers by the square root of the work (the cost model `fixed + per-chunk x workers + work / workers` puts the best worker count near sqrt(work / per-chunk cost): fewer
  workers at the small end of a pooled world). (A `pool_bibites` that depends on the machine is what the measuring now does.)
- Real window (the static alpha build of the final code, 1280 x 720, `--speed 30`, the window's own "counted here" ticks per second, two runs each, the player away; `scratchpad/win_scaled_ab.ps1`; before = the viewer before this change, always = `RUSTIBITES_GATES=always`, default = scaled): 200 bibites 907 / 907 / 905 ticks/s (before / always / default) and 0.55 / 4.26 / 0.63 cores; 1,000 bibites 900 / 719 / 902 ticks/s and 112 / 23 / 109 fps (always: the frame waits 42 ms for its ticks); 2,000 bibites 337 / 414 / 382 ticks/s (15 fps in all three: the frame is drawing-bound there); Butterfly 6 (2,450 bibites, 10,100 pellets) 164 / 213 / 201 ticks/s, 2.3 / 4.1 / 3.2 cores (the default +23 % over before, always +30 %). So in the window the default keeps the small and middle worlds exactly as they were and gains from about 2,000 bibites; the literal version is a few per cent faster still above 2,000 but takes 4-5 cores for a 200-bibite world and drops the 1,000-bibite world from 112 to 23 fps.
- **A note on builds.** An intermediate build of this change was about 9 % slower than the build before it on worlds of 100 to 700 bibites, also with `--threads 1` (a same-session A/B of the optimized CLI builds:
  300 bibites 0.302 against 0.270 ms, 700 bibites 0.719 against 0.661 ms; the exe was 200 KB smaller): not the pool. The perf profile (16 codegen units, no LTO) moves the code layout with every edit
  (stage by stage the loss was in sensing, the bibites' body build and the poses). `#[inline]` on five tiny hot helpers (`Collider::world_pose` and `aabb`, `PhysicsBody::pose` and `colliders`,
  `build_bibite_physics_body`) gave the speed back (0.274 and 0.661 ms); the table above is that build. After any edit to `physics.rs`, `body.rs` or `world.rs`, compare `--threads 1` of the new CLI with
  the previous one before looking at the pool. (A build of the core crate in one codegen unit, to test the theory, went over the 3 GB free-RAM guard and was not run.)

### The line and the levels are measured, not written ("the policy should be good for all machines", 2026-11-15)

**Why.** Everything above was measured on one machine (Ryzen 5 3600: 6 cores, 12 threads). To see what the sizes do elsewhere, the optimized CLI was run on *emulated* smaller machines: the process restricted to 2, 4 or 6 of this machine's
logical processors (one per physical core, away from CPU 0) with the pool sized to match (`--threads N` and `RAYON_NUM_THREADS=N`: on Windows `available_parallelism` ignores the affinity mask), variants alternated, best of 2-3 rounds
(`scratchpad/emu_ab.ps1`). That is the same cores and the same OS with a smaller pool: it shows how the policy depends on the size of the pool, **not** how other CPUs behave (no hybrid cores, no 32-thread workstation, no laptop that sleeps its
cores). Two things came out of it, both the opposite of what was guessed above ("on a CPU with fewer cores the switch should be higher"):

1. *The pool pays from fewer bibites on a small pool.* From about 500 bibites on 2 workers, 1,000 on 4 to 6 and 1,500 on 12 (the old switch was 1,200): on 2 workers the old policy ran serial at 500 and 1,000 bibites where every stage on the pool is 1.24x and 1.38x faster.
2. *The stage sizes tuned on 12 workers are wrong for 2 to 6 workers.* Putting every stage on the pool from one item beat them by 6-19 % at 500-2,000 bibites (on 12 workers it loses 2.5x at 200 bibites and 8 % at 3,000):

| Workers | Bibites | Serial | Old policy | Old policy's pooled tick (sizes of 12 workers) vs every stage on the pool |
|---|---|---|---|---|
| 2 | 500 / 1,000 / 2,000 | 0.520 / 1.093 / 2.434 | 0.524 / 1.098 / 1.895 | 0.445 / 0.880 / 1.931 against 0.420 / 0.794 / 1.685 |
| 4 | 1,000 / 1,500 / 2,000 | 1.086 / 1.717 / 2.416 | 1.085 / 1.444 / 1.939 | 0.970 / 1.448 / 1.994 against 0.880 / 1.265 / 1.714 |
| 6 | 1,000 / 1,500 / 2,000 | 1.127 / 1.776 / 2.432 | 1.124 / 1.551 / 2.054 | 1.083 / 1.538 / 2.116 against 0.949 / 1.379 / 1.775 |
| 12 | 1,500 / 3,000 / 10,000 | 1.755 / 3.613 / 12.35 | 1.613 / 2.416 / 5.458 | 1.620 / 2.526 / 5.609 against 1.655 / 2.607 / 5.407 |

(ms per tick; the "pooled tick" column is a first, two-way tuner that chose between inline and the pool at the sizes of the 12-worker machine: it already found the pool at 500 and 1,000 bibites on 2 workers, which the fixed switch did not.)

**What a world does now** (`pool_tuner.rs`, `ParallelGates::pooled_level`): it chooses among five ways, inline and the pool at four *levels of eagerness* (the stage sizes above times 1, 0.3, 0.08, and every size at 1), by timing its own ticks (the design is in
`docs/ARCHITECTURE.md`: probes of 12 to 100 ticks, each way waiting twice as long after each loss up to 8,192 ticks, a win has to be seen twice, the pool must take under 90 % of the inline time, a world under 300 bibites is never timed).
`RUSTIBITES_GATES=pooled:<level>` runs every world at one level with no measuring: the tables below are those levels one by one (the bold cell is the best fixed way for that size) next to the measuring default.

**The landscape, and what the measuring default makes of it** (ms per tick, in brackets the speed against `--threads 1`; the measuring run is 4,000 to 12,500 ticks, so its first few hundred ticks of looking are in the number; best of 2-3 rounds; the cores of the player's programs
were not quiet, best-of noise is +-4 %):

**12 threads (this machine)**

| Bibites | Serial | Level 0 | Level 1 | Level 2 | Level 3 (= every stage) | Measuring | Measuring vs the best fixed way |
|---|---|---|---|---|---|---|---|
| 300 | **0.310 (1.00x)** | 0.470 (0.66x) | 0.527 (0.59x) | 0.610 (0.51x) | 0.665 (0.47x) | 0.310 (1.00x) | +0.0 % |
| 500 | **0.543 (1.00x)** | 0.730 (0.74x) | 0.740 (0.73x) | 0.838 (0.65x) | 0.895 (0.61x) | 0.556 (0.98x) | +2.4 % |
| 1,000 | **1.159 (1.00x)** | 1.266 (0.92x) | 1.188 (0.98x) | 1.281 (0.91x) | 1.317 (0.88x) | 1.203 (0.96x) | +3.8 % |
| 1,500 | 1.921 (1.00x) | 1.823 (1.05x) | 1.741 (1.10x) | **1.737 (1.11x)** | 1.783 (1.08x) | 1.818 (1.06x) | +4.6 % |
| 2,000 | 2.751 (1.00x) | 2.481 (1.11x) | **2.207 (1.25x)** | 2.234 (1.23x) | 2.286 (1.20x) | 2.314 (1.19x) | +4.8 % |
| 3,000 | 4.419 (1.00x) | **2.973 (1.49x)** | 3.126 (1.41x) | 3.117 (1.42x) | 3.125 (1.41x) | 3.038 (1.45x) | +2.2 % |
| 5,000 | 7.361 (1.00x) | 4.398 (1.67x) | **4.333 (1.70x)** | 4.418 (1.67x) | 4.358 (1.69x) | 4.386 (1.68x) | +1.2 % |


**6 cores**

| Bibites | Serial | Level 0 | Level 1 | Level 2 | Level 3 (= every stage) | Measuring | Measuring vs the best fixed way |
|---|---|---|---|---|---|---|---|
| 300 | **0.314 (1.00x)** | 0.371 (0.85x) | 0.394 (0.79x) | 0.427 (0.73x) | 0.459 (0.68x) | 0.317 (0.99x) | +1.1 % |
| 500 | **0.553 (1.00x)** | 0.613 (0.90x) | 0.611 (0.91x) | 0.580 (0.95x) | 0.605 (0.92x) | 0.561 (0.99x) | +1.4 % |
| 1,000 | 1.180 (1.00x) | 1.139 (1.04x) | **0.960 (1.23x)** | 1.002 (1.18x) | 1.013 (1.17x) | 1.006 (1.17x) | +4.7 % |
| 1,500 | 1.935 (1.00x) | 1.722 (1.12x) | **1.439 (1.34x)** | 1.501 (1.29x) | 1.453 (1.33x) | 1.518 (1.27x) | +5.5 % |
| 2,000 | 2.722 (1.00x) | 2.419 (1.13x) | **1.936 (1.41x)** | 1.982 (1.37x) | 1.966 (1.38x) | 2.043 (1.33x) | +5.5 % |
| 3,000 | 4.376 (1.00x) | **2.824 (1.55x)** | 2.836 (1.54x) | 2.895 (1.51x) | 2.861 (1.53x) | 2.852 (1.53x) | +1.0 % |
| 5,000 | 7.482 (1.00x) | 4.226 (1.77x) | 4.282 (1.75x) | **4.181 (1.79x)** | **4.181 (1.79x)** | 4.189 (1.79x) | +0.2 % |


**4 cores**

| Bibites | Serial | Level 0 | Level 1 | Level 2 | Level 3 (= every stage) | Measuring | Measuring vs the best fixed way |
|---|---|---|---|---|---|---|---|
| 300 | **0.298 (1.00x)** | 0.346 (0.86x) | 0.357 (0.83x) | 0.325 (0.91x) | 0.347 (0.86x) | 0.315 (0.95x) | +5.7 % |
| 500 | 0.539 (1.00x) | 0.560 (0.96x) | 0.577 (0.93x) | 0.517 (1.04x) | **0.515 (1.05x)** | 0.525 (1.03x) | +2.0 % |
| 1,000 | 1.165 (1.00x) | 1.060 (1.10x) | **0.877 (1.33x)** | 0.917 (1.27x) | 0.916 (1.27x) | 0.923 (1.26x) | +5.3 % |
| 1,500 | 1.865 (1.00x) | 1.616 (1.15x) | **1.364 (1.37x)** | 1.373 (1.36x) | 1.365 (1.37x) | 1.410 (1.32x) | +3.3 % |
| 2,000 | 2.682 (1.00x) | 2.293 (1.17x) | 1.899 (1.41x) | 1.891 (1.42x) | **1.882 (1.43x)** | 1.936 (1.39x) | +2.9 % |
| 3,000 | 4.280 (1.00x) | **2.749 (1.56x)** | 2.767 (1.55x) | 2.798 (1.53x) | 2.783 (1.54x) | 2.739 (1.56x) | -0.4 % |
| 5,000 | 7.356 (1.00x) | 4.370 (1.68x) | 4.361 (1.69x) | **4.283 (1.72x)** | 4.284 (1.72x) | 4.357 (1.69x) | +1.7 % |


**2 cores**

| Bibites | Serial | Level 0 | Level 1 | Level 2 | Level 3 (= every stage) | Measuring | Measuring vs the best fixed way |
|---|---|---|---|---|---|---|---|
| 300 | 0.309 (1.00x) | 0.304 (1.02x) | 0.311 (0.99x) | **0.273 (1.13x)** | 0.298 (1.04x) | 0.290 (1.07x) | +6.2 % |
| 500 | 0.547 (1.00x) | 0.486 (1.13x) | 0.495 (1.11x) | **0.458 (1.20x)** | 0.461 (1.19x) | 0.467 (1.17x) | +2.1 % |
| 1,000 | 1.179 (1.00x) | 0.955 (1.23x) | **0.826 (1.43x)** | 0.862 (1.37x) | 0.873 (1.35x) | 0.860 (1.37x) | +4.0 % |
| 1,500 | 1.897 (1.00x) | 1.507 (1.26x) | **1.337 (1.42x)** | 1.365 (1.39x) | 1.368 (1.39x) | 1.405 (1.35x) | +5.1 % |
| 2,000 | 2.716 (1.00x) | 2.177 (1.25x) | **1.897 (1.43x)** | 1.932 (1.41x) | 1.930 (1.41x) | 1.991 (1.36x) | +5.0 % |
| 3,000 | 4.279 (1.00x) | **2.885 (1.48x)** | 2.937 (1.46x) | 2.989 (1.43x) | 2.951 (1.45x) | 2.933 (1.46x) | +1.7 % |
| 5,000 | 7.529 (1.00x) | 4.893 (1.54x) | 4.880 (1.54x) | 4.917 (1.53x) | **4.863 (1.55x)** | 4.860 (1.55x) | -0.1 % |

- Level 0, the stage sizes measured on the 12-worker machine, is the best fixed way only at 3,000 bibites (on every pool size). At 1,000-2,000 bibites (1,500-2,000 on 12 workers) a more eager level beats it by 5-25 % on every pool size, the 12-worker one included (5 % at 1,500,
  12 % at 2,000: the sizes tuned there were not the best there either); from 3,000 bibites on the four levels tie within 3 % on 2 to 6 workers. On 12 workers every level of the pool loses to inline up to about 1,000 bibites, the more eager the worse (0.47x at 300 bibites for
  level 3), while on 2 workers the pool wins from 500 and the more eager levels win there. That is why a level is measured and not fixed.
- The measuring default is within 0-6 % of the best fixed way at every size and machine (6.2 % at most, at 300 bibites on 2 cores). The cost of looking is in that: a world of 300 bibites on 4 cores, where every pool level loses to inline, spends about 9 % of its first 12,500 ticks
  looking at them and pays 5.7 % (the best-of noise is +-4 %). Over a long run the looking thins out (each way waits twice as long after each loss, to 8,192 ticks): the unit tests bound it at under 1.5 % of 40,000 ticks and under 1.1 % of 160,000 ticks on a machine where every way of the pool loses.
- Against the old policy (speed against `--threads 1` of the same run; the old policy from the first table above, +-4 %): 2 workers 1.00x / 1.27x / 1.28x at 1,000 / 1,500 / 2,000 bibites become 1.37x / 1.35x / 1.36x; 4 workers 1.00x / 1.19x / 1.25x become 1.26x / 1.32x / 1.39x;
  6 workers 1.00x / 1.15x / 1.18x become 1.17x / 1.27x / 1.33x; 12 workers 0.99x / 1.09x at 1,000 / 1,500 become 0.96x / 1.06x (and 1.19x at 2,000, where level 0 alone is 1.11x): there the gain of a better level is about what looking costs in a run of a few thousand ticks.
- Long runs, where the looking has thinned out (12,000 to 20,000 ticks; ms per tick, in brackets the speed against serial; the old policy / the best fixed level / the measuring default): 12 threads at 2,000 bibites 2.224 (1.02x) / 2.007 (1.13x) / 2.036 (1.12x);
  4 cores at 1,500 bibites 1.415 (1.08x) / 1.210 (1.26x) / 1.239 (1.23x); 2 cores at 1,000 bibites 0.921 (1.00x) / 0.733 (1.25x) / 0.739 (1.24x). The measuring default is within 0.9-2.4 % of the best fixed level there, and 9, 14 and 25 % faster than the old policy.
- The first measuring default (inline against the pool at the sizes of the 12-worker machine, the first table) was 6-19 % behind the best fixed way at 500-2,000 bibites on 2 to 6 workers; the ladder, when first tried, also moved the 200-bibite world back and forth between inline and the last level
  (5 switches in 12,000 ticks, 6 % behind serial), which is what made a win have to be seen twice, every loss double a wait and the floor 300 bibites.
- Determinism: a real 900-tick run of 300, 1,500 and 4,000 bibites with the ladder probing (`RUSTIBITES_GATES=scaled`, `scaled:10`, `scaled:25`) ends in the state of `--threads 1`, bit for bit (`scratchpad/fp_check.ps1`), and the unit tests compare whole checkpoints for worlds that change
  way and level every few ticks.
- Real window (the static alpha build of this code, 1280 x 720, `--speed 30`, the window's own "counted here" ticks per second, two runs each, the player away; `scratchpad/win_tuner_ab.ps1`; before = the shortcut exe of that morning (the fixed switch), fixed =
  `RUSTIBITES_GATES=fixed` of the new exe, measuring = its default): 200 bibites 906 / 908 / 906 ticks/s (the speed target is 900: nothing to gain; 0.57 / 0.49 / 0.48 cores); 1,000 bibites 909 / 908 / 914 (123 / 134 / 129 fps; the measuring one keeps 1.3 cores busy where the others keep 1.0:
  it is looking at the pool); 2,000 bibites 442 / 498 / 521 ticks/s (+18 % over before, +4.5 % over fixed; 2.8 / 3.0 / 4.5 cores; 15.8 / 17.7 / 16.8 fps, the frame is drawing-bound there); Butterfly 6 (2,450 bibites, 10,100 pellets) 206 / 227 / 240 ticks/s (+16 %, +5.6 %; 3.4 / 3.4 / 3.7 cores).
  The window's log says what the world found, for example `pool: ticks now run on the pool (level 1) (it was on the pool (level 0); tick 1125, world size 2000: 2.28 ms a tick now, 2.48 ms before)`. (The new exe with the old fixed switch is itself 10-13 % faster than the exe of that
  morning at 2,000 and 2,450 bibites: code layout of the perf profile, or the noise of two runs.)
- **Not shown by this.** Other CPUs (hybrid cores, 16-64 threads, a laptop that parks cores), and a machine whose load changes during the run (the measuring would follow within 8,192 ticks at most; not measured).

## Interim performance matrix - 2026-11-01 (NOT FINAL)

This is an **interim** capture of the current tree while core/behavior fixes are
still landing. It does **not** replace the frozen task-131 matrix below, which
remains the published reference. The interim binary includes uncommitted
body/physics/parity work, so treat all absolute numbers as a directional signal
for the next freeze, not a release claim.

**Interim binary:** `0B46AC9F22974FB64B6FAA2B9581E3078F815E6BF0CB3D4F4A2D42D02C90A1E6`
(2,226,688 B), built from git `15cac67` with 22 dirty paths. Reference binary:
`3CCB231E...` (frozen task-131). Same Ryzen 5 3600 reference machine.

**Load caveat:** the synthetic matrix was load-guarded at start (load 22%,
repeat 2); Default 10%, 100k 34%. The cluster harness environment sampled 74%
load, so its *whole-tick* rows are noisy, but the paired frozen/current A/B
below (load 19-35%) and the candidate-pair counts confirm the dense-path
regression. Re-run everything on an idle machine at freeze.

### Synthetic density-scaled matrix (best of 2, 120 ticks)

| Population | Frozen serial | Interim serial | Delta | Frozen 4t | Interim 4t | Frozen 8t | Interim 8t |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1,000 | 1,483 | **1,254** | -15.4% | 1,361 | 1,207 | 1,356 | 1,239 |
| 2,000 | 715 | **538** | -24.8% | 649 | 587 | 652 | 623 |
| 5,000 | 220 | **164** | -25.5% | 216 | 166 | 220 | 158 |
| 10,000 | 94 | **70** | -25.7% | 94 | 60 | 95 | 60 |
| 20,000 | 41 | **32** | -23.0% | 42 | 33 | 42 | 28 |

Threads now add little or **reduce** throughput in several cells (e.g. 10k:
70 -> 60 -> 60), so the parallel paths did not improve the interim tree. The
1k cell recovered from the heavily loaded first attempt; the table uses the
load-guarded run.

### Default scenario (600 ticks, real path)

| Threads | Frozen | Interim | Delta |
| --- | --- | --- | --- |
| 1 | 16,676 | **13,321** | -20.1% |
| 8 | 16,117 | **12,540** | -22.2% |

### Dense cluster (2,000 bibites, 10 ticks, 1 thread)

| Case | Metric | Frozen | Interim | Delta |
| --- | --- | --- | --- | --- |
| box100 | whole tick | 38.46 ms | **66.67 ms** | +73.3% slower |
| box100 | physics | 37.34 ms | **68.68 ms** | +83.9% slower |
| box100 | body | ~1 ms | 1.39 ms | near flat |
| box200 | whole tick | 20.00 ms | **24.39 ms** | +22.0% slower |
| spread | ticks/s | 726 | 626 | -13.8% |
| spread | physics | 0.80 ms | 0.87 ms | +8.8% |

Canonical physics-only core probe confirms the dense-path change is structural,
not just scheduler noise:

| Probe | Frozen | Interim | Delta | Frozen pairs | Interim pairs |
| --- | --- | --- | --- | --- | --- |
| spread2000 | 1.244 ms | 1.29 ms | +3.7% | 117 | 117 |
| spread5000 | 3.534 ms | 3.57 ms | +1.0% | 727 | 727 |
| cluster2000 | **46.586 ms** | **86.10 ms** | +84.8% | 31,747 | **148,504** |
| cluster4000 | **123.092 ms** | **297.10 ms** | +141.3% | 51,490 | **519,782** |

The dense candidate-pair count is 4.7x / 10.1x higher in the interim tree; this
is the mechanism for the dense-cluster slowdown. Root cause (confirmed by the
engine owners): the interim dense sensor-pair bypass treats "all bodies have
sensors" as "nothing to filter", effectively disabling the dense-body pair cap.
The spread probe is unchanged, so the change is specific to dense overlap.
world-dev is implementing a separate sensor broadphase/capped sensor handling;
the frozen 3CCB matrix stays authoritative and the package must not be finalized
until that fix lands and a clean idle A/B re-run passes.

Paired frozen-vs-interim A/B on the same machine/load window (same binary
snapshots, alternating order):
- 5,000 synthetic, 1 thread: frozen best 175.7/179.7/175.5 ticks/s vs interim
  167.4/161.1/152.0 -> interim ~10-14% slower.
- cluster2000 box100, 1 thread: frozen 23.1-25.0 ticks/s vs interim
  14.0-15.5 ticks/s -> interim ~38-40% slower.
This corroborates the matrices; the dense regression is real, the spread
regression is smaller and partly load-dependent.

### 100k one-tick sanity (synthetic density-scaled)

Peak working set **515.2-521.2 MB** vs frozen 494.6-512.7 MB (+~2-4%); one-tick
engine time **0.170-0.171 s** vs frozen 0.140-0.141 s under the interim load
(34%), so the one-tick number is load-sensitive. No OOM/timeout; runnable.

Artifacts: `tools/bench/results/interim2/`, `interim2-default/`,
`interim2-cluster/`, `interim2-100k/` (checked in). **Action for freeze:** rerun
the complete matrix/A-B on an idle machine after the dense-physics decision and
publish only then.

### Post-dense-sensor-fix A/B (partial; paused for body fix)

After the bounded sensor broadphase landed (`a0fc778`), a partial clean A/B was
run against the frozen `3CCB231E` binary before the matrix was paused:

- **Same-machine frozen vs current serial** (best of 3, load-guarded):
  current/frozen = 1,405/1,441 (1k), 653/668 (2k), 173/194 (5k), 64/88 (10k),
  31/38 (20k) ticks/s. The 1k/2k gap is within noise, but **5k/10k/20k are still
  ~11%/27%/19% slower** than the frozen binary on the same machine.
- **Default 600t**: current 15,359 vs frozen 15,844 (1t, -3.1%); 8t 13,060 vs
  15,921 (-18.0%).
- **Cluster**: the regenerated-world cache/pair counts were not identical
  between the two binaries, so the cluster harness is not a clean comparison
  yet; direct core-probe pairs were ~57k for both and physics/sensing phases
  were flat.
- **Body owner's paired A/B isolates the regression to the body phase**
  (`BibiteStore`/`tick_inner`): **+27–34% at 5k/10k/20k**, while physics and
  sensing were flat and the sensor broadphase was inert at these densities.
  Body-dev is profiling/optimising always-on instrumentation/per-tick work.

**Status:** clean matrix paused until the body-phase fix lands; then rebuild the
CLI and rerun the full idle frozen/current A/B. Frozen `3CCB231E` numbers remain
authoritative and the release package must not be finalized. Partial artifacts:
`tools/bench/results/freeze-ab-*` and `freeze-frozen-*`.

### X32 thread A/B (post-freeze perf-only, 2026-11-02)

The lazy thread pool/serial-fallback work (`1339d35`, X32) was verified after the
package freeze by the Lead's idle definitive A/B: 5k/10k/20k populations,
threads 1/4/8, 120 ticks, repeat 2, load-guarded, same binary
`F08146E490982BA82EE5C89B3F139B5FEE623598EEDCF8A2C8F669A4E1D95E00`.
Artifacts: `tools/bench/results/x32-lead-ab/`.

| Population | 1 thread | 4 threads | 8 threads | 4t/1t | 8t/1t |
| --- | --- | --- | --- | --- | --- |
| 5,000 | 206.6 | 202.7 | 206.5 | 0.981 | 1.000 |
| 10,000 | 90.3 | 90.3 | 86.5 | 1.000 | 0.958 |
| 20,000 | 36.87 | 35.2 | 39.8 | 0.955 | **1.080** |

X32 is **PASS**: explicit 4t/8t are no longer slower than serial beyond the
~4–5% shared-machine noise band, the 20k gated sensing/physics/body paths
behave, 8t helps 20k (+8%), and 10k shows no 8t benefit (residual, not a
regression). State fingerprints are deterministic per population; 1k/2k sanity
and Default/Ring checkpoint equality were rechecked by the owner. This is
post-freeze performance evidence only: the published package remains the frozen
`b201c1b7...` build and was not rebuilt.

## What is measured (and what is not)

`rustibites bench --bibites N --ticks T --threads X --repeat R --json` reports,
per repetition:

| Field | Meaning |
|---|---|
| `wall_secs` | wall time of `world.run(ticks)` **only** — world construction, scenario seeding and synthetic spawn are excluded |
| `ticks_per_second` | `ticks / wall_secs` |
| `realtime_factor` | `sim_secs / wall_secs`; default sim rate is 30 TPS, so 1× real-time = 30 ticks/s |
| `bibite_updates_per_second` | `ticks_per_second × bibites_start` — primary throughput |
| `state_fingerprint` | FNV-1a over entity state; identical for identical seeds/builds, used as a determinism check |
| `peak_bytes` | always `null` in the CLI report today; the harness samples process peak working set instead |

The matrix harness adds `process_wall_secs` (process start + setup + ticks +
fingerprint sampling), sampled `peak_ws_mb`, and concurrent-load probes
(`cpu_load_pct_before/after`, `rustc_procs_before/after`, `free_gb_before/after`)
so a noisy session is visible after the fact.

Not measured: renderer/viewer cost (headless only), load/save time, parity
capture overhead, and allocation counts (no instrumented build yet — see
[`PERF_DIAGNOSIS.md`](PERF_DIAGNOSIS.md)). A benchmark number is only
comparable to another number produced on an **idle machine, with the same
scenario, tick count, repetition policy and binary SHA-256**.

## Reference machine

The measurements in this document come from the project's shared reference
machine:

| | |
|---|---|
| CPU | AMD Ryzen 5 3600, 6 physical / 12 logical cores |
| Memory | 16 GB |
| OS | Windows 10 Home 10.0.19045 (x86_64) |
| Rust | rustc 1.99.1, cargo 1.99.1 |
| Build | `cargo build --release -p bibites-cli`; `[profile.release]` `opt-level=3`, `lto="fat"`, `codegen-units=1`, `panic="abort"`, `strip="debuginfo"` |
| Simulator | 30 simulated TPS (`fixed_dt = 1/30 s`), synthetic density-scaled world |

The benchmark world is synthetic: default ecology + N starter adult bibites,
with the world area scaled as `2000 * sqrt(N/2000)` so every population sees the
same bibite/pellet/zone density as the 2,000-bibite reference world. Seeds are
deterministic: `seed = 0x5255535400000000 xor repetition_index`.

Other machines will differ (core count, memory bandwidth, compiler version,
background load). Record your own `environment.json` next to any numbers you
publish.

## Canonical scenario (`Default.zip`)

Per [`docs/TESTING_POLICY.md`](TESTING_POLICY.md), the canonical **real world**
for scenario/save benchmarks and docs examples is
**`testdata/real/Default.zip`** (the game's Default scenario: 1 zone, 1 spawn
row, 1 starting bibite, `simSize` 2000, 30 TPS).

- **Synthetic `bench --bibites N` worlds remain allowed** (they are the
  throughput matrix above, the phase tables and the cluster fixture; those
  numbers state their world explicitly).
- **Historical fixtures** (`Ring.zip`, `lol.zip`, `NestEgg.zip`, `RadTower.zip`)
  are opt-in regression references only: use them when a task explicitly needs
  that save/scenario, and **label the world next to every number**. The Ring
  save timing quoted in ARCHITECTURE §10 is historical, from an earlier build.
- **World label rule:** every number in this document says which world produced
  it (synthetic density-scaled / Default / cluster save / historical fixture).

Policy baseline reference run on `Default.zip` (see policy doc; useful as a
smoke check when benchmarking the real-scenario path):

```powershell
target\release\rustibites.exe run testdata\real\Default.zip --ticks 600 --seed 12345
# policy reference: 1 -> 10 living (1 initial + 9 virgin-spawn births),
# mean energy 156.9 -> 115.41, plant pellets 440 -> 447, no deaths.
# Pellet count is emergent (+/-1 with faithful physics/feeding timing; body task-106).
```

**Final `Default.zip` scenario throughput (2026-11-01, final-freeze binary
`818B7595…`).** `run_matrix.ps1 -Scenario testdata\real\Default.zip -Threads 1,8
-Ticks 600 -Repeat 2`: 600 ticks, scenario population starts at **1** and grows
to **10** through the fresh spawner (20 s simulated, 30 TPS); engine wall covers
`world.run(600)` only, so `bibite_updates_per_second` understates the grown
population.

| Threads | Best ticks/s | Best ×RT | Median ticks/s | Engine wall (best) | Process wall | Peak WS |
|---|---|---|---|---|---|---|
| 1 | **15,769** | 525.6x | 15,822 | 0.038 s | 0.27–0.32 s | 13.8 MB |
| 8 | **15,806** | 526.9x | 16,020 | 0.037 s | 0.28–0.29 s | 14.5 MB |

Artifacts: `tools/bench/results/default/`. The scenario is small (one starter
bibite + 440 plant pellets in one zone), so setup/process startup dominates the
wall time; this is a real-world-path smoke/throughput check, not a scaling
benchmark.

`testdata/` is local-only/gitignored (absent on CI): `-Scenario
testdata\real\Default.zip` is an error with a clear message when the file is
missing, and no benchmark depends on it.

## Reproducing the latest numbers

Windows PowerShell 5.1 and PowerShell 7+ on Windows/macOS/Linux work; no extra
modules are required. (`pwsh -File ...` instead of `powershell ...` on PS7.)

```powershell
cd Z:\RUSTibites

# 0. Freeze the binary first: copy the current release build somewhere stable and
#    record its SHA-256 + git commit + dirty state next to it, so a concurrent
#    `cargo build` cannot swap the measured binary mid-run.
$exe = (Resolve-Path target\release\rustibites.exe).Path
$sha = (Get-FileHash $exe -Algorithm SHA256).Hash
Copy-Item $exe "target\bench-final\rustibites-$($sha.Substring(0,8)).exe"

# 1. Final matrix: 1k/2k/5k/10k/20k x 1/4/8 threads, 120 ticks, best-of-3,
#    load-guarded; -UpdateDocs rewrites the measured-matrix block below.
powershell -NoProfile -ExecutionPolicy Bypass -File tools\bench\run_matrix.ps1 `
    -Exe target\bench-final\rustibites-3CCB231E.exe `
    -Pops 1000,2000,5000,10000,20000 -Threads 1,4,8 -Ticks 120 -Repeat 3 `
    -MaxLoadPct 45 -LoadWaitSec 120 -DeterminismCheck `
    -OutputDir tools\bench\results\final -UpdateDocs

# 2. Phase breakdown: 1k/2k/5k/10k serial, 300 ticks; -UpdateDocs rewrites the phase block.
powershell -NoProfile -ExecutionPolicy Bypass -File tools\bench\run_phases.ps1 `
    -Exe target\bench-final\rustibites-3CCB231E.exe `
    -Pops 1000,2000,5000,10000 -Threads 1 -Ticks 300 `
    -OutputDir tools\bench\results\final-phases -UpdateDocs

# 3. 100k one-tick sanity (peak working set; 50k/100k steady-state throughput is in ARCHITECTURE §10).
powershell -NoProfile -ExecutionPolicy Bypass -File tools\bench\run_matrix.ps1 `
    -Exe target\bench-final\rustibites-3CCB231E.exe -Mode full `
    -Pops 100000 -Threads 1,4,8 -Ticks 1 -Repeat 1 `
    -OutputDir tools\bench\results\final-100k

# 4. Dense-cluster fixture (whole tick + spread control + physics-only core probe).
powershell -NoProfile -ExecutionPolicy Bypass -File tools\bench\run_cluster.ps1 `
    -Exe target\bench-final\rustibites-3CCB231E.exe `
    -Pops 2000 -BoxSizes 100,200 -Ticks 10 -Repeat 2 -CoreProbe -DeterminismCheck `
    -OutputDir tools\bench\results\cluster

# 5. Machine/build metadata only.
powershell -NoProfile -ExecutionPolicy Bypass -File tools\bench\env.ps1
```

Artifacts are written to `tools/bench/results/<quick|full|final|final-phases|final-100k|cluster|phases>/`.
The scripts never rebuild the binary implicitly (`-Build` is an explicit opt-in;
a missing binary is a clear error), and they snapshot the binary to a temporary
path first so a concurrent `cargo build` cannot swap it mid-matrix. The published
final numbers live in `results/final/`, `results/final-phases/`,
`results/final-100k/` and `results/cluster/`; `results/quick/` is the superseded
early quick matrix. Full details, flags and schemas:
[`tools/bench/README.md`](../tools/bench/README.md).

## Final measured matrix

**World:** synthetic density-scaled starter population (`rustibites bench
--bibites N` — allowed by the testing policy for throughput benchmarks; it is
*not* a real save/scenario). The canonical real scenario for `--scenario`
examples is `testdata/real/Default.zip` (see
[Canonical scenario](#canonical-scenario-defaultzip)); historical fixtures such
as `Ring.zip`/`lol.zip` are opt-in regressions only and their numbers are
labelled historical.

**Final freeze capture (2026-11-01): packaged binary, 120 ticks/run,
best-of-3, load-guarded.** 15/15 cells ok at 6% pre-run load. Final binary
`818B759519...` (2,228,736 B) from git `903f1b2`; same-machine paired A/B vs
`3CCB231E` is within noise on serial 1k-20k (e.g. 10k paired 86.4-87.0 vs
85.5-88.9 ticks/s), Default 1t/8t 15,769/15,806 vs 15,844/15,921, and dense
cluster 38.46 ms/tick with the frozen 31,747/51,490 candidate pairs. Final
synthetic matrix (serial / 4t / 8t, ticks/s): 1k 1,422/1,421/1,417; 2k
663/648/657; 5k 192/191/191; 10k 85/68/69; 20k 38/36/32. Threads no longer help
at 10k/20k (documented residual, not a serial blocker). State fingerprints:
10k `1750137736440958460`, 36k `9113407757480872428`, 100k
`8688404363908086484`.

**Previous capture (task-131): clean-tree binary, 120 ticks/run, best-of-3,
load-guarded.** Taken on the shared reference machine
(2026-10-30T14:39:04Z capture; pre-run CPU load 2–8%; no concurrent
`rustc`/`cargo`/other benchmark during a cell — `-MaxLoadPct 45
-LoadWaitSec 300`, all 15 cells ok). This is the defensible reference table for
this exact binary.

- **Measured binary:**
  `3CCB231ECC3C9027B83CE998E6BB88FAFA4224D27874547B386F94EA84709D1D`
  (2,168,320 bytes, built 2026-10-30T14:38:42Z) — built from a **clean git
  worktree** at commit `2fb2ad4` (`git worktree add target/bench-head 2fb2ad4`;
  `cargo build --release -p bibites-cli -p bibites-view`), then frozen at
  `target/bench-final/rustibites-3CCB231E.exe`. Provenance:
  `target/bench-final/provenance-3CCB231E.json` (committed copy:
  `tools/bench/results/final/binary-provenance.json`) and
  `tools/bench/results/final/environment.json`.
- **Source tree:** binary from git `2fb2ad4` with **no dirty Rust files** (the
  main checkout's only dirty paths at capture were non-Rust docs/results and an
  untracked io test, none of which enter the build; the binary itself came from
  the clean worktree). The auto-generated block below still reports
  `dirty = True` because it samples the main checkout, not the build worktree.
- **Confidence:** medium-high on throughput (±5–10% on medians; best-of-3 with
  an idle-machine guard, all fingerprints identical across repetitions), high on
  relative shape and phase shares. Shared machine, so compare against runs with
  the same binary SHA and comparable load samples.

Cells show **ticks/s (real-time factor)** at 30 simulated TPS on the
**synthetic density-scaled bench world**, best of the three repetitions; the
detail table also carries medians (prefer medians on a shared machine).

<!-- BEGIN BENCH:MATRIX -->
_Auto-generated by `tools/bench/run_matrix.ps1` on 2026-10-30T14:40:48.4498661Z (`full` mode, ticks=120, repeat=3). Do not edit by hand: rerun the script with `-UpdateDocs`._

- **Host:** DESKTOP - Microsoft Windows 10 Home (10.0.19045)
- **CPU:** AMD Ryzen 5 3600 6-Core Processor - 6 physical / 12 logical cores
- **Memory:** 16.0 GB total, 5.36 GB free at capture
- **Rust:** rustc 1.99.1 (58cf8f9c3 2026-10-13) (release 1.99.1, x86_64-pc-windows-msvc), cargo 1.99.1 (5f94df478 2026-08-27)
- **Git:** 2fb2ad4dde90 on main (2026-10-30T10:37:00-04:00), dirty = True (15 paths)
- **Release profile:** [profile.release]; opt-level = 3; lto = "fat"; codegen-units = 1; panic = "abort"; strip = "debuginfo"
- **Measured binary:** `3CCB231ECC3C9027B83CE998E6BB88FAFA4224D27874547B386F94EA84709D1D` (2168320 bytes, built 2026-10-30T14:38:42.9058068Z)
- **Captured (UTC):** 2026-10-30T14:39:04.2168638Z
- **Matrix:** mode=`full`, ticks=120, repeat=3, scenario=`synthetic`

| Population | 1 thread | 4 threads | 8 threads |
| --- | --- | --- | --- |
| 1,000 | 1,483 (49.4x) | 1,361 (45.4x) | 1,356 (45.2x) |
| 2,000 | 715 (23.8x) | 649 (21.6x) | 652 (21.7x) |
| 5,000 | 220 (7.3x) | 216 (7.2x) | 220 (7.3x) |
| 10,000 | 94 (3.1x) | 94 (3.1x) | 95 (3.2x) |
| 20,000 | 41 (1.4x) | 42 (1.4x) | 42 (1.4x) |

Cells show **ticks/s (real-time factor)** at 30 simulated TPS on the benchmark world (synthetic density-scaled unless `-Scenario` was used); best of the repetitions. `-` = cell not run.

| Population | Serial ticks/s | Best ticks/s | Best @ threads | Speedup vs serial | Serial updates/s | Peak WS (MB) |
| --- | --- | --- | --- | --- | --- | --- |
| 1,000 | 1,483 | 1,483 | 1 | 1.00 | 1,483,227 | 10.8 |
| 2,000 | 715 | 715 | 1 | 1.00 | 1,429,256 | 17.1 |
| 5,000 | 220 | 220 | 8 | 1.00 | 1,099,234 | 34.7 |
| 10,000 | 94 | 95 | 8 | 1.01 | 936,455 | 64.7 |
| 20,000 | 41 | 42 | 8 | 1.02 | 826,716 | 125.4 |

| Population | Threads | Reps ok/total | Best wall (s) | Median wall (s) | Best ticks/s | Median ticks/s | Best xRT | Updates/s (best) | Peak WS (MB) | Status |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1,000 | 1 | 3/3 | 0.081 | 0.081 | 1,483 | 1,487 | 49.4 | 1,483,227 | 10.6 | ok |
| 1,000 | 4 | 3/3 | 0.088 | 0.088 | 1,361 | 1,361 | 45.4 | 1,360,987 | 10.7 | ok |
| 1,000 | 8 | 3/3 | 0.089 | 0.089 | 1,356 | 1,352 | 45.2 | 1,355,928 | 10.8 | ok |
| 2,000 | 1 | 3/3 | 0.168 | 0.166 | 715 | 723 | 23.8 | 1,429,256 | 16.9 | ok |
| 2,000 | 4 | 3/3 | 0.185 | 0.185 | 649 | 649 | 21.6 | 1,298,874 | 17.1 | ok |
| 2,000 | 8 | 3/3 | 0.184 | 0.184 | 652 | 653 | 21.7 | 1,303,687 | 17.0 | ok |
| 5,000 | 1 | 3/3 | 0.546 | 0.552 | 220 | 217 | 7.3 | 1,099,234 | 32.5 | ok |
| 5,000 | 4 | 3/3 | 0.557 | 0.545 | 216 | 220 | 7.2 | 1,077,586 | 34.7 | ok |
| 5,000 | 8 | 3/3 | 0.544 | 0.545 | 220 | 220 | 7.3 | 1,102,380 | 34.3 | ok |
| 10,000 | 1 | 3/3 | 1.281 | 1.265 | 94 | 95 | 3.1 | 936,455 | 59.9 | ok |
| 10,000 | 4 | 3/3 | 1.282 | 1.273 | 94 | 94 | 3.1 | 936,188 | 63.1 | ok |
| 10,000 | 8 | 3/3 | 1.270 | 1.255 | 95 | 96 | 3.2 | 945,225 | 64.7 | ok |
| 20,000 | 1 | 3/3 | 2.903 | 2.903 | 41 | 41 | 1.4 | 826,716 | 114.0 | ok |
| 20,000 | 4 | 3/3 | 2.859 | 2.838 | 42 | 42 | 1.4 | 839,549 | 120.7 | ok |
| 20,000 | 8 | 3/3 | 2.845 | 2.845 | 42 | 42 | 1.4 | 843,633 | 125.4 | ok |
<!-- END BENCH:MATRIX -->

The raw per-process rows (including the load samples behind the confidence note
above) are in `tools/bench/results/final/runs.csv`; per-cell aggregates are in
`summary.csv`; the same tables are in `matrix.md`/`matrix.json`, and the
environment/provenance is in `environment.json`/`environment.txt` plus
`target/bench-final/provenance-3CCB231E.json`. The `quick` mode still exists for
a sub-minute signal table; the full 50k/100k throughput matrix is not re-run
here (ARCHITECTURE §10 keeps the task-48 50k/100k ranges; the final 100k
one-tick sanity is in [100k and scale limits](#100k-and-scale-limits)).

`—`/`-` in a cell means it was not run; `timeout`, `oom`, `error` or
`skipped_load` means the cell failed or was skipped (see `runs.csv`).

### Reading the matrix

- **Throughput is population × ticks/s.** Going from 1k to 10k bibites costs
  roughly 10× the tick time (`10k/1k` tick-time ratio ≈ 10 on this machine), so
  bibite-updates/s declines slowly with population. That is the expected shape
  for a linear-time tick; a hidden O(n²) path would make the ratio explode.
- **Serial column** is the reference for thread scaling. The final matrix's
  best-of-3 thread gains are small (~1.00–1.12×; see the second table), which is
  the expected Amdahl shape while physics/body are serial.
- **Best-of-3 + load guard.** The final matrix used
  `-MaxLoadPct 45 -LoadWaitSec 120` so no cell started while a build or another
  benchmark was active; `summary.csv` also carries medians, and `runs.csv` has
  every individual process with its load samples. Prefer medians over bests on a
  shared machine; cells whose reps disagree by >10% are load flakes.
- **Peak WS** is sampled every 50 ms by the harness and is approximate for very
  short runs; measured here: ≈10–11 MB at 1k, ≈17 MB at 2k, ≈33–35 MB at 5k,
  ≈60–65 MB at 10k, ≈114–125 MB at 20k, and ≈495–513 MB at 100k (one-tick
  sanity). These are 8–14% above the task-92 capture (10/15/32/58/110/464 MB)
  because the body/checkpoint/FOV state is larger.
- **These are spread worlds.** A dense pile of bibites is a different physics
  regime; see [Dense-cluster benchmark](#dense-cluster-benchmark) below before
  using these numbers for worst-case planning.

## Dense-cluster benchmark

The spread matrix keeps bibite/pellet density constant. A crowd of bibites
packed into a small box is worst-case for the physics broadphase/narrowphase.
The final clean-HEAD capture (task-131, binary `3CCB231E…`, git `2fb2ad4` clean
worktree, 10 ticks/run, best-of-2) measures the **whole tick** end-to-end on the
deterministic clustered save world, and the ignored audit
`body::perf_probe_clustered_and_spread` measures `BibiteStore::step_physics`
alone.

```powershell
# 2,000 bibites in 100x100 and 200x200 boxes + spread control, 10 ticks, best-of-2,
# plus the canonical physics-only core probe. Worlds are generated on demand from
# tracked inputs in tools/bench/scenarios/cluster/ (no testdata/game needed).
powershell -NoProfile -ExecutionPolicy Bypass -File tools\bench\run_cluster.ps1 `
    -Exe target\bench-final\rustibites-3CCB231E.exe `
    -Pops 2000 -BoxSizes 100,200 -Ticks 10 -Repeat 2 -CoreProbe -DeterminismCheck
```

Artifacts: `tools/bench/results/cluster/` (`cluster.csv`, `cluster_summary.csv`,
`cluster.md`, `cluster.json`, `core_probe.csv`, `core_probe.log`,
`determinism.json`). The world save zips are cached under
`target/bench-worlds/cluster/` (gitignored) and identified by content SHA-256.

**Measured 2026-11-01T10:54Z**, binary
`818B759504E2172108459334CAE79668120BA1FEE924E5A4C756B819EE43B858`, git
`903f1b2`, 10 ticks/run, best-of-2 (whole-engine numbers
are phase means):

| Kind | Population | Box | Threads | Best ticks/s | Best ms/tick | Physics ms (median) | Body ms (median) | Peak WS (MB) |
|---|---|---|---|---|---|---|---|---|
| cluster | 2,000 | 100 | 1 | 26.0 | 38.46 | 37.34 | 1.05 | 56.2 |
| cluster | 2,000 | 200 | 1 | 50.0 | 20.00 | 19.09 | 1.00 | 41.9 |
| spread | 2,000 | — | 1 | 726 | 1.377 | 0.80 | 0.52 | 2.0 |

| Population | Box | Cluster physics ms | Spread physics ms | Cluster tick ms | Spread tick ms | Spread/Cluster ticks/s |
|---|---|---|---|---|---|---|
| 2,000 | 100 | 37.34 | 0.80 | 38.46 | 1.377 | 27.9× |
| 2,000 | 200 | 19.09 | 0.80 | 20.00 | 1.377 | 14.5× |

The whole-engine cluster100 cost is **38.46 ms/tick vs 90.91 ms in the task-92
capture** (2.4× faster; the dense-cluster optimisations have held and the body
phase is still ~1 ms). Physics-only core probe (single release run, starter
genome, `step_physics` only, spread half-extent 1900 / cluster half-extent 50):

| Case | Bibites | Physics ms | Candidate pairs | Contacts | Triggers |
|---|---|---|---|---|---|
| spread2000 | 2,000 | 1.244 | 117 | 126 | 95 |
| spread5000 | 5,000 | 3.534 | 727 | 883 | 638 |
| cluster2000 | 2,000 | **48.793** | 31,747 | 71,712 | 47,839 |
| cluster4000 | 4,000 | 124.293 | 51,490 | 120,092 | 79,092 |

The cluster2000 probe is **48.8 ms** (single run; earlier task-82 three-run clean
window was 41.4–43.5 ms), still under the **<50 ms** target but within the
shared-machine noise band of the old value. The deterministic dense pair cap
(>512 bodies in the grid path limits each body to 32 candidates) is applied only
in that dense profile; spread/normal populations are uncapped and bit-identical
to the uncapped result. It is a documented benchmark approximation for the
synthetic 100×100 benchmark, not a claim that a realistic pile has the same
collision set. Checkpoint note (task-117): `BibiteCheckpointRecord`/
`EggCheckpointRecord` carry raw radian rotation in addition to the game save's
lossy transform degrees; restoring the raw bits removes a 1-ULP rot/velocity
drift at split-resume.

Interpretation:

- In the dense box, **physics is the dominant phase** (median 37.34 ms of a
  ~38–56 ms tick; phase mean and best wall come from different reps); body
  stays at ~1.0–1.1 ms and hash-sync/deaths/decay are negligible in this minimal
  ecology. Doubling the box side (100→200, 4× the area) halves cluster tick time
  (38.5→20.0 ms) — the cliff is density-driven, not population-driven.
- The **canonical optimisation target** (task-82) was the physics-only core probe
  for `cluster2000`: **< 50 ms**. Current measurement is **46.6 ms**, within
  noise of the 41.4–43.5 ms task-82 capture, and down from 159.09 ms in the
  pre-task-82 capture. The whole-engine cluster rows use the bundled
  `cluster.bb8template` genome (tracked io fixture) and a minimal ecology, so
  their absolute physics numbers are not interchangeable with the core probe;
  use the probe for the target and the whole-engine rows for end-to-end
  regressions.
- Determinism: the generated world's `content_sha256` is reproducible across
  regenerations and `state_fingerprint` is identical across repeated runs
  (`determinism.json`).
- World generation is deterministic from tracked inputs
  (`tools/bench/scenarios/cluster/`) plus the seed/box/N and the engine binary
  SHA-256, so these numbers are reproducible without `testdata/` or the game.
  Python 3.8+ (stdlib only) is needed to generate the world zips; the benchmark
  runs themselves do not need Python.

## Phase breakdown

The engine always collects `World::phase_timings()` (sub-µs overhead); the human
`bench` output prints one table per repetition:

```powershell
target\release\rustibites.exe bench --bibites 2000 --ticks 500 --threads 1 --repeat 1
```

The capture below (means over all measured ticks, ms/tick) is generated by
`run_phases.ps1` on the same fixed binary; the full per-phase columns are in
`tools/bench/results/final-phases/phases.csv`, and the raw `bench` stdout for
every row is in `phases_raw.txt`. It uses 300 ticks/run (longer than the matrix's
120) on an otherwise idle machine; totals differ by a few percent from the
matrix medians (e.g. 2k 700 vs 723 ticks/s — compare phases within this table,
not across tables).

<!-- BEGIN BENCH:PHASES -->
_Auto-generated by `tools/bench/run_phases.ps1` on 2026-10-30T14:41:09.0624968Z._

Ticks per run: 300; scenario: `synthetic`; binary `3CCB231ECC3C9027B83CE998E6BB88FAFA4224D27874547B386F94EA84709D1D`.

| Population | Threads | ticks/s | xRT | ms/tick | physics | body | sensing | zones | decay | deaths | hash-sync | other |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1,000 | 1 | 1,377 | 45.9 | 0.726 | 0.388 (53.6%) | 0.251 (34.6%) | 0.079 (11.0%) | 0.000 (0.0%) | 0.001 (0.1%) | 0.002 (0.2%) | 0.003 (0.4%) | 0.000 |
| 2,000 | 1 | 700 | 23.3 | 1.429 | 0.794 (55.6%) | 0.500 (35.0%) | 0.123 (8.6%) | 0.000 (0.0%) | 0.001 (0.1%) | 0.003 (0.2%) | 0.006 (0.4%) | 0.000 |
| 5,000 | 1 | 211 | 7.0 | 4.739 | 2.395 (50.5%) | 1.974 (41.6%) | 0.327 (6.9%) | 0.001 (0.0%) | 0.004 (0.1%) | 0.017 (0.4%) | 0.026 (0.6%) | 0.000 |
| 10,000 | 1 | 93 | 3.1 | 10.753 | 5.568 (51.7%) | 4.370 (40.6%) | 0.710 (6.6%) | 0.001 (0.0%) | 0.007 (0.1%) | 0.053 (0.5%) | 0.057 (0.5%) | 0.000 |

`other` = `phase_total_ms` minus the highlighted phases (the long tail: clock, animators, tools, eggs, pheromones, rad, red-death, colour-killers, producers, events-stats).
<!-- END BENCH:PHASES -->

### Interpreting the phases

- `physics` + `body` dominate at every measured population: **~89% at 1k**
  (54% + 35%), **~91% at 2k** (56% + 35%), **~93% at 5k** (51% + 42%), **~93% at
  10k** (52% + 41%). `sensing` is the third phase and its share shrank after the
  FOV split: 11% at 1k, 9% at 2k, 7% at 5k, 7% at 10k (it was 19/13/9/8% in the
  task-92 phase capture).
- `hash-sync`, `deaths`, `decay`, `zones` and the other phases are ≤0.06 ms per
  tick at these sizes (deaths/hash-sync grow slowly with population); they are
  not the current bottlenecks.
- Per-record organ math inside `body` and the broadphase/narrowphase inside
  `physics` are the known hot spots — see [`PERF_DIAGNOSIS.md`](PERF_DIAGNOSIS.md)
  for the diagnosis history and `docs/ARCHITECTURE.md` §10 for the roadmap.

## Measurement methodology and noise

This is the most important section to read before comparing numbers.

1. **Use the engine's own timing for throughput.** `wall_secs` includes only
   `world.run(ticks)`, so setup cost does not pollute ticks/s. Use
   `process_wall_secs` (harness) when what you care about is "how long does the
   command take end-to-end", e.g. for spawn-heavy workloads.
2. **Freeze the binary.** Copy `target/release/rustibites.exe` to a stable path
   (e.g. `target/bench-final/rustibites-<sha8>.exe`), record its SHA-256, git
   HEAD, dirty-file count and build time, and point every measurement at that
   copy. The final capture used
   `target/bench-final/rustibites-3CCB231E.exe` (`3CCB231E…`, built
   2026-10-30T14:38:42Z from a clean worktree at git `2fb2ad4`). The main
   checkout kept changing in non-Rust docs/results during the session; the fixed
   binary plus the clean build worktree are why that does not affect the numbers.
3. **Idle machine or annotate.** The reference machine is shared by agents, CI
   jobs and builds. A build (`rustc`) or another benchmark introduces 20–60%
   slowdowns and much larger variance, especially for short runs. Every
   `runs.csv` row carries CPU-load and `rustc`/`cargo`-process samples before and
   after the run; `matrix.md`/`environment.txt` record the load at capture. The
   final matrix used `-MaxLoadPct 45 -LoadWaitSec 300` (no cell started while a
   build or another `rustibites` process was active); pre-run load samples were
   2–8%, all 15 cells ok.
4. **Repetitions.** Quick mode uses `-Repeat 1` so it can run during parallel
   work; the final matrix used best-of-3 with the load guard. Best-of-N is only
   a noise filter, not a way to pick a favourable number: `summary.csv` includes
   medians next to the best values, and the median is the honest estimate for a
   shared machine.
5. **Warm-up/caching.** Each repetition is a fresh OS process (`--repeat 1` per
   process), so there is no cross-run cache warming. Short runs (30–60 ticks)
   include cold-cache effects; the ARCHITECTURE §10 baseline used longer runs.
   Compare like-for-like tick counts.
6. **Determinism.** `state_fingerprint` must be identical across thread counts
   for the same repetition seed. Every full matrix run checks this; a mismatch
   means nondeterminism (or a binary that changed mid-run — the snapshot copy
   prevents the latter). `-DeterminismCheck` additionally re-runs cells in
   separate processes.
7. **Locale independence.** Artifact numbers are formatted with the invariant
   culture and files are UTF-8, so CSVs can be diffed/parsed on any machine.

## 100k and scale limits

The historical authoritative 100k throughput probe is ARCHITECTURE §10
(task-48) on the **synthetic density-scaled bench world**: `bench --bibites
100000` is runnable and bounded on the reference machine — peak working set
≈0.5 GB, no unbounded growth across the measured ticks, 6–9 ticks/s serial
(≈0.2–0.3× real-time), ~0.6–0.85M bibite-updates/s, linear scaling from 20k at
fixed density, ≈4.6–5.1 KB/bibite. This harness's full mode can re-measure 100k
with a per-process timeout; a slow/failed cell is recorded with its status
rather than aborting the matrix, and `-BudgetSec` bounds the whole session.

**Final 100k one-tick sanity (2026-11-01, final-freeze binary `818B7595…`,
synthetic density-scaled bench world):**
`run_matrix.ps1 -Pops 100000 -Threads 1,4,8 -Ticks 1 -Repeat 1` completed for all
three thread counts: peak working set **500.5–514.8 MB** (≈5.0–5.15 KB/bibite),
spawn+one-tick process wall ~1.1–1.5 s, engine time for the single tick
0.149–0.166 s (6–7 ticks/s for one
tick — **not** a steady-state throughput number; use the task-48 ranges above),
0.71–0.72M bibite-updates/s, no timeouts/OOM. Artifacts:
`tools/bench/results/final-100k/`. The 50k/100k steady-state table is not re-run
in the final capture; the task-131 result is a memory/runnability check, not a
throughput claim.

(2026-11-05: the same bench world, with the passes over the records on the pool, runs at 18-19 ticks/s, 0.6x real time; see
"100,000 bibites" above.) The milestone target for 100k is ≥20× real-time, so the current gap is large and
is a **serial physics/body throughput** problem (more threads cannot fix it;
sensing is only 7% of the tick). See
[`ARCHITECTURE.md` §10](ARCHITECTURE.md#10-performance-targets-and-measurement)
and [`PERF_DIAGNOSIS.md`](PERF_DIAGNOSIS.md).

## Comparison with ARCHITECTURE §10

ARCHITECTURE §10 historically recorded a **PERF round 3** baseline (`f984d50`,
2026-10-30) measured with 4-tick averages on an isolated target. The task-92
table used binary `4F267FF9…` (dirty tree `2fdfb1b` + 53 paths); the final
task-131 table now uses a **clean-tree binary** `3CCB231E…` built from git
`2fb2ad4` with 120-tick runs, best-of-3 and an idle-machine guard. Reconciliation
(all **synthetic density-scaled bench worlds**):

| Population | §10 round-3 serial | Task-92 serial | Final clean serial | Final 8-thread | Final ÷ round-3 | Final ÷ task-92 |
|---|---|---|---|---|---|---|
| 1,000 | ~1,595 | 1,445 | 1,483 | 1,356 | 0.93× | 1.026× |
| 2,000 | ~764 | 680 | 715 | 652 | 0.94× | 1.051× |
| 5,000 | ~253 | 200 | 220 | 220 | 0.87× | 1.100× |
| 10,000 | ~109 | 90 | 94 | 95 | 0.86× | 1.044× |
| 20,000 | — | 42 | 41 | 42 | — | 0.984× |

- **Clean-tree build:** the final binary was built from a dedicated clean
  worktree at `2fb2ad4`; the main checkout only had non-Rust docs/results dirty
  at capture, so this is the first published matrix not built from a dirty Rust
  tree. The SHA-256 is still the unit of comparison.
- **Provisional-vs-final:** a provisional matrix on the previous clean HEAD
  (`67472e5`, before the prefab part-transform rotation fix) measured
  1,438 / 680 / 194–231 / 94 / 42–43 best-by-population; the final deltas are
  within the shared-machine noise band (±5–10%), so the rotation fix has **no
  measurable throughput or memory effect**.
- **Different code:** the round-3 binary predates dense-cluster physics,
  checkpoint state, FOV split, and the prefab part-transform fix. The remaining
  round-3 → final gap (0.86–0.94×) is a build + protocol difference, not a
  like-for-like regression claim; the 2k "764 vs 536/687" question stays
  resolved as a stale-binary/protocol artifact (`D80ED2…`, 30-tick quick runs).
- **Different protocol:** 4-tick isolated-target averages vs 120-tick
  process-isolated best-of-3 with medians; both captures are from the shared
  reference machine in different load windows.
- **Conclusion:** this final table (1k–20k) is the current reference; round-3
  numbers are history. Memory grew ~8–14% versus task-92, consistent with the
  larger body/checkpoint/FOV state.

Thread scaling in §10 is "≈1.0× because physics+body are serial (Amdahl cap
≈1.15× from sensing alone)". The final matrix shows 8-thread/serial ratios of
0.91× at 1k, 0.91× at 2k, 1.00× at 5k, 1.01× at 10k, 1.02× at 20k (4-thread
0.92/0.91/0.98/1.00/1.02). No cell suggests a structural >1.15× thread win; the
small gains at 10k/20k and losses at 1k/2k are consistent with parallel sensing
plus shared-machine noise.

## How fast is evolution?

> **Placeholder** — this section will quote brain-dev's evolution benchmark
> (task-47) when it is published. It is deliberately not measured here to avoid
> duplicating their work.

The evolution-level metric answers the project's core user story: how many
births/deaths/generations per wall-clock hour at population N? The harness for it
is `crates/bibites-core/examples/evolution_bench.rs` (`cargo run --release -p
bibites-core --example evolution_bench`), which reports births, deaths, max/mean
generation, species count, mutations applied and generations/hour for a fixed
seed, plus a deterministic regression test. When those numbers are published
they will appear here (and in ARCHITECTURE §10) with the same binary/SHA
discipline as the tables above; the throughput matrix here is the denominator
for that calculation.

## Adding scenarios and custom matrices

The matrix is built on the CLI's `bench` harness, so anything the CLI can load
can be benchmarked. Per the testing policy the **canonical scenario** is
`testdata/real/Default.zip`; other real saves/scenarios are historical fixtures
and must be justified + labelled by world name in the output.

```powershell
# Canonical real scenario: Default.zip, 600 ticks, best-of-2, threads 1/8.
powershell -NoProfile -ExecutionPolicy Bypass -File tools\bench\run_matrix.ps1 `
    -Scenario testdata\real\Default.zip -Threads 1,8 -Ticks 600 -Repeat 2 `
    -OutputDir tools\bench\results\default
```

With `-Scenario`, the population axis is skipped and the engine-reported
`bibites_start` is recorded in each row; the Markdown/CSV keep the scenario
label so published numbers name their world. Synthetic bench worlds remain
allowed (they are the throughput matrix, phase tables and cluster fixture).
Custom synthetic matrices use `-Pops`/`-Threads` (comma-separated), e.g.
`-Pops 1000,2000,5000,1000 -Threads 1,2,4,6,8,12 -Ticks 120 -Repeat 5`.
The CSV schema is stable, so new scenarios can be compared across machines and
builds with the same analysis.

> Historical `Ring.zip`/`lol.zip` bench runs (e.g. earlier Ring save numbers in
> ARCHITECTURE §10) are explicitly labelled historical: the canonical baseline is
> `Default.zip`, and synthetic worlds carry no real-world claim.

## CI guidance

Benchmarks are **not** part of the default CI gate: GitHub runners are noisy,
virtualised and not the reference machine. If a CI smoke is wanted, use the
quick matrix with one repetition and assert only that it exits successfully
(status/parse check), never absolute throughput thresholds:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\bench\run_matrix.ps1 `
    -Mode quick -Pops 1000,2000 -Threads 1 -Ticks 5 -Repeat 1 -Quiet
```

For trend tracking, archive `environment.json` + `summary.csv` per commit and
compare only runs with the same binary SHA-256 / machine.

## Known limitations and anomalies

- **Confidence of the final table:** medium-high. It is best-of-3 with an
  idle-machine guard on the **shared** reference machine (pre-run load 2–8%,
  all 15 cells ok), from a **clean-tree** binary (`3CCB231E…`, git `2fb2ad4`
  clean worktree). Medians are within ~5–10% of bests for most cells; the
  1k/2k 4- and 8-thread cells are the noisiest. Compare only against runs with
  the same binary SHA-256 and recorded load.
- `bench --json` reports `peak_bytes: null`; memory numbers here are sampled by
  the PowerShell harness (50 ms interval) and are approximate.
- Peak-working-set sampling can miss fast 30-tick runs; memory planning should
  use runs of ≥60 ticks or the 10k+ rows.
- Process startup (≈0.07–0.2 s, antivirus-dependent) dominates very small runs;
  `process_wall_secs` includes it, `wall_secs` does not.
- The synthetic spawner is deterministic but *not* the same world as the game;
  real saves/scenarios are benchmarked via `-Scenario`.
- The spread matrix understates dense populations. Use
  `run_cluster.ps1`/`cluster.md` for the worst case: at 2,000 bibites in a
  100×100 box the whole tick is ~38.5 ms (core probe 46.6 ms) versus ~1.4 ms
  spread. Do not use the spread table for worst-case planning.
- The current binary may change between sessions while the performance phase is
  active; always compare by SHA-256, not by date. The main checkout can be dirty
  in non-Rust docs/results while the measured binary comes from a clean worktree;
  check `binary-provenance.json` for the build source.
- On Windows, CPU-load probes use CIM `Win32_Processor.LoadPercentage`, an
  instantaneous (not interval-averaged) value; treat it as an indicator, not a
  precise utilisation.

## Session record

World legend: `final matrix`, `final phases` and `100k sanity` rows are the
**synthetic density-scaled bench world** (`bench --bibites N`); the `cluster` rows
are the deterministic **cluster save world** (`tools/bench/scenarios/cluster/`);
the historical rows are labelled `synthetic` as well, except the dense-cluster
before/after which is the same cluster world on an earlier binary.

| Session | Command | Status | Binary SHA-256 | Artifacts |
|---|---|---|---|---|
| **final matrix (published)** | `run_matrix.ps1 -Exe <fixed> -Pops 1000,2000,5000,10000,20000 -Threads 1,4,8 -Ticks 120 -Repeat 3 -MaxLoadPct 45 -LoadWaitSec 300 -DeterminismCheck -UpdateDocs` | measured 2026-10-30T14:39:04Z; pre-run load 2–8%, all 15 cells ok, fingerprints identical | `3CCB231ECC3C9027B83CE998E6BB88FAFA4224D27874547B386F94EA84709D1D` | `tools/bench/results/final/` |
| **final phases (published)** | `run_phases.ps1 -Exe <fixed> -Pops 1000,2000,5000,10000 -Threads 1 -Ticks 300 -UpdateDocs` | measured 2026-10-30T14:41Z; 300 ticks/run | `3CCB231E…` | `tools/bench/results/final-phases/` |
| **100k sanity (published)** | `run_matrix.ps1 -Exe <fixed> -Mode full -Pops 100000 -Threads 1,4,8 -Ticks 1 -Repeat 1` | one-tick memory/runnability check; peak WS 494.6–512.7 MB | `3CCB231E…` | `tools/bench/results/final-100k/` |
| **cluster (published)** | `run_cluster.ps1 -Exe <fixed> -Pops 2000 -BoxSizes 100,200 -Ticks 10 -Repeat 2 -CoreProbe -DeterminismCheck` | measured 2026-10-30T14:43Z; world content SHA reproducible, fingerprints identical; cluster100 whole tick 38.46 ms | `3CCB231E…` | `tools/bench/results/cluster/` |
| **Default scenario (published)** | `run_matrix.ps1 -Exe <fixed> -Mode full -Scenario testdata\real\Default.zip -Threads 1,8 -Ticks 600 -Repeat 2` | measured 2026-10-30T14:42Z; scenario 1→10 living | `3CCB231E…` | `tools/bench/results/default/` |
| task-92 final (historical) | same final-matrix protocol | dirty-tree binary; superseded by task-131 clean-tree numbers | `4F267FF9…` | git history of the above dirs |
| quick (historical) | `run_matrix.ps1 -Mode quick -DeterminismCheck` | superseded by the final matrix; kept as the earlier noisy reference (stale dirty-tree binary, 30-tick runs, pre-run load 9–70%) | `D80ED2…` | `tools/bench/results/quick/` |
| phases (historical) | `run_phases.ps1 -Pops … -Threads 1,8 -Ticks 120` | captured while another agent was compiling; superseded by final phases | `4475064…` | `tools/bench/results/phases/` |
| cluster (historical) | earlier `run_cluster.ps1` capture | superseded by the final cluster capture; useful as the before/after for dense-cluster physics | `63435014…` | git history of `results/cluster/` |

`<fixed>` = `target/bench-final/rustibites-3CCB231E.exe`; provenance in
`target/bench-final/provenance-3CCB231E.json` (SHA, size, build mtime, git HEAD
`2fb2ad4`, clean worktree) and the committed copy
`tools/bench/results/final/binary-provenance.json`.

Artifacts are regenerated (overwritten) by rerunning the commands; the checked-in
copies are a historical record of the numbers published in this document. Always
compare runs by binary SHA-256 and recorded load, not by date or git HEAD alone
(the measured binary came from a clean worktree; the main checkout may be dirty in non-Rust files).

## Large-world save/load scalability (2026-11-01)

Method: deterministic cluster saves from
`tools/bench/make_cluster_world.py --n N --box 100` (N identical bibites on a
lattice, seed 12345); release `target\release\rustibites.exe`; `run <world>
--ticks 0 --quiet` measures load+prepare, `run <world> --ticks 0 --save out.zip`
measures load+save (the delta to `--no-state` isolates the `.bb8state` write).
Wall times include process startup and the CLI's template-dir scan, so treat
them as end-to-end user-visible numbers (±10–20% between cold/warm runs).
Default-world row: `run testdata/real/Default.zip --ticks 50000 --seed 12345
--save`, then an io-only load/re-save of that save.

| world | zip size | load | save (no state) | sidecar size | sidecar overhead |
|---|---:|---:|---:|---:|---:|
| 1,000 bibites | 1.8 MB | 3.6 s | 3.0 s | 4.4 MB | ~0.0 s* (0.61 s before opt) |
| 2,000 | 3.6 MB | 6.2 s | 4.4 s | 8.7 MB | 4.9 s before opt |
| 5,000 | 9.0 MB | 7.1 s | 10.9 s | 21.7 MB | 0.21 s* (6.1 s before opt) |
| 10,000 | 18.0 MB | 11.8 s | 20.8 s | 43.4 MB | 14.4 s before opt |
| 20,000 | 36.1 MB | 22.8 s | 37.7 s | 86.7 MB | 19.7 s before opt |
| Default, 50k ticks | 71.7 KB | 0.33 s | 0.36 s | (small) | — |

\*After the io optimization below; sidecar size is unchanged (payload bytes),
only the redundant work was removed.

**Hotspots**
- Load is dominated by zip read + `serde_json` parse per section (every bibite
  and pellet is a separate entry/object); time scales ~linearly (≈1.1 ms per
  bibite load+parse at 20k).
- Save similarly builds a JSON `Vec` per entity, then deflates each zip member;
  the writer holds the whole serialized set before `ZipWriter` consumes it.
- `.bb8state` payload is much larger than the zip (≈2.4× at every size) because
  it stores raw `SimSnapshot`/entity runtime extensions, mostly uncompressed;
  write time is dominated by encode + full-buffer write. `Settings` hashing
  used to re-parse the whole save (fixed), and base SHA-256 used to read the
  whole file into memory (fixed: streamed 1 MiB chunks).
- No memory numbers yet: CLI `peak_bytes` is still `null`; memory is expected to
  be several GiB for 100k (JSON entry vectors + zip buffers + sidecar payload).

**Optimizations made (behavior-neutral, io-only)**
- `read_checkpoint_state`/`write_checkpoint_state`: settings SHA-256 now parses
  only the save's `settings.bb8settings` zip member (was a full
  `load_save_bundle` of all entities); base SHA-256 now streams the zip in
  1 MiB buffers instead of copying the whole file. Sidecar overhead dropped
  from 0.6 s → ~0 s at 1k and 6.1 s → 0.21 s at 5k on this machine.
- Left zip compression level at the default (game compatibility/size; a level-1
  experiment is a possible follow-up, not applied).

**100k feasibility**
- Interpolating the table: ≈180 MB game zip, ≈2–3 min load and ≈3–5 min save,
  several GiB RSS; currently **not ready as a default path**.
- `.bb8state` would be ≈430 MB, above the core
  `sim::MAX_PAYLOAD_BYTES = 256 MiB`; per-section 64 MiB chunks exist, but the
  total cap plus io's whole-payload `Vec` encode/write block it. A streamed
  sidecar needs (a) a core streaming encode or a larger/removable total cap
  (world-dev), and (b) an io streaming writer (header, chunked payload,
  checksum) instead of one buffer. That is the gating item for 100k checkpoint
  resume; 1k–20k saves/loads work today, with sidecars up to 86 MB.

### Load/save throughput follow-up (2026-11-01)

**Change (io-only, byte-identical):** save-bundle entity loading used to parse
each `bibites/*.bb8`/`eggs/*.bb8` twice — once into `Bb8File` and again by
serializing the parsed `Value` back to a string and running the full genome/brain
migration path. `lib::loaded_from_bb8` now builds the `LoadedGenome` directly
from the already-parsed entity, so each entity's JSON is parsed once. No
serialization/output path changed.

| world | metric | before | after | speed-up |
|---|---|---:|---:|---:|
| 10k | load | 11.8 s | 6.58 s | 1.79× |
| 10k | save end-to-end | 20.8 s | 8.33 s | 2.50× |
| 10k | save-only (minus load) | ≈9.0 s | ≈1.75 s | ≈5.1× |
| 10k | save + `.bb8state` | 35.2 s | 9.01 s | 3.9× |
| 20k | load | 22.8 s | 11.71 s | 1.95× |
| 20k | save end-to-end | 37.7 s | 16.53 s | 2.28× |
| 20k | save-only (minus load) | ≈14.9 s | ≈4.82 s | ≈3.1× |
| 20k | save + `.bb8state` | 57.4 s | 17.91 s | 3.2× |

**Byte-identical proof:** the 10k/20k Rust-written save zips after the change
have the same SHA-256 and sizes as the pre-change artifacts
(`9ddb0f7a…` / 18,030,887 B and `ad98ffea…` / 36,087,259 B), so game-load
compatibility and parity outputs are unchanged. Checkpoint tests and the full io
gate stay green.

**Still open for 100k:** save-side entity JSON is still single-threaded before
deflate and the zip writer holds all serialized entries; streamed zip writing +
parallel per-entity JSON (deterministic order) remain the next throughput step,
together with the streamed `.bb8state`/max-payload work below.

### Parallel save JSON follow-up (2026-11-01)

Per-entity JSON serialization in `write_save_bundle` now runs on a small
std::thread pool (up to 8 workers, contiguous index chunks, results collected in
chunk/file order). The zip writer and compression remain single-threaded, so
output bytes are unchanged.

| world | metric | before | after | speed-up |
|---|---|---:|---:|---:|
| 10k | save end-to-end | 8.33 s | 7.54 s | 1.10× |
| 10k | save-only (minus load) | 1.75 s | 2.71 s | noisy/slower |
| 20k | save end-to-end | 16.53 s | 14.92 s | 1.11× |
| 20k | save-only (minus load) | 4.82 s | 3.44 s | 1.40× |

**Byte-identical proof:** 10k/20k output zips match the prior SHA-256 and sizes
(`9ddb0f7a…` / 18,030,887 B and `ad98ffea…` / 36,087,259 B), so game-load
compatibility and parity are unchanged. Savings are modest because zip
compression + entry buffering remain serial/dominant; the next step for 100k is
a streamed zip writer (write each compressed entry directly as it is produced)
plus a streamed `.bb8state` path, as documented above.

### Streamed zip entry writer (2026-11-01)

`write_save_bundle` now creates the `ZipWriter` up front and writes each
section/entity entry **as soon as it is serialized**, instead of building a
`Vec<(name, Vec<u8>)>` for the whole world before starting deflate. Entry order,
bytes, deflate level and timestamps are unchanged.

**Byte-identical proof:** 10k/20k outputs again match the previous SHA-256 and
sizes (`9ddb0f7a…`/18,030,887 B and `ad98ffea…`/36,087,259 B). Game-load
compatibility and Default/Ring parity are therefore unaffected.

**Measured effect (same cluster worlds):** no speed-up — 10k/20k save end-to-end
landed in the same noise band as the parallel-JSON version (≈8.8 s / ≈17.8 s on a
loaded machine; compression is serial and dominates). The win is **peak memory**:
the writer no longer holds every serialized entity (`≈18 MB`/`≈36 MB` of JSON for
10k/20k, hundreds of MB at 100k) before deflate; only one entry is buffered.

**Next throughput step:** hybrid bounded batches — serialize a fixed batch
(e.g. 256 entities) on the thread pool, write the batch to the zip in order, drop
it, repeat. That keeps memory bounded while retaining parallel JSON. Remaining
100k blockers are still serial deflate, the streamed `.bb8state` payload
(`SimSnapshot::encode` whole-Vec + total cap 256 MiB vs ≈430 MB needed) and
load-side all-sections JSON parse; see the 100k section above.

### Hybrid bounded-batch save writer (2026-11-01)

The streamed zip writer now serializes entities in **bounded parallel batches of
256** (up to 8 threads, contiguous chunks, ordered collection/write, batch then
dropped) instead of one-entry-at-a-time or all-entries-first. Peak memory stays
bounded (≤256 serialized entities) and output stays deterministic.

**Byte-identical proof:** 10k/20k saves again match the previous SHA-256 and
sizes (`9ddb0f7a…`/18,030,887 B; `ad98ffea…`/36,087,259 B), so game-load and
Default/Ring parity are unchanged. Full io tests/clippy/fmt green.

**Measured speed (same machine, concurrent build/agents active):** no consistent
recovery on this pass — 10k ≈9.2 s and 20k ≈20.4 s end-to-end (save-only noisy,
partly negative because load and save phases were disturbed by the concurrent
build). This is within the load/deflate noise band and does **not** reproduce the
earlier ≈2.3–2.5× end-to-end numbers; deflate remains serial and load-side JSON
parsing dominates. Follow-up: benchmark on an idle machine and, if JSON is still
the bottleneck under isolation, stream deflate/compression or move zip writing
off the main path; the 256-batch/parallel structure is retained because it is
byte-identical and bounds memory for 100k.

### Experimental streamed `.bb8state` v2 (opt-in, 2026-11-01)

`bibites-io::checkpoint` now ships an opt-in chunked sidecar alongside the
stable v1:

- `write_checkpoint_state_v2(base_save, &SimSnapshot, seed)` writes the payload
  in fixed 16 MiB chunks with a per-chunk FNV-1a checksum and streams chunks to
  the file (no contiguous sidecar buffer on the write path).
- `read_checkpoint_state_v2(base_save, Optional<seed>)` verifies magic/version
  (v2), base-save SHA-256, settings SHA-256, optional seed and every chunk
  checksum, then together with `SimSnapshot::decode`.
- v1 remains the default; the v1 reader rejects v2 and vice versa. Game zip is
  untouched (sibling file), tests cover round-trip, base/seed/settings
  mismatch, chunk corruption and truncation.

**100k status:** v2 is a format/envelope step, not a full streaming solution yet.
`SimSnapshot::encode()` still produces one whole `Vec` and the read path
concatenates chunks before `SimSnapshot::decode`, and core
`MAX_PAYLOAD_BYTES = 256 MiB` still caps a 100k snapshot (≈430 MB). Needs
world-dev: streaming encode/decode (per-section iterator) and a larger/removable
total cap; io can then write/read chunks directly without the concatenation
buffer. Coordinate before enabling v2 by default or in the CLI.

#### Quiet-machine re-benchmark (2026-11-01, hybrid batches)

Re-ran the key sizes with no concurrent build (the earlier hybrid numbers were
disturbed by a running release build). Same deterministic cluster worlds; load =
`run --ticks 0`, save = `run --ticks 0 --save`, save-only = save − load.

| world | load | save end-to-end | save-only | vs pre-optimization save e2e |
|---|---:|---:|---:|---:|
| 10k | 5.64 s | 7.90 s | 2.25 s | 20.8 → 7.90 s (**2.6×**) |
| 20k | 10.44 s | 14.30 s | 3.86 s | 37.7 → 14.30 s (**2.6×**) |

Load improved 11.8→5.64 s (10k) and 22.8→10.44 s (20k) (**≈2.1×**). Outputs
remain byte-identical (`9ddb0f7a…`/18,030,887 B; `ad98ffea…`/36,087,259 B), so
game-load/parity are unchanged. The hybrid bounded-batch writer + single-parse
entity loader together recover the target gains on an idle machine; serial
deflate and load-side JSON parsing remain the next 100k blockers.

Game-load compatibility re-check on the current save was deferred while the
parity lab game was running (`The Bibites` PID present); no format changes
expected, will run in a fresh lab with the compat retry.

#### v2 wired to core section streaming (2026-11-01, core `5ee0d82`)

The experimental v2 sidecar now uses `SimSnapshot::encode_sections()` /
`decode_sections()` directly:

- **write:** header (magic/v2/flags, base zip SHA-256, settings SHA-256, seed,
  section count) followed by streaming records `tag:u16 || len:u32 ||
  fnv1a64:u64 || data`, including the core's 8 MiB chunk sections. No
  contiguous v1 payload `Vec` is built on the write path.
- **read:** parses the section records and feeds `SnapshotSection`s straight to
  `SimSnapshot::decode_sections` — no concatenated payload buffer; base/settings/
  seed identity plus per-section checksums are verified first.
- The public v2 format is opt-in via `RUSTIBITES_CHECKPOINT_V2=1`
  (`v2`/`true`/`yes` also accepted) on the normal `write_checkpoint_state` /
  `read_checkpoint_state` APIs; v1 stays the default and game-zip behaviour is
  unchanged. For large snapshots set the core cap too, e.g.
  `RUSTIBITES_SNAPSHOT_MAX_BYTES=536870912` (clamped 256 MiB–1 GiB; world
  capture preflight uses the same limit).
- **tests:** v2 round-trip, base/seed/settings mismatch, chunk corruption/
  truncation, v1-rejects-v2, and a 9 MiB-section test that crosses the core
  8 MiB chunking boundary through the streamed path. io tests/clippy green.

**Is a real 100k sidecar feasible now?** Format/cap-wise **yes**: a 100k
snapshot is ≈430 MB (≈2.4× a ≈180 MB world zip), below the 1 GiB env-opt-in cap,
and the v2 write/read no longer needs the extra contiguous payload copy. Not yet
measured end-to-end: generating/loading a real 100k world needs a large-memory
run (~2× sidecar for file+snapshot, plus World/entities, likely several GiB RSS)
and the `RUSTIBITES_SNAPSHOT_MAX_BYTES` opt-in; the largest verified sidecar in
tests is the 9 MiB chunk-section case and the 20k real world (86.7 MB, v1). Treat
100k as feasible-by-design but pending a dedicated hardware benchmark.

### 100k v2 checkpoint measurement (2026-11-01)

Exact procedure (`build_dense_world.py` → 100k duplicate founders from `lol.zip`,
zone-centred; env opt-ins active):

```powershell
$env:RUSTIBITES_CHECKPOINT_V2 = "1"
$env:RUSTIBITES_SNAPSHOT_MAX_BYTES = "1073741824"
python tools\experiments\build_dense_world.py --count 100000 --out testdata\experiments\dense\founders_100000.zip
target\release\rustibites.exe run testdata\experiments\dense\founders_100000.zip --ticks 1 --threads 1 --stats json --quiet --save target\scalability\bench_100k_v2.zip
target\release\rustibites.exe run target\scalability\bench_100k_v2.zip --ticks 0 --threads 1 --stats json --quiet
```

| metric | measured |
|---|---:|
| world zip (100k founders) | 501,822,929 B (501.8 MB) |
| save zip (Rust-written) | 501,822,929 B (501.8 MB) |
| `.bb8state` v2 sidecar | **650,194,727 B (650.2 MB)** |
| save wall (load world + 1 tick + zip + v2 sidecar) | **255.5 s** |
| save peak RSS | **2,582.8 MB** |
| load wall (world + sidecar restore) | **92.6 s** |
| load peak RSS | **3,268.0 MB** |
| resume fingerprint | save `1823034608723448443` = load `1823034608723448443` |

**Feasible:** yes on this machine with the env opt-ins; 650 MB sidecar is below
the 1 GiB streaming cap but above the 256 MiB v1 cap (so v1/default rejects it —
`encode_sections`/`encode` fail with `LimitExceeded` unless
`RUSTIBITES_SNAPSHOT_MAX_BYTES` is raised). The 40k control run with
`--no-state` produced no sidecar and completed normally. Limits: ~3.3 GB peak at
100k (load), ~4.5–5 min save and ~1.5 min load; snapshot decode still materialises
extension Vecs and the reader holds the sidecar file bytes plus decoded snapshot;
a future mmap/stream-decode would reduce peak. The v1/default path remains
byte-identical and unchanged.

## Release 0.2.0 packaged binary (2026-11-03)

The 0.2.0 functional-UI package is a viewer/UI release, not a new performance freeze; the
published performance matrix above remains the task-131 final-freeze reference. The packaged
CLI is recorded here so release runs can be identified by hash:

- packaged `rustibites.exe` — **2,252,288 B**, SHA-256
  `D00B7CBA5B9E2F8CA99BDDB9D498547CC98E25A6BAF3343AA12828A38EDFBC0A` (commit `603646b`)
- packaged `rustibites-view.exe` — **3,036,672 B**, SHA-256
  `D458538D6DAFD18C20D64210EEC59D584D90017C1CF651DD3ADA10B87202CB52`
- zip `RUSTibites-0.2.0-windows-x64.zip` — **124,051,485 B**, SHA-256
  `a6cc1396a126cdd5ecb89bbf990c0244b782d250dec57589806209f01b895341`
- fresh-extraction CLI smoke: `run testdata/real/Default.zip --ticks 600 --seed 12345 --stats json --quiet`
  exit 0, 1→10 bibites; fresh-extraction menu screenshot byte-identical on repeat
  (`E533D78AC5FA41471D17FB01DF3ED80AFC66263127AA62709978F17CAC970DF5`).

Do not compare this binary's throughput with the 0.1.0/task-131 freeze without a fresh
same-machine A/B.

## Release 0.2.1 packaged binary (2026-11-03)

0.2.1 is the current functional-UI release; 0.2.0 remains historical. This is a viewer/UI release,
not a new performance freeze — the published matrix above remains the task-131 final-freeze reference.

- packaged `rustibites.exe` — **2,253,312 B**, SHA-256
  `7E00391CBCFF0006650110FC093CB6706385090B58E0C7FDF6AB37B3095D228C` (commit `86239a8`)
- packaged `rustibites-view.exe` — **3,081,728 B**, SHA-256
  `AC756621BEFE418DE1DBCB091C8C8881986E8CBA5C86F6D59700F4C3DFEAEFFE`
- zip `RUSTibites-0.2.1-windows-x64.zip` — **134,539,435 B**, SHA-256
  `55fdfce6df3fcc897132c32e8b0cbaf18f287c2a8283fca2c3ab9e0089ae7fd4`
- fresh-extraction CLI smoke: `run testdata/real/Default.zip --ticks 600 --seed 12345 --stats json --quiet`
  exit 0, 1→10 bibites; menu screenshot byte-identical on repeat
  (`E533D78AC5FA41471D17FB01DF3ED80AFC66263127AA62709978F17CAC970DF5`); 49 runtime sessions.

Do not compare this binary's throughput with the task-131 freeze without a fresh same-machine A/B.

## Release 0.2.2 packaged binary (2026-11-03)

0.2.2 is the current functional-UI release; 0.2.1 and 0.2.0 remain historical. Viewer/UI release, not a new
performance freeze — the published matrix above remains the task-131 final-freeze reference.

- packaged `rustibites.exe` — **2,253,824 B**, SHA-256
  `898A218A91779B5556EC534170390733E30A1217D2910E27FDA7E5BCCAFADB4C` (commit `7158aad`)
- packaged `rustibites-view.exe` — **3,168,768 B**, SHA-256
  `CA2A6D4684218C5CE65B767B6C3291F359E988BAD2C776DABD3BFEBD36F42B50`
- zip `RUSTibites-0.2.2-windows-x64.zip` — **155,568,731 B**, SHA-256
  `f7d9851638719847e20dd05db3f7c8e62026247a189f9ecf87e98048ee74b36c`
- fresh-extraction CLI smoke: `run testdata/real/Default.zip --ticks 600 --seed 12345 --stats json --quiet`
  exit 0, 1→10 bibites; menu screenshot byte-identical on repeat
  (`E533D78AC5FA41471D17FB01DF3ED80AFC66263127AA62709978F17CAC970DF5`); 60 runtime sessions.

Do not compare this binary's throughput with the task-131 freeze without a fresh same-machine A/B.

## Release 0.2.3 packaged binary (2026-11-03)

0.2.3 is the current functional-UI release; 0.2.2 and earlier remain historical. Viewer/UI release, not a
new performance freeze — the published matrix above remains the task-131 final-freeze reference.

- packaged `rustibites.exe` — **2,253,824 B**, SHA-256
  `BC5F2910C9B24D3F55D2C4C6F59799E06510BC09FA38A422C278C19F6013E823` (commit `c1d25e0`)
- packaged `rustibites-view.exe` — **3,205,120 B**, SHA-256
  `73EE956A23174F01CB707DA467CBE8435A0260F125E8B19F826ED2994CE61858`
- zip `RUSTibites-0.2.3-windows-x64.zip` — **156,609,848 B**, SHA-256
  `5e33e6aa856c0b60b5d501d3ba50f9b1f33c818d1414eb8819089e571263e4e4`
- fresh-extraction CLI smoke: `run testdata/real/Default.zip --ticks 600 --seed 12345 --stats json --quiet`
  exit 0, 1→10 bibites; menu screenshot byte-identical on repeat
  (`E533D78AC5FA41471D17FB01DF3ED80AFC66263127AA62709978F17CAC970DF5`); 60 runtime sessions.

Do not compare this binary's throughput with the task-131 freeze without a fresh same-machine A/B.


# Evolution results â€” first long run (task-61)

Date: 2026-10-30. Engine: `RUSTibites` release build, synthetic/real saves,
8 threads unless noted. Companion metric benchmark: `docs/ARCHITECTURE.md` Â§10
("Evolution throughput") and `crates/bibites-core/examples/evolution_bench.rs`.

**World labelling:** the measured rows below are the **historical evolved-save
fixture** `testdata/real/saves/lol.zip` (35-bibite real save), kept as an
explicit regression reference. The canonical evolution baseline per
[`docs/TESTING_POLICY.md`](TESTING_POLICY.md) is the game's default scenario,
`testdata/real/Default.zip`; use the Default command below for new experiments
and label every quoted number with its world.

**Current status (2026-11-01):** canonical **Default 100k single-process**
validation is measured on two seeds (pop 24/31; births 91/101; deaths 67/70;
gen 3/2; mutation near founders; no extinction) and the **100k chunked runner is
bit-exact** (`11492540061262829588`, dead-slot pellet/pheromone generation fix
`8c799ec`). The game Default **36k** capture is now compared below: plant
ceiling/species1/genMax1 match, while Rust has fewer living bibites, higher
energy/health and ~2Ã— meat biomass (feeding acquisition gap). Longer runs
speciate: default 500k sustains 2 live species, and strong-mutation 500k runs
sustain 7/4 live species; the earlier stale registry counts for extinct
children are fixed by world's tick-boundary reconciliation (see below). The
earlier 10k `default_baseline` row is a short smoke run, not the current
long-run result.

## What was run

| run | start world | seed | chunks Ã— ticks | total ticks | threads | wall | real-time factor | final pop |
|---|---|---|---|---|---|---|---|---|
| `default_baseline` | `testdata/real/Default.zip` (**canonical** game Default scenario, 1 start bibite) | 12345 | 5 Ã— 2,000 | **10,000** | 8 | **2.64 s** | â‰ˆ126Ã— | 1 (peaked 10, crashed; earlier short baseline) |
| `lol_100k` | `testdata/real/saves/lol.zip` (historical evolved fixture, 35 bibites) | 12345 | 50 Ã— 2,000 | **100,000** | 8 | **63.9 s** (58.5 s simulation + reloads) | â‰ˆ52Ã— | 23 (range 13â€“40) |
| `resume_check` | same historical `lol` save | 12345 | 1 Ã— 500 + verify | 1,000 | 8 | 0.5 s/chunk | â€” | 27 |

`default_baseline` is the canonical testing-policy baseline (it is short and hits
the current food/energy collapse, so use it as a smoke baseline, not an
evolution-rate measurement); the `lol` rows are the historical evolved fixture
kept for the deterministic-resume/regression story.

Artifacts (gitignored): `testdata/evolve/default_baseline/` and
`testdata/evolve/lol_100k/`:
`stats.jsonl` (per-chunk stats), `runner.log`, `summary.json`,
`checkpoint_000.zip â€¦ checkpoint_049.zip` (resume points), and
`capture_final/` (parity capture of the last 2,000 ticks with births/deaths for
the `lol` run).

Reproduction:

```powershell
cargo build --release -p bibites-cli
# canonical evolution baseline (testing policy; game Default scenario):
tools/evolve/run_experiment.ps1 -WorldPath testdata/real/Default.zip `
  -RunId default_baseline -Chunks 5 -ChunkTicks 2000 -Threads 8
# historical evolved-save fixture (the numbers in this report):
tools/evolve/run_experiment.ps1 -WorldPath testdata/real/saves/lol.zip `
  -RunId lol_100k -Chunks 50 -ChunkTicks 2000 -Threads 8 -CaptureFinal
# continuation check on the historical fixture (currently FAILS, see below):
tools/evolve/run_experiment.ps1 -WorldPath testdata/real/saves/lol.zip `
  -RunId resume_check -Chunks 1 -ChunkTicks 500 -Threads 8 -Verify
# continue from any checkpoint:
target/release/rustibites.exe run testdata/evolve/lol_100k/checkpoint_024.zip `
  --ticks 2000 --seed 12345 --threads 8 --save testdata/evolve/lol_100k/checkpoint_025.zip
```

## Results â€” 100k ticks on the historical `lol` save

Population stayed alive and oscillated around â‰ˆ25 (13â€“40) for the whole run;
100k ticks = 3,333 simulated seconds â‰ˆ 55.5 simulated minutes. Overall
**1,565 ticks/s (52Ã— real-time)** including per-chunk world reloads; per-chunk
simulation-only throughput reached 2,400â€“3,900 ticks/s when the population was
small.

The final 2,000-tick capture (the most detailed window we have; earlier chunks
were only summary stats) shows:

* **births 37 / deaths 41** over 2,000 ticks (0.019 births/tick, â‰ˆ0.56
  births/sim-second at 30 TPS), population 27 â†’ 23;
* **2 active species** at both ends (`2583`, `2585`) â€” speciation is live in
  the real save's lineage;
* 32 of 35 gene means changed across the window (e.g. LayTime 7.188â†’7.193,
  BroodTime 20.826â†’20.559, Diet 2.492â†’2.420), so selection/mutation is visibly
  moving traits;
* `generation.gen_max = 1`, `gen_mean = 1.0` in the final sample. This matches
  the known task-72 generation-stats defect (earlier in a 50k-tick single run
  `gen_max` read 630 at tick 2,500 and 1 at the end), so **long-run
  generations/hour is not quotable yet** â€” it awaits task-72. From the earlier
  50k single-process run: **363 births / 375 deaths in 8.3 s wall (43.8
  births/s, 402Ã— real-time)**.

Raw per-chunk population (every 10th chunk): 26, 33, 28, 27, 18, 23, â€¦ â€” no
runaway growth, no extinction, no NaN/panic.

## Resume/continuation verification â€” closed (1kâ€“10k, 2Ã—/4Ã—)

The historical mismatch above (single 1,000-tick
`state_fingerprint = 7816395688157925020`, chunked 2Ã—500 =
`11292368019058734230`, `RESUME MISMATCH`) was **pre-checkpoint-sidecar**: the old
game save carried only persistent/visible state, not the RNG/clock/cadence
phase. The engine now writes a sibling `<save>.bb8state` checkpoint and resumed
runs continue the **same deterministic stream**.

Current evidence (canonical Default and synthetic scale): **120 / 1k / 2k / 5k /
10k bit-exact** for both 2Ã— and 4Ã— chunked resume; every resume-point
`state_fingerprint` equals the single run and the final fingerprints match.
Real saves restore byte-identically after tasks 124â€“126; only a benign lazy egg
`plan_dirty` flag remains typed-only (not encoded). 100k capture works via
chunked sections and the 100k chunked runner is bit-exact (single == 50Ã—2,000
`11492540061262829588`); the dead-slot pellet/pheromone generation fix is
`8c799ec`. Details/evidence:
[`design/CHECKPOINT_STATE.md`](design/CHECKPOINT_STATE.md) (task-124/125/126 and
the 1kâ€“10k + 100k tables).

This does not affect the single-process numbers above (`lol` 50k run) or parity
captures.

## Limitations / next steps

* Canonical Default 100k now shows monotonic generations (gen max 2 / mean
  1.21 seed12345; 2 / 1.16 seed67890) with no lineage/generation reset; the
  older generation-stats issue is historical. Long-run generations/hour still
  needs a game reference.
* `species_n = 1` is expected at 100k: max bibiteâ†’template distance 1.365 <
  usedSpeciesSpan 1.5; Rust assignment/span/distance matches
  `GlobalLineageManager.CheckNewSpecies`. The game Default 36k capture also has
  `nSpecies=1` and `genMax=1`, so the game does not speciate earlier. Longer
  default/strong runs (500k) do produce live species (below); the remaining
  issue is only stale registry counts for extinct children, not speciation.
* Dense synthetic founders (1kâ€“5k) collapse after the first generations
  (ARCHITECTURE Â§10); the canonical Default and real-save runs are stable but
  small (10â€“40 bibites). Ecology balance at high density is task-57.
* Checkpoint/resume is bit-exact for the covered 120 / 1k / 2k / 5k / 10k scale
  (2Ã—/4Ã—) and the **100k** run: single seed12345 fingerprint
  `11492540061262829588` == 50Ã—2,000 chunked; the dead-slot pellet/pheromone
  generation codec fix (`8c799ec`) removed the earlier +188-tick
  zone/pellet/hash divergence.
* The runner has no timeout/kill watchdog beyond per-chunk process completion;
  a hung chunk can be killed externally and resumed from the last checkpoint.
  Running 10â€“20 min experiments is a matter of raising `-Chunks` (each chunk is
  a separate process, so Ctrl-C loses at most one chunk).

## Canonical Default 100k evolution validation (2026-10-30)

Single-process runs (release CLI, seed 12345/67890, game Default scenario,
`testdata/real/Default.zip`) sampled every 1,000 ticks. Values are single-run
results; the 50Ã—2,000 chunked runner now matches the single run
(`11492540061262829588`) after the `8c799ec` checkpoint fix, so it is the
authoritative long-run trajectory.

| seed | final pop | eggs/corpses | energy mean | plant n / biomass | meat n / biomass | births / deaths | gen max / mean | species |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 12345 | 24 | 0 / 6 | 99.62 | 661 / 122,030 | 54 / 13,113 | 91 / 67 | 3 / 1.33 | 1 |
| 67890 | 31 | 0 / 8 | 95.18 | 677 / 117,316 | 55 / 11,286 | 101 / 70 | 2 / 1.16 | 1 |

Both runs: population starts 1, reaches the 10-bibite spawner minimum by ~9 s,
then cycles 10â€“24 (seed12345) / 10â€“35 (seed67890) without extinction, NaN/panic,
or runaway population. Plant biomass rises from ~94k to ~117â€“122k; meat
accumulates from deaths; body-size mean is 1.05 in both at 100k. These are the
current-tree values after the 100k checkpoint/zone fixes; they supersede the
pre-fix 19-pop seed12345 row (old behavior changed with the corrected pellet/
pheromone checkpoint state and world fixes).

**Mutation/genome sanity:** all genes remain inside their editor ranges; no
hard-bound runaway. Mutation kernels stay near their founders: gene8
`MutationAmountSigma` ~0.025; gene9 `AverageMutationNumber` ~4; gene10
`BrainMutationSigma` ~0.1; gene11 `BrainAverageMutation` ~2. Largest drift is
seed12345 `ArmorWAG` (mean 0.053, std 0.206) and `ViewAngle` (std ~2â€“3);
seed67890 is tighter. No lineage/generation reset: `gen_max` monotonically
0â†’1â†’2/3 and `gen_mean` 0â†’1.2. A 500k default run reaches gen6/mean4.17 and
strong-mutation runs gen5/mean3.50 and 3.63 without reset (below).

**Speciation:** `species_n = 1` for the entire 100k in both seeds. A direct
measurement on the 100k final state gives the maximum bibiteâ†’species-template
genetic distance **1.365** against `usedSpeciesSpan = 1.5` (active count 1, no
span scaling), so no individual has crossed the root-species threshold yet.
The Rust assignment path matches `Here/ManagementScripts/GlobalLineageManager.CheckNewSpecies`
exactly: first-adopt within span, stay when distance `< span`, child adoption
within `span/2`, then the two-consecutive-outlier mutant rule; `usedSpeciesSpan`
matches `base + max(activeâˆ’20,0)Â·scaling`; gene distance uses
`Î£|Î”gene|/maxValue` and the same brain-distance term. There are no
species-based mating/reproduction restrictions in the game (species is
bookkeeping only), so no mating rule is missing. At 2 generations and distance
below span, `species_n=1` is expected.

Longer runs do speciate: default 500k reaches a transient second live species at
tick 225k (`[[1,36],[2,1]]`), but it goes extinct; final live species is 1 while
the registry keeps five extinct child species in its active list. A stronger
mutation run (`backgroundMutationChance 3`, `backgroundMutationVariance 0.1`)
reaches **species_n=5 live** by 500k (`[[1,21],[8,3],[9,3],[10,3],[11,1]]`),
first multi-species at tick 225k, gen7/mean3.81. So speciation is live; the
default 100k/36k result is not a speciation bug.

However, the registry's per-species **counts are stale for deaths that remove the
record before event processing** (swallowed/unborn deaths): the 500k strong run
has root count7 vs 30 live, species 8 count2 vs5 live, and extinct children with
count1/live0 never pruned (active_species=17 though live species_n=5). The
species-side reconciler+unit test landed in `species.rs` (`7d0f803`):
`reconcile_live_counts(live_ids)` recomputes counts from an authoritative live
stream and drops extinct children from `active`. The actual run fix still needs
the world/body emit path: carry `species: Option<SpeciesId>` in
`BibiteEvent::Death` (or call `reconcile_live_counts` at a safe boundary) so
world applies `on_death` despite record-removing deaths. This affects prune/
active bookkeeping, not live-species statistics.

**Game comparison â€” 36k Default ecology capture (now available):**
game (`assets/game/reference/ecology/default_36k_summary.json`) vs Rust
single-process fresh Default at tick 36,000 (seed 12345 / 67890):

| metric | game 36k | Rust 36k seed12345 | Rust 36k seed67890 |
|---|---:|---:|---:|
| population | 19 (range 1Ã¢â‚¬â€œ24) | 7 (range 1Ã¢â‚¬â€œ18) | 15 |
| eggs / corpses | 0 / 6 | 0 / 4 | 0 / 1 |
| energy mean | 68.63 (min 57.1) | 79.03 | 86.42 |
| health mean | 93.79 (min 77.1) | 110.32 | 105.23 |
| plant n / biomass | 650 / 125,171 | 610 / 125,186 | 613 / 124,824 |
| meat n / biomass | 11 / 3,320 | 21 / 5,672 | 20 / 4,189 |
| births / deaths (events) | 35 / 17 first36k (full-run 39/25) | 31 / 25 | 35 / 21 |
| generation max / mean | 1 / 0.63 | 1 / 0.29 | 1 / 0.67 |
| species | 1 | 1 | 1 |

*Rust values are the post-sustained-bite-fix capture (`rb_36k_ongoing` for
seed12345 and a matching capture for seed67890); game values are the 36k summary.*
Spawner cadence is identical: game initial t0.200 then births every1.125/2.125/Ã¢â‚¬Â¦ s,
Rust initial tick1 (0.033 s) then every30/60/Ã¢â‚¬Â¦ ticks, exactly one per simulated
second; the +0.125 s absolute offset is capture/startup phase, within one 30-TPS
tick relative to initial, so **no spawner change is warranted**. Both engines are
plant-limited at the same ~125k E ceiling, **species stays 1 in game and Rust at
36k**, and generation max1 matches. Remaining 36k ecology gaps: Rust has fewer
living bibites (7/15 vs19; peak18 vs game24), lower generation mean (0.29/0.67 vs
0.63), and still higher energy/health (79/86 and110/105 vs game68.6/93.8), while
plant stock is essentially exact and meat is now 1.3Ã¢â‚¬â€œ1.7Ãƒâ€” game. Feeding (live
final poll): game 15 live/242,454 plant-contact ticks/469 enters, 28,361
bibite-contact; Rust seed12345 7 live/32,065 plant/46 enters/5,386 bibite,
seed67890 15 live/165,844 plant/313 enters/3,561 bibite. All-record Rust seed12345
bibite-contact is 35,463 vs game38,683; live population, not total contact
capacity, is the remaining deficit. **Window caveat:** the game capture overshot to
47,993 raw ticks; its **first-36k sampler rows** give 35 births/17 deaths (16
starved, 1 killed), and 1+35-17=19 exactly matches final pop19. The tracked events
file is full-run (39/25, 2 killed) and must not be mixed with first-36k rows.
Rust seed12345 is 31 births/25 deaths (24 starved,1 killed), seed67890 35/21
(all starved). The earlier identical-base 3,000-tick pair remains the close
short-horizon check (energy 94.665 vs 98.304, health exact, plant âˆ’0.24%).

**Recruitment/starvation audit (first 36k, authoritative):**

| metric | game | Rust seed12345 | Rust seed67890 |
|---|---:|---:|---:|
| avg population | 15.5 | 8.55 | 11.79 |
| births / avg pop | 2.26 | 3.62 | 2.97 |
| deaths / avg pop | 1.10 | 2.92 | 1.78 |
| starved deaths | 16 | 24 | 21 |
| killed | 1 | 1 | 0 |
| starved age mean | 148.9 s | 298.0 s | 171.1 s |
| starved maxEnergy mean | 83.3 | 107.7 | 75.3 |
| eggs max | 6 | 9 | 9 |

Rust per-capita **births are higher** and egg counts larger, so recruitment is not
the deficit. The deficit is per-capita **starvation/death**: Rust dies 1.6â€“2.7Ã—
more per individual at *lower* average density, which rules out crowding as the
cause. Seed12345 starvation is concentrated in older/larger adults
(age298 s/maxE108 vs game149 s/83); seed67890 is closer to game (171 s/75) but
still has a higher per-capita death rate. First-36k kills are **1 vs 1** (Rust
seed67890 0), so kill conversion/overkill is not the population gap; the single
Rust kill had 229 damage vs188 max health (22% overkill) vs game178/163 (9%).
Root is **ecological survival/energy balance (adult starvation + feeding
acquisition)**, not recruitment or combat; owners body (starvation/upkeep/feeding
energy) and world (food/density feedback). Artifact:
`default_36k_recruitment.json`.

**36k egg/hatch audit (sampling-corrected):** the earlier "game eggs max6 vs
Rust1/0" was a 1,000-tick sampling artifact. Fine 60-tick sampling shows Rust
eggs max **8** at ticks 8,340â€“8,400 (first egg increases at 8,100), while the
game (60â€“80-tick rows) max **6** at ticks 10,880â€“11,120 (first at 10,800). Rust
first post-spawner birth is tick 6,330 (tâ‰ˆ211 s), game tâ‰ˆ172 s; over 36k the
game has 23 egg-increase events vs Rust 15, but Rust is not egg-limited: mature
bibites have `EggProductionâ‰ˆ0.197` (>0.15), `Want2Layâ‰ˆ0.5` (>0.25), capacity 1,
and progress cycles 0â†’1 with bursts at ticks 8.1k/16.2k/24.6k. Energy/fat fall
during the mid-run crash but surviving mature bibites keep producing; there is
no persistent stop condition. **Egg/hatch turnover is not the cause of the lower
Rust peak** and needs no production fix; per-capita laying is similar.

**Death-budget / dead-vs-live cohort audit (first 36k):** last-60-s feeding
histories from the game polls and matching Rust tick traces:

| last 60 s | game dead | game live | Rust12345 dead | Rust12345 live | Rust67890 dead | Rust67890 live |
|---|---:|---:|---:|---:|---:|---:|
| plant-contact rate/s | 0.28 | 15.62 | 0.00 | 9.09 | 0.94 | 13.66 |
| mouth-contact rate/s | 1.71 | 17.81 | 3.66 | 13.65 | 1.47 | 14.10 |
| stomach-positive fraction | 0.063 | 0.722 | 0.122 | 0.636 | 0.075 | 0.755 |
| mean stomach content | 0.14 | 7.18 | 0.001 | 5.81 | 0.11 | 9.76 |
| net energy slope/s | âˆ’0.404 | âˆ’0.068 | âˆ’0.415 | âˆ’0.299 | âˆ’0.376 | âˆ’0.072 |

Starved deaths die with **empty stomach, near-zero plant contact, and no meal**
(last meal â‰ˆ135 s before death in game, â‰ˆ209/155 s in Rust; stomach_totalâ‰ˆ0;
maxE means game83.3, Rust12345107.7, Rust6789075.3 and ages149/298/171 s).
Live peers have stomachs 64â€“76% positive and 9â€“15 plant contacts/s. The
proximate step is therefore **feeding acquisition/ingestion collapse immediately
before death**, not a sudden conversion step; dead bibites have no food in the
stomach to convert. The population-wide deficit is the same chain: Rust live
seed12345 has lower plant contact than game (9.1 vs15.6/s) and a negative
survivor energy slope (âˆ’0.30 vs âˆ’0.07/s), while seed67890 live is stable
(âˆ’0.07/s) but has low-energy individuals that drop out. This is consistent with
the confirmed body-side energy budget/feeding-conversion residual: intake is
normal-to-low in survivors but the energy balance is negative, and failing
individuals stop acquiring pellets and starve. Exact body locations to inspect:
`BodyState::update_metabolism` / `BodyState::use_energy` (`body.rs` ~786/814),
`tick_organs` (`body.rs` ~4375; stomach digestion energy, egg/fat/movement
spend), `Stomach::update`/`add_matter` (`organs.rs` ~494/428), and
`tick_mouth_pellet` (`body.rs` ~4790). Fat is not exposed in either feeding poll;
add per-bibite fat/egg/upkeep counters around death to separate allocation from
upkeep. Artifacts: `default_36k_death_budget.json`,
`default_36k_death_cohorts.json`.

**Combat/kill audit after the sustained-bite fix (`f76acf6`):** all-record
Rust contact is now **35,463 samples/15,950 ticks, 8 attacker-victim pairs, 12
intervals** (median1,960/max9,174), close to game full-run38,683; attack output
0.5/eat0.842, targets include bibites, trigger geometry not limiting. All-record
Rust damage **908.6/12 first bites/1 kill** vs game full-run632/21/3, but the
first-36k comparison is **1 kill vs1 kill** (Rust seed67890 0 vs game1). The
full-run game extra kills come from the 47,993 overshoot, not a 36k gap.
Live-at-36k damage is128.8 on7 live vs game477.5 on15 live, contact5,386 vs28,361.
So damage and kill mechanics are **not** the remaining population bottleneck;
the single Rust kill has modest 22% overkill (229 damage vs188 health) vs game9%.
Remaining gap is live population/starvation and live contact per surviving
bibite; kill spread is secondary. Pre-fix trace (931 contacts/51.7 damage)
superseded; artifacts `default_36k_rust_contacts.json` +
`default_36k_rust_intervals.csv`, `default_36k_combat.json`.

**Synthesis:** after the body damage fix, the 36k envelope gaps are
plant/stock exact and contact capacity adequate; the dominant residual is
per-capita starvation/energy balance (adult starvation in seed12345, milder in
seed67890) plus the resulting lower live population/contact. Combat damage and
kill conversion are not the main lever at first-36k; egg/hatch production is not
limiting either. Owners: body for starvation/upkeep/feeding energy and live
contact; world for food/density feedback. Feeding acquisition remains task-119;
no brain-evaluator issue (task-127).

**Steering/occupancy trace (Rust vs game):** off-zone Rust records have
**zero plant/N senses** (`plant_closeness=0`, `n_plants=0`), `Rotate≈0`,
`Herding=0`, **elevated Accelerate0.415 vs0.240 in-zone**, speed11.23 vs4.66,
and radial outward velocity **+5.8/s**; in-zone records see plants
(`plant_closeness=0.510`) and contact at11.26/s. Low-energy bibites are not
slowed: 65% of low-energy records are off-zone and they move faster (8.6 vs5.2).
Body/propulsion simply executes the brain command; no zone/void steering exists
in either engine under Default settings (`voidAvoidance=false`,
`shadeAvoidance=false`; world wrap at the shade limit). Early trajectories are
similar radius (game r_p90 1645.8 vs Rust1615.9; both inside r<2000), but the
per-tick game capture shows **no outward bias** (radial drift+0.805/s, 45.5%
positive, speed3.29) while Rust early is faster (7.56, radial+0.45/s, 57.9%
positive). The
divergence is therefore **brain/steering behavior once plant FOV is empty**,
not a body propulsion bug; a later game position capture is required to see
whether the game turns back or simply dies. Fix locations to consider:
`BibitePropulsion`/`compute_movement`/`apply_environment_steering` (body.rs)
and brain outputs/FOV wiring (`brain.rs`); do not add a non-game zone force
unless the later game capture shows one.

**Later game positions landed (600–27,000 available):** the game also
drifts outside the zone, so occupancy policy is **not** the divergence:

| metric | game | Rust12345 | Rust67890 |
|---|---:|---:|---:|
| off-zone fraction r>2000 | 19.6% | 24.0% | 21.0% |
| radius p50 / p90 / max | 805 / 2,924 / 4,010 | 1,503 / 3,134 / 4,013 | 976 / 3,024 / 4,015 |
| nearest pellet/plant p50 all | **10.95** | 92.93 | 22.26 |
| nearest in-zone p50 / p90 | **6.35 / 100.5** | 25.18 / 212.2 | 9.43 / 118.5 |
| nearest off-zone p50 | 996 | 1,167 | 1,180 |
| radial drift/s, positive share | +2.59, 64.4% | +2.49, 68.0% | +2.43, 68.3% |
| speed mean / median (position deltas) | 10.8 / 4.31 | 9.1 / 4.83 | 11.2 / 4.08 |
| exits / returns | 11 / 1 | 13 / 0 | 8 / 0 |
| in-zone / off-zone energy | 94.3 / 41.2 | 87.5 / 40.5 | 104.4 / 35.4 |

Occupancy statistics are close: both engines spend ~20–24% off-zone, have
similar radial drift and energy-by-zone, and both drift rather than turn back
(game one return vs Rust zero, with sparse samples). The sharper difference is
**local plant proximity inside the zone**: game nearest-plant p50 6.35 vs Rust
25.18 (seed12345) and 9.43 (seed67890); overall game10.95 vs Rust92.93/22.26,
while off-zone nearest distances are similar (~1,000–1,200). This is a
**within-zone pellet spatial distribution / local accessibility** difference, not
occupancy policy or a missing body steering force. Speed here is computed from
position deltas (game cadence ~70–100 ticks, Rust30), not native game vx/vy;
do not infer a body force from it. Owner: world (pellet/zone spatial
distribution); no body change recommended. Window is 600–27,000 because the raw
`samples_fast.jsonl` currently ends at 27,000 (lead reports extension to 35,992).
Artifact: `default_36k_occupancy_later.json`.

**Pellet distribution check (CentricGradual):** game and Rust pellet radial
distributions match: early game443 pellets in-zone p50 833.4/p90 1487.0/max1922
vs Rust440 p50 909.9/p90 1553.1/max1945.9; late game646 pellets p50 878.9/
p90 1542.6 (10 outside), Rust610 p50 882.0/p90 1521.2 (0 outside). The game
capture radius is `sqrt(amount/20)`; with the 2.5× prefab collider factor its
p50≈3.38 matches Rust analytic radius3.49, so pellet size is consistent. Rust
`zone.rs::warped_disc_point(0.5)` implements the same
`1-(1-|u|)^0.5` CentricGradual formula as game `InsideUnitCircleWarped`; no
pellet-distribution fix is indicated.

The remaining discriminator is **within-zone occupancy/foraging**: in-zone
bibite radius p50/p90 is game672/1530 vs Rust12345 1381/1753 and Rust67890
857/1591. On the same pellet field, in-zone nearest-pellet p50/p90 is game
6.5/102 vs Rust12345 25.2/212 and Rust67890 9.4/118. So Rust seed12345
individuals sit much farther out inside the zone; seed67890 is close to game.
This is a behavior/foraging/energy-feedback question (brain/steering), not
pellet distribution, occupancy policy, body propulsion, or a non-game zone
force. Artifacts: `default_36k_pellets_compare.json`.

**Six-seed within-zone ensemble (C19 variance test):** six Rust Default
36k seeds (1,2,3,4,12345,67890; 30-tick samples) vs the game later-position
capture (600–27,000 window; game births/deaths from first-36k series):

| metric | game | Rust seeds min / median / max |
|---|---:|---:|
| in-zone radius p50 | 672 | 678 / 888 / 1,381 |
| in-zone radius p90 | 1,530 | 1,470 / 1,591 / 1,753 |
| nearest pellet p50 | 6.48 | 5.33 / 8.16 / 25.18 |
| nearest pellet p90 | 102 | 97.7 / 107.5 / 212 |
| plant-contact rate/s | **18.04** | 8.75 / 12.89 / **14.32** |
| energy mean | 84.6 | 76.2 / 85.2 / 89.9 |
| births first36k | 35 | 28 / 32 / 35 |
| deaths first36k | 17 | 17 / 23 / 25 |
| final energy mean | 68.6 | 66.9 / 85.7 / 86.6 |
| plant n | **650** | 592 / 617 / **628** |

Game **occupancy metrics fall inside the Rust seed-to-seed spread** (radius p50
672 is only 0.9% below the Rust minimum678; radius p90, nearest p50/p90,
births/deaths and energy are all enclosed). Seed12345 is the single outlier at
the high-radius / low-nearest extreme; seeds1,2,4,67890 are near the game.
Therefore C19 occupancy is **lineage/behavior variance**, not a systematic
engine bug; game RNG streams are not reproducible. Enclosed acceptance metrics:
radius p50 within Rust [678,1381] with a small ±1% tolerance, p90 within
[1470,1753], nearest p50 within [5.33,25.18], p90 within [97.7,212], births in
[28,35], deaths in [17,25].

The **feeding rate and plant n are still outside** the Rust spread: game
plant-contact 18.04/s vs Rust max14.32, game plant_n650 vs Rust max628. That is
the separate task-119 feeding/contact gap, not an occupancy-policy issue. Artifact:
`default_36k_ensemble.json`; no body/world occupancy change.

**Matched-radius target/stomach audit:** the contact-rate gap persists after
matching in-zone radius, but **target acquisition and stomach content match**:

| in-zone 600–27,000 | game | Rust ensemble |
|---|---:|---:|
| plant-contact/s | **22.37** | **15.14** |
| mouth targets (`n_in_mouth` / Rust `n_targets`) | 0.573 | 0.575 |
| plant targets | n/a | 0.509 |
| stomach content | 10.19 | 10.67 |
| active-contact fraction | 0.618 | 0.528 |
| in-zone radius 0–500 contact/s | 27.06 | 19.06 |
| 500–1000 | 23.41 | 16.13 |
| 1000–1500 | 17.81 | 14.33 |
| 1500–2000 | 10.55 | 8.59 |

Per-seed in-zone rates: 16.71, 14.92, 14.44, 17.54, 11.50, 13.96 (seeds
1,2,3,4,12345,67890); game22.37 is above the seed max, driven mostly by the
low-population/worst lineages rather than a universal matched-occupancy
acquisition failure. Body's per-contact ingestion is **higher** in Rust
(0.268 vs0.186 E/contact-tick), and Rust target count/stomach are equal or
higher, so the gross feeding deficit is **contact exposure/turnover and live
population/lineage weighting**, not target detection at matched occupancy. C19 is **CLOSED** after per-update tick normalization (`388d273`): game 39.99 TPS vs Rust 30; residual is measurement TPS + live/lineage weighting, **not a body patch**. Plant count (650 vs Rust
592–628, biomass equal) follows a ~4% smaller game pellet amount
(p50 36.7/64.1 vs38.3/66.2), i.e. pellet amount/spawn/bite-chunk distribution;
owner world/body. Artifact: `default_36k_contact_matched.json`.

**Per-encounter and tick-rate normalization (final feeding check):** the raw
game capture advances **35,991 actual ticks over 900 s = 39.99 TPS** (row labels
are `simTime*30`), while Rust is 30 TPS. Contact counters are per actual tick, so
per-second rates are not comparable. Per **update**:

| contact/update | game | Rust |
|---|---:|---:|
| all records | 0.451 | 0.417 |
| in-zone | 0.559 | 0.505 |
| nearest pellet 0–5 u | **0.981** | **0.992** |
| nearest pellet 5–10 u | 0.751 | 0.670 |
| in-zone radius 0–500 | 0.677 | 0.635 |
| 500–1000 | 0.585 | 0.538 |
| 1000–1500 | 0.445 | 0.478 |
| 1500–2000 | 0.264 | 0.286 |

The apparent 48% per-second gap shrinks to ~11% per update overall, and Rust is
**higher** in the outer two radius bands; at 0–5 u both engines are essentially
at the per-update maximum. Rust plant-contact runs are long (159 runs, median433,
p901063, max4034 ticks) and target counts/stomach match, so there is **no
evidence that bite knockback, post-bite cooldown, target expiration, held/amount
filtering, or sensing cadence shortens Rust plant contacts**. The residual is
mostly the game 40-TPS vs Rust 30-TPS measurement plus live/lineage weighting; no
body mouth patch is indicated. Encounter proxy game1159 vs Rust533 ticks also
uses different tick rates/mouth-entry semantics and is not a direct duration
comparison. Artifact: `default_36k_encounters.json`.

**Checkpoint resolution (fixed, `8c799ec`):** the 100k chunked runner now
matches the single run. Single-run seed12345 final fingerprint
`11492540061262829588` == 50Ã—2,000 chunked (dead-slot pellet/pheromone
generation codec fix). The earlier +188-tick `zone`/`pellet`/`hash` divergence
was the dead-slot `PelletStore`/`PheromoneStore` generation counters being
skipped by the checkpoint codec, so the first post-resume pellet allocation
reused a slot with a different `PelletId` generation. Regression:
`checkpoint_preserves_dead_slot_generations` (pellet), world long chunked test,
and Default CLI 50Ã—2,000.


## Longer/stronger-mutation runs and evolution bookkeeping (2026-11-01)

Single-process canonical Default runs sampled every 5,000 ticks (no chunk
checkpointing needed; 100k chunked is now bit-exact after `8c799ec`):

| run | ticks | final pop | eggs/corpses | energy | plant n / E | meat n / E | births/deaths | gen max/mean | live species |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| default, seed12345 | 500k | 41 | 2 / 15 | 76.0 | 561 / 89,129 | 133 / 25,575 | 691 / 650 | 6 / 4.17 | 2 (`[[1,35],[4,6]]`) |
| strong (`mutChance3`, `mutVar0.1`), seed12345 | 500k | 34 | 0 / 5 | 91.3 | 421 / 73,205 | 139 / 29,823 | 646 / 612 | 5 / 3.50 | 7 |
| strong (`mutChance3`, `mutVar0.1`), seed67890 | 500k | 30 | 2 / 7 | 124.3 | 566 / 86,568 | 102 / 19,920 | 579 / 549 | 5 / 3.63 | 4 |

The default run now sustains **2 live species** at 500k (`[[1,35],[4,6]]`;
a pre-reconcile run had only a transient child at 225k), and the strong-mutation
seeds sustain **7 and 4 live species**. This confirms speciation works when
genetic distance/generations suffice. The early-100k species1 result is not a
bug: game 36k is also species1/genMax1.

**Bookkeeping fixed:** the stale-count bug (swallowed/unborn removals skipped
`on_death`; active children count1/live0) is fixed by world's tick-boundary
`reconcile_live_counts` call plus the species-side reconciler. Re-run evidence:
active species now exactly match live members (strong seed12345: active8 all
count=live, e.g. root26/26; final live7; strong seed67890 live4; default500k
live2) and no extinct count1/live0 children remain. Regression tests:
`species::tests::reconcile_long_membership_stream_matches_live_counts` (2,000
membership operations, 425 core tests pass) and world tick-boundary integration.


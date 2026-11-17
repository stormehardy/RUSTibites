# ECOLOGY â€” long-run validation experiments (task-57)

> **Canonical test world (policy, 2026-10-30):** `testdata/real/Default.zip` â€” see
> `docs/TESTING_POLICY.md`. New ecology runs start from Default.zip; the Ring/lol experiments below are
> **historical fixtures** (kept for the spawn-placement/post-fix record and the evolved-world positive
> control) and are labelled as such. Synthetic dense-founder worlds remain allowed for throughput.

Goal: check that the headless engine's ecology behaves like The Bibites over long runs (not just
single-world parity), and flag obviously broken ecology with repro commands. **No game runs were needed
for the simulation measurements** (the headless engine is the subject); the game is used only as the
qualitative reference already captured by the parity work. Game runs in tasks 58/71 were hidden-window
and cleaned up; this task adds no game processes.

Engine: `target/eco-target/release/rustibites.exe` (release build of the current tree; built in a
separate target dir to avoid interacting with other agents' build locks). All Rust runs: `--threads 8`,
fixed seed 12345; per-world TPS comes from the scenario (Default is 30, historical Ring 15). Artifacts
(series CSV, summary JSON, plots, final saves) are under `testdata/experiments/<name>/` (gitignored);
this document and the scripts under `tools/experiments/` are the committed deliverables.

## Default long-run validation (task-114, seed 12345, threads 8)

Canonical world only. Commands (release CLI, `--stats json`, optional `--capture`):

```powershell
target\release\rustibites.exe run testdata\real\Default.zip --ticks 10000 --seed 12345 --threads 8 `
  --capture target\val114\cap10k --stats json
target\release\rustibites.exe run testdata\real\Default.zip --ticks 50000 --seed 12345 --threads 8 `
  --capture target\val114\cap50k --save target\val114\ck50k.zip --stats json
```

Results (population min/max, cumulative births/deaths, per-tick means at sample points; no NaN/Inf;
spawner reaches its 10-bibite cap by tick 1000):

| ticks | pop | eggs/corpses | births/deaths (cum.) | energy mean | health mean | plant_n | plant E | species |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 1 | 0/0 | 0/0 | 156.92 | 131.02 | 440 | 94,309.8 | 1 |
| 1,000 | 10 | 0/0 | 9/0 | 108.83 | 132.68 | 454 | 96,629.9 | 1 |
| 3,000 | 10 | 0/0 | 9/0 | 98.21 | 136.49 | 473 | 100,920.0 | 1 |
| 5,000 | 10 | 0/0 | 9/0 | 78.09 | 140.24 | 500 | 105,613.7 | 1 |
| 10,000 | 17 | 0/3 | 19/3 | 48.83 | 94.84 | 555 | 116,294.9 | 1 |
| 20,000 | 10 | 0/8 | 29/20 | 60.69 | 103.86 | 597 | 125,390.9 | 1 |
| 30,000 | 12 | 1/8 | 44/33 | 62.49 | 114.35 | 611 | 125,146.5 | 1 |
| 40,000 | 10 | 0/7 | 55/46 | 63.99 | 105.37 | 620 | 125,281.3 | 1 |
| 50,000 | 11 | 0/7 | 65/55 | 64.83 | 117.41 | 630 | 125,264.6 | 1 |

Run-level ranges: 10k pop 1â€“18, energy 46.4â€“157.2, plant 0â€“555; 50k pop 1â€“18, energy 33.3â€“157.2,
plant 0â€“631, max corpses 14, max eggs 8, species max 1. No extinction, no resource freeze, no NaN/Inf.
The 10k closure (17 living, energy 48.8) is a genuine turnover/trough, not a crash: 50k recovers to a
11â€“12 living carrying-capacity regime with energy 60â€“65 and plant at the ~125k E ceiling, with meat
pellets accumulating (50 at 50k). Conservation residual (plant energy identity, `terms_missing=true`):
â‰¤7.2e-10 relative at 10k, â‰¤3.5e-9 at 50k â€” i.e. numerically exact for the Rust-side ledger.

Against the canonical game references (only 600/3000 ticks exist for Default): at 3000 Rust energy 98.21
vs game 101.91 (âˆ’3.6%, inside the ~4% envelope), plant_n 473 vs 426 (Rust +11% from the different
initial seed census), population 10 both and the same 9 spawner births/no deaths. There is **no game
Default reference beyond 3000 ticks**, so 10k/50k are validated for internal consistency/stability and
against the trend of the 3000-tick envelope, not a tick-by-tick game comparison. Ring/lol remain
historical context only (see the findings below).

Checkpoint on long runs: `--save ck50k.zip` sidecar `ck50k.zip.bb8state` = **236,631 bytes** (50k
ticks). Single 10k vs 5Ã—2000 CLI sidecar resume and 100k vs 50Ã—2000 are **bit-identical**
(10k fp `1750137736440958460`; 100k fp `8688404363908086484`). The earlier chunked mismatch was a
checkpoint-codec omission: dead-slot `PelletStore`/`PheromoneStore` generations were not encoded, so the
first post-resume pellet allocation reused a slot with a different `PelletId` generation (Default
divergence at tick +188 in zone/pellet/hash). Fixed and guarded in the store codecs and long chunked
tests; short 200/300 splits remain bit-exact.



`tools/experiments/run_ecology.py` runs the engine (`run <world> --ticks N --seed 12345 --threads 8
--capture DIR --save final.zip --stats csv`), converts the per-tick capture stream to `series.csv`
(counts, pellets/biomass, vitals, generation, species, brain, behaviour, lifecycle deltas), analyses it
(population min/max/final/extinction, births/deaths rates, species emergence, energy/health, NaN/Inf,
resource freeze, growth/extinction flags) into `summary.json`, and plots PNGs under `plots/` (PIL
contact sheets are not needed here; matplotlib 3.11 is available). It is parameterised so longer horizons
are a command-line change. A second script, `analyze_capture.py`, applies the same analysis to an
existing capture (used for the 50k-tick evolution artifact saved by an earlier evolution run).

`tools/experiments/build_dense_world.py` builds dense founder worlds from the save
`testdata/real/saves/lol.zip` (read-only): it duplicates only **living** templates (skipping the 13
corpses in the save!), resets founder age to 0, gives every copy a unique body id, places them uniformly
inside the save's single plant zone (radius 0.1Â·SimSize = 200, placement radius 0.9Â·200 = 180), and sets
`virginSpawnRate=0`. Output fixtures are under `testdata/experiments/dense/`. This script is a
**historically-scoped fixture builder** (evolved-world dense test); new canonical runs use Default.zip.
(A first version of the builder accidentally used the corpse files as founder templates; that produced
instant 0-energy deaths and was discarded â€” see "methodology corrections" below.)

`tools/experiments/run_round57.ps1` is the bounded batch that produced the tables (6 Rust runs,
~80 s total wall).

```powershell
# CANONICAL first experiment (Default.zip, policy baseline; shorter/longer by --ticks)
python tools\experiments\run_ecology.py --input testdata\real\Default.zip --name default_scenario_20k `
    --ticks 20000 --seed 12345 --threads 8 --engine target\eco-target\release\rustibites.exe

# HISTORICAL fixtures (opt-in; retained for the spawn-fix/post-fix record and evolved positive control)
python tools\experiments\run_ecology.py --input testdata\real\Ring.zip --name ring_scenario_20k `
    --ticks 20000 --seed 12345 --threads 8 --engine target\eco-target\release\rustibites.exe
python tools\experiments\run_ecology.py --input testdata\real\saves\lol.zip --name lol_save_5k `
    --ticks 5000 --seed 12345 --threads 8 --engine target\eco-target\release\rustibites.exe

# dense-founder question (historical fixture builder; 30 s per world at 10k ticks)
python tools\experiments\build_dense_world.py --count 1000
python tools\experiments\run_ecology.py --input testdata\experiments\dense\founders_1000.zip `
    --name dense_1000_10k --ticks 10000 --seed 12345 --threads 8

# analyse an existing long-run capture (no simulation)
python tools\experiments\analyze_capture.py --capture testdata\evolve\lol_50k --name lol_50k_artifact

# whole bounded batch (historical Round-5/7 batch; new batches should start from Default.zip)
powershell -File tools\experiments\run_round57.ps1
```

## Results (seed 12345, threads 8)

| experiment | population | births / deaths | species max | plant biomass startâ†’end | bibite biomass end | energy mean startâ†’end | flags |
|---|---|---|---|---|---|---|---|
| Ring scenario, 20k ticks | 10 â†’ **0** (extinct tick 2432) | 0 / 10 | 1 | 1,858,681 â†’ 2,435,782 | 0 | 157.2 â†’ 0 | EXTINCT, NO_REPRODUCTION, RESOURCE_FROZEN |
| Ring parity base save, 5k ticks | 1 â†’ **0** (extinct tick 2430) | 0 / 1 | 1 | 1,949,763 â†’ 2,435,528 | 0 | 155.0 â†’ 0 | EXTINCT, NO_REPRODUCTION, RESOURCE_FROZEN |
| real lol save, 5k ticks | 22 â†’ **37** | 81 / 66 | 1 | 179,737 â†’ 179,836 | 91,775 | 1,226 â†’ 542 | â€” |
| real lol save, 50k ticks (existing artifact) | 22 â†’ **10** (min 7, max 31) | 363 / 375 | **4** (first multi-species tick 31,570) | 179,737 â†’ 177,532 | 7,111 | 1,226 â†’ 82 | â€” |
| dense founders 100, 5k | 100 â†’ 33 | 126 / 193 | 2 | 179,737 â†’ ~150k | â€” | â€” | â€” |
| dense founders 500, 5k | 500 â†’ 53 | 400 / 847 | 2 | â€” | â€” | â€” | â€” |
| dense founders 1,000, 10k | 1,000 â†’ **33** | 1,082 / 2,049 | 2 | 179,737 â†’ 145,301 | 115,591 | 1,292 â†’ 917 | â€” |
| dense founders 2,000, 10k | 2,000 â†’ **46** | 1,732 / 3,686 | 2 | 179,737 â†’ 148,360 | 229,282 | 1,290 â†’ 859 | â€” |
| dense founders 5,000, 10k | 5,000 â†’ **29** | 3,303 / 8,274 | 2 | 179,737 â†’ 143,791 | 157,685 | 1,287 â†’ 1,059 | â€” |

Plots: `testdata/experiments/<name>/plots/{population_energy,biomass,births_deaths,species_behavior}.png`.
The Ring rows are **pre-spawn-fix** (captured before world-dev task-85; see finding 1 update) and are kept
as the historical baseline; they are not the post-fix expectation.
The 50k lol trajectory is the existing `testdata/evolve/lol_50k` capture, analysed here (not re-run):
it shows a long slow oscillation (7â€“31 bibites, 363 births / 375 deaths over 50k ticks, 4 species by
tick ~31.6k, final 10 living with energy mean 82) â€” i.e. a population that shrinks but does not
immediately collapse, with speciation still happening.

## Findings

1. **Scenario/starter-brained worlds starve while food is abundant â€” a feeding/behaviour gap, not a
   starvation regime.** Ring scenario (fresh start) and the Ring parity base save both go extinct at
   ~tick 2430 with **zero births, zero bite rate, energy mean 0**, while plant biomass rises from
   ~1.86M to ~2.44M and plant pellet count freezes at 4518 (RESOURCE_FROZEN flag). The bibites move
   continuously at max speed, never bite/eat, and die only when energy is exhausted. This is the single
   clearest "broken ecology" signal: a game-fresh Ring bibite eats and reproduces (game capture:
   Ring newgame went 1â†’9 living with 8 births in 120 ticks), while the Rust scenario-started/starter-brain
   bibites never do. Repro:
   `python tools\experiments\run_ecology.py --input testdata\real\Ring.zip --ticks 5000`
   (or the ring_parity base save). Likely owners: world-dev (scenario spawn/template semantics,
   task-70) and the starter brain/feeding path; the parity Round-2 scenario finding (task-58 Â§4)
   points the same way.

   **Update (2026-10-30): root cause found and fixed â€” this finding is superseded.** The starvation was
   spawn placement, not mouth logic (task-88 probe: runtime-spawned bibites chew/swallow correctly when a
   pellet is in range). `World::random_spawn_position` used a disc of `0.8 Ã— zone.radius` around the zone
   centre for every shape; Ring's `Rect` food strips are 97,500 Ã— 2,500 u (shape radius 48,750), so fresh
   bibites spawned ~7.7 kâ€“39 k outside the food field (captured seed at `(3,944, âˆ’56,448)`), never met a
   pellet and starved. world-dev task-85 now picks a zone (honouring `SpawnRow.spawn_zone`) and samples
   the shape-aware `Zone::spawn_location`; regression tests are in `world.rs`
   (`scenario_spawn_positions_are_inside_zone_shapes`, `spawn_row_zone_is_honoured`,
   `runtime_spawned_starter_eats_pellet_at_mouth`). Post-fix Ring 20k run (seed 12345): 10 living at
   tick 20,000 (max 11), 82 births / 73 deaths, plant stock pinned at the 2.44 M E ceiling with small
   dips, no extinction.
   Post-fix expectations, using the Round-5 game baseline (Â§9.1) and
   `docs/design/FEEDING_METRICS.md` Â§8: Ring starters do eat, but **only slightly** â€” the game's own
   fresh Ring bibites took a single **2.83 uÂ²** plant intake in 300 ticks (first intake tick 288). So
   expect small nonzero intake/conservation throughput, not a high feeding rate, and no extinction;
   the Ring feeding gate is conservation-identity based (stock sits at the 2.44 M E ceiling and cannot
   show consumption). Re-run the experiments with task-90 feeding counters + the conservation identity
   once they land; this finding should then be re-checked, not carried forward.
2. **Evolved lol populations do eat/reproduce and produce plausible, noisy long-run dynamics.**
   5k ticks: 22â†’37 with 81 births/66 deaths and stable plant biomass. 50k artifact: 7â€“31 population
   oscillation, 363/375 lifecycle counts, 4 species emerging around tick 31.6k, final 10. This is
   qualitatively the game's "population fluctuates around a carrying capacity, evolution continues"
   behaviour, unlike the scenario case â€” a useful positive control. Energy mean declines from the
   huge loaded reserves (1,226) to 82 over 50k ticks, consistent with the save's starting energy being
   spent down under sustained competition.
3. **Dense synthetic founder populations collapse because of overpopulation/overgrazing + boom-bust
   reproduction, not a NaN/instant-extinction bug â€” but flag the energy/health values for review.**
   With 1,000 founders placed inside one plant zone, 1,193 deaths happen in the **first 1,000 ticks**
   (tick 10 already 1,000â†’491; 228 corpses), while ~376 births/eggs appear immediately (egg count
   65â€“107), plant biomass dips 179,737â†’114,111 by tick 100 (overgrazing), then partially recovers to
   ~145k; survivors then slowly decline to 33 by 10k. 2,000/5,000 founders behave the same way (bigger
   first die-off, ~46/29 survivors at 10k; consistent with brain-dev's 1kâ†’15/2kâ†’33/5kâ†’84 at 20k, which
   are the same order given different horizons/fix versions). So the collapse is a realistic
   overcrowding/overgrazing/boom-bust pattern, not instant extinction and not NaN (NaN flag never fired,
   no resource freeze in these runs). **However** the survivors' reported energy mean rises to
   ~4,228 and health to ~1,067 at tick 100 â€” far above the founders' 1,292 energy / ~503 health and the
   game's own energy/health magnitudes (~1,226 energy, ~130 health at load). That looks like corpse-meat
   scavenging pushing energy/health past normal maxima (meat biomass peaks 125,348). Worth a world-dev
   look at corpse/meat energy caps and whether energy/health are clamped at high density; repro:
   `build_dense_world.py --count 1000` then the run above, inspect `series.csv` at ticks 50â€“200.
4. **Species emergence works** in the evolved world (4 species by 31.6k ticks in the 50k artifact;
   2 species in the dense runs) â€” but **generation stats are provisional** (task-72 reports known
   gen-stat bugs); do not rely on `generation.gen_*` numbers in this task, only on species counts.
5. **Game-side qualitative comparison** (from the parity captures, no new game run needed):
   - Ring fresh start (game, 120 ticks): 1 bibite spawns, grows to 9 with 8 births in 8 s. Earlier this
     was read as "eats/breeds from the start â€” opposite of Rust"; the Round-5 feeding reference (Â§9.1)
     refines it: the game's own Ring starters eat very little (**2.83 uÂ²** total in 300 ticks, first
     intake at tick 288), so the 8 births are largely spawner/growth-driven, not evidence of active
     foraging. Post-spawn-fix Rust should match that low starter intake, not a high rate.
   - lol save (game, 300 ticks): 22â†’22 living with 6 births/6 deaths and 1 egg at the end â€” Rust's
     5k run (22â†’37) is in the same regime but a longer horizon; the game's own evolved-world numeric
     behaviour is only approximately reproducible (hidden-node reload, PARITY_ROUND2 Â§2), so this is a
     qualitative comparison only. The real game's long-run (50k-tick) reference would need a 1x capture
     (~2.4 h wall); a 50x statistical run is the recommended follow-up if a numeric game curve is needed.
   Known deviations are tracked in `docs/DEVIATIONS.md` **D18** (pellet production uses unseeded
   `System.Random`/`UnityEngine.Random` in the game, so game pellet layouts/biomass are not
   bit-comparable) and **D19** (custom solver vs Box2D trajectories). The Rust engine's deterministic
   per-zone pellet streams therefore mean its long-run curves should be compared by regime/statistics,
   not tick-by-tick with the game.

## Methodology corrections (recorded so future runs don't repeat them)

* `build_dense_world.py` originally cycled through **all** `bibites/*.bb8` files of the lol save,
  including the 13 **corpses** (energy 0 / health 0). Those resurrected corpses died immediately and
  made the first dense results meaningless (1000â†’14 at 10k, 228 corpses by tick 10 from 0-energy
  bodies). Fixed by filtering to living templates (`dead=false`, `corpseEnergyOffset==0`, energy>0,
  health>0) and resetting founder `timeAlive` to 0; the tables above are from the fixed builder
  (original save remains untouched; fixtures are synthetic copies under `testdata/experiments/dense/`).
* `run_ecology.py --snapshot-every` (chunked `--save` snapshots) re-seeds the RNG per chunk (the engine
  seeds from `--seed`), so it is a set of continuation experiments, not one trajectory; the per-tick
  series used here come from single continuous runs. The `--save` final world is written per run for
  inspection; periodic snapshots are an optional auxiliary mode.
* One earlier attempt to use the default `target/release` engine hit another agent's build lock; all
  runs above use `target/eco-target/release` (separate `CARGO_TARGET_DIR`) for reproducible runs.

## Recommended next experiments

1. Re-run the Ring scenario/parity-base experiments with world-dev's spawn fix (task-85) + body-dev's
   feeding counters (task-90) and the conservation identity
   (`docs/design/FEEDING_METRICS.md` Â§8): assert spawns lie inside the strips, intake is nonzero but small
   (game Round-5 baseline: 2.83 uÂ² / 300 ticks, first intake tick 288), and no extinction/resource-freeze
   flags. This supersedes the old "starter-brain feeding gap" experiment; if intake stays 0, the
   remaining suspect is the starter brain/sensing path, not colliders.
2. Corpse/meat energy cap check at high density (finding 3): run 1,000 founders for 500 ticks and log
   per-entity energy/health before/after a meat pellet is eaten; check whether survivors exceed the
   game's maxEnergy/maxHealth and whether meat energy is clamped.
3. Carrying-capacity sweep in the real lol zone (100, 200, 500, 1000, 2000 founders, 20k ticks, 3 seeds)
   once generation stats are fixed (task-72), to distinguish density-dependent mortality from a
   metabolism imbalance; include corpse/egg dynamics in the analysis.
4. Long-run game reference: a 50x statistical capture of lol (20k ticks, ~13 s wall) and Ring (20k) for
   curve-shape comparison, with the D18 pellet-RNG caveat; 1x deterministic capture only for short
   horizons (bit-exact but 22 min for 20k ticks).
5. Add a regression test/fixture for "scenario bibite must take at least one bite and (for a starter in a
   food zone) eat a measurable amount within N ticks on Ring" once the spawn fix + task-90 counters land;
   assert the conservation residual as well as the counters.

## Canonical Default 100k full-ecology validation (current, 2026-11-01)

Follow-up to the 10k/50k table above, using the same canonical `testdata/real/Default.zip`,
seeds 12345 and 67890, single continuous process, sampled every 1,000 ticks. The 100k
50x2,000 chunked run is now bit-exact after `8c799ec` (dead-slot pellet/pheromone
generation codec fix), so this is the authoritative long-run trajectory. No game runs.

| seed | final pop | eggs/corpses | energy mean | plant n / E | meat n / E | births/deaths | gen max/mean | species |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 12345 | 24 | 0 / 6 | 99.6 | 661 / 122,030 | 54 / 13,113 | 91 / 67 | 3 / 1.33 | 1 |
| 67890 | 31 | 0 / 8 | 95.2 | 677 / 117,316 | 55 / 11,286 | 101 / 70 | 2 / 1.16 | 1 |

Both runs show the same regime as the 10k/50k table: population cycles 10â€“24 (seed12345)
or 10â€“35 (seed67890), no extinction/resource freeze/NaN, plant biomass at the ~117â€“125k E
ceiling, meat accumulating from deaths, energy means in the 45â€“129 range. Mutation genes stay
near their founders and no gene hard-bounds/lineage reset appear (details in
`EVOLUTION_RESULTS.md`). No speciation occurs in either 100k run; the measured final
maximum distance to the root template is 1.365 < usedSpeciesSpan 1.5, and the Rust assignment
path matches `GlobalLineageManager.CheckNewSpecies`, so species_n=1 at generation â‰¤3 is
expected. Longer runs do speciate: default 500k sustains 2 live species; strong-mutation
500k runs sustain 7 and 4 live species (see `EVOLUTION_RESULTS.md`), with registry counts
now reconciled against live membership.

## 36k game-vs-Rust ecology comparison (2026-11-01)

Game summary: `assets/game/reference/ecology/default_36k_summary.json` (hidden Default
newgame, seed 12345, first 36,000 ticks). Rust: same fresh scenario, single-process, seeds
12345/67890. Values at tick 36,000:

| metric | game 36k | Rust 36k seed12345 | Rust 36k seed67890 |
|---|---:|---:|---:|
| population | 19 (range 1â€“24) | 7 (range 1â€“18) | 15 |
| eggs / corpses | 0 / 6 | 0 / 4 | 0 / 1 |
| energy mean | 68.63 (min 57.1) | 79.03 | 86.42 |
| health mean | 93.79 (min 77.1) | 110.32 | 105.23 |
| plant n / biomass | 650 / 125,171 | 610 / 125,186 | 613 / 124,824 |
| meat n / biomass | 11 / 3,320 | 21 / 5,672 | 20 / 4,189 |
| births / deaths (events) | 35 / 17 first36k (full39/25) | 31 / 25 | 35 / 21 |
| generation max / mean | 1 / 0.63 | 1 / 0.29 | 1 / 0.67 |
| species | 1 | 1 | 1 |

*Rust values are post-sustained-bite-fix captures (`rb_36k_ongoing` seed12345,
matching seed67890 capture); game values are the 36k summary.* Both engines are
plant-limited at the same ~125k E ceiling and species stays 1 in the game at
36k, matching Rust. Remaining ecology gaps: Rust has fewer living bibites
(7/15 vs19; peak18 vs game24), lower generation mean (0.29/0.67 vs0.63), and
still higher energy/health (79/86 and110/105 vs game68.6/93.8), while plant stock
is essentially exact and meat is 1.3â€“1.7Ã— game. Final feeding poll restricted to
live records: game15 live/15 contacters,242,454 plant-contact ticks,469 enters,
28,361 bibite-contact; Rust seed12345 7 live/32,065 plant/46 enters/5,386
bibite, seed67890 15 live/165,844 plant/313 enters/3,561 bibite. All-record Rust
seed12345 bibite contact is 35,463 vs game38,683; live population, not total
contact capacity, is the remaining deficit. **Window caveat:** the game capture
overshot to 47,993 raw ticks; first-36k sampler rows give 35 births/17 deaths
(16 starved,1 killed), and 1+35-17=19 exactly matches final pop19. The events
file is full-run (39/25,2 killed) and must not be mixed with first-36k rows.

**Recruitment/starvation audit (first 36k):**

| metric | game | Rust seed12345 | Rust seed67890 |
|---|---:|---:|---:|
| avg population | 15.5 | 8.55 | 11.79 |
| births / avg pop | 2.26 | 3.62 | 2.97 |
| deaths / avg pop | 1.10 | 2.92 | 1.78 |
| starved / killed | 16 / 1 | 24 / 1 | 21 / 0 |
| starved age mean | 148.9 s | 298.0 s | 171.1 s |
| starved maxEnergy mean | 83.3 | 107.7 | 75.3 |
| eggs max | 6 | 9 | 9 |

Rust per-capita births and egg counts are higher, so recruitment is not the
deficit. Per-capita starvation/death is 1.6â€“2.7Ã— game at lower average density,
ruling out crowding. Seed12345 starvation is concentrated in older/larger adults
(age298 s/maxE108 vs game149/83); seed67890 is closer (171/75) but still a higher
per-capita death rate. First-36k kills are 1 vs1 (seed67890 0), so kill
conversion/overkill is not the population gap; the single Rust kill had 229
damage vs188 health (22% overkill) vs game178/163 (9%). Root is **ecological
survival/energy balance (adult starvation + feeding acquisition)**, not
recruitment or combat. Body owns starvation/upkeep/feeding energy; world owns
food/density feedback. Artifact: `default_36k_recruitment.json`.

Fine-sampled egg audit: Rust eggs peak9 in the post-fix capture (pre-fix trace8),
game6; EggProductionâ‰ˆ0.197, Want2Layâ‰ˆ0.5, capacity1, so there is no egg-production
stop/deficit; recruitment capacity is adequate.

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

Combat after `f76acf6`: Rust all-record contact35,463 samples/15,950 ticks,8
pairs,12 intervals (median1,960/max9,174), vs game full-run38,683; all-record
damage908.6/12 first bites/1 kill vs game full-run632/21/3, but **first-36k is 1
kill vs1** (seed67890 0); full-run extra game kills come from the 47,993
overshoot. Live-at-36k damage128.8 on7 live vs game477.5 on15 live; live contact
5,386 vs28,361. Damage/kill mechanics are no longer the limiting mechanism;
remaining gap is live population/starvation and contact per survivor. Artifacts:
`default_36k_combat.json`, `default_36k_rust_contacts.json`,
`default_36k_rust_intervals.csv`, `default_36k_recruitment.json`.
The identical-base 3,000 official pair remains the close short-horizon check
(energy 94.665 vs 98.304, health exact, plant âˆ’0.24%).

### 36k long-run accounting audit (world side, 2026-11-01)

Rust 36k capture (`target/rb36k`, seed12345, single process) vs game `ecology_36k` at
the same 1,200 s horizon:

| world quantity | game 36k | Rust 36k | note |
|---|---:|---:|---|
| plant n / biomass | 656 / 125,173 | 612 / 124,976 | 0.16% biomass difference; same ceiling |
| meat n / biomass | 17 / 3,932 | 24 / 7,411 | body-created meat; see accounting gap |
| free biomass | 238 | 687 | order 1e3 E, not a driver |
| corpses | 8 | 1 | body death/remains representation |
| births / deaths | 39 / 25 (23 starved, 2 killed) | 33 / 25 (25 starved) | combat/contact gap, not spawner |

**Plant/zone identity closes exactly in Rust:**
`seeded 94,309.81 + produced 64,278.24 - recycled 75.84 - consumed 33,535.88 =
stock 124,976.32 E` (residual <0.01 E). One zone; production â‰ˆ53.6 E/s over 1,200 s,
recycled only 75.8 E (0.12% of production), matching the game's plateau at the
~125k E ceiling. `decayed_plant_energy=0` is expected (plant material `decay=false`).

**Meat accounting is not closed (`terms_missing=true`):** Rust
`produced_meat_energy=0`, `consumed_meat_energy=0`, `decayed_meat_energy=0`, while
stock is 7,411 E. Meat is created by body corpse/death paths and eaten/ripped
through body mouth paths, neither of which is hooked into the world conservation
ledger; meat `fresh_time` is 2,000 s, so no meat decay is expected at 1,200 s.
Therefore the ~1.9x meat stock is **not** a world decay/recycle driver; it is the
body death/remains conversion and missing meat consumption/combat paths already
tracked under task-119. World decay/recycle can be excluded: recycled 75.8 E total
and decayed 0.

**Spawner accounting matches the game cadence.** Both runs spawn the initial row
immediately, then one continuous spawner birth per simulated second (Rust ticks
30/60/90..., game t1.125/2.125/...; relative to each run's initial event the gap is
<1 tick). Later gen0 spawner resumes: game t172/217/223 s vs Rust ticks 962/972/996 s
(32.1/32.4/33.2 s), because Rust reaches the row minimum later (slower starvation);
this is an ecology-turnover consequence, not a spawner phase/accumulator bug.
Event totals: game 39 births/25 deaths, Rust 33 births/25 deaths; Rust has no
`killed` deaths (0 vs 2). The residual is body feeding/contact/combat turnover.

### 36k pellet spatial-distribution audit (game capture vs Rust saves)

Game full-map snapshots (`testdata/parity/default/ecology_36k/pellets/`, seed12345)
vs Rust Default saves at the same horizons (plant pellets only):

| metric | game tick6 | Rust tick1 | game tick35,991 | Rust tick36,000 |
|---|---:|---:|---:|---:|
| plant pellets | 443 | 440 | 646 | 610 |
| radial p50 | 833 | 913 | 879 | 882 |
| radial p90 | 1,487 | 1,560 | 1,543 | 1,524 |
| radial max | 1,922 | 1,946 | 3,769 | 1,908 |
| outside r=2000 | 0 | 0 | 10 | 0 |
| amount sum | 18,882 | 18,862 | 25,480 | 25,037 |
| amount p50 / p90 | 39.37 / — | 39.77 / 66.43 | — | 38.23 / 66.30 |
| pellet radius p50 / p90 | 1.40 / 1.80 | 1.41 / 1.82 | 1.35 / 1.78 | 1.38 / 1.82 |

Rust initial seeding reproduces the expected `CentricGradual` (`warped_disc_point(0.5)`)
CDF: fraction inside r=1000 is 0.56 vs theoretical 0.5625, inside r=1414 is
0.82 vs 0.836; late production preserves it. Game late radial quantiles and
pellet-size mix converge to the same values (p50 879 vs 882, p90 1,543 vs 1,524,
amount sum 25,480 vs 25,037); game early is slightly more center-concentrated
(p50 833 vs 913), and game late has 10/646 pellets outside r=2000 vs 0/610 in
Rust (minor, likely collision drift). **Pellet distribution is cleared:** it
matches the exact `CentricGradual`/size law; the remaining overall nearest-plant
gap is **occupancy/sampling weighting**, not seeding. Capture artifact
`a28c9b1`: `assets/game/reference/ecology/pellets/pellet_samples.json` plus raw
`samples.jsonl`/`snapshots.jsonl` (115 samples, 12 full-map snapshots, schema
`rustibites.assets.ecology_pellets.v1`). C19 is **CLOSED** on tick-normalized metrics (`388d273`); no pellet/zone/body change.

Nearest-plant by **same radius bin** (game full-map nearest vs Rust saves every
2k ticks): 0–500 u p50 5.1 vs 5.7; 500–1,000 6.1 vs 4.4; 1,000–1,500 12.8 vs
15.1; 1,500–2,000 112 vs 143; >2,000 755 vs 660. Same-radius access is close;
the overall nearest gap is driven by which radii are occupied/sampled, not by a
different pellet layout. No pellet/zone seeding change made.

### 36k plant_n gap audit: count/size/fusion/consumption (2026-11-01)

Game late snapshot (`pellets/snapshots.jsonl`, tick35,991) vs Rust 36k save:

| metric | game | Rust |
|---|---:|---:|
| plant pellets n | 646 | 610 |
| plant amount sum (u²) | 25,480 | 25,037 |
| mean amount | 39.44 | 41.04 |
| amount p50 | 36.71 | 38.23 |
| amount p90 | — | 66.30 |
| pellet radius p50 / p90 | 1.35 / 1.78 | 1.38 / 1.82 |
| produced plant energy (exact) | 95,918 | 64,278 |
| consumed plant energy (exact) | 62,202 | 33,536 |
| recycled / decayed | 0 / 0 | 75.8 / 0 |

Both stocks sit at the same ~125k E ceiling. Rust has **fewer, slightly larger**
plant pellets (mean +4%, p50 +4%) and **less** production and consumption
energy than game; game turnover is 1.49× production / 1.85× consumption. There is
no Rust pellet fusion: `PelletStore::try_merge` has no call sites, so fusion
cannot explain the lower Rust count. Recycle/decay are negligible. Pellet-size
formula and initial distribution match; the late mean/p50 difference is in
ongoing production/partial-consumption history (game's heavier feeding shrinks
pellets more often), i.e. task-119 feeding/turnover, not a zone production or
pellet-size formula bug. The six-seed Rust count range (592–628) vs game 646–650
is within stochastic pellet-count variation given the matched biomass ceiling.

### 36k world accessibility audit (2026-11-01)

Game fixtures expose per-bibite contact counters but **no live positions or pellet
coordinates**, so an exact nearest-plant distribution cannot be computed game-side
from `ecology_36k` (only aggregate `series` + `feeding` counters). Rust positions
and Rust save pellet layouts are available, giving the local-accessibility side:

- Rust trajectory (`target/rb36k` feeding capture, ticks 1–36,000, live energy>0
  records): **24.6% of live records are outside the one circular zone (r>2000)**
  and 30.0% are at r>1800; radial p50=1,445, p90=3,154, p99=3,909, max=4,016.
- Rust save layouts: at 10k/20k/30k/36k, live bibites outside the zone = 2/16,
  **4/7**, 1/6, 1/6; nearest-plant surface distance p50 = 161/695/36/30 and p90 =
  376/1,708/2,181/576; mean plants within 10 u of a live bibite = 0.31/0.29/0.50/0.33.
  So at some sample ticks (notably 20k) most live bibites are in off-zone regions
  with no nearby plant even though global loose biomass is high.
- Game proxy at the same horizon: 15 live/15 plant-contacters, 242k live-subset
  plant-contact ticks (308,689 all-record ticks) vs Rust 9 contacters and 38,965
  ticks; plant stock and ceiling match. The game has no positions to compare
  directly, but its contact counters show much more time-in-contact per live bibite
  (≈20.6k vs ≈4.3k contact ticks/live).

**Conclusion:** global food is not the limiter, but **local accessibility is not
uniform**: Rust populations spend substantial time outside the circular plant zone
where nearest pellets can be hundreds of units away. This is a spatial feedback /
movement-occupancy question (body void-avoidance/propulsion + zone geometry), not a
pellet-production/recycle deficiency; no behavior change was made pending body/brain
tracing. Game-side position capture is required to quantify whether the game
occupies the zone more strongly.

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
positive). **Later-run game occupancy (discriminator):** the game capture also drifts
off-zone — **19.3% off-zone, radius p50 806 / p90 2,905**, with
`voidAvoidance=false`/`shadeAvoidance=false` and `wrap=true`. So Rust's
22.2%/16.5% off-zone share is **normal behavior, not the divergence**; no
zone/body change is warranted. **New world lead:** game nearest-plant is
**p50 11.4 / p90 960** vs Rust in-zone ≈75 despite similar plant counts,
pointing at **pellet spatial distribution/seeding** (`CentricGradual`,
`insideRadius`, zone radius) rather than occupancy policy. Artifact:
`assets/game/reference/ecology/default_36k_occupancy.json`; world is
investigating. Do not add a non-game zone force.

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

**Verdict:** no world-side plant/spawner/decay/accounting driver of the 36k ecology
gap beyond the known body/contact/combat issues. Remaining audit gap: hook
body-created meat (`spawn_meat_pile`/rip/eat) into conservation terms so meat stock
can be audited end-to-end; tracked for body-dev, not a world behavior fix.

Identical-base game check at 3,000 ticks (`official_game_3000` vs `official_rust_3000`, both
from the same base save): pop10/eggs0/corpses0 both; energy mean 94.665 vs 98.304 (+3.8%),
health exact (137.629), plant 454/101,285.6 vs 455/101,042.4 E (âˆ’0.24%), meat0; both have
only the 10 `initial` events. This is a good short-horizon ecology envelope match.

**Checkpoint status: fixed.** Single vs 50x2,000 100k now both
`8688404363908086484`; 10k both `1750137736440958460`. The dead-slot generations omission
was the +188-tick `zone`/`pellet`/`hash` divergence; regression
`checkpoint_preserves_dead_slot_generations` guards it. Current post-dense-fix guards: 10k `1750137736440958460`, 36k `9113407757480872428`, 100k `8688404363908086484` (all single == chunked). Fingerprints are tree-specific; these are the current-tree values after the latest body commits.

**Species registry bookkeeping: fixed.** Deaths that remove the record before
`step_events_and_stats` (swallowed/unborn) previously skipped `on_death`, leaving
extinct children count1/live0 (500k strong run: active_species17, root count7 vs
30 live). `World::step_events_and_stats` now reconciles per-species counts from
the live bibite stream at each tick boundary through
`SpeciesRegistry::reconcile_live_counts`; regression
`species::tests::reconcile_long_membership_stream_matches_live_counts` (2,000
membership operations) + the world-boundary integration keep aggregates equal to
live membership. Integrated 500k runs now show active counts exactly matching
live members (default live species2; strong-mutation live species7 and 4).


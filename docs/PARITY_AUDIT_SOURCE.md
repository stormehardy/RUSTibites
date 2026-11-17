# Source-level parity audit (C# `Here/` vs Rust) - 2026-11-03

Derived only from the decompiled C# and the Rust code, **not** from other docs (which are unreliable on parity).
Five read-only audits (body, mouth/stomach, senses/FOV, propulsion/organs, zones/pellets/settings). Items marked
"agent claim" were not independently re-verified by the maintainer session; verify against the C# before fixing.

## Fixed in this pass (verified against C# first)

| Item | C# | Rust |
|---|---|---|
| SoftLatch bias clamp [0.1,100] on load | `NEATBrain.cs:475` | `brain.rs::clamp_soft_latch_biases` |
| Mouth targets sorted by distance each tick | `BibiteMouth.cs:359-394` | `body.rs` before `tick_organs` |
| One bibite attack per tick (`attackedThisFrame` guard) | `BibiteMouth.cs:618` | `body.rs::tick_mouth` |
| Pheromone threshold and cost use strength-scaled sum | `BibitePheromoneOrgan.cs:70-80` | `pheromone.rs::emit_bibite` (+ test updated) |
| Brain input IsGrabbing = nHeld | `NEATBrain.cs:582` | `body.rs::is_grabbing` |
| `attackedDmg` decay was never called | `BibiteBody.cs:622` | `body.rs` before `tick_organs` |
| FOV candidate query: oriented box (<225 deg) / circle+margin, one shared 128 cap | `FieldOfView.cs:233-251` | `world.rs::fov_lookup_cache` |

## Open (agent claims, ranked)

**Body/energy**
- Collision damage: applied per manifold point per step instead of once per `OnCollisionEnter2D`; mass term uses other body (C# `otherRigidbody` is the receiver in Unity 2D - confirm); corpse burst deferred (`BibitePart.cs:47-66`).
- Energy params (`metabolic_efficiency`, health/energy ratios) cached at spawn, not refreshed on settings change.
- Lethal hit death deferred to end/next tick; organs/heal still run on a dying body (`BibiteBody.cs:767`).
- `delta_area` rounding `(new-old)*80` vs `80*new-80*old`; underfed birth branch; `spawn_loaded` metabolism age/Hurting(overflow); `Revive` coroutine.
- Newborn brain/vision phase counters not randomised (`BibiteBody.cs:436`).

**Mouth/stomach**
- Vomit branch not wired (`BibiteMouth.cs:348`); murder/reaper/instaKill missing (`:730`).
- Bibite swallow/bite stomach add deferred within tick (over-commit); swallow adds meat before prey contents (order reversed).
- Stomach contents not spilled on explosion (`BibiteStomach.cs:227`).
- Grab joints (break force, release on exit), throw impulse/recoil, grabbed check sign, `>=0.15` grab threshold, bite origin = mouth not centre, knockback on dead target, `born` guard, eggs grabbable, pellet ongoing-attack early return.
- Contact tracking is per-body tag with 8-tick grace instead of per-collider enter/exit with 10 slots.

**Senses**
- Wide-FOV (>180) in-cone test should be `|angle| < half` (`FieldOfView.cs:563`); closest-bibite colour inputs 18-20 use last seen, not closest; held-pellet exclusion should be own-held only.

**Propulsion/organs**
- Shade-avoidance steering constant turn; penalty applied twice in avoidance path; wrap not suppressed by shade avoidance and zeroes velocity; steering blend order.
- `limitBibiteBirth` not enforced on egg laying; eggs not laid on death; egg damping hard-coded; fat on_growth resets / over-capacity burn.

**World/settings**
- Virgin spawner ignores `randomizeGenes`; settings animators do not reach zones/bibites/materials; `census_and_cull` never called and ungated; `pelletMerge`, pellet collision/rotation/held-drag toggles unwired; posX/posY animation does not move zone; population limit should count eggs; egg spawn rows; ripped chunks not recycled; producer spawns >1 pellet/tick.

## Checked and matching (per audits)
Mouth/jaw/bite-period constants, bite chunk geometry, stomach digestion, 78+14 setting defaults, zone spawn/growth/move,
pellet size/drag/decay formulas, propulsion cost/force formulas, FOV weights/closeness/angle maths, cadence 2/10/5.

## Fixed in the second pass (each checked against the C# first)

| Item | C# | Rust |
|---|---|---|
| Collision damage once per new collider pair (`OnCollisionEnter2D`), receiver-mass formula (metric = normal speed) | `BibitePart.cs:47-66` | `body.rs` contact loop + `collision_damage` (signature lost `other_mass`) |
| `limitBibiteBirth` enforced on egg laying (bibites + eggs count) | `BibiteEggLayingOrgan.cs:148` | `body.rs::birth_limit_reached` |
| Completed eggs laid on death | `BibiteEggLayingOrgan.cs:215` | `body.rs` first corpse tick |
| Settings animators propagate to zones/bibites/Red Death; posX/posY move zone | `SettingsChanger`, `Zone.cs:548` | `world.rs::propagate_animated_settings` |
| Virgin spawner honours `randomizeGenes` | `WorldObjectsSpawner.cs:148-176` | `world.rs::spawn_bibite_from_template` (new param; Default fresh-newgame pins updated to 448 plants / energy 110.79) |
| Reaper/murder tracking and `instaKill` damage | `BibiteMouth.cs:725-738` | `body.rs::resolve_bite` (`meta.is_killer` is the insta-kill flag) |
| Vomit branch | `BibiteMouth.cs:348,864` | `body.rs::vomit` |
| Bibite swallow adds prey contents before meat | `BibiteMouth.cs:583` | `body.rs::resolve_swallow` |
| Wide-FOV (>180) in-cone test | `FieldOfView.cs:563` | `senses.rs::target_in_view` |
| Body energy params follow settings edits | `BibiteBody.cs:384`, `BibiteGenes.cs:119` | `BibiteStore::set_settings` |
| Fat: `available` clamp, `on_growth` no longer clamps/resets | `Matter.cs:24`, `BibiteFatOrgan.cs:82` | `organs.rs::FatOrgan` |
| Stomach spilled as pellets on explosion | `BibiteStomach.cs:227` | `body.rs::spill_stomach` |
| Shade avoidance: steer toward origin, penalty applied once, void before shade; wrap keeps velocity and is suppressed by shade avoidance | `BibitePropulsion.cs:272-309` | `body.rs::apply_environment_steering`, `write_back_bibite_pose` |
| Egg linear damping = `drag*size/mass` | `EggHatching.cs:154` | `body.rs` egg physics body |

Ecology check (Default fresh scenario, 36,000 ticks, seeds 1-12, final population): baseline 133 total (mean 11.1),
after first pass 145 (12.1), after second pass 166 (13.8); game single sample 19.

Deferred: randomised newborn phase counters (needs RNG in every spawn path), `census_and_cull` wiring (no autosave
cadence in headless), pellet merge/collision toggles, grab joints.

## Third pass (mutation, speciation, tools audits + more fixes)

Audits of mutation/inheritance, speciation/lineage and world tools/lifecycle were run read-only against the C#.

**Mutation/inheritance: no P0.** Gene mutation count/sigma/clamps, Poisson, `NextMutationValue`, GENE_DEFS ranges, brain mutation
cascade and kernels all match. Open P1/P2: split synapses should not be removable by `RemoveDisabledSynapse`
(`NEATBrain.cs:769-780`); `AddNeuron` in/out counters; final floating-hidden repair is unconditional in Rust; self-loop degree
counting; virgin-spawn `Mutate()` uses `Genome::child` (meta discarded); egg-spawn rows with `Mutate(n)` unimplemented.

**Fixed (verified against C#):**
- Species membership recount excluded eggs and included corpses (`Species.cs:81`, `BibiteBody.Die`): `world.rs::step_events_and_stats`.
- Extinct species left the active list every tick; the game only does so in `LogSpeciesInfo`: new `SpeciesRegistry::reconcile_counts`
  (counts only); `log_stats` now cleans up first, then drops empties (`GlobalLineageManager.cs:94-95`).
- The periodic species log never ran: `tick_stats` is now called each tick; period 300 s (`DataLogger` every 5 minutes).
- Hatched bibites ignored `birthOrientation` (all faced +Y): now `up = birthOrientation` or random 1..359 degrees (`EggHatching.cs:185-194`).
- Pellet placer follows `PelletPlacer.cs` (size factor x scenario pellet energy, +-25% jitter, pile radius, single pellet unscattered,
  hold-to-paint every 1.5 pile radii).

Ecology check (Default fresh scenario, 36,000 ticks, seeds 1-12, final population): 133 -> 145 -> 166 -> **195** (mean 16.3; game
single sample 19). Species count 1 on every seed (game: 1).

**Open from the speciation audit:** second-outlier founder should be the parent body and move it (`GlobalLineageManager.cs:235-246`);
`usedSpeciesSpan` should update on events, not lazily; `nPointsPresent` semantics for extinct species; second stage of
`SpeciesTreeCleanup`; `CheckSpeciesOfEgg`; `FindSpeciesFromTemplate`.
**Open from the tools audit:** `isRad` bibites never create a RadioZone (`+10/+0.35` mutation factor to touching bibites);
Red Death should explode corpses; ColorKiller corpse slot use; Pherosense 128 cap; egg-abort meat pile radius jitter.

## Fourth pass (scenarios/challenges audit + fixes)

**Fixed:** parent-as-founder for second-outlier speciation (`GlobalLineageManager.cs:235-246`); newborn sense/brain/lookup phase
counters randomised (`BibiteBody.cs:436-438`); Red Death now bursts corpses (`RedDeathBloomManager.cs:237-252`); egg-abort meat pile
spreads over `size/2`; challenge clock now runs on simulated seconds actually stepped (not while paused) and the game pauses when a
challenge ends (`ChallengeManager.cs:84-85`); champion row always tagged `Champion` (`BibiteSettings.LoadForChampion`).
Ecology after all of the above (12 seeds, 36k ticks): 195 total (mean 16.3), 1 species on every seed.

**Challenge runtime - open (weakest area):**
- P0 challenges run in the *current* world instead of loading a fresh scenario (`ChallengeManager.cs:51-74`, `ScenarioSelectorPanel.StartChallenge`);
  `Time` metric is absolute world time.
- P0 high scores/stars/achievements never persisted (`ChallengesProgress.TrySetHighScoreOfChallenge`); `ChallengeScoreInfo::beats` differs.
- P1 restart semantics (confirm dialog, reload, Return-to-Menu button), picker lists every scenario instead of `isChallenge` ones and
  does not require a champion, champion spawn settings ignored (hard-coded Adult at camera centre, spawned twice), tracker/end popup
  text (descriptions, exit-condition text, high/low label, next-star hint) missing.
- P2 ended non-repeat animators are not deleted; animator time is f32; non-numeric Constant argument returns 0.
- Verified matching: metric formulas, condition evaluation, star priority, 1 s check cadence, default challenge parameters, spline math,
  animator timing, spawner logic, radioactivity table.

## Fifth pass (save/load audit)

**Fixed:** pellet `matterDecay` is now read/written as a sibling of `"pellet"` (with `meatRot` fallback and the old nested spelling still
read) per `SaveSystem.cs:396-407,536-553` - before, every real save loaded with meat rot reset and Rust-written saves carried no decay state
for the game; absolute-size zones (`radiusIsRelative: false`) no longer lose their size (`ZoneSettings.cs:711-720`), covered by
`zone_size_tests`; the challenge `Time` exit metric now counts from the challenge start.

**Open (agent claims):**
- P0 zone-group zones are dropped on load (`SerializationHelper.cs:274-278`, `ScenarioSettings.UpdateAllZonesList`): the runtime zone list is
  `zones` followed by every group's `zones`, ids are list indices; Rust ignores `zoneGroups[*].zones` (`bibites-io/src/world.rs:140,160`).
- P1 scene version written is the scenario's, not the game's present version, so the game re-applies old-version migrations;
  `ColorKiller.followingZone` never read/written; challenge parameters not saved; parent/child/id links not written from a live world
  (ids are indices, eggs lose `parent1/2`); templates referenced by spawn rows are not bundled or installed.
- P2 zone `target`/`spawnZone` ids not remapped on write; species template `nodeAnchors` dropped; pheromone `nR/nG/nB` written as 0;
  version migrations not ported; `scene.nBibites` should count eggs; `data.bin` opaque and never regenerated; loaded bibites without a
  species are not assigned one (`ResumeBody` -> `CheckNewSpecies`).

## Sixth pass (independent review of this session's diff + follow-ups)

An independent read-only reviewer audited the whole uncommitted core/io/cli diff against the C#. Findings fixed:
- Murder counting used `body.dead`, so every bite of a corpse counted and bibites became reapers far too easily; the game tests
  `dying`, which the first corpse tick clears (`BibiteMouth.cs:730`, `BibiteBody.CorpseUpdate`). Now `report.died || body.dying`.
- Collision "enter" state was not checkpointed: after a restore every touching pair would take impact damage again. A restore now
  suppresses damage for the first physics step (`BibiteScratch::reset_collision_tracking`).
- Data logger: births counted only hatches in the game (not seeding/spawner rows), corpses must not appear in biomass/count/brain
  streams, brain means divide by the number of live bibites + eggs, `deaths` subtraction is saturating, the logger resets on restore.
- Lay-on-death fired on every corpse tick while the population limit blocked it; now exactly once at the death transition (`dying`).
- A swallowed prey's stomach was duplicated as pellets when corpses are disabled; the spill is skipped for swallowed prey.
- Mouth-target sort uses `total_cmp`; `scientific_format` no longer overflows on infinity; the parent bibite always moves into a
  newly founded species (member list / mutant flag only for living bodies); metabolism is recomputed when `baseMetabolismCost` changes.
- Brain mutation: disabled synapses created by an `AddNeuron` split are not removal candidates until a toggle or removal
  (`NEATBrain.cs:391-398,758-763,996-1003`).
- Inspector Genes tab lists all 35 genes with `GENE_DISPLAY_NAMES` (scrollable with the wheel).

Reviewer notes still open: mouth-target sort origin is the bibite origin (the Mouth prefab child sits at local (0,0,0), so this
should match; unverified); every animated settings write clones `SimSettings` and loops over all bibites.

## Regression evidence after the passes above (re-run, not copied from other docs)

- Ring FOV capture (`testdata/parity/fov/ring_game_600` vs a fresh Rust run of `ring.base.zip`, seed 12345, 600 ticks):
  **9000/9000 brain outputs bit-exact over 15 nodes**, output digests exact 2/2, species/generation/genes exact - unchanged from before the
  FOV box/cap, pheromone, collision, mouth and speciation changes. (`ecology_curves`/`entity_energy` FAIL on population 10 vs 1 as before:
  the game run used its spawner, the Rust base run does not.)
- Default official captures (600 and 3000 ticks): brain outputs 7233/9000 and 36033/45000 exact, unchanged; energy/position error unchanged.
- Default fresh scenario 36,000 ticks, final population over seeds 1-12: 133 -> 145 -> 166 -> 195 -> 200 total (mean 16.7; game single
  sample 19), one species on every seed.
- Test status at the time of writing: bibites-core 471, bibites-io, bibites-cli suites pass; bibites-view is type-checked only
  (its test build was too heavy for the owner's machine) - run `cargo test -p bibites-view` yourself.
- Determinism / resume (re-run after the passes above): Default fresh scenario, seed 777, 12,000 ticks gives fingerprint
  `5396600356773223879` with 1, 2 and 3 threads; 6,000 ticks + `.bb8state` resume + 6,000 ticks gives the same fingerprint (bit-exact).

## Last batch (core, tested)

- Species `n_points_present` now counts only logs where the species has members (extinct short-lived species stay prunable instead of
  being kept forever by an ever-growing counter); `history_window` test given a member.
- Finished non-repeating settings animators are removed from the scenario (`SettingsChangerManager.FixedUpdate`, cs:55-75).
- `BibiteStore::lay_single_egg` / `World::lay_egg_once` (LAY button, Insert): one egg without consuming progress (test added).
- Registry accessors `n_records()` / `set_n_records()`; the lineage panel is gated on 5 species-log records
  (`SpeciesPanel.OpenPanel`, cs:63) and the Graphs / Lineage panels pause the game while open.
- Final test status when written: bibites-core 472 + bibites-io + bibites-cli = 608 tests, 0 failures; `cargo check --all-targets` clean
  for core/io/cli; bibites-view type-checks without warnings (not run).

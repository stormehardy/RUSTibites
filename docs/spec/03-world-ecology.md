# 03 — World, Ecology, Zones, Pheromones, Sensing & Simulation Loop

**Status:** reverse-engineered spec, phase 0 deliverable (implementation-ready); **implemented and kept current (2026-10-30)**.<br>
**Owner:** world-dev. **Consumers:** `crates/bibites-core/src/{world,zone,pellet,pheromone,senses,tools,sim}.rs`, `bibites-io` (world save/scenario sections), `bibites-cli` (stats), body/brain modules (sensor/emitter interfaces).

**Primary source:** decompiled Unity C# in `Here/`. Every formula/constant below carries a `Here/<file>:<line>` reference. **Secondary cross-checks** (not authoritative): `bibites_brain_reference (4).md`, `Bibites_Code_Breakdown.md`, `BibitesAPI/docs/GAME_GUIDE.md`; where they disagree with the C#, the C# wins and the disagreement is recorded in §14 / Appendix D.

**Confidence tags used inline**

| Tag | Meaning |
|---|---|
| `[V]` | Verified by reading the C# source referenced. |
| `[C]` | Verified against a captured real scenario/save/asset file from the installed game (paths in Appendix C). |
| `[I]` | Inference: the source leaves the behaviour order/shape open, or the game itself is frame-dependent. Reconcile with ground-truth capture. |

**Scope.** Zones/plant biomass, matter materials & pellets & decay, corpses→meat, all world sensors (FieldOfView, Pherosense), pheromone spots/towers, world tools (Rad/Phero towers, ColorKiller), Red Death + shade/bounds, plant & bibite spawners/seeding, the global per-tick ordering and timers, world persistence/migrations, lineage/tracker metrics for CLI stats, and the Rust design notes. Bibite internals (organs, mouth, brain, physics solver) are owned by `02-body-organs-physics.md`/`01-brain-genetics.md`; this document only defines what the world supplies to them and what it consumes from them.

## Implementation status (2026-10-30)

**Status: implemented.** Code: `crates/bibites-core/src/{world,zone,pellet,pheromone,senses,tools,sim}.rs`;
persistence in `crates/bibites-io/src/{bundle,world}.rs`.

| Section | Status | Code | Representative tests |
|---|---|---|---|
| §0 conventions / bounds | implemented | `world.rs` `WorldBounds`, `sim.rs` phases | `world_bounds_shade_and_wrap_constants` |
| §1 zones & biomass | implemented | `zone.rs` geometry/production/seeding/movement/groups | `default_zip_appendix_a_numbers`, `zone_groups_expand_deterministically`, `over_cap_recycle_removes_from_front_and_accounts`, `pellet_biomass_lags_until_ten_second_recount`, `free_zone_movement_stays_inside_and_reflects` |
| §2 matter / pellets / decay | implemented | `pellet.rs` store, formulas, merge/rip, decay processor | `material_defaults_match_captured_table`, `matter_formulas`, `merge_conserves_energy_and_momentum`, `decay_is_fresh_then_rots_and_chops`, `decay_throttle_matches_game_period` |
| §3 FOV vision | implemented | `senses.rs` query, weights, herd metrics, `FovMask` | `weight_formula_hand_computed`, `cone_edge_intersection_makes_big_targets_visible`, `compute_fov_dead_bibite_feeds_meat_channel_too`, `fov_mask_from_brain_uses_brain_usage_masks` |
| §4 pheromone sensing | implemented | `senses.rs` `compute_phero` | `phero_sum_is_inverse_distance`, `phero_angles_and_headings_normalise_by_180`, `phero_red_death_pseudo_channel` |
| §5 pheromone spots | implemented | `pheromone.rs` | `dissipation_is_quantized_at_quarter_second`, `negative_strengths_are_allowed_and_removed_next_update`, `tower_spots_never_dissipate_or_die` |
| §6 world tools | implemented | `tools.rs` RadTower/PheroTower/ColorKiller (+ effects) | `rad_taper_formula`, `color_killer_compiled_polarity_kills_non_matching_colors`, `red_death_damage_cadence_and_formula` |
| §7 boundary / shade / Red Death | implemented | `world.rs` bounds, `tools.rs` RedDeath, propulsion avoidance in `body.rs` | `world_bounds_shade_and_wrap_constants`, `red_death_safe_radius_formula`, `full_tick_red_death_damages_pellets_outside_safe_radius` |
| §8 spawners & seeding | implemented | `world.rs` scenario seeding/population, `zone.rs` initial seeding, `PelletProducer` | `new_world_seed_scenario_seeds_pellets_when_configured`, `spawner_initial_population_is_deterministic_and_limited`, `pellet_producers_skip_source_free_worlds_but_run_for_sources` |
| §9 global per-tick ordering | implemented as the canonical order, measured as 17 phases in `sim.rs` (see note) | `world.rs::tick`, `sim.rs::Phase` | `full_tick_multi_system_smoke`, `full_tick_deterministic_repeat_run`, `settings_animator_applies_scenario_and_zone_settings` |
| §10 persistence / migrations | implemented for 0.6.3.x; `data.bin` opaque; LogLike binary history codec not reimplemented | `bibites-io` bundle/world, core `to_contents`/`from_contents` | `scene_round_trip_time_and_loaded_egg`, `contents_round_trip_populated_world`, `contents_load_then_run_is_deterministic` |
| §11 lineage / metrics | **partial:** main `SimStats` (counts, biomass, vitals, genes, species, behaviour) implemented; death-age rolling quantiles and the zone/species history streams are not reimplemented (`data.bin` is opaque) | `sim.rs::SimStats`; `species.rs` lineage | `stats_match_game_definitions` |
| §12 Rust design notes | implemented | spatial hash (`world.rs`), SoA stores, parallel sensing, thread pool (`sim.rs`) | `queries_match_brute_force`, `parallel_sensing_is_bit_identical_to_serial`, `perf_*` |
| §13 open questions | Q1/Q5/Q10/Q11 closed; Q3 implemented as D6; Q6 guarded; Q14 = opaque `data.bin`; the rest unchanged | — | — |

### Implementation notes (deviations and known gaps)

* **Default TPS is 30** in the engine (`clock.rs::DEFAULT_SIM_TPS`), always overridden by a scenario/save's
  `independents`/`recommended` block. The earlier "foundation/ARCHITECTURE say 40" text in this spec was stale and
  has been corrected; Q1 is closed.
* **Pipeline mapping.** `World::tick` implements the §9.2 order as 17 measured phases
  (`sim.rs::Phase`): `clock → animators → tools → zones → sensing → body → eggs → pheromones → decay → rad →
  red-death → colour-killers → producers → deaths → physics → events → hash-sync`. The §9.2 table's phase 5
  ("per-bibite") is split into explicit `sensing`/`body` phases (and `deaths`/`events`/`hash-sync` are bookkeeping)
  so timing can be attributed; the body-internal order remains authoritative in spec 02 §2.3/§2.6.
* **Immediate destruction (Q3/D6):** removed pellets/spots/bibites are logically removed at the phase boundary,
  not deferred to end-of-frame.
* **Seeding-loop hazard (Q6) is guarded:** `zone.rs` caps `InitialSeeding` at
  `MAX_SEEDING_ITERATIONS_PER_ZONE = 100_000` iterations and per-tick spawning at
  `MAX_SPAWN_ITERATIONS_PER_TICK = 10_000`, so the red-death no-progress case terminates instead of hanging.
* **`data.bin` is opaque (D11/Q14):** loaded bytes are preserved byte-for-byte and re-emitted unchanged; the
  LogLike zone/species history codec is not reimplemented, so §11.2/§10.5 statistics histories are not regenerated.
* **Coroutine timers (Q19) are tick accumulators:** zone biomass recount = 10 s, spawner = 1 s, Red Death/rad dose
  = 0.5 s, pheromone decay = 0.25 s quanta — all advanced by `dt`, not wall clock.

---

## 0. Canonical conventions

### 0.1 Units & coordinates

| Quantity | Unit | Note |
|---|---|---|
| World space | 2-D, metres ("u" in-game) | Simulation is a square `[-S, +S]²`; `S = SimulationSize` (default 2000, range 100–20000). `Here/SettingScripts/ScenarioIndependentSettings.cs:16-40` `[V]` |
| Matter amount | u² (area) | radius = `sqrt(amount/π)`. `Here/SimulationScripts/Matter.cs:26` `[V]` |
| Energy | E | `energy = amount × material.EnergyDensity`. `Here/SimulationScripts/Matter.cs:34-45` `[V]` |
| Time | simulated seconds | `TimeKeeper.simulatedTime`, advanced by `Time.fixedDeltaTime` per fixed tick. `Here/OneUseScripts/TimeKeeper.cs:17,149-169` `[V]` |
| Angle | degrees for FOV/brain input; radians only inside trig | Unity `Vector2.SignedAngle` convention (CCW positive): from `up`, a target on the bibite's **right** yields **negative** angle. See §5.2, `[I]` on label sign. |
| Tick | 1 FixedUpdate step | `dt = 1/simTPS` at time scale 1; default `simTPS = 30`. `Here/SettingScripts/ScenarioIndependentSettings.cs:170-180`, `Here/ManagementScripts/TimeController.cs:78-82` `[V]` |

`ScenarioIndependentSettings.simArea = 4·S²` (`Here/SettingScripts/ScenarioIndependentSettings.cs:230`).

### 0.2 World-space facts

* World bounds are **not** physics walls: bibites/pellets can leave `±S` into "shade". There is no boundary collider. `[V]` (`Here/SimulationScripts/Zone.cs` clamps only zone centres; `Here/SimulationScripts.BibiteScripts/BibitePropulsion.cs:272-307` handles void/shade/wrap).
* Shade geometry: `shadeStart = 1.5·S`, `shadeEnd = 1.5·S + 1000` (fixed 1000 u fade band). `Here/OneUseScripts/BackgroundManager.cs:58-68` `[V]`
* Pellets are culled when `|pos| > 1.5·S + 500` (only when shade is enabled, and only at certain census points — §3.7). `Here/SimulationScripts/ZoneManager.cs:224-265` `[V]`
* Bibites: void avoidance turns them back once outside all zone radii (`voidAvoidance`, distance `voidAvoidanceDistance` = 100 u). Shade avoidance turns them back beyond `|pos| ≥ 1.5·S`. World wrapping teleports them through the shade when `|pos| − bodyLength ≥ 1.5·S + 1000`. `Here/SimulationScripts.BibiteScripts/BibitePropulsion.cs:272-307` `[V]`

### 0.3 Time / tick semantics

| Concept | Value / rule | Ref |
|---|---|---|
| `baseFixedDeltaTime` | `1/simTPS` (30 TPS → 0.03333 s) | `Here/ManagementScripts/TimeController.cs:58,78-82` |
| `Time.fixedDeltaTime` | timeScale ≥ 1 → `base`; 0 < timeScale < 1 → `base·timeScale`; 0 → unchanged (no FixedUpdate runs while `Time.timeScale == 0`) | `Here/ManagementScripts/TimeController.cs:117-130` |
| `Time.maximumDeltaTime` | `= Time.fixedDeltaTime` — caps per-frame fixed-step catch-up, i.e. low FPS slows simulated time instead of skipping ticks | `Here/ManagementScripts/TimeController.cs:129` |
| `TimeKeeper.simulatedTime` | `double`, `+= Time.fixedDeltaTime` every FixedUpdate (pauses during timeScale 0); saved as a JSON float in `scene.bb8scene` | `Here/OneUseScripts/TimeKeeper.cs:17,149-169`; `Here/ManagementScripts/SaveSystem.cs:269,684-687` |
| Pause | `Time.timeScale = 0`; all sim `FixedUpdate`s additionally guard `if (Time.timeScale <= 0f) return;` | `Here/SimulationScripts/Zone.cs:190-193`, `Here/SimulationScripts.BibiteScripts/BibiteBody.cs:592-597` |
| Real-time vs sim-time | Wall-clock speed scaling is purely `Time.timeScale`; sim results depend only on the number of ticks and `dt` (see §10.4 for frame-dependent caveats) | `[V]` |

**Default TPS (resolved 2026-10-29).** `ScenarioIndependentSettings.simTPS` default is **30** (`:170-180` `[V]`),
and the official `Scenarios/Default.zip` `recommended` block does not override it `[C]`. The engine default is now
also **30** (`clock.rs::DEFAULT_SIM_TPS`), and `docs/ARCHITECTURE.md` §6 was corrected to 30. 40 appears only
because many newer official scenarios recommend `simTPS: 40` `[C]` (`Scenarios/RadTower.zip`); the engine always
uses the file's `recommended`/`independents` value when present. §13 Q1 is closed.

---

## 1. Zone model

### 1.1 `ZoneSettings` — per-zone definition & defaults

`ZoneSettings` is the serialized data (`Here/SettingScripts/ZoneSettings.cs`). Runtime `Zone` (`Here/SimulationScripts/Zone.cs`) is the live instance and caches derived values.

| Field (JSON key) | Type | Default | Range / notes | Ref |
|---|---|---|---|---|
| `name` | string | `"Zone {id}"` | zoneID from `ZoneSettings.maxID++`; renamed by group `{group}_{i}` | `:20-23,504-518`; `Here/SettingScripts/ZoneGroupSettings.cs:99-106,129` |
| `material` | `MatterMaterial` | Plant | nullable (`null` = "None" → no production). Only Plant/Meat actually spawn (see §2.8) | `:25-32`; `Here/SimulationScripts/WorldObjectsSpawner.cs:338-349` |
| `distribution` | `SpawnDistribution` | CentricGradual | Flat, CentricGradual, ExteriorGradual, Ring, FlatRing, Rect | `:34-41,255-266` |
| `biomassDensity` | float | 1.0 | 0.001–100 (×global) | `:43-55` |
| `fertility` | float | 1.0 | 0.001–100 (×global) | `:57-68` |
| `pelletSize` | float | 1.0 | 0.01–50 (× global pellet energy) | `:70-81` |
| `renewBiomass` | bool | false | keep spawning over cap, recycling oldest pellets | `:83-89` |
| `movement` | `MovementType` | None | None, Free, Repulsed, Orbit, Attached (only None/Free/Attached are exercised; see §1.7) | `:91-98`; `Here/SettingScripts/MovementType.cs` |
| `target` | zone id / null | null | zone to follow (Attached) — implemented by parenting | `:100-104`; `Here/SimulationScripts/Zone.cs:571-587` |
| `speed` | float | 1.0 | 0.001–1000 (×global zone speed) | `:106-115` |
| `posX`, `posY` | float | 0.0 | normalized to `[-1,1]`, dynamically capped to `±(1−relativeRadius)` (disc) or `±(1−relativeWidth/2)`, `±(1−relativeHeight/2)` (rect) | `:117-139,622-641` |
| `radiusIsRelative` (`sizeScalesWithSim`) | bool | true | size follows `SimulationSize` | `:141-147` |
| `radius` (relative) | float | 0.25 | relative radius × S | `:149-161` |
| `radius` (absolute) | float | 1000 | used when `radiusIsRelative=false`; 0–25000 | `:163-175` |
| `insideRadius` | float | 0.8 | ring inner radius as fraction of outer; 0–0.999 | `:177-189` |
| width/height (relative) | float | 0.25 / 0.5 | rect only; ×S | `:191-231` |
| width/height (absolute) | float | 1000 / 1000 | rect only | `:205-245` |

`ZoneSettings.DefaultTemplate()` = radiusRelative 0.15, fertility 10, biomassDensity 10, CentricGradual, `posX/posY/name = null`, `zoneID = −1` (`:545-559`) — used only as the template of procedurally generated zone groups (`Here/SettingScripts/ZoneGroupSettings.cs:89-90`).
`ZoneSettings.DefaultZone()` = Plant, radiusRelative 1, fertility 1 (`:561-568`) — used by the pre-0.5a4 save migration and editor.

### 1.2 Derived geometry & quantities

Let `S = SimulationSize`, `R = absoluteRadius` (relative×S if `radiusIsRelative` else absolute), `in = insideRadius`, `W = absoluteWidth`, `H = absoluteHeight`.

```text
isRing   = distribution ∈ {Ring, FlatRing}         // ZoneSettings.cs:289-299
isRect   = distribution == Rect                    // :301
isCircle = !isRing && !isRect                       // :303-313

area = π·R²                                     if Flat | CentricGradual | ExteriorGradual
     = π·R²·(1 − in²)                           if Ring | FlatRing
     = W·H                                      if Rect                             // :278-287

maxBiomass  = material==null ? 0 : biomassDensity · area · globalBiomassDensity   // :315-325
totalGrowth = material==null ? 0 : fertility      · area · globalPelletGrowth     // :327-337
estimatedPellets = ceil(maxBiomass / (pelletSize · pelletEnergy))                  // :339-349 (UI/CLI estimate only)
sizeFactor = pelletSize                                                            // :351
avgPelletEnergy (Zone) = pelletEnergy · sizeFactor                                 // Zone.cs:538-541
```

Global defaults: `globalBiomassDensity = 0.0075 E/u²` (`Here/SettingScripts/ScenarioIndependentSettings.cs:42-66`), `globalPelletGrowth = 1e-5 E/u²/s` (`:68-90`), `pelletEnergy = 200 E` (`Here/SettingScripts/ScenarioSettings.cs:431-442`), `initialSeeding = 0.75` (`:415-429`).

`AlignRadius()`/`AlignSize()` keep relative/absolute representations consistent: non-rect zones force `width = height = 2R`; rect zones force `R = max(W,H)/2` (`Here/SettingScripts/ZoneSettings.cs:570-620`).

### 1.3 Runtime `Zone` state

| Field | Meaning | Ref |
|---|---|---|
| `settings` | owning `ZoneSettings` | `Zone.cs:20` |
| `radius` | cached absolute radius (non-rect only; used by void avoidance, gizmos, zone logs) | `:64,514-525` |
| `pelletProgress` | accumulated energy waiting to become a pellet | `:66` |
| `pelletProduction` | `= settings.totalGrowth` (E/s), refreshed on fertility change | `:68,285-289` |
| `pelletBiomass` | cached sum of zone-owned pellet energies; **lagging** (see §1.6) | `:70` |
| `maxBiomass` | cached `settings.maxBiomass` | `:72,279-283` |
| `avgPelletEnergy` | `pelletEnergy.val · sizeFactor` | `:74,538-541` |
| `insideRadius` | cached `settings.insideRadius` | `:76,543-546` |
| `movement`, `speed`, `vel`, `rect`, `maxD` | movement state; `maxD = S − R` (disc) | `:40-49,525,591` |
| `zonePellets: List<MatterPellet>` | pellets owned by this zone, maintained by `onPelletGone` | `:80,317-349` |
| `pelletHolder: Transform` | parent transform holding zone pellet objects (authoritative for save/census) | `:22,176` |
| `freeBiomass` | `maxBiomass − pelletBiomass` (can be negative) | `:86` |

`ZoneManager` owns `List<Zone> zones` and derived world aggregates: `freeBiomass = Σ zone.freeBiomass`, `totalMaxBiomass = Σ maxBiomass` (`Here/SimulationScripts/ZoneManager.cs:19,33-35`).

### 1.4 Plant biomass growth / pellet spawn (`Zone.FixedUpdate`)

`Here/SimulationScripts/Zone.cs:188-214` `[V]`:

```text
# once per fixed tick, per zone, if Time.timeScale > 0
if movement != None: Move()

if pelletBiomass > maxBiomass:
    RecyclePellets(pelletBiomass - maxBiomass)
else if (renewBiomass || pelletBiomass < maxBiomass) && spawnMaterial != null:
    pelletProgress += pelletProduction · dt
    if !renewBiomass:
        pelletProgress = min(pelletProgress, max(freeBiomass, 0))
    while pelletProgress > 1.5 · avgPelletEnergy:
        pelletProgress -= SpawnAPellet()
```

Notes / exact consequences:

* Spawn threshold is **1.5× the average pellet energy**, not 1×; `pelletProgress` can legitimately go negative because `SpawnAPellet()` subtracts the *actual* pellet energy (up to 5× avg, see below).
* Over-cap recycling and spawning are mutually exclusive per tick (else-if). Recycling order is `pelletHolder` child order (`RecyclePellets`), removing `min(overage, pellet.energy + 1)` per pellet via `MatterPellet.RemoveEnergy` and decrementing `pelletBiomass` by the returned energy (`:440-460`). The `+1` guarantees a pellet whose energy ≤ overage is destroyed rather than left with a sliver.
* `SpawnAPellet()` (`:317-343`) `[V]`:
  ```text
  pos   = GetSpawnLocation()                        # §1.5
  e     = max(avgPelletEnergy, pelletEnergy.minValue) · clamp(1.5^N(0,1), 0.1, 5)
  if redDeathEnabled && |pos| > safeRadius: return e            # pellet NOT created, biomass NOT added
  p = WorldObjectsSpawner.SpawnPelletOfMatter(spawnMaterial, pos, energy=e, holder=pelletHolder)
  if p == null: return 0
  zonePellets.add(p); p.onPelletGone += RemovePelletFromList
  pelletBiomass += e
  return e
  ```
  * `1.5^N(0,1)` = log-normal "power Gaussian" (`Here/ScriptHelpers/RandomExtension.cs:19-31`): median 1, mean ≈ 1.086, σ ≈ 0.458 before the clamp; clamp `[0.1, 5]`. So pellets vary about ±46 % around nominal — the settings tooltip's "~25 % variation" (`ScenarioSettings.cs:434`) is optimistic `[V]`.
  * `pelletEnergy.minValue = 10 E` acts as a floor *in energy* (`:438`), so tiny `pelletSize` zones never spawn sub-10-E pellets through this path.
  * `pelletOffset = (0,0,0.1)` is added to every pellet position for Unity sprite sorting (`Here/ManagementScripts/WorldObjectsSpawner.cs:84,269-274`). Ignore `z` in the Rust core.
  * Red-death guard returning `e` **without** creating a pellet and without adding biomass is a landmine for the `while` loop and `InitialSeeding` (§1.6, §13 Q6).

### 1.5 Spawn location distributions — exact formulas

`Zone.GetSpawnLocation()` (`:351-375`) `[V]`, with `pos = zone centre`:

| distribution | offset from zone centre |
|---|---|
| `Ring` | `RandomPointInRing(R, in, warp=3)` |
| `FlatRing` | `RandomPointInRing(R, in, warp=1)` |
| `Rect` | `(U(-1,1)·W/2, U(-1,1)·H/2)` |
| `Flat` / `CentricGradual` / `ExteriorGradual` | `R · InsideUnitCircleWarped()` |

Helpers (`Here/ScriptHelpers/RandomExtension.cs:89-101`, `Here/SimulationScripts/Zone.cs:377-393`) `[V]`:

```text
RandomPointInRing(Rout, rinRel, warp):
    u  = Random.insideUnitCircle            # uniform in unit disc
    dir = u.normalized                      # uniform angle
    a  = rinRel²
    t  = (sqrt(U(0,1)·(1−a) + a) − rinRel) / (1 − rinRel)
    r  = (1 + t^(1/warp) − (1−t)^(1/warp)) / 2 · (1 − rinRel) + rinRel
    return dir · (Rout · r)

InsideUnitCircleWarped():
    u  = Random.insideUnitCircle; m = |u|
    p  = 1   (Flat) | 0.5 (CentricGradual) | 2 (ExteriorGradual)
    return u.normalized · (1 − (1 − m)^p)
```

Note the radial `Random.value` in `RandomPointInRing` comes from Unity's RNG while `insideUnitCircle` also does; all spawn location randomness is Unity's global RNG `[V]`. `Zone.InsideRing()` (`:395-407`) exists but is dead code (never called) `[V]`.

Resulting spatial densities: `Flat` = uniform over the disc; `CentricGradual` = density concentrated toward the centre; `ExteriorGradual` = concentrated toward the rim; `Ring`/`FlatRing` = annulus with `Ring` denser near the middle of the annulus and `FlatRing` uniform across it (the `warp=3` parameter is the only difference).

### 1.6 Biomass accounting: the 10-second lag (critical)

`pelletBiomass` is **not** updated when a pellet is eaten, decays, or is destroyed. It is recomputed only by:

1. `RecountBiomass()` = `Σ zonePellets.energy` (`Zone.cs:435-438`), called from
   * `UpdateBiomassCount()` coroutine: `WaitForSeconds(10)` forever, started in `InitializeZone` (`Zone.cs:185,409-416`), and
   * `ResetList()` after a load (`Zone.cs:418-433`).
2. Increment in `SpawnAPellet` (`:341`), decrement in `RecyclePellets` (`:457`).

Consequences `[V]`:
* `maxBiomass` is enforced against a stale counter: a zone can overshoot the cap by up to ~10 sim-seconds of production + one pellet, and keeps spawning while pellets are being eaten.
* `InitialSeeding()` (`:462-469`) loops `while pelletBiomass < initialSeeding·maxBiomass: SpawnAPellet()`; it runs once at sim start and seeds up to ~75 % of the cap (each iteration adds the actual pellet energy, so it stops on the first pellet that crosses the threshold — overshoot ≤ 1 pellet). If `SpawnAPellet` returns non-zero without creating a pellet (red-death guard) this loop never advances its biomass → **infinite loop** `[V]` (§13 Q6).
* `freeBiomass` inherits the lag and may be negative.
* Slice/energy truth for stats comes from `PlantCounter`/`MeatCounter` (§2.9), not from `zone.pelletBiomass`.

### 1.7 Zone movement

`Zone.Move()` (`:216-277`) `[V]`:

```text
val = transform.position                      # world
if movement == Free:
    vel += Random.insideUnitCircle · dt · speed / 10
    if |vel| > speed: vel = vel.normalized · speed
    val += vel · dt
# bounds reflection (all movement types except None run this):
if isRect: maxX = S − W/2; maxY = S − H/2
else:      maxX = maxY = maxD = S − radius
if |val.x| > maxX: val.x = sign(val.x)·maxX; vel.x *= −1
if |val.y| > maxY: val.y = sign(val.y)·maxY; vel.y *= −1
settings.posX = val.x / S ; settings.posY = val.y / S     # triggers UpdatePosition subscribers
transform.position = val
```

* `speed = S · globalZoneSpeed · settings.speed / 3600` in u/s (`:589-592`). With defaults (`S=2000`, global=1, zone speed=1) this is **0.556 u/s** — despite the tooltip claiming "1 % of sim size per hour" (`ScenarioSettings.cs:933`), which would be 0.0056 u/s. Trust the formula; record discrepancy (§13 Q7).
* `vel` is initialised `Random.insideUnitCircle · speed` at zone init (`:183`).
* `MovementType.Attached` does **not** move the zone in `Move`; it is implemented by re-parenting the zone to the target zone's transform (`UpdateTarget`, `:571-587`). `Repulsed`/`Orbit` exist in the enum (`Here/SettingScripts/MovementType.cs`) but have **no implementation** in `Move`/`UpdateTarget` — they behave like `None` (plus bounds clamps) in this build `[V]`.
* `UpdatePosition()` (`:548-569`) is the settings→transform direction: `posX/posY` clamped to `±(1−relativeRadius)` (or rect half-extents), then `transform.position = S·(posX,posY)`.

### 1.8 Zone lifecycle / list management

* `ZoneManager.InitializeSpawner()` builds one `Zone` per `ScenarioSettings.allZones` entry (subscription to add/remove events for runtime edits) (`ZoneManager.cs:53-98`).
* `ScenarioSettings.allZones` = explicit `zones` + every zone generated by every `ZoneGroupSettings`; `zoneID`s are reassigned sequentially in that list order (`Here/SettingScripts/ScenarioSettings.cs:1092-1107`).
* `ZoneGroupSettings` procedurally generates zones: `count = zoneCount` or `ceil(zoneDensity · simArea/1e6)` if `scaleWithSim` (`Here/SettingScripts/ZoneGroupSettings.cs:32-71`), each a copy of `template` at a random-position-in-range (`:122-135`; `SetRandomPositionInRange` `ZoneSettings.cs:655-670`). Default group count 10, density 0.66 zones/ku², template radiusRelative 0.05 (`ZoneGroupSettings.cs:32-53,89-90`).
* Load rebuilds zones before scene/pellets: `LoadGame → SimulationManager.BuildSpawners()` (`ZoneManager.InitializeSpawner` again) before `LoadScene` (`Here/ManagementScripts/SaveSystem.cs:780-781`) `[V]`.

### 1.9 Zone logging

Each zone holds `zoneData: LogLikeZoneDataArray` (14-byte points) and `LogData()` pushes a `ZoneDataPoint`; `ZoneManager.LogAllZonesData()` is called by the data logger every species-log tick (300 sim-s) (`Zone.cs:18,594-597`; `ZoneManager.cs:193-199`; `DataLogger.cs:387-409`) `[V]`. Point semantics in §11.2.

---

## 2. Matter, materials, pellets, decay

### 2.1 `Matter` — the raw substance model

`Here/SimulationScripts/Matter.cs` `[V]`:

| Property | Formula | Line |
|---|---|---|
| `fullness` | `max(Amount/MaxAmount, 0)` | `:16` |
| `isEmpty` | `Approximately(Amount, 0)` (Unity ~1e-5-ish) | `:18` |
| `overflow` | `max(Amount − MaxAmount, 0)` | `:22` |
| `available` | `max(MaxAmount − Amount, 0)` | `:24` |
| `radius` | `sqrt(Amount/π)` | `:26` |
| `cohesiveness` | `Material.Cohesiveness` | `:28` |
| `EnergyDensity` | `Material.EnergyDensity` | `:30` |
| `mass` | `|Amount · Material.MassDensity|` (game stores mass in grams internally; keeps "g" naming) | `:32` |
| `energy` | `Amount · Material.EnergyDensity` (setter divides) | `:34-45` |
| `ConvertToMaterial(to, withLosses, affFrom, affTo)` | `energy / to.EnergyDensity · (withLosses ? effFrom·effTo : 1)` | `:47-50` |
| `ConvertToFromEnergy(amount, affinity)` | clamps to `MaxAmount`, adjusts `Amount`, returns signed energy at efficiency | `:52-57` |
| `MaxAmount` | `+∞` by default; only stomach/organs set finite caps | `:14` |

`MatterMaterial` (`Here/SimulationScripts/MatterMaterial.cs`): `Name, Cohesiveness, MassDensity, EnergyDensity, Reactivity, MinConversionEfficiency, MaxConversionEfficiency, decay, freshTime, decayRate`.
* `ConversionEfficiency(affinity) = Min + clamp01(affinity)·(Max − Min)` (`:41-44`).
* `NormalAmountAtScale1 = 20 u²`, `NormalRadiusAtScale1 = 2.5 u`, `NormalEnergyAtScale1 = EnergyDensity·20` (`:35-39`).
* `RadiusOfAmount(a) = sqrt(a/π)`; `AmountOfRadius(r) = πr²` (`:56-64`).

### 2.2 Material set & exact default values

The game instantiates 7 materials: `plant, meat, fat, armor, energy, health, none`; `PhysicalMaterials = {Plant, Meat}` (only these can be spawned as world pellets); `BaseMaterials = {Plant, Meat, Fat, Armor, Energy, Health, None}`; settings-editable = Plant, Meat, Fat, Armor (`Here/SimulationScripts/MatterMaterialManager.cs:68-92`) `[V]`.

Defaults, cross-checked from a fresh 0.6.3.x scenario `settings.bb8settings` (`Scenarios/RadTower.zip`, which serializes the reset values) `[C]`:

| Material | Cohesiveness (N/u) | Reactivity (u/s) | EnergyDensity (E/u²) | MassDensity (g/u²) | MinEff | MaxEff | decay | freshTime (s) | decayRate (u²/s) |
|---|---|---|---|---|---|---|---|---|---|
| Plant | 10 | 0.03 | 5 | 0.015 | −0.25 | 0.5 | false | — | — |
| Meat | 35 | 0.075 | 10 | 0.025 | 0.15 | 0.85 | true | 2000 | 0.02 |
| Fat | 0.25 | 1 | 30 | 0.03 | 0.5 | 0.85 | false | — | — |
| Armor | 500 | 1 | 1.5 | 0.05 | 1 | 1 | false | — | — |
| Energy (organ) | — | — | not serialized (internal) | | | | | | |
| Health (blood) | — | — | internal ("blood") | | | | | | |
| None | — | — | 0 / null material | | | | | | |

Sanity: a default-energy plant pellet = 200 E / 5 E/u² = **40 u²**, radius **3.568 u**, sizeFactor **2.0**, mass **0.6 g** (the captured save shows plant amounts ≈ 45–48 u² for ~230–240 E pellets, consistent with the spawn multiplier) `[C]`. A 200 E meat pellet = 20 u², radius 2.524 u, mass 0.5 g.

Material migration rules: pre-0.6.0.16 reactivity was divided by 30; Fat reactivity was then specially corrected to `sqrt(old)·π` (`Here/SimulationScripts/MatterMaterialSettings.cs:233-263`; `Here/ScriptHelpers/SerializationHelper.cs:385-400`) — this is why Plant reactivity is 0.03 (0.9/30) `[V]`.

### 2.3 `MatterPellet` — the physical food item

`Here/SimulationScripts/MatterPellet.cs` `[V]`:

| Quantity | Formula / value | Line |
|---|---|---|
| `amount` | `matter.Amount` u²; setting amount updates physical properties | `:41-53,158-163` |
| `energy` get/set | `amount·EnergyDensity` / `amount = e/EnergyDensity` | `:55-65` |
| `sizeFactor` | `max(amount,0)/20` | `:67` |
| `radiusFactor` (= transform scale) | `sqrt(sizeFactor)` (set in `UpdatePhysicalProperties`) | `:169-170` |
| `radius` (analytic) | `sqrt(amount/π)` | `:69` (`Matter.radius`) |
| `mass` | `|amount·MassDensity|` | `:75` |
| Rigidbody mass | `rb.mass = mass` (`= 0` for amount 0) | `:171` |
| linear damping | `drag · radiusFactor / rb.mass`, or `0` if held and `disableDragOnHeldPellet` | `:130-133` |
| angular damping | `drag/2` set once in `Awake` (not re-applied) | `:87-96` |
| collider | `CircleCollider2D` on the prefab at local radius `NormalRadiusAtScale1 = 2.5`; `ToggleCollision(false)` makes it a trigger and freezes rotation; `ToggleCollision(true)` re-enables | `:223-236`; `Here/ManagementScripts/WorldObjectsSpawner.cs:131-136` |
| merge | `OnCollisionEnter2D`: if both pellets, `pelletMerge`, same material, and other.amount ≤ this.amount → this.amount += other.amount, momentum-weighted velocity, destroy other | `:238-258` |
| remove | `RemoveAmount(m)`: if `amount ≤ m` → return amount, set 0, `Destroy(gameObject)`; else `amount -= m` | `:135-156` |
| remove energy | `RemoveEnergy(e) = material.EnergyOfAmount(RemoveAmount(material.AmountOfEnergy(e)))` | `:153-156` |
| rip chunk | `chunk = min(chunk, amount)`; abort if `chunk < minPelletSize` or remaining `< minPelletSize`; remove chunk, spawn new pellet of same material with chunk amount offset along normal (default `Random.insideUnitCircle`), shift body | `:186-221` |
| save | `{material: Name, amount}` + caller adds transform/rb2d + decay state | `:260-278`; `SaveSystem.cs:395-424` |

`StartPellet()` (`:117-128`): scale = `√sizeFactor`, registers with the decay processor iff `material.decay`, applies drag & collision toggles. Note the **collider radius and `Matter.radius` differ by ≈1 %**: physical radius = `2.5·√(amount/20)` vs analytic `√(amount/π)` (ratio `2.5/2.523 = 0.991`). Sensing uses the analytic radius; collision uses the collider. In the custom solver use a single documented radius (recommend analytic `√(A/π)` for both) — §13 Q8.

### 2.4 Pellet decay processor (`MatterDecayProcessor`)

Runs on a throttle, not every tick. `Here/SimulationScripts/MatterDecayProcessor.cs:106-195`; `Here/Structs/MatterDecayOfArrays.cs:71-82` `[V]`:

```text
every FixedUpdate:
  ticksProgress += 1
  if ticksProgress < decayPeriod: return            # decayPeriod = decayTPS, default 2
  ticksProgress = 0
  dtDecay = Time.fixedDeltaTime · decayPeriod
  for each registered pellet i (parallel job in game):
      freshTimeRemaining[i] -= dtDecay
      decayAmount[i] += (freshTimeRemaining[i] > 0) ? 0 : dtDecay · decayRate[i]
      remove   = decayAmount[i] / amount[i] > 0.05
      toRemove = min(decayAmount[i], amount[i] + 1e-6)
      if remove: decayAmount[i] = 0
  for each flagged: pellet.RemoveAmount(toRemove)   # destroys pellet when toRemove ≥ amount
```

* Registration (`TryAddUnique`): only if `material.decay`; stores `amount`, `freshTimeRemaining = material.freshTime`, `decayAmount = 0`, `decayRate = material.decayRate`. Amount changes are mirrored through an `AfterAmountChange` listener (`:214-237`). Removal is swap-back (`:239-262`).
* State persistence: `{timeAlive: freshTime − freshTimeRemaining, rotAmount: decayAmount}` (`:264-290`).
* Semantics: a pellet is "fresh" for `freshTime` seconds; afterwards rot accumulates at `decayRate` u²/s. Once rot exceeds **5 % of current amount**, that rot is chopped off (amount shrinks by `decayAmount`, then rot resets to 0) — i.e. decay is applied in ≈5 %-of-amount chunks each decay update; when rot ≥ amount the pellet is destroyed. Effective decay ≈ `decayRate` u²/s, quantised by the 5 % gate and the `decayTPS` throttle.
* Default behaviour: **plants never decay** (Plant.decay=false); **meat rots** (fresh 2000 s, then 0.02 u²/s). Both are settings-editable at runtime; changing material params updates decay params for all live pellets of that material and adding/removing the decay flag registers/unregisters them (`Here/SettingScripts/SettingsUpdater.cs:81-124`) `[V]`.
* Capacity starts at 100 and doubles (`MatterDecayOfArrays.Upsize`, allocation churn — irrelevant for Rust) `[V]`.

### 2.5 Meat piles and corpse→meat conversion

`WorldObjectsSpawner.SpawnMeatPile(totalEnergy, pos, radius)` (`Here/ManagementScripts/WorldObjectsSpawner.cs:300-336`) `[V]`:

```text
n = 1                                      # hard-coded; the multi-pellet branch is dead code
if n >= 2: ... (unreachable)
else if totalEnergy >= Meat.EnergyDensity · minPelletSize:
    radius *= 0.1
    pos += (U(-radius,radius), U(-radius,radius))
    SpawnMeatPellet(pos, energy = totalEnergy)
```

So a death/abort creates exactly **one** meat pellet carrying the whole energy (offset randomly within 10 % of the body's half-length), unless it is below `10 E/u² · minPelletSize = 1 E` (default), in which case nothing is spawned and the energy vanishes.

Corpse lifecycle (world-relevant parts) `[V]` (`Here/SimulationScripts.BibiteScripts/BibiteBody.cs:788-863,641-648`):
* `Die()`: if `corpsesEnabled` and born → keep the bibite object alive as a corpse: disable growth & clock, `corpseEnergyOffset = Health.EnergyOfAmount(Health.MaxAmount) − Energy.Amount`, `Energy.Amount = 0`, `Health.Amount = Health.MaxAmount`, recompute metabolism.
* Each tick `CorpseUpdate()`: `UseEnergy(metabolism · corpseDecayFactor · dt)`; because energy is 0, `UseEnergy` converts the cost into damage (`Hurting(Health.Material.AmountOfEnergy(cost))`). When health < 0 → `ExplodeToMeat()`.
* `ExplodeToMeat()`: `SpawnMeatPile(totalEnergy, pos, bodyLength/2)` then destroy. `totalEnergy = gene.SumBodyAndHealthEnergyRatio·baseBodyArea + Energy + eggLayer.storedEnergy + fat.fatReserves.energy − corpseEnergyOffset` (`:256-266`) — the corpse's remaining stored energy (body+blood+eggs+fat) becomes one meat pellet.
* `corpsesEnabled = false` → `ExplodeToMeat()` immediately at death. Swallowed corpses vanish (no meat).
* Defaults: `corpseDecayFactor = 1` (`Here/SettingScripts/ScenarioSettings.cs:984-995`), `healthEnergyFactor = 0.5` (`:915-928`), `healthPerArea = 1.25 HP/u²` (`:846-858`), `baseBodyGrowthCost = 2 E/u²` (`:874-885`), `baseMetabolismCost = 0.001 E/u²/s` (`:674-686`), `energyUsageEfficiency = 0.8` (`:688-701`). Basal metabolism = `baseMetabolismCost · area · (1+agePenalty)` ≈ 0.08 E/s for an 80 u² body, and `UseEnergy` divides by `0.8^SpeedRatio` (≈1.25× cost at SpeedRatio 1) → ≈0.1 E/s. Corpse lifetime then follows from the **Health material's EnergyDensity** (an internal, non-serialized asset) through `Hurting(Health.Material.AmountOfEnergy(cost))` — capture or body-spec if exact corpse lifetime figures are needed `[I]`.
* A corpse remains in `BibiteTracker.bibites` and in `FieldOfView.seenBibites`; it is **also** pushed into `seenCorpses` when `dead` (`FieldOfView.cs:288-297`). See §3.5 for its dual-channel sensing weight.
* Revive exists (`:865-903`) but is UI-only; not needed for the headless engine.

### 2.6 Pellet settings that alter world physics

| Setting | Default | Effect | Ref |
|---|---|---|---|
| `pelletCollision` | true | `false` → pellets become triggers (no collision) | `ScenarioSettings.cs:221-228`; `MatterPellet.cs:127,223-236` |
| `pelletMerge` | false | same-material pellets merge on contact (larger absorbs smaller) | `:230-237`; `MatterPellet.cs:238-258` |
| `pelletRotation` | true | `false` → pellet rotation frozen at 0 | `:239-246`; `SettingsUpdater.cs:134-145` |
| `eggCollision` | true | analog for eggs | `:248-255`; `EggHatching.cs:99-101` |
| `minPelletSize` | 0.1 u² | minimum matter chunk; used by rip/decay/stomach/mouth. Not enforced at spawn | `:265-275` |
| `dragCoefficient` | 5 | pellet drag = drag·√(A/20)/mass | `:459-470` |
| `disableDragOnHeldPellet` | false | held pellet drag 0 | `:472-478` |
| `corpsesEnabled` | true | corpse→meat as §2.5 | `:277-284` |
| `corpseDecayFactor` | 1 | corpse metabolism multiplier | `:984-995` |

### 2.7 Out-of-bounds pellet culling (shade)

`ZoneManager.RefreshPelletBiomassCounter()` (`Here/SimulationScripts/ZoneManager.cs:215-266`) `[V]`:
* Runs on sim-size change/start (`:42-46`, called from `InitializeSpawner`) and **before every autosave** (`Here/ManagementScripts/SaveController.cs:237`), and during load's zone recount; not on manual quick-save and not per-tick.
* Resets `PlantCounter`/`MeatCounter`, re-registers every pellet, and if `shadeOutsideOfBounds` (default true) destroys every free or zone-owned pellet with `|pos| > 1.5·S + 500`.
* Also calls `zone.RecountBiomass()` for every zone (so the stale `pelletBiomass` is corrected at those moments).
* Headless recommendation: expose this as an explicit `census()` system run on the same schedule as the game (init/autosave) for fidelity; do **not** cull every tick.

### 2.8 Spawnable materials

`WorldObjectsSpawner.GeneratePelletOfMatter` only instantiates prefabs for `Plant` and `Meat`; any other material returns `null` (`Here/ManagementScripts/WorldObjectsSpawner.cs:338-349`) `[V]`. Zone `spawnMaterial` of `None` therefore produces nothing (and `maxBiomass = totalGrowth = 0` anyway, `ZoneSettings.cs:315-337`). The dead multi-pellet branch in `SpawnMeatPile` means "meat piles" are always one pellet in 0.6.3.x.

### 2.9 Census counters (`IEntityCounter`)

`PlantCounter` and `MeatCounter` are `MonoBehaviour`s placed on the plant/meat pellet prefabs; they maintain **global** `static int Count` and `static double Biomass` (`Here/SimulationScripts/PlantCounter.cs`, `MeatCounter.cs`, interface `IEntityCounter.cs`) `[V]`:

* `Awake`: `Count += 1`, subscribe to `pellet.AfterEnergyChange` → `Biomass += Δenergy`.
* `OnDestroy`: `Count -= 1`, `Biomass -= pellet.energy`, unsubscribe.
* `AddToGlobalCount()` is used only by the census refresh to re-register after a `ResetCount()` (e.g., load).
* `ChangeCount`/`Biomass` update immediately with every `amount` change (eating, biting, decay, recycling), unlike `Zone.pelletBiomass`.

Rust equivalent: maintain `plant_count/plant_biomass`, `meat_count/meat_biomass` counters updated by a single `pellet_energy_delta(...)` choke point — every pellet mutation must go through it or stats drift (the C# design is fragile exactly here).

---

## 3. Sensor: FieldOfView (vision)

`Here/SimulationScripts.BibiteScripts/FieldOfView.cs` `[V]` unless noted. This is the most expensive per-bibite sensor and is throttled twice (§3.7). Bibite-side wiring is in `BibiteBody.FixedUpdate` (`Here/SimulationScripts.BibiteScripts/BibiteBody.cs:592-639`).

### 3.1 State and caps

| Buffer | Size | Notes |
|---|---|---|
| `elementsInViewRadius: Collider2D[]` | 128 | physics query output, `NonAlloc` (so **not cleared** between queries) | `:201` |
| `seenBibites: BibiteBody[]` | 128 | one per bibite *part-collider* deduped to parent body | `:18` |
| `seenCorpses: BibiteBody[]` | 128 | subset of `seenBibites` where `body.dead` | `:21,288-297` |
| `seenPlantPellets: MatterPellet[]` | 128 | | `:24` |
| `seenMeatPellets: MatterPellet[]` | 128 | | `:27` |
| `plantWeights/meatWeights/bibiteWeights: GameObjectWeight[]` | 128 | `{gameObject, weight}` | `:30-36` |
| `held: Transform[]` / `nHeld` | 10 | mouth attachments, refreshed each sense computation | `:39-42` |

The 128 limit is really the physics query limit; because every found collider maps to at most one array slot of its category, the filters cannot overflow in practice. `seenCorpses` can only overflow if >128 dead bibites are within one query, impossible because `nBibitesInRange ≤ 128` (same query) `[V]`.

### 3.2 Enable mask & laziness

`NEATBrain.StartUsedSystems()` decides which senses to compute at all (`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:533-551`) `[V]`:

```text
mask = 0
if (herding enabled and herding-output wired) OR any of bibite vision inputs {ClosestBibiteWeight/Angle/NBibites, Red/Green/BlueBibite} wired: mask |= 1 (Bibite)
if any plant vision input {PlantCloseness/PlantAngle/NPlants} wired:                                 mask |= 2 (Plant)
if any meat  vision input {MeatCloseness/MeatAngle/NMeats} wired:                                   mask |= 4 (Meat)
if gene(ViewAngle)·gene(ViewRadius) <= 0: mask = 0
fow.StartSeeing(mask)
```

`StartSeeing(mask)` builds the layer mask (`bibite` layer if bit 1, `pellets` layer if bits 2|4), sets `needToSee = mask > 0`, allocates the 128 arrays, and grabs the mouth (`FieldOfView.cs:181-208`). If `needToSee == false`, both `FindSeenEntities` and `ComputeSenses` early-return and all vision inputs stay at their defaults (`:232-235,390-393`). Target-type enum is `[Flags] {None, Bibite=1, Plant=2, Meat=4, Egg=8}` (`TargetType.cs`) though only 1/2/4 are used by FOV.

### 3.3 The overlap query

`FindSeenEntities()` (`:210-300`):

```text
halfAngleRad = (viewAngle/2)·π/180
margin       = min(0.25·viewRadius, 25)                       # u
if halfAngleRad < 5π/8:  # i.e. viewAngle < 225°
    a      = min(0, R·cos(halfAngleRad))
    size   = (2·R·(viewAngle>180 ? 1 : sin(halfAngleRad)),  R − a) + 2·margin·(1,1)
    center = position + ((R + a)/2)·up
    rot    = SignedAngle(Vector2.up, up)                      # degrees, CCW positive
    n = Physics2D.OverlapBoxNonAlloc(center, size, rot, buf, layerMask)
else:
    n = Physics2D.OverlapCircleNonAlloc(position, R + margin, buf, layerMask)
```

* `OverlapBox.size` is the **full** box size; the box extends from the bibite centre to `R+margin` forward and sideways to the cone half-width, correctly covering the cone plus a margin `[V]` (the `2·margin` is *inside* the full size, so total margin is `margin` per side).
* The circle fallback for `viewAngle ≥ 225°` over-queries (full circle) and relies on per-object FOV filtering.
* Query layers include both bibite *parts* and pellets; trigger colliders are matched depending on `Physics2D.queriesHitTriggers` (Unity default true — hold as `[I]`; triggers include non-colliding pellets).
* Results are deduped for bibites by walking `seenBibites[0..nBibitesInRange)` and comparing parent GameObjects (`:274-297`); the dedupe scratch reuses `bibiteWeights[j].gameObject` from the previous `ComputeSenses` call, but only entries already written this pass are inspected `[V]`.
* **Stale-array bug (quirk to mirror or not):** `NonAlloc` does not clear the buffer. Objects are only read up to `nInRange`, and arrays are indexed by per-category counters reset at the top of the method, so behaviour is consistent; no stale reads occur in this call path. (The analogous stale-array bug does bite `ColorKiller`, §6.4.)

### 3.4 `TargetIsInView` — exact distance/angle test

`FieldOfView.TargetIsInView(transform, objectRadius)` (`:538-584`) `[V]`. Sets three instance fields: `dir2Target` (vector with length = effective distance), `dist2Target` (effective surface distance), `angle2Target` (degrees).

```text
dir  = target.position − self.position
d    = |dir|
if d − objectRadius > R:                       return false      # too far even at surface
ang  = SignedAngle(self.up, dir)                                # CCW+, right = negative
if |ang| < viewAngle/2:
    dist2Target = max(d − objectRadius, 0.001)
    dir2Target  = dir.normalized · dist2Target
    angle2Target = ang
    return true
# outside the cone: compute the intersection of the view-cone boundary with the
# target circle (line/circle solve); angle is clamped to the cone edge.
k1 = 2·d·cos((|ang| − viewAngle/2)·π/180)
k2 = d² − objectRadius²
disc = k1² − 4·k2
if disc <= 0: return false
dist2Target = max(0.001, k1/2 − sqrt(disc)/2)
if dist2Target > R: return false
angle2Target = sign(ang) · viewAngle/2
dir2Target   = self.up.rotated(angle2Target·π/180) · dist2Target
return true
```

Key semantics: an object is visible if its **surface** is inside the cone/radius, even when its centre is outside the cone (the intersection branch makes the cone edge "soft" for big objects); `dist2Target` is centre-distance minus object radius, but for the edge case it is the distance to the cone-boundary intersection point.

### 3.5 Per-category weights (exact)

Each surviving object contributes a weight. All use `fovR = viewRadius`, `fovA = viewAngle`, `d = dist2Target` from §3.4, `θ = |angle2Target|`.

| Channel | Weight (per object) | Source |
|---|---|---|
| Plant pellet | `(1.05 − d/fovR)² · pellet.sizeFactor · (1.05 − θ/(fovA/2))` | `:426` |
| Meat pellet | `(1.05 − d/fovR)² · pellet.sizeFactor · (1.05 − θ/(fovA/2))` | `:439` |
| Corpse (added to **meat** channel) | `(1.05 − d/fovR)² · (body.baseBodyArea/20) · (1 − body.healthRatio) · (1.05 − θ/(fovA/2))` | `:452` |
| Bibite (also includes dead bodies) | `(body.maxHealth/100) · (1.05 − d/fovR)² · (1.05 − θ/(fovA/2))` | `:467` |

Additional filters:
* pellets: skip if null, `amount < 0.01`, held by this bibite (`PelletIsHeld`), or not in view (`:421-446`).
* corpses: skip if null or not in view (`:447-459`). Corpses are **also still processed by the bibite loop** (they are in `seenBibites`), so a corpse contributes to both channels `[V]`.
* bibites: skip if null or not in view; no alive/dead filter (`:460-489`).
* `PelletIsHeld` (`:523-536`) quirk: it loops `i < nHeld`, *assigns* `flag` each iteration (not OR), and on a match swaps the last held entry into slot `i` and decrements `nHeld` **while continuing the loop** — is a miss-it-if-not-last bug. `nHeld`/`held` are re-read from the mouth at the start of every `ComputeSenses` (`:413-420`), so the damage is limited to skipping the exclusion for that object. Mirror the intended behaviour in Rust (exclude held pellets) and note the divergence (§13 Q9).

Accumulators and derived outputs (`:396-521`):

```text
plantWeightSum / meatWeightSum / bibiteWeightSum = Σ weights
plantDirSum  = Σ dir2Target·w   (dir2Target is distance-scaled, NOT unit!)
meatDirSum   = Σ dir2Target·w
bibiteDirSum = Σ dir2Target·w
if sum > 0: dirSum /= sum        # weighted mean of distance-scaled direction vectors

plantConcentrationAngle  = 2·SignedAngle(up, plantDirSum)  / viewAngle
meatConcentrationAngle   = 2·SignedAngle(up, meatDirSum)   / viewAngle
bibiteConcentrationAngle = 2·SignedAngle(up, bibiteDirSum) / viewAngle
plantConcentrationWeight  = plantWeightSum > 0 ? 1 − min(|plantDirSum|  / R, 1) : 0
meatConcentrationWeight   = meatWeightSum  > 0 ? 1 − min(|meatDirSum|   / R, 1) : 0
bibiteConcentrationWeight = bibiteWeightSum> 0 ? 1 − min(|bibiteDirSum| / R, 1) : 0
nPlants = #plant pellets; nMeat = #meat entries (pellets + corpses); nBibite = #bibite entries
maxPlantWeight / maxMeatWeight / maxBibiteWeight = max raw weight per channel
```

Notes `[V]`: the angles are normalised to `[-2, 2]` in principle but `|SignedAngle| ≤ 180°` and `viewAngle ≤ 360°` so `|input| ≤ 1` (exactly ±1 only for a 360° view, where `SignedAngle` can be ±180). The "closeness" is derived from the **magnitude of the weighted mean of distance-scaled direction vectors**, not the weighted mean distance; it is ≈ `1 − centroidDistance/R` when all objects lie in one direction.

### 3.6 Herd metrics

All computed in the same bibite loop (`:460-520`) `[V]`:

```text
if any bibite seen:
    herdHeadingVector += bibite.up                       # unnormalised sum
    centerOfHerd      += dir2Target                      # distance-scaled sum, not normalised
    if dist2Target < herdSeparationDistance:
        herdSeparationVector += −dir2Target.normalized · (1 − dist2Target/herdSeparationDistance)
    if dist2Target <= dist2Bibite:                       # note <= : ties favour the later bibite
        dist2Bibite = dist2Target
        if closest.gene.IsReady: targetR/G/B = genes[5],[6],[7]
else: hasHerd = false, herdSumDirection = herdSeparationProjection = 0

if nBibite > 0:
    hasHerd = true
    v = herdSeparationVector.normalized · separationWeight
      + centerOfHerd.normalized       · cohesionWeight
      + herdHeadingVector             · alignmentWeight
    herdSumDirection = SignedAngle(up, v)
    herdSeparationProjection = cos(π/180 · Angle(up, herdSeparationVector))
```

Defaults: `herdSeparationDistance = gene.HerdSeparationDistance × d1Size` (set in `BibiteBody.Set2DSize`, `BibiteBody.cs:722`; gene default 35 u, `Here/SettingScripts/BibiteEditorSettings.cs:358-371`), weights default separation 1.0, alignment 1.5, cohesion 0.75 (`:294-356`) but are overwritten from genes in `FinalizeBirth` (`BibiteBody.cs:513-517`). `disableHerding` setting removes the herding output synapse from the processing structure and disables the bibite vision mask when nothing else needs it `[V]`.

Usage: `herdSumDirection` feeds a `tanh(herdSumDirection/90)` turn blend in propulsion; `herdSeparationProjection` scales a separation push (`Here/SimulationScripts.BibiteScripts/BibitePropulsion.cs:266-271`) — body spec owns that.

### 3.7 Update cadence (three nested throttles)

`BibiteBody.FixedUpdate` (`:592-639`) `[V]`:

```text
brainTicksCount += 1
if brainTicksCount >= brainTickFactor:                 # brainTPS, default 2 ticks
    visionLookupCount += 1
    if visionLookupCount >= visionLookupPeriod:         # visionLookupUpdateFactor, default 10 brain updates
        fow.FindSeenEntities(); visionLookupCount -= visionLookupPeriod
    visionSensingCount += 1
    if visionSensingCount >= visionSensingPeriod:       # visionSenseUpdateFactor, default 5 brain updates
        fow.ComputeSenses(); visionSensingCount -= visionSensingPeriod
    brain.Thinker()                                     # uses the (possibly just refreshed) senses
    brainTicksCount -= brainTickFactor
```

* At the game defaults (TPS 30) a brain update runs every 2 ticks = 15 Hz; the **entity list** refreshes every 10 brain updates = 1.5 s; the **sense values** (angles/weights) refresh every 5 brain updates = 0.75 s `[V]`. With the common 40 TPS scenarios these become 20 Hz / 2.0 s / 1.0 s.
* Counters are seeded with `Random.Range(0, period)` (Unity RNG ints) at birth/resume, so bibites are **phase-offset** (`BibiteBody.cs:436-438,454-456`) `[V]`. This is a per-entity random draw that a deterministic engine must reproduce from a derived per-entity RNG stream.
* `FindSeenEntities` runs *before* `ComputeSenses` in the same brain update when both are due; `Thinker()` consumes `Nodes[i].Value` snapshots taken at the start of `UpdateSenses` (`NEATBrain.cs:570-610`) `[V]`.

**Rust implementation (task-128).** `World::step_sensing` separates the two
phases: on a lookup tick it refreshes `BibiteRecord.fov_seen` (body-owned
`FovSeenCache`: pellet/bibite ids from the deterministic spatial-hash query, no
positions); on a sense tick it builds the FOV channels from those cached ids,
resolving live stores and applying the normal held/amount/dead filters. A cache
is empty on spawn/save-load until the first lookup tick, so the first sense tick
after loading sees no plants (the acquisition-gap behaviour). The cache travels
in `BibiteCheckpointRecord` and is restored after `spawn_loaded`; the counters
are part of `SenseThrottle`. Tests: `fov_sense_before_lookup_uses_empty_cache`
and body's checkpoint round-trip.


### 3.8 Brain-input mapping (world senses)

`NEATBrain.UpdateSenses` (`:570-610`) `[V]` — these are the exact world-sensor clauses (input indices from `NEATBrain.Inputs`):

| Input idx | Name | Value |
|---|---|---|
| 9 | `BibiteCloseness` | `fow.bibiteConcentrationWeight` |
| 10 | `BibiteAngle` | `fow.bibiteConcentrationAngle` |
| 11 | `NBibites` | `fow.nBibite / 4` |
| 12 | `PlantCloseness` | `fow.pelletConcentrationWeight` |
| 13 | `PlantAngle` | `fow.pelletConcentrationAngle` |
| 14 | `NPlants` | `fow.nPlants / 4` |
| 15 | `MeatCloseness` | `fow.meatConcentrationWeight` |
| 16 | `MeatAngle` | `fow.meatConcentrationAngle` |
| 17 | `NMeats` | `fow.nMeat / 4` |
| 18/19/20 | `RedBibite`/`GreenBibite`/`BlueBibite` | `fow.targetR/G/B` (gene values 5/6/7 of the closest bibite, `[0,1]`) |
| 24/25/26 | `PheroSense1..3` | `tanh(phero.pheroSumN / 5)` |
| 27/28/29 | `Phero1Angle..` | `phero.angleToPheroN` |
| 30/31/32 | `Phero1Heading..` | `phero.angleToPheroNHeading` |

The `n/4` divisions are applied **at input write time**, not in FOV `[V]`.

---

## 4. Sensor: Pherosense (smell)

`Here/SimulationScripts.BibiteScripts/Pherosense.cs` `[V]` unless noted. Unlike FOV, pheromone sensing is **not throttled** — `BibiteBody.FixedUpdate` calls `pherosenseOrgan.PherosenseAround()` every tick (`BibiteBody.cs:630`), and the file's `SensePeriod = 0.5`, `progress` fields are dead code (`Pherosense.cs:16,18`).

### 4.1 Enable & radius

`StartPherosensing()` (`:57-68`): runs only if the brain wires any of input nodes 24–32 (`NEATBrain.StartUsedSystems`, `NEATBrain.cs:533-538`). It caches:
* `pheroDist = gene.genes[15]` — the `PheroSense` gene (default 150 u, range 0–500, `BibiteEditorSettings.cs:279-292`). **Not** scaled by body size, and not refreshed after birth `[V]`.
* a 128-collider buffer.

If not enabled, all phero inputs stay 0 and `PherosenseAround` no-ops.

### 4.2 Sampling & sums (exact)

`:70-215`:

```text
pheroSum1..3 = 0; headingConcentrationR = 0; angle sums = 0
n = Physics2D.OverlapCircleNonAlloc(self.pos, pheroDist, buf, pheroMask)     # 128 max
for i in 0..n-1, skip destroyed:
    spot = buf[i].GetComponent<PheromoneSpot>()
    d    = spot.pos − self.pos
    if |d|² <= 0: skip
    if spot.Rstrength > 0:  s = Rstrength / |d|;   pheroSum1 += s;      RdirSum += d·s;    RheadingSum += spot.heading·s
    if spot.Gstrength > 0:  s = Gstrength / |d|;   pheroSum2 += s;      GdirSum += d·s;    GheadingSum += spot.heading·s
    if spot.Bstrength > 0:  s = Bstrength / |d|;   pheroSum3 += s;      BdirSum += d·s;    BheadingSum += spot.heading·s
# red-death pseudo-pheromone (only when enableRedDeath):
if redDeathEnabled:
    Δ = safeRadius − |self.pos|
    if pheroDist > Δ:
        k = 100 / max(1, Δ)
        pheroSum1 += k
        RdirSum   += self.pos.normalized · (Δ·k)
        RheadingSum += −self.pos.normalized · k
# angles, normalised by 180°:
angleToPheroN        = SignedAngle(up, dirSumN)      / 180
angleToPheroNHeading = SignedAngle(up, headingSumN)  / 180
```

* Strength contribution is **inverse distance**, not inverse square: `Σ strength/d` over all spots in radius.
* The direction vector is `Δpos · (strength/d)`, i.e. a weighted unit vector times strength — then angles only, so the magnitude is irrelevant `[V]`.
* `spot.heading` is the spot's stored heading (unit vector set at emission; tower heading = `right.rotated(dir)`).
* Red-death term: `Δ = safeRadius − |pos|`; the `pheroDist > Δ` guard is odd (it fires only when the bibite is within `pheroDist` of the bloom edge, and adds `100/max(1,Δ)` — diverging as the bibite approaches the edge, deliberately a "danger smell") `[V]`. `safeRadius = +∞` when red death is disabled.
* The tanh saturation is **not** here; it is applied when writing brain inputs: `tanh(pheroSum/5)` (`NEATBrain.cs:601-603`).

### 4.3 Open questions on phero sensing

* `pheroDist` is read once at `StartPherosensing`; growth/`Set2DSize` does not update it. A bibite born with `PheroSense=150` keeps 150 u even if it grows 3×. Mirror this (do not scale by size) `[V]`.
* The repo's `Pherosense.cs` has a stray `using SimulationScripts.Events;` for RedDeath (`:3`), confirming the coupling.

---

## 5. Pheromone spots

### 5.1 The `PheromoneSpot` object

`Here/SimulationScripts/PheromoneSpot.cs` `[V]`, fields `:11-45`:

| Field | Type | Meaning |
|---|---|---|
| `heading` | Vector2 | unit direction of the pheromone "flow" (used by `Phero*Heading`); **NOT persisted** (see §5.5) |
| `Rstrength`, `Gstrength`, `Bstrength` | float | channel strengths in u² (emitted amount); sensing sums `strength/d` |
| `nR`, `nG`, `nB` | float | *visual particle* emission remainders — no simulation effect |
| `saveSpot` | bool | set by callers; never read (dead flag) |
| `dissipate` | bool | whether strengths decay; towers pass `false` |
| constants | | `RateOfEmission = 5` (unused), `RateOfDissipation = 0.25`, `DissipationPeriod = 0.25` |

### 5.2 Creation paths

1. **Bibite emission** (`Here/SimulationScripts.BibiteScripts/BibitePheromoneOrgan.cs:63-83` `[V]`):
   ```text
   progress += dt; if progress < 1: return; progress -= 1              # once per simulated second
   r = preventRed   ? 0 : brain.Output(PhereOut1) · pheromoneProductionStrength   # default 10
   g = ... ; b = ...
   if r+g+b > 0.05:
       spot = GenerateNewPheromonesSource(organ.position)             # under pheromoneHolder
       spot.InitPheromones(r,g,b, organ.up)                          # saveInFile=true, dissipate=true
       body.UseEnergy((r+g+b) · pheromoneProductionCost)             # default 0.01 E per unit sum
   ```
   Outputs are ReLU nodes (`[0, +∞)`, clamped ±1024) so strengths are non-negative at emission. Emission period 1 s is sim-time (FixedUpdate). A spot is a leaf object with **no pooling/merging**; every second each emitting bibite creates a new spot.
2. **Pheromone tower** (`Here/SimulationScripts/PheroTower.cs:66-97`): owns exactly one persistent spot; `InitPheromones(r,g,b, Vector2.left.Rotate(dir), saveInFile:false, dissipateSpot:false)` then `UpdatePheroSpot()` sets `SetPhero(r,g,b, Vector2.right.Rotate(dir))` and rotates the tower transform. The spot never dies (dissipate false, strengths > 0 while the tower exists; a tower with all three = 0 leaves a spot that is *not* auto-destroyed because `dissipate == false` in the destroy guard).
3. **Red Death** particles emit into the visual particle system only; the simulation side injects a pseudo-pheromone in `Pherosense` (§4.2) `[V]`.

### 5.3 Dissipation — exact

`PheromoneSpot.FixedUpdate` (`:96-145`) `[V]`:

```text
if Time.timeScale <= 0: return
dissipationProgress += dt
if dissipate and all strengths <= 0: Destroy(gameObject); return     # note: checked before decay
if dissipationProgress < 0.25: return
n = floor(dissipationProgress / 0.25) · 0.25 ; dissipationProgress -= n
for each channel with strength > 0:
    if dissipate: strength -= n · 0.25
    nX += 1.25 · pheromonesStrength            # visual only
emit particles at position with nX,nG,nB ; nX %= 1 ...
```

* Effective decay rate = **0.25 strength/s** (`n·0.25` per `n` seconds), applied in 0.25 s quanta `[V]`. **Capture-confirmed** by parity task-18 (`−0.0625` per 0.25 s quantum, spot destroyed once all strengths `≤ 0`; `docs/PARITY.md` Appendix A3): the code implies **40 s** for a strength-10 spot before it hits 0. The "1 u²/s / ~10 s lifetime" claims in two secondary documents are wrong. §13 Q10 closed.
* Because decay is `strength -= …` without a clamp, strengths can go **negative**; negative strengths are ignored by sensing (`> 0` guards) and by the destroy guard (`> 0` test is on all three `<= 0`, so a spot with one negative and others zero is destroyed, but a spot with a positive channel and a negative channel survives) `[V]`. Real saves contain negative values `[C]`.
* The 0.25 s quantum means a spot created with strengths below 0.0625 dies after ~0.25 s; strengths between 0 and 0.25 flicker.
* `InitPheromones` sets `nX = sqrt(userPheromonesStrength)·U(0.5,1.5)` when the user visual setting > 0.01 (purely cosmetic) and attempts to rotate a local quaternion copy (no effect) (`:59-78`) `[V]`.

### 5.4 Sensing grid & pooling

There is **no diffusion and no grid**: pheromone sensing is an `OverlapCircle` over the actual spot colliders (`pheroMask` layer), capped at 128 (`Pherosense.cs:166`). "Pooling" in the sense of accumulating nearby sources happens in the `Σ strength/d` sum. Two consequences for Rust:
* A dense field of many weak spots is aggregated by a radius query; implement with the same spatial hash layer + radius query and a stable spot iteration order for determinism.
* `pheroMask` is a prefab-configured `LayerMask` (a "pheromones" layer) — the Rust core should treat pheromone spots as their own spatial class.
* Spots are destroyed by `Object.Destroy` which is deferred to end of frame; a spot may therefore be sensed in the same tick it is scheduled for destruction `[I]` (§10.5).

### 5.5 Persistence (and the heading-loss bug)

`SaveSystem` saves **every child of `pheromoneHolder`**, regardless of `saveSpot` (`Here/ManagementScripts/SaveSystem.cs:231-237`) `[V]`. Each entry:
```json
{"transform":{"position":[x,y],"rotation":0.0,"scale":1.0},
 "phero":{"heading":{},"Rstrength":2263.666,"Gstrength":-0.01475811,"Bstrength":0.0,"nR":0.44,"nG":0.98,"nB":0.26}}
```
`PheromoneSpot` does not implement `ISaveable`; the generic serializer writes only `[SerializeField]` fields, and `Vector2 heading` serializes to `{}` because `Vector2`'s components are not `[SerializeField]` `[V]`. On load, `LoadPheromone` instantiates, restores the transform, then deserializes `phero`; the heading field stays at its prefab default (zero) `[V]`. **Capture-confirmed** by parity task-18: saved `heading` is `{}` and stays lost after reload, while strengths round-trip (`docs/PARITY.md` Appendix A3). So **saved bibite spots lose their heading**; their `Phero*Heading` contribution disappears after a reload (tower headings survive because they are stored in the tower's `dir` and re-derived in `InitTower`). `dissipate`/`saveSpot` are not serialized either; loaded spots get prefab defaults (`dissipate=true`), which matches bibite spots. Rust must reproduce this exact on-disk shape for round-trip; whether to reproduce the heading loss semantically is a fidelity decision (§13 Q11).

---

## 6. World tools (placeable objects)

Stored in `scene.bb8scene` and `towerHolder`; share `EffectTower.maxID` / `vars.bb8scene/towerMaxID` (`Here/SimulationScripts/EffectTower.cs:22-37`; `SaveSystem.cs:182,238-268,606-636;466-472` `[V]`). All are user-placed (placement code in `EffectTower.Update`/`ColorKiller.Update` is editor-only; headless engine only loads/simulates placed ones). Each has an optional zone attachment: `EffectTower.FixedUpdate` snaps the tower to the zone's position every tick (`EffectTower.cs:58-65`); ColorKiller does the same via `attachedSpawner` (`ColorKiller.cs:120-133`) `[V]`.

### 6.1 `EffectTower` base

Fields: `ID: ulong` (`= ++maxID` if 0), `placed`, `loaded`, `attachedZone: TargetZoneSetting` (`EffectTower.cs:14-54`). Save shape: `{attachedZone?: id, id, x, y}` (`:122-140`); load: sets `placed = true`, restores ID/position, records `maxID` (`:142-158`).

### 6.2 RadTower (radiation)

`Here/SimulationScripts/RadTower.cs` `[V]`:

| Setting | Default | Range | Unit |
|---|---|---|---|
| `count` (Count Rate) | 0.001 | 0–0.1 | mutation-count units/s |
| `intensity` (Intensity Rate) | 0.001 | 0–0.1 | σ/s |
| `radius` | 250 (but `InitTower` sets `radius.maxValue = S/2` and default `min(250, maxValue/2)`; loaded values usable up to 10000) | 10–10000, clamped to `S/2` as slider max | u |
| `taper` | 1 | 0–3 | — |

Effect, `RadioArea.FixedUpdate` (`Here/OneUseScripts/RadioArea.cs:64-93`) `[V]` — **every 0.5 s** (`period = 0.5`, `progress -= 0.5`, one dose per fixed period):
```text
for gene in affectedGenes:                    # filled by trigger enter/exit of bibitePart colliders
    d = |bibiteGene.pos − area.pos| / radius
    if d > 1: skip
    f = 1 if taper ≈ 0 else 1 − d^(1/taper)   # taper=1 → linear falloff to 0 at edge
    gene.IncreaseMutationFactor("Radioactivity", count·0.5·f, intensity·0.5·f)
```
`IncreaseMutationFactor` **accumulates** into `mutationFactors["Radioactivity"]` (count and σ add) and then recomputes totals as the sum over sources (`Here/SimulationScripts.BibiteScripts/BibiteGenes.cs:501-531,248-252`) `[V]`. The dose is **not persisted** in `BibiteGenes.SaveState` (`:533-590`) — save/load (and autosave-reload) wipes accumulated radiation `[V]` (§13 Q12).
Trigger membership is maintained by `OnTriggerEnter2D`/`OnTriggerExit2D` on the area's `CircleCollider2D`; only non-trigger colliders tagged `bibitePart` are tracked, with per-gene dedupe (`RadioArea.cs:95-117`). Headless equivalent: every 0.5 s, query bibites whose **body capsule intersects the circle** (parts), dedupe by bibite, apply dose. The C# triggers fire only on enter/exit, physics-step dependent; a headless geometric query on the dose tick is behaviourally equivalent for the common case and simpler \( [I] \).

### 6.3 PheroTower (pheromone source)

`Here/SimulationScripts/PheroTower.cs` `[V]`:

| Setting | Default | Range | Unit |
|---|---|---|---|
| `redStrength` | 0 | 0–100 | u² (channel strength) |
| `greenStrength` | 0 | 0–100 | u² |
| `blueStrength` | 0 | 0–100 | u² |
| `heading` | 0 | 0–2π | radians; direction = `Vector2.right.Rotate(heading)` |

Owns one non-dissipating spot (§5.2); heading applied to the spot via `SetPhero`, and the tower transform's `right` is set to the same vector (`:94-96`) `[V]`. Save shape `{…base, r, g, b, dir}`; load restores settings (`:99-116`). A tower with all-zero strengths still exists and adds no sensing contribution (guards `> 0`).

### 6.4 ColorKiller (colour-band culler)

`Here/SimulationScripts/ColorKiller.cs` `[V]`; UI in `Here/UIScripts/ColorKillerPanel.cs`; saved under `colorSelectors` using a `transform` block (not `x/y`).

Fields (defaults are **prefab-serialized, not in code**; a captured instance is the only source of values) `[C]`:

| Field | Captured example | Meaning |
|---|---|---|
| `transform.position` | `[-2275.29, 801.77]` | world position |
| `transform.rotation`,`scale` | 0, 1 | unused |
| `hue` | 0.0 | target hue, `[0,1]` (`Color.HSVToRGB(hue,1,1)` for the crystal/laser) |
| `tolerance` | 0.0749 | hue-distance threshold; slider UI, not serialized with a default in code |
| `bodyOrEye` | false | false → body colour; true → eye colour (body hue shifted by `EyeOffset` gene) |
| `anyOrMost` | true | false → kill first candidate; true → kill the candidate with the **largest** hue distance |
| `radius` | 3033.4 | overlap-circle radius; UI slider min 100, max `SimulationSize` |
| `period` | 143.6 | seconds between kill scans |
| `progress` | 0.0 | accumulator persisted in save |

Algorithm, `FixedUpdate` (`:120-184`) — runs every tick but only fires when `progress ≥ period`; uses `progress -= period` (single subtract, so long frames cannot cause multiple kills):
```text
if attachedSpawner != null: transform.position = attachedSpawner.position
progress += dt
if progress < period: return
progress -= period
Physics2D.OverlapCircleNonAlloc(pos, radius, targets, bibiteMask)       # 128, NOT cleared
best = 0 ; bestObj = null
for i in 0..127 while targets[i] != null:                              # stale-entry bug (§13 Q13)
    genes = targets[i].transform.parent.GetComponent<BibiteGenes>()
    if genes == null: continue
    col = genes.GetBodyColor()
    if Random.value > col.a: continue                                  # translucent bibites partly immune
    (h,s,v) = RGBToHSV(col)
    if bodyOrEye: h = h − gene(EyeOffset)
    h = wrap01(h)
    d = min(|h − hue|, ||h − hue| − 1|)                                 # circular hue distance ∈[0,0.5]
    if !bodyOrEye: d = 1 − (1 − d)·s·v                                  # desaturated → reads as "different"
    if d < tolerance: continue                                          # INSIDE the band → spared
    if !anyOrMost: kill(genes.body); break
    if d >= best: best = d; bestObj = body
if anyOrMost && bestObj != null: kill(bestObj)
```
**Critical semantics:** as compiled, the killer kills bibites **outside** the tolerance band around `hue` — it is a "keep this colour lineage, cull deviants" tool, not "kill matching colour". This is the opposite of what the supplemental GAME_GUIDE/brain reference state; the control flow is unambiguous (`if (!(num5 < tolerance)) { …kill… }`, where `num5` is the hue distance, and `anyOrMost` selects the **largest** `num5`) `[V]`. **Capture-confirmed** by parity task-18 (5/5 cases including the decisive inverted-band test, all four `bodyOrEye`×`anyOrMost` combinations; `docs/PARITY.md` Appendix A, `testdata/parity/experiments/color_killer_polarity`), so the compiled branch is also the real-game behaviour — no ambiguity remains (§13 Q5 closed).
* `KillTargetBibite` fires a visual laser and calls `bibite.Die()`; `killed` counter; at most one kill per fire (`:186-205`) `[V]`.
* `period`/`progress` semantics mean the first scan happens `period` seconds after placement/load (progress persisted).
* The 128-entry `targetsInView` buffer is **never cleared** by `OverlapCircleNonAlloc`, and the loop condition is `i < 128 && targets[i] != null` (not `i < returnedCount`), so stale entries from a previous scan are re-processed until a null slot is hit — a real bug that can scan bibites outside the current radius after they leave `[V]`. Capture will reveal its practical effect; decide explicitly (§13 Q13).
* Save shape: `{transform:{position,rotation,scale}, hue, tolerance, bodyOrEye, anyOrMost, radius, period, progress, followingZone?}` (`:258-294`). Load calls `Place()` (sets `placed=true`) and restores the following zone by ID.
* ColorKiller is **not** an `EffectTower`: it has no `ID` field and does not participate in `EffectTower.maxID`/`towerMaxID`; it merely lives in the same `towerHolder` transform so it is saved with the other tools (`SaveSystem.cs:238-252`; `vars.bb8scene/towerMaxID` belongs to `EffectTower`, `:182,606-609`) `[V]`.

---

## 7. World boundary, shade and the Red Death bloom

### 7.1 Shade / bounds behaviour (world-facing constants)

| Constant | Value | Ref |
|---|---|---|
| shade start radius | `1.5·S` | `Here/OneUseScripts/BackgroundManager.cs:60-66` |
| shade fade length | `1000 u` (constant) | `:68` |
| shade end radius / Red-Death coordinate space | `1.5·S + 1000` | `:66` |
| pellet out-of-bounds cull | `|pos| > 1.5·S + 500` (only when shade enabled, at census points) | `Here/SimulationScripts/ZoneManager.cs:228-235,250-257` |
| shade-avoidance trigger | `|pos|² ≥ 2.25·S²` i.e. `|pos| ≥ 1.5·S`; blend over `[1.5S, 2.25S]` via `InverseLerp(d², 1.5·d²)` | `Here/SimulationScripts.BibiteScripts/BibitePropulsion.cs:78,151,293-302` |
| world-wrap trigger | `|pos| − bodyLength ≥ 1.5·S + 1000`; teleport to `−pos.normalized · (1.5·S + 1000 − 0.95·bodyLength)` | `:303-306` |
| void-avoidance trigger | outside every zone's radius; blend by `(dist − zoneRadius)/voidAvoidanceDistance` | `:272-292` |
| settings defaults | `shadeOutsideOfBounds=true`, `shadeAvoidance=false`, `worldWrapping=true`, `voidAvoidance=false`, `voidAvoidanceDistance=100` | `Here/SettingScripts/ScenarioSettings.cs:294-315,390-413` |

Headless engine: no walls; implement exactly the three behaviours above inside the body/propulsion system (owned by body-dev) driven by `World` settings; the world module supplies `sim_size`, `shade_start/end`, per-zone positions/radii for void avoidance.

### 7.2 Red Death bloom

`Here/SimulationScripts.Events.RedDeathBloomManager.cs` `[V]`, placed with 32 angular trigger segments (`N = 32`, `OnRedDeathEnterExitTrigger`).

**Geometry** (`:185-208`):
```text
fill = clamp01(redDeathFill)                       # default 0, settings range 0..1
tRadius = 1 − sqrt(1 − fill)                       # area-normalised fill 0..1
safeRadius = shadeEnd · (1 − tRadius)              # initially +∞ when disabled
pheroRingArea = π · shadeEnd² · fill
```
The bloom is the annulus outside `safeRadius`; the visual ring is `(shadeEnd − (safeRadius − shadeFadeLength/4)) / 2` wide (`:198-201`). Trigger segments are thin boxes at radius `(1 + w − t)/2` with `w=0.5` and `t = tRadius` (`OnRedDeathEnterExitTrigger.cs:17-38`), i.e. they sweep outward as the fill grows. Headless equivalent: a bibite/pellet is "in the bloom" iff `|pos| ≥ safeRadius` (the trigger annulus is a thick ring; the manager re-checks `|pos| < safeRadius` on every damage tick and removes entries that are safe, so the effective predicate is exactly `|pos| ≥ safeRadius`) `[V]`.

**Damage cadence** (`:224-273`) — every 0.5 s of sim time:
```text
for bibite in tracked:
    if |pos| >= safeRadius:
        acc[bibite] += redDeathDamageVelocity · 0.5
        bibite.Hurting(acc[bibite] · 0.5)
        if dead: remove
    else remove
for pellet in tracked:
    if |pos| >= safeRadius:
        acc[pellet] += redDeathDamageVelocity · 0.5
        pellet.RemoveAmount(acc[pellet] · 0.5)
        if amount <= 0: remove
    else remove
```
On entry (`OnBibiteEnter`/`OnPelletEnter`): `acc = redDeathBaseDamages` (default 0.5) (`:275-283`). So an entity that never leaves takes, at the k-th 0.5-s tick, `0.5·(base + k·0.5·velocity)` damage; with defaults that is 0.2875 HP at t=0.5 s (k=1), and the per-tick damage grows by `0.25·velocity = 0.0375 HP` each tick, i.e. DPS ≈ `base + velocity·t` (0.5 HP/s immediately after entry, ~2 HP/s after 10 s) — a slow escalating DoT, not instant death. Pellets are destroyed the same way (`RemoveAmount`), effectively deleting food. Entities that move back inside `safeRadius` stop taking damage but keep their accumulated `acc` (only removed from the dictionary; re-entry re-initialises `TryAdd` only if the key is absent — since it was removed, **acc resets to base on re-entry**) `[V]`.

Settings (`Here/SettingScripts/ScenarioSettings.cs:341-388`):
| Setting | Default | Unit | Note |
|---|---|---|---|
| `enableRedDeath` | false | — | also enables the pseudo-pheromone in `Pherosense` |
| `redDeathFill` | 0 | 0–1 | bloom area fraction |
| `redDeathBaseDamages` | 0.5 | HP (and u² for pellets) | initial accumulator, and effectively base DPS |
| `redDeathDamageVelocity` | 0.15 | HP/s² (u²/s²) | accumulator growth ≈ DPS growth |
Red Death is off by default and absent from a fresh scenario; it does not exist as a spawner — the manager's `FixedUpdate` drives it.

**Pheromone coupling:** the bloom also (a) emits a visual particle stream at `0.0001 · pheroRingArea · userPheromonesStrength`/s (`:69,210-222`) — visual only; (b) adds the red-channel pseudo-pheromone in `Pherosense` (§4.2) so bibites "smell" danger.

**Zone spawn interaction:** `Zone.SpawnAPellet` refuses to create pellets with `|pos| > safeRadius` while red death is on (`Zone.cs:325-328`), which is why the `InitialSeeding` infinite-loop risk exists (§1.6, §13 Q6).

---

## 8. Spawners & initial seeding

### 8.1 Object holders

`WorldObjectsSpawner` instantiates and parents world objects (`Here/ManagementScripts/WorldObjectsSpawner.cs:115-127,242-245`) `[V]`:
| Holder | Contents | Used by |
|---|---|---|
| `freePelletHolder` | pellets not owned by a zone (meat piles from deaths, `PelletProducer` output, tool-placed pellets) | save bucket `"free pellets"`, census, cull |
| `zone.pelletHolder` (one per zone) | plant pellets spawned by that zone | save bucket per zone name, recycle, census |
| `bibiteHolder` | bibites + eggs | tracker, save `bibites/`, `eggs/` |
| `pheromoneHolder` | pheromone spots | save `pheromones.bb8scene` |
| `towerHolder` | towers + color killers | save `scene.bb8scene` |

### 8.2 Initial plant seeding & sim start order

`SimulationManager.Start()` (`Here/ManagementScripts/SimulationManager.cs:50-77`) `[V]`:
1. reset `PlantCounter`/`MeatCounter`, `targetTimeScale = engineTimeScale = 1`, `TimeKeeper.simulatedTime = 0`.
2. `BuildSpawners()`: `ZoneManager.InitializeSpawner()` (creates `Zone` objects from `allZones`, subscribes to zone add/remove, sets `simSize`) then creates the `BibiteSpawner`.
3. `StartSpawners()`: `ZoneManager.StartSpawner()` → for each zone `InitialSeeding()` (instant), then `ResumeSpawner()` (starts census coroutines); then `BibiteSpawner.StartSpawner()` (spawn table, priority shares, spawn coroutine); then auto-panner.
4. Loaded game instead: `SaveController.LoadWorld` → `SaveSystem.LoadGame` which rebuilds zones from settings, resets the world, loads scene/settings/pellets/pheromones/bibites/eggs, then `ZoneManager.UpdateBiomass()` and `ResumeSpawners()`.

`InitialSeeding()` per zone = `while pelletBiomass < initialSeeding·maxBiomass: SpawnAPellet()`; see §1.6 for overshoot and hazards. The zone's `pelletProgress` is untouched by seeding, so the normal production loop starts from 0 after a seed burst.

### 8.3 Bibite spawning (virgin spawn table)

`BibiteSpawner` + `BibiteSpawnInfo` (`Here/SimulationScripts/BibiteSpawner.cs`, `BibiteSpawnInfo.cs`) `[V]`. Settings row (`BibiteSettings`, `Here/SettingScripts/BibiteSettings.cs`):

| Field | Default | Meaning |
|---|---|---|
| `path`, `isExternal` | — | template file inside the save (`templates/…`) or external path |
| `spawnPriority` | 1 (0.01–100) | share of spawns = priority / Σ priority of non-one-time rows |
| `spawnType` | Continuous | `Continuous` vs `OneTime` |
| `minimum` (Target Count) | 10 (0–500) | maintain N **descendants** (species count) |
| `totalSpawned` | 0 | persisted counter for one-time rows |
| `randomizeGenes` | NormalMutations | No / NormalMutations / Color / AllGenes |
| `growthAtSpawn` | Adult | Egg / Baby / Adult / Elder |
| `tagging` | NoTagging | NoTagging / SpeciesTagging / CustomTagging / RandomTagging |
| `customTag` | "" | for CustomTagging |
| `spawnZone` | null = All zones | target zone; spawn position = that zone's `GetSpawnLocation()` else random zone |

Algorithm (`BibiteSpawner.SpawnEntity`, `:124-185`; info in `BibiteSpawnInfo.cs:18-68`) — a **1 s real/sim-time coroutine** (`WaitForSeconds`, timeScale-scaled):
```text
loop forever:
  wait 1 s
  if no spawn rows: continue
  limit = limitBibiteBirth ? bibiteLimit : ∞
  nToLimit = limit − nBibite
  for each row: descendantCount = species.GetDescendantCount()   # recursive including children species
  if all rows minimumMet or nToLimit < 1: continue
  for each row not yet at minimum and not oneTimeSpawned:
      if !oneTime: spawnProgress += virginSpawnRate · spawnShare · 1s
      else:        spawnProgress  = (missingToMinimum + 0.1) · spawnPriority   # burst all at once
      n = min(missingToMinimum, nToLimit)
      spawned = 0
      while spawnProgress >= 1 and !minimumMet and spawned < n:
          spawnProgress -= 1
          pos = row.zone != null ? row.zone.GetSpawnLocation() : ZoneManager.GetRandomPositionInZone()
          SpawnBibiteFromTemplate(template, randomizeGenes, tagging, growthAtSpawn, pos, angle=null, customTag)
          spawned++; nToLimit--
      row.totalSpawned += spawned
```
* `missingToMinimum = minimum − (oneTime ? totalSpawned : descendantCount)`; `minimumMet` compares the same. "Descendant count" = the species' own count plus all child species counts (§11.3), i.e. speciation makes a template row keep spawning until the whole clade reaches `minimum` `[V]`.
* `spawnShare` is normalised over non-one-time rows only (`:115-122`).
* `virginSpawnRate` default 1 Hz (`ScenarioIndependentSettings.cs:113-124`), so with one row at default priority and 10 minimum, one virgin bibite is spawned per second until 10 exist, then spawning pauses until deaths/speciation lower the descendant count.
* Quirk: `DistributeSpawnPriority` normalises only over non-one-time rows (`BibiteSpawner.cs:115-122`), so a spawn table consisting solely of `OneTime` rows divides by zero (`spawnShare = priority/0 = +∞`, unused) — harmless. But a `OneTime` row's progress is **assigned** every second, `spawnProgress = (missing + 0.1)·spawnPriority` (`:161`); if `spawnPriority < 1/(missing + 0.1)` it never reaches 1 and that row never spawns `[V]`.
* `SpawnBibiteFromTemplate` (`WorldObjectsSpawner.cs:147-190`): `Egg` → creates an egg (hatch path); otherwise instantiate and copy genes/brain, optional `Mutate`/`RandomColor`/`RandomGenes`; `growthAtSpawn` sets the 2-D area multiplier (`Baby`/`Adult`/`Elder`/1.0). RNG: Unity global, plus `Random.Range(0,360)` for orientation (`:154`).
* `AlignInnovationsWithSim()` is called for each template at start (`BibiteSpawner.cs:52-59`) — brain-dev territory.
* `bibiteLimit` ≤ 500, default 100; `limitBibiteBirth` default false (`ScenarioIndependentSettings.cs:92-111`).

**Rust implementation (fresh-scenario start).** `World::start_fresh_scenario_spawner()`
mirrors the coroutine: it runs one immediate iteration (the game's
`yield return null`), then `World::step_spawner(dt)` runs one iteration per
simulated second via `settings.independents.virgin_spawn_rate`; per-row
`spawnProgress`, normalised priority shares, one-time burst assignment,
descendant-count minima (`Species` subtree live bibites+eggs), growth-at-spawn,
tagging and `bibiteLimit`/`limitBibiteBirth` are all honoured. A fresh Ring
scenario therefore produces 1 bibite at t=0 and one per second until 10 exist
(1 at tick 0, 2 at 1 s, 5 at 4 s, 9 at 8 s at 15 TPS). This applies **only to
fresh scenarios**; the game's save-load path does not resume the bibite
spawner, and `World::from_contents` leaves the fresh spawner unstarted.
RNG: the game spawner uses unseeded `System.Random`+`UnityEngine.Random`, so
positions/orientations are seeded streams in RUSTibites — counts/cadence are
structural, entity identity differs (D20). Spawn positions use the per-zone
shape-aware `Zone::spawn_location()` sampler (honouring `SpawnRow.spawn_zone`),
not a generic disc: this matters for `Rect`/`Ring` zones such as Ring's
97,500×2,500 food strips. The legacy `spawn_initial_population` immediate path
uses the same sampler.

### 8.3a Pellet conservation accounting (runtime-only)

`World` accumulates f64 counters for the identity in
`docs/design/FEEDING_METRICS.md` §3: `seeded_*_energy` (initial zone seeding),
`produced_*_energy`/`recycled_*_energy` (`ZoneUpdateReport`), `decayed_*_energy`
(`DecayReport` from `PelletStore::process_decay`), `culled_*_energy`
(`census_and_cull`), `removed_*_energy` (`despawn_pellet`), and
`consumed_plant/meat_u2|energy` from per-bibite body `FeedingCounters` deltas
(sampled in `World::sample_stats`). `World::conservation_snapshot()` adds the
live pellet stock and `terms_missing` (corpse→meat creation is not surfaced
yet). Default 600/3000-tick validation: residual ≈ 4.8e-6 / 7.4e-5 E (~1e-9
relative); counters are runtime-only and never serialized.

### 8.3b Bit-identical checkpoint resume (runtime-only)

`World::capture_checkpoint_state(&self) -> SimSnapshot` and
`World::restore_checkpoint_state(&mut self, &SimSnapshot)` implement
`docs/design/CHECKPOINT_STATE.md` P2. The base snapshot carries tick/time, RNG
streams, decay/animator/red-death timers and spawner accumulator; extensions
carry bibite/egg/pellet/pheromone arenas (layout/generations/free/dense +
runtime fields), per-bibite brain runtime, zone/tool/species runtime, `SimStats`,
the fresh-spawner active flag + per-row progress/species, the exact spatial-hash
layout (query order is behaviour) and a recorded `state_fingerprint` guard.
Restore order: game save → arenas/ids → runtime → RNG/clock → derived rebuild →
fingerprint check (hard error). Runtime-only; never written to game saves.
Validated: Default 300+300 == single 600 and mid-spawn 200+200 == single 400
(Lead CLI, bit-exact); core `checkpoint_round_trip_preserves_populated_world`,
`checkpoint_chunked_resume_matches_single_run` (spawner) and
`checkpoint_round_trip_preserves_empty_world_control_state` pass. Known
low-priority residual: a 120+120 synthetic split can differ ~1e-7 in one body's
velocity/rotation on the first tick after restore (physics contact ordering),
while every captured store/checkpoint field is equal; Default CLI resume is
exact.

### 8.4 Egg laying / hatching (world-visible parts)

Egg laying is organ logic (body spec). World-facing facts needed for the loop:
* Eggs live in `bibiteHolder`, `EggHatching.FixedUpdate` advances `hatchProgress += dt`; aborts if `HatchRatio·totalRequiredEnergy > energy`; hatches at `hatchProgress ≥ hatchTime` (`Here/SimulationScripts.BibiteScripts/EggHatching.cs:159-173`). Hatching creates the bibite in the same tick (`Hatch`, `:175-207`) — the new bibite's first FixedUpdate is the **next** tick.
* Aborting an egg spawns a meat pile with the remaining energy (`Abort`, `:209-228`).
* Eggs are tracked by `BibiteTracker.eggs`, saved as `eggs/egg_N.bb8` and included in counts/biomass stats.

### 8.5 `PelletProducer` (the `isSource` special bibite)

Special-gene bibites (`gene.isSource`) get a `PelletProducer` component (`BibiteBody.cs:543-550`; `Here/OneUseScripts/PelletProducer.cs`) `[V]`:
```text
period = 0.5 / gene.metabolicRate ; pelletSize = body.baseBodyArea · 0.1
every period (FixedUpdate, sim time):
    spawn a free plant pellet at (position − up·8·d1Size) with energy = pelletSize · Plant.energyDensity
```
`isSource`/`isRad`/`isRain`/`isKiller` are special role flags persisted in the gene save (`BibiteGenes.cs:541-590`); `isRad` spawns a `RadioZone` that applies a fixed `MutationIncreases{probability=10, intensity=0.35}` to *other* bibites touching it (`Here/OneUseScripts/RadioZone.cs:38-42,101-129`) — note this is **not** the placeable RadTower and is never in `radTowers` `[V]`.

---

## 9. Global per-tick ordering

### 9.1 What the game actually guarantees

Unity does **not** define an order between `FixedUpdate()` methods of different `MonoBehaviour`s (and `DefaultExecutionOrder` is not used anywhere in the decompiled source — verified by grep). Within one tick Unity does guarantee:

1. all `FixedUpdate()` callbacks,
2. then the 2-D physics simulation step,
3. then `OnTrigger*`/`OnCollision*` callbacks for that step,
4. repeated as needed for catch-up; `Update()`/rendering happen per frame, not per tick.

The source therefore only fixes *within-system* order and a set of **dependencies**; the cross-system order is whatever the build's MonoManager order happens to be `[I]`. The canonical order below is what RUSTibites will implement (it satisfies every dependency found in the C#), and it is the order to validate against the ground-truth capture (§13 Q2).

### 9.2 Canonical tick pipeline (proposed, deterministic)

| # | Phase | What runs | Game source | Cadence |
|---|---|---|---|---|
| 0 | pause gate | if `dt == 0` / paused: no tick at all | `TimeController.cs:117-130` | — |
| 1 | clock | `simulatedTime += dt`; fixed clock tick | `TimeKeeper.cs:149-169` | every tick |
| 2 | settings animators | apply animated scenario settings (`ScenarioSettings.settingsChangers`), `time = simulatedTime` | `SettingsChangerManager.cs:47-74`; `SettingsChanger` | every tick accumulates; evaluates when `progress ≥ 0.1 s` |
| 3 | world tools (positioning) | `EffectTower.FixedUpdate`: attached towers snap to zone position; ColorKiller same | `EffectTower.cs:58-65`; `ColorKiller.cs:120-133` | every tick |
| 4 | zones | per zone, in `ZoneManager.zones` order: movement + bounds; over-cap recycling; biomass production/spawns | `Zone.cs:188-214` | every tick |
| 5 | per-bibite | per bibite (tracker/child order). **Internal order is authoritative in body spec §2.3/§2.6** (`docs/INTERFACES.md` §6.1): base metabolism + brain upkeep → brain (throttled senses → `Brain::step`) → organs in order **armour → stomach → egg layer → fat → move → mouth → pheromone organ** → propulsion forces → `UseEnergy`. World-side FOV/phero sensing is part of the throttled senses slice. | `BibiteBody.cs:592-639,505`; `BibiteGrowth.cs:91-107`; `InternalClock.cs:65-97`; body spec §2.3/§2.6; `docs/INTERFACES.md` §6.1 | every tick; brain parts throttled (§3.7) |
| 6 | eggs | hatch progress → hatch/abort (creates/destroys objects for next tick) | `EggHatching.cs:159-173` | every tick |
| 7 | pheromone dissipation | decay quanta, destroy empty spots | `PheromoneSpot.cs:96-145` | every tick with 0.25 s quanta |
| 8 | pellet decay | decay processor over registered pellets | `MatterDecayProcessor.cs:133-195` | every `decayTPS` ticks (default 2) |
| 9 | rad towers | dose bibites inside `RadioArea` | `RadioArea.cs:64-93` | every 0.5 s per tower |
| 10 | red death | damage bibites/pellets outside `safeRadius` | `RedDeathBloomManager.cs:224-273` | every 0.5 s |
| 11 | color killers | one kill per `period` per killer | `ColorKiller.cs:120-184` | every tick; fires on `progress ≥ period` |
| 12 | pellet producers | `isSource` bibites spawn plant pellets | `PelletProducer.cs:30-47` | each producer's own period |
| 13 | physics step | integrate existing velocities + forces/torques applied in phase 5 (gravity 0), collisions, joints | Box2D (Unity) | every tick |
| 14 | collision/trigger callbacks | pellet merge; red-death enter/exit segment triggers; RadioArea enter/exit; mouth/bite contacts; egg collisions | `MatterPellet.cs:238-258`; `OnRedDeathEnterExitTrigger.cs`; `RadioArea.cs:95-117` | after physics, every tick |
| 15 | data logging | data streams every 60 sim-s; species + zone logging every 300 sim-s; gene stream every 3600 sim-s | `DataLogger.cs:387-409` | see §11.4 |

Body-dev's pipeline (within phase 5) is **authoritative in body spec §2.3/§2.6** (`docs/INTERFACES.md` §6.1): base metabolism + brain upkeep → brain (throttled senses → `Brain::step`) → organs in order armour → stomach → egg layer → fat → move → mouth → pheromone organ → propulsion forces → `UseEnergy`. The game's separate MonoBehaviours make `BibiteGrowth`/`InternalClock` vs `BibiteBody` order undefined in Unity, so RUSTibites fixes it `[I]`. **Corrected 2026-10-29:** the earlier one-line summary (clock→growth→brain→organs→propulsion→`UseEnergy`) was wrong in the organ ordering and omitted the metabolism/upkeep step; this row now follows the body spec, which is the authority.

### 9.3 Timers & throttles (single table)

| System | Period | Clock source | Ref |
|---|---|---|---|
| brain update | `brainTPS` ticks (2 → 15 Hz @30 TPS) | tick counter on bibite | `BibiteBody.cs:604-621`; `ScenarioIndependentSettings.cs:182-192` |
| FOV entity lookup | `visionLookupUpdateFactor` brain updates (10) | brain-tick counter | `:607-612`; `:206-216` |
| FOV sense values | `visionSenseUpdateFactor` brain updates (5) | brain-tick counter | `:613-618`; `:218-228` |
| pheromone sensing | every tick (dead 0.5 s const) | — | `Pherosense.cs:16-18`; `BibiteBody.cs:630` |
| pheromone dissipation | 0.25 s quanta, `-=0.25/s` | FixedUpdate | `PheromoneSpot.cs:110-115` |
| bibite pheromone emission | 1 s | FixedUpdate | `BibitePheromoneOrgan.cs:68-71` |
| pellet decay | `decayTPS` ticks (2) | tick counter | `MatterDecayProcessor.cs:149-154`; `ScenarioIndependentSettings.cs:194-204` |
| zone biomass recount | 10 s (scaled, coroutine) | `WaitForSeconds` | `Zone.cs:54,409-416` |
| zone production | every tick | FixedUpdate | `Zone.cs:202-213` |
| Red Death damage | 0.5 s | FixedUpdate | `RedDeathBloomManager.cs:230-235` |
| RadioArea dose | 0.5 s | FixedUpdate | `RadioArea.cs:70-75` |
| ColorKiller scan | per-killer `period` (default not in code) | FixedUpdate | `ColorKiller.cs:134-139` |
| settings animators | 0.1 s | FixedUpdate | `SettingsChangerManager.cs:19,47-55` |
| data logs | 60 s; species/zone 300 s; genes 3600 s | FixedUpdate accumulator on sim time | `DataLogger.cs:23-27,387-409` |
| bibite spawner tick | 1 s (scaled coroutine) | `WaitForSeconds` | `BibiteSpawner.cs:124-131` |
| zone census coroutine | 1 s (UI readouts only) | `WaitForSeconds` | `ZoneManager.cs:76-80,127-135` |
| habitat cleanup | 600 s scaled (Resources only) | `WaitForSeconds` | `ZoneManager.cs:183-191` |
| pellet census / shade cull | on sim-size change, autosave, load | event-driven | §2.7 |

### 9.4 Frame-rate dependence & durations

* Everything in §9.3 marked "FixedUpdate" advances in **sim time** and is frame-rate independent.
* Coroutines with `WaitForSeconds` are **scaled by `Time.timeScale`** but scheduled on the render loop; when the frame rate is very low, Unity may run several `FixedUpdate`s between two `Update`s, so a 0.1 s accumulator etc. can overshoot. Deterministic headless engines should implement these as tick-count/accumulator systems, not wall-clock timers.
* `TimeController.CheckMinFPS` can reduce `engineTimeScale` below 1 when the user sets a minimum FPS and the game is slow (`TimeController.cs:104-115`); this shrinks `dt` (only when `<1`) and therefore changes simulation trajectories. The headless engine must always run at `timeScale = 1` and document the deviation.
* `Update()`-only logic in the world (skin/shader updates, tower/killer **placement**, popups, camera, tooltips, auto-save scheduling, `ColorKillerPanel`) has **no** effect on simulation state `[V]`. Exceptions to re-check: none found — every world state mutation is in `FixedUpdate`, trigger callbacks, coroutines or load.
* Auto-save uses real time by default (`WaitForSecondsRealtime`) or scaled time (`WaitForSeconds`), and in either mode calls `RefreshPelletBiomassCounter()` → shade cull + zone recounts, and snapshots the world (`SaveController.cs:214-247`). **Saving is therefore a state-mutating operation in this game.** Headless: expose `census_and_cull()` and call it on the same schedule/at save time.

### 9.5 Object creation/destruction mid-tick

* `SpawnAPellet`, pheromone emission, egg hatching, meat piles, bibite spawns all `Instantiate` during FixedUpdate; Unity registers new rigidbodies immediately, so a pellet spawned in phase 4 is simulated by the same tick's physics step (phase 13). Newly created bibites spawned in the spawner coroutine or hatching also get their first FixedUpdate on the next tick (`Instantiate` + `StartBody` sets state; the component's first `FixedUpdate` is next tick).
* `Object.Destroy` is deferred: a pellet/spot/bibite removed during FixedUpdate remains in the scene (and can be found by later queries/`GetAllChilds` in the same frame) until end of frame. For a deterministic headless engine recommend **immediate logical removal** at a defined phase boundary (documented divergence) unless capture shows it changes stats; see §13 Q3.
* `WorldObjectsSpawner.allPellets` / `pelletsOfMaterial` are maintained by `OnPelletDestroy` (fires from `OnDestroy`), so these lists also lag destruction by the deferral.

### 9.6 Random-number streams (determinism)

The game has **two** RNG families and they interleave differently:

| Stream | Used for | Ref |
|---|---|---|
| `UnityEngine.Random` (global static) | FOV/ID phase offsets at birth (`Random.Range(0,n)`), spawn orientation (`Random.Range(0,360)`), pellet spawn positions/directions, meat-pile offsets, ripping chunks, `ColorKiller` translucency roll, zone Free-move noise (Unity `insideUnitCircle`), pheromone visual randoms, bibite template randomization | `BibiteBody.cs:436-438`; `WorldObjectsSpawner.cs:154`; `Zone.cs:183,246,371,383,401`; `MatterPellet.cs:211`; `ColorKiller.cs:154` |
| `System.Random` instances | per-zone pellet size (`Zone.r.NextPowerGaussian(1.5)`), `ZoneManager` random zone index (`ZoneManager.r` exists but unused for index — `Random.Range` is used), some bibite/egg code (`RandomExtension` on `System.Random`) | `Zone.cs:62,324`; `ZoneManager.cs:21,137-140`; `RandomExtension.cs` |

`System.Random` is seeded from the system clock by default, so the game is **not reproducible** from a save file even for the original game; the `UnityEngine.Random` global state is also not saved. Consequences:
* Exact bit-for-bit parity with the original requires instrumenting the game's RNG (the Bridge can do per-tick state dumps, not RNG internals). The realistic target is **behavioural/statistical parity** with a fixed RUSTibites seed, plus exact parity of deterministic *formulas* given identical inputs.
* RUSTibites must replace both streams with the single `Rng` (xoshiro256**, domain-tagged) and document the deviation in `ARCHITECTURE.md` §14.
* For entity-phase offsets (FOV/brain counters) use `Rng::derive(seed, DOMAIN_BRAIN, bibite_id)` or equivalent so parallel execution is order-independent; for zone pellet size use a per-zone derived stream so zone processing order doesn't matter.
* Determinism rules from `docs/ARCHITECTURE.md:116-130` apply: no `std::time`, no thread-local RNG, no hash-map iteration order; parallel sensing/brain phases consume pre-derived streams and must be order-independent.

---

## 10. Persistence, save format & migrations (world sections)

### 10.1 Save bundle layout (a `.zip`)

`Here/ManagementScripts/SaveSystem.cs:176-303` `[V]`, verified against real saves `[C]`:

| Entry | Content | World-relevant |
|---|---|---|
| `settings.bb8settings` | full settings JSON (scalars + `materials` + `zones` + `bibites` + `settingsChangers` + `zoneGroups` + `independents`) | **yes** (zones, materials, spawn table) |
| `scene.bb8scene` | `{version, nPellets, nBibites, colorSelectors[], pheroTowers[], radTowers[], simulatedTime, gameName?}` | **yes** |
| `vars.bb8scene` | `{towerMaxID}` | **yes** |
| `pellets.bb8scene` | `{pellets:[{zone, pellets:[…]}]}` bucketed by zone name / `"free pellets"` | **yes** |
| `pheromones.bb8scene` | `{pheromones:[{transform, phero}]}` | **yes** |
| `speciesData.json` | lineage/species history (`recordedSpecies`, `activeSpeciesList`, `nodeMaxInnovation`, …, optional legacy `speciesData`) | **yes** (see §11.3) |
| `data.bin` | binary history blob: `[DataLogger][BibiteTracker][GlobalLineageManager ≥0.6.0.15][ZoneManager ≥0.6.0.10]` | **yes** (zone/species histories; simulation-irrelevant but round-trip-relevant) |
| `bibites/bibite_N.bb8`, `eggs/egg_N.bb8` | one JSON per bibite/egg (position, genes, body, clock, brain, …) | owned by body/brain/format specs |
| `templates/*.bb8`, `*.bb8template` | bundled templates referenced by the spawn table | yes (spawn table references) |
| `img.png` | screenshot | no |

All JSON is UTF-8 via `StreamWriter(..., Encoding.UTF8)` → games write a **BOM** (`SaveSystem.cs:305-316`; reference doc; fmt-dev owns encoding details). Serialization is Newtonsoft `JObject.ToString(Formatting.None)`.

**Scenario bundles** (`.zip` in `Scenarios/`) share `settings.bb8settings` with saves but contain only: `settings.bb8settings` (= `SerializeScenario(saveRecommendations:true)`: scenario scalars + `zoneGroups` + `materials` + `zones` + `bibites` + `settingsChangers` + `challenge?` + `recommended`), `scenario.info` (`{name, desc, version, isOfficial?, rank?, isChallenge?, star1..3?}`), optional `img.png`, and bundled `templates/` (`Here/SettingScripts/ScenarioSaver.cs:166-200`; `Here/ScriptHelpers/SerializationHelper.cs:283-307`) `[V]`. Starting a scenario = load those settings, then run `SimulationManager.Start` fresh-start path (§8.2); no pellets/bibites are stored in a scenario. `ZoneGroupSettings.GenerateZones()` runs for every group whose `zones` list is empty at load (`SerializationHelper.cs:473-484`).

### 10.2 `scene.bb8scene` fields

| Field | Type | Notes | Ref |
|---|---|---|---|
| `version` | string | game version (e.g. `"0.6.3.1"`); drives migrations | `SaveSystem.cs:184,1040-1052` |
| `nPellets` | int | count across free + zone buckets; informational | `:229` |
| `nBibites` | int | bibites + eggs | `:230` |
| `colorSelectors` | array | `ColorKiller` save objects (`transform` block) | `:242-252,266` |
| `pheroTowers` | array | `PheroTower` save objects (flat x/y) | `:256-259,267` |
| `radTowers` | array | `RadTower` save objects | `:260-263,268` |
| `simulatedTime` | float | `TimeKeeper.simulatedTime` | `:269,684-687` |
| `gameName` | string? | optional world name | `:720-723` |
| legacy `pellets` / `pheromones` | arrays? | pre-0.6.3 saves inline; loader falls back (`LoadScene`, `:637-683`) | `:637-683` |

`vars.bb8scene`: `{"towerMaxID": N}` (`:182,238-268,606-609`).

### 10.3 `pellets.bb8scene`

`{ "pellets": [ { "zone": "<zone name>"|"free pellets", "pellets": [ pellet ] } ] }` (`SaveSystem.cs:201-228`; sample `[C]`).

Each pellet:
```json
{"transform":{"position":[x,y],"rotation":r,"scale":s},
 "rb2d":{"px":x,"py":y,"vx":vx,"vy":vy,"r":r},
 "pellet":{"material":"Plant"|"Meat","amount":A},
 "matterDecay":{"timeAlive":T,"rotAmount":R}}        // only if material.decay (or legacy "meatRot")
```
* `transform` is local transform (`SerializedTransform`: position[2], rotation=localEulerZ, scale=localScaleX) (`Here/ScriptHelpers/SerializedTransform.cs:7-35`); pellet `scale = sqrt(amount/20)`.
* `SerializePellet` also appends `matterDecay` iff the pellet is registered (`SaveSystem.cs:395-424`).
* **Load quirks** (`LoadPellet`, `:534-556`): only `transform.position` is used to place the pellet; saved velocity, rotation and scale are ignored (pellets respawn at rest, identity rotation, scale recomputed from amount). The zone bucket is matched by **zone name** (`FirstOrDefault(zoneName == ...)`) — duplicate names bind to the first zone; `"free pellets"` (or missing `zone`) → `null` holder (`freePelletHolder`); unknown zone name also yields `null` and the pellets land in the free holder `[V]`.
* After loading a zone bucket the game calls `zone.ResetList()` which rebuilds `zonePellets` from the holder's children and recounts (`:661-664`; `Zone.cs:418-433`).
* Legacy: older saves stored pellet arrays directly in `scene.bb8scene["pellets"]` and decay state as `meatRot` with `perishableFactor` (`:541,553`; `MatterMaterialSettings.cs:233-263`) `[V]`.

### 10.4 `pheromones.bb8scene`

`{pheromones:[{transform:{position,rotation,scale}, phero:{heading:{}, Rstrength, Gstrength, Bstrength, nR, nG, nB}}]}` (`SaveSystem.cs:231-237,381-393,558-566`; sample `[C]`). `heading` is always `{}` due to the `Vector2` serializer bug described §5.5; strengths can be negative. `dissipate`/`saveSpot` omitted. Only spots parented under `pheromoneHolder` are saved — bibite-emitted spots and (if the prefab hierarchy places it elsewhere) tower spots. On load `GenerateNewPheromonesSource` → `DeserializePosition` (full transform incl. rotation/scale) → `DeserializeObject(PheromoneSpot, …)`.

### 10.5 Binary `data.bin` (world parts)

`SaveableBinStack` format: for each stack element in order, `int32 length` then `length` bytes of element data; elements are `[DataLogger, BibiteTracker, GlobalLineageManager (≥0.6.0.15), ZoneManager (≥0.6.0.10)]` (`Here/ScriptHelpers/SaveableBinStack.cs:12-51`; `SaveSystem.cs:51-63,270,867-878`) `[V]`.

* **ZoneManager element** (`ZoneManager.cs:294-336`): for each zone in `zones` order: `int32 zoneID`, `int32 nBytes`, then `zone.zoneData.SaveStateBin(...)`.
* **`zone.zoneData` = `LogLikeZoneDataArray`**, a `LogLikePresenceArray<ZoneDataPoint>` with `SerialDataLogger.SerialSpeciesConfig` (11 scales, acquisition minutes ×5) `[V]`:
  * point size **14 bytes**: `posX i16, posY i16` (quantized `clamp(posX,±0.9999)·32767`), `radius u16` (`relativeRadius·32767`, or rect `max(relW/2,relH/2)·32767`), `fertility f32` = `settings.totalGrowth` (E/s), `biomass f32` = `zone.pelletBiomass` (`Here/Utility.DataLogging/ZoneDataPoint.cs:33-64`).
  * serialization header: `int16 indexOfApparition`, `int16 indexOfDisappearance`, `u8 scaleOfDisappearance`, `u8 nScales=scaleOfApparition+1`, then `nScales·(u16 nPresentAtScale + u8 pushCounter)`, then for each saved point a 10-byte-block-major layout: all `posX/posY` interleaved `i16[2n]`, all `radius u16[n]`, all `fertility f32[n]`, all `biomass f32[n]` (the arrays are written consecutively by `SaveArraysToBin`) (`Here/Utility.LogLikePresenceArray.cs:112-164`; `Here/Utility.DataLogging/LogLikeZoneDataArray.cs:59-103`).
  * `nPointsToSave = nPointsPresent + Σ pushCounters`; `present = radius > 0 && fertility > 0` (`ZoneDataPoint.cs:19-29`).
  * Exact compression/push-counter evolution is `LogLikeArraySerial.Push`/`LogLikePresenceArray.NewPointPushedToScale` (`Here/Utility/LogLikeArraySerial.cs:18-76`; `LogLikePresenceArray.cs:52-110`) — a log-like multi-resolution ring buffer (11 scales). Full round-trip requires re-implementing this; for milestone 1 the simplest correct strategy is to **preserve the raw `data.bin` bytes on load and append only if the logging history must stay valid**, or re-emit a structurally valid blob (fmt-dev + world-dev coordinate — §13 Q14).
* **DataLogger element**: `{f32 progress, i32 nMinutes}` + nested bin stack of the 8 data streams (each a `DataParallelStreamGroup`, formats in `DataLogger.cs:38-77`) `[V]`.
* **BibiteTracker element** (`BibiteTracker.cs:180-214`): `i32 nBirth`, `i32 nDeath` (≥0.6.0.9) then nested stack (`deathAgesQuantiles` etc.).
* **GlobalLineageManager element**: per recorded species `i64 speciesID`, `i32 nBytes`, speciesData blob (`GlobalLineageManager.cs:450-488`).

For the headless CLI, `data.bin` is **statistics only** — it never affects the next simulated state; it must merely stay loadable by the game for round-trip. Treat it as opaque bytes + optional history writer.

### 10.6 Settings sections relevant to the world

`settings.bb8settings = { <all ScenarioSettings scalar keys>, "materials": {...}, "zones": [...], "bibites": [...], "settingsChangers": [...], "zoneGroups": [...], "independents": {...} }` for saves; scenarios replace `independents` with `recommended` and drop `settingsChangers`/`challenge` as appropriate (`Here/ScriptHelpers/SerializationHelper.cs:267-307`; `Here/SettingScripts/ScenarioSaver.cs:166-200`) `[V]`.
* Each scalar `FloatSetting`/`BoolSetting`/`IntSetting` serializes via Unity `JsonUtility` as `{"interactableChanged":{...},"Value":…}` — readers must use `Value` (`[C]`).
* `zones[]` = `ZoneSettings.SaveState()`: `name, id, material, distribution, fertility, biomassDensity, pelletSize, renewBiomass?, movement, target?, speed, posX?, posY?, radiusIsRelative, radius|width+height, insideRadius?` (`ZoneSettings.cs:672-726`). Note `renewBiomass` only written when true; `target` only when non-null; `posX/posY` omitted for group templates.
* `materials{materialName+"Settings": {...}}` — Plant/Meat/Fat/Armor serialized by `MatterMaterialSettings.SaveState` (`MatterMaterialSettings.cs:213-231`): `cohesiveness, reactivity, energyDensity, massDensity, minEfficiency, maxEfficiency, decay, [freshTime, decayRate only if decay]`.
* `zoneGroups[]` = `ZoneGroupSettings.SaveStateWithZones`: `{name, scaleWithSim, count, template:{…}, zones:[…]}` (`ZoneGroupSettings.cs:150-205`).
* `speciesData.json` and the binary formats are cross-referenced §11.3.
* Formatting: no whitespace (`Formatting.None`); Newtonsoft field names as above. Enums are written as their C# names (`distribution.val.ToString()` etc.) and parsed with Newtonsoft `JToken.ToObject<TEnum>()` (accepts the written names, case-insensitively, and integers); keep the exact C# casing when writing for maximal compatibility (`ZoneSettings.cs:672-726,740-762`; `BibiteSettings.cs:163-248`).

### 10.7 Migrations affecting the world (in version order)

From `SaveSystem.LoadGame` (`:748-793`) and `SerializationHelper.DeserializeScenario` (`:309-518`), `VersionTracker` (`Here/ScriptHelpers/VersionTracker.cs:9-71`) `[V]`:

| Version boundary | Migration | Ref |
|---|---|---|
| `< 0.5a4` | `scene.pelletSpawner` legacy zone system → synthesize `ZoneSettings.DefaultZone()` per spawn centre (`radius = spawnRadius/S`, `posX/posY = center/S`), plus one full-world zone if >1 centre | `SaveSystem.cs:757-779` |
| `< 0.5.0.6` | `plantGrowth` → `pelletGrowth` | `SerializationHelper.cs:324-327` |
| `< 0.6.0.6` | zone `fertility/biomassDensity/pelletSize/speed` stored as log10 → exponentiate; then normalise each zone's fertility by the sim area (`num5 = S_area·biomassDensity/Σ(...)`) and rescale globals if any exceed 10×max | `SerializationHelper.cs:434-470` |
| `< 0.6.0.7` | `bibiteMassDensity /= 1.9`; `pelletGrowth ·= 0.7`; `pelletEnergy ·= 1.5` | `SerializationHelper.cs:491-505` |
| `< 0.6.0.9` | `BibiteTracker` nBirth/nDeath appear; Fat efficiency divided by 100 if > max | `BibiteTracker.cs:199-213`; `SerializationHelper.cs:410-422` |
| `≥ 0.6.0.10` | `ZoneManager` binary element introduced (`data.bin`) | `SaveSystem.cs:60,322-336` |
| `< 0.6.0.13` | neuron default/mutation chance reset | `SerializationHelper.cs:423-427` |
| `≥ 0.6.0.15` | `GlobalLineageManager` binary element introduced | `SaveSystem.cs:59,450-488` |
| `< 0.6.0.16` | all material `reactivity /= 30`; Fat reactivity = `sqrt(old)·π` | `SerializationHelper.cs:389-400` |
| `< 0.6.0.17` | `bitePeriodFactor /= 35` if > 10 | `SerializationHelper.cs:506-509` |
| `< 0.6.0.10` (lineage) | legacy `speciesData` JSON blob imported via `LineageVersionUpdater` | `GlobalLineageManager.cs:434-447`; `SaveSystem.cs:782-793` |
| `0.5.0.6`+ | `speciesGeneticSpan` reset | `SaveSystem.cs:748-751` |
| fat/reactivity `<0.6.0.9`, `<0.6.0.16` | see above | `SerializationHelper.cs:389-422` |
| material decay legacy | `freshTime = 600 / 6^perishableFactor` when `freshTime` absent; `digestionRate` → `reactivity` | `MatterMaterialSettings.cs:233-263` |

Unsupported-from versions: `VersionTracker.CanUpdateFromVersion` rejects `< 0.4` (and brain changes, §brain spec); `ChangesSinceVersion` warns but tries to load when `≥ 0.6.0.15` (`VersionTracker.cs:47-71`) `[V]`.

### 10.8 Load order (world)

`SaveSystem.LoadGame` (`:690-883`) `[V]`:
1. read settings/species/scene/pellets/pheromones/data.bin; read version; validate can-update.
2. `ResetWorld()`: destroy towers, bibites/eggs, pheromone spots; reset plant/meat counters; destroy old spawner (`:1078-1100`).
3. `SerializationHelper.DeserializeSettings(settings, version)` → scenario + materials + zones + zoneGroups + bibites + changers, zones instantiated later.
4. `GlobalLineageManager.LoadState` (if ≥0.6.0.5).
5. legacy zone migration.
6. `BuildSpawners()` → zone instances created (fresh, unseeded).
7. `LoadScene(scene, pellets, pheromones, vars)` → `towerMaxID`; color killers; phero towers; rad towers; pellets into zone buckets (zone.ResetList after each bucket); pheromones; `simulatedTime`.
8. load bibites then eggs from zip entries; reconstruct parent/child ID links; `data.bin` histories.
9. `ZoneManager.UpdateBiomass()`; `ResumeSpawners()` (census coroutines, bibite spawner, auto-panner); `onLoadingDone`.
Note: **no plant seeding on load** (pellets come from the save; an old save with zero pellets has none); `InitialSeeding` only runs on a fresh start.

---

## 11. Lineage / tracker metrics for CLI stats

### 11.1 `BibiteTracker` aggregates

`Here/ManagementScripts/BibiteTracker.cs` `[V]`. Tracks `bibites: List<BibiteBody>`, `eggs: List<EggHatching>`, `nBirth`, `nDeath`, a rolling age-of-death quantile estimator `FloatRollingQuantileEstimator(nEstimator=3, rollPeriod=20, epsilon=0.05)` and a binary save stack. Quantile mechanics (`Here/Utility/RollingQuantileEstimator.cs:21-54`, `GKQuantileEstimator.cs:33-62`): 3 Greenwald-Khanna estimators (ε = 0.05; compression interval 10) receive every death age; `GetAgeOfDeathData` reads the **oldest** estimator, then `IncrementRollCounter()`. One estimator is reset and rotated every `nEstimator·rollPeriod = 60` reads, and reads happen once per 60 sim-s data sample, so each estimator accumulates ~60 samples ≈ 1 h before being recycled — the age-of-death distribution is documented as a **rolling 1-hour window** (the 3-estimator ring means it takes up to 3 h for an old cohort to leave entirely).

| DataPoint (n values) | Definition | Ref |
|---|---|---|
| Count (3) | `bibites.Count + eggs.Count`, `PlantCounter.Count`, `MeatCounter.Count` | `:49-52` |
| Biomass (4) | `Σ bibite.totalEnergy + Σ egg.energy`, `PlantCounter.Biomass`, `MeatCounter.Biomass`, `ZoneManager.freeBiomass` | `:54-57` |
| Brain size (6) | min/mean/max `nActiveNodes`, min/mean/max `nSynaps` over `allBrains` (bibites ∪ eggs) | `:59-97` |
| Age (4) | 25/50/75th percentile + max of `bibite.age` (hours = `clock.timeAlive/3600`) | `:104-122` |
| Age of death (4) | rolling quantiles 0.25/0.5/0.75/1.0; rolls the window when not peeking | `:124-132` |
| Eggs laid (6) | median/70/80/90th percentile/max/mean of `eggLayer.nEggsLaid` | `:134-147` |
| Birth/death (2) | `nBirth`, `nDeath` since last sample (reset on read) | `:149-157` |
| Gene buckets | `GenesBuckets(allGenes)` — gene histogram buckets | `:99-102` |

`BibiteBirth` increments `nBirth`; `BibiteDeath` removes from list, increments `nDeath`, adds age to the rolling estimator (`:159-178`). `totalEnergy` (used for biomass/energy) is body-summed incl. body+health+eggs+fat, minus the corpse offset for dead bibites (`BibiteBody.cs:256-266`).

**Implementation status (2026-10-30):** `sim::SimStats` samples the count/biomass/vitals/generation/species/
brain/gene/lifecycle/behaviour families directly from live state (`stats_match_game_definitions` covers the game
definitions). The **rolling death-age quantile estimator** and the `data.bin` zone/species history streams are
*not* reimplemented — `data.bin` is an opaque pass-through (D11), so CLI stats report current-value distributions
rather than the game's rolling-window histories.

### 11.2 `ZoneDataPoint` (per-zone history)

Already described §10.5: `posX/posY` normalized×32767 (i16), `radius` relative×32767 (u16; for rect = `max(relW,relH)/2`), `fertility = totalGrowth` (E/s), `biomass = zone.pelletBiomass` — **note it uses the lagging counter**, so zone-history biomass is up to 10 s stale and excludes free pellets. Pushed every 300 sim-s by `ZoneManager.LogAllZonesData()` (`ZoneDataPoint.cs:33-64`; `DataLogger.cs:397-402`).

### 11.3 Species-level records and speciation

`Here/SimulationScripts.BibiteScripts/Species.cs`; `Here/ManagementScripts/GlobalLineageManager.cs` `[V]`:

* `Species`: `speciesID` (monotonic `speciesMaxID++`), `timeCreation = TimeKeeper.simulatedTime`, `generationOfFirstSpecimen`, `genericName`/`specificName`, `favorite`, `description`, `template: BibiteTemplate` (representative genome+brain; saved in `speciesData.json`), `parentID`/`parentSpecies`/`children`/`rootSpecies`, live lists `bibites`, `eggs`, `speciesData: LogLikeSpeciesDataArray`.
* `count = bibites.Count + eggs.Count`; `energy = Σ bibite.totalEnergy + Σ egg.energy` (non-aborting eggs) (`Species.cs:81-83`).
* `GetDescendantCount()` = `count + Σ children.GetDescendantCount()` (recursive; used by the spawn table).
* `GeneticDistanceToIndividual = GeneDistance + BrainDistance`; gene distance = `Σ |Δgene_i| / maxValue_i` (35 genes), brain distance from `BrainDifferencesToIndividual` (disjoint/excess connections, weight deltas, activation deltas, toggles; normalised by counts) (`Species.cs:155-297`).
* Effective speciation span `usedSpeciesSpan = speciesGeneticSpan + max(activeSpecies−20, 0)·speciesSpanScalingFactor` (default 1.5 + 0.05/extra species) (`GlobalLineageManager.cs:73-79`; `ScenarioIndependentSettings.cs:126-154`).
* `CheckNewSpecies(bibite)` called at birth (`BibiteBody.StartBody`):
  * no species → assign to the first active species within `usedSpeciesSpan`, else create new root species.
  * has species and distance < span → keep.
  * else look for a child species with `count ≥ 1` and distance ≤ `span/2` → move there.
  * else (mutant flag dance): if `!isMutant` set `isMutant = true` and return (one-tick grace); else clear flag, take the **parent bibite** (`parent1` else self), remove it from its species, create a new species from the parent as the founding specimen, move both into it (`:194-249`).
* `CheckSpeciesOfEgg(egg)`: nearest active species by distance; if within span assign, else assign the nearest (possibly extinct — may be null) (`:251-270`).
* `LogSpeciesInfo()` every 300 sim-s (`DataLogger` nMinutes % 5): increments `logCounter`, `nRecords` (max 10000), pushes a `SpeciesDataPoint` for every recorded species, updates `maxEnergy`/`maxCount` across active species, prunes the species tree, and removes extinct active species (`GlobalLineageManager.cs:81-96`).
* Pruning (`SpeciesTreeCleanup`): species outside active with `nPointsPresent ≤ speciesPrunePointLimit` (default 2) are merged bottom-up into parents/children (favoring the branch with most points; `PruneSpeciesFromTree` re-parents children, merges speciesData, moves bibites/eggs, drops the node) — templates/favorites (`absoluteKeep`) are never pruned this way (`:98-192`).
* `SpeciesDataPoint` (16 bytes serialized): `count u16`, `energy f32`, `avgPos Vec2`, `posStdDev Vec2`, `posCor f32`; energy-weighted centroid and std-dev with `stdDev = max(10, sqrt(...))`, `count < 2` → identity; exposed derived `espXY` (`Here/Utility/SpeciesDataPoint.cs:24-155`).
* `speciesData.json` shape: `{recordedSpecies:[…], activeSpeciesList:[ids], maxEnergy, maxCount, logCounter, nRecords, nodeMaxInnovation, connectionMaxInnovation, nextSpeciesID, speciesData(legacy)}` (`GlobalLineageManager.cs:361-448`) `[V]`.
* Tagging (`Here/ManagementScripts/TagsManager.cs`; `TagInfo {count, totalEnergy}`) is legacy: tags are per-bibite strings inherited via genes (`speciesTag`), dictionary of `BibiteTag`; useful for stats only (`Tagging` enum: NoTagging, SpeciesTagging, CustomTagging, RandomTagging; `BibiteSettings.cs:102-111`).

### 11.4 Logging cadence (CLI stats should use the same clock)

`Here/Utility/DataLogger.cs:23-27,387-409` `[V]`:
* `periodSeconds = 60` (sim seconds, `progress += dt` on `FixedUpdate`).
* Every 60 s: push all parallel streams (biomass, count, brain size, age, death age, eggs laid, birth/death).
* `nMinutes++`; if `nMinutes % speciesLogPeriod(5) == 0` → `GlobalLineageManager.LogSpeciesInfo()` + `ZoneManager.LogAllZonesData()` → every **300 sim-s**.
* if `nMinutes >= 60` → `nMinutes -= 60`, gene stream logged → every **3600 sim-s**.
* The hour/day/week/month display is UI; `TimeKeeper.ParseTime(simulatedTime)` gives `[hours, minutes, seconds]` (`TimeKeeper.cs:171-180`).

### 11.5 Recommended CLI stats schema (milestone 1)

Emit per sample (default every 60 sim-s, configurable), plus a final summary:
```
tick, sim_time_s, tps(dt), population (bibites), eggs, births_since, deaths_since,
species_active, species_recorded,
bibite_biomass_E (Σ totalEnergy+egg energy), plant_biomass_E, meat_biomass_E,
plant_count, meat_count, free_biomass_E (Σ zone.freeBiomass),
mean/std energy, mean/std health_ratio, mean/std maturity, median age_h,
brain nodes min/mean/max, synapses min/mean/max,
mean genes (35),
zone_count, total_max_biomass, total_growth,
phero_spot_count, tower_count (rad/phero), color_killer_count,
red_death_fill/safe_radius (if enabled)
```
These map 1:1 to `BibiteTracker`/`DataLogger`/`ZoneManager` data so parity diffs are meaningful. Beware: `zone.pelletBiomass` vs `PlantCounter.Biomass` are different quantities (lag vs exact) — report both initially to validate.

---

## 12. Rust design notes (for `world/zone/pellet/pheromone/senses/tools/sim`)

This is a *design contract*, not code. It targets `docs/ARCHITECTURE.md` (SoA, arenas, f32, no deps, deterministic).

### 12.1 Ownership & module map

| Module | Responsibility | Key external interfaces |
|---|---|---|
| `world.rs` | `World` container: `sim_size`, global settings snapshot, `Arena`/stores for zones, pellets, pheromone spots, tools, eggs; spatial hash; tick driver; census/cull; save/load hooks; world-level RNG | `FixedClock`, `Rng`, `Settings` (fmt-dev), `BibiteStore`/`EggStore` (body-dev), `BrainStore` (brain-dev) |
| `zone.rs` | `ZoneData {settings, derived geometry, pellet_progress, pellet_biomass, …}`; per-zone production/spawn/recycle/movement; zone list rebuild (explicit + generated groups) | pellet store `spawn_pellet`; RNG stream per zone |
| `pellet.rs` | chunked SoA pellet store: `amount, material, pos, vel, rot, ang_vel, held, decay{fresh_remaining, rot}, zone_owner/free`; decay scheduling; merge; rip; meat piles; counters | spatial hash insert/remove; counters callback |
| `pheromone.rs` | spot store: `pos, heading, r/g/b strengths, dissipate, tower_owned, decay_quantum_accum`; emission/dissipation; tower spots | spatial hash layer; sensing query |
| `senses.rs` | FOV query + exact weights/aggregations; phero sums/angles; produces a flat `SenseOutputs` per bibite; throttle state lives with the bibite (body-dev) | body pose/size/health/gene accessor trait; pellet/spot/bibite spatial queries; RNG for phase offsets (or body owns counters) |
| `tools.rs` | tower/killer state, dose/kill schedules, attached-zone snap targets | bibite access (position, genes, `die`, mutation-factor hook) |
| `sim.rs` | run config (`SimConfig {tps, seed, threads}`), system schedule, timers (tick-count based), stats collector, deterministic stat sums | all stores; `World::tick` |

### 12.2 Suggested SoA / storage shapes

* **Zones** — `Arena<Zone>` or `Vec<Zone>` (loaded list order is semantic: save order, processing order, zone IDs). `ZoneSettings` is copied in from `settings.rs` (fmt-dev); keep the derived cache (`area, max_biomass, total_growth, avg_pellet_energy, radius, is_rect/is_ring, inside_radius, rect half extents`) computed once per settings change.
* **Pellets** — chunked SoA, recommended:
  ```
  struct PelletStore {
      amount:      ChunkedVec<f32>,
      material:    ChunkedVec<MaterialId>,
      pos:         ChunkedVec<Vec2>,
      vel:         ChunkedVec<Vec2>,     // 0 for milestone-1 "at rest" starts
      rot:         ChunkedVec<f32>,
      ang_vel:     ChunkedVec<f32>,
      held:        BitSet,
      owner_zone:  ChunkedVec<Option<ZoneId>>,  // None = free holder
      decay:       SparseSet<DecayState>,       // only for material.decay
      free_list:   Vec<PelletId>,
  }
  ```
  with `PelletId = Arena`-style generational handle (`ids.rs`) or slot+generation. Pellets are numerous and short-lived → flat arrays + free list + spatial hash, per ARCHITECTURE §4.
* **Pheromone spots** — small dense SoA (`pos`, `heading`, `strength: [f32;3]`, `dissipate: bool`, `accum: f32`, `owner_tool: Option<ToolId>`) + free list; separate spatial-hash layer (spots are queried in a separate pass with the `PheroSense` radius).
* **Spatial hash** — one uniform grid rebuilt per tick (or incrementally) with separate class masks (bibite bodies/pellets/spots). Cell size ~ interaction radius; for FOV the query is a cone/box, so the grid must support box queries (`cells_in_aabb`) plus circle queries. Keep query results in a reusable `QueryScratch` with an explicit cap (128 to match the game) and a deterministic ordering: sort by slot id before weighting if any summation order affects floats (it does — weight sums are order-dependent). The game's order is collider query order, which is not meaningful; pick stable id order and document divergence.
* **Determinism** — every floating sum in sensing/aggregation must be computed in a fixed order. Prefer per-bibite sequential accumulation + stable sort of query results. Parallel sensing is allowed (each bibite owns its scratch/outputs), but **not** order-dependent reductions.

### 12.3 Interfaces to other modules (must be agreed with body/brain/fmt)

1. **`BibiteView` trait/struct** (from body-dev) for sensing/tools: `pos() -> Vec2`, `up() -> Vec2`, `d1_size() -> f32`, `side_radius() -> f32` (= `4.5*d1Size`), `base_body_area() -> f32`, `max_health() -> f32`, `health_ratio() -> f32`, `dead: bool`, `color_genes() -> [f32;3]` (genes 5,6,7), `eye_offset() -> f32` (gene 25), `gene(g) -> f32`, `is_ready: bool` (gene data loaded), `phase_offsets` for FOV/brain, mutation-factor hooks for radiation `add_mutation_increase(source, count, sigma)`.
2. **Sensing output contract**: `SenseOutputs` with exactly the 12 world-sense values the brain consumes (§3.8) + the phero 9 values; the body/brain module writes them into input nodes at the same points.
3. **Pellet/egg/bibite mutation hooks** (body-dev): pellet `amount` changes must funnel through `World::pellet_change_amount(id, new_amount)` to keep `plant/meat counters`, zone ownership, decay registration, spatial hash and `on_gone` bookkeeping exact. Same for `spawn_meat_pile(energy, pos, radius)`.
4. **Tool effects** (body/brain-dev): `RadDose` must reach the gene mutation accumulator used by reproduction; `ColorKiller` must call the same `die()` path so lineage/death stats fire; `PheroTower` needs the phero store.
5. **Settings snapshot** (fmt-dev): the world needs typed access to `ScenarioSettings` (world-relevant subset) and `ScenarioIndependentSettings`; recommend a `WorldSettings` view rather than reaching into the full settings model from `world.rs`.
6. **Format hooks** (fmt-dev): `World` must expose `to_scene_json()`, `to_pellets_json()`, `to_pheromones_json()`, `from_*` with the exact field shapes in §10; plus `zonemanager_data_bin` opaque pass-through. Do not put serde in core.

### 12.4 Performance notes

* FOV is the top cost at scale: per bibite a cone/box spatial query + O(k) weighting. Optimise by: class-separated grids (so plant queries don't scan bibites), cached `inv_view_radius`, avoiding `sqrt` where possible (`d` needed for weight and dist; one `sqrt` per candidate), and precomputing `1.05` constants. Lookups fire every `10 brain updates` (1.5 s = 45 ticks at 30 TPS, 2 s = 80 ticks at 40 TPS) and sense-value refreshes every 5 brain updates (22.5/40 ticks), so the average per-tick vision load is roughly 1/45–1/80 of the population for the expensive lookup pass.
* Pheromone sensing is per tick for every bibite with phero inputs — at large populations make the phero store a second grid and keep radius small (gene default 150 u); consider caching per-cell aggregates as a future `--fast-math` approximation (documented error) because `Σ strength/d` is a smooth field.
* Pellet spawn/recycle is O(pellets) per over-cap tick only (recycle walks children); keep zone pellet lists as intrusive index lists (per-zone `Vec<PelletId>`) and recycle from the front (implementation-defined order is a divergence; the game uses transform child order, roughly creation order).
* Decay: keep a `Vec<PelletId>` of decaying pellets, swap-remove; process every `decayTPS` ticks with `dt*decayTPS`.
* Stats: accumulate in `f64` for sums/reporting, `f32` state.

### 12.5 Behavioural-fidelity checklist (must match the game)

1. spawn threshold `1.5×avg`, lognormal `1.5^N`, clamp `[0.1,5]`, energy floor `max(avg, 10)`.
2. zone biomass lag: only update on spawn/recycle/recount (10 s), not on eating.
3. FOV weight formula incl. `1.05`, size terms, corpse `(1−healthRatio)` meat channel and dead-bibite dual channel.
4. `tanh(sum/5)`, inverse-distance phero sums, `SignedAngle/180` angles.
5. phero decay `0.25 u²/s` in 0.25 s quanta, negative strengths tolerated.
6. ColorKiller band semantics (§6.4) — verify with capture.
7. Red Death `0.5·(base + 0.5k·velocity)` cadence and safeRadius formula.
8. `decayTPS`/`brainTPS`/`vision*` throttle semantics (tick counts, not seconds).
9. census/cull on autosave/load/sim-size-change only.
10. corpse metabolism→health→explode path.

---

## 13. Open questions / ambiguities (resolve with ground-truth capture)

| # | Question | Current best answer | How to resolve |
|---|---|---|---|
| Q1 | ~~**Default simTPS 30 vs 40.**~~ **CLOSED (2026-10-29):** engine default is 30 (`clock.rs::DEFAULT_SIM_TPS`), always overridden by `recommended`/`independents`; `docs/ARCHITECTURE.md` §6 corrected to 30. | Resolved. | Done. |
| Q2 | **Cross-system FixedUpdate order** is undefined by Unity and not in the source. | **RESOLVED (canonical choice):** §9.2 order implemented and measured as 17 phases; body-internal order from spec 02 §2.3/§2.6. Deviation recorded as D3. | Done; validated by parity captures. |
| Q3 | **Deferred `Object.Destroy`** means destroyed pellets/spots/bibites may be visible to later phases in the same tick. | **RESOLVED as a deliberate deviation:** immediate logical removal at the phase boundary (D6). | Done; no behavioural difference expected at phase scope. |
| Q4 | **`SignedAngle` sign labels.** Formula is authoritative: positive = CCW; with `up`=forward, a target on the bibite's **right** is negative. Community docs claim `+`=right. | Implement formula; do not rely on left/right labels. | Capture a bibite with a plant to its right; check `PlantAngle`. |
| Q5 | ~~**ColorKiller polarity.**~~ **CLOSED by parity capture (task-18):** kills when hue distance **≥ tolerance** (keep-the-band), for all four `bodyOrEye`/`anyOrMost` combos. Compiled branch was correct. | Implemented compiled/captured polarity in `tools.rs`; `hue_matches_alternative` kept only as documented negative. | Done — `docs/PARITY.md` Appendix A + `testdata/parity/experiments/color_killer_polarity`. |
| Q6 | **`InitialSeeding` infinite loop** when red death is on and spawn positions fall outside `safeRadius` (SpawnAPellet returns energy without creating a pellet, biomass never grows). | **GUARDED:** `zone.rs` caps seeding at 100 000 iterations per zone and per-tick spawns at 10 000; the behaviour is mirrored (pellet not created, biomass not added) but terminates. | Done (code guard); the game's own behaviour on a hang is untested. |
| Q7 | **Zone speed tooltip** says "1 % of sim size per hour", formula gives `S·speed/3600` = 0.556 u/s at defaults (≈100 %/h). | Trust formula. | Capture a Free zone position over 60 s. |
| Q8 | **Pellet radius mismatch** (collider 2.5·√(A/20) vs analytic √(A/π)). | Use analytic everywhere; document physics divergence. | Compare bite/contact distances in capture. |
| Q9 | **`PelletIsHeld` bug** can fail to exclude held pellets from FOV. | Implement intended "exclude held"; divergence note. | Capture held-pellet bibite FOV `NPlants`. |
| Q10 | ~~**Pheromone decay**~~ **CLOSED by parity capture (task-18):** −0.25 u²/s in 0.25 s quanta (−0.0625/quantum), spot destroyed when all strengths ≤ 0; the "1/s, ~10 s lifetime" doc claims are wrong. | Code and capture agree: 0.25/s. | Done — `docs/PARITY.md` Appendix A3. |
| Q11 | **Pheromone heading lost on save/load** (`heading:{}`). | Reproduce on-disk shape; decide whether to also reproduce the loss (recommend yes for parity, no for "fixed" fidelity). **Parity capture (task-18) confirms the loss and strength round-trip** (`docs/PARITY.md` A3). | Done: capture-confirmed; format layer must emit `heading: {}`. |
| Q12 | **Radiation dose not persisted** in genes; autosave-reload wipes it. | Reproduce (dose is transient). | Capture save/load with a bibite in a rad tower. |
| Q13 | **ColorKiller stale 128-collider buffer** loop can hit stale colliders. | Reproduce? recommend a correct bounded loop + divergence note. | Capture kills with >128 bibites in range then move them out. |
| Q14 | **`data.bin` exact LogLike compression** for zone/species histories. | **M1 IMPLEMENTED AS OPAQUE:** bytes are preserved on load and re-emitted unchanged (D11); history streams are not parsed/regenerated. Full codec remains future work for fmt-dev+world-dev. | Load/save round-trip test against the game (already covered by the save round-trip acceptance). |
| Q15 | **`Zone.pelletBiomass` lag** — reproduce or fix? | Reproduce for parity (it changes spawn/overshoot behaviour). | Capture zone pellet count vs `pelletBiomass` after heavy grazing. |
| Q16 | **`Pherosense.pheroDist` not size-scaled**. | Reproduce. | Capture. |
| Q17 | **Free-pellet zones**: pellets spawned by deaths/producers are never owned by a zone → not recycled by zones. | Reproduce. | Capture. |
| Q18 | **`Rng` mapping**: replacing two game RNG families changes trajectories vs the game; only formulas can be exactly compared. | Accept behavioural parity; derive per-zone/per-entity streams. | Parity plan per ARCHITECTURE §9. |
| Q19 | **Frame-rate coupling of `WaitForSeconds` coroutines** (spawner 1 s, recount 10 s, autosave). | **IMPLEMENTED:** all are tick accumulators over simulated time (zone recount 10 s, spawner 1 s, Red Death/rad 0.5 s); autosave-time census is an explicit `census()` call. D3. | Done. |
| Q20 | **ColorKiller defaults** (`hue/tolerance/anyOrMost/bodyOrEye/radius/period`) are prefab values not in code. | Defaults unknown; UI clamps `radius` to 100..`SimulationSize`; captured instance had hue 0, tolerance 0.075, bodyOrEye false, anyOrMost true, radius 3033, period 143.6. | Inspect the prefab in the capture build, or read a placed ColorKiller from a real save. |

---

## 14. Discrepancy log vs supplemental documents

| Claim in a secondary doc | Source truth | Action |
|---|---|---|
| "Basic TPS 40" (`bibites_brain_reference`) / old ARCHITECTURE "default 40" | `ScenarioIndependentSettings` default 30; 40 comes from newer scenario `recommended` blocks. `docs/ARCHITECTURE.md` §6 has since been corrected to 30 | Q1 (closed) |
| "fade 1 u²/s" pheromones (reference + GAME_GUIDE + settings tooltip) | `0.25·n` per `n` s = 0.25/s | Q10 |
| Color killer kills matching hue (GAME_GUIDE, reference) | Compiled branch kills `hue_distance ≥ tolerance` | Q5 |
| "wounded bibites ... weighted into meat channel" (reference) | Only `dead` bibites enter the corpse/meat channel; the `(1−healthRatio)` term weights *corpse decay* (fresh corpses have `healthRatio = 1` after death, so weight starts at ~0 and grows) | §3.5, §2.5 |
| "Vision ... Closeness = 1 − min(weightedDist/viewRadius,1)" | True in spirit, but the vector is the **weighted mean of distance-scaled direction vectors**, not a mean distance | §3.5 |
| "Pellets fade ... plant 5 E/u², meat 10" (GAME_GUIDE) | Matches captured material defaults | ok |
| "Red Death: hurts 0.5 HP/s + velocity·t" | Matches `acc` accumulation within a factor (`Hurting(acc·0.5)` each 0.5 s ⇒ DPS ≈ acc) | §7.2 |
| "screen label: + = right" angles | Unity `SignedAngle(up, dir)` gives negative for right | Q4 |
| "Dead bibite burst to meat; corpses decay" | Correct, plus exact total-energy accounting and single-pellet meat pile | §2.5 |
| "Basic TPS 40 ... brain ~20×/s" | At TPS 30 / brainTPS 2 = 15 Hz; at 40 = 20 Hz | §3.7 |
| World spec §9.2's old one-line phase-5 order (`clock→growth→brain→organs→propulsion→UseEnergy`) | **Superseded** by body spec §2.3/§2.6 + `docs/INTERFACES.md` §6.1: metabolism+upkeep first, brain, then organs in order armour→stomach→egg→fat→move→mouth→pheromone→propulsion→`UseEnergy`. §9.2 was corrected to match. | body spec §2.3/§2.6; INTERFACES §6.1 |

---

## Appendix A — Worked example: official `Default.zip` scenario `[C]`

Captured `Scenarios/Default.zip/settings.bb8settings`: one zone `World` (Plant, radius relative 1.0, fertility 0.7827918, pelletSize 1, speed 0), one bibite row (Basic bibite, priority 1, minimum omitted), `recommended = {SimulationSize: 2000, biomassDensity: 0.01, pelletGrowth: 1e-5}`.

| Quantity | Computation | Value |
|---|---|---|
| S | — | 2000 u |
| zone area | π·2000² | 12 566 370.6 u² |
| `maxBiomass` | 1·area·0.01 | ≈ 125 664 E |
| `totalGrowth` | 0.7827918·area·1e-5 | ≈ 98.37 E/s |
| `avgPelletEnergy` | 200·1 | 200 E |
| spawn threshold | 1.5·200 | 300 E → one pellet ≈ every 3.05 s at full rate |
| steady pellet count | 125664/200 (mean size ~217 → ~579; nominal 628) | ≈ 580–630 |
| `estimatedPellets` (UI) | ceil(125664/200) | 629 |
| initial seeding | 0.75·125664 = 94 248 E | ≈ 434–471 pellets |
| initial `pelletProgress` | untouched by seeding | 0 |

`ZoneManager.freeBiomass` after seeding ≈ 125664 − 94248 ≈ 31 416 E (minus overshoot). Expected `PlantCounter.Biomass` ≈ `initialSeeding·maxBiomass` immediately after seeding (within one pellet), while `Σ zone.pelletBiomass` equals it exactly (both maintained by spawning) — divergence starts only after grazing/decay.

## Appendix B — Key formulas quick-reference

```
zone.max_biomass       = biomassDensity · area · globalBiomassDensity
zone.total_growth      = fertility · area · globalPelletGrowth
zone.avg_pellet_energy = pelletEnergy · pelletSize
spawn when             pelletProgress > 1.5 · avg_pellet_energy
pellet energy          = max(avg, pelletEnergy.minValue) · clamp(1.5^N(0,1), 0.1, 5)
pellet amount          = energy / material.EnergyDensity
pellet sizeFactor      = amount / 20 ;  scale = sqrt(sizeFactor) ;  radius = sqrt(amount/π)
pellet mass            = |amount · massDensity| ;  linDrag = drag · scale / mass
decay:                 rot += (fresh_remaining > 0 ? 0 : dt · decayRate); chop rot when rot/amount > 5%
FOV weight (pellet)    = (1.05 − d/R)² · sizeFactor · (1.05 − |θ|/(A/2))
FOV weight (bibite)    = (maxHealth/100) · (1.05 − d/R)² · (1.05 − |θ|/(A/2))
FOV weight (corpse)    = (baseBodyArea/20) · (1 − healthRatio) · (1.05 − d/R)² · (1.05 − |θ|/(A/2))
FOV closeness          = 1 − min(|Σ dir·w / Σw| / R, 1)
FOV angle              = 2 · SignedAngle(up, Σ dir·w / Σw) / A
phero sum              = Σ strength / distance   (per channel, over spots in radius)
phero sense            = tanh(sum / 5)
phero angle            = SignedAngle(up, Σ dir·(strength/d)) / 180
phero decay            = 0.25 / s   (0.25 s quanta; may go negative)
phero emission         = brainOutput · pheromoneProductionStrength (default 10), 1/s, cost 0.01·Σ
rad dose (0.5 s)       = count·0.5·f, intensity·0.5·f,  f = 1 if taper≈0 else 1 − (d/r)^(1/taper)
red death safe radius  = shadeEnd·sqrt(1 − fill)   [since safeRadius = shadeEnd·(1 − (1−sqrt(1−fill)))]
red death dmg (0.5 s)  = 0.5·(base + 0.5·k·velocity) after re-entry reset to base
color killer:          d_hue = circular hue distance ; body: d ← 1 − (1−d)·s·v ; kill when d ≥ tolerance
zone speed             = S · globalZoneSpeed · zoneSpeed / 3600  [u/s]
```

## Appendix C — Source & capture index

**Primary C# (world-relevant):**
`Here/SimulationScripts/`: `Zone.cs` (670 L), `ZoneManager.cs` (337), `Matter.cs`, `MatterMaterial.cs`, `MatterMaterialManager.cs`, `MatterDecayProcessor.cs`, `MatterPellet.cs`, `PelletPlacer.cs`, `PheromoneSpot.cs`, `PheroTower.cs`, `RadTower.cs`, `EffectTower.cs`, `ColorKiller.cs`, `BibiteSpawner.cs`, `BibiteSpawnInfo.cs`, `PlantCounter.cs`, `MeatCounter.cs`, `IEntityCounter.cs`, `ChallengeManager.cs`, `TagInfo.cs`, `SettingsChangerManager.cs`.
`Here/SimulationScripts.BibiteScripts/`: `FieldOfView.cs` (690), `Pherosense.cs` (227), `BibitePheromoneOrgan.cs`, `GameObjectWeight.cs`, `TargetType.cs`, `NEATBrain.cs` (senses/inputs slice), `BibiteBody.cs` (loop/corpse/totalEnergy slice), `EggHatching.cs`, `Species.cs`, `MutationIncreases.cs`.
`Here/SimulationScripts.Events/`: `RedDeathBloomManager.cs`, `OnRedDeathEnterExitTrigger.cs`.
`Here/OneUseScripts/`: `RadioArea.cs`, `RadioZone.cs`, `TimeKeeper.cs`, `BackgroundManager.cs`, `PelletProducer.cs`.
`Here/SettingScripts/`: `ZoneSettings.cs` (815), `ZoneGroupSettings.cs`, `ScenarioIndependentSettings.cs`, `ScenarioSettings.cs` (1364), `MatterMaterialSettings.cs`, `MatterMaterialSetting.cs`, `TargetZoneSetting.cs`, `SpawnDistribution.cs`, `MovementType.cs`, `SpawnType.cs`, `SpawnPosition.cs`, `BibiteSettings.cs`, `ScenarioSaver.cs`, `SettingsUpdater.cs`, `BibiteEditorSettings.cs` (gene defaults).
`Here/ManagementScripts/`: `WorldObjectsSpawner.cs`, `ZoneManager` (above), `SaveSystem.cs` (1101), `SaveController.cs`, `SimulationManager.cs`, `TimeController.cs`, `BibiteTracker.cs`, `GlobalLineageManager.cs`, `TagsManager.cs`.
`Here/Structs/`: `MatterDecayOfArrays.cs`. `Here/Utility/`: `DataLogger.cs`, `LogLikePresenceArray.cs`, `LogLikeArray.cs`, `LogLikeArraySerial.cs`, `SpeciesDataPoint.cs`, `Version.cs`. `Here/Utility.DataLogging/`: `ZoneDataPoint.cs`, `LogLikeZoneDataArray.cs`, `LineageVersionUpdater.cs`. `Here/ScriptHelpers/`: `RandomExtension.cs`, `VectorExtend.cs`, `SerializationHelper.cs`, `SaveableBinStack.cs`, `SerializedTransform.cs`, `SerializedTransformExtensions.cs`, `VersionTracker.cs`.

**Captured real data (read-only reference paths):**
* `C:\Users\Storme\AppData\LocalLow\The Bibites\The Bibites\Scenarios\Default.zip` — official default scenario (zones, recommended).
* `...\Scenarios\RadTower.zip` — 0.6.3.1 scenario with full material block, 30 zones, recommended TPS 40.
* `...\Savefiles\RadTower_t0.zip` — live save: `scene` (3 rad towers, simulatedTime), `pellets` (free + zone buckets, matterDecay), `pheromones` (heading bug, negative strengths), `settings`, `data.bin`.
* `...\Savefiles\ReadLLM2.zip` — save with a ColorKiller + PheroTower + RadTower; source of `colorSelectors`/`pheroTowers` field examples.
* `...\SimulationSettings\*.bb8settings` (user settings files) and `...\Savefiles\*.zip` (population/history shapes).

**Secondary docs used only for cross-check** (see §14): `Z:\RUSTibites\bibites_brain_reference (4).md`; `C:\Users\Storme\Desktop\Projects\BibitesInfo\Bibites_Code_Breakdown.md`; `C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI\docs\GAME_GUIDE.md`.

## Appendix D — Rust/open questions to raise with the Lead *before implementation*

1. ~~`DEFAULT_SIM_TPS` in `clock.rs` should be 30 (game default) or be treated as a fallback; engine should construct the clock from settings (`recommended`/`independents`).~~ **RESOLVED:** `clock.rs::DEFAULT_SIM_TPS = 30`, and every run constructs the clock from the effective settings. [no foundation change left]
2. Does the Lead want strict reproduction of the known quirks (Q3, Q5, Q9, Q13, Q15, Q16) or "intended" behaviour with a documented divergence entry? Recommendation: reproduce quirks that affect long-run ecology (Q5, Q15, Q16, Q6) and fix pure bugs/UB (Q3, Q9, Q13) behind a `--strict-parity` option if feasible.
3. `data.bin` write path: preserve opaque vs regenerate — needs a joint decision with fmt-dev.
4. `World` needs a small `BibiteView`-like accessor and a `pellet_energy_delta` choke point from body-dev; brain-dev must consume the 21 world-sense inputs from `senses.rs`. Coordinate trait/API in the Lead's foundation review.


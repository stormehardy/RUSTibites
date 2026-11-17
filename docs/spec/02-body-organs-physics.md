# 02 — Bibite body, organs, physics, feeding & combat

**Owner:** body-dev · **Status:** spec-extraction complete (Phase 0); implemented and kept current (2026-10-30) · **Target:** milestone 1 headless core
**Authority:** the decompiled C# in `Here/` is the source of truth. Every formula below cites `Here/<file>:<line>`.
Real save data (installed game `0.6.3.1`) was used to verify serialized fields and material defaults; those
observations are cited with the file path they came from.

Legend for claim confidence:

| Tag | Meaning |
|---|---|
| **[C]** | Verified directly in `Here/` C# source (line cited). |
| **[C+S]** | Verified in C# **and** cross-checked against a real `0.6.3.1` save/settings file. |
| **[S]** | Observed only in real save data (used where C# has no default, e.g. ScriptableObject constants). |
| **[INF]** | Inferred (state it; must be validated by a ground-truth capture before it becomes contract). |
| **[PREFAB]** | Value lives in Unity prefab/scene/ProjectSettings, not in the decompiled scripts — **not verifiable from `Here/`**; see §11 open questions. |

Units used throughout: `u` length, `u²` area, `E` energy, `HP` health, `s` seconds, `tick` = one fixed
simulation step. Positions/velocities are 2-D (`Vec2`), angles in degrees unless noted (Unity convention:
`transform.up` is forward, `transform.right` is +x, positive rotation = counter-clockwise, `Vector2.Angle`
returns unsigned degrees, `Vector2.SignedAngle` signed degrees).

## Implementation status (2026-10-30)

**Status: implemented.** Code: `crates/bibites-core/src/{body,organs,physics}.rs`; persistence in
`crates/bibites-io/src/{model,bundle,world}.rs`. The custom solver intentionally replaces Box2D (D1/D2) and
destruction is immediate (D6); every other formula below is the contract the code is tested against.

| Section | Status | Code | Representative tests |
|---|---|---|---|
| §1 entity model / body scalars / geometry | implemented | `body.rs` `BodyGeometry`, `BodyState`, `BibiteRecord::recompute_derived`, collider cache | `derived_cache_matches_formulas`, `spawn_vitals_match_starter_spec_numbers`, `spawn_size_scales_with_size_ratio`, `collider_descriptors_match_captured_prefab` |
| §2 tick pipeline / order | implemented (canonical order below; Unity's cross-component order is a documented deviation D3) | `body.rs::tick_brain_pre`/`tick_brain_post`/`tick_organs`; driven by the world pipeline | `tick_spends_energy_and_advances_clock`, `brain_cadence_and_senses_are_wired`, `full_tick_multi_system_smoke` (world) |
| §3 size / growth / maturity / ageing | implemented | `body.rs` size, growth ODE, ageing helpers | `derived_cache_matches_formulas`, `growth_at_spawn_mapping`, `spawn_vitals_match_starter_spec_numbers` |
| §4 energy & health accounting | implemented (Energy density 1.0, Health 0.25 captured — BQ-1 resolved) | `body.rs` `MatterPool`, `use_energy`, `gain_energy`, `hurt`, `heal`, metabolism | `tick_spends_energy_and_advances_clock`, `corpse_decays_to_explosion`; parity: energy bit-exact over 300 ticks |
| §5 organs | implemented | `organs.rs` stomach/mouth/propulsion/fat/armour/egg-layer/pheromone/clock machines; `body.rs` organ loop | `swallow_pellet_moves_matter_to_stomach`, `egg_store_hatches_after_hatch_time_in_dense_order`, organs-module tests |
| §6 feeding & combat | implemented | `organs.rs` 2-iteration Newton bite, grab/throw/vomit; `body.rs::resolve_bite`, `collision_damage` | `whole_bibite_swallow_kills_prey_and_transfers_matter`, `bibite_grab_adds_held_target`, `collision_damage_formula_and_signs`, `overlapping_bibites_take_two_sided_collision_damage` |
| §7 reproduction / eggs | implemented | `body.rs` `EggStore` + egg-layer organ | `egg_laying_sets_parents_and_generation`, `egg_store_hatches_after_hatch_time_in_dense_order`, `egg_with_insufficient_energy_aborts_and_queues_energy`, `egg_save_round_trip` |
| §8 death / corpses / scavenging | implemented | `body.rs` death queue, corpse update, meat conversion | `death_without_corpses_spawns_meat_and_event`, `kill_queues_and_resolver_creates_corpse`, `corpse_decays_to_explosion` |
| §9 persistence | implemented | `bibites-io` `BibiteSave`/`EggSave` + `body.rs::to_save`/`spawn_loaded` | `save_rotation_is_game_degrees_and_record_is_radians`, io `bundle_i3`, `world_i4a` |
| §10 Rust design notes | implemented with deviations D1/D2 (custom solver) and D6 (immediate removal) | `physics.rs` broadphase/narrowphase/solver; body/organ SoA passes | `physics_step_is_deterministic_for_same_state`, `parallel_and_serial_brain_batches_are_bit_identical` |
| §11 open questions | BQ-1/BQ-3/BQ-7 resolved; BQ-2/BQ-4 partially captured; BQ-16 confirmed still open | see the BQ rows | — |
| §12 cheat sheet | verified against implementation | constants live in `settings.rs` (settings defaults) and `body.rs`/`organs.rs`/`physics.rs` (captured prefab/geometry) | `settings.rs::defaults_match_spec_tables`, `organs.rs` derived-formula tests |

### Implementation notes (deviations and known gaps)

* **BQ-1 resolved:** the `Energy`/`Health` material densities are captured (Energy `1.0`, Health `0.25`) and are
  hardcoded as captured constants in `body.rs` (`BodySettings::{energy,health}_material_density`); they also
  explain corpse lifetime and starvation damage.
* **BQ-2 partially resolved:** collider geometry, friction/restitution pairs, damping, layer matrix and Box2D
  solver constants are captured in `physics.rs` (`BODY_CAPSULE_*`, `TAIL_CAPSULE_*`, `MOUTH_BOX_*`,
  `CORPS/PLANT/MEAT_FRICTION|RESTITUTION`, `LINEAR_DAMPING`, `*_ANGULAR_DAMPING`, `LAYER_*`). The modelled body is
  a root capsule + tail capsule + mouth box (3 colliders, `MAX_COLLIDERS = 4`); prefab-only details
  (gravityScale/interpolation/constraints, exact child-part list) remain assumptions.
* **BQ-3 resolved:** the canonical order in §2.6 is implemented (brain/senses → organs in the fixed game order →
  growth/clock → heal/death → physics); it is a deliberate deviation from Unity's undefined cross-component order (D3).
* **BQ-4 parameterised:** `body::CollisionSign` selects the normal-sign convention and defaults to `AsIs`. The
  parity capture confirmed approach-positive closing speed damages **both** partners (`docs/DEVIATIONS.md` Q11),
  which is consistent with the default; the body-mass weighting term of the formula is still not capture-verified.
* **BQ-7 resolved:** the engine default is 30 TPS (`clock.rs::DEFAULT_SIM_TPS = 30`), always overridden by
  `independents`/`recommended`; `docs/ARCHITECTURE.md` §6 now also says 30.
* **BQ-16 still open:** `neuronBirthCost`/`synapseBirthCost` exist in the settings model but are not charged
  anywhere in body/brain code (matching the decompiled source) — kept for round-trip fidelity only.

---

## Table of contents

0. Source map and verification method
1. Entity model, body scalars and derived quantities
2. Tick pipeline, update order, timing
3. Body size, growth, maturity, ageing
4. Energy and health accounting
5. Organs
6. Feeding and combat resolution (bite geometry, grab, collision damage)
7. Reproduction: egg production, laying, hatching, gene flow
8. Death, corpses, scavenging, revival
9. Persistence: `SaveState`/`LoadState`, `.bb8` envelope, version migrations
10. Rust design notes (SoA, organ modules, custom solver, determinism, perf)
11. Open questions / ambiguities / values needing capture
12. Formula and constants cheat sheet
13. Source index

---

## 0. Source map and verification method

### 0.1 Primary C# files

| File | Role |
|---|---|
| `Here/SimulationScripts.BibiteScripts/BibiteBody.cs` | body hub: energy/health matter, size, mass, drag, metabolism, damage, death/corpse, save/load |
| `Here/SimulationScripts.BibiteScripts/BibiteOrgan.cs` | organ base class (init, per-tick `UpdateOrgan`, death/destroy hooks) |
| `Here/SimulationScripts.BibiteScripts/BibiteSpatialOrgan.cs` | organ with an apportioned area (`WAG`) + growth hook |
| `Here/SimulationScripts.BibiteScripts/BibiteMouth.cs` | mouth trigger, bite/swallow/grab/throw/vomit, bite-chunk geometry |
| `Here/SimulationScripts.BibiteScripts/BibiteStomach.cs` | stomach contents, digestion, affinity/efficiency, spill |
| `Here/SimulationScripts.BibiteScripts/StomachContent.cs` | per-material content struct |
| `Here/SimulationScripts.BibiteScripts/BibitePropulsion.cs` | movement forces/torque/costs, herding/void/shade/wrap |
| `Here/SimulationScripts.BibiteScripts/BibiteFatOrgan.cs` | fat storage/burn deadband, body inflation |
| `Here/SimulationScripts.BibiteScripts/BibiteArmor.cs` | armor thickness/strength |
| `Here/SimulationScripts.BibiteScripts/BibiteEggLayingOrgan.cs` | egg production, laying, children list |
| `Here/SimulationScripts.BibiteScripts/EggHatching.cs` | egg energy budget, incubation, hatching/abort |
| `Here/SimulationScripts.BibiteScripts/BibiteGrowth.cs` | growth ODE, maturity, growth energy cost |
| `Here/SimulationScripts.BibiteScripts/InternalClock.cs` | tic/chrono/timeAlive, ageing event |
| `Here/SimulationScripts.BibiteScripts/BibitePheromoneOrgan.cs` | pheromone emission |
| `Here/SimulationScripts.BibiteScripts/Pherosense.cs` | pheromone sensing (adjacent, spec 03 owns senses) |
| `Here/SimulationScripts.BibiteScripts/BibitePart.cs` | child body-part colliders, collision damage |
| `Here/SimulationScripts.BibiteScripts/BibiteID.cs` | random non-zero ids |
| `Here/SimulationScripts.BibiteScripts/NEATBrain.cs` | brain upkeep, output access `Output(Outputs)`, `UpdateSenses` |
| `Here/SimulationScripts.BibiteScripts/FieldOfView.cs` | herd outputs consumed by propulsion, `sideRadius` vision extent |
| `Here/SimulationScripts.BibiteScripts/BibiteGenes.cs` | gene enum + derived body/genome formulas, mutation/copy |
| `Here/SettingScripts/BibiteEditorSettings.cs` | per-gene defaults/ranges (the editor table) |
| `Here/SettingScripts/ScenarioSettings.cs` | scenario constants (defaults + ranges) |
| `Here/SettingScripts/ScenarioIndependentSettings.cs` | TPS, simulation size, birth limit |
| `Here/SettingScripts/UserSettings.cs` | `simTPSDefault = 40` for new scenarios |
| `Here/SimulationScripts/Matter.cs` | matter container math (energy↔amount, radius, mass) |
| `Here/SimulationScripts/MatterMaterial.cs` | material conversion-efficiency formula |
| `Here/SimulationScripts/MatterMaterialSettings.cs` | material settings wrapper (save schema) |
| `Here/SimulationScripts/MatterMaterialManager.cs` | material singletons (Plant/Meat/Fat/Armor/Energy/Health/None) |
| `Here/SimulationScripts/MatterPellet.cs` | pellet physics, amount/energy, `RipChunkOff`, merge/decay hookup |
| `Here/SimulationScripts/MatterDecayProcessor.cs`, `Here/Structs/MatterDecayOfArrays.cs` | meat decay job |
| `Here/ManagementScripts/WorldObjectsSpawner.cs` | prefab spawning: bibites, eggs, pellets, meat piles |
| `Here/ManagementScripts/SaveSystem.cs` | per-entity save envelope and load order |
| `Here/ManagementScripts/BibiteTracker.cs` | birth/death bookkeeping |
| `Here/ScriptHelpers/SerializationHelper.cs` | reflection serializer semantics (which fields persist) |
| `Here/ScriptHelpers/BibiteUpdater.cs` | gene remap migrations pre-`0.6a15` |
| `Here/ManagementScripts/TimeController.cs` | `Time.fixedDeltaTime = 1/simTPS` |
| `Here/SimulationScripts.BibiteScripts/BibiteProceduralSpriter.cs`, `Here/ManagementScripts/ProceduralSpriteManager.cs` | visual-only sprite scaling |

### 0.2 Save data used

| File | Use |
|---|---|
| `C:\Users\Storme\AppData\LocalLow\The Bibites\The Bibites\Savefiles\beeg\saves\_00.zip` (`settings.bb8settings`, `scene.bb8scene` version `0.6.3.1`) | canonical `0.6.3.1` material defaults + scenario defaults |
| `...\Savefiles\Autosaves\autosave_20261008004410.zip` (`bibites\bibite_0.bb8`, `eggs\egg_0.bb8`) | canonical current `.bb8` per-entity save fields (matches `SaveState()` output exactly) |
| `...\Avarus Nyx.bb8` (`0.5.0a3`) | historical `.bb8` schema (old `genes["Strength"]/["Defence"]`, `body.born/totalEnergy/wasteBank`) |
| `...\Scenarios\Default.zip` | default scenario JSON (zones/bibites/`recommended` only — **no** materials; material defaults come from built-in assets) |

### 0.3 Documentation cross-check

* `bibites_brain_reference (4).md` (repo root) — helpful for bite geometry/physics, **but has errors** (e.g. it
  quotes `Armor.cohesiveness = 500` *which matches built-in defaults*, while a modified test scenario saved
  `75`; it also omits several body fields). Where doc and code disagree, code wins.
* `C:\Users\Storme\Desktop\Projects\BibitesInfo\Bibites_Code_Breakdown.md` and `...\BibitesAPI\docs\GAME_GUIDE.md`
  — indexes only; every number below was re-derived from `Here/` or real saves.
* **Known doc-confirmed discrepancy:** `GAME_GUIDE.md` says movement cost scales with `SpeedRatio`; the code
  charges `moveMusclesCost · area · |activation|` and then divides by `MetabolicEfficiency`
  (`BibitePropulsion.cs:315-318`, `BibiteBody.cs:658-669`). Code is authoritative.

---

## 1. Entity model, body scalars and derived quantities

### 1.1 Unity composition (what one live bibite is)

From `BibiteBody.cs:19-60,394-403,505` and `BibiteBody.FinalizeBirth`:

```
GameObject "bibite" (tag "bibite", layer [PREFAB])
├─ Rigidbody2D                  rb2d        (mass/inertia/damping overwritten every tick)
├─ CapsuleCollider2D            bodyCollider(geometry set by UpdateInflateFactor)
├─ BibiteID                     id
├─ BibiteGenes                  gene
├─ NEATBrain                    brain
├─ BibiteGrowth                 growth
├─ InternalClock                clock
├─ FieldOfView                  fow
├─ Pherosense                   pherosenseOrgan
├─ BibiteProceduralSpriter      bibiteProceduralSpriter (visual)
├─ BibiteMouth                  mouth      (BibiteSpatialOrgan; owns a BoxCollider2D trigger, see §1.7)
├─ BibiteStomach                stomach
├─ BibiteEggLayingOrgan         eggLayer
├─ BibiteFatOrgan               fat
├─ BibiteArmor                  armor
├─ BibitePropulsion             move
├─ BibitePheromoneOrgan         pheromoneOrgan
└─ bodyParts: List<BibitePart>  child GameObjects (arms/etc., each with Collider2D, tag "bibitePart") [PREFAB]
```

Egg (`EggHatching.cs:81-106`): one `GameObject` with `BibiteGenes`, `NEATBrain`, `BibiteID`, `EggHatching`,
`Rigidbody2D`, one `Collider2D`, `SpriteRenderer`; tag `"egg"`.

Pellet (`MatterPellet.cs`): one `GameObject` with `MatterPellet`, `Rigidbody2D`, `Collider2D`,
`GrabbableObject` (pellets), tag `"pellet"`.

`BibiteBody.Const NormalBodyPointAtSize1 = 100f` (`BibiteBody.cs:69`) is declared but not referenced anywhere
else in `Here/` — treat as legacy/no-op.

### 1.2 Stored body fields (`BibiteBody`)

"Persisted" = serialized by `BibiteBody.SaveState()` (`BibiteBody.cs:951-975`): explicit JSON keys plus every
field carrying `[SerializeField]` emitted by reflection. "Derived" = recomputed on load by `ResumeBody`/
`Set2DSize`.

| C# field | Type | Persisted JSON key | Kind | Notes / formula |
|---|---|---|---|---|
| `Energy` | `Matter` | `energy` (float, amount only) | state | `Material = Energy`; `MaxAmount` = `80·d2Size·storableEnergyPerArea` (set in `Set2DSize`) |
| `Health` | `Matter` | `health` (float, amount only) | state | `Material = Health`; `MaxAmount` set in `Set2DSize` |
| `d2Size` | float | `d2Size` | state | area sizing state; `d2Size = growth·SizeRatio²` |
| `dying` | bool | `dying` | state | set true at `Die()`; only meaningful for `Start()` load path (`BibiteBody.cs:405-411`) |
| `dead` | bool | `dead` | state | corpse flag |
| `attackedDmg` | float | `attackedDmg` | state | pain accumulator, decays per tick (§4.5) |
| `timesAttacked` | int | `timesAttacked` | stat | incremented on first bite only (`drain == false`) |
| `totalDamageSuffered` | float | `totalDamageSuffered` | stat | all bite damage ever taken |
| `brainTicksCount` | float | `brainTicksCount` | state | brain-update phase |
| `visionLookupCount` | float | `visionLookupCount` | state | FOV lookup phase |
| `visionSensingCount` | float | `visionSensingCount` | state | FOV sense phase |
| `corpseEnergyOffset` | float | `corpseEnergyOffset` | state | energy subtracted from a corpse's meat value |
| `baseEnergyDensity` | float | — (plain public, no attribute) | derived | = `gene.BodyEnergyDensity`, set at birth/resume |
| `metabolism` | float | — (public, no attribute) | derived | recomputed by `UpdateMetabolism` |
| `healingRateEnergyCost` | float | — | derived | last tick's heal E/s (per-second accounting) |
| `inflateFactor` | float | — `[NonSerialized]` | derived | fat inflation, 0…1 |
| `corpseDecay` | float | — `[NonSerialized]` | derived | visual decay 0…1 |
| `destroyed` | bool | — `[NonSerialized]` | runtime | |
| `born` | bool | — `[NonSerialized]` | runtime | gates `FixedUpdate`; set at `FinalizeBirth` |
| `d1Size` | float | — (public, no attribute) | derived | `√d2Size` |
| `baseMass` | float | — (public, no attribute) | derived | `bibiteMassDensity·baseBodyArea` |
| `metabolicRate` | float | — private | derived | = `SpeedRatio` gene |
| `metabolicEfficiency` | float | — private | derived | `energyUsageEfficiency^SpeedRatio` |
| `naturalBirth` | bool | — private | runtime | |
| `localTag` | string | — private | runtime | species tag cache |

Explicit JSON keys written by `SaveState` in order (`BibiteBody.cs:955-973`):
`mouth`, `stomach`, `phero`, `health`, `energy`, `eggLayer`, `control`, `fatReservesAmount`, `id`, then the
`[SerializeField]` reflection fields (`d2Size`, `dying`, `dead`, `attackedDmg`, `timesAttacked`,
`totalDamageSuffered`, `brainTicksCount`, `visionLookupCount`, `visionSensingCount`, `corpseEnergyOffset`).
A real `0.6.3.1` file confirms exactly this set (`...\Autosaves\autosave_20261008004410.zip\bibites\bibite_0.bb8`):
`{"mouth":{...},"stomach":{...},"phero":{...},"health":...,"energy":...,"eggLayer":{...},"control":{...},
"fatReservesAmount":...,"id":...,"d2Size":...,"dying":false,"dead":false,"attackedDmg":0,"timesAttacked":13,
"totalDamageSuffered":631.07,"brainTicksCount":0.0,"visionLookupCount":3.0,"visionSensingCount":1.0,
"corpseEnergyOffset":0.0}`. (Older `0.5.0a3` `.bb8` files additionally had `body.born`, `body.totalEnergy`,
`body.wasteBank`; modern `LoadState` silently ignores unknown keys.)

### 1.3 Derived body quantities (all [C])

| Property | Formula | Source |
|---|---|---|
| `d1Size` | `√d2Size` | `BibiteBody.cs:705` |
| `baseBodyArea` | `80·d2Size` u² | `BibiteBody.cs:298` |
| `realBodyArea` | `baseBodyArea·(1+inflateFactor)` u² | `:300` |
| `bodyLength` | `12.5·d1Size` u | `:296` |
| `effectiveRadius` | `√(realBodyArea/π)` u | `:250` |
| `armorThickness` | `armor.armorThickness` (see §5.6) | `:248` |
| `internalsRadius` | `effectiveRadius − armorThickness` u | `:252` |
| `sideRadius` | `4.5·d1Size` u (FOV bounding radius for other bibites) | `:254` |
| `baseMass` | `bibiteMassDensity·baseBodyArea` | `:724` |
| `mass` | `baseMass + stomach.mass + eggLayer.mass + fat.mass + armor.mass` (mouth/move mass ≡ 0) | `:230`, `BibiteMouth.cs:163`, `BibitePropulsion.cs:88` |
| `maxHealth` | `healthPerArea·baseBodyArea` HP | `:719` |
| `Energy.MaxAmount` | `baseBodyArea·storableEnergyPerArea` E | `:720` |
| `healthRatio` | `Health.Amount / Health.MaxAmount` | `:292` |
| `energyRatio` | `Energy.Amount / Energy.MaxAmount` | `:294` |
| `energyDensity` | `totalEnergy / realBodyArea` | `:302` |
| `totalEnergy` | `gene.SumBodyAndHealthEnergyRatio·baseBodyArea + Energy.Amount + eggLayer.storedEnergy + fat.fatReserves.energy − (dead ? corpseEnergyOffset : 0)` | `:256-266` |
| `age` | `clock.timeAlive / 3600` h | `:304` |
| `agingThreshold` | `baseAgingThreshold / metabolicRate` h | `:306` |
| `ageingFactor` | `max((age−agingThreshold)/agingThreshold, 0)` | `:308` |
| `ageStrengthPenaltyFactor` | `(1−ageStrengthPenalties)^ageingFactor` | `:310` |
| `ageMetabolismPenaltyFactor` | `ageingFactor·ageMetabolismPenalties` | `:312` |
| `metabolism` | `metabolismCost·baseBodyArea·(1+ageMetabolismPenaltyFactor)` E/s | `:745-748` |
| `metabolicEfficiency` | `energyUsageEfficiency^SpeedRatio` (both settings `float`) | `BibiteGenes.cs:137` |
| `healMaxRate` | `(1/d2Size)^healPowerFactor · healRate · metabolicRate` (fraction of max HP/s) | `:244` |
| `armorStrength` | `2·armor.armorStrength` | `:246` |
| `EnergyFlowRate` | UI-only net rate, see §4.4 | `:232-242` |

### 1.4 Gene interface used by body/organs

Gene array `genes[35]`, enum order exactly (`BibiteGenes.cs:16-53`); editor defaults/ranges in
`BibiteEditorSettings.cs`. Body-relevant subset:

| Idx | Gene | Editor default | Range | Body use |
|---:|---|---:|---|---|
| 1 | `BroodTime` | 20 s | 1…101 | birth maturity `(HatchTime/BroodTime)²` |
| 2 | `HatchTime` | 10 s | 1…101 | birth maturity; egg incubation |
| 3 | `SizeRatio` | 1 | 0.05…2.55 | `d2Size = growth·SizeRatio²`; linear scaling of all `d1`-derived quantities |
| 4 | `SpeedRatio` | 0.75 | 1e-6…2.5 | `metabolicRate`; scales digestion, movement strength, growth rate, heal rate; lowers efficiency |
| 12 | `ViewAngle` | 75° | 0…360 | body energy density cost; FOV |
| 13 | `ViewRadius` | 180 u | 0…200 | body energy density cost; FOV radius `×d1Size` |
| 14 | `ClockSpeed` | 1 s | 0.1…10.1 | `InternalClock.period` |
| 15 | `PheroSense` | 150 u | 0…500 | pherosense radius |
| 16 | `Diet` | 0.2 | 0…1 | stomach affinity, mouth width, grab penalty |
| 21 | `HerdSeparationDistance` | 35 u | 0…100 | FOV herd distance `×d1Size` |
| 20 | `HerdVelocityWeight` | 0.25 | 0…1 | acceleration added from herd projection |
| 22 | `GrowthScale` | 0.15 | 0…2 | growth Δ scaling |
| 23 | `GrowthMaturityFactor` | 2 | 0…10 | growth rate denominator |
| 24 | `GrowthMaturityExponent` | 1 | 0…10 | growth rate denominator exponent |
| 26 | `StomachWAG` | 2 | −1…9 | stomach capacity |
| 27 | `WombWAG` | 1 | −1…9 | egg capacity; `growthAtMature` |
| 28 | `FatWAG` | 1 | −1…9 | fat base capacity/inflation |
| 29 | `ArmorWAG` | 0.15 | −1…9 | armor thickness/strength |
| 30 | `ThroatWAG` | 1 | −1…9 | throat area → mouth opening/swallow width |
| 31 | `MouthMusclesWAG` | 1 | −1…9 | jaw area/strength, bite period |
| 32 | `MoveMusclesWAG` | 1 | −1…9 | thrust/torque, movement cost base |
| 33 | `FatStorageThreshold` | 0.5 | 0…1 | fat deadband centre |
| 34 | `FatStorageDeadband` | 0.25 | 0…0.5 | fat deadband half-width |

Source: defaults/ranges `BibiteEditorSettings.cs` (diet 75-89, sizeRatio 91-106, speedRatio 108-123,
broodTime 204-217, hatchTime 219-232, viewRadius 234-247, viewAngle 249-262, clock 264-277, pherosense
279-292, herd 294-371, growth 373-420, WAGs 422-532, fat 534-564). `TotalOrganWAG` sums genes 26…32 with
`max(0,x)` (`BibiteGenes.cs:223-226`).

> **Gene default vs starter template:** `BibiteEditorSettings.DefaultValue` (above) is what is substituted when
> a saved genome lacks a named gene (`BibiteGenes.cs:607-619`, `BibiteTemplate.cs:337-361`). The shipped
> `Basic bibite.bb8template` uses a different "starter" profile (`SpeedRatio 1`, `Diet 0.3`, `GrowthScale 0.35`,
> `StomachWAG 5`, `WombWAG 2.5`, `FatWAG 2.5`, `ArmorWAG 0.06684`, `ThroatWAG 1.31016`,
> `MouthMusclesWAG 1.06952`, `MoveMusclesWAG 0.65508`, `PheroSense 160`). Don't confuse them: defaults are only a
> fallback for missing genes; normal genomes always carry all 35 values.

### 1.5 Organ area apportionment (WAG)

`BibiteSpatialOrgan.InitOrgan` (`BibiteSpatialOrgan.cs:21-31`):

```
organAreaPortion = max(gene[organGene], 0) / totalOrganWAG        // totalOrganWAG = Σ max(0, genes[26..32])
if organAreaPortion > 0: growth.onGrowth += OnGrowth
OnGrowth() ⇒ area = max(baseBodyArea · organAreaPortion, 0)
```

The seven apportioned organs and their effects:

| Organ | WAG gene | `area` meaning | Derived constants |
|---|---|---|---|
| stomach | 26 | capacity (u², 1 u² = 1 amount unit) | `fullness = totalAmount/area` |
| womb (`eggLayer`) | 27 | egg capacity area | `eggCapacity = floor(area / areaAtBirth)` |
| fat | 28 | base fat capacity | `maxInflateAmount = baseBodyArea·0.35`; `area = baseCapacity·min(2, 1+0.35/portion)` |
| armor | 29 | shell cross-section | `armorThickness`, `armorStrength` (§5.6) |
| throat (`mouth`) | 30 | throat opening | `throatWidth = √area` |
| jaw muscles (`mouth`) | 31 | jaw cross-section | `jawArea = baseBodyArea·jawPortion`, `jawMass`, `jawStrength`, `bitePeriod` |
| propulsion (`move`) | 32 | muscle cross-section | `muscleMoveStrength`, `muscleTurnStrength`, `baseAccelerationCost` |

Non-spatial organs (`mouth.mass`, `move.mass` are hard `0f`; `pheromoneOrgan`, `pherosenseOrgan` have no area).

### 1.6 Materials (`MatterMaterial`) — semantics and `0.6.3.1` defaults

Material math (`MatterMaterial.cs:41-64`):

```
conversionEfficiency(affinity) = MinConversionEfficiency + clamp01(affinity)·(MaxConversionEfficiency − MinConversionEfficiency)
EnergyOfAmount(amount)         = amount · EnergyDensity
AmountOfEnergy(energy)         = energy / EnergyDensity
RadiusOfAmount(amount)         = √(amount/π)
```

Built-in `0.6.3.1` defaults (values stored in built-in ScriptableObjects; **not** in `Here/` C# — verified from
every default-scenario save: `...\Savefiles\beeg\saves\_00.zip\settings.bb8settings`, and cross-checked with
`GAME_GUIDE.md`):

| Material | Cohesiveness σ_y (N/u) | Reactivity (u/s) | EnergyDensity (E/u²) | MassDensity (g/u²) | MinEff | MaxEff | decay |
|---|---:|---:|---:|---:|---:|---:|:--:|
| Plant | 10 | 0.03 | 5 | 0.015 | −0.25 | 0.50 | no |
| Meat | 35 | 0.075 | 10 | 0.025 | 0.15 | 0.85 | yes: `freshTime 2000 s`, `decayRate 0.02 u²/s` |
| Fat | 0.25 | 1.0 | 30 | 0.03 | 0.50 | 0.85 | no |
| Armor | 500 | 1.0 | 1.5 | 0.05 | 1.0 | 1.0 | no |

> **[S] Variability warning:** these are *defaults*. A scenario's `settings.bb8settings.materials` overrides them
> at load (`MatterMaterialManager.LoadState`, `MatterMaterialManager.cs:157-170`). A real custom save used
> `Armor.cohesiveness = 75`, `Plant.reactivity = 0.025`, `Meat.reactivity = 0.08333`. Always read materials from
> the scenario/settings file; never hardcode them beyond defaults.
> `Energy` and `Health` materials are **not serialized** in any save file; their density values live only in the
> built-in assets and are needed for starvation/corpse math (open question BQ-1).

Runtime material lookup: `PhysicalMaterials = [Plant, Meat]` only (`MatterMaterialManager.cs:73-91`). That makes
the stomach content array length **2** (`BibiteStomach.cs:149`), and `AddMatter` can only ever store Plant or
Meat (Fat/Armor/Energy/Health are not physical stomach materials).

`Matter` container math (`Matter.cs:16-57`): `fullness = max(Amount/MaxAmount,0)`, `isEmpty`/`isFull` are
`Approximately` checks, `overflow = max(Amount−MaxAmount,0)`, `available = max(MaxAmount−Amount,0)`,
`radius = √(Amount/π)`, `mass = |Amount·MassDensity|`, `energy = Amount·EnergyDensity`,
`ConvertToFromEnergy(amount, affinity)` implements +amount storing / −amount burning with efficiency.

### 1.7 Body geometry (colliders and sprite scaling)

**Body capsule** (`BibiteBody.cs:728-737`): `CapsuleCollider2D` on the root, local-space values (Unity
`CapsuleCollider2D.size` = width × height, `offset` in local units; direction and `isTrigger` are prefab
settings `[PREFAB]`):

```
offset = (0, lerp(−4.7, −5.5, inflateFactor))
size   = ( lerp(5.0, 8.5, inflateFactor), lerp(7.0, 10.5, inflateFactor) )
world  = local × transform.localScale (= d1Size)
```

At `inflateFactor = 0` (no fat overflow) the capsule is `5·d1 × 7·d1` with centre `4.7·d1` behind the root;
at `inflateFactor = 1` it is `8.5·d1 × 10.5·d1`, centre `5.5·d1` behind. `transform.localScale =
Vector3.one·d1Size` whenever size update is enabled (`BibiteBody.cs:699-714`).

**Mouth trigger** (`BibiteMouth.cs:295-298`):

```
mouthTrigger = BoxCollider2D on the mouth object
size = (2.72, diet < 0.5 ? 7 − 8·diet : 3)     // width × length, local units, then scaled by d1Size
```

So herbivore (`Diet=0`) mouth box is `2.72×7`; omnivore `Diet=0.5…1` is `2.72×3`; it is a **trigger**
(only `OnTriggerEnter2D`/`OnTriggerExit2D` are implemented) and is disabled when the bibite dies
(`BibiteMouth.cs:510-519`).

**Body parts** (`BibitePart`, `BibiteBody.cs:518-528`): child GameObjects each with a (solid) `Collider2D`;
within one bibite every pair is passed to `Physics2D.IgnoreCollision`. Their shapes, offsets, tags and whether
they also have `Rigidbody2D`/joints are prefab data — **not recoverable from scripts** (`BQ-2`, `BQ-5`). They
are tagged `"bibitePart"`, are the actual bite target for mouth triggers, the collision-damage detectors, and
the objects radio zones/red death test.

**Sprite scaling** (visual only; needed only for `transform.scale` round-trip, which stores `d1Size`):
`BibiteProceduralSpriter` renders at `localScale = d1Size`, sprite PPU is generated as
`pixelWidth/9` for body parts (`ProceduralSpriteManager.cs:222-233`), and the skin shader gets
`_upp = d1Size/ppu` (`BibiteBody.cs:500-504,710-714`). Egg sprite is `pixelWidth/11`
(`ProceduralSpriteManager.cs:146-147`); pellet sprite `pixelWidth/5` (`:172-186`).

### 1.8 Rigidbody2D configuration

Set from code every tick (`BibiteBody.cs:592-639,739-743`):

| Property | Value | Source |
|---|---|---|
| `mass` | `body.mass` (see §1.3) | `:636` |
| `inertia` | `mass/2 · realBodyArea/π` (overrides auto-inertia; units mass·u²) | `:637` |
| `linearDamping` | `dragCoefficient·d1Size / mass` → = `dragCoefficient/(bibiteMassDensity·80·d1Size)` while `mass ≈ baseMass` | `:741` |
| `angularDamping` | same value as `linearDamping` (overwrites the `dragCoefficient/2` set in `Awake`) | `:742`, `:402` |
| `gravityScale` | **must be 0** (no code sets it; a non-zero value would make bibites fall) `[PREFAB]` | — |
| `bodyType`, `constraints`, `interpolation`, `collisionDetectionMode`, `sleepMode` | `[PREFAB]`; must be Dynamic, no constraints, no interpolation/sleep for parity | — |
| `position`, `linearVelocity`, `rotation` | saved/loaded (`SerializationHelper.cs:189-258`) | — |
| `angularVelocity` | derived by physics; `BibitePropulsion.AngularVelocity = rb2d.angularVelocity/360` (turns/s) | `BibitePropulsion.cs:254` |
| friction / bounciness / PhysicsMaterial2D | **no code sets them** — prefab/collider values; Unity defaults (friction ≈ 0.4, restitution 0) apply `[PREFAB]` | — |

Forces are applied in **`Force`** mode, impulses in **`Impulse`** mode (numeric 0/1 in decompiled enum casts):
`AddForce`/`AddTorque` in `BibitePropulsion.cs:314-316`; `AddForce(..., ForceMode2D.Impulse)` in
`BibiteMouth.cs:772,802,807,899`; `MatterPellet` uses `Force` in `RipChunkOff`? no — it sets `rb.position`
(`MatterPellet.cs:219`).

Collision layers: named layers `"bibite"`, `"pellets"` are resolved at runtime
(`FieldOfView.cs:190-198`); tags `"bibite"`, `"bibitePart"`, `"egg"`, `"pellet"` drive behaviour. The full
Physics2D layer collision matrix is a ProjectSettings value `[PREFAB]`; §10.4 lists the required pair
behaviour derived from code.

---

## 2. Tick pipeline, update order, timing

### 2.1 Tick length and time scale

* `Time.fixedDeltaTime = 1/simTPS` (`TimeController.cs:58,78-82`); at `timeScale < 1` it is multiplied by the
  time scale (`:117-129`). **Interpretation:** when the player slows time, the physics tick gets longer
  (0.0333·scale), not more ticks per second.
* `simTPS` is a **scenario-independent setting**, default `30` in the settings class
  (`ScenarioIndependentSettings.cs:170-180`); a new scenario/UserSettings default is `40`
  (`UserSettings.cs:439-443`), and existing saves carry whatever value they were created with (e.g. `15` in
  `...\Savefiles\beeg\saves\_00.zip`, `30` in the `20261008` autosave). **Read it from the file; never
  hardcode.** The foundation clock now also defaults to `30` (`clock.rs::DEFAULT_SIM_TPS`); BQ-7 is resolved.
* `brainTPS` default 2 (`ScenarioIndependentSettings.cs:182-192`): brain/sense update every 2 ticks.
  `brainPeriod = brainTPS/simTPS` seconds (used by Differential/Integrator, `NEATBrain.cs:335,341`).
* `visionLookupUpdateFactor` default 10, `visionSenseUpdateFactor` default 5: counter resets are in **brain
  updates** (`BibiteBody.cs:604-621`).
* `decayTPS` default 2 for pellet decay (`MatterDecayProcessor.cs:106-108,149-160`).
* `dv = Time.fixedDeltaTime` is passed into every formula as `dt`.

### 2.2 `Update()` is visual/UI only

`BibiteBody.Update` (`BibiteBody.cs:573-590`) only pushes `_RootPos`/`_Dir` to the skin shader and decays the
red damage flash (`damagedFlash -= dt/0.5`). No simulation state. `PheromoneSpot.FixedUpdate` particle visuals
are also visual-only (`PheromoneSpot.cs:96-145`). Per ARCHITECTURE §6 we reproduce the fixed-tick behaviour and
ignore `Update`-driven visuals.

### 2.3 `BibiteBody.FixedUpdate` — exact order [C]

`BibiteBody.cs:592-639`:

```text
if (!born || timeScale <= 0) return
if (dead) { CorpseUpdate(); return }

1. UseEnergy((metabolism + brain.brainUpkeepCost) * dt)          // base metabolism + brain upkeep, /efficiency
2. brainTicksCount += 1
   if brainTicksCount >= brainTickFactor:                         // default 2
       visionLookupCount += 1
       if visionLookupCount >= visionLookupPeriod: fow.FindSeenEntities(); visionLookupCount -= period
       visionSensingCount += 1
       if visionSensingCount >= visionSensingPeriod: fow.ComputeSenses();  visionSensingCount -= period
       brain.Thinker()                                            // UpdateSenses + evaluate network
       brainTicksCount -= brainTickFactor
3. attackedDmg = max(0, attackedDmg·0.70716^dt − 0.003·dt)
4. for o in [armor, stomach, eggLayer, fat, move, mouth, pheromoneOrgan]: o.UpdateOrgan()
5. pherosenseOrgan.PherosenseAround()
6. if health is NaN: Die()
7. Heal()
8. rb2d.mass = mass; rb2d.inertia = mass/2 · realBodyArea/π
9. UpdateDrag()                                                   // sets linear+angular damping
```

The organ order in step 4 is fixed by the list literal built once in `FinalizeBirth`
(`BibiteBody.cs:505`): **armor → stomach → eggLayer → fat → move → mouth → pheromoneOrgan**.
`armor.UpdateOrgan` is empty (`BibiteArmor.cs:64-66`). `mouth.UpdateOrgan` is where all biting/swallowing/
grabbing happens for this tick (§5.3); it uses the mouth-object list filled by *trigger callbacks from the
previous physics step*.

Separate `MonoBehaviour.FixedUpdate` methods that are NOT in that list:

| Component | FixedUpdate content | Source |
|---|---|---|
| `BibiteGrowth` | growth step every `growthUpdatePeriod` seconds (adaptive 0.2/rate), changes `d2Size` mid-tick | `BibiteGrowth.cs:91-107` |
| `InternalClock` | tic toggle, chrono reset/advance, ageing event every 120 s, `timeAlive += dt` | `InternalClock.cs:65-97` |
| `MatterDecayProcessor` | meat pellet decay job every `decayTPS` ticks | `MatterDecayProcessor.cs:133-195` |
| `PheromoneSpot` | pheromone strength fade / particle emit | `PheromoneSpot.cs:96-145` |
| `EggHatching` | hatch progress / abort check | `EggHatching.cs:159-173` |

**Ordering between those separate components is unspecified** in `Here/` (no `[DefaultExecutionOrder]`, no
script execution order asset in the decompiled scripts). Unity uses its own (undocumented, load-order-based)
order. Effects are one-tick-scale only, except: if `BibiteGrowth` runs after `BibiteBody`, the freshly grown
health/energy maxima and `growthCostRate` are accounted in the *next* tick's `body` update and the physics
mass is one tick stale. See §10.7 and BQ-3.

### 2.4 Physics step and contact callbacks

Unity fixed step order is: `FixedUpdate()` callbacks → 2D physics simulate → `OnCollision*`/`OnTrigger*`
callbacks. Consequences that matter for us:

1. Propulsion forces/torque applied in this tick's `move.UpdateOrgan` are integrated by the same tick's
   physics step.
2. Mouth trigger enter/exit and part collision damage callbacks fire **after** `BibiteBody.FixedUpdate`, so the
   mouth acts on the entity list produced by the previous tick's physics.
3. `BibitePart.OnCollisionEnter2D` damage is applied during the physics callback phase (after body tick).
4. `BibiteBody.OnJointBreak2D` (grab joint break) is a physics callback → `mouth.OnLinkBroke`.

### 2.5 Per-tick scratch vs persisted state

* Persisted across a save/load: everything in §1.2, clock fields (`tic`, `ticProgress`, `timeAlive`,
  `chronoTime`), organ save states (§9).
* Reconstructed on load: `d1Size`, `baseMass`, `metabolism`, `maxHealth`, `Energy.MaxAmount`, organ areas,
  jaw/muscle constants, `inflateFactor` (from fat amount), `born` (=true), `growth` (from `d2Size`), `mature`.
* Not persisted (reset to defaults on load): mouth `desire*` values, mouth `nInMouth`/`nHeld` (grab links are
  re-created by `SerializeConnections` in world saves, `SaveSystem.cs:318-376`), `totalContent`, `acidLevel`,
  `digestionEffectiveness`, `fat.inflateFactor` (recomputed), `move` velocity stats, `vision` seen lists,
  `FOV` weights, `pherosense` sums, `PheromoneSpot` particle counters, `BibiteGrowth.progress`,
  `BibiteGrowth.growthUpdatePeriod`, `growthCostRate`, `eggLayer.isMature` (recomputed from maturity on
  `ResumeGrowth`), `BibiteEggLayingOrgan.children` (world saves rebuild it from `childIDs`; single `.bb8` saves
  do not persist children IDs).

### 2.6 Recommended canonical Rust order (deviation; see §10.3–§10.4)

```text
World::tick(dt):
 1. settings animators / world tools / zones spawn & decay (spec 03)
 2. for each bibite: base metabolism + brain upkeep charge            (game: start of body tick)
 3. due brain/sense updates (brain ticket, vision throttles)          (game: step 2 of body tick)
 4. organ updates in game order: armor, stomach, egg, fat, move, mouth, pheromone
 5. pheromone sensing                                                 (game: right after organs)
 6. growth update (chosen: after organ updates, before size-dependent physics)
 7. clock/ageing update (chosen: alongside growth)
 8. heal; NaN/death checks
 9. recompute mass/inertia/damping (size may have changed)
10. physics integrate + contacts (custom solver), using forces/torque produced in step 4
11. process contact callbacks produced by step 10: mouth enter/exit lists for next tick,
    collision damage (game: during step 10)
12. egg hatching / egg laying post-conditions, deaths/corpses
```

Deviations recorded in `docs/ARCHITECTURE.md` §14 style: (a) unrelated components are put in one explicit
order, (b) contact lists are carried explicitly rather than via Unity callbacks, (c) Box2D integration is
replaced by our solver. Keep the *within-bibite organ order* exactly as the game.

**Cross-spec note.** `docs/spec/03-world-ecology.md` §9.2 phase 5 summarises the body pipeline as
`clock → growth → brain → organs → propulsion → UseEnergy`. That one-liner is a simplification: the
authoritative within-bibite order is §2.3 above — base metabolism + brain upkeep are charged *first*, then the
brain/sense cadence, then organs in the literal list order `armor, stomach, eggLayer, fat, move, mouth,
pheromoneOrgan`, then pherosense, NaN check, heal, and mass/inertia/damping refresh. Growth/clock/corpse
positions are the chosen §2.6 boundaries. World phases (zones before bibites, eggs after bibites, physics after
all FixedUpdate work) are compatible with this; the Lead should reconcile 03's summary line. Ownership split:
`physics.rs` (contact solver, this spec §10.3) is body-dev's; pellet/zone/pheromone *state, queries and
lifetime* are world-dev's (`03`). The interfaces 03 asks body to expose (`BibiteView`-style read accessors and
the `pellet_change_amount`/`spawn_meat_pile` choke points) can be provided by `body.rs`/`physics.rs` without
foundation changes; coordinate names at implementation time.

---

## 3. Body size, growth, maturity, ageing

### 3.1 `Set2DSize` — the single size mutation point [C]

`BibiteBody.cs:699-726`:

```text
fn set_2d_size(new_size: f32, initial_set: bool, update_scale: bool) -> f32:
    delta_d2  = new_size - d2Size
    d2Size    = new_size
    d1Size    = sqrt(d2Size)
    delta_area = delta_d2 * 80                 // u², using the constant 80 (same as baseBodyArea)
    if update_scale:
        transform.localScale = (d1Size, d1Size, 1)      // only when the caller asks (growth threshold)
        if born: skin._upp = d1Size / sprite_ppu
    if !initial_set:
        health += healthPerArea * delta_area            // health grows with body (can go down if shrinking)
    maxHealth       = healthPerArea * baseBodyArea
    Energy.MaxAmount= baseBodyArea * storableEnergyPerArea
    UpdateMetabolism()                                  // metabolism = cost*area*(1+age penalty)
    fow.herdSeparationDistance = gene[HerdSeparationDistance] * d1Size
    fow.viewRadius             = gene[ViewRadius] * d1Size
    baseMass        = bibiteMassDensity * baseBodyArea
    return delta_area * baseEnergyDensity * (1 + healthEnergyFactor)   // energy to grow this delta
```

Return value is the **growth energy cost** (E); if `delta_area` is negative (shrink) it is negative (energetic
gain; see §4.2). `maxHealth` is overwritten after the health delta, so `health/maxHealth` drifts slightly on
shrink; no clamp is applied here.

### 3.2 Birth: `StartBody` [C]

`BibiteBody.cs:413-441`:

```text
StartBody():
    d2Size = 0; naturalBirth = true
    metabolicEfficiency = gene.MetabolicEfficiency
    metabolicRate       = gene.metabolicRate                 // SpeedRatio
    new_size = gene.D2SizeAtBirth
    body_energy_at_birth = gene.BodyEnergyAtBirth
    baseEnergyDensity = gene.BodyEnergyDensity
    if Energy.Amount < body_energy_at_birth:
        new_size = Energy.Amount / body_energy_at_birth      // scale down if the egg was under-energised
    qtyE = Set2DSize(new_size, initial_set = true)
    health = maxHealth
    UseEnergy(qtyE, useEfficiency = false)                   // build the body from energy, no efficiency penalty
    if Energy.Amount > Energy.MaxAmount: UseEnergy(Energy.overflow, false)
    growth.StartGrowth()                                     // growth = gene.BibiteGrowthAtBirth
    clock.StartClock()
    species bookkeeping
    brainTicksCount   = Rand(0, brainTickFactor)             // 0..brainTPS-1
    visionSensingCount= Rand(0, visionSensingPeriod)
    visionLookupCount = Rand(0, visionLookupPeriod)
    FinalizeBirth()                                          // sprites, organs, fov genes, parts, id, flags
    BibiteTracker.BibiteBirth()
```

`FinalizeBirth` (`:488-571`) builds the organs list and calls `InitOrgan` on each (which computes all
gene/area-derived constants), copies FOV genes, `StartPart()` on all body parts, ignores intra-bibite part
collisions, assigns `id` (random non-zero int), applies easter-egg flags (`isRad/isRain/isSource/isKiller`),
sets `born = true`, then: if dead → `CorpseUpdate()`; else species/tag registration.
If `dying` was set before `Start()` (load path), `Start()` calls `Die()` (`:405-411`).

### 3.3 Load/spawn variants

`StartBodyAtGrowthAndNormalize(targetGrowth)` (`:443-458`):
`new_size = targetGrowth·SizeRatio²`; `Set2DSize`; `health = maxHealth`; `Energy.Amount = Energy.MaxAmount`;
`growth.growth = targetGrowth`; random brain/vision phases; `ResumeBody()`.
Used for spawned adult/elder/baby bibites (`WorldObjectsSpawner.cs:182-188`) — `GrowthAtSpawn.Baby` uses
`GrowthAtBirth`, `Adult` uses `GrowthAtMature`, `Elder` uses `2·GrowthAtMature`.

`ResumeBody()` (`:460-486`): recompute `baseEnergyDensity/metabolicEfficiency/metabolicRate`; `Set2DSize(d2Size,
initial_set=true)`; `Hurting(Health.overflow)` (clamps loaded health > max by "damaging" the excess);
`growth.ResumeGrowth()`; if dead stop growth+clock else resume clock; ensure species; `FinalizeBirth()`;
tracker. This is the load path for a live bibite or corpse.

### 3.4 Growth organ [C]

`BibiteGrowth.cs` (whole file). Constants: `growthUpdatePeriod` initial 0.2 s; `sizeUpdateThreshold = 0.005`;
`progress` private; `growth` serialized (`growth` only — the rest recomputed).

Init (`ResumeGrowth`, `:74-89`):

```
metabolicRate        = gene.metabolicRate                       // SpeedRatio
growthAtMature       = gene.GrowthAtBigEnoughEggOrgan
mature               = growth >= growthAtMature
if mature: eggLayer.isMature = true
appliedGrowth        = growth
sizeRatio            = gene.SizeRatio
growthScale          = gene.GrowthScale
growthMaturityFactor = gene.GrowthMaturityFactor
growthMaturityExponent= gene.GrowthMaturityExponent
growing              = true
```

Per fixed tick (`:91-107`; **at most one growth step per tick**, `progress` is not a while-loop):

```
if growing && timeScale > 0:
    progress += dt
    if progress >= max(growthUpdatePeriod, dt):
        growthUpdatePeriod = max(growthUpdatePeriod, dt)
        progress -= growthUpdatePeriod
        rate = GrowthRate()                          // = metabolicRate / (1 + factor·maturity^exp)
        delta = brain[Want2Grow] * rate * growthUpdatePeriod * growthScale / 100
        onGrowth(growth + delta)                     // notifies spatial organs (they only re-read area)
        SetMaturity(growth + delta)
        growthUpdatePeriod = 0.2 / rate              // next period adapts to rate
```

`SetMaturity` (`:124-144`):

```
growth = new_maturity
changed = (growth - appliedGrowth) / appliedGrowth > 0.005      // guard: only rescale transform if >0.5% growth
growthCost = body.Set2DSize(growth·sizeRatio², initial_set=false, updateScale=changed)
body.UseEnergy(growthCost)                                       // default efficiency (divided by MetabolicEfficiency)
growthCostRate = growthCost / growthUpdatePeriod                 // E/s used by EnergyFlowRate
if changed: appliedGrowth = growth
if !mature && growth >= growthAtMature: mature = true; eggLayer.isMature = true
if mature && growth < growthAtMature:  mature = false            // NOTE: eggLayer.isMature is never reset
```

Formulas:

```
growthAtBirth        = (HatchTime / BroodTime)²                       BibiteGenes.cs:163-166
growthAtMature       = max(1, growthAtBirth / max(wombAreaPortion, 0.001))    :168-171
maturity             = growth / growthAtMature
growthRate(m)        = metabolicRate / (1 + GrowthMaturityFactor·m^GrowthMaturityExponent)  :114-117
AreaAtMature         = 80·growthAtMature·SizeRatio²                    :173-176
LengthAtMature       = 12.5·√growthAtMature·SizeRatio                  :178-181
```

Quirks to preserve: growth cost is computed from `delta_d2·80` (not from the actual area change after
float rounding); when `Want2Grow` comes from a **Sigmoid** output it is in [0,1] and growth is monotone, but if
mutation changes that output node to TanH it can be negative → shrink (negative growth cost → energy gain,
health decreases); the maturity latch on `eggLayer.isMature` never resets on de-maturity; `progress`/period
are not saved.

### 3.5 Ageing [C]

`InternalClock` (`InternalClock.cs`): `period = genes[14]` (`ClockSpeed`, seconds); tic flips 0↔1 when
`ticProgress` crosses `period`; `ClkReset > 0.75` zeroes `chronoTime`, else `chronoTime += dt`;
`timeAlive += dt`; every 120 s of sim time it invokes `OnAging` → `BibiteBody.UpdateMetabolism()`
(`BibiteBody.cs:510,745-748`). `timeAlive` in **seconds**, `age` in hours.

```
age = timeAlive/3600
agingThreshold = baseAgingThreshold/metabolicRate                 // higher SpeedRatio ⇒ later ageing
ageingFactor = max((age - agingThreshold)/agingThreshold, 0)
ageStrengthPenaltyFactor   = (1 - ageStrengthPenalties)^ageingFactor     // used by propulsion (movementPenalty)
ageMetabolismPenaltyFactor = ageingFactor · ageMetabolismPenalties
metabolism = metabolismCost · baseBodyArea · (1 + ageMetabolismPenaltyFactor)
```

Note: ageing does **not** reduce mouth/jaw strength in code (despite the settings help text); only movement is
weakened through `movementPenalty`, and metabolism is raised. The `EE` "age strength penalty" description
mentions mouth but `BibiteMouth` never reads `ageStrengthPenaltyFactor` — divergence between docs and code,
code wins.

### 3.6 Worked example — shipped starter bibite (all numbers derived from §1.4/§3 formulas)

Inputs: `SizeRatio 1`, `HatchTime 10`, `BroodTime 20`, `SpeedRatio 1`, `ViewAngle 180`, `ViewRadius 75`,
WAGs `5 / 2.5 / 2.5 / 0.06684 / 1.31016 / 1.06952 / 0.65508` (total 13.1016), `Diet 0.3`, settings at defaults.

| Quantity | Birth | Mature (`growth = 1.31016`) |
|---|---:|---:|
| `d2Size` (`growth·SizeRatio²`) | 0.25 | 1.31016 |
| `baseBodyArea = 80·d2Size` | 20 u² | 104.81 u² |
| `bodyLength = 12.5·√d2Size` | 6.25 u | 14.31 u |
| `maxHealth = 1.25·area` | 25 HP | 131.02 HP |
| `Energy.MaxAmount = 1.5·area` | 30 E | 157.22 E |
| `baseMass = 0.025·area` | 0.5 | 2.62 |
| `BodyEnergyDensity` | 2.69765 E/u² | same |
| `BodyEnergyAtBirth` (`100·d2·SumRatio`) | 101.16 E | — |
| `LayBodyEnergy` (`100·d2·(storable+SumRatio)`) | 138.66 E | — |
| `metabolism = 0.001·area` (before /0.8) | 0.02 E/s | 0.1048 E/s |
| `womb portion` | — | 2.5/13.1016 = 0.19082 |
| `growthAtMature` | 1.31016 | — |

Also: `mouth throatWidth = √(area·throatPortion)` = √(104.81·0.1)=3.24 u; `jawStrength` = 80·(jawArea·1)^(2/2)
= 80·8.556 = 684.5; `bitePeriod` = √(3.24·0.107/684.5)·70 ≈ 1.57 s at SpeedRatio 1.

---

## 4. Energy and health accounting

### 4.1 Matter containers

`Energy`: `Matter{Material=Energy}`, `Amount` = active energy (E), `MaxAmount` = `area·storableEnergyPerArea`.
`Health`: `Matter{Material=Health, MaxAmount=1}` initially → overwritten to `healthPerArea·area`; `Amount` in HP.
`fat.fatReserves`: `Matter{Material=Fat}`, `Amount` in u², `.energy = Amount·30`.

### 4.2 `UseEnergy` / `GainEnergy` — exact [C]

`BibiteBody.cs:658-697`:

```text
UseEnergy(qtyE, useEfficiency = true):
    q = qtyE / (useEfficiency ? metabolicEfficiency : 1)
    if Energy.Amount > q:
        Energy.Amount -= q
    else:
        q -= Energy.Amount
        Energy.Amount = 0
        Hurting(Health.Material.AmountOfEnergy(q))       // energy debt is paid with health (BQ-1 density)

GainEnergy(qtyE) -> f32:                                    // WARNING: also used with negative qtyE
    available = Energy.available
    if available > qtyE:
        Energy.Amount += qtyE
        return qtyE
    Energy.Amount = Energy.MaxAmount
    return available
```

Consequences:

* All costs passed through default `UseEnergy` are inflated by `1/MetabolicEfficiency = 1/(0.8^SpeedRatio)`
  (e.g. `SpeedRatio 2` ⇒ ×1.5625 on every cost).
* Negative `qtyE` (shrink growth cost, reabsorbed eggs via `GainEnergy(|Δ·eggEnergy·0.5|)` positive but
  `ConvertToFromEnergy` negative fat flows go through `GainEnergy`) behaves like a raw amount change without
  clamping semantics you'd expect: `GainEnergy(negative)` just subtracts (as long as `available > negative`).
* Energy shortfall converts to **health damage** through the `Health` material's energy density; health < 0 →
  `Die()`.

### 4.3 Every cost in the body/organs (rate & where charged)

| Cost | Rate / amount | Efficiency? | Charged by | Source |
|---|---|---:|---|---|
| base metabolism | `metabolismCost·baseBodyArea·(1+ageMetabPenalty)` E/s | yes (`/metabolicEfficiency`) | `Body.FixedUpdate` first line, `metabolism·dt` | `BibiteBody.cs:603,745-748` |
| brain upkeep | `nSynaps·synapseUpkeep + nHidden·neuronUpkeep` E/s | yes | same line, `brainUpkeepCost·dt` | `NEATBrain.cs:283`, `BibiteBody.cs:603` |
| movement accel | `moveMusclesCost·moveArea·|accel|` E/s | yes | move organ, `movementCost·dt` | `BibitePropulsion.cs:185-189,315,318` |
| movement turn | half of the above × `|turn|` E/s | yes | move organ | `:188,317-318` |
| growth | `Set2DSize` return E (one-off per growth step) | yes | `BibiteGrowth.SetMaturity` | `BibiteGrowth.cs:128-130` |
| heal | `healHP·HealthEnergyRatio/healEfficiency` E | yes (double penalty) | `Heal()` | `BibiteBody.cs:671-685` |
| egg production | `Δprogress·eggEnergy` E | yes | egg organ | `BibiteEggLayingOrgan.cs:131-143` |
| fat storing | `ΔfatAmount·30/Efficiency(affinity)` paid, negative via `GainEnergy` | via conversion eff | fat organ | `BibiteFatOrgan.cs:109-112`, `Matter.cs:52-57` |
| fat burning | `(−ΔfatAmount)·30·Efficiency(affinity)` gained | via conversion eff | fat organ | same |
| fat sustain | `fatReserves.energy·fatSustain` E/s | yes | fat organ | `BibiteFatOrgan.cs:97-98,113-114` |
| pheromone emit | `(outR+outG+outB)·pheromoneProductionCost` E per 1 s emission | yes | phero organ | `BibitePheromoneOrgan.cs:68-81` |
| corpse decay | `metabolism·corpseDecayFactor` E/s (from 0 energy ⇒ health drain) | yes | `CorpseUpdate` | `BibiteBody.cs:641-648` |
| brain birth | `nHidden·neuronBirthCost + nSynaps·synapseBirthCost` — **not charged by body code**; costs exist in settings and brain spec | — | (brain spec) | `ScenarioSettings.cs:717-743` |

`move.gear` note: muscle strength itself also scales with `metabolicRate` (`area·metabolicRate` inside a power),
so speed ratio raises both force and cost (cost quadratically-ish in practice).

### 4.4 `EnergyFlowRate` (UI/logging only) [C]

`BibiteBody.cs:232-242`:

```
if !dead:
    stomach.EnergyGainRate + fat.lastEnergyGainRate
    − (fat.lastEnergySustainRate + growth.growthCostRate + healingRateEnergyCost + metabolism
       + move.movementCost + brain.brainUpkeepCost + eggLayer.energyConsumption) / metabolicEfficiency
else:
    −(fat.lastEnergySustainRate + metabolism) / metabolicEfficiency
```

`stomach.EnergyGainRate = Σ content.digestionRate·material.EnergyDensity·lastConversionEfficiency`
(`StomachContent.cs:35`). This property is not used by the simulation itself; expose it for stats.

### 4.5 Health, healing, damage bookkeeping [C]

```
Heal() (BibiteBody.cs:671-685):
    if health < maxHealth:
        hp = min( maxHealth·healMaxRate·brain[Want2Heal]·dt, maxHealth−health )
        energyCost = hp·gene.HealthEnergyRatio / healEfficiency
        healingRateEnergyCost = energyCost/dt
        UseEnergy(energyCost)                  // again divided by MetabolicEfficiency
        health += hp
    else: healingRateEnergyCost = 0
```

`healMaxRate = (1/d2Size)^healPowerFactor·healRate·metabolicRate` (fraction of max HP per second; default
`healRate 0.001`, `healPowerFactor 1`, `healEfficiency 1`, `HealthEnergyRatio = BodyEnergyDensity·0.5`).

`Attack(dmg, drain)` (`BibiteBody.cs:750-760`):
```
dealt = Hurting(dmg)
attackedDmg += dealt
totalDamageSuffered += dealt
if !drain: timesAttacked += 1
return dealt · gene.HealthEnergyRatio          // energy value of the flesh/blood removed (E)
```

`Hurting(dmg)` (`:762-779`):
```
dealt = min(dmg, health + 0.01)
damagedFlash = max(damagedFlash, min(2·dealt/health, 1))     // visual only
health -= dealt
if health < 0:
    if !dead: Die()
    else:     ExplodeToMeat()
return dealt
```

`attackedDmg` decay (`FixedUpdate` step 3): `attackedDmg = max(0, attackedDmg·0.70716^dt − 0.003·dt)`
(per second that is ≈ ×0.707/s, matching the brain's `AttackedDamage = min(1, 2·attackedDmg/health)`,
`NEATBrain.cs:583-584`). Note this is the only "pain" record; **collision damage does not add to
`attackedDmg`** (it calls `Hurting` directly from `BibitePart`), so only bites create pain input.

NaN guard: if `health` is NaN at step 6, `Die()`. `BibiteStomach` also warns via `MathfE.SafeDiv` (`Here/ScriptHelpers/MathfE.cs:90-98`) but the body's guard is the effective one.

### 4.6 Energy spend order inside one tick (important for starvation)

Body tick order means, if energy hits 0 mid-tick, the deficit is converted to damage **immediately**
(inside `UseEnergy`), in this order:
1. base metabolism + brain upkeep (before anything else),
2. organ updates in listed order: armor (none), stomach (gains), eggLayer (spend/produce), fat (store/burn +
   sustain), move (spend), mouth (none directly — bites transfer into stomach, not active energy), pheromone
   (spend),
3. heal.
So a bibite that starves while moving takes movement-cost damage before healing is considered. `GainEnergy`
from digestion/fat happens in steps 2 (stomach, fat) before movement/egg? Order is stomach **then** egg **then**
fat **then** move — so digestion can fund the egg organ in the same tick; fat can fund movement; but egg spends
before fat burns, etc. Preserve this order.

---

## 5. Organs

### 5.1 Base classes

`BibiteOrgan` (`BibiteOrgan.cs`):
```
InitOrgan(body): body=..., genes=body.gene, brain=body.brain, metabolicRate=genes.metabolicRate,
                 body.onDeathStatusChange += b => OnDeathStatusChange(!b.dead)
UpdateOrgan(): abstract
OnDeathStatusChange(alive): this.enabled = alive          // MonoBehaviour.enabled (no effect on manual calls)
OnBodyDestroyed(): no-op
```

`BibiteSpatialOrgan` adds `area`, `organAreaPortion`, `mass` (abstract), `apportionmentGene` (abstract);
`InitOrgan` computes portion/area, subscribes `growth.onGrowth` if portion > 0 (see §1.5); `OnGrowth(val)`
recomputes `area = max(baseBodyArea·portion,0)` and derived constants via overrides.
`OnBodyDestroyed` unsubscribes from `growth.onGrowth`.

Organ-specific `OnDeathStatusChange` overrides:
* mouth: disables its trigger, releases held objects, `nInMouth = 0` (`BibiteMouth.cs:510-519`).
* eggLayer: on death, lays any complete eggs (subject to global limit) (`BibiteEggLayingOrgan.cs:215-231`).
* all others: default disable.
`OnBodyDestroyed` overrides: stomach spills contents as pellets (`BibiteStomach.cs:227-255`); mouth releases
held objects (`BibiteMouth.OnDestroy`, `:521-536`); `BibiteBody.ExplodeToMeat` calls `OnBodyDestroyed` on all
organs before destroying the object (`BibiteBody.cs:847-863`).

### 5.2 Stomach (`BibiteStomach.cs`, `StomachContent.cs`)

State: `stomachContents[2]` (`Plant`,`Meat` slots), `nContent` (0…2), `diet` gene cached at init.
Per-content struct: `material`, `amount` (u²), `averageChunkAmount` (u²), `efficiency` (cached conversion
efficiency), plus non-persisted `digestionRate`, `lastConversionEfficiency`.
Persisted content per slot: `{material: Name, amount, averageChunkAmount}` (`StomachContent.cs:49-71`,
confirmed in real `.bb8`). On load missing `amount` defaults to `0.0001`, missing `averageChunkAmount` defaults
to `amount`, both floored at 0; material looked up by name.

Derived:
```
mass      = (Σ amount·material.MassDensity) · stomachContentContribution    // default 1.0
fullness  = area > 0 ? totalAmount/area : 0
availableSpace = max(area − totalAmount, 0)
EnergyGainRate = Σ content.digestionRate·material.EnergyDensity·lastConversionEfficiency
```

`UpdateOrgan` (`:162-219`) — exact algorithm:

```
if isEmpty: return                                    // NOTE: totalContent/acidLevel/... keep stale values

flagFinished = false
totalContent = totalAmount
acidLevel = brain[Digestion]                          // sigmoid output 0..1
fill = fullness                                       // computed BEFORE this tick's digestion
acidRatio = fill > 0 ? acidLevel/fill : 0
excess    = acidLevel − fill
digestionPotentialBonus = acidLevel < fill ? 0 : excess          // useful excess
efficiencyMalus         = acidLevel < fill ? 0 : excess/2        // yields loss
digestionEffectiveness  = min(acidRatio,1) · metabolicRate · (1 + digestionPotentialBonus)

for c in contents[0..nContent):
    eff = c.efficiency
    convEff = eff > 0 ? eff·(1−efficiencyMalus) : eff − efficiencyMalus
    r = sqrt(c.averageChunkAmount/π)                             // characteristic chunk radius
    d = min( c.material.Reactivity · digestionEffectiveness · dt, r )   // depth digested this tick
    areaDelta = d·(2r − d)·π                                      // circular-segment area
    n = c.approximateN · areaDelta                                // approximateN = max(1, amount/avgChunk), 1 if avgChunk<=0
    if d >= r:                                                    // whole chunk dissolved
        n = c.amount; flagFinished = true
        c.amount = −0.0001; c.averageChunkAmount = 0
    else
        c.averageChunkAmount = max(0, c.averageChunkAmount − areaDelta)
        c.amount -= n
    c.digestionRate = n/dt
    c.lastConversionEfficiency = convEff
    GainEnergy( c.material.EnergyOfAmount(n) · convEff )
if flagFinished: compact contents (swap-remove any amount<=0), invoke onMaterialFinished per removed slot
```

`AddMatter(material, amount)` (`:257-292`):
```
if amount ≈ 0: return
slot = last index in [0,nContent) whose material == material, else nContent
amount = min(amount, availableSpace)
if slot == nContent:                       // new material
    contents[slot] = {material, amount=amount, averageChunkAmount=max(0,amount), affinity=Affinity(material)}
    onNewMaterialEntered; nContent += 1
else:                                      // merge into chunk-size distribution
    old = c.amount; oldAvg = c.averageChunkAmount
    c.averageChunkAmount = max(0, oldAvg/(1+amount/old) + amount/(1+old/amount))
    c.amount = old + amount
onMatterEnter(material, amount)
```
(Requires `old > 0`; a zero-amount stored slot would divide by zero → NaN. Keep contents strictly positive.)

`AffinityToMaterial` (`:323-334`): `Meat → Diet^meatAffinityPowerFactor`; `Plant → (1−Diet)^plantAffinityPowerFactor`;
other → 0. `efficiency = Material.ConversionEfficiency(affinity)` (set through the `affinity` setter,
`StomachContent.cs:37-43`; `ConversionEfficiency` in §1.6). With default powers 1:
`Diet=0` → plant 50 %, meat 15 %; `Diet=1` → plant −25 % (net energy loss), meat 85 %.
`efficiency` is re-derived on load and whenever the material's min/max efficiency settings change
(`BibiteStomach.cs:151-159,336-347`).

`RemoveMatter(material, amount)` (`:294-321`) — used by vomit. **Code bug:** it first does
`amount = min(amount, c.amount)`, then tests `if (amount > c.amount)` (can never be true), so the
"remove whole slot / invoke onMaterialFinished" branch is dead; it only subtracts and leaves the slot in place
(possibly at 0 amount). Missing material → `slot == nContent` reads the next array element (may be a stale
slot). Reproduce only if parity demands it; preferably implement the intended "remove up to amount, drop empty
slot" and log deviation (BQ-10).

`OnBodyDestroyed` spill (`:227-255`): each content with `amount > minPelletSize` is spawned as a free pellet of
that material/amount at the stomach's world position (`WorldObjectsSpawner.SpawnPelletOfMatter`).

Stomach mass is `content mass · stomachContentContribution` (default 1.0).

### 5.3 Mouth (`BibiteMouth.cs`) — triggers, bite, swallow, grab, vomit

#### 5.3.1 Init constants [C]

`InitOrgan` (`:275-307`):
```
links[10] = mainBody.GetComponents<FixedJoint2D>() (up to 10)      // pre-authored grab joints in prefab
mouthTrigger = BoxCollider2D; diet = gene[Diet]
mouthTrigger.size = (2.72, diet<0.5 ? 7−8·diet : 3)
rb = mainBody.Rigidbody2D; jawAreaPortion = gene.jawAreaPortion
UpdateMouthSizeParameters()
```
`UpdateMouthSizeParameters` (`:315-322`), also run on growth:
```
jawArea     = baseBodyArea · gene[MouthMusclesWAG]/totalWAG
jawMass     = jawArea · bibiteMassDensity / 2
throatWidth = sqrt(area)                     // area = baseBodyArea·ThroatWAG/totalWAG
UpdateJawStrength(); UpdateBitePeriod()
```
`UpdateJawStrength` (`:329-332`):
`jawStrength = bitingPressure · (jawArea·metabolicRate)^(jawMusclesSizingPower/2)`
`UpdateBitePeriod` (`:324-327`):
`bitePeriod = jawArea > 0 ? sqrt(throatWidth·jawMass/jawStrength)·70·bitePeriodFactor : 1e6`
(defaults: `bitingPressure 80`, `jawMusclesSizingPower 2`, `bitePeriodFactor 1`).

#### 5.3.2 Brain-driven threshold values [C]

```
grabOutput   = brain[Grab]
attackOutput = brain[Want2Attack]
swallowOutput= brain[Want2Eat]

biteForce    = desireToAttack > 0.15 ? jawStrength·desireToAttack : 0
biteWidth    = desireToAttack > 0.15 ? desireToAttack·throatWidth : 0
swallowWidth = swallowOutput  > 0.15 ? desireToSwallow·throatWidth : 0
swallowAmount            = π·(swallowWidth/2)²          // full mouth opening area
maxAmountNoBiteNeeded    = π·(swallowWidth/4)²          // "fits in mouth without a bite" area
ableToSwallow = swallowOutput > 0.15 && biteProgress >= bitePeriod
readyToAttack = attackOutput > 0.15 && biteProgress >= bitePeriod
```
Constants (`:121-125`): throwing desire threshold `−0.25`, biting `0.15`, vomiting `−0.15`; laying
want `0.25`; grab dead zone `|x| ≤ 0.15`; egg production dead zone 0.15 (egg organ).

#### 5.3.3 Per-tick state machine (`UpdateOrgan`, `:334-458`)

```text
if !isStarted || timeScale<=0: return
attackedLastFrame = attackedThisFrame; attackedThisFrame = false
if biteProgress < bitePeriod: biteProgress += dt          // always recharges

if swallowOutput < −0.15 && biteProgress >= bitePeriod:
    Vomit(); biteProgress -= bitePeriod
else:
    if nInMouth == 0 && nHeld == 0: return
    sort objectsInMouth by distance to mouth (nulls last)
    recompute nInMouth as index of first null
    if nInMouth == 0: return
    desireToAttack = attackOutput; desireToGrab = grabOutput; desireToSwallow = swallowOutput
    canSwallow = ableToSwallow
    mouthOpening = swallowWidth; biteOpening = biteWidth; biteStrength = biteForce

    if desireToGrab >= 0.15:
        for each object in mouth: grabbable = obj.Grabbable or parent Grabbable
                                  if found: TryGrabTarget(grabbable)          // ≤10 live joints
        breakForce = breakTorque = jawStrength·max(Diet,0.1)·desireToGrab/nHeld  (all links)
    else if nHeld > 0 && desireToGrab <= −0.15:
        ReleaseAndMaybeThrowAllHeldObjects(throw = true)

    for each object in mouth:
        if !canSwallow || desireToSwallow <= (target_is_grabbed ? desireToGrab : 0.15):
            if desireToAttack > 0.15: MaybeAttack(obj, ongoingAttack = attackedLastFrame)
        else if !TrySwallowTarget(obj) && desireToAttack > 0.15:
            MaybeAttack(obj, ongoingAttack = attackedLastFrame)
```

Trigger callbacks (`:460-487`): `OnTriggerEnter2D` appends to `objectsInMouth[10]` (drops if full, still
invokes `OnEnterMouth`); `OnTriggerExit2D` removes by swap-back, releases any joint on that rigidbody, invokes
`OnExitMouth`. Note objects merely *inside* the trigger are acted on every tick; nothing requires the target to
be centred.

#### 5.3.4 Swallowing (`TrySwallowTarget`, `:538-595`)

```text
if target.tag == "pellet":
    p = target.MatterPellet
    if p.radius > mouthOpening/2: return false
    if p.amount > stomach.availableSpace: return false
    if p.radius > mouthOpening/4:                        // needs a "bite" for medium pellets
        if biteProgress < bitePeriod: return false
        biteProgress -= bitePeriod
    stomach.AddMatter(p.material, p.amount); Destroy(target); return true

if target.tag == "bibitePart":
    b = target.BibitePart.GetMainBody()
    if b.bodyLength > mouthOpening || b.stomach == null: return false
    meatAmount = Meat.AmountOfEnergy(b.totalEnergy)               // E→u² of meat (÷10)
    if meatAmount + b.stomach.totalAmount > stomach.availableSpace: return false
    if b.bodyLength > mouthOpening/2:
        if biteProgress < bitePeriod: return false
        biteProgress -= bitePeriod
    for c in b.stomach.stomachContents: stomach.AddMatter(c.material, c.amount)  // transfer stomach contents
    b.stomach.nContent = 0
    stomach.AddMatter(Meat, meatAmount)
    b.Swallow()                                                   // prey dies; object destroyed (no meat pile)
    return true
```

So whole-body swallowing requires `prey.bodyLength ≤ mouthOpening` (head-to-tail fits through throat width!)
and enough room for prey's totalEnergy-as-meat plus prey's own stomach content. If the prey dies by swallowing
with `corpsesEnabled`, only the stomach contents + meat energy transfer; no corpse remains.

#### 5.3.5 Bite/chunk resolution (`MaybeAttack`, `:597-775`)

```text
if attackedThisFrame || desireToAttack <= 0: return
isPellet = tag "pellet"; isBibite = tag "bibitePart"; else return

h_m   = mouthOpening                 // pellet
F     = biteStrength
sigma = pellet.cohesiveness
r     = pellet.radius
if isBibite:
    other = target.BibitePart.GetMainBody(); if other == null || !other.born: return
    h_m = biteOpening
    if !ongoingAttack:
        F = max(biteStrength − other.armorStrength, 0)     // armor is pierced first (reduces bite force)
    r = other.internalsRadius
    h_m = h_m · (r / other.effectiveRadius)                // mouth opening scaled to internal body
    sigma = Meat.cohesiveness                              // 35 by default

if !ongoingAttack:
    if biteProgress < bitePeriod: return
    biteProgress -= bitePeriod                             // consume one bite period
else if isPellet:
    return                                                 // ongoing attacks never hit pellets

K   = h_m > 0 ? F/(h_m·sigma) : 0                          // num5
th  = asin( min( min(K,1)·h_m/(2r), 1 ) )                  // half-angle of phase-1 segment
A1  = r²·(th − sin(th)cos(th))                             // phase 1 always
A   = A1
if K > 1:
    # seed (exact) — a = 0.49·(ln(K−1) − 0.3125)
    a  = 0.49·(ln(K−1) − 0.3125)
    th = (π/2)·(1 − 1 / K^( exp(a/(16^a − 1)) ))
    repeat exactly 2 times:
        th -= (th − K·sin(th)cos(th)) / (1 − th·cos(th)/sin(th) + K·sin(th)²)
    rB = h_m/(2·sin(th))
    A += rB²·(th − sin(th)cos(th))
# A is the bite chunk area (u²) — no convergence test, no iteration limit besides exactly 2 steps

if isPellet:
    if desireToAttack > desireToSwallow:
        if pellet.RipChunkOff(A, normal = (mouth.pos − pellet.pos).normalized): return
    n = min(A, stomach.availableSpace)
    if pellet.amount − n < minPelletSize:
        n = pellet.RemoveAmount(pellet.amount)              // swallow the remainder whole
    else:
        n = pellet.RemoveAmount(n)
    stomach.AddMatter(pellet.material, n); return

# bibite branch
if !ongoingAttack: bibitesBitten += 1
attackedThisFrame = true
damage = bitingDamageFactor · other.maxHealth · A / other.realBodyArea
if biteStrength <= other.armorStrength: return              // armor holds: no damage at all
if ongoingAttack:
    damage *= (bitePeriod > 0 ? dt/(5·bitePeriod) : 0)      // 1/5 bite period worth, spread continuously
totalDamageDealt += damage
if instaKill: damage = other.health + 0.01                  // easter-egg "reaper"
energy = other.Attack(damage, drain = attackedLastFrame)    // = dealtDamage·HealthEnergyRatio
if other.dying:
    totalMurders += 1; murderedArea += other.baseBodyArea
    if totalMurders >= 50 && murderedArea >= 50·self.baseBodyArea: self.BecomeReaper()
# meat transfer: energy → meat amount
m = Matter{Meat, energy = energy}; mAmount = m.Amount (= energy/10)
if m.radius <= mouthOpening/2 && mAmount <= stomach.availableSpace:
    stomach.AddMatter(Meat, mAmount)
else:
    add = min(mAmount, stomach.availableSpace); stomach.AddMatter(Meat, add)
    if mAmount − add > minPelletSize:
        spawn meat pellet at midpoint(mouth.pos, target.pos) with amount (mAmount − add)
if !ongoingAttack && other.health > 0:
    OnAttack event
    if !target grabbed:
        target.AddForce( direction(this→target) · (throwingForceFactor·bitingThrowForceFactor·biteStrength),
                         ForceMode2D.Impulse )
```

Notes / fidelity quirks:
* `bitingDamageFactor` default 5; with `maxHealth ≈ 1.25·area` the formula is equivalent to
  `6.25·A/area·HP` per bite.
* `ongoingAttack` is **"this mouth attacked anything last tick"** (`attackedLastFrame`), not "is biting this
  same target". So successive bites on *different* targets are treated as one continuous attack if attacks
  happen every tick.
* Ongoing damage is applied per tick and does **not** consume `biteProgress`; the same bite continuously drains
  `damage·dt/(5·bitePeriod)` per tick. `attack` stops if target leaves the mouth trigger.
* Piercing rule: the armor subtraction (`F −= armorStrength`) happens only on the first bite. On ongoing ticks
  armor is not subtracted from `F` (chunk can be full size), but the final `biteStrength <= armorStrength`
  check still blocks damage. Both blocks are authoritative.
* Instant kill is only enabled by `BecomeReaper()`/`gene.isKiller` easter egg (`BibiteBody.cs:781-786`).
* The normal passed to `RipChunkOff` points **from pellet toward mouth**; `RipChunkOff` spawns the chunk on that
  side and moves the remaining pellet body slightly away (formula §6.1).

#### 5.3.6 Grab, release, throw, joint break [C]

```
TryGrabTarget(target):
    if nHeld >= 10: return
    rb = target.Rigidbody2D
    if already linked to rb: return
    target.Grabbable.TryGrab(this)                       // registers, fires onGrab(true) (pellet drag)
    j = mainBody.AddComponent<FixedJoint2D>()
    j.breakAction = 1; j.enableCollision = true; j.connectedBody = rb
    links[nHeld++] = j
```
Break force/torque are set every tick while grabbing: `jawStrength·max(Diet,0.1)·desireToGrab/nHeld`
(`:430-436`). The `max(Diet,0.1)` is the herbivore grab penalty (max 90 % penalty; `Diet 0` ⇒ ×0.1).

`ReleaseAndMaybeThrowAllHeldObjects(throwObjects)` (`:777-810`):
```
if !isStarted || nHeld < 1: return
totalHeldMass = Σ link.connectedBody.mass
num2 = throwingForceFactor·jawStrength·|desireToGrab|·totalHeldMass/(jawMass + totalHeldMass)
for each link:
    connected.Release(this); link.connectedBody = null; Destroy(link)
    if throwObjects && desireToGrab <= −0.25:
        connected.AddForce(self.transform.parent.up · num2/nHeld, Impulse)
if throwObjects && desireToGrab < −0.25:
    self.rb.AddForce(−self.transform.parent.up · num2, Impulse)     // recoil
nHeld = 0
```
(Forward direction is the **main body's** `up`, since the mouth object is a child.)
`OnLinkBroke` (`:489-508`): when a joint exceeds break force/torque, releases grabbable, destroys joint,
swap-compacts the `links` array, `nHeld--`.
Held pellet drag can be zeroed by the `disableDragOnHeldPellet` setting (`MatterPellet.cs:132`).

#### 5.3.7 Vomit (`:864-901`)

```
if stomach.isEmpty: return
n = |swallowOutput/0.95|                                  // fraction of each content to expel
for each content with amount > minPelletSize:
    amount = stomach.RemoveMatter(material, content.amount·n)
    spawn pellet of material/amount at mouth position      // pellets are NOT thrown yet
totalMass = Σ spawned.mass
num3 = throwingForceFactor·jawStrength·n·totalMass/(jawMass + totalMass)
for each spawned pellet: rigidbody.AddForce(parent.up · num3·pelletMass/totalMass, Impulse)
```
Requires `swallowOutput < −0.15` and a full bite period (checked in `UpdateOrgan`). Empty-stomach/no-content
case is a no-op.

### 5.4 Propulsion (`BibitePropulsion.cs`)

Constants (`InitOrgan`/`UpdateMuscleStrength`/`UpdateMuscleCosts`, `:154-189`), recomputed on growth:
```
muscleMoveStrength = armMusclePressure · (moveArea·metabolicRate)^(forwardForceSizePower/2)
muscleTurnStrength = armMusclePressure/4 · (moveArea·metabolicRate)^(turnForceSizePower/2)
baseAccelerationCost = moveMusclesCost · moveArea
baseTurnCost         = baseAccelerationCost/2
```
(defaults `armMusclePressure 50`, forward power 1.5, turn power 3, `moveMusclesCost 0.02`;
`moveArea = baseBodyArea·MoveMusclesWAG portion`.)

`UpdateOrgan` (`:191-319`):

```
1. totalTravel += |pos − lastPos|; lastPos = pos
   AngularVelocity = rb.angularVelocity/360
   SignedVelocity  = dot(rb.linearVelocity, transform.up)
   NormalizedVelocity = SignedVelocity/bodyLength
   Angle = Vector2.Angle(linearVelocity, +x); if cross(linearVelocity, +x).z > 0: Angle = 360−Angle
2. accel = brain[Accelerate]; turn = brain[Rotate]
   movementPenalty = sqrt(Health.fullness) · ageStrengthPenaltyFactor     // Health.fullness ∈[0,1] (see above)
3. herding (if fov.hasHerd && !disableHerding):
       H = brain[Herding]
       turn = turn·(1−|H|) + tanh(fov.herdSumDirection/90°)·H
       accel += gene[HerdVelocityWeight]·fov.herdSeparationProjection·H
4. void avoidance (if setting on): find nearest zone; if outside zone radius:
       t = min((dist−radius)/voidAvoidanceDistance, 1)
       turn = turn·(1−t) + tanh(SignedAngle(up, toZone))·t
5. shade avoidance / wrapping (if shadeOutsideOfBounds):
       sq = |pos|² ; out2 = simSize²·2.25
       if shadeAvoidance && sq >= out2:
           t = clamp01(InverseLerp(out2, out2·1.5, sq))
           turn = turn·(1−t) + tanh(SignedAngle(up, −pos))·t
       else if worldWrapping && |pos| − bodyLength >= simSize·1.5 + 1000:
           rb.position = −normalize(pos) · (simSize·1.5 + 1000 − bodyLength·0.95)
6. accel = clamp(accel,−1,1)·movementPenalty
   turn  = clamp(turn ,−1,1)·movementPenalty
   if accel < 0: accel *= backwardFraction                     // default 0.5
7. rb.AddForce(transform.up · accel·muscleMoveStrength)         // ForceMode2D.Force
   movementCost  = baseAccelerationCost·|accel|
   rb.AddTorque(turn·muscleTurnStrength)                        // ForceMode2D.Force
   movementCost += baseTurnCost·|turn|
8. body.UseEnergy(movementCost · dt)                           // divided by MetabolicEfficiency
```
`SignedVelocity`/`NormalizedVelocity`/`AngularVelocity`/`Angle` feed brain inputs `Speed`, `RotationSpeed`
and UI/debug. `movementCost` is a rate (E/s) used by `EnergyFlowRate`.

`disableHerding` removes the Herding output from the brain's processing structure entirely
(`NEATBrain.cs:515`) and makes `BibitePropulsion` ignore the herd contributions (`BibitePropulsion.cs:266`);
`FieldOfView` still computes `hasHerd`/`herdSumDirection` (its `herdingEnabled` static is assigned but never
read — dead code, `FieldOfView.cs:174-178`).

### 5.5 Fat organ (`BibiteFatOrgan.cs`)

State: `fatReserves: Matter{Fat}`, `threshold` gene, `deadband` gene, `inflateFactor`, `baseCapacity`,
`maxInflateAmount`, last-* diagnostics; constant `InflationAtMaxFactor = 0.35`.

Init/growth (`:73-88`):
```
fatReserves.Material = Fat
threshold = gene[FatStorageThreshold]; deadband = gene[FatStorageDeadband]
CheckInflateFactor(check=false)
OnGrowth:
    baseCapacity     = baseBodyArea · fatPortion
    maxInflateAmount = baseBodyArea · 0.35
    area             = baseCapacity · min(2, 1 + 0.35/fatPortion)
    fatReserves.MaxAmount = area
mass = fatReserves.mass   (= amount·Fat.MassDensity)
```
Note `area` inflation uses the **base** body area (not inflated area). If `fatPortion == 0`, `0.35/0` = ∞ →
`min(2, ∞) = 2`, area = 0 (and the organ is not registered on growth since portion = 0; its `UpdateOrgan` still
runs but all quantities are 0-ish; `baseCapacity = 0`, `fatReserves.MaxAmount = 0`). Guard division.

`UpdateOrgan` (`:90-116`) — exact:
```
ratio = Energy.Amount / Energy.MaxAmount
if |ratio−threshold| < deadband
   || (ratio > threshold && fatReserves.isFull)
   || (ratio < threshold && fatReserves.isEmpty):
       lastActivation = lastAmountDelta = lastAmountRate = lastEnergyGain = lastEnergyGainRate = 0
       lastAffinity = 1
       lastEnergySustainRate = fatReserves.energy·fatSustain
       body.UseEnergy(lastEnergySustainRate·dt)
       return
lastActivation = ratio > threshold
    ? InverseLerp(min(threshold+deadband, 0.99), 1, ratio)
    : −InverseLerp(threshold−deadband, 0, ratio)
if lastActivation > 0: lastActivation *= 1 − fatReserves.fullness^10      // storing gets harder when full
lastAmountDelta = lastActivation · d2Size · metabolicRate · (Fat.Reactivity·dt)² · π
lastAmountDelta = lastAmountDelta > 0 ? min(lastAmountDelta, fatReserves.available)
                                      : max(lastAmountDelta, −fatReserves.Amount)
lastAmountRate = lastAmountDelta/dt
lastAffinity   = 1 − sqrt(|lastActivation|)              // 1 at rest, →0 at full activation
lastEnergyGain = fatReserves.ConvertToFromEnergy(lastAmountDelta, lastAffinity)
lastEnergyGainRate = lastEnergyGain/dt
body.GainEnergy(lastEnergyGain)
lastEnergySustainRate = fatReserves.energy·fatSustain
body.UseEnergy(lastEnergySustainRate·dt)
CheckInflateFactor()                                    // may call body.UpdateInflateFactor(...)
```
`ConvertToFromEnergy` (`Matter.cs:52-57`) returns the **negative** energy for storing
(`−Δ·30/eff(affinity)`) and positive for burning (`−Δ·30·eff(affinity)`); so storing wastes energy
(efficiency < 1 means pay more than stored), burning loses some. When `Δ = 0`, it returns `−0·… = 0` (harmless).

Inflation (`:118-126`):
```
inflateFactor = InverseLerp(0, maxInflateAmount, max(fat−baseCapacity,0))
if !check || |inflateFactor − lastInflateFactor| >= 0.025:
    lastInflateFactor = inflateFactor; body.UpdateInflateFactor(inflateFactor)
```
`body.UpdateInflateFactor` changes capsule geometry, sprite frame and `armor.UpdateThickness()`
(`BibiteBody.cs:728-737`). This changes `realBodyArea` → drag/mass/inertia next tick (damping recomputed at
body tick end), and makes the bibite a bigger bite target (`effectiveRadius`).

### 5.6 Armor (`BibiteArmor.cs`)

```
OnGrowth / UpdateThickness (`:58-62`):
    armorThickness = 5·d1Size·(1 − sqrt(1 − organAreaPortion/(1+inflateFactor)))
    armorStrength  = armorThickness · Armor.cohesiveness
mass = area · armor massDensity
```
`BibiteBody.armorStrength = 2·armor.armorStrength` (`BibiteBody.cs:246`); **this doubled value** is what the
mouth compares against (`otherBody.armorStrength`). With one unit of `organAreaPortion` (100 % of body area) the
thickness would equal `5·d1Size` (half the body width); normal armor WAGs give thin shells. Armor also shrinks
the internal radius seen by bites through `internalsRadius`.
Armor has no per-tick energy cost in code; its cost is upfront via `BodyEnergyDensity` (armor area portion ×
Armor energy density 1.5 E/u²) and mass.

### 5.7 Egg layer / womb (`BibiteEggLayingOrgan.cs`)

State: `isMature`, `eggProgress` (eggs stored, fractional), `layingPeriod` (gene LayTime), `eggSize`
(`areaAtBirth`), `eggCapacity`, `eggEnergy` (E per egg), `eggProgressRate`, `energyConsumption` (E/s),
`nEggsLaid`, `children` (IDs), `nEgg = floor(eggProgress)`.
Constants: `EggProductionThreshold = 0.15`, `LayingThreshold = 0.25`, `EggReabsorptionEfficiency = 0.5`.
Persisted: `eggProgress`, `nEggsLaid`, `children[]` (world saves; see §9).

Init (`:104-111`):
```
eggEnergy   = gene.LayBodyEnergy + brain.brainUpkeepCost·gene[SizeRatio]
layingPeriod= gene[LayTime]
eggSize     = gene.areaAtBirth
OnGrowth(): eggCapacity = floor(area/eggSize)                  // area = baseBodyArea·wombPortion
```
(`eggSize` = 80·D2SizeAtBirth; `eggEnergy` uses `gene.LayBodyEnergy` from §1.3 plus **one egg's worth** of the
newborn's brain upkeep, scaled by `SizeRatio` gene, not `SpeedRatio`.)

`UpdateOrgan` (`:119-157`):
```
if !isMature: return
prod = brain[EggProduction]
if |prod| < 0.15: energyConsumption = 0; eggProgressRate = 0; return
progressRemap = ThresholdRemap(prod, 0.15)        // sign-preserving remap |x| from [0.15,1] to [0,1]
Δ = metabolicRate · progressRemap · dt / layingPeriod
Δ = Δ >= 0 ? min(Δ, eggCapacity − eggProgress) : max(Δ, −eggProgress)
if Δ >= 0: body.UseEnergy(Δ·eggEnergy)                      // pay as you build
else:      body.GainEnergy(|Δ·eggEnergy·0.5|)               // reabsorb at 50 %
eggProgress += Δ
eggProgressRate = Δ/dt
energyConsumption = eggProgressRate·eggEnergy
if eggProgress < 1 || brain[Want2Lay] < 0.25: return
limit = limitBibiteBirth ? bibiteLimit : +∞
if bibiteHolder.childCount >= limit: return                 // counts bibites + eggs (all children of holder)
while eggProgress >= 1: LayEgg(); eggProgress -= 1
```
`ThresholdRemap` (`Here/ScriptHelpers/MathfE.cs:14-17`):
`sign(x)·Remap(|x|, threshold, 1, 0, 1)`.

`LayEgg` (`:159-191`):
```
pos = transform.position − transform.up·(8·d1Size)          // behind the bibite
eggGO = WorldObjectsSpawner.GenerateNewEgg(pos)
egg.genes.CopyGene(self.genes, increaseGeneration=true, mutate=true)   // genome spec 01 owns mutation
egg.genes.SetParents(self)
egg.brain.CopyBrain(self.brain.Nodes, self.brain.Synapses, mutate=true)
egg.birthOrientation = −transform.up
egg.StartHatch(eggEnergy)                                   // egg energy budget
children.Add(egg.id); egg.onHatch += OnChildrenEggHatch; nEggsLaid += 1
```
`OnDeathStatusChange(false)` (`:215-231`) lays all complete eggs, subject to the same global limit (i.e. a
dying parent still reproduces). `isMature` is set by `BibiteGrowth` (never unset).

### 5.8 Egg hatching (`EggHatching.cs`)

Init (`:81-106,154-157`):
```
hatchTime = gene[HatchTime]
totalRequiredEnergy = gene.BodyEnergyAtBirth + brain.brainUpkeepCost·gene[SizeRatio]
sizeGene  = gene.D1SizeAtBirth
eggArea   = 5·sizeGene²                          // separate constant from body area!
eggMass   = gene.areaAtBirth · bibiteMassDensity
transform.localScale = sizeGene
rb.mass = eggMass; rb.linearDamping = dragCoefficient·sizeGene/rb.mass
collider.isTrigger = !eggCollision setting
```
`StartHatch(hatchEnergy?)` (`:108-134`): if `hatchTime <= 0` **or** `totalRequiredEnergy > 2·energy` → `Abort()`;
else assign random `bibiteID` (1..int.Max), species/tag, tracker, `hatchProgress = 0`, `isHatching = true`.
Template-spawned eggs set `energy = LayBodyEnergy` *before* calling `StartHatch()` (`WorldObjectsSpawner.cs:220-222`).

`FixedUpdate` (`:159-173`):
```
if !isHatching: return
hatchProgress += dt
if HatchRatio()·totalRequiredEnergy > energy: Abort()        // ran out of budget for remaining build
else if hatchProgress >= hatchTime: Hatch()
HatchRatio() = min(hatchProgress/hatchTime, 1)
```
`Hatch` (`:175-207`):
```
if totalRequiredEnergy > energy: Abort(); return
newBorn = WorldObjectsSpawner.GenerateNewBibite()
newBorn.position = egg.position
newBorn.up = |birthOrientation| > 0.01 ? normalize(birthOrientation) : random rotation Rand(1,360)
copy genes (no mutation), copy brain (no mutation), copy parents
newBorn.Energy.Amount = energy
newBorn.id.Set(egg.id)            // then StartBody()'s FinalizeBirth assigns non-zero if 0
newBorn.StartBody()
energy = 0; onHatch(event); Destroy(egg)
```
`Abort` (`:209-228`): mark aborting, fire `onHatch(this, null)` (parent removes child id), spawn meat pile of
remaining `energy` at egg position with radius `egg.localScale.x/2`, remove from species/tags, destroy.
`ResumeHatch` (`:136-152`): load path; re-`InitEgg` (recomputes derived constants/mass/drag), abort if gene or
brain not ready, re-register species/tag/tracker, `isHatching = true`.

### 5.9 Pheromone organ (`BibitePheromoneOrgan.cs`)

```
progress += dt; if progress < 1: return; progress -= 1                 // 1 Hz emission
r = preventRed   ? 0 : brain[PhereOut1]·pheromoneProductionStrength
g = preventGreen ? 0 : brain[PhereOut2]·pheromoneProductionStrength
b = preventBlue  ? 0 : brain[PhereOut3]·pheromoneProductionStrength
if r+g+b > 0.05:
    spot = WorldObjectsSpawner.GenerateNewPheromonesSource(transform.position)
    spot.InitPheromones(r,g,b, transform.up)
    pheromoneCost = (r+g+b)·pheromoneProductionCost
    body.UseEnergy(pheromoneCost)                                      // per emission ⇒ effectively E/s at 1 Hz
```
`PheromoneSpot` (`PheromoneSpot.cs`): each channel strength decays `0.25·strength` per 0.25 s (=1/s, so a
strength of 10 lasts ~10 s); heading = emitter's up at emission time; spot destroyed when all three ≤ 0 and
`dissipate`; particles are visual only. A spot is a `Collider2D` (trigger) on the pheromone layer read by
`Pherosense`.

### 5.10 Internal clock (`InternalClock.cs`)

See §3.5. Fields persisted: `tic`, `ticProgress`, `timeAlive`, `chronoTime`. `period` comes from gene
`ClockSpeed` in a coroutine that waits for the brain to be ready (`waitReady`, `:99-107`) — i.e. the clock only
starts counting after brain init. `counting` is not persisted; `ResumeClock` restarts the coroutine.
`timeAlive >= 86400` fires an achievement.

### 5.11 Pherosense (adjacent; spec 03 owns senses)

`Pherosense.PherosenseAround` (`:70-215`) runs every body tick **after** organs (manual call, not a
`FixedUpdate`) when `usePheroSense` was enabled by the brain at init (`StartPherosensing`,
`Pherosense.cs:57-68`; radius = gene `PheroSense`, cadence random phase 0.5 s). It sums each channel as
`Σ strength/distance` over `Physics2D.OverlapCircleNonAlloc` hits in `pheroDist`, builds weighted direction
vectors, and outputs `pheroSum{1,2,3}` and signed angles (`/180`) for brain inputs 24-32. Energy cost: none.
Red Death adds an artificial red pheromone source near the bloom edge (`:196-207`).

### 5.12 FieldOfView coupling to the body

Only body-owned pieces listed here (full sensing spec = `docs/spec/03-world-ecology.md`):
* FOV `viewRadius = gene[ViewRadius]·d1Size` and `herdSeparationDistance = gene[HerdSeparationDistance]·d1Size`
  are pushed by `Set2DSize` (`BibiteBody.cs:722-723`).
* Other bibites are detected using `sideRadius = 4.5·d1Size` as their object radius (`FieldOfView.cs:450,463`).
* Herd outputs used by propulsion: `hasHerd`, `herdSumDirection` (signed degrees to combined separation/cohesion/
  alignment vector), `herdSeparationProjection = cos(deg2rad·angle(up, herdSeparationVector))`
  (`FieldOfView.cs:506-520`).
* FOV layer mask is the prefab mask plus `"pellets"` and/or `"bibite"` layers depending on connected brain
  inputs (`:181-208`). Cone box/circle query geometry `:238-250`.
* The FOV object lists (`seenBibites`, `seenPlantPellets`, `seenMeatPellets`, `seenCorpses` and their weights)
  are not persisted.

---

## 6. Feeding and combat resolution

### 6.1 Bibite ↔ pellet

Three different interactions in descending priority:

1. **Whole-pellet swallow** — pellet within trigger, `radius ≤ swallowWidth/2`, `amount ≤
   stomach.availableSpace`; medium pellets (`radius > swallowWidth/4`) additionally require a full bite period
   (§5.3.4). Energy = `amount·material.EnergyDensity` on digestion (not on swallow; `AddMatter` stores amount).
2. **Filter-feeding of tiny pellets**: `maxAmountNoBiteNeeded = π(swallowWidth/4)²` is the "no bite needed"
   area; implemented as the `radius > mouthOpening/4` gate above.
3. **Biting chunks** — `Want2Attack > 0.15` and `Want2Attack > Want2Eat`; chunk area `A` from §5.3.5; the
   chunk is spawned as a *new pellet* in front of the mouth via `MatterPellet.RipChunkOff`
   (`MatterPellet.cs:186-221`):
```
chunk = min(chunkRequested, pellet.amount)
if chunk < minPelletSize || pellet.amount − chunk < minPelletSize: return false   // don't split below min
pellet.RemoveAmount(chunk)                                  // reduces and possibly destroys source pellet
num = sqrt(chunk)/π                                          // NOTE: sqrt(A)/π, not sqrt(A/π)
offset = normal · (num + pellet.radius)
newCenter = pellet.pos − offset·chunk/(chunk+remainingAmount)      // conserve center of mass
spawn new pellet(material, amount=chunk) at newCenter + offset
pellet.rb.position = newCenter
```
The ripped chunk is a free pellet (not auto-swallowed). Only `minPelletSize` (0.1 u²) clamps the chunk size.
It is also possible for `A = 0` (e.g. mouth closed) and then the swallow branch below runs with `n = 0`
(no effect except tiny-pellet remainder swallow).
Energy cost: none directly; mouth/bite has no energy cost besides the jaw's construction/metabolism.

Pellet physics (`MatterPellet.cs`): `sizeFactor = amount/20`; `transform.localScale = √sizeFactor`;
`radiusFactor = √sizeFactor`; `mass = amount·MassDensity`; `linearDamping = dragCoefficient·radiusFactor/mass`
(unless held + `disableDragOnHeldPellet`); `angularDamping = dragCoefficient/2` set in `Awake`;
`freezeRotation = !pelletRotation` (rotation zeroed when disabled, `SettingsUpdater.cs:134-145`); solid collider
iff `pelletCollision` else trigger (`MatterPellet.cs:223-236`); merge on collision only when `pelletMerge` and
`other.amount <= amount` and same material (`:238-258`, momentum-conserving merge).
Meat decay: after `freshTime` (2000 s) it loses `decayRate` (0.02 u²/s), processed every `decayTPS` ticks;
removed when decayed amount/amount > 0.05 (`MatterDecayOfArrays.Process`, `:71-82`).

### 6.2 Bibite ↔ bibite (predation)

Condensed order of a successful first bite (implemented in `MaybeAttack`, §5.3.5):

1. Mouth trigger had the victim's `bibitePart` collider inside it.
2. `h_m` scaled by `internalsRadius/effectiveRadius`, `F` reduced by victim's double armor strength, `r` =
   `internalsRadius`, σ = Meat cohesiveness.
3. Bite cost: one `bitePeriod` of jaw cooldown.
4. Chunk area `A` (u²) from the circular-segment model; `biteArea/realBodyArea` is the fraction of the victim's
   body cross-section removed.
5. Damage `= 5·victim.maxHealth·A/realBodyArea` (`bitingDamageFactor`) — applied through `Attack` → `Hurting`.
6. If `biteStrength ≤ victim.armorStrength`: **no damage** (after computing chunk size).
7. Biter gains flesh energy `= dealtDamage·victim.gene.HealthEnergyRatio`; converted to Meat amount
   (`energy/10`) and put in the biter's stomach if it fits; overflow becomes a meat pellet at the midpoint.
8. Victim gets a knockback impulse `= 0.25·0.25·biteStrength` (defaults) if not grabbed and still alive.
9. If victim dies (`dying`), biter counts a murder; ≥50 murders and total murdered area ≥ 50·own baseBodyArea
   unlocks the "reaper" easter egg (insta-kill bites, visual skin flag).
10. Continuous attack: while `attackedLastFrame` is true and the target stays in the mouth, each tick applies
    `damage = 5·maxHP·A/realBodyArea · dt/(5·bitePeriod)`
    (`BibiteMouth.cs:720-723`) — i.e. one base-bite worth of damage every 5 bite periods.

Corpses are bibites with `dead=true`; their `bibitePart` colliders still work, so they are biteable, their
`totalEnergy` is reduced by `corpseEnergyOffset`, and `TrySwallowTarget` can swallow a corpse whole (no corpse
remains; its stomach content + meat value transfer). Damage to a corpse still calls `Attack`/`Hurting`; if
health < 0 while dead → `ExplodeToMeat` (bursts into its meat pile at that moment).

### 6.3 Collision damage (`BibitePart.cs:47-66`)

For every non-trigger body-part collider on collision enter, if `collisionDamageConstant != 0`:

```
mainBody.onBodyCollided.Invoke(other.gameObject)                     // event for audio/visual/tools
contact = other.GetContact(0)
vn = dot(contact.normal, other.relativeVelocity)                     // scalar closing speed (sign in BQ-4)
mOther = other.otherRigidbody.mass
impulseSpeed = 2·mOther·vn/(mOther + mainBody.mass)                  // reduced-mass-normalised Δv of this body
if impulseSpeed >= collisionDamageThreshold:                         // default 50 u/s
    dmg = (impulseSpeed − collisionDamageThreshold)·collisionDamageConstant   // default 0.1
    mainBody.Hurting(dmg)                                            // does NOT add attackedDmg
```
So ramming damage scales with mass and closing speed above 50 u/s; tiny bibites bouncing off big ones take
little damage (mass ratio), big ones ramming small ones take more. Bodies with `partCollider.isTrigger` never
deal or take collision damage through this path; pellets/eggs become triggers when their collision settings are
off, so they stop causing collision damage.

### 6.4 Grab/throw force summary (`BibiteMouth.cs`)

| Action | Force | Mode | Source |
|---|---|---|---|
| grab joint break strength | `jawStrength·max(Diet,0.1)·desireToGrab/nHeld` on both `breakForce` & `breakTorque` | joint | `:430-436` |
| throw held object | `throwingForceFactor·jawStrength·|desireToGrab|·M/(jawMass+M)/nHeld` per object, along main body `up` | Impulse | `:791-804` |
| release recoil | same magnitude along `−up` on self, if `desireToGrab < −0.25` | Impulse | `:805-808` |
| bite knockback | `throwingForceFactor·bitingThrowForceFactor·biteStrength` along bite direction | Impulse | `:766-773` |
| vomit throw | `throwingForceFactor·jawStrength·vomitFraction·M_total/(jawMass+M_total)` distributed by pellet mass | Impulse | `:895-900` |

Defaults: `throwingForceFactor 0.25`, `bitingThrowForceFactor 0.25`, `jawMass = jawArea·density/2`.
Held objects count ≤ 10; `nHeld` also feeds brain input `IsGrabbing` (raw count, `NEATBrain.cs:582`).

### 6.5 Combat/feeding bookkeeping

`BibiteMouth` serialized state: `attackedLastFrame`, `biteProgress`, `bibitesBitten`, `totalDamageDealt`,
`totalMurders`, `murderedArea`. `bibitesBitten` increments per first bite; `totalDamageDealt` sums applied damage: the
code order is `damage = …` → `if biteStrength <= armorStrength: return` → `if ongoingAttack: damage *= …` →
`totalDamageDealt += damage` (`BibiteMouth.cs:715-724`), so an armor-blocked bite increments
`bibitesBitten`/sets `attackedThisFrame` but **not** `totalDamageDealt` and not the victim's
`health`/`attackedDmg`/`totalDamageSuffered`.
`timesAttacked`/`totalDamageSuffered` live on the victim (`BibiteBody`), incremented in `Attack` only (bites),
not collisions.

---

## 7. Reproduction: egg production, laying, hatching, gene flow

### 7.1 Production

See §5.7. Key numbers: progress rate `Δ = SpeedRatio·remap(EggProductionOutput)·dt/LayTime`; each whole egg
costs `eggEnergy = LayBodyEnergy + brainUpkeep·SizeRatio`; capped by `floor(wombArea/areaAtBirth)`; negative
output reabsorbs at 50 % energy (`EggReabsorptionEfficiency = 0.5`). Maturity gate (`isMature`) comes from
growth (§3.4).

### 7.2 Laying

`Want2Lay ≥ 0.25` and `eggProgress ≥ 1` ⇒ while ≥1 lay one egg (subject to `limitBibiteBirth`/`bibiteLimit`
counting `bibiteHolder.childCount`, which includes live bibites + eggs). Egg is placed `8·d1Size` behind
(`−transform.up`) the parent. On death, complete eggs are laid too.

### 7.3 Gene & brain flow (body-side boundary; full mutation algorithm = spec 01)

At laying (`BibiteEggLayingOrgan.cs:174-183`):
1. `egg.genes.CopyGene(parent.genes, increaseGeneration=true, mutate=true)`
   * copies all 35 genes, generation+1, copies `species`, `speciesTag`, `mutationIncreases`, `deuxParent`,
     `isMutant`;
   * each of `isRad/isRain/isKiller/isSource` inherited with probability 0.1;
   * then mutation is applied (Poisson count `AverageMutationNumber + backgroundMutationChance +
     radiation.probabilityIncrease`, per-gene mutation with `σ = MutationAmountSigma + backgroundMutationVariance
     + radiation.intensityIncrease`, clamped to the gene's editor range — `BibiteGenes.cs:287-334,364-416`).
2. `egg.genes.SetParents(parent.gameObject)`.
3. `egg.brain.CopyBrain(parent.Nodes, parent.Synapses, mutate=true)` (brain mutation uses
   `BrainAverageMutation`/`BrainMutationSigma`; `NEATBrain.cs:373-426`).
4. `egg.StartHatch(eggEnergy)`.

At hatching (`EggHatching.cs:195-203`): genes copied **without** mutation, parents forwarded from the egg,
brain copied **without** mutation, energy = egg energy, then `StartBody()`. So all mutation happens at laying.

`BibiteGenes.CopyGene2` (two-parent uniform crossover) exists (`:336-362`) but has **no call sites** in `Here/` —
reproduction is asexual; ignore unless a future version wires it up.

**Determinism caveat:** mutation values/counts in the game use unseeded `System.Random` instances
(`BibiteGenes.cs:91` `new Random()`, `NEATBrain.cs:249` `new Random()`), while gene/operation *selection* uses
`UnityEngine.Random`. The original game therefore cannot reproduce a mutation sequence exactly across runs.
Our engine will use the session `Rng` (§10.5); exact mutation-for-mutation parity is impossible by construction —
compare distributions/statistics (must be recorded as an accepted fidelity deviation).

### 7.4 Birth limits and IDs

`limitBibiteBirth`/`bibiteLimit` (default off/100) are consulted by both egg laying and virgin spawning.
`BibiteID.CheckGetNew` assigns a random non-zero `int` (`Random.Range(int.MinValue,int.MaxValue)` in a loop);
eggs get `bibiteID = Random.Range(1,int.MaxValue)` at `StartHatch` (a separate legacy field), and the hatched
bibite inherits `egg.id` (usually 0 → then `CheckGetNew` gives it a random id in `FinalizeBirth`).

---

## 8. Death, corpses, scavenging, revival

### 8.1 `Die(swallowed=false)` (`BibiteBody.cs:788-833`)

```
dying = true
if dead: return                                   // already a corpse
tracker.BibiteDeath(this)
remove from species/tags
dead = true
onDeathStatusChange.Invoke(this)                  // organs disable; mouth drops held; egg organ lays eggs
if !corpsesEnabled:
    ExplodeToMeat()
else if born:
    if swallowed:
        eggLayer.eggProgress = 0
        Destroy(gameObject)                       // no meat pile
        return
    growth.enabled = false; clock.enabled = false // stops their FixedUpdate
    corpseEnergyOffset = Health.Material.EnergyOfAmount(Health.MaxAmount) − Energy.Amount
    Energy.Amount = 0
    Health.Amount = Health.MaxAmount
    UpdateMetabolism()
```
Note when `corpsesEnabled == false`, `ExplodeToMeat()` runs while `dead == true` but `corpseEnergyOffset == 0`,
so its meat pile value is `SumBodyAndHealth·area + energy + eggs + fat` (health energy included). A **swallowed**
prey under `corpsesEnabled == false` also bursts into meat, so the biter gets meat transfer *and* leaves a meat
pellet (double-dip; preserve).

### 8.2 Corpse tick (`CorpseUpdate`, `:641-648`)

```
dying = false
UseEnergy(metabolism·corpseDecayFactor·dt)        // energy is 0 ⇒ the whole cost becomes health damage
damagedFlash = max(damagedFlash, pow(1−healthRatio, 8))       // visual
corpseDecay = clamp01(InverseLerp(1, 0.5, healthRatio))       // visual (0 at full health, 1 at ≤50%)
sprite.setCorpseDecay(corpseDecay)
```
Corpse lifetime is therefore `maxHealth / (metabolism·corpseDecayFactor)` in "health-energy units" — exact
seconds require the `Health` material energy density (BQ-1), i.e. `maxHealth·HealthED/(metabolism·decayFactor)`.
Attacks/collision damage speed decay by reducing health. When health crosses 0 during the decay `UseEnergy` →
`Hurting` → `ExplodeToMeat`. `corpseDecayFactor` default 1.0.

### 8.3 `ExplodeToMeat` (`:847-863`)

```
if destroyed: return
blood particles (visual)
WorldObjectsSpawner.SpawnMeatPile(totalEnergy, transform.position, bodyLength/2)
for each organ: OnBodyDestroyed()      // stomach spills content; mouth drops held
Destroy(gameObject)
```
`SpawnMeatPile` (`WorldObjectsSpawner.cs:300-336`) with `totalEnergy`, `pos`, `radius`:
```
num = 1
if num >= 2: ... (dead code in current source)
else if totalEnergy >= Meat.EnergyDensity·minPelletSize (=10·0.1=1 E):
    radius *= 0.1
    pos.x += Rand(−radius, radius); pos.y += Rand(−radius, radius)
    SpawnMeatPellet(pos, energy=totalEnergy)
```
So **one** meat pellet carrying the whole corpse/burst energy (up to tens of thousands of E in one pellet)
jittered within `0.1·bodyLength/2` of the death position. `MatterPellet` scale/radius derive from energy:
`amount = energy/10`, `radius=√(amount/π)`.
`totalEnergy` on a dead body subtracts `corpseEnergyOffset` (see §1.3 formula); with the health-energy bookkeeping
the corpse's meat value is `SumBodyAndHealth·area − HealthED·maxHealth + energyBeforeDeath + storedEggs + fat`.
`OnBodyDestroyed` also invokes `BibiteMouth.OnBodyDestroyed`? No: mouth releases held objects in *both*
`OnDestroy` and `OnDeathStatusChange(false)` (`BibiteMouth.cs:510-536`); the organs list call at explosion also
releases.

### 8.4 `Revive` (`:865-903`)

Developer/edge path: `dead=false`, re-enable growth/clock, `corpseEnergyOffset=0`, tracker/species/tags,
`onDeathStatusChange`; then a coroutine sets `health = maxHealth·(1−corpseDecay/1.1)`, `Energy=max`, and decreases
`corpseDecay` by `1/150` per 0.1 s until 0 (`BibiteBody.cs:888-903`): 6.67 %/s, ~15 s total, then full
health/energy. No in-game trigger found in `Here/` UI paths — treat as unused for milestone 1 unless a save
contains a revived entity.

### 8.5 `OnDestroy` cleanup (`:905-949`)

Destroys `brain`, `fow`, `pheromoneOrgan` components; removes from species/tags; `tracker.BibiteDeath` (guarded —
possible double count if `Die` already removed); fires `onDestroyed`; unsubscribes settings. Any held/grabbed
links are released by those components' own OnDestroy. For clean simulation semantics our entity removal must
close grabs, remove from arena and trigger stats once only.

### 8.6 Bookkeeping

`BibiteTracker` keeps `bibites` list, `eggs` list, birth/death counters and a death-age quantile estimator;
`BibiteBirth` increments `nBirth`, `BibiteDeath` removes (if present) and increments `nDeath` + records age.
Biomass stats use `Σ bibite.totalEnergy + Σ egg.energy` (`BibiteTracker.cs:54-57,159-178`).

---

## 9. Persistence

### 9.1 Save envelope

World save: each live entity is serialized by `SaveSystem.SerializeBibite` (`SaveSystem.cs:426-445`) into
`bibites/bibite_<n>.bb8`:
```json
{ "transform": {...}, "rb2d": {"px","py","vx","vy","r"}, "genes": {...}, "body": {...}, "clock": {...}, "brain": {...} }
```
Eggs: `{transform, rb2d, genes, egg:{...}, brain}` (`SaveSystem.cs:447-465`).
`SerializationHelper.SerializeObject` dispatches: `ISaveable` → its `SaveState()`; collection → item-wise;
otherwise reflection over `[SerializeField]` fields (`SerializationHelper.cs:23-89`). This is why:
* `genes` uses `BibiteGenes.SaveState()` (named gene map),
* `body` uses `BibiteBody.SaveState()`,
* `clock` uses reflection (`tic`, `ticProgress`, `timeAlive`, `chronoTime`),
* `brain` uses reflection (`isReady`, `Nodes`, `Synapses` arrays; `Node` emits its public fields plus the
  computed `archetype` property, while `NIn`/`NOut` are `[NonSerialized]`; `Synaps` emits
  `Inov,NodeIn,NodeOut,Weight,En` — confirmed against a real `0.6.3.1` `.bb8`),
* `egg` uses `EggHatching.SaveState()` (`hatchProgress`, `energy`, `id` only — `bibiteID` excluded because
  `SaveForTemplate`/ISaveable path wins).
`transform` is `SerializedTransform {position:[x,y], rotation, scale}` (older saves may use
`OldSerializedTransform`; `SerializationHelper.DeserializePosition` has a fallback, `:231-258`).
A single saved bibite `.bb8` is the same JSON without the outer world bundle.

### 9.2 `BibiteBody` schema (world save / `.bb8`)

Exact schema from `SaveState` + real `0.6.3.1` sample:

| JSON key | Type | Field | Reconstructed on load from |
|---|---|---|---|
| `mouth` | object | §9.3 | — |
| `stomach` | object | §9.3 | — |
| `phero` | object | `{progress}` | — |
| `health` | number | `Health.Amount` HP | `maxHealth` from `d2Size`; excess clamped |
| `energy` | number | `Energy.Amount` E | `MaxAmount` from `d2Size` |
| `eggLayer` | object | §9.3 | — |
| `control` | object | `{totalTravel}` | — |
| `fatReservesAmount` | number | fat u² | `inflateFactor` recomputed |
| `id` | int | bibite id | fallback `bibiteID` for older saves |
| `d2Size` | number | size state | `d1Size`, areas, mass, caps, metabolism |
| `dying` | bool | | |
| `dead` | bool | | |
| `attackedDmg` | number | | |
| `timesAttacked` | int | | |
| `totalDamageSuffered` | number | | |
| `brainTicksCount` | number | | |
| `visionLookupCount` | number | | |
| `visionSensingCount` | number | | |
| `corpseEnergyOffset` | number | | |

**Not saved / rebuilt:** `born` (set true in `FinalizeBirth`), `growth.growth` (computed in `LoadState` line 1031:
`growth = d2Size/(SizeRatio²)`), `metabolicEfficiency`, `metabolicRate`, `baseEnergyDensity`, `metabolism`
(`UpdateMetabolism` via `Set2DSize`), `maxHealth`, `Energy.MaxAmount`, organ areas/derived constants, inflate
factor, sprite, `d1Size`, `baseMass`, fov radii, grab links (world saves store joints separately in
`scene.bb8scene.connections`; single `.bb8` does not save grabs at all).

### 9.3 Organ schemas

| Organ | JSON | Source |
|---|---|---|
| mouth | `{attackedLastFrame, biteProgress, bibitesBitten, totalDamageDealt, totalMurders, murderedArea}` | `BibiteMouth.cs:924-971` |
| stomach | `{content:[{material, amount, averageChunkAmount}, ...]}` (nContent entries) | `BibiteStomach.cs:349-376`, `StomachContent.cs:49-71` |
| eggLayer | `{eggProgress, nEggsLaid, children:[int...]}` (`children` omitted if empty) | `BibiteEggLayingOrgan.cs:238-266` |
| phero | `{progress}` | `BibitePheromoneOrgan.cs:91-105` |
| control (move) | `{totalTravel}` | `BibitePropulsion.cs:332-346` |
| egg | `{hatchProgress, energy, id}` | `EggHatching.cs:235-264` |
| clock | `{tic, ticProgress, timeAlive, chronoTime}` | reflection over `InternalClock` |
| brain | `{isReady, Nodes[...], Synapses[...]}` | reflection over `NEATBrain` |

`children`: on load `childIDs` are read (`BibiteEggLayingOrgan.LoadState`) and `SaveSystem.LoadScene` later maps
IDs back to `BibiteID` components and repopulates `children` (`SaveSystem.cs:815-868`). For a single `.bb8`
snapshot the children list is dropped on load (no mapping step).

### 9.4 Load order for a bibite (`SaveSystem.LoadBibite`, `:467-505`)

```
instantiate bibite prefab (Awake runs: rb2d refs + angularDamping)
read transform + rb2d
genes.LoadState → then BibiteUpdater.UpdateGenesToPresentVersion(from file version)
body.LoadState:
    for each key → set matching [SerializeField] field
    mouth/stomach/phero/eggLayer/control/fat/id/health/energy loaded
    growth.growth = d2Size / SizeRatio²
clock.LoadState (reflection), brain.LoadState (reflection) + BrainUpdater migration
brain.ResumeBrain()
if birthAtMaturity given: body.StartBodyAtGrowthAndNormalize(m)
else: body.ResumeBody()          // Set2DSize (recompute caps), health overflow clamp, growth resume,
                                 // clock resume (or stop if dead), FinalizeBirth (organs init, born=true,
                                 // species/tags/tracker), corpse update if dead
```
`ResumeBody` ordering matters: `Set2DSize` must run **before** `FinalizeBirth` (organ init uses `area`), and
after body load (energy/health already loaded). `Hurting(Health.overflow)` may damage a loaded bibite whose
size shrank relative to the saved health; if that kills it, it dies immediately at load.
`Start()` (deferred Unity callback) then calls `Die()` if `dying` was true.

### 9.5 Version migrations relevant to body/organs

| Version gate | Migration | Source |
|---|---|---|
| `< 0.6a7` | new gene slots: `EyeOffset`, `StomachWAG`, `WombWAG`, `FatWAG`, `FatStorageThreshold`, `FatStorageDeadband` set to editor defaults | `BibiteUpdater.cs:31-41` |
| `< 0.6a15` | gene remap via `to06a15GenesMap` (34→35 old slots; drops old `Strength`/`Defence` and shifts later genes into the modern enum positions), **then** WAG rebuild: `array[26] *= 2.5` (StomachWAG), `array[27] *= 2.5` (WombWAG), `array[28] *= 2.5` (FatWAG); `array[29] = array[31] = array[32] = array[30] = 0`; `W = Σ_{26..32} max(0,·)`; `strength = min(0.01, (oldStrength/35)²)`; `defence = (oldDefence/35)²`; `rest = 1 − strength − defence − 0.05 − 0.1`; then `MouthMusclesWAG[31] = W·strength/rest`, `ArmorWAG[29] = W·defence/rest`, `MoveMusclesWAG[32] = W·0.05/rest`, `ThroatWAG[30] = W·0.10/rest`; old `Strength`/`Defence` genes are gone | `BibiteUpdater.cs:42-101` |
| `< 0.6.0.6` | zone fertility/biomass/pelletSize/speed were log10 and renormalised by area | `SerializationHelper.cs:434-470` |
| `< 0.6.0.7` | `bibiteMassDensity / 1.9`; `pelletGrowth × 0.7`; `pelletEnergy × 1.5` | `SerializationHelper.cs:491-505` |
| `< 0.6.0.9` | Fat material min/max efficiency ÷ 100 if > 1.0 (they were saved as %) | `SerializationHelper.cs:410-422` |
| `< 0.6.0.13` | neuron mutation chance defaults reset (brain sphere) | `SerializationHelper.cs:423-427` |
| `< 0.6.0.16` | all material `reactivity /= 30`, then Fat reactivity overwritten to `sqrt(oldFat)·π`; note code divides all materials including Fat, then sets Fat from the *pre-division* value | `SerializationHelper.cs:386-400` |
| `< 0.6.0.17` | `bitePeriodFactor /= 35` if it is > 10 (old larger value domain) | `SerializationHelper.cs:506-509` |
| old body keys | `body.born`, `body.totalEnergy`, `body.wasteBank` ignored by modern `LoadState`; missing `dead/timesAttacked/...` default to C# defaults | `BibiteBody.cs:977-1032` |
| old id key | `bibiteID` accepted as fallback for `id` | `BibiteBody.cs:1021-1028` |
| old `matterDecay`/`meatRot` | pellet decay state load fallback | `SaveSystem.cs:541-555` |

Can update from as old as `0.4.2.1` (`VersionTracker` list; `0.4` and `0.3` entries are
`canUpdateFromOlderVersion=false` → loaders refuse older files, `VersionTracker.cs:9-45`).

### 9.6 Round-trip requirements/traps (for `bibites-io`)

* Preserve unknown keys; the game's loaders ignore unknown fields but re-saving drops them.
* Write all `[SerializeField]` body fields even at default (`SaveState` writes every non-null field; value types
  always emitted).
* `transform.scale` must be `d1Size` — it is not derived from `d2Size` on load (`SerializedTransform` loads
  directly; `ResumeBody → Set2DSize` does rescale only if `updateScale` default true — it does pass default
  `initialSet:true, updateScale:true`, so scale is fixed anyway; still write it correctly).
* `rb2d` block is velocity/rotation/position authoritative; body position lives in `transform`, `rb2d.px/py`
  normally equals it.
* Do not write derived fields (e.g. the old-file keys `born`, `totalEnergy`, `wasteBank`) — old files have them,
  the modern game ignores them; for byte-identical round-trip keep them as unknown preserved tokens.
* Gene map keys are enum names (`BibiteGenes.Genes`); missing keys must be backfilled from
  `BibiteEditorSettings` defaults on load (`BibiteGenes.cs:607-619`).
* Brain node order is `[33 inputs][15 outputs][hidden]`; output nodes are identified by `Index`, not list
  position, in modern saves (both coincide for well-formed files), and `Desc`/`TypeName` are informational.
* `mouth.phys` joints (`scene.bb8scene.connections`) restore grabs; a per-bibite `.bb8` loses them.
* Egg `id` is the egg's `BibiteID.id` (0 for fresh eggs) — not the same as `EggHatching.bibiteID`.

---

## 10. Rust design notes

### 10.1 Suggested module layout (within allowed scope: `crates/bibites-core/src/body.rs`, `organs.rs` (may
become `organs/` tree), `physics.rs`)

```
body.rs        BodyStatic (per-size derived), BodyState (energy/health/growth/corpse), size/growth/ageing,
               damage & death, save/load conversion structs, body↔brain scalar glue
organs/mod.rs  OrganKinematics table + shared organ constants (areas/portions)
organs/stomach.rs
organs/mouth.rs      (trigger list, bite chunk math, grab constraints intent, vomit)
organs/propulsion.rs (forces/torque intent + costs + herding/void/wrap)
organs/fat.rs
organs/armor.rs
organs/egg_layer.rs
organs/egg.rs        (egg entity: incubation + hatch)
organs/pheromone.rs
organs/clock.rs
physics.rs     shapes, broadphase hash, contact solve, damping/integration, contact events,
               grab joint approximation, pellet merge
```
Keep all formulas as small pure `fn`s with unit tests (see §12 cheat sheet); keep the game's order (§2.3).

### 10.2 SoA body columns

Suggested columns (all `f32` except ids/flags); size-change recomputes the static bundle once:

```rust
// updated every tick
pub energy: f32, pub health: f32, pub d2_size: f32, pub growth: f32,
pub attacked_dmg: f32, pub brain_ticks: f32, pub vision_lookup: f32, pub vision_sense: f32,
pub inflate: f32, pub corpse_decay: f32, pub corpse_energy_offset: f32,
pub metabolism: f32, pub metabolic_rate: f32, pub metabolic_efficiency: f32,
pub healing_cost_rate: f32,
// flags: born, dying, dead, mature, egg_mature
// derived cache per size (BodyStatic): d1_size, base_area, body_length, effective_radius,
//   internals_radius, side_radius, base_mass, max_health, max_energy, base_energy_density,
//   armor_thickness (also depends on inflate), organ areas/portions
```

Per-tick organ state in parallel arrays/structs: `Stomach { n: u8, content: [Content; 2] }`,
`Mouth { bite_progress, attacked_last, attacked_this, n_in_mouth, in_mouth: [EntityId; 10],
n_held, held: [EntityId; 10], desired_* }`, `Fat { amount, sustain_rate, gain_rate, inflate }`,
`EggLayer { progress, n_laid, energy_consumption }`, `Propulsion { last_pos, total_travel, signed_vel,
norm_vel, angular_vel, angle, movement_cost }`, `Pheromone { progress }`, `Clock { tic, tic_progress,
time_alive, chrono_time, period }`.
Grab constraints are simulation-entity links (`holder_bibite ↔ held_entity`, break_force/torque); a joint that
exceeds break force removes the link and calls the "link broke" path (release + `onJointBreak` semantics).

### 10.3 Custom collision solver — required behaviour

Shapes: pellets = circles (`r = √(amount/π)`); bibite body = capsule from §1.7 (or a compound: capsule + child
parts); eggs = circle/ellipse `5·sizeGene²`-derived (egg collider shape is prefab `[PREFAB]`; use circle radius
`√(5·d1birth²/π)` as an approximation, flag in open questions); mouth = axis-aligned box trigger in mouth local
space (`2.72 × mouthLen`), exposed as a query shape rather than a solver body.

Required interaction pairs (derived from code paths):

| Pair | Behaviour |
|---|---|
| bibite part ↔ bibite part (different bibites) | solid contact (push both), collision damage via §6.3, `onBodyCollided` event |
| bibite part ↔ pellet (pelletCollision on) | solid contact (pellets push/are pushed; pellet drag/mass matter) |
| bibite part ↔ pellet (pelletCollision off) | trigger only (no contact), mouth still detects |
| bibite part ↔ egg (eggCollision) | solid contact; trigger only when disabled |
| bibite part ↔ own parts | ignored (explicit `IgnoreCollision`) |
| mouth trigger ↔ pellets / bibiteParts / eggs | trigger detection only (no impulse), target list for §5.3 |
| pellet ↔ pellet | solid only if `pelletCollision`; merge if `pelletMerge` (momentum-conserving, §6.1) |
| egg ↔ anything | solid only if `eggCollision`; eggs are not grabbable/eatable directly (tag `egg` not handled by mouth) |
| pheromone spot ↔ anything | trigger only (detected by Pherosense query) |
| zone bounds / static world | no solid contacts for entities in normal play (void/shade/wrap handled by code) |

Solver contract:
* Forces/torque are applied as velocities: `v += F·dt/m`; `ω += τ·dt/I` with `I = m/2·realBodyArea/π`.
* Box2D drag approximation: apply `v *= 1/(1+linearDamping·dt)`, `ω *= 1/(1+angularDamping·dt)` (Unity/Box2D
  damping is exponential-ish; exact trajectory divergence is permitted, record it).
* Positional correction + restitution/friction values are **not** in scripts (`[PREFAB]`/Box2D defaults:
  friction 0.4, restitution 0, bounciness 0); choose defaults and validate against behavioural invariants
  (pushing, wedging, pile formation) — do not chase contact-level parity (ARCHITECTURE §7).
* Iterate contacts a fixed number of times with deterministic ordering (sort by entity id) for reproducibility.
* Grabs: implement as a 2-D point/spring constraint between holder and held body, computing the accumulated
  impulse each tick; break when `|impulse| > breakForce` (or torque equivalent) → notify mouth path
  (release, compact links). Box2D FixedJoint2D is rigid + rotational, so use a position+angle constraint.
* Mouth triggers must report enter/exit per tick so the mouth list matches `OnTriggerEnter2D/Exit2D` semantics
  (including the "list is from previous physics step" one-tick latency only if we choose to preserve it).
* Pellet spawn/merge must update mass/drag/scale as §6.1.
* Mass used by collision damage is the full body mass property (§1.3), including stomach/egg/fat/armor mass.

### 10.4 Determinism and RNG mapping

* One `Rng` per simulation; all body/egg randomness draws from it. Document per-call-site domains:
  `body.birth_phase` (3 draws), `egg.id` (hatch-time random), `egg.orientation` (template eggs only),
  `meat_pile.jitter` (2 draws per pile), `mutation.*` (genome spec), `repair/visual` (non-sim, do not draw
  from sim Rng).
* Order of draws must follow processing order (stable entity order, stable organ order). Never draw inside a
  parallel loop without a derived per-entity stream (`Rng::derive(seed, DOMAIN, entity)`), otherwise parallel
  vs serial diverge.
* `UnityEngine.Random.Range(0, n)` uses int semantics; `Random.value`/`Range(f32,f32)` differ — map explicitly.
* Because the original mutation RNG is unseeded (System.Random), exact parity is impossible; record the
  deviation (see §7.3).

### 10.5 Performance notes (body/organ hot path)

* Organ updates are branch-light scalar math; keep them in SoA passes per organ (stomach pass, fat pass, move
  pass, mouth pass) rather than per-entity virtual dispatch; the mouth/bite path is the only one that touches
  the world/query system.
* Do not compute `effectiveRadius`/`internalsRadius` per access; cache in `BodyStatic` on size/inflate/armor
  change (inflate changes only via fat; armor thickness only via WAG/inflate).
* Stomach content max 2; egg/children lists small — use `ArrayVec`-like fixed arrays (dependency-free: plain
  `[T; N]` + len) to avoid allocation.
* Mouth trigger list capacity 10, `nInMouth` scan/sort per tick; if we can process contacts in solver order we
  avoid the sort (but then bite target selection order may differ — sort by distance to preserve determinism
  and behaviour).
* `f32` everywhere; guard divisions by zero exactly where the game does (jawArea 0, fat portion 0, fullness 0,
  `averageChunkAmount` 0) and log/flag NaN at the same points.

---

## 11. Open questions / ambiguities / values needing capture

Each item is a concrete blocker or validation target. **Do not guess silently.**

| # | Topic | Status / needed action |
|---|---|---|
| BQ-1 | `Energy` and `Health` `MatterMaterial` properties (`EnergyDensity`, `MassDensity`, `Cohesiveness`) for **MatterMaterialManager.Health / Energy**. These are built-in assets, absent from all save files; they determine starvation damage per energy deficit (`Health.Material.AmountOfEnergy`), corpse decay time, and `UseEnergy` fallback. | **RESOLVED (2026-10-29 runtime capture):** `Energy.energyDensity = 1.0`, `Health.energyDensity = 0.25` (the other fields are unused by simulation paths). Implemented as settings-derived `BodyParams::{energy,health}_material_density` in `body.rs` with tests; body-dev's constants carry the provenance comment. |
| BQ-2 | `[PREFAB]` constants: `bodyCollider` direction/isTrigger/edge radius, body-part collider shapes/offsets/tags/layers, mouth trigger local position/size, egg collider shape/material, physics material (friction/restitution), rigidbody `gravityScale`/interpolation/constraints/collisionDetection/sleep, Physics2D layer collision matrix. | **PARTIALLY RESOLVED:** collider shape/offset constants, friction/restitution pairs, damping, layer names/matrix and Box2D solver constants are captured in `physics.rs` (see the status notes above). Remaining assumptions: gravityScale 0, no interpolation/sleep/constraints, child body-part list modelled as root capsule + tail capsule + mouth box. |
| BQ-3 | Update order among separate `FixedUpdate` components (`BibiteBody`, `BibiteGrowth`, `InternalClock`, `EggHatching`, `MatterDecayProcessor`). No execution-order attribute; Unity order is load-order dependent. | **RESOLVED (canonical choice):** §2.6 order is implemented in `body.rs` and the world pipeline; one-tick-order effects are validated by parity captures. Deviation recorded as D3. |
| BQ-4 | Sign convention of `Collision2D.relativeVelocity` vs `ContactPoint2D.normal` in `BibitePart.OnCollisionEnter2D` (damage depends on `dot(normal, relativeVelocity)`); Unity does not document orientation precisely. | **PARAMETERISED + capture-consistent:** `body::CollisionSign` (`AsIs` default) makes the choice explicit; the parity capture (`docs/DEVIATIONS.md` Q11) shows both partners taking damage with positive approach speed, matching the default. The body-mass weighting term remains capture-unverified (the bridge cannot read/set mass). |
| BQ-5 | Number/identity of `bodyParts` and whether the root capsule is a trigger for close-range detection; how mouth trigger avoids its own body parts. | Prefab inspection; needed to model bite acceptance (`bibitePart` colliders) and self-collision. |
| BQ-6 | Whether `RipChunkOff`'s `sqrt(chunk)/π` (vs `√(chunk/π)`) is intentional — it makes ripped chunks spawn closer than geometric; affects pellet-eating behaviour. | Preserve exact formula; validate via capture of chunk sizes/positions. |
| BQ-7 | Default `simTPS`: `ScenarioIndependentSettings` default 30, `UserSettings.simTPSDefault` 40 for new games, real saves use 15/30/40. ARCHITECTURE currently says 40. | **RESOLVED:** engine default is 30 (`clock.rs::DEFAULT_SIM_TPS = 30`), always overridden by the effective settings; `docs/ARCHITECTURE.md` §6 was corrected to 30. |
| BQ-8 | `Health`/`Energy` matters' `Amount` clamping when `Energy.MaxAmount` changes on shrink (not clamped in `Set2DSize`), and negative `GainEnergy` semantics. | Decide to mirror code exactly (recommended) and record edge-case tests. |
| BQ-9 | `BibiteMouth` first-tick stale values: `desireToAttack/Grab/Swallow`, `mouthOpening`, `biteOpening`, `biteStrength` are only assigned when `nInMouth > 0`; properties used by other code (UI) may read stale values. | Non-essential for headless (UI only); mouth state machine must assign before use each tick as code does. |
| BQ-10 | `BibiteStomach.RemoveMatter` dead branch (never removes a content slot, can read past `nContent`); vomit/spill behaviour depends on it. | Mirror or fix consciously; recommend mirroring amount subtraction + drop-empty (the intended behaviour) and log deviation for parity. |
| BQ-11 | `BibiteBody.Die` + `!corpsesEnabled` + swallowed: bursts into meat while the biter already got the meat value (double count). | Mirror unless validation says otherwise. |
| BQ-12 | `BibiteGrowth`: `eggLayer.isMature` is never reset when growth drops below maturity; shrinking emits negative growth cost (energy gain). | Mirror; add regression tests. |
| BQ-13 | `Health.Material.EnergyOfAmount(maxHealth)` in `corpseEnergyOffset` vs `HealthEnergyRatio` used by `Attack`: implies two different health-energy densities unless the asset matches; see BQ-1. | Capture asset; may explain corpse meat values. |
| BQ-14 | `PheromoneSpot` layer/collider setup and fade constants; `PheromoneSpot` particle randomness is visual only. | Ensure it is a trigger on the pheromone layer; exact collider size irrelevant (query uses distance). |
| BQ-15 | `.bb8` round-trip: old schema keys (`body.born/totalEnergy/wasteBank`, gene keys `Strength/Defence`, transforms `OldSerializedTransform`) must be preserved as unknown tokens or migrated. | `fmt-dev` handles with `bibites-io`; body spec provides the field mapping/migrations. |
| BQ-16 | Brain birth costs (`neuronBirthCost`, `synapseBirthCost`) are in settings but not charged anywhere in body/brain code found; confirm whether brain spec charges them at `FinalizeEditing`. | **CONFIRMED STILL OPEN:** the settings exist in `settings.rs` but no core code charges them, matching the decompiled source. Kept for round-trip fidelity; do not charge without a spec/capture decision. |

---

## 12. Formula and constants cheat sheet

### 12.1 Constants (settings defaults unless noted)

| Name | Default | Unit | Source |
|---|---:|---|---|
| `healthPerArea` | 1.25 | HP/u² | `ScenarioSettings.cs:846-858` |
| `storableEnergyPerArea` | 1.5 | E/u² | `:860-872` |
| `baseBodyGrowthCost` | 2.0 | E/u² | `:874-885` |
| `viewAngleAddedCost` | 0.003 | E/(°·u²) | `:887-899` |
| `viewRadiusAddedCost` | 0.002 | E/(u·u²) | `:901-913` |
| `healthEnergyFactor` | 0.5 | — | `:915-928` |
| `bibiteMassDensity` | 0.025 | g/u² | `:534-546` |
| `stomachContentContribution` | 1.0 | — | `:548-561` |
| `baseMetabolismCost` | 0.001 | E/(u²·s) | `:674-686` |
| `energyUsageEfficiency` | 0.8 | — | `:688-700` |
| `moveMusclesCost` | 0.02 | E/(u²·s) | `:703-715` |
| `armMusclePressure` | 50 | N/u | `:480-491` |
| `forwardForceSizePower` | 1.5 | — | `:493-504` |
| `turnForceSizePower` | 3.0 | — | `:506-517` |
| `backwardFraction` | 0.5 | — | `:519-532` |
| `dragCoefficient` | 5.0 | 1/s | `:459-470` |
| `bitingPressure` | 80 | N/u | `:563-574` |
| `jawMusclesSizingPower` | 2.0 | — | `:605-615` |
| `bitePeriodFactor` | 1.0 | — | `:617-629` |
| `bitingDamageFactor` | 5.0 | — | `:631-644` |
| `throwingForceFactor` | 0.25 | — | `:576-589` |
| `bitingThrowForceFactor` | 0.25 | — | `:591-603` |
| `collisionDamageConstant` | 0.1 | dmg/(u/s) | `:646-658` |
| `collisionDamageThreshold` | 50 | u/s | `:660-672` |
| `ageingThreshold` | 1.0 | h | `:800-812` |
| `ageStrengthPenalties` | 0.25 | — | `:814-828` |
| `ageMetabolismPenalties` | 0.5 | — | `:830-844` |
| `healRate` | 0.001 | 1/s | `:944-956` |
| `healPowerFactor` | 1.0 | — | `:958-968` |
| `healEfficiency` | 1.0 | — | `:970-982` |
| `corpseDecayFactor` | 1.0 | — | `:984-995` |
| `fatSustain` | 0.0 | 1/s | `:444-457` |
| `minPelletSize` | 0.1 | u² | `:265-275` |
| `corpsesEnabled` | true | — | `:277-284` |
| `pelletCollision` / `pelletMerge` / `pelletRotation` / `eggCollision` | true/false/true/true | — | `:221-255` |
| `neuronUpkeepCost` / `synapseUpkeepCost` | 0.001 / 0.00025 | E/s | `:745-771` |
| `neuronBirthCost` / `synapseBirthCost` | 0.1 / 0.025 | E | `:717-743` |
| `pheromoneProductionCost` / `Strength` | 0.01 E/s / 10 u² | | `:773-798` |
| `brainTPS`/`decayTPS`/vision factors | 2/2/10/5 | ticks | `ScenarioIndependentSettings.cs:182-228` |
| `simTPS` | 30 (class default; 40 user default; file may override) | ticks/s | `ScenarioIndependentSettings.cs:170-180`, `UserSettings.cs:439-443` |
| `SimulationSize` | 2000 | u half-width | `ScenarioIndependentSettings.cs:16-40` |
| `limitBibiteBirth` / `bibiteLimit` | false / 100 | — | `:92-111` |
| `bibiteMassDensity` note | mass ≈ 0.025·80·d2 = 2·d2 g | | |
| Body constants | `80` area at d2=1, `12.5` length at d1=1, `100` legacy | u², u | `BibiteGenes.cs:87-89`, `BibiteBody.cs:69` |
| `BibiteFatOrgan.InflationAtMaxFactor` | 0.35 | — | `BibiteFatOrgan.cs:52,85` |
| `EggHatching.eggArea` | `5·sizeGene²` | u² | `EggHatching.cs:69` |
| Mouth trigger width | 2.72 | u | `BibiteMouth.cs:298` |
| Bite Newton iterations | exactly 2 | — | `BibiteMouth.cs:675-683` |
| Grab slots / mouth slots | 10 / 10 | — | `BibiteMouth.cs:36,40` |
| Stomach slots | 2 (Plant, Meat) | — | `BibiteStomach.cs:149`, `MatterMaterialManager.cs:74-75` |
| `NormalBodyPointAtSize1` | 100 (unused) | — | `BibiteBody.cs:69` |

### 12.2 One-page formula card

```
SIZE/GROWTH
 d2 = growth·SizeRatio²            d1 = √d2
 area = 80·d2                      len = 12.5·d1
 growthAtBirth = (Hatch/Brood)²    growthAtMature = max(1, growthAtBirth/max(wombPortion,1e-3))
 rate(m) = SpeedRatio/(1+GrowthMaturityFactor·m^GrowthMaturityExponent)
 Δgrowth = Want2Grow·rate·P·GrowthScale/100          P = growth update period (0.2/rate)
 growthCost = Δd2·80·BodyEnergyDensity·(1+healthEnergyFactor)
 maturity = growth/growthAtMature
BODY
 BodyEnergyDensity = 2 + ViewAngle·0.003 + ViewRadius·0.002 + armorPortion·1.5
 maxHP = 1.25·area   maxE = 1.5·area   baseMass = 0.025·area
 mass = baseMass + stomachMass + eggMass + fatMass + armorMass
 Eff = 0.8^SpeedRatio
 metab = 0.001·area·(1+0.5·ageingFactor)      ageingFactor = max((ageH−baseAgingThreshold/SpeedRatio)/(baseAgingThreshold/SpeedRatio),0)
COSMETIC/LIFE
 healMaxRate = (1/d2)^healPowerFactor·healRate·SpeedRatio   (fraction of maxHP/s)
 AGE: ageStrengthPenaltyFactor=(1−0.25)^ageingFactor (movement only)
MOVEMENT
 moveArea = area·MoveWAG/ΣWAG
 F = 50·(moveArea·SpeedRatio)^(1.5/2) · clamp(Accelerate·penalty,−1,1)  (×0.5 if negative)
 T = 50/4·(moveArea·SpeedRatio)^(3/2) · clamp(Rotate·penalty,−1,1)
 penalty = √(health/maxHealth)·ageStrengthPenaltyFactor
 cost/s = 0.02·moveArea·|a| + 0.01·moveArea·|t|   then ÷Eff
 damping = 5·d1/mass   inertia = mass/2·realArea/π
MOUTH
 jawArea = area·JawWAG/ΣWAG     jawMass = jawArea·0.025/2
 throatWidth = √(area·ThroatWAG/ΣWAG)
 jawStrength = 80·(jawArea·SpeedRatio)^(2/2)
 bitePeriod = √(throatWidth·jawMass/jawStrength)·70·1
 m = EatOut>0.15 ? EatOut·throatWidth : 0
 swallow r ≤ m/2; r>m/4 needs a period
BITE (target bibite): h_m = AttackOut·throatWidth·(internalsRadius/effectiveRadius)
                   F = max(jawStrength·AttackOut − 2·armorStrength, 0)   (first bite only)
                   r = internalsRadius, σ=35
BITE (pellet): h_m = EatOut·throatWidth, F = jawStrength·AttackOut, r=pelletR, σ=pellet.cohesiveness
 K = F/(h_m·σ); θ = asin(min(min(K,1)·h_m/(2r),1)); A = r²(θ−sinθcosθ)
 if K>1: seed a=0.49(ln(K−1)−0.3125); θ=(π/2)(1−K^(−exp(a/(16^a−1)))); 2×Newton; A += rB²(θ−sinθcosθ)
 damage = 5·target.maxHP·A/target.realArea     (ongoing: ×dt/(5·bitePeriod))
 armor: no damage if jawStrength·AttackOut ≤ target.armorStrength
DIGEST (per content)
 fill = Σamount/area; acid = DigestionOut; excess = max(acid−fill,0)
 effDigest = min(acid/fill,1)·SpeedRatio·(1+excess)      effGain = convEff·(1−excess/2)
 r = √(avgChunk/π); d = min(reactivity·effDigest·dt, r); dA = d(2r−d)π
 chunk → GainEnergy(reactivity·dA·n·energyDensity·effGain)
 affinity: plant (1−Diet)^p, meat Diet^p;  efficiency = minEff+clamp01(aff)·(maxEff−minEff)
FAT
 Δ = act·d2·SpeedRatio·(1·dt)²·π   (act = ±InverseLerp(...) with full² penalty on storing)
 storage costs Δ·30/eff, burn gains −Δ·30·eff, sustain cost fatEnergy·fatSustain
 inflate = (fat−baseCapacity)/(0.35·area)
EGG
 eggEnergy = 100·d2Birth·(1.5+SumRatio) + brainUpkeep·SizeRatio
 Δprogress = SpeedRatio·remap(EggProdOut)·dt/LayTime;  eggProgress≥1 & Want2Lay≥0.25 ⇒ lay
 hatch after HatchTime s if energy ≥ totalRequiredEnergy (=100·d2Birth·SumRatio + brainUpkeep·SizeRatio)
DEATH
 corpse: energy=0, health=max, offset = HealthMatED·maxHP − energyBefore; decay dmg = metab·decay·dt/HealthMatED
 burst meat = SumRatio·area − offset + eggs + fat (+ energy terms as booked)
```

---

## 13. Source index (line anchors used above)

| Ref | File:lines |
|---|---|
| body hub & stats | `BibiteBody.cs:17-60, 69-152, 230-312` |
| body settings cache | `BibiteBody.cs:154-210, 314-392` |
| Awake/Start/StartBody | `BibiteBody.cs:394-441` |
| start-at-growth/resume/finalize | `BibiteBody.cs:443-571` |
| per-tick | `BibiteBody.cs:573-648` |
| coin/trade funcs (`UseEnergy`,`Heal`,`GainEnergy`,`Set2DSize`) | `BibiteBody.cs:658-748` |
| damage/death/corpse/revive/destroy | `BibiteBody.cs:750-949` |
| body save/load | `BibiteBody.cs:951-1032` |
| organs base | `BibiteOrgan.cs:1-38`, `BibiteSpatialOrgan.cs:1-45` |
| mouth | `BibiteMouth.cs:1-972` |
| stomach/content | `BibiteStomach.cs:1-377`, `StomachContent.cs:1-72` |
| propulsion | `BibitePropulsion.cs:1-347` |
| fat/armor | `BibiteFatOrgan.cs:1-127`, `BibiteArmor.cs:1-77` |
| egg layer/hatch | `BibiteEggLayingOrgan.cs:1-267`, `EggHatching.cs:1-280` |
| growth/clock | `BibiteGrowth.cs:1-150`, `InternalClock.cs:1-108` |
| phero | `BibitePheromoneOrgan.cs:1-106`, `PheromoneSpot.cs:1-151`, `Pherosense.cs:1-227` |
| parts/collision damage | `BibitePart.cs:1-67` |
| genes/formulas | `BibiteGenes.cs:16-226, 287-416` |
| gene table | `BibiteEditorSettings.cs:11-591` |
| settings | `ScenarioSettings.cs` (per-formula refs in §12.1), `ScenarioIndependentSettings.cs:16-231` |
| brain upkeep/outputs | `NEATBrain.cs:283, 312-315, 570-641` |
| matter/materials/pellets | `Matter.cs:1-77`, `MatterMaterial.cs:1-65`, `MatterMaterialManager.cs:1-171`, `MatterMaterialSettings.cs:1-264`, `MatterPellet.cs:1-278` |
| decay | `MatterDecayProcessor.cs:1-305`, `MatterDecayOfArrays.cs:71-124` |
| spawning/prefabs | `WorldObjectsSpawner.cs:139-336`, `BibiteSpawner.cs:124-186` |
| save system | `SaveSystem.cs:426-556, 65-84, 815-868`, `SerializationHelper.cs:23-258` |
| migrations | `BibiteUpdater.cs:11-104`, `SerializationHelper.cs:309-518`, `VersionTracker.cs:9-45` |
| time/tps | `TimeController.cs:58, 78-130`, `UserSettings.cs:439-467` |
| sprites (visual) | `BibiteProceduralSpriter.cs:75-168`, `ProceduralSpriteManager.cs:146-186, 222-233` |
| tracker | `BibiteTracker.cs:40-214` |

---

*End of spec 02. Companion specs: `01-brain-genetics.md` (genome/mutation/brain), `03-world-ecology.md`
(pellets/zones/senses/physics world), `04-formats-settings.md` (`.bb8`, `.bb8settings`, save bundles).*

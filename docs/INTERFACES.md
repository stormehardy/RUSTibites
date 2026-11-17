# RUSTibites — Cross-module interface contract

This document freezes the **boundaries between subsystems** so several people
can implement in parallel without churning each other's code. It complements
[`ARCHITECTURE.md`](ARCHITECTURE.md) (whole-project design) and the subsystem
specs in `docs/spec/`.

**Rule:** the concrete types below are the contract. If an implementation needs
a different signature, message the Lead; the Lead updates this document and
notifies the owners. Do not quietly change a shared type.

---

## 1. Ownership summary

| Type / module | Owner | File(s) |
|---|---|---|
| `Vec2`, maths, `Rng`, `Arena`, ids, `Error/Result`, `FixedClock` | Lead | `core::{math,rng,arena,ids,error,clock}` |
| `Gene`, `Genome`, `Genome::child/mutate`, gene-derived formulas | brain-dev | `genome.rs` |
| `Brain`, `BrainInputs`, `BrainOutputs`, `Input`, `Output`, brain mutation/distance | brain-dev | `brain.rs` |
| `SpeciesRegistry`, speciation distance/assignment, lineage records | brain-dev | `species.rs` |
| `SimSettings` and typed setting models (defaults exact) | fmt-dev | `settings.rs` |
| `BibiteStore`, `BibiteView`, body/organs systems, `EggStore` | body-dev | `body.rs`, `organs.rs` |
| Physics solver | body-dev | `physics.rs` |
| `PelletStore`, `Matter`, `MaterialProperties` | world-dev | `pellet.rs` |
| `PheromoneStore` | world-dev | `pheromone.rs` |
| `SpatialHash`, `World`, `ZoneStore`, `ToolStore`, `SenseOutputs`, sensing, `SimStats` | world-dev | `world.rs`, `zone.rs`, `senses.rs`, `tools.rs`, `sim.rs` |
| File formats and core↔format conversion | fmt-dev | `crates/bibites-io/**` |
| CLI, parity harness, integration | Lead | `crates/bibites-cli/**`, `tools/parity/**` |

Dependency direction inside `bibites-core`: everything may depend on the
foundation; `world` may depend on all domain modules; `body` may depend on
`genome`, `brain`, `pellet`, `pheromone`, `settings`; `senses`/`tools`/`zone`
may depend on `body`/`pellet`/`pheromone`/`settings`; `genome`/`brain` must not
depend on `body`/`world`. No cycles: if one appears, route through a plain data
struct or a callback owned by `world`.

---

## 2. Conventions

- **`f32` state, `f64` reporting sums.** No `f64` in per-tick entity state.
- **Entity identity:** generational handles from [`Arena`](../crates/bibites-core/src/arena.rs)
  wrapped in the typed ids in [`ids.rs`](../crates/bibites-core/src/ids.rs)
  (`BibiteId`, `EggId`, `PelletId`, `PheromoneId`, `ZoneId`, `ToolId`).
  `SpeciesId`/`LineageId` are persistent `u32`s.
- **No `serde` in core.** Format crates read/write plain data via public
  accessors or constructors.
- **No hidden randomness.** Every function that needs randomness takes
  `&mut Rng` explicitly. Lists that affect float sums are iterated in slot
  order (or explicitly sorted) for determinism.
- **No allocation in the tick loop.** Reusable scratch structs are passed in
  (`&mut Scratch`), owned by `World`.
- **Units** are the game's: distance in world units, time in seconds, energy in
  energy units. Sign conventions (verified in brain spec §0): angles are
  measured from forward (`transform.up`); a target on the **right** gives a
  **negative** angle input (Unity `SignedAngle` math, positive = CCW). Positive
  `Rotate` output = positive angular velocity = CCW in our world coordinates =
  turn toward the bibite's left; positive angle + positive Rotate steers *toward*
  the target. `RotationSpeed` input = angular velocity in revolutions/s, same
  CCW-positive sense. In-game tooltips mirror the handedness; do not follow
  their labels.

---

## 3. Settings façade (fmt-dev)

`settings.rs` owns the exact typed model and defaults. All other modules consume
it read-only through one aggregate:

```rust
/// Complete effective simulation settings: scenario-independent + scenario +
/// per-zone + materials. Built by `bibites-io` from a scenario/save/settings
/// file, or `SimSettings::game_defaults()` for a vanilla run.
pub struct SimSettings {
    pub sim_tps: u32,               // default 30
    pub brain_tps: u32,             // default 2 (brain update every 2 ticks)
    pub decay_tps: u32,             // default 2
    pub vision_lookup_updates: u32, // default 10 (brain updates)
    pub vision_sense_updates: u32,  // default 5  (brain updates)
    pub scenario: ScenarioSettings, // all scenario settings, exact defaults
    pub independents: IndependentSettings, // sim size, etc.
    pub materials: MaterialSettings, // Plant/Meat/Fat/Armor (see §7)
}

impl SimSettings {
    pub fn game_defaults() -> Self;
    /// Brain period in seconds = brain_tps / sim_tps.
    pub fn brain_period(&self) -> f32;
    pub fn fixed_dt(&self) -> f32; // 1.0 / sim_tps
}
```

Field names for the hundreds of scenario/zone settings are defined by
`docs/spec/04-formats-settings.md` and mirror the game's JSON names in
`snake_case`. **fmt-dev is the source of truth for names.** Subsystems must not
invent ad-hoc setting structs once `settings.rs` exists; request missing fields
through the Lead.

**Genome settings seam:** until `settings.rs` lands (and permanently for
decoupling), `genome.rs` defines a narrow `GenomeSettings` trait with the
handful of values the gene formulas/mutation need (background mutation chance
and variance, relative mutation share, prevent mutations, energy usage
efficiency, base body growth cost, view angle/radius added costs, armour energy
density, storable energy per area, health energy factor). `settings.rs` must
implement `GenomeSettings for SimSettings` (the impl lives in `genome.rs`), and
`DefaultGenomeSettings` is test-only once that lands. If a gene formula needs a
new setting, add it to the trait *and* to the spec table first.

User-level settings not part of a scenario/save (e.g. `AllowEasterEggs`) are out
of scope for M1; use the game defaults (disabled) and note it in the deviation
log.

Runtime settings animators mutate settings during the run; all subsystems read
settings fresh at the point of use (do not cache values across ticks unless the
spec says the game caches them).

**Sequencing requirement:** fmt-dev lands `settings.rs` with `SimSettings` +
exact defaults *before* the io crate, so body/world/brain can compile against
it.

---

## 4. Genome and brain (brain-dev)

### 4.1 Genome

```rust
pub const N_GENES: usize = 35;

#[repr(usize)]
pub enum Gene { LayTime = 0, /* ... 35 variants, exact enum order ... */ }

/// Per-bibite genome state that is not part of the 35-gene array but is
/// serialized by the game and needed for mutation/speciation.
#[derive(Clone, PartialEq)]
pub struct GenomeMeta {
    pub generation: u32,
    pub species: Option<SpeciesId>,      // species inherited at birth
    pub species_tag: Option<String>,
    pub is_mutant: bool,                 // persistent two-step speciation flag
    pub is_rad: bool,
    pub is_rain: bool,
    pub is_killer: bool,
    pub is_source: bool,
    pub deux_parent: bool,               // dead code in 0.6.3.1; round-trip only
    pub parent1: Option<BibiteId>,
    pub parent2: Option<BibiteId>,
}

#[derive(Clone, PartialEq)]
pub struct Genome {
    pub genes: [f32; N_GENES],
}

impl Genome {
    /// The official "Basic bibite" starter gene set (captured template).
    pub fn starter() -> Self;
    pub fn gene(&self, gene: Gene) -> f32;
    pub fn set_gene(&mut self, gene: Gene, value: f32); // clamps to editor range

    // Body/world-facing derived values (exact formulas per body spec §genes):
    pub fn d1_size_at_birth(&self) -> f32;
    pub fn d2_size_at_birth(&self) -> f32;
    pub fn area_at_birth(&self) -> f32;
    pub fn area_at_mature(&self) -> f32;
    pub fn length_at_mature(&self) -> f32;
    pub fn growth_at_mature(&self) -> f32;
    pub fn organ_area_portion(&self, organ: Gene) -> f32; // 0..1
    pub fn total_organ_wag(&self) -> f32;
    pub fn metabolic_efficiency(&self, s: &SimSettings) -> f32;
    pub fn body_energy_density(&self, s: &SimSettings) -> f32;

    /// Creates the child genome and its meta: applies mutations (or clones),
    /// advances generation, inherits flags/species, aggregates mutation factors.
    pub fn child(
        &self,
        meta: &GenomeMeta,
        rng: &mut Rng,
        s: &SimSettings,
        factor: &MutationFactor,
    ) -> (Genome, GenomeMeta);
}

/// Aggregated extra mutation pressure (radiation dose etc.).
#[derive(Clone, Copy, Default)]
pub struct MutationFactor {
    pub probability_increase: f32,
    pub intensity_increase: f32,
}
```

### 4.2 Brain

```rust
pub const N_INPUTS: usize = 33;
pub const N_OUTPUTS: usize = 15;

#[repr(usize)]
pub enum Input { EnergyRatio = 0, /* ... exact brain spec order ... */ }

#[repr(usize)]
pub enum Output { Accelerate = 0, /* ... exact brain spec order ... */ }

/// One brain update's input vector. Body/world write fields by name.
pub struct BrainInputs { values: [f32; N_INPUTS] }
impl BrainInputs {
    pub fn set(&mut self, input: Input, value: f32);
    pub fn get(&self, input: Input) -> f32;
    pub fn write_world_senses<S: WorldSenseSource>(&mut self, senses: &S);
    pub fn write_internal<S: InternalSenseSource>(&mut self, state: &S);
}

/// Implemented by the world-owned sense output struct (21 world-derived inputs).
pub trait WorldSenseSource {
    // one getter per world-derived Input (vision, phero, clock, colour, ...)
}

/// Implemented by the body-owned internal state snapshot (12 body inputs).
pub trait InternalSenseSource {
    // one getter per body-derived Input (energy ratio, maturity, life ratio, ...)
}

pub struct BrainOutputs { values: [f32; N_OUTPUTS] }
impl BrainOutputs {
    pub fn get(&self, output: Output) -> f32;
}

pub struct Brain { /* nodes + connections, CSR-ready */ }

impl Brain {
    pub fn starter() -> Self;                                  // 3-synapse official starter
    pub fn step(&mut self, inputs: &BrainInputs, period: f32) -> BrainOutputs;
    pub fn upkeep(&self) -> BrainUpkeep { .. }                 // synapse/hidden counts
    pub fn mutate(
        &mut self,
        rng: &mut Rng,
        s: &SimSettings,
        genome: &Genome,
        factor: &MutationFactor,
    );
    pub fn distance(&self, other: &Brain, s: &SimSettings) -> f32;
    // io accessors: node/connection plain-record iterators + constructors;
    // node_max_innovation / conn_max_innovation i64 counters are per-brain and
    // exported to the world/species registry after mutation.
}
```

`Brain::step` is called by body-dev at the brain cadence, not by `World`.
The `period` is `s.brain_period()`.

**Batch/parallel brains:** `Brain::step_batch(brains: &mut [&mut Brain], inputs:
&[BrainInputs], period: f32, outputs: &mut [BrainOutputs])` evaluates a batch
bit-identically to serial (unit-tested). Body integration contract: pre-assemble
`BrainInputs` for due bibites from immutable state, split the record slice into
disjoint chunks (`split_at_mut`) and collect `&mut Brain`s, call the batch
function per chunk inside the world's thread pool, then apply outputs to organs
in dense slot order after the join. Brains share no mutable state and draw no
RNG, so parallel execution is order-independent.

**Sensing-usage masks (lazy vision):** `Brain` exposes cached
`used_input_mask() -> u64`, `uses_input(Input)`, `used_output_mask() -> u16`,
`uses_output(Output)`, category constants (`VISION_BIBITE_INPUT_MASK`,
`VISION_PLANT_INPUT_MASK`, `VISION_MEAT_INPUT_MASK`, `PHEROMONE_INPUT_MASK`,
`BODY_INPUT_MASK`, `CLOCK_INPUT_MASK`) and helpers (`needs_bibite_vision`,
`needs_plant_vision`, `needs_meat_vision`, `needs_pheromone_sensing`). The world
sensing phase uses these to skip whole FOV/pheromone computations exactly like
the game. Runtime-state parity accessors:
`node_state`/`set_node_state`, `input_value`, `output_value`,
`output_bias`, `node_type`, `node_innovation`, `input_count`/`output_count`.

---

## 5. Body and organs (body-dev)

`body.rs` owns the bibite store. `organs.rs` owns per-organ systems. The world
drives them; it never reaches inside private internals.

```rust
pub struct BibiteStore { /* SoA: pose, health, energy, genome, brain, organs, senses */ }

impl BibiteStore {
    pub fn new() -> Self;
    pub fn len(&self) -> usize;
    pub fn ids(&self) -> impl Iterator<Item = BibiteId> + '_;

    /// Cheap copy of the fields sensing/tools need (no borrowing conflicts).
    pub fn view(&self, id: BibiteId) -> Option<BibiteView>;

    pub fn genome(&self, id: BibiteId) -> Option<&Genome>;
    pub fn brain(&self, id: BibiteId) -> Option<&Brain>;
    pub fn senses(&self, id: BibiteId) -> Option<&SenseOutputs>;

    /// World sensing phase fills this after computing FOV/phero values.
    pub fn set_senses(&mut self, id: BibiteId, senses: SenseOutputs);
    /// Per-bibite sense throttle counters (owned by body, advanced by body).
    pub fn sense_throttle(&self, id: BibiteId) -> Option<SenseThrottle>;
    pub fn sense_throttle_mut(&mut self, id: BibiteId) -> Option<&mut SenseThrottle>;

    /// Mutation pressure hook used by rad towers / red death / etc.
    pub fn add_mutation_factor(&mut self, id: BibiteId, factor: MutationFactor);

    /// Spawns a bibite (from scenario load, virgin spawn, or egg hatch).
    pub fn spawn(&mut self, spawn: BibiteSpawn) -> Result<BibiteId>;

    /// Death path shared by damage and world tools; world observes `DeathEvent`s.
    pub fn kill(&mut self, id: BibiteId, cause: DeathCause);

    /// Body phase of the tick: brain cadence, organs, energy accounting,
    /// propulsion forces. `WorldCtx` gives read-only world data and the
    /// mutable services body needs (pellets for feeding, pheromones for
    /// emission, settings, rng, scratch).
    pub fn tick(&mut self, ctx: &mut BibiteTickCtx<'_>);

    // io accessors for every serialized field (body spec §save).
}

#[derive(Clone, Copy)]
pub struct BibiteView {
    pub id: BibiteId,
    pub pos: Vec2, pub vel: Vec2, pub rot: f32, pub ang_vel: f32,
    pub d1_size: f32, pub d2_size: f32, pub base_body_area: f32,
    pub max_health: f32, pub health_ratio: f32,
    pub dead: bool, pub is_corpse: bool, pub is_ready: bool,
    pub color: [f32; 3], pub eye_offset: f32,
}

/// Read-only world data + services body needs during its phase.
pub struct BibiteTickCtx<'a> {
    pub pellets: &'a mut PelletStore,
    pub pheromones: &'a mut PheromoneStore,
    pub eggs: &'a mut EggStore,          // egg laying/hatching
    pub zones: &'a [ZoneCircle],         // nearest-zone data for void avoidance
    pub settings: &'a SimSettings,
    pub rng: &'a mut Rng,
    pub scratch: &'a mut BibiteScratch,
    pub events: &'a mut Vec<BibiteEvent>, // preallocated, drained by world logging
}

/// Minimal zone geometry view for body steering (void avoidance).
#[derive(Clone, Copy)]
pub struct ZoneCircle {
    pub id: ZoneId,
    pub pos: Vec2,
    pub radius: f32,
    pub is_rect: bool,
    pub half_extents: Vec2,
}

/// World-visible events produced during the body phase (logging/stats/species).
pub enum BibiteEvent {
    Bitten { by: BibiteId, target: BibiteId, damage: f32 },
    Swallowed { by: BibiteId, pellet: PelletId },
    EggLaid { by: BibiteId, egg: EggId },
    Death { bibite: BibiteId, cause: DeathCause },
}
```

`SenseThrottle` (body-owned, brain-tick counters) is read by the world sensing
phase:

```rust
pub struct SenseThrottle {
    pub brain_counter: u32,      // ticks since last brain update
    pub lookup_counter: u32,     // brain updates since entity relookup
    pub sense_counter: u32,      // brain updates since sense refresh
}
```

**Pellet interaction rule:** body code must never write pellet amounts
directly. It calls `PelletStore::change_amount(id, delta) -> f32` and
`PelletStore::spawn_meat(...)`; those maintain counts, zone ownership, decay
registration and spatial-hash membership. Bites may also return events for
world logging.

**Eggs:** `EggStore` (body-dev) holds eggs; `World` advances hatching and calls
`BibiteStore::spawn` on hatch.

```rust
pub struct EggStore { /* egg records: pos, vel, genome, meta, brain, incubation, energy */ }
impl EggStore {
    pub fn ids(&self) -> impl Iterator<Item = EggId> + '_;
    pub fn spawn(&mut self, spec: EggSpawn) -> EggId;
    pub fn remove(&mut self, id: EggId) -> Option<EggRecord>;
    /// Advance incubation; returns eggs that hatched this tick.
    pub fn tick(&mut self, dt: f32, settings: &SimSettings, out: &mut Vec<EggId>);
}
```

---

## 6. World stores (world-dev)

```rust
pub struct PelletStore { /* SoA + free list */ }
impl PelletStore {
    pub fn ids(&self) -> impl Iterator<Item = PelletId> + '_;
    pub fn get(&self, id: PelletId) -> Option<PelletView>; // amount, material, pos, vel, radius, held, zone
    pub fn spawn(&mut self, spec: PelletSpawn) -> PelletId;
    /// Single choke point for eating/decay/merge amount changes.
    pub fn change_amount(&mut self, id: PelletId, delta: f32) -> f32;
    pub fn remove(&mut self, id: PelletId);
    pub fn spawn_meat_pile(&mut self, pos: Vec2, energy: f32, rng: &mut Rng) -> Option<PelletId>;
    /// Corpse-fidelity variant with explicit jitter radius + minimum pellet size.
    pub fn spawn_meat_pile_with_radius(
        &mut self,
        pos: Vec2,
        energy: f32,
        radius: f32,
        rng: &mut Rng,
        min_size: f32,
    ) -> Option<PelletId>;
    /// Fallible spawn for loaders (contract `spawn` panics on invalid input).
    pub fn try_spawn(&mut self, spec: PelletSpawn) -> Option<PelletId>;
    pub fn counts(&self) -> PelletCounts; // plant/meat count + biomass
}

pub struct PheromoneStore { /* SoA + free list */ }
impl PheromoneStore {
    pub fn emit(&mut self, spec: PheroEmission) -> Option<PheromoneId>;
    pub fn dissipate(&mut self, dt: f32); // 0.25 s quanta, 0.25/s
    // io accessors + queries used by sensing
}

pub struct ZoneStore { pub zones: Vec<ZoneData> }
pub struct ToolStore { /* towers, killers */ }

/// The 21 world-derived brain inputs (plus phero fields); exact field list and
/// semantics in world spec §3.8 / §4. The struct is world-owned and body-fed
/// into `BrainInputs::write_world_senses`.
pub struct SenseOutputs { /* fields per spec */ }

pub struct SpatialHash { /* uniform grid, class masks, reusable scratch */ }
```

### 6.1 World container and tick

```rust
pub struct World {
    pub settings: SimSettings,
    pub clock: FixedClock,
    pub bibites: BibiteStore,
    pub eggs: EggStore,
    pub pellets: PelletStore,
    pub pheromones: PheromoneStore,
    pub zones: ZoneStore,
    pub tools: ToolStore,
    pub species: SpeciesRegistry,
    pub stats: SimStats,
    // private: spatial hash, scratch buffers, rng streams, timers
}

impl World {
    pub fn new(settings: SimSettings, scenario: ScenarioData, seed: u64) -> Result<Self>;
    pub fn tick(&mut self);              // canonical pipeline, world spec §9.2
    pub fn run(&mut self, ticks: u64);
    pub fn stats(&self) -> &SimStats;
    pub fn population(&self) -> PopulationCounts;
    // save hooks: serialized-field accessors used by bibites-io.
}
```

`World::tick` implements exactly the pipeline in
`docs/spec/03-world-ecology.md` §9.2 (phases 0–15), calling:
`settings animators → tools positioning → zones → bibites (senses throttles,
brain, organs, forces) → eggs → phero dissipation → pellet decay → rad towers →
red death → colour killers → producers → physics step → collision callbacks →
stats/logging`.

**Phase-5 internal order (authoritative):** within "bibites", the order is fixed
by `docs/spec/02-body-organs-physics.md` §2.3/§2.6, not by the one-line summary
in the world spec: base metabolism + brain upkeep → brain (throttled senses →
`Brain::step`) → organs in the order armour → stomach → egg layer → fat → move →
mouth → pheromone organ → propulsion forces → `UseEnergy`. World spec §9.2 is
being corrected to match.

**Physics integration:** `physics.rs` borrows `BibiteStore` + `PelletStore`
directly (no duplicate shadow bodies). The stores expose the pose/velocity fields
physics needs as `pub(crate)` accessors; collision events carry the typed ids.

Physics is called as:

```rust
// physics.rs (body-dev)
pub fn step_physics(
    dt: f32,
    bibites: &mut BibiteStore,
    pellets: &mut PelletStore,
    eggs: &mut EggStore,
    bounds: &WorldBounds,
    settings: &SimSettings,
    scratch: &mut PhysicsScratch,
);
```

Bibite grabs are body state; the body phase sets grab links and the solver reads
them to apply joint-like constraints (or the body phase applies them before the
step, documented in physics.rs). The solver emits contact events with typed ids
for damage/collision logging.

---

## 7. Materials (world ↔ settings)

`pellet.rs` defines runtime material properties; `settings.rs` owns the
serialized material settings. The default values (captured from the game) are:

| Material | cohesiveness | reactivity | energyDensity | massDensity | minEfficiency | maxEfficiency | decay | freshTime | decayRate |
|---|---|---|---|---|---|---|---|---|---|
| Plant | 10.0 | 0.03 | 5.0 | 0.015 | -0.25 | 0.5 | false | — | — |
| Meat | 35.0 | 0.075 | 10.0 | 0.025 | 0.15 | 0.85 | true | 2000.0 | 0.02 |
| Fat | 0.25 | 1.0 | 30.0 | 0.03 | 0.5 | 0.85 | false | — | — |
| Armor | 500.0 | 1.0 | 1.5 | 0.05 | 1.0 | 1.0 | false | — | — |

W1 hardcodes these as `MaterialProperties::game_defaults()`; when `settings.rs`
lands, `SimSettings.materials` overrides them at world construction.

---

## 8. Formats boundary (fmt-dev)

`bibites-io` translates files ↔ core state; core contains no serde.

```rust
// bibites-io
pub fn load_scenario(path: &Path) -> Result<ScenarioBundle>;
pub fn load_save(path: &Path) -> Result<SaveBundle>;
pub fn load_genome(path: &Path) -> Result<GenomeFile>;
pub fn load_settings_file(path: &Path) -> Result<SimSettings>;

pub fn save_world(world: &World, path: &Path) -> Result<()>;
pub fn save_zone_data(world: &World, path: &Path) -> Result<()>;

pub struct ScenarioBundle {
    pub settings: SimSettings,
    pub zones: Vec<ZoneData>,
    pub tools: ToolStore,
    pub spawns: Vec<BibiteSpawn>,
    pub initial_pellets: Vec<PelletSpawn>,
    pub species_data: Vec<u8>, // data.bin opaque for M1
}
```

Every field the save format serializes must be readable from `World` and
constructible into a `World` via public APIs. If a field is missing, fmt-dev
raises it to the Lead and the owning dev adds an accessor.

`data.bin` is preserved opaquely in M1 (`Vec<u8>` pass-through).

---

## 9. Species and lineage (brain-dev)

```rust
pub struct SpeciesRegistry { /* species records, ancestors, stats */ }
impl SpeciesRegistry {
    /// Game `CheckNewSpecies` state machine; `meta` carries inherited species
    /// and the persistent `is_mutant` flag (updated in place).
    pub fn assign(
        &mut self,
        genome: &Genome,
        brain: &Brain,
        meta: &mut GenomeMeta,
        s: &SimSettings,
    ) -> SpeciesId;
    pub fn on_birth(&mut self, child: SpeciesId, parent: SpeciesId, generation: u32);
    pub fn on_death(&mut self, species: SpeciesId);
    pub fn tick_stats(&mut self, dt: f32);
    // io accessors for speciesData.json / save species section.
}
```

The registry also merges the per-brain innovation counters
(`node_max_innovation`, `conn_max_innovation`) after mutation; the world owns the
canonical values and saves them.

World owns the registry and calls it at the points the spec defines
(birth/death/stats cadence). Speciation does not affect individual behaviour.

---

## 10. CLI integration (Lead)

`bibites-cli` only uses the public interfaces above:

```rust
let settings = bibites_io::load_scenario(&path)?.settings;
let mut world = World::new(settings, scenario_data, seed)?;
world.run(ticks);
println!("{}", world.stats().summary());
```

The CLI must never need `pub(crate)` access. If it does, the interface is missing
something — fix the interface, not the CLI.

---

## 11. Implementation status (2026-10-30)

This section records the concrete APIs that have landed. Per-subsystem detail, section-by-section
status and the gating test names live in each spec's **Implementation status** table
(`docs/spec/README.md`).

**Foundation** (`math/rng/arena/ids/error/clock`) — done. `Arena<T>`/`Handle`
provide generational ids + dense iteration; `Rng` is xoshiro256\*\* with
`derive`/`fork`; `FixedClock` defaults to 30 TPS.

**Settings** (`settings.rs`, fmt-dev) — done: `SimSettings` (rates + scenario +
independents + materials), `ScenarioSettings` (78), `IndependentSettings` (14),
`MaterialSettings` (4×9), `ZoneSettings`, zone groups, spawn rows, animators,
conditions, challenge, enums, clamps, `game_defaults()`, `brain_period()`,
`fixed_dt()`, `validate()`.

**Genome/brain/species** (brain-dev) — done: `Genome`, `GenomeMeta`,
`MutationFactor`, `Genome::child`, derived gene formulas; `Brain` with CSR plan,
`step`/`step_batch`/`upkeep`/`mutate`/`distance`, `to_node_records`/
`to_synapse_records`, `from_records`, `from_template_records`, node runtime state
accessors, lazy sensing masks (`used_input_mask`, `needs_*_vision`);
`SpeciesRegistry` + `assign`/`assign_with_founder`, lineage records. Genome/brain
version migrations live in `bibites-io::migrate`. Independent Python reference
check passes bitwise (15,474 values, 0 ULP).

**Body** (body-dev) — done (B1 math, B2a store, B2b tick). Concrete API:
`BibiteStore::{new, with_settings, settings, set_settings, len, is_empty,
slot_capacity, ids, iter, for_each_mut, get, get_mut, view, genome, brain,
senses, set_senses, sense_throttle, add_mutation_factor, spawn, spawn_loaded,
remove, kill, to_save, set_collision_sign, tick}`; `BibiteRecord` (pose + genome/
meta/brain + body/organs/clock/growth/senses/throttle/mutation/children fields,
`recompute_derived`, `colliders() -> [physics::Collider; 3]` with captured prefab
constants); `BibiteSpawn`/`SpawnVitals`; `BibiteView`; `EggStore::{ids, spawn,
remove, get, get_mut, len, tick, drain_aborted, to_save, spawn_loaded}`; save
records `BibiteSave`/`EggSave`. The per-bibite tick is the two-pass schedule of
`docs/spec/02-body-organs-physics.md` §2.6 (pass A: metabolism/upkeep + due-brain
assembly and one `Brain::step_batch`; pass B: organs in game order, mouth/feeding/
combat, growth/clock, kinematics/finalization); physics hookup is
`BibiteStore::step_physics`. Parallel brains are gated by `PARALLEL_BRAIN_MIN_DUE`
and are bit-identical to serial.

**World** (world-dev) — done (W4): `SpatialHash`, `PelletStore`, `PheromoneStore`,
`ZoneStore`, `WorldBounds`, `SenseOutputs` (21 world inputs; body owns the other
12), `senses::{compute_fov, compute_phero, FovMask, FovParams}`, `ToolStore`
(RadTower/PheroTower/ColorKiller/RedDeathBloom), `SimStats`/`StatsSample`, and the
full 17-phase `World::tick` (clock → animators → tools → zones → sensing → body →
eggs → pheromones → decay → rad → red death → colour killers → producers → deaths
→ physics → events → hash sync). Sensing and brain batches are parallel and
bit-identical to serial. Load/save and world lifecycle: `World::new(settings,
ScenarioData, seed)`, `from_contents`/`to_contents`, scenario seeding + spawner,
`census_and_cull`, `set_threads`.

**Formats** (fmt-dev) — done for the 0.6.3.x tier (I1–I4): `.bb8`/`.bb8template`
read/write with version migrations and unknown-field preservation; `.bb8settings`
(78 scenario + 14 independent settings + materials + zones/groups/spawn rows/
animators, effective-settings precedence); save and scenario bundles; `WorldContents`
conversion (`load_world_contents`, `save_world_contents`, `world_contents_from_save`,
`save_bundle_from_contents`); `speciesData` typed through `LineageRecordIo` with a
raw fallback, `data.bin` opaque. Rust-written saves, templates and scenarios load in
the real 0.6.3.1 game (`docs/ARCHITECTURE.md` §8). Pre-0.6 settings migrations and
0.6.4a1 folder saves are **out of scope for M1** (Lead decision 2026-10-30).

**CLI** — done (`rustibites` binary): `run` (save/scenario/genome with `--ticks`,
`--seed`, `--threads`, `--settings`, `--save`, `--capture`, `--stats`), `bench`
(per-phase timings + thread scaling), `info`/`check`, `braincheck` (validated
bitwise against the independent Python reference) and the parity capture emitter
(`capture.rs`) consumed by `tools/parity/compare.py`. Remaining roadmap work:
evolution benchmark/CLI ergonomics (task-47).

Known gaps and open questions are tracked in the per-spec status tables,
`docs/DEVIATIONS.md` and `docs/API.md` §14 (rustdoc intra-doc-link clean-up).

---

## 12. Change log

| Date | Change | By |
|---|---|---|
| 2026-10-29 | Initial contract; simTPS corrected to 30 / brain period 2/30 | Lead |
| 2026-10-29 | Phase-5 organ order authority = body spec §2.3/§2.6; physics borrows stores directly | Lead |
| 2026-10-29 | Added `GenomeMeta`; updated `Genome::child`, `Brain::mutate`, `SpeciesRegistry::assign` signatures; corrected RotationSpeed/angle sign conventions to CCW-positive | Lead |
| 2026-10-29 | B2 ctx gained EggStore/zones/event sink; physics step takes eggs; EggStore API frozen; prefab constants captured (Energy density 1.0, Health 0.25) | Lead |
| 2026-10-29 | Brain inputs use `WorldSenseSource`/`InternalSenseSource` traits (defined in brain.rs); world/body implement them on their own structs | Lead |
| 2026-10-30 | §11 refreshed to current implementation (body B2b tick, world W4 pipeline, formats I1–I4, CLI complete); stale W4a/B2b "in progress" text removed | spec-dev (Lead-approved) |
| 2026-10-30 | M1 scope: pre-0.6 settings migrations and 0.6.4a1 folder saves explicitly out of scope | Lead |

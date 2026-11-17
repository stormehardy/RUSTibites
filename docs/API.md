# RUSTibites — public API guide

This is the practical map of the public Rust API. It complements:

* [`ARCHITECTURE.md`](ARCHITECTURE.md) — why the crates look like this (data layout, determinism,
  simulation pipeline);
* [`INTERFACES.md`](INTERFACES.md) — the cross-module type contract;
* [`spec/README.md`](spec/README.md) — the behavioural specifications the code implements;
* [`PARITY.md`](PARITY.md) — how runs are compared against the installed game.

Everything below is taken from the code, not from the design sketches; the runnable example that
accompanies this document is [`crates/bibites-core/examples/api_tour.rs`](../crates/bibites-core/examples/api_tour.rs)
(`cargo run -p bibites-core --example api_tour`). The io snippets were compiled against the real crates
(see §13).

---

## 1. Crate map

```
                 ┌──────────────────────────────────────────────┐
                 │ bibites-cli  (binary: rustibites)            │
                 │ load files, run, stats, captures, bench      │
                 └───────────────▲──────────────────────────────┘
                                 │
                 ┌───────────────┴──────────────────────────────┐
                 │ bibites-io    (serde, zip, file formats)     │
                 │ .bb8/.bb8template, .bb8settings, save/scenario│
                 └───────────────▲──────────────────────────────┘
                                 │  plain records only (no serde in core)
                 ┌───────────────┴──────────────────────────────┐
                 │ bibites-core  (no dependencies, no I/O)      │
                 │ simulation: world, body, organs, brain, ...  │
                 └──────────────────────────────────────────────┘

    optional: bibites-view (binary: rustibites-view) → core + io + rendering
```

| Crate | Role | Rule |
|---|---|---|
| `bibites-core` | Pure, deterministic simulation: maths, RNG, arenas, genome/brain, body/organs/physics, world/ecology, settings | no I/O, no external deps, `#![forbid(unsafe_code)]` |
| `bibites-io` | All file formats; converts files → plain core records and back | serde/zip live here only; no simulation logic |
| `bibites-cli` | Argument parsing, headless runner, stats, parity captures, benchmarks | UX only |
| `bibites-view` | Optional windowed viewer (milestone 2) | must not perturb the simulation |

Dependency direction is strictly `cli/view → io → core`; core never depends on io.

---

## 2. Minimal core flow

A `World` is settings + scenario data + seed. Construct it, seed a fresh scenario, spawn from
templates, tick.

```rust
use bibites_core::brain::Brain;
use bibites_core::genome::{Genome, GenomeMeta};
use bibites_core::settings::{GrowthAtSpawn, SimSettings, Tagging};
use bibites_core::world::{ScenarioData, TemplateSource, World};
use bibites_core::Vec2;

let settings = SimSettings::game_defaults();          // captured 0.6.3.1 defaults (30 TPS)
let scenario = ScenarioData::with_default_zone_group(); // 10 plant zones
let mut world = World::new(settings, scenario, 42)?;    // seed = RNG seed

// A fresh scenario seeds its zones with plants; a loaded save must NOT re-seed.
world.seed_scenario_world();

let template = TemplateSource::new(
    "Basic bibite",
    Genome::starter(),
    GenomeMeta::default(),
    Brain::starter(),
);
world.set_templates(vec![template.clone()]);
let id = world.spawn_bibite_from_template(
    &template,
    GrowthAtSpawn::Adult,
    Tagging::NoTagging,
    "",
    Vec2::ZERO,
)?;

world.run(300);                                         // 300 ticks = 10 s at 30 TPS
let sample = world.sample_stats();
println!(
    "t={:.2}s bibites={} plant_biomass={:.1}",
    world.simulated_time(),
    sample.counts.bibites,
    world.pellets.counts().plant_biomass,
);
```

`World::run(n)` is just `n × World::tick()`; use `tick()` directly when an outer loop needs to
inspect state every tick. `World`'s data is in public stores (`world.bibites`, `world.eggs`,
`world.pellets`, `world.pheromones`, `world.zones`, `world.tools`, `world.species`) — the simulation
borrows them internally, so callers read them between ticks, not during one.

---

## 3. Load a real save/scenario and tick

File parsing never leaks into core: `bibites-io` parses the zip into plain records wrapped in
`WorldContents`, and core rebuilds its stores from that. The snippets assume a function returning
`Result<(), Box<dyn std::error::Error>>`.

```rust
use bibites_core::world::World;
use bibites_io::{load_world_contents, save_world_contents};

// 1. io: every zip section → plain core records (unknown fields preserved in io).
let contents = load_world_contents("world.zip", None)?;
// 2. core: stores + spatial hashes rebuilt; simulated_time and ids restored.
let mut world = World::from_contents(contents)?;
// 3. fixed-timestep pipeline (17 phases, spec 03 §9.2).
world.run(300);
// 4. write a game-loadable save (unknown fields preserved where io has them).
save_world_contents("out.zip", &world.to_contents())?;
println!("t={:.2}s bibites={}", world.simulated_time(), world.bibites.len());
```

Scenarios are a fresh start, not a save: parse the scenario bundle, construct a `World` directly
from its settings + core scenario data, seed it and run its spawn table. Bundled templates are
already parsed in the bundle (`name → LoadedGenome`); core only sees plain `TemplateSource`s.
Canonical test world: `testdata/real/Default.zip` (`docs/TESTING_POLICY.md`); other scenarios are
historical fixtures unless a task documents why.

```rust
use bibites_core::world::{TemplateSource, World};
use bibites_io::load_scenario_bundle;

let bundle = load_scenario_bundle("testdata/real/Default.zip")?;
let mut world = World::new(bundle.settings, bundle.scenario, 12345)?;
world.seed_scenario_world();                       // plant biomass (fresh scenario only)
let templates: Vec<TemplateSource> = bundle
    .templates
    .into_iter()
    .map(|(name, loaded)| TemplateSource::new(name, loaded.genome, loaded.meta, loaded.brain))
    .collect();
world.set_templates(templates);
world.spawn_initial_population();                  // each spawn row's minimum/priority
```

`spawn_initial_population` only spawns rows whose `path` resolves to a name in `world.templates`.
Many shipped scenarios bundle no templates (the historical `Ring.zip` fixture has 1 spawn row and 0
bundled templates) and rely on the game's user template folder: the CLI loads those extra templates
(`--templates` or the default user folder) and adds them to `world.templates`; a library user can do
the same, or call `spawn_bibite_from_template` directly when spawning a specific creature.

Settings override (`--settings`/`--independents`): pass `Some(&SimSettings)` as the second argument
to `load_world_contents`/`load_scenario_world_contents`, or apply
`bibites_io::apply_recommended`/`resolve_effective_settings` before constructing the world.

---

## 4. Load a genome/template and spawn

`.bb8` and `.bb8template` both load into `LoadedGenome` (genes + meta + brain, migrated to the
present version). Templates have a longer, stricter load path in io; core only sees
`TemplateSource`.

```rust
use bibites_core::settings::{GrowthAtSpawn, Tagging};
use bibites_core::world::TemplateSource;
use bibites_core::Vec2;

let loaded = bibites_io::load_template("Basic_bibite_0.6.3.1.bb8template")?; // or load_genome(...)
let template = TemplateSource::new("Basic bibite", loaded.genome, loaded.meta, loaded.brain);
world.set_templates(vec![template.clone()]);
let id = world.spawn_bibite_from_template(
    &template,
    GrowthAtSpawn::Adult,
    Tagging::NoTagging,
    "",
    Vec2::ZERO,
)?;
// `id` is a `BibiteId` handle; `world.bibites.view(id)` reads it later.
```

A template file can also be turned into a standalone `BibiteStore` entity without a `World` when
only the creature is needed: `bibites_core::body::BibiteSpawn::new(genome, meta, brain)` +
`BibiteStore::spawn`. Brains can be stepped without a world at all:

```rust
use bibites_core::brain::{Brain, BrainInputs, Input};
use bibites_core::brain::DEFAULT_BRAIN_PERIOD;

let mut brain = Brain::starter();
let mut inputs = BrainInputs::new();
inputs.set(Input::EnergyRatio, 0.5);
let outputs = brain.step(&inputs, DEFAULT_BRAIN_PERIOD); // 33 inputs → 15 outputs
```

`Brain::step_batch(&mut [&mut Brain], &[BrainInputs], period, &mut [BrainOutputs])` is the
bit-identical batch form used by the parallel brain tick.

---

## 5. Inspect a world: iterate, read stats, fingerprints

```rust
use bibites_core::genome::Gene;
use bibites_core::sim::state_fingerprint;

for (id, record) in world.bibites.iter() {          // dense slot order (deterministic)
    let energy = record.body.energy_ratio();
    let diet = record.genome.gene(Gene::Diet);
    let _ = (id, energy, diet);
}

let sample = world.sample_stats();                  // explicit sampling, no hidden cadence
println!(
    "bibites={} eggs={} plant_n={} mean_energy={}",
    sample.counts.bibites,
    sample.counts.eggs,
    sample.pellets.plant_n,
    sample.vitals.energy_mean,
);

let a = state_fingerprint(&world);                  // deterministic digest of live state
world.run(1);
let b = state_fingerprint(&world);
assert_ne!(a, b);
```

For a cheap passive read, `world.bibites.view(id)` returns a copied `BibiteView` (pose, size,
health ratio, colour) without borrowing the record. `BibiteStore::genome`/`brain`/`senses` return
immutable references; mutation of an entity goes through the store (`get_mut`) or the world
services, never through `PhysicsBody`.

`SimStats` accumulates births/deaths and the latest `StatsSample`; the sample groups are
`counts`, `pellets`, `vitals`, `generation`, `species`, `brain`, `genes`, `lifecycle`, `behavior`,
`body`, `tags`, `metrics`. `sample_stats()` is cheap (no allocation plan) and safe to call every
tick when driving a UI/CLI.

---

## 6. Run a parity capture (CLI)

Captures are a CLI concern; the engine just exposes a deterministic `tick` plus digests.

```powershell
# canonical world (docs/TESTING_POLICY.md): run 600 ticks and write both a save and a parity capture
cargo run --release -p bibites-cli -- run testdata/real/Default.zip --ticks 600 --seed 12345 `
    --save target/default_rust.zip --capture target/default_capture

# machine-readable final stats (includes state_fingerprint)
cargo run --release -p bibites-cli -- run testdata/real/Default.zip --ticks 600 --stats json

# benchmark + file inspection
cargo run --release -p bibites-cli -- bench --bibites 2000 --ticks 600 --threads 8 --json
cargo run --release -p bibites-cli -- info testdata/real/Default.zip
cargo run --release -p bibites-cli -- check testdata/real/Default.zip

# historical fixture examples (opt-in; never the default baseline)
# cargo run --release -p bibites-cli -- run testdata/real/Ring.zip --ticks 300 --seed 12345
# cargo run --release -p bibites-cli -- run testdata/real/saves/lol.zip --ticks 300 --seed 12345
```

The capture schema and the game-side harness are documented in [`PARITY.md`](PARITY.md) and
`tools/parity/README.md`; `tools/parity/compare.py` diffs a game capture against a Rust capture
with the `exact`/`default`/`loose` profiles.

Optional viewer (milestone 2, separate binary): `cargo run --release -p bibites-view -- <world.zip>`.

---

## 7. Key types at a glance

### `bibites-core`

| Module | Main public items | Notes |
|---|---|---|
| `arena` | `Arena<T>`, `Handle` | generational arena; stale handles never alias new values |
| `clock` | `FixedClock`, `DEFAULT_SIM_TPS` | fixed timestep; default 30 TPS, overridden by settings |
| `error` | `Error`, `Result<T>` | one error type for core/io |
| `ids` | `BibiteId`, `EggId`, `PelletId`, `PheromoneId`, `ZoneId`, `ToolId`, `SpeciesId`, `LineageId` | typed wrappers over arena handles / persistent u32 ids |
| `math` | `Vec2`, `clamp`, `PI`, `TAU`, Unity-compatible `sigmoid`/`tanh`/`exp`/`sin`/`pow` | transcendentals go f64→f32 (Unity/Mono semantics) |
| `rng` | `Rng` (`from_seed`, `derive`, `fork`, `f32`, `usize_below`, `normal`, `f64`, `shuffle`) | xoshiro256\*\*, SplitMix64 seeding |
| `settings` | `SimSettings`, `ScenarioSettings`, `IndependentSettings`, `MaterialSettings`, `ZoneSettings`, `BibiteSpawn` rows, `SettingsChanger`, enums | exact captured defaults/ranges; `SimSettings::game_defaults()` |
| `genome` | `Genome`, `Gene`, `GenomeMeta`, `MutationFactor`, `GenomeSettings`, `GENE_DEFS` | 35 genes; derived formulas; `Genome::child` mutation |
| `brain` | `Brain`, `BrainInputs`, `BrainOutputs`, `Input`, `Output`, `NodeType`, `NodeRecord`, `SynapseRecord`, `WorldSenseSource`, `InternalSenseSource`, usage masks | CSR plan; `step`/`step_batch`; no RNG |
| `species` | `SpeciesRegistry`, `TemplateSnapshot`, `SpeciesRecord`, `LineageRecordIo` | speciation/lineage bookkeeping |
| `body` | `BibiteStore`, `BibiteRecord`, `BibiteView`, `BibiteSpawn`, `BibiteSave`, `EggStore`, `EggSave`, `BodyState`, `BodyParams`, `BodySettings`, `CollisionSign` | SoA-ish record store; organs live in `organs` |
| `organs` | organ state machines/constants: stomach, mouth, propulsion, fat, armour, egg layer, clock, pheromone, growth | pure math + per-tick updates |
| `physics` | `step_physics`, `PhysicsBody`, `Collider`, `ContactEvent`, `PhysicsScratch`, layer constants | custom deterministic solver (not Box2D, D1/D2) |
| `pellet` | `PelletStore`, `Matter`, `MaterialProperties`, `MaterialKind`, `PelletSpawn`, `PelletCounts` | chunked SoA + free list; `change_amount` is the only amount choke point |
| `pheromone` | `PheromoneStore`, emission spec | 0.25 s decay quanta, negative strengths allowed |
| `senses` | `SenseOutputs`, `compute_fov`, `compute_phero`, `FovMask`, `FovParams` | exact FOV weights/phero sums; deterministic query order |
| `zone` | `ZoneStore`, `Zone`, `ZoneConfig`, `ZoneGlobals` | biomass production, ring/rect geometry, movement |
| `tools` | `ToolStore`, `RadTower`, `PheroTower`, `ColorKiller`, `RedDeathBloom`, `TowerBase` | world tools and their cadences |
| `world` | `World`, `WorldContents`, `WorldScene`, `ScenarioData`, `ScenarioData::empty/with_default_zone_group`, `TemplateSource`, `WorldBounds`, `SpatialHash`, `QueryScratch`, `QueryResult` | the container + tick driver + core-side save records |
| `sim` | `SimStats`, `StatsSample`, `Phase`, `PhaseTimings`, `ThreadPool`, `ParallelExecutor`, `state_fingerprint` | scheduling/timing/stats |

### `bibites-io`

| Item | Purpose |
|---|---|
| `load_genome` / `load_template` / `load_template_tolerant` / `from_json` | `.bb8` / `.bb8template` → `LoadedGenome` (with migrations) |
| `genome_to_json` / `save_genome` / `save_template` / `template_to_json` | write `.bb8` / `.bb8template` |
| `load_settings_file`, `parse_settings_json`, `parse_scenario_settings`, `resolve_effective_settings`, `apply_recommended`, `serialize_save_settings`, `serialize_scenario_settings`, `write_settings_file` | `.bb8settings` scopes/precedence/emission |
| `load_scenario_bundle`, `write_scenario_bundle`, `load_save_bundle`, `write_save_bundle` | raw bundle access (`SaveBundle`, `ScenarioBundle`, `ScenarioInfo`, parsed templates) |
| `load_world_contents`, `load_scenario_world_contents`, `save_world_contents`, `world_contents_from_save`, `save_bundle_from_contents`, `scenario_world_contents` | bundle ↔ core `WorldContents` |
| `inspect`, `LoadedGenome`, `InspectSummary`, `SourceShape`, `Version`, `present_version` | CLI/inspection helpers |

---

## 8. Ownership, lifetime and handle validity

* **`World` owns everything**: stores, spatial hashes, RNG streams, scratch buffers, stats, species
  registry, tools. There is no global/static simulation state; two `World`s are fully independent
  (and `Send`-able as a whole).
* **Entity identity is generational**, not an index: ids come from `Arena<T>`/`ids.rs`. Removing an
  entity invalidates its handle forever; `BibiteStore::get(stale_id)` returns `None`. Iteration is
  dense and deterministic (slot order); never cache an id across a tick without re-checking
  `is_live`-style lookups.
* **Stores are plain data with public accessors.** `BibiteRecord` fields are public for io/parity
  tooling, but simulation code mutates them through the store/tick APIs so bookkeeping (spatial
  hash, counters, species) stays consistent.
* **Pellets/pheromones are slot+generation free-list stores**; amounts change only through
  `PelletStore::change_amount` (counters, zone ownership, decay registration, hash). This is the
  documented choke point; direct field writes would corrupt stats.
* **Brains own their `Vec`s.** A `Brain` can be cloned/stepped independently of a world; the body
  store keeps one per bibite. Brains draw no RNG, which is why batches are order-independent.
* **`World::from_contents`** is the load path: it rebuilds stores, ids, hashes and `simulated_time`
  from `WorldContents`. It does **not** seed a scenario (a save already has its pellets) and does not
  run a tick.
* **`World::to_contents`** snapshots everything io needs for a save; `data.bin`/species are carried
  opaquely/typed as per the format spec.

## 9. Determinism contract

A run is fully determined by `(seed, scenario, settings, tick count)` (`ARCHITECTURE.md` §5):

* **One RNG type.** `Rng::from_seed(seed)` for the world; `Rng::derive(seed, domain, entity)` for
  per-entity/per-domain streams; `Rng::fork(stream)` for sub-streams. No `std::time`, no
  thread-local/global randomness, no hash-map iteration order in state-affecting code.
* **Floats:** `f32` for all simulation state; `f64` only for reporting sums/timers. Brain
  activations evaluate in `f64` and truncate to `f32` (Unity/Mono `Mathf.*` semantics — this fixed a
  real 1-ULP parity bug); genome-derived formulas use `f32` (`powf`/`sqrt`) and are validated by the
  parity/braincheck harness. Do not "unify" these without a capture.
* **Order:** sensing/brain/organs/physics run in a fixed order (`sim::Phase`); ordered reductions use
  slot order. Query results are sorted/stable where float sums consume them.
* **Threads must not change results.** `World::set_threads(n)` (`0` = auto) configures the internal
  pool; serial and parallel runs produce the same `state_fingerprint`. This is enforced by tests
  (`world::tests::parallel_sensing_is_bit_identical_to_serial`,
  `body::tests::parallel_and_serial_brain_batches_are_bit_identical`).
* **Same seed ⇒ same state:** `sim::state_fingerprint(&world)` is the cheap digest used by the CLI
  (`--stats json`) and tests to assert reproducibility.
* **Mutation outcomes are deterministic for us but only statistically faithful to the game**
  (`DEVIATIONS.md` D4/D5): the game's `System.Random`/`UnityEngine.Random` are unseeded.

## 10. Settings boundary

`SimSettings` is the single effective-settings aggregate; everything reads it at the point of use
(settings animators may change values mid-run):

```rust
let s = bibites_core::settings::SimSettings::game_defaults();
let dt = s.fixed_dt();            // 1 / sim_tps
let brain_period = s.brain_period(); // brain_tps / sim_tps
assert_eq!((s.sim_tps, s.brain_tps), (30, 2));
```

Never hardcode TPS/vision/decay rates: they are serialized per file (`independents`/`recommended`)
and scenario-overridable. Materials are editable and must be read from `s.materials`; only the
captured defaults live in code. User-level settings that are not part of a scenario/save (e.g.
`AllowEasterEggs`) are out of scope for the headless engine (`DEVIATIONS.md` D12).

## 11. File-format boundary

```
.bb8 / .bb8template ──bibites_io::load_*──► LoadedGenome {genome, meta, brain, template}
                                             └► core::world::TemplateSource ─► World::spawn_* 

save .zip / scenario .zip ──bibites_io::load_*_world_contents──► core::world::WorldContents
                                             └► World::from_contents ─► World ─► tick/run
World ─► World::to_contents ─► WorldContents ──bibites_io::save_world_contents──► save .zip
```

* Unknown JSON fields are preserved by io and re-emitted where the game tolerates it; core never
  sees serde types.
* `data.bin` is an opaque `Vec<u8>` pass-through (`DEVIATIONS.md` D11).
* Pre-0.6 settings migrations and 0.6.4a1 folder saves are explicitly out of scope for M1
  (`docs/spec/04-formats-settings.md` §0.1/§8.5).
* Exact round-trip rules: `docs/spec/04-formats-settings.md` §9; real-game acceptance:
  `docs/ARCHITECTURE.md` §8.

## 12. Errors, panics and validations

* Fallible constructors return `bibites_core::Result<T>` (`Error::invalid_data`/`unsupported`):
  `World::new`, `World::from_contents`, `Brain::from_records`, `BibiteStore::spawn`,
  `World::spawn_bibite_from_template`, all io loaders.
* Infalible helpers (`World::tick`, `StatsSample` accessors) assume a validated world; malformed
  files are rejected at the io/constructor boundary, not mid-tick.
* `Brain` has capacity caps (`MAX_NODES`, `MAX_SYNAPSES`) and `Brain::validate`; the io template path
  repairs what the game tolerates (`DEVIATIONS.md` "Template tolerance").
* No `unsafe` anywhere (`#![forbid(unsafe_code)]` in core and io).

## 13. Examples and verification

The runnable tour is `crates/bibites-core/examples/api_tour.rs`:

```powershell
cargo run -p bibites-core --example api_tour
cargo test -p bibites-core --example api_tour
cargo clippy -p bibites-core --example api_tour -- -D warnings
```

It exercises RNG streams, arena handle invalidation, starter genome/brain stepping, world
construction/seeding/template spawn, per-entity iteration, stats sampling and the determinism
fingerprint. Its printed output (on the 2026-10-30 tree) is:

```
rng: seq[0..2]=[1546998764402558742, 6990951692964543102] u=0.7006 normal=-0.5001
arena: live=1 stale_handle_detected=true
genome: size_ratio=1 growth_at_mature=1.788 | brain: nodes=48 synapses=3 accel=0.4248 upkeep=0.000750 E/s
world: t=10.00s bibites=1 eggs=0 plant_count=92 plant_biomass=19913.8 E mean_energy=170.474 E
world: state_fingerprint=0xc6878742b5ab2c81 (deterministic for seed 9999)
api_tour: all checks passed
```

The io snippets in §3/§4 were compiled against the real crates in a scratch harness (same calls as
`crates/bibites-cli/src/engine.rs`) and the capture commands were taken from the CLI's own argument
surface (`crates/bibites-cli/src/args.rs`).

## 14. Rustdoc status and doc patches (2026-10-30)

`cargo doc --workspace --no-deps` **succeeds (exit 0)** and emits **0 `missing_docs` warnings**
(all three crates enable `#![warn(missing_docs)]`). The only remaining defect is a set of
`unresolved link` warnings for module-level doc comments that name items without a path; they are
docs-only (pages build, links are just not hyperlinked).

Status of the mechanical fix (fully-qualifying targets, e.g. `[`Brain`]` → `[`crate::brain::Brain`]`,
`[`MaterialProperties`]` → `[`crate::pellet::MaterialProperties`]`):

| Step | Unresolved links |
|---|---|
| Baseline (before docs work) | **65** |
| Commit `b823b8d` — 13 non-owner-active files + refreshed crate overviews | **22** |
| Follow-up — `genome.rs` + `species.rs` (brain-dev files, owner idle) | **15** |
| Remaining | **15**, all in `body.rs` (3), `sim.rs` (3), `world.rs` (9) — deferred until those owners hand back, per Lead sequencing |

The complete fix is verified: applying the same transform to those three files on a scratch copy of
`bibites-core` yields `cargo doc` exit 0 and **0 unresolved links**. A final commit lands them when
body-dev/world-dev hand back.

Already applied in the commits above: fully-qualified links in brain/physics/senses/settings/tools/
zone/pellet/genome/species (core), io `lib`/`bundle`/`migrate`, cli `capture`/`parity`; plus the three
stale crate-overview texts (core `lib.rs` "default 40 TPS" → 30 and the "empty placeholders" sentence;
io `lib.rs` "Phase I1 …" overview). One optional item remains as a proposal: a short `//!` module
overview for `bibites-cli` pointing at this file (CLI source is Lead-owned; documented here instead).

Remaining undocumented public items after those patches: none flagged by `missing_docs`; deeper
module-level prose (algorithm notes per module) is a nice-to-have, not a gap.

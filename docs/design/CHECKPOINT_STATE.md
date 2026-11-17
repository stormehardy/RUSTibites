# Checkpoint state design — bit-identical resume (task-75)

Status: **closed for the 120 / 1k / 2k / 5k / 10k / 100k scale (2026-10-30)** —
implemented by world/io/CLI (P1/P2/P3), not just design. Default canonical
verification: **1k / 2k / 5k / 10k bit-exact** for both 2× and 4× chunked resume,
**100k single == 50×2000 chunked** (fingerprint `8688404363908086484`), and real
saves restore byte-identically after tasks 124–126. Only the benign lazy egg
`plan_dirty` flag remains (derived, not encoded); details below. Fingerprints are recomputed from the live state, so body/behavior commits change them; the values here are the current-tree 2026-11-01 guard results (earlier 11492540061262829588 / 1423792448087571940 were pre-body-fix runs).
**100k chunked root cause (fixed):** dead-slot generation counters in
`PelletStore`/`PheromoneStore` were skipped by the checkpoint codec, so the next
pellet allocation after a resume reused a slot with a different `PelletId`
generation; the first post-restore production tick (Default tick +188) then
diverged in zone/pellet/hash state while body/brain/egg/spawner/species matched.
Both codecs now encode generations for every slot (alive and free); the 100k
single-vs-chunked fingerprints match. Regression:
`checkpoint_preserves_dead_slot_generations` (pellet), world long chunked test,
and Default CLI 50×2000.
Evidence and repro: `docs/EVOLUTION_RESULTS.md` §"Resume/continuation verification".
**Canonical test world:** `testdata/real/Default.zip` (`docs/TESTING_POLICY.md`); the Ring/lol saves
below are historical fixtures for explicit resume regressions — new verification starts from Default.zip.

## Problem

`rustibites run save.zip --save ckpt.zip` then `run ckpt.zip` continues the
*visible* world but **not the deterministic stream**: a single 1,000-tick run
fingerprints `7816395688157925020`, the same run done as 2 × 500 ticks with a
save/reload in between fingerprints `11292368019058734230`
(`tools/evolve/run_experiment.ps1 -Verify`). Consequently chunked long-run
experiments and resume-based benchmark claims are not reproducible today.

Root cause: the game save format (and therefore `WorldContents`) only holds the
persistent game state. Everything that makes the next tick unique — RNG stream
positions, clock tick, cadence phases, runtime-only caches/queues, arena/ID
allocator layout — is recomputed from defaults/seed on load.

## What the current save path persists (baseline)

`bibites_io::load_world_contents` → `WorldContents` → `World::from_contents`:

* settings (`SimSettings`), seed, `WorldScene` (name, version,
  `simulatedTime`, tags, spawn rows), zones (`Vec<ZoneSettings>` — definitions,
  not runtime), zone groups, settings changers, tools (towers/colour killers),
  pellets (`PelletRecord`: material/amount/pos/owner index/held/decay state),
  pheromones (pos/heading/strength/dissipate/owner), bibites (`BibiteSave`),
  eggs (`EggSave`), species lineage (`LineageRecordIo`), opaque `data.bin`.
* `BibiteSave` covers body/organs/clock/genome/meta/brain (including live node
  `Value`/`LastInput`/`LastOutput`), stomach, mouth counters, egg-layer progress,
  fat, pheromone progress, propulsion totals; `EggSave` covers genome/brain/
  energy/hatch progress/timing.

Everything the game serializer does not need for a *valid loaded world* is
dropped. That is correct for game compatibility but insufficient for
checkpointing.

## Missing mutable state catalogue

### A. RNG streams (`rng.rs`)

`Rng` is xoshiro256** with `{ seed: u64, state: [u64; 4] }`. `fork`/`derive` are
**position-independent from a parent seed**, so derived streams are only a
problem while they hold long-lived mutable state. Audited live streams:

| stream | owner | why it diverges after load |
|---|---|---|
| `World.rng` (`derive(seed, WORLD, 0)`) | `world.rs` | primary stream; consumed every tick (bites, mutations, spawner, red death, zone-ish ops). Loaded world recreates it from seed → draw counter reset. **Must save `state[4]` (seed is already known).** |
| `ZoneData.rng` (private, `rng.rs` per zone) | `zone.rs` | consumed on pellet spawn per zone; recreated from `world_seed + zone_id` on load → reset. Must save per-zone state (zones are restored in order; key by zone index). |
| `SpeciesRegistry.name_rng` (private) | `species.rs` | consumed when a new species name is generated; registry restored from lineage records recreates it from its seed → name sequence diverges after resume (and `next_id`). |
| per-bibite streams | none persistent today | body/brains consume the shared `World.rng` (`BibiteTickCtx.rng`); no per-entity RNG state needed. Re-audit if that changes. |
| test-only forks | tests | irrelevant (recreated per test). |

### B. Clock, time and cadence (`clock.rs`, `world.rs`, stores)

| state | where | note |
|---|---|---|
| `FixedClock { tps, dt, tick }` | `world.clock` | `tick` is **not** in `WorldContents`; resets to 0 on load while `simulated_time` is restored. Must save `tick` and assert `tick == round((simulated_time - base_time)/dt)`. |
| `World.simulated_time: f64` | saved (`scene.simulatedTime`) | keep; must stay consistent with restored clock tick. |
| `SenseThrottle { brain_counter, lookup_counter, sense_counter }` | each `BibiteRecord` | **not** in `BibiteSave`; decides which ticks update senses/brain. Must save per bibite. |
| `DecayThrottle { period, progress }` (private) | `world.decay` | pellet decay cadence phase; not saved (`decay_tps` is in settings). |
| `spawner_accumulator: f32` | `World` | game's 1 s scaled spawner coroutine phase; not saved. |
| settings animators: `animator_progress`, `animator_started[]`, `animator_ended[]` | `World` | runtime progress of `settingsChangers`; definitions are saved, progress is not. |
| zones runtime: `pellet_progress`, `time_since_recount`, `pellet_biomass`, `owned_pellets` order, `vel`, `last_delta`, `target_index`, runtime `pos/radius/half_extents` | `ZoneData` | saved zones are **definitions only**; moving-zone positions, accumulated production and owned-pellet ordering are lost. Must save per-zone runtime state (or reconstruct deterministically and accept a different trajectory for moving zones — not acceptable for bit-identical resume). |
| tools: `RedDeathBloom { progress, safe_radius, ... }`, per-tool runtime (moving towers' pos/vel/phases, phero tower progress, colour-killer timers, tool save ids) | `ToolStore` | only tool *definitions* are saved; must save runtime fields. |
| egg incubation: `Egg.hatch_progress` saved; `is_hatching`, `aborting`, `EggRecord.birth_orientation`, `size_gene` | `EggStore` | verify each against `EggSave`; anything missing must be in sidecar. |
| pellet decay: `fresh_time_remaining`, `rot_amount` saved; pellet **velocity/rotation are intentionally dropped by the game format** | `PelletRecord` | for resume, save the real runtime pellet state (velocity/rotation/ids) in the sidecar; the game-save path keeps its documented quirk. |
| phase timings | `World.phase_timings` | observational only; not needed for simulation equality (exclude or zero on restore). |

### C. Per-bibite runtime-only state (not in `BibiteSave`)

Body/brain runtime fields that affect future ticks but are not serialized by the
game save (see `BibiteRecord` vs `BibiteSave` in `body.rs`):

* `throttle` (see B), `senses` (`SenseOutputs`), `steering`
  (`SteeringInputs`), `brain_outputs` (`BrainOutputs` cache),
  `pending_force: Vec2`, `pending_torque: f32`,
  `mouth_targets[10]`/`n_mouth_targets`, `held_targets[4]`/`n_held_targets`,
  `death_cause`, `is_corpse`, `born`, `is_ready`, `mutation_factor`, `color`
  (if not recomputed from genes), `children` ids (saved by game),
  `brain.plan_dirty`/internal CSR/degree/usage-mask caches (derived but must be
  deterministic after restore).
* Body caches (`derived`, `mouth_derived`, `propulsion_derived`,
  `growth_genes`, geometry) are recomputed on load by `recompute_derived`; they
  should match if the source state matches — but the restore path must run the
  same recomputation order and then be verified by fingerprint.
* `BibiteStore` **arena layout**: `records: Vec<Option<BibiteRecord>>`,
  `generations`, `free`, `dense` order. Loading spawns saves in file order into
  fresh slots; slot ids/generations differ from the pre-save world, and
  `sim::state_fingerprint` includes `handle.slot()/generation()`. Sidecar must
  preserve the arena layout and id mapping (stable game `body.id` maps entity ↔
  slot).
* Same for `EggStore`, `PelletStore`, `PheromoneStore` (arena slots,
  generations, free lists, dense order) and `ToolStore`/`SpeciesRegistry` ids.
* `SpatialHash` is derived, but its internal cell insertion order and free list
  affect query iteration order (sensing accumulation order). Restore-or-canonicalise:
  either save the hash layout (cheap: cells/entries are derived but insertion
  order is deterministic given the same restore order) or make every query sort
  hits canonically. Plan: restore entity arenas first, rebuild the hash in the
  original insertion order (dense/slot order) and add a fingerprint check.
* Physics: the body/physics mirror and broadphase are rebuilt every tick from
  entities; no persistent solver state was found. Restoring entities in the same
  order is sufficient; verify with capture/`state_fingerprint`.

### D. IDs / allocators / counters

* Arena free lists + generations (bibites/eggs/pellets/pheromones) and next
  slots; tool/tower `save_id` allocation; `has_sources` cache.
* `Brain.node_max_innovation` / `connection_max_innovation` (future mutations
  use them; `from_records` derives a value that may differ from the live
  counters).
* `SpeciesRegistry` private state: `active` list **order**, `next_id`,
  `node_max_innovation`, `connection_max_innovation`, `time`, `log_progress`,
  `log_counter`, `n_records`, `max_energy`, `max_count`, `cached_span`,
  `cached_span_active_count`, `name_rng` state. The lineage records are saved,
  the counters/ordering/RNG are not.
* `SimStats`: `births_total`, `deaths_total`, `births_since_sample`,
  `deaths_since_sample`, accumulator sums (behaviour/vitals) and `last` sample —
  needed only for exact *reported* stats, but cheap to include; without them a
  resumed run's first `sample_stats` differs even if the world matches.
* `World.seed`: already restored by the CLI; sidecar must record it and reject
  a mismatch (`--seed` vs sidecar).

### E. Deliberately not checkpointed

Per-tick scratch buffers (`bibite_scratch`, `events`, `zone_circles`,
`query_scratch`, `hatched`, `aborted`, `red_*_acc`, `producer_progress`,
sensing buffers/pools, thread counts): rebuilt/cleared at tick boundaries and
never observed by the simulation across a save point. `phase_timings` is
observational. `data.bin` is game-owned history and must never be rewritten.

## Sidecar format: `<save>.bb8state` (not embedded)

**Do not add members to the game save zip until in-game load compatibility is
verified.** A game save with an extra unknown zip entry may or may not be
ignored by the game's save reader; the safe default is a sibling file:

```
testdata/.../checkpoint_049.zip
testdata/.../checkpoint_049.zip.bb8state     # or <stem>.bb8state
```

The game never sees the sidecar; `rustibites run checkpoint_049.zip` looks for
the sibling automatically and restores it after the normal game-save load. An
explicit `--state-in PATH` / `--state-out PATH` (or `--no-state`) covers
non-standard layouts. Embedding in the zip can be added later behind
`--state-in-zip` once a real game-load test passes (a copy of a save with an
extra member loaded by 0.6.3.1 and re-saved).

Format (core dep-free, manual little-endian; version + checksums so it fails
loudly, never silently mixes with the wrong save):

```
magic           8 bytes  "BB8STATE"
version         u16      = 1
flags           u16      endianness/compression (0 = raw LE)
base_sha256     32 bytes sha256 of the save zip bytes (reject mismatch)
world_seed      u64
settings_hash   [u8; 32] sha256 of the bridge settings JSON (reject mismatch)
tick            u64
simulated_time  f64
section_table   u16 count, then per section: tag u16, offset u32, len u32
sections        ...
crc32/u64       8 bytes  whole-file checksum (or sha256 truncated)
```

Sections (all length-prefixed, deterministic order):

1. `RNG` — world seed/state[4]; number of zone streams; per zone
   `index, state[4]`; species name_rng state.
2. `CLOCK_TIMERS` — clock tick/tps/dt; `spawner_accumulator`;
   `decay.progress/period`; animator progress/started/ended; red-death
   progress/safe radius; tool runtime records (id/pos/vel/timers).
3. `ARENAS` — for bibites/eggs/pellets/pheromones: capacity, generations,
   free list, dense order; plus stable game-id mapping (`body.id`/egg id) so
   the sidecar can be applied to an entity list loaded from the game save.
4. `ENTITY_RUNTIME` — for each bibite slot: throttle counters, senses,
   steering, brain_outputs, pending forces, mouth/held targets, flags
   (`born/is_ready/is_corpse/death_cause`), mutation factor, brain
   innovation counters; for each egg: hatch flags/birth orientation/size;
   for each pellet: velocity/rotation/decay timers; for each pheromone:
   full runtime record.
5. `ZONES` — per-zone runtime (`pos/vel/last_delta/pellet_progress/
   time_since_recount/pellet_biomass/owned_pellets` order); tool runtime.
6. `SPECIES` — active order, next_id, innovation counters, time/log counters,
   cached spans, name_rng already in RNG section.
7. `STATS` — SimStats totals/since-sample/accumulators/last sample.
8. `HASH` (optional, phase 2) — if rebuild order cannot be made canonical.

Unknown sections are skipped (forward compatible); missing sections fail
closed unless `--allow-partial-state`.

## Restore order (must be exact)

1. Load the game save normally (`load_world_contents` → `World::from_contents`);
2. read sidecar, verify magic/version/base sha256/settings hash/world seed;
3. restore **arena + ids** first (create empty stores with the saved arena
   layout), then fill entity/egg/pellet/pheromone records from the game save by
   stable id (or from full records in `ENTITY_RUNTIME`);
4. restore RNG stream states, clock/tick/time, throttles, zone/tool/species
   runtime and counters;
5. rebuild all derived state (`recompute_derived` for every bibite, brain CSR,
   zone derived values, spatial hash in dense/slot order, physics mirror) using
   exactly the same functions as construction;
6. compare `sim::state_fingerprint(&world)` against the fingerprint recorded in
   the sidecar; mismatch is a hard error naming the first differing section.

CLI contract:

* `run input.zip ... --save out.zip` also writes `out.zip.bb8state` (unless
  `--no-state`);
* `run input.zip` auto-loads `input.zip.bb8state` if present; `--state-in` and
  `--state-out` override paths;
* `--seed` must equal the sidecar seed or the command fails (a checkpoint is
  not a fresh run); explicit re-seeding requires `--no-state`;
* game-load compatibility is unchanged because the zip is byte-identical to
  today's save; only the sibling file is extra.

## Phased implementation (hand-offs, each gated by bit-identical tests)

| phase | work | owner/scope |
|---|---|---|
| P0 | This doc + game-compat probe: (a) confirm the game ignores a save zip with one extra member by loading it in 0.6.3.1 via the parity bridge; (b) decide sidecar naming (`<save>.bb8state` default). | brain-dev (design), Lead/game run |
| P1 | `Rng::state()/set_state()/reset_state()` + `FixedClock::tick/set_tick` accessors and a core `SimSnapshot` (`rng.rs`, `sim.rs`) with manual LE encode/decode; unit tests. | brain-dev (`rng.rs`, `sim.rs` scope) |
| P2 | `World::capture_checkpoint_state` / `restore_checkpoint_state` (world.rs): arenas/ids, runtime entity/zone/tool/species/stats/clock; deterministic derived rebuild. | world-dev (world.rs) with brain-dev pair |
| P3 | io sidecar: `write/read_checkpoint_state(path, &SimSnapshot)` (schema above), sha/version checks, corrupt/mismatch tests; keep `WorldContents` game-save path untouched. | fmt-dev (bibites-io) |
| P4 | CLI: `--save` writes sidecar, `run` auto-loads, `--state-in/--state-out/--no-state`, mismatch errors. | Lead (bibites-cli) |
| P5 | Verification: `tools/evolve/run_experiment.ps1 -Verify` must print `RESUME OK`; chunked vs single `state_fingerprint` equal on **Default.zip** (canonical) at 1k/10k ticks, with Ring save + lol save kept as historical-fixture regressions; canonical Default capture tick-stream byte comparison; Default parity exit 0 (Ring default parity retained as historical); game-load round-trip of the save (without sidecar) unaffected. | brain-dev + CI |

## Acceptance tests

1. `World` single run 1,000 ticks vs 2 × 500 with capture+restore →
   `state_fingerprint` equal and `ticks.jsonl` byte-identical.
2. Same on **Default.zip** (canonical) at 1k and 10k ticks; keep `lol.zip` (1k and 10k) and
   `ring_parity.zip` as historical-fixture regressions.
3. Sidecar rejected with clear error when base sha256/settings hash/seed/version
   mismatch or checksum corrupt; `--no-state` ignores it.
4. Save zip bytes remain game-loadable and **unchanged** when sidecar is used
   (normal io round-trip tests + real game load).
5. `tools/evolve/run_experiment.ps1 -Verify` exits 0; long chunked run matches a
   single-process run exactly.

## Open questions / risks

* Arena/ID reconstruction is the largest piece: `BibiteStore`/`EggStore`/
  pellet/pheromone stores need a restore constructor that preserves slots,
  generations and free lists (new `pub(crate)` APIs; coordinate world-dev).
* The game save holds canonical entity data; the sidecar should store *deltas*
  (runtime-only fields) keyed by stable ids where possible, but a full record is
  simpler and self-contained (~few hundred bytes/entity). Pick full records for
  v1 and shrink later.
* Moving zones/tools: runtime positions are currently not in the game save at
  all; confirm whether the game itself persists them (if not, a checkpoint can
  only equal a Rust single run, which is the acceptance criterion — not a game
  resume).
* `SpatialHash` insertion order: rebuild in dense order must match the original
  construction order; if not, save the hash or sort query results canonically
  (sensing order is behaviourally visible through f32 summation order).
* `SimStats` restoration matters for reported stats but not simulation
  state_fingerprint; include anyway so resumed captures/reports match.
* Sidecar size: full entity runtime + arenas for 100k bibites is tens of MB;
  streaming/section compression can come later (v1 raw, measured).

## P1 implementation status (landed 2026-10-30)

P1 is implemented in my scope (`crates/bibites-core/src/rng.rs`,
`crates/bibites-core/src/sim.rs`), dependency-free, with 8 new tests. Core suite:
380 passed / 8 ignored at this revision.

### Exact APIs for world/io/CLI wiring

`rng.rs`:

```rust
pub struct RngState { pub seed: u64, pub words: [u64; 4] }
impl RngState { pub const fn new(seed: u64, words: [u64;4]) -> Self;
                pub fn is_valid(&self) -> bool; } // words != all-zero
impl Rng {
    pub fn state(&self) -> RngState;                 // capture
    pub fn from_state(state: RngState) -> Option<Self>; // restore (None = invalid)
    pub fn set_state(&mut self, state: RngState) -> bool; // in-place restore
    pub fn seed(&self) -> u64;                        // already existed
}
```

Semantics: `seed` is preserved so `fork`/`derive` stay position-independent;
only the four xoshiro words track consumption. All-zero state is rejected
(`SnapshotError::InvalidRngState`).

`sim.rs` (`SimSnapshot` and friends are public):

```rust
pub const SIM_SNAPSHOT_MAGIC: [u8; 8] = *b"RBSNAP01";
pub const SIM_SNAPSHOT_VERSION: u16 = 1;
pub enum SnapshotError { Truncated, BadMagic, UnsupportedVersion(u16),
    UnsupportedFlags(u16), TrailingData, ChecksumMismatch,
    LimitExceeded(&'static str), InvalidBool, InvalidRngState, InvalidSimTps }
pub struct ExtensionSection { pub tag: u16, pub data: Vec<u8> }
pub struct SimSnapshot {
    pub tick: u64, pub sim_tps: u32, pub simulated_time: f64,
    pub rng: RngState,
    pub zone_rngs: Vec<(u32, RngState)>,     // ascending zone index
    pub species_name_rng: Option<RngState>,
    pub spawner_accumulator: f32,
    pub decay_period: u32, pub decay_progress: u32,
    pub animator_progress: f32,
    pub animator_started: Vec<bool>, pub animator_ended: Vec<bool>,
    pub red_death_progress: f32, pub red_death_safe_radius: f32,
    pub extensions: Vec<ExtensionSection>,
}
impl SimSnapshot {
    pub fn new(tick: u64, sim_tps: u32, simulated_time: f64, rng: RngState) -> Self;
    pub fn push_extension(&mut self, tag: u16, data: Vec<u8>) -> Result<(), SnapshotError>;
    pub fn extension(&self, tag: u16) -> Option<&[u8]>;
    pub fn restore_clock(&self) -> Result<FixedClock, SnapshotError>; // new(tps)+advance(tick)
    pub fn encode(&self) -> Result<Vec<u8>, SnapshotError>;
    pub fn decode(bytes: &[u8]) -> Result<Self, SnapshotError>;
}
impl Default for SimSnapshot;  // tick 0, 30 TPS, seed-0 world RNG, decay 2
impl PartialEq for SimSnapshot; // bitwise floats (NaN/-0.0 are checkpoint identity)
```

Encoding layout (all LE): 8-byte magic, `u16` version, `u16` flags (0),
`u32` payload length, payload, `u64` FNV-1a payload checksum. Payload fields in
the struct order above with explicit lengths; booleans as bytes; extensions as
`tag u16 + len u32 + bytes`. Limits: 100k zone streams, 1M flags, 1,024
extensions, 64 MiB/direct extension, 8 MiB chunk payloads, 256 MiB total payload
(chunked sections use `0x0200 | tag` with parent/index/total headers and are
reassembled on read). Decode fails closed
(truncation/version/flags/checksum/trailing/bad bool/zero RNG), never panics;
every strict prefix of a valid encoding is rejected in tests. Floats are raw
bit patterns (NaN/-0.0 preserved), and equality is bitwise.

Reserved extension tags for phase 2 (world may use these; core ignores them):
`0x0101` arenas/ids, `0x0102` per-entity runtime deltas, `0x0103` zone/tool
runtime, `0x0104` species counters/active order, `0x0105` `SimStats`, `0x0106`
spatial-hash layout (only if rebuild order cannot be canonicalised). The world
must append them deterministically (fixed order, ascending zone/entity ids) so
`encode` stays byte-deterministic.

### Notes for world-dev (P2)

* fill `snapshot.rng = world.rng.state()`, `zone_rngs` from each
  `ZoneData.rng` (sort by zone index), `species_name_rng` from the registry
  accessor, `tick = world.clock.tick()`, `sim_tps = world.clock.tps()`,
  `simulated_time`, `spawner_accumulator`, `decay_period/progress`,
  animator flags, red-death progress/safe radius;
* restore: `world.rng.set_state(snapshot.rng)`, per-zone `set_state`, clock via
  `snapshot.restore_clock()` (or keep world's clock and verify equal tick), then
  compare `sim::state_fingerprint` against the value recorded by the sidecar;
* private state accessors needed: zone RNG, species `name_rng`/counters, decay,
  animators, red death, stats; arena restore constructors as in the plan.

### Notes for fmt-dev (P3)

Wrap the core payload with the sidecar header from the planning section:
`<save>.bb8state` = sidecar magic/version + base-save sha256 + settings hash +
`SimSnapshot::encode()` bytes (or store the core payload inside a section). Use
`SnapshotError` for core corruption and a distinct io error for sidecar
mismatches. Do not alter the game zip.

### P0 game-compat probe (static, empirical pending)

Checked the decompiled 0.6.3.1 loader (`Here/.../SaveSystem.cs`): the load path
reads named entries (`scenario.info`, `speciesData.json`, `vars.bb8scene`,
`pellets`, `pheromones`, `data.bin`, `settings`) and only enumerates zip entries
to find `bibites*` / `egg*` templates; an unknown extra root member is not read.
So a save with one extra member *should* still load, but this is **static
evidence, not a live 0.6.3.1 load test**. The default remains the sibling
`<save>.bb8state`; embedding can be re-evaluated after Lead/parity runs the
actual game-load probe (copy Ring save + extra member → launch game → resave).

## P2 brain runtime checkpoint (task-108, landed 2026-10-30)

`crates/bibites-core/src/brain.rs` now exposes the per-entity brain state that
the game save does not round-trip bit-identically. This is the brain-owned piece
of the P2 entity-runtime section (`SimSnapshot` extension tag **0x0102**); the
body store owns the surrounding per-bibite record/cadence fields.

```rust
pub struct BrainCheckpoint {           // brain.rs
    pub node_count: u32,
    pub synapse_count: u32,
    pub topology_hash: u64,            // FNV-1a over immutable topology
    pub value: Vec<f32>,               // Node.Value, per node
    pub last_output: Vec<f32>,         // Node.LastOutput
    pub last_input: Vec<f32>,          // Node.LastInput
    pub node_max_innovation: i64,
    pub connection_max_innovation: i64,
    pub herding_enabled: bool,
    pub plan_dirty: bool,
}
impl Brain {
    pub fn capture_checkpoint(&self) -> BrainCheckpoint;
    pub fn restore_checkpoint(&mut self, cp: BrainCheckpoint) -> crate::error::Result<()>;
}
impl BrainCheckpoint {
    pub fn encode(&self) -> Vec<u8>;                    // deterministic LE, magic "BRCHK001" v1
    pub fn decode(&[u8]) -> crate::error::Result<Self>; // limits/truncation/checksum
}
```

Format: 8-byte magic `BRCHK001`, u16 version 1, u16 flags 0, u32 payload length,
payload (`node_count`, `synapse_count`, `topology_hash`, plan/herding flags,
innovation counters, then `value[]`, `last_output[]`, `last_input[]` as raw f32
bits), u64 FNV-1a checksum. Limits: `MAX_NODES` 4096, `MAX_SYNAPSES` 65,536;
decode verifies exact payload/outer lengths. Encoding is byte-deterministic.

Restore contract: validates node/synapse counts, vector lengths and the topology
fingerprint **before mutating** (wrong brain/corrupt payload returns `Err` and
leaves the brain untouched), copies runtime arrays/counters/flags, rebuilds the
derived CSR plan, then re-applies the captured `plan_dirty` flag (a dirty plan is
rebuilt on the next `step`). Cadence counters (`SenseThrottle`) and RNG are *not*
brain state: body-store checkpoint and `SimSnapshot.rng` restore them.

Tests (`brain::brain_checkpoint_tests`, 4 passing): capture→encode→decode→restore
equality; step-output bit equality after restore (including node/synapse records);
topology-mismatch rejection without mutation; `plan_dirty`/herding preservation;
magic/truncation/trailing/checksum/length rejection. No runtime behavior changed,
so braincheck fixtures stay 0-ULP and Ring parity is unaffected.

Hand-off names for P2/P4: in the entity-runtime section 0x0102, for each bibite
store `brain_checkpoint.encode()` bytes alongside the body-owned fields; on
restore, call `brain.restore_checkpoint(BrainCheckpoint::decode(bytes)?)` after
the game-save brain topology is loaded (and before the first step). `0x0104`
remains the species registry checkpoint; there is no separate brain extension
tag.

## P2 validation (world-dev, committed 318f82e)

`World::capture_checkpoint_state(&self) -> SimSnapshot` and
`World::restore_checkpoint_state(&mut self, &SimSnapshot) -> Result<()>` implement
this design; per-store codecs live in body/zone/pellet/pheromone/brain/species.
Coverage: base control state; `0x0101` bibite arena + full runtime (body codec),
`0x0102` egg arena, `0x010C` brain runtime, `0x0108`/`0x0109` pellet/pheromone
arenas, `0x0103` zones, `0x010A` tools, `0x0104` species registry, `0x0105`
`SimStats`, `0x0106` fresh-spawner **started** flag + per-row progress/species +
accumulator, `0x010B` exact spatial-hash layout, `0x0107` recorded fingerprint
guard. Restore order: game save → arenas/ids → runtime → RNG/clock → derived/hash
rebuild → fingerprint check; runtime-only, never written into game saves.
`BibiteCheckpointRecord.fov_seen` (body `FovSeenCache`: cached FOV pellet/bibite
ids) is captured/restored with the bibite arena, so a resumed world keeps the
lookup cache; a game-save load starts with an empty cache until the first FOV
lookup tick (task-128 game-faithful acquisition cadence).

Evidence (canonical Default, seed 12345): CLI 200+200 mid-spawn resume == single
400, fingerprint `2564713547813341729`; CLI 300+300 == single 600 (Lead). Core
tests: populated 120-tick round-trip + 10 post-restore, empty-world control state,
spawner chunked resume (single vs 2 chunks, 2 seeds); 411 core tests pass.
Sidecar size: Default 200 ticks = 164,296 bytes, Default 300 = 168,942 bytes.
Omissions: `created_meat`/red-death refused-spawn energy remain out of the
conservation identity (`terms_missing`); the last `SimStats` sample is recomputed;
spawn templates are loaded by the CLI rather than stored in the sidecar.

Known low-priority residual: a synthetic 120+120 split can differ ~1e-7 in one
body's velocity/rotation on the first tick after restore while every captured
body/pellet/egg/pheromone/zone/hash/RNG/species field is equal at the restore
point and the fingerprint guard matches. Isolated to physics contact ordering /
solver scratch (`BibiteScratch.physics` is per-step scratch); Default CLI resume
(200/300/600) is bit-exact, so this is synthetic-only.



## Task-115 independent scale/long-run validation (brain-dev, 2026-10-30)

Environment: release CLI for Default; a temporary external core probe for synthetic
worlds (same construction as `bench`: world size `2000*sqrt(N/2000)`, default
zones, starter adults on deterministic `rng.fork` positions, seed `0xC0FFEE`).
No game launches; no body/world edits.

### Default.zip chunked resume

| run | single final fp | chunk final fp | equal |
|---|---|---|---|
| 400 ticks, 2 × 200 | `2564713547813341729` | `2564713547813341729` | yes |
| 400 ticks, 4 × 100 | `2564713547813341729` | `2564713547813341729` | yes |
| 600 ticks, 2 × 300 | `10076096231995370419` | `10076096231995370419` | yes |
| 600 ticks, 4 × 150 | `10076096231995370419` | `10076096231995370419` | yes |
| 10,000 ticks, 5 × 2,000 | `1750137736440958460` | `1750137736440958460` | yes |
| 36,000 ticks, 18 × 2,000 | `9113407757480872428` | `9113407757480872428` | yes |
| 100,000 ticks, 50 × 2,000 | `8688404363908086484` | `8688404363908086484` | yes |

Chunk fingerprints (2× / 4×) matched the single run at every resume point; sidecars
151,275–177,529 bytes for these runs. Rejection paths all fail closed with clear
errors: wrong `--seed` (`checkpoint seed mismatch`), bad magic (`checkpoint header:
bad magic`), truncated sidecar (`length/trailing data mismatch`), sidecar next to a
different save (`checkpoint base mismatch`); `--no-state` ignores a corrupt sidecar
and runs. Measured overhead at 400 ticks (10 reps): save+sidecar 528 ms vs save
without state 302 ms (≈226 ms sidecar capture+encode+write of 177 KB; startup noise
included); read-with-sidecar 272 ms vs `--no-state` 280 ms (read/restore is below
process-start/template-load noise).

### Synthetic residual: first divergence is at restore, not physics scratch

For `N=120`, `120 ticks`, `2 × 60`, all captured checkpoints (body/egg/pellet/
pheromone/zone/RNG/species) and `state_fingerprint` are equal immediately after
restore, but the **internal `BibiteRecord.rot` already differs**:

`slot BibiteId(2v0) rot: src=1.927486062 rst=1.927486181 abs=1.192e-7 ulp=1`

Then after one tick the first difference is still rot (1 ULP); after two ticks it
is `vel.y` (1 ULP, 2.384e-7). The 4×30 split and `N=120/240 ticks` reproduce the
same pattern (rotation 1 ULP at restore, then velocity/position drift). This is a
**body checkpoint representation bug**, not physics contact ordering: `BibiteSave.rot`
is stored in game transform degrees (`body.rs:2663 rot: r.rot.to_degrees()`) and
restored with `save.rot.to_radians()` (`body.rs:2527`); radian→degree→radian is not
bit-identical for many angles, so the checkpoint bytes compare equal while the
restored internal angle differs by 1 ULP. Egg checkpoints have the same conversion
(`body.rs:3378`/`3399`). Fix owner: body-dev — add raw radians (and any other
lossy representation) to `BibiteCheckpointRecord`/`EggCheckpointRecord`, restore the
raw value, keep `BibiteSave.rot` degrees for the game format.

### Scale limits (5k/10k blocked)

`World::capture_checkpoint_state` **panics** at 5k/10k synthetic:
`LimitExceeded("extension bytes")` at the bibite arena section
(`world.rs:3222 .expect("bibite checkpoint section fits")`). Measured encoded
section sizes: 120=598,854 B; 1,000 body 3,618,016 B (total ≈4.26 MB); 2,000
total ≈8.70 MB; **5,000 body 18,090,016 B (total ≈21.28 MB); 10,000 body
36,180,016 B (total ≈42.55 MB)**. `SimSnapshot` currently caps one extension at
16 MiB, so 5k/10k cannot be captured/restored at all. Fix owner: world-dev/sim.rs
— raise `MAX_EXTENSION_BYTES` (e.g. 64 MiB) and/or split large sections into
multiple tagged chunks; replace the `expect` panic with a returned error. A
1,000-bibite probe also hit `InvalidData("invalid pellet checkpoint arena layout")`
when restoring into a freshly constructed world; 500/2,000 restored, so pellet
arena capacity/layout validation needs a scale check too.

Status: Default resume is bit-exact; synthetic scale needs the two owner fixes
above before 5k/10k can be validated.

### Post-fix revalidation (after body task-117 raw-radians and world cap fix)

- **120-bibite, 120+120 split is now bit-exact** (both chunk restores and final
  fingerprint equal; 1 ULP rotation residual gone).
- **5k/10k synthetic serialization now works** after the 64 MiB extension cap:
  5,000-bibite snapshot 21,613,714 B (encode 33.6 ms, decode 25.4 ms), 10,000
  = 43,151,518 B (encode 66.8 ms, decode 51.4 ms). 100k would still exceed one
  64 MiB section.
- **A scale-only residual remains** at 1k/2k/5k/10k dense-contact synthetic
  worlds: restore-point fingerprints/stores are equal, but the first tick after
  restore differs by `energy` 5 ULP (2k/5k/10k; abs 7.629e-5) or velocity 1 ULP
  (1k), then velocity 1 ULP on tick 2. This is no longer the raw-radian bug and
  is absent at 120 bibites; it scales with dense contact load, so the remaining
  suspect is per-step physics solver scratch/contact ordering, not stored world
  state. Owner: body/world physics; not a checkpoint section omission.

### Task-120: scale-only residual root cause — lost stomach `average_chunk_amount`

The post-fix residual is **not** solver scratch, warm-start impulses, pair
order, or thread nondeterminism. It is an uncaptured/restored field in the body
checkpoint path:

- At the restore point all store checkpoints and `state_fingerprint` compare
  equal, but the reconstructed `BibiteRecord.stomach` differs: the checkpoint
  bytes encode `StomachContentSave.average_chunk_amount`, but
  `BibiteStore::spawn_loaded` restores stomach contents with
  `stomach.add_matter(material, content.amount, ...)` (`body.rs:2582`), which
  only carries `amount` and resets `average_chunk_amount` to the amount.
- Record-level diffs: N=120 → 13 bibites; N=1000 → 171; N=2000 → 372.
  First differing checkpoint record is the same bibite as the first post-restore
  divergence: 1k slot 16 (`average_chunk_amount` source 0.9036821 vs restored
  0.90950024) → first tick velocity 1 ULP; 2k slot 9 (1.139375 vs 1.1467111) →
  first tick energy **5 ULP** (7.629e-5). At 120, 13 records differ but the
  first-tick effect is only the already-known 1 ULP class.
- Warm restore (target world already ran the same split, so persistent physics
  scratch/`body_order` is identical to source) produces the **same** diff as a
  fresh restore, so this is not uncaptured physics scratch. Running the 2k probe
  under CPU affinity 1 vs 3 gives identical fingerprints/diffs, so parallelism
  does not contribute; the residual is deterministic.
- Proposed fix (body-dev): restore `average_chunk_amount` (and preserve stomach
  slot order) in `spawn_loaded`/checkpoint restore, e.g. an
  `add_matter_with_average`/`set_content` API used for the checkpoint path;
  the encoder already stores the value. This is the same class as the earlier
  raw-radian bug: field present in codec but dropped by the reconstruction path.

### 100k section limit (world task-118)

Measured total checkpoint payloads: 5,000 = 21,613,714 B, 10,000 =
43,151,518 B (body arena alone 18,090,016 / 36,180,016 B). Linear extrapolation
to 100,000 is ≈**425 MB** (body ≈362 MB), far above the current 64 MiB
per-extension cap. If a single section cap is retained, 100k needs
chunked/multi-section encoding (split per-arena sections into bounded chunks);
raising the cap alone would be memory-heavy.

### Task-118 implementation: chunked sections, size limits, 100k boundary

Landed in `sim.rs`/`world.rs`/`pellet.rs`:

- `SimSnapshot::push_section(tag, data)` keeps small payloads as direct sections
  (Default sidecars byte-compatible) and splits payloads above 64 MiB into
  bounded **8 MiB chunks** tagged `0x0200 | (tag & 0xFF)` with a
  `(parent_tag, chunk_index, chunk_total)` header. `extension_payload(tag)`
  reassembles direct or chunked sections; every world section uses it.
- `World::capture_checkpoint_state` now returns
  `Result<SimSnapshot, SnapshotError>` instead of panicking. A preflight
  estimate (~4 KiB/bibite + pellet allowance) returns
  `LimitExceeded("checkpoint payload ... >100k unsupported")` before allocating
  a 100k-scale arena; the encoded payload cap is 256 MiB.
- Scale measured on the serialization tests (byte size / capture / encode /
  decode / restore): 5k = 21,636,104 B / 122 ms / 58 ms / 30 ms / 208 ms;
  10k = 43,194,650 B / 255 ms / 103 ms / 65 ms / 459 ms. Both round-trip with
  equal `state_fingerprint` (ignored test
  `world::tests::checkpoint_large_world_5k_10k_roundtrip`).
- Default sidecar sizes are unchanged: 300 t = 168,942 B, 50,000 t = 236,631 B;
  10k-scale snapshots fit comfortably. 100k is **not supported** by the current
  256 MiB payload budget (~425 MB estimated): capture fails cleanly with the
  preflight error rather than panicking; supporting it needs a larger/streamed
  sidecar format (future task).
- Also fixed during scale validation: pellet/pheromone `dense_pos` is
  capacity-length (slot→dense position, `u32::MAX` for free slots), not
  dense-length; the previous validation rejected valid long-run saves. The
  spatial-hash cell-index check was removed (cell entries index the `entries`
  arena, not the live count).


## Task-124 restore-completeness audit (brain-dev, 2026-10-30)

Harness: load real saves (`lol`, `game_written_600`, `game_scenario_seed_600_nospawn`,
`ozymandias`), run 0–300 pre-ticks, capture `SimSnapshot`, restore into a fresh
world from the same contents, then compare **every** extension section byte-wise
plus field-by-field `BibiteCheckpointArena`/`EggCheckpointArena` records
(`BibiteSave` + all checkpoint-only fields), and repeat after 1/2 ticks. A
synthetic randomized population audit (4 seeds × 12/24/40/80 bibites, varied
genome/vitals, 0–300 pre-ticks) was also run; those 20 cases matched exactly.

Findings (exact field/slot evidence):

1. **`stomach_total_content` restore-order bug** (body-dev): `BibiteStore::restore_checkpoint`
   sets `stomach.total_content = checkpoint.stomach_total_content` and then calls
   `stomach.set_contents(...)`, which recomputes `total_content` as the sum of
   slot amounts (organs.rs `set_contents`). The saved cache is therefore
   overwritten. Repro `game_scenario_seed_600_nospawn` pre=1: bibite slot 0,
   source `stomach_total_content=12.119801`, restored `12.092552` (abs 0.027249,
   28,572 ULP); pre=30: `14.689549` vs `14.657421` (abs 0.032128, 33,689 ULP).
   Fix: assign `total_content` after `set_contents`, or make `set_contents` not
   mutate the cached total. This is the remaining body field behind game_seed
   body diffs at every pre≥1.
2. **Spatial-hash slot-map class order** (world-dev): `SpatialHash::capture_checkpoint_bytes`
   writes `slot_maps` in enum/array order `[Bibite(0), Pellet(1), Pheromone(2), Egg(3)]`;
   `restore_checkpoint_bytes` reads them in the hard-coded order
   `[Bibite, Pellet, Egg, Pheromone]`, so Pheromone and Egg maps are swapped.
   Repro `lol` pre=0: `CHECKPOINT_HASH_TAG` (0x010B) bytes differ while body/egg/
   pellet/pheromone/zone/species checkpoints are equal. Fix: read/decode in enum
   index order `[Bibite, Pellet, Pheromone, Egg]` and add a capture→restore byte
   equality test with at least one egg and one pheromone entry.
3. **`lol` pheromone arena restore failure** (world/pheromone owner):
   `restore_checkpoint_state` returns `InvalidData("invalid pheromone checkpoint
   arena layout")` for `lol` at every pre≥1 (pre=0 restores). This blocks resume of
   the historical lol fixture; likely capacity/layout validation rejects the
   captured arena when the fresh store was initialized differently. Needs a
   minimal repro test and either restore exact arena slots or rebuild/validate
   deterministically.
4. **Now passing after task-123**: `StomachContent.average_chunk_amount` and slot
   order are restored exactly (game_seed pre≥1 body diff is only the
   `stomach_total_content` cache above). `game_written` and `ozymandias` restore
   byte-identically at every tested pre-tick; egg/pellet/pheromone/zone/species
   sections equal in all cases without the phero layout error.

Re-run of 1k–10k scale after task-123 is pending the `stomach_total_content`
order fix; once that lands the scale residual should reduce to zero (the prior
1–5 ULP energy/velocity differences came from the same stomach restore path).

### Task-124 follow-up: post-fix scale confirmation + audit matrix

After body task-123's `set_contents`/average-chunk fix and the subsequent
`stomach_total_content` ordering fix, the scale residual is **gone**:

| N | 2 × 120 ticks | 4 × 60 ticks | final fingerprint |
|---|---|---|---|
| 1,000 | exact | exact | `13578550628488094705` |
| 2,000 | exact | exact | `7068554433702277588` |
| 5,000 | exact | exact | `1624841482256976742` |
| 10,000 | exact | exact | `13089570177802539238` |

Every chunk showed `before=true restored=true` (fingerprint equality at resume),
and final single-vs-chunked fingerprints matched for 2× and 4× splits. Snapshot
sizes/timings at 10k: 43.23 MB, encode ~69 ms, decode ~52 ms; 5k: 21.65 MB,
~34/26 ms; 2k: 8.72 MB, ~14/10 ms. 120-bibite split also exact.

Remaining audit mismatches are historical-fixture/field-completeness issues, not
scale:
- `game_scenario_seed_600_nospawn` is body/section-exact at all tested pre-ticks
  after the total-content fix; `game_written_600` and `ozymandias` exact.
- `lol.zip` pre=0: only `CHECKPOINT_HASH_TAG` differs (spatial-hash slot-map
  class-order bug, world-dev). pre≥1: body difference in `senses.has_herd` /
  `herd_sum_direction` / `herd_separation_projection` (the body senses codec
  writes only `SenseOutputs::values()` 21 floats and drops the three herd fields),
  and an egg difference in `EggCheckpointRecord.save.brain.herding_enabled`
  (source false vs restored true), plus the hash section. `lol` no longer hits
  the earlier pheromone arena-layout restore error in the current tree.
- `game_seed` pre=1 body used to differ only in `stomach_total_content`; now it
  is exact at pre=1/10/30/60/120/300.
- Synthetic randomized audit (4 seeds × 12/24/40/80 bibites, varied
  genome/vitals, 0–300 pre-ticks) had no body/egg mismatches.

Fix proposals: body-dev should include the three herd sense fields in the body
checkpoint codec (or encode the full `SenseOutputs` struct) and preserve
`herding_enabled` in egg/brain checkpoint restore; world-dev should fix the
spatial-hash map decode order (Pheromone before Egg) and add the egg+pheromone
layout regression test.

### Task-125/126 landed: final audit re-run (2026-10-30)

After the body herd-sense/egg-herding fixes and world hash class-map order fix:

- All tested real saves restore **byte-identically**: `lol` pre=0/1/10/30/60/
  120/300, `game_written_600`, `game_scenario_seed_600_nospawn`, `ozymandias`;
  every extension section (bibite/egg/brain/zone/pellet/pheromone/tools/hash/
  stats/spawner/species/fingerprint) compares equal and the encoded snapshot
  bytes match. The earlier `lol` pheromone arena-layout restore error is gone.
- One typed-only (non-encoded) mismatch remains in `lol` pre≥10 eggs: egg slot 4
  `save.brain.plan_dirty` is `false` in source and `true` after restore. Egg
  section bytes are equal, so this is a derived lazy flag rebuilt by egg spawn;
  it does not affect the encoded checkpoint or immediate tick, but for strict
  completeness body/brain should clear/set it from the saved brain or include
  it in egg-brain serialization.
- Scale re-confirmed after the latest tree: 120/1k/2k/5k/10k synthetic, both
  2× and 4× splits, all resume-point fingerprints and final fingerprints match
  single runs (10k final `13089570177802539238`, 5k `1624841482256976742`,
  2k `7068554433702277588`, 1k `13578550628488094705`, 120
  `5167677665544882241`). 10k snapshot 43.37 MB at this tree; encode/decode
  ~67/52 ms; 5k ~21.72 MB ~55/33 ms.
- No remaining encoded checkpoint field mismatches were found in the audit.

Status: checkpoint correctness is closed modulo the benign egg `plan_dirty`
flag noted above (owner follow-up if strict no-mismatch is required).

### Streaming `SimSnapshot` sections (v2 `.bb8state`, 2026-11-01)

For io's opt-in streamed v2 envelope, core exposes borrowed-zero-copy sections:

```rust
pub struct SnapshotSection<'a> { pub tag: u16, pub data: Cow<'a, [u8]> }

impl SimSnapshot {
    pub fn encode_sections(&self)
        -> Result<impl Iterator<Item = SnapshotSection<'_>>, SnapshotError>;
    pub fn decode_sections<'a, I>(sections: I) -> Result<Self, SnapshotError>
        where I: IntoIterator<Item = SnapshotSection<'a>>;
    pub fn payload_limit() -> usize;
}
```

- The first section has tag `0` and is the control preamble; it is followed by
  extension sections in their original order (including 64 MiB chunk sections).
  Concatenating `tag`/`len`/`data` in iteration order reproduces exactly the v1
  payload, so a v2 writer can stream sections and compute the v1 checksum over
  the same bytes. `decode_sections` requires tag `0` first and rejects duplicate
  control sections, count mismatches, oversized sections, and aggregate payloads
  above the limit.
- `payload_limit()` keeps the v1 default `MAX_PAYLOAD_BYTES = 256 MiB`; a
  streaming/v2 process may raise it up to `STREAM_MAX_PAYLOAD_BYTES = 1 GiB`
  with `RUSTIBITES_SNAPSHOT_MAX_BYTES`. `World::capture_checkpoint_state`
  preflight uses the same limit, so the 100k (~430 MB) capture is accepted only
  when the env var opts in. Per-section chunking stays 64 MiB; v1 encode/decode
  and all existing sidecars are unchanged.
- Tests: `streamed_sections_match_v1_payload_and_roundtrip` checks v1 payload
  byte equality + section decode round-trip.

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

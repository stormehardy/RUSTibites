# RUSTibites — Subsystem specifications

This directory holds the reverse-engineered, implementation-ready specifications of
The Bibites 0.6.3.1 simulation, extracted from the decompiled C# in `Here/` and
validated against the installed game (parity harness, see [`../PARITY.md`](../PARITY.md)).
The specs are the behavioural contract for `crates/bibites-core` and
`crates/bibites-io`; the architecture contract is [`../ARCHITECTURE.md`](../ARCHITECTURE.md)
and the module-boundary contract is [`../INTERFACES.md`](../INTERFACES.md).
**Canonical test world:** `testdata/real/Default.zip` ([`../TESTING_POLICY.md`](../TESTING_POLICY.md));
real-file citations below (`lol.zip`, `Ring.zip`, `RadTower.zip`, …) are historical evidence with the
world named next to each number, not default test commands.

## Reading order

| # | Spec | Covers | Primary implementation |
|---|---|---|---|
| 1 | [`01-brain-genetics.md`](01-brain-genetics.md) | NEAT brain (33 inputs / 15 outputs / 14 node types), activations, scheduling, the 35 genes, mutation, speciation, genome/brain JSON + version migrations | `bibites-core/src/{brain,genome,species}.rs`, migrations in `bibites-io/src/migrate.rs` |
| 2 | [`02-body-organs-physics.md`](02-body-organs-physics.md) | body scalars, growth/ageing, energy & health accounting, organs (stomach, mouth, propulsion, fat, armour, egg layer, pheromone, clock), feeding/combat, reproduction, death/corpses, body save schema, custom-physics contract | `bibites-core/src/{body,organs,physics}.rs` |
| 3 | [`03-world-ecology.md`](03-world-ecology.md) | zones & plant biomass, matter/pellets/decay, FOV & pheromone sensing, pheromone spots & towers, rad/colour-killer tools, shade/Red Death, spawners & seeding, per-tick ordering, world save format, lineage metrics | `bibites-core/src/{world,zone,pellet,pheromone,senses,tools,sim}.rs` |
| 4 | [`04-formats-settings.md`](04-formats-settings.md) | every file format (`.bb8`, `.bb8template`, `.bb8settings`, save/scenario bundles, `speciesData.json`, `data.bin`), the exact settings defaults (78 + 14 + materials), animators/conditions, version model and round-trip rules | `bibites-core/src/settings.rs`, `bibites-io/**` |

Read them in numeric order: 01 defines the genome/brain seam, 02 consumes it for the
body, 03 wraps the body in the world, 04 defines how all of it is persisted. The
cross-module type contract lives in `docs/INTERFACES.md`; intentional divergences
live in [`../DEVIATIONS.md`](../DEVIATIONS.md) and are summarised in
`docs/ARCHITECTURE.md` §14.

## Status (2026-10-30)

All four specs are extracted and kept current with the implemented code. Each spec
starts with its own **Implementation status** table (section → status → code →
tests); the summary is:

| Spec | Overall | Notes |
|---|---|---|
| 01 brain/genetics | **implemented** | brain, genes, mutation, speciation, CSR/parallel batch, io records and pre-0.6.3.1 migrations all in place; mutation outcomes are statistically (not bit-) faithful because the game's RNG is unseeded (D4/D5) |
| 02 body/organs/physics | **implemented** | full organ/energy/growth/combat/death pipeline; custom solver instead of Box2D (D1/D2); body + organ save schemas implemented in `bibites-io` |
| 03 world/ecology | **implemented** | zones/pellets/decay/sensing/pheromones/tools/spawners/17-phase tick/persistence; `data.bin` histories are an opaque pass-through (D11); some lineage history statistics are partial |
| 04 formats/settings | **implemented for the 0.6.3.x tier** | settings model with exact defaults, `.bb8`/template/save/scenario round-trip verified against the real game; pre-0.6 settings migrations and 0.6.4a1 folder saves are not implemented (see the spec's status table) |

Evidence gates used for "implemented": the module's unit tests (382 tests in
`cargo test -p bibites-core --lib`), the io round-trip/integration tests
(`cargo test -p bibites-io`), and the game-vs-Rust parity harness
(`docs/PARITY.md`). Run:

```powershell
cargo test -p bibites-core
cargo test -p bibites-io
cargo clippy --workspace --all-targets -- -D warnings
```

## Conventions used by the specs

* `Here/<file>:<line>` references point at the decompiled C# source of the installed
  build; the C# compiled behaviour is the source of truth. Relative line numbers are
  stable; treat a stale anchor as a doc bug to fix (specs are owned by the Lead and
  the subsystem owners).
* Confidence tags differ per spec (`[C]`, `[C+S]`, `[S]`, `[INF]`, `[PREFAB]` in 02;
  `[V]`, `[C]`, `[I]` in 03). They are defined at the top of each file.
* Units are the game's: world units `u`, area `u²`, energy `E`, health `HP`, seconds;
  time advances in fixed ticks `dt = 1/sim_tps`.
* `sim_tps` / `brain_tps` / `decay_tps` / vision factors are **per-file settings**,
  never constants: read them from the effective settings. Defaults are 30 / 2 / 2 / 10 / 5.
* Brain transcendentals are evaluated in `f64` and truncated to `f32`
  (Unity/Mono `Mathf.*` semantics); this is parity-critical (spec 01 §2.1).
* Angles are Unity `SignedAngle(up, …)` convention: positive = counter-clockwise
  from forward, so a target on the right is negative. In-game tooltips mirror the
  handedness; the specs' behavioural pairing is authoritative (spec 01 Q1).
* Deliberate divergences (custom physics, deterministic RNG, immediate destruction,
  opaque `data.bin`, …) are catalogued in `docs/DEVIATIONS.md`; the specs link the
  relevant entry rather than re-describing it.
* Spec provenance paths such as `docs/spec/_tmp/recent/...` and `testdata/...` are
  gitignored extraction copies and may be absent from a fresh checkout; they exist to
  document where a captured value came from, not as runnable fixtures.

## Maintenance rules

1. Spec first for new behaviour; when code diverges from a spec, update the spec in
   the same change (`ARCHITECTURE.md` §12.5) and record intentional divergences in
   `docs/DEVIATIONS.md`.
2. A spec's constants/formulas/tables are normative for parity. If a value changes,
   change it in the spec, the code and its tests together.
3. When a section is implemented, update its row in that spec's **Implementation
   status** table (and add the test names that gate it).
4. Do not edit `Here/` or the installed game files; they are read-only reference.

## Change log

| Date | Change | By |
|---|---|---|
| 2026-10-30 | Spec-vs-code audit: implementation status tables added to 01–04, this index added, stale `simTPS`-40 notes corrected (resolved to 30), resolved open questions marked (Q1/BQ-1/BQ-3/BQ-7), doc-only formula/interface corrections | spec-dev |

# PARITY — ground-truth capture and comparison against the installed game

> **CANONICAL TESTING POLICY (2026-10-30):** use `testdata/real/Default.zip` (`docs/TESTING_POLICY.md`) as the canonical world for new captures and parity work. The Ring fixture described in §10 is **historical**; the canonical Default baseline is in `docs/PARITY_ROUND2.md` §10.

Status: **harness implemented and exercised on the installed game** (The Bibites 0.6.3.1, BibitesBridge
0.7.0). Read §1 for the determinism answer and §11 for what is still open.

This document is the contract between the real game and RUSTibites for behavioural parity. It defines:

1. how a capture is produced from the installed game (headless, no window, no user save touched);
2. the JSONL/JSON schema RUSTibites must emit to be comparable (§6);
3. which fields are expected **exact** and which are **statistical** (§7), and the tolerances (§8);
4. how `capture.py` / `compare.py` are used in CI.

Owners: Lead / parity-dev. Write scope: `docs/PARITY.md`, `tools/parity/**`, `testdata/parity/**`.
Nothing else in the repo is touched by the harness; the game install, AppData saves and the
BibitesAPI project are strictly read-only except for the temporary, self-removing parity plugin (§4).

---

## 1. The determinism answer (updated after the measured run)

**The installed game is not bit-reproducible by default.** Two runs from the same save + seed drift:

* `TimeController.CheckMinFPS` is an FPS governor: while a world loads, rendering FPS is low, so it
  lowers `engineTimeScale`, and `UpdateTimeScale` sets `Time.fixedDeltaTime = baseFixedDeltaTime *
  engineTimeScale` whenever that scale is below 1. The first few ticks after every load therefore run
  with a **fractional, frame-timing-dependent dt**. The game recovers to the nominal 1/simTPS dt once
  FPS is back, but those first ticks are already differently integrated. (This is exactly the
  "frame-rate coupling" `docs/ARCHITECTURE.md` §6 says we must replace with the 40/30-TPS-equivalent
  behaviour.)
* A save load resumes the simulation immediately; an HTTP `POST /time {paused:true}` is executed on
  the next frame, so 0–2 simulation ticks (including RNG draws, e.g. virgin spawners, and physics)
  run before the pause lands. Different frame timing ⇒ different number of ticks ⇒ different state.
* Scenario-independent settings are game-global at "new world" time; a *save* is self-describing
  (§5) but a capture that starts from a scenario sees whatever the game had at that moment.

The harness therefore uses the companion BepInEx plugin (`tools/parity/plugin`, GUID
`com.rustibites.paritydump`, **not** part of BibitesAPI) to remove exactly those two frame-rate
couplings:

| plugin route | what it does | why |
|---|---|---|
| `POST /d/pacing {"lock":true}` | every frame pins `Time.fixedDeltaTime = Time.maximumDeltaTime = TimeController.baseFixedDeltaTime = 1/simTPS` | the FPS governor can only change the speed (`Time.timeScale`), never the integration dt. This is the steady-state 1x behaviour the game runs at when not throttled — the deterministic target RUSTibites reproduces. |
| `POST /d/load_pause {"enabled":true}` | attaches to `SaveSystem.onLoadingDone` (via `SceneManager.sceneLoaded`, after `Awake`, before `Start`) and pauses the game inside the load-completion callback | a load pauses at exactly the saved `simulatedTime`, before the first `FixedUpdate` and before any RNG draw or spawner tick |
| `POST /d/preload {setting:value,...}` | applies scenario-independent settings in `SceneManager.sceneLoaded`, i.e. before `SimulationManager` restores the save | random spawners (`virginSpawnRate`) are already disabled when the load-time spawners look at the settings |

With all three hooks active, a base save whose `independents` block pins `virginSpawnRate=0` and the
game running at **speed 1x**, the measured result is in §10: **three consecutive passes of 301 ticks
are bit-identical**. Two further conditions are part of the answer:

* **speed matters**: at 1x one fixed step runs per frame and the run is reproducible; at 50x several
  fixed steps share a frame and the run drifts (first divergence observed at tick 54 and tick 151 in
  two attempts, 27–247 differing ticks out of 301). `capture.py` therefore defaults to `--speed 1`
  and records a taint for any other speed. High-speed captures are useful for ecology statistics, not
  for bit-exact evidence.
* **pause/resume perturbs**: an intermediate pause boundary changes the ticks after the resume (the
  multi-segment experiment diverged at the first tick after the first pause). Deterministic captures
  are therefore ONE uninterrupted segment (entity snapshots only at the start and final tick);
  `--snapshot-every > 0` is a tainted diagnostic mode.

The `compare.py --profile exact` gate is the machine-checkable form of this answer;
`capture.py --repeat K` produces the evidence.

Bridge-only mode (`--no-plugin`) still captures, but records `taint` entries and must be treated as a
**single observation**, not as reproducible ground truth.

What this means for RUSTibites: bit-exact parity is only meaningful against a capture whose manifest
has `plugin.dt_pinned && plugin.paused_at_load` (or an equivalent instrumented plugin) and whose
`taint` is empty. For captures without the plugin, compare ecology curves and behavioural invariants
only (`--profile loose`).

---

## 2. Prerequisites and no-modification rules

* Python 3.11 with the `bibites_api` package. `capture.py` uses it as a library
  (`LiveRuntime`/`Lab`/`BridgeClient`); `BIBITES_API_HOME` overrides the default
  `C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI`.
* The installed game (default `C:\Program Files (x86)\Steam\steamapps\common\The Bibites`) with
  BepInEx 5 and BibitesBridge 0.7.0.
* The parity plugin: build + temporarily deploy with `tools/parity/dump_prefabs.py`, or manually
  `dotnet build tools/parity/plugin/ParityDump.csproj -c Release` and copy
  `testdata/parity/plugin/bin/ParityDump.dll` to
  `<game>/BepInEx/plugins/ParityDump/ParityDump.dll`. **Remove that folder when you are done** — the
  dump script does it in a `finally:`; the installed game is otherwise untouched.
* Rules the harness enforces itself:
  * it never writes into the game install, AppData saves or the BibitesAPI project (the plugin is the
    only transient addition, removed afterwards);
  * it refuses to touch a game that it did not start (`--attach` is the explicit override);
  * it always stops a game it started (unless `--keep-game`), including on exceptions;
  * it records and re-applies the **effective** settings, so a capture never silently inherits the
    machine's game-global settings.

---

## 3. Capture workflow

```powershell
# one-time: build + deploy the dump/pacing plugin (it removes itself after dumping)
python tools\parity\dump_prefabs.py        # also writes testdata/parity/prefab_dump.json

# create a base world from a scenario and capture 300 ticks twice (determinism evidence)
python tools\parity\capture.py --scenario testdata\real\Ring.zip `
    --base testdata\parity\bases\ring_parity.zip `
    --ticks 300 --seed 12345 --speed 1 --repeat 2 `
    --out testdata\parity\captures\ring_seed12345

# compare the two passes (must be bit-identical for a deterministic capture)
python tools\parity\compare.py testdata\parity\captures\ring_seed12345\pass-0 `
    testdata\parity\captures\ring_seed12345\pass-1 --profile exact

# compare a RUSTibites capture against the game
python tools\parity\compare.py testdata\parity\captures\ring_seed12345\pass-0 `
    path\to\rust-capture --report human --profile default
```

`capture.py` steps, in order:

1. `/ping` the bridge. If a game already answers and `--attach` was not passed → fail loudly (never
   hijack a world). Otherwise start a hidden game through `bibites_api.lab.Lab.ensure_game()` (no
   window, no taskbar button, `BIBITES_LAB_MODE` on so the player's autosaves are not touched).
2. If `--base` does not exist and `--scenario` is given: `POST /newgame` (the scenario's settings are
   applied, the scenario's zones/pellets are placed), pause, read the effective settings, force
   `virginSpawnRate=0` (unless `--keep-spawners`), write the world as the base save and write
   `<base>.base.json` provenance (scenario hash, base hash+sim time, effective settings, game/bridge
   versions).
3. Per pass: load the base (plugin pauses inside `onLoadingDone`; retries up to `--load-attempts` if
   the world advanced past the save), verify a clean start (`|t0 - saved_t| ≤ 1e-5s`), re-apply the
   recorded settings and verify their hash, `/lab/seed`.
4. Capture the start state (tick 0, `partial:true`) with a per-entity snapshot.
5. Run in segments that end on a paused boundary: for each segment, `/run` with the sampler at
   `sample.every << dt` (one row per tick, evaluated inside the game's `FixedUpdate`, so tick-exact)
   and a scheduled `POST /time {paused:true}` at the nominal boundary − dt/2. Fractional frame
   boundaries are allowed (the actual paused tick is recorded; overshoot is in the manifest); a
   missing tick is a hard error.
6. At every paused boundary take a per-entity snapshot: `/bibites/table` + `/entities/json` →
   compact entity records merged into the tick record.
7. Write `ticks.jsonl.gz`, `events.jsonl.gz`, `manifest.json`; read the event log
   (`GET /events`, exact to a simulation tick) as a separate stream.
8. Repeat from step 3 for `--repeat`; then write `determinism.json` (tick-by-tick digest equality).

### What `/lab/seed` seeds, exactly (bridge 0.7.0, `ApiLab.SeedAll`)

Called while the world is **paused** right after a load, `POST /lab/seed {"seed":N}` reseeds *in
place* (it copies generator state into the existing instances — replacing the static fields made the
game spin on a third reseed):

* `UnityEngine.Random.InitState(N)`;
* `ListExtend.r` (static `System.Random`), `BibiteGenes.SystemRandom`, `NEATBrain.r`;
* `ZoneManager.r` and every `Zone.r` (instance `System.Random` per zone, seeded `N+1, N+2, …`).

It does **not** seed anything created later with `new System.Random()` or anything the game reads from
`System.DateTime`; and it only equalises *random state* — not physics solver order or frame timing.
That is why the pacing/load-pause hooks above are also required.

---

## 4. The parity plugin (`tools/parity/plugin`)

A small BepInEx plugin with a unique GUID (`com.rustibites.paritydump`), deliberately independent of
BibitesAPI and compiled **without** a reference to the game assembly (all game types are reached by
reflection, so a game update cannot break it at compile time). It is only needed for
(a) deterministic pacing/loads during captures and (b) the prefab/serialized-constant dump.

| route | purpose |
|---|---|
| `GET /d/ping` | liveness + game/Unity versions |
| `GET /d/dump` | the full prefab/serialized-constants JSON (§9) |
| `GET/POST /d/pacing` | pin `Time.fixedDeltaTime` to `1/simTPS` (`{"lock":true|false}`) |
| `GET/POST /d/load_pause` | arm/disarm "pause inside `SaveSystem.onLoadingDone`" |
| `GET/POST /d/preload` | settings applied in `SceneManager.sceneLoaded`, before the save is restored |
| `GET /d/time` | `fixedDeltaTime`, `maximumDeltaTime`, `timeScale`, `engineTimeScale`, `simulatedTime`, `fps` |

Build artifacts go to `testdata/parity/plugin/` (gitignored). Deployment is temporary:
`<game>/BepInEx/plugins/ParityDump/ParityDump.dll`.

---

## 5. Inputs that identify a run

A capture is only comparable to another capture with the same **inputs**:

| input | where it is recorded | notes |
|---|---|---|
| scenario | `inputs.scenario {path, sha256}` | only when a base was created from one |
| base save | `inputs.base {path, sha256}` + `base.sim_time` | the exact world state tick 0 is relative to |
| settings | `settings` (full `/settings` `{scenario, independent}`) + per-record `settings_hash` | **effective values**, read back from the game |
| seed | `inputs.seed`, `inputs.seeded_parts` | `/lab/seed` |
| game/bridge/Unity | `engine.*` | version drift is a hard error in `capture.py` |
| tick range | `sim.start_tick..end_tick`, `sim.sim_tps`, `sim.dt` | tick numbering is absolute from the base save |

**Where the scenario-independent settings live (verified for 0.6.3.1):**

* A **save** zip carries them in `settings.bb8settings["independents"]` (14 values: `SimulationSize`,
  `biomassDensity`, `pelletGrowth`, `limitBibiteBirth`, `bibiteLimit`, `virginSpawnRate`,
  `speciesGeneticSpan`, `speciesSpanScalingFactor`, `speciesPrunePointLimit`, `simTPS`, `brainTPS`,
  `decayTPS`, `visionLookupUpdateFactor`, `visionSenseUpdateFactor`) and `LoadGame` applies them.
  `capture.py` records this block (`inputs.independents_in_save`) and after every load cross-checks the
  game's effective values against it (mismatch = drift error).
* A **scenario** zip carries a `recommended` list instead (often empty); a *new* world therefore takes
  the game's current global values. `capture.py` records the effective values after the newgame and
  freezes them into the base, so the fixture is self-describing either way.
* Both are floats in the file (`{"Value": ...}` wrappers); the bridge `/settings` returns the live
  values as numbers under `scenario`/`independent`.

`simTPS` is per-world: the Ring fixture captured here runs at **simTPS 15** (dt = 66.67 ms) because the
save's `independents` block says so; a fresh game's default is 30. Never hardcode 40/30 in RUSTibites —
take it from the manifest/capture settings (`sim.sim_tps`).

---


### 6.x Feeding capture group + gates (from FEEDING_METRICS Appendix D)

## Appendix D — suggested `docs/PARITY.md` §6 patch (parity-dev owns PARITY.md; suggestion only)

Lead decision (2026-10-30): do not edit `docs/PARITY.md` directly; integrate this after task-96. All
additions are **optional and additive** — keep `rustibites.parity.capture.v1` / `.tick.v1` /
`.event.v1` unchanged, and keep `compare.py` ignoring unknown groups/fields. The authoritative
counter/units contract is this document (§2–§7).

**D.1 — §6 preamble (add one paragraph after the three-file layout):**

> Feeding/energy fields are **optional and additive**; readers must ignore unknown groups and fields.
> The counter definitions, units and conservation identities are specified in
> `docs/design/FEEDING_METRICS.md`. `behavior.bite_rate_mean` keeps its legacy meaning (bibite
> first-bites per lifetime bibite-second) and is **not** a feeding metric; feeding rates use
> `behavior.*_per_s` with a live-bibite-second denominator.

**D.2 — §6.1 tick record table: extend the `behavior` row and add two optional rows.**

Replace the existing `behavior` row description with (additions in italics):

> | `behavior` | `{speed_mean, speed_max, moving_frac, travel_rate_mean, bite_rate_mean, damage_dealt_rate_mean, egg_rate_mean, outputs}` *(plus, optional: `plant_bites_per_s`, `meat_bites_per_s`, `bibite_bites_per_s`, `bibite_damage_per_s`, `plant_consumed_u2_per_s`, `meat_consumed_u2_per_s`, `plant_consumed_e_per_s`, `meat_consumed_e_per_s`, `stomach_energy_gain_per_s`)* | measured behaviour; legacy `bite_rate_mean` = bibite first-bites / lifetime; the `*_per_s` fields are per **live bibite-second** and may be absent when counters are unavailable | game: bit **Tier B** derives them from the `feeding` stream (`evidence: "polling_proxy"`); Rust: task-90 counters |

Add after the `tags`/`metrics` rows:

> | `feeding` | object (optional) | raw per-bibite/per-zone feeding evidence snapshot (`schema: "rustibites.parity.feeding.v1"`, §4.3: `bibites[{id, energy, health, stomach_total, stomach_materials{}, held_pellets[], n_held, bite_progress, bite_period, bibites_bitten, damage_dealt, total_murders, murdered_area}]`, `zones[{pellet_biomass, max_biomass, free_biomass, pellet_production, pellet_progress, pellets_n, plant_amount, meat_amount, produced_energy_cumulative, removed_energy_cumulative, culled_energy_cumulative, recycled_energy_cumulative(null), decayed_energy_cumulative(null), counters_evidence}]`, `counters{...polling_census_proxy...}`, `totals`, `material_properties`) | game: plugin polling with `--feeding`; Rust: normally omitted |
> | `feeding_stats` | object (optional) | interval totals behind the `*_per_s` rates: `{plant_bites, meat_bites, bibite_bites, plant_swallows, meat_swallows, bibite_swallows, plant_consumed_u2, meat_consumed_u2, bibite_consumed_u2, plant_ripped_u2, meat_ripped_u2, bibite_damage_dealt, bibite_kills, stomach_energy_gain_e, energy_gained_e, energy_spent_e, live_bibite_seconds}` (u64 counts, f64 amounts/energy) | Rust task-90; game side derives equivalent deltas only when it can attribute causes, else omit |

**D.3 — §6.2 entity record table: add optional feeding fields after `size`:**

> | `stomach_u2` | float (optional) | `BibiteStomach.totalAmount` at the snapshot |
> | `stomach` | array (optional) | `[{material, amount}]` from `body.stomach.content[]` (current Tier-A fallback; plugin `feeding.bibites[].stomach_materials` supersedes it) |
> | `mouth` | object (optional) | `{bibites_bitten, total_damage_dealt, total_murders, murdered_area, bite_progress}` from `body.mouth` |
> | `n_held` | int or null (optional) | held-pellet count; `null` when the source cannot expose grab links (save JSON cannot) |

**D.4 — §6.4 manifest: add after `counts_start/end` (all optional):**

> `feeding` `{requested, available, schema:"rustibites.parity.feeding.v1", file, samples, poll_period_s, evidence: "plugin_events"|"polling_proxy"|"polling_census_proxy"|"save_snapshot"|"inferred"|"none", exact_events: bool, note}` — provenance of the feeding evidence; `evidence` must not be omitted when a `feeding` group exists. Current addendum (`7a5ea90`) writes `evidence:"polling_proxy"` and `exact_events:false`.
>
> `counters` `{plant_consumed_u2, meat_consumed_u2, plant_bites, meat_bites, plant_swallowed, meat_swallowed, bibite_bites, bibite_kills, live_bibite_seconds, produced_energy_cumulative, removed_energy_cumulative, culled_energy_cumulative, recycled_energy_cumulative, decayed_energy_cumulative, consumed_energy_estimate_cumulative, counters_evidence, missing_terms[], ...}` — cumulative engineering totals where exact (Rust task-90) or polling-census proxies where tagged (game); `missing_terms` mandatory when any cumulative term is null/placeholder.
>
> `conservation` `{stock_*_start/end, produced_u2/e, consumed_u2/e, decayed_u2/e, culled_u2/e, recycled_u2/e, residual_u2, residual_rel, terms_available[], terms_missing[]}` — identity terms; `terms_missing` is mandatory so inferred values are never mistaken for measured ones (see FEEDING_METRICS §3). Current game terms: `removed_energy_cumulative` is mixed-cause, `culled_energy_cumulative` is a placeholder equal to removed (not a measured cull term), `recycled`/`decayed` are null → game conservation stays ungated until cause attribution/exact accumulators land.

**D.5 — taint entry for feeding polling (add to the taint vocabulary in §6.4/§7):**

> `"feeding: polling adds periodic main-thread HTTP requests while the world runs; dt/RNG unchanged (fixed-dt pin), wall pacing perturbed"`

**D.6 — §8 compare note (not §6, but lands with it):**

> `compare.py` adds `feeding` (rates; default 10 % rel, loose 25 %, absolute floors 1e-3 u² / 0.01 E /
> 1 event; `bite_rate_mean` informational) and `conservation` (Rust-only until the game has cumulative
> produced/recycled/decayed/culled counters; then gated at 1 % of throughput with absolute floors).
> Unknown feeding fields are ignored for forward compatibility; self-test covers the new sections.

If parity-dev prefers, the whole block can instead be a shorter cross-reference ("feeding schema per
`docs/design/FEEDING_METRICS.md` §6") — the tables above are the integration-ready version.

## 6. Capture schema (what RUSTibites must emit)

Three files, identical for game and Rust captures (gzip JSONL; UTF-8, one JSON object per line):

```
<capture>/
  manifest.json       rustibites.parity.capture.v1
  ticks.jsonl.gz      rustibites.parity.tick.v1   (one record per sampled tick, ascending tick)
  events.jsonl.gz     rustibites.parity.event.v1  (optional: birth/death log)
```

**Rust writer compatibility:** gzip is *not* required. `compare.py`/`parity_common.open_capture()`
accept, inside the capture directory, either `ticks.jsonl.gz` / `events.jsonl.gz` (gzip, RFC 1952 —
any standard encoder, e.g. Rust `flate2::write::GzEncoder`) **or plain `ticks.jsonl` / `events.jsonl`**
(one JSON object per line, no trailing commas). The simplest Rust output is plain `ticks.jsonl` +
`manifest.json`; readers detect which file exists (gz preferred when both do). A bare
`<file>.jsonl[.gz]` path is also accepted by `compare.py` (with an optional sibling
`<file>.manifest.json`), so a single-stream Rust dump can be diffed without a directory. Encoding is
UTF-8, newline-separated, `--report json` / `--out` write the comparison report wherever asked.

The reader dispatches on the `.gz` suffix (`parity_common._read_jsonl_gz`), so a plain Rust stream and
a gzipped game capture compare directly against each other. This is covered by a regression in
`compare.py --self-test`: it writes a plain (non-gzip) `ticks.jsonl` + `manifest.json`, opens it through
`parity_common.open_capture()`, compares a gzip game capture against the plain capture (exit 0), checks a
mutated plain capture is detected (exit 1), and opens/compares a bare `ticks.jsonl` with its sibling
`ticks.manifest.json`. (The reader originally assumed gzip; the suffix dispatch was fixed 2026-10-29.)

`manifest.json` is required for a full comparison (settings/engine/tick-rate meta); without it
`compare.py` still compares records but marks the meta checks as warnings. Every record must carry
`schema/source/tick/t/sim_tps/brain_tps/settings_hash` (§6.1); records are sorted by `tick` when
written and validated as consecutive by `capture.py`.

### 6.1 Tick record (`ticks.jsonl.gz`)

| field | type | meaning |
|---|---|---|
| `schema` | string | `"rustibites.parity.tick.v1"` |
| `source` | string | `"game"` or `"rust"` |
| `tick` | int | **absolute** simulation tick since the base save's state (tick 0 = base state) |
| `t` | float | simulated time in seconds (`= sim.base_time + tick*dt` for game captures) |
| `sim_tps`,`brain_tps` | int | fixed tick rates used (from settings) |
| `settings_hash` | string | `parity_common.settings_hash()` of the capture's effective settings (§6.4) |
| `partial` | bool (optional) | `true` for the start snapshot: `entities`/counts only, aggregates incomplete |
| `counts` | `{bibites, eggs, corpses}` | population counts |
| `pellets` | `{plant_n, plant_biomass, meat_n, meat_biomass, free_biomass, bibite_biomass}` | pellet counters and biomass |
| `vitals` | `{energy_mean, health_mean, age_mean, age_max}` | living population |
| `generation` | `{gen_max, gen_mean}` | generation stats |
| `species` | `{n, counts: {species_id: n}}` | active species |
| `brain` | `{nodes_mean, nodes_max, syn_mean, syn_max}` | brain size stats |
| `genes` | `{mean: [35], std: [35]}` | per-gene population stats, **game gene-enum order** |
| `lifecycle` | `{births, deaths}` | changes between samples |
| `behavior` | `{speed_mean, speed_max, moving_frac, travel_rate_mean, bite_rate_mean, damage_dealt_rate_mean, egg_rate_mean, outputs: {NodeName: mean}}` | measured behaviour; `outputs` = mean activation of the 15 brain output nodes |
| `body` | `{size_mean, area_mean, mass_mean, max_energy_mean, max_health_mean, metabolism_mean}` | body stats |
| `tags` | `{tag: {n, biomass}}` (optional) | tag statistics |
| `metrics` | `{label: value}` (optional) | game-native metrics (`SimulationMetric`) |
| `entities` | array (optional) | per-entity snapshot, see §6.2 (present at snapshot ticks) |

All numeric values are `f32` values widened to JSON doubles. Missing groups are allowed for `rust`
captures only if the Rust engine genuinely does not model them yet; `compare.py` compares the
intersection and reports gaps explicitly.

### 6.2 Entity record (`entities[]`)

| field | type | meaning |
|---|---|---|
| `kind` | `"bibite"` / `"egg"` | |
| `id` | int / null | `body.id.id` from the save (stable across a load; eggs have no stable id) |
| `species`, `generation`, `tag` | int/int/string | genes block |
| `x`, `y`, `vx`, `vy`, `rot` | float | position (rb2d `px/py`, which wins over transform), velocity components, rotation deg |
| `energy`, `max_energy`, `health`, `max_health`, `age_s`, `size` | float | vitals (max_* from the population table) |
| `nodes`, `synapses` | int | brain size |
| `genes_digest` | string | SHA-256/16 of the 35 gene values as **f32 bit patterns** in gene-enum order (§6.3) |
| `sense_digest` | string | digest of the 33 input node `Value`s (the brain's sensor readings) |
| `output_digest` | string | digest of the 15 output nodes' `Value`/`LastInput`/`LastOutput` (brain IO) |
| `brain_digest` | string | digest of full node runtime state + topology + synapses |
| `genes` | array of 35 floats (optional) | only when a capture wants them embedded (`--embed-genes` style); digests suffice for comparison |

### 6.3 Digest definitions (cross-language exact)

All digests are `sha256(bytes).hexdigest()[:16]`, over **little-endian raw bits**, never over decimal
text:

* `f32` → IEEE-754 single, `struct.pack('<f', x)` / Rust `f32::to_bits().to_le_bytes()`;
* `i32` → `struct.pack('<i', x)` / Rust `i32::to_le_bytes()`;
* bool → one byte 0/1.

* **genes_digest**: for each name in `manifest.gene_names` (the 35 game gene names, enum order):
  `f32(gene)`.
* **brain_digest**: `i32(nodes)`; then for each node sorted by `Index`: `i32(Type) i32(Index)
  i32(Inov) f32(baseActivation) f32(Value) f32(LastInput) f32(LastOutput)`; then `i32(synapses)`;
  for each synapse in array order: `i32(Inov) i32(NodeIn) i32(NodeOut) f32(Weight) u8(En)`.
* **sense_digest**: `i32(count)` then for each input node (index 0..32, ascending)
  `i32(Index) f32(Value)`.
* **output_digest**: `i32(count)` then for each output node (index 33..47, ascending)
  `i32(Index) f32(Value) f32(LastInput) f32(LastOutput)`.
* **settings_hash**: for scope in `("scenario","independent")`, keys sorted:
  `scope "{" key "=" tag payload ";" … "}"` where tag is `b`+1 byte (bool), `i`+i32 (int),
  `f`+f32 (float).

**Float normalisation in comparisons:** every simulation scalar in this schema is a game `float`
(f32) widened to a JSON double. The game writes `float`s with .NET shortest-round-trip formatting
(e.g. `154.960068`) while RUSTibites widens f32 to f64 exactly (`154.96006774902344`), so the same
logical value parses to slightly different doubles. `compare.py`/`parity_common.f32_value()` quantise
declared-f32 scalars to f32 bits **before** bit-equality counters and value differencing (digests are
already computed over f32 bits); tolerance-based metrics (position/velocity/biomass curves) still use
their normal tolerances on the quantised values, and integer counters are never f32-rounded.

The gene order is the game's `BibiteGenes.Genes` enum order:
`LayTime BroodTime HatchTime SizeRatio SpeedRatio ColorR ColorG ColorB BrainMutation…` — the full list
is `parity_common.GENE_NAMES` and is repeated in every manifest as `gene_names`. Rust must use it
verbatim for `genes.mean/std` and `genes_digest`.

### 6.4 Manifest (`manifest.json`)

```
schema            "rustibites.parity.capture.v1"
kind              "capture"
source            "game" | "rust"
capture, pass     name of the capture / pass index
created_at        ISO-8601 UTC
engine            {name, game_version, bridge_version, unity_version, ...}   (rust: {name:"RUSTibites", version, git})
gene_names        [35 strings]      gene-enum order used by this capture
inputs            {scenario {path,sha256}|null, base {path,sha256,sim_time}|null, save|null,
                   seed, seeded_parts, settings_requested {scenario,independent},
                   independents_in_save {14 values} | null, settings_file}
settings          {scenario:{...}, independent:{...}}   effective settings the run used
settings_hash     parity_common.settings_hash(settings)
sim               {sim_tps, brain_tps, dt, base_time, start_t, start_tick, end_tick, ticks,
                   wall_s, achieved_ticks_per_s, achieved_speed, overshoot_ticks[], segments[]}
world             {simulation_size, zones[]}            (initial world description)
counts_start/end  {bibites, eggs, corpses}
plugin            {available, version, dt_pinned, paused_at_load, preloaded_settings}
taint             [strings]        non-fatal drift/limitations; empty = clean capture
files             {ticks:{name,sha256,bytes,records}, events:{...}}
```

For RUSTibites, `engine` = `{name:"RUSTibites", version, git_commit, threads}`; `inputs.seed` = the
`--seed`; `inputs.base` = the same base save; `settings` = the effective settings the Rust engine
loaded (same shape, same key names); `plugin` = `{available:false}`; `sim.achieved_*` = Rust throughput.
`tick` numbering is relative to the same base save's state (tick 0).

---

## 7. Exact vs approximate vs unavailable fields

**Expected exact** (same base, same settings, same seed, deterministic capture; compared bitwise by
`compare.py --profile exact`):

* `genes_digest` per entity (genome loaded from the same base save; mutation only on birth/mutation),
* `species`, `generation`, `tag`, brain topology (`nodes`, `synapses`),
* `brain_digest`, `sense_digest`, `output_digest` **while physics has not diverged** — senses are
  computed from positions of entities and pellets, so once trajectories differ these diverge
  legitimately,
* energy/health **amounts** at a given tick as `f32` bits, on the same trajectory,
* population counts, births/deaths event stream (ids/times/kinds), species counts,
* pellet counters.

**Approximate by design** (physics): positions, velocities, rotation, contacts/impulses; aggregate
biomass curves over long runs; `behavior.speed_*`/`travel_*`; anything downstream of trajectories.
`docs/ARCHITECTURE.md` §7 makes the custom solver an intentional divergence; these are compared
statistically (MAE/RMSE/p95), not bitwise.

**Unavailable from the installed game via this harness:**

* per-tick per-entity brain IO: the bridge exposes entity brain runtime state only on demand
  (`/entities/json`); game captures therefore contain `entities` only at paused snapshot ticks
  (`--snapshot-every`, default every 50 ticks) — aggregate brain output means ARE available every
  tick via the sampler `behavior` group;
* velocity vectors in aggregate form (only per-entity snapshots; sampler has speed magnitude only);
* Box2D contact/impulse data (not exposed);
* exact per-tick positions of pellets (only counts/biomass aggregates + a binary `/map/pellets` dump
  for spot checks);
* the game's internal RNG stream/mutation draws (seed state is not serialisable; `/lab/seed`
  reseeds in place, and .NET `System.Random` is not a stream Rust can trivially reproduce — genome
  mutations will only match if RUSTibites implements the identical .NET generator and call order;
  until then compare genetics of *unmutated* populations and record mutation-dependent runs as
  statistical).

---

## 8. Comparison (`tools/parity/compare.py`)

```
python tools/parity/compare.py LEFT RIGHT [--report human|json] [--profile exact|default|loose]
       [--out report.json] [--strict-settings] [--allow-missing-ticks]
python tools/parity/compare.py --self-test        # no game needed; verifies the tool itself
```

* LEFT/RIGHT are capture directories (or `.jsonl.gz` files). Ticks are aligned by **absolute tick**;
  missing ticks are reported and (unless `--allow-missing-ticks`) are a failure.
* Per-system output: `identity/meta` (schema, sim_tps, gene order, settings, tick coverage),
  `ecology` (per-curve MAE/max/exact/tolerance/first diverge tick), `entities` (exact-match counters
  for genes/brain topology/brain IO/sense/output, energy exact fraction + MAE, position/velocity
  MAE/RMSE/max/p95, unmatched ids), `brain.outputs` (15 brain output means), `events`
  (births/deaths, kind distribution, identity matches).
* Profiles:
  * `exact` — bit-identical required (game-vs-game determinism; game-vs-Rust exact-parity claims);
  * `default` — exact systems 100%, ecology curves within 10% relative (>=1 absolute), energy >= 95%
    bit-exact, positions/velocities reported but not gated (physics divergence is expected);
  * `loose` — 25% curves, everything else informational.
* Exit code: `0` all gates pass, `1` a gate failed (CI), `2` usage/IO error.
* First divergence: each curve and entity comparison reports the first tick/id/field that differs, so
  the first behavioural divergence can be traced to a system.

* **Optional gaps (RUSTibites partial captures):** a sampler group or a single field that a Rust
  capture does not model yet may simply be omitted from the tick records. `compare.py` skips it and
  reports it as an informational gap in `meta.gaps` (`curves_skipped`, per-group tick counts) and the
  `optional_gaps` check - never as a failure. Do **not** emit placeholder zeros for unavailable
  fields: a present value is compared and can fail. Required minimum for any comparison: the per-record
  identity fields (§6.1) and a manifest with `schema/source/engine/gene_names/settings/settings_hash`
  and `sim.sim_tps`; a capture must share >=99% of the game's ticks (`--allow-missing-ticks` downgrades
  that check). Per-entity systems compare only on ticks where both sides have `entities` - the
  deterministic game fixture has them at tick 0 and the final tick, so a Rust emitter should provide
  entities there (or every tick) and must preserve the base save's `body.id` values for matching.

`compare.py --self-test` synthesises two identical captures (expect 0) and one mutated capture
(expect 1), checks digest stability, the schema constants and the plain-JSONL path (plain
`ticks.jsonl` + `manifest.json`, gzip-vs-plain comparison, bare `.jsonl` with its sibling
`<stem>.manifest.json`); it must pass before the harness is trusted in CI.

---

## 9. Prefab / serialized-constant dump (`testdata/parity/prefab_dump.json`)

Some values RUSTibites needs are in **neither** save files **nor** the decompiled C# (the C# declares
them but the values live in scene/prefab assets). `tools/parity/dump_prefabs.py` builds the parity
plugin, deploys it temporarily, starts a hidden game with a scenario world, `GET /d/dump`s the values
and removes the plugin again. Output schema: `rustibites.parity.prefab_dump.v1`.

Sections and provenance (game 0.6.3.1, Unity 6000.0.44f1, dumped from runtime objects):

* `matter_materials` — `default_materials` (the untouched `MatterMaterial` prefab/ScriptableObject
  originals), `manager_serialized_materials` (the manager's serialized fields), `live_materials`
  (instantiated, possibly scenario-modified), `matter_material_settings`. Fields: `Cohesiveness`,
  `MassDensity`, `EnergyDensity`, `Reactivity`, `MinConversionEfficiency`,
  `MaxConversionEfficiency`, `decay`, `decayRate`, `freshTime`, plus `NormalEnergyAtScale1`
  (= `EnergyDensity*20`) and the `NormalAmountAtScale1=20`/`NormalRadiusAtScale1=2.5` constants.
  Key values: Plant ED 5 / MD .015; Meat 10 / .025; **Fat ED 30 / MD .03**; **Armor ED 1.5 / MD .05**;
  **Energy ED 1.0 / MD 0** (size-independent: energy is not a physical material); **Health ED 0.25 /
  MD 0**.
* `physics_materials` — `PhysicsMaterial2D` assets referenced by the prefabs (friction/restitution
  live only there): `corps` friction 0.15, bounciness 0.1 (bibite body/tail, egg); `BouncyBox`
  friction 0.4, bounciness 0.4 (plant pellet); `Slippery` friction 0.02, bounciness 0.0 (meat pellet);
  combine modes Mean/Maximum.
* `physics2d` — gravity (0,0), velocityIterations 5, positionIterations 3, `defaultContactOffset`
  0.01, `bounceThreshold` 1, `maxTranslationSpeed` 100, `maxRotationSpeed` 360, etc., plus
  `layer_names` (0..31) and the full 32×32 `ignore_layer_collision` matrix. For layers 0..11 the only
  **enabled** collision pairs are (6,6), (7,8), (7,9), (8,8), (8,9), (9,9): `7=SceneTriggers`,
  `8=bibite`, `9=pellets`.
* `prefabs` — every prefab held by `WorldObjectsSpawner` (`bibiteEntry`, `eggEntry`, plant/meat pellet,
  spawners, `colorKiller`, towers…), dumped as a GameObject hierarchy with, per component, the
  serialized fields and resolved sub-objects (`Rigidbody2D`: bodyType/gravityScale/mass/useAutoMass/
  linearDamping/angularDamping/interpolation/collisionDetectionMode/constraints; `Collider2D`:
  shape/size/direction/offset/isTrigger/density/PhysicsMaterial2D; `ColorKiller`: hue, tolerance,
  bodyOrEye, anyOrMost, radius, period; `RadTower`/`PheroTower`: FloatSetting values; …).
  Key: bibite `Rigidbody2D` gravityScale 0, mass 1, linearDamping 1, angularDamping 0.5; body
  capsule size (5,9) offset (-1.5,0); tail capsule (5,7) offset (0,-4.7); mouth box (2.72,7) offset
  (2.38,0) isTrigger; egg circle r 5; pellet circles r 2.5.
* `provenance` — how it was produced, scenario, bridge ping, live status, timestamp.

Re-run: `python tools/parity/dump_prefabs.py [--scenario <zip>] [--out <json>]`. No compile-time
dependence on the game assembly; a game update only requires re-running the dump and refreshing
`testdata/parity/prefab_dump.json` (gitignored: regenerate locally).

---

## 10. Measured determinism (Ring fixture, this machine)

### 10.1 Reproducing the Ring fixture and the game-vs-Rust diff

```powershell
# 1) game ground truth (needs the ParityDump plugin deployed for the 1x deterministic regime)
python tools\parity\capture.py --base testdata\parity\bases\ring_parity.zip --ticks 300 --seed 12345 --repeat 2 --out testdata\parity\captures\ring_seed12345

# 2) Rust capture from the same base/seed/tick count (plain JSONL, 301 records tick 0..300)
cargo run -q -p bibites-cli -- run testdata\parity\bases\ring_parity.zip --ticks 300 --seed 12345 --capture testdata\parity\rust_ring

# 3) diff (default = exact systems gated, ecology within 10%, physics reported not gated)
python tools\parity\compare.py testdata\parity\captures\ring_seed12345\pass-0 testdata\parity\rust_ring --profile default --report json --out testdata\parity\rust_ring.compare.json
```

Fixture identity: base `ring_parity.zip` (created from Ring.zip), `sim_tps=15`, `brain_tps=2`,
seed 12345, ticks 0..300, 1 bibite (virgin spawns disabled), entity snapshots at tick 0 and the final
tick. Game manifest `sim.base_time` is the save's `scene.bb8scene.simulatedTime`; Rust must number
ticks from the same base state, emit `sim_tps=15`, the 35-name gene order, and preserve the base
save's `body.id` values so per-entity systems match. `--profile exact` remains config-only for
game-vs-game replay; use `default` for game-vs-Rust until the custom physics is tuned.

For **evolved saves** (large `simulatedTime`) the load is accepted as `f32_quantised` (see §10.3)
instead of `clean`: `capture.py` records the reason in `taint`/`sim.load_*` and continues; pass
`--strict-load` to require the old absolute 1e-5 s criterion (small-`t` fixtures only), or use
`python tools/parity/capture.py --self-check` to exercise the acceptance policy without a game.

Fixture: `testdata/parity/bases/ring_parity.zip` (created from Ring.zip, `independents` frozen with
`virginSpawnRate=0`), 300 ticks, seed 12345, speed 50, game 0.6.3.1 / bridge 0.7.0 / Unity 6000.0.44f1.
Command: `capture.py --base … --ticks 300 --seed 12345 --speed 1 --repeat 3`.

| measurement | result |
|---|---|
| clean load | both passes pause at exactly `t0 = 0.20000001788139343` = saved `simulatedTime` (Δ = +7.45e-09 s, i.e. one f32 ULP); start entity snapshot identical |
| **1x, uninterrupted, 3 passes** | **301/301 common ticks identical, 0 differing, 0 overshoot; entity snapshots identical; events identical** → `compare.py --profile exact` exit 0 |
| 50x, uninterrupted, 2 passes | reproducible only for a while: first divergence at tick 54 (247/301 ticks differ) in one attempt, tick 151 (27/301) in another; no overshoot difference needed for divergence |
| intermediate `--snapshot-every 50` at 50x (6 pause/resume boundaries) | first divergence exactly at the first tick after the first pause boundary |
| per-tick record size | ~1.65 KB JSON raw, gzip ≈ **100 B/tick** (0.5 MB raw / 31 KB gzip for 300 ticks) |
| throughput | at 1x: 300 ticks in 20.08 s = **14.9 ticks/s** (1× real time, wall-clock bound by the game's own 1× pacing). At 50x: 0.5–1.8 s per 300 ticks = **170–610 ticks/s**, but not reproducible |
| final-tick overshoot | 0 ticks at 1x; 1–10 ticks at 50x (Unity finishes fixed steps already queued for the frame); recorded in `sim.overshoot_ticks` |
| bridge-only (`--no-plugin`) | not reproducible: the load resumes before the HTTP pause and the FPS governor rescales the first ticks` fixed dt |

**Deterministic captures are therefore 1x/slow (≈1 real-time second per 100 ticks at simTPS 15).**
For fast, large captures use `--speed 50` and compare with `--profile default --allow-missing-ticks`:
curves are statistically comparable, exact per-tick parity is not claimed.

**Verdict:** the installed game *can* provide bit-reproducible ground truth at **1x speed** inside one
uninterrupted run, with (a) `Time.fixedDeltaTime` pinned to 1/simTPS, (b) the pause at load done inside
`SaveSystem.onLoadingDone`, (c) `virginSpawnRate=0` in the base, (d) `/lab/seed` after the load.
It is NOT reliably reproducible at high speed (frame grouping) and NOT across a pause/resume
boundary; both are recorded as taint. Physics trajectories remain the intentionally-divergent system
(`ARCHITECTURE.md` §7): even a perfectly deterministic game capture is only directly comparable to
Rust for brain/sense/energy/genetics until the custom solver is tuned.

### 10.2 Game-vs-Rust diff — current state (2026-10-30, after body/io/stats/brain fixes)

Command (fixture: game capture `ring_seed12345/pass-0`, Rust capture `testdata/parity/rust_ring`):

```powershell
cargo run -q -p bibites-cli -- run testdata\parity\bases\ring_parity.zip --ticks 300 --seed 12345 --capture testdata\parity\rust_ring
python tools\parity\compare.py testdata\parity\captures\ring_seed12345\pass-0 testdata\parity\rust_ring --profile default --report json --out testdata\parity\rust_ring.compare.json
```

Result: **`OVERALL WARN`, exit code 0** under the default profile. No gated failure; the only warning is
`entity_brain` at the final snapshot, which is expected once the physics trajectory has diverged (the
brain/sense digests are identical at tick 0 and differ at tick 300). `settings_match=true`, gene order
identical, tick coverage **301/301**, no curves skipped (only `tags`/`metrics` are absent on both sides,
reported as gaps), events identity 1/1.

**What remains different is exactly the intended set: physics trajectories and pellet-RNG statistics.**
All ecology curves pass: pellet counts/biomass are statistical (first non-exact **pellet tick is 1**
because zone pellet size/position are drawn from the game's *unseeded* `UnityEngine.Random` at spawn, so
pellet populations cannot be bit-identical; max 22 pellets / 1198 biomass of ~1.95 M, MAE 14.9 / 62.6,
well inside the 10% band), `vitals.energy_mean`/`health_mean` differ by <= 3.05e-05 from tick 3
(float-rounding/1-ULP level), `pellets.bibite_biomass` is exact on 206/300 ticks, and
counts/species/generation/age/brain-size are exact for all ticks. Per-entity at the two snapshots:
genes/species/generation/tag/nodes/synapses/**output_digest** exact 2/2, **energy exact 2/2** (including
tick 300), health <= 3.05e-05, position MAE 9.8e-04 (max 2.0e-03), velocity MAE 5.3e-05 - the residual
physics divergence on this fixture is tiny but nonzero, as designed.

**Brain outputs: ALL 15/15 nodes × 300/300 ticks bit-exact** (4500/4500 values), reported per node in
`checks[brain_outputs].detail.per_output` (`{exact, n, max}` per output, plus `values_exact/values_n/
all_exact/non_exact_nodes`). This closes the earlier "Accelerate 1-ULP low" lead: the root cause was
**float rounding**, not the algorithm - Unity/Mono evaluate `Mathf.Tanh`/`Math.Tanh` etc. in f64 and cast
to f32, while Rust used the f32 intrinsics (`math::tanh` now does `(f64::from(x).tanh()) as f32`;
sigmoid/gaussian/exp/sin/pow follow `Mathf` semantics, pinned by a unit test). `compare.py` quantises
declared-f32 values to f32 bits before bit-equality (see §6.3), so a future 1-ULP regression is still
visible as a per-node exact-count drop rather than being hidden by tolerance.

Same-world `write_world_capture` vs `--save` check: `rustibites run ring_parity.zip --ticks 300 --seed
12345 --save ring_rust_after300.zip` loaded in the real game (`tools/roundtrip/load_rust_save.py`):
`inSimulation=true`, paused, counts `{bibites:1, corpses:0, eggs:0}` = both captures' `counts_end`; scene
`simulatedTime=20.200001` vs capture tick 300 `t=20.2000000104` (sub-tick load-clock quantisation);
template converter accepted (`updated=false`); 0 new Player.log exceptions. (The other round-trip result
`24/14/1` is a different world, `lol.zip`, so it is not a signal for this fixture.)

**Guidance:** `--profile default` is now the game-vs-Rust CI gate for this fixture (exit 0). Keep
`--profile exact` for game-vs-game replay, or for Rust captures restricted to deterministic windows with
`--until-tick`; a full-run `exact` diff will still fail on the tick-300 physics trajectory fields
(position/velocity/energy/brain_digest). Reports: `testdata/parity/rust_ring.compare.json`,
`testdata/parity/ring_rust_after300.zip`, `testdata/parity/roundtrip_ring_acceptance.json` (gitignored).

### Round-trip acceptance (file-format claim, 2026-10-30)

Separate from capture parity, the real game has been driven through the
BibitesAPI lab to start Rust-written artifacts (`tools/roundtrip/`):

- save: `load_rust_save.py` → loads `testdata/roundtrip/rust_save.zip`, counts
  `{24,14,1}` equal to the CLI final state, 0 new exceptions, template accepted
  by the game's converter; `testdata/roundtrip/acceptance.json`.
- scenario: `start_rust_scenario.py` → `new_game` from `rust_scenario.zip`
  (Ring.zip re-emitted by `write_scenario_bundle`); `inSimulation`, 4 zones and
  1 spawned bibite equal to an original Ring run in the same session, paused at
  `t≈0.267`, ran to `t≈1.267`, 0 new exceptions;
  `testdata/roundtrip/scenario_acceptance.json`. Pellet seeding is compared statistically (unseeded planting RNG): seeded amount within 2%, pellet and per-zone bucket counts within 3σ (floor 10), via /pellets/select; per-pellet identity is not compared.

These are load/start acceptance checks, not deterministic captures: use
`save_scene.simulatedTime` as the tick-0 reference; post-load `/status` can
differ by f32 quantisation of the clock label (nothing runs).

---

### 10.3 Gating policy for evolved worlds (hidden-node reload non-determinism)

The clean-load check accepts the game's **f32 clock quantisation** on evolved saves. The game keeps the
loaded `simulatedTime` as f32; on a save at `t≈470000 s` the nearest f32 is up to half an f32 ULP
(0.0156 s) from the stored double, so the post-load clock differs from `scene.bb8scene.simulatedTime`
by ~3.6e-03 s (0.055 ticks). `capture.py` classifies this as `load_kind="f32_quantised"` and accepts it
when the delta is sub-tick **and** equals `f32(save_scene_time) - save_scene_time`; it records the
reason as a taint and in `manifest.sim.{load_kind,load_delta_s,load_f32_delta_s,load_parity_clean}`.
Anything larger (a real integer-tick advance or an un-explained partial frame) still triggers reload
attempts and then a hard failure. The historical strict small-`t` path is kept via `--strict-load`
(rejects the quantisation), and `capture.py --self-check` covers both cases plus real-drift rejection
without starting the game. Ring (t=0.2 s) remains a `clean` load.

**Evolved-world gating policy.** A save with **hidden brain nodes** is not bit-reproducible in the game
itself: brain evaluation order is not saved (`SAVE_FORMAT.md`), so two loads evaluate hidden-node
networks in slightly different summation order. Measured on the W1 `lol.zip` fixture
(`docs/PARITY_ROUND2.md` §2): first differing tick record at tick 7 (Want2Heal, ~3e-9 = 1 ULP),
285/301 records drift numerically by tick 300, while counts/events are identical (34/34 lifecycle
events matched by tick/kind/id). Therefore:

* **Exact gating is meaningful only for lifecycle/genetics/topology/counts/events** on evolved worlds:
  genes/species/generation/tag/nodes/synapses digests, population counts, and the birth/death ledger.
* Brain outputs/curves and continuous vitals/biomass are **approximate** whenever the save has hidden
  nodes: use `--profile loose` / explicit value bounds there, not `--profile exact`; `--profile default`
  is appropriate for fixtures whose brains have no hidden nodes (e.g. the Ring base).
* Do not treat a game-vs-game numeric mismatch on an evolved save as a harness/Rust bug until the
  hidden-node evaluation-order effect is accounted for.

**W1 fixture normalisation.** Round 2 uses a copy of the real save under `testdata/parity/`
(`bases/lol_round2.zip`, produced by `tools/parity_round2/prepare_lol_base.py`) with
`scene.bb8scene.simulatedTime` set to its f32 representation and
`settings.bb8settings.independents.virginSpawnRate` 1→0 (unseeded load-time spawner). All other bytes
are preserved. **Both the game capture and the Rust capture must use the same copy**, because the
settings and the tick-0 reference are part of the fixture. Regenerate it with the script (the summary
`.base.json` records source/copy hashes) if the capture fixture ever changes.

## 11. Known limitations / open items

1. **RNG/stream parity**: `.NET System.Random` and the game's call order are not yet replicated by
   Rust. Genetics mutation, virgin spawning and any random draw will diverge until they are. Record
   mutation-dependent runs as statistical; use the `events` log for lineage statistics.
2. **Sparse per-entity data** in game captures (snapshot ticks only). If per-tick entity data is
   needed, the plugin can be extended with a `FixedUpdate` ring-buffer dump (small change; not done
   in v1 because aggregate per-tick telemetry + exact per-snapshot entities already answer the
   milestone-1 questions).
3. **Load-time spawners**: `virginSpawnRate=0` is forced into the base fixture. Runs that need virgin
   spawning must use a base with `--keep-spawners` and accept nondeterministic starts.
4. **`--attach`** exists for interactive debugging only; never capture ground truth from a game with
   a user's world in it.
5. **The plugin is third-party code in the game's BepInEx folder**; `dump_prefabs.py` removes it, but
   after an interrupted run check `<game>/BepInEx/plugins/ParityDump` is gone.
6. **Gravity/iterations**: physics config is dumped, but `Physics2D.autoSimulation`/sub-stepping
   settings matter for frame coupling; RUSTibites targets the fixed-step equivalent.

---

## 12. Rotation / torque sign convention (`probe_rotate.py`)

For body-dev's BQ-4 / world Q1 (which way does a positive brain Rotate output turn a bibite?).
Method (`tools/parity/probe_rotate.py`): take a living bibite from the Ring base, replace its brain with
a bias-only brain (all synapses removed, output node 34 = `Rotate` set to a Linear constant, all other
outputs 0), run 12 ticks at 1x and measure the change of `transform.rotation` (Unity degrees around +Z;
+ is counter-clockwise with +x right, +y up). Artifact: `testdata/parity/rotation_sign.json`.

| Rotate output | Δrotation over 12 ticks | trace (cumulative deg) |
|---|---|---|
| 0 (control) | **0.000°** | 0.00, 0.00, 0.00, 0.00 |
| **+1** | **+13.864°** | 4.16, 8.75, 11.81, 13.86 |
| **−1** | **−13.857°** | −4.15, −8.74, −11.81, −13.86 |

Verdict: **positive Rotate = positive angular velocity = counter-clockwise (toward the bibite's local
left when it faces +x); negative = clockwise/right.** `Rotate` does not flip the y axis anywhere on the
output→torque path (control stays exactly 0, the two signs are symmetric). Magnitude depends on the body
genes (SizeRatio/SpeedRatio/move muscles etc.), so only the sign is the invariant; the measured rate
here is ~1.15°/tick for this Ring-base genome at 15 TPS.

## 13. Quick reference

| path | purpose |
|---|---|
| `tools/parity/capture.py` | run a capture (headless game → gzip JSONL + manifest) |
| `tools/parity/compare.py` | diff two captures, `--profile`, `--report`, CI exit codes |
| `tools/parity/parity_common.py` | schema constants, digests, record builders, capture reader |
| `tools/parity/dump_prefabs.py` | deploy plugin → dump prefab constants → remove plugin |
| `tools/parity/plugin/` | the BepInEx plugin (ParityDump) source + csproj |
| `tools/parity/probe.py` | interactive bridge probe (per-tick sampler, snapshot, seed semantics) |
| `tools/parity/schema/capture-v1.schema.json` | machine-readable JSON Schema for captures |
| `testdata/parity/` | gitignored: bases, captures, prefab dump, build artifacts |


---

## Appendix A: behavioural experiments (task-18)

Three micro-experiments run with [tools/parity/exp_behavior.py](../tools/parity/exp_behavior.py) against
the installed game (0.6.3.1 / bridge 0.7.0 / Unity 6000.0.44f1), bridge-only (no plugin). All evidence is
under `testdata/parity/experiments/` (gitignored). Reproduce with
`python tools/parity/exp_behavior.py [--only color_killer|collision|phero]`.

### A1. ColorKiller polarity (world Q5 / DEVIATIONS Q3) — keep-the-band CONFIRMED

**Question:** does a ColorKiller kill bibites whose hue-distance is `>= tolerance` (keep the band, kill the
rest) or `< tolerance` (kill the band)?

**Method:** Ring base loaded paused (simTPS 15). Two bibites cloned from the base genome with pure colour
genes (`ColorR/G/B = 1,0,0` → hue 0.0, sat 1, val 1; and `0,0,1` → hue 0.6667), placed at x = ∓100,
`EyeOffset = 0`, zeroed brains. `POST /killer/add` at the origin (radius 5000, period 0.5 s) and 1.5 s at
speed 1. Five cases: four `bodyOrEye × anyOrMost` combinations for a red band (`hue=0, tolerance=0.1`),
plus an inverted case with the band on blue (`hue=0.6667, tolerance=0.2`) whose expected survivors differ.

| case | killer hue / tol | bodyOrEye | anyOrMost | dead | expected |
|---|---|---|---|---|---|
| keep_band_red_band | 0.0 / 0.1 | false | false | blue | blue |
| keep_band_red_band_anyMost | 0.0 / 0.1 | false | **true** | blue | blue |
| keep_band_red_band_bodyEye | 0.0 / 0.1 | **true** | false | blue | blue |
| keep_band_red_band_bodyEye_anyMost | 0.0 / 0.1 | **true** | **true** | blue | blue |
| keep_band_blue_band | 0.6667 / 0.2 | false | false | **red** | red |

**Result:** 5/5 cases match the "kill iff hue-distance >= tolerance" predicate (band is kept). The inverted
case is decisive: red (distance 0.333) died while blue (distance 0.167) survived, so the predicate is not
"kill the band". Pure colours make the `bodyOrEye=false` modulation `num5 = 1-(1-num5)*sat*val` a no-op;
`bodyOrEye=true` is identical with `EyeOffset = 0`; `anyOrMost` only changes *which* qualifying target is
picked (max distance vs first in the overlap buffer), not the predicate.

**Recommendation for RUSTibites:** keep world-dev's current `kill iff hue_distance >= tolerance`
(compiled `ColorKiller.FixedUpdate`: `if (!(num5 < tolerance)) KillTargetBibite(...)`). Do NOT flip it.
Evidence: `testdata/parity/experiments/color_killer_polarity/manifest.json`.

### A2. Collision damage sign (body BQ-4 / DEVIATIONS Q11)

**Question:** sign convention of `ContactPoint2D.normal` vs `Collision2D.relativeVelocity` in
`BibitePart.OnCollisionEnter2D`, and which body takes the damage.

**Method:** two zero-brain bibites spawned from the base genome with exact Rigidbody2D velocities (the
save-format JSON's `rb2d` block through `/entity/replace_many` + `/entity/spawn`), driven into a head-on
impact at 1x. Base settings: `collisionDamageConstant = 0.1`, `collisionDamageThreshold = 50`.
Health/attackedDmg/timesAttacked/totalDamageSuffered read before/after; left/right roles swapped; a 2x speed
case; and two attempts at mass asymmetry (SizeRatio gene and saved `fatReservesAmount`).

| case | closing speed | left damage | right damage | equal-mass prediction `(v-50)*0.1` |
|---|---|---|---|---|
| attacker_from_left (L +100, R 0) | 100.0 | 5.0 | 5.0 | 5.0 |
| attacker_from_right (L 0, R −100) | 100.0 | 5.0 | 5.0 | 5.0 |
| head_on_both (±100) | 200.0 | 15.0 | 15.0 | 15.0 |
| fat_heavy_hit_by_lean | 100.0 | 5.0 | 5.0 | — |
| lean_hit_by_fat_heavy | 100.0 | 5.0 | 5.0 | — |

**Result:** damage = `(closing_speed − threshold) × constant` exactly; **both** collision partners are
damaged on the same impact, each with a **positive** approach speed (one moving + one stationary body
damages both equally; reversing the roles swaps the bodies symmetrically). This pins the sign: for each
notified collider, `dot(contact.normal, Collision2D.relativeVelocity) == +closing_speed` (the contact
normal points toward the incoming body on each side), so RUSTibites must treat approach as positive and
apply the damage to both bodies.

**Bridge limitation (mass weighting NOT captured):** the bridge cannot read or set a bibite's physics mass
— entity JSON has no mass field, and `BibiteBody` recomputes `rb2d.mass = bibiteMassDensity·baseBodyArea`
from organ state every tick, so changing `SizeRatio`/`fatReservesAmount` in the spawn JSON did not change
the observed damage (all equal-mass cases). The compiled formula weights by the *other* body's mass
(`num2 = 2·m_other·closing/(m_other+m_this)`); implement it from the C# and validate later. Minimal plugin
hook needed to capture it: `GET /d/bodies -> [{handle,id,mass,baseBodyArea,d2Size}]` and
`POST /d/mass {handle,mass}` that sets `BibiteBody.baseMass` (or `rb2d.mass`) before the next tick — the
existing ParityDump plugin already has the game-side access for this, only the routes are missing.
Evidence: `testdata/parity/experiments/collision_sign/manifest.json`.

### A3. Pheromone decay (world Q10 / DEVIATIONS Q2) and save heading loss (Q4)

**Method:** a bibite whose brain is a bias-only `PheroOut1 = 0.5` (all synapses removed) emits one
pheromone spot after 1 s (`pheromoneProductionStrength = 10` ⇒ expected emitted `Rstrength = 5.0`). The
emitter is deleted; the world is saved (the bridge has no pheromone read route) every ~0.53 s and a 4 s
window is added; the saved spot's `Rstrength` is read from `pheromones.bb8scene`. Then checkpoint 01 is
reloaded and re-saved to test the `heading` field.

| t (sim s) | 1.453 | 1.987 | 2.520 | 3.053 | 3.587 | 4.120 | 8.120 |
|---|---|---|---|---|---|---|---|
| Rstrength | 4.9375 | 4.8125 | 4.6875 | 4.5625 | 4.4375 | 4.25 | 3.25 |

**Result:** `Rstrength` falls **0.25 u²/s in 0.25 s quanta** (−0.0625 per quantum), not 1 u²/s: the
4.000 s window lost exactly 1.0 strength (16 quanta), and the whole-series slope is
**−0.2531 u²/s** (sampling quantum-phase jitter ±1 quantum explains the 1.2% deviation; e.g. the
4.25→3.25 step is exactly 1.0/4.0 s). The spot is destroyed once all three strengths reach 0. Saved
`phero.heading` is **`{}`** and stays `{}` after save→load→save, while the strengths round-trip — the
pheromone heading is lost on save/load (the spot's strength/position are preserved). Confidence: high for
decay and heading. Evidence: `testdata/parity/experiments/phero_decay/manifest.json` (+ `checkpoints/`).

**Recommendation:** world-dev: decay `−0.0625` per 0.25 s quantum (i.e. 0.25 u²/s), quantised, with
destruction at ≤0; on save write `heading: {}` and on load use the default/absent heading (Q4).


---

### 6.y Appendix D status note (task-101)

Appendix D above was merged from `docs/design/FEEDING_METRICS.md` as contract text; its 'conservation stays ungated' wording is **historical**: after task-101 the caller-path exact game hooks + Rust task-99 counters are in place, `compare.py` gates conservation per-side (evidence `harmony_exact` / `counters`) and the canonical Default reports pass on both sides: game residual −3.8e−06 E (600t) and 256.07 E / 0.2464% (3000t), Rust residual 2.09e−04 E (600t) / 1.39e−04 E (3000t), relative ~2e−9. `feeding_rates` remains a normal gate (currently failing: Rust under-consumption, task-93).

## 12. Feeding capture group + default-world policy (2026-10-30)

The canonical world is `testdata/real/Default.zip` (`docs/TESTING_POLICY.md`); Ring/lol captures in
§10 and team docs are historical/diagnostic fixtures. For feeding/energy work use the optional
`capture.py --feeding` group/stream (`rustibites.parity.feeding.v1`, `evidence: polling_proxy`, raw
snapshot + per-tick `behavior.*` polling proxies) plus the plugin's exact-ish cumulative counters
(`produced_energy_exact` works; consumed/recycled/decayed are currently 0 due to an inlined
`MatterPellet.RemoveAmount` hook — see `docs/design/FEEDING_METRICS.md` and `docs/PARITY_ROUND2.md`
§10). Gates: feeding rates/consumed amounts are gated at 10% default / 25% loose with absolute floors
near zero; `bite_rate_mean` is legacy/informational. **Update (task-101, 2026-10-30):** the
caller-path Harmony hooks (BibiteMouth.MaybeAttack / MatterDecayProcessor.FixedUpdate /
Zone.RecyclePellets) and Rust task-99 exact counters have landed, so `compare.py` now gates
per-side conservation (game `harmony_exact`, Rust `counters`) at 1% relative / 1 E absolute;
canonical Default PASSES on both sides (see docs/PARITY_ROUND2.md §10).

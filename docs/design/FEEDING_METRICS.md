# Feeding and energy measurement contract

**Status:** design / contract (docs-only) — **Lead-approved 2026-10-30**: the §6 schema and §7
tolerances are approved; Tier B scope = polling-only + cumulative zone counters (the exact
`onMatterEnter`/`OnAttack` event tap is deferred as an optional future upgrade); `docs/PARITY.md` stays
parity-dev-owned — the suggested §6 patch is in Appendix D. **Canonical world:** `testdata/real/Default.zip`
per `docs/TESTING_POLICY.md`; Ring/lol are historical-fixture regressions (§8). **Owner:** spec-dev
(task-97/88/90/93/94 follow-up). World-dev task-99 (Rust conservation accounting) approved 2026-10-30.
**Consumers:** body-dev (engine counters, task-90/93), world-dev (task-99 zone/pellet accounting),
parity-dev (capture schema, game-side feeding evidence, compare gates, task-94), Lead (scope decisions).
**Implementation status (2026-10-30):** Rust task-90 counters/behavior fields landed (`eb5294d`); world-dev
**task-99 conservation accounting landed and Default-validated** (commit `02fb13a`; residual ≈2e-9
relative; tests green); parity-dev compare `feeding`/`conservation` sections landed (`77f7613`,
self-tests pass) and caller-path exact hooks landed (`c2665d1`: game 600 t residual −3.8e-6 E,
rel −4.0e-11; 3000 t 0.2464 %). **Conservation gate is live and two-sided (`7764d6f`):** game
`harmony_exact` PASS at 600/3000 t; Rust `counters` PASS at 600/3000 t (`gated_sides=[left,right]`);
per-side policy (game requires `harmony_exact`, Rust accepts `counters` with the task-99 conservation
object).

**What this file answers** (the five asks that started it):

1. **(a)** a counter catalogue — plant/meat/bibite bites, swallows, consumed `u²`, stomach fill/drain,
   energy gained/drained, per-live-bibite rates — with units, definitions and instrumentation sites;
2. **(b)** the conservation identities (zone production + initial mass − final mass = consumed + losses),
   term by term, with how each side is measured today and what is still missing;
3. **(c)** game-side mapping/feasibility and Rust-side mapping (what the bridge, save-format entity JSON
   and the parity plugin can expose, and what is infer-only);
4. **(d)** recommended schema names/units for stats/capture and compare tolerance bands;
5. **(e)** the validation plan for Ring (post spawn-placement fix) and the evolved lol save (task-93),
   with the expected envelopes from `docs/PARITY_ROUND2.md` §8.

**Goal.** One vocabulary and one conservation check for feeding/energy, so RUSTibites and the game are
compared on *fluxes*, not on stock curves that zone production masks. The round-4 verdict showed why:
`bite_rate_mean` is a bibite-vs-bibite counter (zero on Ring on **both** sides), and plant stock floats
at the zone ceiling (2.44 M E) while real feeding happens.

**Sources of truth:** Rust `crates/bibites-core/src/{body,organs,sim,zone,pellet,world,physics}.rs` and
`crates/bibites-cli/src/capture.rs`; game `Here/SimulationScripts.BibiteScripts/{BibiteMouth,
BibiteStomach,BibiteBody}.cs` and `Here/SimulationScripts/Zone.cs`; harness `docs/PARITY.md` §3–§6,
`tools/parity/{capture,compare,feeding_ref}.py`, `tools/parity/plugin/Plugin.cs`, `docs/PARITY_ROUND2.md`
§8, `docs/ECOLOGY.md`, `docs/spec/04-formats-settings.md` §4.3.

**Confidence tags:** `[C]` verified in code/decompiled C#; `[I]` inferred from save/bridge format;
`[P]` proposal; `[?]` open question.

---

## 0. TL;DR and current state

1. **Feeding is implemented; counters are not.** Task-88 proved template/runtime-spawned bibites have
   working mouth colliders/triggers and eat when a pellet is in bite range. The Ring "never eats" signal
   was spawn placement; world-dev task-85 has landed the per-shape `Zone::spawn_location` spawner (and the
   regression tests are green — see `PARITY_ROUND2.md` §8.3). This document does not imply a feeding-path
   fix.
2. **Rust has no pellet-feeding counters yet.** `bite_rate_mean` is `bibitesBitten / age` (bibite bites
   only; recently confirmed to be incremented in `tick_mouth`, see §2.5), so it is legitimately 0 on Ring.
   `tick_mouth_pellet` performs the transfers but records no plant/meat bite or consumed-mass counter.
3. **Game-side evidence now exists (task-94, implemented + addendum `7a5ea90`).** parity-dev added a
   transient plugin route `GET /d/feeding` plus `capture.py --feeding` polling, producing a raw
   per-bibite/per-zone snapshot stream (`rustibites.parity.feeding.v1`) and `tools/parity/feeding_ref.py`
   sums it into intake proxies. It exposes per-bibite stomach contents by material, held pellets, mouth
   bite timers/counters, `total_murders`/`murdered_area`, energy/health, per-zone pellet stock +
   production **rates**, and **polling-census cumulative counters** (`produced_energy_cumulative` lower
   bound, `removed_energy_cumulative` mixed-cause, `recycled`/`decayed` explicitly null with
   `missing_terms`). The game-side conservation identity is therefore still **not gateable** until exact
   cause-attributed accumulators land (§4.4–§4.5).
4. **Stock is not the metric.** Ring/lol worlds sit at/near the zone biomass ceiling, so production
   refills what is eaten. Primary validation = **plant mass conservation from counters** + engine rates;
   stock is a cross-check. Ring is a poor feeding-rate world (tiny consumption against a 2.4 M E stock);
   evolved lol is the per-capita feeding gate.
5. **Plan.** Canonical validation is the Default scenario (§8.2); Ring (post-spawn-fix) and lol (task-93)
   are historical-fixture regressions (§8.3/§8.4). body-dev adds runtime counters + interval stats
   (task-90, §5.2); world-dev **task-99 (approved 2026-10-30)** adds zone production/recycle/decay/cull
   energy accounting plus seeded energy (§3.4); parity-dev adds derived `behavior` rates to the game
   capture and the `compare.py` feeding/conservation sections (§7, follow-up), then integrates the §6
   schema into `docs/PARITY.md` via Appendix D. Conservation becomes gateable once body task-90 +
   world task-99 land (Rust) and the game-side exact accumulators are eventually added (deferred).

---

## 1. Units and vocabulary

| Name | Symbol / unit | Meaning |
|---|---|---|
| Energy | `E` | Pellet/body energy. `E = amount_u2 × energyDensity`. |
| Matter amount | `u²` | Physical pellet/stomach amount. `amount_u2 = E / energyDensity`. |
| Plant density | `5 E/u²` | `MaterialSetting::PLANT.energy_density` (`settings.rs`). |
| Meat density | `10 E/u²` | `MaterialSetting::MEAT.energy_density`. |
| Pellet radius | `r = sqrt(amount_u2 / π)` | `MaterialParams::radius_of_amount` (`organs.rs`). |
| Tick | `dt = 1 / sim_tps` | Sample interval; always report seconds too. |
| Live second | `bibite·s` | One living bibite for one simulated second; rate denominator. |
| Consumption | `consumed_P`, `consumed_M` | Matter transferred out of a pellet **into a stomach**, in `u²` and `E`. |
| Rip / drop | `ripped_P`, `ripped_M` | Matter bitten off a pellet into a **new ground pellet** (not stomach). |
| Swallow | `swallow` | Whole-pellet transfer (`TrySwallowTarget` success); distinct from a chunk bite. |
| Decay | `decayed` | Matter destroyed by material decay (`process_decay`; plants don't decay, meat does). |
| Cull | `culled` | Matter removed by the shade/out-of-bounds census (`census_and_cull`). |
| Recycle | `recycled` | Matter destroyed by a zone over-capacity pass (`Zone.RecyclePellets`). |
| Production | `produced` | Matter/energy created by zone `SpawnAPellet`. |
| Throughput | `produced + consumed` | Denominator for conservation-residual tolerances. |

**Conventions**

* Cumulative counters are **monotonic, runtime-only** (never serialised into `.bb8`/save files unless
  explicitly agreed; the game does not persist plant/meat feeding counters either).
* Report both `u²` and `E`. Do not derive one at compare time when a counter exists: the engine converts
  at four places (pellet spawn, bite removal, stomach add, digestion), and drift in any one is a finding.
* Rates are per **live bibite-second**, not per bibite lifetime (§2.4). Keep the legacy
  `bite_rate_mean = bibitesBitten/age` only for backward compatibility; it is not a feeding gate.

---

## 2. Counter catalogue

Names are the recommended canonical JSON names; Rust field names are per row (task-90). Totals are
cumulative for the run; `Δ` means "since the previous stats sample" — captures emit Δ (like `lifecycle`).

### 2.1 Mouth actions and matter transfer

| Canonical counter | Unit | Type | Definition |
|---|---|---|---|
| `plant_bite_events` | count | `u64` | Successful plant-pellet chunk bites that moved matter (stomach or ground chunk). One per bite action, not per tick. |
| `meat_bite_events` | count | `u64` | Same for meat pellets. |
| `bibite_bite_events` | count | `u64` | First bites on another bibite (game `bibitesBitten`; Rust `MouthState.bibites_bitten`). Ongoing-drain ticks are not counted. |
| `bibite_bite_drain_ticks` | count | `u64` | Ticks of an ongoing bite (`attackedLastFrame == true`). Diagnostic. |
| `plant_swallow_events` | count | `u64` | Whole plant pellets swallowed (small/fragmented pellets only, see Appendix A). |
| `meat_swallow_events` | count | `u64` | Whole meat pellets swallowed. |
| `bibite_swallow_events` | count | `u64` | Whole bibites swallowed (`BibiteSwallow::Swallow`). |
| `plant_consumed_u2` | `u²` | `f64` | Matter transferred to stomach from plant pellets (swallow + chunk bites). |
| `meat_consumed_u2` | `u²` | `f64` | Same for meat. |
| `bibite_consumed_u2` | `u²` | `f64` | Meat + stomach matter gained by swallowing a whole bibite. |
| `plant_ripped_u2` | `u²` | `f64` | Plant matter ripped off as a new ground pellet (`rip_chunk`; `desire_attack > desire_eat`). |
| `meat_ripped_u2` | `u²` | `f64` | Same for meat. |
| `bibite_damage_dealt` | HP | `f64` | Sum of bite damage on bibites (game `totalDamageDealt`; Rust `MouthState.total_damage_dealt`). |
| `bibite_kills` | count | `u64` | Victims killed by bites (game `totalMurders`). |
| `plant_bite_attempts` | count | `u64` | Contact ticks where a plant pellet was in the mouth but no matter moved (too big/no space/not ready). Optional diagnostic. |

**Attribution rules**

* Attribute to the **eater**, never the pellet.
* A bite exists only when matter moves: `rip_chunk` success counts `*_bite_events + *_ripped_u2`; direct
  `change_amount`/`remove` counts `*_bite_events + *_consumed_u2`. Zero-transfer attempts are not bites.
* Plant/meat split follows the **target pellet material**, not the biter's diet.
* `bibite_consumed_u2` derives from the victim's meat + stomach contents at swallow time
  (`resolve_swallow`), at the target material density.
* **Do not count a whole swallow as a bite.** Swallows and bites are separate counters; total feeding
  events = bites + swallows.

### 2.2 Stomach, fill and drain

| Canonical counter | Unit | Type | Definition |
|---|---|---|---|
| `stomach_fill_u2` / `_mean` | `u²` | `f32` | `Stomach::total_amount()` summed (and per-living-bibite mean) at sample time (`stomach_total` in the plugin stream). |
| `stomach_digested_u2` | `u²` | `f64` | `Δ` matter dissolved by digestion over the interval. |
| `stomach_energy_gain` | `E` | `f64` | `Δ` `DigestionTick.energy_gain` summed over living bibites (after conversion efficiency). |
| `stomach_energy_gain_rate` | `E/s` | `f32` | Current `Stomach::energy_gain_rate()` (diagnostic sample). |
| `digest_acid_level` / `digest_effectiveness` | ratio | `f32` | Optional (`DigestionTick`), only if task-93 needs digestion diagnostics. |

Instrumentation: `tick_organs` (`body.rs`) computes `digest.energy_gain` and immediately calls
`record.body.gain_energy(digest.energy_gain)`; nothing retains the sum. Add the accumulator at that site
(or return an extra `dissolved` field from `Stomach::update`).

### 2.3 Energy budget

| Canonical counter | Unit | Type | Definition |
|---|---|---|---|
| `energy_gained` | `E` | `f64` | Energy added via `BodyState::gain_energy` (digestion, fat return, growth refunds). |
| `energy_spent` | `E` | `f64` | Energy removed via `BodyState::use_energy` (metabolism, movement, growth, egg, fat, pheromone, heal), including health fallback. |
| `energy_in_digestion` | `E` | `f64` | Subset of `energy_gained` equal to the summed `stomach_energy_gain`. |
| `energy_spent_health_fallback` | `E` | `f64` | Energy-equivalent of the health overdrawn in `use_energy` (`spend.health_damage × health_material_density`, `EnergySpend`). Diagnostic. |
| `body_energy_stock` | `E` | `f32` | Sum of living `body.energy.amount` (cross-check only). |
| `total_biomass` | `E` | `f32` | Existing `PelletStats.bibite_biomass` = body+health+egg+fat−corpse offset; keep as-is. |

Egg/fat/health/corpse conversions are energy-budget diagnostics (task-93), not feeding gates. Keep them
separate so a feeding mismatch is not hidden inside a total-energy mismatch.

### 2.4 Per-live-bibite rates

```
rate(C) = C / live_bibite_seconds
live_bibite_seconds = Σ over ticks of (n_living_at_tick × dt)
```

For per-tick samples this is exact; for statistical captures integrate the `counts.bibites` curve
(rectangle rule) and flag it, or average per-sample means. **Do not** use a bibite's lifetime age as the
denominator for feeding rates.

Recommended derived fields (all `f64`): `plant_bites_per_s`, `meat_bites_per_s`, `bibite_bites_per_s`,
`bibite_damage_per_s`, `plant_consumed_u2_per_s`, `meat_consumed_u2_per_s`, `plant_consumed_e_per_s`,
`meat_consumed_e_per_s`, `stomach_energy_gain_per_s`.

### 2.5 Current implementation audit — Rust task-90 landed (`eb5294d`); task-99 in progress

| Metric | Today | Gap for this contract |
|---|---|---|
| `MouthState.bibites_bitten` | Incremented on a first bite: `record.mouth.record_first_bite()` (`body.rs:3972`, `organs.rs:1538-1541`); restored on load (`body.rs:2348`). | None. Task-90's "never incremented" premise is stale for this field. |
| `MouthState.total_damage_dealt` | Summed per landed bibite bite (`body.rs:3997`). | None. |
| `MouthState.total_murders`, `murdered_area` | Fields exist (`organs.rs:1459-1462`). | Runtime mirror `bibite_kills` now in `FeedingCounters`; game plugin feed exposes `total_murders`/`murdered_area` (task-94 addendum). |
| Plant/meat pellet bites, swallows, consumed mass | **Implemented (task-90, `eb5294d`):** runtime-only `BibiteRecord.feeding: FeedingCounters` (`body.rs:1827-1902`) records plant/meat/bibite bites, swallow events, consumed `u²`, ripped `u²`, damage/kills, plus stomach/energy counters; counted in `tick_mouth`/`tick_mouth_pellet`/`tick_organs`/`gain_energy`/`use_energy`. | Optional extras not implemented: `*_bite_attempts`, `bibite_bite_drain_ticks` (§2.1) — diagnostic only. |
| Stomach fill/drain | `Stomach::total_amount()` sampled; `stomach_digested_u2` + `stomach_energy_gain_e` accumulated per record (task-90); `BehaviorStats.stomach_energy_gain_per_s` emitted (`sim.rs:601-617`). | None for the catalogued fields. |
| Energy gained/spent | `energy_gained_e`/`energy_spent_e` accumulated from `BodyState::gain_energy`/`use_energy` (task-90; `FeedingCounters` fields 1859-1861). | Health-fallback split (`energy_spent_health_fallback`) not separately accumulated (diagnostic only). |
| Zone production | `spawn_ap_pellet` computes exact `energy` (`zone.rs:1190-1245`); `ZoneUpdateReport` still `{spawned, recycled, capped}` (`zone.rs:767-776`); `update_production` subtracts progress (`:1139-1188`). | **Task-99 (world-dev, approved/in progress):** add `produced_energy`, `recycled_energy`, seeded energy. |
| Pellet decay | `DecayReport` type + `process_decay` return landed in the world-dev working tree; `World::step_decay` returns it (`world.rs:3676`). | **Task-99 completion:** aggregate into stats/capture conservation terms + tests. |
| Pellet cull | `census_and_cull` returns count (`world.rs:3396-3416`). | **Task-99:** culled energy/amount. |
| Pellet store paths | `remove/change_amount/remove_amount/rip_chunk` handle amounts (`pellet.rs:1030,1054,1107,1186`); body feeds directly via `ctx.pellets`, bypassing `World::despawn_pellet` zone bookkeeping (zone recount reconciles). | Task-99 decides whether store-level counters are needed beyond the body counters + report terms. |
| `PelletStats` | plant/meat n + biomass + free/bibite biomass (`sim.rs:496-510`); stock cross-check only. | Keep; don't overload. |
| `BehaviorStats` | `bite_rate_mean` from `bibites_bitten/age` (`sim.rs:799-808`); **task-90 landed (`eb5294d`)**: `plant_bites_per_s`, `meat_bites_per_s`, `bibite_bites_per_s`, `bibite_damage_per_s`, `plant_consumed_u2_per_s`, `meat_consumed_u2_per_s`, `plant_consumed_e_per_s`, `meat_consumed_e_per_s`, `stomach_energy_gain_per_s` (`sim.rs:601-617`), fed by `FeedingInterval` (`sim.rs:646-666`). | Wire the capture-side `feeding`/counters groups to compare.py and the game side (parity-dev follow-up); task-99 conservation terms. |
| Game-side feeding evidence | Plugin `/d/feeding` + `capture.py --feeding` implemented (task-94, §4.3); addendum `7a5ea90` adds polling-census cumulative counters + per-bibite murders (§4.4); `feeding_ref.py` derives stomach-delta intake proxies. | Game conservation still ungated: produced is a lower bound, removed is mixed-cause, recycled/decayed null; no per-event identity; polling cadence ≈ 0.05 s, not tick-exact; derived `behavior` fields not yet on tick records. |

---

## 3. Conservation identities

These are the **primary parity gate**: they combine stock, production and counters, so any silent
loss/duplication in the four conversion points shows up. Plant u² is the hard gate; meat/energy are
diagnostic.

Notation for interval `[t0, t1]`; stock is the summed live-pellet amount for that material:

```
stock(t), produced, created_meat, consumed, ripped (neutral within a material),
decayed, culled, recycled
```

### 3.1 Plant matter identity (hard gate)

Plants don't decay by default (`PLANT.decay == false`):

```
residual_plant_u2 = (stock_plant_u2(t1) - stock_plant_u2(t0))
                  - (produced_plant_u2 - consumed_plant_u2 - culled_plant_u2 - recycled_plant_u2)
```

`rip_chunk` is mass-neutral for plants (chunk becomes another plant pellet); if that chunk is later
culled/decayed it appears on the right-hand side. Expected `residual ≈ 0` (f32 rounding only).

Energy form: replace u² with E (per-material density). This catches a density mismatch between spawn and
bite paths.

### 3.2 Meat matter identity (diagnostic/soft gate)

```
residual_meat_u2 = (stock_meat_u2(t1) - stock_meat_u2(t0))
                 - (created_meat_u2 - consumed_meat_u2 - decayed_meat_u2 - culled_meat_u2 - recycled_meat_u2)
```

`created_meat_u2` sums corpse/meat-pile creations (`BibiteStore::die_now` → `spawn_meat_pile_with_radius`,
`body.rs:4520-4527`; the game's kill path also spawns meat). Energy form uses `corpse_meat_energy(...)`
already computed there. Meat decay is enabled (`MEAT.decay == true`, fresh 2000 s), so the decay term
matters.

### 3.3 Energy identity (diagnostic; not a hard gate)

```
produced_E + created_meat_E - consumed_E - decayed_E - culled_E - recycled_E
  ≈ stock_E(t1) - stock_E(t0)
```

Do not attempt whole-world energy closure as a gate: growth, health↔energy fallback, egg
investment/hatching, fat conversion, pheromone cost and corpse offsets are lossy by design. Report the
feeding-specific ratio instead:

```
digestion_efficiency_observed = stomach_energy_gain / consumed_E
```

Expected `conversion_efficiency(affinity)`, where
`conversion_efficiency = min_eff + affinity × (max_eff − min_eff)` (`organs.rs`), `PLANT` −0.25..0.5,
`MEAT` 0.15..0.85, affinity = `(1−diet)^plant_power` / `diet^meat_power` (`Stomach::affinity`). Examples:
pure herbivore plant = **0.5** (meat 0.15); pure carnivore meat = **0.85** (plant −0.25). Digestion also
applies acid/effectiveness modifiers, so use a band: observed ≈
`conversion_efficiency × (1 − efficiency_malus)`.

### 3.4 Measuring each term

| Term | Rust today → needed | Game (task-94 harness) |
|---|---|---|
| `stock` | `PelletStore::counts()` (`pellet.rs:688`); capture `pellets` group. | Bridge per-tick `pellets` group; plugin `zones[].plant_amount/meat_amount/pellet_biomass` + `totals`. |
| `produced` | **Landed (task-99, world-dev, 2026-10-30):** `ZoneUpdateReport::{produced,recycled}_{plant,meat}_energy`; `SeedingReport::{seeded_plant,seeded_meat}_energy`; `World::conservation_totals()/conservation_snapshot()`, `ConservationTotals::plant_energy_residual(...)`, `StatsSample.conservation`. Default-600: seeded 94,309.8082 E, produced 1,679.472 E, consumed 90.6048 E, stock 95,898.6751 E → residual ≈ 2.1e-4 E (2.2e-9 relative). | **Exact `produced_energy_exact`** (Harmony on `SpawnAPellet`, unchanged): 1,773.06 E/600 t; 9,633.43 E/3000 t. Polling-census variant remains as a lower-bound fallback/evidence. |
| `created_meat` | Corpora→meat creation is **body-owned and not surfaced** (world-dev's `terms_missing=true` on Default, where no red death occurs); add a counter in `die_now`/`spawn_meat_pile*` when the meat identity is needed. | Plugin pellet stock per zone by material at each poll (deltas), no cumulative creation counter. |
| `consumed` | **Landed (task-90, `eb5294d`):** `FeedingCounters.plant/meat/bibite_consumed_u2` + `stomach_*`, aggregated into `BehaviorStats` rates and the capture feeding stream; task-99 captures the same terms in the `conservation` object (`consumed_plant/meat_u2/energy`). | **Exact caller-path hook (`c2665d1`):** stomach `totalAmount` Δ × material density in `BibiteMouth.MaybeAttack` = 364.25 E/600 t, 3,373.53 E/3000 t. Stomach-delta polling proxy remains for non-plugin captures. |
| `decayed` | **Landed (task-99):** `pellet::DecayReport{removed_pellets,plant_amount,meat_amount}` from `process_decay`/`step_decay`, aggregated into `ConservationTotals::decayed_{plant,meat}_energy` (Default has no plant decay; meat decay only). | **Exact caller-path hook (`c2665d1`):** `MatterDecayProcessor.FixedUpdate` pellet-list energy before/after; 0 on grass-only Default (meat-decay world needed to exercise it). |
| `culled` | **Landed (task-99):** `census_and_cull` energy summed in `ConservationTotals::culled_{plant,meat}_energy` (Default: 0). | Not hooked yet (0 on Default); the 3000 t 0.246 % residual is likely this/death-boundary term — optional meat/decay-world capture to close. |
| `recycled` | **Landed (task-99):** `ZoneUpdateReport.recycled_{plant,meat}_energy` from `recycle_pellets`/`update_production`, in `ConservationTotals` (Default: 0). | **Exact caller-path hook (`c2665d1`):** `Zone.RecyclePellets` zone pellet energy before/after; 0 on Default (not at over-cap). |

### 3.5 Residuals and tolerance defaults

```
residual_u2  = stock_end - stock_start - (produced - consumed - decayed - culled - recycled)
residual_rel = |residual_u2| / max(1.0, produced + consumed)
```

| Context | Default tolerance |
|---|---|
| Deterministic Rust-vs-Rust (`--profile exact`) | `residual_u2 == 0` after f64 accumulation; all counters bit-identical. |
| **Rust conservation (Default, task-99 — gated and PASSING)** | **PASSES with large margin:** gated residual 2.0885e-4 E (2.1758e-9 relative) at 600 t and 1.3947e-4 E (1.3422e-9 relative) at 3000 t vs the ≤1 % throughput gate (`7764d6f`, `gated_sides=[left,right]`). `terms_missing` is only corpse→meat creation (body-owned) + red-death refused-spawn energy (no red death on Default); document it, don't gate the meat identity until that term exists. |
| Deterministic game-vs-Rust (`--profile default`) | `residual_rel ≤ 0.01`; `|residual_u2| ≤ 0.001 × stock_start`; energy residual ≤ 2 % of throughput. **Both sides gated and PASS (`7764d6f`):** game 600 t −3.8e-6 E (rel −4.0e-11, absolute floor) and 3000 t 0.2464 % (decayed/recycled unhooked, grass-only Default); Rust 2.0885e-4 E (2.1758e-9) and 1.3947e-4 E (1.3422e-9). `missing_terms` visible; per-side evidence policy (game `harmony_exact`, Rust `counters`). |
| Statistical 50× envelopes | Report residual; gate only "Rust in the same band as game under the same protocol" (sampling, D18 pellet RNG dominate). |
| Ring at the biomass ceiling | Absolute floors: `|residual_u2| ≤ 1`, `|residual_E| ≤ 10` (throughput is tiny → relative gate is brittle). |

---

## 4. Game-side mapping and feasibility

### 4.1 What the bridge/plugin exposes today `[C]`

* **Bridge per-tick sampler** (`docs/PARITY.md` §6.1): `counts`, `pellets` (`plant_n/plant_biomass/
  meat_n/meat_biomass/free_biomass/bibite_biomass`), `vitals`, `generation`, `species`, `brain`, `genes`,
  `lifecycle`, `tags`, `behavior`, `body`. Sampled inside `FixedUpdate`, tick-exact.
* **Bridge per-snapshot entity JSON** (`/entities/json`, save format): mouth counters, stomach contents,
  energy/health etc. are present in the file (see §4.2); capture.py does not extract them yet.
* **Transient parity plugin** (`tools/parity/plugin/Plugin.cs`, task-94): `GET /d/feeding` returns a
  live snapshot (`rustibites.parity.feeding.v1`); `capture.py --feeding` polls it while the world runs and
  writes `feeding.jsonl[.gz]` plus an optional `feeding` group on matching tick records and a manifest
  `feeding` block. `tools/parity/feeding_ref.py` summarizes it.
* **Events** (`GET /events`): the game's birth/death/species notification log; **not** a bite stream.

**Not exposed:** per-event bite identity/amount, cumulative created/recycled amounts, per-pellet
identity, and (without the plugin) any per-bibite feeding state. Plan around these (§4.5).

### 4.2 Save-format entity JSON `[C]`

`BibiteBody.SaveState` (`BibiteBody.cs:951-975`) writes these sub-objects (documented in
`docs/spec/04-formats-settings.md` §4.3); `/entities/json` returns them verbatim:

| Key | Fields | Feeding use |
|---|---|---|
| `body.mouth` | `attackedLastFrame`, `biteProgress`, `bibitesBitten`, `totalDamageDealt`, `totalMurders`, `murderedArea` | Cumulative bibite-bite/damage counters, bite readiness. |
| `body.stomach` | `content[] {material, amount, averageChunkAmount}` (max 2) | Stomach fill; combined with snapshots gives intake proxy. |
| `body.energy`, `body.health` | floats | Energy/health trajectory cross-check. |
| `body.fatReservesAmount`, `body.eggLayer`, `body.control.totalTravel` | numbers | Energy-budget diagnostics. |

Extending `capture.py`'s `entity_record` with these fields is a small schema-compatible Tier-A win; the
plugin route already covers the same data at much higher cadence, so this is only needed for
plugin-less/`--no-plugin` captures.

### 4.3 Implemented game-side feeding evidence (task-94) `[C]`

The plugin samples live game state by reflection (no game assembly reference, no patches). Route:
`GET /d/feeding` → object with `schema: "rustibites.parity.feeding.v1"`, `t` (simulated time), `plugin`,
`gameVersion`, and:

| Field | Content |
|---|---|
| `bibites[]` | `id`, `energy`, `health`, `d2_size`, `pos`, `stomach_total`, `n_stomach`, `stomach_materials{material: amount}`, `held_pellets[{amount, material, held}]`, `n_held`, `bite_progress`, `bite_period`, `bibites_bitten`, `damage_dealt`, **`total_murders`, `murdered_area`** |
| `zones[]` | `pellet_biomass`, `max_biomass`, `free_biomass`, `pellet_production` (E/s), `pellet_progress`, `radius`, `pos`, `pellets_n`, `plant_amount`, `meat_amount`, **`produced_energy_cumulative`, `removed_energy_cumulative`, `culled_energy_cumulative`, `recycled_energy_cumulative` (null), `decayed_energy_cumulative` (null), `counters_evidence`** |
| `counters` (top level) | **`initial_energy`, `produced_energy_cumulative`, `removed_energy_cumulative`, `culled_energy_cumulative`, `recycled_energy_cumulative` (null), `decayed_energy_cumulative` (null), `consumed_energy_estimate_cumulative`, `current_pellet_energy`, `evidence: "polling_census_proxy"`, `missing_terms[]`, `note`** |
| `totals` | `pellets_n`, `plant_amount`, `meat_amount` |
| `material_properties` | `{material: {energy_density, mass_density, amount_at_scale1}}` |
| `error` | only when reflection fails |

`capture.py --feeding --feeding-period 0.05` starts a polling thread during the run, stores snapshots by
`t`, attaches each to the nearest tick record as an optional `feeding` group, writes
`feeding.jsonl[.gz]`, and adds a manifest `feeding {requested, available, schema, file, samples,
poll_period_s, note}` block. It also adds a taint entry: *polling adds periodic main-thread HTTP requests
while the world runs; dt/RNG are unchanged (fixed-dt pin) but wall pacing is perturbed.*

`feeding_ref.py` converts the stream into `feeding_summary.json` + `feeding_series.csv`:
per-material **intake proxy** (positive stomach-amount increments between polls; count = proxy "bites"),
consumed amount and gross chemical energy (× `energy_density`), per-second and per-bibite rates,
stomach/held/energy/health trajectories, per-zone stock start/end, and an explicit
`inferable {yes, no, recommended_metric}` block.

**What this proxy is and is not:**

* It measures matter that *stayed in the stomach between two polls*. Matter consumed and fully digested
  inside one poll interval is invisible; a bite that is immediately vomited/spat is invisible. Polling at
  0.05 s (~3 ticks at 1×) keeps this small but not zero.
* The "bite count" is a count of positive stomach increments, not real bite events: multiple small bites
  between polls merge; one bite can add to two material buckets. Gate on **consumed amount/energy**, not
  on this proxy count.
* **Cumulative counters are polling-census proxies, not exact (task-94 addendum, commit `7a5ea90`).**
  `produced_energy_cumulative` is the energy of *newly observed* pellets since the previous poll; pellets
  created and removed between polls are missed, so it is a **lower bound** (Ring's 300-tick pass reported
  ~485 k E "produced" against a zone production rate of ~487 k E/s — clearly census-limited, not exact).
  `removed_energy_cumulative` is the **last-known energy of pellets that disappeared** between polls; it
  mixes whole-pellet consumption, out-of-bounds culling, zone recycling and decay, so it must **not** be
  relabelled as "culled" alone (`culled_energy_cumulative = removed` in the current addendum is a
  placeholder, not a measured term). Consequently `consumed_energy_estimate_cumulative =
  initial + produced − current − removed` is an **unaccounted partial-removal estimate** (it excludes
  energy in pellets that disappeared, i.e. whole swallows), not total consumption — the lol 600-tick pass
  shows it ~24 % below the stomach-intake gross for that reason. Use it as a cross-check only.
  `recycled_energy_cumulative`/`decayed_energy_cumulative` are explicitly `null` with `missing_terms`
  (correct); the exact Harmony accumulator on `SpawnAPellet`/`RecyclePellets`/`MatterDecay` is the
  deferred next step for a gateable game-side conservation identity.
* Mouth counters are cumulative and exposed here (`bibites_bitten`, `damage_dealt`, plus
  `total_murders`/`murdered_area` in the addendum).

**Optional exact upgrade `[P]`:** the game raises `BibiteStomach.onMatterEnter(material, amount)`
(`BibiteStomach.cs:16`, invoked `:291`) and `BibiteMouth.OnAttack(GameObject, damage)`
(`BibiteMouth.cs:31`, invoked `:765`) / `OnEnterMouth` (`:25`, invoked `:466`). A plugin listener could
count exact per-event intake and bite attempts and expose a sequence-numbered
`/d/feeding/events?since=…`. Note `BibiteMouth.OnSwallow` is declared (`:28`) but **not invoked** in the
decompiled build — do not rely on it; derive swallows from `onMatterEnter` + pellet disappearance. This
upgrade is not required for the first validation round if consumed-amount proxies + cumulative
production counters are added.

### 4.4 Zone/pellet accounting `[C]/[P]`

Implemented in the plugin snapshot: per-zone `pellet_biomass`, `max_biomass`, `free_biomass`,
`pellet_production` rate, `pellet_progress`, `pellets_n`, `plant_amount`, `meat_amount`; global `totals`.
These make stock and **production-rate** comparisons possible.

Task-94 addendum (`7a5ea90`) added polling-census cumulative counters, with these exact semantics:

* `produced_energy_cumulative` — energy of pellets **newly observed** since the previous poll. Lower bound:
  pellets created and removed between polls are missed (a cap-limited Ring pass reported only ~485 k E over
  300 ticks against a configured production rate of ~487 k E/s).
* `removed_energy_cumulative` — last-known energy of pellets that **disappeared** between polls. Cause is
  **mixed** (whole-pellet consumption, out-of-bounds cull, zone recycle, decay); it is not a cull counter.
  The current `culled_energy_cumulative = removed` is a placeholder, not a measured term.
* `consumed_energy_estimate_cumulative = initial + produced − current − removed` — an **unaccounted
  partial-removal estimate** (it excludes energy in pellets that disappeared, i.e. whole swallows), so it
  is a cross-check, not a total-consumption gate (the lol 600-tick pass read ~24 % below stomach gross).
* `recycled_energy_cumulative` / `decayed_energy_cumulative` — explicitly `null` with `missing_terms`
  (`["recycled_energy","decayed_energy"]`), `evidence: "polling_census_proxy"`. Correct honesty; the exact
  Harmony accumulator on `SpawnAPellet`/`RecyclePellets`/`MatterDecay` is the deferred next step.
* Per-bibite `total_murders`/`murdered_area` are now in `bibites[]`.

**Exact Harmony/caller-path counters (task-98, `c2665d1`) — game conservation now gateable.** The
inlined-`MatterPellet.RemoveAmount` problem was fixed with caller-path hooks:
`BibiteMouth.MaybeAttack` prefix/postfix (stomach Δ × material density = consumed),
`MatterDecayProcessor.FixedUpdate` (pellet-list energy before/after = decayed), `Zone.RecyclePellets`
(zone pellet energy before/after = recycled), and `Zone.SpawnAPellet` (produced, unchanged); the plugin
removed after capture. Fresh canonical Default captures (hidden, simTPS 30, one pass each):
- 600 t: initial 94,401.81 + produced 1,773.06 − current 95,810.62 − consumed 364.25 →
  **residual −3.8e-06 E (relative −4.0e-11)** — numerically exact.
- 3000 t: initial 94,299.99 + produced 9,633.43 − current 100,303.82 − consumed 3,373.53 →
  **residual 256.07 E (0.2464 %)**; `decayed`/`recycled` are 0 on this grass-only Default, so the
  remaining term is an unhooked cull/decay/census boundary — still ≤1 % of throughput and far better than
  the prior 3,658 E. Optional follow-up: capture nonzero decayed/culled on a meat/decay world to close it.
- Feeding rates moved between passes (scenario-start nondeterminism): game 600 t **0.4608 u²/live-s /
  0.1533 bites/live-s** (prior pass 0.2952/0.0956), 3000 t **0.7384 / 0.2300**; Rust 0.1169/0.0387 and
  0.6210/0.1759.

**Decision:** conservation is **gated on both sides** (`residual_rel ≤ 0.01`, absolute floor 1.0 E,
`missing_terms`/documented 0.2464 % note visible). `compare.py` implements per-side evidence
(`f848bfc`+`7764d6f`): game uses `harmony_exact` (600 t exact, 3000 t 0.2464 %), Rust uses
`evidence:"counters"` with the task-99 conservation object (600 t 2.0885e-4 E / rel 2.1758e-9; 3000 t
1.3947e-4 E / rel 1.3422e-9). Both PASS, `gated_sides=[left,right]`.

Remaining gaps (optional, not blockers for Default):

* cull/decay/census terms on worlds where they actually fire (grass-only Default exercises neither), to
  close the 3000 t 0.246 % residual to near-exact;
* per-material split of created/removed (per-zone material totals only).

These are the game-side counterparts of world-dev's `ZoneUpdateReport` extension. **Status (task-98,
`c2665d1`):** exact produced/consumed/recycled/decayed exist, so the game-side conservation identity is
**gateable** at the 1 % threshold (Default-600 is numerically exact; Default-3000 is 0.246 % due to the
unhooked cull/decay boundary). The game intake proxy remains the comparison source for *feeding rates*.

### 4.5 Infeasible / infer-only today

| Metric | Status | Workaround |
|---|---|---|
| Exact plant/meat bites and consumed mass per event | Event identity is not exposed; plugin polling gives a stomach-delta proxy. | Use consumed amount/energy for gates; caller-path stomach Δ (`c2665d1`) gives exact interval consumption without event identity; `onMatterEnter` tap remains deferred. |
| Cumulative zone production/recycle energy | **Exact** (`c2665d1`): produced via `SpawnAPellet`, recycled via `Zone.RecyclePellets` before/after, decayed via `MatterDecayProcessor.FixedUpdate`, consumed via `BibiteMouth.MaybeAttack`; all 0 on grass-only Default except consumed (364.25 E/600 t). | Gate at 1 % residual; keep `missing_terms` visible (decayed/recycled unhooked on Default; culled still unmeasured). |
| Decay/cull removed amount | Decay exact on meat-decay worlds (0 on Default); **cull still unhooked** (likely the 3000 t 0.246 % residual). | Document; optional meat/decay-world capture to close the residual. |
| Held/grab count | **Available** via plugin (`n_held`, `held_pellets`); not in save JSON (grab links are scene `connections`). | Use plugin; `null` for plugin-less captures. |
| Pellet-per-entity identity | Missing. | Not needed for aggregate identity. |
| Per-tick feeding counters | Missing (polling cadence only; caller-path consumed is per-interval). | Derive interval rates in `capture.py`/`feeding_ref.py`; exact event tap deferred. |

### 4.6 Recommended game-side tiers

1. **Tier A — plugin polling (implemented, task-94):** `/d/feeding` + `--feeding` + `feeding_ref.py`.
   Gives per-bibite stomach/mouth/held/energy state and per-zone stock/production rate at ~0.05 s.
   Use for short 1× runs (300–1000 ticks); record polling taint.
2. **Tier B — polling + polling-census cumulative counters (implemented addendum `7a5ea90`):** the
   snapshot now carries `produced_energy_cumulative`, `removed_energy_cumulative`, per-zone
   `culled/recycled/decayed` placeholders (`recycled`/`decayed` null with `missing_terms`), per-bibite
   `total_murders`/`murdered_area`, and `counters_evidence: "polling_census_proxy"`. This gives a
   cross-check and makes the terms explicit, but **not yet a gateable conservation identity** (cause
   attribution for disappeared pellets and the exact created/recycled/decayed accumulators are still
   needed, §4.4). The exact `onMatterEnter`/`OnAttack` event tap is **deferred** (optional future upgrade,
   §4.3) — do not block task-94 on it.
3. **Tier C — save-JSON entity fields (cheap fallback):** extend `capture.py`'s `entity_record` with
   `body.mouth`/`body.stomach`/`energy/health` so `--no-plugin` captures still carry per-snapshot mouth
   counters and stomach fill. Secondary once Tier A/B exist.
4. **Tier D — upstream BibitesAPI sampler group (optional):** per-tick feeding group. Best long-term,
   but touches the external project; only with Lead/parity-dev agreement.

---

## 5. Rust-side mapping

### 5.1 Code map today

| Area | Where | Notes |
|---|---|---|
| Mouth actions | `tick_mouth` (`body.rs:3906-4011`), `tick_mouth_pellet` (`body.rs:4013-4126`) | One site per action; add counters here. |
| Mouth state | `MouthState` (`organs.rs:1448-1481`), `record_first_bite` (`:1538-1541`), `bite_ready`/`consume_bite_period` (`:1516-1540`) | Runtime-only; load restores the game-serialised subset (`body.rs:2346-2351`). |
| Target snapshots | `TargetSnapshot` (`body.rs:3069`), `snapshot_target` (`:3181`) | Carries material/amount/radius/pos/tag. |
| Trigger detection | `step_physics` trigger→mouth (`body.rs:4430-4446`) | Sanity check only; not a feeding counter. |
| Swallow resolution | `resolve_bite` (`body.rs:3503`), `resolve_swallow` (`:3548`) | Count bibite swallows/consumed/kills here. |
| Stomach/digestion | `Stomach` (`organs.rs:237`), `add_matter` (`:389`), `update` (`:457-547`), `DigestionTick` (`:215-233`) | `energy_gain`, `total_content`, `finished_slots`. |
| Energy | `BodyState::use_energy` (`body.rs:801`, `EnergySpend` `:506`), `gain_energy` (`:829`), `total_energy` (`:753`) | Add cumulative counters, no behaviour change. |
| Events | `BibiteEvent` (`body.rs` ~1570-1600): `Bitten`, `Swallowed{by,pellet}`, `EggLaid`, `Death` | Extend optional payloads: `Swallowed` material/amount; `Death` meat energy; additive only. |
| Stats | `SimStats::sample` (`sim.rs:720-905`), `PelletStats` (`:496`), `BehaviorStats` (`:583-640`), `StatsSample` (`:690-722`), `reset_since_sample` (`:907`) | Add behavior fields + Δ counters; reset Δ. |
| Capture | `capture.rs` Behavior mapping (~1640-1690), `EventRecord` (`:536`), `push_event` (`:1071`) | Optional feeding fields/groups; `kind` extensible. |
| Zones | `ZoneData` (`zone.rs:485-520`), `ZoneUpdateReport` (`:767`), `update_production` (`:1139`), `spawn_ap_pellet` (`:1190`), `recycle_pellets` (`:1247`), `step` (`:1018`), `initial_seeding` (`:1056`) | Add produced/recycled/seeded energy. |
| Pellets | `spawn/remove/change_amount/remove_amount/rip_chunk/process_decay` (`pellet.rs:962,1030,1054,1107,1186,1331`), `counts` (`:688`) | Store-level counters at choke points; decay returns count only. |
| World | `step_zones` (`world.rs:3420`), `step_decay` (~3380), `census_and_cull` (`:3396`), `spawn_pellet`/`despawn_pellet` (~2040/2060), `tick` phases (`:2640-2690`) | Aggregate world/zone counters per tick. |
| Spawn (context) | `random_spawn_position` (`world.rs:2407-2413`), `spawner_iteration` (`:2478-2579`) | Task-88 finding must land before Ring validation is meaningful. |

### 5.2 Instrumentation (implemented in task-90, `eb5294d`)

The field set below is now live as runtime-only `BibiteRecord.feeding: FeedingCounters`
(`body.rs:1827-1902`, never serialised; `state_fingerprint` unchanged) and aggregated into
`BehaviorStats` rates / `FeedingInterval` (`sim.rs:601-666`, capture feeding stream in `capture.rs`).
Optional diagnostics (`*_bite_attempts`, `bibite_bite_drain_ticks`, health-fallback split) remain
unimplemented and are not required for the gates.

```rust
// proposed (names map 1:1 to §2.1); u64 counts, f64 amounts/energy
plant_bite_events, meat_bite_events, bibite_bite_events,
plant_swallow_events, meat_swallow_events, bibite_swallow_events,
plant_consumed_u2, meat_consumed_u2, bibite_consumed_u2,
plant_ripped_u2, meat_ripped_u2,
bibite_damage_dealt, bibite_kills,
stomach_digested_u2, stomach_energy_gain_e,
energy_gained_e, energy_spent_e, live_seconds
```

Instrumentation points:

1. `tick_mouth_pellet` (`body.rs`):
   * whole-swallow branch (`body.rs:4044-4072`): `*_swallow_events += 1`, `*_consumed_u2 += target.amount`
     (no bite event);
   * `rip_chunk` success (`:4096-4110`): `*_bite_events += 1`, `*_ripped_u2 += result.removed_amount`;
   * direct removal (`:4112-4125`): `*_bite_events += 1`, `*_consumed_u2 += removed`.
   * Select plant/meat from `target.material`; leave thresholds untouched.
2. `tick_mouth` bibite branch (`body.rs:3906-4011`): `BibiteSwallow::Swallow` → `bibite_swallow_events`,
   `bibite_consumed_u2` (victim meat + stomach amount, as in `resolve_swallow`); `record_first_bite()` →
   `bibite_bite_events`; `PendingBite` push → `bibite_damage_dealt`; victim death in `resolve_bite` →
   `bibite_kills`.
3. `tick_organs` digestion: after `stomach.update(...)` → `stomach_energy_gain_e += digest.energy_gain`;
   `stomach_digested_u2 += (content_before − digest.total_content)` or add a `dissolved` field to
   `DigestionTick`.
4. `BodyState::gain_energy`/`use_energy`: `energy_gained_e += accepted`; `energy_spent_e += spend.energy`
   (+ optional health-fallback equivalent).
5. Living body tick: `live_seconds += dt`.
6. `SimStats::sample`: sum per-bibite Δ counters into `behavior` rates (§6.1) and `manifest.counters`
   totals; compute `live_bibite_seconds` for the interval.

**Determinism:** counters are integer/`f64` sums; keep them out of f32 state, save bytes,
`to_contents`, `state_fingerprint`. Add a test asserting the fingerprint/save bytes are unchanged with
counters active.

### 5.3 Stats semantics

* `reset_since_sample` (`sim.rs:907`) also clears the feeding Δ accumulators and `live_bibite_seconds`.
* Emit **rates** in `behavior` (per live bibite-second, §6.1) and **cumulative totals** in the manifest
  (or a dedicated `feeding_stats` group if a consumer needs per-sample totals; do not mix monotonic and
  Δ values under one name).
* Keep `bite_rate_mean` with its legacy lifetime-bibite-bite semantic; add a manifest note that
  `plant_*`/`meat_*` rates use `live_bibite_seconds`.
* **The derived rates belong on the tick records, not only in `feeding_ref.py`'s summary.** `compare.py`
  gates per-tick curves, so `capture.py` must attach the same `behavior.*_per_s` fields to the tick record
  nearest each feeding snapshot (game side, `evidence: "polling_proxy"`); the detailed
  `feeding.jsonl`/`feeding_summary.json` stays the analysis artifact. The manifest can carry provenance,
  but it cannot substitute for tick values.

### 5.4 Capture/event schema

* Add the §6.1 `behavior` fields and §6.4 manifest blocks to `capture.rs`.
* Optionally extend `BibiteEvent` payloads (`Swallowed{material, amount_u2, energy}`, `Death{meat_energy}`)
  and a `--feeding-events` stream (kind `bite`/`swallow`; off by default to keep files small).
* Land the `docs/PARITY.md` §6 update together with code so `compare.py`/`parity_common.py` agree.

### 5.5 Tests (task-90 acceptance)

* **Plant feeding:** bibite on a placed plant pellet → `plant_bite_events > 0`, `plant_consumed_u2 > 0`,
  `stomach_digested_u2 > 0`, `stomach_energy_gain > 0`, and pellet amount drops by the same amount.
* **Small-pellet swallow:** `*_swallow_events == 1`, consumed == pellet amount, bite counter 0.
* **Bibite bite:** victim in mouth → `bibite_bite_events == 1`, `bibite_damage_dealt > 0`; kill run →
  `bibite_kills == 1`.
* **Zero-contact control:** all counters 0; conservation residual 0.
* **Conservation unit test:** synthetic zone/store sequence (spawn, consume, decay, cull known amounts)
  → residual within §3.5 exact tolerance.
* **Stats test:** capture `behavior` rates equal totals / live_bibite_seconds; capture writer matches
  `SimStats`.
* **Regression:** Ring default parity exit 0; save bytes/`state_fingerprint` unchanged.

---

## 6. Recommended schema (names, units, types)

Two layers must not be confused:

* **Derived engine/game rates** in `behavior` — compared by `compare.py`, gated by §7.
* **Raw parity evidence** in the optional `feeding` group — the plugin snapshot stream (game) and an
  optional engine snapshot; **never gated field-by-field**, only summarised by `feeding_ref.py`.

### 6.1 `behavior` additions (per live bibite-second, `f64`)

```json
"behavior": {
  "bite_rate_mean": 0.0129,                 // legacy: bibite first-bites / lifetime (game semantic)
  "plant_bites_per_s": 0.42,
  "meat_bites_per_s": 0.0,
  "bibite_bites_per_s": 0.013,
  "bibite_damage_per_s": 1.2,
  "plant_consumed_u2_per_s": 1.8,
  "meat_consumed_u2_per_s": 0.0,
  "plant_consumed_e_per_s": 9.0,
  "meat_consumed_e_per_s": 0.0,
  "stomach_energy_gain_per_s": 4.1
}
```

Rust emits these from task-90 counters. Game side: parity-dev can derive the same fields in
`capture.py`/`feeding_ref.py` from consecutive `feeding` snapshots (stomach-delta proxy) so both captures
carry comparable names; mark proxy values in the manifest (`feeding.evidence: "polling_proxy"`) instead of
pretending they are event-exact.

All fields optional/additive; `compare.py` ignores unknown fields and gates only known names.

### 6.2 Optional `feeding` group (raw parity evidence, `rustibites.parity.feeding.v1`)

Game capture (already implemented, abbreviated):

```json
"feeding": {
  "schema": "rustibites.parity.feeding.v1",
  "t": 469983.85,
  "tick": 123,
  "bibites": [{
    "id": 1234105466, "energy": 6705.5, "health": 1518.4, "d2_size": 7.47,
    "pos": {"x": 67.7, "y": -98.0},
    "stomach_total": 210.4,
    "stomach_materials": {"plant": 204.0, "meat": 6.4},
    "held_pellets": [], "n_held": 0,
    "bite_progress": 2.1, "bite_period": 2.14,
    "bibites_bitten": 12, "damage_dealt": 34.2
  }],
  "zones": [{"pellet_biomass": 609375.0, "max_biomass": 609375.0, "free_biomass": 0.0,
             "pellet_production": 121875.0, "pellet_progress": 0.0, "pellets_n": 1130,
             "plant_amount": 113000.0, "meat_amount": 0.0}],
  "totals": {"pellets_n": 4520, "plant_amount": 487000.0, "meat_amount": 0.0},
  "material_properties": {"plant": {"energy_density": 5.0, "mass_density": 0.015,
                                    "amount_at_scale1": 20.0}}
}
```

Rust captures may omit this group entirely, or emit an engine snapshot under the same field names if
needed for debugging; comparisons must dispatch on `manifest.feeding.available/schema`, not assume
presence.

### 6.3 Optional events (exact upgrade, `--feeding-events`)

Keep `rustibites.parity.event.v1`; add optional kinds (only once a game-side exact source exists):

```json
{"schema":"rustibites.parity.event.v1","tick":123,"t":"4.1","kind":"bite",
 "by":1234105466,"target_kind":"pellet","material":"Plant","amount_u2":6.5,"energy":32.5}
{"schema":"rustibites.parity.event.v1","tick":124,"kind":"swallow","by":1234105466,
 "target_kind":"pellet","material":"Plant","amount_u2":2.1,"energy":10.5}
{"schema":"rustibites.parity.event.v1","tick":130,"kind":"bite","by":123,"target_kind":"bibite","damage":3.2}
```

Never synthesise these on the game side; absent means "not measured".

### 6.4 Manifest additions

`capture.py` already writes `feeding {requested, available, schema, file, samples, poll_period_s, note}`;
`evidence`, `exact_events`, `counters` and `conservation` are proposed additions `[P]`.

```json
"feeding": {"requested": true, "available": true, "schema": "rustibites.parity.feeding.v1",
            "file": {...}, "samples": 6000, "poll_period_s": 0.05,
            "evidence": "polling_proxy", "exact_events": false},
"counters": {"plant_consumed_u2": 12345.0, "meat_consumed_u2": 678.0, "plant_bites": 2011,
             "plant_swallowed": 3, "bibite_bites": 17, "bibite_kills": 2,
             "live_bibite_seconds": 180000.0},
"conservation": {"stock_plant_u2_start": 487000.0, "stock_plant_u2_end": 486800.0,
                 "produced_u2": 1200.0, "consumed_u2": 1230.0, "decayed_u2": 0.0,
                 "culled_u2": 0.0, "recycled_u2": 0.0,
                 "residual_u2": -30.0, "residual_rel": 0.0123,
                 "terms_available": ["stock", "consumed"], "terms_missing": ["produced", "decayed"]}
```

`evidence` must be one of `plugin_events`, `polling_proxy`, `polling_census_proxy`, `save_snapshot`,
`inferred`, `none`; `terms_available/missing` make it impossible to silently treat an inferred number as
ground truth. The current task-94 addendum sets `feeding.evidence: "polling_proxy"`, the
`zones[].counters_evidence` / top-level `counters.evidence: "polling_census_proxy"`, and
`missing_terms: ["recycled_energy","decayed_energy"]`; keep `culled_energy_cumulative` visibly
placeholder (mixed `removed`) until cause attribution exists. The `production` block in an engine capture
should use exact values (`evidence` absent) rather than these proxies.

### 6.5 Optional entity snapshot additions (Tier C fallback)

```json
{"kind":"bibite","id":1234105466,
 "stomach_u2":210.4, "stomach":[{"material":"Plant","amount":204.0}],
 "mouth":{"bibites_bitten":12,"total_damage_dealt":34.2,"total_murders":1,
          "murdered_area":0.4,"bite_progress":2.1},
 "n_held":null, "energy":6705.5,"health":1518.4}
```

`n_held` is `null` when unavailable (save JSON cannot expose grab links); never infer it.

---

## 7. Comparison protocol and tolerances

Approved defaults (Lead, 2026-10-30): same `behavior.*` names both sides; 10 % default / 25 % loose on
consumed rates and energy; `bite_rate_mean` explicitly non-gate; absolute floors for near-zero values.
Game conservation is now gated (task-98 `c2665d1`, gate flipped `f848bfc`): `residual_rel ≤ 0.01`,
absolute energy floor 1.0 E, requires exact evidence on the gated side; the 0.246 % Default-3000
unaccounted cull/decay term is documented and `missing_terms` stays visible.

**Implemented (task-98, `77f7613`; self-tests pass):** `compare.py` derives per-live-bibite-second feeding
rates from the companion `feeding` stream (game intake proxy vs Rust counters), gates plant/meat bites +
consumed u²/E + stomach energy gain at 10 % default / 25 % loose with near-zero absolute floors, ignores
unknown fields, excludes `bite_rate_mean`, and adds the gated `conservation` section with
`evidence`/`missing_terms`/`policy{relative_tolerance, absolute_energy_floor, documented_gap}`.

Canonical Default results (pre-fix task-93 diagnostics; **feeding gap now closed**, see §8.2 closure table):
- **`compare_600.json`:** pre-fix overall fail (`ecology_curves`, `feeding_rates` — the then-open task-93
  gap: plant bites 0.1533 vs 0.0387 /live-s, rel 74.7 %; consumed 0.4608 vs 0.1169 u²/live-s, rel 74.6 %;
  stomach energy gain 2.304 vs 0.0169 E/live-s); **conservation PASS, gate=true** — game residual
  −3.8147e-06 E (rel −3.966e-11, passed via absolute floor), `missing_terms=[recycled_energy,
  decayed_energy]` shown.
- **`compare_3000.json`:** overall fail (same two unrelated failures); **conservation PASS, gate=true** —
  game residual 256.0699 E (rel 0.0024638 = 0.2464 %), under the 1 % tolerance, documented as the
  unhooked cull/decay term; `missing_terms` still shown.
- **Rust side also gated and PASSES (`7764d6f`):** refreshed canonical `rust_600`/`rust_3000` captures
  carry the task-99 conservation object; compare accepts `evidence:"counters"` for the Rust side (game
  stays `harmony_exact`). Residuals: 600 t **2.0885e-4 E / rel 2.1758e-9**, 3000 t
  **1.3947e-4 E / rel 1.3422e-9**; `gated_sides=[left,right]`, both `status=pass`.

The table below is the agreed gate policy; `compare.py` implements it.

| Gate | `--profile exact` | `--profile default` | `--profile loose` |
|---|---|---|---|
| Engine feeding counters (same-seed Rust-vs-Rust) | bit-identical | bit-identical | bit-identical |
| `behavior.plant_consumed_e_per_s`, `meat_consumed_e_per_s`, `stomach_energy_gain_per_s` (game-vs-Rust) | n/a (physics/timing differ) | MAE ≤ 10 % rel (≥ absolute floors) | ≤ 25 % rel |
| `behavior.plant_bites_per_s`, `meat_bites_per_s`, `bibite_bites_per_s` | n/a | 10 % rel where game value > floor; Ring 0 → compare absolute totals only | 25 % rel |
| `bite_rate_mean` legacy | as before | as before; **not a feeding gate** | idem |
| `residual_rel` (Rust-only, per interval) | ≤ 1e-4 | ≤ 1 % of throughput, absolute u²/E floors | ≤ 5 % |
| Game-vs-Rust conservation | exact counters now exist both sides (`c2665d1`) | **gate** `residual_rel ≤ 0.01` + absolute floors; report `evidence`/`missing_terms` and the documented 0.2464 % Default-3000 term | informational (report only) |
| Entity stomach/mouth snapshot deltas | exact (Rust-vs-Rust) | 10 % rel on consumed proxy over matching snapshot intervals | 25 % rel |
| Event kinds/counts | exact | birth/death as today; feeding events informational | informational |

Rules:

* Absolute floors: 1e-3 u², 0.01 E, 1 event. Near-zero counters compare as "both below floor", not as a
  relative ratio.
* Game **proxy** intake values are not gate-eligible as exact counters; compare rates and consumed
  amounts with the default/loose bands and keep the `evidence` tag visible in the report.
* **Fresh-scenario rate sensitivity (task-93):** a single Default pass has ±30–50 % game-to-game spread
  and a 2–4× game-vs-Rust gap at 600 t because of startup/foraging/spawn placement (first intake tick 351
  vs 508; per-bite amount identical at 3.01 vs 3.02 u²). Do **not** treat the single-pass 600 t
  `feeding_rates` fail as a body-formula defect: use multi-pass medians/variance (or `--profile loose`,
  25 %) for the fresh-start gate, and at 3000 t use a ~20 % rate band (current 0.6210 vs 0.7384 =
  15.9 %, inside). The default 10 % gate remains appropriate for evolved/dense worlds, not for a 600 t
  fresh start.
* Statistical 50× captures (§8.5) never use exact gates; compare 1 Hz envelopes and residual bands.
* Add `compare.py --self-test` cases: known feeding/conservation residuals pass/fail as expected;
  unknown feeding fields ignored (forward compatibility).

---

## 8. Validation plan

### 8.1 Prerequisites

0. **Canonical world:** `testdata/real/Default.zip` (`docs/TESTING_POLICY.md`, 2026-10-30). Validate the
   Default baseline (§8.2) first; Ring/lol runs are historical-fixture regressions with documented reasons.
1. world-dev spawn fix landed (task-85, green): scenario/virgin spawns use `Zone::spawn_location()`
   (honouring `SpawnRow.spawn_zone`); tests assert `zone.contains(pos)`.
2. body-dev task-90 counters + stats **landed** (`eb5294d`; §5.5 tests green); world-dev **task-99**
   conservation accounting **landed and Default-validated** (commit `02fb13a`):
   `ZoneUpdateReport::{produced,recycled}_{plant,meat}_energy`, `SeedingReport` seeded energy,
   `DecayReport`, cull energy, `World::conservation_totals/snapshot`, `StatsSample.conservation`.
   Together they make the Rust conservation identity gateable (§3.5, residual ≈2e-9 relative).
3. parity-dev capture schema updated with derived `behavior` rates + conservation terms; game-side
   `/d/feeding` available (Tier A/B polling counters) and the **canonical Default baseline committed**
   (task-98, `94694e1`: `testdata/parity/default/`, `tools/parity/default_baseline.py`). **Caller-path
   exact hooks landed (`c2665d1`):** produced/consumed/recycled/decayed are exact (600 t residual −3.8e-6 E,
   rel −4.0e-11; 3000 t 256.07 E, rel 0.2464 %); game conservation is gateable at the 1 % threshold.
   `missing_terms` (decayed/recycled unhooked on grass-only Default, cull unmeasured) stays visible.
4. Game runs hidden-window only; plugin removed afterwards; no writes to game install/AppData/
   BibitesAPI; follow `docs/PARITY.md` §7 cleanup.

### 8.2 Default scenario (canonical baseline, `testdata/real/Default.zip`)

Per `docs/TESTING_POLICY.md` (2026-10-30), all feeding validation starts from the game's default
scenario. The canonical game+Rust baseline was captured in task-98 (`94694e1`; refreshed with caller-path
hooks `c2665d1`), artifacts under `testdata/parity/default/` (`game_600`, `game_3000`, `rust_600`,
`rust_3000`, `analysis_600.json`, `analysis_3000.json`; driver `tools/parity/default_baseline.py`).
Game feed rates move between fresh captures (scenario-start spawner nondeterminism). **Status: feeding gap
CLOSED** — official post-fix recapture (body `55cda1f` part transforms + task-106 contact persistence,
commit `2fb2ad4`; identical-base game-written fixture, `official_analysis_{600,3000}.json`):

| metric (seed 12345) | game 600 t | Rust 600 t | game 3000 t | Rust 3000 t |
|---|---|---|---|---|
| contacters (of 10) | 8 | 8 | 8 | 8 |
| fed bibites | 8 | 8 | 8 | 8 |
| intake events | 68 | **71** | 249 | **263** |
| consumed u² | 216.36 | **230.24** | 822.95 | **868.33** |
| u² per bite | 3.182 | **3.243** | 3.305 | **3.302** |
| median idle gap (ticks) | 7 | 7 | 9 | 9 |
| first intake tick | 6 | 4 | 4 | 4 |
| Rust over-consumption | — | **+6.4 %** | — | **+5.5 %** |

The gap is closed within the single-pass/default 10 % band: identical contacters/fed counts, intake event
and bite-size parity, and only +5–6 % residual over-consumption (trajectory/phase noise, not an acquisition
failure). This supersedes the pre-fix task-93 rate interpretation below, which is retained as the
diagnostic rationale. Shared artifacts: `testdata/parity/default/official_{game,rust}_{600,3000}`.

The pre-fix canonical baseline (task-98/99 diagnostics; superseded by the closure table above):

| metric (seed 12345) | game 600 t | Rust 600 t | game 3000 t | Rust 3000 t |
|---|---|---|---|---|
| population start→final (min/max) | 1→10 (1/10) | 1→10 (1/10) | 1→10 (1/10) | 1→10 (1/10) |
| energy mean start→final | 155.80→116.22 | 157.22→114.71 | 155.63→101.91 | 157.22→98.35 |
| plant_n start→final | 414→423 | 440→448 | 426→461 | 440→473 |
| plant biomass E start→final | 94,401.8→95,810.6 | 94,309.8→95,898.7 | 94,300.0→100,303.8 | 94,309.8→100,952.5 |
| first intake tick | 351 | 508 | 377 | 508 |
| plant bites / live-s | **0.1533** | 0.0387 | **0.2300** | 0.1759 |
| plant consumed u² / live-s | **0.4608** | 0.1169 | **0.7384** | 0.6210 |
| consumed amount / events (600 t) | 72.15 u² / 24 | 18.12 u² / 6 | — | — |
| per-bite consumed (600 t) | 3.01 u²/event | 3.02 u²/event | — | — |
| consumed gross E (interval) | **364.25** | 90.60 | **3,373.53** | 2,965.14 |
| conservation residual (exact) | **−3.8e-06 E (rel −4.0e-11)** | 2.09e-4 E (2.18e-9) | **256.07 E (0.2464 %)** | 1.39e-4 E (1.34e-9) |

**Historical task-93 interpretation (body-dev wording, 2026-10-30; pre-fix — superseded by the closure
table above):** evolved-world feeding rates are scenario-start
sensitive. On Default 600 t single-pass rates differ 2–4× (game 0.4608 u²/live-s / 0.1533 bites, first
intake tick 351; Rust 0.1169/0.0387, first intake 508), while at 3000 t they converge to game
0.7384/0.2300 vs Rust 0.6210/0.1759 and the **energy envelope is within 4 %** (game →101.91 E vs Rust
→98.35; live_seconds 956 vs 955; same 9 births). **Per-bite consumed amount is identical at 600 t**
(game 72.15/24 = 3.01 u²/event; Rust 18.12/6 = 3.02), and at 3000 t Rust digestion conversion is **24.9 %
of gross** (739.83 E / 2,965.14 E; the 600 t ratio is low because most consumed matter is still in the
stomach at that horizon). Therefore: judge fresh-scenario feeding rates on **multi-pass medians/variance,
not a single-pass strict gate** (game pass-to-pass spread is ±30–50 %); treat the 600 t rate gap as
startup/foraging/scenario-placement variance, not a body-formula defect; at 3000 t use a **~20 % rate
band** on this Default fixture (current 0.6210 vs 0.7384 = 15.9 % below, inside the band).
`compare.py`'s default/loose profiles remain the machinery (10 %/25 %); run **`--profile loose`** or a
multi-pass median when gating fresh-scenario feeding rates. Conservation is green on both sides (Rust
task-99 compare residual 2.18e-9 / 1.34e-9 relative; game exact −4.0e-11 at 600 t, 0.2464 % at 3000 t with
decay/recycle hooks incomplete). lol/Ring numbers are historical only, never a baseline.

Rust feeding counters (`behavior`/capture `feeding`): 600 t = 6 plant bites, 18.12 u² consumed, 1.96 u²
digested, 2.61 E gained, 155.0 live-s; 3000 t = 168 bites, 593.03 u² consumed, 11 plant swallows,
542.25 u² digested, 739.83 E gained, 955.0 live-s.

Conservation (game exact counters, evidence `harmony_exact`, caller-path hooks `c2665d1`): produced
1,773.06 E / 9,633.43 E; consumed 364.25 E / 3,373.53 E; 600 t residual −3.8e-06 E (rel −4.0e-11),
3000 t residual 256.07 E (0.2464 %, unhooked cull/decay boundary). Rust's own conservation residual is
validated against task-99's exact terms (≈2e-9 relative). First divergence is counts at tick 24/25
(game 2 vs Rust 1) and an initial energy delta 1.59 E (spawner placement/seed), not a feeding-path
failure.

```powershell
# canonical Default baseline driver (game hidden-window + Rust + analysis; task-98)
python tools\parity\default_baseline.py --ticks 600
python tools\parity\default_baseline.py --ticks 3000
# artifacts: testdata\parity\default\{game,rust}_<ticks>, analysis_<ticks>.json

# equivalent manual game capture (plugin pacing + feeding polls)
python tools\parity\capture.py --scenario testdata\real\Default.zip --ticks 600 --speed 1 --seed 12345 `
    --sample-every 0.01 --snapshot-every 15 --events --feeding --feeding-period 0.05 `
    --out testdata\parity\default\game_600
python tools\parity\feeding_ref.py --capture testdata\parity\default\game_600

# canonical Rust run (compare in default_baseline.py; or compare.py manually)
target\eco-target\release\rustibites.exe run testdata\real\Default.zip --ticks 600 --seed 12345 `
    --capture testdata\parity\default\rust_600
```

Acceptance for the Default baseline:

* Population 1→10 is exact on both sides (spawner-driven); energy/plant curves are within the default
  profile tolerances (initial delta from spawner placement; see `docs/PARITY_ROUND2.md` §10). The current
  `compare_600.json` also fails `ecology_curves` on `pellets.bibite_biomass`; that is the task-93
  spawner/energy divergence, tracked separately from feeding counters.
* Feeding counters are nonzero on both sides (Rust `behavior`/capture; game intake proxy). Interpret the
  600 t rate gap per the task-93 wording above: single-pass game-vs-Rust differs 2–4× (first intake 351 vs
  508) but per-bite consumed is identical (3.01 vs 3.02 u²) and the 3000 t rates converge (0.6210 vs
  0.7384, 15.9 % lower, inside the ~20 % band) with the energy envelope within 4 % → startup/foraging/
  scenario-placement variance, not a body-formula defect. `compare_600.json` reports the expected
  `feeding_rates` failure with per-key relative errors ~74.6–74.7 % (bites/consumed) and ~99.3 %
  (stomach energy gain); gate fresh-start rates on multi-pass medians/variance (or `--profile loose`) and
  use a ~20 % band at 3000 t.
* **Both conservation identities are gated and PASS (`7764d6f`, `gated_sides=[left,right]`):** game
  600 t numerically exact (−3.8e-6 E) and 3000 t 0.2464 % (< 1 %); Rust 2.0885e-4 E (2.1758e-9) at 600 t
  and 1.3947e-4 E (1.3422e-9) at 3000 t from the refreshed task-99 captures. `terms_missing`
  (corpse→meat creation, red-death, decayed/recycled on grass-only Default) stays visible.
* Ring/lol sections below are historical-fixture regressions, not substitutes for this baseline. Do not
  substitute lol/Ring: policy requires Default unless a task documents the fixture reason.

### 8.3 Ring fresh scenario, post-spawn-fix (historical fixture regression)

Per policy this is **not** the canonical baseline; it is kept as a documented fixture regression for the
spawn-placement fix (task-85/88) and the ceiling behaviour of a 4-zone 97,500 × 2,500 `Rect` world.

```powershell
# game: 1x, plugin pacing + feeding polls (short deterministic reference)
python tools\parity\capture.py --scenario testdata\real\Ring.zip --ticks 300 --speed 1 --seed 12345 `
    --sample-every 0.01 --snapshot-every 15 --events --feeding --feeding-period 0.05 `
    --out testdata\parity\round5\ring_game_300
python tools\parity\feeding_ref.py --capture testdata\parity\round5\ring_game_300

# rust
target\eco-target\release\rustibites.exe run testdata\real\Ring.zip --ticks 300 --seed 12345 `
    --capture testdata\parity\round5\ring_rust_300

python tools\parity\compare.py testdata\parity\round5\ring_game_300 testdata\parity\round5\ring_rust_300 `
    --report human --profile default
```

Expected after the spawn fix (from the default baseline/geometry; task-85 test status):

* Spawned bibites are inside the four rect strips (not at y ≈ −56,448); the first 300 ticks should show
  **nonzero consumption** (Rust `plant_consumed_u2`, game intake proxy) and a nonzero conservation
  throughput.
* Stock stays within ~1 % of the ceiling (~2.437 M E total; `maxBiomass/zone =
  biomassDensity × area × globalBiomassDensity = 1.0 × 243.75e6 × 0.0025 = 609,375 E`, 4 zones);
  production ≈ consumption + decay/cull.
* 20-min envelopes (`§8.5`/`PARITY_ROUND2 §8`): game pop final 10 (min 1, max 15), energy 156→55
  (min 33), health 131→114, plant biomass 2.351→2.436 M (dip ≤ 4 %), births/deaths 54/45, corpses end 10.
  Do **not** gate on `bite_rate_mean` (0 both sides) or stock alone.
* Acceptance: spawn-position test green; consumed > 0 (Rust and, via proxy, game); Rust conservation
  residual ≤ 1 % throughput or absolute floors; energy/pop envelopes inside game ±25 % over 20 min; no
  extinction.

### 8.4 Evolved lol save, task-93 (historical fixture regression)

Per policy this is a **documented historical fixture** for the per-capita feeding/energy gap (task-93);
the canonical baseline remains Default.zip (§8.2).

```powershell
python tools\parity\capture.py --save testdata\real\saves\lol.zip --settings-file <lol-settings>.json `
    --ticks 1000 --speed 1 --seed 12345 --sample-every 0.01 --snapshot-every 15 --events `
    --feeding --feeding-period 0.02 --out testdata\parity\round5\lol_game_1000
python tools\parity\feeding_ref.py --capture testdata\parity\round5\lol_game_1000

target\eco-target\release\rustibites.exe run testdata\real\saves\lol.zip --ticks 1000 --seed 12345 `
    --capture testdata\parity\round5\lol_rust_1000
python tools\parity\compare.py testdata\parity\round5\lol_game_1000 testdata\parity\round5\lol_rust_1000 `
    --report human --profile default
```

§8 envelopes (20 sim-min, statistical; game vs current Rust; world = `lol.zip`): population 22→19
(min 16, max 38) vs 22→43; energy 1226→1534 (min 641) vs 1226→97 (min 97); health 480→545 vs 480→220;
plant biomass 184.5k→178.5k (dip 15.4 %) vs 181.1k→182.5k (dip 10.8 %); births 311 vs 310; deaths 314 vs
289; meat max 18,813 vs 26,189; `bite_rate_mean` game 0.0215 (max 0.112) vs Rust 0.0129 (max 0.092).

Target for task-93 after counters: sustained energy mean **> 600** (game min 641) and population stable
~20–25; per-capita plant consumption and bibite bite rate within 25 % of game; Rust conservation residual
≤ 1 %. Use the counters to localise the gap (bites vs bite amount vs conversion vs upkeep) instead of
tuning stock.

### 8.5 Long statistical protocol (50×, 18k ticks)

Follow `docs/ECOLOGY.md` / `PARITY_ROUND2.md` §8: game hidden-window `--speed 50`, one pass per world
(~145 ticks/s observed); Rust `run <world> --ticks 18000 --seed 12345 --capture ...`; analyse with
`tools/parity/ecoref_long.py` (1 Hz series + envelopes). Run this on **Default.zip** for the canonical
envelope; the Ring/lol 50× references are historical fixtures. With `--feeding`, run `feeding_ref.py` on
the game capture; compare feeding rates against Rust task-90 `behavior` rates. Taint rules: high-speed
captures are statistical, one pass; do not claim exactness.

### 8.6 Pitfalls

* **At-cap masking:** Ring/lol stock sits at/near the ceiling; production and consumption cancel. Never
  conclude "not eating" from flat stock. Task-88's `ring-eco` plant_n frozen at 4,518 was consistent with
  the spawn-placement bug, not broken mouths.
* **`bite_rate_mean` is not feeding:** bibite first-bites/lifetime; 0 on Ring on both sides; never a gate.
* **Game proxy limits:** polling sees only matter that stays in the stomach between polls; "bite counts"
  are positive-increment counts. Gate amounts, not proxy counts, and record `evidence`.
* **Sampling:** game entity snapshots are paused-boundary samples; `--feeding` polling adds wall-clock
  perturbation (taint) and is not tick-exact. Use `--feeding-period 0.05` (~3 ticks at 1×) for short
  runs; a period below the tick dt gains nothing.
* **Save/load:** corpse files in real saves are not founders (dense-world note in `docs/ECOLOGY.md`);
  counters are runtime-only and must not change save bytes/`state_fingerprint`.
* **Engine divergence:** D18 (unseeded game pellet RNG) and D19 (physics) prevent exact game-vs-Rust
  feeding equality; use tolerance bands and conservation.
* **Unit traps:** `E = u² × density` (plant 5, meat 10); never mix `plant_n` with energy, or
  `bite_rate` with per-second feeding rates. `age_s` is seconds; some game lifetime rates use hours —
  keep the legacy lifetime semantic only for `bite_rate_mean`.

---

## 9. Ownership, open questions, checklist

| Owner | Item |
|---|---|
| body-dev | task-90 counters/stats/events **landed (`eb5294d`)**: `FeedingCounters` + §6.1 behavior rates + capture feeding stream; **task-93/task-105/task-106 CLOSED** (official §8.2 closure: contacters/fed/intake parity, +5–6 % over-consumption); keep counter names/semantics aligned with task-99 so the conservation identity stays closed. |
| world-dev | **task-99 LANDED (commit `02fb13a`, 2026-10-30):** `ZoneUpdateReport::{produced,recycled}_{plant,meat}_energy`, `SeedingReport::{seeded_plant,seeded_meat}_energy`, `pellet::DecayReport` from `process_decay`/`step_decay`, cull energy, `World::conservation_totals()/conservation_snapshot()`, `ConservationTotals::{plant,meat}_energy_residual`, `StatsSample.conservation`; Default-validated (residual ≈2e-9 relative; tests `conservation_identity_*` green; Ring default compare exit 0; the 4.77e-6 vs 2.1e-4 E difference is the stock-summation path, same ~1e-9 relative). `terms_missing` = corpse→meat creation (body-owned) + red-death refused spawn. Task-85 spawn fix green; the `world.rs` "fresh Ring scenario" comments were generalised. Do not edit body.rs. |
| parity-dev | task-94/98 **complete:** `/d/feeding` + `--feeding` + polling counters + canonical Default baseline (`94694e1`) + compare feeding/conservation sections (`77f7613`) + caller-path exact hooks (`c2665d1`) + per-side conservation gate (`f848bfc`, `7764d6f`: game `harmony_exact` + Rust `counters`, both PASS at 600/3000 t) + refreshed Rust captures + Appendix-D merge + `tools/parity/README.md` Default walkthrough. Remaining optional: meat/decay-world capture to close the 0.2464 % game residual (decayed/recycled still unhooked on grass-only Default). |
| Lead | decisions recorded 2026-10-30: Tier B scope = polling + cumulative counters (event tap deferred); schema/tolerances approved; `docs/PARITY.md` owned by parity-dev, Appendix D is the suggested patch. |

**Open questions**

1. Does the game's `bite_rate_mean` denominator match Rust's `bibites_bitten/age` for all current
   versions? (Round-4 matched; confirm on the next capture.)
2. If the exact event tap is ever revived (currently deferred), can the plugin attach feeding listeners
   during the load pause so it catches the first unpaused tick? (Polling has no such risk.)
3. Is `stomach_energy_gain` sufficient as the digestion-efficiency proxy, or do we also need acid/
   effectiveness terms? (Recommend the former first; more only if task-93 finds a conversion mismatch.)
4. What is the acceptance band for `bite_rate_mean` on evolved worlds (game 0.0215 vs Rust 0.0129 today)?
   Recommend gating the new `bibite_bites_per_s` at 25 % and treating `bite_rate_mean` as informational.
   task-93 is closed; the official §8.2 recapture keeps `bite_rate_mean` non-gate.
5. Should the game proxy "bite count" ever be compared to Rust `plant_bite_events`? Recommend **no**:
   compare consumed amounts/energy and use event counts as diagnostics only.

**Definition of done**

* `docs/PARITY.md` §6 updated with §6.1 fields and the `feeding` group/evidence policy.
* body-dev counters pass §5.5 and are in the capture (`eb5294d`); Ring `plant_consumed_u2 > 0` post-fix;
  `state_fingerprint` unchanged.
* world-dev task-99 terms landed + Default-validated (residual ≈2e-9 relative; tests green); Rust
  conservation gateable (§3.5). Remaining `terms_missing`: corpse→meat creation (body-owned) + red-death.
* parity-dev `feeding_ref.py` reports `terms_available/missing`; compare `feeding`/`conservation` sections
  + self-test landed (`77f7613`); game exact consumed/decayed/culled hooks and full Appendix-D merge remain
  tracked, not silently assumed.
* Ring (post-fix) and lol results remain historical fixture regressions; the canonical Default baseline is
  reported in §8.2 and its acquisition/feeding gap is **closed** by the official post-fix recapture
  (`official_analysis_{600,3000}.json`; +5–6 % over-consumption, inside the default 10 % band).

---

## Appendix A — worked numbers (sanity checks)

**Ring geometry and stock**

* `simulation_size = 50,000` (bounds ±50,000). Rect zones: width `1.95 × 50,000 = 97,500`, height
  `0.05 × 50,000 = 2,500`; `radius = max(w,h)/2 = 48,750`; area `= 243.75e6 u²`; centres at
  `(±1,250, ±48,750)`.
* `max_biomass/zone = biomassDensity(1.0) × area × globalBiomassDensity(0.0025) = 609,375 E`;
  4 zones → **2,437,500 E** ceiling (matches captured ~2,435,782 E).
* `total_growth/zone = fertility(1.0) × area × pelletGrowth(0.0005) = 121,875 E/s`; at the ceiling
  `pellet_progress` clamps to 0 free biomass, so production ≈ consumption + losses — stock cannot reveal
  feeding.
* Avg pellet energy at cap ≈ 2,437,500 / 4,518 = **539 E = 107.8 u², r ≈ 5.86 u**. Starter mouth opening
  ≈ `0.842 × throat_width ≈ 3.77 u`; `r > opening/2`, so plant pellets are **bite-chunk** food, not
  whole-swallow food. Plant feeding appears in `plant_bite_events`/`plant_consumed_u2`, rarely in
  `plant_swallow_events`.
* Starter maximum bite throughput (pinned probe, task-88): `6.5 u²` per `2.14 s` ≈ `3.0 u²/s` ≈
  `15 E/s` gross; herbivore plant conversion `−0.25 + 1.0 × (0.5 − −0.25) = 0.5` → ≈ `7.6 E/s` net body
  energy while eating continuously. Ring starters mostly don't eat, so rates are much lower; don't gate
  Ring on a high rate.

**lol scale**

* `simulation_size = 2,000`, independent biomass density 1.0, pellet growth 0.005; observed stock
  ~184.5 k E with a 15.4 % game / 10.8 % Rust dip over 20 min — feeding is a real measurable term. Use
  lol for per-capita feeding gates.

**Conversion checks**

* `consumed_E = consumed_u2 × density` (plant 5, meat 10).
* `stomach_energy_gain ≈ consumed_E × conversion_efficiency × (1 − efficiency_malus)` (plant 0.5 for a
  pure herbivore; meat 0.85 for a pure carnivore).
* `residual_rel = |stock_end − stock_start − produced + consumed + decayed + culled + recycled| /
  max(1, produced + consumed)`.

## Appendix B — command examples

```powershell
# Canonical (Default.zip, policy baseline): Rust deterministic short run with counters + capture + save
target\eco-target\release\rustibites.exe run testdata\real\Default.zip --ticks 600 --seed 12345 --threads 1 `
    --capture testdata\parity\round5\default_rust_600 --save testdata\parity\round5\default_rust_final.zip --stats csv

# Canonical game baseline: plugin pacing + feeding polls (hidden window)
python tools\parity\capture.py --scenario testdata\real\Default.zip --ticks 600 --speed 1 --seed 12345 `
    --sample-every 0.01 --snapshot-every 15 --events --feeding --feeding-period 0.05 `
    --out testdata\parity\round5\default_game_600
python tools\parity\feeding_ref.py --capture testdata\parity\round5\default_game_600

# Historical-fixture regression examples (opt-in, documented reason; policy allows only explicit fixtures)
# target\eco-target\release\rustibites.exe run testdata\real\Ring.zip --ticks 300 --seed 12345 ...
# python tools\parity\capture.py --scenario testdata\real\Ring.zip ...

# self-tests after schema changes
python tools\parity\compare.py --self-test
python tools\parity\capture.py --self-test
```

## Appendix C — references

* `crates/bibites-core/src/body.rs` — `tick_mouth` 3906–4011, `tick_mouth_pellet` 4013–4126,
  `resolve_bite` 3503, `resolve_swallow` 3548, `BibiteEvent` ~1570, trigger→mouth 4430–4446,
  `EnergySpend` 506, `use_energy` 801, `gain_energy` 829, `total_energy` 753, `die_now` 4493–4536.
* `crates/bibites-core/src/organs.rs` — `MouthState` 1448–1481, `record_first_bite` 1538–1541,
  `bite_ready`/`consume_bite_period` 1516–1540, `try_swallow_pellet` 1738, `try_swallow_bibite` 1772,
  `Stomach` 237, `add_matter` 389, `update` 457–547, `DigestionTick` 215–233, `conversion_efficiency`,
  `affinity`.
* `crates/bibites-core/src/sim.rs` — `PelletStats` 496, `BehaviorStats` 583, `StatsSample` 690,
  `sample` 720–905, `reset_since_sample` 907.
* `crates/bibites-core/src/zone.rs` — `max_biomass` 358, `total_growth` 369, `ZoneData` 485–520,
  `ZoneUpdateReport` 767, `update_production` 1139–1188, `spawn_ap_pellet` 1190, `recycle_pellets` 1247,
  `initial_seeding` 1056.
* `crates/bibites-core/src/pellet.rs` — `counts` 688, `remove` 1030, `change_amount` 1054,
  `remove_amount` 1107, `remove_energy` 1128, `rip_chunk` 1186, `process_decay` 1331.
* `crates/bibites-core/src/world.rs` — `spawn_pellet` ~2040, `despawn_pellet` ~2060,
  `random_spawn_position` 2407–2413, `spawn_initial_population` 2424, `spawner_iteration` 2478–2579,
  `spawn_loaded_bibite` 3536, `step_decay` ~3380, `census_and_cull` 3396, `step_zones` 3420,
  `tick` 2640–2690.
* `crates/bibites-cli/src/capture.rs` — `EventRecord` 536, Behavior mapping ~1640–1690, `push_event` 1071.
* `docs/PARITY.md` §3–§6 (bridge routes, capture schema, digest rules); `docs/PARITY_ROUND2.md` §8
  (long-horizon envelopes); `docs/ECOLOGY.md` (50× protocol); `docs/spec/04-formats-settings.md` §4.3
  (save body sections).
* `tools/parity/plugin/Plugin.cs` — `/d/feeding` route + `FeedingSnapshot`; `tools/parity/capture.py`
  `--feeding`/`--feeding-period`, manifest `feeding` block; `tools/parity/feeding_ref.py` feeding summary
  schema `rustibites.parity.feeding_summary.v1`.
* Game: `Here/SimulationScripts.BibiteScripts/BibiteMouth.cs` (events 25/28/31/34, counters 111–117,
  attack 765, swallow 553–592), `BibiteStomach.cs` (events 16/19/22/25, save 349),
  `BibiteBody.cs` (save 951–975, events 147/150), `Here/SimulationScripts/Zone.cs`
  (`pelletProduction/pelletBiomass/pelletProgress/maxBiomass` 66–86, production 198–211, recycle 440,
  initial seeding 464–465).

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
> 1 event; `bite_rate_mean` informational) and `conservation` (now gateable with exact counters both
> sides: gate `residual_rel ≤ 0.01` + absolute floors, keep `missing_terms`/documented 0.2464 % note;
> `compare.py` gate flip pending).
> Unknown feeding fields are ignored for forward compatibility; self-test covers the new sections.

If parity-dev prefers, the whole block can instead be a shorter cross-reference ("feeding schema per
`docs/design/FEEDING_METRICS.md` §6") — the tables above are the integration-ready version.



## Optional meat/decay fixture result (2026-10-30)

Default material settings have plant/meat `decay=false`; injecting 250 meat pellets or killing the
starting bibite (1 corpse) did not produce nonzero `decayed`/`recycled` terms over 600/3000 t, and
enabling `materials.*.decay=true` in a saved settings block makes the game reject the load. Game exact
conservation closes for meat 600 t (−1.7e−6 E) but leaves residual −116.73 E (0.115%) at 3000 t and
+175.48 E (0.173%) with a corpse, all with decayed/recycled/culled = 0 — evidence for an unhooked
amount-based removal/corpse term, not decay/recycle. Rust `conservation` on game-written saves is
`terms_missing: true` with zero seeded terms/no initial stock, so its identity cannot be closed there.
Details/captures: `docs/PARITY_ROUND2.md` §10.6, `tools/parity/decay_ref.py`.

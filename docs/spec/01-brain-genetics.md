# 01 — Brain / NEAT genome / mutation / speciation

Status: **spec extracted from decompiled game source; implemented and kept current (2026-10-30)**
Owner: brain-dev. Write scope: `crates/bibites-core/src/{genome,brain,species}.rs` (implementation phase); this file is the contract.

Source of truth: `Here/` = decompiled C# of **The Bibites 0.6.3.1** (installed build; version confirmed in
`The Bibites_Data/globalgamemanagers`, and `Here/SettingScripts/ScenarioIndependentSettings.cs:170-228` matches the
real 0.6.3.1 save at `docs/spec/_tmp/recent/settings.bb8settings`). Every claim below is followed by a
`Here/<file>:<line>` reference. Where a behaviour is not visible in decompiled code (Unity engine internals, e.g.
`Vector2.SignedAngle`), the engine source is quoted and the residual uncertainty is in §12.

The auxiliary doc `bibites_brain_reference (4).md` was cross-checked and is **wrong in several places**
(see §13). Do not use it as authority.

Rust target: `bibites-core` (no I/O, no deps, f32, deterministic). File-format (de)serialization lives in
`bibites-io` (fmt-dev); this spec defines the required JSON shapes and migrations (§10) so io can implement them.

## Implementation status (2026-10-30)

**Status: implemented.** The formulas, tables and order below are the contract the code is tested against.
Code: `crates/bibites-core/src/{brain,genome,species}.rs`; JSON + version migrations in
`crates/bibites-io/src/{lib,model,migrate}.rs`.

| Section | Status | Code | Representative tests |
|---|---|---|---|
| §1 data model / enums | implemented | `brain.rs` (`NodeType`, `Input`, `Output`, `NodeRecord`/`SynapseRecord`), `N_INPUTS=33`, `N_OUTPUTS=15` | `enums_match_game_order`, `records_roundtrip_preserves_state` |
| §2 activations (+ f64 precision) | implemented | `brain.rs::activation`/`soft_latch`; `math::{sigmoid,tanh,exp,sin,pow,gaussian}` use the f64-then-f32 path | `stateless_activations`, `stateful_activations`, `output_clamp_applies`, `extreme_inputs_stay_finite_and_clamped` |
| §3 scheduling | implemented | `brain.rs::step`; body owns cadence (`body.rs`), period from `SimSettings::brain_period()` | `brain_cadence_and_senses_are_wired` (body), `inputs_are_one_brain_tick_stale` |
| §4 inputs (33) | implemented | `brain.rs` `BrainInputs` + `WorldSenseSource`/`InternalSenseSource`; world values `senses.rs`, body values `body.rs` | `trait_getters_round_trip`, `apply_fov_writes_vision_inputs_only`, `apply_phero_writes_phero_inputs_only` |
| §5 outputs (15) | implemented | `brain.rs::Output::node_type`; consumers in `organs.rs`/`body.rs` | `organs_read_live_brain_output_values_on_first_tick`, `starter_structure_and_upkeep` |
| §6 CSR execution | implemented | `brain.rs` CSR plan, synchronous `last_out` reads, floating-hidden repair, usage masks | `compiled_step_matches_reference_interpreter`, `step_is_deterministic_over_many_ticks`, `step_plan_rebuilds_after_topology_change`, `step_batch_is_bit_identical_to_serial` |
| §7 the 35 genes | implemented | `genome.rs` `Gene`, `GENE_DEFS`, derived formulas | `gene_defs_match_enum_order_and_defaults`, `set_gene_clamps_to_editor_range`, `derived_formulas_match_starter_numbers` |
| §8 mutation | implemented (game-statistical, ours deterministic) | `genome.rs::child` + kernel; `brain.rs` mutation ops | `mutate_is_deterministic`, `mutate_keeps_brain_valid`, `child_is_deterministic_and_in_range`, `child_respects_prevent_mutations` |
| §9 speciation | implemented | `species.rs` distance/assignment/lineage/name generator/io | `assign_creates_root_then_stays`, `assign_stays_within_span`, `assign_adopts_child_species_within_half_span`, `two_step_mutant_rule_founds_new_species`, `distance_*`, `records_roundtrip_preserves_lineage` |
| §10 persistence / migrations | implemented for the 0.6.3.x tier; §10.6 "defensive era detection" is not implemented | io `lib.rs`/`model.rs` records; `migrate.rs` brain/gene gates; `version.rs` | io tests `i1`, `template_compat`; version parser tests |
| §11 Rust design notes | implemented (per-brain `Vec` compaction from §11.3 is not done) | `brain.rs` SoA + CSR + masks; `step_batch` for parallel brains | `parity_accessors_read_write_state`, usage-mask tests |
| §12 open questions | Q1, Q3–Q5 closed by parity capture; Q8/Q10/Q13 implemented as documented; the rest remain open | — | — |

### Implementation notes (deviations and known gaps)

* **Mutation is deterministic for us but only statistically faithful to the game** (D4/D5): the
  Poisson/Gaussian kernel, branch probabilities, index ranges and clamps follow §8 exactly, while the
  game's unseeded `System.Random`/`UnityEngine.Random` streams are replaced by domain-derived `Rng` streams.
* **Bounded re-rolls instead of recursion.** `ChangeDefaultAct` retries at most 1024 node draws then no-ops
  (Q8); `AddSynapse`'s herding re-roll loops at most 64 times then returns. RNG consumption matches the
  game unless the guard fires (only reachable with abnormal brains).
* **The 0.6a13 "Constant input removal" migration uses a deterministic input pick**
  (`migrate::deterministic_input`) instead of Unity `Random.Range(1,14)` — see §10.4 and D4.
* **Known gap (code, reported to Lead):** the game re-clamps SoftLatch biases to `[0.1, 100]` on every
  `FinalizeEditing`/`ResumeBrain` (§2.2, `NEATBrain.cs:475-478`); `Brain::build` preserves loaded biases
  verbatim, so an out-of-range hand-edited file is not repaired. Mutation/type-change paths do clamp.
* io template loading is deliberately tolerant (node-index/duplicate repair) and re-emits canonically at
  0.6.3.1 — see `D14` and docs/DEVIATIONS.md "Template tolerance".

---

## 0. Conventions, units, terminology

| Term | Meaning |
|---|---|
| sim tick | one fixed simulation step; `dt = 1/sim_tps`, default `sim_tps = 30` → `dt = 1/30 s` |
| brain tick | one execution of a bibite's brain; once every `brain_tps` sim ticks, `brain_tps` default 2 → 15 Hz (`Here/SettingScripts/ScenarioIndependentSettings.cs:170-192`, `Here/SimulationScripts.BibiteScripts/BibiteBody.cs:592-621`) |
| brain period (`brainPeriod`) | value passed to stateful activations. Computed as `brain_tps / sim_tps` (a **ratio**, numerically equal to the brain dt in seconds only when `Time.fixedDeltaTime == 1/sim_tps`). Default `2/30 = 0.06666…` (`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:332-342`) |
| Inov | NEAT innovation number (long). Base nodes: `Index+1`; connections: monotonically increasing from 1 |
| u | world distance unit |
| E | energy unit |
| `f(x)` | activation function output after bias handling |
| `x` | "full activation" = raw input after bias handling (except Inhibitory/SoftLatch/Integrator which handle bias specially) |
| TPS | ticks per second |

Sign conventions (verified from math + starter-genome behaviour, see §12 Q1 for the tooltip conflict):

- Heading/forward is `transform.up` (Unity 2D, y-up). Angles are measured from forward.
- `Vector2.SignedAngle(from, to)` in Unity is
  `unsigned_angle * Mathf.Sign(from.x*to.y - from.y*to.x)` (UnityCsReference `Vector2.cs`), i.e. positive for a
  target **counter-clockwise** from `from`. Therefore a target on the bibite's **right** (`+x`, forward `+y`)
  gives a **negative** input angle; a target on the left gives positive. The in-game tooltips say the opposite
  ("negative left, positive right"); the tooltips mirror the handedness of the world. Behavioural truth is the
  pairing: **positive angle + positive Rotate steers toward the target** (proof: "Basic bibite" template
  `PlantAngle --(+1.159)--> Rotate`, §13.3).
- `Rotate` output: added directly as `Rigidbody2D.AddTorque(output * turn_strength)`; positive torque = positive
  angular velocity = counter-clockwise in (x-right, y-up) coordinates = turn toward −x = the bibite's left.
  Tooltips call positive "clockwise"; same mirrored labelling.
- `Speed` input: positive = forward. `RotationSpeed` input: `angularVelocity_deg_per_s / 360` → revolutions/s,
  positive = counter-clockwise in math coordinates (tooltip says "clockwise", same mirror).
- All neuron outputs are clamped to **[-1024, +1024]** (`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:622-639`).

---

## 1. Brain data model

### 1.1 Enumerations (authoritative, in declaration order)

`Inputs` — 33 values, indices 0..32 (`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:14-49`), `NInputs = 33`
(`:243`). `Outputs` — 15 values, indices 0..14 (`:51-68`), `NOutputs = 15` (`:245`). `NodeType` — 14 values,
0..13 (`:70-86`), `NNodesTypes = 14` (`:247`).

| NodeType value | Enum | Usable as | Node output range (undamped) |
|---:|---|---|---|
| 0 | `Input` | reserved for inputs only | −∞..∞ (raw sense) |
| 1 | `Sigmoid` | hidden + outputs | 0..1 |
| 2 | `Linear` | hidden + outputs | −∞..∞ (clamped) |
| 3 | `TanH` | hidden + outputs | −1..1 |
| 4 | `Sine` | hidden + outputs | −1..1 |
| 5 | `ReLu` | hidden + outputs | 0..∞ (clamped) |
| 6 | `Gaussian` | hidden + outputs | 0..1 |
| 7 | `Latch` | hidden + outputs | 0 / 1 / held |
| 8 | `Differential` | hidden + outputs | −∞..∞ (clamped) |
| 9 | `Abs` | hidden + outputs | 0..∞ (clamped) |
| 10 | `Mult` | hidden + outputs | −∞..∞ (clamped) |
| 11 | `Integrator` | hidden + outputs | −∞..∞ (clamped) |
| 12 | `Inhibitory` | hidden + outputs | −∞..∞ (clamped) |
| 13 | `SoftLatch` | hidden + outputs | 0..1 (saturates) |

`NodeArchetype` = { `Input`, `Output`, `Hidden` } (`:165-170`). It is **derived**, not stored: a node is
`Input` if `Index < NInputs`, `Output` if `NInputs <= Index < NInputs+NOutputs`, else `Hidden`
(`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:114-128`).

### 1.2 Node record

`NEATBrain.Node` is a struct (`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:88-163`):

| Field | Type | Persisted in memory save (.bb8 / save bundle) | Persisted in `.bb8template` | Meaning |
|---|---|---|---|---|
| `Type` | NodeType (int) | yes | yes | activation function |
| `baseActivation` | float | yes | yes | bias / default activation (name is historical) |
| `TypeName` | string | yes | no | `Type.ToString()`, cached; not used by simulation |
| `Index` | int | yes | yes | **array position**; synapses refer to nodes by this index |
| `Inov` | long | yes | yes | innovation number |
| `Desc` | string | yes | yes | `"EnergyRatio"`, `"Accelerate"`, `"Hidden0"`, … cosmetic; base nodes regenerated from enums on load |
| `Value` | float | yes | no | current output (post-activation, clamped) |
| `LastInput` | float | yes | no | previous tick's full activation `x` (after bias handling) |
| `LastOutput` | float | yes | no | previous tick's output (== `Value` after execution) |
| `NIn` | int | **no** ([NonSerialized]) | no | #incoming synapses (recomputed) |
| `NOut` | int | **no** | no | #outgoing synapses (recomputed) |

Initial values of a freshly copied node: `Value = LastInput = LastOutput = 0`; `NIn/NOut` supplied by the copier
(`:130-143`, `:1006-1009`).

Persistence notes:
- Memory/save JSON is produced by Newtonsoft over `[SerializeField]` fields (`Here/ScriptHelpers/SerializationHelper.cs:59-89`),
  so a node object serializes as the public fields above **plus** the computed read-only property `archetype`
  (Json.NET serializes public getters). Real 0.6.3.1 node:
  `{"Type":0,"baseActivation":0.0,"TypeName":"Input","Index":0,"Inov":1,"Desc":"EnergyRatio","Value":0.7752547,"LastInput":0.0,"LastOutput":0.775226533,"archetype":0}`
  (`docs/spec/_tmp/recent/bibites/bibite_0.bb8`). `NIn/NOut` are excluded by `[NonSerialized]`.
- `.bb8template` uses the hand-written `SaveForTemplate()` subset (`:145-162`).

### 1.3 Synapse record

`Synaps` struct (`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:172-183`):

| Field | Type | JSON key | Meaning |
|---|---|---|---|
| `Inov` | long | `Inov` | innovation number; **primary key for cross-genome alignment** (speciation) |
| `NodeIn` | int | `NodeIn` | source node index |
| `NodeOut` | int | `NodeOut` | destination node index |
| `Weight` | float | `Weight` | weight; `Weight * source.LastOutput` is the signal |
| `En` | bool | `En` | enabled; disabled synapses exist but do not transmit |

Ordering invariant used by the game: **both `Nodes[]` and `Synapses[]` are sorted ascending by `Inov`**.
- Base nodes get `Inov = Index+1`; hidden nodes are appended with `++nodeMaxInov` (monotone)
  (`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:877-914`, `Here/ScriptHelpers/BrainUpdater.cs:301-314`).
- New synapses append `connectionMaxInov++`, so creation order == Inov order (`:680-719`).
- Reproduction copies arrays preserving order; removal compacts preserving order (`:963-1004`).
- Speciation's distance algorithm is a **linear merge that assumes sorted-by-Inov arrays**
  (`Here/SimulationScripts.BibiteScripts/Species.cs:178-255`); Rust must keep the invariant.

### 1.4 Brain container state (`NEATBrain`)

Runtime/SoA-relevant fields:

| Field | Persisted | Meaning |
|---|---|---|
| `Nodes` (`Node[]`) | yes | dense array, length `NInputs+NOutputs+nHidden` |
| `Synapses` (`Synaps[]`) | yes | dense array, length `nSynaps` |
| `isReady` (bool) | yes | brain finished initialisation; gates `ResumeBrain`/clock |
| `nHidden` | derived | `Nodes.Length - 48` (`:364`, `:377`) |
| `nSynaps` | derived | `Synapses.Length` (`:365`, `:388`) |
| `nActiveInputs`, `nActiveOutputs` | recomputed, not saved | #inputs with `NOut>0`, #outputs with `NIn>0` (all synapses, enabled or not) (`:467-507`) |
| `brainTargets`, `brainLinks` | recomputed, not saved | execution CSR (see §6) |
| `mutationProbability`, `mutationVariance` | **not saved** (`[NonSerialized]`-less but not `[SerializeField]`) | copied from genes at reproduction: `genes[11]`, `genes[10]+bgVar+intensityIncrease` (`:377-379`) |
| `nodese`, `synapse` | scratch `List<Node>`/`List<Synaps>` used only during editing/mutation, deleted on `FinalizeEditing` | `:201-203`, `:428-465` |

`brainUpkeepCost = nSynaps * synapse_upkeep + nHidden * neuron_upkeep` — hidden nodes only, all synapses, **per
second**, charged every sim tick as `cost * dt` (`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:283`,
`Here/SimulationScripts.BibiteScripts/BibiteBody.cs:603`). Defaults: neuron upkeep `0.001 E/s`, synapse upkeep
`0.00025 E/s` (`Here/SettingScripts/ScenarioSettings.cs:745-771`).

Innovation counters are **global statics**: `nodeMaxInov = NInputs+NOutputs+1 = 49`,
`connectionMaxInov = 1` (`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:251-253`). First hidden node created
therefore gets Inov 50 (49 is never used). They are saved/restored by `GlobalLineageManager` (`§9.5`).

---

## 2. Node activation functions

### 2.1 Reference implementation (exact)

`ExecuteNeuron` (`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:612-641`):

```
execute_neuron(node, raw_input, period):
    # 1. bias handling
    if type in {Integrator, Inhibitory, SoftLatch}:        # TypeUsesBiasDifferently
        x = raw_input
    elif type == Mult:
        x = raw_input * node.base_activation
    elif type == Input:
        x = raw_input                                        # TypeHasBias(Input) == false
    else:
        x = raw_input + node.base_activation

    # 2. activation + clamp
    switch type:
        Input        -> x
        Sigmoid      -> 1f / (1f + Mathf.Pow((float)Math.E, 0f - x))
        Linear       -> x
        TanH         -> (float)Math.Tanh(x)
        Sine         -> Mathf.Sin(x)
        ReLu         -> Mathf.Max(x, 0f)
        Gaussian     -> 1f / (1f + Mathf.Pow(x, 2f))
        Latch        -> if x >= 1: 1
                        elif x <= 0: 0
                        else: node.LastOutput
        Differential -> (x - node.LastInput) / period
        Abs          -> Mathf.Abs(x)
        Mult         -> x
        Integrator   -> node.LastOutput + x * period
        Inhibitory   -> x - node.LastInput + node.LastOutput * Mathf.Exp((0f - node.base_activation) * period)
        SoftLatch    -> soft_latch(node, x)

    out = Mathf.Clamp(activation, -1024f, 1024f)
    node.Value      = out
    node.LastOutput = out
    node.LastInput  = x
```

**Transcendental precision (parity-critical).** `Mathf.Pow`, `Mathf.Exp`,
`Mathf.Sin` and `Math.Tanh` are Unity/Mono wrappers that evaluate the operation in
**`f64`** and cast the result back to `f32` (`Mathf.Pow(f, p) = (float)Math.Pow(f, p)`,
`Mathf.Sin(f) = (float)Math.Sin(f)`, `Mathf.Exp(f) = (float)Math.Exp(f)`,
`TanH => (float)Math.Tanh(input)`). A Rust implementation must therefore call the
double-precision function and truncate (`(f64::from(x).tanh()) as f32`,
`(f64::from(x).exp()) as f32`, …), **not** the `f32` intrinsics. This was the first
real game-vs-Rust parity bug: `tanhf(0.4535929f32)` returns `0x3ed985a9` while the
game's double path returns `0x3ed985aa`, making the `Accelerate` output 1 ULP low
on every tick. The game sigmoid's base is `(float)Math.E` (the `f32` constant
2.7182817) passed through `Mathf.Pow`, and its `double` power saturates before the
`f32` cast, so the formula is overflow-safe without expf-style branching.

**Mixed precision is intentional and bounded (Lead decision 2026-10-30).** The `f64`-then-`f32` rule
applies to **brain activation** functions only, because those values feed the network directly and were
observably 1 ULP off with `f32` intrinsics. Genome-derived quantities (e.g.
`MetabolicEfficiency = energyUsageEfficiency^SpeedRatio`, `growthAtMature`, body-energy densities) are
computed with `f32` (`powf`/`sqrt`) in `genome.rs`; these matched the game in the Ring parity run
(energy bit-exact over 300 ticks) and the independent braincheck is 0 ULP. Do not generalise the f64
rule to gene formulas, and do not switch the activation path to `f32` intrinsics: both directions were
validated empirically (`docs/PARITY.md`).

The bias classification helpers are (`:285-310`):
`TypeHasBias` = false for `Input` and `Integrator`; `TypeUsesBiasDifferently` = true for `Integrator`, `Mult`,
`Inhibitory`, `SoftLatch`; `TypeIsNonLinear` = true for `Differential`, `Inhibitory`, `Integrator`, `Latch`,
`SoftLatch` (used by the UI only).

Important: for `Inhibitory` and `SoftLatch`, `node.LastInput` receives the **raw input** (no bias added), and for
`Mult` it receives the **bias-multiplied** product. This matters because Inhibitory/SoftLatch compare
`x` to `LastInput` — biases enter them only through their decay/hysteresis term.

### 2.2 Stateful / non-linear functions in detail

**Latch** (`:631`) — ternary switch with hysteresis, no bias (bias added to `x` normally because it is not in
`TypeUsesBiasDifferently`!).

```
out = 1 if x >= 1.0
    = 0 if x <= 0.0
    = previous output otherwise       (initial 0)
```
Both thresholds inclusive; "previous output" is `LastOutput` of the previous brain tick.

**Differential** (`:632`)

```
out = (x - LastInput) / period
```
Euler derivative. `period = brainPeriod`. A constant bias produces a one-tick spike on the first tick after
creation (because `LastInput` starts at 0), then 0. `LastInput` is set to `x` after evaluation.

**Integrator** (`:635`)

```
out = LastOutput + x * period
```
No bias (bias forced 0 by `ChangeNeuronType`; `ChangeDefaultAct` refuses Integrator targets, `:916-935`).
No leak; only the ±1024 clamp limits it. This is the only unbounded memory node.

**Inhibitory** (`:636`)

```
out = x - LastInput + LastOutput * exp(-base_activation * period)
```
`base_activation` is clamped to `[0.0001, 10]` whenever set/changed (`:924-928`, `:947-950`); default bias when a
node is created as Inhibitory is `10^U(-2,1)` (`:893-897`); `DefaultBaseActivationOfType(Inhibitory) = 1`
(`:1083-1092`). Beimut: acts like Differential but leaks the memory term toward 0; higher bias = faster decay.
`LastInput = x` afterwards.

**SoftLatch** (`:1105-1133`)

```
soft_latch(node, x):
    b    = node.base_activation           # clamped [0.1, 100]
    last = node.LastOutput
    li   = node.LastInput
    if x >= 0.999: return 1
    if x <= 0.001: return 0
    if li >= 0.999: li = 1
    if li <= 0.001: li = 0
    if x < li:
        return last / (1 - exp(-b*li)) * (1 - exp(-b*x))
    t = (last - exp(b*(li-1))) / (1 - exp(b*(li-1)))
    return t + (1 - t) * exp(b*(x-1))
```
Saturating hysteresis: hard 1 when driven `>= 0.999`, hard 0 when `<= 0.001`; otherwise moves toward the last
value exponentially, with the bias controlling stiffness (low b ≈ linear, high b ≈ Latch). Bias is **not** added
to the input. Default bias when created: `10^U(-1,1)` (`:901-903`); `DefaultBaseActivationOfType = 5`
(`:1083-1092`). `baseActivation` re-clamped to `[0.1, 100]` every time the brain is finalised
(`:475-478`).

**Mult** (`:634`) — input summation is a **product** (§6.2); the bias is an extra multiplicative factor. Node
output = product; a zero bias silences the node permanently.

> NB `Latch`'s bias *is* added normally (it is not in `TypeUsesBiasDifferently`): a positive bias can push a
> Latch above 1 forever. `Differential` bias is also added normally and only ever fires once at birth
> (in-game description, `Here/Utility.InformationWrapers/NodeInformations.cs:457-463`).

### 2.3 Bias/default-activation rules

| Operation | Rule | Ref |
|---|---|---|
| Any non-input node can carry a bias | stored in `baseActivation` | `:88-143` |
| Input nodes | bias never applied (`TypeHasBias(Input)==false`) | `:285-292` |
| Integrator | bias not applied; mutations skip it; type change forces bias 0 | `:916-923`, `:951-954` |
| Mult | bias multiplies the product; default 1.0 on add; type change clamps | `:898-900`, `:947-950` |
| Inhibitory | bias only in decay; clamp `[1e-4, 10]` | `:924-928`, `:947-950` |
| SoftLatch | bias only in hysteresis; clamp `[0.1, 100]` | `:929-932`, `:955-958`, `:475-478` |
| Brain finalisation executes every non-input node once with `input=0` | seeds `LastOutput/Value` from bias (e.g. Sigmoid with bias 0 → **0.5**) | `:453-460` |

---

## 3. Brain scheduling and update order

### 3.1 Tick constants and derived cadences (defaults)

| Quantity | Setting | Default | Derived behaviour | Ref |
|---|---|---:|---|---|
| sim ticks/s | `simTPS` | **30** | `dt = 1/30 s` | `Here/SettingScripts/ScenarioIndependentSettings.cs:170-180` |
| brain update factor | `brainTPS` (in-game name **"Brain Update Factor"**, units "X") | **2** | brain tick every 2 sim ticks = **15 Hz** | `:182-192` |
| brain dt for activations | `brainPeriod` | **2/30 = 0.0667** | `brainPeriod = brain_tps/sim_tps` | `Here/SimulationScripts.BibiteScripts/NEATBrain.cs:332-342` |
| vision entity scan | `visionLookupUpdateFactor` | 10 | every 10 brain ticks = 20 sim ticks = **0.667 s** | `Here/SettingScripts/ScenarioIndependentSettings.cs:206-216` |
| vision sense values | `visionSenseUpdateFactor` | 5 | every 5 brain ticks = 10 sim ticks = **0.333 s** | `:218-228` |
| pheromone sensing | (no setting) | 1 | every sim tick, **after** the brain tick in the body update (so 1 tick stale) | `Here/SimulationScripts.BibiteScripts/BibiteBody.cs:626-631` |
| internal clock / growth | Unity `FixedUpdate` per script | 1 | every sim tick, but script order vs body update is **not** fixed (see Q4) | `Here/SimulationScripts.BibiteScripts/InternalClock.cs:65-97`, `Here/SimulationScripts.BibiteScripts/BibiteGrowth.cs:91-107` |

`Time.fixedDeltaTime` is `1/simTPS` at timeScale ≥ 1 and is scaled down in slow motion
(`Here/ManagementScripts/TimeController.cs:58,78-82,117-130`), but `brainPeriod` is **not** rescaled — stateful
activations are inaccurate in slow motion. Rust always runs at `dt = 1/sim_tps` (no slow-motion), so this is a
non-issue for the headless engine.

**`BrainUpdateFactor` vs `simTPS` — do not conflate them.** `brainTPS` (settings UI: *Brain Update Factor*, unit
"X") is the number of **sim ticks between brain updates** (`BibiteBody.cs:592-621`); it is *not* a rate and not the
brain period. The scalar handed to stateful neurons is `brainPeriod = brainTPS/simTPS`, which is dimensionally a
*fraction of a second only because* `Time.fixedDeltaTime == 1/simTPS`; the true brain timestep is
`brainTPS * fixedDeltaTime = brainPeriod` seconds at timeScale 1. Consequences for the port:
`Brain::step(..., period)` must receive `s.brain_period()` computed from the **effective settings**
(`sim_tps` is scenario-overridable, e.g. 15 for `Ring.zip`, 30 default, 40 in some scenarios), and the body's
brain-tick accumulator counts **sim ticks**, comparing `brainTicksCount >= brain_tps`. Never hardcode 30/2/0.0667.
If a settings animator changes `simTPS`/`brainTPS` mid-run, recompute both `dt` and `brain_period` from the new
values; the accumulators are not rescaled (the phase shifts, next update uses the new threshold — §10.7).

### 3.2 Per-bibite update ordering (one sim tick)

`BibiteBody.FixedUpdate` (`Here/SimulationScripts.BibiteScripts/BibiteBody.cs:592-639`), body-gated on
`born && timeScale>0`:

```
1. if dead: CorpseUpdate(); return
2. UseEnergy((metabolism + brainUpkeepCost) * dt)
3. brainTicksCount += 1
4. if brainTicksCount >= brainTickFactor:                       # BRAIN TICK BLOCK
       visionLookupCount += 1
       if visionLookupCount >= visionLookupPeriod:  fow.FindSeenEntities(); visionLookupCount -= period
       visionSensingCount += 1
       if visionSensingCount >= visionSensingPeriod: fow.ComputeSenses();      visionSensingCount -= period
       brain.Thinker()
       brainTicksCount -= brainTickFactor
5. attackedDmg decay: attackedDmg = max(0, attackedDmg*0.70716^dt - 0.003*dt)
6. for each organ (armor, stomach, eggLayer, fat, move, mouth, pheromoneOrgan): organ.UpdateOrgan()
7. pherosenseOrgan.PherosenseAround()          # one sim tick after the brain saw pheromone values
8. NaN-health check, Heal(), rigidbody mass/inertia/drag update
```

Other scripts (`InternalClock`, `BibiteGrowth`, `EggHatching`, `BibitePheromoneOrgan`, …) also implement
`FixedUpdate`; Unity does not guarantee relative order between different MonoBehaviours. The Rust pipeline must
pick one order (ARCHITECTURE §6 has sensing → brain → organs) and validate with parity captures; list of
order-sensitive reads in Q4.

At birth/resume the brain-tick phase is randomised to de-synchronise brains
(`Here/SimulationScripts.BibiteScripts/BibiteBody.cs:436-438`, `:454-456`):
`brainTicksCount = Random.Range(0, brainTickFactor)` (int overload → `[0, brainTickFactor-1]`),
`visionSensingCount = Random.Range(0, visionSensingPeriod)`,
`visionLookupCount = Random.Range(0, visionLookupPeriod)`. These three counters are **persisted** in the body save.

### 3.3 `Thinker()` — one brain tick

`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:553-568`

```
thinker():
    update_senses()                              # §4.1
    for i in 0..brainTargets.len:
        target = brainTargets[i]
        acc = 1.0 if Nodes[target].Type == Mult else 0.0
        for syn_index in brainLinks[i]:
            s = Synapses[syn_index]
            term = s.Weight * Nodes[s.NodeIn].LastOutput
            acc = acc * term if Mult else acc + term
        execute_neuron(Nodes[target], acc, brainPeriod)
```

Key semantics:

1. **Every node reads `LastOutput`, i.e. the value produced in the previous brain tick.** Evaluation order is
   therefore behaviourally irrelevant (synchronous update). This is what makes per-bibite parallelisation and a
   CSR layout trivially safe.
2. `update_senses()` first shifts each input's `Value` into its `LastOutput`, then overwrites `Value` with the
   freshly sampled sense. **All 33 inputs therefore reach the network exactly one brain tick late.** The same
   holds for inputs consumed as synapses: they read `LastOutput`.
3. Only nodes that are the **destination of at least one enabled synapse** are executed (`brainLinks` built from
   enabled synapses only, §6). An output/hidden node with only disabled incoming synapses is never re-executed:
   its value stays whatever `FinalizeEditing`/save-data left there.
4. `Mult` targets start from 1.0; with zero incoming enabled synapses a Mult node would output its bias — but it is
   not executed at all in that case.
5. An input node can in principle be a synapse destination (corrupt/edited brains): it would be executed with the
   raw weighted sum as `Input` activation. Not produced by normal mutation.

### 3.4 Initialisation-order facts that affect behaviour

- `FinalizeEditing()` rebuilds `Nodes`/`Synapses`, sets `Index = position`, then executes **each node from
  `NInputs` upward once with `input=0, period=brainPeriod`** (`:428-465`). Consequences: Sigmoid outputs with zero
  bias start at **0.5**, TanH at 0, ReLU at 0, Mult at its bias, SoftLatch at 0, etc. This initial state is what
  the first brain tick's `LastOutput` reads.
- `CopyBrain()` (reproduction/template spawn) copies only static fields (`Type, Inov, Desc, Index, baseActivation`;
  `NIn/NOut` passed through) and **zeroes `Value/LastInput/LastOutput`** (`:1006-1009`), then
  `FinalizeEditing()` re-seeds state as above. So stateful neurons **do not inherit state** from parents.
- `ResumeBrain()` (save load) only computes `nHidden`, `nSynaps`, active-neuron counts, processing structure and
  sensor start-up (`:360-371`); it does **not** execute or reset neurons, so saved
  `Value/LastInput/LastOutput` are live state that must round-trip.
- `ComputeActiveNeurons()` can recursively re-finalise the brain if any hidden node has `NIn<1 || NOut<1`
  ("floating" hidden node): it calls `CopyBrain(Nodes, Synapses, mutate:false, finalizeRightAway:true,
  checkForFloatingHidden:true)` which removes floating hidden nodes (`:488-495`, `:412-421`). This is a load-time
  repair path; Rust should implement it as a validation pass.

---

## 4. Inputs (33)

### 4.1 Sense sampling (`UpdateSenses`, exact)

`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:570-610`

```
update_senses():
    for i in 0..NInputs: Nodes[i].LastOutput = Nodes[i].Value     # previous values become readable by the net
    Nodes[0]  = body.Energy.Amount / body.Energy.MaxAmount
    Nodes[1]  = growth.growth                                    # raw growth, NOT maturity ratio
    Nodes[2]  = body.healthRatio                                 # Health.Amount / Health.MaxAmount
    Nodes[3]  = body.stomach.fullness                            # totalAmount / area
    Nodes[4]  = body.move.NormalizedVelocity                     # signed forward speed / body length
    Nodes[5]  = body.move.AngularVelocity                        # radians? no: angularVelocity_deg_per_s / 360
    Nodes[6]  = body.mouth.nHeld                                 # int count, not a boolean
    Nodes[7]  = health > 0 ? min(1, 2*body.attackedDmg/body.health) : 0
    Nodes[8]  = body.eggLayer.nEgg                               # floor(eggProgress)
    Nodes[9]  = fow.bibiteConcentrationWeight
    Nodes[10] = fow.bibiteConcentrationAngle
    Nodes[12] = fow.pelletConcentrationWeight
    Nodes[13] = fow.pelletConcentrationAngle
    Nodes[15] = fow.meatConcentrationWeight
    Nodes[16] = fow.meatConcentrationAngle
    Nodes[11] = fow.nBibite / 4
    Nodes[14] = fow.nPlants / 4
    Nodes[17] = fow.nMeat / 4
    Nodes[18..20] = fow.targetR/G/B
    Nodes[21] = clk.tic                                          # int 0/1
    Nodes[22] = clk.chronoTime / 60
    Nodes[23] = clk.timeAlive / 60
    Nodes[24..26] = tanh(phero.pheroSum{1,2,3} / 5)
    Nodes[27..29] = phero.angleToPhero{1,2,3}                    # signed angle / 180
    Nodes[30..32] = phero.angleToPhero{1,2,3}Heading             # signed angle / 180
```

### 4.2 Input table

| Idx | Enum | In-game name | Exact computation (game terms) | Range | Units | Refs |
|---:|---|---|---|---|---|---|
| 0 | `EnergyRatio` | Energy Ratio | `Energy.Amount / Energy.MaxAmount`; `MaxAmount = baseBodyArea * storableEnergyPerArea` (1.5) | 0..1 | ratio | `NEATBrain.cs:576`; `BibiteBody.cs:720`; `ScenarioSettings.cs:860-872` |
| 1 | `Maturity` | Maturity | `growth.growth` (raw growth, 0 at birth = `growthAtBirth`), not `growth/growthAtMature` | 0..∞ | growth units | `NEATBrain.cs:577`; `BibiteGrowth.cs:21,52,64` |
| 2 | `LifeRatio` | Health Ratio | `Health.Amount / Health.MaxAmount`; `MaxAmount = healthPerArea * baseBodyArea` (1.25) | 0..1 | ratio | `NEATBrain.cs:578`; `BibiteBody.cs:292,719`; `ScenarioSettings.cs:840-858` |
| 3 | `Fullness` | Fullness | `stomach.totalAmount / stomach.area`; `area = baseBodyArea * max(0,StomachWAG)/totalOrganWAG` | 0..1 | ratio | `NEATBrain.cs:579`; `BibiteStomach.cs:79-89`; `BibiteSpatialOrgan.cs:21-36` |
| 4 | `Speed` | Speed | `dot(linearVelocity, transform.up) / bodyLength`; `bodyLength = 12.5*d1Size` | −∞..∞ | body lengths/s | `NEATBrain.cs:580`; `BibitePropulsion.cs:248-257`; `BibiteBody.cs:296` |
| 5 | `RotationSpeed` | Angular Speed | `rigidbody.angularVelocity_deg_per_s / 360` | −∞..∞ | revolutions/s (CCW+) | `NEATBrain.cs:581`; `BibitePropulsion.cs:254` |
| 6 | `IsGrabbing` | Is Grabbing | `mouth.nHeld` (integer count 0..10; **not** 0/1) | 0..10 | count | `NEATBrain.cs:582`; `BibiteMouth.cs:38,290-292` |
| 7 | `AttackedDamage` | Received Attack Damages | `health>0 ? min(1, 2*attackedDmg/health) : 0`; `attackedDmg` += raw damage per hit (`Attack`), decays per tick | 0..1 | ratio | `NEATBrain.cs:583-584`; `BibiteBody.cs:622-625,750-760` |
| 8 | `EggStored` | Eggs Stored | `floor(eggProgress)` | 0..eggCapacity | eggs | `NEATBrain.cs:585`; `BibiteEggLayingOrgan.cs:79` |
| 9 | `BibiteCloseness` | Seen Bibites Closeness | see §4.3 | 0..1 | ratio | `FieldOfView.cs:502-503` |
| 10 | `BibiteAngle` | Seen Bibites Angle | `2*SignedAngle(up, weightedAvgVec) / viewAngle` → −1..1 (inside FOV) | −1..1 | normalized | `FieldOfView.cs:502` |
| 11 | `NBibites` | Seen Bibites Number | `nBibite / 4` | 0..∞ | count/4 | `NEATBrain.cs:592`; `FieldOfView.cs:460-489` |
| 12 | `PlantCloseness` | Seen Plants Closeness | §4.3 | 0..1 | ratio | `FieldOfView.cs:504-505` |
| 13 | `PlantAngle` | Seen Plants Angle | `2*SignedAngle(up, avgVec)/viewAngle` | −1..1 | normalized | `:504` |
| 14 | `NPlants` | Seen Plants Number | `nPlants / 4` | 0..∞ | count/4 | `NEATBrain.cs:593` |
| 15 | `MeatCloseness` | Seen Meats Closeness | §4.3 | 0..1 | ratio | `FieldOfView.cs:506-507` |
| 16 | `MeatAngle` | Seen Meats Angle | as above | −1..1 | normalized | `:506` |
| 17 | `NMeats` | Seen Meats Number | `nMeat / 4` | 0..∞ | count/4 | `NEATBrain.cs:594` |
| 18 | `RedBibite` | Closest Bibite's Red Gene | raw `genes[5]` of the closest seen bibite (`dist2Target <= dist2Bibite`) | 0..1 | gene value | `NEATBrain.cs:595`; `FieldOfView.cs:479-488` |
| 19 | `GreenBibite` | Closest Bibite's Green Gene | raw `genes[6]` | 0..1 | gene value | `NEATBrain.cs:596` |
| 20 | `BlueBibite` | Closest Bibite's Blue Gene | raw `genes[7]` | 0..1 | gene value | `NEATBrain.cs:597` |
| 21 | `Tic` | Clock tick | square wave 0/1, toggles every `ClockSpeed` seconds (full cycle 2×ClockSpeed) | {0,1} | bool | `NEATBrain.cs:598`; `InternalClock.cs:65-97` |
| 22 | `Minute` | Chronometer | `chronoTime / 60`; `chronoTime` resets when `ClkReset > 0.75`, else `+= dt` | 0..∞ | minutes | `NEATBrain.cs:599`; `InternalClock.cs:76-83` |
| 23 | `TimeAlive` | Time Alive | `timeAlive / 60` | 0..∞ | minutes | `NEATBrain.cs:600`; `InternalClock.cs:90` |
| 24 | `PheroSense1` | Red Pheromones Strength | `tanh(pheroSum1 / 5)` | 0..1 | saturating | `NEATBrain.cs:601`; `Pherosense.cs:157-195` |
| 25 | `PheroSense2` | Green … | `tanh(pheroSum2 / 5)` | 0..1 | | `:602` |
| 26 | `PheroSense3` | Blue … | `tanh(pheroSum3 / 5)` | 0..1 | | `:603` |
| 27 | `Phero1Angle` | Red Pheromones Angle | `SignedAngle(up, Σ(delta_i*w_i)) / 180` | −1..1 | normalized | `NEATBrain.cs:604`; `Pherosense.cs:209` |
| 28 | `Phero2Angle` | Green … | same | −1..1 | | `:605` |
| 29 | `Phero3Angle` | Blue … | same | −1..1 | | `:606` |
| 30 | `Phero1Heading` | Red Pheromones Heading | `SignedAngle(up, Σ(heading_i*w_i)) / 180` | −1..1 | normalized | `NEATBrain.cs:607`; `Pherosense.cs:212` |
| 31 | `Phero2Heading` | Green … | same | −1..1 | | `:608` |
| 32 | `Phero3Heading` | Blue … | same | −1..1 | | `:609` |

Units of the "closeness" channels: ratio of view radius, 0.998 when the weighted centroid is essentially at the
bibite, 0 when at/over the view radius (`1 - min(|avgVec|/viewRadius, 1)`).

### 4.3 Vision pipeline (exact)

Entity list `FindSeenEntities` (every 10 brain ticks) — `Here/SimulationScripts.BibiteScripts/FieldOfView.cs:210-300`:

1. If `needToSee == false` (mask 0) return.
2. Broad phase: if `viewAngle/2 < 5π/8` (≈112.5°) use an oriented `OverlapBox` (half-extent
   `min(0.25*viewRadius, 25)` padding, oriented by the body); else `OverlapCircle(viewRadius + padding)`
   (`:236-251`).
3. Pellets: plant if `material==plant && mask&2`, meat if `material==meat && mask&4`; max 128 each.
4. Bibites: dedupe by parent GameObject, append to `seenBibites`; dead bibites are also appended to
   `seenCorpses` (a corpse thus appears in **both** the bibite and meat channels).

Sense computation `ComputeSenses` (every 5 brain ticks) — `:302-521`:

For every candidate, `TargetIsInView(transform, objectRadius)` (`:538-584`) computes `dir2Target`, `dist2Target`,
`angle2Target`:

```
d       = target.position - self.position
dist    = |d|
if dist - objectRadius > viewRadius: reject
angle   = SignedAngle(self.up, d)
if |angle| < viewAngle/2:                      # centre inside cone
    dist = max(dist - objectRadius, 0.001)
    return d.normalized * dist                  # distance to surface, direction kept
else:                                            # cone edge clipping
    solve circle/cone-edge intersection (quadratic)
    if no positive root or resulting dist > viewRadius: reject
    angle = sign(angle) * viewAngle/2
    return rotate(up, angle*π/180) * dist
```

Weight per object:

```
w = (1.05 - dist/viewRadius)^2
    * sizeTerm
    * (1.05 - |angle| / (viewAngle/2))
sizeTerm:
  plant/meat pellet : pellet.sizeFactor = max(matter.Amount, 0) / 20        # u²/20
  corpse            : corpse.baseBodyArea / 20 * (1 - corpse.healthRatio)
  bibite            : bibite.maxHealth / 100
```

(`FieldOfView.cs:421-489`; `MatterPellet.cs:67`; corpse term `:452`; bibite term `:467`.)

Accumulation per channel:

```
plantWeightSum = Σ w_p ;  plantVec = Σ (dirToPellet * w_p)      # dirToPellet is the surface-clipped vector (u)
meatWeightSum  = Σ w_m ;  meatVec  = Σ (dirToMeat  * w_m)      # includes corpses
bibiteWeightSum= Σ w_b ;  bibiteVec= Σ (dirToBibite* w_b)
if sum > 0: channelVec /= sum
angle_N   = 2 * SignedAngle(up, channelVec) / viewAngle        # normalised to the FOV half-angle => ±1 inside FOV
closeness = sum>0 ? 1 - min(|channelVec| / viewRadius, 1) : 0
```

(`FieldOfView.cs:490-507`; note `channelVec` is a weighted average of **distance-scaled** vectors, so farther
targets do not dominate the angle in the same way as a unit-vector average, but do contribute to
`closeness`.)

Herd bookkeeping inside the same loop (only used by the `Herding` output, §5): `herdHeadingVector +=
bibite.transform.up`, `centerOfHerd += dir2Target`, `herdSeparationVector += -dir2Target.normalized *
(1 - dist/herdSeparationDistance)` if close, `herdSumDirection = SignedAngle(up, normalized(sep)*sepWeight +
normalized(center)*cohesionWeight + heading*alignmentWeight)` (`FieldOfView.cs:473-520`).

Vision channel enable mask (`StartUsedSystems`, `NEATBrain.cs:533-551`) — evaluated at finalise/resume:

```
mask = 0
herdUsed = herdingEnabled ? (Nodes[35].NIn + (|Nodes[35].baseActivation| > ~0 ? 1 : 0)) : 0
if (herdUsed + Nodes[11].NOut + Nodes[10].NOut + Nodes[9].NOut + Nodes[18].NOut + Nodes[19].NOut + Nodes[20].NOut) > 0: mask |= 1  # bibites
if (Nodes[14].NOut + Nodes[13].NOut + Nodes[12].NOut) > 0: mask |= 2                                                               # plants
if (Nodes[17].NOut + Nodes[16].NOut + Nodes[15].NOut) > 0: mask |= 4                                                               # meat
if (ViewAngle * ViewRadius) <= 0: mask = 0
fow.StartSeeing(mask)
```

`NOut` counts **all** synapses (even disabled). `herdingEnabled = !disableHerding` (default true)
(`NEATBrain.cs:265-277`). Net effect: a channel is computed iff some input of that channel is wired
(or herding output is wired), so unconnected senses are free.

Extra vision details/quirks:

- `fow.viewRadius = gene(ViewRadius) * d1Size` — **body-size scaled**; updated on every growth update
  (`BibiteBody.cs:723`, `:513-517` sets `viewAngle`, herd weights at birth). Effective radius at maturity ≈
  `ViewRadius * SizeRatio * sqrt(growthAtMature)`.
- `fow.viewAngle = gene(ViewAngle)` (degrees, 0..360) (`BibiteBody.cs:513`).
- `viewRadius` used in the weight formula is the radius at the time the entity list was last processed.
- `PelletIsHeld` (`FieldOfView.cs:523-536`) **removes** the checked pellet from the `held` list
  (`held[i] = held[nHeld-1]; nHeld--`) when it is seen — a side effect that forgets held food; reproduce as-is.
- Corpses are visible as bibites (colour gene of the corpse is still readable) and as meat simultaneously.
- `seenThisFrame` is set but not used by the brain.

### 4.4 Clock (`InternalClock`) exact behaviour

`Here/SimulationScripts.BibiteScripts/InternalClock.cs`

```
period = gene[14]                        # ClockSpeed gene, seconds; set in waitReady() once brain.isReady
counting = true (only after brain ready)

fixed_update():                          # every sim tick
    if !counting || timeScale <= 0: return
    dt = fixedDeltaTime
    ticProgress += dt
    if ticProgress >= period:
        ticProgress -= period
        tic = 1 - tic                    # toggles every `period` seconds -> full square wave 2*period
    if brain.Output(ClkReset) > 0.75:    # strict >
        chronoTime = 0
    else:
        chronoTime += dt
    agingProgress += dt
    if agingProgress > 120: agingProgress = 0; OnAging()   # strict >; recomputes metabolism
    timeAlive += dt
```

`tic` is an int saved in the clock JSON; `timeAlive`/`chronoTime` are floats saved too (save bundle only; `.bb8`
export excludes them, and `.bb8template` excludes all runtime state).

### 4.5 Pheromone sensing (`Pherosense`) exact behaviour

`Here/SimulationScripts.BibiteScripts/Pherosense.cs`

- Enabled lazily: `StartPherosensing` only if any phero input has `NOut > 0` (`NEATBrain.cs:535-538`); radius =
  `gene[15]` (`Pherosense.cs:65`). `progress` field is initialised with `SensePeriod(=0.5) * U(-0.5,0.5)`
  (`:66`) but is **never used** in the decompiled method — see Q2.
- Every sim tick (called from `BibiteBody.FixedUpdate` step 7): overlap circle radius `pheroDist`, for each
  `PheromoneSpot` with `position != self`:
  - red: `w = Rstrength / |delta|`; `pheroSum1 += w`; `vec1 += delta*w`; `headingVec1 += spot.heading*w`
  - green/blue analogous into sums 2/3 and vectors 2/3.
- Red Death bloom (if enabled): injects a virtual red source at the arena centre (`:196-207`).
- Angles: `SignedAngle(self.up, vec) / 180` and `SignedAngle(self.up, headingVec) / 180` (`:209-214`).
- Inputs 24/25/26 are `tanh(pheroSum/5)` applied later in `UpdateSenses` (`NEATBrain.cs:601-603`).

Pheromone sensing is the **only** sense updated after the brain tick within the same `FixedUpdate`, so its values
are one sim tick older than vision at the moment `UpdateSenses` reads them.

---

## 5. Outputs (15)

### 5.1 Output table

Index `i` (0..14) maps to node array index `33+i`; `Output(o) = Nodes[33+(int)o].Value`
(`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:312-315`). Activation types are assigned by
`SetBaseNeuronsTypeAndDesc` (`:1029-1056`); defaults are Sigmoid for every output except the ones overridden.

| Out idx | Node idx | Enum | Activation | In-game name | Consumer / effect (thresholds exact) | Refs |
|---:|---:|---|---|---|---|---|
| 0 | 33 | `Accelerate` | TanH | Accelerate | `ForwardForce = clamp(±,−1,1) * movementPenalty * muscleMoveStrength`; negative × `backwardFraction` (0.5); `movementCost += baseAccelCost*abs(v)` | `BibitePropulsion.cs:92,178-183,308-318` |
| 1 | 34 | `Rotate` | TanH | Rotate | `Torque = clamp(±,−1,1) * movementPenalty * muscleTurnStrength`; + = CCW | `:94,180-181,308-317` |
| 2 | 35 | `Herding` | TanH | Herding Behavior | blends `accelerate/rotate` with FOV herd vectors by `abs(h)`: `rot = rot*(1-abs(h))+tanh(herdSumDirection/90)*h`, `acc += gene[20]*herdSeparationProjection*h`; skipped entirely if `disableHerding` | `BibitePropulsion.cs:96,263-271`; `NEATBrain.cs:509-531` |
| 3 | 36 | `EggProduction` | TanH | Egg Production | `remap = ThresholdRemap(x, 0.15)`; dead zone `abs(x) < 0.15`; `Δegg = metabolicRate*remap*dt/LayTime`, clamped to `[-eggProgress, capacity-eggProgress]`; positive costs energy `Δegg*eggEnergy`, negative refunds 50 % | `BibiteEggLayingOrgan.cs:93-157`; `MathfE.cs:14-17` |
| 4 | 37 | `Want2Lay` | Sigmoid | Egg Laying | lays only if `x >= 0.25` (strict: `if eggProgress<1 or want2Lay<0.25 return`) | `BibiteEggLayingOrgan.cs:144-156` |
| 5 | 38 | `Want2Eat` | TanH | Swallow | mouth opening & swallow intent; swallow if `x > 0.15`; negative = vomit; dead zone `abs(x) <= 0.15` | `BibiteMouth.cs:169-209,408-452` |
| 6 | 39 | `Digestion` | Sigmoid | Digestion | `acidLevel = x`; `acidRatio = acid/fullness`; digestion speed = `min(acidRatio,1)*metabolicRate*(1+max(0,acid-fullness))`; energy conversion penalty `(=x-fullness)/2` when over-acid | `BibiteStomach.cs:93,162-219` |
| 7 | 40 | `Grab` | TanH | Grabbing | grab if `x >= 0.15`; throw if `x <= -0.15`; dead zone `abs(x) < 0.15`; force distributed over held objects | `BibiteMouth.cs:165,408-452,787-860` |
| 8 | 41 | `ClkReset` | Sigmoid | Internal Clock Reset | `x > 0.75` → chronoTime = 0 (strict) | `InternalClock.cs:36,76-83` |
| 9 | 42 | `PhereOut1` | ReLU | Red Pheromone Production | 1 Hz production gate: if `R+G+B > 0.05` spawn spot with strengths `x*10`; cost `(R+G+B)*0.01 E` per spawn (per second) | `BibitePheromoneOrgan.cs:63-83`; `ScenarioSettings.cs:773-798` |
| 10 | 43 | `PhereOut2` | ReLU | Green … | same | `:72-74` |
| 11 | 44 | `PhereOut3` | ReLU | Blue … | same | `:72-74` |
| 12 | 45 | `Want2Grow` | Sigmoid | Growth | multiplies growth curve; `Δgrowth = x * growthRate * period * GrowthScale/100` | `BibiteGrowth.cs:91-107` |
| 13 | 46 | `Want2Heal` | Sigmoid | Healing | `heal = min(maxHealth*healMaxRate*x*dt, maxHealth-health)`; cost = heal*healthEnergyRatio/healEfficiency | `BibiteBody.cs:671-685` |
| 14 | 47 | `Want2Attack` | Sigmoid | Bite Strength | attack if `x >= 0.15` (strict `> 0.15` for most uses); damage = bitingDamageFactor * target.maxHealth * biteArea/target.realBodyArea, requires `biteForce > target.armorStrength` | `BibiteMouth.cs:167,183-233,618-720`; `NodeInformations.cs:378-385` |

Note: output biases are mutable by the "default activation" neuron mutation for **all non-input nodes**, including
outputs (`ChangeDefaultAct` uses `Random.Range(NInputs, NInputs+NOutputs+nHidden)`, `NEATBrain.cs:916-935,
1073-1076`). Output *types* never change (only hidden nodes can be type-mutated, `ChangeNeuronType` uses
`GetHiddenIndex`, `:937-961`).

`Herd` and `ThroatWAG`/`MouthMusclesWAG` coupling of the bite geometry is body-subsystem scope (02 spec); the
outputs only feed mouth `desireTo*` scalars (`BibiteMouth.cs:408-410`).

---

## 6. Execution structure (CSR-friendly)

### 6.1 Building the processing structure (exact)

`ComputeActiveNeurons` (`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:467-507`):

```
for i in 0..nNodes:
    Nodes[i].NIn = 0; Nodes[i].NOut = 0
    if Nodes[i].Type == SoftLatch: Nodes[i].baseActivation = clamp(baseActivation, 0.1, 100)
for s in Synapses:
    if s.NodeIn != s.NodeOut:                 # self-loops do NOT count as in/out edges
        Nodes[s.NodeIn].NOut += 1
        Nodes[s.NodeOut].NIn += 1
if any hidden node (index >= 48) with NIn < 1 or NOut < 1:
    CopyBrain(Nodes, Synapses, mutate=false, finalizeRightAway=true, checkForFloatingHidden=true)
    return                                  # CopyBrain->FinalizeEditing re-enters and re-checks
nActiveInputs  = #{i < 33  : Nodes[i].NOut > 0}
nActiveOutputs = #{33<=i<48: Nodes[i].NIn  > 0}
```

`NIn/NOut` include **disabled** synapses. `nActiveInputs/nActiveOutputs` are only used by UI/serialisation, but
`NOut` is used by `StartUsedSystems` to decide which senses to compute (§4.3), so it must be exact.

`ComputeProcessingStructure` (`:509-531`):

```
brainTargets = []; brainLinks = []
for i in 0..nSynaps:
    s = Synapses[i]
    if !s.En: continue
    if !herdingEnabled and s.NodeOut == 2: continue   # decompiled literal, see load-order note below
    slot = first index j with brainTargets[j] == s.NodeOut   # linear "Indexed" search
    if slot == -1: brainTargets.push(s.NodeOut); brainLinks.push([i])
    else:          brainLinks[slot].push(i)
```

Properties that matter to the Rust port:

- `brainTargets` order is "first appearance of the destination while scanning the synapse array", i.e. ordered by
  **lowest synapse index** of each destination. Because evaluation is synchronous (every read is `LastOutput`),
  **the order has no behavioural effect**; it only changes float accumulation order for a node's incoming sum, which
  is a fixed (not sorted) order. Rust must therefore preserve, per node, the **original synapse order** of its
  incoming enabled synapses to match f32 rounding exactly (see §11.3).
- A node with no enabled incoming synapses never executes in `Thinker`.
- **Discrepancy (task-80):** the installed 0.6.3.1 decompile compares `Synapses[i].NodeOut != 2`, not
  `NInputs + 2`; for legal brains (targets are always >= 33) this excludes nothing, so the `Herding`
  output (35) is **not** dropped from `brainTargets`. `brain.rs` currently skips node 35 when
  `herding_enabled` is false (its default is true and load does not set it from `disableHerding`), so
  current parity runs evaluate 35 in both engines; resolve this latent deviation separately.
- `CopyBrain(..., checkForFloatingHidden=true)` is the only self-healing path: it deletes hidden nodes that have no
  incoming or no outgoing synapses (`:412-421`) and re-finalises.

### 6.1a Load-time order (task-80 findings)

The game save (`.bb8`) stores only `brain.Nodes` and `brain.Synapses` arrays (node runtime
`Value`/`LastInput`/`LastOutput` included; **no** plan/targets/order metadata). The load path is:

1. `SaveSystem.LoadBibite` (`Here/ManagementScripts/SaveSystem.cs:467-500`) deserialises the arrays
   verbatim into `NEATBrain.Nodes`/`Synapses`;
2. `BrainUpdater.UpdateBrainFromVersion` (`Here/ScriptHelpers/BrainUpdater.cs:148-162`) runs only for
   saves older than a `VersionChanges` entry (0.6.2.1 and earlier). It inserts/removes/remaps base
   nodes and renumbers node `Index` **in array order**; it never sorts by innovation or graph order.
   A 0.6.3a3 save (the lol fixture) is unaffected;
3. `NEATBrain.ResumeBrain` (`NEATBrain.cs:360-371`) recomputes `NIn/NOut` and
   `ComputeProcessingStructure`; it does **not** re-execute neurons unless a floating hidden node
   triggers `CopyBrain(checkForFloatingHidden: true)` -> `FinalizeEditing` (which re-runs all
   non-input nodes in index order with zero input). The lol round-2 fixture has no floating hidden
   nodes, so saved runtime state is used as-is.

Therefore the deterministic post-load rule is: **preserve the saved node-array and synapse-array
order**; build `brainTargets` by first appearance while scanning synapses, and keep incoming links in
ascending synapse-array order. Sorting by innovation/index/type is wrong. `bibites-io`'s
`records_from_json`/`synapse_records_from_json` and `Brain::from_records` already preserve this
order; `brain::eval_order_tests` pins it with a hidden-node fixture.

Measured evidence that this order is *not* the remaining divergence (lol round-2 fixture):
tick 0 per-entity `brain_digest`/`output_digest`/`sense_digest` are exact **22/22**; per-tick
`behavior.outputs` means are exact **15/15** at ticks 1 and 2 (first brain update), then diverge at
tick 3 (e.g. `Accelerate` 0.3990 -> 0.6842). A wrong load-time plan would already change the first
brain update. The first differing tick coincides with ULP-level pose/sense differences and stateful
nodes, so the next suspects are the first-tick body/sense input path (f32/accumulation or cadence)
and snapshot phase, not hidden-node ordering; hand-off to parity/body owners.


### 6.2 Equivalent Rust evaluation (canonical pseudocode)

```
struct BrainCsr {
    targets: Vec<NodeIdx>,       // node ids, game order (first appearance in synapse array)
    offsets: Vec<u32>,           // len == targets.len()+1, incoming enabled synapses per target
    syn_src: Vec<SynIdx>,        // synapse ids grouped by target, ascending synapse index
}

fn tick(b: &mut Brain, period: f32, senses: &Senses) {
    // 1. swap history: prev = last tick's outputs (game's LastOutput before UpdateSenses)
    std::mem::swap(&mut b.prev_out, &mut b.cur_out);
    b.prev_in.copy_from_slice(&b.cur_in);          // not strictly needed for inputs, needed for stateful nodes

    // 2. UpdateSenses: overwrite the 33 input values (cur_out[0..33]) with fresh samples
    sample_senses(b, senses);                       // §4.1; writes cur_out[0..33]

    // 3. non-input nodes not executed keep their previous output
    for j in 33..b.n_nodes { b.cur_out[j] = b.prev_out[j]; b.cur_in[j] = b.prev_in[j]; }

    // 4. execute each target exactly once, reading ONLY prev_* (synchronous semantics)
    for g in 0..b.csr.targets.len() {
        let t = b.csr.targets[g];
        let ty = b.node_type[t];
        let mut acc = if ty == Mult { 1.0f32 } else { 0.0f32 };
        for s in b.csr.offsets[g]..b.csr.offsets[g+1] {
            let term = b.weight[s] * b.prev_out[b.node_in[s]];
            acc = if ty == Mult { acc * term } else { acc + term };
        }
        let (out, last_in) = activate(ty, acc, b.bias[t], b.prev_out[t], b.prev_in[t], period);
        b.cur_out[t] = out;
        b.cur_in[t]  = last_in;
    }
}
```

`activate` is the exact `ExecuteNeuron` switch of §2.1 (including the bias rules and the [-1024, 1024] clamp).
`period` = `brain_period_secs(sim_tps, brain_ticks)` from the **effective** settings (§3.1, §10.6).

Parallelism: one brain is a fully independent unit vectorised over the CSR rows; brains are independent of each
other within a brain tick, so the brain pass is trivially data-parallel per bibite (each brain reads/writes only its
own arrays; sense snapshots are per-bibite).

### 6.3 Validation invariants for tests

- `NInputs + NOutputs + nHidden == Nodes.len()` and every hidden node has `NIn>=1 && NOut>=1` after finalise/load
  repair.
- `Nodes[i].Index == i` (re-established by `FinalizeEditing`/`BrainUpdater`, but **not** re-checked on plain load;
  Rust should normalise).
- Synapses sorted by `Inov` ascending; node `Inov` ascending (base = index+1).
- Every `NodeIn/NodeOut` in range; self-loops possible; duplicate `(NodeIn,NodeOut)` pairs are not created by
  mutation (`AddSynapse` merges, `:704-709`) but may exist in dirty files — `CopySynaps` merges duplicates by adding
  weights (`:1011-1022`).
- `nActiveInputs<=33`, `nActiveOutputs<=15`.
- Executed-once-per-tick property; reading order must be independent (assert with an order-shuffle test that output
  vector is unchanged).

---

## 7. Genome — the 35 genes

### 7.1 Gene array

`BibiteGenes.Genes` is an enum of 35 entries, indices **0..34, used directly as array indices**
(`Here/SimulationScripts.BibiteScripts/BibiteGenes.cs:16-53`, `NGene = 35` at `:117`). The genome is a
`float[35]`. Gene definitions/limits/defaults live in `BibiteEditorSettings.geneSettings`
(`Here/SettingScripts/BibiteEditorSettings.cs:11-572`); `geneSettingsOrdered` is sorted by enum index
(`:574`). The table below is generated from those definitions; "span" = `maxValue-minValue`
(`Here/SettingScripts/GeneSetting.cs:13`), which is the `geneSpan` used by the mutation kernel.

| Idx | Enum | In-game name | Min | Max | Default | Units | isPurelyRelative | Span | Key couplings (consumer refs) |
|---:|---|---|---:|---:|---:|---|---|---:|---|
| 0 | `LayTime` | Lay Time | 1 | 51 | 15 | s | no | 50 | egg period at full production `BibiteEggLayingOrgan.cs:108,131` |
| 1 | `BroodTime` | Brood Time | 1 | 101 | 20 | s | no | 100 | `growthAtBirth=(Hatch/Brood)^2` `BibiteGenes.cs:163-166` |
| 2 | `HatchTime` | Hatch Time | 1 | 101 | 10 | s | no | 100 | egg hatch duration `EggHatching.cs:92`, birth growth |
| 3 | `SizeRatio` | Size Ratio | 0.05 | 2.55 | 1 | — | **yes** | 2.5 | `d2Size=growth*SizeRatio²`, `d1Size=√d2Size`, body area, energy density, view radius, brain-cost scaling `BibiteGenes.cs:129-135`, `BibiteBody.cs:699-726` |
| 4 | `SpeedRatio` | Metabolism Speed | 1e-6 | 2.5 | 0.75 | SPD | **yes** | ≈2.5 | `metabolicRate`, `MetabolicEfficiency=0.8^SpeedRatio` `BibiteGenes.cs:135-137` |
| 5 | `ColorR` | Red Color | 0 | 1 | 0.1 | — | no | 1 | skin colour, `RedBibite` input `FieldOfView.cs:484` |
| 6 | `ColorG` | Green Color | 0 | 1 | 0.1 | — | no | 1 | `GreenBibite` |
| 7 | `ColorB` | Blue Color | 0 | 1 | 0.1 | — | no | 1 | `BlueBibite` |
| 8 | `MutationAmountSigma` | Gene Mutation Variance | 0 | 0.25 | 0.05 | ratio (frac. of span) | no | 0.25 | gene mutation σ base `BibiteGenes.cs:404` |
| 9 | `AverageMutationNumber` | Average Gene Mutations | 0 | 30 | 2 | count | no | 30 | gene mutation Poisson λ `BibiteGenes.cs:388` |
| 10 | `BrainMutationSigma` | Brain Mutation Variance | 0 | 0.25 | 0.1 | ratio | no | 0.25 | brain mutation σ base `NEATBrain.cs:379` |
| 11 | `BrainAverageMutation` | Average Brain Mutations | 0 | 10 | 1 | count | no | 10 | brain mutation Poisson λ `NEATBrain.cs:378,399` |
| 12 | `ViewAngle` | View Angle | 0 | 360 | 75 | ° | no | 360 | `fow.viewAngle` `BibiteBody.cs:513`; body energy density `BibiteGenes.cs:121` |
| 13 | `ViewRadius` | View Radius | 0 | 200 | 180 | u | no | 200 | `fow.viewRadius=gene*d1Size` `BibiteBody.cs:723`; energy density |
| 14 | `ClockSpeed` | Clock Period | 0.1 | 10.1 | 1 | s | no | 10 | `InternalClock.period` `InternalClock.cs:105` |
| 15 | `PheroSense` | Pheromones Sensing Radius | 0 | 500 | 150 | u | no | 500 | `pheroDist` `Pherosense.cs:65` |
| 16 | `Diet` | Diet | 0 | 1 | 0.2 | — | no | 1 | digestion efficiency, grab strength `BibiteStomach.cs:146`, `BibiteMouth.cs:430` |
| 17 | `HerdSeparationWeight` | Herd Separation Weight | 0 | 4 | 1 | — | no | 4 | `fow.separationWeight` `BibiteBody.cs:515` |
| 18 | `HerdAlignmentWeight` | Herd Alignment Weight | 0 | 4 | 1.5 | — | no | 4 | `fow.alignmentWeight` `:517` |
| 19 | `HerdCohesionWeight` | Herd Cohesion Weight | 0 | 4 | 0.75 | — | no | 4 | `fow.cohesionWeight` `:516` |
| 20 | `HerdVelocityWeight` | Herd Velocity Weight | 0 | 1 | 0.25 | — | no | 1 | **used as the separation weight inside the herding blend** `BibitePropulsion.cs:270` (reads `genes[20]`) |
| 21 | `HerdSeparationDistance` | Herd Separation Distance | 0 | 100 | 35 | u | no | 100 | `fow.herdSeparationDistance=gene*d1Size` `BibiteBody.cs:514,722` |
| 22 | `GrowthScale` | Growth Scale Factor | 0 | 2 | 0.15 | — | **yes** | 2 | `Δgrowth *= GrowthScale/100` `BibiteGrowth.cs:101` |
| 23 | `GrowthMaturityFactor` | Growth Maturity Factor | 0 | 10 | 2 | — | no | 10 | growth curve denominator `:116` |
| 24 | `GrowthMaturityExponent` | Growth Maturity Exponent | 0 | 10 | 1 | — | no | 10 | growth curve exponent `:116` |
| 25 | `EyeOffset` | Eye Hue Offset | 0 | 1 | 0.5 | — | no | 1 | eye hue `BibiteGenes.cs:465-475` |
| 26 | `StomachWAG` | Stomach WAG | −1 | 9 | 2 | — | no | 10 | stomach area `BibiteSpatialOrgan.cs:25,33-36` |
| 27 | `WombWAG` | Egg Organ WAG | −1 | 9 | 1 | — | no | 10 | womb area, `growthAtMature` `BibiteGenes.cs:168-171` |
| 28 | `FatWAG` | Fat Organ WAG | −1 | 9 | 1 | — | no | 10 | fat area/capacity |
| 29 | `ArmorWAG` | Armor WAG | −1 | 9 | 0.15 | — | no | 10 | armor area/strength, body energy density `BibiteGenes.cs:121` |
| 30 | `ThroatWAG` | Throat WAG | −1 | 9 | 1 | — | no | 10 | throat area / mouth width |
| 31 | `MouthMusclesWAG` | Jaw Muscles WAG | −1 | 9 | 1 | — | no | 10 | jaw area / bite force |
| 32 | `MoveMusclesWAG` | Arm Muscles WAG | −1 | 9 | 1 | — | no | 10 | move-muscle area / propulsion strength |
| 33 | `FatStorageThreshold` | Fat Storage Threshold | 0 | 1 | 0.5 | — | no | 1 | fat store/burn neutral point |
| 34 | `FatStorageDeadband` | Fat Storage Deadband | 0 | 0.5 | 0.25 | — | no | 0.5 | hysteresis around the threshold |

Notes:
- **There is no per-gene mutation σ.** Every gene uses the same instantaneous σ
  `genes[8] + backgroundMutationVariance + mutationIncreases.intensityIncrease` and the same count
  λ `genes[9] + backgroundMutationChance + mutationIncreases.probabilityIncrease` (§8.3). Per-gene behaviour differs
  only through `span = max-min`, `[min,max]` clamping and `isPurelyRelative` (100 % multiplicative share). Brain
  weight/bias mutations use the brain σ/λ (`genes[10]`/`genes[11]`) with `geneSpan = 1` and are otherwise the same
  kernel (§8.4-§8.6).
- `isPurelyRelative` only affects the mutation kernel (§8.2): for those genes the relative (multiplicative) share
  is forced to 100 %.
- `ColorR/G/B` and `LayTime`…`FatStorageDeadband` are all mutable by the uniform gene index pick (§8.2), including
  the four mutation genes themselves (evolvable evolvability).
- A loaded value is always clamped through `CapGenes()` (`Here/SimulationScripts/BibiteGenes.cs:409-416`,
  called by `StartGenesFromTemplate`, `CopyGene`, `LoadState`); the Clamp range is the min/max above.

### 7.2 Derived gene expressions used by the brain/body interface

`Here/SimulationScripts.BibiteScripts/BibiteGenes.cs`, unless noted; all are f32:

```
growthAtBirth      = (HatchTime / BroodTime)^2                                  :163-166
growthAtMature     = max(1, growthAtBirth / max(wombAreaPortion, 0.001))       :168-171
wombAreaPortion    = max(0, WombWAG) / totalOrganWAG                            :183-186,218-226
totalOrganWAG      = Σ_{g in 26..32} max(0, gene[g])                            :223-226
D2SizeAtBirth      = SizeRatio² * growthAtBirth                                 :131
areaAtBirth        = 80 * D2SizeAtBirth                                         :133
metabolicRate      = SpeedRatio                                                 :135
MetabolicEfficiency= energyUsageEfficiency(0.8) ^ SpeedRatio                    :137
BodyEnergyDensity  = 2.0 + ViewAngle*0.003 + ViewRadius*0.002
                     + armorAreaPortion * 1.5                                   :121 (defaults from ScenarioSettings)
SumBodyAndHealthEnergyRatio = BodyEnergyDensity * (1 + 0.5)                     :119-123
LayBodyEnergy      = 100 * D2SizeAtBirth * (1.5 + SumBodyAndHealthEnergyRatio)  :125
```

Body growth/size (couplings that the brain sees through inputs):

```
growth (gene-relative scalar) starts at growthAtBirth and grows toward growthAtMature
d2Size    = growth * SizeRatio²                                                  BibiteBody.cs:128, BibiteGrowth.cs:128
d1Size    = sqrt(d2Size)                                                         BibiteBody.cs:705
bodyLength= 12.5 * d1Size                                                        BibiteBody.cs:296
baseBodyArea = 80 * d2Size                                                       BibiteBody.cs:298
maxHealth = 1.25 * baseBodyArea                                                  BibiteBody.cs:719
Energy.MaxAmount = 1.5 * baseBodyArea                                            BibiteBody.cs:720
```

Growth update (relevant because `Maturity` input is raw `growth`):

```
growthRate         = metabolicRate / (1 + GrowthMaturityFactor * maturity^GrowthMaturityExponent)   BibiteGrowth.cs:114-117
maturity           = growth / growthAtMature                                                        :52
growthUpdatePeriod = 0.2 / growthRate       (initialised 0.2; updated after each growth step)        :44,104
Δgrowth            = Want2Grow * growthRate * growthUpdatePeriod * GrowthScale / 100                 :96-103
```

Brain upkeep coupling: `eggEnergy = LayBodyEnergy + brainUpkeepCost * SizeRatio` (egg needs to pay for the child's
brain upkeep) `Here/SimulationScripts.BibiteScripts/BibiteEggLayingOrgan.cs:107`.

---

## 8. Mutation

### 8.1 Scenario mutation settings (all scenario-scope, defaults)

`Here/SettingScripts/ScenarioSettings.cs`

| Setting | Field / line | Default | Meaning |
|---|---|---:|---|
| Background Mutation Chance | `backgroundMutationChance` `:13-24` | 1.5 | added to the Poisson λ of **both** genome and brain mutations |
| Background Mutation Variance | `backgroundMutationVariance` `:26-39` | 0.05 | added to σ of both |
| Mutation Value Relativity | `relativeMutationShare` `:41-54` | 0.75 | fraction of σ applied multiplicatively (`isPurelyRelative` genes use 1.0) |
| Synapse Mutation Probability | `synapseMutationChance` `:56-69` | 0.60 | branch: brain mutation is a synapse mutation vs neuron |
| Neuron Mutation Probability | `neuronMutationChance` `:71-84` | 0.40 | *not used by the brain mutation branch* (0.6.3.1 code uses `1 - synapseMutationChance` implicitly); kept for the settings UI/docs only |
| Synapse Strength Mutation Probability | `synapseChangeChance` `:86-99` | 0.75 | of synapse mutations |
| Synapse Flip Probability | `synapseFlipChance` `:101-114` | 0.025 | negate weight |
| Synapse Toggle Probability | `synapseToggleChance` `:116-129` | 0.025 | enable/disable |
| Synapse Add Probability | `synapseAddChance` `:131-144` | 0.10 | add connection |
| Synapse Removal Probability | `synapseRemoveChance` `:146-159` | 0.10 | remove a disabled connection |
| Default Activation Mutation Probability | `neuronDefaultChance` `:161-174` | 0.70 | mutate a bias |
| Neuron Function Mutation Probability | `neuronChangeChance` `:176-189` | 0.10 | change hidden node type |
| Neuron Add Probability | `neuronAddChance` `:191-204` | 0.10 | split a synapse with a new hidden node |
| Neuron Removal Probability | `neuronRemoveChance` `:206-219` | 0.10 | remove a hidden node |
| Prevent Mutations | `preventMutations` `:286-290` | false | global off switch |

Runtime mutation modifiers (`BibiteGenes.mutationIncreases`, struct `MutationIncreases`:
`Here/SimulationScripts.BibiteScripts/MutationIncreases.cs:3-8`):

- `AddMutationFactor(factors, source)` / `IncreaseMutationFactor(source, count, intensity)` /
  `RemoveMutationFactor(source)` maintain `Dictionary<string, MutationIncreases> mutationFactors` and recompute
  `mutationIncreases.probabilityIncrease = Σ count`, `mutationIncreases.intensityIncrease = Σ intensity`
  (`Here/SimulationScripts.BibiteScripts/BibiteGenes.cs:248-252,501-531`). Rad towers / events call
  `IncreaseMutationFactor("Radioactivity", …)` etc. (world scope). These are **accumulated on the live bibite**, are
  not genes, and are **not persisted** (`mutationFactors`/`mutationIncreases` are neither ISaveable nor
  `[SerializeField]`) — a save/load loses them.
- Effective mutation rate/variance at reproduction:
  `λ_gene = genes[9] + backgroundMutationChance + mutationIncreases.probabilityIncrease`;
  `σ_gene = genes[8] + backgroundMutationVariance + mutationIncreases.intensityIncrease`;
  `λ_brain = genes[11] + backgroundMutationChance + mutationIncreases.probabilityIncrease`;
  `σ_brain = genes[10] + backgroundMutationVariance + mutationIncreases.intensityIncrease`.
  `preventMutations` suppresses mutation entirely (both paths).

### 8.2 Mutation kernel (exact formulas)

`Here/ScriptHelpers/RandomExtension.cs`:

```
NextGaussian(r, mu=0, sig=0):
    u1 = 1 - r.NextDouble(); u2 = 1 - r.NextDouble()          // both in (0,1]
    g  = sqrt(-2*ln(u1)) * sin(2π*u2)
    return mu + sig*g

NextRelativeGaussianMutation(r, geneValue=1, mutationSigma=0):
    g = NextGaussian(r, 0, 1)
    factor = (1 + mutationSigma)^g
    return geneValue * (factor - 1)

NextMutationValue(r, geneValue, mutationSigma, geneSpan, purelyRelative=false):
    share = purelyRelative ? 1.0 : relativeMutationShare(0.75)
    rel   = NextRelativeGaussianMutation(r, geneValue, mutationSigma*share)
    abs   = NextGaussian(r, 0, mutationSigma*(1-share)) * geneSpan
    return rel + abs
```

This is **one** Gaussian draw pair per relative part and one per absolute part, i.e. 4 `NextDouble()` calls per
`NextMutationValue` (2 per Gaussian), consumed in the order: relative `NextGaussian` (u1,u2), then absolute
`NextGaussian` (u1,u2).

```
NextPoisson(r, lambda):
    L = exp(-lambda); p = 1; k = 0
    do { k++; p *= r.NextDouble() } while (p > L)
    return k-1
```
One `NextDouble()` per loop iteration (at least one), so the RNG consumption depends on the draw.

### 8.3 Genome mutation (per offspring)

`Here/SimulationScripts.BibiteScripts/BibiteGenes.cs:364-407`

```
mutate(numberOfMutation = null):
    if preventMutations: return
    # easter-egg flag (UserSettings.AllowEasterEggs default false, period default 500000)
    if AllowEasterEggs and rng_unity.value() < 1/EasterEggsProbability:
        switch rng_unity.Range(0,4): case0 isRad; case1 isRain; case2 isSource; case3 isKiller
    n = numberOfMutation ?? rng_sys.NextPoisson(genes[9] + backgroundMutationChance + mutationIncreases.probabilityIncrease)
    for i in 0..n: one_mutation()

one_mutation():
    gene = rng_unity.Range(0, 35)                       # uniform over ALL genes incl. mutation genes
    sigma = genes[8] + backgroundMutationVariance + mutationIncreases.intensityIncrease
    span  = BibiteEditorSettings.SettingOfGene(gene).span
    purelyRel = SettingOfGene(gene).isPurelyRelative
    delta = rng_sys.NextMutationValue(genes[gene], sigma, span, purelyRel)
    genes[gene] = clamp(genes[gene] + delta, min, max)
```

RNG streams are **mixed**: the mutation *count* and the mutation *deltas* come from the C#
`System.Random` instance (`BibiteGenes.SystemRandom`, `:91`), while gene selection and the easter-egg roll come from
UnityEngine.Random (`Random.Range/Random.value`). This split is impossible to reproduce exactly across runtimes; see
§11.2 for the Rust replacement policy.

### 8.4 Brain mutation entry point

`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:373-426` — called on a copy of the parent brain:

```
copy_brain(parentNodes, parentSynapses, mutate):
    nodese=[], synapse=[]
    nHidden = parentNodes.Length - 48
    mutationProbability = genes[11]                              # BrainAverageMutation
    mutationVariance    = genes[10] + backgroundMutationVariance + mutationIncreases.intensityIncrease
    for node in parentNodes: copy (Type, Inov, Desc, Index, NIn, NOut, baseActivation)   # state zeroed
    for syn in parentSynapses: merge-copy (same NodeIn/NodeOut -> add weights, keep existing En)
    nSynaps = len(synapse)
    if mutate and !preventMutations:
        disabledSynapses = indices of synapses with En == false
        n = rng_sys.NextPoisson(mutationProbability + backgroundMutationChance
                                + mutationIncreases.probabilityIncrease)
        for k in 0..n:
            if rng_unity.value() < synapseMutationChance: mutate_synapses()
            else:                                         mutate_neurons()
    if checkForFloatingHidden:  remove hidden nodes with NIn<1 or NOut<1
    if finalizeRightAway:       finalize_editing()                # §3.4
```

`mutationProbability` is the *gene* value (λ), not a probability; the field name is historical.

### 8.5 Synapse mutations

`MutateSynapses` (`:643-678`) — rolling-uniform branch in this exact order; all rolls are
`UnityEngine.Random.Range(0f,1f)` (i.e. one uniform draw), thresholds are cumulative and **not re-normalised**
(defaults sum to exactly 1.0):

```
u = rng_unity.uniform01()
if u < synapseAddChance:                       add_synapse(); return
u -= synapseAddChance
if u < synapseChangeChance:                    change_synapse_strength(); return
u -= synapseChangeChance
if u < synapseFlipChance:                      flip_synapse(); return
u -= synapseFlipChance
if u < synapseToggleChance:                    toggle_synapse_enabled(); return
u -= synapseToggleChance
if u < synapseRemoveChance:                    remove_disabled_synapse()
# else: nothing
```

Operations (each consumes further Unity RNG draws):

| Op | Behaviour | RNG | Ref |
|---|---|---|---|
| `AddSynapse(nodeIn=null,nodeOut=null,strength=null)` | if unset: `nodeIn = Random.Range(0, 33+nHidden) < 33 ? Random.Range(0, 32) : NInputs+NOutputs+Random.Range(0,nHidden)` (note: input roll excludes input index 32 — off-by-one quirk), `nodeOut = Random.Range(33, 48+nHidden)`; if herding disabled and nodeOut==35 → recurse re-rolling nodeOut only; if strength unset `rng_sys.NextGaussian(0,0.5)`; if a synapse with same (in,out) exists → `ChangeSynapseStrength(idx, existing.Weight + strength)` and return −1; else append with `Inov=connectionMaxInov++` | Unity×2 or ×3, System×2 (gaussian) | `:680-719` |
| `ChangeSynapseStrength(idx=null,strength=null)` | if `nSynaps==0` add first; `idx ?? Random.Range(0,nSynaps)`; `Weight = strength ?? Weight + rng_sys.NextMutationValue(Weight, mutationVariance, 1.0)` (span 1, purelyRelative=false) | Unity maybe, System 4 doubles | `:721-732` |
| `FlipSynapse` | `idx ?? Random.Range(0,nSynaps)`; `Weight *= -1` | Unity maybe | `:734-745` |
| `ToggleSynapseEnabled` | `idx ?? Random.Range(0,nSynaps)`; toggle `En`; maintain `disabledSynapses` | Unity maybe | `:747-767` |
| `RemoveDisabledSynapse` | if any disabled: `Random.Range(0,disabledSynapses.Count)` → `RemoveSynapse(that)`; else `ToggleSynapseEnabled()` (disables a random synapse, does not remove) | Unity 1 | `:769-780` |
| `RemoveSynapse(sel)` | remove from lists, decrement NIn/NOut, cascade-remove hidden nodes that become floating; compact; rebuild `disabledSynapses` | Unity 1 if sel null | `:782-813`, `:963-1004` |

`AddSynapse` can create self-loops (hidden→itself or hidden→output that is the same index when nodeIn is a hidden and
nodeOut random covers hidden range; inputs cannot be targets). It cannot target an input.

### 8.6 Neuron mutations

`MutateNeurons` (`:815-844`) — same cumulative pattern; note the decompiled code subtracts `neuronRemoveChance`
*before* the removal test (a harmless quirk when the four defaults sum to 1.0; with custom settings it changes the
effective partition — reproduce as written):

```
u = rng_unity.uniform01()
if u < neuronDefaultChance:                     change_default_act(); return
u -= neuronDefaultChance
if u < neuronChangeChance:                      change_neuron_type(); return
u -= neuronChangeChance
if u < neuronAddChance:                         add_neuron();  return
u -= neuronRemoveChance                          # <-- as decompiled (:835)
if u < neuronRemoveChance:                      remove_neuron()
# else nothing
```

| Op | Behaviour | RNG | Ref |
|---|---|---|---|
| `ChangeDefaultAct(target=null,newValue=null)` | `target ?? Random.Range(33, 48+nHidden)`; if type==Integrator → **recurse** (retry, consumes another roll); `baseActivation = newValue ?? baseActivation + rng_sys.NextMutationValue(baseActivation, mutationVariance, 1.0, purelyRelative = (type==Inhibitory))`; then clamp Inhibitory [1e-4,10] or SoftLatch [0.1,100] | Unity 1, System 4 doubles | `:916-935` |
| `ChangeNeuronType(target=null,newType=null)` | if `nHidden<1` → `AddNeuron()`; target = hidden node (explicit or random); `Type = newType ?? Random.Range(1,14)`; fix bias: Inhibitory/Mult → clamp(abs(b),0.1,1); Integrator → 0; SoftLatch → clamp(b,0.1,100) | Unity ≤2 | `:937-961` |
| `AddNeuron(fromSynaps=null,type=null,desc="")` | if no synapses → add one; pick random synapse (Unity); type `Random.Range(1,14)`; `nHidden++`; default bias: Inhibitory `10^Random.Range(-2,1)`, Mult 1, SoftLatch `10^Random.Range(-1,1)`, else 0; new node `Inov=++nodeMaxInov`, `Index=NInputs+NOutputs+(nHidden-1)`, NIn=NOut=1, Desc="Hidden{nHidden-1}"; disable the split synapse; add `NodeIn->new` and `new->NodeOut`; `flag = Random.value >= 0.5`; weights: `flag ? (split.Weight, 1.0) : (1.0, split.Weight)`; Inov counters `connectionMaxInov++` twice | Unity 3–4, node counter | `:877-914` |
| `RemoveNeuron(sel=null,fromSynapseRemoval=false)` | ignores inputs/outputs (`sel < 48` returns; `nHidden<=0` returns); random hidden; marks removal; removes all synapses touching it (recursively cleans floating nodes); compact + reindex | Unity 1 if sel null | `:846-875`, `:963-1004` |

Important semantics:
- `AddNeuron` **splits** an existing synapse (sets it `En=false` and inserts the new node in a 2-synapse path),
  preserving the total gain: gain is split between `weight` and `1.0` depending on `flag`.
- new hidden nodes default bias `0` except the three special types above; `DefaultBaseActivationOfType`
  (`:1083-1092`) is the UI/editor counterpart (Mult 1, Inhibitory 1, SoftLatch 5, else 0) and is *not* used by the
  mutation path (which rolls random biases instead).
- `AddNeuron` appends the node at the end of `nodese`; the two new synapses append to `synapse`, so Inov ordering is
  preserved.
- `RemoveNeuron`/`RemoveSynapse` compact arrays and renumber `NodeIn/NodeOut` consistently; invariants of §6.3 hold
  after `FinalizeEditing`.

### 8.7 Where mutation is invoked (reproduction order)

1. **Egg laying** (`Here/SimulationScripts.BibiteScripts/BibiteEggLayingOrgan.cs:159-191`):
   `CopyGene(parentGenes, increaseGeneration:true, mutate:true)` → `SetParents(parent)` →
   `CopyBrain(parent.Nodes, parent.Synapses, mutate:true)` → `StartHatch(eggEnergy)`.
   - `CopyGene(increaseGeneration:true)`: generation = parent+1, copies `mutationIncreases`, inherits the four
     easter-egg flags each with probability 0.1, then `Mutate()` (`BibiteGenes.cs:287-334`).
   - Gene mutation happens **before** brain mutation; brain mutation reads the *new* genes for λ/σ (already mutated
     if mutation genes changed in the same event).
2. **Egg hatching** (`Here/SimulationScripts.BibiteScripts/EggHatching.cs:175-207`): `CopyGene(eggGene)` (no
   mutation), `CopyBrain(eggBrain.Nodes, eggBrain.Synapses, mutate:false)`, `StartBody()`.
3. **Scenario spawn from template** (`Here/ManagementScripts/WorldObjectsSpawner.cs:160-190`): genes from template
   (optionally `Mutate()` or `RandomGenes()`), brain copied without mutation.
4. **Egg spawned from template** (`WorldObjectsSpawner.cs:192-224`): genes from template, `Mutate((int)genes[9])`
   forced exactly `floor(AverageMutationNumber)` mutations (not Poisson!) then brain copied **with** mutation.
5. **Editor / world tools** can call the public mutation functions directly (`AddSynapse`, `AddNeuron`,
   `ChangeNeuronType`, … `:643-961`) — the same operations are the editor API.

### 8.8 RNG call-order summary (brain mutation of one offspring)

| Step | Stream | Draws |
|---|---|---|
| brain λ | System.Random | `NextPoisson` (≥1 `NextDouble`) |
| per mutation: synapse/neuron branch | Unity | 1 (`Random.value`) |
| synapse branch | Unity | 1 (`Range(0,1)`), then op draws |
| neuron branch | Unity | 1 (`Range(0,1)`), then op draws |
| weight/bias deltas | System.Random | 4 `NextDouble` per `NextMutationValue` (2 Gaussians) |
| random index picks | Unity | 1 per `Random.Range` |
| add synapse weight | System.Random | 2 `NextDouble` (`NextGaussian`) |
| floating-hidden cleanup, `FinalizeEditing` | none | none |

UnityEngine.Random in this Unity version is a deterministic xorshift128-based generator, but its exact sequence is an
engine implementation detail; .NET `System.Random` is also implementation-defined. Byte-exact reproduction of the
original game's RNG *streams* is therefore not achievable with `xoshiro`; the project's determinism contract
(ARCHITECTURE §5) instead requires *our* runs to be reproducible from `(seed, scenario, settings)`, with statistical
equivalence to the game (§11.2).

---

## 9. Speciation

Speciation is **bookkeeping only** — it never feeds back into behaviour. It is driven by a combined
gene + brain distance to a species' *template* (a snapshot genome/brain of the species founder).

### 9.1 Distance formulas

`Species.GeneticDistanceToIndividual = GeneDistance + BrainDistance`
(`Here/SimulationScripts.BibiteScripts/Species.cs:155-158`).

**Gene distance** `Here/SimulationScripts.BibiteScripts/Species.cs:288-297`:

```
geneDistance = Σ_{i=0..34} |template.genes[i] - individual.genes[i]| / geneSettingOrdered[i].maxValue
```

Note the normaliser is the **max** of the editor range (not the span), so genes whose max is small (e.g.
`SpeedRatio` max 2.5 vs `LayTime` max 51) dominate the metric. `geneSettingsOrdered` is sorted by enum index so
`[i]` really is gene `i` (`Here/SettingScripts/BibiteEditorSettings.cs:574`).

**Brain distance** `Here/SimulationScripts/BibiteScripts/Species.cs:160-286`,
`BrainDifferences.totalDistance` `Here/SimulationScripts.BibiteScripts/BrainDifferences.cs:27`:

```
nMaxConnections = max(template.synapses.len, individual.synapses.len)

# linear merge by Inov (arrays MUST be sorted ascending by Inov)
common=0; disjoint=0; toggled=0; connDeltas=[]; actDeltas=[]; biasedCommon=0
i=0; j=0
while i < A.len and j < B.len:
    if A[i].Inov == B[j].Inov:
        common += 1
        dw = A[i].Weight - B[j].Weight
        if dw != 0: connDeltas.push(|dw / (|A[i].Weight| + 1)|)         # relative delta, signed-denominator abs
        if A[i].En != B[j].En: toggled += 1
        i++; j++
    elif A[i].Inov > B[j].Inov: disjoint += 1; j++                       # only in B
    else:                       disjoint += 1; i++                       # only in A
disjoint += A.len - i + B.len - j

# node bias comparison, same merge on node Inov
while i < A.nodes.len and j < B.nodes.len:
    if A[i].Inov == B[j].Inov:
        if |A[i].baseActivation| > 0 or |B[j].baseActivation| > 0:
            biasedCommon += 1
            db = A[i].baseActivation - B[j].baseActivation
            if db != 0: actDeltas.push(|db|)
    elif A[i].Inov > B[j].Inov: j++
    else: i++

for d in connDeltas: d.dist /= sqrt(max(common,1))
for d in actDeltas:  d.dist /= max(biasedCommon,1)

brainDistance = disjoint / max(nMaxConnections,1)
              + toggled  / max(common,1)
              + Σ connDeltas.dist
              + Σ actDeltas.dist
```

Exact details worth preserving:
- weight delta uses **relative** distance `|Δw| / (|w_template| + 1)` (asymmetric: normalised by the species
  template weight).
- connection-delta terms and bias-delta terms are additionally divided by `sqrt(max(common,1))` and
  `max(biasedCommon,1)` respectively (note: the connection divisor is `sqrt(nCommon)` while the toggled divisor is
  `nCommon`).
- bias deltas only count nodes where **at least one** of the two biases is non-zero; both zero biases are ignored.
- Disjoint counts are computed as a set-ish merge; `nDisjointConnections` is used raw in the numerator.
- The merge assumes Inov-sorted arrays; unsorted arrays silently produce wrong distances (see §6.3).

### 9.2 Species span

`Here/ManagementScripts/GlobalLineageManager.cs:44,73-79`

```
usedSpeciesSpan = speciesGeneticSpan + max(activeSpeciesCount - 20, 0) * speciesSpanScalingFactor
```

Defaults: `speciesGeneticSpan = 1.5`, `speciesSpanScalingFactor = 0.05`
(`Here/SettingScripts/ScenarioIndependentSettings.cs:126-154`). Real scenarios override the span (e.g. 0.4 in the
0.6.3.1 save used for fixtures). `activeSpecies` counts species with `count >= 1`; recomputed when settings change,
species pruned/created, etc.

### 9.3 Assignment at birth (`GlobalLineageManager.CheckNewSpecies`)

Called from `BibiteBody.StartBody()` (`Here/SimulationScripts.BibiteScripts/BibiteBody.cs:435`) and
`ResumeBody()` (`:479`). Exact algorithm (`Here/ManagementScripts/GlobalLineageManager.cs:194-249`):

```
species = bibite.gene.species                         # inherited reference from parent's genome object
dist(genome, brain) = GeneticDistanceToIndividual(...)
if species == null:
    for s in activeSpecies (in activeSpecies list order):
        if dist(s) <= usedSpeciesSpan: gene.species = s; return
    gene.species = create_new_species(bibite, parent=null); return

if dist(species) < usedSpeciesSpan: return            # stays in inherited species

# out of span: try to join an active child species within half the span
half = usedSpeciesSpan * 0.5
candidate = null
for child in species.children:
    if child.count >= 1 and dist(child) <= half: candidate = child     # last match wins (no break)
if candidate != null: gene.species = candidate; return

# two-step mutant mechanism
if !gene.isMutant: gene.isMutant = true; return       # first outlier: just flagged
gene.isMutant = false                                  # second consecutive outlier: found a species
founder = gene.parent1 != null ? parent1.body : self
species.remove_from_species(founder)
newSpecies = create_new_species(founder, parent=species)   # template snapshot of the *parent* individual
founder.gene.species = newSpecies
if !bibite.dead:
    gene.species = newSpecies
    if !founder.dead: newSpecies.add_to_species(founder)
    founder.gene.isMutant = false
```

Notes:
- `dist` is recomputed many times (O(species × genome) at every birth) — significant CPU; Rust should cache the
  inherited-species distance (but keep the *semantics*, including that the child list iteration order is the
  insertion order of `children`).
- `isMutant` is persisted (see genes JSON `isMutant`), so it survives save/load. It is reset to false when the
  species is founded. It is *not* a mutation flag in the genetic sense; only the speciation flag.
- The new species template is built from the **parent** (`gene.parent1` if set, else the current bibite), so the
  parent and child both move into the new species; the species' template is `new BibiteTemplate(parentBibite)`.
- `create_new_species(bibite, parent)` inserts the new species immediately after the parent in `recordedSpecies`
  and in the position-sorted `activeSpecies` list (`:322-349`).

`CheckSpeciesOfEgg` (`:251-270`) runs on **egg resume only** (`EggHatching.ResumeHatch`,
`Here/SimulationScripts.BibiteScripts/EggHatching.cs:144`): iterate `activeSpecies` in order, track the nearest; if
any species is within span adopt it immediately, otherwise adopt the nearest regardless of distance (so an egg can
join a too-distant species rather than create one).

### 9.4 Species creation, records and naming

`Species` (`Here/SimulationScripts.BibiteScripts/Species.cs:14-138`):

| Field | Persisted | Meaning |
|---|---|---|
| `speciesID` (long) | yes | `speciesMaxID++` (static, saved by lineage manager) |
| `timeCreation` (double) | yes | `TimeKeeper.simulatedTime` at creation |
| `generationOfFirstSpecimen` (int) | yes | founder's `generation` |
| `favorite` (bool) | yes | never pruned |
| `genericName`, `specificName` | yes | `name = generic + " " + specific` |
| `description` | yes | free text |
| `parentID` (long) | yes (JSON `parentID`, only if parent != null) | lineage tree |
| `template` | yes (`SaveStateForSpecies`) | genome+brain snapshot |
| `parentSpecies`, `rootSpecies`, `children`, `bibites`, `eggs` | no (rebuilt from `parentID`) | live graph |
| `speciesData` (`LogLikeSpeciesDataArray`) | yes (separate `data.bin` / older JSON) | history buckets, `nPointsPresent` used by pruning |

Naming: species from a template uses the template's `speciesName` (or file/n. `name`), split on spaces;
genus = capitalised first word, species = second word lowercased or first word lowercased
(`Species.cs:94-98`). Species founded from a bibite inherit `parent.genericName` and generate a fresh specific name
via `SpeciesNameGenerator.GenerateNewName` (latin word lists; genus changes after 5 same-genus ancestors)
(`Here/Utility/SpeciesNameGenerator.cs:27-49`).

Template ownership quirk: `new BibiteTemplate(BibiteBody)` stores **references** to the live arrays
(`genes = gene.genes; nodes = brain.Nodes; synapses = brain.Synapses` — `Here/Utility/BibiteTemplate.cs:148-184`),
and `new BibiteTemplate(BibiteTemplate)` copies references too (`:191-202`). The game relies on those arrays being
replaced (not mutated in place) after creation, which is true for genes (`CopyGene` assigns a new array) and for
node/synapse arrays (`FinalizeEditing` assigns new arrays), but the *runtime fields* inside `Node` (`Value`,
`LastOutput`) of the founder's brain ARE the same objects as `template.nodes` while the founder lives. Speciation
distance only reads `Inov/baseActivation` (never `Value`/`LastOutput`) and `Synaps.Weight/En` (only changed by
editing, which re-creates arrays), so the game gets away with it. **Rust must snapshot** the template (deep copy) —
behaviourally equivalent and safer; document the deviation.

### 9.5 Registry persistence and innovation counters

`GlobalLineageManager` (`Here/ManagementScripts/GlobalLineageManager.cs`):

- `SaveState` (`:361-379`): serializes `recordedSpecies` (list of Species JSON), `activeSpeciesList` (ids),
  `nodeMaxInnovation`, `connectionMaxInnovation`, `nextSpeciesID`, `maxEnergy`, `maxCount`, `logCounter`,
  `nRecords`.
- `LoadState` (`:381-448`): restores innovation counters **before** loading species, loads recorded species,
  re-links `parentSpecies`/`children`/`rootSpecies` by `parentID`, restores `activeSpecies` from ids, and if
  `SaveSystem.fromVersion < LineageVersionUpdater.toSpeciesData` runs the legacy `speciesData` importer.
- `nodeMaxInnovation` / `connectionMaxInnovation` are the only persisted state of the NEAT innovation counters
  (statics in `NEATBrain.cs:251-253`); load them before any brain is constructed or IDs will collide.
- Real 0.6.3.1 file layout (`docs/spec/_tmp/recent/speciesData.json`):
  `{"recordedSpecies":[{speciesID,timeCreation,generationOfFirstSpecimen,favorite,genericName,specificName,
  description,parentID?,template:{version,nodes:[{Type,Index,Inov,Desc,baseActivation}],synapses:[{Inov,NodeIn,
  NodeOut,Weight,En}],genes:{35 keys}}}], "maxEnergy":..,"maxCount":..,"logCounter":..,"nRecords":..,
  "usedSpanText":{},\"activeSpeciesList\":[ids],\"nodeMaxInnovation\":155,\"connectionMaxInnovation\":342,
  \"nextSpeciesID\":5}` — `usedSpanText` is a UI field that serializes as `{}` and is irrelevant.
- Save-bundle `scene.bb8scene` also stores `version` (the save's version) and `simulatedTime`; the species/lineage
  state is in its own species section (`recordedSpecies` + `activeSpeciesList`), loaded after the scene.

### 9.6 Pruning (species history)

`LogSpeciesInfo` runs on the logging cadence; it logs a data point for every recorded species, then
`SpeciesTreeCleanup` (`:98-152`) removes species whose `speciesData.nPointsPresent <= speciesPrunePointLimit`
(default 2) except `absoluteKeep` (favourite, or no parent/root). Pruning merges the pruned species' data and
individuals into its parent (or the largest child if no parent) and rewires children
(`PruneSpeciesFromTree`, `:154-192`). Milestone-1 only needs this for parity of species counts/lineage stats; it
does not affect behaviour.

### 9.7 UI-only algorithms (do NOT port to the simulation)

`BrainSorter`, `NodeGroup`, `NodeSortHelper`, `ConnectionSortHelper`, `InputOutputAffinity`,
`HiddenNodeAffinity`, `InputOutputPath`, `HiddenDepthInPath` in `Here/ScriptHelpers/` are the brain-graph layout
force simulation for the editor (node positions, `MaxNodePath = 250` path enumeration guard
`Here/ScriptHelpers/BrainSorter.cs:11,84-87`). `ExpandedBrainParameters`
(`Here/SettingScripts/ExpandedBrainParameters.cs`) is purely visual tuning. They are irrelevant to brain execution
and can be ignored by the Rust port.

---

## 10. Persistence: JSON shapes, version gates and migrations

### 10.1 Where brain/genome data lives

| Container | Extension / location | Brain section | Genes section | Version field |
|---|---|---|---|---|
| Bibite template (blueprint) | `.bb8template` (`SaveSystem.bibiteTemplatePath` = `…/Bibites/Templates/`) | top-level `"nodes"`, `"synapses"` | top-level `"genes"` = **flat** `{GeneName:value}` | top-level `"version"` |
| Exported bibite snapshot | `.bb8` (`SaveSystem.savedBibitePath` = `…/Bibites/`) | `"brain": {"isReady", "Nodes", "Synapses"}` | `"genes": BibiteGenes.SaveState()` (nested) | top-level `"version"` (added by `SaveBibite`, `SaveSystem.cs:135-151`) |
| Save-bundle bibite | `bibites/bibite_N.bb8` inside save zip | same as `.bb8` (`SerializeBibite`, `SaveSystem.cs:426-445`) | same nested | **none** — save version from `scene.bb8scene.version` |
| Save-bundle egg | `eggs/egg_N.bb8` inside save zip | `"brain": {...}` | `"genes": SaveState()` | none (eggs' `.bb8` quick-save gets `version`) |
| Species record | save `recordedSpecies` / `speciesData.json` | `template.nodes/synapses` (template-style records) | `template.genes` flat | `template.version` |

`SerializationHelper.SerializeObject` uses `ISaveable.SaveState()` when available, else
`SerializeGeneralObject` over `[SerializeField]` fields (`Here/ScriptHelpers/SerializationHelper.cs:23-89`).
`NEATBrain` is *not* `ISaveable`, so the brain section is exactly its three serialized fields
`isReady`, `Nodes`, `Synapses` (`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:223-231`). Real 0.6.3.1 export
(`testdata/real/bibites/Spiller 1.bb8`):

```json
{
  "transform": {...}, "rb2d": {...},
  "genes": { "tag": "...", "speciesID": 12, "isReady": true, "gen": 16, "parent1": 123,
             "genes": { "LayTime": 17.9, "...": 0.0 } },
  "body": {...}, "clock": {...},
  "brain": {
    "isReady": true,
    "Nodes": [ {"Type":0,"baseActivation":0.0,"TypeName":"Input","Index":0,"Inov":1,
                "Desc":"EnergyRatio","Value":0.7752547,"LastInput":0.0,
                "LastOutput":0.775226533,"archetype":0}, ... ],
    "Synapses": [ {"Inov":2999,"NodeIn":13,"NodeOut":48,"Weight":1.57027686,"En":true}, ... ]
  },
  "version": "0.6.3.1", "desc": ""
}
```

`.bb8template` node records are the `SaveForTemplate()` subset — **no** runtime state, **no** `TypeName`,
**no** `archetype`:

```json
{"Type":0,"Index":0,"Inov":1,"Desc":"EnergyRatio","baseActivation":0.0}
```

and `.bb8template` top level (real 0.6.3.1 template, `testdata/real/templates_current/Basic_bibite_0.6.3.1.bb8template`):

```json
{
  "name": "Basic bibite", "speciesName": "Basic bibite", "description": "...",
  "generation": 0, "version": "0.6.3.1", "isOfficial": true,
  "nodes": [ {"Type":0,"Index":0,"Inov":1,"Desc":"EnergyRatio","baseActivation":0.0}, ... ],
  "synapses": [ {"Inov":1,"NodeIn":0,"NodeOut":45,"Weight":2.56878,"En":true}, ... ],
  "genes": { "LayTime": 15.0, "...": 0.0, "FatStorageDeadband": 0.25 }
}
```

(`nodeAnchors` may additionally appear when the template has editor node positions:
`"<inov>": [x,y]` map, `Here/Utility/BibiteTemplate.cs:50,262-265`.)

### 10.2 Genes JSON variants

1. **Template / species-template (flat)**: `"genes": { "LayTime": 15.0, ... }` — all 35 names
   (`BibiteGenes.Genes` enum names verbatim, PascalCase). Loaded by `BibiteTemplate.LoadState`
   (`Here/Utility/BibiteTemplate.cs:324-370`) and `BaseGeneticDesc` (`Here/SimulationScripts.BibiteScripts/
   BaseGeneticDesc.cs:19-75`). Missing key → `BibiteEditorSettings.SettingOfGene(i).DefaultValue`.
2. **Runtime bibite (`BibiteGenes.SaveState`, `BibiteGenes.cs:533-590`)**:
   `{"tag"?, "speciesID"?, "isReady", "gen", "isMutant"?, "isRad"?, "isRain"?, "isKiller"?, "isSource"?,
   "parent1"?, "parent2"?, "genes": {35 names}}` — optional keys are only present when true/non-null
   (parent ids are the parents' `BibiteID.id` ints).
3. **Legacy flat array** (templates saved before 0.6.0.1): `"genes": [floats...]` — loaded by
   `BibiteTemplate.LoadState` only when `Version.Parse(version) < 0.6.0.1`, mapped positionally then
   `BibiteUpdater.UpdateGenesToPresentVersion` **remaps the old layout** (§10.5).
4. **Very old named genes** (0.5-era save/`genes` objects): keys `Strength`, `Defence`, `ImmuneSystemStrength`
   existed instead of the WAG/eye/fat genes; `BibiteUpdater` reads their values to reconstruct modern genes
   (§10.5).
5. Egg `.bb8`/save JSON: `"genes": SaveState()` identical to runtime bibite (plus `"egg"` section).

`NeatBrain` + `BibiteGenes` runtime state notes for round-trip:
- Brain `Nodes[].Value/LastInput/LastOutput` and `Synapses` must be written/read for the *bibite snapshot* formats;
  the template format must NOT write them.
- `mutationProbability`, `mutationVariance`, `nHidden`, `nSynaps`, `nActiveInputs`, `nActiveOutputs`,
  `brainTargets`, `brainLinks` are **not** serialized (derived).
- Gene runtime metadata: `generation` (`gen`), `isMutant`, easter-egg flags, `parent1/parent2` ids, `speciesID`,
  `tag`, `isReady`. `mutationIncreases`/`mutationFactors` are **not** persisted (§8.1).
- Body serialized fields that the brain reads (and that must round-trip through the *body* section, not the brain
  section): `brainTicksCount`, `visionLookupCount`, `visionSensingCount`, `attackedDmg`, `d2Size` (from which
  `BibiteGrowth.growth` is reconstructed: `growth = d2Size / SizeRatio²`, `BibiteBody.cs:1031`).

### 10.3 Version parsing and comparison (must be ported exactly for migration gating)

`Here/Utility/Version.cs`:

- Format regex: `^[0-9]+(\.[0-9]+)(\.[0-9]+)?(\.[0-9]+)?([aA][0-9]+)?$` (`:258`).
- `Parse`: numbers are read left-to-right into V0,V1,V2,V3; if the string has `aN`, `N` becomes `Alpha` and the
  numeric components are assigned from the **last** numeric backwards (so `"0.6a5"` → V0=0,V1=6,V2=0,V3=0,
  Alpha=5; `"0.6.1a4"` → 0.6.1.0a4; `"0.6.0.1"` → 0.6.0.1).
- Comparison (`<`/`>`): compare V0..V3 numerically; if still equal, alpha rules:
  `a < b` iff (`b.Alpha == 0` or `a.Alpha < b.Alpha`) and `a.Alpha > 0`;
  `a > b` iff (`a.Alpha == 0` or `a.Alpha > b.Alpha`) and `b.Alpha > 0`.
  Consequences: a version with Alpha 0 is treated as newer than the same V0..V3 with Alpha>0; and the V4
  components always dominate the alpha suffix (`0.6a16 < 0.6.0.13`).
- The game compares a file's version against the constant list below. A file at 0.6.3.1/0.6.4a1 applies **no**
  brain/gene migrations (>= all entries).

### 10.4 Brain version changes (`Here/ScriptHelpers/BrainUpdater.cs:12-104`)

Entries (game order = descending; application order is ascending by version):

| Version | Added inputs | Removed inputs | Added outputs | Removed outputs | Callback |
|---|---|---|---|---|---|
| 0.6.2.1 | `(pos 5, 1)` = **RotationSpeed** | — | — | — | — |
| 0.6.0.18 | — | `(pos 32, 1)` = **InfectionRate** | — | `(pos 15, 1)` = **Spawn/Reproduce? output #15** | — |
| 0.6.0.16 | — | — | — | — | `UpdateTo0_6_0_16` |
| 0.6.0.15 | — | — | — | — | `UpdateTo0_6_0_15` |
| 0.6.0.13 | — | `(pos 0, 1)` = **Constant input** | — | — | `UpdateTo0_6_0_13` (atEnd) |
| 0.6.0.5 | `(pos 8, 1)` = **EggStored** | — | `(pos 3, 1)` = **EggProduction** | — | — |
| 0.4.0.3 | `(pos 28, 3)`, `(pos 4, 1)` — four unnamed inputs, positions as in the 0.3-era layout; identity is assigned by final position, so it depends on the 0.3 base layout (not recoverable from the change list alone) | — | — | — | — |
| 0.3 | — | — | — | — | — (list terminator, no changes) |

`NodeChanges(pos, amount)` positions are in the **pre-change** index space. The (unnameable) 0.6.0.18 removed
output was the 16th output of that era; all surviving outputs shift down. Derived index maps from real fixtures
are in §10.6.

Callbacks (`:330-426`):

```
UpdateTo0_6_0_16:
    n = nodes[38]                       # Want2Eat in that era's indexing
    a=0.28; b=0.528
    n.Type = TanH; n.TypeName="TanH"
    n.baseActivation = a*n.baseActivation + b
    for syn if syn.NodeOut==38: syn.Weight *= a

UpdateTo0_6_0_15:
    n = nodes[38]
    if n.baseActivation ≈ 0: n.baseActivation = 2.5

UpdateTo0_6_0_13:                       # Constant-node removal, run at the very end
    for syn in synapses where syn.NodeIn==0 and syn.En:
        idx = indexMap[syn.NodeOut]
        if idx >= 0: nodes[idx].baseActivation += syn.Weight
    for syn with syn.NodeIn==0:
        syn.NodeIn = Random.Range(1,14)        # Unity RNG! picks one of inputs 1..13 (Speed..NMeats),
        syn.En = false                         # NOT a specific input — migration is RNG-dependent
    for n in nodes where n.Type==Mult and n.baseActivation==0: n.baseActivation = 1
```

`UpdateElementsFromVersion` algorithm (`:203-328`) — the general recipe (must be ported, including its odd accounting):

```
update_elements(nodes, synapses, fromVersion):
    num = 33 (N_INPUTS current); num2 = 15 (N_OUTPUTS current); num3 = 0
    source = changes with version > fromVersion and anyChanges, DESCENDING
    indexMap = [0..num+num2)                    # current index space -> old index space
    for change in source (descending):
        for (pos,amt) in change.addedOutputs (ascending pos):
            indexMap.remove_range(num+pos, amt); num2 -= amt; num3 += amt
        for (pos,amt) in change.addedInputs (ascending pos):
            indexMap.remove_range(pos, amt); num -= amt; num3 += amt
        for (pos,amt) in change.removedInputs (ascending pos):
            indexMap.insert(pos, -1) x amt; num += amt; num3 -= amt
        for (pos,amt) in change.removedOutputs (ascending pos):
            indexMap.insert(num+pos, -1) x amt; num2 += amt; num3 -= amt
    for k in num+num2 .. nodes.len: indexMap.push(k + num3)     # hidden nodes keep relative order
    progIndexMap = copy(indexMap)

    for change in source (ASCENDING):
        updateBeforeFunction (none in 0.6.3.1)
        removedOutputs (descending pos): nodes.remove_range(num+pos, amt); progIndexMap.remove_range(...); num2 -= amt
        removedInputs  (descending pos): nodes.remove_range(pos, amt);     progIndexMap.remove_range(...); num -= amt
        addedInputs    (ascending pos):  for k in 0..amt: nodes.insert(pos, default); progIndexMap.insert(pos, pos+k);   num += amt
        addedOutputs   (ascending pos):  for k in 0..amt: nodes.insert(num+pos, default); progIndexMap.insert(num+pos, num+pos+k); num2 += amt
        updateAfterFunction (0.6.0.15/16)
    if num != 33 or num2 != 15: PopupManager.DisplayError("Bibite Updater", "…counts did not match up…")
    for change in source (ASCENDING): updateAtEndFunction (0.6.0.13)
    for i,n in nodes: n.Index = i; n.Inov = (i < 48) ? i+1 : n.Inov + num3
    synapses.remove_all(indexMap[NodeIn] < 0 or indexMap[NodeOut] < 0)
    for syn: syn.NodeIn = indexMap[syn.NodeIn]; syn.NodeOut = indexMap[syn.NodeOut]
    return indexMap                                     # also stored in static `indexMap` for callbacks
```

Additional notes:
- Inserted nodes are `default(Node)` = Type 0; `SetBaseNeuronsTypeAndDesc` runs afterwards and overwrites
  `Type/Desc/Index` of the **first 33** nodes as Input and next 15 as Output, which is how role/type of inserted
  nodes is assigned by position.
- For **templates**, after the array update the game also calls `SetBaseNeuronsTypeAndDesc` and then each change's
  `updateTemplateFunction` for versions > fromVersion (`:163-201`); in 0.6.3.1 no `updateTemplateFunction` is set
  anywhere, so this is a no-op.
- `UpdateBrainFromVersion` on a loaded bibite does the same and then `SetBaseNeuronsTypeAndDesc`
  (`:150-161`).
- `UpdateTo0_6_0_13` uses the static `indexMap` from the generic pass; the `Random.Range(1,14)` for disabled
  Constant synapses is a **Unity RNG** dependency — migration of pre-0.6.0.13 brains is not deterministic unless the
  RNG is seeded identically (the game's is unseeded). Port as deterministic using our own RNG stream
  (`rng.domain(RngDomain::LegacyBrainMigration)`); document as a parity deviation.
- `nodeMaxInov/connectionMaxInov` are **not** adjusted by BrainUpdater (templates align innovations separately via
  `BibiteTemplate.AlignInnovationsWithSim/WithTemplate`, `Here/Utility/BibiteTemplate.cs:372-421`).

### 10.5 Gene version changes (`Here/ScriptHelpers/BibiteUpdater.cs:19-103`)

`UpdateGenesToPresentVersion(genes, fromVersion, genesJson, geneRemapingNecessary, isSaved)`:

1. `from < 0.6a7` and `geneRemapingNecessary`: set genes 28..33 to defaults of
   `EyeOffset, StomachWAG, WombWAG, FatWAG, FatStorageThreshold, FatStorageDeadband` (`:33-41`).
2. `from < 0.6a15`:
   - read old `Strength` gene: `genesJson["Strength"]` (default 5) when not (`from<0.6a1` and not saved), else
     `genes[8]`;
   - read old `Defence`: `genesJson["Defence"]` (default 0), else `genes[9]`;
   - if `geneRemapingNecessary && !isSaved`: build a new 35-gene array through
     `to06a15GenesMap = [0,1,2,3,4,5,6,7,-1,-1,8,9,10,11,12,13,14,15,16,-1,17,18,19,20,21,22,23,24,25,26,27,28,33,34]`
     (34 old slots → new indices; −1 = dropped `Strength`/`Defence` position);
   - scale `array[26..28] *= 2.5` (Stomach/Womb/Fat WAG), zero `array[31],[29],[32],[30]` (Jaw/Armor/Move/Throat),
     recompute `num6 = TotalOrganWAG(array)`, then distribute:
     `p = min(0.01, (Strength/35)²)`, `d = (Defence/35)²`, `throat=0.05`, `mouth=0.1`,
     `throatWeight = num6*throat/(1-p-d-0.05-0.1)` etc. — the literal formulas are
     `array[31]=num6*num7/(1-num7-num8-0.05-0.1)` (Jaw), `array[29]=num6*num8/…` (Armor),
     `array[32]=num6*0.05/…` (Move), `array[30]=num6*0.1/…` (Throat) (`:90-100`).
   - `geneRemapingNecessary` is **false** for save-bundle bibite loads (`SaveSystem.cs:485-486` passes false) and
     true for templates; `isSaved` distinguishes save JSON from template JSON because of the nesting.
3. Return the (possibly reallocated) gene array.

Legacy versions do **not** get the `CapGenes` clamp here; subsequent `CopyGene`/`LoadState`/template loading clamps.
Very old template files with `version` < 0.6.0.1 and flat-array genes are repositioned by the same map.

### 10.6 Observed per-version brain layouts (ground truth fixtures)

Current game version (0.6.3.1 / 0.6.4a1) writes 33 inputs **with** `RotationSpeed` at index 5, outputs 33..47:
this is the canonical layout of §1/§4/§5. Real `.bb8` exports at 0.6.3.1 have `archetype` in node records; 0.6.0.1
exports do not; parsers must ignore unknown members and never require `archetype`.

Shipped/legacy templates extracted from installed resources and user data (`testdata/real/templates*/`) show:

| Version string | Parsed | Inputs | Constant | InfectionRate | RotationSpeed | Outputs start | Notes |
|---|---|---:|---|---|---|---:|---|
| `0.5` | 0.5.0.0 | 33 | yes (idx 0) | yes (idx 32) | no | 33 | old gene-JSON names (`Strength`, `Defence`, `ImmuneSystemStrength`) / flat array era |
| `0.6a5` | 0.6.0.0a5 | 34 | yes | yes | no | 34 | Constant + 33 named senses |
| `0.6a16` | 0.6.0.0a16 | 33 | no | yes (idx 32) | no | 33 | |
| `0.6` | 0.6.0.0 | 32 | no | no | no | 32 | `Basic bibite`, `Whale`, `Scorpion`, … (official legacy) |
| `0.6.0.1` | 0.6.0.1 | 32 | no | no | no | 32 | real export fixture |
| `0.6.1a4` | 0.6.1.0a4 | 32 | no | no | no | 32 | `Darth bibitus` fixture |
| `0.6.3.1` / `0.6.4a1` | 0.6.3.1 / 0.6.4.0a1 | 33 | no | no | yes (idx 5) | 33 | current; 206 user templates + all current saves/eggs |

Indexed maps:

```
CURRENT (33):   0 EnergyRatio 1 Maturity 2 LifeRatio 3 Fullness 4 Speed 5 RotationSpeed 6 IsGrabbing
                7 AttackedDamage 8 EggStored 9 BibiteCloseness 10 BibiteAngle 11 NBibites
                12 PlantCloseness 13 PlantAngle 14 NPlants 15 MeatCloseness 16 MeatAngle 17 NMeats
                18 RedBibite 19 GreenBibite 20 BlueBibite 21 Tic 22 Minute 23 TimeAlive
                24 PheroSense1 25 PheroSense2 26 PheroSense3 27 Phero1Angle 28 Phero2Angle 29 Phero3Angle
                30 Phero1Heading 31 Phero2Heading 32 Phero3Heading
                outputs 33 Accelerate 34 Rotate 35 Herding 36 EggProduction 37 Want2Lay 38 Want2Eat
                39 Digestion 40 Grab 41 ClkReset 42 PhereOut1 43 PhereOut2 44 PhereOut3
                45 Want2Grow 46 Want2Heal 47 Want2Attack; hidden 48+

V0.6 / V0.6.0.1 / V0.6.1a4 (32): same CURRENT list minus index 5 (RotationSpeed);
                outputs start at 32 (Accelerate@32 … Want2Attack@46); hidden 47+

V0.6a16 (33):   0 EnergyRatio … 4 Speed 5 IsGrabbing 6 AttackedDamage 7 EggStored 8 BibiteCloseness
                … 31 Phero2Heading 32 InfectionRate; outputs 33 Accelerate … 47 WantaAttack;
                i.e. CURRENT minus RotationSpeed plus trailing InfectionRate.

V0.6a5 (34):    0 Constant 1 EnergyRatio 2 Maturity 3 LifeRatio 4 Fullness 5 Speed 6 IsGrabbing
                7 AttackedDamage 8 EggStored 9 BibiteCloseness … 32 Phero2Heading 33 InfectionRate;
                outputs 34+

V0.5 (33):      0 Constant 1 EnergyRatio … (same shape as V0.6a5 but without EggStored at 8:
                8 BibiteCloseness 9 BibiteAngle …), outputs 33+
```

⚠ **Known migration anomaly (open question Q7).** Applying the literal `UpdateElementsFromVersion` to the
shipped `0.6`/`0.6a5`/`0.6a16` templates produces over-shifted maps because their numeric version compares *older*
than the 0.6.0.x changes that their content already includes (e.g. a `0.6` template is run through the
EggStored+EggProduction insertion, the Constant removal, the InfectionRate removal and the RotationSpeed insertion
all over again). The game's count check compares the tracked counters (which coincidentally still equal 33/15 for
`0.6`) rather than the number of `Type==0` nodes, so it does not reject them; the resulting brain has removed
`EnergyRatio`/`Accelerate`, inserted blank nodes and a shifted synapse indexMap. The 206 current templates and
current saves avoid this by being at version 0.6.3.1+. Rust policy recommendation:

1. **Default (parity) mode:** implement the literal algorithm for `version < 0.6.2.1`, including the counting quirk
   and the count-mismatch error.
2. **Defensive mode (CLI/io default for real files):** pre-classify the shipped layout by *content signature*
   (constant count of `Type==0` nodes, presence of `Desc=="Constant"`/`"InfectionRate"`/`"RotationSpeed"`), apply
   only the changes required to reach the current layout, and validate `count(Type==Input)==33`,
   `count(nodes)==48+nHidden`, `Inov` monotonicity. On failure emit a `Warning` with the detected era and skip the
   file (do not silently corrupt).
3. Fixtures in `testdata/real/templates*/` cover every era; the golden test should assert the *detected era* and the
   resulting normalized layout (§11.10).

**Current implementation (2026-10-30):** `bibites-io` applies the literal `UpdateElementsFromVersion` algorithm
(option 1) for files below 0.6.2.1, then normalises base node types/desc (`migrate::normalize_base_nodes`) and
validates the resulting layout; option 2's content-signature era detection is **not** implemented. The tolerant
template path repairs node `Index`/duplicate-innovation issues the game itself tolerates (task-44, `D14`) but does
not reinterpret the numeric version by content. Any leftover anomaly in a shipped `0.6`-era template is therefore
surfaced by validation rather than auto-corrected; report such a file instead of silently rewriting it.

### 10.7 Effective settings and runtime changes (important for scheduling)

- `simTPS`, `brainTPS`, `decayTPS`, vision factors are **world-scope ("independent") settings**, serialized in
  settings files (`"independents": {"simTPS":30,"brainTPS":2,...}`) and overridable per scenario. Real examples:
  the default world is 30/2; `Ring.zip` sets `simTPS = 15`; some scenarios use 40. **Never hardcode 30/2/10/5.**
  Compute `dt = 1/sim_tps`, `brain_period = brain_tps/sim_tps`, and compare the vision/clock accumulators against
  the effective factors every tick.
- Runtime changes: the game subscribes to these settings and updates the live statics
  (`NEATBrain.UpdateBrainTPS/UpdateSimTPS`, `BibiteBody.UpdateBrainTPS/UpdateVision*`, `TimeController` for
  `fixedDeltaTime`) (`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:332-342`,
  `Here/SimulationScripts.BibiteScripts/BibiteBody.cs:369-382`, `Here/ManagementScripts/TimeController.cs:78-130`).
  Effects:
  - `brainPeriod` changes **immediately**; the next brain tick already uses the new period (accumulators are not
    rescaled, so the phase shifts).
  - `brainTickFactor`/vision periods change the *threshold* of the running counters; elapsed counts are preserved.
  - `sim_tps` changes `Time.fixedDeltaTime` but does **not** rescale `brainPeriod` independently of brain_tps (both
    are recomputed from the settings ratio). In Rust, read `s.brain_period()` fresh at every `step` and use the
    current `visionLookupUpdates/visionSenseUpdates` when testing counters.
- Settings animators (scenario "animators") can change any of these at any tick; so the Rust brain/vision code must
  not cache them across ticks. (ARCHITECTURE §14 deviation: we run a fixed `dt = 1/sim_tps`; a mid-run sim_tps change
  is allowed but will re-scale time like the game.)

---

## 11. Rust design notes (implementation contract)

This section maps the above onto `docs/INTERFACES.md` §4/§9. The frozen signatures are the contract; deviations
must be raised to the Lead (see §11.8 — four are currently proposed).

### 11.1 Modules

| File | Contents |
|---|---|
| `genome.rs` | `Gene` enum (35, exact order §7), `N_GENES`, `Genome`, editor ranges table (`gene_range`, `gene_span`, `is_purely_relative`), `Genome::starter()`, derived formulas `d1_size_at_birth`/`d2_size_at_birth`/`area_at_birth`/`area_at_mature`/`length_at_mature`/`growth_at_mature`/`organ_area_portion`/`total_organ_wag`/`metabolic_efficiency`/`body_energy_density`, mutation kernel (`next_gaussian`, `next_mutation_value`, `next_poisson` live in `rng`/`genome`), `Genome::child()` |
| `brain.rs` | `Input`, `Output`, `NodeType`, `NodeArchetype`, `N_INPUTS`, `N_OUTPUTS`, `NodeRecord`/`SynRecord` (io-facing plain records), `Brain` (SoA + CSR), `BrainInputs`, `BrainOutputs`, `execute_neuron`, `Brain::tick`/`step`, building/validation (`rebuild_csr`, `validate`, `finalize_editing`, floating-hidden repair), mutation ops (`add_synapse`, `change_synapse_strength`, `flip_synapse`, `toggle_synapse`, `remove_disabled_synapse`, `remove_synapse`, `change_default_act`, `change_neuron_type`, `add_neuron`, `remove_neuron`), `Brain::mutate`, `Brain::distance` (or delegate to `species.rs` using records), innovation counter state type |
| `species.rs` | `SpeciesId`, `SpeciesRecord`, `SpeciesRegistry`, genetic distance (gene+brain), `used_species_span`, assignment state machine, lineage tree, name generator (optional; can be simple deterministic), pruning |
| `migrate` (inside `brain.rs` or a submodule) | pure version-migration functions over `NodeRecord`/`SynRecord` + `EraLayout` detection (§10.4/§10.6); no serde, no file I/O |

`genome`/`brain` must not depend on body/world (INTERFACES §1). `BrainInputs`/`BrainOutputs` are plain value types;
`write_world_senses`/`write_internal` are thin field copies owned by body/world.

### 11.2 Determinism and RNG policy (statistical parity, not bit parity)

The game mixes C# `System.Random` (Poisson/Gaussian/deltas) and UnityEngine.Random (index picks/rolls), both
unseeded in practice; exact mutation outcome parity is impossible and is **not** required (ARCHITECTURE §5/§9).
Requirements:

- Every stochastic brain/genome operation takes `&mut Rng` explicitly (INTERFACES §2). No thread-local/global RNG.
- Streams: per-Bibite or per-operation derivation via `Rng::derive(seed, domain, entity)` with distinct
  `RngDomain` tags, e.g. `GeneMutation`, `BrainMutation`, `Speciation`, `LegacyBrainMigration`, `RandomGenes`,
  `MutationEasterEgg`. Keeping domains separate makes results independent of execution order/thread count.
- The *distribution* must match the game:
  - gene mutation count ~ Poisson(genes[9] + bg_chance + factor.probability_increase);
  - gene delta = `next_mutation_value(current, σ, span, purely_relative)`, σ = genes[8] + bg_var +
    factor.intensity_increase;
  - brain mutation count ~ Poisson(genes[11] + bg_chance + factor.probability_increase);
  - branch synapse/neuron with P = synapseMutationChance (0.6), then the cumulative subtractions of §8.5/§8.6
    (including the exact default partition; use the same branch logic so custom settings behave identically);
  - weight/bias deltas = `next_mutation_value(value, σ_brain, 1.0, purelyRelative = type==Inhibitory for bias)`;
  - new synapse weight ~ N(0, 0.5); index picks uniform in the exact ranges of §8.5 (keep the `NInputs-1`
    off-by-one for input picks if matching the game's distribution is desired);
  - `AddNeuron` type/bias rolls exactly as §8.6 (uniform type in 1..13 inclusive, `10^U(-2,1)`/`10^U(-1,1)`,
    `Random.value >= 0.5` split).
- Document in code that mutation results are statistically, not byte, comparable to the game. The deterministic
  `(seed, scenario, settings)` contract applies to our engine.
- Migration of pre-0.6.0.13 Constant brains needs a random input pick (game: Unity RNG); use a dedicated domain and
  document that the migrated brain differs run-to-run from the game but is stable for us.

### 11.3 Brain memory layout (SoA + CSR)

Recommended representation (fits ~100k bibites: brain sizes in real saves are typically 48 nodes/8 synapses up to
~168 nodes/330 synapses; cap defensively, e.g. `MAX_NODES = 4096`, `MAX_SYNAPSES = 65536` with fallible
constructors):

```rust
pub struct Brain {
    // nodes (SoA, len = n_nodes; 0..33 inputs, 33..48 outputs, 48.. hidden)
    kind: Vec<NodeType>,          // u8 repr
    bias: Vec<f32>,
    inov: Vec<i64>,               // kept ascending; needed for distance + io
    // per-node dynamic state (persisted in .bb8/save, not in templates)
    prev_out: Vec<f32>,           // LastOutput of the previous tick
    cur_out: Vec<f32>,            // Value / current output
    prev_in: Vec<f32>,            // LastInput (full activation) previous tick
    cur_in: Vec<f32>,
    // synapses (SoA, len = n_syn)
    syn_in: Vec<u32>,
    syn_out: Vec<u32>,
    syn_weight: Vec<f32>,
    syn_enabled: Vec<bool>,
    syn_inov: Vec<i64>,           // ascending (invariant §1.3)
    // CSR over enabled incoming synapses, game order:
    targets: Vec<u32>,            // destination nodes, first-appearance order (§6.1)
    target_offsets: Vec<u32>,     // len = targets.len()+1
    target_syns: Vec<u32>,        // synapse indices grouped by target, ascending synapse index
    // derived counts used by body/settings:
    n_hidden: u16,
    // scratch
    disabled_syns: Vec<u32>,      // maintained by toggle ops only while mutating
}
```

- `N_INPUTS/N_OUTPUTS` fixed; `n_hidden = n_nodes - 48`.
- Inputs do not need `bias` (always 0) but keep the array uniform for index simplicity.
- `execute_neuron` should be a `#[inline]` free function over `(kind, bias, prev_out, prev_in, acc, period)` with
  the exact switch of §2.1; no branch on bias for every node — precompute `bias_mode` (Added/Multiplied/Ignored/
  Special) once per brain finalise (4-valued u8) to keep the hot loop tight.
- The tick loop is allocation-free: reuse `cur_out/cur_in`; swap with `prev_*` by `std::mem::swap` (§6.2).
  No `Vec` growth inside a tick.
- CSR rebuild happens only on load/finalise/mutation, never per tick. `mutate` rebuilds at the end.
- The parallel unit is a whole `Brain`; the `World`/`BibiteStore` can iterate brains in disjoint slots and call an
  `&mut` method per brain or use a split-borrow helper. Within a brain no parallelism is needed/used.

### 11.4 API mapping / call sites

| INTERFACES item | game equivalent | notes |
|---|---|---|
| `BrainInputs::set/get` + body/world writers | `NEATBrain.UpdateSenses` (`NEATBrain.cs:570-610`) | Body/world fill 33 fields *before* `Brain::step`; input ordering/staleness (§4.1) is a `World`/body concern: the **one-tick input delay** must be modelled outside `Brain::step` (pass the previous brain tick's input values) or inside `Brain` via `prev_out` for inputs (see §11.5). |
| `Brain::step(&mut self, inputs, period) -> BrainOutputs` | `NEATBrain.Thinker` | should copy `inputs` into the 33 `cur_out` slots after swapping, then evaluate CSR rows. `period` from `s.brain_period()`. Outputs = current output node values (clamped). |
| `Brain::upkeep() -> BrainUpkeep` | `brainUpkeepCost` (`NEATBrain.cs:283`) | `BrainUpkeep { n_synapses: u32, n_hidden: u32 }`; body computes `n_synapses*synapse_upkeep + n_hidden*neuron_upkeep` per second (settings live in `SimSettings`) and charges `* dt`. |
| `Brain::mutate(&mut self, rng, s)` | `CopyBrain(..., mutate:true)` + §8.5/§8.6 | See §11.8 for the needed params (λ/σ/factor). |
| `Brain::distance(&self, other, s)` | `Species.BrainDistanceToIndividual` | The `s` parameter is unnecessary for the 0.6.3.1 formula (no settings); keep for interface stability. Needs nMaxConnections/union counts; implement in `species.rs` over records or in `brain.rs` over SoA. |
| `Genome::child(&self, rng, s, factor)` | `CopyGene(increaseGeneration:true, mutate:true)` | Must set `generation = parent+1` and copy/aggregate `mutationIncreases` — but `Genome` has no such fields (see §11.8); the caller (body/world) owns generation and meta. If `preventMutations`, return clone. Easter-egg flag inheritance (10% each) and the 1/500000 easter-egg roll are also outside `Genome` unless meta is added. |
| `Genome::starter()` | official "Basic bibite" 0.6.3.1 | genes = editor defaults exactly (§7.1 defaults); brain = 48 nodes, 3 synapses: `Fullness(3)->Digestion(39) w=4.073`, `PlantAngle(13)->Rotate(34) w=1.159`, `PlantCloseness(12)->Accelerate(33) w=-0.401`; output biases `Accelerate 0.4536`, `EggProduction 0.2`, `Want2Eat 1.228`, `Digestion -2.0669`; all other biases 0; node Inov 1..48; synapse Inov 1..3, all enabled (`testdata/real/templates_current/Basic_bibite_0.6.3.1.bb8template`). |
| `Brain::starter()` | same brain as above | no hidden nodes. |

### 11.5 Sense staleness in the Rust pipeline

The game's sequence per brain tick is: `LastOutput_i = Value_i` for all inputs (so the network sees the *previous*
tick's senses), then `Value_i = fresh_sense`. In Rust, keep this out of the sense providers: `Brain` keeps
`prev_out[0..33]` = input values as passed on the previous `step`; `step` swaps, writes the new `inputs.values`
into `cur_out[0..33]`, then the CSR rows read `prev_out` (as in §6.2). This reproduces the exact one-brain-tick
delay for every input without any world-side history. Senses that are themselves throttled (vision every 10/5
brain ticks, pheromones every sim tick but sampled after the brain) are the providers' responsibility; document the
resulting total lag: vision = up to `(lookup factor + 1)` brain ticks for entity list + `(sense factor)` for values
+ 1 tick input delay.

### 11.6 Genome-derived values body/world need (exact formulas)

Implement in `Genome` with the constants from §7.2, taking `&SimSettings` where scenario settings enter
(`viewAngleAddedCost`, `viewRadiusAddedCost`, `baseBodyGrowthCost`, `storableEnergyPerArea`,
`healthEnergyFactor`, `energyUsageEfficiency`):

```
total_organ_wag      = Σ max(0, genes[26..32])
organ_area_portion(g)= max(0, genes[g]) / max(total_organ_wag, 0.0001)
growth_at_birth      = (genes[HatchTime]/genes[BroodTime])^2
growth_at_mature     = max(1, growth_at_birth / max(organ_area_portion(WombWAG), 0.001))
d2_size_at_birth     = genes[SizeRatio]^2 * growth_at_birth
d1_size_at_birth     = genes[SizeRatio] * sqrt(growth_at_birth)
area_at_birth        = 80 * d2_size_at_birth
area_at_mature       = 80 * growth_at_mature * genes[SizeRatio]^2
length_at_mature     = 12.5 * sqrt(growth_at_mature) * genes[SizeRatio]
metabolic_efficiency = s.energy_usage_efficiency ^ genes[SpeedRatio]
body_energy_density  = s.base_body_growth_cost + genes[ViewAngle]*s.view_angle_added_cost
                     + genes[ViewRadius]*s.view_radius_added_cost
                     + organ_area_portion(ArmorWAG) * s.materials.armor.energy_density
```

`Genome::set_gene` clamps to the editor range (§7.1). The editor metadata is one `const` table
(`genome::GENE_DEFS: [GeneDef; 35]` with `name/display_name/min/max/default/units/purely_relative`) — the
single source of truth for `set_gene`, `cap`, `child`, io defaults and tests (implemented; the earlier
`GENE_MIN`/`GENE_MAX`/… sketch was folded into `GENE_DEFS`).

### 11.7 Species registry design

Suggested concrete shape consistent with INTERFACES §9 (see §11.8 for the signature issue):

```rust
pub struct SpeciesRegistry {
    records: Vec<SpeciesRecord>,          // stable SpeciesId = u32 index into recorded history
    active: Vec<u32>,                     // game's activeSpecies list order (insertion/position semantics!)
    children: Vec<Vec<u32>>, parent: Vec<Option<u32>>, root: Vec<u32>,
    next_id: u32, node_max_inov: i64, conn_max_inov: i64,
    used_span: f32,
}
pub struct SpeciesRecord {
    id: u32, time_creation: f64, generation_of_first: u32, favorite: bool,
    generic: String, specific: String, description: String,
    template: TemplateSnapshot,            // deep copy of genes + node/syn records (§9.4)
    parent: Option<u32>, root: u32,
    // stats: count, energy, n_points_present, ...
}
```

`assign()` should implement §9.3's `CheckNewSpecies` state machine; `on_birth`/`on_death`/`tick_stats` maintain
counts and history. **Critical ordering:** `active` is scanned in list order (game order), and `children` in
insertion order; do not sort. Species assignment happens at birth and is order-dependent only across individuals
observed in the same tick if a new species is created; the world must process births in deterministic bibite-slot
order (batch or sequential) so runs are reproducible in single- and multi-threaded mode.

Innovation counters (`node_max_inov`, `conn_max_inov`) live in the registry/world (not in `Brain`) because they are
global statistics; they are saved/loaded from `speciesData`/save species section (§9.5). Mutations need
`&mut` access to them: pass a small `InnovationCounters { node: i64, conn: i64 }` to brain mutation ops, or store
them in `Brain` and merge into the registry. **Recommended:** `Brain` holds its own next-innovation values
(`i64` fields) seeded from the world counters at construction and exported back after mutation; the io layer writes
`nodeMaxInnovation`/`connectionMaxInnovation` from the registry updated after each mutation. This matches the game
(global statics) while keeping `brain.rs` self-contained.

### 11.8 Interface issues raised to the Lead (all resolved 2026-10-29)

The four issues below were resolved in `docs/INTERFACES.md` before implementation; the final contracts are
`GenomeMeta`, `Genome::child(&self, meta, rng, s, factor) -> (Genome, GenomeMeta)`,
`Brain::mutate(&mut self, rng, s, genome, factor)` and `SpeciesRegistry::assign(&mut self, genome, brain,
meta: &mut GenomeMeta, s)`. The original analysis is kept for context.

1. **`SpeciesRegistry::assign(&mut self, genome, brain, s)` has no inherited species and no `is_mutant`.**
   The game algorithm (§9.3) needs (a) the species inherited from the parent genome to decide "stay / adopt child /
   found new species", and (b) a persistent per-bibite `is_mutant` flag (two-step founding). Proposed either:
   - add a `GenomeMeta` value type owned by `genome.rs`:
     `{ generation: u32, species: Option<SpeciesId>, species_tag: Option<String>, is_mutant: bool,
        is_rad/is_rain/is_killer/is_source: bool, parent1: Option<BibiteId>, parent2: Option<BibiteId> }`,
     and extend `assign(&mut self, genome, brain, meta: &mut GenomeMeta, s) -> SpeciesId`; or
   - do assignment in `on_birth(&mut self, genome, brain, meta: &mut GenomeMeta, parent: Option<SpeciesId>, s)`.
   The `GenomeMeta` type is also required for `.bb8`/save round-trip regardless (§10.2) — it exists in the game
   inside `BibiteGenes` but is not in the frozen `Genome` struct.
2. **`Brain::mutate(&mut self, rng, s)` lacks the gene-derived λ/σ and `MutationFactor`.** The game reads
   `genes[BrainAverageMutation]`, `genes[BrainMutationSigma]`, background settings and the parent's
   `mutationIncreases`. Proposed:
   `Brain::mutate(&mut self, rng: &mut Rng, s: &SimSettings, genome: &Genome, factor: &MutationFactor)`.
3. **Sign conventions in INTERFACES §2**: "angles: + = CCW in the source formula, negative = target on the right;
   `RotationSpeed` + = clockwise" — the second half follows the in-game tooltip, but for the official starter
   reflex to steer *toward* a plant it must hold that **positive Rotate turns toward the side that gives a
   positive SignedAngle**. With Unity `SignedAngle(forward,right) = −90`, that means positive Rotate = positive
   angular velocity = CCW in (x-right,y-up) coordinates (the tooltip's "clockwise" is a mirrored label; see Q1).
   Recommend changing the INTERFACES parenthetical to "`RotationSpeed` + = same sense as positive Rotate
   (= CCW in our world coordinates)".
4. **`Genome::child` returning only `Genome`** cannot advance `generation`, copy flag inheritance, easter-egg roll,
   or aggregate `mutationIncreases`. Covered by the `GenomeMeta` proposal (1): `child(&self, rng, s, factor) ->
   (Genome, GenomeMetaDelta)` or a `child_into(&mut self, meta: &mut GenomeMeta, rng, s, factor)`.

Until these are resolved, implement the types and pure functions exactly as frozen and keep the missing state in
`SpeciesRegistry`/`World` locally. **Resolved:** `GenomeMeta` landed in `genome.rs`, the signatures above were
frozen in `docs/INTERFACES.md` §4/§9 and the implementation follows them.

### 11.9 Test plan (unit + golden)

Unit (`genome.rs`):
- ranges/defaults table matches §7.1; `set_gene` clamps; `starter()` genes equal defaults.
- derived formulas vs hand-computed values for 3 fixture genomes (e.g. `Explanatory Bibite`), including
  `total_organ_wag` with negative WAGs (max(0,·)) and `growth_at_mature`.
- mutation kernel statistics: with fixed seeds, ~Poisson mean, delta distribution shape, purely-relative behaves
  multiplicatively; clamp respected.

Unit (`brain.rs`):
- activation table: for each type, compare against a reference implementation of §2.1 for several
  `(x, last_out, last_in, bias, period)` vectors, including clamps; stateful types explicitly:
  Latch thresholds 0/1 inclusive, Differential first-tick spike, Integrator accumulation + clamp, Inhibitory decay
  with bias, SoftLatch branch boundaries (0.001/0.999) and both branches.
- bias rules: Mult multiplies, Integrator/Inhibitory/SoftLatch ignore additive bias, Input ignores bias.
- `Brain::step` equals a naive per-node reference (synchronous semantics); shuffle CSR row order and assert the
  output vector is unchanged (order independence).
- floating-hidden repair and CSR rebuild invariants (§6.3).
- mutation ops: add/remove/flip/toggle/change type/bias invariants (no NaN, in-range indices, Inov sorted, total
  gain preserved by AddNeuron split).
- distance: identical brains → 0; one flipped synapse → expected toggled/disjoint terms; hand-computed examples.

Golden / io fixtures (`testdata/real/`, gitignored, committed digests only):
- parse every template in `testdata/real/templates/` and `templates_current/`: assert detected era, normalized node
  layout (33 inputs / 15 outputs / hidden), Inov monotonicity, and re-serialization byte-shape (key set) for the
  current version.
- parse `testdata/real/bibites/Spiller 1.bb8` (0.6.3.1) and `Lunaloutrae rosalynnae.bb8` (0.6.0.1): assert
  runtime state round-trip (`Value/LastInput/LastOutput` preserved; `.bb8` vs template distinction).
- `Basic bibite` 0.6.3.1: `Brain::starter()` and `Genome::starter()` equal the fixture exactly.
- run the "Basic bibite" brain 100 ticks with a synthetic constant sense vector and compare against a reference
  interpreter built from this spec.

Parity (Lead's harness): per-tick brain input/output vectors from ground-truth captures for exact (f32 rounding)
comparison once physics drift is excluded (ARCHITECTURE §9).

---

## 12. Open questions / ambiguities

Every item below is *unresolved in the decompiled source* or a deliberate porting decision. Ranked by risk to
behavioural fidelity.

| # | Question / ambiguity | Evidence | Recommended resolution |
|---|---|---|---|
| Q1 | **Sign of `Rotate`, `RotationSpeed`, and vision/pheromone angles.** In-game tooltips say "negative angles = left, positive = right", "positive rotate = clockwise", "positive angular speed = clockwise"; Unity math says `SignedAngle(up, right) = −90` and positive z-torque is CCW. If the tooltips are taken literally, the official "Basic bibite" reflex (`PlantAngle --+1.159--> Rotate`) steers *away* from the plant. | `Here/Utility.InformationWrapers/NodeInformations.cs:53,90,281`; UnityCsReference `Vector2.SignedAngle`; `testdata/.../Basic_bibite_0.6.3.1.bb8template` | Treat the **behavioural pairing** as authoritative: angle input positive = counter-clockwise from forward (target on the left); positive Rotate = positive angular acceleration = counter-clockwise (left). The tooltip labels are mirrored. Validate with a capture: drive `Rotate=+1` and observe the sign of `rotation`/`angularVelocity`, and drive a bibite with a plant on its right and confirm it turns toward it. Update INTERFACES §2 wording. |
| Q2 | **Pheromone sensing cadence.** `Pherosense.progress` is initialised with `SensePeriod(0.5)*U(-0.5,0.5)` but never used in the decompiled `PherosenseAround`; the method does full `OverlapCircle` every `FixedUpdate`. A gating block may have been lost by the decompiler (or the field is vestigial). | `Here/SimulationScripts.BibiteScripts/Pherosense.cs:16-18,65-66,70-215`; call site `BibiteBody.cs:630` | Port as 30 Hz (every sim tick) per code, and verify with a capture against the game (count `Physics2D.OverlapCircle` calls / compare phero inputs when a bibite passes a fresh trail). Keep the `SensePeriod`/jitter field only for a possible future gated variant. |
| Q3 | **Ordering of pheromone sensing vs the brain.** `BibiteBody.FixedUpdate` calls `brain.Thinker()` *before* `pherosenseOrgan.PherosenseAround()`, so the brain reads pheromone values one sim tick old (plus the input one-brain-tick delay). If a capture shows differently, the order changed in the live build. | `BibiteBody.cs:603-631` | Reproduce body order: brain → organs → pheromone sense. Total lag: phero = 1 sim tick + 1 brain tick. |
| Q4 | **Unity script execution order for `InternalClock`, `BibiteGrowth`, `EggHatching`, organ `FixedUpdate`s** is not fixed in the decompiled code (no `DefaultExecutionOrder` seen). Within a sim tick the brain may see `tic`/`growth`/`chronoTime` from before or after the clock/growth update. | separate MonoBehaviours; `InternalClock.cs:65`, `BibiteGrowth.cs:91`, `BibiteBody.cs:592` | Rust chooses one explicit order (spec suggestion: clock+growth first, then vision/phero sampling, then brain, then organs — as ARCHITECTURE §6/INTERFACES §6.1 partly say) and records the deviation. Validate with per-tick captures of `tic`/`growth`. Document in ARCHITECTURE §14. |
| Q5 | **`viewRadius`/`viewAngle` sampled at which moment?** `fow.viewRadius` is updated on growth (body-size scaled); the entity list refresh and the sense computation happen at different brain ticks (10 vs 5), so both use the current `viewRadius` and entity list from up to 10 brain ticks ago — exact staleness of the radius itself is unclear. | `BibiteBody.cs:699-726,513`; `FieldOfView.cs:210-521` | Reproduce literally: weights use the *current* `viewRadius` at `ComputeSenses` time, against the entity list and cached distances from the last `FindSeenEntities`. Add capture test with growing bibites. |
| Q6 | **Slow-motion `brainPeriod`.** `Time.fixedDeltaTime` is scaled in slow motion but `brainPeriod = brain_tps/sim_tps` is not, so stateful activations use a larger dt than the actual brain step. | `TimeController.cs:117-130`; `NEATBrain.cs:332-342` | Irrelevant headless (no slow motion). Document as a game bug/UI feature; Rust always uses `brain_period = brain_tps/sim_tps` with the real fixed dt. |
| Q7 | **Version migration over-application for `0.6`/`0.6aN` templates** (§10.6). The literal algorithm re-applies 0.6.0.x changes to content that already has them, and the count check does not catch it. | `BrainUpdater.cs:12-328`; fixture layouts in `testdata/real/templates/`; current templates all 0.6.3.1 | Parity mode = literal algorithm; default io mode = content-signature detection + strict validation, warn/skip on anomaly. Ask Lead/fmt-dev to choose per call site (round-trip vs parity). |
| Q8 | **`ChangeDefaultAct` retry loop.** If the randomly picked node is an Integrator, it recurses (new random node, new uniform roll); with a brain full of Integrators (no bias mutation possible) this could recurse many times. RNG consumption and termination probability are not documented. | `NEATBrain.cs:916-935` | Port literally with a bounded retry and fallback to no-op; statistical behaviour identical for normal brains. **Implemented:** bounded at 1024 retries (`brain.rs::change_default_act`). |
| Q9 | **`MutateNeurons` subtraction quirk** (`num -= neuronRemoveChance` before the removal test, and a dead final `else num -= neuronChangeChance`). With the default settings (0.7+0.1+0.1+0.1=1.0) the outcome partition is still 70/10/10/10; with custom settings the partition differs from the intuitive sequential model. | `NEATBrain.cs:815-844` | Reproduce the code as written (including the early subtraction); note that the effective removal probability equals `max(0, remaining after first three)` and is **not** exactly `neuronRemoveChance` unless the four sum to 1.0. |
| Q10 | **`GetInputIndex` off-by-one**: `Random.Range(0, NInputs - 1)` never picks input index 32 (`Phero3Heading`). | `NEATBrain.cs:1058-1061,685` | Keep for distribution parity (32/33 uniform), or fix and document; recommend keep + comment. |
| Q11 | **`PelletIsHeld` mutates the held list during vision**, removing a held pellet from `held`/`nHeld` when it is seen. This can silently drop grabs (mouth bookkeeping also uses `nHeld`). | `FieldOfView.cs:523-536` | Reproduce exactly; flag as a probable game bug that could affect grabbing parity. |
| Q12 | **Species template aliasing**: `new BibiteTemplate(bibite)` shares gene/node/synapse array references with the live bibite; `Species.template` can therefore observe live `Node.Value/LastOutput` mutations (not read by distance) and mutation invalidation. | `BibiteTemplate.cs:148-202`; `Species.cs:103-130` | Rust snapshots deeply (behaviourally equivalent); document deviation. |
| Q13 | **`isMutant` semantics**: persisted per-bibite flag used only for the two-step speciation rule; on egg hatching it is inherited via `CopyGene` (`species`, `isMutant`, `deuxParent` copied) but `CheckNewSpecies` is only run at body birth/resume (not for the egg's brain). | `BibiteGenes.cs:324-327`; `BibiteBody.cs:435,479`; `GlobalLineageManager.cs:229-234` | Port: `is_mutant` is genome-meta state, copied to the child gene object at egg lay, reset when a species is founded, persisted. |
| Q14 | **`IsGrabbing` is documented as 0/1 but is `mouth.nHeld` (0..10)**, and `Maturity` is documented as "1.0 = can reproduce" but is raw `growth` (which is 1.0 only when `growthAtMature == 1`). | `NEATBrain.cs:582,577`; tooltips `NodeInformations.cs:57-62,22-26` | Implement the code (count / raw growth); note tooltip mismatch. |
| Q15 | **`ComputeActiveNeurons` recursive repair** can re-enter `CopyBrain`/`FinalizeEditing` while `ResumeBrain` is running; a file with many floating hidden nodes triggers repeated full copies (and RNG-free). | `NEATBrain.cs:467-507,412-421` | Implement a single-pass repair (remove all floating, re-finalise once) — equivalent because the loop re-checks until none remain; validate on crafted files. |
| Q16 | **`brainUpkeepCost` units**: the property is a **per-second** rate but is read as `cost` in UI strings and charged every tick as `cost*dt`; egg energy uses `brainUpkeepCost * SizeRatio` (not ×time) as the energetic cost of the child brain. | `NEATBrain.cs:283`; `BibiteBody.cs:603`; `BibiteEggLayingOrgan.cs:107` | Keep per-second rate in `BrainUpkeep` (body multiplies by settings and dt); egg cost = `upkeep_cost_per_sec * SizeRatio` exactly as `:107`. |
| Q17 | **`CopyGene2` / `deuxParent`** (two-parent sexual reproduction, chi-squared crossover of genes `BibiteGenes.cs:336-362,446-449`) is dead code in 0.6.3.1 (no caller found). | grep shows only definitions | Do not implement for M1; keep gene serialization flag `deuxParent` ignored/round-tripped if present. |
| Q18 | **`TypeIsNonLinear`/`GetRandomNodeIndex`/`DefaultBaseActivationOfType`** are UI/editor helpers only; `DefaultBaseActivationOfType` is *not* used by mutation (which rolls random biases for Inhibitory/SoftLatch). | `NEATBrain.cs:303-310,1078-1092`; users in `Here/UIScripts/*` | Ignore for simulation; use the mutation rolls of §8.6. |

---

## 13. Cross-check of `bibites_brain_reference (4).md`

The auxiliary reference was read in full and compared to source. Trustworthy: brain/inputs/outputs overview,
activation-type table, node/synapse JSON field lists, "Basic bibite" topology (3 synapses with those weights),
speciation summary, mutation-kernel description, warning that `.bb8`/`.bb8template` are different formats.

Corrections (do not copy from the reference):

| # | Reference claim | Source truth |
|---|---|---|
| 1 | `simTPS` default 40; brain ~20 Hz | **30 TPS** default; brain 15 Hz (`ScenarioIndependentSettings.cs:170-192`, real settings files) |
| 2 | Integrator `y = y + x/TPS`, period 0.05 s | period = `brain_tps/sim_tps` = **2/30 s** (a ratio; also passed to Differential/Inhibitory) |
| 3 | Hidden-node count "13 types" with a duplicated SoftLatch row | 13 non-input `NodeType` values (1..13); SoftLatch appears twice in the reference table |
| 4 | Gene defaults (e.g. `SpeedRatio=1.0`, `ViewAngle=180`, `ViewRadius=75`, `PheroSense=160`, `Diet=0.3`, `MutationAmountSigma=0.025`, `AverageMutationNumber=4`, `BrainAverageMutation=2`, `GrowthScale=0.35`, WAG 5/2.5/2.5/…, herd cohesion 1.0) | These are the "Basic bibite" **template values of an older era**, not editor defaults. Current defaults are in §7.1 (`SpeedRatio=0.75`, `ViewAngle=75`, `ViewRadius=180`, `PheroSense=150`, `Diet=0.2`, `MutationAmountSigma=0.05`, `AverageMutationNumber=2`, `BrainAverageMutation=1`, `GrowthScale=0.15`, WAGs 2/1/1/0.15/1/1/1, herd 1/1.5/0.75/0.25/35). |
| 5 | "Herding output is disabled — ignore it" | Outdated: 0.6.3.1 has `disableHerding` default **false** and a working herding blend (`BibitePropulsion.cs:263-271`); only the scenario flag disables it. |
| 6 | `IsGrabbing` is 1.0/0.0 | It is `mouth.nHeld` (0..10, an integer count) (`NEATBrain.cs:582`) |
| 7 | Angles: − = left, + = right; RotationSpeed + = clockwise | Mirrored tooltip labels; the operative math is +SignedAngle = CCW (left), positive Rotate = CCW (see Q1). Reference did not check the starter-genome consistency. |
| 8 | Vision weight formula (quadratic falloff, size term, angle term) | Correct, but reference omits: bodies are size-scaled (`viewRadius*growth`), corpses appear in **both** meat and bibite channels, dead bibites' colour genes are readable, held-pellet side effect. |
| 9 | "Maturity 0..∞" / "1.0 = reproduction" | Input is raw `growth`; the ratio growth/growthAtMature is what reaches 1 at maturity. |
| 10 | "Version 0.6 templates had 32 inputs (RotationSpeed added 0.6.2 alpha)" | Correct; per-era maps in §10.6 add the Constant/InfectionRate eras and the migration anomaly. |
| 11 | Mutation mechanics summary | Correct in substance; add: count/delta use System.Random while picks use Unity RNG; brain vs gene branches; the cumulative-subtraction branch quirks; easter-egg roll; `mutationIncreases` not persisted. |
| 12 | Speciation summary | Correct in substance; exact metric/assignment in §9; add `isMutant` two-step rule, `activeSpecies` scan order, egg adoption path, `usedSpeciesSpan` formula, pruning. |
| 13 | File format section says `.bb8template` has `name/speciesName/description/generation/version/nodes/synapses/genes` | Correct; `.bb8` is different (nested genes + `brain:{isReady,Nodes,Synapses}` + extra sections); §10.1/§10.2 have the exact shapes and per-field persistence. |

---

## Appendix A — quick numbers

- `N_INPUTS=33`, `N_OUTPUTS=15`, base node count 48, first hidden index 48, `NNodesTypes=14`,
  `nodeMaxInov` initial 49 (first hidden Inov 50), `connectionMaxInov` initial 1.
- Output clamp ±1024. `brainPeriod = brain_tps/sim_tps` (default 2/30 ≈ 0.0667).
- Brain cadence 15 Hz default; vision list 10 brain ticks, vision values 5 brain ticks, phero 1 sim tick (1 after
  brain), clock 1 sim tick, growth update period `0.2/growthRate` s.
- Thresholds: `Want2Lay >= 0.25`; `ClkReset > 0.75`; `Want2Eat`, `Grab` dead zone `abs(x) <= 0.15`;
  `Want2Attack > 0.15`; egg production dead zone `abs(x) < 0.15`; phero spawn if `R+G+B > 0.05`.
- Upkeep defaults: neuron 0.001 E/s (hidden only), synapse 0.00025 E/s.
- Mutation λs: gene `genes[9]+1.5+Δ`, brain `genes[11]+1.5+Δ`; σs: gene `genes[8]+0.05+Δ`, brain
  `genes[10]+0.05+Δ`; relative share 0.75 (1.0 for `isPurelyRelative` genes).
- Species span: `1.5 + max(active-20,0)*0.05`; gene distance normalised by editor **max**; brain distance terms
  §9.1; child adoption within `span/2`.
- Body-derived constants used by brain coupling: `baseBodyArea=80*d2Size`, `bodyLength=12.5*d1Size`,
  `maxHealth=1.25*baseBodyArea`, `Energy.MaxAmount=1.5*baseBodyArea`, `metabolicEfficiency=0.8^SpeedRatio`.

## Appendix B — fixture inventory (gitignored `testdata/real/`)

| Path | Version | Purpose |
|---|---|---|
| `templates/5_Basic_bibite_0.6.bb8template` (+ 13 more: `Parasite`, `Whale`, `Seeker shooter`, `Vulture`, `Scorpion`, `Fibonacci turner` 0.6a5, `Squalus nova`, `Luscus xhybridus` 0.5, `Nigel` 0.5, `Camel`, `Formica bestiola` 0.6a16, `Easy herbivore`, `Darth bibitus` 0.6.1a4) | legacy | migration tests: every pre-current era |
| `templates_current/Basic_bibite_0.6.3.1.bb8template` | 0.6.3.1 | `Genome::starter`/`Brain::starter` golden |
| `templates_current/Explanatory Bibite.bb8template` | 0.6.3.1 | all 13 activation types, small (61 nodes) |
| `templates_current/Nestegg.bb8template`, `Darwin's Demon.simplified.bb8template`, `Arborea_magna.bb8template` | 0.6.3.1 | richer topologies (up to 168 nodes) |
| `bibites/Spiller 1.bb8` | 0.6.3.1 | runtime `.bb8` round-trip (state fields, `archetype`) |
| `bibites/Lunaloutrae rosalynnae.bb8` | 0.6.0.1 | legacy-version migration + old node records (no `archetype`) |
| `speciesData.json` (Lead-provided) | 0.6.3.1 | species lineage JSON schema |
| `*settings` / `*.zip` (Lead-provided) | mixed | settings spot-checks (`simTPS` overrides, e.g. Ring.zip = 15) |

Reproduce extraction: the templates in `templates/` were carved out of
`C:\Program Files (x86)\Steam\steamapps\common\The Bibites\The Bibites_Data\resources.assets` (embedded sample
scenario zips); `templates_current/` and `bibites/` come from the user data folder
`C:\Users\Storme\AppData\LocalLow\The Bibites\The Bibites\`. All are read-only source material; nothing is
committed to git (`testdata/` is gitignored).

## Appendix C — primary source files

`Here/SimulationScripts.BibiteScripts/NEATBrain.cs`, `BibiteGenes.cs`, `Species.cs`, `BrainDifferences.cs`,
`ActivationDelta.cs`, `ConnectionDelta.cs`, `BaseGeneticDesc.cs`, `SelfGeneticDesc.cs`, `SpeciesGeneticDesc.cs`,
`MutationIncreases.cs`, `BibiteBody.cs`, `BibiteGrowth.cs`, `InternalClock.cs`, `Pherosense.cs`, `FieldOfView.cs`,
`BibiteStomach.cs`, `BibiteEggLayingOrgan.cs`, `BibiteMouth.cs`, `BibitePropulsion.cs`,
`Here/ScriptHelpers/BrainUpdater.cs`, `BibiteUpdater.cs`, `VersionBrainChange.cs`, `NodeChanges.cs`,
`RandomExtension.cs`, `SerializationHelper.cs`, `BrainSorter.cs` (UI), `Here/SettingScripts/*Settings.cs`,
`Here/Utility/BibiteTemplate.cs`, `Here/Utility/Version.cs`, `Here/Utility/InformationWrapers/NodeInformations.cs`,
`Here/ManagementScripts/SaveSystem.cs`, `GlobalLineageManager.cs`, `TimeController.cs`,
`Here/ManagementScripts/WorldObjectsSpawner.cs`, `Here/SimulationScripts/MatterPellet.cs`.




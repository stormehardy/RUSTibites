# braincheck — independent cross-language brain evaluator check

`braincheck` is a **second, independent implementation** of the Bibites brain step
(in Python, written from `docs/spec/01-brain-genetics.md`), plus a bitwise f32
comparator. It exists to catch same-author / same-code bugs in
`crates/bibites-core/src/brain.rs` (`Brain::step`).

It is **validation only**: it is not part of the simulation, it has no game/API
access, it uses only the Python standard library, and it is deterministic.

```
tools/braincheck/
  brain_ref.py        independent Python reference interpreter + CLI
  run_check.py        bitwise comparator (+ --self-test with hand-computed cases)
  make_fixtures.py    deterministic fixture generator (reproduces committed fixtures)
  fixtures/           starter_brain.json, random_stateful_brain.json, random_small_brain.json
  README.md           this file
```

## Quick start

```powershell
# Python 3.11, stdlib only
python tools/braincheck/run_check.py --self-test --verbose

# Reference output for a fixture (independent implementation):
python tools/braincheck/brain_ref.py tools/braincheck/fixtures/starter_brain.json -o ref.json

# Rust output emitted by the CLI (Lead-wired; see JSON contract below):
rustibites braincheck tools/braincheck/fixtures/starter_brain.json -o rust.json

# Bitwise comparison (exit 0 = pass, 1 = divergence):
python tools/braincheck/run_check.py tools/braincheck/fixtures/starter_brain.json rust.json
```

Options: `--ulp-tolerance N` (default `0` = strict bitwise; see the double-math
note below), `--verbose`, `--json` (reserved), `--self-test`.

`make_fixtures.py` regenerates `fixtures/` byte-identically (fixed seeds; no
wall-clock/OS randomness). Nothing here contains game assets or real saves.

## JSON contract

### Fixture (input, committed under `fixtures/`)

```json
{
  "period": 0.06666667,
  "herding_enabled": true,
  "nodes": [
    {"Type": 0, "Index": 0, "Inov": 1, "baseActivation": 0.0,
     "Value": 0.0, "LastInput": 0.0, "LastOutput": 0.0},
    {"Type": 3, "Index": 33, "Inov": 34, "baseActivation": 0.4535929,
     "Value": 0.42506, "LastInput": 0.4535929, "LastOutput": 0.42506}
  ],
  "synapses": [
    {"Inov": 1, "NodeIn": 3, "NodeOut": 39, "Weight": 4.0729575, "En": true}
  ],
  "input_sequences": [
    [[0.0, 0.0, "... 33 floats ..."], ["... next tick ..."]],
    [[0.0, "... second run ..."]]
  ]
}
```

* `nodes` — array order **is** the node index; `Index` must equal the position.
  Layout: `0..=32` inputs, `33..=47` outputs, `>=48` hidden (spec §1).
  `Type` is the game's `NEATBrain.NodeType` integer (`0 Input … 13 SoftLatch`).
  `Value`/`LastInput`/`LastOutput` are the complete initial runtime state.
* `synapses` — `En` may be false (disabled synapses do not transmit but still
  exist); the array order is meaningful (fan-in summation order, spec §6.1).
* `period` — brain period passed to stateful activations:
  `brain_tps / sim_tps` (default `2/30`), not a wall-clock second count.
* `herding_enabled` — optional, default `true`. When false, synapses whose
  `NodeOut == 35` (Herding output) are skipped in execution, matching
  `disableHerding` (spec §3.3/§6.1).
* `input_sequences` — each entry is an independent run starting from the
  fixture's initial state. Each tick is exactly 33 f32 values in `Input` enum
  order. The reference evaluates every sequence from the same initial state.

**Fixture initial state convention:** committed fixtures are seeded exactly like
the game's `FinalizeEditing` (every non-input node executed once with
`input = 0`, so e.g. a Sigmoid output with bias 0 starts at 0.5). Fixtures
contain no "floating" hidden nodes, so no repair/renumbering is required.

### Output (reference and Rust must both emit this shape)

```json
{
  "period": 0.06666667,
  "herding_enabled": true,
  "double_math": "unity",              // diagnostic only; ignored by comparator
  "tick_count": 28,
  "ticks": [
    {
      "sequence": 0,
      "tick": 0,                       // tick index within its sequence
      "inputs": [33 floats],           // echo of the input vector used
      "outputs": [15 floats],          // output-node Values, Output enum order
      "nodes": [
        {"Index": 0, "Type": 0, "Value": 0.0, "LastInput": 0.0, "LastOutput": 0.0}
      ]
    }
  ]
}
```

* All `nodes` entries of the fixture must be present, in array order.
* Floats are emitted as normal JSON numbers; Rust `f32` serialized through
  `serde_json` round-trips to the same `f32` bits (comparator re-quantises to
  binary32 before comparing, so `0.1f32` as `0.10000000149011612` is fine).
* `sequence`/`tick` are informational for diagnostics; the comparator compares
  ticks in order and reports these values on divergence.
* `double_math` (formerly `libm`) is ignored by the comparator.

### Rust emitter requirements (Lead-wired CLI)

1. Load the fixture with **runtime-state semantics**: construct nodes/synapses as
   `brain::NodeRecord`/`SynapseRecord`, then `Brain::from_records(&nodes, &synapses)`
   (state honoured). Do **not** use `from_template_records` (it zeroes state and
   re-initialises with bias-only execution; that would re-seed the fixture).
2. Apply `herding_enabled` from the fixture (default true):
   `brain.set_herding_enabled(fixture.herding_enabled); brain.rebuild();`
   (safe/no-op if it equals the default).
3. For each sequence, **re-load or clone the initial brain** (sequences are
   independent runs), then for each tick vector:
   `let outputs = brain.step(&inputs, fixture.period);`
   and emit `inputs`, `outputs` (`brain.output_value(o)` for all 15 `Output`s)
   and for each node `brain.node_state(i)` (`Index`, `Type = brain.node_type(i)`,
   `Value`, `LastInput`, `LastOutput`).
4. Suggested CLI: `rustibites braincheck <fixture.json> -o <output.json>`
   (mirrors `brain_ref.py`). The check command sweeps `fixtures/*.json` when the
   fixture argument is a directory (optional convenience).

Any discrepancy here is either a `Brain::step` bug or a fixture/contract
mismatch — which is exactly what the harness is for.

## What is compared

Per tick, per sequence, bitwise over the f32 bit patterns:

* the 15 output-node Values;
* every node's `Value`, `LastInput`, `LastOutput`;
* (when the Rust output includes it) the echoed 33 inputs.

`--ulp-tolerance 0` (default) requires identical bits. Divergences report
sequence/tick/scope/index/field, both values, both bit patterns and the ULP
distance; the first divergence is printed first, exit code 1.

## Why bitwise comparison is valid here

The simulation is `f32`; `brain_ref.py` rounds **every** arithmetic operation to
binary32 (`struct.pack('<f', ...)`). Activation maths is special: Unity/Mono
`Mathf.*` wrappers and `Math.Tanh` evaluate in **`double`** and cast back to
`f32` (e.g. `TanH => (float)Math.Tanh(input)`; `Sigmoid => 1f / (1f +
Mathf.Pow((float)Math.E, 0f - input))`; `Inhibitory/SoftLatch` use
`Mathf.Exp`). `brain_ref.py` mirrors that exactly using Python's `math` (the
same platform libm Rust's `f64::tanh`/`f64::exp`/`f64::powf` call), and Rust's
`math::{tanh,sigmoid,sin,exp,gaussian}` do the same. This matters: an `f32`
`tanhf` differs by 1 ULP (e.g. `tanh(0.4535929f32)`: double path `0x3ed985aa`
vs `tanhf` `0x3ed985a9`), which was the first real game-vs-Rust brain output
divergence (`Accelerate`).

Bits are compared bitwise over the f32 patterns, so agreement requires identical
double-math rounding too. On a platform whose libm differs from the game's
(Windows/UCRT is what the game and captures use), transcendental-only 1-ULP
noise is possible; use `--ulp-tolerance 2` only for such environments, never for
validating activation-formula changes.

## Self-test coverage (`run_check.py --self-test`)

Hand-computed expectations (starter values from the captured 0.6.3.1 template):

1. starter brain bias-only output vector (TanH/Sigmoid/ReLU resting values);
2. one brain-tick input lag + starter reflex arcs
   (`PlantAngle→Rotate`, `PlantCloseness→Accelerate`, `Fullness→Digestion`);
3. `Latch` hold between thresholds and inclusive `x >= 1` / `x <= 0`;
4. `Differential` first-step spike (`LastInput` initialisation) and zero
   derivative afterwards;
5. `Integrator` accumulation and ±1024 clamp;
6. `Inhibitory` decay `x - lastInput + lastOut·e^(−bias·period)`;
7. `SoftLatch` hysteresis branches (growth branch, hard 1 at `x >= 0.999`,
   hold branch, hard 0 at `x <= 0.001`);
8. `Mult` product with bias as an extra factor;
9. disabled synapses are skipped;
10. comparator positive test (identical → pass) and negative tests:
    1-ULP output corruption and node-state corruption are both detected;
11. determinism (two reference evaluations are identical);
12. `parity_common.open_capture` reader regression: a Rust-style capture with
    plain `ticks.jsonl`/`events.jsonl` opens and reproduces the byte-exact
    record digests, gzip-only captures still open, and gzip is preferred when
    both formats are present (skipped when `tools/parity/` is unavailable).

## Fixtures

| File | Nodes / synapses | Runs × ticks | Size | Purpose |
|---|---|---|---|---|
| `fixtures/starter_brain.json` | 48 / 3 | 3 × (8,10,10) | ~10 KB | official Basic bibite brain; zeros / plant-phased / feeding sequences |
| `fixtures/random_stateful_brain.json` | 61 / 50 | 3 × 10 | ~30 KB | 48 base + 13 hidden nodes covering **every** non-input activation type (all stateful ones: Latch, Differential, Integrator, Inhibitory, SoftLatch, plus Mult/Sine/Abs/…), random topology incl. disabled synapses and hidden→hidden edges |
| `fixtures/random_small_brain.json` | 50 / 9 | 2 × 8 | ~13 KB | 48 base + 2 stateful hidden nodes (Latch, Integrator), random topology |

All fixtures are synthetic (generated by `make_fixtures.py` with fixed seeds);
no game assets, saves or copyrighted data are committed.

## Assumptions / ambiguities (kept in sync with spec §12)

1. **Fixture state semantics** — output comparison starts at tick 0 from the
   fixture state; there is no separate start-state assertion. The Rust emitter
   must therefore use state-honouring loading (`from_records`), per contract.
2. **Summation order** — the reference groups by destination in first-appearance
   order and sums incoming synapses in synapse-array order, exactly as
   `NEATBrain.Thinker`/spec §6.1. A Rust change to a different summation order
   would be a real behavioural divergence and is intentionally flagged.
3. **Nodes never executed** (no enabled incoming synapses) keep their prior
   `Value`/`LastInput`/`LastOutput`; the reference does not re-run them.
4. **SoftLatch expression order** — the reference assumes the C# evaluation order
   `last_output / (1 - exp(-b·li)) * (1 - exp(-b·x))`; an f32 associativity
   difference in the Rust port is caught by the check.
5. **Herding laziness** — only `herding_enabled` is modelled. The game's
   `StartUsedSystems` sense masks do not affect `step` and are covered by
   brain.rs unit tests instead.
6. **No Inov/Index validation beyond layout** — `Inov` is carried but not used by
   execution; duplicate or unordered innovations do not change results.
7. **`period`** is passed through as an f32 value; the harness does not derive it
   from settings (fixtures fix it so old/new defaults cannot silently differ).
8. **Transcendental precision** — the game computes `Mathf.*`/`Math.Tanh` in
   `double` and casts to `f32`; both Rust and the Python reference mirror that.
   On a platform whose libm differs from the game's, use `--ulp-tolerance 2` and
   treat transcendental-only 1-ULP noise separately.

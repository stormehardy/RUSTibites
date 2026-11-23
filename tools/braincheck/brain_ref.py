#!/usr/bin/env python3
"""Independent Python reference interpreter for The Bibites brain.

This is a SECOND implementation of `Brain::step`, written directly from
`docs/spec/01-brain-genetics.md` (§2 activations, §3 scheduling, §4 senses,
§6 execution) and the decompiled game (`NEATBrain.cs`). It is deliberately NOT
a port of `crates/bibites-core/src/brain.rs`: it shares no code, so agreement
between the two catches same-author/same-code bugs.

Scope: one brain's evaluation over a sequence of brain-tick input vectors.
It does not model senses, bodies or the world; fixtures provide the 33 input
values per tick and the initial node state.

Arithmetic: the simulation is `f32`. Every arithmetic operation here is rounded
to `f32` (via `struct`), including intermediate sums/products. The game's
activation maths uses Unity/Mono `Mathf.*`/`Math.Tanh`, which evaluate in
`double` and cast the result back to `f32`; this module mirrors that with the
platform `math` library (shared libm), so results can be compared bitwise. The
old `f32` transcendental variants (`tanhf` etc.) are 1 ULP off in places — see
`README.md`.

Stdlib only; deterministic; no game/API access.

Fixture JSON (input):
  {
    "period": 0.06666667,                 # brain_period = brain_tps/sim_tps
    "herding_enabled": true,              # optional, default true
    "nodes": [                            # array order == node index
      {"Type": 0, "Index": 0, "Inov": 1, "Desc": "EnergyRatio",
       "baseActivation": 0.0,
       "Value": 0.0, "LastInput": 0.0, "LastOutput": 0.0}, ...
    ],
    "synapses": [
      {"Inov": 1, "NodeIn": 3, "NodeOut": 39, "Weight": 4.0729575, "En": true}, ...
    ],
    "input_sequences": [                  # one entry per run; each is a list of
      [[0.0, ... 33 floats ...], ...],    # 33-float brain-tick input vectors
      ...
    ]
  }

Output JSON (this module / Rust emitter):
  {
    "period": 0.06666667,
    "herding_enabled": true,
    "double_math": "unity",               # diagnostic only (game/Mono semantics)
    "ticks": [
      {"tick": 0,
       "inputs": [33 floats],
       "outputs": [15 floats],
       "nodes": [{"Index":0,"Type":0,"Value":...,"LastInput":...,"LastOutput":...}, ...]},
      ...
    ]
  }

`run_check.py` compares the Rust emitter's output against this reference
bitwise (f32 bits) per tick, per output slot and per node state field.
"""

from __future__ import annotations

import argparse
import json
import math
import struct
import sys
from typing import Any, Dict, List, Optional, Sequence, Tuple

N_INPUTS = 33
N_OUTPUTS = 15
FIRST_OUTPUT = N_INPUTS
FIRST_HIDDEN = N_INPUTS + N_OUTPUTS
HERDING_OUTPUT_NODE = FIRST_OUTPUT + 2
HERDING_SKIPPED_NODE = 2  # NEATBrain.cs:515 compares NodeOut with the literal 2
OUTPUT_CLAMP = 1024.0

# NodeType integers (NEATBrain.cs:70-86; spec §1.1).
INPUT = 0
SIGMOID = 1
LINEAR = 2
TANH = 3
SINE = 4
RELU = 5
GAUSSIAN = 6
LATCH = 7
DIFFERENTIAL = 8
ABS = 9
MULT = 10
INTEGRATOR = 11
INHIBITORY = 12
SOFT_LATCH = 13
NODE_TYPE_COUNT = 14

NODE_TYPE_NAMES = {
    INPUT: "Input",
    SIGMOID: "Sigmoid",
    LINEAR: "Linear",
    TANH: "TanH",
    SINE: "Sine",
    RELU: "ReLu",
    GAUSSIAN: "Gaussian",
    LATCH: "Latch",
    DIFFERENTIAL: "Differential",
    ABS: "Abs",
    MULT: "Mult",
    INTEGRATOR: "Integrator",
    INHIBITORY: "Inhibitory",
    SOFT_LATCH: "SoftLatch",
}


# ---------------------------------------------------------------------------
# f32 arithmetic helpers
# ---------------------------------------------------------------------------

_F32 = struct.Struct("<f")
_F32_BITS = struct.Struct("<I")


def f32(value: float) -> float:
    """Rounds a Python float to the nearest IEEE-754 binary32 value."""
    return _F32.unpack(_F32.pack(value))[0]


def f32_bits(value: float) -> int:
    """Bit pattern of `value` rounded to binary32."""
    return _F32_BITS.unpack(_F32.pack(value))[0]


def exp32(x: float) -> float:
    """Unity `Mathf.Exp`: `(f32)math.exp((double)x)`."""
    return f32(math.exp(f32(x)))


def tanh32(x: float) -> float:
    """Game `TanH`: `(f32)math.tanh((double)x)` (not an f32 tanh)."""
    return f32(math.tanh(f32(x)))


def sin32(x: float) -> float:
    """Unity `Mathf.Sin`: `(f32)math.sin((double)x)`."""
    return f32(math.sin(f32(x)))


def pow32(base: float, exponent: float) -> float:
    """Unity `Mathf.Pow`: `(f32)math.pow((double)b, (double)e)`."""
    return f32(math.pow(f32(base), f32(exponent)))


# Unity/Mono evaluates Mathf.Pow/Exp/Sin and Math.Tanh in `double` and casts
# back to f32. Python's `math` module calls the same platform libm, so this
# reproduces the game bit-for-bit (one notable effect: tanh(0.4535929f32) is
# 0x3ed985aa here, while an f32 `tanhf` gives 0x3ed985a9).
#
# More than that, Mono does the arithmetic of a whole C# expression in `double`
# and rounds to `float` only where a value is STORED (a variable, a field, an
# array element), PASSED to a method or RETURNED. Measured 2026-11-19 against the
# original game (tools/braincheck/game_diff, 9,600 brain ticks, every node type):
# written that way Sigmoid, Gaussian, Integrator, Inhibitory and SoftLatch match
# the game bit for bit; with an f32 step per operation 0.6 to 32 % of their nodes
# were one ulp off, and the nodes after them carried it on.
UNITY_DOUBLE_MATH = True


def clamp(value: float, low: float, high: float) -> float:
    """`math::clamp` semantics (`if x < min ... else if x > max ... else x`)."""
    if value < low:
        return low
    if value > high:
        return high
    return value


# ---------------------------------------------------------------------------
# Activation functions (spec §2)
# ---------------------------------------------------------------------------

def has_bias(kind: int) -> bool:
    """`TypeHasBias`: false for Input and Integrator (NEATBrain.cs:285-292)."""
    return kind not in (INPUT, INTEGRATOR)


def uses_bias_differently(kind: int) -> bool:
    """`TypeUsesBiasDifferently`: Integrator/Mult/Inhibitory/SoftLatch."""
    return kind in (INTEGRATOR, MULT, INHIBITORY, SOFT_LATCH)


def sigmoid(x: float) -> float:
    """Game sigmoid: `1f / (1f + Mathf.Pow((float)Math.E, 0f - input))`.

    `Mathf.Pow` returns a float (a method result); the `1 + p` and the
    reciprocal are ONE double expression, rounded once. The `double` power
    saturates before the f32 cast, so this is also overflow-safe.
    """
    x = f32(x)
    e = f32(math.e)  # `(float)Math.E`
    return f32(1.0 / (1.0 + pow32(e, f32(-x))))


def relu(x: float) -> float:
    """`math::relu`: `if x > 0 { x } else { 0 }` (NaN and -0.0 → 0)."""
    x = f32(x)
    return x if x > 0.0 else 0.0


def gaussian(x: float) -> float:
    """Game Gaussian: `1f / (1f + Mathf.Pow(input, 2f))`."""
    x = f32(x)
    return f32(1.0 / (1.0 + pow32(x, 2.0)))


def soft_latch(bias: float, x: float, last_output: float, last_input: float) -> float:
    """`NEATBrain.SoftLatchFunc` (NEATBrain.cs:1105-1133), exact branches.

    The game's evaluation: each `Mathf.Exp` call takes its argument as a float
    and returns a float, `num3` is a float local, and the rest of every
    `return` expression is ONE double expression (see UNITY_DOUBLE_MATH).
    """
    bias = f32(bias)
    x = f32(x)
    if x >= f32(0.999):
        return 1.0
    if x <= f32(0.001):
        return 0.0
    li = f32(last_input)
    if li >= f32(0.999):
        li = 1.0
    if li <= f32(0.001):
        li = 0.0
    last_output = f32(last_output)
    if x < li:
        a = exp32(f32(-bias * li))
        c = exp32(f32(-bias * x))
        return f32(last_output / (1.0 - a) * (1.0 - c))
    e1 = exp32(f32(bias * (li - 1.0)))
    num3 = f32((last_output - e1) / (1.0 - e1))  # a float local
    e2 = exp32(f32(bias * (x - 1.0)))
    return f32(num3 + (1.0 - num3) * e2)


def activate(
    kind: int,
    bias: float,
    raw_input: float,
    last_output: float,
    last_input: float,
    period: float,
) -> Tuple[float, float]:
    """One `ExecuteNeuron` call (NEATBrain.cs:612-641).

    Returns `(output, last_input)`; `output` is clamped to ±1024.
    `last_input` is the full activation `x` (after bias handling).
    """
    raw_input = f32(raw_input)
    bias = f32(bias)
    period = f32(period)
    if uses_bias_differently(kind):
        if kind == MULT:
            x = f32(raw_input * bias)
        else:
            x = raw_input
    else:  # `if (!TypeUsesBiasDifferently(Type)) input += baseActivation;` (the Input type too)
        x = f32(raw_input + bias)

    if kind == INPUT or kind == LINEAR or kind == MULT:
        out = x
    elif kind == SIGMOID:
        out = sigmoid(x)
    elif kind == TANH:
        out = tanh32(x)
    elif kind == SINE:
        out = sin32(x)
    elif kind == RELU:
        out = relu(x)
    elif kind == GAUSSIAN:
        out = gaussian(x)
    elif kind == LATCH:
        if x >= 1.0:
            out = 1.0
        elif x <= 0.0:
            out = 0.0
        else:
            out = f32(last_output)
    elif kind == DIFFERENTIAL:
        out = f32((x - f32(last_input)) / period)
    elif kind == ABS:
        out = f32(abs(x))
    elif kind == INTEGRATOR:
        out = f32(f32(last_output) + x * period)
    elif kind == INHIBITORY:
        decay = exp32(f32(f32(-bias) * period))
        out = f32((x - f32(last_input)) + f32(last_output) * decay)
    elif kind == SOFT_LATCH:
        out = soft_latch(bias, x, f32(last_output), f32(last_input))
    else:
        raise ValueError(f"unknown node type {kind}")
    return clamp(f32(out), -OUTPUT_CLAMP, OUTPUT_CLAMP), x


# ---------------------------------------------------------------------------
# Brain evaluation
# ---------------------------------------------------------------------------

def validate_brain(nodes: Sequence[Dict[str, Any]], synapses: Sequence[Dict[str, Any]]) -> None:
    if len(nodes) < FIRST_HIDDEN:
        raise ValueError(f"fixture has {len(nodes)} nodes; need at least {FIRST_HIDDEN}")
    for i, node in enumerate(nodes):
        kind = int(node["Type"])
        if kind < 0 or kind >= NODE_TYPE_COUNT:
            raise ValueError(f"node {i}: bad Type {kind}")
        if "Index" in node and int(node["Index"]) != i:
            raise ValueError(
                f"node {i}: Index {node['Index']} does not match array position"
            )
        if i < N_INPUTS and kind != INPUT:
            raise ValueError(f"node {i}: input-range node has Type {kind}")
        if FIRST_OUTPUT <= i < FIRST_HIDDEN and kind == INPUT:
            raise ValueError(f"node {i}: output-range node has Input activation")
    for s, syn in enumerate(synapses):
        node_in = int(syn["NodeIn"])
        node_out = int(syn["NodeOut"])
        if not (0 <= node_in < len(nodes) and 0 <= node_out < len(nodes)):
            raise ValueError(f"synapse {s}: endpoints ({node_in}->{node_out}) out of range")
        f32(float(syn["Weight"]))
    return None


def initialize_states(nodes: List[Dict[str, Any]], period: float) -> None:
    """Game `FinalizeEditing` state seeding for a freshly built brain.

    Every non-input node is executed once with `input = 0` and `period`;
    inputs stay at zero. Mutates the node dictionaries in place.
    """
    for i, node in enumerate(nodes):
        if i < N_INPUTS:
            node["Value"] = 0.0
            node["LastInput"] = 0.0
            node["LastOutput"] = 0.0
        else:
            out, last_in = activate(
                int(node["Type"]),
                float(node["baseActivation"]),
                0.0,
                0.0,
                0.0,
                period,
            )
            node["Value"] = out
            node["LastInput"] = last_in
            node["LastOutput"] = out


def _groups(
    nodes: Sequence[Dict[str, Any]],
    synapses: Sequence[Dict[str, Any]],
    herding_enabled: bool,
) -> List[Tuple[int, List[int]]]:
    """Destination groups, in first-appearance order (game `Thinker`).

    Returns `(target_node, [incoming synapse indices in synapse-array order])`.
    """
    _ = nodes
    slot: Dict[int, int] = {}
    groups: List[Tuple[int, List[int]]] = []
    for s, syn in enumerate(synapses):
        if not bool(syn["En"]):
            continue
        node_out = int(syn["NodeOut"])
        # The game's `herdingEnabled || Synapses[i].NodeOut != 2`: the literal 2 (an input node), not the Herding output (node 35), so with herding off
        # the Herding output is still computed (measured against the game 2026-11-19).
        if not herding_enabled and node_out == HERDING_SKIPPED_NODE:
            continue
        group = slot.get(node_out)
        if group is None:
            slot[node_out] = len(groups)
            groups.append((node_out, [s]))
        else:
            groups[group][1].append(s)
    return groups


def evaluate(fixture: Dict[str, Any]) -> Dict[str, Any]:
    """Runs the fixture's input sequences and returns the output JSON object."""
    period = f32(float(fixture["period"]))
    if period <= 0.0:
        raise ValueError("period must be positive")
    herding_enabled = bool(fixture.get("herding_enabled", True))
    nodes: List[Dict[str, Any]] = [dict(node) for node in fixture["nodes"]]
    synapses: List[Dict[str, Any]] = [dict(syn) for syn in fixture["synapses"]]
    validate_brain(nodes, synapses)
    groups = _groups(nodes, synapses, herding_enabled)

    sequences = fixture.get("input_sequences")
    if not sequences:
        sequences = [[[0.0] * N_INPUTS]]
    for seq in sequences:
        for tick_inputs in seq:
            if len(tick_inputs) != N_INPUTS:
                raise ValueError(
                    f"input vector has {len(tick_inputs)} values; expected {N_INPUTS}"
                )

    result: List[Dict[str, Any]] = []
    for seq_index, sequence in enumerate(sequences):
        value = [f32(float(node.get("Value", 0.0))) for node in nodes]
        last_out = [f32(float(node.get("LastOutput", 0.0))) for node in nodes]
        last_in = [f32(float(node.get("LastInput", 0.0))) for node in nodes]
        for tick, tick_inputs in enumerate(sequence):
            # UpdateSenses: previous values become readable, then are replaced.
            for i in range(N_INPUTS):
                last_out[i] = value[i]
                value[i] = f32(float(tick_inputs[i]))
            for target, syn_indices in groups:
                kind = int(nodes[target]["Type"])
                acc = 1.0 if kind == MULT else 0.0
                for s in syn_indices:
                    node_in = int(synapses[s]["NodeIn"])
                    term = f32(f32(float(synapses[s]["Weight"])) * last_out[node_in])
                    acc = f32(acc * term) if kind == MULT else f32(acc + term)
                out, new_last_in = activate(
                    kind,
                    float(nodes[target]["baseActivation"]),
                    acc,
                    last_out[target],
                    last_in[target],
                    period,
                )
                value[target] = out
                last_out[target] = out
                last_in[target] = new_last_in
            result.append(
                {
                    "sequence": seq_index,
                    "tick": tick,
                    "inputs": [f32(float(v)) for v in tick_inputs],
                    "outputs": [value[FIRST_OUTPUT + i] for i in range(N_OUTPUTS)],
                    "nodes": [
                        {
                            "Index": i,
                            "Type": int(nodes[i]["Type"]),
                            "Value": value[i],
                            "LastInput": last_in[i],
                            "LastOutput": last_out[i],
                        }
                        for i in range(len(nodes))
                    ],
                }
            )
    return {
        "period": period,
        "herding_enabled": herding_enabled,
        "double_math": "unity",
        "tick_count": len(result),
        "ticks": result,
    }


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture", help="brain fixture JSON path")
    parser.add_argument(
        "-o", "--output", help="write the reference output JSON here (default stdout)"
    )
    parser.add_argument("--pretty", action="store_true", help="indent output JSON")
    args = parser.parse_args(argv)
    with open(args.fixture, "r", encoding="utf-8-sig") as handle:
        fixture = json.load(handle)
    output = evaluate(fixture)
    text = json.dumps(output, indent=2 if args.pretty else None, sort_keys=True)
    if args.output:
        with open(args.output, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(text)
            handle.write("\n")
    else:
        sys.stdout.write(text)
        sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

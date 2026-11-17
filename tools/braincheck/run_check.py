#!/usr/bin/env python3
"""Bitwise cross-language check for the Bibites brain.

Usage:
  # 1. Reference output (Python, independent implementation):
  python brain_ref.py fixtures/starter_brain.json -o /tmp/ref.json

  # 2. Rust output (emitted by `rustibites braincheck`, see README):
  rustibites braincheck fixtures/starter_brain.json -o /tmp/rust.json

  # 3. Compare bitwise:
  python run_check.py fixtures/starter_brain.json /tmp/rust.json

Exit code 0 = bitwise identical, 1 = divergence or error.

The comparator checks, per tick and per run (`sequence`):
  * the 15 output-node values (f32 bits),
  * every node's `Value`, `LastInput`, `LastOutput` (f32 bits),
  * (if present in the Rust output) the echoed 33 input values.

With `--ulp-tolerance N` (default 0 = strict bitwise) values may differ by at
most N ULPs. Transcendental maths follows Unity/Mono double semantics via the
platform libm (same functions Rust call), so keep the default 0 for validation.

`--self-test` runs hand-computed activation/semantics cases, comparator
positive/negative tests, and a `parity_common.open_capture` reader regression
for plain vs gzip Rust capture JSONL (skipped if `tools/parity` is absent).
It needs no Rust output.
"""

from __future__ import annotations

import argparse
import copy
import gzip
import importlib
import json
import math
import os
import shutil
import struct
import sys
import tempfile
from typing import Any, Dict, List, Optional, Sequence, Tuple

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import brain_ref as ref  # noqa: E402

N_INPUTS = ref.N_INPUTS
N_OUTPUTS = ref.N_OUTPUTS
FIRST_OUTPUT = ref.FIRST_OUTPUT
FIRST_HIDDEN = ref.FIRST_HIDDEN

_OUTPUT_TYPES = [
    ref.TANH,  # Accelerate
    ref.TANH,  # Rotate
    ref.TANH,  # Herding
    ref.TANH,  # EggProduction
    ref.SIGMOID,  # Want2Lay
    ref.TANH,  # Want2Eat
    ref.SIGMOID,  # Digestion
    ref.TANH,  # Grab
    ref.SIGMOID,  # ClkReset
    ref.RELU,  # PhereOut1
    ref.RELU,  # PhereOut2
    ref.RELU,  # PhereOut3
    ref.SIGMOID,  # Want2Grow
    ref.SIGMOID,  # Want2Heal
    ref.SIGMOID,  # Want2Attack
]

# Starter ("Basic bibite") output biases, NEATBrain template.
_STARTER_BIASES = [
    0.4535929,
    0.0,
    0.0,
    0.2,
    0.0,
    1.228,
    -2.0668955,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
]

# Starter synapses: (node_in, node_out, weight)
_STARTER_SYNAPSES = [
    (3, FIRST_OUTPUT + 6, 4.0729575),  # Fullness -> Digestion
    (13, FIRST_OUTPUT + 1, 1.1592513),  # PlantAngle -> Rotate
    (12, FIRST_OUTPUT + 0, -0.401356),  # PlantCloseness -> Accelerate
]

_INPUT_NAMES = [
    "EnergyRatio",
    "Maturity",
    "LifeRatio",
    "Fullness",
    "Speed",
    "RotationSpeed",
    "IsGrabbing",
    "AttackedDamage",
    "EggStored",
    "BibiteCloseness",
    "BibiteAngle",
    "NBibites",
    "PlantCloseness",
    "PlantAngle",
    "NPlants",
    "MeatCloseness",
    "MeatAngle",
    "NMeats",
    "RedBibite",
    "GreenBibite",
    "BlueBibite",
    "Tic",
    "Minute",
    "TimeAlive",
    "PheroSense1",
    "PheroSense2",
    "PheroSense3",
    "Phero1Angle",
    "Phero2Angle",
    "Phero3Angle",
    "Phero1Heading",
    "Phero2Heading",
    "Phero3Heading",
]

_OUTPUT_NAMES = [
    "Accelerate",
    "Rotate",
    "Herding",
    "EggProduction",
    "Want2Lay",
    "Want2Eat",
    "Digestion",
    "Grab",
    "ClkReset",
    "PhereOut1",
    "PhereOut2",
    "PhereOut3",
    "Want2Grow",
    "Want2Heal",
    "Want2Attack",
]


# ---------------------------------------------------------------------------
# Fixture construction (shared with the self-test and make_fixtures.py)
# ---------------------------------------------------------------------------

def make_node(index: int, node_type: int, bias: float = 0.0) -> Dict[str, Any]:
    return {
        "Type": node_type,
        "Index": index,
        "Inov": index + 1,
        "baseActivation": ref.f32(bias),
        "Value": 0.0,
        "LastInput": 0.0,
        "LastOutput": 0.0,
    }


def make_brain(
    period: float,
    hidden: Sequence[Tuple[int, float]],
    synapses: Sequence[Tuple[int, int, float, bool]],
    input_sequences: Sequence[Sequence[Sequence[float]]],
    output_biases: Optional[Sequence[float]] = None,
    herding_enabled: bool = True,
) -> Dict[str, Any]:
    """Builds a fixture dict.

    `hidden` is a list of `(node_type, bias)` for nodes starting at index 48.
    `synapses` is `(node_in, node_out, weight, enabled)`.
    Output nodes use the game's fixed activation types. Initial node state is
    seeded like the game's `FinalizeEditing` (bias-only execution).
    """
    nodes: List[Dict[str, Any]] = []
    for i in range(N_INPUTS):
        nodes.append(make_node(i, ref.INPUT))
    biases = list(output_biases) if output_biases is not None else [0.0] * N_OUTPUTS
    for i in range(N_OUTPUTS):
        nodes.append(make_node(FIRST_OUTPUT + i, _OUTPUT_TYPES[i], biases[i]))
    for hidden_index, (node_type, bias) in enumerate(hidden):
        nodes.append(make_node(FIRST_HIDDEN + hidden_index, int(node_type), bias))
    ref.initialize_states(nodes, period)
    fixture = {
        "period": ref.f32(period),
        "herding_enabled": herding_enabled,
        "nodes": nodes,
        "synapses": [
            {
                "Inov": inov + 1,
                "NodeIn": int(node_in),
                "NodeOut": int(node_out),
                "Weight": ref.f32(weight),
                "En": bool(enabled),
            }
            for inov, (node_in, node_out, weight, enabled) in enumerate(synapses)
        ],
        "input_sequences": [
            [[ref.f32(float(value)) for value in tick] for tick in sequence]
            for sequence in input_sequences
        ],
    }
    return fixture


def starter_fixture(input_sequences: Sequence[Sequence[Sequence[float]]]) -> Dict[str, Any]:
    return make_brain(
        period=2.0 / 30.0,
        hidden=[],
        synapses=[(a, b, w, True) for (a, b, w) in _STARTER_SYNAPSES],
        input_sequences=input_sequences,
        output_biases=_STARTER_BIASES,
    )


# ---------------------------------------------------------------------------
# Comparator
# ---------------------------------------------------------------------------

def _bits(value: float) -> int:
    return ref.f32_bits(value)


def _ordered_key(bits: int) -> int:
    """Monotonic mapping of f32 bits, for ULP distances."""
    if bits & 0x80000000:
        return 0x80000000 - (bits & 0x7FFFFFFF)
    return bits


def _ulp_distance(a: float, b: float) -> int:
    if a != a or b != b:  # NaN: compare payloads exactly
        return 0 if _bits(a) == _bits(b) else 1_000_000_000
    return abs(_ordered_key(_bits(a)) - _ordered_key(_bits(b)))


class Divergence:
    def __init__(
        self,
        sequence: int,
        tick: int,
        scope: str,
        index: int,
        field: str,
        expected: float,
        actual: float,
        ulp: int,
    ) -> None:
        self.sequence = sequence
        self.tick = tick
        self.scope = scope
        self.index = index
        self.field = field
        self.expected = expected
        self.actual = actual
        self.ulp = ulp

    def __str__(self) -> str:
        return (
            f"sequence {self.sequence} tick {self.tick} {self.scope} {self.index} "
            f"{self.field}: reference={self.expected!r} "
            f"bits=0x{_bits(self.expected):08X}, rust={self.actual!r} "
            f"bits=0x{_bits(self.actual):08X}, ULP={self.ulp}"
        )


def _check_value(
    divergences: List[Divergence],
    sequence: int,
    tick: int,
    scope: str,
    index: int,
    field: str,
    expected: float,
    actual: float,
    ulp_tolerance: int,
) -> None:
    expected = ref.f32(float(expected))
    actual = ref.f32(float(actual))
    ulp = _ulp_distance(expected, actual)
    ok = _bits(expected) == _bits(actual) if ulp_tolerance == 0 else ulp <= ulp_tolerance
    if not ok:
        divergences.append(
            Divergence(sequence, tick, scope, index, field, expected, actual, ulp)
        )


def compare_outputs(
    reference: Dict[str, Any],
    rust: Dict[str, Any],
    ulp_tolerance: int = 0,
    max_divergences: int = 25,
) -> Tuple[int, List[Divergence], int]:
    """Compares Rust output against the reference output.

    Returns `(n_checked_values, divergences, n_ticks)`.
    """
    divergences: List[Divergence] = []
    checked = 0
    ref_ticks = reference.get("ticks", [])
    rust_ticks = rust.get("ticks", [])
    if len(ref_ticks) != len(rust_ticks):
        divergences.append(
            Divergence(-1, -1, "run", 0, "tick_count", float(len(ref_ticks)), float(len(rust_ticks)), 0)
        )
        return checked, divergences, len(ref_ticks)
    for tick_index, (ref_tick, rust_tick) in enumerate(zip(ref_ticks, rust_ticks)):
        sequence = int(ref_tick.get("sequence", 0))
        tick = int(ref_tick.get("tick", tick_index))
        ref_outputs = ref_tick["outputs"]
        rust_outputs = rust_tick.get("outputs", [])
        if len(rust_outputs) != N_OUTPUTS:
            divergences.append(
                Divergence(sequence, tick, "outputs", 0, "length", float(N_OUTPUTS), float(len(rust_outputs)), 0)
            )
            continue
        for o in range(N_OUTPUTS):
            checked += 1
            _check_value(
                divergences,
                sequence,
                tick,
                "output",
                o,
                _OUTPUT_NAMES[o],
                ref_outputs[o],
                rust_outputs[o],
                ulp_tolerance,
            )
        ref_nodes = ref_tick["nodes"]
        rust_nodes = rust_tick.get("nodes", [])
        if len(rust_nodes) != len(ref_nodes):
            divergences.append(
                Divergence(sequence, tick, "nodes", 0, "length", float(len(ref_nodes)), float(len(rust_nodes)), 0)
            )
            continue
        for i, (ref_node, rust_node) in enumerate(zip(ref_nodes, rust_nodes)):
            for field in ("Value", "LastInput", "LastOutput"):
                checked += 1
                _check_value(
                    divergences,
                    sequence,
                    tick,
                    "node",
                    i,
                    field,
                    ref_node[field],
                    rust_node.get(field, float("nan")),
                    ulp_tolerance,
                )
        rust_inputs = rust_tick.get("inputs")
        if rust_inputs is not None:
            if len(rust_inputs) != N_INPUTS:
                divergences.append(
                    Divergence(sequence, tick, "inputs", 0, "length", float(N_INPUTS), float(len(rust_inputs)), 0)
                )
            else:
                for i in range(N_INPUTS):
                    checked += 1
                    _check_value(
                        divergences,
                        sequence,
                        tick,
                        "input",
                        i,
                        _INPUT_NAMES[i],
                        ref_tick["inputs"][i],
                        rust_inputs[i],
                        ulp_tolerance,
                    )
        if len(divergences) >= max_divergences:
            break
    return checked, divergences, len(ref_ticks)


def compare_files(
    fixture_path: str,
    rust_output_path: str,
    ulp_tolerance: int = 0,
) -> Tuple[int, List[Divergence], int]:
    with open(fixture_path, "r", encoding="utf-8-sig") as handle:
        fixture = json.load(handle)
    with open(rust_output_path, "r", encoding="utf-8-sig") as handle:
        rust = json.load(handle)
    reference = ref.evaluate(fixture)
    return compare_outputs(reference, rust, ulp_tolerance)


# ---------------------------------------------------------------------------
# parity_common reader regression (pins the plain-JSONL capture fix)
# ---------------------------------------------------------------------------

def _load_parity_common() -> Optional[Any]:
    """Imports tools/parity/parity_common (sibling directory), or None."""
    here = os.path.dirname(os.path.abspath(__file__))
    parity_dir = os.path.join(os.path.dirname(here), "parity")
    if not os.path.isdir(parity_dir):
        return None
    if parity_dir not in sys.path:
        sys.path.insert(0, parity_dir)
    try:
        return importlib.import_module("parity_common")
    except Exception:  # noqa: BLE001 - optional cross-check, never fail on import
        return None


def _capture_tick(tick: int) -> Dict[str, Any]:
    """The tick record shape emitted by the Rust `CaptureWriter` test fixture."""
    return {
        "schema": "rustibites.parity.tick.v1",
        "source": "rust",
        "tick": tick,
        "t": tick / 30.0,
        "sim_tps": 30,
        "brain_tps": 2,
        "settings_hash": "0123456789abcdef",
        "counts": {"bibites": 3, "eggs": 1, "corpses": 0},
        "pellets": {
            "plant_n": 10,
            "plant_biomass": 1.5,
            "meat_n": 2,
            "meat_biomass": 0.25,
            "free_biomass": 0.0,
            "bibite_biomass": 12.5,
        },
    }


def _write_jsonl(path: str, records: Sequence[Dict[str, Any]], gz: bool) -> None:
    if gz:
        handle = gzip.open(path, "wt", encoding="utf-8")
    else:
        handle = open(path, "w", encoding="utf-8", newline="\n")
    with handle:
        for record in records:
            handle.write(json.dumps(record, separators=(",", ":"), ensure_ascii=False))
            handle.write("\n")


def _write_manifest(path: str, manifest: Dict[str, Any]) -> None:
    with open(path, "w", encoding="utf-8", newline="\n") as handle:
        json.dump(manifest, handle, indent=2, ensure_ascii=False)
        handle.write("\n")


def _capture_manifest(pc: Any, source: str = "rust") -> Dict[str, Any]:
    return {
        "schema": pc.CAPTURE_SCHEMA,
        "kind": "capture",
        "source": source,
        "capture": "reader-regression",
        "pass": 0,
        "gene_names": list(pc.GENE_NAMES),
        "settings": {},
        "settings_hash": pc.settings_hash({}),
        "sim": {"sim_tps": 30, "brain_tps": 2},
        "files": {},
    }


def plain_jsonl_capture_regression(verbose: bool = False) -> bool:
    """Regression for `parity_common.open_capture` reading Rust plain JSONL.

    The Rust `CaptureWriter` emits uncompressed `ticks.jsonl`/`events.jsonl` (the
    harness spec allows it). This pins three behaviours of the reader:
    1. a plain-only capture directory opens and yields the records/events;
    2. a gzip-only directory still opens;
    3. when both plain and gz files exist, gzip wins (documented precedence).

    Returns False (skip) when `tools/parity/parity_common.py` is not present, so
    the braincheck suite stays runnable from a stripped checkout.
    """
    pc = _load_parity_common()
    if pc is None:
        if verbose:
            print("  [skip] parity_common not available (tools/parity missing)")
        return False
    base_dir = tempfile.mkdtemp(prefix="braincheck_capture_")
    try:
        records = [_capture_tick(0), _capture_tick(1)]
        events = [
            {"schema": pc.EVENT_SCHEMA, "tick": 1, "kind": "birth", "id": 42},
        ]

        # 1) Plain JSONL only (what the Rust writer emits).
        plain_dir = os.path.join(base_dir, "plain")
        os.makedirs(plain_dir)
        _write_manifest(os.path.join(plain_dir, "manifest.json"), _capture_manifest(pc))
        _write_jsonl(os.path.join(plain_dir, "ticks.jsonl"), records, gz=False)
        _write_jsonl(os.path.join(plain_dir, "events.jsonl"), events, gz=False)
        capture = pc.open_capture(plain_dir)
        if capture.source != "rust":
            raise AssertionError(f"plain capture source = {capture.source!r}")
        if capture.manifest.get("schema") != pc.CAPTURE_SCHEMA:
            raise AssertionError("plain capture manifest schema mismatch")
        got_ticks = list(capture.records())
        if [int(record["tick"]) for record in got_ticks] != [0, 1]:
            raise AssertionError(f"plain capture ticks = {[r['tick'] for r in got_ticks]}")
        got_events = list(capture.events())
        if len(got_events) != 1 or got_events[0].get("kind") != "birth":
            raise AssertionError(f"plain capture events = {got_events}")
        # The reader must reproduce the byte-exact digests of these records.
        if pc.record_digest(got_ticks[0]) != "c8b381e0df59dd28":
            raise AssertionError(
                f"plain capture digest = {pc.record_digest(got_ticks[0])!r}"
            )
        if pc.record_digest(got_ticks[1]) != "7dd800b8c0ca9f1f":
            raise AssertionError(
                f"plain capture digest = {pc.record_digest(got_ticks[1])!r}"
            )

        # 2) Gzip only (the game capture format) still opens.
        gz_dir = os.path.join(base_dir, "gzip")
        os.makedirs(gz_dir)
        _write_manifest(os.path.join(gz_dir, "manifest.json"), _capture_manifest(pc))
        _write_jsonl(os.path.join(gz_dir, "ticks.jsonl.gz"), records, gz=True)
        _write_jsonl(os.path.join(gz_dir, "events.jsonl.gz"), events, gz=True)
        capture = pc.open_capture(gz_dir)
        if len(list(capture.records())) != 2:
            raise AssertionError("gzip-only capture record count mismatch")
        if len(list(capture.events())) != 1:
            raise AssertionError("gzip-only capture event count mismatch")

        # 3) Both present: gzip must be preferred when both exist.
        both_dir = os.path.join(base_dir, "both")
        os.makedirs(both_dir)
        _write_manifest(os.path.join(both_dir, "manifest.json"), _capture_manifest(pc))
        _write_jsonl(os.path.join(both_dir, "ticks.jsonl"), records[:1], gz=False)
        _write_jsonl(os.path.join(both_dir, "ticks.jsonl.gz"), records, gz=True)
        capture = pc.open_capture(both_dir)
        if len(list(capture.records())) != 2:
            raise AssertionError("gzip precedence not honoured when plain+gz both exist")

        return True
    finally:
        shutil.rmtree(base_dir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Self-test
# ---------------------------------------------------------------------------

def _approx(actual: float, expected: float, eps: float = 2e-6) -> None:
    if not (abs(actual - expected) <= eps):
        raise AssertionError(f"expected {expected!r}, got {actual!r}")


def _node_state(output: Dict[str, Any], tick: int, index: int) -> Dict[str, Any]:
    return output["ticks"][tick]["nodes"][index]


def _zeros(count: int) -> List[float]:
    return [0.0] * count


def _seq(*ticks: Sequence[Tuple[int, float]]) -> List[List[float]]:
    """Builds one input sequence from (input_index, value) tick lists."""
    sequence: List[List[float]] = []
    for tick in ticks:
        values = _zeros(N_INPUTS)
        for index, value in tick:
            values[index] = value
        sequence.append(values)
    return sequence


def self_test(verbose: bool = False) -> None:
    period = 2.0 / 30.0
    failures = 0
    checks = 0

    def run(name: str, fixture: Dict[str, Any]) -> Dict[str, Any]:
        nonlocal checks
        output = ref.evaluate(fixture)
        checks += 1
        if verbose:
            print(f"  [ok] {name}: {output['tick_count']} ticks")
        return output

    # --- A. Starter brain: bias-only output vector -------------------------
    fixture = starter_fixture([[[0.0] * N_INPUTS]])
    output = run("starter bias outputs", fixture)
    notes = output["ticks"][0]["outputs"]
    _approx(notes[0], math.tanh(0.4535929))  # Accelerate
    _approx(notes[1], 0.0)  # Rotate
    _approx(notes[2], 0.0)  # Herding
    _approx(notes[3], math.tanh(0.2))  # EggProduction
    _approx(notes[4], 0.5)  # Want2Lay sigmoid(0)
    _approx(notes[5], math.tanh(1.228))  # Want2Eat
    _approx(notes[6], 1.0 / (1.0 + math.exp(2.0668955)))  # Digestion
    _approx(notes[7], 0.0)  # Grab
    _approx(notes[8], 0.5)  # ClkReset
    _approx(notes[9], 0.0)  # PhereOut1
    _approx(notes[10], 0.0)  # PhereOut2
    _approx(notes[11], 0.0)  # PhereOut3
    _approx(notes[12], 0.5)  # Want2Grow
    _approx(notes[13], 0.5)  # Want2Heal
    _approx(notes[14], 0.5)  # Want2Attack

    # --- B. One brain-tick input lag + starter reflex arcs -----------------
    tick_values = [(13, -0.5), (12, 0.6), (3, 0.8)]
    fixture = starter_fixture([_seq([], tick_values, tick_values)])
    output = run("starter one-tick lag", fixture)
    first = output["ticks"][0]["outputs"]
    second = output["ticks"][1]["outputs"]
    third = output["ticks"][2]["outputs"]
    _approx(first[1], 0.0)  # Rotate sees the previous (zero) input on tick 0
    _approx(first[0], math.tanh(0.4535929))
    _approx(second[1], 0.0)  # tick 1 still sees tick 0's zero inputs
    _approx(third[1], math.tanh(1.1592513 * -0.5))
    _approx(third[0], math.tanh(0.4535929 + (-0.401356 * 0.6)))
    _approx(third[6], 1.0 / (1.0 + math.exp(-(-2.0668955 + 4.0729575 * 0.8))))

    # --- C. Latch: thresholds inclusive, otherwise holds -------------------
    fixture = make_brain(
        period,
        hidden=[(ref.LATCH, 0.0)],
        synapses=[(3, FIRST_HIDDEN, 1.0, True), (FIRST_HIDDEN, FIRST_OUTPUT, 1.0, True)],
        input_sequences=[_seq([(3, 0.5)], [(3, 1.0)], [(3, 0.2)], [(3, 0.0)], [(3, 0.5)])],
    )
    output = run("latch hold/thresholds", fixture)
    latch_outs = [_node_state(output, tick, FIRST_HIDDEN)["LastOutput"] for tick in range(5)]
    for tick, expected in enumerate([0.0, 0.0, 1.0, 1.0, 0.0]):
        _approx(latch_outs[tick], expected)

    # --- D. Differential: first-tick spike, then zero ----------------------
    fixture = make_brain(
        period=0.5,
        hidden=[(ref.DIFFERENTIAL, 0.5)],
        synapses=[
            (3, FIRST_HIDDEN, 1.0, True),
            (FIRST_HIDDEN, FIRST_OUTPUT, 1.0, True),
        ],
        input_sequences=[_seq([], [], [])],
    )
    output = run("differential first-tick spike", fixture)
    # The initialised state is the "spike" (FinalizeEditing executes the node
    # once with input 0: x = bias, LastInput = 0 -> out = bias/period).
    _approx(fixture["nodes"][FIRST_HIDDEN]["LastOutput"], 0.5 / 0.5)
    _approx(fixture["nodes"][FIRST_HIDDEN]["LastInput"], 0.5)
    # Afterwards the input is unchanged, so the derivative is zero.
    _approx(_node_state(output, 0, FIRST_HIDDEN)["LastOutput"], 0.0)
    _approx(_node_state(output, 1, FIRST_HIDDEN)["LastOutput"], 0.0)
    _approx(_node_state(output, 2, FIRST_HIDDEN)["LastOutput"], 0.0)

    # --- E. Integrator accumulation + clamp --------------------------------
    fixture = make_brain(
        period=2.0 / 30.0,
        hidden=[(ref.INTEGRATOR, 0.0)],
        synapses=[(3, FIRST_HIDDEN, 1000.0, True), (FIRST_HIDDEN, FIRST_OUTPUT, 1.0, True)],
        input_sequences=[_seq(*[[(3, 1.0)] for _ in range(20)])],
    )
    output = run("integrator accumulation/clamp", fixture)
    integrator = [_node_state(output, tick, FIRST_HIDDEN)["LastOutput"] for tick in range(20)]
    _approx(integrator[0], 0.0)
    _approx(integrator[1], 1000.0 * (2.0 / 30.0), 1e-4)
    assert integrator[1] < integrator[2] < integrator[3], integrator[:4]
    _approx(integrator[-1], ref.OUTPUT_CLAMP)
    assert all(value <= ref.OUTPUT_CLAMP for value in integrator)

    # --- F. Inhibitory decay ----------------------------------------------
    fixture = make_brain(
        period=0.5,
        hidden=[(ref.INHIBITORY, 1.0)],
        synapses=[(3, FIRST_HIDDEN, 1.0, True), (FIRST_HIDDEN, FIRST_OUTPUT, 1.0, True)],
        input_sequences=[_seq([(3, 1.0)], [], [], [])],
    )
    output = run("inhibitory decay", fixture)
    inh = [_node_state(output, tick, FIRST_HIDDEN)["LastOutput"] for tick in range(4)]
    _approx(inh[0], 0.0)
    _approx(inh[1], 1.0)
    _approx(inh[2], -1.0 + math.exp(-0.5))
    _approx(inh[3], (-1.0 + math.exp(-0.5)) * math.exp(-0.5), 1e-5)

    # --- G. SoftLatch hysteresis ------------------------------------------
    fixture = make_brain(
        period,
        hidden=[(ref.SOFT_LATCH, 2.0)],
        synapses=[(3, FIRST_HIDDEN, 1.0, True), (FIRST_HIDDEN, FIRST_OUTPUT, 1.0, True)],
        input_sequences=[_seq([(3, 0.5)], [(3, 0.999)], [(3, 0.5)], [(3, 0.0)], [(3, 0.0005)])],
    )
    output = run("softlatch hysteresis", fixture)
    soft = [_node_state(output, tick, FIRST_HIDDEN)["LastOutput"] for tick in range(5)]
    _approx(soft[0], 0.0)
    expected_growth = (-math.exp(-2.0)) / (1.0 - math.exp(-2.0))
    expected_soft = expected_growth + (1.0 - expected_growth) * math.exp(-1.0)
    _approx(soft[1], expected_soft, 1e-5)
    _approx(soft[2], 1.0)
    expected_hold = 1.0 / (1.0 - math.exp(-2.0)) * (1.0 - math.exp(-1.0))
    _approx(soft[3], expected_hold, 1e-5)
    _approx(soft[4], 0.0)

    # --- H. Mult with bias -------------------------------------------------
    fixture = make_brain(
        period,
        hidden=[(ref.MULT, 2.0)],
        synapses=[
            (3, FIRST_HIDDEN, 1.0, True),
            (12, FIRST_HIDDEN, 1.0, True),
            (FIRST_HIDDEN, FIRST_OUTPUT, 1.0, True),
        ],
        input_sequences=[_seq([(3, 3.0), (12, 4.0)], [(3, 3.0), (12, 4.0)], [(3, 3.0), (12, 4.0)])],
    )
    output = run("mult bias", fixture)
    _approx(_node_state(output, 1, FIRST_HIDDEN)["LastOutput"], 24.0)
    _approx(
        output["ticks"][2]["outputs"][0],
        math.tanh(0.4535929 + 24.0),
        1e-5,
    )

    # --- I. Disabled synapse is skipped -----------------------------------
    fixture = make_brain(
        period,
        hidden=[],
        synapses=[(3, FIRST_OUTPUT + 6, 5.0, False)],
        input_sequences=[_seq([(3, 1.0)], [(3, 1.0)])],
        output_biases=[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    )
    output = run("disabled synapse", fixture)
    _approx(output["ticks"][1]["outputs"][6], 1.0 / (1.0 + math.exp(2.0)))

    # --- J. Comparator: identical inputs pass, 1-ULP corruption fails ------
    reference = ref.evaluate(starter_fixture([_seq([(3, 0.5), (13, 0.25)])]))
    good = copy.deepcopy(reference)
    checked, divergences, _ = compare_outputs(reference, good)
    if divergences:
        raise AssertionError(f"comparator false positive: {divergences[0]}")
    checks += 1
    bad = copy.deepcopy(reference)
    bad_value = bad["ticks"][0]["outputs"][0]
    bad_bits = (ref.f32_bits(bad_value) + 1) & 0xFFFFFFFF
    bad["ticks"][0]["outputs"][0] = struct.unpack("<f", struct.pack("<I", bad_bits))[0]
    _, divergences, _ = compare_outputs(reference, bad)
    if not divergences:
        raise AssertionError("comparator missed a 1-ULP output corruption")
    checks += 1
    bad = copy.deepcopy(reference)
    bad["ticks"][0]["nodes"][FIRST_HIDDEN - 1]["LastOutput"] += 1e-3
    _, divergences, _ = compare_outputs(reference, bad)
    if not divergences:
        raise AssertionError("comparator missed a node-state corruption")
    checks += 1
    if verbose:
        print("  [ok] comparator detects 1-ULP/corruption divergences")

    # --- K. Determinism ----------------------------------------------------
    first = ref.evaluate(starter_fixture([_seq([(3, 0.3), (13, -0.2)])]))
    second = ref.evaluate(starter_fixture([_seq([(3, 0.3), (13, -0.2)])]))
    if json.dumps(first, sort_keys=True) != json.dumps(second, sort_keys=True):
        raise AssertionError("reference evaluation is not deterministic")
    checks += 1
    _ = run  # keep helper referenced for readability

    # --- L. parity_common reader regression (plain Rust-style capture JSONL) --
    if plain_jsonl_capture_regression(verbose):
        checks += 1
        if verbose:
            print(
                "  [ok] parity_common opens plain and gzip Rust captures "
                "(gz preferred when both exist)"
            )

    print(
        f"self-test: {checks} case groups passed; sanity checks on "
        f"{checked} values; double-math semantics (Unity/Mono)"
    )


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture", nargs="?", help="brain fixture JSON path")
    parser.add_argument("rust_output", nargs="?", help="Rust output JSON path")
    parser.add_argument(
        "--ulp-tolerance",
        type=int,
        default=0,
        help="allowed ULP difference per value (default 0 = strict bitwise)",
    )
    parser.add_argument("--self-test", action="store_true", help="run built-in self-test")
    parser.add_argument("--verbose", action="store_true", help="verbose output")
    args = parser.parse_args(argv)

    if args.self_test:
        try:
            self_test(verbose=args.verbose)
        except AssertionError as error:
            print(f"self-test FAILED: {error}", file=sys.stderr)
            return 1
        return 0

    if not args.fixture or not args.rust_output:
        parser.error("fixture and rust_output are required unless --self-test is used")

    checked, divergences, n_ticks = compare_files(
        args.fixture, args.rust_output, args.ulp_tolerance
    )
    if divergences:
        print(
            f"FAIL: {len(divergences)} divergence(s) over {n_ticks} ticks "
            f"({checked} values checked, tolerance {args.ulp_tolerance} ULP)",
            file=sys.stderr,
        )
        for divergence in divergences:
            print(f"  first: {divergence}" if divergence is divergences[0] else f"  also: {divergence}", file=sys.stderr)
        return 1
    print(
        f"PASS: {checked} values bitwise-identical over {n_ticks} ticks "
        f"(tolerance {args.ulp_tolerance} ULP)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

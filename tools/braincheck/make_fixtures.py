#!/usr/bin/env python3
"""Deterministically generates the committed braincheck fixtures.

Run from this directory:

    python make_fixtures.py

Outputs (all pure JSON, no game assets, each < 50 KB):
  fixtures/starter_brain.json          official "Basic bibite" brain + 3 input runs
  fixtures/random_stateful_brain.json  61-node brain covering all activation types
  fixtures/random_small_brain.json     50-node brain with two stateful hidden nodes

Input sequences are deterministic pseudo-random patterns; no wall-clock or
OS randomness is used, so re-running this script reproduces byte-identical
fixtures.
"""

from __future__ import annotations

import json
import math
import os
import random
from typing import Any, Dict, List, Sequence, Tuple

import brain_ref as ref
import run_check as check

OUT_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fixtures")


def _f32(value: float) -> float:
    return ref.f32(float(value))


def _pattern_sequence(
    rng: random.Random, ticks: int, mode: str
) -> List[List[float]]:
    """Builds one deterministic 33-value input sequence."""
    sequence: List[List[float]] = []
    for tick in range(ticks):
        values = [0.0] * ref.N_INPUTS
        if mode == "zeros":
            pass
        elif mode == "plant":
            values[3] = 0.0  # Fullness
            values[12] = _f32(0.1 + 0.8 * tick / max(ticks - 1, 1))  # PlantCloseness
            values[13] = _f32(0.4 * math.sin(tick * 0.9))  # PlantAngle
            values[0] = _f32(0.5 + 0.3 * math.cos(tick * 0.4))  # EnergyRatio
            values[1] = _f32(0.25 * tick)  # Maturity
        elif mode == "feeding":
            values[3] = _f32(tick / max(ticks - 1, 1))  # Fullness ramp
            values[12] = _f32(0.2 + 0.3 * math.cos(tick * 0.7))
            values[13] = _f32(0.3 * math.sin(tick * 1.3))
            values[24] = _f32(0.5 + 0.5 * math.sin(tick * 0.2))  # PheroSense1
        elif mode == "random":
            for i in range(ref.N_INPUTS):
                values[i] = _f32(rng.uniform(-1.0, 1.0))
        else:
            raise ValueError(mode)
        sequence.append(values)
    return sequence


def build_starter() -> Dict[str, Any]:
    sequences = [
        _pattern_sequence(random.Random(101), 8, "zeros"),
        _pattern_sequence(random.Random(102), 10, "plant"),
        _pattern_sequence(random.Random(103), 10, "feeding"),
    ]
    fixture = check.starter_fixture(sequences)
    fixture["name"] = "starter_basic_bibite"
    fixture["description"] = (
        "Official Basic bibite 0.6.3.1 brain: 48 nodes, 3 synapses, "
        "3 input sequences (zeros/plant/feeding)."
    )
    return fixture


def _random_topology(
    rng: random.Random,
    hidden: Sequence[Tuple[int, float]],
    extra_links: int,
) -> List[Tuple[int, int, float, bool]]:
    """Builds a connected random topology for the hidden nodes."""
    synapses: List[Tuple[int, int, float, bool]] = []
    hidden_indices = [ref.FIRST_HIDDEN + i for i in range(len(hidden))]
    # Every hidden node needs at least one incoming and one outgoing synapse.
    for index in hidden_indices:
        source = rng.randrange(0, ref.FIRST_HIDDEN)
        synapses.append((source, index, _f32(rng.uniform(-2.0, 2.0)), True))
        destination = ref.FIRST_OUTPUT + rng.randrange(0, ref.N_OUTPUTS)
        synapses.append((index, destination, _f32(rng.uniform(-2.0, 2.0)), True))
    # Extra connections, including hidden->hidden and some disabled synapses.
    for _ in range(extra_links):
        source = rng.randrange(0, ref.FIRST_HIDDEN + len(hidden))
        destination = rng.randrange(ref.FIRST_OUTPUT, ref.FIRST_HIDDEN + len(hidden))
        if source == destination:
            continue
        enabled = rng.random() > 0.2
        synapses.append((source, destination, _f32(rng.uniform(-2.0, 2.0)), enabled))
    return synapses


def build_random_stateful() -> Dict[str, Any]:
    rng = random.Random(202)
    # One hidden node of every non-input activation type (1..=13).
    node_types = list(range(1, 14))
    biases = {
        ref.SIGMOID: 0.3,
        ref.LINEAR: -0.7,
        ref.TANH: 0.5,
        ref.SINE: 1.2,
        ref.RELU: 0.0,
        ref.GAUSSIAN: 0.0,
        ref.LATCH: 0.2,
        ref.DIFFERENTIAL: 0.5,
        ref.ABS: 0.0,
        ref.MULT: 2.0,
        ref.INTEGRATOR: 0.0,
        ref.INHIBITORY: 1.5,
        ref.SOFT_LATCH: 5.0,
    }
    hidden = [(node_type, biases[node_type]) for node_type in node_types]
    output_biases = [_f32(rng.uniform(-1.5, 1.5)) for _ in range(ref.N_OUTPUTS)]
    synapses = _random_topology(rng, hidden, extra_links=24)
    sequences = [
        _pattern_sequence(random.Random(201), 10, "random") for _ in range(3)
    ]
    fixture = check.make_brain(
        period=2.0 / 30.0,
        hidden=hidden,
        synapses=synapses,
        input_sequences=sequences,
        output_biases=output_biases,
    )
    fixture["name"] = "random_stateful_61"
    fixture["description"] = (
        "Random brain: 48 base + 13 hidden nodes covering every activation type "
        "(all stateful types included); random topology with disabled synapses; "
        "3 deterministic random input sequences."
    )
    return fixture


def build_random_small() -> Dict[str, Any]:
    rng = random.Random(303)
    hidden = [(ref.LATCH, 0.2), (ref.INTEGRATOR, 0.0)]
    output_biases = [_f32(rng.uniform(-0.5, 0.5)) for _ in range(ref.N_OUTPUTS)]
    synapses = _random_topology(rng, hidden, extra_links=6)
    sequences = [
        _pattern_sequence(random.Random(301), 8, "random"),
        _pattern_sequence(random.Random(302), 8, "plant"),
    ]
    fixture = check.make_brain(
        period=2.0 / 30.0,
        hidden=hidden,
        synapses=synapses,
        input_sequences=sequences,
        output_biases=output_biases,
    )
    fixture["name"] = "random_small_50"
    fixture["description"] = (
        "Random brain with 48 base + 2 stateful hidden nodes (Latch, Integrator); "
        "2 input sequences."
    )
    return fixture


def main() -> None:
    os.makedirs(OUT_DIR, exist_ok=True)
    fixtures = {
        "starter_brain.json": build_starter(),
        "random_stateful_brain.json": build_random_stateful(),
        "random_small_brain.json": build_random_small(),
    }
    for name, fixture in fixtures.items():
        path = os.path.join(OUT_DIR, name)
        with open(path, "w", encoding="utf-8", newline="\n") as handle:
            json.dump(fixture, handle, separators=(",", ":"), sort_keys=True)
            handle.write("\n")
        size = os.path.getsize(path)
        ticks = sum(len(sequence) for sequence in fixture["input_sequences"])
        print(
            f"wrote {path} ({size} bytes, {len(fixture['nodes'])} nodes, "
            f"{len(fixture['synapses'])} synapses, {ticks} ticks)"
        )


if __name__ == "__main__":
    main()

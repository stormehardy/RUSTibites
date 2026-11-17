#!/usr/bin/env python3
"""ecoref.py - ecology reference/diff for a fresh Ring scenario (task-86, supports task-85).

Captures themselves come from:
  * game: `tools/parity_round2/capture_scenario.py` (hidden lab, newgame Ring.zip, 1x pinned, 300 ticks)
  * Rust: `rustibites run testdata/real/Ring.zip --ticks N --seed S --capture DIR --quiet`

This script extracts the ecological trajectory from one capture (`--capture`) or produces a focused
first-divergence report between a game reference and a Rust capture (`--game ... --rust ...`):
population/eggs/corpses, births/deaths (lifecycle counters + event ledger), plant pellet count/biomass,
feeding indicators (bite_rate/egg_rate/speed), energy/health, and the spawner timing
(virginSpawnRate spawn events). Writes JSON under testdata/parity/round3/ by default.

Usage:
    python tools/parity/ecoref.py --capture testdata/parity/round3/ring_scenario_game/pass-0 \
        --out testdata/parity/round3/game_reference.json
    python tools/parity/ecoref.py --game testdata/parity/round3/ring_scenario_game/pass-0 \
        --rust testdata/parity/round3/rust_ring_scenario \
        --out testdata/parity/round3/ring_scenario_divergence.json
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import parity_common as pc  # noqa: E402

SERIES = ["counts.bibites", "counts.eggs", "counts.corpses",
          "pellets.plant_n", "pellets.plant_biomass", "pellets.meat_n", "pellets.meat_biomass",
          "vitals.energy_mean", "vitals.health_mean",
          "behavior.bite_rate_mean", "behavior.egg_rate_mean", "behavior.speed_mean",
          "behavior.moving_frac", "lifecycle.births", "lifecycle.deaths"]


def get(rec: dict, path: str):
    cur = rec
    for part in path.split("."):
        if not isinstance(cur, dict) or part not in cur:
            return None
        cur = cur[part]
    return cur


def capture_series(path: Path) -> dict:
    cap = pc.open_capture(path)
    manifest = cap.manifest
    rows = {}
    for rec in cap.records():
        tick = int(rec.get("tick"))
        row = {"tick": tick}
        for key in SERIES:
            v = get(rec, key)
            row[key] = v
        rows[tick] = row
    events = []
    try:
        events = list(cap.events())
    except Exception:
        events = []
    kinds = {}
    for e in events:
        kinds[e.get("kind")] = kinds.get(e.get("kind"), 0) + 1
    return {
        "path": str(path),
        "source": manifest.get("source"),
        "sim": manifest.get("sim"),
        "settings": manifest.get("settings"),
        "counts_start": manifest.get("counts_start"),
        "counts_end": manifest.get("counts_end"),
        "taint": manifest.get("taint"),
        "rows": rows,
        "events": events,
        "event_kinds": kinds,
    }


def summary(capdata: dict) -> dict:
    rows = capdata["rows"]
    ticks = sorted(rows)
    first = rows[ticks[0]]
    last = rows[ticks[-1]]
    births = [r["lifecycle.births"] or 0 for r in rows.values()]
    deaths = [r["lifecycle.deaths"] or 0 for r in rows.values()]
    cum_b = []
    cum_d = []
    acc_b = acc_d = 0
    for t in ticks:
        acc_b += int(rows[t]["lifecycle.births"] or 0)
        acc_d += int(rows[t]["lifecycle.deaths"] or 0)
        cum_b.append([t, acc_b])
        cum_d.append([t, acc_d])
    bite_ticks = [t for t in ticks if (rows[t]["behavior.bite_rate_mean"] or 0) > 0]
    egg_ticks = [t for t in ticks if (rows[t]["behavior.egg_rate_mean"] or 0) > 0]
    samples = {}
    for t in (0, 15, 30, 60, 90, 120, 150, 180, 240, 300):
        if t in rows:
            r = rows[t]
            samples[t] = {k: r[k] for k in ("counts.bibites", "counts.eggs", "counts.corpses",
                                            "pellets.plant_n", "pellets.plant_biomass",
                                            "vitals.energy_mean", "behavior.bite_rate_mean",
                                            "behavior.egg_rate_mean")}
    birth_events = [(e.get("tick"), e.get("kind"), e.get("id"), e.get("parent1")) for e in capdata["events"]
                    if e.get("kind") in ("birth", "death", "initial")]
    return {
        "source": capdata["source"],
        "path": capdata["path"],
        "ticks": [ticks[0], ticks[-1]],
        "counts_start": capdata["counts_start"],
        "counts_end": capdata["counts_end"],
        "totals": {"births": acc_b, "deaths": acc_d, "event_kinds": capdata["event_kinds"],
                   "events": len(capdata["events"])},
        "cumulative_births": cum_b, "cumulative_deaths": cum_d,
        "first_bite_tick": bite_ticks[0] if bite_ticks else None,
        "first_egg_tick": egg_ticks[0] if egg_ticks else None,
        "bite_ticks": bite_ticks[:20], "egg_ticks": egg_ticks[:20],
        "samples": samples,
        "spawn_events": birth_events,
        "virginSpawnRate": ((capdata["settings"] or {}).get("independent") or {}).get("virginSpawnRate"),
    }


def divergence(game: dict, rust: dict) -> dict:
    common = sorted(set(game["rows"]) & set(rust["rows"]))
    report = {"game": game["path"], "rust": rust["path"], "common_ticks": [common[0], common[-1]],
              "first_divergence": {}, "values_at_first_divergence": {}, "tolerance": {}}
    for key in SERIES:
        tol = 0.0
        if key.startswith(("counts.", "pellets.")) and key.endswith(("_n", "bibites", "eggs", "corpses")):
            tol = 0.0  # integer counts must match exactly
        elif key.startswith("vitals.") or key.startswith("behavior."):
            tol = 1e-6
        else:
            tol = 1e-6
        first = None
        for t in common:
            a = game["rows"][t].get(key)
            b = rust["rows"][t].get(key)
            if a is None or b is None:
                continue
            try:
                if abs(float(b) - float(a)) > tol:
                    first = {"tick": t, "left": a, "right": b, "delta": float(b) - float(a)}
                    break
            except (TypeError, ValueError):
                if a != b:
                    first = {"tick": t, "left": a, "right": b, "delta": None}
                    break
        report["first_divergence"][key] = first
        report["tolerance"][key] = tol
    # exact facts that matter for task-85
    g = game["rows"]
    r = rust["rows"]

    def cumulative(rows, key):
        acc, out = 0, []
        for t in sorted(rows):
            acc += int(rows[t].get(key) or 0)
            out.append([t, acc])
        return out

    game_cum_births = cumulative(g, "lifecycle.births")
    rust_cum_births = cumulative(r, "lifecycle.births")
    report["facts"] = {
        "game_first_bite_tick": next((t for t in common if (g[t]["behavior.bite_rate_mean"] or 0) > 0), None),
        "rust_first_bite_tick": next((t for t in common if (r[t]["behavior.bite_rate_mean"] or 0) > 0), None),
        "game_population_ticks": [[t, g[t]["counts.bibites"]] for t in (0, 30, 60, 120, 180, 300) if t in g],
        "rust_population_ticks": [[t, r[t]["counts.bibites"]] for t in (0, 30, 60, 120, 180, 300) if t in r],
        "game_cum_births_at_120": next((b for t, b in game_cum_births if t == 120), None),
        "rust_cum_births_at_120": next((b for t, b in rust_cum_births if t == 120), None),
        "game_plant_n_ticks": [[t, g[t]["pellets.plant_n"]] for t in (0, 60, 120, 300) if t in g],
        "rust_plant_n_ticks": [[t, r[t]["pellets.plant_n"]] for t in (0, 60, 120, 300) if t in r],
    }
    return report


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--capture", help="single capture to summarise")
    ap.add_argument("--game", help="game reference capture directory")
    ap.add_argument("--rust", help="Rust capture directory")
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    if args.capture:
        data = summary(capture_series(Path(args.capture)))
        out.write_text(json.dumps(data, indent=2), encoding="utf-8")
        print(json.dumps({k: data[k] for k in ("source", "ticks", "counts_start", "counts_end", "totals",
                                               "first_bite_tick", "first_egg_tick", "virginSpawnRate")}, indent=1))
        return 0
    if not (args.game and args.rust):
        raise SystemExit("need --capture, or both --game and --rust")
    game = capture_series(Path(args.game))
    rust = capture_series(Path(args.rust))
    rep = divergence(game, rust)
    rep["game_summary"] = summary(game)
    rep["rust_summary"] = summary(rust)
    out.write_text(json.dumps(rep, indent=2), encoding="utf-8")
    print("first divergence per signal (tick, game, rust):")
    for key, first in rep["first_divergence"].items():
        print(f"  {key:30s} {first}")
    print("facts:", json.dumps(rep["facts"], indent=1))
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

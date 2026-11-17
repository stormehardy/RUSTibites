#!/usr/bin/env python3
"""ecoref_long.py - long-horizon ecology reference comparison (task-89).

Analyses two captures of the same world (game at the 50x statistical protocol vs Rust) and reports a
1 Hz trajectory envelope + a focused feeding/energy verdict:

  * population / eggs / corpses, cumulative births/deaths, plant pellet count+biomass, meat,
    energy/health means, species count at ~1 Hz (every 15 ticks at simTPS=15);
  * envelope (start/min/max/final) for each signal and first divergence tick;
  * plant-consumption evidence from pellet count/biomass (does either side drop below its production
    ceiling while the population feeds?);
  * `bite_rate_mean` caveat: this sampler field is 0 on both sides, so it cannot decide feeding;
    the recommended metric is a real bites/consumed-mass counter (see docs) or pellet-mass conservation.
  * PNG plots + JSON + CSV under --out.

Usage:
    python tools/parity/ecoref_long.py \
        --game testdata/parity/round4/ring_scenario_game_20min/pass-0 \
        --rust testdata/parity/round4/rust_ring_20min \
        --label ring_20min --out testdata/parity/round4/long_analysis
"""
from __future__ import annotations

import argparse
import csv
import json
import math
import sys
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import parity_common as pc  # noqa: E402

SIGNALS = ["counts.bibites", "counts.eggs", "counts.corpses",
           "pellets.plant_n", "pellets.plant_biomass", "pellets.meat_n", "pellets.meat_biomass",
           "vitals.energy_mean", "vitals.health_mean", "species.n", "behavior.bite_rate_mean"]


def get(rec, path):
    cur = rec
    for part in path.split("."):
        if not isinstance(cur, dict) or part not in cur:
            return None
        cur = cur[part]
    return cur


def load(path: Path):
    cap = pc.open_capture(path)
    m = cap.manifest or {}
    rows = {int(r["tick"]): r for r in cap.records()}
    events = list(cap.events()) if hasattr(cap, "events") else []
    return cap, m, rows, events


def series(path: Path, step: int = 15):
    cap, manifest, rows, events = load(path)
    ticks = sorted(rows)
    sim_tps = int((manifest.get("sim") or {}).get("sim_tps") or 15)
    step = max(1, int(round(sim_tps / 15 * step)))  # 1 Hz at 15 TPS
    out = []
    cum_b = cum_d = 0
    for i, t in enumerate(ticks):
        rec = rows[t]
        cum_b += int(((rec.get("lifecycle") or {}).get("births") or 0))
        cum_d += int(((rec.get("lifecycle") or {}).get("deaths") or 0))
        if i % step and t != ticks[-1]:
            continue
        row = {"tick": t, "t_s": t / sim_tps, "minute": t / sim_tps / 60.0,
               "cum_births": cum_b, "cum_deaths": cum_d}
        for key in SIGNALS:
            row[key] = get(rec, key)
        out.append(row)
    kinds = {}
    for e in events:
        kinds[e.get("kind")] = kinds.get(e.get("kind"), 0) + 1
    bite_vals = [float(r["behavior.bite_rate_mean"]) for r in out
                 if r.get("behavior.bite_rate_mean") is not None]
    bite_stats = {"mean": (sum(bite_vals) / len(bite_vals) if bite_vals else None),
                  "max": max(bite_vals) if bite_vals else None,
                  "nonzero_ticks": sum(1 for v in bite_vals if v > 0.0),
                  "n": len(bite_vals)}
    return {"path": str(path), "source": manifest.get("source"), "sim_tps": sim_tps,
            "ticks": [ticks[0], ticks[-1]], "n_records": len(rows), "series": out,
            "event_kinds": kinds, "event_count": len(events), "bite_rate": bite_stats,
            "taint": manifest.get("taint"), "capture": manifest.get("capture"),
            "speed": ((manifest.get("capture") or {}).get("speed")
                      if isinstance(manifest.get("capture"), dict) else None)}


def envelope(rows):
    def col(k):
        return [float(r[k]) for r in rows if r.get(k) is not None]
    pop = col("counts.bibites")
    plant_n = col("pellets.plant_n")
    plant_b = col("pellets.plant_biomass")
    energy = col("vitals.energy_mean")
    health = col("vitals.health_mean")
    meat_b = col("pellets.meat_biomass")
    species = col("species.n")
    out = {
        "population": {"start": pop[0] if pop else None, "final": pop[-1] if pop else None,
                       "min": min(pop) if pop else None, "max": max(pop) if pop else None},
        "births": rows[-1]["cum_births"], "deaths": rows[-1]["cum_deaths"],
        "plant_n": {"start": plant_n[0] if plant_n else None, "final": plant_n[-1] if plant_n else None,
                    "min": min(plant_n) if plant_n else None, "max": max(plant_n) if plant_n else None},
        "plant_biomass": {"start": plant_b[0] if plant_b else None, "final": plant_b[-1] if plant_b else None,
                          "min": min(plant_b) if plant_b else None, "max": max(plant_b) if plant_b else None},
        "meat_biomass": {"max": max(meat_b) if meat_b else None, "final": meat_b[-1] if meat_b else None},
        "energy": {"start": energy[0] if energy else None, "final": energy[-1] if energy else None,
                   "min": min(energy) if energy else None, "max": max(energy) if energy else None},
        "health": {"start": health[0] if health else None, "final": health[-1] if health else None},
        "species_max": max(species) if species else None,
    }
    # plant consumption evidence: does pellet count/biomass fall below the observed production ceiling
    def dip(stats):
        if not stats["max"] or stats["min"] is None:
            return None
        return (stats["max"] - stats["min"]) / stats["max"]
    out["plant_n_dip_frac"] = dip(out["plant_n"])
    out["plant_biomass_dip_frac"] = dip(out["plant_biomass"])
    return out


def fmt(x, nd=2):
    if x is None:
        return "-"
    if isinstance(x, float):
        return f"{x:.{nd}g}"
    return str(x)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--game", required=True)
    ap.add_argument("--rust", required=True)
    ap.add_argument("--label", required=True)
    ap.add_argument("--out", default=str(ROOT / "testdata" / "parity" / "round4" / "long_analysis"))
    args = ap.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    game = series(Path(args.game))
    rust = series(Path(args.rust))
    game_env = envelope(game["series"])
    rust_env = envelope(rust["series"])
    # compare at 1-minute samples over common ticks
    g_by_t = {r["tick"]: r for r in game["series"]}
    r_by_t = {r["tick"]: r for r in rust["series"]}
    common = sorted(set(g_by_t) & set(r_by_t))
    first = {}
    for key in SIGNALS:
        first[key] = None
        for t in common:
            a, b = g_by_t[t].get(key), r_by_t[t].get(key)
            if a is None or b is None:
                continue
            try:
                if abs(float(b) - float(a)) > 1e-6:
                    first[key] = {"tick": t, "minute": round(t / 900.0, 2), "game": a, "rust": b,
                                  "delta": float(b) - float(a)}
                    break
            except (TypeError, ValueError):
                if a != b:
                    first[key] = {"tick": t, "game": a, "rust": b}
                    break
    minute_marks = [900, 1800, 2700, 4500, 9000, 13500, 18000]
    table = []
    for t in minute_marks:
        if t in g_by_t and t in r_by_t:
            row = {"tick": t, "minute": t / 900.0}
            for key in ("counts.bibites", "counts.corpses", "cum_births", "cum_deaths",
                        "pellets.plant_n", "pellets.plant_biomass", "vitals.energy_mean"):
                row[f"game_{key}"] = g_by_t[t].get(key)
                row[f"rust_{key}"] = r_by_t[t].get(key)
            table.append(row)
    result = {"label": args.label, "game": {"path": game["path"], "source": game["source"],
                                            "capture": game["capture"], "speed": game["speed"],
                                            "envelope": game_env, "event_kinds": game["event_kinds"]},
              "rust": {"path": rust["path"], "source": rust["source"],
                       "envelope": rust_env, "event_kinds": rust["event_kinds"]},
              "first_divergence": first, "minute_table": table,
              "bite_rate_caveat": {
                  "field": "behavior.bite_rate_mean",
                  "game": game.get("bite_rate"), "rust": rust.get("bite_rate"),
                  "note": "Ring fresh-scenario starters report bite_rate_mean = 0 on BOTH sides (the game's "
                          "own scenario bibites too), so it is unusable there; lol evolved worlds DO populate "
                          "the field (game/rust means above) and it still diverges. Either way it is not a "
                          "conserved feeding signal: use an explicit bites/consumed-bio-mass counter (game "
                          "sampler + Rust stats) and/or pellet-mass conservation for a real feeding metric.",
              },
              "plant_consumption": {
                  "game_plant_n_dip_frac": game_env["plant_n_dip_frac"],
                  "game_plant_biomass_dip_frac": game_env["plant_biomass_dip_frac"],
                  "rust_plant_n_dip_frac": rust_env["plant_n_dip_frac"],
                  "rust_plant_biomass_dip_frac": rust_env["plant_biomass_dip_frac"],
                  "interpretation": "a sustained dip below the production ceiling while the population is "
                                    "alive is evidence of pellet consumption; flat-at-ceiling means the "
                                    "population does not measurably reduce the zone stock (production "
                                    "keeps up, or nothing is eaten).",
              }}
    (out / f"{args.label}.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    # CSV of the 1 Hz series side by side
    with open(out / f"{args.label}_series.csv", "w", newline="", encoding="utf-8") as fh:
        writer = csv.writer(fh)
        writer.writerow(["tick", "minute", "game_pop", "rust_pop", "game_plant_n", "rust_plant_n",
                         "game_plant_biomass", "rust_plant_biomass", "game_energy", "rust_energy",
                         "game_cum_births", "rust_cum_births", "game_cum_deaths", "rust_cum_deaths"])
        for t in common:
            g, r = g_by_t[t], r_by_t[t]
            writer.writerow([t, round(t / 900.0, 3),
                             g.get("counts.bibites"), r.get("counts.bibites"),
                             g.get("pellets.plant_n"), r.get("pellets.plant_n"),
                             g.get("pellets.plant_biomass"), r.get("pellets.plant_biomass"),
                             g.get("vitals.energy_mean"), r.get("vitals.energy_mean"),
                             g.get("cum_births"), r.get("cum_births"),
                             g.get("cum_deaths"), r.get("cum_deaths")])
    # plots
    try:
        import matplotlib
        matplotlib.use("Agg")
        import matplotlib.pyplot as plt
        ticks = [t / 900.0 for t in common]
        fig, ax = plt.subplots(3, 1, figsize=(11, 9), sharex=True)
        ax[0].plot(ticks, [g_by_t[t].get("counts.bibites") for t in common], label="game")
        ax[0].plot(ticks, [r_by_t[t].get("counts.bibites") for t in common], label="rust")
        ax[0].set_ylabel("population"); ax[0].legend(); ax[0].set_title(f"{args.label}: long-run ecology")
        ax[1].plot(ticks, [g_by_t[t].get("pellets.plant_n") for t in common], label="game plant_n")
        ax[1].plot(ticks, [r_by_t[t].get("pellets.plant_n") for t in common], label="rust plant_n")
        ax[1].set_ylabel("plant pellets"); ax[1].legend()
        ax[2].plot(ticks, [g_by_t[t].get("vitals.energy_mean") for t in common], label="game energy")
        ax[2].plot(ticks, [r_by_t[t].get("vitals.energy_mean") for t in common], label="rust energy")
        ax[2].set_ylabel("energy mean"); ax[2].set_xlabel("simulated minutes"); ax[2].legend()
        fig.tight_layout()
        fig.savefig(out / f"{args.label}.png", dpi=110)
        plt.close(fig)
        result["plot"] = str(out / f"{args.label}.png")
    except Exception as exc:  # noqa: BLE001
        result["plot_error"] = str(exc)
    (out / f"{args.label}.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(json.dumps({"label": args.label, "game_envelope": game_env, "rust_envelope": rust_env,
                      "first_divergence": first, "bite_rate": result["bite_rate_caveat"],
                      "plant_consumption": result["plant_consumption"]}, indent=1))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

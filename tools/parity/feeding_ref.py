#!/usr/bin/env python3
"""feeding_ref.py - turn a feeding-instrumented capture into game-side feeding rates (task-94).

The parity plugin (`GET /d/feeding`, capture.py --feeding) records per-bibite stomach contents by
material, held pellets, mouth bite timers/counters, energy/health, plus per-zone pellet stock and
production counters. This script computes from that stream:

  * per-material successful-intake ("bite") proxy: positive changes of a bibite's stomach amount for
    that material between snapshots, count of increments and consumed amount;
  * per-bibite / per-second rates, stomach fill, held pellets, energy/health trajectories;
  * gross energy income proxy = consumed amount x material EnergyDensity (from the plugin's
    material_properties) and observed population energy change;
  * per-zone/global pellet stock deltas and zone production counters (what is/is not inferable);
  * first tick with any intake, rates per sim-second and per bibite.
Writes <capture>/feeding_summary.json + feeding_series.csv. No game needed.

Usage: python tools/parity/feeding_ref.py --capture testdata/parity/round5/feeding_lol_600
"""
from __future__ import annotations

import argparse
import csv
import gzip
import json
import math
from pathlib import Path


def read_feeding(capture: Path):
    for name in ("feeding.jsonl.gz", "feeding.jsonl"):
        p = capture / name
        if p.exists():
            opener = gzip.open if name.endswith(".gz") else open
            with opener(p, "rt", encoding="utf-8") as fh:
                return [json.loads(line) for line in fh if line.strip()]
    raise SystemExit(f"no feeding stream in {capture}")


def classify(mat: str) -> str:
    m = (mat or "").lower()
    if "plant" in m:
        return "plant"
    if "meat" in m:
        return "meat"
    return "other"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--capture", required=True)
    args = ap.parse_args()
    cap = Path(args.capture)
    manifest = json.loads((cap / "manifest.json").read_text(encoding="utf-8"))
    samples = read_feeding(cap)
    samples.sort(key=lambda s: float(s.get("t") or 0.0))
    dt = float((manifest.get("sim") or {}).get("dt") or 1 / 15.0)
    sim_tps = int((manifest.get("sim") or {}).get("sim_tps") or 15)
    # per-bibite material intake from positive stomach deltas
    prev: dict[int, dict[str, float]] = {}
    per_mat = {"plant": {"amount": 0.0, "bites": 0, "energy": 0.0},
               "meat": {"amount": 0.0, "bites": 0, "energy": 0.0}}
    first_intake = None
    per_bibite = {}
    energy_dens = {}
    series = []
    first_sample = True
    for s in samples:
        # material properties can appear on any sample
        for name, props in (s.get("material_properties") or {}).items():
            try:
                energy_dens[classify(name)] = float(props.get("energy_density") or 0.0)
            except Exception:
                pass
        tick = s.get("tick")
        row = {"tick": tick, "t": s.get("t"), "bibites": len(s.get("bibites") or []),
               "plant_amount": (s.get("totals") or {}).get("plant_amount"),
               "meat_amount": (s.get("totals") or {}).get("meat_amount"),
               "energy_mean": None, "health_mean": None, "held_total": 0, "stomach_total": 0.0}
        energies, healths = [], []
        for b in s.get("bibites") or []:
            try:
                bid = int(b.get("id"))
            except Exception:
                continue
            mats = {k: float(v) for k, v in (b.get("stomach_materials") or {}).items()}
            old = prev.get(bid, {})
            for mat, amt in (mats.items() if not first_sample else []):
                delta = amt - old.get(mat, 0.0)
                if delta > 1e-6:
                    cat = classify(mat)
                    if cat in per_mat:
                        per_mat[cat]["amount"] += delta
                        per_mat[cat]["bites"] += 1
                        per_mat[cat]["energy"] += delta * energy_dens.get(cat, 0.0)
                    if first_intake is None:
                        first_intake = tick
            prev[bid] = mats
            e = b.get("energy")
            h = b.get("health")
            if isinstance(e, (int, float)):
                energies.append(float(e))
            if isinstance(h, (int, float)):
                healths.append(float(h))
            row["held_total"] += len(b.get("held_pellets") or [])
            row["stomach_total"] += float(b.get("stomach_total") or 0.0)
            per_bibite[bid] = per_bibite.get(bid, {"samples": 0, "energy": 0.0, "health": 0.0})
            per_bibite[bid]["samples"] += 1
            per_bibite[bid]["energy"] += float(e or 0.0)
            per_bibite[bid]["health"] += float(h or 0.0)
        first_sample = False
        row["energy_mean"] = sum(energies) / len(energies) if energies else None
        row["health_mean"] = sum(healths) / len(healths) if healths else None
        series.append(row)
    duration_ticks = max(0, (samples[-1].get("tick") or 0) - (samples[0].get("tick") or 0))
    duration_s = duration_ticks * dt
    # zone stock
    zones_start = (samples[0].get("zones") or [])
    zones_end = (samples[-1].get("zones") or [])
    stock = {"plant_start": None, "plant_end": None, "meat_start": None, "meat_end": None,
             "pellet_biomass_start": None, "pellet_biomass_end": None,
             "zone_production_rate_sum": None, "zones": len(zones_start)}
    if zones_start:
        stock["plant_start"] = sum(float(z.get("plant_amount") or 0) for z in zones_start)
        stock["meat_start"] = sum(float(z.get("meat_amount") or 0) for z in zones_start)
        stock["pellet_biomass_start"] = sum(float(z.get("pellet_biomass") or 0) for z in zones_start)
        stock["zone_production_rate_sum"] = sum(float(z.get("pellet_production") or 0) for z in zones_start)
    if zones_end:
        stock["plant_end"] = sum(float(z.get("plant_amount") or 0) for z in zones_end)
        stock["meat_end"] = sum(float(z.get("meat_amount") or 0) for z in zones_end)
        stock["pellet_biomass_end"] = sum(float(z.get("pellet_biomass") or 0) for z in zones_end)
    n_bib_means = [r["bibites"] for r in series if r.get("bibites") is not None]
    mean_pop = sum(n_bib_means) / len(n_bib_means) if n_bib_means else None
    def rate(amount):
        return amount / duration_s if duration_s > 0 else None
    summary = {
        "schema": "rustibites.parity.feeding_summary.v1",
        "capture": str(cap),
        "manifest_feeding": manifest.get("feeding"),
        "samples": len(samples),
        "sample_interval_ticks_median": None,
        "tick_range": [samples[0].get("tick"), samples[-1].get("tick")] if samples else None,
        "duration_sim_seconds": duration_s,
        "mean_population": mean_pop,
        "intake": {cat: {"amount": v["amount"], "bites_proxy": v["bites"],
                         "energy_gross": v["energy"],
                         "amount_per_sim_second": rate(v["amount"]),
                         "bites_per_sim_second": (v["bites"] / duration_s if duration_s > 0 else None),
                         "amount_per_bibite_per_sim_second": (v["amount"] / duration_s / mean_pop
                                                              if duration_s > 0 and mean_pop else None)}
                   for cat, v in per_mat.items()},
        "first_intake_tick": first_intake,
        "material_energy_densities": energy_dens,
        "stomach_and_held": {
            "stomach_total_start": series[0]["stomach_total"] if series else None,
            "stomach_total_end": series[-1]["stomach_total"] if series else None,
            "held_total_start": series[0]["held_total"] if series else None,
            "held_total_end": series[-1]["held_total"] if series else None,
            "energy_mean_start": series[0]["energy_mean"] if series else None,
            "energy_mean_end": series[-1]["energy_mean"] if series else None,
            "health_mean_start": series[0]["health_mean"] if series else None,
            "health_mean_end": series[-1]["health_mean"] if series else None,
        },
        "pellet_stock": stock,
        "counters_last": (samples[-1].get("counters") if samples else None),
        "behavior": {
            "evidence": "polling_proxy",
            "plant_bites_per_s": (per_mat["plant"]["bites"] / duration_s if duration_s > 0 else None),
            "meat_bites_per_s": (per_mat["meat"]["bites"] / duration_s if duration_s > 0 else None),
            "plant_consumed_u2_per_s": rate(per_mat["plant"]["amount"]),
            "meat_consumed_u2_per_s": rate(per_mat["meat"]["amount"]),
            "plant_consumed_e_per_s": rate(per_mat["plant"]["energy"]),
            "meat_consumed_e_per_s": rate(per_mat["meat"]["energy"]),
            "bibite_bites_per_s": None,
            "bibite_damage_per_s": None,
            "stomach_energy_gain_per_s": rate(per_mat["plant"]["energy"] + per_mat["meat"]["energy"]),
            "denominator": "simulated seconds (world total); divide by mean_population for per-capita",
            "note": "bibite_bites_per_s/damage derived from mouth total_murders/damage_dealt deltas require "
                    "per-bibite deltas; this pass exposes total_murders/murdered_area in the raw feeding stream.",
        },
        "bibites_seen": len(per_bibite),
        "inferable": {
            "yes": ["per-material stomach intake (positive deltas) = successful intake/bite proxy",
                    "consumed amount and gross chemical energy (amount x EnergyDensity)",
                    "held pellets and mouth bite timer/counter state",
                    "per-zone pellet stock + pellet_production/pellet_progress counters",
                    "energy/health/stomach trajectories"],
            "no": ["exact per-bite event + its pellet identity/amount (no game bite event/hook used)",
                   "cumulative produced-vs-recycled pellet amount: the plugin exposes production *rates* "
                   "and current stock, not monotonic creation counters; conservation needs cumulative "
                   "created/recycled counters (recommended next plugin addition)",
                   "stomach->energy conversion efficiency per bite (only gross energy x material density "
                   "can be estimated; digestion efficiency lives in the stomach/diet maths)"],
            "recommended_metric": "use per-material stomach-intake increments (bite proxy) + consumed "
                                   "amount/gross energy, cross-checked with pellet-mass conservation once the "
                                   "plugin exposes cumulative created/recycled amounts; do NOT use the old "
                                   "bite_rate_mean (zero on both sides for scenario starters, divergent on "
                                   "evolved worlds)",
        },
        "series": series,
    }
    (cap / "feeding_summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    with open(cap / "feeding_series.csv", "w", newline="", encoding="utf-8") as fh:
        w = csv.DictWriter(fh, fieldnames=list(series[0].keys()))
        w.writeheader()
        w.writerows(series)
    print(json.dumps({k: summary[k] for k in ("tick_range", "duration_sim_seconds", "mean_population",
                                              "intake", "first_intake_tick", "material_energy_densities",
                                              "stomach_and_held", "pellet_stock", "bibites_seen")}, indent=1))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

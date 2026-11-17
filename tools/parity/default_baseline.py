#!/usr/bin/env python3
"""default_baseline.py - canonical Default.zip game-vs-Rust baseline (task-98, new testing policy).

Compares the hidden-window game capture (Default.zip newgame, deterministic 1x, --feeding) with the
Rust capture (testdata/parity/rust_default_*). Outputs envelopes, first divergence, feeding rates
(per live bibite-second), and the game exact/census conservation residual. Writes
testdata/parity/default/analysis_<ticks>.json + a compact CSV series.
"""
from __future__ import annotations

import argparse
import csv
import gzip
import json
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")


def open_text(path: Path):
    path = Path(path)
    if path.is_dir():
        for name in ("feeding.jsonl.gz", "feeding.jsonl"):
            p = path / name
            if p.exists():
                path = p
                break
        else:
            return []
    if path.suffix == ".gz":
        return gzip.open(str(path), "rt", encoding="utf-8").read().splitlines()
    return path.read_text(encoding="utf-8").splitlines()


def load_feeding(path: Path):
    return [json.loads(l) for l in open_text(path) if l.strip()]


def load_ticks(path: Path):
    p = Path(path)
    if p.is_dir():
        for name in ("ticks.jsonl.gz", "ticks.jsonl"):
            if (p / name).exists():
                p = p / name
                break
    rows = {}
    for line in open_text(p):
        if not line.strip():
            continue
        r = json.loads(line)
        rows[int(r["tick"])] = r
    return rows


def events(path: Path):
    p = Path(path) / "events.jsonl"
    out = []
    if (p.parent / "events.jsonl.gz").exists():
        p = p.parent / "events.jsonl.gz"
    if p.exists():
        for line in open_text(p):
            if line.strip():
                out.append(json.loads(line))
    return out


def classify(name):
    n = str(name).lower()
    return "plant" if "plant" in n else ("meat" if "meat" in n else "other")


def game_intake(feeding):
    prev = {}
    amounts = {"plant": 0.0, "meat": 0.0}
    events_n = {"plant": 0, "meat": 0}
    dens = {}
    live_s = 0.0
    prev_t = None
    prev_pop = None
    first_tick = None
    for s in feeding:
        t = float(s.get("t") or 0)
        for name, props in (s.get("material_properties") or {}).items():
            try:
                dens[classify(name)] = float(props.get("energy_density") or 0)
            except Exception:
                pass
        pop = len(s.get("bibites") or [])
        if prev_t is not None:
            live_s += 0.5 * (pop + prev_pop) * max(0.0, t - prev_t)
        cur = {}
        for b in s.get("bibites") or []:
            cur[b.get("id")] = {k: float(v or 0) for k, v in (b.get("stomach_materials") or {}).items()}
        for bid, cm in cur.items():
            pm = prev.get(bid, {})
            for name, v in cm.items():
                d = v - pm.get(name, 0.0)
                if d > 1e-6:
                    cat = classify(name)
                    if cat in amounts:
                        amounts[cat] += d
                        events_n[cat] += 1
                        if first_tick is None:
                            first_tick = s.get("tick")
        prev, prev_t, prev_pop = cur, t, pop
    gross = {c: amounts[c] * dens.get(c, 0.0) for c in amounts}
    return {"live_seconds": live_s, "amounts": amounts, "events": events_n,
            "gross_energy": gross, "first_intake_tick": first_tick, "energy_density": dens}


def rust_feed_counters(feeding):
    if not feeding:
        return {"available": False}
    first = feeding[0].get("counters") or {}
    last = feeding[-1].get("counters") or {}
    out = {"available": True, "live_seconds": float(last.get("live_seconds") or 0)}
    for key in ("plant_bite_events", "meat_bite_events", "plant_consumed_u2", "meat_consumed_u2",
                "plant_swallow_events", "meat_swallow_events", "stomach_digested_u2",
                "stomach_energy_gain_e", "energy_gained_e", "energy_spent_e",
                "bibite_bite_events", "bibite_damage_dealt", "bibite_kills"):
        try:
            out[key] = float(last.get(key) or 0) - float(first.get(key) or 0)
        except Exception:
            out[key] = None
    return out


def envelope(rows, key):
    vals = []
    for r in rows.values():
        cur = r
        for part in key.split("."):
            if not isinstance(cur, dict) or part not in cur:
                cur = None
                break
            cur = cur[part]
        if isinstance(cur, (int, float)):
            vals.append(float(cur))
    if not vals:
        return {}
    return {"start": vals[0], "final": vals[-1], "min": min(vals), "max": max(vals)}


def first_divergence(g, r, key, tol=1e-6):
    common = sorted(set(g) & set(r))

    def get(row, path):
        cur = row
        for part in path.split("."):
            if not isinstance(cur, dict) or part not in cur:
                return None
            cur = cur[part]
        return cur
    for t in common:
        a, b = get(g[t], key), get(r[t], key)
        if a is None or b is None:
            continue
        try:
            if abs(float(a) - float(b)) > tol:
                return {"tick": t, "game": a, "rust": b, "delta": float(b) - float(a)}
        except (TypeError, ValueError):
            if a != b:
                return {"tick": t, "game": a, "rust": b}
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--game", required=True)
    ap.add_argument("--rust", required=True)
    ap.add_argument("--ticks", type=int, required=True)
    ap.add_argument("--out", default=str(ROOT / "testdata" / "parity" / "default"))
    args = ap.parse_args()
    g = load_ticks(Path(args.game))
    r = load_ticks(Path(args.rust))
    gf = load_feeding(Path(args.game))
    rf = load_feeding(Path(args.rust))
    gi = game_intake(gf)
    rc = rust_feed_counters(rf)
    last_counters = (gf[-1].get("counters") if gf else {}) or {}
    exact = {k: last_counters.get(k) for k in
             ("initial_energy", "produced_energy_exact", "consumed_energy_exact", "recycled_energy_exact",
              "decayed_energy_exact", "culled_energy_exact", "conservation_residual_energy",
              "current_pellet_energy") if k in last_counters}
    exact["evidence"] = last_counters.get("evidence_exact")
    out = {
        "ticks": args.ticks,
        "game": {"path": str(args.game), "counts": envelope(g, "counts.bibites"),
                 "energy": envelope(g, "vitals.energy_mean"), "health": envelope(g, "vitals.health_mean"),
                 "plant_n": envelope(g, "pellets.plant_n"), "plant_biomass": envelope(g, "pellets.plant_biomass"),
                 "events": len(events(Path(args.game))),
                 "feeding": gi, "exact_counters": exact},
        "rust": {"path": str(args.rust), "counts": envelope(r, "counts.bibites"),
                 "energy": envelope(r, "vitals.energy_mean"), "health": envelope(r, "vitals.health_mean"),
                 "plant_n": envelope(r, "pellets.plant_n"), "plant_biomass": envelope(r, "pellets.plant_biomass"),
                 "feeding": rc},
        "first_divergence": {k: first_divergence(g, r, k) for k in
                             ("counts.bibites", "vitals.energy_mean", "vitals.health_mean",
                              "pellets.plant_n", "pellets.plant_biomass")},
    }
    # normalized rates
    for side, data, blocks in (("game", gi, g), ("rust", rc, r)):
        if side == "game":
            live = data["live_seconds"]
            out[side]["rates"] = {
                "plant_bites_per_live_s": data["events"]["plant"] / live if live > 0 else None,
                "plant_consumed_u2_per_live_s": data["amounts"]["plant"] / live if live > 0 else None,
                "plant_consumed_e_per_live_s": data["gross_energy"]["plant"] / live if live > 0 else None,
                "meat_bites_per_live_s": data["events"]["meat"] / live if live > 0 else None,
                "meat_consumed_u2_per_live_s": data["amounts"]["meat"] / live if live > 0 else None,
            }
        else:
            live = rc.get("live_seconds") or 0
            out[side]["rates"] = {
                "plant_bites_per_live_s": (rc.get("plant_bite_events") or 0) / live if live else None,
                "plant_consumed_u2_per_live_s": (rc.get("plant_consumed_u2") or 0) / live if live else None,
                "stomach_energy_gain_e_per_live_s": (rc.get("stomach_energy_gain_e") or 0) / live if live else None,
                "energy_gained_e_per_live_s": (rc.get("energy_gained_e") or 0) / live if live else None,
                "energy_spent_e_per_live_s": (rc.get("energy_spent_e") or 0) / live if live else None,
            }
    # conservation residual for the game exact counters, when present
    if exact:
        try:
            lhs = float(exact.get("initial_energy") or 0) + float(exact.get("produced_energy_exact") or 0)
            rhs = (float(exact.get("consumed_energy_exact") or 0) + float(exact.get("recycled_energy_exact") or 0)
                   + float(exact.get("decayed_energy_exact") or 0) + float(exact.get("culled_energy_exact") or 0)
                   + float(exact.get("current_pellet_energy") or 0))
            out["game"]["conservation"] = {"evidence": exact.get("evidence"), "initial_energy": exact.get("initial_energy"),
                                           "produced_energy_exact": exact.get("produced_energy_exact"),
                                           "consumed_energy_exact": exact.get("consumed_energy_exact"),
                                           "recycled_energy_exact": exact.get("recycled_energy_exact"),
                                           "decayed_energy_exact": exact.get("decayed_energy_exact"),
                                           "culled_energy_exact": exact.get("culled_energy_exact"),
                                           "current_pellet_energy": exact.get("current_pellet_energy"),
                                           "lhs": lhs, "rhs": rhs,
                                           "residual_from_plugin": exact.get("conservation_residual_energy"),
                                           "residual_recomputed": lhs - rhs,
                                           "relative_residual": (lhs - rhs) / max(1e-9, lhs)}
        except Exception as exc:
            out["game"]["conservation_error"] = str(exc)
    outp = Path(args.out) / f"analysis_{args.ticks}.json"
    outp.parent.mkdir(parents=True, exist_ok=True)
    outp.write_text(json.dumps(out, indent=2), encoding="utf-8")
    print(json.dumps({k: out[k] for k in ("ticks", "first_divergence")}, indent=1))
    print("game counts/energy:", out["game"]["counts"], out["game"]["energy"])
    print("rust counts/energy:", out["rust"]["counts"], out["rust"]["energy"])
    print("game feeding:", json.dumps(out["game"]["feeding"])[:500])
    print("game rates:", out["game"].get("rates"))
    print("rust feeding/rates:", json.dumps(out["rust"]["feeding"])[:500], out["rust"].get("rates"))
    print("game conservation:", out["game"].get("conservation"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

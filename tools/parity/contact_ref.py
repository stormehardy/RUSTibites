#!/usr/bin/env python3
"""contact_ref.py - identical-base Default feeding/contact comparison (task-105).

Compares the hidden game capture from `testdata/parity/default/rust_seed.zip` with the Rust capture from
the same save, for 600/3000 ticks: first intake tick, fed-bibite count, bite events, u²/bite, idle gaps,
and the game's per-bibite mouth-contact persistence metrics (n_in_mouth / contact ticks / enter events /
first contact tick). Rust currently exposes feeding counters but not the contact-persistence fields
(body-dev task-106 owns the writer); those are reported as unavailable rather than guessed.

Usage: python tools/parity/contact_ref.py --game testdata/parity/default/base_600 \
           --rust testdata/parity/default/rust_base_600 --ticks 600 [--out ...]
"""
from __future__ import annotations

import argparse
import gzip
import json
import statistics
from pathlib import Path


def lines(path: Path):
    if path.is_dir():
        for n in ("feeding.jsonl.gz", "feeding.jsonl"):
            p = path / n
            if p.exists():
                return (gzip.open(p, "rt", encoding="utf-8") if n.endswith(".gz")
                        else open(p, "rt", encoding="utf-8")).read().splitlines()
        return []
    return []


def classify(name):
    n = str(name).lower()
    return "plant" if "plant" in n else ("meat" if "meat" in n else "other")


def capture_manifest(path: Path):
    p = Path(path) / "manifest.json"
    if p.exists():
        try:
            return json.loads(p.read_text(encoding="utf-8"))
        except Exception:
            pass
    return {}


def stats(path: Path):
    rows = [json.loads(l) for l in lines(path) if l.strip()]
    manifest = capture_manifest(Path(path))
    sim_tps = int(((manifest.get("sim") or {}).get("sim_tps")) or 30)
    dt = 1.0 / sim_tps
    first = {"path": str(path), "samples": len(rows), "sim_tps": sim_tps}
    if not rows:
        first["error"] = "no feeding stream"
        return first
    first_t = float(rows[0].get("t") or 0.0)
    base_tick = int(round(first_t / dt))
    first["base_absolute_tick"] = base_tick
    prev = {}
    events = []
    first_tick_by_bib = {}
    fed = set()
    contact = {"n_in_mouth_max": 0, "contact_ticks": 0, "plant_contact_ticks": 0,
               "enter_events": 0, "first_contact_tick": None, "first_plant_contact_tick": None,
               "contact_available": False}
    first_sample = True
    for snap in rows:
        tick = snap.get("tick")
        cur = {}
        for b in snap.get("bibites") or []:
            bid = b.get("id")
            mats = {k: float(v or 0) for k, v in (b.get("stomach_materials") or {}).items()}
            cur[bid] = mats
            n = b.get("n_in_mouth")
            if n is not None:
                contact["contact_available"] = True
                contact["n_in_mouth_max"] = max(contact["n_in_mouth_max"], int(n))
            for k in ("mouth_contact_ticks", "plant_contact_ticks", "mouth_contact_enter_events"):
                if b.get(k) is not None:
                    contact[k.replace("mouth_contact_", "").replace("plant_contact_", "plant_")
                            .replace("_enter_events", "_enter_events")] = b.get(k)
            if b.get("first_mouth_contact_tick") not in (None, -1):
                ft = b["first_mouth_contact_tick"] - base_tick
                if contact["first_contact_tick"] is None or ft < contact["first_contact_tick"]:
                    contact["first_contact_tick"] = ft
            if b.get("first_plant_contact_tick") not in (None, -1):
                ft = b["first_plant_contact_tick"] - base_tick
                if contact["first_plant_contact_tick"] is None or ft < contact["first_plant_contact_tick"]:
                    contact["first_plant_contact_tick"] = ft
        for bid, mats in cur.items():
            pm = prev.get(bid, {})
            for mat, v in mats.items():
                d = v - pm.get(mat, 0.0)
                if (not first_sample) and d > 1e-6 and classify(mat) in ("plant", "meat"):
                    events.append({"tick": tick, "id": bid, "kind": classify(mat), "amount": d})
                    fed.add(bid)
                    first_tick_by_bib.setdefault(bid, tick)
        prev = cur
        first_sample = False
    amounts = [e["amount"] for e in events]
    gaps = [b["tick"] - a["tick"] for a, b in zip(events, events[1:])]
    first["first_intake_tick"] = min((e["tick"] for e in events), default=None)
    first["first_intake_by_bibite"] = first_tick_by_bib
    first["fed_bibites"] = len(fed)
    first["bite_events"] = len(events)
    first["consumed_u2"] = sum(amounts)
    first["mean_u2_per_bite"] = statistics.mean(amounts) if amounts else None
    first["median_idle_gap_ticks"] = statistics.median(gaps) if gaps else None
    first["mean_idle_gap_ticks"] = statistics.mean(gaps) if gaps else None
    first["contact"] = contact
    # aggregate the contact counters from the last sample (they are cumulative per bibite)
    last = rows[-1]
    totals = {"contact_ticks": 0, "plant_contact_ticks": 0, "enter_events": 0}
    for b in last.get("bibites") or []:
        totals["contact_ticks"] += int(b.get("mouth_contact_ticks") or 0)
        totals["plant_contact_ticks"] += int(b.get("plant_contact_ticks") or 0)
        totals["enter_events"] += int(b.get("mouth_contact_enter_events") or 0)
    first["contact_totals_last"] = totals
    first["per_bibite_last"] = [
        {"id": b.get("id"), "contact_ticks": b.get("mouth_contact_ticks"),
         "plant_contact_ticks": b.get("plant_contact_ticks"),
         "enter_events": b.get("mouth_contact_enter_events"),
         "first_contact_tick": b.get("first_mouth_contact_tick"),
         "energy": b.get("energy")}
        for b in (rows[-1].get("bibites") or [])]
    first["contact_ticks_per_enter"] = (totals["contact_ticks"] / totals["enter_events"]
                                        if totals["enter_events"] else None)
    return first


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--game", required=True)
    ap.add_argument("--rust", required=True)
    ap.add_argument("--ticks", type=int, required=True)
    ap.add_argument("--out", default=None)
    args = ap.parse_args()
    g = stats(Path(args.game))
    r = stats(Path(args.rust))
    out = {"schema": "rustibites.parity.contact_ref.v1", "ticks": args.ticks,
           "base": "testdata/parity/default/rust_seed.zip",
           "game": g, "rust": r,
           "comparison": {
               "first_intake_tick": [g.get("first_intake_tick"), r.get("first_intake_tick")],
               "fed_bibites": [g.get("fed_bibites"), r.get("fed_bibites")],
               "bite_events": [g.get("bite_events"), r.get("bite_events")],
               "mean_u2_per_bite": [g.get("mean_u2_per_bite"), r.get("mean_u2_per_bite")],
               "median_idle_gap_ticks": [g.get("median_idle_gap_ticks"), r.get("median_idle_gap_ticks")],
           },
           "contact_persistence": {
               "game_available": g.get("contact", {}).get("contact_available"),
               "rust_available": r.get("contact", {}).get("contact_available"),
               "note": "Rust contact persistence fields are body-dev task-106 scope; this fixture establishes "
                       "the identical-base game contact reference. Game metrics: mouth_targets/n_in_mouth, "
                       "cumulative contact/plant ticks, enter events and first contact ticks.",
           }}
    outp = Path(args.out) if args.out else Path(args.game).parent / f"base_analysis_{args.ticks}.json"
    outp.write_text(json.dumps(out, indent=2), encoding="utf-8")
    print(json.dumps({k: out[k] for k in ("ticks", "comparison", "game", "rust")}, indent=1, default=str)[:5000])
    print("wrote", outp)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""physics2_report.py - per-contact geometry report for task-132.

Reads `testdata/parity/default/trace_phase2/physics2.jsonl` (game collider census + per-tick
Physics2D.GetContacts rows: part name/collider/pellet amount/material/point/normal/relative velocity/
separation/impulse/persistent) and the paired `game_trace.jsonl` for body poses/nearest_plant. Emits
`physics2_report.json/.md` with first-contact geometry for the focus ids, per-part census, and a comparison
table against body-dev's Rust first-enter numbers. Usage:
  python tools/parity/physics2_report.py [--dir testdata/parity/default/trace_phase2]
"""
from __future__ import annotations

import argparse
import json
import math
from collections import defaultdict
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
FOCUS = (379567021, 465701042)
# body-dev's Rust first-enter reference (task-130)
RUST_REF = {
    465701042: {"tick": 66, "impulse": 2.349, "nearest": 9.34, "pellet_radius": 4.38},
    379567021: {"tick": 99, "impulse": 2.1, "nearest": None, "pellet_radius": None},
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", default=str(ROOT / "testdata/parity/default/trace_phase2"))
    args = ap.parse_args()
    d = Path(args.dir)
    rows = [json.loads(l) for l in open(d / "physics2.jsonl", encoding="utf-8")]
    census = defaultdict(list)
    contacts = defaultdict(list)
    for r in rows:
        t = r.get("type")
        if t == "census":
            census[r["id"]].append(r)
        elif t == "contact":
            contacts[r["id"]].append(r)
    # body poses from game trace (dedupe by tick/id)
    pose = {}
    for line in open(d / "game_trace.jsonl", encoding="utf-8"):
        r = json.loads(line)
        if "trace_header" in r:
            continue
        key = (r.get("tick"), r.get("id"))
        pose[key] = r
    out = {"schema": "rustibites.parity.physics2_report.v1", "census": {str(k): v for k, v in census.items()},
           "focus": {}}
    for eid in FOCUS:
        rows_id = sorted(contacts.get(eid, []), key=lambda r: (r["tick"], r["part"]))
        first = rows_id[0] if rows_id else None
        # first non-trigger body-part contact (exclude Mouth box)
        first_solid = next((r for r in rows_id if r.get("part_name") not in ("Mouth",)), None)
        first_mouth = next((r for r in rows_id if r.get("part_name") == "Mouth"), None)
        def enrich(r):
            if not r:
                return None
            amt = r.get("pellet_amount")
            radius = 2.5 * math.sqrt(float(amt) / 20.0) if amt else None
            p = pose.get((r["tick"], eid))
            pt = r.get("point") or {}
            dist = None
            if p and pt:
                dist = math.hypot(pt.get("x", 0) - p.get("x", 0), pt.get("y", 0) - p.get("y", 0))
            return {**r, "pellet_radius_from_amount": radius,
                    "contact_point_dist_from_body": dist,
                    "game_nearest_plant_at_tick": (p or {}).get("nearest_plant"),
                    "game_body_pos": [p.get("x"), p.get("y")] if p else None,
                    "game_vel": [p.get("vx"), p.get("vy")] if p else None,
                    "game_accel_rotate": [(p or {}).get("accelerate"), (p or {}).get("rotate")]}
        out["focus"][str(eid)] = {"rust_reference": RUST_REF.get(eid), "first_contact": enrich(first),
                                  "first_solid_part_contact": enrich(first_solid),
                                  "first_mouth_contact": enrich(first_mouth),
                                  "first_10": [enrich(r) for r in rows_id[:10]],
                                  "census": census.get(eid, [])}
    (d / "physics2_report.json").write_text(json.dumps(out, indent=2), encoding="utf-8")
    lines = ["# Per-contact geometry report (task-132)", "",
             "Game rows are `Physics2D.GetContacts` per bibite collider part each tick; pellet radius computed "
             "as `2.5*sqrt(amount/20)` from the pellet amount. Census lists each part collider/offset/size.", ""]
    for eid in FOCUS:
        f = out["focus"][str(eid)]
        lines += [f"## {eid}", "", f"Rust first enter reference: `{f['rust_reference']}`", ""]
        for label in ("first_contact", "first_solid_part_contact", "first_mouth_contact"):
            r = f[label]
            if not r:
                lines.append(f"- {label}: none")
                continue
            lines.append(f"- {label}: tick {r['tick']} part `{r.get('part_name')}`/{r.get('collider')} "
                         f"pellet amount {r.get('pellet_amount')} radius {r.get('pellet_radius_from_amount')}, "
                         f"impulse {r.get('normal_impulse')}, separation {r.get('separation')}, "
                         f"nearest {r.get('game_nearest_plant_at_tick')}, point-dist {r.get('contact_point_dist_from_body')}, "
                         f"persistent {r.get('persistent')}")
        lines += ["", "Census:", ""]
        for c in f["census"]:
            lines.append(f"- part {c.get('part')} `{c.get('part_name') or c.get('name')}` {c.get('collider')} "
                         f"layer {c.get('layer')} trigger {c.get('is_trigger')} size {c.get('size')} "
                         f"radius {c.get('radius')} offset {c.get('offset')}")
        lines.append("")
    (d / "physics2_report.md").write_text("\n".join(lines), encoding="utf-8")
    print("\n".join(lines[:40]))


if __name__ == "__main__":
    main()

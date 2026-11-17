#!/usr/bin/env python3
"""exp_growth.py - grow a template (default: the Oyster) in the REAL game, like crates/bibites-io/tests/oyster_probe.rs does in the port.

A baby (d2Size = growthAtBirth * SizeRatio^2, full vitals as the port's baby has) is placed in the middle of 400 plant pellets of 20
laid on the same spiral, and the game runs at speed 1 for N seconds; every 5 s the bibite's growth (d2Size / SizeRatio^2), energy,
health, eggs laid and the number of bibites are printed. Output: testdata/parity/experiments/growth/<name>.json.

    python tools/parity/exp_growth.py "C:\\Users\\Storme\\Downloads\\Oyster (Overkill) (Old) 6.bb8template" --seconds 100
"""
from __future__ import annotations

import argparse
import copy
import json
import math
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import exp_behavior as eb  # noqa: E402


def template_entity(base: dict, tpl: dict, d2: float, health: float, energy: float, ident: int) -> dict:
    d = copy.deepcopy(base)
    d["genes"]["genes"] = dict(tpl["genes"])
    d["genes"]["gen"] = 0
    sys.path.insert(0, str(eb.DEFAULT_API_HOME))
    from bibites_api.core import templates  # noqa: E402

    d["brain"] = templates.brain_from_template(tpl)
    b = d["body"]
    b.update({"id": ident, "dead": False, "dying": False, "d2Size": d2, "health": health, "energy": energy,
              "fatReservesAmount": 0.0, "attackedDmg": 0.0, "timesAttacked": 0, "totalDamageSuffered": 0.0})
    b["eggLayer"] = {"eggProgress": 0.0, "nEggsLaid": 0, "children": []}
    for k in ("clock",):
        if k in d:
            d[k] = {"tic": 0, "ticProgress": 0.0, "timeAlive": 0.0, "chronoTime": 0.0}
    d["transform"]["position"] = [0.0, 0.0]
    d["rb2d"].update({"px": 0.0, "py": 0.0, "vx": 0.0, "vy": 0.0})
    return d


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("template")
    ap.add_argument("--seconds", type=float, default=100.0)
    ap.add_argument("--growth-at-birth", type=float, default=0.087)
    ap.add_argument("--health", type=float, default=0.022)
    ap.add_argument("--energy", type=float, default=0.03)
    ap.add_argument("--base", default=str(eb.DEFAULT_BASE))
    ap.add_argument("--api-home", default=str(eb.DEFAULT_API_HOME))
    ap.add_argument("--refill", action="store_true", help="set energy and health to full every simulated second (takes foraging out)")
    ap.add_argument("--refill-until", type=float, default=1e9, help="stop refilling after this many seconds")
    ap.add_argument("--step", type=float, default=5.0, help="seconds between samples")
    ap.add_argument("--speed", type=float, default=1.0)
    ap.add_argument("--port", type=int, default=17800)
    ap.add_argument("--work-dir", default=str(eb.PROJECT / "testdata" / "parity" / "_work"))
    ap.add_argument("--start-timeout", type=float, default=300.0)
    args = ap.parse_args()

    tpl = json.load(open(args.template, encoding="utf-8-sig"))
    sr = float(tpl["genes"]["SizeRatio"])
    session = eb.Session(args)
    session.start()
    rows = []
    try:
        client = session.client
        session.load_base()
        ents = eb.living(client)
        base = eb.entity_json(client, int(ents[0]["h"]))
        eb.delete_all_entities(client)
        try:
            client.pellets_delete(region={"x": 0, "y": 0, "r": 1e6})
        except Exception as exc:  # noqa: BLE001
            eb.log(f"could not clear pellets: {exc}")
        pellets = []
        for i in range(0 if args.refill else 400):
            a = i * 2.399
            r = 4.0 + 2.2 * math.sqrt(i)
            pellets.append((r * math.cos(a), r * math.sin(a), 20.0, 0))
        if pellets:
            client.spawn_pellets(pellets)
        ident = int(base["body"]["id"]) + 1
        ent = template_entity(base, tpl, args.growth_at_birth * sr * sr, args.health, args.energy, ident)
        created = client.spawn([ent])
        eb.log(f"spawned {created}")
        handle = int(created[0]["h"]) if isinstance(created[0], dict) and "h" in created[0] else None
        t = 0.0
        while t <= args.seconds:
            if t > 0:
                session.run_seconds(args.step)
            alive = {int(e.get("id", 0)): e for e in eb.living(client)}
            me = alive.get(ident)
            if me is not None:
                d = eb.entity_json(client, int(me["h"]))
                b = d["body"]
                row = {"t": t, "growth": b["d2Size"] / (sr * sr), "energy": b["energy"], "d2": b["d2Size"], "pos": [me.get("x"), me.get("y")], "vel": [d["rb2d"].get("vx"), d["rb2d"].get("vy")], "rot": d["transform"].get("rotation"), "maxE": me.get("maxEnergy"), "maxH": me.get("maxHealth"), "health": b["health"],
                       "laid": b["eggLayer"]["nEggsLaid"], "womb": b["eggLayer"]["eggProgress"], "bibites": len(alive)}
            else:
                row = {"t": t, "gone": True, "bibites": len(alive)}
            rows.append(row)
            eb.log(json.dumps(row))
            if args.refill and t < args.refill_until:
                cur = {int(e.get("id", 0)): e for e in eb.living(client)}.get(ident)
                if cur is not None:
                    client.vitals([int(cur["h"])], energyFraction=1.0, healthFraction=1.0)
            t += args.step
    finally:
        session.stop()
    out = eb.OUT_ROOT / "growth"
    out.mkdir(parents=True, exist_ok=True)
    (out / (Path(args.template).stem + ".json")).write_text(json.dumps(rows, indent=1))


if __name__ == "__main__":
    main()

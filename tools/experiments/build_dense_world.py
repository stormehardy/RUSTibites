#!/usr/bin/env python3
"""build_dense_world.py - build dense founder worlds from a real save (read-only source).

Used by the ecology validation (task-57) to test high-density founder populations (brain-dev's
1k/2k/5k collapse) in a *real* ecology: take the lol.zip save (1 plant zone, radius 0.1*SimSize at the
world centre, fertility 100), duplicate its living bibite genomes to N founders placed inside the zone
(so they start near food, like the synthetic benchmark), give each a unique body id, and write a save
zip under testdata/experiments/dense/. virginSpawnRate is frozen to 0 so only the founders are present.

The source save is never modified; the output is a synthetic test fixture (gitignored).
"""
from __future__ import annotations

import argparse
import json
import random
import struct
import zipfile
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
DEFAULT_SRC = ROOT / "testdata" / "real" / "saves" / "lol.zip"
DEFAULT_OUT = ROOT / "testdata" / "experiments" / "dense"


def f32(x):
    return struct.unpack("<f", struct.pack("<f", float(x)))[0]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--src", default=str(DEFAULT_SRC))
    ap.add_argument("--count", type=int, required=True)
    ap.add_argument("--out", default=None)
    ap.add_argument("--seed", type=int, default=12345)
    ap.add_argument("--placement-radius", type=float, default=0.9,
                    help="fraction of the zone radius used for founder placement (default 0.9)")
    args = ap.parse_args()
    src = Path(args.src)
    out = Path(args.out) if args.out else DEFAULT_OUT / f"founders_{args.count}.zip"
    out.parent.mkdir(parents=True, exist_ok=True)
    if not src.exists():
        raise SystemExit(f"source save not found: {src}")

    with zipfile.ZipFile(src) as z:
        entries = {name: z.read(name) for name in z.namelist()}
    settings = json.loads(entries["settings.bb8settings"].decode("utf-8-sig"))
    scene = json.loads(entries["scene.bb8scene"].decode("utf-8-sig"))
    ind = settings.get("independents") or {}
    ind.setdefault("virginSpawnRate", {})["Value"] = 0.0
    sim_size = float((ind.get("SimulationSize") or {}).get("Value") or 2000.0)
    zones = settings.get("zones") or []
    if not zones:
        raise SystemExit("source save has no zones; cannot place founders near food")
    zone = zones[0]
    cx = float(zone.get("posX") or 0.0) * sim_size
    cy = float(zone.get("posY") or 0.0) * sim_size
    zr = float(zone.get("radius") or 0.1) * sim_size if zone.get("radiusIsRelative", True) else float(zone.get("radius") or 200.0)
    place_r = max(1.0, zr * args.placement_radius)

    template_names = sorted(n for n in entries if n.startswith("bibites/") and n.endswith(".bb8"))
    if not template_names:
        raise SystemExit("source save has no bibite files")

    def is_living(tpl: dict) -> bool:
        body = tpl.get("body") or {}
        return (not body.get("dead") and not body.get("dying")
                and float(body.get("corpseEnergyOffset") or 0.0) == 0.0
                and float(body.get("health") or 0.0) > 0.0
                and float(body.get("energy") or 0.0) > 0.0)

    all_templates = [json.loads(entries[n].decode("utf-8-sig")) for n in template_names]
    templates = [t for t in all_templates if is_living(t)]
    if not templates:
        raise SystemExit("source save has no LIVING bibite files to use as founders")
    print(f"living founder templates: {len(templates)}/{len(all_templates)} bibite files "
          f"({len(all_templates) - len(templates)} corpses skipped)")

    rng = random.Random(args.seed)
    new_entries = {}
    for name, data in entries.items():
        if name.startswith("bibites/") or name.startswith("eggs/"):
            continue
        new_entries[name] = data
    for i in range(args.count):
        tpl = json.loads(json.dumps(templates[i % len(templates)]))
        while True:
            x = rng.uniform(-1.0, 1.0)
            y = rng.uniform(-1.0, 1.0)
            if x * x + y * y <= 1.0:
                break
        px = cx + x * place_r
        py = cy + y * place_r
        rot = rng.uniform(0.0, 360.0)
        tpl["transform"]["position"] = [f32(px), f32(py)]
        tpl["transform"]["rotation"] = f32(rot)
        tpl["rb2d"].update({"px": f32(px), "py": f32(py), "vx": 0.0, "vy": 0.0, "r": f32(rot)})
        tpl["body"]["id"] = -1_000_000_000 - i
        tpl["body"]["dead"] = False
        tpl["body"]["dying"] = False
        # founders are young adults (the source save is ~470k s old; pre-aged founders would confound
        # the density experiment with senescence)
        tpl["clock"]["timeAlive"] = 0.0
        tpl["clock"]["tic"] = 0
        tpl["clock"]["chronoTime"] = 0.0
        new_entries[f"bibites/bibite_{i}.bb8"] = json.dumps(tpl, separators=(",", ":")).encode("utf-8")
    scene["nBibites"] = args.count
    new_entries["scene.bb8scene"] = json.dumps(scene, separators=(",", ":")).encode("utf-8")
    new_entries["settings.bb8settings"] = json.dumps(settings, separators=(",", ":")).encode("utf-8")

    tmp = out.with_suffix(".zip.tmp")
    with zipfile.ZipFile(tmp, "w", compression=zipfile.ZIP_DEFLATED) as zout:
        for name, data in new_entries.items():
            zout.writestr(name, data)
    tmp.replace(out)
    meta = {
        "schema": "rustibites.ecology.dense_founders.v1",
        "source": str(src), "count": args.count, "seed": args.seed,
        "zone": {"center": [cx, cy], "radius": zr, "placement_radius": place_r},
        "simulation_size": sim_size,
        "out": str(out), "size": out.stat().st_size,
        "note": "founders are duplicates of the source save's living bibites placed inside the first plant zone; "
                "virginSpawnRate=0; body ids are synthetic unique ids; source save untouched",
    }
    (out.with_suffix(".json")).write_text(json.dumps(meta, indent=2), encoding="utf-8")
    print(json.dumps(meta, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

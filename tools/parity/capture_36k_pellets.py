#!/usr/bin/env python3
"""capture_36k_pellets.py - game pellet positions/radii distributions through the 36k Default run.

Hidden Default newgame seed12345, speed8 until simTime900 (~36k ticks). Periodically reads the full pellet
map (x,y,material,amount) and records: count, radial histogram, radius stats, and full snapshots at early/late
(plus every Nth sample). Radius is derived from MatterPellet sizeFactor/radiusFactor: sqrt(max(amount,0)/20)
(game scale). Raw under testdata; tracked schema/summary under assets/game/reference/ecology/pellets/.
"""
from __future__ import annotations

import json
import math
import sys
import time
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import capture as cap  # noqa: E402

DEFAULT = ROOT / "testdata/real/Default.zip"
RAW = ROOT / "testdata/parity/default/ecology_36k/pellets"
REF = ROOT / "assets/game/reference/ecology/pellets"
BINS = [0, 250, 500, 750, 1000, 1250, 1500, 1750, 2000, 2250, 2500, 3000, 3500, 4000, 5000]


def build_args(ns):
    ns.work_dir = str(ROOT / "testdata/parity/_work")
    ns.port = 17800
    ns.plugin_port = 17801
    ns.no_plugin = False
    ns.attach = False
    ns.start_timeout = 300.0
    ns.keep_game = False
    return ns


def pct(vals, q):
    if not vals:
        return None
    v = sorted(vals)
    return v[min(len(v) - 1, max(0, int(q * (len(v) - 1))))]


def main():
    RAW.mkdir(parents=True, exist_ok=True)
    REF.mkdir(parents=True, exist_ok=True)
    B, _, Lo, La = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    s = cap.WorldSession(build_args(__import__("argparse").Namespace()), B, Lo, La)
    samples = []
    snapshots = []
    try:
        c = s.open()
        s.plugin.pacing(True)
        c.new_game(DEFAULT, name="parity_36k_pellets", timeout=300)
        c.set_time(paused=True)
        c.seed(12345)
        c.run_start(until={"simTime": 900.0}, speed=8.0, sample=None, pause=True)
        t0 = time.time()
        last_tick = -1
        while time.time() - t0 < 900:
            st = c.run_state(timeout=15.0)
            tick = int((st.get("simTime") or 0) * 30)
            if tick != last_tick:
                pellets = c.map_pellets()
                xs = [float(p[0]) for p in pellets]
                ys = [float(p[1]) for p in pellets]
                amts = [float(p[3]) for p in pellets]
                radii = [math.sqrt(max(a, 0.0) / 20.0) for a in amts]
                rs = [math.hypot(x, y) for x, y in zip(xs, ys)]
                hist = [0] * (len(BINS) - 1)
                for r in rs:
                    for i in range(len(BINS) - 1):
                        if BINS[i] <= r < BINS[i + 1]:
                            hist[i] += 1
                            break
                rec = {"tick": tick, "simTime": st.get("simTime"), "state": st.get("state"),
                       "count": len(pellets), "radial_bins": BINS, "radial_hist": hist,
                       "radius_p50": pct(rs, 0.5), "radius_p90": pct(rs, 0.9), "radius_max": max(rs) if rs else None,
                       "pellet_radius_p50": pct(radii, 0.5), "pellet_radius_p90": pct(radii, 0.9),
                       "amount_sum": sum(amts), "amount_p50": pct(amts, 0.5)}
                samples.append(rec)
                if len(samples) == 1 or len(samples) % 10 == 0:
                    snapshots.append(rec | {"pellets": [{"x": x, "y": y, "material": int(p[2]), "amount": a,
                                                         "radius": math.sqrt(max(a, 0.0) / 20.0)}
                                                        for p, x, y, a in zip(pellets, xs, ys, amts)]})
                last_tick = tick
            if st.get("state") not in ("running", "stopping"):
                break
            time.sleep(1.0)
        final = c.run_state(timeout=15.0)
        (RAW / "samples.jsonl").write_text("\n".join(json.dumps(r) for r in samples), encoding="utf-8")
        (RAW / "snapshots.jsonl").write_text("\n".join(json.dumps(r) for r in snapshots), encoding="utf-8")
        (RAW / "final_state.json").write_text(json.dumps(final, indent=2), encoding="utf-8")
        summary = {"schema": "rustibites.assets.ecology_pellets.v1",
                   "capture": {"save": "Default.zip newgame", "seed": 12345, "requested_speed": 8,
                               "until_simTime": 900, "samples": len(samples), "snapshots": len(snapshots),
                               "final": final, "bins": BINS,
                               "pellet_radius_formula": "sqrt(max(amount,0)/20) (MatterPellet sizeFactor/radiusFactor)",
                               "raw": {"samples": "testdata/parity/default/ecology_36k/pellets/samples.jsonl",
                                       "snapshots": "testdata/parity/default/ecology_36k/pellets/snapshots.jsonl"}},
                   "early": samples[0] if samples else None, "late": samples[-1] if samples else None,
                   "note": "Full pellet map per sample; snapshots every 10th sample include x/y/material/amount/radius. "
                           "Use with the per-bibite occupancy samples for CentricGradual/near-pellet distribution comparison."}
        (REF / "pellet_samples.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
        (REF / "README.md").write_text(
            "# Game pellet distribution through 36k Default\n\nHidden Default seed12345; pellet count/radial histogram/"
            "pellet radius summary per sample, full-map snapshots every 10th sample (x/y/material/amount/radius). "
            "Radius derived from amount via sqrt(amount/20) (MatterPellet). Raw under "
            "testdata/parity/default/ecology_36k/pellets; tracked summary `pellet_samples.json`.\n", encoding="utf-8")
        print(json.dumps({"samples": len(samples), "snapshots": len(snapshots),
                          "early": summary["early"], "late": summary["late"]}, indent=1)[:5000])
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

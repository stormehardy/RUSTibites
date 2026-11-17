#!/usr/bin/env python3
"""capture_36k_contact_ext.py - per-bibite feeding/contact/position samples through 36k Default ticks.

Hidden Default newgame seed 12345, run at speed 8 until simTime 900 s (~36k ticks at the observed effective
tick rate). Every ~2 s wall: bridge bibite table (id/x/y/energy/health), parity plugin /d/feeding (stomach,
n_in_mouth, mouth/plant contact ticks, bite counters) and nearest plant pellets per live bibite. Raw JSONL
under testdata/parity/default/ecology_36k/contact_ext/ (gitignored); tracked schema/summary under
assets/game/reference/ecology/contact_ext/. Hidden-only.
"""
from __future__ import annotations

import json
import sys
import time
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import capture as cap  # noqa: E402

DEFAULT = ROOT / "testdata/real/Default.zip"
RAW = ROOT / "testdata/parity/default/ecology_36k/contact_ext"
REF = ROOT / "assets/game/reference/ecology/contact_ext"


def build_args(ns):
    ns.work_dir = str(ROOT / "testdata/parity/_work")
    ns.port = 17800
    ns.plugin_port = 17801
    ns.no_plugin = False
    ns.attach = False
    ns.start_timeout = 300.0
    ns.keep_game = False
    return ns


def main():
    RAW.mkdir(parents=True, exist_ok=True)
    REF.mkdir(parents=True, exist_ok=True)
    B, _, Lo, La = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    s = cap.WorldSession(build_args(__import__("argparse").Namespace()), B, Lo, La)
    rows = []
    try:
        c = s.open()
        s.plugin.pacing(True)
        c.new_game(DEFAULT, name="parity_36k_contact", timeout=300)
        c.set_time(paused=True)
        c.seed(12345)
        run = c.run_start(until={"simTime": 900.0}, speed=8.0, sample=None, pause=True)
        last_tick = -1
        t0 = time.time()
        while time.time() - t0 < 900:
            st = c.run_state(timeout=15.0)
            tick = int((st.get("simTime") or 0) * 30)
            if tick != last_tick:
                table = c.bibite_table(genes=False, eggs=False)
                cols = table.get("columns") or {}
                ids = cols.get("id") or []
                by_id = {}
                for i, bid in enumerate(ids):
                    by_id[bid] = {k: (vals[i] if isinstance(vals, list) and i < len(vals) else None)
                                  for k, vals in cols.items()}
                feeding = s.plugin.feeding()
                try:
                    pellet_map = c.map_pellets()
                except Exception:
                    pellet_map = []
                rec = {"tick": tick, "simTime": st.get("simTime"), "state": st.get("state"),
                       "counts": st.get("counts"), "bibites": []}
                for fb in feeding.get("bibites") or []:
                    bid = fb.get("id")
                    tb = by_id.get(bid) or {}
                    x = tb.get("x"); y = tb.get("y")
                    item = {"id": bid, "x": x, "y": y, "energy": tb.get("energy"), "health": tb.get("health"),
                            "stomach_total": fb.get("stomach_total"), "n_stomach": fb.get("n_stomach"),
                            "n_in_mouth": fb.get("n_in_mouth"), "n_held": fb.get("n_held"),
                            "mouth_contact_ticks": fb.get("mouth_contact_ticks"),
                            "plant_contact_ticks": fb.get("plant_contact_ticks"),
                            "mouth_contact_enter_events": fb.get("mouth_contact_enter_events"),
                            "first_mouth_contact_tick": fb.get("first_mouth_contact_tick"),
                            "first_plant_contact_tick": fb.get("first_plant_contact_tick"),
                            "bite_progress": fb.get("bite_progress"), "bite_period": fb.get("bite_period"),
                            "bibites_bitten": fb.get("bibites_bitten"), "damage_dealt": fb.get("damage_dealt")}
                    if isinstance(x, (int, float)) and isinstance(y, (int, float)):
                        try:
                            # nearest pellet from the full pellet map at this sample (distance, not a radius cut)
                            best = None
                            for p in pellet_map:
                                px, py = float(p[0]), float(p[1])
                                dx = px - float(x); dy = py - float(y)
                                d2 = dx * dx + dy * dy
                                if best is None or d2 < best[0]:
                                    best = (d2, px, py, p[2] if len(p) > 2 else None)
                            if best is not None:
                                item["nearest_pellet"] = {"x": best[1], "y": best[2], "material": best[3],
                                                          "distance": best[0] ** 0.5}
                        except Exception as exc:  # noqa: BLE001
                            item["nearest_error"] = str(exc)
                    rec["bibites"].append(item)
                rec["feeding_totals"] = feeding.get("totals")
                rec["feeding_counters"] = feeding.get("counters")
                rows.append(rec)
                last_tick = tick
            if st.get("state") not in ("running", "stopping"):
                break
            time.sleep(0.25)
        final = c.run_state(timeout=15.0)
        (RAW / "samples_fast.jsonl").write_text("\n".join(json.dumps(r) for r in rows), encoding="utf-8")
        (RAW / "final_state.json").write_text(json.dumps(final, indent=2), encoding="utf-8")
        # schema + compact summary (first rows and divergence-window rows around known ticks)
        keep_ticks = [160, 346, 381, 1000, 2000, 3000]
        windows = {str(t): next((r for r in rows if r["tick"] >= t), None) for t in keep_ticks}
        # occupancy/nearest-plant stats: circular single zone radius 2000 (radiusIsRelative 1.0, sim size 2000)
        zone_radius = 2000.0
        all_occ = []
        nearest_d = []
        for r in rows:
            for b in r.get("bibites") or []:
                x = b.get("x"); y = b.get("y")
                if isinstance(x, (int, float)) and isinstance(y, (int, float)):
                    rr = (x * x + y * y) ** 0.5
                    all_occ.append(rr)
                    np = b.get("nearest_pellet")
                    if np:
                        try:
                            nearest_d.append(float(np.get("distance")))
                        except Exception:
                            pass
        def pct(vals, q):
            if not vals: return None
            vals = sorted(vals); return vals[min(len(vals) - 1, max(0, int(q * (len(vals) - 1))))]
        occ = {"zone_radius": zone_radius, "samples": len(rows), "bibite_records": len(all_occ),
               "off_zone_records": sum(1 for r in all_occ if r > zone_radius),
               "off_zone_fraction": (sum(1 for r in all_occ if r > zone_radius) / len(all_occ)) if all_occ else None,
               "radius_p50": pct(all_occ, 0.5), "radius_p90": pct(all_occ, 0.9), "radius_max": max(all_occ) if all_occ else None,
               "nearest_plant_n": len(nearest_d), "nearest_plant_p50": pct(nearest_d, 0.5), "nearest_plant_p90": pct(nearest_d, 0.9)}
        summary = {"schema": "rustibites.assets.ecology_contact_ext.v1", "occupancy": occ,
                   "capture": {"save": "Default.zip newgame", "seed": 12345, "requested_speed": 8,
                               "until_simTime": 900, "samples": len(rows), "final": final,
                               "sample_fields": ["tick", "simTime", "state", "counts", "bibites[*].id/x/y/energy/health/"
                                                 "stomach_total/n_stomach/n_in_mouth/n_held/mouth_contact_ticks/"
                                                 "plant_contact_ticks/mouth_contact_enter_events/first_mouth_contact_tick/"
                                                 "first_plant_contact_tick/bite_progress/bite_period/bibites_bitten/"
                                                 "damage_dealt/nearest_pellets", "feeding_totals", "feeding_counters"]},
                   "raw": "testdata/parity/default/ecology_36k/contact_ext/samples_fast.jsonl",
                   "windows": windows,
                   "note": "Per-bibite positions/nearest plant/contact counters through 36k Default ticks for the "
                           "tick381 divergence triage; timestamps are game sim ticks."}
        (REF / "sample_schema.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
        (REF / "README.md").write_text(
            "# 36k Default per-bibite contact extension\n\n`sample_schema.json` documents the raw samples "
            "(testdata/parity/default/ecology_36k/contact_ext/samples.jsonl): every ~2 s wall, per live bibite "
            "id/x/y/energy/health, stomach/mouth state, mouth/plant contact tick counters and first-contact ticks, "
            "bite progress/period, plus nearest 3 plant pellets within 50 u. Use around ticks 160/346/381/1000 to "
            "separate trajectory/sensing divergence from mouth acquisition.\n", encoding="utf-8")
        print(json.dumps({"samples": len(rows), "final": final, "windows": windows}, indent=1)[:5000])
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""capture_36k_contact_early.py - 1x early-window per-bibite contact samples (ticks 0-600)."""
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
    B, _, Lo, La = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    s = cap.WorldSession(build_args(__import__("argparse").Namespace()), B, Lo, La)
    rows = []
    try:
        c = s.open()
        s.plugin.pacing(True)
        c.new_game(DEFAULT, name="parity_36k_contact_early", timeout=300)
        c.set_time(paused=True)
        c.seed(12345)
        c.run_start(until={"simTime": 20.0}, speed=1.0, sample=None, pause=True)
        last_tick = -1
        t0 = time.time()
        while time.time() - t0 < 120:
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
                            near = c.pellets_near(float(x), float(y), radius=50.0, limit=3)
                            item["nearest_pellets"] = near.get("pellets")
                            item["nearest_pellet_total"] = near.get("total")
                        except Exception as exc:  # noqa: BLE001
                            item["nearest_error"] = str(exc)
                    rec["bibites"].append(item)
                rec["feeding_totals"] = feeding.get("totals")
                rec["feeding_counters"] = feeding.get("counters")
                rows.append(rec)
                last_tick = tick
            if st.get("state") not in ("running", "stopping"):
                break
            time.sleep(0.12)
        (RAW / "samples_early.jsonl").write_text("\n".join(json.dumps(r) for r in rows), encoding="utf-8")
        first = next((r for r in rows if r["tick"] <= 170), None)
        print(json.dumps({"samples": len(rows), "first_tick": rows[0]["tick"] if rows else None,
                          "last_tick": rows[-1]["tick"] if rows else None,
                          "first_sample": first}, indent=1)[:5000])
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

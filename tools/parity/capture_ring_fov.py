#!/usr/bin/env python3
"""capture_ring_fov.py - focused post-task-128 FOV/seen-cache + digestion cadence capture.

Hidden Ring base (small stable population), seed12345, speed1 for 600 ticks (20 s sim). Polls /d/fov_digest
every 0.05 s: per-bibite seen-array counts/ids, needToSee/view radius/angle, plant/meat/bibite range counts
and stomach digestion fields. Saves first/empty-load state, change timeline, cadence intervals per id, and
digestion summary under assets/game/reference/fov/; raw under testdata. Hidden-only.
"""
from __future__ import annotations

import json
import sys
import time
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import capture as cap  # noqa: E402

RING = ROOT / "testdata/parity/bases/ring.base.zip"
RAW = ROOT / "testdata/parity/fov/ring_fov"
REF = ROOT / "assets/game/reference/fov"


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
    samples = []
    try:
        c = s.open()
        s.plugin.pacing(True)
        c.load_world(RING)
        c.set_time(paused=True)
        c.seed(12345)
        initial = s.plugin.httpx.get(s.plugin.base + "/d/fov_digest", timeout=60).json()
        c.run_start(until={"simTime": 20.0}, speed=1.0, sample=None, pause=True)
        t0 = time.time()
        last_tick = -1
        while time.time() - t0 < 120:
            st = c.run_state(timeout=15.0)
            tick = int((st.get("simTime") or 0) * 30)
            if tick != last_tick:
                f = s.plugin.httpx.get(s.plugin.base + "/d/fov_digest", timeout=60).json()
                rec = {"tick": tick, "simTime": st.get("simTime"), "state": st.get("state"),
                       "fov": f.get("fov"), "stomachs": f.get("stomachs")}
                samples.append(rec)
                last_tick = tick
            if st.get("state") not in ("running", "stopping"):
                break
            time.sleep(0.05)
        (RAW / "samples.jsonl").write_text("\n".join(json.dumps(x) for x in samples), encoding="utf-8")
        (RAW / "initial.json").write_text(json.dumps(initial, indent=2), encoding="utf-8")
        # cadence: per-id signatures (nPlants/nMeat/nBibite + seen array counts/ids first 4)
        timeline = {}
        for rec in samples:
            for e in rec.get("fov") or []:
                bid = str(e.get("id"))
                sig = (e.get("seenBibites_n"), e.get("seenCorpses_n"), e.get("seenPlantPellets_n"),
                       e.get("seenMeatPellets_n"), tuple((e.get("seenPlantPellets_ids") or [])[:4]),
                       tuple((e.get("seenBibites_ids") or [])[:4]))
                timeline.setdefault(bid, []).append({"tick": rec["tick"], "sig": sig})
        cadence = {"first_sample": samples[0] if samples else None,
                   "first_nonempty": {}, "change_ticks": {}, "sample_count": len(samples)}
        for bid, seq in timeline.items():
            changes = [seq[0]["tick"]] + [b["tick"] for a, b in zip(seq, seq[1:]) if b["sig"] != a["sig"]]
            cadence["change_ticks"][bid] = changes[:80]
            for rec in samples:
                for e in rec.get("fov") or []:
                    if str(e.get("id")) == bid and (e.get("seenPlantPellets_n") or 0) > 0:
                        cadence["first_nonempty"][bid] = rec["tick"]
                        break
                if bid in cadence["first_nonempty"]:
                    break
        # digestion summary from last sample and per-id first/last
        digest = {}
        for rec in samples:
            for e in rec.get("stomachs") or []:
                bid = str(e.get("id"))
                d = digest.setdefault(bid, {"first": e, "last": e, "contents_max": 0})
                d["last"] = e
                d["contents_max"] = max(d["contents_max"], e.get("contents_n") or 0)
        summary = {"schema": "rustibites.assets.ring_fov_capture.v1",
                   "capture": {"fixture": str(RING), "seed": 12345, "speed": 1, "until_simTime": 20,
                               "samples": len(samples), "poll_s": 0.05},
                   "initial_load": initial,
                   "cadence": cadence,
                   "digestion": digest,
                   "raw": "testdata/parity/fov/ring_fov/samples.jsonl",
                   "note": "Post-task-128 FOV lookup/sense cadence and seen-cache: arrays/ids per bibite over 600 "
                           "ticks; first/empty load state included. Digestion fields are native per-tick stomach "
                           "amounts/rates (no cumulative game digestion counter exists in the component)."}
        (REF / "ring_fov.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
        (REF / "README.md").write_text(
            "# Ring focused FOV/seen-cache capture (post task-128)\n\nHidden Ring base, seed12345, 600 ticks at 1x, "
            "polled 0.05 s for per-bibite FOV seen-array counts/ids, needToSee/radius/angle, plant/meat/bibite "
            "range counts and stomach digestion fields. `ring_fov.json` has the initial/empty state, per-id change "
            "timeline/first nonempty tick and digestion first/last; raw under testdata/parity/fov/ring_fov. Native "
            "game stomach components expose per-tick amounts/rates, not cumulative digestion counters.\n", encoding="utf-8")
        print(json.dumps({"samples": len(samples), "cadence": cadence, "digestion": digest}, indent=1)[:5000])
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

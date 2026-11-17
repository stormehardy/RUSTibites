#!/usr/bin/env python3
"""capture_digestion_acid_brain.py - game acidLevel alongside brain Digestion output.

Hidden Default save seed12345, 600 ticks at 1x; polls /d/digestion_brain (per-bibite acidLevel, brain
Digestion output, nContent/totalContent/digestionEffectiveness/acidRatio) every 0.05 s. Saves paired samples
so the body formula check can use the game's own acid level for the same brain output.
"""
from __future__ import annotations

import json
import sys
import time
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import capture as cap  # noqa: E402

DEFAULT = ROOT / "testdata/parity/default/rust_seed_600.zip"
RAW = ROOT / "testdata/parity/digestion/acid_brain"
REF = ROOT / "assets/game/reference/digestion"


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
        c.load_world(DEFAULT)
        c.set_time(paused=True)
        c.seed(12345)
        c.run_start(until={"simTime": 20.0}, speed=1.0, sample=None, pause=True)
        t0 = time.time()
        last_tick = -1
        while time.time() - t0 < 120:
            st = c.run_state(timeout=15.0)
            tick = int((st.get("simTime") or 0) * 30)
            if tick != last_tick:
                f = s.plugin.httpx.get(s.plugin.base + "/d/digestion_brain", timeout=60).json()
                samples.append({"tick": tick, "simTime": st.get("simTime"), "bibites": f.get("bibites")})
                last_tick = tick
            if st.get("state") not in ("running", "stopping"):
                break
            time.sleep(0.05)
        (RAW / "samples.jsonl").write_text("\n".join(json.dumps(x) for x in samples), encoding="utf-8")
        last = samples[-1] if samples else {}
        summary = {"schema": "rustibites.assets.digestion_acid_brain.v1",
                   "capture": {"save": str(DEFAULT), "seed": 12345, "ticks": 600, "samples": len(samples)},
                   "last": last, "raw": "testdata/parity/digestion/acid_brain/samples.jsonl",
                   "note": "Game acidLevel paired with the brain Digestion output for each bibite at each sample; "
                           "use contents.amount/digestionRate for stock, acidLevel to explain conversion-rate delta."}
        (REF / "acid_brain_trace.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
        print(json.dumps(summary, indent=1)[:6000])
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""capture_digestion.py - native game stomach/digestion fields over a focused feeding run.

Hidden Default save rust_seed_600 paused, seed12345, run 600 ticks at speed8; polls /d/fov_digest every 0.2 s
for each bibite's native stomach fields (nContent/totalContent/acidLevel/acidRatio/digestionEffectiveness/
digestionPotentialBonus/efficiencyMalus/EnergyGainRate and per-content material/amount/digestionRate/
lastConversionEfficiency/chunk). There is no cumulative game digestion counter in BibiteStomach, so raw
per-tick fields are captured; summary documents availability. Hidden-only.
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
RAW = ROOT / "testdata/parity/digestion"
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
        while time.time() - t0 < 180:
            st = c.run_state(timeout=15.0)
            tick = int((st.get("simTime") or 0) * 30)
            if tick != last_tick:
                f = s.plugin.httpx.get(s.plugin.base + "/d/fov_digest", timeout=60).json()
                samples.append({"tick": tick, "simTime": st.get("simTime"), "state": st.get("state"),
                                "stomachs": f.get("stomachs")})
                last_tick = tick
            if st.get("state") not in ("running", "stopping"):
                break
            time.sleep(0.2)
        (RAW / "samples.jsonl").write_text("\n".join(json.dumps(x) for x in samples), encoding="utf-8")
        # per-id summary: first/last, max contents, max energy gain rate, materials seen, nonzero samples
        per = {}
        for rec in samples:
            for e in rec.get("stomachs") or []:
                bid = str(e.get("id"))
                d = per.setdefault(bid, {"first": e, "last": e, "samples": 0, "nonzero_content": 0,
                                         "materials": [], "max_totalContent": 0.0,
                                         "max_energyGainRate": None})
                d["last"] = e; d["samples"] += 1
                if (e.get("nContent") or 0) > 0 or (e.get("totalContent") or 0) > 0:
                    d["nonzero_content"] += 1
                d["max_totalContent"] = max(d["max_totalContent"], float(e.get("totalContent") or 0))
                r = e.get("energyGainRate")
                if r is not None:
                    d["max_energyGainRate"] = max(d["max_energyGainRate"] or r, r)
                for m in e.get("contents") or []:
                    if m.get("material") not in d["materials"]:
                        d["materials"].append(m.get("material"))
        summary = {"schema": "rustibites.assets.digestion_native.v1",
                   "capture": {"save": str(DEFAULT), "seed": 12345, "ticks": 600, "samples": len(samples),
                               "poll_s": 0.2, "plugin": "0.1.0"},
                   "fields": ["id", "nContent", "totalContent", "acidLevel", "acidRatio",
                              "digestionEffectiveness", "digestionPotentialBonus", "efficiencyMalus",
                              "EnergyGainRate", "contents[material,amount,digestionRate,lastConversionEfficiency,"
                              "averageChunkAmount]"],
                   "per_bibite": per,
                   "raw": "testdata/parity/digestion/samples.jsonl",
                   "availability": "BibiteStomach exposes per-tick stomach amount/rate fields but no native cumulative "
                                   "digested/gained-energy counter; raw per-tick fields are the available evidence. "
                                   "Rust digestion can be compared against these rates/contents, not a cumulative counter."}
        (REF / "native_digestion.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
        (REF / "README.md").write_text(
            "# Game native digestion fields\n\nHidden Default save seed12345, 600 ticks, polled 0.2 s. "
            "`native_digestion.json` + raw `testdata/parity/digestion/samples.jsonl`: per-bibite nContent/totalContent/"
            "acid/digestion effectiveness/EnergyGainRate and per-content material/amount/digestionRate/conversion "
            "efficiency. No native cumulative digestion counter exists; document availability accordingly.\n", encoding="utf-8")
        print(json.dumps({"samples": len(samples), "per_bibite": per}, indent=1)[:6000])
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

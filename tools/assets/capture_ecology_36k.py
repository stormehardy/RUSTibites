#!/usr/bin/env python3
"""capture_ecology_36k.py - 36,000-tick (20 min sim) Default ecology capture for brain-dev.

Hidden Default new-game, seeded 12345, paused; then run at speed 8 until simTime=1200 s (36,000 ticks).
Bridge sampler every 2 sim s (60 ticks) records counts/pellets/vitals/generation/species/lifecycle + birth/
death events; the parity plugin /d/feeding is polled while the run is live for intake/stock evidence. Raw
rows/events/feeding go under testdata/parity/default/ecology_36k/ (gitignored); a small summary is tracked
under assets/game/reference/ecology/default_36k_summary.json. Hidden-only; caller removes plugin.
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
RAW = ROOT / "testdata/parity/default/ecology_36k"
REF = ROOT / "assets/game/reference/ecology"


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
    feeding = []
    try:
        c = s.open()
        s.plugin.pacing(True)
        c.new_game(DEFAULT, name="parity_ecology_36k", timeout=300)
        c.set_time(paused=True)
        c.seed(12345)
        before = c.status()
        run = c.run_start(until={"simTime": 1200.0}, speed=8.0,
                          sample={"every": 2.0,
                                  "fields": ["counts", "pellets", "vitals", "generation", "species", "lifecycle"],
                                  "events": True},
                          pause=True)
        last_tick = -1
        deadline = time.time() + 1200
        while time.time() < deadline:
            st = c.run_state(timeout=15.0)
            tick = int(st.get("simTime", 0) * 30) if st.get("simTime") is not None else -1
            if tick != last_tick:
                try:
                    f = s.plugin.feeding()
                    f["_tick"] = tick
                    feeding.append(f)
                except Exception as exc:  # noqa: BLE001
                    feeding.append({"error": str(exc), "_tick": tick})
                last_tick = tick
            if st.get("state") not in ("running", "stopping"):
                break
            time.sleep(1.0)
        final = c.run_state(timeout=15.0)
        series = c.series(limit=10000)
        events = c.events(limit=50000)
        rows = series.get("rows") or []
        evs = events.get("events") or []
        (RAW / "series.json").write_text(json.dumps(series, indent=1), encoding="utf-8")
        (RAW / "events.json").write_text(json.dumps(events, indent=1), encoding="utf-8")
        (RAW / "feeding.json").write_text(json.dumps(feeding, indent=1), encoding="utf-8")
        (RAW / "final_state.json").write_text(json.dumps(final, indent=1), encoding="utf-8")
        births = [e for e in evs if e.get("kind") == "birth"]
        deaths = [e for e in evs if e.get("kind") == "death"]
        causes = {}
        for e in deaths:
            causes[e.get("cause") or "other"] = causes.get(e.get("cause") or "other", 0) + 1
        # compact summary: first/last sampled counts, min/max, final
        counts = rows[-1].get("counts") if rows else {}
        counts_minmax = {}
        for r in rows:
            for k, v in (r.get("counts") or {}).items():
                if isinstance(v, (int, float)):
                    counts_minmax.setdefault(k, [v, v])
                    counts_minmax[k][0] = min(counts_minmax[k][0], v)
                    counts_minmax[k][1] = max(counts_minmax[k][1], v)
        summary = {
            "schema": "rustibites.assets.ecology_36k.v1",
            "capture": {"save": "Default.zip newgame", "seed": 12345, "speed": 8,
                        "ticks": int(final.get("simTime", 0) * 30) if final.get("simTime") is not None else None,
                        "sim_time_s": final.get("simTime"), "sample_every_s": 2.0,
                        "sample_fields": ["counts", "pellets", "vitals", "generation", "species", "lifecycle"],
                        "events": True, "feeding_polls": len(feeding),
                        "before": before, "final": final},
            "series": {"rows": len(rows), "next": series.get("next"), "dropped": series.get("dropped"),
                       "first": rows[0] if rows else None, "last": rows[-1] if rows else None,
                       "counts_minmax": counts_minmax,
                       "sim_time_first_last": [rows[0].get("simTime"), rows[-1].get("simTime")] if rows else None},
            "events": {"births": len(births), "deaths": len(deaths), "death_causes": causes,
                       "initial": sum(1 for e in evs if e.get("kind") == "initial"),
                       "next": events.get("next"), "dropped": events.get("dropped")},
            "feeding": {"polls": len(feeding), "fields": sorted((feeding[0] or {}).keys()) if feeding else []},
            "files": {"raw": "testdata/parity/default/ecology_36k/",
                      "summary": "assets/game/reference/ecology/default_36k_summary.json"},
            "note": "Game-side 20-min Default run for Rust long-run envelope comparison; feeding is plugin /d/feeding "
                    "polled while live (exact tick recorded per poll). Hidden-only.",
        }
        (REF / "default_36k_summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
        (REF / "README.md").write_text(
            "# Default 36,000-tick ecology capture\n\nHidden Default newgame seed 12345, speed 8, 1200 sim s "
            "(36k ticks); bridge sampler every 2 s/60 ticks plus birth/death events and live `/d/feeding` polls. "
            "Raw rows/events/feeding under `testdata/parity/default/ecology_36k/`; summary in "
            "`default_36k_summary.json`. Use for Rust long-run envelope/speciation comparison.\n", encoding="utf-8")
        print(json.dumps(summary, indent=1)[:6000])
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

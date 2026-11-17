import json
from pathlib import Path

root = Path(r"Z:\RUSTibites")
raw = root / "testdata/parity/default/ecology_36k"
ref = root / "assets/game/reference/ecology"
ref.mkdir(parents=True, exist_ok=True)
series = json.loads((raw / "series.json").read_text(encoding="utf-8"))
events = json.loads((raw / "events.json").read_text(encoding="utf-8"))
feeding = json.loads((raw / "feeding.json").read_text(encoding="utf-8"))
final = json.loads((raw / "final_state.json").read_text(encoding="utf-8"))
rows_all = series.get("rows") or []
rows = [r for r in rows_all if int(r.get("tick") or 0) <= 36000]
evs_all = events.get("events") or []
evs = [e for e in evs_all if int(e.get("tick") or 0) <= 36000]
feed = [f for f in feeding if int(f.get("_tick") or 0) <= 36000]
births = [e for e in evs if e.get("kind") == "birth"]
deaths = [e for e in evs if e.get("kind") == "death"]
causes = {}
for e in deaths:
    causes[e.get("cause") or "other"] = causes.get(e.get("cause") or "other", 0) + 1
fields = ["bibites", "eggs", "corpses", "plantN", "plantBiomass", "meatN", "meatBiomass",
          "freeBiomass", "bibiteBiomass", "energyMean", "healthMean", "ageMean", "ageMax",
          "genMax", "genMean", "nSpecies"]
minmax = {}
for k in fields:
    vals = [r[k] for r in rows if isinstance(r.get(k), (int, float))]
    if vals:
        minmax[k] = [min(vals), max(vals)]
summary = {
    "schema": "rustibites.assets.ecology_36k.v1",
    "capture": {"save": "Default.zip newgame", "seed": 12345, "requested_speed": 8,
                "stop": "run until simTime 1200 s; rows/events/feeding filtered to tick<=36000",
                "actual_final": final, "rows_total": len(rows_all), "rows_used": len(rows),
                "events_total": len(evs_all), "events_used": len(evs), "feeding_polls_total": len(feeding),
                "feeding_polls_used": len(feed),
                "tick_first_last": [rows[0].get("tick"), rows[-1].get("tick")] if rows else None,
                "simtime_first_last": [rows[0].get("t"), rows[-1].get("t")] if rows else None,
                "note": "The game achieved effective ~3.46x/40 ticks-per-s and overshot to 47,993 ticks; the tracked "
                        "36,000-tick subset is the first rows/events up to tick 36,000 (raw keeps the full run)."},
    "rows_first": rows[0] if rows else None,
    "rows_last": rows[-1] if rows else None,
    "counts_minmax": minmax,
    "events": {"births": len(births), "deaths": len(deaths), "death_causes": causes,
               "initial": sum(1 for e in evs if e.get("kind") == "initial")},
    "feeding": {"polls_used": len(feed), "fields": sorted((feed[0] or {}).keys()) if feed else []},
    "raw": {"series": "testdata/parity/default/ecology_36k/series.json",
            "events": "testdata/parity/default/ecology_36k/events.json",
            "feeding": "testdata/parity/default/ecology_36k/feeding.json",
            "final_state": "testdata/parity/default/ecology_36k/final_state.json"},
}
(ref / "default_36k_summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
(ref / "README.md").write_text("""# Default 36,000-tick ecology capture (game)

Hidden Default newgame seed 12345, requested speed 8. The bridge run stopped on simTime=1200 s and
overshot to 47,993 ticks (effective ~40 ticks/s, achieved ~3.46x); summary/analysis uses the first
36,000 ticks (601 sampler rows, birth/death events, /d/feeding polls at exact ticks). Raw full-run
rows/events/feeding are under `testdata/parity/default/ecology_36k/` (gitignored); tracked summary:
`default_36k_summary.json`. Use for Rust long-run population/energy/plant/egg/death/feeding envelope
and speciation comparison.
""", encoding="utf-8")
print(json.dumps({k: summary[k] for k in ("capture", "events", "feeding")}, indent=1)[:3000])
print("minmax", summary["counts_minmax"])

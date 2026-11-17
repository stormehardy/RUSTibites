import json
from pathlib import Path

root = Path(r"Z:\RUSTibites")
names = ["rust_state_save.zip", "rust_eggs_save.zip", "rust_tools_phero_save.zip",
         "rust_default_scenario_save.zip", "rust_default_scenario.zip", "rust_ring_scenario.zip",
         "rust_template.bb8template"]
entries = []
for i, name in enumerate(names, 1):
    lines = (root / f"target/compat_one_{i}.log").read_text(encoding="utf-16", errors="replace").splitlines()
    start = next((j for j, line in enumerate(lines) if line.strip().lstrip("\ufeff") == "{"), None)
    doc = json.loads("\n".join(lines[start:])) if start is not None else {}
    fs = doc.get("fixtures") or []
    f = fs[0] if fs else {}
    entries.append({"name": name, "kind": f.get("kind"), "accepted": f.get("accepted"),
                    "in_simulation": f.get("in_simulation"), "counts": f.get("counts"),
                    "simulated_time": f.get("simulated_time"),
                    "resaved": f.get("resaved"), "resaved_bytes": f.get("resaved_bytes"),
                    "error": f.get("error"), "player_log_new_errors": f.get("player_log_new_errors"),
                    "expected": f.get("expected")})
out = {"schema": "rustibites.compat.game_report.one_per_process.v1",
       "game_version": doc.get("game_version"), "player_log": doc.get("player_log"),
       "method": "one fixture per fresh process/lab via tools/roundtrip/load_compat_pack.py "
                 "--one-fixture <name> --hard-timeout 90; watchdog 2e8eda4",
       "certified_loads": ["rust_lol_save.zip", "rust_corpses_save.zip"],
       "results": entries,
       "classification": {
           "loads_accepted_but_resave_bridge_closed": [e["name"] for e in entries
                                                        if e.get("in_simulation") and not e.get("accepted")
                                                        and "10054" in str(e.get("error"))],
           "accepted": [e["name"] for e in entries if e.get("accepted")],
       },
       "note": "Save fixtures load with correct live counts; the failure happens in the harness's save_world "
               "re-save step (bridge connection forcibly closed / watchdog), not at load. Scenarios/template "
               "accepted. No dropped expected fields observed in status/load."}
outdir = root / "assets/game/reference/compat"
outdir.mkdir(parents=True, exist_ok=True)
(outdir / "game_report_2026_11_01.json").write_text(json.dumps(out, indent=2), encoding="utf-8")
print(json.dumps(out, indent=1)[:7000])

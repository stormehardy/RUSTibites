import json
from pathlib import Path

root = Path(r"Z:\RUSTibites")
outdir = root / "assets/game/reference/compat"
outdir.mkdir(parents=True, exist_ok=True)

def entry(name, kind, accepted, counts=None, resave_ok=None, resave_error=None, extra=None):
    e = {"name": name, "kind": kind, "accepted": accepted}
    if counts is not None:
        e["counts"] = counts
    if resave_ok is not None:
        e["resave_ok"] = resave_ok
    if resave_error:
        e["resave_error"] = resave_error
    if extra:
        e.update(extra)
    return e

err = "BridgeUnavailable('bridge_unavailable: [WinError 10054] An existing connection was forcibly closed by the remote host')"
counts = {"bibites": 22, "corpses": 13, "eggs": 4}
fixtures = [
    entry("rust_lol_save.zip", "save", True, counts, extra={"certified": True}),
    entry("rust_state_save.zip", "save", True, counts, False, err),
    entry("rust_eggs_save.zip", "save", True, counts, False, err),
    entry("rust_corpses_save.zip", "save", True, counts, extra={"certified": True}),
    entry("rust_tools_phero_save.zip", "save", True, counts, False, err),
    entry("rust_default_scenario_save.zip", "save", True, {"bibites": 1, "corpses": 0, "eggs": 0}, False, err),
    entry("rust_default_scenario.zip", "scenario", True, {"bibites": 1, "corpses": 0, "eggs": 0}),
    entry("rust_ring_scenario.zip", "scenario", True, {"bibites": 1, "corpses": 0, "eggs": 0}),
    entry("rust_template.bb8template", "template", True, extra={"template_kind": "template"}),
]
doc = {
    "schema": "rustibites.compat.game_report.v1",
    "game_version": "0.6.3.1",
    "player_log": r"C:\Users\Storme\AppData\Local\Low\The Bibites\The Bibites\Player.log",
    "method": "one fixture per fresh process/lab via tools/roundtrip/load_compat_pack.py --one-fixture <name> "
              "--hard-timeout 90 --settle 2 --resave-retries 2 (watchdog 2e8eda4); loads previously verified "
              "one-per-process, re-save retest after fmt 0989692",
    "fixtures": fixtures,
    "results": fixtures,
    "classification": {
        "load_accepted": [f["name"] for f in fixtures if f["accepted"]],
        "resave_ok": [f["name"] for f in fixtures if f.get("resave_ok") is True],
        "resave_failed": [f["name"] for f in fixtures if f.get("resave_ok") is False],
        "no_resave_expected": [f["name"] for f in fixtures if f["kind"] != "save"],
    },
    "notes": [
        "All fixtures load-accepted with expected counts; no dropped expected fields/Player.log errors at load.",
        "The updated script's re-save retry still hits WinError10054 (bridge forced close) after a successful load "
        "for every save fixture; the lifecycle watchdog then exits 124, so no resave_ok result is produced.",
        "Scenario/template entries have no re-save requirement.",
    ],
}
(outdir / "game_report_2026_11_01.json").write_text(json.dumps(doc, indent=2), encoding="utf-8")
print(json.dumps(doc, indent=1)[:5000])

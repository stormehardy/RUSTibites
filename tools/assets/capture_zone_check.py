#!/usr/bin/env python3
"""capture_zone_check.py - canonical-save zone config vs live game + reference-camera frame.

Hidden load of rust_seed_600 paused; prints the serialized zone block from settings.bb8settings alongside
the live game's zones() and /d/zone_material, and captures the closeup-camera frame for the zone/background
mismatch check. Hidden-only; refs under assets/game/reference/zone_check/.
"""
from __future__ import annotations

import json
import sys
import zipfile
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import capture as cap  # noqa: E402

SAVE = ROOT / "testdata/parity/default/rust_seed_600.zip"
OUT = ROOT / "assets/game/reference/zone_check"


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
    OUT.mkdir(parents=True, exist_ok=True)
    z = zipfile.ZipFile(SAVE)
    settings = json.loads(z.read("settings.bb8settings").decode("utf-8-sig"))
    parsed = {"zones": settings.get("zones"), "zoneGroups": settings.get("zoneGroups"),
              "globalZoneSpeed": settings.get("globalZoneSpeed"),
              "showFertility": settings.get("showFertility"),
              "backgroundKeys": [k for k in settings if "background" in k.lower() or "fertility" in k.lower() or "color" in k.lower()]}
    B, _, Lo, La = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    s = cap.WorldSession(build_args(__import__("argparse").Namespace()), B, Lo, La)
    try:
        c = s.open()
        s.plugin.pacing(True)
        c.load_world(SAVE)
        c.set_time(paused=True)
        live = c.zones()
        zmat = s.plugin.httpx.get(s.plugin.base + "/d/zone_material", timeout=120).json()
        selected = (live.get("zones") or [{}])[0] if isinstance(live, dict) else {}
        cam = c.set_camera(x=-5.360003, y=831.0031, size=12.0)
        png = c.screenshot(1280, 720)
        (OUT / "reference_camera_1280x720.png").write_bytes(png)
        doc = {"schema": "rustibites.assets.zone_check.v1",
               "save": str(SAVE), "parsed_settings": parsed, "live_zones": live,
               "live_zone_material": zmat, "camera": cam,
               "reference_png": "assets/game/reference/zone_check/reference_camera_1280x720.png",
               "note": "Compare parsed zone block against live zones/zone material; the closeup frame is the "
                       "same camera/ortho as the user's reference for the left-panel zone/background colour."}
        (OUT / "zone_check.json").write_text(json.dumps(doc, indent=2), encoding="utf-8")
        print(json.dumps({"parsed_zones": parsed["zones"], "live_zones": live.get("zones") if isinstance(live, dict) else live,
                          "zone_material": zmat, "camera": cam}, indent=1)[:6000])
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

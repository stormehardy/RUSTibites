#!/usr/bin/env python3
"""capture_camera_arms_refs.py - live CameraManager fields + zoomed arms reference (user visual fixes).

One hidden load of rust_seed_600 paused: GET /d/camera_controls (CameraManager scrollSpeed/speedScale/zoom
limits), select the bibite nearest the closeup camera, capture a 1280x720 ortho-size-5 closeup for both arms,
and record camera/entity metadata. Refs go under assets/game/reference/{camera,arms}/. Hidden-only.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import capture as cap  # noqa: E402

SAVE = ROOT / "testdata/parity/default/rust_seed_600.zip"
CAM = {"x": -5.360003, "y": 831.0031}


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
    cam_dir = ROOT / "assets/game/reference/camera"
    arm_dir = ROOT / "assets/game/reference/arms"
    cam_dir.mkdir(parents=True, exist_ok=True)
    arm_dir.mkdir(parents=True, exist_ok=True)
    B, _, Lo, La = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    s = cap.WorldSession(build_args(__import__("argparse").Namespace()), B, Lo, La)
    try:
        c = s.open()
        s.plugin.pacing(True)
        c.load_world(SAVE)
        c.set_time(paused=True)
        controls = s.plugin.httpx.get(s.plugin.base + "/d/camera_controls", timeout=60).json()
        (cam_dir / "live_camera_controls.json").write_text(json.dumps(controls, indent=2), encoding="utf-8")
        layers = s.plugin.httpx.get(s.plugin.base + "/d/bibite_layers", timeout=120).json()
        bibites = layers.get("bibites") or []
        selected = min(bibites, key=lambda b: (float((b.get("body_position") or {}).get("x") or 0) - CAM["x"]) ** 2
                       + (float((b.get("body_position") or {}).get("y") or 0) - CAM["y"]) ** 2)
        sid = selected.get("id")
        pos = selected.get("body_position") or {}
        zoom = 5.0
        camera = c.set_camera(x=float(pos.get("x") or CAM["x"]), y=float(pos.get("y") or CAM["y"]), size=zoom)
        png = c.screenshot(1280, 720)
        (arm_dir / "arms_zoom_1280x720.png").write_bytes(png)
        meta = {"schema": "rustibites.assets.arms_zoom.v1",
                "capture": {"method": "hidden load rust_seed_600 paused t=20s; nearest closeup bibite; "
                                      "exact 1280x720 camera, ortho size 5 (6 px/world)",
                            "save": str(SAVE), "selected_id": sid, "selected_body_position": pos,
                            "camera": camera, "screen": [1280, 720], "pixels_per_world_unit": 72.0},
                "image": "assets/game/reference/arms/arms_zoom_1280x720.png",
                "per_layer_refs": ["assets/game/reference/default_save/closeup_single/layers/RightArm_4_alpha.png",
                                   "assets/game/reference/default_save/closeup_single/layers/Left_Arm_5_alpha.png"],
                "note": "Both arms visible at 6 px/world for the user-reported one-side arm issue; the six-layer "
                        "closeup/layer alpha refs above are the same body's per-layer images."}
        (arm_dir / "reference.json").write_text(json.dumps(meta, indent=2), encoding="utf-8")
        (cam_dir / "README.md").write_text(
            "Live CameraManager serialized values for matching Unity CameraZoom(ratio) wheel/gradual zoom. "
            "`live_camera_controls.json` includes scrollSpeed, speedScale, scroll drag height, camSize and the "
            "main camera state. See Here/ManagementScripts/CameraManager.cs for the gradual-zoom input path.\n", encoding="utf-8")
        print(json.dumps({"camera_fields": controls.get("fields"), "selected_id": sid, "camera": camera,
                          "png_bytes": len(png)}, indent=1)[:4000])
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

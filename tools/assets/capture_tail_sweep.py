#!/usr/bin/env python3
"""capture_tail_sweep.py - live Body material dump + Tail-angle shader sweep (task-145).

One hidden load of rust_seed_600.zip paused t=20 s: select the bibite nearest the canonical closeup camera,
dump the live Body material/shader properties (/d/material_dump), set the Body renderer alone, and sweep
`_TailAngle` (0, +/-5..25 deg) x `_MiddleSegmentAngle` (0, +/-3.75, +/-7.5 deg) via /d/material_float.
Each frame is captured at the exact 1280x720 closeup camera; visible-alpha bbox is the diff against an
all-renderers-off frame. Full frames stay under testdata; tracked refs (alpha masks/bboxes/material JSON)
go under assets/game/reference/default_save/tail_sweep/ and assets/game/tail/. Restores material/renderers.
"""
from __future__ import annotations

import argparse
import json
import math
import re
import sys
from pathlib import Path

from PIL import Image, ImageChops

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import capture as cap  # noqa: E402

SAVE = ROOT / "testdata/parity/default/rust_seed_600.zip"
RAW = ROOT / "testdata/parity/default/tail_sweep"
REF = ROOT / "assets/game/reference/default_save/tail_sweep"
TAIL_ASSETS = ROOT / "assets/game/tail"
CAM = {"x": -5.360003, "y": 831.0031, "ortho_size": 12.0, "width": 1280, "height": 720}
TAIL_DEG = [-25, -20, -15, -10, -5, 0, 5, 10, 15, 20, 25]
MID_DEG = [-7.5, -3.75, 0, 3.75, 7.5]


def build_args(ns):
    ns.work_dir = str(ROOT / "testdata/parity/_work")
    ns.port = 17800
    ns.plugin_port = 17801
    ns.no_plugin = False
    ns.attach = False
    ns.start_timeout = 300.0
    ns.keep_game = False
    return ns


def diff_mask(a_path, b_path, threshold=8):
    a = Image.open(a_path).convert("RGBA")
    b = Image.open(b_path).convert("RGBA")
    diff = ImageChops.difference(a, b).convert("L")
    mask = diff.point(lambda v: 255 if v > threshold else 0)
    return mask, mask.getbbox(), sum(1 for v in mask.getdata() if v)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--save", default=str(SAVE))
    ap.add_argument("--api-home", default=r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
    args = ap.parse_args()
    RAW.mkdir(parents=True, exist_ok=True)
    (RAW / "frames").mkdir(parents=True, exist_ok=True)
    REF.mkdir(parents=True, exist_ok=True)
    TAIL_ASSETS.mkdir(parents=True, exist_ok=True)
    BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(args.api_home))
    session = cap.WorldSession(build_args(argparse.Namespace()), BridgeClient, LiveRuntime, Lab)
    try:
        client = session.open()
        if not session.plugin_ok:
            raise SystemExit("parity plugin not available (deploy ParityDump)")
        session.plugin.pacing(True)
        client.load_world(Path(args.save).absolute())
        client.set_time(paused=True)
        layers = session.plugin.httpx.get(session.plugin.base + "/d/bibite_layers", timeout=120).json()
        bibites = layers.get("bibites") or []
        selected = min(bibites, key=lambda b: (float((b.get("body_position") or {}).get("x") or 0) - CAM["x"]) ** 2
                       + (float((b.get("body_position") or {}).get("y") or 0) - CAM["y"]) ** 2)
        sid = selected.get("id")
        pos = selected.get("body_position") or {}
        cam = client.set_camera(x=float(pos.get("x") or CAM["x"]), y=float(pos.get("y") or CAM["y"]),
                                size=CAM["ortho_size"])

        def post(path, body):
            return session.plugin.httpx.post(session.plugin.base + path, json=body, timeout=180).json()

        mat_dump = post("/d/material_dump", {"id": sid, "layer": "Body"})
        (TAIL_ASSETS / "procedural_bibite_material.json").write_text(json.dumps(mat_dump, indent=2), encoding="utf-8")
        props = {p.get("name"): p for p in (mat_dump.get("properties") or [])}
        orig_tail = float((props.get("_TailAngle") or {}).get("float") or 0.0)
        orig_mid = float((props.get("_MiddleSegmentAngle") or {}).get("float") or 0.0)
        # stop every bibite body script from overwriting the shader floats every frame
        ctrl_off = post("/d/body_scripts", {"id": sid, "enabled": False})
        off = post("/d/renderers", {"action": "off", "id": sid})
        (RAW / "all_off.png").write_bytes(client.screenshot(CAM["width"], CAM["height"]))
        body_only = post("/d/renderers", {"action": "only", "id": sid, "path": "Body"})
        frames = []
        for tail in TAIL_DEG:
            for mid in MID_DEG:
                tr = math.radians(tail)
                mr = math.radians(mid)
                post("/d/material_float", {"id": sid, "layer": "Body", "name": "_TailAngle", "value": tr})
                post("/d/material_float", {"id": sid, "layer": "Body", "name": "_MiddleSegmentAngle", "value": mr})
                raw_name = f"tail_{tail:+03d}_mid_{mid:+05.2f}.png".replace("+00", "000").replace(" ", "")
                raw_path = RAW / "frames" / raw_name
                raw_path.write_bytes(client.screenshot(CAM["width"], CAM["height"]))
                mask, bbox, npx = diff_mask(RAW / "all_off.png", raw_path)
                mask_name = raw_name.replace(".png", "_alpha.png")
                mask_path = REF / mask_name
                mask.save(mask_path)
                frames.append({"tail_deg": tail, "mid_deg": mid, "tail_rad": tr, "mid_rad": mr,
                               "raw_frame": str(raw_path.relative_to(ROOT).as_posix()),
                               "alpha_mask": str(mask_path.relative_to(ROOT).as_posix()),
                               "visible_alpha_bbox_pixels": bbox, "visible_alpha_pixels": npx})
        # restore original floats and all renderers; verify base-equivalent
        post("/d/material_float", {"id": sid, "layer": "Body", "name": "_TailAngle", "value": orig_tail})
        post("/d/material_float", {"id": sid, "layer": "Body", "name": "_MiddleSegmentAngle", "value": orig_mid})
        ctrl_on = post("/d/body_scripts", {"id": sid, "enabled": True})
        restore = post("/d/renderers", {"action": "restore", "id": sid})
        verify = client.screenshot(CAM["width"], CAM["height"])
        (RAW / "restored.png").write_bytes(verify)
        ppu = CAM["height"] / (2.0 * CAM["ortho_size"])
        doc = {
            "schema": "rustibites.assets.tail_sweep.v1",
            "capture": {"method": "single hidden load paused t=20s; Body renderer only; shader floats set via "
                                  "/d/material_float; exact closeup camera; visible alpha = diff vs all-off",
                        "save": str(Path(args.save).absolute()), "selected_id": sid,
                        "selected_body_position": pos, "camera": cam, "screen": [CAM["width"], CAM["height"]],
                        "pixels_per_world_unit": ppu,
                        "pixel_formula": "x_px=640+(x-cam.x)*30; y_px_up=360+(y-cam.y)*30; image_y=720-y_px_up",
                        "material": mat_dump.get("material"), "shader": mat_dump.get("shader"),
                        "all_off": str((RAW / "all_off.png").relative_to(ROOT).as_posix()),
                        "renderer_off": off, "renderer_body_only": body_only, "renderer_restore": restore,
                        "controller_off": ctrl_off, "controller_on": ctrl_on,
                        "original_floats": {"_TailAngle": orig_tail, "_MiddleSegmentAngle": orig_mid}},
            "properties": mat_dump.get("properties"),
            "sweep": {"tail_deg": TAIL_DEG, "middle_deg": MID_DEG, "frames": frames},
            "note": "values in radians are what ProceduralSpriteController writes to the shader; frame alpha masks/bboxes "
                    "show the Body sprite warp only. Tail transform node is not a mesh.",
        }
        (REF / "sweep.json").write_text(json.dumps(doc, indent=2), encoding="utf-8")
        (TAIL_ASSETS / "README.md").write_text(
            "Tail deformation is a shader warp of the Body SpriteRenderer (no tail mesh). "
            "`procedural_bibite_material.json` is the live material/shader property dump; "
            "`assets/game/reference/default_save/tail_sweep/sweep.json` + alpha masks are the "
            "`_TailAngle`/`_MiddleSegmentAngle` sweep for fitting the warp. Raw frames: "
            "testdata/parity/default/tail_sweep/frames/ (gitignored).\n", encoding="utf-8")
        print(f"[tail-sweep] selected={sid} shader={mat_dump.get('shader')} material={mat_dump.get('material')} "
              f"props={len(mat_dump.get('properties') or [])} frames={len(frames)}")
    finally:
        session.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

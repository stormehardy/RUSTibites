#!/usr/bin/env python3
"""validate_ui_camera.py - check the joined runtime-UI atlas has everything needed to compose
ScreenSpaceCamera canvases to screen space (task-112 follow-up for bench-dev/view-dev).

The composer needs, per ScreenSpaceCamera canvas: the Canvas `render_mode`, `camera` name,
`plane_distance`, and the camera's transform/projection fields (`orthographic`, `orthographic_size`,
`position`, `rotation`, `pixel_rect`, `rect`, `aspect`, near/far clip) plus CanvasScaler values on the
canvas. This tool reports any missing reference; it exits nonzero if a required field is absent.
No game launch required.
"""
from __future__ import annotations

import argparse
import gzip
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ATLAS = ROOT / "tools" / "ui" / "ui_runtime_atlas.json.gz"
CAMERA_FIELDS = ("orthographic", "orthographic_size", "position", "rotation", "pixel_rect", "rect",
                 "aspect", "near_clip", "far_clip", "depth", "culling_mask")
CANVAS_FIELDS = ("render_mode", "camera", "plane_distance", "sorting_order")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--atlas", default=str(ATLAS))
    ap.add_argument("--session", default="default_world")
    args = ap.parse_args()
    path = Path(args.atlas)
    text = gzip.open(path, "rt", encoding="utf-8").read() if path.suffix == ".gz" else path.read_text(encoding="utf-8")
    atlas = json.loads(text)
    sess = (atlas.get("sessions") or {}).get(args.session)
    if not sess:
        print(f"FAIL: session {args.session!r} not in {path}")
        return 2
    cameras = {c.get("name"): c for c in (sess.get("cameras") or [])}
    print("screen:", sess.get("screen"))
    print("cameras:", sorted(cameras))
    missing = []
    ssc = []
    def walk(node):
        for c in node.get("controls") or []:
            if (c.get("type") or "").endswith(".Canvas") and c.get("render_mode") == "ScreenSpaceCamera":
                ssc.append((node.get("path"), c))
        for k in node.get("children") or []:
            walk(k)
    for root in sess.get("canvases") or []:
        walk(root)
    for path_, c in ssc:
        got = {k: c.get(k) for k in CANVAS_FIELDS}
        print("SSC", path_, got)
        for k in CANVAS_FIELDS:
            if got[k] is None:
                missing.append(f"{path_}: missing Canvas.{k}")
        cam = cameras.get(c.get("camera"))
        if cam is None:
            missing.append(f"{path_}: camera {c.get('camera')!r} not in cameras[]")
            continue
        for k in CAMERA_FIELDS:
            if cam.get(k) is None:
                missing.append(f"camera {c.get('camera')}: missing {k}")
    if not ssc:
        missing.append("no ScreenSpaceCamera canvases found")
    if missing:
        print("FAIL:")
        for m in missing:
            print(" -", m)
        return 1
    print(f"OK: {len(ssc)} ScreenSpaceCamera canvases have camera refs + projection fields; "
          "planeDistance present on each Canvas")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

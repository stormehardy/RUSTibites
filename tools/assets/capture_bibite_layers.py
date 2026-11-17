#!/usr/bin/env python3
"""capture_bibite_layers.py - per-SpriteRenderer layer bounds for the save-state bibite closeup (task-135).

Loads `rust_seed_600.zip` paused in the hidden lab, asks the transient plugin GET /d/bibite_layers for every
live bibite (one record per SpriteRenderer: path/name/sorting/sprite/world+local transforms/sprite rect/
pivot/PPU/world corners/union bbox), selects the bibite nearest the canonical closeup camera (-5.360003,
831.0031), picks 2 varied extra bibites (largest/smallest composited bbox), and writes
`assets/game/reference/default_save/bibite_layers.json` plus README schema/pixel conversion. Hidden-only;
plugin deployed/removed externally.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import capture as cap  # noqa: E402

SAVE = ROOT / "testdata/parity/default/rust_seed_600.zip"
OUT = ROOT / "assets/game/reference/default_save/bibite_layers.json"
CAM = {"x": -5.360003, "y": 831.0031, "ortho_size": 12.0, "width": 1280, "height": 720}


def build_args(ns):
    ns.work_dir = str(ROOT / "testdata/parity/_work")
    ns.port = 17800
    ns.plugin_port = 17801
    ns.no_plugin = False
    ns.attach = False
    ns.start_timeout = 300.0
    ns.keep_game = False
    return ns


def bbox_area(b):
    return (b.get("w") or 0) * (b.get("h") or 0)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--save", default=str(SAVE))
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--api-home", default=r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
    args = ap.parse_args()
    BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(args.api_home))
    session = cap.WorldSession(build_args(argparse.Namespace()), BridgeClient, LiveRuntime, Lab)
    try:
        client = session.open()
        if not session.plugin_ok:
            raise SystemExit("parity plugin not available (deploy ParityDump)")
        session.plugin.pacing(True)
        client.load_world(Path(args.save).absolute())
        client.set_time(paused=True)
        data = session.plugin.httpx.get(session.plugin.base + "/d/bibite_layers", timeout=120).json()
        bibites = data.get("bibites") or []
        # selected entity nearest canonical closeup camera
        def dist(b):
            p = b.get("body_position") or {}
            return (float(p.get("x") or 0) - CAM["x"]) ** 2 + (float(p.get("y") or 0) - CAM["y"]) ** 2
        selected = min(bibites, key=dist, default=None)
        if not bibites:
            raise SystemExit("no bibites in layer dump")
        by_area = sorted(bibites, key=lambda b: bbox_area(b.get("composited_bbox") or {}))
        samples = {by_area[0].get("id"), by_area[-1].get("id"), (selected or {}).get("id")}
        for b in bibites:
            b["selected"] = b is selected
            b["varied_sample"] = b.get("id") in samples
        bb = (selected or {}).get("composited_bbox") or {}
        ppu_screen = CAM["height"] / (2.0 * CAM["ortho_size"])
        out = {
            "schema": "rustibites.assets.bibite_layers.v1",
            "source": "hidden game, transient ParityDump GET /d/bibite_layers",
            "save": str(Path(args.save).absolute()), "paused": True,
            "world_counts": client.status().get("counts"),
            "reference_camera": {**CAM, "pixels_per_world_unit": ppu_screen,
                                 "screen_px_formula": "x_px = 640 + (x - cam.x)*30; y_px_up = 360 + (y - cam.y)*30; "
                                                      "image_y = 720 - y_px_up (web/image top-left)",
                                 "capture_note": "1280x720 orthographic closeup at ortho size 12; 30 px per world unit"},
            "selected_id": (selected or {}).get("id"),
            "selected_composited_bbox": bb,
            "selected_bbox_pixels": {
                "x0": 640 + ((bb.get("x0") or 0) - CAM["x"]) * ppu_screen,
                "x1": 640 + ((bb.get("x1") or 0) - CAM["x"]) * ppu_screen,
                "y0": 720 - (360 + ((bb.get("y1") or 0) - CAM["y"]) * ppu_screen),
                "y1": 720 - (360 + ((bb.get("y0") or 0) - CAM["y"]) * ppu_screen),
            } if bb else None,
            "bibites": bibites,
        }
        outp = Path(args.out)
        outp.parent.mkdir(parents=True, exist_ok=True)
        outp.write_text(json.dumps(out, indent=1), encoding="utf-8")
        print(f"[bibite-layers] wrote {outp} ({outp.stat().st_size} B), bibites={len(bibites)}, "
              f"selected={out['selected_id']} bbox={bb}")
        print(f"[bibite-layers] samples={sorted(x for x in samples if x is not None)}")
    finally:
        session.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

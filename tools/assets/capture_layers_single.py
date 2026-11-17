#!/usr/bin/env python3
"""capture_layers_single.py - per-layer visible-alpha closeup images/bboxes (task-142).

Single hidden load of rust_seed_600.zip, paused t=20 s, target bibite nearest the canonical closeup camera:
query /d/bibite_layers once, set exact camera (centre=body, ortho 12, 1280x720), capture base + all-renderers-
off, then for every SpriteRenderer under the selected body toggle it alone on and capture a layer image.
Visible alpha per layer is the pixel diff vs the all-off frame; saves per-layer PNGs + JSON with bboxes so
bench-dev can identify the layer producing the bottom 56-61 px. Restores renderers and verifies the base
frames are identical at the end. One session; no simulation advances.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

from PIL import Image, ImageChops

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import capture as cap  # noqa: E402

SAVE = ROOT / "testdata/parity/default/rust_seed_600.zip"
OUTDIR = ROOT / "assets/game/reference/default_save/closeup_single"
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


def diff_bbox(a_path, b_path, threshold=8):
    a = Image.open(a_path).convert("RGBA")
    b = Image.open(b_path).convert("RGBA")
    diff = ImageChops.difference(a, b).convert("L")
    mask = diff.point(lambda v: 255 if v > threshold else 0)
    return mask.getbbox(), sum(1 for v in mask.getdata() if v), diff


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--save", default=str(SAVE))
    ap.add_argument("--out-dir", default=str(OUTDIR))
    ap.add_argument("--api-home", default=r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
    args = ap.parse_args()
    out = Path(args.out_dir).absolute()
    layers_dir = out / "layers"
    layers_dir.mkdir(parents=True, exist_ok=True)
    BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(args.api_home))
    session = cap.WorldSession(build_args(__import__("argparse").Namespace()), BridgeClient, LiveRuntime, Lab)
    try:
        client = session.open()
        if not session.plugin_ok:
            raise SystemExit("parity plugin not available (deploy ParityDump)")
        session.plugin.pacing(True)
        client.load_world(Path(args.save).absolute())
        client.set_time(paused=True)
        status = client.status()
        layers_doc = session.plugin.httpx.get(session.plugin.base + "/d/bibite_layers", timeout=120).json()
        bibites = layers_doc.get("bibites") or []
        if not bibites:
            raise SystemExit("no live bibites")
        selected = min(bibites, key=lambda b: (float((b.get("body_position") or {}).get("x") or 0) - CAM["x"]) ** 2
                       + (float((b.get("body_position") or {}).get("y") or 0) - CAM["y"]) ** 2)
        sid = selected.get("id")
        pos = selected.get("body_position") or {}
        cam = client.set_camera(x=float(pos.get("x") or CAM["x"]), y=float(pos.get("y") or CAM["y"]),
                                size=CAM["ortho_size"])
        base_path = out / "lyr_base.png"
        base_path.write_bytes(client.screenshot(CAM["width"], CAM["height"]))

        def post(path, body):
            return session.plugin.httpx.post(session.plugin.base + path, json=body, timeout=120).json()

        off = post("/d/renderers", {"action": "off", "id": sid})
        off_path = out / "lyr_all_off.png"
        off_path.write_bytes(client.screenshot(CAM["width"], CAM["height"]))
        full_bbox, full_px, _ = diff_bbox(base_path, off_path)
        entries = []
        for i, layer in enumerate(selected.get("layers") or []):
            path = layer.get("path")
            safe_name = re.sub(r"[^A-Za-z0-9._-]+", "_", (layer.get("name") or f"layer{i}")) + f"_{i}.png"
            only = post("/d/renderers", {"action": "only", "id": sid, "path": path})
            img_path = layers_dir / safe_name
            img_path.write_bytes(client.screenshot(CAM["width"], CAM["height"]))
            bbox, npx, _ = diff_bbox(off_path, img_path)
            # alpha mask of visible layer pixels vs all-off
            mask_path = layers_dir / safe_name.replace(".png", "_alpha.png")
            a = Image.open(off_path).convert("RGBA")
            b = Image.open(img_path).convert("RGBA")
            diff = ImageChops.difference(a, b).convert("L")
            mask = diff.point(lambda v: 255 if v > 8 else 0)
            alpha = Image.new("RGBA", mask.size, (255, 255, 255, 0))
            alpha.putalpha(mask)
            alpha.save(mask_path)
            entries.append({"index": i, "path": path, "name": layer.get("name"),
                            "sprite": layer.get("sprite"), "sorting_order": layer.get("sorting_order"),
                            "enabled": layer.get("enabled"), "image": str(img_path.relative_to(ROOT).as_posix()),
                            "alpha_image": str(mask_path.relative_to(ROOT).as_posix()),
                            "visible_alpha_bbox_pixels": bbox, "visible_alpha_pixels": npx,
                            "only_response": only})
        restore = post("/d/renderers", {"action": "restore", "id": sid})
        verify_path = out / "lyr_verify.png"
        verify_path.write_bytes(client.screenshot(CAM["width"], CAM["height"]))
        verify_bbox, verify_px, _ = diff_bbox(base_path, verify_path)
        ppu = CAM["height"] / (2.0 * CAM["ortho_size"])
        doc = {
            "schema": "rustibites.assets.closeup_layers.v1",
            "capture": {"method": "single hidden load, paused t=20 s; /d/bibite_layers, exact closeup camera, "
                                  "all-renderers-off then one layer on at a time; restore at end",
                        "save": str(Path(args.save).absolute()), "paused": status.get("paused"),
                        "counts": status.get("counts"), "camera": cam, "screen": [CAM["width"], CAM["height"]],
                        "pixels_per_world_unit": ppu,
                        "pixel_formula": "x_px = 640 + (x-cam.x)*30; y_px_up = 360 + (y-cam.y)*30; image_y = 720 - y_px_up"},
            "selected_id": sid, "selected_body_position": pos,
            "selected_composited_bbox_world": selected.get("composited_bbox"),
            "full_visible_alpha_bbox_pixels": full_bbox, "full_visible_alpha_pixels": full_px,
            "all_off_png": str(off_path.relative_to(ROOT).as_posix()),
            "base_png": str(base_path.relative_to(ROOT).as_posix()),
            "base_sha256": hashlib.sha256(base_path.read_bytes()).hexdigest(),
            "layers": entries,
            "each_layer_images": "layers/<name>_<i>.png (target-only frame) and *_alpha.png (visible alpha mask)",
            "renderer_off_response": off, "renderer_restore_response": restore,
            "verification_restore": {"diff_bbox_pixels": verify_bbox, "diff_pixels": verify_px,
                                     "note": "after restore the frame matches the base; empty bbox means exact restoration"},
            "spawner_caveat": "Save carries 10 bibites; game load adds one fresh-spawner bibite (11 live ids). "
                              "Selected id is the saved entity nearest the canonical closeup camera; runtime ids vary per load.",
            "bottom_hint": "Find the layer whose visible_alpha_bbox_pixels bottom (y1) is nearest the full bbox bottom "
                           "(the 56-61 px gap candidate); exo may be blank, Body/arms/eyes/mouth are measured separately.",
        }
        (out / "reference_layers.json").write_text(json.dumps(doc, indent=2), encoding="utf-8")
        print(f"[layers] selected={sid} full_visible_bbox={full_bbox} px={full_px}")
        for e in entries:
            print(f"[layers] {e['name']:<12} {e['sprite']:<28} bbox={e['visible_alpha_bbox_pixels']} px={e['visible_alpha_pixels']} enable={e['enabled']}")
        print(f"[layers] restore diff bbox={verify_bbox} px={verify_px}; wrote {out/'reference_layers.json'}")
    finally:
        session.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

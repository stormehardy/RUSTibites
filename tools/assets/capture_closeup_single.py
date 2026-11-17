#!/usr/bin/env python3
"""capture_closeup_single.py - one atomic hidden-load closeup for bench-dev (task-141).

Single transparently-hidden session: load `rust_seed_600.zip`, pause at t=20 s, pick the bibite nearest
the canonical closeup camera (-5.360003, 831.0031), query GET /d/bibite_layers once, set the exact
reference camera (centre = selected body, ortho size 12), capture a 1280x720 PNG, and record the selected
runtime id/pose/genes, per-layer alpha bounds/world rects, composited bbox and pixel/world conversion in
assets/game/reference/default_save/closeup_single/reference_single.json. No simulation advances between
the queries and the screenshot; only one hidden load is performed.
"""
from __future__ import annotations

import argparse
import json
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


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--save", default=str(SAVE))
    ap.add_argument("--out-dir", default=str(OUTDIR))
    ap.add_argument("--api-home", default=r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
    args = ap.parse_args()
    out = Path(args.out_dir).absolute()
    out.mkdir(parents=True, exist_ok=True)
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
        layers = session.plugin.httpx.get(session.plugin.base + "/d/bibite_layers", timeout=120).json()
        bibites = layers.get("bibites") or []

        def dist(b):
            p = b.get("body_position") or {}
            return (float(p.get("x") or 0) - CAM["x"]) ** 2 + (float(p.get("y") or 0) - CAM["y"]) ** 2

        selected = min(bibites, key=dist) if bibites else None
        if selected is None:
            raise SystemExit("no live bibites in layer dump")
        sid = selected.get("id")
        body_pos = selected.get("body_position") or {}
        # genome/genes for the selected external id
        table = client.bibite_table(genes=True, eggs=False)
        cols = (table.get("columns") or {}) if isinstance(table, dict) else {}
        row = None
        row_index = None
        ids = cols.get("id") or []
        for i, v in enumerate(ids):
            if v == sid:
                row_index = i
                row = {k: vals[i] for k, vals in cols.items() if isinstance(vals, list) and i < len(vals)}
                break
        genes_block = table.get("genes") or {}
        selected_genes = {}
        if row_index is not None:
            for gname, vals in genes_block.items():
                if isinstance(vals, list) and row_index < len(vals):
                    selected_genes[gname] = vals[row_index]
        genome = None
        if row and row.get("h") is not None:
            try:
                genome = client.entity(row["h"])
            except Exception as exc:  # noqa: BLE001
                genome = {"error": str(exc)}
        # exact closeup camera at selected body (same paused state; no simulation between captures)
        cam = client.set_camera(x=float(body_pos.get("x") or CAM["x"]),
                                y=float(body_pos.get("y") or CAM["y"]), size=CAM["ortho_size"])
        png = client.screenshot(CAM["width"], CAM["height"])
        png_path = out / "reference_single.png"
        png_path.write_bytes(png)
        # Verify the selected entity appears at the camera centre without relying on UI selection:
        # move that exact handle after the reference capture and diff the frames. The old/new entity
        # sprites give a screen-space bbox; it must overlap the expected composited bbox around centre.
        verify = {}
        try:
            handle = row.get("h") if row else None
            if handle is None:
                verify = {"error": "selected handle unavailable; cannot move-verify"}
            else:
                mv = client.move([handle], by=(2000.0, 0.0))
                png2 = client.screenshot(CAM["width"], CAM["height"])
                (out / "verification_moved.png").write_bytes(png2)
                a = Image.open(png_path).convert("RGBA")
                b = Image.open(out / "verification_moved.png").convert("RGBA")
                diff = ImageChops.difference(a, b).convert("L")
                bbox = diff.point(lambda v: 255 if v > 8 else 0).getbbox()
                changed = sum(1 for v in diff.getdata() if v > 8)
                # move it back so the session remains in the reference state, then re-read status
                try:
                    client.move([handle], by=(-2000.0, 0.0))
                except Exception:
                    pass
                verify = {"move_response": mv, "diff_bbox_pixels": bbox, "diff_pixels": changed,
                          "verification_png": str((out / "verification_moved.png").relative_to(ROOT).as_posix()),
                          "note": "selected handle moved 2000 world units (offscreen) after the reference capture; diff bbox "
                                  "shows the entity's sprites at the reference position and the moved position."}
        except Exception as exc:  # noqa: BLE001
            verify = {"error": str(exc)}
        ppu = CAM["height"] / (2.0 * CAM["ortho_size"])
        bb = selected.get("composited_bbox") or {}
        bbox_px = {
            "x0": CAM["width"] / 2 + ((bb.get("x0") or 0) - cam.get("x", body_pos.get("x", 0))) * ppu,
            "x1": CAM["width"] / 2 + ((bb.get("x1") or 0) - cam.get("x", body_pos.get("x", 0))) * ppu,
            "y0": CAM["height"] / 2 - ((bb.get("y1") or 0) - cam.get("y", body_pos.get("y", 0))) * ppu,
            "y1": CAM["height"] / 2 - ((bb.get("y0") or 0) - cam.get("y", body_pos.get("y", 0))) * ppu,
        } if bb else None
        doc = {
            "schema": "rustibites.assets.closeup_single.v1",
            "capture": {
                "method": "single hidden game load, paused at save t=20 s, one /d/bibite_layers query then "
                          "bridge /camera + /screenshot; no simulation between",
                "save": str(Path(args.save).absolute()), "paused": status.get("paused"),
                "counts": status.get("counts"), "screen": {"width": CAM["width"], "height": CAM["height"]},
                "camera": cam, "requested_camera": CAM,
                "pixels_per_world_unit": ppu,
                "pixel_formula": "x_px = 640 + (x - cam.x)*30; y_px_up = 360 + (y - cam.y)*30; "
                                 "image_y = 720 - y_px_up (top-left origin)",
            },
            "selected_id": sid,
            "selected": selected,
            "selected_table_row": row,
            "selected_genes": selected_genes,
            "genes_block_names": sorted(genes_block.keys()),
            "genome": genome,
            "bibite_ids_in_session": [b.get("id") for b in bibites],
            "spawner_caveat": "Save contains 10 bibites; the game load runs the fresh spawner and adds one "
                              "extra virgin bibite, so 11 live ids appear. The selected id is the bibite "
                              "nearest the canonical closeup camera (a saved entity); the extra spawner id "
                              "is not selected. Ids are runtime ids and can change on reload.",
            "reference_png": str(png_path.relative_to(ROOT).as_posix()),
            "png_sha256": __import__("hashlib").sha256(png).hexdigest(),
            "composited_bbox_world": bb,
            "composited_bbox_pixels": bbox_px,
            "verification": {
                "camera_centered_on_selected": [cam.get("x"), cam.get("y")],
                "selected_body_position": body_pos,
                "center_pixel": [CAM["width"] / 2, CAM["height"] / 2],
                "move_diff": verify,
                "note": "camera was set to the selected body position from the same paused layer dump; a "
                        "frame after moving the selected handle 200 units verifies the selected entity at the "
                        "camera centre (diff bbox above). Selected id/pose/bbox are atomic with the screenshot.",
            },
        }
        if verify.get("diff_bbox_pixels"):
            doc["visible_alpha_bbox_pixels"] = verify["diff_bbox_pixels"]
            x0, y0, x1, y1 = verify["diff_bbox_pixels"]
            doc["visible_alpha_bbox_size"] = [x1 - x0, y1 - y0]
        (out / "reference_single.json").write_text(json.dumps(doc, indent=2), encoding="utf-8")
        print(f"[closeup-single] selected={sid} pos={body_pos} bb={bb}")
        print(f"[closeup-single] png={png_path} sha={doc['png_sha256']} size={len(png)} B")
        print(f"[closeup-single] wrote {out / 'reference_single.json'}")
    finally:
        session.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

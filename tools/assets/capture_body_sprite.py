#!/usr/bin/env python3
"""capture_body_sprite.py - exact live Body Sprite/Texture dump for the atomic closeup (task-144).

One hidden load of rust_seed_600.zip paused t=20 s; /d/bibite_layers selects the bibite at the closeup
camera; POST /d/sprite_dump {id, layer:"Body"} returns the live Sprite identity (name/rect/pivot/PPU),
Texture2D name/dimensions/format and a full-texture PNG. Saves raw full texture + cropped cell + metadata
under testdata/parity/default/body_sprite_dump/, compares with extracted
assets/game/sheets/normal_body.png + sprites/normal_body_0.png, and writes a small
assets/game/reference/default_save/closeup_single/body_sprite_ref.json. Hidden-only; plugin
deployed/removed externally.
"""
from __future__ import annotations

import base64
import hashlib
import io
import json
import sys
from pathlib import Path

from PIL import Image, ImageChops

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import capture as cap  # noqa: E402

SAVE = ROOT / "testdata/parity/default/rust_seed_600.zip"
RAW = ROOT / "testdata/parity/default/body_sprite_dump"
REF = ROOT / "assets/game/reference/default_save/closeup_single"
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


def sha(b):
    return hashlib.sha256(b).hexdigest()


def main():
    ap = __import__("argparse").ArgumentParser(description=__doc__, formatter_class=__import__("argparse").RawDescriptionHelpFormatter)
    ap.add_argument("--save", default=str(SAVE))
    ap.add_argument("--api-home", default=r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
    args = ap.parse_args()
    RAW.mkdir(parents=True, exist_ok=True)
    REF.mkdir(parents=True, exist_ok=True)
    BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(args.api_home))
    session = cap.WorldSession(build_args(__import__("argparse").Namespace()), BridgeClient, LiveRuntime, Lab)
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
        dump = session.plugin.httpx.post(session.plugin.base + "/d/sprite_dump",
                                          json={"id": sid, "layer": "Body"}, timeout=120).json()
        if not dump.get("ok"):
            raise SystemExit(f"sprite_dump failed: {dump}")
        full = base64.b64decode(dump["texture_png_base64"])
        (RAW / "live_normal_body_full.png").write_bytes(full)
        tex = Image.open(io.BytesIO(full)).convert("RGBA")
        r = dump["sprite_rect"]
        x, y, w, h = (int(round(r[k])) for k in ("x", "y", "w", "h"))
        top = tex.height - y - h
        cell = tex.crop((x, top, x + w, top + h))
        cell.save(RAW / "live_body_cell.png")
        ext_sheet_path = ROOT / "assets/game/sheets/normal_body.png"
        ext_cell_path = ROOT / "assets/game/sprites/normal_body_0.png"
        ext_sheet = Image.open(ext_sheet_path).convert("RGBA")
        ext_cell = Image.open(ext_cell_path).convert("RGBA")
        cmp_full = {"same_size": (tex.width, tex.height) == (ext_sheet.width, ext_sheet.height),
                    "pixel_equal": list(tex.getdata()) == list(ext_sheet.getdata()),
                    "live_sha256": sha(full), "extracted_sha256": sha(ext_sheet_path.read_bytes())}
        cmp_cell = {"same_size": (cell.width, cell.height) == (ext_cell.width, ext_cell.height),
                    "pixel_equal": list(cell.getdata()) == list(ext_cell.getdata()),
                    "live_alpha_nontransparent": sum(1 for a in cell.getchannel("A").getdata() if a),
                    "extracted_alpha_nontransparent": sum(1 for a in ext_cell.getchannel("A").getdata() if a)}
        # per-cell alpha grid of the live full texture using rect cell size
        cells = []
        if w > 0 and h > 0:
            for row in range(tex.height // h):
                rowcounts = []
                for col in range(tex.width // w):
                    c = tex.crop((col * w, row * h, (col + 1) * w, (row + 1) * h))
                    rowcounts.append(sum(1 for a in c.getchannel("A").getdata() if a))
                cells.append(rowcounts)
        meta = {"schema": "rustibites.assets.body_sprite_live.v1",
                "capture": {"method": "single hidden load paused t=20s; POST /d/sprite_dump layer=Body",
                            "save": str(Path(args.save).absolute()), "selected_id": sid},
                "sprite_dump": {k: v for k, v in dump.items() if k != "texture_png_base64"},
                "row_col_alpha": cells,
                "compare_full_texture": cmp_full, "compare_cell_vs_extracted": cmp_cell,
                "files": {"live_full": str((RAW / "live_normal_body_full.png").relative_to(ROOT).as_posix()),
                          "live_cell": str((RAW / "live_body_cell.png").relative_to(ROOT).as_posix()),
                          "extracted_sheet": "assets/game/sheets/normal_body.png",
                          "extracted_cell": "assets/game/sprites/normal_body_0.png"}}
        (RAW / "meta.json").write_text(json.dumps(meta, indent=2), encoding="utf-8")
        ref = {"schema": "rustibites.assets.body_sprite_ref.v1",
               "selected_id": sid,
               "sprite": dump.get("sprite"), "texture": dump.get("texture"),
               "texture_size": [dump.get("texture_width"), dump.get("texture_height")],
               "texture_format": dump.get("texture_format"),
               "sprite_rect": dump.get("sprite_rect"), "sprite_pivot": dump.get("sprite_pivot"),
               "pixels_per_unit": dump.get("pixels_per_unit"),
               "live_cell_nontransparent": cmp_cell["live_alpha_nontransparent"],
               "extracted_cell_nontransparent": cmp_cell["extracted_alpha_nontransparent"],
               "full_texture_matches_extracted": cmp_full["pixel_equal"],
               "cell_matches_extracted": cmp_cell["pixel_equal"],
               "raw_dump": "testdata/parity/default/body_sprite_dump/meta.json",
               "note": "normal_body_00 maps to the live texture cell at sprite_rect; compare extracted sheet/cell."}
        (REF / "body_sprite_ref.json").write_text(json.dumps(ref, indent=2), encoding="utf-8")
        print(json.dumps({"selected_id": sid, "sprite_dump": ref, "compare_full": cmp_full, "compare_cell": cmp_cell,
                          "row_col_alpha": cells}, indent=1)[:4000])
    finally:
        session.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

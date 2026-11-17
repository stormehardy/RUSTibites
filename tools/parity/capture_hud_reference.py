#!/usr/bin/env python3
"""capture_hud_reference.py - hidden UI-inclusive reference frames at 1920x1080 (task-112 follow-up).

Loads the canonical save `testdata/parity/default/rust_seed_600.zip` in the hidden game, pauses at the
save time, asks the transient ParityDump plugin for a transparent UICamera render (ScreenSpaceCamera UI),
composites it over the bridge main-camera world screenshot with PIL, and writes:
  assets/game/reference/default_save/hud_default_1920x1080.png    (no selection)
  assets/game/reference/default_save/hud_inspector_1920x1080.png  (bibite selected via game UserControl)
plus hud_capture.json recording method/screen/state/selected id/panel/counts. Plugin routes used:
GET /d/ui_overlay, POST /d/select_bibite. Hidden-window only; caller removes the plugin afterwards.
"""
from __future__ import annotations

import argparse
import base64
import io
import json
import sys
from pathlib import Path

from PIL import Image

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import capture as cap  # noqa: E402

SAVE = ROOT / "testdata" / "parity" / "default" / "rust_seed_600.zip"
OUT = ROOT / "assets" / "game" / "reference" / "default_save"


def build_args(ns):
    ns.work_dir = str(ROOT / "testdata" / "parity" / "_work")
    ns.port = 17800
    ns.plugin_port = 17801
    ns.no_plugin = False
    ns.attach = False
    ns.start_timeout = 300.0
    ns.keep_game = False
    return ns


def plugin_json(plugin, path, body=None):
    if body is None:
        return plugin.httpx.get(plugin.base + path, timeout=120).json()
    return plugin.httpx.post(plugin.base + path, json=body, timeout=120).json()


def composite(world_png: bytes, overlay_json: dict) -> Image.Image:
    world = Image.open(io.BytesIO(world_png)).convert("RGBA")
    ov = Image.open(io.BytesIO(base64.b64decode(overlay_json["png_base64"]))).convert("RGBA")
    if ov.size != world.size:
        ov = ov.resize(world.size, Image.LANCZOS)
    return Image.alpha_composite(world, ov)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--save", default=str(SAVE))
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--select-id", type=int, default=946151190)
    ap.add_argument("--api-home", default=r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
    args = ap.parse_args()
    out = Path(args.out).absolute()
    out.mkdir(parents=True, exist_ok=True)
    BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(args.api_home))
    session = cap.WorldSession(build_args(argparse.Namespace()), BridgeClient, LiveRuntime, Lab)
    meta = {"schema": "rustibites.assets.hud_reference.v1",
            "capture_method": "hidden BibitesAPI lab; bridge GET /screenshot (main camera) + ParityDump "
                              "GET /d/ui_overlay (transparent UICamera RenderTexture), composited with PIL",
            "save": str(Path(args.save).absolute()), "screen": {"width": 1920, "height": 1080},
            "ui_scale": 1.0, "frames": []}
    try:
        client = session.open()
        if not session.plugin_ok:
            raise SystemExit("parity plugin not available (deploy ParityDump)")
        session.plugin.pacing(True)
        client.load_world(Path(args.save).absolute())
        client.set_time(paused=True)
        status = client.status()
        meta["world"] = {"simulated_time_s": status.get("simulatedTime"), "paused": status.get("paused"),
                         "counts": status.get("counts")}
        # canonical save entity ids (zip member names are not ids; table ids are runtime ids)
        table = client.bibite_table(genes=False, eggs=False)
        cols = (table.get("columns") or {}) if isinstance(table, dict) else {}
        ids = [int(v) for v in (cols.get("id") or []) if v is not None]
        xs = [float(v) for v in (cols.get("x") or [])]
        ys = [float(v) for v in (cols.get("y") or [])]
        meta["game_bibite_ids"] = ids
        meta["game_bibite_count"] = len(ids)
        meta["game_bibite_positions"] = [{ "id": ids[i], "x": xs[i], "y": ys[i]} for i in range(min(len(ids), len(xs), len(ys)))]
        # compare with the save's 10 bibite positions to identify the spawner-added extra entity
        import zipfile
        save_pos = []
        with zipfile.ZipFile(Path(args.save)) as z:
            for n in z.namelist():
                if n.startswith("bibites/") and n.endswith(".bb8"):
                    d = json.loads(z.read(n).decode("utf-8-sig"))
                    pos = ((d.get("transform") or {}).get("position") or [None, None])[:2]
                    save_pos.append({"file": n, "x": pos[0], "y": pos[1]})
        meta["save_bibite_count"] = len(save_pos)
        matched = set()
        for sp in save_pos:
            best = None; bd = 1e9
            for gp in meta["game_bibite_positions"]:
                d = (gp["x"] - sp["x"]) ** 2 + (gp["y"] - sp["y"]) ** 2
                if d < bd: bd = d; best = gp
            if best and bd < 1.0:
                matched.add(best["id"])
        meta["added_on_load"] = [gp for gp in meta["game_bibite_positions"] if gp["id"] not in matched]

        # ---- default HUD (no selection) ---------------------------------------------------------
        cam = client.set_camera(fit=True)
        world_png = client.screenshot(1920, 1080)
        ov = plugin_json(session.plugin, "/d/ui_overlay")
        if not ov.get("ok"):
            raise SystemExit(f"ui_overlay failed: {ov}")
        ov_img = Image.open(io.BytesIO(base64.b64decode(ov["png_base64"]))).convert("RGBA")
        ov_alpha = sum(1 for a in ov_img.getchannel("A").getdata() if a > 0)
        meta["ui_overlay_alpha_pixels"] = ov_alpha
        op = out / "hud_overlay_1920x1080.png"
        ov_img.save(op)
        if ov_alpha == 0:
            raise SystemExit("ui_overlay is fully transparent (UI not rendered)")
        meta["ui_overlay_file"] = str(op.relative_to(ROOT).as_posix())
        img = composite(world_png, ov)
        p = out / "hud_default_1920x1080.png"
        img.save(p)
        meta["frames"].append({"file": str(p.relative_to(ROOT).as_posix()), "camera": cam,
                               "selected_id": None, "panel": None, "ui_overlay_camera": ov.get("camera"),
                               "ui_overlay_alpha_pixels": ov_alpha})
        print(f"[hud] wrote {p} ({p.stat().st_size} B) counts={meta['world']['counts']}")

        # ---- selected bibite + inspector --------------------------------------------------------
        # pick the runtime id nearest the saved reference bibite position (-5.36, 831.003)
        target = {"x": -5.360003, "y": 831.0031}
        near = min(meta["game_bibite_positions"],
                   key=lambda gp: (gp["x"] - target["x"]) ** 2 + (gp["y"] - target["y"]) ** 2, default=None)
        sel = plugin_json(session.plugin, "/d/select_bibite",
                          {"id": near["id"]} if near else {"x": target["x"], "y": target["y"]})
        if not sel.get("ok"):
            sel = plugin_json(session.plugin, "/d/select_bibite", target)
        meta["selection"] = sel
        if sel.get("ok"):
            pos = sel.get("pos") or {}
            # let the selection panel/tabs animate in (paused UI transitions do not advance)
            client.set_time(paused=False)
            import time as _time
            _time.sleep(0.6)
            client.set_time(paused=True)
            press = plugin_json(session.plugin, "/d/press_button", {"name": "OpenData"})
            meta["stats_press"] = press
            if press.get("ok"):
                client.set_time(paused=False)
                _time.sleep(0.4)
                client.set_time(paused=True)
            cam2 = client.set_camera(x=float(pos.get("x") or 0), y=float(pos.get("y") or 0), size=400.0)
            world_png2 = client.screenshot(1920, 1080)
            ov2 = plugin_json(session.plugin, "/d/ui_overlay")
            if not ov2.get("ok"):
                raise SystemExit(f"ui_overlay (inspector) failed: {ov2}")
            ov2_img = Image.open(io.BytesIO(base64.b64decode(ov2["png_base64"]))).convert("RGBA")
            op2 = out / "hud_inspector_overlay_1920x1080.png"
            ov2_img.save(op2)
            img2 = composite(world_png2, ov2)
            p2 = out / "hud_inspector_1920x1080.png"
            img2.save(p2)
            meta["frames"].append({"file": str(p2.relative_to(ROOT).as_posix()), "camera": cam2,
                                   "selected_id": sel.get("selected_id"), "panel": "Stats (OpenData)",
                                   "ui_overlay_camera": ov2.get("camera"),
                                   "ui_overlay_alpha_pixels": sum(1 for a in ov2_img.getchannel("A").getdata() if a > 0)})
            print(f"[hud] wrote {p2} ({p2.stat().st_size} B) selected={sel.get('selected_id')} {pos}")
        else:
            print("[hud] selection failed:", sel)
        (out / "hud_capture.json").write_text(json.dumps(meta, indent=2), encoding="utf-8")
        # update reference.json with HUD frame entries/counts
        rp = out / "reference.json"
        if rp.exists():
            d = json.loads(rp.read_text(encoding="utf-8"))
            d["hud_frames"] = meta["frames"]
            d["hud_capture_method"] = meta["capture_method"]
            d["hud_world"] = meta["world"]
            d["hud_selection"] = meta.get("selection")
            d["hud_game_bibite_ids"] = ids
            rp.write_text(json.dumps(d, indent=2), encoding="utf-8")
        print(json.dumps(meta, indent=2)[:2500])
    finally:
        session.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""capture_ui_runtime_save.py - save-state runtime UI dump + HUD/inspector overlays (task-121).

Loads `rust_seed_600.zip` paused at t=20 s in the hidden game, fit camera (ruler `200u`), dumps the runtime
UI as the save-state default HUD, then selects a bibite and presses `StatsButton` (the real game path that
opens StatsPanel), lets the panel animate, and dumps the inspector state. Publishes sessions
`default_world` (save HUD) and `default_world_inspector` in `assets/game/ui_runtime.json.gz` (old newgame
session renamed `default_world_newgame`), plus UI-only overlays
`hud_savestate_{,overlay_}1920x1080.png` / `hud_inspector_expanded_{,overlay_}1920x1080.png` and ruler
metadata. Hidden-only; plugin deployed/removed externally.
"""
from __future__ import annotations

import argparse
import base64
import gzip
import io
import json
import re
import sys
import time
from pathlib import Path

from PIL import Image

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import capture as cap  # noqa: E402

SAVE = ROOT / "testdata/parity/default/rust_seed_600.zip"
RUNTIME = ROOT / "assets/game/ui_runtime.json.gz"
OUT = ROOT / "assets/game/ui_runtime_save.json.gz"
OUTDIR = ROOT / "assets/game/reference/default_save"


def build_args(ns):
    ns.work_dir = str(ROOT / "testdata/parity/_work")
    ns.port = 17800
    ns.plugin_port = 17801
    ns.no_plugin = False
    ns.attach = False
    ns.start_timeout = 300.0
    ns.keep_game = False
    return ns


def post(plugin, path, body):
    return plugin.httpx.post(plugin.base + path, json=body, timeout=120).json()


def overlay(plugin):
    ov = plugin.httpx.get(plugin.base + "/d/ui_overlay", timeout=120).json()
    if not ov.get("ok"):
        raise RuntimeError(f"ui_overlay failed: {ov}")
    return base64.b64decode(ov["png_base64"])


def composite(world_png, ui_png):
    world = Image.open(io.BytesIO(world_png)).convert("RGBA")
    ui = Image.open(io.BytesIO(ui_png)).convert("RGBA")
    if ui.size != world.size:
        ui = ui.resize(world.size, Image.LANCZOS)
    return Image.alpha_composite(world, ui)


def alpha_pixels(png):
    return sum(1 for a in Image.open(io.BytesIO(png)).convert("RGBA").getchannel("A").getdata() if a > 0)


def measure_ruler(dump):
    labels, brackets = [], []

    def walk(node):
        for t in node.get("texts") or []:
            txt = str(t.get("text") or "")
            if re.fullmatch(r"\d+u", txt):
                labels.append({"path": node.get("path"), "text": txt, "rect": node.get("rect"),
                               "active_in_hierarchy": node.get("active_in_hierarchy")})
        for im in node.get("images") or []:
            spr = (im.get("sprite") or {}).get("name") or ""
            if "bracket" in spr.lower() or "ruler" in spr.lower():
                brackets.append({"path": node.get("path"), "sprite": spr, "rect": node.get("rect"),
                                 "sprite_manifest": im.get("sprite")})
        for k in node.get("children") or []:
            walk(k)

    for r in dump.get("canvases") or []:
        walk(r)
    return {"labels": labels, "bracket_nodes": brackets}


def stats_panel_active(dump):
    found = []

    def walk(node):
        p = node.get("path") or ""
        if p.endswith("/StatsPanel") or p.endswith("/DataPage"):
            found.append({"path": p, "active_in_hierarchy": node.get("active_in_hierarchy")})
        for k in node.get("children") or []:
            walk(k)

    for r in dump.get("canvases") or []:
        walk(r)
    return found


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--save", default=str(SAVE))
    ap.add_argument("--runtime", default=str(RUNTIME))
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--select-position", default="-5.360003,831.0031")
    ap.add_argument("--api-home", default=r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
    args = ap.parse_args()
    BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(args.api_home))
    session = cap.WorldSession(build_args(argparse.Namespace()), BridgeClient, LiveRuntime, Lab)
    sessions = {}
    OUTDIR.mkdir(parents=True, exist_ok=True)
    try:
        client = session.open()
        if not session.plugin_ok:
            raise SystemExit("parity plugin not available (deploy ParityDump)")
        session.plugin.pacing(True)
        client.load_world(Path(args.save).absolute())
        client.set_time(paused=True)
        cam = client.set_camera(fit=True)
        time.sleep(0.3)
        dump = session.plugin.ui_runtime()
        ruler = measure_ruler(dump)
        world_png = client.screenshot(1920, 1080)
        ov = overlay(session.plugin)
        (OUTDIR / "hud_savestate_overlay_1920x1080.png").write_bytes(ov)
        composite(world_png, ov).save(OUTDIR / "hud_savestate_1920x1080.png")
        dump["state"] = {"save": str(Path(args.save).absolute()), "paused": True, "screen": dump.get("screen"),
                         "camera": cam, "ruler": ruler, "overlay": "assets/game/reference/default_save/"
                         "hud_savestate_overlay_1920x1080.png", "overlay_alpha_pixels": alpha_pixels(ov)}
        sessions["default_world"] = dump
        print(f"[ui-save] default_world: nodes={dump.get('node_count')} ruler={ruler['labels']} "
              f"brackets={len(ruler['bracket_nodes'])} alpha={dump['state']['overlay_alpha_pixels']}")

        # selected + Stats panel via the game's real StatsButton
        sel = None
        try:
            x, y = (float(v) for v in args.select_position.split(","))
            sel = post(session.plugin, "/d/select_bibite", {"x": x, "y": y})
            if sel.get("ok"):
                client.set_time(paused=False)
                time.sleep(0.7)
                client.set_time(paused=True)
                press = post(session.plugin, "/d/press_button", {"name": "StatsButton"})
                if press.get("ok"):
                    client.set_time(paused=False)
                    time.sleep(0.8)
                    client.set_time(paused=True)
                dump2 = session.plugin.ui_runtime()
                ruler2 = measure_ruler(dump2)
                pos = sel.get("pos") or {}
                cam2 = client.set_camera(x=float(pos.get("x") or 0), y=float(pos.get("y") or 0), size=400.0)
                world2 = client.screenshot(1920, 1080)
                ov2 = overlay(session.plugin)
                (OUTDIR / "hud_inspector_expanded_overlay_1920x1080.png").write_bytes(ov2)
                composite(world2, ov2).save(OUTDIR / "hud_inspector_expanded_1920x1080.png")
                dump2["state"] = {"save": str(Path(args.save).absolute()), "paused": True,
                                  "camera": cam2, "selected": sel, "press": press,
                                  "stats_panels": stats_panel_active(dump2), "ruler": ruler2,
                                  "overlay": "assets/game/reference/default_save/"
                                  "hud_inspector_expanded_overlay_1920x1080.png",
                                  "overlay_alpha_pixels": alpha_pixels(ov2)}
                sessions["default_world_inspector"] = dump2
                print(f"[ui-save] inspector: nodes={dump2.get('node_count')} selected={sel.get('selected_id')} "
                      f"stats_panels={dump2['state']['stats_panels']}")
        except Exception as exc:  # noqa: BLE001
            print("[ui-save] inspector capture failed:", exc)

        # publish separate artifact + merge sessions into the canonical runtime dump
        out = {"schema": "rustibites.assets.ui_runtime_save.v1",
               "generatedBy": "tools/assets/capture_ui_runtime_save.py",
               "capture": {"method": "hidden game, save loaded paused t=20 s, fit camera; ParityDump /d/ui",
                           "save": str(Path(args.save).absolute()), "screen": dump.get("screen")},
               "sessions": sessions}
        with gzip.open(Path(args.out), "wb", compresslevel=9) as g:
            g.write(json.dumps(out, indent=1).encode("utf-8"))
        rp = Path(args.runtime)
        if rp.exists():
            main = json.loads(gzip.open(rp, "rt", encoding="utf-8").read())
            main_sessions = main.setdefault("sessions", {})
            if "default_world" in main_sessions and "default_world_newgame" not in main_sessions:
                main_sessions["default_world_newgame"] = main_sessions.pop("default_world")
            main_sessions.update(sessions)
            main.setdefault("capture", {})["save_state"] = out["capture"]
            with gzip.open(rp, "wb", compresslevel=9) as g:
                g.write(json.dumps(main, indent=1).encode("utf-8"))
            (rp.with_name("ui_runtime.index.json")).write_text(json.dumps({
                "schema": "rustibites.assets.ui_runtime.index.v1",
                "files": {"runtime": "assets/game/ui_runtime.json.gz", "runtime_format": "gzipped JSON",
                          "runtime_schema": "rustibites.assets.ui_runtime.v1",
                          "save_state_artifact": "assets/game/ui_runtime_save.json.gz"},
                "capture": main.get("capture"),
                "sessions": {k: {"node_count": v.get("node_count"), "canvases": len(v.get("canvases") or [])}
                             for k, v in main_sessions.items()},
                "consumer": "tools/ui/ui_runtime_atlas.json.gz (default_world = save HUD, "
                            "default_world_inspector = selected Stats panel, default_world_newgame = old t=0.25 dump)",
            }, indent=1), encoding="utf-8")
        meta = {"schema": "rustibites.assets.ui_save_capture.v1",
                "sessions": {k: v.get("state") for k, v in sessions.items()},
                "ruler_default_world": sessions["default_world"]["state"]["ruler"],
                "runtime_sessions": list(sessions)}
        (OUTDIR / "ui_save_capture.json").write_text(json.dumps(meta, indent=2), encoding="utf-8")
        # reference.json HUD entries
        rj = OUTDIR / "reference.json"
        if rj.exists():
            ref = json.loads(rj.read_text(encoding="utf-8"))
            ref["hud_savestate"] = sessions["default_world"]["state"]
            if "default_world_inspector" in sessions:
                ref["hud_inspector_expanded"] = sessions["default_world_inspector"]["state"]
            rj.write_text(json.dumps(ref, indent=2), encoding="utf-8")
        print(f"[ui-save] wrote {args.out}; merged {list(sessions)} into {rp}; metadata in "
              f"{OUTDIR / 'ui_save_capture.json'}")
    finally:
        session.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""capture_inspector_tabs.py - Genes/Biology/Brain inspector runtime UI sessions (bench-dev data).

Hidden load of rust_seed_600.zip paused t=20, select the bibite at the canonical closeup position, open the
inspector via the real StatsButton, then press GeneButton/BiologyButton/BrainButton, let animations settle,
and dump runtime UI + UI-only overlays + composited 1920x1080 frames for each tab. Publishes sessions
`default_world_genes`, `default_world_biology`, `default_world_brain` in assets/game/ui_runtime.json.gz
(keeping default_world/default_world_inspector) and regenerates the atlas; refs under
assets/game/reference/default_save/inspector_tabs/. Hidden-only.
"""
from __future__ import annotations

import base64
import gzip
import io
import json
import sys
import time
from pathlib import Path

from PIL import Image

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import capture as cap  # noqa: E402

SAVE = ROOT / "testdata/parity/default/rust_seed_600.zip"
RUNTIME = ROOT / "assets/game/ui_runtime.json.gz"
RUNTIME_SAVE = ROOT / "assets/game/ui_runtime_save.json.gz"
OUT = ROOT / "assets/game/reference/default_save/inspector_tabs"
SELECT = (-5.360003, 831.0031)


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
        raise RuntimeError(ov)
    return base64.b64decode(ov["png_base64"])


def active_panels(dump):
    found = []

    def walk(n):
        p = n.get("path") or ""
        if n.get("active_in_hierarchy") and any(x in p for x in ("GenePanel", "BiologyPanel", "BrainPanel", "StatsPanel", "GenesPanel")):
            found.append(p)
        for k in n.get("children") or []:
            walk(k)

    for r in dump.get("canvases") or []:
        walk(r)
    return found


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    B, _, Lo, La = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    s = cap.WorldSession(build_args(__import__("argparse").Namespace()), B, Lo, La)
    try:
        c = s.open()
        if not s.plugin_ok:
            raise SystemExit("parity plugin not available (deploy ParityDump)")
        s.plugin.pacing(True)
        c.load_world(SAVE)
        c.set_time(paused=True)
        sel = post(s.plugin, "/d/select_bibite", {"x": SELECT[0], "y": SELECT[1]})
        c.set_time(paused=False)
        time.sleep(0.8)
        c.set_time(paused=True)
        post(s.plugin, "/d/press_button", {"name": "StatsButton"})
        c.set_time(paused=False)
        time.sleep(0.8)
        c.set_time(paused=True)
        pos = sel.get("pos") or {}
        cam = c.set_camera(x=float(pos.get("x") or SELECT[0]), y=float(pos.get("y") or SELECT[1]), size=400.0)
        sessions = {}
        meta_tabs = {}
        tabs = [("genes", "GeneButton", "default_world_genes"),
                ("biology", "BiologyButton", "default_world_biology"),
                ("brain", "BrainButton", "default_world_brain")]
        for tab, button, session in tabs:
            press = post(s.plugin, "/d/press_button", {"name": button})
            c.set_time(paused=False)
            time.sleep(0.9)
            c.set_time(paused=True)
            dump = s.plugin.ui_runtime()
            ov = overlay(s.plugin)
            ov_path = OUT / f"hud_inspector_{tab}_overlay_1920x1080.png"
            ov_path.write_bytes(ov)
            world = c.screenshot(1920, 1080)
            full = OUT / f"hud_inspector_{tab}_1920x1080.png"
            w = Image.open(io.BytesIO(world)).convert("RGBA")
            u = Image.open(io.BytesIO(ov)).convert("RGBA")
            if u.size != w.size:
                u = u.resize(w.size, Image.LANCZOS)
            Image.alpha_composite(w, u).save(full)
            alpha_px = sum(1 for a in u.getchannel("A").getdata() if a > 0)
            dump["state"] = {"save": str(SAVE), "paused": True, "tab": tab, "button": button,
                             "selected": sel, "press": press, "camera": cam,
                             "active_panels": active_panels(dump),
                             "overlay": str(ov_path.relative_to(ROOT).as_posix()),
                             "frame": str(full.relative_to(ROOT).as_posix()),
                             "overlay_alpha_pixels": alpha_px}
            sessions[session] = dump
            meta_tabs[tab] = dump["state"]
            print(f"[inspector] {tab}: nodes={dump.get('node_count')} panels={dump['state']['active_panels']} alpha={alpha_px}")
        # merge sessions preserving existing
        for p in (RUNTIME, RUNTIME_SAVE):
            if not p.exists():
                continue
            main = json.loads(gzip.open(p, "rt", encoding="utf-8").read())
            main.setdefault("sessions", {}).update(sessions)
            main.setdefault("capture", {})["inspector_tabs"] = {
                "method": "hidden rust_seed_600 paused t=20; selected bibite; StatsButton then Gene/Biology/Brain buttons; /d/ui + /d/ui_overlay + screenshot",
                "tabs": meta_tabs}
            with gzip.open(p, "wb", compresslevel=9) as g:
                g.write(json.dumps(main, indent=1).encode("utf-8"))
        (OUT / "reference.json").write_text(json.dumps({
            "schema": "rustibites.assets.inspector_tabs.v1",
            "sessions": {tab: session for tab, _, session in tabs},
            "tabs": meta_tabs,
            "note": "default_world/default_world_inspector kept; these are Genes/Biology/Brain inspector sessions with UI-only overlays and 1920x1080 composites."}, indent=2), encoding="utf-8")
        (OUT / "README.md").write_text(
            "# Inspector tab runtime UI\n\nHidden save rust_seed_600 paused t=20; selected bibite; StatsButton then "
            "GeneButton/BiologyButton/BrainButton with animations settled. Sessions `default_world_genes`, "
            "`default_world_biology`, `default_world_brain` in `assets/game/ui_runtime.json.gz` (and "
            "ui_runtime_save.json.gz); overlays/frames here; selected id/camera/active panels in reference.json.\n", encoding="utf-8")
        print(f"[inspector] wrote sessions {list(sessions)}")
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

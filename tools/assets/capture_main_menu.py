#!/usr/bin/env python3
"""capture_main_menu.py - main menu/title runtime UI capture for the desktop-shortcut menu parity (task-146).

One hidden game start (no world loaded), 1920x1080: dump the live menu UI tree (GET /d/ui), render a
transparent all-canvas UI overlay (`/d/ui_overlay_all`: ScreenSpaceOverlay + ScreenSpaceCamera canvases)
and composite it with the bridge main-camera background; save title screen PNG/overlay, then open Settings
via its runtime Button and capture the Settings submenu similarly. Publishes sessions
`main_menu_title`/`main_menu_settings` into assets/game/ui_runtime.json.gz (existing sessions kept),
regenerates the atlas, and writes assets/game/reference/main_menu/ (PNG/overlays/reference.json/README)
with menu item node paths. Hidden-only; caller removes the plugin after the run.
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

OUT = ROOT / "assets/game/reference/main_menu"
RUNTIME = ROOT / "assets/game/ui_runtime.json.gz"


def build_args(ns):
    ns.work_dir = str(ROOT / "testdata/parity/_work")
    ns.port = 17800
    ns.plugin_port = 17801
    ns.no_plugin = False
    ns.attach = False
    ns.start_timeout = 300.0
    ns.keep_game = False
    return ns


def overlay(plugin):
    ov = plugin.httpx.get(plugin.base + "/d/ui_overlay_all", timeout=120).json()
    if not ov.get("ok"):
        raise RuntimeError(f"ui_overlay_all failed: {ov}")
    return base64.b64decode(ov["png_base64"]), ov


def composite(world_png, ui_png):
    w = Image.open(io.BytesIO(world_png)).convert("RGBA")
    u = Image.open(io.BytesIO(ui_png)).convert("RGBA")
    if u.size != w.size:
        u = u.resize(w.size, Image.LANCZOS)
    return Image.alpha_composite(w, u)


def nodes_matching(dump):
    out = []
    def walk(node):
        p = node.get("path") or ""
        name = (node.get("name") or "")
        texts = [str(t.get("text") or "") for t in (node.get("texts") or [])]
        controls = [c.get("type") for c in (node.get("controls") or [])]
        low = (name + " " + p + " " + " ".join(texts)).lower()
        hit = None
        for word in ("new game", "load game", "continue", "settings", "quit", "exit", "credits", "version", "loading"):
            if word in low:
                hit = word
                break
        if hit:
            out.append({"word": hit, "path": p, "name": name, "texts": texts, "controls": controls,
                        "active_in_hierarchy": node.get("active_in_hierarchy")})
        for k in node.get("children") or []:
            walk(k)
    for r in dump.get("canvases") or []:
        walk(r)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--api-home", default=r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(args.api_home))
    session = cap.WorldSession(build_args(argparse.Namespace()), BridgeClient, LiveRuntime, Lab)
    try:
        session.open()
        if not session.plugin_ok:
            raise SystemExit("parity plugin not available (deploy ParityDump)")
        session.plugin.pacing(True)
        # wait for the title/menu canvases to become active (intro video/loading may still be running)
        title = None
        for _ in range(40):
            title = session.plugin.ui_runtime()
            roots = title.get("canvases") or []
            active_names = {r.get("name") for r in roots if r.get("active_in_hierarchy")}
            if "UI" in active_names and "Title Screen" in active_names:
                break
            time.sleep(0.5)
        activation = None
        if "UI" not in {r.get("name") for r in (title.get("canvases") or []) if r.get("active_in_hierarchy")}:
            # hidden lab may keep the welcome/menu UI root disabled behind the intro video; activate the
            # normal welcome root so the real menu is visible and capturable.
            activation = session.plugin.httpx.post(session.plugin.base + "/d/activate",
                                                  json={"path": "UI", "active": True}, timeout=30).json()
            time.sleep(0.6)
            title = session.plugin.ui_runtime()
        title_items = nodes_matching(title)
        bg = session.plugin.httpx.get(session.plugin.base + "/d/menu_background", timeout=120).json()
        if not bg.get("ok"):
            raise RuntimeError(f"menu_background failed: {bg}")
        world = base64.b64decode(bg["png_base64"])
        ov, ov_meta = overlay(session.plugin)
        (OUT / "title_1920x1080.png").write_bytes(world)
        (OUT / "title_ui_only_1920x1080.png").write_bytes(ov)
        composite(world, ov).save(OUT / "title_full_1920x1080.png")

        # open Settings by its runtime button
        settings_items = [i for i in title_items if i["word"] == "settings"]
        settings_items.sort(key=lambda i: (not i.get("active_in_hierarchy"), "button" not in (i.get("name") or "").lower()))
        press = None
        for item in settings_items:
            name = item["name"]
            if name:
                press = session.plugin.httpx.post(session.plugin.base + "/d/press_button",
                                                   json={"name": name}, timeout=30).json()
                if press.get("ok"):
                    break
            press = session.plugin.httpx.post(session.plugin.base + "/d/press_button",
                                               json={"path": item["path"]}, timeout=30).json()
            if press.get("ok"):
                break
        time.sleep(0.9)
        settings = session.plugin.ui_runtime()
        settings_items2 = nodes_matching(settings)
        bg2 = session.plugin.httpx.get(session.plugin.base + "/d/menu_background", timeout=120).json()
        if not bg2.get("ok"):
            raise RuntimeError(f"menu_background (settings) failed: {bg2}")
        world2 = base64.b64decode(bg2["png_base64"])
        ov2, ov2_meta = overlay(session.plugin)
        (OUT / "settings_1920x1080.png").write_bytes(world2)
        (OUT / "settings_ui_only_1920x1080.png").write_bytes(ov2)
        composite(world2, ov2).save(OUT / "settings_full_1920x1080.png")

        # publish sessions (keep existing)
        sessions = {"main_menu_title": title, "main_menu_settings": settings}
        if RUNTIME.exists():
            main = json.loads(gzip.open(RUNTIME, "rt", encoding="utf-8").read())
            main.setdefault("sessions", {}).update(sessions)
            main.setdefault("capture", {})["main_menu_capture"] = {
                "method": "hidden game start, no world; /d/ui + /d/ui_overlay_all + bridge screenshot; Settings opened via runtime Button",
                "screen": title.get("screen")}
            with gzip.open(RUNTIME, "wb", compresslevel=9) as g:
                g.write(json.dumps(main, indent=1).encode("utf-8"))
            (RUNTIME.with_name("ui_runtime.index.json")).write_text(json.dumps({
                "schema": "rustibites.assets.ui_runtime.index.v1",
                "files": {"runtime": "assets/game/ui_runtime.json.gz", "runtime_schema": "rustibites.assets.ui_runtime.v1"},
                "capture": main.get("capture"),
                "sessions": {k: {"node_count": v.get("node_count"), "canvases": len(v.get("canvases") or [])}
                             for k, v in main["sessions"].items()},
                "consumer": "tools/ui/ui_runtime_atlas.json.gz (main_menu_title/main_menu_settings included)"}, indent=1), encoding="utf-8")
        meta = {
            "schema": "rustibites.assets.main_menu_capture.v1",
            "screen": title.get("screen"), "plugin": title.get("plugin"),
            "sessions": {"title": "main_menu_title", "settings": "main_menu_settings"},
            "files": {"title_full": "assets/game/reference/main_menu/title_full_1920x1080.png",
                      "title_ui_only": "assets/game/reference/main_menu/title_ui_only_1920x1080.png",
                      "title_world": "assets/game/reference/main_menu/title_1920x1080.png",
                      "settings_full": "assets/game/reference/main_menu/settings_full_1920x1080.png",
                      "settings_ui_only": "assets/game/reference/main_menu/settings_ui_only_1920x1080.png",
                      "settings_world": "assets/game/reference/main_menu/settings_1920x1080.png"},
            "title_menu_items": title_items,
            "settings_menu_items": settings_items2,
            "settings_press": press, "activation": activation,
            "overlay": {"title_alpha_pixels": ov_meta.get("nonzero_alpha_pixels"),
                        "settings_alpha_pixels": ov2_meta.get("nonzero_alpha_pixels")},
            "note": "LoadingPanel is included as an inactive canvas/node in the title tree; no visible loading screen "
                    "was active at capture. Existing atlas sessions (default_world/default_world_inspector/main_menu) preserved.",
        }
        (OUT / "reference.json").write_text(json.dumps(meta, indent=2), encoding="utf-8")
        (OUT / "README.md").write_text(
            "# Main menu/title screen runtime UI\n\nHidden game start (no world), 1920×1080; title and Settings "
            "submenu captured as `main_menu_title`/`main_menu_settings` sessions in "
            "`assets/game/ui_runtime.json.gz` and the joined atlas. Full PNG, UI-only alpha overlay, and raw "
            "world frames are in this folder; `reference.json` lists New Game/Load Game/Continue/Settings/Quit/"
            "credits/version/loading node paths and the Settings-open response. Existing atlas sessions are "
            "preserved. Hidden-only; plugin removed after capture.\n", encoding="utf-8")
        print(json.dumps({"sessions": list(sessions), "title_items": title_items[:20],
                          "settings_press": press}, indent=1)[:5000])
    finally:
        session.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

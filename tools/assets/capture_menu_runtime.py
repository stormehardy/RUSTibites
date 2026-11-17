#!/usr/bin/env python3
"""capture_menu_runtime.py - main-menu runtime UI tree + loading-screen frame (menu capture attempt).

Hidden game start (no world). The BibitesAPI lab freezes the menu/intro at its loading frame on this machine
(loading progress ~9%, no further frames even with Time.timeScale=1), so a live rendered menu screenshot is
not obtainable headlessly. Still capture the authoritative menu UI tree: activate the normal `UI` root,
normalize CanvasGroups, then dump `main_menu_title`; activate `UI/SettingsPanel` and dump
`main_menu_settings`. Also capture the actual screen frame (loading screen) and record the runtime menu item
node paths. Publishes sessions into ui_runtime.json.gz and writes assets/game/reference/main_menu/reference.json.
"""
from __future__ import annotations

import base64
import gzip
import json
import sys
import time
from pathlib import Path

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


def node_items(dump):
    out = []

    def walk(n):
        p = n.get("path") or ""
        name = n.get("name") or ""
        texts = [str(t.get("text") or "") for t in (n.get("texts") or [])]
        low = (name + " " + p + " " + " ".join(texts)).lower()
        for word in ("new game", "load game", "continue", "settings", "quit", "exit", "credits", "version", "loading"):
            if word in low:
                out.append({"word": word, "name": name, "path": p, "texts": texts,
                            "controls": [c.get("type") for c in (n.get("controls") or [])],
                            "active_in_hierarchy": n.get("active_in_hierarchy")})
                break
        for k in n.get("children") or []:
            walk(k)

    for r in dump.get("canvases") or []:
        walk(r)
    return out


def screenshot_b64(plugin):
    r = plugin.httpx.get(plugin.base + "/d/screen_capture", timeout=120).json()
    if not r.get("ok"):
        raise RuntimeError(r)
    return base64.b64decode(r["png_base64"]), r


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    B, _, Lo, La = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    s = cap.WorldSession(build_args(__import__("argparse").Namespace()), B, Lo, La)
    try:
        s.open()
        s.plugin.pacing(True)
        time.sleep(2)
        act_ui = s.plugin.httpx.post(s.plugin.base + "/d/activate", json={"path": "UI", "active": True}, timeout=30).json()
        norm = s.plugin.httpx.post(s.plugin.base + "/d/normalize_ui", json={"root": "UI", "alpha": 1}, timeout=30).json()
        time.sleep(0.5)
        title = s.plugin.ui_runtime()
        loading_png, cap_resp = screenshot_b64(s.plugin)
        (OUT / "loading_screen_1920x1080.png").write_bytes(loading_png)
        act_settings = s.plugin.httpx.post(s.plugin.base + "/d/activate",
                                           json={"path": "UI/SettingsPanel", "active": True}, timeout=30).json()
        time.sleep(0.5)
        settings = s.plugin.ui_runtime()
        settings_png, _ = screenshot_b64(s.plugin)
        (OUT / "settings_screen_1920x1080.png").write_bytes(settings_png)
        # publish sessions
        sessions = {"main_menu_title": title, "main_menu_settings": settings}
        if RUNTIME.exists():
            main = json.loads(gzip.open(RUNTIME, "rt", encoding="utf-8").read())
            main.setdefault("sessions", {}).update(sessions)
            main.setdefault("capture", {})["main_menu_capture"] = {
                "method": "hidden game start; UI root activated/normalized; /d/ui; SettingsPanel activated; "
                          "screen capture (menu loading frame frozen in hidden lab)",
                "screen": title.get("screen")}
            with gzip.open(RUNTIME, "wb", compresslevel=9) as g:
                g.write(json.dumps(main, indent=1).encode("utf-8"))
            (RUNTIME.with_name("ui_runtime.index.json")).write_text(json.dumps({
                "schema": "rustibites.assets.ui_runtime.index.v1",
                "files": {"runtime": "assets/game/ui_runtime.json.gz"},
                "sessions": {k: {"node_count": v.get("node_count")} for k, v in main["sessions"].items()},
                "consumer": "tools/ui/ui_runtime_atlas.json.gz"}, indent=1), encoding="utf-8")
        title_items = node_items(title)
        settings_items = node_items(settings)
        meta = {"schema": "rustibites.assets.main_menu_capture.v1",
                "screen": title.get("screen"),
                "sessions": {"title": "main_menu_title", "settings": "main_menu_settings"},
                "files": {"loading_screen": "assets/game/reference/main_menu/loading_screen_1920x1080.png",
                          "settings_screen": "assets/game/reference/main_menu/settings_screen_1920x1080.png"},
                "activation": {"ui": act_ui, "normalize": norm, "settings": act_settings},
                "title_menu_items": title_items, "settings_menu_items": settings_items,
                "limitation": "BibitesAPI hidden menu froze at the loading frame (~9%) in this environment: "
                              "ScreenCapture shows the loading screen, Canvas/ScreenSpaceOverlay UI does not "
                              "advance to a rendered menu. The runtime UI tree/session above is authoritative "
                              "for menu item paths/layout; a true rendered menu screenshot requires a visible "
                              "window or a menu-scene capture path.",
                "screen_capture_response": cap_resp}
        (OUT / "reference.json").write_text(json.dumps(meta, indent=2), encoding="utf-8")
        (OUT / "README.md").write_text(
            "# Main menu runtime UI (title/settings)\n\nSessions `main_menu_title` and `main_menu_settings` "
            "in `assets/game/ui_runtime.json.gz` / joined atlas; `reference.json` lists New Game/Load Game/"
            "Continue/Settings/Quit/Exit/Credits/Version node paths. The hidden lab freezes the menu at its "
            "loading frame, so `loading_screen_1920x1080.png` is the actual captured frame; the UI tree is "
            "authoritative for layout/menu nodes. Existing atlas sessions preserved.\n", encoding="utf-8")
        print(json.dumps({"loading_bytes": len(loading_png), "title_items": title_items[:20],
                          "settings_items": settings_items[:10], "limitation": meta["limitation"]}, indent=1)[:4000])
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

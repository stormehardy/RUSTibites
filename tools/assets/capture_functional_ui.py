#!/usr/bin/env python3
"""capture_functional_ui.py - functional/UI parity atlas capture for menus/in-game panels.

Hidden menu pass: activate/normalize the Welcome UI, then activate and dump each menu/panel root
(Load, Settings + Graphics/Gameplay/Hotkeys, New Game/Scenario, Bibite Library/Editor, Achievements,
Tools, Credits, Workshop, Scenario Editor, New panel), with overlay/screenshot attempts and control labels.

Hidden world pass: load rust_seed_600 paused, press every active toolbar/panel button found by name/path
(Placer, UserActions, View, DynamicParameters, AncestralLineage, Information, Settings), dump UI + overlay
+ screenshot after each. New atlas sessions are merged into assets/game/ui_runtime.json.gz (existing kept);
a label->node-path manifest and refs are written under assets/game/reference/functional_ui/.
"""
from __future__ import annotations

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
OUT = ROOT / "assets/game/reference/functional_ui"


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
    r = plugin.httpx.get(plugin.base + "/d/ui_overlay", timeout=120).json()
    return base64.b64decode(r["png_base64"]) if r.get("ok") else b""


def controls(dump):
    """Flatten Buttons/Toggles/Dropdowns with nearest labels + state for the manifest."""
    rows = []

    def texts(n):
        out = []
        for c in n.get("components") or []:
            if c.get("type") == "TMPro.TextMeshProUGUI" and c.get("text"):
                out.append(str(c["text"]))
        return out

    def walk(n):
        comps = n.get("components") or []
        ctrls = [c for c in comps if c.get("type") in (
            "UnityEngine.UI.Button", "UnityEngine.UI.Toggle", "TMPro.TMP_Dropdown",
            "UnityEngine.UI.Slider", "TMPro.TMP_InputField")]
        if ctrls:
            rows.append({"path": n.get("path"), "name": n.get("name"),
                         "active": n.get("active_in_hierarchy"),
                         "labels": texts(n)[:8],
                         "controls": [{"type": c.get("type"), "enabled": c.get("enabled"),
                                       "interactable": c.get("interactable"), "is_on": c.get("is_on"),
                                       "value": c.get("value")} for c in ctrls]})
        for k in n.get("children") or []:
            walk(k)

    for r in dump.get("canvases") or []:
        walk(r)
    return rows


def save_session(name, dump, extra=None):
    safe = re.sub(r"[^A-Za-z0-9_.-]+", "_", name)
    ov = overlay(s.plugin)
    ovp = OUT / f"{safe}_overlay_1920x1080.png"
    if ov:
        ovp.write_bytes(ov)
    world_path = OUT / f"{safe}_world_1920x1080.png"
    try:
        world = s.client.screenshot(1920, 1080)
        world_path.write_bytes(world)
    except Exception as exc:  # no world camera in the menu
        world = None
        world_path = None
        world_error = str(exc)
        if ov:
            (OUT / f"{safe}_full_1920x1080.png").write_bytes(ov)
    if world is not None and ov:
        w = Image.open(io.BytesIO(world)).convert("RGBA")
        u = Image.open(io.BytesIO(ov)).convert("RGBA")
        if u.size != w.size:
            u = u.resize(w.size, Image.LANCZOS)
        Image.alpha_composite(w, u).save(OUT / f"{safe}_full_1920x1080.png")
    st = {"tab": name, "overlay": str(ovp.relative_to(ROOT).as_posix()) if ov else None,
          "world": str(world_path.relative_to(ROOT).as_posix()) if world_path else None,
          "world_error": world_error if world is None else None,
          "controls": controls(dump),
          "active_panels": sorted({n.get("path") for n in (dump.get("canvases") or []) if n.get("active_in_hierarchy")})}
    if extra:
        st.update(extra)
    dump["state"] = st
    sessions[name] = dump
    print(f"[ui] {name}: nodes={dump.get('node_count')} controls={len(st['controls'])} overlay={len(ov)}")
    return st


sessions = {}


def main():
    global s
    OUT.mkdir(parents=True, exist_ok=True)
    B, _, Lo, La = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    s = cap.WorldSession(build_args(__import__("argparse").Namespace()), B, Lo, La)
    try:
        s.open()
        if not s.plugin_ok:
            raise SystemExit("parity plugin not available")
        s.plugin.pacing(True)
        time.sleep(1)
        # ---------- menu ----------
        post(s.plugin, "/d/activate", {"path": "UI", "active": True})
        post(s.plugin, "/d/normalize_ui", {"root": "UI", "alpha": 1})
        time.sleep(0.4)
        title = s.plugin.ui_runtime()
        save_session("main_menu_functional_title", title)
        panels = [
            ("main_menu_load_game", "UI/LoadGamePanel"),
            ("main_menu_settings", "UI/SettingsPanel"),
            ("main_menu_new_game", "UI/NewGameScenarios"),
            ("main_menu_bibite_library", "UI/BibiteSelector"),
            ("main_menu_achievements", "UI/WelcomeScreen/AchievementsPanel"),
            ("main_menu_additional_tools", "UI/WelcomeScreen/ToolsPanel"),
            ("main_menu_credits", "UI/WelcomeScreen/CreditsPanel"),
            ("main_menu_new_panel", "UI/NewPanel"),
            ("main_menu_workshop", "UI/WorkshopItemsPanel"),
            ("main_menu_scenario_editor", "UI/ScenarioEditor"),
        ]
        for name, path in panels:
            post(s.plugin, "/d/activate", {"path": path, "active": True})
            time.sleep(0.35)
            dump = s.plugin.ui_runtime()
            save_session(name, dump, {"activated_path": path})
        # settings tabs
        post(s.plugin, "/d/activate", {"path": "UI/SettingsPanel", "active": True})
        for name, button in [("main_menu_settings_graphics", "GraphicsButton"),
                             ("main_menu_settings_gameplay", "Gameplay Button"),
                             ("main_menu_settings_hotkeys", "Controls Button")]:
            post(s.plugin, "/d/press_button", {"name": button})
            time.sleep(0.3)
            save_session(name, s.plugin.ui_runtime(), {"button": button})
        # bibite editor tabs
        for name, button in [("main_menu_bibite_editor_genes", "GenesButton"),
                             ("main_menu_bibite_editor_brain", "brainButton")]:
            post(s.plugin, "/d/activate", {"path": "UI/BibiteSelector/BibiteEditor", "active": True})
            post(s.plugin, "/d/press_button", {"name": button})
            time.sleep(0.3)
            save_session(name, s.plugin.ui_runtime(), {"button": button})
        # ---------- in-world toolbar ----------
        s.client.load_world(SAVE)
        s.client.set_time(paused=True)
        c0 = s.plugin.ui_runtime()
        save_session("main_world_toolbar", c0)
        for name, button in [
            ("main_world_tool_placer", "PlacerButton"),
            ("main_world_tool_user_actions", "UserActions"),
            ("main_world_tool_view", "ViewButton"),
            ("main_world_dynamic_parameters", "DynamicParametersButton"),
            ("main_world_lineage", "AncestralLineageButton"),
            ("main_world_information", "InformationPanelButton"),
            ("main_world_settings_panel", "SettingsButton"),
        ]:
            pr = post(s.plugin, "/d/press_button", {"name": button})
            time.sleep(0.35)
            save_session(name, s.plugin.ui_runtime(), {"button": button, "press": pr})
        # ---------- publish sessions + manifests ----------
        main = json.loads(gzip.open(RUNTIME, "rt", encoding="utf-8").read())
        main.setdefault("sessions", {}).update(sessions)
        main.setdefault("capture", {})["functional_ui"] = {
            "method": "hidden menu + rust_seed_600 world; activate menu roots / press toolbar buttons; /d/ui + "
                      "/d/ui_overlay + 1920x1080 screenshots", "sessions": list(sessions)}
        with gzip.open(RUNTIME, "wb", compresslevel=9) as g:
            g.write(json.dumps(main, indent=1).encode("utf-8"))
        manifest = {"schema": "rustibites.assets.functional_ui.v1",
                    "sessions": {k: {"nodes": v.get("node_count"), "controls": len((v.get("state") or {}).get("controls") or []),
                                     "overlay": (v.get("state") or {}).get("overlay")} for k, v in sessions.items()},
                    "controls": {k: (v.get("state") or {}).get("controls") for k, v in sessions.items()},
                    "note": "Menu roots activated in the hidden Welcome UI; in-world panels opened via toolbar "
                            "buttons. Overlay may be empty for ScreenSpaceOverlay menu canvases in hidden mode; "
                            "UI tree/control paths are authoritative."}
        (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
        (OUT / "README.md").write_text(
            "# Functional/UI parity capture\n\nMenu sessions activate the Welcome UI roots for Load, Settings "
            "(Graphics/Gameplay/Hotkeys), New Game/Scenario, Bibite Library/Editor (Genes/Brain), Achievements, "
            "Additional Tools, Credits, Workshop, Scenario Editor; in-world sessions press the toolbar buttons "
            "(Placer/UserActions/View/DynamicParameters/AncestralLineage/Information/Settings). `manifest.json` "
            "maps each session to node paths/labels/control state; screenshots/overlays are per session. Existing "
            "atlas sessions preserved.\n", encoding="utf-8")
        print(f"[ui] wrote {len(sessions)} sessions, manifest, README")
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

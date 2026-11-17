#!/usr/bin/env python3
"""capture_ui_control_behavior.py - settings/save-load/tool/inspector control ground truth.

Adds a focused behavioral extension for the buttons being wired: settings Toggle/Slider/Dropdown before→after,
KILL confirmation full texts/buttons, save/load browser panels + slot/thumbnail state, placer tool property
controls, and inspector Lay Egg/tag state. Hidden-only; writes control_behavior.json + refs.
"""
from __future__ import annotations

import json
import sys
import time
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import capture as cap  # noqa: E402

SAVE = ROOT / "testdata/parity/default/rust_seed_600.zip"
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


def nodes(dump):
    out = []

    def walk(n):
        out.append(n)
        for k in n.get("children") or []:
            walk(k)

    for r in dump.get("canvases") or []:
        walk(r)
    return out


def active_info(dump, prefix=None):
    ns = nodes(dump)
    texts, buttons, images = [], [], []
    for n in ns:
        if not n.get("active_in_hierarchy"):
            continue
        p = n.get("path", "")
        if prefix and not p.startswith(prefix):
            continue
        for c in n.get("components") or []:
            t = c.get("type")
            if t == "TMPro.TextMeshProUGUI" and c.get("text"):
                texts.append(str(c["text"]))
            elif t == "UnityEngine.UI.Button":
                labels = []
                for cc in (n.get("components") or []):
                    if cc.get("type") == "TMPro.TextMeshProUGUI" and cc.get("text"):
                        labels.append(str(cc["text"]))
                buttons.append({"path": p, "name": n.get("name"), "enabled": c.get("enabled"), "labels": labels[:3]})
            elif t in ("UnityEngine.UI.Image", "UnityEngine.UI.RawImage"):
                spr = c.get("sprite")
                if isinstance(spr, dict) and spr.get("name"):
                    images.append({"path": p, "sprite": spr.get("name")})
    return {"texts": texts[:200], "buttons": buttons[:200], "images": images[:100]}


def control_dump(plugin):
    return plugin.httpx.get(plugin.base + "/d/control_dump", timeout=60).json()


def main():
    global s
    B, _, Lo, La = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    s = cap.WorldSession(build_args(__import__("argparse").Namespace()), B, Lo, La)
    out = {"schema": "rustibites.assets.functional_ui.control_behavior.v1", "sections": {}}
    try:
        s.open()
        s.plugin.pacing(True)
        time.sleep(0.8)
        # ---------- settings controls ----------
        post(s.plugin, "/d/activate", {"path": "UI", "active": True})
        post(s.plugin, "/d/normalize_ui", {"root": "UI", "alpha": 1})
        post(s.plugin, "/d/press_label", {"label": "Settings"})
        time.sleep(0.4)
        before = control_dump(s.plugin)
        settings_before = active_info(s.plugin.ui_runtime(), "UI/SettingsPanel")
        actions = []
        for c in (before.get("controls") or [])[:8]:
            kind, path = c.get("kind"), c.get("path")
            if not path:
                continue
            old = c.get("is_on") if kind == "Toggle" else c.get("value")
            if kind == "Toggle":
                newv = not bool(old)
                hit = post(s.plugin, "/d/control_set", {"kind": "Toggle", "path": path, "value": newv})
            elif kind == "Slider":
                newv = 0.5 if not isinstance(old, (int, float)) or old < 0.5 else 0.1
                hit = post(s.plugin, "/d/control_set", {"kind": "Slider", "path": path, "value": newv})
            elif kind == "Dropdown":
                opts = c.get("options") or []
                newv = 1 if len(opts) > 1 and old == 0 else 0
                hit = post(s.plugin, "/d/control_set", {"kind": "Dropdown", "path": path, "value": newv})
            else:
                continue
            time.sleep(0.2)
            after_dump = control_dump(s.plugin)
            newc = next((x for x in after_dump.get("controls") or [] if x.get("path") == path), {})
            actions.append({"kind": kind, "path": path, "labels": c.get("labels") or newc.get("labels"),
                            "options_before": c.get("options"), "options_after": newc.get("options"),
                            "before": old,
                            "after": newc.get("is_on") if kind == "Toggle" else newc.get("value"),
                            "hit": hit})
        out["sections"]["settings_controls"] = {"before": settings_before,
                                               "controls": settings_before.get("texts"),
                                               "actions": actions,
                                               "after": active_info(s.plugin.ui_runtime(), "UI/SettingsPanel")}
        # ---------- save/load browsers ----------
        s.client.load_world(SAVE)
        s.client.set_time(paused=True)
        browsers = {}
        for name, path in [("load", "UICanvas/MainUI/TopLayer/LoadGamePanel"),
                           ("save", "UICanvas/MainUI/TopLayer/Save Game Panel")]:
            post(s.plugin, "/d/activate", {"path": path, "active": True})
            time.sleep(0.3)
            browsers[name] = active_info(s.plugin.ui_runtime(), path)
            browsers[name]["controls"] = control_dump(s.plugin).get("controls")
        out["sections"]["save_load_browsers"] = browsers
        # ---------- KILL dialog full text/buttons ----------
        post(s.plugin, "/d/select_bibite", {"x": -5.360003, "y": 831.0031})
        post(s.plugin, "/d/press_button", {"name": "StatsButton"})
        time.sleep(0.3)
        before_stats = active_info(s.plugin.ui_runtime())
        post(s.plugin, "/d/press_label", {"label": "Kill"})
        time.sleep(0.3)
        dump = s.plugin.ui_runtime()
        out["sections"]["kill_dialog"] = {"before": before_stats, "after": active_info(dump),
                                          "active_controls": control_dump(s.plugin).get("controls")}
        # ---------- placer tool property controls ----------
        post(s.plugin, "/d/press_button", {"name": "PlacerButton"})
        post(s.plugin, "/d/press_label", {"label": "PlacePellets"})
        time.sleep(0.3)
        out["sections"]["placer_properties"] = {"panels": active_info(s.plugin.ui_runtime(), "UICanvas/MainUI/Layer1/Panels/PlacerPanels"),
                                                "controls": control_dump(s.plugin).get("controls")}
        # ---------- inspector lay/tag ----------
        post(s.plugin, "/d/select_bibite", {"x": -5.360003, "y": 831.0031})
        post(s.plugin, "/d/press_button", {"name": "StatsButton"})
        time.sleep(0.3)
        st0 = s.client.status()
        lay = post(s.plugin, "/d/press_label", {"label": "Lay Egg"})
        time.sleep(0.3)
        st1 = s.client.status()
        out["sections"]["inspector_lay"] = {"before": st0.get("counts"), "after": st1.get("counts"), "hit": lay,
                                           "ui": active_info(s.plugin.ui_runtime())}
        out["sections"]["inspector_tag"] = {"controls": control_dump(s.plugin).get("controls"),
                                           "ui": active_info(s.plugin.ui_runtime())}
        (OUT / "control_behavior.json").write_text(json.dumps(out, indent=2), encoding="utf-8")
        manifest = json.loads((OUT / "manifest.json").read_text(encoding="utf-8"))
        manifest["control_behavior"] = "assets/game/reference/functional_ui/control_behavior.json"
        (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
        print(json.dumps({"sections": list(out["sections"]),
                          "settings_actions": len(out["sections"]["settings_controls"]["actions"]),
                          "kill_buttons": len(out["sections"]["kill_dialog"]["after"]["buttons"]),
                          "load_texts": len(out["sections"]["save_load_browsers"]["load"]["texts"]),
                          "save_texts": len(out["sections"]["save_load_browsers"]["save"]["texts"])}, indent=1))
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

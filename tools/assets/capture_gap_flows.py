#!/usr/bin/env python3
"""capture_gap_flows.py - resolve editor/save-library/newgame ground-truth gaps by node path."""
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


def info(dump):
    texts, buttons, panels = [], [], []
    for n in nodes(dump):
        if not n.get("active_in_hierarchy"):
            continue
        p = n.get("path", "")
        if n.get("name", "").endswith(("Panel", "PopUp", "Pop-up", "Selector", "Editor")):
            panels.append(p)
        for c in n.get("components") or []:
            t = c.get("type")
            if t == "TMPro.TextMeshProUGUI" and c.get("text"):
                texts.append(str(c["text"]))
            elif t == "UnityEngine.UI.Button":
                labels = [str(cc.get("text")) for cc in (n.get("components") or [])
                          if cc.get("type") == "TMPro.TextMeshProUGUI" and cc.get("text")]
                buttons.append({"path": p, "name": n.get("name"), "enabled": c.get("enabled"), "labels": labels})
    return {"texts": texts[:200], "buttons": buttons[:200], "panels": panels[:80]}


def controls(plugin):
    return plugin.httpx.get(plugin.base + "/d/control_dump", timeout=60).json().get("controls") or []


def main():
    global s
    B, _, Lo, La = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    s = cap.WorldSession(build_args(__import__("argparse").Namespace()), B, Lo, La)
    out = {"schema": "rustibites.assets.functional_ui.gap_flows.v1", "steps": []}
    try:
        s.open(); s.plugin.pacing(True); time.sleep(0.8)
        post(s.plugin, "/d/activate", {"path": "UI", "active": True})
        post(s.plugin, "/d/normalize_ui", {"root": "UI", "alpha": 1})
        # library by path
        ed = post(s.plugin, "/d/press_button", {"path": "UI/WelcomeScreen/Main Holder/ButtonsPanel/Editor"})
        time.sleep(0.3)
        create = post(s.plugin, "/d/press_button", {"path": "UI/BibiteSelector/SelectionPanel/Holder/Panels/LeftSide Buttons/CreateNew"})
        time.sleep(0.3)
        out["steps"].append({"step": "library_editor_by_path", "editor": ed, "create_new": create, "ui": info(s.plugin.ui_runtime())})
        # open editor / genes and edit sliders
        edit = post(s.plugin, "/d/press_button", {"path": "UI/BibiteSelector/SelectionPanel/Holder/Preview/Right Side Buttons/Edit Button"})
        post(s.plugin, "/d/activate", {"path": "UI/BibiteSelector/BibiteEditor", "active": True})
        post(s.plugin, "/d/press_button", {"path": "UI/BibiteSelector/BibiteEditor/Panel/Buttons/GenesButton"})
        time.sleep(0.3)
        g0 = controls(s.plugin)
        changed = []
        for c in [x for x in g0 if x.get("kind") == "Slider"][:5]:
            old = c.get("value")
            new = 0.25 if not isinstance(old, (int, float)) or old > 0.25 else 0.75
            h = post(s.plugin, "/d/control_set", {"kind": "Slider", "path": c.get("path"), "value": new})
            changed.append({"path": c.get("path"), "old": old, "new": new, "hit": h})
        time.sleep(0.2)
        g1 = controls(s.plugin)
        save_cont = post(s.plugin, "/d/press_button", {"path": "UI/BibiteSelector/BibiteEditor/Panel/Buttons/SaveAndContinue"})
        time.sleep(0.3)
        submit = post(s.plugin, "/d/press_button", {"path": "UI/BibiteSelector/BibiteEditor/Panel/Buttons/Submit"})
        time.sleep(0.3)
        out["steps"].append({"step": "editor_genes_edit_accept", "edit": edit, "slider_changes": changed,
                             "before_controls": g0, "after_controls": g1,
                             "save_continue": save_cont, "submit": submit, "ui": info(s.plugin.ui_runtime())})
        # world: save/load browser, create a user save
        s.client.load_world(SAVE); s.client.set_time(paused=True)
        for path in ("UICanvas/MainUI/TopLayer/Save Game Panel", "UICanvas/MainUI/TopLayer/LoadGamePanel"):
            post(s.plugin, "/d/activate", {"path": path, "active": True})
        ctrl = controls(s.plugin)
        inp = next((c for c in ctrl if c.get("kind") == "Input"), None)
        setname = post(s.plugin, "/d/control_set", {"kind": "Input", "path": inp.get("path"), "value": "parity_ui_test"}) if inp else None
        savebtn = post(s.plugin, "/d/press_button", {"path": "UICanvas/MainUI/TopLayer/Save Game Panel/SelectedPanel/SaveInfo/Save Button Holder/SaveButton"})
        time.sleep(0.6)
        after_save = info(s.plugin.ui_runtime())
        # open load panel again and capture slot list
        post(s.plugin, "/d/activate", {"path": "UICanvas/MainUI/TopLayer/LoadGamePanel", "active": True})
        time.sleep(0.3)
        load_info = info(s.plugin.ui_runtime())
        out["steps"].append({"step": "save_create_load_browser", "controls": ctrl, "input_set": setname,
                             "save_press": savebtn, "save_ui": after_save, "load_ui": load_info,
                             "load_controls": controls(s.plugin)})
        # new game dropdown interactions
        post(s.plugin, "/d/activate", {"path": "UI/NewGameScenarios", "active": True})
        time.sleep(0.3)
        ng0 = controls(s.plugin)
        dd_changes = []
        for c in [x for x in ng0 if x.get("kind") == "Dropdown"][:5]:
            opts = c.get("options") or []
            new = 1 if len(opts) > 1 and c.get("value") == 0 else 0
            dd_changes.append({"path": c.get("path"), "options": opts, "old": c.get("value"), "new": new,
                               "hit": post(s.plugin, "/d/control_set", {"kind": "Dropdown", "path": c.get("path"), "value": new})})
        time.sleep(0.2)
        out["steps"].append({"step": "newgame_dropdowns", "before": ng0, "changes": dd_changes,
                             "after": controls(s.plugin), "ui": info(s.plugin.ui_runtime())})
        (OUT / "gap_flows.json").write_text(json.dumps(out, indent=2), encoding="utf-8")
        print(json.dumps({"steps": [s["step"] for s in out["steps"]], "save": savebtn,
                          "input_set": setname, "load_texts": load_info["texts"][:22]}, indent=1)[:5000])
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

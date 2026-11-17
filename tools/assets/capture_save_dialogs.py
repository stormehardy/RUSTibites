#!/usr/bin/env python3
"""capture_save_dialogs.py - Load/Save browser slot + overwrite/delete dialogs after creating a user save."""
from __future__ import annotations

import json
import sys
import time
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import capture as cap  # noqa: E402

OUT = ROOT / "assets/game/reference/functional_ui"
SAVEFILE = Path(r"C:\Users\Storme\AppData\LocalLow\The Bibites\The Bibites\Savefiles\parity_ui_test.zip")


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
    texts, buttons = [], []
    for n in nodes(dump):
        if not n.get("active_in_hierarchy"):
            continue
        for c in n.get("components") or []:
            if c.get("type") == "TMPro.TextMeshProUGUI" and c.get("text"):
                texts.append(str(c["text"]))
            elif c.get("type") == "UnityEngine.UI.Button":
                labels = [str(cc.get("text")) for cc in (n.get("components") or [])
                          if cc.get("type") == "TMPro.TextMeshProUGUI" and cc.get("text")]
                buttons.append({"path": n.get("path"), "name": n.get("name"), "labels": labels})
    return {"texts": texts[:250], "buttons": buttons[:200]}


def controls(plugin):
    return plugin.httpx.get(plugin.base + "/d/control_dump", timeout=60).json().get("controls") or []


def main():
    global s
    B, _, Lo, La = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    s = cap.WorldSession(build_args(__import__("argparse").Namespace()), B, Lo, La)
    out = {"schema": "rustibites.assets.functional_ui.save_dialogs.v1", "savefile": str(SAVEFILE), "steps": []}
    try:
        s.open(); s.plugin.pacing(True); time.sleep(0.8)
        post(s.plugin, "/d/activate", {"path": "UI", "active": True})
        post(s.plugin, "/d/normalize_ui", {"root": "UI", "alpha": 1})
        post(s.plugin, "/d/activate", {"path": "UI/LoadGamePanel", "active": True})
        time.sleep(0.4)
        d = s.plugin.ui_runtime()
        out["steps"].append({"step": "load_panel_slot_list", "ui": info(d), "controls": controls(s.plugin)})
        # delete dialog
        dele = post(s.plugin, "/d/press_button", {"path": "UI/LoadGamePanel/SelectedPanel/SaveInfo/DeleteButton"})
        if not (dele or {}).get("ok"):
            dele = post(s.plugin, "/d/press_button", {"name": "DeleteButton"})
        time.sleep(0.3)
        out["steps"].append({"step": "load_delete_dialog", "hit": dele, "ui": info(s.plugin.ui_runtime())})
        # save overwrite attempt
        post(s.plugin, "/d/activate", {"path": "UI/Save Game Panel", "active": True})
        ctrl = controls(s.plugin)
        inp = next((c for c in ctrl if c.get("kind") == "Input"), None)
        setn = post(s.plugin, "/d/control_set", {"kind": "Input", "path": inp.get("path"), "value": "parity_ui_test"}) if inp else None
        saveh = post(s.plugin, "/d/press_button", {"path": "UI/Save Game Panel/SelectedPanel/SaveInfo/Save Button Holder/SaveButton"})
        if not (saveh or {}).get("ok"):
            saveh = post(s.plugin, "/d/press_button", {"name": "SaveButton"})
        time.sleep(0.4)
        out["steps"].append({"step": "save_overwrite_attempt", "input_set": setn, "save": saveh,
                             "ui": info(s.plugin.ui_runtime()), "controls": controls(s.plugin)})
        (OUT / "save_dialogs.json").write_text(json.dumps(out, indent=2), encoding="utf-8")
        print(json.dumps({"steps": [x["step"] for x in out["steps"]],
                          "load_texts": out["steps"][0]["ui"]["texts"][:30],
                          "delete": [x for x in out["steps"] if x["step"] == "load_delete_dialog"][0]["ui"]["texts"][:30],
                          "overwrite": [x for x in out["steps"] if x["step"] == "save_overwrite_attempt"][0]["ui"]["texts"][:30]}, indent=1)[:5000])
    finally:
        s.close()
        # clean up the test save (created only for this ground truth)
        try:
            if SAVEFILE.exists():
                SAVEFILE.unlink()
            side = SAVEFILE.with_name(SAVEFILE.name + ".bb8state")
            if side.exists():
                side.unlink()
        except Exception:
            pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

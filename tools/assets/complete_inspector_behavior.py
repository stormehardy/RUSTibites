#!/usr/bin/env python3
"""complete_inspector_behavior.py - full KILL dialog texts/buttons + Lay Egg/tag effects."""
from __future__ import annotations

import json
import sys
import time
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import capture as cap  # noqa: E402

SAVE = ROOT / "testdata/parity/default/rust_seed_600.zip"
BEH = ROOT / "assets/game/reference/functional_ui/control_behavior.json"


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
            t = c.get("type")
            if t == "TMPro.TextMeshProUGUI" and c.get("text"):
                texts.append(str(c["text"]))
            elif t == "UnityEngine.UI.Button":
                labels = [str(cc.get("text")) for cc in (n.get("components") or [])
                          if cc.get("type") == "TMPro.TextMeshProUGUI" and cc.get("text")]
                buttons.append({"path": n.get("path"), "name": n.get("name"), "enabled": c.get("enabled"),
                                "labels": labels})
    return {"texts": texts, "buttons": buttons}


def main():
    global s
    B, _, Lo, La = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    s = cap.WorldSession(build_args(__import__("argparse").Namespace()), B, Lo, La)
    try:
        s.open(); s.plugin.pacing(True)
        s.client.load_world(SAVE); s.client.set_time(paused=True)
        post(s.plugin, "/d/select_bibite", {"x": -5.360003, "y": 831.0031})
        post(s.plugin, "/d/press_button", {"name": "StatsButton"})
        time.sleep(0.4)
        dump = s.plugin.ui_runtime(); n0 = info(dump)
        # Kill by StatsPanel node name/path
        kill = post(s.plugin, "/d/press_button", {"name": "KillBibiteButton"})
        time.sleep(0.4)
        d2 = s.plugin.ui_runtime(); n1 = info(d2)
        # all popup-ish active nodes
        active_paths = sorted({n.get("path") for n in nodes(d2)
                               if n.get("active_in_hierarchy") and any(w in (n.get("path") or "").lower()
                                                                       for w in ("popu", "confirm", "killing", "dialog"))})
        # dismiss popup by deactivating the deepest popup root
        dismissed = None
        for p in sorted(active_paths, key=len, reverse=True):
            if p.rsplit("/", 1)[-1] in ("Panel", "PopUp", "Pop-up", "Popuholder", "ConfirmPanel"):
                dismissed = post(s.plugin, "/d/activate", {"path": p, "active": False})
                if (dismissed or {}).get("ok"):
                    break
        time.sleep(0.2)
        # Lay Egg and tag on fresh Stats panel
        post(s.plugin, "/d/press_button", {"name": "StatsButton"})
        time.sleep(0.2)
        st0 = s.client.status().get("counts")
        lay = post(s.plugin, "/d/press_button", {"name": "LayBibiteEggButton"})
        time.sleep(0.3)
        st1 = s.client.status().get("counts")
        tag = post(s.plugin, "/d/press_button", {"name": "EditTagButton"})
        time.sleep(0.2)
        d3 = s.plugin.ui_runtime(); n3 = info(d3)
        data = json.loads(BEH.read_text(encoding="utf-8"))
        data["sections"]["kill_dialog_full"] = {"before": n0, "after": n1, "hit": kill,
                                                "active_popup_paths": active_paths, "dismiss": dismissed}
        data["sections"]["inspector_lay"] = {"before": st0, "after": st1, "hit": lay}
        data["sections"]["inspector_tag"] = {"before": n0, "after": n3, "hit": tag,
                                             "texts_new": [t for t in n3["texts"] if t not in n0["texts"]][:50]}
        BEH.write_text(json.dumps(data, indent=2), encoding="utf-8")
        print(json.dumps({"kill_hit": kill, "popup_paths": active_paths[:10], "lay": [st0, st1],
                          "tag_hit": tag, "kill_texts": [t for t in n1["texts"] if "sure" in t.lower() or "kill" in t.lower()]}, indent=1)[:4000])
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

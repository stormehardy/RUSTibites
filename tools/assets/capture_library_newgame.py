#!/usr/bin/env python3
"""capture_library_newgame.py - Bibite Library/editor + New Game/scenario editor flows.

Hidden Welcome UI: open Bibite Library, Create New/Templates/Edit, gene editor (control dump + randomize
attempts), brain editor (add-node/synapse control inventory), New Game scenario selector (world size/density/
TPS dropdowns/sliders), scenario editor zone/Challenge panels. Publishes atlas sessions, screenshots/overlays,
and library_newgame_behavior.json with paths/labels/control options.
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


def controls(plugin):
    r = plugin.httpx.get(plugin.base + "/d/control_dump", timeout=60).json()
    return r.get("controls") or []


def nodes(dump):
    out = []

    def walk(n):
        out.append(n)
        for k in n.get("children") or []:
            walk(k)

    for c in dump.get("canvases") or []:
        walk(c)
    return out


def active_buttons(dump):
    out = []
    for n in nodes(dump):
        if not n.get("active_in_hierarchy"):
            continue
        for c in n.get("components") or []:
            if c.get("type") == "UnityEngine.UI.Button":
                labels = [str(cc.get("text")) for cc in (n.get("components") or [])
                          if cc.get("type") == "TMPro.TextMeshProUGUI" and cc.get("text")]
                out.append({"path": n.get("path"), "name": n.get("name"), "enabled": c.get("enabled"), "labels": labels})
    return out


sessions = {}


def save(name, dump, extra=None):
    safe = re.sub(r"[^A-Za-z0-9_.-]+", "_", name)
    ov = b""
    try:
        r = s.plugin.httpx.get(s.plugin.base + "/d/ui_overlay", timeout=120).json()
        if r.get("ok"):
            ov = base64.b64decode(r["png_base64"])
    except Exception:
        pass
    ovp = OUT / f"{safe}_overlay_1920x1080.png"
    if ov:
        ovp.write_bytes(ov)
    st = {"overlay": str(ovp.relative_to(ROOT).as_posix()) if ov else None,
          "controls": controls(s.plugin), "active_buttons": active_buttons(dump)[:200],
          "active_panels": sorted({n.get("path") for n in nodes(dump) if n.get("active_in_hierarchy")
                                   and n.get("name", "").endswith(("Panel", "Selector", "Editor"))})}
    if extra:
        st.update(extra)
    dump["state"] = st
    sessions[name] = dump
    print(f"[flow] {name}: nodes={dump.get('node_count')} controls={len(st['controls'])}")
    return st


def click_label(plugin, label):
    r = post(plugin, "/d/press_label", {"label": label})
    time.sleep(0.3)
    return r


def main():
    global s
    OUT.mkdir(parents=True, exist_ok=True)
    B, _, Lo, La = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    s = cap.WorldSession(build_args(__import__("argparse").Namespace()), B, Lo, La)
    actions = []
    try:
        s.open(); s.plugin.pacing(True); time.sleep(0.8)
        post(s.plugin, "/d/activate", {"path": "UI", "active": True})
        post(s.plugin, "/d/normalize_ui", {"root": "UI", "alpha": 1})
        time.sleep(0.3)
        # library selector
        h = post(s.plugin, "/d/press_button", {"path": "UI/WelcomeScreen/Main Holder/ButtonsPanel/Editor"})
        time.sleep(0.4)
        d = s.plugin.ui_runtime(); save("library_selector_flow", d, {"press": h})
        actions.append({"step": "library_open", "hit": h, "active_buttons": active_buttons(d)[:60]})
        for label in ("Create New", "Template Bibites", "Templates"):
            before = s.plugin.ui_runtime()
            h = click_label(s.plugin, label)
            after = s.plugin.ui_runtime()
            save(f"library_{re.sub(r'[^a-z0-9]+','_',label.lower()).strip('_')}", after,
                 {"clicked": label, "hit": h})
            actions.append({"step": f"library:{label}", "hit": h,
                            "new_panels": sorted({n.get("path") for n in nodes(after) if n.get("active_in_hierarchy")}
                                                 - {n.get("path") for n in nodes(before) if n.get("active_in_hierarchy")})[:10]})
            break
        # open editor and genes
        for label in ("Edit Button", "Edit"):
            h = click_label(s.plugin, label)
            if (h or {}).get("ok"):
                break
        post(s.plugin, "/d/activate", {"path": "UI/BibiteSelector/BibiteEditor", "active": True})
        post(s.plugin, "/d/press_button", {"name": "GenesButton"})
        time.sleep(0.35)
        d = s.plugin.ui_runtime(); save("library_editor_genes_flow", d)
        g0 = controls(s.plugin)
        rand_hits = []
        for b in active_buttons(d):
            lab = " ".join(b.get("labels") or [])
            if any(w in lab.lower() for w in ("random", "randomize")):
                rand_hits.append({"button": b, "hit": click_label(s.plugin, (b.get("labels") or [b.get("name")])[0])})
        g1 = controls(s.plugin)
        actions.append({"step": "editor_genes", "before": g0, "after": g1, "randomize": rand_hits})
        # brain editor
        post(s.plugin, "/d/press_button", {"name": "brainButton"})
        time.sleep(0.35)
        d = s.plugin.ui_runtime(); save("library_editor_brain_flow", d)
        brain_actions = []
        for b in active_buttons(d):
            lab = " ".join(b.get("labels") or [])
            if any(w in lab.lower() for w in ("add", "node", "synapse", "new")):
                brain_actions.append({"button": b, "hit": click_label(s.plugin, (b.get("labels") or [b.get("name")])[0])})
        d2 = s.plugin.ui_runtime(); save("library_editor_brain_after_add", d2)
        actions.append({"step": "editor_brain", "buttons_before": active_buttons(d)[:80], "clicks": brain_actions,
                        "controls_after": controls(s.plugin)})
        # new game/scenario options
        post(s.plugin, "/d/activate", {"path": "UI/NewGameScenarios", "active": True})
        time.sleep(0.35)
        d = s.plugin.ui_runtime(); save("newgame_scenario_options", d)
        actions.append({"step": "newgame_options", "controls": controls(s.plugin),
                        "active_buttons": active_buttons(d)[:120]})
        for label in ("New Scenario Button", "New Challenge Button"):
            h = click_label(s.plugin, label)
            d = s.plugin.ui_runtime()
            save(f"newgame_{re.sub(r'[^a-z0-9]+','_',label.lower()).strip('_')}", d, {"clicked": label})
            actions.append({"step": f"newgame:{label}", "hit": h,
                            "active_panels": [n.get("path") for n in nodes(d) if n.get("active_in_hierarchy")][:30]})
        # scenario editor + zones/challenge
        post(s.plugin, "/d/activate", {"path": "UI/ScenarioEditor", "active": True})
        time.sleep(0.35)
        d = s.plugin.ui_runtime(); save("scenario_editor_flow", d)
        actions.append({"step": "scenario_editor", "controls": controls(s.plugin), "buttons": active_buttons(d)[:100]})
        for name in ("zones", "ChallengeOptions", "sim"):
            h = post(s.plugin, "/d/press_button", {"name": name})
            time.sleep(0.25)
            d = s.plugin.ui_runtime(); save(f"scenario_editor_{name}", d, {"press": h})
            actions.append({"step": f"scenario:{name}", "hit": h, "controls": controls(s.plugin)})
        # behavior + merge sessions + manifest
        behavior = {"schema": "rustibites.assets.functional_ui.library_newgame.v1",
                    "sessions": list(sessions), "actions": actions,
                    "note": "Hidden Welcome UI; library selector/editor gene+brain, New Game option controls, "
                            "scenario editor zone/challenge panels. Text labels/options from live controls."}
        (OUT / "library_newgame_behavior.json").write_text(json.dumps(behavior, indent=2), encoding="utf-8")
        mainj = json.loads(gzip.open(RUNTIME, "rt", encoding="utf-8").read())
        mainj.setdefault("sessions", {}).update(sessions)
        mainj.setdefault("capture", {})["library_newgame"] = {"sessions": list(sessions)}
        with gzip.open(RUNTIME, "wb", compresslevel=9) as g:
            g.write(json.dumps(mainj, indent=1).encode("utf-8"))
        manifest = json.loads((OUT / "manifest.json").read_text(encoding="utf-8"))
        manifest["library_newgame_behavior"] = "assets/game/reference/functional_ui/library_newgame_behavior.json"
        manifest["library_newgame_sessions"] = list(sessions)
        (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
        (OUT / "README.md").write_text((OUT / "README.md").read_text(encoding="utf-8") +
            "\n## Library/New Game flow ground truth\n\n`library_newgame_behavior.json` + sessions cover "
            "Bibite Library selector, Create New/Templates, gene editor controls/randomize, brain editor add-node/"
            "synapse inventory, New Game/scenario option controls, scenario editor zone/challenge panels; "
            "screenshots/overlays under this directory.\n", encoding="utf-8")
        print(json.dumps({"sessions": list(sessions), "actions": len(actions)}, indent=1))
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

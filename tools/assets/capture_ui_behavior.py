#!/usr/bin/env python3
"""capture_ui_behavior.py - click-effect ground truth for menu/toolbar/settings/inspector buttons.

Hidden menu pass resets/activates the Welcome UI and clicks the menu rows (Load, Bibite Library, Settings,
Achievements, Additional Tools, Credits) plus Settings subpages. Hidden world pass loads rust_seed_600,
clicks toolbar buttons (Placer, UserActions, View, DynamicParameters, AncestralLineage, Information, Settings),
tool labels exposed by the placer panel, the speed slider, selects a bibite and clicks StatsPanel actions
(Kill/Lay/Save/Edit). Each step records before/after counts, tools/zones, active panels, active labels/texts,
dialogs, clicked node paths and verdict. Writes behavioral_reference.json and updates functional_ui manifests.
"""
from __future__ import annotations

import gzip
import json
import sys
import time
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import capture as cap  # noqa: E402

SAVE = ROOT / "testdata/parity/default/rust_seed_600.zip"
REF = ROOT / "assets/game/reference/functional_ui"
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


def post(plugin, path, body):
    return plugin.httpx.post(plugin.base + path, json=body, timeout=120).json()


def ui_walk(dump):
    nodes = []

    def walk(n):
        nodes.append(n)
        for k in n.get("children") or []:
            walk(k)

    for r in dump.get("canvases") or []:
        walk(r)
    return nodes


def ui_snapshot(plugin, client):
    dump = plugin.ui_runtime()
    nodes = ui_walk(dump)
    active_panels = sorted({n.get("path") for n in nodes
                            if n.get("active_in_hierarchy") and n.get("name", "").endswith(("Panel", "Window", "PopUp", "Pop-up"))})
    labels = []
    active_texts = []
    for n in nodes:
        if not n.get("active_in_hierarchy"):
            continue
        for c in n.get("components") or []:
            if c.get("type") == "TMPro.TextMeshProUGUI" and c.get("text"):
                txt = str(c["text"]).strip()
                if txt:
                    active_texts.append(txt)
    for n in nodes:
        comps = n.get("components") or []
        if n.get("active_in_hierarchy") and any(c.get("type") in (
                "UnityEngine.UI.Button", "UnityEngine.UI.Toggle", "TMPro.TMP_Dropdown") for c in comps):
            txt = []
            for c in n.get("components") or []:
                if c.get("type") == "TMPro.TextMeshProUGUI" and c.get("text"):
                    txt.append(str(c["text"]))
            labels.append({"name": n.get("name"), "path": n.get("path"), "labels": txt[:4],
                           "controls": [c.get("type") for c in comps if c.get("type") in (
                               "UnityEngine.UI.Button", "UnityEngine.UI.Toggle", "TMPro.TMP_Dropdown")]})
    state = {"tick": None, "counts": None, "tools": None, "zones": None}
    try:
        st = client.run_state(timeout=15.0)
        state["tick"] = st.get("ticks") if st.get("ticks") is not None else int((st.get("simTime") or 0) * 30)
        state["simTime"] = st.get("simTime")
        state["state"] = st.get("state")
        state["counts"] = st.get("counts")
    except Exception as exc:  # noqa: BLE001
        state["state_error"] = str(exc)
    try:
        t = client.tools()
        state["tools"] = t
    except Exception as exc:  # noqa: BLE001
        state["tools_error"] = str(exc)
    try:
        state["zones"] = client.zones()
    except Exception as exc:  # noqa: BLE001
        state["zones_error"] = str(exc)
    dialogs = [t for t in active_texts if any(w in t.lower() for w in (
        "sure", "confirm", "kill", "delete", "save", "overwrite", "exit", "quit", "discard"))]
    return {"state": state, "active_panels": active_panels, "active_controls": labels[:120],
            "active_texts": active_texts[:200], "dialogs": dialogs}


def summarize(diff):
    return diff


def record(actions, name, before, hit, after):
    diff = {}
    for key in ("counts", "tools", "zones"):
        if before["state"].get(key) != after["state"].get(key):
            diff[key] = {"before": before["state"].get(key), "after": after["state"].get(key)}
    new_panels = sorted(set(after["active_panels"]) - set(before["active_panels"]))
    gone_panels = sorted(set(before["active_panels"]) - set(after["active_panels"]))
    new_texts = [t for t in after["active_texts"] if t not in before["active_texts"]][:40]
    action = {"action": name, "hit": hit, "before": before["state"], "after": after["state"],
              "active_panels_before": before["active_panels"], "active_panels_after": after["active_panels"],
              "new_panels": new_panels, "gone_panels": gone_panels,
              "new_texts": new_texts, "dialogs_after": after["dialogs"],
              "changed": bool(diff or new_panels or gone_panels or new_texts),
              "diff": diff}
    actions.append(action)
    return action


def main():
    global s
    OUT = REF
    OUT.mkdir(parents=True, exist_ok=True)
    B, _, Lo, La = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    s = cap.WorldSession(build_args(__import__("argparse").Namespace()), B, Lo, La)
    actions = []
    try:
        s.open()
        s.plugin.pacing(True)
        time.sleep(0.8)
        # menu
        menu_labels = ["Settings", "Bibite Library", "Achievements", "Additional Tools", "Credits", "Load"]
        reset_paths = ["UI/SettingsPanel", "UI/LoadGamePanel", "UI/NewGameScenarios", "UI/BibiteSelector",
                       "UI/WelcomeScreen/AchievementsPanel", "UI/WelcomeScreen/ToolsPanel",
                       "UI/WelcomeScreen/CreditsPanel", "UI/NewPanel", "UI/WorkshopItemsPanel", "UI/ScenarioEditor"]
        for label in menu_labels:
            for rp in reset_paths:
                post(s.plugin, "/d/activate", {"path": rp, "active": False})
            post(s.plugin, "/d/activate", {"path": "UI", "active": True})
            post(s.plugin, "/d/normalize_ui", {"root": "UI", "alpha": 1})
            time.sleep(0.2)
            before = ui_snapshot(s.plugin, s.client)
            hit = post(s.plugin, "/d/press_label", {"label": label})
            time.sleep(0.35)
            after = ui_snapshot(s.plugin, s.client)
            record(actions, f"menu:{label}", before, hit, after)
        # settings tabs
        for label in ("Settings", "Graphic Settings", "Game Settings", "Hotkeys"):
            post(s.plugin, "/d/activate", {"path": "UI", "active": True})
            post(s.plugin, "/d/normalize_ui", {"root": "UI", "alpha": 1})
            post(s.plugin, "/d/press_label", {"label": label})
            time.sleep(0.25)
            before = ui_snapshot(s.plugin, s.client)
            hit = post(s.plugin, "/d/press_label", {"label": label})
            time.sleep(0.25)
            after = ui_snapshot(s.plugin, s.client)
            record(actions, f"settings:{label}", before, hit, after)
        # world pass
        s.client.load_world(SAVE)
        s.client.set_time(paused=True)
        before = ui_snapshot(s.plugin, s.client)
        actions.append({"action": "world:load", "before": None, "after": before["state"],
                        "active_panels": before["active_panels"], "changed": True})
        for button in ["PlacerButton", "UserActions", "ViewButton", "DynamicParametersButton",
                       "AncestralLineageButton", "InformationPanelButton", "SettingsButton", "PauseButton"]:
            before = ui_snapshot(s.plugin, s.client)
            hit = post(s.plugin, "/d/press_button", {"name": button})
            time.sleep(0.3)
            after = ui_snapshot(s.plugin, s.client)
            record(actions, f"world:{button}", before, hit, after)
            if button == "PlacerButton":
                # tool labels exposed by the active placer panel
                tool_labels = []
                for c in after["active_controls"]:
                    txt = " ".join(c.get("labels") or []).lower()
                    name = (c.get("name") or "").lower()
                    if any(w in (txt + " " + name) for w in ("pellet", "rad", "phero", "killer", "placer")):
                        tool_labels.append((c.get("labels") or [c.get("name")])[0])
                for label in sorted(set(tool_labels))[:8]:
                    b2 = ui_snapshot(s.plugin, s.client)
                    h2 = post(s.plugin, "/d/press_label", {"label": label})
                    time.sleep(0.25)
                    a2 = ui_snapshot(s.plugin, s.client)
                    record(actions, f"placer:{label}", b2, h2, a2)
        # speed slider: find active slider path and set it
        snap = ui_snapshot(s.plugin, s.client)
        sliders = [n for n in ui_walk(s.plugin.ui_runtime()) if n.get("active_in_hierarchy") and any(
            c.get("type") == "UnityEngine.UI.Slider" for c in (n.get("components") or []))]
        if sliders:
            sp = sliders[0].get("path")
            before = ui_snapshot(s.plugin, s.client)
            hit = post(s.plugin, "/d/slider_set", {"path": sp, "value": 0.5})
            time.sleep(0.3)
            after = ui_snapshot(s.plugin, s.client)
            record(actions, f"slider:{sp}", before, hit, after)
        # inspector actions: select target and StatsButton
        sel = post(s.plugin, "/d/select_bibite", {"x": -5.360003, "y": 831.0031})
        post(s.plugin, "/d/press_button", {"name": "StatsButton"})
        time.sleep(0.4)
        before = ui_snapshot(s.plugin, s.client)
        for label in ("Kill", "Lay Egg", "Lay", "Save", "Edit"):
            b2 = ui_snapshot(s.plugin, s.client)
            h2 = post(s.plugin, "/d/press_label", {"label": label})
            time.sleep(0.3)
            a2 = ui_snapshot(s.plugin, s.client)
            record(actions, f"inspector:{label}", b2, h2, a2)
            # dismiss confirmation/dialog state before the next action
            if any(any(w in t.lower() for w in ("sure", "confirm", "cancel", "no")) for t in (a2.get("dialogs") or [])) or any(
                    "sure" in t.lower() for t in (a2.get("active_texts") or [])):
                d0 = ui_snapshot(s.plugin, s.client)
                dh = {"hits": []}
                for dl in ("No", "Cancel", "Close"):
                    r = post(s.plugin, "/d/press_label", {"label": dl})
                    dh["hits"].append(r)
                    time.sleep(0.15)
                    now = ui_snapshot(s.plugin, s.client)
                    if not any("sure" in t.lower() for t in (now.get("active_texts") or [])):
                        break
                record(actions, "inspector:dismiss", d0, dh, ui_snapshot(s.plugin, s.client))
        # publish behavior + update manifest/README
        out = {"schema": "rustibites.assets.functional_ui.behavior.v1",
               "capture": {"save": str(SAVE), "menu": "hidden Welcome UI activated/normalized",
                           "world": "hidden rust_seed_600 paused t=20s",
                           "method": "plugin /d/press_label, /d/press_button, /d/slider_set, /d/ui; before/after state"},
               "actions": actions}
        (OUT / "behavioral_reference.json").write_text(json.dumps(out, indent=2), encoding="utf-8")
        manifest = json.loads((OUT / "manifest.json").read_text(encoding="utf-8"))
        manifest["behavioral_reference"] = "assets/game/reference/functional_ui/behavioral_reference.json"
        (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
        readme = OUT / "README.md"
        readme.write_text((readme.read_text(encoding="utf-8") if readme.exists() else "") +
                          "\n## Click-effect behavioral reference\n\n`behavioral_reference.json` records each "
                          "menu/toolbar/settings/inspector action with before/after counts/tools/zones, active panels, "
                          "new labels/texts/dialogs, clicked node hit and a `changed` verdict. Generated by "
                          "`tools/assets/capture_ui_behavior.py` in the hidden game.\n")
        print(json.dumps({"actions": len(actions), "changed": sum(1 for a in actions if a.get("changed")),
                          "summary": [(a["action"], a.get("changed")) for a in actions]}, indent=1)[:5000])
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

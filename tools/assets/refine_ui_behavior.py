#!/usr/bin/env python3
"""refine_ui_behavior.py - fill click-effect gaps: menu rows by path/press, inspector dialog dismissal."""
from __future__ import annotations

import json
import sys
import time
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import capture as cap  # noqa: E402

SAVE = ROOT / "testdata/parity/default/rust_seed_600.zip"
BEH = ROOT / "assets/game/reference/functional_ui/behavioral_reference.json"


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


def snap(plugin, client):
    dump = plugin.ui_runtime()
    ns = nodes(dump)
    texts = []
    for n in ns:
        if not n.get("active_in_hierarchy"):
            continue
        for c in n.get("components") or []:
            if c.get("type") == "TMPro.TextMeshProUGUI" and c.get("text"):
                texts.append(str(c["text"]).strip())
    panels = sorted({n.get("path") for n in ns if n.get("active_in_hierarchy") and n.get("name", "").endswith(("Panel", "PopUp", "Pop-up"))})
    try:
        st = client.run_state(timeout=15.0)
        state = {"tick": st.get("ticks") or int((st.get("simTime") or 0) * 30), "counts": st.get("counts")}
    except Exception as exc:  # noqa: BLE001
        state = {"error": str(exc)}
    return {"state": state, "active_panels": panels, "active_texts": texts[:200]}


def add(actions, name, before, hit, after, note=None):
    a = {"action": name, "hit": hit, "before": before["state"], "after": after["state"],
         "active_panels_before": before["active_panels"], "active_panels_after": after["active_panels"],
         "new_panels": sorted(set(after["active_panels"]) - set(before["active_panels"])),
         "gone_panels": sorted(set(before["active_panels"]) - set(after["active_panels"])),
         "new_texts": [t for t in after["active_texts"] if t not in before["active_texts"]][:40],
         "dialogs_after": [t for t in after["active_texts"] if any(w in t.lower() for w in ("sure", "confirm", "kill", "delete"))],
         "changed": bool(set(after["active_panels"]) ^ set(before["active_panels"])), "note": note}
    actions.append(a)
    print(a["action"], "hit", (hit or {}).get("ok"), "panels+", a["new_panels"][:3])


def main():
    global s
    B, _, Lo, La = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    s = cap.WorldSession(build_args(__import__("argparse").Namespace()), B, Lo, La)
    extra = []
    try:
        s.open()
        s.plugin.pacing(True)
        time.sleep(0.8)
        reset = ["UI/SettingsPanel", "UI/LoadGamePanel", "UI/NewGameScenarios", "UI/BibiteSelector",
                 "UI/WelcomeScreen/AchievementsPanel", "UI/WelcomeScreen/ToolsPanel", "UI/WelcomeScreen/CreditsPanel",
                 "UI/NewPanel", "UI/WorkshopItemsPanel", "UI/ScenarioEditor"]
        for name, path in [("menu:Bibite Library (path)", "UI/WelcomeScreen/Main Holder/ButtonsPanel/Editor"),
                           ("menu:Additional Tools (path)", "UI/WelcomeScreen/Main Holder/ButtonsPanel/Section/More"),
                           ("menu:Load (path)", "UI/WelcomeScreen/Main Holder/ButtonsPanel/LoadSim")]:
            for rp in reset:
                post(s.plugin, "/d/activate", {"path": rp, "active": False})
            post(s.plugin, "/d/activate", {"path": "UI", "active": True})
            post(s.plugin, "/d/normalize_ui", {"root": "UI", "alpha": 1})
            time.sleep(0.2)
            before = snap(s.plugin, s.client)
            hit = post(s.plugin, "/d/press_button", {"path": path})
            if not (hit or {}).get("ok"):
                hit = post(s.plugin, "/d/press_button", {"name": path.rsplit("/", 1)[-1]})
            time.sleep(0.35)
            after = snap(s.plugin, s.client)
            add(extra, name, before, hit, after, "pressed by node path/name")
        # inspector clean pass
        s.client.load_world(SAVE)
        s.client.set_time(paused=True)
        post(s.plugin, "/d/select_bibite", {"x": -5.360003, "y": 831.0031})
        post(s.plugin, "/d/press_button", {"name": "StatsButton"})
        time.sleep(0.4)
        before = snap(s.plugin, s.client)
        kill = post(s.plugin, "/d/press_label", {"label": "Kill"})
        time.sleep(0.3)
        after = snap(s.plugin, s.client)
        add(extra, "inspector:Kill(clean)", before, kill, after)
        # find confirmation root and deactivate it
        dump = s.plugin.ui_runtime()
        ns = nodes(dump)
        candidates = []
        for n in ns:
            if not n.get("active_in_hierarchy"):
                continue
            p = n.get("path", "")
            if any(w in p.lower() for w in ("killing", "confirm", "popup", "popup", "popuholder")):
                candidates.append(p)
        dismiss = {"ok": False, "candidates": candidates}
        for p in candidates:
            if p.endswith(("Panel", "PopUp", "Pop-up", "Popuholder")):
                dismiss = post(s.plugin, "/d/activate", {"path": p, "active": False})
                if (dismiss or {}).get("ok"):
                    break
        time.sleep(0.2)
        after2 = snap(s.plugin, s.client)
        add(extra, "inspector:Kill dismiss", after, dismiss, after2)
        for label in ("Lay Egg", "Save", "Edit"):
            b = snap(s.plugin, s.client)
            h = post(s.plugin, "/d/press_label", {"label": label})
            time.sleep(0.3)
            a = snap(s.plugin, s.client)
            add(extra, f"inspector:{label}(clean)", b, h, a)
        data = json.loads(BEH.read_text(encoding="utf-8"))
        data.setdefault("refined_actions", []).extend(extra)
        data["refined_note"] = "Path/name-pressed menu rows and inspector actions after dismissing the KILL confirmation."
        BEH.write_text(json.dumps(data, indent=2), encoding="utf-8")
        print(json.dumps({"extra": len(extra)}, indent=1))
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

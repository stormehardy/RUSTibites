#!/usr/bin/env python3
"""capture_deferred_last.py - hotkey page (read-only wording), workshop EditShared buttons, challenge params."""
from __future__ import annotations

import base64
import gzip
import json
import re
import sys
import time
from pathlib import Path

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
        if n.get("name", "").endswith(("Panel", "PopUp", "Pop-up", "Selector")):
            panels.append(p)
        for c in n.get("components") or []:
            if c.get("type") == "TMPro.TextMeshProUGUI" and c.get("text"):
                texts.append(str(c["text"]))
            elif c.get("type") == "UnityEngine.UI.Button":
                labels = [str(cc.get("text")) for cc in (n.get("components") or [])
                          if cc.get("type") == "TMPro.TextMeshProUGUI" and cc.get("text")]
                buttons.append({"path": p, "name": n.get("name"), "enabled": c.get("enabled"), "labels": labels})
    return {"texts": texts[:250], "buttons": buttons[:250], "panels": panels[:80]}


def controls(plugin):
    return plugin.httpx.get(plugin.base + "/d/control_dump", timeout=60).json().get("controls") or []


sessions = {}


def save(name, dump):
    safe = re.sub(r"[^A-Za-z0-9_.-]+", "_", name)
    ov = b""
    try:
        r = s.plugin.httpx.get(s.plugin.base + "/d/ui_overlay", timeout=120).json()
        if r.get("ok"):
            ov = base64.b64decode(r["png_base64"])
    except Exception:
        pass
    if ov:
        (OUT / f"{safe}_overlay_1920x1080.png").write_bytes(ov)
    dump["state"] = {"controls": controls(s.plugin), "info": info(dump),
                     "overlay": str((OUT / f"{safe}_overlay_1920x1080.png").relative_to(ROOT).as_posix()) if ov else None}
    sessions[name] = dump
    print(f"[last] {name}: controls={len(dump['state']['controls'])} texts={len(dump['state']['info']['texts'])}")


def main():
    global s
    OUT.mkdir(parents=True, exist_ok=True)
    B, _, Lo, La = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    s = cap.WorldSession(build_args(__import__("argparse").Namespace()), B, Lo, La)
    data = {"schema": "rustibites.assets.functional_ui.deferred_controls.v1", "steps": []}
    try:
        s.open(); s.plugin.pacing(True); time.sleep(0.8)
        post(s.plugin, "/d/activate", {"path": "UI", "active": True})
        post(s.plugin, "/d/normalize_ui", {"root": "UI", "alpha": 1})
        # hotkeys page: read-only list
        post(s.plugin, "/d/press_button", {"path": "UI/WelcomeScreen/Main Holder/ButtonsPanel/Settings"})
        post(s.plugin, "/d/press_button", {"path": "UI/SettingsPanel/SelectorPanel/List/Controls Button"})
        time.sleep(0.35)
        d = s.plugin.ui_runtime(); save("hotkeys_readonly_flow", d_info if False else d)
        data["steps"].append({"step": "hotkeys_page", "ui": info(d), "controls": controls(s.plugin),
                              "note": "Hotkeys page is read-only: disclaimer says controls can't be changed yet; no keybind row buttons."})
        # workshop local edit buttons
        post(s.plugin, "/d/activate", {"path": "UI/WorkshopItemsPanel", "active": True})
        post(s.plugin, "/d/activate", {"path": "UI/WorkshopItemsPanel/EditSharedItemPanel", "active": True})
        time.sleep(0.35)
        d = s.plugin.ui_runtime(); save("workshop_edit_last", d)
        ws = []
        for path in ["UI/WorkshopItemsPanel/EditSharedItemPanel/Return",
                     "UI/WorkshopItemsPanel/EditSharedItemPanel/Holder/EditPanel/DeleteItemButton",
                     "UI/WorkshopItemsPanel/EditSharedItemPanel/Holder/EditPanel/Item Info/TopImage/Preview Holder/preview/ChangePreviewButton",
                     "UI/WorkshopItemsPanel/EditSharedItemPanel/Holder/EditPanel/Item Info/InfoList/File Line/UpdateFileButton",
                     "UI/WorkshopItemsPanel/EditSharedItemPanel/Holder/EditPanel/Item Info/InfoList/Footer/Submit Button"]:
            before = info(s.plugin.ui_runtime())
            hit = post(s.plugin, "/d/press_button", {"path": path})
            time.sleep(0.25)
            after = info(s.plugin.ui_runtime())
            ws.append({"path": path, "hit": hit, "new_panels": sorted(set(after["panels"]) - set(before["panels"])),
                       "new_texts": [t for t in after["texts"] if t not in before["texts"]][:20]})
            # restore edit panel for next action
            post(s.plugin, "/d/activate", {"path": "UI/WorkshopItemsPanel/EditSharedItemPanel", "active": True})
        data["steps"].append({"step": "workshop_edit_buttons", "actions": ws})
        # challenge params under active ScenarioEditor
        post(s.plugin, "/d/activate", {"path": "UI/ScenarioEditor", "active": True})
        time.sleep(0.25)
        ch = post(s.plugin, "/d/press_button", {"name": "ChallengeOptions"})
        if not (ch or {}).get("ok"):
            ch = post(s.plugin, "/d/press_button", {"path": "UI/ScenarioEditor/Settings Tuner/Holder/Panels/LeftSide Buttons/ChallengeOptions"})
        time.sleep(0.35)
        d = s.plugin.ui_runtime(); save("challenge_params_last", d)
        data["steps"].append({"step": "challenge_params", "press": ch, "ui": info(d), "controls": controls(s.plugin)})
        (OUT / "deferred_last_controls.json").write_text(json.dumps(data, indent=2), encoding="utf-8")
        mainj = json.loads(gzip.open(RUNTIME, "rt", encoding="utf-8").read())
        mainj.setdefault("sessions", {}).update(sessions)
        mainj.setdefault("capture", {})["deferred_last_controls"] = {"sessions": list(sessions)}
        with gzip.open(RUNTIME, "wb", compresslevel=9) as g:
            g.write(json.dumps(mainj, indent=1).encode("utf-8"))
        manifest = json.loads((OUT / "manifest.json").read_text(encoding="utf-8"))
        manifest["deferred_last_controls"] = "assets/game/reference/functional_ui/deferred_last_controls.json"
        manifest["deferred_last_sessions"] = list(sessions)
        (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
        (OUT / "README.md").write_text((OUT / "README.md").read_text(encoding="utf-8") +
            "\n## Deferred controls\n\nHotkeys page is read-only (disclaimer; no keybind row buttons). Workshop EditShared "
            "button effects and challenge parameter editor fields/values/star text captured in "
            "`deferred_last_controls.json` + sessions.\n", encoding="utf-8")
        print(json.dumps({"sessions": list(sessions)}, indent=1))
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

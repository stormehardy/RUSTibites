#!/usr/bin/env python3
"""capture_challenge_lineage_workshop.py - local challenge/champion/params, lineage/species, workshop states."""
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


def controls(plugin):
    return plugin.httpx.get(plugin.base + "/d/control_dump", timeout=60).json().get("controls") or []


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
    texts, buttons, panels, images = [], [], [], []
    for n in nodes(dump):
        if not n.get("active_in_hierarchy"):
            continue
        p = n.get("path", "")
        if n.get("name", "").endswith(("Panel", "PopUp", "Pop-up", "Selector")):
            panels.append(p)
        for c in n.get("components") or []:
            t = c.get("type")
            if t == "TMPro.TextMeshProUGUI" and c.get("text"):
                texts.append(str(c["text"]))
            elif t == "UnityEngine.UI.Button":
                labels = [str(cc.get("text")) for cc in (n.get("components") or [])
                          if cc.get("type") == "TMPro.TextMeshProUGUI" and cc.get("text")]
                buttons.append({"path": p, "name": n.get("name"), "enabled": c.get("enabled"), "labels": labels})
            elif t in ("UnityEngine.UI.Image", "UnityEngine.UI.RawImage"):
                spr = c.get("sprite")
                if isinstance(spr, dict) and spr.get("name"):
                    images.append({"path": p, "sprite": spr.get("name")})
    return {"texts": texts[:250], "buttons": buttons[:200], "panels": panels[:100], "images": images[:100]}


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
    if ov:
        (OUT / f"{safe}_overlay_1920x1080.png").write_bytes(ov)
    st = {"controls": controls(s.plugin), "info": info(dump),
          "overlay": str((OUT / f"{safe}_overlay_1920x1080.png").relative_to(ROOT).as_posix()) if ov else None}
    if extra:
        st.update(extra)
    dump["state"] = st
    sessions[name] = dump
    print(f"[cap] {name}: nodes={dump.get('node_count')} controls={len(st['controls'])} texts={len(st['info']['texts'])}")


def main():
    global s
    OUT.mkdir(parents=True, exist_ok=True)
    B, _, Lo, La = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    s = cap.WorldSession(build_args(__import__("argparse").Namespace()), B, Lo, La)
    data = {"schema": "rustibites.assets.functional_ui.challenge_lineage_workshop.v1", "steps": []}
    try:
        s.open(); s.plugin.pacing(True); time.sleep(0.8)
        post(s.plugin, "/d/activate", {"path": "UI", "active": True})
        post(s.plugin, "/d/normalize_ui", {"root": "UI", "alpha": 1})
        # challenge selector/params/champion
        post(s.plugin, "/d/activate", {"path": "UI/NewGameScenarios", "active": True})
        time.sleep(0.35)
        save("challenge_selector", s.plugin.ui_runtime())
        for name, path in [
            ("challenge_scenario_panel", "UI/NewGameScenarios/ScenarioSelector/Holder/Scenario Panel"),
            ("challenge_champion_selection", "UI/NewGameScenarios/ScenarioSelector/Holder/RightPanels/RightGroup/ChampionSelectionPanel"),
            ("challenge_parameters_editor", "UI/ScenarioEditor/Settings Tuner/Holder/Panels/ChallengeParameters"),
        ]:
            post(s.plugin, "/d/activate", {"path": path, "active": True})
            time.sleep(0.3)
            save(name, s.plugin.ui_runtime())
            data["steps"].append({"step": name, "path": path})
        # workshop local panels
        for name, path in [("workshop_panel", "UI/WorkshopItemsPanel"),
                           ("workshop_edit_shared", "UI/WorkshopItemsPanel/EditSharedItemPanel")]:
            post(s.plugin, "/d/activate", {"path": path, "active": True})
            time.sleep(0.3)
            save(name, s.plugin.ui_runtime())
            data["steps"].append({"step": name, "path": path})
        # world lineage/species panels
        s.client.load_world(SAVE); s.client.set_time(paused=True)
        post(s.plugin, "/d/press_button", {"name": "AncestralLineageButton"})
        time.sleep(0.4)
        save("lineage_species_panel", s.plugin.ui_runtime())
        for name, path in [
            ("lineage_species_tabs", "UICanvas/MainUI/Layer2/AncestralLineagePanel/Overlay/SpeciesPanels/Species Tabs"),
            ("lineage_species_info", "UICanvas/MainUI/Layer2/AncestralLineagePanel/Overlay/SpeciesPanels/Species Info Panel"),
            ("lineage_species_genes", "UICanvas/MainUI/Layer2/AncestralLineagePanel/Overlay/SpeciesPanels/Species Genes Panel"),
            ("lineage_species_brain", "UICanvas/MainUI/Layer2/AncestralLineagePanel/Overlay/SpeciesPanels/SpeciesBrainPanel"),
        ]:
            post(s.plugin, "/d/activate", {"path": path, "active": True})
            time.sleep(0.3)
            save(name, s.plugin.ui_runtime())
            data["steps"].append({"step": name, "path": path})
        # challenge tracker/end popup availability note
        allpaths = set()
        for n in nodes(s.plugin.ui_runtime()):
            allpaths.add(n.get("path", ""))
        data["challenge_tracker_paths"] = sorted([p for p in allpaths if any(w in p.lower() for w in ("tracker", "endpopup", "end popup", "challengeend"))])
        data["rung_paths"] = sorted([p for p in allpaths if "rung" in p.lower()])
        (OUT / "challenge_lineage_workshop.json").write_text(json.dumps(data, indent=2), encoding="utf-8")
        mainj = json.loads(gzip.open(RUNTIME, "rt", encoding="utf-8").read())
        mainj.setdefault("sessions", {}).update(sessions)
        mainj.setdefault("capture", {})["challenge_lineage_workshop"] = {"sessions": list(sessions)}
        with gzip.open(RUNTIME, "wb", compresslevel=9) as g:
            g.write(json.dumps(mainj, indent=1).encode("utf-8"))
        manifest = json.loads((OUT / "manifest.json").read_text(encoding="utf-8"))
        manifest["challenge_lineage_workshop"] = "assets/game/reference/functional_ui/challenge_lineage_workshop.json"
        manifest["challenge_lineage_workshop_sessions"] = list(sessions)
        (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
        (OUT / "README.md").write_text((OUT / "README.md").read_text(encoding="utf-8") +
            "\n## Challenge/lineage/workshop ground truth\n\n`challenge_lineage_workshop.json` + sessions cover local "
            "challenge selector/champion selection/challenge parameters, Workshop local panels (subscribed/shared/edit), "
            "and Ancestral Lineage species tabs/info/genes/brain. Challenge tracker/end-popup/rung-history nodes were not "
            "present in the local menu UI (list recorded in the JSON).\n", encoding="utf-8")
        print(json.dumps({"sessions": list(sessions), "tracker_paths": data["challenge_tracker_paths"], "rung": data["rung_paths"]}, indent=1))
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

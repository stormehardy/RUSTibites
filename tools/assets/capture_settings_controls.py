#!/usr/bin/env python3
"""capture_settings_controls.py - full Settings/Graphics/Gameplay/Hotkeys + in-world settings ground truth.

Hidden menu: open SettingsPanel, activate Graphics/Gameplay/Hotkeys pages, dump every active Toggle/Slider/
Dropdown/Input with path, current value, options and label context; per-page UI sessions/screenshots.
Hidden world: open DynamicParameters and Settings panels, dump the same controls. Extends
control_behavior.json/manifest. Hidden-only.
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


def nodes(dump):
    out = []

    def walk(n):
        out.append(n)
        for k in n.get("children") or []:
            walk(k)

    for r in dump.get("canvases") or []:
        walk(r)
    return out


def text_desc(n):
    out = []
    for c in n.get("components") or []:
        if c.get("type") == "TMPro.TextMeshProUGUI" and c.get("text"):
            out.append(str(c["text"]))
    for k in n.get("children") or []:
        out += text_desc(k)
    return out


def enrich(control_list, dump):
    context = {n.get("path"): text_desc(n) for n in nodes(dump) if n.get("path")}
    for c in control_list:
        path = c.get("path") or ""
        labs = []
        parts = path.split("/")
        for i in range(len(parts), 0, -1):
            for t in context.get("/".join(parts[:i]), [])[:12]:
                if t not in labs:
                    labs.append(t)
        c["label_context"] = labs[:10]
    return control_list


def save_session(name, dump):
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
    dump["state"] = {"controls": enrich(controls(s.plugin), dump), "overlay": str((OUT / f"{safe}_overlay_1920x1080.png").relative_to(ROOT).as_posix()) if ov else None}
    sessions[name] = dump


def controls(plugin):
    return plugin.httpx.get(plugin.base + "/d/control_dump", timeout=60).json().get("controls") or []


sessions = {}


def main():
    global s
    OUT.mkdir(parents=True, exist_ok=True)
    B, _, Lo, La = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    s = cap.WorldSession(build_args(__import__("argparse").Namespace()), B, Lo, La)
    data = {"schema": "rustibites.assets.functional_ui.settings_controls.v1", "pages": {}}
    try:
        s.open(); s.plugin.pacing(True); time.sleep(0.8)
        post(s.plugin, "/d/activate", {"path": "UI", "active": True})
        post(s.plugin, "/d/normalize_ui", {"root": "UI", "alpha": 1})
        post(s.plugin, "/d/press_button", {"path": "UI/WelcomeScreen/Main Holder/ButtonsPanel/Settings"})
        time.sleep(0.4)
        pages = [("graphics", "UI/SettingsPanel/SelectorPanel/List/GraphicsButton"),
                 ("gameplay", "UI/SettingsPanel/SelectorPanel/List/Gameplay Button"),
                 ("hotkeys", "UI/SettingsPanel/SelectorPanel/List/Controls Button")]
        for name, path in pages:
            post(s.plugin, "/d/press_button", {"path": path})
            time.sleep(0.4)
            dump = s.plugin.ui_runtime()
            cs = enrich(controls(s.plugin), dump)
            save_session(f"settings_page_{name}_controls", dump)
            data["pages"][f"menu_{name}"] = {"control_path": path, "controls": cs}
        # in-world dynamic params + settings
        s.client.load_world(SAVE); s.client.set_time(paused=True)
        for name, button in [("world_dynamic_parameters", "DynamicParametersButton"),
                             ("world_settings", "SettingsButton")]:
            post(s.plugin, "/d/press_button", {"name": button})
            time.sleep(0.4)
            dump = s.plugin.ui_runtime()
            cs = enrich(controls(s.plugin), dump)
            save_session(f"{name}_controls", dump)
            data["pages"][name] = {"button": button, "controls": cs}
        # merge/extend
        (OUT / "settings_controls.json").write_text(json.dumps(data, indent=2), encoding="utf-8")
        cb = json.loads((OUT / "control_behavior.json").read_text(encoding="utf-8"))
        cb["sections"]["settings_pages_full"] = data["pages"]
        (OUT / "control_behavior.json").write_text(json.dumps(cb, indent=2), encoding="utf-8")
        mainj = json.loads(gzip.open(RUNTIME, "rt", encoding="utf-8").read())
        mainj.setdefault("sessions", {}).update(sessions)
        mainj.setdefault("capture", {})["settings_controls"] = {"sessions": list(sessions)}
        with gzip.open(RUNTIME, "wb", compresslevel=9) as g:
            g.write(json.dumps(mainj, indent=1).encode("utf-8"))
        manifest = json.loads((OUT / "manifest.json").read_text(encoding="utf-8"))
        manifest["settings_controls"] = "assets/game/reference/functional_ui/settings_controls.json"
        manifest["settings_control_sessions"] = list(sessions)
        (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
        (OUT / "README.md").write_text((OUT / "README.md").read_text(encoding="utf-8") +
            "\n## Full settings pages\n\n`settings_controls.json` + `settings_page_{graphics,gameplay,hotkeys}_controls` "
            "and `world_{dynamic_parameters,settings}_controls` sessions contain every active Toggle/Slider/"
            "Dropdown/Input per page with path, current value, options and label context.\n", encoding="utf-8")
        print(json.dumps({k: len(v["controls"]) for k, v in data["pages"].items()}, indent=1))
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

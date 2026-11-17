#!/usr/bin/env python3
"""capture_native_inworld_refs.py - native 1920 transparent in-world UI references.

Standalone hidden launch at native 1920 (bypasses BibitesAPI's 320x240 hidden launcher), loads
rust_seed_600 paused, and captures /d/ui_overlay (transparent UI-only) for toolbar, Placer/tool panel,
Stats/Genes/Biology/Brain inspector, DynamicParameters/Settings/Lineage/Information. Writes native refs +
manifest under assets/game/reference/functional_ui_native/in_world/. No visible window; cleans up.
"""
from __future__ import annotations

import base64
import ctypes
import ctypes.wintypes as wt
import io
import json
import subprocess
import sys
import time
from pathlib import Path

import httpx
import numpy as np
from PIL import Image

ROOT = Path(r"Z:\RUSTibites")
GAME = Path(r"C:\Program Files (x86)\Steam\steamapps\common\The Bibites")
EXE = GAME / "The Bibites.exe"
PLUGIN_DIR = GAME / "BepInEx/plugins/ParityDump"
PLUGIN_DLL = ROOT / "testdata/parity/plugin/bin/ParityDump.dll"
SAVE = ROOT / "testdata/parity/default/rust_seed_600.zip"
OUT = ROOT / "assets/game/reference/functional_ui_native/in_world"


def hide(pid):
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    cb_t = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
    found = []

    def cb(hwnd, _):
        p = wt.DWORD(); user32.GetWindowThreadProcessId(hwnd, ctypes.byref(p))
        if p.value == pid:
            user32.ShowWindow(hwnd, 0)
            user32.SetWindowPos(hwnd, 0, -32000, -32000, 0, 0, 1 | 4)
            found.append(int(hwnd))
        return True
    user32.EnumWindows(cb_t(cb), 0)
    return found


def wait(url, timeout):
    end = time.time() + timeout
    while time.time() < end:
        try:
            if httpx.get(url, timeout=2).status_code == 200:
                return True
        except Exception:
            pass
        time.sleep(0.25)
    return False


def get(base, path):
    return httpx.get(base + path, timeout=120).json()


def post(base, path, body=None):
    return httpx.post(base + path, json=body or {}, timeout=120).json()


def sharp(png):
    im = Image.open(io.BytesIO(png)).convert("RGBA")
    a = np.asarray(im); alpha = a[:, :, 3] > 0
    return {"width": im.width, "height": im.height, "alpha_pixels": int(alpha.sum()),
            "alpha_fraction": float(alpha.mean()),
            "unique_opaque_colors": int(len(np.unique(a[:, :, :3][alpha].reshape(-1, 3), axis=0)) if alpha.any() else 0)}


def capture(plugin, name, steps, extra=None):
    r = get(plugin, "/d/ui_overlay")
    if not r.get("ok"):
        steps.append({"session": name, "error": r.get("error")})
        return
    png = base64.b64decode(r["png_base64"])
    p = OUT / f"{name}.png"
    p.write_bytes(png)
    rec = {"session": name, "file": str(p.relative_to(ROOT).as_posix()), "source": "standalone native 1920 hidden game /d/ui_overlay",
           "role": "overlay", "sharpness": sharp(png)}
    if extra:
        rec.update(extra)
    steps.append(rec)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    PLUGIN_DIR.mkdir(parents=True, exist_ok=True)
    subprocess.run(["copy", str(PLUGIN_DLL), str(PLUGIN_DIR / "ParityDump.dll")], shell=True, check=True,
                   stdout=subprocess.DEVNULL)
    si = subprocess.STARTUPINFO(); si.dwFlags |= subprocess.STARTF_USESHOWWINDOW; si.wShowWindow = 0
    proc = subprocess.Popen([str(EXE), "-screen-width", "1920", "-screen-height", "1080", "-window-mode", "windowed"],
                            cwd=str(GAME), startupinfo=si, creationflags=subprocess.CREATE_NEW_PROCESS_GROUP,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    refs = []
    report = {"pid": proc.pid, "screen": None, "sessions": refs}
    try:
        report["bridge"] = wait("http://127.0.0.1:17800/ping", 180)
        report["plugin"] = wait("http://127.0.0.1:17801/d/ping", 60)
        report["hidden"] = hide(proc.pid)
        sys.path.insert(0, str(ROOT / "tools" / "parity"))
        import capture as cap
        B, _, _, _ = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
        client = B(port=17800)
        client.load_world(SAVE)
        client.set_time(paused=True)
        report["screen"] = get("http://127.0.0.1:17801", "/d/ui").get("screen")
        capture("http://127.0.0.1:17801", "native_world_toolbar", refs)
        # toolbar/panels
        for name, button in [("native_world_placer", "PlacerButton"), ("native_world_user_actions", "UserActions"),
                             ("native_world_view", "ViewButton"), ("native_world_dynamic", "DynamicParametersButton"),
                             ("native_world_lineage", "AncestralLineageButton"),
                             ("native_world_information", "InformationPanelButton"),
                             ("native_world_settings", "SettingsButton")]:
            r = post("http://127.0.0.1:17801", "/d/press_button", {"name": button})
            time.sleep(0.4)
            capture("http://127.0.0.1:17801", name, refs, {"button": button, "press": r})
        # placer tool panel
        post("http://127.0.0.1:17801", "/d/press_button", {"name": "PlacerButton"})
        post("http://127.0.0.1:17801", "/d/press_label", {"label": "PlacePellets"})
        time.sleep(0.4)
        capture("http://127.0.0.1:17801", "native_world_placer_pellets", refs)
        # inspector tabs
        post("http://127.0.0.1:17801", "/d/select_bibite", {"x": -5.360003, "y": 831.0031})
        post("http://127.0.0.1:17801", "/d/press_button", {"name": "StatsButton"})
        time.sleep(0.4)
        capture("http://127.0.0.1:17801", "native_inspector_stats", refs)
        for name, button in [("native_inspector_genes", "GeneButton"), ("native_inspector_biology", "BiologyButton"),
                             ("native_inspector_brain", "BrainButton")]:
            r = post("http://127.0.0.1:17801", "/d/press_button", {"name": button})
            time.sleep(0.4)
            capture("http://127.0.0.1:17801", name, refs, {"button": button, "press": r})
        report["sessions"] = refs
        (OUT / "manifest.json").write_text(json.dumps({"schema": "rustibites.ui_visual_reference_native.in_world.v1",
                                                       "screen": report["screen"], "standalone_launch": True,
                                                       "sessions": refs,
                                                       "note": "Native 1920 transparent UI-only in-world refs; menu "
                                                               "challenge/workshop/settings remain menu-canvas blocked."}, indent=2),
                                           encoding="utf-8")
        print(json.dumps({"screen": report["screen"], "sessions": [(x.get("session"), x.get("sharpness")) for x in refs]}, indent=1)[:6000])
    finally:
        try:
            proc.terminate(); proc.wait(timeout=15)
        except Exception:
            subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"], check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for p in PLUGIN_DIR.glob("*"):
            try: p.unlink()
            except Exception: pass
        try: PLUGIN_DIR.rmdir()
        except Exception: pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

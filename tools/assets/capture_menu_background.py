#!/usr/bin/env python3
"""capture_menu_background.py - native 1920 menu background-only frames + video info."""
from __future__ import annotations

import base64
import ctypes
import ctypes.wintypes as wt
import hashlib
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
OUT = ROOT / "assets/game/reference/functional_ui_native/menu_background"


def hide(pid):
    u = ctypes.WinDLL("user32", use_last_error=True); cb_t = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
    out = []

    def cb(hwnd, _):
        p = wt.DWORD(); u.GetWindowThreadProcessId(hwnd, ctypes.byref(p))
        if p.value == pid:
            u.ShowWindow(hwnd, 0); u.SetWindowPos(hwnd, 0, -32000, -32000, 0, 0, 1 | 4); out.append(int(hwnd))
        return True
    u.EnumWindows(cb_t(cb), 0)
    return out


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


def post(P, path, body=None):
    return httpx.post(P + path, json=body or {}, timeout=120).json()


def get(P, path):
    return httpx.get(P + path, timeout=120).json()


def save(name, b64):
    png = base64.b64decode(b64)
    p = OUT / f"{name}_1920x1080.png"
    p.write_bytes(png)
    im = Image.open(p).convert("RGB"); a = np.asarray(im)
    return {"file": str(p.relative_to(ROOT).as_posix()), "sha256": hashlib.sha256(png).hexdigest(),
            "size": list(im.size), "mean_rgb": [float(a[:, :, i].mean()) for i in range(3)],
            "unique_colors": int(len(np.unique(a.reshape(-1, 3), axis=0)))}


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    PLUGIN_DIR.mkdir(parents=True, exist_ok=True)
    subprocess.run(["copy", str(PLUGIN_DLL), str(PLUGIN_DIR / "ParityDump.dll")], shell=True, check=True,
                   stdout=subprocess.DEVNULL)
    si = subprocess.STARTUPINFO(); si.dwFlags |= subprocess.STARTF_USESHOWWINDOW; si.wShowWindow = 0
    proc = subprocess.Popen([str(EXE), "-screen-width", "1920", "-screen-height", "1080", "-window-mode", "windowed"],
                            cwd=str(GAME), startupinfo=si, creationflags=subprocess.CREATE_NEW_PROCESS_GROUP,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    P = "http://127.0.0.1:17801"
    out = {"schema": "rustibites.ui.native_menu_background.v1", "screen": None, "frames": [], "video": {}}
    try:
        assert wait("http://127.0.0.1:17800/ping", 180)
        assert wait(P + "/d/ping", 60)
        hide(proc.pid)
        time.sleep(2.0)
        post(P, "/d/activate", {"path": "UI", "active": True})
        post(P, "/d/normalize_ui", {"root": "UI", "alpha": 1})
        time.sleep(0.6)
        out["screen"] = get(P, "/d/ui").get("screen")
        out["video"] = get(P, "/d/menu_background_info")
        out["freeze"] = {"timescale": post(P, "/d/timescale", {"value": 0}), "video": post(P, "/d/video_control", {"pause": True})}
        # Background-only: keep BackGround visible, hide all menu/title UI
        for p in ["UI", "Title Screen", "UI/Popuholder/screenBlocker"]:
            post(P, "/d/activate", {"path": p, "active": False})
        post(P, "/d/activate", {"path": "BackGround", "active": True})
        time.sleep(0.4)
        for route, name in [("/d/menu_background", "menu_background_title"),
                            ("/d/ui_overlay", "menu_background_title_canvas")]:
            r = get(P, route)
            if r.get("ok"):
                out["frames"].append({"name": name, "route": route, **save(name, r["png_base64"])})
        # settings background: UI hidden, same BackGround; replay settings panel while UI hidden is not needed
        post(P, "/d/activate", {"path": "UI", "active": True})
        post(P, "/d/normalize_ui", {"root": "UI", "alpha": 1})
        post(P, "/d/press_button", {"name": "Settings"})
        time.sleep(0.5)
        for p in ["UI", "Title Screen", "UI/Popuholder/screenBlocker"]:
            post(P, "/d/activate", {"path": p, "active": False})
        post(P, "/d/activate", {"path": "BackGround", "active": True})
        time.sleep(0.3)
        r = get(P, "/d/menu_background")
        if r.get("ok"):
            out["frames"].append({"name": "menu_background_settings", "route": "/d/menu_background",
                                  **save("menu_background_settings", r["png_base64"])})
        out["ui_hidden_method"] = "deactivate UI, Title Screen, UI/Popuholder/screenBlocker; BackGround kept active"
        out["paused"] = "Time.timeScale=0 + VideoPlayer.Pause()"
        (OUT / "manifest.json").write_text(json.dumps(out, indent=2), encoding="utf-8")
        print(json.dumps(out, indent=1)[:6000])
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

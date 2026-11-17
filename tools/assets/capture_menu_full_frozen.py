#!/usr/bin/env python3
"""capture_menu_full_frozen.py - frozen full menu title/settings frames (background+UI) for exact A/B."""
from __future__ import annotations

import base64
import ctypes
import ctypes.wintypes as wt
import json
import subprocess
import time
from pathlib import Path

import httpx
from PIL import Image

ROOT = Path(r"Z:\RUSTibites")
GAME = Path(r"C:\Program Files (x86)\Steam\steamapps\common\The Bibites")
EXE = GAME / "The Bibites.exe"
PLUGIN_DIR = GAME / "BepInEx/plugins/ParityDump"
PLUGIN_DLL = ROOT / "testdata/parity/plugin/bin/ParityDump.dll"
OUT = ROOT / "assets/game/reference/functional_ui_native/menu_background/frozen_full"


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
    rec = {"schema": "rustibites.ui.menu_full_frozen.v1", "frames": []}
    try:
        assert wait("http://127.0.0.1:17800/ping", 180)
        assert wait(P + "/d/ping", 60)
        hide(proc.pid); time.sleep(2.0)
        httpx.post(P + "/d/activate", json={"path": "UI", "active": True}, timeout=30)
        httpx.post(P + "/d/normalize_ui", json={"root": "UI", "alpha": 1}, timeout=30)
        rec["freeze"] = {"timescale": httpx.post(P + "/d/timescale", json={"value": 0}, timeout=30).json(),
                         "video": httpx.post(P + "/d/video_control", json={"pause": True}, timeout=30).json()}
        # hide title/loading overlays but keep UI + BackGround
        for p in ("Title Screen", "LoadingPanel", "UI/Popuholder/screenBlocker"):
            httpx.post(P + "/d/activate", json={"path": p, "active": False}, timeout=30)
        time.sleep(0.4)
        r = httpx.get(P + "/d/ui_overlay", timeout=120).json()
        if r.get("ok"):
            png = base64.b64decode(r["png_base64"]); p = OUT / "title_full_frozen_1920x1080.png"; p.write_bytes(png)
            rec["frames"].append({"name": "title_full_frozen", "file": str(p.relative_to(ROOT).as_posix()),
                                  "sha256": __import__("hashlib").sha256(png).hexdigest()})
        httpx.post(P + "/d/press_button", json={"path": "UI/WelcomeScreen/Main Holder/ButtonsPanel/Settings"}, timeout=30)
        time.sleep(0.6)
        r = httpx.get(P + "/d/ui_overlay", timeout=120).json()
        if r.get("ok"):
            png = base64.b64decode(r["png_base64"]); p = OUT / "settings_full_frozen_1920x1080.png"; p.write_bytes(png)
            rec["frames"].append({"name": "settings_full_frozen", "file": str(p.relative_to(ROOT).as_posix()),
                                  "sha256": __import__("hashlib").sha256(png).hexdigest()})
        rec["route"] = "/d/ui_overlay (full canvas composite, not ui_only)"
        rec["state"] = ("Time.timeScale=0, VideoPlayer.Pause(); Title Screen/LoadingPanel/screenBlocker hidden; "
                        "UI+BackGround active; Settings pressed after freeze")
        (OUT / "manifest.json").write_text(json.dumps(rec, indent=2), encoding="utf-8")
        print(json.dumps(rec, indent=1)[:3000])
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

#!/usr/bin/env python3
"""dump_ui_geometry.py - native 1920 Canvas/root/panel screen semantics."""
from __future__ import annotations

import ctypes
import ctypes.wintypes as wt
import json
import subprocess
import sys
import time
from pathlib import Path

import httpx

ROOT = Path(r"Z:\RUSTibites")
GAME = Path(r"C:\Program Files (x86)\Steam\steamapps\common\The Bibites")
EXE = GAME / "The Bibites.exe"
PLUGIN_DIR = GAME / "BepInEx/plugins/ParityDump"
PLUGIN_DLL = ROOT / "testdata/parity/plugin/bin/ParityDump.dll"
SAVE = ROOT / "testdata/parity/default/rust_seed_600.zip"
OUT = ROOT / "assets/game/reference/functional_ui_native/ui_geometry.json"


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
    PLUGIN_DIR.mkdir(parents=True, exist_ok=True)
    subprocess.run(["copy", str(PLUGIN_DLL), str(PLUGIN_DIR / "ParityDump.dll")], shell=True, check=True,
                   stdout=subprocess.DEVNULL)
    si = subprocess.STARTUPINFO(); si.dwFlags |= subprocess.STARTF_USESHOWWINDOW; si.wShowWindow = 0
    proc = subprocess.Popen([str(EXE), "-screen-width", "1920", "-screen-height", "1080", "-window-mode", "windowed"],
                            cwd=str(GAME), startupinfo=si, creationflags=subprocess.CREATE_NEW_PROCESS_GROUP,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    P = "http://127.0.0.1:17801"
    out = {}
    try:
        assert wait("http://127.0.0.1:17800/ping", 180)
        assert wait(P + "/d/ping", 60)
        hide(proc.pid)
        time.sleep(1.5)
        httpx.post(P + "/d/activate", json={"path": "UI", "active": True}, timeout=30)
        httpx.post(P + "/d/normalize_ui", json={"root": "UI", "alpha": 1}, timeout=30)
        time.sleep(0.8)
        out["title"] = httpx.get(P + "/d/ui_geometry", timeout=120).json()
        httpx.post(P + "/d/press_button", json={"name": "Settings"}, timeout=30)
        time.sleep(0.6)
        out["settings"] = httpx.get(P + "/d/ui_geometry", timeout=120).json()
        sys.path.insert(0, str(ROOT / "tools" / "parity"))
        import capture as cap
        B, _, _, _ = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
        c = B(port=17800); c.load_world(SAVE); c.set_time(paused=True)
        time.sleep(0.5)
        out["world"] = httpx.get(P + "/d/ui_geometry", timeout=120).json()
        out["schema"] = "rustibites.ui.native_geometry.v1"
        OUT.write_text(json.dumps(out, indent=2), encoding="utf-8")
        print(json.dumps({"title_screen": out["title"].get("screen"),
                          "title_canvases": [(c.get("path"), c.get("render_mode"), c.get("screen_bbox"), c.get("canvas_scaler")) for c in out["title"].get("canvases") or []],
                          "title_keys": [(k.get("path"), k.get("screen_bbox")) for k in out["title"].get("key_rects") or [] if "ButtonsPanel" in (k.get("path") or "")][:3]}, indent=1)[:4000])
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

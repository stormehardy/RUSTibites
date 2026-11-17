#!/usr/bin/env python3
"""capture_native_inspector_tabs.py - correct native 1920 per-tab inspector UI refs + HUD toolbar."""
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
SAVE = ROOT / "testdata/parity/default/rust_seed_600.zip"
OUT = ROOT / "assets/game/reference/functional_ui_native/in_world/inspector"
HUD = ROOT / "assets/game/reference/functional_ui_native/in_world"
PANELS = {
    "stats": "UICanvas/MainUI/Layer1/Panels/BibitePanels/StatsPanel",
    "genes": "UICanvas/MainUI/Layer1/Panels/BibitePanels/GenesPanel",
    "biology": "UICanvas/MainUI/Layer1/Panels/BibitePanels/BiologyPanel",
    "brain": "UICanvas/MainUI/Layer1/Panels/BibitePanels/BrainPanel",
}


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


def save(b64, path):
    png = base64.b64decode(b64)
    p = Path(path); p.write_bytes(png)
    im = Image.open(p).convert("RGBA"); a = np.asarray(im); al = a[:, :, 3] > 0
    return {"file": str(p.relative_to(ROOT).as_posix()), "sha256": hashlib.sha256(png).hexdigest(),
            "alpha_fraction": float(al.mean()), "alpha_pixels": int(al.sum()),
            "unique_opaque_colors": int(len(np.unique(a[:, :, :3][al].reshape(-1, 3), axis=0)) if al.any() else 0)}


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
    out = {"schema": "rustibites.ui.native_inspector_tabs.v1", "sessions": {}, "hud": {}}
    try:
        assert wait("http://127.0.0.1:17800/ping", 180)
        assert wait(P + "/d/ping", 60)
        hide(proc.pid)
        sys.path.insert(0, str(ROOT / "tools" / "parity"))
        import capture as cap
        B, _, _, _ = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
        c = B(port=17800); c.load_world(SAVE); c.set_time(paused=True)
        httpx.post(P + "/d/select_bibite", json={"x": -5.360003, "y": 831.0031}, timeout=30)
        httpx.post(P + "/d/press_button", json={"name": "StatsButton"}, timeout=30)
        time.sleep(0.5)
        # capture each panel with others hidden
        for tab, path in PANELS.items():
            for p in PANELS.values():
                httpx.post(P + "/d/activate", json={"path": p, "active": (p == path)}, timeout=30)
            time.sleep(0.35)
            ui = httpx.get(P + "/d/ui", timeout=120).json()
            ov = httpx.get(P + "/d/ui_overlay", timeout=120).json()
            rec = {"panel": path, "ui_nodes": ui.get("node_count"), "screen": ui.get("screen")}
            if ov.get("ok"):
                rec["image"] = save(ov["png_base64"], OUT / f"native_inspector_{tab}_ui_only_1920x1080.png")
            out["sessions"][tab] = rec
        # HUD/toolbar with inspector closed
        for p in PANELS.values():
            httpx.post(P + "/d/activate", json={"path": p, "active": False}, timeout=30)
        time.sleep(0.3)
        ui = httpx.get(P + "/d/ui", timeout=120).json()
        ov = httpx.get(P + "/d/ui_overlay", timeout=120).json()
        out["hud"] = {"ui_nodes": ui.get("node_count"), "screen": ui.get("screen")}
        if ov.get("ok"):
            out["hud"]["image"] = save(ov["png_base64"], HUD / "native_world_toolbar_hud_1920x1080.png")
        out["note"] = "Per-tab panels activated directly by native path after StatsButton; overlay is native 1920 UI-only."
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

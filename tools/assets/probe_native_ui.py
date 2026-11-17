#!/usr/bin/env python3
"""probe_native_ui.py - standalone native-1920 hidden game probe for UI reference capture.

Launches The Bibites directly (NOT BibitesAPI, whose hidden launcher hardcodes 320x240) with
`-screen-width 1920 -screen-height 1080 -window-mode windowed`, hides its window via Win32, waits for the
BepInEx bridge + parity plugin, captures title/settings/world UI overlays at native 1920, and reports
sharpness metrics (unique colors, down/up residual) before bulk capture.
"""
from __future__ import annotations

import ctypes
import ctypes.wintypes as wt
import json
import os
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
OUT = ROOT / "assets/game/reference/functional_ui_native"


def hide_window(pid: int) -> dict:
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    found = []
    EnumWindowsProc = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)

    def cb(hwnd, lparam):
        p = wt.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(p))
        if p.value == pid:
            user32.ShowWindow(hwnd, 0)  # SW_HIDE
            user32.SetWindowPos(hwnd, 0, -32000, -32000, 0, 0, 0x0001 | 0x0004)  # NOSIZE|NOZORDER
            found.append(int(hwnd))
        return True

    user32.EnumWindows(EnumWindowsProc(cb), 0)
    return {"hidden_hwnds": found}


def wait(url: str, timeout: float) -> bool:
    end = time.time() + timeout
    while time.time() < end:
        try:
            if httpx.get(url, timeout=2).status_code == 200:
                return True
        except Exception:
            pass
        time.sleep(0.25)
    return False


def sharpness(png: bytes):
    img = Image.open(__import__("io").BytesIO(png)).convert("RGBA")
    arr = np.asarray(img)
    alpha = arr[:, :, 3] > 0
    rgb = arr[:, :, :3]
    down = img.resize((img.width // 2, img.height // 2), Image.BICUBIC).resize(img.size, Image.BICUBIC)
    drum = np.asarray(down.convert("RGB")).astype(np.int16)
    res = np.abs(rgb.astype(np.int16) - drum)[alpha] if alpha.any() else np.abs(rgb.astype(np.int16) - drum)
    unique = len(np.unique(rgb[alpha].reshape(-1, 3), axis=0)) if alpha.any() else len(np.unique(rgb.reshape(-1, 3), axis=0))
    return {"size": img.size, "alpha_pixels": int(alpha.sum()), "unique_opaque_colors": int(unique),
            "downup_mae": float(res.mean()) if res.size else None}


def post(url, body=None):
    return httpx.post(url, json=body or {}, timeout=120).json()


def get(url):
    return httpx.get(url, timeout=120).json()


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    if any(Path(PLUGIN_DIR).glob("*.dll")):
        pass
    else:
        PLUGIN_DIR.mkdir(parents=True, exist_ok=True)
        subprocess.run(["copy", str(PLUGIN_DLL), str(PLUGIN_DIR / "ParityDump.dll")], shell=True, check=True)
    report = {"standalone_launch": True, "args": ["-screen-width", "1920", "-screen-height", "1080",
                                                 "-window-mode", "windowed"], "steps": []}
    si = subprocess.STARTUPINFO()
    si.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    si.wShowWindow = 0
    proc = subprocess.Popen([str(EXE), "-screen-width", "1920", "-screen-height", "1080", "-window-mode", "windowed"],
                            cwd=str(GAME), startupinfo=si, creationflags=subprocess.CREATE_NEW_PROCESS_GROUP,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    report["pid"] = proc.pid
    try:
        report["bridge_ready"] = wait("http://127.0.0.1:17800/ping", 180)
        report["plugin_ready"] = wait("http://127.0.0.1:17801/d/ping", 60)
        report["hide"] = hide_window(proc.pid)
        report["status"] = get("http://127.0.0.1:17800/status") if report["bridge_ready"] else None
        # title (wait for menu/loading to settle, then activate/normalize the Welcome UI)
        time.sleep(2.0)
        dump = get("http://127.0.0.1:17801/d/ui")
        report["screen"] = dump.get("screen")
        post("http://127.0.0.1:17801/d/activate", {"path": "UI", "active": True})
        post("http://127.0.0.1:17801/d/normalize_ui", {"root": "UI", "alpha": 1})
        post("http://127.0.0.1:17801/d/activate", {"path": "Title Screen", "active": False})
        post("http://127.0.0.1:17801/d/activate", {"path": "LoadingPanel", "active": False})
        hide = post("http://127.0.0.1:17801/d/hide_fullscreen_graphics", {"action": "hide"})
        report["hide_fullscreen"] = hide
        report["hide_candidates"] = hide.get("candidates")
        time.sleep(1.0)
        ui_dump = get("http://127.0.0.1:17801/d/ui")
        report["active_canvases"] = [{"name": c.get("name"), "path": c.get("path"), "active": c.get("active_in_hierarchy")}
                                    for c in (ui_dump.get("canvases") or []) if c.get("active_in_hierarchy")]
        ov_full = get("http://127.0.0.1:17801/d/ui_overlay")
        if ov_full.get("ok"):
            png = __import__("base64").b64decode(ov_full["png_base64"])
            p = OUT / "probe_title_full_overlay_1920x1080.png"
            p.write_bytes(png)
            report.setdefault("extra", []).append({"step": "title_full_overlay", "png": str(p), "sharpness": sharpness(png)})
        post("http://127.0.0.1:17801/d/activate", {"path": "BackGround", "active": False})
        time.sleep(0.3)
        ov_bg = get("http://127.0.0.1:17801/d/ui_overlay")
        if ov_bg.get("ok"):
            png = __import__("base64").b64decode(ov_bg["png_base64"])
            p = OUT / "probe_title_bg_off_1920x1080.png"
            p.write_bytes(png)
            report.setdefault("extra", []).append({"step": "title_background_off", "png": str(p), "sharpness": sharpness(png)})
        post("http://127.0.0.1:17801/d/activate", {"path": "BackGround", "active": True})
        time.sleep(0.2)
        ov = get("http://127.0.0.1:17801/d/ui_overlay_ui_only")
        if ov.get("ok"):
            png = __import__("base64").b64decode(ov["png_base64"])
            p = OUT / "probe_title_native_1920x1080.png"
            p.write_bytes(png)
            report["steps"].append({"step": "title", "png": str(p), "sharpness": sharpness(png)})
        post("http://127.0.0.1:17801/d/press_button", {"name": "Settings"})
        time.sleep(0.7)
        ov = get("http://127.0.0.1:17801/d/ui_overlay_ui_only")
        if ov.get("ok"):
            png = __import__("base64").b64decode(ov["png_base64"])
            p = OUT / "probe_settings_native_1920x1080.png"
            p.write_bytes(png)
            report["steps"].append({"step": "settings", "png": str(p), "sharpness": sharpness(png)})
        post("http://127.0.0.1:17801/d/hide_fullscreen_graphics", {"action": "restore"})
        # in-world via bridge client
        sys.path.insert(0, str(ROOT / "tools" / "parity"))
        import capture as cap
        B, _, _, _ = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
        c = B(port=17800)
        c.load_world(SAVE)
        time.sleep(0.5)
        ov = get("http://127.0.0.1:17801/d/ui_overlay")
        if ov.get("ok"):
            png = __import__("base64").b64decode(ov["png_base64"])
            p = OUT / "probe_world_native_1920x1080.png"
            p.write_bytes(png)
            report["steps"].append({"step": "world", "png": str(p), "sharpness": sharpness(png)})
        # full screen capture (native window framebuffer, if available)
        sc = get("http://127.0.0.1:17801/d/screen_capture")
        if sc.get("ok"):
            png = __import__("base64").b64decode(sc["png_base64"])
            p = OUT / "probe_world_full_native_1920x1080.png"
            p.write_bytes(png)
            report["steps"].append({"step": "world_full", "png": str(p), "sharpness": sharpness(png)})
        (OUT / "probe_report.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
        print(json.dumps(report, indent=2)[:6000])
    finally:
        try:
            proc.terminate()
            proc.wait(timeout=15)
        except Exception:
            try:
                subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"], check=False,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            except Exception:
                pass
        try:
            if PLUGIN_DIR.exists():
                for f in PLUGIN_DIR.glob("*"):
                    f.unlink()
                PLUGIN_DIR.rmdir()
        except Exception:
            pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

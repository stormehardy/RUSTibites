#!/usr/bin/env python3
"""dump_tmp_fonts.py - standalone hidden launch: dump TMP font assets + isolate native menu UI-only.

Launches The Bibites directly at native 1920 (window hidden), deploys the parity plugin, waits for bridge,
calls /d/font_assets and saves per-font JSON + atlas PNGs under assets/game/ui_fonts/, then deactivates the
menu screenBlocker/fullscreen graphics and captures /d/ui_overlay_ui_only for title/settings to verify
transparent native menu UI. No BibitesAPI launcher.
"""
from __future__ import annotations

import base64
import ctypes
import ctypes.wintypes as wt
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
OUT = ROOT / "assets/game/ui_fonts"
REF = ROOT / "assets/game/reference/functional_ui_native"


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


def pget(base, path):
    return httpx.get(base + path, timeout=120).json()


def ppost(base, path, body=None):
    return httpx.post(base + path, json=body or {}, timeout=120).json()


def sharp(png):
    im = Image.open(__import__("io").BytesIO(png)).convert("RGBA")
    a = np.asarray(im); alpha = a[:, :, 3] > 0
    return {"size": im.size, "alpha_pixels": int(alpha.sum()), "alpha_fraction": float(alpha.mean()),
            "unique_opaque_colors": int(len(np.unique(a[:, :, :3][alpha].reshape(-1, 3), axis=0)) if alpha.any() else 0)}


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    REF.mkdir(parents=True, exist_ok=True)
    PLUGIN_DIR.mkdir(parents=True, exist_ok=True)
    subprocess.run(["copy", str(PLUGIN_DLL), str(PLUGIN_DIR / "ParityDump.dll")], shell=True, check=True,
                   stdout=subprocess.DEVNULL)
    si = subprocess.STARTUPINFO(); si.dwFlags |= subprocess.STARTF_USESHOWWINDOW; si.wShowWindow = 0
    proc = subprocess.Popen([str(EXE), "-screen-width", "1920", "-screen-height", "1080", "-window-mode", "windowed"],
                            cwd=str(GAME), startupinfo=si, creationflags=subprocess.CREATE_NEW_PROCESS_GROUP,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    report = {"pid": proc.pid, "fonts": [], "menu_isolation": []}
    try:
        report["bridge"] = wait("http://127.0.0.1:17800/ping", 180)
        report["plugin"] = wait("http://127.0.0.1:17801/d/ping", 60)
        report["hidden"] = hide(proc.pid)
        time.sleep(2.0)
        fonts = pget("http://127.0.0.1:17801", "/d/font_assets")
        report["font_count"] = fonts.get("count")
        if fonts.get("ok"):
            for f in fonts.get("fonts") or []:
                name = f.get("name") or "unnamed"
                safe = "".join(ch if ch.isalnum() or ch in "._-" else "_" for ch in name)
                (OUT / f"{safe}.json").write_text(json.dumps(f, indent=2), encoding="utf-8")
                for i, t in enumerate(f.get("atlasTextures") or []):
                    if t.get("png_base64"):
                        (OUT / f"{safe}_atlas{i}.png").write_bytes(base64.b64decode(t["png_base64"]))
                report["fonts"].append({"name": name, "glyphs": len(f.get("glyphTable") or []),
                                        "characters": len(f.get("characterTable") or []),
                                        "atlas_count": len(f.get("atlasTextures") or [])})
        # menu isolation diagnostics
        ppost("http://127.0.0.1:17801", "/d/activate", {"path": "UI", "active": True})
        ppost("http://127.0.0.1:17801", "/d/normalize_ui", {"root": "UI", "alpha": 1})
        ppost("http://127.0.0.1:17801", "/d/activate", {"path": "UI/Popuholder/screenBlocker", "active": False})
        ppost("http://127.0.0.1:17801", "/d/activate", {"path": "Title Screen", "active": False})
        ppost("http://127.0.0.1:17801", "/d/hide_fullscreen_graphics", {"action": "hide"})
        time.sleep(0.8)
        ov = pget("http://127.0.0.1:17801", "/d/ui_overlay_ui_only")
        if ov.get("ok"):
            png = base64.b64decode(ov["png_base64"])
            p = REF / "menu_ui_only_native_1920x1080.png"; p.write_bytes(png)
            report["menu_isolation"].append({"step": "title_ui_only", "png": str(p), "sharpness": sharp(png)})
        ppost("http://127.0.0.1:17801", "/d/activate", {"path": "UI", "active": True})
        ppost("http://127.0.0.1:17801", "/d/press_button", {"name": "Settings"})
        time.sleep(0.7)
        ov = pget("http://127.0.0.1:17801", "/d/ui_overlay_ui_only")
        if ov.get("ok"):
            png = base64.b64decode(ov["png_base64"])
            p = REF / "settings_ui_only_native_1920x1080.png"; p.write_bytes(png)
            report["menu_isolation"].append({"step": "settings_ui_only", "png": str(p), "sharpness": sharp(png)})
        (OUT / "fonts_report.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
        print(json.dumps(report, indent=1)[:6000])
    finally:
        try:
            proc.terminate(); proc.wait(timeout=15)
        except Exception:
            subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"], check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for pth in PLUGIN_DIR.glob("*"):
            try: pth.unlink()
            except Exception: pass
        try: PLUGIN_DIR.rmdir()
        except Exception: pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

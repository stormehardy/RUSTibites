#!/usr/bin/env python3
"""capture_native_ui_dump.py - native 1920 runtime UI node dumps + UI-on-black menu refs.

Standalone hidden 1920 launch; captures /d/ui node trees at native screen dimensions for title/menu,
Settings, and one in-world panel, plus UI-on-black menu images (BackGround/Title Screen hidden). Writes
assets/game/ui_runtime_native.json.gz, index/manifest, and native menu refs. First-pass verification for
the old 320-based runtime atlas.
"""
from __future__ import annotations

import base64
import ctypes
import ctypes.wintypes as wt
import gzip
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
OUT = ROOT / "assets/game"
REF = OUT / "reference/functional_ui_native/menu_ui"


def hide(pid):
    u = ctypes.WinDLL("user32", use_last_error=True)
    cb_t = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
    found = []

    def cb(hwnd, _):
        p = wt.DWORD(); u.GetWindowThreadProcessId(hwnd, ctypes.byref(p))
        if p.value == pid:
            u.ShowWindow(hwnd, 0); u.SetWindowPos(hwnd, 0, -32000, -32000, 0, 0, 1 | 4); found.append(int(hwnd))
        return True
    u.EnumWindows(cb_t(cb), 0)
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


def save_png(b64, path):
    png = base64.b64decode(b64)
    Path(path).write_bytes(png)
    im = Image.open(io.BytesIO(png)).convert("RGBA")
    a = np.asarray(im)
    alpha = a[:, :, 3] > 0
    nonblack = (a[:, :, :3].max(axis=2) > 8) & alpha
    bbox = None
    ys, xs = np.nonzero(nonblack)
    if len(xs):
        bbox = [int(xs.min()), int(ys.min()), int(xs.max()), int(ys.max())]
    return {"file": str(Path(path).relative_to(ROOT).as_posix()), "size": im.size,
            "alpha_fraction": float(alpha.mean()), "nonblack_bbox": bbox,
            "nonblack_pixels": int(nonblack.sum())}


def find_node(dump, needle):
    def walk(n):
        if needle in (n.get("path") or ""):
            return n
        for k in n.get("children") or []:
            r = walk(k)
            if r:
                return r
        return None
    for c in dump.get("canvases") or []:
        r = walk(c)
        if r:
            return r
    return None


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
    sessions = {}
    report = {"pid": proc.pid, "screen": None, "sessions": {}, "geometry": {}}
    try:
        assert wait("http://127.0.0.1:17800/ping", 180)
        assert wait("http://127.0.0.1:17801/d/ping", 60)
        hide(proc.pid)
        # menu setup
        post("http://127.0.0.1:17801", "/d/activate", {"path": "UI", "active": True})
        post("http://127.0.0.1:17801", "/d/normalize_ui", {"root": "UI", "alpha": 1})
        time.sleep(1.0)
        title = get("http://127.0.0.1:17801", "/d/ui")
        report["screen"] = title.get("screen")
        sessions["native_menu_title"] = title
        # UI-on-black: hide background/title canvases
        post("http://127.0.0.1:17801", "/d/activate", {"path": "BackGround", "active": False})
        post("http://127.0.0.1:17801", "/d/activate", {"path": "Title Screen", "active": False})
        time.sleep(0.5)
        ov = get("http://127.0.0.1:17801", "/d/ui_overlay")
        if ov.get("ok"):
            report["sessions"]["native_menu_title_ui_on_black"] = save_png(
                ov["png_base64"], REF / "native_menu_title_ui_on_black_1920x1080.png")
        # settings: restore menu UI canvas only, click Settings
        post("http://127.0.0.1:17801", "/d/activate", {"path": "UI", "active": True})
        post("http://127.0.0.1:17801", "/d/press_button", {"name": "Settings"})
        time.sleep(0.6)
        settings = get("http://127.0.0.1:17801", "/d/ui")
        sessions["native_menu_settings"] = settings
        post("http://127.0.0.1:17801", "/d/activate", {"path": "BackGround", "active": False})
        post("http://127.0.0.1:17801", "/d/activate", {"path": "Title Screen", "active": False})
        time.sleep(0.4)
        ov = get("http://127.0.0.1:17801", "/d/ui_overlay")
        if ov.get("ok"):
            report["sessions"]["native_menu_settings_ui_on_black"] = save_png(
                ov["png_base64"], REF / "native_menu_settings_ui_on_black_1920x1080.png")
        # in-world
        sys.path.insert(0, str(ROOT / "tools" / "parity"))
        import capture as cap
        B, _, _, _ = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
        c = B(port=17800)
        c.load_world(SAVE); c.set_time(paused=True)
        time.sleep(0.5)
        world = get("http://127.0.0.1:17801", "/d/ui")
        sessions["native_world_ui"] = world
        ov = get("http://127.0.0.1:17801", "/d/ui_overlay")
        if ov.get("ok"):
            report["sessions"]["native_world_ui_overlay"] = save_png(
                ov["png_base64"], REF / "native_world_ui_overlay_1920x1080.png")
        # geometry: menu ButtonsPanel rect vs screenshot nonblack bbox
        bp = find_node(title, "UI/WelcomeScreen/Main Holder/ButtonsPanel")
        report["geometry"]["buttons_panel_node"] = {"path": bp.get("path"), "rect": bp.get("rect")} if bp else None
        report["geometry"]["title_ui_on_black"] = report["sessions"].get("native_menu_title_ui_on_black")
        # write native runtime atlas
        doc = {"schema": "rustibites.assets.ui_runtime.v1", "generatedBy": "tools/assets/capture_native_ui_dump.py",
               "capture": {"method": "standalone hidden native 1920 game; parity plugin /d/ui",
                           "screen": report["screen"], "native": True},
               "sessions": sessions}
        with gzip.open(OUT / "ui_runtime_native.json.gz", "wb", compresslevel=9) as g:
            g.write(json.dumps(doc, indent=1).encode("utf-8"))
        index = {"schema": "rustibites.assets.ui_runtime_native.index.v1",
                 "files": {"runtime": "assets/game/ui_runtime_native.json.gz"},
                 "capture": doc["capture"],
                 "sessions": {k: {"screen": v.get("screen"), "node_count": v.get("node_count"),
                                  "canvases": len(v.get("canvases") or [])} for k, v in sessions.items()},
                 "consumer": "native runtime UI layout atlas; old ui_runtime.json.gz stays historical"}
        (OUT / "ui_runtime_native.index.json").write_text(json.dumps(index, indent=2), encoding="utf-8")
        (REF / "manifest.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
        print(json.dumps(report, indent=2)[:6000])
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

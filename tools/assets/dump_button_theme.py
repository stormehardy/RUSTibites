#!/usr/bin/env python3
"""dump_button_theme.py - standalone hidden toolbar/top-right button theme + missing sprite dump."""
from __future__ import annotations

import base64
import ctypes
import ctypes.wintypes as wt
import io
import json
import re
import subprocess
import sys
import time
from pathlib import Path

import httpx
from PIL import Image

ROOT = Path(r"Z:\RUSTibites")
GAME = Path(r"C:\Program Files (x86)\Steam\steamapps\common\The Bibites")
EXE = GAME / "The Bibites.exe"
PLUGIN_DIR = GAME / "BepInEx/plugins/ParityDump"
PLUGIN_DLL = ROOT / "testdata/parity/plugin/bin/ParityDump.dll"
SAVE = ROOT / "testdata/parity/default/rust_seed_600.zip"
OUT = ROOT / "assets/game/reference/functional_ui_native/in_world/button_theme"


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


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    PLUGIN_DIR.mkdir(parents=True, exist_ok=True)
    subprocess.run(["copy", str(PLUGIN_DLL), str(PLUGIN_DIR / "ParityDump.dll")], shell=True, check=True,
                   stdout=subprocess.DEVNULL)
    si = subprocess.STARTUPINFO(); si.dwFlags |= subprocess.STARTF_USESHOWWINDOW; si.wShowWindow = 0
    proc = subprocess.Popen([str(EXE), "-screen-width", "1920", "-screen-height", "1080", "-window-mode", "windowed"],
                            cwd=str(GAME), startupinfo=si, creationflags=subprocess.CREATE_NEW_PROCESS_GROUP,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        assert wait("http://127.0.0.1:17800/ping", 180)
        assert wait("http://127.0.0.1:17801/d/ping", 60)
        hide(proc.pid)
        sys.path.insert(0, str(ROOT / "tools" / "parity"))
        import capture as cap
        B, _, _, _ = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
        c = B(port=17800)
        c.load_world(SAVE); c.set_time(paused=True)
        data = httpx.get("http://127.0.0.1:17801/d/button_theme", timeout=120).json()
        (OUT / "button_theme_raw.json").write_text(json.dumps(data, indent=2), encoding="utf-8")
        # save textures and crop missing sprites
        texdir = OUT / "textures"; texdir.mkdir(exist_ok=True)
        spritedir = OUT / "sprites"; spritedir.mkdir(exist_ok=True)
        texpng = data.get("textures_png") or {}
        tex_imgs = {}
        for name, b64 in texpng.items():
            safe = re.sub(r"[^A-Za-z0-9._-]+", "_", name)
            p = texdir / f"{safe}.png"
            p.write_bytes(base64.b64decode(b64))
            tex_imgs[name] = Image.open(p).convert("RGBA")
        # extracted sprite names present in assets
        extracted = {p.stem for p in (ROOT / "assets/game/sprites").glob("*.png")} if (ROOT / "assets/game/sprites").exists() else set()
        missing = []
        crop_records = []
        seen = set()
        for b in data.get("buttons") or []:
            g = b.get("graphic") or {}
            spr = g.get("sprite")
            if not spr:
                continue
            if spr not in extracted:
                if spr not in seen:
                    seen.add(spr); missing.append(spr)
                    tex = tex_imgs.get(g.get("texture") or "")
                    r = g.get("sprite_rect") or {}
                    if tex is not None and all(k in r for k in ("x", "y", "w", "h")):
                        x, y, w, h = (int(round(r[k])) for k in ("x", "y", "w", "h"))
                        top = tex.height - y - h
                        crop = tex.crop((x, top, x + w, top + h))
                        safe = re.sub(r"[^A-Za-z0-9._-]+", "_", spr)
                        cp = spritedir / f"{safe}.png"
                        crop.save(cp)
                        crop_records.append({"sprite": spr, "path": str(cp.relative_to(ROOT).as_posix()),
                                             "rect": r, "texture": g.get("texture"), "size": crop.size})
        out = {"schema": "rustibites.ui.button_theme.v1",
               "source": "standalone hidden native 1920 game /d/button_theme",
               "buttons": data.get("buttons"), "active_images": data.get("active_images"),
               "missing_from_extracted": missing, "sprite_crops": crop_records,
               "textures": {k: f"textures/{re.sub(r'[^A-Za-z0-9._-]+','_',k)}.png" for k in texpng},
               "note": "Button.colors/targetGraphic/Image.color/sprite/texture/material/CanvasGroup alpha included; "
                       "missing sprite crops are dumped from the live sprite texture rect."}
        (OUT / "button_theme.json").write_text(json.dumps(out, indent=2), encoding="utf-8")
        print(json.dumps({"buttons": len(out["buttons"] or []), "active_images": len(out["active_images"] or []),
                          "textures": len(texpng), "missing": missing, "crops": [c["sprite"] for c in crop_records]}, indent=1))
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

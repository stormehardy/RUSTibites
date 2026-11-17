#!/usr/bin/env python3
"""capture_native_runtime_bulk.py - native 1920 /d/ui dumps for all 60 runtime sessions.

Standalone hidden native-1920 launch (bypasses BibitesAPI 320x240 launcher). Menu sessions reset/activate the
Welcome UI and the relevant panel/page, hide BackGround/Title Screen and capture a native UI-on-black PNG;
in-world sessions load rust_seed_600 paused, open panels/inspector tabs, and capture /d/ui + transparent
world UI-only overlays. Writes assets/game/ui_runtime_native.json.gz + index/manifest; old 320 atlas stays
historical. Navigation follows the existing 60 session names; node dumps are always the full native tree.
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
MENU_REF = OUT / "reference/functional_ui_native/menu_ui"
WORLD_REF = OUT / "reference/functional_ui_native/in_world"

MENU = [
    ("main_menu", "UI"), ("main_menu_title", "UI"), ("main_menu_functional_title", "UI"),
    ("main_menu_load_game", "UI/LoadGamePanel"), ("main_menu_new_game", "UI/NewGameScenarios"),
    ("main_menu_bibite_library", "UI/BibiteSelector"), ("main_menu_achievements", "UI/WelcomeScreen/AchievementsPanel"),
    ("main_menu_additional_tools", "UI/WelcomeScreen/ToolsPanel"), ("main_menu_credits", "UI/WelcomeScreen/CreditsPanel"),
    ("main_menu_new_panel", "UI/NewPanel"), ("main_menu_workshop", "UI/WorkshopItemsPanel"),
    ("main_menu_scenario_editor", "UI/ScenarioEditor"),
    ("main_menu_settings", "UI/SettingsPanel"), ("main_menu_settings_graphics", "UI/SettingsPanel"),
    ("main_menu_settings_gameplay", "UI/SettingsPanel"), ("main_menu_settings_hotkeys", "UI/SettingsPanel"),
    ("settings_page_graphics_controls", "UI/SettingsPanel"), ("settings_page_gameplay_controls", "UI/SettingsPanel"),
    ("settings_page_hotkeys_controls", "UI/SettingsPanel"),
    ("main_menu_bibite_editor_genes", "UI/BibiteSelector/BibiteEditor"),
    ("main_menu_bibite_editor_brain", "UI/BibiteSelector/BibiteEditor"),
    ("library_selector_flow", "UI/BibiteSelector"), ("library_create_new", "UI/BibiteSelector"),
    ("library_editor_genes_flow", "UI/BibiteSelector/BibiteEditor"),
    ("library_editor_brain_flow", "UI/BibiteSelector/BibiteEditor"),
    ("library_editor_brain_after_add", "UI/BibiteSelector/BibiteEditor"),
    ("newgame_scenario_options", "UI/NewGameScenarios"), ("newgame_new_scenario_button", "UI/NewGameScenarios"),
    ("newgame_new_challenge_button", "UI/NewGameScenarios"),
    ("scenario_editor_flow", "UI/ScenarioEditor"), ("scenario_editor_zones", "UI/ScenarioEditor"),
    ("scenario_editor_ChallengeOptions", "UI/ScenarioEditor"), ("scenario_editor_sim", "UI/ScenarioEditor"),
    ("challenge_selector", "UI/NewGameScenarios"), ("challenge_scenario_panel", "UI/NewGameScenarios"),
    ("challenge_champion_selection", "UI/NewGameScenarios/ScenarioSelector/Holder/RightPanels/RightGroup/ChampionSelectionPanel"),
    ("challenge_parameters_editor", "UI/ScenarioEditor/Settings Tuner/Holder/Panels/ChallengeParameters"),
    ("workshop_panel", "UI/WorkshopItemsPanel"), ("workshop_edit_shared", "UI/WorkshopItemsPanel/EditSharedItemPanel"),
]
WORLD = ["default_world_newgame", "default_world", "default_world_inspector", "default_world_genes",
         "default_world_biology", "default_world_brain", "main_world_toolbar", "main_world_tool_placer",
         "main_world_tool_user_actions", "main_world_tool_view", "main_world_dynamic_parameters",
         "main_world_lineage", "main_world_information", "main_world_settings_panel",
         "world_dynamic_parameters_controls", "world_settings_controls", "lineage_species_panel",
         "lineage_species_tabs", "lineage_species_info", "lineage_species_genes", "lineage_species_brain"]
RESET = ["UI/SettingsPanel", "UI/LoadGamePanel", "UI/NewGameScenarios", "UI/BibiteSelector",
         "UI/WelcomeScreen/AchievementsPanel", "UI/WelcomeScreen/ToolsPanel", "UI/WelcomeScreen/CreditsPanel",
         "UI/NewPanel", "UI/WorkshopItemsPanel", "UI/ScenarioEditor"]


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


def pget(p, path):
    return httpx.get(p + path, timeout=120).json()


def ppost(p, path, body=None):
    return httpx.post(p + path, json=body or {}, timeout=120).json()


def save_png(b64, path):
    path = Path(path); path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(base64.b64decode(b64))
    im = Image.open(path).convert("RGBA"); a = np.asarray(im); al = a[:, :, 3] > 0
    nb = (a[:, :, :3].max(axis=2) > 8) & al
    ys, xs = np.nonzero(nb)
    return {"file": str(path.relative_to(ROOT).as_posix()), "size": list(im.size),
            "alpha_fraction": float(al.mean()), "nonblack_pixels": int(nb.sum()),
            "nonblack_bbox": [int(xs.min()), int(ys.min()), int(xs.max()), int(ys.max())] if len(xs) else None}


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    MENU_REF.mkdir(parents=True, exist_ok=True)
    WORLD_REF.mkdir(parents=True, exist_ok=True)
    PLUGIN_DIR.mkdir(parents=True, exist_ok=True)
    subprocess.run(["copy", str(PLUGIN_DLL), str(PLUGIN_DIR / "ParityDump.dll")], shell=True, check=True,
                   stdout=subprocess.DEVNULL)
    si = subprocess.STARTUPINFO(); si.dwFlags |= subprocess.STARTF_USESHOWWINDOW; si.wShowWindow = 0
    proc = subprocess.Popen([str(EXE), "-screen-width", "1920", "-screen-height", "1080", "-window-mode", "windowed"],
                            cwd=str(GAME), startupinfo=si, creationflags=subprocess.CREATE_NEW_PROCESS_GROUP,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    P = "http://127.0.0.1:17801"
    sessions = {}
    refs = {}
    try:
        assert wait("http://127.0.0.1:17800/ping", 180)
        assert wait(P + "/d/ping", 60)
        hide(proc.pid)
        time.sleep(1.5)
        ppost(P, "/d/activate", {"path": "UI", "active": True})
        ppost(P, "/d/normalize_ui", {"root": "UI", "alpha": 1})
        for name, path in MENU:
            for rp in RESET:
                ppost(P, "/d/activate", {"path": rp, "active": False})
            ppost(P, "/d/activate", {"path": "UI", "active": True})
            ppost(P, "/d/normalize_ui", {"root": "UI", "alpha": 1})
            ppost(P, "/d/activate", {"path": path, "active": True})
            if name.endswith("_graphics"):
                ppost(P, "/d/press_button", {"name": "GraphicsButton"})
            elif name.endswith("_gameplay"):
                ppost(P, "/d/press_button", {"name": "Gameplay Button"})
            elif name.endswith("_hotkeys"):
                ppost(P, "/d/press_button", {"name": "Controls Button"})
            elif name.endswith("_genes") or name == "library_editor_genes_flow":
                ppost(P, "/d/press_button", {"name": "GenesButton"})
            elif "brain" in name:
                ppost(P, "/d/press_button", {"name": "brainButton"})
            elif name == "newgame_new_scenario_button":
                ppost(P, "/d/press_button", {"name": "New Scenario Button"})
            elif name == "challenge_parameters_editor":
                ppost(P, "/d/press_button", {"name": "ChallengeOptions"})
            elif name == "scenario_editor_zones":
                ppost(P, "/d/press_button", {"name": "zones"})
            elif name == "scenario_editor_ChallengeOptions":
                ppost(P, "/d/press_button", {"name": "ChallengeOptions"})
            elif name == "scenario_editor_sim":
                ppost(P, "/d/press_button", {"name": "sim"})
            time.sleep(0.25)
            sessions[name] = pget(P, "/d/ui")
            # hide menu background/title, capture UI-on-black
            ppost(P, "/d/activate", {"path": "BackGround", "active": False})
            ppost(P, "/d/activate", {"path": "Title Screen", "active": False})
            time.sleep(0.2)
            ov = pget(P, "/d/ui_overlay")
            if ov.get("ok"):
                refs[name] = save_png(ov["png_base64"], MENU_REF / f"{name}_ui_on_black_1920x1080.png")
            ppost(P, "/d/activate", {"path": "BackGround", "active": True})
            ppost(P, "/d/activate", {"path": "Title Screen", "active": True})
        # world sessions
        sys.path.insert(0, str(ROOT / "tools" / "parity"))
        import capture as cap
        B, _, _, _ = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
        c = B(port=17800); c.load_world(SAVE); c.set_time(paused=True)
        for name in WORLD:
            if name in ("default_world_inspector", "default_world_genes", "default_world_biology", "default_world_brain"):
                ppost(P, "/d/select_bibite", {"x": -5.360003, "y": 831.0031})
                ppost(P, "/d/press_button", {"name": "StatsButton"})
                if name != "default_world_inspector":
                    btn = {"default_world_genes": "GeneButton", "default_world_biology": "BiologyButton",
                           "default_world_brain": "BrainButton"}[name]
                    ppost(P, "/d/press_button", {"name": btn})
            elif name.startswith("main_world_") or name.startswith("world_"):
                btn = {"main_world_toolbar": None, "main_world_tool_placer": "PlacerButton",
                       "main_world_tool_user_actions": "UserActions", "main_world_tool_view": "ViewButton",
                       "main_world_dynamic_parameters": "DynamicParametersButton",
                       "main_world_lineage": "AncestralLineageButton", "main_world_information": "InformationPanelButton",
                       "main_world_settings_panel": "SettingsButton", "world_dynamic_parameters_controls": "DynamicParametersButton",
                       "world_settings_controls": "SettingsButton"}.get(name)
                if btn:
                    ppost(P, "/d/press_button", {"name": btn})
            elif name.startswith("lineage_species_"):
                ppost(P, "/d/press_button", {"name": "AncestralLineageButton"})
                sp = {"lineage_species_tabs": "Species Tabs", "lineage_species_info": "Species Info Panel",
                      "lineage_species_genes": "Species Genes Panel", "lineage_species_brain": "SpeciesBrainPanel"}.get(name)
                if sp:
                    ppost(P, "/d/activate", {"path": "UICanvas/MainUI/Layer2/AncestralLineagePanel/Overlay/SpeciesPanels/" + sp, "active": True})
            time.sleep(0.3)
            sessions[name] = pget(P, "/d/ui")
            ov = pget(P, "/d/ui_overlay")
            if ov.get("ok"):
                refs[name] = save_png(ov["png_base64"], WORLD_REF / f"{name}_native_1920x1080.png")
        doc = {"schema": "rustibites.assets.ui_runtime.v2", "generatedBy": "tools/assets/capture_native_runtime_bulk.py",
               "capture": {"method": "standalone hidden native 1920 game; /d/ui with per-active-node screen_bbox/world_corners; menu UI-on-black refs",
                           "screen": sessions.get("main_menu_title", {}).get("screen"), "native": True},
               "sessions": sessions}
        with gzip.open(OUT / "ui_runtime_native.json.gz", "wb", compresslevel=9) as g:
            g.write(json.dumps(doc, indent=1).encode("utf-8"))
        index = {"schema": "rustibites.assets.ui_runtime_native.index.v2",
                 "files": {"runtime": "assets/game/ui_runtime_native.json.gz"},
                 "capture": doc["capture"],
                 "sessions": {k: {"screen": v.get("screen"), "node_count": v.get("node_count"),
                                  "canvases": len(v.get("canvases") or []), "menu_ref": refs.get(k)}
                              for k, v in sessions.items()},
                 "consumer": "native runtime UI atlas; old ui_runtime.json.gz historical"}
        (OUT / "ui_runtime_native.index.json").write_text(json.dumps(index, indent=2), encoding="utf-8")
        (OUT / "reference/functional_ui_native/menu_ui/native_runtime_manifest.json").write_text(
            json.dumps({"schema": "rustibites.ui.native_runtime_manifest.v1", "sessions": len(sessions),
                        "screen": doc["capture"]["screen"], "refs": refs}, indent=2), encoding="utf-8")
        print(json.dumps({"sessions": len(sessions), "screen": doc["capture"]["screen"],
                          "menu_refs": sum(1 for k in MENU if k in refs),
                          "world_refs": sum(1 for k in WORLD if k in refs)}, indent=1))
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

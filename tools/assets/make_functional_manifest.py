import json
from pathlib import Path

root = Path(r"Z:\RUSTibites")
base = root / "assets/game/reference/functional_ui"
manifest = json.loads((base / "manifest.json").read_text(encoding="utf-8"))
label_map = {}
for sess, rows in (manifest.get("controls") or {}).items():
    for r in rows or []:
        for label in r.get("labels") or []:
            label = str(label).strip()
            if not label:
                continue
            label_map.setdefault(label, []).append({
                "session": sess, "path": r.get("path"), "name": r.get("name"),
                "active": r.get("active"), "controls": r.get("controls")})
# menu label -> explicit panel session/path mapping requested by functional parity
menu_map = {
    "Load": ("main_menu_load_game", "UI/LoadGamePanel"),
    "Bibite Library": ("main_menu_bibite_library", "UI/BibiteSelector"),
    "Settings": ("main_menu_settings", "UI/SettingsPanel"),
    "Achievements": ("main_menu_achievements", "UI/WelcomeScreen/AchievementsPanel"),
    "Additional Tools": ("main_menu_additional_tools", "UI/WelcomeScreen/ToolsPanel"),
    "Credits": ("main_menu_credits", "UI/WelcomeScreen/CreditsPanel"),
    "New Game": ("main_menu_new_game", "UI/NewGameScenarios"),
    "Settings/Graphics": ("main_menu_settings_graphics", "UI/SettingsPanel/Graphics"),
    "Settings/Gameplay": ("main_menu_settings_gameplay", "UI/SettingsPanel/Gameplay"),
    "Settings/Hotkeys": ("main_menu_settings_hotkeys", "UI/SettingsPanel/Controls"),
}
out = {"schema": "rustibites.assets.functional_ui.labels.v1",
       "session_keys": sorted(manifest.get("sessions", {})),
       "menu_labels": {k: {"session": v[0], "path": v[1]} for k, v in menu_map.items()},
       "labels": label_map,
       "note": "Label -> runtime node path/control state for menu and in-world panels. Menu roots were activated "
               "in the hidden Welcome UI; in-world panels opened by toolbar buttons. Use session_keys for atlas "
               "sessions; existing atlas sessions are preserved."}
(base / "label_path_manifest.json").write_text(json.dumps(out, indent=2), encoding="utf-8")
print("labels", len(label_map), "sessions", len(out["session_keys"]), "bytes", (base / "label_path_manifest.json").stat().st_size)

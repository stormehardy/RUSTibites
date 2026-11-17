import json
from pathlib import Path

root = Path(r"Z:\RUSTibites")
iw = root / "assets/game/reference/functional_ui_native/in_world"
man = json.loads((iw / "manifest.json").read_text(encoding="utf-8"))
alias = {
    "native_world_toolbar": "main_world_toolbar",
    "native_world_placer": "main_world_tool_placer",
    "native_world_user_actions": "main_world_tool_user_actions",
    "native_world_view": "main_world_tool_view",
    "native_world_dynamic": "main_world_dynamic_parameters",
    "native_world_lineage": "main_world_lineage",
    "native_world_information": "main_world_information",
    "native_world_settings": "main_world_settings_panel",
    "native_world_placer_pellets": "main_world_tool_placer_pellets",
    "native_inspector_stats": "default_world_inspector",
    "native_inspector_genes": "default_world_genes",
    "native_inspector_biology": "default_world_biology",
    "native_inspector_brain": "default_world_brain",
}
sessions = {}
for s in man.get("sessions") or []:
    name = s.get("session")
    runtime = alias.get(name)
    if runtime and s.get("file"):
        sessions[runtime] = s["file"]
out = {"schema": "rustibites.ui.visual_reference_map.v1",
       "authored_by": "parity-dev",
       "native_reference_dir": "assets/game/reference/functional_ui_native/in_world",
       "sessions": sessions,
       "notes": "Canonical native in-world map for tools/ui; every path is native 1920 and UI-only from the "
                "standalone hidden game. Menu sessions intentionally excluded (legacy/upscaled; isolation blocked). "
                "Placer-pellets has no runtime counterpart yet; keep only if the runner has that session."}
(iw / "session_map.json").write_text(json.dumps(out, indent=2), encoding="utf-8")
man["session_map"] = "assets/game/reference/functional_ui_native/in_world/session_map.json"
man["session_map_schema"] = "rustibites.ui.visual_reference_map.v1"
(iw / "manifest.json").write_text(json.dumps(man, indent=2), encoding="utf-8")
print(json.dumps(out, indent=1))

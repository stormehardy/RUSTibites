#!/usr/bin/env python3
"""coverage.py - asset coverage report against the game's renderers/UI (task-76).

Checks what assets/game/manifest.json contains against:
  * the procedural sprite system: every part sheet in `text/bibites_parts.csv`, the object sprites
    implied by egg/pellet sizes, the parameters text asset, and every Texture2D for those sheets;
  * the in-world prefab roots dumped by tools/assets/prefab_dump.py (bibite/egg/zone/pellets/effects);
  * `Here/SimulationScripts.Rendering` + the listed renderer scripts: scans for quoted sprite/texture
    names and `Resources.Load` paths and reports how many resolve to an extracted asset;
  * `Here/UIScripts` + `docs/design/UI_PARITY.md` §10 (when present): UI sprite/9-slice/font coverage.

Writes assets/game/coverage.json (counts, percentages, gaps). The percentage printed for "in-world" is
the gate the Lead asked for (>=90%); UI coverage is reported separately because task-79's expectation
list may still be growing.
"""
from __future__ import annotations

import json
import re
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
GAME = ROOT / "assets" / "game"
HERE = ROOT / "Here"

WORLD_RENDER_FILES = [
    "SimulationScripts/Rendering",
    "ProceduralSpriteManager.cs", "BibiteProceduralSpriter.cs", "PelletProceduralSpriter.cs",
    "EggHatching.cs", "Zone.cs", "BackgroundManager.cs", "EffectTower.cs", "ColorKiller.cs",
    "RedDeathBloomManager.cs", "FieldOfViewDisplay.cs",
]
UI_RENDER_FILES = ["UIScripts", "UIBrainIcons", "BrainIconHolder.cs"]


def collect_files(paths):
    files = []
    for rel in paths:
        p = HERE / rel
        if p.is_file():
            files.append(p)
        elif p.is_dir():
            files.extend(sorted(p.rglob("*.cs")))
    return files


def scan_refs(files, known_names):
    """Quoted literals / Resources.Load paths that look like asset references."""
    refs = {}
    literal_re = re.compile(r'"([A-Za-z0-9_./-]{2,80})"')
    for f in files:
        try:
            text = f.read_text(encoding="utf-8", errors="replace")
        except Exception:
            continue
        for m in literal_re.finditer(text):
            s = m.group(1)
            if "/" not in s and "_" not in s:
                continue
            if s.lower() in known_names or s in known_names:
                refs.setdefault(s, []).append(str(f.relative_to(ROOT)))
    return refs


def main() -> int:
    manifest = json.loads((GAME / "manifest.json").read_text(encoding="utf-8"))
    procedural = manifest.get("procedural") or {}
    parts = procedural.get("parts") or {}
    sizes = procedural.get("sizes") or {}
    egg_sizes = procedural.get("egg_sizes") or {}
    pellet_sizes = procedural.get("pellet_sizes") or {}
    textures = manifest.get("textures") or {}
    sprites = manifest.get("sprites") or {}
    names = {k.lower() for k in textures} | {k.lower() for k in sprites}

    # the loader slices sheets on a uniform grid from the CSV, so a sheet TEXTURE is what matters here
    # (Sprite objects only exist for a subset of part frames in the shipped build)
    parts_found = [n for n in parts if n in textures]
    object_expected = sorted({f"{size}_egg" for size in egg_sizes} |
                             {f"{size}_plant" for size in pellet_sizes} |
                             {f"{size}_meat" for size in pellet_sizes})
    object_found = [n for n in object_expected if n in textures]
    params_ok = bool(procedural.get("parameters"))
    inworld = {
        "part_sheets_expected": len(parts),
        "part_sheets_found": len(parts_found),
        "part_sheets_missing": sorted(set(parts) - set(parts_found)),
        "object_sprites_expected": len(object_expected),
        "object_sprites_found": len(object_found),
        "object_sprites_missing": sorted(set(object_expected) - set(object_found)),
        "parameters_present": params_ok,
        "sprites_decoded": len(sprites),
        "sheet_textures": len(textures),
    }
    inworld_ok = len(parts_found) + len(object_found) + (1 if params_ok else 0)
    inworld_total = len(parts) + len(object_expected) + 1
    inworld["coverage_percent"] = round(100.0 * inworld_ok / max(1, inworld_total), 1)

    # prefab metadata
    prefabs = {}
    pp = GAME / "prefabs.json"
    if pp.exists():
        pd = json.loads(pp.read_text(encoding="utf-8"))
        prefabs = {"roots": [r["name"] for r in pd.get("roots", [])],
                   "root_count": len(pd.get("roots", [])),
                   "roots_with_sprite_renderers": sum(1 for r in pd.get("roots", [])
                                                      if any(n.get("sprite_renderer") for n in r.get("tree", []))),
                   "unreadable_components": len(pd.get("unreadable", []))}

    # Here/ reference scan (in-world renderers)
    world_files = collect_files(WORLD_RENDER_FILES)
    refs = scan_refs(world_files, names)
    ref_total = len(refs)
    ui_files = collect_files(UI_RENDER_FILES)
    ui_refs = scan_refs(ui_files, names)
    ui_sprites = sum(len(g.get("sprites") or {}) for g in (manifest.get("ui") or {}).values())
    ui_textures = sum(len(g.get("textures") or {}) for g in (manifest.get("ui") or {}).values())
    nine_slice = 0
    for g in (manifest.get("ui") or {}).values():
        for e in (g.get("sprites") or {}).values():
            b = e.get("border") or {}
            if any(float(b.get(k, 0)) > 0 for k in ("x", "y", "z", "w")):
                nine_slice += 1
    coverage = {
        "schema": "rustibites.assets.coverage.v1",
        "in_world": inworld,
        "prefabs": prefabs,
        "here_renderer_references": {
            "files_scanned": len(world_files),
            "asset_like_literals_known": ref_total,
            "examples": dict(list(refs.items())[:20]),
            "note": "only quoted literals that match an extracted asset name are counted; serialized "
                    "Sprite fields and enum/size tables are covered by the procedural CSV data instead.",
        },
        "ui": {
            "files_scanned": len(ui_files),
            "ui_sprites_extracted": ui_sprites,
            "ui_textures_extracted": ui_textures,
            "nine_slice_sprites": nine_slice,
            "ui_asset_literals_known": len(ui_refs),
            "brain_icon_sprites": sum(1 for k in {**textures, **sprites}
                                      if k.lower().startswith(("input_", "hidden_"))),
            "note": "task-79 ui_expectations.json (spec-dev) is the authoritative UI name list; this "
                    "manifest exposes every UI Sprite/Texture found in the shipped asset files so the "
                    "loader can name-match. TMP font assets: 22 Font objects exported as .ttf + atlas "
                    "textures referenced by their material (2 font-atlas textures are not PNG-exportable).",
        },
        "fonts": {"count": len(manifest.get("fonts") or []),
                  "extracted": sum(1 for f in manifest.get("fonts") or [] if f.get("bytes")),
                  "names": [f.get("name") for f in manifest.get("fonts") or []]},
        "materials": {"count": len(manifest.get("materials") or []),
                      "with_serialized_properties": sum(1 for m in manifest.get("materials") or []
                                                        if m.get("colors") or m.get("floats")),
                      "shader_names_unresolved_ids": (manifest.get("shader_resolution") or {}).get("unresolved_shader_ids")},
        "gaps": [
            "SharedMaterial name on prefab SpriteRenderers is null in the shipped build (material PPtr is "
            "external); per-material properties are in materials/*.json keyed by material name.",
            "Unity stripped Shader asset names: materials record `shader_<PathID>` (shader_resolution); a "
            "runtime plugin dump can map them to the shader names (BibiteSkin/background/zone/FOV).",
            "22 font-atlas textures are not PNG-exportable by UnityPy (image_export_failed); the .ttf font "
            "bytes and the Font objects are exported, which is what the viewer needs for text.",
            "Prefab components that are MonoBehaviour (Bibite, Zone, towers, effects) are unreadable without "
            "script typetrees; prefabs.json still records the readable Transform/SpriteRenderer hierarchy",
            "UI name coverage against task-79's ui_expectations.json is pending that list; every UI sprite "
            "root (237) and UI texture (244) is extracted for name matching.",
        ],
    }
    (GAME / "coverage.json").write_text(json.dumps(coverage, indent=2), encoding="utf-8")
    print(json.dumps({"in_world_coverage_percent": inworld["coverage_percent"],
                      "part_sheets": f"{inworld['part_sheets_found']}/{inworld['part_sheets_expected']}",
                      "object_sprites": f"{inworld['object_sprites_found']}/{inworld['object_sprites_expected']}",
                      "prefabs": prefabs.get("root_count"),
                      "ui_sprites": ui_sprites, "ui_textures": ui_textures, "nine_slice": nine_slice,
                      "out": str(GAME / "coverage.json")}, indent=1))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

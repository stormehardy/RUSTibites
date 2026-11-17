#!/usr/bin/env python3
"""extract.py - extract The Bibites' runtime art assets into the repo (`assets/game/`, tracked).

Task-76 (visual parity). Uses UnityPy 1.25.4 against the installed game's read-only data folder. The
game is never launched by this script; it only reads the Unity asset files.

What is extracted
-----------------
* every `Texture2D` (lossless PNG, alpha preserved) with width/height + source file;
* every `Sprite` (UnityPy-decoded PNG where possible) with exact `rect` / `pivot` / `pixels_per_unit`,
  9-slice `border`, and the source texture sheet name;
* the `Resources` TextAssets that drive procedural visuals (`parameters`, `sizes`, `egg_sizes`,
  `pellet_sizes`, `parts`, plus any other named text asset) byte-for-byte;
* `Font` objects (raw font bytes when UnityPy exposes them; otherwise metadata only);
* `Material` objects with shader name + saved color/float properties where readable;
* UI sprites/textures/fonts from `level0..level4`, `sharedassets*.assets`, `globalgamemanagers.assets`.

Output layout (all under assets/game/)
    manifest.json           logical name -> file + rect/pivot/PPU + source + counts + coverage stub
    sheets/*.png            Texture2D sheets (procedural parts + object sprites)
    sprites/*.png           decoded Sprite images (resources.assets procedural sprites)
    parts/*.txt             raw TextAssets (parameters/sizes/parts/...)
    fonts/                  font files / font metadata JSON
    materials/              material JSON (shader + properties)
    ui/<assetfile>/...      UI sprite PNGs, sheet PNGs, font files from scene/shared assets
    reference/              (filled by a separate live-game capture step)

Usage:
    python tools/assets/extract.py                     # extract from the default Steam install
    python tools/assets/extract.py --game-data "C:\\...\\The Bibites_Data" --out assets/game
    python tools/assets/extract.py --list              # enumerate assets without writing
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent


def _rel(path):
    """Path for the manifest: relative to the repo when inside it, else as given (a packaged extractor writes elsewhere)."""
    try:
        return Path(path).resolve().relative_to(ROOT).as_posix()
    except ValueError:
        return Path(path).as_posix()
DEFAULT_GAME_DATA = Path(r"C:\Program Files (x86)\Steam\steamapps\common\The Bibites\The Bibites_Data")
DEFAULT_OUT = ROOT / "assets" / "game"

ASSET_FILES = ["resources.assets", "globalgamemanagers.assets", "sharedassets0.assets", "sharedassets1.assets",
               "sharedassets2.assets", "sharedassets3.assets", "level0", "level1", "level2", "level3", "level4"]
WORLD_FILES = {"resources.assets", "globalgamemanagers.assets"}
UI_FILES = ["sharedassets0.assets", "sharedassets1.assets", "sharedassets2.assets", "sharedassets3.assets",
            "level1", "level2", "level3", "level4"]


def safe(name: str) -> str:
    name = name or "unnamed"
    name = re.sub(r"[^A-Za-z0-9._-]+", "_", name).strip("_")
    return name or "unnamed"


def vec(v):
    if v is None:
        return None
    if isinstance(v, (list, tuple)):
        return [float(x) for x in v]
    out = {}
    for axis in ("x", "y", "z", "w", "width", "height"):
        if hasattr(v, axis):
            try:
                out[axis] = float(getattr(v, axis))
            except Exception:
                pass
    return out or None


def crop_sprite_image(sprite, rect):
    """Crop a Sprite's pixels from its Texture2D with Unity's bottom-left rect origin.

    UnityPy's `sprite.image` export can silently use a top-left-origin crop (task-137: y=0 cell came out
    fully transparent while the visible art is on the bottom row). Always crop from the decoded sheet:
    PIL/top = sheet_height - (rect.y + rect.height); x/left = rect.x.
    """
    try:
        tex_ref = getattr(sprite, "m_Texture", None)
        if tex_ref is None:
            return None
        tex = tex_ref.read()
        image = tex.image.convert("RGBA")
        width = image.width
        height = image.height
        x = int(round(float(rect.get("x", 0))))
        y = int(round(float(rect.get("y", 0))))
        w = int(round(float(rect.get("width", 0))))
        h = int(round(float(rect.get("height", 0))))
        if w <= 0 or h <= 0:
            return None
        top = height - y - h
        if x < 0 or top < 0 or x + w > width or top + h > height:
            # fall back to the raw position if the rect is unusual (e.g. partially off-sheet)
            return None
        return image.crop((x, top, x + w, top + h))
    except Exception:
        return None


def sprite_meta(d):
    """Exact sprite geometry from UnityPy's Sprite object (rect/pivot/PPU/border live on the Sprite,
    the texture reference lives in m_RD; verified against UnityPy 1.25.4)."""
    rect = vec(getattr(d, "m_Rect", None))
    pivot = vec(getattr(d, "m_Pivot", None))
    ppu = None
    try:
        ppu = float(d.m_PixelsToUnits)
    except Exception:
        ppu = None
    border = vec(getattr(d, "m_Border", None))
    rd = getattr(d, "m_RD", None)
    texture_name = None
    tex = getattr(rd, "texture", None) if rd is not None else None
    if tex is not None:
        try:
            texture_name = tex.m_Name if hasattr(tex, "m_Name") and tex.m_Name else tex.name
        except Exception:
            texture_name = None
        if not texture_name:
            try:
                texture_name = tex.read().m_Name
            except Exception:
                pass
    return rect, pivot, ppu, border, texture_name


def read_material(d):
    out = {"name": getattr(d, "m_Name", None), "shader": None, "colors": {}, "floats": {}, "textures": {}}
    sh = getattr(d, "m_Shader", None)
    if sh is not None:
        try:
            out["shader"] = sh.read().m_Name
        except Exception:
            out["shader"] = getattr(sh, "name", None)
    sp = getattr(d, "m_SavedProperties", None)
    if sp is not None:
        for attr, dest in (("m_Colors", "colors"), ("m_Floats", "floats"), ("m_TexEnvs", "textures")):
            obj = getattr(sp, attr, None)
            try:
                items = obj.items() if hasattr(obj, "items") else obj
                if items:
                    for k, v in items:
                        key = k if isinstance(k, str) else str(k)
                        if dest == "colors":
                            out[dest][key] = vec(v) if not isinstance(v, (int, float)) else float(v)
                        elif dest == "floats":
                            out[dest][key] = float(v)
                        else:
                            out[dest][key] = getattr(v, "m_Name", None) or str(v)
            except Exception as exc:
                out.setdefault("errors", []).append(f"{attr}: {exc}")
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--game-data", default=str(DEFAULT_GAME_DATA))
    ap.add_argument("--out", default=str(DEFAULT_OUT))
    ap.add_argument("--list", action="store_true", help="enumerate object counts/names only (no writes)")
    args = ap.parse_args()
    try:
        import UnityPy
        from PIL import Image  # noqa: F401  (UnityPy image export needs Pillow)
    except Exception as exc:  # noqa: BLE001
        raise SystemExit(f"UnityPy/Pillow unavailable ({exc}); pip install UnityPy pillow") from exc
    game_data = Path(args.game_data)
    out = Path(args.out)
    if not game_data.exists():
        raise SystemExit(f"game data folder not found: {game_data}")

    manifest = {"schema": "rustibites.assets.manifest.v1",
                "generated_by": "tools/assets/extract.py",
                "game": {"name": "The Bibites", "unity": "6000.0.44f1",
                         "data_dir": str(game_data)},
                "assets_root": "assets/game",
                "layout": {"sheets": "sheets/*.png", "sprites": "sprites/*.png", "parts": "parts/*.txt",
                           "fonts": "fonts/", "materials": "materials/", "ui": "ui/<assetfile>/"},
                "text_files": {}, "textures": {}, "sprites": {}, "fonts": [], "materials": [],
                "ui": {}, "stats": {}, "errors": []}
    counts = {"textures": 0, "sprites": 0, "text_assets": 0, "fonts": 0, "materials": 0, "ui_sprites": 0,
              "ui_textures": 0}
    listing = {}

    shader_maps = {}
    for fname in ASSET_FILES:
        path = game_data / fname
        if not path.exists():
            continue
        env = UnityPy.load(str(path))
        is_world = fname in WORLD_FILES
        listing[fname] = {"sprites": 0, "textures": 0, "text_assets": 0, "fonts": 0, "materials": 0}
        # pass 1: shader PathID -> name (materials reference shaders by PPtr)
        shaders = {}
        for obj in env.objects:
            if obj.type.name != "Shader":
                continue
            try:
                shd = obj.read()
                name = getattr(shd, "m_Name", None)
                if not name:
                    parsed = getattr(shd, "m_ParsedForm", None)
                    name = getattr(parsed, "m_Name", None) if parsed is not None else None
                shaders[obj.path_id] = name or f"shader_{obj.path_id}"
            except Exception:
                shaders[obj.path_id] = f"shader_{obj.path_id}"
        shader_maps[fname] = shaders
        for obj in env.objects:
            t = obj.type.name
            if t not in ("Texture2D", "Sprite", "TextAsset", "Font", "Material"):
                continue
            try:
                d = obj.read()
            except Exception as exc:  # noqa: BLE001
                manifest["errors"].append({"file": fname, "type": t, "error": str(exc)})
                continue
            name = getattr(d, "m_Name", None) or getattr(d, "name", None) or "unnamed"
            if t == "Texture2D":
                listing[fname]["textures"] += 1
                counts["textures"] += 1
                entry = {"name": name, "source_asset": fname,
                         "width": getattr(d, "m_Width", None), "height": getattr(d, "m_Height", None),
                         "format": getattr(getattr(d, "m_TextureFormat", None), "name", None)}
                if not args.list:
                    if is_world:
                        dest = out / "sheets" / f"{safe(name)}.png"
                    else:
                        dest = out / "ui" / safe(fname) / "sheets" / f"{safe(name)}.png"
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    try:
                        img = crop_sprite_image(d, (entry.get("rect") or {})) or d.image
                        img.save(dest)
                        entry["file"] = _rel(dest)
                    except Exception as exc:  # noqa: BLE001
                        if len(manifest["errors"]) < 10:
                            manifest["errors"].append({"file": fname, "type": t, "name": name, "error": str(exc)})
                        counts["image_export_failed"] = counts.get("image_export_failed", 0) + 1
                        continue
                if is_world:
                    manifest["textures"][name] = entry
                else:
                    manifest["ui"].setdefault(fname, {}).setdefault("textures", {})[name] = entry
                    counts["ui_textures"] += 1
            elif t == "Sprite":
                listing[fname]["sprites"] += 1
                counts["sprites"] += 1
                rect, pivot, ppu, border, texture_name = sprite_meta(d)
                entry = {"name": name, "source_asset": fname, "sheet": texture_name,
                         "rect": rect, "pivot": pivot, "pixels_per_unit": ppu, "border": border}
                if not args.list:
                    if is_world:
                        dest = out / "sprites" / f"{safe(name)}.png"
                    else:
                        dest = out / "ui" / safe(fname) / "sprites" / f"{safe(name)}.png"
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    try:
                        d.image.save(dest)
                        entry["file"] = _rel(dest)
                    except Exception as exc:  # noqa: BLE001
                        # fall back to the sheet + rect (view-dev can crop); not fatal
                        manifest["errors"].append({"file": fname, "type": t, "name": name,
                                                   "error": f"image export failed: {exc} (use sheet+rect)"})
                if is_world:
                    # The live game names single-digit sheet cells with two digits (`normal_body_00`),
                    # while the Unity Sprite asset is named `normal_body_0`. Keep both keys/aliases so
                    # runtime SpriteRenderer names resolve without guesswork.
                    m = re.match(r"^(.*_)(\d+)$", name or "")
                    if m and len(m.group(2)) < 2:
                        alias = f"{m.group(1)}{int(m.group(2)):02d}"
                        if alias not in manifest["sprites"]:
                            entry.setdefault("aliases", [])
                            if alias not in entry["aliases"]:
                                entry["aliases"].append(alias)
                            manifest["sprites"][alias] = entry
                    manifest["sprites"][name] = entry
                else:
                    manifest["ui"].setdefault(fname, {}).setdefault("sprites", {})[name] = entry
                    counts["ui_sprites"] += 1
            elif t == "TextAsset":
                listing[fname]["text_assets"] += 1
                counts["text_assets"] += 1
                txt = d.m_Script if hasattr(d, "m_Script") else str(d)
                PROCEDURAL_TEXTS = {"parameters", "sizes", "egg_sizes", "pellet_sizes", "parts"}
                entry = {"name": name, "source_asset": fname, "chars": len(txt),
                         "art_input": name in PROCEDURAL_TEXTS}
                TEXT_LAYOUT = {
                    "parameters": ("text/parameters.csv", "ProceduralSprites/parameters"),
                    "parts": ("text/bibites_parts.csv", "ProceduralSprites/Bibites/parts"),
                    "sizes": ("text/bibites_sizes.csv", "ProceduralSprites/Bibites/sizes"),
                    "egg_sizes": ("text/egg_sizes.csv", "ProceduralSprites/Objects/egg_sizes"),
                    "pellet_sizes": ("text/pellet_sizes.csv", "ProceduralSprites/Objects/pellet_sizes"),
                }
                if name in PROCEDURAL_TEXTS:
                    rel, logical = TEXT_LAYOUT[name]
                    entry["logical"] = logical
                    if not args.list:
                        dest = out / rel
                        dest.parent.mkdir(parents=True, exist_ok=True)
                        dest.write_bytes(txt.encode("utf-8", "surrogatepass"))
                        entry["file"] = _rel(dest)
                        counts["parts_extracted"] = counts.get("parts_extracted", 0) + 1
                # other TextAssets are game data (scenario/template blobs, supporter lists); not art.
                manifest["text_files"][name] = entry
            elif t == "Font":
                listing[fname]["fonts"] += 1
                counts["fonts"] += 1
                entry = {"name": name, "source_asset": fname}
                raw = None
                for attr in ("m_FontData", "m_FontDataRaw", "m_FontDataBytes"):
                    data = getattr(d, attr, None)
                    if isinstance(data, (bytes, bytearray)) and len(data) > 16:
                        raw = bytes(data)
                        break
                    if isinstance(data, list) and len(data) > 16 and all(isinstance(x, int) for x in data[:32]):
                        raw = bytes(x & 0xFF for x in data)
                        break
                entry["bytes"] = len(raw) if raw else 0
                for attr in ("m_FontSize", "m_Ascent", "m_Descent", "m_LineSpacing", "m_CharacterSpacing"):
                    try:
                        entry[attr] = float(getattr(d, attr))
                    except Exception:
                        pass
                atlas = getattr(d, "m_Texture", None)
                if atlas is not None:
                    try:
                        entry["atlas"] = atlas.read().m_Name
                    except Exception:
                        entry["atlas"] = getattr(atlas, "name", None)
                if raw is not None and not args.list:
                    dest = out / "fonts" / f"{safe(name)}.ttf"
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    dest.write_bytes(raw)
                    entry["file"] = _rel(dest)
                    counts["fonts_extracted"] = counts.get("fonts_extracted", 0) + 1
                manifest["fonts"].append(entry)
            elif t == "Material":
                listing[fname]["materials"] += 1
                counts["materials"] += 1
                mat = read_material(d)
                mat["source_asset"] = fname
                sh = getattr(d, "m_Shader", None)
                if sh is not None:
                    try:
                        pid = int(getattr(sh, "m_PathID", 0) or 0)
                        fid = int(getattr(sh, "m_FileID", 0) or 0)
                        if fid == 0:
                            mat["shader"] = shaders.get(pid) or mat.get("shader")
                        else:
                            mat["shader_external_file"] = fid
                    except Exception:
                        pass
                manifest["materials"].append(mat)
                if not args.list:
                    dest = out / "materials" / f"{safe(name)}.json"
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    dest.write_text(json.dumps(mat, indent=1), encoding="utf-8")
                    mat["file"] = _rel(dest)

    # -- structured view of the procedural CSV assets (raw text is preserved in parts/*.txt) ----------
    def parse_semi_csv(text: str):
        import csv as _csv
        import io
        rows = list(_csv.reader(io.StringIO(text), delimiter=";"))
        return [r for r in rows if r and any(c.strip() for c in r)]

    def to_float(tok: str):
        try:
            return float(tok.replace(",", "."))
        except Exception:
            return None

    def read_part(name: str) -> str | None:
        e = manifest["text_files"].get(name)
        if not e or not e.get("file"):
            return None
        try:
            return (ROOT / e["file"]).read_text(encoding="utf-8", errors="replace")
        except Exception:
            return None

    parameters = {}
    raw = read_part("parameters")
    if raw:
        for row in parse_semi_csv(raw):
            if len(row) >= 2 and row[0] not in ("name",):
                v = to_float(row[1])
                if v is not None:
                    parameters[row[0].strip()] = v
    sizes = {}
    raw = read_part("sizes")
    if raw:
        rows = parse_semi_csv(raw)
        for row in rows[1:]:
            if len(row) >= 2:
                v = to_float(row[1])
                if v is not None:
                    sizes[row[0].strip()] = v
    egg_sizes, pellet_sizes = {}, {}
    for target, asset_name in ((egg_sizes, "egg_sizes"), (pellet_sizes, "pellet_sizes")):
        raw = read_part(asset_name)
        if not raw:
            continue
        rows = parse_semi_csv(raw)
        for row in rows[1:]:
            if len(row) >= 2:
                v = to_float(row[1])
                if v is not None:
                    target[row[0].strip()] = v
    parts = {}
    raw = read_part("parts")
    if raw:
        rows = parse_semi_csv(raw)
        for row in rows[1:]:
            if len(row) >= 3:
                name = row[0].strip()
                try:
                    parts[name] = {"rows": int(row[1]), "columns": int(row[2])}
                except ValueError:
                    parts[name] = {"rows": row[1], "columns": row[2]}
    unresolved = [m.get("shader") for m in manifest["materials"]
                  if isinstance(m.get("shader"), str) and m["shader"].startswith("shader_")]
    manifest["shader_resolution"] = {
        "materials_total": len(manifest["materials"]),
        "materials_with_named_shader": sum(1 for m in manifest["materials"]
                                           if m.get("shader") and not str(m["shader"]).startswith("shader_")),
        "unresolved_shader_ids": sorted(set(unresolved)),
        "note": "Unity strips Shader asset names in this build; materials keep the shader PathID "
                "(shader_<pid>) plus their serialized _Color/_FromHue/_VminAtS0/_VmaxAtS0/... properties. "
                "A runtime plugin dump can resolve the real shader names if needed.",
    }
    # UI material aliases for the viewer: spec-dev's task-79 required gap is the color-killer hue
    # spectrum material (game name `rainbowMaterial`); index the serialized properties under a logical
    # UI name so the loader does not have to know the game's internal material names.
    mat_by_name = {m.get("name"): m for m in manifest["materials"] if m.get("name")}
    ui_materials = {}
    for logical, real in {"color_spectrum": "rainbowMaterial",
                          "skin_recolorizer": "recolorizerShader",
                          "eye_recolorizer": "eyesRecolorizerShader",
                          "field_of_view": "FieldOfViewMat",
                          "tolerance": "tolerance"}.items():
        if real in mat_by_name:
            m = dict(mat_by_name[real])
            m["logical"] = logical
            m["file"] = f"assets/game/materials/{safe(real)}.json"
            ui_materials[logical] = m
    manifest["ui"]["materials"] = ui_materials
    manifest["procedural"] = {
        "text_assets": {manifest["text_files"][k].get("logical", k): manifest["text_files"][k].get("file")
                        for k in ("parameters", "sizes", "egg_sizes", "pellet_sizes", "parts")
                        if k in manifest["text_files"]},
        "parameters": parameters,
        "sizes": sizes,
        "egg_sizes": egg_sizes,
        "pellet_sizes": pellet_sizes,
        "parts": parts,
        "note": "raw CSV is preserved byte-for-byte under parts/*.txt (values use a comma decimal in the "
                "shipped build); the parsed maps convert ',' to '.' for convenience. parts[name] = sheet "
                "grid (rows x columns) for the corresponding sheet in sheets/<name>.png.",
    }
    # group decoded part sprites by sheet so a renderer can slice sheets without re-reading Unity data
    by_sheet = {}
    for name, e in manifest["sprites"].items():
        sheet = e.get("sheet")
        if sheet:
            by_sheet.setdefault(sheet, []).append({"name": name, "file": e.get("file"), "rect": e.get("rect"),
                                                   "pivot": e.get("pivot"), "pixels_per_unit": e.get("pixels_per_unit")})
    manifest["sheets"] = by_sheet
    # attach the optional pieces produced by the companion scripts (prefabs/reference frames)
    prefab_path = out / "prefabs.json"
    if prefab_path.exists():
        try:
            pd = json.loads(prefab_path.read_text(encoding="utf-8"))
            manifest["prefabs"] = {
                "file": "assets/game/prefabs.json",
                "source": pd.get("source", "UnityPy level/sharedassets scan"),
                "roots": [r.get("name") for r in pd.get("roots", [])],
                "root_count": len(pd.get("roots", [])),
                "note": "readable Transform/SpriteRenderer hierarchy for the in-world prefab roots; "
                        "MonoBehaviour-only components need a runtime plugin dump (see assets/game/README.md)",
            }
        except Exception as exc:
            manifest.setdefault("errors", []).append({"type": "prefabs", "error": str(exc)})
    ref_path = out / "reference" / "reference.json"
    if ref_path.exists():
        try:
            rd = json.loads(ref_path.read_text(encoding="utf-8"))
            manifest["reference"] = {
                "dir": "assets/game/reference/",
                "readme": "assets/game/reference/README.md",
                "capture_method": rd.get("capture_method"),
                "frames": [{"file": f.get("file"), "width": f.get("width"), "height": f.get("height"),
                            "camera": f.get("camera"), "asked": f.get("asked")} for f in rd.get("frames", [])],
            }
        except Exception as exc:
            manifest.setdefault("errors", []).append({"type": "reference", "error": str(exc)})
    manifest["stats"] = {**counts, "files": listing,
                         "sprites_by_sheet": {k: len(v) for k, v in by_sheet.items()}}
    if args.list:
        print(json.dumps(manifest["stats"], indent=1))
        return 0
    out.mkdir(parents=True, exist_ok=True)
    (out / "manifest.json").write_text(json.dumps(manifest, indent=1), encoding="utf-8")
    total = sum(f.stat().st_size for f in out.rglob("*") if f.is_file())
    print(json.dumps({"counts": counts, "manifest": str(out / "manifest.json"),
                      "total_bytes": total, "total_mb": round(total / 1e6, 2)}, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())

import json
import re
import UnityPy
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
GAME = Path(r"C:\Program Files (x86)\Steam\steamapps\common\The Bibites\The Bibites_Data")
OUT = ROOT / "assets/game/ui_fonts"
fonts = []
for p in sorted(OUT.glob("*.json")):
    if p.name in ("fonts_report.json", "fonts_manifest.json"):
        continue
    try:
        fonts.append(json.loads(p.read_text(encoding="utf-8")))
    except Exception:
        pass
# extract atlas textures for fonts by material _MainTex / atlas names
atlas_files = {}
for f in list(GAME.glob("*.assets")) + [GAME / "resources.assets"]:
    if not f.exists():
        continue
    try:
        env = UnityPy.load(str(f))
    except Exception:
        continue
    for o in env.objects:
        if o.type.name != "Texture2D":
            continue
        try:
            d = o.read()
            name = getattr(d, "m_Name", "") or getattr(d, "name", "")
            if not any(w in name for w in ("SDF Atlas", "SDF - Fallback Atlas", "Roboto-Bold Atlas", "Roboto Bold Atlas")):
                continue
            safe = re.sub(r"[^A-Za-z0-9._-]+", "_", name)
            p = OUT / f"atlas_{safe}.png"
            img = d.image
            img.save(p)
            atlas_files[name] = {"file": p.name, "width": d.m_Width, "height": d.m_Height, "source": f.name, "path_id": o.path_id}
        except Exception as e:
            print("texture fail", o.path_id, e)
manifest = []
for jp in sorted(OUT.glob("*.json")):
    if jp.name in ("fonts_report.json", "fonts_manifest.json"):
        continue
    try:
        f = json.loads(jp.read_text(encoding="utf-8"))
    except Exception:
        continue
    material = f.get("material") or {}
    maintex = material.get("_MainTex")
    atlas = atlas_files.get(maintex)
    if atlas is None and f.get("atlasTextures"):
        at = f["atlasTextures"][0]
        atlas = atlas_files.get(at.get("name"))
    manifest.append({"name": f.get("name"), "json": jp.name, "atlas_texture": maintex,
                     "atlas_file": atlas["file"] if atlas else None,
                     "atlas_width": atlas["width"] if atlas else None,
                     "atlas_height": atlas["height"] if atlas else None,
                     "glyphs": len(f.get("glyphTable") or []), "characters": len(f.get("characterTable") or []),
                     "material": material.get("name"), "shader": material.get("shader"),
                     "face": f.get("faceInfo")})
(OUT / "fonts_manifest.json").write_text(json.dumps({"schema": "rustibites.assets.tmp_fonts.v1",
                                                      "source": "standalone hidden game /d/font_assets reflection + UnityPy atlas extraction",
                                                      "fonts": manifest, "atlas_files": atlas_files}, indent=2), encoding="utf-8")
print(json.dumps({"fonts": manifest, "atlases": atlas_files}, indent=1)[:6000])

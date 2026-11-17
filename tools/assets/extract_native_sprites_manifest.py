import gzip
import json
import re
import UnityPy
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
GAME = Path(r"C:\Program Files (x86)\Steam\steamapps\common\The Bibites\The Bibites_Data")
OUT = ROOT / "assets/game/reference/functional_ui_native/sprites"
OUT.mkdir(parents=True, exist_ok=True)
d = json.loads(gzip.open(ROOT / "assets/game/ui_runtime_native.json.gz", "rt", encoding="utf-8").read())
names = set()
for s in d["sessions"].values():
    def walk(n):
        for c in n.get("components") or []:
            if c.get("type") in ("UnityEngine.UI.Image", "UnityEngine.UI.RawImage"):
                spr = c.get("sprite")
                if isinstance(spr, dict) and spr.get("name"):
                    names.add(str(spr["name"]))
                elif isinstance(spr, str) and spr:
                    names.add(spr)
        for k in n.get("children") or []:
            walk(k)
    for r in s.get("canvases") or []:
        walk(r)
# collect sprite objects and texture2D fallbacks
sprites = {}
for f in list(GAME.glob("*.assets")) + [GAME / "resources.assets"]:
    if not f.exists():
        continue
    try:
        env = UnityPy.load(str(f))
    except Exception:
        continue
    for o in env.objects:
        if o.type.name != "Sprite":
            continue
        try:
            obj = o.read()
            name = getattr(obj, "m_Name", "") or getattr(obj, "name", "")
            if name not in names or name in sprites:
                continue
            img = obj.image
            safe = re.sub(r"[^A-Za-z0-9._-]+", "_", name)
            p = OUT / f"{safe}.png"
            img.save(p)
            try:
                tex = obj.m_RD.texture.path_id if obj.m_RD and obj.m_RD.texture else None
                texname = obj.m_RD.texture.read().m_Name if obj.m_RD and obj.m_RD.texture else None
            except Exception:
                tex = texname = None
            sprites[name] = {"file": str(p.relative_to(ROOT).as_posix()), "width": img.width, "height": img.height,
                             "texture": texname, "texture_path_id": tex, "source": f.name, "path_id": o.path_id}
        except Exception:
            pass
# textures fallback for names not found as sprites
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
            obj = o.read()
            name = getattr(obj, "m_Name", "") or getattr(obj, "name", "")
            if name not in names or name in sprites:
                continue
            safe = re.sub(r"[^A-Za-z0-9._-]+", "_", name)
            p = OUT / f"{safe}.png"
            obj.image.save(p)
            sprites[name] = {"file": str(p.relative_to(ROOT).as_posix()), "width": obj.m_Width, "height": obj.m_Height,
                             "texture": name, "texture_path_id": o.path_id, "source": f.name, "path_id": o.path_id,
                             "kind": "texture"}
        except Exception:
            pass
manifest = {"schema": "rustibites.ui.native_sprite_manifest.v1",
            "source": "UnityPy Sprite/Texture2D extraction from game assets for names referenced by native runtime UI dumps",
            "referenced_count": len(names), "mapped_count": len(sprites),
            "missing": sorted(n for n in names if n not in sprites),
            "sprites": dict(sorted(sprites.items()))}
(OUT.parent / "sprite_manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
print(json.dumps({"referenced": len(names), "mapped": len(sprites), "missing": manifest["missing"][:30],
                  "files": len(list(OUT.glob("*.png")))}, indent=1))

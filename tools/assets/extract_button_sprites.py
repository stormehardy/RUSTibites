import json
import UnityPy
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
GAME = Path(r"C:\Program Files (x86)\Steam\steamapps\common\The Bibites\The Bibites_Data")
OUT = ROOT / "assets/game/reference/functional_ui_native/in_world/button_theme"
d = json.loads((OUT / "button_theme.json").read_text(encoding="utf-8"))
sprites = sorted({(i.get("sprite") or "") for i in (d.get("active_images") or []) if i.get("sprite")})
extracted = {p.stem for p in (ROOT / "assets/game/sprites").glob("*.png")} if (ROOT / "assets/game/sprites").exists() else set()
missing = [s for s in sprites if s not in extracted]
textures = {}
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
            t = o.read()
            name = getattr(t, "m_Name", "") or getattr(t, "name", "")
            if name not in missing or name in textures:
                continue
            p = OUT / "sprites" / f"{name}.png"
            t.image.save(p)
            textures[name] = {"file": str(p.relative_to(ROOT).as_posix()), "source": f.name,
                              "path_id": o.path_id, "width": t.m_Width, "height": t.m_Height}
        except Exception:
            pass
d["active_image_sprites"] = sprites
d["missing_from_extracted"] = missing
d["missing_sprite_textures"] = textures
(OUT / "button_theme.json").write_text(json.dumps(d, indent=2), encoding="utf-8")
print("sprites", sprites)
print("missing", missing)
print("textures", textures)

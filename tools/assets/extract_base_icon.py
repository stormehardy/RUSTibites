import UnityPy
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
GAME = Path(r"C:\Program Files (x86)\Steam\steamapps\common\The Bibites\The Bibites_Data")
OUT = ROOT / "assets/game/reference/functional_ui_native/in_world/button_theme/sprites"
OUT.mkdir(parents=True, exist_ok=True)
found = []
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
            if name != "base_icon":
                continue
            p = OUT / "base_icon.png"
            d.image.save(p)
            found.append({"file": f.name, "path_id": o.path_id, "width": d.m_Width, "height": d.m_Height,
                          "saved": str(p.relative_to(ROOT).as_posix())})
        except Exception as e:
            print("fail", o.path_id, e)
print(found)

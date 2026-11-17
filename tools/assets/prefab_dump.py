"""prefab_dump.py - best-effort UnityPy in-world prefab hierarchy dump for task-76.

Scans the shipped Unity asset files for GameObjects whose names match the in-world prefab roots
(bibite body parts, egg, plant/meat pellets, zone, background, towers/effect prefabs), then walks
their Transform hierarchy and records each child's localPosition / localRotation / localScale and any
SpriteRenderer fields UnityPy can read without script typetrees (sortingOrder, sortingLayerID,
sortingLayerName, color, flipX/Y, size, material pointer). Output: assets/game/prefabs.json.

If the build's prefab components are not readable without typetrees, the script records that fact and
the manifest reports prefab metadata as a runtime-plugin follow-up instead of silently guessing.
"""
from __future__ import annotations

import json
from pathlib import Path

import UnityPy

DATA = Path(r"C:\Program Files (x86)\Steam\steamapps\common\The Bibites\The Bibites_Data")
OUT = Path(r"Z:\RUSTibites\assets\game\prefabs.json")
FILES = ["sharedassets1.assets", "sharedassets2.assets", "sharedassets0.assets", "resources.assets",
         "level1", "level2", "level3"]
ROOTS = ("bibite", "egg", "plant pellet", "matter pellet", "meat pellet", "pellet", "zone", "background",
         "tower", "colorkiller", "reddeath", "fieldofview")
SKIP_UI = ("canvas", "button", "text", "panel", "icon", "layout", "input", "toggle", "slider", "scroll")


def v2(v):
    return [round(float(v.x), 6), round(float(v.y), 6)] if v is not None else None


def v3(v):
    return [round(float(v.x), 6), round(float(v.y), 6), round(float(v.z), 6)] if v is not None else None


def walk_transform(env, tr, depth=0, budget=None):
    """Read a Transform hierarchy depth-first (built-in Transform/RectTransform are readable)."""
    out = []
    stack = [(tr, depth)]
    while stack:
        cur, d = stack.pop()
        if d > 8 or (budget is not None and budget[0] <= 0):
            continue
        if budget is not None:
            budget[0] -= 1
        go = None
        try:
            go = cur.m_GameObject.read() if hasattr(cur, "m_GameObject") else None
        except Exception:
            pass
        node = {"name": getattr(go, "m_Name", None), "depth": d,
                "local_position": v3(getattr(cur, "m_LocalPosition", None)),
                "local_rotation": v3(getattr(cur, "m_LocalRotation", None)),
                "local_scale": v3(getattr(cur, "m_LocalScale", None)),
                "children": []}
        if go is not None:
            for comp in (getattr(go, "m_Component", None) or []):
                cp = getattr(comp, "component", comp)
                try:
                    cd = cp.read()
                except Exception:
                    continue
                cname = type(cd).__name__
                if cname == "SpriteRenderer":
                    mat = getattr(cd, "m_Material", None)
                    mat_name = None
                    try:
                        mat_name = mat.read().m_Name if mat is not None else None
                    except Exception:
                        pass
                    sprite_ptr = getattr(cd, "m_Sprite", None)
                    sprite_name = None
                    try:
                        sprite_name = sprite_ptr.read().m_Name if sprite_ptr is not None else None
                    except Exception:
                        sprite_name = getattr(sprite_ptr, "name", None)
                    node["sprite_renderer"] = {
                        "sprite": sprite_name,
                        "sorting_layer": getattr(cd, "m_SortingLayer", None),
                        "sorting_layer_name": str(getattr(cd, "m_SortingLayerName", "") or ""),
                        "sorting_order": getattr(cd, "m_SortingOrder", None),
                        "color": [round(float(x), 4) for x in
                                  (getattr(getattr(cd, "m_Color", None), "r", 1), getattr(getattr(cd, "m_Color", None), "g", 1),
                                   getattr(getattr(cd, "m_Color", None), "b", 1), getattr(getattr(cd, "m_Color", None), "a", 1))],
                        "flip_x": bool(getattr(cd, "m_FlipX", False)),
                        "flip_y": bool(getattr(cd, "m_FlipY", False)),
                        "material": mat_name,
                    }
        out.append(node)
        children = getattr(cur, "m_Children", None) or []
        for child_ptr in children:
            try:
                child = child_ptr.read()
                stack.append((child, d + 1))
            except Exception:
                continue
    return out


def main() -> int:
    results = {"schema": "rustibites.assets.prefabs.v1", "source": "UnityPy level/sharedassets scan",
               "roots": [], "unreadable": []}
    for fname in FILES:
        p = DATA / fname
        if not p.exists():
            continue
        env = UnityPy.load(str(p))
        for obj in env.objects:
            if obj.type.name != "GameObject":
                continue
            try:
                d = obj.read()
                name = (getattr(d, "m_Name", "") or "")
            except Exception:
                continue
            low = name.lower()
            if not any(r in low for r in ROOTS) or any(s in low for s in SKIP_UI):
                continue
            ALLOW = {"bibite", "egg", "zone", "plant pellet", "meat pellet", "matter pellet",
                     "radiation tower", "colorkiller", "pherotower", "reddeath", "background",
                     "fieldofviewdisplay"}
            if low not in ALLOW:
                continue
            comp_names = []
            for c in (getattr(d, "m_Component", None) or []):
                try:
                    comp_names.append(type(getattr(c, "component", c).read()).__name__)
                except Exception:
                    comp_names.append("unreadable")
            if "RectTransform" in comp_names:
                continue  # UI prefab, not in-world
            comps = []
            for c in (getattr(d, "m_Component", None) or []):
                try:
                    cp = getattr(c, "component", c)
                    comps.append(type(cp.read()).__name__)
                except Exception as exc:
                    comps.append(f"unreadable:{str(exc)[:60]}")
            tree = []
            try:
                tr = d.m_Transform.read() if hasattr(d, "m_Transform") else None
                if tr is None:
                    for c in (d.m_Component or []):
                        cd = c.component.read()
                        if type(cd).__name__ in ("Transform", "RectTransform"):
                            tr = cd
                            break
                if tr is not None:
                    tree = walk_transform(env, tr, budget=[400])
            except Exception as exc:
                results["unreadable"].append({"file": fname, "name": name, "error": str(exc)[:200]})
            results["roots"].append({"file": fname, "name": name, "components": comps, "tree": tree})
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(results, indent=1), encoding="utf-8")
    print(json.dumps({"roots": len(results["roots"]),
                      "files": sorted({r["file"] for r in results["roots"]}),
                      "names": [r["name"] for r in results["roots"]][:40],
                      "out": str(OUT)}, indent=1))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

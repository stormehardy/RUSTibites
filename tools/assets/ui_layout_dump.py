#!/usr/bin/env python3
"""ui_layout_dump.py - extract the game's UI layout trees from Unity asset files (task-96).

Asset-file only (no game launch). UnityPy can read GameObject/RectTransform/Canvas natively, and can
resolve every MonoBehaviour's script class name through its m_Script MonoScript pointer even when the
MonoBehaviour's own typetree is stripped. That gives:

  * exact RectTransform geometry: anchorMin/anchorMax/anchoredPosition/sizeDelta/pivot/localPosition/
    localRotation/localScale, sibling order, active state, layer/tag and the full canvas hierarchy;
  * component lists with script class names (Image/Text/TextMeshProUGUI/CanvasScaler/layout groups/…);
  * Canvas component settings (render mode, sorting order, pixel perfect, target display, etc.);
  * the game's UIScaler formula reconstructed from Here/Utility/UIScaler.cs (CanvasScaler.scaleFactor =
    ScreenResolutionFactor * UIScale; auto ScreenResolutionFactor = min(h/1080, w/1920)).

Known gap: MonoBehaviour *serialized fields* (Image.sprite/color/type/fillAmount, TMP font/size/color/
alignment, CanvasScaler reference resolution/match, LayoutGroup paddings, UIScript button refs) are not
readable without Unity script typetrees; the component class names are still resolved here and the
geometry is exact. Those fields can be filled by a runtime plugin UI dump (recommended follow-up) or,
for sprites/fonts, joined through spec-dev's ui_atlas.json + Here/ UI script defaults.

Output: assets/game/ui_layout.json (deterministic; run twice -> identical sha256).
Usage:  python tools/assets/ui_layout_dump.py [--game-data DIR] [--out PATH]
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
DEFAULT_GAME = Path(r"C:\Program Files (x86)\Steam\steamapps\common\The Bibites\The Bibites_Data")
DEFAULT_OUT = ROOT / "assets" / "game" / "ui_layout.json"
FILES = ["level1", "level2", "level3", "level4", "sharedassets0.assets", "sharedassets1.assets",
         "sharedassets2.assets"]
CORE_HINTS = ("hud", "stats", "genes", "biology", "brain", "panel", "inspector", "legend", "tooltip",
              "selection", "tab", "playback", "clock", "population")


def v2(v):
    return None if v is None else [round(float(v.x), 6), round(float(v.y), 6)]


def v3(v):
    return None if v is None else [round(float(v.x), 6), round(float(v.y), 6), round(float(v.z), 6)]


def quat(q):
    return None if q is None else [round(float(q.x), 8), round(float(q.y), 8), round(float(q.z), 8), round(float(q.w), 8)]


def script_name(mono, cache):
    try:
        ptr = getattr(mono, "m_Script", None)
        if ptr is None:
            return None
        key = (getattr(ptr, "m_FileID", 0), getattr(ptr, "m_PathID", 0))
        if key in cache:
            return cache[key]
        s = ptr.read()
        ns = getattr(s, "m_Namespace", "") or ""
        cls = getattr(s, "m_ClassName", None) or getattr(s, "m_Name", None)
        name = (ns + "." + cls) if ns else cls
        cache[key] = name
        return name
    except Exception:
        return None


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--game-data", default=str(DEFAULT_GAME))
    ap.add_argument("--out", default=str(DEFAULT_OUT))
    args = ap.parse_args()
    import UnityPy
    game = Path(args.game_data)
    out = Path(args.out)
    doc = {"schema": "rustibites.assets.ui_layout.v1", "generatedBy": "tools/assets/ui_layout_dump.py",
           "game": {"name": "The Bibites", "unity": "6000.0.44f1"},
           "method": "UnityPy static asset walk: RectTransform/Canvas native fields + MonoBehaviour script "
                     "class resolution via m_Script MonoScript; no game launch.",
           "uiscaler": {
               "source": "Here/Utility/UIScaler.cs",
               "formula": "CanvasScaler.scaleFactor = totalScale = ScreenResolutionFactor * UIScale",
               "auto_resolution_factor": "ScreenResolutionFactor = min(Screen.height/1080, Screen.width/1920) "
                                         "when AutomaticallyDetectScreenResolution is on, else the closest "
                                         "landmark setting value",
               "note": "CanvasScaler serialized referenceResolution/matchWidthOrHeight are not readable "
                       "without script typetrees; UIScale is a user setting (default 1.0).",
           },
           "files": {}, "canvases": [], "trees": [], "gaps": {}}
    script_cache = {}
    counters = {"rect_nodes": 0, "gameobjects": 0, "components": 0, "components_unreadable_fields": 0,
                "scripts_resolved": 0, "scripts_unresolved": 0}
    class_counts = {}
    canvas_scalers = []
    for fname in FILES:
        path = game / fname
        if not path.exists():
            continue
        env = UnityPy.load(str(path))
        gos, transforms, canvases = {}, {}, {}
        # (counted after the pass below)
        comp_info = {}  # component path_id -> dict
        # pass 1: native objects
        for obj in env.objects:
            t = obj.type.name
            try:
                if t == "GameObject":
                    gos[obj.path_id] = obj.read()
                elif t in ("RectTransform", "Transform"):
                    transforms[obj.path_id] = obj.read()
                elif t == "Canvas":
                    canvases[obj.path_id] = obj.read()
            except Exception:
                continue
        transform_to_go = {}
        # script class map for every MonoBehaviour in the file (their typetrees are stripped, but the
        # base object + m_Script pointer are readable from the ObjectReader)
        script_map = {}
        for obj in env.objects:
            if obj.type.name != "MonoBehaviour":
                continue
            try:
                mono = obj.read(check_read=False)
                script_map[obj.path_id] = (script_name(mono, script_cache), bool(getattr(mono, "m_Enabled", 1)))
            except Exception:
                continue
        go_to_rect = {}
        for pid, tr in transforms.items():
            try:
                transform_to_go[pid] = tr.m_GameObject.m_PathID if tr.m_GameObject else 0
            except Exception:
                continue
        for pid, tr in transforms.items():
            try:
                gpid = tr.m_GameObject.m_PathID if tr.m_GameObject else 0
            except Exception:
                gpid = 0
            if pid in transforms and gpid in gos:
                go_to_rect[gpid] = tr
        # pass 2: component metadata (script classes for MonoBehaviours)
        counters["gameobjects"] += len(gos)
        for gpid, go in gos.items():
            comps = []
            for c in (getattr(go, "m_Component", None) or []):
                ptr = getattr(c, "component", c)
                try:
                    cpid = ptr.m_PathID if hasattr(ptr, "m_PathID") else None
                    try:
                        obj = ptr.read()
                        ctype = type(obj).__name__
                    except Exception:
                        obj = None
                        ctype = "MonoBehaviour" if cpid in script_map else "unreadable"
                    info = {"type": ctype}
                    if ctype == "MonoBehaviour":
                        script, enabled = script_map.get(cpid, (None, True))
                        info["script"] = script
                        info["enabled"] = enabled
                        info["fields_readable"] = False
                        counters["components_unreadable_fields"] += 1
                        if script:
                            counters["scripts_resolved"] += 1
                            class_counts[script] = class_counts.get(script, 0) + 1
                        else:
                            counters["scripts_unresolved"] += 1
                    elif ctype == "Canvas":
                        cv = obj
                        info.update({"render_mode": int(getattr(cv, "m_RenderMode", 0)),
                                     "sorting_order": int(getattr(cv, "m_SortingOrder", 0)),
                                     "override_sorting": bool(getattr(cv, "m_OverrideSorting", 0)),
                                     "pixel_perfect": bool(getattr(cv, "m_PixelPerfect", 0)),
                                     "plane_distance": float(getattr(cv, "m_PlaneDistance", 0.0)),
                                     "target_display": int(getattr(cv, "m_TargetDisplay", 0))})
                    comps.append(info)
                    counters["components"] += 1
                except Exception as exc:
                    comps.append({"type": "unreadable", "error": str(exc)[:120]})
            try:
                comp_info[gpid] = comps
            except Exception:
                pass
        # build rect nodes + trees
        node_by_go = {}
        parent_of = {}
        children_of = {}
        for gpid, tr in go_to_rect.items():
            go = gos.get(gpid)
            if go is None:
                continue
            node = {
                "name": getattr(go, "m_Name", ""),
                "source_file": fname,
                "path_id": gpid,
                "active": bool(getattr(go, "m_IsActive", 1)),
                "layer": int(getattr(go, "m_Layer", 0)),
                "tag": str(getattr(go, "m_TagString", "") or ""),
                "rect": {
                    "anchor_min": v2(getattr(tr, "m_AnchorMin", None)),
                    "anchor_max": v2(getattr(tr, "m_AnchorMax", None)),
                    "anchored_position": v2(getattr(tr, "m_AnchoredPosition", None)),
                    "size_delta": v2(getattr(tr, "m_SizeDelta", None)),
                    "pivot": v2(getattr(tr, "m_Pivot", None)),
                    "local_position": v3(getattr(tr, "m_LocalPosition", None)),
                    "local_rotation": quat(getattr(tr, "m_LocalRotation", None)),
                    "local_scale": v3(getattr(tr, "m_LocalScale", None)),
                },
                "components": comp_info.get(gpid, []),
                "children": [],
            }
            node_by_go[gpid] = node
            counters["rect_nodes"] += 1
            # m_Father/m_Children are Transform path ids; translate to GameObject ids
            try:
                fp_tr = tr.m_Father.m_PathID if tr.m_Father else 0
                parent_of[gpid] = transform_to_go.get(fp_tr, 0)
            except Exception:
                parent_of[gpid] = 0
            for ch in (getattr(tr, "m_Children", None) or []):
                try:
                    child_tr_id = ch.m_PathID
                    cgo = transform_to_go.get(child_tr_id, 0)
                    if cgo:
                        children_of.setdefault(gpid, []).append(cgo)
                except Exception:
                    continue
        # attach children in transform order
        for gpid, node in node_by_go.items():
            node["children"] = [node_by_go[c] for c in children_of.get(gpid, []) if c in node_by_go]
        roots = [node for gpid, node in node_by_go.items() if parent_of.get(gpid, 0) not in node_by_go]
        # canvases
        canvas_roots = []
        for gpid, node in node_by_go.items():
            if any(c.get("type") == "Canvas" for c in node["components"]):
                node["is_canvas_root"] = True
                root = node
                while True:
                    pr = parent_of.get(root["path_id"], 0)
                    if pr in node_by_go:
                        root = node_by_go[pr]
                    else:
                        break
                canvas_roots.append({"canvas": node["name"], "path_id": node["path_id"],
                                     "active": node["active"], "level_root": root["name"],
                                     "root_path_id": root["path_id"]})
            for c in node["components"]:
                if c.get("script") and "CanvasScaler" in str(c.get("script")):
                    canvas_scalers.append({"file": fname, "node": node["name"], "path_id": node["path_id"],
                                           "script": c["script"], "fields_readable": False})
        # depth
        def depth(n):
            if not n["children"]:
                return 1
            return 1 + max(depth(c) for c in n["children"])
        max_depth = max((depth(r) for r in roots), default=0)
        doc["files"][fname] = {"gameobjects": len(gos), "rect_nodes": len(node_by_go),
                               "roots": len(roots), "canvas_roots": len(canvas_roots),
                               "max_depth": max_depth}
        doc["canvases"].extend([{"source_file": fname, **c} for c in canvas_roots])
        doc["trees"].extend(roots)
    doc["stats"] = {**counters, "files": {f: v["rect_nodes"] for f, v in doc["files"].items()},
                    "component_class_counts": dict(sorted(class_counts.items(), key=lambda kv: -kv[1])[:40]),
                    "canvas_scaler_nodes": canvas_scalers}
    doc["gaps"] = {
        "monobehaviour_fields": "Image.sprite/color/type/fillAmount, TMP_Text font/size/alignment/color, "
                                "CanvasScaler referenceResolution/match, layout-group paddings and UIScript "
                                "references are serialized in MonoBehaviours whose typetrees are stripped: "
                                "UnityPy reads the component class via m_Script but not its fields. "
                                "Geometry (RectTransform) and Canvas fields ARE exact. Fill the gap with a "
                                "runtime plugin UI dump (same approach as ParityDump /d/feeding) or join "
                                "spec-dev's ui_atlas.json + Here/ defaults for sprites/fonts.",
        "sprite_font_resolution": "not resolvable statically from MonoBehaviour fields; resolve via "
                                  "tools/ui/ui_atlas.json (logical asset names) + Here/ inspector defaults, "
                                  "or the recommended runtime plugin dump.",
        "canvas_scaler_values": "CanvasScaler serialized values unreadable; formula from Here/Utility/UIScaler.cs "
                                "is included in uiscaler above (scaleFactor = ScreenResolutionFactor * UIScale; "
                                "auto factor = min(h/1080, w/1920)).",
    }
    out.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(doc, indent=1, sort_keys=False)
    out.write_text(text, encoding="utf-8")
    print(json.dumps({"out": str(out), "bytes": len(text.encode("utf-8")),
                      "sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
                      "stats": {k: doc["stats"][k] for k in ("rect_nodes", "gameobjects", "components",
                                                             "components_unreadable_fields", "scripts_resolved",
                                                             "scripts_unresolved")},
                      "files": doc["files"], "canvases": len(doc["canvases"])}, indent=1))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

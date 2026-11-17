#!/usr/bin/env python3
"""ui_runtime_join.py - join the live runtime UI dump with asset layout/manifest/atlas (task-102).

Reads `assets/game/ui_runtime.json` (hidden-window runtime fields), `assets/game/ui_layout.json`
(asset-file geometry), `assets/game/manifest.json` (sprite/font files + rect/pivot/PPU/border) and
`tools/ui/ui_atlas.json` (logical UI roles/aliases/9-slice/fonts), then writes
`tools/ui/ui_runtime_atlas.json`: the consumer file for view-dev with exact runtime RectTransforms +
resolved Image sprite/file/border/color/type/fill + TMP font role/name/size/alignment/color/text +
CanvasScaler values + layout-group paddings, per session (main menu, Default world).

Validation: missing sprite/font names are listed, node/panel coverage and field-completeness counts are
reported. No game needed; run after tools/assets/capture_ui_runtime.py.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
RUNTIME = ROOT / "assets" / "game" / "ui_runtime.json.gz"
LAYOUT = ROOT / "assets" / "game" / "ui_layout.json"
MANIFEST = ROOT / "assets" / "game" / "manifest.json"
ATLAS = ROOT / "tools" / "ui" / "ui_atlas.json"
OUT = ROOT / "tools" / "ui" / "ui_runtime_atlas.json.gz"
CORE_HINTS = ("hud", "stats", "genes", "biology", "brain", "inspector", "tooltip", "playback",
              "species", "lineage", "panel", "clock", "population", "settings", "button")


def load(p):
    import gzip
    p = Path(p)
    if p.suffix == ".gz":
        return json.loads(gzip.open(p, "rt", encoding="utf-8").read())
    return json.loads(p.read_text(encoding="utf-8"))


def lower_map_by_names(entry_names):
    out = {}
    for logical, entry in (entry_names or {}).items():
        if not isinstance(entry, dict):
            continue
        names = [logical, entry.get("manifest_name"), entry.get("alias_used"), *(entry.get("aliases") or [])]
        for n in names:
            if n:
                out.setdefault(str(n).lower(), entry)
    return out


def build_maps():
    manifest = load(MANIFEST)
    atlas = load(ATLAS)
    sprites = {}
    for name, e in (manifest.get("sprites") or {}).items():
        sprites.setdefault(name.lower(), {"file": e.get("file"), "rect": e.get("rect"), "pivot": e.get("pivot"),
                                          "border": e.get("border"), "pixels_per_unit": e.get("pixels_per_unit"),
                                          "source": "sprites"})
    for fname, group in (manifest.get("ui") or {}).items():
        for name, e in (group.get("sprites") or {}).items():
            sprites.setdefault(name.lower(), {"file": e.get("file"), "rect": e.get("rect"), "pivot": e.get("pivot"),
                                              "border": e.get("border"), "pixels_per_unit": e.get("pixels_per_unit"),
                                              "source": "ui.sprites", "asset_file": fname})
        for name, e in (group.get("textures") or {}).items():
            sprites.setdefault(name.lower(), {"file": e.get("file"), "source": "ui.textures", "asset_file": fname})
    atlas_entry = (atlas.get("atlas") or {})
    atlas_by_name = lower_map_by_names(atlas_entry)
    fonts = {}
    for f in manifest.get("fonts") or []:
        fonts.setdefault(str(f.get("name") or "").lower(), {"name": f.get("name"), "file": f.get("file")})
    font_atlas = {}
    for logical, e in (atlas.get("fonts") or {}).items():
        if isinstance(e, dict):
            for n in (logical, e.get("manifest_name"), e.get("alias_used"), *(e.get("aliases") or []), e.get("file")):
                if n:
                    font_atlas.setdefault(str(n).lower(), e)
    return sprites, atlas_by_name, atlas_entry, fonts, font_atlas


def resolve_sprite(name, sprites, atlas_by_name):
    if not name:
        return None
    key = str(name).lower()
    atlas = atlas_by_name.get(key)
    manifest = sprites.get(key)
    out = {"name": name}
    if atlas:
        out.update({"logical": atlas.get("logical"), "file": atlas.get("file"), "kind": atlas.get("kind"),
                    "border": atlas.get("border"), "default_tint": atlas.get("default_tint"),
                    "manifest_name": atlas.get("manifest_name")})
    if manifest:
        out.setdefault("file", manifest.get("file"))
        out["manifest"] = {k: manifest.get(k) for k in ("file", "rect", "pivot", "pixels_per_unit", "border") if manifest.get(k) is not None}
    if not atlas and not manifest:
        out["missing"] = True
    return out


def resolve_font(name, fonts, font_atlas):
    if not name:
        return None
    key = str(name).lower()
    out = {"name": name}
    atlas = font_atlas.get(key)
    if atlas:
        out.update({"role": atlas.get("role"), "logical": atlas.get("logical"), "file": atlas.get("file"),
                    "manifest_name": atlas.get("manifest_name")})
    manifest = fonts.get(key)
    if manifest:
        out.setdefault("file", manifest.get("file"))
    if not atlas and not manifest:
        # fall back to substring match (TMP asset names carry SDF suffixes)
        for k, e in font_atlas.items():
            if key.split(" ")[0] and key.split(" ")[0] in k:
                out.update({"role": e.get("role"), "logical": e.get("logical"), "file": e.get("file"),
                            "manifest_name": e.get("manifest_name"), "match": "substring"})
                return out
        out["missing"] = True
    return out


def join_node(node, sprites, atlas_by_name, fonts, font_atlas, stats):
    o = {"name": node.get("name"), "path": node.get("path"), "active": node.get("active_self"),
         "active_in_hierarchy": node.get("active_in_hierarchy"),
         "sibling_index": node.get("sibling_index"), "rect": node.get("rect")}
    comps = node.get("components") or []
    imgs, texts, scalers, layouts, selects = [], [], [], [], []
    for c in comps:
        t = c.get("type") or ""
        stats["components"] += 1
        if t.endswith(".Image") or t.endswith(".RawImage"):
            spr = resolve_sprite(c.get("sprite") or c.get("texture"), sprites, atlas_by_name)
            if c.get("sprite") or c.get("texture"):
                stats["image_components"] += 1
                if spr and spr.get("missing"):
                    stats["missing_sprites"].append(c.get("sprite") or c.get("texture"))
            imgs.append({"type": t, "sprite": spr, "color": c.get("color"), "image_type": c.get("image_type"),
                         "fill_method": c.get("fill_method"), "fill_amount": c.get("fill_amount"),
                         "preserve_aspect": c.get("preserve_aspect"),
                         "runtime_sprite_border": c.get("sprite_border"),
                         "sprite_rect": c.get("sprite_rect"), "sprite_ppu": c.get("sprite_pixels_per_unit")})
        elif "TextMeshPro" in t or t.endswith(".Text"):
            font = resolve_font(c.get("font"), fonts, font_atlas)
            if c.get("font"):
                stats["text_components"] += 1
                if font and font.get("missing"):
                    stats["missing_fonts"].append(c.get("font"))
            texts.append({"type": t, "text": c.get("text"), "font": font, "font_size": c.get("font_size"),
                          "alignment": c.get("alignment"), "color": c.get("color"),
                          "word_wrap": c.get("word_wrap"), "margin": c.get("margin"),
                          "font_style": c.get("font_style"), "character_spacing": c.get("character_spacing"),
                          "line_spacing": c.get("line_spacing"), "rich_text": c.get("rich_text")})
        elif t.endswith(".CanvasScaler"):
            scalers.append({k: c.get(k) for k in ("ui_scale_mode", "reference_resolution", "screen_match_mode",
                                                   "match_width_or_height", "scale_factor",
                                                   "reference_pixels_per_unit")})
            stats["canvas_scalers"] += 1
        elif "LayoutGroup" in t or t.endswith(".LayoutElement") or t.endswith(".ContentSizeFitter"):
            layouts.append(c)
            stats["layout_components"] += 1
        elif any(t.endswith(s) for s in (".Button", ".Toggle", ".Slider", ".Scrollbar", ".ScrollRect",
                                         ".Selectable", ".Canvas", ".CanvasGroup", ".Mask", ".RectMask2D")):
            selects.append(c)
            stats["selectable_canvas_components"] += 1
    if imgs: o["images"] = imgs
    if texts: o["texts"] = texts
    if scalers: o["canvas_scalers"] = scalers
    if layouts: o["layouts"] = layouts
    if selects: o["controls"] = selects
    kids = [join_node(k, sprites, atlas_by_name, fonts, font_atlas, stats) for k in (node.get("children") or [])]
    if kids: o["children"] = kids
    return o


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--runtime", default=str(RUNTIME))
    ap.add_argument("--out", default=str(OUT))
    args = ap.parse_args()
    sprites, atlas_by_name, atlas_entry, fonts, font_atlas = build_maps()
    runtime = load(args.runtime)
    sessions = runtime.get("sessions") or {}
    stats = {"components": 0, "image_components": 0, "text_components": 0, "canvas_scalers": 0,
             "layout_components": 0, "selectable_canvas_components": 0,
             "missing_sprites": [], "missing_fonts": []}
    joined_sessions = {}
    for name, dump in sessions.items():
        if not dump:
            continue
        trees = [join_node(n, sprites, atlas_by_name, fonts, font_atlas, stats) for n in (dump.get("canvases") or [])]
        paths = []
        def walk(nodes):
            for n in nodes:
                paths.append(n.get("path") or "")
                walk(n.get("children") or [])
        walk(trees)
        core = sorted({p for p in paths if any(h in (p or "").lower() for h in CORE_HINTS)})
        joined_sessions[name] = {"schema": dump.get("schema"), "node_count": dump.get("node_count"),
                                 "screen": dump.get("screen"), "cameras": dump.get("cameras"),
                                 "canvases": trees,
                                 "core_panel_paths": core[:400], "core_panel_count": len(core)}
    out = {
        "schema": "rustibites.assets.ui_runtime_atlas.v1",
        "generatedBy": "tools/assets/ui_runtime_join.py",
        "sources": {"runtime": str(RUNTIME), "layout": str(LAYOUT), "manifest": str(MANIFEST),
                    "atlas": str(ATLAS)},
        "capture": runtime.get("capture"), "determinism": runtime.get("determinism"),
        "sessions": joined_sessions,
        "stats": {**stats, "missing_sprites": sorted(set(map(str, stats["missing_sprites"])))[:200],
                  "missing_fonts": sorted(set(map(str, stats["missing_fonts"])))[:200],
                  "font_atlas_roles": {k: v.get("role") if isinstance(v, dict) else None
                                       for k, v in (atlas_entry and {}).items()} if False else None},
    }
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    payload = json.dumps(out, indent=1).encode("utf-8")
    if out_path.suffix == ".gz":
        import gzip
        with gzip.open(out_path, "wb", compresslevel=9) as g:
            g.write(payload)
    else:
        out_path.write_bytes(payload)
    index = {
        "schema": "rustibites.assets.ui_runtime_atlas.index.v1",
        "files": {"joined": str(out_path.relative_to(ROOT)) if out_path.is_relative_to(ROOT) else str(out_path),
                  "format": "gzipped JSON (decompress with gzip; schema rustibites.assets.ui_runtime_atlas.v1)",
                  "runtime": str(RUNTIME), "layout": str(LAYOUT), "manifest": str(MANIFEST), "atlas": str(ATLAS)},
        "capture": runtime.get("capture"), "determinism": runtime.get("determinism"),
        "sessions": {k: {"nodes": v["node_count"], "core_panels": v["core_panel_count"]}
                     for k, v in joined_sessions.items()},
        "stats": out["stats"],
        "consumer_schema": {
            "sessions.<session>.canvases[]": "runtime hierarchy; each node has name/path/rect/children and "
                                             "resolved images[]/texts[]/canvas_scalers[]/layouts[]/controls[]",
            "node.rect": "anchor_min/anchor_max/anchored_position/size_delta/pivot/local_position/"
                         "local_rotation/local_scale (runtime values; authored copy in ui_layout.json)",
            "node.images[]": "{sprite{name,logical,file,manifest rect/pivot/border}, color, image_type, "
                             "fill_method, fill_amount, preserve_aspect}",
            "node.texts[]": "{text, font{name,role,logical,file}, font_size, alignment, color, word_wrap}",
            "node.canvas_scalers[]": "{reference_resolution, screen_match_mode, match_width_or_height, "
                                     "scale_factor}",
            "node.layouts[]": "LayoutGroup/LayoutElement fields incl padding, spacing, child alignment/control",
            "missing": "stats.missing_sprites / stats.missing_fonts list names not resolvable in manifest/atlas",
        },
    }
    idx_path = out_path.with_name(out_path.name.replace(".json.gz", ".index.json"))
    idx_path.write_text(json.dumps(index, indent=1), encoding="utf-8")
    print(json.dumps({"out": str(out_path), "bytes": out_path.stat().st_size,
                      "sessions": {k: {"nodes": v["node_count"], "core_panels": v["core_panel_count"]}
                                   for k, v in joined_sessions.items()},
                      "stats": {k: out["stats"][k] for k in ("components", "image_components", "text_components",
                                                            "canvas_scalers", "layout_components",
                                                            "selectable_canvas_components")},
                      "missing_sprites": len(out["stats"]["missing_sprites"]),
                      "missing_fonts": len(out["stats"]["missing_fonts"])}, indent=1))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Build ``tools/ui/ui_atlas.json`` for the viewer's UI layer (task-84).

Consumes the two upstream artefacts and resolves them into one implementation-ready map:

* ``tools/assets/ui_expectations.json`` — logical UI asset catalogue (242 entries) produced by
  ``tools/assets/build_ui_expectations.py``.
* ``assets/game/manifest.json`` — parity-dev's extraction output (read-only; we never write it).

Output: for every logical UI asset, the extracted file (sprite/font/material), kind
(``sprite``/``nine_slice``/``icon``/``font``/``material``/``procedural``), 9-slice borders and pivot
(from Unity metadata when present, otherwise inferred with ``needs_prefab``), default tint/draw
size, font-role mapping (TTF + TMP SDF atlas), brain-icon index tables (33 inputs / 15 outputs /
13 activations), materials, and an "implement first" checklist for view-dev.

Deterministic: no timestamps/absolute paths; sorted output. Run::

    python tools/ui/build_ui_atlas.py
    python tools/ui/check_ui_atlas.py            # validate + coverage
    python tools/ui/check_ui_atlas.py --strict   # fail on required gaps
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EXPECTATIONS = ROOT / "tools" / "assets" / "ui_expectations.json"
MANIFEST = ROOT / "assets" / "game" / "manifest.json"
OUT = ROOT / "tools" / "ui" / "ui_atlas.json"

KINDS = {"sprite", "nine_slice", "icon", "font", "material", "procedural"}
CORE_PRIORITIES = {"P0", "P0b", "P1"}

# Inferred 9-slice borders when Unity metadata has no border (logical units to confirm against
# prefabs/runtime dump). Value order matches Unity Sprite.border: [left, bottom, right, top].
INFERRED_BORDERS = {
    "panel": [10, 10, 10, 10],
    "button": [30, 30, 30, 30],
    "toggle": [8, 8, 8, 8],
    "slider": [4, 4, 4, 4],
    "progress": [4, 4, 4, 4],
    "dropdown": [10, 10, 10, 10],
    "input": [10, 10, 10, 10],
    "tabs": [10, 10, 10, 10],
    "scroll": [4, 4, 4, 4],
    "tooltip": [10, 10, 10, 10],
    "popup": [12, 12, 12, 12],
    "notification": [8, 8, 8, 8],
    "menu": [10, 10, 10, 10],
    "graph": [2, 2, 2, 2],
    "chart": [8, 8, 8, 8],
    "icon": [0, 0, 0, 0],
    "hud": [10, 10, 10, 10],
    "overlay": [0, 0, 0, 0],
}

# Inferred draw-size defaults (px at 1080p design canvas). Precise prefab rects need a runtime dump.
INFERRED_DRAW_SIZE = {
    "icon": (32, 32),
    "button": (160, 40),
    "toggle": (28, 28),
    "slider": (200, 20),
    "progress": (180, 12),
    "dropdown": (200, 36),
    "input": (200, 32),
    "tabs": (120, 36),
    "scroll": (12, 120),
    "tooltip": (300, 80),
    "popup": (480, 260),
    "notification": (360, 56),
    "graph": (480, 260),
    "chart": (200, 200),
    "hud": (240, 60),
    "overlay": (16, 16),
    "menu": (480, 240),
    "panel": None,
    "font": None,
    "material": None,
    "procedural": None,
}

SPECIAL_TINTS = {
    "ui/icon/star_locked": [110, 110, 110, 255],
    "ui/icon/star_unlocked": [255, 210, 80, 255],
    "ui/hud/statusbar_health_fill": None,  # computed by viewer from health ratio (green->red)
    "ui/hud/statusbar_energy_fill": None,  # computed/tinted by viewer
    "ui/overlay/selection_rect": [255, 255, 255, 200],
}


def load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def norm(value: str) -> str:
    """Normalise an asset name for matching (lowercase, forward slashes, no extension/prefix)."""
    value = value.strip().replace("\\", "/").lower()
    while value.startswith("./"):
        value = value[2:]
    for prefix in ("assets/game/", "game/", "assets/"):
        if value.startswith(prefix):
            value = value[len(prefix) :]
    for ext in (".png", ".jpg", ".jpeg", ".tga", ".ttf", ".otf", ".json", ".asset"):
        if value.endswith(ext):
            value = value[: -len(ext)]
    return value


def variants(value: str) -> list[str]:
    base = norm(value)
    out = {base, base.replace(" ", "_"), base.replace("_", " "), base.replace(" ", "")}
    if "/" in base:
        tail = base.rsplit("/", 1)[-1]
        out.update({tail, tail.replace(" ", "_"), tail.replace("_", " ")})
    return sorted(v for v in out if v)


def border_list(border: dict | None) -> list[float] | None:
    if not isinstance(border, dict):
        return None
    values = [float(border.get(k, 0.0) or 0.0) for k in ("x", "y", "z", "w")]
    return values if any(v != 0.0 for v in values) else None


class AssetIndex:
    """Index of manifest UI sprites/textures/fonts/materials by normalised name variants."""

    def __init__(self, manifest: dict) -> None:
        self.sprites: dict[str, list[dict]] = {}
        self.textures: dict[str, list[dict]] = {}
        self.world_sprites: dict[str, list[dict]] = {}
        self.world_textures: dict[str, list[dict]] = {}
        self.fonts: dict[str, dict] = {}
        self.materials: dict[str, dict] = {}
        self.ui_materials: dict[str, dict] = {}
        self.tmp_atlases: dict[str, dict] = {}

        for asset, groups in (manifest.get("ui") or {}).items():
            if not isinstance(groups, dict):
                continue
            self._index_sprites(groups.get("sprites"), self.sprites, asset)
            self._index_textures(groups.get("textures"), self.textures, asset)
        self._index_sprites(manifest.get("sprites"), self.world_sprites, "assets")
        self._index_textures(manifest.get("textures"), self.world_textures, "assets")
        for font in manifest.get("fonts") or []:
            name = font.get("name", "")
            for key in variants(name):
                self.fonts.setdefault(key, font)
        for material in manifest.get("materials") or []:
            self._index_material(material)
        for key, material in ((manifest.get("ui") or {}).get("materials") or {}).items():
            record = dict(material)
            record.setdefault("name", key)
            record["logical_key"] = key
            self.ui_materials[norm(key)] = record
            self._index_material(record)
        # TMP SDF atlas textures (used for font rendering).
        for bucket in (self.textures, self.world_textures):
            for name, entries in bucket.items():
                for entry in entries:
                    tex_name = entry.get("name", "")
                    if "SDF Atlas" in tex_name or tex_name.endswith(" Atlas"):
                        self.tmp_atlases[norm(tex_name)] = entry

    def _index_sprites(self, group, bucket, asset: str) -> None:
        if not isinstance(group, dict):
            return
        for entry in group.values():
            if not isinstance(entry, dict):
                continue
            self._add(bucket, entry.get("name", ""), entry)
            self._add(bucket, entry.get("file", ""), entry)
            entry.setdefault("source_asset", asset)

    def _index_textures(self, group, bucket, asset: str) -> None:
        if not isinstance(group, dict):
            return
        for entry in group.values():
            if not isinstance(entry, dict):
                continue
            self._add(bucket, entry.get("name", ""), entry)
            self._add(bucket, entry.get("file", ""), entry)
            entry.setdefault("source_asset", asset)

    def _add(self, bucket: dict[str, list[dict]], name: str, entry: dict) -> None:
        if not name:
            return
        for key in variants(name):
            bucket.setdefault(key, []).append(entry)

    def _index_material(self, material: dict) -> None:
        for key in variants(material.get("name", "")):
            self.materials.setdefault(key, material)
        for key in variants(material.get("shader", "")):
            self.materials.setdefault(key, material)
        if material.get("logical_key"):
            self.materials.setdefault(norm(material["logical_key"]), material)

    def find_sprite(self, aliases: list[str]) -> tuple[dict | None, str | None, str | None]:
        for alias in aliases:
            for key in variants(alias):
                for bucket, kind in ((self.sprites, "ui"), (self.world_sprites, "world")):
                    hits = bucket.get(key)
                    if hits:
                        return sorted(hits, key=lambda e: (e.get("file", "")))[0], kind, alias
        return None, None, None

    def find_texture(self, aliases: list[str]) -> tuple[dict | None, str | None, str | None]:
        for alias in aliases:
            for key in variants(alias):
                for bucket, kind in ((self.textures, "ui"), (self.world_textures, "world")):
                    hits = bucket.get(key)
                    if hits:
                        return sorted(hits, key=lambda e: (e.get("file", "")))[0], kind, alias
        return None, None, None

    def find_material(self, aliases: list[str]) -> tuple[dict | None, str | None]:
        for alias in aliases:
            for key in variants(alias):
                material = self.ui_materials.get(key) or self.materials.get(key)
                if material:
                    return material, alias
        return None, None

    def find_font(self, aliases: list[str]) -> tuple[dict | None, str | None]:
        for alias in aliases:
            for key in variants(alias):
                font = self.fonts.get(key)
                if font:
                    return font, alias
        return None, None


def material_view(record: dict) -> dict:
    floats = record.get("floats") or {}
    colors = {k: v for k, v in (record.get("colors") or {}).items() if v is not None}
    interesting = {
        k: floats[k]
        for k in sorted(floats)
        if k in {"_hue", "_offset", "_hueShift", "_width", "_Color", "_FromHue", "_ToHue"}
        or k.startswith(("_Vmin", "_Vmax", "_Color"))
    }
    return {
        "name": record.get("name"),
        "shader": record.get("shader") or None,
        "file": record.get("file"),
        "source_asset": record.get("source_asset"),
        "colors": colors,
        "floats": interesting,
        "logical_key": record.get("logical_key"),
    }


def font_role(roles: dict, name: str, role: str, *, needles: list[str], atlas_needles: list[str]) -> dict:
    font = None
    for needle in needles:
        for key, entry in roles["_font_index"].items():
            if norm(entry.get("name", "")) == norm(needle):
                font = entry
                break
        if font:
            break
    atlas = None
    for needle in atlas_needles:
        for key, entry in roles["_atlas_index"].items():
            if norm(entry.get("name", "")) == norm(needle):
                atlas = entry
                break
        if atlas:
            break
    return {
        "role": role,
        "logical": f"ui/font/{role}",
        "ttf": font.get("file") if font else None,
        "ttf_name": font.get("name") if font else None,
        "tmp_atlas": atlas.get("file") if atlas else None,
        "tmp_atlas_name": atlas.get("name") if atlas else None,
        "reference_size_px": 16 if role in {"body", "mono_numeric"} else 20,
        "needs_prefab": True,
        "confidence": "inferred",
        "notes": "Exact TMP font asset/prefab size needs a runtime prefab dump; role mapping inferred from font names + TMP SDF atlas assets.",
    }


def build(expectations: dict, manifest: dict) -> dict:
    index = AssetIndex(manifest)
    entries = expectations["entries"]
    atlas: dict[str, dict] = {}
    unmatched: list[dict] = []
    shared: dict[str, list[str]] = {}

    for entry in entries:
        logical = entry["logical"]
        priority = entry["priority"]
        usage = entry["usage"]
        aliases = list(entry.get("manifest_aliases") or [])
        # Logical basename is also a candidate (materials/achievements/fonts use game's own names).
        basename = logical.rsplit("/", 1)[-1]
        if basename not in aliases:
            aliases.append(basename)
        extraction = entry.get("extraction", "sprite_png")
        record: dict = {
            "logical": logical,
            "id": entry.get("id"),
            "priority": priority,
            "usage": usage,
            "surfaces": list(entry.get("surfaces") or []),
            "kind": "sprite",
            "status": "unmatched",
            "needs_prefab": True,
            "file": None,
            "manifest_name": None,
            "source_asset": None,
            "sheet": None,
            "rect": None,
            "natural_size_px": None,
            "pivot": [0.5, 0.5],
            "border": None,
            "nine_slice": bool(entry.get("nine_slice")),
            "tintable": usage in {"icon", "button", "toggle", "progress", "overlay", "hud"},
            "default_tint": list(SPECIAL_TINTS.get(logical) or [255, 255, 255, 255]),
            "default_draw_size_px": None,
            "size_inferred": True,
            "aliases": aliases,
            "extraction": extraction,
            "notes": entry.get("notes", ""),
            "m1_core_ui": priority in CORE_PRIORITIES,
        }

        if extraction == "runtime_procedural":
            record.update(kind="procedural", status="runtime", needs_prefab=False)
            atlas[logical] = record
            continue

        if extraction == "font_asset":
            record.update(kind="font", status="font_role")
            atlas[logical] = record
            continue

        if extraction == "shader_material":
            material, alias = index.find_material(aliases)
            record["kind"] = "material"
            if material:
                record.update(
                    status="mapped",
                    needs_prefab=False,
                    manifest_name=material.get("name"),
                    file=material.get("file"),
                    source_asset=material.get("source_asset"),
                    material=material_view(material),
                    alias_used=alias,
                )
            else:
                record["status"] = "unmatched"
                record["needs_prefab"] = True
                unmatched.append({"logical": logical, "kind": "material", "aliases": aliases})
            atlas[logical] = record
            continue

        # Sprite / icon / nine-slice resolution.
        sprite, source, alias = index.find_sprite(aliases)
        if sprite is None:
            texture, source, alias = index.find_texture(aliases)
        else:
            texture = None
        if sprite is not None:
            rect = sprite.get("rect") or {}
            width = rect.get("width")
            height = rect.get("height")
            border = border_list(sprite.get("border"))
            record.update(
                kind="nine_slice" if entry.get("nine_slice") else ("icon" if usage == "icon" else "sprite"),
                status="mapped",
                manifest_name=sprite.get("name"),
                file=sprite.get("file"),
                source_asset=sprite.get("source_asset"),
                sheet=sprite.get("sheet"),
                rect=rect or None,
                natural_size_px=[
                    round(float(width), 2) if width is not None else None,
                    round(float(height), 2) if height is not None else None,
                ],
                pivot=[
                    round(float((sprite.get("pivot") or {}).get("x", 0.5)), 4),
                    round(float((sprite.get("pivot") or {}).get("y", 0.5)), 4),
                ],
                border=border,
                alias_used=alias,
                needs_prefab=False,
            )
            if entry.get("nine_slice"):
                if border is None:
                    inferred = INFERRED_BORDERS.get(usage, [6, 6, 6, 6])
                    record["border"] = inferred
                    record["border_inferred"] = True
                    record["needs_prefab"] = True
                else:
                    record["border_inferred"] = False
            key = record["file"] or ""
            shared.setdefault(key, []).append(logical)
        elif texture is not None:
            record.update(
                kind="nine_slice" if entry.get("nine_slice") else ("icon" if usage == "icon" else "sprite"),
                status="mapped",
                manifest_name=texture.get("name"),
                file=texture.get("file"),
                source_asset=texture.get("source_asset"),
                natural_size_px=[
                    texture.get("width"),
                    texture.get("height"),
                ],
                alias_used=alias,
                needs_prefab=False,
            )
            if entry.get("nine_slice"):
                record["border"] = INFERRED_BORDERS.get(usage, [6, 6, 6, 6])
                record["border_inferred"] = True
                record["needs_prefab"] = True
            shared.setdefault(record["file"] or "", []).append(logical)
        else:
            record["status"] = "unmatched"
            record["kind"] = "nine_slice" if entry.get("nine_slice") else ("icon" if usage == "icon" else "sprite")
            unmatched.append({"logical": logical, "kind": record["kind"], "aliases": aliases})
            if entry.get("nine_slice"):
                record["border"] = INFERRED_BORDERS.get(usage, [6, 6, 6, 6])
                record["border_inferred"] = True

        # Default draw size (inferred; final prefab rects need the runtime dump).
        if record["kind"] == "nine_slice":
            record["default_draw_size_px"] = None  # stretched by layout
        else:
            record["default_draw_size_px"] = list(INFERRED_DRAW_SIZE.get(usage, (32, 32)) or ())
        atlas[logical] = record

    # Shared aliases: many logical assets intentionally reuse one texture.
    for logical, record in atlas.items():
        key = record.get("file")
        if key:
            others = sorted(x for x in shared.get(key, []) if x != logical)
            record["shared_with"] = others

    # ---------------------------------------------------------------- fonts
    font_index = {norm(f.get("name", "")): f for f in (manifest.get("fonts") or [])}
    atlas_index = dict(index.tmp_atlases)
    roles: dict[str, dict] = {
        "body": font_role(
            {"_font_index": font_index, "_atlas_index": atlas_index},
            "", "body", needles=["Roboto-Regular"], atlas_needles=["Roboto-Regular SDF Atlas"],
        ),
        "bold": font_role(
            {"_font_index": font_index, "_atlas_index": atlas_index},
            "", "bold", needles=["Roboto-Bold"], atlas_needles=["Roboto-Bold SDF Atlas", "Roboto-Bold Atlas"],
        ),
        "mono_numeric": font_role(
            {"_font_index": font_index, "_atlas_index": atlas_index},
            "", "mono_numeric",
            needles=["Inconsolata-SemiBold", "PerfectDOSVGA437", "CONSOLA"],
            atlas_needles=["joystix monospace SDF Atlas"],
        ),
        "fallback": font_role(
            {"_font_index": font_index, "_atlas_index": atlas_index},
            "", "fallback", needles=["LiberationSans"], atlas_needles=["LiberationSans SDF Atlas"],
        ),
        "display": font_role(
            {"_font_index": font_index, "_atlas_index": atlas_index},
            "", "display", needles=[], atlas_needles=["Boston Black SDF Atlas"],
        ),
    }
    roles["mono_numeric"]["candidates"] = ["Inconsolata-SemiBold", "PerfectDOSVGA437", "CONSOLA", "joystix monospace"]
    fonts = {
        "roles": roles,
        "files": sorted(
            {
                (font.get("name"), font.get("file"))
                for font in (manifest.get("fonts") or [])
                if font.get("file")
            }
        ),
        "tmp_atlas_textures": sorted(
            {
                (tex.get("name"), tex.get("file"), tex.get("width"), tex.get("height"))
                for tex in atlas_index.values()
                if tex.get("file")
            }
        ),
        "substitution_rules": [
            "Prefer the TMP SDF atlas texture ('Roboto-Regular SDF Atlas', 'Roboto-Bold SDF Atlas') for text at the game's TMP look; render SDF glyph quads/distance-field text if the backend supports it.",
            "Fallback: load the extracted TTFs (Roboto-Regular.ttf / Roboto-Bold.ttf, LiberationSans.ttf for Latin fallback) and render at logical sizes scaled by the UIScaler factor min(h/1080, w/1920) * uiScale.",
            "Reference sizes at 1080p (inferred from TMP font assets/prefab conventions): body 16 px, heading/bold 20 px, small labels 14 px, monospace counters 16 px. Exact prefab sizes need the runtime prefab dump.",
            "Numerical/time/node-value displays may use the monospace family (Inconsolata-SemiBold / PerfectDOSVGA437 / CONSOLA); validate against screenshots before locking the role.",
            "If an atlas texture is unusable, fall back to the TTF with a slight letter-spacing/scale compensation and note the visual delta in the screenshot-diff report.",
        ],
    }

    # Attach resolved files to the font-role assets in the atlas.
    font_logical_roles = {
        "ui/font/tmp_regular": "body",
        "ui/font/tmp_bold": "bold",
        "ui/font/tmp_sdf_atlas": "body",
    }
    for logical, role in font_logical_roles.items():
        record = atlas.get(logical)
        if record is None:
            continue
        resolved = roles[role]
        record.update(
            kind="font",
            status="mapped",
            font_role=role,
            file=resolved.get("tmp_atlas") or resolved.get("ttf"),
            manifest_name=resolved.get("tmp_atlas_name") or resolved.get("ttf_name"),
            needs_prefab=True,
            natural_size_px=None,
            default_draw_size_px=None,
        )

    # ---------------------------------------------------------- brain icons
    def brain_table(prefix: str, expected_len: int) -> list[dict]:
        rows = {int(m.group(1)): entry for entry in entries if (m := re.match(rf"{re.escape(prefix)}/(\d+)$", entry["logical"]))}
        table = []
        for idx in range(expected_len):
            entry = rows.get(idx)
            if entry is None:
                table.append({"index": idx, "logical": f"{prefix}/{idx:02d}", "name": None, "file": None, "status": "missing_table_entry"})
                continue
            record = atlas[entry["logical"]]
            m = re.search(r"= ([A-Za-z0-9]+)", entry.get("notes", ""))
            table.append(
                {
                    "index": idx,
                    "logical": entry["logical"],
                    "name": m.group(1) if m else None,
                    "filename": record.get("manifest_name"),
                    "file": record.get("file"),
                    "status": record.get("status"),
                }
            )
        return table

    brain_icons = {
        "source": "Here/BrainIconHolder.cs: brainIcons[0..32]=inputs, brainIcons[33..47]=outputs, brainFunctionsIcons[0..12]=activation functions",
        "inputs": brain_table("ui/icon/brain_input", 33),
        "outputs": brain_table("ui/icon/brain_output", 15),
        "activations": brain_table("ui/icon/brain_function", 13),
    }

    # ------------------------------------------------------------ materials
    materials = {
        logical: record["material"]
        for logical, record in atlas.items()
        if record["kind"] == "material" and record.get("material")
    }
    available_ui_materials = {
        key: material_view(value) for key, value in (index.ui_materials or {}).items()
    }

    # ----------------------------------------------------------- procedural
    procedural = [
        {"logical": logical, "priority": record["priority"], "notes": record["notes"]}
        for logical, record in atlas.items()
        if record["kind"] == "procedural"
    ]

    # ---------------------------------------------------------------- counts
    by_kind: dict[str, int] = {}
    by_priority: dict[str, int] = {}
    by_status: dict[str, int] = {}
    for record in atlas.values():
        by_kind[record["kind"]] = by_kind.get(record["kind"], 0) + 1
        by_priority[record["priority"]] = by_priority.get(record["priority"], 0) + 1
        by_status[record["status"]] = by_status.get(record["status"], 0) + 1
    mapped = sum(1 for r in atlas.values() if r["status"] in {"mapped", "font_role", "runtime"})
    missing_required = sorted(
        logical
        for logical, record in atlas.items()
        if record["status"] == "unmatched" and record["priority"] in CORE_PRIORITIES
    )
    optional_missing = sorted(
        logical for logical, record in atlas.items() if record["status"] == "unmatched" and record["priority"] not in CORE_PRIORITIES
    )
    needs_prefab = sorted(logical for logical, record in atlas.items() if record.get("needs_prefab"))
    inferred_borders = sorted(
        logical for logical, record in atlas.items() if record.get("border_inferred")
    )

    # ------------------------------------------- implement-first checklist
    implement_first = {
        "phase_p0": [
            {
                "step": "Atlas loader + 9-slice renderer",
                "assets": [
                    "ui/panel/hud_time_frame", "ui/panel/hud_ecology_frame",
                    "ui/button/button", "ui/button/button_icon", "ui/button/close_x",
                    "ui/panel/tooltip_frame", "ui/panel/popup_frame",
                ],
                "notes": "Load sprites + border/pivot from ui_atlas.json; tint via default_tint. Shared chrome is intentional (many panels reuse UISprite/gray_panel/Long_UI).",
            },
            {
                "step": "Fonts (TMP SDF atlas + TTF fallback)",
                "assets": ["ui/font/tmp_regular", "ui/font/tmp_bold", "ui/font/tmp_sdf_atlas"],
                "notes": "Use fonts.roles: body/bold/mono_numeric; UIScaler factor min(h/1080,w/1920)*uiScale; reference sizes in fonts.substitution_rules.",
            },
            {
                "step": "HUD: time panel, playback icons, status bars",
                "assets": [
                    "ui/hud/time_panel_bg", "ui/hud/pause_icon", "ui/hud/play_icon",
                    "ui/hud/step_icon", "ui/hud/statusbar_frame",
                    "ui/hud/statusbar_health_fill", "ui/hud/statusbar_energy_fill",
                    "ui/hud/rightmost_tabs", "ui/hud/visibility_toggle_icons",
                ],
                "notes": "Status bar fills tint from health/energy ratio. Speed slider reuses slider widgets.",
            },
            {
                "step": "Core widgets: buttons, toggles, sliders, dropdowns, progress, inputs",
                "assets": [
                    "ui/button/button", "ui/button/button_small", "ui/toggle/toggle",
                    "ui/slider/slider_track", "ui/slider/slider_fill", "ui/slider/slider_knob",
                    "ui/dropdown/dropdown_bg", "ui/dropdown/dropdown_arrow", "ui/dropdown/dropdown_item",
                    "ui/progress/progress_bg", "ui/progress/progress_fill",
                    "ui/input/input_bg", "ui/input/input_caret", "ui/tabs/tabs",
                    "ui/scroll/scroll_track", "ui/scroll/scroll_handle", "ui/panel/separator",
                ],
                "notes": "Build as shared immediate-mode widgets; 9-slice borders from atlas (inferred ones flagged needs_prefab).",
            },
            {
                "step": "Selection/overlay basics + core icons",
                "assets": [
                    "ui/overlay/selection_rect", "ui/icon/checkmark", "ui/icon/arrow_up",
                    "ui/icon/arrow_down", "ui/icon/arrow_left", "ui/icon/arrow_right",
                    "ui/icon/warning", "ui/icon/error", "ui/icon/info", "ui/icon/star_locked",
                    "ui/icon/star_unlocked",
                ],
                "notes": "Selection rect is a tinted 1px border; highlight uses ui/material/selection_highlight_material.",
            },
            {
                "step": "Panels: bibite Stats + Genes (P0), tooltips",
                "assets": ["ui/panel/bibite_stats_frame", "ui/panel/bibite_genes_frame", "ui/panel/tooltip_frame"],
                "notes": "Use flat rows/labels composed from widgets; exact prefab rects still need the runtime dump (needs_prefab list).",
            },
        ],
        "phase_p0b": [
            {
                "step": "Biology panel materials + organ widgets",
                "assets": [
                    "ui/panel/bibite_biology_frame", "ui/material/stomach_acid",
                    "ui/material/internals_preview", "ui/panel/bibite_biology_header",
                ],
                "notes": "Organ schematic uses internals_preview/internalssections materials; stomach contents use procedural pellet sprites.",
            },
            {
                "step": "Brain simple panel + brain icon tables",
                "assets": ["ui/panel/bibite_brain_frame", "ui/material/node_activation", "ui/material/synapse_activation"],
                "notes": "Use brain_icons.inputs/outputs/activations index→filename tables exactly; node/synapse colouring uses materials.",
            },
            {
                "step": "Statistics panel (wedges/pies) + multi-selection + escape + tool panels",
                "assets": [
                    "ui/panel/stats_full_frame", "ui/chart/chart_ring_bg", "ui/chart/chart_ring_segment",
                    "ui/panel/selection_multi_frame", "ui/panel/escape_menu_frame",
                    "ui/panel/tool_panel_frame", "ui/material/color_spectrum", "ui/material/tolerance_band",
                    "ui/material/fov_cone",
                ],
                "notes": "Pie segments may be generated meshes (chart_ring_segment kind); colour-killer uses rainbowMaterial + tolerance shader.",
            },
            {
                "step": "Dialogs + notifications",
                "assets": ["ui/panel/popup_frame", "ui/panel/notification_frame", "ui/panel/tooltip_frame"],
                "notes": "Reuse shared chrome; blocking/pause-source stack is UI-state work, not asset work.",
            },
        ],
        "phase_p1": [
            {
                "step": "Graphs panel chrome + lineage connectors",
                "assets": ["ui/panel/graphs_frame", "ui/graph/graph_grid", "ui/graph/graph_axis", "ui/lineage/elbow_left", "ui/lineage/elbow_right", "ui/lineage/junction_left", "ui/lineage/junction_right"],
                "notes": "Graph series come from the viewer-side 60s sampler; line renders may use LineRenderer-like primitives instead of sprites.",
            },
            {
                "step": "Species tabs + ancestral lineage panels",
                "assets": ["ui/panel/species_info_frame", "ui/panel/lineage_frame", "ui/panel/lineage_header"],
                "notes": "Lineage graph materials: ui/material/lineage_graph.",
            },
            {
                "step": "FOV/ruler/pheromone overlays",
                "assets": ["ui/overlay/fov_target_indicator", "ui/overlay/ruler", "ui/material/fov_cone"],
                "notes": "FOV target indicator is runtime-procedural (bibite/pellet/egg sprites); FOV cone is a shader material.",
            },
        ],
    }

    manifest_stats = manifest.get("stats") or {}
    reference = manifest.get("reference") or {}
    return {
        "schema_version": 1,
        "generated_by": "tools/ui/build_ui_atlas.py",
        "sources": {
            "expectations": "tools/assets/ui_expectations.json",
            "manifest": "assets/game/manifest.json",
            "manifest_schema": manifest.get("schema"),
            "manifest_stats": {
                "ui_sprites": manifest_stats.get("ui_sprites"),
                "ui_textures": manifest_stats.get("ui_textures"),
                "fonts": manifest_stats.get("fonts"),
                "materials": manifest_stats.get("materials"),
            },
            "prefabs": (manifest.get("prefabs") or {}).get("file"),
            "reference_dir": reference.get("dir"),
        },
        "reference": {
            "dir": reference.get("dir"),
            "readme": reference.get("readme"),
            "frames": [
                {"file": frame.get("file"), "width": frame.get("width"), "height": frame.get("height")}
                for frame in (reference.get("frames") or [])
            ],
        },
        "naming": {
            "atlas_key": "logical id from tools/assets/ui_expectations.json",
            "file": "repo-relative path from assets/game/manifest.json (do not rewrite)",
            "kind": sorted(KINDS),
            "priority": "P0/P0b/P1 = M1 core UI; P2/P3 deferred",
        },
        "counts": {
            "entries": len(atlas),
            "by_kind": dict(sorted(by_kind.items())),
            "by_priority": dict(sorted(by_priority.items())),
            "by_status": dict(sorted(by_status.items())),
            "mapped": mapped,
            "m1_core_ui": sum(1 for r in atlas.values() if r["m1_core_ui"]),
            "needs_prefab": len(needs_prefab),
            "inferred_borders": len(inferred_borders),
            "missing_required": len(missing_required),
            "optional_missing": len(optional_missing),
        },
        "atlas": dict(sorted(atlas.items())),
        "missing_required": missing_required,
        "optional_missing": optional_missing,
        "needs_prefab": needs_prefab,
        "inferred_borders": inferred_borders,
        "fonts": fonts,
        "brain_icons": brain_icons,
        "materials": dict(sorted(materials.items())),
        "available_ui_materials": dict(sorted(available_ui_materials.items())),
        "procedural": sorted(procedural, key=lambda r: r["logical"]),
        "implement_first": implement_first,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--expectations", type=Path, default=EXPECTATIONS)
    parser.add_argument("--manifest", type=Path, default=MANIFEST)
    parser.add_argument("--out", type=Path, default=OUT)
    parser.add_argument("--check", action="store_true", help="print the summary without writing")
    args = parser.parse_args()

    if not args.expectations.exists():
        print(f"error: expectations file missing: {args.expectations}", file=sys.stderr)
        return 2
    if not args.manifest.exists():
        print(f"error: manifest missing: {args.manifest} (run parity-dev extraction first)", file=sys.stderr)
        return 2
    expectations = load_json(args.expectations)
    manifest = load_json(args.manifest)
    atlas = build(expectations, manifest)
    counts = atlas["counts"]
    print(f"atlas entries: {counts['entries']} (m1 core: {counts['m1_core_ui']})")
    print(f"kinds: {counts['by_kind']}")
    print(f"status: {counts['by_status']}")
    print(f"needs_prefab: {counts['needs_prefab']} | inferred borders: {counts['inferred_borders']}")
    print(f"missing required: {counts['missing_required']} | optional missing: {counts['optional_missing']}")
    if args.check:
        return 1 if counts["missing_required"] else 0
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(atlas, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {args.out.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

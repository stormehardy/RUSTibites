#!/usr/bin/env python3
"""contact_sheets.py - human-review contact sheets for assets/game (task-76).

Builds three PNG grids from assets/game/manifest.json:
  contact_sheets/inworld_sprites.png   every decoded procedural Sprite (parts + object sprites)
  contact_sheets/sheets.png            every Texture2D sheet (part sheets, object textures, UI atlases)
  contact_sheets/ui_sprites.png        every decoded UI Sprite (panels/buttons/icons/achievements)

Run after tools/assets/extract.py. Output under assets/game/contact_sheets/.
"""
from __future__ import annotations

import json
import math
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(r"Z:\RUSTibites")
GAME = ROOT / "assets" / "game"


def label(draw, xy, text, color=(255, 255, 255)):
    try:
        draw.text((xy[0] + 2, xy[1] + 1), text, fill=color)
    except Exception:
        pass


def grid(items, cell=96, cols=16, pad=3, title=""):
    entries = [(name, ROOT / rel) for name, rel in items if (ROOT / rel).exists()]
    if not entries:
        return None
    rows = math.ceil(len(entries) / cols)
    w = cols * (cell + pad) + pad
    h = rows * (cell + pad + 2) + pad
    sheet = Image.new("RGBA", (w, h), (24, 24, 30, 255))
    draw = ImageDraw.Draw(sheet)
    for i, (name, path) in enumerate(entries):
        r, c = divmod(i, cols)
        x = pad + c * (cell + pad)
        y = pad + r * (cell + pad + 2)
        try:
            img = Image.open(path).convert("RGBA")
            img.thumbnail((cell, cell))
            sheet.alpha_composite(img, (x + (cell - img.width) // 2, y + (cell - img.height) // 2))
        except Exception:
            pass
        label(draw, (x, y), name[:22], (180, 210, 255))
    return sheet


def main() -> int:
    manifest = json.loads((GAME / "manifest.json").read_text(encoding="utf-8"))
    out = GAME / "contact_sheets"
    out.mkdir(parents=True, exist_ok=True)
    written = []

    sprites = [(n, e["file"]) for n, e in manifest.get("sprites", {}).items() if e.get("file")]
    sheet = grid(sprites, cell=96, cols=20)
    if sheet:
        p = out / "inworld_sprites.png"
        sheet.save(p)
        written.append((p, len(sprites)))

    textures = [(n, e["file"]) for n, e in manifest.get("textures", {}).items() if e.get("file")]
    sheet = grid(textures, cell=96, cols=16)
    if sheet:
        p = out / "sheets.png"
        sheet.save(p)
        written.append((p, len(textures)))

    ui = []
    for fname, group in (manifest.get("ui") or {}).items():
        for n, e in (group.get("sprites") or {}).items():
            if e.get("file"):
                ui.append((n, e["file"]))
    sheet = grid(ui, cell=72, cols=24)
    if sheet:
        p = out / "ui_sprites.png"
        sheet.save(p)
        written.append((p, len(ui)))

    missing = [n for n, e in manifest.get("sprites", {}).items() if not e.get("file")]
    summary = {"sheets": [{"file": str(p.relative_to(ROOT)).replace("\\", "/"), "items": n} for p, n in written],
               "missing_sprite_pngs": len(missing), "missing_examples": missing[:10]}
    (out / "contact_sheets.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    for p, n in written:
        print(f"[contact] {p} ({n} items)")
    print(f"[contact] missing sprite pngs: {len(missing)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

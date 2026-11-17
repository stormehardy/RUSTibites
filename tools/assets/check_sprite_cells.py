#!/usr/bin/env python3
"""check_sprite_cells.py - audit per-cell sprite PNGs against their extracted sheets.

For every manifest sprite with a sheet+rect, compare the exported cell PNG against both the bottom-left
rect crop (top = sheet_h - y - h; the correct Unity convention) and the top-left crop. Reports fully
transparent exported cells and any cell whose sheet region is non-transparent in either orientation (a
likely extraction/mapping bug). Also prints the fully-transparent list and a match summary.

Usage: python tools/assets/check_sprite_cells.py [--manifest assets/game/manifest.json] [--out report.json]
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from PIL import Image

ROOT = Path(r"Z:\RUSTibites")


def alpha_nontransparent(img):
    return sum(1 for a in img.getchannel("A").getdata() if a)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--manifest", default=str(ROOT / "assets/game/manifest.json"))
    ap.add_argument("--out", default=str(ROOT / "assets/game/coverage_sprite_cells.json"))
    args = ap.parse_args()
    man = json.loads(Path(args.manifest).read_text(encoding="utf-8"))
    textures = man.get("textures") or {}
    stats = {"total": 0, "transparent": 0, "topflip_match": 0, "noflip_match": 0, "neither": 0,
             "transparent_mismatch": 0}
    transparent = []
    mismatches = []
    seen = set()
    for name, e in (man.get("sprites") or {}).items():
        rect = e.get("rect") or {}
        f = e.get("file")
        sheet = (textures.get(e.get("sheet")) or {}).get("file")
        if not f or not rect or not sheet or f in seen:
            continue
        seen.add(f)
        try:
            cell = Image.open(ROOT / f).convert("RGBA")
            sheet_img = Image.open(ROOT / sheet).convert("RGBA")
        except Exception:
            continue
        stats["total"] += 1
        nt = alpha_nontransparent(cell)
        x = int(round(rect.get("x", 0))); y = int(round(rect.get("y", 0)))
        w = int(round(rect.get("width", 0))); h = int(round(rect.get("height", 0)))
        top = sheet_img.height - y - h
        topcrop = sheet_img.crop((x, top, x + w, top + h))
        nocrop = sheet_img.crop((x, y, x + w, y + h))
        top_nt = alpha_nontransparent(topcrop)
        no_nt = alpha_nontransparent(nocrop)
        topmatch = list(topcrop.getdata()) == list(cell.getdata())
        nomatch = list(nocrop.getdata()) == list(cell.getdata())
        if topmatch: stats["topflip_match"] += 1
        if nomatch: stats["noflip_match"] += 1
        if not topmatch and not nomatch: stats["neither"] += 1
        if nt == 0:
            stats["transparent"] += 1
            transparent.append({"name": name, "file": f, "rect": rect,
                                "sheet_topflip_nontransparent": top_nt,
                                "sheet_noflip_nontransparent": no_nt})
            if top_nt or no_nt:
                stats["transparent_mismatch"] += 1
                mismatches.append(name)
        elif not topmatch and not nomatch:
            mismatches.append(name)
    out = {"schema": "rustibites.assets.sprite_cells.v1", "stats": stats,
           "transparent_cells": transparent, "mismatches": mismatches[:100]}
    Path(args.out).write_text(json.dumps(out, indent=2), encoding="utf-8")
    print(json.dumps(out["stats"], indent=1))
    print("fully transparent:", [f"{t['name']} ({t['sheet_noflip_nontransparent']}/{t['sheet_topflip_nontransparent']})"
                                 for t in transparent])
    print("wrote", args.out)


if __name__ == "__main__":
    main()

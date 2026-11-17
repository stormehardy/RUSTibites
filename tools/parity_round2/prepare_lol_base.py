#!/usr/bin/env python3
"""prepare_lol_base.py - build the Round-2 base fixture from testdata/real/saves/lol.zip.

The real save is read-only. Round 2 needs a copy that satisfies the deterministic capture protocol
(docs/PARITY.md 10.1) without touching the source:

1. `scene.bb8scene.simulatedTime` is normalised to its **f32 representation**. The game keeps the
   loaded clock as f32 (TimeKeeper.simulatedTime is double but the save/load path quantises to f32), so
   a save whose stored time is not exactly f32-representable always differs from the post-load clock by
   up to half an f32 ULP (0.0156 s at t~470000). tools/parity/capture.py's clean-load check requires
   |status - base_time| <= 1e-5 s, which is a *small-t* assumption; normalising the fixture makes the
   load clock exact without modifying the harness (a general capture.py fix is reported to Lead).
2. `settings.bb8settings.independents.virginSpawnRate` is set to 0. The save has 1.0, which makes the
   load-time spawner create extra bibites from unseeded RNG before any pause/seed can land - not
   deterministic. Round 2 freezes it to 0 (like the Ring base); the world's own population/eggs are all
   kept. Rust runs the same modified copy, so both sides use identical settings.

Usage:
    python tools/parity_round2/prepare_lol_base.py [--src ...] [--out testdata/parity/bases/lol_round2.zip]
Prints a JSON summary (source/copy hashes, old/new simulatedTime, changed settings).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import struct
import zipfile
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
DEFAULT_SRC = ROOT / "testdata" / "real" / "saves" / "lol.zip"
DEFAULT_OUT = ROOT / "testdata" / "parity" / "bases" / "lol_round2.zip"


def f32(x: float) -> float:
    return struct.unpack("<f", struct.pack("<f", float(x)))[0]


def sha256(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--src", default=str(DEFAULT_SRC))
    ap.add_argument("--out", default=str(DEFAULT_OUT))
    ap.add_argument("--keep-spawners", action="store_true", help="do not zero virginSpawnRate (not deterministic)")
    args = ap.parse_args()
    src = Path(args.src)
    out = Path(args.out)
    if not src.exists():
        raise SystemExit(f"source save not found: {src}")
    out.parent.mkdir(parents=True, exist_ok=True)

    with zipfile.ZipFile(src) as zin:
        entries = {name: zin.read(name) for name in zin.namelist()}

    scene = json.loads(entries["scene.bb8scene"].decode("utf-8-sig"))
    old_t = float(scene["simulatedTime"])
    new_t = f32(old_t)
    scene["simulatedTime"] = new_t
    entries["scene.bb8scene"] = json.dumps(scene, separators=(",", ":")).encode("utf-8")

    settings_raw = entries["settings.bb8settings"].decode("utf-8-sig")
    settings = json.loads(settings_raw)
    ind = settings.get("independents") or {}
    old_virgin = (ind.get("virginSpawnRate") or {}).get("Value") if isinstance(ind.get("virginSpawnRate"), dict) else None
    new_virgin = old_virgin
    if not args.keep_spawners and isinstance(ind.get("virginSpawnRate"), dict):
        ind["virginSpawnRate"]["Value"] = 0.0
        new_virgin = 0.0
    entries["settings.bb8settings"] = json.dumps(settings, separators=(",", ":")).encode("utf-8")

    tmp = out.with_suffix(".zip.tmp")
    with zipfile.ZipFile(tmp, "w", compression=zipfile.ZIP_DEFLATED) as zout:
        for name, data in entries.items():
            zout.writestr(name, data)
    tmp.replace(out)

    summary = {
        "schema": "rustibites.parity.round2_base.v1",
        "source": {"path": str(src), "sha256": sha256(src), "size": src.stat().st_size},
        "base": {"path": str(out), "sha256": sha256(out), "size": out.stat().st_size},
        "scene_version": scene.get("version"),
        "simulated_time": {"old": old_t, "new_f32": new_t, "delta": new_t - old_t},
        "virgin_spawn_rate": {"old": old_virgin, "new": new_virgin},
        "entities": {
            "bibite_files": len([n for n in entries if n.startswith("bibites/")]),
            "egg_files": len([n for n in entries if n.startswith("eggs/")]),
        },
        "note": "read-only copy of the real save with scene.simulatedTime f32-normalised and "
                "virginSpawnRate frozen to 0 (deterministic capture protocol); all other bytes preserved "
                "(unknown zip entries copied verbatim).",
    }
    (out.with_suffix(".base.json")).write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

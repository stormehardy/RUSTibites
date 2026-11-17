#!/usr/bin/env python3
"""capture_ui_runtime.py - hidden-window runtime UI field dump (task-102).

Starts the game through the BibitesAPI lab harness with window="hidden", asks the transient ParityDump
plugin for `GET /d/ui` twice (main menu, then after a fresh Default.zip world), writes
`assets/game/ui_runtime.json`, and repeats the whole capture to verify determinism after stripping
volatile values (createdAtUtc, dynamic HUD text). Runtime MonoBehaviour fields (Image sprite/color/type/
border/fill, TMP/Text font/size/alignment/color, CanvasScaler reference resolution/match, LayoutGroup
paddings) are exactly what the asset-file dump could not read; this fills that gap without a visible
window. The game is stopped in `finally`; deploy/remove the plugin around this script externally.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
TOOLS_PARITY = ROOT / "tools" / "parity"
OUT = ROOT / "assets" / "game" / "ui_runtime.json.gz"
DEFAULT_SCENARIO = ROOT / "testdata" / "real" / "Default.zip"
DEFAULT_API_HOME = Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")

sys.path.insert(0, str(TOOLS_PARITY))
import capture as cap  # noqa: E402


def log(msg):
    print(f"[ui-runtime] {msg}", flush=True)


def build_args(ns):
    ns.work_dir = str(ROOT / "testdata" / "parity" / "_work")
    ns.port = 17800
    ns.plugin_port = 17801
    ns.no_plugin = False
    ns.attach = False
    ns.start_timeout = 300.0
    ns.keep_game = False
    return ns


VOLATILE_KEYS = {"createdAtUtc", "text", "fill_amount", "value", "alpha", "is_on", "scale_factor",
                 "anchored_position", "local_position", "size_delta", "local_rotation"}


def normalise(obj):
    """Structural normalisation for the two-run determinism check.

    Runtime UI is structurally deterministic, but dynamic values differ by design: HUD clock/FPS/
    population text, fill amounts, animated menu rotations, and layout positions/sizes computed from
    those dynamic texts. Strip those volatile fields (listed in VOLATILE_KEYS); everything structural
    (node paths/hierarchy, component types, sprite/font names, colours, anchors/pivot, layout paddings,
    canvas scaler reference/match) must be identical. The report states which fields were ignored.
    """
    if isinstance(obj, dict):
        return {k: normalise(v) for k, v in sorted(obj.items()) if k not in VOLATILE_KEYS}
    if isinstance(obj, list):
        return [normalise(v) for v in obj]
    return obj


def dump_once(session, scenario: Path) -> dict:
    client = session.client
    plugin = session.plugin
    main_menu = plugin.ui_runtime()
    log(f"main menu dump: canvases={len(main_menu.get('canvases') or [])} nodes={main_menu.get('node_count')}")
    client.new_game(scenario, name="ui runtime Default")
    client.set_time(paused=True)
    default_world = plugin.ui_runtime()
    log(f"Default world dump: canvases={len(default_world.get('canvases') or [])} "
        f"nodes={default_world.get('node_count')}")
    return {"main_menu": main_menu, "default_world": default_world}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--scenario", default=str(DEFAULT_SCENARIO))
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--api-home", default=str(DEFAULT_API_HOME))
    ap.add_argument("--repeat", type=int, default=2, help="hidden sessions to capture for determinism")
    ap.add_argument("--save-runs", default=None,
                    help="optional directory: write each raw session dump as run_<i>.json for determinism diffing")
    args = ap.parse_args()
    scenario = Path(args.scenario).absolute()
    BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(args.api_home))
    runs = []
    for i in range(max(1, args.repeat)):
        log(f"hidden session {i + 1}/{args.repeat} (no visible window)")
        session = cap.WorldSession(build_args(argparse.Namespace()), BridgeClient, LiveRuntime, Lab)
        try:
            session.open()
            if not session.plugin_ok:
                raise SystemExit("parity plugin not available on 17801 (deploy ParityDump first)")
            runs.append(dump_once(session, scenario))
        finally:
            session.close()
    if args.save_runs:
        run_dir = Path(args.save_runs)
        run_dir.mkdir(parents=True, exist_ok=True)
        for i, run in enumerate(runs):
            (run_dir / f"run_{i}.json").write_text(json.dumps(run, indent=1), encoding="utf-8")
    first = runs[0]
    det = {"repeats": len(runs), "normalised_equal": None, "hashes": []}
    if len(runs) > 1:
        hashes = [hashlib.sha256(json.dumps(normalise(r), sort_keys=True).encode()).hexdigest() for r in runs]
        det["hashes"] = hashes
        det["structural_equal"] = len(set(hashes)) == 1
        log(f"determinism: structural_equal={det['structural_equal']} hashes={hashes}")
    out = {
        "schema": "rustibites.assets.ui_runtime.v1",
        "generatedBy": "tools/assets/capture_ui_runtime.py",
        "capture": {
            "method": "hidden BibitesAPI lab window; ParityDump GET /d/ui (reflection on live components)",
            "game": {"version": "0.6.3.1", "unity": "6000.0.44f1"},
            "scenario": str(scenario),
            "screen": (first.get("default_world") or {}).get("screen"),
            "captured_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "plugin": "ParityDump (transient; removed after capture)",
        },
        "determinism": det,
        "sessions": first,
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
        "schema": "rustibites.assets.ui_runtime.index.v1",
        "files": {"runtime": str(out_path.relative_to(ROOT)) if out_path.is_relative_to(ROOT) else str(out_path),
                  "runtime_format": "gzipped JSON (decompress with gzip to read the full dump)"},
        "capture": out["capture"], "determinism": out["determinism"],
        "sessions": {k: {"node_count": v.get("node_count"), "canvases": len(v.get("canvases") or [])}
                     for k, v in first.items()},
        "consumer": "read tools/ui/ui_runtime_atlas.json.gz (joined, exact panels) for rendering; this "
                    "file is the full runtime dump with volatile runtime values.",
    }
    idx_path = out_path.with_name(out_path.name.replace(".json.gz", ".index.json"))
    idx_path.write_text(json.dumps(index, indent=1), encoding="utf-8")
    total_nodes = sum((s or {}).get("node_count") or 0 for s in first.values())
    log(f"wrote {out_path} ({out_path.stat().st_size} bytes, {total_nodes} nodes total) + {idx_path.name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

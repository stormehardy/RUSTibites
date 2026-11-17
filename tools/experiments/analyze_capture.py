#!/usr/bin/env python3
"""analyze_capture.py - analyse an existing Rust parity capture directory as an ecology run.

Reuses tools/experiments/run_ecology.py's conversion/analysis/plotting helpers so artifacts produced
by other runs (e.g. the evolution-bench captures under testdata/evolve) get the same series.csv +
summary.json + plots treatment without re-simulating.

Usage:
    python tools/experiments/analyze_capture.py --capture testdata/evolve/lol_50k \
        --name lol_50k_artifact [--out testdata/experiments/lol_50k_artifact]
"""
from __future__ import annotations

import argparse
import json
import shutil
import sys
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "experiments"))
import run_ecology as eco  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--capture", required=True, help="capture directory with ticks.jsonl[.gz]")
    ap.add_argument("--name", required=True)
    ap.add_argument("--out-root", default=str(ROOT / "testdata" / "experiments"))
    ap.add_argument("--out")
    ap.add_argument("--copy-raw", action="store_true")
    args = ap.parse_args()
    src = Path(args.capture).absolute()
    if not src.exists():
        raise SystemExit(f"capture not found: {src}")
    out = Path(args.out).absolute() if args.out else Path(args.out_root).absolute() / args.name
    out.mkdir(parents=True, exist_ok=True)
    rows = eco.capture_to_series(src, out / "series.csv")
    summary = eco.analyse(rows)
    manifest = {}
    mp = src / "manifest.json"
    if mp.exists():
        manifest = json.loads(mp.read_text(encoding="utf-8"))
    summary.update({"name": args.name, "source_capture": str(src),
                    "sim": manifest.get("sim"), "engine": manifest.get("engine"),
                    "inputs": manifest.get("inputs"), "counts_start": manifest.get("counts_start"),
                    "counts_end": manifest.get("counts_end")})
    summary["plots"] = eco.make_plots(rows, out / "plots", args.name)
    (out / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(f"[analyze] {args.name}: records={summary['records']} flags={summary['flags']}")
    print(f"[analyze] population {summary['population']}")
    print(f"[analyze] births={summary['births']} deaths={summary['deaths']} species={summary['species']}")
    print(f"[analyze] plant_n={summary['plant_n']} energy={summary['energy_mean']}")
    print(f"[analyze] artifacts: {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

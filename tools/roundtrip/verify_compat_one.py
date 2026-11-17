#!/usr/bin/env python3
"""verify_compat_one.py - one compat fixture in a brand-new hidden lab, pure-JSON report.

Handles save (load_world + optional re-save), scenario (new_game) and template (upgrade_bibite_file), records
status/counts/bridge/log tail and expected-vs-observed fields. Intended to be invoked once per fixture by a
PowerShell loop (fresh Python process + fresh Lab each time).
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
from pathlib import Path

DEFAULT_API_HOME = Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
LOG = Path(os.environ.get("USERPROFILE", "")) / "AppData/LocalLow/The Bibites/The Bibites/Player.log"


def tail(lines=200):
    try:
        return (LOG.read_text(encoding="utf-8", errors="replace").splitlines())[-lines:]
    except OSError:
        return []


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--fixture", required=True)
    ap.add_argument("--kind", required=True, choices=["save", "scenario", "template"])
    ap.add_argument("--expected", default="")
    ap.add_argument("--out", required=True)
    ap.add_argument("--resave-dir", default="testdata/parity/compat/_resaved_one")
    ap.add_argument("--api-home", default=os.environ.get("BIBITES_API_HOME", str(DEFAULT_API_HOME)))
    ap.add_argument("--timeout", type=float, default=90.0)
    args = ap.parse_args()
    fixture = Path(args.fixture).resolve()
    expected = json.loads(args.expected) if args.expected else {}
    resave_dir = Path(args.resave_dir)
    resave_dir.mkdir(parents=True, exist_ok=True)
    sys.path.insert(0, args.api_home)
    from bibites_api.live.runtime import LiveRuntime
    from bibites_api.lab import Lab

    report = {"fixture": str(fixture), "kind": args.kind, "bytes": fixture.stat().st_size if fixture.is_file() else 0,
              "expected": expected, "player_log": str(LOG)}
    runtime = LiveRuntime(work_dir=Path("target/compat_one") / fixture.stem)
    runtime.work.mkdir(parents=True, exist_ok=True)
    before = tail()
    client = None
    try:
        client = Lab(runtime, work_dir=runtime.work, window="hidden").ensure_game(timeout=args.timeout)
        report["ping"] = client.ping()
        if args.kind == "template":
            reply = client.upgrade_bibite_file(fixture)
            report.update(accepted=True, template=reply)
        elif args.kind == "scenario":
            client.new_game(fixture, timeout=args.timeout)
            status = client.status()
            report.update(accepted=bool(status.get("inSimulation")), in_simulation=status.get("inSimulation"),
                          counts=status.get("counts"), simulated_time=status.get("simulatedTime"))
        else:
            client.load_world(fixture, timeout=args.timeout)
            status = client.status()
            report.update(accepted=True, in_simulation=status.get("inSimulation"), counts=status.get("counts"),
                          simulated_time=status.get("simulatedTime"))
            resaved = resave_dir / fixture.name
            client.save_world(resaved)
            report.update(resaved=str(resaved), resaved_bytes=resaved.stat().st_size,
                          resave_ok=resaved.is_file() and resaved.stat().st_size > 0)
        report["status_after"] = client.status()
    except Exception as exc:  # noqa: BLE001
        report.update(accepted=False, error=repr(exc))
    finally:
        after = tail()
        report["player_log_new_errors"] = [l for l in after if l not in before and ("Exception" in l or "Error" in l)][-20:]
        if client is not None:
            try:
                runtime.close()
            except Exception:  # noqa: BLE001
                pass
        # observed-vs-expected field diff (top-level expected keys, common counts)
        dropped = {}
        for k, v in expected.items():
            if k in ("bibite_entries", "egg_entries", "plant_pellets", "pheromones", "species_records", "zones",
                     "rad_towers", "phero_towers", "color_killers", "data_bin_bytes", "templates", "genes", "nodes",
                     "spawn_rows", "bundled_templates", "zone_groups", "changers"):
                continue
            if k not in report:
                dropped[k] = {"expected": v, "observed": None}
        report["dropped_fields"] = dropped
        report["closed_at"] = time.time()
        Path(args.out).write_text(json.dumps(report, indent=2), encoding="utf-8")
        print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

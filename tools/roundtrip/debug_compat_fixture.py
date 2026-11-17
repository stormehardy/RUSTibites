#!/usr/bin/env python3
"""debug_compat_fixture.py - fresh hidden-lab load of one compat fixture (fmt compat triage).

Starts a brand-new hidden BibitesAPI lab per invocation, pings it, loads the requested fixture with a
bounded timeout, records status/availability/bridge state and the game log tail, then closes the lab. This
distinguishes a fixture problem from a lab-state problem left by the previous fixture in the pack.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
from pathlib import Path

DEFAULT_API_HOME = Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
LOG_CANDIDATES = [
    Path(r"C:\Users\Storme\AppData\LocalLow\The Bibites\The Bibites\Player.log"),
    Path(os.environ.get("LOCALAPPDATA", "")) / "Low" / "The Bibites" / "The Bibites" / "Player.log",
]


def tail_log(paths, lines=250):
    for p in paths:
        try:
            return {"path": str(p), "tail": "\n".join(p.read_text(encoding="utf-8", errors="replace").splitlines()[-lines:])}
        except OSError:
            continue
    return {"path": None, "tail": ""}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--fixture", required=True)
    ap.add_argument("--timeout", type=float, default=90.0)
    ap.add_argument("--api-home", default=os.environ.get("BIBITES_API_HOME", str(DEFAULT_API_HOME)))
    ap.add_argument("--out", default="target/compat_debug")
    args = ap.parse_args()
    fixture = Path(args.fixture).resolve()
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    sys.path.insert(0, args.api_home)
    from bibites_api.live.runtime import LiveRuntime
    from bibites_api.lab import Lab

    runtime = LiveRuntime(work_dir=out / "lab")
    runtime.work.mkdir(parents=True, exist_ok=True)
    report = {"fixture": str(fixture), "bytes": fixture.stat().st_size if fixture.is_file() else None,
              "log_before": tail_log(LOG_CANDIDATES), "started_at": time.time()}
    client = None
    try:
        client = Lab(runtime, work_dir=runtime.work, window="hidden").ensure_game(timeout=args.timeout)
        report["ping"] = client.ping()
        report["status_before"] = client.status()
        t0 = time.time()
        try:
            client.load_world(fixture, timeout=args.timeout)
            report["load"] = "ok"
        except Exception as exc:  # noqa: BLE001
            report["load"] = "error"
            report["error"] = repr(exc)
            try:
                report["status_after_error"] = client.status()
            except Exception as exc2:  # noqa: BLE001
                report["status_after_error_error"] = repr(exc2)
            try:
                report["ping_after_error"] = client.ping()
            except Exception as exc2:  # noqa: BLE001
                report["ping_after_error"] = repr(exc2)
            try:
                report["available_after_error"] = client.available()
            except Exception as exc2:  # noqa: BLE001
                report["available_after_error"] = repr(exc2)
        report["load_wall_s"] = time.time() - t0
        try:
            report["status_after"] = client.status()
        except Exception as exc:  # noqa: BLE001
            report["status_after"] = repr(exc)
    finally:
        report["log_after"] = tail_log(LOG_CANDIDATES)
        if client is not None:
            try:
                runtime.close()
            except Exception:  # noqa: BLE001
                pass
        report["closed_at"] = time.time()
        name = fixture.stem.replace(" ", "_")
        (out / f"{name}.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
        print(json.dumps(report, indent=2)[:6000])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""load_compat_pack_reset.py - compat pack game-load with a fresh hidden lab per fixture.

Workaround/verification for the fmt compat pack: each manifest fixture gets its own BibitesAPI hidden lab
(so a rejection cannot kill the bridge for the rest), with the correct LocalLow Player.log tail and
accepted/resaved/status fields. Reads the same manifest and writes game_report_reset.json.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

DEFAULT_API_HOME = Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
LOG = Path(os.environ.get("USERPROFILE", "")) / "AppData/LocalLow/The Bibites/The Bibites/Player.log"


def tail(lines=200):
    try:
        return "\n".join(LOG.read_text(encoding="utf-8", errors="replace").splitlines()[-lines:])
    except OSError:
        return ""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", default="testdata/parity/compat")
    ap.add_argument("--out", default="testdata/parity/compat/game_report_reset.json")
    ap.add_argument("--resave-dir", default="testdata/parity/compat/_resaved_reset")
    ap.add_argument("--api-home", default=os.environ.get("BIBITES_API_HOME", str(DEFAULT_API_HOME)))
    ap.add_argument("--timeout", type=float, default=120.0)
    args = ap.parse_args()
    folder = Path(args.dir).resolve()
    manifest = json.loads((folder / "manifest.json").read_text(encoding="utf-8"))
    resave_dir = Path(args.resave_dir)
    resave_dir.mkdir(parents=True, exist_ok=True)
    sys.path.insert(0, args.api_home)
    from bibites_api.live.runtime import LiveRuntime
    from bibites_api.lab import Lab

    report = {"schema": "rustibites.compat.game_report_reset.v1", "mode": "fresh lab per fixture",
              "player_log": str(LOG), "fixtures": []}
    for i, entry in enumerate(manifest["fixtures"]):
        if entry["kind"] == "checkpoint-sidecar":
            continue
        path = folder / entry["name"]
        item = {"name": entry["name"], "kind": entry["kind"], "expected": entry.get("expected")}
        runtime = LiveRuntime(work_dir=Path("target/compat_reset") / f"lab_{i}")
        runtime.work.mkdir(parents=True, exist_ok=True)
        client = None
        before = tail()
        try:
            client = Lab(runtime, work_dir=runtime.work, window="hidden").ensure_game(timeout=args.timeout)
            item["ping"] = client.ping()
            if entry["kind"] == "template":
                reply = client.upgrade_bibite_file(path)
                item.update(accepted=True, kind=reply.get("kind"), genes=reply.get("genes"), nodes=reply.get("nodes"))
            elif entry["kind"] == "scenario":
                client.new_game(path, timeout=args.timeout)
                status = client.status()
                item.update(accepted=True, in_simulation=status.get("inSimulation"), counts=status.get("counts"),
                            simulated_time=status.get("simulatedTime"))
            else:
                client.load_world(path, timeout=args.timeout)
                status = client.status()
                item.update(accepted=True, in_simulation=status.get("inSimulation"), counts=status.get("counts"),
                            simulated_time=status.get("simulatedTime"))
                resaved = resave_dir / entry["name"]
                client.save_world(resaved)
                item.update(resaved=str(resaved), resaved_bytes=resaved.stat().st_size if resaved.is_file() else 0,
                            resave_ok=resaved.is_file() and resaved.stat().st_size > 0)
        except Exception as exc:  # noqa: BLE001
            item.update(accepted=False, error=repr(exc))
            try:
                item["status_after"] = client.status() if client is not None else None
            except Exception as exc2:  # noqa: BLE001
                item["status_after_error"] = repr(exc2)
        finally:
            after = tail()
            item["player_log_new_errors"] = [line for line in after.splitlines()
                                             if line not in before.splitlines() and ("Exception" in line or "Error" in line)][-20:]
            if client is not None:
                try:
                    runtime.close()
                except Exception:  # noqa: BLE001
                    pass
        report["fixtures"].append(item)
        print(json.dumps({k: item.get(k) for k in ("name", "kind", "accepted", "counts", "resave_ok", "error")}, default=str))
    (Path(args.out)).write_text(json.dumps(report, indent=2), encoding="utf-8")
    accepted = sum(1 for f in report["fixtures"] if f.get("accepted"))
    print(f"accepted {accepted}/{len(report['fixtures'])} -> {args.out}")
    return 0 if accepted == len(report["fixtures"]) else 1


if __name__ == "__main__":
    raise SystemExit(main())

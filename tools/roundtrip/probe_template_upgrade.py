"""Asks the real game's own converter to parse the templates our strict loader
rejects (task-44 game-tolerance evidence). Starts a hidden BibitesAPI lab game,
calls POST /bibite/upgrade for each file, prints the result, stops the game.

Usage: python tools/roundtrip/probe_template_upgrade.py <file> [<file> ...]
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

DEFAULT_API_HOME = Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("files", nargs="+")
    ap.add_argument("--api-home", default=os.environ.get("BIBITES_API_HOME", str(DEFAULT_API_HOME)))
    ap.add_argument("--out", default="")
    args = ap.parse_args()

    sys.path.insert(0, args.api_home)
    from bibites_api.live.client import BridgeError  # noqa: F401
    from bibites_api.live.runtime import LiveRuntime
    from bibites_api.lab import Lab

    runtime = LiveRuntime(work_dir=Path("testdata/roundtrip/_lab_upgrade"))
    runtime.work.mkdir(parents=True, exist_ok=True)
    client = None
    results = []
    try:
        client = Lab(runtime, work_dir=runtime.work, window="hidden").ensure_game(timeout=300)
        for path in args.files:
            entry = {"path": path}
            try:
                reply = client.upgrade_bibite_file(path)
                entry.update(
                    {
                        "accepted": True,
                        "kind": reply.get("kind"),
                        "fromVersion": reply.get("fromVersion"),
                        "updated": reply.get("updated"),
                        "template": bool(reply.get("template")),
                    }
                )
            except Exception as error:  # noqa: BLE001
                entry.update({"accepted": False, "error": repr(error)})
            results.append(entry)
        return 0
    finally:
        if client is not None:
            try:
                runtime.close()
            except Exception:  # noqa: BLE001
                pass
        payload = {"game_version": "0.6.3.1", "results": results}
        if args.out:
            Path(args.out).write_text(json.dumps(payload, indent=2), encoding="utf-8")
        print(json.dumps(payload, indent=2))


if __name__ == "__main__":
    sys.exit(main())

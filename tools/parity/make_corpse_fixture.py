#!/usr/bin/env python3
"""make_corpse_fixture.py - hidden Default scenario with all starting bibites killed (corpses -> meat
decay/recycle fixture). Saves game-written world for matching Rust capture. Hidden-only."""
from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools/parity"))
import capture as cap  # noqa: E402


def build_args(ns):
    ns.work_dir = str(ROOT / "testdata/parity/_work")
    ns.port = 17800
    ns.plugin_port = 17801
    ns.no_plugin = True
    ns.attach = False
    ns.start_timeout = 300.0
    ns.keep_game = False
    return ns


def main() -> int:
    BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
    session = cap.WorldSession(build_args(__import__("argparse").Namespace()), BridgeClient, LiveRuntime, Lab)
    try:
        client = session.open()
        client.new_game(ROOT / "testdata/real/Default.zip", name="corpse fixture")
        client.set_time(paused=True)
        ents = client.entities(kind="bibite")
        handles = [e.get("h") if e.get("h") is not None else e.get("handle") for e in ents if (e.get("h") is not None or e.get("handle") is not None)]
        print("bibite entries", len(ents), "sample", ents[:1], "handles", handles[:5])
        if handles:
            print("kill:", client.kill(handles))
        out = ROOT / "testdata/parity/default/corpse_world_600.zip"
        client.save_world(out)
        print("saved", out, out.stat().st_size, "counts", client.status().get("counts"))
    finally:
        session.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

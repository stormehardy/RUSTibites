#!/usr/bin/env python3
"""make_decay_fixture.py - hidden Default scenario + a field of meat/plant pellets for the optional
decay/recycle conservation fixture (task-125 follow-up).

Loads Default.zip in the hidden lab, pauses, injects a deterministic grid of meat + plant pellets via the
bridge, saves the game-written world to `testdata/parity/default/decay_world_600.zip` so the same fixture
can be loaded by Rust. No plugin needed; the lab is stopped in `finally`. Run:
  python tools/parity/make_decay_fixture.py
"""
from __future__ import annotations

import argparse
import sys
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
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
    ap = argparse.ArgumentParser()
    ap.add_argument("--scenario", default=str(ROOT / "testdata/real/Default.zip"))
    ap.add_argument("--out", default=str(ROOT / "testdata/parity/default/decay_world_600.zip"))
    ap.add_argument("--meat", type=int, default=250)
    ap.add_argument("--plant", type=int, default=60)
    ap.add_argument("--amount", type=float, default=20.0)
    ap.add_argument("--api-home", default=r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
    args = ap.parse_args()
    BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(args.api_home))
    session = cap.WorldSession(build_args(argparse.Namespace()), BridgeClient, LiveRuntime, Lab)
    try:
        client = session.open()
        client.new_game(Path(args.scenario).absolute(), name="decay fixture")
        client.set_time(paused=True)
        rows = []
        n = max(args.meat, args.plant)
        for i in range(n):
            # deterministic sunflower-ish spread inside the Default world
            import math
            a = i * 2.399963229728653
            r = 900.0 * math.sqrt((i + 0.5) / n)
            x, y = r * math.cos(a), r * math.sin(a)
            if i < args.meat:
                rows.append((x, y, args.amount, 1))
            if i < args.plant:
                rows.append((x + 6.0, y + 6.0, args.amount, 0))
        resp = client.spawn_pellets(rows)
        print("[decay-fixture] spawn_pellets:", resp)
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        client.save_world(out)
        print(f"[decay-fixture] saved {out} ({out.stat().st_size} B) rows={len(rows)} counts={client.status().get('counts')}")
    finally:
        session.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

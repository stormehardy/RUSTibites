#!/usr/bin/env python3
"""probe_rotate.py - sign convention of the brain's Rotate output (body-dev BQ-4 / world Q1).

Takes a living bibite, replaces its brain with a bias-only brain whose Rotate output (node 34) is a
constant, runs a few ticks and reports:

  * the sign of the resulting angularVelocity and the change in rotation (Unity degrees: positive =
    counter-clockwise when the bibite faces +x),
  * whether +1 rotates toward the world +y axis (left) or -y (right) from the bibite's heading.

We run both signs (+1 and -1) so the mapping output->angularVelocity is unambiguous, and repeat with
the output left at 0 as a control. Results are written to testdata/parity/rotation_sign.json.

Usage:
    python tools/parity/probe_rotate.py [--base testdata/parity/bases/ring_parity.zip   # historical Ring base (Default is canonical)]
Exit code 0 when both signs produce opposite, consistent angular velocities.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
PROJECT = HERE.parent.parent
DEFAULT_API_HOME = Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
DEFAULT_BASE = PROJECT / "testdata" / "parity" / "bases" / "ring_parity.zip"
OUT = PROJECT / "testdata" / "parity" / "rotation_sign.json"
ROTATE_NODE = 34          # NEATBrain.Outputs.Rotate (inputs 0-32, outputs 33-47)
ACCELERATE_NODE = 33


def log(msg: str) -> None:
    print(f"[rotate] {msg}", flush=True)


def find_bibite(client):
    ents = client.entities("bibite")
    if not ents:
        raise SystemExit("no living bibite in the world")
    # prefer a non-corpse, living bibite with a brain
    return ents[0]


def brain_with_constant_output(data: dict, output_index: int, value: float) -> dict:
    """A bias-only brain: keep the node array (layout is positional), drop every synapse and set the
    given output node to Linear with baseActivation=value. Other outputs get 0."""
    brain = data["brain"]
    nodes = brain["Nodes"]
    for node in nodes:
        idx = int(node["Index"])
        if idx >= 33:
            node["Type"] = 2            # NEATBrain.NodeType.Linear: value = bias when it has no inputs
            node["baseActivation"] = float(value) if idx == output_index else 0.0
            node["Value"] = float(value) if idx == output_index else 0.0
            node["LastInput"] = 0.0
            node["LastOutput"] = float(value) if idx == output_index else 0.0
    brain["Synapses"] = []
    data["brain"] = brain
    return data


def run_for_ticks(client, ticks: int, sim_tps: int) -> None:
    t = float(client.status()["simulatedTime"])
    target = t + ticks / sim_tps
    client.run_start({"simTime": target}, speed=1.0, sample=None, pause=True, replace=True)
    end = time.time() + max(60.0, ticks / sim_tps * 10)
    while time.time() < end:
        st = client.run_state(timeout=15.0)
        if st.get("state") not in ("running", "stopping"):
            return
        time.sleep(0.01)
    raise TimeoutError("the run did not stop")


def measure(client, handle: int) -> dict:
    e = client.entity(handle)["data"]
    rb = e["rb2d"]
    return {
        "x": float(rb["px"]), "y": float(rb["py"]),
        "vx": float(rb["vx"]), "vy": float(rb["vy"]),
        "rotation_deg": float(e["transform"]["rotation"]),
        "angular_velocity": float(rb.get("r", 0.0)) if False else None,  # rb2d.r is rotation, not angular velocity
        "id": int(e["body"]["id"]),
        "energy": float(e["body"]["energy"]),
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base", default=str(DEFAULT_BASE))
    ap.add_argument("--api-home", default=str(DEFAULT_API_HOME))
    ap.add_argument("--ticks", type=int, default=12, help="ticks to run per condition")
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--work-dir", default=str(PROJECT / "testdata" / "parity" / "_work"))
    args = ap.parse_args()

    api_home = Path(args.api_home)
    if not (api_home / "bibites_api").is_dir():
        raise SystemExit(f"bibites_api not found under {api_home}; set BIBITES_API_HOME")
    sys.path.insert(0, str(api_home))
    from bibites_api.live.client import BridgeClient
    from bibites_api.live.runtime import LiveRuntime
    from bibites_api.lab import Lab

    base = Path(args.base).absolute()
    runtime = None
    client = None
    try:
        client = BridgeClient(port=17800, timeout=60.0)
        if client.available():
            raise SystemExit("a game is already running; close it before running the experiment")
        log("starting hidden game ...")
        runtime = LiveRuntime(work_dir=Path(args.work_dir))
        lab = Lab(runtime, work_dir=Path(args.work_dir), window="hidden")
        client = lab.ensure_game(timeout=300)
        client.load_world(base)
        client.set_time(paused=True)
        sim_tps = int((client.get_settings()["independent"])["simTPS"])
        log(f"simTPS={sim_tps}")

        results = {}
        for label, value in (("control_zero", 0.0), ("rotate_plus_1", 1.0), ("rotate_minus_1", -1.0)):
            ent = find_bibite(client)
            handle = int(ent["h"])
            data = client.entities_json([handle])["items"][0]["data"]
            data = brain_with_constant_output(data, ROTATE_NODE, value)
            data["body"]["dead"] = False
            client.replace_many([{"handle": handle, "json": data}])
            # the replacement kept body id but produced a new handle
            ents = client.entities("bibite")
            new = [e for e in ents if e["id"] == int(data["body"]["id"])]
            new_handle = int(new[0]["h"]) if new else int(client.entities("bibite")[0]["h"])
            client.set_time(paused=True)
            before = measure(client, new_handle)
            # the game's angular velocity is not in the save JSON; the rotation change over a fixed
            # number of ticks is the sign convention we need. Sample after every tick-ish segment.
            trace = []
            for _ in range(4):
                run_for_ticks(client, max(1, args.ticks // 4), sim_tps)
                cur = measure(client, new_handle)
                trace.append({"d_rotation_deg": cur["rotation_deg"] - before["rotation_deg"],
                              "rotation_deg": cur["rotation_deg"],
                              "dx": cur["x"] - before["x"], "dy": cur["y"] - before["y"]})
                before = cur
            results[label] = {"output_value": value, "ticks": args.ticks, "trace": trace,
                              "total_d_rotation_deg": trace[-1]["d_rotation_deg"] if trace else None,
                              "net_dx": trace[-1]["dx"] if trace else None,
                              "net_dy": trace[-1]["dy"] if trace else None}
            log(f"{label}: total d_rotation={results[label]['total_d_rotation_deg']:+.3f} deg")
        out = {
            "schema": "rustibites.parity.rotation_sign.v1",
            "question": "sign of the brain Rotate output -> angular velocity / turn direction (BQ-4/Q1)",
            "method": "replace the bibite's brain with a bias-only Linear output node, remove synapses, "
                      "run ticks and measure the change in transform.rotation (Unity: + = counter-clockwise)",
            "base": str(base),
            "game": client.ping(),
            "sim_tps": sim_tps,
            "rotation_node_index": ROTATE_NODE,
            "results": results,
        }
        Path(args.out).write_text(json.dumps(out, indent=2), encoding="utf-8")
        log(f"wrote {args.out}")
        plus = results["rotate_plus_1"]["total_d_rotation_deg"]
        minus = results["rotate_minus_1"]["total_d_rotation_deg"]
        ok = plus is not None and minus is not None and plus > 0 and minus < 0
        log(f"VERDICT: Rotate +1 -> {'+' if (plus or 0) > 0 else ''}{plus:+.2f} deg (counter-clockwise) "
            f"and Rotate -1 -> {minus:+.2f} deg => {'CONSISTENT' if ok else 'INCONCLUSIVE'}")
        return 0 if ok else 1
    finally:
        try:
            if client is not None:
                client.close()
        except Exception:
            pass
        if runtime is not None:
            try:
                runtime.close()
                log("game stopped")
            except Exception as exc:  # noqa: BLE001
                log(f"could not stop the game: {exc}")


if __name__ == "__main__":
    sys.exit(main())

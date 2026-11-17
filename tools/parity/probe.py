#!/usr/bin/env python3
"""probe.py - interrogate the live BibitesBridge for the primitives the parity harness needs.

This is a development / smoke-test helper (not part of a capture). It talks to the
bridge in the already-running game (default 127.0.0.1:17800) and reports exactly
what the parity capture needs to know:

  * bridge / game / settings identity (ping, status, settings hash)
  * reseeding (/lab/seed): what it reports
  * per-tick sampling: start a run with a very small ``sample.every`` and report
    how many rows came back, the tick deltas between rows (gaps!), row size and
    the achieved simulation speed
  * a per-entity snapshot (``/bibites/table`` + ``/entities/json``): column
    coverage, JSON size, and which brain-state fields exist

It never starts or stops the game and never writes captures. Use it while a
headless game is running, e.g.::

    python -m bibites_api --no-browser            # in BibitesAPI/
    curl -X POST http://127.0.0.1:8800/api/live/game/start \
         -H 'content-type: application/json' \
         -d '{"window":"hidden","scenario":"Z:\\RUSTibites\\testdata\\real\\Default.zip","paused":true,"lab_mode":true}'
    python tools/parity/probe.py --ticks 60 --speed 10

Exit code 0 when the run completed and every sampled tick was observed, 1 otherwise.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import time
from pathlib import Path

DEFAULT_API_HOME = Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
DEFAULT_PORT = 17800

ALL_SAMPLE_FIELDS = ["counts", "pellets", "vitals", "generation", "species",
                     "brain", "genes", "lifecycle", "tags", "behavior", "body"]


def load_client(port: int):
    api_home = Path(os.environ.get("BIBITES_API_HOME", DEFAULT_API_HOME))
    if not (api_home / "bibites_api").is_dir():
        sys.exit(f"bibites_api package not found under {api_home}; set BIBITES_API_HOME")
    sys.path.insert(0, str(api_home))
    from bibites_api.live.client import BridgeClient  # noqa: E402  (after sys.path)
    return BridgeClient(port=port, timeout=60.0), api_home


def settings_hash(settings: dict) -> str:
    """Stable SHA-256 over the bridge's /settings {scenario, independent} JSON (sorted keys, no whitespace)."""
    blob = json.dumps(settings, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(blob).hexdigest()


def wait_run(client, timeout: float = 300.0, poll: float = 0.1) -> dict:
    end = time.time() + timeout
    last = {}
    while time.time() < end:
        last = client.run_state()
        if last.get("state") not in ("running", "stopping"):
            return last
        time.sleep(poll)
    raise TimeoutError(f"run did not finish: {last}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--port", type=int, default=DEFAULT_PORT, help="bridge port (default 17800)")
    ap.add_argument("--ticks", type=int, default=60, help="how many simulation ticks to run (default 60)")
    ap.add_argument("--speed", type=float, default=10.0, help="target game speed multiplier (default 10)")
    ap.add_argument("--seed", type=int, default=12345, help="seed passed to /lab/seed (default 12345; -1 = don't seed)")
    ap.add_argument("--every", type=float, default=0.001, help="sampler 'every' in sim seconds; tiny => one row per tick (default 0.001)")
    ap.add_argument("--fields", default="all", help="comma list of sampler groups, or 'all' (default all)")
    ap.add_argument("--no-snapshot", action="store_true", help="skip the per-entity snapshot probe")
    ap.add_argument("--json", action="store_true", help="print the probe report as JSON")
    args = ap.parse_args()

    client, api_home = load_client(args.port)
    out: dict = {"api_home": str(api_home)}
    try:
        out["ping"] = client.ping()
        st = client.status()
        out["status"] = st
        if not st.get("inSimulation"):
            print("no simulation is running in the game (start a world first)", file=sys.stderr)
            return 1
        settings = client.get_settings()
        out["settings_hash"] = settings_hash(settings)
        out["independent"] = settings.get("independent")
        out["sim_tps"] = (settings.get("independent") or {}).get("simTPS")

        if args.seed >= 0:
            out["seed"] = client.seed(args.seed)

        # -- per-tick sampler probe -------------------------------------------------------
        client.set_time(paused=True)
        start = client.status()
        t0 = float(start["simulatedTime"])
        sim_tps = float(out["sim_tps"] or 30)
        target = t0 + args.ticks / sim_tps
        fields = ALL_SAMPLE_FIELDS if args.fields == "all" else [f.strip() for f in args.fields.split(",") if f.strip()]
        sample = {"every": args.every, "fields": fields, "events": True, "eventsReset": True}
        wall0 = time.time()
        client.run_start({"simTime": target}, speed=args.speed, sample=sample, pause=True, replace=True)
        final = wait_run(client, timeout=max(120.0, args.ticks / sim_tps * 20 + 120))
        wall = time.time() - wall0
        series = client.series(0, 20000)
        rows = series.get("rows") or []
        out["run"] = {"final": final, "wall_s": wall, "rows": len(rows),
                      "achieved_speed": final.get("achievedSpeed"),
                      "sim_tps": sim_tps, "target_sim_s": args.ticks / sim_tps}
        out["t0"] = t0
        out["t1"] = final.get("simTime")
        out["row_sample"] = rows[0] if rows else None
        ticks = [int(r.get("tick", 0)) for r in rows]
        times = [float(r.get("t", 0.0)) for r in rows]
        deltas = [b - a for a, b in zip(ticks, ticks[1:])]
        dt = 1.0 / sim_tps
        expected_ticks = [round((t - t0) / dt) for t in times]
        out["ticks"] = {"n": len(ticks), "first": ticks[0] if ticks else None, "last": ticks[-1] if ticks else None,
                        "delta_histogram": {str(d): deltas.count(d) for d in sorted(set(deltas))},
                        "gaps": sum(1 for d in deltas if d != 1),
                        "time_step_s": dt, "expected_ticks": expected_ticks[:5] + ["..."] if rows else [],
                        "row_bytes": [len(json.dumps(r, separators=(",", ":")).encode()) for r in rows[:5]]}
        out["row_keys"] = sorted(rows[0].keys()) if rows else []
        out["row_keys_all"] = sorted({k for r in rows for k in r.keys()})
        out["events"] = (client.events(0, 20).get("events") or [])[:3]

        # -- per-entity snapshot probe ----------------------------------------------------
        if not args.no_snapshot:
            client.set_time(paused=True)
            tbl = client.bibite_table(genes=True, eggs=True)
            handles = list((tbl.get("columns") or {}).get("h") or [])
            got = client.entities_json(handles) if handles else {"items": []}
            out["snapshot"] = {
                "n": tbl.get("n"), "columns": sorted((tbl.get("columns") or {}).keys()),
                "genes": sorted((tbl.get("genes") or {}).keys()),
                "eggs": len(handles) - (tbl.get("n") or 0),
                "entity_json_bytes": [len(json.dumps(it["data"], separators=(",", ":")).encode()) for it in got.get("items", [])],
                "entity_data_keys": sorted(got["items"][0]["data"].keys()) if got.get("items") else [],
                "brain_keys": sorted(got["items"][0]["data"].get("brain", {}).get("Nodes", [{}])[0].keys()) if got.get("items") else [],
            }
    except Exception as e:  # noqa: BLE001 - a probe reports, it does not crash silently
        out["error"] = f"{type(e).__name__}: {e}"
        if args.json:
            print(json.dumps(out, indent=2))
        else:
            print(f"PROBE FAILED: {out['error']}", file=sys.stderr)
        return 1
    finally:
        client.close()

    if args.json:
        print(json.dumps(out, indent=2))
    else:
        st = out["status"]
        print(f"bridge {out['ping'].get('bridge')}  game {st.get('gameVersion')}  scene {st.get('scene')}")
        print(f"simTPS={out['sim_tps']}  settings_hash={out['settings_hash'][:16]}...  t0={out['t0']:.4f}")
        if "seed" in out:
            print(f"seed -> {out['seed'].get('seeded')}")
        run = out.get("run", {})
        print(f"run: {run.get('rows')} rows, wall {run.get('wall_s'):.2f}s, achievedSpeed {run.get('achieved_speed')}")
        tk = out.get("ticks", {})
        print(f"tick deltas: {tk.get('delta_histogram')}  gaps={tk.get('gaps')}  first={tk.get('first')} last={tk.get('last')}")
        print(f"row bytes (first 5): {tk.get('row_bytes')}  keys: {out.get('row_keys')}")
        if out.get("events"):
            print(f"first event: {json.dumps(out['events'][0])[:200]}")
        if "snapshot" in out:
            s = out["snapshot"]
            print(f"snapshot: n={s['n']} cols={s['columns']}")
            print(f"          genes={len(s['genes'])} entity-json bytes={s['entity_json_bytes']} data keys={s['entity_data_keys']}")
            print(f"          brain node keys={s['brain_keys']}")
    return 0 if "error" not in out and not out.get("ticks", {}).get("gaps") else 1


if __name__ == "__main__":
    sys.exit(main())

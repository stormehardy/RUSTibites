#!/usr/bin/env python3
"""capture_scenario.py - Round-2 scenario-start capture (tools/parity is read-only; this is the
support script under tools/parity_round2/).

What it does: starts the hidden game with the BibitesAPI lab harness, starts a NEW world from a
scenario zip (`POST /newgame`), pauses, records the effective settings, seeds via `/lab/seed`, and
captures the first N ticks at 1x with the same in-process sampler used by tools/parity/capture.py
(one record per tick + start/end per-entity snapshots).

What is and is not deterministic for a scenario start: `POST /newgame` runs the scenario's spawners
and initial seeding BEFORE any capture-side seed can land, using the game's unseeded random sources.
Two fresh starts of the same scenario therefore have different initial spawn positions/ids (and can
differ in seed biomass), so a scenario capture is NOT bit-reproducible ground truth and cannot be
compared per-entity against a Rust scenario start. What can be compared: population/biomass curve
shapes, zone configuration, spawner output counts, and (after the first divergence) statistical
ecology. The manifest records `taint` for exactly this, and `--repeat` documents the observed spread.

Usage:
    python tools/parity_round2/capture_scenario.py --scenario testdata/real/Ring.zip \
        --ticks 120 --seed 12345 --repeat 2 --out testdata/parity/captures_round2/ring_scenario
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
TOOLS_PARITY = ROOT / "tools" / "parity"
DEFAULT_API_HOME = Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
DEFAULT_OUT = ROOT / "testdata" / "parity" / "captures_round2" / "ring_scenario"

sys.path.insert(0, str(TOOLS_PARITY))
import capture as cap          # noqa: E402  (read-only reuse of the protocol helpers)
import parity_common as pc     # noqa: E402


def log(msg: str) -> None:
    print(f"[round2-scenario] {msg}", flush=True)


def build_args(ns: argparse.Namespace) -> argparse.Namespace:
    """The subset of capture.py's CLI namespace that WorldSession needs."""
    ns.work_dir = str(ROOT / "testdata" / "parity" / "_work")
    ns.port = 17800
    ns.plugin_port = 17801
    ns.no_plugin = False
    ns.attach = False
    ns.start_timeout = 300.0
    ns.keep_game = False
    return ns


def table_aggregates(client, gene_names):
    table = client.bibite_table(genes=True, eggs=True)
    return cap.table_aggregates(table, gene_names), table


def capture_pass(client, plugin, args, scenario: Path, pass_index: int, out: Path) -> dict:
    taint = []
    client.new_game(scenario, name=f"round2 scenario {scenario.stem}")
    client.set_time(paused=True)
    status = client.status()
    settings = client.get_settings()
    sim_tps = int(settings["independent"]["simTPS"])
    brain_tps = int(settings["independent"]["brainTPS"])
    dt = 1.0 / sim_tps
    settings_hash = pc.settings_hash(settings)
    gene_names = list(pc.GENE_NAMES)
    if plugin is not None:
        plugin.pacing(True)
    seeded = client.seed(args.seed) if args.seed is not None else None
    t0 = float(client.status()["simulatedTime"])
    entities0, table0 = cap.capture_entity_snapshot(client, gene_names)
    agg0_raw, _ = table_aggregates(client, gene_names)
    counts0 = dict(status.get("counts") or {})
    partial0 = {"schema": pc.TICK_SCHEMA, "source": "game", "tick": 0, "t": t0, "sim_tps": sim_tps,
                "brain_tps": brain_tps, "settings_hash": settings_hash, "partial": True,
                "counts": {"bibites": int(counts0.get("bibites", 0)), "eggs": int(counts0.get("eggs", 0)),
                           "corpses": int(counts0.get("corpses", 0))},
                "entities": entities0}
    partial0.update({k: v for k, v in agg0_raw.items() if v})
    records = {0: partial0}
    events = []
    event_next = 0
    target_t = t0 + args.ticks * dt
    t_now = float(client.status()["simulatedTime"])
    schedule_t = max(0.0, (target_t - t_now) - dt * 0.5)
    client.run_start({"extinct": False}, speed=args.speed,
                     sample={"every": 0.001, "fields": list(pc.SAMPLE_FIELDS), "events": True},
                     schedule=[{"t": schedule_t, "do": "POST /time", "body": {"paused": True}}],
                     pause=True, replace=True)
    cap.wait_until(lambda: client.status().get("paused") is True, 180.0, poll=0.01, what="scenario run to pause")
    client.run_cancel()
    rows = client.series(0, 20000).get("rows") or []
    for row in rows:
        tick = int(round((float(row["t"]) - t0) / dt))
        if tick <= 0:
            continue
        rec = pc.tick_record_from_row(row, source="game", tick=tick, sim_tps=sim_tps,
                                      brain_tps=brain_tps, settings_hash_value=settings_hash)
        records[tick] = rec
    if args.events:
        ev = client.events(event_next, 20000)
        for e in ev.get("events") or []:
            events.append(pc.event_record(e, tick=int(round((float(e["t"]) - t0) / dt))))
        event_next = ev.get("next", event_next)
    final_t = float(client.status()["simulatedTime"])
    final_tick = int(round((final_t - t0) / dt))
    entities1, _ = cap.capture_entity_snapshot(client, gene_names)
    if final_tick in records:
        records[final_tick]["entities"] = entities1
    else:
        records[final_tick] = {"schema": pc.TICK_SCHEMA, "source": "game", "tick": final_tick, "t": final_t,
                               "sim_tps": sim_tps, "brain_tps": brain_tps, "settings_hash": settings_hash,
                               "partial": True, "entities": entities1}
    if args.seed is None:
        taint.append("no --seed: RNG state not fixed")
    taint.append("scenario start: POST /newgame spawners/seeding run before the capture seed (unseeded "
                 "UnityEngine.Random); two fresh starts differ in initial spawn positions/ids and can differ "
                 "in seed biomass, so this capture is NOT bit-reproducible and per-entity comparison against a "
                 "Rust scenario start is not meaningful - compare curve shapes/zone/spawner structure instead")
    if args.speed != 1.0:
        taint.append(f"speed {args.speed}x: scenario capture is statistical only (frame grouping); use 1.0 "
                     f"for deterministic protocol runs")
    if plugin is None:
        taint.append("parity plugin unavailable: Time.fixedDeltaTime not pinned; ticks are frame-rate coupled")
    ordered = [records[k] for k in sorted(records)]
    out.mkdir(parents=True, exist_ok=True)
    files = {"ticks": pc.write_jsonl_gz(out / "ticks.jsonl.gz", ordered)}
    if args.events:
        files["events"] = pc.write_jsonl_gz(out / "events.jsonl.gz", sorted(events, key=lambda e: e["tick"]))
    end_status = client.status()
    manifest = {
        "schema": pc.CAPTURE_SCHEMA, "kind": "capture", "source": "game",
        "capture": scenario.stem, "pass": pass_index,
        "created_at": datetime.now(timezone.utc).isoformat(),
        "engine": {"name": "The Bibites", "game_version": status.get("gameVersion"),
                   "bridge_version": client.ping().get("bridge")},
        "gene_names": gene_names,
        "inputs": {"scenario": {"path": str(scenario), "sha256": cap.sha256_of(scenario)},
                   "base": None, "save": None, "seed": args.seed,
                   "seeded_parts": (seeded or {}).get("seeded"),
                   "settings_requested": {"scenario": settings.get("scenario"), "independent": settings.get("independent")}},
        "settings": settings, "settings_hash": settings_hash,
        "sim": {"sim_tps": sim_tps, "brain_tps": brain_tps, "dt": dt, "base_time": t0, "start_t": t0,
                "start_tick": 0, "end_tick": final_tick, "ticks": final_tick,
                "wall_s": round(time.time() - started_at(pass_index), 3),
                "achieved_ticks_per_s": None, "achieved_speed": None,
                "overshoot_ticks": [final_tick - args.ticks]},
        "world": {"simulation_size": end_status.get("simulationSize"), "zones": client.zones()},
        "counts_start": counts0, "counts_end": dict(end_status.get("counts") or {}),
        "plugin": {"available": plugin is not None, "dt_pinned": plugin is not None},
        "taint": taint,
        "files": files,
    }
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    log(f"pass {pass_index}: {len(ordered)} records tick 0..{final_tick}, counts_end={manifest['counts_end']}")
    return manifest


_START = {}


def started_at(pass_index: int) -> float:
    return _START.setdefault(pass_index, time.time())


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--scenario", default=str(ROOT / "testdata" / "real" / "Default.zip"))
    ap.add_argument("--ticks", type=int, default=120)
    ap.add_argument("--seed", type=int, default=12345)
    ap.add_argument("--repeat", type=int, default=2)
    ap.add_argument("--speed", type=float, default=1.0,
                    help="game speed while sampling (1.0 = deterministic regime; >1 is statistical only)")
    ap.add_argument("--out", default=str(DEFAULT_OUT))
    ap.add_argument("--api-home", default=str(DEFAULT_API_HOME))
    ap.add_argument("--events", action="store_true", default=True)
    ap.add_argument("--no-events", dest="events", action="store_false")
    args = ap.parse_args()
    build_args(args)
    scenario = Path(args.scenario).absolute()
    if not scenario.exists():
        raise SystemExit(f"scenario not found: {scenario}")
    BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(args.api_home))
    out = Path(args.out)
    session = cap.WorldSession(args, BridgeClient, LiveRuntime, Lab)
    manifests = []
    try:
        client = session.open()
        plugin = cap.ParityPlugin(args.plugin_port) if session.plugin_ok else None
        if plugin is not None:
            log("parity plugin present: dt pin armed")
        for i in range(max(1, args.repeat)):
            pass_dir = out / f"pass-{i}" if args.repeat > 1 else out
            manifests.append(capture_pass(client, plugin, args, scenario, i, pass_dir))
        out.mkdir(parents=True, exist_ok=True)
        det = None
        if len(manifests) > 1:
            digs = []
            for m in manifests:
                p = out / f"pass-{m['pass']}"
                digs.append({int(r["tick"]): pc.record_digest(r) for r in pc.open_capture(p).records()})
            common = sorted(set.intersection(*(set(d) for d in digs)))
            diff = [t for t in common if len({d[t] for d in digs}) > 1]
            det = {"passes": len(digs), "common_ticks": len(common), "differing_ticks": len(diff),
                   "first_differing_tick": diff[0] if diff else None,
                   "bit_identical": not diff,
                   "note": "scenario starts run spawners/seeding before the seed; a nonzero first_difference "
                           "confirms the documented scenario-start nondeterminism"}
            (out / "determinism.json").write_text(json.dumps(det, indent=2), encoding="utf-8")
            log(f"determinism: bit_identical={det['bit_identical']} differing={det['differing_ticks']}/{det['common_ticks']} "
                f"first={det['first_differing_tick']}")
        (out / "scenario_capture.json").write_text(json.dumps(
            {"schema": "rustibites.parity.scenario_capture.v1", "scenario": str(scenario),
             "passes": len(manifests), "determinism": det}, indent=2), encoding="utf-8")
        return 0
    finally:
        session.close()


if __name__ == "__main__":
    raise SystemExit(main())

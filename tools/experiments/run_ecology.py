#!/usr/bin/env python3
"""run_ecology.py - long-run ecology validation for the RUSTibites engine (task-57).

Runs the headless engine on a real scenario/save for a bounded number of ticks with a fixed seed and
threads, captures the per-tick parity stream (`--capture`), the final stats row (`--stats csv`), and a
final save. The capture stream is converted to `series.csv`, an analysed `summary.json` and PNG plots.

What is analysed: population dynamics, plant/meat/free/bibite biomass, birth/death rates, species
counts, energy/health means, extinction/collapse, resource freeze, NaN/Inf, and simple "obviously
broken" flags. A chunked mode (`--snapshot-every N`) continues the world through intermediate saves to
produce periodic snapshots; it re-seeds the RNG per chunk (the engine seeds from `--seed`), so the
chunked run is a set of continuation experiments, not one continuous trajectory - use the single-run
capture for the time series.

Usage:
    python tools/experiments/run_ecology.py --input testdata/real/Ring.zip --name ring_scenario \
        --ticks 20000 --seed 12345 --threads 8
    python tools/experiments/run_ecology.py --input testdata/real/saves/lol.zip --name lol_save \
        --ticks 20000 --snapshot-every 5000
"""
from __future__ import annotations

import argparse
import csv
import json
import math
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
DEFAULT_ENGINE = ROOT / "target" / "release" / "rustibites.exe"
DEFAULT_OUT = ROOT / "testdata" / "experiments"
METRIC_KEYS = [
    "counts.bibites", "counts.eggs", "counts.corpses",
    "pellets.plant_n", "pellets.plant_biomass", "pellets.meat_n", "pellets.meat_biomass",
    "pellets.free_biomass", "pellets.bibite_biomass",
    "vitals.energy_mean", "vitals.health_mean", "vitals.age_mean", "vitals.age_max",
    "generation.gen_max", "generation.gen_mean",
    "species.n", "brain.nodes_mean", "brain.syn_mean",
    "behavior.speed_mean", "behavior.moving_frac", "behavior.bite_rate_mean",
    "behavior.egg_rate_mean", "behavior.travel_rate_mean",
]


def log(msg: str) -> None:
    print(f"[ecology] {msg}", flush=True)


def run_engine(engine: Path, input_path: Path, ticks: int, seed: int, threads: int,
               capture: Path | None, save: Path | None, timeout: float) -> tuple[dict, float, str]:
    cmd = [str(engine), "run", str(input_path), "--ticks", str(ticks), "--seed", str(seed),
           "--threads", str(threads), "--stats", "csv", "--quiet"]
    if capture is not None:
        cmd += ["--capture", str(capture)]
    if save is not None:
        cmd += ["--save", str(save)]
    start = time.time()
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, cwd=str(ROOT))
    wall = time.time() - start
    if proc.returncode != 0:
        raise RuntimeError(f"engine failed ({proc.returncode}): {proc.stderr.strip()[:2000]}")
    stats: dict = {}
    # parse the csv block (header + one data row)
    lines = [ln for ln in proc.stdout.splitlines() if ln.strip()]
    for i, ln in enumerate(lines):
        if ln.startswith("label,"):
            header = ln.split(",")
            if i + 1 < len(lines):
                vals = lines[i + 1].split(",")
                stats = dict(zip(header, vals))
            break
    return stats, wall, proc.stdout + proc.stderr


def flat_get(rec: dict, key: str):
    cur = rec
    for part in key.split("."):
        if not isinstance(cur, dict):
            return None
        cur = cur.get(part)
        if cur is None:
            return None
    return cur


def capture_to_series(capture_dir: Path, csv_path: Path) -> list[dict]:
    ticks = capture_dir / "ticks.jsonl"
    if not ticks.exists():
        ticks = capture_dir / "ticks.jsonl.gz"
    opener = (lambda p: __import__("gzip").open(p, "rt", encoding="utf-8")) if ticks.name.endswith(".gz") else (lambda p: open(p, "rt", encoding="utf-8"))
    rows = []
    with opener(ticks) as fh:
        for line in fh:
            if not line.strip():
                continue
            rec = json.loads(line)
            row = {"tick": rec.get("tick"), "t": rec.get("t")}
            for key in METRIC_KEYS:
                row[key] = flat_get(rec, key)
            lc = rec.get("lifecycle") or {}
            row["lifecycle.births"] = lc.get("births", 0)
            row["lifecycle.deaths"] = lc.get("deaths", 0)
            sp = rec.get("species") or {}
            row["species.ids"] = json.dumps(sp.get("counts") or {}, sort_keys=True)
            rows.append(row)
    csv_path.parent.mkdir(parents=True, exist_ok=True)
    with open(csv_path, "w", newline="", encoding="utf-8") as fh:
        writer = csv.DictWriter(fh, fieldnames=list(rows[0].keys()) if rows else ["tick"])
        writer.writeheader()
        writer.writerows(rows)
    return rows


def analyse(rows: list[dict], window: int = 500) -> dict:
    n = len(rows)
    if not n:
        return {"records": 0, "flags": ["NO_RECORDS"]}
    pop = [float(r["counts.bibites"] or 0) for r in rows]
    energy = [float(r["vitals.energy_mean"] or 0) for r in rows]
    health = [float(r["vitals.health_mean"] or 0) for r in rows]
    plant_n = [float(r["pellets.plant_n"] or 0) for r in rows if r["pellets.plant_n"] is not None]
    births = [int(r["lifecycle.births"] or 0) for r in rows]
    deaths = [int(r["lifecycle.deaths"] or 0) for r in rows]
    species_n = [int(r["species.n"] or 0) for r in rows]
    # cumulative lifecycle counters (capture rows are deltas between samples)
    cum_births = [sum(births[: i + 1]) for i in range(n)]
    cum_deaths = [sum(deaths[: i + 1]) for i in range(n)]
    first_birth = next((rows[i]["tick"] for i in range(n) if births[i] > 0), None)
    first_death = next((rows[i]["tick"] for i in range(n) if deaths[i] > 0), None)
    extinction_tick = next((rows[i]["tick"] for i in range(n) if pop[i] == 0), None)
    species_first = next((rows[i]["tick"] for i in range(n) if species_n[i] > 1), None)
    # windowed rates (births/deaths per 1000 ticks)
    rates = []
    for i in range(0, n, window):
        chunk = rows[i:i + window]
        dt_ticks = max(1, (chunk[-1]["tick"] - chunk[0]["tick"]) or window)
        rates.append({"tick0": chunk[0]["tick"], "tick1": chunk[-1]["tick"],
                      "births": sum(int(r["lifecycle.births"] or 0) for r in chunk),
                      "deaths": sum(int(r["lifecycle.deaths"] or 0) for r in chunk),
                      "births_per_1000": 1000.0 * sum(int(r["lifecycle.births"] or 0) for r in chunk) / dt_ticks,
                      "deaths_per_1000": 1000.0 * sum(int(r["lifecycle.deaths"] or 0) for r in chunk) / dt_ticks,
                      "population": chunk[-1]["counts.bibites"]})
    # NaN/Inf scan over numeric columns
    bad = 0
    for r in rows:
        for k, v in r.items():
            if isinstance(v, float) and (math.isnan(v) or math.isinf(v)):
                bad += 1
    final = rows[-1]
    plant_frozen = False
    if len(plant_n) >= max(10, n // 2):
        tail = plant_n[len(plant_n) // 2:]
        plant_frozen = max(tail) == min(tail)
    continuous_growth = pop[-1] > pop[0] and pop[-1] >= max(pop[: max(1, n // 2)]) and pop[-1] >= 50
    flags = []
    if bad:
        flags.append(f"NAN_OR_INF({bad})")
    if extinction_tick is not None:
        flags.append(f"EXTINCT(tick {extinction_tick})")
    if first_birth is None and n > 1000:
        flags.append("NO_REPRODUCTION")
    if n > 1000 and min(pop) == max(pop):
        flags.append("STATIC_POPULATION")
    if final["counts.bibites"] and float(final["vitals.energy_mean"] or 0) == 0.0:
        flags.append("ENERGY_ZERO_ALIVE")
    if plant_frozen:
        flags.append("RESOURCE_FROZEN")
    if continuous_growth:
        flags.append("CONTINUOUS_GROWTH")
    if rows[0]["tick"] is not None and float(rows[0]["vitals.health_mean"] or 0) <= 0:
        flags.append("ZERO_HEALTH_START")
    return {
        "records": n,
        "tick_range": [rows[0]["tick"], rows[-1]["tick"]],
        "population": {"start": pop[0], "final": pop[-1], "min": min(pop), "max": max(pop),
                       "mean": sum(pop) / n},
        "energy_mean": {"start": energy[0], "final": energy[-1], "min": min(energy), "max": max(energy)},
        "health_mean": {"start": health[0], "final": health[-1], "min": min(health), "max": max(health)},
        "plant_n": {"start": plant_n[0] if plant_n else None, "final": plant_n[-1] if plant_n else None,
                    "min": min(plant_n) if plant_n else None, "max": max(plant_n) if plant_n else None},
        "births": {"total": cum_births[-1] if n else 0, "first_tick": first_birth},
        "deaths": {"total": cum_deaths[-1] if n else 0, "first_tick": first_death},
        "species": {"start": species_n[0], "final": species_n[-1], "max": max(species_n),
                    "first_multi_species_tick": species_first},
        "generation_final": final.get("generation.gen_max"),
        "rates": rates,
        "flags": flags,
    }


def make_plots(rows: list[dict], out_dir: Path, name: str) -> list[str]:
    try:
        import matplotlib
        matplotlib.use("Agg")
        import matplotlib.pyplot as plt
    except Exception as exc:  # noqa: BLE001
        log(f"matplotlib unavailable ({exc}); CSV + tables only")
        return []
    out_dir.mkdir(parents=True, exist_ok=True)
    ticks = [r["tick"] for r in rows]
    made = []

    def save(fig, fname):
        path = out_dir / fname
        fig.tight_layout()
        fig.savefig(path, dpi=110)
        plt.close(fig)
        made.append(str(path))

    def series(key):
        return [r[key] for r in rows]

    fig, ax = plt.subplots(2, 1, figsize=(11, 7), sharex=True)
    ax[0].plot(ticks, series("counts.bibites"), label="bibites", color="tab:blue")
    ax[0].plot(ticks, series("counts.eggs"), label="eggs", color="tab:orange")
    ax[0].plot(ticks, series("counts.corpses"), label="corpses", color="tab:gray")
    ax[0].set_ylabel("count"); ax[0].legend(); ax[0].set_title(f"{name}: population")
    ax[1].plot(ticks, series("vitals.energy_mean"), label="energy mean", color="tab:green")
    ax[1].plot(ticks, series("vitals.health_mean"), label="health mean", color="tab:red")
    ax[1].set_ylabel("mean amount"); ax[1].legend(); ax[1].set_xlabel("tick")
    save(fig, "population_energy.png")

    fig, ax = plt.subplots(2, 1, figsize=(11, 7), sharex=True)
    for key, label in (("pellets.plant_biomass", "plant"), ("pellets.meat_biomass", "meat"),
                       ("pellets.free_biomass", "free"), ("pellets.bibite_biomass", "bibite")):
        ax[0].plot(ticks, series(key), label=label)
    ax[0].set_ylabel("biomass"); ax[0].legend(); ax[0].set_title(f"{name}: biomass")
    ax[1].plot(ticks, series("pellets.plant_n"), label="plant pellets", color="tab:green")
    ax[1].plot(ticks, series("pellets.meat_n"), label="meat pellets", color="tab:red")
    ax[1].set_ylabel("pellet count"); ax[1].legend(); ax[1].set_xlabel("tick")
    save(fig, "biomass.png")

    fig, ax = plt.subplots(figsize=(11, 4))
    b = [r["lifecycle.births"] or 0 for r in rows]
    d = [r["lifecycle.deaths"] or 0 for r in rows]
    step = max(1, len(rows) // 2000)
    ax.plot(ticks[::step], b[::step], label="births/sample", alpha=0.7)
    ax.plot(ticks[::step], d[::step], label="deaths/sample", alpha=0.7)
    ax.set_title(f"{name}: births/deaths per sampled tick"); ax.set_xlabel("tick"); ax.legend()
    save(fig, "births_deaths.png")

    fig, ax = plt.subplots(2, 1, figsize=(11, 6), sharex=True)
    ax[0].plot(ticks, series("species.n"), label="species count")
    ax[0].plot(ticks, series("generation.gen_max"), label="max generation")
    ax[0].legend(); ax[0].set_ylabel("count"); ax[0].set_title(f"{name}: species/generation")
    ax[1].plot(ticks, series("behavior.speed_mean"), label="speed mean")
    ax[1].plot(ticks, series("behavior.bite_rate_mean"), label="bite rate mean")
    ax[1].plot(ticks, series("behavior.egg_rate_mean"), label="egg rate mean")
    ax[1].legend(); ax[1].set_xlabel("tick")
    save(fig, "species_behavior.png")
    return made


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--input", required=True, help="scenario zip, save zip or genome .bb8")
    ap.add_argument("--name", help="experiment name (default: input stem)")
    ap.add_argument("--ticks", type=int, default=20000)
    ap.add_argument("--seed", type=int, default=12345)
    ap.add_argument("--threads", type=int, default=8)
    ap.add_argument("--engine", default=str(DEFAULT_ENGINE))
    ap.add_argument("--out-root", default=str(DEFAULT_OUT))
    ap.add_argument("--out", help="output directory (default <out-root>/<name>)")
    ap.add_argument("--snapshot-every", type=int, default=0,
                    help="chunked mode: continue the world through intermediate saves every N ticks "
                         "(RNG is re-seeded per chunk; see module docstring)")
    ap.add_argument("--keep-raw", action="store_true", help="keep the raw --capture ticks.jsonl (can be large)")
    ap.add_argument("--timeout", type=float, default=1800.0)
    args = ap.parse_args()
    engine = Path(args.engine)
    if not engine.exists():
        raise SystemExit(f"engine not found: {engine} (cargo build --release -p bibites-cli)")
    input_path = Path(args.input).absolute()
    if not input_path.exists():
        raise SystemExit(f"input not found: {input_path}")
    name = args.name or input_path.stem.replace(" ", "_")
    out = Path(args.out).absolute() if args.out else Path(args.out_root).absolute() / name
    out.mkdir(parents=True, exist_ok=True)
    capture = out / "raw_capture"
    if capture.exists():
        shutil.rmtree(capture)
    log(f"run {name}: {input_path.name}, {args.ticks} ticks, seed={args.seed}, threads={args.threads}")
    stats, wall, engine_log = run_engine(engine, input_path, args.ticks, args.seed, args.threads,
                                         capture, out / "final.zip", args.timeout)
    (out / "engine_stdout.log").write_text(engine_log, encoding="utf-8")
    rows = capture_to_series(capture, out / "series.csv")
    summary = analyse(rows)
    summary.update({"name": name, "input": str(input_path), "ticks": args.ticks, "seed": args.seed,
                    "threads": args.threads, "wall_s": round(wall, 2), "stats": stats,
                    "engine": str(engine),
                    "engine_version": subprocess.run([str(engine), "version"], capture_output=True, text=True).stdout.strip()})
    plots = make_plots(rows, out / "plots", name)
    summary["plots"] = plots
    if not args.keep_raw:
        shutil.rmtree(capture, ignore_errors=True)
    summary["raw_capture_kept"] = bool(args.keep_raw)

    # optional chunked continuation with periodic saves
    if args.snapshot_every > 0:
        chunks = []
        cur = input_path
        ticks_done = 0
        i = 0
        chunk_dir = out / "snapshots"
        chunk_dir.mkdir(exist_ok=True)
        while ticks_done < args.ticks:
            n = min(args.snapshot_every, args.ticks - ticks_done)
            dest = chunk_dir / f"chunk_{i:02d}_t{ticks_done + n}.zip"
            st, w, _ = run_engine(engine, cur, n, args.seed, args.threads, None, dest, args.timeout)
            chunks.append({"index": i, "ticks": n, "wall_s": round(w, 2), "save": str(dest),
                           "bibites_end": st.get("bibites_end"), "state_fingerprint": st.get("state_fingerprint")})
            log(f"chunk {i}: {n} ticks, bibites_end={st.get('bibites_end')}, save={dest.name}")
            cur = dest
            ticks_done += n
            i += 1
        summary["chunked_snapshots"] = {"every": args.snapshot_every, "chunks": chunks,
                                        "note": "RNG re-seeded per chunk; continuation experiments, not one trajectory"}

    (out / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    flags = summary.get("flags") or []
    log(f"done: wall {wall:.2f}s, pop {summary['population']['start']:.0f}->{summary['population']['final']:.0f} "
        f"(min {summary['population']['min']:.0f}/max {summary['population']['max']:.0f}), "
        f"births {summary['births']['total']}, deaths {summary['deaths']['total']}, flags={flags}")
    log(f"artifacts: {out} (series.csv, summary.json, plots/, final.zip)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

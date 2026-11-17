#!/usr/bin/env python3
"""trace_compare.py - pair game and Rust per-tick traces on the identical-base fixture (task-122).

Game trace: plugin JSONL (`trace_600/game_trace.jsonl`; one or more rows per body per tick; duplicates from
the hook are de-duplicated by (relative_tick,id), keeping the last row). Rust trace: `RUSTIBITES_TRACE=1`
capture `trace.jsonl` (`rustibites.parity.trace.v1`). Both are aligned by relative tick (game trace header
absolute tick vs Rust run-relative tick) and joined by entity id.

Outputs `compare.json` + `report.md` with first-divergence tick per id, position/rot/vel/angvel/energy/
contact/nearest-plant deltas, command deltas (game accelerate/rotate vs Rust), and focused tables for the
two missed feeders 465701042/379567021. Usage:
  python tools/parity/trace_compare.py --game testdata/parity/default/trace_600/game_trace.jsonl \
      --rust testdata/parity/default/rust_trace_600/trace.jsonl --out testdata/parity/default/trace_600
"""
from __future__ import annotations

import argparse
import json
import math
import statistics
from collections import defaultdict
from pathlib import Path

FOCUS = (465701042, 379567021)
FIELDS = ("x", "y", "rot", "vx", "vy", "angvel", "energy", "nearest_plant", "accelerate", "rotate", "n_in_mouth")


def angdiff(a, b):
    d = (a - b) % 360.0
    return d - 360.0 if d > 180.0 else d


def load_game(path):
    base = None
    rows = defaultdict(dict)
    with open(path, encoding="utf-8") as f:
        for line in f:
            d = json.loads(line)
            if "trace_header" in d:
                base = int(d.get("tick") or 0)
                continue
            tick = int(d["tick"]) - (base or 0) - 1  # plugin postfix runs before the tick's physics update
            rows[(tick, int(d["id"]))] = {
                "tick": tick, "id": int(d["id"]), "x": d["x"], "y": d["y"], "rot": d["rot"],
                "vx": d["vx"], "vy": d["vy"], "angvel": d["angvel"], "energy": d["energy"],
                "nearest_plant": d.get("nearest_plant"), "accelerate": d.get("accelerate"),
                "rotate": d.get("rotate"), "n_in_mouth": d.get("n_in_mouth"),
                "pre_x": d.get("pre_x"), "pre_y": d.get("pre_y"), "pre_rot": d.get("pre_rot"),
                "rb_pre_x": d.get("rb_pre_x"), "rb_pre_y": d.get("rb_pre_y"), "rb_pre_rot": d.get("rb_pre_rot"),
                "rb_x": d.get("rb_x"), "rb_y": d.get("rb_y"), "rb_rot": d.get("rb_rot"),
                "contact_count": d.get("contact_count"), "contact_impulse": d.get("contact_impulse"),
                "contact_relvel": d.get("contact_relvel"), "mouth_targets": d.get("mouth_targets")}
    return base, rows


def load_rust(path):
    rows = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            d = json.loads(line)
            tick = int(d["tick"])
            for e in d.get("entities") or []:
                rows[(tick, int(e["id"]))] = {
                    "tick": tick, "id": int(e["id"]), "x": e["x"], "y": e["y"],
                    "rot": e.get("rot_deg"), "vx": e.get("vel_x"), "vy": e.get("vel_y"),
                    "angvel": e.get("ang_vel_deg"), "energy": e.get("energy"),
                    "nearest_plant": e.get("nearest_plant_distance"),
                    "nearest_plant_radius": e.get("nearest_plant_radius"),
                    "accelerate": e.get("accelerate"), "rotate": e.get("rotate"),
                    "n_in_mouth": e.get("n_in_mouth"),
                    "contact_count": e.get("contact_count"), "contact_impulse": e.get("contact_impulse")}
    return rows


def fnum(v):
    return float(v) if isinstance(v, (int, float)) else None


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--game", required=True)
    ap.add_argument("--rust", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--pos-tol", type=float, default=1e-2)
    args = ap.parse_args()
    out = Path(args.out)
    gbase, game = load_game(Path(args.game))
    rust = load_rust(Path(args.rust))
    ids = sorted({k[1] for k in game if k in rust})
    summary = {"schema": "rustibites.parity.trace_compare.v1",
               "game_trace_base_absolute_tick": gbase, "paired_ids": ids,
               "paired_ticks": len({k[0] for k in game if k in rust}), "entities": {}}
    focus = {}
    for eid in ids:
        series = []
        first_div = None
        for (tick, _), g in sorted(game.items()):
            if _ != eid:
                continue
            r = rust.get((tick, eid))
            if not r:
                continue
            dx, dy = g["x"] - r["x"], g["y"] - r["y"]
            pos_err = math.hypot(dx, dy)
            rot_err = angdiff(g["rot"], r["rot"]) if g["rot"] is not None and r["rot"] is not None else None
            vel_err = math.hypot(g["vx"] - r["vx"], g["vy"] - r["vy"])
            ang_err = (g["angvel"] - r["angvel"]) if g["angvel"] is not None and r["angvel"] is not None else None
            npg = fnum(g.get("nearest_plant"))
            npr = fnum(r.get("nearest_plant"))
            np_err = (npg - npr) if npg is not None and npr is not None else None
            acc_err = (fnum(g.get("accelerate")) - fnum(r.get("accelerate"))
                       if fnum(g.get("accelerate")) is not None and fnum(r.get("accelerate")) is not None else None)
            rot_cmd_err = (fnum(g.get("rotate")) - fnum(r.get("rotate"))
                           if fnum(g.get("rotate")) is not None and fnum(r.get("rotate")) is not None else None)
            row = {"tick": tick, "pos_err": pos_err, "rot_err": rot_err, "vel_err": vel_err,
                   "angvel_err": ang_err, "energy_game": g.get("energy"), "energy_rust": r.get("energy"),
                   "nearest_game": npg, "nearest_rust": npr, "nearest_err": np_err,
                   "n_in_mouth_game": g.get("n_in_mouth"), "n_in_mouth_rust": r.get("n_in_mouth"),
                   "accelerate_game": fnum(g.get("accelerate")), "accelerate_rust": fnum(r.get("accelerate")),
                   "rotate_game": fnum(g.get("rotate")), "rotate_rust": fnum(r.get("rotate")),
                   "acc_err": acc_err, "rot_cmd_err": rot_cmd_err}
            series.append(row)
            if first_div is None and pos_err > args.pos_tol:
                first_div = row
        if not series:
            continue
        vals = lambda k: [s[k] for s in series if s[k] is not None]
        pe = vals("pos_err")
        gcontacts = [g.get("contact_count") or 0 for (t, i), g in game.items() if i == eid]
        rcontacts = [r.get("contact_count") or 0 for (t, i), r in rust.items() if i == eid]
        entry = {"first_divergence_tick": first_div["tick"] if first_div else None,
                 "solid_contact_ticks_game": sum(1 for v in gcontacts if v),
                 "solid_contact_ticks_rust": sum(1 for v in rcontacts if v),
                 "solid_impulse_sum_game": sum((g.get("contact_impulse") or 0) for (t, i), g in game.items() if i == eid),
                 "solid_impulse_sum_rust": sum((r.get("contact_impulse") or 0) for (t, i), r in rust.items() if i == eid),
                 "first_divergence": first_div,
                 "pos_err_max": max(pe) if pe else None, "pos_err_mean": statistics.mean(pe) if pe else None,
                 "pos_err_final": series[-1]["pos_err"],
                 "rot_err_max": max(vals("rot_err"), default=None),
                 "vel_err_max": max(vals("vel_err"), default=None),
                 "energy_final_game": series[-1]["energy_game"], "energy_final_rust": series[-1]["energy_rust"],
                 "contact_ticks_game": sum(1 for s in series if (s["n_in_mouth_game"] or 0) > 0),
                 "contact_ticks_rust": sum(1 for s in series if (s["n_in_mouth_rust"] or 0) > 0),
                 "nearest_game_min": min((s["nearest_game"] for s in series if s["nearest_game"] is not None), default=None),
                 "nearest_rust_min": min((s["nearest_rust"] for s in series if s["nearest_rust"] is not None), default=None),
                 "acc_cmd_max_abs_err": max((abs(s["acc_err"]) for s in series if s["acc_err"] is not None), default=None),
                 "rot_cmd_max_abs_err": max((abs(s["rot_cmd_err"]) for s in series if s["rot_cmd_err"] is not None), default=None),
                 "solid_contacts_game": sum(1 for s in series if (s.get("n_in_mouth_game") is not None and False) or False),
                 "game_contact_max": None, "rust_contact_max": None}
        summary["entities"][str(eid)] = entry
        if eid in FOCUS:
            first = first_div or series[0]
            focus[str(eid)] = {"first_divergence": first,
                               "at_first_divergence_game": game.get((first["tick"], eid)),
                               "at_first_divergence_rust": rust.get((first["tick"], eid)),
                               "final_game": game.get((series[-1]["tick"], eid)),
                               "final_rust": rust.get((series[-1]["tick"], eid))}
    summary["focus"] = focus

    # phase/contact detail for the two focus ids (first 80 paired ticks)
    phase = {}
    for eid in FOCUS:
        rows_g = sorted((g for (t, i), g in game.items() if i == eid), key=lambda r: r["tick"])
        rows_r = sorted((r for (t, i), r in rust.items() if i == eid), key=lambda r: r["tick"])
        gm = {r["tick"]: r for r in rows_g}
        rm = {r["tick"]: r for r in rows_r}
        ticks = sorted(set(gm) & set(rm))[:80]
        detail = []
        pg = pr = None
        first_phase = None
        for t in ticks:
            g, r = gm[t], rm[t]
            step_g = None; step_r = None
            if pg is not None:
                step_g = (g["rb_x"] - pg["rb_x"], g["rb_y"] - pg["rb_y"])
                step_r = (r["x"] - pr["x"], r["y"] - pr["y"])
                se = math.hypot(step_g[0] - step_r[0], step_g[1] - step_r[1])
                if first_phase is None and se > 1e-3:
                    first_phase = {"tick": t, "step_err": se, "game_step": step_g, "rust_step": step_r}
            detail.append({"tick": t, "game_rb": [g["rb_x"], g["rb_y"]], "game_pre": [g["pre_x"], g["pre_y"]],
                           "game_rb_pre": [g["rb_pre_x"], g["rb_pre_y"]],
                           "rust": [r["x"], r["y"]], "game_vel": [g["vx"], g["vy"]],
                           "rust_vel": [r["vx"], r["vy"]], "step_game": step_g, "step_rust": step_r,
                           "contact_count": g.get("contact_count"), "contact_impulse": g.get("contact_impulse"),
                           "n_in_mouth_game": g.get("n_in_mouth"), "n_in_mouth_rust": r.get("n_in_mouth"),
                           "acc_game": g.get("accelerate"), "acc_rust": r.get("accelerate"),
                           "rot_game": g.get("rotate"), "rot_rust": r.get("rotate")})
            pg, pr = g, r
        phase[str(eid)] = {"first_step_divergence": first_phase, "rows": detail}
    summary["phase_focus"] = phase
    out.mkdir(parents=True, exist_ok=True)
    (out / "compare.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    lines = ["# Per-tick game vs Rust trace comparison (task-122)", "",
             f"Game trace base absolute tick: {gbase}; paired ids: {ids}; paired ticks: {summary['paired_ticks']}", "",
             "| id | first divergence tick | pos err max | pos err final | rot err max | vel err max | contact gam/rust | nearest gam/rust min | acc cmd err max | rot cmd err max |",
             "|---|---|---|---|---|---|---|---|---|---|"]
    for eid, e in summary["entities"].items():
        lines.append(f"| {eid} | {e['first_divergence_tick']} | {e['pos_err_max']:.4g} | {e['pos_err_final']:.4g} | "
                     f"{e['rot_err_max'] if e['rot_err_max'] is None else round(e['rot_err_max'],4)} | "
                     f"{e['vel_err_max']:.4g} | {e['contact_ticks_game']}/{e['contact_ticks_rust']} | "
                     f"{e['nearest_game_min']:.4g}/{e['nearest_rust_min']:.4g} | {e['acc_cmd_max_abs_err']:.4g} | "
                     f"{e['rot_cmd_max_abs_err']:.4g} |")
    lines += ["", "## Focus ids", ""]
    for eid, f in focus.items():
        lines += [f"### {eid}", "", f"First divergence: `{json.dumps(f['first_divergence'])}`", "",
                  f"Game at divergence: `{json.dumps(f['at_first_divergence_game'])}`", "",
                  f"Rust at divergence: `{json.dumps(f['at_first_divergence_rust'])}`", ""]
    lines += ["", "## Physics-phase detail (focus ids, first 80 paired ticks)", "",
              "Game `pre`/`rb_pre` are sampled around the body FixedUpdate; `rb` is the post state; `step_game` is the rb delta between paired ticks. Rust `step_rust` is the end-of-tick position delta. `contact_count`/`contact_impulse` are the game solid-contact summary; Rust PHYS_TRACE exposes stage hashes rather than per-entity impulses.", ""]
    for eid, ph in phase.items():
        fp = ph["first_step_divergence"]
        lines += [f"### {eid} phase", "", f"First step divergence: `{json.dumps(fp)}`", "",
                  "| tick | game rb | rust pos | game step | rust step | contact | game acc/rot | rust acc/rot |",
                  "|---|---|---|---|---|---|---|---|"]
        for row in ph["rows"][:20]:
            lines.append(f"| {row['tick']} | {row['game_rb']} | {row['rust']} | {row['step_game']} | "
                         f"{row['step_rust']} | {row['contact_count']}/{row['contact_impulse']} | "
                         f"{row['acc_game']}/{row['rot_game']} | {row['acc_rust']}/{row['rot_rust']} |")
    (out / "report.md").write_text("\n".join(lines), encoding="utf-8")
    print("\n".join(lines[:20]))
    print("wrote", out / "compare.json", "and", out / "report.md")


if __name__ == "__main__":
    main()

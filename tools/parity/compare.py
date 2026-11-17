#!/usr/bin/env python3
"""compare.py - diff two parity captures (game vs game, or game vs RUSTibites) system by system.

Inputs are capture directories (or .jsonl.gz files) written to the schema in docs/PARITY.md:
``ticks.jsonl.gz`` + ``events.jsonl.gz`` + ``manifest.json``.

What is compared
----------------
* meta: schema, sim_tps, gene order, settings (game captures record the effective settings), engine
  versions, tick coverage (missing ticks are reported, never silently accepted);
* population / biomass / vitals / species / generation curves: per-common-tick MAE, max error,
  relative error, exact-match fraction, first diverging tick;
* entities (at snapshot ticks present in both): ids matched by save id; exact-match counters for
  genes, brain topology, brain IO (sense/output digests) and energy/health bits; tolerance metrics
  (MAE/RMSE/max/p95) for positions and velocities;
* lifecycle events: birth/death counts, kind distribution, and - for --profile exact - identity;
* brain: mean activations of all 15 outputs (game sampler group ``behavior.outputs``).

Profiles
--------
    --profile exact    bit-identical required: every compared value must be equal (use for game-vs-game
                       determinism checks and for Rust runs that claim exact parity)
    --profile default  (default) exact systems (genes/topology/species/generation) must match 100%,
                       ecology curves within 10% relative, positions/velocities/energy reported with
                       counters but not gated (physics intentionally diverges in RUSTibites)
    --profile loose    curves within 25%, everything else informational

Exit code 0 when all gates pass, 1 when any gate fails (CI), 2 on usage/IO errors.

Self-test (no game needed):
    python tools/parity/compare.py --self-test
"""
from __future__ import annotations

import argparse
import copy
import json
import math
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import parity_common as pc  # noqa: E402

PROFILES = {
    "exact": {"curve_rel": 0.0, "curve_abs": 0.0, "require_exact_systems": True, "require_exact_curves": True,
              "pos_gate": 0.0, "vel_gate": 0.0, "energy_gate": 1.0},
    "default": {"curve_rel": 0.10, "curve_abs": 1.0, "require_exact_systems": True, "require_exact_curves": False,
                "pos_gate": None, "vel_gate": None, "energy_gate": 0.95},
    "loose": {"curve_rel": 0.25, "curve_abs": 1.0, "require_exact_systems": False, "require_exact_curves": False,
              "pos_gate": None, "vel_gate": None, "energy_gate": None},
}

EXACT_SYSTEM_FIELDS = ("genes_digest", "species", "generation", "tag", "nodes", "synapses")
CURVE_FIELDS = [
    ("counts", "bibites"), ("counts", "eggs"), ("counts", "corpses"),
    ("pellets", "plant_n"), ("pellets", "plant_biomass"), ("pellets", "meat_n"), ("pellets", "meat_biomass"),
    ("pellets", "bibite_biomass"), ("pellets", "free_biomass"),
    ("vitals", "energy_mean"), ("vitals", "health_mean"), ("vitals", "age_mean"), ("vitals", "age_max"),
    ("generation", "gen_max"), ("generation", "gen_mean"), ("species", "n"),
    ("brain", "nodes_mean"), ("brain", "nodes_max"), ("brain", "syn_mean"), ("brain", "syn_max"),
]


def _num(value):
    """Numeric curve value: declared-f32 floats are quantised to f32 bits (see pc.f32_value)."""
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    if isinstance(value, int):
        return float(value)              # counts stay exact integers, never f32-rounded
    return pc.f32_value(value)


def _f32v(value):
    """Quantise a declared-f32 entity/field value; leave ints/strings/None untouched."""
    if isinstance(value, bool) or value is None:
        return value
    if isinstance(value, float):
        return pc.f32_value(value)
    return value


def _get_curve(rec: dict, group: str, key: str):
    g = rec.get(group)
    if not isinstance(g, dict):
        return None
    return _num(g.get(key))


def _rot_deg(entity: dict, source: str | None, rust_is_radians: bool = True) -> float | None:
    """Entity rotation in degrees, normalised across capture sources.

    The game capture (and the game save format) writes `rot` in **degrees**. Rust capture writers have
    used both units over time: early writer builds emitted degrees (e.g. `testdata/parity/rust_ring`),
    the current build emits **radians** (task-81: game 353.215881 deg vs Rust 6.164780 rad for the same
    pose). `compare.py` therefore detects the Rust unit per comparison (`_detect_rust_rot_units`) or
    uses an explicit `manifest.rot_units`/`--rot-units` override, and converts to degrees here so a
    matching tick-0 pose compares as ~0 instead of ~270 deg (task-86).
    """
    v = entity.get("rot")
    if v is None:
        return None
    try:
        v = float(v)
    except (TypeError, ValueError):
        return None
    if str(source or "").lower() == "rust" and rust_is_radians:
        v = math.degrees(v)
    return v


def _detect_rust_rot_units(left, right, left_records: dict, right_records: dict,
                           common: list[int]) -> bool:
    """Return True if the Rust-side `rot` values should be treated as radians.

    Detection order: explicit `manifest.rot_units` -> auto-detect on the earliest common per-entity
    snapshot (choose the interpretation whose shortest-arc delta is smaller; a matching tick-0 pose is
    ~0 under the right one and huge under the wrong one). Falls back to radians (current writer) when
    there is nothing to compare.
    """
    for cap in (left, right):
        units = str((cap.manifest or {}).get("rot_units") or "").lower()
        if units in ("degrees", "deg", "radians", "rad"):
            return units.startswith("rad")
    rust_is_right = str(right.source or "").lower() == "rust"
    rust_is_left = str(left.source or "").lower() == "rust"
    if not (rust_is_right or rust_is_left) or str(left.source).lower() == str(right.source).lower():
        return False
    for tick in common:
        la = left_records.get(tick, {}).get("entities") or []
        rb = right_records.get(tick, {}).get("entities") or []
        ids_l = {e.get("id"): e for e in la if e.get("kind") == "bibite" and e.get("id") is not None}
        ids_r = {e.get("id"): e for e in rb if e.get("kind") == "bibite" and e.get("id") is not None}
        common_ids = sorted(set(ids_l) & set(ids_r))
        if not common_ids:
            continue
        scores = {}
        for rad in (True, False):
            deltas = []
            for eid in common_ids:
                a = _rot_deg(ids_l[eid], left.source, rust_is_radians=rad and rust_is_left)
                b = _rot_deg(ids_r[eid], right.source, rust_is_radians=rad and rust_is_right)
                if a is None or b is None:
                    continue
                deltas.append(abs(_angle_delta_deg(a, b)))
            if deltas:
                scores[rad] = sum(deltas) / len(deltas)
        if scores:
            return min(scores, key=scores.get) if scores.get(True, 1e9) <= scores.get(False, 1e9) else False
        return True
    return True


def _angle_delta_deg(a: float, b: float) -> float:
    """Signed shortest-arc difference b - a in degrees, wrapped to [-180, 180)."""
    return (b - a + 180.0) % 360.0 - 180.0


def _stats(diffs: list[float]) -> dict:
    n = len(diffs)
    if not n:
        return {"n": 0}
    absd = [abs(d) for d in diffs]
    return {"n": n, "mae": sum(absd) / n, "max": max(absd),
            "rmse": math.sqrt(sum(d * d for d in diffs) / n),
            "exact": sum(1 for d in diffs if d == 0),
            "p95": sorted(absd)[min(n - 1, int(0.95 * (n - 1)))]}


def _first_diff(common: list[int], tick_maps: list[dict], field_fn) -> int | None:
    for tick in common:
        values = [field_fn(m.get(tick)) for m in tick_maps]
        if any(v is None for v in values):
            continue
        if max(values) != min(values):
            return tick
    return None


def settings_difference(a: dict | None, b: dict | None) -> list[dict]:
    out = []
    for scope in ("scenario", "independent"):
        ea = (a or {}).get(scope) or {}
        eb = (b or {}).get(scope) or {}
        for key in sorted(set(ea) | set(eb)):
            va, vb = ea.get(key), eb.get(key)
            if isinstance(va, float) or isinstance(vb, float):
                try:
                    if abs(float(va) - float(vb)) <= 1e-6 * max(1.0, abs(float(va)), abs(float(vb))):
                        continue
                except (TypeError, ValueError):
                    pass
            if va != vb:
                out.append({"scope": scope, "key": key, "left": va, "right": vb})
    return out


def compare(left_path: str, right_path: str, options) -> dict:
    left = pc.open_capture(left_path)
    right = pc.open_capture(right_path)
    profile = PROFILES[options.profile]
    ml, mr = left.manifest, right.manifest

    left_records = {int(r["tick"]): r for r in left.records()}
    right_records = {int(r["tick"]): r for r in right.records()}
    common = sorted(set(left_records) & set(right_records))
    if options.until_tick is not None:
        common = [t for t in common if t <= options.until_tick]
    rust_rot_radians = _detect_rust_rot_units(left, right, left_records, right_records, common)
    only_left = sorted(set(left_records) - set(right_records))
    only_right = sorted(set(right_records) - set(left_records))

    checks: list[dict] = []

    def check(name: str, status: str, detail=None, gate: bool = True):
        checks.append({"name": name, "status": status, "gate": gate, "detail": detail})

    # ---- meta ---------------------------------------------------------------------------------
    # Official captures are analysis artifacts, not generic parity captures: official_game_* is the
    # per-tick game capture used by tools/parity/contact_ref.py and official_rust_* lacks the newer
    # feeding/conservation groups. When a sibling official_analysis_<ticks>.json exists, the generic
    # per-entity energy/feeding/conservation gates are informational; the dedicated official_analysis
    # check below is authoritative (contact/feeding numbers are asserted by contact_ref.py).
    official_analysis_path = None
    if str(ml.get("capture") or "").startswith("official_"):
        for cand in sorted(left.path.parent.glob("official_analysis_*.json")):
            if str(left.path.name).endswith(cand.stem.rsplit("_", 1)[-1]):
                official_analysis_path = cand
                break
        if official_analysis_path is None:
            official_analysis_path = next(iter(sorted(left.path.parent.glob("official_analysis_*.json"))), None)
    official_mode = official_analysis_path is not None
    meta = {
        "left": {"path": str(left.path), "source": left.source, "capture": ml.get("capture"),
                 "engine": ml.get("engine"), "settings_hash": ml.get("settings_hash")},
        "right": {"path": str(right.path), "source": right.source, "capture": mr.get("capture"),
                  "engine": mr.get("engine"), "settings_hash": mr.get("settings_hash")},
        "ticks_left": len(left_records), "ticks_right": len(right_records), "common_ticks": len(common),
        "only_left": only_left[:20], "only_right": only_right[:20],
        "only_left_n": len(only_left), "only_right_n": len(only_right),
        "tick_range_left": [min(left_records), max(left_records)] if left_records else None,
        "tick_range_right": [min(right_records), max(right_records)] if right_records else None,
        "sim_tps_left": left.sim_tps, "sim_tps_right": right.sim_tps,
        "gene_names_match": left.gene_names == right.gene_names,
        "settings_match": None, "settings_difference": [],
        "schema_left": ml.get("schema"), "schema_right": mr.get("schema"),
        "left_taint": ml.get("taint"), "right_taint": mr.get("taint"),
        "official_analysis": str(official_analysis_path) if official_analysis_path else None,
    }
    if left.gene_names != right.gene_names:
        check("gene_order", "fail", {"left": left.gene_names[:3], "right": right.gene_names[:3]})
    else:
        check("gene_order", "pass", len(left.gene_names))
    if left.sim_tps != right.sim_tps:
        check("sim_tps", "fail", {"left": left.sim_tps, "right": right.sim_tps})
    else:
        check("sim_tps", "pass", left.sim_tps)
    if left.manifest.get("settings") and right.manifest.get("settings"):
        diff = settings_difference(left.manifest["settings"], right.manifest["settings"])
        meta["settings_difference"] = diff
        meta["settings_match"] = len(diff) == 0
        check("settings", "pass" if not diff else ("fail" if options.strict_settings else "warn"),
              {"differences": diff[:10], "n": len(diff), "strict": options.strict_settings})
    else:
        hashes_ok = (ml.get("settings_hash") and mr.get("settings_hash")
                     and ml.get("settings_hash") == mr.get("settings_hash"))
        meta["settings_match"] = hashes_ok
        check("settings", "pass" if hashes_ok else "warn",
              {"reason": "settings_hash only (one side has no settings block)"})
    if not common:
        check("tick_overlap", "fail", "captures share no sampled tick")
    elif options.allow_missing_ticks:
        check("tick_overlap", "warn", {"common": len(common), "only_left": len(only_left), "only_right": len(only_right)})
    else:
        if options.until_tick is not None:
            expected = len(common)
        else:
            expected = min(max(left_records) - min(left_records), max(right_records) - min(right_records)) + 1 if left_records and right_records else 0
        ok = len(common) >= 0.99 * expected if expected else False
        check("tick_overlap", "pass" if ok else "fail",
              {"common": len(common), "expected": expected, "only_left": len(only_left), "only_right": len(only_right)})

    # ---- ecology curves -----------------------------------------------------------------------
    curves = {}
    for group, key in CURVE_FIELDS:
        diffs = []
        lticks = []
        for tick in common:
            a = _get_curve(left_records[tick], group, key)
            b = _get_curve(right_records[tick], group, key)
            if a is None or b is None:
                continue
            diffs.append(b - a)
            lticks.append(tick)
        st = _stats(diffs)
        st["first_difference_tick"] = _first_diff(common, [left_records, right_records],
                                                  lambda r, g=group, k=key: _get_curve(r, g, k) if r else None)
        scale = 1.0
        if diffs:
            base_vals = [abs(_get_curve(left_records[t], group, key) or 0.0) for t in common
                         if _get_curve(left_records[t], group, key) is not None]
            scale = max(1.0, sum(base_vals) / max(1, len(base_vals)))
        st["scale"] = scale
        tol = max(profile["curve_abs"], profile["curve_rel"] * scale)
        st["tolerance"] = tol
        st["status"] = ("pass" if st.get("max", 0.0) == 0.0 else
                        ("pass" if st.get("max", 1e9) <= tol and not profile["require_exact_curves"] else "fail"))
        curves[f"{group}.{key}"] = st
    curve_fails = [k for k, v in curves.items() if v["status"] == "fail"]
    curves_skipped = sorted(k for k, v in curves.items() if not v.get("n"))
    meta["curves_skipped"] = curves_skipped
    check("ecology_curves", "pass" if not curve_fails else "fail", {"failed": curve_fails[:10], "n": len(curve_fails)})
    # Missing groups/fields are reported as gaps (informational), never as failures: RUSTibites may not
    # model every sampler group yet. Emitting placeholder zeros instead of omitting a field WOULD be
    # compared and can fail, so the rule is "omit, don't fake".
    behavior_ticks = {"left": sum(1 for r in left_records.values() if r.get("behavior")),
                      "right": sum(1 for r in right_records.values() if r.get("behavior"))}
    tag_ticks = {"left": sum(1 for r in left_records.values() if r.get("tags")),
                 "right": sum(1 for r in right_records.values() if r.get("tags"))}
    metric_ticks = {"left": sum(1 for r in left_records.values() if r.get("metrics")),
                    "right": sum(1 for r in right_records.values() if r.get("metrics"))}
    entity_ticks = {"left": sum(1 for r in left_records.values() if r.get("entities")),
                    "right": sum(1 for r in right_records.values() if r.get("entities"))}
    gaps = {"curves_skipped": curves_skipped,
            "behavior_group_ticks": behavior_ticks, "tags_group_ticks": tag_ticks,
            "metrics_group_ticks": metric_ticks, "entities_ticks": entity_ticks}
    meta["gaps"] = gaps
    meta["rot_units"] = {"rust_treated_as": "radians" if rust_rot_radians else "degrees"}
    check("optional_gaps", "info", gaps, gate=False)

    # ---- entities -----------------------------------------------------------------------------
    entity = {"ticks_with_entities": 0, "bibites_compared": 0, "unmatched_left": 0, "unmatched_right": 0,
              "digests": {}, "energy": {}, "health": {}, "position": {}, "velocity": {}, "age": {},
              "brain_io": {}, "first_difference_tick": None}
    pos_diffs: list[float] = []
    vel_diffs: list[float] = []
    age_diffs: list[float] = []
    rot_diffs: list[float] = []
    energy_abs: list[float] = []
    energy_rel: list[float] = []
    health_abs: list[float] = []
    digest_exact = {"genes_digest": [0, 0], "brain_digest": [0, 0], "sense_digest": [0, 0], "output_digest": [0, 0],
                    "species": [0, 0], "generation": [0, 0], "tag": [0, 0], "nodes": [0, 0], "synapses": [0, 0]}
    energy_exact = [0, 0]
    first_entity_diff = None
    for tick in common:
        la = left_records[tick].get("entities")
        rb = right_records[tick].get("entities")
        if not la or not rb:
            continue
        entity["ticks_with_entities"] += 1
        by_id_l = {e.get("id"): e for e in la if e.get("kind") == "bibite" and e.get("id") is not None}
        by_id_r = {e.get("id"): e for e in rb if e.get("kind") == "bibite" and e.get("id") is not None}
        for eid in sorted(set(by_id_l) & set(by_id_r)):
            le, re = by_id_l[eid], by_id_r[eid]
            entity["bibites_compared"] += 1
            for f in EXACT_SYSTEM_FIELDS:
                digest_exact[f][1] += 1
                if le.get(f) == re.get(f):
                    digest_exact[f][0] += 1
                elif first_entity_diff is None:
                    first_entity_diff = {"tick": tick, "id": eid, "field": f, "left": le.get(f), "right": re.get(f)}
            if le.get("energy") is not None and re.get("energy") is not None:
                energy_exact[1] += 1
                d = _f32v(re["energy"]) - _f32v(le["energy"])
                energy_abs.append(d)
                denom = max(1e-9, abs(_f32v(le["energy"])))
                energy_rel.append(abs(d) / denom)
                if d == 0:
                    energy_exact[0] += 1
            if le.get("health") is not None and re.get("health") is not None:
                health_abs.append(_f32v(re["health"]) - _f32v(le["health"]))
            for f in ("brain_digest", "sense_digest", "output_digest"):
                digest_exact[f][1] += 1
                if le.get(f) == re.get(f):
                    digest_exact[f][0] += 1
                elif first_entity_diff is None:
                    first_entity_diff = {"tick": tick, "id": eid, "field": f, "left": le.get(f), "right": re.get(f)}
            if all(k in le and k in re for k in ("x", "y")):
                pos_diffs.append(math.hypot(_f32v(re["x"]) - _f32v(le["x"]), _f32v(re["y"]) - _f32v(le["y"])))
            if all(k in le and k in re for k in ("vx", "vy")):
                vel_diffs.append(math.hypot(_f32v(re["vx"]) - _f32v(le["vx"]), _f32v(re["vy"]) - _f32v(le["vy"])))
            if le.get("age_s") is not None and re.get("age_s") is not None:
                age_diffs.append(_f32v(re["age_s"]) - _f32v(le["age_s"]))
            rust_is_left = str(left.source).lower() == "rust" and rust_rot_radians
            rust_is_right = str(right.source).lower() == "rust" and rust_rot_radians
            rot_l = _rot_deg(le, left.source, rust_is_radians=rust_is_left)
            rot_r = _rot_deg(re, right.source, rust_is_radians=rust_is_right)
            if rot_l is not None and rot_r is not None:
                rot_diffs.append(_angle_delta_deg(rot_l, rot_r))
        entity["unmatched_left"] += len(set(by_id_l) - set(by_id_r))
        entity["unmatched_right"] += len(set(by_id_r) - set(by_id_l))
        entity["first_difference_tick"] = entity["first_difference_tick"] or first_entity_diff
    entity["digests"] = {f: {"match": v[0], "n": v[1], "frac": (v[0] / v[1] if v[1] else None)} for f, v in digest_exact.items()}
    entity["energy"] = {"exact": energy_exact[0], "n": energy_exact[1],
                        "exact_frac": (energy_exact[0] / energy_exact[1] if energy_exact[1] else None),
                        "mae": _stats(energy_abs).get("mae"), "max": _stats(energy_abs).get("max"),
                        "rel_p95": _stats(energy_rel).get("p95")}
    entity["health"] = _stats(health_abs)
    entity["position"] = _stats(pos_diffs)
    entity["velocity"] = _stats(vel_diffs)
    entity["age"] = _stats(age_diffs)
    entity["rotation"] = {**_stats([abs(d) for d in rot_diffs]), "unit": "degrees",
                          "rust_rot_treated_as": "radians" if rust_rot_radians else "degrees",
                          "note": "shortest-arc delta in degrees; Rust capture rotation units are detected per "
                                  "comparison (current writer=radians, older captures=degrees) or taken from "
                                  "manifest.rot_units, and converted to degrees before comparing",
                          "left_source": left.source, "right_source": right.source}
    entity["brain_io"] = {"sense_digest": entity["digests"].get("sense_digest"),
                          "output_digest": entity["digests"].get("output_digest"),
                          "note": "brain IO/sense = digests of the brain node runtime values at snapshot ticks"}
    if entity["bibites_compared"] == 0:
        check("entities", "warn",
              {"reason": "no common per-entity snapshot ticks with matching save ids; game captures snapshot at "
                         "tick 0 and the final tick, so a Rust capture that emits entities every tick compares at "
                         "those ticks. If the id sets are disjoint, the Rust emitter is not preserving body ids "
                         "from the base save",
               "entities_ticks": entity_ticks})
    else:
        gene = entity["digests"]["genes_digest"]
        gene_ok = gene["frac"] == 1.0
        check("entity_genes", "pass" if gene_ok else "fail",
              {"matched": gene["match"], "n": gene["n"]})
        brain = entity["digests"]["brain_digest"]
        if profile["require_exact_systems"]:
            check("entity_brain", "pass" if brain["frac"] == 1.0 else ("fail" if options.profile == "exact" else "warn"),
                  {"matched": brain["match"], "n": brain["n"]})
        else:
            check("entity_brain", "info", {"matched": brain["match"], "n": brain["n"]}, gate=False)
        # gates that only exist in some profiles
        if profile["energy_gate"] is not None and not official_mode:
            frac = entity["energy"]["exact_frac"] or 0.0
            check("entity_energy", "pass" if frac >= profile["energy_gate"] else "fail",
                  {"exact_frac": frac, "min": profile["energy_gate"]})
        elif official_mode:
            check("entity_energy", "info",
                  {"exact_frac": entity["energy"]["exact_frac"], "excluded": "official capture/analysis schema"},
                  gate=False)
        else:
            check("entity_energy", "info", {"exact_frac": entity["energy"]["exact_frac"]}, gate=False)
        if profile["pos_gate"] is not None:
            ok = entity["position"].get("max", 1e30) <= profile["pos_gate"] if entity["position"].get("n") else False
            check("entity_position", "pass" if ok else "fail",
                  {"position_max": entity["position"].get("max"), "max_allowed": profile["pos_gate"]})
        else:
            check("entity_position", "info", {"mae": entity["position"].get("mae"),
                                              "max": entity["position"].get("max")}, gate=False)
        if profile["vel_gate"] is not None:
            ok = entity["velocity"].get("max", 1e30) <= profile["vel_gate"] if entity["velocity"].get("n") else False
            check("entity_velocity", "pass" if ok else "fail",
                  {"velocity_max": entity["velocity"].get("max"), "max_allowed": profile["vel_gate"]})
        else:
            check("entity_velocity", "info", {"mae": entity["velocity"].get("mae"),
                                              "max": entity["velocity"].get("max")}, gate=False)

    if entity.get("rotation", {}).get("n"):
        rot = entity["rotation"]
        check("entity_rotation", "info",
              {"mae_deg": rot.get("mae"), "max_deg": rot.get("max"), "n": rot.get("n"),
               "left_source": rot.get("left_source"), "right_source": rot.get("right_source")}, gate=False)

    # ---- behavior (brain output means) --------------------------------------------------------
    behavior = {}
    for name in ("speed_mean", "speed_max", "moving_frac", "travel_rate_mean", "bite_rate_mean",
                 "damage_dealt_rate_mean", "egg_rate_mean"):
        diffs = []
        for tick in common:
            a = _get_curve(left_records[tick], "behavior", name)
            b = _get_curve(right_records[tick], "behavior", name)
            if a is not None and b is not None:
                diffs.append(b - a)
        behavior[name] = _stats(diffs)
    outputs = {}
    out_names = set()
    for tick in common:
        ro = (left_records[tick].get("behavior") or {}).get("outputs") or {}
        out_names.update(ro.keys())
    for name in sorted(out_names):
        diffs = []
        for tick in common:
            a = _num(((left_records[tick].get("behavior") or {}).get("outputs") or {}).get(name))
            b = _num(((right_records[tick].get("behavior") or {}).get("outputs") or {}).get(name))
            if a is not None and b is not None:
                diffs.append(b - a)
        outputs[name] = _stats(diffs)
    brain = {"outputs": outputs}
    values_n = sum(v.get("n", 0) for v in outputs.values())
    values_exact = sum(v.get("exact", 0) for v in outputs.values())
    non_exact_outputs = {k: {"exact": v.get("exact", 0), "n": v.get("n", 0), "max": v.get("max")}
                         for k, v in outputs.items() if v.get("n") and v.get("exact") != v.get("n")}
    check("brain_outputs", "info",
          {"nodes": len(outputs), "values_exact": values_exact, "values_n": values_n,
           "all_exact": bool(outputs) and values_exact == values_n,
           "non_exact_nodes": sorted(non_exact_outputs),
           "per_output": {k: {"exact": v.get("exact"), "n": v.get("n"), "max": v.get("max")}
                          for k, v in outputs.items()}},
          gate=False)

    # ---- events --------------------------------------------------------------------------------
    events = {"left": 0, "right": 0, "kinds_left": {}, "kinds_right": {}, "identity_matches": None}
    try:
        le = list(left.events())
        re = list(right.events())
        events["left"], events["right"] = len(le), len(re)
        for e in le:
            events["kinds_left"][e.get("kind")] = events["kinds_left"].get(e.get("kind"), 0) + 1
        for e in re:
            events["kinds_right"][e.get("kind")] = events["kinds_right"].get(e.get("kind"), 0) + 1
        if le and re:
            key = lambda e: (e.get("tick"), e.get("kind"), e.get("id"))
            lset, rset = set(map(key, le)), set(map(key, re))
            events["identity_matches"] = len(lset & rset)
            events["identity_total"] = len(lset | rset)
    except Exception as exc:  # noqa: BLE001
        events["error"] = str(exc)
    if events["left"] or events["right"]:
        if profile["require_exact_curves"]:
            ok = events["left"] == events["right"] and events.get("identity_matches") == events.get("identity_total")
            check("events", "pass" if ok else "fail", events)
        else:
            check("events", "info", {"left": events["left"], "right": events["right"],
                                     "identity_matches": events.get("identity_matches")}, gate=False)

    # ---- feeding / conservation (optional; FEEDING_METRICS section 7) -------------------------
    left_feeding = _feeding_stats(left.path)
    right_feeding = _feeding_stats(right.path)
    feeding = {"left": left_feeding, "right": right_feeding}
    for fc in _feeding_checks(left_feeding, right_feeding, profile):
        if official_mode:
            check(fc["name"], "info", {**fc["detail"], "excluded": "official capture/analysis schema"}, gate=False)
        else:
            check(fc["name"], fc["status"], fc["detail"], gate=fc.get("gate", False))
    conservation = _conservation_section(left_feeding, right_feeding)
    cons_status = conservation.pop("status", "info")
    cons_gate = bool(conservation.pop("gated", False))
    if official_mode:
        check("conservation", "info", {**conservation, "excluded": "official capture/analysis schema"}, gate=False)
    else:
        check("conservation", cons_status, conservation, gate=cons_gate)
    # Dedicated official-analysis gate: the contact/feeding assertions are in contact_ref.py; generic
    # per-entity/feeding/conservation checks above are not applicable to the analysis capture shape.
    if official_analysis_path is not None:
        try:
            analysis = json.loads(official_analysis_path.read_text(encoding="utf-8"))
            ok = analysis.get("schema") == "rustibites.parity.contact_ref.v1"
            check("official_analysis", "pass" if ok else "fail",
                  {"path": str(official_analysis_path), "schema": analysis.get("schema"),
                   "ticks": analysis.get("ticks"), "game": (analysis.get("game") or {}).get("path"),
                   "game_fed_bibites": (analysis.get("game") or {}).get("fed_bibites"),
                   "game_bite_events": (analysis.get("game") or {}).get("bite_events"),
                   "rust_fed_bibites": (analysis.get("rust") or {}).get("fed_bibites"),
                   "rust_bite_events": (analysis.get("rust") or {}).get("bite_events"),
                   "note": "official_analysis is a contact/feeding assertion artifact; generic parity energy/feeding/"
                           "conservation gates are excluded for this capture shape."})
        except Exception as exc:  # noqa: BLE001
            check("official_analysis", "fail", {"path": str(official_analysis_path), "error": str(exc)})

    # ---- overall ------------------------------------------------------------------------------
    failures = [c["name"] for c in checks if c["gate"] and c["status"] == "fail"]
    warnings = [c["name"] for c in checks if c["gate"] and c["status"] == "warn"]
    result = {
        "schema": pc.COMPARE_SCHEMA,
        "profile": options.profile,
        "overall": "fail" if failures else ("warn" if warnings else "pass"),
        "exit_code": 1 if failures else 0,
        "failures": failures,
        "warnings": warnings,
        "checks": checks,
        "meta": meta,
        "systems": {"ecology": curves, "entities": entity, "behavior": behavior, "brain": brain,
                    "events": events, "feeding": feeding, "conservation": conservation},
    }
    return result


# -------------------------------------------------------------------------------------------------
# optional feeding / conservation sections (task-98; FEEDING_METRICS section 7)
# -------------------------------------------------------------------------------------------------
FEEDING_RATE_KEYS = (
    ("plant_bites_per_live_s", "bites/live-s", 1e-3),
    ("meat_bites_per_live_s", "bites/live-s", 1e-3),
    ("plant_consumed_u2_per_live_s", "u2/live-s", 1e-3),
    ("meat_consumed_u2_per_live_s", "u2/live-s", 1e-3),
    ("plant_consumed_e_per_live_s", "E/live-s", 1e-2),
    ("meat_consumed_e_per_live_s", "E/live-s", 1e-2),
    ("stomach_energy_gain_e_per_live_s", "E/live-s", 1e-2),
)


def _read_feeding_stream(capture_path) -> list[dict]:
    import gzip as _gzip
    from pathlib import Path as _Path
    p = _Path(capture_path)
    if p.is_file():
        cands = [p]
    else:
        cands = [p / "feeding.jsonl.gz", p / "feeding.jsonl"]
    for c in cands:
        if not c.exists():
            continue
        try:
            if c.suffix == ".gz":
                with _gzip.open(c, "rt", encoding="utf-8") as fh:
                    return [json.loads(line) for line in fh if line.strip()]
            return [json.loads(line) for line in c.read_text(encoding="utf-8").splitlines() if line.strip()]
        except Exception:
            return []
    return []


def _feeding_stats(capture_path) -> dict | None:
    """Normalised feeding stats from a capture's feeding stream (game stomach-delta proxy or Rust counters)."""
    rows = _read_feeding_stream(capture_path)
    if not rows:
        return None
    last = rows[-1]
    counters = last.get("counters") or {}
    if "live_seconds" in counters and "plant_bite_events" in counters:
        first = rows[0].get("counters") or {}
        live = float(counters.get("live_seconds") or 0) - float(first.get("live_seconds") or 0)
        def d(k):
            try:
                return float(counters.get(k) or 0) - float(first.get(k) or 0)
            except Exception:
                return None
        plant_e = d("plant_consumed_u2") or 0.0
        meat_e = d("meat_consumed_u2") or 0.0
        dens = {}
        for name, props in (last.get("material_properties") or {}).items():
            n = str(name).lower()
            if "plant" in n or "meat" in n:
                try:
                    dens["plant" if "plant" in n else "meat"] = float(props.get("energy_density") or 0)
                except Exception:
                    pass
        out = {"evidence": "counters", "live_seconds": live,
               "plant_bites_per_live_s": (d("plant_bite_events") or 0) / live if live > 0 else None,
               "meat_bites_per_live_s": (d("meat_bite_events") or 0) / live if live > 0 else None,
               "plant_consumed_u2_per_live_s": plant_e / live if live > 0 else None,
               "meat_consumed_u2_per_live_s": meat_e / live if live > 0 else None,
               "plant_consumed_e_per_live_s": plant_e * dens.get("plant", 0.0) / live if live > 0 else None,
               "meat_consumed_e_per_live_s": meat_e * dens.get("meat", 0.0) / live if live > 0 else None,
               "stomach_energy_gain_e_per_live_s": (d("stomach_energy_gain_e") or 0) / live if live > 0 else None}
    else:
        # game capture: derive intake from positive stomach-material deltas (polling proxy)
        prev, prev_t, prev_pop = {}, None, None
        amounts = {"plant": 0.0, "meat": 0.0}
        events_n = {"plant": 0, "meat": 0}
        dens = {}
        live = 0.0
        for snap in rows:
            t = float(snap.get("t") or 0)
            for name, props in (snap.get("material_properties") or {}).items():
                n = str(name).lower()
                if "plant" in n or "meat" in n:
                    try:
                        dens["plant" if "plant" in n else "meat"] = float(props.get("energy_density") or 0)
                    except Exception:
                        pass
            pop = len(snap.get("bibites") or [])
            if prev_t is not None:
                live += 0.5 * (pop + prev_pop) * max(0.0, t - prev_t)
            cur = {}
            for b in snap.get("bibites") or []:
                cur[b.get("id")] = {k: float(v or 0) for k, v in (b.get("stomach_materials") or {}).items()}
            for bid, cm in cur.items():
                pm = prev.get(bid, {})
                for name, v in cm.items():
                    delta = v - pm.get(name, 0.0)
                    n = str(name).lower()
                    cat = "plant" if "plant" in n else ("meat" if "meat" in n else None)
                    if delta > 1e-6 and cat:
                        amounts[cat] += delta
                        events_n[cat] += 1
            prev, prev_t, prev_pop = cur, t, pop
        out = {"evidence": "polling_proxy", "live_seconds": live,
               "plant_bites_per_live_s": events_n["plant"] / live if live > 0 else None,
               "meat_bites_per_live_s": events_n["meat"] / live if live > 0 else None,
               "plant_consumed_u2_per_live_s": amounts["plant"] / live if live > 0 else None,
               "meat_consumed_u2_per_live_s": amounts["meat"] / live if live > 0 else None,
               "plant_consumed_e_per_live_s": amounts["plant"] * dens.get("plant", 0.0) / live if live > 0 else None,
               "meat_consumed_e_per_live_s": amounts["meat"] * dens.get("meat", 0.0) / live if live > 0 else None,
               "stomach_energy_gain_e_per_live_s": (amounts["plant"] * dens.get("plant", 0.0)
                                                     + amounts["meat"] * dens.get("meat", 0.0)) / live if live > 0 else None}
    out["raw_last_counters"] = counters
    out["raw_last_sample"] = last
    return out


def _feeding_checks(left: dict | None, right: dict | None, profile: dict) -> list[dict]:
    """Per FEEDING_METRICS section 7: gate derived rates/consumed at 10% default / 25% loose, with
    absolute floors near zero; unknown fields ignored; bite_rate_mean is not in this list."""
    checks = []
    if not left or not right:
        return [{"name": "feeding", "status": "info", "gate": False,
                 "detail": {"reason": "missing feeding stream on one side"}}]
    tol = profile.get("curve_rel", 0.10)
    per_key = {}
    failed = []
    for key, unit, floor in FEEDING_RATE_KEYS:
        a, b = left.get(key), right.get(key)
        if a is None or b is None:
            per_key[key] = {"status": "info", "left": a, "right": b, "unit": unit}
            continue
        a, b = float(a), float(b)
        if abs(a) < floor and abs(b) < floor:
            per_key[key] = {"status": "pass", "left": a, "right": b, "unit": unit, "reason": "below absolute floor"}
            continue
        rel = abs(b - a) / max(1e-9, max(abs(a), abs(b)))
        status = "pass" if rel <= tol else "fail"
        if status == "fail":
            failed.append(key)
        per_key[key] = {"status": status, "left": a, "right": b, "unit": unit, "rel": rel, "tolerance": tol}
    checks.append({"name": "feeding_rates", "status": ("fail" if failed else "pass") if per_key else "info",
                   "gate": bool(per_key) and not failed is None, "detail": {"per_key": per_key, "failed": failed,
                                                                            "tolerance": tol,
                                                                            "left_evidence": left.get("evidence"),
                                                                            "right_evidence": right.get("evidence")}})
    return checks


def _conservation_section(left: dict | None, right: dict | None) -> dict:
    """Conservation gate (FEEDING_METRICS section 7), per-side evidence.

    Game side: caller-path Harmony exact terms (`evidence == harmony_exact`), identity
    initial+produced = consumed+recycled+decayed+culled+stock.
    Rust side: task-99 exact engine counters in the feeding sample (`conservation` object,
    `evidence == counters`), identity seeded+produced = consumed+recycled+decayed+culled+removed+stock.
    Gate each exact side: |residual| <= 1 E or |relative residual| <= 1%. Sides without exact terms stay
    informational; `terms_missing`/`missing_terms` are always shown.
    """
    out = {"gated": False, "status": "info", "left": {}, "right": {},
           "policy": {"relative_tolerance": 0.01, "absolute_energy_floor": 1.0,
                      "requires": "exact conservation terms on the gated side: game evidence="
                                  "harmony_exact, Rust evidence=counters",
                      "documented_gap": "Default-3000 game residual 0.246% from unhooked cull/decay paths; "
                                        "must pass under 1% (a meat/decay-world capture will close it)"},
           "note": "game caller-path Harmony hooks: SpawnAPellet=produced, BibiteMouth.MaybeAttack=consumed, "
                   "RecyclePellets=recycled, MatterDecayProcessor.FixedUpdate=decayed; culled_energy still 0 "
                   "and appears as residual. Rust task-99 conservation terms are exact engine counters."}
    gated_sides = []
    for side, st in (("left", left), ("right", right)):
        if not st:
            continue
        counters = st.get("raw_last_counters") or {}
        exact = {k: counters.get(k) for k in
                 ("initial_energy", "produced_energy_exact", "consumed_energy_exact",
                  "recycled_energy_exact", "decayed_energy_exact", "culled_energy_exact",
                  "current_pellet_energy", "conservation_residual_energy") if k in counters}
        game_evidence = counters.get("evidence_exact")
        entry = {"evidence": game_evidence or st.get("evidence"), "exact": exact,
                 "missing_terms": (counters.get("missing_terms") or [])}
        rust_c = ((st.get("raw_last_sample") or {}).get("conservation") or {})
        if game_evidence == "harmony_exact" and exact.get("initial_energy") is not None:
            lhs = float(exact.get("initial_energy") or 0) + float(exact.get("produced_energy_exact") or 0)
            rhs = (float(exact.get("consumed_energy_exact") or 0) + float(exact.get("recycled_energy_exact") or 0)
                   + float(exact.get("decayed_energy_exact") or 0) + float(exact.get("culled_energy_exact") or 0)
                   + float(exact.get("current_pellet_energy") or 0))
            residual = lhs - rhs
            rel = residual / max(1e-9, abs(lhs))
            passed = abs(residual) <= 1.0 or abs(rel) <= 0.01
            entry.update({"evidence": "harmony_exact", "residual_energy": residual, "relative_residual": rel,
                          "passed": passed, "gated": True})
            gated_sides.append(side)
            out["gated"] = True
        elif rust_c:
            def g(*names):
                return sum(float(rust_c.get(n) or 0) for n in names)
            lhs = g("seeded_plant_energy", "seeded_meat_energy", "produced_plant_energy", "produced_meat_energy")
            rhs = g("consumed_plant_energy", "consumed_meat_energy", "recycled_plant_energy", "recycled_meat_energy",
                    "decayed_plant_energy", "decayed_meat_energy", "culled_plant_energy", "culled_meat_energy",
                    "removed_plant_energy", "removed_meat_energy", "stock_plant_energy", "stock_meat_energy")
            residual = lhs - rhs
            rel = residual / max(1e-9, abs(lhs))
            missing = bool(rust_c.get("terms_missing"))
            passed = abs(residual) <= 1.0 or abs(rel) <= 0.01
            entry.update({"evidence": "counters", "terms": rust_c, "residual_energy": residual,
                          "relative_residual": rel, "terms_missing": missing,
                          "passed": passed, "gated": True,
                          "note": "Rust task-99 conservation object; terms_missing is reported but the "
                                  "plant identity still gateable (Default has no meat terms)"})
            gated_sides.append(side)
            out["gated"] = True
        else:
            entry.update({"gated": False,
                          "note": "no exact conservation terms on this side (game needs harmony_exact, "
                                  "Rust needs a task-99 conservation object in the feeding sample)"})
        out[side] = entry
    if gated_sides:
        failed = any(not out[s].get("passed") for s in gated_sides)
        out["status"] = "fail" if failed else "pass"
        out["gated_sides"] = gated_sides
    return out


def print_human(result: dict) -> None:
    m = result["meta"]
    print("=" * 96)
    print(f"parity compare  [{result['profile']}]  {m['left']['path']}  vs  {m['right']['path']}")
    print("=" * 96)
    print(f"sources: {m['left']['source']} vs {m['right']['source']}   "
          f"ticks: {m['ticks_left']} vs {m['ticks_right']} (common {m['common_ticks']}, "
          f"only L {m['only_left_n']} / R {m['only_right_n']})")
    print(f"sim_tps: {m['sim_tps_left']} vs {m['sim_tps_right']}   settings_match: {m['settings_match']}")
    for eng, side in (("left", m["left"]["engine"]), ("right", m["right"]["engine"])):
        if side:
            print(f"engine[{eng}]: game {side.get('game_version')} bridge {side.get('bridge_version')} unity {side.get('unity_version')}")
    print("-" * 96)
    for c in result["checks"]:
        status = c["status"].upper().rjust(5)
        gate = "" if c["gate"] else " (info)"
        print(f"  {status}{gate}  {c['name']}")
    print("-" * 96)
    curves = result["systems"]["ecology"]
    worst = sorted(curves.items(), key=lambda kv: kv[1].get("max", 0.0), reverse=True)[:8]
    print("ecology curves (largest max error):")
    for name, st in worst:
        print(f"  {name:28s} mae={st.get('mae', 0):.4g} max={st.get('max', 0):.4g} "
              f"exact={st.get('exact', 0)}/{st.get('n', 0)} tol={st.get('tolerance', 0):.4g} [{st['status']}]")
    ent = result["systems"]["entities"]
    print(f"entities: {ent['bibites_compared']} bibite pairs on {ent['ticks_with_entities']} snapshot ticks "
          f"(unmatched L/R {ent['unmatched_left']}/{ent['unmatched_right']})")
    for name, st in ent["digests"].items():
        print(f"  {name:16s} exact {st['match']}/{st['n']}" + (f" = {st['frac']:.4f}" if st.get("frac") is not None else ""))
    rot = ent.get("rotation") or {}
    if rot.get("n"):
        print(f"  rotation         mae={rot.get('mae', 0):.4g} deg max={rot.get('max', 0):.4g} "
              f"n={rot.get('n')} (Rust radians normalised to degrees, shortest arc)")
    for name in ("energy", "position", "velocity", "age", "health"):
        st = ent[name]
        if not st or not st.get("n"):
            continue
        if name == "energy":
            print(f"  energy           exact {st.get('exact')}/{st.get('n')} mae={st.get('mae', 0):.4g} "
                  f"rel_p95={st.get('rel_p95', 0):.3g}")
        else:
            print(f"  {name:16s} mae={st.get('mae', 0):.4g} rmse={st.get('rmse', 0):.4g} max={st.get('max', 0):.4g} "
                  f"p95={st.get('p95', 0):.4g}")
    outs = result["systems"]["brain"]["outputs"]
    if outs:
        total_n = sum(v.get("n", 0) for v in outs.values())
        total_exact = sum(v.get("exact", 0) for v in outs.values())
        non_exact = {k: f"{v.get('exact', 0)}/{v.get('n', 0)}" for k, v in outs.items()
                     if v.get("n") and v.get("exact") != v.get("n")}
        if non_exact:
            print(f"brain outputs: {total_exact}/{total_n} values bit-exact over {len(outs)} nodes; "
                  f"non-exact nodes: " + ", ".join(f"{k} {v}" for k, v in sorted(non_exact.items())))
        else:
            print(f"brain outputs: ALL {total_exact}/{total_n} values bit-exact over {len(outs)} nodes")
    ev = result["systems"]["events"]
    print(f"events: left {ev['left']} right {ev['right']} identity {ev.get('identity_matches')}/{ev.get('identity_total')}")
    print("-" * 96)
    print(f"OVERALL: {result['overall'].upper()}  exit={result['exit_code']}")
    if result["failures"]:
        print("failed gates: " + ", ".join(result["failures"]))
    if result["warnings"]:
        print("warnings: " + ", ".join(result["warnings"]))


# -------------------------------------------------------------------------------------------------
# self-test
# -------------------------------------------------------------------------------------------------
def synth_capture(path: Path, *, mutate: bool = False, ticks: int = 8, plain: bool = False,
                  f32_style: bool = False, rot_radians: bool = False) -> None:
    path.mkdir(parents=True, exist_ok=True)
    nb = 4
    records = []
    entities = []
    for i in range(nb):
        entities.append({"kind": "bibite", "id": 1000 + i, "species": 1, "generation": 0, "tag": "",
                         "x": 10.13 * i, "y": -5.07 * i, "vx": 1.0, "vy": 2.0,
                         "rot": (math.radians(30.0) if rot_radians else 30.0),
                         "energy": 100.0 + i, "max_energy": 150.0, "health": 90.0, "max_health": 100.0,
                         "age_s": 1.0 + i, "size": 1.2, "nodes": 48, "synapses": 3,
                         "genes_digest": f"{i:016x}", "sense_digest": f"{i + 10:016x}",
                         "output_digest": f"{i + 20:016x}", "brain_digest": f"{i + 30:016x}"})
    for t in range(ticks):
        rec = {"schema": pc.TICK_SCHEMA, "source": "game", "tick": t, "t": t / 30.0,
               "sim_tps": 30, "brain_tps": 2, "settings_hash": "0" * 64,
               "counts": {"bibites": nb, "eggs": 0, "corpses": 0},
               "pellets": {"plant_n": 100, "plant_biomass": 500.13, "meat_n": 0, "meat_biomass": 0.0,
                           "free_biomass": 0.0, "bibite_biomass": 400.37},
               "vitals": {"energy_mean": 101.53, "health_mean": 90.07, "age_mean": 1.53, "age_max": 4.07},
               "generation": {"gen_max": 0, "gen_mean": 0.0},
               "species": {"n": 1, "counts": {"1": nb}},
               "brain": {"nodes_mean": 48.0, "nodes_max": 48, "syn_mean": 3.07, "syn_max": 3},
               "genes": {"mean": [0.0] * len(pc.GENE_NAMES), "std": [0.0] * len(pc.GENE_NAMES)},
               "lifecycle": {"births": 0, "deaths": 0},
               "behavior": {"speed_mean": 2.03, "speed_max": 3.07, "moving_frac": 1.0,
                            "travel_rate_mean": 0.13, "bite_rate_mean": 0.0,
                            "damage_dealt_rate_mean": 0.0, "egg_rate_mean": 0.0,
                            "outputs": {"Accelerate": 0.13, "Rotate": 0.0}},
               "entities": copy.deepcopy(entities)}
        if mutate and t == 3:
            rec["vitals"]["energy_mean"] += 1.5
            rec["entities"][0]["energy"] += 2.0
            rec["entities"][1]["genes_digest"] = "f" * 16
            rec["behavior"]["outputs"]["Accelerate"] += 0.5      # exercises brain-output exact counts
        records.append(rec)
    if f32_style:
        # RUSTibites writes declared-f32 values widened to f64 exactly. The game writes .NET floats
        # with shortest-round-trip decimals, so the same logical value parses to a slightly different
        # double; compare.py must quantise both sides to f32 bits.
        def widen(obj):
            if isinstance(obj, dict):
                return {k: widen(v) for k, v in obj.items()}
            if isinstance(obj, list):
                return [widen(v) for v in obj]
            if isinstance(obj, float):
                return pc.f32_value(obj)
            return obj
        records = [widen(r) for r in records]
    manifest = {"schema": pc.CAPTURE_SCHEMA, "source": ("rust" if rot_radians else "game"),
                "capture": path.name,
                "engine": {"game_version": "0.6.3.1", "bridge_version": "0.7.0"}, "gene_names": pc.GENE_NAMES,
                "settings": {"scenario": {"x": 1.0}, "independent": {"simTPS": 30}}, "settings_hash": "0" * 64,
                "sim": {"sim_tps": 30, "brain_tps": 2}}
    if plain:
        # the RUSTibites writer emits plain UTF-8 JSONL; the reader must accept it side by side with
        # the gzipped game captures (parity_common._read_jsonl_gz dispatches on the .gz suffix).
        with open(path / "ticks.jsonl", "w", encoding="utf-8") as fh:
            for rec in records:
                fh.write(json.dumps(rec, separators=(",", ":"), ensure_ascii=False))
                fh.write("\n")
        # sibling manifest for the bare-file path test (open_capture(<...>/ticks.jsonl))
        (path / "ticks.manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    else:
        pc.write_jsonl_gz(path / "ticks.jsonl.gz", records)
    (path / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")


def compare_result(left: str, right: str, profile: str = "default", strict_settings: bool = False) -> dict:
    class O:
        pass
    o = O()
    o.profile = profile
    o.strict_settings = strict_settings
    o.allow_missing_ticks = False
    o.until_tick = None
    return compare(left, right, o)


def try_compare(left: str, right: str, profile: str = "default", strict_settings: bool = False) -> int:
    return compare_result(left, right, profile, strict_settings)["exit_code"]


def self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="rustibites-parity-selftest-") as td:
        root = Path(td)
        a, b, c = root / "a", root / "b", root / "c"
        synth_capture(a)
        synth_capture(b)
        synth_capture(c, mutate=True)
        # plain (non-gzip) captures: what the RUSTibites writer emits
        plain = root / "plain"
        plain_mut = root / "plain_mut"
        synth_capture(plain, plain=True)
        synth_capture(plain_mut, plain=True, mutate=True)
        # f32-widened style: what RUSTibites writes for declared-f32 scalars
        f32_rust = root / "f32_rust"
        f32_rust_mut = root / "f32_rust_mut"
        synth_capture(f32_rust, plain=True, f32_style=True)
        synth_capture(f32_rust_mut, plain=True, f32_style=True, mutate=True)
        print("[self-test] identical captures ->", try_compare(a, b, "exact"))
        identical = try_compare(a, b, "exact")
        detected = try_compare(a, c, "exact")
        cross_format = try_compare(a, plain, "exact")                 # gzip game vs plain rust
        plain_mutation = try_compare(plain, plain_mut, "exact")
        f32_cross = try_compare(a, f32_rust, "exact")                 # shortest-decimal vs f32-widened
        f32_mutation = try_compare(a, f32_rust_mut, "exact")
        # Rust capture `rot` is radians, game captures are degrees: identical poses must compare as ~0.
        rot_rust = root / "rot_rust"
        synth_capture(rot_rust, plain=True, f32_style=True, rot_radians=True)
        rot_res = compare_result(a, rot_rust, "default")
        rot_mae = (rot_res["systems"]["entities"].get("rotation") or {}).get("mae")
        # older Rust captures wrote degrees: the auto-detection must NOT convert those
        rot_rust_deg = root / "rot_rust_deg"
        synth_capture(rot_rust_deg, plain=True, f32_style=True)
        mf = rot_rust_deg / "manifest.json"
        md = json.loads(mf.read_text(encoding="utf-8"))
        md["source"] = "rust"
        mf.write_text(json.dumps(md), encoding="utf-8")
        rot_deg_res = compare_result(a, rot_rust_deg, "default")
        rot_deg_mae = (rot_deg_res["systems"]["entities"].get("rotation") or {}).get("mae")
        rot_deg_units = (rot_deg_res["systems"]["entities"].get("rotation") or {}).get("rust_rot_treated_as")
        base_res = compare_result(a, plain, "default")
        mut_res = compare_result(a, c, "default")
        f32_res = compare_result(a, f32_rust, "exact")
        base_bo = next(ch["detail"] for ch in base_res["checks"] if ch["name"] == "brain_outputs")
        mut_bo = next(ch["detail"] for ch in mut_res["checks"] if ch["name"] == "brain_outputs")
        brain_counts_ok = (base_bo["all_exact"] is True and base_bo["values_n"] == len(base_bo["per_output"]) * 8
                           and set(base_bo["per_output"]) and all(v["exact"] == v["n"] for v in base_bo["per_output"].values()))
        brain_mut_visible = (mut_bo["all_exact"] is False and "Accelerate" in mut_bo["non_exact_nodes"]
                             and mut_bo["per_output"]["Accelerate"]["exact"] < mut_bo["per_output"]["Accelerate"]["n"])
        behavior_n = base_res["systems"]["behavior"].get("speed_mean", {}).get("n", 0)
        behavior_out_n = len(base_res["systems"]["brain"]["outputs"])
        f32_energy_frac = f32_res["systems"]["entities"]["energy"].get("exact_frac")
        f32_pos_mae = f32_res["systems"]["entities"]["position"].get("mae")
        f32_changes_formatting = pc.f32_value(154.960068) != 154.960068
        try:
            plain_capture = pc.open_capture(plain)
            plain_records = list(plain_capture.records())
            plain_manifest_ok = plain_capture.manifest.get("schema") == pc.CAPTURE_SCHEMA
            plain_ok = len(plain_records) == 8 and plain_manifest_ok
        except Exception:
            plain_records, plain_ok = [], False
        try:
            bare = pc.open_capture(plain / "ticks.jsonl")            # bare stream + <stem>.manifest.json
            bare_records = list(bare.records())
            bare_manifest_ok = bare.manifest.get("schema") == pc.CAPTURE_SCHEMA
            bare_ok = len(bare_records) == 8 and bare_manifest_ok
            bare_compare = try_compare(a, plain / "ticks.jsonl", "exact") == 0
        except Exception:
            bare_records, bare_ok, bare_compare = [], False, False
        same_manifest = pc.record_digest(  # digest stability
            json.loads(gzip_first_line(a / "ticks.jsonl.gz")))
        base_feed = {"plant_bites_per_live_s": 1.0, "meat_bites_per_live_s": 0.0,
                     "plant_consumed_u2_per_live_s": 10.0, "meat_consumed_u2_per_live_s": 0.0,
                     "plant_consumed_e_per_live_s": 50.0, "meat_consumed_e_per_live_s": 0.0,
                     "stomach_energy_gain_e_per_live_s": 50.0, "evidence": "test"}
        off10 = dict(base_feed, plant_bites_per_live_s=1.1, plant_consumed_u2_per_live_s=11.0,
                     plant_consumed_e_per_live_s=55.0, stomach_energy_gain_e_per_live_s=55.0)
        off20 = dict(base_feed, plant_bites_per_live_s=1.2, plant_consumed_u2_per_live_s=12.0,
                     plant_consumed_e_per_live_s=60.0, stomach_energy_gain_e_per_live_s=60.0)
        near = {"plant_bites_per_live_s": 0.0, "plant_consumed_u2_per_live_s": 0.0,
                "plant_consumed_e_per_live_s": 0.0}
        near2 = {"plant_bites_per_live_s": 1e-5, "plant_consumed_u2_per_live_s": 1e-5,
                 "plant_consumed_e_per_live_s": 1e-4}
        feed_eq = _feeding_checks(base_feed, dict(base_feed), PROFILES["default"])
        feed_off10 = _feeding_checks(base_feed, off10, PROFILES["default"])
        feed_off20_default = _feeding_checks(base_feed, off20, PROFILES["default"])
        feed_off20_loose = _feeding_checks(base_feed, off20, PROFILES["loose"])
        feed_near = _feeding_checks(near, near2, PROFILES["default"])
        feed_unknown = _feeding_checks(dict(base_feed, unknown_field=123), dict(base_feed), PROFILES["default"])
        checks = [
            ("identical exit 0", identical == 0),
            ("mutation detected exit 1", detected == 1),
            ("record digest stable", same_manifest == pc.record_digest(json.loads(gzip_first_line(a / "ticks.jsonl.gz")))),
            ("schema constants", pc.TICK_SCHEMA.endswith(".v1") and len(pc.GENE_NAMES) == 35),
            ("settings hash deterministic", pc.settings_hash({"independent": {"simTPS": 30}}) != pc.settings_hash({"independent": {"simTPS": 31}})),
            ("plain JSONL capture opens (open_capture)", plain_ok),
            ("gzip game vs plain rust compare exit 0", cross_format == 0),
            ("plain mutation detected exit 1", plain_mutation == 1),
            ("bare .jsonl + sibling manifest opens/compares", bare_ok and bare_compare),
            ("behavior curves compared (n = ticks)", behavior_n == 8),
            ("behavior output means compared", behavior_out_n > 0),
            ("f32-widened rust capture compares exit 0", f32_cross == 0),
            ("f32-format mutation still detected exit 1", f32_mutation == 1),
            ("f32 quantiser changes decimal formatting", f32_changes_formatting),
            ("f32 entity energy bit-exact", f32_energy_frac == 1.0),
            ("f32 entity position mae == 0", f32_pos_mae == 0),
        ("rotation normalised: Rust radians vs game degrees identical pose -> mae ~0",
         rot_mae is not None and rot_mae < 1e-3),
        ("rotation auto-detect keeps older Rust-degree captures unchanged (mae ~0)",
         rot_deg_mae is not None and rot_deg_mae < 1e-3 and rot_deg_units == "degrees"),
            ("brain per-output exact counts reported (all exact)", brain_counts_ok),
            ("brain per-output counts expose a mutated node", brain_mut_visible),
            ("feeding gate: identical stats pass",
             all(c["status"] == "pass" for c in feed_eq)),
            ("feeding gate: 10% difference passes default",
             all(c["status"] == "pass" for c in feed_off10)),
            ("feeding gate: 20% difference fails default",
             any(c["status"] == "fail" for c in feed_off20_default)),
            ("feeding gate: 20% difference passes loose",
             all(c["status"] == "pass" for c in feed_off20_loose)),
            ("feeding gate: near-zero absolute floor passes",
             all(c["status"] in ("pass", "info") for c in feed_near)),
            ("feeding gate: unknown fields ignored",
             all(c["status"] == "pass" for c in feed_unknown)),
            ("conservation gate: rust counters evidence gates and passes",
             (lambda c: c["gated"] and c["status"] == "pass" and c["right"].get("gated"))(_conservation_section(
                 {"raw_last_counters": {"evidence_exact": "harmony_exact", "initial_energy": 100.0,
                                        "produced_energy_exact": 10.0, "consumed_energy_exact": 5.0,
                                        "recycled_energy_exact": 0.0, "decayed_energy_exact": 0.0,
                                        "culled_energy_exact": 0.0, "current_pellet_energy": 105.0}},
                 {"evidence": "counters", "raw_last_counters": {},
                  "raw_last_sample": {"conservation": {
                      "seeded_plant_energy": 100.0, "seeded_meat_energy": 0.0,
                      "produced_plant_energy": 10.0, "produced_meat_energy": 0.0,
                      "consumed_plant_energy": 5.0, "consumed_meat_energy": 0.0,
                      "recycled_plant_energy": 0.0, "recycled_meat_energy": 0.0,
                      "decayed_plant_energy": 0.0, "decayed_meat_energy": 0.0,
                      "culled_plant_energy": 0.0, "culled_meat_energy": 0.0,
                      "removed_plant_energy": 0.0, "removed_meat_energy": 0.0,
                      "stock_plant_energy": 105.0, "stock_meat_energy": 0.0,
                      "terms_missing": False}}})),)
        ]
        print(f"[self-test] plain capture: {len(plain_records)} records, bare stream: {len(bare_records)} records")
        ok = all(v for _, v in checks)
        for name, value in checks:
            print(f"[self-test] {'PASS' if value else 'FAIL'}  {name}")
        print(f"[self-test] {'PASS' if ok else 'FAIL'}")
        return 0 if ok else 1


def gzip_first_line(path: Path) -> str:
    import gzip
    with gzip.open(path, "rt", encoding="utf-8") as fh:
        return fh.readline()


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("left", nargs="?", help="capture directory or .jsonl.gz (game)")
    ap.add_argument("right", nargs="?", help="capture directory or .jsonl.gz (game or rust)")
    ap.add_argument("--report", choices=["human", "json"], default="human")
    ap.add_argument("--profile", choices=sorted(PROFILES), default="default")
    ap.add_argument("--out", help="write the JSON report here too")
    ap.add_argument("--strict-settings", action="store_true", help="a settings difference fails the comparison")
    ap.add_argument("--allow-missing-ticks", action="store_true", help="warn instead of failing on partial tick overlap")
    ap.add_argument("--until-tick", type=int, default=None,
                    help="only compare ticks <= N (the game is bit-reproducible only for a short window; "
                         "use e.g. --until-tick 100 with --profile exact)")
    ap.add_argument("--self-test", action="store_true", help="run the synthetic self-test (no game needed)")
    ap.add_argument("--quiet", action="store_true")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    if not args.left or not args.right:
        ap.error("two captures are required (or --self-test)")
    try:
        result = compare(args.left, args.right, args)
    except (FileNotFoundError, ValueError) as exc:
        print(f"compare.py: {exc}", file=sys.stderr)
        return 2
    if args.out:
        Path(args.out).write_text(json.dumps(result, indent=2, default=str), encoding="utf-8")
    if args.report == "json":
        print(json.dumps(result, indent=2, default=str))
    elif not args.quiet:
        print_human(result)
    return result["exit_code"]


if __name__ == "__main__":
    sys.exit(main())

def print_human(result: dict) -> None:
    m = result["meta"]
    print("=" * 96)
    print(f"parity compare  [{result['profile']}]  {m['left']['path']}  vs  {m['right']['path']}")
    print("=" * 96)
    print(f"sources: {m['left']['source']} vs {m['right']['source']}   "
          f"ticks: {m['ticks_left']} vs {m['ticks_right']} (common {m['common_ticks']}, "
          f"only L {m['only_left_n']} / R {m['only_right_n']})")
    print(f"sim_tps: {m['sim_tps_left']} vs {m['sim_tps_right']}   settings_match: {m['settings_match']}")
    for eng, side in (("left", m["left"]["engine"]), ("right", m["right"]["engine"])):
        if side:
            print(f"engine[{eng}]: game {side.get('game_version')} bridge {side.get('bridge_version')} unity {side.get('unity_version')}")
    print("-" * 96)
    for c in result["checks"]:
        status = c["status"].upper().rjust(5)
        gate = "" if c["gate"] else " (info)"
        print(f"  {status}{gate}  {c['name']}")
    print("-" * 96)
    curves = result["systems"]["ecology"]
    worst = sorted(curves.items(), key=lambda kv: kv[1].get("max", 0.0), reverse=True)[:8]
    print("ecology curves (largest max error):")
    for name, st in worst:
        print(f"  {name:28s} mae={st.get('mae', 0):.4g} max={st.get('max', 0):.4g} "
              f"exact={st.get('exact', 0)}/{st.get('n', 0)} tol={st.get('tolerance', 0):.4g} [{st['status']}]")
    ent = result["systems"]["entities"]
    print(f"entities: {ent['bibites_compared']} bibite pairs on {ent['ticks_with_entities']} snapshot ticks "
          f"(unmatched L/R {ent['unmatched_left']}/{ent['unmatched_right']})")
    for name, st in ent["digests"].items():
        print(f"  {name:16s} exact {st['match']}/{st['n']}" + (f" = {st['frac']:.4f}" if st.get("frac") is not None else ""))
    rot = ent.get("rotation") or {}
    if rot.get("n"):
        print(f"  rotation         mae={rot.get('mae', 0):.4g} deg max={rot.get('max', 0):.4g} "
              f"n={rot.get('n')} (Rust radians normalised to degrees, shortest arc)")
    for name in ("energy", "position", "velocity", "age", "health"):
        st = ent[name]
        if not st or not st.get("n"):
            continue
        if name == "energy":
            print(f"  energy           exact {st.get('exact')}/{st.get('n')} mae={st.get('mae', 0):.4g} "
                  f"rel_p95={st.get('rel_p95', 0):.3g}")
        else:
            print(f"  {name:16s} mae={st.get('mae', 0):.4g} rmse={st.get('rmse', 0):.4g} max={st.get('max', 0):.4g} "
                  f"p95={st.get('p95', 0):.4g}")
    outs = result["systems"]["brain"]["outputs"]
    if outs:
        total_n = sum(v.get("n", 0) for v in outs.values())
        total_exact = sum(v.get("exact", 0) for v in outs.values())
        non_exact = {k: f"{v.get('exact', 0)}/{v.get('n', 0)}" for k, v in outs.items()
                     if v.get("n") and v.get("exact") != v.get("n")}
        if non_exact:
            print(f"brain outputs: {total_exact}/{total_n} values bit-exact over {len(outs)} nodes; "
                  f"non-exact nodes: " + ", ".join(f"{k} {v}" for k, v in sorted(non_exact.items())))
        else:
            print(f"brain outputs: ALL {total_exact}/{total_n} values bit-exact over {len(outs)} nodes")
    ev = result["systems"]["events"]
    print(f"events: left {ev['left']} right {ev['right']} identity {ev.get('identity_matches')}/{ev.get('identity_total')}")
    print("-" * 96)
    print(f"OVERALL: {result['overall'].upper()}  exit={result['exit_code']}")
    if result["failures"]:
        print("failed gates: " + ", ".join(result["failures"]))
    if result["warnings"]:
        print("warnings: " + ", ".join(result["warnings"]))


# -------------------------------------------------------------------------------------------------
# self-test
# -------------------------------------------------------------------------------------------------
def synth_capture(path: Path, *, mutate: bool = False, ticks: int = 8, plain: bool = False,
                  f32_style: bool = False, rot_radians: bool = False) -> None:
    path.mkdir(parents=True, exist_ok=True)
    nb = 4
    records = []
    entities = []
    for i in range(nb):
        entities.append({"kind": "bibite", "id": 1000 + i, "species": 1, "generation": 0, "tag": "",
                         "x": 10.13 * i, "y": -5.07 * i, "vx": 1.0, "vy": 2.0,
                         "rot": (math.radians(30.0) if rot_radians else 30.0),
                         "energy": 100.0 + i, "max_energy": 150.0, "health": 90.0, "max_health": 100.0,
                         "age_s": 1.0 + i, "size": 1.2, "nodes": 48, "synapses": 3,
                         "genes_digest": f"{i:016x}", "sense_digest": f"{i + 10:016x}",
                         "output_digest": f"{i + 20:016x}", "brain_digest": f"{i + 30:016x}"})
    for t in range(ticks):
        rec = {"schema": pc.TICK_SCHEMA, "source": "game", "tick": t, "t": t / 30.0,
               "sim_tps": 30, "brain_tps": 2, "settings_hash": "0" * 64,
               "counts": {"bibites": nb, "eggs": 0, "corpses": 0},
               "pellets": {"plant_n": 100, "plant_biomass": 500.13, "meat_n": 0, "meat_biomass": 0.0,
                           "free_biomass": 0.0, "bibite_biomass": 400.37},
               "vitals": {"energy_mean": 101.53, "health_mean": 90.07, "age_mean": 1.53, "age_max": 4.07},
               "generation": {"gen_max": 0, "gen_mean": 0.0},
               "species": {"n": 1, "counts": {"1": nb}},
               "brain": {"nodes_mean": 48.0, "nodes_max": 48, "syn_mean": 3.07, "syn_max": 3},
               "genes": {"mean": [0.0] * len(pc.GENE_NAMES), "std": [0.0] * len(pc.GENE_NAMES)},
               "lifecycle": {"births": 0, "deaths": 0},
               "behavior": {"speed_mean": 2.03, "speed_max": 3.07, "moving_frac": 1.0,
                            "travel_rate_mean": 0.13, "bite_rate_mean": 0.0,
                            "damage_dealt_rate_mean": 0.0, "egg_rate_mean": 0.0,
                            "outputs": {"Accelerate": 0.13, "Rotate": 0.0}},
               "entities": copy.deepcopy(entities)}
        if mutate and t == 3:
            rec["vitals"]["energy_mean"] += 1.5
            rec["entities"][0]["energy"] += 2.0
            rec["entities"][1]["genes_digest"] = "f" * 16
            rec["behavior"]["outputs"]["Accelerate"] += 0.5      # exercises brain-output exact counts
        records.append(rec)
    if f32_style:
        # RUSTibites writes declared-f32 values widened to f64 exactly. The game writes .NET floats
        # with shortest-round-trip decimals, so the same logical value parses to a slightly different
        # double; compare.py must quantise both sides to f32 bits.
        def widen(obj):
            if isinstance(obj, dict):
                return {k: widen(v) for k, v in obj.items()}
            if isinstance(obj, list):
                return [widen(v) for v in obj]
            if isinstance(obj, float):
                return pc.f32_value(obj)
            return obj
        records = [widen(r) for r in records]
    manifest = {"schema": pc.CAPTURE_SCHEMA, "source": ("rust" if rot_radians else "game"),
                "capture": path.name,
                "engine": {"game_version": "0.6.3.1", "bridge_version": "0.7.0"}, "gene_names": pc.GENE_NAMES,
                "settings": {"scenario": {"x": 1.0}, "independent": {"simTPS": 30}}, "settings_hash": "0" * 64,
                "sim": {"sim_tps": 30, "brain_tps": 2}}
    if plain:
        # the RUSTibites writer emits plain UTF-8 JSONL; the reader must accept it side by side with
        # the gzipped game captures (parity_common._read_jsonl_gz dispatches on the .gz suffix).
        with open(path / "ticks.jsonl", "w", encoding="utf-8") as fh:
            for rec in records:
                fh.write(json.dumps(rec, separators=(",", ":"), ensure_ascii=False))
                fh.write("\n")
        # sibling manifest for the bare-file path test (open_capture(<...>/ticks.jsonl))
        (path / "ticks.manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    else:
        pc.write_jsonl_gz(path / "ticks.jsonl.gz", records)
    (path / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")


def compare_result(left: str, right: str, profile: str = "default", strict_settings: bool = False) -> dict:
    class O:
        pass
    o = O()
    o.profile = profile
    o.strict_settings = strict_settings
    o.allow_missing_ticks = False
    o.until_tick = None
    return compare(left, right, o)


def try_compare(left: str, right: str, profile: str = "default", strict_settings: bool = False) -> int:
    return compare_result(left, right, profile, strict_settings)["exit_code"]


def self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="rustibites-parity-selftest-") as td:
        root = Path(td)
        a, b, c = root / "a", root / "b", root / "c"
        synth_capture(a)
        synth_capture(b)
        synth_capture(c, mutate=True)
        # plain (non-gzip) captures: what the RUSTibites writer emits
        plain = root / "plain"
        plain_mut = root / "plain_mut"
        synth_capture(plain, plain=True)
        synth_capture(plain_mut, plain=True, mutate=True)
        # f32-widened style: what RUSTibites writes for declared-f32 scalars
        f32_rust = root / "f32_rust"
        f32_rust_mut = root / "f32_rust_mut"
        synth_capture(f32_rust, plain=True, f32_style=True)
        synth_capture(f32_rust_mut, plain=True, f32_style=True, mutate=True)
        print("[self-test] identical captures ->", try_compare(a, b, "exact"))
        identical = try_compare(a, b, "exact")
        detected = try_compare(a, c, "exact")
        cross_format = try_compare(a, plain, "exact")                 # gzip game vs plain rust
        plain_mutation = try_compare(plain, plain_mut, "exact")
        f32_cross = try_compare(a, f32_rust, "exact")                 # shortest-decimal vs f32-widened
        f32_mutation = try_compare(a, f32_rust_mut, "exact")
        # Rust capture `rot` is radians, game captures are degrees: identical poses must compare as ~0.
        rot_rust = root / "rot_rust"
        synth_capture(rot_rust, plain=True, f32_style=True, rot_radians=True)
        rot_res = compare_result(a, rot_rust, "default")
        rot_mae = (rot_res["systems"]["entities"].get("rotation") or {}).get("mae")
        # older Rust captures wrote degrees: the auto-detection must NOT convert those
        rot_rust_deg = root / "rot_rust_deg"
        synth_capture(rot_rust_deg, plain=True, f32_style=True)
        mf = rot_rust_deg / "manifest.json"
        md = json.loads(mf.read_text(encoding="utf-8"))
        md["source"] = "rust"
        mf.write_text(json.dumps(md), encoding="utf-8")
        rot_deg_res = compare_result(a, rot_rust_deg, "default")
        rot_deg_mae = (rot_deg_res["systems"]["entities"].get("rotation") or {}).get("mae")
        rot_deg_units = (rot_deg_res["systems"]["entities"].get("rotation") or {}).get("rust_rot_treated_as")
        base_res = compare_result(a, plain, "default")
        mut_res = compare_result(a, c, "default")
        f32_res = compare_result(a, f32_rust, "exact")
        base_bo = next(ch["detail"] for ch in base_res["checks"] if ch["name"] == "brain_outputs")
        mut_bo = next(ch["detail"] for ch in mut_res["checks"] if ch["name"] == "brain_outputs")
        brain_counts_ok = (base_bo["all_exact"] is True and base_bo["values_n"] == len(base_bo["per_output"]) * 8
                           and set(base_bo["per_output"]) and all(v["exact"] == v["n"] for v in base_bo["per_output"].values()))
        brain_mut_visible = (mut_bo["all_exact"] is False and "Accelerate" in mut_bo["non_exact_nodes"]
                             and mut_bo["per_output"]["Accelerate"]["exact"] < mut_bo["per_output"]["Accelerate"]["n"])
        behavior_n = base_res["systems"]["behavior"].get("speed_mean", {}).get("n", 0)
        behavior_out_n = len(base_res["systems"]["brain"]["outputs"])
        f32_energy_frac = f32_res["systems"]["entities"]["energy"].get("exact_frac")
        f32_pos_mae = f32_res["systems"]["entities"]["position"].get("mae")
        f32_changes_formatting = pc.f32_value(154.960068) != 154.960068
        try:
            plain_capture = pc.open_capture(plain)
            plain_records = list(plain_capture.records())
            plain_manifest_ok = plain_capture.manifest.get("schema") == pc.CAPTURE_SCHEMA
            plain_ok = len(plain_records) == 8 and plain_manifest_ok
        except Exception:
            plain_records, plain_ok = [], False
        try:
            bare = pc.open_capture(plain / "ticks.jsonl")            # bare stream + <stem>.manifest.json
            bare_records = list(bare.records())
            bare_manifest_ok = bare.manifest.get("schema") == pc.CAPTURE_SCHEMA
            bare_ok = len(bare_records) == 8 and bare_manifest_ok
            bare_compare = try_compare(a, plain / "ticks.jsonl", "exact") == 0
        except Exception:
            bare_records, bare_ok, bare_compare = [], False, False
        same_manifest = pc.record_digest(  # digest stability
            json.loads(gzip_first_line(a / "ticks.jsonl.gz")))
        base_feed = {"plant_bites_per_live_s": 1.0, "meat_bites_per_live_s": 0.0,
                     "plant_consumed_u2_per_live_s": 10.0, "meat_consumed_u2_per_live_s": 0.0,
                     "plant_consumed_e_per_live_s": 50.0, "meat_consumed_e_per_live_s": 0.0,
                     "stomach_energy_gain_e_per_live_s": 50.0, "evidence": "test"}
        off10 = dict(base_feed, plant_bites_per_live_s=1.1, plant_consumed_u2_per_live_s=11.0,
                     plant_consumed_e_per_live_s=55.0, stomach_energy_gain_e_per_live_s=55.0)
        off20 = dict(base_feed, plant_bites_per_live_s=1.2, plant_consumed_u2_per_live_s=12.0,
                     plant_consumed_e_per_live_s=60.0, stomach_energy_gain_e_per_live_s=60.0)
        near = {"plant_bites_per_live_s": 0.0, "plant_consumed_u2_per_live_s": 0.0,
                "plant_consumed_e_per_live_s": 0.0}
        near2 = {"plant_bites_per_live_s": 1e-5, "plant_consumed_u2_per_live_s": 1e-5,
                 "plant_consumed_e_per_live_s": 1e-4}
        feed_eq = _feeding_checks(base_feed, dict(base_feed), PROFILES["default"])
        feed_off10 = _feeding_checks(base_feed, off10, PROFILES["default"])
        feed_off20_default = _feeding_checks(base_feed, off20, PROFILES["default"])
        feed_off20_loose = _feeding_checks(base_feed, off20, PROFILES["loose"])
        feed_near = _feeding_checks(near, near2, PROFILES["default"])
        feed_unknown = _feeding_checks(dict(base_feed, unknown_field=123), dict(base_feed), PROFILES["default"])
        checks = [
            ("identical exit 0", identical == 0),
            ("mutation detected exit 1", detected == 1),
            ("record digest stable", same_manifest == pc.record_digest(json.loads(gzip_first_line(a / "ticks.jsonl.gz")))),
            ("schema constants", pc.TICK_SCHEMA.endswith(".v1") and len(pc.GENE_NAMES) == 35),
            ("settings hash deterministic", pc.settings_hash({"independent": {"simTPS": 30}}) != pc.settings_hash({"independent": {"simTPS": 31}})),
            ("plain JSONL capture opens (open_capture)", plain_ok),
            ("gzip game vs plain rust compare exit 0", cross_format == 0),
            ("plain mutation detected exit 1", plain_mutation == 1),
            ("bare .jsonl + sibling manifest opens/compares", bare_ok and bare_compare),
            ("behavior curves compared (n = ticks)", behavior_n == 8),
            ("behavior output means compared", behavior_out_n > 0),
            ("f32-widened rust capture compares exit 0", f32_cross == 0),
            ("f32-format mutation still detected exit 1", f32_mutation == 1),
            ("f32 quantiser changes decimal formatting", f32_changes_formatting),
            ("f32 entity energy bit-exact", f32_energy_frac == 1.0),
            ("f32 entity position mae == 0", f32_pos_mae == 0),
        ("rotation normalised: Rust radians vs game degrees identical pose -> mae ~0",
         rot_mae is not None and rot_mae < 1e-3),
        ("rotation auto-detect keeps older Rust-degree captures unchanged (mae ~0)",
         rot_deg_mae is not None and rot_deg_mae < 1e-3 and rot_deg_units == "degrees"),
            ("brain per-output exact counts reported (all exact)", brain_counts_ok),
            ("brain per-output counts expose a mutated node", brain_mut_visible),
            ("feeding gate: identical stats pass",
             all(c["status"] == "pass" for c in feed_eq)),
            ("feeding gate: 10% difference passes default",
             all(c["status"] == "pass" for c in feed_off10)),
            ("feeding gate: 20% difference fails default",
             any(c["status"] == "fail" for c in feed_off20_default)),
            ("feeding gate: 20% difference passes loose",
             all(c["status"] == "pass" for c in feed_off20_loose)),
            ("feeding gate: near-zero absolute floor passes",
             all(c["status"] in ("pass", "info") for c in feed_near)),
            ("feeding gate: unknown fields ignored",
             all(c["status"] == "pass" for c in feed_unknown)),
        ]
        print(f"[self-test] plain capture: {len(plain_records)} records, bare stream: {len(bare_records)} records")
        ok = all(v for _, v in checks)
        for name, value in checks:
            print(f"[self-test] {'PASS' if value else 'FAIL'}  {name}")
        print(f"[self-test] {'PASS' if ok else 'FAIL'}")
        return 0 if ok else 1


def gzip_first_line(path: Path) -> str:
    import gzip
    with gzip.open(path, "rt", encoding="utf-8") as fh:
        return fh.readline()


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("left", nargs="?", help="capture directory or .jsonl.gz (game)")
    ap.add_argument("right", nargs="?", help="capture directory or .jsonl.gz (game or rust)")
    ap.add_argument("--report", choices=["human", "json"], default="human")
    ap.add_argument("--profile", choices=sorted(PROFILES), default="default")
    ap.add_argument("--out", help="write the JSON report here too")
    ap.add_argument("--strict-settings", action="store_true", help="a settings difference fails the comparison")
    ap.add_argument("--allow-missing-ticks", action="store_true", help="warn instead of failing on partial tick overlap")
    ap.add_argument("--until-tick", type=int, default=None,
                    help="only compare ticks <= N (the game is bit-reproducible only for a short window; "
                         "use e.g. --until-tick 100 with --profile exact)")
    ap.add_argument("--self-test", action="store_true", help="run the synthetic self-test (no game needed)")
    ap.add_argument("--quiet", action="store_true")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    if not args.left or not args.right:
        ap.error("two captures are required (or --self-test)")
    try:
        result = compare(args.left, args.right, args)
    except (FileNotFoundError, ValueError) as exc:
        print(f"compare.py: {exc}", file=sys.stderr)
        return 2
    if args.out:
        Path(args.out).write_text(json.dumps(result, indent=2, default=str), encoding="utf-8")
    if args.report == "json":
        print(json.dumps(result, indent=2, default=str))
    elif not args.quiet:
        print_human(result)
    return result["exit_code"]


if __name__ == "__main__":
    sys.exit(main())

def print_human(result: dict) -> None:
    m = result["meta"]
    print("=" * 96)
    print(f"parity compare  [{result['profile']}]  {m['left']['path']}  vs  {m['right']['path']}")
    print("=" * 96)
    print(f"sources: {m['left']['source']} vs {m['right']['source']}   "
          f"ticks: {m['ticks_left']} vs {m['ticks_right']} (common {m['common_ticks']}, "
          f"only L {m['only_left_n']} / R {m['only_right_n']})")
    print(f"sim_tps: {m['sim_tps_left']} vs {m['sim_tps_right']}   settings_match: {m['settings_match']}")
    for eng, side in (("left", m["left"]["engine"]), ("right", m["right"]["engine"])):
        if side:
            print(f"engine[{eng}]: game {side.get('game_version')} bridge {side.get('bridge_version')} unity {side.get('unity_version')}")
    print("-" * 96)
    for c in result["checks"]:
        status = c["status"].upper().rjust(5)
        gate = "" if c["gate"] else " (info)"
        print(f"  {status}{gate}  {c['name']}")
    print("-" * 96)
    curves = result["systems"]["ecology"]
    worst = sorted(curves.items(), key=lambda kv: kv[1].get("max", 0.0), reverse=True)[:8]
    print("ecology curves (largest max error):")
    for name, st in worst:
        print(f"  {name:28s} mae={st.get('mae', 0):.4g} max={st.get('max', 0):.4g} "
              f"exact={st.get('exact', 0)}/{st.get('n', 0)} tol={st.get('tolerance', 0):.4g} [{st['status']}]")
    ent = result["systems"]["entities"]
    print(f"entities: {ent['bibites_compared']} bibite pairs on {ent['ticks_with_entities']} snapshot ticks "
          f"(unmatched L/R {ent['unmatched_left']}/{ent['unmatched_right']})")
    for name, st in ent["digests"].items():
        print(f"  {name:16s} exact {st['match']}/{st['n']}" + (f" = {st['frac']:.4f}" if st.get("frac") is not None else ""))
    rot = ent.get("rotation") or {}
    if rot.get("n"):
        print(f"  rotation         mae={rot.get('mae', 0):.4g} deg max={rot.get('max', 0):.4g} "
              f"n={rot.get('n')} (Rust radians normalised to degrees, shortest arc)")
    for name in ("energy", "position", "velocity", "age", "health"):
        st = ent[name]
        if not st or not st.get("n"):
            continue
        if name == "energy":
            print(f"  energy           exact {st.get('exact')}/{st.get('n')} mae={st.get('mae', 0):.4g} "
                  f"rel_p95={st.get('rel_p95', 0):.3g}")
        else:
            print(f"  {name:16s} mae={st.get('mae', 0):.4g} rmse={st.get('rmse', 0):.4g} max={st.get('max', 0):.4g} "
                  f"p95={st.get('p95', 0):.4g}")
    outs = result["systems"]["brain"]["outputs"]
    if outs:
        total_n = sum(v.get("n", 0) for v in outs.values())
        total_exact = sum(v.get("exact", 0) for v in outs.values())
        non_exact = {k: f"{v.get('exact', 0)}/{v.get('n', 0)}" for k, v in outs.items()
                     if v.get("n") and v.get("exact") != v.get("n")}
        if non_exact:
            print(f"brain outputs: {total_exact}/{total_n} values bit-exact over {len(outs)} nodes; "
                  f"non-exact nodes: " + ", ".join(f"{k} {v}" for k, v in sorted(non_exact.items())))
        else:
            print(f"brain outputs: ALL {total_exact}/{total_n} values bit-exact over {len(outs)} nodes")
    ev = result["systems"]["events"]
    print(f"events: left {ev['left']} right {ev['right']} identity {ev.get('identity_matches')}/{ev.get('identity_total')}")
    print("-" * 96)
    print(f"OVERALL: {result['overall'].upper()}  exit={result['exit_code']}")
    if result["failures"]:
        print("failed gates: " + ", ".join(result["failures"]))
    if result["warnings"]:
        print("warnings: " + ", ".join(result["warnings"]))


# -------------------------------------------------------------------------------------------------
# self-test
# -------------------------------------------------------------------------------------------------
def synth_capture(path: Path, *, mutate: bool = False, ticks: int = 8, plain: bool = False,
                  f32_style: bool = False, rot_radians: bool = False) -> None:
    path.mkdir(parents=True, exist_ok=True)
    nb = 4
    records = []
    entities = []
    for i in range(nb):
        entities.append({"kind": "bibite", "id": 1000 + i, "species": 1, "generation": 0, "tag": "",
                         "x": 10.13 * i, "y": -5.07 * i, "vx": 1.0, "vy": 2.0,
                         "rot": (math.radians(30.0) if rot_radians else 30.0),
                         "energy": 100.0 + i, "max_energy": 150.0, "health": 90.0, "max_health": 100.0,
                         "age_s": 1.0 + i, "size": 1.2, "nodes": 48, "synapses": 3,
                         "genes_digest": f"{i:016x}", "sense_digest": f"{i + 10:016x}",
                         "output_digest": f"{i + 20:016x}", "brain_digest": f"{i + 30:016x}"})
    for t in range(ticks):
        rec = {"schema": pc.TICK_SCHEMA, "source": "game", "tick": t, "t": t / 30.0,
               "sim_tps": 30, "brain_tps": 2, "settings_hash": "0" * 64,
               "counts": {"bibites": nb, "eggs": 0, "corpses": 0},
               "pellets": {"plant_n": 100, "plant_biomass": 500.13, "meat_n": 0, "meat_biomass": 0.0,
                           "free_biomass": 0.0, "bibite_biomass": 400.37},
               "vitals": {"energy_mean": 101.53, "health_mean": 90.07, "age_mean": 1.53, "age_max": 4.07},
               "generation": {"gen_max": 0, "gen_mean": 0.0},
               "species": {"n": 1, "counts": {"1": nb}},
               "brain": {"nodes_mean": 48.0, "nodes_max": 48, "syn_mean": 3.07, "syn_max": 3},
               "genes": {"mean": [0.0] * len(pc.GENE_NAMES), "std": [0.0] * len(pc.GENE_NAMES)},
               "lifecycle": {"births": 0, "deaths": 0},
               "behavior": {"speed_mean": 2.03, "speed_max": 3.07, "moving_frac": 1.0,
                            "travel_rate_mean": 0.13, "bite_rate_mean": 0.0,
                            "damage_dealt_rate_mean": 0.0, "egg_rate_mean": 0.0,
                            "outputs": {"Accelerate": 0.13, "Rotate": 0.0}},
               "entities": copy.deepcopy(entities)}
        if mutate and t == 3:
            rec["vitals"]["energy_mean"] += 1.5
            rec["entities"][0]["energy"] += 2.0
            rec["entities"][1]["genes_digest"] = "f" * 16
            rec["behavior"]["outputs"]["Accelerate"] += 0.5      # exercises brain-output exact counts
        records.append(rec)
    if f32_style:
        # RUSTibites writes declared-f32 values widened to f64 exactly. The game writes .NET floats
        # with shortest-round-trip decimals, so the same logical value parses to a slightly different
        # double; compare.py must quantise both sides to f32 bits.
        def widen(obj):
            if isinstance(obj, dict):
                return {k: widen(v) for k, v in obj.items()}
            if isinstance(obj, list):
                return [widen(v) for v in obj]
            if isinstance(obj, float):
                return pc.f32_value(obj)
            return obj
        records = [widen(r) for r in records]
    manifest = {"schema": pc.CAPTURE_SCHEMA, "source": ("rust" if rot_radians else "game"),
                "capture": path.name,
                "engine": {"game_version": "0.6.3.1", "bridge_version": "0.7.0"}, "gene_names": pc.GENE_NAMES,
                "settings": {"scenario": {"x": 1.0}, "independent": {"simTPS": 30}}, "settings_hash": "0" * 64,
                "sim": {"sim_tps": 30, "brain_tps": 2}}
    if plain:
        # the RUSTibites writer emits plain UTF-8 JSONL; the reader must accept it side by side with
        # the gzipped game captures (parity_common._read_jsonl_gz dispatches on the .gz suffix).
        with open(path / "ticks.jsonl", "w", encoding="utf-8") as fh:
            for rec in records:
                fh.write(json.dumps(rec, separators=(",", ":"), ensure_ascii=False))
                fh.write("\n")
        # sibling manifest for the bare-file path test (open_capture(<...>/ticks.jsonl))
        (path / "ticks.manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    else:
        pc.write_jsonl_gz(path / "ticks.jsonl.gz", records)
    (path / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")


def compare_result(left: str, right: str, profile: str = "default", strict_settings: bool = False) -> dict:
    class O:
        pass
    o = O()
    o.profile = profile
    o.strict_settings = strict_settings
    o.allow_missing_ticks = False
    o.until_tick = None
    return compare(left, right, o)


def try_compare(left: str, right: str, profile: str = "default", strict_settings: bool = False) -> int:
    return compare_result(left, right, profile, strict_settings)["exit_code"]


def self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="rustibites-parity-selftest-") as td:
        root = Path(td)
        a, b, c = root / "a", root / "b", root / "c"
        synth_capture(a)
        synth_capture(b)
        synth_capture(c, mutate=True)
        # plain (non-gzip) captures: what the RUSTibites writer emits
        plain = root / "plain"
        plain_mut = root / "plain_mut"
        synth_capture(plain, plain=True)
        synth_capture(plain_mut, plain=True, mutate=True)
        # f32-widened style: what RUSTibites writes for declared-f32 scalars
        f32_rust = root / "f32_rust"
        f32_rust_mut = root / "f32_rust_mut"
        synth_capture(f32_rust, plain=True, f32_style=True)
        synth_capture(f32_rust_mut, plain=True, f32_style=True, mutate=True)
        print("[self-test] identical captures ->", try_compare(a, b, "exact"))
        identical = try_compare(a, b, "exact")
        detected = try_compare(a, c, "exact")
        cross_format = try_compare(a, plain, "exact")                 # gzip game vs plain rust
        plain_mutation = try_compare(plain, plain_mut, "exact")
        f32_cross = try_compare(a, f32_rust, "exact")                 # shortest-decimal vs f32-widened
        f32_mutation = try_compare(a, f32_rust_mut, "exact")
        # Rust capture `rot` is radians, game captures are degrees: identical poses must compare as ~0.
        rot_rust = root / "rot_rust"
        synth_capture(rot_rust, plain=True, f32_style=True, rot_radians=True)
        rot_res = compare_result(a, rot_rust, "default")
        rot_mae = (rot_res["systems"]["entities"].get("rotation") or {}).get("mae")
        # older Rust captures wrote degrees: the auto-detection must NOT convert those
        rot_rust_deg = root / "rot_rust_deg"
        synth_capture(rot_rust_deg, plain=True, f32_style=True)
        mf = rot_rust_deg / "manifest.json"
        md = json.loads(mf.read_text(encoding="utf-8"))
        md["source"] = "rust"
        mf.write_text(json.dumps(md), encoding="utf-8")
        rot_deg_res = compare_result(a, rot_rust_deg, "default")
        rot_deg_mae = (rot_deg_res["systems"]["entities"].get("rotation") or {}).get("mae")
        rot_deg_units = (rot_deg_res["systems"]["entities"].get("rotation") or {}).get("rust_rot_treated_as")
        base_res = compare_result(a, plain, "default")
        mut_res = compare_result(a, c, "default")
        f32_res = compare_result(a, f32_rust, "exact")
        base_bo = next(ch["detail"] for ch in base_res["checks"] if ch["name"] == "brain_outputs")
        mut_bo = next(ch["detail"] for ch in mut_res["checks"] if ch["name"] == "brain_outputs")
        brain_counts_ok = (base_bo["all_exact"] is True and base_bo["values_n"] == len(base_bo["per_output"]) * 8
                           and set(base_bo["per_output"]) and all(v["exact"] == v["n"] for v in base_bo["per_output"].values()))
        brain_mut_visible = (mut_bo["all_exact"] is False and "Accelerate" in mut_bo["non_exact_nodes"]
                             and mut_bo["per_output"]["Accelerate"]["exact"] < mut_bo["per_output"]["Accelerate"]["n"])
        behavior_n = base_res["systems"]["behavior"].get("speed_mean", {}).get("n", 0)
        behavior_out_n = len(base_res["systems"]["brain"]["outputs"])
        f32_energy_frac = f32_res["systems"]["entities"]["energy"].get("exact_frac")
        f32_pos_mae = f32_res["systems"]["entities"]["position"].get("mae")
        f32_changes_formatting = pc.f32_value(154.960068) != 154.960068
        try:
            plain_capture = pc.open_capture(plain)
            plain_records = list(plain_capture.records())
            plain_manifest_ok = plain_capture.manifest.get("schema") == pc.CAPTURE_SCHEMA
            plain_ok = len(plain_records) == 8 and plain_manifest_ok
        except Exception:
            plain_records, plain_ok = [], False
        try:
            bare = pc.open_capture(plain / "ticks.jsonl")            # bare stream + <stem>.manifest.json
            bare_records = list(bare.records())
            bare_manifest_ok = bare.manifest.get("schema") == pc.CAPTURE_SCHEMA
            bare_ok = len(bare_records) == 8 and bare_manifest_ok
            bare_compare = try_compare(a, plain / "ticks.jsonl", "exact") == 0
        except Exception:
            bare_records, bare_ok, bare_compare = [], False, False
        same_manifest = pc.record_digest(  # digest stability
            json.loads(gzip_first_line(a / "ticks.jsonl.gz")))
        base_feed = {"plant_bites_per_live_s": 1.0, "meat_bites_per_live_s": 0.0,
                     "plant_consumed_u2_per_live_s": 10.0, "meat_consumed_u2_per_live_s": 0.0,
                     "plant_consumed_e_per_live_s": 50.0, "meat_consumed_e_per_live_s": 0.0,
                     "stomach_energy_gain_e_per_live_s": 50.0, "evidence": "test"}
        off10 = dict(base_feed, plant_bites_per_live_s=1.1, plant_consumed_u2_per_live_s=11.0,
                     plant_consumed_e_per_live_s=55.0, stomach_energy_gain_e_per_live_s=55.0)
        off20 = dict(base_feed, plant_bites_per_live_s=1.2, plant_consumed_u2_per_live_s=12.0,
                     plant_consumed_e_per_live_s=60.0, stomach_energy_gain_e_per_live_s=60.0)
        near = {"plant_bites_per_live_s": 0.0, "plant_consumed_u2_per_live_s": 0.0,
                "plant_consumed_e_per_live_s": 0.0}
        near2 = {"plant_bites_per_live_s": 1e-5, "plant_consumed_u2_per_live_s": 1e-5,
                 "plant_consumed_e_per_live_s": 1e-4}
        feed_eq = _feeding_checks(base_feed, dict(base_feed), PROFILES["default"])
        feed_off10 = _feeding_checks(base_feed, off10, PROFILES["default"])
        feed_off20_default = _feeding_checks(base_feed, off20, PROFILES["default"])
        feed_off20_loose = _feeding_checks(base_feed, off20, PROFILES["loose"])
        feed_near = _feeding_checks(near, near2, PROFILES["default"])
        feed_unknown = _feeding_checks(dict(base_feed, unknown_field=123), dict(base_feed), PROFILES["default"])
        checks = [
            ("identical exit 0", identical == 0),
            ("mutation detected exit 1", detected == 1),
            ("record digest stable", same_manifest == pc.record_digest(json.loads(gzip_first_line(a / "ticks.jsonl.gz")))),
            ("schema constants", pc.TICK_SCHEMA.endswith(".v1") and len(pc.GENE_NAMES) == 35),
            ("settings hash deterministic", pc.settings_hash({"independent": {"simTPS": 30}}) != pc.settings_hash({"independent": {"simTPS": 31}})),
            ("plain JSONL capture opens (open_capture)", plain_ok),
            ("gzip game vs plain rust compare exit 0", cross_format == 0),
            ("plain mutation detected exit 1", plain_mutation == 1),
            ("bare .jsonl + sibling manifest opens/compares", bare_ok and bare_compare),
            ("behavior curves compared (n = ticks)", behavior_n == 8),
            ("behavior output means compared", behavior_out_n > 0),
            ("f32-widened rust capture compares exit 0", f32_cross == 0),
            ("f32-format mutation still detected exit 1", f32_mutation == 1),
            ("f32 quantiser changes decimal formatting", f32_changes_formatting),
            ("f32 entity energy bit-exact", f32_energy_frac == 1.0),
            ("f32 entity position mae == 0", f32_pos_mae == 0),
        ("rotation normalised: Rust radians vs game degrees identical pose -> mae ~0",
         rot_mae is not None and rot_mae < 1e-3),
        ("rotation auto-detect keeps older Rust-degree captures unchanged (mae ~0)",
         rot_deg_mae is not None and rot_deg_mae < 1e-3 and rot_deg_units == "degrees"),
            ("brain per-output exact counts reported (all exact)", brain_counts_ok),
            ("brain per-output counts expose a mutated node", brain_mut_visible),
            ("feeding gate: identical stats pass",
             all(c["status"] == "pass" for c in feed_eq)),
            ("feeding gate: 10% difference passes default",
             all(c["status"] == "pass" for c in feed_off10)),
            ("feeding gate: 20% difference fails default",
             any(c["status"] == "fail" for c in feed_off20_default)),
            ("feeding gate: 20% difference passes loose",
             all(c["status"] == "pass" for c in feed_off20_loose)),
            ("feeding gate: near-zero absolute floor passes",
             all(c["status"] in ("pass", "info") for c in feed_near)),
            ("feeding gate: unknown fields ignored",
             all(c["status"] == "pass" for c in feed_unknown)),
        ]
        print(f"[self-test] plain capture: {len(plain_records)} records, bare stream: {len(bare_records)} records")
        ok = all(v for _, v in checks)
        for name, value in checks:
            print(f"[self-test] {'PASS' if value else 'FAIL'}  {name}")
        print(f"[self-test] {'PASS' if ok else 'FAIL'}")
        return 0 if ok else 1


def gzip_first_line(path: Path) -> str:
    import gzip
    with gzip.open(path, "rt", encoding="utf-8") as fh:
        return fh.readline()


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("left", nargs="?", help="capture directory or .jsonl.gz (game)")
    ap.add_argument("right", nargs="?", help="capture directory or .jsonl.gz (game or rust)")
    ap.add_argument("--report", choices=["human", "json"], default="human")
    ap.add_argument("--profile", choices=sorted(PROFILES), default="default")
    ap.add_argument("--out", help="write the JSON report here too")
    ap.add_argument("--strict-settings", action="store_true", help="a settings difference fails the comparison")
    ap.add_argument("--allow-missing-ticks", action="store_true", help="warn instead of failing on partial tick overlap")
    ap.add_argument("--until-tick", type=int, default=None,
                    help="only compare ticks <= N (the game is bit-reproducible only for a short window; "
                         "use e.g. --until-tick 100 with --profile exact)")
    ap.add_argument("--self-test", action="store_true", help="run the synthetic self-test (no game needed)")
    ap.add_argument("--quiet", action="store_true")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    if not args.left or not args.right:
        ap.error("two captures are required (or --self-test)")
    try:
        result = compare(args.left, args.right, args)
    except (FileNotFoundError, ValueError) as exc:
        print(f"compare.py: {exc}", file=sys.stderr)
        return 2
    if args.out:
        Path(args.out).write_text(json.dumps(result, indent=2, default=str), encoding="utf-8")
    if args.report == "json":
        print(json.dumps(result, indent=2, default=str))
    elif not args.quiet:
        print_human(result)
    return result["exit_code"]


if __name__ == "__main__":
    sys.exit(main())

def print_human(result: dict) -> None:
    m = result["meta"]
    print("=" * 96)
    print(f"parity compare  [{result['profile']}]  {m['left']['path']}  vs  {m['right']['path']}")
    print("=" * 96)
    print(f"sources: {m['left']['source']} vs {m['right']['source']}   "
          f"ticks: {m['ticks_left']} vs {m['ticks_right']} (common {m['common_ticks']}, "
          f"only L {m['only_left_n']} / R {m['only_right_n']})")
    print(f"sim_tps: {m['sim_tps_left']} vs {m['sim_tps_right']}   settings_match: {m['settings_match']}")
    for eng, side in (("left", m["left"]["engine"]), ("right", m["right"]["engine"])):
        if side:
            print(f"engine[{eng}]: game {side.get('game_version')} bridge {side.get('bridge_version')} unity {side.get('unity_version')}")
    print("-" * 96)
    for c in result["checks"]:
        status = c["status"].upper().rjust(5)
        gate = "" if c["gate"] else " (info)"
        print(f"  {status}{gate}  {c['name']}")
    print("-" * 96)
    curves = result["systems"]["ecology"]
    worst = sorted(curves.items(), key=lambda kv: kv[1].get("max", 0.0), reverse=True)[:8]
    print("ecology curves (largest max error):")
    for name, st in worst:
        print(f"  {name:28s} mae={st.get('mae', 0):.4g} max={st.get('max', 0):.4g} "
              f"exact={st.get('exact', 0)}/{st.get('n', 0)} tol={st.get('tolerance', 0):.4g} [{st['status']}]")
    ent = result["systems"]["entities"]
    print(f"entities: {ent['bibites_compared']} bibite pairs on {ent['ticks_with_entities']} snapshot ticks "
          f"(unmatched L/R {ent['unmatched_left']}/{ent['unmatched_right']})")
    for name, st in ent["digests"].items():
        print(f"  {name:16s} exact {st['match']}/{st['n']}" + (f" = {st['frac']:.4f}" if st.get("frac") is not None else ""))
    rot = ent.get("rotation") or {}
    if rot.get("n"):
        print(f"  rotation         mae={rot.get('mae', 0):.4g} deg max={rot.get('max', 0):.4g} "
              f"n={rot.get('n')} (Rust radians normalised to degrees, shortest arc)")
    for name in ("energy", "position", "velocity", "age", "health"):
        st = ent[name]
        if not st or not st.get("n"):
            continue
        if name == "energy":
            print(f"  energy           exact {st.get('exact')}/{st.get('n')} mae={st.get('mae', 0):.4g} "
                  f"rel_p95={st.get('rel_p95', 0):.3g}")
        else:
            print(f"  {name:16s} mae={st.get('mae', 0):.4g} rmse={st.get('rmse', 0):.4g} max={st.get('max', 0):.4g} "
                  f"p95={st.get('p95', 0):.4g}")
    outs = result["systems"]["brain"]["outputs"]
    if outs:
        total_n = sum(v.get("n", 0) for v in outs.values())
        total_exact = sum(v.get("exact", 0) for v in outs.values())
        non_exact = {k: f"{v.get('exact', 0)}/{v.get('n', 0)}" for k, v in outs.items()
                     if v.get("n") and v.get("exact") != v.get("n")}
        if non_exact:
            print(f"brain outputs: {total_exact}/{total_n} values bit-exact over {len(outs)} nodes; "
                  f"non-exact nodes: " + ", ".join(f"{k} {v}" for k, v in sorted(non_exact.items())))
        else:
            print(f"brain outputs: ALL {total_exact}/{total_n} values bit-exact over {len(outs)} nodes")
    ev = result["systems"]["events"]
    print(f"events: left {ev['left']} right {ev['right']} identity {ev.get('identity_matches')}/{ev.get('identity_total')}")
    print("-" * 96)
    print(f"OVERALL: {result['overall'].upper()}  exit={result['exit_code']}")
    if result["failures"]:
        print("failed gates: " + ", ".join(result["failures"]))
    if result["warnings"]:
        print("warnings: " + ", ".join(result["warnings"]))


# -------------------------------------------------------------------------------------------------
# self-test
# -------------------------------------------------------------------------------------------------
def synth_capture(path: Path, *, mutate: bool = False, ticks: int = 8, plain: bool = False,
                  f32_style: bool = False, rot_radians: bool = False) -> None:
    path.mkdir(parents=True, exist_ok=True)
    nb = 4
    records = []
    entities = []
    for i in range(nb):
        entities.append({"kind": "bibite", "id": 1000 + i, "species": 1, "generation": 0, "tag": "",
                         "x": 10.13 * i, "y": -5.07 * i, "vx": 1.0, "vy": 2.0,
                         "rot": (math.radians(30.0) if rot_radians else 30.0),
                         "energy": 100.0 + i, "max_energy": 150.0, "health": 90.0, "max_health": 100.0,
                         "age_s": 1.0 + i, "size": 1.2, "nodes": 48, "synapses": 3,
                         "genes_digest": f"{i:016x}", "sense_digest": f"{i + 10:016x}",
                         "output_digest": f"{i + 20:016x}", "brain_digest": f"{i + 30:016x}"})
    for t in range(ticks):
        rec = {"schema": pc.TICK_SCHEMA, "source": "game", "tick": t, "t": t / 30.0,
               "sim_tps": 30, "brain_tps": 2, "settings_hash": "0" * 64,
               "counts": {"bibites": nb, "eggs": 0, "corpses": 0},
               "pellets": {"plant_n": 100, "plant_biomass": 500.13, "meat_n": 0, "meat_biomass": 0.0,
                           "free_biomass": 0.0, "bibite_biomass": 400.37},
               "vitals": {"energy_mean": 101.53, "health_mean": 90.07, "age_mean": 1.53, "age_max": 4.07},
               "generation": {"gen_max": 0, "gen_mean": 0.0},
               "species": {"n": 1, "counts": {"1": nb}},
               "brain": {"nodes_mean": 48.0, "nodes_max": 48, "syn_mean": 3.07, "syn_max": 3},
               "genes": {"mean": [0.0] * len(pc.GENE_NAMES), "std": [0.0] * len(pc.GENE_NAMES)},
               "lifecycle": {"births": 0, "deaths": 0},
               "behavior": {"speed_mean": 2.03, "speed_max": 3.07, "moving_frac": 1.0,
                            "travel_rate_mean": 0.13, "bite_rate_mean": 0.0,
                            "damage_dealt_rate_mean": 0.0, "egg_rate_mean": 0.0,
                            "outputs": {"Accelerate": 0.13, "Rotate": 0.0}},
               "entities": copy.deepcopy(entities)}
        if mutate and t == 3:
            rec["vitals"]["energy_mean"] += 1.5
            rec["entities"][0]["energy"] += 2.0
            rec["entities"][1]["genes_digest"] = "f" * 16
            rec["behavior"]["outputs"]["Accelerate"] += 0.5      # exercises brain-output exact counts
        records.append(rec)
    if f32_style:
        # RUSTibites writes declared-f32 values widened to f64 exactly. The game writes .NET floats
        # with shortest-round-trip decimals, so the same logical value parses to a slightly different
        # double; compare.py must quantise both sides to f32 bits.
        def widen(obj):
            if isinstance(obj, dict):
                return {k: widen(v) for k, v in obj.items()}
            if isinstance(obj, list):
                return [widen(v) for v in obj]
            if isinstance(obj, float):
                return pc.f32_value(obj)
            return obj
        records = [widen(r) for r in records]
    manifest = {"schema": pc.CAPTURE_SCHEMA, "source": ("rust" if rot_radians else "game"),
                "capture": path.name,
                "engine": {"game_version": "0.6.3.1", "bridge_version": "0.7.0"}, "gene_names": pc.GENE_NAMES,
                "settings": {"scenario": {"x": 1.0}, "independent": {"simTPS": 30}}, "settings_hash": "0" * 64,
                "sim": {"sim_tps": 30, "brain_tps": 2}}
    if plain:
        # the RUSTibites writer emits plain UTF-8 JSONL; the reader must accept it side by side with
        # the gzipped game captures (parity_common._read_jsonl_gz dispatches on the .gz suffix).
        with open(path / "ticks.jsonl", "w", encoding="utf-8") as fh:
            for rec in records:
                fh.write(json.dumps(rec, separators=(",", ":"), ensure_ascii=False))
                fh.write("\n")
        # sibling manifest for the bare-file path test (open_capture(<...>/ticks.jsonl))
        (path / "ticks.manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    else:
        pc.write_jsonl_gz(path / "ticks.jsonl.gz", records)
    (path / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")


def compare_result(left: str, right: str, profile: str = "default", strict_settings: bool = False) -> dict:
    class O:
        pass
    o = O()
    o.profile = profile
    o.strict_settings = strict_settings
    o.allow_missing_ticks = False
    o.until_tick = None
    return compare(left, right, o)


def try_compare(left: str, right: str, profile: str = "default", strict_settings: bool = False) -> int:
    return compare_result(left, right, profile, strict_settings)["exit_code"]


def self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="rustibites-parity-selftest-") as td:
        root = Path(td)
        a, b, c = root / "a", root / "b", root / "c"
        synth_capture(a)
        synth_capture(b)
        synth_capture(c, mutate=True)
        # plain (non-gzip) captures: what the RUSTibites writer emits
        plain = root / "plain"
        plain_mut = root / "plain_mut"
        synth_capture(plain, plain=True)
        synth_capture(plain_mut, plain=True, mutate=True)
        # f32-widened style: what RUSTibites writes for declared-f32 scalars
        f32_rust = root / "f32_rust"
        f32_rust_mut = root / "f32_rust_mut"
        synth_capture(f32_rust, plain=True, f32_style=True)
        synth_capture(f32_rust_mut, plain=True, f32_style=True, mutate=True)
        print("[self-test] identical captures ->", try_compare(a, b, "exact"))
        identical = try_compare(a, b, "exact")
        detected = try_compare(a, c, "exact")
        cross_format = try_compare(a, plain, "exact")                 # gzip game vs plain rust
        plain_mutation = try_compare(plain, plain_mut, "exact")
        f32_cross = try_compare(a, f32_rust, "exact")                 # shortest-decimal vs f32-widened
        f32_mutation = try_compare(a, f32_rust_mut, "exact")
        # Rust capture `rot` is radians, game captures are degrees: identical poses must compare as ~0.
        rot_rust = root / "rot_rust"
        synth_capture(rot_rust, plain=True, f32_style=True, rot_radians=True)
        rot_res = compare_result(a, rot_rust, "default")
        rot_mae = (rot_res["systems"]["entities"].get("rotation") or {}).get("mae")
        # older Rust captures wrote degrees: the auto-detection must NOT convert those
        rot_rust_deg = root / "rot_rust_deg"
        synth_capture(rot_rust_deg, plain=True, f32_style=True)
        mf = rot_rust_deg / "manifest.json"
        md = json.loads(mf.read_text(encoding="utf-8"))
        md["source"] = "rust"
        mf.write_text(json.dumps(md), encoding="utf-8")
        rot_deg_res = compare_result(a, rot_rust_deg, "default")
        rot_deg_mae = (rot_deg_res["systems"]["entities"].get("rotation") or {}).get("mae")
        rot_deg_units = (rot_deg_res["systems"]["entities"].get("rotation") or {}).get("rust_rot_treated_as")
        base_res = compare_result(a, plain, "default")
        mut_res = compare_result(a, c, "default")
        f32_res = compare_result(a, f32_rust, "exact")
        base_bo = next(ch["detail"] for ch in base_res["checks"] if ch["name"] == "brain_outputs")
        mut_bo = next(ch["detail"] for ch in mut_res["checks"] if ch["name"] == "brain_outputs")
        brain_counts_ok = (base_bo["all_exact"] is True and base_bo["values_n"] == len(base_bo["per_output"]) * 8
                           and set(base_bo["per_output"]) and all(v["exact"] == v["n"] for v in base_bo["per_output"].values()))
        brain_mut_visible = (mut_bo["all_exact"] is False and "Accelerate" in mut_bo["non_exact_nodes"]
                             and mut_bo["per_output"]["Accelerate"]["exact"] < mut_bo["per_output"]["Accelerate"]["n"])
        behavior_n = base_res["systems"]["behavior"].get("speed_mean", {}).get("n", 0)
        behavior_out_n = len(base_res["systems"]["brain"]["outputs"])
        f32_energy_frac = f32_res["systems"]["entities"]["energy"].get("exact_frac")
        f32_pos_mae = f32_res["systems"]["entities"]["position"].get("mae")
        f32_changes_formatting = pc.f32_value(154.960068) != 154.960068
        try:
            plain_capture = pc.open_capture(plain)
            plain_records = list(plain_capture.records())
            plain_manifest_ok = plain_capture.manifest.get("schema") == pc.CAPTURE_SCHEMA
            plain_ok = len(plain_records) == 8 and plain_manifest_ok
        except Exception:
            plain_records, plain_ok = [], False
        try:
            bare = pc.open_capture(plain / "ticks.jsonl")            # bare stream + <stem>.manifest.json
            bare_records = list(bare.records())
            bare_manifest_ok = bare.manifest.get("schema") == pc.CAPTURE_SCHEMA
            bare_ok = len(bare_records) == 8 and bare_manifest_ok
            bare_compare = try_compare(a, plain / "ticks.jsonl", "exact") == 0
        except Exception:
            bare_records, bare_ok, bare_compare = [], False, False
        same_manifest = pc.record_digest(  # digest stability
            json.loads(gzip_first_line(a / "ticks.jsonl.gz")))
        base_feed = {"plant_bites_per_live_s": 1.0, "meat_bites_per_live_s": 0.0,
                     "plant_consumed_u2_per_live_s": 10.0, "meat_consumed_u2_per_live_s": 0.0,
                     "plant_consumed_e_per_live_s": 50.0, "meat_consumed_e_per_live_s": 0.0,
                     "stomach_energy_gain_e_per_live_s": 50.0, "evidence": "test"}
        off10 = dict(base_feed, plant_bites_per_live_s=1.1, plant_consumed_u2_per_live_s=11.0,
                     plant_consumed_e_per_live_s=55.0, stomach_energy_gain_e_per_live_s=55.0)
        off20 = dict(base_feed, plant_bites_per_live_s=1.2, plant_consumed_u2_per_live_s=12.0,
                     plant_consumed_e_per_live_s=60.0, stomach_energy_gain_e_per_live_s=60.0)
        near = {"plant_bites_per_live_s": 0.0, "plant_consumed_u2_per_live_s": 0.0,
                "plant_consumed_e_per_live_s": 0.0}
        near2 = {"plant_bites_per_live_s": 1e-5, "plant_consumed_u2_per_live_s": 1e-5,
                 "plant_consumed_e_per_live_s": 1e-4}
        feed_eq = _feeding_checks(base_feed, dict(base_feed), PROFILES["default"])
        feed_off10 = _feeding_checks(base_feed, off10, PROFILES["default"])
        feed_off20_default = _feeding_checks(base_feed, off20, PROFILES["default"])
        feed_off20_loose = _feeding_checks(base_feed, off20, PROFILES["loose"])
        feed_near = _feeding_checks(near, near2, PROFILES["default"])
        feed_unknown = _feeding_checks(dict(base_feed, unknown_field=123), dict(base_feed), PROFILES["default"])
        checks = [
            ("identical exit 0", identical == 0),
            ("mutation detected exit 1", detected == 1),
            ("record digest stable", same_manifest == pc.record_digest(json.loads(gzip_first_line(a / "ticks.jsonl.gz")))),
            ("schema constants", pc.TICK_SCHEMA.endswith(".v1") and len(pc.GENE_NAMES) == 35),
            ("settings hash deterministic", pc.settings_hash({"independent": {"simTPS": 30}}) != pc.settings_hash({"independent": {"simTPS": 31}})),
            ("plain JSONL capture opens (open_capture)", plain_ok),
            ("gzip game vs plain rust compare exit 0", cross_format == 0),
            ("plain mutation detected exit 1", plain_mutation == 1),
            ("bare .jsonl + sibling manifest opens/compares", bare_ok and bare_compare),
            ("behavior curves compared (n = ticks)", behavior_n == 8),
            ("behavior output means compared", behavior_out_n > 0),
            ("f32-widened rust capture compares exit 0", f32_cross == 0),
            ("f32-format mutation still detected exit 1", f32_mutation == 1),
            ("f32 quantiser changes decimal formatting", f32_changes_formatting),
            ("f32 entity energy bit-exact", f32_energy_frac == 1.0),
            ("f32 entity position mae == 0", f32_pos_mae == 0),
        ("rotation normalised: Rust radians vs game degrees identical pose -> mae ~0",
         rot_mae is not None and rot_mae < 1e-3),
        ("rotation auto-detect keeps older Rust-degree captures unchanged (mae ~0)",
         rot_deg_mae is not None and rot_deg_mae < 1e-3 and rot_deg_units == "degrees"),
            ("brain per-output exact counts reported (all exact)", brain_counts_ok),
            ("brain per-output counts expose a mutated node", brain_mut_visible),
            ("feeding gate: identical stats pass",
             all(c["status"] == "pass" for c in feed_eq)),
            ("feeding gate: 10% difference passes default",
             all(c["status"] == "pass" for c in feed_off10)),
            ("feeding gate: 20% difference fails default",
             any(c["status"] == "fail" for c in feed_off20_default)),
            ("feeding gate: 20% difference passes loose",
             all(c["status"] == "pass" for c in feed_off20_loose)),
            ("feeding gate: near-zero absolute floor passes",
             all(c["status"] in ("pass", "info") for c in feed_near)),
            ("feeding gate: unknown fields ignored",
             all(c["status"] == "pass" for c in feed_unknown)),
        ]
        print(f"[self-test] plain capture: {len(plain_records)} records, bare stream: {len(bare_records)} records")
        ok = all(v for _, v in checks)
        for name, value in checks:
            print(f"[self-test] {'PASS' if value else 'FAIL'}  {name}")
        print(f"[self-test] {'PASS' if ok else 'FAIL'}")
        return 0 if ok else 1


def gzip_first_line(path: Path) -> str:
    import gzip
    with gzip.open(path, "rt", encoding="utf-8") as fh:
        return fh.readline()


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("left", nargs="?", help="capture directory or .jsonl.gz (game)")
    ap.add_argument("right", nargs="?", help="capture directory or .jsonl.gz (game or rust)")
    ap.add_argument("--report", choices=["human", "json"], default="human")
    ap.add_argument("--profile", choices=sorted(PROFILES), default="default")
    ap.add_argument("--out", help="write the JSON report here too")
    ap.add_argument("--strict-settings", action="store_true", help="a settings difference fails the comparison")
    ap.add_argument("--allow-missing-ticks", action="store_true", help="warn instead of failing on partial tick overlap")
    ap.add_argument("--until-tick", type=int, default=None,
                    help="only compare ticks <= N (the game is bit-reproducible only for a short window; "
                         "use e.g. --until-tick 100 with --profile exact)")
    ap.add_argument("--self-test", action="store_true", help="run the synthetic self-test (no game needed)")
    ap.add_argument("--quiet", action="store_true")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    if not args.left or not args.right:
        ap.error("two captures are required (or --self-test)")
    try:
        result = compare(args.left, args.right, args)
    except (FileNotFoundError, ValueError) as exc:
        print(f"compare.py: {exc}", file=sys.stderr)
        return 2
    if args.out:
        Path(args.out).write_text(json.dumps(result, indent=2, default=str), encoding="utf-8")
    if args.report == "json":
        print(json.dumps(result, indent=2, default=str))
    elif not args.quiet:
        print_human(result)
    return result["exit_code"]


if __name__ == "__main__":
    sys.exit(main())


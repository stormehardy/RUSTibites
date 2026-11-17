#!/usr/bin/env python3
"""decay_ref.py - optional meat/decay-world conservation check (task-125 follow-up).

Compares the hidden game exact conservation counters with the Rust `conservation` object on game-written
fixtures that contain meat/plant pellets or a corpse:
  decay_world_600.zip (250 meat + 60 plant pellets injected), 600/3000 t
  corpse_world_600.zip (Default start bibite killed -> 1 corpse), 600/3000 t
Reports which terms are nonzero, the two-sided identity, residuals, and `terms_missing`; writes
`testdata/parity/default/decay_analysis.json` + `.md`. Reads game feeding streams and Rust feeding streams.
"""
from __future__ import annotations

import gzip
import json
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
DEFAULT = ROOT / "testdata/parity/default"


def game_counters(cap_dir: Path) -> dict:
    p = cap_dir / "feeding.jsonl.gz"
    rows = [json.loads(l) for l in gzip.open(p, "rt", encoding="utf-8")]
    return rows[-1].get("counters") or {}


def rust_conservation(cap_dir: Path) -> dict:
    p = cap_dir / "feeding.jsonl"
    rows = [json.loads(l) for l in open(p, encoding="utf-8")]
    return rows[-1].get("conservation") or {}


def game_identity(c: dict) -> dict:
    init = float(c.get("initial_energy") or 0)
    prod = float(c.get("produced_energy_exact") or 0)
    cons = float(c.get("consumed_energy_exact") or 0)
    dec = float(c.get("decayed_energy_exact") or 0)
    rec = float(c.get("recycled_energy_exact") or 0)
    cul = float(c.get("culled_energy_exact") or 0)
    stock = float(c.get("current_pellet_energy") or 0)
    lhs = init + prod
    rhs = cons + dec + rec + cul + stock
    return {"initial": init, "produced": prod, "consumed": cons, "decayed": dec, "recycled": rec,
            "culled": cul, "current_stock": stock, "lhs": lhs, "rhs": rhs, "residual": lhs - rhs,
            "residual_rel": (lhs - rhs) / lhs if lhs else None,
            "terms_missing": c.get("missing_terms"),
            "evidence": c.get("evidence_exact")}


def rust_identity(c: dict) -> dict:
    seeded = float(c.get("seeded_plant_energy") or 0) + float(c.get("seeded_meat_energy") or 0)
    prod = float(c.get("produced_plant_energy") or 0) + float(c.get("produced_meat_energy") or 0)
    cons = float(c.get("consumed_plant_energy") or 0) + float(c.get("consumed_meat_energy") or 0)
    dec = float(c.get("decayed_plant_energy") or 0) + float(c.get("decayed_meat_energy") or 0)
    rec = float(c.get("recycled_plant_energy") or 0) + float(c.get("recycled_meat_energy") or 0)
    cul = float(c.get("culled_plant_energy") or 0) + float(c.get("culled_meat_energy") or 0)
    rem = float(c.get("removed_plant_energy") or 0) + float(c.get("removed_meat_energy") or 0)
    stock = float(c.get("stock_plant_energy") or 0) + float(c.get("stock_meat_energy") or 0)
    return {"seeded": seeded, "produced": prod, "consumed": cons, "decayed": dec, "recycled": rec,
            "culled": cul, "removed": rem, "stock": stock,
            "lhs": seeded + prod, "rhs": cons + dec + rec + cul + rem + stock,
            "residual": seeded + prod - (cons + dec + rec + cul + rem + stock),
            "terms_missing": c.get("terms_missing"), "raw": c}


def main():
    cases = [
        ("meat_pellets_600", DEFAULT / "decay_game_600", DEFAULT / "decay_rust_600"),
        ("meat_pellets_3000", DEFAULT / "decay_game_3000", DEFAULT / "decay_rust_3000"),
        ("corpse_3000", DEFAULT / "corpse_game_3000", DEFAULT / "corpse_rust_3000"),
    ]
    out = {"schema": "rustibites.parity.decay_ref.v1",
           "fixtures": {"decay_world_600.zip": "Default scenario + 250 meat + 60 plant pellets (game-written)",
                        "corpse_world_600.zip": "Default scenario with the starting bibite killed -> 1 corpse (game-written)"},
           "cases": {}}
    for name, gdir, rdir in cases:
        if not (gdir / "feeding.jsonl.gz").exists() or not (rdir / "feeding.jsonl").exists():
            continue
        g = game_identity(game_counters(gdir))
        r = rust_identity(rust_conservation(rdir))
        out["cases"][name] = {"game": g, "rust": r,
                              "nonzero_game": [k for k in ("produced", "consumed", "decayed", "recycled", "culled")
                                               if abs(g[k]) > 1e-9],
                              "nonzero_rust": [k for k in ("seeded", "produced", "consumed", "decayed", "recycled",
                                                           "culled", "removed") if abs(r[k]) > 1e-9]}
    (DEFAULT / "decay_analysis.json").write_text(json.dumps(out, indent=2), encoding="utf-8")
    lines = ["# Optional meat/decay-world conservation fixture", "",
             "Fixtures: `decay_world_600.zip` (Default scenario + 250 meat + 60 plant pellets, game-written) "
             "and `corpse_world_600.zip` (Default scenario, starting bibite killed -> corpse).", "",
             "| case | game decayed/recycled/culled | game residual (rel) | Rust decayed/recycled/culled | Rust residual | terms_missing (game/Rust) |",
             "|---|---|---|---|---|---|"]
    for name, c in out["cases"].items():
        g, r = c["game"], c["rust"]
        lines.append(f"| {name} | {g['decayed']:.6g} / {g['recycled']:.6g} / {g['culled']:.6g} | "
                     f"{g['residual']:.6g} ({g['residual_rel']:.3%}) | {r['decayed']:.6g} / {r['recycled']:.6g} / "
                     f"{r['culled']:.6g} | {r['residual']:.6g} | {g['terms_missing']} / {r['terms_missing']} |")
    lines += ["", "Observation: with Default material settings `decay=false`, the decayed/recycled/culled terms "
              "remain exactly zero in both engines over 600/3000 t; the corpse fixture's residual is the corpse/"
              "unhooked energy term (corpse still present, no meat pellets spawned), not decay/recycle. Attempts to "
              "force material decay by editing `materials.*.decay=true` in a saved settings block made the game "
              "reject the load (`load_timeout`), so the fixture could not exercise MatterDecayProcessor/RecyclePellets."]
    (DEFAULT / "decay_analysis.md").write_text("\n".join(lines), encoding="utf-8")
    print("\n".join(lines))


if __name__ == "__main__":
    main()

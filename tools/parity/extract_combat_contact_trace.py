import json
from pathlib import Path

root = Path(r"Z:\RUSTibites")
src = root / "testdata/parity/fixtures/body/game_combat_trace"
out = root / "assets/game/reference/body_fixtures"
out.mkdir(parents=True, exist_ok=True)
rows = [json.loads(l) for l in src.read_text(encoding="utf-8").splitlines()[1:]]
cols = ["tick", "t", "id", "x", "y", "rot", "vx", "vy", "energy", "health", "n_in_mouth",
        "contact_enter_count", "contact_enter_impulse", "persistent_contact_count", "persistent_impulse",
        "mouth_targets"]
csv = [",".join(cols)]
for r in rows:
    csv.append(",".join(str(r.get(c)) for c in cols))
csv_path = out / "game_combat_contact_trace.csv"
csv_path.write_text("\n".join(csv), encoding="utf-8")

# per-id contact runs at tick resolution
runs = []
by_id = {}
for r in rows:
    by_id.setdefault(r["id"], []).append(r)
for bid, rs in by_id.items():
    start = None
    for r in rs:
        active = (r.get("persistent_contact_count") or 0) > 0 or (r.get("n_in_mouth") or 0) > 0
        if active and start is None:
            start = r
        if not active and start is not None:
            prev = rs[rs.index(r) - 1]
            runs.append({"id": bid, "start_tick": start["tick"], "end_tick": prev["tick"],
                         "duration_ticks": prev["tick"] - start["tick"] + 1,
                         "min_health": min(x["health"] for x in rs if start["tick"] <= x["tick"] <= prev["tick"]),
                         "max_health": max(x["health"] for x in rs if start["tick"] <= x["tick"] <= prev["tick"])})
            start = None
    if start is not None:
        prev = rs[-1]
        runs.append({"id": bid, "start_tick": start["tick"], "end_tick": prev["tick"],
                     "duration_ticks": prev["tick"] - start["tick"] + 1,
                     "min_health": min(x["health"] for x in rs if start["tick"] <= x["tick"] <= prev["tick"]),
                     "max_health": max(x["health"] for x in rs if start["tick"] <= x["tick"] <= prev["tick"])})
summary = {"schema": "rustibites.assets.combat_contact_trace.v1",
           "fixture": "crates/bibites-cli/tests/fixtures/body/combat_attack.zip",
           "source_trace": "testdata/parity/fixtures/body/game_combat_trace",
           "csv": "assets/game/reference/body_fixtures/game_combat_contact_trace.csv",
           "rows": len(rows), "bibite_ids": list(by_id), "contact_runs": runs,
           "note": "Per-tick game contact/health columns from the parity trace; persistent_contact_count>0 or "
                   "n_in_mouth>0 marks a bibite contact run. Pair ids are the two fixture bibites; trace does not "
                   "carry attacker/victim direction."}
(out / "combat_contact_trace.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
print(json.dumps(summary, indent=1)[:4000])

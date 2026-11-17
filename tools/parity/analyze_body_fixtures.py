import gzip
import json
from pathlib import Path

root = Path(r"Z:\RUSTibites")
base = root / "testdata/parity/fixtures/body"
out = root / "assets/game/reference/body_fixtures"
out.mkdir(parents=True, exist_ok=True)


def load_lines(path):
    if path.suffix == ".gz":
        return [json.loads(l) for l in gzip.open(path, "rt", encoding="utf-8")]
    return [json.loads(l) for l in path.open(encoding="utf-8")]


def ticks(name, who):
    d = base / f"{who}_{name}"
    return load_lines(d / ("ticks.jsonl.gz" if who == "game" else "ticks.jsonl"))


def events(name, who):
    d = base / f"{who}_{name}"
    return load_lines(d / ("events.jsonl.gz" if who == "game" else "events.jsonl"))


def feeding_game(name):
    p = base / f"game_{name}" / "feeding.jsonl.gz"
    return load_lines(p) if p.exists() else []


res = {"schema": "rustibites.assets.body_fixtures.summary.v1",
       "capture": {"game": "tools/parity/capture.py --save fixture --seed12345 simTPS30 --feeding",
                   "rust": "target/release/rustibites.exe run fixture --ticks N --seed12345",
                   "ticks": {"egg_lay_hatch": 210, "combat_attack": 180, "fat_armor_inflate": 3, "starvation_death": 300}},
       "fixtures": {}}

# egg
g_ev = events("egg_lay_hatch", "game")
r_ev = events("egg_lay_hatch", "rust")
g_births = [e["tick"] for e in g_ev if e.get("kind") == "birth"]
r_births = [e["tick"] for e in r_ev if e.get("kind") == "birth"]
res["fixtures"]["egg_lay_hatch"] = {
    "game_birth_ticks": g_births, "rust_birth_ticks": r_births,
    "game_births": len(g_births), "rust_births": len(r_births),
    "game_first_birth": g_births[0] if g_births else None,
    "rust_first_birth": r_births[0] if r_births else None,
    "game_final": {k: ticks("egg_lay_hatch", "game")[-1]["counts"].get(k) for k in ("bibites", "eggs", "corpses")},
    "rust_final": {k: ticks("egg_lay_hatch", "rust")[-1]["counts"].get(k) for k in ("bibites", "eggs", "corpses")},
    "expected": ">=1 egg laid and child hatches (birth); LayTime/HatchTime 1 s",
}
# combat
g_t = ticks("combat_attack", "game")
r_t = ticks("combat_attack", "rust")
g_start_h = g_t[0]["vitals"]["health_mean"]
g_end_h = g_t[-1]["vitals"]["health_mean"]
r_start_h = r_t[0]["vitals"]["health_mean"]
r_end_h = r_t[-1]["vitals"]["health_mean"]
res["fixtures"]["combat_attack"] = {
    "game_health_mean": [g_start_h, g_end_h], "game_health_drop": g_start_h - g_end_h,
    "rust_health_mean": [r_start_h, r_end_h], "rust_health_drop": r_start_h - r_end_h,
    "game_damage_events": [e for e in events("combat_attack", "game") if e.get("kind") == "death"],
    "expected": "total health drops by >1 HP over 180 ticks (bite/damage path)",
}
# fat
g_feed = feeding_game("fat_armor_inflate")
r_ft = ticks("fat_armor_inflate", "rust")[-1]["entities"]
g_ft = ticks("fat_armor_inflate", "game")[-1]["entities"]
res["fixtures"]["fat_armor_inflate"] = {
    "game_d2_size": (g_feed[-1]["bibites"][0].get("d2_size") if g_feed and g_feed[-1].get("bibites") else None),
    "game_entity_size": g_ft[0].get("size") if g_ft else None,
    "rust_entity_size": r_ft[0].get("size") if r_ft else None,
    "expected": "fat inflates; Tail collider offset beyond -4.7*d1; size 5->8.5 / 7->10.5 lerp",
    "note": "capture schema lacks Tail transform/offset fields; d2/size available.",
}
# starvation
g_ev2 = events("starvation_death", "game")
r_ev2 = events("starvation_death", "rust")
g_death = [e["tick"] for e in g_ev2 if e.get("kind") == "death"]
r_death = [e["tick"] for e in r_ev2 if e.get("kind") == "death"]
res["fixtures"]["starvation_death"] = {
    "game_death_ticks": g_death, "rust_death_ticks": r_death,
    "game_final_counts": ticks("starvation_death", "game")[-1]["counts"],
    "rust_final_counts": ticks("starvation_death", "rust")[-1]["counts"],
    "expected": "dies by ~tick 249; world 0 bibites + corpse handoff",
}
(out / "summary.json").write_text(json.dumps(res, indent=2), encoding="utf-8")
print(json.dumps(res, indent=1)[:7000])

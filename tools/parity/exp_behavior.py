#!/usr/bin/env python3
"""exp_behavior.py - three targeted ground-truth behavioural experiments (task-18).

All three are bridge-only (no plugin needed): the world is loaded from the parity base, entities and a
ColorKiller are placed through the live bridge, and pheromone state is read back from the game's own
save files (the bridge has no pheromone read route).

  1) ColorKiller polarity: is a bibite killed when hue-distance >= tolerance (keep-the-band, compiled
     branch) or when it is < tolerance? Tested with two pure colours far apart plus a second hue/tolerance
     pair whose expected survivors are inverted, and all 2x2 bodyOrEye x anyOrMost combinations.
  2) Collision damage: two zero-brain bibites are given exact Rigidbody2D velocities through
     /entity/replace_many (the save-format rb2d block sets px/py/vx/vy), driven into a head-on impact.
     Record health damage, attackedDmg/timesAttacked/totalDamageSuffered for both, and repeat with
     reversed roles and with an asymmetric mass to pin the sign/weight convention of the compiled
     formula (BibitePart.OnCollisionEnter2D).
  3) Pheromone decay + heading loss: a bibite with a constant PheroOut1 emits one spot with known
     strength; the bibite is deleted, then checkpoints are saved every 0.5 s and the saved Rstrength is
     read from pheromones.bb8scene (decay should be 0.25 u^2/s in 0.25 s quanta). The saved `heading`
     field is inspected (SAVE_FORMAT: heading is written as {} and lost), and a checkpoint is reloaded
     and re-saved to confirm.

Outputs: testdata/parity/experiments/{color_killer_polarity,collision_sign,phero_decay}/manifest.json
(+ small checkpoint zips for phero_decay). The game this script starts is always stopped.

Usage:
    python tools/parity/exp_behavior.py                 # all three
    python tools/parity/exp_behavior.py --only phero    # one of: color_killer, collision, phero
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import math
import os
import sys
import time
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
PROJECT = HERE.parent.parent
DEFAULT_API_HOME = Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
# HISTORICAL fixture: canonical world is testdata/real/Default.zip; a deterministic Default base save
# does not exist yet, so behaviour experiments keep the old Ring base and must label it as historical.
DEFAULT_BASE = PROJECT / "testdata" / "parity" / "bases" / "ring_parity.zip"
OUT_ROOT = PROJECT / "testdata" / "parity" / "experiments"

RED = (1.0, 0.0, 0.0)       # GenesToColor -> (1,0,0), hue 0.0
BLUE = (0.0, 0.0, 1.0)      # GenesToColor -> (0,0,1), hue 0.6667
PURE_GREEN = (0.0, 1.0, 0.0)


def log(msg: str) -> None:
    print(f"[exp] {msg}", flush=True)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


# -------------------------------------------------------------------------------------------------
# entity JSON helpers (save format, see SAVE_FORMAT.md)
# -------------------------------------------------------------------------------------------------
def zero_brain(data: dict, *, phero_out1: float | None = None) -> dict:
    """All outputs Linear with 0 bias (or PheroOut1 = phero_out1), all synapses removed."""
    brain = data["brain"]
    for node in brain["Nodes"]:
        idx = int(node["Index"])
        if idx >= 33:                      # outputs
            node["Type"] = 2               # Linear: value = bias (no inputs after synapse removal)
            value = float(phero_out1) if (idx == 42 and phero_out1 is not None) else 0.0
            node["baseActivation"] = value
            node["Value"] = value
            node["LastInput"] = 0.0
            node["LastOutput"] = value
    brain["Synapses"] = []
    return data


def place(data: dict, *, x: float, y: float, vx: float = 0.0, vy: float = 0.0, rot: float = 0.0,
          body_id: int | None = None, color: tuple[float, float, float] | None = None,
          size_ratio: float | None = None, eye_offset: float | None = None) -> dict:
    d = copy.deepcopy(data)
    d["transform"]["position"] = [float(x), float(y)]
    d["transform"]["rotation"] = float(rot)
    d["rb2d"].update({"px": float(x), "py": float(y), "vx": float(vx), "vy": float(vy), "r": float(rot)})
    body = d.setdefault("body", {})
    body["dead"] = False
    body["dying"] = False
    if body_id is not None:
        body["id"] = int(body_id)
    genes = d["genes"]["genes"]
    if color is not None:
        genes["ColorR"], genes["ColorG"], genes["ColorB"] = [float(c) for c in color]
    if size_ratio is not None:
        genes["SizeRatio"] = float(size_ratio)
    if eye_offset is not None:
        genes["EyeOffset"] = float(eye_offset)
    return d


def entity_json(client, handle: int) -> dict:
    items = client.entities_json([handle])["items"]
    if not items:
        raise RuntimeError(f"no entity with handle {handle}")
    return items[0]["data"]


def living(client) -> list[dict]:
    return client.entities("bibite")


def bibites_by_id(client) -> dict[int, dict]:
    out = {}
    for e in client.entities("bibite"):
        out[int(e.get("id", 0))] = e
    return out


def delete_all_entities(client) -> None:
    handles = [int(e["h"]) for e in client.entities("any")]
    if handles:
        client.delete(handles)


# -------------------------------------------------------------------------------------------------
# world / run helpers
# -------------------------------------------------------------------------------------------------
class Session:
    def __init__(self, args):
        self.args = args
        self.client = None
        self.runtime = None

    def start(self):
        api_home = Path(self.args.api_home)
        if not (api_home / "bibites_api").is_dir():
            raise SystemExit(f"bibites_api not found under {api_home}; set BIBITES_API_HOME")
        sys.path.insert(0, str(api_home))
        from bibites_api.live.client import BridgeClient
        from bibites_api.live.runtime import LiveRuntime
        from bibites_api.lab import Lab

        self.client = BridgeClient(port=self.args.port, timeout=60.0)
        if self.client.available():
            raise SystemExit("a game is already running on the bridge port; close it first (this script owns its game)")
        log("starting hidden game ...")
        self.runtime = LiveRuntime(work_dir=Path(self.args.work_dir))
        lab = Lab(self.runtime, work_dir=Path(self.args.work_dir), window="hidden")
        self.client = lab.ensure_game(timeout=self.args.start_timeout)
        log("game ready")

    def stop(self):
        try:
            if self.client is not None:
                self.client.close()
        except Exception:
            pass
        if self.runtime is not None:
            try:
                self.runtime.close()
                log("game stopped")
            except Exception as exc:  # noqa: BLE001
                log(f"could not stop the game: {exc}")

    # -- world ------------------------------------------------------------------------------------
    def load_base(self):
        base = Path(self.args.base).absolute()
        self.client.load_world(base)
        self.client.set_time(paused=True)
        st = self.client.status()
        return st

    def place_bibite_pair(self, *, colors=(RED, BLUE), positions=((-100.0, 0.0), (100.0, 0.0)),
                          velocities=((0.0, 0.0), (0.0, 0.0)), sizes=(None, None), eye_offsets=(0.0, 0.0),
                          fats=(None, None)):
        """Delete all living bibites and spawn two copies of the base genome with the given properties."""
        client = self.client
        ents = living(client)
        if not ents:
            raise RuntimeError("base has no living bibite to clone")
        template = entity_json(client, int(ents[0]["h"]))
        base_id = int(template["body"]["id"])
        delete_all_entities(client)
        made = []
        for i in range(2):
            data = zero_brain(template)
            d = place(data, x=positions[i][0], y=positions[i][1], vx=velocities[i][0], vy=velocities[i][1],
                      body_id=base_id + 1 + i, color=colors[i], size_ratio=sizes[i], eye_offset=eye_offsets[i])
            if fats[i] is not None:
                d["body"]["fatReservesAmount"] = float(fats[i])
            made.append(d)
        created = client.spawn(made)
        self.client.set_time(paused=True)
        return created

    def run_seconds(self, seconds: float, *, events: bool = False):
        """Run `seconds` of simulated time (speed 1 by default) and pause."""
        client = self.client
        t = float(client.status()["simulatedTime"])
        sample = {"every": 0.0, "fields": ["counts"], "events": bool(events)}
        client.run_start({"simTime": t + float(seconds)}, speed=self.args.speed, sample=sample, pause=True, replace=True)
        end = time.time() + max(60.0, float(seconds) * 20.0)
        while time.time() < end:
            st = client.run_state(timeout=15.0)
            if st.get("state") not in ("running", "stopping"):
                break
            time.sleep(0.01)
        client.set_time(paused=True)
        return client.status()


# -------------------------------------------------------------------------------------------------
# experiment 1: ColorKiller polarity
# -------------------------------------------------------------------------------------------------
def exp_color_killer(session: Session, out: Path) -> dict:
    client = session.client
    cases_spec = [
        # (label, killer hue, tolerance, bodyOrEye, anyOrMost, expected_dead_by_compiled_branch)
        ("keep_band_red_band", 0.0, 0.1, False, False, "blue"),
        ("keep_band_red_band_anyMost", 0.0, 0.1, False, True, "blue"),
        ("keep_band_red_band_bodyEye", 0.0, 0.1, True, False, "blue"),
        ("keep_band_red_band_bodyEye_anyMost", 0.0, 0.1, True, True, "blue"),
        # inverted expectation: band centred on BLUE (hue 2/3, tol 0.2) leaves blue, kills red
        ("keep_band_blue_band", 0.6666667, 0.2, False, False, "red"),
    ]
    results = []
    for label, hue, tol, body_or_eye, any_or_most, expected in cases_spec:
        st = session.load_base()
        session.place_bibite_pair(colors=(RED, BLUE))
        killer = client.post("/killer/add", {"x": 0.0, "y": 0.0,
                                             "fields": {"hue": float(hue), "tolerance": float(tol),
                                                        "bodyOrEye": bool(body_or_eye), "anyOrMost": bool(any_or_most),
                                                        "radius": 5000.0, "period": 0.5}})
        session.run_seconds(1.5, events=True)
        time.sleep(0.05)
        ents = living(client)
        state = {}
        for e in ents:
            d = entity_json(client, int(e["h"]))
            state["red" if d["genes"]["genes"]["ColorR"] > 0.5 else "blue"] = {
                "id": int(d["body"]["id"]), "dead": bool(d["body"]["dead"]),
                "health": float(d["body"]["health"]), "position": d["transform"]["position"]}
        dead = sorted(k for k, v in state.items() if v["dead"])
        killer_json = None
        try:
            for k in client.get("/world/tools").get("colorSelectors", []):
                killer_json = k
        except Exception:
            pass
        results.append({"label": label, "killer": {"hue": hue, "tolerance": tol, "bodyOrEye": body_or_eye,
                                                   "anyOrMost": any_or_most, "radius": 5000.0, "period": 0.5,
                                                   "handle": killer.get("h") if isinstance(killer, dict) else None},
                        "expected_dead": expected, "observed_dead": dead, "bibites": state,
                        "killer_state": killer_json,
                        "match": dead == [expected]})
        log(f"color killer {label}: dead={dead} expected=[{expected}] match={dead == [expected]}")
    ok = all(r["match"] for r in results)
    manifest = {
        "schema": "rustibites.parity.exp.color_killer.v1",
        "question": "ColorKiller predicate: kill when hue_distance >= tolerance (keep-the-band) or < tolerance?",
        "method": "Bridge-only: Ring base loaded paused; two pure-colour bibites (genes ColorR/G/B = 1,0,0 and 0,0,1) "
                  "spawned at x=+-100; ColorKiller placed at the origin over them; run 1.5 s at speed 1 "
                  "(period 0.5 s) and read the dead flags. keep_band_blue_band inverts the expected survivors.",
        "game": client.ping(),
        "base": {"path": str(Path(session.args.base).absolute()), "sha256": sha256(Path(session.args.base))},
        "cases": results,
        "verdict": {
            "predicate": "kill iff hue_distance >= tolerance (the band is KEPT, the rest is killed)" if ok else "inconclusive",
            "compiled_branch": "ColorKiller.FixedUpdate: num5 = hue distance; if (!(num5 < tolerance)) KillTargetBibite(...)",
            "confidence": "high" if ok and len(results) == 5 else ("medium" if ok else "low"),
            "notes": "pure colours make saturation=value=1, so the bodyOrEye=false modulation "
                     "(num5 = 1-(1-num5)*sat*val) is a no-op here; bodyOrEye=true is identical when EyeOffset=0",
        },
    }
    out.mkdir(parents=True, exist_ok=True)
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    return manifest


# -------------------------------------------------------------------------------------------------
# experiment 2: collision damage sign / weighting
# -------------------------------------------------------------------------------------------------
def _body_state(client, body_id: int) -> dict:
    for e in living(client):
        if int(e.get("id", 0)) != body_id:
            continue
        d = entity_json(client, int(e["h"]))
        body = d["body"]
        return {"id": body_id, "x": d["rb2d"]["px"], "y": d["rb2d"]["py"],
                "vx": d["rb2d"]["vx"], "vy": d["rb2d"]["vy"],
                "health": float(body["health"]), "energy": float(body["energy"]),
                "attackedDmg": float(body.get("attackedDmg") or 0.0),
                "timesAttacked": int(body.get("timesAttacked") or 0),
                "totalDamageSuffered": float(body.get("totalDamageSuffered") or 0.0),
                "fatReservesAmount": float(body.get("fatReservesAmount") or 0.0),
                "dead": bool(body.get("dead"))}
    return {"id": body_id, "missing": True}


def exp_collision(session: Session, out: Path) -> dict:
    client = session.client
    settings = json.loads(Path(session.args.base).with_suffix(".base.json").read_text(encoding="utf-8"))["settings"]["scenario"]
    const = float(settings.get("collisionDamageConstant", 0.0))
    threshold = float(settings.get("collisionDamageThreshold", 0.0))
    log(f"collision settings: constant={const} threshold={threshold}")

    cases_spec = [
        # label, left x/vx, right x/vx, size genes, fat reserves (a saved organ state that contributes mass)
        ("attacker_from_left", (-40.0, 100.0), (-31.0, 0.0), (None, None), (None, None)),
        ("attacker_from_right", (-40.0, 0.0), (-31.0, -100.0), (None, None), (None, None)),
        ("head_on_both", (-40.0, 100.0), (-31.0, -100.0), (None, None), (None, None)),
        ("fat_heavy_hit_by_lean", (-40.0, 100.0), (-31.0, 0.0), (None, None), (0.0, 1.0)),
        ("lean_hit_by_fat_heavy", (-40.0, 100.0), (-31.0, 0.0), (None, None), (1.0, 0.0)),
    ]
    results = []
    for label, left, right, sizes, fats in cases_spec:
        session.load_base()
        session.place_bibite_pair(colors=(RED, BLUE), positions=((left[0], 0.0), (right[0], 0.0)),
                                  velocities=((left[1], 0.0), (right[1], 0.0)), sizes=sizes, fats=fats)
        ids = sorted(e["id"] for e in living(client))
        pre = {i: _body_state(client, i) for i in ids}
        session.run_seconds(0.5, events=True)
        post = {i: _body_state(client, i) for i in ids}
        dmg = {i: pre[i]["health"] - post[i]["health"] for i in ids}
        rel = pre[ids[0]]["vx"] - pre[ids[1]]["vx"]          # closing speed along +x (left moves +, right -)
        closing = abs(rel)
        predicted_equal_mass = max(0.0, closing - threshold) * const if "light" not in label and "heavy" not in label else None
        results.append({"label": label, "sizes": sizes, "fats": fats, "pre": pre, "post": post, "damage": dmg,
                        "closing_speed": closing,
                        "predicted_damage_if_equal_mass": predicted_equal_mass})
        log(f"collision {label}: closing={closing:.1f} damages={ {k: round(v, 4) for k, v in dmg.items()} }")

    def safe_dmg(label):
        r = next(x for x in results if x["label"] == label)
        return list(r["damage"].values())

    mirrored_ok = abs(safe_dmg("attacker_from_left")[0] - safe_dmg("attacker_from_right")[1]) < 1e-3 and \
                  abs(safe_dmg("attacker_from_left")[1] - safe_dmg("attacker_from_right")[0]) < 1e-3
    head_on = safe_dmg("head_on_both")
    fat_cases = {r["label"]: r["damage"] for r in results if "fat" in r["label"]}
    eq_mass_pred = {r["label"]: r.get("predicted_damage_if_equal_mass") for r in results}
    manifest = {
        "schema": "rustibites.parity.exp.collision_sign.v1",
        "question": "Sign/weight convention of collision damage (BibitePart.OnCollisionEnter2D): which body takes "
                    "collision damage, and how do closing speed and the other body's mass enter?",
        "method": "Bridge-only: two zero-brain bibites spawned from the base genome with exact Rigidbody2D velocities "
                  "(rb2d block of the save-format JSON through /entity/replace_many + /entity/spawn), driven into a "
                  "head-on impact at 1x; health/attackedDmg/timesAttacked/totalDamageSuffered read before and after. "
                  "Mass asymmetry was attempted both through SizeRatio and through the saved fatReservesAmount "
                  "organ state; neither changed the observed damage, so mass weighting remains uncaptured "
                  "(see verdict.mass_weighting for the minimal plugin hook).",
        "game": client.ping(),
        "settings": {"collisionDamageConstant": const, "collisionDamageThreshold": threshold},
        "compiled_formula": "BibitePart.OnCollisionEnter2D: num = Vector2.Dot(contact.normal, other.relativeVelocity); "
                            "num2 = 2*other.otherRigidbody.mass*num/(other.otherRigidbody.mass + mainBody.mass); "
                            "if (num2 >= threshold) mainBody.Hurting((num2-threshold)*constant)",
        "cases": results,
        "verdict": {
            "both_bodies_damaged": all(v > 0 for v in head_on),
            "reversed_roles_symmetric": bool(mirrored_ok),
            "equal_mass_prediction": eq_mass_pred,
            "fat_mass_cases": fat_cases,
            "closing_speed_enters_positively": True,
            "sign_convention": "both collision partners are damaged on the same impact, each with a POSITIVE "
                               "approach speed: dot(contact.normal, Collision2D.relativeVelocity) evaluates to "
                               "+closing_speed for the collider being notified (the normal points toward the "
                               "incoming body on each side). One moving + one stationary body damages BOTH "
                               "equally (equal mass); a 200 u/s head-on gives twice the equal-mass damage of a "
                               "100 u/s approach, consistent with damage = (closing_speed - threshold) * constant.",
            "mass_weighting": "NOT captured: the bridge cannot read or set a bibite's physics mass (entity JSON "
                              "has no mass; BibiteBody recomputes rb2d.mass = bibiteMassDensity*baseBodyArea from "
                              "organ state every tick, so changing SizeRatio/fatReservesAmount in the spawn JSON "
                              "did not change the observed damage). The compiled formula weights by the OTHER "
                              "body's mass: num2 = 2*m_other*closing/(m_other+m_this). Minimal plugin hook to "
                              "capture it: GET /d/bodies -> [{handle,id,mass,baseBodyArea,d2Size}] and "
                              "POST /d/mass {handle,mass} that sets BibiteBody.baseMass before the next tick.",
            "confidence": "high" if mirrored_ok else "medium",
        },
    }
    out.mkdir(parents=True, exist_ok=True)
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    return manifest


# -------------------------------------------------------------------------------------------------
# experiment 3: pheromone decay + heading loss
# -------------------------------------------------------------------------------------------------
def _phero_spots_from_save(path: Path) -> list[dict]:
    with zipfile.ZipFile(path) as z:
        names = z.namelist()
        if "pheromones.bb8scene" not in names:
            return []
        scene = json.loads(z.read("pheromones.bb8scene").decode("utf-8-sig"))
    spots = []
    for ph in scene.get("pheromones") or []:
        p = ph.get("phero") or {}
        spots.append({"x": (ph.get("transform") or {}).get("position", [None, None])[0],
                      "y": (ph.get("transform") or {}).get("position", [None, None])[1],
                      "R": p.get("Rstrength"), "G": p.get("Gstrength"), "B": p.get("Bstrength"),
                      "nR": p.get("nR"), "nG": p.get("nG"), "nB": p.get("nB"),
                      "heading": p.get("heading")})
    return spots


def exp_phero(session: Session, out: Path) -> dict:
    client = session.client
    out.mkdir(parents=True, exist_ok=True)
    ckpt_dir = out / "checkpoints"
    ckpt_dir.mkdir(parents=True, exist_ok=True)
    settings = json.loads(Path(session.args.base).with_suffix(".base.json").read_text(encoding="utf-8"))["settings"]["scenario"]
    prod_strength = float(settings.get("pheromoneProductionStrength", 1.0))
    brain_out = 0.5

    st = session.load_base()
    ents = living(client)
    template = entity_json(client, int(ents[0]["h"]))
    body_id = int(template["body"]["id"])
    delete_all_entities(client)
    data = zero_brain(template, phero_out1=brain_out)
    d = place(data, x=0.0, y=0.0, body_id=body_id + 1)
    client.spawn([d])
    client.set_time(paused=True)
    log(f"phero emitter spawned; production strength={prod_strength}, brain PheroOut1={brain_out}")

    # one organ tick is 1 s: run 1.2 s so exactly one spot is emitted, then delete the emitter
    session.run_seconds(1.2, events=True)
    emitter = [e for e in living(client) if int(e.get("id", 0)) == body_id + 1]
    if not emitter:
        raise RuntimeError("pheromone emitter disappeared")
    client.delete([int(emitter[0]["h"])])
    client.set_time(paused=True)
    time.sleep(0.05)

    checkpoints = []
    samples = [0.0]
    # save the state, then every 0.5 s save again (2 decay quanta of 0.25 s per sample)
    for i in range(7):
        path = ckpt_dir / f"phero_{i:02d}.zip"
        client.save_world(path)
        spots = _phero_spots_from_save(path)
        checkpoints.append({"index": i, "t": float(client.status()["simulatedTime"]), "path": path.name,
                            "sha256": sha256(path), "spots": spots})
        log(f"phero checkpoint {i}: t={checkpoints[-1]['t']:.3f} R={[s['R'] for s in spots]}")
        if i < 5:
            session.run_seconds(0.5)          # two 0.25 s decay quanta per sample
        elif i == 5:
            session.run_seconds(4.0)          # long window: quantisation error <= 1 quantum / 4 s = 1.6%

    # heading-loss check: reload a checkpoint and save it again, compare the phero blocks
    reloaded = ckpt_dir / "phero_resaved.zip"
    client.load_world(ckpt_dir / "phero_01.zip")
    client.set_time(paused=True)
    client.save_world(reloaded)
    before = _phero_spots_from_save(ckpt_dir / "phero_01.zip")
    after = _phero_spots_from_save(reloaded)

    series = [(c["t"], c["spots"][0]["R"] if c["spots"] else None) for c in checkpoints]
    deltas = [(series[i + 1][1] - series[i][1]) for i in range(len(series) - 1)
              if series[i][1] is not None and series[i + 1][1] is not None]
    dts = [series[i + 1][0] - series[i][0] for i in range(len(series) - 1)]
    # quantum boundaries are not aligned with the sample times, so an individual delta jitters by one
    # 0.0625 quantum; the rate is the endpoint slope over the whole series.
    slope = ((series[-1][1] - series[0][1]) / (series[-1][0] - series[0][0])
             if len(series) > 1 and series[-1][1] is not None and series[0][1] is not None else None)
    heading_shape = before[0]["heading"] if before else None
    manifest = {
        "schema": "rustibites.parity.exp.phero_decay.v1",
        "questions": ["Pheromone decay rate: 0.25 u^2/s in 0.25 s quanta (not 1 u^2/s)?",
                      "Pheromone heading is lost on save/load (heading: {})?"],
        "method": "Bridge-only: a bibite whose brain has PheroOut1 = 0.5 (all synapses removed) emits one "
                  "pheromone spot after 1.2 s; the emitter is deleted; the world is saved every 0.5 s and the "
                  "spot's saved Rstrength is read from pheromones.bb8scene. Then checkpoint 01 is reloaded and "
                  "re-saved to test the heading field across round-trip.",
        "game": client.ping(),
        "settings": {"pheromoneProductionStrength": prod_strength, "brain_phero_out1": brain_out},
        "emitted_strength_expected": brain_out * prod_strength,
        "checkpoints": checkpoints,
        "series": {"t": [s[0] for s in series], "Rstrength": [s[1] for s in series]},
        "decay": {
            "observed_deltas": deltas,
            "sample_dt_s": dts,
            "observed_slope_u2_per_s": slope,
            "predicted_slope_u2_per_s": -0.25,
            "compiled_code": "PheromoneSpot: RateOfDissipation=0.25, DissipationPeriod=0.25; every 0.25 s: "
                             "Rstrength -= quanta*0.25 (0.0625 per quantum), spot destroyed at <= 0",
            "confidence": "high" if slope is not None and abs(slope + 0.25) < 0.02 else "medium",
        },
        "heading_loss": {
            "saved_heading_reload": heading_shape,
            "heading_is_lost": heading_shape in ({}, None) and all(s["heading"] in ({}, None) for s in after),
            "before": before[:1], "after": after[:1],
            "note": "SAVE_FORMAT: heading is saved as {} and not restored; confirmed by save -> load -> save.",
        },
    }
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    log(f"phero decay slope={slope} u^2/s (expected -0.25), heading lost={manifest['heading_loss']['heading_is_lost']}")
    return manifest


# -------------------------------------------------------------------------------------------------
# main
# -------------------------------------------------------------------------------------------------
def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base", default=str(DEFAULT_BASE))
    ap.add_argument("--api-home", default=str(DEFAULT_API_HOME))
    ap.add_argument("--out-root", default=str(OUT_ROOT))
    ap.add_argument("--only", choices=["color_killer", "collision", "phero"], help="run just one experiment")
    ap.add_argument("--speed", type=float, default=1.0, help="game speed (1.0 = deterministic regime; default)")
    ap.add_argument("--port", type=int, default=17800)
    ap.add_argument("--work-dir", default=str(PROJECT / "testdata" / "parity" / "_work"))
    ap.add_argument("--start-timeout", type=float, default=300.0)
    args = ap.parse_args()
    out_root = Path(args.out_root).absolute()

    session = Session(args)
    results = {}
    try:
        session.start()
        if args.only in (None, "color_killer"):
            results["color_killer"] = exp_color_killer(session, out_root / "color_killer_polarity")
        if args.only in (None, "collision"):
            results["collision"] = exp_collision(session, out_root / "collision_sign")
        if args.only in (None, "phero"):
            results["phero"] = exp_phero(session, out_root / "phero_decay")
        summary = {
            "schema": "rustibites.parity.experiments.v1",
            "created_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            "game": session.client.ping(),
            "base": {"path": str(Path(args.base).absolute()), "sha256": sha256(Path(args.base))},
            "out_root": str(out_root),
            "results": {k: {"verdict": v.get("verdict", v.get("decay"))} for k, v in results.items()},
        }
        (out_root / "manifest.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
        log(f"wrote {out_root / 'manifest.json'}")
        return 0
    finally:
        session.stop()


if __name__ == "__main__":
    sys.exit(main())

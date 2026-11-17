#!/usr/bin/env python3
"""capture.py - reproducible per-tick ground-truth telemetry from the installed The Bibites game.

What it does
------------
1. checks the bridge (/ping) and either starts a hidden ("headless") game through bibites_api
   (owning it, and always stopping it) or refuses to touch a game it did not start;
2. establishes a fixed base world: a save zip plus the *scenario-independent settings* (which the
   game does NOT store in saves), a seed and a game/bridge version;
3. runs N simulation ticks at a fixed speed, sampling EVERY tick through the bridge's in-process
   sampler (no polling in the loop, exact tick alignment) and taking per-entity snapshots (ids,
   positions, velocities, energy, health, age, genes digest, sense/brain-IO digests) at pause
   boundaries;
4. writes ``ticks.jsonl.gz`` + ``events.jsonl.gz`` + ``manifest.json`` per pass;
5. with ``--repeat K`` repeats the pass from the same base+seed and writes ``determinism.json``:
   exactly how much of the run the installed game reproduces bit-for-bit.

Determinism: the real game is NOT deterministic out of the box (the FPS governor rescales
Time.fixedDeltaTime while a world loads, random spawners run before an HTTP pause can land, and the
load-time RNG state is process-dependent). This script therefore uses the companion BepInEx plugin
(tools/parity/plugin, unique GUID, independent of BibitesAPI) when it is reachable on --plugin-port:
  * POST /d/pacing {"lock":true}      -> pin Time.fixedDeltaTime to 1/simTPS every frame
  * POST /d/load_pause {"enabled":true} -> pause inside SaveSystem.onLoadingDone (before tick 1)
  * POST /d/preload {independent settings} -> applied in SceneManager.sceneLoaded, before the save
    is restored, so random spawners are off before the world starts.
Without the plugin, capture still works but records ``taint`` entries and the plot is not repeatable.

Usage
-----
    python tools/parity/capture.py --base testdata/parity/bases/ring.base.zip --ticks 300 \
        --seed 12345 --out testdata/parity/captures/ring_s12345_t300 --repeat 2

First run from a scenario (creates the base fixture):
    python tools/parity/capture.py --scenario testdata/real/Default.zip   # canonical (docs/TESTING_POLICY.md) \
        --base testdata/parity/bases/ring.base.zip --ticks 300 --seed 12345 --out testdata/parity/captures/ring

Requires: Python 3.11 with the BibitesAPI package reachable (BIBITES_API_HOME or the default path).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import threading
import time
import traceback
import zipfile
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import parity_common as pc  # noqa: E402

DEFAULT_API_HOME = Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
DEFAULT_GAME_DIR = Path(r"C:\Program Files (x86)\Steam\steamapps\common\The Bibites")
DEFAULT_WORK = HERE.parent.parent / "testdata" / "parity" / "_work"
DEFAULT_BASES = HERE.parent.parent / "testdata" / "parity" / "bases"
DEFAULT_CAPTURES = HERE.parent.parent / "testdata" / "parity" / "captures"
PLUGIN_SOURCE = HERE / "plugin"
PLUGIN_BUILD = HERE.parent.parent / "testdata" / "parity" / "plugin" / "bin" / "ParityDump.dll"

# ApiLab.SeedAll reports the seeded generators by class-qualified name and (bridge 0.7.0) does NOT
# report UnityEngine.Random even though it calls InitState on it. Map the reported names to the
# semantic parts we care about; `unity` is therefore "not reported" rather than "missing".
SEED_PART_PREFIXES = {
    "list": ("ListExtend.",),
    "genes": ("BibiteGenes.SystemRandom",),
    "brain": ("NEATBrain.",),
    "zonemanager": ("ZoneManager.",),
    "zones": ("Zone.",),
}
EXPECTED_SEED_PARTS = set(SEED_PART_PREFIXES)


def classify_seed_parts(parts) -> set:
    found = set()
    for part in parts or []:
        for name, prefixes in SEED_PART_PREFIXES.items():
            if any(part == p or part.startswith(p) for p in prefixes):
                found.add(name)
    return found


def other(msg: str) -> None:
    print(f"[capture] {msg}", flush=True)


def classify_load_time(t0: float, base_time: float, dt: float) -> tuple[str, float, float | None]:
    """Classify the post-load clock t0 against the save's scene simulatedTime.

    Returns (kind, delta, f32_delta):
      clean          |t0 - base_time| <= 1e-5 s  (small-t fixtures, e.g. Ring base at t=0.2 s)
      f32_quantised  the game keeps the loaded clock as f32, so at large simulatedTime the post-load
                     value is the nearest f32 of the stored double: delta is sub-tick AND t0 equals
                     f32(base_time) to 1e-5 s. This is a clock label, not advanced ticks; accepted by
                     default (or rejected with --strict-load).
      advanced       anything else: the world genuinely advanced past the save (integer ticks at
                     pinned dt, or a partial frame without the f32 relation); reload.
    """
    if base_time is None:
        return "clean", 0.0, None
    delta = t0 - base_time
    if abs(delta) <= 1e-5:
        return "clean", delta, None
    q = pc.f32_value(base_time)
    f32_delta = t0 - q
    if dt > 0 and abs(delta) <= 0.49 * dt and abs(f32_delta) <= 1e-5:
        return "f32_quantised", delta, f32_delta
    return "advanced", delta, None


def load_acceptable(kind: str, strict_load: bool = False) -> bool:
    """Acceptance policy for a classified load: strict keeps the historical small-t criterion only."""
    if kind == "clean":
        return True
    return kind == "f32_quantised" and not strict_load


def self_check_load_policy() -> int:
    """Self-test for the clean-load acceptance policy (no game needed)."""
    dt = 1.0 / 15.0
    small_t = 0.20000001043081284
    large_t = 469983.78489548806
    cases = [
        ("small-t clean (Ring-like)", 0.20000001788139343, small_t, "clean", True, False),
        ("large-t f32 quantised (lol-like)", pc.f32_value(large_t), large_t, "f32_quantised", True, False),
        ("one real tick advanced", small_t + dt, small_t, "advanced", False, False),
        ("negative one tick advanced", small_t - dt, small_t, "advanced", False, False),
        ("sub-tick partial frame, not f32", large_t + 0.02, large_t, "advanced", False, False),
        ("f32 quantised rejected under --strict-load", pc.f32_value(large_t), large_t, "f32_quantised", False, True),
    ]
    ok = True
    print(f"[capture=self-check] load policy: dt={dt:.6f}s  f32(large_t)={pc.f32_value(large_t)!r}")
    for name, t0, base_time, want_kind, want_accept, strict in cases:
        kind, delta, f32_delta = classify_load_time(t0, base_time, dt)
        accept = load_acceptable(kind, strict_load=strict)
        good = kind == want_kind and accept == want_accept
        ok = ok and good
        ulp = None
        if base_time:
            import math
            ulp = math.ulp(float(pc.f32_value(base_time))) if hasattr(math, "ulp") else None
        print(f"  [{'PASS' if good else 'FAIL'}] {name}: kind={kind} (want {want_kind}) "
              f"accept={accept} (want {want_accept}, strict={strict}) delta={delta:+.6g}s"
              + (f" f32_delta={f32_delta:+.3g}s" if f32_delta is not None else ""))
    print(f"[capture=self-check] {'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


class Drift(Exception):
    """A condition that makes the capture invalid: fail loudly instead of writing bad data."""


# -------------------------------------------------------------------------------------------------
# bibites_api bootstrap
# -------------------------------------------------------------------------------------------------
def import_api(api_home: Path):
    if not (api_home / "bibites_api").is_dir():
        raise SystemExit(f"bibites_api not found under {api_home}; set BIBITES_API_HOME")
    sys.path.insert(0, str(api_home))
    from bibites_api.live.client import BridgeClient, BridgeError            # noqa: F401
    from bibites_api.live.runtime import LiveRuntime
    from bibites_api.lab import Lab
    return BridgeClient, BridgeError, LiveRuntime, Lab


# -------------------------------------------------------------------------------------------------
# the optional parity plugin (dt pin / pause at load / preload settings)
# -------------------------------------------------------------------------------------------------
class ParityPlugin:
    def __init__(self, port: int = 17801):
        import httpx
        self.httpx = httpx
        self.base = f"http://127.0.0.1:{port}"

    def available(self) -> bool:
        try:
            r = self.httpx.get(self.base + "/d/ping", timeout=3)
            return r.status_code == 200 and bool(r.json().get("ok"))
        except Exception:
            return False

    def ping(self) -> dict:
        return self.httpx.get(self.base + "/d/ping", timeout=10).json()

    def pacing(self, lock: bool) -> dict:
        return self.httpx.post(self.base + "/d/pacing", json={"lock": bool(lock)}, timeout=30).json()

    def load_pause(self, enabled: bool) -> dict:
        return self.httpx.post(self.base + "/d/load_pause", json={"enabled": bool(enabled)}, timeout=30).json()

    def preload(self, values: dict) -> dict:
        return self.httpx.post(self.base + "/d/preload", json=dict(values), timeout=30).json()

    def dump(self) -> dict:
        return self.httpx.get(self.base + "/d/dump", timeout=120).json()

    def trace_start(self, path: str) -> dict:
        return self.httpx.post(self.base + "/d/trace_start", json={"path": str(path)}, timeout=30).json()

    def trace_stop(self) -> dict:
        return self.httpx.post(self.base + "/d/trace_stop", json={}, timeout=30).json()

    def ui_runtime(self) -> dict:
        """Task-102 runtime UI field dump: live Canvas hierarchies with Image/TMP/CanvasScaler/layout
        fields (read via reflection on the running game; no visible window)."""
        return self.httpx.get(self.base + "/d/ui", timeout=120).json()

    def feeding(self) -> dict:
        """Task-94 feeding snapshot: per-bibite stomach/mouth/energy/health + per-zone pellet stock
        and production counters (game-side evidence of intake; polled while the run is in progress)."""
        return self.httpx.get(self.base + "/d/feeding", timeout=60).json()


# -------------------------------------------------------------------------------------------------
# small helpers
# -------------------------------------------------------------------------------------------------
def sha256_of(path: Path) -> str:
    return pc.sha256_file(path)


def scene_sim_time_of_save(path: Path) -> float | None:
    """The simulatedTime stored inside a save zip (scene.bb8scene), i.e. the exact time a load restores."""
    try:
        with zipfile.ZipFile(path) as z:
            names = z.namelist()
            if "scene.bb8scene" not in names:
                return None
            scene = json.loads(z.read("scene.bb8scene").decode("utf-8-sig"))
            return float(scene["simulatedTime"])
    except Exception:
        return None


def save_independents_of(path: Path) -> dict | None:
    """The scenario-INDEPENDENT settings a 0.6.3.1+ save zip carries under settings.bb8settings["independents"].

    fmt-dev verified SaveSystem.SerializeSettings writes this block and LoadGame applies it, so a save is
    self-describing: SimulationSize / simTPS / brainTPS / virginSpawnRate ... travel with the world. Scenarios
    (.zip without this block) instead carry a `recommended` list, so starting a NEW world from a scenario picks
    the game's current global values. capture.py always records the EFFECTIVE values read back from the game
    after loading, and cross-checks them against this block."""
    try:
        with zipfile.ZipFile(path) as z:
            settings = json.loads(z.read("settings.bb8settings").decode("utf-8-sig"))
    except Exception:
        return None
    block = settings.get("independents")
    if not isinstance(block, dict) or not block:
        return None
    out = {}
    for key, value in block.items():
        out[key] = value.get("Value") if isinstance(value, dict) and "Value" in value else value
    return out


def atomic_write_json(path: Path, obj) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(obj, indent=2, ensure_ascii=False, default=str), encoding="utf-8")
    os.replace(tmp, path)


def wait_until(predicate, timeout: float, poll: float = 0.005, what: str = "condition"):
    end = time.time() + timeout
    while time.time() < end:
        try:
            value = predicate()
        except Exception:
            value = None
        if value:
            return value
        time.sleep(poll)
    raise TimeoutError(f"timed out after {timeout:.0f}s waiting for {what}")


def parse_set_arguments(values: list[str]) -> dict:
    out = {}
    for item in values or []:
        if "=" not in item:
            raise SystemExit(f"--set expects key=value, got {item!r}")
        key, raw = item.split("=", 1)
        out[key.strip()] = parse_scalar(raw.strip())
    return out


def parse_scalar(raw: str):
    low = raw.lower()
    if low in ("true", "false"):
        return low == "true"
    try:
        if "." in raw or "e" in low or low in ("nan", "inf", "-inf"):
            return float(raw)
        return int(raw)
    except ValueError:
        return raw


def table_aggregates(table: dict, gene_names: list[str]) -> dict:
    """vitals/species/genes/brain aggregates from /bibites/table, matching the sampler's definitions."""
    cols = table.get("columns") or {}
    genes = table.get("genes") or {}
    n = int(table.get("n") or 0)
    out: dict = {"vitals": {}, "species": {}, "genes": {}, "brain": {}}
    if n == 0 or not cols:
        return out
    energy = cols.get("energy") or []
    health = cols.get("health") or []
    age = cols.get("age") or []
    out["vitals"] = {"energy_mean": sum(energy) / n, "health_mean": sum(health) / n,
                     "age_mean": sum(age) / n, "age_max": max(age) if age else 0.0}
    species_counts: dict[str, int] = {}
    for sid in cols.get("species") or []:
        species_counts[str(int(sid))] = species_counts.get(str(int(sid)), 0) + 1
    out["species"] = {"n": len(species_counts), "counts": species_counts}
    nodes = cols.get("nodes") or []
    syn = cols.get("synapses") or []
    if nodes:
        out["brain"] = {"nodes_mean": sum(nodes) / n, "nodes_max": max(nodes),
                        "syn_mean": sum(syn) / n if syn else 0.0, "syn_max": max(syn) if syn else 0}
    if genes:
        means, stds = [], []
        for name in gene_names:
            vals = genes.get(name)
            if not vals:
                means.append(0.0)
                stds.append(0.0)
                continue
            mu = sum(vals) / n
            var = max(0.0, sum(v * v for v in vals) / n - mu * mu) if n > 1 else 0.0
            means.append(mu)
            stds.append(var ** 0.5)
        out["genes"] = {"mean": means, "std": stds}
    return out


def capture_entity_snapshot(client, gene_names: list[str]) -> tuple[list[dict], dict]:
    """One per-entity snapshot (merged /bibites/table + /entities/json). Returns (entities, table)."""
    table = client.bibite_table(genes=True, eggs=True)
    cols = table.get("columns") or {}
    handles = list(cols.get("h") or [])
    egg_handles = list(((table.get("eggs") or {}).get("h")) or [])
    all_handles = handles + egg_handles
    got = client.entities_json(all_handles) if all_handles else {"items": []}
    by_handle = {int(it["h"]): it for it in got.get("items", [])}
    entities: list[dict] = []
    for i, h in enumerate(handles):
        item = by_handle.get(int(h))
        if not item:
            continue
        rec = pc.entity_record(item["data"], "bibite", gene_names)
        if rec is None:
            continue
        def col(name):
            arr = cols.get(name) or []
            return arr[i] if i < len(arr) else None
        if col("maxEnergy") is not None:
            rec["max_energy"] = float(col("maxEnergy"))
        if col("maxHealth") is not None:
            rec["max_health"] = float(col("maxHealth"))
        if col("size") is not None:
            rec["size"] = float(col("size"))
        entities.append(rec)
    for h in egg_handles:
        item = by_handle.get(int(h))
        if not item:
            continue
        rec = pc.entity_record(item["data"], "egg", gene_names)
        if rec is not None:
            entities.append(rec)
    return entities, table


# -------------------------------------------------------------------------------------------------
# world lifecycle
# -------------------------------------------------------------------------------------------------
class WorldSession:
    """Owns the game (or attaches to one), the bridge client and the parity plugin."""

    def __init__(self, args, BridgeClient, LiveRuntime, Lab):
        self.args = args
        self.BridgeClient = BridgeClient
        self.LiveRuntime = LiveRuntime
        self.Lab = Lab
        self.client = None
        self.runtime = None
        self.started_game = False
        self.plugin = ParityPlugin(args.plugin_port) if not args.no_plugin else None
        self.plugin_ok = False

    def open(self):
        args = self.args
        client = self.BridgeClient(port=args.port, timeout=60.0)
        if client.available():
            if not args.attach:
                client.close()
                raise Drift("a game is already answering on the bridge port; it holds a world this script "
                            "did not create. Stop it, or pass --attach to use it deliberately.")
            other(f"attaching to the already-running game on port {args.port} (--attach)")
            self.client = client
        else:
            other("starting a hidden (headless) game through bibites_api ...")
            self.runtime = self.LiveRuntime(work_dir=Path(args.work_dir))
            self.runtime.work.mkdir(parents=True, exist_ok=True)
            lab = self.Lab(self.runtime, work_dir=Path(args.work_dir), window="hidden")
            self.client = lab.ensure_game(timeout=args.start_timeout)
            self.started_game = True
            other(f"game ready (pid {self.runtime.host.gp.pid if self.runtime.host.gp else '?'})")
        self.plugin_ok = bool(self.plugin and self.plugin.available())
        if self.plugin_ok:
            other(f"parity plugin {self.plugin.ping().get('plugin')} available on port {args.plugin_port}")
        return self.client

    def close(self):
        try:
            if self.started_game and self.runtime is not None and not self.args.keep_game:
                other("stopping the game this capture started")
                self.runtime.close()
            elif self.runtime is not None and self.args.keep_game:
                other("--keep-game: leaving the game running")
        finally:
            if self.client is not None:
                try:
                    self.client.close()
                except Exception:
                    pass


# -------------------------------------------------------------------------------------------------
# base world
# -------------------------------------------------------------------------------------------------
def base_provenance_path(base: Path) -> Path:
    return base.with_suffix(".base.json")


def create_base(session: WorldSession, args, inputs: dict) -> dict:
    """Create the base save from a scenario, pausing and pinning settings first."""
    client = session.client
    scenario = Path(args.scenario).absolute()
    if not scenario.exists():
        raise Drift(f"scenario {scenario} does not exist")
    other(f"creating base world from scenario {scenario.name} (newgame) ...")
    client.new_game(scenario, name=args.name or f"parity {scenario.stem}")
    client.set_time(paused=True)
    settings = client.get_settings()
    independent = dict(settings.get("independent") or {})
    scenario_settings = dict(settings.get("scenario") or {})
    overrides = dict(args.set or {})
    if args.settings_file:
        overrides.update(json.loads(Path(args.settings_file).read_text(encoding="utf-8-sig")).get("independent") or {})
    independent.update(overrides)
    if not args.keep_spawners:
        independent["virginSpawnRate"] = 0.0
    # the bridge takes ONE flat {key: value} map and routes each key to ScenarioSettings or
    # ScenarioIndependentSettings itself (its /settings response groups them by scope).
    applied = client.set_settings({**scenario_settings, **independent})
    if applied.get("unknown"):
        raise Drift(f"unknown settings keys: {applied['unknown']}")
    if not session.plugin_ok:
        raise Drift("base creation needs the parity plugin (pause-at-load + preload settings); "
                    "install it with tools/parity/dump_prefabs.py or deploy tools/parity/plugin")
    session.plugin.preload(independent)
    settings = client.get_settings()
    status = client.status()
    base = Path(args.base).absolute()
    base.parent.mkdir(parents=True, exist_ok=True)
    client.save_world(base)
    sim_time = scene_sim_time_of_save(base)
    if sim_time is None:
        raise Drift(f"the game did not write a readable scene.bb8scene into {base}")
    counts = status.get("counts") or {}
    prov = {
        "schema": "rustibites.parity.base.v1",
        "name": args.name or scenario.stem,
        "created_at": datetime.now(timezone.utc).isoformat(),
        "game": inputs["game"],
        "scenario": {"path": str(scenario), "sha256": sha256_of(scenario)},
        "base": {"path": str(base), "sha256": sha256_of(base), "sim_time": sim_time},
        "save_independents": save_independents_of(base),
        "settings": settings,
        "settings_hash": pc.settings_hash(settings),
        "counts": counts,
        "note": "0.6.3.1 saves carry the scenario-independent settings under "
                "settings.bb8settings['independents'] (fmt-dev verified SaveSystem.SerializeSettings/LoadGame); "
                "the values here are the EFFECTIVE ones read back from the game after loading. Scenario zips "
                "instead carry a 'recommended' list, so a NEW world started from a scenario uses the game's "
                "current global values - capture.py records those too and freezes them in the base.",
    }
    atomic_write_json(base_provenance_path(base), prov)
    other(f"base written: {base} ({base.stat().st_size} bytes, t={sim_time!r}, bibites={counts.get('bibites')})")
    return prov


def load_base_provenance(base: Path, inputs: dict) -> dict:
    prov_path = base_provenance_path(base)
    if prov_path.exists():
        return json.loads(prov_path.read_text(encoding="utf-8"))
    sim_time = scene_sim_time_of_save(base)
    return {
        "schema": "rustibites.parity.base.v1",
        "name": base.stem,
        "created_at": None,
        "game": inputs["game"],
        "scenario": None,
        "base": {"path": str(base), "sha256": sha256_of(base), "sim_time": sim_time},
        "save_independents": save_independents_of(base),
        "settings": None,
        "settings_hash": None,
        "counts": None,
        "note": "no .base.json provenance next to this save; scenario-independent settings must be supplied "
                "with --settings-file (saves do not store them).",
    }


# -------------------------------------------------------------------------------------------------
# one capture pass
# -------------------------------------------------------------------------------------------------
def _attach_feeding_behavior(records_by_tick: dict, feeding_records: list[dict], dt: float) -> int:
    """Derive the comparable behavior.* rates from consecutive feeding snapshots and attach them to the
    matching tick records (task-94 follow-up / FEEDING_METRICS §6.1). All rates are per live bibite-second
    and tagged polling_proxy; raw stomach/mouth evidence stays in the feeding stream. Returns the number
    of tick records that received fields."""
    if len(feeding_records) < 2:
        return 0
    def mats(snap):
        out = {}
        for b in snap.get("bibites") or []:
            bid = b.get("id")
            mats_b = {str(k): float(v or 0) for k, v in (b.get("stomach_materials") or {}).items()}
            out[bid] = (mats_b, float(b.get("total_murders") or 0), float(b.get("damage_dealt") or 0))
        return out
    def classify(name):
        n = str(name).lower()
        return "plant" if "plant" in n else ("meat" if "meat" in n else "other")
    attached = 0
    prev = feeding_records[0]
    prev_m = mats(prev)
    for cur in feeding_records[1:]:
        t0, t1 = float(prev.get("t") or 0), float(cur.get("t") or 0)
        interval = t1 - t0
        if interval <= 0:
            prev, prev_m = cur, mats(cur)
            continue
        cur_m = mats(cur)
        pop0, pop1 = len(prev_m), len(cur_m)
        bibite_seconds = 0.5 * (pop0 + pop1) * interval
        if bibite_seconds <= 0:
            prev, prev_m = cur, cur_m
            continue
        dens = {}
        for name, props in (cur.get("material_properties") or {}).items():
            try:
                dens[classify(name)] = float(props.get("energy_density") or 0.0)
            except Exception:
                pass
        amount = {"plant": 0.0, "meat": 0.0}
        events = {"plant": 0, "meat": 0}
        for bid, (cm, _, _) in cur_m.items():
            pm = prev_m.get(bid, ({}, 0, 0))[0]
            for name, v in cm.items():
                d = v - float(pm.get(name, 0.0))
                if d > 1e-6:
                    cat = classify(name)
                    if cat in amount:
                        amount[cat] += d
                        events[cat] += 1
        bibite_bites = bibite_damage = 0.0
        for bid, (_, mur, dmg) in cur_m.items():
            pm = prev_m.get(bid)
            if pm:
                bibite_bites += max(0.0, mur - pm[1])
                bibite_damage += max(0.0, dmg - pm[2])
        fields = {
            "plant_bites_per_s": events["plant"] / bibite_seconds,
            "meat_bites_per_s": events["meat"] / bibite_seconds,
            "bibite_bites_per_s": bibite_bites / bibite_seconds,
            "bibite_damage_per_s": bibite_damage / bibite_seconds,
            "plant_consumed_u2_per_s": amount["plant"] / bibite_seconds,
            "meat_consumed_u2_per_s": amount["meat"] / bibite_seconds,
            "plant_consumed_e_per_s": amount["plant"] * dens.get("plant", 0.0) / bibite_seconds,
            "meat_consumed_e_per_s": amount["meat"] * dens.get("meat", 0.0) / bibite_seconds,
            "stomach_energy_gain_per_s": (amount["plant"] * dens.get("plant", 0.0)
                                          + amount["meat"] * dens.get("meat", 0.0)) / bibite_seconds,
            "evidence": "polling_proxy",
            "denominator": "live bibite-seconds (mean population x interval)",
        }
        target = cur.get("tick")
        rec = records_by_tick.get(int(target)) if target is not None else None
        if rec is not None:
            beh = rec.setdefault("behavior", {})
            beh.update(fields)
            attached += 1
        prev, prev_m = cur, cur_m
    return attached


def run_pass(session: WorldSession, args, pass_index: int, out_dir: Path, inputs: dict,
             base: Path | None, base_info: dict, scenario: Path | None) -> dict:
    client = session.client
    taint: list[str] = []
    plugin_ok = session.plugin_ok
    sim_tps = int(((base_info.get("settings") or {}).get("independent") or {}).get("simTPS") or 30)
    dt = 1.0 / sim_tps
    settings = base_info.get("settings")
    if settings is None:
        raise Drift("base has no recorded settings (no .base.json); pass --settings-file with the "
                    "scenario+independent settings, or create the base with --scenario/--base")
    settings_hash_value = base_info.get("settings_hash") or pc.settings_hash(settings)

    # -- (re)load the world; with the plugin, pause exactly at the saved state --------------------
    if plugin_ok:
        session.plugin.pacing(True)
        session.plugin.load_pause(True)
        session.plugin.preload(settings.get("independent") or {})
    start = None
    for attempt in range(1, args.load_attempts + 1):
        if base is not None:
            client.load_world(base)
        else:
            client.new_game(scenario, name=args.name)
            client.set_time(paused=True)
        status = client.status()
        if not status.get("inSimulation"):
            raise Drift("the game reports no simulation after loading the world")
        paused = bool(status.get("paused"))
        if not paused:
            client.set_time(paused=True)
            taint.append("load did not pause by itself; paused over HTTP after load (RNG may have advanced)")
            status = client.status()
        t0 = float(status.get("simulatedTime") or 0.0)
        base_time = base_info.get("base_sim_time")
        if base_time is None and base is not None:
            base_time = scene_sim_time_of_save(base)
            base_info["base_sim_time"] = base_time
        kind, delta, f32_delta = classify_load_time(t0, base_time, dt)
        if load_acceptable(kind, strict_load=args.strict_load):
            if kind == "f32_quantised":
                taint.append(
                    f"load_time_quantisation_f32: post-load clock is the nearest f32 of the save's scene "
                    f"simulatedTime (delta {delta:+.6g}s = {abs(delta) / dt:.3f} ticks, f32_delta "
                    f"{f32_delta:+.3g}s); no simulation ticks advanced - clock label only")
            start = (status, t0, base_time, delta, kind, f32_delta)
            break
        reason = ("rejected by --strict-load" if kind == "f32_quantised" and args.strict_load
                  else f"world advanced {delta / dt:.3f} ticks past the save")
        other(f"load attempt {attempt}: {reason}; reloading")
        taint.append(f"clean-load retry {attempt}: {reason}")
    if start is None:
        raise Drift(f"could not land a clean load of {base} (advanced past the save on every attempt; "
                    f"is the parity plugin installed/armed?)")
    status, t0, base_time, delta, load_kind, load_f32_delta = start

    # scenario starts have no saved clock: tick 0 is the start of this capture (the newgame spawner
    # already ran before the capture seed, recorded as taint by the scenario branch)
    if base_time is None and scenario is not None:
        base_time = t0
        base_info["base_sim_time"] = t0
        taint.append("scenario start: tick 0 = capture start; newgame spawner/seeding ran before the "
                     "capture-side seed, so this is a trajectory reference, not per-entity ground truth")

    # -- fixed settings: re-apply exactly what the base recorded ---------------------------------
    sc = dict(settings.get("scenario") or {})
    ind = dict(settings.get("independent") or {})
    applied = client.set_settings({**sc, **ind})
    if applied.get("unknown"):
        taint.append(f"unknown settings keys: {applied['unknown']}")
    got_settings = client.get_settings()
    got_hash = pc.settings_hash(got_settings)
    if got_hash != settings_hash_value:
        diff = settings_diff(settings, got_settings)
        raise Drift(f"settings drifted from the base: {diff[:5]}")
    sim_tps = int(ind.get("simTPS") or sim_tps)
    dt = 1.0 / sim_tps
    brain_tps = int(ind.get("brainTPS") or 2)

    if args.speed != 1.0:
        taint.append(f"speed {args.speed}x: the game is bit-reproducible at 1x only; at higher speeds several "
                     f"fixed steps share a frame and the run drifts (measured). Use --speed 1 for "
                     f"deterministic evidence; high-speed captures are statistical observations")
    if not plugin_ok:
        taint.append("parity plugin not reachable: Time.fixedDeltaTime is not pinned and loads are not "
                     "paused inside onLoadingDone; this capture is NOT expected to be reproducible")
    elif args.lock_dt:
        pacing = session.plugin.pacing(True)
        if abs(float(pacing.get("fixedDeltaTime", 0.0)) - dt) > 1e-4:
            taint.append(f"dt pin not at 1/simTPS ({pacing})")

    # -- seed -------------------------------------------------------------------------------------
    seeded = None
    if args.seed is not None:
        seeded = client.seed(args.seed)
        parts = classify_seed_parts(seeded.get("seeded"))
        missing = EXPECTED_SEED_PARTS - parts
        if missing and not args.allow_partial_seed:
            raise Drift(f"/lab/seed did not seed {sorted(missing)} (got {sorted(parts)}); the game build "
                        f"changed - refusing to write a capture whose RNG state is only partly fixed")
        if missing:
            taint.append(f"partial seed, missing {sorted(missing)}")
    else:
        taint.append("no --seed: RNG state not fixed")

    expected_ind = (base_info.get("save_independents") or {})
    if expected_ind:
        actual_ind = (got_settings.get("independent") or {})
        bad = []
        for key, want in expected_ind.items():
            got = actual_ind.get(key)
            if isinstance(want, (int, float)) and not isinstance(want, bool):
                if got is None or abs(float(got) - float(want)) > 1e-6 * max(1.0, abs(float(want))):
                    bad.append({"key": key, "save": want, "game": got})
            elif got != want:
                bad.append({"key": key, "save": want, "game": got})
        if bad:
            raise Drift(f"the loaded world's independent settings differ from the save block: {bad[:5]} "
                        f"(game build changed how saves are applied?)")
    start_tick = int(round((t0 - base_time) / dt))
    if abs(start_tick) > args.max_start_ticks:
        raise Drift(f"world starts {start_tick} ticks past the saved state")
    end_tick = start_tick + args.ticks
    gene_names = list((base_info.get("gene_names") or pc.GENE_NAMES))
    other(f"pass {pass_index}: t0={t0!r} (tick {start_tick} since base, delta={delta:+.2e}s), "
          f"simTPS={sim_tps}, seed={args.seed}, ticks {start_tick}..{end_tick}")

    # -- initial snapshot (the exact state Rust must load) ---------------------------------------
    entities0, table0 = capture_entity_snapshot(client, gene_names)
    agg0 = table_aggregates(table0, gene_names)
    counts0 = dict(status.get("counts") or {})
    partial0 = {"schema": pc.TICK_SCHEMA, "source": "game", "tick": start_tick, "t": t0,
                "sim_tps": sim_tps, "brain_tps": brain_tps, "settings_hash": settings_hash_value,
                "partial": True, "counts": {"bibites": int(counts0.get("bibites", 0)),
                                            "eggs": int(counts0.get("eggs", 0)),
                                            "corpses": int(counts0.get("corpses", 0))},
                "entities": entities0}
    partial0.update({k: v for k, v in agg0.items() if v})
    records: list[dict] = [partial0]
    records_by_tick = {start_tick: partial0}
    events: list[dict] = []
    event_next = 0
    segments = []
    overshoots = []
    snapshot_ticks = [start_tick]

    # -- run segments -----------------------------------------------------------------------------
    if args.snapshot_every > 0:
        taint.append("intermediate pause boundaries requested (--snapshot-every): measured against the "
                     "installed game, the ticks after a pause/resume boundary are NOT bit-reproducible; "
                     "use the aggregate curves + start/end entity snapshots for deterministic evidence")
    # -- feeding snapshots (task-94) ---------------------------------------------------------------
    feeding_requested = bool(getattr(args, "feeding", False))
    feeding_ok = bool(plugin_ok and feeding_requested)
    feeding_samples: dict[float, dict] = {}
    if feeding_requested and not plugin_ok:
        taint.append("feeding requested but the parity plugin is unavailable; no feeding group written")
    elif feeding_ok:
        taint.append("feeding polling adds periodic main-thread HTTP requests while the world runs; the "
                     "simulation tick dt/RNG are unchanged (fixed-dt pin), but wall-clock pacing is perturbed")

    wall_start = time.time()
    cursor = start_tick
    while cursor < end_tick:
        target = min(cursor + args.snapshot_every, end_tick) if args.snapshot_every > 0 else end_tick
        if target <= cursor:
            target = end_tick
        t_current = float(client.status().get("simulatedTime") or t0)
        target_t = base_time + target * dt
        schedule_t = (target_t - t_current) - dt * 0.5
        if schedule_t < 0:
            schedule_t = 0.0
        client.run_start({"extinct": False}, speed=args.speed,
                         sample={"every": args.sample_every, "fields": list(pc.SAMPLE_FIELDS), "events": bool(args.events)},
                         schedule=[{"t": schedule_t, "do": "POST /time", "body": {"paused": True}}],
                         pause=True, replace=True)
        seg_start = time.time()
        feeding_stop = threading.Event()
        feeding_thread = None
        if feeding_ok:
            def _feeding_poll(plugin=session.plugin, stop=feeding_stop, period=args.feeding_period,
                              samples=feeding_samples):
                while not stop.is_set():
                    try:
                        snap = plugin.feeding()
                        if isinstance(snap, dict) and snap.get("t") is not None:
                            samples[float(snap["t"])] = snap
                    except Exception:
                        pass
                    stop.wait(max(0.005, period))
            feeding_thread = threading.Thread(target=_feeding_poll, daemon=True, name="feeding-poll")
            feeding_thread.start()
        try:
            wait_until(lambda: client.status().get("paused") is True, args.run_timeout,
                       poll=0.01, what="the run to pause")
            run_state = client.run_state()
            if run_state.get("state") == "error":
                raise Drift(f"run error: {run_state.get('error')}")
            client.run_cancel()
        finally:
            feeding_stop.set()
            if feeding_thread is not None:
                feeding_thread.join(timeout=2.0)
        series = client.series(0, 20000)
        rows = series.get("rows") or []
        seg_t_end = float(client.status().get("simulatedTime") or t_current)
        seg_over = round((seg_t_end - base_time) / dt) - target
        overshoots.append(seg_over)
        for row in rows:
            tick = int(round((float(row["t"]) - base_time) / dt))
            if tick in records_by_tick and not records_by_tick[tick].get("partial"):
                continue                          # already recorded (a boundary can repeat)
            rec = pc.tick_record_from_row(row, source="game", tick=tick, sim_tps=sim_tps,
                                          brain_tps=brain_tps, settings_hash_value=settings_hash_value)
            records.append(rec)
            records_by_tick[tick] = rec
        if args.events:
            ev = client.events(event_next, 20000)
            for e in ev.get("events") or []:
                events.append(pc.event_record(e, tick=int(round((float(e["t"]) - base_time) / dt))))
            event_next = ev.get("next", event_next)
        # per-entity snapshot at the pause boundary whose tick we actually reached
        snap_tick = int(round((seg_t_end - base_time) / dt))
        if snap_tick not in records_by_tick or snap_tick == cursor or target >= end_tick:
            entities, table = capture_entity_snapshot(client, gene_names)
            rec = records_by_tick.get(snap_tick)
            if rec is None:
                rec = {"schema": pc.TICK_SCHEMA, "source": "game", "tick": snap_tick, "t": seg_t_end,
                       "sim_tps": sim_tps, "brain_tps": brain_tps, "settings_hash": settings_hash_value,
                       "partial": True, "entities": entities}
                records.append(rec)
                records_by_tick[snap_tick] = rec
            else:
                rec["entities"] = entities
            snapshot_ticks.append(snap_tick)
        cursor = snap_tick
        achieved = (seg_t_end - t_current) / max(1e-6, time.time() - seg_start)
        segments.append({"index": len(segments), "target_tick": target, "end_tick": seg_tick if (seg_tick := snap_tick) is not None else snap_tick,
                         "overshoot_ticks": seg_over, "rows": len(rows), "wall_s": round(time.time() - seg_start, 4),
                         "achieved_speed": round(achieved, 3)})
        if cursor >= end_tick:
            break
        if snap_tick <= cursor - 10_000:
            raise Drift("segment made no progress")
    wall = time.time() - wall_start

    # -- ordering, gap and completeness checks ---------------------------------------------------
    ordered = sorted(records, key=lambda r: r["tick"])
    prev_tick = None
    gaps = 0
    for rec in ordered:
        if prev_tick is not None and rec["tick"] != prev_tick + 1:
            if rec["tick"] <= prev_tick:
                taint.append(f"duplicate/out-of-order tick {rec['tick']} after {prev_tick}")
            else:
                gaps += rec["tick"] - prev_tick - 1
        prev_tick = rec["tick"]
    if gaps:
        msg = f"{gaps} simulated tick(s) have no sample (max rows {len(records)})"
        if args.allow_gaps:
            taint.append(msg)
        else:
            raise Drift(msg + ": the sampler is not keeping up; refusing to write a drifting capture")
    first_tick = ordered[0]["tick"]
    last_tick = ordered[-1]["tick"]
    if last_tick < end_tick:
        raise Drift(f"the run stopped at tick {last_tick}, before the requested end tick {end_tick}")
    if last_tick > end_tick:
        taint.append(f"stop overshoot: finished at tick {last_tick} (+{last_tick - end_tick} ticks past the "
                     f"nominal end {end_tick}); Unity finishes the fixed steps already queued for the frame")

    # -- final status and manifest ---------------------------------------------------------------
    end_status = client.status()
    counts_end = dict(end_status.get("counts") or {})
    # -- attach feeding snapshots to the matching tick records (optional group) -------------------
    feeding_file = None
    if feeding_samples:
        feeding_records = []
        for t in sorted(feeding_samples):
            snap = dict(feeding_samples[t])
            tick = int(round((t - base_time) / dt))
            snap["tick"] = tick
            feeding_records.append(snap)
            rec = records_by_tick.get(tick)
            if rec is not None:
                rec["feeding"] = snap
        _attach_feeding_behavior(records_by_tick, feeding_records, dt)
        feeding_file = pc.write_jsonl_gz(out_dir / "feeding.jsonl.gz", feeding_records)
        try:
            append = getattr(pc, "append_manifest", None)
        except Exception:
            append = None
    files = {"ticks": pc.write_jsonl_gz(out_dir / "ticks.jsonl.gz", ordered)}
    if args.events:
        files["events"] = pc.write_jsonl_gz(out_dir / "events.jsonl.gz", sorted(events, key=lambda e: (e["tick"], e.get("id") or 0)))
    if feeding_file is not None:
        files["feeding"] = feeding_file
    engine = dict(inputs["game"])
    if plugin_ok:
        ping = session.plugin.ping()
        engine["unity_version"] = ping.get("unity")
    achieved = (last_tick - first_tick) / wall if wall > 0 else 0.0
    manifest = {
        "schema": pc.CAPTURE_SCHEMA,
        "kind": "capture",
        "source": "game",
        "capture": args.name or out_dir.name,
        "pass": pass_index,
        "created_at": datetime.now(timezone.utc).isoformat(),
        "engine": engine,
        "gene_names": gene_names,
        "inputs": {
            "scenario": base_info.get("scenario"),
            "base": base_info.get("base"),
            "save": inputs.get("save"),
            "seed": args.seed,
            "seeded_parts": (seeded or {}).get("seeded"),
            "settings_requested": {"scenario": sc, "independent": ind},
            "independents_in_save": base_info.get("save_independents"),
            "settings_file": inputs.get("settings_file"),
        },
        "settings": got_settings,
        "settings_hash": got_hash,
        "sim": {
            "sim_tps": sim_tps,
            "brain_tps": brain_tps,
            "dt": dt,
            "base_time": base_time,
            "start_t": t0,
            "start_tick": start_tick,
            "end_tick": last_tick,
            "ticks": last_tick - first_tick,
            "wall_s": round(wall, 3),
            "achieved_ticks_per_s": round(achieved, 1),
            "achieved_speed": round(achieved * dt, 2),
            "overshoot_ticks": overshoots,
            "load_kind": load_kind,
            "load_delta_s": delta,
            "load_f32_delta_s": load_f32_delta,
            "load_parity_clean": True,
            "segments": segments,
        },
        "world": {"simulation_size": end_status.get("simulationSize"),
                  "zones": client.zones()},
        "counts_start": counts0,
        "counts_end": counts_end,
        "feeding": {
            "requested": feeding_requested,
            "available": feeding_ok,
            "schema": "rustibites.parity.feeding.v1",
            "evidence": "polling_proxy",
            "exact_events": False,
            "file": files.get("feeding"),
            "samples": len(feeding_samples),
            "poll_period_s": getattr(args, "feeding_period", None),
            "note": "per-bibite stomach contents by material (intake proxy), held pellets, mouth bite "
                    "timers/counters, energy/health; per-zone pellet stock/production counters. "
                    "Capture-side diffs give successful-intake ('bite') proxies and pellet-mass balance. "
                    "bite_rate_mean is not used here.",
        },
        "plugin": {"available": plugin_ok,
                   "version": session.plugin.ping().get("plugin") if plugin_ok else None,
                   "dt_pinned": bool(plugin_ok and args.lock_dt),
                   "paused_at_load": bool(plugin_ok),
                   "preloaded_settings": bool(plugin_ok)},
        "taint": taint,
        "files": files,
    }
    atomic_write_json(out_dir / "manifest.json", manifest)
    other(f"pass {pass_index}: wrote {len(ordered)} tick records, {len(events)} events, "
          f"wall {wall:.2f}s ({achieved:.1f} ticks/s), overshoot {overshoots}")
    if taint:
        other(f"pass {pass_index} taint: {taint}")
    return manifest


def settings_diff(expected: dict, actual: dict) -> list[dict]:
    out = []
    for scope in ("scenario", "independent"):
        e = (expected or {}).get(scope) or {}
        a = (actual or {}).get(scope) or {}
        for key in sorted(set(e) | set(a)):
            if e.get(key) != a.get(key):
                out.append({"scope": scope, "key": key, "expected": e.get(key), "actual": a.get(key)})
    return out


# -------------------------------------------------------------------------------------------------
# determinism across repeated passes
# -------------------------------------------------------------------------------------------------
def compare_passes(out: Path, manifests: list[dict], args) -> dict:
    captures = [pc.open_capture((out / f"pass-{i}") if args.repeat > 1 else out) for i in range(len(manifests))]
    digests = []
    for cap in captures:
        by_tick = {}
        entities = {}
        for rec in cap.records():
            d = pc.record_digest(rec)
            by_tick[int(rec["tick"])] = d
            if rec.get("entities"):
                entities[int(rec["tick"])] = pc.sha16(pc.canonical_json(rec["entities"]).encode())
        digests.append((by_tick, entities))
    ref_ticks, ref_entities = digests[0]
    common = set(ref_ticks)
    for t, _ in digests[1:]:
        common &= set(t)
    common = sorted(common)
    differing = [t for t in common if len({d[0][t] for d in digests}) > 1]
    common_entity_ticks = set(ref_entities)
    for _, e in digests[1:]:
        common_entity_ticks &= set(e)
    entity_diff = [t for t in sorted(common_entity_ticks) if len({d[1][t] for d in digests}) > 1]
    first = differing[0] if differing else None
    detail = None
    if first is not None:
        rows = []
        for cap in captures:
            for rec in cap.records():
                if int(rec["tick"]) == first:
                    rows.append(rec)
                    break
        detail = {"tick": first, "records": rows}
    result = {
        "schema": "rustibites.parity.determinism.v1",
        "passes": len(captures),
        "identical_ticks": len(common) - len(differing),
        "common_ticks": len(common),
        "differing_ticks": len(differing),
        "first_differing_tick": first,
        "entity_snapshot_ticks_compared": len(common_entity_ticks),
        "differing_entity_snapshots": len(entity_diff),
        "bit_identical": len(differing) == 0 and len(entity_diff) == 0,
        "first_difference": detail,
        "taint": {m.get("pass"): m.get("taint") for m in manifests},
        "engine": manifests[0].get("engine"),
        "sim": manifests[0].get("sim"),
    }
    atomic_write_json(out / "determinism.json", result)
    return result


# -------------------------------------------------------------------------------------------------
# main
# -------------------------------------------------------------------------------------------------
def build_arg_parser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    world = ap.add_argument_group("world / inputs")
    world.add_argument("--scenario", help="scenario zip used to CREATE --base when it does not exist")
    world.add_argument("--base", help="base save zip (created from --scenario if missing; recommended)")
    world.add_argument("--save", help="an existing save zip to capture (no base provenance; needs --settings-file)")
    world.add_argument("--save-out", help="write the final game world to this save zip at the end of the capture")
    world.add_argument("--trace-out", help="per-tick per-bibite plugin trace JSONL (task-122)")
    world.add_argument("--settings-file", help="JSON {scenario:{...}, independent:{...}} of fixed settings")
    world.add_argument("--set", action="append", default=[], metavar="KEY=VALUE",
                       help="override one setting (repeatable); applied to the base/independent settings")
    world.add_argument("--seed", type=int, default=12345, help="seed for /lab/seed (default 12345; -1 = none)")
    world.add_argument("--keep-spawners", action="store_true",
                       help="do not force virginSpawnRate=0 in the base (breaks load reproducibility)")
    run = ap.add_argument_group("run")
    run.add_argument("--ticks", type=int, default=300, help="simulation ticks to capture after the start state")
    run.add_argument("--speed", type=float, default=1.0,
                     help="game speed multiplier while running (default 1.0 = the deterministic regime). "
                          "The installed game is bit-reproducible at 1x; at higher speeds frame grouping "
                          "(several fixed steps per frame) perturbs the run, so speeds > 1 get a taint entry "
                          "and are only suitable for statistical/ecology captures.")
    run.add_argument("--snapshot-every", type=int, default=0,
                     help="ticks between paused per-entity snapshots; 0 (default) = one uninterrupted run "
                          "with snapshots only at the start and final tick. >0 adds intermediate pause/resume "
                          "boundaries which are NOT bit-reproducible (measured: the ticks after a resume differ), "
                          "so such captures get a taint entry.")
    run.add_argument("--sample-every", type=float, default=0.001,
                     help="sampler interval in sim seconds; much smaller than dt = one row per tick")
    run.add_argument("--events", dest="events", action="store_true", default=True,
                     help="record the birth/death event log too (default on)")
    run.add_argument("--no-events", dest="events", action="store_false")
    run.add_argument("--feeding", action="store_true",
                     help="poll the parity plugin /d/feeding while running and store per-tick feeding "
                          "snapshots (per-bibite stomach/mouth/energy/health + per-zone pellet stock/"
                          "production). Requires the parity plugin; adds a 'feeding' optional group.")
    run.add_argument("--feeding-period", type=float, default=0.05,
                     help="seconds between feeding polls while the world runs (default 0.05 = every ~3 ticks at 1x)")
    run.add_argument("--repeat", type=int, default=1, help="repeat the pass K times to measure determinism")
    run.add_argument("--out", help="output directory (default testdata/parity/captures/<name>)")
    run.add_argument("--name", help="capture name (default: base/scenario stem)")
    out = ap.add_argument_group("output / safety")
    out.add_argument("--work-dir", default=str(DEFAULT_WORK), help="bibites_api work dir (pulls/checkpoints)")
    out.add_argument("--port", type=int, default=17800, help="bridge port (default 17800)")
    out.add_argument("--plugin-port", type=int, default=17801, help="parity plugin port (default 17801)")
    out.add_argument("--no-plugin", action="store_true", help="ignore the parity plugin (non-reproducible)")
    out.add_argument("--lock-dt", dest="lock_dt", action="store_true", default=True,
                     help="pin Time.fixedDeltaTime via the plugin (default on)")
    out.add_argument("--no-lock-dt", dest="lock_dt", action="store_false")
    out.add_argument("--attach", action="store_true", help="use an already-running game instead of starting one")
    out.add_argument("--keep-game", action="store_true", help="do not stop the game this run started")
    out.add_argument("--start-timeout", type=float, default=300.0)
    out.add_argument("--run-timeout", type=float, default=180.0)
    out.add_argument("--load-attempts", type=int, default=4, help="clean-load retries before failing")
    out.add_argument("--max-start-ticks", type=int, default=5,
                     help="refuse if the world starts more than this many ticks past the base save")
    out.add_argument("--allow-gaps", action="store_true", help="write the capture even if sampled ticks are missing")
    out.add_argument("--allow-partial-seed", action="store_true")
    out.add_argument("--strict-load", action="store_true",
                     help="require the historical |delta| <= 1e-5 s load criterion; reject legitimate f32 "
                          "quantisation of large simulatedTime (evolved saves)")
    out.add_argument("--self-check", action="store_true",
                     help="run the clean-load acceptance self-test (no game needed) and exit")
    out.add_argument("--expect-game-version", default="0.6.3.1",
                     help="game version the schema/fixtures were captured on (default 0.6.3.1)")
    out.add_argument("--expect-bridge-version", default="0.7.0",
                     help="minimum BibitesBridge version (default 0.7.0)")
    out.add_argument("--allow-version-mismatch", action="store_true",
                     help="capture even if the game/bridge versions differ (records a taint)")
    out.add_argument("--json", action="store_true", help="print the manifests as JSON at the end")
    return ap


def main(argv: list[str] | None = None) -> int:
    args = build_arg_parser().parse_args(argv)
    if args.self_check:
        return self_check_load_policy()
    args.set = parse_set_arguments(args.set)
    if args.seed is not None and args.seed < 0:
        args.seed = None
    if not (args.base or args.save or args.scenario):
        print("capture.py: need --base, --save or --scenario", file=sys.stderr)
        return 2
    api_home = Path(os.environ.get("BIBITES_API_HOME", DEFAULT_API_HOME))
    BridgeClient, BridgeError, LiveRuntime, Lab = import_api(api_home)

    base = Path(args.base).absolute() if args.base else None
    save = Path(args.save).absolute() if args.save else None
    scenario = Path(args.scenario).absolute() if args.scenario else None
    name = args.name or (base.stem if base else (save.stem if save else scenario.stem))
    out = Path(args.out).absolute() if args.out else DEFAULT_CAPTURES / f"{name}_s{args.seed}_t{args.ticks}"

    started = time.time()
    session = WorldSession(args, BridgeClient, LiveRuntime, Lab)
    manifests: list[dict] = []
    try:
        client = session.open()
        ping = client.ping()
        status = client.status()
        game = {"name": "The Bibites", "game_version": status.get("gameVersion"), "bridge_version": ping.get("bridge"),
                "bridge_port": args.port, "executable": None, "unity_version": None}
        if not args.allow_version_mismatch:
            if args.expect_game_version and status.get("gameVersion") != args.expect_game_version:
                raise Drift(f"game version {status.get('gameVersion')!r} != expected {args.expect_game_version!r} "
                            f"(pass --allow-version-mismatch to override)")
            if args.expect_bridge_version:
                try:
                    ok = tuple(int(x) for x in str(ping.get("bridge")).split(".")) >= tuple(
                        int(x) for x in args.expect_bridge_version.split("."))
                except ValueError:
                    ok = False
                if not ok:
                    raise Drift(f"bridge version {ping.get('bridge')!r} < expected {args.expect_bridge_version!r}")
        inputs = {"game": game, "scenario": None, "save": None, "settings_file": args.settings_file}

        # -- resolve/create the base ---------------------------------------------------------------
        if save is not None:
            base = save
            inputs["save"] = {"path": str(save), "sha256": sha256_of(save)}
            base_info = load_base_provenance(base, inputs)
        elif base is not None and base.exists():
            base_info = load_base_provenance(base, inputs)
        elif base is not None and scenario is not None:
            base_info = create_base(session, args, inputs)
            base = Path(base_info["base"]["path"])
            # after creating the base the world is the base state; pass 0 reloads it anyway
        elif scenario is not None:
            base = None
            base_info = {"schema": "rustibites.parity.base.v1", "name": scenario.stem, "base": None,
                         "scenario": {"path": str(scenario), "sha256": sha256_of(scenario)},
                         "settings": None, "settings_hash": None}
        else:
            raise Drift(f"base {base} does not exist and no --scenario was given to create it")
        inputs["scenario"] = base_info.get("scenario")
        if base is not None:
            inputs["base"] = base_info.get("base") or {"path": str(base), "sha256": sha256_of(base)}

        if base_info.get("settings") is None:
            # scenario: apply explicit --set overrides BEFORE newgame so the world starts with them
            # (simTPS is consumed when the simulation scene is built, so post-newgame set_settings cannot
            # change the tick rate of the already-built world).
            if base is None and scenario is not None and args.set:
                try:
                    if session.plugin_ok:
                        session.plugin.preload(dict(args.set))   # applied on sceneLoaded, before Start
                    else:
                        client.set_settings(dict(args.set))
                    client.set_time(paused=True)
                except Exception as exc:
                    other(f"pre-newgame settings apply failed ({exc}); continuing with game globals")
            # attach/load and read the effective settings now; store them for this run only
            client.load_world(base) if base is not None else client.new_game(scenario, name=name)
            client.set_time(paused=True)
            settings = client.get_settings()
            base_info["settings"] = settings
            base_info["settings_hash"] = pc.settings_hash(settings)
            base_info["base_sim_time"] = scene_sim_time_of_save(base) if base else None
            if args.settings_file:
                override = json.loads(Path(args.settings_file).read_text(encoding="utf-8-sig"))
                merged = {"scenario": dict(settings.get("scenario") or {}, **(override.get("scenario") or {})),
                          "independent": dict(settings.get("independent") or {}, **(override.get("independent") or {}))}
                base_info["settings"] = merged
                base_info["settings_hash"] = pc.settings_hash(merged)
            other("warning: no base provenance; using the settings effective now (record them in the manifest)")
        gene_names = list((base_info.get("settings") or {}).get("gene_names") or pc.GENE_NAMES)

        # -- optional per-tick trace ----------------------------------------------------------------
        trace_path = None
        if args.trace_out:
            trace_path = Path(args.trace_out).absolute()
            trace_path.parent.mkdir(parents=True, exist_ok=True)
            if session.plugin_ok:
                resp = session.plugin.trace_start(str(trace_path))
                other(f"per-tick trace -> {trace_path} start={resp}")
            else:
                other("warning: --trace-out needs the parity plugin; trace not started")

        # -- passes ---------------------------------------------------------------------------------
        for i in range(max(1, args.repeat)):
            pass_dir = (out / f"pass-{i}") if args.repeat > 1 else out
            pass_dir.mkdir(parents=True, exist_ok=True)
            manifest = run_pass(session, args, i, pass_dir, inputs, base, base_info, scenario)
            manifest["gene_names"] = gene_names
            manifests.append(manifest)
            if i + 1 < args.repeat:
                other("reloading the base for the next determinism pass ...")
        if trace_path is not None and session.plugin_ok:
            stop = session.plugin.trace_stop()
            other(f"trace stopped -> {trace_path} ({trace_path.stat().st_size if trace_path.exists() else 0} B) stop={stop}")
        out.mkdir(parents=True, exist_ok=True)
        if args.save_out:
            save_out = Path(args.save_out).absolute()
            save_out.parent.mkdir(parents=True, exist_ok=True)
            client.save_world(save_out)
            other(f"saved final game world -> {save_out} ({save_out.stat().st_size} B)")
        summary = {"schema": "rustibites.parity.capture_set.v1", "name": name, "passes": len(manifests),
                   "out": str(out), "manifests": sorted(str(p.relative_to(out)) if args.repeat > 1 else "manifest.json"
                                                        for p in out.glob("pass-*/manifest.json")) if args.repeat > 1 else ["manifest.json"],
                   "elapsed_s": round(time.time() - started, 2), "args": vars(args)}
        atomic_write_json(out / "capture.json", summary)
        det = None
        if len(manifests) > 1:
            det = compare_passes(out, manifests, args)
            other(f"determinism: bit_identical={det['bit_identical']} differing_ticks={det['differing_ticks']}/"
                  f"{det['common_ticks']} differing_entity_snapshots={det['differing_entity_snapshots']} "
                  f"first_difference_tick={det['first_differing_tick']}")
        else:
            m = manifests[0]
            other(f"capture ok: {m['files']['ticks']['records']} records, "
                  f"{m['sim']['start_tick']}..{m['sim']['end_tick']}, {m['sim']['achieved_ticks_per_s']} ticks/s, "
                  f"exit={'TAINTED' if m['taint'] else 'clean'}")
        if args.json:
            print(json.dumps({"manifests": manifests, "determinism": det}, indent=2, default=str))
        return 0 if all(not m["taint"] for m in manifests) or args.allow_gaps else 0
    except Drift as e:
        print(f"[capture] DRIFT: {e}", file=sys.stderr)
        return 1
    except Exception as e:  # noqa: BLE001
        print(f"[capture] FAILED: {type(e).__name__}: {e}", file=sys.stderr)
        traceback.print_exc()
        return 1
    finally:
        session.close()


if __name__ == "__main__":
    sys.exit(main())

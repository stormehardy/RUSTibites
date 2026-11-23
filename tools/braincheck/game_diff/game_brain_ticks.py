"""game_brain_ticks.py <save.zip> <out dir> <ticks> - the ORIGINAL game (hidden), one tick at a time.

Loads the save, pauses it, and writes the state of every living bibite (its whole JSON, brain node Value / LastInput / LastOutput included) before the first tick and after each of
the first <ticks> ticks as game_0000.json, game_0001.json, ...  ({body.id: {brain, brainTicksCount, dead, dying, timeAlive}}; timeAlive counts the physics ticks the bibite lived), plus manifest.json with the settings the brain
depends on (simTPS, brainTPS, disableHerding).  `brain_vs_game` (crates/bibites-cli/examples) then replays every brain tick in the port's `Brain::step` from the game's own state
and inputs and compares each node bit for bit.  See README.md in this folder.

Two optional environment variables (used by `sense_vs_game`, see README.md):
  SAVE_WORLDS=1       also writes the game's own save of the WHOLE world after each tick as world_0000.zip, world_0001.zip, ... (the port loads them);
  GAME_SETTINGS='{"visionLookupUpdateFactor": 1, ...}'   settings written through the bridge right after the load (JSON object).

Needs the installed game + BibitesAPI (BIBITES_API_HOME overrides the default location); the game is started hidden and stopped at the end.
"""
import json
import os
import sys
import time
from pathlib import Path

API = Path(os.environ.get("BIBITES_API_HOME", r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI"))
sys.path.insert(0, str(API))
from bibites_api.live.runtime import LiveRuntime  # noqa: E402
from bibites_api.lab import Lab  # noqa: E402

save = Path(sys.argv[1]).absolute()
out = Path(sys.argv[2]).absolute()
n_ticks = int(sys.argv[3])
out.mkdir(parents=True, exist_ok=True)
work = out / "_work"
work.mkdir(exist_ok=True)
runtime = LiveRuntime(work_dir=work)
runtime.work.mkdir(parents=True, exist_ok=True)
lab = Lab(runtime, work_dir=work, window="hidden")


def snapshot(client, path):
    """Writes {bibite id: {brain, brainTicksCount, dead, dying, timeAlive}} for every bibite the game holds."""
    ents = client.entities("bibite")
    handles = [e.get("h", e.get("handle")) for e in ents]
    handles = [h for h in handles if h is not None]
    rows = []
    for i in range(0, len(handles), 50):
        part = client.entities_json(handles[i:i + 50])
        rows += part.get("items", [])
    snap = {}
    for row in rows:
        data = row.get("data", row)
        body = data["body"]
        snap[str(body["id"])] = {
            "brain": data["brain"],
            "brainTicksCount": body.get("brainTicksCount"),
            "visionLookupCount": body.get("visionLookupCount"),
            "visionSensingCount": body.get("visionSensingCount"),
            "dead": body.get("dead"),
            "dying": body.get("dying"),
            "timeAlive": (data.get("clock") or {}).get("timeAlive"),
        }
    Path(path).write_text(json.dumps(snap), encoding="utf-8")
    return len(rows)


def find(d, key):
    """The first value stored under `key` anywhere in a nested settings dict (a {"Value": v} wrapper is unwrapped)."""
    if isinstance(d, dict):
        if key in d:
            v = d[key]
            return v.get("Value", v) if isinstance(v, dict) else v
        for v in d.values():
            r = find(v, key)
            if r is not None:
                return r
    return None


try:
    client = lab.ensure_game(timeout=300.0)
    client.load_world(save)
    client.set_time(paused=True)
    if os.environ.get("GAME_SETTINGS"):
        print("set settings", client.set_settings(json.loads(os.environ["GAME_SETTINGS"])), flush=True)
    settings = client.get_settings()
    tps = float((settings.get("independent") or {}).get("simTPS", 40))
    manifest = {
        "simTPS": find(settings, "simTPS"),
        "brainTPS": find(settings, "brainTPS"),
        "disableHerding": find(settings, "disableHerding"),
        "visionLookupUpdateFactor": find(settings, "visionLookupUpdateFactor"),
        "visionSenseUpdateFactor": find(settings, "visionSenseUpdateFactor"),
        "save": save.name,
    }
    (out / "manifest.json").write_text(json.dumps(manifest, indent=1), encoding="utf-8")
    print("manifest", manifest, flush=True)
    for tick in range(n_ticks + 1):
        n = snapshot(client, out / f"game_{tick:04d}.json")
        if os.environ.get("SAVE_WORLDS"):
            client.save_world(out / f"world_{tick:04d}.zip")
        if tick % 10 == 0 or tick == n_ticks:
            print(f"tick {tick}: {n} bibites", flush=True)
        if tick < n_ticks:
            t = float(client.status()["simulatedTime"])
            client.run_start({"simTime": t + 1.0 / tps}, speed=1.0, sample=None, pause=True, replace=True)
            end = time.time() + 60
            while time.time() < end:
                st = client.run_state(timeout=15.0)
                if st.get("state") not in ("running", "stopping"):
                    break
                time.sleep(0.005)
finally:
    try:
        runtime.close()
    except Exception as error:
        print("close:", error, flush=True)
    print("game stopped", flush=True)

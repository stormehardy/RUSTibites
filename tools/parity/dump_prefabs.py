#!/usr/bin/env python3
"""dump_prefabs.py - build + deploy the parity dump plugin, dump the game's prefab/serialized
constants to JSON, then stop the game and remove the plugin again.

The constants written here are NOT in save files and are only *declared* (not valued) in the
decompiled C#: MatterMaterial prefab densities, PhysicsMaterial2D friction/restitution, bibite /
pellet / egg prefab Rigidbody2D + Collider2D settings, Physics2D configuration and the collision
layer matrix, ColorKiller / RadTower / PheroTower prefab defaults, and every serialized field of
every component on those prefabs.

The plugin (tools/parity/plugin, GUID com.rustibites.paritydump) is a SECOND BepInEx plugin next to
BibitesBridge. It is copied into <game>/BepInEx/plugins/ParityDump/ for the dump and deleted again
afterwards, so the installed game is left exactly as it was (verify with --dry-run listing).

    python tools/parity/dump_prefabs.py                     # default Default.zip world (canonical; Ring.zip is a historical fixture)
    python tools/parity/dump_prefabs.py --scenario "C:\\...\\Defaulty.zip" --out testdata/parity/prefab_dump.json
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
PROJECT_DIR = HERE.parent.parent
DEFAULT_API_HOME = Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
DEFAULT_GAME_DIR = Path(r"C:\Program Files (x86)\Steam\steamapps\common\The Bibites")
PLUGIN_DIR = HERE / "plugin"
BUILD_OUT = PROJECT_DIR / "testdata" / "parity" / "plugin" / "bin" / "ParityDump.dll"
DEPLOY_NAME = "ParityDump"


def log(msg: str) -> None:
    print(f"[dump_prefabs] {msg}", flush=True)


def build_plugin(game_dir: Path) -> Path:
    log("building ParityDump.dll (dotnet build) ...")
    if not (game_dir / "The Bibites.exe").exists():
        raise SystemExit(f"game not found at {game_dir} (use --game-dir)")
    subprocess.run(["dotnet", "build", str(PLUGIN_DIR / "ParityDump.csproj"), "-c", "Release", "-v:q",
                    f"-p:GameDir={game_dir}"], check=True)
    if not BUILD_OUT.exists():
        raise SystemExit(f"build did not produce {BUILD_OUT}")
    return BUILD_OUT


def deploy(game_dir: Path) -> tuple[Path, bool]:
    plugins = game_dir / "BepInEx" / "plugins"
    dest = plugins / DEPLOY_NAME
    existed = dest.exists()
    if existed:
        log(f"warning: {dest} already exists; it will be replaced and NOT restored")
    dest.mkdir(parents=True, exist_ok=True)
    shutil.copy2(BUILD_OUT, dest / "ParityDump.dll")
    log(f"deployed {dest / 'ParityDump.dll'}")
    return dest, existed


def remove(dest: Path, existed: bool) -> None:
    if existed:
        log(f"leaving pre-existing {dest} in place (not ours to delete)")
        return
    try:
        shutil.rmtree(dest)
        log("plugin removed from the game install")
    except FileNotFoundError:
        pass


def import_api(api_home: Path):
    if not (api_home / "bibites_api").is_dir():
        raise SystemExit(f"bibites_api not found under {api_home}; set BIBITES_API_HOME")
    sys.path.insert(0, str(api_home))
    from bibites_api.live.client import BridgeClient
    from bibites_api.live.runtime import LiveRuntime
    from bibites_api.lab import Lab
    return BridgeClient, LiveRuntime, Lab


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--scenario", default=str(PROJECT_DIR / "testdata" / "real" / "Default.zip"),
                    help="scenario used to start a world (prefabs only exist in the simulation scene)")
    ap.add_argument("--out", default=str(PROJECT_DIR / "testdata" / "parity" / "prefab_dump.json"))
    ap.add_argument("--game-dir", default=str(DEFAULT_GAME_DIR))
    ap.add_argument("--api-home", default=str(DEFAULT_API_HOME))
    ap.add_argument("--port", type=int, default=17800)
    ap.add_argument("--plugin-port", type=int, default=17801)
    ap.add_argument("--work-dir", default=str(PROJECT_DIR / "testdata" / "parity" / "_work"))
    ap.add_argument("--start-timeout", type=float, default=300.0)
    ap.add_argument("--list-only", action="store_true", help="print which plugin files would be deployed and exit")
    args = ap.parse_args()

    game_dir = Path(args.game_dir)
    dest = game_dir / "BepInEx" / "plugins" / DEPLOY_NAME
    if args.list_only:
        print(json.dumps({"plugin_build": str(BUILD_OUT), "deploy_to": str(dest),
                          "already_deployed": dest.exists()}, indent=2))
        return 0

    scenario = Path(args.scenario).absolute()
    if not scenario.exists():
        raise SystemExit(f"scenario {scenario} does not exist")
    out = Path(args.out).absolute()
    BridgeClient, LiveRuntime, Lab = import_api(Path(args.api_home))
    build_plugin(game_dir)
    deployment_existed = False
    runtime = None
    client = None
    try:
        dest, deployment_existed = deploy(game_dir)
        client = BridgeClient(port=args.port, timeout=60.0)
        if client.available():
            raise SystemExit("a game is already running on the bridge port; close it first "
                             "(dump_prefabs needs to start its own game so it can stop it)")
        log("starting hidden game with a world loaded ...")
        runtime = LiveRuntime(work_dir=Path(args.work_dir))
        lab = Lab(runtime, work_dir=Path(args.work_dir), window="hidden")
        client = lab.ensure_game(timeout=args.start_timeout)
        client.new_game(scenario, name=f"parity dump {scenario.stem}")
        client.set_time(paused=True)
        status = client.status()
        log(f"world loaded: game {status.get('gameVersion')}, scene {status.get('scene')}, counts {status.get('counts')}")
        # the plugin's HTTP server comes up in Awake; wait for it
        import httpx
        deadline = time.time() + 60
        dump_url = f"http://127.0.0.1:{args.plugin_port}/d/dump"
        while True:
            try:
                resp = httpx.get(dump_url, timeout=120)
                if resp.status_code == 200:
                    break
            except Exception:
                pass
            if time.time() > deadline:
                raise SystemExit("the parity dump plugin did not answer on /d/dump within 60s")
            time.sleep(0.5)
        dump = resp.json()
        if dump.get("schema") != "rustibites.parity.prefab_dump.v1":
            raise SystemExit(f"unexpected dump schema: {dump.get('schema')}")
        if dump.get("gameVersion") != status.get("gameVersion"):
            log(f"warning: dump gameVersion {dump.get('gameVersion')} != live {status.get('gameVersion')}")
        for key in ("matter_materials", "physics2d", "prefabs", "physics_materials"):
            if key not in dump:
                raise SystemExit(f"dump is missing section {key}")
        dump["provenance"] = {
            "method": "BepInEx plugin (com.rustibites.paritydump 0.1.0) running inside the game, "
                      "reflection over loaded assemblies and prefab assets; plugin copied into "
                      "BepInEx/plugins/ParityDump for the dump and removed afterwards",
            "scenario": str(scenario),
            "bridge": client.ping(),
            "status": status,
            "host": os.environ.get("COMPUTERNAME"),
            "dumped_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        }
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(dump, indent=1, ensure_ascii=False), encoding="utf-8")
        log(f"wrote {out} ({out.stat().st_size} bytes)")
        return 0
    finally:
        try:
            if client is not None:
                client.close()
        except Exception:
            pass
        if runtime is not None:
            try:
                runtime.close()
                log("game stopped")
            except Exception as exc:  # noqa: BLE001
                log(f"could not stop the game: {exc}")
        remove(dest, deployment_existed)


if __name__ == "__main__":
    sys.exit(main())

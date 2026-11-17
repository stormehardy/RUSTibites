#!/usr/bin/env python3
"""capture_reference.py - capture in-world reference screenshots from the hidden game (task-76).

Starts the game through the BibitesAPI lab harness with window="hidden" (NO visible window; the bridge
renders the main camera into an offscreen RenderTexture and returns PNG), loads the real lol save via
`/load`, then takes camera-controlled reference frames:

  reference/full_world.png      fit=true (whole world, 1920x1080)
  reference/population.png      camera centred on the planted zone at orthoSize=120 (mixed population)
  reference/bibite_closeup.png  camera on one bibite at orthoSize=12 (sprite layers/tint)
  reference/zone_pellets_eggs.png  camera over the zone at orthoSize=220 (pellet/egg/zone art)

Each frame records the writer's camera state (x, y, orthographic size) and world state (save path,
tick/time, seed); reference/README.md is generated with those values so comparisons are objective.
The game is stopped in `finally`; no plugin is needed for screenshots and none is deployed.
"""
from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
TOOLS_PARITY = ROOT / "tools" / "parity"
OUT = ROOT / "assets" / "game" / "reference"
DEFAULT_SAVE = ROOT / "testdata" / "real" / "Default.zip"   # canonical (docs/TESTING_POLICY.md); lol reference frames historical

sys.path.insert(0, str(TOOLS_PARITY))
import capture as cap  # noqa: E402


def log(msg: str) -> None:
    print(f"[reference] {msg}", flush=True)


def build_args(ns: argparse.Namespace) -> argparse.Namespace:
    ns.work_dir = str(ROOT / "testdata" / "parity" / "_work")
    ns.port = 17800
    ns.plugin_port = 17801
    ns.no_plugin = True
    ns.attach = False
    ns.start_timeout = 300.0
    ns.keep_game = False
    return ns


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--save", default=str(DEFAULT_SAVE))
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--api-home", default=r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
    args = ap.parse_args()
    out = Path(args.out).absolute()
    out.mkdir(parents=True, exist_ok=True)
    BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(args.api_home))
    session_args = build_args(argparse.Namespace(save=args.save))
    session = cap.WorldSession(session_args, BridgeClient, LiveRuntime, Lab)
    input_path = Path(args.save).absolute()
    is_scenario = False
    try:
        import zipfile
        if zipfile.is_zipfile(input_path):
            with zipfile.ZipFile(input_path) as z:
                names = set(z.namelist())
            is_scenario = "scene.bb8scene" not in names
    except Exception:
        pass
    meta = {"schema": "rustibites.assets.reference.v1",
            "game": {"title": "The Bibites", "version": "0.6.3.1", "unity": "6000.0.44f1"},
            "capture_method": "BibitesBridge GET /screenshot (main camera -> offscreen RenderTexture); "
                              "game window hidden via BibitesAPI lab harness",
            "input": str(input_path), "input_kind": ("scenario_newgame" if is_scenario else "save_load"),
            "captured_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "frames": []}
    try:
        client = session.open()
        if is_scenario:
            client.new_game(input_path, name="reference Default")
        else:
            client.load_world(str(input_path))
        client.set_time(paused=True)
        status = client.status()
        settings = client.get_settings()
        meta["world"] = {"simulated_time_s": float(status.get("simulatedTime") or 0),
                         "paused": bool(status.get("paused")),
                         "counts": status.get("counts"),
                         "sim_tps": (settings.get("independent") or {}).get("simTPS")}
        # a living bibite for the close-up
        try:
            table = client.bibite_table(genes=False, eggs=False)
            target = None
            cols = (table.get("columns") or {}) if isinstance(table, dict) else {}
            if isinstance(table, dict) and cols.get("x"):
                n = len(cols["x"])
                def col(k):
                    return cols.get(k) or [None] * n
                for i, x in enumerate(cols["x"]):
                    y = col("y")[i]
                    if x is not None and y is not None:
                        target = {"id": col("id")[i] if col("id")[i] is not None else col("h")[i],
                                  "x": float(x), "y": float(y),
                                  "species": col("species")[i], "generation": col("generation")[i],
                                  "size": col("size")[i]}
                        break
            entries = table if isinstance(table, list) else []
            if target is None:
                living = [e for e in entries if not e.get("is_corpse") and not e.get("egg")]
                target = next((e for e in living if e.get("x") is not None and e.get("y") is not None), None)
        except Exception as exc:  # noqa: BLE001
            log(f"bibite table unavailable ({exc}); close-up will use the world centre")
            target = None
        shots = [
            ("full_world.png", {"fit": True}, 1920, 1080),
            ("population.png", {"x": 0.0, "y": 0.0, "size": 120.0}, 1920, 1080),
            ("zone_pellets_eggs.png", {"x": 0.0, "y": 0.0, "size": 230.0}, 1920, 1080),
        ]
        if target is not None:
            shots.append(("bibite_closeup.png", {"x": float(target["x"]), "y": float(target["y"]), "size": 12.0}, 1280, 720))
        for name, body, w, h in shots:
            cam = client.set_camera(**body)
            png = client.screenshot(w, h)
            path = out / name
            path.write_bytes(png)
            rel = (out / name).relative_to(ROOT).as_posix() if (out / name).is_relative_to(ROOT) else name
            frame = {"file": rel, "width": w, "height": h,
                     "camera": cam, "asked": body}
            if name == "bibite_closeup.png" and target is not None:
                frame["bibite"] = {k: target.get(k) for k in ("id", "x", "y", "species", "generation", "size", "tag") if target.get(k) is not None}
            meta["frames"].append(frame)
            log(f"wrote {path} camera={cam}")
    finally:
        session.close()
    (out / "reference.json").write_text(json.dumps(meta, indent=2), encoding="utf-8")
    lines = ["# Reference in-world frames (The Bibites 0.6.3.1 / Unity 6000.0.44f1)", "",
             "Captured with **no visible window**: the game ran through the BibitesAPI lab harness with "
             "`window=\"hidden\"`; the bridge renders the main camera into an offscreen RenderTexture "
             "(`GET /screenshot`) and returns a PNG. No BepInEx plugin was installed for these captures.", "",
             f"- input: `{meta['input']}` ({meta['input_kind']})",
             f"- world at capture: simulatedTime = {meta['world']['simulated_time_s']:.3f} s, paused = "
             f"{meta['world']['paused']}, counts = {json.dumps(meta['world']['counts'])}",
             f"- captured (UTC): {meta['captured_utc']}", "",
             "| frame | size | camera x, y | orthographic size | note |", "|---|---|---|---|---|"]
    notes = {"full_world.png": "fit=true whole world (1920x1080)",
             "population.png": "planted zone at orthoSize 120 (mixed population, pellets, zone ring)",
             "zone_pellets_eggs.png": "zone + pellets + eggs at orthoSize 230",
             "bibite_closeup.png": "one living bibite at orthoSize 12 (sprite layers/tint; camera on its position)"}
    for f in meta["frames"]:
        cam = f.get("camera") or {}
        lines.append(f"| [![{f['file'].split('/')[-1]}]({f['file'].split('/')[-1]})]"
                     f"({f['file'].split('/')[-1]}) | {f['width']}x{f['height']} | "
                     f"{cam.get('x')}, {cam.get('y')} | {cam.get('size')} | {notes.get(f['file'].split('/')[-1], '')} |")
    lines += ["", "The world camera is orthographic; `orthographic size` is half the visible height in world "
                  "units. To reproduce a frame, load the same save (read-only from `testdata/real/saves/`) and "
                  "call `POST /camera {{x, y, size}}` (or `fit: true`) before `GET /screenshot?w=&h=`."]
    (out / "README.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    log(f"reference frames: {len(meta['frames'])} -> {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

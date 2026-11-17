"""Round-trip acceptance: load a RUSTibites-written save in the real game (task-34).

Starts a hidden BibitesAPI lab game, loads `--save` through the live bridge
(`POST /load`), runs a few ticks, asks the game's own converter to parse
`--template`, checks the game log for new exceptions, and stops the game it
started. Never writes to AppData/saves beyond what the lab's load does.

Usage:
  python tools/roundtrip/load_rust_save.py \
    --save testdata/roundtrip/rust_save.zip \
    --template testdata/roundtrip/rust_template.bb8template \
    --ticks 0.5
"""
from __future__ import annotations

import argparse
import json
import math
import os
import sys
import traceback
import zipfile
from pathlib import Path

DEFAULT_API_HOME = Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
PLAYER_LOG = Path(os.environ.get("LOCALAPPDATA", "")) / "Low" / "The Bibites" / "The Bibites" / "Player.log"


def import_api(api_home: Path):
    if not (api_home / "bibites_api").is_dir():
        raise SystemExit(f"bibites_api not found under {api_home}")
    sys.path.insert(0, str(api_home))
    from bibites_api.live.client import BridgeClient, BridgeError  # noqa: F401
    from bibites_api.live.runtime import LiveRuntime
    from bibites_api.lab import Lab
    return BridgeClient, BridgeError, LiveRuntime, Lab


def read_save_scene(save: Path) -> dict:
    """Reads the saved `scene.bb8scene` metadata (simulatedTime/version/counts)."""
    try:
        with zipfile.ZipFile(save) as archive:
            name = next(
                (entry for entry in archive.namelist() if entry.replace("\\", "/") == "scene.bb8scene"),
                None,
            )
            if name is None:
                return {"error": "scene.bb8scene missing"}
            raw = archive.read(name)
        if raw.startswith(b"\xef\xbb\xbf"):
            raw = raw[3:]
        scene = json.loads(raw.decode("utf-8"))
    except Exception as error:  # noqa: BLE001 - diagnostics only
        return {"error": repr(error)}
    return {
        "version": scene.get("version"),
        "simulatedTime": scene.get("simulatedTime"),
        "nBibites": scene.get("nBibites"),
        "nPellets": scene.get("nPellets"),
        "pheroTowers": len(scene.get("pheroTowers") or []),
        "radTowers": len(scene.get("radTowers") or []),
        "colorSelectors": len(scene.get("colorSelectors") or []),
    }


def read_log_tail(lines: int = 200) -> str:
    try:
        data = PLAYER_LOG.read_text(encoding="utf-8", errors="replace").splitlines()
        return "\n".join(data[-lines:])
    except OSError:
        return ""


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--save", default="testdata/roundtrip/rust_save.zip")
    ap.add_argument("--template", default="testdata/roundtrip/rust_template.bb8template")
    ap.add_argument("--ticks", type=float, default=0.5, help="sim seconds to run after loading")
    ap.add_argument("--speed", type=float, default=10.0)
    ap.add_argument("--timeout", type=float, default=300.0)
    ap.add_argument("--api-home", default=os.environ.get("BIBITES_API_HOME", str(DEFAULT_API_HOME)))
    ap.add_argument("--work-dir", default=str(Path("testdata/roundtrip/_lab")))
    ap.add_argument("--out", default="", help="acceptance.json path to update (optional)")
    args = ap.parse_args()

    save = Path(args.save).resolve()
    template = Path(args.template).resolve()
    if not save.is_file():
        raise SystemExit(f"save not found: {save}")
    if not template.is_file():
        raise SystemExit(f"template not found: {template}")

    _, BridgeError, LiveRuntime, Lab = import_api(Path(args.api_home))
    runtime = LiveRuntime(work_dir=Path(args.work_dir))
    runtime.work.mkdir(parents=True, exist_ok=True)
    client = None
    summary: dict = {
        "save": str(save),
        "template": str(template),
        "save_scene": read_save_scene(save),
    }
    try:
        lab = Lab(runtime, work_dir=Path(args.work_dir), window="hidden")
        client = lab.ensure_game(timeout=args.timeout)
        summary["ping"] = client.ping()
        before = client.status()
        summary["status_before"] = {
            key: before.get(key) for key in ("inSimulation", "counts", "simTime", "loadsDone", "sceneLoads")
        }
        log_before = read_log_tail()

        loaded = client.load_world(save, timeout=args.timeout)
        summary["load_ok"] = True
        # Parity hand-off: freeze the freshly loaded world and record the exact
        # load-state clock (parity tick 0 is defined relative to this value).
        try:
            client.post("/time", {"paused": True})
            summary["paused_after_load"] = True
        except BridgeError as error:
            summary["paused_after_load"] = f"error: {error}"
        status_after_load = client.status()
        run_state_after_load = client.run_state()
        summary["status_after_load"] = {
            key: status_after_load.get(key)
            for key in (
                "inSimulation",
                "counts",
                "simTime",
                "simulatedTime",
                "simSeconds",
                "totalTime",
                "loadsDone",
                "sceneLoads",
            )
            if key in status_after_load
        }
        summary["status_after_load_full"] = status_after_load
        summary["run_state_after_load"] = {
            key: run_state_after_load.get(key)
            for key in ("state", "simTime", "simSeconds", "tick", "counts")
            if key in run_state_after_load
        }
        summary["load_clock"] = {
            "save_scene_simulatedTime": summary["save_scene"].get("simulatedTime"),
            "status_simulatedTime": status_after_load.get("simulatedTime")
            or status_after_load.get("simTime"),
            "run_state_simTime": run_state_after_load.get("simTime"),
            "counts": status_after_load.get("counts"),
        }

        if args.ticks > 0:
            base_sim = float(loaded.get("simTime") or client.status().get("simTime") or 0.0)
            until = {"simTime": base_sim + args.ticks}
            summary["run_until"] = until
            client.run_start(until, speed=args.speed, sample=None)
            run_state = client.run_wait(timeout=args.timeout)
            summary["run_state"] = run_state.get("state")
            summary["run_simTime"] = run_state.get("simTime")
            final = client.status()
            summary["status_after_run"] = {
                key: final.get(key) for key in ("inSimulation", "counts", "simTime", "loadsDone")
            }

        try:
            upgrade = client.upgrade_bibite_file(template)
            summary["template_upgrade"] = {
                "kind": upgrade.get("kind"),
                "fromVersion": upgrade.get("fromVersion"),
                "updated": upgrade.get("updated"),
                "has_template": bool(upgrade.get("template")),
            }
        except BridgeError as error:
            summary["template_upgrade_error"] = str(error)

        log_after = read_log_tail()
        new_lines = log_after[len(log_before):] if log_after.startswith(log_before) else log_after
        errors = [
            line
            for line in new_lines.splitlines()
            if ("Exception" in line or "Error" in line or "error CS" in line)
        ]
        summary["player_log_new_error_lines"] = errors[-20:]
        summary["player_log_checked"] = str(PLAYER_LOG)
        counts = (summary.get("status_after_run") or summary.get("status_after_load") or {}).get("counts") or {}
        summary["counts"] = counts
        summary["game_loaded_and_ran"] = bool(
            summary.get("load_ok")
            and (summary.get("status_after_run") or summary.get("status_after_load") or {}).get("inSimulation")
            and summary.get("run_state", "done") not in ("error",)
            and not errors
        )
        if args.out:
            out_path = Path(args.out)
            if not out_path.is_absolute():
                out_path = Path.cwd() / out_path
            out_path.parent.mkdir(parents=True, exist_ok=True)
            existing: dict = {}
            if out_path.is_file():
                try:
                    existing = json.loads(out_path.read_text(encoding="utf-8"))
                except (OSError, ValueError):
                    existing = {}
            clock = summary.get("load_clock") or {}
            if clock.get("status_simulatedTime") is not None and clock.get("save_scene_simulatedTime") is not None:
                save_t = clock["save_scene_simulatedTime"]
                delta = clock["status_simulatedTime"] - save_t
                ulp = 2.0 ** (math.floor(math.log2(abs(save_t))) - 23) if save_t else 0.0
                clock["status_minus_save_seconds"] = delta
                clock["parity_clean"] = abs(delta) <= 1e-5
                clock["parity_clean_reason"] = (
                    "load_time_quantisation_f32: save keeps simulatedTime as f64, loaded clock is the nearest f32; "
                    "nothing ran, only the clock label differs"
                )
                clock["quantisation"] = {
                    "kind": "load_time_quantisation_f32",
                    "f32_ulp_at_save_seconds": ulp,
                    "delta_in_ulp": abs(delta) / ulp if ulp else None,
                }
            existing.update(
                {
                    "result": "YES" if summary.get("game_loaded_and_ran") else "NO",
                    "statement": "Rust-written save loads and runs in The Bibites 0.6.3.1",
                    "save_scene": summary.get("save_scene"),
                    "load_clock": summary.get("load_clock"),
                    "status_after_load": summary.get("status_after_load"),
                    "status_after_load_full": summary.get("status_after_load_full"),
                    "run_state_after_load": summary.get("run_state_after_load"),
                    "counts": summary.get("counts"),
                    "player_log_new_error_lines": summary.get("player_log_new_error_lines"),
                    "template_upgrade": summary.get("template_upgrade"),
                    "game_version": "0.6.3.1",
                    "interpretation": {
                        "scene_nBibites": "display-only: saved bibite entries include corpses; eggs are separate files; do not compare scene.nBibites with living-bibite counts.",
                        "version": "source scene version may be older (e.g. 0.6.3a3) while the installed game is 0.6.3.1; the game migrates older section metadata on load. Template accepted at 0.6.3.1 with updated=false is the strongest current-format signal.",
                        "load_clock": "save_scene.simulatedTime is the authoritative tick-0 reference; post-load /status may differ by f32 quantisation of the clock label (nothing ran). parity_clean stays false under the strict |status - saved| <= 1e-5 s capture definition; this harness verifies load/run only.",
                    },
                    "checked_at": __import__("datetime").datetime.now(
                        __import__("datetime").timezone.utc
                    ).isoformat(),
                }
            )
            out_path.write_text(json.dumps(existing, indent=2), encoding="utf-8")
            summary["acceptance_out"] = str(out_path)
        return 0 if summary["game_loaded_and_ran"] else 1
    except BridgeError as error:
        summary["bridge_error"] = str(error)
        return 2
    except Exception as error:  # noqa: BLE001 - report anything the harness hits
        summary["harness_error"] = repr(error)
        summary["traceback"] = traceback.format_exc()
        return 3
    finally:
        if client is not None:
            try:
                runtime.close()
            except Exception:  # noqa: BLE001 - cleanup best effort
                pass
        print(json.dumps(summary, indent=2, default=str))


if __name__ == "__main__":
    sys.exit(main())

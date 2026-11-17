"""Game-load verification for the compat fixture pack (parity-dev harness).

Starts a hidden BibitesAPI lab game, loads each Rust-written fixture, records the
counts/sim time, re-saves saves/scenarios, asks the game's converter to parse the
template, scans Player.log for new exceptions, and writes game_report.json. Stops
the game it started. Never writes AppData beyond the lab's own load; no install
changes.

Usage:
  python tools/roundtrip/load_compat_pack.py --dir testdata/parity/compat
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
import threading
from pathlib import Path

DEFAULT_API_HOME = Path(r"C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI")
PLAYER_LOG = Path(os.environ.get("LOCALAPPDATA", "")) / "Low" / "The Bibites" / "The Bibites" / "Player.log"


def log_tail(lines: int = 300) -> str:
    try:
        return "\n".join(PLAYER_LOG.read_text(encoding="utf-8", errors="replace").splitlines()[-lines:])
    except OSError:
        return ""



def kill_game_processes() -> None:
    """Force-close the hidden game if ensure_game/load hangs; harness cleanup may
    otherwise leave Unity alive and block the next lab start."""
    for image in ("The Bibites.exe", "UnityCrashHandler64.exe"):
        try:
            subprocess.run(["taskkill", "/F", "/IM", image], capture_output=True, timeout=20)
        except (OSError, subprocess.SubprocessError):
            pass


def hard_timeout(call, seconds: float, label: str):
    """Run a blocking lab/bridge call with an external wall clock; on timeout kill
    the game processes so the next fixture starts with a clean lab."""
    outcome = {}
    def worker():
        try:
            outcome["value"] = call()
        except Exception as error:  # noqa: BLE001 - forwarded to caller
            outcome["error"] = error
    thread = threading.Thread(target=worker, daemon=True)
    thread.start()
    thread.join(seconds)
    if thread.is_alive():
        kill_game_processes()
        raise TimeoutError(f"{label} exceeded {seconds:.0f}s; killed game processes")
    if "error" in outcome:
        raise outcome["error"]
    return outcome.get("value")



class LifecycleWatchdog:
    """Whole-fixture wall clock: covers lab launch, bridge calls, status and
    re-save. On expiry it kills The Bibites/UnityCrashHandler and force-exits the
    runner, so a blocked bridge thread can never leave the lab held."""

    def __init__(self) -> None:
        self._timer = None

    def arm(self, seconds: float, label: str) -> None:
        self.cancel()
        def fire() -> None:
            print(f"compat: WATCHDOG timeout in {label} after {seconds:.0f}s; killing game and exiting", flush=True)
            kill_game_processes()
            os._exit(124)
        self._timer = threading.Timer(max(1.0, seconds), fire)
        self._timer.daemon = True
        self._timer.start()

    def cancel(self) -> None:
        if self._timer is not None:
            self._timer.cancel()
            self._timer = None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", default="testdata/parity/compat")
    ap.add_argument("--out", default="testdata/parity/compat/game_report.json")
    ap.add_argument("--resave-dir", default="testdata/parity/compat/_resaved")
    ap.add_argument("--api-home", default=os.environ.get("BIBITES_API_HOME", str(DEFAULT_API_HOME)))
    ap.add_argument("--timeout", type=float, default=300.0)
    ap.add_argument("--hard-timeout", type=float, default=120.0)
    ap.add_argument("--one-fixture", default="", help="run only this fixture filename")
    ap.add_argument("--selftest-timeout", action="store_true", help="regression selftest: watchdog must exit 124")
    ap.add_argument("--start-timeout", type=float, default=300.0)
    ap.add_argument("--fixture-timeout", type=float, default=300.0)
    ap.add_argument("--settle", type=float, default=2.0, help="seconds to settle after a successful load before re-save")
    ap.add_argument("--resave-retries", type=int, default=2)
    ap.add_argument("--require-resave", action="store_true", help="fail the fixture when re-save cannot complete (default: report but accept load)")
    args = ap.parse_args()

    if args.selftest_timeout:
        watchdog = LifecycleWatchdog()
        watchdog.arm(2.0, "selftest")
        print("compat: selftest blocking forever (watchdog should exit 124)", flush=True)
        threading.Event().wait(300)
        return 0
    folder = Path(args.dir).resolve()
    manifest = json.loads((folder / "manifest.json").read_text(encoding="utf-8"))
    sys.path.insert(0, args.api_home)
    from bibites_api.live.runtime import LiveRuntime
    from bibites_api.lab import Lab

    runtime = LiveRuntime(work_dir=Path("target/compat_lab"))
    runtime.work.mkdir(parents=True, exist_ok=True)
    resave_dir = Path(args.resave_dir)
    resave_dir.mkdir(parents=True, exist_ok=True)
    client = None
    report = {"schema": "rustibites.compat.game_report.v1", "game_version": "0.6.3.1", "fixtures": [], "player_log": str(PLAYER_LOG)}
    try:
        watchdog = LifecycleWatchdog()
        watchdog.arm(args.hard_timeout, "lab startup")
        print(f"compat: starting hidden lab (work={runtime.work}, player_log={PLAYER_LOG})", flush=True)
        client = hard_timeout(
            lambda: Lab(runtime, work_dir=runtime.work, window="hidden").ensure_game(
                timeout=args.start_timeout
            ),
            args.hard_timeout,
            "lab start",
        )
        watchdog.cancel()
        print("compat: lab ready", flush=True)
        report["ping"] = client.ping()
        fixtures = [f for f in manifest["fixtures"] if not args.one_fixture or f["name"] == args.one_fixture]
        print(f"compat: {len(fixtures)} fixture(s)", flush=True)
        for entry in fixtures:
            if entry["kind"] == "checkpoint-sidecar":
                continue
            path = folder / entry["name"]
            watchdog.arm(args.hard_timeout, f"fixture {entry['name']}")
            print(f"compat: fixture {entry['name']} ({entry['kind']})", flush=True)
            result = {"name": entry["name"], "kind": entry["kind"], "path": str(path)}
            before = log_tail()
            try:
                if entry["kind"] == "template":
                    reply = hard_timeout(lambda: client.upgrade_bibite_file(path), args.hard_timeout, entry["name"])
                    result.update(accepted=True, kind=reply.get("kind"), from_version=reply.get("fromVersion"), updated=reply.get("updated"))
                elif entry["kind"] == "scenario":
                    hard_timeout(lambda: client.new_game(path, timeout=args.fixture_timeout), args.hard_timeout, entry["name"])
                    status = client.status()
                    result.update(accepted=True, in_simulation=status.get("inSimulation"), counts=status.get("counts"), simulated_time=status.get("simulatedTime"))
                else:
                    hard_timeout(lambda: client.load_world(path, timeout=args.fixture_timeout), args.hard_timeout, entry["name"])
                    status = client.status()
                    result.update(accepted=True, in_simulation=status.get("inSimulation"), counts=status.get("counts"), simulated_time=status.get("simulatedTime"))
                    resaved = resave_dir / entry["name"]
                    resaved.parent.mkdir(parents=True, exist_ok=True)
                    # Give the game a moment after load; the bridge can drop
                    # right after a large world load on this harness.
                    time.sleep(max(0.0, args.settle))
                    last_error = None
                    for attempt in range(args.resave_retries + 1):
                        try:
                            hard_timeout(
                                lambda: client.save_world(resaved),
                                args.hard_timeout,
                                f"re-save {entry['name']}",
                            )
                            last_error = None
                            break
                        except Exception as error:  # noqa: BLE001 - retry bridge closes
                            last_error = error
                            result.setdefault("resave_attempt_errors", []).append(repr(error))
                            print(
                                f"compat: re-save {entry['name']} attempt {attempt + 1} failed: {error!r}",
                                flush=True,
                            )
                            if "10054" in repr(error) or "BridgeUnavailable" in repr(error):
                                # Bridge closed after load: retrying (or pinging) only
                                # hangs until the watchdog; record and continue.
                                break
                            if attempt < args.resave_retries:
                                time.sleep(1.5 * (attempt + 1))
                                try:
                                    client.ping()
                                except Exception:  # noqa: BLE001 - bridge may be gone
                                    pass
                    if resaved.is_file():
                        result.update(resaved=str(resaved), resaved_bytes=resaved.stat().st_size, resave_ok=True)
                    else:
                        result.update(resaved=None, resave_ok=False, resave_error=repr(last_error))
                        if args.require_resave:
                            raise TimeoutError(f"re-save failed: {last_error!r}")
            except Exception as error:  # noqa: BLE001 - report to parity-dev
                result.update(accepted=False, error=repr(error))
            after = log_tail()
            result["player_log_new_errors"] = [line for line in after[len(before):].splitlines() if "Exception" in line or "Error" in line]
            result["expected"] = entry.get("expected")
            report["fixtures"].append(result)
            watchdog.cancel()
        return 0 if all(item.get("accepted") for item in report["fixtures"]) else 1
    finally:
        try:
            watchdog.cancel()
        except NameError:
            pass
        if client is not None:
            try:
                runtime.close()
            except Exception:  # noqa: BLE001
                pass
        Path(args.out).write_text(json.dumps(report, indent=2), encoding="utf-8")
        print(json.dumps(report, indent=2))


if __name__ == "__main__":
    sys.exit(main())

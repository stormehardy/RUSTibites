"""Round-trip acceptance 2: start a RUSTibites-written scenario in the real game.

Starts a hidden BibitesAPI lab game, starts a NEW world from the Rust-written
scenario zip (`client.new_game` = scenario menu's Start), records zone/spawn/
seeding state, runs a few ticks, scans Player.log for new exceptions, then
repeats with the original scenario in the same session for comparison. Stops
the game it started. Never writes the game install/AppData (lab mode).

Usage:
  python tools/roundtrip/start_rust_scenario.py \
    --rust testdata/roundtrip/rust_scenario.zip --original testdata/real/Default.zip
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import traceback
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


def log_tail(lines: int = 300) -> str:
    try:
        return "\n".join(
            PLAYER_LOG.read_text(encoding="utf-8", errors="replace").splitlines()[-lines:]
        )
    except OSError:
        return ""


def snapshot(client) -> dict:
    status = client.status()
    zones = client.zones()
    sim_size = float(status.get("simulationSize") or 50000.0)
    region = {"x0": -sim_size, "y0": -sim_size, "x1": sim_size, "y1": sim_size}
    try:
        pellets = client.post("/pellets/select", {"region": region, "limit": 1})
    except Exception as error:  # noqa: BLE001 - diagnostics
        pellets = {"error": repr(error)}
    by_bucket: dict = {}
    try:
        for bucket in [zone.get("name") for zone in zones if zone.get("name")] + ["free pellets"]:
            reply = client.post(
                "/pellets/select", {"region": region, "bucket": bucket, "limit": 1}
            )
            by_bucket[bucket] = {
                "total": reply.get("total"),
                "plant": reply.get("plant"),
                "meat": reply.get("meat"),
                "amount": reply.get("amount"),
            }
    except Exception as error:  # noqa: BLE001 - diagnostics
        by_bucket["error"] = repr(error)
    return {
        "inSimulation": status.get("inSimulation"),
        "scene": status.get("scene"),
        "simulatedTime": status.get("simulatedTime"),
        "counts": status.get("counts"),
        "zone_count": len(zones),
        "zones": [
            {
                key: zone.get(key)
                for key in ("name", "material", "distribution", "radius", "posX", "posY")
                if key in zone
            }
            for zone in zones
        ],
        "pellets": {
            "total": pellets.get("total"),
            "plant": pellets.get("plant"),
            "meat": pellets.get("meat"),
            "amount": pellets.get("amount"),
            "by_bucket": by_bucket,
            "region": region,
            "t": status.get("simulatedTime"),
        },
        "status_fulls": status,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--rust", default="testdata/roundtrip/rust_scenario.zip")
    ap.add_argument("--original", default="testdata/real/Default.zip")
    ap.add_argument("--ticks", type=float, default=1.0)
    ap.add_argument("--timeout", type=float, default=300.0)
    ap.add_argument("--api-home", default=os.environ.get("BIBITES_API_HOME", str(DEFAULT_API_HOME)))
    ap.add_argument("--work-dir", default=str(Path("testdata/roundtrip/_lab")))
    ap.add_argument("--out", default="testdata/roundtrip/scenario_acceptance.json")
    args = ap.parse_args()

    rust = Path(args.rust).resolve()
    original = Path(args.original).resolve()
    for path in (rust, original):
        if not path.is_file():
            raise SystemExit(f"scenario not found: {path}")

    _, BridgeError, LiveRuntime, Lab = import_api(Path(args.api_home))
    runtime = LiveRuntime(work_dir=Path(args.work_dir))
    runtime.work.mkdir(parents=True, exist_ok=True)
    client = None
    summary: dict = {"rust_scenario": str(rust), "original_scenario": str(original)}
    try:
        client = Lab(runtime, work_dir=Path(args.work_dir), window="hidden").ensure_game(
            timeout=args.timeout
        )
        summary["ping"] = client.ping()

        log_before = log_tail()
        client.new_game(rust, timeout=args.timeout)
        try:
            client.post("/time", {"paused": True})
        except BridgeError:
            pass
        summary["rust"] = snapshot(client)
        base = summary["rust"]["simulatedTime"] or 0.0
        client.run_start({"simTime": float(base) + args.ticks}, speed=10.0, sample=None)
        run_state = client.run_wait(timeout=args.timeout)
        summary["rust_run"] = {"state": run_state.get("state"), "simTime": run_state.get("simTime")}
        summary["rust_after_run"] = snapshot(client)
        summary["rust_after_run"].pop("status_fulls", None)
        log_after_rust = log_tail()

        client.new_game(original, timeout=args.timeout)
        try:
            client.post("/time", {"paused": True})
        except BridgeError:
            pass
        summary["original"] = snapshot(client)
        log_after_original = log_tail()

        new_rust_errors = [
            line
            for line in log_after_rust[len(log_before):].splitlines()
            if "Exception" in line or "Error" in line or "error CS" in line
        ]
        new_original_errors = [
            line
            for line in log_after_original[len(log_after_rust):].splitlines()
            if "Exception" in line or "Error" in line or "error CS" in line
        ]
        summary["player_log_new_errors_rust"] = new_rust_errors[-20:]
        summary["player_log_new_errors_original"] = new_original_errors[-20:]
        summary["player_log"] = str(PLAYER_LOG)

        rust_zone = summary["rust"]["zone_count"]
        orig_zone = summary["original"]["zone_count"]
        rust_counts = summary["rust"]["counts"] or {}
        orig_counts = summary["original"]["counts"] or {}
        summary["comparison"] = {
            "zone_count_equal": rust_zone == orig_zone,
            "counts_rust": rust_counts,
            "counts_original": orig_counts,
            "bibite_count_equal": rust_counts.get("bibites") == orig_counts.get("bibites"),
            "pellets_rust": summary["rust"]["pellets"],
            "pellets_original": summary["original"]["pellets"],
        }
        rust_pellets = summary["rust"]["pellets"]
        original_pellets = summary["original"]["pellets"]

        def within_sigma(rust: float, original: float) -> bool:
            # Counts follow sqrt(N) RNG spread; allow 3 sigma (and a floor of 10)
            # so a correct pair of starts does not flake ~7% of runs.
            rust, original = float(rust), float(original)
            return abs(rust - original) <= max(10.0, 3.0 * max(rust, original) ** 0.5)

        counts_within = all(
            within_sigma(rust_pellets.get(key) or 0, original_pellets.get(key) or 0)
            for key in ("total", "plant", "meat")
        )
        bucket_names = set((rust_pellets.get("by_bucket") or {})) & set(
            (original_pellets.get("by_bucket") or {})
        ) - {"error"}
        buckets_within = all(
            within_sigma(
                (rust_pellets.get("by_bucket", {}).get(name) or {}).get("total") or 0,
                (original_pellets.get("by_bucket", {}).get(name) or {}).get("total") or 0,
            )
            for name in bucket_names
        )
        rust_amount = float(rust_pellets.get("amount") or 0.0)
        original_amount = float(original_pellets.get("amount") or 0.0)
        amount_band = 0.02 * max(rust_amount, original_amount, 1e-9)
        summary["comparison"]["pellet_counts_within_band"] = counts_within
        summary["comparison"]["pellet_buckets_within_band"] = buckets_within
        summary["comparison"]["pellet_amounts_within_band"] = (
            abs(rust_amount - original_amount) <= amount_band
        )
        summary["comparison"]["pellet_amount_band"] = amount_band
        summary["comparison"]["amount_per_pellet"] = {
            "rust": rust_amount / rust_pellets.get("total", 1) if rust_pellets.get("total") else None,
            "original": original_amount / original_pellets.get("total", 1)
            if original_pellets.get("total")
            else None,
        }
        summary["comparison"]["plant_biomass_by_bucket"] = {
            "rust": {
                name: (value or {}).get("amount")
                for name, value in (rust_pellets.get("by_bucket") or {}).items()
                if isinstance(value, dict)
            },
            "original": {
                name: (value or {}).get("amount")
                for name, value in (original_pellets.get("by_bucket") or {}).items()
                if isinstance(value, dict)
            },
        }
        summary["comparison"]["pellet_count_band_note"] = (
            "seeded biomass amount within 2%; pellet/bucket counts within 3 sigma (floor 10); per-pellet identity not compared (unseeded planting RNG). 1.6%-level amount wobble can come from a start sampled a frame earlier/later; repeats/median are the tie-breaker."
        )
        summary["seeding_pass"] = bool(
            counts_within and buckets_within and abs(rust_amount - original_amount) <= amount_band
        )
        summary["game_started_and_ran"] = bool(
            summary["rust"]["inSimulation"]
            and not new_rust_errors
            and summary["rust_run"]["state"] not in ("error",)
            and rust_zone == orig_zone
        )
        summary["scenario_acceptance"] = bool(
            summary["game_started_and_ran"] and summary.get("seeding_pass")
        )
        summary["result"] = "YES" if summary["scenario_acceptance"] else "NO"
        summary["statement"] = (
            "Rust-written scenario bundle starts and runs in The Bibites 0.6.3.1"
            if summary["game_started_and_ran"]
            else "Rust-written scenario bundle did not start cleanly"
        )

        out = Path(args.out)
        if not out.is_absolute():
            out = Path.cwd() / out
        out.parent.mkdir(parents=True, exist_ok=True)
        existing = {}
        if out.is_file():
            try:
                existing = json.loads(out.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                existing = {}
        existing.update(summary)
        out.write_text(json.dumps(existing, indent=2), encoding="utf-8")
        summary["acceptance_out"] = str(out)
        return 0 if summary.get("scenario_acceptance") else 1
    except BridgeError as error:
        summary["bridge_error"] = str(error)
        return 2
    except Exception as error:  # noqa: BLE001
        summary["harness_error"] = repr(error)
        summary["traceback"] = traceback.format_exc()
        return 3
    finally:
        if client is not None:
            try:
                runtime.close()
            except Exception:  # noqa: BLE001
                pass
        print(json.dumps(summary, indent=2, default=str))


if __name__ == "__main__":
    sys.exit(main())

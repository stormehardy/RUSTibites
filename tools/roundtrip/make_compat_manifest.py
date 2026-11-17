"""Writes manifest.json for the game-load verification fixture pack.

Usage: python tools/roundtrip/make_compat_manifest.py --dir testdata/parity/compat
"""
from __future__ import annotations

import argparse
import hashlib
import json
import zipfile
from datetime import datetime, timezone
from pathlib import Path

TOOLS = ("radTowers", "pheroTowers", "colorSelectors")


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def read_json(z: zipfile.ZipFile, name: str):
    try:
        raw = z.read(name)
    except KeyError:
        return None
    if raw.startswith(b"\xef\xbb\xbf"):
        raw = raw[3:]
    return json.loads(raw.decode("utf-8"))


def describe(path: Path) -> dict:
    info = {
        "name": path.name,
        "path": path.as_posix(),
        "bytes": path.stat().st_size,
        "sha256": sha256(path),
    }
    if path.suffix == ".bb8state":
        info["kind"] = "checkpoint-sidecar"
        return info
    if path.suffix == ".bb8template":
        data = json.loads(path.read_text(encoding="utf-8-sig"))
        info.update(
            kind="template",
            version=data.get("version"),
            expected={"genes": len(data.get("genes", {})), "nodes": len(data.get("nodes", []))},
            load_command="client.upgrade_bibite_file(path)",
            game_expectations=["converter accepts kind=template", "no Player.log exception"],
        )
        return info
    if not zipfile.is_zipfile(path):
        info["kind"] = "unknown"
        return info
    with zipfile.ZipFile(path) as z:
        names = z.namelist()
        scene = read_json(z, "scene.bb8scene")
        if scene is None:
            # Scenario bundle.
            settings = read_json(z, "settings.bb8settings") or {}
            info.update(
                kind="scenario",
                version=(read_json(z, "scenario.info") or {}).get("version"),
                expected={
                    "zones": len(settings.get("zones", [])),
                    "zone_groups": len(settings.get("zoneGroups", [])),
                    "spawn_rows": len(settings.get("bibites", [])),
                    "changers": len(settings.get("settingsChangers", [])),
                    "bundled_templates": sum(
                        1 for n in names if n.lower().endswith((".bb8template", ".bb8"))
                    ),
                },
                load_command="client.new_game(path)",
                resave_command="client.save_world(<tmp>/resave.zip)",
                game_expectations=["inSimulation", "spawner starts", "no exception"],
            )
            return info
        bibite_ids, egg_ids = [], []
        for name in names:
            low = name.replace("\\", "/").lower()
            if low.startswith("bibites/") and low.endswith(".bb8"):
                body = read_json(z, name) or {}
                bibite_ids.append((body.get("body") or {}).get("id"))
            elif low.startswith("eggs/") and low.endswith(".bb8"):
                egg_ids.append(((read_json(z, name) or {}).get("egg") or {}).get("id"))
        species = read_json(z, "speciesData.json")
        info.update(
            kind="save",
            version=scene.get("version"),
            expected={
                "bibite_entries": len(bibite_ids),
                "egg_entries": len(egg_ids),
                "nBibites_display": scene.get("nBibites"),
                "nPellets_display": scene.get("nPellets"),
                "rad_towers": len(scene.get("radTowers") or []),
                "phero_towers": len(scene.get("pheroTowers") or []),
                "color_killers": len(scene.get("colorSelectors") or []),
                "plant_pellets": sum(
                    len(group.get("pellets", []))
                    for group in (read_json(z, "pellets.bb8scene") or {}).get("pellets", [])
                ),
                "pheromones": len(
                    (read_json(z, "pheromones.bb8scene") or {}).get("pheromones", [])
                ),
                "species_records": len((species or {}).get("recordedSpecies", [])),
                "zones": len((read_json(z, "settings.bb8settings") or {}).get("zones", [])),
                "data_bin_bytes": len(z.read("data.bin")) if "data.bin" in names else 0,
                "templates": sum(1 for n in names if n.lower().startswith("templates/")),
            },
            entity_ids={"bibites": bibite_ids, "eggs": egg_ids},
            load_command="client.load_world(path)",
            resave_command="client.save_world(<tmp>/resave.zip)",
            game_expectations=[
                "inSimulation",
                "counts match living/corpse split",
                "re-save succeeds",
                "no Player.log exception",
                "sidecar is ignored (if present)",
            ],
        )
        return info


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", default="testdata/parity/compat")
    args = ap.parse_args()
    folder = Path(args.dir)
    priority = [
        "rust_lol_save.zip",
        "rust_state_save.zip",
        "rust_eggs_save.zip",
        "rust_corpses_save.zip",
        "rust_tools_phero_save.zip",
        "rust_default_scenario_save.zip",
        "rust_default_scenario.zip",
        "rust_ring_scenario.zip",
        "rust_template.bb8template",
    ]
    paths = [p for p in folder.iterdir() if p.is_file() and p.name not in ("manifest.json", "INVENTORY.txt")]
    paths.sort(key=lambda p: (priority.index(p.name) if p.name in priority else len(priority), p.name))
    fixtures = [describe(p) for p in paths]
    manifest = {
        "schema": "rustibites.compat.fixture-pack.v1",
        "created_at": datetime.now(timezone.utc).isoformat(),
        "game_version": "0.6.3.1",
        "producer": "RUSTibites bibites-io writer + CLI",
        "fixtures": fixtures,
        "commands": {
            "game_verify": "python tools/roundtrip/load_compat_pack.py --dir " + folder.as_posix(),
            "regenerate": "pwsh -File tools/roundtrip/gen_compat_pack.ps1",
        },
        "notes": [
            "All files are Rust-written (save/zip/template) except the inputs they derive from.",
            "Sidecar .bb8state is a sibling file and must not be inside the game zip.",
            "Compare game load counts to expected; a living/corpse split is expected for corpse fixtures.",
            "Any Player.log Exception/Error line or load popup is a rejection; report fixture name + log tail.",
        ],
    }
    (folder / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"manifest: {folder / 'manifest.json'} ({len(fixtures)} fixtures)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Generate a deterministic dense-cluster benchmark world for RUSTibites.

The benchmark harness needs a world where N identical bibites are packed into a
small box, so the physics broadphase/narrowphase cliff is measurable end-to-end
(the core `body::perf_probe_clustered_and_spread` audit only measures
`BibiteStore::step_physics`; this world is a full save that `rustibites bench`
can load).

Pipeline (all inputs are tracked files under tools/bench/scenarios/cluster/):

  1. build a minimal scenario bundle (one zone, no plant/meat seeding, one
     spawn row, one bundled starter genome);
  2. run it for one tick with a fixed seed and save it -> a deterministic
     1-bibite save world;
  3. clone that bibite N times on a deterministic jittered lattice inside a
     `--box` x `--box` square centred on the origin, writing a normal save zip
     that `rustibites bench --scenario <zip>` loads as a `save ...` world.

World generation is deterministic: the zip member contents depend only on the
tracked scenario files, the seed, the box size and the engine binary. Zip
timestamps are pinned so the archive bytes are reproducible too. Every world
gets a `content_sha256` (sha256 over sorted member names + bytes) recorded in
its metadata JSON, which is what benchmark artifacts should reference.

Requires Python 3.8+ (standard library only) to generate worlds; the benchmark
run itself does not need Python.

Examples:
    python tools/bench/make_cluster_world.py --n 2000 --box 100
    python tools/bench/make_cluster_world.py --n 2000 --box 100 --json
    python tools/bench/make_cluster_world.py --n 4000 --box 200 --force
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import zipfile

ZIP_DATE_TIME = (2026, 1, 1, 0, 0, 0)  # pinned for byte-reproducible archives
SCENARIO_FILES = ("scenario.info", "settings.bb8settings", "cluster.bb8template")


def repo_root() -> str:
    return os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def default_exe(repo: str) -> str:
    name = "rustibites.exe" if os.name == "nt" else "rustibites"
    return os.path.join(repo, "target", "release", name)


def sha256_file(path: str) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_zip(path: str) -> dict[str, bytes]:
    with zipfile.ZipFile(path) as archive:
        return {name: archive.read(name) for name in archive.namelist()}


def write_zip(path: str, entries: dict[str, bytes]) -> None:
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    tmp = path + ".tmp"
    with zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED) as archive:
        for name in sorted(entries):
            info = zipfile.ZipInfo(name, date_time=ZIP_DATE_TIME)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            archive.writestr(info, entries[name])
    os.replace(tmp, path)


def content_sha256(path: str) -> str:
    digest = hashlib.sha256()
    with zipfile.ZipFile(path) as archive:
        for name in sorted(archive.namelist()):
            digest.update(name.encode("utf-8"))
            digest.update(b"\0")
            digest.update(archive.read(name))
            digest.update(b"\0")
    return digest.hexdigest()


def deterministic_jitter(index: int, step: float) -> tuple[float, float, float]:
    """Small deterministic sub-cell jitter/rotation so the pile is not a grid."""
    state = (index + 1) * 0x9E3779B97F4A7C15 & 0xFFFFFFFFFFFFFFFF
    values = []
    for _ in range(3):
        state ^= state >> 30
        state = (state * 0xBF58476D1CE4E5B9) & 0xFFFFFFFFFFFFFFFF
        state ^= state >> 27
        state = (state * 0x94D049BB133111EB) & 0xFFFFFFFFFFFFFFFF
        state ^= state >> 31
        values.append((state & 0xFFFFFF) / float(0x1000000))
    jx = (values[0] - 0.5) * step * 0.8
    jy = (values[1] - 0.5) * step * 0.8
    rot = values[2] * 6.283185307179586
    return jx, jy, rot


def build_scenario(repo: str, scenario_dir: str, out_dir: str) -> tuple[str, str]:
    missing = [name for name in SCENARIO_FILES if not os.path.exists(os.path.join(scenario_dir, name))]
    if missing:
        raise SystemExit(f"cluster scenario files missing under {scenario_dir}: {missing}")
    entries = {}
    for name in SCENARIO_FILES:
        with open(os.path.join(scenario_dir, name), "rb") as handle:
            entries[name] = handle.read()
    scenario_zip = os.path.join(out_dir, "scenario.zip")
    write_zip(scenario_zip, entries)
    return scenario_zip, content_sha256(scenario_zip)


def build_base_save(repo: str, exe: str, scenario_zip: str, work_dir: str, seed: int, quiet: bool) -> tuple[str, str]:
    empty_templates = os.path.join(work_dir, "empty-templates")
    os.makedirs(empty_templates, exist_ok=True)
    base = os.path.join(work_dir, "base1.zip")
    if os.path.exists(base):
        os.remove(base)
    command = [
        exe,
        "run",
        scenario_zip,
        "--ticks",
        "1",
        "--seed",
        str(seed),
        "--templates",
        empty_templates,
        "--save",
        base,
        "--quiet",
    ]
    if not quiet:
        print("generating base save: " + " ".join(command), file=sys.stderr)
    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode != 0 or not os.path.exists(base):
        raise SystemExit(
            "base world generation failed "
            f"(exit {result.returncode}): {result.stderr.strip()} {result.stdout.strip()}"
        )
    return base, content_sha256(base)


def clone_cluster(base: str, out_path: str, count: int, box: float) -> dict:
    entries = read_zip(base)
    bibite_names = sorted(name for name in entries if name.startswith("bibites/") and name.endswith(".bb8"))
    if len(bibite_names) != 1:
        raise SystemExit(f"expected exactly one bibite in base save, found {len(bibite_names)}")
    template = json.loads(entries[bibite_names[0]])

    side = 1
    while side * side < count:
        side += 1
    step = box / side
    out_entries = {}
    for name, data in entries.items():
        if name.startswith("bibites/") or name.startswith("eggs/"):
            continue
        out_entries[name] = data
    for index in range(count):
        row, col = divmod(index, side)
        jx, jy, rot = deterministic_jitter(index, step)
        x = (col + 0.5) * step - box / 2 + jx
        y = (row + 0.5) * step - box / 2 + jy
        body = json.loads(json.dumps(template))
        body["transform"]["position"] = [x, y]
        body["transform"]["rotation"] = rot
        body["rb2d"]["px"] = x
        body["rb2d"]["py"] = y
        body["rb2d"]["vx"] = 0.0
        body["rb2d"]["vy"] = 0.0
        body["rb2d"]["r"] = rot
        out_entries[f"bibites/bibite_{index}.bb8"] = json.dumps(body, separators=(",", ":")).encode("utf-8")
    write_zip(out_path, out_entries)
    return {"count": count, "box": box, "lattice_side": side, "step": step}


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="Generate a deterministic dense-cluster RUSTibites world.")
    parser.add_argument("--n", type=int, default=2000, help="number of bibites (default 2000)")
    parser.add_argument("--box", type=float, default=100.0, help="box side length in world units (default 100)")
    parser.add_argument("--seed", type=int, default=12345, help="world seed (default 12345)")
    parser.add_argument("--exe", default="", help="rustibites CLI binary (default target/release/rustibites)")
    parser.add_argument("--scenario-dir", default="", help="cluster scenario input dir (default tools/bench/scenarios/cluster)")
    parser.add_argument("--world-dir", default="", help="output/cache dir (default target/bench-worlds/cluster)")
    parser.add_argument("--force", action="store_true", help="regenerate even if a matching cached world exists")
    parser.add_argument("--json", action="store_true", help="print a JSON result object on stdout")
    parser.add_argument("--quiet", action="store_true", help="suppress progress output on stderr")
    args = parser.parse_args(argv)

    if args.n < 1:
        raise SystemExit("--n must be >= 1")
    if args.box <= 0:
        raise SystemExit("--box must be > 0")

    repo = repo_root()
    exe = args.exe or default_exe(repo)
    if not os.path.exists(exe):
        raise SystemExit(f"rustibites binary not found: {exe} (build it first, or pass --exe)")
    scenario_dir = args.scenario_dir or os.path.join(repo, "tools", "bench", "scenarios", "cluster")
    world_dir = args.world_dir or os.path.join(repo, "target", "bench-worlds", "cluster")
    os.makedirs(world_dir, exist_ok=True)

    engine_sha = sha256_file(exe)
    box_label = ("%g" % args.box).replace(".", "p")
    world_name = f"cluster_n{args.n}_box{box_label}_seed{args.seed}.zip"
    world_path = os.path.join(world_dir, world_name)
    meta_path = os.path.join(world_dir, "meta", world_name + ".json")

    scenario_zip, scenario_sha = build_scenario(repo, scenario_dir, world_dir)

    cached = None
    if os.path.exists(world_path) and os.path.exists(meta_path) and not args.force:
        try:
            with open(meta_path, "r", encoding="utf-8") as handle:
                cached = json.load(handle)
        except (OSError, ValueError):
            cached = None
        if cached and (
            cached.get("engine_sha256") != engine_sha
            or cached.get("scenario_sha256") != scenario_sha
            or cached.get("seed") != args.seed
            or cached.get("count") != args.n
            or cached.get("box") != args.box
            or cached.get("content_sha256") != content_sha256(world_path)
        ):
            cached = None

    if cached is not None:
        result = dict(cached)
        result["reused"] = True
        result["world_path"] = world_path
    else:
        base, base_sha = build_base_save(repo, exe, scenario_zip, world_dir, args.seed, args.quiet or args.json)
        lattice = clone_cluster(base, world_path, args.n, args.box)
        world_sha = content_sha256(world_path)
        result = {
            "schema": "rustibites.bench.cluster-world/1",
            "world_path": world_path,
            "content_sha256": world_sha,
            "base_content_sha256": base_sha,
            "scenario_sha256": scenario_sha,
            "engine_sha256": engine_sha,
            "engine_path": exe,
            "seed": args.seed,
            "count": args.n,
            "box": args.box,
            "lattice_side": lattice["lattice_side"],
            "step": lattice["step"],
            "bytes": os.path.getsize(world_path),
            "reused": False,
        }
        os.makedirs(os.path.dirname(meta_path), exist_ok=True)
        with open(meta_path, "w", encoding="utf-8") as handle:
            json.dump(result, handle, indent=2)
            handle.write("\n")

    if args.json:
        print(json.dumps(result))
    else:
        print(
            "world: {world_path} ({bytes} bytes)\n"
            "  bibites={count} box={box} seed={seed}\n"
            "  content_sha256={content_sha256}\n"
            "  reused={reused}".format(**result)
        )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

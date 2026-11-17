#!/usr/bin/env python3
"""RUSTibites compatibility-matrix scanner (read-only).

Enumerates real The Bibites artifacts and records, per artifact:

  * type / game version / size / SHA-256 / duplicate count
  * load result (`rustibites check`, which parses bundle/genome/settings)
  * round-trip result:
      - .bb8 / .bb8template / .bb8settings: `check` is the structural /
        serialize->parse round-trip.
      - .zip save/scenario: `run --ticks 0 --seed 1 --stats json --save OUT`
        then reload OUT and compare the modeled `state_fingerprint`
        (load -> write -> reload semantic equality).

Everything is READ-ONLY on the input side. The only writes are temp files and
reports under `target/compat/` (never next to an artifact, never in AppData or
the game install), and the game is never launched.

Usage:
    python tools/compat/scan.py                    # full scan (all roots)
    python tools/compat/scan.py --limit 50          # smoke: metadata all, test 50
    python tools/compat/scan.py --no-build --jobs 8
    python tools/compat/scan.py --root testdata/real --root "C:/.../The Bibites"

Reports: target/compat/report.json, report.csv, unexpected.txt, summary on stdout.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import csv
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
import zipfile
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from pathlib import Path

EXTENSIONS = {".zip", ".bb8", ".bb8template", ".bb8settings"}
REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_OUT = REPO_ROOT / "target" / "compat"
CREATE_NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)

# Known-unsupported categories (docs/DEVIATIONS.md D13/D14).
D13_HINT = "0.6.3.3+/0.6.4a1 folder-save layout (D13)"
D14_HINT = "pre-0.6a1 legacy genome migration floor (D14)"


@dataclass
class Artifact:
    path: str
    root: str
    size: int
    sha256: str
    ext: str
    kind: str = "unknown"
    version: str = "unknown"
    bucket: str = "unknown"
    gene_count: int | None = None
    has_data_bin: bool = False
    duplicate_count: int = 1
    duplicate_paths: list[str] = field(default_factory=list)
    info_ok: bool | None = None
    info_error: str = ""
    load_ok: bool | None = None
    load_error: str = ""
    roundtrip: str = "not-run"
    roundtrip_detail: str = ""
    semantic_fingerprint: str = ""
    failure_class: str = ""

    def public(self) -> dict:
        data = asdict(self)
        # Keep the matrix readable; full duplicate paths stay in JSON only.
        if len(data["duplicate_paths"]) > 5:
            data["duplicate_paths"] = data["duplicate_paths"][:5] + [
                f"... {len(self.duplicate_paths) - 5} more"
            ]
        return data


# ---------------------------------------------------------------- discovery


def game_install_candidates() -> list[Path]:
    env = os.environ
    bases = [
        env.get("ProgramFiles(x86)", ""),
        env.get("ProgramFiles", ""),
        env.get("LOCALAPPDATA", ""),
    ]
    candidates: list[Path] = []
    for base in bases:
        if not base:
            continue
        base_path = Path(base)
        if "LocalAppData" in base:
            candidates.append(base_path / "Programs" / "The Bibites")
        else:
            candidates.append(base_path / "Steam" / "steamapps" / "common" / "The Bibites")
    return candidates


def default_roots() -> list[Path]:
    roots: list[Path] = [REPO_ROOT / "testdata" / "real"]
    user = os.environ.get("USERPROFILE", "")
    if user:
        roots.append(Path(user) / "AppData" / "LocalLow" / "The Bibites")
    roots.extend(game_install_candidates())
    return roots


def enumerate_files(roots: list[Path]) -> tuple[list[Path], list[dict]]:
    files: list[Path] = []
    root_info: list[dict] = []
    for root in roots:
        if not root.exists():
            root_info.append({"root": str(root), "exists": False, "files": 0})
            continue
        count = 0
        for dirpath, dirnames, filenames in os.walk(root, followlinks=False):
            dirnames[:] = sorted(d for d in dirnames if not d.startswith("."))
            for name in sorted(filenames):
                path = Path(dirpath) / name
                if path.suffix.lower() not in EXTENSIONS:
                    continue
                try:
                    if not path.is_file():
                        continue
                except OSError:
                    continue
                files.append(path)
                count += 1
        root_info.append(
            {"root": str(root), "exists": True, "files": count, "kind": "source"}
        )
    # De-duplicate identical paths (e.g. explicit --root overlapping defaults).
    seen: set[str] = set()
    unique_paths: list[Path] = []
    for path in files:
        key = str(path.resolve()).lower()
        if key in seen:
            continue
        seen.add(key)
        unique_paths.append(path)
    return unique_paths, root_info


# ---------------------------------------------------------------- metadata


def read_json_bytes(raw: bytes) -> dict | None:
    try:
        text = raw.decode("utf-8-sig")
    except UnicodeDecodeError:
        text = raw.decode("utf-8", errors="replace")
    try:
        value = json.loads(text)
    except json.JSONDecodeError:
        return None
    return value if isinstance(value, dict) else None


def safe_zip_read(zf: zipfile.ZipFile, names: list[str], wanted: str) -> bytes | None:
    lower_to_name = {name.lower(): name for name in names}
    actual = lower_to_name.get(wanted.lower())
    if actual is None:
        return None
    try:
        return zf.read(actual)
    except (KeyError, OSError, zipfile.BadZipFile):
        return None


def zip_metadata(path: Path, art: Artifact) -> None:
    try:
        with zipfile.ZipFile(path) as zf:
            names = zf.namelist()
            lowered = [name.lower() for name in names]
            has_scene = any(n.endswith("scene.bb8scene") for n in lowered)
            has_scenario = any(n.endswith("scenario.info") for n in lowered)
            has_data_bin = any(n.endswith("data.bin") for n in lowered)
            art.has_data_bin = has_data_bin
            if has_scene:
                art.kind = "save"
                raw = safe_zip_read(zf, names, "scene.bb8scene")
                scene = read_json_bytes(raw) if raw else None
                art.version = str(scene.get("version", "unknown")) if scene else "unknown"
            elif has_scenario:
                art.kind = "scenario"
                raw = safe_zip_read(zf, names, "scenario.info")
                info = read_json_bytes(raw) if raw else None
                art.version = str(info.get("version", "unknown")) if info else "unknown"
            elif any(n.endswith("sim.info") for n in lowered) or any(
                n.startswith("saves/") for n in lowered
            ):
                # D13 folder-save layout: `sim.info` + `saves/_00.zip`.
                art.kind = "folder-save"
                raw = safe_zip_read(zf, names, "sim.info")
                info = read_json_bytes(raw) if raw else None
                art.version = (
                    str(info.get("version", "0.6.4+")) if info else "0.6.4+"
                )
            else:
                art.kind = "bundle-unknown"
                art.version = "unknown"
    except (zipfile.BadZipFile, OSError):
        art.kind = "zip-corrupt"
        art.version = "unknown"


def genome_metadata(path: Path, art: Artifact) -> None:
    art.kind = "template" if art.ext == ".bb8template" else "genome"
    raw = path.read_bytes()
    data = read_json_bytes(raw)
    if data is None:
        art.version = "unknown"
        return
    # A living bibite/egg extracted from a save folder is not a standalone
    # genome artifact; it has the save-entity shape (transform/rb2d/body/clock)
    # and no top-level version. Egg files have a slightly different shape but are
    # still internal save-folder entries.
    parts = [part.lower() for part in path.parts]
    in_save_folder = "saves" in parts or "savefiles" in parts
    if "version" not in data and (
        {"transform", "rb2d", "body", "clock"} <= set(data)
        or (in_save_folder and ("eggs" in parts or "bibites" in parts))
    ):
        art.kind = "save-entity"
        art.version = "save-internal"
        return
    art.version = str(data.get("version", "unknown"))
    genes = data.get("genes")
    if isinstance(genes, list):
        art.gene_count = len(genes)
    # Modern `.bb8` files store a compact "genes" object (schema fields, not one
    # key per gene), so only a list gives a meaningful gene count.


def settings_metadata(_path: Path, art: Artifact) -> None:
    art.kind = "settings"
    art.version = "n/a"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def inspect_artifact(path: Path, root: Path) -> Artifact:
    art = Artifact(
        path=str(path),
        root=str(root),
        size=path.stat().st_size,
        sha256="",
        ext=path.suffix.lower(),
    )
    try:
        art.sha256 = sha256_file(path)
    except OSError:
        art.sha256 = ""
    try:
        if art.ext == ".zip":
            zip_metadata(path, art)
        elif art.ext == ".bb8":
            genome_metadata(path, art)
        elif art.ext == ".bb8template":
            genome_metadata(path, art)
        elif art.ext == ".bb8settings":
            settings_metadata(path, art)
    except OSError:
        art.kind = "unreadable"
    art.bucket = version_bucket(art.version, art.gene_count)
    return art


def version_bucket(version: str, gene_count: int | None = None) -> str:
    if version in ("n/a", "save-internal"):
        return "not-applicable"
    # 28-gene templates are pre-0.6a1 regardless of a missing/wrong version
    # string; modern .bb8 files have no per-gene list count.
    if gene_count is not None and gene_count < 35:
        return "pre-0.6a1 legacy"
    match = re.match(r"(\d+)\.(\d+)(?:\.(\d+))?", version or "")
    if not match:
        return "unknown"
    major = int(match.group(1))
    minor = int(match.group(2))
    patch = int(match.group(3)) if match.group(3) is not None else 0
    if major == 0 and minor == 5:
        return "0.5.x"
    if major == 0 and minor == 6:
        if "0.6.4" in version:
            return "0.6.4+"
        if patch >= 3:
            return "0.6.3.x"
        return "0.6.0-0.6.2"
    if major == 0 and minor < 6:
        return "pre-0.6a1 (game data)"
    return "other"


# ---------------------------------------------------------------- CLI runs


def run_cli(exe: Path, args: list[str], timeout: int) -> tuple[int | None, str, str]:
    try:
        completed = subprocess.run(
            [str(exe), *args],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=timeout,
            creationflags=CREATE_NO_WINDOW,
        )
        return completed.returncode, completed.stdout, completed.stderr
    except subprocess.TimeoutExpired:
        return None, "", f"timeout after {timeout}s"
    except OSError as error:
        return None, "", f"could not run CLI: {error}"


def last_json_object(stdout: str) -> dict | None:
    for line in reversed(stdout.splitlines()):
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            return value
    return None


def error_text(stdout: str, stderr: str) -> str:
    text = "\n".join(part.strip() for part in (stderr, stdout) if part.strip())
    return text.replace("\r", " ").replace("\n", " | ")


def classify_failure(art: Artifact, message: str) -> str:
    lower = message.lower()
    if "older than the supported" in lower or "migration floor" in lower:
        return f"known-unsupported: {D14_HINT}"
    if "folder-save" in lower or "sim.info" in lower or "0.6.4" in lower:
        return f"known-unsupported: {D13_HINT}"
    if art.kind == "folder-save":
        return f"known-unsupported: {D13_HINT}"
    if art.bucket in ("pre-0.6a1 legacy", "0.5.x", "pre-0.6a1 (game data)"):
        return f"known-unsupported: {D14_HINT}"
    if art.bucket == "0.6.4+":
        return f"known-unsupported: {D13_HINT}"
    if art.ext == ".zip" and art.kind == "zip-corrupt":
        return "known-unsupported: corrupt zip"
    if "not a scenario" in lower and "not a save" in lower:
        return f"known-unsupported: not a loadable scenario/save (maybe folder-save, {D13_HINT})"
    return "unexpected"


def test_artifact(art: Artifact, exe: Path, out_dir: Path, timeout: int,
                  max_roundtrip_bytes: int, keep_temp: bool) -> Artifact:
    if art.kind == "save-entity":
        # Internal per-bibite file from a folder-save unpack; not a standalone
        # artifact users open, so neither load nor round-trip applies.
        art.load_ok = None
        art.roundtrip = "not-applicable"
        art.roundtrip_detail = "internal save-folder bibite file (no standalone loader)"
        return art
    # Load / parse via `check` (genome structural round-trip, settings
    # serialize/parse, bundle parse).
    code, stdout, stderr = run_cli(exe, ["check", art.path], timeout)
    art.load_ok = code == 0
    if art.load_ok:
        art.load_error = ""
    else:
        art.load_error = error_text(stdout, stderr) or f"check exit {code}"
        art.failure_class = classify_failure(art, art.load_error)

    info_code, info_stdout, info_stderr = run_cli(exe, ["info", art.path], timeout)
    art.info_ok = info_code == 0
    if not art.info_ok:
        art.info_error = error_text(info_stdout, info_stderr) or f"info exit {info_code}"

    if not art.load_ok:
        art.roundtrip = "blocked-load"
        return art

    if art.ext != ".zip":
        # `check` already is the round-trip for genomes/settings.
        art.roundtrip = "ok"
        art.roundtrip_detail = "check (structural/serialize round-trip)"
        return art

    if art.size > max_roundtrip_bytes:
        art.roundtrip = "skipped-large"
        art.roundtrip_detail = (
            f"{art.size} bytes > --max-roundtrip-bytes {max_roundtrip_bytes}; "
            "load/check only"
        )
        return art

    stem = art.sha256[:16] if art.sha256 else f"a{abs(hash(art.path)) & 0xFFFFFFFF:08x}"
    first = out_dir / "roundtrip" / f"{stem}-1.zip"
    second = out_dir / "roundtrip" / f"{stem}-2.zip"
    first.parent.mkdir(parents=True, exist_ok=True)
    for path in (first, second):
        try:
            path.unlink()
        except FileNotFoundError:
            pass

    run_code, run_stdout, run_stderr = run_cli(
        exe,
        [
            "run",
            art.path,
            "--ticks",
            "0",
            "--seed",
            "1",
            "--stats",
            "json",
            "--save",
            str(first),
            "--quiet",
        ],
        timeout,
    )
    report1 = last_json_object(run_stdout)
    if run_code != 0 or not first.exists():
        art.roundtrip = "fail-run"
        art.roundtrip_detail = error_text(run_stdout, run_stderr) or f"run exit {run_code}"
        art.failure_class = art.failure_class or classify_failure(art, art.roundtrip_detail)
        return art

    reload_code, reload_stdout, reload_stderr = run_cli(exe, ["check", str(first)], timeout)
    run2_code, run2_stdout, run2_stderr = run_cli(
        exe,
        [
            "run",
            str(first),
            "--ticks",
            "0",
            "--seed",
            "1",
            "--stats",
            "json",
            "--save",
            str(second),
            "--quiet",
        ],
        timeout,
    )
    report2 = last_json_object(run2_stdout)
    fp1 = str(report1.get("state_fingerprint", "")) if report1 else ""
    fp2 = str(report2.get("state_fingerprint", "")) if report2 else ""
    art.semantic_fingerprint = f"{fp1}->{fp2}"
    if reload_code != 0:
        art.roundtrip = "fail-reload"
        art.roundtrip_detail = error_text(reload_stdout, reload_stderr) or f"reload exit {reload_code}"
        art.failure_class = art.failure_class or classify_failure(art, art.roundtrip_detail)
    elif run2_code != 0:
        art.roundtrip = "fail-rerun"
        art.roundtrip_detail = error_text(run2_stdout, run2_stderr) or f"rerun exit {run2_code}"
        art.failure_class = art.failure_class or classify_failure(art, art.roundtrip_detail)
    elif not fp1 or not fp2:
        art.roundtrip = "fail-fingerprint-missing"
        art.roundtrip_detail = "state_fingerprint missing from --stats json output"
        art.failure_class = art.failure_class or "unexpected"
    elif fp1 != fp2:
        art.roundtrip = "fail-semantic"
        art.roundtrip_detail = f"state_fingerprint changed across save/reload: {fp1} -> {fp2}"
        art.failure_class = art.failure_class or classify_failure(art, art.roundtrip_detail)
    else:
        art.roundtrip = "ok"
        art.roundtrip_detail = (
            f"run->save->reload semantic equality (state_fingerprint {fp1}); "
            "zip bytes are intentionally not byte-stable"
        )
    if not keep_temp:
        for path in (first, second):
            try:
                path.unlink()
            except FileNotFoundError:
                pass
    return art


# ---------------------------------------------------------------- reporting


def write_reports(artifacts: list[Artifact], roots_info: list[dict], out_dir: Path,
                  exe: Path, cli_args: dict) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    grouped: dict[str, int] = {}
    for art in artifacts:
        grouped[f"{art.ext}|{art.kind}"] = grouped.get(f"{art.ext}|{art.kind}", 0) + 1

    def counts(predicate) -> int:
        return sum(1 for art in artifacts if predicate(art))

    totals = {
        "files_enumerated": len(artifacts),
        "unique_by_sha256": len({art.sha256 for art in artifacts if art.sha256}),
        "duplicate_files": counts(lambda art: art.duplicate_count > 1),
        "by_type": grouped,
        "load_ok": counts(lambda art: art.load_ok is True),
        "load_fail": counts(lambda art: art.load_ok is False),
        "load_fail_known": counts(
            lambda art: art.load_ok is False and art.failure_class.startswith("known-unsupported")
        ),
        "load_fail_unexpected": counts(
            lambda art: art.load_ok is False and art.failure_class == "unexpected"
        ),
        "load_not_applicable": counts(lambda art: art.load_ok is None),
        "roundtrip_ok": counts(lambda art: art.roundtrip == "ok"),
        "roundtrip_fail": counts(lambda art: art.roundtrip.startswith("fail")),
        "roundtrip_skipped_large": counts(lambda art: art.roundtrip == "skipped-large"),
        "roundtrip_not_applicable": counts(
            lambda art: art.roundtrip == "not-applicable"
        ),
        "roundtrip_not_run_or_blocked": counts(
            lambda art: art.roundtrip in ("not-run", "blocked-load")
        ),
        "bytes_total": sum(art.size for art in artifacts),
    }
    by_bucket: dict[str, dict[str, int]] = {}
    for art in artifacts:
        entry = by_bucket.setdefault(
            art.bucket, {"files": 0, "load_ok": 0, "load_fail": 0, "roundtrip_ok": 0}
        )
        entry["files"] += 1
        if art.load_ok is True:
            entry["load_ok"] += 1
        elif art.load_ok is False:
            entry["load_fail"] += 1
        if art.roundtrip == "ok":
            entry["roundtrip_ok"] += 1

    payload = {
        "schema": "rustibites.compat.v1",
        "generated_utc": datetime.now(timezone.utc).isoformat(),
        "repo_root": str(REPO_ROOT),
        "cli": {"exe": str(exe), "args": cli_args},
        "roots": roots_info,
        "totals": totals,
        "by_version_bucket": by_bucket,
        "artifacts": [art.public() for art in artifacts],
    }
    (out_dir / "report.json").write_text(
        json.dumps(payload, indent=2, ensure_ascii=False), encoding="utf-8"
    )

    csv_path = out_dir / "report.csv"
    fields = [
        "path", "root", "ext", "kind", "version", "bucket", "size", "sha256",
        "duplicate_count", "gene_count", "has_data_bin", "info_ok", "load_ok",
        "roundtrip", "failure_class", "load_error", "roundtrip_detail",
        "semantic_fingerprint",
    ]
    with open(csv_path, "w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields, extrasaction="ignore")
        writer.writeheader()
        for art in artifacts:
            row = asdict(art)
            row["duplicate_paths"] = ""
            writer.writerow(row)

    unexpected = [
        art
        for art in artifacts
        if art.failure_class == "unexpected"
        or (art.roundtrip.startswith("fail") and not art.failure_class.startswith("known-unsupported"))
    ]
    lines = [
        f"unexpected failures: {len(unexpected)}",
        "path\tkind\tversion\troundtrip\tmessage",
    ]
    for art in unexpected:
        message = art.load_error or art.roundtrip_detail
        lines.append(
            "\t".join(
                [art.path, art.kind, art.version, art.roundtrip, message.replace("\t", " ")]
            )
        )
    (out_dir / "unexpected.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")


def print_summary(artifacts: list[Artifact], roots_info: list[dict], out_dir: Path) -> None:
    def count(predicate) -> int:
        return sum(1 for art in artifacts if predicate(art))

    print()
    print("=" * 78)
    print("RUSTibites compatibility scan")
    print("=" * 78)
    for info in roots_info:
        state = "ok" if info.get("exists") else "absent"
        print(f"  root [{state:6}] {info['root']}  ({info.get('files', 0)} artifacts)")
    print()
    print(f"  enumerated files : {len(artifacts)}")
    print(f"  unique sha256    : {len({a.sha256 for a in artifacts if a.sha256})}")
    print(f"  load ok/fail     : {count(lambda a: a.load_ok is True)} / "
          f"{count(lambda a: a.load_ok is False)}")
    print(f"  load not-applic. : {count(lambda a: a.load_ok is None)}")
    known = count(lambda a: a.failure_class.startswith("known-unsupported"))
    unexpected = count(lambda a: a.failure_class == "unexpected")
    print(f"    known unsupported / unexpected: {known} / {unexpected}")
    print(f"  round-trip ok    : {count(lambda a: a.roundtrip == 'ok')} "
          f"(fail {count(lambda a: a.roundtrip.startswith('fail'))}, "
          f"skipped-large {count(lambda a: a.roundtrip == 'skipped-large')}, "
          f"n/a {count(lambda a: a.roundtrip == 'not-applicable')})")
    print()
    print("  by type:")
    types: dict[str, int] = {}
    for art in artifacts:
        key = f"{art.ext} ({art.kind})"
        types[key] = types.get(key, 0) + 1
    for key in sorted(types):
        print(f"    {key:34} {types[key]:5}")
    print()
    print("  by version bucket:")
    buckets: dict[str, list[int]] = {}
    for art in artifacts:
        row = buckets.setdefault(art.bucket, [0, 0, 0, 0])
        row[0] += 1
        if art.load_ok is True:
            row[1] += 1
        elif art.load_ok is False:
            row[2] += 1
        if art.roundtrip == "ok":
            row[3] += 1
    for bucket in sorted(buckets):
        files, ok, fail, rt = buckets[bucket]
        print(f"    {bucket:24} files={files:5} load_ok={ok:5} load_fail={fail:4} rt_ok={rt:5}")
    if unexpected:
        print()
        print(f"  UNEXPECTED failures ({unexpected}) - see unexpected.txt:")
        shown = 0
        for art in artifacts:
            if art.failure_class != "unexpected":
                continue
            message = (art.load_error or art.roundtrip_detail).replace("\n", " ")[:150]
            print(f"    {art.path}\n      {message}")
            shown += 1
            if shown >= 20:
                break
    print()
    print(f"  reports: {out_dir / 'report.json'}")
    print(f"           {out_dir / 'report.csv'}")
    print(f"           {out_dir / 'unexpected.txt'}")
    print("=" * 78)


# ---------------------------------------------------------------- main


def ensure_exe(exe: Path, build: bool) -> Path:
    if exe.exists() and not build:
        return exe
    if not build:
        raise SystemExit(f"CLI not found: {exe}\nRun `cargo build --release --bin rustibites` or omit --no-build.")
    print(f"building {exe} ...")
    completed = subprocess.run(
        ["cargo", "build", "--release", "--bin", "rustibites"],
        cwd=REPO_ROOT,
        creationflags=CREATE_NO_WINDOW,
    )
    if completed.returncode != 0 or not exe.exists():
        raise SystemExit("cargo build failed; cannot scan the CLI's round-trip results")
    return exe


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", action="append", default=[],
                        help="extra source root (repeatable); defaults include testdata/real and user AppData")
    parser.add_argument("--exe", default=str(REPO_ROOT / "target" / "release" / "rustibites.exe"))
    parser.add_argument("--build", action="store_true", help="rebuild the CLI first")
    parser.add_argument("--no-build", action="store_true", help="require an existing CLI binary")
    parser.add_argument("--out", default=str(DEFAULT_OUT))
    parser.add_argument("--jobs", type=int, default=8, help="parallel CLI jobs (default 8)")
    parser.add_argument("--timeout", type=int, default=120, help="per-CLI-call timeout seconds")
    parser.add_argument("--max-roundtrip-bytes", type=int, default=8 * 1024 * 1024,
                        help="skip full save/reload round-trip above this size (default 8 MiB)")
    parser.add_argument("--limit", type=int, default=0, help="test only N unique artifacts (0 = all)")
    parser.add_argument("--sample-per-group", type=int, default=0,
                        help="test up to N unique artifacts per (kind, version-bucket) group "
                             "(0 = all; testdata/real artifacts are always tested)")
    parser.add_argument("--keep-temp", action="store_true", help="keep target/compat/roundtrip files")
    args = parser.parse_args(argv)

    repo_target = (REPO_ROOT / "target").resolve()
    out_dir = Path(args.out).resolve()
    if repo_target not in out_dir.parents and out_dir != repo_target:
        raise SystemExit(f"--out must stay under {repo_target} (read-only safety); got {out_dir}")

    exe = Path(args.exe).resolve()
    build = args.build or (not args.no_build and not exe.exists())
    exe = ensure_exe(exe, build)

    # Work from a private copy of the CLI: other agents rebuild
    # target/release/rustibites.exe during a long scan, which makes concurrent
    # subprocess launches fail with WinError 2. The copy is temp-only under
    # target/compat and makes results reproducible.
    out_dir.mkdir(parents=True, exist_ok=True)
    stable_exe = out_dir / ("rustibites.exe" if os.name == "nt" else "rustibites")
    shutil.copyfile(exe, stable_exe)
    if os.name != "nt":
        stable_exe.chmod(0o755)
    exe = stable_exe

    roots = [Path(p) for p in args.root] if args.root else default_roots()
    files, roots_info = enumerate_files(roots)
    print(f"enumerated {len(files)} candidate artifacts under {len(roots)} root(s)")

    started = time.time()
    artifacts: list[Artifact] = []
    by_hash: dict[str, list[Artifact]] = {}
    for index, path in enumerate(files):
        if index % 200 == 0:
            print(f"  metadata {index}/{len(files)} ...", flush=True)
        root = next((r for r in roots if str(path).startswith(str(r))), Path(""))
        art = inspect_artifact(path, root)
        artifacts.append(art)
        if art.sha256:
            by_hash.setdefault(art.sha256, []).append(art)
    for sha, group in by_hash.items():
        if len(group) <= 1:
            continue
        representative = group[0]
        representative.duplicate_count = len(group)
        representative.duplicate_paths = [item.path for item in group]
        for item in group[1:]:
            item.duplicate_count = len(group)
            # Duplicates mirror the representative's test fields for the CSV.
            item.info_ok = None

    # Test one representative per unique SHA-256.
    representatives: list[Artifact] = list(by_hash.values())
    representatives = [group[0] for group in representatives]
    representatives.sort(key=lambda art: (art.ext, art.path))
    if args.limit > 0:
        representatives = representatives[: args.limit]
    if args.sample_per_group > 0:
        # Stratified sample: always cover the repository fixtures, then up to N
        # per (kind, version bucket). Large user libraries contain thousands of
        # autosaves of the same format; testing every 80 MB save is not useful.
        selected: list[Artifact] = []
        selected_shas: set[str] = set()
        for art in representatives:
            normalised = art.path.replace("\\", "/")
            if "/testdata/real/" in normalised:
                selected.append(art)
                selected_shas.add(art.sha256)
        counts: dict[tuple[str, str], int] = {}
        for art in representatives:
            if art.sha256 in selected_shas:
                continue
            key = (art.kind, art.bucket)
            used = counts.get(key, 0)
            if used >= args.sample_per_group:
                continue
            selected.append(art)
            selected_shas.add(art.sha256)
            counts[key] = used + 1
        representatives = selected
    print(
        f"testing {len(representatives)} unique artifacts on {args.jobs} job(s)"
        + (f" (sample {args.sample_per_group}/group)" if args.sample_per_group else "")
    )

    tested: dict[str, Artifact] = {}
    completed_count = 0
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as executor:
        futures = {
            executor.submit(
                test_artifact,
                art,
                exe,
                out_dir,
                args.timeout,
                args.max_roundtrip_bytes,
                args.keep_temp,
            ): art
            for art in representatives
        }
        for future in concurrent.futures.as_completed(futures):
            art = future.result()
            tested[art.sha256] = art
            completed_count += 1
            if completed_count % 25 == 0 or completed_count == len(representatives):
                print(f"  tested {completed_count}/{len(representatives)}", flush=True)

    # Fold unique results back onto every duplicate artifact for reporting.
    for art in artifacts:
        source = tested.get(art.sha256)
        if source is None:
            if art.duplicate_count > 1:
                art.roundtrip = "duplicate-not-tested"
            continue
        for name in (
            "info_ok", "info_error", "load_ok", "load_error", "roundtrip",
            "roundtrip_detail", "semantic_fingerprint", "failure_class",
        ):
            setattr(art, name, getattr(source, name))

    write_reports(artifacts, roots_info, out_dir, exe, vars(args))
    print_summary(artifacts, roots_info, out_dir)
    print(f"elapsed {time.time() - started:.1f}s")
    if not args.keep_temp:
        shutil.rmtree(out_dir / "roundtrip", ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""run_visual_parity.py - deterministic 60-session visual-parity matrix.

Renders every captured runtime session through the viewer's
``--ui-session`` path at 1920x1080 and compares it with parity's reference
overlays via ``tools/ui/visual_diff.py``.

Typical use (headless, no game window)::

    cargo build --release -p bibites-view
    python tools/ui/run_visual_parity.py

CI gate::

    python tools/ui/run_visual_parity.py --strict

Artifacts:
  * full renders/diffs/results under ``target/ui_visual_diff`` (gitignored)
  * compact, committed summary under ``tools/ui/results/visual_parity``
    (sessions.json, results.json, results.csv, leaderboard.md, manifest.json)

The runner is deterministic: sessions are sorted from the control manifest,
reference discovery is fixed by parity, and identical renders produce identical
hashes. Pass ``--repeat 2`` to verify that explicitly (slower).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_MANIFEST = ROOT / "tools/ui/viewer_control_ids.json"
DEFAULT_RUNTIME = ROOT / "tools/ui/ui_runtime_atlas.json.gz"
DEFAULT_PARITY = ROOT / "tools/ui/visual_diff.py"
DEFAULT_VIEWER = ROOT / "target/release/rustibites-view.exe"
DEFAULT_WORLD = ROOT / "testdata/parity/default/rust_seed_600.zip"
DEFAULT_REF = ROOT / "assets/game/reference/functional_ui"
DEFAULT_NATIVE_REF = ROOT / "assets/game/reference/functional_ui_native"
DEFAULT_WORK = ROOT / "target/ui_visual_diff"
DEFAULT_RESULTS = ROOT / "tools/ui/results/visual_parity"
SCHEMA = "rustibites.ui.visual_parity_matrix.v1"


def sessions_from_manifest(path: Path) -> list[str]:
    doc = json.loads(path.read_text(encoding="utf-8"))
    sessions = doc.get("sessions") or {}
    names = sorted(sessions.keys() if isinstance(sessions, dict) else sessions)
    if not names:
        raise SystemExit(f"no sessions in {path}")
    return names


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


# Native in-world capture names -> runtime atlas session names.
NATIVE_SESSION_ALIASES = {
    "native_world_toolbar": "main_world_toolbar",
    "native_world_placer": "main_world_tool_placer",
    "native_world_placer_pellets": "main_world_tool_placer",
    "native_world_user_actions": "main_world_tool_user_actions",
    "native_world_view": "main_world_tool_view",
    "native_world_dynamic": "main_world_dynamic_parameters",
    "native_world_information": "main_world_information",
    "native_world_lineage": "main_world_lineage",
    "native_world_settings": "main_world_settings_panel",
    "native_inspector_stats": "default_world_inspector",
    "native_inspector_genes": "default_world_genes",
    "native_inspector_biology": "default_world_biology",
    "native_inspector_brain": "default_world_brain",
}


def _load_reference_map(path: Path) -> dict[str, str]:
    doc = json.loads(path.read_text(encoding="utf-8"))
    if isinstance(doc, dict):
        sessions = doc.get("sessions", doc)
        return {str(k): str(v) for k, v in sessions.items()} if isinstance(sessions, dict) else {}
    return {}


def native_ui_on_black_map(native_dir: Path | None, allowed: set[str]) -> dict[str, str]:
    """60-session native UI-on-black map from parity's bulk runtime manifest."""
    if native_dir is None:
        return {}
    manifest = native_dir / "menu_ui" / "native_runtime_manifest.json"
    if not manifest.is_file():
        return {}
    try:
        doc = json.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}
    mapping = {}
    for session, entry in (doc.get("refs") or {}).items():
        path = entry.get("file") if isinstance(entry, dict) else None
        if not path:
            continue
        candidate = ROOT / path
        if not candidate.is_file():
            candidate = manifest.parent / Path(path).name
        if candidate.is_file() and session in allowed:
            mapping[session] = str(candidate)
    return mapping


def native_reference_map(
    native_dir: Path | None,
    explicit: Path | None,
    allowed: set[str] | None = None,
) -> dict[str, str]:
    """Map runtime session names to native captures via parity's canonical map."""
    if explicit and explicit.is_file():
        mapping = _load_reference_map(explicit)
    elif native_dir is not None:
        mapping = {}
        canonical = native_dir / "in_world" / "session_map.json"
        if canonical.is_file():
            mapping = _load_reference_map(canonical)
        else:
            for manifest in sorted(native_dir.rglob("manifest.json")):
                try:
                    doc = json.loads(manifest.read_text(encoding="utf-8"))
                except (OSError, ValueError):
                    continue
                entries = doc.get("sessions") or []
                for entry in entries:
                    if not isinstance(entry, dict):
                        continue
                    source = entry.get("session") or entry.get("name")
                    path = entry.get("file") or entry.get("path")
                    if not source or not path:
                        continue
                    runtime = NATIVE_SESSION_ALIASES.get(source, source)
                    candidate = ROOT / path
                    if not candidate.is_file():
                        candidate = native_dir / Path(path).name
                    if candidate.is_file():
                        mapping[runtime] = str(candidate)
                if mapping:
                    break
    else:
        return {}
    if allowed is not None:
        mapping = {runtime: path for runtime, path in mapping.items() if runtime in allowed}
    return mapping


def render_hashes(render_dir: Path) -> dict[str, str]:
    out: dict[str, str] = {}
    if render_dir.is_dir():
        for png in sorted(render_dir.glob("*.png")):
            out[png.stem] = sha256_file(png)
    return out


def build_command(args, sessions_file: Path, out_dir: Path, reference_map: Path | None = None) -> list[str]:
    cmd = [
        sys.executable,
        str(args.parity),
        "--reference-dir",
        str(args.reference_dir),
        "--viewer",
        str(args.viewer),
        "--sessions-file",
        str(sessions_file),
        "--out-dir",
        str(out_dir),
        "--json",
        str(out_dir / "results.json"),
        "--csv",
        str(out_dir / "results.csv"),
        "--md",
        str(out_dir / "leaderboard.md"),
        "--diffs-dir",
        str(out_dir / "diffs"),
        "--timeout",
        str(args.timeout),
        "--min-iou",
        str(args.min_iou),
        "--max-mae",
        str(args.max_mae),
        "--min-exact-pct",
        str(args.min_exact_pct),
    ]
    if args.world:
        cmd += ["--world", str(args.world)]
    if args.viewer_args:
        cmd += ["--viewer-args", args.viewer_args]
    if args.viewer_compose:
        cmd += ["--viewer-compose"]
    if args.render_background:
        cmd += ["--render-background", str(args.render_background)]
    if args.diagnostics:
        cmd += ["--diagnostics"]
    if reference_map is not None:
        cmd += ["--reference-map", str(reference_map)]
    if args.native_reference_dir:
        cmd += ["--native-reference-dir", str(args.native_reference_dir)]
    if args.require_native_references:
        cmd += ["--require-native-references"]
    if args.allow_missing and not args.strict:
        cmd += ["--allow-missing"]
    if args.dry_run:
        cmd += ["--dry-run"]
    return cmd


def copy_summary(out_dir: Path, results_dir: Path, sessions: list[str], cmd: list[str], repeat: int, render_hashes_by_run):
    results_dir.mkdir(parents=True, exist_ok=True)
    for name in ("results.json", "results.csv", "leaderboard.md"):
        source = out_dir / name
        if source.is_file():
            shutil.copyfile(source, results_dir / name)
    (results_dir / "sessions.json").write_text(
        json.dumps(sessions, indent=1) + "\n", encoding="utf-8"
    )
    manifest = {
        "schema": SCHEMA,
        "sessions": len(sessions),
        "repeat": repeat,
        "command": cmd,
        "renders_sha256": render_hashes_by_run[0] if render_hashes_by_run else {},
        "deterministic": len(render_hashes_by_run) <= 1
        or all(hashes == render_hashes_by_run[0] for hashes in render_hashes_by_run[1:]),
    }
    (results_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=1, sort_keys=True) + "\n", encoding="utf-8"
    )


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--manifest", default=str(DEFAULT_MANIFEST), help="control manifest with 60 session keys")
    ap.add_argument("--parity", default=str(DEFAULT_PARITY), help="tools/ui/visual_diff.py")
    ap.add_argument("--viewer", default=str(DEFAULT_VIEWER))
    ap.add_argument("--world", default=str(DEFAULT_WORLD))
    ap.add_argument("--reference-dir", default=str(DEFAULT_REF), help="legacy reference root")
    ap.add_argument("--native-reference-dir", default=str(DEFAULT_NATIVE_REF) if DEFAULT_NATIVE_REF.is_dir() else None,
                    help="authoritative native-1920 reference root (tried first)")
    ap.add_argument("--require-native-references", action="store_true",
                    help="fail on legacy/rescale-blur references")
    ap.add_argument("--work-dir", default=str(DEFAULT_WORK))
    ap.add_argument("--results-dir", default=str(DEFAULT_RESULTS))
    ap.add_argument("--viewer-args", default="")
    ap.add_argument("--only", action="append", default=[], help="session name filter (repeatable)")
    ap.add_argument("--limit", type=int, default=None)
    ap.add_argument("--timeout", type=float, default=120.0)
    ap.add_argument("--min-iou", type=float, default=0.98)
    ap.add_argument("--max-mae", type=float, default=3.0)
    ap.add_argument("--min-exact-pct", type=float, default=90.0)
    ap.add_argument("--native-in-world", action="store_true",
                    help="run only sessions covered by the canonical native in-world map")
    ap.add_argument("--native-ui-on-black", action="store_true",
                    help="use parity's 60 native UI-on-black refs and flatten renders over black")
    ap.add_argument("--native-reference-map", default=None,
                    help="explicit runtime-session -> native-reference JSON")
    ap.add_argument("--viewer-compose", action="store_true",
                    help="let viewer compose background for full-frame menu A/B (omit --ui-only)")
    ap.add_argument("--render-background", default=None,
                    help="flatten renders over black/white/#RRGGBB for UI-on-black menu refs")
    ap.add_argument("--diagnostics", action="store_true",
                    help="record alpha/color/edge diagnostics for font/TMP blur")
    ap.add_argument("--repeat", type=int, default=1, help="render the whole matrix N times (determinism check)")
    ap.add_argument("--allow-missing", action="store_true", default=True)
    ap.add_argument("--strict", action="store_true", help="fail on missing references or any threshold failure")
    ap.add_argument("--dry-run", action="store_true", help="write the visual_diff plan only")
    ap.add_argument("--self-test", action="store_true", help="validate manifest/reference/session plumbing without rendering")
    args = ap.parse_args()

    for path, label in (
        (Path(args.manifest), "manifest"),
        (Path(args.parity), "visual_diff.py"),
        (Path(args.viewer), "viewer binary"),
    ):
        if not path.is_file():
            raise SystemExit(f"missing {label}: {path}")
    sessions = sessions_from_manifest(Path(args.manifest))
    native_dir = Path(args.native_reference_dir) if args.native_reference_dir else None
    if args.native_ui_on_black and not args.native_reference_map:
        native_map = native_ui_on_black_map(native_dir, set(sessions))
        if args.render_background is None:
            args.render_background = "black"
    else:
        native_map = native_reference_map(
            native_dir,
            Path(args.native_reference_map) if args.native_reference_map else None,
            set(sessions),
        )
    if args.only:
        wanted = set(args.only)
        sessions = [name for name in sessions if name in wanted]
    if args.limit is not None:
        sessions = sessions[: args.limit]
    if args.native_in_world:
        sessions = [name for name in sessions if name in native_map]
    if not sessions:
        raise SystemExit("no sessions selected")

    roots = [Path(args.native_reference_dir)] if args.native_reference_dir else []
    roots.append(Path(args.reference_dir))
    missing_refs = [
        name
        for name in sessions
        if name not in native_map
        and not any(
            (root / f"{name}{suffix}").is_file()
            for root in roots
            for suffix in ("_overlay_1920x1080.png", "_full_1920x1080.png", "_1920x1080.png")
        )
    ]
    if args.self_test:
        print(f"sessions: {len(sessions)}")
        print(f"reference dir: {args.reference_dir}")
        print(f"native mappings: {len(native_map)}")
        print(f"missing references: {len(missing_refs)}")
        for name in missing_refs:
            print(f"  missing: {name}")
        return 0 if (not args.strict or not missing_refs) else 1

    work = Path(args.work_dir)
    work.mkdir(parents=True, exist_ok=True)
    sessions_file = work / "sessions.json"
    sessions_file.write_text(json.dumps(sessions, indent=1) + "\n", encoding="utf-8")
    map_file = work / "native_reference_map.json"
    if native_map:
        map_file.write_text(json.dumps(native_map, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    else:
        map_file = None

    hashes = []
    last_cmd = []
    last_run_dir = work
    exit_code = 0
    for repetition in range(max(1, args.repeat)):
        run_dir = work if args.repeat == 1 else work / f"repeat-{repetition + 1}"
        last_run_dir = run_dir
        if run_dir.exists() and args.repeat > 1:
            shutil.rmtree(run_dir)
        run_dir.mkdir(parents=True, exist_ok=True)
        cmd = build_command(args, sessions_file, run_dir, map_file)
        last_cmd = cmd
        print("+ " + " ".join(cmd))
        completed = subprocess.run(cmd, cwd=ROOT, check=False)
        hashes.append(render_hashes(run_dir / "renders"))
        if completed.returncode != 0:
            exit_code = completed.returncode
            if args.strict:
                break
        if args.repeat == 1:
            break

    copy_summary(last_run_dir, Path(args.results_dir), sessions, last_cmd, args.repeat, hashes)
    if hashes and args.repeat > 1 and any(h != hashes[0] for h in hashes[1:]):
        print("error: repeated renders are not byte-identical", file=sys.stderr)
        exit_code = 1
    print(f"wrote {Path(args.results_dir) / 'results.json'}")
    if missing_refs:
        print(f"note: {len(missing_refs)} session(s) have no reference overlay yet", file=sys.stderr)
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())

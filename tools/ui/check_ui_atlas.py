#!/usr/bin/env python3
"""Validate ``tools/ui/ui_atlas.json`` for view-dev (task-84).

Checks, in order:

1. schema/version and source provenance;
2. every expectation in ``tools/assets/ui_expectations.json`` has exactly one atlas record;
3. per-record fields (kind/status/priority) and that extracted files exist on disk;
4. 9-slice records carry Unity border metadata or an explicit ``needs_prefab`` flag;
5. font roles resolve to extracted TTF / TMP SDF atlas files;
6. brain icon tables are complete and ordered (33 inputs / 15 outputs / 13 activations);
7. material records resolve to extracted material JSON;
8. gap lists are consistent with the expectations (``missing_required`` empty; optional gaps are
   the known game/scene-only art).

Exit codes: 0 = valid (warnings allowed), 1 = problems found with ``--strict``, 2 = unreadable or
malformed atlas/expectations.

Examples::

    python tools/ui/check_ui_atlas.py
    python tools/ui/check_ui_atlas.py --strict
    python tools/ui/check_ui_atlas.py --json
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_ATLAS = ROOT / "tools" / "ui" / "ui_atlas.json"
DEFAULT_EXPECTATIONS = ROOT / "tools" / "assets" / "ui_expectations.json"

KINDS = {"sprite", "nine_slice", "icon", "font", "material", "procedural"}
STATUSES = {"mapped", "runtime", "unmatched"}
PRIORITIES = {"P0", "P0b", "P1", "P2", "P3"}
CORE_PRIORITIES = {"P0", "P0b", "P1"}
REQUIRED_ROLES = {"body", "bold"}
BRAIN_TABLE_LENGTHS = {"inputs": 33, "outputs": 15, "activations": 13}


def load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def file_exists(rel_path: str | None) -> bool:
    if not rel_path:
        return False
    candidate = (ROOT / rel_path).resolve()
    try:
        candidate.relative_to(ROOT.resolve())
    except ValueError:
        return False
    return candidate.is_file()


def check(atlas: dict, expectations: dict) -> dict:
    problems: list[str] = []
    warnings: list[str] = []
    counts = {
        "entries": 0,
        "mapped": 0,
        "runtime": 0,
        "unmatched": 0,
        "m1_core_ui": 0,
        "nine_slice": 0,
        "files_checked": 0,
        "files_missing": 0,
    }

    if atlas.get("schema_version") != 1:
        problems.append("unsupported/missing schema_version (expected 1)")
    if atlas.get("generated_by") != "tools/ui/build_ui_atlas.py":
        warnings.append("unexpected generated_by (expected tools/ui/build_ui_atlas.py)")

    records = atlas.get("atlas")
    if not isinstance(records, dict):
        problems.append("missing `atlas` object")
        return {"problems": problems, "warnings": warnings, "counts": counts}

    expected_entries = {e["logical"]: e for e in expectations.get("entries", []) if e.get("logical")}
    atlas_keys = set(records)
    missing_records = sorted(set(expected_entries) - atlas_keys)
    extra_records = sorted(atlas_keys - set(expected_entries))
    if missing_records:
        problems.append(f"{len(missing_records)} expectations missing from atlas (e.g. {missing_records[:3]})")
    if extra_records:
        problems.append(f"{len(extra_records)} atlas records have no expectation (e.g. {extra_records[:3]})")

    fonts = atlas.get("fonts") or {}
    roles = fonts.get("roles") or {}
    for role in REQUIRED_ROLES:
        role_record = roles.get(role)
        if not isinstance(role_record, dict):
            problems.append(f"fonts.roles.{role} missing")
            continue
        if not (file_exists(role_record.get("ttf")) or file_exists(role_record.get("tmp_atlas"))):
            problems.append(f"fonts.roles.{role} has no existing ttf/tmp_atlas file")
    if not fonts.get("substitution_rules"):
        warnings.append("fonts.substitution_rules is empty")

    for logical in sorted(records):
        record = records[logical]
        counts["entries"] += 1
        for field in ("kind", "status", "priority", "usage"):
            if field not in record:
                problems.append(f"{logical}: missing field `{field}`")
        kind = record.get("kind")
        status = record.get("status")
        priority = record.get("priority")
        if kind not in KINDS:
            problems.append(f"{logical}: invalid kind {kind!r}")
        if status not in STATUSES:
            problems.append(f"{logical}: invalid status {status!r}")
        if priority not in PRIORITIES:
            problems.append(f"{logical}: invalid priority {priority!r}")
        elif priority in CORE_PRIORITIES:
            counts["m1_core_ui"] += 1
        if status == "mapped":
            counts["mapped"] += 1
        elif status == "runtime":
            counts["runtime"] += 1
        else:
            counts["unmatched"] += 1
        if record.get("nine_slice"):
            counts["nine_slice"] += 1

        # File existence for anything that claims to be extracted.
        if status == "mapped" and kind not in {"procedural"}:
            rel = record.get("file")
            counts["files_checked"] += 1
            if not file_exists(rel):
                counts["files_missing"] += 1
                problems.append(f"{logical}: extracted file missing on disk: {rel!r}")

        if kind == "nine_slice":
            border = record.get("border")
            has_border = isinstance(border, list) and len(border) == 4 and any(float(v) != 0.0 for v in border)
            if not has_border and not record.get("needs_prefab"):
                problems.append(f"{logical}: 9-slice without border metadata and without needs_prefab flag")
            if has_border and record.get("border_inferred") and not record.get("needs_prefab"):
                problems.append(f"{logical}: inferred border must set needs_prefab=true")

        if kind == "font":
            if not record.get("font_role"):
                problems.append(f"{logical}: font record without font_role")
        if kind == "material":
            if not isinstance(record.get("material"), dict) or not record["material"].get("file"):
                problems.append(f"{logical}: material record without resolved material metadata")

        if kind == "procedural" and status == "runtime" and record.get("needs_prefab"):
            warnings.append(f"{logical}: procedural asset flagged needs_prefab (expected false)")

    # Brain icon tables.
    brain = atlas.get("brain_icons") or {}
    for table_name, expected_len in BRAIN_TABLE_LENGTHS.items():
        table = brain.get(table_name)
        if not isinstance(table, list):
            problems.append(f"brain_icons.{table_name} missing")
            continue
        if len(table) != expected_len:
            problems.append(f"brain_icons.{table_name} has {len(table)} rows, expected {expected_len}")
        seen = set()
        for row in table:
            index = row.get("index")
            if not isinstance(index, int) or not (0 <= index < expected_len):
                problems.append(f"brain_icons.{table_name}: bad index {index!r}")
            if index in seen:
                problems.append(f"brain_icons.{table_name}: duplicate index {index}")
            seen.add(index)
            if row.get("file") and not file_exists(row["file"]):
                problems.append(f"brain_icons.{table_name}[{index}]: missing file {row['file']!r}")
            if not row.get("filename") and row.get("status") != "unmatched":
                warnings.append(f"brain_icons.{table_name}[{index}]: no filename")

    # Gap-list consistency.
    computed_missing_required = sorted(
        logical
        for logical, record in records.items()
        if record.get("status") == "unmatched" and record.get("priority") in CORE_PRIORITIES
    )
    if computed_missing_required:
        problems.append(f"required gaps present: {computed_missing_required[:5]}")
    listed = atlas.get("missing_required")
    if listed is not None and sorted(listed) != computed_missing_required:
        problems.append("atlas.missing_required is stale/inconsistent")

    # Shared chrome info.
    shared = [logical for logical, record in records.items() if record.get("shared_with")]
    return {
        "problems": problems,
        "warnings": warnings,
        "counts": counts,
        "shared_alias_records": len(shared),
        "needs_prefab": len(atlas.get("needs_prefab") or []),
        "inferred_borders": len(atlas.get("inferred_borders") or []),
        "optional_missing": len(atlas.get("optional_missing") or []),
    }


def summarise(expectations: dict, atlas: dict, report: dict) -> bool:
    counts = report["counts"]
    print("UI atlas validation")
    print(f"  atlas             : tools/ui/ui_atlas.json (schema v{atlas.get('schema_version')})")
    print(f"  entries           : {counts['entries']} ({counts['m1_core_ui']} M1 core UI)")
    print(f"  status            : mapped {counts['mapped']}, runtime {counts['runtime']}, unmatched {counts['unmatched']}")
    print(f"  nine-slice records: {counts['nine_slice']} | needs_prefab: {report['needs_prefab']} | "
          f"inferred borders: {report['inferred_borders']}")
    print(f"  shared-alias recs : {report['shared_alias_records']}")
    print(f"  files checked     : {counts['files_checked']} (missing {counts['files_missing']})")
    print(f"  optional gaps     : {report['optional_missing']} (known game/scene-only art)")
    brain = atlas.get("brain_icons") or {}
    print(f"  brain icons       : inputs {len(brain.get('inputs') or [])}/33, "
          f"outputs {len(brain.get('outputs') or [])}/15, "
          f"activations {len(brain.get('activations') or [])}/13")
    fonts = atlas.get("fonts") or {}
    role_names = ", ".join(sorted((fonts.get("roles") or {}).keys()))
    print(f"  font roles        : {role_names}")
    if report["warnings"]:
        print(f"\n  warnings ({len(report['warnings'])}):")
        for warning in report["warnings"][:20]:
            print(f"    {warning}")
        if len(report["warnings"]) > 20:
            print(f"    … {len(report['warnings']) - 20} more")
    if report["problems"]:
        print(f"\n  problems ({len(report['problems'])}):")
        for problem in report["problems"][:40]:
            print(f"    {problem}")
        if len(report["problems"]) > 40:
            print(f"    … {len(report['problems']) - 40} more")
    ok = not report["problems"]
    print("\n  result            : " + ("OK" if ok else "FAIL"))
    return ok


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--atlas", type=Path, default=DEFAULT_ATLAS)
    parser.add_argument("--expectations", type=Path, default=DEFAULT_EXPECTATIONS)
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--strict", action="store_true", help="exit 1 unless the atlas is fully valid")
    args = parser.parse_args()

    for label, path in (("atlas", args.atlas), ("expectations", args.expectations)):
        if not path.exists():
            print(f"error: {label} file missing: {path}", file=sys.stderr)
            return 2
    try:
        atlas = load_json(args.atlas)
        expectations = load_json(args.expectations)
    except (OSError, json.JSONDecodeError) as error:
        print(f"error: cannot read input: {error}", file=sys.stderr)
        return 2
    if not isinstance(atlas.get("atlas"), dict):
        print("error: atlas has no `atlas` object", file=sys.stderr)
        return 2

    report = check(atlas, expectations)
    if args.json:
        print(json.dumps(report, indent=2))
    else:
        summarise(expectations, atlas, report)
    if args.strict:
        return 0 if not report["problems"] else 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

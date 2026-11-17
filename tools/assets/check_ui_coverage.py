#!/usr/bin/env python3
"""Check UI asset expectations against parity-dev's extracted manifest.

Loads ``tools/assets/ui_expectations.json`` (see ``build_ui_expectations.py``)
and, when it exists, ``assets/game/manifest.json``.  Reports per expectation:
extracted (exact or suffix match), renamed (same basename, different path),
missing, or runtime-procedural (no PNG needed).

If ``assets/game/manifest.json`` does not exist yet (parity-dev task-76 still in
flight), the script prints the expected-vs-actual plan and exits cleanly (0), so
it is safe to run in CI before extraction lands.

Examples::

    python tools/assets/check_ui_coverage.py                  # human summary
    python tools/assets/check_ui_coverage.py --json           # machine summary
    python tools/assets/check_ui_coverage.py --strict         # exit 1 on missing
    python tools/assets/check_ui_coverage.py --expectations tools/assets/ui_expectations.json \
        --manifest assets/game/manifest.json

Exit codes: 0 = ok (or manifest not yet present), 1 = missing items with
``--strict``, 2 = malformed expectations file.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_EXPECTATIONS = ROOT / "tools" / "assets" / "ui_expectations.json"
DEFAULT_MANIFEST = ROOT / "assets" / "game" / "manifest.json"

SPRITE_EXTENSIONS = (".png", ".jpg", ".jpeg", ".tga", ".psd", ".asset")
FONT_EXTENSIONS = (".ttf", ".otf", ".asset")
RUNTIME_EXTRACTION = {"runtime_procedural"}


def normalise(raw: str) -> str:
    """Normalise a logical name / manifest path for comparison."""
    value = raw.strip().replace("\\", "/").lower()
    while value.startswith("./"):
        value = value[2:]
    for prefix in ("assets/game/", "game/", "assets/"):
        if value.startswith(prefix):
            value = value[len(prefix) :]
    for ext in SPRITE_EXTENSIONS + FONT_EXTENSIONS:
        if value.endswith(ext):
            value = value[: -len(ext)]
    return value.rstrip("/")


def collect_manifest_candidates(node, out: set[str], depth: int = 0) -> None:
    """Collect string leaves/keys from an arbitrary manifest JSON shape."""
    if depth > 12:
        return
    if isinstance(node, dict):
        for key, value in node.items():
            if isinstance(key, str) and ("/" in key or key.lower().endswith(SPRITE_EXTENSIONS + FONT_EXTENSIONS)):
                out.add(normalise(key))
            collect_manifest_candidates(value, out, depth + 1)
    elif isinstance(node, list):
        for item in node:
            collect_manifest_candidates(item, out, depth + 1)
    elif isinstance(node, str):
        out.add(normalise(node))


def load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def check(expectations: dict, manifest: dict | None) -> dict:
    entries = expectations.get("entries")
    if not isinstance(entries, list):
        raise ValueError("expectations file has no `entries` array")

    candidates: set[str] = set()
    name_set: set[str] = set()
    if manifest is not None:
        collect_manifest_candidates(manifest, candidates)
        candidates.discard("")
        name_set = {c.rsplit("/", 1)[-1] for c in candidates if c}

    results = []
    counts = {
        "exact": 0, "suffix": 0, "alias": 0, "renamed": 0, "missing": 0,
        "missing_optional": 0, "runtime": 0,
    }
    claimed_names: set[str] = set()
    for entry in entries:
        logical = entry.get("logical")
        if not logical:
            raise ValueError("expectation without `logical`")
        aliases = [normalise(a) for a in entry.get("manifest_aliases", []) or []]
        status = "runtime" if entry.get("extraction") in RUNTIME_EXTRACTION else "missing"
        candidate = None
        suggestion = None
        matched_on = None
        if manifest is not None and status != "runtime":
            target = normalise(logical)
            basename = target.rsplit("/", 1)[-1]
            if target in candidates:
                status = "exact"
                candidate = target
                matched_on = "logical"
            else:
                suffix_hits = [c for c in candidates if c.endswith("/" + target) and c != target]
                if suffix_hits:
                    status = "suffix"
                    candidate = sorted(suffix_hits)[0]
                    matched_on = "logical-suffix"
                else:
                    alias_hits = [a for a in aliases if a in name_set or a in candidates]
                    if alias_hits:
                        status = "alias"
                        candidate = sorted(alias_hits)[0]
                        matched_on = "manifest-alias"
                    else:
                        renamed = [c for c in candidates if c.rsplit("/", 1)[-1] == basename]
                        if renamed:
                            status = "renamed"
                            suggestion = sorted(renamed)[0]
        if candidate:
            claimed_names.add(candidate.rsplit("/", 1)[-1])
        if status == "missing" and entry.get("optional"):
            status = "missing_optional"
        counts[status] += 1
        results.append(
            {
                "id": entry.get("id"),
                "logical": logical,
                "priority": entry.get("priority"),
                "usage": entry.get("usage"),
                "extraction": entry.get("extraction"),
                "prefab_only": entry.get("prefab_only"),
                "nine_slice": entry.get("nine_slice"),
                "manifest_aliases": aliases,
                "status": status,
                "matched": candidate,
                "matched_on": matched_on,
                "rename_suggestion": suggestion,
            }
        )
    # Which extracted UI asset names are not claimed by any expectation? (informational)
    unclaimed_ui: list[str] = []
    if manifest is not None:
        ui_names: set[str] = set()
        collect_manifest_candidates(manifest.get("ui", {}), ui_names)
        ui_name_set = {name.rsplit("/", 1)[-1] for name in ui_names if name}
        unclaimed_ui = [name for name in sorted(ui_name_set) if name not in claimed_names]
    return {
        "counts": counts,
        "results": results,
        "manifest_present": manifest is not None,
        "unclaimed_ui_names": unclaimed_ui,
    }


def summarise(expectations: dict, report: dict) -> bool:
    """Print a human summary; returns True when nothing is missing."""
    counts = report["counts"]
    counts_meta = expectations.get("counts", {})
    print("UI asset expectations coverage")
    print(f"  expectations file : tools/assets/ui_expectations.json "
          f"({counts_meta.get('entries', '?')} entries)")
    if not report["manifest_present"]:
        print("  manifest          : assets/game/manifest.json NOT PRESENT (parity-dev task-76 pending)")
        by_priority = counts_meta.get("by_priority", {})
        print(f"  plan              : {counts_meta.get('entries', '?')} entries "
              f"({', '.join(f'{k}={v}' for k, v in sorted(by_priority.items()))})")
        print(f"  9-slice entries   : {counts_meta.get('nine_slice_required', '?')} "
              f"| field refs scanned: {counts_meta.get('field_inventory', '?')} "
              f"({counts_meta.get('field_inventory_prefab_only', '?')} prefab-only)")
        print("  status            : waiting for extraction; re-run once assets/game/manifest.json exists.")
        print("  naming scheme     : assets/game/ui/<usage>/<name>; see docs/ASSETS_UI.md")
        return True

    total = sum(counts.values())
    extracted = counts["exact"] + counts["suffix"] + counts["alias"]
    print(f"  manifest          : assets/game/manifest.json present")
    print(f"  extracted         : {extracted}/{total} "
          f"(exact {counts['exact']}, suffix {counts['suffix']}, alias {counts['alias']})")
    print(f"  renamed           : {counts['renamed']}")
    print(f"  missing           : {counts['missing']}")
    print(f"  missing (optional): {counts['missing_optional']} (game ships no art / scene-only)")
    print(f"  runtime generated : {counts['runtime']}")
    unclaimed = report.get("unclaimed_ui_names", [])
    print(f"  unclaimed extracted UI assets (informational): {len(unclaimed)}")

    missing = [r for r in report["results"] if r["status"] == "missing"]
    renamed = [r for r in report["results"] if r["status"] == "renamed"]
    if renamed:
        print("\n  renamed (same basename, different path) — update manifest or expectation:")
        for r in renamed[:25]:
            print(f"    {r['logical']}  ->  {r['rename_suggestion']}  [{r['priority']}]")
        if len(renamed) > 25:
            print(f"    … {len(renamed) - 25} more")
    if missing:
        print("\n  missing (not found in manifest):")
        for r in missing[:40]:
            kind = "prefab-only" if r["prefab_only"] else "code-reference"
            print(f"    {r['logical']}  [{r['priority']}, {kind}]")
        if len(missing) > 40:
            print(f"    … {len(missing) - 40} more (use --json for the full list)")
    optional = [r for r in report["results"] if r["status"] == "missing_optional"]
    if optional:
        print("\n  optional missing (expected — game/scene-only art):")
        for r in optional[:20]:
            print(f"    {r['logical']}  [{r['priority']}]")
        if len(optional) > 20:
            print(f"    … {len(optional) - 20} more")
    return counts["missing"] == 0 and counts["renamed"] == 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--expectations", type=Path, default=DEFAULT_EXPECTATIONS)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--json", action="store_true", help="print the machine-readable report")
    parser.add_argument("--strict", action="store_true", help="exit 1 when anything is missing/renamed")
    args = parser.parse_args()

    if not args.expectations.exists():
        print(f"error: expectations file not found: {args.expectations}", file=sys.stderr)
        return 2
    try:
        expectations = load_json(args.expectations)
    except (OSError, json.JSONDecodeError) as error:
        print(f"error: cannot read expectations: {error}", file=sys.stderr)
        return 2
    if expectations.get("schema_version") != 1:
        print("error: unsupported expectations schema_version", file=sys.stderr)
        return 2

    manifest = None
    if args.manifest.exists():
        try:
            manifest = load_json(args.manifest)
        except (OSError, json.JSONDecodeError) as error:
            print(f"warning: cannot read manifest ({error}); treating as absent", file=sys.stderr)

    try:
        report = check(expectations, manifest)
    except ValueError as error:
        print(f"error: malformed expectations: {error}", file=sys.stderr)
        return 2

    if args.json:
        report["expectations_counts"] = expectations.get("counts", {})
        print(json.dumps(report, indent=2))
    else:
        ok = summarise(expectations, report)
        if report["manifest_present"]:
            print("\n  result            : " + ("OK" if ok else "GAPS (see above)"))
    if args.strict and report["manifest_present"]:
        ok = report["counts"]["missing"] == 0 and report["counts"]["renamed"] == 0
        return 0 if ok else 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

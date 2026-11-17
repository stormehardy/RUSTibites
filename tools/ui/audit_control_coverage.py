#!/usr/bin/env python3
"""audit_control_coverage.py - functional coverage audit for viewer_control_ids.json.

Classifies every native runtime control as:
  concrete   - registry action/binding is a concrete UiAction
  value      - generic control with captured value/dropdown metadata (viewer applies SetValue/Choice)
  fallback   - no concrete action, but viewer's dispatch_runtime_control keyword fallback handles it
  generic    - generic ActivateControl with no fallback (id-based dispatch only)
  inert      - generic control that the current viewer dispatch leaves as a no-op

Also validates every registry rect against native atlas v2 `screen_bbox` (the
same field the viewer composer uses for hit rects), so visual and hit rects are
checked to share the native geometry.

Usage:
  python tools/ui/audit_control_coverage.py            # write docs/UI_FUNCTIONAL_COVERAGE.md
  python tools/ui/audit_control_coverage.py --check    # fail on missing/mismatched rects
"""

from __future__ import annotations

import argparse
import gzip
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "tools/ui/viewer_control_ids.json"
ATLAS = ROOT / "assets/game/ui_runtime_native.json.gz"
DOC = ROOT / "docs/UI_FUNCTIONAL_COVERAGE.md"
SCHEMA = "rustibites.ui.functional_coverage.v1"


def keyword_handled(session: str, path: str, label: str) -> bool:
    """Mirror ui_runtime/main.rs dispatch_runtime_control keyword fallback."""
    key = f"{session} {path} {label}".lower()
    if "workshop" in key:
        return True
    if "challenge picker" in key or "challenge selector" in key or "champion selection" in key:
        return True
    if "challenge parameters" in key or "stardescription" in key:
        return True
    if "challenge" in key:
        return True
    if "dynamic parameters" in key or "dynamicparameters" in key:
        return True
    if "world settings" in key or "worldsettings" in key:
        return True
    if "gene editor" in key or "genes tab" in key:
        return True
    if "brain editor" in key or "brain tab" in key:
        return True
    if "scenario editor" in key or "scenarioeditor" in key:
        return True
    if "apply" in key:
        return True
    if "save scenario" in key:
        return True
    if "add zone" in key or "zones/add" in key:
        return True
    if "remove zone" in key or "zones/remove" in key:
        return True
    if "settings" in key or "options" in key:
        return True
    if "place pellets" in key or "placepellets" in key or "placer" in key:
        return True
    if "phero" in key or "tower" in key:
        return True
    if "kill half" in key or "killhalf" in key:
        return True
    if "grid" in key:
        return True
    if "pheromone" in key and "toggle" in key:
        return True
    if "lineage" in key:
        return True
    if "library" in key or "editor" in key:
        return True
    if "achievement" in key:
        return True
    if "tool" in key:
        return True
    if "credit" in key:
        return True
    if "close" in key or "back" in key:
        return True
    if "kill" in key:
        return True
    if "revive" in key:
        return True
    if "lay" in key or "egg" in key:
        return True
    if "edit" in key:
        return True
    if "save" in key:
        return True
    if "load" in key:
        return True
    if "info" in key or "help" in key:
        return True
    return False


def classify(control: dict) -> str:
    action = control.get("action")
    if control.get("binding"):
        return "concrete"
    if action:
        return "generic" if "ActivateControl" in action else "concrete"
    value = control.get("value") or {}
    if control.get("kind") == "dropdown" and value.get("options"):
        return "value"
    if control.get("kind") in ("toggle", "slider") and value:
        return "value"
    if keyword_handled(control["session"], control["path"], control.get("label", "")):
        return "fallback"
    return "inert"


def active_nodes(session: dict, out: dict[str, list[dict]]) -> None:
    def walk(node):
        if not node.get("active_in_hierarchy"):
            return
        path = node.get("path")
        if path:
            out.setdefault(path, []).append(node)
        for child in node.get("children") or []:
            walk(child)

    for canvas in session.get("canvases") or []:
        walk(canvas)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", action="store_true", help="fail on rect mismatches instead of writing the doc")
    args = ap.parse_args()
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    atlas = json.load(gzip.open(ATLAS, "rt", encoding="utf-8"))
    controls = manifest["controls"]
    counts = Counter()
    by_session = defaultdict(Counter)
    inert = defaultdict(list)
    mismatches = []
    missing_rects = []
    sessions = sorted({control["session"] for control in controls})
    for session_name in sessions:
        session = atlas["sessions"][session_name]
        nodes = {}
        active_nodes(session, nodes)
        for control in controls:
            if control["session"] != session_name:
                continue
            kind = classify(control)
            counts[kind] += 1
            by_session[session_name][kind] += 1
            if kind == "inert":
                inert[session_name].append(control)
            candidates = nodes.get(control["path"], [])
            bboxes = [node.get("screen_bbox") for node in candidates if node.get("screen_bbox")]
            if not bboxes:
                missing_rects.append(f"{session_name}/{control['path']}")
                continue
            actual = control["rect"]
            matched = False
            expected = None
            for bbox in bboxes:
                # native v2 screen_bbox is already in 1920 screen pixels; the
                # viewer composer and control registry both consume it directly.
                expected = [bbox["x0"], bbox["y0"], bbox["w"], bbox["h"]]
                if all(abs(float(a) - float(b)) <= 0.1 for a, b in zip(actual, expected)):
                    matched = True
                    break
            if not matched:
                mismatches.append(
                    f"{session_name}/{control['path']}: registry {actual} != screen_bbox {expected}"
                )
    if args.check:
        for line in mismatches[:20]:
            print(line, file=sys.stderr)
        if mismatches or missing_rects:
            print(
                f"FAIL: {len(mismatches)} rect mismatches, {len(missing_rects)} missing screen_bbox",
                file=sys.stderr,
            )
            return 1
        print(
            f"ok: {len(controls)} controls / {len(sessions)} sessions; "
            f"rects match native screen_bbox; concrete={counts['concrete']} "
            f"value={counts['value']} fallback={counts['fallback']} "
            f"generic={counts['generic']} inert={counts['inert']}"
        )
        return 0

    total = len(controls)
    lines = [
        "# UI functional coverage audit (native registry v5)",
        "",
        "Generated by `tools/ui/audit_control_coverage.py` from `tools/ui/viewer_control_ids.json`",
        "(1493 native controls / 60 sessions, native atlas v2).",
        "",
        f"- total controls: **{total}**",
        f"- concrete action/binding: **{counts['concrete']}**",
        f"- handled value controls (toggle/slider/dropdown): **{counts['value']}**",
        f"- keyword-fallback handled: **{counts['fallback']}**",
        f"- generic `ActivateControl` only: **{counts['generic']}**",
        f"- inert/no-op: **{counts['inert']}**",
        "",
        "Rect check: every registry rect was compared to native atlas v2 `screen_bbox`; mismatches are listed",
        "in the audit run output. The viewer composer and hit registry both consume that native rect.",
        "",
        "## Per-session coverage",
        "",
        "| session | controls | concrete | value | fallback | generic | inert |",
        "|---|---:|---:|---:|---:|---:|---:|",
    ]
    for session_name in sessions:
        row = by_session[session_name]
        lines.append(
            f"| `{session_name}` | {sum(row.values())} | {row['concrete']} | {row['value']} | "
            f"{row['fallback']} | {row['generic']} | {row['inert']} |"
        )
    examples = [
        "PlacerButton", "UserActions", "ViewButton", "SpeciesHolder", "BrainDiffToggle",
        "ExpandedViewerButton", "SortButton", "BrainHelpButton", "LoadBibite", "RadTower",
        "RemovePellets", "BackButton", "ResetButton", "Edit Button",
    ]
    lines.extend(["", "## Named example controls", "", "| name | classification | session | path |", "|---|---|---|---|"])
    for name in examples:
        matches = [control for control in controls if control.get("name") == name]
        if not matches:
            lines.append(f"| `{name}` | not captured | - | - |")
            continue
        for control in matches[:3]:
            lines.append(
                f"| `{name}` | {classify(control)} | `{control['session']}` | `{control['path']}` |"
            )
    lines.extend(["", "## Inert / unsupported controls", ""])
    if not inert:
        lines.append("None.")
    for session_name in sorted(inert):
        lines.append(f"### `{session_name}`")
        for control in inert[session_name]:
            lines.append(
                f"- `{control['path']}` — name=`{control.get('name', '')}` label=`{control.get('label', '')}` "
                f"kind=`{control.get('kind')}` rect={control['rect']}"
            )
        lines.append("")
    DOC.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(
        f"wrote {DOC}: total={total} concrete={counts['concrete']} value={counts['value']} "
        f"fallback={counts['fallback']} generic={counts['generic']} inert={counts['inert']} "
        f"rect_mismatches={len(mismatches)} missing_bbox={len(missing_rects)}"
    )
    return 1 if mismatches else 0


if __name__ == "__main__":
    raise SystemExit(main())

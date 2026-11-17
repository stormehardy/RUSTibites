#!/usr/bin/env python3
"""build_control_ids.py - regenerate tools/ui/viewer_control_ids.json from the
viewer's native control dump.

The dump is produced by::

    rustibites-view --ui-controls --ui-controls-out target/viewer_controls_native.json

It is the authoritative enumeration of ``RuntimeUi::controls(1920, 1080)``:
native atlas sessions, stable FNV-1a ids, composed native rects, and captured
value/action metadata. This script validates every id, parses the Rust debug
``value`` metadata into JSON, and writes the committed registry.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_DUMP = ROOT / "target/viewer_controls_native.json"
DEFAULT_OUT = ROOT / "tools/ui/viewer_control_ids.json"
SCHEMA = "rustibites.viewer.ui_control_ids.v5"
FNV_OFFSET = 0xCBF29CE484222325
FNV_PRIME = 0x100000001B3
VALUE_FIELDS = (
    "is_on",
    "value",
    "min_value",
    "max_value",
    "whole_numbers",
    "options",
    "selected_index",
    "option_rects",
)


def control_id(session: str, path: str) -> int:
    value = FNV_OFFSET
    for byte in (session + "/" + path).encode("utf-8"):
        value ^= byte
        value = (value * FNV_PRIME) & 0xFFFFFFFFFFFFFFFF
    return value


def _rust_scalar_to_json(text: str):
    text = text.strip()
    if text.startswith("Some(") and text.endswith(")"):
        text = text[len("Some(") : -1].strip()
    if text == "None":
        return None
    if text in ("true", "false"):
        return text == "true"
    if text.startswith("["):
        return json.loads(text)
    if text.startswith('"'):
        return json.loads(text)
    return int(text) if re.fullmatch(r"-?\d+", text) else float(text)


def parse_value_meta(debug: str | None):
    if not debug:
        return None
    if not debug.startswith("ControlValueMeta {"):
        raise ValueError(f"unexpected value debug: {debug[:80]}")
    body = debug[len("ControlValueMeta {") :].rstrip().rstrip("}")
    fields = {}
    for index, field in enumerate(VALUE_FIELDS):
        following = VALUE_FIELDS[index + 1] if index + 1 < len(VALUE_FIELDS) else None
        pattern = re.escape(field) + r":\s*(.*?)(?=,\s*" + re.escape(following) + r":|\Z)" if following else re.escape(field) + r":\s*(.*)$"
        match = re.search(pattern, body, re.S)
        if not match:
            raise ValueError(f"missing {field} in {debug[:120]}")
        fields[field] = _rust_scalar_to_json(match.group(1))
    return fields


def build(dump_path: Path) -> dict:
    dump = json.loads(dump_path.read_text(encoding="utf-8"))
    if dump.get("schema") != "rustibites.viewer.controls.v1":
        raise SystemExit(f"unexpected dump schema: {dump.get('schema')}")
    controls = []
    errors = []
    for session in sorted(dump.get("sessions") or {}):
        session_controls = dump["sessions"][session]
        for control in session_controls:
            expected = control_id(session, control["path"])
            if int(control["id"]) != expected:
                errors.append(f"{session}/{control['path']}: id {control['id']} != {expected}")
            controls.append(
                {
                    "id": int(control["id"]),
                    "session": session,
                    "path": control["path"],
                    "name": control.get("name", ""),
                    "label": control.get("label", ""),
                    "kind": str(control.get("kind", "")).lower(),
                    "rect": [float(v) for v in control.get("rect") or [0.0, 0.0, 0.0, 0.0]],
                    "value": parse_value_meta(control.get("value")),
                    "action": control.get("action"),
                    "binding": control.get("binding"),
                }
            )
    if errors:
        for error in errors[:20]:
            print(error, file=sys.stderr)
        raise SystemExit(f"{len(errors)} control id mismatches")
    controls.sort(key=lambda control: (control["session"], control["path"]))
    return {
        "schema": SCHEMA,
        "source": "target/viewer_controls_native.json",
        "source_atlas": "assets/game/ui_runtime_native.json.gz",
        "id_algorithm": "FNV-1a 64 over bytes session + '/' + path",
        "dynamic_row_id_algorithm": "FNV-1a 64 over bytes session + '/' + scroll_view_path + '/#row:' + index",
        "screen": {"width": 1920, "height": 1080},
        "sessions": {name: len(dump["sessions"][name]) for name in sorted(dump["sessions"])},
        "count": len(controls),
        "controls": controls,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dump", default=str(DEFAULT_DUMP))
    ap.add_argument("--out", default=str(DEFAULT_OUT))
    ap.add_argument("--check", action="store_true", help="verify the committed registry is up to date")
    args = ap.parse_args()
    built = build(Path(args.dump))
    text = json.dumps(built, indent=1) + "\n"
    if args.check:
        current = Path(args.out).read_text(encoding="utf-8")
        if current != text:
            print(f"out of date: {args.out}", file=sys.stderr)
            return 1
        print(f"up to date: {built['count']} controls / {len(built['sessions'])} sessions")
        return 0
    Path(args.out).write_text(text, encoding="utf-8")
    print(
        f"wrote {args.out}: {built['count']} controls / {len(built['sessions'])} sessions, "
        f"sessions={sum(1 for _ in built['sessions'])}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

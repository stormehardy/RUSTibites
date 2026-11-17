"""Extracts the UI event wiring serialized in a Unity scene: every UnityEvent persistent call (Button.onClick,
Toggle.onValueChanged, Slider, InputField, UIPanel.onOpen/onClose, ...) with its target component, method and
argument, keyed by the hierarchy path of the control that hosts it (same path format as the runtime UI dumps).

Release builds strip MonoBehaviour typetrees, so the raw bytes are scanned for `PersistentCall` records:
    m_Target PPtr (fileID i32, pathID i64) | m_TargetAssemblyTypeName string | m_MethodName string |
    m_Mode i32 | m_Arguments { m_ObjectArgument PPtr, m_ObjectArgumentAssemblyTypeName string, m_IntArgument i32,
    m_FloatArgument f32, m_StringArgument string, m_BoolArgument u8 (+pad) } | m_CallState i32
Strings are length-prefixed and padded to 4 bytes. Listeners added in code (`onClick.AddListener`) are not in the
scene data; the C# in `Here/` has those.

Usage: python tools/unity/dump_ui_wiring.py <game data dir> <level file> <out.json>
Read-only on the game install.
"""
import json
import os
import re
import struct
import sys

import UnityPy

data_dir, level, out_path = sys.argv[1], sys.argv[2], sys.argv[3]
env = UnityPy.load(os.path.join(data_dir, level))
objects = {o.path_id: o for o in env.objects}
external = {}  # file name -> {path id: object}

TYPE_NAME = re.compile(rb'^[\w.+`]+(, [\w.]+)?$')
METHOD_NAME = re.compile(rb'^[A-Za-z_][\w`<>]*$')
MODES = {0: 'EventDefined', 1: 'Void', 2: 'Object', 3: 'Int', 4: 'Float', 5: 'String', 6: 'Bool'}


def resolve(owner, file_id, path_id):
    if file_id == 0:
        return objects.get(path_id)
    try:
        ext = owner.assets_file.externals[file_id - 1]
    except IndexError:
        return None
    name = os.path.basename(ext.path)
    if name not in external:
        try:
            loaded = UnityPy.load(os.path.join(data_dir, name))
            external[name] = {o.path_id: o for o in loaded.objects}
        except Exception:  # noqa: BLE001
            external[name] = {}
    return external[name].get(path_id)


script_cache = {}


def script_class(owner, raw):
    file_id, path_id = struct.unpack_from('<iq', raw, 16)
    key = (os.path.basename(owner.assets_file.name) if hasattr(owner.assets_file, 'name') else '', file_id, path_id)
    if key not in script_cache:
        try:
            target = resolve(owner, file_id, path_id)
            script_cache[key] = target.read().m_ClassName if target is not None else '?'
        except Exception:  # noqa: BLE001
            script_cache[key] = '?'
    return script_cache[key]


# --- hierarchy paths ------------------------------------------------------------------------------
go_names = {}
transform_of_go = {}
father_of_transform = {}
for o in env.objects:
    kind = o.type.name
    if kind == 'GameObject':
        try:
            go_names[o.path_id] = o.read_typetree()['m_Name']
        except Exception:  # noqa: BLE001
            pass
    elif kind in ('RectTransform', 'Transform'):
        try:
            tree = o.read_typetree()
        except Exception:  # noqa: BLE001
            continue
        transform_of_go[tree['m_GameObject']['m_PathID']] = o.path_id
        father_of_transform[o.path_id] = tree['m_Father']['m_PathID']
go_of_transform = {t: g for g, t in transform_of_go.items()}
path_cache = {}


def go_path(go_id):
    if go_id in path_cache:
        return path_cache[go_id]
    names = []
    transform = transform_of_go.get(go_id)
    guard = 0
    while transform is not None and guard < 80:
        go = go_of_transform.get(transform)
        names.append(go_names.get(go, '?'))
        father = father_of_transform.get(transform)
        transform = father if father else None
        guard += 1
    path = '/'.join(reversed(names)) if names else go_names.get(go_id, '?')
    path_cache[go_id] = path
    return path


def read_string(raw, p):
    if p + 4 > len(raw):
        return None, p
    n = struct.unpack_from('<I', raw, p)[0]
    if n > 400 or p + 4 + n > len(raw):
        return None, p
    return raw[p + 4:p + 4 + n], p + 4 + ((n + 3) & ~3)


def parse_call(raw, p):
    """A PersistentCall starting at `p`, or None."""
    if p + 12 > len(raw):
        return None
    file_id, path_id = struct.unpack_from('<iq', raw, p)
    if file_id < 0 or file_id > 20 or path_id < 0:
        return None
    type_name, q = read_string(raw, p + 12)
    if type_name is None or not TYPE_NAME.match(type_name) or len(type_name) < 3:
        return None
    method, q = read_string(raw, q)
    if method is None or not METHOD_NAME.match(method):
        return None
    if q + 4 > len(raw):
        return None
    mode = struct.unpack_from('<i', raw, q)[0]
    q += 4
    if mode not in MODES:
        return None
    if q + 12 > len(raw):
        return None
    arg_obj = struct.unpack_from('<iq', raw, q)
    q += 12
    arg_type, q = read_string(raw, q)
    if arg_type is None:
        return None
    if q + 8 > len(raw):
        return None
    int_arg, float_arg = struct.unpack_from('<if', raw, q)
    q += 8
    str_arg, q = read_string(raw, q)
    if str_arg is None:
        return None
    if q + 8 > len(raw):
        return None
    bool_arg = raw[q]
    q += 4
    state = struct.unpack_from('<i', raw, q)[0]
    q += 4
    if state not in (0, 1, 2):
        return None
    arg = {3: int_arg, 4: round(float_arg, 6), 5: str_arg.decode('utf-8', 'replace'), 6: bool(bool_arg)}.get(mode)
    return {
        'file_id': file_id,
        'path_id': path_id,
        'target_type': type_name.decode(),
        'method': method.decode(),
        'mode': MODES[mode],
        'arg': arg,
        'call_state': ['Off', 'EditorAndRuntime', 'RuntimeOnly'][state],
    }, q


def describe_target(owner, call):
    target = resolve(owner, call['file_id'], call['path_id'])
    if target is None:
        return {'class': '?', 'path': None, 'external': call['file_id'] != 0}
    kind = target.type.name
    if kind == 'MonoBehaviour':
        raw = target.get_raw_data()
        go_id = struct.unpack_from('<q', raw, 4)[0]
        return {'class': script_class(target, raw), 'path': go_path(go_id) if call['file_id'] == 0 else None,
                'external': call['file_id'] != 0}
    if kind == 'GameObject':
        return {'class': 'GameObject', 'path': go_path(target.path_id) if call['file_id'] == 0 else None,
                'external': call['file_id'] != 0}
    try:
        go_id = target.read_typetree()['m_GameObject']['m_PathID']
        return {'class': kind, 'path': go_path(go_id) if call['file_id'] == 0 else None,
                'external': call['file_id'] != 0}
    except Exception:  # noqa: BLE001
        return {'class': kind, 'path': None, 'external': call['file_id'] != 0}


records = []
for o in env.objects:
    if o.type.name != 'MonoBehaviour':
        continue
    raw = o.get_raw_data()
    if len(raw) < 64:
        continue
    calls = []
    groups = 0
    p = 32
    previous_end = -1
    while p + 24 <= len(raw):
        parsed = parse_call(raw, p)
        if parsed is None:
            p += 4
            continue
        call, end = parsed
        if p != previous_end:
            groups += 1
        call['event'] = groups
        calls.append(call)
        previous_end = end
        p = end
    if not calls:
        continue
    host_go = struct.unpack_from('<q', raw, 4)[0]
    host_class = script_class(o, raw)
    for call in calls:
        target = describe_target(o, call)
        call['target_class'] = target['class']
        call['target_path'] = target['path']
        call['target_external'] = target['external']
        del call['file_id'], call['path_id']
    records.append({
        'path': go_path(host_go),
        'host': host_class,
        'calls': calls,
    })

records.sort(key=lambda r: (r['path'], r['host']))
with open(out_path, 'w', encoding='utf-8') as handle:
    json.dump({'schema': 'rustibites.ui_wiring.v1', 'scene': level, 'controls': records}, handle, indent=1)
print(f'{len(records)} hosts with persistent calls, {sum(len(r["calls"]) for r in records)} calls -> {out_path}')

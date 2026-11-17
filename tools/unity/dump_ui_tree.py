"""Dumps the UI hierarchy below one GameObject of a scene: names, activity, RectTransform geometry and
the useful bits of Image / TextMeshPro components (the release build strips MonoBehaviour typetrees,
so those are read from the raw bytes).

Usage: python tools/unity/dump_ui_tree.py <game data dir> <level file> <GameObject path id> [max depth]
Read-only on the game install. `find_objects.py` lists path ids by name.

Raw layouts (Unity UI 1.0 / TMP 3.x, 32-byte MonoBehaviour header first):
* Image (136 bytes): m_Color @44 (4 floats), m_Sprite PPtr @88 (fileID i32, pathID i64), m_Type @100,
  pixelsPerUnitMultiplier @132.
* TextMeshProUGUI: m_Color @44, m_text string @88 (u32 length + UTF-8, padded to 4).
"""
import os
import struct
import sys

import UnityPy

data_dir, level, root_id = sys.argv[1], sys.argv[2], int(sys.argv[3])
max_depth = int(sys.argv[4]) if len(sys.argv) > 4 else 6

env = UnityPy.load(os.path.join(data_dir, level))
objects = {o.path_id: o for o in env.objects}
file_objects = {}  # (file name) -> {path id: object} for external references


def resolve(owner, file_id, path_id):
    """The object a PPtr (found in `owner`) points to, loading external files on demand."""
    if file_id == 0:
        return objects.get(path_id)
    external = owner.assets_file.externals[file_id - 1]
    name = os.path.basename(external.path)
    if name not in file_objects:
        loaded = UnityPy.load(os.path.join(data_dir, name))
        file_objects[name] = {o.path_id: o for o in loaded.objects}
    return file_objects[name].get(path_id)


def script_name(owner, raw):
    """Class name of a MonoBehaviour from its m_Script PPtr (@16: fileID i32, pathID i64)."""
    file_id, path_id = struct.unpack_from('<iq', raw, 16)
    try:
        target = resolve(owner, file_id, path_id)
        return '?' if target is None else target.read().m_ClassName
    except Exception as error:  # noqa: BLE001
        return f'?{error.__class__.__name__}:{error}'


def read_string(raw, offset):
    if offset + 4 > len(raw):
        return None
    length = struct.unpack_from('<I', raw, offset)[0]
    if length > 2000 or offset + 4 + length > len(raw):
        return None
    try:
        return raw[offset + 4:offset + 4 + length].decode('utf-8')
    except UnicodeDecodeError:
        return None


def sprite_name(owner, file_id, path_id):
    try:
        target = resolve(owner, file_id, path_id)
        return target.read().m_Name if target is not None else None
    except Exception:  # noqa: BLE001
        return None


def describe_component(obj):
    kind = obj.type.name
    if kind in ('RectTransform', 'CanvasRenderer', 'Canvas', 'CanvasGroup'):
        return None
    if kind != 'MonoBehaviour':
        return kind
    raw = obj.get_raw_data()
    name = script_name(obj, raw)
    out = f'{name}[{len(raw)}]'
    if name == 'Image' and len(raw) == 136:
        colour = struct.unpack_from('<4f', raw, 44)
        sprite = sprite_name(obj, *struct.unpack_from('<iq', raw, 88))
        image_type = struct.unpack_from('<i', raw, 100)[0]
        multiplier = struct.unpack_from('<f', raw, 132)[0]
        out += f' color=({colour[0]:.3f},{colour[1]:.3f},{colour[2]:.3f},{colour[3]:.3f}) sprite={sprite} type={image_type} ppum={multiplier:g}'
    elif name.startswith('TextMeshPro') or name == 'TMP_Text':
        colour = struct.unpack_from('<4f', raw, 44)
        text = read_string(raw, 88)
        out += f' text={text!r} color=({colour[0]:.2f},{colour[1]:.2f},{colour[2]:.2f},{colour[3]:.2f})'
    return out


def rect_of(tree):
    return (f"anchor=({tree['m_AnchorMin']['x']:g},{tree['m_AnchorMin']['y']:g})-({tree['m_AnchorMax']['x']:g},{tree['m_AnchorMax']['y']:g})"
            f" pos=({tree['m_AnchoredPosition']['x']:g},{tree['m_AnchoredPosition']['y']:g})"
            f" size=({tree['m_SizeDelta']['x']:g},{tree['m_SizeDelta']['y']:g})"
            f" pivot=({tree['m_Pivot']['x']:g},{tree['m_Pivot']['y']:g})"
            f" scale=({tree['m_LocalScale']['x']:g},{tree['m_LocalScale']['y']:g})")


def dump(go_id, depth):
    go = objects[go_id].read_typetree()
    pad = '  ' * depth
    rect_tree = None
    parts = []
    for component in go['m_Component']:
        obj = objects.get(component['component']['m_PathID'])
        if obj is None:
            continue
        if obj.type.name == 'RectTransform':
            rect_tree = obj.read_typetree()
        else:
            try:
                described = describe_component(obj)
            except Exception as error:  # noqa: BLE001
                described = f'{obj.type.name}!{error.__class__.__name__}'
            if described:
                parts.append(described)
    active = '' if go.get('m_IsActive', 1) else ' (inactive)'
    print(f"{pad}{go['m_Name']}{active} {rect_of(rect_tree) if rect_tree else ''}")
    for part in parts:
        print(f'{pad}    - {part}')
    if rect_tree is None or depth >= max_depth:
        return
    for child in rect_tree['m_Children']:
        child_transform = objects.get(child['m_PathID'])
        if child_transform is not None:
            dump(child_transform.read_typetree()['m_GameObject']['m_PathID'], depth + 1)


dump(root_id, 0)

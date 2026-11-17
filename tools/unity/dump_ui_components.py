"""Raw serialized values of the layout and drawing components below one GameObject of a scene (read-only).

`dump_ui_tree.py` prints the hierarchy and the RectTransforms; the numbers a layout needs (padding, spacing, child alignment, preferred and
flexible sizes, a text's size and alignment, a Shapes line's colour and thickness) are in components whose typetrees the release build
strips, so they are read from the raw bytes (offsets documented in the memory note "unity-ui-prefab-dump"):

* `LayoutElement` (64 bytes): ignoreLayout @32, min W/H @36/40, preferred W/H @44/48, flexible W/H @52/56, priority @60 (-1 = unset).
* `HorizontalLayoutGroup` / `VerticalLayoutGroup` (84): padding L/R/T/B @32..47, childAlignment @48 (TextAnchor: 0 UpperLeft .. 8 LowerRight),
  spacing @52, then four 4-byte bools: childForceExpandWidth / Height, childControlWidth / Height @56..71, then childScaleWidth / Height and
  reverseArrangement @72..83 (a reversed vertical group lists its first child at the BOTTOM).
* `TextMeshProUGUI`: the text at @88; after its padded end: colour32 at +48, fontStyle +168, fontSize +192, horizontal alignment +220
  (1 left, 2 centre, 4 right), vertical +224 (0x200 middle).
* `FloatValueTextHandle` (len 76 / 80): after the 32 byte header and a 12 byte text PPtr: prefix and suffix strings (u32 length + padded
  bytes), precision i32, factor f32, then SI, alwaysSign, precisionIsSI, isInt as 4 byte bools.
* Shapes `Line` / `Rectangle`: the colour is four floats from @40 (`Line`) / @40 (`Rectangle`), the rest are start / end / thickness / size floats
  that are printed unlabelled.

Usage: python tools/unity/dump_ui_components.py <game data dir> <level file> <GameObject path id> [ScriptClass ...]
Without classes it prints LayoutElement, the two layout groups, Rectangle, Line, FloatValueTextHandle and TextMeshProUGUI.
"""
import os
import struct
import sys

import UnityPy

data_dir, level, root = sys.argv[1], sys.argv[2], int(sys.argv[3])
wanted = set(sys.argv[4:]) or {
    'LayoutElement', 'HorizontalLayoutGroup', 'VerticalLayoutGroup', 'Rectangle', 'Line', 'FloatValueTextHandle', 'TextMeshProUGUI',
}
env = UnityPy.load(os.path.join(data_dir, level))
objects = {o.path_id: o for o in env.objects}
external = {}
cache = {}


def target(owner, file_id, path_id):
    if file_id == 0:
        return objects.get(path_id)
    name = os.path.basename(owner.assets_file.externals[file_id - 1].path)
    if name not in external:
        external[name] = {o.path_id: o for o in UnityPy.load(os.path.join(data_dir, name)).objects}
    return external[name].get(path_id)


def script_of(obj):
    key = struct.unpack_from('<iq', obj.get_raw_data(), 16)
    if key not in cache:
        try:
            script = target(obj, *key)
            cache[key] = '?' if script is None else script.read().m_ClassName
        except Exception:  # noqa: BLE001
            cache[key] = '?'
    return cache[key]


def floats(raw, start, end):
    return [round(struct.unpack_from('<f', raw, o)[0], 4) for o in range(start, min(end, len(raw) - 3), 4)]


def ints(raw, start, end):
    return [struct.unpack_from('<i', raw, o)[0] for o in range(start, min(end, len(raw) - 3), 4)]


def padded_string(raw, offset):
    length = struct.unpack_from('<I', raw, offset)[0]
    text = raw[offset + 4:offset + 4 + length].decode('utf-8', 'replace')
    return text, offset + 4 + ((length + 3) // 4) * 4


def describe(cls, raw, pad):
    if cls == 'LayoutElement':
        print(pad + f'ignore {ints(raw, 32, 36)} min {floats(raw, 36, 44)} pref {floats(raw, 44, 52)} '
                    f'flex {floats(raw, 52, 60)} prio {ints(raw, 60, 64)}')
    elif cls in ('HorizontalLayoutGroup', 'VerticalLayoutGroup'):
        print(pad + f'padding {ints(raw, 32, 48)} align {ints(raw, 48, 52)} spacing {floats(raw, 52, 56)} '
                    f'expand/control {ints(raw, 56, 72)} scale/reverse {ints(raw, 72, 84)}')
    elif cls in ('Rectangle', 'Line'):
        print(pad + f'floats {floats(raw, 32, min(len(raw), 280))}')
    elif cls == 'FloatValueTextHandle':
        try:
            prefix, off = padded_string(raw, 44)
            suffix, off = padded_string(raw, off)
            precision = struct.unpack_from('<i', raw, off)[0]
            factor = struct.unpack_from('<f', raw, off + 4)[0]
            flags = [raw[off + 8 + 4 * i] for i in range(4)]
            print(pad + f'prefix {prefix!r} suffix {suffix!r} precision {precision} factor {factor} '
                        f'SI/sign/precisionIsSI/isInt {flags}')
        except Exception as error:  # noqa: BLE001
            print(pad + f'(could not parse: {error})')
    elif cls == 'TextMeshProUGUI':
        text, end = padded_string(raw, 88)
        color = list(raw[end + 48:end + 52])
        print(pad + f'text {text!r} color32 {color} size {struct.unpack_from("<f", raw, end + 192)[0]:.4g} '
                    f'style {struct.unpack_from("<i", raw, end + 168)[0]} '
                    f'align {struct.unpack_from("<i", raw, end + 220)[0]}/{struct.unpack_from("<i", raw, end + 224)[0]}')


def walk(go_id, depth):
    go = objects[go_id].read_typetree()
    pad = '  ' * depth
    print(f'{pad}GO {go_id} {go["m_Name"]}')
    rect = None
    for component in go['m_Component']:
        obj = objects.get(component['component']['m_PathID'])
        if obj is None:
            continue
        if obj.type.name == 'RectTransform':
            rect = obj.read_typetree()
        if obj.type.name != 'MonoBehaviour':
            continue
        cls = script_of(obj)
        if cls in wanted:
            print(f'{pad}  [{cls}] mono {obj.path_id}')
            describe(cls, obj.get_raw_data(), pad + '    ')
    if rect is not None:
        for child in rect['m_Children']:
            child_rect = objects[child['m_PathID']].read_typetree()
            walk(child_rect['m_GameObject']['m_PathID'], depth + 1)


walk(root, 0)

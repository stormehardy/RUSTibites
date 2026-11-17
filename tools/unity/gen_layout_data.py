"""Generates Rust source for the layout data of one prefab hierarchy: every GameObject below a root with its active flag, authored
RectTransform, layout group (`HorizontalLayoutGroup` / `VerticalLayoutGroup`), `ContentSizeFitter`, `LayoutElement` and TextMeshPro text.

The viewer's `unity_layout.rs` solves these the way UnityEngine.UI does, so a panel whose captured geometry is one state of many (the
Stats panel with or without the parents line, the egg section, the dead bibite's buttons; the Brain panel with its holder's size and the
species name) can be laid out for any state (`stats_layout_data.rs`, `brain_panel_layout_data.rs`).

Usage: python tools/unity/gen_layout_data.py <game data dir> <level file> <GameObject path id> <CONST_NAME> > data.rs
Read-only on the game install. Raw layouts (MonoBehaviours have no typetree in release builds, 32-byte header first):
* `HorizontalOrVerticalLayoutGroup`: padding L/R/T/B i32 @32, childAlignment i32 @48, spacing f32 @52, then forceExpandW, forceExpandH,
  controlW, controlH, scaleW, scaleH, reverse as 4-byte bools @56;
* `ContentSizeFitter`: horizontalFit, verticalFit i32 @32 (0 unconstrained, 1 min, 2 preferred);
* `LayoutElement`: ignoreLayout @32, minW minH prefW prefH flexW flexH f32 @36 (-1 = unset), layoutPriority @60;
* `TextMeshProUGUI`: text string @88, font asset PPtr 4 bytes after its end, fontSize f32 @end+192, alignment @end+220 / end+224.
"""
import os
import struct
import sys

import UnityPy

data_dir, level, root_id, const_name = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
env = UnityPy.load(os.path.join(data_dir, level))
objects = {o.path_id: o for o in env.objects}
cache = {}


def resolve(owner, file_id, path_id):
    if file_id == 0:
        return objects.get(path_id)
    external = owner.assets_file.externals[file_id - 1]
    name = os.path.basename(external.path)
    if name not in cache:
        cache[name] = {o.path_id: o for o in UnityPy.load(os.path.join(data_dir, name)).objects}
    return cache[name].get(path_id)


def script_name(obj):
    raw = obj.get_raw_data()
    file_id, path_id = struct.unpack_from('<iq', raw, 16)
    target = resolve(obj, file_id, path_id)
    return '?' if target is None else target.read().m_ClassName


def rust_str(text):
    out = text.replace('\\', '\\\\').replace('"', '\\"').replace('\n', '\\n').replace('\r', '\\r').replace('\u200b', '\\u{200b}')
    return '"' + out + '"'


def f(value):
    text = repr(float(value))
    return text if ('.' in text or 'e' in text or 'inf' in text) else text + '.0'


def emit(pid, depth):
    go_obj = objects[pid]
    go = go_obj.read_typetree()
    rect = None
    group = None
    fitter = None
    element = None
    text = None
    image = None
    for comp in go['m_Component']:
        co = resolve(go_obj, comp['component']['m_FileID'], comp['component']['m_PathID'])
        if co is None:
            continue
        if co.type.name == 'RectTransform':
            rect = co.read_typetree()
        elif co.type.name == 'MonoBehaviour':
            name = script_name(co)
            raw = co.get_raw_data()
            if name in ('HorizontalLayoutGroup', 'VerticalLayoutGroup'):
                pad = struct.unpack_from('<4i', raw, 32)
                align = struct.unpack_from('<i', raw, 48)[0]
                spacing = struct.unpack_from('<f', raw, 52)[0]
                flags = struct.unpack_from('<7i', raw, 56)
                group = (name == 'VerticalLayoutGroup', pad, spacing, align, flags)
            elif name == 'ContentSizeFitter':
                fitter = struct.unpack_from('<2i', raw, 32)
            elif name == 'LayoutElement':
                ignore = struct.unpack_from('<i', raw, 32)[0]
                values = struct.unpack_from('<6f', raw, 36)
                element = (ignore, values)
            elif name == 'Image' and len(raw) == 136:
                sprite_file, sprite_path = struct.unpack_from('<iq', raw, 88)
                image_type = struct.unpack_from('<i', raw, 100)[0]
                multiplier = struct.unpack_from('<f', raw, 132)[0]
                sprite = resolve(co, sprite_file, sprite_path) if sprite_path else None
                if sprite is not None:
                    info = sprite.read_typetree()
                    scale = (info['m_PixelsToUnits'] / 100.0) * (multiplier if multiplier > 0 else 1.0)
                    if image_type in (1, 2):
                        border = info['m_Border']
                        w, h = border['x'] + border['z'], border['y'] + border['w']
                    else:
                        w, h = info['m_Rect']['width'], info['m_Rect']['height']
                    image = (w / scale, h / scale)
            elif name == 'TextMeshProUGUI':
                length = struct.unpack_from('<I', raw, 88)[0]
                end = (88 + 4 + length + 3) & ~3
                font_file, font_path = struct.unpack_from('<iq', raw, end + 4)
                font = resolve(co, font_file, font_path)
                font_name = ''
                if font is not None:
                    font_raw = font.get_raw_data()
                    name_len = struct.unpack_from('<I', font_raw, 28)[0]
                    font_name = font_raw[32:32 + name_len].decode('utf-8', 'replace')
                size = struct.unpack_from('<f', raw, end + 192)[0]
                style = struct.unpack_from("<i", raw, end + 216)[0]
                halign = struct.unpack_from('<i', raw, end + 220)[0]
                valign = struct.unpack_from('<i', raw, end + 224)[0]
                text = (raw[92:92 + length].decode('utf-8', 'replace'), font_name, size, halign, valign, bool(style & 1))
    pad = '    ' * depth
    lines = [f'{pad}LNode {{']
    lines.append(f'{pad}    name: {rust_str(go["m_Name"])},')
    lines.append(f'{pad}    active: {"true" if go["m_IsActive"] else "false"},')
    amin, amax = rect['m_AnchorMin'], rect['m_AnchorMax']
    pos, size, pivot = rect['m_AnchoredPosition'], rect['m_SizeDelta'], rect['m_Pivot']
    lines.append(
        f'{pad}    rect: [{f(amin["x"])}, {f(amin["y"])}, {f(amax["x"])}, {f(amax["y"])}, {f(pos["x"])}, {f(pos["y"])}, '
        f'{f(size["x"])}, {f(size["y"])}, {f(pivot["x"])}, {f(pivot["y"])}],'
    )
    if group is None:
        lines.append(f'{pad}    group: None,')
    else:
        vertical, padding, spacing, align, flags = group
        lines.append(
            f'{pad}    group: Some(Group {{ vertical: {"true" if vertical else "false"}, pad: [{f(padding[0])}, {f(padding[1])}, {f(padding[2])}, {f(padding[3])}], '
            f'spacing: {f(spacing)}, align: {align}, control: [{"true" if flags[2] else "false"}, {"true" if flags[3] else "false"}], '
            f'expand: [{"true" if flags[0] else "false"}, {"true" if flags[1] else "false"}] }}),'
        )
    lines.append(f'{pad}    fit: [{fitter[0] if fitter else 0}, {fitter[1] if fitter else 0}],')
    if element is None:
        lines.append(f'{pad}    elem: None,')
    else:
        ignore, v = element
        lines.append(
            f'{pad}    elem: Some(Elem {{ ignore: {"true" if ignore else "false"}, min: [{f(v[0])}, {f(v[1])}], pref: [{f(v[2])}, {f(v[3])}], flex: [{f(v[4])}, {f(v[5])}] }}),'
        )
    if text is None:
        lines.append(f'{pad}    text: None,')
    else:
        lines.append(
            f'{pad}    text: Some(Text {{ text: {rust_str(text[0])}, font: {rust_str(text[1])}, size: {f(text[2])}, bold: {"true" if text[5] else "false"}, halign: {text[3]}, valign: {text[4]} }}),'
        )
    if image is None:
        lines.append(f'{pad}    image: None,')
    else:
        lines.append(f'{pad}    image: Some([{f(image[0])}, {f(image[1])}]),')
    children = []
    for child in rect['m_Children']:
        child_tr = resolve(go_obj, child['m_FileID'], child['m_PathID'])
        if child_tr is not None:
            children.append(child_tr.read_typetree()['m_GameObject']['m_PathID'])
    if not children:
        lines.append(f'{pad}    children: &[],')
    else:
        lines.append(f'{pad}    children: &[')
        for child in children:
            lines.append(emit(child, depth + 2) + ',')
        lines.append(f'{pad}    ],')
    lines.append(f'{pad}}}')
    return '\n'.join(lines)


print(f'// Generated by tools/unity/gen_layout_data.py ({level}, GameObject {root_id}): do not edit by hand.')
print('use crate::unity_layout::{Elem, Group, LNode, Text};')
print()
print('#[rustfmt::skip]')
print('#[allow(clippy::excessive_precision)]')
print(f'pub const {const_name}: LNode = {emit(root_id, 0)};')

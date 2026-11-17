"""Writes crates/bibites-view/src/biology_popups_data.rs: the layout of the Biology panel's organ pop-ups (womb, mouth, fat), read from the native capture.

python tools/ui/gen_biology_popups.py

Each pop-up is a tree of RectTransforms the capture measured (layout groups already applied). A node's rectangle is turned into pixels from the
pop-up's top left. A text box that is as wide as its text (capture width 0) is a 65 wide box (the layout elements' minimum) centred where the capture
puts it. The tree is flattened in drawing order; the runtime binds values to it by path (`biology_popups.rs`).
"""
import gzip
import json
import os
import re
import subprocess
import sys

capture = json.load(gzip.open('assets/game/ui_runtime_native.json.gz'))
session = capture['sessions']['default_world_biology']
roots = {}


WANTED = [('WOMB', 'EggsDetailPopup', 1751), ('MOUTH', 'MouthDetailsPopUp', 2117), ('FAT', 'FatDetailsPopup', 1459), ('HEALTH', 'HealthDetailsPopUp', 2832), ('ENERGY_INFO', 'EnergyInfoPopUp', 2560), ('GROWTH', 'MaturityDetailsPopUp', 2115), ('AGE', 'AgeDetailsPopUp', 950)]


def find(o):
    if isinstance(o, dict):
        path = o.get('path', '')
        name = path.rsplit('/', 1)[-1]
        if 'BiologyPanel/' in path and name in [w[1] for w in WANTED] and name not in roots:
            roots[name] = o
        for v in o.values():
            find(v)
    elif isinstance(o, list):
        for v in o:
            find(v)


find(session)
GAME = 'C:/Users/Storme/TheBibites/The Bibites_Data'


def formats(go_id):
    """`FloatValueTextHandle` of every text below a pop-up (path under it -> prefix, suffix, precision, factor, flags), from the prefab's raw bytes."""
    env = dict(os.environ, PYTHONIOENCODING='utf-8')
    out = subprocess.run([sys.executable, 'tools/unity/dump_ui_components.py', GAME, 'level2', str(go_id)], capture_output=True, env=env).stdout.decode('utf-8')
    stack, result, counts = [], {}, {}
    for line in out.splitlines():
        m = re.match(r'^(\s*)GO \d+ (.*)$', line)
        if m:
            depth = len(m.group(1)) // 2
            stack = stack[:depth] + [m.group(2)]
            continue
        m = re.search(r"prefix '(.*)' suffix '(.*)' precision (-?\d+) factor ([-\d.e]+) SI/sign/precisionIsSI/isInt \[(\d), (\d), (\d), (\d)\]", line)
        if m and stack:
            rel = '/'.join(stack[1:])
            counts[rel] = counts.get(rel, 0) + 1
            if counts[rel] > 1:
                rel = '%s#%d' % (rel, counts[rel])
            result[rel] = (m.group(1), m.group(2), int(m.group(3)), float(m.group(4)), *[int(m.group(i)) for i in range(5, 9)])
    return result


def rs(text):
    return '"' + text.replace('\\', '\\\\').replace('"', '\\"').replace('\n', '\\n') + '"'


def flatten(root, fmts):
    nodes, key_of, parent_key, seen = [], {}, {}, {}

    def collect(o, parent):
        path = o['path']
        seen[path] = seen.get(path, 0) + 1
        key = path if seen[path] == 1 else '%s#%d' % (path, seen[path])
        key_of[id(o)] = key
        parent_key[key] = parent
        nodes.append(o)
        for c in o.get('children', []):
            collect(c, key)

    collect(root, None)
    by_key = {key_of[id(n)]: n for n in nodes}
    height = root['rect']['size_delta']['y']
    root_key = key_of[id(root)]

    def pivot_pos(key):
        x = y = 0.0
        while key != root_key:
            n = by_key[key]
            x += n['rect']['local_position']['x']
            y += n['rect']['local_position']['y']
            key = parent_key[key]
        return x, y

    rects = {}
    for key, n in by_key.items():
        r = n['rect']
        w, h = r['size_delta']['x'], r['size_delta']['y']
        pv = r['pivot']
        px, py = pivot_pos(key)
        rects[key] = [px - pv['x'] * w, height / 2 - (py + (1 - pv['y']) * h), w, h]
    # The fat reserves bar (a 20 high layout element, 1 under its title) was measured with no height: put it where the section's height says it is.
    for key in list(rects):
        if key.endswith('/Stored/fatReserves'):
            title = rects[parent_key[key] + '/TitleHolder']
            rects[key] = [title[0], title[1] + title[3] + 1.0, title[2], 20.0]
    for key in list(rects):
        if '/fatReserves/' in key:
            bar = rects[key.split('/fatReserves/')[0] + '/fatReserves']
            node = by_key[key]
            if node['name'] == 'holder' or any(c['type'].endswith('TextMeshProUGUI') for c in node.get('components', [])):
                rects[key][1] = bar[1]
                rects[key][3] = 20.0
    out = []
    bottom = 0.0
    for key, n in by_key.items():
        x, y, w, h = rects[key]
        parent = parent_key[key]
        comps = n.get('components', [])
        image = next((c for c in comps if c['type'].endswith('.Image')), None)
        text = next((c for c in comps if c['type'].endswith('TextMeshProUGUI')), None)
        if text and w == 0:
            x, w = x - 32.5, 65.0
        if image and parent is not None and w == 0 and h == 0:
            x, y, w, h = rects[parent]
        if image and parent is not None and h == 0 and w != 0:
            _, y, _, h = rects[parent]
        rel = key.split(root['name'], 1)[1].lstrip('/') if key != root_key else ''
        kind = 'Kind::Group'
        if text:
            c = text['color']
            align = {'Left': -1, 'Center': 0, 'Right': 1}.get(text['alignment'], -1)
            spec = fmts.get(rel)
            fmt = 'None' if spec is None else 'Some(Fmt { prefix: %s, suffix: %s, precision: %d, factor: %s, si: %s, sign: %s, precision_is_si: %s, int: %s })' % (
                rs(spec[0]), rs(spec[1]), spec[2], repr(spec[3]), *['true' if v else 'false' for v in spec[4:8]])
            kind = 'Kind::Text { text: %s, font: %s, px: %s, align: %d, color: [%d, %d, %d], bold: %s, fmt: %s }' % (
                rs(text['text']), rs(text['font']), repr(float(text['font_size'])), align,
                round(c['r'] * 255), round(c['g'] * 255), round(c['b'] * 255),
                'true' if text['font_style'] != 'Normal' else 'false', fmt)
        elif image:
            c = image['color']
            kind = 'Kind::Image { sprite: %s, tint: [%d, %d, %d, %d] }' % (
                rs(image.get('sprite') or ''), round(c['r'] * 255), round(c['g'] * 255), round(c['b'] * 255), round(c['a'] * 255))
        out.append('    Node { path: %s, x: %.2f, y: %.2f, w: %.2f, h: %.2f, shown: %s, kind: %s },' % (
            rs(rel), x, y, w, h, 'true' if n.get('active_self') else 'false', kind))
        if key != root_key:
            bottom = max(bottom, y + h)
    return root['rect']['size_delta']['x'], height, height - bottom, out


lines = [
    '//! Generated by tools/ui/gen_biology_popups.py from the native capture; do not edit.',
    '',
    '/// The `FloatValueTextHandle` of a text: how the number is written.',
    '#[derive(Clone, Copy, Debug)]',
    'pub struct Fmt {',
    "    pub prefix: &'static str,",
    "    pub suffix: &'static str,",
    '    pub precision: i32,',
    '    pub factor: f32,',
    '    pub si: bool,',
    '    pub sign: bool,',
    '    pub precision_is_si: bool,',
    '    pub int: bool,',
    '}',
    '',
    '/// What a node draws.',
    '#[derive(Clone, Copy, Debug)]',
    'pub enum Kind {',
    '    /// Only a place in the tree.',
    '    Group,',
    '    /// An `Image`.',
    '    Image { sprite: &\'static str, tint: [u8; 4] },',
    '    /// A TextMeshPro text.',
    '    Text { text: &\'static str, font: &\'static str, px: f32, align: i8, color: [u8; 3], bold: bool, fmt: Option<Fmt> },',
    '}',
    '',
    '/// One node of a pop-up: its path under the pop-up, its rectangle from the pop-up\'s top left, whether the prefab has it active.',
    '#[derive(Clone, Copy, Debug)]',
    'pub struct Node {',
    '    pub path: &\'static str,',
    '    pub x: f32,',
    '    pub y: f32,',
    '    pub w: f32,',
    '    pub h: f32,',
    '    pub shown: bool,',
    '    pub kind: Kind,',
    '}',
    '',
    '/// A pop-up: its size, the room under the last node, and its nodes in drawing order.',
    'pub struct Def {',
    '    pub w: f32,',
    '    pub h: f32,',
    '    pub pad_bottom: f32,',
    '    pub nodes: &\'static [Node],',
    '}',
    '',
]
for const, name, go in WANTED:
    w, h, pad, nodes = flatten(roots[name], formats(go))
    lines.append('pub const %s: Def = Def {' % const)
    lines.append('    w: %.2f,' % w)
    lines.append('    h: %.2f,' % h)
    lines.append('    pad_bottom: %.2f,' % pad)
    lines.append('    nodes: &[')
    lines.extend(nodes)
    lines.append('    ],')
    lines.append('};')
    lines.append('')
open('crates/bibites-view/src/biology_popups_data.rs', 'w', encoding='utf8', newline='\n').write('\n'.join(lines))
print('ok')

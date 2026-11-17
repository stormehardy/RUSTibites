# Prints the active nodes of a /d/ui dump under a root node name with their screen boxes (top-left origin), sprites, fonts and texts: print_ui_tree.py dump.json DynamicSettingsPanel 24
import json, sys
path = sys.argv[1]
root_filter = sys.argv[2] if len(sys.argv) > 2 else 'DynamicSettingsPanel'
maxdepth = int(sys.argv[3]) if len(sys.argv) > 3 else 30
d = json.load(open(path, encoding='utf-8'))
H = d['screen']['height']


def compact(c):
    t = str(c.get('type'))
    short = t.split('.')[-1]
    if short == 'Image' or short == 'RawImage':
        col = c.get('color') or {}
        return 'Img[%s %s %s %s a=%.2f en=%s]' % (c.get('sprite'), c.get('image_type'), tuple(round(col.get(k, 0), 2) for k in 'rgb'), c.get('sprite_border') and '' or '', col.get('a', 1), c.get('enabled'))
    if 'TextMeshPro' in t:
        col = c.get('color') or {}
        return 'TMP[%r %s %s al=%s %s sp=%s]' % ((c.get('text') or '')[:40], c.get('font'), c.get('font_size'), c.get('alignment'), c.get('font_style'), c.get('character_spacing'))
    if short in ('Button', 'Toggle', 'Slider', 'ScrollRect', 'Scrollbar', 'Mask', 'RectMask2D', 'TMP_Dropdown', 'MyDropdown', 'TMP_InputField', 'InputField'):
        return short
    if short in ('VerticalLayoutGroup', 'HorizontalLayoutGroup', 'GridLayoutGroup', 'LayoutElement', 'ContentSizeFitter'):
        return short + '[' + ','.join('%s=%s' % (k, v) for k, v in c.items() if k not in ('type', 'enabled')) [:120] + ']'
    return short


def walk(n, depth, inside):
    p = n.get('path', '')
    name = n.get('name')
    now_inside = inside or p.endswith(root_filter) or ('/' + root_filter) in p and p.split('/')[-1] == root_filter
    if now_inside and depth <= maxdepth and n.get('active_in_hierarchy'):
        bb = n.get('screen_bbox')
        box = ''
        if bb:
            box = 'x%.1f y%.1f w%.1f h%.1f' % (bb['x0'], H - bb['y1'], bb['w'], bb['h'])
        comps = [compact(c) for c in n.get('components', []) if isinstance(c, dict)]
        print('%s%s  %s  %s' % ('  ' * depth, name, box, ' | '.join(comps)[:300]))
    for c in n.get('children', []) or []:
        walk(c, depth + 1, now_inside)


# children may be absent: the dump is a flat list of canvases with nested 'children'? print structure keys once
for cv in d['canvases']:
    walk(cv, 0, False)

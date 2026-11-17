import json, sys
d = json.load(open(sys.argv[1], encoding='utf-8'))
rows = []
def walk(x, path):
    p = path + '/' + x.get('name', '')
    if p.endswith('BodyPreview/Label'):
        for c in x.get('components', []):
            if 'TextMeshPro' in c.get('type', ''):
                rows.append((c.get('text'), c.get('font_size'), x.get('active_in_hierarchy'), c.get('text_info', {}).get('line0_length')))
    for c in x.get('children', []):
        walk(c, p)
for cv in d.get('canvases', []):
    walk(cv, '')
for r in sorted(rows, key=lambda r: -(r[1] or 0)):
    print(r)

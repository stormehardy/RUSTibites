import json, sys
d = json.load(open(sys.argv[1], encoding='utf-8'))
n = 0
def walk(x, path):
    global n
    p = path + '/' + x.get('name', '')
    if 'SpeciesMesh(Clone)' in x.get('name', '') and n < 6:
        n += 1
        r = x.get('rect', {})
        print(x.get('name'), x.get('active_self'), 'bbox', x.get('screen_bbox'), {k: r.get(k) for k in ('anchored_position', 'size_delta', 'local_scale')})
        print('   comps', [(c.get('type','').split('.')[-1], c.get('enabled')) for c in x.get('components', [])])
        for c in x.get('children', [])[:4]:
            print('   child', c.get('name'), c.get('active_self'), c.get('screen_bbox') and [round(v) for v in (c['screen_bbox']['x0'], c['screen_bbox']['y0'], c['screen_bbox']['x1'], c['screen_bbox']['y1'])], [(cc.get('type','').split('.')[-1], cc.get('sprite'), cc.get('text')) for cc in c.get('components', [])][:4])
    for c in x.get('children', []):
        walk(c, p)
for cv in d.get('canvases', []):
    walk(cv, '')

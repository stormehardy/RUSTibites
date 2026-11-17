import json, sys
d = json.load(open(sys.argv[1], encoding='utf-8'))
want = sys.argv[2]
def walk(n, path):
    p = path + '/' + n.get('name', '')
    if p.endswith(want):
        r = n.get('rect', {})
        print(p[-70:], 'active', n.get('active_self'), n.get('active_in_hierarchy'), 'bbox', n.get('screen_bbox') and [round(n['screen_bbox'][k], 1) for k in ('x0','y0','x1','y1')])
        print('   rect', {k: r.get(k) for k in ('anchor_min','anchor_max','anchored_position','size_delta','pivot','local_scale')})
        print('   comps', [c.get('type','').split('.')[-1] for c in n.get('components', [])])
        for c in n.get('children', [])[:2]:
            print('   child', c.get('name'), c.get('screen_bbox') and [round(c['screen_bbox'][k], 1) for k in ('x0','y0','x1','y1')], c.get('rect', {}).get('size_delta'), c.get('rect', {}).get('anchored_position'))
        return
    for c in n.get('children', []):
        walk(c, p)
for cv in d.get('canvases', []):
    walk(cv, '')

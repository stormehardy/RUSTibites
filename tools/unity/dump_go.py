"""Dump components of one GameObject (file, path_id) incl. children transforms."""
import sys, os, json
import UnityPy

root, fname, pid = sys.argv[1], sys.argv[2], int(sys.argv[3])
env = UnityPy.load(os.path.join(root, fname))
objs = {o.path_id: o for o in env.objects}
def comp_dump(o, depth=0):
    d = o.read_typetree()
    return d
go = objs[pid].read_typetree()
print('GameObject', go['m_Name'], 'layer', go.get('m_Layer'))
for c in go['m_Component']:
    p = c['component']['m_PathID']
    o = objs.get(p)
    if not o: print(' ext', c); continue
    t = o.type.name
    d = o.read_typetree()
    d.pop('m_GameObject', None)
    if t == 'MonoBehaviour':
        d = {k: v for k, v in d.items() if k not in ('m_Script',)}
    print(' ', t, json.dumps(d)[:1500])

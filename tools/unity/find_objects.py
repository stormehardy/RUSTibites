"""List Unity objects whose name matches a pattern (read-only; game install untouched)."""
import sys, re, glob, os
import UnityPy

root = sys.argv[1]
pat = re.compile(sys.argv[2], re.I)
for f in sorted(glob.glob(os.path.join(root, '*'))):
    base = os.path.basename(f)
    if not (base.startswith(('level', 'sharedassets', 'resources', 'globalgamemanagers')) and not base.endswith(('.resS', '.resource', '.json'))):
        continue
    try:
        env = UnityPy.load(f)
    except Exception as e:
        print(base, 'ERR', e); continue
    for o in env.objects:
        if o.type.name in ('GameObject',):
            try:
                n = o.read().m_Name
            except Exception:
                continue
            if pat.search(n):
                print(base, o.path_id, o.type.name, n)

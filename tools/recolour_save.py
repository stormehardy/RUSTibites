"""recolour_save.py <in.zip> <out.zip> <x> <y> <r> <g> <b>: copy of a save whose bibite nearest to (x, y) has its pigment genes ColorR, ColorG, ColorB set to (r, g, b) (0..1; the game's default bibite is 0.1, 0.1, 0.1 = light grey).

For checks of what the window does with a coloured body (tools/window_hairline_probe.ps1): the default world has only neutral bibites."""
import json
import sys
import zipfile

src, dst = sys.argv[1], sys.argv[2]
x, y = float(sys.argv[3]), float(sys.argv[4])
colour = [float(v) for v in sys.argv[5:8]]
zin = zipfile.ZipFile(src)
best = None
for name in zin.namelist():
    if not name.startswith('bibites/') or not name.endswith('.bb8'):
        continue
    doc = json.loads(zin.read(name).decode('utf-8-sig'))
    pos = doc.get('transform', {}).get('position')
    if not pos:
        continue
    d = (pos[0] - x) ** 2 + (pos[1] - y) ** 2
    if best is None or d < best[0]:
        best = (d, name, doc)
d, name, doc = best
print('bibite', name, 'at', doc['transform']['position'], 'distance', d ** 0.5)
genes = doc['genes']['genes']
print('colour genes before:', genes.get('ColorR'), genes.get('ColorG'), genes.get('ColorB'))
genes['ColorR'], genes['ColorG'], genes['ColorB'] = colour
with zipfile.ZipFile(dst, 'w', zipfile.ZIP_DEFLATED) as zout:
    for item in zin.infolist():
        data = zin.read(item.filename)
        if item.filename == name:
            data = json.dumps(doc, separators=(',', ':')).encode('utf-8')
        zout.writestr(item.filename, data)
print('wrote', dst)

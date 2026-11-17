import json, zipfile, sys, glob, os, copy
sys.path.insert(0, r'Z:\RUSTibites\tools\parity\bd_contacts')
from make_fixture import SRC, load, zero_brain

COLORS = [
    ('basic', (0.1, 0.1, 0.1)),
    ('black', (1.0, 1.0, 1.0)),
    ('nearblack', (0.9, 0.9, 0.9)),
    ('white', (0.0, 0.0, 0.0)),
    ('red', (1.0, 0.0, 0.0)),
    ('green', (0.0, 1.0, 0.0)),
    ('blue', (0.0, 0.0, 1.0)),
    ('grey', (0.5, 0.5, 0.5)),
    ('purple', (0.8, 0.2, 0.9)),
    ('darkred', (1.0, 0.5, 0.5)),
    ('blackish', (1.0, 1.0, 0.9)),
    ('pale', (0.3, 0.1, 0.1)),
]
SPACING = 40.0
COLS = 4


def build(name='colors'):
    files = sorted(glob.glob(SRC + '/bibites/*.bb8'))
    base = None
    for f in files:
        d = json.load(open(f, encoding='utf-8-sig'))
        if 3.0 < d['body']['d2Size'] < 3.5 and d['body'].get('fatReservesAmount', 0) < 5 and not d['body'].get('dead'):
            base = d
            break
    bibites = {}
    positions = []
    for i, (cname, (r, g, b)) in enumerate(COLORS):
        d = copy.deepcopy(base)
        d = zero_brain(d, 0.0)
        x = (i % COLS - (COLS - 1) / 2.0) * SPACING
        y = (i // COLS - 1) * SPACING + 2000.0
        x += 2000.0
        d['transform']['position'] = [x, y]
        d['transform']['rotation'] = 0.0
        d['rb2d'].update({'px': x, 'py': y, 'vx': 0.0, 'vy': 0.0, 'r': 0.0})
        d['body']['dead'] = False
        d['body']['id'] = 880000 + i
        # fully healthy, no energy trouble
        d['genes']['genes']['ColorR'] = r
        d['genes']['genes']['ColorG'] = g
        d['genes']['genes']['ColorB'] = b
        bibites[f'bibites/bibite_{i}.bb8'] = d
        positions.append((cname, x, y))
    pel = load('pellets.bb8scene')
    pel['pellets'] = [{'zone': 'free pellets', 'pellets': []}]
    scene = load('scene.bb8scene')
    scene['nPellets'] = 0
    scene['nBibites'] = len(bibites)
    zp = f'C:/Temp/bd/fix_{name}.zip'
    with zipfile.ZipFile(zp, 'w', zipfile.ZIP_DEFLATED) as z:
        for fn in ('settings.bb8settings', 'speciesData.json', 'vars.bb8scene', 'data.bin', 'img.png'):
            z.write(os.path.join(SRC, fn), fn)
        z.writestr('scene.bb8scene', json.dumps(scene))
        z.writestr('pellets.bb8scene', json.dumps(pel))
        ph = load('pheromones.bb8scene')
        for k, v in ph.items():
            if isinstance(v, list):
                ph[k] = []
        z.writestr('pheromones.bb8scene', json.dumps(ph))
        for k, v in bibites.items():
            z.writestr(k, json.dumps(v))
    json.dump(positions, open(f'C:/Temp/bd/{name}_positions.json', 'w'))
    return zp, positions


if __name__ == '__main__':
    print(build())

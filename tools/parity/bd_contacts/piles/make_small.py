import json, zipfile, sys, glob, os, copy, math
sys.path.insert(0, r'Z:\RUSTibites\tools\parity\bd_contacts')
sys.path.insert(0, r'C:\Temp\bd')
from make_fixture import SRC, load, zero_brain


def custom(name, positions, amount=20.0, material='Plant', accel=1.0):
    files = sorted(glob.glob(SRC + '/bibites/*.bb8'))
    for f in files:
        d = json.load(open(f, encoding='utf-8-sig'))
        if 3.0 < d['body']['d2Size'] < 3.5 and d['body'].get('fatReservesAmount', 0) < 5 and not d['body'].get('dead'):
            break
    d = zero_brain(d, accel)
    d['transform']['position'] = [0.0, 0.0]; d['transform']['rotation'] = 0.0
    d['rb2d'].update({'px': 0.0, 'py': 0.0, 'vx': 0.0, 'vy': 0.0, 'r': 0.0}); d['body']['dead'] = False
    pel = load('pellets.bb8scene'); proto = pel['pellets'][0]['pellets'][0]; plist = []
    for (x, y) in positions:
        p = copy.deepcopy(proto)
        p['transform'] = {'position': [x, y], 'rotation': 0.0, 'scale': 1.0}
        p['rb2d'] = {'px': x, 'py': y, 'vx': 0.0, 'vy': 0.0, 'r': 0.0}
        p['pellet'] = {'material': material, 'amount': float(amount)}
        p['matterDecay'] = {'timeAlive': 0.0, 'rotAmount': 0.0}
        plist.append(p)
    pel['pellets'] = [{'zone': 'free pellets', 'pellets': plist}]
    scene = load('scene.bb8scene'); scene['nPellets'] = len(plist); scene['nBibites'] = 1
    zp = f'C:/Temp/bd/fix_{name}.zip'
    with zipfile.ZipFile(zp, 'w', zipfile.ZIP_DEFLATED) as z:
        for fn in ('settings.bb8settings', 'speciesData.json', 'vars.bb8scene', 'data.bin', 'img.png'):
            z.write(os.path.join(SRC, fn), fn)
        z.writestr('scene.bb8scene', json.dumps(scene)); z.writestr('pellets.bb8scene', json.dumps(pel))
        ph = load('pheromones.bb8scene')
        for k, v in ph.items():
            if isinstance(v, list): ph[k] = []
        z.writestr('pheromones.bb8scene', json.dumps(ph)); z.writestr('bibites/bibite_0.bb8', json.dumps(d))


if __name__ == '__main__':
    custom('chain1', [(0, 40)])
    custom('chain2', [(0, 40), (0, 45)])
    custom('chain3', [(0, 40), (0, 45), (0, 50)])
    custom('side2', [(-2.5, 40), (2.5, 40)])
    custom('row3', [(-5, 40), (0, 40), (5, 40)])
    custom('tri', [(-2.5, 40), (2.5, 40), (0, 44.33)])

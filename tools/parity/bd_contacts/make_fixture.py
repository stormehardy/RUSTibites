import json, zipfile, sys, glob, os, copy

SRC = 'C:/Temp/bd/sv'
ACC = 33


def load(p):
    return json.load(open(os.path.join(SRC, p), encoding='utf-8-sig'))


def zero_brain(d, accel):
    for node in d['brain']['Nodes']:
        idx = int(node['Index'])
        if idx >= 33:
            node['Type'] = 2
            v = accel if idx == ACC else 0.0
            node['baseActivation'] = v
            node['Value'] = v
            node['LastInput'] = 0.0
            node['LastOutput'] = v
    d['brain']['Synapses'] = []
    return d


def make(name, accel, pellet=None, other_bibite=None, gap=40.0, speed0=0.0, dx=0.0, material='Plant'):
    """One bibite at the origin facing +y (rotation 0) with a constant Accelerate, optionally a pellet or a second (still) bibite `gap` in front."""
    files = sorted(glob.glob(SRC + '/bibites/*.bb8'))
    # a mid-size adult
    best = None
    for f in files:
        d = json.load(open(f, encoding='utf-8-sig'))
        if 3.0 < d['body']['d2Size'] < 3.5 and d['body'].get('fatReservesAmount', 0) < 5 and not d['body'].get('dead'):
            best = (f, d)
            break
    f, d = best
    d = zero_brain(d, accel)
    d['transform']['position'] = [0.0, 0.0]
    d['transform']['rotation'] = 0.0
    d['rb2d'].update({'px': 0.0, 'py': 0.0, 'vx': 0.0, 'vy': speed0, 'r': 0.0})
    d['body']['dead'] = False
    out = {}
    bibites = {'bibites/bibite_0.bb8': d}
    nb = 1
    if other_bibite:
        e = copy.deepcopy(d)
        e['body']['id'] = 77777777
        e = zero_brain(e, 0.0)
        e['transform']['position'] = [dx, gap]
        e['rb2d'].update({'px': dx, 'py': gap, 'vx': 0.0, 'vy': 0.0, 'r': 0.0})
        bibites['bibites/bibite_1.bb8'] = e
        nb = 2
    pel = load('pellets.bb8scene')
    proto = pel['pellets'][0]['pellets'][0]
    plist = []
    if pellet:
        p = copy.deepcopy(proto)
        p['transform'] = {'position': [dx, gap], 'rotation': 0.0, 'scale': 1.0}
        p['rb2d'] = {'px': dx, 'py': gap, 'vx': 0.0, 'vy': 0.0, 'r': 0.0}
        p['pellet'] = {'material': material, 'amount': float(pellet)}
        p['matterDecay'] = {'timeAlive': 0.0, 'rotAmount': 0.0}
        plist.append(p)
    pel['pellets'] = [{'zone': 'free pellets', 'pellets': plist}]
    scene = load('scene.bb8scene')
    scene['nPellets'] = len(plist)
    scene['nBibites'] = nb
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
    return zp


if __name__ == '__main__':
    print(make('alone', 1.0))
    for amt in (10, 100, 400):
        print(make(f'pellet{amt}', 1.0, pellet=amt))
    print(make('bibite', 1.0, other_bibite=True))
    print(make('meat400', 1.0, pellet=400, material='Meat'))
    print(make('meat1500', 1.0, pellet=1500, material='Meat'))
    print(make('pellet400off', 1.0, pellet=400, dx=5.0))
    print(make('pellet100off', 1.0, pellet=100, dx=-5.0))
    print(make('bibiteoff', 1.0, other_bibite=True, dx=6.0))

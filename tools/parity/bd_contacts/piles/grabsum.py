import json, re, sys
G = json.load(open('C:/Temp/bd/game_grab_out.json'))
print('fixture | at tick 30/60/89: bibite y diff (rust-game), pellet y diff, pellet x diff')
for name in G:
    rv = {}; pv = {}
    for line in open(f'C:/Temp/bd/rust_pile_{name}.txt'):
        if line.startswith('TR '):
            t = int(line.split()[1]); m = re.search(r'P=\(([-\d.]+),([-\d.]+)\)', line); rv[t] = tuple(map(float, m.groups()))
        if line.startswith('PL '):
            t = int(line.split()[1]); pv[t] = [tuple(map(float, p)) for p in re.findall(r'\(([-\d.]+),([-\d.]+),v', line)][0]
    out = []
    for t in (30, 60, 89):
        g = G[name][t]; b = g['bibites'][0]; p = g['pellets'][0]
        out.append('%+.2f/%+.2f/%+.2f' % (rv[t+1][1] - b['y'], pv[t+1][1] - p[1], pv[t+1][0] - p[0]))
    print('%-12s' % name, '  '.join(out))

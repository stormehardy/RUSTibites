import json, re, sys, os
name = sys.argv[1]; G = json.load(open('C:/Temp/bd/game_grab_out.json'))[name]
rv = {}; pv = {}
for line in open(f'C:/Temp/bd/rust_pile_{name}.txt'):
    if line.startswith('TR '):
        t = int(line.split()[1]); m = re.search(r'P=\(([-\d.]+),([-\d.]+)\) rot=([-\d.]+) v=\(([-\d.]+),([-\d.]+)\)', line); rv[t] = tuple(map(float, m.groups()))
    if line.startswith('PL '):
        t = int(line.split()[1]); pv[t] = [tuple(map(float, p)) for p in re.findall(r'\(([-\d.]+),([-\d.]+),v([-\d.]+),([-\d.]+),h(\d)', line)][0]
print('t | game bib y, pellet y, rel | rust(t+1) bib y, pellet y, rel, held')
for r in G:
    t = r['tick']
    if t < int(sys.argv[2]) or t > int(sys.argv[3]) or t + 1 not in rv: continue
    b = r['bibites'][0]['y']; p = r['pellets'][0][1]; a = rv[t+1][1]; q = pv[t+1]
    print('%2d | %7.3f %7.3f %7.3f | %7.3f %7.3f %7.3f %d' % (t, b, p, p - b, a, q[1], q[1] - a, int(q[4])))

import json, re, sys, os
name = sys.argv[1]; G = json.load(open('C:/Temp/bd/' + os.environ['GF']))[name]
rv = {}; pv = {}
for line in open(f'C:/Temp/bd/rust_pile_{name}.txt'):
    if line.startswith('TR '):
        t = int(line.split()[1]); m = re.search(r'P=\(([-\d.]+),([-\d.]+)\) rot=([-\d.]+) v=\(([-\d.]+),([-\d.]+)\) w=([-\d.]+)', line); rv[t] = tuple(map(float, m.groups()))
    if line.startswith('PL '):
        t = int(line.split()[1]); pv[t] = [tuple(map(float, p)) for p in re.findall(r'\(([-\d.]+),([-\d.]+),v([-\d.]+),([-\d.]+),h', line)][0]
print('t | game bibite x y rot vx vy | rust(t+1) | game pellet x y | rust pellet x y')
for r in G:
    t = r['tick']
    if t < int(sys.argv[2]) or t > int(sys.argv[3]): continue
    b = r['bibites'][0]; a = rv[t+1]; p = r['pellets'][0]; q = pv[t+1]
    print('%2d | %7.3f %7.3f %7.2f %6.2f %6.2f | %7.3f %7.3f %7.2f %6.2f %6.2f | %7.3f %7.3f | %7.3f %7.3f' % (t, b['x'], b['y'], b['rot'], b['vx'], b['vy'], a[0], a[1], a[2], a[3], a[4], p[0], p[1], q[0], q[1]))

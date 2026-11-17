import json, re, sys, os
G = json.load(open('C:/Temp/bd/' + os.environ['GF']))[sys.argv[1]]
rv = {}
for line in open(f'C:/Temp/bd/rust_pile_{sys.argv[1]}.txt'):
    if line.startswith('TR '):
        t = int(line.split()[1]); m = re.search(r'P=\(([-\d.]+),([-\d.]+)\) rot=([-\d.]+) v=\(([-\d.]+),([-\d.]+)\)', line); rv[t] = tuple(map(float, m.groups()))
print('t | game y vy | rust(t+1) y vy | rust(t) y vy')
for r in G:
    t = r['tick']
    if 24 <= t <= 44:
        b = r['bibites'][0]; a = rv[t+1]; c = rv[t]
        print('%2d | %7.3f %7.3f | %7.3f %7.3f | %7.3f %7.3f' % (t, b['y'], b['vy'], a[1], a[4], c[1], c[4]))

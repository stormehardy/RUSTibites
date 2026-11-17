import os, sys
os.environ['FIXSET'] = 'jit'
sys.path.insert(0, r'C:\Temp\bd')
import fit
env = dict(kv.split('=') for kv in sys.argv[1:])
res = {nm: fit.run_rust(nm, env)[0] for nm in fit.FIX}
print('t | mean(rust-game) y | vy   | per fixture vy diff')
for t in range(22, 48):
    dy = []; dv = []
    for nm, rows in res.items():
        g = fit.GAME[nm][t]['bibites'][0]; r = rows[t + 1]
        dy.append(r[1] - g['y']); dv.append(r[4] - g['vy'])
    print('%2d | %+.3f | %+.3f | ' % (t, sum(dy) / len(dy), sum(dv) / len(dv)) + ' '.join('%+5.1f' % x for x in dv))

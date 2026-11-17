import os, sys, json
os.environ['FIXSET'] = 'jit'
sys.path.insert(0, r'C:\Temp\bd')
import fit
env = dict(kv.split('=') for kv in sys.argv[1:])
tot = []
for nm in fit.FIX:
    rows, pl = fit.run_rust(nm, env)
    g = fit.GAME[nm]
    d = [rows[t + 1][1] - g[t]['bibites'][0]['y'] for t in (40, 60, 76)]
    dv = [rows[t + 1][4] - g[t]['bibites'][0]['vy'] for t in (40, 60, 76)]
    print(nm, 'rust-game y at 40/60/76: %+.2f %+.2f %+.2f  vy: %+.2f %+.2f %+.2f' % (*d, *dv))
    tot.append(d)
print('mean y diff at 40/60/76: ' + ' '.join('%+.2f' % (sum(x[i] for x in tot) / len(tot)) for i in range(3)))

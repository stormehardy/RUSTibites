import json, os, re, subprocess, sys, math
from concurrent.futures import ThreadPoolExecutor

EXE = r'Z:\RUSTibites\target_light\release\deps\negative_health_probe-ed4534927864b741.exe'
FIX = {  # name: (game file, n original pellets)
    'chain1': ('game_small_out.json', 1), 'chain2': ('game_small_out.json', 2), 'chain3': ('game_small_out.json', 3),
    'side2': ('game_small_out.json', 2), 'row3': ('game_small_out.json', 3), 'tri': ('game_small_out.json', 3),
    'rows1': ('game_rows_out.json', 6), 'rows2': ('game_rows_out.json', 11), 'rows3': ('game_rows_out.json', 17),
    'rows4': ('game_rows_out.json', 22), 'pile30': ('game_pile30_out.json', 28), 'pile12': ('game_pile30_out.json', 11),
    'pilemeat': ('game_pile30_out.json', 28),
}
if os.environ.get('FIXSET') == 'jit':
    FIX = {k: ('game_jit_out.json', v) for k, v in json.load(open('C:/Temp/bd/jit_n.json')).items()}
GAME = {}
for f in set(v[0] for v in FIX.values()):
    GAME.update(json.load(open('C:/Temp/bd/' + f)))


def run_rust(name, env):
    e = dict(os.environ); e.update(env)
    e['RUSTIBITES_PROBE_SAVE'] = f'C:/Temp/bd/fix_{name}.zip'; e['RUSTIBITES_PROBE_TICKS'] = '81'
    out = subprocess.run([EXE, 'track_probe', '--ignored', '--nocapture', '--test-threads=1'], env=e, capture_output=True, text=True).stderr
    rows = {}; pl = {}
    for line in out.splitlines():
        if line.startswith('TR '):
            t = int(line.split()[1]); m = re.search(r'P=\(([-\d.]+),([-\d.]+)\) rot=([-\d.]+) v=\(([-\d.]+),([-\d.]+)\)', line)
            rows[t] = tuple(map(float, m.groups()))
        elif line.startswith('PL '):
            t = int(line.split()[1])
            pl[t] = [tuple(map(float, p)) for p in re.findall(r'\(([-\d.]+),([-\d.]+),v([-\d.]+),([-\d.]+),h', line)]
    return rows, pl


def track(frames, first, n):
    cur = list(first[:n]); out = [list(cur)]
    for fr in frames:
        nxt = []
        for p in cur:
            q = min(fr, key=lambda q: (q[0] - p[0]) ** 2 + (q[1] - p[1]) ** 2)
            nxt.append((q[0], q[1]))
        cur = nxt; out.append(list(cur))
    return out


def score(name, env, SH=1):
    rows, pl = run_rust(name, env)
    g = GAME[name]; n = FIX[name][1]
    T = len(g) - 1
    gframes = [[(p[0], p[1]) for p in r['pellets']] for r in g][:T]
    rframes = [[(p[0], p[1]) for p in pl[t + SH]] for t in range(T)]
    gt = track(gframes[1:], gframes[0], n); rt = track(rframes[1:], rframes[0], n)
    pair = [min(range(len(rt[0])), key=lambda k: (rt[0][k][0] - s[0]) ** 2 + (rt[0][k][1] - s[1]) ** 2) for s in gt[0]]
    dy = []; dv = []; dp = []
    for t in range(26, T):
        b = g[t]['bibites'][0]; rr = rows[t + SH]
        dy.append(abs(b['y'] - rr[1])); dv.append(abs(b['vy'] - rr[4]))
        dp.append(sum(math.hypot(gt[t][i][0] - rt[t][pair[i]][0], gt[t][i][1] - rt[t][pair[i]][1]) for i in range(n)) / n)
    m = lambda a: sum(a) / len(a)
    return m(dy), m(dv), m(dp)


def total(env, verbose=False):
    with ThreadPoolExecutor(6) as ex:
        res = dict(zip(FIX, ex.map(lambda nm: score(nm, env), FIX)))
    tot = [0, 0, 0]
    for nm, (a, b, c) in res.items():
        if verbose:
            print('  %-8s dy %.3f dvy %.3f dpellet %.3f' % (nm, a, b, c))
        tot[0] += a; tot[1] += b; tot[2] += c
    return tot


if __name__ == '__main__':
    env = dict(kv.split('=') for kv in sys.argv[1:])
    tot = total(env, verbose=True)
    print('TOTAL dy %.3f dvy %.3f dpellet %.3f | combined %.3f' % (tot[0], tot[1], tot[2], tot[0] + tot[1] * 0.2 + tot[2]))

import json, re, sys, math
import os
G = json.load(open('C:/Temp/bd/' + os.environ.get('GF', 'game_pile_out.json')))
name = sys.argv[1]; SH = int(sys.argv[2]); N0 = int(sys.argv[3])
rows = {}; pl = {}
for line in open(f'C:/Temp/bd/rust_pile_{name}.txt'):
    if line.startswith('TR '):
        t = int(line.split()[1])
        ms = re.findall(r'P=\(([-\d.]+),([-\d.]+)\) rot=([-\d.]+) v=\(([-\d.]+),([-\d.]+)\)', line)
        rows[t] = [tuple(map(float, m)) for m in ms]
    if line.startswith('PL '):
        t = int(line.split()[1])
        pl[t] = [tuple(map(float, p)) for p in re.findall(r'\(([-\d.]+),([-\d.]+),v([-\d.]+),([-\d.]+),h', line)]
def track(frames, first):
    cur = list(first[:N0]); out = [list(cur)]
    for fr in frames:
        nxt = []
        for p in cur:
            q = min(fr, key=lambda q: (q[0]-p[0])**2 + (q[1]-p[1])**2)
            nxt.append((q[0], q[1]))
        cur = nxt; out.append(list(cur))
    return out
gframes = [[(p[0], p[1]) for p in r['pellets']] for r in G[name]]
rframes = [[(p[0], p[1]) for p in pl[t + SH]] for t in range(len(gframes) - 1)]
gframes = gframes[:-1]
# start positions: the first N0 pellets in game order <-> nearest in rust
gt = track(gframes[1:], gframes[0])
rt = track(rframes[1:], rframes[0])
print('tick | bibite y g/r | vy g/r | mean pellet disp g/r | mean y of pile g/r | max pos err | mean speed of pile g/r')
for t in range(len(gt)):
    if t % 4 and t > 3: continue
    b = G[name][t]['bibites'][0]; rr = rows[t + SH]
    gd = [math.hypot(a[0]-s[0], a[1]-s[1]) for a, s in zip(gt[t], gt[0])]
    rd = [math.hypot(a[0]-s[0], a[1]-s[1]) for a, s in zip(rt[t], rt[0])]
    # nearest-in-rust to each game pellet start, to pair them
    pair = [min(range(len(rt[0])), key=lambda k: (rt[0][k][0]-s[0])**2 + (rt[0][k][1]-s[1])**2) for s in gt[0]]
    err = [math.hypot(gt[t][i][0]-rt[t][pair[i]][0], gt[t][i][1]-rt[t][pair[i]][1]) for i in range(len(gt[t]))]
    gs = sum(math.hypot(gt[t][i][0]-gt[t-1][i][0], gt[t][i][1]-gt[t-1][i][1]) for i in range(len(gt[t])))/len(gt[t])/0.05 if t else 0
    rs = sum(math.hypot(rt[t][i][0]-rt[t-1][i][0], rt[t][i][1]-rt[t-1][i][1]) for i in range(len(rt[t])))/len(rt[t])/0.05 if t else 0
    print('%3d | %7.2f %7.2f | %6.2f %6.2f | %6.2f %6.2f | %7.2f %7.2f | %6.2f | %6.2f %6.2f' % (t, b['y'], rr[0][1], b['vy'], rr[0][4], sum(gd)/len(gd), sum(rd)/len(rd), sum(p[1] for p in gt[t])/len(gt[t]), sum(p[1] for p in rt[t])/len(rt[t]), max(err), gs, rs))

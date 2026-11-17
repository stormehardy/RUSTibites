import sys, math, statistics as st
sys.path.insert(0, r'Z:\RUSTibites\tools\parity')
import trace_compare as tc
gb, G = tc.load_game('trace/game_trace.jsonl')
R = tc.load_rust(sys.argv[1] + '/trace.jsonl')


def second_diffs(tr, key):
    by = {}
    for (t, i), v in tr.items():
        by.setdefault(i, {})[t] = v
    out = []
    for i, rows in by.items():
        for t in range(3, 70):
            if t - 1 in rows and t in rows and t + 1 in rows:
                if key == 'rot':
                    a, b, c = rows[t - 1]['rot'], rows[t]['rot'], rows[t + 1]['rot']
                    d = lambda x, y: (x - y + 180.0) % 360.0 - 180.0
                    out.append(abs(d(c, b) - d(b, a)))
                elif key == 'angvel':
                    out.append(abs(rows[t + 1]['angvel'] - rows[t]['angvel']))
                else:
                    out.append(math.hypot(rows[t + 1]['vx'] - rows[t]['vx'], rows[t + 1]['vy'] - rows[t]['vy']))
    return out


for key in ('rot', 'angvel', 'vel'):
    g = second_diffs(G, key)
    r = second_diffs(R, key)
    q = lambda a, p: sorted(a)[int(p * (len(a) - 1))]
    print('%-7s game n %d median %.4f p90 %.4f p99 %.4f | port n %d median %.4f p90 %.4f p99 %.4f' % (key, len(g), st.median(g), q(g, .9), q(g, .99), len(r), st.median(r), q(r, .9), q(r, .99)))

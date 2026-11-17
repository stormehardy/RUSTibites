import re, sys
def load(p):
    out = {}
    for line in open(p, encoding='utf-8-sig', errors='replace'):
        m = re.match(r'^(\S+) : mean ([\d.]+)', line)
        if m:
            out[m.group(1)] = float(m.group(2))
    return out
a, b = load(sys.argv[1]), load(sys.argv[2])
tot_a = tot_b = 0
for k in a:
    if k in b:
        tot_a += a[k]; tot_b += b[k]
        d = b[k] - a[k]
        if abs(d) > 0.0005:
            print('%-34s %7.3f -> %7.3f  %+.3f' % (k, a[k], b[k], d))
print('sum of means: %.3f -> %.3f over %d sessions' % (tot_a, tot_b, len(a)))

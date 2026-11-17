import sys, random, json, zipfile, os, copy, math, glob
sys.path.insert(0, r'C:\Temp\bd'); sys.path.insert(0, r'Z:\RUSTibites\tools\parity\bd_contacts')
import make_small
def jit(name, rows, per_row, seed, amp=0.06, spacing=5.0, y0=40.0, amount=20.0):
    rnd = random.Random(seed); pos = []
    for r in range(rows):
        n = per_row - (r % 2)
        for c in range(n):
            x = (c - (n - 1) / 2.0) * spacing + rnd.uniform(-amp, amp)
            y = y0 + r * spacing * math.sqrt(3) / 2.0 + rnd.uniform(-amp, amp)
            pos.append((x, y))
    make_small.custom(name, pos, amount=amount)
    return len(pos)
specs = [('jit1', 4, 6, 1), ('jit2', 4, 6, 2), ('jit3', 5, 6, 3), ('jit4', 5, 6, 4), ('jit5', 3, 5, 5), ('jit6', 6, 5, 6)]
ns = {}
for nm, r, pr, sd in specs:
    ns[nm] = jit(nm, r, pr, sd)
json.dump(ns, open('C:/Temp/bd/jit_n.json', 'w'))
print(ns)

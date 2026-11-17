import sys
import numpy as np
from PIL import Image
a = np.asarray(Image.open(sys.argv[1]).convert('RGB'), dtype=np.float32)   # game
b = np.asarray(Image.open(sys.argv[2]).convert('RGB'), dtype=np.float32)   # port
x0, y0, x1, y1 = [int(v) for v in sys.argv[3].split(',')]
d = np.abs(a - b).mean(axis=2)[y0:y1, x0:x1]
print('mean diff %.3f  >40: %.2f%%' % (d.mean(), (d > 40).mean() * 100))
best = None
for dy in range(-6, 7):
    for dx in range(-4, 5):
        aa = a[y0 + dy:y1 + dy, x0 + dx:x1 + dx]
        dd = np.abs(aa - b[y0:y1, x0:x1]).mean()
        if best is None or dd < best[0]:
            best = (dd, dx, dy)
print('best shift of game vs port: dx %d dy %d -> %.3f' % (best[1], best[2], best[0]))
if len(sys.argv) > 4:
    big = (np.abs(a - b).mean(axis=2) > 40)
    ys, xs = np.where(big[y0:y1, x0:x1])
    Image.fromarray((big[y0:y1, x0:x1] * 255).astype(np.uint8)).save(sys.argv[4])

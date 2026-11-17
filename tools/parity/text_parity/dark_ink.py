import sys
import numpy as np
from PIL import Image
a = np.asarray(Image.open(sys.argv[1]).convert('L'), dtype=np.float32)   # ref
b = np.asarray(Image.open(sys.argv[2]).convert('L'), dtype=np.float32)   # render
bg = float(sys.argv[3])
for spec in sys.argv[4:]:
    x0, y0, x1, y1 = [int(v) for v in spec.split(',')]
    ra, rb = a[y0:y1, x0:x1], b[y0:y1, x0:x1]
    ia, ib = (bg - ra).clip(0).sum() / bg, (bg - rb).clip(0).sum() / bg
    ys, xs = np.mgrid[y0:y1, x0:x1]
    wa, wb = (bg - ra).clip(0), (bg - rb).clip(0)
    cxa = (xs * wa).sum() / max(wa.sum(), 1); cxb = (xs * wb).sum() / max(wb.sum(), 1)
    cya = (ys * wa).sum() / max(wa.sum(), 1); cyb = (ys * wb).sum() / max(wb.sum(), 1)
    # extent: columns with ink
    ca = np.where(wa.sum(axis=0) > 20)[0]; cb = np.where(wb.sum(axis=0) > 20)[0]
    print('%-16s ink ref %7.1f port %7.1f ratio %.3f  dx %+.2f dy %+.2f  x-extent ref %d..%d port %d..%d' % (
        spec, ia, ib, ib / max(ia, 1e-6), cxb - cxa, cyb - cya, x0 + ca.min(), x0 + ca.max(), x0 + cb.min(), x0 + cb.max()))

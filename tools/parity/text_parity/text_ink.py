"""Per-text-block ink comparison between the real game's UI-on-black reference and the port's render.

usage: text_ink.py <render.png> <reference.png> [min_ink]
Blocks = connected groups of lit pixels in the reference (after a horizontal dilation joining the characters of a line),
reported with the ink (sum of luminance / 255), the ink-weighted centroid and the bbox in both pictures.
"""
import sys
import numpy as np
from PIL import Image
from scipy import ndimage

ren = np.asarray(Image.open(sys.argv[1]).convert('RGB'), dtype=np.float32)
ref = np.asarray(Image.open(sys.argv[2]).convert('RGB'), dtype=np.float32)
min_ink = float(sys.argv[3]) if len(sys.argv) > 3 else 6.0
lr = ref.mean(axis=2) / 255.0
lp = ren.mean(axis=2) / 255.0
mask = lr > 0.12
mask = ndimage.binary_dilation(mask, structure=np.ones((3, 9)))
lab, n = ndimage.label(mask)
rows = []
for i, sl in enumerate(ndimage.find_objects(lab), start=1):
    sub = lab[sl] == i
    a = lr[sl] * sub
    b = lp[sl] * sub
    ia, ib = a.sum(), b.sum()
    if ia < min_ink:
        continue
    ys, xs = np.mgrid[sl[0], sl[1]]
    ca = ((xs * a).sum() / ia, (ys * a).sum() / ia)
    cb = ((xs * b).sum() / max(ib, 1e-6), (ys * b).sum() / max(ib, 1e-6))
    h = sl[0].stop - sl[0].start
    w = sl[1].stop - sl[1].start
    rows.append((sl[1].start, sl[0].start, w, h, ia, ib, cb[0] - ca[0], cb[1] - ca[1]))
tot_a = sum(r[4] for r in rows)
tot_b = sum(r[5] for r in rows)
print('blocks %d  ink ref %.1f port %.1f ratio %.3f' % (len(rows), tot_a, tot_b, tot_b / max(tot_a, 1e-6)))
for r in sorted(rows, key=lambda r: -r[4])[:12]:
    print('  x%4d y%4d %4dx%3d  ink ref %7.1f port %7.1f ratio %.3f  dx %+.2f dy %+.2f' % (r[0], r[1], r[2], r[3], r[4], r[5], r[5] / r[4], r[6], r[7]))

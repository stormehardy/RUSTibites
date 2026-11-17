"""Single-line text blocks (height <= 60) whose ink or position differs from the reference, over every sweep render."""
import sys, glob, os
import numpy as np
from PIL import Image
from scipy import ndimage

tag = sys.argv[1]
ref_dir = r'Z:\RUSTibites\assets\game\reference\functional_ui_native\menu_ui'
rows_all = []
for p in sorted(glob.glob(r'C:\Temp\sweep_%s\*.png' % tag)):
    name = os.path.basename(p)[:-4]
    ref_p = os.path.join(ref_dir, name + '_ui_on_black_1920x1080.png')
    if not os.path.exists(ref_p):
        continue
    lr = np.asarray(Image.open(ref_p).convert('RGB'), dtype=np.float32).mean(axis=2) / 255.0
    lp = np.asarray(Image.open(p).convert('RGB'), dtype=np.float32).mean(axis=2) / 255.0
    mask = ndimage.binary_dilation(lr > 0.25, structure=np.ones((3, 9)))
    lab, n = ndimage.label(mask)
    for i, sl in enumerate(ndimage.find_objects(lab), start=1):
        h = sl[0].stop - sl[0].start
        w = sl[1].stop - sl[1].start
        if h > 60 or w < 15 or h < 8:
            continue
        sub = lab[sl] == i
        a = lr[sl] * sub
        b = lp[sl] * sub
        ia, ib = a.sum(), b.sum()
        if ia < 60:
            continue
        # skip blocks that are mostly flat fills (panels, buttons): text has many edge pixels
        if (a[sub] > 0.9).mean() > 0.6:
            continue
        ys, xs = np.mgrid[sl[0], sl[1]]
        dx = (xs * b).sum() / max(ib, 1e-6) - (xs * a).sum() / ia
        dy = (ys * b).sum() / max(ib, 1e-6) - (ys * a).sum() / ia
        rows_all.append((name, sl[1].start, sl[0].start, w, h, ia, ib / ia, dx, dy))
bad = [r for r in rows_all if abs(r[6] - 1) > 0.15 or abs(r[7]) > 1.5 or abs(r[8]) > 1.5]
print('single-line blocks %d, outliers %d' % (len(rows_all), len(bad)))
for r in sorted(bad, key=lambda r: -r[5])[:int(sys.argv[2]) if len(sys.argv) > 2 else 40]:
    print('%-30s x%4d y%4d %4dx%3d ink %7.1f ratio %.2f dx %+5.2f dy %+5.2f' % r)
ratios = np.array([r[6] for r in rows_all])
print('ink ratio: median %.3f  p10 %.3f  p90 %.3f' % (np.median(ratios), np.percentile(ratios, 10), np.percentile(ratios, 90)))

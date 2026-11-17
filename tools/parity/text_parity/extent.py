import sys
import numpy as np
from PIL import Image
ref = np.asarray(Image.open(sys.argv[1]).convert('RGB'), dtype=np.float32).mean(axis=2)
ren = np.asarray(Image.open(sys.argv[2]).convert('RGB'), dtype=np.float32).mean(axis=2)
for spec in sys.argv[3:]:
    x0, y0, x1, y1 = [int(v) for v in spec.split(',')]
    out = []
    for im in (ref, ren):
        sub = im[y0:y1, x0:x1]
        cols = np.where(sub.max(axis=0) > 128)[0]
        rows = np.where(sub.max(axis=1) > 128)[0]
        # subpixel edges: first/last column coverage-weighted
        out.append((x0 + cols.min(), x0 + cols.max(), y0 + rows.min(), y0 + rows.max()))
    print(spec, 'ref x %d..%d y %d..%d | port x %d..%d y %d..%d | dx0 %+d dx1 %+d dy0 %+d dy1 %+d' % (*out[0], *out[1], out[1][0]-out[0][0], out[1][1]-out[0][1], out[1][2]-out[0][2], out[1][3]-out[0][3]))

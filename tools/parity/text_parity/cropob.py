import sys
import numpy as np
from PIL import Image
ref = np.asarray(Image.open(sys.argv[1]).convert('RGBA'), dtype=np.float32)[..., :3]
ren = np.asarray(Image.open(sys.argv[2]).convert('RGBA'), dtype=np.float32)
ren = ren[..., :3] * ren[..., 3:4] / 255.0
x0, y0, x1, y1 = [int(v) for v in sys.argv[3].split(',')]
s = int(sys.argv[5])
a = Image.fromarray(ref[y0:y1, x0:x1].clip(0, 255).astype(np.uint8)).resize(((x1-x0)*s, (y1-y0)*s), Image.NEAREST)
b = Image.fromarray(ren[y0:y1, x0:x1].clip(0, 255).astype(np.uint8)).resize(((x1-x0)*s, (y1-y0)*s), Image.NEAREST)
out = Image.new('RGB', (a.width, a.height*2+6), (255, 0, 255)); out.paste(a, (0, 0)); out.paste(b, (0, a.height+6)); out.save(sys.argv[4])

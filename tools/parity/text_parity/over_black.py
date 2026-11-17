import sys
import numpy as np
from PIL import Image
ref = np.asarray(Image.open(sys.argv[1]).convert('RGBA'), dtype=np.float32)
ren = np.asarray(Image.open(sys.argv[2]).convert('RGBA'), dtype=np.float32)
refb = ref[..., :3]                         # premultiplied: RGB is the picture over black
renb = ren[..., :3] * (ren[..., 3:4] / 255.0)   # straight alpha over black
Image.fromarray(refb.clip(0, 255).astype(np.uint8)).save(sys.argv[3])
Image.fromarray(renb.clip(0, 255).astype(np.uint8)).save(sys.argv[4])
d = np.abs(refb - renb).mean(axis=2)
print('whole-screen mean diff %.3f' % d.mean())

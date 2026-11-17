import sys
import numpy as np
from PIL import Image
a = np.asarray(Image.open(sys.argv[1]).convert('RGBA'), dtype=np.float32)
Image.fromarray((a[..., :3] * a[..., 3:4] / 255.0).clip(0, 255).astype(np.uint8)).save(sys.argv[2])

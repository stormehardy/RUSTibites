import sys
import numpy as np
from PIL import Image
a = np.asarray(Image.open(sys.argv[1]).convert('RGB'), dtype=np.float32)
b = np.asarray(Image.open(sys.argv[2]).convert('RGB'), dtype=np.float32)
d = np.abs(a - b).mean(axis=2)
print('mean %.3f  >40: %.2f%%' % (d.mean(), (d > 40).mean() * 100))

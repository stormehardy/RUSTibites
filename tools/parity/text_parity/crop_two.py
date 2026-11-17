"""usage: crop_two.py render.png reference.png x0,y0,x1,y1 out.png [scale]  -> reference over render, NEAREST upscaled"""
import sys
from PIL import Image
ren = Image.open(sys.argv[1]).convert('RGB')
ref = Image.open(sys.argv[2]).convert('RGB')
x0, y0, x1, y1 = [int(v) for v in sys.argv[3].split(',')]
s = int(sys.argv[5]) if len(sys.argv) > 5 else 4
a = ref.crop((x0, y0, x1, y1)).resize(((x1 - x0) * s, (y1 - y0) * s), Image.NEAREST)
b = ren.crop((x0, y0, x1, y1)).resize(((x1 - x0) * s, (y1 - y0) * s), Image.NEAREST)
out = Image.new('RGB', (a.width, a.height * 2 + 6), (255, 0, 255))
out.paste(a, (0, 0))
out.paste(b, (0, a.height + 6))
out.save(sys.argv[4])

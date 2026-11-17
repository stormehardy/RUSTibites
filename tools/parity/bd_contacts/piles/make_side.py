import sys
sys.path.insert(0, r'C:\Temp\bd')
import make_small
R = 4.502914 + 2.5
for nm, g in (('sd_m10', -0.010), ('sd_m02', -0.002), ('sd_p02', 0.002), ('sd_p06', 0.006), ('sd_p11', 0.011), ('sd_p30', 0.03)):
    make_small.custom(nm, [(R + g, 31.0)], amount=20.0)

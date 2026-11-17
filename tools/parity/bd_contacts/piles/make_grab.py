import sys
sys.path.insert(0, r'C:\Temp\bd')
import make_small
for amt in (60, 200, 400):
    for gap in (18, 24):
        make_small.custom(f'grab{amt}_{gap}', [(0, gap)], amount=amt, accel=0.6, extra={40: 1.0})
make_small.custom('grabmeat200', [(0, 20)], amount=200, material='Meat', accel=0.6, extra={40: 1.0})
make_small.custom('grabside', [(6.0, 22)], amount=100, accel=0.6, extra={40: 1.0})

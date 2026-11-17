import sys
sys.path.insert(0, r'C:\Temp\bd')
import make_small
R = 4.502914 + 2.5
make_small.custom('sdm_p02', [(R + 0.002, 31.0)], amount=20.0, material='Meat')
make_small.custom('sdm_p30', [(R + 0.3, 31.0)], amount=20.0, material='Meat')
make_small.custom('sd_p30b', [(R + 0.3, 31.0)], amount=20.0)
make_small.custom('sd_p80', [(R + 0.8, 31.0)], amount=20.0)

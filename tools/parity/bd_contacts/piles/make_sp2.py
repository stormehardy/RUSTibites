import sys
sys.path.insert(0, r'C:\Temp\bd')
from make_pile import pile
for nm, sp in (('t4998', 4.998), ('t49995', 4.9995), ('t50005', 5.0005), ('t5002', 5.002)):
    print(pile(nm, rows=2, spacing=sp))

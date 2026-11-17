import sys
sys.path.insert(0, r'C:\Temp\bd')
from make_pile import pile
for sp in (4.96, 4.99, 5.01, 5.03, 5.06):
    print(pile(f'sp{int(sp*100)}', rows=2, spacing=sp))

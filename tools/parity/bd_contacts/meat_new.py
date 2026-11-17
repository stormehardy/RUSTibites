import sys
sys.path.insert(0,'C:/Temp')
from meat_an import meat
import math
init=meat('bd/bd2.zip')
for p in sys.argv[1:]:
    cur=meat(p)
    # match each cur pellet to an init pellet by amount equality(~) and position < 15
    used=set(); new=[]
    byamt={}
    for i,x in enumerate(init): byamt.setdefault(round(x[1],1),[]).append(i)
    cnt=0
    for x in cur:
        ok=False
        for i in byamt.get(round(x[1],1),[]):
            if i in used: continue
            if math.hypot(init[i][2]-x[2],init[i][3]-x[3])<30: used.add(i); ok=True; break
        if not ok: new.append(x)
    lost=[init[i] for i in range(len(init)) if i not in used]
    print(p,'cur',len(cur),'unchanged',len(used),'new/changed',len(new),round(sum(x[1] for x in new)),'| init pellets gone/changed',len(lost),round(sum(x[1] for x in lost)))
print()
import collections
def hist(p):
    cur=meat(p); used=set(); new=[]
    byamt={}
    for i,x in enumerate(init): byamt.setdefault(round(x[1],1),[]).append(i)
    for x in cur:
        ok=False
        for i in byamt.get(round(x[1],1),[]):
            if i in used: continue
            if math.hypot(init[i][2]-x[2],init[i][3]-x[3])<30: used.add(i); ok=True; break
        if not ok: new.append(x)
    b=[0,10,30,60,100,200,400,800,1e9]
    h=collections.Counter(); s=collections.Counter()
    for x in new:
        for i in range(len(b)-1):
            if b[i]<=x[1]<b[i+1]: h[i]+=1; s[i]+=x[1]
    print(p,[(b[i],h[i],round(s[i])) for i in range(len(b)-1)])
    return new
g=hist('meat/g30.zip'); r=hist('meat/r600.zip')
print(sorted(round(x[1]) for x in g)[-15:]); print(sorted(round(x[1]) for x in r)[-15:])

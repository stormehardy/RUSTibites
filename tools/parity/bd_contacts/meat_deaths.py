import sys,math,json,zipfile
sys.path.insert(0,'C:/Temp')
from meat_an import meat
def bibs(p):
    z=zipfile.ZipFile(p); out={}
    for n in z.namelist():
        if n.startswith('bibites/') and n.endswith('.bb8'):
            d=json.loads(z.read(n).decode('utf-8-sig')); out[d['body']['id']]=d
    return out
init=meat('bd/bd2.zip'); B0=bibs('bd/bd2.zip')
def analyse(p):
    cur=meat(p); B=bibs(p)
    used=set(); new=[]
    byamt={}
    for i,x in enumerate(init): byamt.setdefault(round(x[1],1),[]).append(i)
    for x in cur:
        ok=False
        for i in byamt.get(round(x[1],1),[]):
            if i in used: continue
            if math.hypot(init[i][2]-x[2],init[i][3]-x[3])<30: used.add(i); ok=True; break
        if not ok: new.append(x)
    gone=[b for k,b in B0.items() if k not in B or B[k]['body'].get('dead')]
    born=[k for k in B if k not in B0]
    print(p,'died/dead',len(gone),'newborn',len(born),'new pellets',len(new))
    # est meat of each death
    tot=0
    for b in gone:
        tot+=b['body']['d2Size']
    rem=list(new); matched=0; mamt=0
    for b in gone:
        x,y=b['rb2d']['px'],b['rb2d']['py']
        near=[q for q in rem if math.hypot(q[2]-x,q[3]-y)<60]
        for q in near: rem.remove(q)
        matched+=len(near); mamt+=sum(q[1] for q in near)
    print('   pellets near dead bibites:',matched,round(mamt),'| other new:',len(rem),round(sum(q[1] for q in rem)))
    return gone
for p in sys.argv[1:]: analyse(p)

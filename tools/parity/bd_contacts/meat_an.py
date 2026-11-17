import json,zipfile,sys,collections
def load(p):
    z=zipfile.ZipFile(p)
    d=json.loads(z.read('pellets.bb8scene').decode('utf-8-sig'))
    out=[]
    for g in d['pellets']:
        for q in g['pellets']:
            out.append((q['pellet']['material'],q['pellet']['amount'],q['transform']['position'][0],q['transform']['position'][1],q['rb2d'].get('vx',0),q['rb2d'].get('vy',0)))
    return out
def meat(p): return [x for x in load(p) if x[0]=='Meat']
if __name__=='__main__':
    for p in sys.argv[1:]:
        m=meat(p); a=sorted(x[1] for x in m)
        print(p,len(m),round(sum(a)),'min',round(a[0],1),'med',round(a[len(a)//2],1),'max',round(a[-1]))
        b=[0,50,100,200,500,1000,2000,5000,1e9]
        h=collections.Counter(); s=collections.Counter()
        for x in a:
            for i in range(len(b)-1):
                if b[i]<=x<b[i+1]: h[i]+=1; s[i]+=x
        print('  ',[ (b[i],h[i],round(s[i])) for i in range(len(b)-1)])

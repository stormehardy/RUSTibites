import json,re,sys
G=json.load(open('game_fix_out2.json'))
def rust(name):
    rows={};pl={}
    for line in open(f'rust_b24_{name}.txt'):
        if line.startswith('TR '):
            t=int(line.split()[1]); ms=re.findall(r'P=\(([-\d.]+),([-\d.]+)\) rot=([-\d.]+) v=\(([-\d.]+),([-\d.]+)\) w=([-\d.]+)',line)
            rows[t]=[tuple(map(float,m)) for m in ms]
        if line.startswith('PL '):
            t=int(line.split()[1]); ps=re.findall(r'\(([-\d.]+),([-\d.]+),v([-\d.]+),([-\d.]+),h',line)
            pl[t]=[tuple(map(float,p)) for p in ps]
    return rows,pl
for name in sys.argv[1:]:
    rows,pl=rust(name)
    print('=====',name,'game: bib (x,y,rot,vx,vy) other (x,y) | rust same, shift 1')
    for r in G[name]:
        t=r['tick']
        if not 21<=t<=34: continue
        b=sorted(r['bibites'],key=lambda b:b['id']!=r['bibites'][0]['id'])[0]
        rr=rows.get(t+1); 
        if rr is None: continue
        rb=min(rr,key=lambda m:abs(m[1]-b['y']-0.3)) if len(rr)>1 else rr[0]
        o=r['pellets'][0][:2] if r['pellets'] else None
        ro=pl.get(t+1)
        ro=ro[0][:2] if ro else None
        if o is None and len(r['bibites'])>1:
            ob=[x for x in r['bibites'] if x['id']==77777777][0]; o=(ob['x'],ob['y'])
            ro=max(rr,key=lambda m:m[1])[:2] if len(rr)>1 else None
        print('%2d | %6.2f %7.3f %7.2f %6.2f %6.2f | %s | %6.2f %7.3f %7.2f %6.2f %6.2f | %s'%(t,b['x'],b['y'],b['rot'],b['vx'],b['vy'],('(%.2f,%.2f)'%tuple(o)) if o else '-',rb[0],rb[1],rb[2],rb[3],rb[4],('(%.2f,%.2f)'%tuple(ro)) if ro else '-'))

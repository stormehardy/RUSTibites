import json,re,sys
G=json.load(open('game_fix_out.json'))
SH=int(sys.argv[1]); tag=sys.argv[2]
def rust(name):
    rows={};pl={}
    for line in open(f'rust_{tag}_{name}.txt'):
        if line.startswith('TR '):
            t=int(line.split()[1]); ms=re.findall(r'P=\(([-\d.]+),([-\d.]+)\) rot=([-\d.]+) v=\(([-\d.]+),([-\d.]+)\) w=([-\d.]+)',line)
            rows[t]=[tuple(map(float,m)) for m in ms]
        if line.startswith('PL '):
            t=int(line.split()[1]); ps=re.findall(r'\(([-\d.]+),([-\d.]+),v([-\d.]+),([-\d.]+),h',line)
            pl[t]=[tuple(map(float,p)) for p in ps]
    return rows,pl
for name in sys.argv[3:]:
    rows,pl=rust(name)
    print('=====',name,'(game t <-> rust t+%d)  game y vy | rust y vy | game other y | rust other y'%SH)
    for r in G[name]:
        t=r['tick']; b=r['bibites'][0]; rr=rows.get(t+SH); p=r['pellets']; rp=pl.get(t+SH,[])
        if rr is None: continue
        if not (20<=t<=36): continue
        go=('%.2f'%p[0][1]) if p else ('%.2f'%r['bibites'][1]['y'] if len(r['bibites'])>1 else '-')
        ro=('%.2f'%rp[0][1]) if rp else ('%.2f'%rr[1][1] if len(rr)>1 else '-')
        print('%3d | %8.3f %7.3f | %8.3f %7.3f | %s | %s'%(t,b['y'],b['vy'],rr[0][1],rr[0][4],go,ro))

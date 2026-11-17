import json,sys,collections,math,statistics as st
sys.path.insert(0,r'Z:\RUSTibites\tools\parity')
import trace_compare as tc
BASE=9333797
gb,G=tc.load_game('trace/game_trace.jsonl')
R=tc.load_rust(sys.argv[1]+'/trace.jsonl')
cont=collections.defaultdict(lambda: collections.defaultdict(float))
for line in open('trace/game_trace.jsonl.physics2.jsonl'):
    d=json.loads(line)
    if d.get('type')!='contact': continue
    t=d['tick']-BASE-1
    k='bibite' if d['other_layer']==8 else 'pellet'
    cont[(t,d['id'])][k+('_p' if d['persistent'] else '_n')]+=d['normal_impulse']
def dv(tr,i,a):
    x=tr.get((a,i));y=tr.get((a+1,i))
    return None if not x or not y else math.hypot(y['vx']-x['vx'],y['vy']-x['vy'])
groups=collections.defaultdict(lambda: ([],[]))
ids={i for (t,i) in G if t==1}
for t in range(4,60):
    for i in ids:
        c=cont.get((t,i),{})
        g=dv(G,i,t); r=dv(R,i,t)
        if g is None or r is None: continue
        if not c: key='none'
        elif c.get('bibite_n') or c.get('bibite_p'): key='bibite contact'
        elif c.get('pellet_n'): key='pellet new'
        else: key='pellet persistent only'
        groups[key][0].append(g); groups[key][1].append(r)
for k,(g,r) in groups.items():
    print('%-24s body-ticks %5d | mean |dv| game %.3f rust %.3f | median game %.3f rust %.3f'%(k,len(g),st.mean(g),st.mean(r),st.median(g),st.median(r)))

import json,sys,collections,math,statistics as st
sys.path.insert(0,r'Z:\RUSTibites\tools\parity')
import trace_compare as tc
BASE=9333797
gb,G=tc.load_game('trace/game_trace.jsonl')
R=tc.load_rust(sys.argv[1]+'/trace.jsonl')
cnt=collections.Counter(); rows=[]
for line in open('trace/game_trace.jsonl.physics2.jsonl'):
    d=json.loads(line)
    if d.get('type')!='contact': continue
    t=d['tick']-BASE-1; cnt[(t,d['id'])]+=1
    if not d['persistent'] and d['other_layer']==9: rows.append((t,d))
rows=[(t,d) for t,d in rows if cnt[(t,d['id'])]==1 and 3<=t<=60]
def dv(tr,i,a):
    x=tr.get((a,i));y=tr.get((a+1,i))
    return None if not x or not y else (y['vx']-x['vx'],y['vy']-x['vy'],x['rot'],y['angvel']-x['angvel'])
pn=[];ph=[];bypell=collections.defaultdict(list)
for t,d in rows:
    g=dv(G,d['id'],t); r=dv(R,d['id'],t)
    if not g or not r: continue
    ex=(g[0]-r[0],g[1]-r[1]); n=(d['normal']['x'],d['normal']['y'])
    a=math.radians(g[2]+90); hd=(math.cos(a),math.sin(a))
    pn.append(ex[0]*n[0]+ex[1]*n[1]); ph.append(ex[0]*hd[0]+ex[1]*hd[1])
    amt=d['pellet_amount'] or 0
    bypell[min(int(math.log10(max(amt,1))),3)].append(math.hypot(*ex))
print('n',len(pn),'mean (game-rust) dv along contact normal %.3f  along heading %.3f  | |excess| median %.3f'%(st.mean(pn),st.mean(ph),st.median([math.hypot(a,b) for a,b in zip(pn,ph)])))
for k in sorted(bypell): print('pellet amount 10^%d: n %d median |dv_game-dv_rust| %.3f'%(k,len(bypell[k]),st.median(bypell[k])))

import json,sys,collections,math,statistics as st
sys.path.insert(0,r'Z:\RUSTibites\tools\parity')
import trace_compare as tc
BASE=9333797
gb,G=tc.load_game('trace/game_trace.jsonl')
R=tc.load_rust(sys.argv[1]+'/trace.jsonl')
ev=collections.defaultdict(set)   # id -> {(tick,kind)} of non-persistent contacts
anyc=collections.defaultdict(set)
for line in open('trace/game_trace.jsonl.physics2.jsonl'):
    d=json.loads(line)
    if d.get('type')!='contact': continue
    t=d['tick']-BASE-1
    kind='bibite' if d['other_layer']==8 else ('pellet' if d['other_layer']==9 else 'other')
    anyc[d['id']].add(t)
    if not d['persistent']: ev[d['id']].add((t,kind))
def dv(tr,i,t0,t1):
    a=tr.get((t0,i));b=tr.get((t1,i))
    if not a or not b: return None
    return (b['vx']-a['vx'],b['vy']-a['vy'],b['angvel']-a['angvel'])
def stats(name,items):
    e=[];ew=[]
    for i,t in items:
        g=dv(G,i,t-1,t+2); r=dv(R,i,t-1,t+2)
        if g is None or r is None: continue
        e.append(math.hypot(g[0]-r[0],g[1]-r[1])); ew.append(abs(g[2]-r[2]))
    if e: print('%-22s n=%4d  dv err median %.3f p90 %.3f max %.2f | dangvel err median %.3f p90 %.2f'%(name,len(e),st.median(e),sorted(e)[int(.9*(len(e)-1))],max(e),st.median(ew),sorted(ew)[int(.9*(len(ew)-1))]))
bb=[(i,t) for i,s in ev.items() for (t,k) in s if k=='bibite' and 2<=t<=70]
pp=[(i,t) for i,s in ev.items() for (t,k) in s if k=='pellet' and 2<=t<=70]
stats('bibite first contacts',bb)
stats('pellet first contacts',pp)
ids=[i for i in {i for (t,i) in G if t==1}]
ctrl=[(i,t) for i in ids if not anyc.get(i) for t in (10,30,50)]
stats('no contact at all',ctrl)
print('----- samples (pellet first contacts, tick t): game dv(t-1..t+2) vs rust')
import random
random.seed(3)
rows=[]
for line in open('trace/game_trace.jsonl.physics2.jsonl'):
    d=json.loads(line)
    if d.get('type')=='contact' and not d['persistent'] and d['other_layer']==9:
        t=d['tick']-BASE-1
        if 3<=t<=60: rows.append((t,d))
for t,d in random.sample(rows,8):
    i=d['id']; g=dv(G,i,t-1,t+2); r=dv(R,i,t-1,t+2)
    if g and r:
        print('id',i,'t',t,'part',d['part_name'],'sep %.3f imp %.2f relv (%.1f,%.1f) n (%.2f,%.2f) pellet %.1f'%(d['separation'],d['normal_impulse'],d['relative_velocity']['x'],d['relative_velocity']['y'],d['normal']['x'],d['normal']['y'],d['pellet_amount'] or 0))
        print('   game dv (%.2f,%.2f) dw %.1f | rust dv (%.2f,%.2f) dw %.1f'%(g[0],g[1],g[2],r[0],r[1],r[2]))

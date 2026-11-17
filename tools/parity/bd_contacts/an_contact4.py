import json,sys,collections,math,statistics as st
sys.path.insert(0,r'Z:\RUSTibites\tools\parity')
import trace_compare as tc
BASE=9333797
gb,G=tc.load_game('trace/game_trace.jsonl')
R=tc.load_rust(sys.argv[1]+'/trace.jsonl')
cnt=collections.Counter(); first={}
rows=[]
for line in open('trace/game_trace.jsonl.physics2.jsonl'):
    d=json.loads(line)
    if d.get('type')!='contact': continue
    t=d['tick']-BASE-1; cnt[(t,d['id'])]+=1
    if not d['persistent'] and d['other_layer']==9 and d['normal_impulse']>8: rows.append((t,d))
rows=[(t,d) for t,d in rows if cnt[(t,d['id'])]==1 and 3<=t<=60]
print('single pellet first contacts with impulse>8:',len(rows))
def dv(tr,i,a,b):
    x=tr.get((a,i));y=tr.get((b,i))
    return None if not x or not y else (y['vx']-x['vx'],y['vy']-x['vy'])
for off in (-1,0,1,2):
    e=[];rat=[]
    for t,d in rows:
        g=dv(G,d['id'],t+off,t+off+1); r=dv(R,d['id'],t+off,t+off+1)
        if g and r:
            e.append(math.hypot(g[0]-r[0],g[1]-r[1]))
            ng=math.hypot(*g)
            if ng>0.3: rat.append(math.hypot(*r)/ng)
    print('one-tick dv window starting at t%+d: n %d err median %.3f p90 %.3f | |dv_rust|/|dv_game| median %.2f (n %d)'%(off,len(e),st.median(e),sorted(e)[int(.9*len(e))],st.median(rat) if rat else 0,len(rat)))
import glob
d2={}
for f in glob.glob('sv/bibites/*.bb8'):
    j=json.load(open(f,encoding='utf-8-sig')); d2[j['body']['id']]=j['body']['d2Size']
print('--- samples: impulse, mass~2*d2, game |dv|*mass, rust |dv|*mass  (window t+0)')
import random; random.seed(5)
for t,d in random.sample(rows,12):
    i=d['id']; g=dv(G,i,t,t+1); r=dv(R,i,t,t+1)
    if not g or not r or i not in d2: continue
    m=2.0*d2[i]
    print('t %2d imp %6.2f m~%.1f game dv (%.2f,%.2f) |.|*m %.1f | rust dv (%.2f,%.2f) |.|*m %.1f | normal (%.2f,%.2f) sep %.3f relv (%.1f,%.1f)'%(t,d['normal_impulse'],m,g[0],g[1],math.hypot(*g)*m,r[0],r[1],math.hypot(*r)*m,d['normal']['x'],d['normal']['y'],d['separation'],d['relative_velocity']['x'],d['relative_velocity']['y']))
print('--- mean |dv| per tick offset (window off..off+1) around the game contact tick, impulse>8 singles')
for off in range(-4,6):
    gg=[];rr=[]
    for t,d in rows:
        g=dv(G,d['id'],t+off,t+off+1); r=dv(R,d['id'],t+off,t+off+1)
        if g and r: gg.append(math.hypot(*g)); rr.append(math.hypot(*r))
    print('off %+d game %.3f rust %.3f'%(off,st.mean(gg),st.mean(rr)))

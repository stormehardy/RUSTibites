import json,sys,statistics as st,math
sys.path.insert(0,r'Z:\RUSTibites\tools\parity')
import trace_compare as tc
gb,g=tc.load_game('trace/game_trace.jsonl')
r=tc.load_rust(sys.argv[1]+'/trace.jsonl')
print(sys.argv[1])
for t in (1,3,5,10,20,40,60,77):
    pe=[];ang=[];ve=[];wv=[];de=[]
    for (tk,i),gv in g.items():
        if tk!=t: continue
        rv=r.get((tk,i))
        if not rv: continue
        pe.append(math.hypot(rv['x']-gv['x'],rv['y']-gv['y'])); ang.append(abs(tc.angdiff(rv['rot'],gv['rot'])))
        ve.append(math.hypot(rv['vx']-gv['vx'],rv['vy']-gv['vy'])); wv.append(abs(rv['angvel']-gv['angvel']))
        rv1=r.get((tk+1,i))
        if rv1: de.append(rv1['energy']-gv['energy'])
    n=len(pe)
    f=lambda a,q: sorted(a)[int(q*(len(a)-1))]
    print(' tick %2d n %d | pos med %.4f p90 %.3f max %.2f | rot med %.4f p90 %.3f | vel med %.4f p90 %.3f | angvel med %.4f p90 %.3f | dE med %.4f'%(t,n,st.median(pe),f(pe,.9),max(pe),st.median(ang),f(ang,.9),st.median(ve),f(ve,.9),st.median(wv),f(wv,.9),st.median(de) if de else 0))

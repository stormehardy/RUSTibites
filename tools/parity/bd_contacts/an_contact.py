import json,sys,collections,statistics as st
BASE=9333797
g=collections.defaultdict(lambda: collections.defaultdict(list))
for line in open('trace/game_trace.jsonl.physics2.jsonl'):
    d=json.loads(line)
    if d.get('type')!='contact': continue
    t=d['tick']-BASE-1
    kind='bibite' if d['other_layer']==8 else ('pellet' if d['other_layer']==9 else 'other')
    g[t][kind].append(d)
def rust(path):
    r=collections.defaultdict(lambda: collections.defaultdict(float))
    for line in open(path+'/trace.jsonl'):
        d=json.loads(line); t=d['tick']
        for e in d['entities']:
            r[t]['bibite']+=e.get('contact_bibite',0); r[t]['pellet']+=e.get('contact_pellet',0)
            r[t]['impulse']+=e.get('contact_impulse',0.0); r[t]['count']+=e.get('contact_count',0)
            r[t]['bodies_touch']+= 1 if e.get('contact_count',0)>0 else 0
            r[t]['enter']+=e.get('contact_enter_count',0)
    return r
R=rust(sys.argv[1])
print('tick | game: bibite-contacts pellet-contacts impulse(bibite) persistent | rust: bibite pellet impulse total count')
for t in (0,1,2,3,5,10,20,40,60,78):
    gb=g[t]['bibite'];gp=g[t]['pellet']
    imp=sum(x['normal_impulse'] for x in gb); pers=sum(1 for x in gb+gp if x['persistent'])
    print(t,'|',len(gb),len(gp),'%.0f'%imp,pers,'|',R[t]['bibite'],R[t]['pellet'],'%.0f'%R[t]['impulse'],R[t]['count'])
print('--- totals incl. pellets: game impulse all / rust impulse; game mean separation')
for t in (1,2,3,5,10,20,40,60,78):
    allc=g[t]['bibite']+g[t]['pellet']+g[t]['other']
    imp=sum(x['normal_impulse'] for x in allc)
    sep=st.mean(x['separation'] for x in allc) if allc else 0
    pm=sum(x['normal_impulse'] for x in g[t]['pellet'])
    print(t,'game rows',len(allc),'impulse all %.0f (pellets %.0f)'%(imp,pm),'mean sep %.4f'%sep,'| rust count',R[t]['count'],'impulse %.0f'%R[t]['impulse'])

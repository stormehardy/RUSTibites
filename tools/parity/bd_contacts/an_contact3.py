import json,sys,collections,math,statistics as st
BASE=9333797
gi=collections.defaultdict(float); gc=collections.defaultdict(int)
for line in open('trace/game_trace.jsonl.physics2.jsonl'):
    d=json.loads(line)
    if d.get('type')!='contact': continue
    t=d['tick']-BASE-1
    gi[(t,d['id'])]+=d['normal_impulse']; gc[(t,d['id'])]+=1
ri={};rc={};rb={}
for line in open(sys.argv[1]+'/trace.jsonl'):
    d=json.loads(line)
    for e in d['entities']:
        ri[(d['tick'],e['id'])]=e.get('contact_impulse',0.0); rc[(d['tick'],e['id'])]=e.get('contact_count',0)
for SH in (-1,0,1):
    both=[];gonly=0;ronly=0;n=0
    for (t,i),v in gi.items():
        if not 3<=t<=40: continue
        r=ri.get((t+SH,i),0.0); n+=1
        if r>0: both.append((v,r))
        else: gonly+=1
    for (t,i),v in ri.items():
        if 3<=t<=40 and v>0 and gi.get((t-SH,i),0)==0: ronly+=1
    ratios=[r/v for v,r in both if v>0.5]
    print('shift',SH,'game bodies with contact',n,'both',len(both),'game only',gonly,'rust only',ronly,'| impulse ratio rust/game median %.2f p25 %.2f p75 %.2f'%(st.median(ratios),sorted(ratios)[len(ratios)//4],sorted(ratios)[3*len(ratios)//4]))
print('--- by contact counts (shift 0): (game rows, rust count) -> n, median ratio')
grp=collections.defaultdict(list)
for (t,i),v in gi.items():
    if 3<=t<=40 and ri.get((t,i),0)>0 and v>0.5:
        grp[(gc[(t,i)],rc[(t,i)])].append(ri[(t,i)]/v)
for k in sorted(grp,key=lambda k:-len(grp[k]))[:10]: print(k,len(grp[k]),'%.2f'%st.median(grp[k]))
ro=[v for (t,i),v in ri.items() if 3<=t<=40 and v>0 and gi.get((t,i),0)==0]
print('rust-only impulse: n',len(ro),'median %.3f p90 %.2f max %.1f'%(st.median(ro),sorted(ro)[int(.9*len(ro))],max(ro)))
rcn=[rc[(t,i)] for (t,i),v in ri.items() if 3<=t<=40 and v>0 and gi.get((t,i),0)==0]
print('rust-only contact counts: mean %.2f'%st.mean(rcn))

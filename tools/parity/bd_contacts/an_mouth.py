import json,sys,statistics as st,math
sys.path.insert(0,r'Z:\RUSTibites\tools\parity')
import trace_compare as tc
gb,g=tc.load_game('trace/game_trace.jsonl')
r=tc.load_rust(sys.argv[1]+'/trace.jsonl')
for SH in (0,1):
  print('shift',SH)
  for t in (1,2,3,5,10,20,40):
    n=0;bad=0;gm=0;rm=0;both=0
    for (tk,i),gv in g.items():
        if tk!=t: continue
        rv=r.get((tk+SH,i))
        if not rv: continue
        n+=1
        a=gv.get('n_in_mouth') or 0; b=rv.get('n_in_mouth') or 0
        gm+= a>0; rm+= b>0
        if (a>0)!=(b>0): bad+=1
    print('  tick',t,'n',n,'game in-mouth',gm,'rust in-mouth',rm,'disagree',bad)

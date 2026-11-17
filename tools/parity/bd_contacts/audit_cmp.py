import json,sys,collections
name,capdir,tps,t0key=sys.argv[1],sys.argv[2],int(sys.argv[3]),None
A=json.load(open('C:/Temp/audit_game.json'))[name]
rows=A['rows']; ev=A['events']
base=rows[0]['t']
P={}
for line in open(capdir+'/ticks.jsonl'):
    r=json.loads(line)
    if 'behavior' in r: P[r['tick']]=r
t0=P[min(P)]['t']
f=lambda x,n=1: round(x,n) if x is not None else None
print('t | bib g/p | eggs g/p | plantN g/p | meatN g/p | E g/p | H g/p | speed g/p | bite g/p | Acc g/p')
for r in rows[::max(1,len(rows)//10)]:
    t=r['t']-base
    tk=min(P,key=lambda k:abs((P[k]['t']-t0)-t))
    p=P[tk]
    print(round(t),'|',r['bibites'],p['counts']['bibites'],'|',r['eggs'],p['counts']['eggs'],'|',r['plantN'],p['pellets']['plant_n'],'|',r['meatN'],p['pellets']['meat_n'],'|',f(r['energyMean']),f(p['vitals']['energy_mean']),'|',f(r['healthMean']),f(p['vitals']['health_mean']),'|',f(r['speedMean']),f(p['behavior']['speed_mean']),'|',f(r['biteRateMean'],4),f(p['behavior']['bite_rate_mean'],4),'|',f(r['outputs']['Accelerate'],3),f(p['behavior']['outputs']['Accelerate'],3))
c=collections.Counter((e.get('kind'),e.get('cause')) for e in ev)
pc=collections.Counter()
for line in open(capdir+'/events.jsonl'):
    r=json.loads(line); pc[(r.get('kind'),r.get('cause'))]+=1
print('game events',sorted(c.items(),key=lambda x:-x[1])[:6]); print('port events',sorted(pc.items(),key=lambda x:-x[1])[:6])

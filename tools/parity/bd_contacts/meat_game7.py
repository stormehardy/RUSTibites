import sys, json, time, argparse
from pathlib import Path
sys.path.insert(0, r'Z:\RUSTibites\tools\parity')
import capture_hud_reference as h
cap = h.cap
BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(r'C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI'))
args = h.build_args(argparse.Namespace()); args.no_plugin = True
session = cap.WorldSession(args, BridgeClient, LiveRuntime, Lab)
try:
    client = session.open()
    client.load_world('C:/Temp/bd/fix_die_s2.zip')
    client.set_time(paused=True)
    last=None
    for tick in range(120):
        ms=[p for p in client.map_pellets() if p[2]==1]
        bs=client.entities('bibite')
        d=None
        for e in bs:
            d=client.entity(int(e['h']))['data']['body']; 
        info=(len(ms), None if d is None else (d['dead'],round(d['health'],2),round(d['energy'],2)))
        if info!=last: print(tick, info, [ (round(p[0],1),round(p[1],1),round(p[3],1)) for p in ms], flush=True); last=info
        t = float(client.status()['simulatedTime'])
        client.run_start({'simTime': t + 0.05}, speed=1.0, sample=None, pause=True, replace=True)
        end=time.time()+30
        while time.time()<end:
            st=client.run_state(timeout=15.0)
            if st.get('state') not in ('running','stopping'): break
            time.sleep(0.005)
finally:
    session.close()

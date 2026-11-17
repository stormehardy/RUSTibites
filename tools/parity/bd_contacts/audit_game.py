import sys, json, time, argparse
from pathlib import Path
sys.path.insert(0, r'Z:\RUSTibites\tools\parity')
import capture_hud_reference as h
cap = h.cap
BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(r'C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI'))
args = h.build_args(argparse.Namespace()); args.no_plugin = True
session = cap.WorldSession(args, BridgeClient, LiveRuntime, Lab)
SECS = float(sys.argv[1])
saves = sys.argv[2:]
out = {}
try:
    client = session.open()
    for sv in saves:
        name = Path(sv).stem.strip().replace(' ', '_')
        try:
            client.load_world(sv)
            client.set_time(paused=True)
            st0 = client.status()
            client.run_start({'elapsed': SECS}, speed=50, sample={'every': 5.0, 'fields': ['counts','pellets','vitals','species','brain','lifecycle','behavior','body'], 'events': True})
            t0 = time.time()
            while True:
                st = client.run_state()
                if st.get('state') != 'running' or time.time() - t0 > 900: break
                time.sleep(10)
            rows = client.series(since=0, limit=5000).get('rows') or []
            ev = []; since = 0
            while True:
                e = client.events(since=since, limit=20000)
                got = e.get('events') or e.get('rows') or []
                if not got: break
                ev.extend(got); since = e.get('next', since + len(got))
                if len(got) < 20000: break
            out[name] = {'rows': rows, 'events': ev, 'status0': {k: st0.get(k) for k in ('simulatedTime', 'counts')}}
            print('done', name, len(rows), len(ev), round(time.time() - t0), flush=True)
            json.dump(out, open(r'C:\Temp\audit_game.json', 'w'))
        except Exception as ex:
            print('FAILED', name, repr(ex)[:200], flush=True)
finally:
    session.close()

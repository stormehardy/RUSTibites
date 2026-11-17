import sys, json, time, argparse
from pathlib import Path
sys.path.insert(0, r'Z:\RUSTibites\tools\parity')
import capture_hud_reference as h
cap = h.cap
BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(r'C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI'))
args = h.build_args(argparse.Namespace()); args.no_plugin = True
session = cap.WorldSession(args, BridgeClient, LiveRuntime, Lab)
NT = int(sys.argv[1]) if len(sys.argv) > 1 else 40
names = sys.argv[2:]
out = {}
try:
    client = session.open()
    for name in names:
        client.load_world(rf'C:\Temp\bd\fix_{name}.zip')
        client.set_time(paused=True)
        rows = []
        for tick in range(NT + 1):
            b = client.entities('bibite')
            ent = None
            rec = {'tick': tick}
            bs = []
            for e in b:
                d = client.entity(int(e['h']))['data']
                rb = d['rb2d']
                bs.append({'id': d['body']['id'], 'x': rb['px'], 'y': rb['py'], 'vx': rb['vx'], 'vy': rb['vy'], 'rot': d['transform']['rotation'], 'hp': d['body']['health'], 'en': d['body']['energy']})
            rec['bibites'] = bs
            rec['pellets'] = [list(p) for p in client.map_pellets()]
            rows.append(rec)
            if tick < NT:
                t = float(client.status()['simulatedTime'])
                client.run_start({'simTime': t + 0.05}, speed=1.0, sample=None, pause=True, replace=True)
                end = time.time() + 30
                while time.time() < end:
                    st = client.run_state(timeout=15.0)
                    if st.get('state') not in ('running', 'stopping'):
                        break
                    time.sleep(0.005)
        out[name] = rows
        print('done', name, flush=True)
    json.dump(out, open('C:/Temp/bd/' + __import__('os').environ.get('OUT','game_pile_out.json'), 'w'))
finally:
    session.close()

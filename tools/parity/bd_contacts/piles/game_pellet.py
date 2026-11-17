import sys, json, argparse, time
from pathlib import Path
sys.path.insert(0, r'Z:\RUSTibites\tools\parity')
import capture_hud_reference as h
cap = h.cap
BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(r'C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI'))
args = h.build_args(argparse.Namespace()); args.no_plugin = True
session = cap.WorldSession(args, BridgeClient, LiveRuntime, Lab)
name = sys.argv[1]; NT = int(sys.argv[2])
try:
    client = session.open()
    client.load_world(rf'C:\Temp\bd\fix_{name}.zip'); client.set_time(paused=True)
    px, py = 7.0, 31.0
    rows = []
    for tick in range(NT + 1):
        near = client.pellets_near(px, py, 12.0)['pellets']
        pel = near[0]['h']; d = client.entity(int(pel))['data']; px, py = d['rb2d']['px'], d['rb2d']['py']
        b = client.entities('bibite')[0]
        bd = client.entity(int(b['h']))['data']
        rows.append({'tick': tick, 'pellet': d.get('rb2d'), 'bibite': bd['rb2d'], 'bibite_rot': bd['transform']['rotation']})
        if tick < NT:
            t = float(client.status()['simulatedTime'])
            client.run_start({'simTime': t + 0.05}, speed=1.0, sample=None, pause=True, replace=True)
            end = time.time() + 30
            while time.time() < end:
                st = client.run_state(timeout=15.0)
                if st.get('state') not in ('running', 'stopping'): break
                time.sleep(0.005)
    json.dump(rows, open(rf'C:\Temp\bd\game_pellet_{name}.json', 'w'))
    print(json.dumps(rows[26])[:500])
finally:
    session.close()

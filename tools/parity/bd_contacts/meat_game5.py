import sys, json, time, argparse
from pathlib import Path
sys.path.insert(0, r'Z:\RUSTibites\tools\parity')
import capture_hud_reference as h
cap = h.cap
BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(r'C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI'))
args = h.build_args(argparse.Namespace()); args.no_plugin = True
session = cap.WorldSession(args, BridgeClient, LiveRuntime, Lab)
out={}
try:
    client = session.open()
    for name in sys.argv[1:]:
        client.load_world(f'C:/Temp/bd/fix_die_{name}.zip')
        client.set_time(paused=True)
        client.run_start({'elapsed': 30.0}, speed=50, sample=None)
        t0 = time.time()
        while time.time() - t0 < 300 and client.run_state().get('state') == 'running': time.sleep(1)
        out[name]=[(round(p[3],2)) for p in client.map_pellets() if p[2]==1]
        print(name, out[name], flush=True)
finally:
    session.close()

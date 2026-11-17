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
    for sv in sys.argv[1:]:
        try:
            client.load_world(sv)
            client.set_time(paused=True)
            st = client.status()
            print(Path(sv).name, 'LOADED counts', st.get('counts'), 'simTime', st.get('simulatedTime'), flush=True)
            client.run_start({'elapsed': 10.0}, speed=50, sample=None)
            t0 = time.time()
            while time.time() - t0 < 300 and client.run_state().get('state') == 'running': time.sleep(3)
            st = client.status(); print('  after 10 s:', st.get('counts'), flush=True)
        except Exception as ex:
            print(Path(sv).name, 'FAILED', repr(ex)[:300], flush=True)
finally:
    session.close()

import sys, json, argparse, time
from pathlib import Path
sys.path.insert(0, r'Z:\RUSTibites\tools\parity')
import capture_hud_reference as h
cap = h.cap
BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(r'C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI'))
args = h.build_args(argparse.Namespace()); args.no_plugin = True
session = cap.WorldSession(args, BridgeClient, LiveRuntime, Lab)
name = sys.argv[1]
cx, cy, size = float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4])
out = sys.argv[5]
w, hgt = (int(sys.argv[6]), int(sys.argv[7])) if len(sys.argv) > 7 else (1920, 1080)
try:
    client = session.open()
    client.load_world(rf'C:\Temp\bd\fix_{name}.zip')
    client.set_time(paused=True)
    time.sleep(1.0)
    png = client.screenshot(w, hgt, x=cx, y=cy, size=size)
    open(out, 'wb').write(png)
    print('shot', out, len(png))
finally:
    session.close()

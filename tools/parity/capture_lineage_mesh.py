# Captures the hidden game's lineage MESH view, after resetting the lineage window (the first switch leaves GraphElementHolder at NaN in a hidden game).
# usage: capture_lineage_mesh.py <save.zip> <species id>. Output in C:\Temp\lineage_out. Needs the ParityDump plugin (remove it afterwards).
import sys, json, io, base64, time
from pathlib import Path
sys.path.insert(0, r'Z:\RUSTibites\tools\parity')
import argparse
import capture_hud_reference as h
from PIL import Image
cap = h.cap
OUT = Path(r'C:\Temp\lineage_out'); OUT.mkdir(exist_ok=True)
BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(r'C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI'))
session = cap.WorldSession(h.build_args(argparse.Namespace()), BridgeClient, LiveRuntime, Lab)
pj = h.plugin_json
SAVE = Path(sys.argv[1])
SID = int(sys.argv[2]) if len(sys.argv) > 2 else 0
SCENE = 'UICanvas/MainUI/Layer2/AncestralLineagePanel/'


def tick(client, sec=0.5):
    client.set_time(paused=False); time.sleep(sec); client.set_time(paused=True)


def shot(name):
    ov = pj(session.plugin, '/d/ui_overlay')
    img = Image.open(io.BytesIO(base64.b64decode(ov['png_base64']))).convert('RGBA')
    bg = Image.new('RGBA', img.size, (0, 0, 0, 255))
    Image.alpha_composite(bg, img).convert('RGB').save(OUT / (name + '.png'))
    ui = session.plugin.ui_runtime()
    json.dump(ui, open(OUT / ('ui_' + name + '.json'), 'w'))
    print('saved', name)


def invoke(path, typ, method, args=None):
    r = pj(session.plugin, '/d/invoke', {'path': path, 'type': typ, 'method': method, 'args': args or []})
    print('invoke', path, typ, method, args, '->', json.dumps(r)[:200])
    return r


try:
    client = session.open()
    assert session.plugin_ok, 'plugin missing'
    session.plugin.pacing(True)
    client.load_world(SAVE.absolute())
    client.set_time(paused=True)
    tick(client, 0.6)
    # `GlobalLineageManager.maxEnergy` is 0 until the first species log of the run (every 5 simulated minutes): the mesh's widths are
    # `3^log10(energy / maxEnergy)`, Infinity / NaN before that, and the whole mesh is invisible. Run the game past one log first.
    RUN = float(sys.argv[3]) if len(sys.argv) > 3 else 420.0
    client.run_start({'elapsed': RUN}, speed=50, sample=None)
    t0 = time.time()
    while time.time() - t0 < 600 and client.run_state().get('state') == 'running':
        time.sleep(2)
    print('ran', RUN, 's in', round(time.time() - t0), 's real')
    client.set_time(paused=True)
    if SID > 0:
        print(json.dumps(pj(session.plugin, '/d/open_species', {'id': SID}))[:100])
    else:
        # No species selected (a selection focuses the mesh on its centre, which is NaN in a hidden game and hides the whole mesh).
        print(json.dumps(pj(session.plugin, '/d/press_button', {'path': 'UICanvas/MainUI/Layer0/UI Buttons/TopRight/AncestralLineageButton'}))[:120])
    tick(client, 1.5)
    shot('tree_sel%d' % SID)
    print(json.dumps(pj(session.plugin, '/d/press_button', {'path': SCENE + 'Top Bar/TopRight/MeshButton'}))[:120])
    tick(client, 2.0)
    shot('mesh_sel%d' % SID)
finally:
    session.close()

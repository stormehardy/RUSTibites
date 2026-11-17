# Captures the hidden game's Ancestral Lineage panel, tree and mesh views (overlay png + /d/ui dump in C:\Temp\lineage_out).
# usage: capture_lineage.py <save.zip> <species id> [more ids]. Needs the ParityDump plugin in BepInEx\plugins (remove it afterwards).
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
IDS = [int(v) for v in sys.argv[2:]] or [184]
SCENE = 'UICanvas/MainUI/Layer2/AncestralLineagePanel/'


def tick(client, sec=0.5):
    client.set_time(paused=False); time.sleep(sec); client.set_time(paused=True)


def shot(name):
    ov = pj(session.plugin, '/d/ui_overlay')
    img = Image.open(io.BytesIO(base64.b64decode(ov['png_base64']))).convert('RGBA')
    bg = Image.new('RGBA', img.size, (0, 0, 0, 255))
    Image.alpha_composite(bg, img).convert('RGB').save(OUT / (name + '.png'))   # premultiplied: RGB over black is the picture
    img.save(OUT / (name + '_raw.png'))
    ui = session.plugin.ui_runtime()
    json.dump(ui, open(OUT / ('ui_' + name + '.json'), 'w'))
    print('saved', name, img.size)


try:
    client = session.open()
    assert session.plugin_ok, 'plugin missing'
    session.plugin.pacing(True)
    client.load_world(SAVE.absolute())
    client.set_time(paused=True)
    tick(client, 0.6)
    for sid in IDS:
        res = pj(session.plugin, '/d/open_species', {'id': sid})
        print('open', sid, res.get('ok'), res.get('opened'), res.get('error'), res.get('count'))
        tick(client, 1.5)
        shot('tree_%d' % sid)
        print(json.dumps(pj(session.plugin, '/d/press_button', {'path': SCENE + 'Top Bar/TopRight/MeshButton'}))[:160])
        tick(client, 2.0)
        shot('mesh_%d' % sid)
        print(json.dumps(pj(session.plugin, '/d/press_button', {'path': SCENE + 'Top Bar/TopRight/TreeButton'}))[:160])
        tick(client, 1.0)
finally:
    session.close()

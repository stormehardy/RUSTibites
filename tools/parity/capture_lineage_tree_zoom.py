# Captures the hidden game's lineage TREE at given zooms (overlay png over black + /d/ui dump in C:\Temp\lineage_out).
# usage: capture_lineage_tree_zoom.py <save.zip> <zoom> [more zooms]. Needs the ParityDump plugin in BepInEx\plugins (remove it afterwards).
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
ZOOMS = [float(v) for v in sys.argv[2:]] or [1.0]
import os
SPECIES = int(os.environ.get('LINEAGE_SPECIES', '0'))   # a species id: the panel opens on it (its node is selected); 0: the panel opens with none


def tick(client, sec=0.5):
    client.set_time(paused=False); time.sleep(sec); client.set_time(paused=True)


def shot(name):
    ov = pj(session.plugin, '/d/ui_overlay')
    img = Image.open(io.BytesIO(base64.b64decode(ov['png_base64']))).convert('RGBA')
    bg = Image.new('RGBA', img.size, (0, 0, 0, 255))
    Image.alpha_composite(bg, img).convert('RGB').save(OUT / (name + '.png'))
    json.dump(session.plugin.ui_runtime(), open(OUT / ('ui_' + name + '.json'), 'w'))
    print('saved', name)


try:
    client = session.open()
    assert session.plugin_ok, 'plugin missing'
    session.plugin.pacing(True)
    client.load_world(SAVE.absolute())
    client.set_time(paused=True)
    tick(client, 0.6)
    if SPECIES:
        print(json.dumps(pj(session.plugin, '/d/open_species', {'id': SPECIES}))[:100])
    else:
        print(json.dumps(pj(session.plugin, '/d/press_button', {'path': 'UICanvas/MainUI/Layer0/UI Buttons/TopRight/AncestralLineageButton'}))[:100])
    tick(client, 1.5)
    for z in ZOOMS:
        r = pj(session.plugin, '/d/invoke', {'path': 'AncestralLineagePanel/LineageView', 'type': 'LineageWindow', 'method': 'ZoomToScale', 'args': [z]})
        print('zoom', z, json.dumps(r)[:140])
        tick(client, 1.0)
        shot('tree_zoom_%g_sp%d' % (z, SPECIES))
finally:
    session.close()

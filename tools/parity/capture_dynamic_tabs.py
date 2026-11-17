# Captures the hidden game's dynamic settings panel with the Zones and Bibite tabs open (overlay png + /d/ui dump in C:Temphud_out). Needs the ParityDump plugin in BepInExplugins (remove it afterwards).
import sys, json, io, base64, time
from pathlib import Path
sys.path.insert(0, r'Z:\RUSTibites\tools\parity')
import argparse
import capture_hud_reference as h
from PIL import Image
cap = h.cap
OUT = Path(r'C:\Temp\hud_out'); OUT.mkdir(exist_ok=True)
BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(Path(r'C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI'))
session = cap.WorldSession(h.build_args(argparse.Namespace()), BridgeClient, LiveRuntime, Lab)
pj = h.plugin_json
BASE = 'UICanvas/MainUI/Layer1/Panels/Right Panels/DynamicSettingsPanel/TabButtons/'


def tick(client, sec=0.5):
    client.set_time(paused=False); time.sleep(sec); client.set_time(paused=True)


def shot(name):
    ov = pj(session.plugin, '/d/ui_overlay')
    img = Image.open(io.BytesIO(base64.b64decode(ov['png_base64']))).convert('RGBA')
    img.convert('RGB').save(OUT / (name + '.png'))
    ui = session.plugin.ui_runtime()
    json.dump(ui, open(OUT / ('ui_' + name + '.json'), 'w'))
    print('saved', name, img.size)


try:
    client = session.open()
    assert session.plugin_ok, 'plugin missing'
    session.plugin.pacing(True)
    client.load_world(h.SAVE.absolute())
    client.set_time(paused=True)
    tick(client, 0.6)
    print(json.dumps(pj(session.plugin, '/d/press_button', {'name': 'DynamicParametersButton'}))[:100])
    tick(client, 0.8)
    print(json.dumps(pj(session.plugin, '/d/press_button', {'path': BASE + 'ZonesTabButton '}))[:100])
    tick(client, 0.8)
    shot('zones_tab')
    print(json.dumps(pj(session.plugin, '/d/press_button', {'path': BASE + 'BibiteTabButton'}))[:100])
    tick(client, 0.8)
    shot('bibite_tab')
finally:
    session.close()

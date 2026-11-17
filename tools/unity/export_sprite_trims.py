"""Records how the exported UI sprite PNGs are trimmed relative to the game's sprites.

UnityPy exports a `Sprite` with a tight mesh as its bounding rectangle (`Sprite.image`), dropping the
transparent margin that Unity keeps: a 35x35 sprite whose artwork is 28x22 is exported as a 28x22 PNG.
Unity draws the *whole* sprite rect (margins included) into the Image's rect, so stretching the trimmed
PNG over that rect enlarges the artwork (and shifts 9-slice borders). This script reads each sprite's
`rect` and `textureRectOffset` from the game's assets and writes
`assets/game/reference/functional_ui_native/sprite_trims.json`, which the viewer uses to pad the PNGs
back to the sprite rect when it loads them.

Usage: python tools/unity/export_sprite_trims.py [game_data_dir] [out.json]
Read-only on the game install; run from the repository root.
"""
import glob
import json
import os
import sys

import UnityPy
from PIL import Image

data_dir = sys.argv[1] if len(sys.argv) > 1 else r'C:\Users\Storme\TheBibites\The Bibites_Data'
out_path = sys.argv[2] if len(sys.argv) > 2 else 'assets/game/reference/functional_ui_native/sprite_trims.json'

# name -> list of (asset, rect w/h, texture rect w/h, offset x/y)
sprites = {}
def normal(name):
    """File-name form of a sprite name (the exporter replaces spaces and appends .png to some)."""
    name = name.replace(' ', '_')
    return name[:-4] if name.lower().endswith('.png') else name


assets = sorted(glob.glob(os.path.join(data_dir, 'sharedassets*.assets')))
assets += sorted(glob.glob(os.path.join(data_dir, 'resources.assets')))
assets += sorted(f for f in glob.glob(os.path.join(data_dir, 'level*')) if not f.endswith(('.resS', '.resource', '.json')))
for asset in assets:
    try:
        env = UnityPy.load(asset)
    except Exception:
        continue
    base = os.path.basename(asset)
    for obj in env.objects:
        if obj.type.name != 'Sprite':
            continue
        try:
            tree = obj.read_typetree()
        except Exception:
            continue
        rd = tree.get('m_RD', {})
        rect, tex, off = tree.get('m_Rect', {}), rd.get('textureRect', {}), rd.get('textureRectOffset', {})
        if not rect or not tex:
            continue
        sprites.setdefault(normal(tree['m_Name']), []).append({
            'asset': base,
            'path_id': obj.path_id,
            'full': [rect['width'], rect['height']],
            'tex': [tex['width'], tex['height']],
            'offset': [off.get('x', 0.0), off.get('y', 0.0)],
        })
print('sprites read:', sum(len(v) for v in sprites.values()))

png_files = []
for pattern in (
    'assets/game/ui/sharedassets*.assets/sprites/*.png',
    'assets/game/reference/functional_ui_native/sprites/*.png',
    'assets/game/reference/functional_ui_native/in_world/button_theme/sprites/*.png',
):
    png_files.extend(sorted(glob.glob(pattern)))

native = {}
try:
    manifest = json.load(open('assets/game/reference/functional_ui_native/sprite_manifest.json', encoding='utf-8'))
    for name, entry in manifest.get('sprites', {}).items():
        native[entry['file'].replace('\\', '/')] = (entry.get('source'), entry.get('path_id'))
except OSError:
    pass

trims = {}
unmatched = []
for path in png_files:
    key = path.replace('\\', '/')
    name = normal(os.path.splitext(os.path.basename(path))[0])
    width, height = Image.open(path).size
    candidates = sprites.get(name, [])
    wanted = native.get(key)
    chosen = None
    if wanted:
        chosen = next((c for c in candidates if c['asset'] == wanted[0] and c['path_id'] == wanted[1]), None)
    if chosen is None:
        asset_dir = key.split('/')[3] if key.startswith('assets/game/ui/') else None
        pool = [c for c in candidates if asset_dir is None or c['asset'] == asset_dir] or candidates
        # the exported PNG is the texture rect rounded up
        fit = [c for c in pool if abs(round(c['tex'][0]) - width) <= 1 and abs(round(c['tex'][1]) - height) <= 1]
        chosen = (fit or pool or [None])[0]
    if chosen is None:
        unmatched.append(key)
        continue
    full_w, full_h = round(chosen['full'][0]), round(chosen['full'][1])
    if (width, height) == (full_w, full_h):
        continue  # not trimmed
    off_x = chosen['offset'][0]
    # Unity's rect origin is bottom-left; PNG rows run top-down.
    off_y_top = chosen['full'][1] - (chosen['offset'][1] + chosen['tex'][1])
    trims[key] = {'full': [full_w, full_h], 'offset': [round(off_x, 3), round(off_y_top, 3)], 'size': [width, height]}

with open(out_path, 'w', encoding='utf-8') as handle:
    json.dump({'schema': 'rustibites.ui.sprite_trims.v1', 'trims': trims}, handle, indent=1, sort_keys=True)
print(len(png_files), 'PNGs;', len(trims), 'trimmed;', len(unmatched), 'unmatched')
for key in unmatched[:20]:
    print('  unmatched', key)

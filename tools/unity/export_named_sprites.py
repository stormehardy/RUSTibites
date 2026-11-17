"""Exports named Unity sprites of the game's asset files as PNGs (read-only on the game install).

Usage: python tools/unity/export_named_sprites.py <out dir> <sprite name> [<sprite name> ...]
       [--data <game data dir>]   (default C:\\Users\\Storme\\TheBibites\\The Bibites_Data)

The first sprite of each name that is found is saved as `<out dir>/<name>.png` (`Sprite.image`: a sprite with a
tight mesh is exported as its bounding rectangle, so run `tools/unity/export_sprite_trims.py` afterwards to
record how the PNGs sit inside their sprite rects) and its rect, pixels per unit, pivot and 9-slice border are
printed. The lineage tree's sprites (`lineage-*`) are exported with:

    python tools/unity/export_named_sprites.py assets/game/reference/functional_ui_native/lineage \
        lineage-vertical lineage-horizontal lineage-elbow-right lineage-elbow-left lineage-junction-left \
        lineage-junction-right lineage-arrowhead lineage-template lineage-termination lineage-node-small
"""
import glob
import os
import sys

import UnityPy

args = sys.argv[1:]
data_dir = r'C:\Users\Storme\TheBibites\The Bibites_Data'
if '--data' in args:
    i = args.index('--data')
    data_dir = args[i + 1]
    del args[i:i + 2]
out_dir, wanted = args[0], set(args[1:])
os.makedirs(out_dir, exist_ok=True)

found = {}
assets = sorted(glob.glob(os.path.join(data_dir, 'sharedassets*.assets'))) + [os.path.join(data_dir, 'resources.assets')]
for asset in assets:
    if not os.path.exists(asset) or len(found) == len(wanted):
        continue
    try:
        env = UnityPy.load(asset)
    except Exception as error:  # noqa: BLE001
        print('skip', asset, error)
        continue
    for obj in env.objects:
        if obj.type.name != 'Sprite':
            continue
        try:
            sprite = obj.read()
        except Exception:  # noqa: BLE001
            continue
        name = sprite.m_Name
        if name not in wanted or name in found:
            continue
        image = sprite.image
        path = os.path.join(out_dir, name + '.png')
        image.save(path)
        tree = obj.read_typetree()
        rect = tree.get('m_Rect', {})
        found[name] = path
        print(f"{name}: {image.width}x{image.height} png, rect {rect.get('width')}x{rect.get('height')}, "
              f"ppu {tree.get('m_PixelsToUnits')}, pivot {tree.get('m_Pivot')}, border {tree.get('m_Border')}, "
              f"from {os.path.basename(asset)}")
missing = sorted(wanted - set(found))
if missing:
    print('not found:', ', '.join(missing))

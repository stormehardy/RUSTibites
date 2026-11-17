"""Records `Image.pixelsPerUnitMultiplier` of every UI Image that is not 1.

The runtime UI captures list each Image's sprite, border and type but not its
`pixelsPerUnitMultiplier`, which scales the drawn 9-slice borders (a button with 0.5 draws its frame
twice as thick). The value is the last float of the serialized `Image` MonoBehaviour. This script
walks the game's scenes and prefabs, and writes
`assets/game/reference/functional_ui_native/image_multipliers.json`:

* `scene`: full hierarchy path (as in the captures) -> multiplier, for objects placed in a scene;
* `prefab`: `PrefabName/relative/path` -> multiplier, for prefab contents (captured instances are
  named `PrefabName(Clone)`; the part below that node is the relative path).

Usage: python tools/unity/export_image_multipliers.py [game_data_dir] [out.json]
Read-only on the game install; run from the repository root.
"""
import glob
import json
import os
import struct
import sys

import UnityPy

data_dir = sys.argv[1] if len(sys.argv) > 1 else r'C:\Users\Storme\TheBibites\The Bibites_Data'
out_path = sys.argv[2] if len(sys.argv) > 2 else 'assets/game/reference/functional_ui_native/image_multipliers.json'

IMAGE_LEN = 136  # MonoBehaviour header (32) + serialized Image fields (Unity UI 1.0.0)


def image_multiplier(raw):
    """The multiplier of an `Image` MonoBehaviour, or None when `raw` is not one."""
    if len(raw) != IMAGE_LEN:
        return None
    # m_Color (4 floats) at 44..60, m_Type at 100, multiplier = last float
    colour = struct.unpack_from('<4f', raw, 44)
    if not all(-0.01 <= c <= 1.01 for c in colour):
        return None
    image_type = struct.unpack_from('<i', raw, 100)[0]
    if image_type not in (0, 1, 2, 3):
        return None
    value = struct.unpack_from('<f', raw, IMAGE_LEN - 4)[0]
    if not (0.01 < value < 100.0):
        return None
    return value


scene, prefab = {}, {}
files = sorted(glob.glob(os.path.join(data_dir, 'level*')))
files = [f for f in files if not f.endswith(('.resS', '.resource', '.json'))]
files += sorted(glob.glob(os.path.join(data_dir, 'sharedassets*.assets')))
for path in files:
    try:
        env = UnityPy.load(path)
    except Exception as error:
        print('skip', path, error)
        continue
    base = os.path.basename(path)
    is_scene = base.startswith('level')
    objects = {obj.path_id: obj for obj in env.objects}
    transforms = {}   # transform id -> (game object id, father id)
    names = {}        # game object id -> name
    for obj in objects.values():
        if obj.type.name in ('RectTransform', 'Transform'):
            try:
                tree = obj.read_typetree()
            except Exception:
                continue
            transforms[obj.path_id] = (tree['m_GameObject']['m_PathID'], tree['m_Father']['m_PathID'])
        elif obj.type.name == 'GameObject':
            try:
                names[obj.path_id] = obj.read_typetree()['m_Name']
            except Exception:
                pass
    by_go = {go: tid for tid, (go, _) in transforms.items()}

    def full_path(go_id):
        parts = []
        guard = 0
        tid = by_go.get(go_id)
        while tid is not None and guard < 64:
            go, father = transforms[tid]
            parts.append(names.get(go, '?'))
            tid = father if father in transforms else None
            guard += 1
        return list(reversed(parts))

    count = 0
    for obj in objects.values():
        if obj.type.name != 'MonoBehaviour':
            continue
        try:
            raw = bytes(obj.get_raw_data())
        except Exception:
            continue
        value = image_multiplier(raw)
        if value is None or abs(value - 1.0) < 1e-4:
            continue
        go_id = struct.unpack_from('<q', raw, 4)[0]
        parts = full_path(go_id)
        if not parts:
            continue
        count += 1
        if is_scene:
            scene['/'.join(parts)] = round(value, 4)
        else:
            # prefab roots have no father; the capture names instances `Root(Clone)`
            prefab['/'.join(parts)] = round(value, 4)
    print(base, count, 'images with a multiplier != 1')

with open(out_path, 'w', encoding='utf-8') as handle:
    json.dump({'schema': 'rustibites.ui.image_multipliers.v1', 'scene': scene, 'prefab': prefab}, handle, indent=1, sort_keys=True)
print('scene entries', len(scene), 'prefab entries', len(prefab))

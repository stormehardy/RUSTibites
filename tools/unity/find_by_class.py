"""Lists the GameObjects that carry a MonoBehaviour of the given script classes in a scene (read-only).

The release build strips MonoBehaviour typetrees, so the script class is found through the `m_Script` PPtr at byte 16 of the raw
component and the owner through the `m_GameObject` PPtr at byte 0. `find_objects.py` finds GameObjects by NAME; this one finds them by
the C# class that is on them, which is how a panel's root is found from its `Here/` source (`GraphsPanel` -> `StatsPanel`, path id 739).

Usage: python tools/unity/find_by_class.py <game data dir> <level file> <ScriptClass> [<ScriptClass> ...]
Example: python tools/unity/find_by_class.py "C:/Users/<you>/TheBibites/The Bibites_Data" level2 GraphsPanel BarGraph

Prints `<class> mono <MonoBehaviour path id> on GO <GameObject path id> <name>`; feed the GameObject id to `dump_ui_tree.py` (hierarchy,
geometry, images, texts) or `dump_ui_components.py` (the raw values of the layout groups, layout elements, text and shapes below it).
"""
import os
import struct
import sys

import UnityPy

data_dir, level, wanted = sys.argv[1], sys.argv[2], set(sys.argv[3:])
env = UnityPy.load(os.path.join(data_dir, level))
objects = {o.path_id: o for o in env.objects}
external = {}
cache = {}


def target(owner, file_id, path_id):
    if file_id == 0:
        return objects.get(path_id)
    name = os.path.basename(owner.assets_file.externals[file_id - 1].path)
    if name not in external:
        external[name] = {o.path_id: o for o in UnityPy.load(os.path.join(data_dir, name)).objects}
    return external[name].get(path_id)


for obj in env.objects:
    if obj.type.name != 'MonoBehaviour':
        continue
    raw = obj.get_raw_data()
    key = struct.unpack_from('<iq', raw, 16)
    if key not in cache:
        try:
            script = target(obj, *key)
            cache[key] = '?' if script is None else script.read().m_ClassName
        except Exception:  # noqa: BLE001
            cache[key] = '?'
    name = cache[key]
    if name in wanted:
        _, go_id = struct.unpack_from('<iq', raw, 0)
        go = objects.get(go_id)
        print(name, 'mono', obj.path_id, 'on GO', go_id, go.read_typetree()['m_Name'] if go else '?')

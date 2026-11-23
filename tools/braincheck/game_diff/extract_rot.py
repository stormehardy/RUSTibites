"""extract_rot.py <dir>: writes rot_KKKK.json ({body.id: rb2d.r}) next to every world_KKKK.zip in <dir>.

The save holds each bibite's rotation twice: transform.rotation (degrees wrapped to 0..360, from the quaternion) and rb2d.r (the physics angle in degrees, unwrapped).
sense_vs_game starts the port from rb2d.r, which is the closest thing to the game's own radians."""
import json
import sys
import zipfile
from pathlib import Path

d = Path(sys.argv[1])
n = 0
for z in sorted(d.glob('world_*.zip')):
    out = {}
    with zipfile.ZipFile(z) as zf:
        for name in zf.namelist():
            if name.startswith('bibites/'):
                data = json.loads(zf.read(name))
                out[str(data['body']['id'])] = data['rb2d']['r']
    (d / z.name.replace('world_', 'rot_').replace('.zip', '.json')).write_text(json.dumps(out), encoding='utf-8')
    n += 1
print('wrote', n, 'files')

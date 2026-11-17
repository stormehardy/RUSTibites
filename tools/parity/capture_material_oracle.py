"""The Biology panel's two UI programs as the game's GPU runs them: the fixtures `internals_gpu_maps.txt` and `internals_gpu_preview.bin`
(crates/bibites-view/src) that `internals::tests` compare the translated programs (`internals_shaders.rs`) with.

The parity plugin (tools/parity/plugin, copy `ParityDump.dll` into the Steam install's BepInEx/plugins first and remove it afterwards) has
`POST /d/blit_material`: a copy of a UI panel's material (`BiologyPanel.sections_mat` by default, `interns` for the preview) with the given
properties set (`_Name` floats, `_Name.r|g|b|a` colour parts; Int properties such as `_Width` are set as integers), drawn into a cleared
render texture exactly as `GenerateInternalsMap` does (`Graphics.Blit` + `ReadPixels`) and read back (`red_hex`, or `rgba_hex` with `rgba`).

    python tools/parity/capture_material_oracle.py <out dir>

writes `internals_gpu_maps.txt` and `internals_gpu_preview.bin` there. (A pooled render texture keeps what the last user drew and the
UI programs blend, so the plugin clears it first; without that the pictures were unions of earlier ones.)
"""
import argparse
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import capture_hud_reference as h  # noqa: E402

cap = h.cap
W, H = 61, 39
PORTIONS = ['_WombPortion', '_ThroatPortion', '_StomachPortion', '_MusclesPortion', '_JawPortion', '_ArmorPortion']
NAMES = {'_InflateFactor': 'inflate', '_WombPortion': 'womb', '_ThroatPortion': 'throat', '_StomachPortion': 'stomach',
         '_MusclesPortion': 'muscles', '_JawPortion': 'jaw', '_ArmorPortion': 'armor'}


def main():
    out_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('.')
    out_dir.mkdir(parents=True, exist_ok=True)
    api = Path(r'C:\Users\Storme\Desktop\Projects\BibitesInfo\BibitesAPI')
    BridgeClient, BridgeError, LiveRuntime, Lab = cap.import_api(api)
    session = cap.WorldSession(h.build_args(argparse.Namespace()), BridgeClient, LiveRuntime, Lab)
    pj = h.plugin_json

    def blit(field, w, h_, **props):
        body = {k: float(v) for k, v in props.items()}
        body.update(field=field, w=w, h=h_, rgba=1)
        r = pj(session.plugin, '/d/blit_material', body)
        if not r.get('ok'):
            raise RuntimeError(r)
        return r['rgba_hex']

    try:
        client = session.open()
        assert session.plugin_ok, 'the parity plugin is not loaded'
        session.plugin.pacing(True)
        client.load_world(h.SAVE.absolute())
        client.set_time(paused=True)
        # the Biology panel (and so its materials) exists once a bibite is selected and the panel opened
        pj(session.plugin, '/d/select_bibite', {'x': -5.360003, 'y': 831.0031})
        pj(session.plugin, '/d/press_button', {'name': 'BiologyButton'})
        time.sleep(0.5)
        client.set_time(paused=False)
        time.sleep(0.3)
        client.set_time(paused=True)

        zero = {p: 0.0 for p in PORTIONS}
        live = {'_InflateFactor': 0.0, '_WombPortion': 0.190816328, '_ThroatPortion': 0.099999994, '_StomachPortion': 0.381632656,
                '_MusclesPortion': 0.049999997, '_JawPortion': 0.08163266, '_ArmorPortion': 0.005102041}
        map_cases = [dict(zero, _InflateFactor=0.0), dict(zero, _InflateFactor=0.5), dict(zero, _InflateFactor=0.97)]
        for p in ('_WombPortion', '_ThroatPortion', '_StomachPortion', '_MusclesPortion', '_JawPortion', '_ArmorPortion'):
            map_cases.append(dict(zero, **{p: 0.3}))
        map_cases.append(dict(zero, _InflateFactor=0.5, _StomachPortion=0.3))
        lines = ['# The organ map (UI/InternalsSections, 61 x 39 cells, one digit per cell: 0 outside .. 7 fat) the game\'s GPU makes for these material values,',
                 '# read back from a hidden game with the parity plugin\'s `POST /d/blit_material` (tools/parity/capture_material_oracle.py); the top row comes first.',
                 '# `internals::tests::the_organ_map_is_what_the_games_gpu_makes` runs the translated pixel program (`internals_shaders.rs`) over the same values.']

        def map_rows(hexs):
            rows = []
            for y in range(H):
                rows.append(''.join(str(round(int(hexs[(x + y * W) * 8:(x + y * W) * 8 + 2], 16) / 255 * 7)) for x in range(W)))
            return list(reversed(rows))

        def head(props):
            keys = sorted(NAMES, key=list(NAMES).index)
            return 'case ' + ' '.join('%s=%r' % (NAMES[k], round(float(props.get(k, 0.0)), 9)) for k in keys)

        for props in map_cases:
            lines.append(head(props))
            lines.extend(map_rows(blit('sections_mat', W, H, **props)))
        # the map the game itself generated for the selected bibite (`internsMap`), with the material values it was generated from
        mat = pj(session.plugin, '/d/panel_material', {'field': 'sections_mat'})
        values = {pr['name']: pr['float'] for pr in mat['properties']}
        own = pj(session.plugin, '/d/panel_ints', {'field': 'internsMap'})['map']
        lines.append(head(values))
        lines.extend(reversed([own[y * W:(y + 1) * W] for y in range(H)]))
        (out_dir / 'internals_gpu_maps.txt').write_text('\n'.join(lines) + '\n', encoding='utf-8', newline='\n')

        # the preview material: the live values (the colour is the material's own), full fat, inflated, and a narrow fat range on grey
        fat = {'_FatFullness': 0.0259575658, '_FatStart': 0.8196721, '_FatEnd': 0.196721315}
        grey = {'_Color.r': 0.5, '_Color.g': 0.5, '_Color.b': 0.5, '_Color.a': 1.0}
        previews = [dict(live, **fat), dict(live, **dict(fat, _FatFullness=1.0)),
                    dict(zero, _InflateFactor=0.0, _FatFullness=0.5, _FatStart=0.6, _FatEnd=0.4, **grey),
                    dict(live, **dict(fat, _FatFullness=0.5), _InflateFactor=0.5)]
        blob = bytearray()
        for props in previews:
            blob += bytes.fromhex(blit('interns', W, H, **props))
        (out_dir / 'internals_gpu_preview.bin').write_bytes(bytes(blob))
        print('wrote', out_dir)
    finally:
        session.close()


if __name__ == '__main__':
    main()

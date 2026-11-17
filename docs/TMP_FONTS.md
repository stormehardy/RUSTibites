# TMP font assets (game-exact SDF)

Extracted via the standalone hidden game plugin route `GET /d/font_assets` (TMP_FontAsset reflection:
faceInfo, characterTable unicode→glyph, glyphTable atlas rect/metrics/scale, kerning, material params, atlas
textures) plus UnityPy extraction of atlas PNGs from the game's `Texture2D` assets.

Per font: `assets/game/ui_fonts/<Font>.json`; atlases: `atlas_<texture>.png`; index:
`assets/game/ui_fonts/fonts_manifest.json`; runtime report: `fonts_report.json`.

| font | glyphs/chars | atlas |
|---|---|---|
| Boston Black SDF | 97/97 | atlas_Boston_Black_SDF_Atlas.png (512×512) |
| LiberationSans SDF | 250/250 | atlas_LiberationSans_SDF_Atlas.png (1024×1024) |
| LiberationSans SDF - Fallback | 0/0 | atlas_LiberationSans_SDF_Atlas.png (1024×1024) |
| Roboto-Bold SDF NoOutline | 112/112 | atlas_Roboto-Bold_Atlas.png (1024×1024) |
| Roboto-Bold SDF WhiteOutline | 98/98 | atlas_Roboto-Bold_SDF_Atlas.png (512×512) |
| Roboto-Regular SDF | 98/98 | atlas_Roboto-Regular_SDF_Atlas.png (512×512) |

Notes:
- `LiberationSans SDF`, `Roboto-Regular SDF`, `Boston Black SDF` are exact game assets.
- Roboto-Bold exists as `Roboto-Bold SDF NoOutline` and `Roboto-Bold SDF WhiteOutline` variants (no plain
  `Roboto-Bold SDF` asset was loaded in the menu).
- `joystix_monospace SDF` glyph/face JSON was not loaded in the menu session; its atlas PNG is extracted and
  listed in `atlas_files` for completeness (no face/glyph table available from that live session).
- Material params include `_FaceDilate`, `_OutlineWidth`, `_OutlineSoftness`, `_GradientScale`, scale ratios,
  underlay offsets/dilate/softness, `_FaceColor`, `_OutlineColor`, shader, and `_MainTex` texture name.
- Native menu UI-only isolation remains blocked: excluding Title Screen/BackGround yields an opaque frame;
  disabling the `UI/Popuholder/screenBlocker` plus fullscreen Graphic suppression yielded a transparent-but-empty
  overlay (alpha 0). World UI-only native refs are usable; menu refs remain full/opaque.

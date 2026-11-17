# Native 1920 UI reference probe

Standalone hidden launch of The Bibites with `-screen-width 1920 -screen-height 1080 -window-mode windowed`
(not BibitesAPI's 320x240 hidden launcher), BepInEx bridge/plugin, native 1920 captures. Sharpness metrics in
`probe_report.json`/`manifest.json` confirm native pixels (title/settings full overlays unique colors ~55k/56k,
down/up residual ~0.11; world UI-only 347 colors). The world overlay is transparent and suitable for A/B. Menu
overlays are currently opaque because the Title Screen/BackGround composition covers UI; see manifest notes.

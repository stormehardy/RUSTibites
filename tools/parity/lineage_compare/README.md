# Lineage panel: game vs port (2026-11-11)

Capture the game (hidden, with the ParityDump plugin in BepInEx\plugins, removed afterwards):
- `tools/parity/capture_lineage.py <save> <species id>`: tree and mesh with a species selected.
- `tools/parity/capture_lineage_mesh.py <save> 0 [sim seconds]`: the panel opened from the HUD button with NO species selected, then the Mesh button.
  (A selection focuses the mesh on its centre, which is NaN in a hidden game and hides the whole mesh; use a save the GAME wrote, such as
  `testdata/real/saves/lol.zip`: a port-written save has `maxEnergy` 0 and the game's mesh widths come out NaN.)
- `tools/parity/capture_lineage_tree_zoom.py <save> <zoom>` (`LINEAGE_SPECIES=<id>` for a selection): the tree at a zoom (`LineageWindow.ZoomToScale` through `/d/invoke`).
The port: `rustibites-view <save> --no-menu --width 1920 --height 1080 --frames 5 --lineage-species N [--lineage-mesh] [--lineage-zoom Z] --ui-only --screenshot x.png`
(the lineage flags also work in the real window now, with `RUSTIBITES_SELF_SHOT=<frame>:<png>`).
Compare: `over_black_one.py` (premultiplied UI-only render over black), then `cmp_region.py game.png port.png x0,y0,x1,y1 [mask.png]` (mean difference, best shift),
`crop_two.py` (game over port, upscaled). `mesh_nodes.py` / `node_rect.py` / `mesh_labels.py` read the game's `/d/ui` dumps.
Paths inside are the session's (`C:\Temp\...`).

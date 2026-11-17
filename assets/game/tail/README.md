# Tail deformation reference (diagnostic)

The Tail is **not a mesh**: it is a shader deformation of the Body SpriteRenderer driven by
`ProceduralSpriteController` (`_MiddleSegmentAngle`/`_TailAngle`, plus `_ArmAngles`/`_MouthOpenning`/`_Size`).
`procedural_bibite_material.json` is the live Body material property dump. The attempted angle sweep
(`assets/game/reference/default_save/tail_sweep/`) is diagnostic: the shipped shader does not expose the
tail property IDs to `Shader.GetPropertyName`/`Material` (21 props only), and setting them in a paused hidden
load produced byte-identical frames, so the per-frame alpha bboxes are not usable as a fitted warp target.
No tail mesh assets exist.

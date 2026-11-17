# Single-load closeup reference (task-141)

One hidden load of `testdata/parity/default/rust_seed_600.zip`, paused at t=20 s, one `/d/bibite_layers`
query and an exact 1280×720 ortho-size-12 closeup at the selected bibite's position. All fields in
`reference_single.json` are atomically consistent with `reference_single.png` (same paused state):
selected runtime id/pose, full 35-gene row + game-save genome, per-layer alpha bounds/world rects, live
texture region counts, composited world bbox, expected and visible pixel bboxes, camera and 30 px/world
conversion. `verification_moved.png` is the same load after moving the selected handle offscreen; its diff
against the reference gives the visible alpha bbox (441×485 px in this capture), confirming the selected
entity is the one at the camera centre. The save has 10 bibites; the game load adds the virgin spawner's
one extra bibite (11 live ids), so runtime ids can change per load; the recorded id is the saved entity
nearest the canonical closeup camera, not the spawner extra. No simulation advanced between the layer
query, reference screenshot and verification move.


## Per-layer visible-alpha (task-142)

`reference_layers.json` + `layers/` capture the same atomic closeup one hidden load: all renderers off,
then each of the six `SpriteRenderer`s alone; each `layers/<name>_<i>.png` is the target-only frame and
`*_alpha.png` the visible alpha mask. 1280×720, ortho size 12, 30 px/world. Selected id/pose/genome and
camera are recorded atomically; renderers are restored (verify diff empty). Per-layer visible-alpha pixel
bboxes in that capture: full `(459,177,900,662)` = **441×485**; Body `(521,261,897,662)` 77,307 px;
Mouth `(459,177,759,459)`; Eyes `(503,230,732,438)`; RightArm `(759,238,900,343)`; Left Arm
`(495,460,583,609)`; Exoskeleton blank. **The y≈662 bottom extent is the Body layer** (its visible alpha
extends below the captured world AABB), not an extra/inactive renderer — all child SpriteRenderers were
enumerated and the six named layers are the complete set. Bench should compare/fix the Body sprite
mapping/art against `layers/Body_2.png`/`Body_2_alpha.png`.

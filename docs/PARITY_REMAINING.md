# What is still not at parity (consolidated, 2026-11-03)

## Every screen at any window shape or UI scale ("make everything look the same on all resolutions", 2026-11-21: DONE for the captured screens, both editions; the real window was not driven)

The game's canvas is the window over `UIScaler.totalScale` (`min(h / 1080, w / 1920)` x UI Scale): a 16:9 window at Normal is the 1920 x 1080 the captures were made at, any other shape or scale is a canvas of another size, and the game's `RectTransform` anchors and layout groups then give every panel another place and size (the settings pages grow wider, the scenario list takes the width the window has more, a taller window stretches panels and gives lists room for more rows). The viewer drew the captured 1080p rectangles, moved by one anchor's share of the difference: panels moved but never grew, and every title screen but the first one stayed in the top left.
- **`ui_layout.rs` solves the layout the way `UnityEngine.UI` does**, as a change from the captured solution: anchors / pivots (point anchors keep their offset, stretched ones their margins), horizontal / vertical layout groups (the flexible children share what the size gained; groups with nothing flexible move by their alignment), grid groups, content size fitters, aspect ratio fitters, and the `ScrollRect` `AutoHideAndExpandViewport` rule (a list that fits its taller view has no scroll bar and its viewport takes the bar's room). A 1920 x 1080 canvas changes nothing (the capture is right as it is). Every screen of the title menu and the in-world panels that come from the capture go through it (`FrameContext::relaid`).
- **Measured against the original game itself**: the game was run hidden at 2560 x 1080 and 1920 x 1200 (windowed, resized through the plugin) and every node's rectangle written down (`assets/game/reference/layout_gt`, 126 KB; `tools` of the session: `capture_ui_res.py`). Static nodes of the 60 captured screens within 1.5 px of the game's: 98.9 % at 2560 x 1080 and 94.9 % at 1920 x 1200 in the prototype, and the test `ui_runtime::tests::the_layout_in_a_window_of_another_shape_is_the_original_games` (at least 93 % overall and 75 % on every title-menu screen, dynamic nodes - graph ticks, status bars, the credits' auto-scroll, the title canvas - left out) keeps it.
- **The menu's lists follow their screen** (`MenuState::list_delta`, set from the layout when a frame is built): the scenario, Load and Library lists, the Settings pages, the Credits and Additional Tools tabs, their scroll bars, the wheel's and the drag's limits, the rows' places and the mask that cuts the Tools / Credits content, and the New Game screen's TPS settings pop-up (`tps_delta`). `ui_runtime::tests::the_menus_scroll_bar_follows_the_scroll_view_of_the_window` has the game's own numbers (the bar of the Graphics page is gone at 1920 x 1200 and at x 1711.43 at 2560 x 1080).
- **Not covered**: the in-world settings panel (`dyn_panel`) and the scenario editor are made of nodes the viewer generates, which no capture has a layout for: they keep the old composition (flush right, nothing moves down). The game stretches the in-world panel taller in a taller window (its tabs keep their distance from the top, its bottom reaches the window's); the port's stays 1000 high. A UI scale above 1 can need scroll bars that were hidden in the capture (only the hiding direction is modelled), and a text that wraps differently in another width is not re-measured. The captures at 3440 x 1440 and 1280 x 720 could not be used as ground truth (`screen_bbox` mixes pixels and canvas units when the UI scale is not 1). Heavy `ui_runtime::` tests: Faithful 93 of 118 ran in the last pass (the rest were stopped by the 1.9 GB free-RAM floor of this machine, not failed), Improved 100 of 124; the layout, menu and settings-panel tests and the 798 / 946 quick-set tests all passed.

## Child brains start fresh ("some output nodes output a constant even with the synapse disabled and no bias", PhereOut and Herding, reported 2026-11-19: FIXED in both editions, unit-tested, mechanism read from the game's code and checked on its saves)

**What the game does.** A brain built for a new bibite or egg is `NEATBrain.CopyBrain(parent nodes, parent synapses, mutate)`: `CopyNode` copies only a node's structure (type, innovation, bias, ...), never its `Value`, `LastInput` or `LastOutput`; `FinalizeEditing` then runs every non-input node ONCE with input 0, so a node holds `f(0 + bias)`
(`NEATBrain.cs:458`). `Thinker` runs only the nodes some enabled synapse ends in (`brainTargets`), so a node with no enabled incoming synapse keeps that value for the bibite's whole life: `f(bias)`, which is 0 for TanH/ReLU/Linear/Abs/Sine without a bias, 0.5 for a Sigmoid and 1 for a Gaussian. Callers of `CopyBrain`: laying an egg,
hatching, `WorldObjectsSpawner` (templates), the floating-hidden repair. Measured on the original game's own saves (the shrek save: 622 evolved bibites, 4,881 nodes without an enabled incoming synapse): every one holds exactly `f(bias)`, none holds anything else.

**What the port did.** The egg layer cloned the parent's RUNNING brain and mutated the clone (`record.brain.clone()` then `mutate`), a hatched egg and a template spawn kept whatever node values the brain carried. A child therefore started with its parent's node values, and a node its mutation (or the Bibite Editor's `set_synapse_enabled`) left unfed kept the parent's last output for its whole life:
a constant pheromone output with the synapse disabled and no bias, a constant herding output, and any other node (hidden ones feed their `LastOutput` into the nodes behind them). Stateful nodes (Integrator, Latch, Differential, Inhibitory, SoftLatch) also started from the parent's state.

**Fix** (`brain.rs`, `body.rs`, `world.rs`, the view's and the CLI's initial-population spawns): `Brain::reset_runtime_state(period)` = what `CopyBrain` + `FinalizeEditing` leave (inputs zero, every other node `f(0 + bias)`), called for an egg's brain after its mutation, for a hatching egg, for the three template spawns of `World` and for the initial population of the viewer and the CLI. A brain loaded from a save keeps its state, but a BIBITE loaded by the world is healed:
`Brain::heal_unfed_nodes` puts a node no enabled synapse feeds back to `f(bias)` when it holds something else (a save the port wrote before this fix; a game save is never changed, its values already are `f(bias)`). `from_records` still keeps the records' state exactly (the replay of the game's brain ticks and the editor need that).
Tests: `a_reset_brain_has_no_stale_node_values`, `an_egg_gets_a_brain_with_fresh_nodes`, `loading_heals_a_stale_value_in_an_unfed_node`. The pinned `evolution_regression_fingerprint` moved (0xa818_5c92_2bf4_bf31 -> 0xf292_be2d_a8ad_d122, deaths 44 -> 43).

**Not changed, on purpose:** a node nothing feeds and with no bias still rests at `f(0)`, which is 0.5 for a Sigmoid and 1 for a Gaussian, because the game does the same (the user asked 2026-11-19 to leave that as it is).

## Senses against the original game (2026-11-19, "do the sense check": field of view, counters and Unity's vector maths made exact; what a save cannot give is listed)

**How it is measured** (`tools/braincheck/game_diff/game_brain_ticks.py` with `SAVE_WORLDS=1`, `extract_rot.py`, `crates/bibites-cli/examples/sense_vs_game.rs`): the original game (hidden) is stepped one tick at a time and saves its whole world after each tick; the port loads
the save of tick K, runs ONE `World::tick()` and compares the 33 input values of every bibite (`NEATBrain.UpdateSenses`) with the game's own values after tick K+1, bit for bit, plus the cadence counters. The game senses the world as it stands at the start of the tick (`FixedUpdate`
runs before the physics step), so a port started from that same saved world has nothing to disagree about but the senses. `lol.zip`, 60 ticks, 15 ticks per second, vision lookup and sensing factors set to 1 (every brain tick a full sense): 677 brain ticks of 38 bibites.

| | before | after |
|---|---|---|
| brain ticks with all 12 field-of-view inputs bit-exact (rotation as the save has it, `rb2d.r`) | not measured (BibiteAngle 237 of 677 ticks off, PlantAngle 380, BibiteCloseness 120, PlantCloseness 127) | **471 of 677** (BibiteAngle 80 off, PlantAngle 108, BibiteCloseness 71, PlantCloseness 54) |
| the same, each bibite's rotation allowed to move up to 3 / 10 floats (see "what a save cannot give") | | **571 / 587 of 677** |
| `visionLookupCount` / `visionSensingCount` after the tick | 521 / 410 of 1,340 differ | **0 / 0** |
| `brainTicksCount`, `LastOutput` of the input nodes (22,341), Tic, Minute, TimeAlive, LifeRatio, IsGrabbing, AttackedDamage, EggStored, NPlants, NMeats | exact | exact |

**What was wrong** (all in `bibites-core`, found by the test above):
1. **Unity's own vector maths runs in double too** (Mono, see the brain entry): `Vector2.magnitude` is `(float)Math.Sqrt(x*x + y*y)` with the sum in double, `Angle` / `SignedAngle` are `acos` of the normalised dot product (the port used `atan2`), `normalized`, `Dot` and `Rotate` likewise. `math::unity` copies them; `senses.rs` uses them
   everywhere (`target_in_view`, the concentration angle `2f * SignedAngle(..) / viewAngle`, the herd vectors, the pheromone angles).
2. **The weights are one double expression each** (`Mathf.Pow(1.05f - d / R, 2f) * sizeFactor * (1.05f - Abs(angle) / (viewAngle / 2f))`, three variants for pellets, bibites and corpses): `senses::fov_weight_of`.
3. **`Transform.up` is not `(-sin a, cos a)`.** Unity keeps the rotation as the quaternion `(0, 0, sin(a/2), cos(a/2))` and rotates `(0, 1, 0)` in single precision: `(-(w * 2z), 1 - z * 2z)`. Measured: 471 instead of 433 rows exact (`math::unity::transform_up`, used for every `transform.up` of the senses and
   of the movement organ's `SignedVelocity = Vector2.Dot(velocity, transform.up)`).
4. **The cadence counters subtract the period** (`brainTicksCount -= brainTickFactor`, `visionLookupCount -= visionLookupPeriod`), the port reset them to 0: the same schedule while a counter stays below its period, not after a setting was lowered (the game kept counters 2 and 4 at periods 1 for ever).
5. **The pheromone sense is one tick late in the game.** `Pherosense.PherosenseAround` runs at the END of every fixed update (from the pose and pheromones of that moment) and the next brain tick reads it. Measured: the sums the game's brain read match the pose of the tick BEFORE (119 of 283 rows, 0 for the pose of the same tick). The port senses at
   the start of the brain tick with the current pose. Now a bibite that uses pheromone inputs senses in every tick and a brain tick reads the values of the tick before (`SenseOutputs::pending_phero`, `world::delayed_phero`); a just-loaded bibite reads zeros on its first brain tick, as a loaded game does.
6. Smaller, same class: the growth step (`GrowthRateFunction`, `num2 = Want2Grow * rate * period * scale / 100f`, `Set2DSize(growth * sizeRatio * sizeRatio)`, the 0.5 % test), `UseEnergy((metabolism + brainUpkeepCost) * fixedDeltaTime)` and the load's `growth = d2Size / (SizeRatio * SizeRatio)` (`BibiteBody.LoadState`) are single
   double expressions (`body.rs`, `bundle.rs`). Only the last one moved a measured number: the Maturity input was one float off in 206 of 677 brain ticks, now 81 (the game's own load rebuilds `growth` from `d2Size`, so a loaded bibite and the game's continuing one can differ by a float; the other expressions act on state a save does not hold).

**Consequence:** the pinned `evolution_regression_fingerprint` moved again (0x160b_d186_45b3_8cc5 -> 0xa818_5c92_2bf4_bf31, births 25 -> 20, deaths 41 -> 44): the population of a world moves when the senses move by a float. Core tests 680 (Improved: see the memory note), the three new ones are
`cadence_counters_subtract_the_period`, `the_pheromone_sense_is_read_one_tick_late` and `unity_helpers_follow_the_engine`.

**What a save cannot give the port** (so these inputs cannot be compared bit for bit by a replay; the replay says how far each is off):
* the rotation is saved as degrees rounded to a float (`rb2d.r`, `transform.rotation`), the game's state is the angle in radians: every angle the senses compute carries one or two floats of it. `sense_vs_game --rot-search N` moves each rotation by up to N floats and keeps the best; a formula that is exact matches with some step on every row.
  86 % of the rows match with a step within 10; the other 13 % (90 rows) differ by 1-40 floats in a closeness (a single seen target mostly) and are NOT explained: not the subject's growth (same share with and without), not everybody growing first (worse), not a bigger collider margin. Candidates left: the order of the overlap
  query's results (the sums add in that order), the order of the objects' own updates (a target that has already grown or decayed this tick).
* the angular velocity is not saved (RotationSpeed), the growth value is rebuilt from `d2Size` (Maturity is one float off in 12 % of the rows after the loader does the game's expression, 30 % before), the organ areas are rebuilt from the size (Fullness), the growth timer and the movement organ's last speed are not saved: Speed is compared with
  the value the port's own tick before computed (`--no-prime` shows the difference).
* a pheromone spot's `heading` is saved as `{}`: a loaded game (and port) reads 0 for all three heading inputs; the sums and angles depend on whether the spot's own update ran before or after the bibite's in that tick (not in a save), which matches in about 42 % of the rows with the previous tick's strengths.
* the vision cache (`FindSeenEntities` up to `visionLookupUpdateFactor` brain ticks ago) is not saved: only ticks whose lookup is due can be compared at the default factors (10 and 5).

## Brains are MATH exact against the original game (2026-11-19: bit for bit, 9,600 brain ticks, five worlds, both editions)

**What was asked:** "ensure brains are MATH exact in each version". **How it is measured** (`tools/braincheck/game_diff`, `crates/bibites-cli/examples/brain_vs_game.rs`): the original game (hidden) is stepped ONE tick at a time and every bibite's brain is written before the
first tick and after each; every brain tick is then replayed in `Brain::step` from the game's own node state and the game's own input values, and all three numbers of every node (`Value`, `LastInput`, `LastOutput`) are compared bit for bit. The game's senses are not under test
here (they are the game's own); the brain's arithmetic, the order of the sums and the nodes, the clamps and the brain's schedule are.

| world | brain ticks replayed | port before today | now |
|---|---|---|---|
| `lol.zip`, 35 real brains (all 14 node types), herding off | 653 | 609 ticks with a node off | 0 |
| the same bibites with RANDOM brains (every type 2-3 times, random weights, random saved states), herding off | 433 | 337 | 0 |
| the same, herding on | 451 | 245 | 0 |
| the same, weights from 1e-3 to 1e20 (clamps at +-1024, infinities, NaN) | 332 | 13 | 0 |
| `shrekvssabertoothtest1.zip`, 686 evolved brains at 40 ticks per second | 7,730 | (not run) | 0 |

Faithful and Improved give the same numbers. Ticks where the game does not think (the brain must stay untouched) are checked too (9,576 of them in all), and the schedule (`brainTicksCount`) matches in every one.

**What was wrong** (all in `bibites-core`, found by the test above, not by reading):
1. **The game does the arithmetic of a C# expression in double and rounds to float only where a value is stored, passed to a method or returned** (Mono). Five node types hold more than one float operation in one expression and the port rounded after every step: Sigmoid (`1f / (1f + Mathf.Pow(..))`),
   Gaussian, Integrator (`LastOutput + input * period`), Inhibitory (`input - LastInput + LastOutput * Mathf.Exp(..)`) and SoftLatch (`Exp` calls take and return floats, `num3` is a float local, the rest is one double expression). Given the game's own sums the port was one ulp off in Gaussian 32 %,
   Sigmoid 16 %, SoftLatch 4 %, Inhibitory 2 % and Integrator 0.6 % of the nodes (up to 128 ulps after the nodes behind them carried it on); now 100 % bit-exact (`brain.rs::activation_function`, `soft_latch`, `math.rs::sigmoid`, `gaussian`). A single float operation is the same either way (double holds the
   exact result of one float operation), which is why Linear, ReLu, Abs, Mult, Latch, TanH, Sine and the sums (`weight * LastOutput` stored in a local, `num` stored in a local: float steps) were already exact.
2. **With herding off the game still computes the Herding output.** `ComputeProcessingStructure` skips synapses `NodeOut != 2` (the literal 2: an INPUT node), not the Herding output (node 35); the port skipped node 35, so that node kept its old value in every world with `disableHerding` (which `lol.zip` has). It changes no behaviour (the
   steering ignores it with herding off) but the node's value is in the Brain panel and in saves. `brain.rs::HERDING_SKIPPED_NODE`.
3. **`(float)Math.Tanh(pheroSum / 5f)`** (the three pheromone inputs): the quotient is a double handed straight to `Math.Tanh`; the port divided in float and used the float `tanh`. `senses.rs::phero_sense_value`.
4. **`brainUpkeepCost`** (`nSynaps * synapseUpkeepCost + nHidden * neuronUpkeepCost`) is one double expression, rounded once (`BrainUpkeep::energy_per_second`).
5. `ExecuteNeuron` adds the bias to an Input-type node too (`!TypeUsesBiasDifferently`); the port skipped it (never executed in a brain, an input node's bias is 0; the Bibite Editor can).

**Consequence:** the pinned `evolution_regression_fingerprint` moved (0xc40a_a2b2_b958_ec7c -> 0x160b_d186_45b3_8cc5, births 25, deaths 41; the same value in Faithful and Improved): the nodes behind a node that used to be one ulp off carry it on, so a population moves. Core tests 677 (759 Improved), 0 failed; the independent Python reference
(`tools/braincheck/brain_ref.py`) was changed to the same model and agrees with the Rust brain bit for bit on its three fixtures.

**Traps of the measurement.** The game's JSON has no negative zero (it prints `-0.0f` as `0`) and writes infinities and NaN as strings; the first ~4 ticks after a load run with another time step (`--warm-up`); dead bodies do not think; `game_diff/README.md`.

**What is NOT bit-exact and cannot be tested this way** (the brain's INPUTS are the game's senses): the sense values themselves (`PlantCloseness`, `Speed`, `RotationSpeed`, ...) come from the body's pose and the physics, which is not bit-exact against Unity's Box2D (a bibite's position was measured 0.0004 off after one tick), and from `FieldOfView` /
`Pherosense` arithmetic that has the same double-expression structure as the brain's (`Vector3.magnitude` is `(float)Math.Sqrt(x*x + y*y + z*z)` in double, and so on); these were not compared bit for bit and probably are off in the last place. The brain MUTATION arithmetic (weights, biases drawn with `Random.Range`) uses another random stream than the game's and cannot be compared at all.

## "Shrek population stabilizes in the original game but eventually crashes in RUSTibites" (`shrekvssabertoothtest1`, reported 2026-11-18: FIXED, three causes, measured against the original game)

**Save and what the game does.** 603 shrek (species 1, diet 0, steer on `Phero1Heading`) and 19 sabertooth (species 2) at t = 163.8 s, 17 eggs, 7,000+ plant pellets. The original 0.6.3.1 game (hidden, same save, 900 s, through the BibitesAPI lab) peaks at 929 shrek near t = 390 s and settles at 641-690
(predators 19 -> 6); per 100 s: births 494 / 859 / 762 / 598 / 515 / 409 / 414 / 385 / 425. The port fell to 196 shrek by t = 1064 s and 78 by t = 1364 s (births 475 / 582 / 383 / 248 / 162 / 130 / 62 / 47 / 15 after the first two causes below were already fixed; the original exe's births were not logged). Without the predators (`nopred`) the port was stable too, so the predator pressure was
amplifying something that was wrong with the shrek themselves.

**How it was found.** A world of 49 identical shrek, each with ONE plant pellet in its mouth (`make_eatfix.py`), read tick by tick in both programs, then the mean of every brain node of the living shrek at 10 / 30 / 60 s from the same save (`game_probe.py` against the port's `run --save`). The
nodes whose means differed told what to look at: `IsGrabbing` (game 0.09 vs port 0.7), `PlantCloseness`, and then `Rotate` (game 0.00, port +0.10) with six `Phero*Angle/Heading` inputs at 0.2 in the port and 0.000 in the game.

**The three causes (all in `bibites-core`):**
1. **Pheromone angle and heading inputs read 180 degrees with no pheromone at all** (`senses.rs`, `signed_angle_deg`). `Vector2.SignedAngle(up, zero)` is 0 in Unity (`Angle` returns 0 under `kEpsilonNormalSqrt`); the port's `atan2(cross, dot)` of a zero `to` is `atan2(+-0, -0)` = 180 degrees when the
   bibite's `up` has BOTH components negative (`x * 0` and `y * 0` are both -0), that is for a bibite facing down-left, a quarter of them: all six `Phero{1,2,3}{Angle,Heading}` inputs read +-1. The shrek's brains steer on `Phero1Heading` (weight +1.4 into `Rotate`), so a fifth of them kept turning for a signal that did not exist.
   (The start save itself already holds those 1.0 values in 118 of the 603 shrek, which the game cannot write there: it was saved from a port run.) Fixed with the game's zero guard; test `signed_angle_matches_unity_convention` covers every sign of `from`.
2. **A held pellet vanished from the holder's field of view** (`world.rs`, `fov_senses_from_cache` / `fov_targets`). `FieldOfView.PelletIsHeld` compares the pellet with `mouth.links[i].attachedRigidbody.transform`, and a joint's `attachedRigidbody` is the body the joint sits ON (the holder), so no pellet ever matches and a held pellet stays visible: a shrek holding a plant pellet in
   its mouth reads `PlantCloseness` 1.0 in the game and read 0.0 here. DEVIATIONS D7 said the opposite ("held pellets are excluded") and was wrong.
3. **A grab never broke** (`body.rs`, the weld's `break_force` was 0 = never). `BibiteMouth.FixedUpdate` writes `breakForce = breakTorque = jawStrength * max(diet, 0.1) * desireToGrab / nHeld` on every link while the grab output is at least 0.15, so a herbivore's grip is a tenth of a carnivore's: a shrek lets go of a 70-unit plant pellet
   within about ten ticks of its first acceleration (49 identical shrek, all let go by tick 8 in the game). Here a grab held for ever: 63 % of the shrek held something at every moment against 5 % in the game, and what a shrek holds it cannot eat, so it starved or was caught. `grab_break_force` (kept unused since 2026-11-09) is wired in; below a grab output of 0.15 the game
   leaves the last value in place, so the port uses `max(desire, 0.15)`.

**Result, same save, seed 1** (`run --capture`, 900 s):

| | game | port before (original exe) | port now |
|---|---|---|---|
| shrek at the peak (t ~ 360-390 s) | 929 | 850 | 961 |
| shrek at t = 1064 s | 641 | 196 (78 at t = 1364) | 645 |
| births per 100 s | 494 859 762 598 515 409 414 385 425 | not logged (first two causes fixed: 475 582 383 248 162 130 62 47 15) | 500 823 749 618 507 435 404 391 396 |
| pellets at t = 1064 s | 3,039 | 12,749 | 3,055 |
| mean speed / `Rotate` of the shrek after 60 s | 62.5 / 0.00 | 66.2 / 0.11 (first two causes fixed) | 61.1 / 0.01 |

The segmented long run (`run --ticks 4000` x 24) stays at 580-660 shrek (sabertooth 8 down to 2-5) up to t = 2564 s, 2,400 s of world time after the start. The first two fixes alone slowed the decline (238 shrek at t = 1163 s instead of 78) and the third one removed it. Tests: core 675 passed (0 failed, 16 ignored); the ignored `evolution_regression_fingerprint` is unchanged.

**Not at parity (found on the way, not changed):**
- The weld lets go LATER than the game's on the eating fixture: tick 17-18 here against tick ~9-10 there (49 shrek, 70-unit pellet). The force the weld feels grows with speed in the game; here the same break force is reached later. The population numbers above do not suffer from it, but it is a measured difference in the weld/drag model (`phys::WeldDef`, held-pellet drag).
- Death labels: over 900 s the game's capture calls 1,563 shrek deaths "killed", 1,781 "other" and 1,479 "starved" (4,823); the port 3,048 "killed", 1,444 "other" and 295 "starved" (4,787): the port's `DeathCause` and the game harness's state-based guess are not the same classification (a swallowed bibite is "killed" here). Totals and per-window counts agree within 10 %.
- `MeatCloseness` / `NMeats` / `MeatAngle` read 0 here for a shrek that wires no meat input but sees corpses (the game fills them when the bibite-vision mask is on: 0.04 on average); display only, nothing steers on it.
- Predators: 12 sabertooth at t = 1004 s here against 6 in the game (both falling).
- **Capture traps**: `rustibites run --capture` writes `feeding.jsonl` of 16-22 GB in 15 minutes; use `RUSTIBITES_NO_FEEDING=1`. `game_probe.py`'s "ticks" are ticks, not seconds.

## "Zooming out heavily lags the game" (players' report, 2026-11-16: FIXED, measured in the real window)

Cause, from `RUSTIBITES_LOG_FRAMES=1` in the real window (Archipelago, 2,263 bibites, 4,773 pellets, 1920 x 1080, paused): up to ortho 1,500 the frame was 145 fps (the monitor's rate); at ortho 6,000 it was 37 fps and from 20,000 up 13 fps
(draw 53 ms, present 19 ms). Two costs, both per frame and both of things nobody can see: (1) every bibite was six sprite quads in six atlases and every pellet one, from any distance (18,500 primitives and a quad each at ortho 20,000, for what is a pixel or two;
the capsule fallback had a size cut-off, the sprite path had none); (2) as soon as the view reached past 1.5 x the world's size the microscope shade (the dark ring round the world) was rastered on the processor at a quarter of the resolution and uploaded as a new texture every frame.

What changed (`render.rs`, `sprites.rs`; all uncommitted when this was written):
- **Level of detail.** A bibite under 6 px long is drawn as its body sprite alone (no mouth, eyes, arms, tail; the skin programs that only show on the arms are off), under 2.5 px as ONE dot of the body sprite's mean colour (through the same skin tint, the killer / hurt effects kept);
  the selected bibite is always in full. A pellet whose sprite would be under 2 px across and a pheromone / blood particle under 1.5 px is a dot too. Dots live in `Scene::dots` (20 bytes) and are referred to by `Primitive::Dots { first, count }` runs, which keep the draw order; a dot is a whole
  output pixel at the least (what a sprite drawn a pixel across at the least looked like), in the window and in the software rasteriser alike.
- **The shade is a GPU shader** (`SHADE_FRAGMENT_SHADER`: `saturate((|p| - start) / fade)` at the world point of the pixel); the software picture is the fallback when the driver refuses the program (a `warning: GPU shade material failed` line in the log; there was none here).

Measured (real window, same saves and views, before = the build of 2026-11-16 morning, after = this change; fps / draw scene ms / present ms):

| view (paused) | before | after |
|---|---|---|
| Archipelago ortho 1,500 | 145 / 0.14 / 5.2 | 145 / 0.04 / 5.5 |
| Archipelago ortho 6,000 | 37 / 1.35 / 22.6 | 145 / 0.28 / 5.0 |
| Archipelago ortho 20,000 | 13 / 53.1 / 19.0 | 145 / 0.68 / 4.2 |
| Archipelago ortho 60,000 | 13 / 51.8 / 18.6 | 145 / 0.71 / 4.2 |
| 3 Islands ortho 20,000 | 29 / 2.7 / 28.0 | 145 / 1.14 / 4.1 |
| 3 Islands ortho 60,000 | 17 / 2.5 / 53.0 | 145 / 1.16 / 4.1 |

The same views with the world running at speed 1 give the same (12 .. 36 fps before, 145 after). Display list at ortho 20,000: Archipelago 18,515 primitives -> 190 + 7,014 dots, scene build 4.2 -> 1.3 ms (headless); 3 Islands 18,159 -> 112 + 12,631 dots, 2.9 -> 1.2 ms.
**Looks the same**: window shots of old and new exe at eight views (`RUSTIBITES_SELF_SHOT`, Archipelago 1,500 / 6,000 / 20,000 / 60,000, 3 Islands 20,000, Branching Desert 6,000, the maze 1,500 / 6,000) differ by 0.07 .. 1.25 (mean sum of |dRGB| per pixel) and
the mean brightness by 0.3 at the most; the difference image is specks inside the pellet clouds (a dot snaps to a pixel where the sprite sat between two) and nothing at the world's ring or the shade, so the GPU shade matches the software one.
Tests: `a_bibite_a_few_pixels_long_is_drawn_as_its_body_alone_and_a_tiny_one_as_a_dot`, `a_pellet_a_pixel_across_is_a_dot_and_a_bigger_one_a_sprite`, `the_software_rasteriser_draws_a_dot_as_at_least_one_pixel`, `a_bibites_dot_has_the_colour_of_its_body_drawn_in_full`; the viewer's quick set (769 pass, 3 ignored), and the sim thread's published scene is compared with its dots too.
**Not covered**: only two saves were run in the window (a 100,000-bibite world was not: its dots are 20 B each, and each is a `draw_rectangle` in the window, about a tenth of a microsecond, so 100,000 are about 10 ms: a mesh / texture path is the next step if it is wanted); the experimental
background's cells (opt-in, `draw_background_cells`) are still rastered on the processor every frame; the headless pictures cannot show the GPU shade (the GLSL is only run in the window); the 145 fps is the monitor's rate, so how much headroom is left is not measured (scene + draw are about 1.5 ms at ortho 20,000).

## Code audit ("audit the code for bugs, then fix them", 2026-11-16): what was found, fixed and tested

A review of the whole `crates/` tree at the "high" level of the code-review skill, then four narrower readers on the riskiest parts (the pool tuner and the threads, the uncommitted core changes, the viewer's panels, the renderer / title menu / settings file),
and a sweep of every `clamp`, string slice, division, `unwrap` / `expect` / `panic!`, `chunks` / `step_by` and file write / rename / delete in the shipped code (the shipped code has about 60 `unwrap` / `expect` / `panic!` sites, all on invariants; none was reachable from a file or from input).

- **A save written over another lost both when it failed half-way.** `write_save_bundle` (and `write_zip`, the scenario bundle) created the zip in place, which empties an existing file first: the quick save (always the same file `quick.zip`) or a save the player confirmed to overwrite
  was gone when the disk filled up, a serialisation step failed after the file was created or the program was killed during the write. Now the zip is written to `<name>.part` beside it and swapped in when it is whole (`bibites_io::write_replacing`: the data is flushed first, a failed write removes
  the half file and leaves the old save as it was, a refused swap, the old file being held open, falls back to copying the finished file over it); the template, genome, entity and settings writers of the io crate use the same helper (`write_bytes_replacing`). The Savefiles list only
  shows `.zip` files, so a half-written file no longer shows up as a (corrupt) save while it is made. Tests: `write_replacing_tests::*` (a failed write, a whole write over an old file and over a leftover half file, a save bundle over another file loads back and leaves nothing beside); the io suites
  (22 unit + the 14 integration exes: storage, bundle, world, robustness ...) pass. Leftover `<name>.zip.part` files of a killed run are not cleaned up (an autosave's name is never used again).
- **The settings file was rewritten in place at every slider step** (`UserSettings::save`: truncate, then write): a kill in the middle left it empty, which the next start reads as "no settings". It goes through the same write-beside-then-swap (`write_whole`); test
  `the_file_is_written_through_one_beside_it_and_a_leftover_one_does_no_harm`.
- **A press on a Settings slider's handle that did not move still changed the value** (`UserSettings::set_fraction`: the value is worked out again from the handle's pixel, a millionth off): the setting was pinned over the installed game's own value and the file rewritten. `Slider.OnPointerDown`
  leaves the value alone on the handle. A change below a hundred-thousandth of the range is none now (a pixel of any track is a thousandth or more); test `a_press_on_a_sliders_handle_that_works_out_the_same_place_changes_nothing`.
- **Editing `speciesGeneticSpan` or `speciesSpanScalingFactor` did not recount the active species** (`GlobalLineageManager.Awake` subscribes `UpdateEffectiveSpeciesSpan` to the `OnChange` of both): after some species died out (the species log drops them without recounting) the port judged births by the old count until a species
  was founded or the Lineage panel opened. `World::apply_settings` recounts when either span setting changes (settings animators that write them inside the tick still do not: not a case the game's panel allows). Test `editing_a_span_setting_recounts_the_active_species` (core, light).
- **The window stood still from its second frame without the themed UI** (`--simple-hud`, no extracted game art, or the UI's upload to the GPU failing: "using simple HUD"). A frame's owed ticks go to the tick helper after its UI frame is built (`App::dispatch_overlapped_ticks`), and the simple HUD has no UI frame, so they
  were never run and the accumulator grew; removing the 0.5 ms floor of `overlap_worth_it` (the uncommitted pool work) had turned a bug of slow worlds into every world's. The window loop now sets `overlap_ticks` each frame from `--no-overlap` and from whether it has the themed UI and its GPU upload;
  a frame without runs its ticks in `advance`. (A window cannot be started by a test: not run in a real window either.)
- **In-game Settings / Load Game / Library panels closed with Escape left their menu behind** (`overlay_menu` was dropped only by the panel's own Back button): the Load list showed the saves of the panel's first opening (a quick save or an autosave made since was not in it), the Library the bibites of then,
  and every click in the panel put the settings of then back into the running world (`apply_user_settings`: the Biology pop-up's Sort toggle, `include_autosaves`, `save_as_template` ...) and wrote them to the file; a slider held when Escape came stayed armed. The menu is dropped when no panel of its kind is on the stack any more
  (`overlay_panel_open`; the Bibite Editor over the Library keeps the Library's). Test `the_menu_behind_an_in_game_panel_lives_while_the_panel_is_on_the_stack`.
- **The Library's picture showed the old bibite after the editor saved the template under the same name** (the genes were read once per file and kept for the life of the menu; a file that failed to read stayed "no template" for good): `reload_library` and `refill_library` forget them. Test `a_template_saved_again_gets_a_new_picture_in_the_library`.
- **A throw of a held mass that overflows gave NaN** (`BibiteMouth.ReleaseAndMaybeThrowAllHeldObjects`: `Mtotal / (jawMass + Mtotal)`): a held wall of the maze save weighs infinity (plant density 1e35), so the force is NaN, and any held plant of about 14 u2 or more makes the product infinite. The same arithmetic ran here and put NaN into the holder's
  and the pellet's velocities; a bibite with a NaN pose dies as a hazard. A force that is not finite moves nothing now (what Unity's `AddForce` does with one was not found out). Test `a_throw_of_a_mass_that_overflows_the_force_moves_nothing`.
- **A pellet was flagged held by a mouth that already held ten** (`tick_mouth_pellet` flagged it, then `add_held` found no room): nothing ever let it go, and a held pellet is exempt from Red Death, never sleeps and loses its drag. The game checks `nHeld < 10` first (`TryGrabTarget`), as the bibite branch here does.
  Test `a_mouth_that_holds_ten_flags_no_pellet_as_held`.
- **When one of two holders' weld to a pellet gave way, the wrong holder let go** (the step reported only the held body; the first holder of it was released): it reports the holder's tag as well now (`PhysicsScratch::broken_welds` is `(held, holder tag)`). A weld only gives way when its two ends are 200 units apart (a holder wrapped across the world),
  so it was rare. Test `the_holder_of_the_weld_that_gave_way_is_named_when_a_body_has_two`.
- **A genus was kept for one species more than in the game** (`SpeciesNameGenerator.GenerateNewName` counts from `species.parentSpecies`, the parent; the port started at the grandparent): a root and its next four descendants share a genus, the sixth takes a new one. This changes the names of NEW species in deep lines for a seed (names have a random stream of their own, so
  nothing else moves; saves keep the names they have). Test `a_genus_is_kept_for_five_species_of_a_line_and_the_sixth_gets_a_new_one`.
- **The settings panel's X did nothing while the pellet placer, a bibite placer or a tower's panel was open** (`ClosePanel` closes the newest entry of the Escape stack, and `sync_escape_stack` puts those entries above the panel and back again as soon as they are removed): a panel's close button skips them now
  (`PanelKind::is_tool_entry`; the placers have close buttons of their own, Escape still pops them). Test `a_panels_close_button_closes_the_panel_and_not_the_placer_or_tower_entry_above_it`.
- **The mouse wheel could not zoom over the place of the Information panel's list while the HUD was hidden (H)** (the panel is "open" but not drawn, and the list's rectangle is the one it last had): not while `show_hud` is off. Test `the_wheel_is_the_cameras_where_the_information_panel_was_while_the_hud_is_hidden`.
- Small ones: the `--threads` help said every stage of the tick uses the workers however small the world (the pool tuner decides now); the `ParallelGates` docs said the tuner's floor is the prior divided by eight (it is four); `step_sensing` kept an `ensure_sensing_pool` call that could never create a pool;
  the `species_span_probe` example divided by a zero `every`; an unused `mut` in a body test.
- **Checked**: `cargo check --all-targets` of the workspace, the core tests (670 pass, 16 ignored) and the pinned 12,000-tick fingerprint (`evolution_regression_fingerprint --ignored`: unchanged, so a world computes what it computed, short of the new throw / grab / genus guards, which a world of 12,000 ticks does not reach), the viewer's quick set (765 pass, 3 ignored),
  the 44 light `menu::tests` / `challenge_menu::tests` one process each, three light `ui_runtime::` tests that click close buttons (the creator's OK button, the settings panel's buttons in any window, the ids of the window's frame), the io crate's suites, clippy of core and the viewer (no new warning at an edited line). **Not run**: the real window (so the overlay menu, the
  frozen-window and wheel fixes were checked by tests of their parts and by reading the loop), and the heavy `ui_runtime::` / the two heavy `menu::tests` (3.5 .. 3.8 GB each). The Desktop shortcut's exe and the tester zip are the builds of 2026-11-15 and do not have any of this.
- **Looked at and left**: `UiFrame::interact` fires every action under the topmost id (the rule the click-area fix relies on: two panels on screen together must not share an id; a test lists the ids of a frame that are shared); the title menu's second render in `run_menu_screenshot` is a timing line of the pixel-diff loop;
  `sanitize_file_name` does not refuse Windows' reserved device names (`CON`, `NUL` ..; it mirrors the game's bad-character check, which has none either; what a save of that name does was not tried); `GamePrefs::load` waits for its PowerShell child without a time limit (nothing reported).
  Found by the readers and not fixed: a held slider whose button is let go OUTSIDE the window stays held (miniquad takes no mouse capture on Windows: the release over the title bar or beyond a side never arrives, `is_mouse_button_down` stays true, and the slider, a Settings one writing the file, follows the pointer
  when it comes back, until the next click; a fix needs `unsafe` or another window layer, and the crate forbids `unsafe`); the settings panel's cached frame keeps the old UI scale and offset after a UI Scale change until some state in its cache key changes (clicks still land where it is drawn);
  the Information panel's snapshot is kept across a world replacement for up to a second (a click on a species row in that time picks a member of whichever new species has that id); `lineage_mesh_view` numbers its hit areas `HIT_PART + species % 9_999` (a clash only after about ten thousand species; not checked).

## Panels that hang over each other shared click-area ids ("fix the several panels", 2026-11-15: FIXED and tested; the real window was not driven)

`UiFrame::interact` answers a click with EVERY action registered under the id of the topmost click area under the pointer (`action_at`, `tint_pressed`, the slider presses and `remember_click_rect` go by id too), so two panels that are on screen together must
not use one id. `UiId` is a bare `u32` and each panel picked its numbers by eye, so the ~190 `frame.hit` sites were audited for pairs that can be up together. What was found:

- **The Save Bibite dialog (8800 .. 8807), the Biology Flow pop-up (8800, 8801) and the Information panel (8800 .. 8861) shared a block.** The Information panel stays open on the right while the player works, so: a click on the dialog's panel (8801) also closed the Information panel
  (`InformationClose`), a click on the dialog's Name field (8802) also swapped its count / energy section (`InformationToggleMode`) and a click on the Flow pop-up's Sort toggle (8801) also closed the panel (a click on the dialog's blocker (8800) only
  added the Information body's own swallow-the-click action to the dialog's). The dialog's Description field, toggles and buttons (8803 ..) were safe by luck.
- **The pause menu's veil (8100) was the Graphs page's exit button (8100).** With the page open, a click on the dimmed screen outside the menu also fired `ClosePanel`, which closes the TOP panel: the pause menu closed itself.
- **The Information panel's tag rows (`8830 + index`) ran into its own scroll bar (8860, 8861) from the 31st tag on** (a row is made for every tag of the world, only seven show at a time): a click on such a row also paged the list or took the handle.
- Now: Save dialog 9010 .. 9017, Flow pop-up 9000, 9001, Graphs exit 8103 (the pause menu keeps 8100), tag rows 930_000 + index (a block of their own: the lists that grow with the world's content are far apart: lineage shapes 200_000, neurons 900_000 .., tags 930_000).
  `UiId`'s doc (ui.rs) lists where each panel's block is, and the constants of the three panels say why they are where they are. The other blocks were left where they were (a number nobody else uses stays valid).
- **Tests** (`app::sprite_renderer_tests`, light, `App::new(synthetic ..)` + `hud::draw_extra_panels`, i.e. the frame the window builds, then a click through `UiFrame::interact`, which is what the pointer routing applies):
  `a_click_on_the_save_dialog_fires_nothing_of_the_information_panel_under_it`, `a_click_on_the_flow_pop_ups_sort_toggle_fires_nothing_of_the_information_panel`, `a_click_on_the_pause_menus_veil_fires_nothing_of_the_graphs_page_under_it`,
  `the_information_panels_tag_rows_never_share_an_id_with_its_scroll_bar` (35 tags, the list wheeled to its end). Each also asserts `UiFrame::shared_ids()` (a test-only helper: the ids a frame carries twice) is empty. Checked the other way round too: with the old ids put back, all four
  fail on exactly the ids found (`[8800, 8801, 8802]`, `[8800, 8801]`, `[8100]`, `[8860, 8861]`). `ui_runtime::tests::the_windows_frame_gives_every_click_area_an_id_of_its_own` (light, `native_fixture`) builds the WINDOW's frame (the captured runtime frame, the panels over it, an open dropdown)
  in twelve states (the world, the Information panel, each of the five settings tabs, the animator creator, three animators, the Graphs page, the pause menu over it: 14 .. 114 click areas each) and finds no shared id (the old ids: the pause menu over the Graphs page, `8100 carries [ClosePanel, Consume]`).
  `ui::tests::areas_that_share_an_id_answer_together_and_shared_ids_lists_them` pins the rule itself. Headless pictures of three scenes (Information panel, Graphs page, the pen at 1600 x 900) are byte-identical between the exe before and the exe after (the ids move no pixel).
- **Not found / left:** no other pair of panels that can be on screen together shares an id in the sites read (the curated blocks 8000 .. 9999, the captured runtime frames 20_000 .. 44_500, the dropdown and the editors from 46_000, the lineage panel from 200_000).
  One latent overlap was noticed and not touched: the captured toolbar overlays take `20_000 + 1_000 * n` (n = the n-th open overlay) and the settings panel's nodes start at 22_000, so a THIRD simultaneously open toolbar panel would number its hits like the settings panel's first ones;
  the toolbar panels open one at a time on hover, so it needs a transition frame with three open and the pen. The heavy `ui_runtime::` / `menu::tests` modules (3.5 .. 3.8 GB each, over the RAM floor) were not run.

## "Is the rising cost of a new species, as extant species increase, functioning?" (asked 2026-11-15: YES, measured on running worlds; two small gaps fixed and tested)

- **The game's rule** (`GlobalLineageManager.UpdateEffectiveSpeciesSpan`): `usedSpeciesSpan = speciesGeneticSpan + max(activeSpecies.Count - 20, 0) * speciesSpanScalingFactor` (defaults 1.5 and 0.05, shown as "Effective Species Span" in the Lineage panel's settings).
  `CheckNewSpecies` reads it at every birth (`BibiteBody.StartBody`): a newborn stays in its species while its distance to the species' template is below the span, else an active child species within half the span takes it, else the
  two-step mutant rule (a second consecutive outlier founds a new species from its parent). The span is recounted only when a species is created or found from a template, on load, when a span setting is written and when the Lineage panel opens
  (`SpeciesPanel.OpenPanel`); NOT when `LogSpeciesInfo` drops the extinct species, so it stays where it was after a die-out until one of those events.
- **The port has it** (`species.rs`: `used_span`, `current_used_span`, `SPECIES_SPAN_FREE_COUNT = 20`, `assign_with_founder` with the game's `<` / `<=` and last-child-wins; unit tests since the first port) and keeps the held count the same way. Measured with `crates/bibites-cli/examples/species_span_probe.rs`
  (`hyun.zip`, 195 recorded species of which 21 active, base span lowered to 0.25 so that species get founded fast, same seed, 3,020 births, 30,000 ticks): scaling 0 (no cost): 68 species founded, active 21 -> 65, span fixed at 0.25;
  the saved 0.05: 42 founded, active 21 -> 45, span 0.30 -> 1.50 (always `0.25 + (active - 20) * 0.05`), the founding rate falls from 26 to 11 per 1,000 births; scaling 0.5: 6 founded, active falls to 16, the span 0.75 -> 2.75, at the end 1 bibite is past it.
  The "held" and "recounted" columns show the game's quirk reproduced: at scaling 0.5 the span stays 2.75 while the active count is down to 16 (a recount gives 0.25), so no species is founded until one is created some other way or the Lineage panel is opened.
  The game's own saves hold few active species (the saved `activeSpeciesList` keeps only species with members: `lol.zip` 1, `hyun.zip` 21 of 195), so the cost hardly shows in them (span 1.50 / 1.55); the world in a save is within its span except ~20 % of `hyun.zip`'s bibites (28 of the 97
  past 1.55 are first-step mutants; the rest were born under the larger span the running game had held).
- **Two gaps found and fixed** (tests: `species::tests::the_span_keeps_counting_the_species_that_died_out_until_it_is_refreshed`, `app::sprite_renderer_tests::opening_the_lineage_panel_gives_the_used_span_the_species_that_are_active_then`): (1) opening the Lineage panel did not refresh the span
  (`SpeciesRegistry::refresh_span`, called by `OpenLineage` after the five-record gate, where the game calls `UpdateEffectiveSpeciesSpan`); (2) the inspector's Genes / Brain "distance / span" ratio recounted the active list itself (`species_distance`) instead of using the span the registry holds.
  Not done: a write of a span setting does not recount the species (the game's `OnChange` listener does); the two sliders live in the Lineage panel, which pauses the game and has just recounted, so no count can differ there.
- Not checked against the game itself: no game-side founding rates exist here, only the decompiled rule and the port's behaviour.

## "Info and stats appears over the other menus" and "I can't click OK to create a settings animator" (reported 2026-11-15: FIXED; checked headless and in tests, NOT in the real window)

Report: "Info and stats is appearing over the other menus when it should disappear. Also - and this one matters more - I can't create a settings animator. When I dismiss the info screen, I can't click "okay" to create an
animator".

- **The game hides `Right Panels/QuickStatsHolder` while the settings panel (the pen) is open.** `DynamicSettingsPanel.onOpen` calls `QuickStatsHolder.SetActive(false)` and `onClose` `SetActive(true)` (the UnityEvents of the world scene,
  `wiring_level2.json`); the holder is the "Info +" tab (`OpenButtons`), the Information and Statistics panel and the challenge's tracker. The port left the Information panel (and the Info tab) up over the settings panel: the panel (x 1410 .. 1910,
  y 85 .. 575) covers the settings panel's own Save and Close buttons and the right half of the animator creator, OK button included (x 1451 .. 1551, y 505 .. 540 for the Linear preset): a click on OK landed on the Information panel and
  did nothing. Now `App::quick_stats_hidden()` (the settings panel is open) gates `hud::draw_extra_panels` (the Information panel, the tracker and its tab), `ui_runtime::emit_node_inner` (every node under `/Right Panels/QuickStatsHolder`: the Info tab with
  its tooltip) and the wheel routing of the tags list (main.rs). The Information panel stays "open" in the panel stack (Escape pops the settings panel first, as the game's stack does) and is there again as it was when the settings panel closes.
- **In a window that is not 16:9 (or at a UI scale other than 1) the settings panel's buttons were not clickable where they are drawn** (found while looking for why OK would not answer; `ui_runtime::tests::the_settings_panels_buttons_answer_clicks_where_they_are_drawn_in_any_window`).
  The panel is flush with the right edge of the canvas, which is `window / UI scale` reference px wide: the drawn panel moves right by what the canvas has beyond 1920 px, but the click areas (`dyn_panel::DynHit`, laid out for 1920 x 1080) stayed. At 1900 x 1000
  the creator's OK is drawn at x 1621 and answered at 1451 .. 1551; at 2560 x 1080 at 2129 against 1451; tabs, Save, Close and the sliders' tracks were off the same way. The creator's synthetic nodes also took a vertical shift of their own
  (drawn 360 px lower than the panel at 1600 x 1200). Now the panel's subtree is shifted by (`dyn_panel_offset`, 0) exactly, its click areas, tooltip rectangles and the tracks / graph rectangles that slider actions carry (`dyn_panel::offset_action`) by the same.
  At 1920 x 1080 nothing moved (the headless pictures of the Info tab + pen flows are bit-identical to the build before). The Zones tab's Preview panel (`zone_preview`) follows the panel (`UiFrame::translate_from`).
  Checked headless in a 1600 x 900 window (UI scale 1: a canvas 320 px narrower): clicking the pen, the Animation tab, `+` and OK where they are DRAWN did nothing in the build before and creates the animator now. The unit test uses the panel's texts as markers of where it is drawn
  (the test atlas has no sprites). In the real viewer the UI scale comes from the game's registry prefs, so the canvas is not always the window over a factor.
- **Not reproduced: "OK does nothing after the Information panel is dismissed".** Headless, one frame (`--click` Info tab, pen, Animation tab, `+`, the Information panel's X, OK), the previous build creates the animator; the multi-frame test (a press on one frame, the
  release on a later one built after the app moved on, the window's order of panels) creates it for all three presets with the dropdown in between. The real window was not driven (the player was at the machine). The user's own window is fullscreen (userdata: `Fullscreen: true`), a
  canvas of exactly 1920 x 1080, so the second bullet is not what they hit.
- Tests: `app::tests::the_information_panel_is_not_drawn_nor_clicked_while_the_settings_panel_is_open` (+ `the_right_hand_panels_replace_each_other` extended), `ui_runtime::tests::the_creators_ok_button_creates_the_animator_with_the_information_panel_around` (the whole flow, frame by frame, the three
  presets, both dismissal orders) and `the_settings_panels_buttons_answer_clicks_where_they_are_drawn_in_any_window` (six window shapes). Found there, fixed afterwards (the next section up): the Information panel's hit ids (8800 ..) were also `save_dialog`'s and the
  Biology flow popup's (8800, 8801): with two of them up at once a click on one fired the other's action too (`UiFrame::interact` returns every action of the id).

## Batch of 2026-11-15 (UI): sliders that drag, node values on hover, the game's own Latin names, and "selected bibites starve" (DONE; the window parts were checked headless and in tests, not in the real window)

Four reports of one session: "interacting with the UI at high resolutions is still annoying, especially when trying to drag sliders", "there are also no node values when hovering over them", "I have a paranoid suspicion that every
bibite I open the panels for has an increased energy cost, causing them to quickly starve", "I want the ACTUAL latin names from the game being used instead of our placeholders".

- **Sliders of the title menu could not be dragged at all.** The Settings pages and the New Game screen's five TPS sliders took the press only (`MenuState::click_native` set the value at the press point and forgot the button): on a
  Settings page the press was even held back for a drag of the page (`press_hold`) and given back at the button's release, at the point where it went DOWN, so a drag on a slider did nothing, and a drag that drifted ten pixels
  up or down scrolled the page instead. Now a press on a slider (`MenuState::slider_under`) is the slider's: it is set where the press lands (or, when the press is on the handle, not at all: see below) and held
  (`HeldSlider`, `slider_drag_move`), the pointer moves it wherever it goes until the button is let go, the page does not scroll, and the pause menu's Settings hand every step to the running world
  (`apply_user_settings`); `UserSettings::set_fraction` writes the settings file only when the value changed. Both window loops of `main.rs` follow the held slider.
- **A press on a slider's handle no longer makes the value jump** (the game's sliders are Unity `Slider`s: `OnPointerDown` on the handle only records the pointer's offset, one on the track puts the handle under the pointer). Done for
  the title menu's sliders (`slider_grab`), the time-speed slider and the settings panel's rows (`App::begin_slider_hold`, `speed_slider_grab`, `dyn_slider_grab`: the slider changes nothing until the pointer moves,
  then follows the pointer's travel); the species Position slider and the Gene Analysis time slider already worked like that. **Not done:** the zone, material, default-bibite, tower, pellet-placer, gene and brain-editor
  sliders still jump to the pointer on a press on their handle (each needs its value's place on the track at the press).
- **High resolutions: what grows with the window is the cost of a repaint of a title-menu screen (4.6x cut, bit-exact).** Nothing in the pointer maths depends on the resolution (a slider's track is in reference px, the pointer is divided
  by the UI scale, a window of any size and DPI scaling gives the same 1920 x 1080 canvas). What does grow is the software painter: every title-menu and pause-menu screen is painted on the CPU at the window's pixel size (capped at
  2560 x 1440) on EVERY repaint, and a change of the hovered control or a slider step is a repaint. Measured headless (`--screenshot` of a menu prints `menu-render:` with the first pass and the repaint; `RUSTIBITES_UI_PROFILE=1`
  prints what each kind of draw cost): the Settings page took 118 ms at 1280 x 720, 251 ms at 1920 x 1080 and 455 ms at 2560 x 1440 (2 pictures a second), 82 % of it in 40 nine-slice draws: a panel's middle is one colour and was painted
  pixel by pixel, each through a blend with three runtime divisions. Now (`render.rs`: one `blend_over`, flat fills for opaque colours and for regions of one colour, the clip applied once per draw, the sampling worked out once per column and row,
  the bilinear filter skipped where its four texels lie in a one-colour block, a blend over an opaque pixel without the runtime divisions, the final copy at one sample a pixel) the Settings page repaints in 29 / 55 / 96 ms (best of five, 720p / 1080p / 1440p), Load
  and Tools and Credits in 40-60 ms at 1080p, the scenario editor in 17 / 35 / 63 ms, the Library in 11 / 22 / 39 ms. **Every picture comes out byte for byte as before**: the 14 menu screens measured (settings, editor, new, load, library, achievements, tools,
  credits at 1080p; settings, editor, library at 720p and 1440p) and three in-world HUD pictures (Brain, Biology, Stats) have the same hash / PNG bytes as the previous exe, and the tests compare each fast path with the per-pixel
  blend (`render::zoom_tests`: flat fills with and without a clip, texel draws, filtered draws, the opaque-destination blend for every alpha and many colours). **Still open:** a repaint is a whole picture; a drag step at 1440p costs ~100 ms
  (10 pictures a second) and the editor ~60 ms. Painting only the rectangle that changed (the slider's row, the hovered control) over the kept picture would make it smooth at any size, but it changes the window loop's texture
  handling and needs a real-window check (not done: the player was at the machine); a banded multi-threaded painter is the other lever. Headless check of the drag glue: `--menu --menu-view settings --menu-drag 989,445,1100,445 --screenshot x.png`
  (a press on the Pheromones Strength handle, the pointer dragged right, the button let go, through the window's own calls: 50 % -> 83.2 %).
- **Node values on hover.** In the game the inspector's Brain tab shows the value of the neuron under the pointer: `BrainPanelCommon.InitPanel` makes its neuron pool with `showValueOnHover = false`, but `BrainPanel.InitPanel` then
  REPLACES the pool with one that has the prefab's default (on), so `UINeuron.OnPointerEnter` turns `ActivationText` on (16 pt, `Round(100 * value) / 100` as a .NET float, refreshed every frame) in the neuron's text holder (a
  horizontal layout group of spacing 5, fitted to its contents, its bottom middle on the neuron's top middle: the name, when shown, and the value stand side by side, centred together). The species panel's Brain tab keeps
  `showValueOnHover = false` (a tooltip only) and the Expanded View's neurons show the prefab's "0.00" for ever (`ExUINeuron.OnPointerEnter` never sets `isHovered`): both unchanged. The port drew only the tooltip in all three. Now
  `DiagramDraw.value_on_hover` (the inspector's) draws it (`species_panel/brain.rs`, `neuron_value_text`); the topmost neuron under the pointer only.
- **"Every bibite I open the panels for starves faster": not found, and shown not to happen in the viewer's frame loop.** The core has no notion of a selection (nothing in `World::tick` reads one); the viewer writes a bibite's
  state only for the drag of the body itself (velocity, rotation) and the explicit actions (kill, revive, lay an egg, tag), none of which a panel does; the inspector's Brain tab reads `record.brain` through `graph_view`, which
  copies. Test `selecting_a_bibite_and_opening_its_panels_leaves_the_simulation_bit_identical`: two identical worlds through `App::handle_input` + `App::advance` for 800 frames, one untouched and one with its first bibite
  selected, each of the four inspector panels opened in turn and the pointer moving over the screen: the selected bibite's energy and the state fingerprint of the whole world are the same bits at every check. What can look like
  it: a bibite is opened because it is the one that looks interesting (often one that is already low), the camera and the Biology panel show its energy falling in real time, and a press on a bibite held longer than 0.15 s
  drags it (the game does too) instead of selecting it.
- **The game's own Latin names.** The words species are named after are the game's asset `Resources/Patrons/patreon_supporters` (`resources.assets`), not in the decompiled source, and the port had 50 Roman names of its own
  (D10). `tools/extract_latin_names.py` reads the asset the way `PatronsImporter.LoadPatronsInfo` does (the four entries written in the C#, then the CSV rows, ordered by descending pledge) and writes only the `latinized` column,
  866 words in the game's order, to `crates/bibites-core/src/latin_names.rs`; `species.rs` draws from it (`WeightedRandomPatron`: `1 - U^w` of the list, w = 4 for a genus, 1.5 for an epithet, so the END of the list is what
  species are mostly named after). The game's quirks come with the data: 172 words end in a space (a species can be "Corbinus  waldentimotei "), a few have `ö` or a hyphen, and the "genus already used" check never matches
  (it compares the raw lower-case word with the stored capitalised genus). The golden `birth_path_is_bit_identical_to_pre_optimization_golden` no longer hashes the names (re-pinned 0x1deb_4e72_9188_fe19; with the old
  list put back, the vector with the names was bit-identical to the old golden 0x8dbf_8f7e_8145_a174: nothing but the names moved). The names a run makes differ from a game's only by the random draws (the port's own name stream).

## Bibites thrown at 400 u/s by a weld to a giant pellet: "meat that kills upon touch" (maze save, reported 2026-11-15: FIXED)

**What the player saw.** "Bibites who die from getting stuck against an inf-mass pellet spawn meat that immediately kills bibites upon touch." The save (`maze bug bug bug.zip`, 873 bibites, a maze of 124 plant
pellets of 300,000 to 500,000 u^2 whose collider radius is 307 to 395 units; the save's Plant material has `massDensity` 1e35 and `cohesiveness` 1e11, so a pellet's mass is past the largest f32: an infinite mass, a wall)
reproduces it from the start.

**What happens.** A mouth grabs anything it touches (the game has no limit of mass, and a joint never breaks): a bibite that touches a wall is welded to it, cannot move, and starves there (203 of the 674 deaths in the first
10,000 ticks were bibites that had not moved 15 units in 100 ticks and lay against a rim, 182 of them out of energy: "stuck against an inf-mass pellet", and in the game too). Their meat lies pressed against the rim. When a
second bibite grabs that meat while it is welded to the wall (tick 8860 of the save, seed 1: the bibite held 2 things, the meat and the wall), the weld solver threw it at **434 u/s spinning at 414 rad/s** (the meat followed at
360 u/s a tick later), and what such a bibite hits is hurt by a collision of a speed that no bibite has.

**Cause.** The weld's anchor (`weld_arms`, `physics.rs`) was the **held body's origin**. For a wall that is 364 units from the holder: the rigid weld's 3 x 3 block (`b2WeldJoint`'s: the two anchors' coincidence and the angle) had
terms of 7,000 and a determinant of 0.02, and solving it by cofactors in f32 left the holder turning at 34 rad/s after the solve that was to stop it; the step then turned it 0.3 rad (a hundred units at the anchor), the relax solve
that follows (no clamp after it) answered with 2,500 u/s and 570 rad/s, and the next step's warm start carried it on. Found by dumping the island's state after each phase from a checkpoint of the world 5 ticks before (a few seconds
to replay; the dump is gone, the examples stay).

**Fix.** The anchor is the **holder's origin**, as in the game (a `FixedJoint2D` added to the holder: anchor (0, 0), connected anchor auto-configured to the same place): the holder's arm is its own centre-of-mass offset, the held body's arm
is the distance to the holder, which a body of infinite or huge mass does not feel. The frame the weld keeps (`anchor`, `angle`) is unchanged: the holder's origin in the held body's frame is `(-anchor).rotate(-angle)`. And the rigid step
clamps the velocities once more after the relax pass (it clamped before the position integration only), so nothing leaves a step with a weld faster than the physics' limits (400 u/s, 6.3 rad/s) whatever else goes wrong.
Result on the save, 10,000 ticks: no bibite faster than 149 u/s or spinning over the 6.3 rad/s limit (before, the throw at tick 8860), no pellet faster than 82 u/s (before, 360 u/s); the deaths are the same kind (692 against 674; stuck bibites 238
against 203: the run is not the same run after the first weld). Tests: `a_light_body_welded_to_a_giant_one_far_off_is_not_thrown_by_its_own_turning` (`physics.rs`: the live geometry, an infinite-mass giant, fails without the fix: the holder spins
at 6.28 rad/s), `a_holder_of_a_wall_that_grabs_the_meat_pressed_against_it_is_not_thrown` (`body.rs`: the whole flow; calm before the fix too). Tools: `cargo run --example maze_probe -- <save> <ticks> [seed] [threads] [fast] [drop] [trace_tick x y]` (deaths,
fast pellets, thrown bibites, the meat of each death) and `maze_replay snap <save> <tick> <out.state>` / `play <save> <state> <ticks> <x> <y>` (a checkpoint of the world, and a replay of a few ticks from it).

**Not changed.** A bibite welded to a wall stays welded (as in the game: a held body is only let go of by the holder, and a joint never breaks), and starves there. 3 bibites of the save start inside a wall and meat is born inside walls
(8 pieces in 10,000 ticks, up to 140 units deep): they are where the save put them or where an egg hatched; the physics does not move them out of a body of infinite mass.

## Multithreading sized to the work, then measured (requested 2026-11-15 as "ALWAYS run": DONE)

**What it was.** The worker pool is rayon's, and a world has the worker count of `--threads` (0 = every core, the default, in the window and in the CLI). But a stage of the tick used the pool only from a size
(`ParallelGates`: 3,000 bibite records, 4,096 bodies, 20,000 due bibites for sensing, and three more that were literals in `physics.rs`), so a world of a few hundred or a thousand bibites ticked on one core.
In the window a frame's ticks went to the tick helper thread only when they took 0.5 ms or more.

**What it is now.** A world with more than one thread **measures how its ticks run fastest on the machine it is on** (`pool_tuner.rs`; the owner: "the policy should be good for all machines"): where the pool starts to pay (about 1,200 bibites on the
6-core / 12-thread machine of the first measurements, from a few hundred on an emulated pool of two to six workers) and how eagerly the stages of a pooled tick should use it (on 2 to 6 workers every stage from one item beat the sizes measured on 12 workers by 6-19 % at 500-2,000 bibites) are properties of
the machine, and a size written into the code is right on one at best. It chooses among five ways: *inline* (the calling thread with no handle to the pool, as with `--threads 1`) and *the pool at four levels of eagerness* (the whole tick on a pool worker; the stage sizes of `ParallelGates::SCALED`,
which keep the stages that measurably lose in worlds of a few thousand bodies off the pool, times 1, 0.3, 0.08, and every size at 1: `ParallelGates::pooled_level`). The world times its own ticks, keeps the fastest way, and looks at the others for 12 to 100 ticks now and then (each waiting twice as long
after each loss, up to 8,192 ticks: about 1 % of the ticks at most); a way has to win two looks in a row to take the world (timings are noisy in one direction), the pool has to take under 90 % of the inline time (it takes cores from everything else) and inline takes back within 5 %; the measuring starts again
when the world's size changes by 41 %, and a world under 300 bibites (750 bodies) is never timed (its tick takes a fraction of a millisecond). 1,200 bibites / 3,000 bodies are only the first guess. `RUSTIBITES_GATES=fixed|pooled:<level>|always|eager|nosleep|scaled:<percent>` picks other policies in the viewer
and the CLI (`fixed` = the old switch, no measuring; `pooled:<level>` = one level for every world, to measure the levels one by one). The window hands every frame's ticks to the helper (the
first frame, before a tick has been timed, runs its own). The startup log (`logs/last_run.log`) has a `threads:` line that says how many workers the world has and which sizes. Results are bit-identical to the serial
tick (tests: `the_gates_a_world_starts_with_size_the_pool_to_the_work_with_the_serial_worlds_results`, `a_world_with_no_bibite_or_one_ticks_on_the_pool_as_it_does_serially`, `the_gates_of_the_store_reach_the_physics_step`,
`a_world_starts_with_gates_that_size_the_pool_to_the_work`, `a_stage_runs_inline_below_its_size_and_gets_a_worker_for_every_quarter_of_it_above`, the column fill and the pooled step from small sizes, and the existing byte-for-byte
checkpoint comparisons).

**What it costs and gains (measured, `docs/BENCHMARKS.md`, "Multithreading sized to the work").** Ticks per world size against `--threads 1`: 1.00x (within 1-5 %) up to 1,000 bibites, 1.10x at 1,500, 1.21x at 2,000,
1.61x at 3,000, 1.96x at 5,000, 2.74x at 10,000, 3.42x at 30,000 (the old sizes, at those same six sizes: 0.88x, 0.87x, 1.23x, 1.75x, 2.36x, 3.33x). The literal "always" (every stage of every world on the pool) was measured first and was slower than
one thread up to 1,200 bibites (5x slower at 50, 2.6x at 200, 1.7x at 500): the stages of a small world cost less than the hand-off to the pool.
Real window (the static alpha build of the final code, 1280 x 720, `--speed 30`, the window's own "counted here" ticks per second, two runs each, the player away; `scratchpad/win_scaled_ab.ps1`; before = the viewer before this change, always = `RUSTIBITES_GATES=always`, default = scaled): 200 bibites 907 / 907 / 905 ticks/s (before / always / default) and 0.55 / 4.26 / 0.63 cores; 1,000 bibites 900 / 719 / 902 ticks/s and 112 / 23 / 109 fps (always: the frame waits 42 ms for its ticks); 2,000 bibites 337 / 414 / 382 ticks/s (15 fps in all three: the frame is drawing-bound there); Butterfly 6 (2,450 bibites, 10,100 pellets) 164 / 213 / 201 ticks/s, 2.3 / 4.1 / 3.2 cores (the default +23 % over before, always +30 %). So in the window the default keeps the small and middle worlds exactly as they were and gains from about 2,000 bibites; the literal version is a few per cent faster still above 2,000 but takes 4-5 cores for a 200-bibite world and drops the 1,000-bibite world from 112 to 23 fps.

**Checked on, and not.** The measuring was checked on this machine at its full 12 threads and on **emulated** smaller ones (the process restricted to 2, 4 and 6 of its logical processors with the pool sized to match; `docs/BENCHMARKS.md`, "The line and the levels are measured"):
within 0-6 % of the best fixed way at every size from 300 to 5,000 bibites, 9-25 % faster than the old fixed switch at 1,000-2,000 bibites in runs of 12,000+ ticks, and bit-identical to the serial tick. **Not checked:** other hardware (hybrid P/E-core CPUs, 16-64 thread workstations, laptops that park
cores: the tuner measures whatever it finds, and what it costs where nothing beats inline is a few per cent of the first thousands of ticks and about 1 % later), a machine whose load changes during a run (it would follow within 8,192 ticks), and a real window beyond the four worlds of the BENCHMARKS table (200, 1,000, 2,000 and 2,450 bibites: no loss at 200 and 1,000, +4.5-5.6 % over the old switch at 2,000 and 2,450). An intermediate build of the first
version was 9 % slower than the previous one on small worlds for a reason that is not the pool (code layout of the 16-codegen-unit perf profile; fixed with `#[inline]` hints, see `docs/BENCHMARKS.md`).

**Not changed.** `--sim-thread` stays opt-in (a world on its own thread cannot be read by the panels; achievements checks do not run).

## Plant (or meat) decay switched on while a world runs did nothing to the plants already there (reported 2026-11-16: FIXED)

**What the player saw.** "Plant decay isn't working": the Decay switch of the Plant material (the in-game settings panel's Energy tab, the pen) was turned on in a running world and the plants did not rot.

**Cause.** The game's `SettingsUpdater` acts on the pellets that exist when a material's decay settings change: the Decay switch registers every pellet of the material with the `MatterDecayProcessor` (`TryAddUnique`: a fresh timer from the material's freshness time, no rot) or removes it (`TryRemove`), and a new Freshness Time adds its difference (new - old) to the fresh timer of every decaying pellet of the material (`UpdateFreshValueDifference`, `UpdateDecayParameters`). `World::apply_settings` only swapped the material table, so only pellets SPAWNED afterwards followed it (`try_spawn` reads the table); the registration functions existed (`register_decay`, `update_decay_parameters`) and nothing called them. A world that STARTS with plant decay on was always right (a test pins it), and a save restores each pellet's saved decay state as the game does (read against `SaveSystem.LoadPellet`, not run).

**Fix.** `PelletStore::replace_materials` (called by `World::apply_settings`) does what the game's handlers do for Plant and Meat, in the order the game's settings fire them (the Decay switch, the freshness time, the decay rate), each against the table as it is at that moment: `set_material_decay` registers or unregisters every pellet of the material, a freshness difference is kept and added to the decaying pellets' timers. Fat and Armor have no such handlers in the game. One quirk of the game is kept on purpose: `SettingsUpdater.freshValueDifference` is a field that stays, and the decay RATE's handler runs `UpdateDecayParameters` with the last kept difference again, so a decay rate edited after a freshness edit moves the timers of the decaying pellets once more (by that same difference, whatever the material of the earlier edit). Tests (`world::tests`): a world that starts with plant decay on rots its plants; switching it on while the world runs registers the plants already there (fresh timer = the freshness time, no rot) and they rot away; switching it off stops it, and on again starts the timer over; a new freshness time moves the timers of that material's decaying pellets by the difference and not the other material's; the rate's repeat of the last difference; an edit that leaves the decay settings alone touches nothing. The core suite passes (675, 16 ignored) and so does the pinned 12,000-tick fingerprint (nothing in a run without a live edit changed). **Not checked:** a real window (the edit path from the panel to `apply_settings` is `edit_material` -> `dyn_edit`, read, not driven), and the game itself side by side. With the default plant numbers (fresh for 100 s, then 0.005 u2/s) plants rot slowly in the game too: a 20 u2 pellet takes over an hour of game time to be gone.

## A pure black bibite drawn black all over WITH the GPU colour program (reported again 2026-11-16, reproduced in the real window: FIXED)

**What the player saw.** The player ran a save of one bibite whose three pigment genes are exactly 1 (a template "Black Bibite", save `GLITCH`, EyeOffset 0.5) in the default window: the whole bibite, the outline and arms too, was black, only the cyan eyes coloured, like the tester's picture of 2026-11-14. The earlier fix did not cover it: that window HAD the GPU colour program (`renderer: GPU hue-match shader enabled`).

**Reproduced and narrowed (real window, framebuffer shots with `RUSTIBITES_SELF_SHOT`, the GPU colour program against the software shot at the same camera and size).** A save of twelve bibites with the pigments 1, 0.98, 0.95, 0.9, 0.85, 0.8, 0.7, 0.6, 0.5, 0.3, 0.1 and 0 (and the twelve-colour fixture of `tools/parity/bd_contacts/piles/make_colors.py`): every cell matched the software shot (mean luma within 3 of it, the same offset in every cell) EXCEPT the tint of exactly (0, 0, 0): luma 6 against 115, the whole sprite black. `--no-gpu-hue-shader` (the processor recolour) was right for all twelve, and so was the headless software shot of the same save. So the maths of the skin shader is right; one input value, a black tint, broke it on the card.

**Cause.** `rgb2hsv` in `HUE_FRAGMENT_SHADER` divided by `q.x + 1.0e-7` and `6.0 * d + 1.0e-7`. For a black tint (q.x = 0, d = 0) that is 0 / 1.0e-7 = 0 in the maths, and in the rust replica that the tests compare with the software skin, which therefore passed. In the real window every texel of such a sprite came out black while a tint of 0.02 was right; a lost epsilon (0 / 0 = NaN, which hsv2rgb turns into black) fits every measurement, but which step of the card does it was not isolated (the fix removed the divisions it could be, and the window then matched the software shot). A reaper's colour times (1 - killer) is a tint of exactly 0 too.

**Fix.** `rgb2hsv` has no epsilon any more: `float hue = d > 0.0 ? abs(q.z + (q.w - q.y) / (6.0 * d)) : 0.0; float sat = q.x > 0.0 ? d / q.x : 0.0;` (the same numbers for every colour with d > 0, and exactly the hue 0 and saturation 0 that the epsilon stood for otherwise). Test: `the_hue_shader_has_no_epsilon_to_lose_and_keeps_a_pure_black_bibites_grey_outline` (the source holds no epsilon, the replica gives the light outline texel grey and the core dark for tints of 0, 1/255 and 5/255, nothing NaN). Checked in the real window with the rebuilt alpha exe: the pigment lattice (all twelve cells, the black one 118 against the software's 115.5 like the others) and the player's `GLITCH` save at two zooms (dark core, mid to light grey outline and arms, as in the game).

## A black bibite drawn black all over in a window without the GPU colour program (reported 2026-11-14: FIXED)

**What the player saw.** A tester's bibite (its three pigment genes had evolved up to 1, "a shadow"; the tester plays game 0.6.3.0) drawn solid black: the outline, the arms and the tail as black as the core, only the eyes coloured and small. In the game a bibite with the pigments at 1 (body colour (0, 0, 0)) has a black core but a mid to light grey outline and arms (`testdata/parity/_work/black_bibite_game_vs_port.png`, 2026-11-12: the port equal to the game), so the picture was far too black.

**Cause.** The extracted art is one blue hue and the skin shader recolours it. The window does that with the GPU hue material (`GpuHueMaterial`); without it (the driver rejects the shader: `warning: GPU hue material failed: ...` in `logs/last_run.log`; or `--no-gpu-hue-shader`; the log's `renderer:` line says which) every skin, eye and egg sprite was drawn as art times tint: blue bibites for any colour and, for the tint (0, 0, 0), black all over. A throwaway probe (a pigment-1.0 bibite of each size tier, through the shader and drawn the way that window drew) gave the reported picture: a black silhouette with small bright eyes. The shader's maths is not the cause (`the_hue_shader_formulas_equal_the_software_rasterisers_skin_shader` covers the pigment lattice, (1, 1, 1) included), and neither is the tester's game version: nothing in the game's code is keyed to 0.6.3.0 against 0.6.3.1 (its thresholds are 0.6.2.1 and 0.6.3.3), the colour is `GenesToColor` of genes 5, 6 and 7, and the 0.6.3.1 patch notes (a corpse setting, Red Death settings, template fixes) say nothing on colours.

**Fix.** `Painting::Recoloured` (`render.rs`): without the hue material, and with the software atlases, a skin, eye or egg sprite is drawn from a copy of its cell of the sheet that `SkinTint` has recoloured (`recoloured_cell`, with the reaper's `1 - k` colour and alpha and the hurt flash in eighths as `fill_sprite` has them), made once per cell and tint and kept (`RecolourCache`: 8192 cells at most, cleared when the shader globals change), and drawn with the default material through the default draw's place, turn, flips and inset. The special skins and the big bent bodies come from the software picture also without the material (they were the sheet times the tint). The startup log says which path the window is on. Tests: `a_recoloured_cell_is_the_skin_shaders_colour_of_every_texel_with_the_reapers_and_the_flashs_amounts`, `a_black_bibites_recoloured_cells_keep_the_greys_of_the_game_and_are_not_black_all_over`, `the_recolour_cache_makes_each_cell_once_and_starts_over_when_the_shader_globals_change_or_it_is_full` and the fallback's cases of `a_sprite_drawn_from_a_picture_does_not_get_the_colour_material`. Not checked in the real window (`--no-gpu-hue-shader` there runs this path); why that tester's window is without the GPU program is not known (the log says).

## A bibite saved as a template was nowhere in the Library (reported 2026-11-14: FIXED)

**What the player saw** (a tester on the alpha, started with `Play.bat`): "does the bibite template save function work? the 2 bibite species I saved are nowhere to be found; they are in the user data Bibites folder, but that is not recognized. Do I have to copy them to the real Bibites folder?"

**Cause: two separate things.** (1) The Species panel's Save button (`SpeciesInfoPanel.SaveSpecies`) was a direct file write to `Bibites/<name>.bb8template`. The game calls `SaveController.SaveTemplateOfBibite`, which opens the Save Bibite dialog (the template's name and description; "Save as template" on and not interactable: `OpenDialog(allowSaved: false)`) and saves to `Bibites/Templates/<name>.bb8template` (`SaveBibiteDialogHandle.GetSavePath`). The Library lists the `*.bb8template` files of `Bibites/Templates` and the `*.bb8` files of `Bibites`, so a template lying next to the saved bibites was never listed (by this port, or by the game). (2) The title menu's Library (also the in-world one, and the names of the default bibites) listed the installed game's folder (`%USERPROFILE%\AppData\LocalLow\The Bibites\The Bibites`) whatever the port saves to: `Play.bat` sets `RUSTIBITES_DATA_DIR` to the zip's private `userdata` folder, so everything saved there (a bibite saved from the Stats panel as a template too) was invisible to the Library. With the game's folder as the content root (the owner's setup, `Play-with-my-game-saves.bat`) only (1) showed.

**Fix.** Save on the Species panel opens the Save Bibite dialog (`save_dialog::Target::Species`, `SaveDialog::allow_saved`): the species' name and notes, "Save as template" on, drawn with the disabled tint and with no click (switching it on stores the player's choice like a click on it, `saveAsTemplate.onValueChanged`), the game paused; Save writes `Bibites/Templates/<typed name>.bb8template` with the typed description and the generation of the species' first specimen. The dialog is drawn after the Species panel (it was under it) and blocks the lineage view's wheel and drags. The Library lists the folder the port saves to first (`StoragePaths::default_content`), then the installed game's folder when that is another one (`menu::library_roots`): a `Play.bat` Library shows its own bibites and still the player's. "Open folder" opens the first. Tests: `species_panel::tests::save_opens_the_save_dialog_on_the_species_and_writes_its_template_where_the_library_lists_it`, `menu::tests::the_library_lists_what_the_port_saved_in_its_own_folder_before_the_games_bibites`, `save_dialog::tests::a_species_template_is_saved_as_a_template_and_its_box_is_locked`.

**What to do with the files an earlier build wrote.** `Bibites/<name>.bb8template` (next to the saved bibites) is not listed, as in the game: move such files into `Bibites/Templates/`. Not checked in the real window.

## The left arm of large and huge bibites hovered off the body or lay on it (reported 2026-11-14: FIXED)

**What the player saw.** Bibites drawn with the large sprite tier had their left arm hovering a little off the body, and bibites with the huge ("giant") tier had it lying across the body (two window pictures); the right arm and the normal tier looked right. The four tiers are picked by size (`ClosestSizeIndex`: tiny 5, normal 9, large 15, huge 19 pixels wide, `ppu = pixelWidth / 9`).

**Cause.** A constant. The game's prefab hangs `RightArm` (`flipY`) at x = +4.5 and `Left Arm` (no flip) at x = -4.5, both turned 90 degrees, with the arm sprite's pivot at its bottom right corner (Unity's, y up; the arms' `(1, 0)`), and Unity turns a flipped sprite about its pivot: that, the tier's own pixels per unit and nothing else are in the game (`BibiteProceduralSpriter` sets only the sprites; the renderer bounds of the live game's six-layer dump, `assets/game/reference/default_save/bibite_layers.json`, 11 bibites, are reproduced to 0.0001 by it). The port's renderers (the software one and the window's `draw_texture_ex`) count a pivot from the TOP and flip the picture inside the sprite's own rectangle, and the world path passed the sprite's pivot to them as it was (the comment in `sheet_pivot_v1` said the manifest's pivot was already top-left): the flipped right arm came out right, the unflipped left arm sat one arm's THICKNESS too far in. An earlier session measured that on one normal bibite as "an extra local offset of (-3.002, -0.016) that the prefab lacks" and hard-coded it for every bibite: right for the normal tier (arm 9 x 3 pixels at 1 pixel per unit: 3.0 units thick) and wrong for the others, whose arms are 5 x 3 (tiny, 5.4 units thick), 17 x 4 (large, 2.4) and 23 x 8 (huge, 3.8). Against the game: the large left arm 0.6 units too far out (a gap), the huge one 0.8 too far in (on the body), the tiny one 2.4 too far in (all times the bibite's size).

**Fix.** `sprites::drawn_pivot` turns Unity's pivot into the renderers' by the layer's flips (an unflipped sprite is drawn round `1 - y`, a flipped one round `y`; the body's, mouth's, eyes' and exoskeleton's pivots are on the middle line and do not change), `bibite_sprites` uses it and the constant is gone; the Library's and the editor's preview, which had worked it out for itself (`preview.rs`), uses the same function. The window and the picture share the one source, so both are fixed. Checked against the game's geometry for all four tiers: `render::tests::the_arms_of_every_size_tier_hang_from_the_body_where_the_games_prefab_puts_them` (each arm is a rectangle as long as the arm sprite behind the joint and as thick as it is outside the body's side, the right one the mirror image of the left one; `ARMS_DUMP=<dir>` writes a picture of the four tiers) and `sprites::tests::a_unity_pivot_becomes_the_rasterisers_by_the_sprites_flips`.

## Ticks per second can be edited while a simulation runs, and the Load click no longer freezes the window (2026-11-14: FIXED)

**Ticks per second in the middle of a simulation.** The game applies `simTPS` live: the in-game panel's "Simulation Performance Parameters" group has the slider (`MenuDynamicSettingsManager.TPSParameters`), and `TimeController.UpdateFixedDeltaTime` and `NEATBrain.UpdateSimTPS` subscribe to the setting (the helper text only says that changing it after the start is "not recommended, aside from testing purposes"). The port refused the edit: `World::apply_settings` answered "simTPS change requires a new world (clock/hash are prebuilt)", so the slider showed an error line and snapped back (the spatial hash is built from the bounds and the cell size, not from the tick rate). Now `apply_settings` takes it: `FixedClock::set_tps` gives the next ticks `1 / tps` seconds and keeps the tick count and the time made so far (`time_secs` stays continuous: it is the time before the change plus the ticks since), `World::tick` and everything that reads the rate per tick (the bodies' energy and ageing, the brains' period `brain_tps / sim_tps`, the physics step, eggs, pheromones, decay, the spawner) follow from the next tick, and the simulated time goes on from where it was. `World::set_sim_tps(n)` is the one entry point for code that is not the panel (the old simple controls wrote `settings.sim_tps` directly: they use it now; `World::tick` also follows a direct write, so the clock and the bodies cannot disagree). The panel's typed number, the slider and the reset all land in `dyn_edit`, so all three work, and so do undo and redo; the time panel's tps and sim/real follow by themselves (it measures the ticks that really ran). Like the game's setter it does not clamp to the slider's 5 to 100, but a rate is never less than 1. Tests: `clock::tests::a_rate_change_keeps_the_ticks_and_the_time_made_so_far`, `world::tests::the_ticks_per_second_change_while_the_world_runs` (30 -> 60 -> 15: ticks, simulated time and a bibite's age), `a_tool_that_writes_the_rate_itself_is_followed_at_the_next_tick`, `the_other_performance_factors_change_while_the_world_runs_too`, `any_rate_is_taken_but_never_less_than_one`, and in the viewer `the_ticks_per_second_can_be_edited_in_the_middle_of_a_simulation` (a typed value through real frames: the time keeper reads 30, 60, 15 tps).

**Clicking Load froze the window.** The player's list holds 1,557 saves: 44 saves and 1,513 autosaves (4.97 GB; the player's own `AutoSaveFiles` setting keeps up to 2,000). The click on the title menu's Load ran `report_corrupted_saves` before the window could repaint: it opened every zip, read the scene and parsed it (`LoadGamePanel.FillList`'s "Corrupted Save" popups). A Python replica of that check takes 16.6 s over these files (9 ms for a normal autosave, 0.95 s for the 695 MB `glitch claude.zip` whose directory has 264,309 entries), and finds the one truncated autosave; on the window's thread that is what Windows calls "not responding", and a cold disk cache makes it worse. Then pressing Load on a save read the whole world on the window's thread too: 1.8 s for 507 bibites, 4.3 s for 5,563, 5.3 s for 7,823 (these are the player's own autosaves, headless, UI start-up included), minutes for the 157 to 567 MB autosaves of the glitch run. Now: the saves are checked on a `save-check` thread (`MenuState::report_corrupted_saves`, polled once a frame by `poll_background` in the title menu and in the in-game panel; the popups come up as it finds the unreadable ones, in the list's order; opening the list again stops the old check, and so does a world starting to load or the menu going away, because the rest of the saves would only compete with the world's own read for the disk; a verdict is kept per file by size and time of writing, so the second opening costs nothing; the Save panel uses the same remembered verdicts), and the world is read on a `world-loader` thread while the Loading screen goes on being drawn and the window answers (Escape leaves the screen and drops the result). `reload_saves` no longer opens each file twice, and the list's frame no longer clones all 1,557 rows (once per text of the screen) to draw sixteen. `--time-load-list` times the click on this machine's own folders: the click returned after 11 ms, the check of every save ran beside the window for 9.0 s more (1 unreadable, one popup each), and the old way (every save read before the click returned, on files the check had just warmed) took 8.5 s. The log says `load: <file> took N s` for the world's own read and `load: the world is open after N s more on the window's own thread` for what is left of the opening on the window's thread (the app around the world), so a report names where any wait is. Not done: a progress bar on the Loading screen; the detail card of a save the player selects (one zip's directory, up to a second for the 567 MB ones); a load of a 200,000-bibite world still takes minutes, but the window answers meanwhile. Tests: `menu::tests::the_saves_are_checked_beside_the_window_so_the_click_on_load_returns_at_once`, `the_check_of_the_saves_stops_when_a_world_starts_loading_and_when_the_menu_goes_away` and the corrupted-save popup test (it now waits for the check).

## Tester crash report of 2026-11-13 (a 3.9 GB / GTX 1050 Ti machine: exit code 0xC0000005 right after a world was opened; NOT reproduced) and the report script's invalid regex (FIXED)

The report zip (`RUSTibites-report-20261113-184140.zip`, alpha c8c2488, a Brazilian Windows 10, 3.9 GB RAM, Ryzen 3 1200, GTX 1050 Ti, Fullscreen off): `logs\last_run.log` ends with `exit code -1073741819` = 0xC0000005 (an access violation), right after a second `storage:` line. That line is printed when a world is opened from the title menu (`use_game_storage` runs for the new `App`), so the crash is in the first frames of the world; the run before it (16:20) ended with 0 and never opened one. There is no panic message and the release profile aborts on a panic (`panic = "abort"`: the message first, then 0xC0000409), so this is a native crash outside our Rust code, most likely in the graphics driver. Not reproduced here (GTX 1660 SUPER). A headless world render needs only about 210 MB, so memory alone is not obviously it (the window adds the GPU textures and the sim thread: not measured on a 4 GB machine).

What the development machine's own Windows record says (`Get-WinEvent` of the Application log, provider "Application Error", the fields read by position): 8 records in 45 days, 7 of them processes that died in their first seconds (panics of test runs, all 0xc0000409) and ONE long run: the Desktop exe (c8c2488), started 20:07, failed after 104 minutes at 21:51 on 2026-11-13 in `nvoglv64.dll` 32.0.15.8157 (NVIDIA's OpenGL driver), exception 0xc0000409, fault offset 0x108f0cd. So two crashes of the same kind on two NVIDIA machines; the cause is unknown. Suspects, the cheapest first: the window's picture path (a bent body seen big is drawn from a software picture) makes a new GL texture for every such sprite every frame (macroquad frees them at the end of the frame: a churn of thousands a second, not a leak); the 2x multisampled surface (see the section below); the GPU colour shader on an older driver (`--no-gpu-hue-shader` leaves it out). Next step: the tester's crash record.

Fixed on the way: `tools/release/alpha/report.ps1` line 9 used `-notmatch '\assets\'`, which is not a valid regular expression (a backslash at the end of the pattern): the picture search threw "OperationStopped" twice and the report went out without screenshots. Now `-notlike`, and the search does not walk `assets\` at all (thousands of pictures); the report also has free RAM and commit, the language, every graphics adapter with its driver version and date, and `crash_events.txt` (Windows' "Application Error" records of the program: faulting module, exception code, offset); `-OutDir` writes the zip elsewhere than the Desktop (tests). `play.ps1`: the exit code is shown in hex (0xC0000005), the log gets the crash record's faulting module, and `-ViewerArgs --a,--b` / `'--a --b'` are split (from a .bat they arrived as one string, so two flags could not be tried together). README: a short "if the window closes by itself" section (what to send; `Play.bat -ViewerArgs --no-gpu-hue-shader` and `--no-overlap`, one at a time, to see which stops it).

## The faint dotted lines round a bibite's sprites in the window, and the coloured bodies that went grey (reported 2026-11-13: FIXED, both measured in the real window)

The report was "there are some artifacts floating around the bibite"; a picture followed (a bibite at 18.7 pixels per texel with a faint dotted rectangle round it). Two defects of the window's GPU drawing, neither of them in the headless render or in the game.

**1. The dotted lines.** The window drew single faint pixels along the rectangles of a bibite's sprites (body, eyes, mouth, arms, exoskeleton), a pixel or two outside the art. Cause: the viewer's `Conf` asks for `sample_count: 1`, and miniquad's WGL pixel-format chooser (`gl_choose_fbconfig`, GLFW's rule: a format without samples counts as "missing" when samples are asked for, then the smallest sample count wins) takes a 2x multisampled surface on the NVIDIA GTX 1660 SUPER. A pixel on the edge of a sprite's quad whose centre lies outside the quad but one of whose samples lies inside runs the fragment shader with the texture coordinate extrapolated past the quad (by up to the sample's distance from the pixel's centre: 0.35 px at 2x), and the part sheets are packed cell against cell with art on the cells' borders (`normal_body.png` is 11 x 27 = three 11 x 9 cells), so the pixel showed the neighbouring cell's texel at the samples' coverage: the fitted alpha of the window-only pixels is 0.50 for 69 of 89, and the texel is a value of the art's palette. The dash pattern is the coverage pattern along a rotated edge (5 px of every 21 on a 2.6 degree edge, as in the player's picture; the 0.73 strength there is 0.5 resolved in linear light). Fix: `render.rs` `inset_sprite_source` samples the cell inset by `SPRITE_INSET_PIXELS` = 0.75 px (at most 0.45 texel) on every side; the quad is not changed. An extrapolated coordinate then stays in the sprite's own cell, whose edge texel is what the software rasteriser shows there. 0.75 is above half a pixel's diagonal (0.71), so it covers any sample pattern; `RUSTIBITES_SPRITE_INSET_PIXELS=<px>` overrides it (0 = off; 0.4 is enough on this 2x surface).

Measured in the real window against the headless render of the same view (`tools/window_hairline_probe.ps1`, the default world's bibite 0v0; `RUSTIBITES_SPRITE_INSET_PIXELS=0` is the old behaviour in the same exe). Window-only specks (of them in empty space = the dotted lines), before -> after: 17.5 px per world unit 89 (48) -> 25 (0); the same with a forced 20 degree tail 105 (47) -> 34 (0); the default zoom of 4.1 px per unit 33 (11) -> 19 (0), with a tail 31 (11) -> 22 (0); 1.8 px per unit 26 (16) -> 25 (14), where what is left are one-texel arms that the supersampled software render draws softer, not edges. The pixels with the half-strength signature went from 69 / 67 / 12 / 12 to 2 / 1 / 0 / 0. The price: the texel boundaries inside a sprite move by less than the inset (the sampled rectangle is smaller by a fraction of a percent), a one-pixel sliver at some boundaries near the ends of a sprite, invisible side by side; the window's mean |dRGB| against the exact-mapping software render in the 700 x 520 crop goes 1.32 -> 1.68 at 17.5 px per unit (1.45 at 0.4 px, 1.49 at 0.5 px) and 1.41 -> 1.55 at 4.1.

Not done: (a) `glDisable(GL_MULTISAMPLE)` around the runs of sprites (the display list has two or three of them; exact pixel-centre rasterisation like the game's and nothing moves inside a sprite): it needs `unsafe` and the viewer crate says `#![forbid(unsafe_code)]`, so it is the owner's call (a few lines in a small crate, like `prefetch`); (b) `sample_count: 0` would change the anti-aliasing of every shape; (c) sheets padded with a transparent border in the GPU copy would be exact but need every cell known at load.

**2. Coloured bodies that went grey.** `wants_hue_material` bound the colour material for every `hue_shift` sprite without a program effect, while `draw_scene` drew a bent body seen big (a warp angle over 0.01 and more than 24 px across) from a picture the software rasteriser makes, with `draw_texture(.., WHITE)`: through the colour material a white vertex colour is a saturation of 0, so the finished, coloured picture went grey while the arms, mouth and eyes (their own tint) kept their colour, and the material's hurt / reaper values left by the sprite before tinted the picture. The neutral default bibite does not show it. Measured with the default bibite recoloured (`tools/recolour_save.py`, genes 0.95 / 0.2 / 0.05) and a forced 20 degree tail (`RUSTIBITES_FORCE_TAIL=20`): the window's mean |dRGB| against the software render was 11.81 at 17.5 px per unit (1.70 without the tail) and 2.07 at 4.1, with 21,156 saturated pixels fewer than the software render; after the fix 1.54 and 1.53 (225 fewer). One function, `painting_of` (Picture / HueMaterial / Default), now decides both the material binding and the sprite itself (unit test `a_sprite_drawn_from_a_picture_does_not_get_the_colour_material`).

Method and traps. `tools/window_hairline_probe.ps1` (idle-gated captures of the real window with `RUSTIBITES_SELF_SHOT`, the software twins, `-Variants` for the inset size, `-Only`, `-ReuseSoftware`), `tools/window_specks.py` (window-only specks sorted into empty space / silhouette edge / art, with the alpha fit; exit 1 when one is in empty space: the raw window-only count cannot reach 0, because the software render is supersampled), `tools/window_pair_diff.py` (mean |dRGB| and saturated pixels), `tools/recolour_save.py`. The viewer exe is a GUI-subsystem binary: run its headless renders from Bash or `Start-Process -Wait` (PowerShell `&` prints nothing and writes no picture); a negative `--center` needs a space (`--center -5.4,831.0`); a PowerShell string like `"$name: x"` reads `$name:` as a drive (`${name}:`); a rebuild replaces the light build's exe while a probe may be running: run the probe on a copy.

## Glitch save of 2026-11-13: a long sim grew 264,000 bibites with NaN positions that no ecology could feed (FIXED: four causes, the first one a grab joint)

The report: "I ran a longterm sim and found a glitch: it is producing WAY more bibites than the sim can ecologically support, and when I go to see them they are in the void(?), dying, with NaN distance travelled" (the save `glitch claude`, 695 MB: Butterfly 6 at 69,179 s, 264,302 bibites and eggs, 5,928 pellets; 257,441 of the 259,326 saved bibites had a position of `null`, i.e. NaN, and 232,118 had an energy of exactly 0).

**What the autosaves of the run say** (`Savefiles/Autosaves`, one every ten minutes of wall-clock; the viewer had been started at 01:46 on Butterfly 6): population 1,414 (01:56) -> 5,017 (03:36), then a crash to 2,514 (03:56, pellets 20,845 -> 6,948), 3,110 (04:06), 59,430 (04:16), 215,722 (04:26), 264,302 (04:33); the growth after 04:06 is exponential with a doubling time of 350 s of game time. The first bibite with a NaN pose is in the 02:16 autosave (one, a dead body whose size was NaN), a handful of them live from then on (full NaN: position, velocity, rotation, three brain inputs), and from 03:56 there are NaN-position bibites with a FINITE velocity and rotation (their children): 12 at 03:56, 849 at 04:06 (of 3,078).

**The chain, each link reproduced** (`crates/bibites-cli/examples/nan_trace.rs` and `nan_hunt.rs` run a save tick by tick and stop at the first impossible state; the 02:06 autosave reproduces a NaN at tick 45,741, sim time 21,317 s, in the port itself):
1. **A grab joint (a weld) joined a holder to the wrong bibite.** A bibite that grabs another keeps the prey's physics TAG (kind, generation, slot) in `held_targets`, and the weld was built from the tag's SLOT only (`dense_pos[slot]`): when the prey was gone (eaten, killed) and a newcomer had taken its slot in the same tick (a hatchling takes the slot just freed, last in, first out), the holder was welded to the newcomer, with the frame the prey had, wherever in the world the newcomer was. The rigid weld (`solve_weld_position`) takes the whole error out in one step: welds with errors of 4,470, 14,283, 21,000 units were logged (13 in 36,000 ticks of a 1,850-bibite world): a random bibite was pulled across the world, and what the solver left in the velocities grew 1 -> 212 -> 3,884 u/s in two ticks (angular speed -128, then -435 rad/s, past the 125 rad/s the step is meant to clamp to), until a body sat 1.2e8 units from the origin with a rotation of -4.4e8 rad and an angular speed of -26,557 rad/s. A dead holder (a corpse) kept its holds for good (the game releases everything when a bibite dies: `BibiteMouth.OnDeathStatusChange(false)` -> `ReleaseAndMaybeThrowAllHeldObjects(false)`), which made the dangling holds last.
2. **A pose that is NaN stays NaN.** At that distance the arithmetic breaks down and position, velocity, rotation turn NaN; nothing in the step sanitised them (`phys::step` of a body with a NaN position leaves it NaN, with the velocity finite: the signature of the children). Such a body touches nothing and nothing touches it (every NaN comparison is false), so it can neither eat nor be eaten nor hurt; the eggs it lays are born at `pos - up * 8 * d1` = NaN, and its brain's inputs `Speed`, `RotationSpeed`, `Phero*Angle/Heading` are NaN.
3. **A NaN brain output + the order of a turn = bibites that cannot starve.** In the founders a synapse `Phero2Heading -> Want2Heal` made the output NaN; a child whose mutation disabled that synapse keeps the NaN for good (a node with no enabled synapse into it is not recomputed: `plan_groups` only holds nodes that have incoming enabled synapses). `BodyState::heal` computes `hp = (max * rate * want * dt).min(max - health)`: `min` of a NaN is the other number (so in the game's `Mathf.Min`), `hp` = the whole missing health. The healing runs LAST in a turn and does `health += hp` after its own `use_energy`, which at zero energy hurts the bibite by more than `hp`, so the health ends near `max - dealt` and the end-of-turn check (`health < 0`) never saw the lethal spends of the turn. In the game `Hurting` calls `Die()` on the spot: the death stands. A bibite with 0 energy and egg production on lived on, and laid eggs (one per ~20 s of game time, 141 energy each), on health alone: the exponential growth.
4. **The port wrote what it could not read back.** serde writes NaN as `null`; the loader's `[f32; 2]` position took no `null` ("invalid type: null, expected f32"), so the glitch save did not open again.

**Fixes** (core `body.rs` / `physics.rs`; `bibites-io`):
- The weld is built only from a tag whose generation is the slot's (`BibiteStore::dense_position_of`, both places: the live holds and the grabs let go of this tick). Test `a_hold_on_a_bibite_that_is_gone_does_not_weld_the_newcomer_in_its_slot` (serial and 4 threads).
- A bibite that dies lets go of everything it holds, no throw (`BibiteRecord::let_go_on_death`, in both death paths; a swallowed or burst holder's pellets stop being flagged as held). Tests `a_bibite_that_dies_lets_go_of_what_it_holds`, `a_bibite_that_is_swallowed_lets_go_of_its_pellets`.
- A weld whose two anchors are more than 200 units apart is let go of instead of solved (`WELD_MAX_ERROR`, in `run_island`; the position solver takes the error out at every step, so a real weld never has one): defence in depth against any other way a weld can get a far partner (a holder wrapped across the world). Test `a_weld_between_bodies_far_apart_is_let_go_of_and_pulls_nobody`.
- A death in a spend stands: `pass_b_live` also asks `death_hurts > 0` (the count of lethal `Hurting` calls of the turn), not only `health < 0` after the whole turn. For finite numbers nothing changes (a normal heal is a few thousandths of the health bar; the pinned 12,000-tick evolution fingerprint is the same before and after); with a NaN Want2Heal the bibite without energy now dies like the others. Test `a_bibite_out_of_energy_dies_whatever_its_want2heal_output_is` (Want2Heal 0, 0.5 and NaN).
- A pose that is not a number is never written back: the bibite keeps its last pose, stands still and dies (`DeathCause::Hazard`, in dense order, in the step); pellets and eggs the step leaves as NaN stay where they were (`write_back_bibite_pose` returns whether the pose was lost). Tests `a_bibite_the_step_leaves_with_a_nan_pose_dies_and_keeps_its_last_position` (serial, 4 threads), `a_pellet_and_an_egg_the_step_leaves_with_nan_stay_where_they_were`.
- A body whose size, health bar or energy is not a number leaves no corpse and no meat pile when it dies (`report_for_numbers`): its mass would be the least a body can have (`max` ignores NaN) and a pile of NaN energy is NaN for good. A `GainEnergy` of NaN is no gain (it filled the bar: `available > NaN` is false, in the game too). Tests `a_bibite_with_a_nan_size_is_destroyed_when_it_dies`, `gain_energy_semantics`.
- Loading: a position written as `null` reads as NaN, and a bibite or egg whose position, size, health or energy is not a number is left out of the loaded world (`bibites-io` prints how many); the glitch save now opens (the 257,000 are left out).

**Checked.** A mutation check puts each fix back to the old code, one at a time, and the test that pins it fails (the lethal spend that stands, the generation check together with the weld guard, the guard, the release on death, the NaN pose, the NaN size, the NaN gain; the generation check alone leaves its test passing, because the guard also drops the stale weld: two layers of the same defence). Replays with `nan_trace` (stops at a speed above 2,000 u/s, a spin above 5,000 rad/s, a distance above 100,000 units or any NaN): the 02:06 autosave for 200,000 ticks (game time 18,268 -> 31,468 s; without the fixes the first NaN came at 21,317 s) and the 03:36 autosave, 5,000 bibites, for 30,000 ticks: nothing out of the ordinary in either, and the population stayed where it was (1,801 -> 2,636; 5,017 -> 5,052). The real `glitch claude` save opens in the CLI in 136 s with 1,885 bibites (257,441 bibites and 4,947 eggs left out) and runs. The core, io, CLI and viewer unit tests pass (core 624, viewer 732 quick), and the pinned 12,000-tick evolution fingerprint is the same with and without the fixes.

**Left open.** (a) The 695 MB save itself is not worth loading (the NaN bibites are left out, the 1,885 others and the pellets remain); (b) the physics of a very deep overlap (a contact pushes with no speed limit, `contact_push_speed`) was not changed; (c) a NaN brain input from some other source would still give NaN outputs (only the poses and the lethal spends were closed); (d) the pinned evolution fingerprint (`species::tests::evolution_regression_fingerprint`, ignored) had not been run since the physics changes of 2026-11-11 and was stale at HEAD: pinned again.

## Batch of 2026-11-11 (late): animators with a jump, lineage frame times, the disk report

- **An animator may have two points at one time** (a tester's "Constant Flux Bb Experiment" would not open: "points must be strictly increasing in x (point 2)"). That is how the game makes an instant jump: an end value at one y, then a point at the same x and another y; the points are an ordered list, not a function. `SettingsChanger.FactorAtTime` takes the first segment whose end is strictly after the time, so at the shared time the curve is already at the later point's value and the zero-width segment is never evaluated (the port's `spline_factor` always did this; only `SettingsChanger::validate` refused the file, and the editor's `move_point` / `add_point` kept points `MIN_GAP` apart). `validate` now refuses only a point BEFORE its predecessor; the editor follows `EditableLineGraph.AddPoint` / `MovePoint` (a new point goes after the points that have its time; a dragged point hops only over strictly later or earlier points, so it stays on the side it came from). Tests `a_curve_with_two_points_at_one_time_jumps_there_as_the_game_does`, `an_animator_with_a_jump_is_valid_and_repeats`, `points_move_get_added_and_removed_in_order`.
- **Lineage frame times** (`species_panel::scroll_probe::lineage_scroll_probe`, ignored; `RUSTIBITES_PROBE_SAVE=<zip>` on a 59-species save, light build): scrolling costs about 0.2 ms a frame. Zooming cost 35 to 60 ms in the tree (every node asked for its picture, also the ones outside the view, and made them one by one: now only a visible box asks, and the visible ones are made on all cores, `PreviewCache::warm`) and about 25 ms in the mesh (CPU tiles: now painted on all cores, `warm_tiles`, and a row's clear middle is written as one colour instead of blended per pixel: about 10 ms, pictures identical to before). The shortcut exe is the light build (opt-level 1); an optimised build is several times faster still.
- **"Hammers the SSD" while a sim runs (a tester's Task Manager picture: disk 100 %, reads 78 MB/s, writes 0.5 MB/s): not reproduced.** Measured: the window on a save and on a scenario file for 30 s each (no reads or writes at all after loading, 0.3 GB), the simulation of 20 000 bibites (0.14 GB), a New Game world "Enormous" + "Packed" (100 000 pellets, 0.1 GB, 75 ticks/s), the autosave (every 10 minutes of wall-clock by default as in the game, on its own thread, a few MB). Nothing in the sim or the frame loop reads or writes files; the menu's video decoder is killed when the world starts. Reads at that rate with almost no writes are what paging looks like, or something scanning files (an antivirus re-reading each new autosave zip); the tester's Resource Monitor (Disk tab) names the process and the files.

## Tester report of 2026-11-13: the speed depends on the camera and on the window's size, and drops as a sim ages (FIXED, measured in the real window)

The reports: "the sim speed is 1.5x in the middle of my sim, but 5x when I move over to the void"; "the performance drops over time: a sim at around 1x, resetting the game brings it back to 3x"; and, from the testing that followed, "the speed is heavily dependent on the screen resolution: the smaller the screen, the faster it runs and the more CPU it uses".

**Measured** (`RUSTIBITES_LOG_FRAMES=1`, `--speed 30`, Butterfly 6 after 100,000 ticks: 2,450 bibites, 10,100 pellets, 12 threads, a GTX-class GPU, the minimum FPS 15; the numbers are the means of 15-20 one-second lines, ticks per second counted by the window):

| | in the middle (display list of 10,800 primitives) | over the void | middle / void |
| --- | --- | --- | --- |
| the code of the 2026-11-12 alpha, 1920 x 1080 | 91 | 178 | 0.51 |
| the same, 960 x 540 | 130 | 161 | 0.81 |
| now, 1920 x 1080 (alternating A/B, 3 rounds) | 163 | 169 | 0.96 |

**Causes**
1. One thread did input, the ticks, the display list, the UI, the drawing and the presenting, in that order, and the minimum FPS gives a frame `1 / 15` s: everything after the world had been read came out of the ticks' share. In the middle at 1920 x 1080 a frame was ticks 34 ms (5.6 ticks), display list 6.3, drawing 14.9, presenting 5.3; over the void ticks filled the frame (10-12 ticks).
2. The window's size entered through the drawing: every zone's fill (`Primitive::ZoneDisc`) was worked out by the SOFTWARE rasteriser, on a picture a quarter the size of the window, uploaded and stretched, in every frame: 5 ms at 960 x 540 and 15 ms at 1920 x 1080 for three zones in view, so the CPU's work grew with the window's pixels (a hard zone edge also came out blurred).
3. The camera entered through the display list: about 6 microseconds per visible bibite in every frame, nearly all of it hidden constants in `sprites::bibite_sprites`: three `std::env::var` lookups (the debug switches `RUSTIBITES_FORCE_SKIN`, `_NO_WARP`, `_FORCE_TAIL`: a system call, a lock and allocations each) and, for every eye and arm, `alpha_fallback_sprite`, which allocated two vectors, sorted the sheet's cells and counted the opaque pixels of the art.

**Fixes** (each checked on its own)
- `tick_overlap.rs` + `App::dispatch_overlapped_ticks` / `join_overlapped_ticks` (window only; `--no-overlap` or `RUSTIBITES_OVERLAP=0` turns it off): the frame's owed ticks run on a helper thread from the moment the display list and the UI are built (the last readers of the world) until the frame has been presented; the world moves there (`mem::replace` with a kept husk) and comes back right after `next_frame`, and every method that reads the world joins first, so a world on the helper can never be read. The ticks are the world's own, one after the other (test `ticks_run_beside_the_drawing_are_the_ticks_the_loop_would_have_run`: same fingerprint, tick counts, accumulator and time panel as the loop); the picture shows the world as it was before the frame's ticks (one frame of ticks behind). The room the minimum FPS gives the ticks is `1/min_fps` minus what the frame did BEFORE them (input, display list, UI); the drawing and the wait for the display run beside them. Worlds whose frame ticks take under 0.5 ms (a thread's wake-up costs as much) keep running them in the frame. The first alternating A/B in the real window: +5.4 % in the middle, +0.8 % over the void, with the cheap drawing of the two fixes below; with the old drawing it gave +56 % at 1920 x 1080 (91 -> 142) and +15 % at 960 x 540. `RUSTIBITES_LOG_FRAMES=1` prints the new `join` column (the time the window waited for ticks that took longer than the drawing).
- `GpuZoneMaterial` (`render.rs`): the zone fill is one quad per zone with the shader's maths (`zone_alpha`) at the window's own resolution; the software picture stays as the fallback when the material does not compile. Checked in the real window (framebuffer shot of a zones-only world against the headless render): the zone interiors, overlaps and the outside are within one level of the software picture. Drawing in the middle at 1920 x 1080: 14.9 -> 1.5 ms.
- `sprites::debug_switches` (the three switches read once) and `GameAssets::fallback_cache` (the cell a part resolves to is worked out once per sheet, formula cell and preferred cell; test `the_remembered_cell_of_a_part_is_the_one_the_full_search_picks` compares it with the full search for every cell of every sheet): the display list of a view with 5,300 primitives 5.2 -> 0.9 ms, of 10,800 primitives 10.3 -> 1.4 ms.
- `--speed X` starts the window at a time speed (for measurements).

**The sim's age.** Nothing in the simulation was found that costs more with the sim's age at equal population: Butterfly 6 from tick 20,000 to tick 140,000 (2.6 simulated hours at 15 TPS, run in six segments that each load the last one's save) cost 1.8-2.0 microseconds per bibite per tick throughout (1,860 -> 2,640 bibites, 3.6 -> 4.8 ms a tick; the saved `data.bin` stayed in the tens of kilobytes). What grows with the age is the population and with it what the camera sees: the display list and the zone / sprite drawing were SERIAL with the ticks, so a growing world slowed the speed twice, and a reset brought it back. The tester's own world was not available: if a world is found where the cost per bibite grows with time, `rustibites run <save> --ticks N --save <next>` in segments (see the age runs of this report: ms per tick and `info` per segment) shows it without the window.

**Blood (and pheromone) particles stayed on the screen for ever at a high speed (FIXED; the tester: "blood particles are not disappearing", right after the speed fix).** `blood_particles::advance` and `phero_particles::advance` aged the particles by the game time since the last call, but skipped a step over a second ("a long step is not played back"). A frame steps up to 32 ticks (`MAX_CATCHUP_TICKS`), 1.07 s at 30 ticks a second, so from a time speed about equal to the frame rate up nothing aged and every burst's blood stayed (up to the 20,000 cap); the speed fix made such frames common, the rule was older. A step as long as a blood particle's life (0.5 s) now clears the blood, and a pheromone particle ages by the whole step (Unity's particle system ages by the frame's scaled time as well). Tests `blood_is_gone_when_every_frame_steps_more_than_a_second_of_game_time` (without the tick helper and with it) and `pheromone_particles_age_through_a_step_of_many_seconds`; both fail on the old code.

What is left: the display list and the UI (about 2 ms) are still serial with the ticks; with the ticks saturating the minimum FPS's frame the window runs at 15 frames a second (the game's own governor does the same); a bibite with a bent tail bigger than 24 pixels, or a rad / rain / producer skin, is still drawn from a software picture of its own box (not measured here: the world of the measurements is sparse at a close zoom); `draw_background_cells` (the experimental background) and `draw_microscope_shade` still use the quarter-size software picture.

## Tester report of 2026-11-12: thirteen items

1. / 3. **Pellet sizes over 50x came out as 50x, also in saves the port wrote (FIXED).** `pelletSize` has a 0.01..50 slider but, like the zone's fertility, biomass density and speed factor, leaves `NumericSetting.canGoOutOfBounds` at its default `true`: `SetValue` clamps only the settings that say `canGoOutOfBounds = false` (a zone's position, radius, inside radius, width and height), so a vanilla save with pellets of 100x keeps them. The port's `ZoneSettings::clamp_to_ranges` (every load) cut all of them to the slider's ends. It now keeps a ceiling off those four (a floor at the slider's minimum stays: a zero pellet size would spawn a million pellets), and the typed-value rules of the Zones tab follow the game's flag per field (`typed_rules`). A save the old build wrote has the clamped value in it. The audit of every scenario / independent / bibite setting's `clamped` flag against the C# found no other mismatch (92 settings). Tests `zone_settings_clamp_and_validate`, `a_zones_pellet_size_typed_past_the_slider_is_kept_and_a_position_is_not`. Not understood: "an immediate drop in population even when setting the pellets back to 100x before resuming": the loaded value was 50x, and a zone with half-size pellets at the same biomass spawns twice the pellets; the drop may be that.
2. **Bibites vibrate when the camera is locked on one (FIXED; the tester: "vibration is when you lock onto a bibite").** `CameraManager.LateUpdate` puts the camera on the followed target after the frame's `FixedUpdate`s (the ticks); `App::advance` followed it BEFORE the frame's ticks, and the picture is built from the world after them: on every frame that ran a tick the locked bibite was drawn one tick's movement off the centre, on the frames without a tick exactly on it, a flicker at the display's rate (a few pixels at a high zoom). The follow is now `App::follow_target_camera`, run after the ticks and the target's life cycle (and while paused, for a body that is held or moved). Test `the_camera_locked_on_a_bibite_ends_every_frame_where_the_bibite_is_after_the_ticks`. The first look found nothing because the simulation's own jitter IS the game's (rotation second difference median 0.044 / 0.043 deg, p99 1.08 / 1.04; angular velocity p99 21.5 / 20.8, 1,200 bibites, 80 ticks): the lag was in the camera only. What stays as in the game: nothing is interpolated between ticks (the prefab's Rigidbody2D interpolation is `None`), so the world around a locked bibite moves in steps of one tick.
4. **Typesetting blurry (probable cause FIXED, unconfirmed).** The window's text is rasterised per (text, size) into textures drawn in LOGICAL pixels. On a display scaled by the system (125 % / 150 %, a 2560 x 1440 monitor typically) the window is rendered at the logical size and stretched, so every glyph was stretched by 1.25 or 1.5 with nearest sampling. `TextTextures::draw` now rasterises at the physical pixel size and draws on whole physical pixels. The Zones tab's layout itself matches the capture (headless render at 1067 x 627). Not placed "the greatest": nothing specific found.
5. **Typing a speed (FIXED).** The time slider's number is its edit button (`SettingSlider.OpenEditPanel`): a click turns the slider into an input field (the same field and rules as the settings panel's rows, `FieldRef::TimeSpeed`), Enter applies (a `LogSettingSlider`: kept past 100, a positive speed plays, a zero pauses), Escape or a press elsewhere closes it. Test `a_speed_can_be_typed_into_the_time_slider`.
6. **No bibites from a spawner (FIXED for the cases found).** (a) The game resumes the default-bibite spawner after loading a save (`SaveSystem.LoadWorld` -> `SimulationManager.ResumeSpawners` -> `BibiteSpawner.StartSpawner`); the port did not (only fresh scenarios): `World::resume_spawner_after_load`, called by the viewer's three loads (templates read from the player's folders) and by `rustibites run` on a save. (b) `set_bibite_spawns` (the Default Bibites panel's add / delete) reset the progress and the species of every row; it now keeps the rows that stay (`SettingsAdded` / `SettingsRemoved` touch only the one that came or went) and gives a new row `BibiteSpawnInfo`'s start (a one-time row starts due, its species is the one its template names, founded when there is none). Tests `a_spawner_row_added_to_a_running_world_spawns`, `a_loaded_world_resumes_its_spawner`. The tester's own scenario was not available.
7. **A placed Basic bibite got a new species name (FIXED).** The game gives every template a species name (`BibiteTemplate`: a `.bb8template`'s `name` in species format, so "Basic bibite"; `Species(BibiteTemplate)` takes its generic and specific name from it) and `StartGenesFromTemplate` -> `FindSpeciesFromTemplate` joins the species of that name, the first bibite founding it. The placer (`spawn_template_at`) and the spawner (`spawn_bibite_from_template`) used a genetic-distance match when the template carried no species name, founding a species with a random name; both now use the template's name (`TemplateSource::game_species_name`, `LoadedGenome::game_species_name`); eggs placed from a template too.
8. **Void-No-Mo' slower to trigger (FIXED, to the C#).** `BibitePropulsion.cs:272-292` picks ONE zone, the one whose edge is nearest (a zone replaces the chosen one when `distance - radius < |vector to the chosen edge|`, the vector starting at infinity, so the last zone a bibite is inside wins), and steers for ITS centre only when the bibite is outside that zone, with weight `min((distance - radius) / voidAvoidanceDistance, 1)`; the port took, of all the zones the bibite was outside of, the one it was farthest outside of, and treated rectangles as rectangles (the game's `Zone.radius` is set for circles only: a rectangle is a point for this). Test `void_avoidance_follows_the_nearest_zone_only`.
9. / 10. **Zones reach beyond the green (no cause found).** Radius (`relative x SimulationSize`), the spawn distributions (`GetSpawnLocation`, `InsideUnitCircleWarped`, rings, rectangles) and the zone shader (alpha `opacity x (0.5 + c x (0.5 - d^2))`, which is 0 at the rim of a centric zone and 0.19 of the opacity at 90 % of it: the green fades long before the radius, as in the game) all match the C#. Needs the scenario.
11. **A black bibite / the portrait's outline.** Bibite colours were compared with the game's own picture: twelve pigments (basic, black (1, 1, 1), near black, white, the primaries, grey, purple, dark red, pale) as a grid, photographed through the bridge's `/screenshot` and rendered with `--screenshot` at the same camera (`tools/parity/bd_contacts/piles/make_colors.py`, `game_shot.py`): the bodies are equal (black (14, 14, 14) in both, grey (100) vs (101), mean difference 0.63 over the grid, all of it on the edges). So a black bibite is that dark in the game too. The "portrait outline over the pellet" was not found; needs a picture.
12. **Gene values not capped (FIXED).** The game's `CapGenes()` runs when a bibite is made from a template (`StartGenesFromTemplate`), when genes are copied (`CopyGene`) and when a saved bibite is loaded (`BibiteGenes.LoadState`); the port capped only on mutation. A template with an average brain mutation of 16 now spawns with 10 (`genome.cap()` in the template spawns, eggs included, and in `World::from_contents`).
13. **The bar at maturity 1.0 (FIXED).** The Biology panel's growth and age bars each carry a `ProgressSliderHandle.targetIndicator` (a 4 x 8 (38, 38, 38) rectangle: growth at 1.0, age at the ageing threshold). The port positioned it (`biology_view::bar_edit`) but its image has no sprite and sprite-less images were drawn only in the generated dynamic-panel sessions: the marker is now drawn.

## Contact response, grabs, slow motion, the "deliberate differences" (2026-11-11, late)

Method: fixtures with ONE bibite (bias-only brain) driving into pellets, stepped tick by tick in the hidden game and in the port (`tools/parity/bd_contacts/piles/`, its README).

- **Grabs were broken at the default one step per tick (FIXED, `4edb15f`).** The plain one-step solver (`solve_soft_step`) knew no joints: a bibite that "held" a pellet or prey had the weld recorded but not solved, so the held body stayed where it was for 3-6 ticks (until a contact moved it) and sat 0.9-3 u closer to the holder than in the game. The grab tests had all been written for ten steps. Now a tick with a weld or a mouth that wants to grab goes through the island solver with ONE step (`fine_steps` with the Box2D 2.4 rigid solver, a rigid weld position correction `solve_weld_position`), and the weld forms at the start of the step in which the trigger overlap shows (as the game does: 60 / 200 / 400-unit pellets from 18 / 24 u, pellet and bibite within 0.1-0.3 u of the game's over 90 ticks; one fixture 0.7 u). Test `at_one_step_per_tick_a_grabbed_pellet_travels_with_its_holder`. Not checked: a bibite grabbing a bibite.
- **Circle-circle contacts have no margin (FIXED, `763ba24`).** Pellets 0.0005 apart do not push each other, 0.0005 into each other they do (piles spaced 4.998 ... 5.002); the port used 0.01 for every pair. Now zero for two circles (pellets, eggs); the bibite's capsule keeps 0.01 (not measured).
- **Plant pellet friction 0.7, not the asset's 0.4 (calibrated, `763ba24`).** A pellet passed at the side of a bibite is carried forward 7.4 u in the game against 6.3 in the port (a meat pellet, friction 0.02, matches within 3 %); piles of plant pellets pushed by a bibite end ~1 u behind the game's at 76 ticks. 0.7 cuts the glancing-hit error from 6.6 to 2.6 u and the jittered-pile score from 16.7 to 12.2; why the asset's value is not what runs is not known (the bibite capsule's 0.15 was tried: it also lifts meat, which then misses).
- **Still open: the contact response of the save.** `an_contact6`-style aggregate (80 game ticks of Branching Desert, per-contact velocity change): unchanged by every material knob tried (friction, bounce, iteration count, slop, Baumgarte, contact order), so it is not material. The pile fixtures say the port's bibite slows less than the game's in the 5 ticks after first contact (the game's response comes in waves; the port's contact is continuous): a pulse structure not explained. Trajectory error against the game on the save improved (tick 40 position p90 0.65 -> 0.57, tick 77 velocity p90 6.31 -> 5.83).
- **Slow motion.** The game's physics step is `fixedDeltaTime = base x timeScale`, i.e. `1/speed` steps per tick. Against the game with the jittered piles: 0.5x fits `RUSTIBITES_PHYS=2,2` (score 9.9 against 48.5 for one step) and 0.25x fits 4,4 (15.5 against 70.3); 0.1x best fits 3,1 (14.2 against 63.9); 0.75x, 0.33x and 0.2x fit NO step count (the game's step count per tick is then irregular: 1/speed is not a whole number, or float phase). The viewer still runs one step per tick at every speed (the world has no notion of speed); mapping `round(1/speed)` steps is supported only at 0.5x and 0.25x, so it was not made general.
- **Starvation / organ deaths, "one tick later" (the old note had the direction wrong).** One starving bibite (energy 0.3, health 6, no fat), game against port: the game removes it at row 79, the port at row 81 (one tick after the shifted game), and the energy of the first ticks differs by ~0.03 (the game spends 0.043 in tick 1 then 0.0227 per tick; the port 0.0169 for three ticks): a fixture-start artefact, not changed.
- **Pherosense and colour killers already use colliders** (`touches_circle`, the spots' own trigger radii): the old note was stale.
- **Held-pellet drag toggle:** applied (grab weld zeroes the damping; rebuilt pellet bodies read `is_held`); the game reads its static flag only at a pellet's next drag update, the port reads the setting live; no effect worth a model.
- **Bite-killed victims' own `CorpseUpdate`:** with corpses off the victim's body lives to the end of the frame, so its own update after the killer's would burst it again (as the starving ones do); whether it runs after the killer's is the game's update order, not known; the port's meat is already 5-10 % above the game's, so extra bursts were not added.
- **Brain-mutation counter quirks: not found.** Not re-checked.
- **Worlds compared still: Branching Desert 2 and `lol.zip` plus fixtures;** none with towers, killers, red death.

## Title menu: any UI scale but normal, or a window that is not 16:9, put the whole menu off the screen (FIXED)

A tester: "having the UI scale at anything except normal makes the main menu unresponsive; also when scaling or maximising the window after opening; only the main menu". Reproduced offline: the first probe (below) compared hit rectangles with hit rectangles, which agree with each other, so it missed this; `title_hit_probe::the_title_buttons_hit_where_they_are_drawn_at_any_window_size` now looks for a drawn button image under every click rectangle and for the rectangles on the screen, at UI scales 0.5 to 1.5 and seven window sizes. Cause: the title (and the settings screen's) canvas root is stored by the capture with its centre at (960, 540); `compose_root` recognised it as "centred" only when that equalled the CURRENT canvas's centre, i.e. only at a canvas of 1920 x 1080 (UI scale 1, window 16:9). In every other canvas the root was shifted by (960, 540) and the menu was drawn, and clicked, off the screen (at 1.5 only a corner icon was left). The centre is now compared with the capture's own as well. Second, smaller fix in the same place: the title click was tested in logical window pixels against buttons laid out for that size, while the picture is painted at the window's physical pixels (capped at 2560 x 1440) and stretched: with a fixed "Screen Resolution" on a scaled display (the pixels differ) the two layouts differed. The click is now mapped into the picture's own pixels (`menu::picture_point`, like every other menu screen). Tests `a_click_on_a_title_button_hits_it_whatever_the_ui_scale_and_the_window` (misses 18 of 45 button/size cases the old way), the probe above. The first tester's "fullscreen on 2560 x 1440 does not click" may be this (a UI scale other than 1 or a scaled display); `RUSTIBITES_INPUT_LOG=1` stays for it. (SUPERSEDED 2026-11-21: the other title screens follow the game's layout now, see the first section.) STILL OPEN then, the other title screens (New, Load, Settings, Library, Achievements, Tools, Credits; `menu::tests::every_menu_screen_at_other_ui_scales`, `RUSTIBITES_PROBE_OUT=<dir>` writes a picture of each): they keep their 1920 x 1080 layout, so at UI scale 1.25 (canvas 1536 x 864) their right and bottom edges are cut off and at 0.75 or on a wide window they sit in the top left corner. In the game their children are driven by layout groups (the capture stores them top-left anchored at the 1080p results) that re-solve for the canvas; the capture has no group parameters, so this needs a live capture of the game at another scale. Usable but wrong; the Settings screen's dropdown (where the scale is changed back) stays reachable. Not checked: how the lower buttons sit at 1.15 and 1.25 (the canvas is then 940 and 864 units tall; at 1.5 the panel is taller than the canvas, a setting the game does not offer).

## Tester report: title screen buttons do not click in fullscreen (2560 x 1440 monitor) - not reproduced, a click log added

Checked without a 1440p screen (this machine is 1920 x 1080 at 100 %): `ui_runtime::title_hit_probe` compares the rectangles `hit_row` tests clicks against with the buttons the title frame draws, at 1280 x 720, 1920 x 1080, 2560 x 1440, 2048 x 1152 and 1706 x 960 (the logical sizes of 125 % and 150 % scaling), 3840 x 2160 and 2560 x 1080: they coincide at every size. The window's mouse position and `screen_width()` are both logical (macroquad divides by the DPI scale; miniquad 0.4.11 `SetProcessDPIAware` with `high_dpi`), the menu picture is drawn at the window's physical pixels capped at 2560 x 1440 (`menu_pixels_for`), and rebuilding it costs 89 ms at 2560 x 1440 (light build), so neither the mapping nor the cost explains it on paper. `RUSTIBITES_INPUT_LOG=1` (set before starting the program, run from a terminal) prints a line for every press on the title screen: the mouse position, the screen and framebuffer sizes, the DPI scale, the menu picture's size, the button the click hit and the button rectangles. One run of that in the tester's fullscreen window says which of those is off.

## Batch of 2026-11-11 (lineage panel): input in the window, the mesh shader, the view's place

The user: the mesh view "is completely off and doesn't let you scroll or zoom", the tree view doesn't let you zoom. Compared with the hidden game (`tools/parity/lineage_compare/README.md`: captures of the tree and the mesh of `lol.zip`, and of the tree at zoom 1) and fixed:

- **The real window never gave the panel its wheel or buttons** (`main.rs::route_ui_frame`, `App::lineage_raw`). The panel is full screen (`HIT_BACKDROP` is a widget everywhere), so the loop's "over the UI" rule cleared the wheel and the primary button before `lineage_input` ran: no Ctrl+wheel zoom, no wheel scroll, no scroll-bar drag (the headless tests call `handle_input` directly and passed). The raw pointer state is now handed to the panel (test `the_window_loop_leaves_the_lineage_panel_its_wheel_and_its_scroll_bars`, which fails without it). Either Control key zooms, as the game tests `KeyCode` 305 and 306. The game's zoom is Ctrl + wheel (`LineageWindow.Update`); the plain wheel scrolls (Shift: sideways), the right button drags.
- **The mesh's pixel program is the game's** (`Lineage Mesh Shader`, sharedassets2 path 86, DXBC): `out = lerp(border, lerp(parent, own, g), 1 - x * t)`; `g` is 1 up to `1 - _BlendHeight` of the shape's height, 0 at `1 - _BlendHeight / 2` and above (the top of a child is its parent's colour); the border is a gradient from black at the LEFT edge to nothing 5 units inside, none on the right edge; the selected species has a yellow gradient 10 units wide on both edges (`_Focus`: `x` = 1, so it does not fade with `g`). The port drew a solid 5-unit black band inside both edges, which read as thick black seams between shapes. `lineage_mesh_view.rs::paint_shape_row`.
- **The view's place** (`View::reset_view`): `ResetView` is only called when switching tree <-> mesh and is `ZoomToScale(1)` from the old zoom about the middle of the viewport's height, so the mesh opens scrolled to wherever that point lands (not at the bottom); a rebuild for a changed option keeps the zoom and the place.
- **The mesh previewer does not turn the bibite with its diet** (`preview::MESH`, `rotate_with_diet` false): the game's pictures are axis-aligned. The species names are auto-sized (up to 30.7, TMP's 0.05 steps, trailing space not counted: 21.85 to 30.7 in the capture) and a name the viewport edge cuts is drawn cut (`UiDraw::Clip`), in the tree and the mesh.
- **Axis labels**: `LiberationSans SDF - Fallback` bold auto-sized to the holder's height (0.8941 of it: 15.2 at zoom 1, 16.95 in the 18.96 holder of the earlier capture; the port had a fixed 16.95 x zoom, 11 % too big at zoom 1) with the holder's 5 padding (the port 2). The highlight of the selected node and its links (`Unlit/highlightableGraph`: alpha grows by the four neighbours 2 sprite pixels away) is clipped to the sprite's own quad and has no diagonal copies (the yellow bled over the white trunk).
- **`GlobalLineageManager.maxEnergy` was 0 in every world the port saved** (`SpeciesRegistry::log_stats_with` summed `stats.energy`, which the world never keeps): a log now sums the samples' energy of the active species. The game's mesh divides by it, so a port-written save showed NO mesh in the game (every width NaN); saves written before this still have 0 until their next species log (5 simulated minutes). Test `a_log_raises_max_energy_to_the_most_the_active_species_held_together`.
- **Saves written before the maxEnergy fix are repaired on load** (`raise_max_energy_from_history`: the most all species held together at any logged point; a stored value above that, as in a game save, stays). Without it a port save has `maxEnergy` 0, every moment's total width clamps to the full strip, and only the shares between species move: the mesh's outer edges showed no jaggedness for biomass change (reported on the 2026-11-11 17:04 "rust" save; the probe is `species_log_probe` in `negative_health_probe.rs`).
- `--lineage-species` and its companions work in the real window too (`apply_lineage_flags`), for `RUSTIBITES_SELF_SHOT` captures.

Measured against the game (1920 x 1080, UI only, outside the species panel): mesh of `lol.zip` 37.2 -> 3.1 mean difference (what is left: the pictures' pixel rows, the hover tooltip the game's capture has, antialiasing of the shape edges), tree at zoom 1 1.74 -> 1.09 around the nodes, selected-node region 0.93; the real window's framebuffer (GPU path) matches the headless picture. Not done: the mesh's focus pan on selecting a shape (`SelectAndFocusSpecies` subtracts the mesh's absolute screen position from the holder, then the ScrollRect clamps), the shader's colour space, the tree at its fit zoom (lines differ by half a pixel; the labels there are 2.5 px and the port skips text under 3 px), the game's mesh of a save with `maxEnergy` 0.

## Batch of 2026-11-11 (text): synthetic bold everywhere, bold weight from the materials, TMP's outline

Method (offline, no game needed): render each captured menu session with `--ui-session NAME --ui-background black.png --width 1920 --height 1080` and compare it with `assets/game/reference/functional_ui_native/menu_ui/NAME_ui_on_black_1920x1080.png`: mean pixel difference per screen (39 screens, sum of the means is the number to watch) and, per single-line text block, the **ink** (sum of luminance), the ink-weighted centroid and the extent against the reference (`tools/parity/text_parity/`: `text_sweep.ps1`, `text_outliers.py`, `text_ink.py`). Dark-on-light panel text: the inspector's `native_inspector_*_ui_only` references composited over black (`over_black.py`, `dark_ink.py`).

- **Any TMP text in the Bold style is TMP's synthetic bold** (`ui_runtime.rs`): the regular atlas widened and 0.07 em more per advance, on every asset (the game's assets have no bold typeface). Only `LiberationSans SDF`, the species/tag nodes and some hand-built panels had it; a Boston Black label in the Bold style (the title screen's "The Bibites 0.6.3.1", 28) was 14 % too narrow and its words 17 px off; the other Bold nodes (Roboto-Regular, Roboto-Bold, the Fallback Liberation, ~15 000 captured nodes) were drawn with a Roboto Bold stand-in. Title 1.60 -> 1.37 mean difference, Load Game 3.19 -> 2.93, New Game 8.46 -> 8.02, the challenge screens -0.45, the four inspector tabs -0.03 to -0.06 each; no screen got worse. The title's big text extents are now exact (the old "2 % wider" note is gone).
- **The bold weight comes from the material** (`text_sdf.rs::bold_dilate_of`): all materials have `_WeightBold` 0.75, TMP moves the edge by `(0.75/4 + _FaceDilate) * _ScaleRatioA * 0.5` of the field (0.156 in this module's dilation unit for Boston Black / Roboto, gradient scale 6; a sweep against the title label gave 0.15 to 0.16). `LiberationSans SDF` (the Mobile shader) has the same formula (0.169) but its Biology labels reach the game's ink only at 0.21, kept as measured. The old fixed 0.25 was 5 to 10 % too heavy.
- **Outline materials use TMP's `GetColor`** (`outlined_pixel`): a ring `_OutlineWidth * _ScaleRatioA` of the field wide, centred on the glyph's edge (half outside), opacity scaled by `sqrt(min(1, width in ramp units))`, face colour inside, premultiplied mix cut by the face's silhouette; the glyph quad is widened by the atlas padding for these only. The speed label "X 0.00" (`Roboto-Bold SDF WhiteOutline`) was a dark face with a barely visible ring; it is now the game's white ring (Biology screen 1.233 -> 1.214). The hand-fitted `outline_width * 0.01` / `softness * 0.02` terms are gone.
- **Tried and dropped:** widening every glyph's quad by the padding (TMP does; the atlas rect is only the metrics box) measured slightly worse (sum 341.7 -> 342.9) with or without a half-texel sampling shift, so plain text stays inside the box.
- **Where the text stands:** over 1286 single-line blocks on the 39 menu screens the median ink ratio port / game is 1.02 (10th to 90th percentile 0.97 to 1.04), extents and baselines to within a pixel. The outliers left are not text rendering: the `--ui-session` harness has no menu state (the port draws its title text where the reference hides it, tab content that wraps differently), and the Credits tab's "Eric (eschurma)" row sits 3-4 px off (row heights with a smaller description). Not looked at: the `Roboto-Bold SDF BlackOutline` asset (no font JSON is loaded for it), `LiberationSans SDF - Outline` labels (drawn black as before), small-text weight on white-on-black Roboto (about 3 % light in the title's Patreon note).

## Batch of 2026-11-11 (meat): the dead body that bursts four times

The meat stock of Branching Desert fell 1.7x faster in the port than in the game. Not decay, not eating (a lone bibite eating / biting a pellet matches the game bite for bite: 380.02, 359.95, 346.96 u² at the same ticks), not the speed the game ran at. What was different was how much meat a **death** leaves.

Method: a world of the same bibites with their brains zeroed and 25 energy (`tools/parity/bd_contacts/make_starve.py`) starves without eating; the game made +24.6k u² of meat in 120 s, the port +5.9k. The saved worlds showed the new pellets in the game in **groups of four of exactly the same amount** (the port: one). One bibite with `energy -30`, health 1, stepped tick by tick in the game: four pellets of 77.88 u² appear in the very tick it dies.

Cause (from `BibiteBody.cs`): with `corpsesEnabled` off `Die()` calls `ExplodeToMeat()` at once, but the body is only destroyed at the end of the frame. `Hurting()` of a dead body below zero health calls `ExplodeToMeat()` again, and `UseEnergy` calls `Hurting` whenever the energy pool cannot cover the cost, even for a cost of 0. The dying call is the metabolism spend at the start of `FixedUpdate`; after it come the fat organ, the muscles, `Heal()` (always, the health is below max), the egg layer (when it makes eggs) and a pheromone puff (when one is made): each one bursts the body again, with the same energy and a new stomach spill. A starving bibite leaves 4 piles (5 when it makes eggs). A fixture with 30 fat leaves 1 (the fat refills the pool, the later spends are covered), in the game and in the port. Default worlds (corpses on) are not affected: `Die()` resets the health there.

Port: `BodyState::death_hurts` counts the hurts of a tick that leave the health below zero of a body not yet dead (reset at the start of the metabolism spend); `resolve_death` and `die_now` spawn `max(1, death_hurts)` piles and stomach spills. The egg layer now spends whenever production is on (`EggLayerTick::spends`, also at cost 0) and the pheromone organ only when a puff is made, as in the game. Not modelled: a victim whose own `FixedUpdate` comes after the bite that killed it (its `CorpseUpdate` spends from a pool that is not empty, so no extra burst in the cases looked at).

Checks: one-bibite fixtures of 5 sizes: game 4 x 45.11 / 77.88 / 176.19 / 307.28 / 106.18 u², port the same; the fat fixture 1 pellet in both. The starving world after 120 s: game 1339 meat pellets / 185.5k u², port 1342 / 185.8k, and the sizes of the groups of equal pellets are the same (1: 464 / 467, 2: 105 / 106, 3: 35 / 35, 4: 80 / 78, 5: 29 / 29). Branching Desert 300 s: meat 153.6k and 156.6k u² (two seeds) against the game's 148.8k and 138.2k (two runs; the game varies by +-5k itself), bibites 1216 / 1226 against 1228 / 1233. New test `a_starving_body_without_corpses_bursts_once_per_energy_spend_of_the_frame`. The pinned evolution fingerprint did not change (corpses are on there).

Tools: `meat_an.py` (meat of a save), `meat_new.py` / `meat_deaths.py` (which pellets are new), `make_starve.py`, `make_eat.py` (eating fixtures), `meat_game3.py` / `meat_game5.py` / `meat_game7.py` (the game's death fixtures, the last one tick by tick). The ripping in the game also has `desireToAttack > desireToSwallow`: unchanged.

## Audit of 2026-11-11 (re-audit of where the port differs from the game)

Done with the hidden game (bridge sampler + events) against the port's headless runs, same save, same sim time:
- **Branching Desert 2** (1202 bibites, 300 s): births 335 game / 330 port, starved 203 / 203, killed 82 / 98, other 24 / 20; population 1228 / 1211; energy 305.5 / 305.4; health 247.5 / 251.3; speed 33.2 / 33.2; bite rate 0.0038 / 0.0035; Accelerate 0.713 / 0.707; plants 1672 / 1671.
  **Different at first: the meat stock** (meat pellets 1040 / 896, meat biomass 1.38M / 1.16M); found and fixed the same day, see "Batch of 2026-11-11 (meat)" (the port's meat was 120k u², now 154-157k against the game's 138-149k).
- **lol.zip** (22 bibites, 300 s): births 79 / 85, starved 47 / 43, killed 10 / 14, other 10 / 11; plant stock equal; population 34 / 39 (small-sample noise).
- **Port-written saves** (lol, Branching Desert after 400 ticks) load in the game with the same bibite / corpse / egg counts and run.
- Controlled contact fixtures: see "Batch of 2026-11-11".
Not re-run in this audit (the previous evidence stands, dated in their sections): the visual / UI pixel diffs of 2026-11-10 (Graphs 0.50, pause menu 0.46, Gene Analysis 0.69, dynamic settings panel 4.04, Credits 2.49, inspector Stats 3.05 / Genes 1.82 / Brain 1.74, New Game 8.23) and the
mechanics lists in "UI mechanics"; the animators / zone-group editors of 2026-11-10 are done in the world and in the scenario editor. The "Open items" block below is the current list.

## Batch of 2026-11-10 (night): zone groups and settings animators editors

**Zone groups (done in the world and in the scenario editor's draft).** Core: the zone store keeps the game's order (explicit zones first, then each group's, ids renumbered as
`UpdateAllZonesList` does) and groups own their zones (`ZoneStore` / `splice_zones`); `GenerateZones`, `RenameZones`, `ApplyChangesToAllZones` (the template's `CopySettings`, skipping
name / id / position) and `AlignCountTypes` (density = count / (4 simSize^2 / 1e6)) are ported; deleting a group is immediate; the header's Remove All is a dangling call in the game
(a no-op here too). The Zones tab shows the group cards and the open zone card in the game's two-column layout, rebuilt from live captures (`zone_cards.rs`, held by the fixtures of
`zone_card_states.json`). Saves keep the groups with their zones (`bibites-io` round trip tests). The template card's fertility / biomass / size text keeps two decimals ("10.00x", as
the game's capture; `SliderInfo::format`, test `the_cards_slider_texts_follow_the_games`).

**Settings animators (done in the world's Animation tab and in the scenario editor's Animation list).** `animator_session.rs` is the shared editor (`Session` + `Env`: the creator, the
picked / dragged point, the graphs' frames, the dropdowns, sliders, graph press / drag, point removal) over a list of `SettingsChanger`; the world (`app/animator_actions.rs`) and the
scenario editor (`scenario_editor/anim.rs`) both call it. Cards: name, Start, Repeat, base items, the curve graph (drag a point, Ctrl-click empty ground adds, Ctrl-click a point removes;
the first point and the last two stay), the point panel; creator (Linear / Sine / Cosine); delete / remove-all confirmations; one animator per setting; an animated setting cannot be edited
by hand and its slider handle / toggle background take the ColorBlock's disabled tint. The base item is laid out from the live game (876 x 37: name, slider, value on the right,
35 x 35 cross, 2 px liner; "Add Setting:" line with a 150 + 460 wide dropdown; widths follow Unity's flexible layout when the card is wider than the world's 926).

**The scenario editor now has text fields.** Typing a name into a zone / zone group / animator card (Enter renames, Escape or a press elsewhere cancels) and typed slider values
(`DynEditValue`: settings, zone / template / group fields, materials, default bibites, animator fields) with the game's rules (`value_edit::Rules`); the Ctrl state and the cursor travel
to the graph through `MenuState::ctrl_down` + `EditorState.pointer`.

**The move arrows (`ReorderableItem`) work** for the zone, zone group and animator lists: the card swaps places with its neighbour in the shown order and the data keeps its order, as
in the game (`DynState.order` + `card_order`; the order follows deletes; first card cannot go up, last cannot go down).

Also fixed on the way: the world's animator card could not be opened or closed again from its bar (`ZoneOpen(3, _)` was ignored).

**The point panel (`PointEditor`) is laid out from the game's own dump.** It was thought uncapturable (the popup is a `FollowLikeTooltip`, inactive in the dump, so the screenshot shows
nothing), but the inactive hierarchy is in `/d/ui` as a root object: 280 wide, 15 padding, 4 between 25 high rows, `simpleTextLine` (name 95 + value 150), `simpleDropdown` ("Interp." 92 + a
158 wide dropdown), `simpleSlider` (name 90, a 100 wide slider panel, value 50) only while the interpolation is not linear (height 113 / 142), a 25 x 25 delete button 5 in from the corner. Its
place follows `FollowLikeTooltip.Update`: 10 from the point, left of it when the point is in the right half of the screen, below it when in the upper half.

**The scenario editor's text fields have carets.** Names take Left / Right / Home / End / Delete, Shift selection, Ctrl + A, Ctrl + V; value fields blink their caret (`EditorState::tick_field`).

A press inside an open text field of the scenario editor puts the caret under the pointer (Shift keeps the selection's anchor) and a drag selects: the build measures the open field's
characters with the real font (`EditorState::measure_field`, 26 pt names, 16 pt slider input fields inset 12, 18 pt point-panel lines) and the menu hands the click's position and the
control under it to the editor (`Pointer.rect`). The point panel's delete button is `base_icon_thin` (3 px border at 100 per unit, from the dump).

## Batch of 2026-11-10 (afternoon / evening): the in-game settings panel, the shader translators, the GPU oracle

**Rendering: two dropped minus signs bent every translated shader's outline (fixed).** `tools/unity/dxbc_to_rust.py` / `dxbc_to_rust_tex.py` read the operands of
`movc` / `and` / `xor` as raw bits and ignored their negate / abs modifiers, so `movc r1.z, r1.z, -r1.w, r1.w` selected `r1.w` either way. `UI/InternalsSections`
(the Biology panel's organ map, 2 selects), `UI/InternalsPreview` (the fat gradient, 2), `UIEggShader` (2) and the background program (2 lines of 4 lanes) came out wrong
(the map's body outline was a diagonal-cut bell instead of the game's lens: the womb / stomach / jaw / fat sections were in the wrong cells). Fixed in the translators
(`src_raw`), all six functions of `internals_shaders.rs` regenerated (`tools/unity/splice_shader.py` puts one back; the header of the file lists the steps). Found with
a **GPU oracle**: the parity plugin's `POST /d/blit_material` draws a copy of a UI panel's material (`BiologyPanel.sections_mat` / `interns`) with chosen values into a
**cleared** render texture (a pooled one keeps what the last user drew and the UI programs blend: the first captures were unions of earlier pictures) exactly as
`GenerateInternalsMap` does and reads it back; Int properties (`_Width`, `_Height`, `_Factor`: `SetFloat` does not reach them) are set as integers, colours as `_Color.r|g|b|a`;
`GET`-style `POST /d/panel_material` now lists colours too. `tools/parity/capture_material_oracle.py` writes the fixtures `internals_gpu_maps.txt` (11 value sets, the last the
`internsMap` the game made itself for a live bibite) and `internals_gpu_preview.bin` (4 sets); `internals::tests` compare: **0 organ-map cells differ, every preview channel
within 1** (the GPU's unorm rounding). Checked end to end: the port's Biology panel of the fat-40 save has the game's yellow fat ring (`testdata/parity/fat_reference`). The
cb slot layouts of both programs were right all along (the oracle proves it); the stomach acid program had no negated select and is unchanged.
Not verified by an oracle (needs the material of a scene object or the camera's built-ins): the egg program's glow, `bg_pixel` (the experimental background: its parallax
still rests on guessed camera inputs), the skin / eye programs on the GPU path.

**UI: the in-game settings panel (`dyn_panel.rs`).** The Bibite, Animation and Zones tabs are rebuilt from live dumps of the game (`tools/parity/capture_dynamic_tabs.py`,
`print_ui_tree.py`): the Bibite tab 21.6 -> 1.7 mean pixel error (card rows, sliders with their `Background` / `Fill` 0.5 multipliers, dropdown rows, group gaps), Zones 1.2,
Animation (was blank) 0.9, Simulation 1.8. A scroll bar that has nothing to scroll is inactive (`AutoHideAndExpandViewport`: the viewport is 25 wider), the rows that lie partly
outside the viewport are cut at its edge (`Mask`: `UiDraw::Clip` around the children of a generated `Viewport`, their hit and tip rectangles cut too), images on turned nodes are
drawn turned (`UiDraw::ImageTurned`: the zone card's arrow, a quarter turn about z read from `localRotation`), clicking a default bibite / zone item selects it and clicking the
selected one again changes nothing (`DefaultBibitesPanel.ItemClicked`; the old code toggled). `--click x,y` now works in the real window too (before the first frame, like the headless
path); coordinates are window pixels (the 1280 x 720 window has its toolbar at x = 1045 / 1110 / 1175 / 1240, the tabs of the panel at x = 325 + 65 k, y = 108).

**The real window (GPU path), checked against the headless pictures (commits fc6a817, 7d356fd, 79fc4b2).** `RUSTIBITES_SELF_SHOT=<world frame>:<png>` makes the window write its framebuffer
(`get_screen_data`, UI included) at that frame and exit: the operating-system capture of `tools/window_capture.ps1` showed stale frames, this does not (runner pattern: an idle-gated PowerShell script that
sets the variable per variant). Found and fixed with it: (1) `draw_frame_macroquad` drew every `UiDraw::Text` with a TTF stand-in by role and ignored the font name (a face without a TTF, such as
the dynamic panel's "Bibite Balance" title, came out as a thin Roboto, and every run lost TMP's kerning, synthetic bold, italic and tracking): it now rasterises the run with `SdfFont::draw_line_styled`, the
software renderer's own routine, into textures cached per (text, font, role, size, colour, 1/8-pixel phase) (`TextTextures`); (2) Unity's orthographic camera keeps `orthographicSize` when the window changes size,
the port kept pixels per unit, so the window that starts 1280 x 720 and goes fullscreen showed a third more world than the game: `App::set_viewport` scales the zoom with the height in a manual view;
(3) `--click` waits 8 frames (the fullscreen switch resizes the window after start-up), and the window takes `--select-first / --tab / --center-on-selected / --show-targets / --paused` like the headless path.
Eleven scenes (world, Stats / Genes / Biology / Brain tabs, two bibite close-ups, Graphs, Information, pause menu) now differ from the headless picture by 0.5 - 6.3 mean (sum of |dRGB| over three channels), what is left being
anti-aliasing of the toolbar icons. (The headless ruler after a sudden zoom jump is not converged: `RulerHandle.UpdateSize` takes one step per camera-size event, so a 12 000 px bar in a 2-frame shot is the game's own behaviour.)

**Labels auto-size with the real font.** `set_label` shrank a label with 0.49 em per character: `text_sdf::fit_size` is TMP's search (halve the distance between a size that fits and one that does not, steps of 0.05, stop within
0.051) over `SdfFonts::measure_named`; the game's own sizes from its live dump (24.6 / 25 / 25 for the Balance tab's three longest labels) are the test. The Balance tab differs from the game by 1.84 mean.

**Tests.** The two "environment failures" of the heavy suite were stale (the registry audit counted only the 7 eagerly parsed sessions of 60; the library test read the player's 221 templates): fixed, 98 / 98.

**A radiation tower's area is the game's program (commit a7dee61).** `RadioArea` draws `Shader Graphs/radioZone` over a quad of its diameter (`_size` = 2 x radius, `_upp` 5, `_steps` 48, `_wavePeriod` 4,
`_animationSpeed` 0.25, `_Color` (176, 255, 0, 128)): the translated program (`radio_pixel`) runs once per visible 5-unit cell (`radio_cells`: it only reads `floor(uv * cells)`; kept for 1 / 30 s per area and view),
the window draws a rectangle per cell, the software renderer fills them. A glow that pulses with sin(2 pi t / 4 s) plus a ring that runs out, the alpha in steps of 1 / 48; at the pulse peak the colours match the game's
capture within 3 per channel. The clock is the wall clock (`render::set_effect_time`: the game's `Time.time` goes on in a paused world; headless takes `RUSTIBITES_EFFECT_TIME`). The tower's icon is its tint colour
(the sprite is a dark grey the game does not multiply: a white mask now). World pictures of the default save against the game's: 0.85 - 1.67 mean over the central area (the earlier "orange against green" was the port's own
settings override: `userdata/game_user_settings.json` held `FertilityColor` = the default #0B8000 over the player's registry value (0.907, 0.609, 0, 1): removed, backup `.bak-nov10`).

**The colour killer and the towers (commit 88202e4).** Read from the live game with the new plugin route `POST /d/object_renderers {type}` (the sprite renderers, images, animators, components and the plain fields of the game's own scripts of
every object of a component type: `SimulationScripts.ColorKiller`, `RadTower`, `PheroTower`): the killer's crystal is the magenta `laser_crystal` art under a `Square` with `Utility.UnscaledRotation` (rate 0.5: half a turn a second on
the real clock), drawn with `Shader Graphs/LaserCrystal`, which keeps each texel's saturation and value and turns its hue to `_color` = `HSVToRGB(hue, 1, 1)` (the port tinted the art: dark blue-teal for hue 0.4 against the game's bright
green). The translated program (`laser_pixel`) is baked over the art for each degree of hue (`ToolSprites::crystal_hue_cell`), `Primitive::Spinner` turns the sprite when it is drawn (`effect_time`); the seven colours of a hue-0.4 crystal
are the game's capture's (test). The towers' prefabs have a root colour of alpha 0.7725 (the killer's is 1): `EffectTower.LoadState` leaves it, `Place()` sets 1: `TowerBase::loaded` (set by the loader, cleared on export) makes a loaded
tower's base 0.7725 opaque, so the glow shows through it as in the game.

**Still open from this batch.** The cross pattern on the colour killer's disc in the game's capture (it is not the sprite, not the pheromone particles (the player's game hides them) and not on the killer's hierarchy: its source is
unknown); the kill laser (a `Laser` image, 3 units thick, the length of the shot, in the killer's colour for 0.25 s after a kill); the Brain panel's synapse animation; the experimental background's camera inputs; the reset buttons'
sub-pixel corner coverage (the GPU draws a 1.9 px frame at 1/16 scale with filtering); hover / pressed tints (ColorTint 0.961 / 0.784: deliberately skipped); post-impact spin 1.4x / second bounce tick.

**Simulation.** A pheromone spot's trigger collider reaches into the sense query (0.001 for a bibite's spots, 0.1 for a tower's: `pheromone_spot_radius`). A bibite's moment of
inertia is `mass / 2 * realBodyArea / pi`, set every FixedUpdate (verified identical to the port's; the post-impact spin and bounce timing differences listed above are NOT from
the inertia or the shapes).

## Physics, measured against the original game (2026-11-08)

**Superseded 2026-11-11:** what follows describes the game as measured slowed down (ten steps per tick, a Box2D 3 soft step). At 1x the game runs ONE step per tick with Box2D 2.4's solver; the port does that by default now (see "Batch of 2026-11-11": `PhysicsConfig::fine_steps` / `box2d_solver` / `catch_fast_approach`, `RUSTIBITES_PHYS=10,10` brings the ten-step model back). Kept as the log of how the ten-step model was found.

The original's solver is not Box2D 2.4 but a soft-step solver with substeps (found by fitting free motion with damping at several drag values:
the speed kept per tick is `(1 + h c / 10)^-10`, and a body pushed from rest travels only the mean of its ten slices' speeds in the first tick).
`crates/bibites-core/src/physics.rs` now does that: force, damping and motion in ten slices (closed form for free bodies), speculative contacts
(0.05 apart, more for fast approaches), soft push-out, relax and restitution passes, warm starting, two-point manifolds for capsules side by side,
no 100 u/s speed cap (the game reaches 150). Matches to about 0.2 % for a free body, an oyster's acceleration and a held pellet's drag; a head-on
collision's damage within 4 % (16.6 vs 16.0 hp at 250 u/s closing).

Bodies turn about their centre of mass, which Unity computes from the colliders (the area-weighted centre of the solid ones: the body and tail
capsules lie behind the pivot, so a bibite's transform swings round a point ~3 units behind it when it spins). Found because the game's spin after
a head-on impact had the opposite sign to the port's; with the centre of mass the sign, the transform positions after the impact (within 0.2 u) and
the timing of the tails catching again (tick 7 instead of the game's tick 5) are much closer. Still different: the speeds the two bodies leave with
(24.4 / 16.1 u/s in the game, 20.5 / 19.8 in the port: the game's bodies are still closing at 8 u/s after the impact tick), the spin rate (the game's is
about 1.4x), and the second bounce 2 ticks later. Whether the mouth trigger counts in the centre of mass was tried both ways: without it is closer.
The contact islands are solved on their own (on the pool), with the same result as one list (bit-identical, tested through the pinned fingerprint).

**A tick is ten physics steps (found 2026-11-09).** The game's `FixedUpdate` runs ten times per sim tick, each with its own physics step (one substep): the
muscles' `AddForce` is added before every step along the transform's `up` *of that moment*, so a body that turns while it pushes drifts sideways (a 40-tick run
of a bibite with Accelerate 1.0 + Rotate 0.7 now matches the game to 4 digits: (90.6791, 23.1941) both), and contacts are found again at every step, so a fast
body is stopped by the first part it touches at the step where it gets there. The port does this for the bodies that touch (or will within the tick): they form
islands (union-find over the near contacts) and each island steps ten times with its colliders' poses, narrow phase and one-substep soft solve made again at every
step (`fine_steps` / `run_island` in `physics.rs`; the contact events, with the velocities of the step that first saw the contact, come from these steps).
Free bodies keep the closed form over the whole tick (ten slices; with the force turning slice by slice when the body spins). Head-on impact: leaving speeds
24.3 / 16.0 u/s (game 24.4 / 15.9), damage 16.2 hp (game 16.0); a three-body push and two bibites pushing each other match the game's positions to 0.05-0.2 u
over 70 ticks. Movement energy costs agree to ~0.3 % (the game rounds its energy to float32 ten times per tick, which the port does not).
Open: the spin right after a hard impact is ~1.4x in the game, the second bounce comes at tick 7 against the game's 5.

**Grabbing (2026-11-09).** A grab is a real weld joint now (`WeldDef` in `physics.rs`, solved with the contacts in the islands' fine steps; `b2WeldJoint` soft
solve): what is held keeps the pose it had relative to the holder when it was grabbed (frozen at the first step of the grab, kept by tag in
`BibiteScratch::weld_frames`), pellets and prey are ordinary dynamic bodies that collide with the holder as the game's joints (`enableCollision = true`) do,
and nothing is pinned to the mouth any more. Against the game: a bibite pushing/holding pellets of 20, 60, 200 and 400 units reaches the same terminal speeds
(9.90, 7.33, 4.86, 3.72 u/s) and a prey running away at full thrust from a grab of 1.0 / 0.5 / 0.2 drags the holder at the game's speeds. The game's joint never broke in
any test (a 3000-unit pellet at full thrust, a prey at full thrust from a grab of 0.15), so the weld never gave way here either (the `breakForce` formula is
`grab_break_force`, kept unused). **Superseded 2026-11-18**: the game's joint does break for a herbivore's grip (a shrek lets go of a 70-unit plant within ten ticks) and the formula is used now, see the first entry of this file; the comparisons above were made with it switched off and were not repeated with it on. Grabs start at `desireToGrab >= 0.15` as in the C# (it was `>`). Found on the way: the body boxes of the broadphase turned
every collider's offset by the collider's own rotation as well (the mouth's trigger box and the body capsule were misplaced: pellets in front of the mouth were not
seen, pairs missed); the contacts' softness was measured against bodies spawned deep inside each other (hertz at its cap of a quarter of the step rate, damping
ratio 1, no push limit: the game separates a 400-unit pellet that overlaps by 1.2 units within the first tick, Box2D's 30 Hz / 10 / 3 take twelve ticks).
Open: a grab made in the tick right after a spawn is made one tick late (the game's mouth sees the trigger in the second `FixedUpdate`), and the joint's
break rule (when it breaks at all) is unknown.

**Grabbing, second pass (2026-11-09, from `BibiteMouth.cs` line by line).** A holder holds up to 10 bodies (`links = new FixedJoint2D[10]`; the port held 4). What
leaves the mouth trigger is let go (`OnTriggerExit2D` -> `ReleaseTargetIfHeld`; in the port a held target the last step did not see in the mouth). The throw
(`ReleaseAndMaybeThrowAllHeldObjects`) follows the C#: the force comes from the summed mass of everything held, pellets included
(`throwingForceFactor * jawStrength * |grab| * M / (jawMass + M)`), every held body gets `up * force / nHeld` as an impulse added to the velocity it has (the port
overwrote a pellet's velocity and gave pellets a force of their own), and the holder recoils by the whole force when the grab is below -0.25; between -0.25 and
-0.15 the holder only lets go. Measured in the game (`C:\Temp\break_game.py`): with `bitingPressure` from 80 down to 1 and a grab of 1.0 or 0.15 (break force
down to ~0.4), a prey running away at full thrust is dragged along identically in every run: the joint did not break once. So "never breaks" is what the game does,
whatever the formula says.

**Grabbing, third pass (2026-11-09): the grab happens at the physics step, not at the tick.** The game's mouth sees a trigger enter in one physics step and
grabs in the next `FixedUpdate`. A mouth that wants to grab (`desireToGrab >= 0.15`, links left: `PhysicsBody::grab_slots`) now asks the broadphase for a box
that reaches `(speed + 30) * dt` further (`box_margin`; it also wakes resting pellets ahead of it), the pairs it may meet are put in the islands
(`GrabPair`), the island tests the mouth's trigger collider against the body at the start of every step, and the weld is made at the start of the step after the
overlap was first seen, with the relative pose of that moment (`NewWeld`, recorded as held by the bridge after the step). The mouths' trigger lists are corrected
to what the last step saw, so what leaves the mouth is let go by the exit rule above. Against the game, a bibite at 0.6 acceleration driving into resting pellets
of 60 / 60 / 200 units from 14 / 18 / 24 u away: the grab happens in the same tick, and the pellets are within 0.01-0.02 u of the game's after 89 ticks (0.13-0.15
u behind with the grab at the start of the next tick). Still open: a pellet spawned overlapping the body (the first-tick scene) ends 0.46 u off after 60 ticks (200 units;
the 60-unit one matches exactly): the game's contacts of a pair that exists from the start begin one step later than the port's (Box2D finds new pairs at the end
of a step), so the first step does not push the bodies apart there. Triggers of mouths that do not want to grab are still found once per tick (eating and
biting keep the tick's timing).

Speed, 100,000 starter bibites, 12 threads (`ab.ps1`, alternating builds): 34 ticks/s before the physics work, 26.8 after the soft step, ~27-30 with
the island solve and the shared slice terms (solve 12 -> 3 ms); 20,000 bibites 146 -> 125 ticks/s. The rest of the cost is the mirror (bigger solver
bodies, the centre of mass), the narrow phase (the speed of each pair for the fast margin) and the island build.

## Findings of 2026-11-08 (side by side with the original game, driven through the BepInEx bridge and the real window)

- The original game cannot load a save that holds a bibite at negative health (`Object reference not set...`, bisected on the tester's "3 Islands Rust.zip": one such bibite among nine others). The port no longer leaves bibites alive at negative health, so its new saves load; saves written by the first alphas may still hold them.
- At 1920 x 1080 the original game's HUD is about 1.2x larger than the port's (pause button, top-left panel icons, stats box, Biology panel): the port's HUD is ~0.83 of it. Not fixed (the cause is not known: the scale setting reads 1.0 on both sides).
- Biology panel Internals: the fat sections are drawn as thin yellow outlines in the port where the game fills them with a smooth yellow gradient, and the port adds yellow outlines round the womb and the stomach (the shader program is the game's; its fat parameters or a texture it samples are suspected). Stomach pellets and womb eggs now turn at the screen's resolution like the game's.
  **Fat, measured against the game (2026-11-09).** A real reference now exists (`testdata/parity/fat_reference/`, gitignored, with the capture scripts): the hidden game with the ParityDump plugin (copy `testdata/parity/plugin/bin/ParityDump.dll` into `...\Steam\steamapps\common\The Bibites\BepInEx\plugins\ParityDump\` for the run, delete it after) shows the first bibite of `rust_seed_600.zip` with `fatReservesAmount` 0.52 and 40 (buttons `OpenData` then `BiologyButton`, camera on the bibite). At 40 the game fills the four wedges between the organs and the body edge with solid bright yellow (a smooth gradient, thick bands) and draws NO yellow round the organs; the port draws the same wedges as thin yellow stripes with dark gaps and a yellow outline round every organ. The port's inputs are the game's (inflate 0.536 = (40 - baseCapacity 20.17) / (0.35 * area), `_FatFullness` 1, fat range 0.049-0.951 from the sections map, as `BiologyPanel.GenerateInternalsMap` / `UpdateFatSection` set them), the body outline and organ layout agree, and no inflate / fullness pair (0-1 in tenths, fullness up to 20) or ordering of the three fat constants in the buffer reproduces the game's picture (best yellow-mask overlap 0.22). So the difference is in the translated `UI/InternalsPreview` program (it contains `sin`/`cos`, `round_ties_even` and `floor`/`fract` steps in its fat branch) or in a constant the buffer layout has wrong (its slots 5.y-5.w and 3.x are guesses from the material's property order); checked afterwards: `UI/InternalsPreview` and `UI/InternalsPreview_Fallback` are the same program (globalgamemanagers 77 / 67, 14420 bytes each) and its disassembly agrees with the translation on the fat constants (`cb0[4].z` fullness, `cb0[4].w` start, `cb0[5].x` end, `cb0[2].z` width), and the fat range forced to (1, 0) or to the captured material's (0.82, 0.197) gives the same stripes. Live values read from the game for that bibite (plugin route `POST /d/panel_material {"field": "interns"|"sections_mat"}`, which reads the Biology panel's two materials; the DLL must be rebuilt with `dotnet build tools/parity/plugin/ParityDump.csproj -c Release -p:GameDir=<Steam dir>`): `_InflateFactor` 0.535855, the six portions (0.190816328, 0.099999994, 0.381632656, 0.049999997, 0.08163266, 0.005102041), `_FatFullness` 1, `_FatEnd` 0.04918033 (3/61), `_Width` 61, `_Height` 39, `_Factor` 1 (an Int) and **`_FatStart` 0.8196721 (50/61)**: all identical to the port's inputs except `_FatStart`, which the port finds at 58/61 (0.9508). `_FatStart` comes from the sections map (`GenerateInternalsMap`), so the port's map differs from the game's: in the port the thin ring of fat cells (7) between the organs and the body reaches columns 51-58 of the map's upper half (the arms), in the game it ends at column 50. `_FatStart` does not change the preview at fullness 1, so it is a symptom, not the cause of the stripes: the same inputs give a different picture, and the translation reads the same as the disassembly, so the remaining suspects are an evaluation difference (the GPU's `sqrt` / `sincos` / `asin` polynomial, `mad` fusing was tried and changes nothing) in the code that decides which organ a cell belongs to, at the one-cell rings between organs (the game's rings are dark and two cells wide; the port's are one cell of yellow beside a dark band). **The game's own sections map was read back** (plugin route `POST /d/panel_ints {"field": "internsMap"}`, saved as `testdata/parity/fat_reference/game_sections_map_fat40.txt`, 61 x 39, row 0 at the bottom): 438 of 2379 cells differ from `sections()`, all of them between body (1) and fat (7) (plus a few 6/1 at the arms): in the game the fat fills the whole wedges between the organs and the body edge and the thin rings round the organs are body (1); in the port the wedges carry body stripes and the rings are fat. In the sections program the last distance test (`ge r0.x, 0, r0.x` giving 7/7 against 1/7) is 0-3.4 cells too large in the port in the cells the game calls fat. This is the simpler program (419 lines, `sharedassets2` shader 85) with the same inputs, so it is the place to find the evaluation difference (compare it register by register with a float32 numpy evaluation of the disassembly at one differing cell, e.g. map column 28 / row 34). Tried without effect: all 720 orders of the six portions in the buffer, cell shifts of up to 4, all start/end/inflate/fullness values. What is left: the order in which the fat branch uses the hatch term (`fract` of a rounded coordinate, the 0.2 grey constant) against the game's picture, or inputs the port cannot see (the game's `_Width`/`_Height`/`_SizeFactor` slots 2.z, 2.w, 3.x are shared with the organ layout, which does agree).
- Flow suffix reads `E/s` in the port and `E/S` in the game. (Checked 2026-11-09: the game has `E/S` only on the main Flow bar's two texts, and the port's bar prints `E/S`; the breakdown pop-up's rows are `E/s` in both.)
- HUD scale, rechecked 2026-11-09: rendered at 1920 x 1080 the port's HUD (pause row, time block, toolbar buttons, scale bar, Biology / Stats / Brain panels) lines up with the hidden game's UI-camera references to the pixel, so the "1.2x" of the real window is not in the prefab numbers: the canvas scaler is constant-pixel-size with `scaleFactor = ScreenResolutionFactor * UIScale` (`UIScaler.cs`, factor = min(height / 1080, width / 1920) when "Automatically Detect Screen Resolution" is on), both of which the port implements. It needs a real-window capture of both programs on the same monitor to settle (their window sizes / the saved UI settings).

**Text of the inspector panels (2026-11-09).** Their titles and labels are `LiberationSans SDF` with the Bold style, and the game ships only the regular face: TextMeshPro makes it bold by widening the glyphs (`_WeightBold / 4` of the distance field) and advancing every character 0.07 em more (`TMP_FontAsset.boldSpacing = 7`). The port drew them with Roboto Bold (12-15 % too narrow, grey). It now draws the game's own Liberation atlas with those two changes (`text_sdf.rs`: a font name with the `+bold` suffix; `ui_runtime.rs`), in pure black (the material's face colour is black and multiplies the label's colour), centred on the glyphs' ascent-to-descent extent (TMP does not centre on the line height) and ignoring trailing spaces when centring a line. Against the references: the Biology panel's labels have the game's width to one pixel and its ink mass within 3 %, the Stats panel rows are within 2 px, the Brain tab's buttons are in place; two captured menu screens also got closer (New Game 8.99 -> 8.73, Load Game 3.23 -> 2.94 mean pixel difference).
- The lineage graph with the player's own lineage settings matches the game (339 recorded, 325 displayed).
- Species panel, measured against the game (2026-11-09 and 2026-11-10; `testdata/parity/species_reference/` and `C:\Temp\species_out` are gitignored scratch: the game's overlay and the live `GET /d/ui` dump of species 2310 of `testdata/real/saves/lol.zip` on each tab, captured with the plugin route `POST /d/open_species {"id": 2310, "tab": 0..3}` (the `tab` option is new: `SpeciesTabManager.tabButtons[tab].Select()`); the port's picture of the same state is `rustibites-view lol.zip --lineage-species 3 --lineage-tab N --ui-only --screenshot x.png` (**use `--ui-only`: it renders at 1x like the game; the default 2x supersampled picture blurs every 1-px line and makes the diffs look worse than they are**)). Fixed on 2026-11-10: (1) both panels are content-sized like the game (Info: `species_info_layout_data.rs`, generated from level2 GameObject 331 by `tools/unity/gen_layout_data.py`, solved by `unity_layout` with the species' texts: 308.44 / 273.21 / 375.75 / 347.09 wide for the four species measured, the Preview Section as wide as its widest row, the root / parent buttons' names and the 125 of the titles deciding for a short name; Genes: 15 + the longest of the gene lines (the longest gene name, "Pheromones Sensing Radius:", + 15 + 90 + 100 = 405.91), the name row, the parent row and the compared row), the tests compare the solver with those live numbers; (2) the close tab ends 10 before the panel's right edge (`Species Tabs` is stretched over the panel, a flexible spacer before the last tab), the tabs rest 45 from the top and the selected / hovered one rises 11 (`TabsButton.initialY` -5, `verticalTravelOnSelection` 11: the port had 50 and 16); (3) the favourite heart has no frame, a grey `output_want2heal` drawn by `flatMat` (the sprite's shape in the Image's colour: the port multiplied the dark sprite), 1.5x, the orange check mark a child of it (so as big); (4) the notes field's `InputFieldBackground` border is 5 units (ppu 200), "Notes:" has 10 of padding over it, the placeholder is italic (TMP's shear, `text_sdf::ITALIC_SUFFIX`); (5) a disabled button greys only its frame (`ColorTint` reaches the target graphic, the child icon keeps its colour); (6) bold-style labels (`Roboto-Regular SDF`, Bold) are TMP's synthetic bold (`+bold` font names, 0.07 em per character but the last), the gene names are black like all `LiberationSans SDF`; (7) the species picture: the parts sit 4 units ahead (`INFO_PANEL.part_shift`), the arms stand on the body (Unity's pivot (1, 0) is the bottom right; the rasteriser's counts from the top), the arm column follows the speed gene on every tier (the hard-coded normal-tier cell is gone), a tree node's bibite does not turn with its diet (only a `BibiteGenePreviewer` with food icons does); (8) the Brain tab: the highlighted differences are the game's shaders, read from the DXBC (`tools/unity/shader_asm.py sharedassets1.assets 82|81`): `synapseActivation` outlines the point-sampled 12 x 7 tile one texel out (`1 - prod(1 - alpha of the four neighbours) - alpha`) mixed toward blue (0, 0.0119, 1) with alpha `sat(a + sat(outline * _HighlightAmount + a))`, `nodeActivation` does the same two texels out for `_Selected` (a neuron with any difference), `_Invalid` mixes toward red; the textures are Point / Clamp so the strips are point sampled now (the port blurred them); the node names are 1.2857 big and black; (9) the Position tab draws each zone's name, `125.66 kE` and `~26 pellets` over its middle (`ZonePreview`), here and in the Zones tab's preview; (10) the top bar's "Ancestral Lineage Graph -" is `LiberationSans SDF - Fallback` 24 bold, the time axis labels the same face at 16.95 x the graph's zoom, the lineage scroll bars use the `Background` / `UISprite` sliced sprites, a solid rect covers the pixels whose middle is inside it (the GPU's rule: a 1-px line at a fractional height is one row, not two). Still different: the node layout of the Brain tab (the game's sorter uses Unity's random numbers; a template has no saved anchors); the bar's handle size (the game's `Scrollbar.size` was 0.2164, the port's 0.29: the game's capture may be stale); text anti-aliasing (the game's SDF text is a little heavier / crisper); the zone's colour comes from the player's own `FertilityColor`; the inspector's Brain header (fixed later on 2026-11-10: the species box was given half of the row's spare width by the layout solver; the game's is exactly its content wide).

## Visual and UI parity, 2026-11-10 (hidden-game captures at 1x, diffed with the port)

**Method (reusable).** The hidden game with the ParityDump plugin (copy `testdata/parity/plugin/bin/ParityDump.dll` into `...\The Bibites\BepInEx\plugins\`, delete it
afterwards), driven by a Python script on `tools/parity/capture.py` (`WorldSession`, `plugin_json`): `/d/ui_overlay` is the UI camera's picture and is **premultiplied**
(over black its RGB is the picture: composite the port's straight-alpha render over black, never the game's again), `/d/ui` has every node's `screen_bbox` (y up), font,
size, style, sprite and colour (inactive nodes have no box), `/d/press_button {name | path}` (a name can match another panel's button: pass the path), `/d/select_bibite {id}` or
`{x, y}` (new: the nearest body), `/d/open_species {id, tab}`, `/d/activate {path, active}` (this opens a custom `MyDropdown`: it only shows its inactive `Template`), and the bridge's
`client.screenshot(1920, 1080, x=, y=, size=)` is a world-only picture of any spot (`size` = half the visible height). The port's picture of the same state:
`rustibites-view testdata/parity/default/rust_seed_600.zip --no-menu --width 1920 --height 1080 --frames 40 --select-first --tab stats|genes|biology|brain --ui-only --screenshot x.png`
(`--click 1685,40` bar chart, `1750,40` pen, `1880,40` gear, `1868,95` Info +, then `139,30` Go To Gene Analysis; world views: `--center X,Y --ortho-size N --no-runtime-ui`).
Never run `python -` in this shell (it waits for stdin until the tool times out) and write patch scripts with the Write tool.

Whole-screen mean absolute difference, before -> after: Graphs panel 4.04 -> 0.50 (rebuilt from the dump: the 258.6 x 39.4 page button, 400 x 40 `UISprite` dropdowns (ppu 200),
238 x 40 tabs with the disabled `ColorBlock` tint, the 2 px grid at 77.9 / 69.72, white Roboto labels, `Color.yellow`, curves drawn first-over-last, flat ends); pause menu
36.2 -> 0.46 (the dark blue veil, a 386.7 section with the version at its top left, seven 50 high buttons in Boston Black capitals with their icons at the right); Gene
Analysis page 5.06 -> 0.69; dynamic settings panel 7.47 -> 4.04 and the Credits tab 6.39 -> 2.49 (a scroll viewport's `UIMask` is the `Mask`'s graphic and is not drawn);
inspector Stats (region) 6.14 -> 3.05, Genes 4.93 -> 1.82, Brain 1.88 -> 1.74. What did it:
- **The SDF edge ramp is TMP's sqrt 2 px, not twice that** (`text_sdf.rs::ramp_half_width`: the atlases change by `1 / (2 * gradientScale)` per texel, measured; the old formula assumed `1 / gradientScale`). A live 30 px stem is `[190, 255]` / `[255, 133]`, now `[210, 255]` / `[255, 132]`.
- **Pair kerning is the game's own table, and most faces have none** (`kerning.rs`, `assets/game/ui_fonts/*_kerning.json`): the plugin route `GET /d/font_kerning` reads `TMP_FontAsset.fontFeatureTable.glyphPairAdjustmentRecords` (the dump's `kerningTable` is the legacy table and is empty). Of the game's font assets only `LiberationSans SDF` (104 pairs, point size 86) and `Roboto-Bold SDF NoOutline` (505 pairs, point size 90) have pairs; `Roboto-Regular SDF` (the body face of most of the UI), `Roboto-Bold SDF WhiteOutline` / `BlackOutline`, `Boston Black SDF` and `Inconsolata-SemiBold SDF` have none, so their text is never kerned (the first version of this applied the TTFs' whole GPOS table, which kerned "Tag:" in Roboto-Regular by 1.2 px the game does not). The value is `xAdvance / pointSize` ems added to the first character's advance (`measure_named` and the drawing); proof from a capture: "Total Damages Suffered:" (Liberation, bold style) is 221.14 kerned + 21 x 0.07 x 20 = 251.94 wide (captured 251.95), "Tag:" (Roboto-Regular, bold style) 43.31 + 3 x 0.07 x 22 = 47.93 (captured 47.94). The plugin also dumps each text's `enable_kerning`. The stats solver's measure bolds every face by 0.07 em per character but the last (the Liberation titles used a fitted 1.1277 ratio), and `the_stats_layout_solver_reproduces_the_captured_inspector` now holds every node of the panel to 0.6 px with no exemption. Dynamic panel 4.04 -> 3.73, Gene Analysis 0.69 -> 0.64, Credits 2.56 -> 2.44 after the correction.
- **Character spacing is drawn** (`+sp<n>` font-name mark, thousandths of an em): the inspector's gene values are Bold with a spacing of -5, so the right-aligned block was placed 4 px narrow and drawn 4 px wide.
- The New Game screen's modifier names (`ParametersPanel/Content/Options/.../Name`, boxes up to 123 high) are middle aligned like the game's; a nine-slice frame drawn smaller than its texels (the dropdowns' `UISprite` at ppu 200, half size) is sampled bilinearly like the GPU does (the 1-texel outline comes out 147 against the game's 148, was 255): New Game 8.79 -> 8.23. Most of what is left there is the harness: `--ui-session` has no `MenuState` (no label wrapping, the placeholder preview), the real flow is `--menu --menu-view new` and it wraps "Size of the / Simulation Area" like the game.
- The Information panel's titles and the Stats panel's Tag row are bold black (`LiberationSans SDF +bold` / `Roboto-Regular SDF +bold`); the Graphs dropdown lists use `checkmark_icon`; the inspector Brain header's species box takes no share of its row's surplus (Unity's rule would give it half: the capture and the live game both show it exactly content wide).
- **World**: the zone fill is the game's pixel program (`Shader Graphs/PelletSpawnPoint`, sharedassets1 path 82 -> `tools/unity/shader_asm.py`): alpha = `opacity * saturate(100 (1 - d)) * (0.5 + c (0.5 - d^2))`, c = +1 centric / ring, -1 exterior, 0 flat, d the distance over the radius (a ring measures from its middle): the Default world was a flat plane, the game's falls to 0.48 at the corners (`Primitive::ZoneDisc`); the grid is the end of `Shader Graphs/background`'s program (sharedassets2 path 78): lines are world units wide (1 u fine, 5 x that coarse: the settings say "u"), a solid core and a soft edge a tenth of the half width, over the whole view (not clipped to the square), the fine mask is `max(x, y)` less the coarse one (`Primitive::GridLines`); the selected bibite's view cone has an opaque white border 5 px across at 18 px/u and 2 at 3.6 (about 1.3 sqrt(zoom), at least 2).
Panels opened from the toolbar and the dynamic settings panel were built **without the SDF font metrics** (`build_session_frame_filtered(.., None, ..)` for every overlay but the base frame: the text height fell back to `1.25 x size` and the widths to `0.52 x size` per character): the Simulation Options labels (Roboto-Bold 25 in a 40 high box) sat 0.97 px high and the Boston title 0.9 px low (the line is 29.30 / 46.80, not 31.25 / 45; TMP's own line info read with the plugin's new `text_info` dump agrees: baseline = box centre + (ascent - descent) / 2, ascent 23.19). They now get the fonts (`build_frame_with_fonts` passes them on, also to `build_dyn_panel_frame`): dynamic settings panel 3.73 -> 2.51. What is left there: the game's inspector tab buttons and the time readout (state of the capture), and the scroll viewport's bottom (the game shows "Ticks Per Second" under "Simulation Performance Parameters" where the port's mask has already cut it).

The dynamic panel's **tabs**: the five tab buttons are identical in the game whichever tab is selected (captured with the Simulation, Zones and Bibite tabs selected: all (227, 227, 227)); the port dimmed the unselected ones to 72 % (an early guess, `tint_tabs`, gone: the tab row 14.8 -> 0.43, Simulation tab 2.51 -> 2.28). The in-game Bibite tab's list title is "Bibite Templates" with no "Bibite Settings" heading after the list (the scenario editor's declaration has both; fixed here). The in-game **Zones tab** was rebuilt from a live dump (`ZoneEditorPanel`): the headers are the game's own (a 40 x 40 `+`, the 36 px Roboto-Bold title, a 40 x 40 "remove all" whose frame is greyed by the disabled tint while its list is empty, a 42.2 high row; `layout_header_in_sim`), the zone card is 38.5 + 20 high with the zone's name only (no "Name" caption: that one is the scenario editor's) and the list ends with 60 of padding, and the content's children are 15 apart (the other tabs' groups 20). Whole screen against the hidden game: **8.3 -> 4.8** (Simulation 2.0). Left there: the card's arrow (the game draws `Arrow` turned 180 degrees, `local_rotation` z = 1, when closed; the renderer has no flipped / rotated image draw, so none is drawn) and the scroll bar, which the game hides while the content fits. Still different: the **Bibite tab** (21.6): the game's template card is a compact two-column grid of real dropdowns and sliders (Tagging / Spawn Zone / Spawn Logic / Priority / Target Count / Growth At Spawn / Mutate Genes) with a trash icon, the port's open card is a one-column list of plain values with an X, and its headings are a little smaller. The Bibite tab needs rebuilding from a `/d/ui` dump of it (`tools/parity/capture_dynamic_tabs.py` captures both tabs; `tools/parity/print_ui_tree.py dump.json DynamicSettingsPanel 24` prints the active tree with screen boxes) (`/d/press_button` with the paths `...DynamicSettingsPanel/TabButtons/ZonesTabButton ` (trailing space) and `.../BibiteTabButton`), like the Graphs panel was.

Still different / not done: the **window** (GPU) path of the new world primitives was seen in the real window (`tools/window_capture.ps1`, far and mid views of `rust_seed_600.zip`, 2026-11-10: the zone fade, the 5 u coarse and 1 u fine lines and their colours match the headless picture; note `--click` does nothing in the window, panels there need real input, and `--ortho-size` gives a different zoom from the headless one (60 -> 5.9 px per unit against 9)); it draws the grid as strokes (crossings stack a little; a GPU material would be exact) and the zone through the same quarter-resolution texture as the microscope shade; the Graphs legend popup (hover) and the dropdown lists' rounded frame were not compared; the game's text is still a hair heavier in places (the Information panel's "count" button); the Information panel's pie outline is thicker in the port; the zone colour comes from the player's own `FertilityColor` (the hidden game's is orange, (231, 154, 0): the port's default is the game's green); `Go To Gene Analysis` ink is 2 px wider than the game's (kerning of other pairs); sub-pixel rectangle edges (a GPU quad covers the pixels whose middle is inside it; the software image path rounds outward, which the title screen's reference prefers: tried and left).

## Tester reports of 2026-11-07 (first alpha)

- Fixed in core: a holder was pushed backwards every step by the body it held (a held pellet / bibite collided with its holder; the game's `FixedJoint2D` does not collide). Probably also the "inside a pellet" report.
- Not reproduced yet (need a save or a real-window drive): Back to Menu leaves the title menu dead; icon hit areas not following a window resize; lineage list not scrolling and a too-small species spatial bubble on one machine; a negative-health bibite laying eggs at ~10000 E/s (in the game `Hurting` is the only health death check); oyster growth rate; sprite details (border artifacts, limbs, fat, internal pellet / egg orientation, species-view sprites).

This is the short list. Details, C# file:line citations and the reasoning are in
[`PARITY_AUDIT_SOURCE.md`](PARITY_AUDIT_SOURCE.md) (simulation, save/load, challenges) and
[`PARITY_AUDIT_UI.md`](PARITY_AUDIT_UI.md) (viewer). Everything below was derived by reading the decompiled C# in `Here/` against the Rust
code, not from the older status docs (which were found unreliable).

**Honest status.** Full visual + mechanical + UI parity is **not** reached and cannot be proven from here. The simulation core is much closer
than at the start of this pass and has measured evidence. The viewer has now been driven with real mouse/keyboard input in its window
(see "Found with real input" below) and its title-menu screens, Esc panel, camera, selection, inspector tabs and in-game
Settings/Library/Load panels were checked that way; most other panels were only checked by headless screenshots or not at all
(the viewer's unit tests can be built and run with the light profile, see "UI mechanics" below; the heavy `ui_runtime` ones
load the captured sessions and need `--test-threads=1`).

## Open items as of 2026-11-25 (re-audited 2026-11-11, updated with the tester reports of 2026-11-12 and 2026-11-13, the glitch save of 2026-11-13, the reports of 2026-11-14 and 2026-11-15, the code audit of 2026-11-16 and the brain, sense and population checks of 2026-11-18 and 2026-11-19, the fixes of 2026-11-20 the calibration and window-shape layout of 2026-11-21 the speed work of 2026-11-22 and the weld-group fix and the tester reports of 2026-11-23 and 2026-11-24 and the scenario editor's animators of 2026-11-25; the sections below are a log, where they disagree with this list this list is current)
Simulation, measured against the game on three worlds (the 1202-bibite "Branching Desert 2" save for 300 s, `lol.zip` for 300 s, the controlled contact fixtures): births, starvation deaths, population, energy, health, speed, bite rate and
Accelerate output all agree within noise; a bibite's per-tick motion without contacts is exact (position error 0.0004 at tick 1); isolated bibite / pellet / bibite contacts match in timing, velocity, position and rotation. Open:
(1) the meat stock: FIXED 2026-11-11: with corpses off a starving body leaves four meat piles in the game, one in the port; Branching Desert after 300 s: meat 148.8k / 138.2k u² in two game runs, 153.6k / 156.6k in the port (it was 120k);
(2) the contact response of the save is still weaker than the game's in the 5 ticks after a first contact (pile fixtures; not a material value): circle margin and plant friction were fixed, grabs were broken at one step per tick and are FIXED; (3) kills by other bibites: NOT a gap (the 98 was the capture's guess at the cause; four port seeds give kills 86 / 108 / 79 / 86 against the game's one run of 82); (4) grabs: re-checked and fixed (a bibite grabbing a bibite is not); (5) slow motion: the game runs 1/speed steps per tick; the port reproduces 0.5x (2,2) and 0.25x (4,4) with `RUSTIBITES_PHYS` and does not follow the speed; 0.75x, 0.33x, 0.2x fit no step count; (6) only Branching Desert 2, lol.zip and fixtures were compared (no towers, killers, red death); starvation death comes one tick AFTER the game's in a fixture (not the other way); pherosense and colour killers use colliders already; the held-pellet drag toggle is applied (the game reads it at the next drag update); bite-killed victims' own CorpseUpdate is not modelled (update order unknown); the brain-mutation counter quirks are not found. Port-written saves load in the game with the same counts (checked: lol, Branching Desert) and run.
Viewer: the experimental background's parallax rests on guessed camera inputs; the synapse shader's animation, the animated radio-zone shader and the kill laser; hover / pressed tints (the pressed colour is done for buttons held in the window); tooltips of the placer, tower and Brain panels;
the compared dropdown's overflow; the Graphs legend popup; real mouse / keyboard drives of the Brain editor's editing and the lineage panels; the title-menu screens replayed over a running world. Settings screens, the Library and the scenario editor have each been rebuilt from live dumps; see their sections.

New since the first alpha: text (any Bold-style label is TMP's synthetic bold, the bold weight and the outline follow the game's materials), the Lineage panel (the window never gave it the wheel / buttons: Ctrl + wheel zooms, the wheel scrolls, the right button drags; the mesh view's shader, view place and labels, the tree's highlight and axis labels follow the game; zooming is smooth), saves written before the 2026-11-11 fix get their lineage `maxEnergy` rebuilt on load, animators may have two points at one time (an instant jump, as in the game), the optimised viewer, the title menu at a UI scale other than Normal or in a window that is not 16:9 (it was drawn and clicked off the screen: the menu was unresponsive; also after maximising or resizing), grabs (a bibite holding a pellet or prey now carries it from the moment the mouth meets it: at the default one physics step per tick the hold was recorded but not applied), contact physics against the game (pellets touch only when they overlap, plant pellet friction fitted), and from the tester report of 2026-11-12: pellet sizes above 50x are kept (so are the other zone numbers the game lets go past their slider: fertility, biomass density, speed factor; a vanilla save with 100x pellets came in at 50x), the time speed can be typed (click the number beside the slider), a loaded world resumes its bibite spawner and a spawner row keeps its progress when another row is added, a template's species name is the species a placed or spawned bibite joins (it used to found a new one), Void-No-Mo' steers by the nearest zone only (as the game does), genes are capped when a bibite is made from a template (an average brain mutation of 16 spawns as 10), the Biology panel shows the bar at maturity 1.0, window text is drawn at the display's real pixels (125 % / 150 % Windows scaling made it blurry: probable cause, unconfirmed), and a camera locked on a bibite no longer vibrates (it followed the bibite a tick behind); and, from the tester report of 2026-11-13, the simulation's speed no longer depends on where the camera is or how big the window is (a frame's ticks run beside its drawing, the zones are drawn by the GPU at the window's resolution, the display list of a busy view costs a seventh; in a big world with the camera over it the speed was half of what it is over the void, now it is within 4 %), and the blood and pheromone particles no longer stay on the screen at a high time speed, and, from the glitch save of 2026-11-13, a long run can no longer grow bibites that never starve (a grab joint was welded to whichever bibite had taken the slot of a prey that was gone, and threw bodies out of the world until their positions were NaN; a NaN output then kept a bibite without energy alive and laying eggs: 264,000 of them in three hours; a save written by such a run opens again, with its NaN bibites left out), and, from the tester's picture of 2026-11-13 ("artifacts floating around the bibite"), the faint dotted lines the window drew along the rectangles of a bibite's sprites (the window's surface is 2x multisampled and the part sheets have no padding between their cells: a pixel on the edge of a sprite's quad read the neighbouring cell; the sampled rectangle is now kept 0.75 px inside the cell; in the real window 48 such pixels in empty space round a bibite at 17.5 pixels per world unit, none now) and the body of a coloured bibite with a bent tail that went grey when it was seen big (the finished picture was drawn through the colour material again); the bug report is fixed too (Make-Bug-Report.bat had an invalid regular expression and put no screenshots in the report; it now also holds the memory, the graphics card and driver and Windows' record of a crash, so a crash can be looked into, and Play.bat takes two flags at once: `Play.bat -ViewerArgs --no-overlap,--no-gpu-hue-shader`); the ticks per second can be changed in the in-game settings panel while a simulation runs (the game does it live; the port refused with an error and the slider snapped back: it now takes effect from the next tick and keeps the time made so far); a click on Load no longer freezes the window ("not responding": the list of 1,500+ autosaves was checked file by file on the window's thread, and a world was read there too; both run beside the window now); the left arm of large and giant bibites is drawn where the game draws it (it hovered off the body, or lay on it); and a bibite or species saved as a template is listed by the Library (the Species panel's Save opens the game's Save dialog and writes into the Templates folder, and the Library lists the folder the alpha saves to, not only the real game's); and a window that does not run the GPU colour program (a driver that rejects the shader, or --no-gpu-hue-shader) no longer draws the bibites blue and a black one black all over (it recolours them on the processor); and, since the alpha of 2026-11-14: the simulation times its own ticks and runs them the fastest way on your machine (a big world on all your cores, a small one on a single core when that is faster; the lines that start with "pool:" in logs\last_run.log say what it chose; the title menu starts worlds with every core), bibites welded to a wall of enormous pellets are no longer thrown across the world at hundreds of units a second (a maze built of giant plant pellets, a Plant massDensity of 1e35: a bibite that touches such a wall is still welded to it and starves there, as in the game, but a bibite that grabs the meat it leaves is no longer thrown out of the world, and meat flung at hundreds of units a second no longer kills whatever it touches), the sliders of the title menu's Settings and New Game screens and of the pause menu's Settings can be dragged (they only took the click), a press on the handle of one of those sliders, of the time-speed slider or of a row of the settings panel no longer makes the value jump (the handle stays under the pointer until you move, as in the game), the inspector's Brain tab shows the value of the node the pointer rests on, the title menu repaints about four times faster (a repaint took a quarter of a second at 1920 x 1080 and nearly half a second at 2560 x 1440, so a drag stuttered; now 55 and 96 ms), new species are named from the game's own list of Latin names (it was a placeholder list), the Information and Statistics panel, its Info tab and the challenge's tracker go away while the settings panel (the pen) is open, as in the game (they covered the settings panel: the OK of the animator creator was under the Information panel), the settings panel's tabs, Save, Close, sliders and animator creator answer clicks where they are drawn in a window that is not 16:9 or at another UI scale (they answered somewhere else, and the Zones preview did not follow the panel), opening the Lineage panel recounts the active species for the species span as the game does (and the inspector's distance / span ratio reads the span the simulation uses), and panels that hang over each other no longer share click areas (a click on the Save Bibite dialog also closed the Information panel behind it or swapped its count / energy section, a click on the Biology Flow pop-up's Sort toggle also closed that panel, a click on the dimmed screen of the pause menu over the Graphs page closed the menu, and a click on the 31st or a later tag of the Information panel's list also paged the list), and, since the alpha of 2026-11-15: a save written over another (the quick save, an overwrite you confirmed) keeps the old one when the write fails half-way (a full disk, the program killed): the new file is written beside it and swapped in whole, and the settings file and the templates are written the same way; a window that cannot use the game's own UI (`--simple-hud`, or a graphics driver that rejects it: "UI upload failed ... using simple HUD" in logs\last_run.log) no longer stops the world from its second frame; the in-game Settings, Load Game and Library panels closed with Escape no longer come back with the saves, bibites and settings of their first opening (the Load list missed the saves made since, and a click in the panel put the old settings back, so the Biology pop-up's Sort toggle jumped back); a bibite that lets go of a held wall of enormous pellets (the maze) with a throw is no longer given a NaN speed, which killed it as a hazard; a mouth that already holds ten things no longer marks more pellets as held with nobody holding them; when one of two bibites that hold the same pellet loses its hold, it is that one that lets go; a new species keeps its parent's genus for five species of a line, not six, as in the game (the names of new species in long lines differ from earlier builds for the same seed); the settings panel's X closes the panel while the pellet placer, a bibite placer or a tower's panel is open (it did nothing); the mouse wheel zooms over the place of the Information panel's list while the HUD is hidden (H); a click on the handle of a slider of the Settings screens without moving it no longer changes the setting; the Library's picture follows a template that was saved again under the same name; and changing the species span in the Lineage panel's settings counts the species that are active now, as the game does; and, from the players' report of 2026-11-16, zooming out far no longer drops the frame rate (a bibite a few pixels long is drawn as its body alone and a smaller one, a pellet or a particle as a dot, and the dark ring round the world is drawn by the graphics card: Archipelago at the widest view went from 13 to the monitor's 145 frames a second), and a bibite whose three pigments are exactly 1 is no longer drawn black all over by the graphics card's colour program (it gave black for a colour of exactly (0, 0, 0) and was right for 0.98; reproduced in the real window with a save of one such bibite), and the title screen shows the edition's name ("RUSTibites Faithful" or "RUSTibites Improved") in its top left corner, and, from the report of 2026-11-16, the Decay switch and the Freshness Time of the Plant and Meat materials in the in-game settings panel now act on the pellets already in the world (they only changed the pellets spawned afterwards, so switching plant decay on did nothing to the plants that were there), and the program has a new icon (the bibites picture with a red pellet that reads "RUST"), in its window and on the RUSTibites.exe file itself, and, since the alpha of 2026-11-17: a long run of a big world no longer collapses (a saved world of 600 "shrek" bibites that held steady in the game, 641 of them after 900 s, shrank to 196 in this program and now ends at 645: a pheromone angle read 180 degrees when there was no pheromone at all, so a fifth of the bibites kept turning for a signal that did not exist; a pellet in a bibite's mouth vanished from its field of view, so it dropped it and ate less; and a grab that should have broken never did), a bibite's brain does its arithmetic exactly as the game does (compared one tick at a time with the original game: every node of every brain bit for bit over 9,600 brain ticks on five worlds), what a bibite senses is computed the way the game computes it (the game's own vector maths, field-of-view weights, counters and its pheromone sense that is one tick late), and a new bibite or egg starts with fresh brain nodes (a child used to keep the values its parent's nodes held, so a node with its synapse disabled and no bias could output a constant pheromone or herding value for the bibite's whole life; a world saved by an earlier build is corrected when it is loaded; a node nothing feeds and with no bias still rests at 0.5 if it is a Sigmoid and at 1 if it is a Gaussian, as in the game). And, since the alpha of 2026-11-19: a long run of a world with many grabbing bibites no longer throws pellets and bibites out at hundreds of units a second from two big piles that kill whatever they touch (the contacts between a bibite and what it holds fought the hold and wound up; they are now left out of the solve, as the code always meant; a smaller leftover of the same kind in a jam of pellets is not found yet), Mutate Genes: All Genes no longer spawns enormous bibites (the size of a baby, an adult or an elder is read from the template's genes, as the game does) and a placed egg takes that choice too, the Random and Species tags of a placed or spawned bibite or egg tag it (they did nothing; a bibite spawned with Mutate Genes on All Genes is tagged too, which the original game does not do), the time-speed slider works in a window that is not 16:9 or at another UI scale (its track stayed where it is at 1920 x 1080), the Armor material's picture in the settings panel is the grey pellet, the Red Death bloom works in a world that is loaded or started with it switched on (it only started after some setting had been edited, so it hurt nothing, was not smelled and nothing showed) and is drawn as the game draws it (the game's own shader program on the graphics card, per pixel: its purple and red cells, their sizes, the ring that grows in from outside the dark ring round the world and the red it leaves in the world were fitted to pictures of the original game, and the bloom lies over the zones and under the pellets and bibites; its red pheromone particles are not drawn yet), and the grid is drawn the way the game's shader draws it (per pixel, with the game's line edges), fades out as the camera zooms out as it does in the game, and lies under the zones, the Red Death bloom and the dark ring round the world, so it fades out with the dark ring instead of showing on top of it (the dark ring now also covers the pellets and the bibites that are beyond it, as in the game). And, also since the alpha of 2026-11-19: Settings has a new page, Calibration (a check box, Use Calibration, on by default, and a Run Test button): it times the simulation on your machine for about half a minute (start it from the title screen, or pause the world: it will not start while one runs) and keeps what it found in userdata\calibration.txt, which the next start uses: how many of your threads a tick uses, from what size of world sharing the work pays, and whether the graphics card or the processor draws the bibites' colours; it changes how FAST the simulation runs and nothing else (the simulation is the same whatever it finds), and a machine that never runs it gets the old first guess; and the title menu's screens follow the shape of the window the way the game's own layout does (the settings pages grow with the window and lose their scroll bar when it is taller, the New Game, Load Game and Library lists take the room the window has, the Credits and Additional Tools tabs, the scroll bars, the mouse wheel and the New Game screen's ticks-per-second pop-up follow; checked against the original game's own layout at 2560 x 1080 and 1920 x 1200 and in pictures; before, those screens were drawn as at 1920 x 1080 and sat in the top left of a bigger window). Because of these, a world started from the same seed no longer plays out as it did in earlier builds. The speed work since the alpha of 2026-11-21 changes no result: big worlds (thousands of bibites, tens of thousands of pellets) tick a few percent faster, because the pellet decay, the bibites' eating turns and the physics step's search for who touches whom use more of the processor, and the same seed plays out exactly as it did in that alpha (checked bit for bit against that alpha's simulation on several worlds). Worlds of pellets run much faster since the work of 2026-11-23, with the same result: pellets that rest are left out of the physics step in every world of 64 pellets or more (it was 2,048), pellets that touch each other and only turn by rounding noise rest too, nothing is looked at for pellet decay while no pellet decays, and the command line reads only the bibite templates a scenario names; scenarios of a few hundred to thousands of pellets tick 2 to 20 times faster, and the top time speed of 100x is reached in them (it stopped at about 70x to 90x). The only thing that differs is the turn of a resting pellet in a checkpoint file, by under 1e-5 radians; worlds saved by the old and the new build are identical. Since the alpha of 2026-11-22 one more bug is fixed: in a long run a world could grow dense clumps of pellets that threw and killed the bibites that touched them, and were quiet again after a reload (the "deadly pile" save of 2026-11-22). Bibites that grab other bibites and pellets form groups of rigid joints; in a group that is not a simple chain (a bibite that holds a bibite that holds a pellet, a pellet in two mouths) the physics step kept contacts between the group's own members and solved the joints once instead of in every iteration as the game's physics does, so the velocities of the group wound up tick after tick until it threw what it touched. Bodies that a joint ties together, or that one holder holds, no longer push each other (the game's joints do let the pair collide; the port has kept those contacts out of its solver since the glitch of 2026-11-20, because they wound it up), and the joints are solved in every velocity iteration; a world started from the same seed plays out differently from then on wherever such a group forms. A second way for a pile to turn dangerous is not a bug: a big bibite with a strong jaw that grabs and then throws pellets (the game's throw: every held body gets an impulse, the thrower gets the whole force back as a recoil) flings them, and itself, at the speed limit, and what that hits is hurt; the replays of that save show it too. Since then, from four tester reports of 2026-11-23: the "Removing half of all pellets" pop-up (and "Killing half the population", "Delete All Zones" and the other choice pop-ups) can be clicked away in a window that is not 16:9 (it was drawn in the middle of the window but its Cancel and YES buttons answered where a 16:9 window has them; the further the window's shape from 16:9, and the smaller the window, the further apart); the check boxes of the Settings pages answer on their box and not on a 350-unit strip round it (Fullscreen, Automatically Detect Screen Resolution, Show Grid and the others: the strip ran under the gear beside "Show Grid" and "Show Experimental Background" and under the folder button beside "Include autosaves" in the Load list, so a click on those toggled the box; the gears open the grid and background option panels in the game and do nothing here yet); the sliders of the Settings pages keep their handle on the bar in any window shape (the handle was placed from the 1920 x 1080 picture and slid off the bar when the window was stretched); a scenario whose settings animators start at a negative time starts (the published Seasonal Desert v3 starts its winter animators at -43,200, which the game takes as a phase shift: the curve is already that far along at the first tick; the program refused to build the world and the tester's Start went back to the main menu); and a running world can be saved as a scenario (the Save button of the in-game settings panel opens the scenario saver, the game's own form: a name, a description and the picture of the world; it writes Scenarios\NAME.zip with the world's settings, zones, zone groups, default bibites and animators and the templates they use, asks before it overwrites a scenario, refuses the name of a shipped scenario and pauses the world while it is open). Two more reports the same day: the Ancestral Lineage Graph is drawn to fill a window of any shape (it was drawn in a 1920 x 1080 corner: in a wider window grey stood beside the graph while the buttons and the title sat in the middle of the window, in a taller one grey stood under the graph, and only that corner answered the wheel and the right button; the graph, the buttons on the right, the title and the panel's Settings now follow the window the way the game's panel does, and the tree and the mesh are placed again when the window changes shape); and the time speed no longer collapses when the window is slow to draw ("at 4K it does not let you go fast, but as soon as the window is smaller it goes super fast": the minimum-FPS rule took the time scale down in proportion to the frame rate whenever the frames were slower than about 13 a second, as the game does where its ticks and its drawing take turns, although here the ticks run beside the drawing and a slow display does not hold them up; with a model of a slow display (RUSTIBITES_FRAME_PAD_MS=120, 8 frames a second) a time speed of 3 gave x0.09 and a time speed of 100, the fastest the machine can go, gave x0.09 as well, and now they give x3.0 and x16, what a fast display gives; the rule still slows the simulation when its ticks are what hold the frames up, and RUSTIBITES_LEGACY_PACING=1 brings the old rule back to compare with). Still open or not reproduced: the in-world settings panel and the scenario editor keep a 1920 x 1080 composition in a window that is not 16:9 (the in-world panel does not grow taller in a taller window; the title menu's other screens follow the game's layout since 2026-11-21, but that was checked in pictures and against the game at two window shapes and has not been driven by hand yet: a screen that looks wrong, or a scroll bar that is missing or in the wrong place, in a window of another shape or at a UI scale above Normal is worth a picture and the window's size; a text that wraps differently in a wider window keeps its old line breaks), slow motion runs one physics step per tick where the game runs 1/speed, the title screen's buttons reportedly did not click in fullscreen on a 2560 x 1440 monitor (the menu fix above may be the cause; if not, run with `RUSTIBITES_INPUT_LOG=1` from a terminal and send the lines it prints for each click), a report of heavy disk reads while a simulation runs (not reproduced: nothing in the simulation or the frame loop touches the disk; if it happens, Resource Monitor's Disk tab names the process and the files), the mesh view's pan when a shape is clicked, the tree at its zoomed-out fit level is half a pixel off the game's, zones that look as if they reach beyond their green (the radius, the spawn positions and the zone shader match the game's code, and the green fades out long before the radius as in the game: send the scenario or a picture with the zone's settings), the "portrait outline over the pellet" (not found; probably the editor or library preview: a picture and the screen it is on would help), the species window missing from a Print Screen picture (reported 2026-11-13, not reproduced: the Lineage panel's species window is in an operating-system capture of the real window and in the F12 picture, windowed and fullscreen, on the development machine; which key or tool took the picture, windowed or fullscreen, the Windows display scale and the picture that came out would help), and a tester's crash on a 3.9 GB RAM / GTX 1050 Ti / Windows 10 machine (2026-11-13, alpha c8c2488: exit code 0xC0000005 in the first frames of a world opened from the title menu; not reproduced; the Make-Bug-Report.bat zip of this build names the failing module in crash_events.txt: send it, and say whether `Play.bat -ViewerArgs --no-gpu-hue-shader` or `--no-overlap` stops it). One report says the animator creator's OK button did nothing after the Information panel had been dismissed (not reproduced: the Information panel, which covered the creator's right half, now goes away while the settings panel is open, and the whole flow creates the animator in the developer's tests; if OK still does nothing, send the window size, the Windows display scale, the preset you chose and the order of your clicks, with the logs\last_run.log of that run). The sliders of the zones, the materials, the default bibites, the towers, the pellet placer, the genes and the brain editor still jump to the pointer when pressed on their handle (the game keeps the handle where it is until the pointer moves). A slider let go OUTSIDE the window (over its title bar or beyond a side) stays held until the next click: the window library takes no mouse capture, so the release never arrives; let go of it inside the window. A bibite with black pigment is that dark in the original too (body core 14, 14, 14 in both; its outline, arms and tail stay mid to light grey there and here), so that one is not a gap. One drawn black ALL OVER (arms and outline too, only the eyes coloured) was either a window that did not run the graphics card's colour program (the line that starts with "renderer:" (or "warning: GPU hue material failed") in logs\last_run.log says why; this build recolours the sprites on the processor instead) or, found on 2026-11-16, the colour program itself on a colour of exactly (0, 0, 0) (all three pigments at exactly 1): fixed; send the "renderer:" line if it still happens. A sim that "slows down as it ages" is the population growing (the cost per bibite per tick stayed flat over 140,000 ticks in the world that was run for it, and over 120,000 ticks in ONE process without a reload: 1,400 to 2,000 bibites, the memory following the population; a reload of the same world makes the simulation about 10 % faster, not several times, because the bibites are in memory order again). A sim that runs several times faster after a restart most likely ran an older build: before 2026-11-13 a frame drew serially and the blood and pheromone particles did not age in a frame of more than a second of game time (not measured on that world). If it is slow again on this build, start the game from a terminal with RUSTIBITES_LOG_FRAMES=1 and send the last lines it prints (where the frame's time goes). A world where it does not is wanted (the save, or its scenario). Senses are not bit for bit everywhere yet: in a tick-by-tick comparison with the original game, all the field-of-view inputs are exact in about 70 % of the brain ticks (about 85 % once the rounding of the rotation a save holds is allowed for), and a bibite's energy, fullness, speed and rotation-speed inputs differ slightly in some ticks (a save file does not hold everything the game senses from, such as a body's angular velocity, so those could not be compared exactly); in about one tick in eight a closeness differs in its last digits for a reason that was not found. From the tester report of 2026-11-24: the stomach picture of a bibite that has died stands still, as the game's panel does (it went on digesting from the last rate the stomach had and brought a pellet in from the right every few seconds, which looked like a bibite that kept eating after it died; the stomach itself did not change). Since then: the scenario editor's Animation tab makes settings animators (its + opened a creator panel that was never drawn, so no animator could be made on that page; the world's own panel was fine): the creator hangs from the tab's header, its preset list, its sliders, Cancel and OK work, OK adds the animator to the scenario that Start runs and Save writes, and another tab closes it (where it hangs was worked out from the world's panel, it was not measured in the game's own editor). The streams, the force towers and the whirlpool towers cost far less a tick (thirty of them over a world of 55,000 pellets went from 12.7 to 1.3 ms a tick, and over 20,000 bibites on six threads they added under 1 ms where they added 4.5); what they do is exactly what it was.

## Evidence that exists (re-run this session)
- Ring FOV capture vs the game: 9000/9000 brain outputs bit-exact over 15 nodes (unchanged by all fixes).
- Default official captures (600 / 3000 ticks): brain outputs 7233/9000 and 36033/45000 exact, energy and position error unchanged.
- Default fresh scenario, 36,000 ticks, final population over seeds 1-12: 133 (before) -> about 190-200 (now); game's single sample is 19
  (mean here about 16); one species on every seed, as in the game.
- Determinism across thread counts and bit-exact checkpoint resume re-verified.
- `bibites-core`, `bibites-io` and `bibites-cli` test suites pass (615 tests). `bibites-view`: `cargo check --tests` only.

## Viewer batch of 2026-11-06 evening (done; each checked headless, the GPU/window parts in the real window)
Hover delay 0.3 s on the Biology pop-ups, Flow pop-up, blood particles, Corrupted Save popup (Save panel too), fullscreen start, pellet collision/merge
toggles, Save sprite, Escape-stack zombie, bundled templates, egg rotation and GLOW (the game's `UIEggShader` translated from its DXBC with
`tools/unity/dxbc_to_rust_tex.py`, which also reads textures, `discard`, log/exp, several constant buffers and the integer sign trick), the value
field's caret (Left / Right / Home / End / Delete / Ctrl + A / Shift), elastic wheel + row masks on the Save list and the elastic title-menu lists,
stomach pellets that spin (friction), UI Scale / Screen Resolution (see "Settings screens"), the skin shader's hurt flash, reaper darkening and
virus / rainbow / producer patterns (`skin_pixel` in `internals_shaders.rs` is the game's whole proceduralBibiteShader pixel program; the window draws
bibites that have a pattern from a picture the software rasteriser makes of the sprite, the others with the GLSL material; `RUSTIBITES_FORCE_SKIN=
rad,rain,source` puts the patterns on every bibite to look at them), grab joints that break at the game's `breakForce`, collisions that kill call `Die`
at once, species span events + `CheckSpeciesOfEgg`, save-version verdict. Found only by the real-window pass: the clipped Save list drew upside down.
Added afterwards: the Save list and the title-menu lists drag with the pointer (rubber band, inertia; a still press is the click, row actions wait for the release),
held bibites are thrown on a strong negative grab, and every text field has a real caret (`text_field::edit`: tags, zone name, species name, the template editor's
name and description, the save dialog, the brain editor's label and filter; species NOTES still type at the end). The experimental background works
(`Show Experimental Background` / the new `Texture Color` setting): the game's whole background pixel program (`bg_pixel`, with loops and the integer hash) runs
in software with the grid switched off, in the window at a quarter of the resolution (blocky, ~10 ms). Its camera globals are GUESSED from the program
(`cb0[21]` the camera's place, `cb0[25]` the orthographic half sizes, `cb0[19]` the time); the cells look right but their parallax speed is unverified.

## Simulation (core) - largest remaining items
1. Challenge runtime: a challenge started from the New Game screen now runs (see "Challenges" below: champion pick, the champion's row in the
   world, the session with the scenario's texts, scoring, stars, stored best, restart, the tracker and end popup of the game's prefabs, the hidden controls
   and the Challenge tab).
2. Collision damage: `BibitePart.OnCollisionEnter2D` uses `other.otherRigidbody.mass`, which is the bibite's OWN body, so the mass term is 1 (it is
   what the code has; the earlier "mass term" worry was a misreading). The contact point (`GetContact(0)`) is "the first event per pair". A collision that
   kills now calls `Die` at once, as `Hurting` does (a second hit of the same step meets the corpse); starvation deaths are still resolved at the start
   of the next tick.
3. Grab joints: a grabbed bibite is carried at the holder's mouth velocity and the joint breaks (`OnLinkBroke`) when the force that carries it
   (`mass * dv / dt`, the pull of what the step did to it) passes the game's `breakForce = jawStrength * max(diet, 0.1) * desireToGrab / nHeld`. It is
   an estimate of the solver's reaction force, not Box2D's; releasing a held bibite with a strong negative grab throws it and recoils the holder (as pellets always did).
4. Death timing: starvation deaths and organ-side `use_energy` deaths still resolve one tick late (collisions no longer, see 2).
5. Species: the used span now follows the game's events (`UpdateEffectiveSpeciesSpan` runs when a species is created or found from a template, not
   when `LogSpeciesInfo` drops the empty ones, so it stays stale until the next creation), a loaded egg takes `CheckSpeciesOfEgg`'s species, and
   `SpeciesRegistry::find_species_from_template` exists with its tests; nothing calls it yet because spawn rows do not carry a species name.
   (The presence-array semantics `nPointsPresent` / apparition / disappearance, the game's `SpeciesTreeCleanup` with its second stage and the
   `data.bin` species block are done, see "Species history".)
6. `isRad` bibites now carry a `RadioZone` (radius 250 = collider 0.5 x scale 500, read from `sharedassets2.assets` with
   `tools/unity/`): bibites overlapping it get the permanent +10 / +0.35 mutation factor (idempotent; the flag is not checkpointed, it is
   re-detected on contact). Still missing: ColorKiller/Pherosense use centres instead of colliders. A pheromone tower's spot now follows
   edits of its strengths / heading, and a new colour killer takes the prefab's values (hue 0, tolerance 0.1, radius 400, period 100).
7. Save/load: challenge parameters, `data.bin` regeneration. (Parent / child links are written and read: `bibite_ids` are `index + 1` in saves this port
   writes and the game's own ids are kept on a load-save round trip; persistent ids inside the core records are not needed for that. Gene and brain
   migrations exist (`migrate.rs`); the save-level verdict of `VersionTracker` is wired: older than 0.4 is refused with the game's error popup, older
   than `0.6a15` loads with a warning line. This port's floor is still 0.5, so 0.4.x saves are refused with the popup although the game updates them.) (Saves now bundle the templates their spawn rows use and loading puts the missing ones in the player's folders, as `AddTemplatesToArchive` / `CheckTemplatesOfArchive`; the game's "you did not posses" dialog is only a line in the status text.) (Zone-group zones now load in the game's `allZones` order and saves stamp the game version `0.6.3.1`.)
8. Egg-spawn `Mutate(n)`, a few brain-mutation counter quirks, newborn `underfed birth` branch, held-drag toggle, ripped chunks not recycled by zones.
   (Spawn rows with `growthAtSpawn = Egg` place eggs; the `Revival` coroutine is ported: a revived corpse's health follows `1 - decay / 1.1` in 0.1 s steps while the decay falls by 1/150; the microscope shade outside the world is drawn: black, alpha `saturate((|p| - 1.5 x size) / 1000)`, `Add Shaded Bounds`.)
9. Physics is a custom solver (intentional, `DEVIATIONS.md`); trajectories are not a parity target.

## Viewer / UI
1. Rendering: target-weight labels (*Show Targets*: the View panel's switch and the setting do nothing yet; the filled view cone and the status
   bars are done), microscope shade outside the world, arm/mouth animation
   (the pheromone particles are done, see "Pheromones" below), per-bibite shader effects (rainbow, virus, producer, reaper, hurt flash). Visual-axis items that need shader
   internals are not decompiled and cannot be matched exactly.
2. Native inspector: Stats/Genes/Biology/Brain header values are now live; the Brain node graph values, Biology flow/slider values and
   egg stats are still frozen captures.
3. Done since this list was written (each in its own section or paragraph): the Bibite Editor (Genes and Brain tabs, every entry point; in "UI mechanics"),
   the Expanded Brain View, the lineage tree and mesh, the species Genes, Brain and Position tabs, the Graphs panel with its hover indicator and Gene
   Analysis page, the achievements. The editor's Save sprite button writes the game's sprite sheet (`sprite_export.rs`: body, mouth, the arm twice, exoskeleton and eyes laid out as `BuildAndSaveSprite` does, magnified 10 times) as `bibite_yyyyMMddHHmmss.png` in the data folder (the game opens a system save dialog; the `SpriteGeneratorSavePath` setting and the Output Pixel Ratio setting are not read). Not done: real-mouse checks of the Brain editor's editing.
4. Input: see "UI mechanics" below (K, Z and the tooltips are done).
5. Settings: the viewer has an invented `UserPreferences` model driven by label substring matching; ~25 real user settings are absent;
   dynamic-parameter controls bind by fuzzy label; value labels are not refreshed after edits; hotkey rebinding is an invention.
6. Several panels that were added (Graphs, Information, pause menu) are drawn over both UI paths (the Information panel follows the game's prefab since
   2026-11-05, see "Information panel"), but the curated Stats/Genes/Biology rows,
   species buttons only exist in the fallback path used when the native capture assets are missing.

## Investigated and deliberately not done (with the reason)
- **Death timing (core item 4):** the game calls `Die()` inside `Hurting` (immediately); the core queues kills and resolves them at the start
  of the next tick. The difference is one tick (~20 ms) and changing it would perturb the bit-exact capture comparisons for almost no
  behavioural gain. Left as is until an ecology measurement shows it matters.
- **Pellet collision / merge toggles (done 2026-11-06):** `pelletCollision` off makes a pellet's collider a trigger with frozen rotation (`MatterPellet.ToggleCollision`; the mouth still finds it through its sensor, which is this port's reading of a trigger-vs-trigger overlap: untested against the game, where Unity 2D may not report one), `pelletMerge` on lets two touching pellets of one material merge (the not-smaller one takes the amount and the momentum-weighted velocity, `OnCollisionEnter2D`; with equal amounts the lower tag wins). Defaults (collision on, merge off) are unchanged, so the bit-exact comparisons are too. The pellet rotation setting (`UpdatePelletRotation`) is not wired.
- **K (auto-panner) hotkey** (`autopanner.rs`, `UserControl.Update`, `Utility.AutoPanner`): `SelectTarget(autoPanner)` makes the camera follow a rigid
  body that starts at the origin and drifts between random zone positions: force 10, max speed 5 (the scene overrides the script's 1), mass 10,
  linear damping 0.05 (read from level2), a new destination when it is within 10 units or every 30-90 s of game time, stepped with the simulation
  ticks (it stands still while paused) with a Box2D-style integration. The destination is a random zone's own spawn location, drawn on a copy of
  the zone with a random stream of the viewer's own, so the simulation is untouched. K is blocked like the other selection keys (placers,
  dialogs, text fields); Escape deselects. The project's 2D gravity is zero (`Physics2DSettings`, checked). Z (random zone) is done too, see "In-world zone editor".
- **Bibite ids / parent-child links in saves:** needs a persistent per-bibite id and children list in the core records and the checkpoint
  format (format version bump). Large and cross-cutting; saves currently use `index` ids and empty children lists.
- **Everything that used to be under "Not implemented at all" in the Viewer section** (brain/gene editors, lineage tree, species actions,
  phero-tower tool...) was large UI work held back because viewer tests were off-limits on this machine; the viewer's tests now build and run with
  the light profile (see "UI mechanics"), and all of it was built and tested that way (each has its own section).

## Title menu (reworked after "I can't even start a new game")
Cause: the menu looked for `testdata/real/Default.zip`, `saves/` and the settings file relative to the working directory, so outside the
repo root it listed no scenarios and could not start; the game's own folder (`%USERPROFILE%/AppData/LocalLow/The Bibites/The Bibites`,
the old `LOCALAPPDATA` guess was wrong) was never read. Now the menu reads `Scenarios/`, `Savefiles/` and `Savefiles/Autosaves/` there
(plus the viewer's own data folder), and every screen replays the captured native layout with live data:
- **New**: scenario/challenge list (official first, by rank), description, preview image, star text, modifier dropdowns derived from the
  scenario's own settings (only modifiers you change override it), Start Simulation, Return, challenge-list toggle, open-folder button.
- **Load**: newest-first save list (+ autosaves toggle), name/version/last edit/sim time/size/bibites/pellets, preview, Load, delete with the
  game's confirmation. **Continue** loads the newest save including autosaves (`GameManager.Continue`).
- **Settings**: Graphics / Game / Hotkeys pages bound to the game's user settings (labels, defaults and ranges from `UserSettings.cs`),
  persisted to `game_user_settings.json`; autosave/pause-after-load/grid/node-names/min-fps/pheromone-strength are applied to new worlds (the minimum FPS also drives the in-world governor, `TimeController.CheckMinFPS`: `time_keeper::min_fps_scale`).
- **Library**: real template list, name/description, Select Template starts a world with it. **Tools**: the wiki and Discord links open `URLManager`'s
  pages (on the icon or the title: what is a graphic under the `Link`'s button; the Patreon simulation and the sprite editor are other scenes of the game and say
  they are not available), the tab scrolls (wheel 35, the live bar: drag and page) under its mask, its paragraphs wrapped. **Credits**: opens 622.96 down, as the
  prefab stores it (the team and the thanks), scrolls the same way (the social buttons and the Patreon sentence are above), and its five social buttons open
  `URLManager`'s pages (Patreon, YouTube, Twitter, BlueSky, Reddit: the layout's order is not the order of the buttons' names). Each tab keeps its place while the
  title scene lives. **Achievements**: static replay. **Loading**: the game's loading panel.
- Verified headlessly with `--menu --menu-view new|load|settings|library|tools|credits|achievements --menu-click X,Y --screenshot`.
**Player data now comes from the installed game.** The game keeps its user settings, challenge results, achievements and warnings in Unity
`PlayerPrefs` (Windows registry `HKCU\Software\The Bibites\The Bibites`; floats are 8-byte doubles). `bibites_io::prefs::GamePrefs` reads it
read-only (PowerShell child, ~0.2 s, prefetched at start; `RUSTIBITES_IGNORE_GAME_PREFS=1` disables it for deterministic runs). Effects:
the Settings screens show the player's real values, the zone/background colours and grid follow `FertilityColor`/`Background`/`ShowGrid`
(the orange world of the reference captures *is* this player's setting; a fresh game is green on dark blue), challenge bests (and the
"NEW HIGH SCORE" rule) include the game's stored results, and the Achievements icons follow the stored unlock flags (and this port's own unlocks, see "Achievements"). Nothing is written to the game's registry;
changes made here persist to `game_user_settings.json` (format 2: only changed values).
New Game TPS: the game resets the TPS panel to `UserSettings.simTPSDefault` etc. (40 / 2 / 2 / 10 / 5) every time the list loads, official 0.6
scenarios do not define these settings, so a menu-started Default runs at 40 TPS (not the core's 30); scenario files that do define them win.
The "TPS settings" popup is implemented (5 log sliders, reset, "Save as Default", Esc closes it).
Bibite Library: list = `Bibites/Templates/*.bb8template` then `Bibites/*.bb8` in file-name order (names may repeat, as in the game), hidden-node
and synapse counts as `BibiteTemplateGenePreviewer`, first item selected on open. The side buttons work as in `BibiteTemplateSelectorPanel`: the
Templates / Saved tabs show the `.bb8template` / `.bb8` files under their own titles ("Template Bibites" / "Saved Bibites"; the selection stays when
the tab changes, `OpenTab`), Reload re-reads the folders and selects the first bibite (`FillLists`), the folder button opens `Bibites` in the explorer
(`OpenSaveFolder`), and Delete (enabled only for a selected bibite that is not official) asks with the `fileDeletionWarning` popup (do-not-show-again
checkbox, Cancel remembers nothing; once switched off the file goes at once - "Reset Warnings" does not bring this one back, it is not in
`allInfoShown`), deletes the file and selects the first bibite of the same list; a file that cannot be deleted shows a status line where the game shows an
error popup. A disabled Delete button gets the Button's default `disabledColor` tint (0.784 grey, 0.502 alpha; its colour block was read from the scene).
Esc closes a warning popup first (as its Cancel). The Library's submit button is the game's "Edit Bibite"
(`EditSelectedBibite`: the Bibite Editor opens over the Library, see the Bibite Editor paragraphs of "UI mechanics"; "Select Bibite" while a template is being picked for the placer /
the default bibites); the stand-in that started a world with the template is gone.
Still missing in the menu: the scenario editor is partial (see "Scenario editor" below), challenge editing, champion picker for challenges
(the Library's template preview picture is done, see "Procedural bibite previews"), the Library's workshop buttons (no Steam), importing
saves, achievement tooltips (the unlocks of the game's flags and of this port's `achievements.json` are shown), hotkey rebinding screen is static,
screen resolution / UI scale are applied since 2026-11-06 (see below, as is Fullscreen).

## Found with real input (2026-11-04)
Real mouse/keyboard in the window found bugs that headless screenshots and type-checking could not: one wheel notch slammed the zoom to its limit
(miniquad reports the raw Windows delta, 120 per notch; now normalised to notches), a new game opened framed on its single bibite instead of the
game's own camera (origin, orthographic size 400, from the scene's Main Camera), SendKeys is ignored by miniquad (scancodes are needed), the
selected bibite's view cone was drawn as dots (now a translucent `FieldOfViewMat` sector, `_Color` (0.14, 1, 0, 0.114), plus white border), and
Esc opened an invented "PAUSED" panel (now the game's `EscapePanel`: Save Game, Load Game, Settings, Bibite Library, Wiki, Back to Menu,
Exit To Desktop; Back to Menu returns to the title menu). The captured Escape/TPS panels were inactive, so their geometry is reconstructed.
In-game Settings (gear), Library, Achievements, Tools, Credits and Load Game (Escape panel) now reuse the title-menu screens (live data, software text,
dimmed world behind); the pen opens only the dynamic-parameters panel (the gear used to open both, and that panel replayed a whole cumulative capture).
Verified by hand: pause (Space), wheel zoom, right-drag pan, click-select, inspector tabs, Esc panel, Load/New flows.
**2026-11-06, real window with the player's own folders:** pause menu > Load Game froze the window for 4.8 s (and every F5 and every delete did the same):
`App::refresh_save_entries` opened every zip of the save folder to read its version / population / time for the fallback browser's rows, and the game's own
`Savefiles/Autosaves` holds 1,382 files (3.4 GB) with *Include autosaves* on. It now reads that metadata for the first 12 rows only
(`list_save_entries_head`); the Load screen opens in under a second. (The title menu's own list only stats the files.) The menu's save dates no longer start a
PowerShell of their own for the UTC offset (the game-preferences reader, started at launch, has it). A real-window run of the Save Game panel and of the Load
Game screen (rows, wheel, click, Escape back to the pause menu) is in this session's notes: the Save run wrote one test file into the real `Savefiles`, which
was removed by its exact path.

## Dynamic settings panel (pen button) - regenerated, live
The first tab was a frozen capture and the other tabs did not exist. `dyn_panel.rs` now rebuilds the panel from the game's own declaration
(`MenuDynamicSettingsManager`): the captured chrome (frame, tab buttons, scroll bar) and captured row prefabs (toggle, slider, master-toggle
group) are cloned and repositioned with live labels/values from `world.settings` (`tools/extract_setting_ui.py` extracts labels, units,
precision, SI and factor of all 92 settings into `setting_ui_data.rs`). Working: Simulation / Zones (parameters only) / Bibite / Energy
(Bibite Balance + affinity sliders) tabs, toggles, linear/int/log sliders with drag, default pointers, reset buttons, expandable sub-panels
(Pellet / Shade / Red Death / Void / Limit Bibite Birth), fixed-sum mutation groups, wheel scrolling (row-level clipping), tab tint.
The Zones tab has the game's zone list and the Energy tab its four Materials Parameters blocks (Plant / Meat / Fat / Armor: six sliders, the decay
switch with its two sliders, reset pointers and "reset all", all applied live through `World::apply_settings`) since 2026-11-04 (below). Not done: the Animation tab
(settings changers), "Save Scenario" (done 2026-11-23: see the open items), and changing TPS at runtime (the core rejects simTPS/SimulationSize changes: "requires a new world"); the value
edit fields are done for this panel only (see "Typed values" below).

## UI mechanics (priority since 2026-11-04: what the controls DO first, looks second)
Method: `tools/unity/dump_ui_wiring.py` extracts every serialized UnityEvent call of a scene (level2 = world, level1 = menu; 374 + 235 hosts), so
a control's real callback is known without guessing from names. Findings were checked against `Here/*.cs`; changes are covered by unit tests that
were run (`cargo test --release -j 2 -p bibites-view --bins` with the light profile, `app::` and the `ui_runtime::` ones one thread at a time).
Only the window-only parts (clipboard, typed characters, real pointer hover) were type-checked and not exercised in a real window.

Done (each from the C# named):
- **Inspector** (`GUIManager.OpenBibitePanel` / `curMenu`): selecting a bibite shows only the four tab buttons and (with *Show Status Bars*, off by
  default) the bars; a panel opens with its button or digit 1-4 (egg: 3 = Brain, 4 nothing) and the open one closes again; the choice survives
  deselecting. The digits no longer set the speed.
- **Escape stack** (`UINavigationManager`): panels, the target, the inspector panel and the selection panel are popped newest first; only an empty
  stack opens the pause menu; a panel's close button never deselects. A species-tag edit takes Escape first.
- **The pause menu stays open under what it opens** (found in the real window: the editor reached through pause menu > Bibite Library > Edit Bibite had
  the menu drawn over it, whose click catcher took every press meant for the editor): the wiring of its buttons is only `UserSettingsManager.OpenPanel`,
  `SaveGamePanel.OpenPanel`, `LoadGamePanel.OpenPanel` and `BibiteSelector.OpenWithoutGoals`, and `BibiteTemplateSelectorPanel.panelToHide` is not set in
  either scene, so nothing closes `EscapePanel`: those panels cover it (`TopLayer`, and the Library after it in `Layer2`) and Escape / their close button
  shows it again; the next Escape closes the menu and gives the pause back. The port used to close the menu when Settings / Save / Load opened (and to
  draw it over the Library's editor); now it is drawn, and takes the clicks, only while it is the newest panel. `SaveGamePanel.Save` ends with
  `ClosePanel` (the port's Save panel closed only by its X); a load rebuilds the scene (`OnSceneChange` empties the stack), so a load issued from the menu
  ends the menu and its pause (the world runs unless *Pause After Load*). *Pause After Load* itself was only applied to loads made inside a running world;
  a save started from the title menu (Continue, Load Game, also the world's Load Game screen, which rebuilds the app) now starts paused when it is on
  (`SimulationManager.Start`: only `loadedGame`, not a new scenario; the menu remembers which it is loading). Checked by unit tests of the menu's flag and
  of the app's stack; the window-loop line itself (`main.rs`, after `App::new`) is type-checked only.
- **The Save Game panel** is the prefab's (`save_panel.rs`; level2 `Save Game Panel`; it was an invented LOAD / SAVE browser in which a name could not even be
  typed). Opening it (`SaveGamePanel.OpenPanel`) reads the world (the time as hh:mm:ss, the size as the prefab's `FloatValueTextHandle` writes it - "2000u", no
  SI - bibites + eggs, plant + meat pellets) and takes the 400 x 400 picture of the whole world (`SceneScreenShotHandler`: the simulation square from above,
  zones and pellets, no grid, with the software rasteriser), lists `Savefiles/*.zip` newest first (no autosaves: `GetSaves()`) in `SaveItem` rows (50 high,
  `base_icon`, the name Roboto-Bold 25, the date 20 and 200 wide), and starts with an empty name and a disabled Save button. A row puts its file's name in
  the field (`OnSaveNameChanged`); the field (a click selects all - `onFocusSelectAll`: the next character replaces the text - Backspace, Ctrl + V, Enter ends
  the editing, a click elsewhere or on a button ends it too) enables the button when the name is not empty and holds none of
  `Path.GetInvalidFileNameChars()`, and the bug icon shows when `<name>.zip` exists. Save asks first then (`DisplayChoiceDialog("Are you SURE???", ...)`),
  writes the world AND the picture and closes the panel (the pause menu shows again); Delete asks (`deleteSaveFileActionWarning`, "do not show again"
  honoured), removes the file and selects the row that takes its place (its name goes into the field; with no row left the field keeps its text; with no
  row selected the game's `DeleteSelectedSave` has nothing to delete and nothing happens); the X and Escape close it. While it is open the controls are
  blocked (`AllowControl = false`: no Space, no hotkeys) and the wheel scrolls the list by the prefab's `ScrollRect` sensitivity (50 per notch; a scrollbar
  from 16 rows). Geometry is the prefab's, solved (the two columns' widths are the two panels' sprites' minimum sizes plus half of what is left, so they only
  depend on the screen's width; the picture's square is as high as what the 253.92 high list of lines and the 100 high button leave: 491.08 at 1080) and
  checked against the stored sizes by a test. **Every save the port writes now carries a fresh `img.png`** (the Save panel, F5 and the autosaves: `SaveSystem`
  writes `SceneScreenShotHandler`'s picture into every save); they used to carry the picture of the save that had been loaded, or none, so the game's own
  Load screen showed a wrong or empty preview for them (`WorldContents.img_png`). Checked by 14 tests and headless captures (`--save-panel -|row:N|NAME
  --saves-dir ROOT-WITH-Savefiles`). The name field has a real caret (`text_field.rs`: Left / Right / Home / End / Delete / Backspace, Shift extends, Ctrl + A, a click on the focused field places it, a drag selects; the slider edit fields (`value_edit.rs`) and the other text fields still have the caret at the end). *Not matched:* the Save panel's list masks the rows at its edges (`UiDraw::Clip`: a clip rectangle in the software renderer; in the window it is a camera whose viewport is the rectangle, not tried in a real window) and its wheel overshoots the ends and springs back (`SmoothDamp`, elasticity 0.1, Unity's default: the prefab's value was not read; no inertia / drag of the content; the title menu's Load list is not masked or elastic), the title menu's Load Game list raises the game's "Corrupted Save" error popup (`error_popup.rs`, the `ErrorPopup` prefab: red "A Problem Occured", the source, the text, OK; one per unreadable save, on opening the list or ticking Include autosaves; geometry from the prefab's values, not captured, only unit-tested) and so does the in-world Save panel (1.35 scale),and the save's progress popup is not shown. The list's scrollbar is the game's
  (`scrollbar.rs`, `VBar`: a track and a handle as wide as the bar, the handle as long as the share of the content the view shows, `Scrollbar.size = view /
  content`, reaching half a width over each end of its sliding area; from 16 rows; the viewport gives up the bar's width): a press on the handle takes hold of it
  and the held button drags, a press on the track pages toward the pointer. The title menu's three long lists have one each, with their prefabs' sprites and
  numbers read from the captures (`MenuState::list_bar`): the Load Game list's (20 wide, `Background` / `UISprite`), the scenario list's (16 wide at (733.2, 108),
  946 high, `base_icon_thin` at multiplier 0.5, the track grey at 0.58 alpha) and the Library's (12 wide at (1114.8, 91), the dark `scroll` track and the `base_icon`
  handle); the Library has 221 templates in the game's own folder, which the wheel alone (100 a notch) crossed in 125 notches. The Settings pages have the live bar
  too (the capture's still picture of it is replaced: `MenuRule::ListBar`; 20 wide at (1254.3, 80), 903 high, in a view 920 high): their wheel limit was one invented
  520 for all three pages, it is now what the captures measured for each (content 1035 / 1415 / 1384.8: 115, 495 and 464.8 to scroll; the Graphics page used to scroll
  into blank space). The wheel and the bars share their extent (the old scenario-list formula also counted the number of saves).
  The title menu's Load Game list is made of the same `SaveItem` rows now (`save_panel::draw_row`: it used the scenario list's 62 high rows with the scenario
  script's light blue selection, which `SaveItemReference` has not: every button of these panels has Unity's default colours), starting under the "Include
  autosaves" line (which does not scroll with the rows here; it loses the scrollbar's 17 on its right) and scrolling 55 a row, with the same draggable scrollbar
  (with the 1,382 autosaves of the game's own folder it is the way to move through the list: a wheel notch is 100 of the 77,000); and a wheel notch moves a title-menu list by the `scrollSensitivity` of its
  prefab's `ScrollRect` (read from the captures: 100 for the scenario, save and template lists and the settings pages, 35 for the Tools and Credits tabs; it was
  70 everywhere).
  The Credits and Additional Tools tabs (the title menu's screens, which the world's panels reuse) were a still picture of the capture (the Credits one taken at the scroll the prefab stores,
  622.96 of 631.57: the team above the view was drawn over the title, nothing scrolled and the social buttons could not be reached; the Tools paragraphs ran off
  the screen on one line). They scroll now like the Settings pages (`panel_scroll`: the nodes move by the difference to the captured scroll; what the mask would
  cut is left out, a text line by line: a text cannot be cut), with the live bar (`Style::TABS`, 20 wide at (1875, 100), 955 high; contents 1586.57 and 1028.32),
  paragraphs wrapped at their boxes (the capture's box height says how many lines the game made: a measure a hair wide does not add one), TMP's `<3` kept as text
  (`strip_rich_text` only strips what is a tag), and each list and tab keeps its own place (one scroll offset was shared, so the Library opened where the New Game list had been left
  and a champion pick moved the scenario list). The Patreon button's text on the title screen wraps in its box too. Texts now take the line height of their own font asset (`line_height_named`: Boston
  Black's is 1.3 of its size, a role's font was used) and a single line taller than its box is centred on it, as TMP does: the mean pixel difference to the real game
  fell on every title-menu screen (title 3.04 -> 2.19, new game 9.76 -> 8.99, load 3.41 -> 3.23, library 5.56 -> 5.35, settings 2.17 / 2.64 / 4.30 -> 1.12 / 1.55 /
  3.32, credits 7.35 -> 6.98, additional tools 10.71 -> 10.49, scenario editor 16.51 -> 15.94). Not done: the links of the Steam Workshop section (inactive in the
  capture), the Patreon simulation and the sprite editor scenes.
- **Hotkeys** are the game's: space, H, R/G/O/E (Ctrl repeats the pick after *Reselection Delay* of game time until Escape), F5, Insert, Delete,
  Home, =/-/keypad +-, PageUp/PageDown (keypad minus does both zoom out and camera speed down), Ctrl+C/Ctrl+V (species tag of the target; the
  selection panel tags all; at most 100 characters). Removed the viewer-only keys (`.` step, F, B, P, digit speeds).
- **Camera / pointer**: the right button deselects the target and pans even over a panel (`MouseDrag` has no UI check; only the wheel does);
  *Allow Arrow Key* gates the arrow/WASD move; keypad minus bug fixed (it was keypad 1).
- **Target life cycle** (`UserControl.TargetDied`): a selected egg that hatches hands the selection (and the camera) to its newborn; a bibite picked
  with R/G/O ends the selection when it dies (*Reselect On Corpses*), a click-picked one stays as a corpse; a removed target is dropped.
  Core: `World::take_hatched_children`; **`World::abort_egg` was ignored for incubating eggs** (so Delete / Kill / Kill Half on a real egg did
  nothing) and now works.
- **Bottom-left toolbar** (`OnHoverShow`, 0.5 s open / 0.5 s close, real time): the Placer / User Actions / View panels open after resting on the
  button (or on a press) and close after leaving button and panel; they used to toggle on click.
- **User Actions**: Kill Half and the new Remove Half Pellets (`UserActions.RemoveHalfPellets`: remove when `Random.value >= 0.5`, then recount) ask
  with the game's own title / text / Cancel+YES in the game's popup look; Kill Half no longer deselects. Other confirmations use the game's texts.
- **View panel**: its seven switches now flip their own setting (*Status Bars*, *Field Of View* and *Targets* used to toggle the whole HUD or nothing)
  and start from the player's stored values (Show Pheromones / Fertility / Ruler / Status Bars / Field Of View / Targets are now settings).
- **Top-right gear** opens the pause menu (`EscapePanel.OpenPanel`), not a settings screen. The three right-hand panels (statistics, dynamic
  parameters, lineage) replace each other (`RightmostPanelsManager`).
- **Undo / redo** (`UINavigationManager` revertable stack): Ctrl+Z / Ctrl+Shift+Z step through the edits of the dynamic settings panel (a
  slider drag is one step, 60 kept, a new edit drops the redo tail, a new world clears it; only with something open on the Escape stack, as in the
  game). Not covered: edits made in the Settings screens and the scenario editor.
- **Object dragging** (`DraggableObject`): a bibite / egg held past 0.15 s follows the cursor (velocity 10 x offset / timeScale, a jump while paused;
  the selected one never moves; Ctrl turns it towards the cursor); pellets follow at once; towers and colour killers jump to the cursor (no rigid
  body; the selected one and one that follows a zone stay put, no Ctrl turning); nothing moves in a challenge.
- **Bibite placer** (`BibitePlacer`, `BibiteTemplateSelectorPanel.OpenWithCallBacks`; `placer_panel.rs`): the Placer's `LoadBibite` button resets
  the custom tag and opens the template selector (the Library in pick mode, submit "Select Bibite") with the game paused
  (`TogglePauseGame("bibitePlacer")`; Back / Escape close it and give the previous pause back); the choice ends that pause, drops the target, turns
  selection off (`AllowSelection`: no click or rectangle selection, no R/G/O/E, Insert, Delete or F5 meanwhile, the camera keys stay) and opens the
  options panel bottom-left (prefab geometry: Mutate Genes / Growth At Spawn / Tagging dropdowns (real lists), the custom tag field you can
  type into, the close button). Every left press off the UI spawns the template there with those choices (`SpawnBibiteFromTemplate`) until the
  close button or Escape. The choices persist between uses like the game's static `BibiteSettings`. Not done: aligning the template's brain
  innovations with the sim (`AlignInnovationsWithSim`), the mutate-genes choice for eggs (the core's egg path), dragging bodies while placing,
  tooltips. The real-window path (selector overlay, then placing) is covered by unit tests of its parts and headless screenshots,
  not driven with the mouse.
- **Pellet placer** (`PelletPlacer`, `PlacePelletPanel`; `placer_panel.rs`): the real panel replaces the invented "TOOL" box (which also had a
  "Radius" control the game does not have: the pile radius follows from the count and the energy). Bottom-left, prefab geometry: Material (Plant /
  Meat, a real list), "Pellet Size Factor" (log slider 0.1 to 1000, default 1, shown with the game's SI format) and "Pellet Number" (1 to 10),
  the close button; opening it drops the target and turns selection off, Escape and the button close it. Not done: tooltips.
- **Wheel zoom** sets the orthographic size at once about the screen centre like `CameraManager.CameraZoom` (it used to ease towards the cursor).
- **Towers and the colour killer** (`EffectTower`, `ColorKiller`, `PheroTowerPanel`, `RadTowerPanel`, `ColorKillerPanel`; `tower_panel.rs`): the
  three Placer buttons arm their own kind (the first used to place a radiation tower) and a translucent ghost follows the cursor (half opacity,
  its radiation area too); a left press on ground the UI does not cover places it once, the right button cancels. A placed tower follows no zone,
  becomes the target and opens its panel; a placed colour killer is *not* selected (`ColorKiller.Place`), it opens its panel when clicked
  (`TargetObjectOnClick`: a press shorter than 0.15 s on the collider circle, 17.5 / 19 units; a bibite within the selection radius takes over
  the target as in the game). The panels follow the prefabs (584 x 275, 555 x 275, 514 x 428 at 10, 10): title and `(#id)`, Remove (destroys it,
  also the Delete key on a colour killer), the "Attached to Zone" dropdown (None and every zone; for the killer the five closest with their
  distance, and it moves onto the zone), the sliders with the game's ranges and value texts (phero red / green / blue 0-100 u² log, heading;
  rad count / intensity / radius / taper; killer hue, tolerance, radius, period), the killer's result strip, Body / Eyes and Any / Most switches
  and the progress bar. Escape closes a tower's panel first and deselects with the next press; a colour killer deselects at once; the right
  button and Home deselect it like any target. The towers are drawn (they were not drawn at all): `laser_circle`, `radioactive`, `pheroTower`
  (rotated by its heading) and `laser_crystal` (tinted by the hue) from `sharedassets2.assets`, on the `Recessed` sorting layer, under pellets and
  bibites; the selected colour killer shows its scan radius at 30 % opacity. Core fixes found on the way: a pheromone tower's spot ignored
  edits of its strengths / heading (`UpdatePheroSpot`), and a new colour killer used invented values (now the prefab's: tolerance 0.1, radius
  400, period 100). Not done: the kill laser, the animated `radioZone` shader (a faint disc and rim stand in), typed values (`ClickToEditHover`), tooltips, Ctrl+Z of tower edits, the 0.77 opacity of towers loaded from a save, the Ctrl turning
  of a dragged tower. Brain Help opens the game's web page; the species name in the Genes / Brain panel opens the species panel on
  that species (with the same "not enough data" gate); Edit and Play (x1 speed + unpause) map from the wiring.
- **Stats panel buttons**: Kill / Lay egg / Revive do nothing in a challenge; Lay needs a living bibite; the magnifier beside "Alive children"
  selects a random living child; the tag label is a text field (type, Backspace, Enter confirms, Escape cancels, hotkeys blocked meanwhile).
- **Save Bibite dialog** (`SaveBibiteDialogHandle`, `SaveController.SaveBibiteOrEgg` / `SaveDialogConfirmed`; `save_dialog.rs`): the Stats panel's Save
  (and the egg's) opens the game's dialog instead of writing `bibite-<id>.bb8` at once: a modal over a dimmed world with the game paused and every
  control blocked (Escape and Cancel close it and give the previous pause back); "Bibite Name" starts as the species name (the game's "Bibite
  personalis" without one), "Description" as the species description (Enter breaks a line, Ctrl+V pastes), "Save as template" and "Format JSON"
  start from the player's stored choices (`SaveBibiteAsTemplate`, `FormatBibiteJSON`, read from the game's own preferences and kept in the viewer's
  user settings; they also became live rows of the in-world settings panel). A name with a Windows-invalid file character shows the game's warning
  and disables Save; an existing file shows "*The existing file will be overwritten*" and Save overwrites it. Save writes `Bibites/<name>.bb8`
  (the entity plus the game `version` and the player's `desc`) or `Bibites/Templates/<name>.bb8template` (named and described), indented with
  Format JSON, and the template shows up in the Library, the placer and the default bibites list. Geometry from the prefab (320 x 453, rows 10
  apart, a hidden warning takes no room). Not done: the real `TMP_InputField` look and selection, the dimming is a plain 43 % black.

- **In-world zone editor** (`ZonesEditorPanel` is the same class as the scenario editor's, `ZoneSettingsHandle`, `GUIManager.OnTargetChanged`):
  the Dynamic panel's Zones tab is the game's `ZoneEditorPanel`: the "Zones Parameters" group, then the Zones header (`+` = `AddZone`: a Plant
  zone "Zone n" with the game's defaults at the world's centre, its item opens; "remove all" asks "Delete All Zones ... You will lose all data"),
  one item per live zone (the bar opens / closes it, several can be open, the delete button only shows while open and asks "Delete Zone / Are you
  sure you want to delete <name>?"), and the Zone Groups header. The item's controls edit the live zone at once through the core's
  `apply_zone_settings` (sliders drag, toggles, the dropdowns open real lists, reset pointers), as the game's settings do through their
  change events. The Z key (`UserControl.Update`, no modifier check, so Ctrl + Z also undoes) selects a random zone: it becomes `UserControl.target`
  (Escape deselects it after closing the panel; right-drag / another selection drops it) and, with the new user setting *Open Zone Editor On Select*
  (default on), the Dynamic panel opens on the Zones tab with that item open. A zone cannot be selected by clicking it: the prefab has no collider
  and no `TargetObjectOnClick`. The camera follows any target (`CameraManager.SetFollowTarget` / `LateUpdate`), so it now centres on a selected
  zone and on a selected tower or colour killer too (it only followed bibites and eggs before). Core fixes found on the way: removing a zone kept
  the other zones' follow targets and the default bibites' spawn zones on their ids, which the removal renumbers (they now keep following the same
  zone, the removed zone's followers lose their target); `apply_zone_settings` did not make a new movement type / target take effect nor move the
  zone when posX / posY changed (the next tick wrote the old position back over the setting); it now does, carrying attached zones along as the
  game's transform parenting does. The zone name is a text field as in the game (`TextLineHandle`: a click on the name starts it, typed characters and
  Backspace edit it, Enter renames the zone, Escape or a click elsewhere closes it unchanged, hotkeys are blocked meanwhile). The corner button of
  the tab (`TogglePreview`, with its tooltip) shows and hides the Preview panel (`zone_preview.rs`, `ZonePreview`: `gray_panel` 464 x 600 left of the
  dynamic panel, the zones drawn as in the scenario editor's preview, and the three estimation lines "Total Biomass / Pellet Production / Estimated
  Pellets" summed over the world's zones; hidden at first like the game's). Not done: the up / down reorder buttons of an open item (the game's
  `ReorderableItem` only changes the UI sibling order, no callback), the zone *groups* (the world has no group membership: a new-game world's
  group zones are ordinary zones in the Zones list, which the game would only list under their group; the Zone Groups buttons say they are not
  available). The zone item's controls (sliders, toggles, dropdown picks, the name) are on the Ctrl + Z history since
  2026-11-04, one step per edit and one per slider drag, in the same history as the other settings (`UndoAction::Zone`: the zone's settings before
  and after, found again by its persistent id; deleting a zone renumbers the others, so the steps that edited zones are dropped then). Adding or
  deleting a zone, the default bibites and the in-world dragging are not on it.

- **Dropdown lists** (`TMP_Dropdown.Show`, `MyDropdown` / `RealDropdown` / `TogglesDropdown`; `dropdown.rs`): every dropdown the port has hangs a
  real list. A click opens it two px under the control's bottom edge, as wide as the control, with the game's items (`Item Background` 0.961
  grey, lighter under the pointer and on the selected item, the checkmark on the current choice), a scroll bar and the mouse wheel when the
  options outgrow the template's 150 (the list flips above the control at the screen's bottom edge) and a blocker that closes it on any other
  click; Escape closes it first and the hotkeys are blocked while it is open. Covered: the zone items (spawn material, distribution, movement,
  target: "None" and every zone), the default-bibite items (tagging, spawn zone, growth, spawn logic, mutate genes), the bibite placer's three, the
  pellet placer's material, the towers' Follow Zone (None and every zone), every dropdown row of the three Settings screens (options from
  `SettingChoices`) and the five New Game modifiers. The captured controls that have no target of their own (`App::open_dropdown`) hang the same
  list instead of the old hand-drawn one. The Graphs panel's two are the game's custom `MyDropdown` family: 35-high items listed in full (no
  scrolling), the list closes when the pointer leaves the control and its list, and each item shows its description as a tooltip at once; the
  Curves dropdown is a multi-select that stays open while its items toggle. **Item tooltips** (`ChoiceSettingDropdown.OnItemHovered`,
  `NumericSettingDropdown.OnItemHovered`; `TooltipSystem.Show`, so no delay): resting on an item of a zone's Spawn distribution or Movement, of a
  default bibite's (and the placer's) Tagging, Mutate Genes or Growth At Spawn, of the New Game screen's size, biomass and fertility, or of the
  Settings screens' Screen Resolution and UI Scale shows "Setting name: Choice" over the choice's `tooltipText` (a landmark's value is its
  `ScientificFormat` with the setting's precision and units, "1.00 ku" for the smallest world; "Sterile" has a text of its own). The other lists have
  none in the game either (the material, target and spawn zone dropdowns, radioactivity, the bibite limit, Procedural Size and Autosave Period are
  classes that never set `onItemHovered`). Found on the way: `BibiteSettings.spawnType` is never given its `spawnTypeChoices`, so the Spawn Logic
  list is the enum's names with spaces, "Continuous" and "One Time" (it said "Once"), and its items have no tooltip. Not done: the dropdowns of
  panels that do not exist yet (the Animations tab's animator preset, the species panels' "compared to" rows, the brain editor node's function,
  the genes graph page, the Workshop). No reference screenshot of an open list exists: the geometry
  comes from the serialized prefab values.
- **Procedural bibite previews** (`BibiteGenePreviewer`, `BibiteTemplateGenePreviewer`, `ProceduralSpriteManager`; `preview.rs`): a preview composes a
  bibite from a template's genes with the sprites the world uses (the size tier by `UserSettings.ProceduralSize`, the inflate-free body, the mouth, the
  eyes, the arms and the exoskeleton the genes pick), scaled by `Clamp(baseScale * sqrt(size), 0.05, maxScale)` canvas units per sprite pixel
  (`size = genes[3] * sqrt(GrowthAtMature)`; the prefab values were read with UnityPy: the tree node 5 / 9, the info panel 5 / 10, the mesh's disc
  10 / 20, the Library 10 / 24), hung at its prefab's offset from the middle of the picture (the UI previewer moves it with the scale, `adjustOffset`),
  turned towards its food with `rotateModelWithDiet` (`-2 (carn - 0.5) atan(40 / (112 sqrt(size)))`); the parts are the prefab's children in order
  (mouth, body, left arm, right arm mirrored about its pivot, exoskeleton, eyes). The Library's picture also draws the plant and meat icons of its
  `FoodPreviewHolder` (200 right, 40 above and below, 40 x 40, on the holder's `1 / sqrt(size)`; alpha `4 (1 - carn)^2` / `4 carn^2`, each scaled by
  `sqrt(share + 0.5)`); with no template selected it shows what `UpdateTemplate(null)` does (size 1, white, an even diet). The picture is rendered
  once per genome and size by the software rasteriser at the size it is shown at (sprites are point sampled, each sprite pixel a block) and cached
  (`PreviewCache`), then drawn as `UiDraw::Pixels` (`PixelsRegion` where the tree's mask cuts a node; the window's GPU path uploads and caches the
  pictures, it only drew them in software before). Shown in the species Info panel, in every tree node that has room for its preview box and in the
  Library selector (the window loop makes the Library's picture, the menu is painted in software). Found on the way: the software sprite rasteriser
  covered only the area within half the sprite's diagonal of its anchor, which clips a sprite hanging off its pivot (a body's left part); it covers the
  rotated rectangle now. Checked by tests (the numbers, the sprite order and pivots, the pictures and the cache) and headless screenshots; the GPU upload path
  of the pictures was only type-checked, nothing was driven in the real window. The lineage mesh's previewer (`MESH`) is drawn too (below). Not done: the
  gene editor's `ProceduralGenePreviewer`, the scenario editor's and the champion picker's previews, the plant / meat icons' 1 / sqrt(size) holder for a preview that
  has no food.
- **Typed values** (`SettingSlider.OpenEditPanel` / `CloseEditPanel` / `SubmitEditValue` / `UpdateValueDuringEdit`, `LogSettingSlider.SubmitEditValue`,
  `value_edit.rs`): in the in-world dynamic panel (the setting rows, the zone items' sliders, the default bibites' Priority and Target Count and the
  Energy tab's material sliders) a slider row's value text is a button (`editButton`; `ClickToEditHover` only swaps the cursor, which is not done):
  pressing it hides the slider and its reset button (`NormalSecton`) and opens the row's `EditSection` - a `TMP_InputField` with an integer
  (`IntSettingSlider`, Target Count) or decimal content type - filled with `setting.val * format.factor` (.NET `float.ToString()`, e.g. 7.5 for the
  0.0075 biomass density that shows as "7.5 mE/u²") and selected whole (`onFocusSelectAll`). The row is a `HorizontalLayoutGroup`, so the name, the
  value text and the field share its width by Unity's flexible sizes again (name 100 / 125 / 2, value text 60 / 80 / 1, the field at least 300 wide,
  15 apart: `value_edit::row_parts`, checked against two captured rows, 403.4 / 219.2 / 394.2 and 343.5 / 189.3 / 364.2 wide). The field is 34 high with
  the sliced `InputFieldBackground`, `LiberationSans SDF` 20 in dark grey, the `selectionColor` highlight behind a selected text, a 1-wide caret
  blinking at 0.85 Hz after the text and the "Enter value..." placeholder when empty. Typing follows `TMP_InputField.Validate` for integers
  and decimals (digits, a leading minus, one point or comma that becomes the separator, checked against the whole text so a selected "7.5" refuses a
  point); the first valid key replaces the selection, Backspace deletes it, Ctrl + V pastes through the same validation. While typing the row's
  value text shows the value the text means (`UpdateValueDuringEdit`: an empty text is 0, a lone sign or point gets its zero, divided by the factor,
  not negative when the setting's minimum is 0, clamped to the ends only when it cannot go out of bounds). Enter applies it (`SubmitEditValue`:
  the same, cut to a whole number for an `IntSetting`; a `LogSettingSlider` clamps before dividing by the factor, has no fix-ups for a lone sign or
  point and is the only kind that puts the change on the Ctrl + Z history: a linear slider's typed value is `SetValue(.., revertable: false)`); the
  zone, material and default-bibite settings leave `canGoOutOfBounds` at its default so a typed value past the slider's end is kept (the setters
  `set_value_unclamped`; the core may still refuse what it validates). Escape, a press on any other control or on the world closes the field
  without applying it (`closeEditPanel`, `onDeselect`), a press on the value text opens it afresh, and the keyboard hotkeys, Space and Ctrl + Z are
  blocked meanwhile (`SetKeyboardBlockFromSource("SliderEdit")`). `--dyn-tab 1 --dyn-edit biomassDensity:2.5` shows it headlessly. Checked by unit tests
  (the rules, the float text, the layout against the captured rows, the keys, the history, the zone and material setters), a render test
  of the open panel and headless screenshots; **not driven with the real mouse and keyboard**. Not done: the caret keys (arrows, Home, End, Delete,
  Ctrl + A, selecting with the mouse: the caret is always at the end), the scenario editor's and the Settings screens' sliders, the towers' and the
  pellet placer's, the edit cursor. The Lineage panel's Settings panel (it draws its own rows through `lineage_settings.rs`) has the same field.
- **Graphs panel** (`GraphsPanel`, `DataStreamLinesGraph`, `BibiteTracker`, `DataLogger`; `hud.rs::draw_graphs`, `datalog.rs`): the Data dropdown
  lists the game's seven groups in the game's order, each with its description as the tooltip; picking one plots it and, like `SetDataStream`,
  falls back to the deepest scale that has data. The core was missing *Age of Death Data* (the ages the dead bibites died at): it is fed from
  the game's rolling Greenwald-Khanna estimator (3 estimators, a roll every 20 minutes, epsilon 0.05, ported with `List.BinarySearch`'s tie
  behaviour) at each death. The Curves dropdown switches each curve (all on to begin with; the caption reads "All" / "None Selected" /
  "N Selected"; the state is kept per group like `DataStreamDescription.state`); only the curves that show are plotted and the y axis fits them
  (`UpdateGraphData`). The newest point is the group's current value (`PeekCurrentDataPoint`, `World::data_points`), not the last log, so a new
  world's plot is not empty. A time-range tab whose scale has no data is not interactable. The "?" button shows the legend (the curves that show,
  colour swatch and label, their description as the tooltip) while the pointer rests on the button or the legend (`OnHoverShow` /
  `OnHoverKeep`). The number formats follow the `DataLogger` text formats (the ages in hours, the eggs and the births / deaths without SI
  prefixes). A loaded save's history fills the plot and the time-range tabs (see **Graphs history in saves**). The **hover indicator** (`GraphTooltipTrigger` -> `BaseLineGraph.UpdateTooltip`, `GraphDataLine.UpdateTooltip`; 2026-11-06): while the pointer is over the
  plot a white line (3 thick, the prefab's `Line`) stands at the point nearest to it (`IndexOfClosest`: of equal distances the last one) and every
  curve that shows has a dot (the prefab's 10 x 10 `Disc`, the curve's colour) with its value in the group's format over its upper right (the prefab's
  `FloatInfoText`: 16 pt bold white, its bottom left corner 5 right of and above the dot's centre); an open dropdown list takes the pointer away. The
  disc's radius and the line's thickness are read from the serialized Shapes components' floats (no reference capture of a hovered graph exists), the text's
  font is the bold one. Not done: the exact legend width (the label widths are estimated) and its fade.
  **Gene Analysis page** (`GraphsPanel.genesGraphPage`, `BarGraph`, `GraphDataBar`, `LogLikeTimeSpanSlider`; `gene_analysis.rs`; 2026-11-06): "Go To
  Gene Analysis" (`OpenGenes`) opens it and "Go To Data Graphs" (`OpenData`) goes back; the panel remembers the page, the gene and the time. The
  "Gene Distribution" dropdown (a `RealDropdown`: 250 x 35, caption 18.75, 22-high items of 12.8) lists the 35 genes in the enum's order with their
  help texts and starts on the 17th, Diet. The histogram (`BarGraph.UpdateGraphData`) is the 20 bars of the chosen point of the genes stream, "Now"
  (the living bibites and the eggs as they are; the panel pauses the game so it needs no other refresh) or a logged hour: the y axis tops at the
  largest count rounded up to a multiple of its decade (at least 10), the 26 x-axis labels run over the gene's whole range in the setting's own
  format (precision, units, factor, SI, read from `BibiteEditorSettings` by `tools/extract_gene_info.py`, which now also reads the range and
  handles `1E-06f`), and the bars stand between two flexible spacers that put each at its value on that axis (the layout groups' maths, a bar
  never taller than the holder's inner height). A bar's tooltip is `[a : b]`, `count: n (p%)`. The time slider runs right to left from "Now" (`-1`) through the
  logged points (`loggedGenes - 1`), with the text "Now" or "N ago" (`TimeOfPointInSerialConfig(SerialHeavyConfig)`: 1h ... 6h, then 9h in 3 hour
  steps ..., "1d12h ago"), pressing the track jumps, the handle keeps its grab offset, the value is clamped into the range (a world with fewer hours
  clamps it too). Checked by unit tests (the maths, the frame, the app state), a routing test through the window's pointer path (button, list, row,
  slider handle and track) and headless screenshots of the real save's genes stream (`--genes-gene N --genes-time N`); **not driven with the real
  mouse**, no captured reference of this page exists (the colours, the label sizes and where the grid lines start are the historic page's).
- **Bibite Editor** (`BibiteEditorPanel`, `BibiteEditorPreview`, `BibiteGrowthGraph`, `BibiteTemplateSelectorPanel`; `template_editor.rs` the model,
  `template_editor_ui.rs` the screen; 2026-11-06, **in progress**).
  *Entry points.* The editor always sits over the Library (`BibiteTemplateSelectorPanel`): the inspector's Edit button (`BibiteStatsPanel.EditBibite`:
  `new BibiteTemplate(body)`, named after the species or the tag, its genes and its brain) and the species panel's Edit (`SpeciesPanel.EditBibite`: the
  species' template) call `OpenForBibiteEditor`, which opens the Library and the editor over it; the Library's own submit button (the one the title menu's
  "Bibite Library" and the pause menu's "Bibite Editor" open it with says "Edit Bibite": `OpenWithoutGoals`) and its Edit button are `EditSelectedBibite`
  (the selected template, or a saved `.bb8` bibite named after its file, or a new template when nothing is selected), and Create New is
  `OpenEditorForNewBibite` (`EditTemplate(null)`: the genes' defaults, the name "Bibite template", the bare 33 + 15 base neurons, and no "Save and
  submit": `saveAndSubmitButton.SetActive(actionOnSubmit != null)`, the buttons left of it move 80 right). The title menu's Library no longer starts a world
  with the selected template (that was this port's stand-in; the game's submit there is "Edit Bibite"). The Library pauses the game while it is open
  (`TogglePauseGame("BibiteLibrary")`, given back as it was; the placer's pick keeps its own pause) and the editor does too; Escape closes the editor first
  (a focused text field before that) and lands in the Library (`canBeEscapedFlag` is false while the editor is up), Back says "Back To Bibite Library". When
  it closes the Library fills its lists again: `AfterEditorClose` keeps what was selected, `AfterEditorSubmit` (Save and submit) selects the template that
  was saved (the Templates list's first of that name, nothing when there is none). The editor opens over the title menu as well (the window's pointer and
  keyboard are its, through the same routing as the world's panels, the menu waits behind it) and over a Library that is picking a template (the pick goes
  on after it). A new template saved from the editor has generation 0 and `speciesName` as typed, as `SaveTemplate` writes it (the bibite's generation does
  not come along).
  *Genes tab.* The 35 `FloatSettingSlider` rows in `geneSettings` order (name with the gene's help as its tooltip, value in the setting's own format, slider
  with the default value's pointer, reset button), 16 rows at a time, the list scrolled a row a wheel notch or a page by the scroll bar's track; a press on
  the track or the knob sets the value and the drag follows the cursor (never out of the gene's range, `canGoOutOfBounds` is off), the value text opens the
  typed-value field of the other slider rows (`FieldRef::Gene`; the number is divided by the format's factor, clamped into the range, Enter applies, Escape or
  a press elsewhere closes it). The game's hotkeys, Space included, wait while the editor is open. The right column has the name field (single line: Enter
  leaves it) with its three warning icons (`incorrectWarning`, `existingOfficial`, `existingWarning`: `CheckExistingBibite`, the three icons' tooltips from
  the scene) and the description field (new lines allowed). Save (`SaveAndContinue`) writes `<name>.bb8template` into the templates folder (indented with the
  player's `FormatBibiteJSON`, anchors kept), Save and submit (`SaveAndSubmit`) writes it and closes, Back closes; both save buttons are greyed while the name
  is invalid (bad file-name character, empty, or one of the game's own template names); every top button has its tooltip. The preview frame draws the
  template's procedural bibite (`ProceduralGenePreviewer`: scale 9, largest 22, the bibite 3 units ahead of the middle of the 548 x 392 frame, which is
  where the part objects' `anchoredPosition (0, 3)` put it and what centres it in the reference screenshot, turning with its diet, the plant and meat icons of
  `FoodPreviewHolder` at its right; its size is its own (`UpdatePreview`: the size gene, the growth at maturity or their product by the `ProceduralSize`
  choice, not the template previewers' `sizeRatio * sqrt(GrowthAtMature)`); `preview::EDITOR` through the same `PreviewCache` as the other previews, so it
  needs the game's sprites and stays empty in a sprite-less test). In the game the Library's own previewer bibite (a world-space `SpriteRenderer`, not a UI
  image) is drawn over the editor too, at the Library's picture place (the reference screenshots have two bibites): not reproduced. The info button over the
  picture (`InfoButton`) swaps the name and the
  description for the stats panel (`Bibite Stats Preview`): the growth curve (`BibiteGrowthGraph`: 11 x 5 grid lines, axes in the game's number formats) and
  the maturity at birth, the size and length at maturity and the seven organs' shares of the area, all computed from the genes as they are
  (`TemplateEditor::preview`).
  *Checked by* unit tests (the model: ranges, name rules, save / load round trip, files; the screen: rows, scroll, slider, typed value, text fields, buttons,
  Escape, hotkeys, the stats panel, the Library under it and its pause, Create New, the pick, the species entry; the Library: its three buttons and the refill),
  a routing test through the window's pointer path and headless screenshots (`--bibite-editor [--select-first] [--editor-scroll N] [--editor-info]`: without a
  selection it is Create New); **not driven with the real mouse and keyboard**, in the world or over the title menu. The geometry is the 1920 x 1080
  reference's (read from the blurred reference screenshots and the prefab's layout structure: row pitch 60, slider track 856 to 1236, reset button at
  1258...); no sharp capture of the editor exists (the captured sessions have it hidden), so sizes are within a few pixels (the top buttons' prefab size is
  70, drawn 62 as measured on the reference).
  *Brain tab* (`BibiteBrainEditor`, `NodeEditor`, `ConnectionEditor`, `NodeEditPanel`, `ConnectionEditPanel`, `AddNodePanel`; `brain_editor.rs` the model,
  `brain_editor_ui.rs` the screen; 2026-11-06). Only the nodes `BrainSorter.UsedIndicesOfGroups` calls used are on the canvas (the connected ones, and the
  outputs that are Sigmoid or have a bias); the add-node panel offers the other inputs and outputs and the 13 functions of a hidden node. The canvas is the
  `BrainEditorScroll`: content `max(1.5 * 0.7 * Screen.width, 2 * width)` by `max(1.5 * 0.8 * Screen.height, 2 * height)` (the window's pixels, as the
  game sizes it at `FillEditor` and `ReSortBrain`), clamped, no inertia, centred when it fits; the right button pans it after 10 px (a node under the press
  keeps it from panning), the wheel zooms about the cursor by 10 % a notch (0.1 to 5) unless something is selected, the add-node panel or the help is open or the
  name or description is being typed; 12-pixel scroll bars show while the content does not fit (the handle in proportion, `AutoHideAndExpandViewport`).
  The toolbar: Step (key `1`, held `1` steps by the 0.1 s period), Play / Pause (the little simulator: `NodeEditor.ExecuteNode` through the core's
  `execute_neuron`, connections propagating `LastOutput * Weight`, Mult nodes multiplying), the eye (`UserSettings.ShowNodeValues`, off by default,
  kept in the preferences), Save node positions (gone once they are kept; the notice over the preview says "WILL" in green or "WILL NOT" in red), Sort
  (greyed while a node is invalid) and the help (it shows after the pointer rests on its button for 0.5 s, or when pressed, and stays while the pointer is
  on it; its text is the prefab's and scrolls 25 px a notch). A left press on a node that moves less than 10 px selects it and opens its panel at the click
  (the game's pivot rule: 75 px to the side and below or above), further on it drags the node to the cursor (stuck to the brain's left and right edges
  within 10) and drops the selection; a click on a connection selects it (its strip is highlighted); Shift + click starts a connection from the node (at an
  output, one that ends there), the new connection follows the cursor until a node it may end at is clicked (Escape drops it); Space opens the add-node
  panel at the pointer (filter field focused, typing narrows the lists, Enter adds the one thing left, a click adds it, the section headers fold their
  lists; Space is not typed into it), Delete removes the selected node (with its connections; an active output stays, and is offered by the panel again)
  or connection, Escape undoes the newest entry of the game's stack (the selection, the connection being made, the add-node panel; the help is not on
  it, it goes when the pointer leaves) and, in the same press, a text field that had the keyboard gives it up (`TMP_InputField` cancels: the label has
  its text from before the edit again), and then closes the editor; a press on anything but one of the two fields takes the keyboard off them (the text
  stays). A node added from the panel is selected as a click would select it (`SelectElement`): while a connection waits for its end the new node ends
  it (when it can), with Shift held it starts a connection. The node panel: its three buttons (connection in,
  connection out, delete; the tooltips are the scene's), the label (a hidden node's own text), the activation function dropdown (13 choices with their
  tooltips; hidden nodes only), the notes ("can't change", the non-linear warning), the bias slider (-5 to 5; an Inhibitory node's 0.0001 to 10 and a
  SoftLatch's 0.0001 to 1000 stay in range, a typed value too) and an input's value slider (its sense's range; a typed value has the setting's -5 to 5
  and may leave it); the panel's bias is the game's own `bias` setting, which `EditFunction` does not refresh: a new function gets its default bias, then
  an Inhibitory or a SoftLatch node's slider is forced into the range with the value the panel held (so a Sigmoid node of bias 0 switched to SoftLatch
  ends at 0.0001, and the row goes on showing a stale value after a switch to a function whose slider is not forced); the connection panel: add node
  before / after (splits it: a Linear hidden node of bias 0, the other half of weight 1), delete, the weight slider (-10 to 10, typed values may leave it)
  and the Enabled toggle. Every slider has the game's typed-value field (`FieldRef::BrainBias`, `BrainAct`, `BrainWeight`). Save and Save and submit are
  greyed while the brain is not valid (an input without a connection out, a hidden node without one in and one out) as well as the name; saving builds the
  template's nodes (hidden nodes re-indexed in order, innovations `index + 1`) and connections (innovations 1..n) as they are (`Brain::from_records_unrepaired`:
  a SoftLatch bias above 100 is kept) and, when the positions are kept, their fractions of the brain's extents. `anyChange` (the achievement trigger) is
  set only where the game sets it. *Checked by* unit tests (the model; the canvas maths, picking, the panels' places and sizes, the sliders, the add-node
  lists and Enter, the dropdown, typed values, the template), a routing test through the window's pointer path (wheel, click, slider drag, right-button pan)
  and headless screenshots (`--bibite-editor [--select-first] --editor-brain "tab;values;wheel X,Y -1;click X,Y;space X,Y;type TEXT;enter;info"`:
  the scroll bars' handles match the reference's to a few pixels, 64 % and 68 % of the tracks); **not driven with the real mouse and keyboard**, and the
  reference screenshot is blurred, so the node, panel and toolbar sizes are the prefab's. *Differences:* the node's value text is drawn white (the
  prefab's TMP face colour is (56, 56, 56) with an outline material the port does not model; the reference's blurred text is lighter than that); the
  node's selected and invalid highlights are a filled ring behind it instead of the node shader's `_Selected` / `_Invalid` effects; the add-node list clips
  whole rows only (partly visible ones show as plain strips) and its scroll bar jumps instead of paging on a track press; the edit field of a slider row
  does not make the panel taller by the 4 px the game's layout does; the panels do not reflow their rows when a text wraps; the connections' synapse
  shader animation is not modelled; leaving the tab with a node selected keeps the selection (the game keeps `elementSelected` too, but hides the panel);
  opening a template for editing still goes through `Brain::from_records`, which repairs it (a hidden node without a connection in or out is dropped, a
  SoftLatch bias is clamped to 0.1 to 100), where the game's editor reads the stored nodes as they are; the name and description fields still need two
  Escape presses (one to leave the field, one for the editor).
  The editor's achievements work (`ACH_BIBITE_EDITOR` on a save after a change, `ACH_BIBITE_EDITOR_100` on the hundredth: see "Achievements").
  *Not done yet:* the Save sprite button (drawn, with its tooltip; `SpriteGenerator.SaveSpriteOfPreview` opens the
  system's save dialog and builds the picture from the previewer's five sprites: it says it is not available).
- **Menu text sharpness**: the software SDF renderer ramped the alpha over `0.3 + ...` distance units whatever the size, so Boston Black text (the
  title and every title-menu label: 68-point atlas, gradient scale 6) was several pixels soft at 40-100 px. It now ramps over
  `1 / sqrt 2 / (gradientScale * pixels per texel)`, about a pixel and a half (`TMP_SDF.shader`'s `1 / scale`), calibrated against the game's own
  captures: the mean pixel difference to the real game fell on most screens (settings 2.40 -> 2.17, credits 8.00 -> 7.43, title 3.16 -> 3.08,
  library 5.64 -> 5.56) and rose on the text-dense New Game screen (9.58 -> 9.76) because crisp text shows sub-pixel layout offsets that blur used
  to hide. The title menu is also painted at the window's own pixel size (capped at 2560 x 1440) instead of the launch size stretched over a
  maximised or fullscreen window. Not done: "The Bibites" is about 2 % wider than the game's (kerning and the small-caps advance are not
  verified).

- **Species panel** (`AncestralLineagePanel`, `SpeciesPanel`, `SpeciesTabManager`, `SpeciesInfoPanel`; `species_panel.rs`): the in-world
  Lineage panel is the game's layout instead of the old dark stub (prefab values from level2): the full-screen grey panel with the 50-high top
  bar (the title with "N Recorded Species" and "(M displayed)", the Mesh / Tree / Settings / Exit buttons at x 1745-1895), the navy graph view
  with its scroll bar tracks, and, once a species is picked, the tab row (Info, Genes, Brain, Position icons at x 45-195, the selected or
  hovered tab rising 16, the others half behind the panel, and the close tab) with the 332 x 489.71 Info panel at the overlay's top-left:
  the name row (a click turns it into a text field: typed characters, Backspace, Ctrl + V, Enter or a press on another control applies a
  name of the form "Genus species" (`^[A-Z][a-z]+ [a-z]+$`), anything else is dropped, Escape drops it, the red bug icon and its tooltip show
  while the typed text is wrong), the species ID, the preview frame, the root and parent buttons (a root shows "None" for its parent; they
  select that species), the six 35 x 35 buttons (Favorite toggle, Save, Place, Select, Prune, Edit), "Time of creation" (`{d}d {h}:{m}`),
  "Gen at creation", the alive members and alive descendants (`Species.GetDescendantCount`, with their tooltips) and the notes field (a
  multi-line field that saves as you type; Escape puts the text back as it was when the field was entered). Save opens the Save Bibite dialog (`SaveTemplateOfBibite`: the species' name and
  notes, "Save as template" on and locked) and writes `Bibites/Templates/<name>.bb8template` with the typed description and the founder's generation; Place closes the panel and starts the bibite placer
  with the species' template; Select picks a random living member (disabled without members, with the game's tooltip); Prune asks the game's
  warning (with "do not show again", `pruneSpeciesWarning`), removes the species from the tree, merges its history and members into its parent and
  selects it (a species with more present points than the prune point limit stays, as in the game), and is enabled for a species that is neither
  a favourite nor a root, has no members and whose log shows it gone (`totalIndexOfDisappearance > 0`; after a favourite toggle the game's handler
  leaves it disabled until the panel is filled again, which is copied). Escape ends a text field, then closes the species panels, then the
  lineage panel; the panel opens with no species picked; `--lineage-species N` opens it on the N-th species for headless screenshots.
  Core: `SpeciesRegistry::{is_valid_species_name, rename, set_description, can_prune, alive_descendant_count, current_used_span}`.
  **Genes tab** (`SpeciesGenesPanel`, `GeneLineInfo`; 379.52 x 839.61): the species' name, the "Parent species:" row (its button and the gene
  distance to it as a percentage of the species span, `(+196.8%)`, green) and the "Compared species:" row (a dropdown of the species of the graph
  in `dropdown.rs`, default the parent, a root compares with itself, and the gene distance `(2.952)`), then the 35 genes in `geneSettings` order
  (`tools/extract_gene_info.py` -> `gene_info_data.rs`: name, help text, max): the name (LiberationSans 14, its tooltip the gene's help), the
  template's value (3 decimals, flush right in 90) and the signed difference from the compared species (centred in 100, black whose alpha is
  `Lerp(0.15, 1, sqrt(contribution / largest))`, its tooltip the flat / relative / span differences and the contribution to speciation).
  `--lineage-tab N` opens a tab headlessly. Text alignments of prefab TMPs can be read now: the horizontal alignment int sits at `end + 220`
  and the vertical one at `end + 224` after the serialized text (1 / 2 / 4 left / centre / right; 0x100 / 0x200 / 0x400 top / middle / bottom),
  which also corrected the Info panel's root / parent buttons (centred names).
  **The lineage tree** (`LineageTreePanel`, `LineageSpeciesNode`, `SpeciesColumn`, `LineageWindow`; `lineage_tree.rs`, `lineage_view.rs`,
  `lineage_graph.rs`, `time_format.rs`): the graph is the game's tree, from the species logs (`Species history` below). Layout (a port, tested):
  the log-like time axis (`HeightOfPoint`: 75 a point, `linesProgress` of the 11 scales), a node per species with two logged points or more, a
  species too short to draw (`heightSpan` under 20 / 17.5 + 50.5) left out and its children handed to its parent, branches with no living species
  hidden (unless Show Favorited Lineages keeps a favourite), the columns (`BuildColumns`: the heaviest child continues the parent's column, others
  go either side, short leaves are packed into a free neighbouring column; the game's column structs are copied in its lists, so a packed node
  does not change its column's weight, which is kept), the widths by weight (`Scale Important Species`, else equal), 100 between lineages, and
  the viewport fit (`BuildTree`'s zoom: the width, and the tallest living node's height). Window: Ctrl + wheel zooms a sixth of the zoom about the
  middle of the viewport's height (0.14 - 1.5, the least from the graph's size), the wheel scrolls 35 a notch (Shift: sideways), the right
  button drags, the scroll bars (20 wide, handle = the visible share, a press on the track pages) follow and `ScrollRect`'s rules apply
  (clamped to the content, smaller content centred across and on the bottom, a bar hides when the content fits and the viewport takes its room,
  the tree is placed again for the new viewport; all values from the prefab with `tools/unity`). Drawing: the prefab's parts with sprites
  exported by `tools/unity/export_named_sprites.py` (`lineage-*`): the 11-wide bar from 17.5 above the bottom to 50.5 below the top, the link
  (horizontal line, elbow and junction ends picked by which side the parent is on, arrow head and neck), the template symbol, the arrow of a
  living species and the cross of an extinct one, the preview box (`Long_UI`, dark field, the name in Roboto-Bold) scaled to what fits or the
  small point, the time lines across the viewport with "now" / "1h 10m ago" at both edges (`GraphLineTextReference`), the selected species
  outlined in yellow and its box tinted yellow (`highlight` material, `_focus`); everything the viewport's mask would cut is cut by hand
  (`UiDraw::ImageRegion`: a sprite part). A press on any part selects the species (the view does not move: `centerPoint` of a centred rect is
  zero, as in the game); tooltips: the box (name, "Child species of ..."), the link ("Species' Birth", instant), the template ("Species'
  Introduction"), the arrow ("Alive") and the cross ("Extinction") with the time as "3 hours 20 minutes" (`TimeFormat.FormattedTimeValue`).
  "N displayed" and the compared dropdown list the species the tree draws. The Mesh button only shows while the tree is up (and the Tree
  button while the mesh is), at the second place from the right. `--lineage-species N` (any N past the list opens it without a selection),
  `--lineage-zoom Z` and `--lineage-pos X,Y` open the tree headlessly; checked on the 26 species of `testdata/real/saves/lol.zip`.
  **The Settings panel** (`LineagePanelSettingsManager`, `lineage_settings.rs`): the gear button toggles it (it closes with Escape, with the Exit
  button and when the panel opens). A 745.5 x 740 `Long_UI` panel at the overlay's top-right corner (x 1134.5, y 50) over a vertical `ScrollRect`
  (clamped, 35 a notch, the bar 20 wide) whose column is laid out by hand from the prefab's layout groups (padding 20 / 20 / 20 / 35, spacing 20; Unity's
  flexible-width sharing is computed: a slider row gives the name 125 + 2 shares, the value 80 + 1, the slider 200 + 1, the reset button 40):
  the title, "Effective Species Span: 1.500" (`usedSpeciesSpan`), "Lineage Options" (Hide Extinct Lineages, Show Favorited Lineages and the
  "Lineage Tree" group with Scale Important Species: the three are user settings under the game's keys; a toggle builds the tree again, and a
  species that is no longer drawn leaves the species panels as in `UpdateHideExtinctLineages`) and "Species Options" (the span, its scaling and the
  prune point limit: sliders of the dynamic panel's rows, with the default marker, the circle handle and the reset button, edits on the
  Ctrl + Z history). The registry now reads the span and the prune point limit from the settings when it uses them (it cached the span by the
  species count only, so the sliders did nothing until the count changed). The mesh view hides the "Lineage Tree" group
  (`ShowSettingsOfType`). `--lineage-settings S` opens it headlessly (scrolled by S). Not checked in the real window.
  Its three sliders have the value edit fields too (typed values, below; the edit cursor of `ClickToEditHover` is not done).
  Edit opens the Bibite Editor over the Library on a copy of the species' template (`SpeciesPanel.EditBibite`, see the Bibite Editor paragraphs of "UI mechanics").
  Not done: the Help button, the compared dropdown's overflow for names wider
  than its label (they shrink to fit). The GPU window path of the new `ImageRegion` was only type-checked (headless runs use the software
  rasteriser).

- **Lineage mesh view** (`LineageMeshPanel`, `LineageMesh`, `SpeciesRungData`, `UIMeshEventHandler`, `SpeciesPanel.SelectPanel`; `lineage_mesh.rs` the
  layout, `lineage_mesh_view.rs` the drawing, `lineage_view.rs` the window): the top bar's Mesh button switches the graph from the tree to a stream
  graph (`treeOrMesh`, kept as the player left it; the Tree button switches back, `ResetView` puts the zoom back to 1 at the bottom, the Settings panel
  drops its "Lineage Tree" group, the selected species stays selected). `GenerateMesh` is ported rung by rung: the recorded species with a history
  (apparition not equal to disappearance; with "Hide Extinct Lineages" the ones with members, or favourite ones with "Show Favorited Lineages", and their
  ancestors, in the recorded order), "now" at the bottom and a rung at the height of every log point (the tree's time axis with a default step of 50,
  `HeightOfPoint`), every species as wide at a rung as its share of that point's energy of a strip `(viewport - 300) 3^log10(energy / maxEnergy)` wide
  and centred (`GlobalLineageManager.maxEnergy`: `SpeciesRegistry::max_energy`), a species that is extinct ending in a point at its first log without it
  (`AddBottomCap`) and every shape in a point on the log before its oldest present one (`AddTopCap`), the fade end heights (`AddMaxFadeEnd`: a child fades
  into its parent's colour from its top down over its last one to five rungs, and no lower than where its parent starts and no higher than where its
  own children end) and the square for the species' picture (`CalculateDisplayBibite`, at least 50, at most 400). The graph is as tall as the oldest point's line or the view, the least zoom
  shows all of it, the most is 1.5. A shape is painted by hand (no renderer has polygons): the graph is cut into 512 px tiles at the current zoom, painted
  the first time they are seen and kept (64 at most, a selection or a new zoom paints the visible ones again), each row between the rungs' interpolated
  ends, the species' colour (`GenesToColor`) fading to its parent's over the top `(topHeight - fadeEnd) / height` of the shape, with the outline band the
  shader draws (5 graph units black, 10 yellow for the selected species, a slanted edge gets the wider band across the row); the big shapes get their
  navy disc (`BodyPreview`), the species' procedural bibite (`MESH` previewer, scaled `size / 400`) and its name in Roboto-Bold 30.7. The pointer
  tells the shape under it as `CheckPointIsInsideBounds` does and its tooltip is the species' name over `SpeciesRungData.Text` of the nearest rung
  ("5m ago", "Count: 6  (25.0%)", "Energy: 900 E  (25.0%)" - the carriage return of the game's format draws nothing); the tooltip follows the pointer
  along the shape without waiting again (`UpdateTooltip`: a tip whose path starts with `@` is the same target while its path is), tooltip boxes now
  show a text's line feeds as lines, a press and a release on the same shape selects the species, a panel drawn over a shape takes its tooltip, and
  "N displayed" and the compared dropdown list the shapes (oldest first, `OrderByDescending(totalIndexOfApparition)`). The registry keeps its member
  counts at the tick boundary, so opening the panel (and building the mesh) counts them again first (`World::reconcile_species_counts`: a world just
  loaded, or paused, had every species at 0 members), and the mesh's "now" rungs are made of what the species have at this moment
  (`World::species_points_now`). `--lineage-mesh` opens it headlessly. {TESTS} viewer tests (the layout's numbers, the shapes' outlines and colours,
  the tile cache, the hit and tooltip rules, the view's zoom / bars / inverse mapping); checked in headless screenshots of the 26 species of `lol.zip`.
  Not the game's, or not known: the Lineage Mesh Shader's source is not in the decompiled scripts, so the fade is a linear gradient and a selected
  shape changes nothing but its outline (`_Focus` may do more); the outline's edges are anti-aliased sideways only; selecting a species does not pan the
  view to it (the tree's `centerPoint` is zero, so it does not move either; the mesh's `SelectAndFocusSpecies` subtracts the shape's world-space centre
  from the graph's world position and leaves the rest to the ScrollRect's clamp, which depends on where the canvas is - not ported); the pixels go
  through the GPU window path's raw-pixel upload (type-checked only), and nothing was driven with the real mouse.

- **Species Brain tab** (`SpeciesBrainPanel` = `BrainPanelCommon`; `species_panel/brain.rs`, `brain_layout.rs`, the `bibites-brainsort` crate,
  `node_info_data.rs` generated by `tools/unity/gen_node_info.py`): the prefab's panel (padding 10 / 10 / 8 / 10: the header with the name, the Genes
  tab's parent and compared rows - now one `species_header` helper - with the brains' distances, "Brain Diagram:" and the node and connection counts; the
  body with the holder; the bottom line with the "Highlight differences" toggle and the Sort / Save pos. / Reset pos. buttons), as wide as its widest
  row or the holder plus 70 and as tall as its parts. The layout is `FillBrainPanel`: `BrainSorter.MakeGroups`, the heights `Σ(weight + 1) - 1` times 75
  capped at `1080 - 350 s`, the width `400 + 150 (1 - 2^(-hidden / 5))` (and never narrower than the holder was: it only grows), `Sort(groups, 3)` and
  `PlaceNodesAccordingToSort`, or `PlaceNodesAccordingToAnchors` when more than three fifths of the nodes have a saved position (the game never saves the
  outputs nothing reaches, so a brain with many of them never uses its saved positions - kept). A neuron is the 45 px `small_circle_base_icon` with
  its icon (`BrainIconHolder`'s arrays, read from level0: 48 + 13 sprites), the plus / minus of a bias (not for the first output: `Index > NInputs`)
  and its name with "Show Node Names"; a synapse is a strip from its source's middle to its target's, 10 - 16 thick by weight, tiled with
  `synapse_0` (12 x 7, one tile as high as the strip), green / red / faint white, rasterised into one picture of the holder (made once per layout and
  scale). Left-dragging a neuron moves it (inside the holder, Shift snaps to its step), Sort / F1 / F3 / F4 sort again (1 / 6 / 8 times) with new random
  numbers, F2 held steps the hidden nodes, Save pos. keeps the places (in the session), Reset pos. forgets them, the toggle marks the differences with
  the compared species (`BrainDifferencesToIndividual`: new, toggled and changed connections and biases with the game's tooltips; the match of a
  changed weight by index is the game's own and so is `sqrt(|delta| / max distance)`). Neuron tooltips are `UINeuron.SetDesc`'s (name, the node
  information table, function, range, default activation, innovation); a hovered synapse shows its weight. Not the game's: the random numbers (a seeded
  xorshift: the hidden nodes' initial jitter differs), the synapse shader's animation, the highlight glow, the weight text not turned along the strip,
  the saved positions are not read from or written to saves (`template.nodeAnchors` of the species) and the inspector's Brain tab still uses its own
  layout. 340 viewer tests; checked in headless screenshots of three species of `lol.zip` (`--lineage-tab 2`, `--brain-differences`, `--node-names`);
  the pixels go through the GPU path's raw-pixel upload (type-checked only), nothing was driven with the real mouse.

- **Species Position tab** (`SpeciesDistributionPanel`, `SimZonePreviewer`, `ZonePreview`, `SpeciesDistributionItem`, `ZoneMatch`,
  `LogLikeTimeSpanSlider`; `species_panel/position.rs`): the 420 x (536.81 + the zone lines) panel of the prefab: the title, a `Slider` (right to left,
  whole numbers, newest point at the right, 295 x 20 in its dim `Background` panel; the time text "Now" / "5m ago" / "1h10m ago" takes at least 90 off
  the 405 of its row and the slider the rest, as the horizontal layout does; the press jumps to the cursor on the track and keeps its place under the
  cursor on the handle, the value rounds halves to even like `Mathf.Round`), a 400 x 400 navy map with the 300 x 300 preview square in the middle on
  which every zone is drawn as it was logged at the slider's point (the zone as it is now for `timeIndex < 1` and past the end of its log; present
  zones only; the zone preview's flat fills, colours and opacities of the scenario editor's preview), the species' ellipse (`SpeciesDistributionItem`:
  the spreads clamped to 3% - 100% of the world, the correlation turning it, kept within 1.5 world sizes of the middle; white at 0.239, rotated, drawn
  as pixels over the zones), and the "Possible Zone Presence" lines (`ZoneMatch`: the up to five zones that overlap the ellipse by more than 15%,
  the best first, `-name (xx%)`; "None" and "Unknown"); when the species had fewer than three bibites or no spread at that point the map is covered
  by the grey disclaimer. The panel is filled like the game does it - when the species is selected, keeping the slider's value (clamped to the new
  range: from the point after the species' disappearance, or "Now", to its apparition) and again for every slider move; the picture is a snapshot,
  not live. Core: `World::species_point_now` (`new SpeciesDataPoint(species)`), `SpeciesLog::at` (the global index), the zone logs (below).
  `--position-time N` sets the slider headlessly. The numbers were hand-checked (overlaps of 100 / 66 / 37 / 18 %, the axis-aligned and the rotated
  ellipse, the slider's ends and its halves); checked in headless screenshots on `lol.zip` (26 species) and on a save with 901 zones (a map of the
  USA made of rectangle zones: 4 s to load and draw). Not done: the zones' name / biomass / pellet labels (the Zones tab's preview lacks them too),
  the keyboard navigation of the slider; the pixels (zones, ellipse) go through the GPU window path's raw-pixel upload, which was only type-checked,
  and nothing was driven with the real mouse.

- **Species history** (`Species.speciesData` = `LogLikeSpeciesDataArray`, `GlobalLineageManager.LogSpeciesInfo` / `SpeciesTreeCleanup` /
  `PruneSpeciesFromTree`; core `species_log.rs`, `data_bin.rs`, `species.rs`, `world.rs`): the registry's own history (a bounded list of
  (time, count, energy) samples) is replaced by the game's multi-resolution log: scale 0 keeps the newest 12 points, every scale hands blocks of its
  feed ratio, compressed, to the next (the 11 `SerialSpeciesConfig` scales, the last unbounded), one point per species every 5 simulated minutes
  with the count, the energy and the energy weighted position (mean, spread per axis, correlation) of its bibites and eggs
  (`SpeciesDataPoint(species)`, collected by `World::species_samples`). The presence marks (`totalIndexOfApparition` / `Disappearance`,
  `nPointsPresent`) are what the lineage tree is drawn from and what pruning counts; `SpeciesTreeCleanup` is now the game's (a kept species -
  favourite or root - with few points absorbs its children and siblings, least relevant first, until it has more than the limit, even alive ones,
  and one that still has none is pruned too; the members of a pruned species go to the survivor, `World::prune_species` / `take_merges`), the
  record counter stops at 10 000 and counts only while species exist. The game's `data.bin` species block is read on load (game version
  0.6.0a15 and later, `Version.Parse`) and written back (a log that was not touched since loading is written exactly as the game wrote it, the
  others re-encoded with the positions quantised against the simulation size as the game does; a world that was not loaded from a game save
  gets a blank history around its species block once something was logged). Checkpoints carry the log losslessly (species checkpoint version 2;
  version 1's samples become logs). Tested against the 26 species of a real save (`testdata/real/saves/lol.zip`: every payload loads with the
  game's byte length and re-saves with the same structure, counts and energies). The first two `data.bin` blocks (the Graphs streams and the
  bibite tracker) are read and written too, see **Graphs history in saves** below.
  **Zone logs** (`Zone.zoneData` = `LogLikeZoneDataArray`, `ZoneDataPoint`; core `zone_log.rs`, `ZoneData.log`, `ZoneStore::log_all`): every zone
  logs a 14-byte point (position and radius relative to the simulation size as shorts, the fertility `totalGrowth` and the pellet biomass) in
  the same tick as the species (`DataLogger` calls `LogSpeciesInfo` then `LogAllZonesData`), into a log of the species log's shape; a zone is present
  while it has a radius and a fertility, and old points compress weighted by fertility. The log lives on the zone, so it survives the renumbering of
  the zone ids when a zone is removed. The fourth `data.bin` block (`ZoneManager`: `i32` zone id, `i32` length, the log; read from game version
  0.6.0a10) is read on load and written back (untouched logs as loaded); checkpoints carry them in their own section (`CHECKPOINT_ZONE_LOG_TAG`).
  Checked against every zone log of `testdata/real/saves` and of the player's own saves (one has 901 zones): each loads and re-encodes to the game's
  exact bytes (`RUSTIBITES_REAL_SAVES=<folder>` points the test at another folder of saves, read only).
  **Graphs history in saves** (2026-11-06; `DataLogger`, `BibiteTracker`, `DataParallelStreamGroup`, `GKQuantileEstimator`,
  `RollingQuantileEstimator`; core `datalog.rs`, `data_bin.rs`, `World::load_logger_blocks` / `data_bin_for_save`): the first two `data.bin`
  blocks are read on load (game version 0.6.0a9 and later - an older file has another stream format and keeps an empty history) and written again
  from the world's logger, so a loaded save's Graphs panel plots the history the game recorded and a saved one carries it on. Block 0 is the
  progress to the next point (`f32`), the minutes (`i32`) and the eight streams, each behind its length (biomass, count, genes, brain size, age,
  age of death, eggs laid, births and deaths; a stream is the point count `i32`, the width and the scale count a byte each, the floats stream after
  stream, the push counters of all scales but the last and the point count of every scale a byte each); the loaded points go to the start of each
  scale's ring, newest first, as `LoadStateBin` puts them. Block 1 is the births and deaths since the last point (`i32` each) and the estimator
  of the ages of death (the roll counter and three Greenwald-Khanna summaries, each behind a length; the game sizes each slot by the summary's size
  *before* the summary compresses itself and pads the compressed one with zeros, which the core reproduces). The pending deaths are now a counter
  (`DataLogger.deaths_since_log`; it was a difference to `stats.deaths_total`, which a loaded world does not have, and which a checkpoint restore
  left out of step). Checked against `testdata/real/saves/lol.zip` (7 groups of 60 / 60 / 96 / 43 / 5 / 0 points per scale): the logger, species and
  zone blocks re-save byte for byte, the tracker with the same content (its slot padding differs, as the game itself would write the compressed
  size after a load); a fresh world writes the very bytes of a blank game's two blocks (188 and 46). The **genes stream** (`GenesBucketsStreamGroup`, `GenesBucketPoint`, `LogLikeArraySerial`; core `genes_log.rs`) is the hourly
  distribution of every gene over the living bibites and the eggs (20 buckets per gene over a range the population fits in, the points of
  `DataLogger.SerialHeavyConfig`'s seven scales: the last 6 hours, then points made of 3, 4, 2, 7, 4, 2 older ones); `DataLogger.nMinutes`
  is counted (saved and restored) and an hour that fills logs a point. The merge (`CompressGeneBuckets`) is ported as the game writes it, including
  its quirk of laying the source buckets out with the NEW range's bucket width (so counts of points with other ranges are not conserved; the
  real save's merged points show it). A real save's genes element (`lol.zip`: 20 points over the first three scales) loads and re-saves byte
  for byte. Not done: the merge itself could only be checked for its invariants (the inputs of the real save's merged points are gone), a save
  from before game version 0.6.0a9 keeps an empty history, and the species / zone logs still run on their own five-minute clock instead of
  `nMinutes % 5` (a loaded save's phase differs from the game's by up to four minutes).

- **Materials Parameters in the world** (`MenuDynamicSettingsManager.MaterialParameters`): the Energy tab's last group lists the four
  `MatterMaterialSettingsHandle` blocks before the two affinity sliders (the scenario editor's block layout and controls are reused; the captured
  row is re-seated in the world's content area). Sliders follow the cursor like the others, "reset" and the block's "reset all" restore the
  game's defaults, the decay switch shows or hides its two sliders; the edits are on the Ctrl + Z history like the other settings (a drag is one step).

- **Tooltips** (`TooltipTrigger`, `TooltipSystem`, `TooltipHandle`; `PelletTooltipTrigger`, `PelletTooltipHandle`; `tooltip.rs`): every titled
  `TooltipTrigger` of the two scenes (312: 109 menu, 203 world; the release build strips their typetrees, so `tools/unity/extract_tooltips.py` reads the
  raw bytes) is generated into `tooltip_data.rs`, keyed by the GameObject path, which is the `path` of the captured UI nodes: a captured control
  that has one gets a hover rectangle in the frame. The pointer must rest on it 0.5 s of real time (`instant` triggers show at once), the box fades
  in over 0.25 s, hangs 10 px from the pointer towards the middle of the screen (the pivot flips with the quadrant), has the prefab's `base_icon`
  panel (alpha 0.949), padding 10, Roboto-Bold 21 header and Roboto-Regular 18 text in black, and is 500 units wide with wrapped text when the title
  or the text has more than 80 characters (`characterWrapLimit`). A trigger with `isUITooltip` false on a UI element never shows (the game's
  `Update` hides it at once; e.g. the Load Game panel's delete button). With no UI under the pointer, resting on a pellet that no bibite, egg or
  tower covers shows the pellet box: the material, "Amount: x u²", "Energy: x E", "Mass: x g" (SI) and, for a perishable material, "State: fresh
  (x s)" or "State: decaying..." with the rotted part left out of the amounts (all formats from the prefab's `FloatValueTextHandle`s). A dialog's
  blocker hides all of it. The generated rows of the dynamic panel's Simulation / Zones / Bibite / Energy tabs show the setting's name and help text
  on their label (`SettingSlider.UpdateText(Name, HelperText)`; all 92 settings of `setting_ui_data.rs` have a help text), and so do the rows of the
  zone items, the default bibites and the material blocks (33 more, `tools/extract_item_help.py` -> `item_help_data.rs`; the size fields show the
  text of the size type in use, the position and the growth / spawn-logic dropdowns have none, as in the game). The title menu's controls show theirs
  too (the menu frame's hover rectangles feed a timer of their own, and the box is drawn over the menu texture by the GPU path), and so do the name
  labels of the Settings screens' rows (46 help texts of `UserSettings.cs`, `tools/extract_item_help.py` group `user`; the path settings, the
  autosave-in-load-panel and a few others have none, as in the game). Not done: the title-menu screens replayed over a world (Settings / Library /
  Load Game opened from the game) have no tooltips, texts the game changes at run time (`UpdateText`), and the `²` glyph in the software renderer (the
  GPU path draws it).

Known gaps found by this audit and not done yet (all confirmed in the C#):
- Stats panel: the Parents line (`parent1Button`/`parent2Button`, `SelectParent`) is not drawn (the capture had no parents; the prefab shifts the
  rows below it), Revive / Edit exist only as far as the old mapping goes; Biology panel buttons are unmapped (the Brain panel's Sort, Expanded View, Help and Highlight differences work now, see "Expanded Brain View"); graph scale buttons.
- Status bars: done 2026-11-06 (see "Status bars" in the Egg inspector section); *Show Targets* draws its labels since 2026-11-06 (`target_weights.rs`, `World::fov_targets`; the weights, 5 % threshold, sqrt fade, offsets and row layout are from `FieldOfViewDisplay` / `WeightDisplay` and the prefab; checked in a headless screenshot and unit tests only: not driven in the real window, no integration test of the View-panel switch yet, plant/meat icons are the UI atlas's `medium_plant` / `medium_meat`, the prefab's procedural pellet sprites are not drawn; in `--sim-thread` mode the HUD labels need the world, the pointers do not).
- Settings (audited 2026-11-06 against `UserSettings.cs`, the captured Settings rows and the viewer's use of each label): the rows that were inert are
  now **Allow Easter Eggs** (wired: `SimSettings.allow_easter_eggs` / `easter_eggs_period` reach `BibiteGenes.Mutate`'s rare roll: a mutated child may be born
  radioactive, rainbow, producer or reaper; the app hands the player's values to whichever world runs, the period is the player's stored
  `EasterEggsProbability`) and **Show Difference** (`showBrainDifferences`, the Brain tabs' toggle, read from the stored value). **Fullscreen** now follows the
  setting: a change (Settings screens of the title menu or the world, Reset included) reaches the window loop, which calls
  `miniquad::window::set_fullscreen` (`UserSettingsManager.UpdateFullscreen`: `Screen.fullScreen = val`); type-checked and unit-tested for the setting's
  side only, **not tried in a real window**. The window now opens as the stored value says (fullscreen by default, as in the game); whether
  the port should also start fullscreen is the owner's call. **Automatically Detect Screen Resolution / Screen Resolution / UI Scale** now work (2026-11-06): the game's
  `CanvasScaler.scaleFactor = ScreenResolutionFactor x UIScale` is `UiFrame::scaler`; the canvas is the window over it (`ui::canvas_size`), and the capture's
  absolute boxes (`native_rect`) are shifted by the first non-stretched anchor of their path (`FrameContext.shift`), so panels stay at their corners at
  any window shape or scale (verified with headless shots at 1280 x 800 and UI Scale 0.5; windows that are not 16:9 had panels left at the top left before).
  Still inert: **Show Experimental Background / Texture Color** (a background texture), **Easter Eggs
  Probability Period** (the slider row; the stored value is used), **Cloud Diagnostic / Analytics Consent**. View toggles are not written back to the
  settings file.
- Action warnings (`ActionWarnings`, `PopupManager.DisplayActionWarning`, `WarningHandle`): the popup of the game's `WarningPopup` prefab
  (`warning_popup.rs`: title, wrapped text, the "do not show again" checkbox, Cancel left / proceed right, over the 0.4-alpha `screenBlocker`; the
  `Popuholder` scales it 1.35 in the world and 1.0 in the title menu) is used by the delete-save confirmation (in the world and in the title menu's
  Load Game panel; the title menu's old hand-drawn box is gone) and by the scenario editor's launch warning. Ticking the box and pressing the proceed
  button switches the warning off for good (`WarningHandle.Dismiss`), Cancel remembers nothing, a switched-off warning runs its action at once; the
  flags are the game's `PlayerPrefs` keys (`saveDeletionWarning`, `startScenarioAfterEdit`, ...): the installed game's own flags are read, this port's
  changes go to `game_user_settings.json` and win. The Settings screen's **Reset Warnings** button now does what the game's does
  (`UserSettingsManager.ResetAllWarnings`: the three warnings of `allInfoShown` are shown again; the file and workshop ones are not in that list) instead
  of resetting every preference / being unavailable. The prefab was never captured in a running game, so the geometry (400-wide popup, padding
  10 / 10 / 5 / 10, spacing 10, 20 x 20 box, 80 x 30 buttons, Roboto-Bold 24 / Regular 18 / Bold 20 in ink (30, 31, 30)) is derived from its serialized
  values and not compared with a screenshot; the checkbox label is Liberation Sans 16 white in the prefab, which the game's outline material draws
  dark and heavy (drawn as dark bold here). The Library's Delete button uses the `fileDeletionWarning` one (see the Library paragraph above). Not
  done: the species **Prune** warning (the species panel has no Prune button yet), the workshop one (no workshop); Escape does not close a
  popup in the world (it does in the title menu). The other title-menu confirmations (scenario editor exit / overwrite / delete) are the captured
  `DialogPopup` re-laid out for the menu's unscaled `Popuholder` (400 wide, 80 x 30 buttons, 24 / 18 / 20 pt text; the capture was taken in the
  world at 1.35, so the replay scales the font sizes and the layout by `popup_scale`) - checked by the numbers of the layout, not on a
  screenshot of the real menu. Verified by unit tests (flag layering
  and persistence, the cancel / confirm / skip / reset flows in the world, the title menu and the scenario editor, the prefab's layout numbers) and
  the heavy frame tests, not driven with the real mouse in the window.

## Information panel (the "Info +" tab, `InformationPanel`)
`crates/bibites-view/src/information_panel.rs`, replacing an invented dark panel (proportion bars, "Graphs" button, SI-prefixed numbers). Geometry and
fonts are the prefab's (level2 `UICanvas/MainUI/Layer1/Panels/Right Panels/QuickStatsHolder/InformationPanel`, the row prefabs `sharedassets2` 172
`SpeciesInfoHandle`, 183 `wedgePrefab`, 195 `TagElementHandle`): a `base_icon` panel (alpha 0.949) hanging from the top-right corner of `Right Panels`
(10 in from the right, 80 down), title "Information and Statistics" (Roboto-Bold 26, centred, black), the 30 x 30 close X, "Entities:" with the
`count` / `energy` button (60 / 75 wide), five wedge rows (Total, Bibites, Eggs, Plants, Meats; the energy section adds Free first: Roboto-Bold 20,
a 20 x 20 `base_icon` colour square except on Total) next to the 200 x 200 pie chart, "Top Species (N of M active species) :" over up to five rows,
"Tags:" over the scrolling tag list (`Long_UI` frame, `ScrollRect`). The prefab's stored sizes are stale (the panel is saved inactive), so the layout is
computed the way Unity does it from the groups' padding / spacing and the `LayoutElement`s (panel `VerticalLayoutGroup` padding 20 / 20 / 10 / 20, spacing 20,
`ContentSizeFitter`): the width is the widest section's preferred width (wedge rows 20 + 5 + label + 5 + max(150, value) plus 20 plus the pie: about 508
with real fonts), the height 10 + 30.47 + 20 + 233.44 + 20 + species + 20 + tags + 20 (437.67 without rows, the prefab's stored value, is a test).
- **Data:** `ZoneManager.CountBiomass(1f)` calls `UpdateBiomass` every second of REAL time whether the panel is open or not; here it refreshes while open
  (at once when it opens). Bibites exclude corpses, eggs are separate, "Free" is the zones' free biomass, plants / meats come from the pellet counters;
  `Species.count` / `Species.energy` and `BibiteTag.count` / `BibiteTag.energy` are computed from the members (a species' count is its living bibites
  plus its eggs that are not aborting; a tag's energy is the bibites' `totalEnergy` plus the non-aborting eggs' energy), NOT the registry's `stats.energy`
  (which the port keeps only for the five-minute species log). "Untagged" is a pseudo-tag for the bibites and eggs with an empty tag, always the first item.
  The numbers are plain digits (`FloatValueTextHandle` with `contract` off: no SI prefix, precision 0, suffix "E" on energies).
- **Sorting:** species and tags by count, or by energy while the energy section shows, descending and stable (tags keep the order the items were
  made in, `tagItems`); the order only changes at the next refresh, as in the game. Species: the first five, stopping at an empty one.
- **Pie charts:** `PieChartHandle.UpdateValues`: wedge j is a clockwise radial fill from the top of `1 - (the share of the wedges before it)`, painted in
  order over the dark `Circle` (wedges 3 units inside it); with nothing to share only the first wedge shows. Software-drawn, cached per picture.
- **Clicks:** a species row -> `UserControl.SelectRandomBibiteOfSpecies` (a random living member, an egg of the species when there is none);
  a tag row -> `SelectRandomBibiteOfTag` (a random living bibite OR egg with the tag, `Untagged` meaning none; eggs of any state). Ctrl held at the click
  arms `toRepeatOnDeath` for that species / tag (`Reselect::Species` / `Reselect::Tag`), which then follows the same corpse / delay rules as Ctrl+R.
  The `count` / `energy` button swaps the sections and the sort key. The close X (`InformationPanel.ClosePanel`) and Esc close it; while it is open
  `OpenButtons` (the Info and challenge tabs) are hidden (`onOpen` / `onClose`).
- **Tag list scrolling:** `ScrollRect` vertical, clamped, sensitivity 35 per wheel notch; the viewport shows at most seven rows (`15 + 30 x min(7, n)`),
  the scroll bar (20 wide, `AutoHideAndExpandViewport`, spacing 25) appears only when the content (`12 + 31 n`) does not fit, so from four tags on.
  The wheel over the list scrolls it instead of zooming the camera; the scroll bar's handle can be dragged (it keeps where it was grabbed) and a press on
  the track pages towards the click by the handle's length (once: the game repeats while the button is held).
Not done: dragging the rows themselves to scroll (`ScrollRect` drag), the track's press repeat, hover / pressed tints of the buttons and rows, the wrapping of a name
longer than its 150 / 120 wide box (it is cut), clipping of the texts of a row that is only partly in view (they are left out), the title bars' white texts
are as the prefab says (white on a 39 % `Background`: hard to read on the light panel, left as is). The geometry, the data model, the clicks, the Ctrl
repeats, the sorting, the refresh timing and the scrolling are covered by 27 unit tests (one of them, in `main.rs`, drives `route_ui_frame` with the
wheel, the handle drag and a Ctrl click); the pictures were checked with headless screenshots only
(the real game's panel was not captured, so there is no reference to diff).

## Inspector Stats panel in the state of its target (2026-11-05: parents, traits, corpse, egg layout)
The captured `default_world_inspector` session is ONE state of `StatsPanel` (a living bibite without parents). The game changes the panel with the
target (`BibiteStatsPanel.FillPanel` / `CheckAndSetAliveOrDead`): a **parents line** ("Select Parent:" + "Egg Layer" / "Contributor" buttons,
`SelectParent` = `UserControl.SelectTarget(genes.parent1 / parent2)`) when `parent1 != null` (a parent that rotted away is a destroyed object: no line), a
**traits row** (RAD / CUTE / SOURCE / REAPER and, for a corpse, DEAD), the **egg section** instead of the bibite's rows for an egg, and the **button
column** (`hideOnDead` = Kill, Lay, Edit; `showOnDead` = the DEAD badge and Revive; an egg and a challenge lose Lay and Revive). Each shows or hides nodes of
`HorizontalLayoutGroup` / `VerticalLayoutGroup` hierarchies, so the rest of the panel moves. Done with a layout SOLVER instead of per-state hacks:
- `tools/unity/gen_layout_data.py` dumps a prefab hierarchy (active flags, RectTransforms, layout groups, `ContentSizeFitter`, `LayoutElement`, TMP texts with
  font / size / bold, `Image` preferred sizes) to Rust (`stats_layout_data.rs`, generated, `#[rustfmt::skip]`); `unity_layout.rs` solves it the UGUI way
  (`CalcAlongAxis` totals bottom-up; `SetChildrenAlongAxis`: preferred / flexible shares, force-expand, alignment, padding, spacing; `LayoutUtility.GetLayoutProperty`:
  a `LayoutElement` value of 0 or more beats the group's / text's / image's, equal priorities take the maximum).
- Validated against the capture: the solver reproduces every group-placed rectangle of the captured Stats panel to within 0.6 units (and the panel's size to
  within 1) with the game's fonts, except the tag row's title (TMP kerns "Ta"; the exported font JSONs carry no kerning table, 7 units off). TMP specifics measured on
  the capture: a bold style (`m_fontStyle` is at `end + 216` in this build, NOT `end + 168`) widens every advance by 1.1277 on LiberationSans, trailing spaces do not
  count, a text's height is `ascentLine - descentLine` (no line gap).
- `stats_states.rs`: the state (`StatsState::of(app)`), the solve for the captured state and for the target's state, and the DIFFERENCE moves the captured rectangles
  (a native capture's screen boxes shift, a v1 capture's local rectangles move), so the captured state stays exactly as captured; a node the capture had hidden takes the
  state's rectangle. The species name and the tag text are measured too: a long species name widens the panel as in the game.
- Wired: the parent buttons (`ControlCommand::SelectParent`), Revive (`ReviveCorpseButton`), the visibility of every node of the panel in `emit_node_inner`.
Core / io (the parents exist now): see "Parent / child ids" (commit 88b3f59). The egg inspector is in the next section. Not done: tooltips of the trait
badges, `Alive Children` for a loaded save that lists children in `eggLayer.children` (derived from the parent links instead).

## Egg inspector (2026-11-06: `GUIManager.OnTargetChanged`, `BibiteStatsPanel.FillPanel` / `UpdateEggStats`, `EggHatching`)
Selecting an egg used to show an invented box ("Egg n", "TAG:", Set TAG / Clear TAG / SAVE egg, no tabs). It is the game's inspector now, from the same
captured panels as a bibite's, with the egg's own data (`SelectionPanel::collect_egg`; `SelectionPanel.target` says bibite or egg):
- **Tabs:** the tab buttons show without Biology (`OpenBiologyPanelButton.SetActive(false)`); the row is a `HorizontalLayoutGroup` (60 wide, 10 apart), so
  the Brain button takes Biology's place. Digits 1 / 2 / 3 = Stats / Genes / Brain, 4 nothing (done before). An open Biology panel hands over to the Stats
  panel when an egg is selected (`BiologyPanel.FillPanel` -> `OpenBibitePanel(StatsPanel)`), and Stats is the chosen panel from then on. Escape peels the
  panel off first, then the egg; the camera follows the egg (`select_egg` sets it, every caller did by itself before).
- **Stats panel, egg section** (`UpdateEggStats`): Generation, Size (`eggArea` = `5 sizeGene^2`), Mass (`eggMass` = `areaAtBirth x bibiteMassDensity`), Total
  energy, and the two bars: "Hatch progress" (`hatchProgress` over `hatchTime`, red) and "Energy Stored" (`freeEnergy` over `energy`, orange). A bar's fill
  is `InfoSliderHandle.ScaledX`: a cap of `8 / ppu` (`ppu = max(targetppu 0.5, 8 / width, 12 / height)`, 16 for the 200 x 26 bars) plus the share
  `min(|value / scale|, 1)` of the rest; the prefab's `bufferFactor` / `bufferValue` are 0, so the scale only grows to the value. The numbers are the
  prefab's `FloatValueTextHandle` formats (read from level2). The parents line, tag, species and the buttons were done with the Stats states (Kill aborts
  the egg, Save opens the dialog, Edit does nothing like the game's null body, no Lay / Revive).
- **Genes and Brain tabs** read the egg's genes and brain (`BibiteGenes` / `NEATBrain` are components of the egg too): the species line and its
  distances, the difference column, the node and connection counts and the diagram. **Status bars** (*Show Status Bars*): health = `HatchRatio x
  totalRequiredEnergy` over `totalRequiredEnergy`, energy = `freeEnergy` over `energy` (`UpdateRealTimeUI`); the bars are placed, scaled and filled live
  (see "Status bars" below).
- **Found on the way (bibites too):** the Stats values were printed with `{:.2} u` and a made-up "12m 5s" for the time alive; the prefab's handles print
  `ScientificFormat`, with the SI prefix of the number's decade when `contract` is on (0.5 u/s is "500.00 mu/s", a 0.8 g bibite "800.00 mg", 1234 u2
  "1.23 ku2") and plain "725s" for the time (`stats_values.rs` has every format). The species line says "None" without a species like `FillPanel`. The
  native loaders gave `Node.active` the hierarchical flag, so the egg section (captured inactive) had no active node for the layout solver and
  drew nothing; `active` is the node's own flag now.
- Tests: the snapshot, keys, Escape, hand-over, refresh and clean-up (`app.rs`), the frames of the window's own composition (the native capture with its HUD
  sessions adopted) for the egg texts, the bar fills, the buttons and the Genes / Brain tabs (`ui_runtime.rs`), the formats and the fill width
  (`stats_values.rs`). Not driven with the real mouse in the window; the picture was not compared with a capture of the real game (none exists).
  The egg section has no tooltips in the game, so none are missing.
- **Status bars** (`StatusBarsReference`, *Show Status Bars*): the object hangs at the CENTRE of the screen (the camera follows the target), its bottom middle
  `25 x scale` above it, with `scale = targetScale x 200 / camSize` (`targetScale` = the target's `transform.localScale.x` when it was selected: a bibite's
  `d1Size`, an egg's size gene; `camSize` = the orthographic size). The port drew the capture's static pose (right at the capture's zoom only) with full
  bars; the nodes are now moved and scaled about the capture's pivot to where the window's centre and the current zoom put them (texts scale with them),
  and `imgFilled` shows the left `value / max` of its sprite (a bibite's health and energy, an egg's hatch ratio and free energy). Not done: the bars stay
  under a panel that covers the screen centre like any HUD layer (the game draws them on `Layer0`, below the panels: same).

## Inspector Brain tab (2026-11-06: `BrainPanel` / `BrainPanelCommon`, `UINeuron`, `UISynaps`)
The tab drew a simplified diagram (its own placement, dead Sort / Highlight buttons, the panel at its captured size whatever the brain). It is the
game's now, sharing the species panel's diagram code (`species_panel/brain.rs`: `Diagram`, the synapse strips, the neurons, tooltips; `inspector_brain.rs`):
- **Layout** (`FillBrainPanel`): the `BrainSorter` layout (`bibites-brainsort` through `brain_layout.rs`), or the species' saved positions when there
  are enough of them (`template.nodeAnchors`, what the species panel's Save pos. keeps); made when the tab shows for a target and kept while the target
  stays (also with the tab closed and opened again), dropped with the target. The holder keeps the width it was given and only grows
  (`holder.preferredWidth = width`, `minHeight = height`; the prefab's holder is 580 wide, `preferredHeight` 75, the 300 of the capture is the runtime
  `minHeight`). Works for an egg's brain too.
- **The panel follows the diagram and the species name**: `panel_layout.rs` solves the panel's prefab (`brain_panel_layout_data.rs`, generated by
  `tools/unity/gen_layout_data.py` from level2 `BrainPanel`) with the unity layout solver for the captured state (holder 580 x 300, the captured name) and for
  the current one, and the difference moves the captured screen boxes: a taller diagram pushes the bottom line down and lengthens the panel; a species name
  longer than "Basic bibite" (all of the usual two-word names) widens the species row and moves its distance. The solver takes a `LayoutElement` the code sets
  at run time (`Request.elem`). The first picture of this had the panel 225 px too tall: the prefab's holder is 75 high and only the capture shows the 300.
- **Live**: neurons are tinted by the brain's current value every frame (`UINeuron.Update`; the species panel shows the template's), labels are the
  captured ones (14 pt under the neuron's 1.28571 scale, dark: the white outlined TMP label); the info row counts the nodes the diagram draws
  (`usedIndexes.Count`, not every node) and every connection.
- **Controls**: Sort (`SortN()`) and the keys F1 (sort), F2 held (`SortHiddenOnce`, right Ctrl = scalar 0), F3 (6 times), F4 (8 times) while the tab is open
  (each `BrainPanelCommon.UpdatePanel` reads them: both Brain tabs sort when both show); a neuron can be dragged (Shift snaps it to its step); a neuron's
  tooltip (`UINeuron.SetDesc`) and the weight of the synapse under the pointer; **Highlight differences** (`showBrainDifferences`, one setting for both Brain
  tabs) marks how the target's brain differs from its species' template (`species.BrainDifferencesToIndividual`: new connections, toggled, changed weights,
  changed activations, with the tooltip numbers); its check mark follows the setting, which starts from the player's stored value
  (`showBrainDifferences` in the game's PlayerPrefs, off by default; the viewer does not write the game's registry, so a change lasts for the session).
- Tests: the layout lifecycle (fill when the tab shows, kept across closing the tab, replaced with the target, dropped with it), sort / keys, highlight
  against a changed weight, an egg's tab (`app.rs`); the frames of the window's composition for the controls at the layout's places, the info row, the
  check mark, the panel growing with the diagram and the species row widening (`ui_runtime.rs`); a headless screenshot of the basic bibite's tab agrees with
  the capture (same layout, same panel size, ticked toggle).
- Not done: hover and pressed tints of the tab's buttons (the Expanded View button opens the Expanded Brain View since 2026-11-06), the synapse shader's animation, the node-value tooltip (`showValueOnHover`
  is off in this panel anyway). The Genes tab's species row has the same hierarchy: its `SpeciesText` has a minimum width of about 258, so names up to that
  fit and the captured layout is right for them.

## Achievements (2026-11-06: `AchievementManager`, `Achievement`, `ChallengeStarsAchievement`, `UserActionCount`, `AchievementPopupHandle`)
The port only READ the game's unlocks (the Achievements screen); nothing ever unlocked. `achievements.rs` is the list (the 8 of the simulation and 3 stars of each
of the 4 challenges, titles and descriptions from the C#), the unlock rule (`Achievement.Trigger`: nothing if it is unlocked, else unlock + the popup), the
counters (`UserActionCount.Trigger` returns the count after the save PLUS ONE, so `> 100` is the hundredth save) and the popup; the unlocks are stored in
`achievements.json` of the state folder (the registry is never written; the installed game's flags count as unlocks too, `GamePrefs`), and the title menu's gallery
reads this port's unlocks as well (`achievements::shared_unlocked`). Triggers, each read from the C#:
- `ACH_SIM_7DAYS` (`TimeKeeper.Update`: the simulated `hours` in 168 .. 170, once per session) and `ACH_SIM_1FPS` (a minute of real time and more than ten
  frames with the measured frame rate under 1.5 while the game runs and after the first measurement), from `App::advance`;
- `ACH_BIBITE_1DAY` (a living bibite with `timeAlive >= 86 400`) and `ACH_BIBITE_100CHILDREN` (100 in its children list): the bibites' own updates check them
  in the game, the app looks at the world once a second of real time (not while the world runs on the simulation thread); `ACH_BIBITE_EINSTEIN` (a natural birth
  with 100 hidden nodes or more): the core counts such hatches (`World::take_einstein_births`, not part of the simulation's state);
- `ACH_USER_SNAP` (the confirmed Kill Half, once per session), `ACH_BIBITE_EDITOR` and `_100` (a save with `anyChange || brainEditor.anyChange`), and the
  challenge stars (`TrySetHighScoreOfChallenge`: one achievement for every star attained, from the challenge's result).
The popup (`TopLayer/TooltipCanvas/AchievementPopup`): a `base_icon` panel 13 from the bottom right corner, the unlocked icon (`ACH_*_1`, else the placeholder) in a
108 x 108 border, the title (Roboto-Bold 24) over the description (Roboto-Regular 24) wrapped to the title's width (the description's layout element asks for a
preferred width of 0); 5 s at full alpha, then half an alpha a second unless the pointer is on it; a new unlock replaces it; the panel takes the clicks on it.
*Checked by* 13 tests (the list and keys, the unlock / store / game flags, the popup's timing, the counter, the layout, each trigger through the app) and a
headless screenshot (`--select-first --achievement ACH_BIBITE_EDITOR`). *Differences:* the popup's geometry comes from the prefab's layout numbers, not from a capture of a
real popup (none exists); with the world on the simulation thread (`--sim-thread`) the 1-day / children / Einstein checks do not run; `Achievement.Sync` with Steam
is not modelled (no Steam).

## Challenges (2026-11-06: `ScenarioSelectorPanel`, `ChallengeManager`, `ChallengeTrackerPanel`, `ChallengeEndPopup`)
*Audit.* A challenge started from the title menu used to load the scenario as a plain world: `main.rs` only set `challenge_params` (restrictions), no
`ChallengeSession` existed, so there was no scoring, no tracker, no end popup, no stars, no stored best, no achievement; only an in-app "challenge picker"
(the game has none) could start a session, and it spawned one extra champion at the camera on top of the spawn row's `minimum`. The champion section of the New
Game screen was missing, and the star lines overlapped a long description.
*Done* (`challenge_run.rs`, `challenge_menu.rs`, the menu and `app.rs`): the New Game screen's challenge list (`SwitchList(false)` forgets the champion;
`ParametersPanel` is `showWithScenarios` so the scenario modifiers and their "Start Simulation" are gone) shows the `ChampionSelectionPanel` in the place of the
modifiers (both have a flexible height of 1, so it is as tall as they were; its inner layout is solved from the prefab: padding 25, the title, a spacer, "Select
Champion" (410 x 57.5) or the champion box (picture 110, name Roboto-Regular 40 white, "Select Other" 70) and the bottom row with the bug-icon warner, "Start
Challenge" 400 x 80 and the small `simulation_icon` button), "Select Champion" / "Select Other" open the Library to pick (`TemplatePick::Champion`, "Select Bibite", the
New Game screen is still there after Escape), "Start Challenge" needs a champion (`startChallengeButton.interactable`, drawn with the disabled tint) and starts the
scenario as a challenge (`StartChallenge(true)`), the small button starts it as a scenario with the champion's row (`StartChallenge(false)`). In the challenge list the buttons beside the list are the challenge ones (`showWithChallenges`: `ScenariosButton`,
the `simulation_icon` over the whole 70 x 70 button, "Scenarios", in the place of `Challenges Buttons`, whose grey star is inset 7.5; `New Challenge Button`, "Create New
Challenge", in the place of `New Scenario Button`: the same `plus_icon`). The world is built with the
champion's row in the scenario's own rows before the spawner's first pass (tagged `Champion`, `LoadForChampion`, so `minimum` champions spawn and no extra one), the
session carries `scenario.info`'s name, description and star texts and the champion, and the end stores the result (`TrySetHighScoreOfChallenge`), unlocks the stars'
achievements, pauses ("ChallengeEnd", given back by Continue as scenario), and Retry builds the same scenario and champion again (another seed). The core's template
lookup now takes Windows paths (a champion row keeps the absolute path; `rsplit('/')` could not find it, so the old in-app champion never spawned on Windows). The
description's real number of lines moves the star lines and the high score under it (`Details` is a vertical layout group; the capture has its own text's lines).
*In the world* (`challenge_tracker.rs`, `challenge_end.rs`, hooks in `ui_runtime.rs`, `hud.rs`, `stats_states.rs`, `app.rs`): `ChallengeManager.objectsToHide` is applied
(the pen, the placer and the user actions of the HUD, Kill and Lay in the Stats panel, Save Game and Load Game in the pause menu, which closes over them and stays
centred; the buttons beside a hidden one move: the top-right row is right aligned so the time panel and the chart button move 65 right, the bottom-left row is left
aligned so the view button moves 120 left). The "Challenge +" tab (`OpenChallengeTracker`) hangs 40 under the Information tab, as wide as its text and "+" box need; it
opens the tracker (`ChallengeInfo`: 500 wide `base_icon` at the Information panel's place, padding 20 / 20 / 10 / 20, spacing 20, TMP lines 1.172 x the size: it is
exactly the prefab's 299.55 high for its stored text; the title, the retry and close buttons, the scenario's description wrapped, `SimConditionText` of the exit
condition's first condition ("h:mm:ss remaining" as .NET's `TimeSpan "g"`, the count / tag / species / biomass wordings of the C#), "Current score:" with the score and
"(High is better)" / "(Low is better)", the three star lines lit with the stars attained); the tab and the panel exclude each other (`onOpen` / `onClose`
hide `OpenButtons`), Escape and the X close the panel (it is an escapable `UIPanel`), Retry asks "Retry Challenge" first and builds the same challenge again. The end
popup is the prefab's (`Button_UI` slice, the holder's 1.35: "Congratulation!" or "Better luck next time...", the three stars with the middle one 15 higher, the lost text
or the win section's lines - "You completed the <name> Challenge!", "With a score of <n>", "Try to <next star, lower case> next time" under three stars, "This is a new
Highscore for you and <champion>!" or "Highscore: <n> With Champion: <name>" - and "Continue as a Simulation" / "Return To Menu"), its width follows its widest line (never
under 640.4) and its height the lines (389.39 for the five stored ones); it blocks the keyboard (`SetKeyboardBlockFromSource("Challenge")`: no Space, no pause menu).
*Checked* by 23 unit tests (the run, the champion row, the menu's pick and start, the frame's controls, the hidden HUD buttons and their shifts, the tracker's and popup's
layouts against the prefab's stored sizes, the tab / tracker / popup flows, the pause menu), headless screenshots (`--challenge-champion FILE --input SCENARIO.zip
--challenge-seconds N [--open-tracker]`, `--menu --menu-view new --width 1920 --height 1080 --menu-click 528,62 --menu-champion FILE`) and the REAL window (title menu > New > challenge list > Select
Champion > the Library > Start Challenge > the HUD, the tab, the tracker, Escape closing it, then the pause menu without Save / Load, the 20 s "Quick Challenge" to its end
popup and "Continue as a Simulation": the popup goes, the world runs, the hidden controls and Save / Load are back; the data folder was a throwaway one, but the scenario
list and the Library still read the player's folders).
*Not done yet:* the in-app
invented challenge picker is still in the code (unreachable from the real UI); the champion picture, the tracker and the popup were compared with the prefab's numbers
and not with a capture of the real game (none exists); `SimConditionText` keeps the prefab's placeholder text for metric pairs the C# does not word; the champion can
only be a Library template (the Saved tab's `.bb8` files load as genomes, not tried); challenge editing and the workshop buttons are as before.

## Expanded Brain View (2026-11-06: `ExpandedBrainPanel`, `ExUINeuron`, `UISynaps`, `ExpandedBrainParameters`, `ExpandedBrainScroll`)
The Brain panel's *Expanded View* button was drawn and did nothing; it now opens the game's force-directed brain window (`expanded_brain.rs` the model,
`expanded_brain_ui.rs` the screen, `App.expanded_brain`, `PanelKind::ExpandedBrain`, `UiAction::ExpandedBrain(ExAction)`, `ControlCommand::OpenExpandedBrain`;
the captured button is mapped by name in `ui_runtime.rs`). Read from the C# and the scene / prefab data (level2 `ExpandedBrainPanel`, sharedassets2 `ExUINeuron`):
- *Opening* (`GUIManager.OpenBibitePanel(ExpendedBrainPanel)`): the inspector's panel closes, the game pauses (`TogglePauseGame("ExpendedBrainPanel")`:
  the same `panel_pause` as the Graphs and Lineage panels), `UserControl.AllowControl = false` (the hotkeys, Space and the world's wheel / right button stay off),
  every node with a connection becomes a neuron (inputs in a left column, outputs in a right one, hidden neurons at a random x between 20 % and 80 %: the
  port's own generator) on a `BrainHolder` of `nMax x graphWidthStep` by `nMax x graphHeightStep` plus 20 000 / 10 000 of padding, zoomed `5 / nMax` (so the
  graph starts 1000 x 750 px in the middle of the window). Exit (`OpenBibitePanel(BrainPanel)`) closes it and opens the Brain panel; Escape closes the settings
  panel first and then the view alone (`BibitePanel.Escape` = `ResetPanelToNone`: no panel opens in its place). The species Brain tab's own button is
  inactive in the prefab (nothing activates it), so it is not drawn. Loading a world closes the view.
- *Top Bar*: the gear (settings), and from the right Exit, Help (`GameManager.brainHelp`: the web page), Reset (`reset()` = `ClosePanel(); OpenPanel()`: new random
  places, the default settings, the graph type, the canvas' place and the open settings panel stay) and Start / Stop (`StartForce` / `HaltForce`, swapping
  places), each 60 x 60 on a `base_icon` (ppum 0.75) with their tooltips.
- *Canvas* (`ExpandedBrainScroll`: clamped, no inertia, sensitivity 0): the right button pans it after 10 px (a neuron under the press does not pan; the canvas is
  where the drag began plus the cursor's whole way, kept inside its limits), the wheel zooms 10 % a notch (0.1 to 5) with `Zoom`'s own maths, which is wrong in the game
  and so here: it converts `mousePosition - screenCentre` as if it were a screen point (the fixed point of the zoom is half a window down and left of the cursor)
  and moves the holder by the UNCLAMPED step. A left press on a neuron that moves less than 10 px is a click (`BFS`: the neurons and synapses reachable
  through hidden neurons stay, the rest drop to alpha 0.02 / 0.01, `RemoveBranches` trims dead ends); further on it drags the neuron to the cursor (it stays still for
  the simulation); a left click that ends on the background or a synapse puts everything back (`resetBFS`). A hovered neuron shows the prefab's "0.00" (the game
  never updates it: `ExUINeuron.OnPointerEnter` does not set `isHovered`) and its tooltip; a hovered synapse is drawn on top with its weight above its middle.
  Neurons are the prefab's 35 x 35 at scale 1.7; the names above them are never shown (`NameText` stays inactive).
- *Simulation* (`UpdatePanel` while Start is on): every frame, with the real frame time (`Time.unscaledDeltaTime`, a stall counts as 0.1 s): the linear graph pushes
  neurons apart (inputs and outputs from their `hiddenPosition`), swaps neighbouring inputs / outputs when the push on their hidden positions passes
  `outputStep x switchingThreshold` (the game uses the OUTPUT step for the inputs too, and the value from the opening for the inputs' loop), lays the columns out by
  rank and attracts synapse ends; the circular graph attracts everything to the middle, repels inputs and outputs from it, neurons from each other and attracts
  synapse ends. `Move` adds the speed to the WORLD position (pixels: local units by `speed x dt / (UI scale x zoom)`), for the linear graph's inputs and outputs to the
  `hiddenPosition` in plain local units; friction is `1 - friction x dt`.
- *Settings panel* (`BrainViewSettingsPanel`, 600 x 800 at (10, 10)): the graph-type button ("Switch To Circular / Linear Graph"), "Toggle Disabled Synapse
  Visibility" (`DisableSynapse`), and the group of the graph type (Linear: Synaptic Attraction, Neuron Repulsion, Graph Width, Height Step, Friction, Switching
  Threshold; Spherical: Synaptic Attraction, Neuron Repulsion, Gravity to Center, Repulsion from Center, Friction) as the same slider rows as the Lineage settings
  (`lineage_settings::Drawer::slider_cells`, now shared: name, value text that opens the typed field, slider with its default tick, reset button, tooltips with the
  settings' `HelperText`); a typed value may leave the range (`canGoOutOfBounds` stays on); the settings go back to their defaults when the view closes
  (`ResetValue`); the panel is a child with its own flag, so it is open again when the view is opened next. The two buttons the prefab leaves inactive ("Reset Branch
  Selection", "Toggle Trim Branches": nothing activates them) are not there, so `toggleTrimBranches` has no UI.
- *Checked by* 14 model tests (layout, zoom maths, force steps, rank swaps, BFS), 11 screen tests (opening and pause, Escape, the top bar, the settings panel and its
  rows, sliders / reset / typed values, closing, click highlight, drag, wheel and pan, the simulation, hover), a routing test through the captured Brain panel's
  button, and headless screenshots (`--select-first --expanded-brain "settings;graph;start;run 8;bfs 0;wheel 960,540 3;pan X,Y;drag N X,Y;disabled;hover X,Y;edit N"`).
  **Not driven with the real mouse and keyboard**, and the game's own screenshots of this panel do not exist (the captured sessions have it hidden), so the sizes
  are the prefab's. *Differences:* the hidden neurons' first places are the port's own random numbers; the synapse shader's animation is not modelled (the Brain tabs'
  synapses are drawn the same way); the neurons' value / sign alphas follow `SetAlphaLow` but the shader's node colour is plain `small_circle_base_icon`
  tinting; a synapse is hovered within its thickness (at least 2 px); the settings panel's scroll view never needs scrolling at these contents, so none is drawn; the
  Escape stack's entry of the settings panel (`PanelKind::ExpandedSettings`) stays when the view closes with the panel open, as in the game, and swallows one Escape press.

## Bibite colours (2026-11-05: "the basic bibite looks blue", "ensure bibite colours are right")
Five separate things were wrong. The numbers below are checked against the game's own captures (`assets/game/reference/default/bibite_closeup.png`,
the lol save's `bibite_closeup.png`) and, for the window, against screenshots of the real window on the player's GPU.
1. **The window drew the art blue.** The extracted part art is ONE blue hue (0.549) with all the shading in its saturation and value; the
   game's shaders recolour it. `--screenshot` (the software rasteriser) recoloured it, but the window's default path only multiplied the art
   with the bibite colour (`draw_texture_ex(.., tint)`), so a light grey basic bibite came out blue. Now the skin / eye / egg sprites go
   through a GPU material by default (`render.rs::GpuHueMaterial`; `--no-gpu-hue-shader` turns it off and brings the blue back). The tint
   travels in the vertex colour (macroquad's `color0` is a BYTE vector: the first version of the shader forgot the `/ 255.0`), the shader
   globals are one uniform set once per frame, so the sprites of an atlas are still one draw call; a shader that does not compile prints a
   warning and falls back to the old multiply. **Checked in the real window (2026-11-05, idle-gated captures, GTX 1660 SUPER):** the basic
   bibite has the game's grey levels (101 123 157 172 178 199 200 214 against the game's 100 123 156 172 177 199 200 214) and cyan eyes
   (70,198,198) (126,236,235) (24,170,169) exactly; the same frame with `--no-gpu-hue-shader` is blue; the lol save's purple bibites match
   the software render to one level.
2. **The recolouring itself was a guess, and too flat.** The pixel programs of `proceduralBibiteShader` and `proceduralEyeShader` were read
   from the installed game (`tools/unity/shader_asm.py`: UnityPy -> LZ4 blob -> DXBC -> `D3DDisassemble`; the shader globals are set by
   `ProceduralSpriteManager.LoadParametersCSV`). For a texel `(h_t, s_t, v_t)` and the material colour `(h_c, s_c, v_c)`:
   `hue = h_t - FromHue + h_c`, `saturation = s_c * s_t`, `widened = VmaxAtS0 + s_c (1 - VmaxAtS0)`,
   `shade = clamp(v_c (widened - VminAtS0) + VminAtS0, min(widened, VminAtS0), max(widened, VminAtS0))`,
   `value = v_t (1 - s_t (1 - shade))` (`skin.rs`; `FromHue 0.55`, `VminAtS0 0`, `VmaxAtS0 0.75` from `parameters.csv`: they were extracted
   but never used). The old hue replacement kept the art's value, so a basic bibite's darkest grey was 142; the game's is 100 (a grey
   colour has `shade` 0.675). The eye shader first turns `_Color` into `HSV(h - EyeOffset, lerp(s, 1, so), lerp(v, 1, so))` with
   `_saturationOverride so = 1` (the material asset) and then shades like the body, which is exactly `GetEyeColor`; a corpse's eyes are
   now exact too (`so = 1 - decay`, colour `HSV(h, 0.01, v)`, `SetCorpseDecay`). Every colour of the two reference captures is reproduced
   to one level of 255 (unit tests in `skin.rs`, and an end-to-end render test).
3. **`GenesToColor`, not the raw genes.** The body colour is `BibiteGenes.GenesToColor` of `ColorR/G/B` (a basic bibite's 0.1 / 0.1 / 0.1 is
   0.9 grey): it keeps the genes' hue and `max - min` but sets the brightest channel to `1 - min` and the darkest to `1 - max`. Fixed:
   the colour killer's candidates (`bibites-core::genome::genes_to_color`; the hue distance is the same in exact arithmetic, the floats are
   now the game's), the capsule fallback when no game art is loaded (it also lifted dark skins, which the game does not), the species legend
   swatch and the selection panel's colour. Vision (`FieldOfView`) still reads the raw genes, as in the game.
4. **The selected bibite's field of view was drawn OVER its sprites** (last in the display list), so the translucent green cone tinted the colours
   of a selected bibite (and of the pellets inside it): teal eyes instead of cyan. `FieldOfViewDisplay` is on sorting layer -2, order 100 (the
   prefab dump): above the zones (-2, order 1), below towers and eggs (-1) and pellets and bibites (0). It is now drawn right after the zones
   (`render.rs`, step 3b). Colour checks must still use an unselected bibite when comparing with a capture that had none.
5. **Found while checking:** the old test for `genes_to_color` had its assertion inside a comment (a `\n` in a `//` line), so it never ran.

Not done (the same pixel program draws them, none is modelled): the virus pattern of a radioactive bibite (`_InfectedRatio`, `_VirusColor`,
a noise gain), the rainbow of an `isRain` bibite and the producer pattern of an `isSource` bibite (both depend on `_RootPos` / `_Dir` and
the time), the reaper (`_IsKiller`: black, alpha - 0.75) and the red flash of a hurt bibite (`colour = lerp(colour, (1, 0.1011, 0),
0.75 * _HurtAmount)`, `_HurtAmount = damagedFlash`, set to `min(2 dmg / health, 1)` on a hit and falling 2 per second; the core omits
`damagedFlash`). Eggs are tinted like bodies (`egg_tint`), assuming the same shader. The window still draws bodies without the tail bend.

## Selection (click, drag rectangle, multi-selection panel)
`crates/bibites-view/src/selection.rs` + `selection_ui.rs`, from `UserControl.EnableMultiSelect`, `SelectionPanel` and the scene's prefab (dumped
with `tools/unity/dump_ui_tree.py`). The left button no longer pans the camera (it did; the game pans with the right button). A press shorter than
0.15 s is a click: on a bibite or egg body it selects that one (corpses too), otherwise the closest living bibite on screen whose edge is within
`orthographicSize x SelectionRadius` of the cursor (`SelectionRadius` = the game's setting, 0.075 by default); clicking empty space with nothing near
does NOT deselect (the game only deselects with Esc, an empty rectangle or Home). A longer press that did not start on a body draws the
`SelectionBox` (translucent `gray_panel2`); on release every bibite, egg and plant/meat pellet with a corner of its collider bounds inside the
rectangle is picked (bibites: union of the Mouth box and Tail capsule bounds from the prefab shapes; centre within 20 % of the window). One bibite
becomes the normal selection, one egg/pellet opens the panel, several open the panel. The game's call order is kept on purpose: opening the panel
and then deselecting an existing target closes it again, so a drag while something is selected only clears the target. The panel follows the prefab
(`base_icon` 397.69 wide at (10, 10), black text 26/20): "N Entities ( X )", one row per kind present with an X that removes the kind, a species list
(up to five rows shown, no scrolling yet) with the same X, Delete (`KillSelection`: living bibites die, corpses explode, eggs abort, pellets
vanish; also the Delete key) and LAY (`LayEggs`, also Insert; only when species are selected). Numbers are a snapshot taken when the selection or a
removal happens (as in the game), formatted with the prefab's `FloatValueTextHandle` settings. Esc closes it. Ctrl+V tags the whole selection.
Not done: nothing of the hotkeys. Tooltips, K (auto panner) and Z (random zone) are done; zones cannot be clicked in the game (their prefab has no collider). The geometry and the call order are covered by unit
tests, which were first executed on 2026-11-04 (two of them were wrong and are fixed).

## Where the viewer saves (the game's own data folder)
Decision (2026-11-04): the interactive viewer writes to the installed game's data folder
(`%USERPROFILE%\AppData\LocalLow\The Bibites\The Bibites`, `StoragePaths::default_content()`), not to `userdata/`, so the original game lists
what this port saves. `Savefiles/<name>.zip` (Save Game panel), `Savefiles/quick.zip` (F5), `Savefiles/Autosaves/autosave_<local time>.zip`,
`Scenarios/<name>.zip` (scenario editor), `Bibites/Templates/*.bb8template` and single-bibite / egg exports (`Bibites/*.bb8`; they used to land in
`Savefiles/`). The port's own files (`user_settings.json`, `challenge_progress.json`, `game_user_settings.json`, the thumbnail cache, the local
`Workshop/` folder and the menu video cache) stay in `userdata/`. `RUSTIBITES_DATA_DIR` overrides everything (use it to keep the game's folder
untouched); the CLI and headless screenshot runs never write to the game's folder. While reading `SaveController` for this: autosaves now use
local time in the file name (they were UTC), are skipped during challenges (they were not), and keep `AutoSaveMaxFiles` + the new file like the
game (cleanup happens before the write there). Safety deviations, because the folder holds a real history (this player has ~1,400 autosaves and
`AutoSaveFiles` = 2000): only `autosave_<digits>.zip` files are removed (the game removes every zip there) and at most one per autosave.
Not verified: that the original game opens a save written by this build in its Load panel (an earlier build's saves were loaded through the
BibitesAPI bridge without errors; `data.bin` is optional in the game's loader) - try one and tell me if it fails.

## Title background video
The title screen's background is a looping VP8 video, not a still: a Unity `VideoPlayer` plays `Assets/Videos/showOff.webm` (1920x1080, 30 fps,
93.7 s, audio not routed) into `BackGround/VideoTexture`, and the scene's Global Volume blurs it (gaussian about 4.4 px at 1080p) and adds a
vignette; the committed `menu_background_*` reference PNGs are that video at t of about 1.77 s with the post-processing. The clip is
`sharedassets1.resource` offset 0 / 13,487,128 bytes in the game's data folder. `crates/bibites-view/src/menu_video.rs` copies it once to
`userdata/menu_video/showOff.webm`, decodes it with ffmpeg in real time (`-re -stream_loop -1`, scale + gblur, raw RGBA), applies the fitted
vignette on a reader thread and the window loop draws the newest frame under the transparent menu texture (title buttons, Credits, Additional
Tools, Achievements, loading; the New Game / Load Game / Library / Settings screens fill the whole screen with an opaque navy root, as in the game:
`BibiteSelector`'s Image is (0, 0, 0.106, 1), `SettingsPanel`'s and `LoadGamePanel`'s (0, 0, 0.122, 1), so no video, no dimmed world and no HUD shows
through their margins; the port used to show the video behind the Library and Settings and the dimmed world with its HUD in the world, found in the real window).
Needs ffmpeg (`RUSTIBITES_FFMPEG`, PATH or the WinGet install) and the installed game (`RUSTIBITES_GAME_DIR`, `%USERPROFILE%\TheBibites`, Steam
paths) or `RUSTIBITES_MENU_VIDEO` / `assets/game/menu_video/showOff.webm`; without them the static captured frame is shown. Headless
`--menu --screenshot` stays static (deterministic). Not replicated: bloom (the real frame has a faint halo around the pellets), the exact start
offset, the audio. Check with `tools/ui_menu_video_check.ps1`.

## Scenario editor (New Scenario / Edit on the New Game screen)
`scenario_editor.rs` + `menu/editor.rs` (+ the shared row builder in `dyn_panel.rs`). The editor works on a draft of the selected scenario
(effective settings with the New Game modifiers and TPS applied, zones, zone groups, animators, challenge). Working, and driven with real
input in the window (open from New Game, toggle, wheel scroll, tabs, slider drag, add zone, delete-all dialog, Save form typing, Start ->
confirmation -> world runs with the edited settings): chrome and row prefabs replayed from the captured editor with live values (Simulation,
Energy, Bibites tabs; World Parameters on the Zones tab), sub-panels, fixed-sum mutation groups, estimation (total biomass / pellet
production / pellets, summed over zones and groups like `ScenarioSettings.TotalBiomass`), the zone preview (flat zone-shader fills, clipped),
Start / Back / Save with the game's dialogs (captured `DialogPopup`: exit warning, "Launching Simulation", overwrite, delete-all, delete
item), the Save form (captured `Saver` anatomy, typed name/description, invalid/official/existing-name rules, writes
`Scenarios/<name>.zip` in the game's data folder), challenge tab (turn into a challenge / back).
Zone items open into their controls (spawn material / distribution / movement / target dropdowns with real lists, fertility,
biomass, pellet size, speed, position, radius / width / height / inside-radius sliders, renew and "size scales with sim" toggles, per-field
reset) with clipping to the viewport and a scroll bar when the list outgrows the tab; the Energy tab's Materials Parameters blocks (plant /
meat / fat / armor) open into the six parameter sliders plus the decay toggle and its two sliders, with reset.
Default bibites (`DefaultBibitesPanel`, `BibiteSettingsHandle`, `BibiteSettings`; `bibite_editor.rs`): the Bibites tab of the editor and the
in-world Dynamic panel's Bibite tab start with the "+" / "Default Bibites" / "Remove All" header and the list. "+" opens the Library as the
template selector (`BibiteTemplateSelectorPanel.OpenWithCallBacks`: the submit button reads "Select Bibite", Back / Esc return to where it was
opened), the picked template file becomes a new row with the game's `BibiteSettings` defaults (in the editor the file also travels inside the
saved scenario zip, like `SaveSystem.AddTemplatesToArchive`; in the world it is registered with the world's template list and the spawner
restarts like `set_bibite_spawns`). An item opens (one at a time, `ItemClicked`) into Tagging, Spawn Zone (All / each zone), Priority (log
slider 0.01-100), Growth At Spawn, Tag (custom tagging only), Spawn Logic, Target Count (log slider 0-500), Mutate Genes and, in a running
world, the "Spawned" counter of "Once" rows; dropdowns open real lists, sliders have the reset button, the delete X removes the item at once
(`BibiteDeleted` asks nothing) and Remove All asks the game's confirmation text.
Not done: the zone items' exact layout (the game's `ZoneHandle` was hidden when captured: the item is a generated light box with the
prefab's slider / toggle rows stacked in one column instead of its two compact columns, no up / down
reorder arrows; the default-bibite items stack their controls the same way), zone-group items and their template editor, the armor
material's pellet picture (sprite name unknown), the decay parameters as a nested sub-panel (they are inline), typing the custom tag of a
default bibite (it is shown only), the template's picture / name preview in the item header, settings animators, challenge parameter
editors (conditions), the preview's zone labels, the Save form's preview image / reset / import buttons, tooltips, value edit fields. The bibite editor was never captured either, so its layout has to be reconstructed. The in-world "+"
flow (window loop: Library overlay in pick mode, then `PickedTemplate`) is covered by unit tests of its parts and headless screenshots, not
yet driven with the real mouse in the window.

## UI replay fidelity (found while checking the editor against the real references)
Three global defects of the captured-UI replay were fixed and measured with `--ui-session NAME --ui-background black.png` against
`assets/game/reference/functional_ui_native/menu_ui/*_ui_on_black_*.png` (mean abs diff: load_game 13.8 -> 3.4, settings_graphics 16.9 -> 2.4,
library 14.9 -> 5.6, new_game 15.6 -> 9.6, editor icon regions 47-57 -> 0.6): exported sprite PNGs are tight-cropped (padded back with
`sprite_trims.json`), `Image.pixelsPerUnitMultiplier` was missing (`image_multipliers.json`), and TMP margins / middle alignment were ignored.
Details and the extraction scripts: `tools/unity/export_sprite_trims.py`, `tools/unity/export_image_multipliers.py`.
Still different in the raw session replays: the New Game preview placeholder checker, some TMP fonts (button labels use another face),
word-wrap of long labels outside the menu paths, and the vertical alignment of tall paragraph boxes is inferred (the capture has none).

## Real-window input harness
`tools/ui_drive.ps1` (dot-source it) launches the light viewer and drives the real window with real mouse/keyboard input
(`Click-Window`, `Wheel-Window`, `Send-Keys`, `Shot-Window`); it needs an interactive desktop and moves the real cursor. First run
(2026-11-04): `--menu` window took ~30 s and 1 GB (the runtime-UI dump was 1.35 GB of pretty-printed JSON); fixed: the dump is now minified (390 MB) and split per session into `assets/game/ui_runtime_native/` (`tools/split_native_runtime.py`, re-run after regenerating the monolith), HUD sessions parse eagerly and the rest lazily: window in ~4 s, ~300 MB, the main menu renders,
a real click on New opens the scenario screen, a real click on Start Simulation starts the Default scenario in the same window, wheel
zoom works.  The New Game modifier labels now wrap inside their boxes like the game's TMP labels.
Game data (prefabs/assets) can be read from the installed game (C:/Users/Storme/TheBibites/The Bibites_Data) with
`tools/unity/find_objects.py` and `dump_go.py` (UnityPy); the RadioZone radius came from there.

## Visual verification (works, use it)
The viewer renders headlessly, so UI work can be checked by looking at PNGs instead of guessing:
```
CARGO_TARGET_DIR=target_light CARGO_PROFILE_RELEASE_LTO=off CARGO_PROFILE_RELEASE_CODEGEN_UNITS=8 CARGO_PROFILE_RELEASE_OPT_LEVEL=1   cargo build -j 2 --release -p bibites-view            # ~1 min incremental, light on memory
target_light/release/rustibites-view.exe testdata/real/saves/lol.zip --width 1920 --height 1080 --frames 60   --select-first --tab stats --screenshot out.png       # --tab stats|genes|biology|brain
target_light/release/rustibites-view.exe --ui-session default_world_inspector --screenshot out.png   # captured session replay
target_light/release/rustibites-view.exe testdata/real/Default.zip --no-menu --frames 450 --screenshot out.png --drag 2,2,1918,1078            # drag selection (+ --drag-hold: rectangle still held)
```
Compare against `assets/game/reference/functional_ui_native/in_world/*_native_1920x1080.png` (premultiplied-alpha UI-only overlays:
composite them over a background before comparing colours). Delete `target_light/` when done.

Found and fixed this way (all confirmed by looking at the rendered PNGs):
- sliced sprites drew their borders at the wrong size (dark button squares, notched panels): borders are now `border * 100 / spritePPU`
  divided by the Image's `pixelsPerUnitMultiplier` (read from the game's assets; the old measured `Button_UI` 0.2x is only the fallback),
  shrunk proportionally when the rect is smaller than the border sum;
- inspector/dialog labels were pale and thin (white outline-material TMP text on light panels, bold LiberationSans);
- the top-right bar-chart button opened the wrong panel; it now opens the full-screen Stats and Analysis (graphs) panel laid out like
  the native reference (data dropdown, six time-range tabs, 26x11 grid);
- in native mode the bottom-left Placer/User actions/View buttons, the pen/gear Dynamic Settings panel, the lineage "not enough data"
  popup, the lineage panel, load/save browser and all curated modals (confirm dialog, dropdowns, tool property panel, challenge
  picker/tracker/end popup, achievements, tools, gene and brain editors) were invisible; they are now drawn (captured panels are replayed
  as overlays of the nodes newly active in their session; the rest are the curated panels drawn over the native layer);
- the Brain tab shows a live diagram of the selected bibite (simplified layout, no BrainSorter anchors).

Still visibly wrong / unverified:
- The inspector's Brain tab: layout is simplified (the species Brain tab has the game's `BrainSorter` layout in `brain_layout.rs`; the inspector's
  diagram still uses its own heuristic), neuron value fills are not animated, the species header box overlaps a long species name.
- Biology tab (2026-11-06): the Internals picture is the game's own `UI/InternalsPreview` program, the organ map (`internsMap`) its `UI/InternalsSections`
  program, the acid over the stomach `UI/StomachAcid`: translated mechanically from the compiled shaders (`tools/unity/dxbc_to_rust.py`, output
  `internals_shaders.rs`), so the pictures are the game's. The constant-buffer slots were matched by reading the programs side by side (checked: the
  stomach the port computes for the captured bibite is 26 x 27 cells, the `acidLiquid` material's `_StomachDim`). Hovering a cell of an organ opens its
  pop-up as `BiologyPanel.OnPointerMove` does (map ids 1/5 body, 2 stomach, 3 womb, 4/6 mouth, 7 fat): the stomach's (`biology_view.rs`, with one
  `StomachContentElement` per material), the womb's, the mouth's and the fat's (`biology_popups.rs`: the prefab's layout read from the capture by
  `tools/ui/gen_biology_popups.py` into `biology_popups_data.rs`, the number format of every text from the prefab's `FloatValueTextHandle`) and the body's
  (never laid out in the capture, so its rows are laid out in code). The pellets in the picture move (`stomach_sim.rs`: mass 20, damping 0.1, bounce 0.4,
  400 down above the acid, plant floating and meat sinking in it, swallowed matter entering from the right, digestion shrinking them). The Flow bar is
  `BibiteBody.EnergyFlowRate`; the Health, Energy, Growth and Age fills and the growth / age markers follow the bibite. The sliders that raise their own
  scale (prefab buffer 0) keep the highest value seen since the bibite was selected. The womb draws its eggs (a whole one per unit of progress and a growing one, in the body's colour; places and angles are a hash, the pictures are turned by point sampling per 10 degrees). The Health, Energy, Growth and Age bars open their pop-ups on hover (`HealthDetailsPopUp`, `EnergyInfoPopUp`, `MaturityDetailsPopUp`, `AgeDetailsPopUp`). A dead bibite shows "Integrity:", the "This bibite is DEAD" line and no Energy row (`hideOnDead` / `showOnDead` read from the prefab; the panel below moves by 26.16 - 34). The Flow bar opens `EnergyDetailsPopUp` (ten rate sliders in the prefab's layout, one shared scale starting at 0.25, the Sort toggle; the pheromone row is `pheromoneCost`, the last emission's cost, which the Flow total leaves out; the bar pop-ups open after 0.3 s on their row and close 0.3 s after the pointer left row and pop-up, as `OnHoverShow` + `OnHoverKeep`). Not done: the pellets' friction and rotation (circles with impulses only), the egg shader's growing glow. Only checked in
  headless screenshots and unit tests, not in the real window.
- Scrolling content is not masked (Dynamic Settings rows run past the viewport). Dynamic Settings checkboxes are live (click toggles the
  scenario flag and the checkmark follows it); sliders, dropdowns and numeric values are still the frozen capture.
- Curated panels (save/load, pickers, editors) use the old dark style, not the game's light panels (the lineage panel
  follows the prefab since 2026-11-04, see "Species panel"; the Information panel since 2026-11-05, see "Information panel").
- The reference PNGs named `main_world_settings_panel`/`lineage_species_*` are mislabelled captures (graphs / "not enough data" popup);
  the real settings and lineage panels have no native reference.

## Process notes for the next session
- Do not run `cargo test -p bibites-view` or a workspace/fat-LTO release build on the owner's machine; use `cargo check -j 2` and the
  light profile (`CARGO_PROFILE_RELEASE_LTO=off CARGO_PROFILE_RELEASE_CODEGEN_UNITS=8 cargo build -j 2 --release -p bibites-cli`).
- Work up to the challenge-runtime batch is committed; the `assets/game/reference/**` PNG modifications are unrelated and left uncommitted.

## Pheromones (2026-11-06)
- Graphics: a pheromone spot draws nothing itself in the game; `PheromoneSpot.FixedUpdate` makes `ParticlesMaster` emit particles at it. `phero_particles.rs`
  does the same from the world's clock: every 0.25 s of game time each colour whose strength is still above 0 adds `1.25 x` the "Pheromones Strength"
  setting (default 0.1, the old viewer used it as an opacity) to a counter and the whole part is emitted; a new spot starts its counters at
  `sqrt(strength) x Random(0.5, 1.5)`. The three systems (`Red`, `Green`, `Blue` of `ParticlesSpawner`, level2) are read from the prefab: a cone of 25 degrees
  and radius 1 along the camera's axis (only the sideways speed shows: `speed x sin(theta)`), speed 7-12 / 15-25 / 10-15, lifetime 2-3 s, a flat
  textureless quad turned 45 degrees of size 1-2 / 0.5-1.5 / 1-3 (`Primitive::Diamond`), a two-colour gradient (red to orange, green to lime, blue to
  cyan; blue also shrinks) and an alpha that fades in over the first tenth and out over the last fifth. Nothing is emitted while "Show Pheromones" is off,
  what is in the air finishes its life; a pause stops them. The particles are not saved and are only for the eye.
- Mechanics were already the game's (emission once a second above 0.05, dissipation 0.25/s in 0.25 s quanta, `strength / distance` sensing, red death);
  the spots' trigger colliders are 0.001 (bibites) and 0.1 (towers) wide, so sensing by centre distance is exact enough.
- Blood (`BloodSplatterAtPosition`, `blood_particles.rs`): a body that bursts into meat sprays `(int)baseBodyArea / 20` particles five times 0.05 s apart over a disc of `bodyLength / 2`; each is the 7 unit pixel splat of the `blood particles` sheet (frame follows the prefab's curve, random row), moves out by the radial curve, lives 0.5 s; read from the prefab by `tools/unity/gen_blood_sheet.py`. The material's `Color` mode is `hue/saturation of the particle colour with the texture's brightness`. Not modelled: the particle size cap (0.5 of the screen), drag (the start speed is 0), the particles of one burst moving with the master's later positions. Not done: a check in the real window.

## Batch of 2026-11-11: the population crash on "Branching Desert 2" (bibite vision) - FIXED

**Root cause: a bibite's spatial-hash entry never moved.** `World::register_bibite` put the bibite in the hash at the place it was spawned / loaded / hatched and nothing updated it afterwards
(`sync_bibite_hash` only drops dead ones). The field of view finds the bibites around it with a hash query, so the others stopped seeing a bibite once it had left its spawn place: the bibite
inputs (`BibiteCloseness`, `BibiteAngle`, `NBibites`, `RedBibite` / `GreenBibite` / `BlueBibite`, the Herding steering) fell to zero over a few seconds, for every bibite, and faster for newborns. Measured on the
save: bibites with a nonzero bibite input 317 at load -> 97 after 60 s (the game keeps ~26 %); plants were never affected (they do not move). Evolved bibites that rely on seeing others (fleeing, herding)
walked into each other's mouths: 155 "killed" deaths per 150 s against the game's 38, and the population fell 1202 -> 374 in 1800 s (the game keeps ~1230). Fix: `World::refresh_bibite_hash`, called when the
senses are due (the entries are moved to where the bibites are now, `Physics2D.Overlap*` reads the colliders' current places). After it: 1202 -> 1210 (450 s) -> 1253 (900 s); deaths in 150 s: births 169 / starved 81 / killed 55 /
other 5 against the game's 173 / 88 / 38 / 9. The evolution fingerprint was re-pinned (`0x5879_c7f2_fd9e_395c`); test `the_bibites_hash_entries_follow_the_bibites`; probes in `bibites-io/tests/negative_health_probe.rs`
(`vision_fraction_probe`, `hash_staleness_probe`, `newborn_vision_probe`).

Neither of the user's two suspects was the cause: energy use matches the game (median per-tick energy difference 0.0000 over 80 ticks and 1200 bibites, trace compare), and the Rotate response is the same up to the
physics-step finding below.

**Then made to match the game (same day, owner request "make it match the game"):**
1. *One physics step per tick.* Trace comparison of the game at speed 1 (hidden game, plugin trace, 80 ticks, 1200 bibites) shows ONE physics step per tick: a body's displacement is `dt * v_next` (fit weights p = 0, q = 1 of the start / end velocity;
   ten steps gave 0.37 / 0.63), the torque / force gain was 2-4 % too strong and the decay 0.1 % too high. `PhysicsConfig::default` is now one fine step of one substep; `RUSTIBITES_PHYS="<fine_steps>,<substeps>"` brings back
   the ten-step model (`10,10` = the game slowed to 0.1x: `TimeController` sets `fixedDeltaTime = base * timeScale` below 1x, so a tick is then 1/timeScale FixedUpdates; the earlier "ten steps" measurements fit that). Tests of the ten-step
   machinery pin it with `physics::ten_steps_for_this_test()`. Position error at tick 1 went 0.0158 -> 0.0004 (median), velocity / angular velocity 0.017 / 0.055 -> 0.0000, tick 40 p90 position error 3.55 -> 0.94.
2. *No mouth contact grace* (`MOUTH_CONTACT_GRACE_DEFAULT = 0`, was 8; `RUSTIBITES_GRACE=<ticks>`): bibites with something in the mouth 154 vs the game's 152 (178 with grace 8).
Result on the save over 100 s against the game: energy 314.7 vs 312.5, health 244.1 vs 244.9, speed 32.6 vs 32.4, bite rate and damage rate equal (0.0039 / 0.104-0.108); population 1202 -> 1222 (450 s), 1249 (900 s), 1250 (1800 s) against the game's ~1233 at 450 s.
Deaths in 150 s: births 166 / starved 79 / killed 64 / other 7 against the game's one sample 173 / 88 / 38 / 9 (killed is still ~1.5x; not chased, one game sample). The evolution fingerprint was re-pinned again (`0x2cab_d7bc_8449_03cf`).
Not re-validated with the new step count: the contact softness (`contact_hertz` is capped at a quarter of the step rate, 5 Hz at one step of 0.05 s), grabs (the mouth runs at tick granularity) and the world-level collision tests
(they still pass); the contact-heavy tails of the trace comparison (p90 position error 7 at tick 77) are chaos plus contacts.

Tools added: `RUSTIBITES_NO_FEEDING=1` (the capture skips the per-bibite feeding stream: it was 17 GB for this save), `health` in the Rust trace, `RUSTIBITES_PHYS`, `RUSTIBITES_GRACE`.

**Contacts at one physics step per tick - FIXED to Box2D 2.4's solver (2026-11-11; scripts `tools/parity/bd_contacts/`).** Checked first on `Branching Desert 2` (game trace + per-contact `physics2` rows): no-contact bodies matched exactly, bodies
in contact responded ~40 % too weakly. A controlled experiment then isolated the cause: one bibite with a constant Accelerate (a bias-only brain) driven at a resting pellet (Plant 100 / 400 / 10, Meat 400 / 1500, 5 units off centre) or at a second bibite, the same
save loaded in the game (hidden, tick by tick through the bridge) and in the port (`make_fixture.py`, `game_fix.py`, `negative_health_probe::track_probe`). Findings:
1. The soft step (a Box2D 3 solver) found contacts one tick early: "a contact the step's motion will bring to touching" (`catch_fast_approach`) is how the game behaves at ten steps per tick, but at one step the game's contact only exists once the shapes touch
   (the impulse came a tick later, at 1.7 units further). Now `PhysicsConfig::catch_fast_approach` is off at one step and the contact distance is the shapes' skin (0.01).
2. The soft contact's push-out velocity and relax pass moved the bodies in the contact tick by 0.4 / 0.3 units differently from the game, and the restitution was applied after the positions were integrated. At one step the solver is now Box2D 2.4's
   (`PhysicsConfig::box2d_solver`, `solve_island`'s `rigid` path): velocity iterations without bias, restitution inside the velocity solve, positions integrated with the solved velocities, then 3 Baumgarte position iterations (0.2, slop 0.005, max 0.2).
   The ten-step soft step stays for the game slowed to 0.1x (`RUSTIBITES_PHYS=10,10`; also `RUSTIBITES_BOX2D`, `RUSTIBITES_CATCH`, `RUSTIBITES_SPEC` switches).
Result on the fixtures (aligned by the one tick the harness's first step runs ahead): contact tick velocity 16.13 vs 15.97 (heavy pellet), pellet position 41.57 vs 41.58, 32.43 vs 32.34 (light), off-centre rotation -4.4 vs -4.2 deg and vx -2.7 vs -3.3 (heavy),
bibite-bibite 20.8 vs 20.7 / rotation -5.4 vs -5.2 deg, meat 400 / 1500 within 0.1 u; before, the contact came a tick early and the pellet was 0.35-1.8 u behind. On the save: first-contact velocity-change error 0.57 -> 0.48 (pellets), 1.47 -> 1.26 (bibites), tick-77 position p90 7.5 -> 5.8;
population 1202 -> 1217 (450 s) -> 1201 (1800 s); deaths 169 / 83 / 63 / 11 (births / starved / killed / other) against the game's 173 / 88 / 38 / 9.
Still open: the contact-heavy bodies of the save respond ~35 % less than the game's in aggregate (per-tick |dv| in a pellet contact 1.21 game vs 0.77 port) although every isolated fixture now matches, so the rest comes from something the fixtures do not have (pellet piles,
the mouth's eating / ripping chunks, grabs, brains that react); the game's resting bibite-pellet contacts also lose contact for a tick every ~5 ticks (Baumgarte overshoot) where the port's hold steady; the trace's `contact_impulse` is now half the game's `normal_impulse`
(the old soft-step total double counted: velocities, not the report, are what match). The evolution fingerprint was re-pinned (`0xc2bc_ff31_8241_1f0e`).

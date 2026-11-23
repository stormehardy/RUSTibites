# game_diff — the port's brain against the ORIGINAL game, bit for bit

`braincheck` (the folder above) compares the port's brain with a second implementation written from the same reading of the C#. This folder compares it with the
game itself: every brain tick the installed game makes is replayed in `Brain::step` from the game's own numbers, and every node must come out with the same bits.

```
game_brain_ticks.py   runs the hidden game ONE TICK at a time and writes every bibite's brain before the first tick and after each (game_0000.json, ...) + manifest.json
make_random_brains.py gives every bibite of a save a random brain that uses all 13 node types (and, with HOSTILE=1, weights up to 1e20: clamps, infinities, NaN)
../../../crates/bibites-cli/examples/brain_vs_game.rs
                      replays each tick in `Brain::step` and compares Value / LastInput / LastOutput of every node
```

```powershell
# 1. (needs the installed game + BibitesAPI, about 1.1 GB of RAM; the game is started hidden and stopped again)
python tools\braincheck\game_diff\make_random_brains.py testdata\real\saves\lol.zip rb.zip 1 enable_herding     # optional: random brains
python tools\braincheck\game_diff\game_brain_ticks.py rb.zip out\rb 60
# 2.
cargo run --release -p bibites-cli --example brain_vs_game -- out\rb          # exit 0 = every node of every tick identical
#    --acts   the activation functions alone, from the game's own sums     --sums   which way of adding up the terms the game uses
#    --show N  print N differing brain ticks   --dump f.jsonl  write the activation cases for offline modelling
```

What a pair of neighbouring ticks gives the test: the brain before (all three numbers of every node, the synapses in the game's order), the input nodes' new `Value`s
(what `UpdateSenses` wrote: the game's own senses, so only the brain's arithmetic is under test) and the brain after. The game thinks when `brainTicksCount + 1 >= brainTPS`;
on the other ticks the brain must be untouched (checked too, and the counter's schedule is checked).

## What it found (2026-11-19)

* **The game evaluates a C# expression in double and rounds to float only where a value is stored, passed to a method or returned** (Mono). Sigmoid, Gaussian, Integrator,
  Inhibitory and SoftLatch each hold more than one float operation in one expression; written as `f32` steps the port was one ulp off in Gaussian 32 %, Sigmoid 16 %, SoftLatch 4 %,
  Inhibitory 2 % and Integrator 0.6 % of the nodes given the game's own sums, and the nodes after them carried it on (up to 128 ulps on a Sigmoid, 93 on an Inhibitory node in
  `lol.zip`). Now `brain.rs::activation_function` / `soft_latch` and `math.rs::sigmoid` / `gaussian` do it the game's way. A single float operation is the same either way.
* **With herding off the game still computes the Herding output** when synapses end in it: `ComputeProcessingStructure` skips `NodeOut != 2` (the literal 2 is an input node), not
  node 35. `brain.rs::HERDING_SKIPPED_NODE`.
* The pheromone inputs `(float)Math.Tanh(pheroSum / 5f)` take the double quotient straight into `Math.Tanh` (`senses.rs::phero_sense_value`), and `brainUpkeepCost` is one double
  expression (`BrainUpkeep::energy_per_second`).

## Traps of the harness

* The first ~4 ticks after a load run with another time step (two FixedUpdates of 1/30 s where 1/15 s was asked for: `timeAlive` jumps by 1.7 ticks): `--warm-up` (default 5)
  leaves them out, and any step whose `timeAlive` moved by other than one tick is left out and counted.
* The game's JSON has no negative zero (old Mono prints `-0.0f` as `0`) and writes infinities and NaN as the strings "Infinity" / "-Infinity" / "NaN": the comparison treats
  +0 = -0 and NaN = NaN, and the reader understands the strings.
* Dead bodies do not think; they are left out. Bibites are matched by `body.id`.

## Result of the day it was written

| world | brain ticks replayed | nodes differing before | after |
|---|---|---|---|
| `lol.zip` (35 real brains, herding off) | 653 | 609 ticks | 0 |
| the same with random brains, herding off | 433 | 337 ticks | 0 |
| the same, herding on | 451 | 245 ticks | 0 |
| the same, weights up to 1e20 | 332 | 13 ticks | 0 |
| `shrekvssabertoothtest1.zip` (686 evolved brains, 40 ticks per second) | 7,730 | not run before the fix | 0 |

## sense_vs_game (2026-11-19)

The brain is exact (above). This is the same idea for the senses: `game_brain_ticks.py` with `SAVE_WORLDS=1` (and `GAME_SETTINGS='{"visionLookupUpdateFactor":1,"visionSenseUpdateFactor":1}'` so every brain tick senses) writes the game's own save of every tick (`world_0000.zip`, ...);
`extract_rot.py <dir>` writes the unwrapped physics angle of each bibite (`rot_0000.json`); `cargo run --release -p bibites-cli --example sense_vs_game -- <dir> [--rot-search 3] [--csv f]` loads the save of tick K, runs one port tick and compares the 33 input values and the cadence counters with the game's tick K+1.
Read the header of `sense_vs_game.rs` for what a save cannot give (rotation in rounded degrees, angular velocity, growth, organ areas, spot headings) and how the example works round it. Result of the day: `docs/PARITY_REMAINING.md`, "Senses against the original game".

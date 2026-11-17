# Pellet piles and glancing hits: game vs port (2026-11-11)

Fixtures with ONE bibite (bias-only brain, Accelerate 1) driving into pellets, stepped tick by tick in the hidden game (`game_pile.py <ticks> <fixture...>`,
`OUT=<file>.json`) and in the port (`negative_health_probe::track_probe`, ignored; `RUSTIBITES_PROBE_SAVE`, `RUSTIBITES_PROBE_TICKS`).
They write to `C:/Temp/bd` (paths are hard-coded; copy `testdata`-style saves there: `C:/Temp/bd/sv` = an unzipped Branching Desert save).

* `make_pile.py`, `make_small.py` (chains, rows, trees), `make_jit.py` (hex piles with random +-0.06 gaps: the realistic set), `make_sp*.py` (spacing 4.96 .. 5.06),
  `make_side*.py` (a pellet passed at the bibite's side: a glancing hit).
* `fit.py` scores a set of fixtures (`FIXSET=jit` for the jittered one) from the port's probe exe against the game's JSON; `bias.py` / `onset.py` give the signed
  difference and when it starts; `cmp_pile3.py`, `seq.py`, `seq2.py` print one fixture tick by tick.

Findings: circle-circle contacts have NO margin in the game (a gap of 0.0005 does not push, an overlap of 0.0005 does: `make_sp2.py`), the plant pellet's effective
friction is about 0.7 and not the asset's 0.4. See docs/PARITY_REMAINING.md "Contact response".

# RUSTibites user guide

This guide is for running RUSTibites headless: loading real Bibites worlds,
simulating them fast, saving results the original game can open, benchmarking,
and capturing parity data. It assumes Windows PowerShell and a 64-bit Windows
machine; the engine is cross-platform but paths below are Windows-style.

> Status (2026-10-30): the headless engine loads real saves/scenarios/templates,
> runs the full world tick, writes saves the real game (The Bibites 0.6.3.1)
> loads and runs, and passes the canonical **Default** baseline including the
> closed acquisition/feeding gap (§10.4.1). Checkpoint/resume is bit-exact on
> Default 1k–10k; the `0.1.0` portable package is frozen (see
> `RELEASE_CERTIFICATION.md`). `Ring`/`lol` are historical fixtures only — new
> work starts from `testdata/real/Default.zip` ([TESTING_POLICY.md](TESTING_POLICY.md)).
> Physics trajectories and pellet-seeding statistics intentionally differ; see
> [DEVIATIONS.md](DEVIATIONS.md) and [PARITY.md](PARITY.md).

## 1. Install and build

1. Install the Rust toolchain (stable, 1.85+): <https://rustup.rs/>.
   On Windows also install the *Desktop development with C++* workload (linker).
2. Clone the repository and build the headless CLI:

```powershell
git clone <your-fork-url> RUSTibites
cd RUSTibites
cargo build --release -p bibites-cli
```

The binary is `target\release\rustibites.exe`. The whole workspace builds with
`cargo build --release`.

There are no installer steps and no runtime dependencies; the portable release
zip is described in [INSTALL.md](INSTALL.md).

## 2. Quick start (copy-paste)

```powershell
target\release\rustibites.exe help
target\release\rustibites.exe info testdata\real\Default.zip
target\release\rustibites.exe check testdata\real\Default.zip
target\release\rustibites.exe run  testdata\real\Default.zip --ticks 600 --seed 12345
target\release\rustibites.exe run  testdata\real\Default.zip --ticks 600 --seed 12345 --stats json
target\release\rustibites.exe bench --bibites 2000 --ticks 300 --threads 8
```

`testdata/` is **local-only** (real game files and captures, gitignored). Without
it `info`/`run` fail with a clear file-not-found error; only the built-in
synthetic benchmark (`bench` without `--scenario`) works with no data.

## 3. What can be loaded

| Path kind | Accepted by `info` / `run` |
|---|---|
| Save bundle | `*.zip` written by the game (`Savefiles\*.zip`) |
| Scenario bundle | `*.zip` written by the game (`Scenarios\*.zip`, e.g. `Default.zip`) |
| Genome / template | `*.bb8`, `*.bb8template` (real game files and RUSTibites exports) |
| Settings file | `*.bb8settings` for `--settings` (scenario-independent overrides) |

Real game data on Windows lives in
`%USERPROFILE%\AppData\LocalLow\The Bibites\The Bibites\`:
`Savefiles/` (saves), `Scenarios/` (scenario zips), `Bibites/` (templates and
genomes), `SimulationSettings/` (`.bb8settings`). Copy what you need into
`testdata/`; the command-line tool does not modify the game's folders (the
interactive viewer saves worlds, autosaves, scenarios and bibites there on purpose,
so the original game can open them; see [INSTALL.md](INSTALL.md)).

The tool is tolerant of real-world templates the game itself accepts: shifted
node indexes, `-1` synapse endpoints, duplicate innovations and short old gene
arrays are repaired (see [DEVIATIONS.md](DEVIATIONS.md)).

## 4. Running headless (`run`)

```powershell
target\release\rustibites.exe run <PATH> [options]
```

| Option | Meaning |
|---|---|
| `--ticks <N>` | ticks to simulate (default 1000) |
| `--seed <N>` | RNG seed (default 0). Same seed + same input + same thread count = same world trajectory |
| `--threads <N>` | worker threads, `0` = automatic (default); with more than one the world times its own ticks, inline and on the pool at four levels of eagerness, and keeps the fastest (where the pool pays depends on the machine, so it is measured, not fixed; a world under about 300 bibites ticks on one thread without being timed); `1` = the serial tick |
| `--stats human\|json\|csv` | final statistics format; `json` is machine-readable |
| `--settings <PATH>` | scenario-independent override (`.bb8settings`); save `independents` take precedence |
| `--save <PATH>` | write the world as a game-loadable `.zip` when the run ends |
| `--capture <DIR>` | write a parity capture while running (see §7) |
| `--templates <DIR>` | extra `.bb8template` folder for scenario spawn rows |
| `--quiet` | suppress per-checkpoint progress output |

**Scenario starts:** a fresh `run <scenario.zip>` now matches the game's
`BibiteSpawner` coroutine: one bibite spawns at tick 0 and the virgin spawner
adds one per simulated second until the spawn row's minimum (canonical Default:
10 at t≈9 s; historical Ring uses the same spawner rule). Save loads
(`run <save.zip>`) are unchanged: the saved population is
restored exactly and the virgin spawner is not run. This is documented as a
seeded-vs-unseeded deviation in [DEVIATIONS.md](DEVIATIONS.md) D20.

Templates are resolved from the game's user template folder automatically; add
`--templates <DIR>` for other folders (e.g. a scenario's bundled templates that
were copied out of the zip). Missing template files are reported as warnings and
the affected spawn rows are skipped — the run continues.

Statistics labels include `ticks`, `bibites_start/end`, `wall_secs`, `sim_secs`,
`ticks_per_second`, `realtime_factor`, `bibite_updates_per_second`, `threads`
and `state_fingerprint` (stable across runs with the same seed/threads).

## 5. Saving results (game-loadable)

```powershell
target\release\rustibites.exe run testdata\real\Default.zip --ticks 600 --seed 12345 `
    --save testdata\roundtrip\my_run.zip
target\release\rustibites.exe info target\my_run.zip
```

The Rust-written save is accepted by the real game (verified by loading it with
the game's headless BibitesAPI bridge: no exceptions, matching population,
`data.bin` preserved). Copy it into `Savefiles\` to open it in the game.
Writing templates is done through the io API (`save_template`), e.g.
`cargo run -p bibites-io --example export_template -- my_template.bb8template`.

## 6. Benchmarks (`bench`)

```powershell
target\release\rustibites.exe bench --bibites 2000 --ticks 600 --threads 8 --repeat 3
target\release\rustibites.exe bench --scenario testdata\real\Default.zip --ticks 600 --json
```

Options: `--bibites <N>` synthetic population, `--ticks`, `--threads`, `--repeat`,
`--scenario`, `--json`. Reports per-repetition statistics plus the median table.
The synthetic harness scales the world to the requested population; use a real
scenario to measure a realistic world.

## 6b. Calibrating the machine (`calibrate`, Settings > Calibration)

How many of the machine's threads a tick should use, and from what size of world sharing the work pays, depend on the machine (its cores, how fast the
system wakes one, its memory bandwidth) as much as on the world. The simulation always measures its own ticks and keeps the faster way
(`docs/BENCHMARKS.md`, "The line and the levels are measured"), but it starts from a first guess made on one machine. A calibration looks at the machine once,
on purpose, and hands the simulation a first guess that is right for it:

1. **Workers**: a world of a few thousand bibites ticks on 2, 3, 4 ... up to every thread the machine has, at each of the four levels of eagerness; the most
   workers that are within 12 % of the fastest are the ones a tick uses (the tick is memory-bound from a few workers on, so the times of 3 and of 12 workers differ by the
   noise of the measurement, about a tenth).
2. **The landscape**: with those workers, worlds of 150 to 10,000 bibites tick inline and at each level, round after round and alternating (the cost of a world
   drifts); the pool pays from the size where its best level takes under 92 % of the inline time for good, and the level that was the best at the big sizes is
   the one a world starts at.
3. **Graphics** (the window only): a crowd of bibites is drawn into a picture of its own with the GPU hue shader and with the processor's way, each timed with the
   graphics card made to finish; the shader is the default unless it measured at least half as slow again as the processor's way (a machine with no graphics driver runs it in software).

In the window: **Settings > Calibration** has the check box (*Use Calibration*, on by default), the **Run Test** button (about half a minute; it becomes *Stop*
while it runs, and it will not start while a world is running in the window: pause it or use the title screen) and four lines (what the test is doing, the
processor and the graphics card, what the pool of workers found, what the graphics test found). The result is kept in `userdata\calibration.txt` and is used from
the next start (and at once by the world that is open); it is for a machine with that many threads only. Headless: `rustibites calibrate [--quick] [--save]`
prints the whole table (`--save` keeps it for the window; `--threads N` calibrates for a smaller machine).

What it changes is **speed only**: serial and pool ticks are bit-identical whatever the workers, the level or the mix (the tests compare whole checkpoints), so a
calibration, a stale one, or none gives the same simulation. What it does not do: the tick stays on the processor, because a graphics card's floating point is not
bit-exact with the game's and the results must be; the card draws everything that is drawn (the bibites' skins, the zones, the grid, the Red Death bloom, the dark ring).

## 7. Parity captures and comparison

**Canonical Default (policy):**

```powershell
python tools\parity\default_baseline.py --ticks 600
# game + Rust captures + analysis land under testdata\parity\default\
```

**Historical fixtures (opt-in; not the default baseline):**

```powershell
target\release\rustibites.exe run testdata\parity\bases\ring_parity.zip --ticks 300 `
    --seed 12345 --capture testdata\parity\rust_ring
python tools\parity\compare.py testdata\parity\captures\ring_seed12345\pass-0 `
    testdata\parity\rust_ring --profile default --report human
```

`--capture` writes a parity capture (manifest + gzip JSONL tick series + entity
snapshots). `compare.py` diffs two captures; `--profile exact` is for
game-vs-game replay, `default` is the game-vs-Rust gate, `loose` for
bridge-only single observations. The protocol, schemas and determinism limits
are in [PARITY.md](PARITY.md). Requires the parity capture tooling and a game
capture; both live under `testdata/parity/` (gitignored).

## 8. Determinism and threads

- Same input + `--seed` + thread count gives the same `state_fingerprint`.
- The engine is deterministic across thread counts for tested workloads, but
  thread-count changes can reorder floating-point reductions in future optimised
  paths; pin `--threads 1` when reproducing an exact run.
- The game itself is not bit-reproducible by default (frame-rate couplings); the
  parity docs explain the pinned loading protocol used for captures.

## 9. Common errors and fixes

| Symptom | Fix |
|---|---|
| `file not found` / empty `info` | `testdata/` is gitignored; copy a real save/scenario/genome there, or use `bench` without `--scenario` |
| `unrecognised file kind` | Use a save/scenario `.zip`, `.bb8`, `.bb8template` or `.bb8settings`; old 0.6.3.3+ folder-save formats are unsupported |
| `unsupported: pre-0.6a1 ...` | Legacy genomes outside the supported version floor; use a current-format file or let the game export it first |
| Template warnings during `run` | The file is structurally invalid (not one of the tolerated game quirks); check it with `info <file>` or export it from the game |
| `Access is denied` writing a save | Pick a writable output path; the game's `Savefiles\` folder is read/write for the user, but running processes may hold the file |
| Very slow first run | First build is release-compiled; subsequent runs reuse the binary. Use `--threads 8` (or `0` for automatic) |
| Many `--templates` notes | Extra template folders are optional; scenario-bundled templates are used automatically when present |

## 9b. Viewer and extracted game art

A windowed alpha viewer lives in `crates/bibites-view`
(`cargo run --release -p bibites-view -- testdata\real\Default.zip`). It loads
saves/scenarios/genomes, pans/zooms, pauses/steps, selects bibites, and shows a
tabbed inspector (Stats/Genes/Biology/Brain; click the tab strip or use
`--tab stats|genes|biology|brain`; in-window tab-click routing is still being
wired, while `--tab`/headless `--click` work). Common options:

- `--screenshot PATH --frames N` — deterministic **headless** PNG, software
  rasteriser (no GPU/display needed); `--select-first`, `--center-on-selected`,
  `--ortho-size`, `--fit`, `--no-assets`, `--simple-hud`, `--ui-only`,
  `--runtime-ui/--no-runtime-ui`, `--click X,Y` for HUD A/B.
- `--sim-thread` (or `RUSTIBITES_SIM_THREAD=1`) — **interactive window only**:
  while you only move the camera, the world is ticked on a thread of its own
  and the window draws what that thread publishes, so a big world no longer
  limits the window's frame rate to its tick rate. Anything that needs the
  world (a click, a panel, a selection, the pause...) takes it back first.
  Off by default.
- `--gpu-hue-shader` / `--no-gpu-hue-shader` — the bibites' skin, eye and egg
  sprites are recoloured with the game's skin shader on the GPU in the
  **interactive window**; this is the default, and `--no-gpu-hue-shader` turns
  it off (the sprites are then recoloured on the processor with the same maths:
  the same colours, slower with many bibites). `--screenshot`
  always uses the software rasteriser, which has the same maths.
- **Title/main menu:** a no-PATH interactive window opens the game-style menu
  (Continue/New/Load/Bibite Library/Settings/Quit) using the exact runtime menu
  geometry/labels from the committed `main_menu_title` session (dark backdrop for
  menu art the hidden-lab capture could not expose), with a loading panel; arrow
  keys/Enter/Esc and mouse navigate. `--menu` forces it, `--no-menu` bypasses it,
  `--saves-dir DIR` selects the Continue/Load directory, and
  `--menu --screenshot PATH` renders the menu deterministically.
- Controls (the game's own keys): right-drag pans (and deselects), `Ctrl` + right-drag slides,
  wheel / `=` / `-` zoom, middle-drag drag-zooms, `PageUp`/`PageDown` rescale the camera speed,
  arrows/WASD move (setting *Allow Arrow Key*), `Home` goes to the origin and deselects. Left click
  selects a bibite or egg, a left drag draws a selection rectangle. `Space` pause, `H` hides the UI,
  `R`/`G`/`O`/`E` pick a random / highest-generation / oldest bibite / random egg, `F5` quick save,
  `Insert` lays an egg, `Delete` kills (again: explodes the corpse). With a bibite selected `1`-`4`
  (or the four buttons at the top left) open its Stats / Genes / Biology / Brain panel, pressing the
  open one closes it. `Esc` closes the newest panel, then deselects, then opens the pause menu.
  `F12` saves a screenshot (viewer extra).

Known viewer limitations: the Body-sprite **tail bend** is implemented in the
software renderer (there is no Tail mesh; it is a shader deformation; deterministic
angles + row warp are in place, while the macroquad GPU mesh and `_TailAngle`
sweep fit remain — tracked in [VIEWER.md](VIEWER.md)); text is plain TTF (no TMP
SDF outline); the full runtime HUD/inspector layout is still being wired for
in-window clicks. `assets/game/` is optional; deleting it (or `--no-assets`) falls
back to procedural rendering. Code is MIT/Apache-2.0, while the art under
`assets/game/` remains the original creator's property (unofficial fan build, not
affiliated); see `assets/game/README.md` / `LICENSE-ASSETS.txt` and the README
licensing note.

## 10. Data, logs and cleanup

- All outputs are explicit: `--save`, `--capture`, and stdout stats. Nothing is
  written into the game install or `AppData` by these commands.
- Temporary/experimental output can go under `target/` (gitignored). `testdata/`
  is also gitignored; never commit game data.
- Parity comparisons write reports only where you point them (`--report`).

## 11. Contributing

Workspace layout: `crates/bibites-core` (dependency-free simulation),
`crates/bibites-io` (formats/migrations), `crates/bibites-cli` (headless CLI),
`docs/spec/` (reverse-engineered specs), `Here/` (decompiled game source,
**read-only reference**).

Before pushing, run the workspace gates from the repository root:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Rules:
- Never modify `Here/`, the game install, or the game's `AppData` folders.
- Game-data tests skip cleanly when `testdata/` is absent, so CI stays green
  without the game.
- New format/migration behavior needs a spec note in `docs/spec/` and a test
  under `crates/bibites-io/tests/`; behavior deviations go in
  [DEVIATIONS.md](DEVIATIONS.md).
- The team task board (shared tasks) is the workflow; claim a task before
  starting, keep write scopes disjoint, and report exact commands/results.

## 12. Where to learn more

- [ARCHITECTURE.md](ARCHITECTURE.md) — crates, determinism, validation, perf.
- [INTERFACES.md](INTERFACES.md) — cross-module contracts.
- [PARITY.md](PARITY.md) — game-vs-Rust capture protocol and current results.
- [DEVIATIONS.md](DEVIATIONS.md) — deliberate differences from the game.
- `docs/spec/` — per-subsystem reverse-engineered specifications.

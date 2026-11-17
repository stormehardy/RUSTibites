# RUSTibites Improved

RUSTibites comes in two editions that share one repository:

| | **Faithful** | **Improved** |
|---|---|---|
| Branch | `main` | `improved` |
| Folder | `Z:\RUSTibites` | `Z:\RUSTibites-improved` (a `git worktree` of the same repository) |
| What it is | The game, reproduced as exactly as the port can. Fixes and parity work only. | The same program plus additions. |
| Name | "RUSTibites Faithful by Storme" (title card), zips `RUSTibites-alpha-<date>-<hash>.zip` | "RUSTibites Improved by Storme", zips `RUSTibites-Improved-alpha-<date>-<hash>.zip` |
| Reference point | tag `faithful-2026-11-17` (the build that was on the Desktop that day) | branched from that tag |

The tag has a backup: `C:\Users\Storme\Documents\RUSTibites-backups\rustibites-faithful-2026-11-17.bundle` (`git bundle`, all refs at that time; restore with `git clone <bundle> <folder>`). The repository has no remote: the disk is the only other copy.

## Rules

1. **Faithful never changes for Improved's sake.** Nothing Improved adds may alter what `main` does. `main` takes bug fixes and parity work only.
2. **Fixes flow one way: `main` into `improved`.** Merge them with `git -C Z:\RUSTibites-improved merge main` (do it after every batch of fixes on `main`, so the two never drift far). Never merge `improved` into `main`. A fix found while working on Improved is made on `main` first (it is a Faithful bug too), then merged across.
3. **A feature that changes how a world behaves is opt-in.** Anything that changes what a tick computes (a number, an order, a new rule) is a `Feature` (`crates/bibites-core/src/improved.rs`) and sits behind its switch, which is OFF unless asked for, so Improved with every switch off is the Faithful simulation. The test that enforces this is the pinned run `evolution_regression_fingerprint` (and `birth_path_is_bit_identical_to_pre_optimization_golden`) in `crates/bibites-core/src/species.rs`: **it must pass unchanged on this branch and its pinned numbers must never be edited here**; a changed number means a feature leaked into the Faithful behaviour. Details of the switches:

   * The switches are `ImprovedFeatures` (a `u64` of bits, `Copy`, all off by default), held by the world as `World::improved`. They are NOT written into a save and NOT part of the state fingerprint: they belong to the run.
   * In the simulation: `if self.improved.is_on(Feature::TheFeature) { the new behaviour } else { exactly what the game does }`, and nothing else may differ with it off, not even the order of random numbers drawn. How to add one is in the module's documentation (a variant, its key / title / description, the tests).
   * Choosing them (the viewer): the environment variable `RUSTIBITES_IMPROVED=key1,key2` (set but empty = every switch off) overrides `improved_features.json` in the state folder (`{"enabled": ["switch_probe"]}`; `./userdata` of the folder it starts from). The start-up log line says what is on and where that came from, and names keys it does not know. The viewer gives the choice to whatever world it holds before every tick it runs (`App::tick_world`). There is no settings-panel entry yet: the first real feature adds its row. The CLI does not read the switches yet.
   * `switch_probe` is a switch that changes nothing; it exists to prove the chain end to end (try `RUSTIBITES_IMPROVED=switch_probe` and read the log line) and can go once there are real features.
4. **Features around the simulation** (menus, tools, editors, overlays, statistics, quality of life) do not need a switch; they may be on by default.
5. **Saves stay in the game's format.** Improved reads and writes the original game's save files. Anything extra goes in extra files or entries beside the game's data, and whether the original game tolerates a particular extra must be tried with the game before it is relied on.
6. **The product's name is in one place.** `crates/bibites-view/src/edition.rs` (`PRODUCT_NAME`, the world window's title). The other two spots that differ from `main` are the default zip name in `tools/release/build_alpha.ps1` and the `--help` header in `crates/bibites-view/src/main.rs`. Keep it that way so merging `main` never conflicts over strings. The icon is the same as Faithful's (`assets/icon/`).

## Where things live (so the editions do not mix)

* **Settings and the port's own state** (`user_settings.json`, challenge progress, achievements) are in `./userdata` of the folder the program is started from, or in `RUSTIBITES_DATA_DIR` when that is set. The Improved shortcut starts in `Z:\RUSTibites-improved`, so Improved has its own `userdata` (empty at first). `RUSTIBITES_DATA_DIR` is not set on this machine; do not set it globally, it would make both editions share.
* **Worlds, autosaves, scenarios and bibite files** go into the installed game's own folder (`%USERPROFILE%\AppData\LocalLow\The Bibites\The Bibites`) in both editions, by design: the original game lists what the port saves. They are shared between Faithful, Improved and the game. Set `RUSTIBITES_DATA_DIR` for a run to keep one edition's saves apart.
* **Build output** is per folder (`target_alpha`, `target_light`, ... inside each). The Improved folder started with a copy of Faithful's `target_alpha` and `target_light` so that the registry crates did not have to be compiled again; the workspace crates rebuild on their own paths.
* **Shared by junction, read-only use:** `Z:\RUSTibites-improved\Here` (the decompiled game source, the ground truth for behaviour) and `Z:\RUSTibites-improved\testdata` (the test saves) point at the Faithful folder's. A tool that writes into `testdata` writes there. `target\pyi` (the frozen art extractor used by `build_alpha.ps1`) is a copy.
* **Not carried over:** the `sdf-text-outline-WIP` stash (it belongs to the repository, `git stash list` shows it from either folder) and the two old detached worktrees in `AppData\Local\Temp` from earlier performance work (`git worktree list`).

## Day to day

* Work in `Z:\RUSTibites-improved` on the `improved` branch. Build, test and package exactly as for Faithful (the same RAM guard and light/alpha scripts, pointed at that folder).
* Desktop: `RUSTibites Improved.lnk` runs `Z:\RUSTibites-improved\target\release\rustibites-view.exe` with that folder as its start-in directory; the Faithful shortcuts are untouched.
* After fixes on `main`: `git -C Z:\RUSTibites-improved merge main`, build, run the tests.
* Removing the Improved folder cleanly: `git worktree remove Z:\RUSTibites-improved` (the branch stays until `git branch -d improved`).

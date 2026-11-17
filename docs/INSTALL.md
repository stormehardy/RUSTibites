# Installing RUSTibites (Windows)

RUSTibites is distributed as a **portable zip**: there is no installer, no
service and no registry changes. Download the release zip, unzip it anywhere
writable (for example `%USERPROFILE%\RUSTibites`), and run the executable.

## Release status: 0.2.3 released (2026-11-03)

| field | value |
|---|---|
| artifact | `RUSTibites-0.2.3-windows-x64.zip` |
| size | 156,609,848 bytes (149.35 MB) |
| SHA-256 | `5e33e6aa856c0b60b5d501d3ba50f9b1f33c818d1414eb8819089e571263e4e4` |
| snapshot | `8c30538` (packaged VERSION 0.2.3) |
| `rustibites.exe` | 2,253,824 B, SHA-256 `BC5F2910C9B24D3F55D2C4C6F59799E06510BC09FA38A422C278C19F6013E823` |
| `rustibites-view.exe` | 3,205,120 B, SHA-256 `73EE956A23174F01CB707DA467CBE8435A0260F125E8B19F826ED2994CE61858` |

Adds challenge parameters/star editor, hotkey rebinding, Workshop local edit/online-status, plus 0.2.2 features. Gate:
viewer 96/96, core/io/cli green, clippy/fmt clean; 60 runtime sessions. **Closure criteria met.** Unavoidable
limits: exact editor skin, unavailable rung history, champion pre-seed ordering, Workshop metadata follow-ups.
Steam/audio/online out of scope. Packaged docs may lag the external hash; `SHA256SUMS.txt` is authoritative.

## Historical 0.2.2 release (2026-11-03)

| field | value |
|---|---|
| artifact | `RUSTibites-0.2.2-windows-x64.zip` |
| size | 155,568,731 bytes (148.36 MB) |
| SHA-256 | `f7d9851638719847e20dd05db3f7c8e62026247a189f9ecf87e98048ee74b36c` |
| packaged VERSION | 0.2.2 (windows-x64, portable), snapshot commit `7158aad` |
| `rustibites.exe` | 2,253,824 B, SHA-256 `898A218A91779B5556EC534170390733E30A1217D2910E27FDA7E5BCCAFADB4C` |
| `rustibites-view.exe` | 3,168,768 B, SHA-256 `CA2A6D4684218C5CE65B767B6C3291F359E988BAD2C776DABD3BFEBD36F42B50` |

Gate at 0.2.2 package HEAD: viewer 94/94, core 457, io/cli green, clippy/fmt clean. Packaged menu/Default600
smoke passed. **Packaged docs may lag the external zip hash**; `SHA256SUMS.txt`/this table authoritative.

## Historical 0.2.1 release (2026-11-03)

| field | value |
|---|---|
| artifact | `RUSTibites-0.2.1-windows-x64.zip` |
| size | 134,539,435 bytes (128.31 MB) |
| SHA-256 | `55fdfce6df3fcc897132c32e8b0cbaf18f287c2a8283fca2c3ab9e0089ae7fd4` |
| packaged VERSION | 0.2.1 (windows-x64, portable), snapshot commit `86239a8`, built 2026-11-03T07:08:26Z |
| `rustibites.exe` | 2,253,312 B, SHA-256 `7E00391CBCFF0006650110FC093CB6706385090B58E0C7FDF6AB37B3095D228C` |
| `rustibites-view.exe` | 3,081,728 B, SHA-256 `AC756621BEFE418DE1DBCB091C8C8881986E8CBA5C86F6D59700F4C3DFEAEFFE` |

Gate at 0.2.1 package HEAD: viewer 87/87, core 453, io/cli green, clippy/fmt clean. Packaged
`--menu --screenshot` deterministic; packaged CLI Default 600 t exits 0. **Packaged docs may lag the external
zip hash**; `SHA256SUMS.txt`/this table authoritative.

## Historical 0.2.0 release (2026-11-03)

| field | value |
|---|---|
| artifact | `RUSTibites-0.2.0-windows-x64.zip` |
| size | 124,051,485 bytes (118.31 MB) |
| SHA-256 | `a6cc1396a126cdd5ecb89bbf990c0244b782d250dec57589806209f01b895341` |
| packaged VERSION | 0.2.0 (windows-x64, portable), snapshot commit `603646b`, built 2026-11-03T05:50:33Z |
| `rustibites.exe` | 2,252,288 B, SHA-256 `D00B7CBA5B9E2F8CA99BDDB9D498547CC98E25A6BAF3343AA12828A38EDFBC0A` |
| `rustibites-view.exe` | 3,036,672 B, SHA-256 `D458538D6DAFD18C20D64210EEC59D584D90017C1CF651DD3ADA10B87202CB52` |

Bench verified the package; Lead independently re-extracted it and confirmed packaged `--menu --screenshot`
matches `E533D78A…` and packaged CLI Default 600 t exits 0 (1→10 bibites). **Packaged docs may lag the
external zip hash** (the packaging pass embeds docs before the final self-referential hash update); use
`SHA256SUMS.txt`/this external table as authoritative.

**0.2.0:** historical package (118.31 MB / `a6cc1396…`). **0.2.1/0.2.2:** historical packages above; **0.2.3:** released above.

**Deferred in 0.2.1:** editor template save/continue, exact editor skin, node drag/anchors, exact lineage
rung/distribution, secondary panels/challenges; Steam/audio out of scope. The **0.1.0** values below are
historical.

## Requirements

- 64-bit Windows 10/11.
- No runtime dependencies (no .NET, no Visual C++ redistributable, no Rust
  toolchain) — the executable is self-contained.
- Optional: real The Bibites data (saves/scenarios) if you want to continue an
  existing world.

## Install

1. Download `RUSTibites-<version>-windows-x64.zip` and `SHA256SUMS.txt` from the
   release page.
2. Verify the download (PowerShell, in the download folder):

   ```powershell
   Get-FileHash .\RUSTibites-<version>-windows-x64.zip -Algorithm SHA256
   # compare with the hash in SHA256SUMS.txt
   ```

3. Unzip to a writable folder (right-click → *Extract All…*, or
   `Expand-Archive .\RUSTibites-*.zip -DestinationPath $env:USERPROFILE\RUSTibites`).
4. SmartScreen: the executable is unsigned, so Windows may show *"Windows
   protected your PC"*. Click **More info → Run anyway** (or unblock the file:
   `Unblock-File .\rustibites.exe`). This is expected for an unsigned
   open-source build; the source and SHA-256 are published.
5. Run:

   ```powershell
   .\rustibites.exe help
   .\rustibites.exe info <path-to-save-or-scenario.zip>
   .\rustibites.exe run <path-to-save-or-scenario.zip> --ticks 300 --seed 1
   ```

The zip contains `rustibites.exe`, optional `rustibites-view.exe`, `README.md`,
`LICENSE-*`, `VERSION.txt` and a `docs/` folder with the user guide.

## Where to put game data

The `rustibites` command-line tool never writes to the game install or game
folders. The interactive viewer (`rustibites-view`) does write to the game's *data*
folder (never the install): saves, autosaves, quick saves, scenarios made in the
scenario editor and single-bibite files go to
`%USERPROFILE%\AppData\LocalLow\The Bibites\The Bibites\` (`Savefiles\`,
`Savefiles\Autosaves\`, `Scenarios\`, `Bibites\`), so the original game lists them
too; set `RUSTIBITES_DATA_DIR` to use another folder instead. Its own settings stay
in `userdata\` next to where it is started.

For the command-line tool, put/copy the files you want to use anywhere, e.g. a
`data\` folder next to the executable:

```
RUSTibites\
  rustibites.exe
  data\
    Default.zip             (scenario, canonical test world)
    my_save.zip             (save)
    Basic bibite.bb8template
```

The game's own data lives in
`%USERPROFILE%\AppData\LocalLow\The Bibites\The Bibites\` (`Savefiles\`,
`Scenarios\`, `Bibites\`). Copy files from there; do not modify them in place.

Runs write only where you ask: `--save <file>.zip`, `--capture <dir>`, or
stdout statistics.

## Updating

1. Download the new zip and checksum, verify it, unzip to a new folder (keep the
   old one until the new build works).
2. Delete the old folder when satisfied. Your data/saves are not stored in the
   install folder unless you put them there.

## Uninstall / cleanup

- Close any running `rustibites.exe` processes, then delete the unzipped folder.
- Remove any `--save`/`--capture` output you created yourself.
- No registry entries, services or system files are created; no uninstaller is
  needed.

## Optional: desktop shortcut

A helper exists in the source repository (`scripts/make_shortcut.ps1`, not
included in the release zip). From a source checkout:

```powershell
pwsh -File scripts\make_shortcut.ps1 -TargetPath "$env:USERPROFILE\RUSTibites\rustibites.exe"
```

Or create a shortcut manually: right-click `rustibites.exe` → *Send to* →
*Desktop (create shortcut)*, then edit the shortcut's *Start in* to the folder
that holds your data.

## Verification — historical 0.1.0 (2026-10-30, task-131)

The 0.1.0 portable package was rebuilt by `tools/release/package.ps1`; the artifact was verified end-to-end
from a temp extraction:

- zip: `RUSTibites-0.1.0-windows-x64.zip`, **36.71 MB** (38,496,196 bytes);
  SHA-256 `b201c1b73631f77f04d6032a6fad07ebb5bbcce7678ea03e8e8df3bb310d8479`
  (`SHA256SUMS.txt`, hash matches on re-computation).
  Full fresh-extraction certification (CLI, checkpoint resume, feeding §10.4.1
  reproduction, deterministic viewer screenshots):
  [`RELEASE_CERTIFICATION.md`](RELEASE_CERTIFICATION.md).
- contents: `rustibites.exe` (2,228,736 B), `rustibites-view.exe` (2,378,752 B),
  `README.md`, `LICENSE-MIT`, `LICENSE-APACHE`, `VERSION.txt`, `docs/`,
  `assets/` and `tools/ui/` (UI atlas + runtime layout data so the viewer can
  render the game HUD in the portable build).
- packaged CLI (from the extracted folder): `rustibites.exe info <scenario>`;
  `rustibites.exe run <scenario> --ticks 600 --seed 12345 --stats json` →
  exit 0, `bibites_start: 1`, `bibites_end: 10`, no deaths, 600 ticks in
  ≈0.038 s (≈16k ticks/s, 530× real-time on this machine).
- packaged viewer (headless, no window): with bundled assets
  `rustibites-view.exe <scenario> --screenshot out.png --frames 30` → exit 0,
  1280×800 PNG; log shows game assets loaded and the runtime HUD loaded from
  **`tools/ui/ui_runtime_atlas.json.gz`** (7,874 nodes). With `--no-assets` →
  exit 0, 1280×800 PNG, procedural renderer fallback while the UI runtime still
  loads from the bundled `tools/ui/`.
- CLI commands checked against the release binary: `help`, `info`, `check`,
  `run` (scenario/save/genome, `--stats human|json|csv`, `--seed`,
  `--threads`, `--templates`, `--save`, `--capture` + parity compare), `bench`
  (synthetic + `--scenario`, `--json`); viewer `--screenshot` modes
  (`--frames`, `--fit`, `--center`, `--zoom`, `--assets`, `--no-assets`,
  `--runtime-ui`, `--simple-hud`, `--ui-only`).
- **Historical package scope:** the `0.1.0` artifact predates the functional-UI tranche; its hashes above
  remain authoritative for 0.1.0 only. The 0.2.0 package adds the functional menus/Library/New Game/
  lineage/KILL dialog/save browser/tool panels/scenario-editor core; its authoritative size/SHA are in the
  table above.

Known limitations: `testdata/` is **not shipped** (game files are local-only);
the interactive viewer needs a window/GPU (headless screenshot mode does not);
software screenshots render the HUD/text via the CPU rasteriser (no GPU), and
the game art under `assets/game/` is optional and can be removed for a
procedural fallback. In the 0.2.0 candidate the Body-sprite tail bend is
implemented in the software renderer (no Tail mesh; shader deformation; angles +
row warp landed, macroquad GPU mesh and `_TailAngle` fit pending), the title/main
menu and functional panels use the captured runtime layout with a dark backdrop
where menu art was not captured, and text uses TTF with a five-copy TMP-like
outline (true SDF atlas pending). **0.2.0 deferred polish:** gene scroll >14, exact editor skin, node
drag/anchors, mutate-settings wiring, template save/continue, exact lineage rung data, challenges/Steam,
TAG beyond the egg inspector. The window recolours the bibites with a GPU
version of the game's skin shader by default (`--no-gpu-hue-shader` turns it
off); `--screenshot` always uses the software rasteriser, which has the same maths.
The code is MIT/Apache-2.0, but the art under `assets/game/` remains the
original creator's property (unofficial fan build; see
`assets/game/README.md`/`LICENSE-ASSETS.txt`; not affiliated/endorsed).

## Developer build (not the portable zip)

To build from source instead, see [USER_GUIDE.md](USER_GUIDE.md) §1
(`cargo build --release -p bibites-cli`). The release script that produces the
portable zip is `tools/release/package.ps1`.


## Release certification

The historical 0.2.0 windows-x64 zip is authoritative from bench's package report and Lead's independent
extraction; 0.2.1 values are recorded above;
see `docs/RELEASE_CERTIFICATION.md` for the full table. Historical 0.1.0 summary: zip SHA-256
`b201c1b73631f77f04d6032a6fad07ebb5bbcce7678ea03e8e8df3bb310d8479`; packaged CLI reproduces the canonical
Default 600/3000 t results and bit-exact save+checkpoint resume; packaged feeding/contact numbers match
`PARITY_ROUND2 §10.4.1`; packaged viewer headless screenshots are deterministic with and without assets.

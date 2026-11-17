# The tester alpha package

`tools/release/build_alpha.ps1` builds `target/alpha-package/RUSTibites-alpha-<date>-<commit>.zip` (about 46 MB) for people who own The Bibites.

**What is in it:** `RUSTibites.exe` (perf profile, static CRT: no Visual C++ redistributable needed), `Play.bat` / `play.ps1` (first run builds the
art, then starts the viewer with a private `userdata` folder, windowed, log in `logs/`), `Play-with-my-game-saves.bat`, `Make-Bug-Report.bat`
(zips logs, settings, screenshots and machine info to the Desktop), `extract_art.exe`, `art_map.json`, `README.txt`, `KNOWN_GAPS.txt`
(the "Open items" block of `PARITY_REMAINING.md`), the licences, and `assets/game` WITHOUT any art (layouts, manifests, metadata: ~28 MB).

**No game art is shipped.** `extract_art.exe` (the repo's `tools/assets/extract.py` frozen with UnityPy in a clean venv, plus `tools/release/extract_art.py`)
finds the game (Steam registry and library folders, common paths, a folder picker when it cannot), reads it read-only and writes
`sheets/ sprites/ fonts/ ui/` plus the lineage / button-theme / UI-font pictures listed in `art_map.json` (pixel-identical to the repo's
`assets/game`; checked with headless screenshots of ~20 screens, which came out byte-identical, except the title background still, see below).
`art_map.json` is regenerated with `python tools/release/...` only if the repo's art layout changes (the script that made it is in the history of
this file: hash every art file of `assets/game` against a fresh extraction).

**Not in the package, on purpose:** `assets/game/reference/` (captures of the live game: ~70 MB; only a few JSON files of it are read), `ui_runtime.json.gz`,
`ui_layout.json` and `ui_runtime_save.json.gz` (not read at runtime), the menu-background stills (they were screenshots of the game; the title
background is the game's own video, read from the tester's install and decoded with ffmpeg when it is on the PATH, else the menu is dark).

**Maintainer notes:** F12 in the window saves what it shows to `screenshots/`. Build with `pwsh tools/release/build_alpha.ps1` (builds the viewer into
`target_alpha`, freezes the extractor, assembles, zips, writes the SHA-256). Test a package on a machine without the repo: unzip, run `Play.bat`.

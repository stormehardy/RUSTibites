#!/usr/bin/env python3
"""extract_art.py - builds the game art RUSTibites needs from the player's OWN copy of The Bibites.

The alpha package ships no game art. This tool reads the installed game (read-only), runs the repo's extractor into a temporary
folder and puts the pictures where the viewer looks for them (`assets/game/sheets|sprites|fonts|ui`, the lineage / button-theme /
UI-font pictures through `art_map.json`). Run it with no arguments: it finds the game by itself.

    extract_art.exe                       # find the game, write ./assets/game
    extract_art.exe --game "D:\\Games\\The Bibites"
    extract_art.exe --out path\\to\\assets\\game
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import sys
import tempfile
from pathlib import Path


def here() -> Path:
    """The folder of this tool (the package root when frozen, the repo's tools/release otherwise)."""
    if getattr(sys, "frozen", False):
        return Path(sys.executable).resolve().parent
    return Path(__file__).resolve().parent


def steam_libraries() -> list[Path]:
    out: list[Path] = []
    roots: list[Path] = []
    try:
        import winreg  # type: ignore

        for hive, key, name in [
            (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
            (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam", "InstallPath"),
            (winreg.HKEY_CURRENT_USER, r"SOFTWARE\Valve\Steam", "SteamPath"),
        ]:
            try:
                with winreg.OpenKey(hive, key) as handle:
                    roots.append(Path(winreg.QueryValueEx(handle, name)[0]))
            except OSError:
                pass
    except ImportError:
        pass
    for drive in "CDEFGH":
        roots.append(Path(f"{drive}:/Program Files (x86)/Steam"))
        roots.append(Path(f"{drive}:/Steam"))
        roots.append(Path(f"{drive}:/SteamLibrary"))
    for root in roots:
        out.append(root)
        vdf = root / "steamapps" / "libraryfolders.vdf"
        if vdf.is_file():
            try:
                for match in re.finditer(r'"path"\s+"([^"]+)"', vdf.read_text(encoding="utf-8", errors="replace")):
                    out.append(Path(match.group(1).replace("\\\\", "\\")))
            except OSError:
                pass
    return out


def find_game(explicit: str | None) -> Path | None:
    """The folder that holds `The Bibites_Data` (or that folder itself, returned as the install)."""
    candidates: list[Path] = []
    if explicit:
        candidates.append(Path(explicit))
    for var in ("RUSTIBITES_GAME_DIR",):
        if os.environ.get(var):
            candidates.append(Path(os.environ[var]))
    marker = here() / "game_dir.txt"
    if marker.is_file():
        candidates.append(Path(marker.read_text(encoding="utf-8").strip()))
    profile = os.environ.get("USERPROFILE")
    if profile:
        candidates.append(Path(profile) / "TheBibites")
    for library in steam_libraries():
        candidates.append(library / "steamapps" / "common" / "The Bibites")
        candidates.append(library / "The Bibites")
    for drive in "CDEFGH":
        candidates.append(Path(f"{drive}:/Games/The Bibites"))
        candidates.append(Path(f"{drive}:/The Bibites"))
    for candidate in candidates:
        if (candidate / "The Bibites_Data").is_dir():
            return candidate
        if candidate.name == "The Bibites_Data" and candidate.is_dir():
            return candidate.parent
    return None


def main() -> int:
    ap = argparse.ArgumentParser(description="Extract The Bibites' art from your own install for RUSTibites.")
    ap.add_argument("--game", help="the game's folder (the one with The Bibites.exe)")
    ap.add_argument("--out", default=str(here() / "assets" / "game"))
    ap.add_argument("--map", default=str(here() / "art_map.json"))
    ap.add_argument("--force", action="store_true", help="extract again even if the art is there")
    args = ap.parse_args()

    out = Path(args.out)
    stamp = out / ".art_extracted.json"
    game = find_game(args.game)
    if game is None:
        print("Could not find The Bibites. Give its folder (the one with 'The Bibites.exe'):")
        print('    extract_art.exe --game "D:\\Games\\The Bibites"')
        return 2
    data = game / "The Bibites_Data"
    print(f"Game found: {game}")
    if stamp.is_file() and not args.force:
        try:
            if json.loads(stamp.read_text(encoding="utf-8")).get("game") == str(game):
                print("The art is already extracted (use --force to do it again).")
                return 0
        except (OSError, ValueError):
            pass

    # The extractor lives next to the tool (frozen: bundled; from the repo: tools/assets).
    sys.path.insert(0, str(here()))
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "assets"))
    import extract  # type: ignore

    work = Path(tempfile.mkdtemp(prefix="rustibites-art-"))
    try:
        print("Reading the game's files (about 10 seconds)...")
        saved = sys.argv
        sys.argv = ["extract.py", "--game-data", str(data), "--out", str(work / "game")]
        try:
            code = extract.main()
        finally:
            sys.argv = saved
        if code not in (0, None):
            print(f"The extractor failed (code {code}).")
            return 1
        tmp = work / "game"
        try:
            errors = json.loads((tmp / "manifest.json").read_text(encoding="utf-8")).get("errors", [])
            # The game's dynamic font atlases ("Font Texture") hold no picture data to read ([Errno 13] on the data folder): nothing the
            # viewer uses, so they are not worth a worrying line.
            for entry in [e for e in errors if e.get("name") != "Font Texture"][:3]:
                print("note:", entry)
        except (OSError, ValueError):
            pass
        out.mkdir(parents=True, exist_ok=True)
        for name in ("sheets", "sprites", "fonts", "ui"):
            source = tmp / name
            if source.is_dir():
                target = out / name
                if target.exists():
                    shutil.rmtree(target)
                shutil.copytree(source, target)
        art_map = Path(args.map)
        if art_map.is_file():
            mapping = json.loads(art_map.read_text(encoding="utf-8"))
            for destination, source in mapping.get("copy", {}).items():
                target = out / destination
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(tmp / source, target)
            for destination in mapping.get("blank", []):
                from PIL import Image  # type: ignore

                target = out / destination
                target.parent.mkdir(parents=True, exist_ok=True)
                Image.new("RGBA", (1, 1), (0, 0, 0, 0)).save(target)
        else:
            print(f"warning: {art_map} is missing; some pictures will be absent")
        stamp.write_text(json.dumps({"game": str(game)}), encoding="utf-8")
    finally:
        shutil.rmtree(work, ignore_errors=True)
    print(f"Done: the art is in {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

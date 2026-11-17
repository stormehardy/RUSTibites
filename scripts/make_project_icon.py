"""Build the RUSTibites project icon from the installed game's icon, tinted red.

This is a local development tool. It reads the icon that ships inside the
installed copy of The Bibites (which the developer owns) and writes a
red-tinted derivative into assets/icon/. The derivative is used for the local
RUSTibites binary and shortcuts; it is not required to build or run the
simulation. Do not redistribute the game's artwork.

Usage:
    python scripts/make_project_icon.py
    python scripts/make_project_icon.py --exe "C:/path/The Bibites.exe"
    python scripts/make_project_icon.py --png my_icon.png

With --png the game's icon is not read at all: the finished 256x256 picture you
pass becomes the master image (the committed icon since 2026-11-17 is one such
hand-edited picture, so running this script WITHOUT --png would overwrite it
with the red-tinted version again).

Outputs:
    assets/icon/rustibites_256.png   master image (red-tinted, or the --png picture)
    assets/icon/rustibites.ico       multi-size Windows icon (16..256)
    assets/icon/original.ico         extracted original (local reference only; not with --png)
    assets/viewer/rustibites-icon.png, assets/viewer/rustibites.ico   copies of the two above
"""

from __future__ import annotations

import argparse
import shutil
import struct
from pathlib import Path

from PIL import Image

DEFAULT_EXE = r"C:\Program Files (x86)\Steam\steamapps\common\The Bibites\The Bibites.exe"
REPO = Path(__file__).resolve().parent.parent
OUT_DIR = REPO / "assets" / "icon"
VIEWER_DIR = REPO / "assets" / "viewer"

ICON_SIZES = [16, 24, 32, 48, 64, 96, 128, 192, 256]

RT_ICON = 3
RT_GROUP_ICON = 14


def read_icon_frames(exe_path: Path) -> list[tuple[int, int, bytes]]:
    """Returns (width, height, image_bytes) frames from the PE's RT_GROUP_ICON."""
    import pefile

    pe = pefile.PE(str(exe_path))
    icons: dict[int, bytes] = {}
    groups: dict[int, bytes] = {}
    for entry in pe.DIRECTORY_ENTRY_RESOURCE.entries:
        if entry.id not in (RT_ICON, RT_GROUP_ICON):
            continue
        for e1 in entry.directory.entries:
            for e2 in e1.directory.entries:
                data = e2.data.struct
                blob = pe.get_data(data.OffsetToData, data.Size)
                if entry.id == RT_ICON:
                    icons[e1.id] = blob
                else:
                    groups[e1.id] = blob

    frames: list[tuple[int, int, bytes]] = []
    for group in groups.values():
        count = struct.unpack_from("<H", group, 4)[0]
        offset = 6
        for _ in range(count):
            width, height, _colors, _reserved, _planes, _bpp, _size, icon_id = (
                struct.unpack_from("<BBBBHHIH", group, offset)
            )
            offset += 14
            blob = icons.get(icon_id)
            if blob is not None:
                frames.append((width or 256, height or 256, blob))
    if not frames:
        raise SystemExit(f"no icons found in {exe_path}")
    frames.sort(key=lambda frame: frame[0] * frame[1])
    return frames


def write_ico(frames: list[tuple[int, int, bytes]], path: Path) -> None:
    """Writes an ICO containing the given frames (PNG or BMP payloads)."""
    header = struct.pack("<HHH", 0, 1, len(frames))
    entries = b""
    payload = b""
    offset = 6 + 16 * len(frames)
    for width, height, blob in frames:
        entries += struct.pack(
            "<BBBBHHII",
            0 if width >= 256 else width,
            0 if height >= 256 else height,
            0,
            0,
            1,
            32,
            len(blob),
            offset,
        )
        offset += len(blob)
        payload += blob
    path.write_bytes(header + entries + payload)


def tint_red(image: Image.Image) -> Image.Image:
    """Red-tints the icon, preserving its alpha and luminance detail."""
    rgba = image.convert("RGBA")
    red, green, blue, alpha = rgba.split()
    # Luminance as a weighted sum, computed in Pillow's C layer.
    luminance = Image.merge("RGB", (red, green, blue)).convert("L")
    # Slightly boost contrast so the red silhouette stays readable at 16 px.
    luminance = luminance.point(lambda value: min(255, int(value * 1.08 + 8)))
    tinted = Image.merge(
        "RGBA",
        (
            luminance,
            luminance.point(lambda value: int(value * 0.16)),
            luminance.point(lambda value: int(value * 0.16)),
            alpha,
        ),
    )
    return tinted


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", default=DEFAULT_EXE, help="path to The Bibites.exe")
    parser.add_argument(
        "--png",
        help="a finished square picture (256x256 or larger) to use as the master image instead of the tinted game icon",
    )
    args = parser.parse_args()

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    if args.png:
        picture = Image.open(args.png).convert("RGBA")
        if picture.width != picture.height:
            raise SystemExit(f"{args.png} is {picture.width}x{picture.height}: the icon must be square")
        if picture.width < 256:
            raise SystemExit(f"{args.png} is {picture.width} px wide: 256 px or more is needed (no upscaling)")
        master = picture if picture.width == 256 else picture.resize((256, 256), Image.LANCZOS)
    else:
        exe = Path(args.exe)
        if not exe.exists():
            raise SystemExit(
                f"game executable not found: {exe}\n"
                "Pass --exe with the installed copy of The Bibites."
            )

        frames = read_icon_frames(exe)
        print(f"extracted {len(frames)} icon frame(s), largest {frames[-1][0]}x{frames[-1][1]}")

        write_ico(frames, OUT_DIR / "original.ico")

        original = Image.open(OUT_DIR / "original.ico")
        # Pillow opens the largest frame; normalise to 256x256 RGBA.
        original = original.convert("RGBA").resize((256, 256), Image.LANCZOS)
        master = tint_red(original)

    master.save(OUT_DIR / "rustibites_256.png", "PNG")
    master.save(
        OUT_DIR / "rustibites.ico",
        format="ICO",
        sizes=[(size, size) for size in ICON_SIZES],
    )
    print(f"wrote {OUT_DIR / 'rustibites_256.png'}")
    print(f"wrote {OUT_DIR / 'rustibites.ico'}")
    # The viewer's asset folder keeps copies of the two files (docs/VIEWER.md).
    VIEWER_DIR.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(OUT_DIR / "rustibites_256.png", VIEWER_DIR / "rustibites-icon.png")
    shutil.copyfile(OUT_DIR / "rustibites.ico", VIEWER_DIR / "rustibites.ico")
    print(f"copied both to {VIEWER_DIR}")


if __name__ == "__main__":
    main()

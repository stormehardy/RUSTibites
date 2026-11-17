"""Faint marks that the GPU window draws round a bibite and the software render does not (the "hairlines" along sprite rectangles).

usage: python -I tools/window_specks.py <window.png> <software.png> <x0,y0,x1,y1> [<marked.png>]

<window.png> is an in-process framebuffer capture of the real window (RUSTIBITES_SELF_SHOT, see tools/window_hairline_probe.ps1),
<software.png> the same view from the headless renderer (--screenshot) at the same size. The region is the part of the picture to look at (keep the HUD out).

A "speck" is a pixel that differs from the median of its 5x5 ring by >= 10 and whose neighbours mostly do not (a block edge is not a speck).
The window-only specks are the window's specks with no software speck within 3 px. They are sorted by what surrounds them:

  * in empty space (>= 20 of the 24 ring pixels are the world's floor): the dotted-line artefact of docs/PARITY_REMAINING.md ("artifacts floating round the
    bibite"): the window's multisampled surface read the neighbouring cell of the part sheet at the edge of a sprite's quad;
  * on the silhouette edge (8..19 floor pixels) and inside the art (fewer): differences of the art itself. The software render is 2x supersampled, the window is
    not, so a few of these always exist (a one-texel arm drawn at full strength against a softened one, a rotated texel corner).

For each window-only speck the script also fits

    pixel = alpha * texel + (1 - alpha) * background           (the texel is a neutral grey: the art of a bibite with no pigment)

over the three channels: a multisampled edge shows as alpha = 1/2 (2 samples) or 3/4 (4 samples) and the texel is a value of the art's palette
(the signature of an edge pixel whose texture coordinate was extrapolated past the sprite's cell).
Exit code 0 when there are no window-only specks in empty space (the artefact), 1 otherwise.
"""
import sys
from collections import Counter

from PIL import Image


def dist(a, b):
    return max(abs(a[0] - b[0]), abs(a[1] - b[1]), abs(a[2] - b[2]))


def ring_of(px, x, y):
    return [px[x + dx, y + dy] for dy in (-2, -1, 0, 1, 2) for dx in (-2, -1, 0, 1, 2) if (dx, dy) != (0, 0)]


def specks(px, size, region):
    w, h = size
    x0, y0, x1, y1 = region
    found = []
    for y in range(max(y0, 3), min(y1, h - 3)):
        for x in range(max(x0, 3), min(x1, w - 3)):
            ring = ring_of(px, x, y)
            median = Counter(ring).most_common(1)[0][0]
            if dist(px[x, y], median) < 10:
                continue
            close = sum(1 for dx, dy in ((-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)) if dist(px[x + dx, y + dy], median) <= 6)
            if close >= 5:
                found.append((x, y))
    return found


def fit_alpha(px, x, y):
    ring = ring_of(px, x, y)
    bg = Counter(ring).most_common(1)[0][0]
    hair = px[x, y]
    bs, hs = list(bg), list(hair)
    mean_b, mean_h = sum(bs) / 3, sum(hs) / 3
    var = sum((b - mean_b) ** 2 for b in bs)
    if var < 1:
        return None  # a grey background cannot separate alpha from the texel
    slope = sum((b - mean_b) * (hh - mean_h) for b, hh in zip(bs, hs)) / var
    alpha = 1 - slope
    texel = (mean_h - slope * mean_b) / alpha if alpha > 0.01 else float('nan')
    return alpha, texel


def floor_palette(px, region):
    """The colours that cover the region's floor: every colour that holds at least 4 % of the region (the default world has two shades)."""
    x0, y0, x1, y1 = region
    counts = Counter(px[x, y] for y in range(y0, y1) for x in range(x0, x1))
    area = (x1 - x0) * (y1 - y0)
    return [colour for colour, n in counts.most_common(6) if n >= 0.04 * area]


def surroundings(px, x, y, palette):
    """How many of the 24 ring pixels are floor."""
    return sum(1 for c in ring_of(px, x, y) if any(dist(c, p) <= 6 for p in palette))


def main():
    window = Image.open(sys.argv[1]).convert('RGB')
    software = Image.open(sys.argv[2]).convert('RGB')
    region = [int(v) for v in sys.argv[3].split(',')]
    marked = sys.argv[4] if len(sys.argv) > 4 else None
    if window.size != software.size:
        sys.exit(f'sizes differ: window {window.size}, software {software.size}')
    wp, sp = window.load(), software.load()
    window_specks = specks(wp, window.size, region)
    software_specks = set(specks(sp, software.size, region))
    only = [p for p in window_specks if not any((p[0] + dx, p[1] + dy) in software_specks for dx in range(-3, 4) for dy in range(-3, 4))]
    palette = floor_palette(wp, region)
    empty, edge, art = [], [], []
    for p in only:
        floor = surroundings(wp, *p, palette)
        (empty if floor >= 20 else edge if floor >= 8 else art).append(p)
    print(f'specks: window {len(window_specks)}, software {len(software_specks)}, window only {len(only)}: in empty space {len(empty)}, on the silhouette edge {len(edge)}, inside the art {len(art)}')
    alphas = []
    for p in only:
        fit = fit_alpha(wp, *p)
        if fit is not None:
            alphas.append(round(fit[0], 2))
    if alphas:
        histogram = Counter(alphas)
        print('fitted alpha of the window-only specks (value: count):', dict(sorted(histogram.items())))
    if marked:
        out = window.copy()
        op = out.load()
        for group, colour in ((empty, (255, 0, 255)), (edge, (255, 255, 0)), (art, (0, 255, 255))):
            for x, y in group:
                for d in range(-4, 5):
                    for xx, yy in ((x + d, y - 4), (x + d, y + 4), (x - 4, y + d), (x + 4, y + d)):
                        if 0 <= xx < out.width and 0 <= yy < out.height:
                            op[xx, yy] = colour
        out.save(marked)
    sys.exit(0 if not empty else 1)


if __name__ == '__main__':
    main()

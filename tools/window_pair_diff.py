"""How far the real window's picture of a view is from the headless software render of it (tools/window_pair.ps1).

usage: python -I tools/window_pair_diff.py <window.png> <software.png> [x0,y0,x1,y1] [<diff.png>]

Prints the mean |dR|+|dG|+|dB| over the region (the unit the parity notes use) and, for each picture, the mean saturation of the saturated pixels and how many
there are: a body that goes grey in the window shows as a lot fewer saturated pixels there than in the software render. With a fourth argument the per-pixel
difference (x4) is written there.
"""
import colorsys
import sys

from PIL import Image


def main():
    window = Image.open(sys.argv[1]).convert('RGB')
    software = Image.open(sys.argv[2]).convert('RGB')
    if window.size != software.size:
        sys.exit(f'sizes differ: window {window.size}, software {software.size}')
    region = [int(v) for v in sys.argv[3].split(',')] if len(sys.argv) > 3 and sys.argv[3] else [0, 0, window.width, window.height]
    x0, y0, x1, y1 = region
    wp, sp = window.load(), software.load()
    total = 0
    differing = 0
    sat = {'window': [0, 0.0], 'software': [0, 0.0]}
    diff = Image.new('RGB', (x1 - x0, y1 - y0))
    dp = diff.load()
    for y in range(y0, y1):
        for x in range(x0, x1):
            a, b = wp[x, y], sp[x, y]
            d = abs(a[0] - b[0]) + abs(a[1] - b[1]) + abs(a[2] - b[2])
            total += d
            if d > 24:
                differing += 1
            dp[x - x0, y - y0] = tuple(min(255, abs(a[i] - b[i]) * 4) for i in range(3))
            for name, pixel in (('window', a), ('software', b)):
                h, s, v = colorsys.rgb_to_hsv(*(c / 255 for c in pixel))
                if s > 0.25 and v > 0.2:
                    sat[name][0] += 1
                    sat[name][1] += s
    area = (x1 - x0) * (y1 - y0)
    print(f'region {region}: mean |dRGB| {total / area:.3f}, pixels differing by more than 24: {differing}')
    for name, (count, total_s) in sat.items():
        print(f'{name}: {count} saturated pixels, mean saturation {total_s / count if count else 0:.3f}')
    if len(sys.argv) > 4:
        diff.save(sys.argv[4])


if __name__ == '__main__':
    main()

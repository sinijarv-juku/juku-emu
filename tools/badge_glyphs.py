#!/usr/bin/env python3
"""Write the lettering of the monitor's Электроника badge into both shaders.

    badge_glyphs.py

The badge on the МС6105's front reads ЭЛЕКТРОНИКА in a narrow sans serif
(MD §7.4). The shaders draw it with Whole Monitor on, as strokes: line
segments whose distance gives the letters at any size, with no texture and
no font. This lays the word out from the letters' paths below and writes
the segments, in cap heights, x from the start of the word and y up from
the baseline, between the `badge strokes` markers of
frontends/macos/src/Shaders.metal and frontends/windows/src/Shaders.hlsl.
Run it again after changing a letter.
"""
import math
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent
SHADERS = {
    ROOT / 'frontends/macos/src/Shaders.metal': ('constant int', 'constant float4', '{', '}'),
    ROOT / 'frontends/windows/src/Shaders.hlsl': ('static const int', 'static const float4', 'float4(', ')'),
}
WORD = 'ЭЛЕКТРОНИКА'
SPACING = 0.17


def arc(cx, cy, rx, ry, start, end, steps):
    """Points along an ellipse from angle `start` to `end`, in degrees."""
    points = []

    for i in range(steps + 1):
        a = math.radians(start + (end - start) * i / steps)
        points.append((cx + rx * math.cos(a), cy + ry * math.sin(a)))

    return points


# Each letter: its width and its paths, in cap heights, y up.
LETTERS = {
    'Э': (0.50, [arc(0.25, 0.5, 0.25, 0.5, 145, -145, 10), [(0.16, 0.5), (0.5, 0.5)]]),
    'Л': (0.50, [[(0.0, 0.0), (0.07, 0.12), (0.11, 1.0), (0.5, 1.0), (0.5, 0.0)]]),
    'Е': (0.44, [[(0.44, 1.0), (0.0, 1.0), (0.0, 0.0), (0.44, 0.0)], [(0.0, 0.5), (0.38, 0.5)]]),
    'К': (0.50, [[(0.0, 0.0), (0.0, 1.0)], [(0.47, 1.0), (0.0, 0.42)], [(0.16, 0.6), (0.5, 0.0)]]),
    'Т': (0.50, [[(0.0, 1.0), (0.5, 1.0)], [(0.25, 1.0), (0.25, 0.0)]]),
    'Р': (0.48, [[(0.0, 0.0), (0.0, 1.0), (0.26, 1.0)] + arc(0.26, 0.765, 0.22, 0.235, 90, -90, 6) + [(0.0, 0.53)]]),
    'О': (0.52, [arc(0.26, 0.5, 0.26, 0.5, 0, 360, 16)]),
    'Н': (0.50, [[(0.0, 0.0), (0.0, 1.0)], [(0.5, 0.0), (0.5, 1.0)], [(0.0, 0.5), (0.5, 0.5)]]),
    'И': (0.50, [[(0.0, 1.0), (0.0, 0.0), (0.5, 1.0), (0.5, 0.0)]]),
    'А': (0.54, [[(0.0, 0.0), (0.27, 1.0), (0.54, 0.0)], [(0.09, 0.33), (0.45, 0.33)]]),
}


def segments():
    out = []
    x = 0.0

    for letter in WORD:
        width, paths = LETTERS[letter]

        for path in paths:
            for (x0, y0), (x1, y1) in zip(path, path[1:]):
                out.append((x + x0, y0, x + x1, y1))

        x += width + SPACING

    return out


def main():
    strokes = segments()

    for path, (int_type, vec_type, open_, close) in SHADERS.items():
        rows = '\n'.join(f'    {open_}{", ".join(f"{c:.3f}" for c in s)}{close},' for s in strokes)
        block = (f'// badge strokes: made by tools/badge_glyphs.py, not edited by hand\n'
                 f'{int_type} kBadgeSegments = {len(strokes)};\n'
                 f'{vec_type} kBadgeText[{len(strokes)}] = {{\n{rows}\n}};\n'
                 f'// end of badge strokes')
        text = path.read_text(encoding='utf-8')
        new, count = re.subn(r'// badge strokes:.*?// end of badge strokes', lambda _: block, text, flags=re.S)

        if count != 1:
            raise SystemExit(f'{path}: no badge strokes markers')

        path.write_text(new, encoding='utf-8', newline='\n')
        print(f'{path.relative_to(ROOT)}: {len(strokes)} segments')


if __name__ == '__main__':
    main()

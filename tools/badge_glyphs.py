#!/usr/bin/env python3
"""Write the lettering of the monitor's Электроника badge into both shaders.

    badge_glyphs.py

The badge on the МС6105's front reads электроника in a bold geometric sans
serif, all its letters as tall as capitals, and its emblem holds a spire
between a small Э and С (MD §7.4). The shaders draw both with Whole Monitor
on, as strokes: line segments whose distance gives the letters at any size,
with no texture and no font. The shapes were drawn after close-up photos of
the badge.

This lays out the word, in cap heights, x from the start of the word and y up
from the baseline, and the emblem's two letters, in the emblem frame's widths,
x from its left edge and y down from its top, and writes them between the
`badge strokes` markers of frontends/macos/src/Shaders.metal and
frontends/windows/src/Shaders.hlsl. Each path is the centre line of a stroke
STROKE wide, inside its letter's box. Run it again after changing a letter.
"""
import math
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent
SHADERS = {
    ROOT / 'frontends/macos/src/Shaders.metal': ('constant int', 'constant float4', '{', '}'),
    ROOT / 'frontends/windows/src/Shaders.hlsl': ('static const int', 'static const float4', 'float4(', ')'),
}
WORD = 'электроника'
SPACING = 0.14

# Half the stroke, and the centre lines of the top and bottom strokes.
S = 0.085
TOP = 1.0 - S
BOTTOM = S


def arc(cx, cy, rx, ry, start, end, steps):
    """Points along an ellipse from angle `start` to `end`, in degrees."""
    points = []

    for i in range(steps + 1):
        a = math.radians(start + (end - start) * i / steps)
        points.append((cx + rx * math.cos(a), cy + ry * math.sin(a)))

    return points


# Each letter: its width and its paths, in cap heights, y up.
LETTERS = {
    'э': (0.63, [arc(0.315, 0.5, 0.315 - S, 0.5 - S, 140, -140, 12), [(0.17, 0.5), (0.63 - S, 0.5)]]),
    'л': (0.77, [[(0.04, BOTTOM), (0.13, 0.11), (0.19, 0.24), (0.215, 0.45), (0.22, TOP), (0.77 - S, TOP), (0.77 - S, BOTTOM)]]),
    'е': (0.71, [arc(0.355, 0.5, 0.355 - S, 0.5 - S, 0, 318, 14), [(S, 0.5), (0.71 - S, 0.5)]]),
    'к': (0.66, [[(S, BOTTOM), (S, TOP)], [(0.6, TOP), (S, 0.46)], [(0.25, 0.6), (0.62, BOTTOM)]]),
    'т': (0.77, [[(0.03, TOP), (0.74, TOP)], [(0.385, TOP), (0.385, BOTTOM)]]),
    'р': (0.66, [[(S, BOTTOM), (S, TOP), (0.35, TOP)] + arc(0.35, 0.69, 0.225, 0.225, 90, -90, 8) + [(S, 0.465)]]),
    'о': (0.77, [arc(0.385, 0.5, 0.385 - S, 0.5 - S, 0, 360, 18)]),
    'н': (0.66, [[(S, BOTTOM), (S, TOP)], [(0.66 - S, BOTTOM), (0.66 - S, TOP)], [(S, 0.5), (0.66 - S, 0.5)]]),
    'и': (0.66, [[(S, TOP), (S, BOTTOM), (0.66 - S, TOP), (0.66 - S, BOTTOM)]]),
    'а': (0.71, [[(0.04, BOTTOM), (0.355, TOP), (0.67, BOTTOM)], [(0.17, 0.33), (0.54, 0.33)]]),
}

# The emblem's Э and С, beside the spire's foot, in the frame's widths, y
# down: their centres, half-sizes and arcs.
MARK = [
    arc(0.21, 1.42, 0.11, 0.17, 140, -140, 10), [(0.15, 1.42), (0.31, 1.42)],
    arc(0.71, 1.42, 0.11, 0.17, 40, 320, 10),
]


def word_segments():
    out = []
    x = 0.0

    for letter in WORD:
        width, paths = LETTERS[letter]

        for path in paths:
            for (x0, y0), (x1, y1) in zip(path, path[1:]):
                out.append((x + x0, y0, x + x1, y1))

        x += width + SPACING

    return out, x - SPACING


def path_segments(paths):
    return [(x0, y0, x1, y1) for path in paths for (x0, y0), (x1, y1) in zip(path, path[1:])]


def array(int_type, vec_type, open_, close, name, count_name, segments):
    rows = '\n'.join(f'    {open_}{", ".join(f"{c:.3f}" for c in s)}{close},' for s in segments)
    return (f'{int_type} {count_name} = {len(segments)};\n'
            f'{vec_type} {name}[{len(segments)}] = {{\n{rows}\n}};\n')


def main():
    word, width = word_segments()
    mark = path_segments(MARK)

    for path, (int_type, vec_type, open_, close) in SHADERS.items():
        block = ('// badge strokes: made by tools/badge_glyphs.py, not edited by hand\n'
                 f'// The word, {width:.2f} cap heights wide.\n'
                 + array(int_type, vec_type, open_, close, 'kBadgeText', 'kBadgeSegments', word)
                 + array(int_type, vec_type, open_, close, 'kBadgeMark', 'kBadgeMarkSegments', mark)
                 + '// end of badge strokes')
        text = path.read_text(encoding='utf-8')
        new, count = re.subn(r'// badge strokes:.*?// end of badge strokes', lambda _: block, text, flags=re.S)

        if count != 1:
            raise SystemExit(f'{path}: no badge strokes markers')

        path.write_text(new, encoding='utf-8', newline='\n')
        print(f'{path.relative_to(ROOT)}: {len(word)} + {len(mark)} segments, the word {width:.2f} cap heights wide')


if __name__ == '__main__':
    main()

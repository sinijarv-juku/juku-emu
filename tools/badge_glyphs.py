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
SPACING = 0.19

# Half the stroke, and the centre lines of the top and bottom strokes. The
# strokes have square ends, and the shader cuts the letters off at the
# baseline and at the cap height, so strokes that end there run past them
# (to LOW or HIGH) and end flat, as the diagonals of к, и and а do.
S = 0.09
TOP = 1.0 - S
BOTTOM = S
LOW = -0.02
HIGH = 1.02


def arc(cx, cy, rx, ry, start, end, steps):
    """Points along an ellipse from angle `start` to `end`, in degrees."""
    points = []

    for i in range(steps + 1):
        a = math.radians(start + (end - start) * i / steps)
        points.append((cx + rx * math.cos(a), cy + ry * math.sin(a)))

    return points


# Each letter: its width and its paths, in cap heights, y up. The widths are
# measured off the photos.
LETTERS = {
    'э': (0.62, [arc(0.31, 0.5, 0.31 - S, 0.5 - S, 140, -140, 12), [(0.2, 0.5), (0.62 - S, 0.5)]]),
    'л': (0.75, [[(0.0, BOTTOM), (0.1, 0.12), (0.17, 0.25), (0.2, 0.45), (0.21, TOP), (0.75 - S, TOP), (0.75 - S, LOW)]]),
    'е': (0.63, [arc(0.315, 0.5, 0.315 - S, 0.5 - S, 0, 318, 14), [(S, 0.5), (0.63 - S, 0.5)]]),
    # к: a short bar from the stem's middle, an arm that curves up into a
    # vertical stroke at the top right, and a leg down to the bottom right.
    'к': (0.61, [[(S, LOW), (S, HIGH)],
                 [(S, 0.48), (0.33, 0.48), (0.41, 0.5), (0.47, 0.56), (0.51, 0.65), (0.525, 0.77), (0.53, HIGH)],
                 [(0.31, 0.48), (0.53, LOW)]]),
    'т': (0.59, [[(S, TOP), (0.59 - S, TOP)], [(0.295, TOP), (0.295, LOW)]]),
    'р': (0.61, [[(S, LOW), (S, TOP), (0.3, TOP)] + arc(0.3, 0.69, 0.23, 0.23, 90, -90, 8) + [(S, 0.46)]]),
    'о': (0.68, [arc(0.34, 0.5, 0.34 - S, 0.5 - S, 0, 360, 18)]),
    'н': (0.64, [[(S, LOW), (S, HIGH)], [(0.64 - S, LOW), (0.64 - S, HIGH)], [(S, 0.5), (0.64 - S, 0.5)]]),
    'и': (0.66, [[(S, HIGH), (S, BOTTOM), (0.66 - S, TOP), (0.66 - S, LOW)]]),
    'а': (0.66, [[(0.07, LOW), (0.33, HIGH), (0.59, LOW)], [(0.18, 0.3), (0.48, 0.3)]]),
}

# The emblem's Э and С, beside the spire's foot, in the frame's widths, y
# down: their centres, half-sizes and arcs.
MARK = [
    arc(0.21, 1.42, 0.11, 0.17, 140, -140, 10), [(0.15, 1.42), (0.31, 1.42)],
    arc(0.71, 1.42, 0.11, 0.17, 40, 320, 10),
]


def word_segments():
    """The word's segments, and for each which of its ends ends its path: bit
    0 its start, bit 1 its end. Those ends are square. Where segments meet,
    and all round a closed path, the strokes are round, so curves stay
    smooth."""
    out = []
    ends = []
    x = 0.0

    for letter in WORD:
        width, paths = LETTERS[letter]

        for path in paths:
            # A square end needs a direction: no segment of length 0.
            points = [path[0]] + [b for a, b in zip(path, path[1:]) if math.hypot(b[0] - a[0], b[1] - a[1]) > 1e-6]
            closed = math.hypot(points[0][0] - points[-1][0], points[0][1] - points[-1][1]) < 1e-6
            last = len(points) - 2

            for i, ((x0, y0), (x1, y1)) in enumerate(zip(points, points[1:])):
                out.append((x + x0, y0, x + x1, y1))
                ends.append(0 if closed else int(i == 0) | int(i == last) << 1)

        x += width + SPACING

    return out, ends, x - SPACING


def path_segments(paths):
    return [(x0, y0, x1, y1) for path in paths for (x0, y0), (x1, y1) in zip(path, path[1:])]


def array(int_type, vec_type, open_, close, name, count_name, segments):
    rows = '\n'.join(f'    {open_}{", ".join(f"{c:.3f}" for c in s)}{close},' for s in segments)
    return (f'{int_type} {count_name} = {len(segments)};\n'
            f'{vec_type} {name}[{len(segments)}] = {{\n{rows}\n}};\n')


def flags(uint_type, name, values):
    rows = ', '.join(str(v) for v in values)
    return f'{uint_type} {name}[{len(values)}] = {{ {rows} }};\n'


def main():
    word, ends, width = word_segments()
    mark = path_segments(MARK)

    for path, (int_type, vec_type, open_, close) in SHADERS.items():
        block = ('// badge strokes: made by tools/badge_glyphs.py, not edited by hand\n'
                 f'// The word, {width:.2f} cap heights wide.\n'
                 + array(int_type, vec_type, open_, close, 'kBadgeText', 'kBadgeSegments', word)
                 + flags(int_type.replace(' int', ' uint'), 'kBadgeEnds', ends)
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

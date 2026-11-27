#!/usr/bin/env python3
"""Write the lettering of the monitor's Электроника badge into both shaders.

    badge_glyphs.py

The badge on the Juku's МС6105 reads ЭЛЕКТРОНИКА in a bold sans serif,
narrow and with small counters (MD §7.4). The shaders draw it with Whole
Monitor on, as strokes: line segments whose distance gives the letters at
any size, with no texture and no font. The shapes, widths and places were
measured off a close-up photo of the badge.

This lays out the word, in cap heights, x from the start of the word and y
up from the baseline, and writes it between the `badge strokes` markers of
frontends/macos/src/Shaders.metal and frontends/windows/src/Shaders.hlsl.
Each path is the centre line of a stroke 2 S wide, inside its letter's box:
the shaders' kStroke is S. Run it again after changing a letter.
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

# Half the stroke, and the centre lines of the top and bottom strokes. The
# strokes have square ends, and the shader cuts the letters off at the
# baseline and at the cap height, so strokes that end there run past them
# (to LOW or HIGH) and end flat, as the diagonals of К, И and А do.
S = 0.085
TOP = 1.0 - S
BOTTOM = S
LOW = -0.02
HIGH = 1.02


def arc(cx, cy, rx, ry, start, end, steps, n=2.0):
    """Points along a superellipse of exponent n (2: an ellipse) from angle
    `start` to `end`, in degrees. A larger n squares the curve off, as the
    round letters of a grotesque are."""
    points = []

    for i in range(steps + 1):
        a = math.radians(start + (end - start) * i / steps)
        c, s = math.cos(a), math.sin(a)
        points.append((cx + rx * math.copysign(abs(c) ** (2 / n), c), cy + ry * math.copysign(abs(s) ** (2 / n), s)))

    return points


# Each letter: where it starts in the word and its paths, in cap heights, x
# from its left edge. The places and widths are the photo's.
ROUND = 2.6

LETTERS = [
    ('Э', 0.0, [arc(0.2925, 0.5, 0.2925 - S, 0.5 - S, 145, -145, 16, ROUND), [(0.27 + S, 0.44), (0.585 - S, 0.44)]]),
    # Л: the right stem, the top bar, and a left leg that comes down from
    # it and turns a little to the left at the foot.
    ('Л', 0.72, [[(0.54 - S, LOW), (0.54 - S, TOP), (0.27, TOP), (0.215, 0.88), (0.2, 0.7),
                  (0.19, 0.4), (0.17, 0.2), (0.13, 0.08), (0.06, LOW)]]),
    ('Е', 1.394, [[(0.49 - S, TOP), (S, TOP), (S, BOTTOM), (0.54 - S, BOTTOM)], [(S, 0.52), (0.40 - S, 0.52)]]),
    ('К', 2.024, [[(S, LOW), (S, HIGH)], [(0.16, 0.45), (0.57 - S, HIGH)], [(0.25, 0.52), (0.575 - S, LOW)]]),
    ('Т', 2.699, [[(S, TOP), (0.585 - S, TOP)], [(0.30, TOP), (0.30, LOW)]]),
    ('Р', 3.418, [[(S, LOW), (S, TOP), (0.29, TOP)] + arc(0.29, 0.6475, 0.54 - S - 0.29, TOP - 0.6475, 90, -90, 10, ROUND)
                  + [(S, 0.37)]]),
    ('О', 4.093, [arc(0.2925, 0.5, 0.2925 - S, 0.5 - S, 0, 360, 24, ROUND)]),
    ('Н', 4.858, [[(S, LOW), (S, HIGH)], [(0.54 - S, LOW), (0.54 - S, HIGH)], [(S, 0.61), (0.54 - S, 0.61)]]),
    ('И', 5.577, [[(S, LOW), (S, HIGH)], [(0.54 - S, LOW), (0.54 - S, HIGH)], [(S, 0.15), (0.54 - S, 0.85)]]),
    ('К', 6.297, [[(S, LOW), (S, HIGH)], [(0.16, 0.45), (0.57 - S, HIGH)], [(0.25, 0.52), (0.575 - S, LOW)]]),
    ('А', 7.062, [[(0.07, LOW), (0.2925, HIGH), (0.515, LOW)], [(0.15, 0.28), (0.435, 0.28)]]),
]
WIDTH = 7.647


def word_segments():
    """The word's segments, and for each which of its ends ends its path: bit
    0 its start, bit 1 its end. Those ends are square. Where segments meet,
    and all round a closed path, the strokes are round, so curves stay
    smooth."""
    assert ''.join(letter for letter, _, _ in LETTERS) == WORD
    out = []
    ends = []

    for _, x, paths in LETTERS:
        for path in paths:
            # A square end needs a direction: no segment of length 0.
            points = [path[0]] + [b for a, b in zip(path, path[1:]) if math.hypot(b[0] - a[0], b[1] - a[1]) > 1e-6]
            closed = math.hypot(points[0][0] - points[-1][0], points[0][1] - points[-1][1]) < 1e-6
            last = len(points) - 2

            for i, ((x0, y0), (x1, y1)) in enumerate(zip(points, points[1:])):
                out.append((x + x0, y0, x + x1, y1))
                ends.append(0 if closed else int(i == 0) | int(i == last) << 1)

    return out, ends


def array(int_type, vec_type, open_, close, name, count_name, segments):
    rows = '\n'.join(f'    {open_}{", ".join(f"{c:.3f}" for c in s)}{close},' for s in segments)
    return (f'{int_type} {count_name} = {len(segments)};\n'
            f'{vec_type} {name}[{len(segments)}] = {{\n{rows}\n}};\n')


def flags(uint_type, name, values):
    rows = ', '.join(str(v) for v in values)
    return f'{uint_type} {name}[{len(values)}] = {{ {rows} }};\n'


def main():
    word, ends = word_segments()

    for path, (int_type, vec_type, open_, close) in SHADERS.items():
        block = ('// badge strokes: made by tools/badge_glyphs.py, not edited by hand\n'
                 f'// The word, {WIDTH:.2f} cap heights wide.\n'
                 + array(int_type, vec_type, open_, close, 'kBadgeText', 'kBadgeSegments', word)
                 + flags(int_type.replace(' int', ' uint'), 'kBadgeEnds', ends)
                 + '// end of badge strokes')
        text = path.read_text(encoding='utf-8')
        new, count = re.subn(r'// badge strokes:.*?// end of badge strokes', lambda _: block, text, flags=re.S)

        if count != 1:
            raise SystemExit(f'{path}: no badge strokes markers')

        path.write_text(new, encoding='utf-8', newline='\n')
        print(f'{path.relative_to(ROOT)}: {len(word)} segments, the word {WIDTH:.2f} cap heights wide')


if __name__ == '__main__':
    main()

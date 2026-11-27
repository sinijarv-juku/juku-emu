#!/usr/bin/env python3
"""Make core/src/keyboard/glyphs.rs, the keycap lettering as outlines.

    keycap_glyphs.py FONT [-w WEIGHT] [-o OUT]

FONT is Sofia Sans (OFL), the variable font SofiaSans[wght].ttf from
https://github.com/google/fonts/tree/main/ofl/sofiasans, taken at WEIGHT
(700, Bold, by default). Needs fontTools: python3 -m pip install fonttools.

The characters are those of the keycaps' legends in
core/src/keyboard/layout.rs but the arrows, the badge's JUKU E5101, and − (U+2212), which
draws the `_` of the 0 key. Each outline is written in font units, y up,
as the path commands of glyphs.rs. The keyboard drawing (drawing.rs) sets
its text with these, so every front-end draws the same letters without a
font of its own (spec §6.4).
"""
import argparse
import pathlib
import re
import sys

from fontTools.pens.basePen import BasePen
from fontTools.pens.boundsPen import BoundsPen
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

ROOT = pathlib.Path(__file__).resolve().parent.parent
LAYOUT = ROOT / 'core/src/keyboard/layout.rs'
OUT = ROOT / 'core/src/keyboard/glyphs.rs'
EXTRA = 'JUKUE5101\u2212'
ARROWS = '\u2190\u2191\u2192\u2193'  # drawn as shapes, not letters

# Path commands, as glyphs.rs documents them.
MOVE, LINE, QUAD, CUBIC, CLOSE = range(5)


class RecordingPen(BasePen):
    """Records a glyph as path commands, quadratic curves split into single segments."""

    def __init__(self, glyph_set):
        super().__init__(glyph_set)
        self.path = []

    def _moveTo(self, p):
        self.path += [MOVE, *map(round, p)]

    def _lineTo(self, p):
        self.path += [LINE, *map(round, p)]

    def _qCurveToOne(self, p1, p2):
        self.path += [QUAD, *map(round, p1), *map(round, p2)]

    def _curveToOne(self, p1, p2, p3):
        self.path += [CUBIC, *map(round, p1), *map(round, p2), *map(round, p3)]

    def _closePath(self):
        self.path.append(CLOSE)

    _endPath = _closePath


def legend_characters():
    source = LAYOUT.read_text(encoding='utf-8')
    table = source[source.index('pub const KEYCAPS'):]
    table = table[:table.index('];')]
    strings = re.findall(r'"((?:[^"\\]|\\.)*)"', table)
    chars = set(EXTRA)

    for s in strings:
        chars.update(s.replace('\\"', '"').replace('\\\\', '\\'))

    return sorted(chars - set(ARROWS))


def rust_char(c):
    return {'\\': "'\\\\'", "'": "'\\''"}.get(c, f"'{c}'") if c.isprintable() and ord(c) < 0x2000 else f"'\\u{{{ord(c):04x}}}'"


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('font')
    parser.add_argument('-w', '--weight', type=float, default=700)
    parser.add_argument('-o', '--out', type=pathlib.Path, default=OUT)
    args = parser.parse_args()

    font = TTFont(args.font)

    if 'fvar' in font:
        font = instancer.instantiateVariableFont(font, {'wght': args.weight})

    glyph_set = font.getGlyphSet()
    cmap = font.getBestCmap()
    hmtx = font['hmtx']
    name = f"{font['name'].getDebugName(1)}, weight {args.weight:g}"
    lines = []

    for c in legend_characters():
        if ord(c) not in cmap:
            sys.exit(f'{name} has no glyph for {c!r}')

        glyph = cmap[ord(c)]
        pen = RecordingPen(glyph_set)
        glyph_set[glyph].draw(pen)
        bounds = BoundsPen(glyph_set)
        glyph_set[glyph].draw(bounds)
        x0, y0, x1, y1 = (round(v) for v in bounds.bounds or (0, 0, 0, 0))
        path = ', '.join(map(str, pen.path))
        lines.append(f'    Glyph {{ ch: {rust_char(c)}, advance: {hmtx[glyph][0]}, bounds: [{x0}, {y0}, {x1}, {y1}], path: &[{path}] }},')

    os2 = font['OS/2']
    text = f'''//! The keycap lettering as outlines: {name}, made by tools/keycap_glyphs.py
//! from Sofia Sans (SIL Open Font License 1.1, see Sofia-Sans-OFL.txt beside
//! this file). Do not edit; run the tool again.
//!
//! Coordinates are font units, y up from the baseline. A path is a list of
//! commands: 0 move x y, 1 line x y, 2 quad cx cy x y, 3 cubic c1x c1y c2x
//! c2y x y, 4 close. Fill with the nonzero winding rule.

pub struct Glyph {{
    pub ch: char,
    pub advance: i16,
    /// x min, y min, x max, y max
    pub bounds: [i16; 4],
    pub path: &'static [i16],
}}

pub const UNITS_PER_EM: f32 = {font['head'].unitsPerEm}.0;
pub const CAP_HEIGHT: f32 = {os2.sCapHeight}.0;

/// Sorted by character.
pub static GLYPHS: [Glyph; {len(lines)}] = [
{chr(10).join(lines)}
];
'''
    args.out.write_text(text, encoding='utf-8')
    print(f'{args.out}: {len(lines)} glyphs from {name}')


if __name__ == '__main__':
    main()

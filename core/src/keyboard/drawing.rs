//! The keyboard window's picture (spec §6.4): the E5101 keyboard drawn from
//! its layout (MD §6.6), in the colors of the photographs, as a list of
//! drawing commands that every front-end replays. All the drawing decisions
//! are made here, so the front-ends draw the same keyboard; each needs only
//! a small replayer for the commands below, and draws the Mac or PC key
//! labels itself, since their text is the host's.
//!
//! Coordinates are drawing units, 64 to a key, y down, in a picture of
//! [`WIDTH`] × [`HEIGHT`]; the front-end scales it to fit its view. The
//! lettering is set from outlines (`glyphs.rs`), so no font is needed.
//!
//! The commands, as bytes: an opcode, then its operands, numbers as
//! little-endian `f32`, colors as `u32` 0xRRGGBBAA in sRGB, not premultiplied.
//!
//! | Op | Operands | Meaning |
//! |---|---|---|
//! | 01 BACKGROUND | color | fill the whole view, around the picture too |
//! | 02 MOVE | x y | start a subpath |
//! | 03 LINE | x y | |
//! | 04 QUAD | cx cy x y | quadratic Bézier |
//! | 05 CUBIC | c1x c1y c2x c2y x y | cubic Bézier |
//! | 06 CLOSE | | close the subpath |
//! | 10 FILL | color | fill the path, nonzero winding |
//! | 11 LINEAR | x0 y0 x1 y1 n, n × (offset color) | fill the path with a linear gradient, extended at both ends |
//! | 12 RADIAL | cx cy r n, n × (offset color) | fill the path with a radial gradient from the center out to r |
//! | 13 STROKE | width color | stroke the path, butt caps, miter joins |
//! | 20 LABEL | index (u16) cx top height max_width font_size radius padding fill text | a host key label, below |
//!
//! FILL, LINEAR, RADIAL and STROKE use up the path; the next command starts
//! a new one. LABEL draws the label of keycap `index` with the host's
//! interface font, semibold, at `font_size`: a rounded rectangle of `fill`
//! with corner `radius`, `height` high from `top`, centered on `cx`, as wide
//! as the text plus `padding` but at least as wide as it is high, with the
//! text in `text` centered in it. A text wider than `max_width` less
//! `padding` is set smaller to fit.

use super::glyphs::{CAP_HEIGHT, GLYPHS, Glyph};
use super::layout::{BADGE_WIDTH, BADGE_X, KEYCAPS, Keycap, LEFT, Legend, RIGHT, ROWS};

pub const UNIT: f32 = 64.0;
const QUARTER: f32 = UNIT / 4.0;
/// The case around the keys.
const CASE_MARGIN: f32 = 0.4 * UNIT;
pub const WIDTH: f32 = (RIGHT - LEFT) as f32 * QUARTER + 2.0 * CASE_MARGIN;
pub const HEIGHT: f32 = ROWS as f32 * UNIT + 2.0 * CASE_MARGIN;

pub mod op {
    pub const BACKGROUND: u8 = 0x01;
    pub const MOVE: u8 = 0x02;
    pub const LINE: u8 = 0x03;
    pub const QUAD: u8 = 0x04;
    pub const CUBIC: u8 = 0x05;
    pub const CLOSE: u8 = 0x06;
    pub const FILL: u8 = 0x10;
    pub const LINEAR: u8 = 0x11;
    pub const RADIAL: u8 = 0x12;
    pub const STROKE: u8 = 0x13;
    pub const LABEL: u8 = 0x20;
}

// The colors, sampled from photographs of the E5101. They are the object's,
// so they stay the same in light and dark mode.
const CASE: u32 = 0xE9E2C8FF;
const CASE_EDGE: u32 = 0xCFC5A2FF; // the case's edge around the keys
const WELL: u32 = 0x121213FF; // the opening the keys sit in
const SKIRT: u32 = 0x1B1B1DFF;
const FACE_TOP: u32 = 0x45454AFF;
const FACE_BOTTOM: u32 = 0x313135FF;
const LIGHT: u32 = 0xEBE5D3FF; // the light legends
const ORANGE: u32 = 0xE4A03EFF;
const SHEEN: u32 = 0xFFFFFF12; // a face's rim, and the face under the pointer
const LABEL: u32 = 0x84C8EEFF; // a host key label, and the ring of a key down
const LABEL_TEXT: u32 = 0x06263AFF;

// Legend sizes as cap heights. The orange letters are a little smaller than
// the light ones.
const LIGHT_CAP: f32 = 14.4;
const ORANGE_CAP: f32 = 12.9;
const FUNCTION_CAP: f32 = 15.1;
const WORD_CAP: f32 = 11.5;
const WIDE_WORD_CAP: f32 = 14.4;
const TWO_LINES_CAP: f32 = 11.5;
/// The space kept between a face's edges and its legends.
const LEGEND_PADDING: f32 = 8.5;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    fn inset(&self, dx: f32, dy: f32) -> Rect {
        Rect { x: self.x + dx, y: self.y + dy, w: self.w - 2.0 * dx, h: self.h - 2.0 * dy }
    }

    fn union(&self, o: &Rect) -> Rect {
        let (x, y) = (self.x.min(o.x), self.y.min(o.y));
        Rect { x, y, w: (self.x + self.w).max(o.x + o.w) - x, h: (self.y + self.h).max(o.y + o.h) - y }
    }

    fn max_x(&self) -> f32 {
        self.x + self.w
    }

    fn max_y(&self) -> f32 {
        self.y + self.h
    }

    fn mid_x(&self) -> f32 {
        self.x + self.w / 2.0
    }

    fn mid_y(&self) -> f32 {
        self.y + self.h / 2.0
    }
}

/// A key label for LABEL.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Label {
    pub index: u16,
    pub center_x: f32,
    pub top: f32,
    pub height: f32,
    pub max_width: f32,
    pub font_size: f32,
    pub radius: f32,
    pub padding: f32,
    pub fill: u32,
    pub text: u32,
}

/// A drawing command, as [`ops`] reads them.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Background(u32),
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
    Fill(u32),
    Linear { from: (f32, f32), to: (f32, f32), stops: Vec<(f32, u32)> },
    Radial { center: (f32, f32), radius: f32, stops: Vec<(f32, u32)> },
    Stroke { width: f32, color: u32 },
    Label(Label),
}

/// A picture being drawn: the commands as bytes.
#[derive(Clone, Debug, Default)]
pub struct Picture {
    pub bytes: Vec<u8>,
}

impl Picture {
    fn op(&mut self, op: u8, numbers: &[f32]) {
        self.bytes.push(op);

        for n in numbers {
            self.bytes.extend_from_slice(&n.to_le_bytes());
        }
    }

    fn color(&mut self, color: u32) {
        self.bytes.extend_from_slice(&color.to_le_bytes());
    }

    fn stops(&mut self, stops: &[(f32, u32)]) {
        self.bytes.push(stops.len() as u8);

        for &(offset, color) in stops {
            self.bytes.extend_from_slice(&offset.to_le_bytes());
            self.color(color);
        }
    }

    fn background(&mut self, color: u32) {
        self.op(op::BACKGROUND, &[]);
        self.color(color);
    }

    fn move_to(&mut self, x: f32, y: f32) {
        self.op(op::MOVE, &[x, y]);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.op(op::LINE, &[x, y]);
    }

    fn cubic_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        self.op(op::CUBIC, &[c1x, c1y, c2x, c2y, x, y]);
    }

    fn close(&mut self) {
        self.op(op::CLOSE, &[]);
    }

    fn fill(&mut self, color: u32) {
        self.op(op::FILL, &[]);
        self.color(color);
    }

    fn linear(&mut self, from: (f32, f32), to: (f32, f32), stops: &[(f32, u32)]) {
        self.op(op::LINEAR, &[from.0, from.1, to.0, to.1]);
        self.stops(stops);
    }

    fn radial(&mut self, center: (f32, f32), radius: f32, stops: &[(f32, u32)]) {
        self.op(op::RADIAL, &[center.0, center.1, radius]);
        self.stops(stops);
    }

    fn stroke(&mut self, width: f32, color: u32) {
        self.op(op::STROKE, &[width]);
        self.color(color);
    }

    fn label(&mut self, l: &Label) {
        self.bytes.push(op::LABEL);
        self.bytes.extend_from_slice(&l.index.to_le_bytes());

        for n in [l.center_x, l.top, l.height, l.max_width, l.font_size, l.radius, l.padding] {
            self.bytes.extend_from_slice(&n.to_le_bytes());
        }

        self.color(l.fill);
        self.color(l.text);
    }

    /// A rectangle with rounded corners, as a closed subpath.
    fn rounded_rect(&mut self, r: Rect, radius: f32) {
        let k = radius * (1.0 - 0.552_284_8); // control points of a quarter circle
        let (x0, y0, x1, y1) = (r.x, r.y, r.max_x(), r.max_y());
        self.move_to(x0 + radius, y0);
        self.line_to(x1 - radius, y0);
        self.cubic_to(x1 - k, y0, x1, y0 + k, x1, y0 + radius);
        self.line_to(x1, y1 - radius);
        self.cubic_to(x1, y1 - k, x1 - k, y1, x1 - radius, y1);
        self.line_to(x0 + radius, y1);
        self.cubic_to(x0 + k, y1, x0, y1 - k, x0, y1 - radius);
        self.line_to(x0, y0 + radius);
        self.cubic_to(x0, y0 + k, x0 + k, y0, x0 + radius, y0);
        self.close();
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32) {
        self.rounded_rect(Rect { x: cx - r, y: cy - r, w: 2.0 * r, h: 2.0 * r }, r);
    }
}

/// Reads a picture's commands. Stops at the end, or at a malformed command.
pub fn ops(bytes: &[u8]) -> impl Iterator<Item = Op> + '_ {
    let mut at = 0;

    std::iter::from_fn(move || {
        let mut reader = Reader { bytes, at };
        let op = reader.op();
        at = reader.at;
        op
    })
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let b = self.bytes.get(self.at..self.at + N)?.try_into().ok()?;
        self.at += N;
        Some(b)
    }

    fn f(&mut self) -> Option<f32> {
        self.take::<4>().map(f32::from_le_bytes)
    }

    fn color(&mut self) -> Option<u32> {
        self.take::<4>().map(u32::from_le_bytes)
    }

    fn stops(&mut self) -> Option<Vec<(f32, u32)>> {
        let [n] = self.take::<1>()?;
        (0..n).map(|_| Some((self.f()?, self.color()?))).collect()
    }

    fn op(&mut self) -> Option<Op> {
        let [code] = self.take::<1>()?;

        Some(match code {
            op::BACKGROUND => Op::Background(self.color()?),
            op::MOVE => Op::Move(self.f()?, self.f()?),
            op::LINE => Op::Line(self.f()?, self.f()?),
            op::QUAD => Op::Quad(self.f()?, self.f()?, self.f()?, self.f()?),
            op::CUBIC => Op::Cubic(self.f()?, self.f()?, self.f()?, self.f()?, self.f()?, self.f()?),
            op::CLOSE => Op::Close,
            op::FILL => Op::Fill(self.color()?),
            op::LINEAR => Op::Linear { from: (self.f()?, self.f()?), to: (self.f()?, self.f()?), stops: self.stops()? },
            op::RADIAL => Op::Radial { center: (self.f()?, self.f()?), radius: self.f()?, stops: self.stops()? },
            op::STROKE => Op::Stroke { width: self.f()?, color: self.color()? },
            op::LABEL => Op::Label(Label {
                index: u16::from_le_bytes(self.take::<2>()?),
                center_x: self.f()?,
                top: self.f()?,
                height: self.f()?,
                max_width: self.f()?,
                font_size: self.f()?,
                radius: self.f()?,
                padding: self.f()?,
                fill: self.color()?,
                text: self.color()?,
            }),
            _ => return None,
        })
    }
}

// ---- Lettering ---------------------------------------------------------------

fn glyph(c: char) -> Option<&'static Glyph> {
    GLYPHS.binary_search_by(|g| g.ch.cmp(&c)).ok().map(|i| &GLYPHS[i])
}

/// The scale from font units to drawing units for letters `cap` high.
fn scale_for_cap(cap: f32) -> f32 {
    cap / CAP_HEIGHT
}

#[derive(Clone, Copy, PartialEq)]
enum Align {
    Left,
    Center,
    Right,
}

fn text_width(s: &str, cap: f32, kern: f32) -> f32 {
    let scale = scale_for_cap(cap);
    let advances: f32 = s.chars().filter_map(glyph).map(|g| g.advance as f32 * scale).sum();
    advances + kern * (s.chars().count().max(1) - 1) as f32
}

/// Adds a glyph's outline with its pen position at `x`, baseline at `y`.
fn outline(p: &mut Picture, g: &Glyph, x: f32, y: f32, scale: f32) {
    let pt = |gx: i16, gy: i16| (x + gx as f32 * scale, y - gy as f32 * scale);
    let path = g.path;
    let mut i = 0;

    while i < path.len() {
        match path[i] {
            0 => {
                let (px, py) = pt(path[i + 1], path[i + 2]);
                p.move_to(px, py);
                i += 3;
            }
            1 => {
                let (px, py) = pt(path[i + 1], path[i + 2]);
                p.line_to(px, py);
                i += 3;
            }
            2 => {
                let (cx, cy) = pt(path[i + 1], path[i + 2]);
                let (px, py) = pt(path[i + 3], path[i + 4]);
                p.op(op::QUAD, &[cx, cy, px, py]);
                i += 5;
            }
            3 => {
                let (c1x, c1y) = pt(path[i + 1], path[i + 2]);
                let (c2x, c2y) = pt(path[i + 3], path[i + 4]);
                let (px, py) = pt(path[i + 5], path[i + 6]);
                p.cubic_to(c1x, c1y, c2x, c2y, px, py);
                i += 7;
            }
            _ => {
                p.close();
                i += 1;
            }
        }
    }
}

/// How a text is set: letters `cap` high, `kern` apart, in `color`.
#[derive(Clone, Copy)]
struct Ink {
    cap: f32,
    kern: f32,
    color: u32,
}

/// Sets `s` with its baseline at `y`, its left edge, center or right edge
/// at `x`. Returns its width.
fn text(p: &mut Picture, s: &str, ink: Ink, x: f32, y: f32, align: Align) -> f32 {
    let Ink { cap, kern, color } = ink;
    let width = text_width(s, cap, kern);
    let scale = scale_for_cap(cap);
    let mut pen = match align {
        Align::Left => x,
        Align::Center => x - width / 2.0,
        Align::Right => x - width,
    };

    for g in s.chars().filter_map(glyph) {
        outline(p, g, pen, y, scale);
        pen += g.advance as f32 * scale + kern;
    }

    p.fill(color);
    width
}

/// A line of text centered on `cx`, its capitals centered on `cy`.
fn centered(p: &mut Picture, s: &str, cap: f32, kern: f32, cx: f32, cy: f32) {
    text(p, s, Ink { cap, kern, color: LIGHT }, cx, cy + cap / 2.0, Align::Center);
}

/// Where a symbol sits in its corner: at the top, a quarter of the way down,
/// in the middle or on the baseline.
#[derive(Clone, Copy)]
enum Place {
    Top,
    Quarter,
    Middle,
    Baseline,
}

/// The symbols of the two-legend keys, sized as in the photographs relative
/// to the light letters' cap height: by height, or for the bars by width,
/// since a font's own are smaller (a colon as high as a lower-case letter).
struct Symbol {
    legend: char,
    glyph: char, // drawn as this
    by_width: bool,
    ratio: f32,
    place: Place,
}

const fn sym(legend: char, ratio: f32, place: Place) -> Symbol {
    Symbol { legend, glyph: legend, by_width: false, ratio, place }
}

const fn bar(legend: char, ratio: f32, place: Place) -> Symbol {
    Symbol { legend, glyph: '−', by_width: true, ratio, place }
}

use Place::*;

const SYMBOLS: [Symbol; 29] = [
    sym('!', 0.9, Top),
    sym('"', 0.35, Top),
    sym('#', 0.8, Top),
    sym('$', 1.05, Middle),
    sym('%', 0.9, Top),
    sym('&', 0.95, Top),
    sym('\'', 0.35, Top),
    sym('(', 1.05, Middle),
    sym(')', 1.05, Middle),
    bar('_', 0.7, Quarter), // a short dash high in its corner
    Symbol { legend: '=', glyph: '=', by_width: true, ratio: 1.0, place: Middle },
    sym('@', 1.0, Middle),
    sym('^', 0.55, Top),
    sym('{', 1.0, Middle),
    sym('}', 1.0, Middle),
    sym('*', 0.7, Top),
    sym('<', 0.75, Middle),
    sym('>', 0.75, Middle),
    sym('?', 0.95, Top),
    sym('+', 0.8, Middle),
    bar('−', 1.0, Middle),
    sym('\\', 1.0, Baseline),
    sym('[', 0.85, Baseline),
    sym(']', 0.85, Baseline),
    sym(':', 0.8, Baseline),
    sym(',', 0.35, Baseline),
    sym('.', 0.16, Baseline),
    sym('/', 1.2, Baseline),
    sym(';', 0.95, Baseline),
];

/// A corner of a two-legend key: its letters have their cap top at
/// `cap_top` and their baseline at `baseline`, and run from `x` rightward,
/// or for the bottom corner (`right`), leftward to `x`.
struct Corner {
    x: f32,
    right: bool,
    cap_top: f32,
    baseline: f32,
}

/// One legend of a two-legend key in its corner. Letters are set with
/// `ink`; symbols are sized against the light letters' cap height.
fn corner(p: &mut Picture, s: &str, ink: Ink, at: Corner) {
    let Corner { x, right, cap_top, baseline } = at;
    let mut chars = s.chars();
    let symbol = match (chars.next(), chars.next()) {
        (Some(c), None) => SYMBOLS.iter().find(|sym| sym.legend == c),
        _ => None,
    };
    let Some((symbol, g)) = symbol.and_then(|sym| Some((sym, glyph(sym.glyph)?))) else {
        text(p, s, ink, x, baseline, if right { Align::Right } else { Align::Left });
        return;
    };

    let [x0, y0, x1, y1] = g.bounds.map(f32::from);
    let measure = if symbol.by_width { x1 - x0 } else { y1 - y0 };

    if measure <= 0.0 {
        return;
    }

    let scale = symbol.ratio * LIGHT_CAP / measure;
    let (width, height) = ((x1 - x0) * scale, (y1 - y0) * scale);
    let top = match symbol.place {
        Top => cap_top,
        Quarter => cap_top + (baseline - cap_top) / 4.0 - height / 2.0,
        Middle => (cap_top + baseline - height) / 2.0,
        Baseline => baseline - height,
    };
    // The glyph's box is relative to its baseline and pen position.
    let pen = if right { x - width } else { x } - x0 * scale;
    outline(p, g, pen, top + y1 * scale, scale);
    p.fill(ink.color);
}

// ---- Keys ----------------------------------------------------------------

/// Keycap `k`'s whole key, one key high, in the picture.
pub fn key_rect(k: &Keycap) -> Rect {
    Rect {
        x: CASE_MARGIN + (k.x - LEFT) as f32 * QUARTER,
        y: CASE_MARGIN + k.row as f32 * UNIT,
        w: k.width as f32 * QUARTER,
        h: UNIT,
    }
}

/// The arrows as printed on the Juku: a thin shaft and a solid head.
fn arrow(p: &mut Picture, which: &str, cx: f32, cy: f32) {
    let (dx, dy) = match which {
        "←" => (-1.0, 0.0),
        "→" => (1.0, 0.0),
        "↑" => (0.0, -1.0),
        _ => (0.0, 1.0),
    };
    let half = if dy != 0.0 { 15.0 } else { 18.0 };
    let (head_length, head_width) = (9.5, 5.5);
    let tip = (cx + dx * half, cy + dy * half);
    let base = (tip.0 - dx * head_length, tip.1 - dy * head_length);
    p.move_to(cx - dx * half, cy - dy * half);
    p.line_to(base.0, base.1);
    p.stroke(2.4, LIGHT);
    p.move_to(tip.0, tip.1);
    p.line_to(base.0 + dy * head_width, base.1 + dx * head_width);
    p.line_to(base.0 - dy * head_width, base.1 - dx * head_width);
    p.close();
    p.fill(LIGHT);
}

/// Draws keycap `k` filling `key`: the skirt, then the face, which moves
/// down while the key is down. The legends are where they are on the real
/// key, LEGEND_PADDING inside the face. A host key label, if `label` gives
/// its index, sits across the face's bottom edge, centered, like a label
/// stuck on the key, so that it never covers a legend.
fn keycap(p: &mut Picture, k: &Keycap, key: Rect, down: bool, hover: bool, label: Option<u16>) {
    let skirt = key.inset(2.5, 2.5);
    p.rounded_rect(skirt, 6.0);
    p.fill(SKIRT);

    let face = Rect { x: skirt.x + 4.5, y: skirt.y + 3.0 + if down { 2.6 } else { 0.0 }, w: skirt.w - 9.0, h: skirt.h - 13.0 };
    p.rounded_rect(face, 4.5);
    p.linear((face.mid_x(), face.y), (face.mid_x(), face.max_y()), &[(0.0, FACE_TOP), (1.0, FACE_BOTTOM)]);
    p.rounded_rect(face.inset(0.5, 0.5), 4.0);
    p.stroke(1.0, SHEEN);

    if hover && !down {
        p.rounded_rect(face, 4.5);
        p.fill(SHEEN);
    }

    if down {
        p.rounded_rect(face.inset(-1.0, -1.0), 5.5);
        p.stroke(2.0, LABEL);
    }

    let (cx, cy) = (face.mid_x(), face.mid_y());
    let inside = face.inset(LEGEND_PADDING, LEGEND_PADDING);

    match k.legend {
        Legend::Pair => {
            let light = Ink { cap: LIGHT_CAP, kern: 0.0, color: LIGHT };
            let orange = Ink { cap: ORANGE_CAP, kern: 0.0, color: ORANGE };
            corner(p, k.top, light, Corner { x: inside.x, right: false, cap_top: inside.y, baseline: inside.y + LIGHT_CAP });
            let bottom = Corner { x: inside.max_x(), right: true, cap_top: inside.max_y() - ORANGE_CAP, baseline: inside.max_y() };
            corner(p, k.bottom, orange, bottom);
        }
        Legend::Function => centered(p, k.top, FUNCTION_CAP, 0.0, cx, cy),
        Legend::Word => {
            // ERASE fills the face from side to side.
            let width = text_width(k.top, WORD_CAP, 0.3);
            centered(p, k.top, WORD_CAP.min(WORD_CAP * inside.w / width), 0.3, cx, cy);
        }
        Legend::WideWord => centered(p, k.top, WIDE_WORD_CAP, 1.5, cx, cy),
        Legend::TwoLines => {
            let step = (TWO_LINES_CAP + 3.0) / 2.0; // the lines' centers from the face's
            centered(p, k.top, TWO_LINES_CAP, 0.3, cx, cy - step);
            centered(p, k.bottom, TWO_LINES_CAP, 0.3, cx, cy + step);
        }
        Legend::Arrow => arrow(p, k.top, cx, cy),
        Legend::Blank => {}
    }

    if let Some(index) = label {
        p.label(&Label {
            index,
            center_x: face.mid_x(),
            top: face.max_y() - 6.5,
            height: 14.5,
            max_width: face.w - 8.0,
            font_size: 12.0,
            radius: 3.5,
            padding: 7.0,
            fill: LABEL,
            text: LABEL_TEXT,
        });
    }
}

/// The JUKU E5101 badge beside F8, with the power LED lit.
fn badge(p: &mut Picture) {
    let plate = Rect {
        x: CASE_MARGIN + (BADGE_X - LEFT) as f32 * QUARTER,
        y: CASE_MARGIN + 0.16 * UNIT,
        w: BADGE_WIDTH as f32 * QUARTER,
        h: 0.68 * UNIT,
    };
    p.rounded_rect(plate, 2.5);
    p.fill(0x151515FF);
    p.rounded_rect(plate, 2.5);
    p.stroke(1.0, 0x2A2A2AFF);
    let name = Ink { cap: 22.3, kern: 2.5, color: 0xF1EFE8FF };
    let model = Ink { cap: 9.3, kern: 1.0, color: 0xC9C6BDFF };
    text(p, "JUKU", name, plate.x + 16.0, plate.mid_y() + 11.0, Align::Left);
    text(p, "E5101", model, plate.x + 2.42 * UNIT, plate.mid_y() + 8.0, Align::Left);

    let (lx, ly) = (plate.max_x() - 0.34 * UNIT, plate.y + plate.h * 0.32);
    p.circle(lx, ly, 13.0);
    p.radial((lx, ly), 13.0, &[(0.0, 0xFF3B2F8C), (1.0, 0xFF3B2F00)]);
    p.circle(lx, ly, 6.2);
    p.fill(0x8D8A83FF);
    p.circle(lx, ly, 4.6);
    p.radial((lx, ly), 4.6, &[(0.0, 0xFFD0C8FF), (0.35, 0xFF3B2FFF), (1.0, 0x9A0D08FF)]);
}

/// What the keyboard shows, per keycap of [`KEYCAPS`].
pub struct KeyboardState<'a> {
    pub down: &'a [bool],
    /// Keycaps with a host key label.
    pub labeled: &'a [bool],
    /// The keycap under the pointer.
    pub hover: Option<usize>,
}

/// The whole keyboard: the ivory case, the opening the keys sit in, the
/// badge and the keycaps.
pub fn keyboard(state: &KeyboardState) -> Picture {
    let mut p = Picture { bytes: Vec::with_capacity(96 * 1024) };
    p.background(CASE);

    let mut rows: [Option<Rect>; ROWS as usize] = [None; ROWS as usize];

    for k in &KEYCAPS {
        let key = key_rect(k);
        let row = &mut rows[k.row as usize];
        *row = Some(row.map_or(key, |r| r.union(&key)));
    }

    for row in rows.iter().flatten() {
        p.rounded_rect(row.inset(-5.0, -5.0), 5.0);
        p.fill(CASE_EDGE);
    }

    for row in rows.iter().flatten() {
        p.rounded_rect(row.inset(-2.0, -2.0), 3.0);
        p.fill(WELL);
    }

    badge(&mut p);

    for (i, k) in KEYCAPS.iter().enumerate() {
        let down = state.down.get(i).copied().unwrap_or(false);
        let label = state.labeled.get(i).copied().unwrap_or(false).then_some(i as u16);
        keycap(&mut p, k, key_rect(k), down, state.hover == Some(i), label);
    }

    p
}

/// One keycap on a piece of the opening, for a picture of the key alone,
/// and the picture's width and height. The space bar is cut to three keys.
pub fn keycap_alone(index: usize) -> Option<(Picture, f32, f32)> {
    let k = KEYCAPS.get(index)?;
    let width = k.width.min(12) as f32 * QUARTER + 8.0;
    let height = UNIT + 8.0;
    let mut p = Picture::default();
    p.rounded_rect(Rect { x: 0.0, y: 0.0, w: width, h: height }, 6.0);
    p.fill(WELL);
    keycap(&mut p, k, Rect { x: 4.0, y: 4.0, w: width - 8.0, h: UNIT }, false, false, None);
    Some((p, width, height))
}

/// The keycap at a point of the picture, as its index in [`KEYCAPS`].
pub fn keycap_at(x: f32, y: f32) -> Option<usize> {
    KEYCAPS.iter().position(|k| key_rect(k).inset(1.0, 1.0).contains(x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_legend_has_letters() {
        for k in &KEYCAPS {
            if k.legend == Legend::Arrow {
                continue;
            }

            for c in k.top.chars().chain(k.bottom.chars()) {
                assert!(glyph(c).is_some(), "no outline for {c:?}");
            }
        }

        for c in "JUKUE5101−".chars() {
            assert!(glyph(c).is_some(), "no outline for {c:?}");
        }

        assert!(GLYPHS.windows(2).all(|w| w[0].ch < w[1].ch), "glyphs sorted");
    }

    #[test]
    fn picture_reads_back_and_stays_inside() {
        let n = KEYCAPS.len();
        let mut down = vec![false; n];
        down[3] = true;
        let labeled = vec![true; n];
        let p = keyboard(&KeyboardState { down: &down, labeled: &labeled, hover: Some(10) });
        let ops: Vec<Op> = ops(&p.bytes).collect();
        assert_eq!(ops[0], Op::Background(CASE));
        assert_eq!(ops.iter().filter(|o| matches!(o, Op::Label(_))).count(), n);

        for o in &ops {
            let points: Vec<(f32, f32)> = match *o {
                Op::Move(x, y) | Op::Line(x, y) => vec![(x, y)],
                Op::Quad(a, b, x, y) => vec![(a, b), (x, y)],
                Op::Cubic(a, b, c, d, x, y) => vec![(a, b), (c, d), (x, y)],
                _ => vec![],
            };

            for (x, y) in points {
                assert!((-1.0..=WIDTH + 1.0).contains(&x) && (-1.0..=HEIGHT + 1.0).contains(&y), "({x}, {y}) outside");
            }
        }

        // Every byte is read: the last command ends the picture.
        let mut end = 0;
        let mut reader = Reader { bytes: &p.bytes, at: 0 };

        while reader.op().is_some() {
            end = reader.at;
        }

        assert_eq!(end, p.bytes.len(), "the whole picture is commands");
    }

    #[test]
    fn state_changes_the_picture() {
        let n = KEYCAPS.len();
        let none = vec![false; n];
        let base = keyboard(&KeyboardState { down: &none, labeled: &none, hover: None }).bytes;
        let mut down = none.clone();
        down[0] = true;
        assert_ne!(base, keyboard(&KeyboardState { down: &down, labeled: &none, hover: None }).bytes);
        assert_ne!(base, keyboard(&KeyboardState { down: &none, labeled: &none, hover: Some(0) }).bytes);
    }

    #[test]
    fn hit_test_finds_each_key() {
        for (i, k) in KEYCAPS.iter().enumerate() {
            let r = key_rect(k);
            let found = keycap_at(r.mid_x(), r.mid_y());
            // The two SHIFTs are separate keycaps with one ID.
            assert_eq!(found, Some(i), "{}", k.top);
        }

        assert_eq!(keycap_at(1.0, 1.0), None);
    }
}

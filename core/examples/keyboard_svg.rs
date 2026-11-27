//! Writes the keyboard window's picture (core/src/keyboard/drawing.rs) as
//! SVG, a reference for what a front-end's replayer should draw. The host
//! key labels are drawn with each key's legend as a stand-in for its text.
//! Usage: cargo run --profile test --example keyboard_svg -- [out.svg] [down,down,...]
//! The optional list holds keycap indexes to draw down.
use std::fmt::Write;

use juku::keyboard::drawing::{self, HEIGHT, KeyboardState, Op, WIDTH};
use juku::keyboard::layout::KEYCAPS;

fn color(c: u32) -> String {
    format!("#{:06x}\" fill-opacity=\"{:.3}", c >> 8, (c & 0xFF) as f32 / 255.0)
}

fn stops(s: &mut String, stops: &[(f32, u32)]) {
    for &(offset, c) in stops {
        let _ = write!(s, r##"<stop offset="{offset}" stop-color="#{:06x}" stop-opacity="{:.3}"/>"##, c >> 8, (c & 0xFF) as f32 / 255.0);
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "keyboard.svg".into());
    let mut down = vec![false; KEYCAPS.len()];

    for i in args.next().iter().flat_map(|l| l.split(',')).filter_map(|i| i.parse::<usize>().ok()) {
        down[i] = true;
    }

    let labeled: Vec<bool> = KEYCAPS.iter().map(|k| !k.top.is_empty()).collect();
    let picture = drawing::keyboard(&KeyboardState { down: &down, labeled: &labeled, hover: None });
    let mut svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {WIDTH} {HEIGHT}" width="{WIDTH}" height="{HEIGHT}">"#);
    let mut path = String::new();
    let mut gradients = 0;

    for op in drawing::ops(&picture.bytes) {
        match op {
            Op::Background(c) => {
                let _ = write!(svg, r#"<rect width="100%" height="100%" fill="{}"/>"#, color(c));
            }
            Op::Move(x, y) => {
                let _ = write!(path, "M{x} {y}");
            }
            Op::Line(x, y) => {
                let _ = write!(path, "L{x} {y}");
            }
            Op::Quad(a, b, x, y) => {
                let _ = write!(path, "Q{a} {b} {x} {y}");
            }
            Op::Cubic(a, b, c, d, x, y) => {
                let _ = write!(path, "C{a} {b} {c} {d} {x} {y}");
            }
            Op::Close => path.push('Z'),
            Op::Fill(c) => {
                let _ = write!(svg, r#"<path d="{path}" fill="{}" fill-rule="nonzero"/>"#, color(c));
                path.clear();
            }
            Op::Linear { from, to, stops: s } => {
                gradients += 1;
                let _ = write!(
                    svg,
                    r#"<linearGradient id="g{gradients}" gradientUnits="userSpaceOnUse" x1="{}" y1="{}" x2="{}" y2="{}">"#,
                    from.0, from.1, to.0, to.1
                );
                stops(&mut svg, &s);
                let _ = write!(svg, r#"</linearGradient><path d="{path}" fill="url(#g{gradients})"/>"#);
                path.clear();
            }
            Op::Radial { center, radius, stops: s } => {
                gradients += 1;
                let _ = write!(
                    svg,
                    r#"<radialGradient id="g{gradients}" gradientUnits="userSpaceOnUse" cx="{}" cy="{}" r="{radius}">"#,
                    center.0, center.1
                );
                stops(&mut svg, &s);
                let _ = write!(svg, r#"</radialGradient><path d="{path}" fill="url(#g{gradients})"/>"#);
                path.clear();
            }
            Op::Stroke { width, color: c } => {
                let _ = write!(
                    svg,
                    r##"<path d="{path}" fill="none" stroke="#{:06x}" stroke-opacity="{:.3}" stroke-width="{width}"/>"##,
                    c >> 8,
                    (c & 0xFF) as f32 / 255.0
                );
                path.clear();
            }
            Op::Label(l) => {
                let k = &KEYCAPS[l.index as usize];
                let text = k.top.to_lowercase();
                let w = (text.chars().count() as f32 * l.font_size * 0.6 + l.padding).clamp(l.height, l.max_width);
                let _ = write!(
                    svg,
                    r#"<rect x="{}" y="{}" width="{w}" height="{}" rx="{}" fill="{}"/><text x="{}" y="{}" font-family="sans-serif" font-weight="600" font-size="{}" text-anchor="middle" fill="{}">{}</text>"#,
                    l.center_x - w / 2.0,
                    l.top,
                    l.height,
                    l.radius,
                    color(l.fill),
                    l.center_x,
                    l.top + l.height / 2.0 + l.font_size * 0.35,
                    l.font_size,
                    color(l.text),
                    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
                );
            }
        }
    }

    svg.push_str("</svg>\n");
    std::fs::write(&out, svg).unwrap();
    println!("{out}: {} bytes of commands", picture.bytes.len());
}

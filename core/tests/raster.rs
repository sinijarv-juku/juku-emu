//! juku-sdk's raster demos (juku-demos), run from the expansion window: the picture
//! moves where the timers put it, and video memory changed while a frame is
//! drawn shows as the beam would show it (spec §4.5).

mod common;

use common::*;
use juku::Machine;
use juku::machine::CPU_HZ;
use juku::video::{RASTER_BYTES, RASTER_LINES};

use juku_demos::{BOUNCE, Demo, SCROLLER};

/// Boots with the demo plugged in and starts it with A.
fn start(demo: &Demo) -> (Machine, Vec<u8>) {
    let rom = load_rom();
    let mut m = Machine::new(&rom).unwrap();
    m.insert_expansion(demo.image).unwrap();
    run_to_prompt(&mut m, &rom, 10.0);
    type_text(&mut m, "a");
    m.run_cycles(CPU_HZ / 2);
    (m, rom)
}

fn next_frame(m: &mut Machine) {
    let seq = m.frame().seq;
    while m.frame().seq == seq {
        m.run_cycles(200);
    }
}

/// The 40 bytes of the RomBios picture on raster line `y`.
fn line(m: &Machine, y: usize) -> &[u8] {
    &m.frame().data[y * RASTER_BYTES + 16..][..40]
}

/// The first raster line with anything lit.
fn top(m: &Machine) -> Option<usize> {
    (0..RASTER_LINES).find(|&y| line(m, y).iter().any(|&b| b != 0))
}

/// bounce.bin lengthens the vertical blank a frame at a time: the bar moves
/// from the top of the picture (line 47) down 201 lines and back.
#[test]
fn bounce_moves_the_bar_by_the_vertical_blank() {
    let (mut m, rom) = start(&BOUNCE);
    let mut tops = Vec::new();
    for _ in 0..100 {
        next_frame(&mut m);
        tops.push(top(&m).expect("the bar"));
    }
    let (min, max) = (*tops.iter().min().unwrap(), *tops.iter().max().unwrap());
    assert_eq!((min, max), (47, 47 + 201), "{tops:?}");
    assert!(tops.windows(2).all(|w| w[0].abs_diff(w[1]) <= 7), "smooth: {tops:?}");

    type_text(&mut m, "x");
    run_to_prompt(&mut m, &rom, 5.0);
    next_frame(&mut m);
    assert_eq!(m.frame().active_y, 47, "the BIOS timing again");
    let text = screen_text(&m, &rom);
    assert!(text.iter().any(|l| l.starts_with("RomBios")), "{text:?}");
}

/// scroller.bin shifts its text band, lines drawn in pairs, after the beam
/// has passed it. A frame copied at the wrong moment shows a pair that
/// differs: the copy caught the shift between its two lines.
#[test]
fn scroller_text_is_never_caught_half_shifted() {
    let (mut m, _) = start(&SCROLLER);
    let mut tops = Vec::new();
    for i in 0..300 {
        next_frame(&mut m);
        let t = top(&m).expect("the bar");
        tops.push(t);
        for r in 0..9 {
            let y = t + 10 + 2 * r;
            assert_eq!(line(&m, y), line(&m, y + 1), "frame {i}, bar at {t}: lines {y} and {}", y + 1);
        }
    }
    let (min, max) = (*tops.iter().min().unwrap(), *tops.iter().max().unwrap());
    assert_eq!((min, max), (47, 47 + 203), "the bounce");
}

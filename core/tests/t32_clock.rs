//! The clock of PIT #3 counter 2, T3.2 (MD §10.2, docs/T3.2_and_TIMCNT.md).
//! On the frame clock, the default, the RomBios stopwatch TIMCNT counts
//! frames. On 1.23 MHz the RDEM demo gets random numbers from it, and
//! sprinkles random pixels around the JUKU logo. rdem.com is on the programs
//! disk, jukprog1.juk; that test needs JUKU_TEST_DISKS pointing to the
//! directory holding it and is skipped otherwise.

mod common;

use common::*;
use juku::machine::CPU_HZ;

/// The stopwatch's count in RAM, which the frame interrupt adds T3.2's
/// pulses to (ROM 053D).
const TIMCNT: usize = 0xD463;

fn timcnt(m: &juku::Machine) -> u16 {
    u16::from_le_bytes([m.ram()[TIMCNT], m.ram()[TIMCNT + 1]])
}

/// What the next frame interrupt adds to TIMCNT.
fn frame_step(m: &mut juku::Machine) -> u16 {
    let next = |m: &mut juku::Machine| {
        let before = timcnt(m);
        while timcnt(m) == before {
            m.run_cycles(500);
        }
        timcnt(m)
    };
    let start = next(m);
    next(m).wrapping_sub(start)
}

#[test]
fn timcnt_counts_frames_on_the_frame_clock() {
    for name in ROMS {
        let rom = load_rom_named(name);
        let mut m = juku::Machine::new(&rom).unwrap();
        run_to_prompt(&mut m, &rom, 10.0);
        let start = timcnt(&m);
        m.run_cycles(CPU_HZ);
        assert_eq!(timcnt(&m).wrapping_sub(start), 50, "{name}: frame clock");

        // On 1.23 MHz a frame adds its 24 655 pulses, less those between the
        // latch and the reload.
        m.set_t32_fast_clock(true);
        let step = frame_step(&mut m);
        assert!((24_500..=24_655).contains(&step), "{name}: 1.23 MHz, {step} in a frame");

        // And back.
        m.set_t32_fast_clock(false);
        m.run_cycles(CPU_HZ / 10);
        assert_eq!(frame_step(&mut m), 1, "{name}: frame clock again");
    }
}

/// Runs RDEM for 8 s from when it clears the screen, and returns the lit
/// pixels below the logo, on lines 160–240, and the number of columns they
/// are in.
fn rdem_pixels(image: &[u8], t32_fast: bool) -> (usize, usize) {
    let rom = load_rom();
    let mut m = juku::Machine::new(&rom).unwrap();
    m.set_t32_fast_clock(t32_fast);
    m.insert_disk(0, image.to_vec(), true).unwrap();
    boot_ekdos(&mut m, &rom);
    start_program(&mut m, &rom, "rdem");
    m.run_cycles(8 * CPU_HZ);

    let f = m.frame();
    let mut lit: Vec<usize> = (160..f.active_h as usize)
        .flat_map(|y| {
            let line = f.active_line(y);
            (0..line.len() * 8).filter(move |&x| line[x / 8] & 0x80 >> (x % 8) != 0)
        })
        .collect();
    let pixels = lit.len();
    lit.sort();
    lit.dedup();
    (pixels, lit.len())
}

#[test]
fn rdem_needs_the_fast_clock() {
    let Some(image) = test_file(PROGRAMS_DISK) else { return };

    // Every random bit is 1: the pixels fall on two places above the logo.
    assert_eq!(rdem_pixels(&image, false), (0, 0), "frame clock");

    let (pixels, columns) = rdem_pixels(&image, true);
    assert!(pixels >= 40 && columns >= 30, "1.23 MHz: {pixels} pixels in {columns} columns");
}

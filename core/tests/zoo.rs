//! Runs the ZOO game, which switches to a wider video mode (48 bytes ×
//! 201 lines) for its playfield. zoo.com is on the games disk,
//! jukgame1.juk. Needs JUKU_TEST_DISKS pointing to the directory holding
//! it; skipped otherwise.

mod common;

use common::*;
use juku::machine::CPU_HZ;

#[test]
fn zoo_demo_uses_384x201_mode() {
    let Some(image) = test_file(GAMES_DISK) else { return };
    let rom = load_rom();
    let mut m = juku::Machine::new(&rom).unwrap();
    m.insert_disk(0, image, true).unwrap();
    boot_ekdos(&mut m, &rom);

    // The title screen is in the RomBios mode.
    start_program(&mut m, &rom, "zoo");
    m.run_cycles(2 * CPU_HZ);
    let f = m.frame();
    assert_eq!((f.active_w, f.active_h), (40, 241), "title screen");

    // Demo: PIT #1 ch1/ch2 ← 16, 4 µs and PIT #2 ch1/ch2 ← 112, 45 lines.
    type_text(&mut m, "2");
    m.run_cycles(2 * CPU_HZ);
    let f = m.frame();
    assert_eq!((f.active_w, f.active_h), (48, 201), "playfield mode");
    assert_eq!((f.active_x, f.active_y), (12, 67));

    // The playfield reaches past the 320 pixels of the RomBios mode.
    let wide = (0..f.active_h as usize).any(|y| f.active_line(y)[44..48].iter().any(|&b| b != 0));
    assert!(wide, "nothing drawn in columns 352–383");
}

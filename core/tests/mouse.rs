//! Runs the gtr.com graphics editor and moves its cursor with the emulated
//! mouse. gtr.com is on the programs disk, jukprog1.juk. Needs
//! JUKU_TEST_DISKS pointing to the directory holding it; skipped otherwise.

mod common;

use common::*;
use juku::Machine;
use juku::machine::CPU_HZ;

/// Bounding box (x0, y0, x1, y1) of the pixels that differ between two frames.
fn changed_box(a: &[u8], b: &[u8], bpl: usize) -> Option<(usize, usize, usize, usize)> {
    let mut bbox: Option<(usize, usize, usize, usize)> = None;
    for (i, (&x, &y)) in a.iter().zip(b).enumerate() {
        let diff = x ^ y;
        for bit in 0..8 {
            if diff & (0x80 >> bit) != 0 {
                let (px, py) = ((i % bpl) * 8 + bit, i / bpl);
                bbox = Some(match bbox {
                    None => (px, py, px, py),
                    Some((x0, y0, x1, y1)) => (x0.min(px), y0.min(py), x1.max(px), y1.max(py)),
                });
            }
        }
    }
    bbox
}

/// Writes the frame as PBM to $JUKU_DUMP_DIR/<name>.pbm when set (debugging).
fn dump(m: &Machine, name: &str) {
    if let Ok(dir) = std::env::var("JUKU_DUMP_DIR") {
        let f = m.frame();
        let mut pbm = format!("P4\n{} {}\n", f.active_line(0).len() * 8, f.active_h).into_bytes();
        pbm.extend_from_slice(&snapshot(m));
        std::fs::write(format!("{dir}/{name}.pbm"), pbm).unwrap();
    }
}

fn snapshot(m: &Machine) -> Vec<u8> {
    m.frame().active_picture()
}

/// Bounding box of the lit pixels above gtr's status line: the cursor, on an
/// empty canvas.
fn cursor_box(m: &Machine) -> Option<(usize, usize, usize, usize)> {
    let blank = vec![0u8; 40 * 220];
    changed_box(&blank, &snapshot(m)[..40 * 220], 40)
}

#[test]
fn gtr_cursor_follows_mouse() {
    let Some(image) = test_file(PROGRAMS_DISK) else { return };
    let rom = load_rom();
    let mut m = Machine::new(&rom).unwrap();
    m.insert_disk(0, image, true).unwrap();
    boot_ekdos(&mut m, &rom);
    type_text(&mut m, "gtr\r");
    run_until(&mut m, &rom, 30.0, |l| l.contains("mouse attached"));
    type_text(&mut m, "y");
    run_until(&mut m, &rom, 10.0, |l| l.contains("Press any key"));
    type_text(&mut m, " ");
    m.run_cycles(CPU_HZ * 2);
    dump(&m, "gtr_start");
    let start = cursor_box(&m).expect("cursor on the canvas");

    // 1 count = 1 pixel in gtr.
    m.mouse_input(40, 30, 0);
    m.run_cycles(CPU_HZ / 2);
    dump(&m, "gtr_moved");
    let moved = cursor_box(&m).expect("cursor on the canvas");
    println!("cursor {start:?} -> {moved:?}");
    assert_eq!((moved.0 - start.0, moved.1 - start.1), (40, 30));

    // Holding the left button in Draw mode leaves a trail.
    m.mouse_input(0, 0, juku::mouse::BUTTON_LEFT);
    m.run_cycles(CPU_HZ / 5);
    m.mouse_input(50, 0, juku::mouse::BUTTON_LEFT);
    m.run_cycles(CPU_HZ / 2);
    m.mouse_input(0, 0, 0);
    m.run_cycles(CPU_HZ / 5);
    dump(&m, "gtr_drawn");
    let drawn = cursor_box(&m).expect("drawing");
    println!("after drawing: {drawn:?}");
    assert!(drawn.2 - drawn.0 >= 50, "a line about 50 pixels long");
}

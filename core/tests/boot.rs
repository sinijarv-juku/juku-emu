//! Boots the supplied RomBios 3.43m ROM headless and reads the screen back as
//! text by matching ROM font glyphs against the captured frame.

mod common;

use common::*;
use juku::Machine;
use juku::machine::CPU_HZ;

#[test]
fn boots_to_monitor_prompt() {
    let rom = load_rom();
    let mut m = Machine::new(&rom).unwrap();
    let t = run_to_prompt(&mut m, &rom, 10.0);
    let text = screen_text(&m, &rom);
    println!("cold boot to prompt in {t:.1} s:\n{}", text.join("\n"));

    assert!(text[0].starts_with("EktaSoft '88"), "{text:?}");
    assert!(text.iter().any(|l| l.starts_with("DiskBios (Fdc 1793")), "{text:?}");
    let f = m.frame();
    assert_eq!((f.active_w, f.active_h), (40, 241));
}

#[test]
fn warm_reset_returns_to_prompt() {
    let rom = load_rom();
    let mut m = Machine::new(&rom).unwrap();
    run_to_prompt(&mut m, &rom, 10.0);
    m.reset(false);
    m.run_cycles(CPU_HZ / 10);
    let t = run_to_prompt(&mut m, &rom, 10.0);
    println!("warm boot to prompt in {t:.1} s");
}

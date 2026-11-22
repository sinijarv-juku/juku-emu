//! The guest Caps Lock follows the host's (spec §4.5): RomBios keeps it in
//! CONSW bit 0, and the core presses CAPS while the two differ.

mod common;

use common::*;
use juku::Machine;
use juku::keyboard::CONSW;
use juku::machine::CPU_HZ;

fn caps(m: &Machine) -> bool {
    m.ram()[CONSW as usize] & 1 != 0
}

#[test]
fn caps_lock_follows_the_host() {
    let rom = load_rom();
    let mut m = Machine::new(&rom).unwrap();
    m.set_caps_lock(true);
    run_to_prompt(&mut m, &rom, 10.0);
    m.run_cycles(CPU_HZ / 2);
    assert!(caps(&m), "on after a cold boot");
    type_text(&mut m, "a");
    let text = screen_text(&m, &rom);
    assert!(text.iter().any(|l| l == "*A"), "typed in capitals:\n{}", text.join("\n"));

    m.reset(false);
    m.run_cycles(CPU_HZ / 10);
    run_to_prompt(&mut m, &rom, 10.0);
    m.run_cycles(CPU_HZ / 2);
    assert!(caps(&m), "on again after a warm reset, which clears CONSW");

    m.set_caps_lock(false);
    m.run_cycles(CPU_HZ / 2);
    assert!(!caps(&m), "off with the host");
}

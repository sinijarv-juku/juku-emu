//! The key click can be turned off by patching the ROM (spec §4.3). The ROM
//! test must still pass, and a key press must no longer start the tone.

mod common;

use common::*;
use juku::Machine;
use juku::machine::CPU_HZ;

/// Writes to PIT #3 ch1, the sound counter, while typing a space at the
/// monitor prompt (answered with `?`, no bell).
fn tone_writes(m: &mut Machine) -> usize {
    m.log_io(true);
    type_text(m, " ");
    m.log_io(false).iter().filter(|&&(_, port, _)| port == 0x19).count()
}

#[test]
fn key_click_can_be_turned_off() {
    let rom = load_rom();
    let mut m = Machine::new(&rom).unwrap();
    assert!(m.set_key_click(true), "click code found in the supplied ROM");
    run_to_prompt(&mut m, &rom, 10.0);
    assert!(tone_writes(&mut m) > 0, "clicks by default");

    assert!(m.set_key_click(false));
    assert!(m.set_key_click(false), "turning it off twice is harmless");
    assert_eq!(tone_writes(&mut m), 0, "silent when off");

    m.reset(true);
    run_to_prompt(&mut m, &rom, 10.0);
    assert_eq!(tone_writes(&mut m), 0, "the ROM test passes with the patch, and it stays off");

    assert!(m.set_key_click(true));
    assert!(tone_writes(&mut m) > 0, "clicks again");
    m.reset(true);
    run_to_prompt(&mut m, &rom, 10.0);
}

/// Every bundled ROM can be patched, and stays patched across a cold reset.
/// The screen is not read, because the fonts and prompts differ.
#[test]
fn key_click_can_be_turned_off_in_every_rom() {
    for name in ROMS {
        let rom = load_rom_named(name);
        let mut m = Machine::new(&rom).unwrap();
        assert!(m.set_key_click(false), "{name}: click code found");
        // Mode 0 after reset: 0000–3FFF reads the ROM.
        let sum = (0x1000..0x1800).fold(0u8, |s, a| s.wrapping_add(m.peek(a)));
        assert_eq!(sum, m.peek(0x0008), "{name}: the ROM test's sum still matches");
        m.run_cycles(5 * CPU_HZ);
        assert_eq!(tone_writes(&mut m), 0, "{name}: silent when off");
        assert!(m.set_key_click(true));
        assert!(tone_writes(&mut m) > 0, "{name}: clicks when on");
    }
}

//! JBASIC 1.1 from the expansion window: the monitor's A command copies it
//! to 0100h and runs it (MD §3.2, §14.1). The image is not part of the
//! repository: put jbasic11.bin in the JUKU_TEST_DISKS directory, or the test
//! is skipped.

mod common;

use common::*;
use juku::Machine;

#[test]
fn jbasic_starts_with_a_and_runs_a_line() {
    let Some(image) = test_file(JBASIC) else { return };
    for name in ROMS {
        let rom = load_rom_named(name);
        let mut m = Machine::new(&rom).unwrap();
        m.insert_expansion(&image).expect("a cartridge image");
        run_to_prompt(&mut m, &rom, 10.0);
        type_text(&mut m, "a");
        let text = run_until(&mut m, &rom, 10.0, |l| l.starts_with("READY"));
        println!("{name}:\n{}", text.join("\n"));

        type_text(&mut m, "print 6*7\r");
        let text = run_until(&mut m, &rom, 5.0, |l| l.trim() == "42");
        println!("{}", text.join("\n"));
    }
}

#[test]
fn the_monitor_restarts_without_a_cartridge() {
    let rom = load_rom();
    let mut m = Machine::new(&rom).unwrap();
    run_to_prompt(&mut m, &rom, 10.0);
    type_text(&mut m, "a");
    // The window reads FFh: RST 7 lands back in the monitor.
    run_to_prompt(&mut m, &rom, 10.0);
}

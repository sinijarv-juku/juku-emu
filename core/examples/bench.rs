//! Emulation speed for a few workloads, as multiples of real time: the idle
//! `*` prompt, the monitor scrolling a memory dump, and the raster demos in
//! demos/ (a write to the timers per frame; the CPU shifting video memory).
//! Two run again under the debugger (spec §12.4), with a breakpoint and
//! watchpoints set that nothing reaches. Each runs three times and the best
//! is shown.
//! Usage: cargo run --profile test --example bench
#[path = "../tests/common/mod.rs"]
mod common;

use common::*;
use juku::Machine;
use juku::debug::Access;
use juku::machine::CPU_HZ;
use std::time::Instant;

const DEMOS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../demos");

/// A workload: its name, how long it runs, and how to set it up.
type Case<'a> = (&'a str, u64, Box<dyn Fn() -> Machine + 'a>);

/// Seconds of host time for `secs` of emulated time.
fn time(m: &mut Machine, secs: u64) -> f64 {
    let t = Instant::now();
    m.run_cycles(CPU_HZ * secs);
    t.elapsed().as_secs_f64()
}

fn at_prompt(rom: &[u8], expansion: Option<&[u8]>) -> Machine {
    let mut m = Machine::new(rom).unwrap();
    if let Some(image) = expansion {
        m.insert_expansion(image).expect("a demo image");
    }
    run_to_prompt(&mut m, rom, 10.0);
    m
}

/// `m` under the debugger, with a breakpoint and watchpoints of each kind.
fn debugging(mut m: Machine) -> Machine {
    m.set_debugging(true);
    let d = m.debugger_mut().unwrap();
    d.set_breakpoint(0x7000, true);
    for access in [Access::Read, Access::Write, Access::In, Access::Out] {
        d.set_watch(access, if matches!(access, Access::In | Access::Out) { 0x70 } else { 0x7000 }, true);
    }
    m
}

fn main() {
    let rom = load_rom();
    let bounce = std::fs::read(format!("{DEMOS}/bounce.bin")).unwrap();
    let scroller = std::fs::read(format!("{DEMOS}/scroller.bin")).unwrap();
    let cases: [Case; 6] = [
        ("idle prompt", 60, Box::new(|| at_prompt(&rom, None))),
        (
            "monitor dump, scrolling",
            30,
            Box::new(|| {
                let mut m = at_prompt(&rom, None);
                type_text(&mut m, "d0000ffff");
                m
            }),
        ),
        (
            "bounce.bin",
            30,
            Box::new(|| {
                let mut m = at_prompt(&rom, Some(&bounce));
                type_text(&mut m, "a");
                m
            }),
        ),
        (
            "scroller.bin",
            30,
            Box::new(|| {
                let mut m = at_prompt(&rom, Some(&scroller));
                type_text(&mut m, "a");
                m
            }),
        ),
        ("idle prompt, debugging", 60, Box::new(|| debugging(at_prompt(&rom, None)))),
        (
            "scroller.bin, debugging",
            30,
            Box::new(|| {
                let mut m = at_prompt(&rom, Some(&scroller));
                type_text(&mut m, "a");
                debugging(m)
            }),
        ),
    ];
    for (name, secs, setup) in &cases {
        let best = (0..3).map(|_| time(&mut setup(), *secs)).fold(f64::MAX, f64::min);
        println!("{name:26} {:5.0}× real time", *secs as f64 / best);
    }
}

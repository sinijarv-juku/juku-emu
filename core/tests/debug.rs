//! The debugger in the machine (spec §12.4): running under it changes
//! nothing, a stop holds the machine still, and steps go over the frame
//! interrupt.

mod common;

use common::*;
use juku::Machine;
use juku::cpu::disasm::Instr;
use juku::debug::{Access, Resume, Stop};
use juku::machine::CPU_HZ;

fn debugged(rom: &[u8]) -> Machine {
    let mut m = Machine::new(rom).unwrap();
    m.set_debugging(true);
    run_to_prompt(&mut m, rom, 10.0);
    m
}

/// Runs until the debugger stops the machine, for at most `secs`.
fn run_to_stop(m: &mut Machine, secs: u64) -> Option<Stop> {
    for _ in 0..secs * 10 {
        m.run_cycles(CPU_HZ / 10);
        if let Some(why) = m.debugger().unwrap().stopped() {
            return Some(why);
        }
    }
    None
}

fn interrupt_frames(m: &Machine) -> usize {
    m.debugger().unwrap().stack().iter().filter(|f| f.interrupt).count()
}

#[test]
fn debugging_changes_nothing() {
    let rom = load_rom();
    let mut plain = Machine::new(&rom).unwrap();
    let mut debugged = Machine::new(&rom).unwrap();
    debugged.set_debugging(true);
    // A breakpoint and watchpoints that nothing reaches.
    let d = debugged.debugger_mut().unwrap();
    d.set_breakpoint(0x7000, true);
    d.set_watch(Access::Read, 0x7000, true);
    d.set_watch(Access::Write, 0x7000, true);
    d.set_watch(Access::Out, 0x70, true);
    for m in [&mut plain, &mut debugged] {
        run_to_prompt(m, &rom, 10.0);
        // Looking at the devices brings the timers up to date, which
        // changes nothing.
        if m.debugger().is_some() {
            let d = m.devices();
            assert_eq!(d.pic[2] & 0x20, 0, "the frame interrupt is unmasked");
            assert!(d.pits[0][1].count.is_some(), "the line timer has a count");
        }
        type_text(m, "d0000001f");
        m.run_cycles(CPU_HZ);
    }
    assert_eq!(debugged.debugger().unwrap().stopped(), None);
    assert_eq!(plain.cycles(), debugged.cycles());
    assert!(plain.save_snapshot(None, &[]) == debugged.save_snapshot(None, &[]));
}

/// The same with the FDC's CPU waits: EKDOS booted and DIR run, with the
/// disk read through the debugger's bus (`JUKU_TEST_DISKS`).
#[test]
fn debugging_changes_nothing_on_disk() {
    let Some(image) = test_file(PROGRAMS_DISK) else { return };
    let rom = load_rom();
    let mut plain = Machine::new(&rom).unwrap();
    let mut debugged = Machine::new(&rom).unwrap();
    debugged.set_debugging(true);
    for m in [&mut plain, &mut debugged] {
        m.insert_disk(0, image.clone(), true).unwrap();
        boot_ekdos(m, &rom);
        command(m, &rom, "dir");
    }
    assert_eq!(plain.cycles(), debugged.cycles());
    assert!(plain.save_snapshot(None, &[]) == debugged.save_snapshot(None, &[]));
}

#[test]
fn a_stop_holds_the_machine() {
    let rom = load_rom();
    let mut m = debugged(&rom);
    m.debugger_mut().unwrap().pause();
    let (cycles, seq) = (m.cycles(), m.frame().seq);
    assert_eq!(m.run_cycles(CPU_HZ), 0);
    assert_eq!((m.cycles(), m.frame().seq), (cycles, seq));

    // The time asked for while stopped is not made up afterwards.
    m.resume(Resume::Continue);
    assert!(m.run_cycles(CPU_HZ) - CPU_HZ < 20);

    // Ending debugging lets a stopped machine run.
    m.debugger_mut().unwrap().pause();
    m.set_debugging(false);
    assert!(m.run_cycles(CPU_HZ) >= CPU_HZ);
}

#[test]
fn breakpoint_in_the_rom() {
    let rom = load_rom();
    let mut m = debugged(&rom);
    // CONSTAT, which the monitor polls at the prompt (MD §14.2).
    m.debugger_mut().unwrap().set_breakpoint(0xFF98, true);
    assert_eq!(run_to_stop(&mut m, 1), Some(Stop::Breakpoint));
    assert_eq!(m.cpu().pc, 0xFF98);
}

/// At the prompt, steps from CALL to CALL and over each one, until three
/// of the calls had a frame begin while they ran. Each step over ends just
/// after its CALL with SP as it was, and an instruction step never ends in
/// an interrupt handler.
#[test]
fn steps_go_over_the_frame_interrupt() {
    let rom = load_rom();
    let mut m = debugged(&rom);
    m.debugger_mut().unwrap().pause();
    let mut with_frame = 0;
    for _ in 0..100_000 {
        let i = Instr::at(m.cpu().pc, |a| m.peek(a));
        if i.op != 0xCD {
            let depth = interrupt_frames(&m);
            m.resume(Resume::Instruction);
            assert_eq!(run_to_stop(&mut m, 1), Some(Stop::Step));
            assert!(interrupt_frames(&m) <= depth, "stopped in a handler at {:04X}", m.cpu().pc);
            continue;
        }
        let (pc, sp, frame) = (m.cpu().pc, m.cpu().sp, m.frame_cycle());
        m.resume(Resume::Over);
        match run_to_stop(&mut m, 1) {
            Some(why) => {
                assert_eq!((why, m.cpu().pc, m.cpu().sp), (Stop::Step, pc + 3, sp), "CALL at {pc:04X}");
                if m.frame_cycle() != frame {
                    with_frame += 1;
                    if with_frame == 3 {
                        return;
                    }
                }
            }
            // The call that waits for a key does not return.
            None => m.debugger_mut().unwrap().pause(),
        }
    }
    panic!("only {with_frame} calls had a frame begin in them");
}

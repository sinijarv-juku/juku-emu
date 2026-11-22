//! Guest debugger hooks (spec §12.4): breakpoints, watchpoints, steps and
//! the shadow call stack.
//!
//! While a client is connected the machine runs a loop of its own, which
//! steps the CPU through [`DebugBus`], a wrapper that passes every bus call
//! on and looks at it as it goes by. With no client the machine's ordinary
//! loop runs, and nothing here is touched.

use core::ops::Range;

use crate::cpu::disasm::{self, Instr};
use crate::cpu::i8080::{Bus, Cpu};

/// Frames the shadow stack keeps; a deeper call drops the oldest.
const MAX_FRAMES: usize = 64;

/// A kind of access a watchpoint can watch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
    In,
    Out,
}

/// The access that hit a watchpoint. For a port, `addr` is the port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WatchHit {
    pub access: Access,
    pub addr: u16,
    pub value: u8,
}

/// Why the machine stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// Asked for with [`Debug::pause`].
    Pause,
    /// PC reached a breakpoint.
    Breakpoint,
    /// An instruction made an access a watchpoint watches.
    Watch(WatchHit),
    /// A step ended.
    Step,
}

/// A frame of the shadow call stack: a CALL, RST or interrupt not yet
/// returned from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The address it returns to.
    pub ret: u16,
    /// SP after the return address was pushed.
    pub sp: u16,
    /// The address called.
    pub target: u16,
    pub interrupt: bool,
}

/// How to go on from a stop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resume {
    Continue,
    /// One instruction. HLT ends when the CPU leaves the HALT state.
    Instruction,
    /// One instruction, a call running until it returns.
    Over,
    /// Until the innermost frame of the shadow stack returns.
    Out,
    /// While PC is in one of the ranges, over calls made from them unless
    /// `into` (a source line, spec §12.5).
    Range { ranges: Vec<Range<u16>>, into: bool },
}

/// The step in progress.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    Run,
    /// `executed` once the instruction has run.
    Instruction { executed: bool },
    /// Until PC and SP are these again.
    Over { pc: u16, sp: u16 },
    /// `over` holds the PC and SP that end a call being stepped over.
    Range { ranges: Vec<Range<u16>>, into: bool, over: Option<(u16, u16)> },
    /// Until the shadow stack is shallower than `depth`; with no frames, until
    /// a return lifts SP above `sp`.
    Out { depth: usize, sp: u16 },
}

/// A set of `N` × 64 bits.
#[derive(Clone)]
struct Bits<const N: usize>([u64; N]);

impl<const N: usize> Bits<N> {
    #[inline]
    fn get(&self, i: u16) -> bool {
        self.0[i as usize >> 6] >> (i & 63) & 1 != 0
    }

    fn set(&mut self, i: u16, on: bool) {
        let (word, bit) = (&mut self.0[i as usize >> 6], 1 << (i & 63));
        *word = if on { *word | bit } else { *word & !bit };
    }

    fn any(&self) -> bool {
        self.0.iter().any(|&w| w != 0)
    }
}

/// The debugger's state in the machine.
pub struct Debug {
    breakpoints: Box<Bits<1024>>,
    /// Where instructions are known to start: each address PC has held
    /// while debugging, until written to. Backwards disassembly is
    /// ambiguous; these keep it on the instructions the CPU runs.
    code: Box<Bits<1024>>,
    reads: Box<Bits<1024>>,
    writes: Box<Bits<1024>>,
    ins: Bits<4>,
    outs: Bits<4>,
    /// Any watchpoint set.
    watching: bool,
    step: Step,
    stop: Option<Stop>,
    stack: Vec<Frame>,
    /// While a step runs through an interrupt handler: the depth of the
    /// shadow stack to come back to.
    hidden: Option<usize>,
    step_into_interrupts: bool,
    /// The step a breakpoint or watchpoint stop broke into, and its
    /// interrupt depth, for [`Debug::go_on`].
    interrupted: Option<(Step, Option<usize>)>,
}

impl Default for Debug {
    fn default() -> Debug {
        Debug {
            breakpoints: Box::new(Bits([0; 1024])),
            code: Box::new(Bits([0; 1024])),
            reads: Box::new(Bits([0; 1024])),
            writes: Box::new(Bits([0; 1024])),
            ins: Bits([0; 4]),
            outs: Bits([0; 4]),
            watching: false,
            step: Step::Run,
            stop: None,
            stack: Vec::with_capacity(MAX_FRAMES),
            hidden: None,
            step_into_interrupts: false,
            interrupted: None,
        }
    }
}

fn is_call(op: u8) -> bool {
    // CALL, its undocumented twins, Ccc, RST
    matches!(op, 0xCD | 0xDD | 0xED | 0xFD) || op >= 0xC0 && matches!(op & 7, 4 | 7)
}

fn is_ret(op: u8) -> bool {
    // RET, RET*, Rcc
    matches!(op, 0xC9 | 0xD9) || op >= 0xC0 && op & 7 == 0
}

fn contains(ranges: &[Range<u16>], pc: u16) -> bool {
    ranges.iter().any(|r| r.contains(&pc))
}

impl Debug {
    pub fn new() -> Debug {
        Debug::default()
    }

    pub fn breakpoint(&self, addr: u16) -> bool {
        self.breakpoints.get(addr)
    }

    pub fn set_breakpoint(&mut self, addr: u16, on: bool) {
        self.breakpoints.set(addr, on);
    }

    pub fn clear_breakpoints(&mut self) {
        self.breakpoints.0.fill(0);
    }

    /// Watches `access` to `addr`, a port for `In` and `Out`, which takes
    /// the low byte.
    pub fn set_watch(&mut self, access: Access, addr: u16, on: bool) {
        match access {
            Access::Read => self.reads.set(addr, on),
            Access::Write => self.writes.set(addr, on),
            Access::In => self.ins.set(addr & 0xFF, on),
            Access::Out => self.outs.set(addr & 0xFF, on),
        }
        self.watching = self.reads.any() || self.writes.any() || self.ins.any() || self.outs.any();
    }

    pub fn clear_watches(&mut self) {
        self.reads.0.fill(0);
        self.writes.0.fill(0);
        self.ins.0.fill(0);
        self.outs.0.fill(0);
        self.watching = false;
    }

    /// Steps stop at an interrupt handler's first instruction, instead of
    /// running through the handler.
    pub fn set_step_into_interrupts(&mut self, on: bool) {
        self.step_into_interrupts = on;
    }

    /// Why the machine is stopped, or `None` while it runs.
    pub fn stopped(&self) -> Option<Stop> {
        self.stop
    }

    /// Stops the machine before its next instruction.
    pub fn pause(&mut self) {
        if self.stop.is_none() {
            self.halt(Stop::Pause);
        }
    }

    /// The shadow call stack, outermost frame first.
    pub fn stack(&self) -> &[Frame] {
        &self.stack
    }

    /// Forgets the shadow stack and where code starts, for a reset or a
    /// snapshot.
    pub fn forget(&mut self) {
        self.stack.clear();
        self.hidden = None;
        self.code.0.fill(0);
    }

    /// An instruction is known to start at `addr`.
    pub fn code_start(&self, addr: u16) -> bool {
        self.code.get(addr)
    }

    /// Notes that an instruction starts at `addr`.
    pub fn note_code(&mut self, addr: u16) {
        self.code.set(addr, true);
    }

    /// Forgets an instruction known to start at `addr`, which has been
    /// written to.
    pub fn forget_code(&mut self, addr: u16) {
        self.code.set(addr, false);
    }

    /// Lets the machine run on from a stop, as `how` says. `read` reads
    /// memory as the CPU sees it.
    pub fn resume(&mut self, how: Resume, cpu: &Cpu, read: impl FnMut(u16) -> u8) {
        self.stop = None;
        self.interrupted = None;
        self.hidden = None;
        self.step = match how {
            Resume::Continue => Step::Run,
            Resume::Instruction => Step::Instruction { executed: false },
            Resume::Over => {
                let i = Instr::at(cpu.pc, read);
                if !cpu.halted && is_call(i.op) {
                    Step::Over { pc: cpu.pc.wrapping_add(i.size() as u16), sp: cpu.sp }
                } else {
                    Step::Instruction { executed: false }
                }
            }
            Resume::Out => Step::Out { depth: self.stack.len(), sp: cpu.sp },
            Resume::Range { ranges, into } => Step::Range { ranges, into, over: None },
        };
    }

    fn halt(&mut self, why: Stop) {
        let step = std::mem::replace(&mut self.step, Step::Run);
        let hidden = self.hidden.take();
        self.interrupted = matches!(why, Stop::Breakpoint | Stop::Watch(_)).then_some((step, hidden));
        self.stop = Some(why);
    }

    /// Goes on from a breakpoint or watchpoint stop as if it had not been:
    /// with the step it broke into, if any. For a condition that does not
    /// hold, or a log point (spec §12.5).
    pub fn go_on(&mut self) {
        if let Some((step, hidden)) = self.interrupted.take() {
            self.step = step;
            self.hidden = hidden;
        }
        self.stop = None;
    }

    /// Executes one instruction (or interrupt acknowledge, or HALT cycle) and
    /// stops the machine if that ends a step or hits a breakpoint or a
    /// watchpoint. Returns the cycles taken.
    pub(crate) fn step<B: Bus>(&mut self, cpu: &mut Cpu, bus: &mut B) -> u32 {
        let (pc, sp) = (cpu.pc, cpu.sp);
        let mut db = DebugBus { bus, debug: self, fetches: 1, op: None, int: false, hit: None };
        let cycles = cpu.step(&mut db);
        let DebugBus { op, int, hit, .. } = db;
        self.code.set(cpu.pc, true);

        // The shadow stack. A frame has been left when the word holding its
        // return address is taken off the stack: by a RET, or a POP or INX SP
        // past it. Loading SP (LXI SP, SPHL) switches stacks and leaves the
        // frames as they are: RomBios moves to a stack of its own and back.
        let (began, op) = (op.is_some(), op.unwrap_or(0));
        if began && cpu.sp > sp && !matches!(op, 0x31 | 0xF9) {
            // Frames left below the interrupt a step runs through move the
            // depth it waits for.
            let (taken, hidden) = (sp..cpu.sp, self.hidden.unwrap_or(0));
            let (mut i, mut below) = (0, 0);
            self.stack.retain(|f| {
                let kept = !taken.contains(&f.sp);
                below += (!kept && i < hidden) as usize;
                i += 1;
                kept
            });
            if let Some(d) = &mut self.hidden {
                *d -= below;
            }
        }
        let called = began && cpu.sp == sp.wrapping_sub(2) && (int || is_call(op));
        if called {
            if self.stack.len() == MAX_FRAMES {
                self.stack.remove(0);
                self.hidden = self.hidden.map(|d| d.saturating_sub(1));
            }
            let ret = if int { pc } else { pc.wrapping_add(disasm::len(op) as u16) };
            self.stack.push(Frame { ret, sp: cpu.sp, target: cpu.pc, interrupt: int });
        }

        // An interrupt taken during a step is stepped over: the step goes on
        // once the handler has returned.
        let hidden_before = self.hidden.is_some();
        let into = self.step_into_interrupts;
        if int && called && !into && !hidden_before && !matches!(self.step, Step::Run) {
            self.hidden = Some(self.stack.len() - 1);
        }
        if self.hidden.is_some_and(|d| self.stack.len() <= d) {
            self.hidden = None;
        }
        // An instruction of the code being stepped ran.
        let counted = began && !hidden_before && !(int && !into);

        let done = self.hidden.is_none()
            && match &mut self.step {
                Step::Run => false,
                Step::Instruction { executed } => {
                    *executed |= counted;
                    *executed && !cpu.halted
                }
                Step::Over { pc, sp } => cpu.pc == *pc && cpu.sp == *sp,
                Step::Range { ranges, into, over } => {
                    if counted && called && !*into && contains(ranges, pc) {
                        *over = Some((pc.wrapping_add(disasm::len(op) as u16), sp));
                    }
                    match *over {
                        Some(end) if end != (cpu.pc, cpu.sp) => false,
                        _ => {
                            *over = None;
                            !contains(ranges, cpu.pc) && !cpu.halted
                        }
                    }
                }
                Step::Out { depth, sp: start } => {
                    if *depth > 0 {
                        self.stack.len() < *depth
                    } else {
                        counted && is_ret(op) && cpu.sp > *start
                    }
                }
            };

        if let Some(hit) = hit {
            self.halt(Stop::Watch(hit));
        } else if done {
            self.halt(Stop::Step);
        } else if !cpu.halted && self.breakpoints.get(cpu.pc) {
            self.halt(Stop::Breakpoint);
        }
        cycles
    }
}

/// The bus as the CPU sees it while debugging: every call goes on to `bus`,
/// and the debugger notes what it needs.
struct DebugBus<'a, B> {
    bus: &'a mut B,
    debug: &'a mut Debug,
    /// Reads still to come that fetch the instruction's own bytes.
    fetches: u8,
    /// The opcode executed, if an instruction was.
    op: Option<u8>,
    /// The instruction came from an interrupt acknowledge.
    int: bool,
    hit: Option<WatchHit>,
}

impl<B> DebugBus<'_, B> {
    #[inline]
    fn watch(&mut self, access: Access, addr: u16, value: u8) {
        if self.hit.is_none() {
            self.hit = Some(WatchHit { access, addr, value });
        }
    }
}

impl<B: Bus> Bus for DebugBus<'_, B> {
    #[inline]
    fn read(&mut self, addr: u16) -> u8 {
        let v = self.bus.read(addr);
        if self.fetches > 0 {
            self.fetches -= 1;
        } else if self.debug.watching && self.debug.reads.get(addr) {
            self.watch(Access::Read, addr, v);
        }
        v
    }

    #[inline]
    fn write(&mut self, addr: u16, val: u8) {
        if self.debug.watching && self.debug.writes.get(addr) {
            self.watch(Access::Write, addr, val);
        }
        // Code written over may start elsewhere now. A write under the ROM
        // leaves the ROM's code as it is.
        if self.debug.code.get(addr) && self.bus.reads_back(addr) {
            self.debug.code.set(addr, false);
        }
        self.bus.write(addr, val);
    }

    fn reads_back(&self, addr: u16) -> bool {
        self.bus.reads_back(addr)
    }

    fn input(&mut self, port: u8) -> u8 {
        let v = self.bus.input(port);
        if self.debug.watching && self.debug.ins.get(port as u16) {
            self.watch(Access::In, port as u16, v);
        }
        v
    }

    fn output(&mut self, port: u8, val: u8) {
        if self.debug.watching && self.debug.outs.get(port as u16) {
            self.watch(Access::Out, port as u16, val);
        }
        self.bus.output(port, val);
    }

    #[inline]
    fn begin(&mut self, op: u8) {
        // The opcode has been fetched; the operands come next, from memory
        // unless an interrupt acknowledge supplied them.
        self.op = Some(op);
        self.fetches = if self.int { 0 } else { disasm::len(op) - 1 };
        self.bus.begin(op);
    }

    fn int_pending(&self) -> bool {
        self.bus.int_pending()
    }

    fn int_ack(&mut self) -> [u8; 3] {
        self.int = true;
        self.fetches = 0;
        self.bus.int_ack()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 64 KB of RAM, an interrupt line, and an 8259 that supplies CALL 0040h.
    struct TestBus {
        mem: Vec<u8>,
        irq: bool,
        out: Vec<(u8, u8)>,
    }

    impl Bus for TestBus {
        fn read(&mut self, addr: u16) -> u8 {
            self.mem[addr as usize]
        }
        fn write(&mut self, addr: u16, val: u8) {
            self.mem[addr as usize] = val;
        }
        fn input(&mut self, port: u8) -> u8 {
            port
        }
        fn output(&mut self, port: u8, val: u8) {
            self.out.push((port, val));
        }
        fn int_pending(&self) -> bool {
            self.irq
        }
        fn int_ack(&mut self) -> [u8; 3] {
            self.irq = false;
            [0xCD, 0x40, 0x00]
        }
    }

    #[rustfmt::skip]
    const PROGRAM: &[(u16, &[u8])] = &[
        (0x0000, &[0x31, 0x00, 0x10]), // LXI SP,1000
        (0x0003, &[0xFB]),             // EI
        (0x0004, &[0xCD, 0x10, 0x00]), // CALL 0010
        (0x0007, &[0x00]),             // NOP
        (0x0008, &[0x76]),             // HLT
        (0x0009, &[0x00]),             // NOP
        (0x0010, &[0xCD, 0x20, 0x00]), // CALL 0020
        (0x0013, &[0xC9]),             // RET
        (0x0020, &[0x3E, 0x05]),       // MVI A,5
        (0x0022, &[0x32, 0x00, 0x01]), // STA 0100
        (0x0025, &[0x3A, 0x00, 0x01]), // LDA 0100
        (0x0028, &[0xD3, 0x07]),       // OUT 07
        (0x002A, &[0xC9]),             // RET
        (0x0030, &[0xE1]),             // POP H
        (0x0031, &[0xE9]),             // PCHL
        (0x0040, &[0x04, 0xFB, 0xC9]), // interrupt: INR B / EI / RET
    ];

    fn setup() -> (Debug, Cpu, TestBus) {
        let mut mem = vec![0; 0x10000];
        for &(at, bytes) in PROGRAM {
            mem[at as usize..at as usize + bytes.len()].copy_from_slice(bytes);
        }
        (Debug::new(), Cpu::new(), TestBus { mem, irq: false, out: Vec::new() })
    }

    /// Steps until the machine stops, at most `limit` times. Returns whether
    /// it stopped.
    fn run(d: &mut Debug, cpu: &mut Cpu, bus: &mut TestBus, limit: usize) -> bool {
        for _ in 0..limit {
            d.step(cpu, bus);
            if d.stopped().is_some() {
                return true;
            }
        }
        false
    }

    /// Runs on to a breakpoint at `addr`, which is then cleared.
    fn run_to(d: &mut Debug, cpu: &mut Cpu, bus: &mut TestBus, addr: u16) {
        resume(d, cpu, bus, Resume::Continue);
        d.set_breakpoint(addr, true);
        assert!(run(d, cpu, bus, 100));
        assert_eq!((d.stopped(), cpu.pc), (Some(Stop::Breakpoint), addr));
        d.set_breakpoint(addr, false);
    }

    fn resume(d: &mut Debug, cpu: &Cpu, bus: &TestBus, how: Resume) {
        d.resume(how, cpu, |a| bus.mem[a as usize]);
    }

    #[test]
    fn breakpoint_stops_before_the_instruction() {
        let (mut d, mut cpu, mut bus) = setup();
        run_to(&mut d, &mut cpu, &mut bus, 0x0022);
        assert_eq!(bus.mem[0x0100], 0);
        // Going on runs the instruction at the breakpoint, without stopping
        // there again.
        d.set_breakpoint(0x0022, true);
        d.set_breakpoint(0x0025, true);
        resume(&mut d, &cpu, &bus, Resume::Continue);
        assert!(run(&mut d, &mut cpu, &mut bus, 10));
        assert_eq!((cpu.pc, bus.mem[0x0100]), (0x0025, 5));
    }

    #[test]
    fn pause_stops_at_once() {
        let (mut d, mut cpu, mut bus) = setup();
        assert!(!run(&mut d, &mut cpu, &mut bus, 3));
        d.pause();
        assert_eq!(d.stopped(), Some(Stop::Pause));
        resume(&mut d, &cpu, &bus, Resume::Continue);
        assert_eq!(d.stopped(), None);
    }

    #[test]
    fn shadow_stack_follows_calls_and_returns() {
        let (mut d, mut cpu, mut bus) = setup();
        run_to(&mut d, &mut cpu, &mut bus, 0x0020);
        let outer = Frame { ret: 0x0007, sp: 0x0FFE, target: 0x0010, interrupt: false };
        let inner = Frame { ret: 0x0013, sp: 0x0FFC, target: 0x0020, interrupt: false };
        assert_eq!(d.stack(), [outer, inner]);
        run_to(&mut d, &mut cpu, &mut bus, 0x0013);
        assert_eq!(d.stack(), [outer]);
        run_to(&mut d, &mut cpu, &mut bus, 0x0007);
        assert_eq!(d.stack(), []);
    }

    /// RomBios moves to a stack of its own for a call and back: the frames
    /// on the first stack stay.
    #[test]
    fn frames_stay_across_a_stack_switch() {
        let (mut d, mut cpu, mut bus) = setup();
        // At 0020h: LXI SP,2000 / CALL 0030 ... where 0030h returns at once.
        bus.mem[0x0020..0x0026].copy_from_slice(&[0x31, 0x00, 0x20, 0xCD, 0x30, 0x00]);
        bus.mem[0x0030] = 0xC9;
        run_to(&mut d, &mut cpu, &mut bus, 0x0020);
        let outer = d.stack().to_vec();
        assert_eq!(outer.len(), 2);
        resume(&mut d, &cpu, &bus, Resume::Instruction);
        assert!(run(&mut d, &mut cpu, &mut bus, 1)); // LXI SP, up from 0FFCh
        assert_eq!(d.stack(), outer);
        run_to(&mut d, &mut cpu, &mut bus, 0x0030);
        assert_eq!(d.stack().len(), 3);
        // Its RET takes the word at 1FFEh, not the frames at 0FFCh-0FFEh.
        run_to(&mut d, &mut cpu, &mut bus, 0x0026);
        assert_eq!(d.stack(), outer);
    }

    #[test]
    fn a_frame_left_without_a_return_is_dropped() {
        let (mut d, mut cpu, mut bus) = setup();
        bus.mem[0x0010..0x0013].copy_from_slice(&[0xCD, 0x30, 0x00]); // CALL 0030
        run_to(&mut d, &mut cpu, &mut bus, 0x0030);
        assert_eq!(d.stack().len(), 2);
        resume(&mut d, &cpu, &bus, Resume::Instruction);
        assert!(run(&mut d, &mut cpu, &mut bus, 1)); // POP H
        assert_eq!(d.stack().len(), 1);
    }

    #[test]
    fn instruction_step() {
        let (mut d, mut cpu, mut bus) = setup();
        run_to(&mut d, &mut cpu, &mut bus, 0x0010);
        resume(&mut d, &cpu, &bus, Resume::Instruction);
        assert!(run(&mut d, &mut cpu, &mut bus, 1));
        assert_eq!((d.stopped(), cpu.pc), (Some(Stop::Step), 0x0020));
    }

    #[test]
    fn step_over_a_call() {
        let (mut d, mut cpu, mut bus) = setup();
        run_to(&mut d, &mut cpu, &mut bus, 0x0010);
        resume(&mut d, &cpu, &bus, Resume::Over);
        assert!(run(&mut d, &mut cpu, &mut bus, 100));
        assert_eq!((d.stopped(), cpu.pc, d.stack().len()), (Some(Stop::Step), 0x0013, 1));
        // Anything else is one instruction.
        resume(&mut d, &cpu, &bus, Resume::Over);
        assert!(run(&mut d, &mut cpu, &mut bus, 1));
        assert_eq!(cpu.pc, 0x0007);
    }

    #[test]
    fn breakpoint_inside_a_call_stops_a_step_over() {
        let (mut d, mut cpu, mut bus) = setup();
        run_to(&mut d, &mut cpu, &mut bus, 0x0010);
        d.set_breakpoint(0x0025, true);
        resume(&mut d, &cpu, &bus, Resume::Over);
        assert!(run(&mut d, &mut cpu, &mut bus, 100));
        assert_eq!((d.stopped(), cpu.pc), (Some(Stop::Breakpoint), 0x0025));
    }

    #[test]
    fn step_out_of_nested_calls() {
        let (mut d, mut cpu, mut bus) = setup();
        run_to(&mut d, &mut cpu, &mut bus, 0x0022);
        resume(&mut d, &cpu, &bus, Resume::Out);
        assert!(run(&mut d, &mut cpu, &mut bus, 100));
        assert_eq!((d.stopped(), cpu.pc, d.stack().len()), (Some(Stop::Step), 0x0013, 1));
        resume(&mut d, &cpu, &bus, Resume::Out);
        assert!(run(&mut d, &mut cpu, &mut bus, 100));
        assert_eq!((cpu.pc, d.stack().len()), (0x0007, 0));
    }

    #[test]
    fn step_out_with_no_frames_waits_for_a_return() {
        let (mut d, mut cpu, mut bus) = setup();
        run_to(&mut d, &mut cpu, &mut bus, 0x0022);
        d.forget();
        resume(&mut d, &cpu, &bus, Resume::Out);
        assert!(run(&mut d, &mut cpu, &mut bus, 100));
        assert_eq!(cpu.pc, 0x0013);
    }

    #[test]
    fn interrupt_in_a_step_is_stepped_over() {
        let (mut d, mut cpu, mut bus) = setup();
        run_to(&mut d, &mut cpu, &mut bus, 0x0010);
        bus.irq = true;
        resume(&mut d, &cpu, &bus, Resume::Instruction);
        assert!(run(&mut d, &mut cpu, &mut bus, 100));
        // The handler ran, then the CALL.
        assert_eq!((d.stopped(), cpu.pc, cpu.b), (Some(Stop::Step), 0x0020, 1));
        assert_eq!(d.stack().len(), 2);
    }

    #[test]
    fn interrupt_in_a_step_over() {
        let (mut d, mut cpu, mut bus) = setup();
        run_to(&mut d, &mut cpu, &mut bus, 0x0020);
        bus.irq = true;
        resume(&mut d, &cpu, &bus, Resume::Over);
        assert!(run(&mut d, &mut cpu, &mut bus, 100));
        assert_eq!((cpu.pc, cpu.b, cpu.a), (0x0022, 1, 5));
    }

    #[test]
    fn step_into_an_interrupt() {
        let (mut d, mut cpu, mut bus) = setup();
        d.set_step_into_interrupts(true);
        run_to(&mut d, &mut cpu, &mut bus, 0x0010);
        bus.irq = true;
        resume(&mut d, &cpu, &bus, Resume::Instruction);
        assert!(run(&mut d, &mut cpu, &mut bus, 1));
        assert_eq!(cpu.pc, 0x0040);
        let top = *d.stack().last().unwrap();
        assert_eq!(top, Frame { ret: 0x0010, sp: 0x0FFC, target: 0x0040, interrupt: true });
        // Out of the handler, back where it came in.
        resume(&mut d, &cpu, &bus, Resume::Out);
        assert!(run(&mut d, &mut cpu, &mut bus, 100));
        assert_eq!((cpu.pc, d.stack().len()), (0x0010, 1));
    }

    #[test]
    fn step_over_hlt_ends_after_the_interrupt() {
        let (mut d, mut cpu, mut bus) = setup();
        run_to(&mut d, &mut cpu, &mut bus, 0x0008);
        resume(&mut d, &cpu, &bus, Resume::Over);
        assert!(!run(&mut d, &mut cpu, &mut bus, 50));
        assert!(cpu.halted);
        bus.irq = true;
        assert!(run(&mut d, &mut cpu, &mut bus, 100));
        assert_eq!((d.stopped(), cpu.pc, cpu.b, cpu.halted), (Some(Stop::Step), 0x0009, 1, false));
    }

    #[test]
    fn no_breakpoint_while_halted() {
        let (mut d, mut cpu, mut bus) = setup();
        run_to(&mut d, &mut cpu, &mut bus, 0x0008);
        d.set_breakpoint(0x0009, true);
        resume(&mut d, &cpu, &bus, Resume::Continue);
        assert!(!run(&mut d, &mut cpu, &mut bus, 50));
        bus.irq = true;
        assert!(run(&mut d, &mut cpu, &mut bus, 100));
        assert_eq!((d.stopped(), cpu.pc, cpu.halted), (Some(Stop::Breakpoint), 0x0009, false));
    }

    #[test]
    fn range_step_over_calls() {
        let (mut d, mut cpu, mut bus) = setup();
        run_to(&mut d, &mut cpu, &mut bus, 0x0010);
        resume(&mut d, &cpu, &bus, Resume::Range { ranges: vec![Range { start: 0x0010, end: 0x0014 }], into: false });
        assert!(run(&mut d, &mut cpu, &mut bus, 100));
        assert_eq!((d.stopped(), cpu.pc, cpu.a), (Some(Stop::Step), 0x0007, 5));
    }

    #[test]
    fn range_step_into_a_call() {
        let (mut d, mut cpu, mut bus) = setup();
        run_to(&mut d, &mut cpu, &mut bus, 0x0010);
        resume(&mut d, &cpu, &bus, Resume::Range { ranges: vec![Range { start: 0x0010, end: 0x0013 }], into: true });
        assert!(run(&mut d, &mut cpu, &mut bus, 100));
        assert_eq!(cpu.pc, 0x0020);
    }

    #[test]
    fn code_starts() {
        let (mut d, mut cpu, mut bus) = setup();
        d.note_code(0x0100);
        run_to(&mut d, &mut cpu, &mut bus, 0x0022);
        assert!(d.code_start(0x0010) && d.code_start(0x0020) && d.code_start(0x0022));
        assert!(!d.code_start(0x0021));
        // STA 0100 writes over a start noted there.
        assert!(d.code_start(0x0100));
        run_to(&mut d, &mut cpu, &mut bus, 0x0025);
        assert!(!d.code_start(0x0100));
        d.forget();
        assert!(!d.code_start(0x0010));
    }

    /// A breakpoint whose condition does not hold goes on with the step it
    /// broke into.
    #[test]
    fn going_on_keeps_the_step() {
        let (mut d, mut cpu, mut bus) = setup();
        run_to(&mut d, &mut cpu, &mut bus, 0x0010);
        d.set_breakpoint(0x0022, true);
        resume(&mut d, &cpu, &bus, Resume::Over);
        assert!(run(&mut d, &mut cpu, &mut bus, 100));
        assert_eq!((d.stopped(), cpu.pc), (Some(Stop::Breakpoint), 0x0022));
        d.go_on();
        assert!(run(&mut d, &mut cpu, &mut bus, 100));
        assert_eq!((d.stopped(), cpu.pc), (Some(Stop::Step), 0x0013), "the step over ends after the call");
    }

    #[test]
    fn write_watchpoint() {
        let (mut d, mut cpu, mut bus) = setup();
        d.set_watch(Access::Write, 0x0100, true);
        assert!(run(&mut d, &mut cpu, &mut bus, 100));
        let hit = WatchHit { access: Access::Write, addr: 0x0100, value: 5 };
        assert_eq!((d.stopped(), cpu.pc), (Some(Stop::Watch(hit)), 0x0025));
    }

    #[test]
    fn read_watchpoint_skips_fetches() {
        let (mut d, mut cpu, mut bus) = setup();
        // STA's operand bytes are fetched, not read as data.
        d.set_watch(Access::Read, 0x0023, true);
        d.set_watch(Access::Read, 0x0024, true);
        d.set_watch(Access::Read, 0x0100, true);
        assert!(run(&mut d, &mut cpu, &mut bus, 100));
        let hit = WatchHit { access: Access::Read, addr: 0x0100, value: 5 };
        assert_eq!((d.stopped(), cpu.pc), (Some(Stop::Watch(hit)), 0x0028));
    }

    #[test]
    fn port_watchpoints() {
        let (mut d, mut cpu, mut bus) = setup();
        d.set_watch(Access::Out, 0x07, true);
        assert!(run(&mut d, &mut cpu, &mut bus, 100));
        let hit = WatchHit { access: Access::Out, addr: 0x07, value: 5 };
        assert_eq!((d.stopped(), cpu.pc, bus.out.as_slice()), (Some(Stop::Watch(hit)), 0x002A, &[(7, 5)][..]));
        d.clear_watches();
        resume(&mut d, &cpu, &bus, Resume::Continue);
        assert!(!run(&mut d, &mut cpu, &mut bus, 20));
    }

    #[test]
    fn interrupt_operands_are_not_data_reads() {
        let (mut d, mut cpu, mut bus) = setup();
        run_to(&mut d, &mut cpu, &mut bus, 0x0020);
        // The injected CALL's operands come from the 8259, not from memory
        // at PC; the handler's fetches are not data reads either.
        for a in 0x0020..0x0024 {
            d.set_watch(Access::Read, a, true);
        }
        d.set_watch(Access::Read, 0x0041, true);
        bus.irq = true;
        resume(&mut d, &cpu, &bus, Resume::Instruction);
        assert!(run(&mut d, &mut cpu, &mut bus, 100));
        assert_eq!((d.stopped(), cpu.pc), (Some(Stop::Step), 0x0022));
    }
}

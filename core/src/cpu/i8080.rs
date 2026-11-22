//! Intel 8080A (КР580ВМ80А) CPU core.
//!
//! Instruction-level emulation with exact cycle counts (T-states at the 2 MHz
//! CPU clock). All 256 opcodes are implemented, including the undocumented
//! aliases. Flags follow the 8080, not the Z80.

use crate::snapshot::{Reader, Writer};

/// Memory and I/O as seen by the CPU.
pub trait Bus {
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);
    fn input(&mut self, port: u8) -> u8;
    fn output(&mut self, port: u8, val: u8);

    /// An instruction starts executing: its opcode, or the one an interrupt
    /// acknowledge supplied. Lets the bus time the accesses that follow.
    #[inline]
    fn begin(&mut self, _op: u8) {}

    /// A write to `addr` is what a read of it returns afterwards: no ROM
    /// or window lies over it. For the debugger (`debug::DebugBus`).
    fn reads_back(&self, _addr: u16) -> bool {
        true
    }

    /// State of the INT input.
    fn int_pending(&self) -> bool {
        false
    }

    /// Interrupt acknowledge: the instruction placed on the data bus during
    /// the INTA cycles. The 8259 supplies a 3-byte CALL; a single-byte RST
    /// leaves the other bytes unused.
    fn int_ack(&mut self) -> [u8; 3] {
        [0xFF, 0, 0] // RST 7
    }
}

/// T-states per opcode. Conditional CALL and RET add `EXTRA_TAKEN` when taken.
#[rustfmt::skip]
const CYCLES: [u8; 256] = [
//  0   1   2   3   4   5   6   7   8   9   A   B   C   D   E   F
    4, 10,  7,  5,  5,  5,  7,  4,  4, 10,  7,  5,  5,  5,  7,  4, // 0
    4, 10,  7,  5,  5,  5,  7,  4,  4, 10,  7,  5,  5,  5,  7,  4, // 1
    4, 10, 16,  5,  5,  5,  7,  4,  4, 10, 16,  5,  5,  5,  7,  4, // 2
    4, 10, 13,  5, 10, 10, 10,  4,  4, 10, 13,  5,  5,  5,  7,  4, // 3
    5,  5,  5,  5,  5,  5,  7,  5,  5,  5,  5,  5,  5,  5,  7,  5, // 4
    5,  5,  5,  5,  5,  5,  7,  5,  5,  5,  5,  5,  5,  5,  7,  5, // 5
    5,  5,  5,  5,  5,  5,  7,  5,  5,  5,  5,  5,  5,  5,  7,  5, // 6
    7,  7,  7,  7,  7,  7,  7,  7,  5,  5,  5,  5,  5,  5,  7,  5, // 7
    4,  4,  4,  4,  4,  4,  7,  4,  4,  4,  4,  4,  4,  4,  7,  4, // 8
    4,  4,  4,  4,  4,  4,  7,  4,  4,  4,  4,  4,  4,  4,  7,  4, // 9
    4,  4,  4,  4,  4,  4,  7,  4,  4,  4,  4,  4,  4,  4,  7,  4, // A
    4,  4,  4,  4,  4,  4,  7,  4,  4,  4,  4,  4,  4,  4,  7,  4, // B
    5, 10, 10, 10, 11, 11,  7, 11,  5, 10, 10, 10, 11, 17,  7, 11, // C
    5, 10, 10, 10, 11, 11,  7, 11,  5, 10, 10, 10, 11, 17,  7, 11, // D
    5, 10, 10, 18, 11, 11,  7, 11,  5,  5, 10,  4, 11, 17,  7, 11, // E
    5, 10, 10,  4, 11, 11,  7, 11,  5,  5, 10,  4, 11, 17,  7, 11, // F
];

/// Extra T-states of a conditional CALL or RET when the condition holds.
const EXTRA_TAKEN: u32 = 6;

/// T-states of the HALT state per step (one machine cycle).
const HALT_CYCLES: u32 = 4;

/// Even parity of each byte value.
const fn parity_table() -> [bool; 256] {
    let mut t = [false; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = (i as u8).count_ones().is_multiple_of(2);
        i += 1;
    }
    t
}

static PARITY: [bool; 256] = parity_table();

#[derive(Clone, Debug, Default)]
pub struct Cpu {
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,

    // Flags
    pub sf: bool,
    pub zf: bool,
    pub hf: bool, // auxiliary carry (AC)
    pub pf: bool,
    pub cf: bool,

    /// Interrupt enable flip-flop (INTE).
    pub inte: bool,
    /// Set by EI: interrupts are not accepted before the next instruction ends.
    ei_delay: bool,
    pub halted: bool,

    /// Bytes supplied by the interrupt controller during INTA.
    inject: [u8; 3],
    inject_pos: u8,
    injecting: bool,
}

impl Cpu {
    pub fn new() -> Cpu {
        Cpu::default()
    }

    /// RESET input: PC = 0, interrupts disabled, HALT left. Other registers
    /// are unaffected, as on the real chip.
    pub fn reset(&mut self) {
        self.pc = 0;
        self.inte = false;
        self.ei_delay = false;
        self.halted = false;
        self.injecting = false;
    }

    // ---- register pairs and flags ----

    #[inline]
    pub fn bc(&self) -> u16 {
        u16::from_be_bytes([self.b, self.c])
    }

    #[inline]
    pub fn de(&self) -> u16 {
        u16::from_be_bytes([self.d, self.e])
    }

    #[inline]
    pub fn hl(&self) -> u16 {
        u16::from_be_bytes([self.h, self.l])
    }

    #[inline]
    fn set_bc(&mut self, v: u16) {
        [self.b, self.c] = v.to_be_bytes();
    }

    #[inline]
    fn set_de(&mut self, v: u16) {
        [self.d, self.e] = v.to_be_bytes();
    }

    #[inline]
    fn set_hl(&mut self, v: u16) {
        [self.h, self.l] = v.to_be_bytes();
    }

    /// Flag byte as pushed by PUSH PSW: S Z 0 AC 0 P 1 CY.
    pub fn flags(&self) -> u8 {
        (self.sf as u8) << 7
            | (self.zf as u8) << 6
            | (self.hf as u8) << 4
            | (self.pf as u8) << 2
            | 0x02
            | self.cf as u8
    }

    pub fn set_flags(&mut self, f: u8) {
        self.sf = f & 0x80 != 0;
        self.zf = f & 0x40 != 0;
        self.hf = f & 0x10 != 0;
        self.pf = f & 0x04 != 0;
        self.cf = f & 0x01 != 0;
    }

    #[inline]
    fn set_zsp(&mut self, v: u8) {
        self.zf = v == 0;
        self.sf = v & 0x80 != 0;
        self.pf = PARITY[v as usize];
    }

    // ---- fetch and stack ----

    #[inline]
    fn fetch_byte<B: Bus>(&mut self, bus: &mut B) -> u8 {
        if self.injecting {
            let pos = self.inject_pos as usize;
            self.inject_pos += 1;
            return if pos < 3 { self.inject[pos] } else { 0 };
        }
        let v = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }

    #[inline]
    fn fetch_word<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let lo = self.fetch_byte(bus);
        let hi = self.fetch_byte(bus);
        u16::from_le_bytes([lo, hi])
    }

    #[inline]
    fn read_word<B: Bus>(bus: &mut B, addr: u16) -> u16 {
        u16::from_le_bytes([bus.read(addr), bus.read(addr.wrapping_add(1))])
    }

    #[inline]
    fn write_word<B: Bus>(bus: &mut B, addr: u16, v: u16) {
        let [lo, hi] = v.to_le_bytes();
        bus.write(addr, lo);
        bus.write(addr.wrapping_add(1), hi);
    }

    #[inline]
    fn push<B: Bus>(&mut self, bus: &mut B, v: u16) {
        self.sp = self.sp.wrapping_sub(2);
        Self::write_word(bus, self.sp, v);
    }

    #[inline]
    fn pop<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let v = Self::read_word(bus, self.sp);
        self.sp = self.sp.wrapping_add(2);
        v
    }

    // ---- 8-bit register file by opcode index: B C D E H L M A ----

    #[inline]
    fn get_reg<B: Bus>(&mut self, bus: &mut B, r: u8) -> u8 {
        match r {
            0 => self.b,
            1 => self.c,
            2 => self.d,
            3 => self.e,
            4 => self.h,
            5 => self.l,
            6 => bus.read(self.hl()),
            _ => self.a,
        }
    }

    #[inline]
    fn set_reg<B: Bus>(&mut self, bus: &mut B, r: u8, v: u8) {
        match r {
            0 => self.b = v,
            1 => self.c = v,
            2 => self.d = v,
            3 => self.e = v,
            4 => self.h = v,
            5 => self.l = v,
            6 => bus.write(self.hl(), v),
            _ => self.a = v,
        }
    }

    // ---- ALU ----

    #[inline]
    fn add(&mut self, v: u8, carry: bool) {
        let result = self.a as u16 + v as u16 + carry as u16;
        self.hf = (self.a ^ v ^ result as u8) & 0x10 != 0;
        self.cf = result & 0x100 != 0;
        self.a = result as u8;
        self.set_zsp(self.a);
    }

    /// Subtraction is an addition of the complement with the borrow inverted,
    /// which gives the 8080's auxiliary carry behaviour.
    #[inline]
    fn sub(&mut self, v: u8, borrow: bool) {
        self.add(!v, !borrow);
        self.cf = !self.cf;
    }

    #[inline]
    fn cmp(&mut self, v: u8) {
        let a = self.a;
        self.sub(v, false);
        self.a = a;
    }

    #[inline]
    fn ana(&mut self, v: u8) {
        self.hf = (self.a | v) & 0x08 != 0;
        self.a &= v;
        self.cf = false;
        self.set_zsp(self.a);
    }

    #[inline]
    fn xra(&mut self, v: u8) {
        self.a ^= v;
        self.hf = false;
        self.cf = false;
        self.set_zsp(self.a);
    }

    #[inline]
    fn ora(&mut self, v: u8) {
        self.a |= v;
        self.hf = false;
        self.cf = false;
        self.set_zsp(self.a);
    }

    #[inline]
    fn alu(&mut self, op: u8, v: u8) {
        match op {
            0 => self.add(v, false),
            1 => self.add(v, self.cf),
            2 => self.sub(v, false),
            3 => self.sub(v, self.cf),
            4 => self.ana(v),
            5 => self.xra(v),
            6 => self.ora(v),
            _ => self.cmp(v),
        }
    }

    #[inline]
    fn inr(&mut self, v: u8) -> u8 {
        let r = v.wrapping_add(1);
        self.hf = r & 0x0F == 0;
        self.set_zsp(r);
        r
    }

    #[inline]
    fn dcr(&mut self, v: u8) -> u8 {
        let r = v.wrapping_sub(1);
        self.hf = r & 0x0F != 0x0F;
        self.set_zsp(r);
        r
    }

    #[inline]
    fn dad(&mut self, v: u16) {
        let (r, carry) = self.hl().overflowing_add(v);
        self.cf = carry;
        self.set_hl(r);
    }

    fn daa(&mut self) {
        let mut correction = 0u8;
        let mut carry = self.cf;
        let lsb = self.a & 0x0F;
        let msb = self.a >> 4;
        if self.hf || lsb > 9 {
            correction |= 0x06;
        }
        if self.cf || msb > 9 || (msb >= 9 && lsb > 9) {
            correction |= 0x60;
            carry = true;
        }
        self.add(correction, false);
        self.cf = carry;
    }

    /// Condition codes NZ Z NC C PO PE P M.
    #[inline]
    fn condition(&self, cc: u8) -> bool {
        match cc {
            0 => !self.zf,
            1 => self.zf,
            2 => !self.cf,
            3 => self.cf,
            4 => !self.pf,
            5 => self.pf,
            6 => !self.sf,
            _ => self.sf,
        }
    }

    // ---- execution ----

    /// Executes one instruction, or accepts a pending interrupt, or idles one
    /// machine cycle in HALT. Returns the T-states used.
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> u32 {
        if self.ei_delay {
            self.ei_delay = false;
        } else if self.inte && bus.int_pending() {
            return self.accept_interrupt(bus);
        }

        if self.halted {
            return HALT_CYCLES;
        }

        let opcode = self.fetch_byte(bus);
        self.execute(bus, opcode)
    }

    fn accept_interrupt<B: Bus>(&mut self, bus: &mut B) -> u32 {
        self.inte = false;
        self.halted = false;
        self.inject = bus.int_ack();
        self.inject_pos = 1;
        self.injecting = true;
        let opcode = self.inject[0];
        let cycles = self.execute(bus, opcode);
        self.injecting = false;
        cycles
    }

    fn execute<B: Bus>(&mut self, bus: &mut B, opcode: u8) -> u32 {
        bus.begin(opcode);
        let mut cycles = CYCLES[opcode as usize] as u32;

        match opcode {
            // MOV r, r' (76 is HLT)
            0x76 => self.halted = true,
            0x40..=0x7F => {
                let v = self.get_reg(bus, opcode & 7);
                self.set_reg(bus, (opcode >> 3) & 7, v);
            }

            // ALU A, r
            0x80..=0xBF => {
                let v = self.get_reg(bus, opcode & 7);
                self.alu((opcode >> 3) & 7, v);
            }

            // ALU A, imm
            0xC6 | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => {
                let v = self.fetch_byte(bus);
                self.alu((opcode >> 3) & 7, v);
            }

            // MVI r, imm
            0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x36 | 0x3E => {
                let v = self.fetch_byte(bus);
                self.set_reg(bus, (opcode >> 3) & 7, v);
            }

            // INR r / DCR r
            0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x34 | 0x3C => {
                let r = (opcode >> 3) & 7;
                let v = self.get_reg(bus, r);
                let v = self.inr(v);
                self.set_reg(bus, r, v);
            }
            0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x35 | 0x3D => {
                let r = (opcode >> 3) & 7;
                let v = self.get_reg(bus, r);
                let v = self.dcr(v);
                self.set_reg(bus, r, v);
            }

            // NOP and undocumented NOPs
            0x00 | 0x08 | 0x10 | 0x18 | 0x20 | 0x28 | 0x30 | 0x38 => {}

            // LXI rp, imm16
            0x01 => {
                let v = self.fetch_word(bus);
                self.set_bc(v);
            }
            0x11 => {
                let v = self.fetch_word(bus);
                self.set_de(v);
            }
            0x21 => {
                let v = self.fetch_word(bus);
                self.set_hl(v);
            }
            0x31 => self.sp = self.fetch_word(bus),

            // STAX / LDAX
            0x02 => bus.write(self.bc(), self.a),
            0x12 => bus.write(self.de(), self.a),
            0x0A => self.a = bus.read(self.bc()),
            0x1A => self.a = bus.read(self.de()),

            // SHLD / LHLD / STA / LDA
            0x22 => {
                let addr = self.fetch_word(bus);
                Self::write_word(bus, addr, self.hl());
            }
            0x2A => {
                let addr = self.fetch_word(bus);
                let v = Self::read_word(bus, addr);
                self.set_hl(v);
            }
            0x32 => {
                let addr = self.fetch_word(bus);
                bus.write(addr, self.a);
            }
            0x3A => {
                let addr = self.fetch_word(bus);
                self.a = bus.read(addr);
            }

            // INX / DCX
            0x03 => self.set_bc(self.bc().wrapping_add(1)),
            0x13 => self.set_de(self.de().wrapping_add(1)),
            0x23 => self.set_hl(self.hl().wrapping_add(1)),
            0x33 => self.sp = self.sp.wrapping_add(1),
            0x0B => self.set_bc(self.bc().wrapping_sub(1)),
            0x1B => self.set_de(self.de().wrapping_sub(1)),
            0x2B => self.set_hl(self.hl().wrapping_sub(1)),
            0x3B => self.sp = self.sp.wrapping_sub(1),

            // DAD
            0x09 => self.dad(self.bc()),
            0x19 => self.dad(self.de()),
            0x29 => self.dad(self.hl()),
            0x39 => self.dad(self.sp),

            // Rotates
            0x07 => {
                self.cf = self.a & 0x80 != 0;
                self.a = self.a.rotate_left(1);
            }
            0x0F => {
                self.cf = self.a & 0x01 != 0;
                self.a = self.a.rotate_right(1);
            }
            0x17 => {
                let carry = self.cf;
                self.cf = self.a & 0x80 != 0;
                self.a = (self.a << 1) | carry as u8;
            }
            0x1F => {
                let carry = self.cf;
                self.cf = self.a & 0x01 != 0;
                self.a = (self.a >> 1) | (carry as u8) << 7;
            }

            0x27 => self.daa(),
            0x2F => self.a = !self.a,
            0x37 => self.cf = true,
            0x3F => self.cf = !self.cf,

            // Jumps (CB is an undocumented JMP)
            0xC3 | 0xCB => self.pc = self.fetch_word(bus),
            0xC2 | 0xCA | 0xD2 | 0xDA | 0xE2 | 0xEA | 0xF2 | 0xFA => {
                let addr = self.fetch_word(bus);
                if self.condition((opcode >> 3) & 7) {
                    self.pc = addr;
                }
            }

            // Calls (DD, ED, FD are undocumented CALLs)
            0xCD | 0xDD | 0xED | 0xFD => {
                let addr = self.fetch_word(bus);
                self.push(bus, self.pc);
                self.pc = addr;
            }
            0xC4 | 0xCC | 0xD4 | 0xDC | 0xE4 | 0xEC | 0xF4 | 0xFC => {
                let addr = self.fetch_word(bus);
                if self.condition((opcode >> 3) & 7) {
                    self.push(bus, self.pc);
                    self.pc = addr;
                    cycles += EXTRA_TAKEN;
                }
            }

            // Returns (D9 is an undocumented RET)
            0xC9 | 0xD9 => self.pc = self.pop(bus),
            0xC0 | 0xC8 | 0xD0 | 0xD8 | 0xE0 | 0xE8 | 0xF0 | 0xF8 => {
                if self.condition((opcode >> 3) & 7) {
                    self.pc = self.pop(bus);
                    cycles += EXTRA_TAKEN;
                }
            }

            // RST n
            0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => {
                self.push(bus, self.pc);
                self.pc = (opcode & 0x38) as u16;
            }

            // PUSH / POP
            0xC5 => self.push(bus, self.bc()),
            0xD5 => self.push(bus, self.de()),
            0xE5 => self.push(bus, self.hl()),
            0xF5 => self.push(bus, u16::from_be_bytes([self.a, self.flags()])),
            0xC1 => {
                let v = self.pop(bus);
                self.set_bc(v);
            }
            0xD1 => {
                let v = self.pop(bus);
                self.set_de(v);
            }
            0xE1 => {
                let v = self.pop(bus);
                self.set_hl(v);
            }
            0xF1 => {
                let [a, f] = self.pop(bus).to_be_bytes();
                self.a = a;
                self.set_flags(f);
            }

            // Exchanges and SP/PC loads
            0xE3 => {
                let v = Self::read_word(bus, self.sp);
                Self::write_word(bus, self.sp, self.hl());
                self.set_hl(v);
            }
            0xEB => {
                core::mem::swap(&mut self.h, &mut self.d);
                core::mem::swap(&mut self.l, &mut self.e);
            }
            0xE9 => self.pc = self.hl(),
            0xF9 => self.sp = self.hl(),

            // I/O
            0xD3 => {
                let port = self.fetch_byte(bus);
                bus.output(port, self.a);
            }
            0xDB => {
                let port = self.fetch_byte(bus);
                self.a = bus.input(port);
            }

            // Interrupt control
            0xF3 => self.inte = false,
            0xFB => {
                self.inte = true;
                self.ei_delay = true;
            }
        }

        cycles
    }
}

/// Snapshot state (spec §4.6).
impl Cpu {
    pub(crate) fn save(&self, w: &mut Writer) {
        w.bytes(&[self.a, self.b, self.c, self.d, self.e, self.h, self.l]);
        w.u16(self.sp);
        w.u16(self.pc);
        let flags = [self.sf, self.zf, self.hf, self.pf, self.cf, self.inte, self.ei_delay, self.halted, self.injecting];
        for f in flags {
            w.bool(f);
        }
        w.bytes(&self.inject);
        w.u8(self.inject_pos);
    }

    pub(crate) fn load(r: &mut Reader) -> Option<Cpu> {
        let [a, b, c, d, e, h, l] = r.array()?;
        let (sp, pc) = (r.u16()?, r.u16()?);
        let mut flags = [false; 9];
        for f in &mut flags {
            *f = r.bool()?;
        }
        let [sf, zf, hf, pf, cf, inte, ei_delay, halted, injecting] = flags;
        let inject = r.array()?;
        let inject_pos = r.u8()?;
        Some(Cpu { a, b, c, d, e, h, l, sp, pc, sf, zf, hf, pf, cf, inte, ei_delay, halted, inject, inject_pos, injecting })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestBus {
        mem: Vec<u8>,
        irq: bool,
        out: Vec<(u8, u8)>,
    }

    impl TestBus {
        fn new(program: &[u8]) -> TestBus {
            let mut mem = vec![0; 0x10000];
            mem[..program.len()].copy_from_slice(program);
            TestBus { mem, irq: false, out: Vec::new() }
        }
    }

    impl Bus for TestBus {
        fn read(&mut self, addr: u16) -> u8 {
            self.mem[addr as usize]
        }
        fn write(&mut self, addr: u16, val: u8) {
            self.mem[addr as usize] = val;
        }
        fn input(&mut self, port: u8) -> u8 {
            port ^ 0xFF
        }
        fn output(&mut self, port: u8, val: u8) {
            self.out.push((port, val));
        }
        fn int_pending(&self) -> bool {
            self.irq
        }
        fn int_ack(&mut self) -> [u8; 3] {
            self.irq = false;
            [0xCD, 0x40, 0xFE] // CALL FE40h, as the 8259 supplies it
        }
    }

    fn run(cpu: &mut Cpu, bus: &mut TestBus, steps: usize) -> u32 {
        (0..steps).map(|_| cpu.step(bus)).sum()
    }

    #[test]
    fn push_psw_has_fixed_bits() {
        let mut cpu = Cpu::new();
        cpu.set_flags(0xFF);
        assert_eq!(cpu.flags(), 0xD7);
        cpu.set_flags(0x00);
        assert_eq!(cpu.flags(), 0x02);
    }

    #[test]
    fn ana_sets_aux_carry_from_bit3_or() {
        let mut cpu = Cpu::new();
        cpu.a = 0x08;
        cpu.ana(0x00);
        assert!(cpu.hf);
        cpu.a = 0x00;
        cpu.ana(0x00);
        assert!(!cpu.hf);
    }

    #[test]
    fn daa_after_bcd_add() {
        let mut cpu = Cpu::new();
        cpu.a = 0x19;
        cpu.add(0x28, false); // 0x41, AC set
        cpu.daa();
        assert_eq!(cpu.a, 0x47);
        assert!(!cpu.cf);
        cpu.a = 0x99;
        cpu.add(0x01, false);
        cpu.daa();
        assert_eq!(cpu.a, 0x00);
        assert!(cpu.cf);
    }

    #[test]
    fn conditional_call_and_ret_timing() {
        // CNZ 0010h (taken since Z=0), at 0010h: RZ (not taken), RET
        let mut bus = TestBus::new(&[0xC4, 0x10, 0x00]);
        bus.mem[0x10] = 0xC8;
        bus.mem[0x11] = 0xC9;
        let mut cpu = Cpu::new();
        cpu.sp = 0x8000;
        assert_eq!(cpu.step(&mut bus), 17);
        assert_eq!(cpu.pc, 0x10);
        assert_eq!(cpu.step(&mut bus), 5);
        assert_eq!(cpu.step(&mut bus), 10);
        assert_eq!(cpu.pc, 3);
    }

    #[test]
    fn interrupt_injects_call_after_ei_delay() {
        // EI, NOP, NOP ...
        let mut bus = TestBus::new(&[0xFB, 0x00, 0x00, 0x00]);
        let mut cpu = Cpu::new();
        cpu.sp = 0x8000;
        bus.irq = true;
        cpu.step(&mut bus); // EI
        cpu.step(&mut bus); // NOP: interrupt held off by the EI delay
        assert_eq!(cpu.pc, 2);
        let cycles = cpu.step(&mut bus); // INTA → CALL FE40h
        assert_eq!(cycles, 17);
        assert_eq!(cpu.pc, 0xFE40);
        assert!(!cpu.inte);
        assert_eq!(Cpu::read_word(&mut bus, cpu.sp), 2);
    }

    #[test]
    fn halt_waits_for_interrupt() {
        let mut bus = TestBus::new(&[0xFB, 0x76, 0x00]);
        let mut cpu = Cpu::new();
        cpu.sp = 0x8000;
        run(&mut cpu, &mut bus, 2);
        assert!(cpu.halted);
        assert_eq!(cpu.step(&mut bus), HALT_CYCLES);
        bus.irq = true;
        cpu.step(&mut bus);
        assert!(!cpu.halted);
        assert_eq!(cpu.pc, 0xFE40);
        assert_eq!(Cpu::read_word(&mut bus, cpu.sp), 2);
    }

    #[test]
    fn io_instructions() {
        // MVI A,5Ah ; OUT 07h ; IN 10h
        let mut bus = TestBus::new(&[0x3E, 0x5A, 0xD3, 0x07, 0xDB, 0x10]);
        let mut cpu = Cpu::new();
        assert_eq!(run(&mut cpu, &mut bus, 3), 7 + 10 + 10);
        assert_eq!(bus.out, vec![(0x07, 0x5A)]);
        assert_eq!(cpu.a, 0xEF);
    }
}

//! Intel 8259A programmable interrupt controller (КР580ВН59), 8080 mode.
//!
//! Single controller (no cascade). On acknowledge it supplies a 3-byte
//! `CALL` whose address comes from ICW1/ICW2 with an interval of 4 or 8.

use crate::snapshot::{Reader, Writer};

#[derive(Clone, Debug)]
pub struct Pic {
    irr: u8,
    isr: u8,
    imr: u8,
    /// Current IR input levels.
    lines: u8,
    icw1: u8,
    icw2: u8,
    /// Next initialisation word expected: 0 = none (operational), 2, 3 or 4.
    init_step: u8,
    auto_eoi: bool,
    read_isr: bool,
    poll: bool,
    /// The IR with the lowest priority; the next one round has the highest.
    lowest: u8,
}

impl Default for Pic {
    fn default() -> Pic {
        Pic::new()
    }
}

impl Pic {
    pub fn new() -> Pic {
        Pic {
            irr: 0,
            isr: 0,
            imr: 0xFF,
            lines: 0,
            icw1: 0,
            icw2: 0,
            init_step: 0,
            auto_eoi: false,
            read_isr: false,
            poll: false,
            lowest: 7,
        }
    }

    fn level_triggered(&self) -> bool {
        self.icw1 & 0x08 != 0
    }

    /// Drives IR input `n`.
    pub fn set_irq(&mut self, n: u8, level: bool) {
        let mask = 1 << n;
        if level {
            if self.lines & mask == 0 || self.level_triggered() {
                self.irr |= mask;
            }
            self.lines |= mask;
        } else {
            // The request must be held until acknowledged.
            self.lines &= !mask;
            self.irr &= !mask;
        }
    }

    /// IR numbers from highest to lowest priority.
    fn priority_order(&self) -> impl Iterator<Item = u8> {
        let first = (self.lowest + 1) & 7;
        (0..8).map(move |i| (first + i) & 7)
    }

    /// Highest-priority request that may interrupt now.
    fn pending(&self) -> Option<u8> {
        if self.init_step != 0 {
            return None;
        }
        let req = self.irr & !self.imr;
        for n in self.priority_order() {
            if self.isr & (1 << n) != 0 {
                return None;
            }
            if req & (1 << n) != 0 {
                return Some(n);
            }
        }
        None
    }

    /// Interrupt mask register (OCW1).
    pub fn mask(&self) -> u8 {
        self.imr
    }

    /// IRR, ISR and IMR, for the debugger.
    pub fn registers(&self) -> [u8; 3] {
        [self.irr, self.isr, self.imr]
    }

    /// State of the INT output.
    pub fn int(&self) -> bool {
        self.pending().is_some()
    }

    /// Marks the highest pending request as in service and returns its IR
    /// (IR7 if the request went away: a spurious interrupt).
    fn accept(&mut self) -> u8 {
        match self.pending() {
            Some(n) => {
                if !self.level_triggered() {
                    self.irr &= !(1 << n);
                }
                if !self.auto_eoi {
                    self.isr |= 1 << n;
                }
                n
            }
            None => 7,
        }
    }

    /// INTA cycle: the CALL instruction for the accepted interrupt.
    pub fn ack(&mut self) -> [u8; 3] {
        let n = self.accept();
        let lo = if self.icw1 & 0x04 != 0 {
            (self.icw1 & 0xE0) | (n << 2)
        } else {
            (self.icw1 & 0xC0) | (n << 3)
        };
        [0xCD, lo, self.icw2]
    }

    fn highest_in_service(&self) -> Option<u8> {
        self.priority_order().find(|&n| self.isr & (1 << n) != 0)
    }

    pub fn write(&mut self, a0: u8, v: u8) {
        if a0 & 1 == 0 {
            if v & 0x10 != 0 {
                // ICW1
                self.icw1 = v;
                self.init_step = 2;
                self.imr = 0;
                self.isr = 0;
                self.irr = if self.level_triggered() { self.lines } else { 0 };
                self.lowest = 7;
                self.read_isr = false;
                self.poll = false;
                self.auto_eoi = false;
            } else if v & 0x08 != 0 {
                // OCW3
                self.poll = v & 0x04 != 0;
                if v & 0x02 != 0 {
                    self.read_isr = v & 0x01 != 0;
                }
            } else {
                // OCW2
                let level = v & 7;
                match v >> 5 {
                    0b001 => {
                        if let Some(n) = self.highest_in_service() {
                            self.isr &= !(1 << n);
                        }
                    }
                    0b011 => self.isr &= !(1 << level),
                    0b101 => {
                        if let Some(n) = self.highest_in_service() {
                            self.isr &= !(1 << n);
                            self.lowest = n;
                        }
                    }
                    0b111 => {
                        self.isr &= !(1 << level);
                        self.lowest = level;
                    }
                    0b110 => self.lowest = level,
                    _ => {}
                }
            }
            return;
        }
        let single = self.icw1 & 0x02 != 0;
        let icw4 = self.icw1 & 0x01 != 0;
        self.init_step = match self.init_step {
            2 => {
                self.icw2 = v;
                if !single {
                    3
                } else if icw4 {
                    4
                } else {
                    0
                }
            }
            3 => {
                if icw4 { 4 } else { 0 }
            }
            4 => {
                self.auto_eoi = v & 0x02 != 0;
                0
            }
            _ => {
                self.imr = v;
                0
            }
        };
    }

    pub fn read(&mut self, a0: u8) -> u8 {
        if a0 & 1 != 0 {
            return self.imr;
        }
        if self.poll {
            self.poll = false;
            return match self.pending() {
                Some(_) => 0x80 | self.accept(),
                None => 0,
            };
        }
        if self.read_isr { self.isr } else { self.irr }
    }
}

/// Snapshot state (spec §4.6).
impl Pic {
    pub(crate) fn save(&self, w: &mut Writer) {
        w.bytes(&[self.irr, self.isr, self.imr, self.lines, self.icw1, self.icw2, self.init_step, self.lowest]);
        w.bool(self.auto_eoi);
        w.bool(self.read_isr);
        w.bool(self.poll);
    }

    pub(crate) fn load(r: &mut Reader) -> Option<Pic> {
        let [irr, isr, imr, lines, icw1, icw2, init_step, lowest] = r.array()?;
        if !matches!(init_step, 0 | 2 | 3 | 4) || lowest > 7 {
            return None;
        }
        let (auto_eoi, read_isr, poll) = (r.bool()?, r.bool()?, r.bool()?);
        Some(Pic { irr, isr, imr, lines, icw1, icw2, init_step, auto_eoi, read_isr, poll, lowest })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Initialised as RomBios does: ICW1 D6h, ICW2 FEh, all unmasked.
    fn rombios_pic() -> Pic {
        let mut p = Pic::new();
        p.write(0, 0xD6);
        p.write(1, 0xFE);
        p.write(1, 0x00);
        p
    }

    #[test]
    fn call_vector_interval_4() {
        let mut p = rombios_pic();
        p.set_irq(5, true);
        assert!(p.int());
        assert_eq!(p.ack(), [0xCD, 0xD4, 0xFE]); // CALL FED4h = FEC0h + 4·5
        assert!(!p.int());
    }

    #[test]
    fn requests_are_edge_triggered() {
        let mut p = rombios_pic();
        p.set_irq(5, true);
        p.ack();
        p.write(0, 0x20); // EOI
        assert!(!p.int(), "a held line must not re-trigger");
        p.set_irq(5, false);
        p.set_irq(5, true);
        assert!(p.int());
    }

    #[test]
    fn fully_nested_priority_and_eoi() {
        let mut p = rombios_pic();
        p.set_irq(5, true);
        assert_eq!(p.ack()[1], 0xD4);
        p.set_irq(6, true);
        assert!(!p.int(), "lower priority waits for EOI");
        p.set_irq(2, true);
        assert!(p.int(), "higher priority nests");
        assert_eq!(p.ack()[1], 0xC8);
        p.write(0, 0x20); // clears IR2
        assert!(!p.int());
        p.write(0, 0x20); // clears IR5
        assert_eq!(p.ack()[1], 0xD8);
    }

    #[test]
    fn mask_blocks_requests() {
        let mut p = rombios_pic();
        p.write(1, 0xDF);
        p.set_irq(6, true);
        assert!(!p.int());
        assert_eq!(p.read(1), 0xDF);
        p.set_irq(5, true);
        assert!(p.int());
        assert_eq!(p.read(0), 0x60, "IRR");
    }

    #[test]
    fn interval_8_and_poll() {
        let mut p = Pic::new();
        p.write(0, 0x12); // interval 8, single, edge
        p.write(1, 0x20);
        p.write(1, 0x00);
        p.set_irq(3, true);
        assert_eq!(p.ack(), [0xCD, 0x18, 0x20]);
        p.write(0, 0x20);
        p.set_irq(1, true);
        p.write(0, 0x0C); // poll
        assert_eq!(p.read(0), 0x81);
    }
}

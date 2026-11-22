//! Intel 8255A programmable peripheral interface (КР580ВВ55), mode 0.
//!
//! Only basic input/output (mode 0) is modelled; the Juku uses nothing else.
//! The machine supplies input-port values on each read and looks at the
//! output latches after each write.

use crate::snapshot::{Reader, Writer};

#[derive(Clone, Debug)]
pub struct Ppi {
    /// Output latches of ports A, B and C.
    pub a: u8,
    pub b: u8,
    pub c: u8,
    ctrl: u8,
}

impl Default for Ppi {
    fn default() -> Ppi {
        Ppi::new()
    }
}

impl Ppi {
    pub fn new() -> Ppi {
        Ppi { a: 0, b: 0, c: 0, ctrl: 0x9B }
    }

    /// RESET input: all ports become inputs, latches are cleared.
    pub fn reset(&mut self) {
        *self = Ppi::new();
    }

    pub fn write(&mut self, reg: u8, v: u8) {
        match reg & 3 {
            0 => self.a = v,
            1 => self.b = v,
            2 => self.c = v,
            _ if v & 0x80 != 0 => {
                // Mode set: clears all output latches.
                self.ctrl = v;
                self.a = 0;
                self.b = 0;
                self.c = 0;
            }
            _ => {
                // Port C bit set/reset.
                let bit = 1 << ((v >> 1) & 7);
                if v & 1 != 0 {
                    self.c |= bit;
                } else {
                    self.c &= !bit;
                }
            }
        }
    }

    /// Reads a register. `ext` gives the levels on the A, B and C pins, used
    /// for whichever ports (or port C halves) are inputs.
    pub fn read(&self, reg: u8, ext: [u8; 3]) -> u8 {
        let pick = |latch: u8, pins: u8, input_mask: u8| (latch & !input_mask) | (pins & input_mask);
        match reg & 3 {
            0 => pick(self.a, ext[0], if self.ctrl & 0x10 != 0 { 0xFF } else { 0 }),
            1 => pick(self.b, ext[1], if self.ctrl & 0x02 != 0 { 0xFF } else { 0 }),
            2 => {
                let mut m = 0;
                if self.ctrl & 0x08 != 0 {
                    m |= 0xF0;
                }
                if self.ctrl & 0x01 != 0 {
                    m |= 0x0F;
                }
                pick(self.c, ext[2], m)
            }
            _ => 0xFF,
        }
    }
}

/// Snapshot state (spec §4.6).
impl Ppi {
    pub(crate) fn save(&self, w: &mut Writer) {
        w.bytes(&[self.a, self.b, self.c, self.ctrl]);
    }

    pub(crate) fn load(r: &mut Reader) -> Option<Ppi> {
        let [a, b, c, ctrl] = r.array()?;
        Some(Ppi { a, b, c, ctrl })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_word_sets_directions_and_clears_latches() {
        let mut p = Ppi::new();
        p.write(0, 0x55);
        p.write(3, 0x82); // A out, B in, C out
        assert_eq!(p.a, 0);
        p.write(0, 0x27);
        assert_eq!(p.read(0, [0xFF; 3]), 0x27);
        assert_eq!(p.read(1, [0x3C, 0x5A, 0xFF]), 0x5A);
        assert_eq!(p.read(2, [0xFF; 3]), 0x00);
    }

    #[test]
    fn port_c_bit_set_reset() {
        let mut p = Ppi::new();
        p.write(3, 0x82);
        p.write(3, 0x0F); // set PC7
        assert_eq!(p.c, 0x80);
        p.write(3, 0x0E); // reset PC7
        assert_eq!(p.c, 0x00);
        p.write(3, 0x03); // set PC1
        assert_eq!(p.c, 0x02);
    }

    #[test]
    fn inputs_after_reset() {
        let p = Ppi::new();
        assert_eq!(p.read(0, [0x12, 0x34, 0x56]), 0x12);
        assert_eq!(p.read(2, [0x12, 0x34, 0x56]), 0x56);
    }
}

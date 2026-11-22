//! Intel 8251A USART (КР580ВВ51) — a stub with no serial line attached.
//!
//! It tracks the mode/command word sequence so software can program it, and
//! reports an idle transmitter with nothing received. Transmitted bytes are
//! discarded.

use crate::snapshot::{Reader, Writer};

#[derive(Clone, Debug, Default)]
pub struct Usart {
    /// True while the next control write is a mode word (after reset).
    expect_mode: bool,
    /// Remaining sync characters expected after a synchronous mode word.
    sync_chars: u8,
    command: u8,
}

const TXRDY: u8 = 0x01;
const TXEMPTY: u8 = 0x04;

impl Usart {
    pub fn new() -> Usart {
        Usart { expect_mode: true, sync_chars: 0, command: 0 }
    }

    pub fn reset(&mut self) {
        *self = Usart::new();
    }

    pub fn write(&mut self, a0: u8, v: u8) {
        if a0 & 1 == 0 {
            return; // data: discarded
        }
        if self.sync_chars > 0 {
            self.sync_chars -= 1;
        } else if self.expect_mode {
            self.expect_mode = false;
            if v & 0x03 == 0 {
                // Synchronous mode: one or two sync characters follow.
                self.sync_chars = if v & 0x80 != 0 { 1 } else { 2 };
            }
        } else if v & 0x40 != 0 {
            // Internal reset: back to expecting a mode word.
            self.expect_mode = true;
        } else {
            self.command = v;
        }
    }

    pub fn read(&mut self, a0: u8) -> u8 {
        if a0 & 1 == 0 { 0x00 } else { TXRDY | TXEMPTY }
    }
}

/// Snapshot state (spec §4.6).
impl Usart {
    pub(crate) fn save(&self, w: &mut Writer) {
        w.bool(self.expect_mode);
        w.u8(self.sync_chars);
        w.u8(self.command);
    }

    pub(crate) fn load(r: &mut Reader) -> Option<Usart> {
        Some(Usart { expect_mode: r.bool()?, sync_chars: r.u8()?, command: r.u8()? })
    }
}

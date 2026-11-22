//! Keyboard ДГШ5.104.015: a 16-column × 6-row matrix scanned by the CPU
//! through 8255 #1, a 74148 row encoder, SHIFT/CTRL lines and the CONTRDAT
//! DIP switch banks (MD §6).
//!
//! Keys are identified by Juku key IDs: `column << 3 | row`, with the column
//! as selected on PA3..0 and the row as the BIOS reads it (MD §6.3), plus
//! [`KEY_SHIFT`] and [`KEY_CTRL`]. Mapping host keys to IDs is the host's job.

use crate::snapshot::{Reader, Writer};

pub const KEY_SHIFT: u8 = 0x80;
pub const KEY_CTRL: u8 = 0x81;

/// Builds a key ID from a matrix column (0–15) and row (0–5).
pub const fn key_id(col: u8, row: u8) -> u8 {
    col << 3 | row
}

#[derive(Clone, Debug)]
pub struct Keyboard {
    /// Bit r of `cols[c]` = key at (column c, row r) is down.
    cols: [u8; 16],
    shift: bool,
    ctrl: bool,
    /// DIP banks read on PB4 and PB5 while columns 8–15 are selected:
    /// bit (column − 8) set = switch ON = the line reads 0.
    dip_pb4: u8,
    dip_pb5: u8,
}

impl Default for Keyboard {
    fn default() -> Keyboard {
        Keyboard::new()
    }
}

impl Keyboard {
    /// Defaults as in MAME: the PB5 bank (network settings) all OFF, the PB4
    /// bank reading 0.
    pub fn new() -> Keyboard {
        Keyboard { cols: [0; 16], shift: false, ctrl: false, dip_pb4: 0xFF, dip_pb5: 0x00 }
    }

    /// Presses or releases a key. Unknown IDs are ignored.
    pub fn set_key(&mut self, id: u8, down: bool) {
        match id {
            KEY_SHIFT => self.shift = down,
            KEY_CTRL => self.ctrl = down,
            _ if id < 0x80 && id & 7 < 6 => {
                let (col, bit) = ((id >> 3) as usize, 1 << (id & 7));
                if down {
                    self.cols[col] |= bit;
                } else {
                    self.cols[col] &= !bit;
                }
            }
            _ => {}
        }
    }

    pub fn release_all(&mut self) {
        self.cols = [0; 16];
        self.shift = false;
        self.ctrl = false;
    }

    /// Sets a DIP bank: `bank` is 4 or 5 (the PB bit it is read on).
    pub fn set_dip(&mut self, bank: u8, value: u8) {
        match bank {
            4 => self.dip_pb4 = value,
            5 => self.dip_pb5 = value,
            _ => {}
        }
    }

    /// Port B of 8255 #1 with column `col` (PA3..0) selected.
    pub fn port_b(&self, col: u8) -> u8 {
        let col = col & 0x0F;
        let rows = self.cols[col as usize];
        let mut pb = if rows == 0 {
            0x0F // −FK = 1, K2..K0 = 111
        } else {
            let r = 7 - rows.leading_zeros() as u8; // 74148: highest row wins
            (!r & 7) << 1
        };
        if !self.shift {
            pb |= 0x40;
        }
        if !self.ctrl {
            pb |= 0x80;
        }
        let dip_bit = if col >= 8 { 1 << (col - 8) } else { 0 };
        if self.dip_pb4 & dip_bit == 0 {
            pb |= 0x10;
        }
        if self.dip_pb5 & dip_bit == 0 {
            pb |= 0x20;
        }
        pb
    }
}

/// The CAPS key: column 3, row 5 (MD §6.3).
pub const KEY_CAPS: u8 = key_id(3, 5);

/// RomBios 3.43m keeps its Caps Lock state in bit 0 of CONSW (MD §14). Each
/// CAPS press flips it, and the BIOS clears it during start-up.
pub const CONSW: u16 = 0xD4BA;

/// Frames the follower holds CAPS down, and frames it then waits before
/// looking at CONSW again. The BIOS scans the keyboard once per frame.
const CAPS_HOLD_FRAMES: u8 = 3;
const CAPS_WAIT_FRAMES: u8 = 5;
/// Presses tried per sync request before giving up, so a program that uses
/// CONSW for something else gets only a few stray CAPS presses.
const CAPS_TRIES: u8 = 4;

/// Makes the guest Caps Lock follow a target state, the host's (spec §4.5).
/// The Juku CAPS key only toggles, so while CONSW bit 0 differs from the
/// target, the follower presses CAPS. It presses only while software scans
/// the CAPS column, since a press nobody reads would be lost.
#[derive(Clone, Debug, Default)]
pub struct CapsFollower {
    target: Option<bool>,
    tries: u8,
    /// Frames left with CAPS held down, then frames left to wait.
    hold: u8,
    wait: u8,
    /// The CAPS column was read since the last frame.
    scanned: bool,
}

impl CapsFollower {
    /// Sets the target. A change starts a new sync request.
    pub fn set_target(&mut self, on: bool) {
        if self.target != Some(on) {
            self.target = Some(on);
            self.tries = CAPS_TRIES;
        }
    }

    /// Starts a new sync request: after a reset the BIOS clears CONSW.
    pub fn rearm(&mut self, kbd: &mut Keyboard) {
        if self.hold > 0 {
            kbd.set_key(KEY_CAPS, false);
        }
        self.hold = 0;
        self.wait = 0;
        self.scanned = false;
        self.tries = CAPS_TRIES;
    }

    /// Software read the keyboard with column `col` selected.
    pub fn note_scan(&mut self, col: u8) {
        if col & 0x0F == KEY_CAPS >> 3 {
            self.scanned = true;
        }
    }

    /// Called once per frame with the current CONSW.
    pub fn frame(&mut self, consw: u8, kbd: &mut Keyboard) {
        let scanned = std::mem::take(&mut self.scanned);
        if self.hold > 0 {
            self.hold -= 1;
            if self.hold == 0 {
                kbd.set_key(KEY_CAPS, false);
                self.wait = CAPS_WAIT_FRAMES;
            }
            return;
        }
        if self.wait > 0 {
            self.wait -= 1;
            return;
        }
        let Some(target) = self.target else { return };
        if self.tries == 0 || !scanned || (consw & 1 != 0) == target {
            return;
        }
        self.tries -= 1;
        kbd.set_key(KEY_CAPS, true);
        self.hold = CAPS_HOLD_FRAMES;
    }
}

/// Snapshot state (spec §4.6).
impl Keyboard {
    pub(crate) fn save(&self, w: &mut Writer) {
        w.bytes(&self.cols);
        w.bool(self.shift);
        w.bool(self.ctrl);
        w.u8(self.dip_pb4);
        w.u8(self.dip_pb5);
    }

    pub(crate) fn load(r: &mut Reader) -> Option<Keyboard> {
        let cols: [u8; 16] = r.array()?;
        if cols.iter().any(|c| c & !0x3F != 0) {
            return None;
        }
        Some(Keyboard { cols, shift: r.bool()?, ctrl: r.bool()?, dip_pb4: r.u8()?, dip_pb5: r.u8()? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `frames` frames of a BIOS that scans every frame and flips
    /// CONSW on each CAPS press. Returns the new CONSW.
    fn run(f: &mut CapsFollower, k: &mut Keyboard, mut consw: u8, frames: usize, bios: bool) -> u8 {
        let mut was_down = false;
        for _ in 0..frames {
            if bios {
                f.note_scan(3);
                let down = k.port_b(3) & 1 == 0;
                if down && !was_down {
                    consw ^= 1;
                }
                was_down = down;
            }
            f.frame(consw, k);
        }
        consw
    }

    #[test]
    fn caps_follower_presses_until_in_step() {
        let (mut f, mut k) = (CapsFollower::default(), Keyboard::new());
        assert_eq!(run(&mut f, &mut k, 0, 20, true), 0, "no target, no presses");
        f.set_target(true);
        assert_eq!(run(&mut f, &mut k, 0, 20, true), 1);
        assert_eq!(k.port_b(3), 0xFF, "CAPS released");
        f.set_target(false);
        assert_eq!(run(&mut f, &mut k, 1, 20, true), 0);
    }

    #[test]
    fn caps_follower_waits_for_a_scan_and_gives_up() {
        let (mut f, mut k) = (CapsFollower::default(), Keyboard::new());
        f.set_target(true);
        run(&mut f, &mut k, 0, 20, false);
        assert_eq!(f.tries, CAPS_TRIES, "nothing scans the keyboard");
        // CONSW that never changes: a few presses, then none.
        let mut presses = 0;
        for _ in 0..200 {
            f.note_scan(3);
            let before = k.port_b(3) & 1;
            f.frame(0, &mut k);
            presses += (before == 1 && k.port_b(3) & 1 == 0) as u32;
        }
        assert_eq!(presses, CAPS_TRIES as u32);
    }

    #[test]
    fn idle_columns() {
        let k = Keyboard::new();
        assert_eq!(k.port_b(0), 0xFF);
        assert_eq!(k.port_b(8), 0xEF, "PB4 bank reads 0 on columns 8–15");
    }

    #[test]
    fn encoder_reports_highest_row() {
        let mut k = Keyboard::new();
        k.set_key(key_id(5, 1), true); // Z
        assert_eq!(k.port_b(5), 0xF0 | (!1 & 7) << 1);
        k.set_key(key_id(5, 5), true); // A
        assert_eq!((!k.port_b(5) >> 1) & 7, 5);
        assert_eq!(k.port_b(5) & 1, 0, "−FK active");
        assert_eq!(k.port_b(4), 0xFF, "other columns idle");
        k.set_key(key_id(5, 5), false);
        assert_eq!((!k.port_b(5) >> 1) & 7, 1);
    }

    #[test]
    fn shift_ctrl_and_dip() {
        let mut k = Keyboard::new();
        k.set_key(KEY_SHIFT, true);
        k.set_key(KEY_CTRL, true);
        assert_eq!(k.port_b(0), 0x3F);
        k.release_all();
        k.set_dip(5, 0b0000_0100);
        assert_eq!(k.port_b(10) & 0x20, 0);
        assert_eq!(k.port_b(9) & 0x20, 0x20);
        assert_eq!(k.port_b(2) & 0x20, 0x20);
    }
}

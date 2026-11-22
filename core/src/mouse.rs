//! E4701 mouse and its interface card on the I-41 bus (MD §10.4).
//!
//! The card answers I/O reads at 80h/81h with one event per read and raises
//! INT6 (8259 IR6) while it has something to report. As in MAME, the host
//! mouse is polled at 600 Hz; each poll that finds movement or a button change
//! requests an interrupt. Each read reports at most one count per axis; the
//! rest of the host movement stays queued for later reads. As on the real
//! card, whose latches hold one step per axis, a step that comes while the
//! previous event is still unread is lost, so a slow reader sees the cursor
//! move less rather than lag behind.

use crate::machine::CPU_HZ;
use crate::snapshot::{Reader, Writer};

/// Polls per second, as in MAME (real hardware peaks near 1400 events/s).
const POLL_HZ: u64 = 600;
/// Largest backlog of counts per axis (about 0.1 s of steps at the poll rate).
const MAX_BACKLOG: i32 = 64;

pub const BUTTON_RIGHT: u8 = 0x01;
pub const BUTTON_LEFT: u8 = 0x02;

#[derive(Clone, Debug, Default)]
pub struct Mouse {
    /// Counts still to report: +x = right, +y = down.
    dx: i32,
    dy: i32,
    /// Current buttons and the buttons in the last byte read.
    buttons: u8,
    reported: u8,
    /// Number of the next poll; it happens at `poll * CPU_HZ / POLL_HZ`.
    poll: u64,
    /// An interrupt was requested and the event has not been read yet.
    pending: bool,
}

impl Mouse {
    pub fn new() -> Mouse {
        Mouse::default()
    }

    /// Host input: movement in Juku counts and the button state
    /// ([`BUTTON_LEFT`], [`BUTTON_RIGHT`]).
    pub fn input(&mut self, dx: i32, dy: i32, buttons: u8) {
        self.dx = (self.dx + dx).clamp(-MAX_BACKLOG, MAX_BACKLOG);
        self.dy = (self.dy + dy).clamp(-MAX_BACKLOG, MAX_BACKLOG);
        self.buttons = buttons & (BUTTON_LEFT | BUTTON_RIGHT);
    }

    /// Cycle of the next poll.
    pub fn next_poll(&self) -> u64 {
        self.poll * CPU_HZ / POLL_HZ
    }

    /// Performs the next poll. Returns true if the card should (re)request
    /// an interrupt.
    pub fn do_poll(&mut self) -> bool {
        self.poll += 1;
        if self.pending {
            // The latches are still full: this poll's steps are lost.
            self.dx -= self.dx.signum();
            self.dy -= self.dy.signum();
        }
        self.pending = self.dx != 0 || self.dy != 0 || self.buttons != self.reported;
        self.pending
    }

    /// Skips polls that are already in the past (after the machine was idle
    /// or the time base jumped).
    pub fn catch_up(&mut self, now: u64) {
        let due = now * POLL_HZ / CPU_HZ;
        if self.poll < due {
            self.poll = due;
        }
    }

    /// `IN 80h`: one event. Bit 7 = 1, bit 6 = 0, bit 5/4 = Y step and
    /// direction (1 = down), bit 3/2 = X step and direction (1 = left), bit 1
    /// = left button, bit 0 = right button (1 = pressed).
    pub fn read(&mut self) -> u8 {
        let mut v = 0x80 | self.buttons;
        if self.dx != 0 {
            v |= if self.dx < 0 { 0x0C } else { 0x08 };
            self.dx -= self.dx.signum();
        }
        if self.dy != 0 {
            v |= if self.dy > 0 { 0x30 } else { 0x20 };
            self.dy -= self.dy.signum();
        }
        self.reported = self.buttons;
        self.pending = false;
        v
    }
}

/// Snapshot state (spec §4.6).
impl Mouse {
    pub(crate) fn save(&self, w: &mut Writer) {
        w.i32(self.dx);
        w.i32(self.dy);
        w.u8(self.buttons);
        w.u8(self.reported);
        w.u64(self.poll);
        w.bool(self.pending);
    }

    /// Restores the card at cycle `now`. Polls are numbered from power-on,
    /// so the next one cannot be far ahead of `now`.
    pub(crate) fn load(r: &mut Reader, now: u64) -> Option<Mouse> {
        let axis = |v: &i32| v.abs() <= MAX_BACKLOG;
        let buttons = |b: &u8| b & !(BUTTON_LEFT | BUTTON_RIGHT) == 0;
        let dx = r.i32().filter(axis)?;
        let dy = r.i32().filter(axis)?;
        let buttons_now = r.u8().filter(buttons)?;
        let reported = r.u8().filter(buttons)?;
        let poll = r.u64().filter(|&p| p <= now * POLL_HZ / CPU_HZ + 2)?;
        Some(Mouse { dx, dy, buttons: buttons_now, reported, poll, pending: r.bool()? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_mouse_reads_80h_and_requests_nothing() {
        let mut m = Mouse::new();
        assert!(!m.do_poll());
        assert_eq!(m.read(), 0x80);
    }

    #[test]
    fn one_count_per_axis_per_read() {
        let mut m = Mouse::new();
        m.input(-2, 1, 0);
        assert!(m.do_poll());
        assert_eq!(m.read(), 0x80 | 0x0C | 0x30, "left and down");
        assert_eq!(m.read(), 0x80 | 0x0C, "left only");
        assert_eq!(m.read(), 0x80);
        assert!(!m.do_poll());
        m.input(1, -1, 0);
        assert_eq!(m.read(), 0x80 | 0x08 | 0x20, "right and up");
    }

    #[test]
    fn button_changes_request_an_event() {
        let mut m = Mouse::new();
        m.input(0, 0, BUTTON_LEFT);
        assert!(m.do_poll());
        assert_eq!(m.read(), 0x82);
        assert!(!m.do_poll());
        m.input(0, 0, 0);
        assert!(m.do_poll(), "release is an event too");
        assert_eq!(m.read(), 0x80);
        m.input(0, 0, BUTTON_RIGHT);
        assert_eq!(m.read(), 0x81);
    }

    #[test]
    fn steps_are_lost_while_an_event_is_unread() {
        let mut m = Mouse::new();
        m.input(10, 0, 0);
        assert!(m.do_poll());
        for _ in 0..4 {
            m.do_poll(); // nobody reads
        }
        let reads = (0..20).take_while(|_| m.read() & 0x08 != 0).count();
        assert_eq!(reads, 6, "4 of 10 steps lost");
    }

    #[test]
    fn backlog_is_capped_and_polls_are_600_hz() {
        let mut m = Mouse::new();
        m.input(1000, 0, 0);
        let reads = (0..200).take_while(|_| m.read() & 0x08 != 0).count();
        assert_eq!(reads, MAX_BACKLOG as usize);
        m.do_poll();
        m.do_poll();
        assert_eq!(m.next_poll(), 2 * CPU_HZ / 600);
    }
}

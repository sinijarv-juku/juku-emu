//! Intel 8253 programmable interval timer (КР580ВИ53).
//!
//! Each counter is advanced one clock pulse at a time by [`Pit::clock`]; the
//! machine decides when pulses happen (fixed clocks, or edges of another
//! counter's OUT). Every call that can change an OUT pin returns whether it
//! did, so the machine can propagate cascaded clock and gate connections.
//!
//! Most pulses only count down. [`Pit::quiet`] tells how many of the coming
//! pulses are like that and [`Pit::skip`] applies them in one step, so a
//! counter on a fixed clock needs a [`Pit::clock`] call only for the pulses
//! on which something happens. [`Pit::run`] does all of it for a counter
//! whose OUT pin nobody watches.
//!
//! All six modes, binary and BCD counting, latch and the LSB/MSB/word access
//! sequences are implemented.

use crate::snapshot::{Reader, Writer};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// No count written since the control word.
    Idle,
    /// Mode 1/5: count available, waiting for a gate trigger (or finished).
    Waiting,
    /// The count register is copied into the counting element on the next clock.
    Load,
    Counting,
    /// Mode 4: strobe done; the counter keeps decrementing with no effect on OUT.
    Done,
}

/// [`Pit::quiet`] for a counter that needs no clocking at all as it is.
pub const FOREVER: u64 = u64::MAX;

/// True if every digit of `v` is 0–9.
fn bcd_valid(v: u16) -> bool {
    (0..4).all(|i| (v >> (4 * i)) & 0xF <= 9)
}

/// A counter as the debugger shows it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct CounterState {
    pub mode: u8,
    pub bcd: bool,
    /// The count written, as a number (BCD decoded, 0 = maximum), if any.
    pub count: Option<u32>,
    /// The counting element, as it counts (BCD digits in BCD mode).
    pub value: u16,
    pub out: bool,
    pub gate: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Counter {
    mode: u8,
    /// Access mode: 1 = LSB, 2 = MSB, 3 = LSB then MSB.
    rw: u8,
    bcd: bool,
    /// Count register, as written (BCD digits in BCD mode).
    cr: u16,
    /// Counting element.
    ce: u16,
    latch: Option<u16>,
    write_lo: Option<u8>,
    read_hi: bool,
    gate: bool,
    out: bool,
    phase: Phase,
    has_count: bool,
    /// Gate rising edge seen; acted upon at the next clock.
    trigger: bool,
    /// Mode 4/5: OUT is low for this one clock and goes high at the next.
    strobe: bool,
    /// Mode 3: the first clock after a (re)load, which handles odd counts.
    first: bool,
}

impl Counter {
    fn new() -> Counter {
        Counter {
            mode: 0,
            rw: 3,
            bcd: false,
            cr: 0,
            ce: 0,
            latch: None,
            write_lo: None,
            read_hi: false,
            gate: true,
            out: true,
            phase: Phase::Idle,
            has_count: false,
            trigger: false,
            strobe: false,
            first: false,
        }
    }

    /// Value of a count, with 0 meaning the maximum (10000 or 65536).
    fn value(&self, v: u16) -> u32 {
        if self.bcd {
            let d = |s: u32| ((v as u32) >> s) & 0xF;
            let n = d(12) * 1000 + d(8) * 100 + d(4) * 10 + d(0);
            if n == 0 { 10000 } else { n }
        } else if v == 0 {
            0x10000
        } else {
            v as u32
        }
    }

    fn dec(&self, v: u16) -> u16 {
        if !self.bcd {
            return v.wrapping_sub(1);
        }
        let mut v = v;
        for shift in [0, 4, 8, 12] {
            if (v >> shift) & 0xF != 0 {
                return v - (1 << shift);
            }
            v |= 9 << shift;
        }
        v
    }

    /// Pulses until plain decrementing brings the counting element to zero,
    /// or `None` if it holds BCD digits above 9, which do not count evenly.
    fn left(&self) -> Option<u32> {
        (!self.bcd || bcd_valid(self.ce)).then(|| self.value(self.ce))
    }

    /// `v` after `n` decrements, wrapping through zero. In BCD mode `v` must
    /// hold valid digits.
    fn minus(&self, v: u16, n: u64) -> u16 {
        if !self.bcd {
            return v.wrapping_sub(n as u16);
        }
        let d = |s: u32| ((v as u64) >> s) & 0xF;
        let bin = d(12) * 1000 + d(8) * 100 + d(4) * 10 + d(0);
        let r = (bin + 10000 - n % 10000) % 10000;
        (r / 1000 << 12 | r / 100 % 10 << 8 | r / 10 % 10 << 4 | r % 10) as u16
    }

    /// The coming pulses that only count down: how many there are and how
    /// much each takes off the counting element. None of them changes OUT or
    /// any other state.
    fn coast(&self) -> (u64, u16) {
        const NEVER: (u64, u16) = (FOREVER, 0);
        const NOW: (u64, u16) = (0, 0);
        if self.phase == Phase::Idle {
            return NEVER;
        }
        // Counting down to an event on the pulse that reaches `stop`.
        let until = |stop: u32| self.left().map_or(NOW, |n| (n.saturating_sub(stop + 1) as u64, 1));
        // Counting on past zero with no effect.
        let free = || if self.left().is_some() { (FOREVER, 1) } else { NOW };
        match self.mode {
            0 => match self.phase {
                Phase::Load => NOW,
                Phase::Counting if self.gate => {
                    if self.out {
                        free()
                    } else {
                        until(0)
                    }
                }
                _ => NEVER,
            },
            1 | 5 => {
                if self.strobe || self.trigger {
                    NOW
                } else if self.phase == Phase::Counting {
                    until(0)
                } else {
                    NEVER
                }
            }
            2 => {
                if !self.gate {
                    NEVER
                } else if self.trigger || self.phase == Phase::Load {
                    NOW
                } else {
                    until(1)
                }
            }
            3 => {
                if !self.gate {
                    NEVER
                } else if self.trigger || self.phase == Phase::Load || self.first {
                    NOW
                } else {
                    // Two off per pulse until two or fewer are left.
                    self.left().map_or(NOW, |n| ((n.saturating_sub(1) / 2) as u64, 2))
                }
            }
            _ => {
                // Mode 4.
                if self.strobe {
                    return NOW;
                }
                match self.phase {
                    Phase::Load => NOW,
                    Phase::Counting if self.gate => until(0),
                    Phase::Done if self.gate => free(),
                    _ => NEVER,
                }
            }
        }
    }

    /// How many of the coming pulses do nothing but count down ([`FOREVER`]
    /// if that never ends). The pulse after them needs [`Counter::clock`].
    fn quiet(&self) -> u64 {
        self.coast().0
    }

    /// Applies `n` pulses at once; `n` must not exceed [`Counter::quiet`].
    fn skip(&mut self, n: u64) {
        let (quiet, step) = self.coast();
        debug_assert!(n <= quiet);
        if step != 0 && n != 0 {
            self.ce = self.minus(self.ce, n * step as u64);
        }
    }

    /// In modes 2 and 3, right after a reload with OUT high: the number of
    /// pulses after which the counter is in this same state again.
    fn period(&self) -> Option<u64> {
        let reloaded = self.gate && self.phase == Phase::Counting && !self.trigger && self.out && self.ce == self.cr;
        if !reloaded || (self.bcd && !bcd_valid(self.cr)) {
            return None;
        }
        match self.mode {
            2 => Some(self.value(self.cr) as u64),
            // A count of 1 toggles on every pulse.
            3 if self.first => Some(self.value(self.cr).max(2) as u64),
            _ => None,
        }
    }

    /// Applies `n` pulses, for a counter whose OUT nobody watches. The work
    /// does not grow with `n`: whole periods of a free-running counter are
    /// left out.
    fn run(&mut self, mut n: u64) {
        loop {
            let quiet = self.quiet();
            if n <= quiet {
                self.skip(n);
                return;
            }
            self.skip(quiet);
            self.clock();
            n -= quiet + 1;
            if let Some(period) = self.period() {
                n %= period;
            }
        }
    }

    fn control(&mut self, cw: u8) {
        if cw & 0x30 == 0 {
            if self.latch.is_none() {
                self.latch = Some(self.ce);
            }
            return;
        }
        self.rw = (cw >> 4) & 3;
        self.mode = match (cw >> 1) & 7 {
            6 => 2,
            7 => 3,
            m => m,
        };
        self.bcd = cw & 1 != 0;
        self.phase = Phase::Idle;
        self.has_count = false;
        self.latch = None;
        self.write_lo = None;
        self.read_hi = false;
        self.trigger = false;
        self.strobe = false;
        self.out = self.mode != 0;
    }

    fn write(&mut self, v: u8) {
        match self.rw {
            1 => self.cr = v as u16,
            2 => self.cr = (v as u16) << 8,
            _ => match self.write_lo.take() {
                None => {
                    self.write_lo = Some(v);
                    if self.mode == 0 {
                        // Writing the first byte stops the count.
                        self.phase = Phase::Idle;
                        self.out = false;
                    }
                    return;
                }
                Some(lo) => self.cr = lo as u16 | (v as u16) << 8,
            },
        }
        self.has_count = true;
        match self.mode {
            0 => {
                self.out = false;
                self.phase = Phase::Load;
            }
            1 | 5 => {
                if self.phase == Phase::Idle {
                    self.phase = Phase::Waiting;
                }
            }
            2 | 3 => {
                if matches!(self.phase, Phase::Idle | Phase::Waiting) {
                    self.phase = Phase::Load;
                }
            }
            _ => self.phase = Phase::Load,
        }
    }

    fn read(&mut self) -> u8 {
        let v = self.latch.unwrap_or(self.ce);
        match self.rw {
            1 => {
                self.latch = None;
                v as u8
            }
            2 => {
                self.latch = None;
                (v >> 8) as u8
            }
            _ => {
                if self.read_hi {
                    self.read_hi = false;
                    self.latch = None;
                    (v >> 8) as u8
                } else {
                    self.read_hi = true;
                    v as u8
                }
            }
        }
    }

    fn set_gate(&mut self, level: bool) -> bool {
        let old = self.out;
        let rising = level && !self.gate;
        self.gate = level;
        match self.mode {
            1 | 5 if rising && self.has_count => self.trigger = true,
            2 | 3 => {
                if !level {
                    self.out = true;
                } else if rising && self.has_count {
                    self.trigger = true;
                }
            }
            _ => {}
        }
        self.out != old
    }

    /// One CLK pulse. Returns true if OUT changed.
    fn clock(&mut self) -> bool {
        if self.phase == Phase::Idle {
            return false;
        }
        let old = self.out;
        match self.mode {
            0 => match self.phase {
                Phase::Load => {
                    self.ce = self.cr;
                    self.phase = Phase::Counting;
                }
                Phase::Counting if self.gate => {
                    self.ce = self.dec(self.ce);
                    if self.ce == 0 {
                        self.out = true;
                    }
                }
                _ => {}
            },
            1 | 5 => {
                if self.strobe {
                    self.out = true;
                    self.strobe = false;
                }
                if self.trigger {
                    self.trigger = false;
                    self.ce = self.cr;
                    self.phase = Phase::Counting;
                    if self.mode == 1 {
                        self.out = false;
                    }
                } else if self.phase == Phase::Counting {
                    self.ce = self.dec(self.ce);
                    if self.ce == 0 {
                        self.phase = Phase::Waiting;
                        if self.mode == 1 {
                            self.out = true;
                        } else {
                            self.out = false;
                            self.strobe = true;
                        }
                    }
                }
            }
            2 => {
                if self.gate {
                    if self.trigger || self.phase == Phase::Load {
                        self.trigger = false;
                        self.ce = self.cr;
                        self.phase = Phase::Counting;
                        self.out = true;
                    } else {
                        self.ce = self.dec(self.ce);
                        if self.ce == 1 {
                            self.out = false;
                        } else if self.ce == 0 {
                            self.ce = self.cr;
                            self.out = true;
                        }
                    }
                }
            }
            3 => {
                if self.gate {
                    if self.trigger || self.phase == Phase::Load {
                        self.trigger = false;
                        self.ce = self.cr;
                        self.phase = Phase::Counting;
                        self.out = true;
                        self.first = true;
                    } else {
                        let odd = self.value(self.cr) & 1 == 1;
                        let step = match (self.first && odd, self.out) {
                            (true, true) => 1,
                            (true, false) => 3,
                            _ => 2,
                        };
                        self.first = false;
                        if self.value(self.ce) <= step {
                            self.out = !self.out;
                            self.ce = self.cr;
                            self.first = true;
                        } else {
                            for _ in 0..step {
                                self.ce = self.dec(self.ce);
                            }
                        }
                    }
                }
            }
            _ => {
                // Mode 4.
                if self.strobe {
                    self.out = true;
                    self.strobe = false;
                }
                match self.phase {
                    Phase::Load => {
                        self.ce = self.cr;
                        self.phase = Phase::Counting;
                    }
                    Phase::Counting | Phase::Done if self.gate => {
                        self.ce = self.dec(self.ce);
                        if self.ce == 0 && self.phase == Phase::Counting {
                            self.out = false;
                            self.strobe = true;
                            self.phase = Phase::Done;
                        }
                    }
                    _ => {}
                }
            }
        }
        self.out != old
    }
}

/// Three counters behind one 4-register interface.
#[derive(Clone, Debug)]
pub struct Pit {
    ch: [Counter; 3],
}

impl Default for Pit {
    fn default() -> Pit {
        Pit::new()
    }
}

impl Pit {
    pub fn new() -> Pit {
        Pit { ch: [Counter::new(), Counter::new(), Counter::new()] }
    }

    /// Register write: 0–2 counters, 3 control word.
    pub fn write(&mut self, reg: u8, v: u8) {
        match reg & 3 {
            3 => {
                let sel = (v >> 6) as usize;
                if sel < 3 {
                    self.ch[sel].control(v);
                }
            }
            n => self.ch[n as usize].write(v),
        }
    }

    pub fn read(&mut self, reg: u8) -> u8 {
        match reg & 3 {
            3 => 0xFF,
            n => self.ch[n as usize].read(),
        }
    }

    /// One clock pulse on counter `ch`. Returns true if its OUT changed.
    #[inline]
    pub fn clock(&mut self, ch: usize) -> bool {
        self.ch[ch].clock()
    }

    /// Sets the GATE input of counter `ch`. Returns true if its OUT changed.
    pub fn set_gate(&mut self, ch: usize, level: bool) -> bool {
        self.ch[ch].set_gate(level)
    }

    pub fn out(&self, ch: usize) -> bool {
        self.ch[ch].out
    }

    /// OUT pins as a bit mask (bit n = counter n).
    pub fn outs(&self) -> u8 {
        self.ch.iter().enumerate().fold(0, |m, (i, c)| m | (c.out as u8) << i)
    }

    /// How many of the coming pulses on counter `ch` only count down,
    /// changing neither OUT nor anything else ([`FOREVER`] if that never
    /// ends). The pulse after them needs [`Pit::clock`].
    pub fn quiet(&self, ch: usize) -> u64 {
        self.ch[ch].quiet()
    }

    /// Applies `n` pulses to counter `ch` at once. `n` must not exceed
    /// [`Pit::quiet`].
    pub fn skip(&mut self, ch: usize, n: u64) {
        self.ch[ch].skip(n);
    }

    /// Applies `n` pulses to counter `ch` without reporting OUT changes, in
    /// time that does not grow with `n`.
    pub fn run(&mut self, ch: usize, n: u64) {
        self.ch[ch].run(n);
    }

    /// The programmed count of counter `ch` as a number (BCD decoded, 0 =
    /// maximum), or `None` if no count was written since its control word.
    pub fn count(&self, ch: usize) -> Option<u32> {
        let c = &self.ch[ch];
        c.has_count.then(|| c.value(c.cr))
    }

    /// Counter `ch`, for the debugger.
    pub fn state(&self, ch: usize) -> CounterState {
        let c = &self.ch[ch];
        CounterState { mode: c.mode, bcd: c.bcd, count: self.count(ch), value: c.ce, out: c.out, gate: c.gate }
    }
}

/// Snapshot state (spec §4.6).
impl Counter {
    fn save(&self, w: &mut Writer) {
        w.u8(self.mode);
        w.u8(self.rw);
        w.bool(self.bcd);
        w.u16(self.cr);
        w.u16(self.ce);
        w.opt_u16(self.latch);
        w.opt_u8(self.write_lo);
        w.bool(self.read_hi);
        w.bool(self.gate);
        w.bool(self.out);
        w.u8(self.phase as u8);
        w.bool(self.has_count);
        w.bool(self.trigger);
        w.bool(self.strobe);
        w.bool(self.first);
    }

    fn load(r: &mut Reader) -> Option<Counter> {
        let mode = r.u8().filter(|&m| m <= 5)?;
        let rw = r.u8().filter(|rw| (1..=3).contains(rw))?;
        let bcd = r.bool()?;
        let (cr, ce) = (r.u16()?, r.u16()?);
        let latch = r.opt_u16()?;
        let write_lo = r.opt_u8()?;
        let (read_hi, gate, out) = (r.bool()?, r.bool()?, r.bool()?);
        let phase = match r.u8()? {
            0 => Phase::Idle,
            1 => Phase::Waiting,
            2 => Phase::Load,
            3 => Phase::Counting,
            4 => Phase::Done,
            _ => return None,
        };
        let (has_count, trigger, strobe, first) = (r.bool()?, r.bool()?, r.bool()?, r.bool()?);
        Some(Counter { mode, rw, bcd, cr, ce, latch, write_lo, read_hi, gate, out, phase, has_count, trigger, strobe, first })
    }
}

impl Pit {
    pub(crate) fn save(&self, w: &mut Writer) {
        for c in &self.ch {
            c.save(w);
        }
    }

    pub(crate) fn load(r: &mut Reader) -> Option<Pit> {
        Some(Pit { ch: [Counter::load(r)?, Counter::load(r)?, Counter::load(r)?] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clocks `ch` `n` times and returns the OUT level after each pulse.
    fn trace(p: &mut Pit, ch: usize, n: usize) -> String {
        (0..n)
            .map(|_| {
                p.clock(ch);
                if p.out(ch) { '1' } else { '0' }
            })
            .collect()
    }

    /// A copy of `p` through a snapshot.
    fn snapshot_copy(p: &Pit) -> Pit {
        let mut w = Writer::new();
        p.save(&mut w);
        let bytes = w.into_bytes();
        let mut r = Reader::new(&bytes);
        let q = Pit::load(&mut r).unwrap();
        r.end().unwrap();
        q
    }

    #[test]
    fn snapshot_round_trip_in_every_mode() {
        for mode in 0..6u8 {
            for bcd in [false, true] {
                let mut p = Pit::new();
                p.write(3, 0x30 | mode << 1 | bcd as u8);
                p.write(0, 0x07);
                p.write(0, 0x00);
                // A gate trigger, for modes 1 and 5.
                p.set_gate(0, false);
                p.clock(0);
                p.set_gate(0, true);
                for _ in 0..20 {
                    let mut q = snapshot_copy(&p);
                    assert_eq!(q.ch, p.ch, "mode {mode}, bcd {bcd}");
                    assert_eq!(trace(&mut q, 0, 25), trace(&mut p.clone(), 0, 25));
                    p.clock(0);
                }
                // A latch and half a word read.
                p.write(3, 0x00);
                p.read(0);
                assert_eq!(snapshot_copy(&p).ch, p.ch);
            }
        }
        let mut w = Writer::new();
        Pit::new().save(&mut w);
        let mut bytes = w.into_bytes();
        bytes[0] = 6;
        assert!(Pit::load(&mut Reader::new(&bytes)).is_none(), "no mode 6");
    }

    #[test]
    fn mode0_interrupt_on_terminal_count() {
        let mut p = Pit::new();
        p.write(3, 0x10); // ch0, LSB, mode 0
        assert!(!p.out(0));
        p.write(0, 3);
        // load, 2, 1, 0 (high), wraps and stays high
        assert_eq!(trace(&mut p, 0, 6), "000111");
    }

    #[test]
    fn mode2_rate_generator_period() {
        let mut p = Pit::new();
        p.write(3, 0x14); // ch0, LSB, mode 2
        p.write(0, 4);
        // load, 3, 2, 1 (low), reload (high) ...
        assert_eq!(trace(&mut p, 0, 9), "111011101");
    }

    #[test]
    fn mode3_square_wave_odd_count() {
        let mut p = Pit::new();
        p.write(3, 0x16); // ch0, LSB, mode 3
        p.write(0, 5);
        p.clock(0); // load
        // high for 3 clocks, low for 2
        assert_eq!(trace(&mut p, 0, 10), "1100111001");
    }

    #[test]
    fn mode1_one_shot_retriggered_by_gate() {
        let mut p = Pit::new();
        p.write(3, 0x12); // ch0, LSB, mode 1
        p.write(0, 3);
        p.set_gate(0, false);
        assert_eq!(trace(&mut p, 0, 2), "11");
        p.set_gate(0, true);
        assert_eq!(trace(&mut p, 0, 5), "00011");
    }

    #[test]
    fn mode4_strobe() {
        let mut p = Pit::new();
        p.write(3, 0x18); // ch0, LSB, mode 4
        p.write(0, 2);
        assert_eq!(trace(&mut p, 0, 6), "110111");
    }

    #[test]
    fn bcd_counts_and_wraps() {
        let mut p = Pit::new();
        p.write(3, 0x31); // ch0, word, mode 0, BCD
        p.write(0, 0x00);
        p.write(0, 0x01); // 0100 BCD = 100
        assert_eq!(p.count(0), Some(100));
        p.clock(0); // load
        p.clock(0);
        p.write(3, 0x00); // latch
        assert_eq!((p.read(0), p.read(0)), (0x99, 0x00));
        for _ in 0..99 {
            p.clock(0);
        }
        assert!(p.out(0));
        p.clock(0); // 0000 → 9999
        p.write(3, 0x00);
        assert_eq!((p.read(0), p.read(0)), (0x99, 0x99));
    }

    #[test]
    fn quiet_pulses_up_to_the_next_out_change() {
        let mut p = Pit::new();
        assert_eq!(p.quiet(0), FOREVER, "idle");
        p.write(3, 0x15); // ch0, LSB, mode 2, BCD
        p.write(0, 0x64);
        assert_eq!(p.quiet(0), 0, "load");
        p.clock(0);
        // 64 → 2 in 62 pulses; the next one makes it 1 and OUT low.
        assert_eq!(p.quiet(0), 62);
        p.skip(0, 62);
        assert!(p.out(0));
        assert!(p.clock(0));
        assert_eq!(p.quiet(0), 0);
        assert!(p.clock(0), "reload");
        assert_eq!(p.quiet(0), 62);

        p.write(3, 0x52); // ch1, LSB, mode 1
        p.write(1, 24);
        assert_eq!(p.quiet(1), FOREVER, "waiting for a trigger");
        p.set_gate(1, false);
        p.set_gate(1, true);
        assert_eq!(p.quiet(1), 0);
        assert!(p.clock(1));
        assert_eq!(p.quiet(1), 23);
    }

    #[test]
    fn run_leaves_out_whole_periods() {
        let mut a = Pit::new();
        a.write(3, 0x1F); // ch0, LSB, mode 3, BCD: the RomBios baud rate setup
        a.write(0, 0x32);
        let mut b = a.clone();
        for _ in 0..1_000_003 {
            a.clock(0);
        }
        b.run(0, 1_000_003);
        assert_eq!(a.ch[0], b.ch[0]);
    }

    /// xorshift64.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// Random programming, gate changes and reads, with random stretches of
    /// pulses in between. Three counters get the same treatment: one is
    /// clocked pulse by pulse, one with `quiet`, `skip` and `clock`, and one
    /// with `run`. They must agree in every field, and the second must see
    /// OUT change on exactly the same pulses as the first.
    #[test]
    fn bulk_stepping_matches_single_pulses() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for round in 0..1000 {
            let mut single = Counter::new();
            let mut events = Counter::new();
            let mut bulk = Counter::new();
            for op in 0..40 {
                let what = rng.below(10);
                let v = match rng.below(4) {
                    0 => rng.below(4) as u8,
                    1 => (rng.below(10) << 4 | rng.below(10)) as u8,
                    _ => rng.next() as u8,
                };
                for c in [&mut single, &mut events, &mut bulk] {
                    match what {
                        0 => c.control(v | 0x10),
                        1 => c.control(v & 0x0F),
                        2 | 3 => c.write(v),
                        4 => {
                            c.set_gate(v & 1 != 0);
                        }
                        5 => {
                            c.read();
                        }
                        _ => {}
                    }
                }
                let n = match rng.below(6) {
                    0 => rng.below(4),
                    1 | 2 => rng.below(300),
                    3 | 4 => rng.below(12_000),
                    _ => rng.below(150_000),
                };

                let mut changes = Vec::new();
                for i in 0..n {
                    if single.clock() {
                        changes.push(i);
                    }
                }

                let mut seen = Vec::new();
                let mut i = 0;
                while i < n {
                    let quiet = events.quiet().min(n - i);
                    events.skip(quiet);
                    i += quiet;
                    if i < n {
                        if events.clock() {
                            seen.push(i);
                        }
                        i += 1;
                    }
                }

                bulk.run(n);

                assert_eq!(changes, seen, "round {round} op {op}: OUT changes over {n} pulses");
                assert_eq!(single, events, "round {round} op {op}: events, {n} pulses");
                assert_eq!(single, bulk, "round {round} op {op}: run, {n} pulses");
            }
        }
    }

    #[test]
    fn latch_holds_value_until_read() {
        let mut p = Pit::new();
        p.write(3, 0x34); // ch0, word, mode 2
        p.write(0, 0x39);
        p.write(0, 0x01);
        p.clock(0);
        p.write(3, 0x00);
        p.clock(0);
        p.clock(0);
        assert_eq!(p.read(0), 0x39);
        assert_eq!(p.read(0), 0x01);
        assert_eq!(p.read(0), 0x37); // unlatched now
    }
}

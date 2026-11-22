//! Speaker → PCM.
//!
//! The speaker level is a step function of emulated time. [`Speaker`] box-
//! filters it into samples at the host rate, shapes them like a small speaker
//! (DC block, gentle low-pass) and pushes them into an [`AudioRing`], which the
//! host's audio thread drains. The ring is a separate, shared heap object, so
//! the audio thread never touches the machine. The ring holds stereo frames:
//! the speaker is in the middle, and the floppy drives' sounds
//! ([`DriveSounds`]), mixed in after the speaker's filters, are panned to
//! where each drive was.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::drive_sound::{DriveEvent, DriveSounds};
use crate::machine::CPU_HZ;

const RING_SIZE: usize = 8192;

/// Single-producer single-consumer ring of stereo frames, each a left and a
/// right f32 in one atomic word.
pub struct AudioRing {
    buf: Box<[AtomicU64]>,
    /// Total frames written (producer) and read (consumer).
    head: AtomicUsize,
    tail: AtomicUsize,
    /// Consumer side: false after an underrun, until `prime` frames are queued.
    primed: AtomicBool,
    prime: usize,
    sample_rate: u32,
}

impl AudioRing {
    /// A ring for `sample_rate` Hz. After an underrun, output resumes once
    /// `prime` frames are buffered, which sets the steady-state latency.
    pub fn new(sample_rate: u32, prime: usize) -> AudioRing {
        AudioRing {
            buf: (0..RING_SIZE).map(|_| AtomicU64::new(0)).collect(),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            primed: AtomicBool::new(false),
            prime: prime.min(RING_SIZE / 2),
            sample_rate,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Producer: appends a frame; drops it if the ring is full.
    pub fn push(&self, left: f32, right: f32) {
        let h = self.head.load(Ordering::Relaxed);
        let t = self.tail.load(Ordering::Acquire);
        if h - t < RING_SIZE {
            let frame = left.to_bits() as u64 | (right.to_bits() as u64) << 32;
            self.buf[h % RING_SIZE].store(frame, Ordering::Relaxed);
            self.head.store(h + 1, Ordering::Release);
        }
    }

    /// Consumer: fills `left` and `right`, as many frames as the shorter
    /// holds, padding with silence on underrun. Returns the number of real
    /// frames delivered.
    pub fn pull(&self, left: &mut [f32], right: &mut [f32]) -> usize {
        let len = left.len().min(right.len());
        let (left, right) = (&mut left[..len], &mut right[..len]);
        let t = self.tail.load(Ordering::Relaxed);
        let avail = self.head.load(Ordering::Acquire) - t;
        if !self.primed.load(Ordering::Relaxed) {
            if avail < self.prime {
                left.fill(0.0);
                right.fill(0.0);
                return 0;
            }
            self.primed.store(true, Ordering::Relaxed);
        }
        let n = avail.min(len);
        for i in 0..n {
            let frame = self.buf[(t + i) % RING_SIZE].load(Ordering::Relaxed);
            left[i] = f32::from_bits(frame as u32);
            right[i] = f32::from_bits((frame >> 32) as u32);
        }
        self.tail.store(t + n, Ordering::Release);
        if n < len {
            left[n..].fill(0.0);
            right[n..].fill(0.0);
            self.primed.store(false, Ordering::Relaxed);
        }
        n
    }

    /// Frames queued.
    pub fn fill(&self) -> usize {
        self.head.load(Ordering::Acquire) - self.tail.load(Ordering::Acquire)
    }
}

/// Output gain applied after filtering.
const GAIN: f32 = 0.5;

pub struct Speaker {
    ring: Option<Arc<AudioRing>>,
    /// Samples are computed but not queued (warp speed).
    muted: bool,
    rate: u64,
    level: f32,
    /// Times in units of 1/(CPU_HZ·rate) s: one CPU cycle = `rate` units,
    /// one sample = CPU_HZ units.
    last: u64,
    sample_end: u64,
    acc: f64,
    // Filters: DC-blocking high-pass, then a 1-pole low-pass.
    hp_x: f32,
    hp_y: f32,
    hp_r: f32,
    lp_y: f32,
    lp_a: f32,
    drives: DriveSounds,
}

impl Speaker {
    pub fn new(ring: Option<Arc<AudioRing>>) -> Speaker {
        let rate = ring.as_ref().map_or(48_000, |r| r.sample_rate().max(1)) as u64;
        let fs = rate as f32;
        Speaker {
            ring,
            muted: false,
            rate,
            level: 0.0,
            last: 0,
            sample_end: CPU_HZ,
            acc: 0.0,
            hp_x: 0.0,
            hp_y: 0.0,
            hp_r: 1.0 - 2.0 * std::f32::consts::PI * 20.0 / fs,
            lp_y: 0.0,
            lp_a: 1.0 - (-2.0 * std::f32::consts::PI * 8000.0 / fs).exp(),
            drives: DriveSounds::new(fs),
        }
    }

    /// While muted the filters keep running, so unmuting does not click,
    /// but nothing is queued: the ring drains and re-primes afterwards.
    /// The drive sounds stop where they are, so as not to play on after.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        if muted {
            self.drives.settle();
        }
    }

    pub fn drive_sounds(&self) -> bool {
        self.drives.on()
    }

    pub fn set_drive_sounds(&mut self, on: bool) {
        self.drives.set_on(on);
    }

    pub fn set_drive_volume(&mut self, volume: f32) {
        self.drives.set_volume(volume);
    }

    pub fn set_drive_idle_mute(&mut self, on: bool) {
        self.drives.set_idle_mute(on);
    }

    /// A drive did something at CPU cycle `t`. It is heard from then, or
    /// from now if the speaker is past it; with `play` false, or with the
    /// sounds off or muted, it only changes the drives' state.
    pub fn drive_event(&mut self, t: u64, event: DriveEvent, play: bool) {
        self.advance(t);
        let play = play && self.drives.on() && !self.muted;
        self.drives.event(event, play);
    }

    /// The drives are as given, with nothing playing (a snapshot was restored).
    pub fn sync_drives(&mut self, motor: bool, disks: [bool; 2]) {
        self.drives.sync(motor, disks);
    }

    /// Moves the speaker's clock to CPU cycle `t`, which may be earlier than
    /// its own (a snapshot was restored), as if the level had been the same
    /// since the start of the current sample. The filters carry on, so the
    /// output does not click.
    pub fn restart(&mut self, t: u64) {
        self.last = t * self.rate;
        self.sample_end = (self.last / CPU_HZ + 1) * CPU_HZ;
        self.acc = self.level as f64 * (self.last + CPU_HZ - self.sample_end) as f64;
    }

    /// Changes the level at CPU cycle `t`.
    pub fn set_level(&mut self, t: u64, level: f32) {
        if level != self.level {
            self.advance(t);
            self.level = level;
        }
    }

    /// Emits all samples that end at or before CPU cycle `t`.
    pub fn advance(&mut self, t: u64) {
        if self.ring.is_none() {
            return;
        }
        let now = t * self.rate;
        if now <= self.last {
            return;
        }
        let level = self.level as f64;
        while self.sample_end <= now {
            self.acc += level * (self.sample_end - self.last) as f64;
            let s = (self.acc / CPU_HZ as f64) as f32;
            self.emit(s);
            self.acc = 0.0;
            self.last = self.sample_end;
            self.sample_end += CPU_HZ;
        }
        self.acc += level * (now - self.last) as f64;
        self.last = now;
    }

    fn emit(&mut self, x: f32) {
        let y = x - self.hp_x + self.hp_r * self.hp_y;
        self.hp_x = x;
        self.hp_y = y;
        self.lp_y += self.lp_a * (y - self.lp_y);
        if let Some(ring) = &self.ring
            && !self.muted
        {
            let speaker = self.lp_y * GAIN;
            let (left, right) = self.drives.sample();
            ring.push(speaker + left, speaker + right);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_primes_and_pads_underrun() {
        let r = AudioRing::new(48_000, 4);
        let (mut left, mut right) = ([9.0f32; 3], [9.0f32; 3]);
        r.push(1.0, -1.0);
        assert_eq!(r.pull(&mut left, &mut right), 0, "not primed yet");
        assert_eq!((left, right), ([0.0; 3], [0.0; 3]));
        for _ in 0..4 {
            r.push(2.0, -2.0);
        }
        assert_eq!(r.pull(&mut left, &mut right), 3);
        assert_eq!((left, right), ([1.0, 2.0, 2.0], [-1.0, -2.0, -2.0]));
        assert_eq!(r.pull(&mut left, &mut right), 2);
        assert_eq!((left, right), ([2.0, 2.0, 0.0], [-2.0, -2.0, 0.0]));
        r.push(3.0, -3.0);
        assert_eq!(r.pull(&mut left, &mut right), 0, "re-primes after underrun");
    }

    #[test]
    fn box_filter_averages_within_a_sample() {
        let ring = Arc::new(AudioRing::new(48_000, 0));
        let mut s = Speaker::new(Some(ring.clone()));
        s.hp_r = 1.0; // pass DC for this test
        s.lp_a = 1.0;
        // One sample is 41⅔ cycles: high for exactly the first 3 samples.
        s.set_level(0, 1.0);
        s.set_level(125, 0.0);
        s.advance(250);
        let (mut left, mut right) = ([0.0f32; 6], [0.0f32; 6]);
        assert_eq!(ring.pull(&mut left, &mut right), 6);
        assert_eq!(left, right, "the speaker is in the middle");
        assert_eq!(left[0], GAIN);
        assert_eq!(left[2], GAIN);
        assert_eq!(left[3], 0.0);
    }

    #[test]
    fn samples_match_elapsed_time() {
        let ring = Arc::new(AudioRing::new(48_000, 0));
        let mut s = Speaker::new(Some(ring.clone()));
        s.advance(CPU_HZ / 10);
        assert_eq!(ring.fill(), 4800);
    }

    #[test]
    fn muted_speaker_queues_nothing() {
        let ring = Arc::new(AudioRing::new(48_000, 0));
        let mut s = Speaker::new(Some(ring.clone()));
        s.set_muted(true);
        s.advance(CPU_HZ / 10);
        assert_eq!(ring.fill(), 0);
        s.set_muted(false);
        s.advance(CPU_HZ / 5);
        assert_eq!(ring.fill(), 4800, "time muted is skipped, not queued later");
    }
}

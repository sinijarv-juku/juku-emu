//! The floppy drives' sounds: the spindle motor, the head stepping, and the
//! disk going in and out with its lever.
//!
//! The sounds are made from noise as they play, with no samples, after
//! recordings of a real 5.25" drive (spec §4.5). The noise goes through 21
//! third-octave band-passes, and each sound is a level per band over time:
//! a knock rises and then dies away band by band, a rub (a disk sliding)
//! follows an uneven envelope. Loud knocks also ring the drive frame's
//! resonances. The sounds' numbers were fitted to the recordings and are in
//! [`fitted`]. The speaker mixes the result in after its own filters, since
//! the drives are not heard through it.

mod fitted;

use std::f32::consts::{PI, SQRT_2, TAU};

use fitted::{CENTERS, EJECT, HUM, INSERT, LEVER, MODES, SPIN, STEP, TURN};

/// What a drive did, for its sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveEvent {
    /// The shared motor line went on or off.
    Motor(bool),
    /// The head of `drive` stepped. `phase` is the stepper phase it went to
    /// (the cylinder mod 4); `stop`: the head was against its stop and did
    /// not move.
    Step { drive: u8, phase: u8, stop: bool },
    /// A disk went into `drive`; `swap`: in place of one that comes out first.
    Insert { drive: u8, swap: bool },
    Eject { drive: u8 },
    /// The controller was given a command.
    Access,
}

const BANDS: usize = 21;
/// Each band is two band-pass stages of this Q: a third of an octave wide.
const Q: f32 = 2.9;
/// The recordings' level to the output's, where the speaker's square wave
/// is ±0.5.
const GAIN: f32 = 0.5;
/// The disk's turns per second at 300 rpm. The recorded drive turned at
/// 360 rpm; its once-a-turn swish keeps the emulated drive's pace.
const TURNS_HZ: f32 = 5.0;
/// The spindle's speed changes by this fraction of the way still to go per
/// second: it is up to speed in about a quarter of a second, and its noise
/// dies away in a few tenths, as recorded.
const SPIN_UP: f32 = 1.0 / 0.08;
const SPIN_DOWN: f32 = 1.0 / 0.1;
/// A drive with no disk turns with this much of the noise of one with a disk.
const EMPTY: f32 = 0.2;
/// Steps closer together than this are a seek, where each is quieter than a
/// step alone, and every other one quieter still, as recorded.
const SEEK_SECS: f32 = 0.015;
const SEEK_LEVEL: f32 = 0.5;
const SEEK_ODD: f32 = 0.7;
/// After the disk has slid in, the hand turns the lever down after this
/// pause.
const LEVER_PAUSE: f32 = 0.25;
/// A disk put in where there is one goes in this long after the old one
/// starts out.
const SWAP_SECS: f32 = 0.6;
/// How far apart the drives are heard by default: A: at −`SPREAD` (left)
/// and B: at `SPREAD` (right), on a scale where ±1 is one speaker alone.
/// The drives sat side by side in the disk unit under the monitor, A: the
/// left one, their slots' centres about 8 cm either side of the
/// middle: some 8° off centre for a user 60 cm away, a quarter of the way to
/// stereo speakers at ±30°. The host may set another (`set_spread`), for
/// speakers placed otherwise.
pub const SPREAD: f32 = 0.25;
/// Turning the sounds on or off fades them in or out over this time, and a
/// new volume is reached in about as long.
const FADE_SECS: f32 = 0.02;
/// The most volume, against the recordings' level halved (`GAIN`).
const MAX_VOLUME: f32 = 2.0;
/// With the idle motor muted, the motor is heard this long after the
/// controller's last command, then fades out evenly in dB to −60 dB over
/// `IDLE_FADE`. At the next command it is back in `IDLE_RETURN`.
const IDLE_SECS: f32 = 10.0;
const IDLE_FADE: f32 = 3.0;
const IDLE_RETURN: f32 = 0.2;
/// The fade's end, where the motor is silent.
const IDLE_FLOOR: f32 = 1e-3;
const VOICES: usize = 32;
const TAILS: usize = 8;
/// A tail is let go once every band's power is below this.
const SILENT: f32 = 1e-10;
/// The hum's oscillators are turned to the spindle's speed every this many
/// samples.
const HUM_EVERY: u32 = 64;

/// A knock: noise in each band, rising over `attack` (s) to `gains` (dB,
/// −99 for none) and dying away with time constants `taus` (ms). `modes`
/// are its pushes to the frame's resonances.
struct Knock {
    attack: f32,
    gains: [f32; BANDS],
    taus: [f32; BANDS],
    modes: [f32; MODES.len()],
}

/// A rub: noise in each band at `gains` (dB) under an envelope of `env`
/// (dB, one point every `step` s) made uneven by a random wander of about
/// `rough` dB. `len` is in s.
struct Rub {
    len: f32,
    gains: [f32; BANDS],
    step: f32,
    env: &'static [f32],
    rough: f32,
}

enum Part {
    Knock(&'static Knock),
    Rub(&'static Rub),
}

/// A part of a sound, `at` s from its start.
struct Cue {
    at: f32,
    part: Part,
}

fn level(db: f32) -> f32 {
    if db <= -99.0 { 0.0 } else { 10f32.powf(db / 20.0) }
}

/// Two band-pass stages (RBJ, 0 dB at the centre), scaled so that white
/// noise comes out at about unit RMS.
#[derive(Clone, Copy, Default)]
struct Band {
    b0: f32,
    a1: f32,
    a2: f32,
    norm: f32,
    /// x1, x2, y1, y2 of each stage.
    s: [[f32; 4]; 2],
}

impl Band {
    fn new(fs: f32, f0: f32) -> Band {
        let f0 = f0.min(0.45 * fs);
        let w = TAU * f0 / fs;
        let alpha = w.sin() / (2.0 * Q);
        let a0 = 1.0 + alpha;
        // The noise is uniform in −1..1: variance 1/3.
        let norm = 1.0 / ((1.0 / 3.0) * (PI / 4.0) * (f0 / Q) / (fs / 2.0)).sqrt();
        Band { b0: alpha / a0, a1: -2.0 * w.cos() / a0, a2: (1.0 - alpha) / a0, norm, s: [[0.0; 4]; 2] }
    }

    fn tick(&mut self, mut x: f32) -> f32 {
        for s in &mut self.s {
            let y = self.b0 * (x - s[1]) - self.a1 * s[2] - self.a2 * s[3];
            *s = [x, s[0], y, s[2]];
            x = y;
        }
        x * self.norm
    }
}

/// A resonance of the drive frame: a two-pole filter whose response to a
/// unit impulse peaks at about 1.
#[derive(Clone, Copy, Default)]
struct Mode {
    a1: f32,
    a2: f32,
    g: f32,
    y1: f32,
    y2: f32,
}

impl Mode {
    fn new(fs: f32, (freq, tau): (f32, f32)) -> Mode {
        let w = TAU * freq / fs;
        let r = (-1.0 / (tau * fs)).exp();
        Mode { a1: 2.0 * r * w.cos(), a2: -r * r, g: w.sin(), y1: 0.0, y2: 0.0 }
    }

    fn tick(&mut self, x: f32) -> f32 {
        let y = self.a1 * self.y1 + self.a2 * self.y2 + self.g * x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

#[derive(Clone, Copy)]
enum Sound {
    /// A knock rising over `attack` samples to `gain`. At its peak the
    /// frame's resonances get `modes`, and it goes on as part of its kind's
    /// tail.
    Knock { knock: &'static Knock, attack: u32, gain: [f32; BANDS], modes: [f32; MODES.len()] },
    /// `wander` is the envelope's random wander.
    Rub { rub: &'static Rub, gain: [f32; BANDS], wander: f32 },
    /// The drive's disk is clamped on the hub or let go: its noise starts or
    /// stops.
    Clamp { on: bool },
}

/// The knocks of one kind past their peak, dying away together: their power
/// adds, and falls by the same `decay` per sample.
#[derive(Clone, Copy)]
struct Tail {
    knock: &'static Knock,
    decay: [f32; BANDS],
    power: [f32; BANDS],
}

#[derive(Clone, Copy)]
struct Voice {
    /// The drive it comes from.
    drive: u8,
    /// Samples before it starts.
    wait: u32,
    pos: u32,
    sound: Sound,
}

/// One drive as a source of sound: its noise in bands, its frame's
/// resonances, its knocks' tails and its motor's hum, each tone a turning
/// pointer. The drives have noises of their own, so that two heard at once
/// stay two.
struct Source {
    noise: u32,
    bands: [Band; BANDS],
    modes: [Mode; MODES.len()],
    tails: [Option<Tail>; TAILS],
    hum: [(f32, f32); HUM.len()],
    /// Gains into the left and right channels: those heard, which follow
    /// `target` as the volume does.
    pan: (f32, f32),
    target: (f32, f32),
}

/// The gains into the left and right channels of a source at `position`,
/// from −1 (left) to 1 (right): constant power, with the centre at unit gain
/// in each channel.
fn pan_gains(position: f32) -> (f32, f32) {
    let angle = (position.clamp(-1.0, 1.0) + 1.0) * PI / 4.0;
    (SQRT_2 * angle.cos(), SQRT_2 * angle.sin())
}

/// Where drive `drive` is heard with the drives `spread` apart.
fn position(drive: usize, spread: f32) -> f32 {
    if drive == 0 { -spread } else { spread }
}

impl Source {
    fn new(fs: f32, drive: usize) -> Source {
        let pan = pan_gains(position(drive, SPREAD));
        // B:'s motor hums out of step with A:'s.
        let phase = drive as f32 * 1.3;
        Source {
            noise: [0x1234_5678, 0x9E37_79B9][drive],
            bands: CENTERS.map(|f| Band::new(fs, f)),
            modes: MODES.map(|m| Mode::new(fs, m)),
            tails: [None; TAILS],
            hum: [(phase.cos(), phase.sin()); HUM.len()],
            pan,
            target: pan,
        }
    }

    /// White noise from −1 to 1.
    fn white(&mut self) -> f32 {
        let mut x = self.noise;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.noise = x;
        x as i32 as f32 * (1.0 / 2_147_483_648.0)
    }

    /// A knock at its peak joins the tail of its kind.
    fn join_tail(&mut self, fs: f32, knock: &'static Knock, gain: &[f32; BANDS]) {
        let found = self.tails.iter().position(|t| t.is_some_and(|t| std::ptr::eq(t.knock, knock)));
        let Some(i) = found.or_else(|| self.tails.iter().position(Option::is_none)) else { return };
        let tail = self.tails[i].get_or_insert_with(|| Tail {
            knock,
            decay: knock.taus.map(|t| (-2000.0 / (t * fs)).exp()),
            power: [0.0; BANDS],
        });
        for (p, g) in tail.power.iter_mut().zip(gain) {
            *p += g * g;
        }
    }

    /// The drive's sound for one sample: the tails added to `power`, which
    /// shapes the noise in each band, and the resonances given `push`.
    fn sample(&mut self, mut power: [f32; BANDS], push: [f32; MODES.len()]) -> f32 {
        for slot in &mut self.tails {
            let Some(tail) = slot else { continue };
            let mut loudest = 0.0f32;
            for ((p, t), d) in power.iter_mut().zip(tail.power.iter_mut()).zip(tail.decay.iter()) {
                *p += *t;
                *t *= d;
                loudest = loudest.max(*t);
            }
            if loudest < SILENT {
                *slot = None;
            }
        }

        let mut out = 0.0;
        // A silent drive costs no filtering.
        if power.iter().any(|&p| p > 0.0) {
            let n = self.white();
            for (band, p) in self.bands.iter_mut().zip(power) {
                let x = band.tick(n);
                if p > 0.0 {
                    out += x * p.sqrt();
                }
            }
        }
        for (m, p) in self.modes.iter_mut().zip(push) {
            if p != 0.0 || m.y1.abs() + m.y2.abs() > 1e-9 {
                out += m.tick(p);
            }
        }
        out
    }

    /// Nothing playing.
    fn silence(&mut self) {
        self.tails = [None; TAILS];
        for m in &mut self.modes {
            (m.y1, m.y2) = (0.0, 0.0);
        }
    }
}

pub struct DriveSounds {
    fs: f32,
    on: bool,
    /// 0 to 1, fading towards `on`.
    level: f32,
    /// The volume set, and the one heard, which follows it.
    volume: f32,
    gain: f32,
    /// The motor fades out `IDLE_SECS` after the last command.
    idle_mute: bool,
    /// Samples since the controller's last command.
    since_access: u32,
    /// The motor's sounds' level, 0 to 1, lowered while it is idle.
    idle_level: f32,
    sources: [Source; 2],
    voices: [Option<Voice>; VOICES],
    spin: [f32; BANDS],
    // The motor: its line, the spindle's speed (0 to 1), each drive's disk,
    // clamped or not, with its noise fading in or out, and where each disk
    // is in its turn.
    motor: bool,
    speed: f32,
    clamped: [bool; 2],
    friction: [f32; 2],
    turn: [f32; 2],
    /// The hum's tones' turn per sample, and the samples until it is worked
    /// out again.
    hum_turn: [(f32, f32); HUM.len()],
    hum_count: u32,
    /// Samples since the last step.
    since_step: u32,
}

impl DriveSounds {
    pub fn new(fs: f32) -> DriveSounds {
        DriveSounds {
            fs,
            on: false,
            level: 0.0,
            volume: 1.0,
            gain: 1.0,
            idle_mute: false,
            since_access: 0,
            idle_level: 1.0,
            sources: [Source::new(fs, 0), Source::new(fs, 1)],
            voices: [None; VOICES],
            spin: SPIN.map(level),
            motor: false,
            speed: 0.0,
            clamped: [false; 2],
            friction: [0.0; 2],
            // B:'s disk turns out of step with A:'s.
            turn: [0.0, 0.4],
            hum_turn: [(1.0, 0.0); HUM.len()],
            hum_count: 0,
            since_step: u32::MAX,
        }
    }

    pub fn on(&self) -> bool {
        self.on
    }

    /// Sets the volume, from 0 to `MAX_VOLUME`; 1 is the recordings' level
    /// halved.
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, MAX_VOLUME);
    }

    /// Sets how far apart the drives are heard, from 0 (both in the middle)
    /// to 1 (A: in the left speaker alone, B: in the right); `SPREAD` by
    /// default. A change is reached in about `FADE_SECS`, as a new volume is.
    pub fn set_spread(&mut self, spread: f32) {
        let spread = spread.clamp(0.0, 1.0);
        for (d, source) in self.sources.iter_mut().enumerate() {
            source.target = pan_gains(position(d, spread));
        }
    }

    /// With `on`, the motor fades out once the controller has had no
    /// command for `IDLE_SECS`, and is heard again at the next one. The
    /// spindle keeps turning: only its sound goes.
    pub fn set_idle_mute(&mut self, on: bool) {
        self.idle_mute = on;
    }

    /// The motor has been idle long enough to fade out.
    fn idle(&self) -> bool {
        self.idle_mute && self.since_access as f32 >= IDLE_SECS * self.fs
    }

    /// Turns the sounds on or off; they fade in or out. They start with the
    /// drives as they are now, the spindle at its speed.
    pub fn set_on(&mut self, on: bool) {
        if on && !self.on {
            self.gain = self.volume;
            for source in &mut self.sources {
                source.pan = source.target;
            }
            self.settle();
        }
        self.on = on;
    }

    /// The drives are as given, at once, with nothing playing: after a
    /// snapshot is restored.
    pub fn sync(&mut self, motor: bool, disks: [bool; 2]) {
        self.motor = motor;
        self.since_access = 0;
        self.clamped = disks;
        self.voices = [None; VOICES];
        self.settle();
    }

    /// Takes in what a drive did. With `play` false only the drives' state
    /// changes, at once and without a sound.
    pub fn event(&mut self, event: DriveEvent, play: bool) {
        if let DriveEvent::Motor(on) = event {
            self.motor = on;
        }
        if matches!(event, DriveEvent::Motor(true) | DriveEvent::Access | DriveEvent::Step { .. }) {
            self.since_access = 0;
        }
        match event {
            DriveEvent::Motor(_) | DriveEvent::Access => {
                if !play {
                    self.speed = if self.motor { 1.0 } else { 0.0 };
                    self.idle_level = 1.0;
                }
            }
            _ if !play => {
                if let DriveEvent::Insert { drive, .. } | DriveEvent::Eject { drive } = event {
                    let d = drive as usize & 1;
                    self.clamped[d] = matches!(event, DriveEvent::Insert { .. });
                    self.friction[d] = self.clamped[d] as u8 as f32;
                }
            }
            DriveEvent::Step { drive, phase, stop } => self.step(drive & 1, phase, stop),
            DriveEvent::Insert { drive, swap } => {
                let at = if swap {
                    self.eject(drive & 1, 0.0);
                    SWAP_SECS
                } else {
                    0.0
                };
                self.insert(drive & 1, at);
            }
            DriveEvent::Eject { drive } => self.eject(drive & 1, 0.0),
        }
    }

    /// The spindle at the speed it is heading for, the disks clamped or not,
    /// and nothing playing.
    pub fn settle(&mut self) {
        self.speed = if self.motor { 1.0 } else { 0.0 };
        self.idle_level = if self.idle() { 0.0 } else { 1.0 };
        for slot in &mut self.voices {
            if let Some(Voice { drive, sound: Sound::Clamp { on }, .. }) = *slot {
                self.clamped[drive as usize] = on;
            }
            *slot = None;
        }
        self.friction = self.clamped.map(|c| c as u8 as f32);
        for source in &mut self.sources {
            source.silence();
        }
    }

    fn step(&mut self, drive: u8, phase: u8, stop: bool) {
        let mut scale = 1.0;
        if (self.since_step as f32) < SEEK_SECS * self.fs {
            scale = SEEK_LEVEL * if phase & 1 == 0 { SEEK_ODD } else { 1.0 };
        }
        self.since_step = 0;
        if stop {
            // Against its stop the head only jolts the carriage, harder.
            self.knock(drive, 0.0, &STEP, scale * 1.4, 2.0);
        } else {
            self.knock(drive, 0.0, &STEP, scale, 1.0);
        }
    }

    /// The disk slides in, `at` s from now; the hand turns the lever down,
    /// and the hub clamps the disk as the lever latches.
    fn insert(&mut self, drive: u8, at: f32) {
        self.play(drive, INSERT, at);
        let lever = at + INSERT.last().map_or(0.0, |c| c.at) + LEVER_PAUSE;
        self.play(drive, LEVER, lever);
        self.clamp(drive, lever + LEVER.last().map_or(0.0, |c| c.at), true);
    }

    /// The lever flips up, `at` s from now, letting the disk go, and the
    /// spring pushes the disk out.
    fn eject(&mut self, drive: u8, at: f32) {
        self.clamp(drive, at, false);
        self.play(drive, EJECT, at);
    }

    fn play(&mut self, drive: u8, cues: &'static [Cue], at: f32) {
        for cue in cues {
            match cue.part {
                Part::Knock(k) => self.knock(drive, at + cue.at, k, 1.0, 1.0),
                Part::Rub(r) => self.rub(drive, at + cue.at, r),
            }
        }
    }

    fn samples(&self, secs: f32) -> u32 {
        (secs * self.fs) as u32
    }

    /// A knock `at` s from now, at `scale` of its level, ringing the frame
    /// `ring` times as hard.
    fn knock(&mut self, drive: u8, at: f32, knock: &'static Knock, scale: f32, ring: f32) {
        let gain = knock.gains.map(|g| level(g) * scale);
        let modes = knock.modes.map(|m| m * scale * ring);
        let sound = Sound::Knock { knock, attack: self.samples(knock.attack).max(1), gain, modes };
        self.add(Voice { drive, wait: self.samples(at), pos: 0, sound });
    }

    fn rub(&mut self, drive: u8, at: f32, rub: &'static Rub) {
        let sound = Sound::Rub { rub, gain: rub.gains.map(level), wander: 0.0 };
        self.add(Voice { drive, wait: self.samples(at), pos: 0, sound });
    }

    fn clamp(&mut self, drive: u8, at: f32, on: bool) {
        self.add(Voice { drive, wait: self.samples(at), pos: 0, sound: Sound::Clamp { on } });
    }

    /// Queues a voice; with every slot taken it is dropped.
    fn add(&mut self, voice: Voice) {
        if let Some(slot) = self.voices.iter_mut().find(|s| s.is_none()) {
            *slot = Some(voice);
        }
    }

    /// The next output sample: left and right.
    pub fn sample(&mut self) -> (f32, f32) {
        if self.on {
            if self.level < 1.0 {
                self.level = (self.level + 1.0 / (FADE_SECS * self.fs)).min(1.0);
            }
        } else {
            if self.level <= 0.0 {
                return (0.0, 0.0);
            }
            self.level -= 1.0 / (FADE_SECS * self.fs);
            if self.level <= 0.0 {
                self.level = 0.0;
                self.settle();
                return (0.0, 0.0);
            }
        }
        self.since_step = self.since_step.saturating_add(1);
        self.since_access = self.since_access.saturating_add(1);
        self.gain += (self.volume - self.gain) / (FADE_SECS * self.fs);

        // Each drive's power per band, from its spinning disk and its voices.
        // A drive's sounds share its noise: power adds as it would for
        // noises of their own.
        let mut power = [[0.0f32; BANDS]; 2];
        let mut push = [[0.0f32; MODES.len()]; 2];
        let out = self.motor_sample(&mut power);
        for i in 0..VOICES {
            let Some(mut v) = self.voices[i] else { continue };
            if v.wait > 0 {
                v.wait -= 1;
                self.voices[i] = Some(v);
                continue;
            }
            let d = v.drive as usize;
            let done = match &mut v.sound {
                Sound::Knock { knock, attack, gain, modes } => {
                    if v.pos < *attack {
                        let rise = v.pos as f32 / *attack as f32;
                        for (p, g) in power[d].iter_mut().zip(gain.iter()) {
                            *p += g * g * rise * rise;
                        }
                        false
                    } else {
                        for (p, m) in push[d].iter_mut().zip(modes.iter()) {
                            *p += m;
                        }
                        self.sources[d].join_tail(self.fs, knock, gain);
                        true
                    }
                }
                Sound::Rub { rub, gain, wander } => {
                    let t = v.pos as f32 / self.fs;
                    let x = (t / rub.step - 0.5).clamp(0.0, (rub.env.len() - 1) as f32);
                    let i = x as usize;
                    let j = (i + 1).min(rub.env.len() - 1);
                    let db = rub.env[i] + (rub.env[j] - rub.env[i]) * (x - i as f32);
                    // Low-passed noise, a couple of milliseconds slow, whose
                    // spread (0.058) is scaled to `rough` dB.
                    let n = self.sources[d].white();
                    *wander = 0.98 * *wander + 0.02 * n;
                    let env = 10f32.powf((db + *wander * (rub.rough / 0.058)) / 20.0);
                    for (p, g) in power[d].iter_mut().zip(gain.iter()) {
                        *p += g * g * env * env;
                    }
                    t >= rub.len
                }
                Sound::Clamp { on } => {
                    self.clamped[d] = *on;
                    true
                }
            };
            v.pos += 1;
            self.voices[i] = (!done).then_some(v);
        }

        let (mut left, mut right) = (0.0, 0.0);
        let follow = 1.0 / (FADE_SECS * self.fs);
        for (d, source) in self.sources.iter_mut().enumerate() {
            source.pan.0 += (source.target.0 - source.pan.0) * follow;
            source.pan.1 += (source.target.1 - source.pan.1) * follow;
            let x = out[d] + source.sample(power[d], push[d]);
            left += x * source.pan.0;
            right += x * source.pan.1;
        }
        let g = self.level * self.gain * GAIN;
        (left * g, right * g)
    }

    /// Each drive's motor hum; its spinning disk's noise, added to `power`;
    /// and at each turn of a disk, its swish.
    fn motor_sample(&mut self, power: &mut [[f32; BANDS]; 2]) -> [f32; 2] {
        let dt = 1.0 / self.fs;
        if self.motor {
            self.speed += (1.0 - self.speed) * SPIN_UP * dt;
        } else if self.speed > 0.0 {
            self.speed -= self.speed * SPIN_DOWN * dt;
            if self.speed < 1e-3 {
                self.speed = 0.0;
            }
        }
        if self.idle() {
            if self.idle_level > 0.0 {
                self.idle_level *= IDLE_FLOOR.powf(dt / IDLE_FADE);
                if self.idle_level < IDLE_FLOOR {
                    self.idle_level = 0.0;
                }
            }
        } else if self.idle_level < 1.0 {
            self.idle_level = (self.idle_level.max(IDLE_FLOOR) + dt / IDLE_RETURN).min(1.0);
        }
        if self.speed == 0.0 || self.idle_level == 0.0 {
            return [0.0; 2];
        }
        // The idle fade scales everything the motor is heard by.
        let s = self.speed;
        let heard = s * self.idle_level;

        if self.hum_count == 0 {
            self.hum_count = HUM_EVERY;
            for (turn, &(f, _)) in self.hum_turn.iter_mut().zip(HUM.iter()) {
                let w = TAU * f * s * dt;
                *turn = (w.cos(), w.sin());
            }
            for source in &mut self.sources {
                for p in &mut source.hum {
                    let m = (p.0 * p.0 + p.1 * p.1).sqrt();
                    *p = (p.0 / m, p.1 / m);
                }
            }
        }
        self.hum_count -= 1;

        let mut out = [0.0; 2];
        for d in 0..2 {
            let target = self.clamped[d] as u8 as f32;
            self.friction[d] += (target - self.friction[d]) * 20.0 * dt;
            let noise = heard * (EMPTY + (1.0 - EMPTY) * self.friction[d]);
            for (p, g) in power[d].iter_mut().zip(self.spin.iter()) {
                *p += g * g * noise * noise;
            }
            if self.friction[d] > 0.05 {
                self.turn[d] += TURNS_HZ * s * dt;
                if self.turn[d] >= 1.0 {
                    self.turn[d] -= 1.0;
                    self.knock(d as u8, 0.0, &TURN, heard * self.friction[d], 1.0);
                }
            }
            // The motor hums, rising with its speed.
            let mut hum = 0.0;
            for ((p, &(c, si)), &(_, a)) in self.sources[d].hum.iter_mut().zip(self.hum_turn.iter()).zip(HUM.iter()) {
                *p = (p.0 * c - p.1 * si, p.0 * si + p.1 * c);
                hum += p.1 * a;
            }
            out[d] = hum * heard;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The left and right channels' mean.
    fn run(s: &mut DriveSounds, secs: f32) -> Vec<f32> {
        (0..(secs * s.fs) as usize).map(|_| s.sample()).map(|(l, r)| (l + r) / 2.0).collect()
    }

    fn peak(v: &[f32]) -> f32 {
        v.iter().fold(0.0f32, |m, x| m.max(x.abs()))
    }

    fn rms(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt()
    }

    #[test]
    fn silent_while_off() {
        let mut s = DriveSounds::new(48_000.0);
        s.event(DriveEvent::Motor(true), false);
        s.event(DriveEvent::Insert { drive: 0, swap: false }, false);
        assert!(run(&mut s, 0.1).iter().all(|&x| x == 0.0));
    }

    #[test]
    fn levels_stay_in_range() {
        for fs in [44_100.0, 48_000.0] {
            let mut s = DriveSounds::new(fs);
            s.set_on(true);
            s.event(DriveEvent::Insert { drive: 0, swap: true }, true);
            let insert = run(&mut s, 3.0);
            println!("{fs}: insert peak {}", peak(&insert));
            assert!((0.25..0.8).contains(&peak(&insert)), "insert peak {}", peak(&insert));
            assert!(s.clamped[0]);

            s.event(DriveEvent::Motor(true), true);
            run(&mut s, 1.0);
            let spin = run(&mut s, 1.0);
            println!("{fs}: motor rms {}", rms(&spin));
            assert!((0.002..0.01).contains(&rms(&spin)), "motor rms {}", rms(&spin));

            let mut seek = Vec::new();
            for i in 0..40 {
                s.event(DriveEvent::Step { drive: 0, phase: i as u8 & 3, stop: false }, true);
                seek.extend(run(&mut s, 0.010));
            }
            println!("{fs}: seek rms {}", rms(&seek));
            assert!((0.01..0.03).contains(&rms(&seek)), "seek rms {}", rms(&seek));
            assert!(seek.iter().all(|x| x.is_finite()));

            s.event(DriveEvent::Motor(false), true);
            run(&mut s, 2.0);
            s.event(DriveEvent::Eject { drive: 0 }, true);
            let eject = run(&mut s, 1.0);
            println!("{fs}: eject peak {}", peak(&eject));
            assert!((0.2..0.6).contains(&peak(&eject)), "eject peak {}", peak(&eject));
            assert_eq!(s.speed, 0.0);
            assert!(!s.clamped[0]);
            assert!(s.voices.iter().all(Option::is_none) && s.sources.iter().all(|s| s.tails.iter().all(Option::is_none)));
            assert!(rms(&run(&mut s, 0.5)) < 1e-4, "quiet once all is done");
        }
    }

    #[test]
    fn volume_scales_the_sounds() {
        let render = |volume: f32| {
            let mut s = DriveSounds::new(48_000.0);
            s.set_volume(volume);
            s.set_on(true);
            s.event(DriveEvent::Eject { drive: 0 }, true);
            run(&mut s, 0.5)
        };
        let (full, half) = (render(1.0), render(0.5));
        let ratio = rms(&half[2000..]) / rms(&full[2000..]);
        assert!((ratio - 0.5).abs() < 0.01, "{ratio}");
        assert!(render(0.0)[2000..].iter().all(|&x| x == 0.0));
    }

    #[test]
    fn idle_motor_fades_out_after_a_while() {
        let mut s = DriveSounds::new(48_000.0);
        s.set_idle_mute(true);
        s.set_on(true);
        s.event(DriveEvent::Insert { drive: 0, swap: false }, false);
        s.event(DriveEvent::Motor(true), true);
        let full = rms(&run(&mut s, 9.5)[48_000..]);
        assert!(full > 0.002, "spinning after the motor starts");
        // 10 s after the motor-on: half way through the fade, −30 dB.
        let fading = rms(&run(&mut s, 2.0)[(1.4 * 48_000.0) as usize..]);
        assert!((0.01..0.1).contains(&(fading / full)), "fading: {}", fading / full);
        assert!(rms(&run(&mut s, 2.0)[48_000..]) < 1e-5, "quiet once faded out");
        s.event(DriveEvent::Access, true);
        assert!(rms(&run(&mut s, 1.0)[24_000..]) > 0.002, "heard again with a command");
        s.set_idle_mute(false);
        run(&mut s, 12.0);
        assert!(rms(&run(&mut s, 1.0)) > 0.002, "heard on when not muted");
    }

    #[test]
    fn turning_off_fades_and_settles() {
        let mut s = DriveSounds::new(48_000.0);
        s.set_on(true);
        s.event(DriveEvent::Insert { drive: 1, swap: false }, true);
        run(&mut s, 0.1);
        s.set_on(false);
        let fade = run(&mut s, 0.05);
        assert!(fade[fade.len() / 2..].iter().all(|&x| x == 0.0));
        // The disk that was going in is clamped: its lever was on the way.
        assert!(s.clamped[1]);
        assert!(s.voices.iter().all(Option::is_none) && s.sources.iter().all(|s| s.tails.iter().all(Option::is_none)));
    }

    #[test]
    fn drives_are_panned_apart() {
        let mut s = DriveSounds::new(48_000.0);
        s.set_on(true);
        s.event(DriveEvent::Step { drive: 1, phase: 0, stop: false }, true);
        let (l, r): (Vec<f32>, Vec<f32>) = (0..4800).map(|_| s.sample()).unzip();
        let ratio = rms(&r) / rms(&l);
        assert!((ratio - (3.0 * PI / 16.0).tan().recip()).abs() < 0.01, "{ratio}");
        // The two powers add up to the middle's, which has unit gain in each.
        let (gl, gr) = s.sources[1].pan;
        assert!((gl * gl + gr * gr - 2.0).abs() < 1e-5);
    }

    #[test]
    fn spread_moves_the_drives() {
        // B: stepping: right over left.
        let ratio = |spread: f32| {
            let mut s = DriveSounds::new(48_000.0);
            s.set_spread(spread);
            s.set_on(true);
            s.event(DriveEvent::Step { drive: 1, phase: 0, stop: false }, true);
            let (l, r): (Vec<f32>, Vec<f32>) = (0..4800).map(|_| s.sample()).unzip();
            rms(&r) / rms(&l)
        };
        assert!((ratio(0.0) - 1.0).abs() < 1e-4, "both in the middle");
        assert!(ratio(1.0) > 1e3, "B: in the right speaker alone");
        assert!(ratio(0.5) > ratio(SPREAD) && ratio(SPREAD) > 1.0);

        // A change while playing glides to the new place.
        let mut s = DriveSounds::new(48_000.0);
        s.set_on(true);
        s.set_spread(0.0);
        s.sample();
        assert!(s.sources[0].pan.0 > 1.0, "not at once");
        for _ in 0..9600 {
            s.sample();
        }
        let (gl, gr) = s.sources[0].pan;
        assert!((gl - 1.0).abs() < 1e-3 && (gr - 1.0).abs() < 1e-3, "{gl} {gr}");
    }
}

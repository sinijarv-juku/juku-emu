//! The floppy drives' sounds (spec §4.5): heard when they are on, and the
//! machine runs the same with them on or off, and A: is heard to the left of
//! B:. With JUKU_SOUND_WAV set to a path, the EKDOS boot with them on is
//! written there as a WAV file, to listen to.

mod common;

use std::sync::Arc;

use common::*;
use juku::Machine;
use juku::audio::AudioRing;
use juku::machine::CPU_HZ;

const RATE: u32 = 48_000;

/// A machine with its audio, which is pulled as it runs.
struct Rig {
    m: Machine,
    ring: Arc<AudioRing>,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl Rig {
    fn new(rom: &[u8], sounds: bool) -> Rig {
        let ring = Arc::new(AudioRing::new(RATE, 0));
        let mut m = Machine::with_audio(rom, Some(ring.clone())).unwrap();
        m.set_drive_sounds(sounds);
        Rig { m, ring, left: Vec::new(), right: Vec::new() }
    }

    /// Runs in 20 ms slices, well within the ring.
    fn run(&mut self, secs: f64) {
        let end = self.m.cycles() + (secs * CPU_HZ as f64) as u64;
        while self.m.cycles() < end {
            self.m.run_cycles((CPU_HZ / 50).min(end - self.m.cycles()));
            let (start, n) = (self.left.len(), self.ring.fill());
            self.left.resize(start + n, 0.0);
            self.right.resize(start + n, 0.0);
            self.ring.pull(&mut self.left[start..], &mut self.right[start..]);
        }
    }

    fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            let (col, row, shift) = key_for(ch).unwrap();
            let key = juku::keyboard::key_id(col, row);
            self.m.set_key(juku::keyboard::KEY_SHIFT, shift);
            self.m.set_key(key, true);
            self.run(0.06);
            self.m.set_key(key, false);
            self.m.set_key(juku::keyboard::KEY_SHIFT, false);
            self.run(0.06);
        }
    }

    fn run_until(&mut self, rom: &[u8], limit_s: f64, pred: impl Fn(&str) -> bool) {
        let start = self.m.cycles();
        while !screen_text(&self.m, rom).iter().any(|l| pred(l)) {
            assert!(self.m.cycles() - start < (limit_s * CPU_HZ as f64) as u64, "{}", screen_text(&self.m, rom).join("\n"));
            self.run(0.1);
        }
    }

    fn snapshot(&self) -> Vec<u8> {
        self.m.save_snapshot(None, &[]).unwrap()
    }
}

/// The drive sounds in `on`: what it has over `off`, which played the same
/// without them, as RMS over the samples from `from` to `to` seconds, in the
/// left channel, the right, and both.
fn added_lr(on: &Rig, off: &Rig, from: f64, to: f64) -> (f32, f32, f32) {
    let range = (from * RATE as f64) as usize..(to * RATE as f64) as usize;
    let rms = |a: &[f32], b: &[f32]| {
        let (a, b) = (&a[range.clone()], &b[range.clone()]);
        (a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>() / a.len() as f32).sqrt()
    };
    let (l, r) = (rms(&on.left, &off.left), rms(&on.right, &off.right));
    (l, r, ((l * l + r * r) / 2.0).sqrt())
}

fn added(on: &Rig, off: &Rig, from: f64, to: f64) -> f32 {
    added_lr(on, off, from, to).2
}

fn write_wav(path: &str, left: &[f32], right: &[f32]) {
    let data: Vec<u8> = left
        .iter()
        .zip(right)
        .flat_map(|(l, r)| [*l, *r])
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())
        .collect();
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    for v in [16u32, 1 | 2 << 16, RATE, RATE * 4, 4 | 16 << 16] {
        wav.extend_from_slice(&v.to_le_bytes());
    }
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    std::fs::write(path, wav).unwrap();
}

#[test]
fn disks_going_in_and_out_are_heard() {
    let rom = load_rom();
    let mut rigs = [true, false].map(|on| Rig::new(&rom, on));
    for r in &mut rigs {
        // Put in before the machine runs: already in the drive, no sound.
        r.m.insert_blank_disk(1);
        r.run(1.0);
        r.m.insert_blank_disk(0);
        r.run(2.0);
        r.m.eject_disk(0);
        r.run(1.0);
        r.m.set_drive_sounds(false);
        r.m.insert_blank_disk(0);
        r.run(1.0);
    }
    let [on, off] = &rigs;
    assert_eq!(on.snapshot(), off.snapshot(), "the machine runs the same");
    // At the monitor prompt the motor is off.
    assert_eq!(added(on, off, 0.0, 1.0), 0.0, "B:'s disk was in already");
    // The disk slides in, and the lever closes 1.4 s after it starts.
    let insert = added(on, off, 1.0, 3.0);
    let eject = added(on, off, 3.0, 3.5);
    println!("insert {insert}, eject {eject}");
    assert!(insert > 0.01 && eject > 0.01);
    assert_eq!(added(on, off, 4.1, 5.0), 0.0, "silent once turned off");
}

/// A disk going into A: is heard more on the left, into B: more on the right.
#[test]
fn drives_are_heard_left_and_right() {
    let rom = load_rom();
    let mut rigs = [true, false].map(|on| Rig::new(&rom, on));
    for r in &mut rigs {
        r.run(0.5);
        r.m.insert_blank_disk(0);
        r.run(2.0);
        r.m.insert_blank_disk(1);
        r.run(2.0);
    }
    let [on, off] = &rigs;
    let (al, ar, _) = added_lr(on, off, 0.5, 2.5);
    let (bl, br, _) = added_lr(on, off, 2.5, 4.5);
    println!("A: {al} {ar}, B: {bl} {br}");
    // Panned a quarter of the way: cos and sin of 3π/16, a ratio of 1.5.
    assert!((1.4..1.6).contains(&(al / ar)), "A: {}", al / ar);
    assert!((1.4..1.6).contains(&(br / bl)), "B: {}", br / bl);
}

#[test]
fn ekdos_boot_steps_the_head() {
    let Some(image) = test_file(PROGRAMS_DISK) else { return };
    let rom = load_rom();
    let mut rigs = [true, false].map(|on| Rig::new(&rom, on));
    for r in &mut rigs {
        r.m.insert_disk(0, image.clone(), true).unwrap();
        r.run_until(&rom, 10.0, |l| l.starts_with('*'));
        r.type_text("t");
        r.run_until(&rom, 5.0, |l| l.contains("<D>isk"));
        r.type_text("dd");
        r.run_until(&rom, 10.0, |l| l.starts_with("A>"));
        r.type_text("dir\r");
        r.run(3.0);
        r.m.eject_disk(0);
        r.run(1.5);
    }
    let [on, off] = &rigs;
    if let Some(path) = std::env::var_os("JUKU_SOUND_WAV") {
        write_wav(path.to_str().unwrap(), &on.left, &on.right);
    }
    assert_eq!(on.snapshot(), off.snapshot(), "the machine runs the same");
    let len = on.left.len().min(off.left.len()) as f64 / RATE as f64;
    let sound = added(on, off, 0.0, len);
    println!("{len:.1} s, drive sounds {sound}");
    assert!(sound > 0.005);
}

/// With the idle motor muted, EKDOS at its prompt leaves the drive alone,
/// so the spinning fades out after 10 s, and a command brings it back.
#[test]
fn idle_motor_fades_out() {
    let Some(image) = test_file(PROGRAMS_DISK) else { return };
    let rom = load_rom();
    let mut rigs = [true, false].map(|on| Rig::new(&rom, on));
    rigs[0].m.set_drive_idle_mute(true);
    let mut marks = [0.0; 3];
    for r in &mut rigs {
        r.m.insert_disk(0, image.clone(), true).unwrap();
        r.run_until(&rom, 10.0, |l| l.starts_with('*'));
        r.type_text("t");
        r.run_until(&rom, 5.0, |l| l.contains("<D>isk"));
        r.type_text("dd");
        r.run_until(&rom, 10.0, |l| l.starts_with("A>"));
        r.run(1.0);
        marks[0] = r.left.len() as f64 / RATE as f64;
        r.run(13.0);
        marks[1] = r.left.len() as f64 / RATE as f64;
        r.type_text("dir\r");
        r.run(1.0);
        marks[2] = r.left.len() as f64 / RATE as f64;
    }
    let [on, off] = &rigs;
    let spinning = added(on, off, marks[0], marks[0] + 1.0);
    let idle = added(on, off, marks[1] - 0.5, marks[1]);
    let again = added(on, off, marks[2] - 0.5, marks[2]);
    println!("spinning {spinning}, idle {idle}, after DIR {again}");
    assert!(spinning > 0.001 && again > 0.001);
    assert!(idle < 1e-4);
}

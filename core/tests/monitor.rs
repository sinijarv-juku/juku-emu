//! Drives the RomBios monitor through the emulated keyboard and checks the
//! speaker output.

mod common;

use std::sync::Arc;

use common::*;
use juku::Machine;
use juku::audio::AudioRing;
use juku::machine::CPU_HZ;

const SAMPLE_RATE: u32 = 48_000;

/// Runs for `cycles`, draining the audio ring as it goes. The speaker is in
/// the middle, so the left channel is kept.
fn run_collect(m: &mut Machine, ring: &AudioRing, cycles: u64, out: &mut Vec<f32>) {
    let slice = CPU_HZ / 100;
    let (mut left, mut right) = (vec![0.0f32; 8192], vec![0.0f32; 8192]);
    for _ in 0..cycles / slice {
        m.run_cycles(slice);
        let n = ring.fill();
        ring.pull(&mut left[..n], &mut right[..n]);
        assert_eq!(left[..n], right[..n]);
        out.extend_from_slice(&left[..n]);
    }
}

/// Frequency and duration (s) of the longest steady tone in `samples`: the
/// longest run of rising zero crossings (with hysteresis) whose periods stay
/// within 5% of each other.
fn measure_tone(samples: &[f32]) -> (f64, f64) {
    let mut crossings = Vec::new();
    let mut high = false;
    for (i, &s) in samples.iter().enumerate() {
        if !high && s > 0.02 {
            high = true;
            crossings.push(i);
        } else if high && s < -0.02 {
            high = false;
        }
    }
    let periods: Vec<usize> = crossings.windows(2).map(|w| w[1] - w[0]).collect();
    let (mut best, mut start) = ((0, 0), 0);
    for i in 1..=periods.len() {
        let steady = i < periods.len() && periods[i].abs_diff(periods[start]) * 20 <= periods[start];
        if !steady {
            if i - start > best.1 - best.0 {
                best = (start, i);
            }
            start = i;
        }
    }
    let (first, last) = (best.0, best.1);
    assert!(last - first > 10, "no tone: {periods:?}");
    let span = (crossings[last] - crossings[first]) as f64 / SAMPLE_RATE as f64;
    ((last - first) as f64 / span, span)
}

#[test]
fn dump_command_shows_memory() {
    let rom = load_rom();
    let mut m = Machine::new(&rom).unwrap();
    run_to_prompt(&mut m, &rom, 10.0);
    type_text(&mut m, "D0000001F\r");
    m.run_cycles(CPU_HZ / 2);
    let text = screen_text(&m, &rom);
    println!("{}", text.join("\n"));
    let at = text.iter().position(|l| l.starts_with("*D0000/001F")).expect("command echoed");
    // Mode 1 view of RAM: the BIOS has put a JMP at 0000h.
    assert!(text[at + 1].starts_with("0000: C3 "), "{text:?}");
    assert!(text[at + 4].starts_with("0018: "), "{text:?}");
    assert_eq!(text[at + 5], "*");
}

#[test]
fn bleep_900_hz() {
    let rom = load_rom();
    let ring = Arc::new(AudioRing::new(SAMPLE_RATE, 0));
    let mut m = Machine::with_audio(&rom, Some(ring.clone())).unwrap();
    let mut samples = Vec::new();
    run_collect(&mut m, &ring, CPU_HZ * 3, &mut samples);
    assert!(has_prompt(&screen_text(&m, &rom)));

    // MVI A,5 / LXI D,08AEh / CALL BLEEP / JMP MONITOR: 900 Hz for 500 ms.
    // The ROM counts A in 100 ms units (5 frame ticks each, ROM 058Ah).
    let program = [0x3E, 5, 0x11, 0xAE, 0x08, 0xCD, 0x86, 0xFF, 0xC3, 0xC4, 0xFF];
    for (i, &b) in program.iter().enumerate() {
        m.poke(0x0100 + i as u16, b);
    }
    type_text(&mut m, "G0100");
    samples.clear();

    m.set_key(juku::keyboard::key_id(8, 5), true);
    run_collect(&mut m, &ring, CPU_HZ / 20, &mut samples);
    m.set_key(juku::keyboard::key_id(8, 5), false);
    run_collect(&mut m, &ring, CPU_HZ, &mut samples);
    println!("{}", screen_text(&m, &rom).join("\n"));

    if let Ok(path) = std::env::var("JUKU_DUMP_AUDIO") {
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        std::fs::write(path, bytes).unwrap();
    }
    let (freq, secs) = measure_tone(&samples);
    println!("tone {freq:.1} Hz for {secs:.3} s");
    assert!((freq - 900.0).abs() < 2.0, "{freq} Hz");
    assert!((secs - 0.5).abs() < 0.03, "{secs} s");
}

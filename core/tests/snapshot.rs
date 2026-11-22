//! Snapshots: a restored machine runs on exactly as the original would, and
//! a bad file changes nothing.

mod common;

use common::*;
use juku::Machine;
use juku::keyboard::key_id;
use juku::machine::CPU_HZ;
use juku::lz;
use juku::snapshot::{self, Reader, Sections, SnapshotError, SnapshotWriter};

/// Everything a snapshot holds. Two machines with equal saves are in the
/// same state.
fn state(m: &Machine) -> Vec<u8> {
    m.save_snapshot(None, &[]).unwrap()
}

/// A machine at the monitor prompt, a little past it.
fn at_prompt(rom: &[u8]) -> Machine {
    let mut m = Machine::new(rom).unwrap();
    run_to_prompt(&mut m, rom, 10.0);
    m.run_cycles(123_457);
    m
}

/// Types a monitor command and runs on, the same way on every machine.
fn script(m: &mut Machine) {
    type_text(m, "D0000001F\r");
    m.run_cycles(CPU_HZ / 3);
}

/// Replaces a section's data, or drops the section (`None`).
type Edit = fn(&[u8]) -> Option<Vec<u8>>;

/// Copies a snapshot file, letting `edit` replace or drop sections.
fn rebuild(file: &[u8], edit: impl Fn(&[u8; 4], &[u8]) -> Option<Vec<u8>>) -> Vec<u8> {
    let sections = Sections::parse(file).unwrap();
    let mut w = SnapshotWriter::new(&snapshot::frame_png(&juku::Frame::new_boxed())).unwrap();
    for (tag, data) in sections.iter() {
        if let Some(data) = edit(&tag, data) {
            w.section(tag, &data);
        }
    }
    w.finish()
}

#[test]
fn restored_machine_runs_on_identically() {
    let rom = load_rom();
    let mut a = at_prompt(&rom);
    // In the middle of a key press, so the keyboard state matters too.
    a.set_key(key_id(1, 0), true);
    a.run_cycles(CPU_HZ / 50);
    let file = state(&a);

    // A machine running something else entirely, on another ROM.
    let other = load_rom_named("ekta31");
    let mut b = Machine::new(&other).unwrap();
    b.run_cycles(CPU_HZ * 3);
    b.set_key_click(false);
    b.load_snapshot(&file).unwrap();
    assert_eq!(state(&b), file, "a restored machine saves the same snapshot");

    for m in [&mut a, &mut b] {
        m.run_cycles(CPU_HZ / 20);
        m.set_key(key_id(1, 0), false);
        script(m);
    }
    assert_eq!(a.cycles(), b.cycles());
    assert_eq!(a.ram(), b.ram());
    assert!(state(&a) == state(&b), "the machines went apart");
    let text = screen_text(&b, &rom);
    assert!(text.iter().any(|l| l.ends_with("D0000/001F")), "{text:?}");
}

/// Versions 1 and 2, which no release wrote, are refused as older.
#[test]
fn older_versions_are_refused() {
    let rom = load_rom();
    let file = state(&at_prompt(&rom));
    let mut m = Machine::new(&rom).unwrap();
    for version in [1u32, 2] {
        let old = rebuild(&file, |t, d| Some(if t == b"SNAP" { version.to_le_bytes().to_vec() } else { d.to_vec() }));
        assert_eq!(m.load_snapshot(&old), Err(SnapshotError::Old), "version {version}");
    }
}

#[test]
fn snapshot_is_a_png_with_the_screen() {
    let rom = load_rom();
    let m = at_prompt(&rom);
    let file = state(&m);
    assert_eq!(file[..8], [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    // The picture comes first: a viewer reads it before the state.
    assert_eq!(&file[12..16], b"IHDR");
    assert_eq!(file[16..24], [0, 0, 1, 64, 0, 0, 0, 241], "320 × 241");
    assert!(file.len() < 140_000, "{} bytes with no disks", file.len());

    // A host thumbnail replaces the core's.
    let thumb = snapshot::frame_png(&juku::Frame::new_boxed());
    let with_thumb = m.save_snapshot(Some(&thumb), &[]).unwrap();
    assert_eq!(with_thumb[..thumb.len() - 12], thumb[..thumb.len() - 12]);
    assert!(m.save_snapshot(Some(b"not a png"), &[]).is_none());
}

#[test]
fn host_sections_come_back() {
    let rom = load_rom();
    let m = at_prompt(&rom);
    let file = m.save_snapshot(None, &[(*b"meta", b"name=test"), (*b"mac ", &[1, 2, 3])]).unwrap();
    assert_eq!(snapshot::section(&file, b"meta"), Ok(Some(&b"name=test"[..])));
    assert_eq!(snapshot::section(&file, b"mac "), Ok(Some(&[1, 2, 3][..])));
    assert_eq!(snapshot::section(&file, b"none"), Ok(None));
    assert!(m.save_snapshot(None, &[(*b"META", b"")]).is_none(), "capitals are the core's");
    assert!(m.save_snapshot(None, &[(*b"m\0\0\0", b"")]).is_none());

    let mut b = Machine::new(&rom).unwrap();
    b.load_snapshot(&file).unwrap();
    assert_eq!(state(&b), state(&m), "host sections do not affect the machine");
}

#[test]
fn rom_is_stored_as_its_file_with_the_click_flag() {
    let rom = load_rom();
    let mut a = at_prompt(&rom);
    assert!(a.set_key_click(false));
    let file = state(&a);
    assert_eq!(a.system_rom(), rom, "unpatched");
    let mut r = Reader::new(Sections::parse(&file).unwrap().get(b"ROM ").unwrap());
    assert_eq!(r.packed(rom.len()), Some(rom.clone()), "unpatched");
    assert_eq!(r.bool(), Some(true), "click off");

    let mut b = Machine::new(&rom).unwrap();
    b.load_snapshot(&file).unwrap();
    assert_eq!(state(&b), file, "the patch is applied again");
    assert!(b.set_key_click(true));
    assert!(state(&b) != file);
}

#[test]
fn expansion_rom_is_restored() {
    let rom = load_rom();
    let mut image = vec![0u8; 0x1000];
    image[..7].copy_from_slice(&[0xC3, 0x07, 0x01, 0xBA, 0xDC, 0x00, 0x10]);
    image[0xFFF] = 0x5A;
    let mut a = Machine::new(&rom).unwrap();
    a.insert_expansion(&image).unwrap();
    run_to_prompt(&mut a, &rom, 10.0);
    let file = state(&a);
    assert_eq!(a.expansion_rom(), &image[..], "trailing FFh is left out");

    let mut b = Machine::new(&rom).unwrap();
    b.load_snapshot(&file).unwrap();
    assert_eq!(state(&b), file);
    // And back out again: a snapshot without one unplugs it.
    let empty = state(&at_prompt(&rom));
    b.load_snapshot(&empty).unwrap();
    assert!(b.expansion_rom().is_empty());
}

#[test]
fn bad_files_change_nothing() {
    let rom = load_rom();
    let good = state(&at_prompt(&rom));
    let mut m = Machine::new(&rom).unwrap();
    m.run_cycles(CPU_HZ);
    let before = state(&m);

    let mut bad: Vec<(String, Vec<u8>, SnapshotError)> = vec![
        ("empty".into(), vec![], SnapshotError::NotSnapshot),
        ("text".into(), b"hello".to_vec(), SnapshotError::NotSnapshot),
        ("thumbnail only".into(), snapshot::frame_png(m.frame()), SnapshotError::NotSnapshot),
    ];
    for len in (0..good.len()).step_by(997).chain([good.len() - 13, good.len() - 1]) {
        let error = if len < 8 { SnapshotError::NotSnapshot } else { SnapshotError::Damaged };
        bad.push((format!("truncated to {len}"), good[..len].to_vec(), error));
    }
    for at in [100, 5_000, good.len() / 2, good.len() - 20] {
        let mut file = good.clone();
        file[at] ^= 0x04;
        bad.push((format!("bit flipped at {at}"), file, SnapshotError::Damaged));
    }
    let edits: [(&str, &[u8; 4], Edit, SnapshotError); 6] = [
        ("newer version", b"SNAP", |_| Some(99u32.to_le_bytes().to_vec()), SnapshotError::Version),
        ("memory mode 7", b"SYS ", |d| Some([&[7], &d[1..]].concat()), SnapshotError::Damaged),
        ("short RAM", b"RAM ", |d| Some(d[1..].to_vec()), SnapshotError::Damaged),
        ("no FDC", b"FDC ", |_| None, SnapshotError::Damaged),
        ("PIT mode 6", b"PIT1", |d| Some([&[6], &d[1..]].concat()), SnapshotError::Damaged),
        ("extra byte", b"CPU ", |d| Some([d, &[0]].concat()), SnapshotError::Damaged),
    ];
    for (name, tag, edit, error) in edits {
        let file = rebuild(&good, |t, d| if t == tag { edit(d) } else { Some(d.to_vec()) });
        bad.push((name.into(), file, error));
    }

    for (name, file, error) in bad {
        assert_eq!(m.load_snapshot(&file), Err(error), "{name}");
        assert!(state(&m) == before, "{name}: the machine changed");
    }
    // The rebuilt file itself is good.
    assert_eq!(m.load_snapshot(&rebuild(&good, |_, d| Some(d.to_vec()))), Ok(()));
}

#[test]
fn restore_during_a_disk_command() {
    let Some(image) = test_file(PROGRAMS_DISK) else { return };
    let rom = load_rom();
    let mut a = Machine::new(&rom).unwrap();
    a.insert_disk(0, image.clone(), true).unwrap();
    boot_ekdos(&mut a, &rom);
    // Start a directory listing and stop while a sector is being read.
    type_text(&mut a, "dir\r");
    let mut slices = 0;
    while !a.drive_active(0) {
        a.run_cycles(1000);
        slices += 1;
        assert!(slices < 4000, "the disk was never read");
    }
    a.run_cycles(3000);
    assert!(a.drive_active(0));
    let file = state(&a);

    let mut b = Machine::new(&rom).unwrap();
    b.load_snapshot(&file).unwrap();
    assert_eq!(b.disk(0).unwrap().data(), &image[..]);
    assert!(b.disk(0).unwrap().write_protect());
    assert!(b.disk(1).is_none());
    for m in [&mut a, &mut b] {
        m.run_cycles(CPU_HZ * 2);
    }
    assert!(state(&a) == state(&b), "the machines went apart");
    let text = screen_text(&b, &rom);
    assert!(text.iter().filter(|l| l.starts_with("A:")).count() > 3, "{text:?}");
}

#[test]
fn c_interface_round_trip() {
    use juku::ffi::*;
    use std::ptr;

    let rom = load_rom();
    unsafe {
        let a = juku_new(rom.as_ptr(), rom.len(), ptr::null());
        let b = juku_new(rom.as_ptr(), rom.len(), ptr::null());
        juku_run_cycles(a, CPU_HZ * 3);

        let meta = b"name=test";
        let sections = [JukuSection { tag: *b"meta", data: meta.as_ptr(), len: meta.len() }];
        let mut len = 0;
        let file = juku_snapshot_save(a, ptr::null(), 0, sections.as_ptr(), 1, &mut len);
        assert!(!file.is_null() && len > 0);
        let data = std::slice::from_raw_parts(file, len);

        let (mut out, mut out_len) = (ptr::null(), 0);
        assert_eq!(juku_snapshot_section(file, len, b"meta".as_ptr(), &mut out, &mut out_len), 0);
        assert_eq!(std::slice::from_raw_parts(out, out_len), meta);
        assert_eq!(juku_snapshot_section(file, len, b"none".as_ptr(), &mut out, &mut out_len), 0);
        assert!(out.is_null() && out_len == 0);
        assert_eq!(juku_snapshot_section(b"x".as_ptr(), 1, b"meta".as_ptr(), &mut out, &mut out_len), -1);

        assert_eq!(juku_snapshot_load(b, file, len), 0);
        assert_eq!(state(&*b), state(&*a));
        assert_eq!(juku_snapshot_load(b, ptr::null(), 0), -1);
        assert_eq!(juku_snapshot_load(b, file, len - 1), -2);
        let newer = rebuild(data, |t, d| {
            Some(if t == b"SNAP" { (snapshot::VERSION + 1).to_le_bytes().to_vec() } else { d.to_vec() })
        });
        assert_eq!(juku_snapshot_load(b, newer.as_ptr(), newer.len()), -3);
        let older = rebuild(data, |t, d| Some(if t == b"SNAP" { 2u32.to_le_bytes().to_vec() } else { d.to_vec() }));
        assert_eq!(juku_snapshot_load(b, older.as_ptr(), older.len()), -4);
        juku_snapshot_free(file, len);

        // A bad thumbnail or tag gives NULL.
        let bad = [JukuSection { tag: *b"META", data: ptr::null(), len: 0 }];
        assert!(juku_snapshot_save(a, ptr::null(), 0, bad.as_ptr(), 1, &mut len).is_null());
        assert!(juku_snapshot_save(a, b"png".as_ptr(), 3, ptr::null(), 0, &mut len).is_null());
        assert_eq!(len, 0);
        juku_free(a);
        juku_free(b);
    }
}

/// The disk images go into snapshots compressed and come back as they were.
#[test]
fn disk_images_compress() {
    for name in [PROGRAMS_DISK, GAMES_DISK] {
        let Some(image) = test_file(name) else { return };
        let start = std::time::Instant::now();
        let packed = lz::compress(&image);
        let packing = start.elapsed();
        assert_eq!(lz::decompress(&packed, image.len()).as_deref(), Some(&image[..]), "{name}");
        eprintln!("{name}: {} bytes, {:?} to compress, {:?} in all", packed.len(), packing, start.elapsed());
        assert!(packed.len() < image.len() * 9 / 10, "{name}: {} bytes", packed.len());
    }
}

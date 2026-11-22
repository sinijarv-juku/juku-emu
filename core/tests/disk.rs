//! Boots EKDOS from the disk images and saves a file. The images are not
//! part of the repository: set JUKU_TEST_DISKS to the directory holding
//! jukprog1.juk and jukgame1.juk, or the tests are skipped.

mod common;

use common::*;
use juku::Machine;

fn boots_ekdos_and_lists_files(disk: &str) {
    let Some(image) = test_file(disk) else { return };
    let rom = load_rom();
    let mut m = Machine::new(&rom).unwrap();
    m.insert_disk(0, image, true).expect("800 KB image");
    boot_ekdos(&mut m, &rom);
    let text = screen_text(&m, &rom);
    println!("{}", text.join("\n"));
    assert!(text.iter().any(|l| l.contains("EKDOS")), "{disk}: {text:?}");

    let text = command(&mut m, &rom, "dir");
    println!("{}", text.join("\n"));
    assert!(text.iter().filter(|l| l.starts_with("A:")).count() > 3, "{disk}: {text:?}");
}

#[test]
fn boots_ekdos_from_programs_disk() {
    boots_ekdos_and_lists_files(PROGRAMS_DISK);
}

#[test]
fn boots_ekdos_from_games_disk() {
    boots_ekdos_and_lists_files(GAMES_DISK);
}

#[test]
fn saved_file_survives_reinsert() {
    let Some(image) = test_file(PROGRAMS_DISK) else { return };
    let rom = load_rom();
    let mut m = Machine::new(&rom).unwrap();
    m.insert_disk(0, image, false).unwrap();
    boot_ekdos(&mut m, &rom);

    command(&mut m, &rom, "era zz.com");
    let text = command(&mut m, &rom, "save 2 zz.com");
    println!("{}", text.join("\n"));
    assert!(m.disk(0).unwrap().dirty(), "SAVE wrote to the disk");

    // Take the image out, put it back, and boot again from scratch.
    let saved = m.eject_disk(0).unwrap().data().to_vec();
    let mut m = Machine::new(&rom).unwrap();
    m.insert_disk(0, saved, true).unwrap();
    boot_ekdos(&mut m, &rom);
    let text = command(&mut m, &rom, "dir zz.com");
    println!("{}", text.join("\n"));
    assert!(text.iter().any(|l| l.contains("ZZ") && l.contains("COM")), "{text:?}");
}

/// A new disk is formatted and empty: EKDOS lists nothing on it, and a file
/// saved to it is found there.
#[test]
fn blank_disk_takes_files() {
    let Some(image) = test_file(PROGRAMS_DISK) else { return };
    let rom = load_rom();
    let mut m = Machine::new(&rom).unwrap();
    m.insert_disk(0, image, true).unwrap();
    m.insert_blank_disk(1);
    boot_ekdos(&mut m, &rom);

    let text = command(&mut m, &rom, "dir b:");
    println!("{}", text.join("\n"));
    assert!(!text.iter().any(|l| l.starts_with("B:")), "{text:?}");

    let text = command(&mut m, &rom, "save 1 b:new.com");
    println!("{}", text.join("\n"));
    assert!(m.disk(1).unwrap().dirty(), "SAVE wrote to the disk");
    let text = command(&mut m, &rom, "dir b:");
    println!("{}", text.join("\n"));
    assert!(text.iter().any(|l| l.contains("NEW") && l.contains("COM")), "{text:?}");
}

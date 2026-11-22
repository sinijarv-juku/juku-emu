//! Helpers shared by the machine-level integration tests: ROM loading,
//! reading the screen back as text, and typing on the keyboard.
#![allow(dead_code)]

use juku::Machine;
use juku::machine::CPU_HZ;

/// The Latin font in the ROM has 9 bytes per glyph from code 20h (MD §14.3).
/// It is at 1D02h in RomBios 3.43m and 1D01h in 3.43 #0031, so it is found
/// by the glyphs for A and B.
const GLYPHS_AB: [u8; 18] = [
    0x08, 0x14, 0x22, 0x22, 0x3E, 0x22, 0x22, 0x00, 0x00, // A
    0x3C, 0x12, 0x12, 0x1C, 0x12, 0x12, 0x3C, 0x00, 0x00, // B
];

fn rom_font(rom: &[u8]) -> usize {
    let at = rom.windows(GLYPHS_AB.len()).position(|w| w == GLYPHS_AB).expect("Latin font in the ROM");
    at - (b'A' - 0x20) as usize * 9
}
/// RomBios text cells are one byte wide and 10 lines high, with the glyph
/// starting on the cell's second line.
const CELL_LINES: usize = 10;
const GLYPH_TOP: usize = 1;

/// The RomBios ROMs in resources/roms; the first is the default. The other
/// one there, jmon33, has another font and prompt, so the text helpers
/// cannot read its screen.
pub const ROMS: [&str; 2] = ["ekta37", "ekta31"];

/// The default ROM, RomBios 3.43m.
pub fn load_rom() -> Vec<u8> {
    load_rom_named(ROMS[0])
}

pub fn load_rom_named(name: &str) -> Vec<u8> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../resources/roms");
    std::fs::read(format!("{dir}/{name}.bin")).expect(name)
}

/// Decodes the last frame into lines of text. Normal and inverse glyphs are
/// recognised; an unrecognised cell becomes `?`.
pub fn screen_text(m: &Machine, rom: &[u8]) -> Vec<String> {
    let f = m.frame();
    let bpl = f.active_line(0).len();
    let font = rom_font(rom);
    let glyph = |ch: usize| &rom[font + (ch - 0x20) * 9..][..9];
    (0..(f.active_h as usize - GLYPH_TOP) / CELL_LINES)
        .map(|row| {
            let line: String = (0..bpl)
                .map(|col| {
                    let cell: Vec<u8> = (0..9).map(|r| f.active_line(row * CELL_LINES + GLYPH_TOP + r)[col]).collect();
                    let inverse: Vec<u8> = cell.iter().map(|b| !b).collect();
                    (0x20..0x80)
                        .find(|&ch| glyph(ch) == cell || glyph(ch) == inverse)
                        .map_or('?', |ch| ch as u8 as char)
                })
                .collect();
            line.trim_end().to_string()
        })
        .collect()
}

pub fn has_prompt(lines: &[String]) -> bool {
    lines.iter().any(|l| l.starts_with('*'))
}

/// Runs in 0.1 s slices until the screen shows the banner and the prompt.
/// Returns the emulated time taken, in seconds.
pub fn run_to_prompt(m: &mut Machine, rom: &[u8], limit_s: f64) -> f64 {
    let slice = CPU_HZ / 10;
    let start = m.cycles();
    while ((m.cycles() - start) as f64) < limit_s * CPU_HZ as f64 {
        m.run_cycles(slice);
        let text = screen_text(m, rom);
        if text.iter().any(|l| l.starts_with("RomBios 3.4")) && has_prompt(&text) {
            return (m.cycles() - start) as f64 / CPU_HZ as f64;
        }
    }
    panic!("no prompt after {limit_s} s:\n{}", screen_text(m, rom).join("\n"));
}

/// Juku key (column, row) for a character, and whether it needs SHIFT
/// (MD §6.3).
pub fn key_for(ch: char) -> Option<(u8, u8, bool)> {
    const ROWS: [(&str, &str, u8); 4] = [
        // Unshifted characters by column 0..14 for rows 1, 3, 4 and 5; ' ' = no key.
        ("nxv bzcm   ;/.,", "NXV BZCM   +?><", 1),
        ("ywr tqeu    poi", "YWR TQEU    POI", 3),
        ("624 5137  @-098", "&\"$ %!#'  `=_)(", 4),
        ("hsf gadj  :  lk", "HSF GADJ  *  LK", 5),
    ];
    if ch == ' ' {
        return Some((11, 2, false));
    }
    if ch == '\r' {
        return Some((8, 5, false));
    }
    for (plain, shifted, row) in ROWS {
        if let Some(col) = plain.chars().position(|c| c == ch && c != ' ') {
            return Some((col as u8, row, false));
        }
        if let Some(col) = shifted.chars().position(|c| c == ch && c != ' ') {
            return Some((col as u8, row, true));
        }
    }
    None
}

/// Types `text` with realistic key timing (60 ms down, 60 ms up).
pub fn type_text(m: &mut Machine, text: &str) {
    let hold = CPU_HZ * 6 / 100;
    for ch in text.chars() {
        let (col, row, shift) = key_for(ch).unwrap_or_else(|| panic!("no key for {ch:?}"));
        if shift {
            m.set_key(juku::keyboard::KEY_SHIFT, true);
        }
        m.set_key(juku::keyboard::key_id(col, row), true);
        m.run_cycles(hold);
        m.set_key(juku::keyboard::key_id(col, row), false);
        m.set_key(juku::keyboard::KEY_SHIFT, false);
        m.run_cycles(hold);
    }
}

/// Runs in 0.1 s slices until a screen line satisfies `pred`, up to `limit_s`.
pub fn run_until(m: &mut Machine, rom: &[u8], limit_s: f64, pred: impl Fn(&str) -> bool) -> Vec<String> {
    let start = m.cycles();
    while ((m.cycles() - start) as f64) < limit_s * CPU_HZ as f64 {
        m.run_cycles(CPU_HZ / 10);
        let text = screen_text(m, rom);
        if text.iter().any(|l| pred(l)) {
            return text;
        }
    }
    panic!("timed out after {limit_s} s:\n{}", screen_text(m, rom).join("\n"));
}

/// Boots EKDOS from drive A: through the monitor's T command.
pub fn boot_ekdos(m: &mut Machine, rom: &[u8]) {
    run_to_prompt(m, rom, 10.0);
    type_text(m, "t");
    run_until(m, rom, 5.0, |l| l.contains("<D>isk"));
    type_text(m, "d");
    run_until(m, rom, 5.0, |l| l.contains("System disk type"));
    type_text(m, "d");
    run_until(m, rom, 10.0, |l| l.starts_with("A>"));
}

/// Types a CCP command and waits for the next `A>` prompt.
pub fn command(m: &mut Machine, rom: &[u8], cmd: &str) -> Vec<String> {
    let before = screen_text(m, rom).iter().filter(|l| l.starts_with("A>")).count();
    type_text(m, &format!("{cmd}\r"));
    let start = m.cycles();
    loop {
        m.run_cycles(CPU_HZ / 10);
        let text = screen_text(m, rom);
        // The screen scrolls, so look for a prompt on the last non-empty line.
        let last = text.iter().rev().find(|l| !l.is_empty()).cloned().unwrap_or_default();
        if last == "A>" && (text.iter().filter(|l| l.starts_with("A>")).count() != before || m.cycles() - start > CPU_HZ) {
            return text;
        }
        assert!(m.cycles() - start < 20 * CPU_HZ, "{cmd}: no prompt\n{}", text.join("\n"));
    }
}

/// Types the name of a program at the `A>` prompt and runs until it has
/// cleared the screen. With the sectors in order on the track, EKDOS loads
/// about 2 KB a second in the emulator.
pub fn start_program(m: &mut Machine, rom: &[u8], name: &str) {
    type_text(m, &format!("{name}\r"));
    let start = m.cycles();
    while screen_text(m, rom).iter().any(|l| l.starts_with("A>")) {
        m.run_cycles(CPU_HZ / 10);
        assert!(m.cycles() - start < 30 * CPU_HZ, "{name} did not start");
    }
}

/// Disk images the tests boot from. They are not part of the repository:
/// JUKU_TEST_DISKS names the directory that holds them.
pub const GAMES_DISK: &str = "jukgame1.juk";
pub const PROGRAMS_DISK: &str = "jukprog1.juk";
/// JBASIC 1.1, the 8 KB expansion ROM.
pub const JBASIC: &str = "jbasic11.bin";

/// Reads a disk image or the JBASIC cartridge from the JUKU_TEST_DISKS
/// directory, or returns None (the test is skipped) when the variable is not
/// set. Tests work on the copy in memory, so the file never changes.
pub fn test_file(name: &str) -> Option<Vec<u8>> {
    let Some(dir) = std::env::var_os("JUKU_TEST_DISKS") else {
        eprintln!("skipping: JUKU_TEST_DISKS not set");
        return None;
    };
    let path = std::path::Path::new(&dir).join(name);
    Some(std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

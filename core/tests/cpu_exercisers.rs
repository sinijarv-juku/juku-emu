//! Runs the classic 8080 test programs under a minimal CP/M environment.
//!
//! The programs are not in the repository: they are other people's. Each test
//! downloads its program the first time (`program`), and is skipped, with a
//! note, if it cannot. 8080EXM takes ~24 billion cycles and is `#[ignore]`d by
//! default: run it with `cargo test -p juku --test cpu_exercisers -- --ignored`.

use std::path::Path;
use std::process::Command;

use juku::cpu::i8080::{Bus, Cpu};

/// Where the programs come from: the `cpu_tests` folder of superzazu/8080,
/// which took them from altairclone.com, at a fixed commit, so that a file
/// never changes.
const SOURCE: &str = "https://raw.githubusercontent.com/superzazu/8080/274ffd700b81baabea99b0963bc1260b67132185/cpu_tests";

/// Initial stack pointer; programs that need more set their own.
const INITIAL_SP: u16 = 0xFE00;

struct CpmBus {
    mem: Vec<u8>,
}

impl Bus for CpmBus {
    fn read(&mut self, addr: u16) -> u8 {
        self.mem[addr as usize]
    }
    fn write(&mut self, addr: u16, val: u8) {
        self.mem[addr as usize] = val;
    }
    fn input(&mut self, _port: u8) -> u8 {
        0xFF
    }
    fn output(&mut self, _port: u8, _val: u8) {}
}

struct RunResult {
    output: String,
    instructions: u64,
    cycles: u64,
}

/// The program `name`, `size` bytes long. It is downloaded with curl, which
/// macOS and Windows 10 and later have, into the repository's `target/` the
/// first time, and read from there after: `target/cpu_tests`, outside cargo's
/// `target/cargo`, so that `cargo clean` keeps it. None if it cannot be
/// downloaded.
fn program(name: &str, size: usize) -> Option<Vec<u8>> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/cpu_tests");
    let path = dir.join(name);
    if !path.exists() {
        std::fs::create_dir_all(&dir).unwrap();
        // A download cut short is left as .part, not taken for the program.
        let part = dir.join(format!("{name}.part"));
        let url = format!("{SOURCE}/{name}");
        let downloaded = Command::new("curl").args(["-fsSL", "--retry", "2", "-o"]).arg(&part).arg(&url).status();
        if !downloaded.is_ok_and(|s| s.success()) {
            eprintln!("skipping {name}: it could not be downloaded from {url}");
            return None;
        }
        std::fs::rename(&part, &path).unwrap();
    }

    let program = std::fs::read(&path).unwrap();
    assert_eq!(program.len(), size, "{} is not {name}: delete it to download it again", path.display());
    Some(program)
}

/// Loads a .COM file at 0100h and runs it until it jumps to 0000h (warm boot).
/// BDOS functions 2 (console output) and 9 (print string) are emulated by
/// trapping execution at 0005h.
///
/// As in the widely used reference harness (superzazu/8080), 0000h holds
/// `OUT 0` and 0005h holds `OUT 1 / RET`, so instruction and cycle totals are
/// directly comparable with its published figures.
fn run_com(name: &str, size: usize) -> Option<RunResult> {
    let program = program(name, size)?;

    let mut bus = CpmBus { mem: vec![0; 0x10000] };
    bus.mem[0x100..0x100 + program.len()].copy_from_slice(&program);
    bus.mem[0x0000..0x0002].copy_from_slice(&[0xD3, 0x00]); // OUT 0: exit
    bus.mem[0x0005..0x0008].copy_from_slice(&[0xD3, 0x01, 0xC9]); // OUT 1: BDOS, RET
    // LHLD 0006h must yield the top of memory; it reads the OUT 1 / RET
    // bytes as in the reference harness, so the stack sits just below C901h.

    let mut cpu = Cpu::new();
    cpu.pc = 0x100;
    cpu.sp = INITIAL_SP;

    let mut output = String::new();
    let mut instructions = 0u64;
    let mut cycles = 0u64;

    loop {
        let at_exit = cpu.pc == 0x0000;
        if cpu.pc == 0x0005 {
            match cpu.c {
                2 => output.push(cpu.e as char),
                9 => {
                    let mut addr = cpu.de();
                    while bus.mem[addr as usize] != b'$' {
                        output.push(bus.mem[addr as usize] as char);
                        addr = addr.wrapping_add(1);
                    }
                }
                _ => {}
            }
        }
        cycles += cpu.step(&mut bus) as u64;
        instructions += 1;
        if at_exit {
            break;
        }
        assert!(!cpu.halted, "{name}: CPU halted at {:04X}\n{output}", cpu.pc);
    }

    println!("{name}: {instructions} instructions, {cycles} cycles\n{output}");
    Some(RunResult { output, instructions, cycles })
}

#[test]
fn tst8080() {
    if let Some(r) = run_com("TST8080.COM", 1536) {
        assert!(r.output.contains("CPU IS OPERATIONAL"), "{}", r.output);
        assert_eq!(r.cycles, 4924);
    }
}

#[test]
fn prelim_8080pre() {
    if let Some(r) = run_com("8080PRE.COM", 1024) {
        assert!(r.output.contains("8080 Preliminary tests complete"), "{}", r.output);
        assert_eq!(r.cycles, 7817);
    }
}

#[test]
fn supersoft_cputest() {
    if let Some(r) = run_com("CPUTEST.COM", 19200) {
        assert!(r.output.contains("CPU TESTS OK"), "{}", r.output);
        assert_eq!(r.cycles, 255_653_383);
    }
}

#[test]
#[ignore = "takes ~24 billion emulated cycles; run with --ignored"]
fn exerciser_8080exm() {
    if let Some(r) = run_com("8080EXM.COM", 4608) {
        assert!(r.output.contains("Tests complete"), "{}", r.output);
        assert!(!r.output.contains("ERROR"), "{}", r.output);
        assert_eq!(r.instructions, 2_919_050_698);
        assert_eq!(r.cycles, 23_803_381_171);
    }
}

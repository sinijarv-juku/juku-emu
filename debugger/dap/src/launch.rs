//! Launching a program (spec §12.7): it is written into memory when the
//! machine waits for a key at a prompt, and started as the monitor or EKDOS
//! would start it.

use juku::Machine;
use juku::machine::CPU_HZ;

/// CONSTAT, which RomBios's key wait in TTI calls over and over (MD §14.2).
pub(crate) const CONSTAT: u16 = 0xFF98;
/// MONITOR, the monitor's warm start: where a program run from the monitor
/// returns to.
const MONITOR: u16 = 0xFFC4;
/// EKDOS's warm boot, through the JMP at 0000h.
const WARM_BOOT: u16 = 0x0000;
/// Under the monitor a program must end below its workspace (MD §3.3).
const MONITOR_WORKSPACE: u16 = 0xD300;
/// The longest wait for a safe point: ten seconds of the machine's time.
pub(crate) const MAX_WAIT: u64 = 10 * CPU_HZ;

/// What the launch request asked for.
pub(crate) struct Program {
    pub image: Vec<u8>,
    pub load: u16,
    pub entry: u16,
    /// Where the program ends; the monitor's or EKDOS's by default.
    pub exit: Option<u16>,
    /// The command tail, under EKDOS.
    pub args: String,
    pub stop_on_entry: bool,
    pub reset: bool,
    /// A snapshot to open first, through the host (§12.7).
    pub snapshot: Option<std::path::PathBuf>,
}

/// The machine waits for a key in RomBios: PC is at CONSTAT, called from
/// the ROM, which is TTI's key wait (D9D0h in RomBios 3.43m, D9CFh in 3.43
/// #0031) at the monitor's `*` prompt and EKDOS's `A>` alike. No interrupt
/// handler, disk command or BIOS call is half done there. A program polling
/// CONSTAT itself calls it from RAM, and is left alone.
pub(crate) fn at_safe_point(m: &Machine) -> bool {
    let c = m.cpu();
    let ret = u16::from_le_bytes([m.peek(c.sp), m.peek(c.sp.wrapping_add(1))]);
    c.pc == CONSTAT && matches!(m.memory_mode(), 1 | 2) && ret >= 0xD800
}

/// EKDOS is in memory and waiting: the JMPs at 0000h and 0005h are there,
/// and SP is in EKDOS's memory, below D400h (MD §3.4); the monitor's stack
/// is at D450h. Returns BDOS's address, from the JMP at 0005h.
fn ekdos(m: &Machine) -> Option<u16> {
    let ram = m.ram();
    (ram[0] == 0xC3 && ram[5] == 0xC3 && m.cpu().sp < 0xD400).then(|| u16::from_le_bytes([ram[6], ram[7]]))
}

/// The CCP's FCB for a file name: drive (0 for the default, 1 for A:), the
/// name in 8 bytes and the type in 3, blank padded, `*` filling the rest of
/// its part with `?`.
fn fcb(word: &str) -> [u8; 12] {
    let mut out = [b' '; 12];
    out[0] = 0;
    let mut word = word.as_bytes();
    if word.len() >= 2 && word[1] == b':' && word[0].is_ascii_uppercase() {
        out[0] = word[0] - b'A' + 1;
        word = &word[2..];
    }
    let (name, ext) = match word.iter().position(|&c| c == b'.') {
        Some(i) => (&word[..i], &word[i + 1..]),
        None => (word, &[][..]),
    };
    let (name_field, ext_field) = out[1..].split_at_mut(8);
    for (part, field) in [(name, name_field), (ext, ext_field)] {
        for (i, &c) in part.iter().take(field.len()).enumerate() {
            if c == b'*' {
                field[i..].fill(b'?');
                break;
            }
            field[i] = c;
        }
    }
    out
}

/// The command line as EKDOS's CCP leaves it for a program: the tail at
/// 0080h (its length, then the text from the blank before the first
/// argument, in capitals), and the first two arguments as FCBs at 005Ch and
/// 006Ch.
fn command_line(m: &mut Machine, args: &str) {
    let args = args.trim().to_ascii_uppercase();
    let tail = if args.is_empty() { String::new() } else { " ".to_string() + &args };
    let tail = &tail.as_bytes()[..tail.len().min(127)];
    m.poke(0x0080, tail.len() as u8);
    for (i, &c) in tail.iter().chain(&[0]).enumerate().take(127) {
        m.poke(0x0081 + i as u16, c);
    }
    for a in 0x005C..0x0080 {
        m.poke(a, 0);
    }
    let mut words = args.split_whitespace();
    for at in [0x005C, 0x006C] {
        let f = fcb(words.next().unwrap_or(""));
        for (i, &b) in f.iter().enumerate() {
            m.poke(at + i as u16, b);
        }
    }
}

/// Writes the program into RAM and starts it, at a safe point: under EKDOS
/// with SP at BDOS's address and the command line set, as the CCP does;
/// under the monitor on the monitor's stack. The address it returns to is
/// pushed. Returns that address.
pub(crate) fn start(m: &mut Machine, p: &Program) -> Result<u16, String> {
    let ekdos = ekdos(m);
    let limit = ekdos.unwrap_or(MONITOR_WORKSPACE) as usize;
    if p.load as usize + p.image.len() > limit {
        let under = if ekdos.is_some() { "EKDOS" } else { "the monitor's workspace" };
        return Err("The program does not fit below ".to_string() + under);
    }
    for (i, &b) in p.image.iter().enumerate() {
        m.poke(p.load + i as u16, b);
    }
    let (sp, exit) = match ekdos {
        Some(bdos) => (bdos, WARM_BOOT),
        None => (m.cpu().sp, MONITOR),
    };
    let exit = p.exit.unwrap_or(exit);
    if ekdos.is_some() {
        command_line(m, &p.args);
    }
    let sp = sp.wrapping_sub(2);
    let [lo, hi] = exit.to_le_bytes();
    m.poke(sp, lo);
    m.poke(sp.wrapping_add(1), hi);
    let c = m.cpu_mut();
    c.sp = sp;
    c.pc = p.entry;
    c.halted = false;
    if let Some(d) = m.debugger_mut() {
        // A new program: what the CPU ran before, and its calls, are gone.
        d.forget();
        d.note_code(p.entry);
    }
    Ok(exit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fcbs() {
        assert_eq!(&fcb("FOO.COM"), b"\0FOO     COM");
        assert_eq!(&fcb("B:LONGERNAME.TEXT"), b"\x02LONGERNATEX");
        assert_eq!(&fcb("*.ASM"), b"\0????????ASM");
        assert_eq!(&fcb("A*.*"), b"\0A??????????");
        assert_eq!(&fcb(""), b"\0           ");
    }
}

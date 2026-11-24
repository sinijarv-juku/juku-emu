//! A debugging session: DAP requests handled on the emulation thread, with
//! the machine (spec §12.5).

use juku::Machine;
use juku::cpu::disasm::{self, Instr};
use juku::debug::{Access, Resume, Stop};

use std::path::{Path, PathBuf};

use crate::base64;
use crate::expr;
use crate::info::{Info, normal_path};
use crate::launch::{self, CONSTAT, Program};
use crate::json::{Value, obj};
use crate::text::{dec, hex};

/// The one thread a client sees.
const THREAD: i64 = 1;

/// Variable references: the scopes, and F's flags.
const REGISTERS: i64 = 1;
const FLAGS: i64 = 2;
const MACHINE: i64 = 3;
const DEVICES: i64 = 4;
const SYMBOLS: i64 = 5;
/// The groups under Devices.
const PIC: i64 = 10;
const TIMERS: i64 = 11; // 11–13
const PPIS: i64 = 14; // 14–15
const FDC: i64 = 16;
const VIDEO: i64 = 17;
const PORTS: i64 = 18;

/// The ports on the board and the mouse card's (MD §4), for watchpoints.
const PORT_NAMES: [(u8, &str); 30] = [
    (0x00, "8259, A0 = 0"),
    (0x01, "8259, A0 = 1"),
    (0x04, "PPI 1 port A"),
    (0x05, "PPI 1 port B"),
    (0x06, "PPI 1 port C"),
    (0x07, "PPI 1 control"),
    (0x08, "8251 data"),
    (0x09, "8251 control"),
    (0x0C, "PPI 2 port A"),
    (0x0D, "PPI 2 port B"),
    (0x0E, "PPI 2 port C"),
    (0x0F, "PPI 2 control"),
    (0x10, "Timer 1 counter 0"),
    (0x11, "Timer 1 counter 1"),
    (0x12, "Timer 1 counter 2"),
    (0x13, "Timer 1 control"),
    (0x14, "Timer 2 counter 0"),
    (0x15, "Timer 2 counter 1"),
    (0x16, "Timer 2 counter 2"),
    (0x17, "Timer 2 control"),
    (0x18, "Timer 3 counter 0"),
    (0x19, "Timer 3 counter 1"),
    (0x1A, "Timer 3 counter 2"),
    (0x1B, "Timer 3 control"),
    (0x1C, "Floppy command and status"),
    (0x1D, "Floppy track"),
    (0x1E, "Floppy sector"),
    (0x1F, "Floppy data"),
    (0x80, "Mouse data"),
    (0x81, "Mouse control"),
];

/// The system's symbols (§12.6), loaded when their system is there.
const ROMBIOS_SYMBOLS: &str = include_str!("../../../resources/symbols/rombios.dbg");
const EKDOS_SYMBOLS: &str = include_str!("../../../resources/symbols/ekdos.dbg");

/// Most bytes one `readMemory` returns.
const MAX_READ: i64 = 0x10000;
/// Most instructions one `disassemble` returns.
const MAX_INSTRUCTIONS: i64 = 1024;

/// What a request left to do.
#[derive(PartialEq, Eq)]
pub(crate) enum Next {
    Continue,
    /// The client disconnected: the session is over.
    End,
}

pub(crate) struct Session {
    /// Messages for the client, ready to be written.
    pub(crate) out: Vec<u8>,
    seq: i64,
    /// The client has been told that the machine stopped, and not yet that
    /// it runs again.
    told_stopped: bool,
    /// The debug information loaded (§12.6).
    info: Info,
    /// Breakpoints, on addresses and on source lines.
    breakpoints: Vec<Breakpoint>,
    data_breakpoints: Vec<DataBreakpoint>,
    next_id: i64,
    /// What each address was last shown as in a disassembly: `START` of a
    /// line, `INSIDE` one, or 0 for not shown.
    shown: Vec<u8>,
    /// Extra zeros in the instruction references of stack frames (§12.5).
    respelled: usize,
    launch: Launch,
    /// Breakpoints of the session's own, for launching: a safe point to
    /// wait for, or the program's end.
    internal: Vec<u16>,
    /// Where the program launched was started: its outermost frame's
    /// routine.
    entry: Option<u16>,
    /// The reason to give for the next stop reported, when it is not the
    /// core's: `entry`, or `goto`.
    reason: Option<&'static str>,
    /// The launch settings, for a restart.
    launch_args: Value,
    /// A snapshot for the host to open (§12.7).
    request: Option<PathBuf>,
}

/// A launch request's progress (§12.7).
enum Launch {
    None,
    /// Asked for; it starts at `configurationDone`, once the breakpoints are
    /// set.
    Ready(Program),
    /// The host is opening the launch's snapshot.
    Opening(Program),
    /// Waiting for a safe point since the cycle given.
    Waiting(Program, u64),
    /// Started; it ends when PC reaches `exit`.
    Running { exit: u16 },
}

/// A breakpoint the client set: on an address, or on a source line, which
/// may make code in several places.
struct Breakpoint {
    id: i64,
    addrs: Vec<u16>,
    /// The source file and the line asked for, for a line's breakpoint.
    file: Option<PathBuf>,
    line: u32,
    when: When,
}

/// A watchpoint the client set: on memory or on a port.
struct DataBreakpoint {
    id: i64,
    target: Watched,
    /// `read`, `write` or `readWrite`.
    access: String,
    when: When,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Watched {
    /// Address and length.
    Memory(u16, u16),
    Port(u8),
}

/// When a breakpoint stops: its condition, its hit count and whether it is a
/// log point (§12.5).
#[derive(Default)]
struct When {
    condition: Option<String>,
    /// The comparison and the count: `>= 5`, `== 3`, `% 2`.
    hit: Option<(&'static str, u32)>,
    log: Option<String>,
    hits: u32,
}

/// What a breakpoint does when it is reached.
enum Fire {
    /// Nothing: its condition or hit count does not hold.
    Pass,
    Stop,
    /// A log point's message, without stopping.
    Log(String),
}

impl When {
    fn from(bp: &Value) -> When {
        let text = |key: &str| bp[key].as_str().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
        When { condition: text("condition"), hit: text("hitCondition").and_then(|h| hit_condition(&h)), log: text("logMessage"), hits: 0 }
    }

    /// Counts the hit if the condition holds. A condition that cannot be
    /// worked out stops, with the reason in the log.
    fn fire(&mut self, m: &Machine, info: &Info, log: &mut Vec<String>) -> Fire {
        if let Some(c) = &self.condition {
            match expr::eval(c, m, info) {
                Ok(0) => return Fire::Pass,
                Ok(_) => {}
                Err(e) => {
                    log.push("Breakpoint condition “".to_string() + c + "”: " + &e);
                    return Fire::Stop;
                }
            }
        }
        self.hits += 1;
        if let Some((op, n)) = self.hit {
            let h = self.hits;
            let holds = match op {
                ">" => h > n,
                "==" => h == n,
                "<" => h < n,
                "<=" => h <= n,
                "%" => n != 0 && h.is_multiple_of(n),
                _ => h >= n,
            };
            if !holds {
                return Fire::Pass;
            }
        }
        match &self.log {
            Some(message) => Fire::Log(interpolate(message, m, info)),
            None => Fire::Stop,
        }
    }
}

/// A hit count condition: a number, which the count must reach, or one after
/// `>`, `>=`, `==`, `=`, `<`, `<=` or `%` (every nth hit).
fn hit_condition(text: &str) -> Option<(&'static str, u32)> {
    let t = text.trim();
    for op in [">=", "<=", "==", ">", "<", "%", "="] {
        if let Some(n) = t.strip_prefix(op) {
            let op = if op == "=" { "==" } else { op };
            return n.trim().parse().ok().map(|n| (op, n));
        }
    }
    t.parse().ok().map(|n| (">=", n))
}

/// A log point's message with each `{expression}` worked out: in hex, or in
/// decimal after `,d` (`{A,d}`).
fn interpolate(message: &str, m: &Machine, info: &Info) -> String {
    let mut out = String::new();
    let mut rest = message;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        let inner = &rest[open + 1..open + close];
        let (text, decimal) = match inner.strip_suffix(",d") {
            Some(t) => (t, true),
            None => (inner, false),
        };
        out.push_str(&match expr::eval(text, m, info) {
            Ok(v) if decimal || !(0..=0xFFFF).contains(&v) => dec(v),
            Ok(v) if v <= 0xFF => hex8(v as u8),
            Ok(v) => hex16(v as u16),
            Err(e) => "{".to_string() + &e + "}",
        });
        rest = &rest[open + close + 1..];
    }
    out + rest
}

/// A value as `evaluate` shows it: in hex and in decimal.
fn show_value(v: i64) -> String {
    match v {
        0..=0xFF => hex8(v as u8) + " (" + &dec(v) + ")",
        0x100..=0xFFFF => hex16(v as u16) + " (" + &dec(v) + ")",
        _ => dec(v),
    }
}

/// The ROM is a RomBios 3.43 whose entries and variables are where
/// `rombios.dbg` says: JMPs or CALLs at its entry points, and the pointer
/// at FFA5h to the keyboard variables at D4BEh (MD §14.3).
fn rombios(m: &Machine) -> bool {
    let rom = m.rom();
    let at = |a: u16| rom[a as usize - 0xC000];
    [0xFF98, 0xFFC4, 0xFFD3, 0xFFD9].iter().all(|&a| matches!(at(a), 0xC3 | 0xCD)) && at(0xFFA5) == 0xBE && at(0xFFA6) == 0xD4
}

/// EKDOS is in memory: JMPs at 0000h and 0005h (MD §3.4).
fn ekdos(m: &Machine) -> bool {
    m.ram()[0] == 0xC3 && m.ram()[5] == 0xC3
}

const START: u8 = 1;
const INSIDE: u8 = 2;

/// The memory a reference names: what the CPU sees, RAM, or the ROM.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Space {
    Cpu,
    Ram,
    Rom,
}

impl Space {
    fn size(self) -> i64 {
        match self {
            Space::Cpu | Space::Ram => 0x10000,
            Space::Rom => 0x4000,
        }
    }
}

/// A memory reference: `0x1234` (what the CPU sees), or `ram:0x1234` or
/// `rom:0x1234`. An instruction reference is the first kind.
fn parse_reference(text: &str) -> Option<(Space, i64)> {
    let (space, addr) = match text.split_once(':') {
        Some(("cpu", a)) => (Space::Cpu, a),
        Some(("ram", a)) => (Space::Ram, a),
        Some(("rom", a)) => (Space::Rom, a),
        Some(_) => return None,
        None => (Space::Cpu, text),
    };
    let hex = addr.strip_prefix("0x").or_else(|| addr.strip_prefix("0X"))?;
    Some((space, i64::from_str_radix(hex, 16).ok()?))
}

fn reference(addr: u16) -> Value {
    ("0x".to_string() + &hex(addr as u32, 4)).into()
}

fn hex8(v: u8) -> String {
    hex(v as u32, 2) + "h"
}

fn hex16(v: u16) -> String {
    hex(v as u32, 4) + "h"
}

/// The registers: name, width in bits.
const REGISTER_NAMES: [(&str, u32); 15] = [
    ("A", 8),
    ("F", 8),
    ("B", 8),
    ("C", 8),
    ("D", 8),
    ("E", 8),
    ("H", 8),
    ("L", 8),
    ("BC", 16),
    ("DE", 16),
    ("HL", 16),
    ("SP", 16),
    ("PC", 16),
    ("INTE", 1),
    ("HALT", 1),
];

/// The flags in F: name and bit.
const FLAG_BITS: [(&str, u8); 5] = [("S", 7), ("Z", 6), ("AC", 4), ("P", 2), ("CY", 0)];

fn register(m: &Machine, name: &str) -> u16 {
    let c = m.cpu();
    match name {
        "A" => c.a as u16,
        "F" => c.flags() as u16,
        "B" => c.b as u16,
        "C" => c.c as u16,
        "D" => c.d as u16,
        "E" => c.e as u16,
        "H" => c.h as u16,
        "L" => c.l as u16,
        "BC" => c.bc(),
        "DE" => c.de(),
        "HL" => c.hl(),
        "SP" => c.sp,
        "PC" => c.pc,
        "INTE" => c.inte as u16,
        _ => c.halted as u16,
    }
}

fn set_register(m: &mut Machine, name: &str, v: u16) {
    let c = m.cpu_mut();
    let [hi, lo] = v.to_be_bytes();
    match name {
        "A" => c.a = lo,
        "F" => c.set_flags(lo),
        "B" => c.b = lo,
        "C" => c.c = lo,
        "D" => c.d = lo,
        "E" => c.e = lo,
        "H" => c.h = lo,
        "L" => c.l = lo,
        "BC" => (c.b, c.c) = (hi, lo),
        "DE" => (c.d, c.e) = (hi, lo),
        "HL" => (c.h, c.l) = (hi, lo),
        "SP" => c.sp = v,
        "PC" => c.pc = v,
        "INTE" => c.inte = v != 0,
        _ => c.halted = v != 0,
    }
}

fn register_value(bits: u32, v: u16) -> String {
    match bits {
        1 => dec(v as i64),
        8 => hex8(v as u8),
        _ => hex16(v),
    }
}

/// Where `disassemble` shows something.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Place {
    Instruction(i64),
    /// A byte shown as data: no instruction decoded backwards ends there,
    /// or one would run over a known start.
    Byte(i64),
    /// Before 0000h or past FFFFh.
    Outside,
}

/// The place at `at`, and where the next one starts. An instruction that
/// would run over a known start, where the CPU has started one
/// (`Debug::code_start`) or the debug information has a line's code, is
/// shown as a byte of data instead, so that the known start is a line of its
/// own: VS Code marks the current instruction only on a line that starts at
/// its address. What the debug information calls data is shown as bytes.
fn place_at(m: &Machine, info: &Info, at: i64) -> (Place, i64) {
    if !(0..0x10000).contains(&at) {
        return (Place::Outside, at + 1);
    }
    if info.data(at as u16) && !info.line_start(at as u16) {
        return (Place::Byte(at), at + 1);
    }
    let len = disasm::len(m.peek(at as u16)) as i64;
    let known = |a: i64| a < 0x10000 && (info.line_start(a as u16) || m.debugger().is_some_and(|d| d.code_start(a as u16)));
    if (at + 1..at + len).any(known) { (Place::Byte(at), at + 1) } else { (Place::Instruction(at), at + len) }
}

/// A source as DAP gives it: the file's name and its path.
fn source(path: &Path) -> Value {
    let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
    obj([("name", name.into()), ("path", path.to_string_lossy().into_owned().into())])
}

/// Whether the instruction with opcode `op` goes to its 16-bit operand: a
/// jump or a call, whose operand an EQU may name, as `CALL tto`.
fn goes_to(op: u8) -> bool {
    matches!(op, 0xC3 | 0xCB | 0xCD | 0xDD | 0xED | 0xFD) || op >= 0xC0 && matches!(op & 7, 2 | 4)
}

/// One line of disassembly. A place outside the address space is given the
/// address -1, which VS Code leaves out. With debug information, a label at
/// the address is its symbol, a line starting there its source, and a word
/// operand is named: a jump's or call's by a label or an EQU, any other's by
/// a label only.
fn instruction(m: &Machine, info: &Info, at: Place) -> Value {
    let mut line = match at {
        Place::Instruction(addr) => {
            let i = Instr::at(addr as u16, |a| m.peek(a));
            let bytes = [i.op, i.b1, i.b2][..i.size() as usize].iter().map(|&b| hex(b as u32, 2)).collect::<Vec<_>>();
            let text = i.text_with(|v| info.name(v, goes_to(i.op)).map(str::to_string));
            obj([("address", reference(addr as u16)), ("instructionBytes", bytes.join(" ").into()), ("instruction", text.into())])
        }
        Place::Byte(addr) => {
            let b = m.peek(addr as u16);
            obj([("address", reference(addr as u16)), ("instructionBytes", hex(b as u32, 2).into()), ("instruction", ("DB ".to_string() + &hex(b as u32, 2)).into())])
        }
        Place::Outside => return obj([("address", "-1".into()), ("instruction", "".into()), ("presentationHint", "invalid".into())]),
    };
    let (Place::Instruction(addr) | Place::Byte(addr)) = at else { unreachable!() };
    let addr = addr as u16;
    if let Some(label) = info.label(addr) {
        line.set("symbol", label.into());
    }
    if info.line_start(addr)
        && let Some((path, n)) = info.source_line(addr)
    {
        line.set("location", source(path));
        line.set("line", n.into());
    }
    line
}

/// The `count` places before `addr`, earliest first.
///
/// Decoding backwards is ambiguous, so this decodes forwards from far enough
/// back to hold `count` instructions of 3 bytes and some to spare, taking
/// the first start from which decoding lands on `addr`. Decoding cannot run
/// over a known start, so it always lands on one. When no start holds
/// enough, the longest run that lands is taken, and the bytes before it are
/// shown as data; before 0000h there is nothing.
fn places_before(m: &Machine, info: &Info, addr: i64, count: i64) -> Vec<Place> {
    let lands = |start: i64| {
        let mut at = start;
        let mut places = Vec::new();
        while at < addr {
            let (p, next) = place_at(m, info, at);
            places.push(p);
            at = next;
        }
        (at == addr).then_some(places)
    };
    let mut best: Vec<Place> = Vec::new();
    for start in (addr - 3 * count - 16).max(0)..addr {
        let Some(places) = lands(start) else {
            continue;
        };
        if places.len() as i64 >= count {
            return places[places.len() - count as usize..].to_vec();
        }
        if places.len() > best.len() {
            best = places;
        }
    }
    let first = match best.first() {
        Some(Place::Instruction(a) | Place::Byte(a)) => *a,
        _ => addr,
    };
    let bytes = (first - (count - best.len() as i64)).max(0)..first;
    let mut places = vec![Place::Outside; count as usize - best.len() - bytes.clone().count()];
    places.extend(bytes.map(Place::Byte));
    places.extend(best);
    places
}

impl Session {
    pub(crate) fn new() -> Session {
        Session {
            out: Vec::new(),
            seq: 0,
            told_stopped: false,
            info: Info::default(),
            breakpoints: Vec::new(),
            data_breakpoints: Vec::new(),
            next_id: 1,
            shown: vec![0; 0x10000],
            respelled: 0,
            launch: Launch::None,
            internal: Vec::new(),
            entry: None,
            reason: None,
            launch_args: Value::Null,
            request: None,
        }
    }

    /// Notes a line of disassembly sent to the client.
    fn shown(&mut self, m: &Machine, at: Place) {
        let (addr, size) = match at {
            Place::Instruction(a) => (a as usize, disasm::len(m.peek(a as u16)) as usize),
            Place::Byte(a) => (a as usize, 1),
            Place::Outside => return,
        };
        self.shown[addr] = START;
        let end = (addr + size).min(0x10000);
        self.shown[addr + 1..end].fill(INSIDE);
    }

    /// The instruction reference of a frame at `addr`. VS Code keeps the
    /// lines it has been sent for the whole session and looks a reference up
    /// again only if it has not seen it, so when `addr` was last shown inside
    /// a line (decoded before the CPU had started an instruction there), the
    /// reference gets one more leading zero: VS Code then fetches the lines
    /// around it anew, as they are decoded now.
    fn frame_reference(&mut self, addr: u16) -> Value {
        if self.shown[addr as usize] == INSIDE {
            self.respelled += 1;
        }
        ("0x".to_string() + &"0".repeat(self.respelled) + &hex(addr as u32, 4)).into()
    }

    fn send(&mut self, mut message: Value) {
        self.seq += 1;
        message.set("seq", self.seq.into());
        let body = message.to_json();
        self.out.extend_from_slice(b"Content-Length: ");
        self.out.extend_from_slice(dec(body.len() as i64).as_bytes());
        self.out.extend_from_slice(b"\r\n\r\n");
        self.out.extend_from_slice(body.as_bytes());
    }

    fn event(&mut self, event: &str, body: Value) {
        self.send(obj([("type", "event".into()), ("event", event.into()), ("body", body)]));
    }

    /// Moves a launch on, and reports a stop the client has not heard of.
    pub(crate) fn update(&mut self, m: &mut Machine) {
        self.advance_launch(m);
        let Some(stop) = m.debugger().and_then(|d| d.stopped()) else {
            return;
        };
        if self.told_stopped {
            return;
        }
        let mut ids = Vec::new();
        // A stop the session gives its own reason to (entry, goto) has been
        // through the breakpoints already, or never was a breakpoint's.
        if self.reason.is_none() && matches!(stop, Stop::Breakpoint | Stop::Watch(_)) {
            match self.breakpoints_fire(m, stop) {
                Some(fired) => ids = fired,
                None => {
                    // Not a stop after all: on with whatever step it broke into.
                    if let Some(d) = m.debugger_mut() {
                        d.go_on();
                    }
                    return;
                }
            }
        }
        self.told_stopped = true;
        let given = self.reason.take();
        let (reason, description) = match stop {
            _ if given == Some("entry") => ("entry", "Entry".to_string()),
            _ if given == Some("goto") => ("goto", "Jumped".to_string()),
            _ if given == Some("pause") => ("pause", "Paused".to_string()),
            Stop::Pause => ("pause", "Paused".to_string()),
            Stop::Step => ("step", "Step".to_string()),
            Stop::Breakpoint => {
                let on_line = self.breakpoints.iter().any(|b| b.file.is_some() && b.addrs.contains(&m.cpu().pc));
                (if on_line { "breakpoint" } else { "instruction breakpoint" }, "Breakpoint at ".to_string() + &hex16(m.cpu().pc))
            }
            Stop::Watch(hit) => {
                let access = match hit.access {
                    Access::Read => "Read ",
                    Access::Write => "Write ",
                    Access::In => "IN ",
                    Access::Out => "OUT ",
                };
                ("data breakpoint", access.to_string() + &hex8(hit.value) + " at " + &hex16(hit.addr))
            }
        };
        let mut body = obj([
            ("reason", reason.into()),
            ("description", description.into()),
            ("threadId", THREAD.into()),
            ("allThreadsStopped", true.into()),
        ]);
        if !ids.is_empty() {
            body.set("hitBreakpointIds", ids.into_iter().map(Value::from).collect::<Vec<_>>().into());
        }
        self.event("stopped", body);
    }

    /// The breakpoints or watchpoints a stop reached, with their conditions
    /// and hit counts worked out and log points' messages sent. The ids of
    /// those that stop, or None if none does. A stop no breakpoint of the
    /// client's accounts for stops.
    fn breakpoints_fire(&mut self, m: &Machine, stop: Stop) -> Option<Vec<i64>> {
        let pc = m.cpu().pc;
        let (info, mut log, mut ids, mut reached) = (&self.info, Vec::new(), Vec::new(), false);
        let mut fire = |id: i64, when: &mut When| {
            reached = true;
            match when.fire(m, info, &mut log) {
                Fire::Pass => {}
                Fire::Stop => ids.push(id),
                Fire::Log(text) => log.push(text),
            }
        };
        match stop {
            Stop::Watch(hit) => {
                for b in &mut self.data_breakpoints {
                    let ours = match (b.target, hit.access) {
                        (Watched::Memory(a, n), Access::Read) => b.access != "write" && hit.addr.wrapping_sub(a) < n,
                        (Watched::Memory(a, n), Access::Write) => b.access != "read" && hit.addr.wrapping_sub(a) < n,
                        (Watched::Port(p), Access::In) => b.access != "write" && hit.addr == p as u16,
                        (Watched::Port(p), Access::Out) => b.access != "read" && hit.addr == p as u16,
                        _ => false,
                    };
                    if ours {
                        fire(b.id, &mut b.when);
                    }
                }
            }
            _ => {
                for b in self.breakpoints.iter_mut().filter(|b| b.addrs.contains(&pc)) {
                    fire(b.id, &mut b.when);
                }
            }
        }
        for line in log {
            self.event("output", obj([("category", "console".into()), ("output", (line + "\n").into())]));
        }
        (!reached || !ids.is_empty()).then_some(ids)
    }

    /// The app's Pause.
    pub(crate) fn pause(&mut self, m: &mut Machine) {
        self.stop_here(m);
        self.update(m);
    }

    /// Pause. A stop not reported yet, at a breakpoint the session may go
    /// on from (a log point, a condition that does not hold), becomes the
    /// pause, rather than going on and losing it.
    fn stop_here(&mut self, m: &mut Machine) {
        let Some(d) = m.debugger_mut() else {
            return;
        };
        match d.stopped() {
            Some(_) if !self.told_stopped => self.reason = Some("pause"),
            _ => d.pause(),
        }
    }

    /// The app's Pause turned off.
    pub(crate) fn resume(&mut self, m: &mut Machine) {
        if m.debugger().is_some_and(|d| d.stopped().is_some()) {
            m.resume(Resume::Continue);
            self.told_stopped = false;
            self.event("continued", obj([("threadId", THREAD.into()), ("allThreadsContinued", true.into())]));
        }
    }

    /// Handles a message from the client.
    pub(crate) fn handle(&mut self, message: &Value, m: &mut Machine) -> Next {
        if message["type"].as_str() != Some("request") {
            return Next::Continue;
        }
        let command = message["command"].as_str().unwrap_or("");
        let args = &message["arguments"];
        let result = self.request(command, args, m);
        let ok = result.is_ok();
        let mut response = obj([
            ("type", "response".into()),
            ("request_seq", message["seq"].clone()),
            ("command", command.into()),
            ("success", result.is_ok().into()),
        ]);
        match result {
            Ok(body) => response.set("body", body),
            Err(text) => response.set("message", text.into()),
        }
        self.send(response);

        match command {
            // Breakpoints come after it: the debug information is loaded.
            "attach" | "launch" if ok => self.event("initialized", obj([])),
            "disconnect" => return Next::End,
            _ => self.update(m),
        }
        Next::Continue
    }

    fn request(&mut self, command: &str, args: &Value, m: &mut Machine) -> Result<Value, String> {
        match command {
            "initialize" => Ok(obj([
                ("supportsConfigurationDoneRequest", true.into()),
                ("supportsDisassembleRequest", true.into()),
                ("supportsSteppingGranularity", true.into()),
                ("supportsInstructionBreakpoints", true.into()),
                ("supportsReadMemoryRequest", true.into()),
                ("supportsWriteMemoryRequest", true.into()),
                ("supportsSetVariable", true.into()),
                ("supportsEvaluateForHovers", true.into()),
                ("supportsConditionalBreakpoints", true.into()),
                ("supportsHitConditionalBreakpoints", true.into()),
                ("supportsLogPoints", true.into()),
                ("supportsDataBreakpoints", true.into()),
                ("supportsDataBreakpointBytes", true.into()),
                ("supportsGotoTargetsRequest", true.into()),
                ("supportsRestartRequest", true.into()),
            ])),
            "attach" => {
                self.attach(args)?;
                self.settings(args, m);
                Ok(obj([]))
            }
            "configurationDone" => {
                self.begin_launch(m);
                Ok(obj([]))
            }
            "launch" => self.launch(args, m),
            "restart" => self.restart(args, m),
            "disconnect" => Ok(obj([])),
            "threads" => Ok(obj([("threads", vec![obj([("id", THREAD.into()), ("name", "8080".into())])].into())])),
            "pause" => {
                self.stop_here(m);
                Ok(obj([]))
            }
            "continue" => {
                self.go(m, Resume::Continue);
                Ok(obj([("allThreadsContinued", true.into())]))
            }
            "next" | "stepIn" => self.step(m, command == "stepIn", args["granularity"].as_str() == Some("instruction")),
            "stepOut" => self.step_out(m),
            "stackTrace" => Ok(self.stack_trace(args, m)),
            "scopes" => Ok(self.scopes()),
            "variables" => Ok(obj([("variables", self.variables(args["variablesReference"].as_i64(), m).into())])),
            "evaluate" => self.evaluate(args, m),
            "dataBreakpointInfo" => Ok(self.data_breakpoint_info(args, m)),
            "setDataBreakpoints" => Ok(self.set_data_breakpoints(args, m)),
            "gotoTargets" => Ok(self.goto_targets(args)),
            "goto" => self.goto(args, m),
            "setVariable" => self.set_variable(args, m),
            "setBreakpoints" => Ok(self.set_breakpoints(args, m)),
            "setExceptionBreakpoints" => Ok(obj([])),
            "setInstructionBreakpoints" => Ok(self.set_instruction_breakpoints(args, m)),
            "readMemory" => self.read_memory(args, m),
            "writeMemory" => self.write_memory(args, m),
            "disassemble" => self.disassemble(args, m),
            _ => Err(command.to_string() + " is not supported"),
        }
    }

    fn go(&mut self, m: &mut Machine, how: Resume) {
        m.resume(how);
        self.told_stopped = false;
    }

    /// Loads the debug information named in `debugInfo`: a path, or a list
    /// of them.
    fn attach(&mut self, args: &Value) -> Result<Value, String> {
        let paths: Vec<&str> = match &args["debugInfo"] {
            Value::String(path) => vec![path.as_str()],
            list => list.as_array().iter().filter_map(Value::as_str).collect(),
        };
        for path in paths {
            self.info.load(Path::new(path))?;
        }
        Ok(obj([]))
    }

    /// The settings both attach and launch take, and the system's symbols,
    /// unless `systemSymbols` is false: RomBios's for a RomBios ROM,
    /// EKDOS's when it is in memory.
    fn settings(&mut self, args: &Value, m: &mut Machine) {
        if let Some(d) = m.debugger_mut() {
            d.set_step_into_interrupts(args["stepIntoInterrupts"].as_bool() == Some(true));
        }
        if args["systemSymbols"].as_bool() != Some(false) {
            let folder = Path::new(".");
            if rombios(m) {
                let _ = self.info.load_text(ROMBIOS_SYMBOLS, folder, "rombios.dbg");
            }
            if ekdos(m) {
                let _ = self.info.load_text(EKDOS_SYMBOLS, folder, "ekdos.dbg");
            }
        }
    }

    /// Reads the program and its debug information, which is `debugInfo`,
    /// or else the `.dbg` file beside the program if there is one. The load
    /// address and the entry are the launch settings', else the debug
    /// information's, else 0100h.
    fn launch(&mut self, args: &Value, m: &mut Machine) -> Result<Value, String> {
        self.launch_args = args.clone();
        let path = args["program"].as_str().ok_or("The launch settings name no program")?;
        let image = std::fs::read(path).map_err(|_| path.to_string() + " could not be read")?;
        if args["debugInfo"].is_null() {
            let dbg = Path::new(path).with_extension("dbg");
            if dbg.exists() {
                self.info.load(&dbg)?;
            }
        } else {
            self.attach(args)?;
        }
        self.settings(args, m);
        let address = |key: &str| args[key].as_i64().and_then(|v| u16::try_from(v).ok());
        let load = address("load").or(self.info.load_address()).unwrap_or(0x0100);
        let entry = address("entry").or(self.info.entry()).unwrap_or(load);
        if load as usize + image.len() > 0x10000 {
            return Err(path.to_string() + " does not fit in memory");
        }
        self.launch = Launch::Ready(Program {
            image,
            load,
            entry,
            exit: address("exit"),
            args: args["args"].as_str().unwrap_or("").to_string(),
            stop_on_entry: args["stopOnEntry"].as_bool() == Some(true),
            reset: args["reset"].as_bool() == Some(true),
            snapshot: args["snapshot"].as_str().map(PathBuf::from),
        });
        Ok(obj([]))
    }

    /// Restart: the launch again, with the settings the client sends, or
    /// those it had, so a program assembled again is read again. The machine
    /// gets back to a prompt by opening the snapshot again or power cycling;
    /// a launch with neither cannot restart, as the program runs where no
    /// prompt comes. Line breakpoints are found again in the new debug
    /// information.
    fn restart(&mut self, args: &Value, m: &mut Machine) -> Result<Value, String> {
        let settings = if args["arguments"].is_null() { self.launch_args.clone() } else { args["arguments"].clone() };
        if settings["request"].as_str() == Some("attach") || settings["program"].is_null() {
            return Err("Only a launched program restarts".into());
        }
        if settings["snapshot"].is_null() && settings["reset"].as_bool() != Some(true) {
            return Err("A restart needs a snapshot or reset in the launch settings, to get back to a prompt".into());
        }
        self.info = Info::default();
        self.launch(&settings, m)?;
        for b in &mut self.breakpoints {
            if let Some(file) = &b.file {
                b.addrs = self.info.breakpoint(file, b.line).map_or(Vec::new(), |(_, addrs)| addrs);
            }
        }
        self.internal.clear();
        self.sync_breakpoints(m);
        self.entry = None;
        self.begin_launch(m);
        Ok(obj([]))
    }

    /// The host has opened the launch's snapshot, or could not.
    pub(crate) fn snapshot_opened(&mut self, m: &mut Machine, opened: bool) {
        let Launch::Opening(p) = std::mem::replace(&mut self.launch, Launch::None) else {
            return;
        };
        if opened {
            self.wait_for_prompt(m, p);
        } else {
            self.fail_launch("The snapshot was not opened: the program was not started.");
        }
    }

    /// The snapshot the session asks the host to open, taken.
    pub(crate) fn take_request(&mut self) -> Option<PathBuf> {
        self.request.take()
    }

    /// The breakpoints are set: the launch waits for a safe point, with the
    /// machine running, power cycled first if asked.
    fn begin_launch(&mut self, m: &mut Machine) {
        let Launch::Ready(p) = std::mem::replace(&mut self.launch, Launch::None) else {
            return;
        };
        if let Some(path) = &p.snapshot {
            self.request = Some(path.clone());
            self.launch = Launch::Opening(p);
            return;
        }
        if p.reset {
            m.reset(true);
        }
        self.wait_for_prompt(m, p);
    }

    fn wait_for_prompt(&mut self, m: &mut Machine, p: Program) {
        self.internal = vec![CONSTAT];
        self.sync_breakpoints(m);
        self.launch = Launch::Waiting(p, m.cycles());
        if m.debugger().is_some_and(|d| d.stopped().is_some()) {
            self.go(m, Resume::Continue);
        }
    }

    /// The session's own stops: at CONSTAT while waiting for a safe point,
    /// where the program is started once one comes, and at the program's
    /// end, which the client is told of. Neither is reported as a stop.
    fn advance_launch(&mut self, m: &mut Machine) {
        let pc = m.cpu().pc;
        let at_internal = m.debugger().and_then(|d| d.stopped()) == Some(Stop::Breakpoint)
            && self.internal.contains(&pc)
            && !self.breakpoints.iter().any(|b| b.addrs.contains(&pc));
        match std::mem::replace(&mut self.launch, Launch::None) {
            Launch::Waiting(p, since) if at_internal => {
                if !launch::at_safe_point(m) {
                    self.launch = Launch::Waiting(p, since);
                    m.resume(Resume::Continue);
                    return;
                }
                match launch::start(m, &p) {
                    Ok(exit) => {
                        self.entry = Some(p.entry);
                        self.internal = vec![exit];
                        self.launch = Launch::Running { exit };
                        m.resume(Resume::Continue);
                        if p.stop_on_entry
                            && let Some(d) = m.debugger_mut()
                        {
                            // Reported as the entry: a pause before its first
                            // instruction.
                            d.pause();
                            self.reason = Some("entry");
                        }
                    }
                    Err(why) => {
                        self.internal.clear();
                        m.resume(Resume::Continue);
                        self.fail_launch(&why);
                    }
                }
                self.sync_breakpoints(m);
            }
            Launch::Waiting(p, since) => {
                if m.cycles() - since > launch::MAX_WAIT {
                    self.internal.clear();
                    self.sync_breakpoints(m);
                    self.fail_launch("The machine is not at a prompt: the program was not started. Launch it at the monitor's * or EKDOS's A>.");
                } else {
                    self.launch = Launch::Waiting(p, since);
                }
            }
            Launch::Running { exit } if at_internal && pc == exit => {
                self.internal.clear();
                self.sync_breakpoints(m);
                self.entry = None;
                m.resume(Resume::Continue);
                self.event("exited", obj([("exitCode", 0.into())]));
                self.event("terminated", obj([]));
            }
            other => self.launch = other,
        }
    }

    fn fail_launch(&mut self, why: &str) {
        self.event("output", obj([("category", "important".into()), ("output", (why.to_string() + "\n").into())]));
        self.event("terminated", obj([]));
    }

    fn stopped(m: &Machine) -> Result<(), String> {
        match m.debugger().is_some_and(|d| d.stopped().is_some()) {
            true => Ok(()),
            false => Err("The machine is running".into()),
        }
    }

    /// `next` and `stepIn`. By line where PC is in a line of source and the
    /// client asks for no finer step: the step runs while PC is in that
    /// line's code, over calls for `next`, into them for `stepIn`. Otherwise
    /// by instruction: `stepIn` is one instruction, `next` goes over a call.
    fn step(&mut self, m: &mut Machine, into: bool, by_instruction: bool) -> Result<Value, String> {
        Self::stopped(m)?;
        let ranges = if by_instruction { Vec::new() } else { self.info.line_ranges(m.cpu().pc) };
        let how = match (ranges.is_empty(), into) {
            (false, _) => Resume::Range { ranges, into },
            (true, true) => Resume::Instruction,
            (true, false) => Resume::Over,
        };
        self.go(m, how);
        Ok(obj([]))
    }

    fn step_out(&mut self, m: &mut Machine) -> Result<Value, String> {
        Self::stopped(m)?;
        self.go(m, Resume::Out);
        Ok(obj([]))
    }

    /// Breakpoints on the lines of one source file, replacing those it had.
    /// A line that makes no code moves to the next one that does.
    fn set_breakpoints(&mut self, args: &Value, m: &mut Machine) -> Value {
        let file = normal_path(Path::new(args["source"]["path"].as_str().unwrap_or("")));
        self.breakpoints.retain(|b| b.file.as_ref() != Some(&file));
        let mut answers = Vec::new();
        for bp in args["breakpoints"].as_array() {
            let line = bp["line"].as_i64().and_then(|l| u32::try_from(l).ok()).unwrap_or(0);
            match self.info.breakpoint(&file, line) {
                Some((found, addrs)) => {
                    let id = self.next_id;
                    self.next_id += 1;
                    self.breakpoints.push(Breakpoint { id, addrs, file: Some(file.clone()), line, when: When::from(bp) });
                    answers.push(obj([("id", id.into()), ("verified", true.into()), ("line", found.into()), ("source", source(&file))]));
                }
                None => {
                    let why = if self.info.is_empty() { "No debug information is loaded" } else { "No code on or after this line" };
                    answers.push(obj([("verified", false.into()), ("message", why.into())]));
                }
            }
        }
        self.sync_breakpoints(m);
        obj([("breakpoints", answers.into())])
    }

    /// Puts the breakpoints into the core, the session's own with them.
    fn sync_breakpoints(&self, m: &mut Machine) {
        if let Some(d) = m.debugger_mut() {
            d.clear_breakpoints();
            for &a in self.breakpoints.iter().flat_map(|b| &b.addrs).chain(&self.internal) {
                d.set_breakpoint(a, true);
            }
        }
    }

    /// Frame 0 is where the CPU is; each frame of the shadow stack, innermost
    /// first, gives the one where its call returns to. A last frame, "…",
    /// stands for the calls made before the client connected; a program
    /// launched has none before it, and its outermost frame is its entry's.
    ///
    /// A frame is named by the routine it is in: the address its call went
    /// to, by its label where the debug information has one. The outermost
    /// known frame, whose routine was entered before the client connected,
    /// is named by where it is. A frame in a line of source points there.
    fn stack_trace(&mut self, args: &Value, m: &Machine) -> Value {
        let stack = m.debugger().map_or(&[][..], |d| d.stack());
        let n = stack.len();
        let mut frames = Vec::new();
        for k in 0..=n {
            let at = if k == 0 { m.cpu().pc } else { stack[n - k].ret };
            // A routine by the label at its entry (`tto` from RomBios's
            // symbols), or by its place in a program, or by an EQU naming
            // its address.
            let named = |a: u16| {
                let exact = self.info.label(a).or_else(|| self.info.name(a, true)).map(str::to_string);
                self.info.place_name(a).filter(|_| exact.is_none()).or(exact).unwrap_or_else(|| hex16(a))
            };
            let name = match stack.get(n.wrapping_sub(k + 1)) {
                Some(f) if f.interrupt => named(f.target) + " (interrupt)",
                Some(f) => named(f.target),
                None => match (self.entry, self.info.place_name(at)) {
                    (Some(entry), _) => named(entry),
                    (None, Some(place)) => "? at ".to_string() + &place,
                    (None, None) => "? at ".to_string() + &hex16(at),
                },
            };
            let mut frame = obj([
                ("id", k.into()),
                ("name", name.into()),
                ("line", 0.into()),
                ("column", 0.into()),
                ("instructionPointerReference", self.frame_reference(at)),
            ]);
            if let Some((path, line)) = self.info.source_line(at) {
                frame.set("source", source(path));
                frame.set("line", line.into());
                frame.set("column", 1.into());
            }
            frames.push(frame);
        }
        if self.entry.is_none() {
            frames.push(obj([
                ("id", (n + 1).into()),
                ("name", "…".into()),
                ("line", 0.into()),
                ("column", 0.into()),
                ("presentationHint", "label".into()),
            ]));
        }
        let total = frames.len();
        let start = (args["startFrame"].as_i64().unwrap_or(0).max(0) as usize).min(total);
        let levels = match args["levels"].as_i64().unwrap_or(0) {
            l if l > 0 => l as usize,
            _ => total,
        };
        let frames: Vec<Value> = frames.into_iter().skip(start).take(levels).collect();
        obj([("stackFrames", frames.into()), ("totalFrames", total.into())])
    }

    /// Registers, the devices, the program's symbols (not looked at until
    /// opened: there may be thousands) and the machine.
    fn scopes(&self) -> Value {
        let scope = |name: &str, reference: i64, expensive: bool| {
            obj([("name", name.into()), ("variablesReference", reference.into()), ("expensive", expensive.into())])
        };
        let mut registers = scope("Registers", REGISTERS, false);
        registers.set("presentationHint", "registers".into());
        let mut scopes = vec![registers, scope("Devices", DEVICES, false)];
        if !self.info.labels().is_empty() {
            scopes.push(scope("Symbols", SYMBOLS, true));
        }
        scopes.push(scope("Machine", MACHINE, false));
        obj([("scopes", scopes.into())])
    }

    fn variables(&self, which: Option<i64>, m: &mut Machine) -> Vec<Value> {
        let plain = |name: &str, value: String| obj([("name", name.into()), ("value", value.into()), ("variablesReference", 0.into())]);
        let group = |name: &str, value: &str, reference: i64| {
            obj([("name", name.into()), ("value", value.into()), ("variablesReference", reference.into())])
        };
        let bit = |b: bool| if b { "1" } else { "0" }.to_string();
        match which {
            Some(DEVICES) => vec![
                group("Interrupts", "8259", PIC),
                group("Timer 1", "8253: video", TIMERS),
                group("Timer 2", "8253: video", TIMERS + 1),
                group("Timer 3", "8253: serial, sound, T3.2", TIMERS + 2),
                group("PPI 1", "8255: memory mode, keyboard, sound, floppy", PPIS),
                group("PPI 2", "8255: keyboard, printer", PPIS + 1),
                group("Floppy", "1793", FDC),
                group("Video", "", VIDEO),
                group("Ports", "for watchpoints on IN and OUT", PORTS),
            ],
            Some(PIC) => {
                let [irr, isr, imr] = m.devices().pic;
                vec![plain("IRR", hex8(irr)), plain("ISR", hex8(isr)), plain("IMR", hex8(imr))]
            }
            Some(t) if (TIMERS..TIMERS + 3).contains(&t) => {
                let pit = m.devices().pits[(t - TIMERS) as usize];
                (0..3)
                    .map(|ch| {
                        let c = pit[ch];
                        let count = c.count.map_or("none".to_string(), |n| dec(n as i64));
                        let now = if c.bcd { hex(c.value as u32, 4) + " BCD" } else { dec(c.value as i64) };
                        let text = "mode ".to_string() + &dec(c.mode as i64) + ", count " + &count + ", now " + &now + ", OUT " + &bit(c.out) + ", gate " + &bit(c.gate);
                        plain(&("Counter ".to_string() + &dec(ch as i64)), text)
                    })
                    .collect()
            }
            Some(p) if (PPIS..PPIS + 2).contains(&p) => {
                let ports = m.devices().ppis[(p - PPIS) as usize];
                ["A", "B", "C"].iter().zip(ports).map(|(n, v)| plain(&("Port ".to_string() + n), hex8(v))).collect()
            }
            Some(FDC) => {
                let d = m.devices();
                let f = d.fdc;
                let drive = |n: usize| match d.drives[n] {
                    (cyl, true) => "track ".to_string() + &dec(cyl as i64),
                    (cyl, false) => "track ".to_string() + &dec(cyl as i64) + ", no disk",
                };
                let mut flags = Vec::new();
                for (on, name) in [(f.busy, "BUSY"), (f.drq, "DRQ"), (f.intrq, "INTRQ"), (f.motor, "motor")] {
                    if on {
                        flags.push(name);
                    }
                }
                vec![
                    plain("Command", hex8(f.command)),
                    plain("Track", dec(f.track as i64)),
                    plain("Sector", dec(f.sector as i64)),
                    plain("Data", hex8(f.data)),
                    plain("Lines", flags.join(", ")),
                    plain("Drive", (if f.select == 0 { "A:" } else { "B:" }).to_string() + ", side " + &dec(f.side as i64)),
                    plain("A:", drive(0)),
                    plain("B:", drive(1)),
                ]
            }
            Some(VIDEO) => {
                let f = m.frame();
                let (w, h, x, y) = (f.active_w, f.active_h, f.active_x, f.active_y);
                let since = m.cycles() - m.frame_cycle();
                let picture = dec(w as i64) + " bytes × " + &dec(h as i64) + " lines, from " + &dec(x as i64) + " µs and line " + &dec(y as i64);
                let beam = "line ".to_string() + &dec((since / 128) as i64) + ", " + &dec((since % 128 / 2) as i64) + " µs";
                vec![plain("Picture", picture), plain("Beam", beam), plain("Memory mode", dec(m.memory_mode() as i64))]
            }
            Some(PORTS) => PORT_NAMES.iter().map(|&(p, name)| plain(&hex8(p), name.to_string())).collect(),
            Some(SYMBOLS) => self
                .info
                .labels()
                .iter()
                .map(|(a, name)| {
                    let word = u16::from_le_bytes([m.peek(*a), m.peek(a.wrapping_add(1))]);
                    let value = hex16(*a) + ": " + &hex8(m.peek(*a)) + ", word " + &hex16(word);
                    let mut var = plain(name, value);
                    var.set("memoryReference", reference(*a));
                    var
                })
                .collect(),
            Some(REGISTERS) => REGISTER_NAMES
                .iter()
                .map(|&(name, bits)| {
                    let v = register(m, name);
                    let mut var = obj([
                        ("name", name.into()),
                        ("value", register_value(bits, v).into()),
                        ("variablesReference", if name == "F" { FLAGS } else { 0 }.into()),
                    ]);
                    if bits == 16 {
                        var.set("memoryReference", reference(v));
                    }
                    var
                })
                .collect(),
            Some(FLAGS) => {
                let f = m.cpu().flags();
                FLAG_BITS
                    .iter()
                    .map(|&(name, bit)| {
                        obj([("name", name.into()), ("value", if f >> bit & 1 != 0 { "1" } else { "0" }.into()), ("variablesReference", 0.into())])
                    })
                    .collect()
            }
            Some(MACHINE) => {
                let plain = |name: &str, value: String| obj([("name", name.into()), ("value", value.into()), ("variablesReference", 0.into())]);
                vec![
                    plain("Memory mode", dec(m.memory_mode() as i64)),
                    plain("Cycles", dec(m.cycles() as i64)),
                    plain("Frame", dec(m.frame().seq as i64)),
                ]
            }
            _ => Vec::new(),
        }
    }

    /// An expression's value (§12.5): for the Debug Console, watches and
    /// hovers. Hovering a symbol shows the byte and the word there too.
    fn evaluate(&self, args: &Value, m: &Machine) -> Result<Value, String> {
        let text = args["expression"].as_str().unwrap_or("").trim();
        let v = expr::eval(text, m, &self.info)?;
        let mut result = show_value(v);
        let hover = args["context"].as_str() == Some("hover");
        if hover && matches!(self.info.value(text), Ok(Some(_))) && (0..=0xFFFF).contains(&v) {
            let a = v as u16;
            let word = u16::from_le_bytes([m.peek(a), m.peek(a.wrapping_add(1))]);
            result = result + ": " + &hex8(m.peek(a)) + ", word " + &hex16(word);
        }
        let mut body = obj([("result", result.into()), ("variablesReference", 0.into())]);
        if (0..=0xFFFF).contains(&v) {
            body.set("memoryReference", reference(v as u16));
        }
        Ok(body)
    }

    /// What a watchpoint on a variable, or on an address, would watch. A
    /// symbol watches the bytes its line of data makes (a DW, two), else one;
    /// an entry under Ports watches IN and OUT there; an expression, with
    /// `bytes`, watches that many bytes at its value.
    fn data_breakpoint_info(&self, args: &Value, m: &Machine) -> Value {
        let name = args["name"].as_str().unwrap_or("");
        let none = |why: &str| obj([("dataId", Value::Null), ("description", why.into())]);
        let access = || Value::from(vec!["read".into(), "write".into(), "readWrite".into()]);
        let (id, description) = match args["variablesReference"].as_i64() {
            Some(PORTS) => match PORT_NAMES.iter().find(|(p, _)| hex8(*p) == name) {
                Some(&(p, what)) => ("port:".to_string() + &hex(p as u32, 2), "Port ".to_string() + name + " (" + what + ")"),
                None => return none("Not a port"),
            },
            Some(SYMBOLS) => match self.info.value(name) {
                Ok(Some(a)) => {
                    let n = self.info.size(name).unwrap_or(1).clamp(1, 256);
                    let id = "mem:".to_string() + &hex(a as u32, 4) + ":" + &hex(n as u32, 4);
                    (id, name.to_string() + " (" + &dec(n as i64) + " at " + &hex16(a) + ")")
                }
                _ => return none("Not a symbol"),
            },
            Some(_) => return none("Only symbols and ports can be watched"),
            None => match expr::eval(name, m, &self.info) {
                Ok(a) if (0..=0xFFFF).contains(&a) => {
                    let n = args["bytes"].as_i64().unwrap_or(1).clamp(1, 256);
                    let id = "mem:".to_string() + &hex(a as u32, 4) + ":" + &hex(n as u32, 4);
                    (id, dec(n) + " at " + &hex16(a as u16))
                }
                Ok(_) => return none("Not an address"),
                Err(e) => return none(&e),
            },
        };
        let mut body = obj([("dataId", id.into()), ("description", description.into()), ("canPersist", true.into())]);
        body.set("accessTypes", access());
        body
    }

    /// The watchpoints, replacing those there were.
    fn set_data_breakpoints(&mut self, args: &Value, m: &mut Machine) -> Value {
        self.data_breakpoints.clear();
        let mut answers = Vec::new();
        for bp in args["breakpoints"].as_array() {
            let id = bp["dataId"].as_str().unwrap_or("");
            let target = match id.split(':').collect::<Vec<_>>()[..] {
                ["mem", a, n] => u16::from_str_radix(a, 16).ok().zip(u16::from_str_radix(n, 16).ok()).map(|(a, n)| Watched::Memory(a, n)),
                ["port", p] => u8::from_str_radix(p, 16).ok().map(Watched::Port),
                _ => None,
            };
            let Some(target) = target else {
                answers.push(obj([("verified", false.into()), ("message", "Not something to watch".into())]));
                continue;
            };
            let id = self.next_id;
            self.next_id += 1;
            let access = bp["accessType"].as_str().unwrap_or("write").to_string();
            self.data_breakpoints.push(DataBreakpoint { id, target, access, when: When::from(bp) });
            answers.push(obj([("id", id.into()), ("verified", true.into())]));
        }
        if let Some(d) = m.debugger_mut() {
            d.clear_watches();
            for b in &self.data_breakpoints {
                let (read, write) = (b.access != "write", b.access != "read");
                match b.target {
                    Watched::Memory(a, n) => {
                        for i in 0..n {
                            let addr = a.wrapping_add(i);
                            if read {
                                d.set_watch(Access::Read, addr, true);
                            }
                            if write {
                                d.set_watch(Access::Write, addr, true);
                            }
                        }
                    }
                    Watched::Port(p) => {
                        if read {
                            d.set_watch(Access::In, p as u16, true);
                        }
                        if write {
                            d.set_watch(Access::Out, p as u16, true);
                        }
                    }
                }
            }
        }
        obj([("breakpoints", answers.into())])
    }

    /// Where Jump to Cursor can go on a line of source: where its code
    /// starts, or that of the next line with code.
    fn goto_targets(&self, args: &Value) -> Value {
        let file = normal_path(Path::new(args["source"]["path"].as_str().unwrap_or("")));
        let line = args["line"].as_i64().and_then(|l| u32::try_from(l).ok()).unwrap_or(0);
        let targets: Vec<Value> = match self.info.breakpoint(&file, line) {
            Some((found, addrs)) => addrs
                .iter()
                .map(|&a| {
                    obj([
                        ("id", (a as i64).into()),
                        ("label", ("Line ".to_string() + &dec(found as i64) + " at " + &hex16(a)).into()),
                        ("line", found.into()),
                        ("instructionPointerReference", reference(a)),
                    ])
                })
                .collect(),
            None => Vec::new(),
        };
        obj([("targets", targets.into())])
    }

    /// Jump to Cursor: PC set to a target, reported as a stop there.
    fn goto(&mut self, args: &Value, m: &mut Machine) -> Result<Value, String> {
        Self::stopped(m)?;
        let a = args["targetId"].as_i64().and_then(|a| u16::try_from(a).ok()).ok_or("Not a target")?;
        m.cpu_mut().pc = a;
        m.cpu_mut().halted = false;
        if let Some(d) = m.debugger_mut() {
            d.note_code(a);
        }
        self.reason = Some("goto");
        self.told_stopped = false;
        Ok(obj([]))
    }

    fn set_variable(&mut self, args: &Value, m: &mut Machine) -> Result<Value, String> {
        let name = args["name"].as_str().unwrap_or("");
        let text = args["value"].as_str().unwrap_or("");
        let v = expr::eval(text, m, &self.info)?;
        if v < 0 {
            return Err("A register holds no negative number".into());
        }
        match args["variablesReference"].as_i64() {
            Some(REGISTERS) => {
                let &(_, bits) = REGISTER_NAMES.iter().find(|(n, _)| *n == name).ok_or("No such register")?;
                if v >= 1 << bits {
                    return Err(name.to_string() + " holds " + &dec(bits as i64) + " bits");
                }
                let v = v as u32;
                set_register(m, name, v as u16);
                // The CPU goes on from there: an instruction starts at PC.
                if let Some(d) = m.debugger_mut().filter(|_| name == "PC") {
                    d.note_code(v as u16);
                }
                let mut body = obj([("value", register_value(bits, v as u16).into())]);
                if bits == 16 {
                    body.set("memoryReference", reference(v as u16));
                }
                Ok(body)
            }
            Some(FLAGS) => {
                let &(_, bit) = FLAG_BITS.iter().find(|(n, _)| *n == name).ok_or("No such flag")?;
                if v > 1 {
                    return Err("A flag is 0 or 1".into());
                }
                let f = m.cpu().flags() & !(1 << bit) | (v as u8) << bit;
                m.cpu_mut().set_flags(f);
                Ok(obj([("value", dec(v).into())]))
            }
            _ => Err("This cannot be changed".into()),
        }
    }

    fn set_instruction_breakpoints(&mut self, args: &Value, m: &mut Machine) -> Value {
        self.breakpoints.retain(|b| b.file.is_some());
        let mut answers = Vec::new();
        for bp in args["breakpoints"].as_array() {
            let found = bp["instructionReference"].as_str().and_then(parse_reference);
            let addr = found.map(|(_, a)| a + bp["offset"].as_i64().unwrap_or(0));
            match addr.filter(|a| (0..0x10000).contains(a)) {
                Some(a) => {
                    let id = self.next_id;
                    self.next_id += 1;
                    self.breakpoints.push(Breakpoint { id, addrs: vec![a as u16], file: None, line: 0, when: When::from(bp) });
                    answers.push(obj([("id", id.into()), ("verified", true.into()), ("instructionReference", reference(a as u16))]));
                }
                None => answers.push(obj([("verified", false.into()), ("message", "Not an address".into())])),
            }
        }
        self.sync_breakpoints(m);
        obj([("breakpoints", answers.into())])
    }

    fn read_memory(&self, args: &Value, m: &Machine) -> Result<Value, String> {
        let (space, base) = args["memoryReference"].as_str().and_then(parse_reference).ok_or("Bad memory reference")?;
        let start = base + args["offset"].as_i64().unwrap_or(0);
        let count = args["count"].as_i64().unwrap_or(0).clamp(0, MAX_READ);
        let from = start.clamp(0, space.size());
        let to = (start + count).clamp(0, space.size());
        let bytes: Vec<u8> = (from..to)
            .map(|a| match space {
                Space::Cpu => m.peek(a as u16),
                Space::Ram => m.ram()[a as usize],
                Space::Rom => m.rom()[a as usize],
            })
            .collect();
        let unreadable = count - bytes.len() as i64;
        let address = match space {
            Space::Cpu => "0x".to_string() + &hex(from as u32, 4),
            Space::Ram => "ram:0x".to_string() + &hex(from as u32, 4),
            Space::Rom => "rom:0x".to_string() + &hex(from as u32, 4),
        };
        Ok(obj([("address", address.into()), ("data", base64::encode(&bytes).into()), ("unreadableBytes", unreadable.into())]))
    }

    fn write_memory(&self, args: &Value, m: &mut Machine) -> Result<Value, String> {
        let (space, base) = args["memoryReference"].as_str().and_then(parse_reference).ok_or("Bad memory reference")?;
        let start = base + args["offset"].as_i64().unwrap_or(0);
        let data = args["data"].as_str().and_then(base64::decode).ok_or("Bad data")?;
        if space == Space::Rom {
            return Err("The ROM cannot be written".into());
        }
        if start < 0 || start + data.len() as i64 > space.size() {
            return Err("Outside the address space".into());
        }
        for (i, &v) in data.iter().enumerate() {
            let a = (start + i as i64) as u16;
            match space {
                Space::Cpu => m.store(a, v),
                _ => m.poke(a, v),
            }
            if let Some(d) = m.debugger_mut() {
                d.forget_code(a);
            }
        }
        Ok(obj([("bytesWritten", data.len().into())]))
    }

    fn disassemble(&mut self, args: &Value, m: &Machine) -> Result<Value, String> {
        let (space, base) = args["memoryReference"].as_str().and_then(parse_reference).ok_or("Bad memory reference")?;
        if space != Space::Cpu {
            return Err("Only what the CPU sees is disassembled".into());
        }
        let base = base + args["offset"].as_i64().unwrap_or(0);
        let first = args["instructionOffset"].as_i64().unwrap_or(0);
        let count = args["instructionCount"].as_i64().unwrap_or(0).clamp(0, MAX_INSTRUCTIONS);

        // Instruction 0 is the one at `base`. Those before it come first
        // when `first` is negative.
        let mut places = Vec::with_capacity(count as usize);
        let mut at = base;
        if first < 0 {
            places = places_before(m, &self.info, base.clamp(0, 0x10000), (-first).min(MAX_INSTRUCTIONS));
            places.truncate(count as usize);
        } else {
            for _ in 0..first.min(MAX_INSTRUCTIONS) {
                at = place_at(m, &self.info, at).1;
            }
        }
        while (places.len() as i64) < count {
            let (p, next) = place_at(m, &self.info, at);
            places.push(p);
            at = next;
        }
        for &p in &places {
            self.shown(m, p);
        }
        let instructions: Vec<Value> = places.into_iter().map(|p| instruction(m, &self.info, p)).collect();
        Ok(obj([("instructions", instructions.into())]))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references() {
        assert_eq!(parse_reference("0x00FF"), Some((Space::Cpu, 0xFF)));
        assert_eq!(parse_reference("rom:0x3FFF"), Some((Space::Rom, 0x3FFF)));
        assert_eq!(parse_reference("ram:0x10"), Some((Space::Ram, 0x10)));
        assert_eq!(parse_reference("disk:0x10"), None);
        assert_eq!(parse_reference("1234"), None);
    }

    fn machine() -> Machine {
        let rom = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/roms/ekta37.bin")).unwrap();
        Machine::new(&rom).unwrap()
    }

    #[test]
    fn disassembly_backwards() {
        let mut m = machine();
        // NOPs, then LXI B,0000 at 8100h: decoding lands on 8103h from anywhere.
        m.poke(0x8100, 0x01);
        assert_eq!(places_before(&m, &Info::default(), 0x8103, 3), [Place::Instruction(0x80FE), Place::Instruction(0x80FF), Place::Instruction(0x8100)]);
        // A JMP at 80FEh that every start before it runs into, overshooting
        // 8100h: only 80FFh lands, and the bytes before it are data.
        m.poke(0x80FE, 0xC3);
        let want = [Place::Byte(0x80FC), Place::Byte(0x80FD), Place::Byte(0x80FE), Place::Instruction(0x80FF)];
        assert_eq!(places_before(&m, &Info::default(), 0x8100, 4), want);
        // Before 0000h there is nothing.
        assert_eq!(places_before(&m, &Info::default(), 0x0001, 3)[..2], [Place::Outside, Place::Outside]);
    }

    #[test]
    fn disassembly_keeps_known_starts() {
        let mut m = machine();
        m.set_debugging(true);
        // LXI B at 8000h, but the CPU has started an instruction at 8001h:
        // the LXI is shown as a byte, and 8001h as an instruction.
        m.poke(0x8000, 0x01);
        assert_eq!(place_at(&m, &Info::default(), 0x8000), (Place::Instruction(0x8000), 0x8003));
        m.debugger_mut().unwrap().note_code(0x8001);
        assert_eq!(place_at(&m, &Info::default(), 0x8000), (Place::Byte(0x8000), 0x8001));
        assert_eq!(places_before(&m, &Info::default(), 0x8001, 3), [Place::Instruction(0x7FFE), Place::Instruction(0x7FFF), Place::Byte(0x8000)]);
    }

}

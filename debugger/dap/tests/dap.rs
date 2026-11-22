//! A DAP client talking to the server over a socket, with the machine
//! running at the monitor prompt (spec §12).

use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use juku::Machine;
use juku::cpu::disasm::Instr;
use juku::machine::CPU_HZ;
use juku_dap::json::{Value, obj};
use juku_dap::{Server, base64};

const ROM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/roms/ekta37.bin");
const DEMOS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../demos");

/// How long a reply may take.
const TIMEOUT: Duration = Duration::from_secs(5);

struct Rig {
    server: Server,
    m: Machine,
    stream: TcpStream,
    buf: Vec<u8>,
    seq: i64,
    events: Vec<Value>,
}

impl Rig {
    /// A machine at the `*` prompt and a client connected and attached.
    fn new() -> Rig {
        Rig::with(None, obj([]))
    }

    /// The same with an expansion ROM plugged in, attached with `attach`'s
    /// arguments.
    fn with(expansion: Option<&[u8]>, attach: Value) -> Rig {
        Rig::connect(machine(expansion), "attach", attach)
    }

    /// `m` with a client connected, attached or launching (`request`) with
    /// `args`, and its configuration done.
    fn connect(m: Machine, request: &str, args: Value) -> Rig {
        let server = Server::start(0).unwrap();
        let stream = TcpStream::connect(("127.0.0.1", server.port())).unwrap();
        stream.set_read_timeout(Some(Duration::from_millis(2))).unwrap();
        let mut rig = Rig { server, m, stream, buf: Vec::new(), seq: 0, events: Vec::new() };
        let start = Instant::now();
        while !rig.server.poll(&mut rig.m).connected {
            assert!(start.elapsed() < TIMEOUT, "not connected");
        }
        let caps = rig.request("initialize", obj([("adapterID", "juku".into())]));
        assert_eq!(caps["body"]["supportsDisassembleRequest"].as_bool(), Some(true));
        rig.ok(request, args);
        rig.event("initialized");
        rig.ok("configurationDone", obj([]));
        rig
    }

    /// Runs the machine a little, as the app's ticks do, and reads what
    /// came back.
    fn pump(&mut self) {
        self.server.poll(&mut self.m);
        self.server.run(&mut self.m, CPU_HZ / 100);
        // The host's part of a launch from a snapshot: open it.
        if let Some(path) = self.server.take_request() {
            let opened = std::fs::read(&path).ok().is_some_and(|file| self.m.load_snapshot(&file).is_ok());
            self.server.snapshot_opened(&mut self.m, opened);
        }
        let mut chunk = [0; 65536];
        match self.stream.read(&mut chunk) {
            Ok(0) => panic!("the server closed the connection"),
            Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) => panic!("{e}"),
        }
    }

    /// The next whole message in the buffer.
    fn take_message(&mut self) -> Option<Value> {
        let text = String::from_utf8_lossy(&self.buf).into_owned();
        let end = text.find("\r\n\r\n")?;
        let length: usize = text[..end].strip_prefix("Content-Length: ")?.parse().unwrap();
        let start = end + 4;
        if self.buf.len() < start + length {
            return None;
        }
        let body = String::from_utf8(self.buf[start..start + length].to_vec()).unwrap();
        self.buf.drain(..start + length);
        Some(Value::parse(&body).unwrap())
    }

    fn send(&mut self, command: &str, args: Value) -> i64 {
        self.seq += 1;
        let body = obj([("seq", self.seq.into()), ("type", "request".into()), ("command", command.into()), ("arguments", args)]).to_json();
        write!(self.stream, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.seq
    }

    fn request(&mut self, command: &str, args: Value) -> Value {
        let seq = self.send(command, args);
        let start = Instant::now();
        loop {
            while let Some(message) = self.take_message() {
                if message["type"].as_str() == Some("response") && message["request_seq"].as_i64() == Some(seq) {
                    return message;
                }
                self.events.push(message);
            }
            assert!(start.elapsed() < TIMEOUT, "no response to {command}");
            self.pump();
        }
    }

    /// A request that must succeed; its body.
    fn ok(&mut self, command: &str, args: Value) -> Value {
        let response = self.request(command, args);
        assert_eq!(response["success"].as_bool(), Some(true), "{command}: {}", response.to_json());
        response["body"].clone()
    }

    fn event(&mut self, name: &str) -> Value {
        let start = Instant::now();
        loop {
            if let Some(i) = self.events.iter().position(|e| e["event"].as_str() == Some(name)) {
                return self.events.remove(i)["body"].clone();
            }
            while let Some(message) = self.take_message() {
                self.events.push(message);
            }
            assert!(start.elapsed() < TIMEOUT, "no {name} event");
            self.pump();
        }
    }

    fn pause(&mut self) {
        self.ok("pause", obj([("threadId", 1.into())]));
        assert_eq!(self.event("stopped")["reason"].as_str(), Some("pause"));
    }

    /// Presses and lets go of the key at `column`, `row` (MD §6.3).
    fn key(&mut self, column: u8, row: u8) {
        let id = juku::keyboard::key_id(column, row);
        for down in [true, false] {
            self.m.set_key(id, down);
            for _ in 0..6 {
                self.pump();
            }
        }
    }

    /// The top frame's name, source file name and line.
    fn top(&mut self) -> (String, String, i64) {
        let trace = self.ok("stackTrace", obj([("threadId", 1.into())]));
        let f = trace["stackFrames"].as_array()[0].clone();
        let file = f["source"]["name"].as_str().unwrap_or("").to_string();
        (f["name"].as_str().unwrap().to_string(), file, f["line"].as_i64().unwrap())
    }

    /// Runs until the machine is stopped at a breakpoint the session has
    /// not looked at yet, as the server finds it in a tick.
    fn pump_until_stopped_at_log_point(&mut self) {
        for _ in 0..1000 {
            self.m.run_cycles(1000);
            if self.m.debugger().unwrap().stopped().is_some() {
                return;
            }
        }
        panic!("no stop at the log point");
    }

    fn register(&mut self, name: &str) -> String {
        let vars = self.ok("variables", obj([("variablesReference", 1.into())]));
        let var = vars["variables"].as_array().iter().find(|v| v["name"].as_str() == Some(name)).unwrap().clone();
        var["value"].as_str().unwrap().to_string()
    }
}

/// A machine at the `*` prompt, with an expansion ROM if given.
fn machine(expansion: Option<&[u8]>) -> Machine {
    let rom = std::fs::read(ROM).unwrap();
    let mut m = Machine::new(&rom).unwrap();
    if let Some(image) = expansion {
        m.insert_expansion(image).unwrap();
    }
    m.run_cycles(3 * CPU_HZ);
    m
}

/// Writes a program file for a launch; its path.
fn program(name: &str, image: &[u8]) -> String {
    let dir = std::env::temp_dir().join("juku-dap-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, image).unwrap();
    path.to_string_lossy().into_owned()
}

/// Prints "OK" through TTO and returns: from 0100h, `LXI H,msg / loop: MOV
/// A,M / ORA A / RZ / CALL TTO / INX H / JMP loop / msg: DB 'OK',0`.
const PRINT_OK: [u8; 16] = [0x21, 0x0D, 0x01, 0x7E, 0xB7, 0xC8, 0xCD, 0xD9, 0xFF, 0x23, 0xC3, 0x03, 0x01, b'O', b'K', 0];

fn hex(text: &str) -> u32 {
    u32::from_str_radix(text.trim_start_matches("0x").trim_end_matches('h'), 16).unwrap()
}

#[test]
fn pause_inspect_and_step() {
    let mut r = Rig::new();
    assert_eq!(r.ok("threads", obj([]))["threads"].as_array().len(), 1);
    r.pause();
    assert!(r.m.debugger().unwrap().stopped().is_some());

    let trace = r.ok("stackTrace", obj([("threadId", 1.into())]));
    let frames = trace["stackFrames"].as_array();
    assert!(frames.len() >= 2);
    assert_eq!(hex(frames[0]["instructionPointerReference"].as_str().unwrap()), r.m.cpu().pc as u32);
    assert_eq!(frames.last().unwrap()["name"].as_str(), Some("…"));
    let scopes = r.ok("scopes", obj([("frameId", 0.into())]));
    assert_eq!(scopes["scopes"].as_array()[0]["name"].as_str(), Some("Registers"));
    assert_eq!(hex(&r.register("PC")), r.m.cpu().pc as u32);
    assert_eq!(hex(&r.register("SP")), r.m.cpu().sp as u32);

    // One instruction: the cycle count moves on by its few cycles.
    let cycles = r.m.cycles();
    r.ok("stepIn", obj([("threadId", 1.into()), ("granularity", "instruction".into())]));
    assert_eq!(r.event("stopped")["reason"].as_str(), Some("step"));
    assert!((1..=40).contains(&(r.m.cycles() - cycles)), "{} cycles", r.m.cycles() - cycles);

    r.ok("continue", obj([("threadId", 1.into())]));
    r.pump();
    assert!(r.m.debugger().unwrap().stopped().is_none());
}

#[test]
fn change_registers_and_memory() {
    let mut r = Rig::new();
    r.pause();
    let body = r.ok("setVariable", obj([("variablesReference", 1.into()), ("name", "A".into()), ("value", "42h".into())]));
    assert_eq!(body["value"].as_str(), Some("42h"));
    assert_eq!(r.m.cpu().a, 0x42);
    r.ok("setVariable", obj([("variablesReference", 1.into()), ("name", "HL".into()), ("value", "0x1234".into())]));
    assert_eq!(r.m.cpu().hl(), 0x1234);
    r.ok("setVariable", obj([("variablesReference", 2.into()), ("name", "CY".into()), ("value", "1".into())]));
    assert!(r.m.cpu().cf);
    let refused = r.request("setVariable", obj([("variablesReference", 1.into()), ("name", "B".into()), ("value", "100h".into())]));
    assert_eq!(refused["success"].as_bool(), Some(false));

    r.ok("writeMemory", obj([("memoryReference", "0x8000".into()), ("data", base64::encode(&[1, 2, 3]).into())]));
    assert_eq!([r.m.peek(0x8000), r.m.peek(0x8001), r.m.peek(0x8002)], [1, 2, 3]);
    let read = r.ok("readMemory", obj([("memoryReference", "0x7FFF".into()), ("offset", 1.into()), ("count", 3.into())]));
    assert_eq!(base64::decode(read["data"].as_str().unwrap()).unwrap(), [1, 2, 3]);

    let rom = r.ok("readMemory", obj([("memoryReference", "rom:0x3FF0".into()), ("count", 32.into())]));
    assert_eq!(base64::decode(rom["data"].as_str().unwrap()).unwrap(), &r.m.rom()[0x3FF0..]);
    assert_eq!(rom["unreadableBytes"].as_i64(), Some(16));
    assert_eq!(r.request("writeMemory", obj([("memoryReference", "rom:0x0".into()), ("data", "AA==".into())]))["success"].as_bool(), Some(false));
}

#[test]
fn disassembly_around_pc() {
    let mut r = Rig::new();
    r.pause();
    let pc = r.m.cpu().pc;
    let at = format!("0x{pc:04X}");
    let body = r.ok("disassemble", obj([("memoryReference", at.into()), ("instructionOffset", (-8).into()), ("instructionCount", 16.into())]));
    let list = body["instructions"].as_array();
    assert_eq!(list.len(), 16);
    assert_eq!(hex(list[8]["address"].as_str().unwrap()), pc as u32);
    assert_eq!(list[8]["instruction"].as_str().unwrap(), Instr::at(pc, |a| r.m.peek(a)).to_string());
    // Each instruction starts where the one before ends.
    for pair in list.windows(2) {
        let (a, b) = (hex(pair[0]["address"].as_str().unwrap()), hex(pair[1]["address"].as_str().unwrap()));
        let len = pair[0]["instructionBytes"].as_str().unwrap_or_else(|| panic!("{}", body.to_json())).split(' ').count() as u32;
        assert_eq!(a + len, b);
    }

    // At 0000h there is nothing before.
    let body = r.ok("disassemble", obj([("memoryReference", "0x0002".into()), ("instructionOffset", (-4).into()), ("instructionCount", 6.into())]));
    let list = body["instructions"].as_array();
    assert_eq!(list.len(), 6);
    assert_eq!(list[0]["presentationHint"].as_str(), Some("invalid"));
}

/// VS Code keeps the lines it was sent. When the CPU stops inside a line
/// sent earlier, the frame's reference is spelled anew, so that VS Code asks
/// again, and the new lines start at the stop.
#[test]
fn stop_inside_a_line_shown_earlier() {
    let mut r = Rig::new();
    r.pause();
    // LXI B,0000 / NOP at 8000h, shown as such.
    r.ok("writeMemory", obj([("memoryReference", "0x8000".into()), ("data", base64::encode(&[0x01, 0, 0, 0]).into())]));
    let body = r.ok("disassemble", obj([("memoryReference", "0x8000".into()), ("instructionCount", 2.into())]));
    assert_eq!(body["instructions"].as_array()[0]["instruction"].as_str(), Some("LXI B,0000"));

    // PC into the LXI's operand.
    r.ok("setVariable", obj([("variablesReference", 1.into()), ("name", "PC".into()), ("value", "8001h".into())]));
    let frame = |r: &mut Rig| {
        let trace = r.ok("stackTrace", obj([("threadId", 1.into())]));
        trace["stackFrames"].as_array()[0]["instructionPointerReference"].as_str().unwrap().to_string()
    };
    let reference = frame(&mut r);
    assert_eq!(reference, "0x08001");
    let args = obj([("memoryReference", reference.as_str().into()), ("instructionOffset", (-2).into()), ("instructionCount", 4.into())]);
    let body = r.ok("disassemble", args);
    let lines: Vec<(u32, String)> = body["instructions"]
        .as_array()
        .iter()
        .map(|l| (hex(l["address"].as_str().unwrap()), l["instruction"].as_str().unwrap().to_string()))
        .collect();
    assert_eq!(lines[1], (0x8000, "DB 01".to_string()));
    assert_eq!(lines[2], (0x8001, "NOP".to_string()));
    // Now shown as a line of its own, the reference stays as it is.
    assert_eq!(frame(&mut r), "0x08001");
}

#[test]
fn instruction_breakpoint_in_the_rom() {
    let mut r = Rig::new();
    // CONSTAT, which the monitor polls at the prompt (MD §14.2).
    let body = r.ok("setInstructionBreakpoints", obj([("breakpoints", vec![obj([("instructionReference", "0xFF98".into())])].into())]));
    assert_eq!(body["breakpoints"].as_array()[0]["verified"].as_bool(), Some(true));
    let stopped = r.event("stopped");
    assert_eq!(stopped["reason"].as_str(), Some("instruction breakpoint"));
    assert_eq!(r.m.cpu().pc, 0xFF98);

    // Cleared, the machine runs on.
    r.ok("setInstructionBreakpoints", obj([("breakpoints", Vec::new().into())]));
    r.ok("continue", obj([("threadId", 1.into())]));
    for _ in 0..20 {
        r.pump();
    }
    assert!(r.m.debugger().unwrap().stopped().is_none());
}

#[test]
fn the_apps_pause() {
    let mut r = Rig::new();
    let state = r.server.set_paused(&mut r.m, true);
    assert!(state.stopped);
    assert_eq!(r.event("stopped")["reason"].as_str(), Some("pause"));
    let state = r.server.set_paused(&mut r.m, false);
    assert!(state.connected && !state.stopped);
    r.event("continued");
}

#[test]
fn disconnect_lets_the_machine_run() {
    let mut r = Rig::new();
    r.pause();
    r.ok("disconnect", obj([]));
    assert!(r.m.debugger().is_none());
    let cycles = r.m.cycles();
    r.m.run_cycles(CPU_HZ / 10);
    assert!(r.m.cycles() > cycles);
    assert!(!r.server.poll(&mut r.m).connected);
}

#[test]
fn one_client_at_a_time() {
    let mut r = Rig::new();
    let mut second = TcpStream::connect(("127.0.0.1", r.server.port())).unwrap();
    second.set_read_timeout(Some(Duration::from_millis(2))).unwrap();
    // Closed by the server: the end of the stream, or a reset.
    let start = Instant::now();
    loop {
        r.server.poll(&mut r.m);
        match second.read(&mut [0]) {
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Ok(n) => {
                assert_eq!(n, 0, "the second client is turned away");
                break;
            }
            Err(_) => break,
        }
        assert!(start.elapsed() < TIMEOUT, "the second client is still connected");
    }
    // The first one still works.
    r.pause();
}

#[test]
fn a_new_client_after_one_leaves() {
    let mut r = Rig::new();
    drop(std::mem::replace(&mut r.stream, TcpStream::connect(("127.0.0.1", r.server.port())).unwrap()));
    // The new connection may be turned away while the old one is still
    // being let go; try again until it is taken.
    let start = Instant::now();
    loop {
        r.server.poll(&mut r.m);
        let mut byte = [0];
        r.stream.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        match r.stream.read(&mut byte) {
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => break,
            _ => r.stream = TcpStream::connect(("127.0.0.1", r.server.port())).unwrap(),
        }
        assert!(start.elapsed() < TIMEOUT);
    }
    r.stream.set_read_timeout(Some(Duration::from_millis(2))).unwrap();
    r.seq = 0;
    r.ok("initialize", obj([]));
    r.ok("attach", obj([]));
    r.pause();
}

/// The bounce demo assembled, once, with its debug information written to
/// a file: the image and the file's path.
fn bounce_built() -> &'static (Vec<u8>, String) {
    static BUILT: OnceLock<(Vec<u8>, String)> = OnceLock::new();
    BUILT.get_or_init(|| {
        let program = juku_asm::assemble(Path::new(&format!("{DEMOS}/bounce.asm"))).unwrap();
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
        let (image, dbg) = (dir.join("bounce.bin"), dir.join("bounce.dbg"));
        std::fs::write(&image, &program.image).unwrap();
        std::fs::write(&dbg, program.debug_info(&dbg, &image)).unwrap();
        (program.image, dbg.to_str().unwrap().to_string())
    })
}

/// The number of the line of bounce.asm that starts with `text`, after its
/// indent: the tests find their lines by what is on them.
fn bounce_line(text: &str) -> i64 {
    let source = std::fs::read_to_string(format!("{DEMOS}/bounce.asm")).unwrap();
    let mut found = source.lines().enumerate().filter(|(_, l)| l.trim_start().starts_with(text));
    let (i, _) = found.next().unwrap_or_else(|| panic!("no line {text} in bounce.asm"));
    assert!(found.next().is_none(), "two lines {text} in bounce.asm");
    i as i64 + 1
}

/// The bounce demo, from its source (spec §12.6): a breakpoint on a
/// line, frames named by their labels, steps by line, over a call and out.
#[test]
fn bounce_by_source() {
    let (image, dbg) = bounce_built();
    let asm = format!("{DEMOS}/bounce.asm");
    let mut r = Rig::with(Some(image), obj([("debugInfo", vec![Value::from(dbg.as_str())].into())]));

    // `print:  mov a, m`; the comment above it moves to it.
    let print = bounce_line("print:");
    let lines = vec![obj([("line", (print - 1).into())])];
    let body = r.ok("setBreakpoints", obj([("source", obj([("path", asm.as_str().into())])), ("breakpoints", lines.into())]));
    let bp = body["breakpoints"].as_array()[0].clone();
    assert_eq!((bp["verified"].as_bool(), bp["line"].as_i64()), (Some(true), Some(print)));

    // A at the prompt starts the expansion ROM.
    r.key(5, 5);
    let stopped = r.event("stopped");
    assert_eq!(stopped["reason"].as_str(), Some("breakpoint"));
    assert_eq!(stopped["hitBreakpointIds"].as_array(), [bp["id"].clone()]);
    assert_eq!(r.top(), ("print".to_string(), "bounce.asm".to_string(), print));
    let trace = r.ok("stackTrace", obj([("threadId", 1.into())]));
    let caller = trace["stackFrames"].as_array()[1].clone();
    let after_call = bounce_line("lxi h, setup") + 2; // after start's call print
    assert_eq!(caller["line"].as_i64(), Some(after_call), "after CALL print, in START");
    assert!(caller["name"].as_str().unwrap().contains("start"), "{}", caller["name"].as_str().unwrap());

    // The disassembly names labels, calls and source lines.
    let pc = format!("0x{:04X}", r.m.cpu().pc);
    let body = r.ok("disassemble", obj([("memoryReference", pc.into()), ("instructionCount", 6.into())]));
    let list = body["instructions"].as_array();
    assert_eq!(list[0]["symbol"].as_str(), Some("print"));
    assert_eq!(list[0]["line"].as_i64(), Some(print));
    assert_eq!(list[0]["location"]["name"].as_str(), Some("bounce.asm"));
    assert_eq!(list[3]["instruction"].as_str(), Some("CALL TTO"));

    // By line: print, then ora a, rz, call TTO (stepped over), inx h.
    r.ok("setBreakpoints", obj([("source", obj([("path", asm.as_str().into())])), ("breakpoints", Vec::new().into())]));
    for want in [print + 1, print + 2, print + 3, print + 4] {
        r.ok("next", obj([("threadId", 1.into())]));
        assert_eq!(r.event("stopped")["reason"].as_str(), Some("step"));
        assert_eq!(r.top().2, want);
    }
    // Round the loop (JMP print) to CALL TTO, and into TTO: the ROM, with
    // no source.
    for want in [print + 5, print, print + 1, print + 2, print + 3] {
        r.ok("next", obj([("threadId", 1.into())]));
        r.event("stopped");
        assert_eq!(r.top().2, want);
    }
    r.ok("stepIn", obj([("threadId", 1.into())]));
    r.event("stopped");
    assert_eq!((r.m.cpu().pc, r.top().1), (0xFFD9, String::new()));
    // Out of TTO and out of print, back in start after the call.
    r.ok("stepOut", obj([("threadId", 1.into())]));
    r.event("stopped");
    assert_eq!(r.top().2, print + 4);
    r.ok("stepOut", obj([("threadId", 1.into())]));
    r.event("stopped");
    assert_eq!(r.top().2, after_call);
}

/// A program launched at the monitor's prompt: written to 0100h, stopped
/// at its entry with nothing before it on the stack, run to its RET into
/// MONITOR, which ends the session (spec §12.7).
#[test]
fn launch_at_the_monitor() {
    let path = program("print_ok.bin", &PRINT_OK);
    let args = obj([("program", path.as_str().into()), ("stopOnEntry", true.into())]);
    let mut r = Rig::connect(machine(None), "launch", args);
    assert_eq!(r.event("stopped")["reason"].as_str(), Some("entry"));
    assert_eq!(r.m.cpu().pc, 0x0100);
    assert_eq!((0..16).map(|i| r.m.peek(0x0100 + i)).collect::<Vec<_>>(), PRINT_OK);
    let trace = r.ok("stackTrace", obj([("threadId", 1.into())]));
    let frames = trace["stackFrames"].as_array();
    assert_eq!(frames.len(), 1, "nothing before the program");
    assert_eq!(frames[0]["name"].as_str(), Some("0100h"));
    // It returns to MONITOR, pushed for it.
    let sp = r.m.cpu().sp;
    assert_eq!([r.m.peek(sp), r.m.peek(sp + 1)], [0xC4, 0xFF]);

    r.ok("continue", obj([("threadId", 1.into())]));
    assert_eq!(r.event("exited")["exitCode"].as_i64(), Some(0));
    r.event("terminated");
    assert!(r.m.debugger().unwrap().stopped().is_none());
    assert!(!r.events.iter().any(|e| e["event"].as_str() == Some("stopped")), "the session's own stops are not reported");
}

/// With the machine busy in a program, there is no safe point: the launch
/// gives up after ten seconds of the machine's time, and says why.
#[test]
fn launch_with_no_prompt() {
    let mut m = machine(None);
    m.poke(0x8000, 0xC3); // JMP 8000h
    m.poke(0x8001, 0x00);
    m.poke(0x8002, 0x80);
    m.cpu_mut().pc = 0x8000;
    let path = program("never.bin", &PRINT_OK);
    let mut r = Rig::connect(m, "launch", obj([("program", path.as_str().into())]));
    for _ in 0..1100 {
        r.pump();
    }
    let out = r.event("output");
    assert_eq!(out["category"].as_str(), Some("important"));
    assert!(out["output"].as_str().unwrap().contains("not at a prompt"));
    r.event("terminated");
    assert_eq!(r.m.peek(0x0100), 0, "nothing was written");
}

/// Under EKDOS (`JUKU_TEST_DISKS`): the program gets the CCP's command line
/// and a stack below BDOS, returns to the warm boot, and EKDOS comes back to
/// its prompt.
#[test]
fn launch_under_ekdos() {
    let Some(dir) = std::env::var_os("JUKU_TEST_DISKS") else {
        eprintln!("skipping: JUKU_TEST_DISKS not set");
        return;
    };
    let disk = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core").join(dir).join("jukprog1.juk")).unwrap();
    let mut m = machine(None);
    m.insert_disk(0, disk, true).unwrap();
    // T, D, D boots EKDOS (MD §14.1).
    for (column, row) in [(4, 3), (6, 5), (6, 5)] {
        let id = juku::keyboard::key_id(column, row);
        for down in [true, false] {
            m.set_key(id, down);
            m.run_cycles(CPU_HZ * 6 / 100);
        }
        m.run_cycles(CPU_HZ);
    }
    let at_prompt = |m: &Machine| m.ram()[0] == 0xC3 && m.cpu().pc == 0xFF98 && m.cpu().sp < 0xD400;
    let mut waited = 0;
    while !at_prompt(&m) {
        m.run_cycles(1000);
        waited += 1000;
        assert!(waited < 30 * CPU_HZ, "EKDOS did not boot");
    }

    // MVI C,9 / LXI D,msg / CALL 5 / RET / msg: DB 'HI$'
    let image = [0x0E, 0x09, 0x11, 0x09, 0x01, 0xCD, 0x05, 0x00, 0xC9, b'H', b'I', b'$'];
    let path = program("hi.com", &image);
    let args = obj([("program", path.as_str().into()), ("args", "foo.txt b:bar".into()), ("stopOnEntry", true.into())]);
    let mut r = Rig::connect(m, "launch", args);
    assert_eq!(r.event("stopped")["reason"].as_str(), Some("entry"));
    let peek = |r: &Rig, a: u16, n: u16| (0..n).map(|i| r.m.peek(a + i)).collect::<Vec<u8>>();
    assert_eq!(peek(&r, 0x0080, 16), b"\x0E FOO.TXT B:BAR\0");
    assert_eq!(peek(&r, 0x005C, 12), b"\0FOO     TXT");
    assert_eq!(peek(&r, 0x006C, 12), b"\x02BAR        ");
    let bdos = u16::from_le_bytes([r.m.ram()[6], r.m.ram()[7]]);
    assert_eq!(r.m.cpu().sp, bdos - 2, "a stack below BDOS");
    assert_eq!([r.m.peek(bdos - 2), r.m.peek(bdos - 1)], [0, 0], "returning to the warm boot");

    r.ok("continue", obj([("threadId", 1.into())]));
    r.event("exited");
    r.event("terminated");
    let mut waited = 0;
    while !at_prompt(&r.m) {
        r.m.run_cycles(1000);
        waited += 1000;
        assert!(waited < 30 * CPU_HZ, "no A> after the program");
    }
}

/// A client that connects and leaves at once, as the VS Code extension does
/// to see whether anything listens, does not keep out the one after it.
#[test]
fn a_probe_and_then_a_client() {
    let mut m = machine(None);
    let server = Server::start(0).unwrap();
    let port = server.port();
    drop(TcpStream::connect(("127.0.0.1", port)).unwrap());
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    // Both wait in the listener's queue before the server looks.
    std::thread::sleep(Duration::from_millis(20));
    let mut server = server;
    let state = server.poll(&mut m);
    assert!(state.connected);
    stream.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    let mut byte = [0];
    match (&stream).read(&mut byte) {
        Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
        other => panic!("the client was turned away: {other:?}"),
    }
}

/// The bounce demo with its debug information, attached, and a breakpoint
/// on each of `lines` in bounce.asm with the settings given; the program
/// started with A.
fn bounce(lines: Vec<Value>) -> (Rig, String) {
    let (image, dbg) = bounce_built();
    let asm = format!("{DEMOS}/bounce.asm");
    let mut r = Rig::with(Some(image), obj([("debugInfo", dbg.as_str().into())]));
    r.ok("setBreakpoints", obj([("source", obj([("path", asm.as_str().into())])), ("breakpoints", lines.into())]));
    r.key(5, 5);
    (r, asm)
}

/// Conditions, hit counts and log points (spec §12.5). `print` sends
/// the setup string a character at a time; the line after `print:`, `ora a`,
/// sees each in A.
#[test]
fn conditions_hit_counts_and_log_points() {
    let ora = bounce_line("print:") + 1;
    let (mut r, _) = bounce(vec![obj([("line", ora.into()), ("condition", "A == 'J'".into())])]);
    assert_eq!(r.event("stopped")["reason"].as_str(), Some("breakpoint"));
    assert_eq!(r.m.cpu().a, b'J');

    // The setup string starts ESC, 'L', ESC: the third hit has ESC in A.
    let (mut r, _) = bounce(vec![obj([("line", ora.into()), ("hitCondition", "== 3".into())])]);
    r.event("stopped");
    assert_eq!(r.m.cpu().a, 0x1B);

    // A log point prints each character and stops nowhere; the breakpoint
    // on `loop:` after the printing does.
    let lp = bounce_line("loop:");
    let lines = vec![obj([("line", ora.into()), ("logMessage", "char {A} at {HL}".into())]), obj([("line", lp.into())])];
    let (mut r, _) = bounce(lines);
    let stopped = r.event("stopped");
    assert_eq!(r.top().2, lp, "{}", stopped.to_json());
    let logged: Vec<String> = r
        .events
        .iter()
        .filter(|e| e["event"].as_str() == Some("output"))
        .map(|e| e["body"]["output"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(logged.len(), 4 + 4 * 40 + 4 + 1, "the setup string and its 00h");
    assert!(logged[0].starts_with("char 1Bh at 01"), "{}", logged[0]);
    assert!(logged.iter().any(|l| l.starts_with("char 4Ah")), "a J");
}

/// A watchpoint on a variable written in a loop, with a condition: it stops
/// after the store that makes it 3.
#[test]
fn write_watchpoint_with_a_condition() {
    // LXI H,0 / loop: INX H / SHLD 0200h / JMP loop
    let image = [0x21, 0x00, 0x00, 0x23, 0x22, 0x00, 0x02, 0xC3, 0x03, 0x01];
    let path = program("count.bin", &image);
    let mut r = Rig::connect(machine(None), "launch", obj([("program", path.as_str().into()), ("stopOnEntry", true.into())]));
    r.event("stopped");
    let info = r.ok("dataBreakpointInfo", obj([("name", "200h".into()), ("asAddress", true.into()), ("bytes", 2.into())]));
    assert_eq!(info["dataId"].as_str(), Some("mem:0200:0002"));
    let bp = obj([("dataId", "mem:0200:0002".into()), ("accessType", "write".into()), ("condition", "w[200h] == 3".into())]);
    let set = r.ok("setDataBreakpoints", obj([("breakpoints", vec![bp].into())]));
    let id = set["breakpoints"].as_array()[0]["id"].clone();
    r.ok("continue", obj([("threadId", 1.into())]));
    let stopped = r.event("stopped");
    assert_eq!(stopped["reason"].as_str(), Some("data breakpoint"));
    assert_eq!(stopped["hitBreakpointIds"].as_array(), [id]);
    assert_eq!((r.m.cpu().pc, r.m.cpu().hl()), (0x0107, 3), "stopped after the SHLD that stored 3");
}

/// A watchpoint on a port: bounce writes the vertical blank to T2.1 (15h)
/// every frame.
#[test]
fn port_watchpoint() {
    let (mut r, _) = bounce(Vec::new());
    for _ in 0..50 {
        r.pump();
    }
    let ports = r.ok("variables", obj([("variablesReference", 18.into())]));
    assert!(ports["variables"].as_array().iter().any(|v| v["name"].as_str() == Some("15h")));
    let info = r.ok("dataBreakpointInfo", obj([("variablesReference", 18.into()), ("name", "15h".into())]));
    assert_eq!(info["dataId"].as_str(), Some("port:15"));
    r.ok("setDataBreakpoints", obj([("breakpoints", vec![obj([("dataId", "port:15".into()), ("accessType", "write".into())])].into())]));
    assert_eq!(r.event("stopped")["reason"].as_str(), Some("data breakpoint"));
    let at = r.m.cpu().pc - 2;
    assert_eq!(r.m.peek(at), 0xD3, "after an OUT");
    assert_eq!(r.m.peek(at + 1), 0x15);
}

/// Expressions, hovers, Jump to Cursor, the Devices and Symbols scopes, and
/// RomBios's symbols.
#[test]
fn expressions_scopes_and_jumps() {
    let print = bounce_line("print:");
    let (mut r, asm) = bounce(vec![obj([("line", print.into())])]);
    r.event("stopped");
    let hl = r.m.cpu().hl();
    let v = r.ok("evaluate", obj([("expression", "HL + 1".into()), ("context", "watch".into())]));
    let want = format!("{:04X}h ({})", hl + 1, hl + 1);
    assert_eq!(v["result"].as_str(), Some(want.as_str()));
    let hover = r.ok("evaluate", obj([("expression", "print".into()), ("context", "hover".into())]));
    assert_eq!(hover["result"].as_str(), Some("014Ah (330): 7Eh, word B77Eh"));
    // Names are case-sensitive, but found in another case: PRINT is print,
    // and conch RomBios's CONCH.
    for (text, want) in [("PRINT", "014Ah (330)"), ("conch", "D4BBh (54459)")] {
        let v = r.ok("evaluate", obj([("expression", text.into()), ("context", "watch".into())]));
        assert_eq!(v["result"].as_str(), Some(want), "{text}");
    }
    assert_eq!(r.request("evaluate", obj([("expression", "MOV".into())]))["success"].as_bool(), Some(false));

    // RomBios's names: CALL TTO in the program, and in RomBios itself.
    let at = format!("0x{:04X}", r.m.cpu().pc);
    let lines = r.ok("disassemble", obj([("memoryReference", at.into()), ("instructionCount", 5.into())]));
    assert_eq!(lines["instructions"].as_array()[3]["instruction"].as_str(), Some("CALL TTO"));
    let scopes = r.ok("scopes", obj([("frameId", 0.into())]));
    let names: Vec<&str> = scopes["scopes"].as_array().iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Registers", "Devices", "Symbols", "Machine"]);
    let symbols = r.ok("variables", obj([("variablesReference", 5.into())]));
    let named = |n: &str| symbols["variables"].as_array().iter().any(|v| v["name"].as_str() == Some(n));
    assert!(named("print") && named("TTO") && named("CONCH"));
    let devices = r.ok("variables", obj([("variablesReference", 4.into())]));
    assert_eq!(devices["variables"].as_array().len(), 9);
    let pic = r.ok("variables", obj([("variablesReference", 10.into())]));
    assert_eq!(pic["variables"].as_array()[2]["name"].as_str(), Some("IMR"));
    let timer = r.ok("variables", obj([("variablesReference", 12.into())]));
    assert!(timer["variables"].as_array()[1]["value"].as_str().unwrap().starts_with("mode "));

    // Jump to Cursor: from print to its INX H.
    let targets = r.ok("gotoTargets", obj([("source", obj([("path", asm.as_str().into())])), ("line", (print + 4).into())]));
    let target = targets["targets"].as_array()[0].clone();
    r.ok("goto", obj([("threadId", 1.into()), ("targetId", target["id"].clone())]));
    assert_eq!(r.event("stopped")["reason"].as_str(), Some("goto"));
    assert_eq!(r.top().2, print + 4);
}

/// The core's disassembler, which the server shows, writes each opcode as
/// juku-sdk's instruction table has it: `MVI A,34`, `JMP* 1234` for an
/// undocumented one.
#[test]
fn cpu_disassembler_is_the_sdks() {
    for (code, op) in juku_i8080::OPS.iter().enumerate() {
        let i = Instr { op: code as u8, b1: 0x34, b2: 0x12 };
        let mut operands: Vec<&str> = op.regs[..op.reg_count()].to_vec();
        match op.data {
            juku_i8080::Data::None => {}
            juku_i8080::Data::Byte => operands.push("34"),
            juku_i8080::Data::Word => operands.push("1234"),
        }
        let mut want = op.mnemonic.to_string() + if op.documented { "" } else { "*" };
        if !operands.is_empty() {
            want = want + " " + &operands.join(",");
        }
        assert_eq!((i.to_string(), i.size() as usize), (want, op.size()), "opcode {code:02X}");
    }
}

/// RomBios's and EKDOS's symbols, built into the server, are the names and
/// values of juku-sdk's rombios.inc and ekdos.inc (spec §12.6).
#[test]
fn system_symbols_are_the_sdks() {
    let symbols = concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/symbols");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    for (dbg, inc) in [("rombios.dbg", "rombios.inc"), ("ekdos.dbg", "ekdos.inc")] {
        let file = Value::parse(&std::fs::read_to_string(format!("{symbols}/{dbg}")).unwrap()).unwrap();
        let mut ours: Vec<(String, i64)> = file["labels"]
            .as_array()
            .iter()
            .map(|l| (l.as_array()[0].as_str().unwrap().to_string(), l.as_array()[1].as_i64().unwrap()))
            .collect();
        let source = dir.join(format!("{inc}.asm"));
        std::fs::write(&source, format!("        include '{inc}'\n        db 0\n")).unwrap();
        let program = juku_asm::assemble(&source).unwrap();
        let mut sdks: Vec<(String, i64)> = program
            .symbols()
            .lines()
            .map(|l| {
                let (name, value) = l.split_once(' ').unwrap();
                (name.to_string(), i64::from_str_radix(value, 16).unwrap())
            })
            .collect();
        ours.sort();
        sdks.sort();
        assert_eq!(ours, sdks, "{dbg} and juku-sdk's {inc}");
    }
}

/// A log point in the monitor's key wait, reached thousands of times a
/// second: the machine keeps its time, and a pause still stops it.
#[test]
fn busy_log_point() {
    let mut r = Rig::new();
    let bp = obj([("instructionReference", "0xFF98".into()), ("logMessage", "constat {A}".into())]);
    r.ok("setInstructionBreakpoints", obj([("breakpoints", vec![bp].into())]));
    let start = r.m.cycles();
    for _ in 0..100 {
        r.pump();
    }
    assert!(r.m.cycles() - start >= CPU_HZ, "{} cycles in 100 ticks of 20 000", r.m.cycles() - start);
    while let Some(message) = r.take_message() {
        r.events.push(message);
    }
    let logged = r.events.iter().filter(|e| e["event"].as_str() == Some("output")).count();
    assert!(logged > 3000, "{logged} lines");
    // The machine stops at the log point now and then: a pause that comes
    // then is not lost.
    r.pump_until_stopped_at_log_point();
    r.pause();
}

/// Launches from a snapshot taken at the prompt, and restarts: each run
/// starts from the same state, the machine's state at the entry the same
/// to the byte (spec §12.7).
#[test]
fn restart_from_a_snapshot() {
    let mut at_prompt = machine(None);
    let snapshot = program("prompt.jukusnap", &at_prompt.save_snapshot(None, &[]).unwrap());
    // The machine launched in is elsewhere: run on and a key typed.
    at_prompt.run_cycles(CPU_HZ);
    let mut m = at_prompt;
    let id = juku::keyboard::key_id(6, 5);
    m.set_key(id, true);
    m.run_cycles(CPU_HZ / 10);
    m.set_key(id, false);
    let path = program("print_ok_again.bin", &PRINT_OK);
    let args = obj([("program", path.as_str().into()), ("snapshot", snapshot.as_str().into()), ("stopOnEntry", true.into())]);
    let mut r = Rig::connect(m, "launch", args);
    assert_eq!(r.event("stopped")["reason"].as_str(), Some("entry"));
    let first = (r.m.cycles(), r.m.save_snapshot(None, &[]).unwrap());

    for _ in 0..2 {
        // Into the program, then back to the start.
        r.ok("next", obj([("threadId", 1.into())]));
        r.event("stopped");
        r.ok("restart", obj([]));
        assert_eq!(r.event("stopped")["reason"].as_str(), Some("entry"));
        assert_eq!(r.m.cycles(), first.0, "the same time");
        assert!(r.m.save_snapshot(None, &[]).unwrap() == first.1, "the same state");
    }
}

/// A restart needs a way back to a prompt; a snapshot that does not open
/// ends the launch with the reason.
#[test]
fn restarts_that_cannot_be() {
    let path = program("print_ok_3.bin", &PRINT_OK);
    let mut r = Rig::connect(machine(None), "launch", obj([("program", path.as_str().into()), ("stopOnEntry", true.into())]));
    r.event("stopped");
    let refused = r.request("restart", obj([]));
    assert_eq!(refused["success"].as_bool(), Some(false));
    assert!(refused["message"].as_str().unwrap().contains("snapshot or reset"));

    let args = obj([("program", path.as_str().into()), ("snapshot", "/nonexistent/x.jukusnap".into())]);
    let mut r = Rig::connect(machine(None), "launch", args);
    assert!(r.event("output")["output"].as_str().unwrap().contains("snapshot was not opened"));
    r.event("terminated");
}

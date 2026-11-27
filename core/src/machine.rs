//! The Juku E5104 machine: memory modes, I/O decode, device wiring and the
//! run loop.

use std::sync::Arc;

use crate::audio::{AudioRing, Speaker};
use crate::cpu::i8080::{Bus, Cpu};
use crate::debug::{Debug, Resume};
use crate::dev::fdc1793::{Fdc, FdcState};
use crate::dev::pic8259::Pic;
use crate::dev::pit8253::{CounterState, Pit};
use crate::dev::ppi8255::Ppi;
use crate::dev::usart8251::Usart;
use crate::drive_sound::DriveEvent;
use crate::expansion::{self, ExpansionError};
use crate::floppy::{Disk, DiskError, Drive};
use crate::keyboard::{self, CapsFollower, Keyboard};
use crate::mouse::Mouse;
use crate::snapshot::{self, Reader, Sections, SnapshotError, SnapshotWriter, Writer};
use crate::video::{self, Frame, Geometry, RASTER_BYTES, RASTER_SIZE, Raster, RasterState};

/// CPU clock, Hz.
pub const CPU_HZ: u64 = 2_000_000;
/// CPU cycles per scanline (64 µs).
pub const CYCLES_PER_LINE: u64 = 128;
/// CPU cycles per frame (313 lines, ≈ 49.92 Hz).
pub const CYCLES_PER_FRAME: u64 = 313 * CYCLES_PER_LINE;
/// Size of the system ROM (two 8 KB EPROMs).
pub const ROM_SIZE: usize = 0x4000;
pub const RAM_SIZE: usize = 0x10000;

// Layout of `System::mem`: RAM, ROM, the external window (the expansion ROM,
// FFh where there is none), then one page that swallows writes.
const PAGE: usize = 0x400;
const ROM_OFS: usize = RAM_SIZE;
const EXP_OFS: usize = ROM_OFS + ROM_SIZE;
const SINK_OFS: usize = EXP_OFS + expansion::WINDOW_SIZE;
const MEM_SIZE: usize = SINK_OFS + PAGE;

/// The key click (MD §14.6): unless CONCH bit 7 is set, the keyboard handler
/// sets the quiet volume and jumps into BLEEP with a 2 kHz tone. RomBios
/// 3.43m and 3.43 #0031 have it at ROM 125F, 3.43 #0035 at 1265, so it is
/// looked for in the chip that holds it.
const CLICK: [u8; 20] = [
    0x3A, 0xBB, 0xD4, // LDA CONCH
    0x07, 0xD8, // RLC / RC
    0x21, 0x01, 0x00, // LXI H,1
    0x11, 0xE8, 0x03, // LXI D,03E8h
    0xDB, 0x04, 0xE6, 0xEF, 0xD3, 0x04, // clear PA4
    0xC3, 0x97, 0x05, // JMP 0597h
];
/// The JMP that starts the tone, as an offset into `CLICK`. A RET there
/// silences the click and keeps the volume change.
const CLICK_JMP: usize = 17;
const RET: u8 = 0xC9;
/// The 2 KB EPROM chip holding the click.
const CLICK_CHIP: core::ops::Range<usize> = 0x1000..0x1800;
/// The ROM test's expected sum of `CLICK_CHIP`. No summed range covers it.
const CLICK_CHIP_SUM: usize = 0x0008;

/// PIT #1 is clocked at 1 MHz.
const PIT1: usize = 0;
const PIT2: usize = 1;
/// PIT #3: ch0 at 16/13 MHz (baud), ch1 at 2 MHz (sound), ch2 from the frame.
const PIT3: usize = 2;

/// IR input of the frame interrupt.
const IRQ_FRAME: u8 = 5;
/// IR input of the mouse card (bus INT6).
const IRQ_MOUSE: u8 = 6;

/// Longest CPU wait on the FDC data register: six revolutions (1.2 s). A
/// transfer command's first byte can come more than a revolution after the
/// command, when its sector's ID has just gone by, and a sector that is not
/// there ends the command, and with it the wait, after five revolutions and
/// the head settling. So this only stops a wait that would never end, where
/// the real board would hang; the access then completes.
const MAX_FDC_STALL: u64 = 6 * crate::floppy::REV_CYCLES;

/// When, in an instruction, an I/O port is read or written: in T2 of its
/// third machine cycle, 8 T-states after the instruction starts.
const IO_AT: u64 = 8;

/// When, in the instruction with opcode `op`, its `n`th memory write (from
/// 0) happens: in T2 of the machine cycle that writes, counted from the
/// start of the instruction. A second write comes 3 T-states after the
/// first.
fn write_at(op: u8, n: u8) -> u64 {
    let first = match op {
        0x34..=0x36 => 8,                     // INR M, DCR M, MVI M
        0x32 | 0x22 | 0xE3 => 11,             // STA, SHLD, XTHL
        0xC5 | 0xD5 | 0xE5 | 0xF5 => 6,       // PUSH
        0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => 6, // RST
        0xC4 | 0xCC | 0xCD | 0xD4 | 0xDC | 0xDD | 0xE4 | 0xEC | 0xED | 0xF4 | 0xFC | 0xFD => 12, // CALL
        _ => 5,                               // MOV M,r, STAX
    };
    first + 3 * n as u64
}

/// The devices' state, for the debugger (spec §12.5).
#[derive(Clone, Debug, Default)]
pub struct Devices {
    /// The 8259's IRR, ISR and IMR.
    pub pic: [u8; 3],
    /// PIT #1–#3, counters 0–2.
    pub pits: [[CounterState; 3]; 3],
    /// PPI #1 and #2, ports A, B and C.
    pub ppis: [[u8; 3]; 2],
    pub fdc: FdcState,
    /// Each drive's head position, and whether it holds a disk.
    pub drives: [(u8, bool); 2],
}

pub struct Machine {
    cpu: Cpu,
    sys: System,
    /// The debugger's state while a client is connected (spec §12.4).
    debug: Option<Box<Debug>>,
}

/// Everything but the CPU: the CPU's view of the world.
struct System {
    mem: Box<[u8]>,
    /// Offset into `mem` for each 1 KB page, for reads and for writes.
    read_map: [usize; 64],
    write_map: [usize; 64],
    mode: u8,
    /// Emulated time in CPU cycles since power-on.
    cycles: u64,
    /// Time up to which `run_cycles` has been asked to run.
    target: u64,
    /// Cycle of the most recent frame capture.
    frame_cycle: u64,
    pic: Pic,
    pit: [Pit; 3],
    ppi1: Ppi,
    ppi2: Ppi,
    usart: Usart,
    fdc: Fdc,
    /// Clock pulses delivered so far to the timers on fixed clocks: PIT #1
    /// (1 MHz), PIT #3 ch1 (2 MHz) and PIT #3 ch0 (16/13 MHz), with ch2
    /// when `t32_fast`. They lag behind `cycles` while the pulses only count
    /// down.
    pit1_pulses: u64,
    sound_pulses: u64,
    baud_pulses: u64,
    /// The next PIT #1 pulse and the next PIT #3 ch1 pulse on which
    /// something other than counting down may happen.
    pit1_event: u64,
    sound_event: u64,
    /// Cycle of the earliest device event; nothing needs doing before it.
    next_event: u64,
    /// Wait cycles added to the current instruction by FDC data accesses.
    stall: u64,
    /// PIT #3 ch2 counts at 16/13 MHz, as the 1987 schematic draws it,
    /// instead of once a frame (`docs/T3.2_and_TIMCNT.md`).
    t32_fast: bool,
    kbd: Keyboard,
    caps: CapsFollower,
    mouse: Mouse,
    speaker: Speaker,
    /// When enabled, every OUT as (cycle, port, value), for debugging.
    io_log: Option<Vec<(u64, u8, u8)>>,
    /// The beam and the frames it draws.
    raster: Raster,
    /// The instruction being executed, and how many memory writes it has
    /// made, for the time of a write that may race the beam.
    op: u8,
    writes: u8,
}

/// The ROM address of the click's JMP (or the RET that replaced it), if the
/// click code is in `CLICK_CHIP` exactly once and the chip's sum is the one
/// the ROM test expects, so that patching can keep the test passing.
fn find_click(rom: &[u8]) -> Option<usize> {
    let chip = &rom[CLICK_CHIP];
    let matches = |code: &[u8]| {
        code.iter()
            .zip(CLICK)
            .enumerate()
            .all(|(i, (&b, c))| b == c || i == CLICK_JMP && b == RET)
    };
    let mut found = chip.windows(CLICK.len()).enumerate().filter(|(_, w)| matches(w));
    let (start, _) = found.next()?;
    if found.next().is_some() {
        return None;
    }
    let sum = chip.iter().fold(0u8, |s, &b| s.wrapping_add(b));
    (sum == rom[CLICK_CHIP_SUM]).then_some(CLICK_CHIP.start + start + CLICK_JMP)
}

/// Turns the key click in `rom` on or off (see [`Machine::set_key_click`]).
fn patch_click(rom: &mut [u8], on: bool) -> bool {
    let Some(at) = find_click(rom) else {
        return false;
    };
    let new = if on { CLICK[CLICK_JMP] } else { RET };
    let old = std::mem::replace(&mut rom[at], new);
    rom[CLICK_CHIP_SUM] = rom[CLICK_CHIP_SUM].wrapping_add(new).wrapping_sub(old);
    true
}

impl Machine {
    /// Creates a machine from the 16 KB system ROM. Returns `None` if the ROM
    /// has the wrong size.
    pub fn new(rom: &[u8]) -> Option<Machine> {
        Machine::with_audio(rom, None)
    }

    /// Like [`Machine::new`], with the speaker feeding `audio`.
    pub fn with_audio(rom: &[u8], audio: Option<Arc<AudioRing>>) -> Option<Machine> {
        if rom.len() != ROM_SIZE {
            return None;
        }
        let mut mem = vec![0u8; MEM_SIZE].into_boxed_slice();
        mem[ROM_OFS..EXP_OFS].copy_from_slice(rom);
        mem[EXP_OFS..SINK_OFS].fill(0xFF);
        let mut m = Machine {
            cpu: Cpu::new(),
            sys: System {
                mem,
                read_map: [0; 64],
                write_map: [0; 64],
                mode: 0,
                cycles: 0,
                target: 0,
                frame_cycle: 0,
                pic: Pic::new(),
                pit: [Pit::new(), Pit::new(), Pit::new()],
                ppi1: Ppi::new(),
                ppi2: Ppi::new(),
                usart: Usart::new(),
                fdc: Fdc::new(),
                pit1_pulses: 0,
                sound_pulses: 0,
                baud_pulses: 0,
                pit1_event: 0,
                sound_event: 0,
                next_event: 0,
                stall: 0,
                t32_fast: false,
                kbd: Keyboard::new(),
                caps: CapsFollower::default(),
                mouse: Mouse::new(),
                speaker: Speaker::new(audio),
                io_log: None,
                raster: Raster::new(),
                op: 0,
                writes: 0,
            },
            debug: None,
        };
        m.reset(true);
        Some(m)
    }

    /// Replaces the system ROM and cold-starts the machine, as if the EPROMs
    /// had been swapped with the power off. Disks stay in their drives.
    /// Returns false, changing nothing, if the ROM has the wrong size.
    pub fn load_rom(&mut self, rom: &[u8]) -> bool {
        if rom.len() != ROM_SIZE {
            return false;
        }
        self.sys.mem[ROM_OFS..EXP_OFS].copy_from_slice(rom);
        self.reset(true);
        true
    }

    /// Plugs in an expansion ROM (`expansion::check`) and cold-starts the
    /// machine: the expander is changed with the power off. The image fills
    /// the external window from 4000h; the rest reads FFh. On error nothing
    /// changes.
    pub fn insert_expansion(&mut self, image: &[u8]) -> Result<(), ExpansionError> {
        expansion::check(image)?;
        let window = &mut self.sys.mem[EXP_OFS..SINK_OFS];
        window.fill(0xFF);
        window[..image.len()].copy_from_slice(image);
        self.reset(true);
        Ok(())
    }

    /// Unplugs the expansion ROM and cold-starts the machine.
    pub fn eject_expansion(&mut self) {
        self.sys.mem[EXP_OFS..SINK_OFS].fill(0xFF);
        self.reset(true);
    }

    /// Warm reset (the RESET button) keeps RAM and the timers; cold reset is
    /// a power cycle.
    pub fn reset(&mut self, cold: bool) {
        if let Some(d) = &mut self.debug {
            d.forget();
        }
        let s = &mut self.sys;
        if cold {
            s.mem[..RAM_SIZE].fill(0);
            s.pic = Pic::new();
            s.pit = [Pit::new(), Pit::new(), Pit::new()];
            s.mouse = Mouse::new();
            s.mouse.catch_up(s.cycles);
        }
        s.caps.rearm(&mut s.kbd);
        self.cpu.reset();
        s.ppi1.reset();
        s.ppi2.reset();
        s.usart.reset();
        let motor = s.fdc.motor();
        s.fdc.reset();
        if motor {
            s.speaker.drive_event(s.cycles, DriveEvent::Motor(false), true);
        }
        s.set_mode(0);
        s.update_speaker(s.cycles);
        let (hor, ver, pof) = (s.pit[PIT1].out(1), s.pit[PIT2].out(1), s.ppi1.c & 0x80 != 0);
        s.raster.set_levels(s.cycles >> 1, &s.mem[..RAM_SIZE], hor, ver, pof);
        s.schedule();
    }

    /// Runs the machine for `cycles` more CPU cycles. Instructions are not
    /// split, so a call may run a few cycles long; the excess is deducted
    /// from the next call. Returns the cycles executed.
    ///
    /// While debugging, it stops early when the debugger stops the machine,
    /// and runs nothing while it is stopped.
    pub fn run_cycles(&mut self, cycles: u64) -> u64 {
        let start = self.sys.cycles;
        if self.debug.is_some() {
            self.run_debugging(cycles);
        } else {
            self.sys.target += cycles;
            while self.sys.cycles < self.sys.target {
                self.step();
            }
        }
        // Nothing watches the baud rate counter, or T3.2 on its fast clock,
        // so they are only brought up to date here and when PIT #3 is
        // accessed.
        self.sys.sync_pit(PIT3);
        self.sys.drive_sounds(self.sys.cycles);
        self.sys.speaker.advance(self.sys.cycles);
        self.sys.cycles - start
    }

    /// `run_cycles` while debugging: each instruction through the debugger,
    /// until the time is up or the debugger stops the machine. Kept out of
    /// line, so that the ordinary loop compiles as it would without it.
    #[inline(never)]
    fn run_debugging(&mut self, cycles: u64) {
        let Some(d) = self.debug.as_deref_mut() else {
            return;
        };
        if d.stopped().is_none() {
            self.sys.target += cycles;
        }
        while self.sys.cycles < self.sys.target {
            let c = d.step(&mut self.cpu, &mut self.sys) + std::mem::take(&mut self.sys.stall) as u32;
            self.sys.advance(c);
            if d.stopped().is_some() || self.sys.target < self.sys.cycles {
                self.sys.target = self.sys.cycles;
            }
        }
    }

    /// Executes one instruction (or interrupt acknowledge) and advances the
    /// devices. Returns the cycles taken.
    pub fn step(&mut self) -> u32 {
        let c = self.cpu.step(&mut self.sys) + std::mem::take(&mut self.sys.stall) as u32;
        self.sys.advance(c);
        if self.sys.target < self.sys.cycles {
            self.sys.target = self.sys.cycles;
        }
        c
    }

    pub fn cycles(&self) -> u64 {
        self.sys.cycles
    }

    /// The last completed frame.
    pub fn frame(&self) -> &Frame {
        self.sys.raster.frame()
    }

    /// Cycle at which the last frame was completed (start of vertical sync).
    pub fn frame_cycle(&self) -> u64 {
        self.sys.frame_cycle
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    /// The devices' state, for the debugger. The timers are brought up to
    /// date first: they count in batches (spec §4.1), which changes nothing
    /// they show.
    pub fn devices(&mut self) -> Devices {
        let s = &mut self.sys;
        s.sync_pit(PIT1);
        s.sync_pit(PIT3);
        Devices {
            pic: s.pic.registers(),
            pits: [0, 1, 2].map(|p| [0, 1, 2].map(|ch| s.pit[p].state(ch))),
            ppis: [[s.ppi1.a, s.ppi1.b, s.ppi1.c], [s.ppi2.a, s.ppi2.b, s.ppi2.c]],
            fdc: s.fdc.state(),
            drives: [0, 1].map(|d| (s.fdc.drives[d].cyl, s.fdc.drives[d].disk.is_some())),
        }
    }

    /// The CPU's registers, for the debugger to change.
    pub fn cpu_mut(&mut self) -> &mut Cpu {
        &mut self.cpu
    }

    /// Starts debugging, with no breakpoints and the machine running, or
    /// ends it (spec §12.4). Ending lets a stopped machine run on.
    pub fn set_debugging(&mut self, on: bool) {
        if on != self.debug.is_some() {
            self.debug = on.then(Box::default);
        }
        if let Some(d) = &mut self.debug {
            d.note_code(self.cpu.pc);
        }
    }

    pub fn debugger(&self) -> Option<&Debug> {
        self.debug.as_deref()
    }

    pub fn debugger_mut(&mut self) -> Option<&mut Debug> {
        self.debug.as_deref_mut()
    }

    /// Lets the machine run on from a debugger stop, as `how` says.
    pub fn resume(&mut self, how: Resume) {
        if let Some(d) = &mut self.debug {
            let sys = &self.sys;
            d.resume(how, &self.cpu, |a| sys.peek(a));
        }
    }

    /// The 64 KB of RAM, as the video circuit sees it.
    pub fn ram(&self) -> &[u8] {
        &self.sys.mem[..RAM_SIZE]
    }

    /// Reads memory as the CPU would, without side effects.
    pub fn peek(&self, addr: u16) -> u8 {
        self.sys.peek(addr)
    }

    pub fn set_key(&mut self, id: u8, down: bool) {
        self.sys.kbd.set_key(id, down);
    }

    /// Whether a key is down, pressed by the host or by the Caps Lock
    /// follower.
    pub fn key_down(&self, id: u8) -> bool {
        self.sys.kbd.is_down(id)
    }

    /// RomBios's keyboard state CONSW (MD §14.3): bit 0 CAPS LOCK, bit 1 RUS.
    pub fn consw(&self) -> u8 {
        self.sys.mem[keyboard::CONSW as usize]
    }

    /// Makes the guest Caps Lock follow `on` from now on: the host's state.
    pub fn set_caps_lock(&mut self, on: bool) {
        self.sys.caps.set_target(on);
    }

    /// Turns the RomBios key click on or off by patching the ROM, and
    /// adjusts the ROM test's checksum to match (MD §11). Returns false,
    /// changing nothing, if the ROM does not have the click code once in its
    /// chip, or that chip's sum does not match the ROM test's.
    pub fn set_key_click(&mut self, on: bool) -> bool {
        patch_click(&mut self.sys.mem[ROM_OFS..EXP_OFS], on)
    }

    /// Clocks PIT #3 counter 2 (T3.2) at 1.2308 MHz, as the 1987 schematic
    /// draws it, or once a frame, from VER RTR, as MAME does. The frame clock
    /// is the default: the RomBios stopwatch TIMCNT counts frames with it.
    /// RDEM needs the fast clock for its random numbers (MD §10.2).
    pub fn set_t32_fast_clock(&mut self, on: bool) {
        // The pulses until now come from the old clock.
        self.sys.sync_pit(PIT3);
        self.sys.t32_fast = on;
    }

    pub fn release_all_keys(&mut self) {
        self.sys.kbd.release_all();
    }

    /// Host mouse input: movement in Juku counts (+x right, +y down) and the
    /// buttons (bit 1 left, bit 0 right).
    pub fn mouse_input(&mut self, dx: i32, dy: i32, buttons: u8) {
        self.sys.mouse.input(dx, dy, buttons);
    }

    /// True while software has the mouse interrupt (IR6) unmasked: a program
    /// that uses the mouse is running.
    pub fn mouse_enabled(&self) -> bool {
        self.sys.pic.mask() & (1 << IRQ_MOUSE) == 0
    }

    pub fn set_dip(&mut self, bank: u8, value: u8) {
        self.sys.kbd.set_dip(bank, value);
    }

    /// Starts recording I/O writes, or stops and returns the record.
    pub fn log_io(&mut self, on: bool) -> Vec<(u64, u8, u8)> {
        let old = self.sys.io_log.take().unwrap_or_default();
        if on {
            self.sys.io_log = Some(Vec::new());
        }
        old
    }

    /// Inserts a disk image into drive 0 (A:) or 1 (B:), replacing any disk
    /// there.
    pub fn insert_disk(&mut self, drive: usize, image: Vec<u8>, write_protect: bool) -> Result<(), DiskError> {
        let disk = Disk::new(image, write_protect)?;
        self.sys.insert_disk(drive, disk);
        Ok(())
    }

    /// Inserts a new disk, formatted and empty, and not write-protected.
    pub fn insert_blank_disk(&mut self, drive: usize) {
        self.sys.insert_disk(drive, Disk::blank());
    }

    pub fn eject_disk(&mut self, drive: usize) -> Option<Disk> {
        let disk = self.sys.fdc.drives[drive & 1].eject();
        if disk.is_some() {
            self.sys.disk_sound(DriveEvent::Eject { drive: drive as u8 & 1 });
        }
        disk
    }

    /// Turns the floppy drives' sounds on or off (spec §4.5). They are off
    /// in a new machine.
    pub fn set_drive_sounds(&mut self, on: bool) {
        self.sys.speaker.set_drive_sounds(on);
    }

    /// The drive sounds' volume, from 0 to 2; 1 is the recordings' level
    /// halved (spec §4.5).
    pub fn set_drive_sounds_volume(&mut self, volume: f32) {
        self.sys.speaker.set_drive_volume(volume);
    }

    /// With `on`, the drive motors fade out once the controller has had no
    /// command for 10 s, though the motor line stays on.
    pub fn set_drive_idle_mute(&mut self, on: bool) {
        self.sys.speaker.set_drive_idle_mute(on);
    }

    pub fn disk(&self, drive: usize) -> Option<&Disk> {
        self.sys.fdc.drives[drive & 1].disk.as_ref()
    }

    pub fn disk_mut(&mut self, drive: usize) -> Option<&mut Disk> {
        self.sys.fdc.drives[drive & 1].disk.as_mut()
    }

    /// Stops queueing audio (warp speed) or resumes it.
    pub fn set_audio_muted(&mut self, muted: bool) {
        self.sys.speaker.set_muted(muted);
    }

    /// A disk command is running on drive `n`.
    pub fn drive_active(&self, drive: usize) -> bool {
        self.sys.fdc.active(drive & 1)
    }

    /// Writes RAM directly, bypassing the memory mode (for tests and tools).
    pub fn poke(&mut self, addr: u16, val: u8) {
        self.sys.mem[addr as usize] = val;
    }

    /// Writes memory as the CPU would in the present mode, without timing: a
    /// write to the external window in mode 2 is dropped (for the debugger).
    pub fn store(&mut self, addr: u16, val: u8) {
        let s = &mut self.sys;
        s.mem[s.write_map[(addr >> 10) as usize] + (addr as usize & (PAGE - 1))] = val;
    }

    /// The system ROM as the machine has it, with the key click patch if it
    /// is applied (for the debugger).
    pub fn rom(&self) -> &[u8] {
        &self.sys.mem[ROM_OFS..EXP_OFS]
    }

    /// The addressing mode, 0–3 (MD §3.2).
    pub fn memory_mode(&self) -> u8 {
        self.sys.mode
    }
}

/// Snapshots keep emulated time far from overflow: 2⁵² cycles are 71 years.
const MAX_SNAPSHOT_CYCLES: u64 = 1 << 52;

/// A snapshot decoded and checked, ready to replace the machine's state.
struct Restored<'a> {
    cpu: Cpu,
    mode: u8,
    cycles: u64,
    frame_cycle: u64,
    pit1_pulses: u64,
    sound_pulses: u64,
    baud_pulses: u64,
    ram: Vec<u8>,
    rom: Vec<u8>,
    click_off: bool,
    expansion: Vec<u8>,
    pic: Pic,
    pit: [Pit; 3],
    ppi1: Ppi,
    ppi2: Ppi,
    usart: Usart,
    fdc: Fdc,
    kbd: Keyboard,
    mouse: Mouse,
    /// The raster without its frames, which are in `frames`: the last
    /// completed one and the one being drawn.
    raster: RasterState<'a>,
    frames: [Vec<u8>; 2],
}

fn section(tag: [u8; 4], f: impl FnOnce(&mut Writer)) -> ([u8; 4], Vec<u8>) {
    let mut w = Writer::new();
    f(&mut w);
    (tag, w.into_bytes())
}

/// Reads section `tag` with `f`, which must use all of it.
fn read_section<'a, T>(s: &Sections<'a>, tag: &[u8; 4], f: impl FnOnce(&mut Reader<'a>) -> Option<T>) -> Option<T> {
    let mut r = s.reader(tag)?;
    let v = f(&mut r)?;
    r.end()?;
    Some(v)
}

/// Snapshots (spec §4.6).
impl Machine {
    /// Saves the machine as a snapshot file. `thumbnail` is the PNG the file
    /// shows as a picture; without one the core uses the frame as it is.
    /// The `host` sections are stored as they are, and their tags must start
    /// with a lowercase letter. Returns `None` for a bad thumbnail or tag.
    pub fn save_snapshot(&self, thumbnail: Option<&[u8]>, host: &[([u8; 4], &[u8])]) -> Option<Vec<u8>> {
        if !host.iter().all(|(tag, _)| snapshot::is_host_tag(tag)) {
            return None;
        }
        let own;
        let png = match thumbnail {
            Some(png) => png,
            None => {
                own = snapshot::frame_png(self.frame());
                &own
            }
        };
        let mut file = SnapshotWriter::new(png)?;
        for (tag, data) in self.sections() {
            file.section(tag, &data);
        }
        for &(tag, data) in host {
            file.section(tag, data);
        }
        Some(file.finish())
    }

    /// The system ROM as its file has it: with the key click patch undone.
    /// A host matches it to its ROM files by these bytes.
    pub fn system_rom(&self) -> Vec<u8> {
        let mut rom = self.sys.mem[ROM_OFS..EXP_OFS].to_vec();
        if self.key_click_off() {
            patch_click(&mut rom, true);
        }
        rom
    }

    fn key_click_off(&self) -> bool {
        let rom = &self.sys.mem[ROM_OFS..EXP_OFS];
        find_click(rom).is_some_and(|at| rom[at] == RET)
    }

    /// The expansion window up to its last byte that is not FFh: empty with
    /// no expansion ROM plugged in.
    pub fn expansion_rom(&self) -> &[u8] {
        let window = &self.sys.mem[EXP_OFS..SINK_OFS];
        &window[..window.iter().rposition(|&b| b != 0xFF).map_or(0, |i| i + 1)]
    }

    fn sections(&self) -> Vec<([u8; 4], Vec<u8>)> {
        let s = &self.sys;
        vec![
            section(*b"SNAP", |w| w.u32(snapshot::VERSION)),
            section(*b"SYS ", |w| {
                w.u8(s.mode);
                w.u64(s.cycles);
                w.u64(s.frame_cycle);
                w.u64(s.pit1_pulses);
                w.u64(s.sound_pulses);
                w.u64(s.baud_pulses);
            }),
            section(*b"CPU ", |w| self.cpu.save(w)),
            section(*b"RAM ", |w| w.packed(&s.mem[..RAM_SIZE])),
            // The ROM is stored with the key click on, so that it compares
            // equal to its file, and a flag says whether it was patched.
            section(*b"ROM ", |w| {
                w.packed(&self.system_rom());
                w.bool(self.key_click_off());
            }),
            section(*b"EXPN", |w| w.packed(self.expansion_rom())),
            section(*b"PIC ", |w| s.pic.save(w)),
            section(*b"PIT1", |w| s.pit[PIT1].save(w)),
            section(*b"PIT2", |w| s.pit[PIT2].save(w)),
            section(*b"PIT3", |w| s.pit[PIT3].save(w)),
            section(*b"PPI1", |w| s.ppi1.save(w)),
            section(*b"PPI2", |w| s.ppi2.save(w)),
            section(*b"USRT", |w| s.usart.save(w)),
            section(*b"FDC ", |w| s.fdc.save(w)),
            section(*b"DRV0", |w| s.fdc.drives[0].save(w)),
            section(*b"DRV1", |w| s.fdc.drives[1].save(w)),
            section(*b"KBD ", |w| s.kbd.save(w)),
            section(*b"MOUS", |w| s.mouse.save(w)),
            section(*b"RAST", |w| {
                let r = s.raster.state();
                w.u64(r.t);
                w.u64(r.line_t);
                w.u32(r.y);
                w.u16(r.addr);
                w.bool(r.hor);
                w.bool(r.ver);
                w.bool(r.pof);
                w.u16(r.geom.bytes_per_line);
                w.u16(r.geom.lines);
                w.u16(r.geom.hporch_us);
                w.u16(r.geom.vporch_lines);
                w.packed(r.front);
                w.packed(r.back);
            }),
        ]
    }

    /// Restores a snapshot file made by [`Machine::save_snapshot`]. On
    /// error nothing changes. What the host connected stays: the audio ring
    /// and its mute, the Caps Lock follower, I/O logging and the T3.2 clock. Keys held in the
    /// snapshot stay down until the host releases them.
    pub fn load_snapshot(&mut self, file: &[u8]) -> Result<(), SnapshotError> {
        let sections = Sections::parse(file)?;
        let restored = self.decode(&sections).ok_or(SnapshotError::Damaged)?;
        self.restore(restored);
        Ok(())
    }

    fn decode<'a>(&self, s: &Sections<'a>) -> Option<Restored<'a>> {
        let sys = |r: &mut Reader| Some((r.u8()?, r.u64()?, r.u64()?, r.u64()?, r.u64()?, r.u64()?));
        let (mode, cycles, frame_cycle, pit1_pulses, sound_pulses, baud_pulses) = read_section(s, b"SYS ", sys)?;
        let times_ok = cycles < MAX_SNAPSHOT_CYCLES
            && frame_cycle <= cycles
            && pit1_pulses <= cycles >> 1
            && sound_pulses <= cycles
            && baud_pulses <= cycles * 8 / 13;
        if mode > 3 || !times_ok {
            return None;
        }
        let (rom, click_off) = read_section(s, b"ROM ", |r| Some((r.packed(ROM_SIZE).filter(|rom| rom.len() == ROM_SIZE)?, r.bool()?)))?;
        let drives = [
            read_section(s, b"DRV0", |r| Drive::load(r, cycles))?,
            read_section(s, b"DRV1", |r| Drive::load(r, cycles))?,
        ];
        let (raster, frames) = read_section(s, b"RAST", |r| {
            let mut state = RasterState {
                t: r.u64()?,
                line_t: r.u64()?,
                y: r.u32()?,
                addr: r.u16()?,
                hor: r.bool()?,
                ver: r.bool()?,
                pof: r.bool()?,
                geom: Geometry { bytes_per_line: r.u16()?, lines: r.u16()?, hporch_us: r.u16()?, vporch_lines: r.u16()? },
                front: &[],
                back: &[],
            };
            let frames = [r.packed(RASTER_SIZE)?, r.packed(RASTER_SIZE)?];
            (state.front, state.back) = (&frames[0], &frames[1]);
            let g = state.geom;
            let ok = Raster::state_ok(&state, cycles >> 1)
                && (1..=RASTER_BYTES as u16).contains(&g.bytes_per_line)
                && (1..=video::RASTER_LINES as u16).contains(&g.lines)
                && (g.hporch_us as usize) < RASTER_BYTES
                && (g.vporch_lines as usize) < video::RASTER_LINES;
            ok.then(|| (RasterState { front: &[], back: &[], ..state }, frames))
        })?;
        Some(Restored {
            cpu: read_section(s, b"CPU ", Cpu::load)?,
            mode,
            cycles,
            frame_cycle,
            pit1_pulses,
            sound_pulses,
            baud_pulses,
            ram: read_section(s, b"RAM ", |r| r.packed(RAM_SIZE).filter(|ram| ram.len() == RAM_SIZE))?,
            rom,
            click_off,
            expansion: read_section(s, b"EXPN", |r| r.packed(expansion::WINDOW_SIZE))?,
            pic: read_section(s, b"PIC ", Pic::load)?,
            pit: [
                read_section(s, b"PIT1", Pit::load)?,
                read_section(s, b"PIT2", Pit::load)?,
                read_section(s, b"PIT3", Pit::load)?,
            ],
            ppi1: read_section(s, b"PPI1", Ppi::load)?,
            ppi2: read_section(s, b"PPI2", Ppi::load)?,
            usart: read_section(s, b"USRT", Usart::load)?,
            fdc: read_section(s, b"FDC ", |r| Fdc::load(r, drives, cycles))?,
            kbd: read_section(s, b"KBD ", Keyboard::load)?,
            mouse: read_section(s, b"MOUS", |r| Mouse::load(r, cycles))?,
            raster,
            frames,
        })
    }

    fn restore(&mut self, r: Restored) {
        self.cpu = r.cpu;
        if let Some(d) = &mut self.debug {
            d.forget();
        }
        let s = &mut self.sys;
        s.mem[..RAM_SIZE].copy_from_slice(&r.ram);
        let rom = &mut s.mem[ROM_OFS..EXP_OFS];
        rom.copy_from_slice(&r.rom);
        if r.click_off {
            patch_click(rom, false);
        }
        let window = &mut s.mem[EXP_OFS..SINK_OFS];
        window.fill(0xFF);
        window[..r.expansion.len()].copy_from_slice(&r.expansion);
        s.cycles = r.cycles;
        s.target = r.cycles;
        s.frame_cycle = r.frame_cycle;
        s.pit1_pulses = r.pit1_pulses;
        s.sound_pulses = r.sound_pulses;
        s.baud_pulses = r.baud_pulses;
        s.stall = 0;
        s.pic = r.pic;
        s.pit = r.pit;
        s.ppi1 = r.ppi1;
        s.ppi2 = r.ppi2;
        s.usart = r.usart;
        s.fdc = r.fdc;
        s.kbd = r.kbd;
        s.mouse = r.mouse;
        s.set_mode(r.mode);

        // The snapshot's picture becomes a new frame: `seq` only counts up,
        // so the host sees the change.
        s.raster.restore(&RasterState { front: &r.frames[0], back: &r.frames[1], ..r.raster });

        // Only the Caps Lock follower presses CAPS. The host's Caps Lock may
        // differ from the snapshot's, so the follower starts again.
        s.kbd.set_key(keyboard::KEY_CAPS, false);
        s.caps.rearm(&mut s.kbd);
        s.mouse.catch_up(s.cycles);
        s.speaker.restart(s.cycles);
        let disks = [0, 1].map(|d| s.fdc.drives[d].disk.is_some());
        s.speaker.sync_drives(s.fdc.motor(), disks);
        s.update_speaker(s.cycles);
        s.schedule();
    }
}

impl System {
    /// Addressing mode = PPI #1 PC1..0 (MD §3.2).
    fn set_mode(&mut self, mode: u8) {
        self.mode = mode;
        for p in 0..64 {
            let ram = p * PAGE;
            let (r, w) = match mode {
                0 if p < 16 => (ROM_OFS + ram, ram),
                1 | 2 if p >= 54 => (ROM_OFS + 0x1800 + (p - 54) * PAGE, ram),
                2 if (16..48).contains(&p) => (EXP_OFS + (p - 16) * PAGE, SINK_OFS),
                _ => (ram, ram),
            };
            self.read_map[p] = r;
            self.write_map[p] = w;
        }
    }

    #[inline]
    fn peek(&self, addr: u16) -> u8 {
        self.mem[self.read_map[(addr >> 10) as usize] + (addr as usize & (PAGE - 1))]
    }

    /// Advances all devices by `c` CPU cycles.
    #[inline]
    fn advance(&mut self, c: u32) {
        let end = self.cycles + c as u64;
        if end >= self.next_event {
            self.run_events(end);
        }
        self.cycles = end;
    }

    /// Handles every device event up to cycle `end`.
    ///
    /// The timers on fixed clocks are not clocked pulse by pulse. Most pulses
    /// only count down, so a timer is left alone until the first pulse that
    /// may do more (`Pit::quiet`); then the pulses before it are applied in
    /// one step and that one is clocked properly, with its OUT changes
    /// passed on.
    fn run_events(&mut self, end: u64) {
        // PIT #1 at 1 MHz drives the whole raster.
        while self.pit1_event <= end >> 1 {
            let t = self.pit1_event;
            for ch in 0..3 {
                self.pit[PIT1].skip(ch, t - 1 - self.pit1_pulses);
            }
            self.pit1_pulses = t;
            self.cycles = t << 1;
            for ch in 0..3 {
                if self.pit[PIT1].clock(ch) {
                    self.out_changed(PIT1, ch);
                }
            }
            self.schedule_pit1();
        }
        // PIT #3 ch1 at 2 MHz is the sound.
        while self.sound_event <= end {
            let t = self.sound_event;
            self.pit[PIT3].skip(1, t - 1 - self.sound_pulses);
            self.sound_pulses = t;
            if self.pit[PIT3].clock(1) {
                self.update_speaker(t);
            }
            self.schedule_sound();
        }
        while self.mouse.next_poll() <= end {
            if self.mouse.do_poll() {
                // A new request edge on INT6.
                self.pic.set_irq(IRQ_MOUSE, false);
                self.pic.set_irq(IRQ_MOUSE, true);
            }
        }
        self.next_event = self.pit1_event.saturating_mul(2).min(self.sound_event).min(self.mouse.next_poll());
    }

    fn schedule_pit1(&mut self) {
        let quiet = (0..3).map(|ch| self.pit[PIT1].quiet(ch)).min().unwrap_or(0);
        self.pit1_event = self.pit1_pulses.saturating_add(quiet).saturating_add(1);
    }

    fn schedule_sound(&mut self) {
        self.sound_event = self.sound_pulses.saturating_add(self.pit[PIT3].quiet(1)).saturating_add(1);
    }

    /// Works out the next events again, after anything that may have changed
    /// what the timers or the mouse are about to do.
    fn schedule(&mut self) {
        self.schedule_pit1();
        self.schedule_sound();
        // The next `advance` sorts out which comes first.
        self.next_event = 0;
    }

    /// Applies the pulses a timer is behind by, before its registers are
    /// read or written. These are all pulses that only count down.
    fn sync_pit(&mut self, pit: usize) {
        match pit {
            PIT1 => {
                let now = self.cycles >> 1;
                for ch in 0..3 {
                    self.pit[PIT1].skip(ch, now - self.pit1_pulses);
                }
                self.pit1_pulses = now;
            }
            PIT3 => {
                self.pit[PIT3].skip(1, self.cycles - self.sound_pulses);
                self.sound_pulses = self.cycles;
                // 16/13 MHz: 8 pulses per 13 CPU cycles. The OUTs of ch0 and,
                // on that clock, ch2 drive nothing (MD §10.1, §10.2).
                let now = self.cycles * 8 / 13;
                self.pit[PIT3].run(0, now - self.baud_pulses);
                if self.t32_fast {
                    self.pit[PIT3].run(2, now - self.baud_pulses);
                }
                self.baud_pulses = now;
            }
            _ => {}
        }
    }

    /// Speaker level = PIT #3 OUT1, at full or half volume by PA4 (MD §9).
    fn update_speaker(&mut self, t: u64) {
        if self.speaker.drive_sounds() {
            self.drive_sounds(t);
        }
        let level = match (self.pit[PIT3].out(1), self.ppi1.a & 0x10 != 0) {
            (false, _) => 0.0,
            (true, true) => 1.0,
            (true, false) => 0.5,
        };
        self.speaker.set_level(t, level);
    }

    /// Brings the FDC up to cycle `t` and hands what its drives did to the
    /// speaker, before the speaker gets that far, so that each sound starts
    /// on time. Advancing the FDC early changes nothing: its events happen
    /// at their own times whenever it is advanced.
    fn drive_sounds(&mut self, t: u64) {
        self.fdc.advance(t);
        for (at, event) in self.fdc.sounds.drain(..) {
            self.speaker.drive_event(at.min(t), event, true);
        }
    }

    fn insert_disk(&mut self, drive: usize, disk: Disk) {
        let swap = self.fdc.drives[drive & 1].disk.is_some();
        self.fdc.drives[drive & 1].insert(disk, self.cycles);
        self.disk_sound(DriveEvent::Insert { drive: drive as u8 & 1, swap });
    }

    /// The host put a disk in or took one out. Disks put in before the
    /// machine first runs were in their drives already: they make no sound.
    fn disk_sound(&mut self, event: DriveEvent) {
        self.speaker.drive_event(self.cycles, event, self.cycles > 0);
    }

    fn clock_pit(&mut self, pit: usize, ch: usize) {
        if self.pit[pit].clock(ch) {
            self.out_changed(pit, ch);
        }
    }

    fn gate_pit(&mut self, pit: usize, ch: usize, level: bool) {
        if self.pit[pit].set_gate(ch, level) {
            self.out_changed(pit, ch);
        }
    }

    /// Propagates a change of a timer OUT pin (MD §7.2; spec §4.5). Counters
    /// clocked from an OUT count on its falling edge.
    fn out_changed(&mut self, pit: usize, ch: usize) {
        let out = self.pit[pit].out(ch);
        match (pit, ch) {
            (PIT1, 0) => {
                // Line rate: clocks the line counter, gates the horizontal one-shots.
                if !out {
                    self.clock_pit(PIT2, 0);
                }
                self.gate_pit(PIT1, 1, out);
                self.gate_pit(PIT1, 2, out);
            }
            (PIT1, 1) => {
                // HOR RTR: the bytes of a line are shown while it is high.
                self.raster.set_hor(self.cycles >> 1, &self.mem[..RAM_SIZE], out);
            }
            (PIT1, 2) => {
                // Its rising edge fires the horizontal sync one-shot, whose
                // Q̄ clocks the vertical one-shots (MD §7.2).
                if out {
                    self.raster.hsync(self.cycles >> 1, &self.mem[..RAM_SIZE]);
                    self.clock_pit(PIT2, 1);
                    self.clock_pit(PIT2, 2);
                }
            }
            (PIT2, 0) => {
                // Frame rate: gates the vertical one-shots.
                self.gate_pit(PIT2, 1, out);
                self.gate_pit(PIT2, 2, out);
            }
            (PIT2, 1) => {
                // VER RTR, low during vertical blank; inverted onto IR5. Its
                // rising edge loads the video address counter, and its
                // falling edge clocks T3.2 unless that is on 16/13 MHz.
                self.raster.set_ver(self.cycles >> 1, &self.mem[..RAM_SIZE], out);
                self.pic.set_irq(IRQ_FRAME, !out);
                if !out {
                    self.caps.frame(self.mem[keyboard::CONSW as usize], &mut self.kbd);
                    if !self.t32_fast {
                        self.clock_pit(PIT3, 2);
                    }
                }
            }
            (PIT2, 2) => {
                // Its rising edge fires the vertical sync one-shot: the frame
                // is complete.
                if out {
                    let h = [0, 1, 2].map(|ch| self.pit[PIT1].count(ch));
                    let v = [0, 1, 2].map(|ch| self.pit[PIT2].count(ch));
                    self.raster.vsync(self.cycles >> 1, &self.mem[..RAM_SIZE], Geometry::from_counts(h, v));
                    self.frame_cycle = self.cycles;
                }
            }
            (PIT3, 1) => self.update_speaker(self.cycles),
            _ => {}
        }
    }

    /// Brings every device and the raster up to cycle `at`, inside the
    /// instruction being executed, before a bus access at that time.
    fn catch_up(&mut self, at: u64) {
        let start = self.cycles;
        if at >= self.next_event {
            self.run_events(at);
        }
        self.cycles = start;
        self.raster.advance(at >> 1, &self.mem[..RAM_SIZE]);
    }

    /// A write to RAM at `index` that the beam may have fetched: the raster
    /// is drawn up to the time of the write, with the old value.
    #[cold]
    fn racing_write(&mut self) {
        self.catch_up(self.cycles + write_at(self.op, self.writes));
    }

    /// READY/WAIT: a data register access during a transfer command holds the
    /// CPU until DRQ (or the end of the command), unless PA6 disables the wait
    /// (MD §10.3).
    fn fdc_wait(&mut self) {
        if self.ppi1.a & 0x40 != 0 {
            return;
        }
        let start = self.cycles + self.stall;
        self.fdc.advance(start);
        while self.fdc.wants_wait() {
            let t = self.fdc.next_event();
            if t > start + MAX_FDC_STALL {
                break;
            }
            self.fdc.advance(t);
            self.stall = t - self.cycles;
        }
    }

    fn pit_read(&mut self, pit: usize, reg: u8) -> u8 {
        self.sync_pit(pit);
        self.pit[pit].read(reg)
    }

    fn pit_write(&mut self, pit: usize, reg: u8, v: u8) {
        self.sync_pit(pit);
        let before = self.pit[pit].outs();
        self.pit[pit].write(reg, v);
        let changed = before ^ self.pit[pit].outs();
        for ch in 0..3 {
            if changed & (1 << ch) != 0 {
                self.out_changed(pit, ch);
            }
        }
        self.schedule();
    }
}

impl Bus for System {
    #[inline]
    fn read(&mut self, addr: u16) -> u8 {
        self.peek(addr)
    }

    #[inline]
    fn begin(&mut self, op: u8) {
        self.op = op;
        self.writes = 0;
    }

    #[inline]
    fn write(&mut self, addr: u16, val: u8) {
        let index = self.write_map[(addr >> 10) as usize] + (addr as usize & (PAGE - 1));
        if index < RAM_SIZE && (index as u16).wrapping_sub(self.raster.watch) < self.raster.watch_len {
            self.racing_write();
        }
        self.writes = self.writes.wrapping_add(1);
        self.mem[index] = val;
    }

    fn input(&mut self, port: u8) -> u8 {
        // The devices see the read at its time in the instruction.
        let start = self.cycles;
        self.catch_up(start + IO_AT);
        self.cycles = start + IO_AT;
        let v = self.port_in(port);
        self.cycles = start;
        v
    }

    fn output(&mut self, port: u8, val: u8) {
        let start = self.cycles;
        self.catch_up(start + IO_AT);
        self.cycles = start + IO_AT;
        self.port_out(port, val);
        self.cycles = start;
    }

    fn reads_back(&self, addr: u16) -> bool {
        let p = (addr >> 10) as usize;
        self.read_map[p] == self.write_map[p]
    }

    fn int_pending(&self) -> bool {
        self.pic.int()
    }

    fn int_ack(&mut self) -> [u8; 3] {
        self.pic.ack()
    }
}

impl System {
    fn port_in(&mut self, port: u8) -> u8 {
        let reg = port & 3;
        match port {
            0x00..=0x03 => self.pic.read(port),
            0x04..=0x07 => {
                if reg == 1 {
                    self.caps.note_scan(self.ppi1.a);
                }
                let pb = self.kbd.port_b(self.ppi1.a);
                self.ppi1.read(reg, [0xFF, pb, 0xFF])
            }
            0x08..=0x0B => self.usart.read(port),
            0x0C..=0x0F => self.ppi2.read(reg, [0xFF; 3]),
            0x10..=0x13 => self.pit_read(PIT1, reg),
            0x14..=0x17 => self.pit_read(PIT2, reg),
            0x18..=0x1B => self.pit_read(PIT3, reg),
            0x1C..=0x1F => {
                if reg == 3 {
                    self.fdc_wait();
                }
                self.fdc.read(self.cycles + self.stall, reg)
            }
            0x80 | 0x81 => {
                // The read clears the card's latches, and with them INT6.
                self.pic.set_irq(IRQ_MOUSE, false);
                self.mouse.read()
            }
            _ => 0xFF,
        }
    }

    fn port_out(&mut self, port: u8, val: u8) {
        if let Some(log) = &mut self.io_log {
            log.push((self.cycles, port, val));
        }
        let reg = port & 3;
        match port {
            0x00..=0x03 => self.pic.write(port, val),
            0x04..=0x07 => {
                self.ppi1.write(reg, val);
                self.update_speaker(self.cycles);
                let c = self.ppi1.c;
                self.fdc.set_control(self.cycles, c & 0x04 != 0, (c >> 5) as usize & 1, (c >> 6) & 1, c & 0x10 != 0);
                let mode = self.ppi1.c & 3;
                if mode != self.mode {
                    self.set_mode(mode);
                }
                let pof = self.ppi1.c & 0x80 != 0;
                if pof != self.raster.pof() {
                    self.raster.set_pof(self.cycles >> 1, &self.mem[..RAM_SIZE], pof);
                }
            }
            0x08..=0x0B => self.usart.write(port, val),
            0x0C..=0x0F => self.ppi2.write(reg, val),
            0x10..=0x13 => self.pit_write(PIT1, reg, val),
            0x14..=0x17 => self.pit_write(PIT2, reg, val),
            0x18..=0x1B => self.pit_write(PIT3, reg, val),
            0x1C..=0x1F => {
                if reg == 3 {
                    self.fdc_wait();
                }
                self.fdc.write(self.cycles + self.stall, reg, val);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_wrong_rom_size() {
        assert!(Machine::new(&[0u8; 100]).is_none());
        assert!(Machine::new(&[0u8; ROM_SIZE]).is_some());
    }

    #[test]
    fn expansion_rom_appears_in_the_mode_2_window() {
        let mut m = machine_with_rom(&[0x76]);
        let mut image = vec![0u8; 0x2000];
        image[..7].copy_from_slice(&[0xC3, 0x07, 0x01, 0xBA, 0xDC, 0x00, 0x20]);
        image[0x1FFF] = 0x5A;
        m.sys.write(0xC000, 0x55);
        assert!(m.insert_expansion(&[0x00; 16]).is_err());
        assert_eq!(m.ram()[0xC000], 0x55, "a rejected image changes nothing");

        assert!(m.insert_expansion(&image).is_ok());
        assert_eq!(m.ram()[0xC000], 0x00, "cold start");
        m.sys.write(0x4000, 0x11);
        m.sys.set_mode(2);
        assert_eq!(m.peek(0x4000), 0xC3);
        assert_eq!(m.peek(0x5FFF), 0x5A);
        assert_eq!(m.peek(0x6000), 0xFF, "past the image");
        assert_eq!(m.peek(0xBFFF), 0xFF);
        m.sys.write(0x4001, 0x22);
        assert_eq!(m.peek(0x4001), 0x07, "writes to the window are dropped");
        for mode in [0, 1, 3] {
            m.sys.set_mode(mode);
            assert_eq!(m.peek(0x4000), 0x11, "mode {mode}: RAM");
        }

        m.eject_expansion();
        m.sys.set_mode(2);
        assert_eq!(m.peek(0x4000), 0xFF, "nothing plugged in");
    }

    /// Where a write is read back (`Bus::reads_back`): not under the ROM
    /// or the external window (MD §3.2).
    #[test]
    fn writes_read_back_where_nothing_lies_over_them() {
        let mut m = machine_with_rom(&[0x76]);
        let cases = [(0, [false, true, true]), (1, [true, true, false]), (2, [true, false, false]), (3, [true, true, true])];
        for (mode, [low, window, top]) in cases {
            m.sys.set_mode(mode);
            assert_eq!([m.sys.reads_back(0x0100), m.sys.reads_back(0x8000), m.sys.reads_back(0xD9CC)], [low, window, top], "mode {mode}");
        }
    }

    #[test]
    fn load_rom_replaces_rom_and_cold_starts() {
        let mut m = machine_with_rom(&[0x76]);
        m.sys.write(0xC000, 0x55);
        assert!(!m.load_rom(&[0u8; 100]));
        assert_eq!(m.ram()[0xC000], 0x55, "a rejected ROM changes nothing");
        let mut rom = vec![0u8; ROM_SIZE];
        rom[0] = 0xC3;
        assert!(m.load_rom(&rom));
        assert_eq!(m.peek(0x0000), 0xC3);
        assert_eq!(m.ram()[0xC000], 0x00, "RAM cleared");
        assert_eq!(m.cpu().pc, 0x0000);
    }

    #[test]
    fn key_click_patch_leaves_unknown_roms_alone() {
        let mut m = machine_with_rom(&[]);
        let before = m.sys.mem.clone();
        assert!(!m.set_key_click(false));
        assert!(m.sys.mem == before);
    }

    /// A ROM with the click code at `at`, and the chip sum at 0008 off by
    /// `sum_error`.
    fn rom_with_click(at: &[usize], sum_error: u8) -> Vec<u8> {
        let mut rom = vec![0u8; ROM_SIZE];
        for &a in at {
            rom[a..][..CLICK.len()].copy_from_slice(&CLICK);
        }
        let sum = rom[CLICK_CHIP].iter().fold(0u8, |s, &b| s.wrapping_add(b));
        rom[CLICK_CHIP_SUM] = sum.wrapping_add(sum_error);
        rom
    }

    #[test]
    fn key_click_patch_finds_the_code_and_keeps_the_rom_test_sum() {
        let mut m = Machine::new(&rom_with_click(&[0x1265], 0)).unwrap();
        assert!(m.set_key_click(false));
        assert_eq!(m.peek(0x1265 + CLICK_JMP as u16), RET);
        let sum = (0x1000..0x1800).fold(0u8, |s, a| s.wrapping_add(m.peek(a)));
        assert_eq!(sum, m.peek(CLICK_CHIP_SUM as u16));
        assert!(m.set_key_click(true));
        assert_eq!(m.peek(0x1265 + CLICK_JMP as u16), CLICK[CLICK_JMP]);
    }

    #[test]
    fn key_click_patch_needs_one_copy_and_a_matching_sum() {
        for rom in [rom_with_click(&[0x1265], 1), rom_with_click(&[0x1100, 0x1300], 0)] {
            let mut m = Machine::new(&rom).unwrap();
            let before = m.sys.mem.clone();
            assert!(!m.set_key_click(false));
            assert!(m.sys.mem == before);
        }
    }

    fn machine_with_rom(code: &[u8]) -> Machine {
        let mut rom = vec![0u8; ROM_SIZE];
        for (i, b) in rom.iter_mut().enumerate().skip(0x1800) {
            *b = i as u8;
        }
        rom[..code.len()].copy_from_slice(code);
        Machine::new(&rom).unwrap()
    }

    #[test]
    fn memory_modes() {
        let mut m = machine_with_rom(&[0x76]);
        m.sys.write(0x0000, 0x55); // under ROM in mode 0
        m.sys.write(0xD800, 0x66);
        assert_eq!(m.peek(0x0000), 0x76);
        assert_eq!(m.peek(0xD800), 0x66);

        m.sys.output(0x07, 0x82);
        m.sys.output(0x06, 0x01);
        assert_eq!(m.peek(0x0000), 0x55);
        assert_eq!(m.peek(0xD800), 0x00, "ROM 1800h");
        assert_eq!(m.peek(0xFFFF), 0xFF, "ROM 3FFFh");
        assert_eq!(m.ram()[0xD800], 0x66, "video RAM under ROM");

        m.sys.output(0x06, 0x02);
        m.sys.write(0x4000, 0x77);
        assert_eq!(m.peek(0x4000), 0xFF, "empty external window");
        assert_eq!(m.ram()[0x4000], 0x00);
        assert_eq!(m.peek(0xD801), 0x01);

        m.sys.output(0x06, 0x03);
        assert_eq!(m.peek(0xD800), 0x66);
        assert_eq!(m.peek(0x0000), 0x55);

        m.reset(false);
        assert_eq!(m.peek(0x0000), 0x76, "reset selects mode 0");
    }

    /// A Read Sector given just after its sector's ID went by waits more
    /// than a revolution for the first data byte. The CPU, held on its
    /// first IN 1Fh, must get that byte and not the stale data register
    /// (QDISK.COM read a sector one byte late this way).
    #[test]
    fn a_read_waits_over_a_revolution_for_its_first_byte() {
        use crate::floppy::{CYLINDERS, REV_CYCLES, SECTOR_SIZE, SECTORS};
        const BYTE: u64 = 64; // 32 µs in MFM
        let mut m = machine_with_rom(&[0x76]);
        let mut image = vec![0u8; CYLINDERS as usize * 2 * SECTORS as usize * SECTOR_SIZE];
        for (i, b) in image[..SECTOR_SIZE].iter_mut().enumerate() {
            *b = i as u8 ^ 0xA5;
        }
        m.insert_disk(0, image, false).unwrap();
        m.sys.output(0x07, 0x82);
        m.sys.output(0x04, 0x00); // PA6 = 0: the wait is on
        m.sys.output(0x06, 0x04); // motor on, drive 0, side 0, MFM
        m.run_cycles(CPU_HZ);

        // Reads sector 1 as the BIOS does; returns the bytes and when the
        // first one came.
        let read = |m: &mut Machine| {
            m.sys.output(0x1D, 0);
            m.sys.output(0x1E, 1);
            m.sys.output(0x1C, 0x80);
            let mut first = 0;
            let bytes: Vec<u8> = (0..SECTOR_SIZE)
                .map(|i| {
                    let b = m.sys.input(0x1F);
                    if i == 0 {
                        first = m.sys.cycles + m.sys.stall;
                    }
                    b
                })
                .collect();
            m.sys.cycles += std::mem::take(&mut m.sys.stall);
            m.sys.target = m.sys.cycles;
            m.run_cycles(CPU_HZ / 50);
            assert_eq!(m.sys.input(0x1C) & 0x9D, 0, "status");
            (bytes, first)
        };

        let (_, first) = read(&mut m);
        // The ID field ends some 38 bytes before the data. A command given
        // from then until the data starts misses the ID this turn.
        for late in [2, 10, 20, 30, 36] {
            let data = first + (m.cycles() / REV_CYCLES + 2) * REV_CYCLES;
            m.run_cycles(data - late * BYTE - m.cycles());
            let (bytes, _) = read(&mut m);
            assert_eq!(bytes[0], 0xA5, "{late} bytes before the data: first byte");
            assert!(bytes.iter().enumerate().all(|(i, &b)| b == i as u8 ^ 0xA5), "{late} bytes before the data");
        }
    }

    #[test]
    fn keyboard_on_port_b() {
        let mut m = machine_with_rom(&[]);
        m.sys.output(0x07, 0x82);
        m.sys.output(0x04, 0x05);
        assert_eq!(m.sys.input(0x05), 0xFF);
        m.set_key(crate::keyboard::key_id(5, 3), true); // Q
        assert_eq!((!m.sys.input(0x05) >> 1) & 7, 3);
        m.sys.output(0x04, 0x06);
        assert_eq!(m.sys.input(0x05), 0xFF);
    }

    /// Programs the timers as RomBios does (ROM 01D4–0220) and checks the
    /// frame interrupt rate.
    #[test]
    fn frame_interrupt_every_40064_cycles() {
        let mut m = machine_with_rom(&[]);
        let init: &[(u8, u8)] = &[
            (0x13, 0x15), (0x13, 0x53), (0x13, 0x93), (0x17, 0x73), (0x17, 0x93), (0x17, 0x34),
            (0x14, 0x39), (0x14, 0x01), (0x10, 0x64), (0x11, 0x24), (0x12, 0x08),
            (0x15, 0x72), (0x15, 0x00), (0x16, 0x25),
            (0x00, 0xD6), (0x01, 0xFE), (0x01, 0xDF),
        ];
        for &(p, v) in init {
            m.sys.output(p, v);
        }
        // The CPU runs NOPs with interrupts disabled; the test acknowledges.
        let mut edges = Vec::new();
        let mut seq = m.frame().seq;
        while edges.len() < 4 {
            m.step();
            if m.frame().seq != seq {
                seq = m.frame().seq;
                edges.push(m.frame_cycle());
                assert!(m.sys.pic.int(), "IR5 requested");
                m.sys.pic.ack();
                m.sys.pic.write(0, 0x20);
            }
        }
        for w in edges.windows(2) {
            assert_eq!(w[1] - w[0], CYCLES_PER_FRAME);
        }
        let f = m.frame();
        assert_eq!((f.active_w, f.active_h, f.active_x, f.active_y), (40, 241, 16, 47));
    }

    /// The RomBios video timing (ROM 01D4–0220), on a ROM of NOPs.
    fn video_machine() -> Machine {
        let mut m = machine_with_rom(&[]);
        let init: &[(u8, u8)] = &[
            (0x13, 0x15), (0x13, 0x53), (0x13, 0x93), (0x17, 0x73), (0x17, 0x93), (0x17, 0x34),
            (0x14, 0x39), (0x14, 0x01), (0x10, 0x64), (0x11, 0x24), (0x12, 0x08),
            (0x15, 0x72), (0x15, 0x00), (0x16, 0x25),
        ];
        for &(p, v) in init {
            m.sys.output(p, v);
        }
        // Video memory with no two neighbouring bytes alike, and no zeros.
        for i in 0..0x2800u16 {
            m.poke(video::VIDEO_BASE + i, (i % 253) as u8 + 1);
        }
        next_frame(&mut m);
        next_frame(&mut m);
        m
    }

    fn next_frame(m: &mut Machine) {
        let seq = m.frame().seq;
        while m.frame().seq == seq {
            m.step();
        }
    }

    /// Runs to raster line `y`, `x` µs after the horizontal sync. Returns
    /// the line, x and the address the counter fetches there.
    fn run_to(m: &mut Machine, y: u32, x: u64) -> (usize, usize, u16) {
        loop {
            m.step();
            m.sys.catch_up(m.sys.cycles);
            let st = m.sys.raster.state();
            if st.y == y && st.t - st.line_t >= x {
                return (y as usize, (st.t - st.line_t) as usize, st.addr);
            }
        }
    }

    fn raster_line(m: &Machine, y: usize) -> &[u8] {
        &m.frame().data[y * RASTER_BYTES..][..RASTER_BYTES]
    }

    /// A CPU write to a byte the beam has just fetched shows in the next
    /// frame only; one to a byte further on shows in this one.
    #[test]
    fn writes_race_the_beam() {
        let mut m = video_machine();
        let (y, x, a) = run_to(&mut m, 100, 30);
        let old = m.ram()[a as usize];
        // MOV M,A writes 5 T-states in, 2 µs after the beam fetched `a`.
        m.sys.begin(0x77);
        m.sys.write(a, 0xEE);
        m.sys.begin(0x77);
        m.sys.write(a + 8, 0xEE);
        next_frame(&mut m);
        let line = raster_line(&m, y);
        assert_eq!(line[x], old, "fetched before the write");
        assert_eq!(line[x + 8], 0xEE, "fetched after it");
        next_frame(&mut m);
        assert_eq!(raster_line(&m, y)[x], 0xEE, "the next frame has it");
    }

    /// A mode 0 control word to T1.1 drops HOR RTR at once: a gap that
    /// uses no memory, until the count runs out (MD §7.5).
    #[test]
    fn a_mid_line_gap_holds_the_address_counter() {
        let mut m = video_machine();
        let (y, x, a) = run_to(&mut m, 100, 20);
        m.sys.output(0x13, 0x50); // T1.1: mode 0, LSB only, binary
        m.step();
        m.step();
        m.sys.output(0x11, 6);
        run_to(&mut m, 100, 50);
        m.sys.output(0x13, 0x53); // back to mode 1, BCD 24 for the next line
        m.sys.output(0x11, 0x24);
        next_frame(&mut m);

        let line = raster_line(&m, y);
        let expect = |k: usize| m.ram()[a as usize + k];
        let mut k = 0;
        let mut gap = 0;
        for &b in &line[x..56] {
            if b == 0 {
                gap += 1;
            } else {
                assert_eq!(b, expect(k), "byte {k} after x = {x}: {line:?}");
                k += 1;
            }
        }
        assert!((8..=14).contains(&gap), "a gap of {gap} µs: {line:?}");
        assert_eq!(line[16..x + 1].iter().filter(|&&b| b == 0).count(), 0);
        let next = raster_line(&m, y + 1);
        assert_eq!(next[16], expect(k), "the next line carries on");
    }

    /// VER RTR taken low and high again in horizontal blank: its rising
    /// edge reloads the counter, and the line starts from D800h (MD §7.5).
    #[test]
    fn a_ver_rtr_restart_shows_the_top_again() {
        let mut m = video_machine();
        let (y, _, _) = run_to(&mut m, 100, 2);
        m.sys.output(0x17, 0x71); // T2.1: mode 0, LSB and MSB, BCD: low
        m.step();
        m.sys.output(0x17, 0x73); // mode 1: high
        run_to(&mut m, 100, 60);
        m.sys.output(0x15, 0x72); // 72 lines for the next frame's blank
        m.sys.output(0x15, 0x00);
        next_frame(&mut m);
        assert_eq!(raster_line(&m, y)[16..56], raster_line(&m, 47)[16..56]);
        assert_eq!(raster_line(&m, y + 1)[16..56], raster_line(&m, 48)[16..56]);
        next_frame(&mut m);
        assert_ne!(raster_line(&m, y)[16..56], raster_line(&m, 47)[16..56], "one frame only");
    }
}

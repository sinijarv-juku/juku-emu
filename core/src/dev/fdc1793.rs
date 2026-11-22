//! WD1793 floppy disk controller (КР1818ВГ93), clocked at 1 MHz.
//!
//! Event driven: the controller keeps the time of its next internal event
//! and is brought up to date by [`Fdc::advance`] whenever the machine touches
//! it. Disk rotation is derived from absolute time, so nothing needs to run
//! between accesses (INTRQ is not wired to the CPU on the Juku).
//!
//! The track layout is that of real disks, as the Juku's formatters write it
//! (MD §10.3): MFM, 6250 bytes per revolution, the sectors in the order 1–10.

use crate::drive_sound::DriveEvent;
use crate::floppy::{CYLINDERS, Disk, Drive, INDEX_CYCLES, REV_CYCLES, SECTOR_SIZE, SECTORS};
use crate::machine::CPU_HZ;
use crate::snapshot::{Reader, Writer};

const MS: u64 = CPU_HZ / 1000;
/// One MFM byte: 32 µs.
const BYTE: u64 = 64;
const TRACK_BYTES: usize = (REV_CYCLES / BYTE) as usize;
// Byte offsets in the track: 32 × 4E, the index mark, 32 × 4E, then one slot
// per sector. A slot ends in gap 3, 24 × 4E and three A1 syncs, which the
// formatters' table puts there.
const FIRST_SLOT: usize = 32 + 4 + 32;
const SLOT: usize = 12 + 4 + 4 + 2 + 22 + 12 + 4 + SECTOR_SIZE + 2 + 24 + 3;
/// The BIOS's Write Track stops after 6161 bytes of the formatters' buffer,
/// 6181 on the track since each F7 writes two CRC bytes. The 1793 writes 00
/// from there to the index.
const FORMATTED: usize = 6181;
/// Slot on the track that holds sector `id`. Real disks have the sectors in
/// order, with no skew (MD §10.3). EKDOS, which reads a track as 1, 3, 5, 7,
/// 9, 2, 4, 6, 8, 10, then misses the next sector here and waits a
/// revolution; whether it did on the machine is not known.
fn slot_of(id: u8) -> usize {
    id.saturating_sub(1) as usize
}
/// From the start of a slot: the ID field (after the IDAM), its end (after
/// CRC), and the first data byte.
const ID_START: usize = 16;
const ID_END: usize = 22;
const DATA_START: usize = 60;
/// Write Sector: the first byte must be supplied within this many bytes of
/// the ID field, or the command ends with LOST DATA.
const WRITE_DRQ_BYTES: u64 = 22;
/// Step rates for r1r0 at 1 MHz, and the head settle time for V / E.
const STEP_MS: [u64; 4] = [6, 12, 20, 30];
const SETTLE: u64 = 30 * MS;
/// An ID that does not come round within this many revolutions is not found.
const SEARCH_REVS: u64 = 5;
/// The most drive sounds kept for the machine to take.
const MAX_SOUNDS: usize = 64;

// Status bits.
const BUSY: u8 = 0x01;
const DRQ_INDEX: u8 = 0x02;
const LOST_TR00: u8 = 0x04;
const RNF_SEEK: u8 = 0x10;
const HEAD_LOADED: u8 = 0x20;
const WRITE_PROTECT: u8 = 0x40;
const NOT_READY: u8 = 0x80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Idle,
    /// Type I: one step per event until done.
    Step,
    /// Type I: head settle, then check the track.
    Verify,
    /// Type II/III: waiting for an ID field (`found`) or the deadline.
    Search,
    /// Streaming `buf` to the CPU, one DRQ per byte.
    ReadBytes,
    /// Write Sector: the first byte must have been supplied by now.
    WriteCheck,
    /// Write Sector: taking bytes from the CPU into `buf`.
    WriteBytes,
    /// Read/Write Track: waiting for the index pulse.
    TrackWait,
    /// Write Track: taking bytes from the CPU until the next index pulse.
    TrackBytes,
    /// Trailing CRC bytes; then the command ends (or continues, multi-sector).
    Finish,
}

/// The registers as the debugger shows them.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FdcState {
    pub command: u8,
    pub track: u8,
    pub sector: u8,
    pub data: u8,
    pub busy: bool,
    pub drq: bool,
    pub intrq: bool,
    pub motor: bool,
    /// The drive selected.
    pub select: u8,
    pub side: u8,
}

pub struct Fdc {
    pub drives: [Drive; 2],
    // Registers.
    track: u8,
    sector: u8,
    data: u8,
    cmd: u8,
    /// Error and condition bits gathered by the current command.
    status: u8,
    busy: bool,
    drq: bool,
    intrq: bool,
    /// The status register shows Type I bits.
    type1: bool,
    head_loaded: bool,
    // Command execution.
    op: Op,
    next: u64,
    deadline: u64,
    /// Sector ID found by the current search.
    found: Option<u8>,
    steps: u8,
    step_out: bool,
    buf: Vec<u8>,
    pos: usize,
    // Drive control from 8255 #1 port C.
    motor_since: Option<u64>,
    select: usize,
    side: u8,
    fm: bool,
    /// Sectors from Write Track that the image cannot hold.
    pub format_errors: u32,
    /// What the drives did that makes a sound, with its time, for the
    /// machine to take (spec §4.5).
    pub sounds: Vec<(u64, DriveEvent)>,
}

impl Default for Fdc {
    fn default() -> Fdc {
        Fdc::new()
    }
}

/// CRC-CCITT as used in ID and data fields.
fn crc16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0xFFFF, |crc, &b| {
        let mut crc = crc ^ (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { crc << 1 ^ 0x1021 } else { crc << 1 };
        }
        crc
    })
}

/// The raw MFM track as Read Track returns it.
fn build_track(disk: Option<&Disk>, cyl: u8, side: u8) -> Vec<u8> {
    let Some(disk) = disk.filter(|d| d.has_track(cyl, side)) else {
        return vec![0x4E; TRACK_BYTES];
    };
    let mut t = vec![0x4E; 32];
    t.extend_from_slice(&[0xC2, 0xC2, 0xC2, 0xFC]);
    t.extend_from_slice(&[0x4E; 32]);
    for id in 1..=SECTORS {
        let field = [0xA1, 0xA1, 0xA1, 0xFE, cyl, side, id, 2];
        t.extend_from_slice(&[0; 12]);
        t.extend_from_slice(&field);
        t.extend_from_slice(&crc16(&field).to_be_bytes());
        t.extend_from_slice(&[0x4E; 22]);
        t.extend_from_slice(&[0; 12]);
        let mut data = vec![0xA1, 0xA1, 0xA1, 0xFB];
        data.extend_from_slice(disk.sector(cyl, side, id).unwrap_or(&[0; SECTOR_SIZE]));
        let crc = crc16(&data);
        t.extend_from_slice(&data);
        t.extend_from_slice(&crc.to_be_bytes());
        t.extend_from_slice(&[0x4E; 24]);
        t.extend_from_slice(&[0xA1; 3]);
    }
    t.resize(FORMATTED, 0x4E);
    t.resize(TRACK_BYTES, 0);
    t
}

impl Fdc {
    pub fn new() -> Fdc {
        Fdc {
            drives: [Drive::default(), Drive::default()],
            track: 0,
            sector: 1,
            data: 0,
            cmd: 0,
            status: 0,
            busy: false,
            drq: false,
            intrq: false,
            type1: true,
            head_loaded: false,
            op: Op::Idle,
            next: u64::MAX,
            deadline: 0,
            found: None,
            steps: 0,
            step_out: false,
            buf: Vec::new(),
            pos: 0,
            motor_since: None,
            select: 0,
            side: 0,
            fm: false,
            format_errors: 0,
            sounds: Vec::new(),
        }
    }

    /// MR input: abandons any command. Drives and disks are untouched.
    pub fn reset(&mut self) {
        let drives = std::mem::take(&mut self.drives);
        let sounds = std::mem::take(&mut self.sounds);
        *self = Fdc { drives, sounds, ..Fdc::new() };
    }

    /// Drive control lines (8255 #1 PC2–PC6, MD §10.3): the shared motor
    /// line, which also enables drive select; drive 0/1; side; FM.
    pub fn set_control(&mut self, now: u64, motor: bool, drive: usize, side: u8, fm: bool) {
        self.advance(now);
        if motor != self.motor_since.is_some() {
            self.motor_since = motor.then_some(now);
            self.sound(now, DriveEvent::Motor(motor));
        }
        self.select = drive & 1;
        self.side = side & 1;
        self.fm = fm;
    }

    /// The motor line is on.
    pub fn motor(&self) -> bool {
        self.motor_since.is_some()
    }

    /// Keeps a drive sound for the machine, which takes them as it goes.
    fn sound(&mut self, now: u64, event: DriveEvent) {
        if self.sounds.len() < MAX_SOUNDS {
            self.sounds.push((now, event));
        }
    }

    /// The selected drive: none while the motor/enable line is off.
    fn drive(&self) -> Option<&Drive> {
        self.motor_since.map(|_| &self.drives[self.select])
    }

    fn ready(&self, now: u64) -> bool {
        self.drive().is_some_and(|d| d.ready(now, self.motor_since))
    }

    fn index(&self, now: u64) -> bool {
        self.drive().is_some_and(|d| d.disk.is_some()) && now % REV_CYCLES < INDEX_CYCLES
    }

    /// A command is running on drive `n` (for an activity indicator). The
    /// motor line alone says little: RomBios leaves it on.
    pub fn active(&self, n: usize) -> bool {
        self.busy && self.motor_since.is_some() && self.select == n
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    /// The registers, for the debugger.
    pub fn state(&self) -> FdcState {
        FdcState {
            command: self.cmd,
            track: self.track,
            sector: self.sector,
            data: self.data,
            busy: self.busy,
            drq: self.drq,
            intrq: self.intrq,
            motor: self.motor(),
            select: self.select as u8,
            side: self.side,
        }
    }

    pub fn drq(&self) -> bool {
        self.drq
    }

    pub fn intrq(&self) -> bool {
        self.intrq
    }

    /// Time of the next internal event (`u64::MAX` if none).
    pub fn next_event(&self) -> u64 {
        self.next
    }

    /// True while a data transfer command waits for DRQ: a data register
    /// access now would hold the CPU in WAIT on the Juku (MD §10.3).
    pub fn wants_wait(&self) -> bool {
        self.busy && !self.drq && self.cmd >= 0x80 && self.cmd & 0xF0 != 0xD0
    }

    /// Processes all events up to `now`.
    pub fn advance(&mut self, now: u64) {
        while self.next <= now {
            let t = self.next;
            self.next = u64::MAX;
            self.event(t);
        }
    }

    pub fn read(&mut self, now: u64, reg: u8) -> u8 {
        self.advance(now);
        match reg & 3 {
            0 => {
                self.intrq = false;
                self.status_register(now)
            }
            1 => self.track,
            2 => self.sector,
            _ => {
                self.drq = false;
                self.data
            }
        }
    }

    pub fn write(&mut self, now: u64, reg: u8, v: u8) {
        self.advance(now);
        match reg & 3 {
            0 => self.command(now, v),
            1 if !self.busy => self.track = v,
            2 if !self.busy => self.sector = v,
            3 => {
                self.drq = false;
                self.data = v;
            }
            _ => {}
        }
    }

    fn status_register(&self, now: u64) -> u8 {
        let mut s = self.status;
        if self.busy {
            s |= BUSY;
        }
        if !self.ready(now) {
            s |= NOT_READY;
        }
        if self.type1 {
            if self.drive().is_some_and(|d| d.write_protect()) {
                s |= WRITE_PROTECT;
            }
            if self.head_loaded {
                s |= HEAD_LOADED;
            }
            if self.drive().is_some_and(|d| d.cyl == 0) {
                s |= LOST_TR00;
            }
            if self.index(now) {
                s |= DRQ_INDEX;
            }
        } else if self.drq {
            s |= DRQ_INDEX;
        }
        s
    }

    fn step_cycles(&self) -> u64 {
        STEP_MS[(self.cmd & 3) as usize] * MS
    }

    fn command(&mut self, now: u64, v: u8) {
        self.sound(now, DriveEvent::Access);
        if v & 0xF0 == 0xD0 {
            // Force Interrupt.
            if self.busy {
                self.busy = false;
                self.op = Op::Idle;
                self.next = u64::MAX;
            } else {
                self.type1 = true;
                self.status = 0;
            }
            self.cmd = v;
            self.intrq = v & 0x0F != 0;
            return;
        }
        if self.busy {
            return;
        }
        self.cmd = v;
        self.status = 0;
        self.intrq = false;
        self.drq = false;
        self.busy = true;
        self.type1 = v < 0x80;

        if self.type1 {
            self.head_loaded = v & 0x08 != 0;
            self.steps = 0;
            self.op = Op::Step;
            match v >> 5 {
                0 => {
                    // Restore / Seek: the step events walk the head.
                    self.next = now;
                }
                n => {
                    // Step (keeps the direction), Step In, Step Out.
                    if n == 2 {
                        self.step_out = false;
                    } else if n == 3 {
                        self.step_out = true;
                    }
                    if v & 0x10 != 0 {
                        self.track = if self.step_out { self.track.wrapping_sub(1) } else { self.track.wrapping_add(1) };
                    }
                    self.step_head(now);
                    self.next = now + self.step_cycles();
                }
            }
            return;
        }

        self.head_loaded = true;
        if !self.ready(now) {
            self.status = NOT_READY;
            self.finish(now);
            return;
        }
        let writes = matches!(v >> 4, 0xA | 0xB | 0xF);
        if writes && self.drive().is_some_and(|d| d.write_protect()) {
            self.status = WRITE_PROTECT;
            self.finish(now);
            return;
        }
        let start = if v & 0x04 != 0 { now + SETTLE } else { now };
        match v >> 4 {
            0xE | 0xF => {
                self.drq = v >> 4 == 0xF;
                self.op = Op::TrackWait;
                self.next = (start / REV_CYCLES + 1) * REV_CYCLES;
            }
            _ => self.search(start),
        }
    }

    fn step_head(&mut self, now: u64) {
        if self.motor_since.is_some() {
            let drive = &mut self.drives[self.select];
            let stop = !drive.step(self.step_out);
            let phase = drive.cyl & 3;
            self.sound(now, DriveEvent::Step { drive: self.select as u8, phase, stop });
        }
    }

    /// Looks for the next ID field this command wants under the head.
    fn search(&mut self, now: u64) {
        self.op = Op::Search;
        self.deadline = now + SEARCH_REVS * REV_CYCLES;
        let read_address = self.cmd >> 4 == 0xC;
        let offset = if read_address { ID_START } else { ID_END };
        let side = self.side;
        let wanted = self.sector;
        let side_ok = self.cmd & 0x02 == 0 || (self.cmd >> 3) & 1 == side;
        let found = self.drive().filter(|_| !self.fm).and_then(|d| {
            let disk = d.disk.as_ref()?;
            if !disk.has_track(d.cyl, side) {
                return None;
            }
            let ids: Vec<u8> = if read_address {
                (1..=SECTORS).collect()
            } else if d.cyl == self.track && side_ok && (1..=SECTORS).contains(&wanted) {
                vec![wanted]
            } else {
                vec![]
            };
            let rev = now / REV_CYCLES * REV_CYCLES;
            ids.into_iter()
                .map(|id| {
                    let t = rev + (FIRST_SLOT + slot_of(id) * SLOT + offset) as u64 * BYTE;
                    (if t <= now { t + REV_CYCLES } else { t }, id)
                })
                .min()
        });
        match found {
            Some((t, id)) if t < self.deadline => {
                self.found = Some(id);
                self.next = t;
            }
            _ => {
                self.found = None;
                self.next = self.deadline;
            }
        }
    }

    fn finish(&mut self, _now: u64) {
        self.busy = false;
        self.intrq = true;
        self.op = Op::Idle;
        self.next = u64::MAX;
    }

    fn event(&mut self, t: u64) {
        match self.op {
            Op::Idle => {}
            Op::Step => self.step_event(t),
            Op::Verify => {
                let ok = !self.fm
                    && self.ready(t)
                    && self.drive().is_some_and(|d| {
                        d.cyl == self.track && d.disk.as_ref().is_some_and(|disk| disk.has_track(d.cyl, self.side))
                    });
                if !ok {
                    self.status |= RNF_SEEK;
                }
                self.finish(t);
            }
            Op::Search => {
                let Some(id) = self.found else {
                    self.status |= RNF_SEEK;
                    self.finish(t);
                    return;
                };
                let (cyl, side) = (self.drive().map_or(0, |d| d.cyl), self.side);
                match self.cmd >> 4 {
                    0x8 | 0x9 => {
                        let disk = self.drives[self.select].disk.as_ref();
                        self.buf = disk.and_then(|d| d.sector(cyl, side, id)).unwrap_or(&[0; SECTOR_SIZE]).to_vec();
                        self.start_read(t + (DATA_START - ID_END) as u64 * BYTE);
                    }
                    0xC => {
                        let mut id_field = vec![cyl, side, id, 2];
                        let crc = crc16(&[&[0xA1, 0xA1, 0xA1, 0xFE][..], &id_field].concat());
                        id_field.extend_from_slice(&crc.to_be_bytes());
                        self.buf = id_field;
                        self.start_read(t);
                    }
                    _ => {
                        // Write Sector: ask for the first byte.
                        self.drq = true;
                        self.op = Op::WriteCheck;
                        self.next = t + WRITE_DRQ_BYTES * BYTE;
                    }
                }
            }
            Op::ReadBytes => {
                if self.drq {
                    self.status |= LOST_TR00;
                }
                self.data = self.buf[self.pos];
                self.drq = true;
                self.pos += 1;
                if self.pos < self.buf.len() {
                    self.next = t + BYTE;
                } else {
                    self.op = Op::Finish;
                    self.next = t + 2 * BYTE;
                }
            }
            Op::WriteCheck => {
                if self.drq {
                    self.drq = false;
                    self.status |= LOST_TR00;
                    self.finish(t);
                } else {
                    self.buf.clear();
                    self.op = Op::WriteBytes;
                    self.next = t + (DATA_START as u64 - ID_END as u64 - WRITE_DRQ_BYTES) * BYTE;
                }
            }
            Op::WriteBytes => {
                let byte = self.take_byte();
                self.buf.push(byte);
                if self.buf.len() < SECTOR_SIZE {
                    self.drq = true;
                    self.next = t + BYTE;
                } else {
                    let (cyl, side, id) = (self.drives[self.select].cyl, self.side, self.found.unwrap_or(0));
                    if let Some(disk) = self.drives[self.select].disk.as_mut() {
                        disk.write_sector(cyl, side, id, &self.buf);
                    }
                    self.op = Op::Finish;
                    self.next = t + 3 * BYTE;
                }
            }
            Op::TrackWait => {
                if self.cmd >> 4 == 0xE {
                    let d = &self.drives[self.select];
                    self.buf = build_track(d.disk.as_ref(), d.cyl, self.side);
                    self.start_read(t + BYTE);
                } else if self.drq {
                    self.drq = false;
                    self.status |= LOST_TR00;
                    self.finish(t);
                } else {
                    self.buf.clear();
                    self.buf.push(self.data);
                    self.drq = true;
                    self.op = Op::TrackBytes;
                    self.next = t + BYTE;
                }
            }
            Op::TrackBytes => {
                let byte = self.take_byte();
                self.buf.push(byte);
                if self.buf.len() < TRACK_BYTES {
                    self.drq = true;
                    self.next = t + BYTE;
                } else {
                    self.drq = false;
                    self.commit_track();
                    self.finish(t);
                }
            }
            Op::Finish => {
                if self.cmd >> 4 == 0xC {
                    self.sector = self.buf[0];
                }
                if matches!(self.cmd >> 4, 0x9 | 0xB) {
                    // Multiple sectors: carry on with the next one.
                    self.sector = self.sector.wrapping_add(1);
                    self.search(t);
                } else {
                    self.finish(t);
                }
            }
        }
    }

    fn start_read(&mut self, first_byte: u64) {
        self.pos = 0;
        self.op = Op::ReadBytes;
        self.next = first_byte;
    }

    /// The byte the CPU supplied for this byte time, or 0 with LOST DATA.
    fn take_byte(&mut self) -> u8 {
        if self.drq {
            self.status |= LOST_TR00;
            0
        } else {
            self.data
        }
    }

    fn step_event(&mut self, t: u64) {
        match self.cmd >> 4 {
            0 => {
                // Restore: step out until TRACK 00, at most 255 steps.
                if self.drive().is_some_and(|d| d.cyl == 0) {
                    self.track = 0;
                    return self.end_type1(t);
                }
                if self.steps == 255 {
                    self.status |= RNF_SEEK;
                    return self.finish(t);
                }
                self.steps += 1;
                self.step_out = true;
                self.step_head(t);
                self.next = t + self.step_cycles();
            }
            1 => {
                // Seek to the track number in the data register.
                if self.track == self.data {
                    return self.end_type1(t);
                }
                self.step_out = self.data < self.track;
                self.track = if self.step_out { self.track - 1 } else { self.track + 1 };
                self.step_head(t);
                self.next = t + self.step_cycles();
            }
            _ => self.end_type1(t),
        }
    }

    fn end_type1(&mut self, t: u64) {
        if self.cmd & 0x04 != 0 {
            self.op = Op::Verify;
            self.next = t + SETTLE;
        } else {
            self.finish(t);
        }
    }

    /// Write Track: stores the sectors of the written stream (F5 = A1 sync,
    /// FE = ID mark, FB/F8 = data mark) that the image geometry can hold.
    fn commit_track(&mut self) {
        let (cyl, side) = (self.drives[self.select].cyl, self.side);
        let raw = std::mem::take(&mut self.buf);
        let Some(disk) = self.drives[self.select].disk.as_mut() else { return };
        let mut i = 0;
        while i + 8 <= raw.len() {
            if raw[i..i + 4] != [0xF5, 0xF5, 0xF5, 0xFE] {
                i += 1;
                continue;
            }
            let (c, h, r, n) = (raw[i + 4], raw[i + 5], raw[i + 6], raw[i + 7]);
            let len = 128usize << (n & 3);
            let mark = (i + 8..(i + 72).min(raw.len().saturating_sub(4)))
                .find(|&j| raw[j..j + 3] == [0xF5; 3] && matches!(raw[j + 3], 0xFB | 0xF8));
            let Some(j) = mark else {
                i += 8;
                continue;
            };
            let data = &raw[(j + 4).min(raw.len())..(j + 4 + len).min(raw.len())];
            let fits = c == cyl && h == side && c < CYLINDERS && data.len() == SECTOR_SIZE;
            if !(fits && disk.write_sector(cyl, side, r, data)) {
                self.format_errors += 1;
            }
            i = j + 4 + data.len();
        }
    }
}

/// Op values in snapshots, by position.
const OPS: [Op; 10] = [
    Op::Idle,
    Op::Step,
    Op::Verify,
    Op::Search,
    Op::ReadBytes,
    Op::WriteCheck,
    Op::WriteBytes,
    Op::TrackWait,
    Op::TrackBytes,
    Op::Finish,
];

/// Snapshot state (spec §4.6).
impl Fdc {
    /// Everything but the drives, which are saved one by one.
    pub(crate) fn save(&self, w: &mut Writer) {
        w.bytes(&[self.track, self.sector, self.data, self.cmd, self.status, self.steps, self.side]);
        for b in [self.busy, self.drq, self.intrq, self.type1, self.head_loaded, self.step_out, self.fm] {
            w.bool(b);
        }
        w.u8(OPS.iter().position(|&op| op == self.op).unwrap_or(0) as u8);
        w.u64(self.next);
        w.u64(self.deadline);
        w.opt_u8(self.found);
        w.blob(&self.buf);
        w.u32(self.pos as u32);
        w.opt_u64(self.motor_since);
        w.u8(self.select as u8);
        w.u32(self.format_errors);
    }

    /// Restores a controller with the given drives, at cycle `now`.
    pub(crate) fn load(r: &mut Reader, drives: [Drive; 2], now: u64) -> Option<Fdc> {
        let [track, sector, data, cmd, status, steps, side] = r.array()?;
        let mut flags = [false; 7];
        for f in &mut flags {
            *f = r.bool()?;
        }
        let [busy, drq, intrq, type1, head_loaded, step_out, fm] = flags;
        let op = *OPS.get(r.u8()? as usize)?;
        let (next, deadline) = (r.u64()?, r.u64()?);
        let found = r.opt_u8()?;
        let buf = r.blob()?.to_vec();
        let pos = r.u32()? as usize;
        let motor_since = r.opt_u64()?;
        let select = r.u8()? as usize;
        let format_errors = r.u32()?;
        // The states the command steps index `buf` in. Outside ReadBytes
        // `pos` is left over from the last read.
        let ok = side <= 1
            && select <= 1
            && buf.len() <= TRACK_BYTES
            && (op != Op::ReadBytes || pos < buf.len())
            && (op != Op::Finish || cmd >> 4 != 0xC || !buf.is_empty())
            && motor_since.is_none_or(|t| t <= now);
        ok.then_some(Fdc {
            drives,
            track,
            sector,
            data,
            cmd,
            status,
            busy,
            drq,
            intrq,
            type1,
            head_loaded,
            op,
            next,
            deadline,
            found,
            steps,
            step_out,
            buf,
            pos,
            motor_since,
            select,
            side,
            fm,
            format_errors,
            sounds: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk() -> Disk {
        let mut data = vec![0u8; 819_200];
        for (i, chunk) in data.chunks_mut(SECTOR_SIZE).enumerate() {
            chunk.fill(i as u8);
        }
        Disk::new(data, false).unwrap()
    }

    /// An FDC with a disk in drive 0, motor on since 0, run up to `now`.
    fn fdc_ready() -> (Fdc, u64) {
        let mut f = Fdc::new();
        f.drives[0].insert(disk(), 0);
        f.set_control(0, true, 0, 0, false);
        (f, 2 * REV_CYCLES)
    }

    /// Reads the data register as the stalled CPU would: waits for DRQ.
    fn read_data(f: &mut Fdc, now: &mut u64) -> u8 {
        while f.wants_wait() {
            *now = f.next_event();
            f.advance(*now);
        }
        f.read(*now, 3)
    }

    fn wait_idle(f: &mut Fdc, now: &mut u64) {
        while f.busy() {
            *now = f.next_event();
            f.advance(*now);
        }
    }

    /// The controller and its drives as snapshot bytes.
    fn saved(f: &Fdc) -> Vec<u8> {
        let mut w = Writer::new();
        f.save(&mut w);
        for d in &f.drives {
            d.save(&mut w);
        }
        w.into_bytes()
    }

    fn restored(bytes: &[u8], now: u64) -> Fdc {
        let mut r = Reader::new(bytes);
        let mut f = Fdc::load(&mut r, Default::default(), now).unwrap();
        f.drives = [Drive::load(&mut r, now).unwrap(), Drive::load(&mut r, now).unwrap()];
        r.end().unwrap();
        f
    }

    #[test]
    fn snapshot_round_trip_through_every_step() {
        let (mut f, mut now) = fdc_ready();
        let mut ops = vec![f.op];
        // Seek with verify, read and write a sector, read an ID, read and
        // write a track: every Op comes up.
        f.write(now, 3, 5);
        for (cmd, sector) in [(0x16, 1), (0x80, 3), (0xA0, 4), (0xC0, 1), (0xE4, 1), (0xF4, 1)] {
            f.write(now, 2, sector);
            f.write(now, 0, cmd);
            let mut step = 0;
            while f.busy() {
                if !ops.contains(&f.op) || step % 97 == 0 {
                    ops.push(f.op);
                    let bytes = saved(&f);
                    assert_eq!(saved(&restored(&bytes, now)), bytes, "command {cmd:02X}, {:?}", f.op);
                }
                if f.drq() {
                    if cmd & 0x20 != 0 {
                        f.write(now, 3, 0x5A);
                    } else {
                        f.read(now, 3);
                    }
                }
                now = f.next_event();
                f.advance(now);
                step += 1;
            }
        }
        assert!(OPS.iter().all(|op| ops.contains(op)), "{ops:?}");
        assert_eq!(f.drives[0].disk.as_ref().unwrap().sector(5, 0, 4).unwrap()[0], 0x5A);
    }

    #[test]
    fn crc_of_id_mark() {
        // Known value: CRC of A1 A1 A1 FE 00 00 01 02.
        assert_eq!(crc16(&[0xA1, 0xA1, 0xA1, 0xFE, 0, 0, 1, 2]), 0xCA6F);
    }

    #[test]
    fn not_ready_without_motor() {
        let mut f = Fdc::new();
        f.drives[0].insert(disk(), 0);
        assert_eq!(f.read(10 * REV_CYCLES, 0) & NOT_READY, NOT_READY);
        f.write(10 * REV_CYCLES, 0, 0x80);
        assert!(!f.busy());
        assert_eq!(f.read(10 * REV_CYCLES, 0), NOT_READY);
    }

    #[test]
    fn seek_then_read_sector() {
        let (mut f, mut now) = fdc_ready();
        f.write(now, 0, 0x02); // Restore, 20 ms steps
        wait_idle(&mut f, &mut now);
        assert_eq!(f.read(now, 0) & LOST_TR00, LOST_TR00, "TRACK 00");

        f.write(now, 3, 5);
        let start = now;
        f.write(now, 0, 0x12); // Seek to 5
        wait_idle(&mut f, &mut now);
        assert_eq!(f.read(now, 1), 5);
        assert_eq!(now - start, 5 * 20 * MS);
        assert_eq!(f.drives[0].cyl, 5);

        f.write(now, 2, 3);
        f.write(now, 0, 0x80);
        let mut sector = Vec::new();
        for _ in 0..SECTOR_SIZE {
            sector.push(read_data(&mut f, &mut now));
        }
        wait_idle(&mut f, &mut now);
        assert_eq!(sector, vec![(5 * 2 * 10 + 2) as u8; SECTOR_SIZE]);
        assert_eq!(f.read(now, 0) & 0x9C, 0);
    }

    #[test]
    fn missing_sector_is_record_not_found() {
        let (mut f, mut now) = fdc_ready();
        f.write(now, 2, 11);
        let start = now;
        f.write(now, 0, 0x80);
        wait_idle(&mut f, &mut now);
        assert_eq!(f.read(now, 0) & RNF_SEEK, RNF_SEEK);
        assert_eq!(now - start, SEARCH_REVS * REV_CYCLES);
    }

    #[test]
    fn side_compare_and_side_select() {
        let (mut f, mut now) = fdc_ready();
        f.set_control(now, true, 0, 1, false);
        f.write(now, 2, 1);
        f.write(now, 0, 0x8A); // read, side compare, expect side 1
        let b = read_data(&mut f, &mut now);
        assert_eq!(b, 10, "cylinder 0, head 1, sector 1");
        wait_idle(&mut f, &mut now);
        f.write(now, 0, 0x82); // expects side 0: not found
        wait_idle(&mut f, &mut now);
        assert_eq!(f.read(now, 0) & RNF_SEEK, RNF_SEEK);
    }

    #[test]
    fn write_sector_and_lost_data() {
        let (mut f, mut now) = fdc_ready();
        f.write(now, 2, 2);
        f.write(now, 0, 0xA0);
        for i in 0..SECTOR_SIZE {
            while f.wants_wait() {
                now = f.next_event();
                f.advance(now);
            }
            f.write(now, 3, i as u8);
        }
        wait_idle(&mut f, &mut now);
        assert_eq!(f.read(now, 0) & 0xFC, 0);
        let d = f.drives[0].disk.as_ref().unwrap();
        assert!(d.dirty());
        assert_eq!(d.sector(0, 0, 2).unwrap()[100], 100);

        // No data supplied at all: LOST DATA, nothing written.
        f.write(now, 2, 4);
        f.write(now, 0, 0xA0);
        wait_idle(&mut f, &mut now);
        assert_eq!(f.read(now, 0) & LOST_TR00, LOST_TR00);
        assert_eq!(f.drives[0].disk.as_ref().unwrap().sector(0, 0, 4).unwrap()[0], 3);
    }

    #[test]
    fn write_protected_disk_refuses_writes() {
        let mut f = Fdc::new();
        f.drives[0].insert(Disk::new(vec![0; 819_200], true).unwrap(), 0);
        f.set_control(0, true, 0, 0, false);
        let now = 2 * REV_CYCLES;
        f.write(now, 0, 0xA0);
        assert!(!f.busy());
        assert_eq!(f.read(now, 0) & WRITE_PROTECT, WRITE_PROTECT);
    }

    #[test]
    fn read_track_matches_write_track_format() {
        let (mut f, mut now) = fdc_ready();
        f.write(now, 0, 0xE4);
        let track: Vec<u8> = (0..TRACK_BYTES).map(|_| read_data(&mut f, &mut now)).collect();
        wait_idle(&mut f, &mut now);
        assert_eq!(&track[32..36], &[0xC2, 0xC2, 0xC2, 0xFC], "index mark");
        assert_eq!(&track[FIRST_SLOT + 12..FIRST_SLOT + 20], &[0xA1, 0xA1, 0xA1, 0xFE, 0, 0, 1, 2]);
        assert_eq!(track[FIRST_SLOT + DATA_START], 0, "sector 1 data");
        assert_eq!(&track[FIRST_SLOT + SLOT - 4..FIRST_SLOT + SLOT], &[0x4E, 0xA1, 0xA1, 0xA1], "end of gap 3");
        assert_eq!(&track[FIRST_SLOT + SLOT + 12..FIRST_SLOT + SLOT + 20], &[0xA1, 0xA1, 0xA1, 0xFE, 0, 0, 2, 2]);
        assert_eq!(track[FIRST_SLOT + SLOT + DATA_START], 1, "sector 2 data");
        assert_eq!(FIRST_SLOT + SECTORS as usize * SLOT + 103, FORMATTED);
        assert_eq!(&track[FORMATTED - 1..FORMATTED + 1], &[0x4E, 0], "where the BIOS stops writing");

        // Format cylinder 0 side 0 as FORMAT.COM does: its table, 6161 bytes
        // of it as the BIOS writes them (MD §10.3).
        let mut stream = vec![0x4E; 32];
        stream.extend_from_slice(&[0xF6, 0xF6, 0xF6, 0xFC]);
        stream.extend_from_slice(&[0x4E; 32]);
        for id in 1..=SECTORS {
            stream.extend_from_slice(&[0; 12]);
            stream.extend_from_slice(&[0xF5, 0xF5, 0xF5, 0xFE, 0, 0, id, 2, 0xF7]);
            stream.extend_from_slice(&[0x4E; 22]);
            stream.extend_from_slice(&[0; 12]);
            stream.extend_from_slice(&[0xF5, 0xF5, 0xF5, 0xFB]);
            stream.extend_from_slice(&[0xE5; SECTOR_SIZE]);
            stream.extend_from_slice(&[0xF7]);
            stream.extend_from_slice(&[0x4E; 24]);
            stream.extend_from_slice(&[0xF5; 3]);
        }
        stream.resize(6161, 0x4E);
        f.write(now, 0, 0xF4);
        for &b in &stream {
            while f.wants_wait() {
                now = f.next_event();
                f.advance(now);
            }
            f.write(now, 3, b);
        }
        wait_idle(&mut f, &mut now);
        assert_eq!(f.format_errors, 0);
        let d = f.drives[0].disk.as_ref().unwrap();
        assert_eq!(d.sector(0, 0, 1).unwrap(), &[0xE5; SECTOR_SIZE]);
        assert_eq!(d.sector(0, 0, 10).unwrap(), &[0xE5; SECTOR_SIZE]);
        assert_eq!(d.sector(0, 1, 1).unwrap()[0], 10, "other side untouched");
    }

    #[test]
    fn force_interrupt_stops_a_command() {
        let (mut f, now) = fdc_ready();
        f.write(now, 2, 11);
        f.write(now, 0, 0x80);
        assert!(f.busy());
        f.write(now + 10, 0, 0xD0);
        assert!(!f.busy());
        assert_eq!(f.read(now + 10, 0) & BUSY, 0);
    }
}

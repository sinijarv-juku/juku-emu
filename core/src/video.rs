//! Video: the raster the beam draws, built µs by µs from the timer outputs
//! (MD §7.1–7.2), and the frame published to the host once per vertical
//! sync.
//!
//! The hardware, as the 1987 schematic draws it:
//! - The video address counter steps once per µs while HOR RTR is high,
//!   whatever VER RTR and POF do, and is loaded with the base address
//!   (DIP switch S3) on the rising edge of VER RTR.
//! - A byte is shown only while both HOR RTR and VER RTR are high; POF blanks
//!   the output and nothing else.
//! - The monitor places the picture by the sync pulses: x is the time since
//!   the start of horizontal sync, y the number of lines since the start of
//!   vertical sync.
//!
//! [`Raster`] keeps that state and draws lazily: the bytes shown between two
//! events (a change of HOR RTR, VER RTR or POF, a sync pulse, a CPU write that
//! may race the beam) are copied from RAM in one run when the later event
//! arrives. Time is counted in µs, the pulses of PIT #1.
//!
//! The Juku framebuffer is 1 bit per pixel, bit 7 = leftmost pixel, 1 = lit.
//! The host renderer receives the raster and expands the bits on the GPU.

/// Bytes (µs) per raster line: one line period from the start of
/// horizontal sync.
pub const RASTER_BYTES: usize = 64;
/// Raster lines from the start of vertical sync: one frame of 313 lines.
pub const RASTER_LINES: usize = 313;
pub const RASTER_SIZE: usize = RASTER_BYTES * RASTER_LINES;

/// Without a horizontal sync pulse the monitor's line oscillator runs on by
/// itself and starts a new line after this many µs, a little slower than
/// the 64 µs of the Juku's lines.
const FREE_RUN_LINE: u64 = 72;
/// Likewise the frame oscillator without vertical sync, in lines.
const FREE_RUN_LINES: u32 = 340;

/// Fixed video base address: DIP switch S3 = 110110 (MD §7.1).
pub const VIDEO_BASE: u16 = 0xD800;

/// Default geometry as programmed by RomBios (40 bytes × 241 lines = 320×241).
pub const DEFAULT_BYTES_PER_LINE: u16 = 40;
pub const DEFAULT_LINES: u16 = 241;
/// Time from the start of horizontal sync to the first active pixel, µs.
pub const DEFAULT_HPORCH_US: u16 = 16;
/// Lines from the start of vertical sync to the first active line.
pub const DEFAULT_VPORCH_LINES: u16 = 47;

/// A completed frame, shared with the host through the C ABI.
///
/// Must stay in sync with `JukuFrame` in `frontends/macos/src/Core.m`.
#[repr(C)]
pub struct Frame {
    /// Increments once per completed frame.
    pub seq: u64,
    /// Where the timer counts put the picture inside the raster: its first
    /// byte (µs after the start of horizontal sync), first line (after the
    /// start of vertical sync), bytes per line and lines. A program that
    /// changes the timing during the frame can draw outside it.
    pub active_x: u16,
    pub active_y: u16,
    pub active_w: u16,
    pub active_h: u16,
    pub _pad: [u8; 8],
    /// `RASTER_LINES` lines of `RASTER_BYTES` bytes: what the beam drew.
    pub data: [u8; RASTER_SIZE],
}

const _: () = assert!(core::mem::size_of::<Frame>() == 24 + RASTER_SIZE);
const _: () = assert!(core::mem::offset_of!(Frame, data) == 24);

impl Frame {
    pub fn new_boxed() -> Box<Frame> {
        let g = Geometry::default();
        Box::new(Frame {
            seq: 0,
            active_x: g.hporch_us,
            active_y: g.vporch_lines,
            active_w: g.bytes_per_line,
            active_h: g.lines,
            _pad: [0; 8],
            data: [0; RASTER_SIZE],
        })
    }

    /// The active area as `active_h` lines of `active_line(0).len()` bytes.
    pub fn active_picture(&self) -> Vec<u8> {
        (0..self.active_h as usize).flat_map(|y| self.active_line(y).iter().copied()).collect()
    }

    /// Line `y` of the active area (see `active_x`), clipped to the raster.
    pub fn active_line(&self, y: usize) -> &[u8] {
        let row = (self.active_y as usize + y).min(RASTER_LINES - 1);
        let x = (self.active_x as usize).min(RASTER_BYTES);
        let end = (x + self.active_w as usize).min(RASTER_BYTES);
        &self.data[row * RASTER_BYTES..][x..end]
    }
}

/// Raster geometry derived from the 8253 counts (MD §7.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub bytes_per_line: u16,
    pub lines: u16,
    pub hporch_us: u16,
    pub vporch_lines: u16,
}

impl Default for Geometry {
    fn default() -> Geometry {
        Geometry {
            bytes_per_line: DEFAULT_BYTES_PER_LINE,
            lines: DEFAULT_LINES,
            hporch_us: DEFAULT_HPORCH_US,
            vporch_lines: DEFAULT_VPORCH_LINES,
        }
    }
}

impl Geometry {
    /// From the programmed counts of PIT #1 ch0–2 (line period, horizontal
    /// blank, horizontal sync delay; µs) and PIT #2 ch0–2 (frame period,
    /// vertical blank, vertical sync delay; lines). Falls back to the default
    /// for anything missing or out of range.
    pub fn from_counts(h: [Option<u32>; 3], v: [Option<u32>; 3]) -> Geometry {
        let d = Geometry::default();
        let span = |total: Option<u32>, blank: Option<u32>, max: u32| match (total, blank) {
            (Some(t), Some(b)) if t > b && t - b <= max => Some((t - b) as u16),
            _ => None,
        };
        let porch = |blank: Option<u32>, delay: Option<u32>| match (blank, delay) {
            (Some(b), Some(s)) if b >= s => Some((b - s) as u16),
            _ => None,
        };
        Geometry {
            bytes_per_line: span(h[0], h[1], RASTER_BYTES as u32).unwrap_or(d.bytes_per_line),
            lines: span(v[0], v[1], RASTER_LINES as u32).unwrap_or(d.lines),
            hporch_us: porch(h[1], h[2]).filter(|&p| (p as usize) < RASTER_BYTES).unwrap_or(d.hporch_us),
            vporch_lines: porch(v[1], v[2]).filter(|&p| (p as usize) < RASTER_LINES).unwrap_or(d.vporch_lines),
        }
    }
}

/// The beam, the address counter and the frame being drawn.
pub struct Raster {
    /// Double-buffered frames; `front` is the last completed one, the other
    /// is being drawn.
    frames: [Box<Frame>; 2],
    front: usize,
    seq: u64,
    /// The active area last given to [`Raster::vsync`], for frames that end
    /// without one.
    geom: Geometry,
    /// Time, in µs, up to which the raster is drawn.
    t: u64,
    /// Start of the current line: the last horizontal sync, or where the
    /// line oscillator ran on without one.
    line_t: u64,
    /// Lines since the start of vertical sync.
    y: u32,
    /// The video address counter.
    addr: u16,
    /// HOR RTR, VER RTR and POF since `t`.
    hor: bool,
    ver: bool,
    pof: bool,
    /// While bytes are shown, RAM writes to `watch..watch + watch_len`
    /// (wrapping) may hit a byte the beam has fetched since `t` but that is
    /// not drawn yet; the machine draws up to the write first. Empty when
    /// nothing is shown.
    pub watch: u16,
    pub watch_len: u16,
}

/// The state of a [`Raster`], for snapshots.
pub struct RasterState<'a> {
    pub t: u64,
    pub line_t: u64,
    pub y: u32,
    pub addr: u16,
    pub hor: bool,
    pub ver: bool,
    pub pof: bool,
    pub geom: Geometry,
    /// The last completed frame and the one being drawn.
    pub front: &'a [u8],
    pub back: &'a [u8],
}

impl Raster {
    pub fn new() -> Raster {
        Raster {
            frames: [Frame::new_boxed(), Frame::new_boxed()],
            front: 0,
            seq: 0,
            geom: Geometry::default(),
            t: 0,
            line_t: 0,
            y: 0,
            addr: VIDEO_BASE,
            hor: false,
            ver: false,
            pof: false,
            watch: 0,
            watch_len: 0,
        }
    }

    /// The last completed frame.
    pub fn frame(&self) -> &Frame {
        &self.frames[self.front]
    }

    /// Takes the timer and POF levels as they are after a reset, at `now`.
    pub fn set_levels(&mut self, now: u64, ram: &[u8], hor: bool, ver: bool, pof: bool) {
        self.advance(now, ram);
        self.hor = hor;
        self.ver = ver;
        self.pof = pof;
        self.update_watch();
    }

    /// Draws everything up to `now` (µs).
    pub fn advance(&mut self, now: u64, ram: &[u8]) {
        while self.t < now {
            let x = self.t - self.line_t;
            if x >= FREE_RUN_LINE {
                // No sync: the monitor's line oscillator starts a line.
                self.line_t += FREE_RUN_LINE;
                self.next_line();
                continue;
            }
            let n = (now - self.t).min(FREE_RUN_LINE - x);
            if self.hor {
                if self.ver && !self.pof {
                    self.draw(x as usize, n as usize, ram);
                }
                self.addr = self.addr.wrapping_add(n as u16);
            }
            self.t += n;
        }
        self.update_watch();
    }

    /// Copies the `n` bytes shown from `x` on the current line.
    fn draw(&mut self, x: usize, n: usize, ram: &[u8]) {
        let y = self.y as usize;
        if y >= RASTER_LINES || x >= RASTER_BYTES {
            return;
        }
        let count = n.min(RASTER_BYTES - x);
        let row = &mut self.frames[1 - self.front].data[y * RASTER_BYTES..][x..x + count];
        let a = self.addr as usize;
        let first = count.min(0x10000 - a);
        row[..first].copy_from_slice(&ram[a..a + first]);
        row[first..].copy_from_slice(&ram[..count - first]);
    }

    fn update_watch(&mut self) {
        let shown = self.hor && self.ver && !self.pof;
        self.watch = self.addr;
        self.watch_len = if shown { RASTER_BYTES as u16 } else { 0 };
    }

    fn next_line(&mut self) {
        self.y += 1;
        if self.y >= FREE_RUN_LINES {
            // No vertical sync: the frame oscillator starts a frame.
            self.publish(self.geom);
            self.y = 0;
        }
    }

    pub fn set_hor(&mut self, now: u64, ram: &[u8], level: bool) {
        self.advance(now, ram);
        self.hor = level;
        self.update_watch();
    }

    /// VER RTR; its rising edge loads the address counter.
    pub fn set_ver(&mut self, now: u64, ram: &[u8], level: bool) {
        self.advance(now, ram);
        if level && !self.ver {
            self.addr = VIDEO_BASE;
        }
        self.ver = level;
        self.update_watch();
    }

    pub fn set_pof(&mut self, now: u64, ram: &[u8], on: bool) {
        self.advance(now, ram);
        self.pof = on;
        self.update_watch();
    }

    pub fn pof(&self) -> bool {
        self.pof
    }

    /// The start of horizontal sync: a new line.
    pub fn hsync(&mut self, now: u64, ram: &[u8]) {
        self.advance(now, ram);
        self.line_t = now;
        self.next_line();
    }

    /// The start of vertical sync: the frame is complete and published with
    /// `geom` as its active area, and a new one starts.
    pub fn vsync(&mut self, now: u64, ram: &[u8], geom: Geometry) {
        self.advance(now, ram);
        self.geom = geom;
        self.publish(geom);
        self.y = 0;
    }

    fn publish(&mut self, geom: Geometry) {
        self.seq += 1;
        let back = 1 - self.front;
        let f = &mut self.frames[back];
        f.seq = self.seq;
        f.active_x = geom.hporch_us;
        f.active_y = geom.vporch_lines;
        f.active_w = geom.bytes_per_line;
        f.active_h = geom.lines;
        self.front = back;
        self.frames[1 - back].data.fill(0);
    }

    pub fn state(&self) -> RasterState<'_> {
        RasterState {
            t: self.t,
            line_t: self.line_t,
            y: self.y,
            addr: self.addr,
            hor: self.hor,
            ver: self.ver,
            pof: self.pof,
            geom: self.geom,
            front: &self.frames[self.front].data,
            back: &self.frames[1 - self.front].data,
        }
    }

    /// Puts back a saved state. `seq` keeps counting up, and the saved last
    /// frame is published as a new one, so the host sees it.
    pub fn restore(&mut self, s: &RasterState) {
        self.t = s.t;
        self.line_t = s.line_t;
        self.y = s.y;
        self.addr = s.addr;
        self.hor = s.hor;
        self.ver = s.ver;
        self.pof = s.pof;
        self.geom = s.geom;
        let back = 1 - self.front;
        self.frames[back].data.copy_from_slice(s.front);
        self.publish(s.geom);
        let back = 1 - self.front;
        self.frames[back].data.copy_from_slice(s.back);
        self.update_watch();
    }

    /// Checks a saved state for values that could not have been reached.
    pub fn state_ok(s: &RasterState, now_us: u64) -> bool {
        s.t <= now_us
            && s.line_t <= s.t
            && s.t - s.line_t <= FREE_RUN_LINE
            && s.y < FREE_RUN_LINES
            && s.front.len() == RASTER_SIZE
            && s.back.len() == RASTER_SIZE
    }
}

impl Default for Raster {
    fn default() -> Raster {
        Raster::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rombios_counts_give_320x241() {
        let g = Geometry::from_counts([Some(64), Some(24), Some(8)], [Some(313), Some(72), Some(25)]);
        assert_eq!(g, Geometry::default());
    }

    #[test]
    fn bad_counts_fall_back_to_default() {
        let g = Geometry::from_counts([Some(64), None, Some(8)], [Some(10), Some(72), Some(25)]);
        assert_eq!(g, Geometry::default());
        let g = Geometry::from_counts([Some(64), Some(16), Some(8)], [Some(313), Some(113), Some(30)]);
        assert_eq!((g.bytes_per_line, g.lines, g.hporch_us, g.vporch_lines), (48, 200, 8, 83));
    }

    /// RAM with each byte holding the low byte of its address, from D800h.
    fn ram() -> Vec<u8> {
        let mut ram = vec![0u8; 0x10000];
        for (i, b) in ram.iter_mut().enumerate().skip(VIDEO_BASE as usize) {
            *b = (i as u8) | 1;
        }
        ram
    }

    /// One line as the BIOS times it, starting at `t`: sync at +8 µs,
    /// display from +24 to +64.
    fn line(r: &mut Raster, ram: &[u8], t: u64) {
        r.hsync(t + 8, ram);
        r.set_hor(t + 24, ram, true);
        r.set_hor(t + 64, ram, false);
    }

    #[test]
    fn draws_a_line_from_the_sync_and_counts_only_while_hor_is_high() {
        let ram = ram();
        let mut r = Raster::new();
        r.set_ver(0, &ram, true);
        line(&mut r, &ram, 0);
        line(&mut r, &ram, 64);
        r.vsync(200, &ram, Geometry::default());
        let f = r.frame();
        // The first line is y = 1 (vsync starts line 0), x = 16 after sync.
        let row = &f.data[RASTER_BYTES..][..RASTER_BYTES];
        assert_eq!(row[15], 0);
        assert_eq!(row[16], 0x01);
        assert_eq!(row[55], 39 | 1);
        assert_eq!(row[56], 0);
        let row = &f.data[2 * RASTER_BYTES..][..RASTER_BYTES];
        assert_eq!(row[16], 40 | 1, "the counter carries on from the line before");
    }

    #[test]
    fn ver_low_counts_without_drawing_and_its_rising_edge_reloads() {
        let ram = ram();
        let mut r = Raster::new();
        line(&mut r, &ram, 0); // VER low: counted, not drawn
        r.set_ver(66, &ram, true);
        line(&mut r, &ram, 64);
        r.vsync(140, &ram, Geometry::default());
        let f = r.frame();
        assert!(f.data[RASTER_BYTES..][..RASTER_BYTES].iter().all(|&b| b == 0));
        assert_eq!(f.data[2 * RASTER_BYTES + 16], 0x01, "reloaded to D800h");
    }

    #[test]
    fn pof_blanks_but_the_counter_runs() {
        let ram = ram();
        let mut r = Raster::new();
        r.set_ver(0, &ram, true);
        r.set_pof(0, &ram, true);
        line(&mut r, &ram, 0);
        r.set_pof(66, &ram, false);
        line(&mut r, &ram, 64);
        r.vsync(140, &ram, Geometry::default());
        let f = r.frame();
        assert!(f.data[RASTER_BYTES..][..RASTER_BYTES].iter().all(|&b| b == 0));
        assert_eq!(f.data[2 * RASTER_BYTES + 16], 40 | 1);
    }

    #[test]
    fn a_gap_in_hor_rtr_holds_the_counter() {
        let ram = ram();
        let mut r = Raster::new();
        r.set_ver(0, &ram, true);
        r.hsync(8, &ram);
        r.set_hor(24, &ram, true);
        r.set_hor(30, &ram, false); // 6 bytes, then a 4 µs gap
        r.set_hor(34, &ram, true);
        r.set_hor(64, &ram, false);
        r.vsync(100, &ram, Geometry::default());
        let row = &r.frame().data[RASTER_BYTES..][..RASTER_BYTES];
        assert_eq!(&row[16..22], &[1, 1, 3, 3, 5, 5]);
        assert_eq!(&row[22..26], &[0; 4]);
        assert_eq!(row[26], 6 | 1, "the byte after the gap is the next one");
    }

    #[test]
    fn the_watch_covers_the_bytes_being_shown() {
        let ram = ram();
        let mut r = Raster::new();
        r.set_ver(0, &ram, true);
        r.hsync(8, &ram);
        assert_eq!(r.watch_len, 0, "HOR RTR low");
        r.set_hor(24, &ram, true);
        r.advance(30, &ram);
        assert_eq!((r.watch, r.watch_len), (VIDEO_BASE + 6, RASTER_BYTES as u16));
    }

    #[test]
    fn without_sync_the_monitor_runs_on() {
        let ram = ram();
        let mut r = Raster::new();
        let seq = r.frame().seq;
        r.advance(FREE_RUN_LINE * FREE_RUN_LINES as u64, &ram);
        assert_eq!(r.frame().seq, seq, "one line short");
        r.advance(FREE_RUN_LINE * FREE_RUN_LINES as u64 + 1, &ram);
        assert_eq!(r.frame().seq, seq + 1);
    }

    #[test]
    fn active_line_is_the_active_area() {
        let mut f = Frame::new_boxed();
        f.data[47 * RASTER_BYTES + 16] = 0xAA;
        assert_eq!(f.active_line(0).len(), 40);
        assert_eq!(f.active_line(0)[0], 0xAA);
    }
}

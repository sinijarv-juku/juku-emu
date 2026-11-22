//! Floppy drives and disk images.
//!
//! Images use MAME's `.juk` layout (MAME `juku_dsk.cpp`): raw 512-byte
//! sectors, 10 per track with IDs 1–10, 80 cylinders, one or two heads,
//! track-major with the heads interleaved. There is no header.

use crate::machine::CPU_HZ;
use crate::snapshot::{Reader, Writer};

pub const SECTOR_SIZE: usize = 512;
pub const SECTORS: u8 = 10;
pub const CYLINDERS: u8 = 80;
/// The head can travel a few cylinders past the last formatted one.
pub const MAX_CYLINDER: u8 = 83;

/// One revolution at 300 rpm, in CPU cycles.
pub const REV_CYCLES: u64 = CPU_HZ / 5;
/// Width of the index pulse.
pub const INDEX_CYCLES: u64 = CPU_HZ / 250;
/// What a freshly formatted sector holds. EKDOS, like CP/M, takes a
/// directory of E5h bytes as empty.
pub const FORMAT_FILL: u8 = 0xE5;
/// The size of a double-sided image.
pub const DOUBLE_SIDED: usize = 2 * CYLINDERS as usize * SECTORS as usize * SECTOR_SIZE;

/// A drive reports READY after this many revolutions with motor and disk.
const SPIN_UP_REVS: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskError {
    /// The image size matches no supported geometry.
    BadSize,
}

pub struct Disk {
    data: Vec<u8>,
    heads: u8,
    write_protect: bool,
    dirty: bool,
}

impl Disk {
    /// Accepts 819 200-byte (double-sided) and 409 600-byte (single-sided)
    /// images.
    pub fn new(data: Vec<u8>, write_protect: bool) -> Result<Disk, DiskError> {
        let heads = Disk::heads_for(data.len()).ok_or(DiskError::BadSize)?;
        Ok(Disk { data, heads, write_protect, dirty: false })
    }

    /// A new double-sided disk, formatted and empty.
    pub fn blank() -> Disk {
        Disk { data: vec![FORMAT_FILL; DOUBLE_SIDED], heads: 2, write_protect: false, dirty: false }
    }

    /// True if an image `len` bytes long is a size the drives take.
    pub fn size_ok(len: usize) -> bool {
        Disk::heads_for(len).is_some()
    }

    /// The number of heads of an image `len` bytes long, if it is a size
    /// the drives take.
    fn heads_for(len: usize) -> Option<u8> {
        let side = SECTORS as usize * SECTOR_SIZE * CYLINDERS as usize;
        match len {
            n if n == side * 2 => Some(2),
            n if n == side => Some(1),
            _ => None,
        }
    }

    pub fn heads(&self) -> u8 {
        self.heads
    }

    pub fn write_protect(&self) -> bool {
        self.write_protect
    }

    pub fn set_write_protect(&mut self, on: bool) {
        self.write_protect = on;
    }

    fn offset(&self, cyl: u8, head: u8, id: u8) -> Option<usize> {
        if cyl >= CYLINDERS || head >= self.heads || !(1..=SECTORS).contains(&id) {
            return None;
        }
        let index = (cyl as usize * self.heads as usize + head as usize) * SECTORS as usize + (id - 1) as usize;
        Some(index * SECTOR_SIZE)
    }

    /// True if the track under (cyl, head) is formatted.
    pub fn has_track(&self, cyl: u8, head: u8) -> bool {
        self.offset(cyl, head, 1).is_some()
    }

    pub fn sector(&self, cyl: u8, head: u8, id: u8) -> Option<&[u8]> {
        self.offset(cyl, head, id).map(|o| &self.data[o..o + SECTOR_SIZE])
    }

    pub fn write_sector(&mut self, cyl: u8, head: u8, id: u8, data: &[u8]) -> bool {
        match self.offset(cyl, head, id) {
            Some(o) if data.len() == SECTOR_SIZE => {
                self.data[o..o + SECTOR_SIZE].copy_from_slice(data);
                self.dirty = true;
                true
            }
            _ => false,
        }
    }

    /// The whole image, for saving to a file.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }
}

/// A 5.25" 80-track double-sided drive (ЕС 5323.01).
#[derive(Default)]
pub struct Drive {
    pub disk: Option<Disk>,
    /// Head position.
    pub cyl: u8,
    /// Cycle at which the current disk was inserted.
    inserted_at: u64,
}

impl Drive {
    pub fn insert(&mut self, disk: Disk, now: u64) {
        self.disk = Some(disk);
        self.inserted_at = now;
    }

    pub fn eject(&mut self) -> Option<Disk> {
        self.disk.take()
    }

    /// Steps the head one cylinder. Returns false if it was against its
    /// stop and did not move.
    pub fn step(&mut self, outward: bool) -> bool {
        let cyl = if outward { self.cyl.saturating_sub(1) } else { (self.cyl + 1).min(MAX_CYLINDER) };
        std::mem::replace(&mut self.cyl, cyl) != cyl
    }

    /// READY: disk present and spinning for a while. `motor_since` is when
    /// the shared motor line came on, if it is on.
    pub fn ready(&self, now: u64, motor_since: Option<u64>) -> bool {
        match (self.disk.is_some(), motor_since) {
            (true, Some(t)) => now >= t.max(self.inserted_at) + SPIN_UP_REVS * REV_CYCLES,
            _ => false,
        }
    }

    pub fn write_protect(&self) -> bool {
        self.disk.as_ref().is_some_and(|d| d.write_protect())
    }
}

/// Snapshot state (spec §4.6).
impl Drive {
    pub(crate) fn save(&self, w: &mut Writer) {
        w.u8(self.cyl);
        w.u64(self.inserted_at);
        w.bool(self.disk.is_some());
        if let Some(disk) = &self.disk {
            w.bool(disk.write_protect);
            w.packed(&disk.data);
        }
    }

    /// Restores a drive at cycle `now`. The disk comes back clean: the host
    /// decides which file it belongs to.
    pub(crate) fn load(r: &mut Reader, now: u64) -> Option<Drive> {
        let cyl = r.u8().filter(|&c| c <= MAX_CYLINDER)?;
        let inserted_at = r.u64().filter(|&t| t <= now)?;
        let disk = if r.bool()? {
            let write_protect = r.bool()?;
            Some(Disk::new(r.packed(DOUBLE_SIDED)?, write_protect).ok()?)
        } else {
            None
        };
        Some(Drive { disk, cyl, inserted_at })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_and_offsets() {
        assert_eq!(Disk::new(vec![0; 1000], false).err(), Some(DiskError::BadSize));
        let mut data = vec![0u8; 819_200];
        // Cylinder 1, head 1, sector 3: ((1·2 + 1)·10 + 2)·512.
        data[(3 * 10 + 2) * 512] = 0xAB;
        let d = Disk::new(data, false).unwrap();
        assert_eq!(d.heads(), 2);
        assert_eq!(d.sector(1, 1, 3).unwrap()[0], 0xAB);
        assert!(d.sector(0, 0, 0).is_none());
        assert!(d.sector(0, 0, 11).is_none());
        assert!(d.sector(80, 0, 1).is_none());

        let d = Disk::new(vec![0; 409_600], false).unwrap();
        assert_eq!(d.heads(), 1);
        assert!(d.sector(79, 0, 10).is_some());
        assert!(d.sector(0, 1, 1).is_none());
    }

    #[test]
    fn blank_disks_are_formatted() {
        let d = Disk::blank();
        assert_eq!((d.heads(), d.data().len()), (2, 819_200));
        assert!(d.data().iter().all(|&b| b == FORMAT_FILL));
        assert!(!d.dirty() && !d.write_protect());
    }

    #[test]
    fn writes_mark_dirty() {
        let mut d = Disk::new(vec![0; 409_600], false).unwrap();
        assert!(!d.dirty());
        assert!(d.write_sector(2, 0, 5, &[7; SECTOR_SIZE]));
        assert!(d.dirty());
        assert_eq!(d.data()[(2 * 10 + 4) * 512], 7);
    }

    #[test]
    fn drives_round_trip_with_their_disks_clean() {
        let mut data = vec![0xE5; 819_200];
        data[..3].copy_from_slice(b"EK!");
        let mut drive = Drive::default();
        drive.insert(Disk::new(data, true).unwrap(), 5);
        drive.cyl = 40;
        let disk = drive.disk.as_mut().unwrap();
        assert!(disk.write_sector(3, 1, 7, &[1; SECTOR_SIZE]));
        assert!(disk.dirty());
        let mut w = Writer::new();
        drive.save(&mut w);
        let bytes = w.into_bytes();
        assert!(bytes.len() < 1000, "{} bytes", bytes.len());

        let mut r = Reader::new(&bytes);
        let back = Drive::load(&mut r, 10).unwrap();
        assert!(r.end().is_some());
        assert_eq!(back.cyl, 40);
        let disk = back.disk.as_ref().unwrap();
        assert_eq!(disk.data(), drive.disk.as_ref().unwrap().data());
        assert!(disk.write_protect());
        assert!(!disk.dirty());

        // A size the drives do not take is refused before it is unpacked.
        let mut bad = bytes.clone();
        let at = 1 + 8 + 1 + 1;
        bad[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Drive::load(&mut Reader::new(&bad), 10).is_none());
    }

    #[test]
    fn ready_after_spin_up() {
        let mut drive = Drive::default();
        assert!(!drive.ready(10 * REV_CYCLES, Some(0)));
        drive.insert(Disk::new(vec![0; 409_600], false).unwrap(), 0);
        assert!(!drive.ready(REV_CYCLES, Some(0)));
        assert!(drive.ready(2 * REV_CYCLES, Some(0)));
        assert!(!drive.ready(2 * REV_CYCLES, None));
    }
}

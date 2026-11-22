//! Snapshot files (spec §4.6): the whole machine state inside a PNG.
//!
//! A snapshot is a valid PNG whose picture is a thumbnail of the screen.
//! The state follows the image data, in private ancillary chunks of type
//! `juKu`, each holding one section: a 4-byte tag and the section's bytes.
//! Image viewers skip those chunks; every chunk carries the PNG CRC, so a
//! damaged file is caught before anything is decoded.
//!
//! Section tags in capitals belong to the core. Tags that start with a
//! lowercase letter belong to the host, which the core stores and returns
//! without looking inside. All numbers are little-endian, so a file is the
//! same on every platform.

use crate::lz;
use crate::video::Frame;

/// Format version written into the `SNAP` section, the only one read.
/// Version 4 compresses the memory, the ROMs and the raster's frames as
/// well (`lz`); version 3 compressed the disks only, version 2 nothing, and
/// version 1 also had the active picture (`FRAM`) in place of the beam's
/// raster (`RAST`). No release wrote them.
pub const VERSION: u32 = 4;

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
/// Ancillary, private, safe to copy.
const CHUNK_TYPE: [u8; 4] = *b"juKu";
const IEND: [u8; 4] = *b"IEND";
/// PNG chunk lengths are limited to 2³¹ − 1.
const MAX_CHUNK: usize = 0x7FFF_FFFF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// Not a PNG, or a PNG without a snapshot in it.
    NotSnapshot,
    /// A snapshot, but truncated, failing a CRC or holding impossible state.
    Damaged,
    /// Written by a newer version of the format.
    Version,
    /// Written by an older version of the format, which is no longer read.
    Old,
}

/// CRC-32 as PNG uses it, computed bit by bit: a table would cost a
/// kilobyte for a function that runs once per save or load.
fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc = !0u32;
    for part in parts {
        for &b in *part {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { crc >> 1 ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    b << 16 | a
}

fn put_chunk(out: &mut Vec<u8>, kind: [u8; 4], parts: &[&[u8]]) {
    let len: usize = parts.iter().map(|p| p.len()).sum();
    out.extend_from_slice(&(len as u32).to_be_bytes());
    out.extend_from_slice(&kind);
    for p in parts {
        out.extend_from_slice(p);
    }
    let mut covered: Vec<&[u8]> = vec![&kind];
    covered.extend_from_slice(parts);
    out.extend_from_slice(&crc32(&covered).to_be_bytes());
}

/// A PNG chunk, or a section: its 4-byte type or tag, and its data.
type Chunk<'a> = ([u8; 4], &'a [u8]);

/// Walks the chunks of a PNG file, checking each CRC and that the file ends
/// with IEND. Returns the chunks as (type, data).
fn png_chunks(file: &[u8]) -> Result<Vec<Chunk<'_>>, SnapshotError> {
    if file.len() < 8 || file[..8] != PNG_SIGNATURE {
        return Err(SnapshotError::NotSnapshot);
    }
    let mut chunks = Vec::new();
    let mut rest = &file[8..];
    loop {
        if rest.len() < 12 {
            return Err(SnapshotError::Damaged);
        }
        let len = u32::from_be_bytes(rest[..4].try_into().unwrap()) as usize;
        if len > MAX_CHUNK || rest.len() - 12 < len {
            return Err(SnapshotError::Damaged);
        }
        let kind: [u8; 4] = rest[4..8].try_into().unwrap();
        let data = &rest[8..8 + len];
        let crc = u32::from_be_bytes(rest[8 + len..12 + len].try_into().unwrap());
        if crc != crc32(&[&kind, data]) {
            return Err(SnapshotError::Damaged);
        }
        chunks.push((kind, data));
        rest = &rest[12 + len..];
        if kind == IEND {
            return if rest.is_empty() { Ok(chunks) } else { Err(SnapshotError::Damaged) };
        }
    }
}

/// Builds a snapshot file: the thumbnail PNG with sections added before its
/// IEND chunk.
pub struct SnapshotWriter {
    out: Vec<u8>,
}

impl SnapshotWriter {
    /// Starts from `png`, which must be a whole, valid PNG file. Returns
    /// `None` otherwise.
    pub fn new(png: &[u8]) -> Option<SnapshotWriter> {
        let chunks = png_chunks(png).ok()?;
        if chunks.iter().any(|(kind, _)| *kind == CHUNK_TYPE) {
            return None;
        }
        // IEND is the last 12 bytes.
        Some(SnapshotWriter { out: png[..png.len() - 12].to_vec() })
    }

    pub fn section(&mut self, tag: [u8; 4], data: &[u8]) {
        put_chunk(&mut self.out, CHUNK_TYPE, &[&tag, data]);
    }

    pub fn finish(mut self) -> Vec<u8> {
        put_chunk(&mut self.out, IEND, &[]);
        self.out
    }
}

/// The sections of a snapshot file, in file order.
pub struct Sections<'a> {
    list: Vec<Chunk<'a>>,
}

impl<'a> Sections<'a> {
    /// Parses `file`. The first section must be `SNAP` with a version this
    /// core can read. Duplicate tags make the file damaged.
    pub fn parse(file: &'a [u8]) -> Result<Sections<'a>, SnapshotError> {
        let mut list: Vec<Chunk> = Vec::new();
        for (kind, data) in png_chunks(file)? {
            if kind != CHUNK_TYPE {
                continue;
            }
            if data.len() < 4 {
                return Err(SnapshotError::Damaged);
            }
            let tag: [u8; 4] = data[..4].try_into().unwrap();
            if list.iter().any(|(t, _)| *t == tag) {
                return Err(SnapshotError::Damaged);
            }
            list.push((tag, &data[4..]));
        }
        let Some((tag, head)) = list.first() else {
            return Err(SnapshotError::NotSnapshot);
        };
        if tag != b"SNAP" || head.len() < 4 {
            return Err(SnapshotError::Damaged);
        }
        let version = u32::from_le_bytes(head[..4].try_into().unwrap());
        match version {
            0 => Err(SnapshotError::Damaged),
            v if v < VERSION => Err(SnapshotError::Old),
            v if v > VERSION => Err(SnapshotError::Version),
            _ => Ok(Sections { list }),
        }
    }

    /// The sections as (tag, data), in file order.
    pub fn iter(&self) -> impl Iterator<Item = Chunk<'a>> + '_ {
        self.list.iter().copied()
    }

    pub fn get(&self, tag: &[u8; 4]) -> Option<&'a [u8]> {
        self.list.iter().find(|(t, _)| t == tag).map(|&(_, d)| d)
    }

    /// A reader over section `tag`, or `None` if it is missing.
    pub fn reader(&self, tag: &[u8; 4]) -> Option<Reader<'a>> {
        self.get(tag).map(Reader::new)
    }
}

/// Section `tag` of a snapshot file, typically a host section. `Ok(None)`
/// if the file is a good snapshot without that section.
pub fn section<'a>(file: &'a [u8], tag: &[u8; 4]) -> Result<Option<&'a [u8]>, SnapshotError> {
    Ok(Sections::parse(file)?.get(tag))
}

/// True if `tag` may be used for a host section.
pub fn is_host_tag(tag: &[u8; 4]) -> bool {
    tag[0].is_ascii_lowercase() && tag.iter().all(|b| b.is_ascii_graphic() || *b == b' ')
}

/// A PNG of the frame's active area as the core sees it: 1 bit per pixel,
/// lit = white, one pixel per Juku pixel. The host may supply a
/// better-looking one.
pub fn frame_png(frame: &Frame) -> Vec<u8> {
    let (bpl, lines) = (frame.active_line(0).len(), frame.active_h as usize);
    let mut raw = Vec::with_capacity((bpl + 1) * lines);
    for y in 0..lines {
        raw.push(0); // filter: none
        raw.extend_from_slice(frame.active_line(y));
    }
    // zlib stream of stored deflate blocks.
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(0xFFFF).collect();
    for (i, block) in blocks.iter().enumerate() {
        let len = block.len() as u16;
        z.push((i + 1 == blocks.len()) as u8);
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(bpl as u32 * 8).to_be_bytes());
    ihdr.extend_from_slice(&(lines as u32).to_be_bytes());
    ihdr.extend_from_slice(&[1, 0, 0, 0, 0]); // 1-bit greyscale
    let mut out = PNG_SIGNATURE.to_vec();
    put_chunk(&mut out, *b"IHDR", &[&ihdr]);
    put_chunk(&mut out, *b"IDAT", &[&z]);
    put_chunk(&mut out, IEND, &[]);
    out
}

/// Serializes one section.
///
/// The methods of `Writer` and `Reader` are not inlined: a snapshot is
/// saved or loaded once in a while, and inlining them into every device's
/// `save` and `load` added 3 KB to the app.
#[derive(Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Writer {
        Writer::default()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    #[inline(never)]
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    #[inline(never)]
    pub fn bool(&mut self, v: bool) {
        self.buf.push(v as u8);
    }

    #[inline(never)]
    pub fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline(never)]
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline(never)]
    pub fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline(never)]
    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline(never)]
    pub fn opt_u8(&mut self, v: Option<u8>) {
        self.bool(v.is_some());
        self.u8(v.unwrap_or(0));
    }

    #[inline(never)]
    pub fn opt_u16(&mut self, v: Option<u16>) {
        self.bool(v.is_some());
        self.u16(v.unwrap_or(0));
    }

    #[inline(never)]
    pub fn opt_u64(&mut self, v: Option<u64>) {
        self.bool(v.is_some());
        self.u64(v.unwrap_or(0));
    }

    /// Bytes as they are; the reader must know how many.
    #[inline(never)]
    pub fn bytes(&mut self, v: &[u8]) {
        self.buf.extend_from_slice(v);
    }

    /// Bytes preceded by their count.
    #[inline(never)]
    pub fn blob(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.bytes(v);
    }

    /// Bytes compressed (`lz`): their count, then the stream as a blob.
    #[inline(never)]
    pub fn packed(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.blob(&lz::compress(v));
    }
}

/// Deserializes one section. Every read returns `None` when the section is
/// too short or a value is impossible, which the caller turns into
/// [`SnapshotError::Damaged`].
pub struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data }
    }

    #[inline(never)]
    pub fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.data.len() < n {
            return None;
        }
        let (head, rest) = self.data.split_at(n);
        self.data = rest;
        Some(head)
    }

    #[inline(never)]
    pub fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.bytes(N).map(|b| b.try_into().unwrap())
    }

    #[inline(never)]
    pub fn u8(&mut self) -> Option<u8> {
        self.array::<1>().map(|[b]| b)
    }

    /// A byte that must be 0 or 1.
    #[inline(never)]
    pub fn bool(&mut self) -> Option<bool> {
        match self.u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    #[inline(never)]
    pub fn u16(&mut self) -> Option<u16> {
        self.array().map(u16::from_le_bytes)
    }

    #[inline(never)]
    pub fn u32(&mut self) -> Option<u32> {
        self.array().map(u32::from_le_bytes)
    }

    #[inline(never)]
    pub fn i32(&mut self) -> Option<i32> {
        self.array().map(i32::from_le_bytes)
    }

    #[inline(never)]
    pub fn u64(&mut self) -> Option<u64> {
        self.array().map(u64::from_le_bytes)
    }

    #[inline(never)]
    pub fn opt_u8(&mut self) -> Option<Option<u8>> {
        let some = self.bool()?;
        let v = self.u8()?;
        Some(some.then_some(v))
    }

    #[inline(never)]
    pub fn opt_u16(&mut self) -> Option<Option<u16>> {
        let some = self.bool()?;
        let v = self.u16()?;
        Some(some.then_some(v))
    }

    #[inline(never)]
    pub fn opt_u64(&mut self) -> Option<Option<u64>> {
        let some = self.bool()?;
        let v = self.u64()?;
        Some(some.then_some(v))
    }

    #[inline(never)]
    pub fn blob(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        self.bytes(n)
    }

    /// Bytes from [`Writer::packed`], at most `max` of them; the count is
    /// checked before anything that large is made.
    #[inline(never)]
    pub fn packed(&mut self, max: usize) -> Option<Vec<u8>> {
        let n = self.u32()? as usize;
        if n > max {
            return None;
        }
        lz::decompress(self.blob()?, n)
    }

    /// Checks that the whole section was read.
    #[inline(never)]
    pub fn end(self) -> Option<()> {
        self.data.is_empty().then_some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_png() {
        // The CRC of an empty IEND chunk, as in every PNG file.
        assert_eq!(crc32(&[b"IEND"]), 0xAE42_6082);
        assert_eq!(crc32(&[b"123456789"]), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn sections_round_trip() {
        let png = frame_png(&Frame::new_boxed());
        let mut w = SnapshotWriter::new(&png).unwrap();
        w.section(*b"SNAP", &VERSION.to_le_bytes());
        w.section(*b"meta", b"hello");
        let file = w.finish();
        assert_eq!(file[..png.len() - 12], png[..png.len() - 12]);
        let s = Sections::parse(&file).unwrap();
        assert_eq!(s.get(b"meta"), Some(&b"hello"[..]));
        assert_eq!(s.get(b"none"), None);
        assert_eq!(section(&file, b"meta"), Ok(Some(&b"hello"[..])));

        // The thumbnail alone is not a snapshot; a snapshot is not a thumbnail.
        assert_eq!(Sections::parse(&png).err(), Some(SnapshotError::NotSnapshot));
        assert!(SnapshotWriter::new(&file).is_none());
        assert!(SnapshotWriter::new(b"not a png").is_none());
    }

    #[test]
    fn damage_is_caught() {
        let png = frame_png(&Frame::new_boxed());
        let mut w = SnapshotWriter::new(&png).unwrap();
        w.section(*b"SNAP", &VERSION.to_le_bytes());
        w.section(*b"DATA", &[7; 100]);
        let file = w.finish();
        for len in 0..file.len() {
            assert!(Sections::parse(&file[..len]).is_err(), "truncated to {len}");
        }
        for at in [8, png.len(), file.len() - 50, file.len() - 1] {
            let mut bad = file.clone();
            bad[at] ^= 0x10;
            assert!(Sections::parse(&bad).is_err(), "bit flipped at {at}");
        }
        let mut w = SnapshotWriter::new(&png).unwrap();
        w.section(*b"SNAP", &(VERSION + 1).to_le_bytes());
        assert_eq!(Sections::parse(&w.finish()).err(), Some(SnapshotError::Version));
        let mut w = SnapshotWriter::new(&png).unwrap();
        w.section(*b"SNAP", &(VERSION - 1).to_le_bytes());
        assert_eq!(Sections::parse(&w.finish()).err(), Some(SnapshotError::Old));
        let mut w = SnapshotWriter::new(&png).unwrap();
        w.section(*b"SNAP", &VERSION.to_le_bytes());
        w.section(*b"DATA", &[]);
        w.section(*b"DATA", &[]);
        assert_eq!(Sections::parse(&w.finish()).err(), Some(SnapshotError::Damaged));
    }

    #[test]
    fn frame_png_has_the_frame_size() {
        let mut f = Frame::new_boxed();
        f.data[47 * crate::video::RASTER_BYTES + 16] = 0x80;
        let png = frame_png(&f);
        let chunks = png_chunks(&png).unwrap();
        assert_eq!(chunks[0].0, *b"IHDR");
        assert_eq!(chunks[0].1[..8], [0, 0, 1, 64, 0, 0, 0, 241]);
    }

    #[test]
    fn reader_checks_bounds_and_values() {
        let mut w = Writer::new();
        w.u16(0x1234);
        w.opt_u64(Some(5));
        w.blob(b"abc");
        w.packed(&[9; 1000]);
        w.u8(2);
        let bytes = w.into_bytes();
        let mut r = Reader::new(&bytes);
        assert_eq!(r.u16(), Some(0x1234));
        assert_eq!(r.opt_u64(), Some(Some(5)));
        assert_eq!(r.blob(), Some(&b"abc"[..]));
        let mut over = Reader::new(r.data);
        assert_eq!(over.packed(999), None, "over the limit");
        assert_eq!(r.packed(1000), Some(vec![9; 1000]));
        assert_eq!(r.bool(), None, "2 is not a bool");
        assert!(r.end().is_some());
        assert_eq!(Reader::new(&[1, 2]).u32(), None);
    }
}

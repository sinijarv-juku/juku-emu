//! Snapshot files (spec §4.6): the whole machine state in a file of its own.
//!
//! A snapshot is a signature and a run of sections, each a 4-byte tag, its
//! length, its bytes and a CRC-32, ending with an empty `END ` section, so
//! a damaged or truncated file is caught before anything is decoded. `SNAP`
//! comes first with the format version, then `THMB`, a PNG of the screen
//! for Finder's and Explorer's thumbnails.
//!
//! Section tags in capitals belong to the core. Tags that start with a
//! lowercase letter belong to the host, which the core stores and returns
//! without looking inside. All numbers are little-endian, so a file is the
//! same on every platform.

use crate::lz;
use crate::video::Frame;

/// Format version written into the `SNAP` section, the only one read.
/// Version 5 is a file of its own. Version 4 had the same sections in a
/// PNG, in private chunks after the picture; version 3 compressed the disks
/// only, version 2 nothing, and version 1 also had the active picture
/// (`FRAM`) in place of the beam's raster (`RAST`). Only version 4 was
/// released.
pub const VERSION: u32 = 5;

/// The first bytes of a snapshot file. As in PNG's signature, the byte
/// with its top bit set, CR LF, Ctrl-Z and LF catch a file mangled by a
/// transfer as text.
pub const SIGNATURE: [u8; 9] = [0x8A, b'J', b'U', b'K', b'U', 0x0D, 0x0A, 0x1A, 0x0A];
/// The section that ends the file, with no data.
const END: [u8; 4] = *b"END ";
/// The section holding the thumbnail, a PNG.
pub const THUMBNAIL: [u8; 4] = *b"THMB";

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const IEND: [u8; 4] = *b"IEND";
/// The chunk type of a version 4 file's sections.
const OLD_CHUNK_TYPE: [u8; 4] = *b"juKu";
/// PNG chunk lengths are limited to 2³¹ − 1.
const MAX_CHUNK: usize = 0x7FFF_FFFF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// Not a snapshot file.
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

/// A PNG chunk: big-endian length, type, data, CRC.
fn put_chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(&[&kind, data]).to_be_bytes());
}

/// A PNG chunk, or a section: its 4-byte type or tag, and its data.
type Chunk<'a> = ([u8; 4], &'a [u8]);

/// Walks the chunks of a PNG file, checking each CRC and that the file ends
/// with IEND. Returns the chunks as (type, data), or `None` if `file` is not
/// a whole, valid PNG.
fn png_chunks(file: &[u8]) -> Option<Vec<Chunk<'_>>> {
    let mut rest = file.strip_prefix(&PNG_SIGNATURE)?;
    let mut chunks = Vec::new();
    loop {
        if rest.len() < 12 {
            return None;
        }
        let len = u32::from_be_bytes(rest[..4].try_into().unwrap()) as usize;
        if len > MAX_CHUNK || rest.len() - 12 < len {
            return None;
        }
        let kind: [u8; 4] = rest[4..8].try_into().unwrap();
        let data = &rest[8..8 + len];
        let crc = u32::from_be_bytes(rest[8 + len..12 + len].try_into().unwrap());
        if crc != crc32(&[&kind, data]) {
            return None;
        }
        chunks.push((kind, data));
        rest = &rest[12 + len..];
        if kind == IEND {
            return rest.is_empty().then_some(chunks);
        }
    }
}

/// True if `file` is a whole PNG file whose chunks all check out.
pub fn is_png(file: &[u8]) -> bool {
    png_chunks(file).is_some()
}

/// What a file without the signature is: a version 4 snapshot, which is a
/// PNG with the sections in `juKu` chunks, is old; anything else is not a
/// snapshot.
fn unsigned(file: &[u8]) -> SnapshotError {
    match png_chunks(file) {
        Some(chunks) if chunks.iter().any(|(kind, _)| *kind == OLD_CHUNK_TYPE) => SnapshotError::Old,
        _ => SnapshotError::NotSnapshot,
    }
}

/// Walks the sections of a snapshot file, checking each CRC and that the
/// file ends with the `END ` section. Returns the sections before it as
/// (tag, data).
fn file_sections(file: &[u8]) -> Result<Vec<Chunk<'_>>, SnapshotError> {
    let Some(mut rest) = file.strip_prefix(&SIGNATURE) else {
        return Err(unsigned(file));
    };
    let mut list = Vec::new();
    loop {
        if rest.len() < 12 {
            return Err(SnapshotError::Damaged);
        }
        let tag: [u8; 4] = rest[..4].try_into().unwrap();
        let len = u32::from_le_bytes(rest[4..8].try_into().unwrap()) as usize;
        if rest.len() - 12 < len {
            return Err(SnapshotError::Damaged);
        }
        let data = &rest[8..8 + len];
        let crc = u32::from_le_bytes(rest[8 + len..12 + len].try_into().unwrap());
        if crc != crc32(&[&tag, data]) {
            return Err(SnapshotError::Damaged);
        }
        rest = &rest[12 + len..];
        if tag == END {
            return if data.is_empty() && rest.is_empty() { Ok(list) } else { Err(SnapshotError::Damaged) };
        }
        list.push((tag, data));
    }
}

/// Builds a snapshot file: the signature, the sections in the order given,
/// and the `END ` section.
pub struct SnapshotWriter {
    out: Vec<u8>,
}

impl Default for SnapshotWriter {
    fn default() -> SnapshotWriter {
        SnapshotWriter::new()
    }
}

impl SnapshotWriter {
    pub fn new() -> SnapshotWriter {
        SnapshotWriter { out: SIGNATURE.to_vec() }
    }

    /// Adds a section: tag, little-endian length, data, and the CRC-32 of
    /// the tag and data.
    pub fn section(&mut self, tag: [u8; 4], data: &[u8]) {
        self.out.extend_from_slice(&tag);
        self.out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.out.extend_from_slice(data);
        self.out.extend_from_slice(&crc32(&[&tag, data]).to_le_bytes());
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.section(END, &[]);
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
        let list = file_sections(file)?;
        for (i, (tag, _)) in list.iter().enumerate() {
            if list[..i].iter().any(|(t, _)| t == tag) {
                return Err(SnapshotError::Damaged);
            }
        }
        let Some((tag, head)) = list.first() else {
            return Err(SnapshotError::Damaged);
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

/// The thumbnail of a snapshot file, a PNG, whatever the file's version, or
/// `None` if the file is not a whole snapshot or has no thumbnail.
pub fn thumbnail(file: &[u8]) -> Option<&[u8]> {
    file_sections(file).ok()?.into_iter().find(|(tag, _)| *tag == THUMBNAIL).map(|(_, data)| data)
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
    put_chunk(&mut out, *b"IHDR", &ihdr);
    put_chunk(&mut out, *b"IDAT", &z);
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
        let mut w = SnapshotWriter::new();
        w.section(*b"SNAP", &VERSION.to_le_bytes());
        w.section(THUMBNAIL, &png);
        w.section(*b"meta", b"hello");
        let file = w.finish();
        assert_eq!(file[..SIGNATURE.len()], SIGNATURE);
        assert_eq!(file[SIGNATURE.len()..][..12], [b'S', b'N', b'A', b'P', 4, 0, 0, 0, 5, 0, 0, 0]);
        assert_eq!(file[file.len() - 12..file.len() - 4], [b'E', b'N', b'D', b' ', 0, 0, 0, 0]);
        let s = Sections::parse(&file).unwrap();
        assert_eq!(s.get(b"meta"), Some(&b"hello"[..]));
        assert_eq!(s.get(b"none"), None);
        assert_eq!(section(&file, b"meta"), Ok(Some(&b"hello"[..])));
        assert_eq!(thumbnail(&file), Some(&png[..]));
        assert!(is_png(&png) && !is_png(&file) && !is_png(b"not a png"));

        // The thumbnail alone is not a snapshot.
        assert_eq!(Sections::parse(&png).err(), Some(SnapshotError::NotSnapshot));
        assert_eq!(thumbnail(&png), None);
    }

    /// Version 4 was a PNG with the sections in `juKu` chunks before IEND.
    #[test]
    fn png_snapshots_are_old() {
        let png = frame_png(&Frame::new_boxed());
        let mut old = png[..png.len() - 12].to_vec();
        put_chunk(&mut old, OLD_CHUNK_TYPE, &[b"SNAP".as_slice(), &4u32.to_le_bytes()].concat());
        put_chunk(&mut old, IEND, &[]);
        assert!(is_png(&old));
        assert_eq!(Sections::parse(&old).err(), Some(SnapshotError::Old));
        assert_eq!(Sections::parse(&png).err(), Some(SnapshotError::NotSnapshot));
    }

    #[test]
    fn damage_is_caught() {
        let mut w = SnapshotWriter::new();
        w.section(*b"SNAP", &VERSION.to_le_bytes());
        w.section(*b"DATA", &[7; 100]);
        let file = w.finish();
        for len in 0..file.len() {
            let error = if len < SIGNATURE.len() { SnapshotError::NotSnapshot } else { SnapshotError::Damaged };
            assert_eq!(Sections::parse(&file[..len]).err(), Some(error), "truncated to {len}");
        }
        for at in 0..file.len() {
            let mut bad = file.clone();
            bad[at] ^= 0x10;
            assert!(Sections::parse(&bad).is_err(), "bit flipped at {at}");
        }
        assert!(Sections::parse(&[&file[..], &[0]].concat()).is_err(), "a byte after END");
        let mut w = SnapshotWriter::new();
        w.section(*b"SNAP", &(VERSION + 1).to_le_bytes());
        assert_eq!(Sections::parse(&w.finish()).err(), Some(SnapshotError::Version));
        let mut w = SnapshotWriter::new();
        w.section(*b"SNAP", &(VERSION - 1).to_le_bytes());
        assert_eq!(Sections::parse(&w.finish()).err(), Some(SnapshotError::Old));
        let mut w = SnapshotWriter::new();
        w.section(*b"SNAP", &VERSION.to_le_bytes());
        w.section(*b"DATA", &[]);
        w.section(*b"DATA", &[]);
        assert_eq!(Sections::parse(&w.finish()).err(), Some(SnapshotError::Damaged));
        let mut w = SnapshotWriter::new();
        w.section(*b"DATA", &[]);
        assert_eq!(Sections::parse(&w.finish()).err(), Some(SnapshotError::Damaged), "no SNAP first");
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

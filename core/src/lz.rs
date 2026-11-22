//! LZ77 for the disk images in snapshots (spec §4.6), of the core's own,
//! since the core takes no crates.
//!
//! The stream is a run of steps: a literal count, that many literal bytes,
//! and a match length, followed by its distance back unless the length is
//! 0. Every number is an unsigned LEB128 varint. A distance shorter than the
//! length repeats the bytes it copies, so a run of one byte is one step.

/// The shortest match the compressor takes.
const MIN_MATCH: usize = 4;
/// How far back a match may start.
const WINDOW: usize = 1 << 16;
/// The hash table has 2^HASH_BITS entries, the last position of each hash.
const HASH_BITS: u32 = 15;

fn put_varint(out: &mut Vec<u8>, mut v: usize) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// Reads a varint of at most 32 bits.
fn varint(r: &mut &[u8]) -> Option<usize> {
    let mut v = 0u64;
    for shift in (0..35).step_by(7) {
        let (&b, rest) = r.split_first()?;
        *r = rest;
        v |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            return u32::try_from(v).ok().map(|v| v as usize);
        }
    }
    None
}

/// Hash of the 4 bytes at the start of `b`.
fn hash(b: &[u8]) -> usize {
    let v = u32::from_le_bytes(b[..4].try_into().unwrap());
    (v.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize
}

/// Compresses `data` greedily: at each position, the match through the last
/// position with the same hash, if it is long enough.
pub fn compress(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut table = vec![u32::MAX; 1 << HASH_BITS];
    // Where the literals not yet written start.
    let mut literals = 0;
    let mut i = 0;
    while i + MIN_MATCH <= data.len() {
        let h = hash(&data[i..]);
        let candidate = table[h] as usize;
        table[h] = i as u32;
        if candidate < i && i - candidate <= WINDOW {
            let len = data[candidate..].iter().zip(&data[i..]).take_while(|(a, b)| a == b).count();
            if len >= MIN_MATCH {
                put_varint(&mut out, i - literals);
                out.extend_from_slice(&data[literals..i]);
                put_varint(&mut out, len);
                put_varint(&mut out, i - candidate);
                for j in i + 1..(i + len).min(data.len() + 1 - MIN_MATCH) {
                    table[hash(&data[j..])] = j as u32;
                }
                i += len;
                literals = i;
                continue;
            }
        }
        i += 1;
    }
    if literals < data.len() {
        put_varint(&mut out, data.len() - literals);
        out.extend_from_slice(&data[literals..]);
        put_varint(&mut out, 0);
    }
    out
}

/// Decompresses a stream that must make exactly `size` bytes. `None` for a
/// stream that would write past the end, reach back before the start, end
/// short or leave bytes over, or holds a varint over 32 bits.
pub fn decompress(stream: &[u8], size: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(size);
    let mut r = stream;
    while out.len() < size {
        let n = varint(&mut r)?;
        if n > size - out.len() || n > r.len() {
            return None;
        }
        let (bytes, rest) = r.split_at(n);
        out.extend_from_slice(bytes);
        r = rest;
        let len = varint(&mut r)?;
        if len == 0 {
            continue;
        }
        let dist = varint(&mut r)?;
        if dist == 0 || dist > out.len() || len > size - out.len() {
            return None;
        }
        let start = out.len() - dist;
        if dist >= len {
            out.extend_from_within(start..start + len);
        } else {
            for k in start..start + len {
                out.push(out[k]);
            }
        }
    }
    r.is_empty().then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes from xorshift, the same on every run.
    fn noise(len: usize) -> Vec<u8> {
        let mut x = 0x2545_F491u32;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    fn round_trip(data: &[u8]) -> usize {
        let packed = compress(data);
        assert_eq!(decompress(&packed, data.len()).as_deref(), Some(data));
        packed.len()
    }

    #[test]
    fn data_round_trips() {
        assert_eq!(round_trip(&[]), 0);
        round_trip(b"abc");
        round_trip(&noise(100_000));
        // A run is one step.
        assert!(round_trip(&[0; 819_200]) < 10);
        assert!(round_trip(&[0xE5; 409_600]) < 10);
        // Sectors that repeat further back than the window, and others
        // within it.
        let mut disk = vec![0xE5; 819_200];
        let sector = noise(512);
        for at in [0, 512, 70_000, 400_000, 819_200 - 512] {
            disk[at..at + 512].copy_from_slice(&sector);
        }
        disk[1000..1100].copy_from_slice(&noise(100));
        // The sector is stored 4 times, once for each copy more than the
        // window away from the last; the one at 512 is a match.
        let len = round_trip(&disk);
        assert!(len < 4 * 512 + 200, "{len} bytes");
        // Short repeats and noise mixed.
        let text: Vec<u8> = b"the quick brown fox ".iter().cycle().take(5000).copied().chain(noise(3000)).collect();
        round_trip(&text);
    }

    #[test]
    fn bad_streams_are_refused() {
        let data: Vec<u8> = noise(2000).into_iter().chain([7; 2000]).collect();
        let packed = compress(&data);
        assert_eq!(decompress(&packed, data.len() - 1), None, "past the end");
        assert_eq!(decompress(&packed, data.len() + 1), None, "short");
        assert_eq!(decompress(&[&packed[..], &[0]].concat(), data.len()), None, "bytes over");
        for len in 0..packed.len() {
            assert_eq!(decompress(&packed[..len], data.len()), None, "truncated to {len}");
        }
        // One literal, then a match reaching 2 back.
        assert_eq!(decompress(&[1, 9, 4, 2], 5), None, "before the start");
        assert_eq!(decompress(&[1, 9, 4, 0], 5), None, "distance 0");
        assert_eq!(decompress(&[1, 9, 4, 1], 5), Some(vec![9; 5]));
        assert_eq!(decompress(&[0x80, 0x80, 0x80, 0x80, 0x10], 5), None, "a varint over 32 bits");
        assert_eq!(decompress(&[0x80, 0x80, 0x80, 0x80, 0x80, 0], 5), None, "a long varint");
    }
}

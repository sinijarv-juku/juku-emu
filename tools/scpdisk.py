#!/usr/bin/env python3
"""Decode a SuperCard Pro (.scp) flux image of a Juku MFM disk.

    scpdisk.py IMAGE                     one line per track: IDs, good sectors, order
    scpdisk.py IMAGE -t CYL.SIDE [-r N]  the byte layout of one track (revolution N)
    scpdisk.py IMAGE -o OUT.juk          a raw sector image, as juku-disk reads

The flux is turned into MFM cells with a Greaseweazle-style PLL whose cell is
the revolution's time over 100 000 (250 kbit/s at 300 rpm), so dumps made at
360 rpm decode too. The decoder finds A1 sync runs (and C2 runs, the index
mark), checks the CRCs and takes each sector from the first revolution that
reads it cleanly. The track format of real disks is in MD §10.3
(docs/Juku_machine_description.md).

An image keeps the physical cylinder and side of each track: offset
((cyl * sides + side) * 10 + id - 1) * 512, sides = 2 unless the dump holds
one side only. Sectors that read on no revolution are left 00 and listed.
"""
import argparse
import struct
import sys
from concurrent.futures import ProcessPoolExecutor

SECTOR = 512
SECTORS = 10
CYLINDERS = 80
TICK = 25e-9  # SCP resolution 0
SYNC_A1 = '0100010010001001'  # A1 with the clock bit between bits 4 and 5 missing
SYNC_C2 = '0101001000100100'  # C2 with the clock bit between bits 3 and 4 missing
DATA_MARKS = (0xF8, 0xF9, 0xFA, 0xFB)


def crc16(data, crc=0xFFFF):
    for b in data:
        crc ^= b << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021 if crc & 0x8000 else crc << 1) & 0xFFFF
    return crc


def read_scp(path):
    """(heads, {track: [(index time in ticks, flux intervals in ticks), ...]})
    with track = cylinder * 2 + side."""
    d = open(path, 'rb').read()
    if d[:3] != b'SCP':
        raise ValueError('not an SCP image')
    revs, start, end, cell_bits, heads, res = d[5], d[6], d[7], d[9], d[10], d[11]
    if cell_bits not in (0, 16) or res != 0:
        raise ValueError(f'unsupported: {cell_bits}-bit cells, resolution {res}')
    offsets = struct.unpack_from('<168I', d, 16)
    tracks = {}
    for t in range(start, end + 1):
        o = offsets[t]
        if not o or d[o:o + 3] != b'TRK':
            continue
        revolutions = []
        for r in range(revs):
            index, n, at = struct.unpack_from('<III', d, o + 4 + 12 * r)
            flux, carry = [], 0
            for v in struct.unpack_from(f'>{n}H', d, o + at):
                if v == 0:
                    carry += 65536
                else:
                    flux.append(carry + v)
                    carry = 0
            revolutions.append((index, flux))
        tracks[t] = revolutions
    return heads, tracks


def to_cells(flux, cell):
    """MFM cells as a string of 0 and 1, with a PLL that follows the cell
    length within 10% and pulls the phase 60% of the way to each transition."""
    clock, low, high = cell, cell * 0.9, cell * 1.1
    out = []
    ticks = 0.0
    for f in flux:
        ticks += f
        if ticks < clock / 2:
            continue
        zeros = 0
        ticks -= clock
        while ticks >= clock / 2:
            zeros += 1
            ticks -= clock
        out.append('0' * zeros + '1')
        clock += ticks * 0.05 if zeros <= 3 else (cell - clock) * 0.05
        clock = min(max(clock, low), high)
        ticks *= 0.4
    return ''.join(out)


def byte_at(s, i):
    """The data byte in the 16 cells from i (clock, data, clock, data...)."""
    v = 0
    for k in range(i + 1, i + 16, 2):
        v = v << 1 | (s[k] == '1')
    return v


def read_bytes(s, i, n):
    n = min(n, (len(s) - i) // 16)
    return bytes(byte_at(s, i + 16 * k) for k in range(n))


def find_marks(s):
    """Sync runs as (kind, cell position, count, end). The search skips the
    contents of a data field, which can hold A1 sync patterns by chance."""
    marks = []
    i = 0
    while True:
        a = s.find(SYNC_A1, i)
        c = s.find(SYNC_C2 * 3, i)
        if a < 0 and c < 0:
            return marks
        j = a if c < 0 or 0 <= a < c else c
        pattern = SYNC_A1 if j == a else SYNC_C2
        end, n = j, 0
        while s.startswith(pattern, end):
            end += 16
            n += 1
        marks.append(('A1' if pattern == SYNC_A1 else 'C2', j, n, end))
        i = end
        if pattern == SYNC_A1 and end + 16 <= len(s) and byte_at(s, end) in DATA_MARKS:
            i = end + 16 * (1 + SECTOR + 2)


def decode(index, flux):
    """The track's cells, its marks, and its sectors as
    {(c, h, r): data}, IDs as [(position in bytes, c, h, r, n, crc ok)]."""
    s = to_cells(flux, index / 100_000)
    marks = find_marks(s)
    ids, sectors = [], {}
    last_id = None
    for kind, pos, _, end in marks:
        if kind != 'A1':
            continue
        b = read_bytes(s, end, 7)
        if len(b) == 7 and b[0] == 0xFE:
            ok = crc16(b'\xA1\xA1\xA1' + b[:5]) == b[5] << 8 | b[6]
            ids.append((pos // 16, b[1], b[2], b[3], b[4], ok))
            last_id = (pos, tuple(b[1:4])) if ok else None
        elif b and b[0] in DATA_MARKS:
            # A data field belongs to the ID just before it, within gap 2.
            if last_id and pos - last_id[0] < 16 * 64:
                field = read_bytes(s, end, 1 + SECTOR + 2)
                if len(field) == 1 + SECTOR + 2 and \
                        crc16(b'\xA1\xA1\xA1' + field[:-2]) == field[-2] << 8 | field[-1]:
                    sectors.setdefault(last_id[1], field[1:-2])
            last_id = None
    return s, marks, ids, sectors


def decode_job(job):
    t, index, flux = job
    _, _, ids, sectors = decode(index, flux)
    return t, ids, sectors


def rle(data):
    runs = []
    for b in data:
        if runs and runs[-1][0] == b:
            runs[-1][1] += 1
        else:
            runs.append([b, 1])
    return ' '.join(f'{n}x{b:02X}' if n > 1 else f'{b:02X}' for b, n in runs)


def layout(index, flux):
    """The track as lines of byte runs, starting again at each sync run so a
    write splice does not misalign what follows. A trailing (+n) counts the
    cells left over before the next sync."""
    s, marks, _, _ = decode(index, flux)
    if not marks:
        return ['no sync marks']
    first = marks[0][1]
    lines = ['index  ' + rle(read_bytes(s, first % 16, first // 16))]
    for k, (kind, pos, n, end) in enumerate(marks):
        nxt = marks[k + 1][1] if k + 1 < len(marks) else len(s)
        body = read_bytes(s, end, (nxt - end) // 16)
        if body and body[0] in DATA_MARKS and len(body) >= 1 + SECTOR + 2:
            text = f'{body[0]:02X} [{SECTOR} bytes] CRC {body[SECTOR + 1]:02X}{body[SECTOR + 2]:02X} ' \
                + rle(body[1 + SECTOR + 2:])
        else:
            text = rle(body)
        extra = (nxt - end) % 16
        lines.append(f'{n}x{kind}  {text}' + (f' (+{extra})' if extra else ''))
    return lines


def main():
    p = argparse.ArgumentParser(description='Decode a SuperCard Pro flux image of a Juku MFM disk.')
    p.add_argument('image')
    p.add_argument('-t', '--track', help='print the byte layout of CYL.SIDE, for example 20.1')
    p.add_argument('-r', '--rev', type=int, default=0, help='revolution for -t (default 0)')
    p.add_argument('-o', '--out', help='write a raw sector image')
    a = p.parse_args()

    try:
        heads, tracks = read_scp(a.image)
    except (OSError, ValueError, struct.error) as err:
        sys.exit(f'{a.image}: {err}')

    if a.track:
        cyl, _, side = a.track.partition('.')
        t = int(cyl) * 2 + int(side or 0)
        if t not in tracks or not 0 <= a.rev < len(tracks[t]):
            sys.exit(f'{a.image}: no track {a.track}, revolution {a.rev}')
        index, flux = tracks[t][a.rev]
        print(f'cylinder {t // 2} side {t % 2}, revolution {a.rev}: '
              f'{60 / (index * TICK):.1f} rpm, {len(to_cells(flux, index / 100_000)) / 16:.0f} bytes')
        for line in layout(index, flux):
            print(line)
        return

    jobs = [(t, index, flux) for t, revs in sorted(tracks.items()) for index, flux in revs]
    with ProcessPoolExecutor() as pool:
        results = list(pool.map(decode_job, jobs, chunksize=4))

    found = {}
    for t, ids, sectors in results:
        per = found.setdefault(t, {'ids': set(), 'bad': 0, 'orders': set(), 'sectors': {}, 'other': set()})
        good = [i for i in ids if i[5]]
        per['ids'].update(i[3] for i in good)
        per['bad'] += len(ids) - len(good)
        per['orders'].add(tuple(i[3] for i in good))
        per['other'].update((c, h, n) for _, c, h, _, n, _ in good if (c, h, n) != (t // 2, t % 2, 2))
        for (c, h, r), data in sectors.items():
            if (c, h) == (t // 2, t % 2):
                per['sectors'].setdefault(r, data)

    if not a.out:
        print(f'{a.image}: {len(tracks)} tracks, {len(next(iter(tracks.values())))} revolutions')
        for t, per in sorted(found.items()):
            longest = max(per['orders'], key=len)
            note = f'  ID fields (C, H, N) {sorted(per["other"])}' if per['other'] else ''
            print(f'{t // 2:2}.{t % 2}  IDs {len(per["ids"]):2}  good {len(per["sectors"]):2}'
                  f'  bad ID CRCs {per["bad"]:2}  order {" ".join(map(str, longest)) or "-"}{note}')
        return

    sides = 1 if heads in (1, 2) else 2
    image = bytearray(CYLINDERS * sides * SECTORS * SECTOR)
    missing = []
    for cyl in range(CYLINDERS):
        for side in range(sides):
            t = cyl * 2 + (side if sides == 2 else heads - 1)
            sectors = found.get(t, {}).get('sectors', {})
            for r in range(1, SECTORS + 1):
                if r in sectors:
                    o = ((cyl * sides + side) * SECTORS + r - 1) * SECTOR
                    image[o:o + SECTOR] = sectors[r]
                else:
                    missing.append(f'{cyl}.{side}:{r}')
    open(a.out, 'wb').write(image)
    print(f'{a.out}: {len(image)} bytes, {CYLINDERS * sides * SECTORS - len(missing)} sectors read')
    if missing:
        print(f'not read ({len(missing)}): ' + ' '.join(missing), file=sys.stderr)


if __name__ == '__main__':
    main()

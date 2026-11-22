//! C ABI used by the host application.
//!
//! Keep in sync with the `extern` declarations in `src/Core.m`.
//! All functions must be called from the emulation thread, except
//! `juku_audio_pull` and `juku_audio_fill`, which the audio thread may call.

use core::ptr;
use std::sync::Arc;

use crate::audio::AudioRing;
use crate::floppy::Disk;
use crate::machine::Machine;
use crate::snapshot::{self, SnapshotError};
use crate::video::Frame;

/// Creates the audio output ring for `sample_rate` Hz. It is shared between
/// the machine (producer) and the host's audio thread (consumer).
#[unsafe(no_mangle)]
pub extern "C" fn juku_audio_new(sample_rate: u32) -> *const AudioRing {
    // Latency after an underrun: about 43 ms.
    let prime = sample_rate as usize * 2048 / 48_000;
    Arc::into_raw(Arc::new(AudioRing::new(sample_rate, prime)))
}

/// Releases the host's reference to the ring. The machine keeps its own.
///
/// # Safety
/// `a` must come from [`juku_audio_new`] and not be used afterwards (or be NULL).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_audio_free(a: *const AudioRing) {
    if !a.is_null() {
        drop(unsafe { Arc::from_raw(a) });
    }
}

/// Audio thread: fills `left` and `right` with `frames` samples each,
/// padding with silence on underrun. Returns the number of real frames
/// delivered.
///
/// # Safety
/// `a` must be a valid ring pointer, and `left` and `right` must each point
/// to `frames` floats, in buffers that do not overlap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_audio_pull(a: *const AudioRing, left: *mut f32, right: *mut f32, frames: usize) -> usize {
    let left = unsafe { core::slice::from_raw_parts_mut(left, frames) };
    let right = unsafe { core::slice::from_raw_parts_mut(right, frames) };
    unsafe { &*a }.pull(left, right)
}

/// Samples queued in the ring, for rate control.
///
/// # Safety
/// `a` must be a valid ring pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_audio_fill(a: *const AudioRing) -> usize {
    unsafe { &*a }.fill()
}

/// Mutes the machine's audio (for warp speed): samples are no longer queued,
/// so the ring drains and the output is silent until unmuted.
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_audio_mute(m: *mut Machine, muted: bool) {
    unsafe { &mut *m }.set_audio_muted(muted);
}

/// Creates a machine from the 16 KB system ROM, with its speaker feeding
/// `audio` (or silent if NULL). Returns NULL if the ROM is missing or has the
/// wrong size.
///
/// # Safety
/// `rom` must point to `rom_len` readable bytes (or be NULL). `audio` must be
/// NULL or a valid ring pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_new(rom: *const u8, rom_len: usize, audio: *const AudioRing) -> *mut Machine {
    if rom.is_null() {
        return ptr::null_mut();
    }
    let rom = unsafe { core::slice::from_raw_parts(rom, rom_len) };
    let audio = (!audio.is_null()).then(|| {
        // Take a reference of our own, leaving the host's intact.
        unsafe { Arc::increment_strong_count(audio) };
        unsafe { Arc::from_raw(audio) }
    });
    match Machine::with_audio(rom, audio) {
        Some(m) => Box::into_raw(Box::new(m)),
        None => ptr::null_mut(),
    }
}

/// # Safety
/// `m` must come from [`juku_new`] and not be used afterwards (or be NULL).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_free(m: *mut Machine) {
    if !m.is_null() {
        drop(unsafe { Box::from_raw(m) });
    }
}

/// Checks that an image is an expansion ROM the monitor can start
/// (`expansion::check`). Returns 0, −1 (empty or over 32 KB) or −2 (no
/// program header).
///
/// # Safety
/// `data` must point to `len` readable bytes (or be NULL).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_expansion_check(data: *const u8, len: usize) -> i32 {
    if data.is_null() {
        return -1;
    }
    expansion_result(crate::expansion::check(unsafe { core::slice::from_raw_parts(data, len) }))
}

/// Plugs in an expansion ROM and cold-starts the machine. Returns as
/// [`juku_expansion_check`]; on error nothing changes.
///
/// # Safety
/// `m` must be a valid machine pointer; `data` must point to `len` readable
/// bytes (or be NULL).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_expansion_insert(m: *mut Machine, data: *const u8, len: usize) -> i32 {
    if data.is_null() {
        return -1;
    }
    let image = unsafe { core::slice::from_raw_parts(data, len) };
    expansion_result(unsafe { &mut *m }.insert_expansion(image))
}

/// Unplugs the expansion ROM and cold-starts the machine.
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_expansion_eject(m: *mut Machine) {
    unsafe { &mut *m }.eject_expansion();
}

fn expansion_result(r: Result<(), crate::expansion::ExpansionError>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(crate::expansion::ExpansionError::BadSize) => -1,
        Err(crate::expansion::ExpansionError::NoHeader) => -2,
    }
}

/// Replaces the 16 KB system ROM and cold-starts the machine. Returns false,
/// changing nothing, if the ROM is missing or has the wrong size. A key click
/// patch is lost with the old ROM; apply it again.
///
/// # Safety
/// `m` must be a valid machine pointer; `rom` must point to `rom_len`
/// readable bytes (or be NULL).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_load_rom(m: *mut Machine, rom: *const u8, rom_len: usize) -> bool {
    if rom.is_null() {
        return false;
    }
    let rom = unsafe { core::slice::from_raw_parts(rom, rom_len) };
    unsafe { &mut *m }.load_rom(rom)
}

/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_reset(m: *mut Machine, cold: bool) {
    unsafe { &mut *m }.reset(cold);
}

/// Runs for `cycles` CPU cycles (2 MHz). Returns the cycles executed.
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_run_cycles(m: *mut Machine, cycles: u64) -> u64 {
    unsafe { &mut *m }.run_cycles(cycles)
}

/// Emulated time in CPU cycles since power-on.
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_cycles(m: *const Machine) -> u64 {
    unsafe { &*m }.cycles()
}

/// The last completed frame. The pointer stays valid until the next call to
/// [`juku_run_cycles`] or [`juku_free`].
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_frame(m: *const Machine) -> *const Frame {
    unsafe { &*m }.frame()
}

/// Presses or releases a Juku key: `column << 3 | row` (MD §6.3), or 0x80 =
/// SHIFT, 0x81 = CTRL. Unknown IDs are ignored.
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_key(m: *mut Machine, key: u8, down: bool) {
    unsafe { &mut *m }.set_key(key, down);
}

/// Makes the guest Caps Lock follow `on`, the host's Caps Lock state. The
/// core presses CAPS while the RomBios state differs; calling this often
/// with an unchanged state costs nothing.
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_caps_lock(m: *mut Machine, on: bool) {
    unsafe { &mut *m }.set_caps_lock(on);
}

/// Turns the RomBios key click on or off by patching the ROM. Takes effect
/// at the next key press and survives resets. Returns false if the ROM has
/// no known click code.
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_set_key_click(m: *mut Machine, on: bool) -> bool {
    unsafe { &mut *m }.set_key_click(on)
}

/// Clocks PIT #3 counter 2 (T3.2) at 1.2308 MHz, as on the 1987 schematic,
/// or, by default, once a frame (MD §10.2). Takes effect at once and
/// survives resets and snapshot loads.
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_set_t32_fast_clock(m: *mut Machine, on: bool) {
    unsafe { &mut *m }.set_t32_fast_clock(on);
}

/// Turns the floppy drives' sounds on or off: the motor, the head stepping
/// and the disks going in and out. Off in a new machine.
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_set_drive_sounds(m: *mut Machine, on: bool) {
    unsafe { &mut *m }.set_drive_sounds(on);
}

/// Sets the drive sounds' volume, from 0 to 2; 1 by default.
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_set_drive_sounds_volume(m: *mut Machine, volume: f32) {
    unsafe { &mut *m }.set_drive_sounds_volume(volume);
}

/// With `on`, the drive motors fade out once the controller has had no
/// command for 10 s. Off in a new machine.
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_set_drive_idle_mute(m: *mut Machine, on: bool) {
    unsafe { &mut *m }.set_drive_idle_mute(on);
}

/// Releases every key, e.g. when the window loses focus.
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_keys_release_all(m: *mut Machine) {
    unsafe { &mut *m }.release_all_keys();
}

/// Sets a keyboard DIP switch bank: `bank` 4 or 5 (the port B bit it is read
/// on); bit n = switch on column 8 + n is ON.
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_set_dip(m: *mut Machine, bank: u8, value: u8) {
    unsafe { &mut *m }.set_dip(bank, value);
}

/// Checks that an image `len` bytes long is one the drives take: 0, or −1
/// if its size is not supported.
#[unsafe(no_mangle)]
pub extern "C" fn juku_disk_check(len: usize) -> i32 {
    if Disk::size_ok(len) { 0 } else { -1 }
}

/// Inserts a disk image (copied) into drive 0 (A:) or 1 (B:). Returns 0 on
/// success, −1 if the image size is not supported, −2 for a bad drive.
///
/// # Safety
/// `m` must be a valid machine pointer and `data` must point to `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_disk_insert(m: *mut Machine, drive: u8, data: *const u8, len: usize, write_protect: bool) -> i32 {
    if drive > 1 {
        return -2;
    }
    let image = unsafe { core::slice::from_raw_parts(data, len) }.to_vec();
    match unsafe { &mut *m }.insert_disk(drive as usize, image, write_protect) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// Inserts a new disk, formatted and empty, into drive 0 (A:) or 1 (B:).
/// Returns 0, or −2 for a bad drive.
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_disk_insert_blank(m: *mut Machine, drive: u8) -> i32 {
    if drive > 1 {
        return -2;
    }
    unsafe { &mut *m }.insert_blank_disk(drive as usize);
    0
}

/// Removes the disk from a drive. Unsaved changes are lost: write them back
/// first.
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_disk_eject(m: *mut Machine, drive: u8) {
    unsafe { &mut *m }.eject_disk(drive as usize);
}

/// True if the disk in `drive` was written since it was inserted, restored
/// from a snapshot or marked clean after a save.
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_disk_dirty(m: *const Machine, drive: u8) -> bool {
    unsafe { &*m }.disk(drive as usize).is_some_and(|d| d.dirty())
}

/// The current image data of the disk in `drive`, or NULL if the drive is
/// empty. Valid until the next call that runs or changes the machine.
///
/// # Safety
/// `m` must be a valid machine pointer and `len` a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_disk_data(m: *const Machine, drive: u8, len: *mut usize) -> *const u8 {
    match unsafe { &*m }.disk(drive as usize) {
        Some(d) => {
            unsafe { *len = d.data().len() };
            d.data().as_ptr()
        }
        None => {
            unsafe { *len = 0 };
            ptr::null()
        }
    }
}

/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_disk_clear_dirty(m: *mut Machine, drive: u8) {
    if let Some(d) = unsafe { &mut *m }.disk_mut(drive as usize) {
        d.clear_dirty();
    }
}

/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_disk_set_write_protect(m: *mut Machine, drive: u8, on: bool) {
    if let Some(d) = unsafe { &mut *m }.disk_mut(drive as usize) {
        d.set_write_protect(on);
    }
}

/// True if the disk in `drive` is write-protected (false if the drive is
/// empty). A restored snapshot brings its own setting.
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_disk_write_protected(m: *const Machine, drive: u8) -> bool {
    unsafe { &*m }.disk(drive as usize).is_some_and(|d| d.write_protect())
}

/// Copies the system ROM, as its file has it (the key click patch undone),
/// into `out`: 16 384 bytes. After a snapshot is restored, the host matches
/// it to its ROM files.
///
/// # Safety
/// `m` must be a valid machine pointer and `out` must point to 16 384
/// writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_system_rom(m: *const Machine, out: *mut u8) {
    let rom = unsafe { &*m }.system_rom();
    unsafe { core::slice::from_raw_parts_mut(out, rom.len()) }.copy_from_slice(&rom);
}

/// The expansion window up to its last byte that is not FFh, and its length
/// in `len`: 0 with no expansion ROM plugged in. Valid until the next call
/// that changes the machine.
///
/// # Safety
/// `m` must be a valid machine pointer and `len` a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_expansion_rom(m: *const Machine, len: *mut usize) -> *const u8 {
    let window = unsafe { &*m }.expansion_rom();
    unsafe { *len = window.len() };
    window.as_ptr()
}

/// Bit n set: a disk command is running on drive n.
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_drive_activity(m: *const Machine) -> u8 {
    let m = unsafe { &*m };
    (0..2).fold(0, |bits, n| bits | (m.drive_active(n) as u8) << n)
}

/// Host mouse input: movement in Juku counts (+x right, +y down; 1 count =
/// 1 pixel in gtr.com) and the buttons (bit 1 left, bit 0 right, 1 = pressed).
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_mouse(m: *mut Machine, dx: i32, dy: i32, buttons: u8) {
    unsafe { &mut *m }.mouse_input(dx, dy, buttons);
}

/// True while the running software has the mouse interrupt unmasked, i.e. it
/// uses the mouse (for click-to-capture).
///
/// # Safety
/// `m` must be a valid machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_mouse_enabled(m: *const Machine) -> bool {
    unsafe { &*m }.mouse_enabled()
}

/// A host section of a snapshot file (spec §4.6): a 4-byte tag that starts
/// with a lowercase letter, and the bytes to store.
#[repr(C)]
pub struct JukuSection {
    pub tag: [u8; 4],
    pub data: *const u8,
    pub len: usize,
}

/// A slice from a C pointer and length; NULL gives an empty slice.
///
/// # Safety
/// `data` must be NULL or point to `len` readable bytes.
unsafe fn bytes<'a>(data: *const u8, len: usize) -> &'a [u8] {
    if data.is_null() { &[] } else { unsafe { core::slice::from_raw_parts(data, len) } }
}

fn snapshot_error(e: SnapshotError) -> i32 {
    match e {
        SnapshotError::NotSnapshot => -1,
        SnapshotError::Damaged => -2,
        SnapshotError::Version => -3,
        SnapshotError::Old => -4,
    }
}

/// Saves the machine as a snapshot file: a PNG showing `thumbnail` (a whole
/// PNG file), or the screen as the core sees it if `thumbnail` is NULL, with
/// the machine state and the `count` host sections in it. Returns the file,
/// to be released with [`juku_snapshot_free`], and its length in `len`; or
/// NULL if the thumbnail is not a valid PNG or a tag is not a host tag.
///
/// # Safety
/// `m` must be a valid machine pointer and `len` a valid pointer.
/// `thumbnail` must be NULL or point to `thumbnail_len` bytes, `sections`
/// NULL or point to `count` sections, each with `len` readable bytes at
/// `data` (or NULL data).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_snapshot_save(
    m: *const Machine,
    thumbnail: *const u8,
    thumbnail_len: usize,
    sections: *const JukuSection,
    count: usize,
    len: *mut usize,
) -> *mut u8 {
    unsafe { *len = 0 };
    let thumbnail = (!thumbnail.is_null()).then(|| unsafe { bytes(thumbnail, thumbnail_len) });
    let sections: &[JukuSection] = if sections.is_null() { &[] } else { unsafe { core::slice::from_raw_parts(sections, count) } };
    let host: Vec<([u8; 4], &[u8])> = sections.iter().map(|s| (s.tag, unsafe { bytes(s.data, s.len) })).collect();
    match unsafe { &*m }.save_snapshot(thumbnail, &host) {
        Some(file) => {
            let file = file.into_boxed_slice();
            unsafe { *len = file.len() };
            Box::into_raw(file) as *mut u8
        }
        None => ptr::null_mut(),
    }
}

/// Releases a file from [`juku_snapshot_save`].
///
/// # Safety
/// `data` and `len` must be as [`juku_snapshot_save`] returned them, and
/// `data` not used afterwards (or be NULL).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_snapshot_free(data: *mut u8, len: usize) {
    if !data.is_null() {
        drop(unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(data, len)) });
    }
}

/// Restores a snapshot file. Returns 0, −1 (not a snapshot), −2 (damaged),
/// −3 (made by a newer version) or −4 (an older one); on error nothing
/// changes. The disks
/// come back from the file, clean; their image files are the host's
/// business.
///
/// # Safety
/// `m` must be a valid machine pointer; `data` must be NULL or point to
/// `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_snapshot_load(m: *mut Machine, data: *const u8, len: usize) -> i32 {
    match unsafe { &mut *m }.load_snapshot(unsafe { bytes(data, len) }) {
        Ok(()) => 0,
        Err(e) => snapshot_error(e),
    }
}

/// Finds section `tag` (4 bytes) in a snapshot file, typically a host
/// section, without loading it. Returns 0 with the section in `out` and
/// `out_len`, which point into `data`, or with NULL and 0 if the file has no
/// such section. Returns as [`juku_snapshot_load`] if the file is bad.
///
/// # Safety
/// `data` must be NULL or point to `len` readable bytes, `tag` to 4 bytes,
/// and `out` and `out_len` must be valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_snapshot_section(
    data: *const u8,
    len: usize,
    tag: *const u8,
    out: *mut *const u8,
    out_len: *mut usize,
) -> i32 {
    unsafe {
        *out = ptr::null();
        *out_len = 0;
    }
    let tag: [u8; 4] = unsafe { bytes(tag, 4) }.try_into().unwrap_or([0; 4]);
    match snapshot::section(unsafe { bytes(data, len) }, &tag) {
        Ok(found) => {
            if let Some(section) = found {
                unsafe {
                    *out = section.as_ptr();
                    *out_len = section.len();
                }
            }
            0
        }
        Err(e) => snapshot_error(e),
    }
}

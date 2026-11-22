//! Snapshot files (frontends/macos/src/Snapshots.m and the snapshot half of
//! Main.m, spec §6.8, §11.4): the core's PNG holding the machine (§4.6), with
//! a `meta` section from the app, and a `win ` section with the files the
//! snapshot came from, sealed so that only this installation can read it.
//! ROMs in a snapshot are matched to the bundled ROMs and the user's files by
//! their bytes; a ROM this computer does not have is kept in
//! `%LOCALAPPDATA%\Juku\ROMs`, so the settings can name it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use juku::snapshot::SnapshotError;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat24bppBGR, IWICImagingFactory, WICBitmapEncoderNoCache,
};
use windows::Win32::Security::Cryptography::{
    BCRYPT_AES_CBC_ALG_HANDLE, BCRYPT_BLOCK_PADDING, BCRYPT_HMAC_SHA256_ALG_HANDLE, BCRYPT_KEY_HANDLE, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    BCryptDecrypt, BCryptDestroyKey, BCryptEncrypt, BCryptGenRandom, BCryptGenerateSymmetricKey, BCryptHash,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, STREAM_SEEK_END, STREAM_SEEK_SET};
use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
use windows::Win32::UI::Shell::SHCreateMemStream;

use crate::disks::{self, Alert};
use crate::emu_window::{self, Emu};
use crate::file_dialogs::{self, Folder, SNAPSHOTS};
use crate::recent_disks;
use crate::renderer::{THUMBNAIL_HEIGHT, THUMBNAIL_WIDTH};
use crate::roms::{self, SYSTEM_ROMS, is_rom_file};
use crate::settings::{self, EXPANSION_ROM, EXPANSION_ROM_FILE, SYSTEM_ROM, SYSTEM_ROM_FILE};
use crate::strings::localized;
use crate::{settings_window, show_alerts};

pub const EXTENSION: &str = "jukusnap";

const CANNOT_READ: &str = "The snapshot “%@” could not be read.";
const CANNOT_SAVE: &str = "The snapshot could not be saved as “%@”.";

pub fn is_snapshot(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

/// A folder in `%LOCALAPPDATA%\Juku`, made if needed.
fn subfolder(name: &str) -> Option<PathBuf> {
    let folder = settings::folder()?.join(name);
    std::fs::create_dir_all(&folder).ok()?;
    Some(folder)
}

/// Machine → Quick Save and Quick Load use one snapshot file.
pub fn quick_save_path() -> Option<PathBuf> {
    settings::folder().map(|f| f.join("Quick Save.jukusnap"))
}

/// Why a snapshot could not be opened, as an alert format.
fn error_format(error: SnapshotError) -> &'static str {
    match error {
        SnapshotError::NotSnapshot => "“%@” is not a Juku snapshot.",
        SnapshotError::Damaged => "The snapshot “%@” is damaged.",
        SnapshotError::Version => "“%@” was made by a newer version of Juku.",
        SnapshotError::Old => "“%@” was made by an older version of Juku.",
    }
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

/// Section `tag` of a snapshot file.
fn section<'a>(file: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    juku::snapshot::section(file, tag).ok().flatten()
}

// ---- The meta section -----------------------------------------------------------
// UTF-8 lines of key=value, readable on any platform: app (name and
// version), saved (UTC, ISO 8601), rom (a bundled ROM's name or the ROM
// file's name), and expansion, disk0 and disk1 (file names). Paths are left
// out: they are of no use on another computer and tell more about the user
// than a shared file should.

fn meta_text(values: &BTreeMap<&str, String>) -> Vec<u8> {
    values.iter().map(|(key, value)| format!("{key}={}\n", value.replace(['\r', '\n'], " "))).collect::<String>().into_bytes()
}

fn parse_lines(data: &[u8]) -> Vec<(String, String)> {
    String::from_utf8_lossy(data)
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn utc_now() -> String {
    let t = unsafe { GetSystemTime() };
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

/// A ROM's name for `meta`: a bundled ROM's own name, or the file's name.
fn rom_meta_name(setting: &str) -> String {
    if is_rom_file(setting) { file_name(setting) } else { setting.to_string() }
}

// ---- The sealed win section ------------------------------------------------------
// The files a snapshot came from, as `key=value` lines: disk0 and disk1 with
// their file IDs (recent_disks.rs, FileRef), and the paths of ROM files in
// SystemROM and ExpansionROM. They are encrypted with AES-256-CBC and
// authenticated with HMAC-SHA256, both from BCrypt, which is part of
// Windows. The keys are 64 random bytes in SnapshotKey in Juku.ini: the
// first half encrypts, the second authenticates. Another computer, or this
// one after Juku.ini was deleted, cannot read the section, and it cannot even
// tell whose it is: the file holds no identifier, only `version, IV,
// ciphertext, MAC`. Not DPAPI: its output names the user's master key.

const SEAL_VERSION: u8 = 1;
const SEAL_KEY: usize = 32;
const SEAL_IV: usize = 16;
const SEAL_MAC: usize = 32;
const SNAPSHOT_KEY: &str = "SnapshotKey";

fn random(bytes: &mut [u8]) -> bool {
    unsafe { BCryptGenRandom(None, bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }.0 >= 0
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok()).collect()
}

/// The installation's keys, made on first use.
fn snapshot_key() -> Option<Vec<u8>> {
    if let Some(key) = settings::string_setting(SNAPSHOT_KEY).and_then(|k| unhex(&k)).filter(|k| k.len() == 2 * SEAL_KEY) {
        return Some(key);
    }

    let mut key = vec![0u8; 2 * SEAL_KEY];

    if !random(&mut key) {
        return None;
    }

    settings::set_string_setting(SNAPSHOT_KEY, Some(&hex(&key)));
    Some(key)
}

fn seal_mac(key: &[u8], data: &[u8]) -> Option<[u8; SEAL_MAC]> {
    let mut mac = [0u8; SEAL_MAC];
    let status = unsafe { BCryptHash(BCRYPT_HMAC_SHA256_ALG_HANDLE, Some(&key[SEAL_KEY..]), data, &mut mac) };
    (status.0 >= 0).then_some(mac)
}

/// AES-256-CBC with PKCS#7 padding, one way or the other.
fn aes(key: &[u8], iv: &[u8], input: &[u8], encrypt: bool) -> Option<Vec<u8>> {
    let mut handle = BCRYPT_KEY_HANDLE::default();

    if unsafe { BCryptGenerateSymmetricKey(BCRYPT_AES_CBC_ALG_HANDLE, &mut handle, None, &key[..SEAL_KEY], 0) }.0 < 0 {
        return None;
    }

    let mut output = vec![0u8; input.len() + SEAL_IV];
    let mut len = 0u32;
    // BCrypt moves the IV on as it goes.
    let mut iv = iv.to_vec();

    let status = unsafe {
        if encrypt {
            BCryptEncrypt(handle, Some(input), None, Some(&mut iv), Some(&mut output), &mut len, BCRYPT_BLOCK_PADDING)
        } else {
            BCryptDecrypt(handle, Some(input), None, Some(&mut iv), Some(&mut output), &mut len, BCRYPT_BLOCK_PADDING)
        }
    };

    let _ = unsafe { BCryptDestroyKey(handle) };

    if status.0 < 0 {
        return None;
    }

    output.truncate(len as usize);
    Some(output)
}

/// The win section for `plain`, or None if it cannot be made.
fn seal(plain: &[u8]) -> Option<Vec<u8>> {
    let key = snapshot_key()?;
    let mut sealed = vec![SEAL_VERSION];
    let mut iv = [0u8; SEAL_IV];

    if !random(&mut iv) {
        return None;
    }

    sealed.extend_from_slice(&iv);
    sealed.extend(aes(&key, &iv, plain, true)?);
    let mac = seal_mac(&key, &sealed)?;
    sealed.extend_from_slice(&mac);
    Some(sealed)
}

/// What a win section holds, or None if it is missing, damaged or was
/// sealed by another installation.
fn open_sealed(sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < 1 + SEAL_IV + 16 + SEAL_MAC || sealed[0] != SEAL_VERSION {
        return None;
    }

    let key = snapshot_key()?;
    let body = sealed.len() - SEAL_MAC;
    let mac = seal_mac(&key, &sealed[..body])?;
    // Compared in constant time.
    let differs = mac.iter().zip(&sealed[body..]).fold(0u8, |acc, (a, b)| acc | (a ^ b));

    if differs != 0 {
        return None;
    }

    aes(&key, &sealed[1..1 + SEAL_IV], &sealed[1 + SEAL_IV..body], false)
}

// ---- ROMs --------------------------------------------------------------------------

/// An expansion ROM as a snapshot stores it: without its trailing FFh.
fn trimmed_expansion(image: &[u8]) -> &[u8] {
    let len = image.iter().rposition(|&b| b != 0xFF).map_or(0, |i| i + 1);
    &image[..len]
}

/// The path of a ROM file that `matches`: one of `candidates`, one kept
/// before, or else `contents` written to a new file in `ROMs` named after
/// `name`. None if that cannot be written.
fn kept_rom_file(contents: &[u8], name: &str, candidates: &[Option<String>], matches: impl Fn(&[u8]) -> bool) -> Option<String> {
    let folder = subfolder("ROMs");
    let mut paths: Vec<PathBuf> = candidates.iter().flatten().filter(|p| is_rom_file(p)).map(PathBuf::from).collect();

    if let Some(entries) = folder.as_ref().and_then(|f| std::fs::read_dir(f).ok()) {
        paths.extend(entries.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_file()));
    }

    for path in &paths {
        if std::fs::read(path).is_ok_and(|file| matches(&file)) {
            return Some(path.to_string_lossy().into_owned());
        }
    }

    let folder = folder?;
    let stem = Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let base: String = stem.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '-' } else { c }).collect();
    let base = base.trim_matches(['.', ' ']);
    let base = if base.is_empty() { "ROM" } else { base };

    for n in 1..100 {
        let file = if n == 1 { format!("{base}.bin") } else { format!("{base} {n}.bin") };
        let path = folder.join(file);

        if path.exists() {
            continue;
        }

        let written = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).and_then(|mut f| {
            use std::io::Write;
            f.write_all(contents)
        });
        return written.is_ok().then(|| path.to_string_lossy().into_owned());
    }

    None
}

/// The SystemROM setting for a snapshot's ROM: the bundled ROM or the
/// user's file with the same bytes, or a kept copy. `name` is the ROM's name
/// in the snapshot, and `path` the file it came from, if this installation
/// knows it.
fn system_rom_setting(rom: &[u8], name: Option<&str>, path: Option<&str>) -> Option<String> {
    for bundled in &SYSTEM_ROMS {
        if roms::load_system_rom(bundled.name).is_some_and(|image| *image == *rom) {
            return Some(bundled.name.to_string());
        }
    }

    let candidates = [path.map(str::to_string), settings::string_setting(SYSTEM_ROM), settings::string_setting(SYSTEM_ROM_FILE)];
    kept_rom_file(rom, name.unwrap_or("ROM"), &candidates, |file| file == rom)
}

/// The ExpansionROM setting for a snapshot's expansion window (not empty):
/// the user's file with the same program, or a kept copy. A copy is padded
/// with FFh to the length in its program header, so that it passes the check
/// again.
fn expansion_rom_setting(window: &[u8], name: Option<&str>, path: Option<&str>) -> Option<String> {
    let mut image = window.to_vec();

    if image.len() >= 7 {
        let declared = image[5] as usize | (image[6] as usize) << 8;

        if image.len() < declared {
            image.resize(declared, 0xFF);
        }
    }

    let candidates = [path.map(str::to_string), settings::string_setting(EXPANSION_ROM), settings::string_setting(EXPANSION_ROM_FILE)];
    kept_rom_file(&image, name.unwrap_or("Expansion ROM"), &candidates, |file| trimmed_expansion(file) == window)
}

// ---- The thumbnail -------------------------------------------------------------------

/// BGRA pixels as a PNG, with WIC.
fn encode_png(bgra: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let stream = SHCreateMemStream(None)?;
        let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null()).ok()?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache).ok()?;
        let mut frame = None;
        let mut options = None;
        encoder.CreateNewFrame(&mut frame, &mut options).ok()?;
        let frame = frame?;
        frame.Initialize(options.as_ref()).ok()?;
        frame.SetSize(width, height).ok()?;
        let mut format = GUID_WICPixelFormat24bppBGR;
        frame.SetPixelFormat(&mut format).ok()?;

        if format != GUID_WICPixelFormat24bppBGR {
            return None;
        }

        let bgr: Vec<u8> = bgra.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        frame.WritePixels(height, width * 3, &bgr).ok()?;
        frame.Commit().ok()?;
        encoder.Commit().ok()?;

        let mut size = 0u64;
        stream.Seek(0, STREAM_SEEK_END, Some(&mut size)).ok()?;
        stream.Seek(0, STREAM_SEEK_SET, None).ok()?;
        let mut png = vec![0u8; size as usize];
        let mut read = 0u32;
        stream.Read(png.as_mut_ptr().cast(), size as u32, Some(&mut read)).ok().ok()?;
        png.truncate(read as usize);
        Some(png)
    }
}

/// The screen as the window shows it, as a PNG for a snapshot's thumbnail.
fn thumbnail_png(emu: &mut Emu) -> Option<Vec<u8>> {
    let pixels = emu.thumbnail()?;
    encode_png(&pixels, THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT)
}

// ---- Saving and restoring --------------------------------------------------------------

/// The machine as a snapshot file, as it is now, its disks as they are in
/// memory, with the picture the window shows.
fn snapshot_data(emu: &mut Emu) -> Option<Vec<u8>> {
    let mut meta = BTreeMap::new();
    meta.insert("app", format!("Juku {}", env!("JUKU_VERSION")));
    meta.insert("saved", utc_now());
    meta.insert("rom", rom_meta_name(emu.roms().system()));

    if let Some(expansion) = emu.roms().expansion() {
        meta.insert("expansion", file_name(expansion));
    }

    // Where the files are on this computer, readable only here.
    let mut links = Vec::new();

    for (drive, key) in ["disk0", "disk1"].into_iter().enumerate() {
        if disks::DiskDrives::has_disk(emu.machine(), drive) {
            meta.insert(key, emu.disks().name(drive));
        }

        if let Some(path) = emu.disks().path(drive) {
            links.push((key.to_string(), path.display().to_string()));
        }
    }

    if is_rom_file(emu.roms().system()) {
        links.push(("SystemROM".to_string(), emu.roms().system().to_string()));
    }

    if let Some(expansion) = emu.roms().expansion() {
        links.push(("ExpansionROM".to_string(), expansion.to_string()));
    }

    let meta = meta_text(&meta);
    let links: String = links.iter().map(|(key, value)| format!("{key}={value}\n")).collect();
    let sealed = seal(links.as_bytes());
    let mut sections: Vec<([u8; 4], &[u8])> = vec![(*b"meta", &meta)];

    if let Some(sealed) = &sealed {
        sections.push((*b"win ", sealed));
    }

    let png = thumbnail_png(emu);
    let mut file = emu.machine().save_snapshot(png.as_deref(), &sections);

    if file.is_none() && png.is_some() {
        file = emu.machine().save_snapshot(None, &sections);
    }

    file
}

/// Restores a snapshot file. The settings are pointed at its ROMs before
/// they are applied, so that applying them does not start the machine again.
/// The disks are the snapshot's, with their files on this computer if they
/// are still there. Returns what went wrong on the way, or the alert format
/// that says why the file could not be used.
fn restore(emu: &mut Emu, file: &[u8]) -> Result<Vec<Alert>, &'static str> {
    emu.machine().load_snapshot(file).map_err(error_format)?;

    let meta = parse_lines(section(file, b"meta").unwrap_or_default());
    let meta = |key: &str| meta.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
    // Empty for a snapshot from another computer.
    let links = section(file, b"win ").and_then(open_sealed).map(|plain| parse_lines(&plain)).unwrap_or_default();
    let link = |key: &str| links.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());

    // The ROMs as the snapshot put them in the machine, matched by their bytes.
    let rom = emu.machine().system_rom();

    if let Some(name) = system_rom_setting(&rom, meta("rom").as_deref(), link("SystemROM").as_deref()) {
        emu.roms_mut().set_system(&name);

        if is_rom_file(&name) {
            settings::set_string_setting(SYSTEM_ROM_FILE, Some(&name));
        }

        settings::set_string_setting(SYSTEM_ROM, Some(&name));
    }

    let window = emu.machine().expansion_rom().to_vec();
    let expansion =
        if window.is_empty() { None } else { expansion_rom_setting(&window, meta("expansion").as_deref(), link("ExpansionROM").as_deref()) };

    if window.is_empty() || expansion.is_some() {
        emu.roms_mut().set_expansion(expansion.as_deref());

        if let Some(path) = &expansion {
            settings::set_string_setting(EXPANSION_ROM_FILE, Some(path));
        }

        settings::set_string_setting(EXPANSION_ROM, expansion.as_deref());
    }

    // The key click follows the setting, not the snapshot.
    let alerts = emu.apply_settings();

    let names = [meta("disk0"), meta("disk1")];
    let paths = [link("disk0"), link("disk1")];
    emu.with_disks(|disks, m| disks.attach_restored(m, names, paths));
    emu.machine_restored();
    Ok(alerts)
}

/// Opens a snapshot file (`file`, read from `path`), after asking about each
/// changed disk, since the snapshot brings disks of its own. True once the
/// snapshot is in.
fn open(owner: HWND, file: Option<Vec<u8>>, path: &Path) -> bool {
    let Some(file) = file else {
        show_alerts(owner, vec![Alert::new(CANNOT_READ, path)]);
        return false;
    };

    // A file that is not a snapshot is reported before anything is asked.
    if let Err(error) = juku::snapshot::section(&file, b"meta") {
        show_alerts(owner, vec![Alert::new(error_format(error), path)]);
        return false;
    }

    if !disks::may_remove_all(owner) {
        return false;
    }

    let alerts = match emu_window::with_emu(|emu| restore(emu, &file)) {
        Some(Ok(alerts)) => alerts,
        Some(Err(format)) => {
            show_alerts(owner, vec![Alert::new(format, path)]);
            return false;
        }
        None => return false,
    };

    settings_window::refresh();
    show_alerts(owner, alerts);
    true
}

/// Opens the snapshot at `path`: from Explorer, a drop, or the dialog.
pub fn open_path(owner: HWND, path: &Path) {
    if open(owner, std::fs::read(path).ok(), path) {
        recent_disks::add_to_jump_list(path);
    }
}

/// Machine → Open Snapshot… (Alt+O).
pub fn choose_and_open(owner: HWND) {
    let folder = subfolder("Snapshots");

    if let Some(path) = file_dialogs::open_file(owner, "Open Snapshot…", &SNAPSHOTS, folder.as_deref().map(Folder::Default)) {
        open_path(owner, &path);
    }
}

/// Machine → Save Snapshot… (Alt+Shift+S). The snapshot is taken when the
/// item is chosen, not when the dialog closes. The dialog opens in
/// `Snapshots` the first time, then in the folder used last.
pub fn save(owner: HWND) {
    let Some(Some(data)) = emu_window::with_emu(snapshot_data) else {
        return;
    };

    let t = unsafe { GetLocalTime() };
    let stamp = format!("{:04}-{:02}-{:02} {:02}.{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute);
    let name = localized("Juku %@").replace("%@", &stamp);
    let folder = subfolder("Snapshots");

    let Some(path) = file_dialogs::save_file(owner, "Save Snapshot…", &SNAPSHOTS, EXTENSION, folder.as_deref().map(Folder::Default), &name)
    else {
        return;
    };

    if crate::disks::write_atomically(&path, &data).is_ok() {
        recent_disks::add_to_jump_list(&path);
    } else {
        show_alerts(owner, vec![Alert::new(CANNOT_SAVE, &path)]);
    }
}

/// Machine → Quick Save (Alt+S).
pub fn quick_save(owner: HWND) {
    let Some(data) = emu_window::with_emu(snapshot_data) else {
        return;
    };

    let path = quick_save_path();

    match (data, &path) {
        (Some(data), Some(path)) if crate::disks::write_atomically(path, &data).is_ok() => {
            emu_window::with_emu(|emu| emu.show_note(localized("Snapshot Saved")));
        }
        _ => show_alerts(owner, vec![Alert::new(CANNOT_SAVE, path.as_deref().unwrap_or(Path::new("Quick Save.jukusnap")))]),
    }
}

/// Machine → Quick Load (Alt+L), disabled until there is a Quick Save file.
pub fn quick_load(owner: HWND) {
    let Some(path) = quick_save_path() else {
        return;
    };

    if open(owner, std::fs::read(&path).ok(), &path) {
        emu_window::with_emu(|emu| emu.show_note(localized("Snapshot Loaded")));
    }
}

/// True if Quick Load has a file to load.
pub fn can_quick_load() -> bool {
    quick_save_path().is_some_and(|p| p.is_file())
}

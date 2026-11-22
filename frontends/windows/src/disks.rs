//! The disks in the two drives (frontends/macos/src/Disks.m, spec §6.3,
//! §11.4). A disk lives in memory, in the core: its file is read when it
//! goes in and written only when the user saves it, always through the save
//! dialog. A changed disk asks to be saved before it leaves its drive.
//!
//! `DiskDrives` shows no window: what goes wrong comes back as an `Alert`.
//! The functions after it ask the user, and take the window's state only
//! around their dialogs.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use juku::Machine;
use juku::floppy::Disk;
use windows::Win32::Foundation::HWND;
use windows::Win32::Storage::FileSystem::{REPLACEFILE_WRITE_THROUGH, ReplaceFileW};
use windows::Win32::UI::Controls::{
    LIM_LARGE, LoadIconMetric, TASKDIALOG_BUTTON, TASKDIALOG_FLAGS, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TDF_ALLOW_DIALOG_CANCELLATION,
    TDF_USE_HICON_MAIN, TaskDialogIndirect,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, IDCANCEL, IDI_QUESTION};
use windows::core::{HSTRING, PCWSTR};

use crate::emu_window;
use crate::file_dialogs::{self, DISK_IMAGES, Folder};
use crate::recent_disks::{self, FileRef};
use crate::settings::{self, SECTION_DRIVES};
use crate::show_alerts;
use crate::strings::{self, localized};

pub const DRIVES: usize = 2;

/// Something to tell the user about a file: a format from the strings, with
/// `%@` for the file's name.
pub struct Alert {
    format: &'static str,
    name: String,
}

impl Alert {
    pub fn new(format: &'static str, path: &Path) -> Alert {
        Alert { format, name: file_name(path) }
    }

    /// An alert naming something other than a file: `%@` becomes `name`.
    pub fn named(format: &'static str, name: &str) -> Alert {
        Alert { format, name: name.to_string() }
    }

    pub fn message(&self) -> String {
        localized(self.format).replace("%@", &self.name)
    }
}

const CANNOT_READ: &str = "The disk image “%@” could not be read.";
const NOT_A_DISK: &str = "“%@” is not a Juku disk image (800 KB or 400 KB expected).";
const CANNOT_SAVE: &str = "Changes to “%@” could not be saved.";

/// The key of a drive's entries in the Drives section: A or B.
fn drive_key(drive: usize) -> &'static str {
    ["A", "B"][drive]
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Writes `data` to a new file beside `path`, which then replaces it, so a
/// crash leaves the old image whole. ReplaceFileW keeps the old file's
/// attributes and permissions.
pub fn write_atomically(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".juku-tmp");
    let temp = path.with_file_name(name);
    std::fs::write(&temp, data)?;

    let replaced = unsafe { ReplaceFileW(&HSTRING::from(path), &HSTRING::from(temp.as_path()), PCWSTR::null(), REPLACEFILE_WRITE_THROUGH, None, None) };

    if replaced.is_err() && std::fs::rename(&temp, path).is_err() {
        let _ = std::fs::remove_file(&temp);
        return Err(std::io::Error::last_os_error());
    }

    Ok(())
}

/// The image in `path`, if it is one the drives take.
fn read_image(path: &Path) -> Result<Vec<u8>, Alert> {
    let data = std::fs::read(path).map_err(|_| Alert::new(CANNOT_READ, path))?;

    if !Disk::size_ok(data.len()) {
        return Err(Alert::new(NOT_A_DISK, path));
    }

    Ok(data)
}

pub struct DiskDrives {
    paths: [Option<PathBuf>; DRIVES],
    names: [Option<String>; DRIVES], // for a disk without a file
    not_saved: [bool; DRIVES],       // a new disk or one from a snapshot, until it is saved
}

impl DiskDrives {
    pub fn new() -> DiskDrives {
        DiskDrives { paths: [None, None], names: [None, None], not_saved: [false; DRIVES] }
    }

    pub fn has_disk(m: &Machine, drive: usize) -> bool {
        m.disk(drive).is_some()
    }

    /// True if the running program has written to the disk since it was
    /// put in, saved or restored.
    pub fn changed(m: &Machine, drive: usize) -> bool {
        m.disk(drive).is_some_and(|d| d.dirty())
    }

    pub fn is_write_protected(m: &Machine, drive: usize) -> bool {
        m.disk(drive).is_some_and(|d| d.write_protect())
    }

    /// The disk's file; None for an empty drive or a disk from a snapshot
    /// whose file is not on this computer.
    pub fn path(&self, drive: usize) -> Option<&Path> {
        self.paths[drive].as_deref()
    }

    /// The disk's name: its file's, the one a snapshot gave it, or Disk A.juk.
    pub fn name(&self, drive: usize) -> String {
        match (&self.paths[drive], &self.names[drive]) {
            (Some(path), _) => file_name(path),
            (None, Some(name)) => name.clone(),
            (None, None) => localized("Disk %@.juk").replace("%@", drive_key(drive)),
        }
    }

    /// The disk's name for the Disks menu, in square brackets for a disk
    /// that is not saved to a file, with a dot for one the running program
    /// has written to. None for an empty drive.
    pub fn label(&self, m: &Machine, drive: usize) -> Option<String> {
        let name = self.name(drive);
        let name = if self.not_saved[drive] { format!("[{name}]") } else { name };
        let edited = if Self::changed(m, drive) { " •" } else { "" };
        Self::has_disk(m, drive).then(|| format!("{name}{edited}"))
    }

    /// The status line at the top of the drive's submenu: Not saved to a
    /// file, for a new disk or one from a snapshot, and Edited, for one the
    /// running program has written to. None when neither is so.
    pub fn status(&self, m: &Machine, drive: usize) -> Option<String> {
        let parts: Vec<&str> = [(self.not_saved[drive], "Not saved to a file"), (Self::changed(m, drive), "Edited")]
            .into_iter()
            .filter_map(|(on, key)| on.then(|| localized(key)))
            .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// Puts `data`, read from `path`, in the drive in place of the disk there.
    fn put(&mut self, m: &mut Machine, path: &Path, data: Vec<u8>, drive: usize, write_protect: bool) {
        if m.insert_disk(drive, data, write_protect).is_ok() {
            self.paths[drive] = Some(path.to_path_buf());
            self.names[drive] = None;
            self.not_saved[drive] = false;
        }
    }

    fn insert(&mut self, m: &mut Machine, path: &Path, data: Vec<u8>, drive: usize) {
        self.put(m, path, data, drive, false);
        recent_disks::note(path);
        self.save_drives(m);
    }

    /// A formatted, empty disk without a file, in place of the disk there.
    fn insert_blank(&mut self, m: &mut Machine, drive: usize) {
        m.insert_blank_disk(drive);
        self.paths[drive] = None;
        self.names[drive] = Some(localized("Untitled.juk").to_string());
        self.not_saved[drive] = true;
        self.save_drives(m);
    }

    fn eject(&mut self, m: &mut Machine, drive: usize) {
        m.eject_disk(drive);
        self.paths[drive] = None;
        self.names[drive] = None;
        self.not_saved[drive] = false;
        self.save_drives(m);
    }

    /// Writes the disk to `path`, which becomes its file.
    fn save(&mut self, m: &mut Machine, drive: usize, path: &Path) -> Result<(), Alert> {
        let Some(disk) = m.disk_mut(drive) else {
            return Ok(());
        };

        write_atomically(path, disk.data()).map_err(|_| Alert::new(CANNOT_SAVE, path))?;
        disk.clear_dirty();
        self.paths[drive] = Some(path.to_path_buf());
        self.names[drive] = None;
        self.not_saved[drive] = false;
        recent_disks::note(path);
        // The new file has a new ID.
        self.save_drives(m);
        Ok(())
    }

    pub fn toggle_write_protect(&mut self, m: &mut Machine, drive: usize) {
        if let Some(disk) = m.disk_mut(drive) {
            disk.set_write_protect(!disk.write_protect());
        }

        self.save_drives(m);
    }

    /// The files in the drives and their Write Protect go to the Drives
    /// section, so the next launch starts with them (spec §6.3).
    pub fn save_drives(&self, m: &Machine) {
        let mut entries = Vec::new();

        for (drive, path) in self.paths.iter().enumerate() {
            if let Some(path) = path {
                let key = drive_key(drive);
                entries.extend(FileRef::new(path).to_entries(key));
                entries.push((format!("{key}.WriteProtected"), (Self::is_write_protected(m, drive) as u8).to_string()));
            }
        }

        settings::set_section(SECTION_DRIVES, entries);
    }

    /// After a snapshot is restored, which brings its own disks (Disks.m,
    /// attachRestoredDisks): per drive, `names` holds the snapshot's name
    /// for the disk and `paths` the path of its file on this computer. A
    /// path with no file at it is left out.
    pub fn attach_restored(&mut self, m: &Machine, names: [Option<String>; DRIVES], paths: [Option<String>; DRIVES]) {
        for (drive, (name, path)) in names.into_iter().zip(paths).enumerate() {
            let disk = Self::has_disk(m, drive);
            self.paths[drive] = path.map(PathBuf::from).filter(|p| disk && p.is_file());
            self.names[drive] = name.filter(|_| disk);
            self.not_saved[drive] = disk;
        }

        self.save_drives(m);
    }

    /// Called at launch, before the machine runs, so it boots from the disk
    /// in A:. An image that has gone or no longer loads is reported, and its
    /// drive is left empty.
    pub fn restore_drives(&mut self, m: &mut Machine) -> Vec<Alert> {
        let entries = settings::section(SECTION_DRIVES);
        let mut alerts = Vec::new();

        for drive in 0..DRIVES {
            let key = drive_key(drive);
            let Some(saved) = FileRef::from_entries(&entries, key) else {
                continue;
            };
            let protect = settings::get(SECTION_DRIVES, &format!("{key}.WriteProtected")).is_some_and(|v| v == "1");

            match saved.resolve() {
                Some(path) => match read_image(&path) {
                    Ok(data) => self.put(m, &path, data, drive, protect),
                    Err(alert) => alerts.push(alert),
                },
                None => alerts.push(Alert::new(CANNOT_READ, &saved.path)),
            }
        }

        self.save_drives(m);
        alerts
    }
}

// ---- Asking the user ----------------------------------------------------------------

thread_local! {
    // The save alert or the save dialog is open: a file from Explorer or
    // another start must not change the drives meanwhile.
    static ASKING: Cell<bool> = const { Cell::new(false) };
}

pub fn asking() -> bool {
    ASKING.get()
}

/// Runs a dialog with `ASKING` set.
fn ask<R>(f: impl FnOnce() -> R) -> R {
    let outer = ASKING.replace(true);
    let result = f();
    ASKING.set(outer);
    result
}

/// The Insert dialog: an image file, or None if the user cancels.
pub fn choose_disk(owner: HWND, drive: usize) -> Option<PathBuf> {
    let title = if drive == 0 { "Insert in A:" } else { "Insert in B:" };
    file_dialogs::open_file(owner, title, &DISK_IMAGES, None)
}

/// Puts the image in `path` in the drive, once the disk there may go: from
/// Insert…, Recent Images, a drop or Explorer.
pub fn insert_file(owner: HWND, path: &Path, drive: usize) {
    let data = match read_image(path) {
        Ok(data) => data,
        Err(alert) => return show_alerts(owner, vec![alert]),
    };

    if may_remove(owner, drive) {
        emu_window::with_emu(|emu| emu.with_disks(|disks, m| disks.insert(m, path, data, drive)));
    }
}

/// New Disk: a formatted, empty disk, once the disk there may go.
pub fn insert_new_disk(owner: HWND, drive: usize) {
    if may_remove(owner, drive) {
        emu_window::with_emu(|emu| emu.with_disks(|disks, m| disks.insert_blank(m, drive)));
    }
}

pub fn eject_disk(owner: HWND, drive: usize) {
    if may_remove(owner, drive) {
        emu_window::with_emu(|emu| emu.with_disks(|disks, m| disks.eject(m, drive)));
    }
}

/// Save Disk…: the save dialog, on the disk's own file, or on its name in
/// Documents. True if the disk was saved.
pub fn save_disk(owner: HWND, drive: usize) -> bool {
    let Some(Some((name, folder))) = emu_window::with_emu(|emu| {
        DiskDrives::has_disk(emu.machine(), drive).then(|| (emu.disks().name(drive), emu.disks().path(drive).and_then(Path::parent).map(Path::to_path_buf)))
    }) else {
        return false;
    };

    let folder = folder.or_else(file_dialogs::documents_folder);

    let Some(path) = ask(|| file_dialogs::save_file(owner, "Save Disk…", &DISK_IMAGES, "juk", folder.as_deref().map(Folder::Always), &name)) else {
        return false;
    };

    match emu_window::with_emu(|emu| emu.with_disks(|disks, m| disks.save(m, drive, &path))) {
        Some(Ok(())) => true,
        Some(Err(alert)) => {
            show_alerts(owner, vec![alert]);
            false
        }
        None => false,
    }
}

/// Asks before the disk in `drive` leaves it, if it is changed: Save…, Don't
/// Save or Cancel (Disks.m, mayRemoveDisk). True if it may go.
pub fn may_remove(owner: HWND, drive: usize) -> bool {
    loop {
        let Some((changed, name)) = emu_window::with_emu(|emu| (DiskDrives::changed(emu.machine(), drive), emu.disks().name(drive))) else {
            return false;
        };

        if !changed {
            return true;
        }

        match ask(|| ask_to_save(owner, &name, drive)) {
            // Saved, or asked again.
            Answer::Save => {
                save_disk(owner, drive);
            }
            Answer::DontSave => return true,
            Answer::Cancel => return false,
        }
    }
}

/// `may_remove` for both drives, A: first: on quit and before a snapshot
/// is opened.
pub fn may_remove_all(owner: HWND) -> bool {
    (0..DRIVES).all(|drive| may_remove(owner, drive))
}

enum Answer {
    Save,
    DontSave,
    Cancel,
}

/// The save alert: a task dialog with the Mac's message and note, and the
/// system's question mark icon. Its buttons are in the app's language,
/// Cancel included, and Esc cancels.
fn ask_to_save(owner: HWND, name: &str, drive: usize) -> Answer {
    const SAVE: i32 = 100;
    const DONT_SAVE: i32 = 101;

    let title = HSTRING::from(localized("Window"));
    let message = HSTRING::from(strings::format(localized("Do you want to save the changes to “%@” in %@?"), &[name, ["A:", "B:"][drive]]));
    let note = HSTRING::from(localized("Your changes are lost if you don't save them."));
    let save = HSTRING::from(localized("Save…"));
    let dont_save = HSTRING::from(localized("Don't Save"));
    let cancel = HSTRING::from(localized("Cancel"));
    let buttons = [
        TASKDIALOG_BUTTON { nButtonID: SAVE, pszButtonText: PCWSTR(save.as_ptr()) },
        TASKDIALOG_BUTTON { nButtonID: DONT_SAVE, pszButtonText: PCWSTR(dont_save.as_ptr()) },
        TASKDIALOG_BUTTON { nButtonID: IDCANCEL.0, pszButtonText: PCWSTR(cancel.as_ptr()) },
    ];
    // Task dialogs have no question mark of their own (TD_*_ICON).
    let icon = unsafe { LoadIconMetric(None, IDI_QUESTION, LIM_LARGE) }.ok();
    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner,
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | if icon.is_some() { TDF_USE_HICON_MAIN } else { TASKDIALOG_FLAGS(0) },
        Anonymous1: TASKDIALOGCONFIG_0 { hMainIcon: icon.unwrap_or_default() },
        pszWindowTitle: PCWSTR(title.as_ptr()),
        pszMainInstruction: PCWSTR(message.as_ptr()),
        pszContent: PCWSTR(note.as_ptr()),
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: SAVE,
        ..Default::default()
    };
    let mut pressed = IDCANCEL.0;
    let shown = unsafe { TaskDialogIndirect(&config, Some(&mut pressed), None, None) };

    if let Some(icon) = icon {
        let _ = unsafe { DestroyIcon(icon) };
    }

    if shown.is_err() {
        return Answer::Cancel;
    }

    match pressed {
        SAVE => Answer::Save,
        DONT_SAVE => Answer::DontSave,
        _ => Answer::Cancel,
    }
}

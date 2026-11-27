//! Juku for Windows (spec §11): the counterpart of frontends/macos/src/Main.m.
//! It builds the menus and carries out their commands, creates the machine and
//! the window, and runs the main loop that paces the machine by the display
//! (§11.2).
//!
//! Commands run outside `with_emu`: one that shows a dialog or an alert takes
//! the window's state only for the moments it needs it, so the machine keeps
//! running while the dialog is open, as it does behind a sheet on the Mac.

// Debug builds are console programs, so their log shows in the terminal.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Writes a line to the debugger's output (DebugView shows it too), and in
/// debug builds to the console.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log_line(&format!($($arg)*))
    };
}

mod audio;
mod disks;
mod emu_window;
mod file_dialogs;
mod file_types;
mod keyboard_picture;
mod keyboard_window;
mod keymap;
mod recent_disks;
mod renderer;
mod roms;
mod settings;
mod settings_window;
mod snapshots;
mod strings;
mod theme;

use std::cell::{Cell, RefCell};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use juku::Machine;
use juku::audio::AudioRing;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, RECT, WAIT_OBJECT_0, WPARAM};
use windows::Win32::Globalization::{GetLocaleInfoEx, LOCALE_SDECIMAL};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateMutexW, CreateWaitableTimerExW, ExitProcess, INFINITE, SetWaitableTimer, TIMER_ALL_ACCESS,
};
use windows::Win32::UI::Controls::{TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TD_ERROR_ICON, TDCBF_OK_BUTTON, TaskDialog, TaskDialogIndirect};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, SetFocus, VK_LMENU, VK_OEM_COMMA, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, Result, w};

use disks::{Alert, DRIVES, DiskDrives};
use roms::{InstalledRoms, SYSTEM_ROMS};
use settings::{EXPANSION_ROM, EXPANSION_ROM_FILE, SYSTEM_ROM, SYSTEM_ROM_FILE};
use strings::{localized, menu_text, menu_title};

/// The app's icon in Juku.rc.
pub const APP_ICON: PCWSTR = PCWSTR(1 as *const u16);

pub fn log_line(line: &str) {
    let text = HSTRING::from(format!("Juku: {line}\n"));
    unsafe { OutputDebugStringW(&text) };

    if cfg!(debug_assertions) {
        eprintln!("{line}");
    }
}

fn fatal_error(message: &str) -> ! {
    unsafe {
        MessageBoxW(None, &HSTRING::from(localized(message)), &HSTRING::from(localized("Window")), MB_OK | MB_ICONERROR);
        ExitProcess(1)
    }
}

/// Shows each alert in turn (the Mac's NSAlert): the message as the main
/// instruction, the error icon, since every alert says that something could
/// not be done, and an OK button.
pub fn show_alerts(owner: HWND, alerts: Vec<Alert>) {
    for alert in alerts {
        let _ = unsafe {
            TaskDialog(
                Some(owner),
                None,
                &HSTRING::from(localized("Window")),
                &HSTRING::from(alert.message()),
                PCWSTR::null(),
                TDCBF_OK_BUTTON,
                TD_ERROR_ICON,
                None,
            )
        };
    }
}

// ---- Menus --------------------------------------------------------------------
// The menus of §6.2 and §11.3, in the same order.

const ID_EXIT: u16 = 1;
const ID_ABOUT: u16 = 2;
const ID_SETTINGS: u16 = 3;
const ID_RESET: u16 = 100;
const ID_POWER_CYCLE: u16 = 101;
const ID_PAUSE: u16 = 102;
const ID_WARP: u16 = 103;
const ID_OPEN_SNAPSHOT: u16 = 104;
const ID_SAVE_SNAPSHOT: u16 = 106;
const ID_QUICK_SAVE: u16 = 107;
const ID_QUICK_LOAD: u16 = 108;
const ID_SAVE_QUICK_SAVE_AS: u16 = 109;
const ID_SYSTEM_ROM: u16 = 110; // + the bundled ROM's place in SYSTEM_ROMS
const ID_SYSTEM_ROM_FILE: u16 = 118;
const ID_SYSTEM_ROM_OTHER: u16 = 119;
const ID_NO_EXPANSION_ROM: u16 = 120;
const ID_EXPANSION_ROM_FILE: u16 = 121;
const ID_EXPANSION_ROM_OTHER: u16 = 122;
const ID_INSERT: u16 = 200; // + drive
const ID_EJECT: u16 = 210; // + drive
const ID_WRITE_PROTECT: u16 = 220; // + drive
const ID_CLEAR_RECENT: u16 = 230;
const ID_SAVE_DISK: u16 = 232; // + drive
const ID_NEW_DISK: u16 = 234; // + drive
const ID_DISK_STATUS: u16 = 236; // + drive
const ID_CRT_EFFECTS: u16 = 240;
const ID_FULL_SCREEN: u16 = 241;
const ID_SIZE: u16 = 250; // + half steps: 252 is 1×, 256 is 3×
const ID_CAPTURE_MOUSE: u16 = 270;
const ID_KEYBOARD: u16 = 271;
const ID_RECENT: u16 = 300; // + RECENT_IDS × drive + the image's place in the list
const RECENT_IDS: u16 = 100;

/// The Left Alt shortcuts (§11.3): a key's virtual-key code and Shift.
const SHORTCUTS: [(u16, bool, u16); 17] = [
    (b'O' as u16, false, ID_OPEN_SNAPSHOT),
    (b'S' as u16, true, ID_SAVE_SNAPSHOT),
    (b'S' as u16, false, ID_QUICK_SAVE),
    (b'L' as u16, false, ID_QUICK_LOAD),
    (VK_OEM_COMMA.0, false, ID_SETTINGS),
    (b'E' as u16, false, ID_CRT_EFFECTS),
    (0x0D, false, ID_FULL_SCREEN), // Enter
    (b'M' as u16, false, ID_CAPTURE_MOUSE),
    (b'K' as u16, false, ID_KEYBOARD),
    (b'R' as u16, false, ID_RESET),
    (b'R' as u16, true, ID_POWER_CYCLE),
    (b'P' as u16, false, ID_PAUSE),
    (b'W' as u16, false, ID_WARP),
    (b'1' as u16, false, ID_INSERT),
    (b'2' as u16, false, ID_INSERT + 1),
    (b'1' as u16, true, ID_EJECT),
    (b'2' as u16, true, ID_EJECT + 1),
];

thread_local! {
    // The Disks menu, whose items are titled with the drives' images as it
    // opens, and each drive's submenu.
    static DISKS_MENU: Cell<isize> = const { Cell::new(0) };
    static DRIVE_MENUS: Cell<[isize; DRIVES]> = const { Cell::new([0; DRIVES]) };
    // Submenus built as they open, and the images Recent Images stands for.
    static RECENT_MENUS: Cell<[isize; DRIVES]> = const { Cell::new([0; DRIVES]) };
    static RECENT_IMAGES: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
    static SYSTEM_ROM_MENU: Cell<isize> = const { Cell::new(0) };
    static EXPANSION_ROM_MENU: Cell<isize> = const { Cell::new(0) };
    // The menus of the menu bar in a popup menu, for full screen.
    static MENU_POPUP: Cell<isize> = const { Cell::new(0) };
    // The underlined letters of the menu bar's titles, lowercase.
    static MENU_LETTERS: RefCell<Vec<char>> = const { RefCell::new(Vec::new()) };
}

/// The command for Left Alt with the key `vk`, Shift held or not.
pub fn shortcut(vk: u16, shift: bool) -> Option<u16> {
    SHORTCUTS.iter().find(|&&(key, s, _)| key == vk && s == shift).map(|&(_, _, id)| id)
}

/// True if `c` is the underlined letter of a menu on the menu bar, which
/// Left Alt with that letter opens.
pub fn is_menu_letter(c: char) -> bool {
    let c = c.to_lowercase().next().unwrap_or(c);
    MENU_LETTERS.with_borrow(|letters| letters.contains(&c))
}

/// The letter after the `&` in a menu title, lowercase.
fn underlined_letter(title: &str) -> Option<char> {
    let mut chars = title.chars();

    while let Some(c) = chars.next() {
        if c == '&' {
            match chars.next() {
                Some('&') | None => {}
                Some(letter) => return letter.to_lowercase().next(),
            }
        }
    }

    None
}

fn add_item(menu: HMENU, id: u16, title: &str, shortcut: &str) -> Result<()> {
    let title = menu_title(title);
    let title = if shortcut.is_empty() { title } else { format!("{title}\t{shortcut}") };
    unsafe { AppendMenuW(menu, MF_STRING, id as usize, &HSTRING::from(title)) }
}

fn add_separator(menu: HMENU) -> Result<()> {
    unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, None) }
}

fn add_submenu(parent: HMENU, title: &str) -> Result<HMENU> {
    let menu = unsafe { CreatePopupMenu()? };
    unsafe { AppendMenuW(parent, MF_POPUP, menu.0 as usize, &HSTRING::from(menu_title(title)))? };
    Ok(menu)
}

fn create_menu_bar() -> Result<HMENU> {
    let bar = unsafe { CreateMenu()? };

    let app = add_submenu(bar, "Juku")?;
    add_item(app, ID_ABOUT, "About Juku", "")?;
    add_separator(app)?;
    add_item(app, ID_SETTINGS, "Settings…", "Alt+,")?;
    add_separator(app)?;
    add_item(app, ID_EXIT, "Exit", "Alt+F4")?;

    let machine = add_submenu(bar, "Machine")?;
    add_item(machine, ID_RESET, "Reset", "Alt+R")?;
    add_item(machine, ID_POWER_CYCLE, "Power Cycle", "Alt+Shift+R")?;
    SYSTEM_ROM_MENU.set(add_submenu(machine, "System ROM")?.0 as isize);
    EXPANSION_ROM_MENU.set(add_submenu(machine, "Expansion ROM")?.0 as isize);
    add_separator(machine)?;
    add_item(machine, ID_OPEN_SNAPSHOT, "Open Snapshot…", "Alt+O")?;
    add_item(machine, ID_SAVE_SNAPSHOT, "Save Snapshot…", "Alt+Shift+S")?;
    add_item(machine, ID_QUICK_SAVE, "Quick Save", "Alt+S")?;
    add_item(machine, ID_QUICK_LOAD, "Quick Load", "Alt+L")?;
    add_item(machine, ID_SAVE_QUICK_SAVE_AS, "Save Quick Save As…", "")?;
    add_separator(machine)?;
    add_item(machine, ID_PAUSE, "Pause", "Alt+P")?;
    add_item(machine, ID_WARP, "Warp Speed", "Alt+W")?;

    // One submenu per drive, titled with the image in it as the menu opens.
    let disks = add_submenu(bar, "Disks")?;
    DISKS_MENU.set(disks.0 as isize);
    let mut drive_menus = [0; DRIVES];
    let mut recent_menus = [0; DRIVES];

    for drive in 0..DRIVES {
        let id = drive as u16;
        let key = drive + 1;
        let menu = unsafe { CreatePopupMenu()? };
        unsafe { AppendMenuW(disks, MF_POPUP, menu.0 as usize, &HSTRING::from(drive_title(drive, None)))? };
        add_item(menu, ID_INSERT + id, "Insert…", &format!("Alt+{key}"))?;
        add_item(menu, ID_NEW_DISK + id, "New Disk", "")?;
        recent_menus[drive] = add_submenu(menu, "Recent Images")?.0 as isize;
        add_separator(menu)?;
        add_item(menu, ID_SAVE_DISK + id, "Save Disk…", "")?;
        add_item(menu, ID_EJECT + id, "Eject", &format!("Alt+Shift+{key}"))?;
        add_separator(menu)?;
        add_item(menu, ID_WRITE_PROTECT + id, "Write Protect", "")?;
        drive_menus[drive] = menu.0 as isize;
    }

    DRIVE_MENUS.set(drive_menus);
    RECENT_MENUS.set(recent_menus);

    let input = add_submenu(bar, "Input")?;
    add_item(input, ID_CAPTURE_MOUSE, "Capture Mouse", "Alt+M")?;
    add_separator(input)?;
    add_item(input, ID_KEYBOARD, "Keyboard", "Alt+K")?;

    let view = add_submenu(bar, "View")?;
    add_item(view, ID_CRT_EFFECTS, "CRT Effects", "Alt+E")?;
    add_separator(view)?;

    // 1.5× in English, 1,5× in Estonian: the decimal mark of the user's
    // region, as the Mac's NSNumberFormatter gives it.
    let mut mark = [0u16; 8];
    let len = unsafe { GetLocaleInfoEx(None, LOCALE_SDECIMAL, Some(&mut mark)) } as usize;
    let mark = if len > 1 { String::from_utf16_lossy(&mark[..len - 1]) } else { ".".to_string() };

    for halves in emu_window::MIN_WINDOW_HALVES..=emu_window::MAX_WINDOW_HALVES {
        let title = if halves % 2 == 0 { format!("{}×", halves / 2) } else { format!("{}{mark}5×", halves / 2) };
        unsafe { AppendMenuW(view, MF_STRING, (ID_SIZE as u32 + halves) as usize, &HSTRING::from(menu_text(&title)))? };
    }

    add_separator(view)?;
    add_item(view, ID_FULL_SCREEN, "Enter Full Screen", "Alt+Enter")?;

    // The same menus in a popup, for full screen, which has no menu bar.
    let popup = unsafe { CreatePopupMenu()? };
    let menus = [("Juku", app), ("Machine", machine), ("Disks", disks), ("Input", input), ("View", view)];

    for (title, menu) in menus {
        unsafe { AppendMenuW(popup, MF_POPUP, menu.0 as usize, &HSTRING::from(menu_title(title)))? };
    }

    MENU_POPUP.set(popup.0 as isize);
    MENU_LETTERS.set(menus.iter().filter_map(|(title, _)| underlined_letter(&menu_title(title))).collect());
    Ok(bar)
}

/// In full screen: the menus, opened from the top left corner (F10).
pub fn show_menu_popup(hwnd: HWND) {
    let mut frame = RECT::default();
    let popup = HMENU(MENU_POPUP.get() as *mut _);

    unsafe {
        let _ = GetWindowRect(hwnd, &mut frame);
        let _ = TrackPopupMenuEx(popup, (TPM_LEFTALIGN | TPM_TOPALIGN).0, frame.left, frame.top, hwnd, None);
    }
}

/// Empties a submenu that is built as it opens.
fn clear_menu(menu: HMENU) {
    unsafe {
        while GetMenuItemCount(Some(menu)) > 0 {
            let _ = DeleteMenu(menu, 0, MF_BYPOSITION);
        }
    }
}

fn append(menu: HMENU, id: u16, title: &str, checked: bool) {
    let flags = if checked { MF_STRING | MF_CHECKED } else { MF_STRING };
    let _ = unsafe { AppendMenuW(menu, flags, id as usize, &HSTRING::from(title)) };
}

/// The last file of `key` (SystemROMFile, ExpansionROMFile) as a menu item:
/// its path and its file name as the title.
fn rom_file_item(key: &str) -> Option<(String, String)> {
    let path = settings::string_setting(key)?;
    let name = Path::new(&path).file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
    Some((path, menu_text(&name)))
}

/// Machine → System ROM, built as it opens: one item per bundled ROM, the
/// user's last ROM file and Other…; the ROM in the machine is checked.
fn fill_system_rom_menu(menu: HMENU, installed: &str) {
    clear_menu(menu);

    for (i, rom) in SYSTEM_ROMS.iter().enumerate() {
        append(menu, ID_SYSTEM_ROM + i as u16, &menu_text(&rom.title()), rom.name == installed);
    }

    let _ = add_separator(menu);

    if let Some((path, title)) = rom_file_item(SYSTEM_ROM_FILE) {
        append(menu, ID_SYSTEM_ROM_FILE, &title, path == installed);
    }

    append(menu, ID_SYSTEM_ROM_OTHER, &menu_title("Other…"), false);
}

/// Machine → Expansion ROM: None, the user's last expansion ROM file and
/// Other…; the one plugged in is checked.
fn fill_expansion_rom_menu(menu: HMENU, installed: Option<&str>) {
    clear_menu(menu);
    append(menu, ID_NO_EXPANSION_ROM, &menu_title("None"), installed.is_none());
    let _ = add_separator(menu);

    if let Some((path, title)) = rom_file_item(EXPANSION_ROM_FILE) {
        append(menu, ID_EXPANSION_ROM_FILE, &title, Some(path.as_str()) == installed);
    }

    append(menu, ID_EXPANSION_ROM_OTHER, &menu_title("Other…"), false);
}

/// A drive's item in the Disks menu: its letter, underlined, and the disk's
/// label (`DiskDrives::label`), if there is a disk.
fn drive_title(drive: usize, label: Option<String>) -> String {
    let letter = (b'A' + drive as u8) as char;

    match label {
        Some(label) => format!("&{letter}: {}", menu_text(&label)),
        None => format!("&{letter}:"),
    }
}

/// The status line at the top of a drive's submenu, and the separator under
/// it, there only while there is something to say (`DiskDrives::status`):
/// Windows menus cannot hide an item, so they are put in and taken out.
fn set_disk_status(submenu: HMENU, drive: usize, status: Option<String>) {
    let id = ID_DISK_STATUS + drive as u16;
    let present = unsafe { GetMenuItemID(submenu, 0) } == id as u32;

    unsafe {
        match status {
            Some(text) => {
                let text = HSTRING::from(menu_text(&text));

                if present {
                    let _ = ModifyMenuW(submenu, 0, MF_BYPOSITION | MF_STRING | MF_GRAYED, id as usize, &text);
                } else {
                    let _ = InsertMenuW(submenu, 0, MF_BYPOSITION | MF_STRING | MF_GRAYED, id as usize, &text);
                    let _ = InsertMenuW(submenu, 1, MF_BYPOSITION | MF_SEPARATOR, 0, None);
                }
            }
            None if present => {
                let _ = DeleteMenu(submenu, 0, MF_BYPOSITION);
                let _ = DeleteMenu(submenu, 0, MF_BYPOSITION);
            }
            None => {}
        }
    }
}

/// Disks → A: or B: → Recent Images, built as it opens: an item per image
/// that can still be found, which inserts it in that drive, and Clear Menu.
fn fill_recent_menu(menu: HMENU, drive: usize) {
    clear_menu(menu);
    let images = recent_disks::images();

    for (i, image) in images.iter().enumerate() {
        let name = image.file_name().map_or_else(|| image.display().to_string(), |n| n.to_string_lossy().into_owned());
        append(menu, ID_RECENT + RECENT_IDS * drive as u16 + i as u16, &menu_text(&name), false);
    }

    if !images.is_empty() {
        let _ = add_separator(menu);
    }

    let flags = if images.is_empty() { MF_STRING | MF_GRAYED } else { MF_STRING };
    let _ = unsafe { AppendMenuW(menu, flags, ID_CLEAR_RECENT as usize, &HSTRING::from(menu_title("Clear Menu"))) };
    RECENT_IMAGES.set(images);
}

/// Brings a menu's items up to date as it opens (the Mac's validateMenuItem).
pub fn update_menu(menu: HMENU) {
    if let Some(drive) = RECENT_MENUS.get().iter().position(|&m| m == menu.0 as isize) {
        fill_recent_menu(menu, drive);
        return;
    }

    if menu.0 as isize == DISKS_MENU.get() {
        let states = emu_window::with_emu(|emu| {
            [0, 1].map(|d| emu.with_disks(|disks, m| (drive_title(d, disks.label(m, d)), disks.status(m, d))))
        });

        for (drive, (title, status)) in states.into_iter().flatten().enumerate() {
            let submenu = HMENU(DRIVE_MENUS.get()[drive] as *mut _);
            let _ = unsafe { ModifyMenuW(menu, drive as u32, MF_BYPOSITION | MF_POPUP, submenu.0 as usize, &HSTRING::from(title)) };
            set_disk_status(submenu, drive, status);
        }
    }

    if menu.0 as isize == SYSTEM_ROM_MENU.get() {
        if let Some(installed) = emu_window::with_emu(|emu| emu.roms().system().to_string()) {
            fill_system_rom_menu(menu, &installed);
        }

        return;
    }

    if menu.0 as isize == EXPANSION_ROM_MENU.get() {
        if let Some(installed) = emu_window::with_emu(|emu| emu.roms().expansion().map(str::to_string)) {
            fill_expansion_rom_menu(menu, installed.as_deref());
        }

        return;
    }

    let Some((paused, warp, disks, protected, captured)) = emu_window::with_emu(|emu| {
        let disks = [0, 1].map(|d| DiskDrives::has_disk(emu.machine(), d));
        let protected = [0, 1].map(|d| DiskDrives::is_write_protected(emu.machine(), d));
        (emu.paused(), emu.warp(), disks, protected, emu.mouse_captured())
    }) else {
        return;
    };
    let Some((crt, full_screen, current, fits)) = emu_window::with_emu(|emu| {
        let fits = [2, 3, 4, 5, 6].map(|h| emu.size_fits(h));
        (emu.crt_effects(), emu.full_screen(), emu.window_halves(), fits)
    }) else {
        return;
    };

    let check = |id: u16, on: bool| {
        let flags = MF_BYCOMMAND | if on { MF_CHECKED } else { MF_UNCHECKED };
        unsafe { CheckMenuItem(menu, id as u32, flags.0) };
    };
    let enable = |id: u16, on: bool| {
        let flags = MF_BYCOMMAND | if on { MF_ENABLED } else { MF_GRAYED };
        let _ = unsafe { EnableMenuItem(menu, id as u32, flags) };
    };

    check(ID_PAUSE, paused);
    check(ID_WARP, warp);
    check(ID_CAPTURE_MOUSE, captured);
    check(ID_KEYBOARD, keyboard_window::visible());
    enable(ID_QUICK_LOAD, snapshots::can_quick_load());
    enable(ID_SAVE_QUICK_SAVE_AS, snapshots::can_quick_load());

    for drive in 0..DRIVES {
        let id = drive as u16;
        enable(ID_SAVE_DISK + id, disks[drive]);
        enable(ID_EJECT + id, disks[drive]);
        enable(ID_WRITE_PROTECT + id, disks[drive]);
        check(ID_WRITE_PROTECT + id, disks[drive] && protected[drive]);
    }

    check(ID_CRT_EFFECTS, crt);

    // Checked when the tube is at that size. None in full screen, and none
    // that would not fit on the screen.
    for (i, halves) in (emu_window::MIN_WINDOW_HALVES..=emu_window::MAX_WINDOW_HALVES).enumerate() {
        let id = ID_SIZE + halves as u16;
        check(id, halves == current);
        enable(id, !full_screen && fits[i]);
    }

    // As AppKit does with the Mac's item.
    let title = format!("{}\tAlt+Enter", menu_title(if full_screen { "Exit Full Screen" } else { "Enter Full Screen" }));
    let _ = unsafe { ModifyMenuW(menu, ID_FULL_SCREEN as u32, MF_BYCOMMAND | MF_STRING, ID_FULL_SCREEN as usize, &HSTRING::from(title)) };
}

/// A file from Explorer, the command line or a drop: a snapshot is opened,
/// and a disk image goes in `drive`. Not while a disk is being asked about.
pub fn open_file(owner: HWND, path: &Path, drive: usize) {
    if disks::asking() {
        return;
    }

    if snapshots::is_snapshot(path) {
        snapshots::open_path(owner, path);
    } else {
        disks::insert_file(owner, path, drive);
    }
}

/// The settings changed (the Mac's NSUserDefaultsDidChangeNotification):
/// the machine, the picture and the Settings window follow them. What could
/// not be applied is reported over `owner`.
pub fn settings_changed(owner: HWND) {
    let applied = emu_window::with_emu(|emu| (emu.apply_settings(), emu.apply_debugger()));

    if applied.is_none() {
        log!("settings changed while the window was busy");
    }

    let (alerts, taken) = applied.unwrap_or_default();
    settings_window::refresh();
    show_alerts(owner, alerts);

    if let Some(port) = taken {
        show_debugger_port_alert(owner, port);
    }
}

/// The debug server could not listen on `port` (spec §12.3): reported once
/// for each port, with the Mac's message and note.
pub fn show_debugger_port_alert(owner: HWND, port: u16) {
    let message = localized("The debugger cannot listen on port %d.").replace("%d", &port.to_string());

    let _ = unsafe {
        TaskDialog(
            Some(owner),
            None,
            &HSTRING::from(localized("Window")),
            &HSTRING::from(message),
            &HSTRING::from(localized("Another program may be using it.")),
            TDCBF_OK_BUTTON,
            TD_ERROR_ICON,
            None,
        )
    };
}

/// Machine → System ROM → Other…, and the same item in Settings. The file is
/// checked here, so a wrong one never reaches the setting.
pub fn choose_system_rom_file(owner: HWND) {
    let Some(path) = roms::choose_rom_file(owner, "System ROM") else {
        return;
    };

    let text = path.to_string_lossy();

    if roms::load_system_rom(&text).is_none() {
        show_alerts(owner, vec![Alert::new(roms::NOT_A_ROM, &path)]);
        return;
    }

    settings::set_string_setting(SYSTEM_ROM_FILE, Some(&text));
    emu_window::with_emu(|emu| emu.roms_mut().reload_system(&text));
    settings::set_string_setting(SYSTEM_ROM, Some(&text));
    settings_changed(owner);
}

/// Machine → Expansion ROM → Other…, and the same item in Settings. As for
/// the system ROM, a file that is not an expansion ROM never reaches the
/// setting.
pub fn choose_expansion_rom_file(owner: HWND) {
    let Some(path) = roms::choose_rom_file(owner, "Expansion ROM") else {
        return;
    };

    let text = path.to_string_lossy();

    if let Err(format) = roms::load_expansion_rom(&text) {
        show_alerts(owner, vec![Alert::new(format, &path)]);
        return;
    }

    settings::set_string_setting(EXPANSION_ROM_FILE, Some(&text));
    emu_window::with_emu(|emu| emu.roms_mut().reload_expansion(&text));
    settings::set_string_setting(EXPANSION_ROM, Some(&text));
    settings_changed(owner);
}

/// Juku → About Juku: the name, the version and the copyright, as the Mac's
/// standard About panel shows them.
fn show_about(owner: HWND) {
    let version = localized("Version %@").replace("%@", env!("JUKU_VERSION"));
    let content = HSTRING::from(format!("{version}\n{}", env!("JUKU_COPYRIGHT")));
    let title = HSTRING::from(localized("About Juku"));
    let name = HSTRING::from(localized("Window"));
    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner,
        hInstance: unsafe { GetModuleHandleW(None) }.unwrap_or_default().into(),
        dwCommonButtons: TDCBF_OK_BUTTON,
        pszWindowTitle: PCWSTR(title.as_ptr()),
        // The app's icon, as the Mac's About panel shows it.
        Anonymous1: TASKDIALOGCONFIG_0 { pszMainIcon: APP_ICON },
        pszMainInstruction: PCWSTR(name.as_ptr()),
        pszContent: PCWSTR(content.as_ptr()),
        ..Default::default()
    };
    let _ = unsafe { TaskDialogIndirect(&config, None, None, None) };
}

/// Carries out a menu command or shortcut.
pub fn command(hwnd: HWND, id: u16) {
    let drive = |base: u16| (id - base) as usize;

    match id {
        ID_EXIT => {
            let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
        }
        ID_ABOUT => show_about(hwnd),
        ID_SETTINGS => settings_window::show(hwnd),
        ID_SYSTEM_ROM..ID_SYSTEM_ROM_FILE => {
            if let Some(rom) = SYSTEM_ROMS.get((id - ID_SYSTEM_ROM) as usize) {
                settings::set_string_setting(SYSTEM_ROM, Some(rom.name));
                settings_changed(hwnd);
            }
        }
        ID_SYSTEM_ROM_FILE => {
            if let Some(path) = settings::string_setting(SYSTEM_ROM_FILE) {
                settings::set_string_setting(SYSTEM_ROM, Some(&path));
                settings_changed(hwnd);
            }
        }
        ID_SYSTEM_ROM_OTHER => choose_system_rom_file(hwnd),
        ID_NO_EXPANSION_ROM => {
            settings::set_string_setting(EXPANSION_ROM, None);
            settings_changed(hwnd);
        }
        ID_EXPANSION_ROM_FILE => {
            if let Some(path) = settings::string_setting(EXPANSION_ROM_FILE) {
                settings::set_string_setting(EXPANSION_ROM, Some(&path));
                settings_changed(hwnd);
            }
        }
        ID_EXPANSION_ROM_OTHER => choose_expansion_rom_file(hwnd),
        ID_CAPTURE_MOUSE => {
            // From Settings, say: the mouse is the emulator window's.
            let _ = unsafe { SetFocus(Some(hwnd)) };
            emu_window::with_emu(|emu| emu.toggle_mouse_capture());
        }
        ID_KEYBOARD => keyboard_window::toggle(hwnd),
        // Warm, like the hardware button, and cold.
        ID_RESET => {
            emu_window::with_emu(|emu| emu.machine().reset(false));
        }
        ID_POWER_CYCLE => {
            emu_window::with_emu(|emu| emu.machine().reset(true));
        }
        ID_PAUSE => {
            emu_window::with_emu(|emu| emu.set_paused(!emu.paused()));
        }
        ID_WARP => {
            emu_window::with_emu(|emu| emu.set_warp(!emu.warp()));
        }
        ID_OPEN_SNAPSHOT => snapshots::choose_and_open(hwnd),
        ID_SAVE_SNAPSHOT => snapshots::save(hwnd),
        ID_QUICK_SAVE => snapshots::quick_save(hwnd),
        ID_QUICK_LOAD => {
            if snapshots::can_quick_load() {
                snapshots::quick_load(hwnd);
            }
        }
        ID_SAVE_QUICK_SAVE_AS => {
            if snapshots::can_quick_load() {
                snapshots::save_quick_save_as(hwnd);
            }
        }
        ID_INSERT..ID_EJECT if drive(ID_INSERT) < DRIVES => {
            let drive = drive(ID_INSERT);

            if let Some(path) = disks::choose_disk(hwnd, drive) {
                disks::insert_file(hwnd, &path, drive);
            }
        }
        ID_EJECT..ID_WRITE_PROTECT if drive(ID_EJECT) < DRIVES => {
            disks::eject_disk(hwnd, drive(ID_EJECT));
        }
        ID_WRITE_PROTECT..ID_CLEAR_RECENT if drive(ID_WRITE_PROTECT) < DRIVES => {
            let drive = drive(ID_WRITE_PROTECT);
            emu_window::with_emu(|emu| emu.with_disks(|disks, m| disks.toggle_write_protect(m, drive)));
        }
        ID_CLEAR_RECENT => recent_disks::clear(),
        ID_SAVE_DISK..ID_NEW_DISK if drive(ID_SAVE_DISK) < DRIVES => {
            disks::save_disk(hwnd, drive(ID_SAVE_DISK));
        }
        ID_NEW_DISK..ID_CRT_EFFECTS if drive(ID_NEW_DISK) < DRIVES => disks::insert_new_disk(hwnd, drive(ID_NEW_DISK)),
        ID_CRT_EFFECTS => {
            settings::set_bool_setting(settings::CRT_EFFECTS, !settings::crt_settings().crt_effects);
            settings_changed(hwnd);
        }
        ID_FULL_SCREEN => {
            emu_window::with_emu(|emu| emu.toggle_full_screen());
        }
        _ if (ID_SIZE + emu_window::MIN_WINDOW_HALVES as u16..=ID_SIZE + emu_window::MAX_WINDOW_HALVES as u16).contains(&id) => {
            emu_window::with_emu(|emu| emu.set_window_halves((id - ID_SIZE) as u32));
        }
        _ if (ID_RECENT..ID_RECENT + RECENT_IDS * DRIVES as u16).contains(&id) => {
            let (drive, place) = ((id - ID_RECENT) / RECENT_IDS, (id - ID_RECENT) % RECENT_IDS);
            let image = RECENT_IMAGES.with_borrow(|images| images.get(place as usize).cloned());

            if let Some(image) = image {
                disks::insert_file(hwnd, &image, drive as usize);
            }
        }
        _ => {}
    }
}

// ---- One instance -----------------------------------------------------------------
// A second start hands its file to the first and exits (§11.2), as Finder
// sends a file to the running app on the Mac.

/// `WM_COPYDATA` kinds: a file to open, as UTF-16, and the port of
/// `--debug-server`, a `u16`.
pub const COPYDATA_OPEN: usize = 1;
pub const COPYDATA_DEBUG_SERVER: usize = 2;

/// What the command line holds: a file to open (a disk image or a snapshot
/// from Explorer) and the port of `--debug-server` or
/// `--debug-server=<port>`, 0 without it (spec §12.3). Without a port of its
/// own, `--debug-server` takes the setting's.
fn command_line() -> (Option<PathBuf>, u16) {
    let mut file = None;
    let mut port = 0;

    for arg in std::env::args_os().skip(1) {
        match arg.to_str() {
            Some("--debug-server") => port = settings::debugger_port().unwrap_or(0),
            Some(a) if a.starts_with("--debug-server=") => port = a[15..].parse().unwrap_or(0),
            _ if file.is_none() => file = Some(PathBuf::from(arg)),
            _ => {}
        }
    }

    (file, port)
}

fn copy_data(hwnd: HWND, kind: usize, data: &[u8]) {
    let data = COPYDATASTRUCT { dwData: kind, cbData: data.len() as u32, lpData: data.as_ptr() as *mut _ };
    unsafe { SendMessageW(hwnd, WM_COPYDATA, None, Some(LPARAM(&raw const data as isize))) };
}

/// True if Juku is running already; `file` and the debug server's port, if
/// any, have then been handed to it.
fn hand_over(file: Option<&Path>, debug_port: u16) -> bool {
    // The mutex lives as long as the process: it marks the running Juku.
    let created = unsafe { CreateMutexW(None, false, w!("Local\\io.github.sinijarv-juku.juku-emu")) };

    if created.is_err() || unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
        return false;
    }

    // The other Juku may still be opening its window.
    for _ in 0..30 {
        if let Ok(hwnd) = unsafe { FindWindowW(emu_window::CLASS_NAME, None) } {
            if let Some(file) = file {
                let text: Vec<u8> = file.as_os_str().encode_wide().flat_map(u16::to_ne_bytes).collect();
                copy_data(hwnd, COPYDATA_OPEN, &text);
            }

            if debug_port != 0 {
                copy_data(hwnd, COPYDATA_DEBUG_SERVER, &debug_port.to_ne_bytes());
            }

            unsafe {
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }

                let _ = SetForegroundWindow(hwnd);
            }

            break;
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    true
}

// ---- Main loop ------------------------------------------------------------------

/// A message for the Settings window, which is not a dialog but works like
/// one: Tab moves between the controls and Esc closes the window
/// (`IsDialogMessageW`). Left Alt shortcuts go to the menus, as the Mac's
/// menu bar serves every window, except Alt+W, which closes the window as
/// Cmd-W does on the Mac. True if the message is handled. The Keyboard
/// window never has the keyboard, so it gets no key messages.
fn other_window_message(msg: &MSG) -> bool {
    let Some(window) = settings_window::window() else {
        return false;
    };

    if msg.hwnd != window && !unsafe { IsChild(window, msg.hwnd) }.as_bool() {
        return false;
    }

    if msg.message == WM_SYSKEYDOWN && unsafe { GetKeyState(VK_LMENU.0 as i32) } < 0 {
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;

        match shortcut(msg.wParam.0 as u16, shift) {
            Some(ID_WARP) => {
                let _ = unsafe { PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0)) };
                return true;
            }
            Some(id) => {
                if let Some(main) = emu_window::main_window() {
                    command(main, id);
                }

                return true;
            }
            None => {}
        }
    }

    if settings_window::pane_key(msg) {
        return true;
    }

    unsafe { IsDialogMessageW(window, msg) }.as_bool()
}

/// Hands out the waiting window messages. False once the app is to quit.
fn pump_messages() -> bool {
    let mut msg = MSG::default();

    while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
        if msg.message == WM_QUIT {
            return false;
        }

        if other_window_message(&msg) {
            continue;
        }

        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    true
}

/// Ticks the machine once per display refresh (spec §6.1, §11.2), on a
/// high-resolution timer, and handles window messages in between.
fn run() {
    let timer = unsafe { CreateWaitableTimerExW(None, PCWSTR::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0) }
        .expect("a waitable timer");
    let mut next = emu_window::now();

    loop {
        let wait = next - emu_window::now();
        let due = if wait > 0.0 {
            // Relative due times are negative, in 100 ns units.
            let relative = -((wait * 1e7) as i64).max(1);
            let _ = unsafe { SetWaitableTimer(timer, &relative, 0, None, None, false) };
            let woken = unsafe { MsgWaitForMultipleObjectsEx(Some(&[timer]), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE) };
            woken == WAIT_OBJECT_0
        } else {
            true
        };

        if due {
            let now = emu_window::now();
            emu_window::tick(now);
            let period = emu_window::tick_period();
            next += period;

            if next < now {
                next = now + period;
            }
        }

        if !pump_messages() {
            return;
        }
    }
}

fn main() {
    // The installer's commands (frontends/windows/installer): the file types
    // as Settings → Register File Types writes them, or their removal. No
    // window; the exit code says whether it worked.
    match std::env::args_os().nth(1).as_deref().and_then(|a| a.to_str()) {
        Some("--register") => {
            strings::init(settings::string_setting(settings::LANGUAGE).as_deref());
            std::process::exit(if file_types::register() { 0 } else { 1 });
        }
        Some("--unregister") => std::process::exit(if file_types::unregister() { 0 } else { 1 }),
        _ => {}
    }

    // A file to open: a disk image from Explorer or the command line. And
    // `--debug-server`, as VS Code's extension starts Juku (spec §12.8).
    let (file, debug_port) = command_line();

    if hand_over(file.as_deref(), debug_port) {
        return;
    }

    // The file dialogs need a single-threaded apartment.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    // Settings → Language, or the system's (§11.4).
    strings::init(settings::string_setting(settings::LANGUAGE).as_deref());
    // Light or dark, before the first window (§11.3).
    theme::init();

    let ring = Arc::new(AudioRing::new(audio::SAMPLE_RATE, audio::PRIME));
    let (rom_name, rom, rom_alert) = roms::startup_rom();
    let Some(mut machine) = Machine::with_audio(&rom, Some(ring.clone())) else {
        fatal_error("ROM files are missing or damaged.");
    };

    // The expansion ROM, the key click, the drive sounds and the T3.2 clock,
    // before the machine runs.
    let mut alerts: Vec<Alert> = rom_alert.into_iter().collect();
    let mut roms = InstalledRoms::new(rom_name);
    alerts.extend(roms.apply(&mut machine));
    settings::apply_drive_sounds(&mut machine);
    settings::apply_t32_clock(&mut machine);

    // The disks of the last run go back in before the machine runs, so it
    // boots from A: as before.
    let mut disks = DiskDrives::new();
    alerts.extend(disks.restore_drives(&mut machine));

    // Without an output device the machine runs silently on the host clock.
    let output = audio::AudioOutput::start(ring.clone());
    let menu = create_menu_bar().expect("the menu bar");
    let hwnd = emu_window::create(machine, disks, roms, output.as_ref().map(|_| ring), menu).unwrap_or_else(|e| {
        log!("creating the window: {e}");
        fatal_error("Direct3D 12 is not available.")
    });

    show_alerts(hwnd, alerts);

    // The debug server, as the setting or the command line says.
    if let Some(Some(port)) = emu_window::with_emu(|emu| emu.set_debug_server_argument(debug_port)) {
        show_debugger_port_alert(hwnd, port);
    }

    // An image opened with the app goes into A:, replacing the restored one;
    // a snapshot is opened.
    if let Some(file) = file {
        open_file(hwnd, &file, 0);
    }

    run();
    drop(output);
    emu_window::destroy();
}

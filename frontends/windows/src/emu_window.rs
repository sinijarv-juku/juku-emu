//! The emulator window (frontends/macos/src/EmuView.m): runs the machine,
//! draws the picture, takes the keyboard, the mouse and dropped files, and
//! writes the disks back (spec §6.1, §6.3–§6.5, §11.2, §11.3).
//!
//! The window procedure reaches the window's state through `with_emu`, which
//! borrows it for one message. A message that arrives while it is borrowed (one
//! sent by a call made from a handler) finds it busy and gets the default
//! handling; whatever it would have changed is picked up on the next tick.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use juku::audio::AudioRing;
use juku::keyboard::{KEY_CTRL, KEY_SHIFT};
use juku::machine::CPU_HZ;
use juku::mouse::{BUTTON_LEFT, BUTTON_RIGHT};
use juku::{Frame, Machine};
use juku_dap::{Server, State};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, DEVMODEW, ENUM_CURRENT_SETTINGS, EndPaint, EnumDisplaySettingsW, GetMonitorInfoW,
    MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITORINFO, MONITORINFOEXW, MonitorFromRect, MonitorFromWindow, PAINTSTRUCT,
    ScreenToClient,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, INFINITE, SetWaitableTimer, TIMER_ALL_ACCESS, WaitForSingleObject,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CAPITAL, VK_F10, VK_LBUTTON, VK_LMENU, VK_MENU, VK_RBUTTON, VK_SHIFT};
use windows::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, MOUSE_MOVE_ABSOLUTE, MOUSE_VIRTUAL_DESKTOP, RAWINPUT, RAWINPUTDEVICE, RAWINPUTDEVICE_FLAGS, RAWINPUTHEADER,
    RID_INPUT, RIDEV_REMOVE, RIM_TYPEMOUSE, RegisterRawInputDevices,
};
use windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, Result, w};

use crate::audio::PRIME;
use crate::disks::{Alert, DiskDrives};
use crate::keymap;
use crate::renderer::{CANVAS_HEIGHT, CrtSettings, Renderer, TUBE_ASPECT, tube_rect};
use crate::roms::InstalledRoms;
use crate::settings::{self, DEFAULT_MOUSE_SENSITIVITY, MOUSE_SENSITIVITY, SECTION_WINDOW, WINDOW_FRAME, WINDOW_MAXIMIZED};
use crate::strings::localized;

// Longest stretch of host time emulated in one tick. Longer gaps (window
// dragged, machine asleep) are dropped instead of caught up.
const MAX_TICK_SECONDS: f64 = 0.1;

// Emulation speed follows the audio clock: it is trimmed by up to ±0.5% to
// keep the ring at its primed fill (spec §6.1).
const AUDIO_TARGET_FILL: f64 = PRIME as f64;
const MAX_RATE_TRIM: f64 = 0.005;

// Warp speed: at most this many times real time, using at most this share of
// each tick (and never more than WARP_MAX_BUSY seconds), in slices of
// WARP_SLICE cycles so the time limit is checked often.
const WARP_FACTOR: f64 = 20.0;
const WARP_BUSY_SHARE: f64 = 0.75;
const WARP_MAX_BUSY: f64 = 0.015;
const WARP_SLICE: u64 = 20_000;

// The drive letter in the title stays lit this long after a disk command, so
// short commands are visible and a run of sectors does not flicker.
const DRIVE_LIGHT_HOLD: f64 = 0.3;

// How often the measured warp speed in the title is refreshed.
const WARP_REPORT_INTERVAL: f64 = 0.5;

// How long a note such as "Snapshot Saved" stays in the title, seconds.
const NOTE_SECONDS: f64 = 2.0;

// While the main loop cannot tick (Windows runs a modal loop to move or size
// the window, for an open menu or for a dialog), a thread of its own posts
// this to the window once per display refresh, and the modal loop delivers
// it (spec §11.5).
const WM_APP_TICK: u32 = WM_APP + 2;

// In full screen the cursor hides when the mouse has been still this long.
const CURSOR_TIMER: usize = 3;
const CURSOR_IDLE_MS: u32 = 3000;

// Posted to the window to open a file another Juku start handed over.
const WM_APP_OPEN: u32 = WM_APP + 1;

// Posted to the window to open the snapshot a debugger's launch asks for
// (spec §12.7), after the tick.
const WM_APP_DEBUG_SNAPSHOT: u32 = WM_APP + 3;

// Posted to the window with the port of another Juku start's
// `--debug-server` in wParam.
const WM_APP_DEBUG_SERVER: u32 = WM_APP + 4;

// Drawing (spec §6.1). The picture is drawn only while it can still change on
// the screen: for this long after the Juku picture, the size or a setting
// last changed, which is the time the phosphor afterglow takes to fade out.
const SETTLE_SECONDS: f64 = 0.3;

// While settling, each new Juku frame is drawn, but never two draws closer
// than this (warp speed makes frames faster than anyone can see).
const MIN_DRAW_INTERVAL: f64 = 0.012;

// With no new Juku frames (paused, or the raster stopped), the fade is drawn
// at this interval instead.
const IDLE_DRAW_INTERVAL: f64 = 0.03;

// The View menu's window sizes, 1× to 3× in steps of 0.5×, counted in half
// steps (Main.m).
pub const MIN_WINDOW_HALVES: u32 = 2;
pub const MAX_WINDOW_HALVES: u32 = 6;

pub const CLASS_NAME: PCWSTR = w!("Juku");
const STYLE: WINDOW_STYLE = WS_OVERLAPPEDWINDOW;

/// Host time in seconds.
pub fn now() -> f64 {
    static FREQUENCY: OnceLock<f64> = OnceLock::new();
    let frequency = *FREQUENCY.get_or_init(|| {
        let mut f = 0;
        let _ = unsafe { QueryPerformanceFrequency(&mut f) };
        f as f64
    });
    let mut count = 0;
    let _ = unsafe { QueryPerformanceCounter(&mut count) };
    count as f64 / frequency
}

/// Mouse capture (spec §6.5, §11.2): while captured, the cursor is hidden
/// and held in the middle of the window, and raw input brings the mouse's
/// movement and buttons to the Juku.
#[derive(Default)]
struct MouseCapture {
    captured: bool,
    // Movement not yet sent, in fractions of a Juku count.
    x: f64,
    y: f64,
    buttons: u8,
    // The last position of a mouse that reports absolute positions (remote
    // desktop, virtual machines), in pixels.
    last_absolute: Option<(f64, f64)>,
}

pub struct Emu {
    hwnd: HWND,
    machine: Machine,
    disks: DiskDrives,
    roms: InstalledRoms,
    mouse: MouseCapture,
    /// Juku counts per mouse count (Settings → Mouse Speed).
    mouse_speed: f64,
    /// The ring the audio output plays, for the rate trim; None when there is
    /// no output to follow.
    audio: Option<Arc<AudioRing>>,
    renderer: Renderer,
    crt: CrtSettings,
    menu: HMENU,
    full_screen: bool,
    /// The window's placement before full screen, to go back to.
    windowed: WINDOWPLACEMENT,
    // Full screen: the cursor is hidden after CURSOR_IDLE_MS without
    // movement, and not while a menu is open.
    cursor_hidden: bool,
    menu_open: bool,
    /// Seconds between the main loop's ticks: one display refresh.
    tick_period: f64,
    last_tick: f64,
    cycle_remainder: f64,
    /// The machine stops; the picture stays on the screen.
    paused: bool,
    /// Runs as fast as the host allows, up to WARP_FACTOR×, with audio muted.
    warp: bool,
    /// The debug server (spec §12.3), its port, and what it last said: a
    /// client is connected, the debugger has stopped the machine.
    dap: Option<Server>,
    dap_port: u16,
    debug_state: State,
    /// The port of `--debug-server` on the command line or from another
    /// start of Juku, which wins over the setting; 0 for none.
    dap_arg_port: u16,
    /// The port last reported as taken, so it is reported once; or 0.
    dap_failed_port: u16,
    // Title bar: drive lights and the measured warp speed.
    drive_lit_until: [f64; 2],
    warp_report_start: f64,
    warp_report_cycles: u64,
    warp_speed: f64,
    // A note such as "Snapshot Saved", and until when it shows.
    note: String,
    note_until: f64,
    title: String,
    // Drawing: see tick().
    seen: Box<Frame>,       // the last Juku frame looked at
    drawn_seq: u64,         // the last Juku frame drawn
    settle_until: f64,      // drawing goes on until then
    last_draw: f64,
    phosphor_time: f64,     // when the phosphor textures were last right
    // Keyboard: which of the two Shift and Ctrl keys are down, and Left Alt,
    // which belongs to the menus.
    shift: u8,
    ctrl: u8,
    left_alt: bool,
    altgr_ctrl: bool,       // the Left Ctrl Windows sent with AltGr is down
}

thread_local! {
    static EMU: RefCell<Option<Emu>> = const { RefCell::new(None) };
    // F10 asked for the menu bar (Alt alone does not, §11.3).
    static F10_MENU: Cell<bool> = const { Cell::new(false) };
    // The mouse's last position in the window, as WM_MOUSEMOVE gives it.
    static LAST_MOUSE: Cell<u32> = const { Cell::new(u32::MAX) };
    // Files another Juku start handed over, to open on WM_APP_OPEN.
    static HANDED_OVER: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
    // The snapshot a debugger's launch asks for, to open on
    // WM_APP_DEBUG_SNAPSHOT, and whether it opened, for the server.
    static DEBUG_SNAPSHOT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static DEBUG_SNAPSHOT_OPENED: Cell<Option<bool>> = const { Cell::new(None) };
    // The window, for the other windows to send commands to.
    static MAIN_WINDOW: Cell<isize> = const { Cell::new(0) };
    // The last Alt key down was a shortcut: the character it brings is not a
    // menu's letter.
    static SHORTCUT_TAKEN: Cell<bool> = const { Cell::new(false) };
    // The mouse button that activates the window is not a click in it, as
    // on the Mac (acceptsFirstMouse).
    static ACTIVATING_CLICK: Cell<bool> = const { Cell::new(false) };
}

/// The emulator window, once it is there.
pub fn main_window() -> Option<HWND> {
    let hwnd = MAIN_WINDOW.get();
    (hwnd != 0).then_some(HWND(hwnd as *mut _))
}

/// Runs `f` on the window's state, unless it is not there yet or busy.
pub fn with_emu<R>(f: impl FnOnce(&mut Emu) -> R) -> Option<R> {
    EMU.with(|emu| emu.try_borrow_mut().ok().and_then(|mut emu| emu.as_mut().map(f)))
}

/// Drops the window's state, the machine and the renderer with it.
pub fn destroy() {
    EMU.with(|emu| emu.borrow_mut().take());
}

/// The content size for the View menu's `halves` (2 = 1×) in device
/// independent pixels: at N× the tube is N points per raster line, plus the
/// bezel margin (Main.m, contentSizeForHalves).
fn content_size(halves: u32, margin: f32) -> (f64, f64) {
    let height = (CANVAS_HEIGHT as f64 * halves as f64 * 0.5 * (1.0 + margin as f64)).round();
    ((height * TUBE_ASPECT as f64).round(), height)
}

fn scale(dips: f64, dpi: u32) -> i32 {
    (dips * dpi as f64 / 96.0).round() as i32
}

/// The window, frame and menu bar included, for the View menu's `halves`.
fn window_size(halves: u32, margin: f32, dpi: u32) -> (i32, i32) {
    let (w, h) = content_size(halves, margin);
    let (extra_w, extra_h) = frame_extra(dpi);
    (scale(w, dpi) + extra_w, scale(h, dpi) + extra_h)
}

/// The work area of the monitor the window is on: the screen without the
/// taskbar.
fn work_area(hwnd: HWND) -> RECT {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = unsafe { GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info) };
    info.rcWork
}

/// The frame Juku.ini keeps, if it is still on a monitor.
fn saved_frame() -> Option<RECT> {
    let text = settings::get(SECTION_WINDOW, WINDOW_FRAME)?;
    let n: Vec<i32> = text.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    let rect = match n[..] {
        [left, top, right, bottom] if right > left && bottom > top => RECT { left, top, right, bottom },
        _ => return None,
    };
    (!unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONULL) }.is_invalid()).then_some(rect)
}

/// What the window's frame and menu bar add to the client area, in pixels.
fn frame_extra(dpi: u32) -> (i32, i32) {
    let mut r = RECT::default();
    let _ = unsafe { AdjustWindowRectExForDpi(&mut r, STYLE, true, WINDOW_EX_STYLE(0), dpi) };
    (r.right - r.left, r.bottom - r.top)
}

/// One refresh of the display the window is on.
fn refresh_period(hwnd: HWND) -> f64 {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    let mut mode = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };

    let hz = unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);

        if GetMonitorInfoW(monitor, (&raw mut info).cast::<MONITORINFO>()).as_bool()
            && EnumDisplaySettingsW(PCWSTR(info.szDevice.as_ptr()), ENUM_CURRENT_SETTINGS, &mut mode).as_bool()
        {
            mode.dmDisplayFrequency
        } else {
            0
        }
    };

    // 0 and 1 stand for the hardware's default rate.
    1.0 / if hz > 1 { hz as f64 } else { 60.0 }
}

/// The app's icon at the size of system metrics `cx` and `cy`.
fn icon(instance: windows::Win32::Foundation::HINSTANCE, cx: SYSTEM_METRICS_INDEX, cy: SYSTEM_METRICS_INDEX) -> HICON {
    let (cx, cy) = unsafe { (GetSystemMetrics(cx), GetSystemMetrics(cy)) };
    unsafe { LoadImageW(Some(instance), crate::APP_ICON, IMAGE_ICON, cx, cy, LR_DEFAULTCOLOR) }.map_or(HICON::default(), |h| HICON(h.0))
}

/// Creates the window and the renderer and takes over `machine` and its
/// drives. `audio` is the ring an audio output plays, if there is one.
pub fn create(machine: Machine, disks: DiskDrives, roms: InstalledRoms, audio: Option<Arc<AudioRing>>, menu: HMENU) -> Result<HWND> {
    let instance = unsafe { GetModuleHandleW(None)? };
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(window_proc),
        hInstance: instance.into(),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW)? },
        hIcon: icon(instance.into(), SM_CXICON, SM_CYICON),
        hIconSm: icon(instance.into(), SM_CXSMICON, SM_CYSMICON),
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    unsafe { RegisterClassExW(&class) };

    let title: Vec<u16> = localized("Window").encode_utf16().chain(Some(0)).collect();
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS_NAME,
            PCWSTR(title.as_ptr()),
            STYLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            None,
            Some(menu),
            Some(instance.into()),
            None,
        )?
    };
    crate::theme::apply(hwnd);
    MAIN_WINDOW.set(hwnd.0 as isize);

    // It opens with the frame it had when last used (the Mac's frame
    // autosave), unless that is on no monitor now. The first time, it opens
    // centred at the largest of the View menu's sizes that fits on the
    // screen, 3× if it can.
    let crt = settings::crt_settings();

    if let Some(frame) = saved_frame() {
        let placement = WINDOWPLACEMENT {
            length: size_of::<WINDOWPLACEMENT>() as u32,
            showCmd: SW_HIDE.0 as u32,
            rcNormalPosition: frame,
            ..Default::default()
        };
        unsafe { SetWindowPlacement(hwnd, &placement)? };
    } else {
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let work = work_area(hwnd);
        let fits = |(w, h): (i32, i32)| w <= work.right - work.left && h <= work.bottom - work.top;
        let (w, h) = (MIN_WINDOW_HALVES + 1..=MAX_WINDOW_HALVES)
            .rev()
            .map(|halves| window_size(halves, crt.tube_margin(false), dpi))
            .find(|&size| fits(size))
            .unwrap_or_else(|| window_size(MIN_WINDOW_HALVES, crt.tube_margin(false), dpi));
        let x = work.left + (work.right - work.left - w) / 2;
        let y = work.top + (work.bottom - work.top - h) / 2;
        unsafe { SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE)? };
    }

    let mut client = RECT::default();
    unsafe { GetClientRect(hwnd, &mut client)? };
    let renderer = Renderer::new(hwnd, client.right as u32, client.bottom as u32)?;
    let now = now();

    EMU.with(|emu| {
        *emu.borrow_mut() = Some(Emu {
            hwnd,
            machine,
            disks,
            roms,
            mouse: MouseCapture::default(),
            mouse_speed: settings::f64_setting(MOUSE_SENSITIVITY, DEFAULT_MOUSE_SENSITIVITY),
            audio,
            renderer,
            crt,
            menu,
            full_screen: false,
            windowed: WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() },
            cursor_hidden: false,
            menu_open: false,
            tick_period: refresh_period(hwnd),
            last_tick: 0.0,
            cycle_remainder: 0.0,
            paused: false,
            warp: false,
            dap: None,
            dap_port: 0,
            debug_state: State { connected: false, stopped: false },
            dap_arg_port: 0,
            dap_failed_port: 0,
            drive_lit_until: [0.0; 2],
            warp_report_start: now,
            warp_report_cycles: 0,
            warp_speed: 0.0,
            note: String::new(),
            note_until: 0.0,
            title: localized("Window").to_string(),
            seen: Frame::new_boxed(),
            drawn_seq: 0,
            settle_until: now + SETTLE_SECONDS,
            last_draw: 0.0,
            phosphor_time: now,
            shift: 0,
            ctrl: 0,
            left_alt: false,
            altgr_ctrl: false,
        })
    });

    start_pacer(hwnd);

    unsafe {
        DragAcceptFiles(hwnd, true);
        let maximized = settings::get(SECTION_WINDOW, WINDOW_MAXIMIZED).is_some_and(|v| v == "1");
        let _ = ShowWindow(hwnd, if maximized { SW_SHOWMAXIMIZED } else { SW_SHOW });
    }

    Ok(hwnd)
}

impl Emu {
    pub fn machine(&mut self) -> &mut Machine {
        &mut self.machine
    }

    pub fn disks(&self) -> &DiskDrives {
        &self.disks
    }

    /// Runs `f` on the drives and the machine they are in.
    pub fn with_disks<R>(&mut self, f: impl FnOnce(&mut DiskDrives, &mut Machine) -> R) -> R {
        f(&mut self.disks, &mut self.machine)
    }

    pub fn tick_period(&self) -> f64 {
        self.tick_period
    }

    /// Paused, or stopped in the debugger: a stop by the debugger is the
    /// app's Pause (spec §12.3).
    pub fn paused(&self) -> bool {
        self.paused || self.debug_state.stopped
    }

    pub fn crt_effects(&self) -> bool {
        self.crt.crt_effects
    }

    pub fn roms(&self) -> &InstalledRoms {
        &self.roms
    }

    pub fn roms_mut(&mut self) -> &mut InstalledRoms {
        &mut self.roms
    }

    /// The settings changed (the Mac's applySettings and the view's
    /// NSUserDefaultsDidChangeNotification): the ROMs, the key click, the
    /// drive sounds, the T3.2 clock, the picture and the mouse speed follow
    /// them. Returns what could not be applied.
    pub fn apply_settings(&mut self) -> Vec<Alert> {
        let alerts = self.roms.apply(&mut self.machine);
        settings::apply_drive_sounds(&mut self.machine);
        settings::apply_t32_clock(&mut self.machine);
        self.crt = settings::crt_settings();
        self.mouse_speed = settings::f64_setting(MOUSE_SENSITIVITY, DEFAULT_MOUSE_SENSITIVITY);
        self.set_needs_redraw();
        alerts
    }

    /// A snapshot replaced the machine's state, and its time may have gone
    /// back: keys are released, the warp measurement and the cycle remainder
    /// start again, and the picture is drawn again.
    pub fn machine_restored(&mut self) {
        self.release_keys();
        self.cycle_remainder = 0.0;
        self.warp_report_start = now();
        self.warp_report_cycles = self.machine.cycles();
        self.set_needs_redraw();
    }

    /// Shows `note` in the title for a moment.
    pub fn show_note(&mut self, note: &str) {
        self.note = note.to_string();
        self.note_until = now() + NOTE_SECONDS;
        self.update_title(now());
    }

    /// The screen as the window shows it, for a snapshot's thumbnail: BGRA
    /// pixels, THUMBNAIL_WIDTH × THUMBNAIL_HEIGHT.
    pub fn thumbnail(&mut self) -> Option<Vec<u8>> {
        self.renderer.thumbnail(self.machine.frame(), &self.crt)
    }

    pub fn mouse_captured(&self) -> bool {
        self.mouse.captured
    }

    /// Input → Capture Mouse (Alt+M).
    pub fn toggle_mouse_capture(&mut self) {
        if self.mouse.captured {
            self.release_mouse();
        } else {
            self.capture_mouse();
        }
    }

    /// Hides the cursor, holds it in the middle of the window and takes the
    /// mouse's raw input.
    fn capture_mouse(&mut self) {
        if self.mouse.captured {
            return;
        }

        let device = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 2, dwFlags: RAWINPUTDEVICE_FLAGS(0), hwndTarget: self.hwnd };

        if let Err(e) = unsafe { RegisterRawInputDevices(&[device], size_of::<RAWINPUTDEVICE>() as u32) } {
            log!("raw mouse input: {e}");
            return;
        }

        self.mouse = MouseCapture { captured: true, ..Default::default() };
        self.park_cursor();
        unsafe { SetCursor(None) };
        let _ = unsafe { KillTimer(Some(self.hwnd), CURSOR_TIMER) };
        self.update_title(now());
    }

    /// Gives the mouse back to Windows: Alt+M, focus lost, a menu opened.
    fn release_mouse(&mut self) {
        if !self.mouse.captured {
            return;
        }

        let device = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 2, dwFlags: RIDEV_REMOVE, hwndTarget: HWND::default() };

        unsafe {
            let _ = RegisterRawInputDevices(&[device], size_of::<RAWINPUTDEVICE>() as u32);
            let _ = ClipCursor(None);
            SetCursor(LoadCursorW(None, IDC_ARROW).ok());
        }

        self.mouse = MouseCapture::default();
        self.machine.mouse_input(0, 0, 0);
        self.cursor_hidden = false;
        self.cursor_moved();
        self.update_title(now());
    }

    /// Holds the hidden cursor in the middle of the window, where it shows
    /// again on release. Again after the window moved or changed size.
    fn park_cursor(&self) {
        let mut client = RECT::default();
        let _ = unsafe { GetClientRect(self.hwnd, &mut client) };
        let mut middle = POINT { x: client.right / 2, y: client.bottom / 2 };

        unsafe {
            let _ = ClientToScreen(self.hwnd, &mut middle);
            let _ = SetCursorPos(middle.x, middle.y);
            let _ = ClipCursor(Some(&RECT { left: middle.x, top: middle.y, right: middle.x + 1, bottom: middle.y + 1 }));
        }
    }

    /// Raw mouse input while captured: movement scaled by Mouse Speed, the
    /// remainder kept as a fraction, and the buttons, left to left and right
    /// to right.
    fn raw_input(&mut self, input: HRAWINPUT) {
        let mut raw = RAWINPUT::default();
        let mut size = size_of::<RAWINPUT>() as u32;
        let read = unsafe { GetRawInputData(input, RID_INPUT, Some((&raw mut raw).cast()), &mut size, size_of::<RAWINPUTHEADER>() as u32) };

        if !self.mouse.captured || read == u32::MAX || raw.header.dwType != RIM_TYPEMOUSE.0 {
            return;
        }

        let mouse = unsafe { raw.data.mouse };
        let (dx, dy) = if mouse.usFlags.0 & MOUSE_MOVE_ABSOLUTE.0 != 0 {
            // 0–65535 across the screen, or across all of them.
            let (w, h) = if mouse.usFlags.0 & MOUSE_VIRTUAL_DESKTOP.0 != 0 {
                unsafe { (GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN)) }
            } else {
                unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
            };
            let at = (mouse.lLastX as f64 * w as f64 / 65535.0, mouse.lLastY as f64 * h as f64 / 65535.0);
            let last = self.mouse.last_absolute.replace(at).unwrap_or(at);
            (at.0 - last.0, at.1 - last.1)
        } else {
            (mouse.lLastX as f64, mouse.lLastY as f64)
        };

        let flags = unsafe { mouse.Anonymous.Anonymous.usButtonFlags } as u32;
        let mut buttons = self.mouse.buttons;

        for (down, up, button) in [
            (RI_MOUSE_LEFT_BUTTON_DOWN, RI_MOUSE_LEFT_BUTTON_UP, BUTTON_LEFT),
            (RI_MOUSE_RIGHT_BUTTON_DOWN, RI_MOUSE_RIGHT_BUTTON_UP, BUTTON_RIGHT),
        ] {
            if flags & down != 0 {
                buttons |= button;
            }

            if flags & up != 0 {
                buttons &= !button;
            }
        }

        // Windows' y grows downwards, as the Juku's does.
        self.mouse.x += dx * self.mouse_speed;
        self.mouse.y += dy * self.mouse_speed;
        let (x, y) = (self.mouse.x as i32, self.mouse.y as i32);
        self.mouse.x -= x as f64;
        self.mouse.y -= y as f64;

        if x != 0 || y != 0 || buttons != self.mouse.buttons {
            self.mouse.buttons = buttons;
            self.machine.mouse_input(x, y, buttons);
        }
    }

    pub fn full_screen(&self) -> bool {
        self.full_screen
    }

    /// Full screen is a borderless window over the whole monitor, without the
    /// menu bar; F10 opens the menus at its top left corner (§11.5).
    pub fn toggle_full_screen(&mut self) {
        let hwnd = self.hwnd;

        unsafe {
            if self.full_screen {
                SetWindowLongPtrW(hwnd, GWL_STYLE, (STYLE | WS_VISIBLE).0 as isize);
                let _ = SetMenu(hwnd, Some(self.menu));
                let _ = SetWindowPlacement(hwnd, &self.windowed);
                let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_FRAMECHANGED);
                self.full_screen = false;
                self.show_cursor();
            } else {
                let _ = GetWindowPlacement(hwnd, &mut self.windowed);
                let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
                let _ = GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info);
                let m = info.rcMonitor;
                let _ = SetMenu(hwnd, None);
                SetWindowLongPtrW(hwnd, GWL_STYLE, (WS_POPUP | WS_VISIBLE).0 as isize);
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOP),
                    m.left,
                    m.top,
                    m.right - m.left,
                    m.bottom - m.top,
                    SWP_FRAMECHANGED | SWP_NOOWNERZORDER,
                );
                self.full_screen = true;
                self.hide_idle_cursor();
            }
        }

        self.set_needs_redraw();
        self.save_placement();
    }

    /// The window's frame and maximized state go to Juku.ini; in full screen,
    /// those it had before.
    pub fn save_placement(&self) {
        let mut placement = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };

        if self.full_screen {
            placement = self.windowed;
        } else if unsafe { GetWindowPlacement(self.hwnd, &mut placement) }.is_err() {
            return;
        }

        let r = placement.rcNormalPosition;
        let frame = format!("{},{},{},{}", r.left, r.top, r.right, r.bottom);
        let maximized = placement.showCmd == SW_SHOWMAXIMIZED.0 as u32;
        settings::set(SECTION_WINDOW, WINDOW_FRAME, Some(&frame));
        settings::set(SECTION_WINDOW, WINDOW_MAXIMIZED, Some(if maximized { "1" } else { "0" }));
    }

    /// The View size, in half steps, the tube is drawn at in the window
    /// (within a point), or 0 if it is at none of them, or in full screen.
    pub fn window_halves(&self) -> u32 {
        let mut client = RECT::default();

        if self.full_screen || unsafe { GetClientRect(self.hwnd, &mut client) }.is_err() {
            return 0;
        }

        let dpi = unsafe { GetDpiForWindow(self.hwnd) } as f32 / 96.0;
        let tube = tube_rect(client.right as f32, client.bottom as f32, self.crt.tube_margin(false));
        let half = CANVAS_HEIGHT as f32 * 0.5 * dpi;
        let halves = (tube[3] / half).round() as u32;
        let exact = (tube[3] - halves as f32 * half).abs() <= dpi;
        if (MIN_WINDOW_HALVES..=MAX_WINDOW_HALVES).contains(&halves) && exact { halves } else { 0 }
    }

    /// True if the window at View size `halves` fits on its monitor.
    pub fn size_fits(&self, halves: u32) -> bool {
        let (w, h) = window_size(halves, self.crt.tube_margin(false), unsafe { GetDpiForWindow(self.hwnd) });
        let work = work_area(self.hwnd);
        w <= work.right - work.left && h <= work.bottom - work.top
    }

    /// Sizes the window for View size `halves`, keeping its title bar in
    /// place, moved up or across only to stay on the screen.
    pub fn set_window_halves(&mut self, halves: u32) {
        if self.full_screen {
            return;
        }

        let hwnd = self.hwnd;
        let (w, h) = window_size(halves, self.crt.tube_margin(false), unsafe { GetDpiForWindow(hwnd) });
        let work = work_area(hwnd);
        let mut frame = RECT::default();

        unsafe {
            if IsZoomed(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }

            let _ = GetWindowRect(hwnd, &mut frame);
        }

        let x = frame.left.min(work.right - w).max(work.left);
        let y = frame.top.min(work.bottom - h).max(work.top);
        let _ = unsafe { SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE) };
        self.set_needs_redraw();
    }

    fn show_cursor(&mut self) {
        let _ = unsafe { KillTimer(Some(self.hwnd), CURSOR_TIMER) };

        if self.cursor_hidden {
            self.cursor_hidden = false;
            unsafe { SetCursor(LoadCursorW(None, IDC_ARROW).ok()) };
        }
    }

    /// The mouse moved: the cursor shows, and in full screen the countdown
    /// to hiding it starts again. A captured mouse hides it anyway.
    fn cursor_moved(&mut self) {
        if self.mouse.captured {
            return;
        }

        self.show_cursor();

        if self.full_screen {
            unsafe { SetTimer(Some(self.hwnd), CURSOR_TIMER, CURSOR_IDLE_MS, None) };
        }
    }

    /// Hides the cursor in full screen, unless a menu is open, a button is
    /// held, the cursor is on another monitor or the window is not active.
    fn hide_idle_cursor(&mut self) {
        let _ = unsafe { KillTimer(Some(self.hwnd), CURSOR_TIMER) };
        let mut cursor = POINT::default();
        let mut frame = RECT::default();

        let held = unsafe { GetKeyState(VK_LBUTTON.0 as i32) < 0 || GetKeyState(VK_RBUTTON.0 as i32) < 0 };
        let inside = unsafe { GetCursorPos(&mut cursor).is_ok() && GetWindowRect(self.hwnd, &mut frame).is_ok() }
            && (frame.left..frame.right).contains(&cursor.x)
            && (frame.top..frame.bottom).contains(&cursor.y);
        let active = unsafe { GetForegroundWindow() } == self.hwnd;

        if self.full_screen && !self.menu_open && !held && inside && active {
            self.cursor_hidden = true;
            unsafe { SetCursor(None) };
        }
    }

    /// With a client connected, Pause stops the machine in the debugger, and
    /// the client is told.
    pub fn set_paused(&mut self, paused: bool) {
        match &mut self.dap {
            Some(dap) if self.debug_state.connected => self.debug_state = dap.set_paused(&mut self.machine, paused),
            _ => self.paused = paused,
        }

        self.set_needs_redraw();
        self.update_title(now());
    }

    /// Starts, moves or stops the debug server as the setting and the
    /// command line say (spec §12.3). Returns a port that could not be had,
    /// the first time it fails, to be reported.
    pub fn apply_debugger(&mut self) -> Option<u16> {
        let setting = settings::bool_setting(settings::ACCEPT_DEBUGGER, false).then(settings::debugger_port).flatten();
        let port = if self.dap_arg_port != 0 { self.dap_arg_port } else { setting.unwrap_or(0) };

        if self.dap.is_some() && port == self.dap_port {
            return None;
        }

        // The session ends, and the machine runs on without the debugger.
        if let Some(mut dap) = self.dap.take() {
            dap.end(&mut self.machine);
        }

        let mut failed = None;

        if port == 0 {
            self.dap_failed_port = 0;
        } else {
            match Server::start(port) {
                Ok(dap) => {
                    self.dap = Some(dap);
                    self.dap_failed_port = 0;
                }
                Err(e) => {
                    log!("debug server on port {port}: {e}");

                    if port != self.dap_failed_port {
                        self.dap_failed_port = port;
                        failed = Some(port);
                    }
                }
            }
        }

        self.dap_port = if self.dap.is_some() { port } else { 0 };
        self.debug_state = State { connected: false, stopped: false };
        self.set_needs_redraw();
        self.update_title(now());
        failed
    }

    /// `--debug-server` from another start of Juku: the server listens on
    /// `port` from now on, whatever the setting says. Returns a port that
    /// could not be had, as `apply_debugger` does.
    pub fn set_debug_server_argument(&mut self, port: u16) -> Option<u16> {
        self.dap_arg_port = port;
        self.apply_debugger()
    }

    /// Runs the machine. With the debug server on, the server runs it, so
    /// that a breakpoint it goes on from (a log point, a condition that does
    /// not hold) costs no time, and a stop is reported at once.
    fn run(&mut self, cycles: u64) -> u64 {
        let Some(dap) = &mut self.dap else {
            return self.machine.run_cycles(cycles);
        };

        let ran = dap.run(&mut self.machine, cycles);
        let state = dap.poll(&mut self.machine);
        self.debug_state_changed(state);
        ran
    }

    /// Lets the debug server take a client and handle its messages, and
    /// takes in the state. A client that connects to a paused machine finds
    /// it stopped in the debugger. Called on every tick, so a request waits
    /// at most one.
    fn poll_debugger(&mut self) {
        let Some(dap) = &mut self.dap else {
            DEBUG_SNAPSHOT_OPENED.take();
            return;
        };

        let mut state = dap.poll(&mut self.machine);

        if let Some(opened) = DEBUG_SNAPSHOT_OPENED.take() {
            state = dap.snapshot_opened(&mut self.machine, opened);
        }

        if state.connected && !self.debug_state.connected && self.paused {
            self.paused = false;
            state = dap.set_paused(&mut self.machine, true);
        }

        self.debug_state_changed(state);
    }

    /// Takes in what the server said after it ran: a snapshot that a launch
    /// asks for (spec §12.7) is opened after this tick, as the user would
    /// open it, and the title follows the state.
    fn debug_state_changed(&mut self, state: State) {
        if let Some(path) = self.dap.as_mut().and_then(Server::take_request) {
            DEBUG_SNAPSHOT.set(Some(path));
            let _ = unsafe { PostMessageW(Some(self.hwnd), WM_APP_DEBUG_SNAPSHOT, WPARAM(0), LPARAM(0)) };
        }

        if state != self.debug_state {
            self.debug_state = state;
            self.set_needs_redraw();
            self.update_title(now());
        }
    }

    pub fn warp(&self) -> bool {
        self.warp
    }

    pub fn set_warp(&mut self, warp: bool) {
        self.warp = warp;
        // Muted, the audio ring drains; afterwards it re-primes at the normal
        // latency instead of playing a backlog.
        self.machine.set_audio_muted(warp);
        self.warp_report_start = now();
        self.warp_report_cycles = self.machine.cycles();
        self.warp_speed = 0.0;
        self.update_title(now());
    }

    /// Title: "Juku", plus a lit letter per busy drive ("Juku — A●"), then
    /// what the Mac shows as the subtitle: the paused or warp state (§11.2).
    fn update_title(&mut self, now: f64) {
        let mut lights = Vec::new();

        for (drive, light) in ["A●", "B●"].into_iter().enumerate() {
            if self.machine.drive_active(drive) {
                self.drive_lit_until[drive] = now + DRIVE_LIGHT_HOLD;
            }

            if now < self.drive_lit_until[drive] {
                lights.push(light);
            }
        }

        let mut title = localized("Window").to_string();

        if !lights.is_empty() {
            title = format!("{title} — {}", lights.join(" "));
        }

        let subtitle = if self.mouse.captured {
            localized("Press Alt+M to release the mouse").to_string()
        } else if now < self.note_until {
            self.note.clone()
        } else {
            // "Debugging · Paused", as the Mac's subtitle.
            let mut states = Vec::new();

            if self.debug_state.connected {
                states.push(localized("Debugging").to_string());
            }

            if self.paused() {
                states.push(localized("Paused").to_string());
            } else if self.warp && self.warp_speed > 0.0 {
                states.push(localized("Warp Speed (%.0f×)").replace("%.0f", &format!("{:.0}", self.warp_speed)));
            } else if self.warp {
                states.push(localized("Warp Speed").to_string());
            }

            states.join(" · ")
        };

        if !subtitle.is_empty() {
            title = format!("{title} — {subtitle}");
        }

        if title != self.title {
            let text: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
            let _ = unsafe { SetWindowTextW(self.hwnd, PCWSTR(text.as_ptr())) };
            self.title = title;
        }
    }

    /// The picture has to be drawn again: the size, a setting or the pause
    /// state changed, or the window came back into view.
    pub fn set_needs_redraw(&mut self) {
        self.settle_until = now() + SETTLE_SECONDS;
    }

    /// Releases every key, e.g. when the window loses the keyboard.
    fn release_keys(&mut self) {
        self.machine.release_all_keys();
        self.shift = 0;
        self.ctrl = 0;
        self.altgr_ctrl = false;
        self.left_alt = false;
    }

    /// Follows the client area's size; true if it changed.
    fn sync_size(&mut self) -> bool {
        let mut client = RECT::default();

        if unsafe { GetClientRect(self.hwnd, &mut client) }.is_err() || client.right <= 0 || client.bottom <= 0 {
            return false;
        }

        let size = (client.right as u32, client.bottom as u32);

        if size == self.renderer.size() {
            return false;
        }

        self.renderer.resize(size.0, size.1);
        self.set_needs_redraw();
        self.last_draw = 0.0; // drawn on the next tick, so a resize follows the mouse
        true
    }

    /// Warp: runs slices until WARP_FACTOR × the elapsed time is emulated or
    /// the tick's share of host time is used up.
    fn run_warp(&mut self, elapsed: f64) {
        let start = now();
        let deadline = start + (elapsed * WARP_BUSY_SHARE).min(WARP_MAX_BUSY);
        let target = (elapsed * CPU_HZ as f64 * WARP_FACTOR) as u64;
        let mut done = 0;

        while done < target {
            done += self.run(WARP_SLICE.min(target - done));

            if now() >= deadline {
                break;
            }
        }

        let span = start - self.warp_report_start;

        if span >= WARP_REPORT_INTERVAL {
            let cycles = self.machine.cycles();
            self.warp_speed = (cycles - self.warp_report_cycles) as f64 / (span * CPU_HZ as f64);
            self.warp_report_start = start;
            self.warp_report_cycles = cycles;
        }
    }

    /// Runs the machine for the host time elapsed since the last tick.
    fn run_until(&mut self, now: f64) {
        let elapsed = if self.last_tick > 0.0 { now - self.last_tick } else { 0.0 };
        self.last_tick = now;
        let elapsed = elapsed.clamp(0.0, MAX_TICK_SECONDS);
        self.poll_debugger();

        if self.paused() {
            self.cycle_remainder = 0.0;
        } else {
            // The guest Caps Lock follows the host's (spec §6.4). Windows
            // updates a window's view of it only while the window has the
            // keyboard.
            let caps = unsafe { GetKeyState(VK_CAPITAL.0 as i32) } & 1 != 0;
            self.machine.set_caps_lock(caps);

            if self.warp {
                self.run_warp(elapsed);
            } else {
                let mut trim = 1.0;

                if let Some(ring) = &self.audio {
                    let error = (AUDIO_TARGET_FILL - ring.fill() as f64) / AUDIO_TARGET_FILL;
                    trim += MAX_RATE_TRIM * error.clamp(-1.0, 1.0);
                }

                let cycles = elapsed * CPU_HZ as f64 * trim + self.cycle_remainder;
                let whole = cycles as u64;
                self.cycle_remainder = cycles - whole as f64;
                self.run(whole);
            }
        }

        self.update_title(now);
    }

    /// True if `frame` shows something else than the last frame looked at.
    fn picture_changed(seen: &mut Frame, frame: &Frame) -> bool {
        let header = |f: &Frame| (f.active_x, f.active_y, f.active_w, f.active_h);
        let changed = header(seen) != header(frame) || seen.data != frame.data;

        if changed {
            (seen.active_x, seen.active_y, seen.active_w, seen.active_h) = header(frame);
            seen.data.copy_from_slice(&frame.data);
        }

        changed
    }

    /// Runs the machine and draws the picture if there is anything new to
    /// show (spec §6.1): only while the window is shown, the picture is
    /// still settling, and there is a Juku frame not drawn yet or none has
    /// come for a while.
    pub fn tick(&mut self, now: f64) {
        self.run_until(now);

        if unsafe { IsIconic(self.hwnd) }.as_bool() {
            return; // what is on the screen when it comes back is drawn then
        }

        self.sync_size();
        let frame = self.machine.frame();

        if frame.seq != self.seen.seq {
            self.seen.seq = frame.seq;

            if Self::picture_changed(&mut self.seen, frame) {
                self.settle_until = now + SETTLE_SECONDS;
            }
        }

        if now >= self.settle_until {
            // Settled: the textures hold the picture as the tube shows it now.
            self.phosphor_time = now;
            return;
        }

        let fresh = frame.seq != self.drawn_seq;

        if now - self.last_draw < if fresh { MIN_DRAW_INTERVAL } else { IDLE_DRAW_INTERVAL } {
            return;
        }

        if self.renderer.draw(frame, now - self.phosphor_time, &self.crt, self.full_screen) {
            self.drawn_seq = frame.seq;
            self.last_draw = now;
            self.phosphor_time = now;
        }
    }

    /// A key went down or up. Keys go to the Juku by position (keymap.rs).
    /// Left Alt combinations belong to the menus, and host auto-repeat is
    /// ignored because the BIOS repeats keys itself. A key going up always
    /// reaches the Juku, so none is left held.
    fn key(&mut self, scan: u16, down: bool, repeat: bool) {
        let bit = |bits: u8, mask: u8| if down { bits | mask } else { bits & !mask };

        match scan {
            keymap::LEFT_SHIFT | keymap::RIGHT_SHIFT => {
                self.shift = bit(self.shift, if scan == keymap::LEFT_SHIFT { 1 } else { 2 });
                self.machine.set_key(KEY_SHIFT, self.shift != 0);
            }
            keymap::LEFT_CTRL | keymap::RIGHT_CTRL => {
                self.ctrl = bit(self.ctrl, if scan == keymap::LEFT_CTRL { 1 } else { 2 });
                self.machine.set_key(KEY_CTRL, self.ctrl != 0);
            }
            keymap::LEFT_ALT => self.left_alt = down,
            keymap::RIGHT_ALT => {
                if !repeat {
                    self.machine.set_key(keymap::SEMICOLON_PLUS, down);
                }
            }
            // The Shift Windows adds around some keys with Num Lock on.
            s if s == keymap::EXT | keymap::LEFT_SHIFT || s == keymap::EXT | keymap::RIGHT_SHIFT => {}
            _ => {
                if let Some(key) = keymap::juku_key(scan) {
                    if !down {
                        self.machine.set_key(key, false);
                    } else if !repeat && !self.left_alt {
                        self.machine.set_key(key, true);
                    }
                }
            }
        }
    }
}

/// For `WM_NCLBUTTONDOWN` in any of the app's windows, before
/// `DefWindowProc`. On a press on the title bar Windows waits for the mouse
/// to move, up to half a second and handing out no messages to any window of
/// the thread, before it starts the modal loop that moves the window, and
/// the machine and the picture stop meanwhile. A mouse message waiting in
/// the queue starts the loop at once, and the loop hands out the pacer's
/// ticks.
pub fn title_bar_pressed(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    if wparam.0 as u32 != HTCAPTION {
        return;
    }

    let mut point = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
    let _ = unsafe { ScreenToClient(hwnd, &mut point) };
    let position = (point.x as u16 as u32 | (point.y as u16 as u32) << 16) as isize;
    let _ = unsafe { PostMessageW(Some(hwnd), WM_MOUSEMOVE, WPARAM(0), LPARAM(position)) };
}

/// Ticks the machine, if the window is there and not busy. Called by the
/// main loop.
pub fn tick(now: f64) {
    MAIN_LOOP_TICK.store(now.to_bits(), Ordering::Relaxed);
    with_emu(|emu| emu.tick(now));
}

// The pacer's view of the main loop and the window: the time of the main
// loop's last tick, the tick period (f64 bits), the window to post to (0 once
// it is gone), and whether a posted tick is still waiting in the queue.
static MAIN_LOOP_TICK: AtomicU64 = AtomicU64::new(0);
static PACER_PERIOD: AtomicU64 = AtomicU64::new(0);
static PACER_WINDOW: AtomicIsize = AtomicIsize::new(0);
static TICK_POSTED: AtomicBool = AtomicBool::new(false);

/// Starts the thread that keeps the machine ticking at the display's rate
/// while a modal loop holds up the main loop. It posts a tick only once the
/// main loop has missed two, and never a second before the first is taken,
/// so a busy window does not get a queue of them.
fn start_pacer(hwnd: HWND) {
    PACER_WINDOW.store(hwnd.0 as isize, Ordering::Relaxed);
    PACER_PERIOD.store(tick_period().to_bits(), Ordering::Relaxed);

    let started = std::thread::Builder::new().name("pacer".into()).spawn(|| {
        let Ok(timer) = (unsafe { CreateWaitableTimerExW(None, PCWSTR::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0) })
        else {
            log!("the pacer has no timer");
            return;
        };
        let mut next = now();

        loop {
            let period = f64::from_bits(PACER_PERIOD.load(Ordering::Relaxed));
            next += period;
            let wait = next - now();

            if wait > 0.0 {
                let relative = -((wait * 1e7) as i64).max(1);
                unsafe {
                    let _ = SetWaitableTimer(timer, &relative, 0, None, None, false);
                    WaitForSingleObject(timer, INFINITE);
                }
            } else {
                next = now();
            }

            let window = PACER_WINDOW.load(Ordering::Relaxed);

            if window == 0 {
                return;
            }

            let main_loop = f64::from_bits(MAIN_LOOP_TICK.load(Ordering::Relaxed));

            if now() - main_loop > 2.0 * period && !TICK_POSTED.swap(true, Ordering::Relaxed) {
                let hwnd = HWND(window as *mut _);

                if unsafe { PostMessageW(Some(hwnd), WM_APP_TICK, WPARAM(0), LPARAM(0)) }.is_err() {
                    TICK_POSTED.store(false, Ordering::Relaxed);
                }
            }
        }
    });

    if let Err(e) = started {
        log!("starting the pacer: {e}");
    }
}

/// The main loop's tick interval.
pub fn tick_period() -> f64 {
    with_emu(|emu| emu.tick_period()).unwrap_or(1.0 / 60.0)
}

/// Windows sends a Left Ctrl down just before Right Alt on keyboard layouts
/// with AltGr, with the same time. True if this Left Ctrl is that one.
fn is_altgr_ctrl(hwnd: HWND) -> bool {
    let mut next = MSG::default();
    let time = unsafe { GetMessageTime() } as u32;

    unsafe { PeekMessageW(&mut next, Some(hwnd), WM_KEYFIRST, WM_KEYLAST, PM_NOREMOVE) }.as_bool()
        && (next.message == WM_KEYDOWN || next.message == WM_SYSKEYDOWN)
        && next.wParam.0 == VK_MENU.0 as usize
        && (next.lParam.0 >> 24) & 1 == 1
        && next.time == time
}

/// A key message: to the Juku, or a menu shortcut. Returns true if the
/// message is handled and must not reach DefWindowProcW.
fn key_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
    let (handled, command) = key_event(hwnd, msg, wparam, lparam);

    if msg == WM_SYSKEYDOWN {
        SHORTCUT_TAKEN.set(command.is_some());
    }

    if let Some(command) = command {
        crate::command(hwnd, command);
    }

    handled
}

/// The key message's effect on the Juku, and the shortcut's command, if it
/// is one.
fn key_event(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> (bool, Option<u16>) {
    let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
    let flags = lparam.0 as u32;
    let scan = ((flags >> 16) & 0xFF) as u16 | if flags & (1 << 24) != 0 { keymap::EXT } else { 0 };
    let repeat = down && flags & (1 << 30) != 0;

    if wparam.0 == VK_F10.0 as usize && msg == WM_SYSKEYDOWN {
        F10_MENU.set(true);
    }

    let command = with_emu(|emu| {
        if scan == keymap::LEFT_CTRL {
            if down && !emu.altgr_ctrl && is_altgr_ctrl(hwnd) {
                emu.altgr_ctrl = true;
                return None;
            }

            if !down && emu.altgr_ctrl {
                emu.altgr_ctrl = false;
                return None;
            }
        }

        emu.key(scan, down, repeat);

        // Left Alt shortcuts go to the menus (§11.3); not Right Alt, which is
        // a Juku key, and not a held key's repeats.
        if down && !repeat && emu.left_alt && scan != keymap::LEFT_ALT {
            let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
            return crate::shortcut(wparam.0 as u16, shift);
        }

        None
    })
    .flatten();

    // Plain key messages are the Juku's alone. System key messages also go
    // on to Windows, for Alt+F4, Alt+Space, Alt+Tab and F10.
    (command.is_some() || msg == WM_KEYDOWN || msg == WM_KEYUP, command)
}

/// The files dropped on the window: the first goes in A: read-only, or in B:
/// with Alt held (Option on the Mac); a snapshot is opened.
fn dropped(hwnd: HWND, drop: HDROP) {
    let path = unsafe {
        let len = DragQueryFileW(drop, 0, None) as usize;
        let mut name = vec![0u16; len + 1];
        DragQueryFileW(drop, 0, Some(&mut name));
        DragFinish(drop);
        PathBuf::from(String::from_utf16_lossy(&name[..len]))
    };
    let drive = (unsafe { GetKeyState(VK_MENU.0 as i32) } < 0) as usize;
    crate::open_file(hwnd, &path, drive);
}

/// Keeps the content 4:3 while the window is sized by its edge `edge`.
fn keep_aspect(hwnd: HWND, edge: u32, r: &mut RECT) {
    let (extra_w, extra_h) = frame_extra(unsafe { GetDpiForWindow(hwnd) });
    let width = (r.right - r.left - extra_w).max(1) as f32;
    let height = (r.bottom - r.top - extra_h).max(1) as f32;

    match edge {
        WMSZ_TOP | WMSZ_BOTTOM => r.right = r.left + (height * TUBE_ASPECT).round() as i32 + extra_w,
        WMSZ_TOPLEFT | WMSZ_TOPRIGHT => r.top = r.bottom - (width / TUBE_ASPECT).round() as i32 - extra_h,
        _ => r.bottom = r.top + (width / TUBE_ASPECT).round() as i32 + extra_h,
    }
}

extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if let Some(result) = crate::theme::menu_bar_message(hwnd, msg, wparam, lparam) {
        return result;
    }

    if crate::theme::changed(msg, lparam) {
        crate::theme::apply(hwnd);
        crate::theme::redraw(hwnd);
    }

    match msg {
        WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP => {
            if key_message(hwnd, msg, wparam, lparam) {
                return LRESULT(0);
            }
        }
        // Left Alt with a menu's underlined letter opens that menu, and
        // Alt+Space the window menu. Any other Alt+letter is dropped, since
        // Windows would beep for it; so is the letter of a shortcut, and
        // Right Alt, which is a Juku key, with a letter.
        WM_SYSCHAR if wparam.0 != ' ' as usize => {
            let letter = char::from_u32(wparam.0 as u32).is_some_and(crate::is_menu_letter);
            let menu_bar = with_emu(|emu| emu.left_alt && !emu.full_screen) == Some(true);

            if SHORTCUT_TAKEN.replace(false) || !letter || !menu_bar {
                return LRESULT(0);
            }
        }
        WM_NCLBUTTONDOWN => title_bar_pressed(hwnd, wparam, lparam),
        // Alt pressed and released alone does not open the menu bar, since
        // the next keys would go to the menu instead of the Juku; F10 does.
        WM_SYSCOMMAND if (wparam.0 as u32 & 0xFFF0) == SC_KEYMENU && lparam.0 == 0 => {
            if !F10_MENU.replace(false) {
                return LRESULT(0);
            }

            // In full screen there is no menu bar: F10 opens the menus at the
            // top left corner instead.
            if with_emu(|emu| emu.full_screen) == Some(true) {
                crate::show_menu_popup(hwnd);
                return LRESULT(0);
            }
        }
        WM_SETFOCUS => {
            with_emu(|emu| emu.left_alt = unsafe { GetKeyState(VK_LMENU.0 as i32) } < 0);
        }
        WM_KILLFOCUS => {
            with_emu(|emu| {
                emu.release_keys();
                emu.release_mouse();
            });
        }
        WM_ENTERMENULOOP => {
            with_emu(|emu| {
                emu.release_keys();
                emu.release_mouse();
                emu.menu_open = true;
                emu.show_cursor();
            });
        }
        WM_ENTERSIZEMOVE => {
            with_emu(Emu::release_mouse);
        }
        WM_MOVE => {
            with_emu(|emu| {
                if emu.mouse.captured {
                    emu.park_cursor();
                }
            });
        }
        WM_INPUT => {
            with_emu(|emu| emu.raw_input(HRAWINPUT(lparam.0 as *mut _)));
        }
        WM_MOUSEACTIVATE => {
            ACTIVATING_CLICK.set((lparam.0 & 0xFFFF) as u32 == HTCLIENT);
        }
        // A click captures the mouse only when the running program uses it
        // (IR6 unmasked), so clicking the window at the monitor or in EKDOS
        // never hides the cursor.
        WM_LBUTTONDOWN => {
            if !ACTIVATING_CLICK.replace(false) {
                with_emu(|emu| {
                    if !emu.mouse.captured && emu.machine.mouse_enabled() {
                        emu.capture_mouse();
                    }
                });
            }
        }
        WM_EXITMENULOOP => {
            with_emu(|emu| {
                emu.menu_open = false;
                emu.cursor_moved();
            });
        }
        WM_MOUSEMOVE => {
            // Windows also sends this when nothing moved (a window above
            // changed, say): only a new position counts.
            let position = (lparam.0 & 0xFFFF_FFFF) as u32;

            if LAST_MOUSE.replace(position) != position {
                with_emu(Emu::cursor_moved);
            }
        }
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            if with_emu(|emu| emu.cursor_hidden || emu.mouse.captured) == Some(true) {
                unsafe { SetCursor(None) };
                return LRESULT(1);
            }
        }
        WM_TIMER if wparam.0 == CURSOR_TIMER => {
            with_emu(Emu::hide_idle_cursor);
            return LRESULT(0);
        }
        WM_INITMENUPOPUP => {
            crate::update_menu(HMENU(wparam.0 as *mut _));
        }
        WM_COMMAND => {
            crate::command(hwnd, (wparam.0 & 0xFFFF) as u16);
            return LRESULT(0);
        }
        WM_DROPFILES => {
            dropped(hwnd, HDROP(wparam.0 as *mut _));
            return LRESULT(0);
        }
        // A file from another start of Juku. It is opened once this message
        // has returned, so that start does not wait on an alert.
        WM_COPYDATA => {
            let data = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };

            if data.dwData == crate::COPYDATA_OPEN && !data.lpData.is_null() {
                let text = unsafe { std::slice::from_raw_parts(data.lpData as *const u16, data.cbData as usize / 2) };
                HANDED_OVER.with_borrow_mut(|files| files.push(PathBuf::from(String::from_utf16_lossy(text))));
                let _ = unsafe { PostMessageW(Some(hwnd), WM_APP_OPEN, WPARAM(0), LPARAM(0)) };
            }

            // `--debug-server`, as VS Code's extension starts Juku: the
            // port, which this Juku listens on from now on.
            if data.dwData == crate::COPYDATA_DEBUG_SERVER && data.cbData == 2 && !data.lpData.is_null() {
                let port = unsafe { (data.lpData as *const u16).read_unaligned() };
                let _ = unsafe { PostMessageW(Some(hwnd), WM_APP_DEBUG_SERVER, WPARAM(port as usize), LPARAM(0)) };
            }

            return LRESULT(1);
        }
        WM_APP_OPEN => {
            for file in HANDED_OVER.take() {
                crate::open_file(hwnd, &file, 0);
            }

            return LRESULT(0);
        }
        WM_APP_DEBUG_SERVER => {
            if let Some(Some(port)) = with_emu(|emu| emu.set_debug_server_argument(wparam.0 as u16)) {
                crate::show_debugger_port_alert(hwnd, port);
            }

            return LRESULT(0);
        }
        // A launch's snapshot (spec §12.7), opened as one from Explorer is,
        // the changed disks asked about first. The server hears on the next
        // poll whether it opened.
        WM_APP_DEBUG_SNAPSHOT => {
            if let Some(path) = DEBUG_SNAPSHOT.take() {
                let opened = !crate::disks::asking() && crate::snapshots::open_for_debugger(hwnd, &path);
                DEBUG_SNAPSHOT_OPENED.set(Some(opened));
                with_emu(Emu::poll_debugger);
            }

            return LRESULT(0);
        }
        // Quitting: each changed disk asks to be saved first, and Cancel
        // keeps the window (the Mac's applicationShouldTerminate).
        WM_CLOSE => {
            with_emu(Emu::release_mouse);
            let quit = crate::disks::may_remove_all(hwnd);
            let _ = unsafe { ShutdownBlockReasonDestroy(hwnd) };

            if quit {
                with_emu(|emu| {
                    emu.save_placement();
                    settings::flush();
                });
                let _ = unsafe { DestroyWindow(hwnd) };
            }

            return LRESULT(0);
        }
        // Log off or shut down with a changed disk: Windows waits, showing
        // why, while the window asks as on quit. If the user ends the
        // session anyway, the changes are lost.
        WM_QUERYENDSESSION => {
            let changed = with_emu(|emu| (0..crate::disks::DRIVES).any(|d| DiskDrives::changed(&emu.machine, d))).unwrap_or(false);

            if !changed {
                return LRESULT(1);
            }

            let reason = HSTRING::from(crate::strings::localized("Disks have unsaved changes."));
            let _ = unsafe { ShutdownBlockReasonCreate(hwnd, &reason) };
            let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
            return LRESULT(0);
        }
        WM_APP_TICK => {
            TICK_POSTED.store(false, Ordering::Relaxed);
            let now = now();
            with_emu(|emu| {
                emu.tick(now);
                PACER_PERIOD.store(emu.tick_period.to_bits(), Ordering::Relaxed);
            });
            return LRESULT(0);
        }
        WM_SIZE => {
            let now = now();
            with_emu(|emu| {
                if emu.sync_size() {
                    emu.tick(now);
                }

                if emu.mouse.captured {
                    emu.park_cursor();
                }
            });
        }
        WM_SIZING => {
            let rect = unsafe { &mut *(lparam.0 as *mut RECT) };
            keep_aspect(hwnd, wparam.0 as u32, rect);
            return LRESULT(1);
        }
        WM_GETMINMAXINFO => {
            // At least 1× without the bezel margin.
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            let (w, h) = content_size(MIN_WINDOW_HALVES, 0.0);
            let (extra_w, extra_h) = frame_extra(dpi);
            let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
            info.ptMinTrackSize.x = scale(w, dpi) + extra_w;
            info.ptMinTrackSize.y = scale(h, dpi) + extra_h;
            return LRESULT(0);
        }
        WM_DPICHANGED => {
            let r = unsafe { &*(lparam.0 as *const RECT) };
            let _ = unsafe { SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE) };
            with_emu(|emu| emu.tick_period = refresh_period(hwnd));
            return LRESULT(0);
        }
        WM_EXITSIZEMOVE | WM_DISPLAYCHANGE => {
            with_emu(|emu| {
                emu.tick_period = refresh_period(hwnd);
                emu.save_placement();
            });
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            unsafe {
                BeginPaint(hwnd, &mut paint);
                let _ = EndPaint(hwnd, &paint);
            }
            with_emu(Emu::set_needs_redraw);
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_DESTROY => {
            PACER_WINDOW.store(0, Ordering::Relaxed);
            let _ = unsafe { KillTimer(Some(hwnd), CURSOR_TIMER) };
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        _ => {}
    }

    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

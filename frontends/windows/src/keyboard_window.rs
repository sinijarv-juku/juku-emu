//! Input → Keyboard (spec §6.4, §11.3), the counterpart of the Mac's
//! KeyboardWindow.m and KeyboardView.m: the Juku keyboard, drawn by the core
//! and replayed with Direct2D (keyboard_picture.rs), above an info strip that
//! describes the key under the pointer or the one last typed and shows
//! RomBios's LAT/RUS and CAPS state.
//!
//! The window never takes the keyboard (`WS_EX_NOACTIVATE`), so the
//! emulator window keeps it and typing goes on while it is open; it has no
//! controls for that reason, and draws everything itself. Like Settings it
//! is created once, hidden when closed and owned by the emulator window.

use std::cell::{Cell, RefCell};

use juku::Machine;
use juku::keyboard::drawing::{self, HEIGHT, KeyboardState, WIDTH};
use juku::keyboard::layout::{KEYCAPS, Keycap, Legend};
use juku::keyboard::{KEY_CAPS, KEY_CTRL, KEY_SHIFT};
use windows::Win32::Foundation::{COLORREF, D2DERR_RECREATE_TARGET, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_UNKNOWN, D2D1_COLOR_F, D2D1_PIXEL_FORMAT};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_IMMEDIATELY,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, ID2D1HwndRenderTarget, ID2D1RenderTarget, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN;
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromRect, MonitorFromWindow, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetActiveWindow, ReleaseCapture, SetActiveWindow, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

use crate::emu_window;
use crate::keyboard_picture::{Graphics, color, fit, rect, replay, to_picture};
use crate::keymap::{self, HostKey};
use crate::settings::{self, KEYBOARD_TAGS, SECTION_KEYBOARD_WINDOW, WINDOW_FRAME};
use crate::strings::{format, localized};
use crate::theme;

const CLASS_NAME: PCWSTR = w!("JukuKeyboard");
const STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_CAPTION.0 | WS_SYSMENU.0 | WS_THICKFRAME.0);
const EX_STYLE: WINDOW_EX_STYLE = WS_EX_NOACTIVATE;
const KEYS: usize = KEYCAPS.len();

// Sizes in DIPs, as the Mac's in points: the window opens half as wide as the
// screen, at most 1240, and is at least 600 wide; the strip below the
// keyboard keeps its height.
const MAX_DEFAULT_WIDTH: i32 = 1240;
const MIN_WIDTH: i32 = 600;
const STRIP: f32 = 80.0;
const PREVIEW: (f32, f32) = (110.0, 54.0);
const PAD: f32 = 16.0;

/// How long the info strip keeps a key after the pointer leaves it.
const LINGER_TIMER: usize = 1;
const LINGER_MS: u32 = 150;

/// How PC key tags are shown on the keys (`KeyboardTags`).
#[derive(Clone, Copy, PartialEq)]
enum TagMode {
    WhereDifferent = 0,
    All = 1,
    Off = 2,
}

const TAG_MODES: [(TagMode, &str); 3] = [(TagMode::WhereDifferent, "Where Different"), (TagMode::All, "All"), (TagMode::Off, "Off")];

fn tag_mode_setting() -> TagMode {
    match settings::string_setting(KEYBOARD_TAGS).as_deref() {
        Some("1") => TagMode::All,
        Some("2") => TagMode::Off,
        _ => TagMode::WhereDifferent,
    }
}

/// Where the strip's parts are, in pixels of the client area.
struct StripLayout {
    strip: D2D_RECT_F,
    preview: D2D_RECT_F,
    info: (f32, f32, f32), // left, right, top of the first line
    tags_label: (f32, f32), // left, middle
    segments: [D2D_RECT_F; 3],
    pills: [D2D_RECT_F; 3],
}

struct KeyboardWindow {
    hwnd: HWND,
    graphics: Graphics,
    target: Option<ID2D1HwndRenderTarget>,
    down: [bool; KEYS],
    consw: u8,
    hover: Option<usize>,
    /// The key the strip describes: the one under the pointer or last typed.
    shown: Option<usize>,
    /// Pressed by the mouse, until it is let go.
    held: Option<usize>,
    /// SHIFT and CTRL, held by a click until the next key.
    latched: Vec<u8>,
    host_keys: Vec<Vec<HostKey>>,
    tags: Vec<Option<String>>,
    differing: Vec<bool>,
    tag_mode: TagMode,
    hover_segment: Option<usize>,
    tracking: bool,
    /// The key the strip keeps after the pointer left it.
    lingering: Option<usize>,
}

thread_local! {
    static WINDOW: RefCell<Option<KeyboardWindow>> = const { RefCell::new(None) };
    // In the modal loop that moves or sizes the window.
    static SIZING: Cell<bool> = const { Cell::new(false) };
    static DRAG: Cell<Option<Drag>> = const { Cell::new(None) };
    // Whether the app is the active one, which the title bar shows.
    static APP_ACTIVE: Cell<bool> = const { Cell::new(true) };
}

fn with_window<R>(f: impl FnOnce(&mut KeyboardWindow) -> R) -> Option<R> {
    WINDOW.with(|w| w.try_borrow_mut().ok().and_then(|mut w| w.as_mut().map(f)))
}

fn hwnd() -> Option<HWND> {
    WINDOW.with(|w| w.try_borrow().ok().and_then(|w| w.as_ref().map(|w| w.hwnd)))
}

/// Whether the window is shown, for the Input menu's tick.
pub fn visible() -> bool {
    hwnd().is_some_and(|hwnd| unsafe { IsWindowVisible(hwnd) }.as_bool())
}

/// Shows the window, creating it the first time, or closes it if it is
/// shown.
pub fn toggle(owner: HWND) {
    if let Some(hwnd) = hwnd() {
        if unsafe { IsWindowVisible(hwnd) }.as_bool() {
            close(hwnd);
        } else {
            with_window(KeyboardWindow::update_tags);
            show(hwnd);
        }

        return;
    }

    let Some(hwnd) = create(owner) else {
        return;
    };

    show(hwnd);
}

/// Shows the window without activating it, its title bar drawn as the app's
/// state says.
fn show(hwnd: HWND) {
    APP_ACTIVE.set(app_is_active());

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SendMessageW(hwnd, WM_NCACTIVATE, Some(WPARAM(APP_ACTIVE.get() as usize)), None);
    }
}

/// Whether the foreground window is one of the app's.
fn app_is_active() -> bool {
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), None) == GetCurrentThreadId() }
}

/// The keyboard layout changed: the tags follow it.
pub fn layout_changed() {
    with_window(|w| {
        w.update_tags();
        w.invalidate();
    });
}

/// Reads which keys are down and RomBios's mode, and redraws what changed.
/// Called on each tick of the emulator window, with its machine.
pub fn follow(machine: &Machine) {
    with_window(|w| {
        if unsafe { IsWindowVisible(w.hwnd) }.as_bool() {
            w.poll(machine);
        }
    });
}

fn scale(dips: f32, dpi: u32) -> f32 {
    dips * dpi as f32 / 96.0
}

fn create(owner: HWND) -> Option<HWND> {
    let instance = unsafe { GetModuleHandleW(None) }.ok()?;
    let graphics = Graphics::new().map_err(|e| crate::log!("Direct2D for the Keyboard window: {e}")).ok()?;

    let hwnd = unsafe {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: emu_window::icon(instance.into(), SM_CXICON, SM_CYICON),
            hIconSm: emu_window::icon(instance.into(), SM_CXSMICON, SM_CYSMICON),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        RegisterClassExW(&class);

        CreateWindowExW(
            EX_STYLE,
            CLASS_NAME,
            &HSTRING::from(localized("Keyboard")),
            STYLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            0,
            0,
            Some(owner),
            None,
            Some(instance.into()),
            None,
        )
    }
    .ok()?;

    theme::apply(hwnd);

    let mut window = KeyboardWindow {
        hwnd,
        graphics,
        target: None,
        down: [false; KEYS],
        consw: 0xFF,
        hover: None,
        shown: None,
        held: None,
        latched: Vec::new(),
        host_keys: Vec::new(),
        tags: Vec::new(),
        differing: Vec::new(),
        tag_mode: tag_mode_setting(),
        hover_segment: None,
        tracking: false,
        lingering: None,
    };
    window.update_tags();
    WINDOW.with_borrow_mut(|w| *w = Some(window));
    place(hwnd, owner);
    Some(hwnd)
}

/// The frame Juku.ini keeps, if it is still on a monitor.
fn saved_frame() -> Option<RECT> {
    let text = settings::get(SECTION_KEYBOARD_WINDOW, WINDOW_FRAME)?;
    let n: Vec<i32> = text.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    let rect = match n[..] {
        [left, top, right, bottom] if right > left && bottom > top => RECT { left, top, right, bottom },
        _ => return None,
    };
    (!unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONULL) }.is_invalid()).then_some(rect)
}

fn save_frame(hwnd: HWND) {
    let mut r = RECT::default();

    if unsafe { GetWindowRect(hwnd, &mut r) }.is_ok() {
        settings::set(SECTION_KEYBOARD_WINDOW, WINDOW_FRAME, Some(&format!("{},{},{},{}", r.left, r.top, r.right, r.bottom)));
    }
}

/// What the frame adds to the client area, in pixels.
fn frame_extra(hwnd: HWND) -> (i32, i32) {
    let mut r = RECT::default();
    let _ = unsafe { AdjustWindowRectExForDpi(&mut r, STYLE, false, EX_STYLE, GetDpiForWindow(hwnd)) };
    (r.right - r.left, r.bottom - r.top)
}

/// The client height that keeps the keyboard's proportions at `width`.
fn client_height(width: i32, dpi: u32) -> i32 {
    (width as f32 * HEIGHT / WIDTH + scale(STRIP, dpi)).round() as i32
}

/// The client width for a client `height`.
fn client_width(height: i32, dpi: u32) -> i32 {
    ((height as f32 - scale(STRIP, dpi)).max(1.0) * WIDTH / HEIGHT).round() as i32
}

/// Puts the window where it was, keeping its width and top edge, since a
/// frame saved by another version may have other proportions; or the first
/// time, centered on the owner's screen.
fn place(hwnd: HWND, owner: HWND) {
    let (extra_w, extra_h) = frame_extra(hwnd);

    if let Some(frame) = saved_frame() {
        // Moved first, so the size is taken at that monitor's DPI.
        unsafe {
            let _ = SetWindowPos(hwnd, None, frame.left, frame.top, frame.right - frame.left, frame.bottom - frame.top, SWP_NOZORDER | SWP_NOACTIVATE);
        }

        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let (extra_w, extra_h) = frame_extra(hwnd);
        let width = (frame.right - frame.left - extra_w).max(scale(MIN_WIDTH as f32, dpi) as i32);
        let _ = unsafe { SetWindowPos(hwnd, None, 0, 0, width + extra_w, client_height(width, dpi) + extra_h, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE) };
        return;
    }

    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = unsafe { GetMonitorInfoW(MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST), &mut info) };
    let work = info.rcWork;
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let width = ((work.right - work.left) / 2 - extra_w)
        .min(scale(MAX_DEFAULT_WIDTH as f32, dpi) as i32)
        .max(scale(MIN_WIDTH as f32, dpi) as i32);
    let (w, h) = (width + extra_w, client_height(width, dpi) + extra_h);
    let x = work.left + (work.right - work.left - w) / 2;
    let y = work.top + (work.bottom - work.top - h) / 2;
    let _ = unsafe { SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE) };
}

/// Keeps the keyboard's proportions while the window is sized by `edge`.
fn keep_proportions(hwnd: HWND, edge: u32, r: &mut RECT) {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let (extra_w, extra_h) = frame_extra(hwnd);
    let min = scale(MIN_WIDTH as f32, dpi) as i32;

    match edge {
        WMSZ_TOP | WMSZ_BOTTOM => {
            let width = client_width(r.bottom - r.top - extra_h, dpi).max(min);
            r.right = r.left + width + extra_w;

            if edge == WMSZ_TOP {
                r.top = r.bottom - client_height(width, dpi) - extra_h;
            } else {
                r.bottom = r.top + client_height(width, dpi) + extra_h;
            }
        }
        _ => {
            let width = (r.right - r.left - extra_w).max(min);
            let height = client_height(width, dpi) + extra_h;

            if edge == WMSZ_TOPLEFT || edge == WMSZ_TOPRIGHT {
                r.top = r.bottom - height;
            } else {
                r.bottom = r.top + height;
            }
        }
    }
}

/// The smallest window, frame included: 600 DIPs of keyboard and the strip.
fn min_size(hwnd: HWND) -> (i32, i32) {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let (extra_w, extra_h) = frame_extra(hwnd);
    let width = scale(MIN_WIDTH as f32, dpi) as i32;
    (width + extra_w, client_height(width, dpi) + extra_h)
}

/// A move or size by the mouse, which the window does itself: Windows' own
/// loop activates the window, taking the keyboard from the emulator window.
#[derive(Clone, Copy)]
struct Drag {
    /// What was pressed: the title bar or an edge (`HTCAPTION`, `HTLEFT`…).
    hit: u32,
    cursor: POINT,
    frame: RECT,
}

/// Starts moving or sizing the window for a press on its title bar or an
/// edge; false for any other part of the frame.
fn begin_drag(hwnd: HWND, hit: u32) -> bool {
    if hit != HTCAPTION && !(HTLEFT..=HTBOTTOMRIGHT).contains(&hit) {
        return false;
    }

    let (mut cursor, mut frame) = (POINT::default(), RECT::default());

    unsafe {
        let _ = GetCursorPos(&mut cursor);
        let _ = GetWindowRect(hwnd, &mut frame);
        SetCapture(hwnd);
    }

    DRAG.set(Some(Drag { hit, cursor, frame }));
    true
}

/// Follows the mouse while a move or size goes on; false if none does.
fn drag(hwnd: HWND) -> bool {
    let Some(d) = DRAG.get() else {
        return false;
    };
    let mut cursor = POINT::default();
    let _ = unsafe { GetCursorPos(&mut cursor) };
    let (dx, dy) = (cursor.x - d.cursor.x, cursor.y - d.cursor.y);
    let mut r = d.frame;
    let flags = SWP_NOZORDER | SWP_NOACTIVATE;

    if d.hit == HTCAPTION {
        let _ = unsafe { SetWindowPos(hwnd, None, r.left + dx, r.top + dy, 0, 0, flags | SWP_NOSIZE) };
        return true;
    }

    // HTLEFT … HTBOTTOMRIGHT are in the order of WMSZ_LEFT … WMSZ_BOTTOMRIGHT.
    let edge = d.hit - HTLEFT + WMSZ_LEFT;
    let left = matches!(edge, WMSZ_LEFT | WMSZ_TOPLEFT | WMSZ_BOTTOMLEFT);
    let top = matches!(edge, WMSZ_TOP | WMSZ_TOPLEFT | WMSZ_TOPRIGHT);

    match edge {
        WMSZ_LEFT | WMSZ_TOPLEFT | WMSZ_BOTTOMLEFT => r.left += dx,
        WMSZ_RIGHT | WMSZ_TOPRIGHT | WMSZ_BOTTOMRIGHT => r.right += dx,
        _ => {}
    }

    match edge {
        WMSZ_TOP | WMSZ_TOPLEFT | WMSZ_TOPRIGHT => r.top += dy,
        WMSZ_BOTTOM | WMSZ_BOTTOMLEFT | WMSZ_BOTTOMRIGHT => r.bottom += dy,
        _ => {}
    }

    // At least the smallest size, the edges not dragged staying where they are.
    let (min_w, min_h) = min_size(hwnd);

    if r.right - r.left < min_w {
        if left { r.left = r.right - min_w } else { r.right = r.left + min_w }
    }

    if r.bottom - r.top < min_h {
        if top { r.top = r.bottom - min_h } else { r.bottom = r.top + min_h }
    }

    keep_proportions(hwnd, edge, &mut r);
    let _ = unsafe { SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, flags) };
    true
}

/// Ends a move or size, if one goes on, and keeps the frame.
fn end_drag(hwnd: HWND) {
    if DRAG.take().is_some() {
        save_frame(hwnd);
    }
}

/// Closing hides the window, so it opens again where it was, and lets go of
/// what clicks hold.
fn close(hwnd: HWND) {
    with_window(KeyboardWindow::release_clicks);
    save_frame(hwnd);
    let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
}

/// A theme color as Direct2D takes it, with alpha `a`.
fn ui_color(c: COLORREF, a: f32) -> D2D1_COLOR_F {
    let channel = |shift: u32| ((c.0 >> shift) & 0xFF) as f32 / 255.0;
    D2D1_COLOR_F { r: channel(0), g: channel(8), b: channel(16), a }
}

fn contains(r: &D2D_RECT_F, x: f32, y: f32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

impl KeyboardWindow {
    fn invalidate(&self) {
        let _ = unsafe { InvalidateRect(Some(self.hwnd), None, false) };
    }

    fn dpi(&self) -> u32 {
        unsafe { GetDpiForWindow(self.hwnd) }
    }

    fn client(&self) -> (f32, f32) {
        let mut r = RECT::default();
        let _ = unsafe { GetClientRect(self.hwnd, &mut r) };
        (r.right as f32, r.bottom as f32)
    }

    /// The keyboard's part of the client area.
    fn keyboard_rect(&self) -> D2D_RECT_F {
        let (w, h) = self.client();
        D2D_RECT_F { left: 0.0, top: 0.0, right: w, bottom: (h - scale(STRIP, self.dpi())).max(0.0) }
    }

    fn keyboard_transform(&self) -> windows_numerics::Matrix3x2 {
        fit(self.keyboard_rect(), (WIDTH, HEIGHT))
    }

    /// Each keycap's PC keys in the current layout, and its tag.
    fn update_tags(&mut self) {
        self.host_keys = KEYCAPS.iter().map(keymap::host_keys).collect();
        self.tags = self.host_keys.iter().map(|keys| keymap::tag_key(keys).map(|k| k.tag.clone())).collect();
        self.differing = self.host_keys.iter().map(|keys| keymap::tag_key(keys).is_some_and(|k| !k.natural)).collect();
    }

    fn tag_shown(&self, index: usize) -> bool {
        let has_tag = self.tags.get(index).is_some_and(Option::is_some);

        match self.tag_mode {
            TagMode::All => has_tag,
            TagMode::WhereDifferent => has_tag && self.differing.get(index).copied().unwrap_or(false),
            TagMode::Off => false,
        }
    }

    // ---- Following the machine -------------------------------------------------

    /// Redraws the keys whose state changed. A key typed shows in the info
    /// strip, unless the pointer is over another one.
    fn poll(&mut self, machine: &Machine) {
        let mut typed = None;
        let mut changed = false;

        for (i, k) in KEYCAPS.iter().enumerate() {
            let down = machine.key_down(k.id);

            if down == self.down[i] {
                continue;
            }

            self.down[i] = down;
            changed = true;

            // The Caps Lock follower presses CAPS by itself.
            if down && typed.is_none() && self.held != Some(i) && k.id != KEY_CAPS {
                typed = Some(i);
            }
        }

        // A latch the emulator window let go of: the PC's Shift released, or
        // every key on focus loss.
        self.latched.retain(|&key| machine.key_down(key));

        if let Some(typed) = typed
            && self.hover.is_none()
            && !self.latched.contains(&KEYCAPS[typed].id)
        {
            self.shown = Some(typed);
        }

        let consw = machine.consw() & 3;

        if consw != self.consw {
            self.consw = consw;
            changed = true;
        }

        if changed {
            self.invalidate();
        }
    }

    // ---- Clicks -----------------------------------------------------------------

    fn set_key(&self, key: u8, down: bool) {
        emu_window::with_emu(|emu| emu.machine().set_key(key, down));
    }

    fn poll_now(&mut self) {
        let _ = emu_window::with_emu(|emu| self.poll(emu.machine()));
    }

    fn keycap_at(&self, x: f32, y: f32) -> Option<usize> {
        if !contains(&self.keyboard_rect(), x, y) {
            return None;
        }

        let (px, py) = to_picture(&self.keyboard_transform(), x, y);
        drawing::keycap_at(px, py)
    }

    fn mouse_down(&mut self, x: f32, y: f32) {
        if let Some(segment) = self.strip_layout().segments.iter().position(|r| contains(r, x, y)) {
            self.tag_mode = TAG_MODES[segment].0;
            settings::set_string_setting(KEYBOARD_TAGS, Some(&segment.to_string()));
            self.invalidate();
            return;
        }

        let Some(index) = self.keycap_at(x, y) else {
            return;
        };
        let key = KEYCAPS[index].id;
        self.shown = Some(index);
        self.invalidate();

        if key == KEY_CAPS {
            return; // the guest Caps Lock follows the PC's
        }

        if key == KEY_SHIFT || key == KEY_CTRL {
            let latch = !self.latched.contains(&key);

            if latch {
                self.latched.push(key);
            } else {
                self.latched.retain(|&k| k != key);
            }

            self.set_key(key, latch);
            self.poll_now();
            return;
        }

        self.held = Some(index);
        self.set_key(key, true);
        let _ = unsafe { SetCapture(self.hwnd) };
        self.poll_now();
    }

    fn mouse_up(&mut self) {
        let Some(held) = self.held.take() else {
            return;
        };

        self.set_key(KEYCAPS[held].id, false);

        for key in std::mem::take(&mut self.latched) {
            self.set_key(key, false);
        }

        self.poll_now();
    }

    fn release_clicks(&mut self) {
        if self.held.is_some() {
            self.mouse_up();
            let _ = unsafe { ReleaseCapture() };
        }

        for key in std::mem::take(&mut self.latched) {
            self.set_key(key, false);
        }

        self.poll_now();
    }

    fn mouse_move(&mut self, x: f32, y: f32) {
        if !self.tracking {
            let mut track = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.hwnd, dwHoverTime: 0 };
            self.tracking = unsafe { TrackMouseEvent(&mut track) }.is_ok();
        }

        let segment = self.strip_layout().segments.iter().position(|r| contains(r, x, y));
        self.set_hover(self.keycap_at(x, y), segment);
    }

    fn set_hover(&mut self, index: Option<usize>, segment: Option<usize>) {
        if segment != self.hover_segment {
            self.hover_segment = segment;
            self.invalidate();
        }

        if index == self.hover {
            return;
        }

        self.hover = index;
        self.invalidate();

        // Off a key, the strip keeps it a moment, so that crossing the gap
        // between two keys does not flash its idle text.
        if index.is_none() && self.shown.is_some() {
            self.lingering = self.shown;
            let _ = unsafe { SetTimer(Some(self.hwnd), LINGER_TIMER, LINGER_MS, None) };
            return;
        }

        let _ = unsafe { KillTimer(Some(self.hwnd), LINGER_TIMER) };
        self.shown = index;
    }

    /// The moment off a key is over: the strip shows no key, unless the
    /// pointer is on one again or a key was typed meanwhile.
    fn linger_over(&mut self) {
        let _ = unsafe { KillTimer(Some(self.hwnd), LINGER_TIMER) };

        if self.hover.is_none() && self.shown == self.lingering.take() {
            self.shown = None;
            self.invalidate();
        }
    }

    // ---- The info strip -----------------------------------------------------------

    /// The strip's three lines for the key shown, or for none.
    fn strip_lines(&self) -> [String; 3] {
        let Some(index) = self.shown else {
            return [
                localized("Juku E5101 keyboard").into(),
                localized("Click a key to press it. SHIFT and CTRL stay down until the next key.").into(),
                localized("Keys go down as you type, too.").into(),
            ];
        };
        let k: &Keycap = &KEYCAPS[index];
        let name = match k.legend {
            Legend::Pair | Legend::TwoLines => format!("{} {}", k.top, k.bottom),
            Legend::Blank => localized("Space bar").into(),
            _ => k.top.into(),
        };
        let names: Vec<&str> = self.host_keys.get(index).map(|keys| keys.iter().map(|k| k.name.as_str()).collect()).unwrap_or_default();
        let names = if names.is_empty() { "—".to_string() } else { names.join(", ") };
        let meta = match k.id {
            KEY_CAPS => localized("It follows your PC’s Caps Lock, so it cannot be clicked.").into(),
            KEY_SHIFT | KEY_CTRL => localized("A click holds it down until the next key.").into(),
            id => localized("Column %d, row %d of the key matrix").replacen("%d", &(id >> 3).to_string(), 1).replacen("%d", &(id & 7).to_string(), 1),
        };
        [name, format(localized("On your PC: %@"), &[&names]), meta]
    }

    /// Measures the strip's controls and places its parts: the preview at
    /// the left, the text beside it, and at the right the PC Keys control
    /// above the mode pills.
    fn strip_layout(&self) -> StripLayout {
        let dpi = self.dpi();
        let s = |dips: f32| scale(dips, dpi);
        let g = &self.graphics;
        let (w, h) = self.client();
        let strip = D2D_RECT_F { left: 0.0, top: (h - s(STRIP)).max(0.0), right: w, bottom: h };
        let mid = (strip.top + strip.bottom) / 2.0;
        let preview = rect(s(PAD), mid - s(PREVIEW.1) / 2.0, s(PREVIEW.0), s(PREVIEW.1));

        // The control: three segments as wide as their titles, and its label.
        let (control_h, pill_h, gap) = (s(22.0), s(19.0), s(8.0));
        let control_top = mid - (control_h + gap + pill_h) / 2.0;
        let widths: Vec<f32> = TAG_MODES
            .iter()
            .map(|(_, title)| g.text(localized(title), s(12.0), DWRITE_FONT_WEIGHT_NORMAL, 10_000.0).map_or(s(40.0), |t| t.width) + s(20.0))
            .collect();
        let mut x = w - s(PAD) - widths.iter().sum::<f32>();
        let control_left = x;
        let segments = [0, 1, 2].map(|i| {
            let r = rect(x, control_top, widths[i], control_h);
            x += widths[i];
            r
        });
        let label_width = g.text(localized("PC Keys:"), s(12.0), DWRITE_FONT_WEIGHT_NORMAL, 10_000.0).map_or(0.0, |t| t.width);
        let tags_label = (control_left - s(8.0) - label_width, control_top + control_h / 2.0);

        // The pills, right-aligned below it.
        let pill_top = control_top + control_h + gap;
        let mut right = w - s(PAD);
        let mut pills = [D2D_RECT_F::default(); 3];

        for (i, text) in ["CAPS", "RUS", "LAT"].iter().enumerate() {
            let width = g.text(text, s(10.5), DWRITE_FONT_WEIGHT_BOLD, 10_000.0).map_or(s(30.0), |t| t.width) + s(18.0);
            pills[2 - i] = rect(right - width, pill_top, width, pill_h);
            right -= width + s(6.0);
        }

        let info_left = preview.right + s(PAD);
        let info_right = (tags_label.0.min(pills[0].left) - s(PAD)).max(info_left + 1.0);
        let lines_h = s(20.0) + s(3.0) + s(17.0) + s(3.0) + s(16.0);
        StripLayout { strip, preview, info: (info_left, info_right, mid - lines_h / 2.0), tags_label, segments, pills }
    }

    fn draw_strip(&self, target: &ID2D1RenderTarget, brush: &ID2D1SolidColorBrush) -> windows::core::Result<()> {
        let dpi = self.dpi();
        let s = |dips: f32| scale(dips, dpi);
        let g = &self.graphics;
        let l = self.strip_layout();
        let text = theme::text_color();
        let gray = theme::gray_text_color();
        let background = theme::window_color();

        unsafe {
            brush.SetColor(&ui_color(background, 1.0));
            target.FillRectangle(&l.strip, brush);
            brush.SetColor(&ui_color(text, 0.15));
            target.FillRectangle(&D2D_RECT_F { bottom: l.strip.top + s(1.0).round().max(1.0), ..l.strip }, brush);
        }

        // The key, larger, on a piece of the opening.
        if let Some((picture, width, height)) = self.shown.and_then(drawing::keycap_alone) {
            replay(g, target, &picture.bytes, l.preview, &fit(l.preview, (width, height)), &|_| None)?;
        }

        let [name, line, meta] = self.strip_lines();
        let (left, right, mut y) = l.info;
        let lines = [(name, s(14.5), DWRITE_FONT_WEIGHT_SEMI_BOLD, text, s(20.0)), (line, s(12.0), DWRITE_FONT_WEIGHT_NORMAL, text, s(17.0)), (meta, s(11.5), DWRITE_FONT_WEIGHT_NORMAL, gray, s(16.0))];

        for (string, size, weight, color, height) in lines {
            let set = g.text(&string, size, weight, right - left)?;
            unsafe { brush.SetColor(&ui_color(color, 1.0)) };
            set.draw(target, brush, left, y + (height - set.height) / 2.0);
            y += height + s(3.0);
        }

        // PC Keys: Where Different | All | Off
        let label = g.text(localized("PC Keys:"), s(12.0), DWRITE_FONT_WEIGHT_NORMAL, 10_000.0)?;
        unsafe { brush.SetColor(&ui_color(gray, 1.0)) };
        label.draw_centered(target, brush, l.tags_label.0 + label.width / 2.0, l.tags_label.1);

        let radius = s(4.0);
        let whole = D2D_RECT_F { left: l.segments[0].left, right: l.segments[2].right, ..l.segments[0] };
        let rounded = |r: D2D_RECT_F, radius: f32| D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius };

        unsafe {
            brush.SetColor(&ui_color(text, 0.06));
            target.FillRoundedRectangle(&rounded(whole, radius), brush);
            brush.SetColor(&ui_color(text, 0.18));
            target.DrawRoundedRectangle(&rounded(inset(whole, 0.5), radius), brush, 1.0, None);
        }

        for (i, (mode, title)) in TAG_MODES.iter().enumerate() {
            let r = l.segments[i];
            let selected = *mode == self.tag_mode;

            unsafe {
                if selected {
                    let fill = if theme::dark() { ui_color(text, 0.16) } else { color(0xFFFFFFFF) };
                    brush.SetColor(&fill);
                    target.FillRoundedRectangle(&rounded(inset(r, s(2.0)), radius - s(1.0)), brush);
                    brush.SetColor(&ui_color(text, 0.12));
                    target.DrawRoundedRectangle(&rounded(inset(r, s(2.0) + 0.5), radius - s(1.0)), brush, 1.0, None);
                } else if self.hover_segment == Some(i) {
                    brush.SetColor(&ui_color(text, 0.06));
                    target.FillRoundedRectangle(&rounded(inset(r, s(2.0)), radius - s(1.0)), brush);
                }

                if i > 0 && !selected && TAG_MODES[i - 1].0 != self.tag_mode {
                    brush.SetColor(&ui_color(text, 0.15));
                    target.FillRectangle(&rect(r.left, r.top + s(5.0), 1.0, r.bottom - r.top - s(10.0)), brush);
                }
            }

            let weight = if selected { DWRITE_FONT_WEIGHT_SEMI_BOLD } else { DWRITE_FONT_WEIGHT_NORMAL };
            let set = g.text(localized(title), s(12.0), weight, 10_000.0)?;
            unsafe { brush.SetColor(&ui_color(text, 1.0)) };
            set.draw_centered(target, brush, (r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        }

        // LAT, RUS and CAPS, lit while that mode is on.
        let consw = if self.consw == 0xFF { 0 } else { self.consw };
        let on = [consw & 2 == 0, consw & 2 != 0, consw & 1 != 0];

        for (i, title) in ["LAT", "RUS", "CAPS"].iter().enumerate() {
            let r = l.pills[i];
            let pill = (r.bottom - r.top) / 2.0;

            unsafe {
                if on[i] {
                    brush.SetColor(&ui_color(text, 1.0));
                    target.FillRoundedRectangle(&rounded(r, pill), brush);
                } else {
                    brush.SetColor(&ui_color(text, 0.3));
                    target.DrawRoundedRectangle(&rounded(inset(r, 0.5), pill), brush, 1.0, None);
                }
            }

            let set = g.text(title, s(10.5), DWRITE_FONT_WEIGHT_BOLD, 10_000.0)?;
            unsafe { brush.SetColor(&ui_color(if on[i] { background } else { gray }, 1.0)) };
            set.draw_centered(target, brush, (r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        }

        Ok(())
    }

    // ---- Drawing --------------------------------------------------------------------

    fn render_target(&mut self) -> windows::core::Result<ID2D1HwndRenderTarget> {
        if let Some(target) = &self.target {
            return Ok(target.clone());
        }

        let (w, h) = self.client();
        let properties = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_UNKNOWN, alphaMode: D2D1_ALPHA_MODE_UNKNOWN },
            // Pixels: the window scales its sizes by its DPI itself.
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        // Immediately: EndDraw must not wait for the display's refresh, which
        // would hold up the main loop.
        let hwnd_properties = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd: self.hwnd,
            pixelSize: D2D_SIZE_U { width: w as u32, height: h as u32 },
            presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
        };
        let target = unsafe { self.graphics.d2d.CreateHwndRenderTarget(&properties, &hwnd_properties)? };

        unsafe {
            target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        }

        self.target = Some(target.clone());
        Ok(target)
    }

    fn draw(&mut self) -> windows::core::Result<()> {
        let hwnd_target = self.render_target()?;
        let target: &ID2D1RenderTarget = &hwnd_target;
        let labeled: Vec<bool> = (0..KEYS).map(|i| self.tag_shown(i)).collect();
        let picture = drawing::keyboard(&KeyboardState { down: &self.down, labeled: &labeled, hover: self.hover });
        let keyboard = self.keyboard_rect();

        unsafe {
            target.BeginDraw();
            let brush = target.CreateSolidColorBrush(&color(0), None)?;
            target.PushAxisAlignedClip(&keyboard, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            let drawn = replay(&self.graphics, target, &picture.bytes, keyboard, &self.keyboard_transform(), &|i| {
                self.tags.get(i as usize).cloned().flatten()
            });
            target.PopAxisAlignedClip();
            let drawn = drawn.and_then(|()| self.draw_strip(target, &brush));
            let ended = target.EndDraw(None, None);

            if ended.as_ref().is_err_and(|e| e.code() == D2DERR_RECREATE_TARGET) {
                self.target = None;
                self.invalidate();
                return Ok(());
            }

            drawn.and(ended)
        }
    }

    fn resized(&mut self) {
        let (w, h) = self.client();

        if let Some(target) = &self.target {
            let _ = unsafe { target.Resize(&D2D_SIZE_U { width: w as u32, height: h as u32 }) };
        }

        self.invalidate();
    }
}

fn inset(r: D2D_RECT_F, d: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: r.left + d, top: r.top + d, right: r.right - d, bottom: r.bottom - d }
}

/// Activates the emulator window if this one is active.
fn give_back_keyboard(hwnd: HWND) {
    unsafe {
        if GetActiveWindow() == hwnd
            && let Ok(owner) = GetWindow(hwnd, GW_OWNER)
        {
            let _ = SetActiveWindow(owner);
        }
    }
}

fn mouse_position(lparam: LPARAM) -> (f32, f32) {
    let point = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
    (point.x as f32, point.y as f32)
}

extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if theme::changed(msg, lparam) {
        theme::apply(hwnd);
        theme::redraw(hwnd);
    }

    match msg {
        // A click never takes the keyboard from the emulator window.
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        // The window is never active, but its title bar is drawn active while
        // the app is, as a tool window's is beside the window it serves.
        WM_ACTIVATEAPP => {
            APP_ACTIVE.set(wparam.0 != 0);
            unsafe { SendMessageW(hwnd, WM_NCACTIVATE, Some(WPARAM(wparam.0)), None) };
        }
        WM_NCACTIVATE => return unsafe { DefWindowProcW(hwnd, msg, WPARAM(APP_ACTIVE.get() as usize), lparam) },
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            unsafe { BeginPaint(hwnd, &mut ps) };

            if let Some(Err(e)) = with_window(KeyboardWindow::draw) {
                crate::log!("drawing the Keyboard window: {e}");
            }

            let _ = unsafe { EndPaint(hwnd, &ps) };
            return LRESULT(0);
        }
        WM_SIZE => {
            with_window(KeyboardWindow::resized);
        }
        // A press on the title bar or an edge moves or sizes the window.
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if begin_drag(hwnd, wparam.0 as u32) => return LRESULT(0),
        WM_MOUSEMOVE if drag(hwnd) => return LRESULT(0),
        WM_MOUSEMOVE => {
            let (x, y) = mouse_position(lparam);
            with_window(|w| w.mouse_move(x, y));
        }
        WM_MOUSELEAVE => {
            with_window(|w| {
                w.tracking = false;
                w.set_hover(None, None);
            });
        }
        WM_TIMER if wparam.0 == LINGER_TIMER => {
            with_window(KeyboardWindow::linger_over);
            return LRESULT(0);
        }
        WM_LBUTTONDOWN => {
            let (x, y) = mouse_position(lparam);
            with_window(|w| w.mouse_down(x, y));
            return LRESULT(0);
        }
        WM_LBUTTONUP if DRAG.get().is_some() => {
            end_drag(hwnd);
            let _ = unsafe { ReleaseCapture() };
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            let held = with_window(|w| {
                let held = w.held.is_some();
                w.mouse_up();
                held
            });

            if held == Some(true) {
                let _ = unsafe { ReleaseCapture() };
            }

            return LRESULT(0);
        }
        // The mouse taken away while a key is held: it is let go.
        WM_CAPTURECHANGED => {
            end_drag(hwnd);
            with_window(KeyboardWindow::mouse_up);
        }
        WM_SIZING => {
            let r = unsafe { &mut *(lparam.0 as *mut RECT) };
            keep_proportions(hwnd, wparam.0 as u32, r);
            return LRESULT(1);
        }
        WM_GETMINMAXINFO => {
            let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
            let (x, y) = min_size(hwnd);
            info.ptMinTrackSize = POINT { x, y };
            return LRESULT(0);
        }
        // Windows' move or size loop (the window menu's Move or Size)
        // activates the window, WS_EX_NOACTIVATE or not: the emulator window
        // gets the keyboard back once it is done, or at once for any other
        // activation.
        WM_ACTIVATE if (wparam.0 & 0xFFFF) as u32 != WA_INACTIVE => {
            if !SIZING.get() {
                give_back_keyboard(hwnd);
            }
        }
        WM_ENTERSIZEMOVE => SIZING.set(true),
        WM_EXITSIZEMOVE => {
            SIZING.set(false);
            save_frame(hwnd);
            give_back_keyboard(hwnd);
        }
        WM_DPICHANGED => {
            let dpi = ((wparam.0 >> 16) & 0xFFFF) as u32;
            let suggested = unsafe { &*(lparam.0 as *const RECT) };
            let mut frame = RECT::default();
            let _ = unsafe { AdjustWindowRectExForDpi(&mut frame, STYLE, false, EX_STYLE, dpi) };
            let width = suggested.right - suggested.left - (frame.right - frame.left);
            let height = client_height(width, dpi) + frame.bottom - frame.top;

            unsafe {
                let _ = SetWindowPos(hwnd, None, suggested.left, suggested.top, suggested.right - suggested.left, height, SWP_NOZORDER | SWP_NOACTIVATE);
            }

            // A move or size goes on from the new frame.
            if let Some(d) = DRAG.get() {
                let (mut cursor, mut frame) = (POINT::default(), RECT::default());

                unsafe {
                    let _ = GetCursorPos(&mut cursor);
                    let _ = GetWindowRect(hwnd, &mut frame);
                }

                DRAG.set(Some(Drag { cursor, frame, ..d }));
            }

            return LRESULT(0);
        }
        WM_CLOSE => {
            close(hwnd);
            return LRESULT(0);
        }
        // The window goes with the emulator window as the app quits. Its
        // Direct2D and DirectWrite objects are released now: left to the
        // thread-local's destructor, they were released at process exit,
        // after their DLLs had been torn down, and the app crashed.
        WM_NCDESTROY => {
            let state = WINDOW.with(|w| w.try_borrow_mut().ok().and_then(|mut w| w.take()));
            drop(state);
        }
        _ => {}
    }

    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

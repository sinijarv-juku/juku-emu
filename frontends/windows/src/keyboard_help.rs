//! Input → Keyboard Mapping (frontends/macos/src/KeyboardHelp.m): the PC
//! keys for Juku keys that are not where their legends suggest. Letters,
//! digits and the rest map by position and are not listed. Keep in step with
//! keymap.rs.
//!
//! The window draws its table itself; like Settings it is created once,
//! hidden when closed and owned by the emulator window.

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_BTNFACE, CreateFontIndirectW, DT_CALCRECT, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_WORDBREAK,
    DeleteObject, DrawTextW, EndPaint, FW_BOLD, FW_SEMIBOLD, FillRect, GetDC, GetMonitorInfoW, GetSysColorBrush, HDC, HFONT, HGDIOBJ,
    InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, PAINTSTRUCT, ReleaseDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow, SystemParametersInfoForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetActiveWindow, SetActiveWindow};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

use crate::strings::localized;
use crate::theme;

const CLASS_NAME: PCWSTR = w!("JukuKeyboardMapping");
const STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_CAPTION.0 | WS_SYSMENU.0);
const EX_STYLE: WINDOW_EX_STYLE = WINDOW_EX_STYLE(WS_EX_DLGMODALFRAME.0 | WS_EX_CONTROLPARENT.0);

/// PC key (localized where it has words), Juku key legend. The Mac's table
/// with the PC's names for its keys.
const ROWS: [(&str, &str); 17] = [
    ("[", "Ü"),
    ("]", "Õ"),
    (";", "Ö"),
    ("'", "Ä"),
    ("\\", "* :"),
    ("-", "− ="),
    ("=", "Ю @"),
    ("` and the key left of Z", "LAT/RUS"),
    ("Right Alt, keypad +", "; +"),
    ("Backspace", "←"),
    ("Delete", "DEL"),
    ("Home", "ERASE"),
    ("End", "{ ["),
    ("Page Down", "} ]"),
    ("Page Up", "\\ ^"),
    ("Caps Lock", "CAPS"),
    ("F1–F8", "F1–F8"),
];

const NOTE: &str = "Other keys map by position: letters, digits, punctuation, the arrows, Esc, Tab, Enter, Shift and Ctrl are where they are on the Juku keyboard. Left Alt shortcuts always go to the menus.";

struct Fonts {
    normal: HFONT,
    bold: HFONT,
    juku: HFONT,
}

impl Fonts {
    /// The system's message font (Segoe UI) at `dpi`, bold for the headings,
    /// and a semibold monospaced one for the Juku legends, as on the Mac.
    fn new(dpi: u32) -> Fonts {
        let mut metrics = NONCLIENTMETRICSW { cbSize: size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
        let _ = unsafe { SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS.0, metrics.cbSize, Some((&raw mut metrics).cast()), 0, dpi) };
        let normal = metrics.lfMessageFont;
        let mut bold = normal;
        bold.lfWeight = FW_BOLD.0 as i32;
        let mut juku = normal;
        juku.lfWeight = FW_SEMIBOLD.0 as i32;
        juku.lfFaceName = [0; 32];

        for (i, c) in "Consolas".encode_utf16().enumerate() {
            juku.lfFaceName[i] = c;
        }

        unsafe { Fonts { normal: CreateFontIndirectW(&normal), bold: CreateFontIndirectW(&bold), juku: CreateFontIndirectW(&juku) } }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        for font in [self.normal, self.bold, self.juku] {
            let _ = unsafe { DeleteObject(HGDIOBJ(font.0)) };
        }
    }
}

/// Where everything is drawn, in pixels of the client area.
#[derive(Default)]
struct Layout {
    host_x: i32,
    juku_x: i32,
    row_tops: Vec<i32>,
    row_height: i32,
    note: RECT,
    size: (i32, i32),
}

struct HelpWindow {
    hwnd: HWND,
    fonts: Fonts,
    layout: Layout,
}

thread_local! {
    static WINDOW: RefCell<Option<HelpWindow>> = const { RefCell::new(None) };
}

/// The window, if it has been created.
pub fn window() -> Option<HWND> {
    WINDOW.with(|w| w.try_borrow().ok().and_then(|w| w.as_ref().map(|w| w.hwnd)))
}

fn scale(dips: i32, dpi: u32) -> i32 {
    (dips as i64 * dpi as i64 / 96) as i32
}

/// `text` measured, or drawn, in `font` into `rect`.
fn draw(dc: HDC, font: HFONT, text: &str, rect: &mut RECT, format: windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT) {
    let mut wide: Vec<u16> = text.encode_utf16().collect();

    unsafe {
        let old = SelectObject(dc, HGDIOBJ(font.0));
        DrawTextW(dc, &mut wide, rect, format | DT_NOPREFIX);
        SelectObject(dc, old);
    }
}

fn measure(dc: HDC, font: HFONT, text: &str) -> (i32, i32) {
    let mut rect = RECT::default();
    draw(dc, font, text, &mut rect, DT_CALCRECT | DT_SINGLELINE);
    (rect.right, rect.bottom)
}

fn headers() -> [&'static str; 2] {
    [localized("PC"), localized("Juku")]
}

/// The two columns, with 24 DIPs between them, the note under them, 300
/// DIPs wide (KeyboardHelp.m).
fn compute_layout(hwnd: HWND, fonts: &Fonts) -> Layout {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let s = |dips: i32| scale(dips, dpi);
    let dc = unsafe { GetDC(Some(hwnd)) };
    let [host, juku] = headers();

    let host_width = ROWS.iter().map(|(h, _)| measure(dc, fonts.normal, localized(h)).0).chain([measure(dc, fonts.bold, host).0]).max();
    let row_height = measure(dc, fonts.normal, "Ag").1.max(measure(dc, fonts.juku, "Ag").1);
    let margin = s(20);
    let host_x = margin;
    let juku_x = host_x + host_width.unwrap_or(0) + s(24);
    let juku_width = ROWS.iter().map(|(_, j)| measure(dc, fonts.juku, j).0).chain([measure(dc, fonts.bold, juku).0]).max().unwrap_or(0);

    // The headings, a little apart, then a row per key.
    let mut row_tops = vec![margin];
    let mut y = margin + row_height + s(4) + s(4);

    for _ in ROWS {
        row_tops.push(y);
        y += row_height + s(4);
    }

    let mut note = RECT { left: margin, top: y - s(4) + s(16), right: margin + s(300), bottom: 0 };
    draw(dc, fonts.normal, localized(NOTE), &mut note, DT_CALCRECT | DT_WORDBREAK | DT_LEFT);
    note.right = note.left + s(300);
    unsafe { ReleaseDC(Some(hwnd), dc) };

    let width = (juku_x + juku_width).max(note.right) + margin;
    Layout { host_x, juku_x, row_tops, row_height, size: (width, note.bottom + margin), note }
}

/// Shows the window, creating it the first time, centred on the screen of
/// `owner`.
pub fn show(owner: HWND) {
    if let Some(hwnd) = window() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetActiveWindow(hwnd);
        }
        return;
    }

    let Ok(instance) = (unsafe { GetModuleHandleW(None) }) else {
        return;
    };

    let hwnd = unsafe {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        RegisterClassExW(&class);

        CreateWindowExW(
            EX_STYLE,
            CLASS_NAME,
            &HSTRING::from(localized("Keyboard Mapping")),
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
    };

    let Ok(hwnd) = hwnd else {
        return;
    };

    theme::apply(hwnd);

    let fonts = Fonts::new(unsafe { GetDpiForWindow(hwnd) });
    let layout = compute_layout(hwnd, &fonts);
    let size = layout.size;
    WINDOW.with_borrow_mut(|w| *w = Some(HelpWindow { hwnd, fonts, layout }));

    let (width, height) = window_size(hwnd, size);
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = unsafe { GetMonitorInfoW(MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST), &mut info) };
    let work = info.rcWork;
    let x = work.left + (work.right - work.left - width) / 2;
    let y = work.top + (work.bottom - work.top - height) / 2;

    unsafe {
        let _ = SetWindowPos(hwnd, None, x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
}

/// The window around a client area of `size`.
fn window_size(hwnd: HWND, size: (i32, i32)) -> (i32, i32) {
    let mut frame = RECT { left: 0, top: 0, right: size.0, bottom: size.1 };
    let _ = unsafe { AdjustWindowRectExForDpi(&mut frame, STYLE, false, EX_STYLE, GetDpiForWindow(hwnd)) };
    (frame.right - frame.left, frame.bottom - frame.top)
}

fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(hwnd, &mut ps) };

    WINDOW.with(|w| {
        let Ok(w) = w.try_borrow() else {
            return;
        };
        let Some(w) = w.as_ref() else {
            return;
        };
        let l = &w.layout;
        let cell = |x: i32, top: i32| RECT { left: x, top, right: x + l.size.0, bottom: top + l.row_height };
        let [host, juku] = headers();

        unsafe {
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, theme::text_color());
        }

        draw(dc, w.fonts.bold, host, &mut cell(l.host_x, l.row_tops[0]), DT_SINGLELINE);
        draw(dc, w.fonts.bold, juku, &mut cell(l.juku_x, l.row_tops[0]), DT_SINGLELINE);

        for (i, (host, juku)) in ROWS.iter().enumerate() {
            let top = l.row_tops[i + 1];
            draw(dc, w.fonts.normal, localized(host), &mut cell(l.host_x, top), DT_SINGLELINE);
            draw(dc, w.fonts.juku, juku, &mut cell(l.juku_x, top), DT_SINGLELINE);
        }

        unsafe { SetTextColor(dc, theme::gray_text_color()) };
        let mut note = l.note;
        draw(dc, w.fonts.normal, localized(NOTE), &mut note, DT_WORDBREAK | DT_LEFT);
    });

    let _ = unsafe { EndPaint(hwnd, &ps) };
}

/// Closing hides the window, so it opens again where it was.
fn close(hwnd: HWND) {
    let active = unsafe { GetActiveWindow() } == hwnd;

    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);

        if active && let Ok(owner) = GetWindow(hwnd, GW_OWNER) {
            let _ = SetActiveWindow(owner);
        }
    }
}

extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCLBUTTONDOWN {
        crate::emu_window::title_bar_pressed(hwnd, wparam, lparam);
    }

    if theme::changed(msg, lparam) {
        theme::apply(hwnd);
        theme::redraw(hwnd);
    }

    match msg {
        // The window's colour in the current mode (theme.rs).
        WM_ERASEBKGND => {
            let mut client = RECT::default();

            unsafe {
                let _ = GetClientRect(hwnd, &mut client);
                FillRect(HDC(wparam.0 as *mut _), &client, theme::window_brush());
            }

            return LRESULT(1);
        }
        WM_PAINT => {
            paint(hwnd);
            return LRESULT(0);
        }
        // Esc, through IsDialogMessageW.
        WM_COMMAND if (wparam.0 & 0xFFFF) as i32 == IDCANCEL.0 => {
            close(hwnd);
            return LRESULT(0);
        }
        WM_CLOSE => {
            close(hwnd);
            return LRESULT(0);
        }
        WM_DPICHANGED => {
            let dpi = ((wparam.0 >> 16) & 0xFFFF) as u32;
            let suggested = unsafe { &*(lparam.0 as *const RECT) };
            let fonts = Fonts::new(dpi);
            let layout = compute_layout(hwnd, &fonts);
            let (width, height) = window_size(hwnd, layout.size);

            WINDOW.with_borrow_mut(|w| {
                if let Some(w) = w.as_mut() {
                    w.fonts = fonts;
                    w.layout = layout;
                }
            });

            unsafe {
                let _ = SetWindowPos(hwnd, None, suggested.left, suggested.top, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
                let _ = InvalidateRect(Some(hwnd), None, true);
            }

            return LRESULT(0);
        }
        _ => {}
    }

    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

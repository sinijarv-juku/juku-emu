//! The Settings window (frontends/macos/src/SettingsWindow.m, spec §6.7,
//! §7.3a, §11.5): the Mac's panes, General (mouse, language and Windows'
//! appearance and file types), Display (the CRT), Sound (the key click and
//! the drive sounds) and Machine (the ROMs, the T3.2 clock and the
//! debugger), as the tabs of a tab control, each a simple form
//! built in code with no dialog template. Changes apply at once; the window
//! and the menus stay in step through `crate::settings_changed`, which calls
//! `refresh`.
//!
//! The window is created the first time it is shown and hidden when closed,
//! so it opens where it was, on the pane it showed. It is owned by the
//! emulator window, and the message loop gives it dialog keys
//! (`IsDialogMessageW`): Tab, Esc, and `pane_key`'s Ctrl+Tab.

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CLR_INVALID, COLOR_BTNFACE, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontIndirectW, CreateSolidBrush, DT_CALCRECT,
    DT_NOPREFIX, DT_WORDBREAK, BeginPaint, DeleteDC, DeleteObject, DrawFocusRect, DrawTextW, EndPaint, FillRect, GetDC, GetMonitorInfoW, GetPixel, GetSysColor, GetSysColorBrush, GetTextExtentPoint32W, HBRUSH, HDC, HFONT,
    HGDIOBJ, InvalidateRect, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT, MONITORINFO, MonitorFromWindow, ReleaseDC, SelectObject, SetBkColor, SetTextColor,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    BPBF_COMPATIBLEBITMAP, BeginBufferedPaint, CloseThemeData, EndBufferedPaint, DrawThemeBackground, ICC_BAR_CLASSES, ICC_STANDARD_CLASSES, ICC_TAB_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx,
    NM_CUSTOMDRAW, NMHDR, OpenThemeData, TABP_PANE, TB_ENDTRACK, TB_THUMBTRACK, TBM_SETLINESIZE, TBM_SETPAGESIZE, TBM_SETPOS, TBM_SETRANGEMAX,
    TBM_GETCHANNELRECT, TBM_SETRANGEMIN, TBS_HORZ, TBS_NOTICKS, TCIF_TEXT, TCITEMW, TCM_ADJUSTRECT, TCM_GETCURSEL, TCM_INSERTITEMW, TCM_SETCURSEL,
    TCN_SELCHANGE, TOOLTIPS_CLASSW, TRACKBAR_CLASSW, TTF_IDISHWND, TTF_SUBCLASS, TTM_ADDTOOLW, TTM_SETMAXTIPWIDTH, TTM_UPDATETIPTEXTW,
    TTTOOLINFOW, WC_TABCONTROLW,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow, SystemParametersInfoForDpi};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetActiveWindow, GetCapture, GetFocus, GetKeyState, IsWindowEnabled, SetActiveWindow, SetFocus, VK_CONTROL, VK_NEXT,
    VK_PRIOR, VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, PWSTR, w};

use crate::renderer::CrtSettings;
use crate::roms::{self, SYSTEM_ROMS};
use crate::settings::{self, *};
use crate::strings::localized;
use crate::theme::{self, ControlKind};

const CLASS_NAME: PCWSTR = w!("JukuSettings");
const STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_CAPTION.0 | WS_SYSMENU.0 | WS_CLIPCHILDREN.0);
const EX_STYLE: WINDOW_EX_STYLE = WINDOW_EX_STYLE(WS_EX_DLGMODALFRAME.0 | WS_EX_CONTROLPARENT.0);

// Static control styles, from a part of the windows crate the app does not
// otherwise need.
const SS_LEFT: u32 = 0x0;
const SS_RIGHT: u32 = 0x2;
const SS_NOPREFIX: u32 = 0x80;
const TBM_GETPOS: u32 = WM_USER;

/// The least width of the controls' column, in device-independent pixels,
/// to which the notes under them wrap.
const NOTE_WIDTH: i32 = 260;

/// A slider's positions: its range is cut into this many steps.
const SLIDER_STEPS: i32 = 1000;

/// What a row of the form holds.
#[derive(Clone, Copy, PartialEq)]
enum Control {
    /// A setting on or off, with its default.
    Check(&'static str, &'static str, bool),
    /// A setting from `min` to `max`.
    Slider(&'static str, f64, f64),
    RestoreCrt,
    SystemRom,
    ExpansionRom,
    Language,
    /// Light, dark or as Windows is set (Windows only).
    Appearance,
    Note(&'static str),
    /// Register File Types (Windows only), and the note that says whether
    /// they open in this copy of Juku.
    RegisterFiles,
    FilesNote,
    /// The note under Accept Connections, which names the port.
    DebuggerNote,
}

const FILES_NOTE: &str = "Lets Explorer open .juk images and .jukusnap snapshots in Juku, and show snapshots' pictures.";
const FILES_REGISTERED_NOTE: &str = "Explorer opens .juk images and .jukusnap snapshots in this copy of Juku.";
const DEBUGGER_NOTE: &str = "VS Code and other DAP clients connect on port %ld, from this PC only.";

struct Row {
    pane: usize,
    label: Option<&'static str>,
    control: Control,
    /// Space above the row on top of the usual, in device-independent
    /// pixels: groups stand apart, a note keeps close to its control.
    extra: i32,
}

/// The panes, in the order of the Mac's toolbar; their names are the tabs'.
const PANES: [&str; 4] = ["General", "Display", "Sound", "Machine"];
const GENERAL: usize = 0;
const DISPLAY: usize = 1;
const SOUND: usize = 2;
const MACHINE: usize = 3;

const KEY_CLICK_NOTE: &str = "This ROM's key click cannot be changed.";
const SPREAD_NOTE: &str = "The two drives sat side by side in the disk unit under the monitor, A: on the left. The default places them as heard from in front of the monitor.";

// The rows of SettingsWindow.m's panes, in its order, with Windows' own at
// the end of General.
const ROWS: [Row; 32] = [
    Row { pane: GENERAL, label: Some("Mouse Speed:"), control: Control::Slider(MOUSE_SENSITIVITY, 0.1, 2.0), extra: 0 },
    Row { pane: GENERAL, label: Some("Language:"), control: Control::Language, extra: 12 },
    Row { pane: GENERAL, label: None, control: Control::Note("Takes effect the next time Juku opens."), extra: -4 },
    Row { pane: GENERAL, label: Some("Appearance:"), control: Control::Appearance, extra: 12 },
    Row { pane: GENERAL, label: Some("Files:"), control: Control::RegisterFiles, extra: 12 },
    Row { pane: GENERAL, label: None, control: Control::FilesNote, extra: -4 },
    Row { pane: DISPLAY, label: None, control: Control::Check(CRT_EFFECTS, "CRT Effects", true), extra: 0 },
    Row { pane: DISPLAY, label: Some("Curvature:"), control: Control::Slider(CRT_CURVATURE, 0.0, 0.12), extra: 0 },
    Row { pane: DISPLAY, label: Some("Scanlines:"), control: Control::Slider(CRT_SCANLINES, 0.0, 1.0), extra: 0 },
    Row { pane: DISPLAY, label: Some("Glow:"), control: Control::Slider(CRT_GLOW, 0.0, 0.4), extra: 0 },
    Row { pane: DISPLAY, label: Some("Background:"), control: Control::Slider(CRT_BACKGROUND, 0.0, 1.0), extra: 0 },
    Row { pane: DISPLAY, label: Some("Frame:"), control: Control::Slider(CRT_FRAME, 0.0, 1.0), extra: 0 },
    Row { pane: DISPLAY, label: Some("Room Light:"), control: Control::Slider(CRT_ROOM, 0.0, 1.0), extra: 0 },
    Row { pane: DISPLAY, label: None, control: Control::Check(CRT_BEZEL_3D, "3D Frame", true), extra: 0 },
    Row { pane: DISPLAY, label: None, control: Control::Check(CRT_MONITOR_FRONT, "Whole Monitor", false), extra: 0 },
    Row { pane: DISPLAY, label: Some("Brightness:"), control: Control::Slider(CRT_BRIGHTNESS, 0.5, 1.5), extra: 0 },
    Row { pane: DISPLAY, label: None, control: Control::RestoreCrt, extra: 12 },
    Row { pane: SOUND, label: Some("Keyboard:"), control: Control::Check(KEY_CLICK, "Key Click", true), extra: 0 },
    Row { pane: SOUND, label: None, control: Control::Note(KEY_CLICK_NOTE), extra: -4 },
    Row { pane: SOUND, label: Some("Disk Drives:"), control: Control::Check(DRIVE_SOUNDS, "Drive Sounds", true), extra: 12 },
    Row { pane: SOUND, label: Some("Drive Volume:"), control: Control::Slider(DRIVE_SOUNDS_VOLUME, 0.0, 2.0), extra: 0 },
    Row { pane: SOUND, label: Some("Drive Separation:"), control: Control::Slider(DRIVE_SOUNDS_SPREAD, 0.0, 1.0), extra: 0 },
    Row { pane: SOUND, label: None, control: Control::Note(SPREAD_NOTE), extra: -4 },
    // The note goes with the slider above it, not with the checkbox below.
    Row { pane: SOUND, label: None, control: Control::Check(DRIVE_IDLE_MUTE, "Fade Out the Motor When Idle for 10 s", false), extra: 4 },
    Row { pane: MACHINE, label: Some("System ROM:"), control: Control::SystemRom, extra: 0 },
    Row { pane: MACHINE, label: None, control: Control::Note("Changing the ROM restarts the machine."), extra: -4 },
    Row { pane: MACHINE, label: Some("Expansion ROM:"), control: Control::ExpansionRom, extra: 12 },
    Row { pane: MACHINE, label: None, control: Control::Note("Start it with A at the * prompt. Changing it restarts the machine."), extra: -4 },
    Row { pane: MACHINE, label: Some("Timer T3.2:"), control: Control::Check(T32_FAST_CLOCK, "Clock at 1.23 MHz", false), extra: 12 },
    Row { pane: MACHINE, label: None, control: Control::Note("RDEM needs it, but the RomBios stopwatch then runs too fast."), extra: -4 },
    Row { pane: MACHINE, label: Some("Debugger:"), control: Control::Check(ACCEPT_DEBUGGER, "Accept Connections", false), extra: 12 },
    Row { pane: MACHINE, label: None, control: Control::DebuggerNote, extra: -4 },
];

/// The note under Accept Connections, with the port that `DebuggerPort`
/// sets (spec §12.3).
fn debugger_note() -> String {
    let port = settings::debugger_port().map_or_else(String::new, |p| p.to_string());
    localized(DEBUGGER_NOTE).replace("%ld", &port)
}

/// The row that holds `control`.
fn row_of(control: Control) -> usize {
    ROWS.iter().position(|r| r.control == control).unwrap_or(0)
}

/// The control of row `row` has the ID FIRST_ID + row, and its label
/// FIRST_LABEL_ID + row.
const FIRST_ID: i32 = 100;
const FIRST_LABEL_ID: i32 = 200;
const TAB_ID: i32 = 99;

/// Language codes in the Language list; None is "System Default". The
/// languages are named in their own language, as Windows and macOS do.
const LANGUAGES: [(Option<&str>, &str); 3] = [(None, "System Default"), (Some("en"), "English"), (Some("et"), "Eesti")];

/// A setting's default, for the sliders.
fn default_value(key: &str) -> f64 {
    let d = CrtSettings::default();

    (match key {
        CRT_CURVATURE => d.curvature,
        CRT_SCANLINES => d.scanlines,
        CRT_GLOW => d.glow,
        CRT_BACKGROUND => d.background,
        CRT_FRAME => d.frame,
        CRT_ROOM => d.room,
        CRT_BRIGHTNESS => d.brightness,
        DRIVE_SOUNDS_VOLUME => 1.0,
        DRIVE_SOUNDS_SPREAD => juku::drive_sound::SPREAD,
        _ => DEFAULT_MOUSE_SENSITIVITY as f32,
    }) as f64
}

/// What an item of the System ROM or Expansion ROM list stands for.
#[derive(Clone, PartialEq)]
enum RomItem {
    /// The ROM named so (a bundled ROM's name or a path); None for no
    /// expansion ROM.
    Rom(Option<String>),
    /// Other…: the open dialog.
    Other,
}

struct SettingsWindow {
    hwnd: HWND,
    tab: HWND,
    /// The pane shown, which the window keeps while it is hidden.
    pane: usize,
    labels: Vec<Option<HWND>>,
    controls: Vec<HWND>,
    tooltip: HWND,
    font: HFONT,
    note_font: HFONT,
    system_roms: Vec<RomItem>,
    expansion_roms: Vec<RomItem>,
    key_click_available: bool,
}

thread_local! {
    static WINDOW: RefCell<Option<SettingsWindow>> = const { RefCell::new(None) };
    // The control that had the keyboard when the window was last active.
    static LAST_FOCUS: Cell<isize> = const { Cell::new(0) };
    // The tab control's page, in client coordinates, and the brush and colour
    // of its background, on which the controls stand. Not in WINDOW, which
    // is borrowed while the controls are placed and ask for their colours.
    static PAGE: Cell<RECT> = const { Cell::new(RECT { left: 0, top: 0, right: 0, bottom: 0 }) };
    static PAGE_BRUSH: Cell<isize> = const { Cell::new(0) };
    static PAGE_COLOR: Cell<u32> = const { Cell::new(0) };
}

/// The window, if it has been created.
pub fn window() -> Option<HWND> {
    WINDOW.with(|w| w.try_borrow().ok().and_then(|w| w.as_ref().map(|w| w.hwnd)))
}

fn send(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))) }.0
}

fn scale(dips: i32, dpi: u32) -> i32 {
    (dips as i64 * dpi as i64 / 96) as i32
}

/// The message font of the system at `dpi` (Segoe UI), and a smaller one
/// for the notes, as the Mac uses the small system font for them.
pub(crate) fn create_fonts(dpi: u32) -> (HFONT, HFONT) {
    let mut metrics = NONCLIENTMETRICSW { cbSize: size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
    let _ = unsafe { SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS.0, metrics.cbSize, Some((&raw mut metrics).cast()), 0, dpi) };
    let mut font = metrics.lfMessageFont;
    let normal = unsafe { CreateFontIndirectW(&font) };
    font.lfHeight = font.lfHeight * 11 / 12;
    (normal, unsafe { CreateFontIndirectW(&font) })
}

/// The size of `text` in `font`, in pixels.
fn text_size(dc: HDC, font: HFONT, text: &str) -> (i32, i32) {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE::default();

    unsafe {
        let old = SelectObject(dc, HGDIOBJ(font.0));
        let _ = GetTextExtentPoint32W(dc, &wide, &mut size);
        SelectObject(dc, old);
    }

    (size.cx, size.cy)
}

/// The height of `text` in `font` wrapped at words to `width` pixels.
fn wrapped_height(dc: HDC, font: HFONT, text: &str, width: i32) -> i32 {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let mut rect = RECT { left: 0, top: 0, right: width, bottom: 0 };

    unsafe {
        let old = SelectObject(dc, HGDIOBJ(font.0));
        DrawTextW(dc, &mut wide, &mut rect, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX);
        SelectObject(dc, old);
    }

    rect.bottom - rect.top
}

/// A slider is wider than the column by its channel's insets (`layout`),
/// and a trackbar's focus rectangle goes round all of it, up to the label.
/// So the trackbar's own is kept hidden, and the slider draws one round its
/// channel, which the thumb stays within, when its window shows focus.
///
/// It is also painted off screen and copied in one go. A trackbar erases
/// its background with its class's light brush before it paints the page's
/// color over it, which flashed in dark mode as a pane's sliders were shown.
extern "system" fn slider_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    match msg {
        WM_UPDATEUISTATE => unsafe {
            let result = DefSubclassProc(hwnd, msg, wparam, lparam);
            DefSubclassProc(hwnd, msg, WPARAM((UISF_HIDEFOCUS << 16 | UIS_SET) as usize), LPARAM(0));
            let _ = InvalidateRect(Some(hwnd), None, false);
            result
        },
        WM_SETFOCUS | WM_KILLFOCUS => unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
            DefSubclassProc(hwnd, msg, wparam, lparam)
        },
        // The background is painted with the rest, in WM_PAINT.
        WM_ERASEBKGND => LRESULT(1),
        // The trackbar paints into the DC it is given, its background too.
        WM_PAINT => unsafe {
            let mut paint = PAINTSTRUCT::default();
            let target = BeginPaint(hwnd, &mut paint);
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let mut buffered = HDC::default();
            let buffer = BeginBufferedPaint(target, &client, BPBF_COMPATIBLEBITMAP, None, &mut buffered);
            // Straight to the window if no buffer could be had.
            let dc = if buffer != 0 { buffered } else { target };
            DefSubclassProc(hwnd, WM_PAINT, WPARAM(dc.0 as usize), LPARAM(0));
            let parent = GetParent(hwnd).unwrap_or_default();
            let cues = SendMessageW(parent, WM_QUERYUISTATE, None, None).0 as u32;

            if GetFocus() == hwnd && cues & UISF_HIDEFOCUS == 0 {
                let mut channel = RECT::default();
                send(hwnd, TBM_GETCHANNELRECT, 0, (&raw mut channel) as isize);
                let gap = scale(2, GetDpiForWindow(hwnd));
                let rect = RECT { left: channel.left - gap, top: client.top, right: channel.right + gap, bottom: client.bottom };
                let _ = DrawFocusRect(dc, &rect);
            }

            if buffer != 0 {
                let _ = EndBufferedPaint(buffer, true);
            }

            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        },
        _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    }
}

fn create_control(parent: HWND, class: PCWSTR, text: &str, style: u32, id: i32) -> HWND {
    let instance = unsafe { GetModuleHandleW(None) }.ok();

    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            &HSTRING::from(text),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | style),
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(id as isize as *mut _)),
            instance.map(Into::into),
            None,
        )
    }
    .unwrap_or_default()
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

    let Some(hwnd) = create(owner) else {
        return;
    };

    refresh();
    layout(hwnd, true);

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
}

fn create(owner: HWND) -> Option<HWND> {
    let instance = unsafe { GetModuleHandleW(None) }.ok()?;
    let controls = INITCOMMONCONTROLSEX {
        dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_BAR_CLASSES | ICC_STANDARD_CLASSES | ICC_TAB_CLASSES,
    };

    unsafe {
        let _ = InitCommonControlsEx(&controls);
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        RegisterClassExW(&class);
    }

    let hwnd = unsafe {
        CreateWindowExW(
            EX_STYLE,
            CLASS_NAME,
            &HSTRING::from(localized("Settings")),
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

    let mut labels = Vec::new();
    let mut controls = Vec::new();

    for (row, spec) in ROWS.iter().enumerate() {
        let id = FIRST_ID + row as i32;
        labels.push(spec.label.map(|label| create_control(hwnd, w!("STATIC"), localized(label), SS_RIGHT | SS_NOPREFIX, FIRST_LABEL_ID + row as i32)));

        let control = match spec.control {
            Control::Check(_, title, _) => {
                create_control(hwnd, w!("BUTTON"), localized(title), BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0, id)
            }
            Control::Slider(..) => {
                let slider = create_control(hwnd, TRACKBAR_CLASSW, "", TBS_HORZ | TBS_NOTICKS | WS_TABSTOP.0, id);
                send(slider, TBM_SETRANGEMIN, 0, 0);
                send(slider, TBM_SETRANGEMAX, 1, SLIDER_STEPS as isize);
                send(slider, TBM_SETLINESIZE, 0, (SLIDER_STEPS / 100) as isize);
                send(slider, TBM_SETPAGESIZE, 0, (SLIDER_STEPS / 10) as isize);
                let _ = unsafe { SetWindowSubclass(slider, Some(slider_proc), 1, 0) };
                send(slider, WM_UPDATEUISTATE, (UISF_HIDEFOCUS << 16 | UIS_SET) as usize, 0);
                slider
            }
            Control::RestoreCrt => {
                create_control(hwnd, w!("BUTTON"), localized("Restore CRT Defaults"), BS_PUSHBUTTON as u32 | WS_TABSTOP.0, id)
            }
            Control::SystemRom | Control::ExpansionRom | Control::Language | Control::Appearance => {
                create_control(hwnd, w!("COMBOBOX"), "", CBS_DROPDOWNLIST as u32 | WS_VSCROLL.0 | WS_TABSTOP.0, id)
            }
            Control::Note(text) => create_control(hwnd, w!("STATIC"), localized(text), SS_LEFT | SS_NOPREFIX, id),
            Control::RegisterFiles => {
                create_control(hwnd, w!("BUTTON"), localized("Register File Types"), BS_PUSHBUTTON as u32 | WS_TABSTOP.0, id)
            }
            Control::FilesNote | Control::DebuggerNote => create_control(hwnd, w!("STATIC"), "", SS_LEFT | SS_NOPREFIX, id),
        };
        controls.push(control);
    }

    // The ROM lists show the file's path as a tooltip, as the Mac's items do.
    let tooltip = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST,
            TOOLTIPS_CLASSW,
            PCWSTR::null(),
            WS_POPUP,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            Some(hwnd),
            None,
            Some(instance.into()),
            None,
        )
    }
    .unwrap_or_default();

    // Last, so it is below the controls: they take the clicks on its page,
    // and the keyboard reaches it after them, as in a property sheet.
    let tab = create_control(hwnd, WC_TABCONTROLW, "", WS_CLIPSIBLINGS.0 | WS_TABSTOP.0, TAB_ID);

    for (index, name) in PANES.iter().enumerate() {
        let mut text: Vec<u16> = localized(name).encode_utf16().chain(Some(0)).collect();
        let item = TCITEMW { mask: TCIF_TEXT, pszText: PWSTR(text.as_mut_ptr()), ..Default::default() };
        send(tab, TCM_INSERTITEMW, index, (&raw const item) as isize);
    }

    set_page_color(tab);

    for row in [row_of(Control::SystemRom), row_of(Control::ExpansionRom)] {
        let mut tool = tool_info(hwnd, controls[row]);
        send(tooltip, TTM_ADDTOOLW, 0, (&raw mut tool) as isize);
    }

    send(tooltip, TTM_SETMAXTIPWIDTH, 0, 600);

    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let (font, note_font) = create_fonts(dpi);

    WINDOW.with_borrow_mut(|w| {
        *w = Some(SettingsWindow {
            hwnd,
            tab,
            pane: GENERAL,
            labels,
            controls,
            tooltip,
            font,
            note_font,
            system_roms: Vec::new(),
            expansion_roms: Vec::new(),
            key_click_available: true,
        });
    });

    theme::paint_tab_control_dark(tab);
    apply_theme();
    set_fonts();
    show_pane(GENERAL, false);
    Some(hwnd)
}

/// The window and its controls in the current mode, light or dark
/// (theme.rs).
fn apply_theme() {
    WINDOW.with_borrow(|w| {
        let Some(w) = w else {
            return;
        };

        theme::apply(w.hwnd);

        for (row, spec) in ROWS.iter().enumerate() {
            let kind = match spec.control {
                Control::Check(..) | Control::RestoreCrt | Control::RegisterFiles => ControlKind::Explorer,
                Control::SystemRom | Control::ExpansionRom | Control::Language | Control::Appearance => ControlKind::ComboBox,
                Control::Slider(..) | Control::Note(_) | Control::FilesNote | Control::DebuggerNote => ControlKind::Plain,
            };
            theme::apply_control(w.controls[row], kind);
        }

        theme::apply_control(w.tooltip, ControlKind::Explorer);
        set_page_color(w.tab);
    });
}

/// The background of the tab control's page in the current theme, for the
/// controls on it (in Windows 10 and 11 white, where the window is gray),
/// as `EnableThemeDialogTexture` gives a property sheet's pages. It is read
/// from the theme's drawing of the page; without visual styles it is the
/// window's colour. In dark mode it is the page colour of theme.rs.
fn set_page_color(tab: HWND) {
    let mut color = unsafe { GetSysColor(COLOR_BTNFACE) };

    if theme::dark() {
        color = theme::PAGE.0;
    } else {
        unsafe {
            let theme = OpenThemeData(Some(tab), w!("TAB"));

            if !theme.is_invalid() {
                let screen = GetDC(None);
                let dc = CreateCompatibleDC(Some(screen));
                let bitmap = CreateCompatibleBitmap(screen, 32, 32);
                let old = SelectObject(dc, HGDIOBJ(bitmap.0));
                let rect = RECT { left: 0, top: 0, right: 32, bottom: 32 };

                if DrawThemeBackground(theme, dc, TABP_PANE.0, 0, &rect, None).is_ok() {
                    let pixel = GetPixel(dc, 16, 16);

                    if pixel.0 != CLR_INVALID {
                        color = pixel.0;
                    }
                }

                SelectObject(dc, old);
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
                let _ = DeleteDC(dc);
                ReleaseDC(None, screen);
                let _ = CloseThemeData(theme);
            }
        }
    }

    let brush = unsafe { CreateSolidBrush(COLORREF(color)) };
    let old = PAGE_BRUSH.replace(brush.0 as isize);
    PAGE_COLOR.set(color);

    if old != 0 {
        let _ = unsafe { DeleteObject(HGDIOBJ(old as *mut _)) };
    }
}

/// Whether row `row` is shown: it is on the pane shown, and is not the Key
/// Click note while the key click can be changed.
fn row_visible(w: &SettingsWindow, row: usize) -> bool {
    ROWS[row].pane == w.pane && !(ROWS[row].control == Control::Note(KEY_CLICK_NOTE) && w.key_click_available)
}

fn show_rows(w: &SettingsWindow) {
    for row in 0..ROWS.len() {
        let show = if row_visible(w, row) { SW_SHOWNA } else { SW_HIDE };

        unsafe {
            let _ = ShowWindow(w.controls[row], show);

            if let Some(label) = w.labels[row] {
                let _ = ShowWindow(label, show);
            }
        }
    }
}

/// Shows pane `pane`. With `focus` (Ctrl+Tab) the keyboard goes to its
/// first control, unless the tabs have it; otherwise it moves only off a
/// control that is now hidden, to the tabs.
fn show_pane(pane: usize, focus: bool) {
    // Before hiding it, which may hand the keyboard to the window.
    let current = unsafe { GetFocus() };

    let Some((hwnd, tab)) = WINDOW.with_borrow_mut(|w| {
        let w = w.as_mut()?;
        w.pane = pane;
        send(w.tab, TCM_SETCURSEL, pane, 0);
        show_rows(w);
        Some((w.hwnd, w.tab))
    }) else {
        return;
    };

    unsafe {
        let hidden = IsChild(hwnd, current).as_bool() && !IsWindowVisible(current).as_bool();

        if focus && current != tab {
            let _ = SetFocus(GetNextDlgTabItem(hwnd, None, false).ok());
        } else if hidden {
            let _ = SetFocus(Some(tab));
        }
    }
}

/// Ctrl+Tab and Ctrl+Page Down show the next pane, Ctrl+Shift+Tab and
/// Ctrl+Page Up the one before, as in a property sheet. True if `msg` was
/// one of them.
pub fn pane_key(msg: &MSG) -> bool {
    if msg.message != WM_KEYDOWN || unsafe { GetKeyState(VK_CONTROL.0 as i32) } >= 0 {
        return false;
    }

    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
    let back = match msg.wParam.0 as u16 {
        key if key == VK_TAB.0 => shift,
        key if key == VK_NEXT.0 => false,
        key if key == VK_PRIOR.0 => true,
        _ => return false,
    };

    let Some(pane) = WINDOW.with_borrow(|w| w.as_ref().map(|w| w.pane)) else {
        return false;
    };

    show_pane(if back { (pane + PANES.len() - 1) % PANES.len() } else { (pane + 1) % PANES.len() }, true);
    true
}

fn tool_info(parent: HWND, control: HWND) -> TTTOOLINFOW {
    TTTOOLINFOW {
        cbSize: size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_IDISHWND | TTF_SUBCLASS,
        hwnd: parent,
        uId: control.0 as usize,
        lpszText: PWSTR::null(),
        ..Default::default()
    }
}

fn set_fonts() {
    WINDOW.with_borrow(|w| {
        let Some(w) = w else {
            return;
        };

        for (row, spec) in ROWS.iter().enumerate() {
            let font = if matches!(spec.control, Control::Note(_) | Control::FilesNote | Control::DebuggerNote) { w.note_font } else { w.font };
            send(w.controls[row], WM_SETFONT, font.0 as usize, 1);

            if let Some(label) = w.labels[row] {
                send(label, WM_SETFONT, w.font.0 as usize, 1);
            }
        }

        send(w.tab, WM_SETFONT, w.font.0 as usize, 1);
    });
}

/// Places each pane's controls in two columns, labels on the right of the
/// first, on the tab control's page, and fits the window to the largest
/// pane, so it keeps its size from tab to tab, as a property sheet does. The
/// first time it is centred on its screen; after that its title bar stays
/// in place.
fn layout(hwnd: HWND, center: bool) {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let s = |dips: i32| scale(dips, dpi);

    let Some((width, height)) = WINDOW.with_borrow(|w| {
        let w = w.as_ref()?;
        let dc = unsafe { GetDC(Some(hwnd)) };
        let text = |font: HFONT, key: &str| text_size(dc, font, key);
        let (_, text_height) = text(w.font, "Ag");

        let label_width = ROWS.iter().filter_map(|r| r.label).map(|l| text(w.font, localized(l)).0).max().unwrap_or(0);
        let combo_width = |items: &[String]| items.iter().map(|i| text(w.font, i).0).max().unwrap_or(0) + s(40);

        let mut sizes = Vec::new();

        for (row, spec) in ROWS.iter().enumerate() {
            let control = w.controls[row];
            let size = match spec.control {
                Control::Check(_, title, _) => (text(w.font, localized(title)).0 + s(24), text_height + s(4)),
                Control::Slider(..) => (s(220), s(26)),
                Control::RestoreCrt => ((text(w.font, localized("Restore CRT Defaults")).0 + s(24)).max(s(88)), text_height + s(10)),
                Control::SystemRom | Control::ExpansionRom | Control::Language | Control::Appearance => {
                    let count = send(control, CB_GETCOUNT, 0, 0).max(0) as usize;
                    let items: Vec<String> = (0..count).map(|i| combo_text(control, i)).collect();
                    let mut frame = RECT::default();
                    let _ = unsafe { GetWindowRect(control, &mut frame) };
                    // A combo box is as tall as its font makes it.
                    let height = if frame.bottom > frame.top { frame.bottom - frame.top } else { text_height + s(10) };
                    (combo_width(&items).max(s(220)), height)
                }
                Control::RegisterFiles => ((text(w.font, localized("Register File Types")).0 + s(24)).max(s(88)), text_height + s(10)),
                // Below, once the column's width is known.
                Control::Note(_) | Control::FilesNote | Control::DebuggerNote => (0, 0),
            };
            sizes.push(size);
        }

        // The column is as wide as its widest control, or NOTE_WIDTH, and the
        // notes wrap to it. Files' note is as tall as the taller of its two
        // texts, so the form does not move when it changes.
        let column_width = sizes.iter().map(|&(w, _)| w).max().unwrap_or(0).max(s(NOTE_WIDTH));

        for (row, spec) in ROWS.iter().enumerate() {
            let notes: Vec<String> = match spec.control {
                Control::Note(note) => vec![localized(note).to_string()],
                Control::FilesNote => vec![localized(FILES_NOTE).to_string(), localized(FILES_REGISTERED_NOTE).to_string()],
                Control::DebuggerNote => vec![debugger_note()],
                _ => continue,
            };
            let height = notes.iter().map(|n| wrapped_height(dc, w.note_font, n, column_width)).max().unwrap_or(0);
            sizes[row] = (column_width, height);
        }

        unsafe { ReleaseDC(Some(hwnd), dc) };

        // Each row's top in its pane's form, and the forms' size.
        let column = label_width + s(8);
        let mut bottoms = [0; PANES.len()];
        let mut tops = [0; ROWS.len()];

        for (row, spec) in ROWS.iter().enumerate() {
            if spec.control == Control::Note(KEY_CLICK_NOTE) && w.key_click_available {
                continue;
            }

            if bottoms[spec.pane] > 0 {
                bottoms[spec.pane] += s(8 + spec.extra);
            }

            tops[row] = bottoms[spec.pane];
            bottoms[spec.pane] += sizes[row].1.max(text_height);
        }

        // The tab control around its page, which has the forms' margins.
        let margin = s(12);
        let inside = s(20);
        let (page_width, page_height) = (column + column_width + 2 * inside, bottoms.iter().copied().max().unwrap_or(0) + 2 * inside);
        let mut tab = RECT { left: 0, top: 0, right: page_width, bottom: page_height };
        send(w.tab, TCM_ADJUSTRECT, 1, (&raw mut tab) as isize);
        let (left, top) = (margin - tab.left, margin - tab.top);
        PAGE.set(RECT { left, top, right: left + page_width, bottom: top + page_height });

        for (row, spec) in ROWS.iter().enumerate() {
            let (width, height) = sizes[row];
            let row_height = height.max(text_height);
            let y = top + inside + tops[row];
            let combo = matches!(spec.control, Control::SystemRom | Control::ExpansionRom | Control::Language);
            // A combo box's height includes its list.
            let list = if combo { s(200) } else { height };
            let x = left + inside;

            let control = w.controls[row];
            let label_top = y + (row_height - text_height) / 2;

            unsafe {
                let _ = SetWindowPos(control, None, x + column, y + (row_height - height) / 2, width, list, SWP_NOZORDER | SWP_NOACTIVATE);

                // A trackbar draws its channel above its middle, leaving
                // room for tick marks below even without them: the channel
                // goes level with the label's middle instead. The channel is
                // also inset from the trackbar's sides, by about half the
                // thumb: the trackbar is widened by the insets, so that the
                // channel spans the column as the other controls do.
                if let Control::Slider(..) = spec.control {
                    let mut channel = RECT::default();
                    send(control, TBM_GETCHANNELRECT, 0, (&raw mut channel) as isize);
                    let (inset_left, inset_right) = (channel.left, width - channel.right);
                    let top = label_top + text_height / 2 - (channel.top + channel.bottom) / 2;
                    let (left, width) = (x + column - inset_left, width + inset_left + inset_right);
                    let _ = SetWindowPos(control, None, left, top, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
                }

                if let Some(label) = w.labels[row] {
                    let _ = SetWindowPos(label, None, x, label_top, label_width, text_height, SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
        }

        let (width, height) = (tab.right - tab.left, tab.bottom - tab.top);
        let _ = unsafe { SetWindowPos(w.tab, None, margin, margin, width, height, SWP_NOZORDER | SWP_NOACTIVATE) };
        Some((width + 2 * margin, height + 2 * margin))
    }) else {
        return;
    };

    let mut frame = RECT { left: 0, top: 0, right: width, bottom: height };
    let _ = unsafe { AdjustWindowRectExForDpi(&mut frame, STYLE, false, EX_STYLE, dpi) };
    let (width, height) = (frame.right - frame.left, frame.bottom - frame.top);

    let mut current = RECT::default();
    let _ = unsafe { GetWindowRect(hwnd, &mut current) };
    let (mut x, mut y) = (current.left, current.top);

    if center {
        let owner = unsafe { GetWindow(hwnd, GW_OWNER) }.unwrap_or(hwnd);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = unsafe { GetMonitorInfoW(MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST), &mut info) };
        let work = info.rcWork;
        x = work.left + (work.right - work.left - width) / 2;
        y = work.top + (work.bottom - work.top - height) / 2;
    }

    let _ = unsafe { SetWindowPos(hwnd, None, x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE) };
}

fn combo_text(combo: HWND, index: usize) -> String {
    let len = send(combo, CB_GETLBTEXTLEN, index, 0).max(0) as usize;
    let mut text = vec![0u16; len + 1];
    send(combo, CB_GETLBTEXT, index, text.as_mut_ptr() as isize);
    String::from_utf16_lossy(&text[..len])
}

fn fill_combo(combo: HWND, items: &[String], selected: usize) {
    send(combo, CB_RESETCONTENT, 0, 0);

    for item in items {
        let text = HSTRING::from(item.as_str());
        send(combo, CB_ADDSTRING, 0, text.as_ptr() as isize);
    }

    send(combo, CB_SETCURSEL, selected, 0);
}

/// The last ROM file of `key`, as a list item: its file name, and the path.
fn rom_file(key: &str) -> Option<(String, String)> {
    let path = settings::string_setting(key)?;
    let name = std::path::Path::new(&path).file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
    Some((name, path))
}

/// Brings the controls in line with the settings (the Mac's
/// defaultsChanged:).
pub fn refresh() {
    let key_click_available = crate::emu_window::with_emu(|emu| emu.roms().key_click_available());

    let relayout = WINDOW.with(|w| {
        let Ok(mut w) = w.try_borrow_mut() else {
            return None;
        };
        let w = w.as_mut()?;
        let dragged = unsafe { GetCapture() };

        for (row, spec) in ROWS.iter().enumerate() {
            let control = w.controls[row];

            match spec.control {
                Control::Check(key, _, default) => {
                    send(control, BM_SETCHECK, settings::bool_setting(key, default) as usize, 0);
                }
                Control::Slider(key, min, max) if control != dragged => {
                    let value = settings::f64_setting(key, default_value(key));
                    let position = ((value - min) / (max - min) * SLIDER_STEPS as f64).round() as isize;
                    send(control, TBM_SETPOS, 1, position);
                }
                Control::RestoreCrt => {
                    enable(w.hwnd, control, settings::has_any(&CRT_SLIDERS));
                }
                Control::FilesNote => {
                    let note = if crate::file_types::registered_here() { FILES_REGISTERED_NOTE } else { FILES_NOTE };
                    let _ = unsafe { SetWindowTextW(control, &HSTRING::from(localized(note))) };
                }
                Control::DebuggerNote => {
                    let _ = unsafe { SetWindowTextW(control, &HSTRING::from(debugger_note())) };
                }
                _ => {}
            }
        }

        // The bundled ROMs, the user's last ROM file and Other…, as in
        // Machine → System ROM.
        let rom = roms::system_rom_setting();
        let file = rom_file(SYSTEM_ROM_FILE);
        let mut titles: Vec<String> = SYSTEM_ROMS.iter().map(|r| r.title()).collect();
        w.system_roms = SYSTEM_ROMS.iter().map(|r| RomItem::Rom(Some(r.name.to_string()))).collect();

        if let Some((name, path)) = &file {
            titles.push(name.clone());
            w.system_roms.push(RomItem::Rom(Some(path.clone())));
        }

        titles.push(localized("Other…").to_string());
        w.system_roms.push(RomItem::Other);
        let default = SYSTEM_ROMS.iter().position(|r| r.name == roms::DEFAULT_SYSTEM_ROM).unwrap_or(0);
        let selected = w.system_roms.iter().position(|i| *i == RomItem::Rom(Some(rom.clone()))).unwrap_or(default);
        let combo = w.controls[row_of(Control::SystemRom)];
        fill_combo(combo, &titles, selected);
        let tip = file.filter(|(_, path)| *path == rom).map(|(_, path)| path).unwrap_or_default();
        set_tooltip(w.tooltip, w.hwnd, combo, &tip);

        // None, the user's last expansion ROM file and Other….
        let expansion = settings::string_setting(EXPANSION_ROM);
        let file = rom_file(EXPANSION_ROM_FILE);
        let mut titles = vec![localized("None").to_string()];
        w.expansion_roms = vec![RomItem::Rom(None)];

        if let Some((name, path)) = &file {
            titles.push(name.clone());
            w.expansion_roms.push(RomItem::Rom(Some(path.clone())));
        }

        titles.push(localized("Other…").to_string());
        w.expansion_roms.push(RomItem::Other);
        let selected = w.expansion_roms.iter().position(|i| *i == RomItem::Rom(expansion.clone())).unwrap_or(0);
        let combo = w.controls[row_of(Control::ExpansionRom)];
        fill_combo(combo, &titles, selected);
        let tip = file.filter(|(_, path)| Some(path) == expansion.as_ref()).map(|(_, path)| path).unwrap_or_default();
        set_tooltip(w.tooltip, w.hwnd, combo, &tip);

        // A value with a region ("et-EE") matches its language.
        let language = settings::string_setting(LANGUAGE);
        let selected = LANGUAGES
            .iter()
            .position(|(code, _)| code.is_some_and(|c| language.as_deref().is_some_and(|l| l == c || l.starts_with(&format!("{c}-")))))
            .unwrap_or(0);
        let titles: Vec<String> = LANGUAGES.iter().map(|(_, name)| localized(name).to_string()).collect();
        fill_combo(w.controls[row_of(Control::Language)], &titles, selected);

        let appearance = theme::Appearance::chosen();
        let selected = theme::Appearance::ALL.iter().position(|a| *a == appearance).unwrap_or(0);
        let titles: Vec<String> = theme::Appearance::ALL.iter().map(|a| localized(a.title()).to_string()).collect();
        fill_combo(w.controls[row_of(Control::Appearance)], &titles, selected);

        // Key Click is grayed out, with a note, when the ROM has no click
        // code the patch can change (§4.3).
        let available = key_click_available.unwrap_or(w.key_click_available);
        let key_click = ROWS.iter().position(|r| matches!(r.control, Control::Check(KEY_CLICK, ..))).unwrap_or(0);
        enable(w.hwnd, w.controls[key_click], available);

        // The drive sounds' settings apply only while they are on.
        let drives = settings::bool_setting(DRIVE_SOUNDS, true);
        for row in ROWS.iter().enumerate().filter(|(_, r)| matches!(r.control, Control::Slider(DRIVE_SOUNDS_VOLUME | DRIVE_SOUNDS_SPREAD, ..) | Control::Check(DRIVE_IDLE_MUTE, ..))) {
            enable(w.hwnd, w.controls[row.0], drives);
        }
        let changed = available != w.key_click_available;
        w.key_click_available = available;
        show_rows(w);
        Some((w.hwnd, changed))
    });

    if let Some((hwnd, true)) = relayout {
        layout(hwnd, false);
    }
}

/// Enables or disables a control. One that is disabled while it has the
/// keyboard hands it on to the next control first, as a dialog does.
fn enable(hwnd: HWND, control: HWND, on: bool) {
    unsafe {
        if !on && GetFocus() == control {
            let _ = SetFocus(GetNextDlgTabItem(hwnd, Some(control), false).ok());
        }

        let _ = EnableWindow(control, on);

        // Its label follows (`is_dimmed_label`).
        if let Ok(label) = GetDlgItem(Some(hwnd), GetDlgCtrlID(control) - FIRST_ID + FIRST_LABEL_ID) {
            let _ = InvalidateRect(Some(label), None, true);
        }
    }
}

fn set_tooltip(tooltip: HWND, parent: HWND, control: HWND, text: &str) {
    let mut text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let mut tool = tool_info(parent, control);
    tool.lpszText = PWSTR(text.as_mut_ptr());
    send(tooltip, TTM_UPDATETIPTEXTW, 0, (&raw mut tool) as isize);
}

/// A slider moved: its setting follows, saved once the drag ends.
fn slider_moved(hwnd: HWND, slider: HWND, code: u32) {
    let id = unsafe { GetDlgCtrlID(slider) };
    let Some(Control::Slider(key, min, max)) = ROWS.get((id - FIRST_ID) as usize).map(|r| r.control) else {
        return;
    };

    if code == TB_ENDTRACK {
        settings::flush();
        return;
    }

    let position = send(slider, TBM_GETPOS, 0, 0) as f64 / SLIDER_STEPS as f64;
    let value = ((min + (max - min) * position) * 1e5).round() / 1e5;
    let text = value.to_string();

    if settings::get(SECTION_SETTINGS, key).as_deref() == Some(text.as_str()) {
        return;
    }

    if code == TB_THUMBTRACK {
        settings::set_live(SECTION_SETTINGS, key, Some(&text));
    } else {
        settings::set(SECTION_SETTINGS, key, Some(&text));
    }

    crate::settings_changed(hwnd);
}

/// A list's choice is made: when an item is picked from the open list, or
/// with the arrow keys on the closed one.
fn combo_chosen(hwnd: HWND, row: usize, combo: HWND) {
    let index = send(combo, CB_GETCURSEL, 0, 0);
    let Ok(index) = usize::try_from(index) else {
        return;
    };

    let item = WINDOW.with_borrow(|w| {
        let w = w.as_ref()?;
        match ROWS[row].control {
            Control::SystemRom => w.system_roms.get(index).cloned(),
            Control::ExpansionRom => w.expansion_roms.get(index).cloned(),
            _ => None,
        }
    });

    match (ROWS[row].control, item) {
        (Control::SystemRom, Some(RomItem::Rom(Some(rom)))) => {
            if rom != roms::system_rom_setting() {
                settings::set_string_setting(SYSTEM_ROM, Some(&rom));
                crate::settings_changed(hwnd);
            }
        }
        (Control::ExpansionRom, Some(RomItem::Rom(rom))) => {
            if rom != settings::string_setting(EXPANSION_ROM) {
                settings::set_string_setting(EXPANSION_ROM, rom.as_deref());
                crate::settings_changed(hwnd);
            }
        }
        // Other…: the list shows the ROM in use until a file is chosen.
        (Control::SystemRom, Some(RomItem::Other)) => {
            refresh();
            crate::choose_system_rom_file(hwnd);
        }
        (Control::ExpansionRom, Some(RomItem::Other)) => {
            refresh();
            crate::choose_expansion_rom_file(hwnd);
        }
        (Control::Language, _) => {
            let code = LANGUAGES.get(index).and_then(|(code, _)| *code);

            if code.map(str::to_string) != settings::string_setting(LANGUAGE) {
                settings::set_string_setting(LANGUAGE, code);
                crate::settings_changed(hwnd);
            }
        }
        // Takes effect at once, in every window.
        (Control::Appearance, _) => {
            let appearance = theme::Appearance::ALL.get(index).copied().unwrap_or(theme::Appearance::System);

            if appearance != theme::Appearance::chosen() {
                settings::set_string_setting(APPEARANCE, appearance.setting_value());
                theme::appearance_chosen();
            }
        }
        _ => {}
    }
}

fn command(hwnd: HWND, id: i32, code: u32, control: HWND) {
    if id == IDCANCEL.0 {
        close(hwnd);
        return;
    }

    let row = (id - FIRST_ID) as usize;
    let Some(spec) = ROWS.get(row) else {
        return;
    };

    match spec.control {
        Control::Check(key, ..) if code == BN_CLICKED => {
            let on = send(control, BM_GETCHECK, 0, 0) == 1;
            settings::set_bool_setting(key, on);
            crate::settings_changed(hwnd);
        }
        // Back to the defaults; CRT Effects and 3D Frame on or off are left
        // alone.
        Control::RestoreCrt if code == BN_CLICKED => {
            for key in CRT_SLIDERS {
                settings::set(SECTION_SETTINGS, key, None);
            }

            crate::settings_changed(hwnd);
        }
        Control::RegisterFiles if code == BN_CLICKED => {
            if !crate::file_types::register() {
                crate::show_alerts(hwnd, vec![crate::disks::Alert::named("The file types could not be registered.", "")]);
            }

            refresh();
        }
        Control::SystemRom | Control::ExpansionRom | Control::Language | Control::Appearance => {
            let dropped = send(control, CB_GETDROPPEDSTATE, 0, 0) != 0;

            if code == CBN_CLOSEUP || (code == CBN_SELCHANGE && !dropped) {
                combo_chosen(hwnd, row, control);
            }
        }
        _ => {}
    }
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

/// The notes are gray, as secondary text.
fn is_note(control: HWND) -> bool {
    let id = unsafe { GetDlgCtrlID(control) };
    ROWS.get((id - FIRST_ID) as usize).is_some_and(|r| matches!(r.control, Control::Note(_) | Control::FilesNote | Control::DebuggerNote))
}

/// True for the label of a disabled control, which is grayed with it, as
/// Drive Volume: and Drive Separation: are while Drive Sounds is off. Not for a
/// checkbox's label, such as Keyboard:, which heads a group and whose
/// checkbox has a title of its own to gray.
fn is_dimmed_label(label: HWND) -> bool {
    let row = unsafe { GetDlgCtrlID(label) } - FIRST_LABEL_ID;

    let Some(spec) = usize::try_from(row).ok().and_then(|row| ROWS.get(row)) else {
        return false;
    };

    let control = unsafe { GetDlgItem(GetParent(label).ok(), FIRST_ID + row) };
    !matches!(spec.control, Control::Check(..)) && control.is_ok_and(|c| !unsafe { IsWindowEnabled(c) }.as_bool())
}

extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCLBUTTONDOWN {
        crate::emu_window::title_bar_pressed(hwnd, wparam, lparam);
    }

    if theme::changed(msg, lparam) {
        apply_theme();
        theme::redraw(hwnd);
    }

    match msg {
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            command(hwnd, id, code, HWND(lparam.0 as *mut _));
            return LRESULT(0);
        }
        WM_HSCROLL => {
            slider_moved(hwnd, HWND(lparam.0 as *mut _), (wparam.0 & 0xFFFF) as u32);
            return LRESULT(0);
        }
        WM_NOTIFY => {
            let header = unsafe { &*(lparam.0 as *const NMHDR) };

            if header.code == NM_CUSTOMDRAW
                && let Some(result) = theme::custom_draw(lparam)
            {
                return result;
            }

            if header.idFrom == TAB_ID as usize && header.code == TCN_SELCHANGE {
                show_pane(send(header.hwndFrom, TCM_GETCURSEL, 0, 0).max(0) as usize, false);
            }

            return LRESULT(0);
        }
        // Every control stands on the tab control's page.
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            let dc = HDC(wparam.0 as *mut _);
            let control = HWND(lparam.0 as *mut _);
            let gray = is_note(control) || is_dimmed_label(control);

            let text = if gray { theme::gray_text_color() } else { theme::text_color() };

            unsafe {
                SetTextColor(dc, text);
                SetBkColor(dc, COLORREF(PAGE_COLOR.get()));
                return LRESULT(PAGE_BRUSH.get());
            }
        }
        // The open list of a drop-down list, in dark mode.
        WM_CTLCOLORLISTBOX if theme::dark() => {
            let dc = HDC(wparam.0 as *mut _);

            unsafe {
                SetTextColor(dc, theme::TEXT);
                SetBkColor(dc, theme::PAGE);
            }

            return LRESULT(theme::brush(theme::PAGE).0 as isize);
        }
        // The window's colour, and the page's under the controls, which
        // themed controls ask for behind their rounded corners.
        WM_ERASEBKGND => {
            let dc = HDC(wparam.0 as *mut _);
            let mut client = RECT::default();

            unsafe {
                let _ = GetClientRect(hwnd, &mut client);
                FillRect(dc, &client, theme::window_brush());
                FillRect(dc, &PAGE.get(), HBRUSH(PAGE_BRUSH.get() as *mut _));
            }

            return LRESULT(1);
        }
        WM_THEMECHANGED | WM_SYSCOLORCHANGE => {
            if let Some(tab) = WINDOW.with(|w| w.try_borrow().ok().and_then(|w| w.as_ref().map(|w| w.tab))) {
                set_page_color(tab);
                let _ = unsafe { InvalidateRect(Some(hwnd), None, true) };
            }
        }
        // The keyboard goes back to the control that had it.
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                let focus = unsafe { GetFocus() };

                if unsafe { IsChild(hwnd, focus) }.as_bool() {
                    LAST_FOCUS.set(focus.0 as isize);
                }
            } else {
                let last = HWND(LAST_FOCUS.get() as *mut _);
                let shown = !last.is_invalid() && unsafe { IsChild(hwnd, last).as_bool() && IsWindowVisible(last).as_bool() };
                let focus = if shown { Some(last) } else { unsafe { GetNextDlgTabItem(hwnd, None, false) }.ok() };
                let _ = unsafe { SetFocus(focus) };
            }

            return LRESULT(0);
        }
        WM_CLOSE => {
            close(hwnd);
            return LRESULT(0);
        }
        WM_DPICHANGED => {
            let dpi = ((wparam.0 >> 16) & 0xFFFF) as u32;
            let suggested = unsafe { &*(lparam.0 as *const RECT) };
            let (font, note_font) = create_fonts(dpi);

            let old = WINDOW.with_borrow_mut(|w| {
                w.as_mut().map(|w| (std::mem::replace(&mut w.font, font), std::mem::replace(&mut w.note_font, note_font)))
            });

            set_fonts();

            if let Some((a, b)) = old {
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(a.0));
                    let _ = DeleteObject(HGDIOBJ(b.0));
                }
            }

            let _ = unsafe { SetWindowPos(hwnd, None, suggested.left, suggested.top, 0, 0, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSIZE) };
            layout(hwnd, false);
            return LRESULT(0);
        }
        _ => {}
    }

    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

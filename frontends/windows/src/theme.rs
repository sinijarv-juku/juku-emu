//! Light and dark mode (spec §11.5): the app follows Windows' setting for
//! apps, unless Settings → Appearance chooses one (Windows only). Win32 has a supported dark title bar and a Mica backdrop, but no
//! supported dark menus or common controls, so the rest is pieced together:
//!
//! - Popup menus turn dark through functions `uxtheme.dll` exports only by
//!   ordinal, which Explorer and Notepad++ use. Without them, on Windows 10
//!   before 1809, the menus stay light.
//! - The menu bar is drawn here, through the messages `DefWindowProc` sends
//!   to have it drawn (`WM_UAHDRAWMENU`, `WM_UAHDRAWMENUITEM`), which are
//!   undocumented too.
//! - Controls take the dark themes Explorer's own windows use
//!   (`DarkMode_Explorer`, `DarkMode_CFD`), and the colour messages give the
//!   rest; what no theme covers, the tab control, is painted here.

use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

use windows::Win32::Foundation::{COLORREF, ERROR_SUCCESS, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_BTNFACE, COLOR_GRAYTEXT, CreateSolidBrush, GetSysColor, GetSysColorBrush, SYS_COLOR_INDEX, DT_CENTER, DT_HIDEPREFIX, DT_SINGLELINE, DT_VCENTER, DrawFocusRect, DrawTextW, EndPaint, FillRect,
    FrameRect, GetWindowDC, HBRUSH, HDC, HFONT, HGDIOBJ, InflateRect, MapWindowPoints, OffsetRect, PAINTSTRUCT, RDW_ALLCHILDREN, RDW_ERASE, RDW_FRAME,
    RDW_INVALIDATE, RedrawWindow, ReleaseDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW};
use windows::Win32::UI::Controls::{
    BP_PUSHBUTTON, CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDRF_DODEFAULT, CDRF_NOTIFYITEMDRAW, CDRF_SKIPDEFAULT, COMBOBOXINFO,
    CloseThemeData, DRAWITEMSTRUCT, DrawThemeBackground, GetComboBoxInfo, NMCUSTOMDRAW, ODS_DISABLED, OpenThemeData, PBS_DISABLED, TBCD_CHANNEL, ODS_GRAYED, ODS_HOTLIGHT, ODS_INACTIVE, ODS_NOACCEL, ODS_SELECTED, SetWindowTheme, TCIF_TEXT, TCITEMW,
    TCM_ADJUSTRECT, TCM_GETCURSEL, TCM_GETITEMCOUNT, TCM_GETITEMRECT, TCM_GETITEMW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, IsWindowEnabled};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, PCSTR, PCWSTR, PWSTR, w};

/// Colours as `COLORREF`, which is 0x00BBGGRR.
const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

// The dark colours, Windows 11's: the window, the raised page and its edge,
// text and secondary text, and the menu bar's items under the mouse and
// opened.
pub const WINDOW: COLORREF = rgb(32, 32, 32);
pub const PAGE: COLORREF = rgb(43, 43, 43);
const EDGE: COLORREF = rgb(64, 64, 64);
pub const TEXT: COLORREF = rgb(255, 255, 255);
pub const GRAY_TEXT: COLORREF = rgb(160, 160, 160);
const DIM_TEXT: COLORREF = rgb(110, 110, 110);
const CHANNEL: COLORREF = rgb(100, 100, 100);
const MENU_HOT: COLORREF = rgb(56, 56, 56);
const MENU_OPEN: COLORREF = rgb(66, 66, 66);

// The messages `DefWindowProc` sends to have the menu bar drawn.
const WM_UAHDRAWMENU: u32 = 0x0091;
const WM_UAHDRAWMENUITEM: u32 = 0x0092;

/// The menu bar being drawn (`UAHMENU`).
#[repr(C)]
struct UahMenu {
    menu: HMENU,
    dc: HDC,
    flags: u32,
}

/// One item of it (`UAHDRAWMENUITEM`): the item's place and state, the menu,
/// and the item's position in it, after its metrics.
#[repr(C)]
struct UahDrawMenuItem {
    item: DRAWITEMSTRUCT,
    menu: UahMenu,
    position: i32,
    metrics: [u32; 8],
    popup_metrics: [u32; 5],
}

/// The system colour index of button text, which the windows crate does
/// not name in the features the app uses.
const COLOR_BTNTEXT: SYS_COLOR_INDEX = SYS_COLOR_INDEX(18);

/// Settings → Appearance.
#[derive(Clone, Copy, PartialEq)]
pub enum Appearance {
    /// Windows' setting for apps.
    System,
    Light,
    Dark,
}

impl Appearance {
    /// In the order of the Appearance list.
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];

    pub fn title(self) -> &'static str {
        match self {
            Appearance::System => "Same as Windows",
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
        }
    }

    /// The value in Juku.ini; None for the system's.
    pub fn setting_value(self) -> Option<&'static str> {
        match self {
            Appearance::System => None,
            Appearance::Light => Some("light"),
            Appearance::Dark => Some("dark"),
        }
    }

    pub fn chosen() -> Appearance {
        match crate::settings::string_setting(crate::settings::APPEARANCE).as_deref() {
            Some("light") => Appearance::Light,
            Some("dark") => Appearance::Dark,
            _ => Appearance::System,
        }
    }
}

thread_local! {
    static DARK: Cell<bool> = const { Cell::new(false) };
    static BRUSHES: RefCell<Vec<(u32, HBRUSH)>> = const { RefCell::new(Vec::new()) };
}

/// The functions of `uxtheme.dll` known only by their ordinals: 135
/// `SetPreferredAppMode` (from Windows 10 1903; in 1809 the same slot was
/// `AllowDarkModeForApp`, which takes the same 1 for yes), 133
/// `AllowDarkModeForWindow`, 104 `RefreshImmersiveColorPolicyState` and 136
/// `FlushMenuThemes`, all there from 1809 (build 17763).
struct Uxtheme {
    set_preferred_app_mode: unsafe extern "system" fn(i32) -> i32,
    allow_dark_mode_for_window: unsafe extern "system" fn(HWND, BOOL) -> BOOL,
    refresh_immersive_color_policy_state: unsafe extern "system" fn(),
    flush_menu_themes: unsafe extern "system" fn(),
}

fn uxtheme() -> Option<&'static Uxtheme> {
    static UXTHEME: OnceLock<Option<Uxtheme>> = OnceLock::new();

    UXTHEME
        .get_or_init(|| {
            if windows_build() < 17763 {
                return None;
            }

            unsafe {
                let module = LoadLibraryExW(w!("uxtheme.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
                let ordinal = |n: usize| GetProcAddress(module, PCSTR(n as *const u8));

                Some(Uxtheme {
                    set_preferred_app_mode: std::mem::transmute::<unsafe extern "system" fn() -> isize, _>(ordinal(135)?),
                    allow_dark_mode_for_window: std::mem::transmute::<unsafe extern "system" fn() -> isize, _>(ordinal(133)?),
                    refresh_immersive_color_policy_state: std::mem::transmute::<unsafe extern "system" fn() -> isize, _>(ordinal(104)?),
                    flush_menu_themes: std::mem::transmute::<unsafe extern "system" fn() -> isize, _>(ordinal(136)?),
                })
            }
        })
        .as_ref()
}

/// Windows' build number, 0 if it cannot be read.
fn windows_build() -> u32 {
    let mut text = [0u16; 16];
    let mut size = size_of_val(&text) as u32;
    let found = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"Software\Microsoft\Windows NT\CurrentVersion"),
            w!("CurrentBuildNumber"),
            RRF_RT_REG_SZ,
            None,
            Some(text.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };

    if found != ERROR_SUCCESS {
        return 0;
    }

    let len = text.iter().position(|&c| c == 0).unwrap_or(text.len());
    String::from_utf16_lossy(&text[..len]).parse().unwrap_or(0)
}

/// Whether Windows is set to dark mode for apps.
fn read_setting() -> bool {
    let mut light: u32 = 1;
    let mut size = size_of::<u32>() as u32;
    let found = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut light).cast()),
            Some(&mut size),
        )
    };
    found == ERROR_SUCCESS && light == 0
}

/// Takes the mode from Settings → Appearance or from Windows, and lets the
/// menus follow it. Called before the first window is created, and again
/// when either changes.
pub fn init() {
    let appearance = Appearance::chosen();
    DARK.set(match appearance {
        Appearance::System => read_setting(),
        Appearance::Light => false,
        Appearance::Dark => true,
    });

    if let Some(ux) = uxtheme() {
        // AllowDark follows the system, ForceDark and ForceLight do not;
        // those two are there from 1903 (build 18362).
        let mode = match appearance {
            Appearance::Dark if windows_build() >= 18362 => 2,
            Appearance::Light if windows_build() >= 18362 => 3,
            _ => 1,
        };

        unsafe {
            (ux.set_preferred_app_mode)(mode);
            (ux.refresh_immersive_color_policy_state)();
            (ux.flush_menu_themes)();
        }
    }
}

/// The message that tells the app's windows that Settings → Appearance
/// changed.
fn appearance_changed_message() -> u32 {
    static MESSAGE: OnceLock<u32> = OnceLock::new();
    *MESSAGE.get_or_init(|| unsafe { RegisterWindowMessageW(w!("JukuAppearanceChanged")) })
}

/// Settings → Appearance changed: the mode is taken again and every window
/// of the app is told, so each redraws itself in it.
pub fn appearance_chosen() {
    init();

    extern "system" fn tell(hwnd: HWND, _: LPARAM) -> BOOL {
        unsafe { SendMessageW(hwnd, appearance_changed_message(), None, None) };
        BOOL(1)
    }

    let _ = unsafe { EnumThreadWindows(GetCurrentThreadId(), Some(tell), LPARAM(0)) };
}

/// Whether the app is drawn dark.
pub fn dark() -> bool {
    DARK.get()
}

/// True if `msg` says the mode may have changed: Windows' light or dark
/// mode (`WM_SETTINGCHANGE` with `ImmersiveColorSet`, after which the mode
/// is taken again), or Settings → Appearance. The window then calls `apply`
/// and redraws.
pub fn changed(msg: u32, lparam: LPARAM) -> bool {
    if msg == appearance_changed_message() {
        return true;
    }

    let name = lparam.0 as *const u16;

    if msg != WM_SETTINGCHANGE || name.is_null() || !unsafe { PCWSTR(name).to_string() }.is_ok_and(|n| n == "ImmersiveColorSet") {
        return false;
    }

    init();
    true
}

/// A top-level window in the current mode: its title bar, its menus, and a
/// Mica backdrop behind the title bar (Windows 11 22H2 and later; with
/// transparency effects off, Windows draws it as a plain colour).
pub fn apply(hwnd: HWND) {
    let dark = BOOL::from(dark());
    let backdrop = DWMSBT_MAINWINDOW;

    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, (&raw const dark).cast(), size_of::<BOOL>() as u32);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, (&raw const backdrop).cast(), size_of_val(&backdrop) as u32);

        if let Some(ux) = uxtheme() {
            let _ = (ux.allow_dark_mode_for_window)(hwnd, dark);
        }
    }
}

/// Redraws a window and its controls, frame and menu bar included, after
/// the mode changed.
pub fn redraw(hwnd: HWND) {
    unsafe {
        let _ = DrawMenuBar(hwnd);
        let _ = RedrawWindow(Some(hwnd), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN);
    }
}

/// The background of a window, its text and its secondary text, in the
/// current mode.
pub fn window_brush() -> HBRUSH {
    if dark() { brush(WINDOW) } else { unsafe { GetSysColorBrush(COLOR_BTNFACE) } }
}

pub fn text_color() -> COLORREF {
    if dark() { TEXT } else { COLORREF(unsafe { GetSysColor(COLOR_BTNTEXT) }) }
}

pub fn gray_text_color() -> COLORREF {
    if dark() { GRAY_TEXT } else { COLORREF(unsafe { GetSysColor(COLOR_GRAYTEXT) }) }
}

/// A brush of `color`, kept for the life of the app.
pub fn brush(color: COLORREF) -> HBRUSH {
    BRUSHES.with_borrow_mut(|brushes| {
        if let Some((_, brush)) = brushes.iter().find(|(c, _)| *c == color.0) {
            return *brush;
        }

        let brush = unsafe { CreateSolidBrush(color) };
        brushes.push((color.0, brush));
        brush
    })
}

/// What a control is, for the theme it takes.
#[derive(Clone, Copy)]
pub enum ControlKind {
    /// Push buttons, check boxes, scroll bars, tooltips.
    Explorer,
    /// Drop-down lists.
    ComboBox,
    /// Labels, sliders: they take the colours they are given.
    Plain,
}

/// A control in the current mode.
pub fn apply_control(control: HWND, kind: ControlKind) {
    let theme = match (dark(), kind) {
        (true, ControlKind::Explorer) => w!("DarkMode_Explorer"),
        (true, ControlKind::ComboBox) => w!("DarkMode_CFD"),
        _ => PCWSTR::null(),
    };

    unsafe {
        let _ = SetWindowTheme(control, theme, PCWSTR::null());

        if let Some(ux) = uxtheme() {
            let _ = (ux.allow_dark_mode_for_window)(control, BOOL::from(dark()));
        }
    }

    // The open list of a drop-down list scrolls with a scroll bar of its
    // own.
    if matches!(kind, ControlKind::ComboBox) {
        let mut info = COMBOBOXINFO { cbSize: size_of::<COMBOBOXINFO>() as u32, ..Default::default() };

        if unsafe { GetComboBoxInfo(control, &mut info) }.is_ok() && !info.hwndList.is_invalid() {
            apply_control(info.hwndList, ControlKind::Explorer);
        }
    }
}

/// `NM_CUSTOMDRAW` from a control, in dark mode, for what its dark theme
/// draws in light colours: a slider's channel, and the text of a push
/// button that is off, which would look nearly as bright as one that is on.
pub fn custom_draw(lparam: LPARAM) -> Option<LRESULT> {
    if !dark() {
        return None;
    }

    let draw = unsafe { &*(lparam.0 as *const NMCUSTOMDRAW) };
    let control = draw.hdr.hwndFrom;
    let mut class = [0u16; 32];
    let len = unsafe { GetClassNameW(control, &mut class) } as usize;
    let class = String::from_utf16_lossy(&class[..len]);

    if class.eq_ignore_ascii_case("msctls_trackbar32") {
        return Some(LRESULT(match draw.dwDrawStage {
            CDDS_PREPAINT => CDRF_NOTIFYITEMDRAW,
            CDDS_ITEMPREPAINT if draw.dwItemSpec == TBCD_CHANNEL as usize => {
                unsafe { FillRect(draw.hdc, &draw.rc, brush(CHANNEL)) };
                CDRF_SKIPDEFAULT
            }
            _ => CDRF_DODEFAULT,
        } as isize));
    }

    let style = unsafe { GetWindowLongW(control, GWL_STYLE) };

    if class.eq_ignore_ascii_case("Button")
        && style & BS_TYPEMASK == BS_PUSHBUTTON
        && draw.dwDrawStage == CDDS_PREPAINT
        && !unsafe { IsWindowEnabled(control) }.as_bool()
    {
        let mut text = [0u16; 128];
        let len = unsafe { GetWindowTextW(control, &mut text) } as usize;
        let font = unsafe { SendMessageW(control, WM_GETFONT, None, None) };
        let mut rect = draw.rc;

        unsafe {
            let theme = OpenThemeData(Some(control), w!("Button"));
            let _ = DrawThemeBackground(theme, draw.hdc, BP_PUSHBUTTON.0, PBS_DISABLED.0, &draw.rc, None);
            let _ = CloseThemeData(theme);
            let old = SelectObject(draw.hdc, HGDIOBJ(font.0 as *mut _));
            SetBkMode(draw.hdc, TRANSPARENT);
            SetTextColor(draw.hdc, DIM_TEXT);
            DrawTextW(draw.hdc, &mut text[..len], &mut rect, DT_CENTER | DT_SINGLELINE | DT_VCENTER);
            SelectObject(draw.hdc, old);
        }

        return Some(LRESULT(CDRF_SKIPDEFAULT as isize));
    }

    None
}

/// Draws the menu bar for `msg` in dark mode; None for anything else, which
/// then goes on to `DefWindowProc`. The bar is filled, each item drawn on
/// it, and the line Windows draws under the bar in the light colour is
/// painted over once the frame is drawn.
pub fn menu_bar_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    if !dark() {
        return None;
    }

    match msg {
        WM_UAHDRAWMENU => {
            let menu = unsafe { &*(lparam.0 as *const UahMenu) };
            let mut bar = MENUBARINFO { cbSize: size_of::<MENUBARINFO>() as u32, ..Default::default() };
            let mut frame = RECT::default();

            unsafe {
                if GetMenuBarInfo(hwnd, OBJID_MENU, 0, &mut bar).is_ok() && GetWindowRect(hwnd, &mut frame).is_ok() {
                    let mut rect = bar.rcBar;
                    let _ = OffsetRect(&mut rect, -frame.left, -frame.top);
                    FillRect(menu.dc, &rect, brush(WINDOW));
                }
            }

            Some(LRESULT(1))
        }
        WM_UAHDRAWMENUITEM => {
            let draw = unsafe { &*(lparam.0 as *const UahDrawMenuItem) };
            let state = draw.item.itemState;
            let mut text = [0u16; 256];
            let mut info = MENUITEMINFOW {
                cbSize: size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_STRING,
                dwTypeData: PWSTR(text.as_mut_ptr()),
                cch: (text.len() - 1) as u32,
                ..Default::default()
            };
            let _ = unsafe { GetMenuItemInfoW(draw.menu.menu, draw.position as u32, true, &mut info) };
            let len = (info.cch as usize).min(text.len() - 1);

            let background = if state.0 & ODS_SELECTED.0 != 0 {
                MENU_OPEN
            } else if state.0 & ODS_HOTLIGHT.0 != 0 {
                MENU_HOT
            } else {
                WINDOW
            };
            let color = if state.0 & (ODS_GRAYED.0 | ODS_DISABLED.0) != 0 {
                DIM_TEXT
            } else if state.0 & ODS_INACTIVE.0 != 0 {
                GRAY_TEXT
            } else {
                TEXT
            };
            let mut flags = DT_CENTER | DT_SINGLELINE | DT_VCENTER;

            if state.0 & ODS_NOACCEL.0 != 0 {
                flags |= DT_HIDEPREFIX;
            }

            let dc = draw.item.hDC;
            let mut rect = draw.item.rcItem;

            unsafe {
                FillRect(dc, &rect, brush(background));
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(dc, color);
                DrawTextW(dc, &mut text[..len], &mut rect, flags);
            }

            Some(LRESULT(1))
        }
        WM_NCPAINT | WM_NCACTIVATE => {
            let result = unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
            cover_menu_bar_line(hwnd);
            Some(result)
        }
        _ => None,
    }
}

/// The line between the menu bar and the client area, a pixel above the
/// client area, in the bar's colour.
fn cover_menu_bar_line(hwnd: HWND) {
    let mut client = RECT::default();
    let mut frame = RECT::default();

    unsafe {
        if GetMenu(hwnd).is_invalid() || GetClientRect(hwnd, &mut client).is_err() || GetWindowRect(hwnd, &mut frame).is_err() {
            return;
        }

        let mut corners = [POINT { x: client.left, y: client.top }, POINT { x: client.right, y: client.bottom }];
        MapWindowPoints(Some(hwnd), None, &mut corners);
        let line = RECT {
            left: corners[0].x - frame.left,
            top: corners[0].y - frame.top - 1,
            right: corners[1].x - frame.left,
            bottom: corners[0].y - frame.top,
        };
        let dc = GetWindowDC(Some(hwnd));
        FillRect(dc, &line, brush(WINDOW));
        ReleaseDC(Some(hwnd), dc);
    }
}

/// Lets a tab control be painted dark: no theme has a dark tab control.
pub fn paint_tab_control_dark(tab: HWND) {
    let _ = unsafe { SetWindowSubclass(tab, Some(tab_control_proc), 1, 0) };
}

extern "system" fn tab_control_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    if dark() {
        match msg {
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => {
                paint_tab_control(hwnd);
                return LRESULT(0);
            }
            _ => {}
        }
    }

    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// A tab control as the light theme draws it, in the dark colours: the
/// page raised, with an edge, and the chosen tab joined to it.
fn paint_tab_control(hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(hwnd, &mut paint) };
    let send = |msg: u32, wparam: usize, lparam: isize| unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))) }.0;

    let mut client = RECT::default();
    let _ = unsafe { GetClientRect(hwnd, &mut client) };
    let mut page = client;
    send(TCM_ADJUSTRECT, 0, (&raw mut page) as isize);
    let selected = send(TCM_GETCURSEL, 0, 0);
    let font = HFONT(send(WM_GETFONT, 0, 0) as *mut _);

    unsafe {
        FillRect(dc, &client, brush(WINDOW));
        let old_font = SelectObject(dc, HGDIOBJ(font.0));
        SetBkMode(dc, TRANSPARENT);

        // The page's edge runs round the area TCM_ADJUSTRECT leaves, a few
        // pixels out, as the theme's does.
        let edge = RECT { left: client.left, top: page.top - 2, right: client.right, bottom: client.bottom };
        FillRect(dc, &edge, brush(PAGE));
        FrameRect(dc, &edge, brush(EDGE));

        for index in 0..send(TCM_GETITEMCOUNT, 0, 0) {
            let mut rect = RECT::default();
            send(TCM_GETITEMRECT, index as usize, (&raw mut rect) as isize);
            let chosen = index == selected;

            if chosen {
                // Taller, and open at the bottom into the page.
                rect.top -= 2;
                rect.left -= 2;
                rect.right += 2;
                rect.bottom = edge.top + 1;
            }

            FillRect(dc, &rect, brush(if chosen { PAGE } else { WINDOW }));
            FrameRect(dc, &rect, brush(EDGE));

            if chosen {
                let gap = RECT { left: rect.left + 1, top: edge.top, right: rect.right - 1, bottom: edge.top + 1 };
                FillRect(dc, &gap, brush(PAGE));
            }

            let mut text = [0u16; 64];
            let mut item = TCITEMW { mask: TCIF_TEXT, pszText: PWSTR(text.as_mut_ptr()), cchTextMax: text.len() as i32, ..Default::default() };
            send(TCM_GETITEMW, index as usize, (&raw mut item) as isize);
            let len = text.iter().position(|&c| c == 0).unwrap_or(0);
            SetTextColor(dc, if chosen { TEXT } else { GRAY_TEXT });
            let mut label = rect;
            DrawTextW(dc, &mut text[..len], &mut label, DT_CENTER | DT_SINGLELINE | DT_VCENTER);

            if chosen && GetFocus() == hwnd {
                let mut focus = rect;
                let _ = InflateRect(&mut focus, -3, -3);
                SetTextColor(dc, TEXT);
                let _ = DrawFocusRect(dc, &focus);
            }
        }

        SelectObject(dc, old_font);
        let _ = EndPaint(hwnd, &paint);
    }
}

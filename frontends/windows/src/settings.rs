//! The settings (frontends/macos/src/Settings.m, spec §11.4): `Juku.ini` in
//! `%LOCALAPPDATA%\Juku`, the Windows counterpart of NSUserDefaults. The same
//! folder holds the app's other files.
//!
//! The file is UTF-8, read once at start-up and written whole on every
//! change, to a temporary file that then replaces it. The parser is the app's
//! own: the Windows profile functions do not read UTF-8. Keys have the names
//! of the Mac's defaults; a list is a section of its own with numbered keys.

use std::cell::RefCell;
use std::path::PathBuf;

use crate::renderer::CrtSettings;

// Sections. The single settings are in Settings; lists (the drives, Recent
// Images) have sections of their own, written by their modules.
pub const SECTION_SETTINGS: &str = "Settings";
pub const SECTION_WINDOW: &str = "Window";
pub const SECTION_KEYBOARD_WINDOW: &str = "KeyboardWindow"; // its Frame, as the Window section's
pub const SECTION_DRIVES: &str = "Drives";
pub const SECTION_RECENT_DISKS: &str = "RecentDisks";

// Keys of the Settings section: the Mac's NSUserDefaults keys (Settings.m).
pub const CRT_EFFECTS: &str = "CRTEffects";
pub const CRT_CURVATURE: &str = "CRTCurvature";
pub const CRT_SCANLINES: &str = "CRTScanlines";
pub const CRT_GLOW: &str = "CRTGlow";
pub const CRT_BACKGROUND: &str = "CRTBackground"; // what the unlit face gives back of the room light, 0–1
pub const CRT_FRAME: &str = "CRTFrame"; // the monitor's plastic, 0–1: dark to light
pub const CRT_ROOM: &str = "CRTRoom"; // the room light, 0–1
pub const CRT_BEZEL_3D: &str = "CRTBezel3D"; // the frame's inner wall in perspective
pub const CRT_MONITOR_FRONT: &str = "CRTMonitorFront"; // the monitor's whole front, with its badge
pub const CRT_BRIGHTNESS: &str = "CRTBrightness";
pub const MOUSE_SENSITIVITY: &str = "MouseSensitivity"; // Juku counts per mouse count
pub const KEY_CLICK: &str = "KeyClick"; // RomBios key click on/off
pub const DRIVE_SOUNDS: &str = "DriveSounds"; // floppy drive sounds on/off
pub const DRIVE_SOUNDS_VOLUME: &str = "DriveSoundsVolume"; // 0–2, 1 = the recordings' level halved
pub const DRIVE_SOUNDS_SPREAD: &str = "DriveSoundsSpread"; // 0–1: how far apart A: and B: are heard
pub const DRIVE_IDLE_MUTE: &str = "DriveIdleMute"; // the motor fades out 10 s after the last disk command
pub const T32_FAST_CLOCK: &str = "T32FastClock"; // timer T3.2 at 1.23 MHz instead of the frame rate
pub const SYSTEM_ROM: &str = "SystemROM"; // bundled ROM name or file path (roms.rs)
pub const SYSTEM_ROM_FILE: &str = "SystemROMFile"; // the user's last ROM file, a path
pub const EXPANSION_ROM: &str = "ExpansionROM"; // path of the plugged-in expansion ROM; absent = none
pub const EXPANSION_ROM_FILE: &str = "ExpansionROMFile"; // the user's last expansion ROM file
pub const ACCEPT_DEBUGGER: &str = "AcceptDebugger"; // the debug server listens (spec §12.3)
pub const DEBUGGER_PORT: &str = "DebuggerPort"; // its TCP port on 127.0.0.1; no control of its own
pub const LANGUAGE: &str = "Language"; // "en" or "et"; absent = the system's (§11.4)
pub const APPEARANCE: &str = "Appearance"; // "light" or "dark"; absent = Windows' setting (§11.5)
pub const KEYBOARD_TAGS: &str = "KeyboardTags"; // PC key tags in the Keyboard window: 0 where different, 1 all, 2 off

/// The CRT sliders, which Restore CRT Defaults resets (Settings.m,
/// crtSettingKeys).
pub const CRT_SLIDERS: [&str; 7] = [CRT_CURVATURE, CRT_SCANLINES, CRT_GLOW, CRT_BACKGROUND, CRT_FRAME, CRT_ROOM, CRT_BRIGHTNESS];

pub const DEFAULT_MOUSE_SENSITIVITY: f64 = 0.5;
pub const DEFAULT_DEBUGGER_PORT: u16 = 5104;

// Keys of the Window section: the window's frame (the Mac's "NSWindow Frame
// Juku"), as `left,top,right,bottom` of its normal position, and whether it
// is maximized.
pub const WINDOW_FRAME: &str = "Frame";
pub const WINDOW_MAXIMIZED: &str = "Maximized";

struct Ini {
    path: Option<PathBuf>,
    sections: Vec<(String, Vec<(String, String)>)>,
    /// Changed since it was last saved (set_live).
    dirty: bool,
}

thread_local! {
    static INI: RefCell<Ini> = RefCell::new(Ini::load());
}

/// `%LOCALAPPDATA%\Juku`, created if it is not there; None if it cannot be.
pub fn folder() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    let folder = PathBuf::from(base).join("Juku");
    std::fs::create_dir_all(&folder).ok()?;
    Some(folder)
}

fn parse(text: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();

    for line in text.trim_start_matches('\u{FEFF}').lines() {
        let line = line.trim();

        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }

        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push((name.trim().to_string(), Vec::new()));
        } else if let (Some((key, value)), Some(section)) = (line.split_once('='), sections.last_mut()) {
            section.1.push((key.trim().to_string(), value.trim().to_string()));
        }
    }

    sections
}

impl Ini {
    fn load() -> Ini {
        let path = folder().map(|f| f.join("Juku.ini"));
        let sections = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse(&t)).unwrap_or_default();
        Ini { path, sections, dirty: false }
    }

    fn save(&mut self) {
        self.dirty = false;

        let Some(path) = &self.path else {
            return;
        };

        let mut text = String::from("; Juku settings. Juku rewrites this file; edit it while Juku is not running.\n");

        for (name, entries) in &self.sections {
            if entries.is_empty() {
                continue;
            }

            text.push_str(&format!("\n[{name}]\n"));

            for (key, value) in entries {
                text.push_str(&format!("{key}={value}\n"));
            }
        }

        // Written whole, then put in place: a crash leaves the old file.
        let temp = path.with_extension("ini.tmp");

        if let Err(e) = std::fs::write(&temp, text).and_then(|()| std::fs::rename(&temp, path)) {
            log!("saving {}: {e}", path.display());
        }
    }

    fn section_mut(&mut self, name: &str) -> &mut Vec<(String, String)> {
        let at = match self.sections.iter().position(|(n, _)| n == name) {
            Some(at) => at,
            None => {
                self.sections.push((name.to_string(), Vec::new()));
                self.sections.len() - 1
            }
        };
        &mut self.sections[at].1
    }
}

/// The value of `key` in `section`.
pub fn get(section: &str, key: &str) -> Option<String> {
    INI.with_borrow(|ini| {
        let (_, entries) = ini.sections.iter().find(|(n, _)| n == section)?;
        entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    })
}

/// The entries of `section`, in the file's order.
pub fn section(section: &str) -> Vec<(String, String)> {
    INI.with_borrow(|ini| ini.sections.iter().find(|(n, _)| n == section).map(|(_, e)| e.clone()).unwrap_or_default())
}

/// Sets `key` in `section`, or removes it for None; true if that changed it.
fn change(section: &str, key: &str, value: Option<&str>) -> bool {
    INI.with_borrow_mut(|ini| {
        let entries = ini.section_mut(section);
        let at = entries.iter().position(|(k, _)| k == key);

        match (at, value) {
            (Some(at), Some(value)) if entries[at].1 != value => entries[at].1 = value.to_string(),
            (None, Some(value)) => entries.push((key.to_string(), value.to_string())),
            (Some(at), None) => {
                entries.remove(at);
            }
            _ => return false,
        }

        ini.dirty = true;
        true
    })
}

/// Sets `key` in `section`, or removes it for None, and saves the file.
pub fn set(section: &str, key: &str, value: Option<&str>) {
    if change(section, key, value) {
        flush();
    }
}

/// Like `set` but leaves the file to `flush`, for a slider being dragged,
/// which changes its setting many times a second.
pub fn set_live(section: &str, key: &str, value: Option<&str>) {
    change(section, key, value);
}

/// Saves the file if a change has not been saved yet.
pub fn flush() {
    INI.with_borrow_mut(|ini| {
        if ini.dirty {
            ini.save();
        }
    });
}

/// A setting of the Settings section.
pub fn string_setting(key: &str) -> Option<String> {
    get(SECTION_SETTINGS, key).filter(|v| !v.is_empty())
}

pub fn set_string_setting(key: &str, value: Option<&str>) {
    set(SECTION_SETTINGS, key, value);
}

/// True if the Settings section holds any of `keys`: they are not at their
/// defaults.
pub fn has_any(keys: &[&str]) -> bool {
    keys.iter().any(|key| get(SECTION_SETTINGS, key).is_some())
}

/// A yes-or-no setting, written 1 or 0.
pub fn bool_setting(key: &str, default: bool) -> bool {
    get(SECTION_SETTINGS, key).map_or(default, |v| v == "1" || v.eq_ignore_ascii_case("true"))
}

pub fn set_bool_setting(key: &str, value: bool) {
    set(SECTION_SETTINGS, key, Some(if value { "1" } else { "0" }));
}

pub fn f64_setting(key: &str, default: f64) -> f64 {
    get(SECTION_SETTINGS, key).and_then(|v| v.parse().ok()).filter(|v: &f64| v.is_finite()).unwrap_or(default)
}

fn f32_setting(key: &str, default: f32) -> f32 {
    f64_setting(key, default as f64) as f32
}

/// The picture settings, the Mac's defaults where Juku.ini has none.
pub fn crt_settings() -> CrtSettings {
    let d = CrtSettings::default();

    CrtSettings {
        crt_effects: bool_setting(CRT_EFFECTS, d.crt_effects),
        curvature: f32_setting(CRT_CURVATURE, d.curvature),
        scanlines: f32_setting(CRT_SCANLINES, d.scanlines),
        glow: f32_setting(CRT_GLOW, d.glow),
        background: f32_setting(CRT_BACKGROUND, d.background),
        frame: f32_setting(CRT_FRAME, d.frame),
        room: f32_setting(CRT_ROOM, d.room),
        bezel3d: bool_setting(CRT_BEZEL_3D, d.bezel3d),
        monitor_front: bool_setting(CRT_MONITOR_FRONT, d.monitor_front),
        brightness: f32_setting(CRT_BRIGHTNESS, d.brightness),
    }
}

/// `DebuggerPort`, 5104 by default; None if it is not a port.
pub fn debugger_port() -> Option<u16> {
    get(SECTION_SETTINGS, DEBUGGER_PORT).map_or(Some(DEFAULT_DEBUGGER_PORT), |v| v.parse().ok().filter(|&p| p > 0))
}

/// Hands the drive sounds' settings to the machine (spec §4.5).
pub fn apply_drive_sounds(m: &mut juku::Machine) {
    m.set_drive_sounds(bool_setting(DRIVE_SOUNDS, true));
    m.set_drive_sounds_volume(f32_setting(DRIVE_SOUNDS_VOLUME, 1.0));
    m.set_drive_sounds_spread(f32_setting(DRIVE_SOUNDS_SPREAD, juku::drive_sound::SPREAD));
    m.set_drive_idle_mute(bool_setting(DRIVE_IDLE_MUTE, false));
}

/// Hands the T3.2 clock to the machine (MD §10.2).
pub fn apply_t32_clock(m: &mut juku::Machine) {
    m.set_t32_fast_clock(bool_setting(T32_FAST_CLOCK, false));
}

/// Replaces the whole of `section` and saves the file.
pub fn set_section(section: &str, entries: Vec<(String, String)>) {
    INI.with_borrow_mut(|ini| {
        *ini.section_mut(section) = entries;
        ini.save();
    });
}

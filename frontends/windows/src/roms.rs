//! The system ROMs and expansion ROMs (frontends/macos/src/Roms.m and the
//! ROM half of Main.m's applySettings, spec §6.7). The bundled system ROMs
//! from resources/roms are built into the exe; the user can also pick ROM
//! files. A system ROM is named by a string: a bundled ROM's file name
//! without `.bin`, or the absolute path of the user's file. Expansion ROMs
//! only come from the user's files, named by path; the core checks them.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use juku::Machine;
use juku::expansion::{self, ExpansionError};
use juku::machine::ROM_SIZE;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH};
use windows::core::{HSTRING, PCWSTR};

use crate::disks::Alert;
use crate::settings::{self, EXPANSION_ROM, KEY_CLICK, SYSTEM_ROM};
use crate::strings::localized;

/// A bundled system ROM: the full 16 KB, both EPROMs, as the core expects.
pub struct SystemRom {
    pub name: &'static str,
    /// What it prints at power-on: version and serial.
    title: &'static str,
    image: &'static [u8],
}

pub const SYSTEM_ROMS: [SystemRom; 3] = [
    SystemRom {
        name: "ekta31",
        title: "RomBios 3.43 #0031",
        image: include_bytes!("../../../resources/roms/ekta31.bin"),
    },
    SystemRom {
        name: "ekta37",
        title: "RomBios 3.43m #0037",
        image: include_bytes!("../../../resources/roms/ekta37.bin"),
    },
    SystemRom {
        name: "jmon33",
        title: "Monitor 3.3 + BASIC",
        image: include_bytes!("../../../resources/roms/jmon33.bin"),
    },
];

pub const DEFAULT_SYSTEM_ROM: &str = "ekta37";

pub const NOT_A_ROM: &str = "“%@” is not a Juku ROM image (16 KB expected).";
pub const CANNOT_READ_ROM: &str = "The ROM file “%@” could not be read.";

impl SystemRom {
    /// For the menu and the Settings list, e.g. "RomBios 3.43m #0037 (Default)".
    pub fn title(&self) -> String {
        if self.name == DEFAULT_SYSTEM_ROM {
            localized("%@ (Default)").replace("%@", self.title)
        } else {
            self.title.to_string()
        }
    }
}

/// The system ROM setting, the default when there is none.
pub fn system_rom_setting() -> String {
    settings::string_setting(SYSTEM_ROM).unwrap_or_else(|| DEFAULT_SYSTEM_ROM.to_string())
}

/// True if the ROM name is the path of a file of the user's.
pub fn is_rom_file(rom: &str) -> bool {
    Path::new(rom).is_absolute()
}

/// Reads a system ROM: a bundled one by name, or the user's file by path.
/// None if the name is unknown, or the file is missing or not 16 KB.
pub fn load_system_rom(rom: &str) -> Option<Cow<'static, [u8]>> {
    let image = if is_rom_file(rom) {
        Cow::Owned(std::fs::read(rom).ok()?)
    } else {
        Cow::Borrowed(SYSTEM_ROMS.iter().find(|r| r.name == rom)?.image)
    };
    (image.len() == ROM_SIZE).then_some(image)
}

/// The system ROM to start with, its image, and what to tell the user. A ROM
/// no longer in the app, or a file that has gone, falls back to the default,
/// and the setting is removed; a missing file is reported once the window
/// is up.
pub fn startup_rom() -> (String, Cow<'static, [u8]>, Option<Alert>) {
    let name = system_rom_setting();

    if let Some(image) = load_system_rom(&name) {
        return (name, image, None);
    }

    let alert = is_rom_file(&name).then(|| Alert::new(CANNOT_READ_ROM, Path::new(&name)));
    settings::set_string_setting(SYSTEM_ROM, None);
    let image = load_system_rom(DEFAULT_SYSTEM_ROM).unwrap_or_default();
    (DEFAULT_SYSTEM_ROM.to_string(), image, alert)
}

/// Why an expansion ROM was refused, as an alert format.
fn expansion_error(error: ExpansionError) -> &'static str {
    match error {
        ExpansionError::BadSize => "“%@” is not an expansion ROM image (32 KB at most).",
        ExpansionError::NoHeader => "“%@” is not an expansion ROM image: it has no program header the monitor can start.",
    }
}

/// Reads and checks the user's expansion ROM file, or says why it is no
/// good (an alert format).
pub fn load_expansion_rom(path: &str) -> Result<Vec<u8>, &'static str> {
    let image = std::fs::read(path).map_err(|_| CANNOT_READ_ROM)?;
    expansion::check(&image).map_err(expansion_error)?;
    Ok(image)
}

/// The ROMs in the machine, as the Mac's app delegate keeps them
/// (`_romName`, `_expansionPath`), and whether the key click can be changed
/// for that ROM.
pub struct InstalledRoms {
    system: String,
    expansion: Option<String>,
    key_click_available: bool,
    // The file chosen again: load it anyway, since it may have changed.
    reload_system: bool,
    reload_expansion: bool,
}

impl InstalledRoms {
    /// The machine has the system ROM `system` and no expansion ROM.
    pub fn new(system: String) -> InstalledRoms {
        InstalledRoms { system, expansion: None, key_click_available: true, reload_system: false, reload_expansion: false }
    }

    pub fn system(&self) -> &str {
        &self.system
    }

    pub fn expansion(&self) -> Option<&str> {
        self.expansion.as_deref()
    }

    pub fn key_click_available(&self) -> bool {
        self.key_click_available
    }

    /// A snapshot put this system ROM in the machine.
    pub fn set_system(&mut self, name: &str) {
        self.system = name.to_string();
        self.reload_system = false;
    }

    /// A snapshot put this expansion ROM in the machine, or none.
    pub fn set_expansion(&mut self, path: Option<&str>) {
        self.expansion = path.map(str::to_string);
        self.reload_expansion = false;
    }

    /// The next `apply` loads the ROM file `path` even if it is in already.
    pub fn reload_system(&mut self, path: &str) {
        self.reload_system = path == self.system;
    }

    pub fn reload_expansion(&mut self, path: &str) {
        self.reload_expansion = self.expansion.as_deref() == Some(path);
    }

    /// Brings the machine in line with the settings (Main.m, applySettings):
    /// a new system ROM or expansion ROM cold-starts it, like changing the
    /// hardware with the power off. A ROM that cannot be used is reported,
    /// and its setting goes back to what is in the machine. The key click
    /// patch is cheap and idempotent, and a new ROM needs it again.
    pub fn apply(&mut self, m: &mut Machine) -> Vec<Alert> {
        let mut alerts = Vec::new();
        let name = system_rom_setting();

        if name != self.system || std::mem::take(&mut self.reload_system) {
            match load_system_rom(&name) {
                Some(rom) if m.load_rom(&rom) => self.system = name,
                _ => {
                    if is_rom_file(&name) {
                        alerts.push(Alert::new(CANNOT_READ_ROM, Path::new(&name)));
                    }

                    settings::set_string_setting(SYSTEM_ROM, Some(&self.system));
                }
            }
        }

        let path = settings::string_setting(EXPANSION_ROM);

        if path != self.expansion || std::mem::take(&mut self.reload_expansion) {
            match &path {
                None => {
                    m.eject_expansion();
                    self.expansion = None;
                }
                Some(p) => match load_expansion_rom(p).and_then(|image| m.insert_expansion(&image).map_err(expansion_error)) {
                    Ok(()) => self.expansion = path,
                    Err(format) => {
                        alerts.push(Alert::new(format, Path::new(p)));
                        settings::set_string_setting(EXPANSION_ROM, self.expansion.as_deref());
                    }
                },
            }
        }

        self.key_click_available = m.set_key_click(settings::bool_setting(KEY_CLICK, true));
        alerts
    }
}

/// The open dialog for `.bin` and `.rom` files. None if the user cancels.
pub fn choose_rom_file(owner: HWND, title: &str) -> Option<PathBuf> {
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let name = HSTRING::from(localized("ROM Images"));
        let spec = HSTRING::from("*.bin;*.rom");
        dialog.SetFileTypes(&[COMDLG_FILTERSPEC { pszName: PCWSTR(name.as_ptr()), pszSpec: PCWSTR(spec.as_ptr()) }]).ok()?;
        dialog.SetOptions(dialog.GetOptions().ok()? | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM).ok()?;
        let _ = dialog.SetTitle(&HSTRING::from(localized(title)));
        dialog.Show(Some(owner)).ok()?;
        let item = dialog.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = PathBuf::from(name.to_string().ok()?);
        CoTaskMemFree(Some(name.0 as *const _));
        Some(path)
    }
}

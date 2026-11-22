//! The file types (spec §11.4), the counterpart of the Mac's document types in
//! Info.plist: Settings → Register File Types writes them for the current
//! user in `HKCU\Software\Classes`, so no administrator rights are needed.
//! The installer does the same (spec §11.7).
//!
//! - `.juk` opens in Juku (`SinijarvJuku.JukuEmu.DiskImage`); `.img`, `.dsk`
//!   and `.cpm` get Juku under Open With without being taken over, as the
//!   Mac's Alternate rank does.
//! - `.jukusnap` opens in Juku (`SinijarvJuku.JukuEmu.Snapshot`), and its
//!   thumbnails come from JukuThumbnail.dll beside Juku.exe.
//! - `Applications\Juku.exe` names the app and its types for Open With.
//!
//! The ProgIDs are the Mac's type identifiers without the hyphen, which
//! ProgIDs do not allow. Registering again after Juku has moved points them
//! at the new place.
//!
//! The installer (frontends/windows/installer) runs `Juku.exe --register`
//! and, on uninstall, `Juku.exe --unregister`, so the Settings button and the
//! installer write the same keys.

use std::path::PathBuf;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteKeyValueW, RegDeleteKeyW, RegDeleteTreeW, RegGetValueW, RegOpenKeyExW, RegQueryInfoKeyW, RegSetValueExW,
};
use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
use windows::core::{HSTRING, PCWSTR};

use crate::strings::localized;

const CLASSES: &str = r"Software\Classes";
const DISK_IMAGE: &str = "SinijarvJuku.JukuEmu.DiskImage";
const SNAPSHOT: &str = "SinijarvJuku.JukuEmu.Snapshot";
/// The shell's key for a type's thumbnail handler (IThumbnailProvider).
const THUMBNAIL_HANDLER: &str = "{e357fccd-a995-4576-b01f-234630154e96}";
/// JukuThumbnail.dll's class. Keep in step with frontends/windows/thumbnail.
const THUMBNAIL_CLASS: &str = "{df6fb75e-81fd-457d-be7c-f8bf50d7a27f}";
const THUMBNAIL_DLL: &str = "JukuThumbnail.dll";

/// Juku.exe, and the command that opens a file in it.
fn exe() -> Option<(PathBuf, String)> {
    let exe = std::env::current_exe().ok()?;
    let command = format!("\"{}\" \"%1\"", exe.display());
    Some((exe, command))
}

/// Writes a value under `HKCU\Software\Classes\<key>`: a string, or a DWORD.
fn set(key: &str, name: Option<&str>, value: Value) -> bool {
    let path = HSTRING::from(format!(r"{CLASSES}\{key}"));
    let mut hkey = HKEY::default();
    let created =
        unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, &path, None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, None, &mut hkey, None) };

    if created != ERROR_SUCCESS {
        return false;
    }

    let name = name.map(HSTRING::from);
    let name = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
    let written = match value {
        Value::Text(text) => {
            let wide: Vec<u8> = text.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
            unsafe { RegSetValueExW(hkey, name, None, REG_SZ, Some(&wide)) }
        }
        Value::Number(n) => unsafe { RegSetValueExW(hkey, name, None, REG_DWORD, Some(&n.to_le_bytes())) },
    };

    let _ = unsafe { RegCloseKey(hkey) };
    written == ERROR_SUCCESS
}

enum Value<'a> {
    Text(&'a str),
    Number(u32),
}

/// A string value under `HKCU\Software\Classes\<key>`.
fn get(key: &str, name: Option<&str>) -> Option<String> {
    let path = HSTRING::from(format!(r"{CLASSES}\{key}"));
    let name = name.map(HSTRING::from);
    let name = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
    let mut buffer = vec![0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    let read = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, &path, name, RRF_RT_REG_SZ, None, Some(buffer.as_mut_ptr().cast()), Some(&mut size))
    };
    (read == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buffer[..(size as usize / 2).saturating_sub(1)]))
}

fn classes_path(key: &str) -> HSTRING {
    HSTRING::from(format!(r"{CLASSES}\{key}"))
}

/// Deletes a key under `HKCU\Software\Classes` with all it holds. True if it
/// is gone.
fn delete_tree(key: &str) -> bool {
    let result = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &classes_path(key)) };
    result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND
}

/// Deletes one value; None is the key's default value.
fn delete_value(key: &str, name: Option<&str>) -> bool {
    let name = name.map(HSTRING::from);
    let name = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
    let result = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &classes_path(key), name) };
    result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND
}

/// Deletes a key that holds nothing any more, neither values nor subkeys:
/// what other apps keep under an extension stays.
fn delete_if_empty(key: &str) -> bool {
    let path = classes_path(key);
    let mut hkey = HKEY::default();

    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &path, None, KEY_QUERY_VALUE, &mut hkey) } != ERROR_SUCCESS {
        return true;
    }

    let (mut subkeys, mut values) = (0u32, 0u32);
    let queried = unsafe {
        RegQueryInfoKeyW(hkey, None, None, None, Some(&mut subkeys), None, None, Some(&mut values), None, None, None, None)
    };
    let _ = unsafe { RegCloseKey(hkey) };

    if queried != ERROR_SUCCESS || subkeys > 0 || values > 0 {
        return true;
    }

    unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, &path) == ERROR_SUCCESS }
}

/// True if the file types open in this copy of Juku.
pub fn registered_here() -> bool {
    exe().is_some_and(|(_, command)| get(&format!(r"{DISK_IMAGE}\shell\open\command"), None).as_deref() == Some(command.as_str()))
}

/// Registers the file types for this copy of Juku. False if a value could
/// not be written.
pub fn register() -> bool {
    let Some((exe, command)) = exe() else {
        return false;
    };

    let icon = format!("{},0", exe.display());
    let dll = exe.with_file_name(THUMBNAIL_DLL);
    let thumbnails = dll.is_file();
    let mut ok = true;
    let mut put = |key: &str, name: Option<&str>, value: Value| ok &= set(key, name, value);

    for (progid, title) in [(DISK_IMAGE, "Juku Disk Image"), (SNAPSHOT, "Juku Snapshot")] {
        put(progid, None, Value::Text(localized(title)));
        put(progid, Some("FriendlyTypeName"), Value::Text(localized(title)));
        put(&format!(r"{progid}\DefaultIcon"), None, Value::Text(&icon));
        put(&format!(r"{progid}\shell\open\command"), None, Value::Text(&command));
    }

    // The thumbnail is the picture itself: no frame or shadow around it.
    put(SNAPSHOT, Some("Treatment"), Value::Number(0));

    put(".juk", None, Value::Text(DISK_IMAGE));
    put(".jukusnap", None, Value::Text(SNAPSHOT));
    put(r".jukusnap\OpenWithProgids", Some(SNAPSHOT), Value::Text(""));

    for extension in [".juk", ".img", ".dsk", ".cpm"] {
        put(&format!(r"{extension}\OpenWithProgids"), Some(DISK_IMAGE), Value::Text(""));
    }

    if thumbnails {
        let dll = dll.display().to_string();
        put(&format!(r"CLSID\{THUMBNAIL_CLASS}"), None, Value::Text("Juku Snapshot Thumbnail"));
        put(&format!(r"CLSID\{THUMBNAIL_CLASS}\InprocServer32"), None, Value::Text(&dll));
        put(&format!(r"CLSID\{THUMBNAIL_CLASS}\InprocServer32"), Some("ThreadingModel"), Value::Text("Apartment"));
        put(&format!(r"{SNAPSHOT}\ShellEx\{THUMBNAIL_HANDLER}"), None, Value::Text(THUMBNAIL_CLASS));
        put(&format!(r".jukusnap\ShellEx\{THUMBNAIL_HANDLER}"), None, Value::Text(THUMBNAIL_CLASS));
    }

    let app = format!(r"Applications\{}", exe.file_name().map_or_else(|| "Juku.exe".into(), |n| n.to_string_lossy()));
    put(&app, Some("FriendlyAppName"), Value::Text("Juku"));
    put(&format!(r"{app}\DefaultIcon"), None, Value::Text(&icon));
    put(&format!(r"{app}\shell\open\command"), None, Value::Text(&command));

    for extension in [".juk", ".img", ".dsk", ".cpm", ".jukusnap"] {
        put(&format!(r"{app}\SupportedTypes"), Some(extension), Value::Text(""));
    }

    // Explorer reads the types again.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };

    if !thumbnails {
        log!("{} is not beside {}: no snapshot thumbnails", THUMBNAIL_DLL, exe.display());
    }

    ok
}

/// Removes the file types if they open in this copy of Juku; a copy
/// elsewhere that the user registered keeps them. Values other apps keep
/// under the same extensions stay. False if something could not be removed.
pub fn unregister() -> bool {
    let Some((exe, command)) = exe() else {
        return false;
    };

    if !registered_here() {
        return true;
    }

    let mut ok = delete_tree(DISK_IMAGE) & delete_tree(SNAPSHOT);
    let dll = exe.with_file_name(THUMBNAIL_DLL).display().to_string();
    let server = format!(r"CLSID\{THUMBNAIL_CLASS}");

    if get(&format!(r"{server}\InprocServer32"), None).as_deref() == Some(dll.as_str()) {
        ok &= delete_tree(&server);
    }

    let app = format!(r"Applications\{}", exe.file_name().map_or_else(|| "Juku.exe".into(), |n| n.to_string_lossy()));

    if get(&format!(r"{app}\shell\open\command"), None).as_deref() == Some(command.as_str()) {
        ok &= delete_tree(&app);
    }

    for (extension, progid) in [(".juk", DISK_IMAGE), (".img", DISK_IMAGE), (".dsk", DISK_IMAGE), (".cpm", DISK_IMAGE), (".jukusnap", SNAPSHOT)] {
        let open_with = format!(r"{extension}\OpenWithProgids");
        ok &= delete_value(&open_with, Some(progid)) & delete_if_empty(&open_with);

        if get(extension, None).as_deref() == Some(progid) {
            ok &= delete_value(extension, None);
        }
    }

    let handler = format!(r".jukusnap\ShellEx\{THUMBNAIL_HANDLER}");

    if get(&handler, None).as_deref() == Some(THUMBNAIL_CLASS) {
        ok &= delete_tree(&handler);
    }

    ok &= delete_if_empty(r".jukusnap\ShellEx");

    for extension in [".juk", ".img", ".dsk", ".cpm", ".jukusnap"] {
        ok &= delete_if_empty(extension);
    }

    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    ok
}

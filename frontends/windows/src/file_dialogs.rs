//! The open and save dialogs for snapshots and disk images (the Mac's
//! NSOpenPanel and NSSavePanel, spec §11.2), on `IFileOpenDialog` and
//! `IFileSaveDialog`.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOLDERID_Documents, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, FileOpenDialog, FileSaveDialog, IFileDialog,
    IFileOpenDialog, IFileSaveDialog, IShellItem, KF_FLAG_DEFAULT, SHCreateItemFromParsingName, SHGetKnownFolderPath, SIGDN_FILESYSPATH,
};
use windows::core::{HSTRING, PCWSTR};

use crate::strings::localized;

/// The files a dialog shows: a localized name and the patterns.
pub struct Filter {
    pub name: &'static str,
    pub spec: &'static str,
}

pub const DISK_IMAGES: Filter = Filter { name: "Disk Images", spec: "*.juk;*.img;*.dsk;*.cpm" };
pub const SNAPSHOTS: Filter = Filter { name: "Juku Snapshots", spec: "*.jukusnap" };

/// Where a dialog opens: `Default` is used only until the user has picked a
/// folder in this kind of dialog, which Windows then remembers; `Always`
/// opens it there.
pub enum Folder<'a> {
    Default(&'a Path),
    Always(&'a Path),
}

fn prepare(dialog: &IFileDialog, title: &str, filter: &Filter, folder: Option<Folder>) -> Option<()> {
    let name = HSTRING::from(localized(filter.name));
    let spec = HSTRING::from(filter.spec);

    unsafe {
        dialog.SetFileTypes(&[COMDLG_FILTERSPEC { pszName: PCWSTR(name.as_ptr()), pszSpec: PCWSTR(spec.as_ptr()) }]).ok()?;
        dialog.SetOptions(dialog.GetOptions().ok()? | FOS_FORCEFILESYSTEM).ok()?;
        let _ = dialog.SetTitle(&HSTRING::from(localized(title).trim_end_matches('…')));

        match folder {
            Some(Folder::Default(path)) => {
                if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(path), None) {
                    let _ = dialog.SetDefaultFolder(&item);
                }
            }
            Some(Folder::Always(path)) => {
                if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(path), None) {
                    let _ = dialog.SetFolder(&item);
                }
            }
            None => {}
        }
    }

    Some(())
}

fn result(dialog: &IFileDialog) -> Option<PathBuf> {
    unsafe {
        let item = dialog.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = PathBuf::from(name.to_string().ok()?);
        CoTaskMemFree(Some(name.0 as *const _));
        Some(path)
    }
}

/// The user's Documents folder.
pub fn documents_folder() -> Option<PathBuf> {
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None).ok()?;
        let folder = path.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(path.0 as *const _));
        folder
    }
}

/// An existing file, or None if the user cancels. `title` is a string key.
pub fn open_file(owner: HWND, title: &str, filter: &Filter, folder: Option<Folder>) -> Option<PathBuf> {
    let dialog: IFileOpenDialog = unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }.ok()?;
    prepare(&dialog.clone().into(), title, filter, folder)?;

    unsafe {
        dialog.SetOptions(dialog.GetOptions().ok()? | FOS_FILEMUSTEXIST).ok()?;
        dialog.Show(Some(owner)).ok()?;
    }

    result(&dialog.into())
}

/// A file to save as, proposed as `name` with the extension `extension`, or
/// None if the user cancels. Windows asks before a file is replaced.
pub fn save_file(owner: HWND, title: &str, filter: &Filter, extension: &str, folder: Option<Folder>, name: &str) -> Option<PathBuf> {
    let dialog: IFileSaveDialog = unsafe { CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER) }.ok()?;
    prepare(&dialog.clone().into(), title, filter, folder)?;

    unsafe {
        dialog.SetOptions(dialog.GetOptions().ok()? | FOS_OVERWRITEPROMPT).ok()?;
        dialog.SetDefaultExtension(&HSTRING::from(extension)).ok()?;
        dialog.SetFileName(&HSTRING::from(name)).ok()?;
        dialog.Show(Some(owner)).ok()?;
    }

    result(&dialog.into())
}

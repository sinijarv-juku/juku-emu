//! Disk images inserted lately, newest first (frontends/macos/src/RecentDisks.m,
//! spec §6.2, §11.4), and file references, the Windows counterpart of the
//! Mac's bookmarks.
//!
//! A reference is a file's path plus its volume serial number and 128-bit
//! NTFS file ID. The path is tried first: Juku's own write-back replaces an
//! image file with a new one, which has a new ID (the drives are saved again
//! afterwards). When the path no longer finds a file, the ID finds it
//! wherever it was moved or renamed on the same volume.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ExtendedFileIdType, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_128, FILE_ID_DESCRIPTOR, FILE_ID_DESCRIPTOR_0, FILE_ID_INFO,
    FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdInfo,
    GetFileInformationByHandleEx, GetFinalPathNameByHandleW, OPEN_EXISTING, OpenFileById,
};
use windows::Win32::UI::Shell::{SHARD_PATHW, SHAddToRecentDocs};
use windows::core::{HSTRING, PCWSTR};

use crate::settings::{self, SECTION_RECENT_DISKS};

/// Longest list in Disks → Recent Images.
const MAX_RECENT_DISKS: usize = 10;

/// A file's identity: the volume's serial number and the file's ID on it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FileId {
    volume: u64,
    id: [u8; 16],
}

impl FileId {
    fn of_handle(handle: HANDLE) -> Option<FileId> {
        let mut info = FILE_ID_INFO::default();
        unsafe { GetFileInformationByHandleEx(handle, FileIdInfo, (&raw mut info).cast(), size_of::<FILE_ID_INFO>() as u32).ok()? };
        Some(FileId { volume: info.VolumeSerialNumber, id: info.FileId.Identifier })
    }

    /// The identity of the file or folder at `path`.
    pub fn of(path: &Path) -> Option<FileId> {
        let handle = open_for_attributes(path)?;
        let id = FileId::of_handle(handle);
        let _ = unsafe { CloseHandle(handle) };
        id
    }

    /// As stored: the volume serial and the ID in hex, `volume:id`.
    fn encode(&self) -> String {
        let id: String = self.id.iter().map(|b| format!("{b:02X}")).collect();
        format!("{:016X}:{id}", self.volume)
    }

    fn decode(text: &str) -> Option<FileId> {
        let (volume, id) = text.split_once(':')?;
        let volume = u64::from_str_radix(volume, 16).ok()?;
        let mut bytes = [0u8; 16];

        if id.len() != 32 {
            return None;
        }

        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&id[2 * i..2 * i + 2], 16).ok()?;
        }

        Some(FileId { volume, id: bytes })
    }
}

/// A handle for reading a file's or folder's attributes only.
fn open_for_attributes(path: &Path) -> Option<HANDLE> {
    unsafe {
        CreateFileW(
            &HSTRING::from(path),
            FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .ok()
}

/// The path of an open file, without the `\\?\` prefix.
fn final_path(handle: HANDLE) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 1024];
    let len = unsafe { GetFinalPathNameByHandleW(handle, &mut buffer, FILE_NAME_NORMALIZED) } as usize;

    if len == 0 || len >= buffer.len() {
        return None;
    }

    let path = String::from_utf16_lossy(&buffer[..len]);
    let path = match path.strip_prefix(r"\\?\UNC\") {
        Some(share) => format!(r"\\{share}"),
        None => path.strip_prefix(r"\\?\").unwrap_or(&path).to_string(),
    };
    Some(PathBuf::from(path))
}

/// True if `a` and `b` are the same file, whatever paths name them.
pub fn same_file(a: &Path, b: &Path) -> bool {
    match (FileId::of(a), FileId::of(b)) {
        (Some(a), Some(b)) => a == b,
        _ => a == b,
    }
}

/// A file remembered across launches.
#[derive(Clone)]
pub struct FileRef {
    pub path: PathBuf,
    id: Option<FileId>,
}

impl FileRef {
    pub fn new(path: &Path) -> FileRef {
        FileRef { path: path.to_path_buf(), id: FileId::of(path) }
    }

    /// The entries `<key>` (the path) and `<key>.FileID` of a section.
    pub fn to_entries(&self, key: &str) -> Vec<(String, String)> {
        let mut entries = vec![(key.to_string(), self.path.display().to_string())];

        if let Some(id) = self.id {
            entries.push((format!("{key}.FileID"), id.encode()));
        }

        entries
    }

    pub fn from_entries(entries: &[(String, String)], key: &str) -> Option<FileRef> {
        let value = |k: &str| entries.iter().find(|(name, _)| name == k).map(|(_, v)| v.as_str());
        let path = value(key).filter(|p| !p.is_empty())?;
        let id = value(&format!("{key}.FileID")).and_then(FileId::decode);
        Some(FileRef { path: PathBuf::from(path), id })
    }

    /// Where the file is now: at its path, or else wherever its ID finds it
    /// on the same volume. None if it has gone.
    pub fn resolve(&self) -> Option<PathBuf> {
        if self.path.is_file() {
            return Some(self.path.clone());
        }

        let id = self.id?;
        // Any handle on the volume will do: the root of the old path's one.
        let root = self.path.ancestors().last()?;
        let volume = open_for_attributes(root)?;
        let path = (FileId::of_handle(volume).map(|v| v.volume) == Some(id.volume))
            .then(|| {
                let descriptor = FILE_ID_DESCRIPTOR {
                    dwSize: size_of::<FILE_ID_DESCRIPTOR>() as u32,
                    Type: ExtendedFileIdType,
                    Anonymous: FILE_ID_DESCRIPTOR_0 { ExtendedFileId: FILE_ID_128 { Identifier: id.id } },
                };
                let file = unsafe {
                    OpenFileById(
                        volume,
                        &descriptor,
                        FILE_READ_ATTRIBUTES.0,
                        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                        None,
                        FILE_FLAG_BACKUP_SEMANTICS,
                    )
                }
                .ok()?;
                let path = final_path(file);
                let _ = unsafe { CloseHandle(file) };
                path
            })
            .flatten();
        let _ = unsafe { CloseHandle(volume) };
        path.filter(|p| p.is_file())
    }
}

/// The stored list, as references, newest first.
fn stored() -> Vec<FileRef> {
    let entries = settings::section(SECTION_RECENT_DISKS);
    (1..=MAX_RECENT_DISKS).filter_map(|n| FileRef::from_entries(&entries, &n.to_string())).collect()
}

fn store(list: &[FileRef]) {
    let entries = list.iter().enumerate().flat_map(|(n, r)| r.to_entries(&(n + 1).to_string())).collect();
    settings::set_section(SECTION_RECENT_DISKS, entries);
}

/// Puts `path` at the top of the list, and on the taskbar's jump list.
pub fn note(path: &Path) {
    let mut list = vec![FileRef::new(path)];

    for old in stored() {
        if let Some(found) = old.resolve()
            && !same_file(&found, path)
            && list.len() < MAX_RECENT_DISKS
        {
            list.push(FileRef { path: found, ..old });
        }
    }

    store(&list);
    add_to_jump_list(path);
}

/// Puts a file Juku opened or saved on the taskbar's jump list (the Mac's
/// recent documents), which shows it once the file types are registered.
pub fn add_to_jump_list(path: &Path) {
    let wide = HSTRING::from(path);
    unsafe { SHAddToRecentDocs(SHARD_PATHW.0 as u32, Some(PCWSTR(wide.as_ptr()).0.cast())) };
}

/// Empties the list. The jump list is left alone: Windows can clear only
/// every app's recent documents at once.
pub fn clear() {
    store(&[]);
}

/// The images in the list that can still be found, newest first.
pub fn images() -> Vec<PathBuf> {
    stored().iter().filter_map(FileRef::resolve).collect()
}

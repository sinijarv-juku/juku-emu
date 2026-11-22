//! The Explorer thumbnail handler for snapshots (spec §11.6), the counterpart
//! of frontends/macos/src/thumbnail/Thumbnail.m. A snapshot is a PNG whose
//! picture is the screen, so the thumbnail is that picture, scaled to the
//! size asked for. WIC reads it by its PNG signature, whatever the file's
//! extension, and skips the state in the private chunks.
//!
//! JukuThumbnail.dll is an in-process COM server with one class, an
//! `IThumbnailProvider` taking its file through `IInitializeWithStream`.
//! Juku's Settings registers it (frontends/windows/src/file_types.rs);
//! Explorer runs it in an isolated process of its own.

// The DLL is named as Windows names DLLs.
#![allow(non_snake_case)]

use std::ffi::c_void;
use std::sync::Mutex;

use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_POINTER, S_FALSE};
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, HBITMAP, HGDIOBJ};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA, IWICImagingFactory, WICBitmapDitherTypeNone,
    WICBitmapInterpolationModeHighQualityCubic, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IClassFactory, IClassFactory_Impl, IStream};
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithStream, IInitializeWithStream_Impl};
use windows::Win32::UI::Shell::{IThumbnailProvider, IThumbnailProvider_Impl, WTS_ALPHATYPE, WTSAT_RGB};
use windows::core::{BOOL, Error, GUID, HRESULT, IUnknown, Interface, Ref, Result, implement};

/// The handler's class. Keep in step with frontends/windows/src/file_types.rs.
pub const CLSID_JUKU_THUMBNAIL: GUID = GUID::from_u128(0xdf6fb75e_81fd_457d_be7c_f8bf50d7a27f);

/// Larger files are not snapshots Juku made (about 1 MB with two disks).
const MAX_FILE: usize = 16 << 20;

#[implement(IInitializeWithStream, IThumbnailProvider)]
struct Provider {
    file: Mutex<Option<Vec<u8>>>,
}

impl IInitializeWithStream_Impl for Provider_Impl {
    /// Reads the whole file: the stream is only good during this call.
    fn Initialize(&self, stream: Ref<IStream>, _mode: u32) -> Result<()> {
        let stream = stream.ok()?;
        let mut file = Vec::new();
        let mut chunk = vec![0u8; 1 << 16];

        loop {
            let mut read = 0u32;
            unsafe { stream.Read(chunk.as_mut_ptr().cast(), chunk.len() as u32, Some(&mut read)) }.ok()?;

            if read == 0 {
                break;
            }

            file.extend_from_slice(&chunk[..read as usize]);

            if file.len() > MAX_FILE {
                return Err(E_FAIL.into());
            }
        }

        *self.file.lock().map_err(|_| Error::from(E_FAIL))? = Some(file);
        Ok(())
    }
}

impl IThumbnailProvider_Impl for Provider_Impl {
    /// The picture, its longer side `cx` pixels, as an opaque 32-bit bitmap.
    fn GetThumbnail(&self, cx: u32, bitmap: *mut HBITMAP, alpha: *mut WTS_ALPHATYPE) -> Result<()> {
        if bitmap.is_null() || alpha.is_null() {
            return Err(E_POINTER.into());
        }

        let file = self.file.lock().map_err(|_| Error::from(E_FAIL))?;
        let file = file.as_deref().ok_or(Error::from(E_FAIL))?;
        let (width, height, pixels) = scaled_picture(file, cx.max(1))?;

        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32), // top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let dib = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)? };

        if bits.is_null() {
            let _ = unsafe { DeleteObject(HGDIOBJ(dib.0)) };
            return Err(E_FAIL.into());
        }

        unsafe {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
            *bitmap = dib;
            *alpha = WTSAT_RGB;
        }

        Ok(())
    }
}

/// The PNG's picture scaled so that its longer side is `size`, as BGRA rows.
fn scaled_picture(file: &[u8], size: u32) -> Result<(u32, u32, Vec<u8>)> {
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = factory.CreateStream()?;
        stream.InitializeFromMemory(file)?;
        let decoder = factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let frame = decoder.GetFrame(0)?;
        let (mut width, mut height) = (0, 0);
        frame.GetSize(&mut width, &mut height)?;

        if width == 0 || height == 0 {
            return Err(E_FAIL.into());
        }

        let scale = size as f64 / width.max(height) as f64;
        let (w, h) = (((width as f64 * scale).round() as u32).max(1), ((height as f64 * scale).round() as u32).max(1));
        let scaler = factory.CreateBitmapScaler()?;
        scaler.Initialize(&frame, w, h, WICBitmapInterpolationModeHighQualityCubic)?;
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(&scaler, &GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        converter.CopyPixels(std::ptr::null(), w * 4, &mut pixels)?;
        Ok((w, h, pixels))
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Ref<IUnknown>, iid: *const GUID, object: *mut *mut c_void) -> Result<()> {
        if object.is_null() || iid.is_null() {
            return Err(E_POINTER.into());
        }

        unsafe { *object = std::ptr::null_mut() };

        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }

        let provider: IUnknown = Provider { file: Mutex::new(None) }.into();
        unsafe { provider.query(iid, object) }.ok()
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

/// COM asks the DLL for its class's factory.
///
/// # Safety
/// The arguments are COM's: valid pointers to GUIDs and an out pointer.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(clsid: *const GUID, iid: *const GUID, object: *mut *mut c_void) -> HRESULT {
    if clsid.is_null() || iid.is_null() || object.is_null() {
        return E_POINTER;
    }

    unsafe {
        *object = std::ptr::null_mut();

        if *clsid != CLSID_JUKU_THUMBNAIL {
            return CLASS_E_CLASSNOTAVAILABLE;
        }

        let factory: IClassFactory = Factory.into();
        factory.query(iid, object)
    }
}

/// The DLL stays loaded as long as its process: it holds no state that
/// would make unloading worth the bookkeeping.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

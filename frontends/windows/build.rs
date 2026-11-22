//! Build script of the Windows front-end (spec §11.1):
//!
//! - compiles `src/Shaders.hlsl` to shader model 5.1 with `d3dcompiler_47.dll`,
//!   which is part of Windows, into `$OUT_DIR/<entry>.cso` for include_bytes!;
//! - writes `$OUT_DIR/version.h` from the repository's VERSION, which the Mac
//!   app takes too, and the Mac app's copyright, and passes the version as
//!   the app shows it to the crate as JUKU_VERSION;
//! - runs `rc.exe` from the newest Windows SDK on `resources/Juku.rc` (the
//!   manifest and the version information) and links the `.res` file.
//!
//! It uses no crates: the few Windows functions it needs are declared here.

use std::env;
use std::ffi::c_void;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// The shader entry points and their targets.
const SHADERS: [(&str, &str); 6] = [
    ("vs_fullscreen", "vs_5_1"),
    ("ps_decode", "ps_5_1"),
    ("ps_persist", "ps_5_1"),
    ("ps_downsample", "ps_5_1"),
    ("ps_crt", "ps_5_1"),
    ("ps_flat", "ps_5_1"),
];

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let debug = env::var("PROFILE").as_deref() == Ok("debug");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/Shaders.hlsl");
    println!("cargo:rerun-if-changed=resources");
    println!("cargo:rerun-if-changed=../macos/resources/Info.plist");
    println!("cargo:rerun-if-changed=../../VERSION");
    // A -dev build shows its commit: built again when HEAD moves.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");

    compile_shaders(Path::new("src/Shaders.hlsl"), &out, debug);
    write_version(Path::new("../../VERSION"), Path::new("../macos/resources/Info.plist"), &out);
    compile_resources(Path::new("resources"), &out);
}

// ---- Shaders ----------------------------------------------------------------

#[repr(C)]
struct BlobVtbl {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut Blob) -> u32,
    get_buffer_pointer: unsafe extern "system" fn(*mut Blob) -> *const u8,
    get_buffer_size: unsafe extern "system" fn(*mut Blob) -> usize,
}

/// ID3DBlob, as far as it is used here.
#[repr(C)]
struct Blob {
    vtbl: *const BlobVtbl,
}

impl Blob {
    /// The blob's bytes; releases it.
    unsafe fn take(blob: *mut Blob) -> Vec<u8> {
        if blob.is_null() {
            return Vec::new();
        }

        unsafe {
            let vtbl = &*(*blob).vtbl;
            let bytes = std::slice::from_raw_parts((vtbl.get_buffer_pointer)(blob), (vtbl.get_buffer_size)(blob)).to_vec();
            (vtbl.release)(blob);
            bytes
        }
    }
}

type D3DCompile = unsafe extern "system" fn(
    src: *const c_void,
    len: usize,
    name: *const u8,
    defines: *const c_void,
    include: *const c_void,
    entry: *const u8,
    target: *const u8,
    flags1: u32,
    flags2: u32,
    code: *mut *mut Blob,
    errors: *mut *mut Blob,
) -> i32;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> Option<unsafe extern "system" fn()>;
}

const D3DCOMPILE_DEBUG: u32 = 1 << 0;
const D3DCOMPILE_SKIP_OPTIMIZATION: u32 = 1 << 2;
const D3DCOMPILE_ENABLE_STRICTNESS: u32 = 1 << 11;
const D3DCOMPILE_OPTIMIZATION_LEVEL3: u32 = 1 << 15;
const D3DCOMPILE_WARNINGS_ARE_ERRORS: u32 = 1 << 18;

fn compile_shaders(source: &Path, out: &Path, debug: bool) {
    let hlsl = fs::read(source).expect("src/Shaders.hlsl");
    let name: Vec<u16> = "d3dcompiler_47.dll\0".encode_utf16().collect();
    let module = unsafe { LoadLibraryW(name.as_ptr()) };
    assert!(!module.is_null(), "d3dcompiler_47.dll is missing");
    let compile: D3DCompile = unsafe {
        std::mem::transmute(GetProcAddress(module, c"D3DCompile".as_ptr().cast()).expect("D3DCompile"))
    };

    let mut flags = D3DCOMPILE_ENABLE_STRICTNESS | D3DCOMPILE_WARNINGS_ARE_ERRORS;
    flags |= if debug { D3DCOMPILE_DEBUG | D3DCOMPILE_SKIP_OPTIMIZATION } else { D3DCOMPILE_OPTIMIZATION_LEVEL3 };
    let source_name = format!("{}\0", source.display());

    for (entry, target) in SHADERS {
        let entry_z = format!("{entry}\0");
        let target_z = format!("{target}\0");
        let mut code = std::ptr::null_mut();
        let mut errors = std::ptr::null_mut();
        let hr = unsafe {
            compile(
                hlsl.as_ptr().cast(),
                hlsl.len(),
                source_name.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                entry_z.as_ptr(),
                target_z.as_ptr(),
                flags,
                0,
                &mut code,
                &mut errors,
            )
        };
        let messages = unsafe { Blob::take(errors) };
        let messages = String::from_utf8_lossy(&messages);
        let code = unsafe { Blob::take(code) };

        if hr < 0 {
            panic!("{entry}: shader compilation failed (HRESULT {hr:#010x}):\n{messages}");
        }

        for line in messages.lines() {
            println!("cargo:warning={entry}: {line}");
        }

        fs::write(out.join(format!("{entry}.cso")), code).unwrap();
    }
}

// ---- Version ------------------------------------------------------------------

/// The `<string>` value that follows `<key>key</key>` in a plist.
fn plist_string(plist: &str, key: &str) -> String {
    let tag = format!("<key>{key}</key>");
    let rest = &plist[plist.find(&tag).unwrap_or_else(|| panic!("{key} in Info.plist")) + tag.len()..];
    let start = rest.find("<string>").unwrap() + "<string>".len();
    let end = rest.find("</string>").unwrap();
    rest[start..end].trim().to_string()
}

/// A string literal for rc: quotes doubled, backslashes escaped.
fn rc_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\"\""))
}

/// The version from `version_file` (spec §3.3), the copyright from the Mac
/// app's Info.plist.
fn write_version(version_file: &Path, plist: &Path, out: &Path) {
    let version = fs::read_to_string(version_file).expect("VERSION").trim().to_string();
    let plist = fs::read_to_string(plist).expect("Info.plist");
    let copyright = plist_string(&plist, "NSHumanReadableCopyright");

    // FILEVERSION a,b,c,d: MAJOR.MINOR.PATCH, then the build number, 0 for
    // a -dev version and 1 for a release, so that a -dev version comes
    // before its release.
    let dev = version.ends_with("-dev");
    let numbers = version.strip_suffix("-dev").unwrap_or(&version);
    let plain = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) && (p == "0" || !p.starts_with('0'));
    let mut parts: Vec<u16> = numbers.split('.').filter(|p| plain(p)).filter_map(|p| p.parse().ok()).collect();
    if parts.len() != 3 || numbers.split('.').count() != 3 {
        panic!("VERSION: {version} is not MAJOR.MINOR.PATCH, or that and -dev");
    }
    parts.push(if dev { 0 } else { 1 });
    let number = parts.iter().map(u16::to_string).collect::<Vec<_>>().join(",");

    // What the app shows: a -dev build with its commit, 0.5.0-dev+g046bcd2.
    let commit = Command::new("git").args(["rev-parse", "--short", "HEAD"]).output().ok().filter(|o| o.status.success());
    let shown = match commit {
        Some(o) if dev => format!("{version}+g{}", String::from_utf8_lossy(&o.stdout).trim()),
        _ => version.clone(),
    };

    // "© 2026 Name" → "Name": the publisher, for CompanyName and the
    // installer.
    let author = copyright.trim_start_matches(|c: char| c == '©' || c.is_ascii_digit() || " -–,".contains(c)).trim();

    let header = format!(
        "#define JUKU_VERSION {}\n#define JUKU_VERSION_NUMBER {number}\n#define JUKU_COPYRIGHT {}\n#define JUKU_AUTHOR {}\n",
        rc_string(&version),
        rc_string(&copyright),
        rc_string(author)
    );
    fs::write(out.join("version.h"), header).unwrap();
    println!("cargo:rustc-env=JUKU_VERSION={shown}");
    println!("cargo:rustc-env=JUKU_COPYRIGHT={copyright}");
}

// ---- Resources ------------------------------------------------------------------

/// rc.exe of the newest Windows SDK, for this computer's architecture.
fn find_rc() -> PathBuf {
    if let Ok(rc) = env::var("RC") {
        return rc.into();
    }

    let output = Command::new("reg")
        .args(["query", r"HKLM\SOFTWARE\Microsoft\Windows Kits\Installed Roots", "/v", "KitsRoot10", "/reg:32"])
        .output()
        .expect("reg query");
    let output = String::from_utf8_lossy(&output.stdout);
    let root = output
        .lines()
        .find_map(|l| l.trim().strip_prefix("KitsRoot10"))
        .and_then(|l| l.trim().strip_prefix("REG_SZ"))
        .map(|l| PathBuf::from(l.trim()))
        .expect("the Windows SDK is not installed (KitsRoot10)");
    let arch = if env::var("PROCESSOR_ARCHITECTURE").as_deref() == Ok("ARM64") { "arm64" } else { "x64" };

    let mut versions: Vec<PathBuf> = fs::read_dir(root.join("bin"))
        .expect("Windows SDK bin")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join(arch).join("rc.exe").is_file())
        .collect();
    versions.sort_by_key(|p| {
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        name.split('.').map(|n| n.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>()
    });
    versions.last().expect("rc.exe in the Windows SDK").join(arch).join("rc.exe")
}

fn compile_resources(dir: &Path, out: &Path) {
    let res = out.join("Juku.res");
    let status = Command::new(find_rc())
        .current_dir(dir)
        .arg("/nologo")
        .arg("/i")
        .arg(out)
        .arg("/fo")
        .arg(&res)
        .arg("Juku.rc")
        .status()
        .expect("rc.exe");
    assert!(status.success(), "rc.exe failed");

    println!("cargo:rustc-link-arg-bins={}", res.display());
    // The manifest is a resource of ours; the linker must not add another.
    println!("cargo:rustc-link-arg-bins=/MANIFEST:NO");
}

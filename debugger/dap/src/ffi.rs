//! C ABI of the debug server, for the macOS app (spec §12.2). Keep in sync
//! with the declarations in `frontends/macos/src/Core.m`. All functions must
//! be called from the emulation thread.

use core::ptr;

use juku::Machine;

use crate::server::{Server, State};

/// Bit 0: a client is connected. Bit 1: the debugger has stopped the
/// machine.
fn bits(s: State) -> u32 {
    s.connected as u32 | (s.stopped as u32) << 1
}

/// Starts listening on 127.0.0.1 at `port`. Returns NULL if the port cannot
/// be had.
#[unsafe(no_mangle)]
pub extern "C" fn juku_dap_new(port: u16) -> *mut Server {
    match Server::start(port) {
        Ok(server) => Box::into_raw(Box::new(server)),
        Err(_) => ptr::null_mut(),
    }
}

/// Stops the server, ending the session with `m` if there is one: the
/// machine runs on.
///
/// # Safety
/// `d` must come from [`juku_dap_new`] and not be used afterwards (or be
/// NULL); `m` must be the valid machine it served.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_dap_free(d: *mut Server, m: *mut Machine) {
    if !d.is_null() {
        let mut server = unsafe { Box::from_raw(d) };
        server.end(unsafe { &mut *m });
    }
}

/// Takes a client, handles its messages and reports stops (`Server::poll`).
/// Call it on every tick, after running the machine. Returns the state
/// bits.
///
/// # Safety
/// `d` and `m` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_dap_poll(d: *mut Server, m: *mut Machine) -> u32 {
    bits(unsafe { &mut *d }.poll(unsafe { &mut *m }))
}

/// Runs the machine for `cycles` and does the server's work
/// (`Server::run`): use it in place of `juku_run_cycles` while the server is
/// on. Returns the state bits; `*ran` is set to the cycles run.
///
/// # Safety
/// `d` and `m` must be valid; `ran` must be valid or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_dap_run(d: *mut Server, m: *mut Machine, cycles: u64, ran: *mut u64) -> u32 {
    let (d, m) = (unsafe { &mut *d }, unsafe { &mut *m });
    let done = d.run(m, cycles);
    if !ran.is_null() {
        unsafe { *ran = done };
    }
    bits(d.poll(m))
}

/// A snapshot the session asks the app to open for a launch (§12.7): its
/// path, in UTF-8, is written into `buf` and its length returned; 0 if there
/// is none. A path longer than `cap` is cut short, and will not open. The
/// app then opens it as the user would, and calls
/// [`juku_dap_snapshot_opened`].
///
/// # Safety
/// `d` must be valid, and `buf` point to `cap` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_dap_snapshot_request(d: *mut Server, buf: *mut u8, cap: usize) -> usize {
    let Some(path) = unsafe { &mut *d }.take_request() else {
        return 0;
    };
    let text = path.to_string_lossy();
    let n = text.len().min(cap);
    unsafe { core::ptr::copy_nonoverlapping(text.as_ptr(), buf, n) };
    n
}

/// The app has opened the snapshot asked for, or could not. Returns the
/// state bits.
///
/// # Safety
/// `d` and `m` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_dap_snapshot_opened(d: *mut Server, m: *mut Machine, opened: bool) -> u32 {
    bits(unsafe { &mut *d }.snapshot_opened(unsafe { &mut *m }, opened))
}

/// The app's Pause while a client is connected (`Server::set_paused`).
/// Returns the state bits.
///
/// # Safety
/// `d` and `m` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn juku_dap_set_paused(d: *mut Server, m: *mut Machine, paused: bool) -> u32 {
    bits(unsafe { &mut *d }.set_paused(unsafe { &mut *m }, paused))
}

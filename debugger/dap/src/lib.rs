//! The guest debugger's server (spec §12): the Debug Adapter Protocol over
//! TCP, so that VS Code and other DAP clients can stop, step and inspect the
//! emulated machine.
//!
//! No dependencies but the core, std and juku-sdk's `juku-json`, JSON and
//! DAP's framing, which is `json` here. base64 is written here (§12.2).

#![deny(unsafe_op_in_unsafe_fn)]

pub mod base64;
mod expr;
pub mod ffi;
pub use juku_json as json;
mod info;
mod launch;
mod server;
mod session;
mod text;

pub use server::{Server, State};

// The core's C ABI goes into this crate's static library with it.
pub use juku;

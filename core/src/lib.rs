//! Juku E5104 emulation core.
//!
//! Platform independent: no OS APIs are used here. The macOS app talks to the
//! core only through the C ABI in [`ffi`]; the Windows app, written in Rust,
//! uses the crate's own API.

#![deny(unsafe_op_in_unsafe_fn)]

pub mod audio;
pub mod cpu;
pub mod debug;
pub mod dev;
pub mod drive_sound;
pub mod expansion;
pub mod ffi;
pub mod floppy;
pub mod keyboard;
pub mod lz;
pub mod machine;
pub mod mouse;
pub mod snapshot;
pub mod video;

pub use machine::Machine;
pub use video::Frame;

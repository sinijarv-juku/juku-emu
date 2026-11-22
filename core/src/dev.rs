//! Peripheral chips. Each is a plain state machine with no knowledge of how
//! it is wired; the wiring lives in [`crate::machine`].

pub mod fdc1793;
pub mod pic8259;
pub mod pit8253;
pub mod ppi8255;
pub mod usart8251;

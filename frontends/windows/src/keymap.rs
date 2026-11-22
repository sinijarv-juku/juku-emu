//! Positional mapping from scan codes to Juku keys (MD §6.3, spec §6.4 and
//! §11.3), the same table as frontends/macos/src/Keymap.m. A scan code is
//! bits 16–23 of a key message's lParam, with the extended-key flag (bit 24)
//! as 0x100 here. Scan codes name positions, not characters, so the mapping
//! does not depend on the keyboard layout.

use juku::keyboard::key_id;

/// The extended-key flag.
pub const EXT: u16 = 0x100;

// Scan codes with a meaning of their own (emu_window.rs handles them).
pub const LEFT_SHIFT: u16 = 0x2A;
pub const RIGHT_SHIFT: u16 = 0x36;
pub const LEFT_CTRL: u16 = 0x1D;
pub const RIGHT_CTRL: u16 = EXT | 0x1D;
pub const LEFT_ALT: u16 = 0x38;
pub const RIGHT_ALT: u16 = EXT | 0x38;

/// `; +` is an extra key to the right of `/` on the Juku, with no host key
/// in its place. It is Right Alt, as Right Option is on the Mac, and keypad +.
pub const SEMICOLON_PLUS: u8 = key_id(11, 1);

const ENTRIES: &[(u16, u8, u8)] = &[
    // Letters
    (0x1E, 5, 5),  // A
    (0x30, 4, 1),  // B
    (0x2E, 6, 1),  // C
    (0x20, 6, 5),  // D
    (0x12, 6, 3),  // E
    (0x21, 2, 5),  // F
    (0x22, 4, 5),  // G
    (0x23, 0, 5),  // H
    (0x17, 14, 3), // I
    (0x24, 7, 5),  // J
    (0x25, 14, 5), // K
    (0x26, 13, 5), // L
    (0x32, 7, 1),  // M
    (0x31, 0, 1),  // N
    (0x18, 13, 3), // O
    (0x19, 12, 3), // P
    (0x10, 5, 3),  // Q
    (0x13, 2, 3),  // R
    (0x1F, 1, 5),  // S
    (0x14, 4, 3),  // T
    (0x16, 7, 3),  // U
    (0x2F, 2, 1),  // V
    (0x11, 1, 3),  // W
    (0x2D, 1, 1),  // X
    (0x15, 0, 3),  // Y
    (0x2C, 5, 1),  // Z
    // Digit row: 1 ! … 0 _
    (0x02, 5, 4),  // 1
    (0x03, 1, 4),  // 2
    (0x04, 6, 4),  // 3
    (0x05, 2, 4),  // 4
    (0x06, 4, 4),  // 5
    (0x07, 0, 4),  // 6
    (0x08, 7, 4),  // 7
    (0x09, 14, 4), // 8
    (0x0A, 13, 4), // 9
    (0x0B, 12, 4), // 0
    (0x0C, 11, 4), // -  → − =
    (0x0D, 10, 4), // =  → Ю @
    // Punctuation and Estonian letters
    (0x33, 14, 1), // ,  → , <
    (0x34, 13, 1), // .  → . >
    (0x35, 12, 1), // /  → / ?
    (0x1A, 11, 3), // [  → Ü
    (0x1B, 10, 3), // ]  → Õ
    (0x27, 12, 5), // ;  → Ö
    (0x28, 11, 5), // '  → Ä
    (0x2B, 10, 5), // \  → * :
    (0x29, 14, 2), // `, left of 1  → LAT/RUS
    (0x56, 14, 2), // ISO key left of Z → LAT/RUS (the Mac's ISO § is the key left of 1)
    // Editing and cursor keys
    (0x39, 11, 2),       // Space
    (0x1C, 8, 5),        // Enter → RETURN
    (EXT | 0x1C, 8, 5),  // keypad Enter → RETURN
    (0x0E, 13, 2),       // Backspace → ←
    (EXT | 0x53, 8, 2),  // Delete → DEL
    (EXT | 0x47, 8, 4),  // Home → ERASE
    (EXT | 0x4F, 9, 3),  // End → { [
    (EXT | 0x51, 8, 3),  // Page Down → } ]
    (EXT | 0x49, 9, 4),  // Page Up → \ ^
    (EXT | 0x4B, 13, 2), // ←
    (EXT | 0x4D, 12, 2), // →
    (EXT | 0x48, 10, 2), // ↑
    (EXT | 0x50, 9, 2),  // ↓
    (0x01, 3, 4),        // Esc
    (0x0F, 3, 3),        // Tab
    // Function keys
    (0x3B, 3, 0),  // F1
    (0x3C, 5, 0),  // F2
    (0x3D, 6, 0),  // F3
    (0x3E, 2, 0),  // F4
    (0x3F, 4, 0),  // F5
    (0x40, 0, 0),  // F6
    (0x41, 7, 0),  // F7
    (0x42, 14, 0), // F8
    // Keypad: the keys with the same legends. Its digits have these scan
    // codes whether Num Lock is on or not.
    (0x52, 12, 4),       // 0
    (0x4F, 5, 4),        // 1
    (0x50, 1, 4),        // 2
    (0x51, 6, 4),        // 3
    (0x4B, 2, 4),        // 4
    (0x4C, 4, 4),        // 5
    (0x4D, 0, 4),        // 6
    (0x47, 7, 4),        // 7
    (0x48, 14, 4),       // 8
    (0x49, 13, 4),       // 9
    (0x4E, 11, 1),       // +  → ; +
    (0x4A, 11, 4),       // -  → − =
    (0x37, 10, 5),       // *  → * :
    (EXT | 0x35, 12, 1), // /  → / ?
    (0x53, 13, 1),       // .  → . >
];

/// Entries hold `column << 3 | row` plus a "mapped" flag, so that key 0 (F6
/// at column 0, row 0) differs from an empty slot.
const fn build() -> [u16; 512] {
    let mut table = [0u16; 512];
    let mut i = 0;

    while i < ENTRIES.len() {
        let (scan, col, row) = ENTRIES[i];
        table[scan as usize] = 0x100 | key_id(col, row) as u16;
        i += 1;
    }

    table
}

static KEYMAP: [u16; 512] = build();

/// The Juku key at scan code `scan`, if it is mapped.
pub fn juku_key(scan: u16) -> Option<u8> {
    let entry = KEYMAP.get(scan as usize).copied().unwrap_or(0);
    (entry != 0).then_some(entry as u8)
}

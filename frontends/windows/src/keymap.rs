//! Positional mapping from scan codes to Juku keys (MD §6.3, spec §6.4 and
//! §11.3), the same table as frontends/macos/src/Keymap.m. A scan code is
//! bits 16–23 of a key message's lParam, with the extended-key flag (bit 24)
//! as 0x100 here. Scan codes name positions, not characters, so the mapping
//! does not depend on the keyboard layout.
//!
//! The Keyboard window names the PC keys for each Juku key by reversing the
//! table (`host_keys`), in the current keyboard layout.

use juku::keyboard::layout::{Keycap, Legend};
use juku::keyboard::{KEY_CAPS, KEY_CTRL, KEY_SHIFT, key_id};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, HKL, MAPVK_VSC_TO_VK_EX, MapVirtualKeyExW, ToUnicodeEx, VK_ADD, VK_DECIMAL, VK_DIVIDE, VK_MULTIPLY, VK_NUMPAD0,
    VK_SUBTRACT,
};

use crate::strings::{format, localized};

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

// ---- PC keys by name, for the Keyboard window (spec §11.3) --------------------

/// The ISO key left of Z: it is not on ANSI keyboards, so it is named by
/// its place.
const LEFT_OF_Z: u16 = 0x56;

/// A PC key that reaches a Juku key: its tag, the short label the Keyboard
/// window puts on the Juku key, and its name in the window's info strip.
pub struct HostKey {
    pub tag: String,
    pub name: String,
    /// Where the Juku legend says it is.
    pub natural: bool,
    pub keypad: bool,
}

/// The keypad keys of the table and the virtual key each types, whatever
/// Num Lock's state: their characters come from the layout, which may give
/// `,` for the decimal key.
const KEYPAD: [(u16, u16); 16] = [
    (0x52, VK_NUMPAD0.0),
    (0x4F, VK_NUMPAD0.0 + 1),
    (0x50, VK_NUMPAD0.0 + 2),
    (0x51, VK_NUMPAD0.0 + 3),
    (0x4B, VK_NUMPAD0.0 + 4),
    (0x4C, VK_NUMPAD0.0 + 5),
    (0x4D, VK_NUMPAD0.0 + 6),
    (0x47, VK_NUMPAD0.0 + 7),
    (0x48, VK_NUMPAD0.0 + 8),
    (0x49, VK_NUMPAD0.0 + 9),
    (0x4E, VK_ADD.0),
    (0x4A, VK_SUBTRACT.0),
    (0x37, VK_MULTIPLY.0),
    (EXT | 0x35, VK_DIVIDE.0),
    (0x53, VK_DECIMAL.0),
    (EXT | 0x1C, 0), // keypad Enter, named
];

/// Keys with a name rather than a character: tag, name (a string key), and
/// whether it is the key the Juku legend names (Esc for ESC, Enter for
/// RETURN).
fn named_key(scan: u16) -> Option<(&'static str, &'static str, bool)> {
    Some(match scan {
        0x1C => ("enter", "Enter", true),
        s if s == EXT | 0x1C => ("enter", "keypad Enter", true),
        0x0F => ("tab", "Tab", true),
        0x39 => ("space", "Space bar", true),
        0x0E => ("bksp", "Backspace", false),
        0x01 => ("esc", "Esc", true),
        s if s == EXT | 0x53 => ("del", "Delete", true),
        s if s == EXT | 0x47 => ("home", "Home", false),
        s if s == EXT | 0x4F => ("end", "End", false),
        s if s == EXT | 0x49 => ("pgup", "Page Up", false),
        s if s == EXT | 0x51 => ("pgdn", "Page Down", false),
        s if s == EXT | 0x4B => ("←", "←", true),
        s if s == EXT | 0x4D => ("→", "→", true),
        s if s == EXT | 0x48 => ("↑", "↑", true),
        s if s == EXT | 0x50 => ("↓", "↓", true),
        0x3B => ("F1", "F1", true),
        0x3C => ("F2", "F2", true),
        0x3D => ("F3", "F3", true),
        0x3E => ("F4", "F4", true),
        0x3F => ("F5", "F5", true),
        0x40 => ("F6", "F6", true),
        0x41 => ("F7", "F7", true),
        0x42 => ("F8", "F8", true),
        _ => return None,
    })
}

/// The character virtual key `vk` (or, for 0, the key at `scan`) types
/// without modifiers in `layout`, upper case for a letter, or None. A dead
/// key gives its accent. The keyboard state is left as it is.
fn layout_character(scan: u16, vk: u16, layout: HKL) -> Option<String> {
    let code = (scan & 0xFF) as u32 | if scan & EXT != 0 { 0xE000 } else { 0 };
    let vk = if vk != 0 { vk as u32 } else { unsafe { MapVirtualKeyExW(code, MAPVK_VSC_TO_VK_EX, Some(layout)) } };

    if vk == 0 {
        return None;
    }

    let state = [0u8; 256];
    let mut chars = [0u16; 8];
    // Flag 4: leaves the keyboard state, a dead key's too, unchanged.
    let n = unsafe { ToUnicodeEx(vk, code, &state, &mut chars, 4, Some(layout)) };
    let n = if n < 0 { 1 } else { n as usize };
    let text = String::from_utf16_lossy(&chars[..n.min(chars.len())]);

    if !text.chars().next().is_some_and(|c| c > ' ') {
        return None;
    }

    let upper = text.to_uppercase();
    // Not ß → SS.
    Some(if upper.chars().count() == text.chars().count() { upper } else { text })
}

/// Whether a character key's tag is one of the Juku key's legends.
fn tag_matches(tag: &str, keycap: &Keycap) -> bool {
    let tag = if tag == "-" { "−".to_string() } else { tag.to_lowercase() };

    match keycap.legend {
        Legend::Pair => tag == keycap.top.to_lowercase() || tag == keycap.bottom.to_lowercase(),
        Legend::Blank => false,
        _ => tag == keycap.top.to_lowercase(),
    }
}

/// The PC keys that reach a Juku keycap, in the current keyboard layout:
/// the main keys, the ones where the Juku legend says first, then the
/// keypad.
pub fn host_keys(keycap: &Keycap) -> Vec<HostKey> {
    let layout = unsafe { GetKeyboardLayout(0) };
    let mut keys = Vec::new();
    let mut keypad = Vec::new();

    for &(scan, col, row) in ENTRIES {
        if key_id(col, row) != keycap.id {
            continue;
        }

        if let Some(&(_, vk)) = KEYPAD.iter().find(|(s, _)| *s == scan) {
            let key = match named_key(scan) {
                Some((tag, name, _)) => HostKey { tag: tag.into(), name: localized(name).into(), natural: false, keypad: true },
                None => {
                    let Some(c) = layout_character(scan, vk, layout) else { continue };
                    HostKey { name: format(localized("keypad %@"), &[&c]), tag: c, natural: false, keypad: true }
                }
            };
            keypad.push(key);
            continue;
        }

        let key = match named_key(scan) {
            Some((tag, name, natural)) => HostKey { tag: tag.into(), name: localized(name).into(), natural, keypad: false },
            None => {
                let Some(c) = layout_character(scan, 0, layout) else { continue };
                let name = if scan == LEFT_OF_Z { format(localized("the key left of Z (%@)"), &[&c]) } else { c.clone() };
                HostKey { natural: tag_matches(&c, keycap), tag: c, name, keypad: false }
            }
        };
        keys.push(key);
    }

    // The keys emu_window.rs handles itself.
    let modifier = match keycap.id {
        KEY_SHIFT => Some(("shift", "Shift", true)),
        KEY_CTRL => Some(("ctrl", "Ctrl", true)),
        KEY_CAPS => Some(("caps", "Caps Lock", true)),
        SEMICOLON_PLUS => Some(("right Alt", "Right Alt", false)),
        _ => None,
    };

    if let Some((tag, name, natural)) = modifier {
        keys.push(HostKey { tag: localized(tag).into(), name: localized(name).into(), natural, keypad: false });
    }

    keys.sort_by_key(|k| !k.natural); // stable
    keys.extend(keypad);
    keys
}

/// The key a keycap's tag names: the first main key that is not where the
/// Juku legend says, if there is one, else the one that is.
pub fn tag_key(keys: &[HostKey]) -> Option<&HostKey> {
    keys.iter().filter(|k| !k.keypad).fold(None, |tag: Option<&HostKey>, key| match tag {
        None => Some(key),
        Some(t) if t.natural && !key.natural => Some(key),
        t => t,
    })
}

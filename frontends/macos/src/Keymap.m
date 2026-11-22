#pragma once

#import <Carbon/Carbon.h>   // kVK_* virtual key codes

#import "Core.m"

// Positional mapping from macOS virtual key codes to Juku keys (MD §6.3,
// spec §6.4). Entries hold `column << 3 | row` plus a "mapped" flag, so that
// key 0 (F6 at column 0, row 0) differs from an empty slot.
#define JUKU_KEY(col, row) (uint16_t)(0x100 | (col) << 3 | (row))

static const uint16_t kKeymap[128] = {
    // Letters
    [kVK_ANSI_A] = JUKU_KEY(5, 5),
    [kVK_ANSI_B] = JUKU_KEY(4, 1),
    [kVK_ANSI_C] = JUKU_KEY(6, 1),
    [kVK_ANSI_D] = JUKU_KEY(6, 5),
    [kVK_ANSI_E] = JUKU_KEY(6, 3),
    [kVK_ANSI_F] = JUKU_KEY(2, 5),
    [kVK_ANSI_G] = JUKU_KEY(4, 5),
    [kVK_ANSI_H] = JUKU_KEY(0, 5),
    [kVK_ANSI_I] = JUKU_KEY(14, 3),
    [kVK_ANSI_J] = JUKU_KEY(7, 5),
    [kVK_ANSI_K] = JUKU_KEY(14, 5),
    [kVK_ANSI_L] = JUKU_KEY(13, 5),
    [kVK_ANSI_M] = JUKU_KEY(7, 1),
    [kVK_ANSI_N] = JUKU_KEY(0, 1),
    [kVK_ANSI_O] = JUKU_KEY(13, 3),
    [kVK_ANSI_P] = JUKU_KEY(12, 3),
    [kVK_ANSI_Q] = JUKU_KEY(5, 3),
    [kVK_ANSI_R] = JUKU_KEY(2, 3),
    [kVK_ANSI_S] = JUKU_KEY(1, 5),
    [kVK_ANSI_T] = JUKU_KEY(4, 3),
    [kVK_ANSI_U] = JUKU_KEY(7, 3),
    [kVK_ANSI_V] = JUKU_KEY(2, 1),
    [kVK_ANSI_W] = JUKU_KEY(1, 3),
    [kVK_ANSI_X] = JUKU_KEY(1, 1),
    [kVK_ANSI_Y] = JUKU_KEY(0, 3),
    [kVK_ANSI_Z] = JUKU_KEY(5, 1),

    // Digit row: 1 ! … 0 _
    [kVK_ANSI_1] = JUKU_KEY(5, 4),
    [kVK_ANSI_2] = JUKU_KEY(1, 4),
    [kVK_ANSI_3] = JUKU_KEY(6, 4),
    [kVK_ANSI_4] = JUKU_KEY(2, 4),
    [kVK_ANSI_5] = JUKU_KEY(4, 4),
    [kVK_ANSI_6] = JUKU_KEY(0, 4),
    [kVK_ANSI_7] = JUKU_KEY(7, 4),
    [kVK_ANSI_8] = JUKU_KEY(14, 4),
    [kVK_ANSI_9] = JUKU_KEY(13, 4),
    [kVK_ANSI_0] = JUKU_KEY(12, 4),
    [kVK_ANSI_Minus] = JUKU_KEY(11, 4),          // − =
    [kVK_ANSI_Equal] = JUKU_KEY(10, 4),          // Ю @

    // Punctuation and Estonian letters
    [kVK_ANSI_Comma] = JUKU_KEY(14, 1),          // , <
    [kVK_ANSI_Period] = JUKU_KEY(13, 1),         // . >
    [kVK_ANSI_Slash] = JUKU_KEY(12, 1),          // / ?
    [kVK_ANSI_LeftBracket] = JUKU_KEY(11, 3),    // Ü
    [kVK_ANSI_RightBracket] = JUKU_KEY(10, 3),   // Õ
    [kVK_ANSI_Semicolon] = JUKU_KEY(12, 5),      // Ö
    [kVK_ANSI_Quote] = JUKU_KEY(11, 5),          // Ä
    [kVK_ANSI_Backslash] = JUKU_KEY(10, 5),      // * :
    [kVK_ANSI_Grave] = JUKU_KEY(14, 2),          // LAT/RUS
    [kVK_ISO_Section] = JUKU_KEY(14, 2),         // LAT/RUS

    // Editing and cursor keys
    [kVK_Space] = JUKU_KEY(11, 2),
    [kVK_Return] = JUKU_KEY(8, 5),
    [kVK_ANSI_KeypadEnter] = JUKU_KEY(8, 5),
    [kVK_Delete] = JUKU_KEY(13, 2),              // Backspace → ←
    [kVK_ForwardDelete] = JUKU_KEY(8, 2),        // DEL
    [kVK_Home] = JUKU_KEY(8, 4),                 // ERASE
    [kVK_End] = JUKU_KEY(9, 3),                  // { [
    [kVK_PageDown] = JUKU_KEY(8, 3),             // } ]
    [kVK_PageUp] = JUKU_KEY(9, 4),               // \ ^
    [kVK_LeftArrow] = JUKU_KEY(13, 2),
    [kVK_RightArrow] = JUKU_KEY(12, 2),
    [kVK_UpArrow] = JUKU_KEY(10, 2),
    [kVK_DownArrow] = JUKU_KEY(9, 2),
    [kVK_Escape] = JUKU_KEY(3, 4),
    [kVK_Tab] = JUKU_KEY(3, 3),

    // Function keys
    [kVK_F1] = JUKU_KEY(3, 0),
    [kVK_F2] = JUKU_KEY(5, 0),
    [kVK_F3] = JUKU_KEY(6, 0),
    [kVK_F4] = JUKU_KEY(2, 0),
    [kVK_F5] = JUKU_KEY(4, 0),
    [kVK_F6] = JUKU_KEY(0, 0),
    [kVK_F7] = JUKU_KEY(7, 0),
    [kVK_F8] = JUKU_KEY(14, 0),

    // Keypad: the keys with the same legends
    [kVK_ANSI_Keypad0] = JUKU_KEY(12, 4),
    [kVK_ANSI_Keypad1] = JUKU_KEY(5, 4),
    [kVK_ANSI_Keypad2] = JUKU_KEY(1, 4),
    [kVK_ANSI_Keypad3] = JUKU_KEY(6, 4),
    [kVK_ANSI_Keypad4] = JUKU_KEY(2, 4),
    [kVK_ANSI_Keypad5] = JUKU_KEY(4, 4),
    [kVK_ANSI_Keypad6] = JUKU_KEY(0, 4),
    [kVK_ANSI_Keypad7] = JUKU_KEY(7, 4),
    [kVK_ANSI_Keypad8] = JUKU_KEY(14, 4),
    [kVK_ANSI_Keypad9] = JUKU_KEY(13, 4),
    [kVK_ANSI_KeypadPlus] = JUKU_KEY(11, 1),     // ; +
    [kVK_ANSI_KeypadMinus] = JUKU_KEY(11, 4),    // − =
    [kVK_ANSI_KeypadMultiply] = JUKU_KEY(10, 5), // * :
    [kVK_ANSI_KeypadDivide] = JUKU_KEY(12, 1),   // / ?
    [kVK_ANSI_KeypadDecimal] = JUKU_KEY(13, 1),  // . >
};

// Keys reached through flagsChanged rather than keyDown. Caps Lock is not
// one: the core makes the guest follow the host's lock state.
static const uint8_t kJukuKeySemicolonPlus = 11 << 3 | 1;   // Right Option: no host key in its place

// Juku key for a host key code, or -1 if the key is not mapped.
static int jukuKeyForKeyCode(unsigned short keyCode)
{
    uint16_t entry = keyCode < 128 ? kKeymap[keyCode] : 0;
    return entry ? (entry & 0xFF) : -1;
}

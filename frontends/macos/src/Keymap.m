#pragma once

#import <Carbon/Carbon.h>   // kVK_* virtual key codes

#import "Core.m"
#import "Strings.m"

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
static const uint8_t kJukuKeyCaps = 3 << 3 | 5;             // follows the host's Caps Lock

// Juku key for a host key code, or -1 if the key is not mapped.
static int jukuKeyForKeyCode(unsigned short keyCode)
{
    uint16_t entry = keyCode < 128 ? kKeymap[keyCode] : 0;
    return entry ? (entry & 0xFF) : -1;
}

// ---- Host keys by name, for the keyboard window (spec §6.4) ------------------

// A Mac key that reaches a Juku key: its tag, the short label the keyboard
// window puts on the Juku key, and its name in the window's info strip.
@interface HostKey: NSObject
@property(nonatomic, copy) NSString *tag;
@property(nonatomic, copy) NSString *name;
@property(nonatomic) BOOL natural;      // where the Juku legend says it is
@property(nonatomic) BOOL keypad;
@end

@implementation HostKey
@end

static BOOL isKeypadKeyCode(unsigned short keyCode)
{
    switch (keyCode) {
    case kVK_ANSI_Keypad0: case kVK_ANSI_Keypad1: case kVK_ANSI_Keypad2: case kVK_ANSI_Keypad3:
    case kVK_ANSI_Keypad4: case kVK_ANSI_Keypad5: case kVK_ANSI_Keypad6: case kVK_ANSI_Keypad7:
    case kVK_ANSI_Keypad8: case kVK_ANSI_Keypad9: case kVK_ANSI_KeypadPlus: case kVK_ANSI_KeypadMinus:
    case kVK_ANSI_KeypadMultiply: case kVK_ANSI_KeypadDivide: case kVK_ANSI_KeypadDecimal:
    case kVK_ANSI_KeypadEnter:
        return YES;
    default:
        return NO;
    }
}

// The character a key types without modifiers in the current input source,
// upper case for a letter, or nil.
static NSString *layoutCharacter(unsigned short keyCode)
{
    TISInputSourceRef source = TISCopyCurrentKeyboardLayoutInputSource();
    CFDataRef data = source ? TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData) : NULL;

    if (!data) {
        // An input method without a layout of its own, e.g. Japanese.
        if (source) {
            CFRelease(source);
        }

        source = TISCopyCurrentASCIICapableKeyboardLayoutInputSource();
        data = source ? TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData) : NULL;
    }

    NSString *result = nil;

    if (data) {
        const UCKeyboardLayout *layout = (const UCKeyboardLayout *)CFDataGetBytePtr(data);
        UInt32 deadKeys = 0;
        UniChar chars[4];
        UniCharCount length = 0;
        OSStatus status = UCKeyTranslate(layout, keyCode, kUCKeyActionDisplay, 0, LMGetKbdType(),
                                         kUCKeyTranslateNoDeadKeysBit, &deadKeys, 4, &length, chars);

        if (status == noErr && length > 0 && chars[0] > ' ') {
            result = [NSString stringWithCharacters:chars length:length];
            NSString *upper = [result uppercaseString];

            if ([upper length] == [result length]) {
                result = upper;     // not ß → SS
            }
        }
    }

    if (source) {
        CFRelease(source);
    }

    return result;
}

// Keys with a name rather than a character: tag, name.
static NSArray<NSString *> *namedHostKey(unsigned short keyCode)
{
    switch (keyCode) {
    case kVK_Return: return @[@"return", localized(@"Return")];
    case kVK_ANSI_KeypadEnter: return @[@"enter", localized(@"keypad Enter")];
    case kVK_Tab: return @[@"tab", localized(@"Tab")];
    case kVK_Space: return @[@"space", localized(@"Space bar")];
    case kVK_Delete: return @[@"⌫", localized(@"Delete ⌫")];
    case kVK_ForwardDelete: return @[@"⌦", localized(@"Forward Delete ⌦ (fn ⌫ on a laptop)")];
    case kVK_Escape: return @[@"esc", localized(@"Esc")];
    case kVK_Home: return @[@"home", localized(@"Home (fn ← on a laptop)")];
    case kVK_End: return @[@"end", localized(@"End (fn → on a laptop)")];
    case kVK_PageUp: return @[@"pgup", localized(@"Page Up (fn ↑ on a laptop)")];
    case kVK_PageDown: return @[@"pgdn", localized(@"Page Down (fn ↓ on a laptop)")];
    case kVK_LeftArrow: return @[@"←", @"←"];
    case kVK_RightArrow: return @[@"→", @"→"];
    case kVK_UpArrow: return @[@"↑", @"↑"];
    case kVK_DownArrow: return @[@"↓", @"↓"];
    case kVK_F1: return @[@"F1", @"F1"];
    case kVK_F2: return @[@"F2", @"F2"];
    case kVK_F3: return @[@"F3", @"F3"];
    case kVK_F4: return @[@"F4", @"F4"];
    case kVK_F5: return @[@"F5", @"F5"];
    case kVK_F6: return @[@"F6", @"F6"];
    case kVK_F7: return @[@"F7", @"F7"];
    case kVK_F8: return @[@"F8", @"F8"];
    default: return nil;
    }
}

// Whether a key's tag is what the Juku key says: one of its legends, or for
// the named keys, the key in the same place (Esc for ESC, ← for ←).
static BOOL tagMatchesKeycap(NSString *tag, unsigned short keyCode, const JukuKeycap *keycap)
{
    NSString *top = [NSString stringWithUTF8String:keycap->top];
    NSString *bottom = [NSString stringWithUTF8String:keycap->bottom];

    switch (keycap->legend) {
    case JukuLegendPair:
        tag = [tag isEqualToString:@"-"] ? @"−" : tag;
        return [tag caseInsensitiveCompare:top] == NSOrderedSame || [tag caseInsensitiveCompare:bottom] == NSOrderedSame;
    case JukuLegendBlank:
        return keyCode == kVK_Space;
    default:
        return [tag caseInsensitiveCompare:top] == NSOrderedSame;
    }
}

// The Mac keys that reach a Juku keycap, in the current input source: the
// main keys, the ones where the Juku legend says first, then the keypad.
static NSArray<HostKey *> *hostKeysForKeycap(const JukuKeycap *keycap)
{
    NSMutableArray<HostKey *> *keys = [NSMutableArray array];
    NSMutableArray<HostKey *> *keypad = [NSMutableArray array];
    BOOL iso = KBGetLayoutType(LMGetKbdType()) == kKeyboardISO;

    // On an ISO keyboard § is the key left of 1, where an ANSI one has `.
    if (iso && keycap->id == jukuKeyForKeyCode(kVK_ISO_Section)) {
        HostKey *key = [[HostKey alloc] init];
        [key setTag:layoutCharacter(kVK_ISO_Section) ?: @"§"];
        [key setName:[key tag]];
        [keys addObject:key];
    }

    for (unsigned short code = 0; code < 128; code++) {
        if (jukuKeyForKeyCode(code) != keycap->id || code == kVK_ISO_Section) {
            continue;
        }

        HostKey *key = [[HostKey alloc] init];
        NSArray<NSString *> *named = namedHostKey(code);
        NSString *character = named ? nil : layoutCharacter(code);

        if (!named && !character) {
            continue;
        }

        [key setTag:named ? named[0] : character];

        if (isKeypadKeyCode(code)) {
            [key setName:named ? named[1] : [NSString stringWithFormat:localized(@"keypad %@"), character]];
            [key setKeypad:YES];
            [keypad addObject:key];
            continue;
        }

        [key setName:named ? named[1] : character];
        [key setNatural:tagMatchesKeycap([key tag], code, keycap)];
        [keys addObject:key];
    }

    // The keys flagsChanged: gives.
    NSArray<NSString *> *modifier = nil;
    BOOL natural = YES;

    switch (keycap->id) {
    case JukuKeyShift: modifier = @[@"⇧", localized(@"⇧ Shift")]; break;
    case JukuKeyCtrl: modifier = @[@"⌃", localized(@"⌃ Control")]; break;
    case kJukuKeyCaps: modifier = @[@"⇪", localized(@"⇪ Caps Lock")]; break;
    case kJukuKeySemicolonPlus: modifier = @[localized(@"right ⌥"), localized(@"Right Option ⌥")]; natural = NO; break;
    }

    if (modifier) {
        HostKey *key = [[HostKey alloc] init];
        [key setTag:modifier[0]];
        [key setName:modifier[1]];
        [key setNatural:natural];
        [keys addObject:key];
    }

    [keys sortWithOptions:NSSortStable usingComparator:^NSComparisonResult(HostKey *a, HostKey *b) {
        return a.natural == b.natural ? NSOrderedSame : a.natural ? NSOrderedAscending : NSOrderedDescending;
    }];
    [keys addObjectsFromArray:keypad];
    return keys;
}

//! The keyboard as it looks: the 73 keycaps of ДГШ5.104.015, where they are
//! and what is printed on them (MD §6.6), for the front-ends' keyboard
//! windows.
//!
//! Positions and sizes are in quarter keys. `x` counts from the left edge of
//! ESC, so TAB, which starts half a key further left, is at −2. Rows count
//! from the F row, 0, down to the space bar row, 5. The front-ends draw the
//! keys with these legends; the colors and the lettering are theirs.

use super::{KEY_CTRL, KEY_SHIFT, key_id};

/// How a keycap's legends are printed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Legend {
    /// Two characters: `top` at the top left in the light color, `bottom` at
    /// the bottom right in the orange one (Q and Я, ! and 1).
    Pair = 0,
    /// A function key's name, centered (F1).
    Function = 1,
    /// A short word in small letters, centered (ESC, CTRL, ERASE, DEL).
    Word = 2,
    /// A word in large letters, centered, on a wide key (TAB, RETURN, SHIFT).
    WideWord = 3,
    /// Two lines in small letters, centered (CAPS LOCK, LAT RUS).
    TwoLines = 4,
    /// An arrow, `top` saying which: ← → ↑ ↓.
    Arrow = 5,
    /// Nothing: the space bar.
    Blank = 6,
}

#[derive(Clone, Copy, Debug)]
pub struct Keycap {
    /// The key's ID: `column << 3 | row`, or KEY_SHIFT or KEY_CTRL.
    pub id: u8,
    pub x: i8,
    pub row: u8,
    pub width: u8,
    pub legend: Legend,
    pub top: &'static str,
    pub bottom: &'static str,
}

/// Quarter keys per key.
pub const QUARTERS: i8 = 4;

/// The JUKU E5101 badge beside F8 in the F row, with the power LED at its
/// right end: its left edge and width in quarter keys.
pub const BADGE_X: i8 = 38;
pub const BADGE_WIDTH: u8 = 17;

/// A key at matrix position `(column, row)`.
const fn key((col, row): (u8, u8), x: i8, y: u8, w: u8, legend: Legend, top: &'static str, bottom: &'static str) -> Keycap {
    Keycap { id: key_id(col, row), x, row: y, width: w, legend, top, bottom }
}

const fn pair(col: u8, row: u8, x: i8, y: u8, top: &'static str, bottom: &'static str) -> Keycap {
    key((col, row), x, y, 4, Legend::Pair, top, bottom)
}

const fn special(id: u8, x: i8, y: u8, w: u8, legend: Legend, top: &'static str) -> Keycap {
    Keycap { id, x, row: y, width: w, legend, top, bottom: "" }
}

use Legend::*;

/// The keycaps from left to right and top to bottom. The left SHIFT comes
/// before the right one; both are KEY_SHIFT. Taken from the assembly drawing
/// ДГШ5.104.015 СБ and photographs of the E5101.
pub const KEYCAPS: [Keycap; 73] = [
    // F row: F1 above 1
    key((3, 0), 4, 0, 4, Function, "F1", ""),
    key((5, 0), 8, 0, 4, Function, "F2", ""),
    key((6, 0), 12, 0, 4, Function, "F3", ""),
    key((2, 0), 16, 0, 4, Function, "F4", ""),
    key((4, 0), 20, 0, 4, Function, "F5", ""),
    key((0, 0), 24, 0, 4, Function, "F6", ""),
    key((7, 0), 28, 0, 4, Function, "F7", ""),
    key((14, 0), 32, 0, 4, Function, "F8", ""),
    // Digit row
    key((3, 4), 0, 1, 4, Word, "ESC", ""),
    pair(5, 4, 4, 1, "!", "1"),
    pair(1, 4, 8, 1, "\"", "2"),
    pair(6, 4, 12, 1, "#", "3"),
    pair(2, 4, 16, 1, "$", "4"),
    pair(4, 4, 20, 1, "%", "5"),
    pair(0, 4, 24, 1, "&", "6"),
    pair(7, 4, 28, 1, "'", "7"),
    pair(14, 4, 32, 1, "(", "8"),
    pair(13, 4, 36, 1, ")", "9"),
    pair(12, 4, 40, 1, "_", "0"),
    pair(11, 4, 44, 1, "=", "−"),
    pair(10, 4, 48, 1, "@", "Ю"),
    pair(9, 4, 52, 1, "^", "\\"),
    key((8, 4), 56, 1, 4, Word, "ERASE", ""),
    // Q row
    key((3, 3), -2, 2, 8, WideWord, "TAB", ""),
    pair(5, 3, 6, 2, "Q", "Я"),
    pair(1, 3, 10, 2, "W", "В"),
    pair(6, 3, 14, 2, "E", "Е"),
    pair(2, 3, 18, 2, "R", "Р"),
    pair(4, 3, 22, 2, "T", "Т"),
    pair(0, 3, 26, 2, "Y", "Ы"),
    pair(7, 3, 30, 2, "U", "У"),
    pair(14, 3, 34, 2, "I", "И"),
    pair(13, 3, 38, 2, "O", "О"),
    pair(12, 3, 42, 2, "P", "П"),
    pair(11, 3, 46, 2, "Ü", "Ч"),
    pair(10, 3, 50, 2, "Õ", "Щ"),
    pair(9, 3, 54, 2, "{", "["),
    pair(8, 3, 58, 2, "}", "]"),
    // A row
    special(KEY_CTRL, -1, 3, 4, Word, "CTRL"),
    key((3, 5), 3, 3, 4, TwoLines, "CAPS", "LOCK"),
    pair(5, 5, 7, 3, "A", "А"),
    pair(1, 5, 11, 3, "S", "С"),
    pair(6, 5, 15, 3, "D", "Д"),
    pair(2, 5, 19, 3, "F", "Ф"),
    pair(4, 5, 23, 3, "G", "Г"),
    pair(0, 5, 27, 3, "H", "Х"),
    pair(7, 5, 31, 3, "J", "Й"),
    pair(14, 5, 35, 3, "K", "К"),
    pair(13, 5, 39, 3, "L", "Л"),
    pair(12, 5, 43, 3, "Ö", "Э"),
    pair(11, 5, 47, 3, "Ä", "Ш"),
    pair(10, 5, 51, 3, "*", ":"),
    key((8, 5), 55, 3, 8, WideWord, "RETURN", ""),
    // Z row
    special(KEY_SHIFT, 1, 4, 8, WideWord, "SHIFT"),
    pair(5, 1, 9, 4, "Z", "З"),
    pair(1, 1, 13, 4, "X", "Ь"),
    pair(6, 1, 17, 4, "C", "Ц"),
    pair(2, 1, 21, 4, "V", "Ж"),
    pair(4, 1, 25, 4, "B", "Б"),
    pair(0, 1, 29, 4, "N", "Н"),
    pair(7, 1, 33, 4, "M", "М"),
    pair(14, 1, 37, 4, "<", ","),
    pair(13, 1, 41, 4, ">", "."),
    pair(12, 1, 45, 4, "?", "/"),
    pair(11, 1, 49, 4, "+", ";"),
    special(KEY_SHIFT, 53, 4, 8, WideWord, "SHIFT"),
    // Space bar row
    key((14, 2), 3, 5, 4, TwoLines, "LAT", "RUS"),
    key((13, 2), 7, 5, 4, Arrow, "←", ""),
    key((12, 2), 11, 5, 4, Arrow, "→", ""),
    key((11, 2), 15, 5, 32, Blank, "", ""),
    key((10, 2), 47, 5, 4, Arrow, "↑", ""),
    key((9, 2), 51, 5, 4, Arrow, "↓", ""),
    key((8, 2), 55, 5, 4, Word, "DEL", ""),
];

/// The keyboard's extent in quarter keys: from TAB's left edge to RETURN's
/// right edge, and six rows.
pub const LEFT: i8 = -2;
pub const RIGHT: i8 = 63;
pub const ROWS: u8 = 6;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_once() {
        let mut matrix = [0u8; 16];
        let (mut shift, mut ctrl) = (0, 0);

        for k in KEYCAPS {
            match k.id {
                KEY_SHIFT => shift += 1,
                KEY_CTRL => ctrl += 1,
                id => {
                    let (col, row) = (id >> 3, id & 7);
                    assert!(col < 15 && row < 6, "{} at column {col}, row {row}", k.top);
                    assert_eq!(matrix[col as usize] & 1 << row, 0, "{} twice", k.top);
                    matrix[col as usize] |= 1 << row;
                }
            }
        }

        assert_eq!((shift, ctrl), (2, 1));
        // MD §6.3: the matrix positions with a key, column by column.
        let expected = [0x3B, 0x3A, 0x3B, 0x39, 0x3B, 0x3B, 0x3B, 0x3B, 0x3C, 0x1C, 0x3C, 0x3E, 0x3E, 0x3E, 0x3F];
        assert_eq!(matrix[..15], expected);
    }

    #[test]
    fn rows_are_contiguous_and_inside() {
        for row in 0..ROWS {
            let mut keys: Vec<_> = KEYCAPS.iter().filter(|k| k.row == row).collect();
            keys.sort_by_key(|k| k.x);

            for pair in keys.windows(2) {
                assert_eq!(pair[0].x + pair[0].width as i8, pair[1].x, "gap or overlap after {}", pair[0].top);
            }

            let first = keys[0];
            let last = keys[keys.len() - 1];
            assert!(first.x >= LEFT && last.x + last.width as i8 <= RIGHT);
        }

        let f8 = KEYCAPS.iter().find(|k| k.top == "F8").unwrap();
        assert!(BADGE_X > f8.x + f8.width as i8 && BADGE_X + BADGE_WIDTH as i8 <= RIGHT);
    }

    #[test]
    fn legends_fit_the_c_abi() {
        for k in KEYCAPS {
            assert!(k.top.len() < 8 && k.bottom.len() < 8, "{}", k.top);
        }
    }
}

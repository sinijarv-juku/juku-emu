//! The UI strings, shared with the macOS app (spec §11.1): one file per
//! language in resources/strings, the `.strings` format the Mac reads as
//! Localizable.strings. The keys are the Mac app's English titles; Windows-only
//! strings have keys of their own in the same files.

use std::collections::HashMap;
use std::sync::OnceLock;

use windows::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};
use windows::core::PWSTR;

const EN: &str = include_str!("../../../resources/strings/en.strings");
const ET: &str = include_str!("../../../resources/strings/et.strings");

static TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();

/// Picks the language: `language` ("en", "et") if given, else the first of
/// the user's preferred UI languages that the app has, else English.
pub fn init(language: Option<&str>) {
    let language = language.map(str::to_string).or_else(preferred_language).unwrap_or_default();
    let text = if language == "et" { ET } else { EN };
    let _ = TABLE.set(parse(text));
}

/// The string for `key` in the chosen language, or the key itself.
pub fn localized(key: &str) -> &str {
    TABLE.get().and_then(|t| t.get(key)).map_or(key, String::as_str)
}

/// `format` with each `%@` replaced by the next of `args`, and `%1$@`,
/// `%2$@` by the one they name, as NSString formats them: a translation may
/// put the arguments in another order.
pub fn format(format: &str, args: &[&str]) -> String {
    let mut out = String::new();
    let mut rest = format;
    let mut next = 0;

    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];

        if let Some(after) = tail.strip_prefix('@') {
            out.push_str(args.get(next).copied().unwrap_or_default());
            next += 1;
            rest = after;
        } else if let Some((n, after)) = tail.split_once("$@").filter(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
            let index = n.parse::<usize>().unwrap_or(0);
            out.push_str(index.checked_sub(1).and_then(|i| args.get(i)).copied().unwrap_or_default());
            rest = after;
        } else {
            out.push('%');
            rest = tail;
        }
    }

    out.push_str(rest);
    out
}

/// A menu item's title with its underlined letter (§11.3): the Windows-only
/// string `&<key>`, which marks the letter with `&`, or else the plain title
/// with no letter underlined.
pub fn menu_title(key: &str) -> String {
    let marked = format!("&{key}");

    match TABLE.get().and_then(|t| t.get(&marked)) {
        Some(title) => title.clone(),
        None => menu_text(localized(key)),
    }
}

/// `text` as a menu shows it, with no letter underlined: a file name may
/// hold `&`.
pub fn menu_text(text: &str) -> String {
    text.replace('&', "&&")
}

/// The first language in the user's list that is English or Estonian.
fn preferred_language() -> Option<String> {
    let mut count = 0;
    let mut len = 0;

    unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut len).ok()? };
    let mut buffer = vec![0u16; len as usize];
    unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, Some(PWSTR(buffer.as_mut_ptr())), &mut len).ok()? };

    // A list of names ("et-EE", "en-US"), each ending in NUL.
    buffer
        .split(|&c| c == 0)
        .map(String::from_utf16_lossy)
        .map(|name| name.split('-').next().unwrap_or_default().to_ascii_lowercase())
        .find(|lang| lang == "en" || lang == "et")
}

/// Parses `"key" = "value";` pairs. Comments (`/* */`, `//`) and anything
/// outside the quotes are skipped; `\"`, `\\` and `\n` are unescaped.
fn parse(text: &str) -> HashMap<String, String> {
    let mut strings = Vec::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut s = String::new();

                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(other) => s.push(other),
                            None => {}
                        },
                        other => s.push(other),
                    }
                }

                strings.push(s);
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut last = ' ';

                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }

                    last = c;
                }
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            _ => {}
        }
    }

    let mut pairs = strings.into_iter();
    let mut table = HashMap::new();

    while let (Some(key), Some(value)) = (pairs.next(), pairs.next()) {
        table.insert(key, value);
    }

    table
}

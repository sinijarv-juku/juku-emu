//! Expressions (spec §12.5): for `evaluate`, watches, hovers, breakpoint
//! conditions and log points. Numbers as the assembler writes them, the
//! registers, the program's symbols, `[x]` for the byte at x and `w[x]` for
//! the word, and the assembler's operators with `&&` and `||`.

use juku::Machine;

use crate::info::Info;

/// Deepest nesting taken, so that an expression cannot exhaust the stack.
const MAX_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Num(i64),
    Name(String),
    Op(&'static str),
}

const OPS: [&str; 24] =
    ["<<", ">>", "==", "!=", "<=", ">=", "&&", "||", "+", "-", "*", "/", "%", "&", "|", "^", "~", "!", "<", ">", "(", ")", "[", "]"];

/// A number as the assembler writes it (`123`, `0FFh`, `1010b`, `12d`), or
/// with `0x`.
pub(crate) fn number(text: &str) -> Option<i64> {
    let t = text.to_ascii_uppercase();
    let (digits, radix) = if let Some(hex) = t.strip_prefix("0X") {
        (hex, 16)
    } else if let Some(hex) = t.strip_suffix('H') {
        (hex, 16)
    } else if let Some(bin) = t.strip_suffix('B').filter(|b| b.bytes().all(|c| c == b'0' || c == b'1')) {
        (bin, 2)
    } else if let Some(dec) = t.strip_suffix('D') {
        (dec, 10)
    } else {
        (t.as_str(), 10)
    };
    i64::from_str_radix(digits, radix).ok().filter(|_| !digits.is_empty())
}

fn tokens(text: &str) -> Result<Vec<Token>, String> {
    let s = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let name_char = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'?' | b'@' | b'.');
    while i < s.len() {
        let c = s[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() {
            let start = i;
            while i < s.len() && s[i].is_ascii_alphanumeric() {
                i += 1;
            }
            let word = &text[start..i];
            out.push(Token::Num(number(word).ok_or_else(|| "“".to_string() + word + "” is not a number")?));
        } else if c == b'\'' && i + 2 < s.len() && s[i + 2] == b'\'' {
            out.push(Token::Num(s[i + 1] as i64));
            i += 3;
        } else if name_char(c) {
            let start = i;
            while i < s.len() && name_char(s[i]) {
                i += 1;
            }
            out.push(Token::Name(text[start..i].to_string()));
        } else {
            let op = OPS.iter().find(|op| s[i..].starts_with(op.as_bytes())).ok_or("Unexpected character")?;
            out.push(Token::Op(op));
            i += op.len();
        }
    }
    Ok(out)
}

/// Operators by precedence, loosest first: the assembler's, with `||` and
/// `&&` below them.
const LEVELS: [&[&str]; 9] = [
    &["||"],
    &["&&"],
    &["|"],
    &["^"],
    &["&"],
    &["==", "!=", "<", ">", "<=", ">="],
    &["<<", ">>"],
    &["+", "-"],
    &["*", "/", "%"],
];

struct Eval<'a> {
    tokens: Vec<Token>,
    at: usize,
    m: &'a Machine,
    info: &'a Info,
}

/// The value of `text` in the machine as it is.
pub(crate) fn eval(text: &str, m: &Machine, info: &Info) -> Result<i64, String> {
    let mut e = Eval { tokens: tokens(text)?, at: 0, m, info };
    let v = e.binary(0, 0)?;
    if e.at != e.tokens.len() {
        return Err("Unexpected text after the expression".into());
    }
    Ok(v)
}

impl Eval<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn take(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.at).cloned();
        self.at += 1;
        t
    }

    fn expect(&mut self, op: &str) -> Result<(), String> {
        match self.take() {
            Some(Token::Op(o)) if o == op => Ok(()),
            _ => Err("“".to_string() + op + "” expected"),
        }
    }

    fn binary(&mut self, level: usize, depth: usize) -> Result<i64, String> {
        if level == LEVELS.len() {
            return self.unary(depth);
        }
        let mut v = self.binary(level + 1, depth)?;
        while let Some(&Token::Op(op)) = self.peek() {
            if !LEVELS[level].contains(&op) {
                break;
            }
            self.at += 1;
            let r = self.binary(level + 1, depth)?;
            v = match op {
                "||" => (v != 0 || r != 0) as i64,
                "&&" => (v != 0 && r != 0) as i64,
                "|" => v | r,
                "^" => v ^ r,
                "&" => v & r,
                "==" => (v == r) as i64,
                "!=" => (v != r) as i64,
                "<" => (v < r) as i64,
                ">" => (v > r) as i64,
                "<=" => (v <= r) as i64,
                ">=" => (v >= r) as i64,
                "<<" => v.wrapping_shl(r as u32),
                ">>" => v.wrapping_shr(r as u32),
                "+" => v.wrapping_add(r),
                "-" => v.wrapping_sub(r),
                "*" => v.wrapping_mul(r),
                "/" => v.checked_div(r).ok_or("Division by zero")?,
                _ => v.checked_rem(r).ok_or("Division by zero")?,
            };
        }
        Ok(v)
    }

    fn unary(&mut self, depth: usize) -> Result<i64, String> {
        if depth > MAX_DEPTH {
            return Err("Nested too deep".into());
        }
        match self.take().ok_or("Expression expected")? {
            Token::Num(n) => Ok(n),
            Token::Op("-") => Ok(self.unary(depth + 1)?.wrapping_neg()),
            Token::Op("~") => Ok(!self.unary(depth + 1)?),
            Token::Op("!") => Ok((self.unary(depth + 1)? == 0) as i64),
            Token::Op("(") => {
                let v = self.binary(0, depth + 1)?;
                self.expect(")")?;
                Ok(v)
            }
            Token::Op("[") => {
                let a = self.binary(0, depth + 1)?;
                self.expect("]")?;
                Ok(self.m.peek(a as u16) as i64)
            }
            Token::Name(name) if name.eq_ignore_ascii_case("W") && self.peek() == Some(&Token::Op("[")) => {
                self.at += 1;
                let a = self.binary(0, depth + 1)? as u16;
                self.expect("]")?;
                Ok(u16::from_le_bytes([self.m.peek(a), self.m.peek(a.wrapping_add(1))]) as i64)
            }
            Token::Name(name)
                if (name.eq_ignore_ascii_case("HIGH") || name.eq_ignore_ascii_case("LOW")) && self.peek() == Some(&Token::Op("(")) =>
            {
                self.at += 1;
                let v = self.binary(0, depth + 1)?;
                self.expect(")")?;
                Ok(if name.eq_ignore_ascii_case("HIGH") { v >> 8 & 0xFF } else { v & 0xFF })
            }
            Token::Name(name) => self.name(&name),
            Token::Op(_) => Err("Expression expected".into()),
        }
    }

    /// A register, in any case, or else a symbol (`Info::value`).
    fn name(&self, name: &str) -> Result<i64, String> {
        let c = self.m.cpu();
        let v = match name.to_ascii_uppercase().as_str() {
            "A" => c.a as u16,
            "B" => c.b as u16,
            "C" => c.c as u16,
            "D" => c.d as u16,
            "E" => c.e as u16,
            "H" => c.h as u16,
            "L" => c.l as u16,
            "F" => c.flags() as u16,
            "M" => self.m.peek(c.hl()) as u16,
            "BC" => c.bc(),
            "DE" => c.de(),
            "HL" => c.hl(),
            "SP" => c.sp,
            "PC" => c.pc,
            "PSW" => u16::from_be_bytes([c.a, c.flags()]),
            _ => {
                return self.info.value(name)?.map(i64::from).ok_or_else(|| "No symbol or register “".to_string() + name + "”");
            }
        };
        Ok(v as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine() -> Machine {
        let rom = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/roms/ekta37.bin")).unwrap();
        let mut m = Machine::new(&rom).unwrap();
        m.cpu_mut().a = 0x3E;
        m.cpu_mut().h = 0x80;
        m.cpu_mut().l = 0x00;
        m.poke(0x8000, 0x12);
        m.poke(0x8001, 0x34);
        m
    }

    #[test]
    fn values() {
        let (m, info) = (machine(), Info::default());
        let ev = |t: &str| eval(t, &m, &info);
        for (text, want) in [
            ("1 + 2 * 3", 7),
            ("(1 + 2) * 3", 9),
            ("0FFh", 255),
            ("0x10 + 1010b + 12d + 'A'", 16 + 10 + 12 + 65),
            ("a", 0x3E),
            ("A == 3Eh && HL == 8000h", 1),
            ("[HL]", 0x12),
            ("M", 0x12),
            ("w[8000h]", 0x3412),
            ("high(w[hl]) | low(1234h) << 8", 0x3434),
            ("-1", -1),
            ("~0 & 0FFh", 0xFF),
            ("!0 + !5", 1),
            ("1 << 4 + 1", 32),
            ("7 % 3 == 1 || 0", 1),
        ] {
            assert_eq!(ev(text), Ok(want), "{text}");
        }
        for text in ["", "1 +", "(1", "[1", "1 / 0", "NOSUCH", "1 2", "#", "0FFx"] {
            assert!(ev(text).is_err(), "{text}");
        }
        let deep = "(".repeat(100) + "1" + &")".repeat(100);
        assert!(ev(&deep).is_err());
    }
}

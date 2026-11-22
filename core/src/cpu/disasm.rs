//! 8080 disassembler: Intel mnemonics, as juku-sdk's instruction table
//! (`juku-i8080`) names them; the debug server's test
//! `cpu_disassembler_is_the_sdks` checks every opcode against it.
//!
//! Operands are hex without a suffix (`JMP 0103`). Undocumented opcodes show
//! as their documented twins with a `*` (08h `NOP*`, CBh `JMP*`, D9h `RET*`,
//! DDh/EDh/FDh `CALL*`).

use core::fmt;

const R: [&str; 8] = ["B", "C", "D", "E", "H", "L", "M", "A"];
const RP: [&str; 4] = ["B", "D", "H", "SP"];
const RP_PSW: [&str; 4] = ["B", "D", "H", "PSW"];
const ALU: [&str; 8] = ["ADD", "ADC", "SUB", "SBB", "ANA", "XRA", "ORA", "CMP"];
const ALU_IMM: [&str; 8] = ["ADI", "ACI", "SUI", "SBI", "ANI", "XRI", "ORI", "CPI"];
const ROT: [&str; 8] = ["RLC", "RRC", "RAL", "RAR", "DAA", "CMA", "STC", "CMC"];

/// One instruction: its opcode and the two bytes after it, of which only
/// `size() - 1` belong to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instr {
    pub op: u8,
    pub b1: u8,
    pub b2: u8,
}

/// Length in bytes of the instruction with opcode `op`.
pub fn len(op: u8) -> u8 {
    match op {
        // LXI, SHLD, LHLD, STA, LDA
        0x01 | 0x11 | 0x21 | 0x31 | 0x22 | 0x2A | 0x32 | 0x3A => 3,
        // MVI
        0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x36 | 0x3E => 2,
        // Jcc, JMP, Ccc, CALL
        0xC0..=0xFF if matches!(op & 7, 2 | 4) || matches!(op, 0xC3 | 0xCB | 0xCD | 0xDD | 0xED | 0xFD) => 3,
        // ALU immediate, OUT, IN
        0xC0..=0xFF if op & 7 == 6 || matches!(op, 0xD3 | 0xDB) => 2,
        _ => 1,
    }
}

impl Instr {
    /// The instruction at `addr`, its bytes read with `read`.
    pub fn at(addr: u16, mut read: impl FnMut(u16) -> u8) -> Instr {
        Instr { op: read(addr), b1: read(addr.wrapping_add(1)), b2: read(addr.wrapping_add(2)) }
    }

    /// Its length in bytes.
    pub fn size(&self) -> u8 {
        len(self.op)
    }

    fn word(&self) -> u16 {
        u16::from_le_bytes([self.b1, self.b2])
    }
}

/// What follows the register operands.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Imm {
    None,
    Byte,
    Word,
}

const RCC: [&str; 8] = ["RNZ", "RZ", "RNC", "RC", "RPO", "RPE", "RP", "RM"];
const JCC: [&str; 8] = ["JNZ", "JZ", "JNC", "JC", "JPO", "JPE", "JP", "JM"];
const CCC: [&str; 8] = ["CNZ", "CZ", "CNC", "CC", "CPO", "CPE", "CP", "CM"];
const DIGITS: [&str; 8] = ["0", "1", "2", "3", "4", "5", "6", "7"];

impl Instr {
    /// The mnemonic, up to two register operands ("" for none) and the kind
    /// of immediate operand.
    fn parts(&self) -> (&'static str, [&'static str; 2], Imm) {
        let o = self.op;
        let (x, y, z) = (o >> 6, (o >> 3) as usize & 7, o as usize & 7);
        let none = Imm::None;
        match (x, z) {
            _ if o == 0x76 => ("HLT", ["", ""], none),
            (1, _) => ("MOV", [R[y], R[z]], none),
            (2, _) => (ALU[y], [R[z], ""], none),
            (0, 0) => (if y == 0 { "NOP" } else { "NOP*" }, ["", ""], none),
            (0, 1) if y % 2 == 0 => ("LXI", [RP[y >> 1], ""], Imm::Word),
            (0, 1) => ("DAD", [RP[y >> 1], ""], none),
            (0, 2) => match y {
                0 => ("STAX", ["B", ""], none),
                1 => ("LDAX", ["B", ""], none),
                2 => ("STAX", ["D", ""], none),
                3 => ("LDAX", ["D", ""], none),
                4 => ("SHLD", ["", ""], Imm::Word),
                5 => ("LHLD", ["", ""], Imm::Word),
                6 => ("STA", ["", ""], Imm::Word),
                _ => ("LDA", ["", ""], Imm::Word),
            },
            (0, 3) => (if y % 2 == 0 { "INX" } else { "DCX" }, [RP[y >> 1], ""], none),
            (0, 4) => ("INR", [R[y], ""], none),
            (0, 5) => ("DCR", [R[y], ""], none),
            (0, 6) => ("MVI", [R[y], ""], Imm::Byte),
            (0, _) => (ROT[y], ["", ""], none),
            (_, 0) => (RCC[y], ["", ""], none),
            (_, 1) if y % 2 == 0 => ("POP", [RP_PSW[y >> 1], ""], none),
            (_, 1) => (["RET", "RET*", "PCHL", "SPHL"][y >> 1], ["", ""], none),
            (_, 2) => (JCC[y], ["", ""], Imm::Word),
            (_, 3) => match y {
                0 => ("JMP", ["", ""], Imm::Word),
                1 => ("JMP*", ["", ""], Imm::Word),
                2 => ("OUT", ["", ""], Imm::Byte),
                3 => ("IN", ["", ""], Imm::Byte),
                _ => (["XTHL", "XCHG", "DI", "EI"][y - 4], ["", ""], none),
            },
            (_, 4) => (CCC[y], ["", ""], Imm::Word),
            (_, 5) if y % 2 == 0 => ("PUSH", [RP_PSW[y >> 1], ""], none),
            (_, 5) => (if y == 1 { "CALL" } else { "CALL*" }, ["", ""], Imm::Word),
            (_, 6) => (ALU_IMM[y], ["", ""], Imm::Byte),
            _ => ("RST", [DIGITS[y], ""], none),
        }
    }

    /// The instruction as Intel writes it: `MVI A,3E`, `JMP 0103`.
    /// Written out by hand, so that no formatting machinery comes with it.
    pub fn text(&self) -> String {
        self.text_with(|_| None)
    }

    /// As [`Instr::text`], with a 16-bit operand given as the name `name`
    /// returns for it, if any: `CALL TTO`.
    pub fn text_with(&self, name: impl FnOnce(u16) -> Option<String>) -> String {
        let (mnemonic, regs, imm) = self.parts();
        let mut out = String::from(mnemonic);
        for (i, r) in regs.iter().filter(|r| !r.is_empty()).enumerate() {
            out.push(if i == 0 { ' ' } else { ',' });
            out.push_str(r);
        }
        let (v, digits) = match imm {
            Imm::None => return out,
            Imm::Byte => (self.b1 as u16, 2),
            Imm::Word => (self.word(), 4),
        };
        out.push(if regs[0].is_empty() { ' ' } else { ',' });
        if imm == Imm::Word
            && let Some(name) = name(v)
        {
            out.push_str(&name);
            return out;
        }
        for d in (0..digits).rev() {
            out.push(b"0123456789ABCDEF"[(v >> (4 * d) & 15) as usize] as char);
        }
        out
    }
}

impl fmt::Display for Instr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_for_words() {
        let named = |v| (v == 0xFFD9).then(|| "TTO".to_string());
        assert_eq!(Instr { op: 0xCD, b1: 0xD9, b2: 0xFF }.text_with(named), "CALL TTO");
        assert_eq!(Instr { op: 0x21, b1: 0xD9, b2: 0xFF }.text_with(named), "LXI H,TTO");
        assert_eq!(Instr { op: 0x3E, b1: 0xD9, b2: 0xFF }.text_with(named), "MVI A,D9");
        assert_eq!(Instr { op: 0xCD, b1: 0x00, b2: 0x01 }.text_with(named), "CALL 0100");
    }

    #[test]
    fn reads_wrap_around() {
        let mem = [0xC3, 0x00, 0x01];
        let i = Instr::at(0xFFFF, |a| mem[(a.wrapping_add(1)) as usize]);
        assert_eq!(i.to_string(), "JMP 0100");
    }
}

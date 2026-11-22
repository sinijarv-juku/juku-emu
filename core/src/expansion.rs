//! The plug-in memory expander E6201 (MD §3.2): a ROM on the I-41 bus that
//! appears in the external window, 4000–BFFF, in mode 2. The monitor's `A`
//! command copies the program from the window to 0100h and runs it.
//!
//! Only images the monitor can start are accepted: they begin with the
//! bootable program header (MD §14.1), `JMP start / DW key / DW length`, as
//! laid out once copied to 0100h.

/// Size of the external window, and so the largest image.
pub const WINDOW_SIZE: usize = 0x8000;

/// Where the `A` command copies the program.
const LOAD_ADDR: usize = 0x0100;
/// `JMP start`, `DW key`, `DW length`.
const HEADER_LEN: usize = 7;
const JMP: u8 = 0xC3;

#[derive(Debug, PartialEq, Eq)]
pub enum ExpansionError {
    /// Empty, or larger than the 32 KB window.
    BadSize,
    /// No program header the monitor can start: no `JMP` at the start, a
    /// length that does not fit the image, or a jump outside the program.
    NoHeader,
}

/// Checks that `image` is an expansion ROM the monitor can start.
pub fn check(image: &[u8]) -> Result<(), ExpansionError> {
    if image.is_empty() || image.len() > WINDOW_SIZE {
        return Err(ExpansionError::BadSize);
    }
    if image.len() < HEADER_LEN || image[0] != JMP {
        return Err(ExpansionError::NoHeader);
    }
    let word = |i: usize| u16::from_le_bytes([image[i], image[i + 1]]) as usize;
    let (start, length) = (word(1), word(5));
    // The header itself may be the first code to run (JMP 0103h).
    let program = LOAD_ADDR + 3..LOAD_ADDR + length;
    if !(HEADER_LEN..=image.len()).contains(&length) || !program.contains(&start) {
        return Err(ExpansionError::NoHeader);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// JBASIC 1.1's header: JMP 0107h, key DCBAh, length 2000h, then a JMP
    /// to its relocator.
    fn jbasic_like() -> Vec<u8> {
        let mut image = vec![0xFF; 0x2000];
        image[..10].copy_from_slice(&[0xC3, 0x07, 0x01, 0xBA, 0xDC, 0x00, 0x20, 0xC3, 0x00, 0x20]);
        image
    }

    #[test]
    fn accepts_a_program_header() {
        assert_eq!(check(&jbasic_like()), Ok(()));
        let mut image = jbasic_like();
        image.resize(WINDOW_SIZE, 0xFF);
        assert_eq!(check(&image), Ok(()), "a full window");
    }

    #[test]
    fn rejects_wrong_sizes() {
        assert_eq!(check(&[]), Err(ExpansionError::BadSize));
        let mut image = jbasic_like();
        image.resize(WINDOW_SIZE + 1, 0xFF);
        assert_eq!(check(&image), Err(ExpansionError::BadSize));
    }

    #[test]
    fn rejects_images_without_a_header() {
        let cases: [(usize, u8); 6] = [
            (0, 0x00), // no JMP
            (6, 0x30), // length 3000h, longer than the image
            (6, 0x00), // length 0
            (2, 0x00), // jumps to 0007h, below the program
            (1, 0x01), // jumps to 0101h, into the JMP itself
            (2, 0x30), // jumps to 3007h, past the end
        ];
        for (at, byte) in cases {
            let mut image = jbasic_like();
            image[at] = byte;
            assert_eq!(check(&image), Err(ExpansionError::NoHeader), "byte {at} = {byte:02X}");
        }
        assert_eq!(check(&[0xC3, 0x03, 0x01]), Err(ExpansionError::NoHeader), "too short");
    }
}
